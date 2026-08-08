//! Shared, pure domain-comparison logic consumed by `packages/domain-blocklist` (build time) and
//! `packages/net-shield` (query time). See `docs/components/domain-blocklist/plan.md`, Module 0.
//!
//! No I/O, no HTTP, no FST, no DNS — just the comparison key ([`normalize`]) and the apex-scope
//! decision ([`classify_scope`]).

mod normalize;
mod scope;

pub use normalize::{NormalizeError, normalize};
pub use scope::{RuleScope, classify_scope};
