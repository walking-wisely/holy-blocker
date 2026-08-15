//! Real `SourceFetcher` for Université Toulouse 1 Capitole's category blocklists.
//!
//! DRAFT — written for folding into `packages/domain-blocklist`'s module-7 `cli`
//! integration pass, not for direct inclusion yet. Nothing under
//! `packages/domain-blocklist/src/` was modified to produce this.
//!
//! Why this file exists at all, in the crate's own words
//! (`docs/components/domain-blocklist/plan.md` §7 assumption audit, run 2026-08-15):
//!
//! - UT1 serves category downloads as `.tar.gz` archives, e.g.
//!   `https://dsi.ut-capitole.fr/blacklists/download/adult.tar.gz` (and `gambling.tar.gz`,
//!   `dating.tar.gz` for this crate's other two categories). Each archive holds exactly
//!   `<category>/domains`, `<category>/urls`, `<category>/expressions`, `<category>/usage` — so a
//!   fetcher must decompress and extract `<category>/domains` (and `<category>/urls`, whose entry
//!   count `super::ut1::count_urls_entries` reports as the documented coverage gap) before any
//!   parser sees bytes. Handing the raw gzip bytes to `parse_document` would parse to all-
//!   `dropped_malformed` garbage rather than erroring loudly; this module's `SourceFetcher`
//!   impls the decompress-and-extract step so that failure is an explicit `FetchError` instead.
//! - UT1 has no license API. The only license source is the HTML page at
//!   <https://dsi.ut-capitole.fr/blacklists/>, which carries a live `<a rel="license" href=...>`
//!   anchor pointing at a Creative Commons BY-SA 4.0 URL. The raw page *also* contains a second,
//!   contradictory CC BY-NC-SA block inside an HTML `<!-- ... -->` comment (dead leftover badge
//!   markup). A naive substring scan for `creativecommons.org/licenses/` would match the comment
//!   too and misreport `FetchError::LicenseChanged`/`LicenseNotAllowed` on every run against a
//!   page that hasn't actually changed. `extract_license` here strips HTML comments first and
//!   then parses only the `rel="license"` anchor's `href`.
//! - UT1 publishes no per-fetch revision the way StevenBlack/Hagezi have a Git commit SHA. What
//!   `FetchedSource.revision` is instead — an HTTP `ETag`, else `Last-Modified`, else a content
//!   hash — and the pin-churn tradeoff that choice creates against `fetch_source`'s exact-match
//!   pin check are discussed in `ut1_fetch_notes.md`. The decision is *not* silently invented
//!   here.
//!
//! Dependency shape (all new to `packages/domain-blocklist`; see the notes file):
//! `reqwest` (async, `rustls-tls` feature, no `native-tls`) + `tokio` for HTTP, `tar` + `flate2`
//! for in-memory archive extraction. No bytes ever touch disk, and the two pure functions —
//! [`extract_domains_file`] and [`extract_license`] — are unit-tested against fixtures, not
//! against live fetches, per this project's test-first rule.

use std::io::Read;
use std::marker::PhantomData;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::types::{Category, LicenseId, SourceId, Timestamp};

use crate::sources::ut1::count_urls_entries;
use crate::sources::{FetchError, FetchedSource, SourceConfig, SourceFetcher};

use flate2::read::GzDecoder;
use reqwest::header::{ETAG, LAST_MODIFIED};
use sha2::Digest;

/// The UT1 license page, scraped for the live `rel="license"` anchor. The only machine-readable
/// license source UT1 publishes; the fetch audit verified the page answers normally.
pub const UT1_LICENSE_PAGE: &str = "https://dsi.ut-capitole.fr/blacklists/";

/// Hard ceiling on one fetched payload, past which the fetch fails rather than growing memory
/// without bound. A category tarball is at most a few MB; 512 MiB is an order-of-magnitude
/// headroom that still refuses an accidentally-served log dump or a corrupted endless stream.
pub const MAX_FETCHED_BYTES: u64 = 512 * 1024 * 1024;

/// Hard ceiling on one extracted (decompressed) archive member, applied both to the reservation
/// and to the read in [`extract_member`]. A tar entry's declared `size()` header and its actual
/// decompressed byte count are both untrusted data from inside the fetched archive — a crafted
/// header declaring e.g. 4 GiB would make a naive `Vec::with_capacity` reserve that much from a
/// few kilobytes of compressed input, and reading with no cap permits a decompression-bomb-shaped
/// gzip stream to actually produce that much. UT1's largest `domains`/`urls` member measures a few
/// MB (assumption audit, run 2026-08-15); 64 MiB is generous headroom that still refuses either
/// attack from a tiny compressed payload.
pub const MAX_MEMBER_BYTES: u64 = 64 * 1024 * 1024;

/// A UT1 category the fetcher can target, as a type so two fetchers for different categories
/// are distinct types and a `source: SourceId::Ut1` `SourceConfig` can't be silently fetched
/// with the wrong extraction directory. The directory name is what `SourceConfig.url`'s tarball
/// is expected to embed, and the `Category` is what `super::ut1::parse_domains` must tag entries
/// with (UT1 gives no in-band signal of which category a `domains` file belongs to, per that
/// function's own doc comment).
pub trait Ut1Category {
    /// The archive directory this category unpacks into: `"adult"`, `"gambling"`, `"dating"`.
    const DIR: &'static str;
    /// The crate's filter category this directory maps to.
    fn to_category() -> Category;
}

/// UT1's `adult` directory.
pub struct Adult;
impl Ut1Category for Adult {
    const DIR: &'static str = "adult";
    fn to_category() -> Category {
        Category::Adult
    }
}

/// UT1's `gambling` directory.
pub struct Gambling;
impl Ut1Category for Gambling {
    const DIR: &'static str = "gambling";
    fn to_category() -> Category {
        Category::Gambling
    }
}

/// UT1's `dating` directory.
pub struct Dating;
impl Ut1Category for Dating {
    const DIR: &'static str = "dating";
    fn to_category() -> Category {
        Category::Dating
    }
}

/// Bounded retry policy for fetches against `dsi.ut-capitole.fr`.
///
/// The server is a French university origin, not a CDN: the assumption audit verified live
/// tarball and license-page fetches both answer normally, but deferred to no availability or
/// latency guarantee from UT1. The policy is therefore small and bounded — a monthly batch
/// pipeline must fail loudly rather than retry forever — while still tolerating the transient
/// `5xx`/connect-timeout a single-origin server realistically throws under load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ut1RetryPolicy {
    /// Total HTTP attempts per resource (the first try plus retries). Each attempt is a fresh
    /// request; the tarball and the license page each get their own budget.
    pub max_attempts: u32,
    /// Base backoff before retry *k*, `min(base_backoff * 2^(k-1), max_backoff)`. Base is `2s`
    /// by default — long enough to let a transient university-server hiccup clear, short
    /// enough that a genuinely unhealthy origin costs the build only tens of seconds.
    pub base_backoff: Duration,
    /// Hard ceiling on a single backoff sleep.
    pub max_backoff: Duration,
}

impl Default for Ut1RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_backoff: Duration::from_secs(2),
            max_backoff: Duration::from_secs(8),
        }
    }
}

