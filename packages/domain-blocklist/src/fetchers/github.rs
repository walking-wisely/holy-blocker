//! DRAFT — module 7 (`cli`, unbuilt) GitHub-backed fetcher for the two GitHub-hosted sources.
//!
//! Integration pathway: this file is written to be copied in as
//! `packages/domain-blocklist/src/fetchers/github.rs` with a `pub mod fetchers;` line added to
//! `src/lib.rs`. It does not move any code in `sources/` and does not alter the `SourceConfig` /
//! `SourceFetcher` / `FetchError` contract in `sources/mod.rs`.
//!
//! Why this exists and what it fetches, per the plan's `### 7. cli` assumption audit
//! (`docs/components/domain-blocklist/plan.md`, run 2026-08-15):
//!
//! - Neither StevenBlack's `hosts` file nor Hagezi's list files carry an in-band license or
//!   revision string, so `FetchedSource.revision` / `.license` come from GitHub's **REST API**,
//!   never from parsing the fetched list body.
//! - `SourceConfig.pinned_revision` is a commit SHA baked into the `raw.githubusercontent.com` URL
//!   path (e.g. `.../StevenBlack/hosts/<sha>/alternates/porn-only/hosts`), never a branch name — a
//!   branch HEAD moves out from under a pin. Because the pin is content-addressed, the only way a
//!   build can serve drifted content is a config that pinned something else; the revision check in
//!   [`fetch_source`] still catches that, per this module's `revision` resolution below.
//! - Hagezi's crate-relevant content lives at `wildcard/nsfw-onlydomains.txt` (plain-domain
//!   format); `adblock/nsfw.txt` is AdBlock filter syntax (`||domain^`) and would parse to
//!   all-`dropped_malformed`. Both facts verified live 2026-08-15, see
//!   `github_fetch_notes.md`.
//!
//! The fetcher is deliberately transport-only: it produces a [`FetchedSource`] (revision,
//! license, bytes). The revision/license *policy* — `RevisionMismatch`, `LicenseChanged`,
//! `LicenseNotAllowed`, allowlist — stays in [`crate::sources::fetch_source`], unchanged, exactly
//! as that function's doc comment intends. This module therefore defines a *richer* transport-side
//! error ([`GithubFetchError`]) so named failures (rate-limit exhaustion, malformed JSON, an
//! HTTP status that retrying cannot fix) survive to the build log, and maps it down to the crate's
//! opaque [`FetchError::Transport`] at the trait boundary — see [`GithubSourceFetcher`].

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::HeaderMap;

use crate::sources::{FetchError, FetchedSource, SourceConfig, SourceFetcher};
use crate::types::{LicenseId, SourceId, Timestamp};

// ---------------------------------------------------------------------------
// HTTP abstraction
// ---------------------------------------------------------------------------

/// One HTTP response the fetcher actually acts on: status, headers (for the rate-limit
/// contract), body. The fields are detached from `reqwest::Response` on purpose so the trait is
/// implementable by a test fake without touching the network — the real `reqwest::Client` code
/// lives in [`ReqwestHttp`]; a synchronous test harness lives in this module's test folder.
#[derive(Debug, Clone)]
pub struct GithubHttpResponse {
    /// HTTP status code as an integer (202 — a tri-digit `StatusCode` is a claim about the future
    /// this module doesn't need).
    pub status: u16,
    /// All response headers. Header names are matched case-insensitively by `HeaderMap`, which
    /// matters because GitHub's API answers HTTP/2 with lowercase header names
    /// (`x-ratelimit-remaining` was observed live 2026-08-15) while its docs and every curl
    /// example write `X-RateLimit-Remaining`. The HTTP spec mandadutores case-insensitivity
    /// (RFC 9110 §5.2) and `HeaderMap` normalizes for us.
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

/// A transport error from the HTTP layer. Every variant in here is, by construction, a candidate
/// for a bounded retry — the *caller* (`get_with_retry`) owns the retry decision, not the HTTP
/// layer, so the two sides of the "what is retryable" question stay in separate files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubHttpError {
    /// Could not establish or maintain the connection (connect refused, DNS failure, TLS
    /// handshake dropped, mid-body read failure). A hung TCP connection surfaces here as a
    /// `Timeout` only if it outlasted the client's per-request timeout — the one real protection
    /// against blocking the whole build forever (see [`RetryPolicy`]).
    Connect(String),
    /// The client's configured per-request timeout fired before the response body was complete.
    Timeout(String),
}

impl std::fmt::Display for GithubHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GithubHttpError::Connect(msg) => write!(f, "github transport error: {msg}"),
            GithubHttpError::Timeout(msg) => write!(f, "github request timed out: {msg}"),
        }
    }
}

/// The seam the module is written against so the retry/classification logic can be tested without
/// a network. Chosen over `wiremock` (see `github_fetch_notes.md`, "Why a trait, not `wiremock`"):
/// a scripted fake can sequence responses *and* count attempts, which the retry tests need, with
/// zero extra dev-dependencies.
///
/// Written as a native `async fn` in a trait (stable since Rust 1.75; this repo is on Rust 1.96).
/// The returned future is deliberately not forced `Send`: the sync bridge runs it under
/// `Runtime::block_on`, which does not require `Send` (`tokio::runtime::Runtime::block_on` has no
/// `F: Send` bound), and the per-fetch call sites never spawn these futures onto another thread. A
/// multi-thread pipeline that wants to parallelize the three fetches would need a `Send`-audited
/// variant — a `trait_variant::make`/`async-trait` `+ Send` copy, or an `Arc`-owned http layer —
/// flagged in the notes as the trade of the day.
/// `#[allow(async_fn_in_trait)]` is intentional: the lint exists to warn that `async fn` in a
/// public trait cannot name auto-trait bounds (`Send`) on the returned future. That is exactly
/// this trait's point — futures are deliberately not `Send`-bound (see the doc comment) — so the
/// warning carries the usual "only use in your own code" note and the repo's tools are free to
/// lint the call sites instead.
#[allow(async_fn_in_trait)]
pub trait GithubHttp: Send + Sync {
    /// Perform a single GET. No retries and no classification happen here — those are
    /// [`GithubSourceFetcher::get_with_retry`]'s job, so the fake only needs to script *raw*
    /// outcomes.
    async fn get(&self, url: &str) -> Result<GithubHttpResponse, GithubHttpError>;
}

/// The production `GithubHttp`. Builds a `reqwest::Client` with:
///
/// - a per-request total timeout of 60 s and a 10 s connect timeout. The total timeout is the
///   hang-catcher the plan's "a hung TCP connection must not block the whole build forever" rule
///   is about: GitHub API responses here measure ~1–3 KB and the largest raw file ~2.9 MB
///   (StevenBlack root `hosts`, observed live 2026-08-15), both trivially inside 60 s on any CI,
///   while a dead connection at least fails by 60 s instead of hanging the build. See
///   [`RetryPolicy`] for how the bounded retry multiplies that worst case.
/// - a `User-Agent`. GitHub's API rejects requests with none — "all API requests must include a
///   valid `User-Agent` header" is a documented requirement
///   (<https://docs.github.com/en/rest/using-the-rest-api/getting-started-with-the-rest-api>).
pub struct ReqwestHttp {
    client: reqwest::Client,
}

/// The `User-Agent` sent on every request. A GitHub-hosted fork running this pipeline should swap
/// in its own identifier; the value only needs to be non-empty and distinctive enough not to be
/// mistaken for a bot gate trigger.
const USER_AGENT: &str = concat!("holy-blocker/", env!("CARGO_PKG_VERSION"));

impl ReqwestHttp {
    pub fn new() -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
            .user_agent(USER_AGENT)
            .build()?;
        Ok(Self { client })
    }
}

impl GithubHttp for ReqwestHttp {
    async fn get(&self, url: &str) -> Result<GithubHttpResponse, GithubHttpError> {
        // Note: `.send()` is what the per-request timeout applies to, exactly once per call; a
        // timed-out attempt is an `Err` here and the caller's retry loop re-issues the request
        // under a fresh timeout rather than extending the dead one.
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| classify_reqwest_error(&e))?;
        let status = resp.status().as_u16();
        let headers = resp.headers().clone();
        let body = resp
            .bytes()
            .await
            .map_err(|e| classify_reqwest_error(&e))?;
        Ok(GithubHttpResponse {
            status,
            headers,
            body: body.to_vec(),
        })
    }
}

