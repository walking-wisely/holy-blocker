//! The persistent liveness cache's per-entry shape, and the pure decisions built on it:
//! [`due_for_check`] (is a cached verdict still trusted?) and [`should_prune`] (does the current
//! verdict prune the domain?). Storage and load/save I/O are left to `cli` (module 7, unbuilt) —
//! see the parent module's doc comment.

#[cfg(test)]
use super::lookup::UnknownReason;
use super::lookup::Verdict;
use crate::types::Timestamp;

/// One entry in the persistent liveness cache the plan describes (`domain → { last_checked,
/// verdict }`, storage and load/save I/O left to `cli`, module 7 — see this module's doc comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheEntry {
    pub last_checked: Timestamp,
    pub verdict: Verdict,
}

/// Whether a cached verdict is still trusted or needs a fresh lookup. `cache_entry` is `None` for
/// a domain never checked before, which is always due — a build never treats "we have no prior
/// information" as a reason to skip checking it.
///
/// `ttl_seconds` is the only threshold this function takes; the plan's cadence (how often a sweep
/// runs at all) is a separate, coarser-grained scheduling concern the caller owns and is
/// deliberately **not** a parameter here — TTL is what this per-domain decision is actually made
/// against, decoupled from and a multiple of the run cadence (plan: "cadence monthly, TTL 3
/// cadences"), and folding cadence into this signature would let a caller conflate the two.
///
/// Uses `saturating_sub` so a `now` before `last_checked` (clock skew, or a cache entry from a
/// future-dated run) reads as "just checked" rather than underflowing into a bogus multi-decade
/// gap that would look due no matter the TTL.
pub fn due_for_check(cache_entry: Option<&CacheEntry>, now: Timestamp, ttl_seconds: u64) -> bool {
    match cache_entry {
        None => true,
        Some(entry) => now.saturating_sub(entry.last_checked) >= ttl_seconds,
    }
}

/// The IANA [Special-Use Domain
/// Names](https://www.iana.org/assignments/special-use-domain-names/special-use-domain-names.xhtml)
/// registry entries relevant to liveness pruning — names a compliant resolver either never queries
/// upstream for, or that were never meant to resolve through the public DNS hierarchy at all:
///
/// - `.onion` — [RFC 7686](https://www.rfc-editor.org/rfc/rfc7686): resolvers are instructed to
///   answer NXDOMAIN **locally**, without ever querying upstream. A liveness check on a `.onion`
///   name is therefore *specified* to return `Dead` on every honest resolver, deterministically,
///   on every sweep — a guaranteed false prune, not evidence of anything about the name.
/// - `.local` — [RFC 6762](https://www.rfc-editor.org/rfc/rfc6762) (mDNS).
/// - `.localhost` — [RFC 6761](https://www.rfc-editor.org/rfc/rfc6761).
/// - `.test`, `.invalid`, `.example` — [RFC 2606](https://www.rfc-editor.org/rfc/rfc2606) /
///   [RFC 6761](https://www.rfc-editor.org/rfc/rfc6761).
///
/// Matches by TLD label only (not substring — `notonion.com` and `onion.com` are ordinary
/// registered domains, not the reserved TLD), case-insensitively, and tolerates one trailing dot
/// the same way a normalized domain would carry one.
///
/// Rejecting these at merge time (a different crate/module) is the primary defence; this is
/// defence in depth so the sweep is not merely discouraged from producing a false `Dead` prune on
/// such a name but structurally unable to.
pub fn is_special_use_domain(domain: &str) -> bool {
    const SPECIAL_USE_TLDS: &[&str] =
        &["onion", "local", "localhost", "test", "invalid", "example"];

    let trimmed = domain.strip_suffix('.').unwrap_or(domain);
    match trimmed.rsplit_once('.') {
        Some((_, tld)) => SPECIAL_USE_TLDS
            .iter()
            .any(|special| tld.eq_ignore_ascii_case(special)),
        // A bare, dot-free label (e.g. "onion" or "localhost" with no further labels) is itself
        // the TLD — still worth catching rather than silently falling through.
        None => SPECIAL_USE_TLDS
            .iter()
            .any(|special| trimmed.eq_ignore_ascii_case(special)),
    }
}