/// Whether a `5xx` status from the *latest* attempt earns another try, given `retries_so_far`
/// already spent. Pure so the retry decision is unit-testable without a server.
///
/// 404 is deliberately **not** retried: that is "category directory does not exist", a
/// config/upstream-shape problem, and it surfaces as a clear `FetchError` instead of a masked
/// series of futile requests. Only 5xx (plausibly-transient server-side failure) is retried,
/// matching the task's "connect timeout or 5xx" bar. Timeout retries are budgeted the same way
/// by the fetch loop.
pub fn should_retry_status(status: u16, retries_so_far: u32, policy: &Ut1RetryPolicy) -> bool {
    (500..=599).contains(&status) && retries_so_far < policy.max_attempts.saturating_sub(1)
}

/// Whether a `reqwest`-originated transport failure (a timeout, or a mid-body/read
/// [`AttemptFailure::NetworkError`]) earns another attempt, given `retries_so_far` already spent.
/// Mirrors [`should_retry_status`]'s budget shape and is derived from the loop's own guard so the
/// decision is testable without a server. A body-read error is retried exactly like a connect
/// failure: the in-flight response is dead and its bytes are already lost, so a fresh request is
/// the only recovery that can work — the identical reasoning `fetchers::github::classify_reqwest_error`
/// documents for its connect bucket. A deliberate size-ceiling rejection
/// ([`AttemptFailure::Transport`]) is *not* retried: same URL, same answer.
fn should_retry_transport_failure(
    failure: &AttemptFailure,
    retries_so_far: u32,
    policy: &Ut1RetryPolicy,
) -> bool {
    matches!(failure, AttemptFailure::Timeout(_) | AttemptFailure::NetworkError(_))
        && retries_so_far < policy.max_attempts.saturating_sub(1)
}

/// Backoff for retry `k` (0-based), bounded by `max_backoff`. `2s, 4s, 8s` at the defaults.
pub fn backoff_for(k: u32, policy: &Ut1RetryPolicy) -> Duration {
    let exponent = k.min(6); // 2^6 = 64s already caps; any higher stays at max_backoff.
    let secs = policy
        .base_backoff
        .as_secs()
        .saturating_mul(1u64 << exponent)
        .min(policy.max_backoff.as_secs());
    Duration::from_secs(secs)
}

/// Why extracting a member from a UT1 `.tar.gz` failed. A `FetchError::Transport(String)`
/// wrapper is what actually reaches the build (`FetchError`'s variants are fixed upstream); this
/// enum exists so the pure [`extract_domains_file`]/[`extract_urls_file`] functions are
/// independently matchable in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ut1ExtractError {
    /// Bytes don't start with the gzip magic header (`0x1f 0x8b`, [RFC 1952 §2.2]). Almost
    /// certainly the wrong URL was fetched (e.g. the HTML index page instead of a tarball).
    NotGzip,
    /// Gzip decompression failed partway through (corrupt/truncated download).
    GzipInvalid(String),
    /// The decompressed stream isn't a valid tar archive.
    TarInvalid(String),
    /// The archive is valid but contains no `<dir>/<member>` regular file. For the fetch path
    /// this is the loud version of "UT1 changed their layout in a way that breaks this crate".
    MemberNotFound {
        category_dir: String,
        member: &'static str,
    },
    /// `<dir>/<member>` exists but is a directory/symlink/etc., not a regular file — reading it
    /// would return archive metadata or a link target, never the document the parser expects.
    MemberNotRegularFile {
        category_dir: String,
        member: &'static str,
    },
    /// A category directory string that isn't safe to interpolate into an archive entry path
    /// (empty, absolute, or containing `/`, `..`, `\`, or a leading `.`).
    InvalidCategoryDir(String),
    /// `<dir>/<member>` decompressed to more than [`MAX_MEMBER_BYTES`] — refused rather than
    /// trusted, since both the declared tar header size and the actual decompressed byte count
    /// are untrusted data from inside a payload this crate does not control (a UT1 `domains`
    /// member is a few MB; a crafted header or a decompression-bomb-shaped gzip stream is not).
    MemberTooLarge {
        category_dir: String,
        member: &'static str,
    },
}

impl std::fmt::Display for Ut1ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotGzip => write!(
                f,
                "bytes are not a gzip stream (missing 0x1f 0x8b magic) — wrong URL fetched?"
            ),
            Self::GzipInvalid(e) => write!(f, "gzip decode failed: {e}"),
            Self::TarInvalid(e) => write!(f, "invalid tar archive: {e}"),
            Self::MemberNotFound { category_dir, member } => {
                write!(f, "archive contains no `{category_dir}/{member}` entry")
            }
            Self::MemberNotRegularFile { category_dir, member } => {
                write!(f, "`{category_dir}/{member}` is not a regular file")
            }
            Self::InvalidCategoryDir(d) => write!(f, "invalid category directory `{d}`"),
            Self::MemberTooLarge { category_dir, member } => write!(
                f,
                "`{category_dir}/{member}` exceeds the {MAX_MEMBER_BYTES}-byte member ceiling"
            ),
        }
    }
}

impl std::error::Error for Ut1ExtractError {}

/// Why `extract_license` couldn't produce a `LicenseId`. Also wrapped in
/// `FetchError::Transport` for the build; the enumeration keeps the pure function's failure
/// cases matchable in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ut1LicenseError {
    /// No `<a rel="license" href="...">` anchor exists on the (comment-stripped) page. The
    /// page's license markup is gone or changed shape; guessing would be worse than failing.
    LicenseAnchorNotFound,
    /// An anchor carried `rel="license"` but no `href` attribute to map.
    LicenseHrefMissing,
    /// The anchor's `href` names a Creative Commons license this module has not explicitly
    /// mapped to an SPDX identifier. Failing closed (rather than guessing `CC-BY-SA-4.0`) is
    /// deliberate: a new license the pin never reviewed must surface as a `LicenseChanged`/
    /// `LicenseNotAllowed` abort, not as a silently same-value result.
    UnsupportedLicenseUrl(String),
}

impl std::fmt::Display for Ut1LicenseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LicenseAnchorNotFound => write!(
                f,
                "no `<a rel=\"license\" href=...>` anchor on the license page (after stripping comments)"
            ),
            Self::LicenseHrefMissing => write!(f, "license anchor has no `href` attribute"),
            Self::UnsupportedLicenseUrl(u) => {
                write!(f, "license URL `{u}` is not a mapped Creative Commons license")
            }
        }
    }
}

impl std::error::Error for Ut1LicenseError {}

/// The full result of one UT1 category fetch: the [`FetchedSource`] the trait plumbing expects
/// plus the sibling `urls` file's bytes, extracted from the *same* tarball response so the
/// caller never makes two HTTP requests (the requirement that `count_urls_entries` — the
/// documented `urls` coverage-gap metric — have its input without a second download).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ut1Fetched {
    /// The trait-compatible result; `bytes` is exactly `<dir>/domains` and feeds
    /// `parse_domains(bytes, C::to_category())`.
    pub fetched: FetchedSource,
    /// The sibling `<dir>/urls` file's bytes.
    pub urls_bytes: Vec<u8>,
    /// Precomputed `count_urls_entries(&self.urls_bytes)` so the caller sees the gap metric
    /// without having to import the counter itself.
    pub urls_entry_count: u64,
}

