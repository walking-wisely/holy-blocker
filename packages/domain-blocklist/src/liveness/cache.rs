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

/// Only a `Dead` verdict ever prunes a domain from the next build's output — an `Unknown` domain,
/// first-seen or previously cached, is always kept. See the plan's "erring toward keeping an
/// entry is the intended direction — false negatives are the budget, false positives are the
/// price" rule. Note this function only ever sees the current verdict, not the prior one: an
/// `Unknown` is always "keep," which is *not* the same as the plan's "left exactly as it was" for
/// a domain that was previously pruned as `Dead` — that domain is re-included here, since this
/// signature has no visibility into what a past build did. Still the safe direction: a wrongly
/// resurrected domain costs a wasted slot, a wrongly kept one is the intended failure mode.
pub fn should_prune(verdict: Verdict) -> bool {
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
            assert!(should_prune(Verdict::Dead));
            assert!(!should_prune(Verdict::Alive));
            assert!(!should_prune(Verdict::Unknown(UnknownReason::Timeout)));
        }

        #[test]
        fn a_first_seen_unknown_is_kept_the_same_as_a_previously_cached_unknown() {
            // Named separately per the plan: first-seen-Unknown and previously-cached-Unknown
            // must both be retained, not just "Unknown in general" by coincidence of one test.
            let first_seen = Verdict::Unknown(UnknownReason::NoData);
            let previously_cached = Verdict::Unknown(UnknownReason::ServFail);
            assert!(!should_prune(first_seen));
            assert!(!should_prune(previously_cached));
        }
    }
}
