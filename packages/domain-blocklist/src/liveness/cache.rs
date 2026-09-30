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
///
/// `first_dead_at` is the hysteresis primitive: the timestamp of the **first** sweep in an
/// unbroken run of `Dead` reads, `None` whenever the most recent sweep read anything else
/// (`Alive` or `Unknown`). It is a "since when has this been continuously dead" timestamp rather
/// than a raw streak counter because that composes directly with this module's TTL-based cadence
/// (`due_for_check`'s `ttl_seconds`) — a quarantine window is just another duration compared
/// against `now`, the same shape `due_for_check` already uses, rather than a count that would need
/// its own separate "how many sweeps is 35 days" conversion.
///
/// # Why a single bad `Dead` answer must not immediately prune
///
/// DNS has several states where a registered, soon-to-return domain NXDOMAINs *right now*:
///
/// - Registrar **clientHold**/**serverHold** removes the name from the zone while the
///   registration itself is fully intact — abuse complaints and payment disputes put adult
///   domains on hold more than average, and holds are routinely lifted.
/// - **Redemption Grace Period**: an expired domain leaves the zone but is restorable by the
///   original registrant for 30 days, then sits pending-delete for a further 5. A sweep that
///   catches it as NXDOMAIN on day 1 would otherwise drop the entry from the build outright, even
///   though the domain can come back as late as day 20.
/// - Registry/registrar transfer and DNS-operator migration windows, which can NXDOMAIN a
///   perfectly live domain for the duration of the cutover.
///
/// At one sweep per month with a 3-cadence TTL, a single bad answer previously removed an entry
/// for roughly 3 months with no way back short of a fresh discovery. Requiring `Dead` to persist
/// across sweeps for a quarantine window — recommended at ~35 days, past the Redemption Grace
/// Period above — turns that into a bounded quarantine instead of an outright, immediate removal.
/// See [`should_prune_with_hysteresis`] for the pure decision built on this field.
///
/// This module only *stores* the field and exposes the pure decision over it — it performs no
/// I/O, so it is `cli` (module 7, unbuilt) that is responsible for actually setting/clearing
/// `first_dead_at` when it writes the cache back after each sweep: set it to `now` the first time
/// a sweep reads `Dead` from an entry whose previous `first_dead_at` was `None`, leave it
/// unchanged on a subsequent `Dead` read, and clear it back to `None` the moment a sweep reads
/// anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheEntry {
    pub last_checked: Timestamp,
    pub verdict: Verdict,
    pub first_dead_at: Option<Timestamp>,
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
///
/// **`verdict` MUST be a *corroborated* verdict** — the output of
/// [`corroborate`](super::corroborate) (or [`check_corroborated`](super::check_corroborated))
/// applied across two independently-configured resolvers, never a single resolver's own
/// [`check`](super::lookup::check) result. See [`corroborate`](super::corroborate)'s doc comment
/// for why a lone resolver's `Dead` is no longer sufficient proof on its own.
///
/// # This is not the same claim [`canary_check`](super::canary_check) makes about the same names
///
/// It can look contradictory that this function treats a `Dead` verdict on `.invalid`/`.test` as
/// proof of nothing, while [`canary_check`](super::canary_check) uses exactly those names —
/// expecting `Dead` from them — as its evidence the resolver is behaving honestly. They are not in
/// tension: the two functions are asking different questions about the same `Dead` verdict. This
/// function asks "does a `Dead` verdict *for this specific domain* prove that domain is
/// unregistered?" — and for a special-use name the answer is no, because every honest resolver is
/// specified to answer `Dead` for it regardless of what's actually registered, so the verdict
/// carries zero information about registration state. `canary_check` asks a different question
/// entirely: "does the resolver's *behavior* on a name whose correct answer is known in advance
/// match that known-correct answer?" — for a name specified to always be `Dead`, an honest
/// resolver reporting anything else (`Alive`, most dangerously) is exactly what identifies a
/// wildcard-sink resolver, regardless of whether the domain itself is "really" registered. Put
/// another way: this function is about what a verdict proves *about a domain*; `canary_check` is
/// about what a verdict proves *about a resolver*. A `Dead` result on `.invalid` is uninformative
/// evidence for the first question and definitive evidence for the second, simultaneously and
/// without contradiction.
pub fn should_prune(domain: &str, verdict: Verdict) -> bool {
    if is_special_use_domain(domain) {
        return false;
    }
    matches!(verdict, Verdict::Dead)
}

