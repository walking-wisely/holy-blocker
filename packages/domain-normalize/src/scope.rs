//! Apex-eligibility: whether a normalized domain can carry subdomain-covering scope.
//!
//! Registrability is decided against the [Public Suffix List](https://publicsuffix.org/),
//! compiled in via the `psl` crate (no I/O, no network fetch at build or query time), backstopped
//! by a small checked-in shared-hosting denylist since the PSL is community-maintained and
//! incomplete. See `docs/decisions/domain-blocklist-sourcing.md`, "Combining sources" §1.

/// What a rule matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum RuleScope {
    /// This domain and everything under it.
    Apex,
    /// This exact name only.
    ExactHost,
}

/// Decides the scope a rule for `normalized` is entitled to.
///
/// `normalized` must already be the output of [`crate::normalize`] — this function does no
/// normalization of its own and trusts its caller to have applied it, so the same string always
/// classifies the same way regardless of how it reached this call.
///
/// Returns:
/// - `Some(RuleScope::Apex)` when `normalized` is itself a registrable domain (eTLD+1) under the
///   PSL and is not on `shared_hosting_denylist`.
/// - `Some(RuleScope::ExactHost)` when `normalized` sits below its registrable domain (e.g.
///   `foo.bar.example.com`, or `www.example.com`) — kept as-is, never promoted.
/// - `None` when `normalized` *is* a public suffix itself (`com`, `co.uk`, `s3.amazonaws.com`) or
///   is on `shared_hosting_denylist` — refused as an apex rule outright, and refused entirely
///   rather than silently downgraded, since a source that only ever meant to name a whole
///   multi-tenant provider has no valid rule left once apex is off the table.
pub fn classify_scope(normalized: &str, shared_hosting_denylist: &[&str]) -> Option<RuleScope> {
    if shared_hosting_denylist.contains(&normalized) {
        return None;
    }

    match psl::domain_str(normalized) {
        // `psl::domain_str` returns the registrable domain (eTLD+1) under `normalized`. When it
        // equals `normalized` exactly, `normalized` *is* the registrable domain — apex-eligible.
        Some(registrable) if registrable == normalized => Some(RuleScope::Apex),
        // A registrable domain exists but sits below `normalized`'s own name — `normalized` is
        // itself further down the tree than eTLD+1. Kept as `ExactHost`, never promoted.
        Some(_) => Some(RuleScope::ExactHost),
        // No registrable domain at all: `normalized` has nothing above the public suffix it
        // matches, i.e. it *is* the suffix (`com`, `co.uk`, `s3.amazonaws.com`, `blogspot.com`).
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_apex_is_apex() {
        assert_eq!(classify_scope("example.com", &[]), Some(RuleScope::Apex));
    }

    #[test]
    fn subdomain_is_exact_host() {
        assert_eq!(
            classify_scope("www.example.com", &[]),
            Some(RuleScope::ExactHost)
        );
        assert_eq!(
            classify_scope("foo.bar.example.com", &[]),
            Some(RuleScope::ExactHost)
        );
    }

    #[test]
    fn simple_public_suffix_is_never_apex() {
        assert_eq!(classify_scope("com", &[]), None);
    }

    #[test]
    fn multi_label_public_suffix_is_never_apex() {
        assert_eq!(classify_scope("co.uk", &[]), None);
    }

    #[test]
    fn private_registry_suffix_is_never_apex() {
        // s3.amazonaws.com and blogspot.com are themselves listed in the PSL (private section) —
        // naming them exactly must never grant apex-wide coverage over every tenant.
        assert_eq!(classify_scope("s3.amazonaws.com", &[]), None);
        assert_eq!(classify_scope("blogspot.com", &[]), None);
    }

    #[test]
    fn tenant_domain_under_a_provider_suffix_is_apex() {
        // someone.blogspot.com IS the registrable domain under the blogspot.com suffix — that's
        // the tenant's own registrable space, and is legitimately Apex.
        assert_eq!(
            classify_scope("someone.blogspot.com", &[]),
            Some(RuleScope::Apex)
        );
    }

    #[test]
    fn deep_name_under_a_provider_suffix_is_exact_host() {
        assert_eq!(
            classify_scope("cdn.someone.blogspot.com", &[]),
            Some(RuleScope::ExactHost)
        );
    }

    #[test]
    fn shared_hosting_denylist_beats_an_otherwise_apex_eligible_domain() {
        assert_eq!(classify_scope("example.com", &["example.com"]), None);
    }

    #[test]
    fn shared_hosting_denylist_does_not_affect_other_domains() {
        assert_eq!(
            classify_scope("other.com", &["example.com"]),
            Some(RuleScope::Apex)
        );
    }

    #[test]
    fn www_subdomain_matches_only_itself_not_the_apex_or_a_sibling() {
        // Documented in the plan: an ExactHost rule for www.example.com must match a query for
        // www.example.com and must not match example.com or cdn.example.com. classify_scope's
        // part of that contract is just returning ExactHost for www.example.com and Apex for
        // example.com as two independent, non-aliased keys — the matching itself is the FST
        // consumer's job (module 4), not this function's.
        assert_eq!(
            classify_scope("www.example.com", &[]),
            Some(RuleScope::ExactHost)
        );
        assert_eq!(classify_scope("example.com", &[]), Some(RuleScope::Apex));
    }
}
