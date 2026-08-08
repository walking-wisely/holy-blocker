//! Module 1 of the domain-blocklist plan — per-source fetch and parse.
//!
//! This file holds what every source shares: fetch validation (pin + license, per
//! `fetch_source`) and line-parsing (`parse_document`, shared by `stevenblack`/`hagezi`/`ut1`
//! since hosts-file and plain-domain-per-line formats differ only in whether an address column
//! precedes the domain). Each source's own file is a thin wrapper naming its `SourceId` and
//! `LineFormat`. See `docs/components/domain-blocklist/plan.md`'s module 1 for the input-shape
//! table this implements and the reasoning behind each drop.

pub mod hagezi;
pub mod stevenblack;
pub mod ut1;

use std::net::IpAddr;

use crate::types::{Category, LicenseId, RawEntry, ScopeHint, SourceId, SourceSnapshot, Timestamp};

/// A source's fetch coordinates, checked into the repository. Moving `pinned_revision` is a
/// reviewed pull request, per the plan's "Each source is fetched at a pinned revision — a release
/// tag or commit hash ..., never a branch HEAD" rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceConfig {
    pub source: SourceId,
    pub url: String,
    pub pinned_revision: String,
    /// What this config's author recorded the source's license as, the last time the pin was
    /// moved. Checked against what the fetch actually served (`FetchedSource.license`) so a
    /// source silently relicensing between pin bumps is caught at fetch time rather than only
    /// showing up if it happens to fall outside `allowlist` too — see [`fetch_source`].
    pub expected_license: LicenseId,
}

/// What a [`SourceFetcher`] returns for one [`SourceConfig`]: the raw list body plus everything
/// [`fetch_source`] needs to validate before any parser sees the bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedSource {
    pub revision: String,
    pub license: LicenseId,
    pub fetched_at: Timestamp,
    pub bytes: Vec<u8>,
}

/// One fetch against one [`SourceConfig`]. A trait so tests can supply fixture bytes (see this
/// module's tests) instead of hitting the network; the pipeline binary (module 7, unbuilt) wires
/// in a real HTTP client.
pub trait SourceFetcher {
    fn fetch(&self, config: &SourceConfig) -> Result<FetchedSource, FetchError>;
}

/// Why a source's fetch didn't produce bytes a parser is allowed to see. Every variant is a
/// whole-build abort per the plan ("A failed fetch aborts the whole build ... not a warning, not
/// a skip, not 'continue with the other two'") — nothing here is meant to be caught and skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// The underlying fetch itself failed (network error, non-2xx response, ...). The `String` is
    /// a human-readable cause for the build log, not a stable, matchable code.
    Transport(String),
    /// The fetcher reported a revision different from `SourceConfig.pinned_revision` — the source
    /// moved out from under the pin (a tag was force-moved, a mirror served stale content, ...).
    RevisionMismatch { expected: String, actual: String },
    /// The fetch served a license different from `SourceConfig.expected_license` — the source
    /// relicensed since this pin was last reviewed. Distinguished from [`FetchError::LicenseNotAllowed`]
    /// because this can fire even when the *new* license would itself be allowlisted: a silent
    /// relicense is worth a human's attention regardless of which license it lands on.
    LicenseChanged {
        expected: LicenseId,
        actual: LicenseId,
    },
    /// `FetchedSource.license` is not on the build's license allowlist. Re-checked by
    /// `gates::license_gate` at publish time so it can never be bypassed or weakened between
    /// fetch and publish — see that function's doc comment.
    LicenseNotAllowed(LicenseId),
}

/// Fetches and validates one source: checks the served revision against `config.pinned_revision`,
/// the served license against `config.expected_license`, and the served license against
/// `allowlist` — in that order, so a build failure names the most specific problem first — and
/// only then hands back the raw bytes for that source's parser plus the [`SourceSnapshot`]
/// `fst_build`'s manifest (module 4, unbuilt) will carry forward. Every failure aborts; see
/// [`FetchError`].
pub fn fetch_source(
    fetcher: &dyn SourceFetcher,
    config: &SourceConfig,
    allowlist: &[LicenseId],
) -> Result<(SourceSnapshot, Vec<u8>), FetchError> {
    let fetched = fetcher.fetch(config)?;

    if fetched.revision != config.pinned_revision {
        return Err(FetchError::RevisionMismatch {
            expected: config.pinned_revision.clone(),
            actual: fetched.revision,
        });
    }

    if !fetched.license.spdx_matches(&config.expected_license) {
        return Err(FetchError::LicenseChanged {
            expected: config.expected_license.clone(),
            actual: fetched.license,
        });
    }

    if !allowlist
        .iter()
        .any(|allowed| allowed.spdx_matches(&fetched.license))
    {
        return Err(FetchError::LicenseNotAllowed(fetched.license));
    }

    let snapshot = SourceSnapshot {
        source: config.source,
        revision: fetched.revision,
        license: fetched.license,
        fetched_at: fetched.fetched_at,
    };

    Ok((snapshot, fetched.bytes))
}

