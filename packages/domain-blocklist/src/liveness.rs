//! Module 3 of the domain-blocklist plan — DNS-only liveness revalidation.
//!
//! This module is pure decision logic behind a [`DnsLookup`] trait, per the plan's own
//! implementation order: `due_for_check` first (the trickiest edge cases — reappeared-stale vs.
//! genuinely-revived entries), then `canary_check` against a fake resolver simulating a healthy,
//! a family-filtering, and a wildcard-sink resolver, then [`check`] combining two per-domain
//! lookups. **The real network resolver, the persistent [`CacheEntry`] store, and the ~24-hour
//! qps-paced sweep loop are left to `cli` (module 7, unbuilt)** — the same split module 1 draws
//! between its pure parsers and the pipeline binary's real HTTP client. Nothing here does I/O or
//! needs live network access to test.
//!
//! # Reference documents
//!
//! - [RFC 1035 §4.1.1](https://www.rfc-editor.org/rfc/rfc1035#section-4.1.1) — the RCODE field a
//!   real resolver's NXDOMAIN/SERVFAIL/REFUSED answers are read from; [`LookupResult`] is this
//!   crate's typed stand-in for that outcome.
//! - [RFC 3596 §2.1](https://www.rfc-editor.org/rfc/rfc3596#section-2.1) — the AAAA record type,
//!   why [`check`] issues two lookups per domain rather than one.
//! - [RFC 8482](https://www.rfc-editor.org/rfc/rfc8482) — why `QTYPE=ANY` cannot collapse the A
//!   and AAAA queries into one; there is no shortcut, hence [`RecordType`] has exactly two values.
//! - [RFC 2606](https://www.rfc-editor.org/rfc/rfc2606) and
//!   [RFC 6761](https://www.rfc-editor.org/rfc/rfc6761) — the reserved, guaranteed-never-to-resolve
//!   names [`CanaryConfig::dead_controls`] is meant to be drawn from.
//! - [RFC 2308 §2.2](https://www.rfc-editor.org/rfc/rfc2308#section-2.2) — NODATA and negative-
//!   caching semantics; RFC 1035 §4.1.1 defines the RCODE field itself but not the NODATA case
//!   (RCODE 0, no matching record), which is what [`UnknownReason::NoData`] models. RFC 2308 §2.1
//!   is the equivalent NXDOMAIN negative-caching definition.

use crate::types::Timestamp;

/// Why a single-QTYPE lookup could not be read as a definite answer. Never used to prune a
/// domain on its own — see [`Verdict::Unknown`] and the plan's "a resolver hiccup or a transient
/// SERVFAIL must never be able to silently shrink the blocklist" rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownReason {
    /// A response with no error but no matching record (RCODE 0, empty answer) — the NODATA case,
    /// whose semantics come from RFC 2308 §2.2, not RFC 1035 §4.1.1 (which defines only the RCODE
    /// field, not what an empty-answer/RCODE-0 response means).
    NoData,
    /// RFC 1035 §4.1.1 RCODE 2.
    ServFail,
    /// RFC 1035 §4.1.1 RCODE 5 — the resolver declined to answer at all.
    Refused,
    /// No response arrived within the resolver client's deadline.
    Timeout,
    /// A response arrived but could not be parsed as a valid DNS message.
    Malformed,
}

/// The outcome of one [`DnsLookup::lookup`] call — one QTYPE, one domain. Never conflated with
/// [`Verdict`], which is what two of these (A and AAAA) combine into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupResult {
    /// An address record, or a CNAME chain that resolved to one.
    Resolved,
    /// An unambiguous negative answer (RFC 1035 §4.1.1 RCODE 3).
    NxDomain,
    Unknown(UnknownReason),
}

/// Per-domain liveness, from combining both address-family lookups. **Not a boolean** — see the
/// plan's module 3: only [`Verdict::Dead`] ever prunes a domain, and [`Verdict::Unknown`] must
/// leave a previously-included domain exactly as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Alive,
    Dead,
    Unknown(UnknownReason),
}

