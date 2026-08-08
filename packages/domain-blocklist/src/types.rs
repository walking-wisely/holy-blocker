//! Shared types passed between `sources` (module 1, unbuilt), `merge` (module 2), and every
//! module downstream of it. See `docs/components/domain-blocklist/plan.md`.
//!
//! These types live here rather than inside `merge.rs` because module 1's per-source parsers will
//! construct [`RawEntry`] values directly and have no other natural home to import them from.

use domain_normalize::RuleScope;

/// The upstream list a [`RawEntry`] was read from. See the plan's module 1 for the sources this
/// project consumes and the format each one is parsed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceId {
    StevenBlack,
    Hagezi,
    Ut1,
}

/// What kind of site a source flagged a domain as. A domain can carry more than one — see
/// [`MergedEntry::categories`] and the plan's module 2 "Don't blend categories silently" rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    Adult,
    Gambling,
    Dating,
}

/// What a source line meant by its domain, before `merge` resolves it against the PSL.
///
/// Set by the module-1 parser per the plan's input-shape table: a plain entry gets `None`, and a
/// wildcard (`*.example.com`) gets `Apex` after the parser extracts the base domain. `merge`
/// combines this with `domain_normalize::classify_scope` to decide the entry's final
/// [`RuleScope`] — see [`crate::merge::resolve_scope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeHint {
    None,
    Apex,
}

/// One line from one source, after that source's own parser has extracted a domain but before
/// `merge` has normalized or scoped it. Not yet deduplicated against any other source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    pub domain: String,
    pub source: SourceId,
    pub category: Category,
    pub scope_hint: ScopeHint,
}

/// The union of every [`RawEntry`] that normalized to the same comparison key, produced by
/// `merge`. `sources` and `categories` are sets, not single values — see the plan's module 2 for
/// why blending either into one value silently discards a true statement about the domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedEntry {
    pub domain: String,
    pub scope: RuleScope,
    pub sources: Vec<SourceId>,
    pub categories: Vec<Category>,
}
