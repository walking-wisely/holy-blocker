//! Offline pipeline that fetches, merges, DNS-liveness-prunes, gates, and signs the domain
//! blocklist `net-shield`'s `DomainFilter` loads. See `docs/components/domain-blocklist/plan.md`
//! for the build order and `docs/decisions/domain-blocklist-sourcing.md` for the design this
//! implements.
//!
//! Runs as a periodic offline batch job, never on an end-user device.

pub mod merge;
pub mod types;

pub use merge::{MergeOutput, MergeReport, filter_by_category, flag_personal_name, merge as merge_entries, resolve_scope};
pub use types::{Category, MergedEntry, RawEntry, ScopeHint, SourceId};