/// Combines one A and one AAAA [`LookupResult`] into a single [`Verdict`], per the plan's
/// evaluation order:
///
/// 1. Either lookup resolving is enough — one working address family means the domain is
///    reachable.
/// 2. Otherwise, both unambiguously `NxDomain` is `Dead`.
/// 3. Otherwise, at least one side is `Unknown` and neither is `Resolved` — the result is
///    `Unknown`. This is the mixed `NxDomain`/`Unknown` case: one family failing to resolve while
///    the other is merely inconclusive must never be conflated with both families cleanly saying
///    the domain doesn't exist.
///
/// When both sides are `Unknown` with different reasons, the A side's reason is kept — an
/// arbitrary but deterministic tie-break; the plan does not specify one, and only the `Verdict`
/// variant (never the specific reason) affects pruning.
fn combine(a: LookupResult, aaaa: LookupResult) -> Verdict {
    match (a, aaaa) {
        (LookupResult::Resolved, _) | (_, LookupResult::Resolved) => Verdict::Alive,
        (LookupResult::NxDomain, LookupResult::NxDomain) => Verdict::Dead,
        (LookupResult::Unknown(reason), _) => Verdict::Unknown(reason),
        (_, LookupResult::Unknown(reason)) => Verdict::Unknown(reason),
    }
}

/// Which address-family query a [`DnsLookup`] answers. Exactly two variants, per RFC 8482 — there
/// is no `Any` shortcut here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordType {
    A,
    Aaaa,
}

/// One DNS lookup, behind a trait so [`check`] and [`canary_check`] need no live network access
/// to test. The pipeline binary (module 7, unbuilt) wires in the real client — an explicitly
/// configured resolver (defaulting to `1.1.1.1` / `2606:4700:4700::1111`, never the filtering
/// `1.1.1.2`/`1.1.1.3` variants, and never the host's system resolver), per the plan.
pub trait DnsLookup {
    fn lookup(&self, domain: &str, record: RecordType) -> LookupResult;
}

/// Looks up `domain`'s liveness: one A and one AAAA query, combined per [`combine`]'s ordering.
pub fn check<R: DnsLookup + ?Sized>(resolver: &R, domain: &str) -> Verdict {
    let a = resolver.lookup(domain, RecordType::A);
    let aaaa = resolver.lookup(domain, RecordType::Aaaa);
    combine(a, aaaa)
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

/// A fixed set of domains with a **known** expected [`Verdict`] (never `Unknown`), used to sanity
/// check the resolver itself before trusting anything it says about the real sweep. The two lists
/// are deliberately the same shape — both are "domains this build already knows the answer for" —
/// because a resolver can misbehave in either direction: a content-filtering resolver NXDOMAINs
/// real sites it blocks (caught by `alive_controls`, several known-always-alive, definitely-non-
/// adult domains), and a wildcard-sink resolver resolves everything, including names that must
/// never resolve (caught by `dead_controls`, drawn from the RFC 2606/6761 reserved, guaranteed-
/// never-to-resolve names). `dead_controls` takes more than one for the same reason
/// `alive_controls` does: a single control is one resolver quirk away from a false pass — e.g. a
/// resolver that only mis-answers under `test.` and not `invalid.` — and `canary_check` runs the
/// full set either way, so there is no cost to including more than one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanaryConfig {
    pub alive_controls: Vec<String>,
    pub dead_controls: Vec<String>,
}

/// The result of one [`canary_check`] run. `Failed` names which control domain misbehaved and
/// what was expected, since a lying resolver invalidates the entire sweep and the failure needs
/// to be diagnosable from the build log alone — the same "name the evidence" convention
/// `gates::GateResult::Fail` follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanaryResult {
    Passed,
    Failed { detail: String },
}

