//! Shared types passed between `sources` (module 1, unbuilt), `merge` (module 2), and every
//! module downstream of it. See `docs/components/domain-blocklist/plan.md`.
//!
//! These types live here rather than inside `merge.rs` because module 1's per-source parsers will
//! construct [`RawEntry`] values directly and have no other natural home to import them from.

use domain_normalize::RuleScope;

/// The upstream list a [`RawEntry`] was read from. See the plan's module 1 for the sources this
/// project consumes and the format each one is parsed in.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum SourceId {
    StevenBlack,
    Hagezi,
    Ut1,
}

/// What kind of site a source flagged a domain as. A domain can carry more than one — see
/// [`MergedEntry::categories`] and the plan's module 2 "Don't blend categories silently" rule.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
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

/// SPDX-style identifier for a source's license (e.g. `"CC0-1.0"`, `"MIT"`). A newtype rather
/// than a closed enum: the license identifiers a real source can carry are an open external set
/// (SPDX itself grows), and what constrains them — the checked-in allowlist `gates::license_gate`
/// checks against — is configuration, not something this crate's type system should try to
/// enumerate exhaustively.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct LicenseId(pub String);

impl LicenseId {
    /// SPDX license identifiers are compared case-insensitively; this also trims surrounding
    /// whitespace, which a value freshly read from a `LICENSE` file or an upstream API response
    /// can carry. `derive(PartialEq)` above stays exact-string, byte-identity equality — this is
    /// the separate, deliberately looser comparison `gates::license_gate` uses against its
    /// allowlist, so `"MIT"` and `"mit"` (or `"MIT "`) aren't treated as different licenses. See
    /// <https://spdx.org/licenses/> for the identifier registry this project's allowlist draws
    /// from.
    ///
    /// `eq_ignore_ascii_case` folds only ASCII case, which is deliberate here rather than an
    /// oversight: the SPDX License List's short identifiers this crate compares against are
    /// specified as ASCII strings, so there is no legitimate identifier this would compare wrong.
    /// A non-ASCII `LicenseId` (a source snapshot with a corrupted or non-SPDX license field)
    /// simply fails to match anything on the allowlist and `license_gate` fails closed, which is
    /// the correct outcome for a value that was never a valid SPDX identifier to begin with.
    pub fn spdx_matches(&self, other: &LicenseId) -> bool {
        self.0.trim().eq_ignore_ascii_case(other.0.trim())
    }
}

/// Seconds since the Unix epoch. A plain alias rather than a wrapping type or a `chrono`/`time`
/// dependency: nothing in this crate does date arithmetic on it yet, only records and compares it.
pub type Timestamp = u64;

/// Provenance for one source's fetch, independent of the entries it produced. Module 1's
/// per-source fetcher produces one of these alongside its `RawEntry`s; `fst_build`'s manifest
/// (module 4) carries it forward, and `gates::license_gate` (module 5) checks its `license`
/// against a checked-in allowlist. Defined here, ahead of `sources` (module 1, unbuilt) — the
/// same reason `RawEntry`/`MergedEntry` live here rather than in `merge.rs`: the modules that
/// construct and gate on this type need somewhere to import it from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SourceSnapshot {
    pub source: SourceId,
    pub revision: String,
    pub license: LicenseId,
    pub fetched_at: Timestamp,
}