/// Maps a `reqwest::Error` onto the two-shot [`GithubHttpError`] taxonomy. Anything that is not a
/// timeout is reported as a connect-style failure (including a mid-body read error, whose
/// recovery semantics — the bytes are already lost, retry the request — are identical to a
/// dropped connection).
fn classify_reqwest_error(e: &reqwest::Error) -> GithubHttpError {
    if e.is_timeout() {
        GithubHttpError::Timeout(e.to_string())
    } else {
        GithubHttpError::Connect(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Repo coordinates
// ---------------------------------------------------------------------------

/// Repo coordinates plus the path-within-repo to fetch the list content from. This is the piece
/// a plain `SourceConfig` cannot carry: `SourceConfig.url` is one URL, but a GitHub source needs
/// *two* different hosts (`api.github.com` for revision/license, `raw.githubusercontent.com` for
/// content) plus a path under the pinned SHA. So the fetcher maps a [`SourceId`] to this struct —
/// the decision is written down in `github_fetch_notes.md` under "How the fetcher learns owner/
/// repo/path".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubRepo {
    pub owner: String,
    pub repo: String,
    /// Path of the list file within the repo, rendered under
    /// `raw.githubusercontent.com/{owner}/{repo}/{sha}/{path}`. Hagezi: `wildcard/nsfw-onlydomains.txt`.
    /// StevenBlack: `alternates/porn-only/hosts` — see the module doc for why not the root `hosts`.
    pub raw_path: String,
}

impl GithubRepo {
    /// StevenBlack's `porn` extension — the list this project's `SourceConfig` for
    /// [`SourceId::StevenBlack`] pins, per `sources/stevenblack.rs`'s doc comment ("the `porn`
    /// extension specifically"). Deliberately *not* the repo-root `hosts` file: that file is the
    /// unified list and also carries ad/malware/tracking entries (~2.9 MB, observed live
    /// 2026-08-15) which this crate's single-category parser would wrongly tag `Adult`;
    /// `alternates/porn-only/hosts` is ~1.96 MB and its `# Title: ... extension porn` header
    /// confirms porn-only scope. Both are live; the config must pick this one.
    pub fn stevenblack() -> Self {
        Self {
            owner: "StevenBlack".to_string(),
            repo: "hosts".to_string(),
            raw_path: "alternates/porn-only/hosts".to_string(),
        }
    }

    /// Hagezi's NSFW list — `wildcard/nsfw-onlydomains.txt`. Verified live 2026-08-15 by listing
    /// the repo tree
    /// (`GET https://api.github.com/repos/hagezi/dns-blocklists/git/trees/main?recursive=1`):
    /// the `wildcard/*-onlydomains.txt` entry whose `adblock/` sibling is named `nsfw` is
    /// `wildcard/nsfw-onlydomains.txt`. The sibling `adblock/nsfw.txt` is AdBlock filter syntax —
    /// the wrong file entirely (see plan audit, "Hagezi's list content" row).
    pub fn hagezi_nsfw() -> Self {
        Self {
            owner: "hagezi".to_string(),
            repo: "dns-blocklists".to_string(),
            raw_path: "wildcard/nsfw-onlydomains.txt".to_string(),
        }
    }
}

/// Resolves a [`SourceId`] to its GitHub repo coordinates, or `None` for a source this fetcher
/// does not serve (UT1 lives on `dsi.ut-capitole.fr`, has no GitHub API, and gets its own
/// `SourceFetcher` in module 7 — fetch_source is what picks which fetcher, so this `None` case is
/// only reachable through a caller wiring mistake and still fails loudly).
pub trait GithubRepos: Send + Sync {
    fn repo_for(&self, source: SourceId) -> Option<&GithubRepo>;
}

/// A static registry mapping [`SourceId`] to [`GithubRepo`]. The checked-in source configuration
/// (the real `SourceConfig` values with their pinned SHAs and `expected_license`) is the
/// integration pass's job to add; this type just needs to be populated from wherever those configs
/// live so the fetcher can be constructed with one `Vec` instead of a separate match arm per
/// source. Public so tests and the eventual CLI can register exactly the sources they mean.
pub struct StaticGithubRepos {
    entries: Vec<(SourceId, GithubRepo)>,
}

impl StaticGithubRepos {
    pub fn new(entries: impl IntoIterator<Item = (SourceId, GithubRepo)>) -> Self {
        Self {
            entries: entries.into_iter().collect(),
        }
    }

    pub fn with(mut self, source: SourceId, repo: GithubRepo) -> Self {
        self.entries.push((source, repo));
        self
    }

    /// The two sources this fetcher serves today, with the paths verified live 2026-08-15. The
    /// integration pass should replace this with the checked-in config's values rather than
    /// trusting these defaults blindly — it exists so the draft has a known-good starting point.
    pub fn the_two_github_sources() -> Self {
        Self::new([
            (SourceId::StevenBlack, GithubRepo::stevenblack()),
            (SourceId::Hagezi, GithubRepo::hagezi_nsfw()),
        ])
    }
}

impl GithubRepos for StaticGithubRepos {
    fn repo_for(&self, source: SourceId) -> Option<&GithubRepo> {
        self.entries
            .iter()
            .find(|(s, _)| *s == source)
            .map(|(_, repo)| repo)
    }
}

// ---------------------------------------------------------------------------
// Fetch error taxonomy
// ---------------------------------------------------------------------------

/// Why a GitHub fetch didn't produce bytes a parser is allowed to see. This is the *transport*
/// error type — the policy errors (`RevisionMismatch`, `LicenseChanged`, `LicenseNotAllowed`)
/// live in [`crate::sources::fetch_source`] and are deliberately not re-represented here. Its job
/// is to keep the failures this fetch layer can actually observe *named*, so the listener can
/// tell "build will not run today, quota exhausted" from "transient 503, worth a rerun" without a
/// string match.
///
/// "Which failures are worth retrying" is answered structurally by the variants themselves:
///
/// | Retried | Not retried | Why |
/// |---|---|---|
/// | [`GithubHttpError`] (connect/timeout) | — | a dead network is transient by definition |
/// | HTTP 500/502/503/504 | — | GitHub's own docs describe these as
///   server-availability errors (<https://docs.github.com/en/rest/guides/best-practices-for-using-the-rest-api>) |
/// | HTTP 429 | — | secondary rate limit / abuse gate; documented as transient-shaped |
/// | [`GithubFetchError::RateLimit`] **when the reset is within the retry horizon** | [`GithubFetchError::RateLimit`] **when it is not** | a 403 with `X-RateLimit-Remaining: 0` can only be cured by waiting until
///   `X-RateLimit-Reset`, so it is only retried when that wait fits the bounded backoff budget —
///   see [`GithubSourceFetcher::get_with_retry`]. Burned-once, never burned-forever. |
/// | — | HTTP 404 (missing file/repo/ref) | the pin is a real SHA; 404 means the repo or path
///   moved/died and a retry will not resurrect it |
/// | — | HTTP 401 / 403 without rate-limit exhaustion | an auth/permission problem is a
///   configuration error, not a transient condition |
/// | — | malformed JSON body | the endpoint's contract broke or the response is garbage;
///   re-requesting the same URL will not fix that |
///
/// See the notes file's "Retry classification" for the full rationale and the reconciliation with
/// the task's two rate-limit constraints (retryable-list vs. named-error).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubFetchError {
    /// A transport failure that survived the bounded retry (or a retry-ineligible transport
    /// failure). The string is for the build log, not a matchable contract.
    Transport(String),
    /// GitHub's unauthenticated REST quota exhausted: HTTP 403 with `X-RateLimit-Remaining: 0`.
    /// `reset_at` is the epoch-seconds `X-RateLimit-Reset` value, or `0` if the header was absent
    /// (in which case the error says so in its `Display`). Named, rather than lumped into
    /// [`Self::Transport`], so the build can say exactly why it cannot run today.
    RateLimit { reset_at: Timestamp },
    /// An HTTP status this module will not retry: 404, 401, a permission-shaped 403, a 4xx the
    /// server will keep repeating, etc. `url` and a (truncated) `preview` of the response body,
    /// which for GitHub normally carries a one-line `message` — e.g. the license endpoint's
    /// `Not Found` body observed live 2026-08-15 on [`octocat/Hello-World`].
    Http {
        status: u16,
        url: String,
        preview: String,
    },
    /// A response body that did not deserialize into the endpoint's documented shape. Never
    /// retried: same URL, same body.
    Json { url: String, reason: String },
    /// `repo_for` answered `None` for a source. A wiring mistake, and a quiet `FetchedSource` is
    /// the wrong failure mode for one.
    UnconfiguredSource(SourceId),
}

impl std::fmt::Display for GithubFetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GithubFetchError::Transport(msg) => write!(f, "github fetch transport failure: {msg}"),
            GithubFetchError::RateLimit { reset_at } => write!(
                f,
                "github API rate limit exhausted (unauthenticated REST quota, 60/hr, observed live \
                 2026-08-15); reset at epoch seconds {reset_at}{}",
                if *reset_at == 0 { " (reset header absent)" } else { "" }
            ),
            GithubFetchError::Http { status, url, preview } => {
                write!(f, "GET {url} -> HTTP {status}{}", if preview.is_empty() { String::new() } else { format!(": {preview}") })
            }
            GithubFetchError::Json { url, reason } => {
                write!(f, "GET {url}: response was not the documented JSON shape: {reason}")
            }
            GithubFetchError::UnconfiguredSource(source) => {
                write!(f, "no GitHub repo coordinates registered for source {source:?}")
            }
        }
    }
}