/// The real UT1 `SourceFetcher`: one tarball request + one license-page request per fetch, both
/// bounded by [`Ut1RetryPolicy`], with in-memory archive extraction.
///
/// `C` is the [`Ut1Category`] this fetcher targets; it (a) picks which member the tarball is
/// extracted from and (b) seeds the `Category` the caller hands `parse_domains`. The tarball
/// URL's last path segment is validated against `C::DIR` so a copy-pasted config can't silently
/// fetch `gambling.tar.gz` with an `Adult` fetcher.
///
/// Async/`sync` bridge: `reqwest` futures need a Tokio runtime, but [`SourceFetcher::fetch`] is
/// synchronous (the trait is fixed upstream). This impl therefore builds a current-thread
/// runtime per sync `fetch` call and `block_on`s inside it. Callers already inside an async
/// runtime must call [`Ut1SourceFetcher::fetch_files`] directly — `block_on` from inside a
/// runtime panics — see the notes file.
pub struct Ut1SourceFetcher<C> {
    retry: Ut1RetryPolicy,
    client: reqwest::Client,
    _category: PhantomData<C>,
}

impl<C: Ut1Category> Ut1SourceFetcher<C> {
    /// A fetcher with an idle `User-Agent` (`holy-blocker/<version>`) and a client built with
    /// `rustls` (the crate must be built with `default-features = false,
    /// features = ["rustls-tls"]` — never `native-tls`).
    ///
    /// Timeout rationale: `dsi.ut-capitole.fr` is a single French university origin, not a CDN —
    /// the assumption audit measured live fetches but no availability promise. `connect` gets
    /// `15s` (generous for a sometimes-slow origin), the whole request `120s` (a category tarball
    /// is a few MB at most, but this origin has been slow under load). Bounded so a hung server
    /// fails the batch loudly after two minutes instead of stalling it indefinitely.
    pub fn new(retry: Ut1RetryPolicy, connect_timeout: Duration, request_timeout: Duration) -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(connect_timeout)
            .timeout(request_timeout)
            .user_agent(concat!("holy-blocker/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("building a reqwest client is infallible given valid timeouts");
        Self {
            retry,
            client,
            _category: PhantomData,
        }
    }

    /// The one real fetch: tarball request (`config.url`), license-page request
    /// ([`UT1_LICENSE_PAGE`]), extraction of both archive members, revision/license/fetched_at
    /// assembly. Exactly two HTTP requests — the `urls` member comes out of the same tarball
    /// response as `domains`, never a third request.
    pub async fn fetch_files(&self, config: &SourceConfig) -> Result<Ut1Fetched, FetchError> {
        Self::verify_config_category(config)?;

        let (tarball_bytes, etag, last_modified) =
            self.get_bytes_with_retry(&config.url, "the category tarball (renamed upstream?)").await?;
        let license_page = self.get_text_with_retry(UT1_LICENSE_PAGE).await?;
        let license = extract_license(&license_page).map_err(ut1_license_to_fetch)?;
        let fetched_at = now_timestamp();

        let domains =
            extract_domains_file(&tarball_bytes, C::DIR).map_err(ut1_extract_to_fetch)?;
        let urls = extract_urls_file(&tarball_bytes, C::DIR).map_err(ut1_extract_to_fetch)?;
        let revision = pick_revision(etag.as_deref(), last_modified.as_deref(), &domains);
        let urls_entry_count = count_urls_entries(&urls);

        Ok(Ut1Fetched {
            fetched: FetchedSource {
                revision,
                license,
                fetched_at,
                bytes: domains,
            },
            urls_bytes: urls,
            urls_entry_count,
        })
    }

    /// Checks `config` names the category this fetcher targets: `source` must be `Ut1` (a
    /// `SourceConfig` for StevenBlack/Hagezi reaching here is a wiring bug), and the tarball
    /// URL's last path segment must be `<dir>.tar.gz` (a copy-paste error between source configs
    /// must abort, not silently extract the wrong category).
    fn verify_config_category(config: &SourceConfig) -> Result<(), FetchError> {
        if config.source != SourceId::Ut1 {
            return Err(FetchError::Transport(format!(
                "Ut1SourceFetcher<{}> called with a config for {:?}",
                C::DIR,
                config.source
            )));
        }
        let path = config
            .url
            .split(['/', '\\'])
            .next_back()
            .unwrap_or(config.url.as_str());
        let expected = format!("{}.tar.gz", C::DIR);
        if path != expected {
            return Err(FetchError::Transport(format!(
                "UT1 config URL ends in `{path}`, expected `{expected}` (category/fetcher mismatch)"
            )));
        }
        Ok(())
    }

    /// One bounded, backoff'd GET returning the bytes plus the `ETag`/`Last-Modified` headers
    /// (captured before the body is consumed), or a terminal `FetchError`. 404 (category never
    /// existed / was renamed) is a distinct, non-retried, loud failure; 5xx, timeouts, and
    /// body-read/transport `reqwest` errors retry within the policy budget. A body-read error is
    /// retried on the same terms as a connect failure — the in-flight response is dead and its
    /// bytes are already lost, so a fresh request is the only recovery that can work (see
    /// [`AttemptFailure::NetworkError`]).
    async fn get_bytes_with_retry(
        &self,
        url: &str,
        resource: &str,
    ) -> Result<(Vec<u8>, Option<String>, Option<String>), FetchError> {
        let mut retries: u32 = 0;
        loop {
            match self.request_once(url).await {
                Ok(success) => return Ok(success),
                Err(AttemptFailure::Status(404)) => {
                    return Err(FetchError::Transport(format!(
                        "UT1 returned HTTP 404 for {resource} at {url}: no point retrying"
                    )));
                }
                Err(AttemptFailure::Status(status))
                    if should_retry_status(status, retries, &self.retry) =>
                {
                    let wait = backoff_for(retries, &self.retry);
                    retries += 1;
                    tokio::time::sleep(wait).await;
                }
                Err(failure)
                    if should_retry_transport_failure(&failure, retries, &self.retry) =>
                {
                    let wait = backoff_for(retries, &self.retry);
                    retries += 1;
                    tokio::time::sleep(wait).await;
                }
                Err(failure) => {
                    let detail = failure.describe();
                    return Err(FetchError::Transport(format!(
                        "fetching {url} failed: {detail}"
                    )));
                }
            }
        }
    }

    /// One request: `(bytes, etag, last_modified)` on success, with the headers read before the
    /// body (the body read consumes the response struct).
    async fn request_once(
        &self,
        url: &str,
    ) -> Result<(Vec<u8>, Option<String>, Option<String>), AttemptFailure> {
        let mut resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(AttemptFailure::from_reqwest)?;
        let status = resp.status();
        if !status.is_success() {
            return Err(AttemptFailure::Status(status.as_u16()));
        }
        if let Some(len) = resp.content_length()
            && len > MAX_FETCHED_BYTES
        {
            return Err(AttemptFailure::Transport(format!(
                "response content-length {len} exceeds the {MAX_FETCHED_BYTES}-byte ceiling"
            )));
        }
        let etag = resp
            .headers()
            .get(ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let last_modified = resp
            .headers()
            .get(LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        // `Content-Length` is optional (chunked/HTTP2 responses can omit it), so the check above
        // does not by itself bound the body — read incrementally via `chunk()` (reqwest 0.12) and
        // reject *before* appending a chunk that would cross the ceiling, rather than collecting
        // the whole body first via `bytes()`, which would already have paid the memory cost the
        // ceiling exists to avoid.
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(AttemptFailure::from_reqwest)? {
            if bytes.len() as u64 + chunk.len() as u64 > MAX_FETCHED_BYTES {
                return Err(AttemptFailure::Transport(format!(
                    "response body exceeds the {MAX_FETCHED_BYTES}-byte ceiling"
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((bytes, etag, last_modified))
    }

    /// The license-page request, sharing the same retry policy: the license is fetched fresh on
    /// every category fetch. That is three page fetches per monthly build (one per category
    /// directory); the page is site-global and unchanged, so a caller doing all three categories
    /// in one process *may* fetch it once and reuse — a benchmark question, not a correctness
    /// one, since the page is re-scraped each run either way.
    async fn get_text_with_retry(&self, url: &str) -> Result<String, FetchError> {
        let bytes = self
            .get_bytes_with_retry(url, "the UT1 license page — the only license source this module has")
            .await?
            .0;
        String::from_utf8(bytes).map_err(|e| {
            FetchError::Transport(format!(
                "license page at {url} is not UTF-8 — treating as changed (actual error: {e})"
            ))
        })
    }
}

/// Maps a UT1 archive-extraction failure into the trait's `FetchError`. The `Transport(String)`
/// variant is the crate's designated bucket for "underlying fetch/extract failed" carrying a
/// human-readable cause for the build log.
fn ut1_extract_to_fetch(e: Ut1ExtractError) -> FetchError {
    FetchError::Transport(format!("UT1 archive extraction failed: {e}"))
}

/// Maps a license-scrape failure into the trait's `FetchError`, same bucket as above.
fn ut1_license_to_fetch(e: Ut1LicenseError) -> FetchError {
    FetchError::Transport(format!("UT1 license scrape failed: {e}"))
}

/// What one HTTP attempt returned, split from `reqwest::Error` so retry decisions are readable
/// and testable.
enum AttemptFailure {
    Status(u16),
    Timeout(String),
    /// A `reqwest`-originated I/O failure that is neither a clean timeout nor a connect refusal —
    /// e.g. a TCP reset partway through a multi-megabyte body read (`resp.bytes().await` in
    /// [`Ut1SourceFetcher::request_once`]). Retried on the same budget as a timeout: a body-read
    /// error is a dropped in-flight response whose bytes are already lost, so the only recovery
    /// that can work is a fresh request — the identical reasoning
    /// `fetchers::github::classify_reqwest_error` states for its connect bucket. Deliberately
    /// *not* folded into [`Self::Transport`], which [`Self::from_reqwest`] used to produce for
    /// everything non-timeout/non-connect and `request_once` still uses for its deterministic
    /// size-ceiling rejections (those, unlike this, are never worth retrying).
    NetworkError(String),
    /// A deliberate rejection, not a transient transport fault: the two content-length-ceiling
    /// checks in `request_once` report a too-large payload here. Retrying the same URL cannot
    /// change a payload over the ceiling, so this variant is not retried.
    Transport(String),
}

impl AttemptFailure {
    fn from_reqwest(e: reqwest::Error) -> Self {
        if e.is_timeout() || e.is_connect() {
            Self::Timeout(e.to_string())
        } else {
            Self::NetworkError(e.to_string())
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Status(s) => format!("HTTP {s}"),
            Self::Timeout(d) => d.clone(),
            Self::NetworkError(d) => d.clone(),
            Self::Transport(d) => d.clone(),
        }
    }
}

/// The sync `SourceFetcher` impl. `C::DIR` is the category this fetcher targets; the config's
/// tarball URL must end in `<dir>.tar.gz`.
///
/// `fetch` builds a small current-thread runtime per call and `block_on`s [`fetch_files`]. A
/// caller already inside a Tokio runtime must call `fetch_files` directly — `block_on` from
/// inside a runtime panics, which would turn a wiring choice into a confusing crash.
impl<C: Ut1Category> SourceFetcher for Ut1SourceFetcher<C> {
    fn fetch(&self, config: &SourceConfig) -> Result<FetchedSource, FetchError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|e| FetchError::Transport(format!("failed to start tokio runtime: {e}")))?;
        runtime
            .block_on(self.fetch_files(config))
            .map(|f| f.fetched)
    }
}

/// Pure, independently-testable extraction of `<dir>/domains` from a UT1 `.tar.gz`'s bytes.
///
/// Round-trips the archive in memory (`flate2` gzip → `tar` member → `Vec<u8>`); no disk is
/// ever touched, whether the archive is healthy or not. Member lookup is exact (`<dir>/domains`
/// after normalizing a leading `./`), so the four-member archive from the audit
/// (`domains`/`urls`/`expressions`/`usage`) yields exactly the right file and never a sibling.
pub fn extract_domains_file(
    tar_gz_bytes: &[u8],
    category_dir: &str,
) -> Result<Vec<u8>, Ut1ExtractError> {
    extract_member(tar_gz_bytes, category_dir, "domains")
}

/// Pure extraction of `<dir>/urls`, the sibling whose entry count `count_urls_entries` reports
/// (the plan's documented coverage gap, not a silent drop).
pub fn extract_urls_file(
    tar_gz_bytes: &[u8],
    category_dir: &str,
) -> Result<Vec<u8>, Ut1ExtractError> {
    extract_member(tar_gz_bytes, category_dir, "urls")
}

/// Shared single-member extraction core. Validates the category-directory string before it is
/// interpolated into a member path, verifies the gzip magic, decompresses in memory, and scans
/// the tar entries for an exact `<dir>/member` regular-file match. A missing member is
/// `MemberNotFound`, never an empty `Vec` — an empty `domains` fetch is as loud a signal as a
/// failed one.
fn extract_member(
    tar_gz_bytes: &[u8],
    category_dir: &str,
    member: &'static str,
) -> Result<Vec<u8>, Ut1ExtractError> {
    validate_category_dir(category_dir)?;

    if tar_gz_bytes.len() < 2 || tar_gz_bytes[0] != 0x1f || tar_gz_bytes[1] != 0x8b {
        return Err(Ut1ExtractError::NotGzip);
    }

    let mut archive = tar::Archive::new(GzDecoder::new(tar_gz_bytes));
    let target = format!("{category_dir}/{member}");
    let mut found: Option<Vec<u8>> = None;

    for entry in archive
        .entries()
        .map_err(|e| Ut1ExtractError::TarInvalid(e.to_string()))?
    {
        let entry = entry.map_err(|e| Ut1ExtractError::GzipInvalid(e.to_string()))?;
        let Ok(path) = entry.path() else {
            continue; // non-UTF-8 or pathological path can't be our member
        };
        if normalize_tar_path(&path) != target {
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err(Ut1ExtractError::MemberNotRegularFile {
                category_dir: category_dir.to_string(),
                member,
            });
        }
        // Reserve only what the ceiling allows — the tar header's declared size is untrusted
        // archive data, per MAX_MEMBER_BYTES's doc comment.
        let declared = entry.header().size().unwrap_or(0).min(MAX_MEMBER_BYTES);
        let mut buf = Vec::with_capacity(declared as usize);
        entry
            .take(MAX_MEMBER_BYTES + 1)
            .read_to_end(&mut buf)
            .map_err(|e| Ut1ExtractError::GzipInvalid(e.to_string()))?;
        if buf.len() as u64 > MAX_MEMBER_BYTES {
            return Err(Ut1ExtractError::MemberTooLarge {
                category_dir: category_dir.to_string(),
                member,
            });
        }
        found = Some(buf);
    }

    found.ok_or_else(|| Ut1ExtractError::MemberNotFound {
        category_dir: category_dir.to_string(),
        member,
    })
}

/// Rejects category-directory strings that would be unsafe or meaningless as the first path
/// segment of `/<dir>/domains`. These values come from this module's own type implementors
/// today, but the extraction function is public and testable, so it defends its own inputs.
fn validate_category_dir(dir: &str) -> Result<(), Ut1ExtractError> {
    if dir.is_empty()
        || dir == "."
        || dir == ".."
        || dir.contains('/')
        || dir.contains('\\')
        || dir.starts_with('.')
    {
        Err(Ut1ExtractError::InvalidCategoryDir(dir.to_string()))
    } else {
        Ok(())
    }
}

/// Normalizes an archive entry path for exact matching: keeps only `Normal` components joined
/// by `/` (dropping any leading `./`, `dir/`, and trailing `/`) so comparison is always against
/// `"<dir>/<member>"`. Nothing extracted here is ever written to disk, so `..` components in a
/// hostile member path are a non-match, not a traversal.
fn normalize_tar_path(path: &std::path::Path) -> String {
    use std::path::Component;
    let mut components: Vec<String> = Vec::new();
    for comp in path.components() {
        if let Component::Normal(c) = comp {
            components.push(c.to_string_lossy().into_owned());
        }
    }
    components.join("/")
}

/// Pure, independently-testable license extraction from `dsi.ut-capitole.fr/blacklists/` HTML.
///
/// Two deliberate guards, taken from the assumption audit's two trap findings:
///
/// 1. **Comments are stripped first.** The page embeds a dead CC BY-NC-SA badge inside an
///    `<!-- ... -->` comment; a substring scan for `creativecommons.org/licenses/` would match
///    it and report `LicenseChanged` against a page that hasn't changed. `strip_html_comments`
///    removes `<!--`…`-->` spans before anything else runs.
/// 2. **Only the `rel="license"` anchor's `href` is consulted.** Not a bare license-URL
///    substring search anywhere in the page — [`map_cc_url_to_spdx`] is fed the anchor's href
///    and nothing else, and it fails closed on any CC URL shape it hasn't explicitly enumerated.
pub fn extract_license(html: &str) -> Result<LicenseId, Ut1LicenseError> {
    let clean = strip_html_comments(html);
    let href = find_license_anchor_href(&clean).ok_or(Ut1LicenseError::LicenseAnchorNotFound)?;
    let href = href.ok_or(Ut1LicenseError::LicenseHrefMissing)?;
    map_cc_url_to_spdx(&href).ok_or(Ut1LicenseError::UnsupportedLicenseUrl(href))
}

/// Removes HTML `<!-- ... -->` comment spans. Returns a new `String` so the caller's slice
/// lifetime doesn't couple to the strip. Handles an unterminated comment by dropping to the end
/// — a page whose comment never closes is malformed, and failing closed beats accidentally
/// matching license bytes inside it. Per the HTML spec a comment body must not contain `--`;
/// the scanner treats everything up to the next `-->` as one comment either way, which always
/// errs toward stripping more, never toward surfacing commented text.
pub fn strip_html_comments(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        match rest.find("<!--") {
            None => {
                out.push_str(rest);
                return out;
            }
            Some(start) => {
                out.push_str(&rest[..start]);
                let after_open = &rest[start + 4..];
                match after_open.find("-->") {
                    Some(end) => rest = &after_open[end + 3..],
                    None => return out,
                }
            }
        }
    }
}

/// Finds the `href` of the first `<a ... rel="license" ...>` anchor in document order.
/// Returns `None` for "no anchor on the page" and `Some(None)` for "anchor present but href
/// missing". Attribute order is irrelevant (`rel` may follow `href`); both single- and
/// double-quoted values are accepted.
fn find_license_anchor_href(html: &str) -> Option<Option<String>> {
    let mut pos = 0;
    loop {
        let found = html[pos..].find("<a")?;
        let tag_start = pos + found;
        // Must be a real start tag: `<a` immediately followed by whitespace, `>`, or `/`.
        // `<abbr` and friends fail this check.
        let after_name = tag_start + 2;
        let next = html[after_name..].chars().next()?;
        if !(next.is_whitespace() || next == '>' || next == '/') {
            pos = after_name;
            continue;
        }
        let Some(end_rel) = find_tag_end(&html[after_name..]) else {
            pos = after_name;
            continue;
        };
        let tag_body = &html[after_name..after_name + end_rel];
        let attrs = parse_tag_attributes(tag_body);
        let has_license_rel = attrs.iter().any(|(k, v)| {
            k == "rel" && v.split_whitespace().any(|t| t.eq_ignore_ascii_case("license"))
        });
        if has_license_rel {
            let href = attrs
                .iter()
                .find(|(k, _)| k == "href")
                .map(|(_, v)| v.clone());
            return Some(href);
        }
        pos = after_name + end_rel + 1;
    }
}

/// Index of the tag's closing `>` (quote-aware) in `body`, or `None` if the tag never closes.
fn find_tag_end(body: &str) -> Option<usize> {
    let mut in_quote: Option<char> = None;
    for (idx, ch) in body.char_indices() {
        if let Some(q) = in_quote {
            if ch == q {
                in_quote = None;
            }
        } else if ch == '"' || ch == '\'' {
            in_quote = Some(ch);
        } else if ch == '>' {
            return Some(idx);
        }
    }
    None
}

/// Parses a tag's attribute text into `(lowercased_name, value)` pairs, quote-aware. Values are
/// decoded for the common named entities (`&quot;`, `&amp;`, `&lt;`, `&gt;`, `&apos;`) so an
/// `href` is compared against its dereferenced form.
fn parse_tag_attributes(body: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let name_start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-' || bytes[i] == b'_')
        {
            i += 1;
        }
        if name_start == i {
            i += 1;
            continue;
        }
        let name = body[name_start..i].to_ascii_lowercase();
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let mut value = String::new();
        if j < bytes.len() && bytes[j] == b'=' {
            j += 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                let quote = bytes[j];
                j += 1;
                let start = j;
                while j < bytes.len() && bytes[j] != quote {
                    j += 1;
                }
                value = decode_entities(&body[start..j]);
                if j < bytes.len() {
                    j += 1; // closing quote
                }
            } else {
                let start = j;
                while j < bytes.len() && !bytes[j].is_ascii_whitespace() && bytes[j] != b'>' {
                    j += 1;
                }
                value = decode_entities(&body[start..j]);
            }
        }
        attrs.push((name, value));
        i = j;
    }
    attrs
}

/// Decodes the handful of named entities that can plausibly appear in an attribute value.
fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Creative Commons license codes this module maps to SPDX, paired with the SPDX short-name
/// prefix for each. Any license code *not* listed here — a new CC code, or a host that merely
/// *mentions* creativecommons.org — fails closed via `None`.
const CC_SPEC_DECODES: &[(&str, &str)] = &[
    ("by", "CC-BY-"),
    ("by-sa", "CC-BY-SA-"),
    ("by-nc", "CC-BY-NC-"),
    ("by-nc-sa", "CC-BY-NC-SA-"),
    ("by-nd", "CC-BY-ND-"),
    ("by-nc-nd", "CC-BY-NC-ND-"),
];

/// CC license versions the SPDX map above accepts, and therefore `FetchedSource.license` can
/// legitimately name.
const CC_VERSIONS: &[&str] = &["2.0", "2.5", "3.0", "4.0"];

/// Maps an accepted Creative Commons license URL to its SPDX identifier. Accepted shape:
/// `http(s)://creativecommons.org/licenses/<code>/<version>` with an optional trailing `/` or a
/// trailing `legalcode`/`deed.*` page this module has enumerated. Any other shape (a version
/// this module hasn't pinned, a non-CC host, an unfamiliar suffix) returns `None` — fail closed,
/// never guess.
///
/// The enumerated set is deliberately small and explicit rather than derived:
/// `CC-BY-SA-4.0` is the live license the audit verified, and the whole `{code}×{version}` grid
/// above is exactly what SPDX has stable short identifiers for. A page that moves to something
/// else must produce `UnsupportedLicenseUrl`, not a confidently-wrong `LicenseId`.
pub fn map_cc_url_to_spdx(url: &str) -> Option<LicenseId> {
    let rest = url.split("://").nth(1)?;
    let segments: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    let (host, remainder) = segments.split_first()?;
    // Host must *be* creativecommons.org (or its www/subdomain), not merely contain the string.
    if *host != "creativecommons.org" && !host.ends_with(".creativecommons.org") {
        return None;
    }
    let (licenses, code, version, suffix) = match remainder {
        [licenses, code, version] => (licenses, code, version, None),
        [licenses, code, version, suffix] => (licenses, code, version, Some(*suffix)),
        _ => return None,
    };
    if *licenses != "licenses" {
        return None;
    }
    if let Some(suffix) = suffix
        && suffix != "legalcode"
        && !suffix.starts_with("deed.")
    {
        return None;
    }
    if !CC_VERSIONS.contains(version) {
        return None;
    }
    let prefix = CC_SPEC_DECODES.iter().find(|(c, _)| *c == *code)?.1;
    Some(LicenseId(format!("{prefix}{version}")))
}

/// What `FetchedSource.revision` is for a UT1 fetch. UT1 publishes no revision (the audit's
/// finding); this function prefers the server's own identity headers, else a content hash of the
/// **extracted domains** file — not the tarball, deliberately: a re-archived `.tar.gz` with
/// identical domain content would move a pin for no reason, since gzip's default header embeds
/// mtime. Hashing the member content ties the pin to the only thing the parser consumes.
///
/// The pin-churn tradeoff against `fetch_source`'s exact-match comparison is laid out in
/// `ut1_fetch_notes.md`; the operative rule is that *any* content change hashes or dates
/// differently, so a UT1 change surfaces as `FetchError::RevisionMismatch` and forces the
/// reviewed pin-bump process — the same cost StevenBlack/Hagezi pay per commit.
fn pick_revision(etag: Option<&str>, last_modified: Option<&str>, domains: &[u8]) -> String {
    if let Some(e) = etag.filter(|s| !s.trim().is_empty()) {
        return format!("etag={e}");
    }
    if let Some(lm) = last_modified.filter(|s| !s.trim().is_empty()) {
        return format!("last-modified={lm}");
    }
    let digest = sha2::Sha256::digest(domains);
    format!("sha256={}", hex_encode(&digest))
}

/// Lowercase hex of a byte slice, for the revision fallback. Tiny enough not to warrant a dep.
fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Current unix time in seconds, for `FetchedSource.fetched_at`. Falls back to 0 if the system
/// clock is pre-1970.
pub fn now_timestamp() -> Timestamp {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------
// Tests. Written before the implementation, per this project's test-first rule: the two pure
// functions (`extract_domains_file` / `extract_license`) and their helpers carry the assertions
// the integration pass will keep; nothing here needs a live network fetch.
// ---------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ScopeHint;

    /// Builds a real `.tar.gz` fixture in memory with the `tar` + `flate2` *writer* APIs, the
    /// same way the integration tests should — never a base64 blob whose provenance is opaque.
    fn fixture_tarball(category_dir: &str, domains: &str, urls: &str) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        let gz = flate2::write::GzEncoder::new(&mut tar_bytes, flate2::Compression::default());
        let mut builder = tar::Builder::new(gz);
        append(&mut builder, category_dir, "domains", domains.as_bytes());
        append(&mut builder, category_dir, "urls", urls.as_bytes());
        append(&mut builder, category_dir, "expressions", b"regex\\nlines\n".as_slice());
        append(&mut builder, category_dir, "usage", b"metadata\n".as_slice());
        builder
            .into_inner()
            .expect("builder owns the gz writer")
            .finish()
            .expect("gzip flush");
        tar_bytes
    }

    fn append<W: std::io::Write>(
        builder: &mut tar::Builder<flate2::write::GzEncoder<W>>,
        dir: &str,
        member: &str,
        data: &[u8],
    ) {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        builder
            .append_data(&mut header, format!("{dir}/{member}"), data)
            .expect("append member");
    }

    #[test]
    fn extract_domains_file_reads_the_right_member() {
        let tar = fixture_tarball("adult", "0-12kids.com\n*.boobhub.test\n", "https://x.test/1\n");

        let domains = extract_domains_file(&tar, "adult").unwrap();
        let urls = extract_urls_file(&tar, "adult").unwrap();

        assert_eq!(String::from_utf8(domains).unwrap(), "0-12kids.com\n*.boobhub.test\n");
        assert_eq!(String::from_utf8(urls).unwrap(), "https://x.test/1\n");
    }

    #[test]
    fn extract_domains_file_matches_even_under_a_leading_dot_slash() {
        // Some tar producers write "./adult/domains"; normalize_tar_path must still match.
        let mut tar_bytes = Vec::new();
        let gz = flate2::write::GzEncoder::new(&mut tar_bytes, flate2::Compression::default());
        let mut builder = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(b"example.com\n".len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        builder
            .append_data(&mut header, "./adult/domains", b"example.com\n".as_slice())
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        assert_eq!(
            String::from_utf8(extract_domains_file(&tar_bytes, "adult").unwrap()).unwrap(),
            "example.com\n"
        );
    }

    #[test]
    fn extract_domains_file_can_extract_a_different_category_from_an_archive() {
        let mut tar_bytes = Vec::new();
        let gz = flate2::write::GzEncoder::new(&mut tar_bytes, flate2::Compression::default());
        let mut builder = tar::Builder::new(gz);
        append(&mut builder, "gambling", "domains", b"bet.example\n".as_slice());
        append(&mut builder, "gambling", "urls", b"https://bet.example/\n".as_slice());
        builder.into_inner().unwrap().finish().unwrap();

        assert_eq!(
            String::from_utf8(extract_domains_file(&tar_bytes, "gambling").unwrap()).unwrap(),
            "bet.example\n"
        );
    }

    #[test]
    fn extract_domains_file_fails_loudly_on_garbage_that_is_not_gzip() {
        assert_eq!(
            extract_domains_file(b"not a tar where a tarball should be", "adult"),
            Err(Ut1ExtractError::NotGzip)
        );
    }

    #[test]
    fn extract_domains_file_fails_loudly_when_the_member_is_missing() {
        let mut tar_bytes = Vec::new();
        let gz = flate2::write::GzEncoder::new(&mut tar_bytes, flate2::Compression::default());
        let mut builder = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(b"nothing to see".len() as u64);
        header.set_mode(0o644);
        header.set_mtime(0);
        builder
            .append_data(&mut header, "./other/countries", b"nothing to see".as_slice())
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        assert_eq!(
            extract_domains_file(&tar_bytes, "adult"),
            Err(Ut1ExtractError::MemberNotFound {
                category_dir: "adult".to_string(),
                member: "domains",
            })
        );
    }

    #[test]
    fn extract_domains_file_rejects_a_member_over_the_size_ceiling() {
        // Real (compressible, so the fixture stays cheap to build) data past MAX_MEMBER_BYTES —
        // the tar header's declared size matches, so this exercises the post-read check, not the
        // reservation clamp alone.
        let oversized = vec![b'a'; (MAX_MEMBER_BYTES + 1) as usize];
        let mut tar_bytes = Vec::new();
        let gz = flate2::write::GzEncoder::new(&mut tar_bytes, flate2::Compression::fast());
        let mut builder = tar::Builder::new(gz);
        append(&mut builder, "adult", "domains", &oversized);
        builder.into_inner().unwrap().finish().unwrap();

        assert_eq!(
            extract_domains_file(&tar_bytes, "adult"),
            Err(Ut1ExtractError::MemberTooLarge {
                category_dir: "adult".to_string(),
                member: "domains",
            })
        );
    }

    #[test]
    fn extract_domains_file_rejects_an_unsafe_category_dir_before_any_archive_work() {
        assert_eq!(
            extract_domains_file(&fixture_tarball("adult", "a\n", "b\n"), "../adult"),
            Err(Ut1ExtractError::InvalidCategoryDir("../adult".to_string()))
        );
        assert_eq!(
            extract_domains_file(&fixture_tarball("adult", "a\n", "b\n"), ""),
            Err(Ut1ExtractError::InvalidCategoryDir("".to_string()))
        );
    }

    #[test]
    fn extract_domains_file_wrong_category_is_member_not_found() {
        // Fetched the `adult` tarball with a `Gambling` fetcher: loud, specific failure.
        let tar = fixture_tarball("adult", "x.test\n", "url\n");
        assert_eq!(
            extract_domains_file(&tar, "gambling"),
            Err(Ut1ExtractError::MemberNotFound {
                category_dir: "gambling".to_string(),
                member: "domains",
            })
        );
    }

    /// A fixture shaped like the real page, per the audit: a live `rel="license"` anchor to CC
    /// BY-SA 4.0, plus a dead courtesy badge for CC BY-NC-SA 4.0 sitting inside an HTML comment.
    const LIVE_LIKE_HTML: &str = r#"<!DOCTYPE html>
<html><head><title>Index of /blacklists</title></head>
<body>
<h1>UT1 blacklists</h1>
  <p>Blocklists of students and research.</p>
  <a href="https://dsi.ut-capitole.fr/blacklists/"><img src="logo"/></a>
  <footer>
    <a rel="license" href="https://creativecommons.org/licenses/by-sa/4.0/">
      <img alt="Creative Commons License" style="border-width:0"
           src="https://i.creativecommons.org/l/by-sa/4.0/88x31.png"/>
    </a>
  </footer>
  <!--
    <div typeof="rdf:RDF" xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
      <span rel="rdf:license" href="https://creativecommons.org/licenses/by-nc-sa/4.0/">
        This work is licensed under a Creative Commons Attribution-NonCommercial-ShareAlike 4.0
      </span>
    </div>
  -->
</body></html>"#;

    #[test]
    fn extract_license_reads_by_sa_from_the_live_anchor_and_not_the_commented_nc_block() {
        let license = extract_license(LIVE_LIKE_HTML).unwrap();
        assert_eq!(license, LicenseId("CC-BY-SA-4.0".to_string()));
    }

    #[test]
    fn extract_license_fails_cleanly_when_there_is_no_rel_license_anchor() {
        let html = "<html><body>Nothing here has a license.</body></html>";
        assert_eq!(
            extract_license(html),
            Err(Ut1LicenseError::LicenseAnchorNotFound)
        );
    }

    #[test]
    fn extract_license_fails_closed_on_an_unmapped_cc_url_shape() {
        let html = r#"
            <a rel="license" href="https://creativecommons.org/licenses/by-sa/9.9/">
            "#
        .to_string();
        assert_eq!(
            extract_license(&html),
            Err(Ut1LicenseError::UnsupportedLicenseUrl(
                "https://creativecommons.org/licenses/by-sa/9.9/".to_string()
            ))
        );
    }

    #[test]
    fn extract_license_fails_closed_on_a_license_url_that_is_not_cc() {
        let html = r#"<a rel="license" href="https://example.com/not-a-cc-license">"#.to_string();
        assert_eq!(
            extract_license(&html),
            Err(Ut1LicenseError::UnsupportedLicenseUrl(
                "https://example.com/not-a-cc-license".to_string()
            ))
        );
    }

    #[test]
    fn extract_license_fails_when_anchor_has_rel_license_but_no_href() {
        let html = r#"<a rel="license"><img src="x"/></a>"#.to_string();
        assert_eq!(
            extract_license(&html),
            Err(Ut1LicenseError::LicenseHrefMissing)
        );
    }

    #[test]
    fn strip_html_comments_removes_the_nc_block_but_keeps_surrounding_live_html() {
        let cleaned = strip_html_comments(LIVE_LIKE_HTML);
        assert!(cleaned.contains("rel=\"license\""));
        assert!(cleaned.contains("by-sa/4.0"));
        assert!(!cleaned.contains("by-nc-sa"));
    }

    #[test]
    fn strip_html_comments_handles_unterminated_and_adjacent_comments() {
        assert_eq!(strip_html_comments("<p>a</p><!-- never closed"), "<p>a</p>");
        assert_eq!(strip_html_comments("a<!-- x -->b<!-- y -->c"), "abc");
        // A comment body containing `--` is illegal HTML; the scanner errs toward stripping it.
        assert_eq!(strip_html_comments("a<!-- x--y -->b"), "ab");
    }

    #[test]
    fn map_cc_url_to_spdx_maps_the_known_by_sa_shape_and_fails_closed_elsewhere() {
        assert!(
            map_cc_url_to_spdx("https://creativecommons.org/licenses/by-sa/4.0/")
                .is_some_and(|l| l.0 == "CC-BY-SA-4.0")
        );
        assert!(
            map_cc_url_to_spdx("http://creativecommons.org/licenses/by-sa/4.0/legalcode")
                .is_some_and(|l| l.0 == "CC-BY-SA-4.0")
        );
        assert!(
            map_cc_url_to_spdx("https://creativecommons.org/licenses/by-sa/4.0/deed.en")
                .is_some_and(|l| l.0 == "CC-BY-SA-4.0")
        );
        // A version we haven't enumerated — fail closed, not guessed.
        assert_eq!(map_cc_url_to_spdx("https://creativecommons.org/licenses/by-sa/5.0/"), None);
        // A non-CC host and a non-licenses path.
        assert_eq!(map_cc_url_to_spdx("https://not-cc.org/licenses/by-sa/4.0/"), None);
        assert_eq!(map_cc_url_to_spdx("https://creativecommons.org/static/logo.png"), None);
        // Host matching by suffix, not a bare substring anywhere.
        assert_eq!(
            map_cc_url_to_spdx("https://evil.com/creativecommons.org/licenses/by-sa/4.0/"),
            None
        );
    }

    #[test]
    fn pick_revision_prefers_etag_then_last_modified_then_content_hash() {
        let domains = b"example.com\n";
        let etag = "W/\"abc123\"";
        let lastmod = "Wed, 21 Oct 2015 07:28:00 GMT";
        assert_eq!(
            pick_revision(Some(etag), Some(lastmod), domains),
            format!("etag={etag}")
        );
        assert_eq!(
            pick_revision(None, Some(lastmod), domains),
            format!("last-modified={lastmod}")
        );
        let hash = pick_revision(None, None, domains);
        assert!(hash.starts_with("sha256="), "revision was {hash}");
        // Two identical domain contents hash identically — pin stability is the point.
        assert_eq!(hash, pick_revision(None, None, b"example.com\n"));
        // A one-domain change moves the hash — the pin-churn trigger.
        assert_ne!(hash, pick_revision(None, None, b"example.org\n"));
    }

    #[test]
    fn should_retry_status_retries_5xx_not_404_and_respects_the_budget() {
        let policy = Ut1RetryPolicy::default();
        assert!(should_retry_status(500, 0, &policy));
        assert!(should_retry_status(503, 1, &policy)); // second retry fits (max 3 total)
        assert!(!should_retry_status(503, 2, &policy)); // budget exhausted (max 3 total)
        assert!(!should_retry_status(404, 0, &policy));
        assert!(!should_retry_status(400, 0, &policy));
        assert!(!should_retry_status(199, 0, &policy));
    }

    #[test]
    fn should_retry_status_lets_the_second_attempt_retry() {
        let policy = Ut1RetryPolicy { max_attempts: 3, ..Default::default() };
        // retries_so_far < max_attempts - 1 = 2; retries 0 and 1 both allowed.
        assert!(should_retry_status(503, 0, &policy));
        assert!(should_retry_status(503, 1, &policy));
        assert!(!should_retry_status(503, 2, &policy));
    }

    #[test]
    fn a_mid_body_network_error_is_retried_on_the_same_budget_as_a_timeout() {
        let policy = Ut1RetryPolicy { max_attempts: 3, ..Default::default() };
        let mid_body = AttemptFailure::NetworkError("mock: TCP reset mid-body read".to_string());
        let timeout = AttemptFailure::Timeout("mock: request timed out".to_string());

        // First and second failures earn a retry, budgeted identically to Timeout.
        assert!(should_retry_transport_failure(&mid_body, 0, &policy));
        assert!(should_retry_transport_failure(&mid_body, 1, &policy));
        assert!(should_retry_transport_failure(&timeout, 0, &policy));
        // Budget exhausted (max 3 total): no more retries, exactly like Timeout.
        assert!(!should_retry_transport_failure(&mid_body, 2, &policy));
        assert!(!should_retry_transport_failure(&timeout, 2, &policy));

        // A deliberate size-ceiling rejection is not retried (same URL, same answer).
        let too_big = AttemptFailure::Transport(
            "response body 999 exceeds the MAX_FETCHED_BYTES-byte ceiling".to_string(),
        );
        assert!(!should_retry_transport_failure(&too_big, 0, &policy));
        // An HTTP status is decided by should_retry_status, never by this transport rule.
        assert!(!should_retry_transport_failure(&AttemptFailure::Status(503), 0, &policy));

        // describe() renders the new variant for the build log without panicking.
        assert_eq!(mid_body.describe(), "mock: TCP reset mid-body read");
    }

    #[test]
    fn backoff_for_grows_exponentially_and_is_capped() {
        let policy = Ut1RetryPolicy::default();
        assert_eq!(backoff_for(0, &policy), Duration::from_secs(2));
        assert_eq!(backoff_for(1, &policy), Duration::from_secs(4));
        assert_eq!(backoff_for(2, &policy), Duration::from_secs(8));
        assert_eq!(backoff_for(50, &policy), Duration::from_secs(8)); // capped at max_backoff
    }

    #[test]
    fn verify_config_category_rejects_wrong_source_and_wrong_tarball_name() {
        let good = SourceConfig {
            source: SourceId::Ut1,
            url: "https://dsi.ut-capitole.fr/blacklists/download/adult.tar.gz".into(),
            pinned_revision: "etag=whatever".into(),
            expected_license: LicenseId("CC-BY-SA-4.0".into()),
        };

        assert!(Ut1SourceFetcher::<Adult>::verify_config_category(&good).is_ok());

        let wrong_source = SourceConfig {
            source: SourceId::StevenBlack,
            url: good.url.clone(),
            pinned_revision: good.pinned_revision.clone(),
            expected_license: good.expected_license.clone(),
        };
        assert!(matches!(
            Ut1SourceFetcher::<Adult>::verify_config_category(&wrong_source),
            Err(FetchError::Transport(_))
        ));

        let wrong_tarball = SourceConfig {
            source: SourceId::Ut1,
            url: "https://dsi.ut-capitole.fr/blacklists/download/gambling.tar.gz".into(),
            pinned_revision: good.pinned_revision.clone(),
            expected_license: good.expected_license.clone(),
        };
        assert!(matches!(
            Ut1SourceFetcher::<Adult>::verify_config_category(&wrong_tarball),
            Err(FetchError::Transport(_))
        ));
    }

    #[test]
    fn extract_and_then_parse_domains_round_trips_through_ut1_parse() {
        use crate::sources::ut1;
        let tar = fixture_tarball(
            "adult",
            "# UT1 adult\n0-12kids.com\n*.boobhub.test\n",
            "https://0-12kids.com/a\n",
        );
        let domains = extract_domains_file(&tar, "adult").unwrap();
        let urls = extract_urls_file(&tar, "adult").unwrap();

        let parsed = ut1::parse_domains(&domains, Category::Adult);
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].domain, "0-12kids.com");
        assert_eq!(parsed.entries[1].domain, "boobhub.test");
        assert_eq!(parsed.entries[1].scope_hint, ScopeHint::Apex);

        assert_eq!(count_urls_entries(&urls), 1);
    }
}