/// Only a `Dead` verdict ever prunes a domain from the next build's output — an `Unknown` domain,
/// first-seen or previously cached, is always kept. See the plan's "erring toward keeping an
/// entry is the intended direction — false negatives are the budget, false positives are the
/// price" rule. Note this function only ever sees the current verdict, not the prior one: an
/// `Unknown` is always "keep," which is *not* the same as the plan's "left exactly as it was" for
/// a domain that was previously pruned as `Dead` — that domain is re-included here, since this
/// signature has no visibility into what a past build did. Still the safe direction: a wrongly
/// resurrected domain costs a wasted slot, a wrongly kept one is the intended failure mode.
///
/// `domain` is checked against [`is_special_use_domain`] before anything else: a `Dead` verdict on
/// a special-use name (e.g. `.onion`, RFC 7686) is never proof the name is actually dead — it is
/// what every honest resolver is specified to answer for that name regardless of registration
/// state — so this function refuses to prune such a name no matter what `verdict` says. This is
/// defence in depth; rejecting these entries earlier (merge time, a different crate/module) is the
/// primary control, and this guard is what makes the failure this reviewer flagged impossible
/// rather than merely prevented upstream.
pub fn should_prune(domain: &str, verdict: Verdict) -> bool {
    if is_special_use_domain(domain) {
        return false;
    }
    matches!(verdict, Verdict::Dead)
}

#[cfg(test)]
mod tests {
    use super::*;

    mod due_for_check_tests {
        use super::*;

        #[test]
        fn a_domain_never_checked_before_is_due() {
            assert!(due_for_check(None, 1_000, 300));
        }

        #[test]
        fn a_freshly_checked_entry_is_not_due() {
            let entry = CacheEntry {
                last_checked: 1_000,
                verdict: Verdict::Alive,
            };
            assert!(!due_for_check(Some(&entry), 1_001, 300));
        }

        #[test]
        fn an_entry_exactly_at_the_ttl_boundary_is_due() {
            let entry = CacheEntry {
                last_checked: 1_000,
                verdict: Verdict::Alive,
            };
            assert!(due_for_check(Some(&entry), 1_300, 300));
        }

        #[test]
        fn an_entry_one_second_under_the_ttl_is_not_due() {
            let entry = CacheEntry {
                last_checked: 1_000,
                verdict: Verdict::Alive,
            };
            assert!(!due_for_check(Some(&entry), 1_299, 300));
        }

        #[test]
        fn clock_skew_where_now_precedes_last_checked_is_not_due() {
            // now < last_checked (a future-dated cache entry, or a clock correction) must not
            // underflow into a bogus multi-decade gap that reads as due regardless of TTL.
            let entry = CacheEntry {
                last_checked: 10_000,
                verdict: Verdict::Alive,
            };
            assert!(!due_for_check(Some(&entry), 1_000, 300));
        }

        #[test]
        fn ttl_is_independent_of_any_notion_of_cadence() {
            // Named per the plan's own trickiest case: cadence and TTL are different numbers
            // (cadence monthly, TTL ~3 cadences) and this function only ever sees the TTL — a
            // cached entry 90 days old is due against a 90-day TTL regardless of how often the
            // caller's sweep runs.
            let ninety_days = 90 * 24 * 60 * 60;
            let entry = CacheEntry {
                last_checked: 0,
                verdict: Verdict::Alive,
            };
            assert!(due_for_check(Some(&entry), ninety_days, ninety_days));
            assert!(!due_for_check(Some(&entry), ninety_days - 1, ninety_days));
        }
    }

    mod should_prune_tests {
        use super::*;

