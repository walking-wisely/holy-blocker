//! Offline pipeline that fetches, merges, DNS-liveness-prunes, gates, and signs the domain
//! blocklist `net-shield`'s `DomainFilter` loads. See `docs/components/domain-blocklist/plan.md`
//! for the build order and `docs/decisions/domain-blocklist-sourcing.md` for the design this
//! implements.
//!
//! Runs as a periodic offline batch job, never on an end-user device.

#[cfg(feature = "cli")]
pub mod fetchers;
pub mod fst_build;
pub mod gates;
pub mod liveness;
pub mod merge;
pub mod sources;
pub mod types;

pub use fst_build::{
    FstArtifact, FstBuildError, FstBuildReport, KeyId, Manifest, ProvenanceEntry, Signature, build,
    reverse_key, signable_bytes, verify_manifest,
};
pub use gates::{
    FalsePositiveHit, GateResult, PreviousBuild, false_positive_gate, false_positive_hits,
    growth_gate, license_gate, review_queue, shrinkage_gate, size_gate,
};
pub use liveness::{
    CacheEntry, CanaryConfig, CanaryResult, DnsLookup, LookupResult, RecordType, UnknownReason,
    Verdict, canary_check, check as check_liveness, check_corroborated, combine, corroborate,
    due_for_check, is_special_use_domain, should_prune, should_prune_with_hysteresis,
};
pub use merge::{
    MergeOutput, MergeReport, filter_by_category, flag_personal_name, merge as merge_entries,
    resolve_scope,
};
pub use sources::{
    FetchError, FetchedSource, ParseOutput, ParseReport, SourceConfig, SourceFetcher, fetch_source,
};
pub use types::{
    Category, LicenseId, MergedEntry, RawEntry, ScopeHint, SourceId, SourceSnapshot, Timestamp,
};