/// Runs every control in `config` — both `alive_controls` and `dead_controls` — against `resolver`
/// and reports whether each answered as expected. Per the plan, **any** canary result other than
/// expected must abort the entire sweep — this function only reports that; discarding the sweep's
/// verdicts and refusing to write the cache back is the caller's (`cli`, module 7, unbuilt)
/// responsibility, since there is no way to know at which query a lying resolver started lying and
/// so no partial sweep to salvage.
pub fn canary_check<R: DnsLookup + ?Sized>(resolver: &R, config: &CanaryConfig) -> CanaryResult {
    for domain in &config.alive_controls {
        let verdict = check(resolver, domain);
        if verdict != Verdict::Alive {
            return CanaryResult::Failed {
                detail: format!(
                    "alive control {domain:?} expected Verdict::Alive, got {verdict:?}"
                ),
            };
        }
    }

    for domain in &config.dead_controls {
        let verdict = check(resolver, domain);
        if verdict != Verdict::Dead {
            return CanaryResult::Failed {
                detail: format!("dead control {domain:?} expected Verdict::Dead, got {verdict:?}"),
            };
        }
    }

    CanaryResult::Passed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A resolver whose answers are configured per `(domain, RecordType)`. Any combination not
    /// explicitly configured answers `Unknown(NoData)` — deliberately not `NxDomain`, so a test
    /// fixture typo (an unconfigured domain) never silently reads as a legitimate negative answer.
    struct FakeResolver {
        answers: HashMap<(String, RecordType), LookupResult>,
    }

    impl FakeResolver {
        fn new() -> Self {
            FakeResolver {
                answers: HashMap::new(),
            }
        }

        fn with(mut self, domain: &str, record: RecordType, result: LookupResult) -> Self {
            self.answers.insert((domain.to_string(), record), result);
            self
        }

        /// Both A and AAAA resolve for `domain`.
        fn with_alive(self, domain: &str) -> Self {
            self.with(domain, RecordType::A, LookupResult::Resolved)
                .with(domain, RecordType::Aaaa, LookupResult::Resolved)
        }

        /// Both A and AAAA come back NXDOMAIN for `domain`.
        fn with_dead(self, domain: &str) -> Self {
            self.with(domain, RecordType::A, LookupResult::NxDomain)
                .with(domain, RecordType::Aaaa, LookupResult::NxDomain)
        }
    }

    impl DnsLookup for FakeResolver {
        fn lookup(&self, domain: &str, record: RecordType) -> LookupResult {
            self.answers
                .get(&(domain.to_string(), record))
                .copied()
                .unwrap_or(LookupResult::Unknown(UnknownReason::NoData))
        }
    }

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

    mod combine_and_check_tests {
        use super::*;

        #[test]
        fn both_resolving_is_alive() {
            let resolver = FakeResolver::new().with_alive("example.com");
            assert_eq!(check(&resolver, "example.com"), Verdict::Alive);
        }

        #[test]
        fn one_resolving_and_the_other_nxdomain_is_alive() {
            // One working address family is enough for the domain to be reachable.
            let resolver = FakeResolver::new()
                .with("example.com", RecordType::A, LookupResult::Resolved)
                .with("example.com", RecordType::Aaaa, LookupResult::NxDomain);
            assert_eq!(check(&resolver, "example.com"), Verdict::Alive);
        }

        #[test]
        fn one_resolving_and_the_other_unknown_is_alive() {
            let resolver = FakeResolver::new()
                .with("example.com", RecordType::A, LookupResult::Resolved)
                .with(
                    "example.com",
                    RecordType::Aaaa,
                    LookupResult::Unknown(UnknownReason::Timeout),
                );
            assert_eq!(check(&resolver, "example.com"), Verdict::Alive);
        }

        #[test]
        fn both_nxdomain_is_dead() {
            let resolver = FakeResolver::new().with_dead("example.invalid");
            assert_eq!(check(&resolver, "example.invalid"), Verdict::Dead);
        }

        #[test]
        fn nxdomain_paired_with_unknown_is_unknown_not_dead() {
            // The named mixed-verdict case from the plan: one family cleanly failing to resolve
            // while the other is merely inconclusive must never be conflated with both families
            // agreeing the domain doesn't exist.
            let resolver = FakeResolver::new()
                .with("example.com", RecordType::A, LookupResult::NxDomain)
                .with(
                    "example.com",
                    RecordType::Aaaa,
                    LookupResult::Unknown(UnknownReason::ServFail),
                );
            assert_eq!(
                check(&resolver, "example.com"),
                Verdict::Unknown(UnknownReason::ServFail)
            );
        }

        #[test]
        fn unknown_paired_with_unknown_is_unknown() {
            let resolver = FakeResolver::new()
                .with(
                    "example.com",
                    RecordType::A,
                    LookupResult::Unknown(UnknownReason::Refused),
                )
                .with(
                    "example.com",
                    RecordType::Aaaa,
                    LookupResult::Unknown(UnknownReason::Timeout),
                );
            assert_eq!(
                check(&resolver, "example.com"),
                Verdict::Unknown(UnknownReason::Refused)
            );
        }

        #[test]
        fn a_domain_with_no_configured_answers_is_unknown_not_dead() {
            // Guards the fixture itself: an unconfigured (domain, record) pair must never read as
            // a legitimate NXDOMAIN.
            let resolver = FakeResolver::new();
            assert_eq!(
                check(&resolver, "unconfigured.example"),
                Verdict::Unknown(UnknownReason::NoData)
            );
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

    mod canary_check_tests {
        use super::*;

        fn config() -> CanaryConfig {
            CanaryConfig {
                alive_controls: vec!["example.com".to_string(), "iana.org".to_string()],
                // RFC 2606 §2 reserves "invalid." as guaranteed never to resolve; RFC 6761 §6.2
                // reserves a "test." name the same way. Two dead controls, not one, so a resolver
                // that only mis-answers one reserved name still fails the canary.
                dead_controls: vec!["invalid.".to_string(), "example.test.".to_string()],
            }
        }

        #[test]
        fn a_healthy_resolver_passes() {
            let resolver = FakeResolver::new()
                .with_alive("example.com")
                .with_alive("iana.org")
                .with_dead("invalid.")
                .with_dead("example.test.");
            assert_eq!(canary_check(&resolver, &config()), CanaryResult::Passed);
        }

        #[test]
        fn a_family_filtering_resolver_fails_an_alive_control() {
            // Simulates a content-filtering resolver that NXDOMAINs a real, non-adult site along
            // with everything else it blocks.
            let resolver = FakeResolver::new()
                .with_dead("example.com")
                .with_alive("iana.org")
                .with_dead("invalid.")
                .with_dead("example.test.");
            match canary_check(&resolver, &config()) {
                CanaryResult::Failed { detail } => {
                    assert!(detail.contains("example.com"));
                    assert!(detail.contains("Alive"));
                }
                CanaryResult::Passed => panic!("expected the canary to fail"),
            }
        }

        #[test]
        fn a_wildcard_sink_resolver_fails_the_dead_control() {
            // Simulates a resolver that NXDOMAIN-rewrites nothing and instead resolves everything
            // (including a name it should refuse) to a sink address.
            let resolver = FakeResolver::new()
                .with_alive("example.com")
                .with_alive("iana.org")
                .with_alive("invalid.")
                .with_alive("example.test.");
            match canary_check(&resolver, &config()) {
                CanaryResult::Failed { detail } => {
                    assert!(detail.contains("invalid."));
                    assert!(detail.contains("Dead"));
                }
                CanaryResult::Passed => panic!("expected the canary to fail"),
            }
        }

        #[test]
        fn a_resolver_that_only_mis_answers_the_second_dead_control_still_fails() {
            // The named reason dead_controls holds more than one entry: a resolver could pass a
            // single dead control by coincidence (or a narrowly scoped rewrite rule) while still
            // resolving other reserved names to a sink.
            let resolver = FakeResolver::new()
                .with_alive("example.com")
                .with_alive("iana.org")
                .with_dead("invalid.")
                .with_alive("example.test.");
            match canary_check(&resolver, &config()) {
                CanaryResult::Failed { detail } => {
                    assert!(detail.contains("example.test."));
                    assert!(detail.contains("Dead"));
                }
                CanaryResult::Passed => panic!("expected the canary to fail"),
            }
        }

        #[test]
        fn an_inconclusive_resolver_still_fails_the_canary() {
            // Unknown is the safe default for a real sweep, but the canary control set is
            // supposed to be unambiguous — an Unknown here means the resolver itself is
            // untrustworthy for this run, not that the sweep should proceed cautiously.
            let resolver = FakeResolver::new()
                .with(
                    "example.com",
                    RecordType::A,
                    LookupResult::Unknown(UnknownReason::Timeout),
                )
                .with(
                    "example.com",
                    RecordType::Aaaa,
                    LookupResult::Unknown(UnknownReason::Timeout),
                )
                .with_alive("iana.org")
                .with_dead("invalid.")
                .with_dead("example.test.");
            assert!(matches!(
                canary_check(&resolver, &config()),
                CanaryResult::Failed { .. }
            ));
        }
    }
}