        #[test]
        fn only_dead_prunes() {
            assert!(should_prune("dead.example.com", Verdict::Dead));
            assert!(!should_prune("dead.example.com", Verdict::Alive));
            assert!(!should_prune(
                "dead.example.com",
                Verdict::Unknown(UnknownReason::Timeout)
            ));
        }

        #[test]
        fn a_first_seen_unknown_is_kept_the_same_as_a_previously_cached_unknown() {
            // Named separately per the plan: first-seen-Unknown and previously-cached-Unknown
            // must both be retained, not just "Unknown in general" by coincidence of one test.
            let first_seen = Verdict::Unknown(UnknownReason::NoData);
            let previously_cached = Verdict::Unknown(UnknownReason::ServFail);
            assert!(!should_prune("dead.example.com", first_seen));
            assert!(!should_prune("dead.example.com", previously_cached));
        }

        #[test]
        fn a_dead_onion_domain_is_never_pruned() {
            // RFC 7686 makes .onion special-use and instructs resolvers to answer NXDOMAIN
            // locally without ever querying upstream — so a liveness check on a .onion name is
            // *specified* to return Dead on every honest resolver, deterministically, every
            // sweep. A Dead verdict on a name the sweep structurally cannot evaluate must never
            // be able to prune it.
            assert!(!should_prune("abcdefghijklmnop.onion", Verdict::Dead));
        }

        #[test]
        fn an_ordinary_dead_domain_is_still_pruned() {
            // The special-use guard must not become a blanket "never prune" escape hatch — an
            // everyday registered domain that genuinely died is unaffected.
            assert!(should_prune("dead.example.com", Verdict::Dead));
        }

        #[test]
        fn onion_case_and_trailing_dot_variants_are_still_caught() {
            assert!(!should_prune("example.ONION", Verdict::Dead));
            assert!(!should_prune("example.onion.", Verdict::Dead));
            assert!(!should_prune("EXAMPLE.ONION.", Verdict::Dead));
        }

        #[test]
        fn other_special_use_names_are_never_pruned_either() {
            // RFC 6762 (.local), RFC 6761 (.localhost), RFC 2606/6761 (.test/.invalid/.example) —
            // every one of these is specified to never resolve through the normal DNS hierarchy
            // the same way .onion is, so a Dead verdict on any of them is equally meaningless.
            assert!(!should_prune("printer.local", Verdict::Dead));
            assert!(!should_prune("myhost.localhost", Verdict::Dead));
            assert!(!should_prune("site.test", Verdict::Dead));
            assert!(!should_prune("thing.invalid", Verdict::Dead));
            assert!(!should_prune("thing.example", Verdict::Dead));
        }
    }

    mod is_special_use_domain_tests {
        use super::*;

        #[test]
        fn recognizes_onion() {
            assert!(is_special_use_domain("abcdefghijklmnop.onion"));
        }

        #[test]
        fn recognizes_local_localhost_test_invalid_example() {
            assert!(is_special_use_domain("printer.local"));
            assert!(is_special_use_domain("myhost.localhost"));
            assert!(is_special_use_domain("site.test"));
            assert!(is_special_use_domain("thing.invalid"));
            assert!(is_special_use_domain("thing.example"));
        }

        #[test]
        fn is_case_insensitive() {
            assert!(is_special_use_domain("Example.ONION"));
        }

        #[test]
        fn tolerates_a_trailing_dot() {
            assert!(is_special_use_domain("example.onion."));
        }

        #[test]
        fn does_not_flag_a_label_that_merely_contains_the_tld_as_a_substring() {
            // A domain like "notonion.com" or "onion.com" itself (registered, not the reserved
            // TLD) must not be caught by a naive substring/suffix check.
            assert!(!is_special_use_domain("notonion.com"));
            assert!(!is_special_use_domain("onion.com"));
        }

        #[test]
        fn ordinary_domains_are_unaffected() {
            assert!(!is_special_use_domain("example.com"));
            assert!(!is_special_use_domain("example.invalid.com"));
        }
    }
}
