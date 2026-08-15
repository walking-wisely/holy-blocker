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
/// - `Some(RuleScope::ExactHost)` in every other case that isn't on `shared_hosting_denylist` —
///   both when `normalized` sits below its registrable domain (e.g. `foo.bar.example.com`,
///   `www.example.com`) and when `normalized` *is itself* the public suffix boundary (`com`,
///   `co.uk`, or a wildcard-tenant slot like `s3.amazonaws.com`/`someone-else.ec2-x.
///   compute-1.amazonaws.com`'s own suffix rule). The PSL fact "this is a suffix" only ever vetoes
///   *`Apex`* — claiming the whole shared boundary would blackhole every other tenant under it —
///   it never vetoes naming this one exact literal string, which can't match anything but that one
///   query regardless of what PSL says about it. Refusing the rule entirely here would silently
///   drop a source's specific, real, individually-resolvable hostname (measured: ~20 such entries
///   in one real ~4.78M-domain merge, e.g. `pornslut.cn.st` — a wildcard-PSL-listed provider's own
///   tenant boundary, structurally identical to `someone.blogspot.com` one level up) for no safety
///   reason.
/// - `None` only when `normalized` is on `shared_hosting_denylist` — a distinct, deliberate
///   operator decision ("never make a rule targeting this domain, full stop", e.g. a shared
///   provider's own landing page nobody wants blocked even exactly) rather than a PSL fact about
///   scope, so it stays a hard refusal instead of downgrading.
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
        // matches, i.e. it *is* the suffix (`com`, `co.uk`, `s3.amazonaws.com`, `blogspot.com`,
        // or one specific tenant's own wildcard-PSL boundary like `pornslut.cn.st`). Never Apex —
        // but the literal string itself is exactly as safe to name as any other ExactHost.
        None => Some(RuleScope::ExactHost),
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
    fn simple_public_suffix_is_exact_host_never_apex() {
        // "com" is itself a public suffix — never Apex (would claim the entire TLD) — but naming
        // it exactly is still a safe, literal ExactHost rule, so it's no longer dropped outright.
        assert_eq!(classify_scope("com", &[]), Some(RuleScope::ExactHost));
    }

    #[test]
    fn multi_label_public_suffix_is_exact_host_never_apex() {
        assert_eq!(classify_scope("co.uk", &[]), Some(RuleScope::ExactHost));
    }

    #[test]
    fn private_registry_suffix_is_exact_host_never_apex() {
        // s3.amazonaws.com and blogspot.com are themselves listed in the PSL (private section) —
        // naming them exactly must never grant apex-wide coverage over every tenant, but the
        // literal provider root itself is still a safe ExactHost rule (it only ever matches that
        // one exact query, never a tenant subdomain).
        assert_eq!(
            classify_scope("s3.amazonaws.com", &[]),
            Some(RuleScope::ExactHost)
        );
        assert_eq!(
            classify_scope("blogspot.com", &[]),
            Some(RuleScope::ExactHost)
        );
    }

    #[test]
    fn wildcard_tenant_boundary_is_exact_host_never_apex() {
        // Some PSL private-section entries are wildcards (e.g. AWS's own `*.compute-1.
        // amazonaws.com`), which pushes the suffix boundary one level lower than a plain rule
        // like `blogspot.com` does: the *tenant's own* assigned name (an EC2 instance's public
        // DNS host) is itself the PSL boundary, not one level above it — structurally identical
        // to `someone.blogspot.com`, just shaped differently by PSL. Still never Apex (claiming
        // it would cover whatever, if anything, gets registered one level further down), but the
        // literal, specific, individually-assigned hostname is exactly as safe to block as any
        // other ExactHost rule — this is the real gap a live run against ~4.78M real merged
        // domains found: ~20 entries like this were being dropped outright instead.
        assert_eq!(
            classify_scope("ec2-100-26-145-53.compute-1.amazonaws.com", &[]),
            Some(RuleScope::ExactHost)
        );
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