/// Maps the fetcher's rich transport error onto the crate's single opaque [`FetchError::Transport`]
/// at the trait boundary. The policy variants of `FetchError` are never produced here — only
/// [`crate::sources::fetch_source`] does that. The `RateLimit` message keeps the named shape: a
/// failed build's log says "rate limit" with a reset time, not "some transport thing failed".
/// The integration pass may extend `FetchError` itself with a `RateLimit` variant; this `From`
/// impl is the fallback that keeps the existing trait contract untouched meanwhile.
impl From<GithubFetchError> for FetchError {
    fn from(e: GithubFetchError) -> Self {
        // Every variant's Display already names it ("rate limit...", "GET <url> -> HTTP 404: ...");
        // the opaque string keeps that shape in the build log instead of collapsing it.
        FetchError::Transport(e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Revision and license value helpers
// ---------------------------------------------------------------------------

/// A canonical 40-hex commit SHA as reported *by GitHub's API* for a ref. Distinct from the
/// config's `pinned_revision` string so it is obvious which value came from the wire and which
/// from checked-in configuration — `fetch_source` compares them and emits
/// `FetchError::RevisionMismatch` when they differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubRevision(pub String);

/// The SPDX identifier for a repo as reported by GitHub's license API. The sentinel value is
/// [`NOASSERTION`][Self::NOASSERTION], covering the three cases where GitHub has no SPDX id to
/// give — the API's own `"NOASSERTION"` string (a LICENSE file GitHub detected but could not
/// classify; observed live 2026-08-15 on `fair-test/e2e-license-noassertion` → HTTP 200 with
/// `license.spdx_id == "NOASSERTION"`), a `null`/absent `spdx_id` inside a 200, and the HTTP 404
/// "no license detectable" answer (observed live 2026-08-15 on `octocat/Hello-World` → HTTP 404).
/// `NOASSERTION` is the SPDX expression for "no license assertion", so reusing the string means a
/// license comparison that fails on a genuinely unlicensed upstream does so through the *same*
/// value the API denotes an unclassifiable-one with — and it is a plain [`LicenseId`] the caller
/// can compare, which is the point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubLicense(pub LicenseId);

impl GithubLicense {
    pub const NOASSERTION: &'static str = "NOASSERTION";

    /// `true` when the resolved license is the no-assertion sentinel.
    pub fn is_noassertion(&self) -> bool {
        self.0.spdx_matches(&LicenseId(Self::NOASSERTION.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Retry policy
// ---------------------------------------------------------------------------

/// The bounded retry budget. The whole point of bounding it is the plan's "a failed fetch aborts
/// the whole build ... not a hang" rule: exponential backoff with a cap and a small attempt count
/// keeps a genuinely sick GitHub or edge network from wedging the monthly build, while still
/// riding out the blips a real content-CDN goes through.
///
/// Defaults (all configurable): 4 attempts, 500 ms base backoff doubling per attempt
/// (0.5 s → 1 s → 2 s → 4 s) capped at 8 s — about 7.5 s of worst-case sleep per call, and the
/// fetch of one source makes at most ~3 calls (revision, license, content). A rate-limited 403 is
/// only waited out when `X-RateLimit-Reset` is within `max_backoff` (a few seconds out); the
/// primary unauthenticated quota resets hourly, so the usual answer is the named
/// [`GithubFetchError::RateLimit`] and a logged failure, never a minutes-long sleep inside the
/// retry loop.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Total attempts including the first. `1` disables retrying entirely for callers that want a
    /// strict single-shot fetch.
    pub max_attempts: u32,
    /// First retry's delay; each later retry doubles it up to `max_backoff`.
    pub base_backoff: Duration,
    /// Cap on any single backoff sleep (and on the wait-for-rate-reset sleep).
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(8),
        }
    }
}

impl RetryPolicy {
    /// The delay to sleep before attempt number `attempt` (0-based). Exponential: `base * 2^attempt`,
    /// capped. The shift is clamped to 10 so a pathological near-u32 attempt count cannot overflow.
    fn backoff_for(&self, attempt: u32) -> Duration {
        let exp = self.base_backoff.saturating_mul(1u32 << attempt.min(10));
        exp.min(self.max_backoff)
    }
}

/// What a single response's status+headers mean for the retry loop. Kept as an enum so the
/// classification function is a pure mapping and the loop is just a `match`, which is the whole
/// testable surface of the retry story.
#[derive(Debug)]
enum RemoteOutcome {
    Ok,
    Retry,
    /// A 403 with `X-RateLimit-Remaining: 0` whose reset lies *inside* the retry horizon: the
    /// quota cannot un-exhaust itself before then, so retrying on the ordinary exponential
    /// backoff schedule would just re-request into the same 403 until the attempt budget runs
    /// out. `delay` is the wait that actually clears the reset (not `RetryPolicy::backoff_for`'s
    /// schedule); `reset_at` is carried through so the terminal error, if the budget still runs
    /// out, can name the quota as the cause via [`GithubFetchError::RateLimit`] rather than a
    /// generic exhausted-retries [`GithubFetchError::Transport`].
    RetryAfter {
        delay: Duration,
        reset_at: Timestamp,
    },
    /// HTTP 403 with `X-RateLimit-Remaining: 0` whose reset lies outside the retry horizon.
    /// Carries the epoch-seconds reset.
    QuotaExhausted(Timestamp),
    Fail(GithubFetchError),
}

/// The epoch-seconds now, as a [`Timestamp`]. Used both for `FetchedSource.fetched_at` and the
/// rate-reset horizon check; time comes from the system clock, not an injected clock, because a
/// batch pipeline that needs to *test* time-dependence should inject the rate set rather than the
/// clock (see the rate-limit tests' far-future resets).
fn now_epoch_seconds() -> Timestamp {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Reads `X-RateLimit-Remaining` (the documented header for the unauthenticated REST quota, see
/// <https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api>; observed
/// live 2026-08-15 on every `api.github.com` response alongside `X-RateLimit-Limit: 60`).
fn rate_limit_remaining(headers: &HeaderMap) -> Option<u64> {
    headers.get("x-ratelimit-remaining").and_then(|v| v.to_str().ok()).and_then(|s| s.parse().ok())
}

/// Reads `X-RateLimit-Reset`, documented as "the number of seconds until the limit resets"
/// (a Unix timestamp; observed live 2026-08-15: the header's value equaled the wall-clock epoch
/// seconds).
fn rate_limit_reset(headers: &HeaderMap) -> Option<Timestamp> {
    headers.get("x-ratelimit-reset").and_then(|v| v.to_str().ok()).and_then(|s| s.parse().ok())
}

// ---------------------------------------------------------------------------
// The fetcher
// ---------------------------------------------------------------------------

/// An injectable async sleep for the retry loop, stored behind a plain trait object:
/// `Fn(Duration)` returning a boxed, `Send` future with `()` output — the standard shape for an
/// awaitable closure in stable Rust. Type-aliased because the full type is too long to repeat at
/// every construction site. Production uses `tokio::time::sleep`; tests inject
/// `Arc::new(|_| Box::pin(async {}))` to exercise the retry loop without sleeping.
pub type DelayFn =
    Arc<dyn Fn(Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// A GitHub-backed [`SourceFetcher`] for the StevenBlack + Hagezi sources.
///
/// Generic over the HTTP layer (`H: GithubHttp`) so tests can inject a scripted fake, and over
/// the repo registry (`R: GithubRepos`). Holds **no** tokio runtime: the async core
/// ([`GithubSourceFetcher::fetch_async`]) is awaited directly by an async caller, and the
/// *synchronous* `SourceFetcher::fetch` contract is met by building a small current-thread
/// runtime locally inside that one call. Storing a runtime here would be a trap — building or
/// dropping a `Runtime` (or calling `block_on`) from inside an already-running async task
/// panics, and the real CLI awaits `fetch_async` from within a multi-thread tokio runtime (see
/// the notes file's "sync bridge" section for the caveat and why `fetch` must stay out of async
/// context).
pub struct GithubSourceFetcher<H: GithubHttp, R: GithubRepos> {
    http: H,
    repos: R,
    retry: RetryPolicy,
    /// Injectable async sleep for the retry loop; see [`DelayFn`] for the type shape. The backoff
    /// runs inside a shared multi-thread tokio runtime, so a blocking sleep would stall every
    /// other concurrent fetch on that runtime for the whole backoff — `tokio::time::sleep` yields
    /// the current task instead. Tests inject `Arc::new(|_| Box::pin(async {}))` to exercise the
    /// retry loop without sleeping.
    delay: DelayFn,
}

impl<H: GithubHttp, R: GithubRepos> GithubSourceFetcher<H, R> {
    /// Full constructor with all knobs; `new` is the sensible-default wrapper.
    ///
    /// The `Result` wrapper is kept for caller stability: it used to fail by building the
    /// fetcher's stored tokio runtime, so existing callers (`main.rs`) `.context(...)?` its
    /// return. Nothing here is fallible anymore, so this always constructs `Ok`.
    pub fn with_policy(
        http: H,
        repos: R,
        retry: RetryPolicy,
        delay: DelayFn,
    ) -> Result<Self, std::io::Error> {
        Ok(Self { http, repos, retry, delay })
    }

    /// Defaults: [`RetryPolicy::default`] and a real `tokio::time::sleep` backoff. Stores no
    /// tokio runtime — the sync bridge builds one per `fetch` call.
    pub fn new(http: H, repos: R) -> Result<Self, std::io::Error> {
        Self::with_policy(
            http,
            repos,
            RetryPolicy::default(),
            Arc::new(|d| Box::pin(tokio::time::sleep(d))),
        )
    }
}

impl<H: GithubHttp, R: GithubRepos> GithubSourceFetcher<H, R> {
    /// The async core. Resolves revision and license from the GitHub REST API, fetches the list at
    /// the **pinned SHA**, and returns a [`FetchedSource`] for `fetch_source` to validate.
    ///
    /// Order is significant and mirrors the shared module's own ordering rationale: revision
    /// first (cheapest, and the pin is the thing most likely to be wrong), then license, then the
    /// one potentially multi-MB content fetch. All three are awaited in sequence; parallelizing
    /// them is a per-build latency win the integration pass can make behind `fetch_async`'s
    /// signature without touching the trait.
    pub async fn fetch_async(&self, config: &SourceConfig) -> Result<FetchedSource, GithubFetchError> {
        let repo = self
            .repos
            .repo_for(config.source)
            .ok_or(GithubFetchError::UnconfiguredSource(config.source))?;

        // The pin is a commit SHA, so revision resolution uses the git-database "get a commit"
        // endpoint (`/repos/{o}/{r}/git/commits/{sha}`, documented at
        // <https://docs.github.com/en/rest/git/commits#get-a-commit>), which returns only the
        // commit object (~1 KB, observed live) — *not* the list-commit endpoint
        // `/repos/{o}/{r}/commits/{ref}` whose response carries the full file diff (`files[].patch`)
        // for that commit (3.2 KB for a trivial hagezi commit, but megabytes-scale for a StevenBlack
        // hosts commit that replaces its file wholesale — measured 3236 B vs 1089 B live
        // 2026-08-15). That `.sha` echo is `FetchedSource.revision`; when it differs from the pin
        // (a config that pinned a branch name rather than a SHA, or a force-pushed-away ref),
        // `fetch_source` reports `RevisionMismatch`.
        let revision = self.resolve_revision(repo, &config.pinned_revision).await?;

        // License from the API per the audit — neither source carries one in-band.
        let license = self.resolve_license(repo).await?;

        let bytes = self.fetch_content(repo, &config.pinned_revision).await?;

        Ok(FetchedSource {
            revision: revision.0,
            license: license.0,
            fetched_at: now_epoch_seconds(),
            bytes,
        })
    }

    /// Resolves a ref (commit SHA, branch name, tag, or the repo's default branch when `reff` is
    /// empty) to the canonical commit SHA.
    ///
    /// Empty `reff` routes through the repository endpoint's `default_branch` field
    /// (`/repos/{o}/{r}`, documented at <https://docs.github.com/en/rest/repos/repos#get-a-repository>)
    /// — observed live 2026-08-15: `StevenBlack/hosts` → `default_branch: master`. A 40-hex `reff`
    /// uses the cheap git-database endpoint (the per-build path). Any other ref (branch/tag) falls
    /// back to the list-commit endpoint `/repos/{o}/{r}/commits/{ref}` per the module's spec — that
    /// response's top-level `sha` is the HEAD commit of the ref (observed live 2026-08-15,
    /// hagezi `main` → `3975aafc…`); this path is only taken pre-pin (manual pin bumps), so its
    /// bigger payload does not burden the monthly build.
    pub async fn resolve_revision(
        &self,
        repo: &GithubRepo,
        reff: &str,
    ) -> Result<GithubRevision, GithubFetchError> {
        if reff.is_empty() {
            return self.resolve_default_branch_revision(repo).await;
        }
        self.resolve_commit_ref(repo, reff).await
    }

    /// The non-recursive core of [`GithubSourceFetcher::resolve_revision`], factored out so the
    /// empty-ref default-branch dance above cannot turn into an infinitely-sized future (a
    /// recursive `async fn` — `resolve_revision → resolve_default_branch_revision →
    /// resolve_revision` — does not compile without boxing). Both entry points funnel here.
    async fn resolve_commit_ref(
        &self,
        repo: &GithubRepo,
        reff: &str,
    ) -> Result<GithubRevision, GithubFetchError> {
        let url = if is_full_sha(reff) {
            format!("https://api.github.com/repos/{}/{}/git/commits/{}", repo.owner, repo.repo, reff)
        } else {
            format!("https://api.github.com/repos/{}/{}/commits/{}", repo.owner, repo.repo, reff)
        };
        let resp = self.get_with_retry(&url).await?;
        parse_commit_sha(&url, &resp.body)
    }

    async fn resolve_default_branch_revision(
        &self,
        repo: &GithubRepo,
    ) -> Result<GithubRevision, GithubFetchError> {
        let url = format!("https://api.github.com/repos/{}/{}", repo.owner, repo.repo);
        let resp = self.get_with_retry(&url).await?;
        #[derive(serde::Deserialize)]
        struct RepositoryShape {
            #[serde(default)]
            default_branch: Option<String>,
        }
        let shape: RepositoryShape =
            serde_json::from_slice(&resp.body).map_err(|e| GithubFetchError::Json {
                url: url.clone(),
                reason: e.to_string(),
            })?;
        let Some(default_branch) = shape.default_branch else {
            return Err(GithubFetchError::Json {
                url,
                reason: "no default_branch field (docs: /repos/{owner}/{repo} returns it)".to_string(),
            });
        };
        self.resolve_commit_ref(repo, &default_branch).await
    }

    /// Resolves the repo's SPDX license id via `/repos/{o}/{r}/license` — documented at
    /// <https://docs.github.com/en/rest/licenses/licenses#get-the-license-for-a-repository>.
    ///
    /// The three "no usable SPDX id" shapes all map to the [`GithubLicense::NOASSERTION`] sentinel,
    /// not a panic or a transport error, so the caller can *compare* `FetchedSource.license` and let
    /// `fetch_source`'s gates decide (a genuinely unlicensed upstream fails `LicenseChanged`/
    /// `LicenseNotAllowed` unless the operator explicitly pinned and allowlisted `NOASSERTION`):
    /// HTTP 200 with `license.spdx_id == "NOASSERTION"` (observed live 2026-08-15),
    /// HTTP 200 with `spdx_id` null/absent, and HTTP 404 (no license detectable; observed live
    /// 2026-08-15). Everything else is a non-retried `Http` error.
    pub async fn resolve_license(&self, repo: &GithubRepo) -> Result<GithubLicense, GithubFetchError> {
        let url = format!("https://api.github.com/repos/{}/{}/license", repo.owner, repo.repo);
        let resp = match self.get_with_retry(&url).await {
            // No license detectable — same sentinel as the API's own NOASSERTION, returned through
            // the usual comparison path rather than as a transport error.
            Err(GithubFetchError::Http { status: 404, .. }) => {
                return Ok(GithubLicense(LicenseId(GithubLicense::NOASSERTION.to_string())));
            }
            Err(other) => return Err(other),
            Ok(resp) => resp,
        };

        parse_license_id(&url, &resp.body)
    }

    /// Fetches the list file at the exact pinned SHA
    /// (`/repos/{o}/{r}/<sha>/<raw_path>` on `raw.githubusercontent.com`, which serves any ref —
    /// branch, tag, or SHA — in the path; observed live 2026-08-15 with a pinned SHA → HTTP 200,
    /// `text/plain`). Content is expected to be text, but the bytes pass straight through —
    /// `parse_document` owns format checking (a wrong-format fetch dies in that parser as
    /// all-`dropped_malformed`, which is the same failure this module would otherwise have to
    /// predict).
    async fn fetch_content(&self, repo: &GithubRepo, sha: &str) -> Result<Vec<u8>, GithubFetchError> {
        let url = format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            repo.owner, repo.repo, sha, repo.raw_path
        );
        Ok(self.get_with_retry(&url).await?.body)
    }

    /// One GET with the bounded retry. The single place "is this worth a retry" is answered, so
    /// the classification policy is testable against a scripted fake without any network. See
    /// [`GithubFetchError`]'s table for what is retried and why.
    ///
    /// `classify_outcome` stays deliberately pure (it returns only the verdict) and the loop keeps
    /// the winning response itself, so the verdict table has no hidden access to the response and
    /// the response has no hidden assumptions about how it was classified.
    async fn get_with_retry(&self, url: &str) -> Result<GithubHttpResponse, GithubFetchError> {
        let policy = self.retry;
        let mut last_detail: Option<String> = None;
        let mut win: Option<GithubHttpResponse> = None;
        // Set only by RemoteOutcome::RetryAfter — carries the rate-limit reset through so the
        // terminal error, if the attempt budget still runs out, names the quota as the cause
        // instead of a generic "exhausted retries" Transport error (see RemoteOutcome's doc
        // comment).
        let mut pending_reset: Option<Timestamp> = None;
        let mut next_delay: Option<Duration> = None;

        for attempt in 0..policy.max_attempts {
            let outcome = match self.http.get(url).await {
                Ok(resp) => {
                    let verdict = self.classify_outcome(url, &resp);
                    match &verdict {
                        RemoteOutcome::Ok => {
                            // The response belongs to the winner; keep it for the return.
                            win = Some(resp);
                        }
                        RemoteOutcome::Retry => {
                            last_detail = Some(format!("HTTP status {}", resp.status));
                        }
                        RemoteOutcome::RetryAfter { .. } => {
                            last_detail = Some(format!(
                                "HTTP status {} (rate-limited, waiting for reset)",
                                resp.status
                            ));
                        }
                        _ => {}
                    }
                    verdict
                }
                Err(conn) => {
                    last_detail = Some(conn.to_string());
                    RemoteOutcome::Retry
                }
            };

            match outcome {
                RemoteOutcome::Ok => {
                    return Ok(win.expect("an Ok verdict is only ever paired with the response it was classified from"))
                }
                RemoteOutcome::Retry => {
                    // A plain retryable failure after an earlier rate-limit wait means the quota
                    // is no longer the live cause — don't let a stale reset from an earlier
                    // attempt misattribute the eventual terminal error.
                    pending_reset.take();
                    next_delay.take();
                }
                RemoteOutcome::RetryAfter { delay, reset_at } => {
                    pending_reset = Some(reset_at);
                    next_delay = Some(delay);
                }
                RemoteOutcome::QuotaExhausted(reset_at) => {
                    return Err(GithubFetchError::RateLimit { reset_at });
                }
                RemoteOutcome::Fail(e) => return Err(e),
            }

            if attempt + 1 >= policy.max_attempts {
                if let Some(reset_at) = pending_reset {
                    return Err(GithubFetchError::RateLimit { reset_at });
                }
                return Err(GithubFetchError::Transport(format!(
                    "HTTP request to {url} failed after {} attempts; last error: {}",
                    policy.max_attempts,
                    last_detail.as_deref().unwrap_or("exhausted retry budget")
                )));
            }
            (self.delay)(next_delay.take().unwrap_or_else(|| policy.backoff_for(attempt))).await;
        }
        unreachable!("the loop above always returns; this line exists only to satisfy the type checker")
    }

    /// Buckets a raw response into the retry loop's verdicts. Pure and unit-testable by itself;
    /// the live-value justifications are at [`GithubFetchError`]'s table and inline here.
    fn classify_outcome(&self, url: &str, resp: &GithubHttpResponse) -> RemoteOutcome {
        let status = resp.status;
        match status {
            200..=299 => RemoteOutcome::Ok,
            429 | 500 | 502 | 503 | 504 => RemoteOutcome::Retry,
            403 => {
                match (rate_limit_remaining(&resp.headers), rate_limit_reset(&resp.headers)) {
                    (Some(0), reset) => {
                        let reset_at = reset.unwrap_or(0);
                        // Only wait when the reset is close enough to fit the bounded backoff
                        // budget; otherwise fail now, named — retrying an hourly-reset quota a few
                        // seconds apart is the definition of "burning through retries against a
                        // 403 that retrying cannot fix", and the docs
                        // (<https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api>)
                        // present the reset header specifically so a client can decide this.
                        let wait = reset_at.saturating_sub(now_epoch_seconds());
                        if reset_at != 0 && wait <= self.retry.max_backoff.as_secs() {
                            // +1s so the retry lands after the reset boundary, never on it.
                            RemoteOutcome::RetryAfter {
                                delay: Duration::from_secs(wait + 1),
                                reset_at,
                            }
                        } else {
                            RemoteOutcome::QuotaExhausted(reset_at)
                        }
                    }
                    _ => RemoteOutcome::Fail(GithubFetchError::Http {
                        status,
                        url: url.to_string(),
                        preview: body_preview(resp),
                    }),
                }
            }
            _ => RemoteOutcome::Fail(GithubFetchError::Http {
                status,
                url: url.to_string(),
                preview: body_preview(resp),
            }),
        }
    }
}

/// A 40-hex-character string is a full commit SHA; anything else is a ref name (branch/tag) and
/// must go through the endpoints that resolve refs.
fn is_full_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A few hundred bytes of the body for an error message — GitHub JSON errors are one line
/// (`{"message":"Not Found",...}`, observed live 2026-08-15), so a short preview has the whole
/// story.
fn body_preview(resp: &GithubHttpResponse) -> String {
    let limit = 240;
    String::from_utf8_lossy(&resp.body[..resp.body.len().min(limit)]).to_string()
}

/// Parses the `sha` field off a commit GET response. Both the git-database and list-commit
/// endpoints put it at the top level (`sha`, `commit.tree.sha`, `parents[... ]`), observed live
/// 2026-08-15. Absent `sha` is a malformed-JSON failure, not a panic.
fn parse_commit_sha(url: &str, body: &[u8]) -> Result<GithubRevision, GithubFetchError> {
    #[derive(serde::Deserialize)]
    struct CommitShape {
        #[serde(default)]
        sha: Option<String>,
    }
    let shape: CommitShape = serde_json::from_slice(body).map_err(|e| GithubFetchError::Json {
        url: url.to_string(),
        reason: e.to_string(),
    })?;
    match shape.sha {
        Some(sha) => Ok(GithubRevision(sha)),
        None => Err(GithubFetchError::Json {
            url: url.to_string(),
            reason: "no top-level sha field on a commit response (docs: /repos/{o}/{r}/commits/{ref})"
                .to_string(),
        }),
    }
}

/// Parses `license.spdx_id` off the license endpoint's response, mapping the null/absent and
/// `"NOASSERTION"` shapes onto the sentinel. See [`GithubLicense`] for the live observations this
/// table is built from.
fn parse_license_id(url: &str, body: &[u8]) -> Result<GithubLicense, GithubFetchError> {
    #[derive(serde::Deserialize)]
    struct LicenseShape {
        #[serde(default)]
        license: Option<LicenseDetail>,
    }
    #[derive(serde::Deserialize)]
    struct LicenseDetail {
        #[serde(default)]
        spdx_id: Option<String>,
    }
    let shape: LicenseShape = serde_json::from_slice(body).map_err(|e| GithubFetchError::Json {
        url: url.to_string(),
        reason: e.to_string(),
    })?;
    let spdx = shape
        .license
        .and_then(|detail| detail.spdx_id)
        .unwrap_or_else(|| GithubLicense::NOASSERTION.to_string());
    Ok(GithubLicense(LicenseId(spdx)))
}

// ---------------------------------------------------------------------------
// Trait implementation (sync bridge)
// ---------------------------------------------------------------------------

impl<H: GithubHttp, R: GithubRepos> SourceFetcher for GithubSourceFetcher<H, R> {
    /// The sync bridge the existing `SourceFetcher::fetch` contract demands. It builds a small
    /// current-thread runtime for this one call, `block_on`s the async core on it, and maps the
    /// rich transport error down to the crate's [`FetchError`].
    ///
    /// The runtime is built and dropped inside this single plain synchronous call — never nested
    /// inside another runtime's async task (building *or* dropping a `Runtime` in an async
    /// context panics, tokio's documented behavior; that is exactly why the fetcher stores no
    /// runtime and why `fetch` must not be called from inside a tokio task). The batch CLI waits
    /// `fetch_async` directly from inside its own tokio runtime; this `fetch` exists so
    /// `fetch_source`'s tests and any sync callers keep working unchanged.
    fn fetch(&self, config: &SourceConfig) -> Result<FetchedSource, FetchError> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build a runtime for GithubSourceFetcher::fetch — this is the sync \
                     trait bridge, only ever called from plain sync code (tests, or a future \
                     non-async caller), never from inside an already-running async context");
        rt.block_on(self.fetch_async(config)).map_err(Into::into)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{FetchError, hagezi, fetch_source};
    use std::collections::HashMap;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// A scripted fake `GithubHttp`: every registered URL has a sequence of responses; the last
    /// response repeats forever after the queue runs out, so "always 500" needs one seed while
    /// "500 then 200" needs two. `attempts` counts every GET and is what the retry tests assert on.
    /// Uses `std::sync::Mutex`, fine for both the plain `#[test]` and `#[tokio::test]` worlds this
    /// module runs in.
    #[derive(Clone, Default)]
    struct MockHttp {
        script: Arc<Mutex<HashMap<String, VecDeque<StubResp>>>>,
        attempts: Arc<AtomicUsize>,
    }

    #[derive(Debug, Clone)]
    struct StubResp {
        status: Option<u16>,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
        /// When set, the request fails at the HTTP layer (returns `Err(Connect(..))`).
        conn_error: Option<String>,
    }

    impl MockHttp {
        fn script(&self, url: &str, status: u16, body: impl Into<Vec<u8>>) -> &Self {
            self.script
                .lock()
                .unwrap()
                .entry(url.to_string())
                .or_default()
                .push_back(StubResp {
                    status: Some(status),
                    headers: Vec::new(),
                    body: body.into(),
                    conn_error: None,
                });
            self
        }

        fn with_headers(&self, url: &str, status: u16, headers: Vec<(&str, &str)>, body: &[u8]) -> &Self {
            self.script
                .lock()
                .unwrap()
                .entry(url.to_string())
                .or_default()
                .push_back(StubResp {
                    status: Some(status),
                    headers: headers.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                    body: body.to_vec(),
                    conn_error: None,
                });
            self
        }

        fn with_conn_error(&self, url: &str) -> &Self {
            self.script
                .lock()
                .unwrap()
                .entry(url.to_string())
                .or_default()
                .push_back(StubResp {
                    status: None,
                    headers: Vec::new(),
                    body: Vec::new(),
                    conn_error: Some("mock: connection refused".to_string()),
                });
            self
        }

        fn attempts(&self) -> usize {
            self.attempts.load(Ordering::SeqCst)
        }
    }

    impl GithubHttp for MockHttp {
        async fn get(&self, url: &str) -> Result<GithubHttpResponse, GithubHttpError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            let mut script = self.script.lock().unwrap();
            let queue = script
                .get_mut(url)
                .unwrap_or_else(|| panic!("no scripted response for {url}"));
            let popped = queue.pop_front().expect("script queue must not run empty");
            // Last-response-repeats: after the seeded sequence is spent, the final response is what
            // the retry loop keeps seeing.
            queue.push_back(popped.clone());
            match popped.status {
                Some(status) => {
                    let mut headers = HeaderMap::new();
                    for (name, value) in &popped.headers {
                        headers.insert(
                            reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                            reqwest::header::HeaderValue::from_str(value).unwrap(),
                        );
                    }
                    Ok(GithubHttpResponse { status, headers, body: popped.body })
                }
                None => Err(GithubHttpError::Connect(popped.conn_error.unwrap_or_default())),
            }
        }
    }

    // -- shared fixtures -----------------------------------------------------

    const PINNED: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const DEFAULT_BRANCH: &str = "main";

    fn hagezi_repo() -> GithubRepo {
        GithubRepo::hagezi_nsfw()
    }

    fn config(pinned: &str, expected_license: &str, source: SourceId) -> SourceConfig {
        // `url` is only the raw content URL to a parser-configured checkout; the fetcher ignores
        // it and builds every URL from `GithubRepo` + `pinned`. Keeping it populated so the config
        // round-trips honestly when moved into the real crate.
        SourceConfig {
            source,
            url: format!(
                "https://raw.githubusercontent.com/hagezi/dns-blocklists/{pinned}/wildcard/nsfw-onlydomains.txt"
            ),
            pinned_revision: pinned.to_string(),
            expected_license: LicenseId(expected_license.to_string()),
        }
    }

    fn commit_url() -> String {
        format!("https://api.github.com/repos/hagezi/dns-blocklists/git/commits/{PINNED}")
    }
    fn license_url() -> String {
        "https://api.github.com/repos/hagezi/dns-blocklists/license".to_string()
    }
    fn raw_url() -> String {
        format!("https://raw.githubusercontent.com/hagezi/dns-blocklists/{PINNED}/wildcard/nsfw-onlydomains.txt")
    }

    fn no_sleep() -> DelayFn {
        Arc::new(|_| Box::pin(async {}))
    }

    /// A `DelayFn` that records every duration it's asked to sleep (without actually sleeping),
    /// so a test can assert the retry loop chose the reset-covering wait rather than the ordinary
    /// exponential backoff schedule.
    fn recording_sleep() -> (DelayFn, Arc<std::sync::Mutex<Vec<Duration>>>) {
        let log: Arc<std::sync::Mutex<Vec<Duration>>> = Arc::default();
        let recorded = log.clone();
        let delay: DelayFn = Arc::new(move |d| {
            recorded.lock().unwrap().push(d);
            Box::pin(async {})
        });
        (delay, log)
    }

    fn fetcher(mock: MockHttp) -> GithubSourceFetcher<MockHttp, StaticGithubRepos> {
        GithubSourceFetcher::with_policy(
            mock,
            StaticGithubRepos::new([(SourceId::Hagezi, hagezi_repo())]),
            RetryPolicy { base_backoff: Duration::from_millis(1), ..Default::default() },
            no_sleep(),
        )
        .unwrap()
    }

    fn sha200_body() -> Vec<u8> {
        format!("{{\"sha\":\"{PINNED}\",\"commit\":{{\"tree\":{{\"sha\":\"bbbb\"}}}}}}").into_bytes()
    }
    fn license200_body(spdx: &str) -> Vec<u8> {
        format!("{{\"license\":{{\"spdx_id\":\"{spdx}\"}}}}").into_bytes()
    }

    // -- the behavior tests -------------------------------------------------

    #[test]
    fn happy_path_serves_snapshot_and_bytes_through_fetch_source() {
        let mock = MockHttp::default();
        mock.script(&commit_url(), 200, sha200_body())
            .script(&license_url(), 200, license200_body("GPL-3.0"))
            .script(&raw_url(), 200, b"# hagezi nsfw\nblocked.example.net\n");
        let f = fetcher(mock.clone());
        let cfg = config(PINNED, "GPL-3.0", SourceId::Hagezi);

        let (snapshot, bytes) = fetch_source(&f, &cfg, &[LicenseId("GPL-3.0".to_string())]).unwrap();

        assert_eq!(snapshot.source, SourceId::Hagezi);
        assert_eq!(snapshot.revision, PINNED);
        assert_eq!(snapshot.license, LicenseId("GPL-3.0".to_string()));
        let parsed = hagezi::parse(&bytes);
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].domain, "blocked.example.net");
        assert_eq!(mock.attempts(), 3);
    }

    #[test]
    fn revision_mismatch_is_reported_when_the_api_echoes_a_different_sha() {
        let other = "cccccccccccccccccccccccccccccccccccccccc";
        let mock = MockHttp::default();
        mock.script(&commit_url(), 200, format!("{{\"sha\":\"{other}\"}}").into_bytes());
        // license and content are still fetched before fetch_source gets to compare — the fetcher
        // is transport-only by design — so the mock scripts them too, but validation never reads
        // them: revision validation in fetch_source short-circuits first.
        mock.script(&license_url(), 200, license200_body("GPL-3.0"));
        mock.script(&raw_url(), 200, b"blocked.example.net\n");
        let f = fetcher(mock);

        let err = fetch_source(&f, &config(PINNED, "GPL-3.0", SourceId::Hagezi), &[LicenseId("GPL-3.0".to_string())])
            .unwrap_err();

        assert_eq!(
            err,
            FetchError::RevisionMismatch { expected: PINNED.to_string(), actual: other.to_string() }
        );
    }

    #[test]
    fn license_changed_fires_even_when_the_new_license_is_allowlisted() {
        let mock = MockHttp::default();
        mock.script(&commit_url(), 200, sha200_body());
        // Served MIT (allowlisted) but the pin was reviewed against GPL-3.0 → must still fail.
        mock.script(&license_url(), 200, license200_body("MIT"));
        mock.script(&raw_url(), 200, b"blocked.example.net\n");
        let f = fetcher(mock.clone());
        let cfg = config(PINNED, "GPL-3.0", SourceId::Hagezi);
        let allowlist = [LicenseId("GPL-3.0".to_string()), LicenseId("MIT".to_string())];

        let err = fetch_source(&f, &cfg, &allowlist).unwrap_err();

        assert_eq!(
            err,
            FetchError::LicenseChanged {
                expected: LicenseId("GPL-3.0".to_string()),
                actual: LicenseId("MIT".to_string()),
            }
        );
        assert_eq!(mock.attempts(), 3, "1 commit + 1 license + 1 content — the fetcher fetches all three before fetch_source compares");
    }

    #[test]
    fn license_not_on_allowlist_is_rejected() {
        let mock = MockHttp::default();
        mock.script(&commit_url(), 200, sha200_body());
        mock.script(&license_url(), 200, license200_body("GPL-3.0"));
        mock.script(&raw_url(), 200, b"blocked.example.net\n");
        let f = fetcher(mock);

        let err = fetch_source(&f, &config(PINNED, "GPL-3.0", SourceId::Hagezi), &[LicenseId("MIT".to_string())])
            .unwrap_err();

        assert_eq!(err, FetchError::LicenseNotAllowed(LicenseId("GPL-3.0".to_string())));
    }

    #[test]
    fn a_404_is_not_retried_and_fails_immediately() {
        let mock = MockHttp::default();
        mock.script(&commit_url(), 200, sha200_body());
        mock.script(&license_url(), 200, license200_body("GPL-3.0"));
        mock.script(&raw_url(), 404, br#"{"message":"Not Found"}"#);
        let f = fetcher(mock.clone());

        let err = fetch_source(&f, &config(PINNED, "GPL-3.0", SourceId::Hagezi), &[LicenseId("GPL-3.0".to_string())])
            .unwrap_err();

        match err {
            FetchError::Transport(msg) => {
                assert!(msg.contains("404"), "transport message should carry the status: {msg}");
                assert!(msg.contains("raw.githubusercontent.com"), "…and the URL: {msg}");
            }
            other => panic!("expected a transport failure, got {other:?}"),
        }
        assert_eq!(mock.attempts(), 3, "1 commit + 1 license + exactly 1 content attempt — no retry on 404");
    }

    #[test]
    fn a_transient_500_is_retried_and_then_succeeds() {
        let mock = MockHttp::default();
        mock.script(&commit_url(), 200, sha200_body());
        mock.script(&license_url(), 200, license200_body("GPL-3.0"));
        // 500 once, then 200: the mock's last-response-repeats makes the second attempt the 200.
        mock.script(&raw_url(), 500, b"{}");
        mock.script(&raw_url(), 200, b"blocked.example.net\n");
        // `script` push_front order: pop is FIFO, so the lexicographic _same-url_ pushes above run
        // in the order written (500 first, then 200).
        let f = fetcher(mock.clone());
        let cfg = config(PINNED, "GPL-3.0", SourceId::Hagezi);

        let (_snapshot, bytes) =
            fetch_source(&f, &cfg, &[LicenseId("GPL-3.0".to_string())]).unwrap();

        assert!(bytes.starts_with(b"blocked.example.net"));
        assert_eq!(mock.attempts(), 4, "1 commit + 1 license + 2 content attempts (1 retry)");
    }

    #[test]
    fn rate_limit_exhaustion_is_a_named_error_and_burns_no_retries() {
        let mock = MockHttp::default();
        mock.script(&commit_url(), 200, sha200_body());
        // The other API calls succeed; the license call answers 403 with the quota gone and a
        // reset an hour out — the documented primary-rate-limit shape
        // (<https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api>).
        let now = now_epoch_seconds();
        let far_reset = (now + 3600).to_string();
        mock.with_headers(
            &license_url(),
            403,
            vec![("X-RateLimit-Remaining", "0"), ("X-RateLimit-Reset", far_reset.as_str())],
            b"{}",
        );
        let f = fetcher(mock.clone());

        let err = tokio_rt().block_on(f.fetch_async(&config(PINNED, "GPL-3.0", SourceId::Hagezi))).unwrap_err();

        match err {
            GithubFetchError::RateLimit { reset_at } => {
                assert!(reset_at >= now + 3599, "reset must be the far-future mock value, got {reset_at}");
            }
            other => panic!("expected the named RateLimit error, got {other:?}"),
        }
        assert_eq!(mock.attempts(), 2, "the 403 is not retried — quota errors cannot be fixed by retrying");
    }

    #[test]
    fn exhausted_rate_limit_retried_when_reset_is_within_budget() {
        let mock = MockHttp::default();
        mock.script(&commit_url(), 200, sha200_body());
        // Reset 2 s out (< max_backoff 8 s): the retry loop is allowed one wait-then-retry. The
        // second attempt answers 200 — the real-world story being "the quota reset while we
        // waited", which is exactly the state the wait is for.
        let now = now_epoch_seconds();
        let near_reset = (now + 2).to_string();
        mock.with_headers(
            &license_url(),
            403,
            vec![("X-RateLimit-Remaining", "0"), ("X-RateLimit-Reset", near_reset.as_str())],
            b"{}",
        );
        mock.script(&license_url(), 200, license200_body("GPL-3.0"));
        mock.script(&raw_url(), 200, b"blocked.example.net\n");
        let f = fetcher(mock.clone());

        let result = tokio_rt().block_on(f.fetch_async(&config(PINNED, "GPL-3.0", SourceId::Hagezi)));

        assert!(result.is_ok(), "waited-out reset then succeeded: got {result:?}");
        assert_eq!(mock.attempts(), 4, "commit + 2 license attempts + 1 content");
    }

    #[test]
    fn exhausted_rate_limit_sleeps_the_reset_wait_not_the_ordinary_backoff() {
        let mock = MockHttp::default();
        mock.script(&commit_url(), 200, sha200_body());
        // Reset 2 s out — far longer than the 1 ms base_backoff the other tests use, so a pass
        // here proves the loop chose the reset-covering wait and not RetryPolicy::backoff_for's
        // ordinary exponential schedule.
        let now = now_epoch_seconds();
        let near_reset = (now + 2).to_string();
        mock.with_headers(
            &license_url(),
            403,
            vec![("X-RateLimit-Remaining", "0"), ("X-RateLimit-Reset", near_reset.as_str())],
            b"{}",
        );
        mock.script(&license_url(), 200, license200_body("GPL-3.0"));
        mock.script(&raw_url(), 200, b"blocked.example.net\n");
        let (delay, log) = recording_sleep();
        let f = GithubSourceFetcher::with_policy(
            mock,
            StaticGithubRepos::new([(SourceId::Hagezi, hagezi_repo())]),
            RetryPolicy { base_backoff: Duration::from_millis(1), ..Default::default() },
            delay,
        )
        .unwrap();

        let result = tokio_rt().block_on(f.fetch_async(&config(PINNED, "GPL-3.0", SourceId::Hagezi)));

        assert!(result.is_ok(), "waited-out reset then succeeded: got {result:?}");
        let sleeps = log.lock().unwrap();
        assert!(
            sleeps.iter().any(|d| *d >= Duration::from_secs(2)),
            "expected a sleep covering the ~2s reset, got {sleeps:?}"
        );
    }

    #[test]
    fn rate_limit_that_never_clears_stays_named_ratelimit_not_generic_transport() {
        let mock = MockHttp::default();
        mock.script(&commit_url(), 200, sha200_body());
        // Every attempt sees the same near-future reset — the quota never actually clears within
        // the attempt budget (a flaky reset header, or a reset that keeps rolling forward).
        let now = now_epoch_seconds();
        let near_reset = (now + 2).to_string();
        mock.with_headers(
            &license_url(),
            403,
            vec![("X-RateLimit-Remaining", "0"), ("X-RateLimit-Reset", near_reset.as_str())],
            b"{}",
        );
        let f = fetcher(mock.clone());

        let err = tokio_rt().block_on(f.fetch_async(&config(PINNED, "GPL-3.0", SourceId::Hagezi))).unwrap_err();

        match err {
            GithubFetchError::RateLimit { reset_at } => {
                assert!(reset_at > now, "reset must be the mocked near-future value, got {reset_at}");
            }
            other => panic!("expected the exhausted-budget error to stay named RateLimit, got {other:?}"),
        }
    }

    #[test]
    fn noassertion_license_resolves_to_the_comparable_sentinel() {
        let mock = MockHttp::default();
        mock.script(&commit_url(), 200, sha200_body());
        mock.script(&license_url(), 200, license200_body("NOASSERTION"));
        mock.script(&raw_url(), 200, b"blocked.example.net\n");
        let f = fetcher(mock.clone());

        let fetched = tokio_rt().block_on(f.fetch_async(&config(PINNED, "MIT", SourceId::Hagezi))).unwrap();

        assert_eq!(fetched.license, LicenseId("NOASSERTION".to_string()));
        assert!(fetched.license.spdx_matches(&LicenseId("NOASSERTION".to_string())));
        // Policy still works: an unvetted unasserted upstream fails LicenseChanged against a
        // reviewed MIT pin, rather than silently passing because the fetch "succeeded".
        let err = fetch_source(&f, &config(PINNED, "MIT", SourceId::Hagezi), &[LicenseId("MIT".to_string())])
            .unwrap_err();
        assert_eq!(
            err,
            FetchError::LicenseChanged {
                expected: LicenseId("MIT".to_string()),
                actual: LicenseId("NOASSERTION".to_string()),
            }
        );
    }

    #[test]
    fn license_404_and_null_spdx_both_map_to_noassertion() {
        let cases: [(u16, Vec<u8>); 2] = [
            (404, Vec::new()),
            (200, br#"{"license":{"spdx_id":null}}"#.to_vec()),
        ];
        for (status, body) in cases {
            let mock = MockHttp::default();
            mock.script(&commit_url(), 200, sha200_body());
            mock.script(&license_url(), status, body);
            mock.script(&raw_url(), 200, b"blocked.example.net\n");
            let f = fetcher(mock.clone());

            let fetched =
                tokio_rt().block_on(f.fetch_async(&config(PINNED, "GPL-3.0", SourceId::Hagezi))).unwrap();

            assert_eq!(fetched.license, LicenseId("NOASSERTION".to_string()));
            assert!(!fetched.license.spdx_matches(&LicenseId("GPL-3.0".to_string())));
        }
    }

    #[test]
    fn default_branch_revision_resolution_reaches_the_commits_endpoint() {
        let mock = MockHttp::default();
        mock.script("https://api.github.com/repos/hagezi/dns-blocklists", 200, format!("{{\"default_branch\":\"{DEFAULT_BRANCH}\"}}").into_bytes());
        mock.script(
            &format!("https://api.github.com/repos/hagezi/dns-blocklists/commits/{DEFAULT_BRANCH}"),
            200,
            format!("{{\"sha\":\"{PINNED}\"}}").into_bytes(),
        );
        let f = fetcher(mock.clone());

        let rev = tokio_rt()
            .block_on(f.resolve_revision(&hagezi_repo(), ""))
            .unwrap();

        assert_eq!(rev.0, PINNED);
        assert_eq!(mock.attempts(), 2);
    }

    #[test]
    fn a_conn_error_is_retried_then_succeeds() {
        let mock = MockHttp::default();
        mock.script(&commit_url(), 200, sha200_body());
        mock.script(&license_url(), 200, license200_body("GPL-3.0"));
        mock.with_conn_error(&raw_url());
        mock.script(&raw_url(), 200, b"blocked.example.net\n");
        let f = fetcher(mock.clone());

        let (_s, bytes) = fetch_source(&f, &config(PINNED, "GPL-3.0", SourceId::Hagezi), &[LicenseId("GPL-3.0".to_string())]).unwrap();

        assert!(bytes.starts_with(b"blocked.example.net"));
        assert_eq!(mock.attempts(), 4);
    }

    #[test]
    fn a_persistent_500_exhausts_the_budget_and_fails_as_transport() {
        let mock = MockHttp::default();
        mock.script(&commit_url(), 200, sha200_body());
        mock.script(&license_url(), 200, license200_body("GPL-3.0"));
        mock.script(&raw_url(), 500, b"{}"); // repeat-last ⇒ all attempts 500
        let f = fetcher(mock);
        let policy_attempts = 4;

        let err = fetch_source(&f, &config(PINNED, "GPL-3.0", SourceId::Hagezi), &[LicenseId("GPL-3.0".to_string())])
            .unwrap_err();

        match err {
            FetchError::Transport(msg) => {
                assert!(msg.contains(&policy_attempts.to_string()), "message should carry attempt count: {msg}");
            }
            other => panic!("expected transport failure, got {other:?}"),
        }
    }

    /// A scratch current-thread runtime for the async-core tests, mirroring exactly what the sync
    /// bridge uses internally.
    fn tokio_rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
    }
}