/// The two native formats module 1's sources actually ship in. StevenBlack is hosts-file syntax
/// (an address column precedes the domain); hagezi and UT1's `domains` files are plain
/// domain-per-line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineFormat {
    HostsFile,
    PlainDomain,
}

/// Counts of lines `parse_document` dropped, one counter per reason — the plan requires every
/// drop to be counted and named rather than silently absorbed, so a parser regression shows up as
/// one counter exploding rather than a quietly shrinking list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParseReport {
    /// A comment line (`#...`), a blank line, or a line that became blank after its inline
    /// comment was stripped.
    pub skipped_comment_or_blank: u64,
    /// A line that survived comment-stripping but had no domain field to extract at all (a hosts
    /// line with an address and nothing else, or an all-whitespace remainder).
    pub dropped_malformed: u64,
    /// The extracted domain field parses as an IPv4 or IPv6 literal. Per the plan's input-shape
    /// table: "IP-based blocking is out of scope for a domain-keyed artifact — that is
    /// `net-shield`'s separate `IpFilter`". `merge` (module 2) keeps its own copy of this same
    /// check as defense in depth; see that module's `MergeReport::dropped_ip_literal` doc comment.
    pub dropped_ip_literal: u64,
    /// How many surviving entries carried a wildcard prefix (`*.example.com`) and so were emitted
    /// with `ScopeHint::Apex`. Not a drop — informational, so a source's wildcard usage is visible
    /// in the build's own metrics.
    pub wildcard_count: u64,
}

/// The result of parsing one source document: the extracted entries plus the drop/informational
/// counters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParseOutput {
    pub entries: Vec<RawEntry>,
    pub report: ParseReport,
}