/// The hysteresis-aware companion to [`should_prune`]: a domain is only pruned once it has been
/// **continuously** `Dead` for at least `quarantine_seconds`, per [`CacheEntry::first_dead_at`]'s
/// doc comment on why a single bad NXDOMAIN (registrar hold, Redemption Grace Period, a transfer
/// window) must not immediately drop an entry. Composes with, rather than replaces,
/// [`should_prune`]: the special-use guard and the "only `Dead` prunes at all" rule are identical,
/// this just adds a minimum-duration requirement on top of them.
///
/// `entry` is the cache entry as read **before** this sweep updates it — `first_dead_at` reflects
/// how long the domain has been continuously dead as of the *previous* sweep, and `verdict` is the
/// verdict from the *current* sweep. A caller combining the two this way (rather than pre-updating
/// `entry.first_dead_at` before calling) keeps this function pure and free of the "did I already
/// advance the streak" bookkeeping `cli` (module 7, unbuilt) owns when it writes the cache back.
///
/// Three cases:
/// - `verdict` is not `Dead`: never pruned, regardless of `entry` — a domain that resolved (or was
///   merely `Unknown`) this sweep has its dead streak broken, full stop.
/// - `verdict` is `Dead` and `entry.first_dead_at` is `None` (this is the first sweep to see it
///   dead): not pruned yet — one `Dead` read alone proves nothing, per the RGP/hold reasoning
///   above.
/// - `verdict` is `Dead` and `entry.first_dead_at` is `Some(first)`: pruned only once
///   `now - first >= quarantine_seconds`. Uses `saturating_sub` for the same clock-skew reason
///   `due_for_check` does — a `first` timestamp after `now` reads as "just went dead", not as a
///   bogus multi-decade streak.
///
/// `quarantine_seconds` is caller-supplied, not hardcoded, matching `due_for_check`'s
/// `ttl_seconds` convention — this module keeps thresholds as parameters. ~35 days (past the
/// Redemption Grace Period's 30+5-day window) is the recommended value a caller might pass.
///
/// Like [`should_prune`], **`verdict` (and, transitively, every `Dead` read that ever set
/// `entry.first_dead_at`) MUST be a corroborated verdict** — see
/// [`corroborate`](super::corroborate). This matters for a class of cause this hysteresis window
/// does not, by itself, cover: a *deterministic* false-`Dead` cause specific to one resolver's own
/// implementation (e.g. an over-aggressive QNAME-minimization implementation misbehaving against
/// an authoritative server that predates [RFC 8020](https://www.rfc-editor.org/rfc/rfc8020))
/// reproduces the same wrong answer on every sweep against that resolver, so a domain affected by
/// it would eventually age past any quarantine window and be pruned regardless of how long that
/// window is — this hysteresis mechanism only protects against causes that are transient in time
/// (a registrar hold, an RGP window, a transfer), not causes that are constant in time but
/// specific to one resolver's software. Tracking `first_dead_at` off the *corroborated* verdict
/// closes that gap: a domain that a buggy resolver reads `Dead` forever, but a second,
/// differently-implemented resolver never agrees with, never becomes `Dead` in the corroborated
/// verdict in the first place, so `first_dead_at` is never set and this quarantine logic never
/// even engages for it — the two mechanisms are complementary, covering time-bounded and
/// resolver-specific-deterministic causes respectively, and neither substitutes for the other. See
/// `corroboration`'s test module for a regression test pinning this composition down directly.
pub fn should_prune_with_hysteresis(
    entry: &CacheEntry,
    domain: &str,
    verdict: Verdict,
    now: Timestamp,
    quarantine_seconds: u64,
) -> bool {
    if !should_prune(domain, verdict) {
        return false;
    }
    match entry.first_dead_at {
        None => false,
        Some(first_dead_at) => now.saturating_sub(first_dead_at) >= quarantine_seconds,
    }
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
                first_dead_at: None,
            };
            assert!(!due_for_check(Some(&entry), 1_001, 300));
        }

        #[test]
        fn an_entry_exactly_at_the_ttl_boundary_is_due() {
            let entry = CacheEntry {
                last_checked: 1_000,
                verdict: Verdict::Alive,
                first_dead_at: None,
            };
            assert!(due_for_check(Some(&entry), 1_300, 300));
        }

        #[test]
        fn an_entry_one_second_under_the_ttl_is_not_due() {
            let entry = CacheEntry {
                last_checked: 1_000,
                verdict: Verdict::Alive,
                first_dead_at: None,
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
                first_dead_at: None,
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
                first_dead_at: None,
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

    mod should_prune_with_hysteresis_tests {
        use super::*;

        const THIRTY_FIVE_DAYS: u64 = 35 * 24 * 60 * 60;

        #[test]
        fn a_domain_freshly_gone_dead_with_no_prior_streak_is_not_pruned() {
            // The entry as read before this sweep has never seen Dead before (first_dead_at is
            // None) — one Dead read alone must not prune, per the RGP/hold reasoning.
            let entry = CacheEntry {
                last_checked: 1_000,
                verdict: Verdict::Unknown(UnknownReason::Timeout),
                first_dead_at: None,
            };
            assert!(!should_prune_with_hysteresis(
                &entry,
                "dead.example.com",
                Verdict::Dead,
                1_000,
                THIRTY_FIVE_DAYS,
            ));
        }

        #[test]
        fn a_domain_freshly_gone_dead_where_first_dead_at_equals_now_is_not_pruned() {
            // The other legitimate shape of "just went dead": first_dead_at was already set to
            // now by the caller on this same sweep. Zero elapsed time is still under any
            // meaningful quarantine window.
            let entry = CacheEntry {
                last_checked: 1_000,
                verdict: Verdict::Dead,
                first_dead_at: Some(1_000),
            };
            assert!(!should_prune_with_hysteresis(
                &entry,
                "dead.example.com",
                Verdict::Dead,
                1_000,
                THIRTY_FIVE_DAYS,
            ));
        }

        #[test]
        fn a_domain_dead_for_less_than_the_quarantine_window_is_not_pruned() {
            let entry = CacheEntry {
                last_checked: 1_000,
                verdict: Verdict::Dead,
                first_dead_at: Some(0),
            };
            assert!(!should_prune_with_hysteresis(
                &entry,
                "dead.example.com",
                Verdict::Dead,
                THIRTY_FIVE_DAYS - 1,
                THIRTY_FIVE_DAYS,
            ));
        }

        #[test]
        fn a_domain_dead_for_at_least_the_quarantine_window_is_pruned() {
            let entry = CacheEntry {
                last_checked: 1_000,
                verdict: Verdict::Dead,
                first_dead_at: Some(0),
            };
            assert!(should_prune_with_hysteresis(
                &entry,
                "dead.example.com",
                Verdict::Dead,
                THIRTY_FIVE_DAYS,
                THIRTY_FIVE_DAYS,
            ));
        }

        #[test]
        fn a_domain_that_flips_back_to_alive_clears_the_dead_streak_state() {
            // Tests the caller-facing contract: a fresh CacheEntry with verdict: Alive and no
            // dead-streak info (first_dead_at: None) needs no special handling — passing the
            // *current* sweep's Alive verdict is never pruned, regardless of a long-past streak
            // still recorded in first_dead_at (which a real caller would also be clearing, but
            // this function's own rule already makes that moot: a non-Dead verdict is never
            // pruned no matter what entry says).
            let entry = CacheEntry {
                last_checked: 1_000,
                verdict: Verdict::Dead,
                first_dead_at: Some(0),
            };
            assert!(!should_prune_with_hysteresis(
                &entry,
                "revived.example.com",
                Verdict::Alive,
                THIRTY_FIVE_DAYS * 2,
                THIRTY_FIVE_DAYS,
            ));

            // And the ordinary fresh-entry shape: Alive verdict, no dead-streak history at all.
            let fresh = CacheEntry {
                last_checked: 1_000,
                verdict: Verdict::Alive,
                first_dead_at: None,
            };
            assert!(!should_prune_with_hysteresis(
                &fresh,
                "revived.example.com",
                Verdict::Alive,
                1_000,
                THIRTY_FIVE_DAYS,
            ));
        }

        #[test]
        fn a_dead_onion_domain_past_the_quarantine_window_is_still_never_pruned() {
            // The special-use guard composes through should_prune, so it must still hold here.
            let entry = CacheEntry {
                last_checked: 1_000,
                verdict: Verdict::Dead,
                first_dead_at: Some(0),
            };
            assert!(!should_prune_with_hysteresis(
                &entry,
                "abcdefghijklmnop.onion",
                Verdict::Dead,
                THIRTY_FIVE_DAYS,
                THIRTY_FIVE_DAYS,
            ));
        }

        #[test]
        fn clock_skew_where_now_precedes_first_dead_at_is_not_pruned() {
            // saturating_sub, mirroring due_for_check's own clock-skew handling: a first_dead_at
            // after now must read as "just went dead," not underflow into a bogus streak.
            let entry = CacheEntry {
                last_checked: 1_000,
                verdict: Verdict::Dead,
                first_dead_at: Some(10_000),
            };
            assert!(!should_prune_with_hysteresis(
                &entry,
                "dead.example.com",
                Verdict::Dead,
                1_000,
                THIRTY_FIVE_DAYS,
            ));
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
