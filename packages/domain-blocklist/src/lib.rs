//! Offline pipeline that fetches, merges, DNS-liveness-prunes, gates, and signs the domain
//! blocklist `net-shield`'s `DomainFilter` loads. See `docs/components/domain-blocklist/plan.md`
//! for the build order and `docs/decisions/domain-blocklist-sourcing.md` for the design this
//! implements.
//!
//! Runs as a periodic offline batch job, never on an end-user device.

pub mod gates;
pub mod merge;
pub mod sources;
pub mod types;

pub use gates::{
    FalsePositiveHit, GateResult, PreviousBuild, false_positive_gate, false_positive_hits,
    growth_gate, license_gate, review_queue, shrinkage_gate, size_gate,
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