/// Parses `bytes` as `format`, tagging every surviving entry with `source` and `category`. Shared
/// by every module-1 parser; the per-source files (`stevenblack.rs`, `hagezi.rs`, `ut1.rs`) are
/// thin wrappers over this that only supply `source`/`category`/`format`.
///
/// No normalization happens here — per the plan, that is `domain_normalize::normalize`, applied
/// once by `merge` (module 2) so every source is normalized identically rather than each parser
/// re-implementing it slightly differently. This function's job is purely mechanical extraction:
/// pull a domain-shaped token out of a line, or drop the line and count why.
pub(crate) fn parse_document(
    bytes: &[u8],
    source: SourceId,
    category: Category,
    format: LineFormat,
) -> ParseOutput {
    let mut entries = Vec::new();
    let mut report = ParseReport::default();
    let text = String::from_utf8_lossy(bytes);

    for raw_line in text.lines() {
        // Strip a trailing inline comment (`example.com # note`) before deciding whether the
        // line is blank — a line that is only a comment must count the same as an empty one.
        let line = match raw_line.split_once('#') {
            Some((before, _)) => before.trim(),
            None => raw_line.trim(),
        };
        if line.is_empty() {
            report.skipped_comment_or_blank += 1;
            continue;
        }

        let mut fields = line.split_whitespace();
        let domain_field = match format {
            // Hosts syntax is `<address> <domain>`; the address (0.0.0.0, 127.0.0.1, ...) is
            // discarded per the plan's input-shape table.
            LineFormat::HostsFile => {
                fields.next();
                fields.next()
            }
            LineFormat::PlainDomain => fields.next(),
        };
        let Some(domain_field) = domain_field else {
            report.dropped_malformed += 1;
            continue;
        };

        let (domain, scope_hint) = match domain_field.strip_prefix("*.") {
            Some(base) if !base.is_empty() => (base, ScopeHint::Apex),
            _ => (domain_field, ScopeHint::None),
        };

        if domain.parse::<IpAddr>().is_ok() {
            report.dropped_ip_literal += 1;
            continue;
        }

        if scope_hint == ScopeHint::Apex {
            report.wildcard_count += 1;
        }

        entries.push(RawEntry {
            domain: domain.to_string(),
            source,
            category,
            scope_hint,
        });
    }

    ParseOutput { entries, report }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct FixtureFetcher {
        revision: String,
        license: String,
        bytes: Vec<u8>,
        fetched_at: Timestamp,
        calls: Cell<u32>,
    }

    impl SourceFetcher for FixtureFetcher {
        fn fetch(&self, _config: &SourceConfig) -> Result<FetchedSource, FetchError> {
            self.calls.set(self.calls.get() + 1);
            Ok(FetchedSource {
                revision: self.revision.clone(),
                license: LicenseId(self.license.clone()),
                fetched_at: self.fetched_at,
                bytes: self.bytes.clone(),
            })
        }
    }

    fn config(revision: &str, license: &str) -> SourceConfig {
        SourceConfig {
            source: SourceId::StevenBlack,
            url: "https://example.invalid/hosts".to_string(),
            pinned_revision: revision.to_string(),
            expected_license: LicenseId(license.to_string()),
        }
    }

    #[test]
    fn fetch_source_passes_through_snapshot_and_bytes_on_a_clean_fetch() {
        let fetcher = FixtureFetcher {
            revision: "v3.1.0".to_string(),
            license: "MIT".to_string(),
            bytes: b"0.0.0.0 example.com\n".to_vec(),
            fetched_at: 1_700_000_000,
            calls: Cell::new(0),
        };
        let cfg = config("v3.1.0", "MIT");
        let allowlist = [LicenseId("MIT".to_string())];

        let (snapshot, bytes) = fetch_source(&fetcher, &cfg, &allowlist).unwrap();

        assert_eq!(snapshot.source, SourceId::StevenBlack);
        assert_eq!(snapshot.revision, "v3.1.0");
        assert_eq!(snapshot.license, LicenseId("MIT".to_string()));
        assert_eq!(snapshot.fetched_at, 1_700_000_000);
        assert_eq!(bytes, b"0.0.0.0 example.com\n".to_vec());
        assert_eq!(fetcher.calls.get(), 1);
    }

    #[test]
    fn fetch_source_rejects_a_revision_that_drifted_from_the_pin() {
        let fetcher = FixtureFetcher {
            revision: "v3.2.0".to_string(),
            license: "MIT".to_string(),
            bytes: Vec::new(),
            fetched_at: 0,
            calls: Cell::new(0),
        };
        let cfg = config("v3.1.0", "MIT");
        let allowlist = [LicenseId("MIT".to_string())];

        let err = fetch_source(&fetcher, &cfg, &allowlist).unwrap_err();

        assert_eq!(
            err,
            FetchError::RevisionMismatch {
                expected: "v3.1.0".to_string(),
                actual: "v3.2.0".to_string(),
            }
        );
    }

    #[test]
    fn fetch_source_rejects_a_license_that_drifted_from_expected_even_if_still_allowlisted() {
        let fetcher = FixtureFetcher {
            revision: "v3.1.0".to_string(),
            license: "Apache-2.0".to_string(),
            bytes: Vec::new(),
            fetched_at: 0,
            calls: Cell::new(0),
        };
        let cfg = config("v3.1.0", "MIT");
        // Apache-2.0 is allowlisted, but the pin was reviewed against "MIT" — a silent relicense
        // must still fail, per FetchError::LicenseChanged's doc comment.
        let allowlist = [
            LicenseId("MIT".to_string()),
            LicenseId("Apache-2.0".to_string()),
        ];

        let err = fetch_source(&fetcher, &cfg, &allowlist).unwrap_err();

        assert_eq!(
            err,
            FetchError::LicenseChanged {
                expected: LicenseId("MIT".to_string()),
                actual: LicenseId("Apache-2.0".to_string()),
            }
        );
    }

    #[test]
    fn fetch_source_rejects_a_license_not_on_the_allowlist() {
        let fetcher = FixtureFetcher {
            revision: "v3.1.0".to_string(),
            license: "GPL-3.0".to_string(),
            bytes: Vec::new(),
            fetched_at: 0,
            calls: Cell::new(0),
        };
        let cfg = config("v3.1.0", "GPL-3.0");
        let allowlist = [LicenseId("MIT".to_string())];

        let err = fetch_source(&fetcher, &cfg, &allowlist).unwrap_err();

        assert_eq!(
            err,
            FetchError::LicenseNotAllowed(LicenseId("GPL-3.0".to_string()))
        );
    }

    #[test]
    fn fetch_source_license_check_is_case_and_whitespace_insensitive() {
        let fetcher = FixtureFetcher {
            revision: "v3.1.0".to_string(),
            license: " mit ".to_string(),
            bytes: Vec::new(),
            fetched_at: 0,
            calls: Cell::new(0),
        };
        let cfg = config("v3.1.0", "MIT");
        let allowlist = [LicenseId("MIT".to_string())];

        assert!(fetch_source(&fetcher, &cfg, &allowlist).is_ok());
    }

    struct FailingFetcher;
    impl SourceFetcher for FailingFetcher {
        fn fetch(&self, _config: &SourceConfig) -> Result<FetchedSource, FetchError> {
            Err(FetchError::Transport("connection reset".to_string()))
        }
    }

    #[test]
    fn fetch_source_propagates_a_transport_failure() {
        let cfg = config("v3.1.0", "MIT");
        let allowlist = [LicenseId("MIT".to_string())];

        let err = fetch_source(&FailingFetcher, &cfg, &allowlist).unwrap_err();

        assert_eq!(err, FetchError::Transport("connection reset".to_string()));
    }

    #[test]
    fn parse_document_skips_comments_and_blank_lines() {
        let out = parse_document(
            b"# a comment\n\n   \nexample.com\n",
            SourceId::Hagezi,
            Category::Adult,
            LineFormat::PlainDomain,
        );

        assert_eq!(out.entries.len(), 1);
        assert_eq!(out.entries[0].domain, "example.com");
        assert_eq!(out.report.skipped_comment_or_blank, 3);
    }

    #[test]
    fn parse_document_strips_an_inline_comment() {
        let out = parse_document(
            b"example.com # flagged 2024-01\n",
            SourceId::Hagezi,
            Category::Adult,
            LineFormat::PlainDomain,
        );

        assert_eq!(out.entries.len(), 1);
        assert_eq!(out.entries[0].domain, "example.com");
    }

    #[test]
    fn parse_document_extracts_hosts_format_and_discards_the_address() {
        let out = parse_document(
            b"0.0.0.0 example.com\n127.0.0.1 other.example.com\n",
            SourceId::StevenBlack,
            Category::Adult,
            LineFormat::HostsFile,
        );

        assert_eq!(out.entries.len(), 2);
        assert_eq!(out.entries[0].domain, "example.com");
        assert_eq!(out.entries[1].domain, "other.example.com");
        assert!(out.entries.iter().all(|e| e.scope_hint == ScopeHint::None));
    }

    #[test]
    fn parse_document_extracts_a_wildcard_base_and_sets_apex_hint() {
        let out = parse_document(
            b"*.example.com\n",
            SourceId::Hagezi,
            Category::Adult,
            LineFormat::PlainDomain,
        );

        assert_eq!(out.entries.len(), 1);
        assert_eq!(out.entries[0].domain, "example.com");
        assert_eq!(out.entries[0].scope_hint, ScopeHint::Apex);
        assert_eq!(out.report.wildcard_count, 1);
    }

    #[test]
    fn parse_document_drops_a_bare_ip_literal_entry() {
        let out = parse_document(
            b"0.0.0.0 0.0.0.0\n0.0.0.0 example.com\n::1\n",
            SourceId::StevenBlack,
            Category::Adult,
            LineFormat::HostsFile,
        );

        assert_eq!(out.entries.len(), 1);
        assert_eq!(out.entries[0].domain, "example.com");
        // The bare hosts line "::1" has no address column under HostsFile format, so its single
        // field is consumed as the (discarded) address, leaving no domain field at all.
        assert_eq!(out.report.dropped_ip_literal, 1);
        assert_eq!(out.report.dropped_malformed, 1);
    }

    #[test]
    fn parse_document_drops_a_malformed_hosts_line_with_no_domain_field() {
        let out = parse_document(
            b"0.0.0.0\n",
            SourceId::StevenBlack,
            Category::Adult,
            LineFormat::HostsFile,
        );

        assert!(out.entries.is_empty());
        assert_eq!(out.report.dropped_malformed, 1);
    }

    #[test]
    fn parse_document_tags_every_entry_with_the_given_source_and_category() {
        let out = parse_document(
            b"example.com\n",
            SourceId::Ut1,
            Category::Gambling,
            LineFormat::PlainDomain,
        );

        assert_eq!(out.entries[0].source, SourceId::Ut1);
        assert_eq!(out.entries[0].category, Category::Gambling);
    }
}
