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
//! - [RFC 6604 §3](https://www.rfc-editor.org/rfc/rfc6604#section-3) — when a query is answered via
//!   a CNAME chain, the RCODE describes the **last** name in the chain, not the queried name; why
//!   [`LookupResult::NxDomainViaCname`] exists and is never treated as proof the queried name itself
//!   is unregistered.

use crate::types::Timestamp;
use std::net::IpAddr;

/// A control-set failure message stops naming individual mismatches past this many, mirroring
/// `gates::MAX_NAMED_HITS` — canary control sets are expected to be tiny (a handful of domains),
/// so this cap will rarely matter in practice, but it keeps the convention consistent across the
/// crate rather than assuming it away here.
const MAX_NAMED_HITS: usize = 20;

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
    /// The queried name resolved via a CNAME chain whose terminal target came back NXDOMAIN (RFC
    /// 6604 §3) — see [`LookupResult::NxDomainViaCname`]. Named so a log line stating this reason
    /// says exactly what evidence was seen, not just that the answer was inconclusive.
    CnameToDeadTarget,
}

/// The outcome of one [`DnsLookup::lookup`] call — one QTYPE, one domain. Never conflated with
/// [`Verdict`], which is what two of these (A and AAAA) combine into.
///
/// Not `Copy` — [`LookupResult::Resolved`] owns a `Vec<IpAddr>`, so this type is `Clone` only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupResult {
    /// An address record, or a CNAME chain that resolved to one — carrying the resolved
    /// addresses themselves, not just the fact of resolution. [`combine`] today only asks
    /// *whether* this variant appears, but a bare unit variant would foreclose the addresses
    /// permanently, and three things this module wants later all need them: sink detection
    /// mid-sweep (a hijacking resolver answering every unrelated domain with the same parking IP
    /// still combines to [`Verdict::Alive`] — the safe direction — but nothing could ever notice
    /// 40,000 domains sharing one address without the address being kept); `0.0.0.0` /
    /// `127.0.0.1` / `::` null-route answers (several upstream sources are hosts-format, and some
    /// operators genuinely null-route a retired domain — those resolve today with no way to say
    /// so); and RFC 8767 serve-stale detection (a suspiciously uniform stale-answer pattern is
    /// only visible in the addresses, never in the outcome alone). Keeping the addresses costs
    /// nothing now and avoids a breaking change later.
    Resolved(Vec<IpAddr>),
    /// An unambiguous negative answer for the **queried name's own** RCODE (RFC 1035 §4.1.1 RCODE
    /// 3) — no CNAME chain was involved. This is the only `LookupResult` [`combine`] will ever read
    /// as proof of non-registration.
    NxDomain,
    /// The queried name resolved via a CNAME chain, but the chain's **terminal target** came back
    /// NXDOMAIN. Per [RFC 6604 §3](https://www.rfc-editor.org/rfc/rfc6604#section-3), the RCODE in
    /// that response describes the last name in the chain, not the queried name itself — so this is
    /// evidence the *target* doesn't currently resolve, not that the queried name is unregistered.
    /// The queried name can still be a live registration that gets repointed tomorrow, so this must
    /// never combine into [`Verdict::Dead`] the way a direct [`LookupResult::NxDomain`] can.
    NxDomainViaCname,
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
///    reachable, regardless of what the other side says.
/// 2. Otherwise, both sides unambiguously `NxDomain` **on the queried name's own RCODE** is `Dead`
///    — the only case that proves non-registration.
/// 3. Otherwise, if either side is [`LookupResult::NxDomainViaCname`], the result is
///    `Unknown(CnameToDeadTarget)`, never `Dead`. Per [RFC 6604
///    §3](https://www.rfc-editor.org/rfc/rfc6604#section-3), an NXDOMAIN reached through a CNAME
///    chain describes the chain's terminal target, not the queried name — so a queried name that
///    resolved via a dead-ended CNAME chain (whether paired with a plain `NxDomain`, another
///    `NxDomainViaCname`, or an `Unknown`) is not proven unregistered and must be kept, per this
///    module's "false negatives are the budget, false positives are the price" rule.
/// 4. Otherwise, at least one side is `Unknown` and neither is `Resolved` — the result is
///    `Unknown`. This is the mixed `NxDomain`/`Unknown` case: one family failing to resolve while
///    the other is merely inconclusive must never be conflated with both families cleanly saying
///    the domain doesn't exist.
///
/// When both sides are `Unknown` with different reasons, the A side's reason is kept — an
/// arbitrary but deterministic tie-break; the plan does not specify one, and only the `Verdict`
/// variant (never the specific reason) affects pruning.
///
/// `pub` so a caller issuing A and AAAA lookups concurrently (the obvious 2x win `check` doesn't
/// take, since it issues them sequentially) can reuse this exact three-step evaluation order
/// rather than forking the one piece of genuinely subtle policy in this module.
pub fn combine(a: LookupResult, aaaa: LookupResult) -> Verdict {
    match (a, aaaa) {
        (LookupResult::Resolved(_), _) | (_, LookupResult::Resolved(_)) => Verdict::Alive,
        (LookupResult::NxDomain, LookupResult::NxDomain) => Verdict::Dead,
        (LookupResult::NxDomainViaCname, _) | (_, LookupResult::NxDomainViaCname) => {
            Verdict::Unknown(UnknownReason::CnameToDeadTarget)
        }
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
/// Relies on the caller's [`DnsLookup`] impl to distinguish a direct NXDOMAIN on `domain` itself
/// from one reached via a dead-ended CNAME chain ([`LookupResult::NxDomainViaCname`], RFC 6604
/// §3) — only the former can ever combine into [`Verdict::Dead`].
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
/// never resolve (caught by `dead_controls`). `dead_controls` takes more than one for the same
/// reason `alive_controls` does: a single control is one resolver quirk away from a false pass —
/// e.g. a resolver that only mis-answers under `test.` and not `invalid.` — and `canary_check`
/// runs the full set either way, so there is no cost to including more than one.
///
/// A real deployment's `dead_controls` should mix **two distinct failure classes**, not just take
/// more reserved names: at least one RFC 2606/6761 reserved name (e.g. `invalid.`), which catches
/// a resolver that sinks *everything* indiscriminately, and at least one nonce control built via
/// [`nonce_dead_control`], which catches a resolver that only NXDOMAIN-rewrites *registered* space
/// while special-casing the reserved TLDs (see that function's doc comment). This module has no
/// way to distinguish a nonce string from a hand-typed one, so it cannot enforce that mix — only
/// the non-empty cardinality [`CanaryConfig::new`] checks — building `dead_controls` correctly is
/// a `cli`-level (module 7, unbuilt) construction responsibility.
///
/// **`new()` is the only way to get the enforced non-empty guarantee.** Fields are kept `pub`
/// (matching this file's existing style, e.g. [`CacheEntry`]'s public fields) because tests and
/// other trusted call sites that already know a literal is valid need to construct one directly —
/// but a struct literal built by hand bypasses the invariant entirely; treat that as a deliberate
/// opt-out, not an oversight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanaryConfig {
    pub alive_controls: Vec<String>,
    pub dead_controls: Vec<String>,
}

/// Why [`CanaryConfig::new`] refused to build a config. Names which list was empty, per this
/// module's "name the evidence" convention (see [`CanaryResult`]) rather than a bare unit error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanaryConfigError {
    /// `alive_controls` was empty — see the "silently passes half-open" note on [`canary_check`].
    EmptyAliveControls,
    /// `dead_controls` was empty — a canary with no dead control can never catch a sink resolver.
    EmptyDeadControls,
}

impl CanaryConfig {
    /// Builds a [`CanaryConfig`], rejecting either list being empty. An empty `alive_controls`
    /// would make [`canary_check`] silently skip every alive check and pass half-open; an empty
    /// `dead_controls` would make it silently skip every dead check the same way — both directions
    /// are refused here so a caller can't construct a canary that is quietly missing one half of
    /// its job.
    pub fn new(
        alive_controls: Vec<String>,
        dead_controls: Vec<String>,
    ) -> Result<Self, CanaryConfigError> {
        if alive_controls.is_empty() {
            return Err(CanaryConfigError::EmptyAliveControls);
        }
        if dead_controls.is_empty() {
            return Err(CanaryConfigError::EmptyDeadControls);
        }
        Ok(CanaryConfig {
            alive_controls,
            dead_controls,
        })
    }
}

/// Builds a nonce dead-control domain — a random label under a real, currently-registered base
/// domain, e.g. `hb-canary-<nonce>.example.com`. This function is **pure and deterministic**: it
/// does not generate the nonce itself. `cli` (module 7, unbuilt) is responsible for generating a
/// fresh random nonce per sweep run and calling this to build the actual dead-control domain
/// string before constructing a [`CanaryConfig`].
///
/// # Why this control exists
///
/// RFC 6761 §6.4 (<https://www.rfc-editor.org/rfc/rfc6761#section-6.4>) instructs caching
/// resolvers to answer reserved TLDs like `.invalid` negatively **from a local hardcoded rule,
/// without ever querying upstream** — and systemd-resolved, dnsmasq and unbound all do. A
/// hijacking resolver that sinks every *real* name while special-casing the reserved TLDs (the
/// common configuration, since NXDOMAIN-monetization appliances only care about registered space)
/// passes a canary built only from RFC 2606/6761 reserved names cleanly — the control would be
/// drawn from precisely the namespace the threat is required to treat correctly.
///
/// A nonce label under a real, currently-registered base domain closes that gap: an unregistered
/// `<nonce>.base_domain` genuinely NXDOMAINs on an honest resolver, so a resolver that rewrites it
/// to a sink address is caught regardless of how it treats reserved TLDs. This is the same
/// technique Chrome has long used for captive-portal/hijack detection, and there is no substitute
/// for it. **It must be random per run** — a fixed, checked-in nonce could simply be allowlisted
/// by a sink operator, defeating the control.
///
/// `base_domain` must be a real, currently-registered domain the caller controls or trusts to
/// always answer honestly for random subdomains (i.e., a domain where an unregistered
/// `<nonce>.base_domain` genuinely NXDOMAINs). This module has no way to verify that property —
/// it's a `cli`-level configuration responsibility, same as the non-empty invariant above only
/// enforces cardinality, never content.
pub fn nonce_dead_control(base_domain: &str, nonce: &str) -> String {
    format!("hb-canary-{nonce}.{base_domain}")
}

/// The result of one [`canary_check`] run. `Failed` collects **every** control that mismatched,
/// not just the first — since a lying resolver invalidates the entire sweep and the failure needs
/// to be diagnosable from the build log alone, and stopping at the first mismatch can lose the
/// diagnosis: "resolver entirely down" (every control `Unknown`) and "content filter" (one alive
/// control `Dead`, dead control also `Dead`) would otherwise produce indistinguishable single-line
/// details, because a short-circuiting check never reaches the second failure. Each entry follows
/// the same "name the evidence" convention `gates::GateResult::Fail` uses, one
/// `"{kind} control {domain:?} expected Verdict::X, got {verdict:?}"` string per mismatched
/// control, capped at [`MAX_NAMED_HITS`] the same way `gates.rs` caps its own hit lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanaryResult {
    Passed,
    Failed { failures: Vec<String> },
}

/// Runs every control in `config` — both `alive_controls` and `dead_controls` — against `resolver`
/// and reports whether each answered as expected. Per the plan, **any** canary result other than
/// expected must abort the entire sweep — this function only reports that; discarding the sweep's
/// verdicts and refusing to write the cache back is the caller's (`cli`, module 7, unbuilt)
/// responsibility, since there is no way to know at which query a lying resolver started lying and
/// so no partial sweep to salvage.
///
/// Checks the **whole** control set rather than stopping at the first mismatch, and reports every
/// failing control together in [`CanaryResult::Failed`] — see that type's doc comment for why a
/// short-circuiting check can lose the diagnosis. Capped at [`MAX_NAMED_HITS`] named failures.
///
/// A [`CanaryConfig`] built through [`CanaryConfig::new`] can never have an empty `alive_controls`
/// or `dead_controls`, closing the "empty list silently passes half-open" gap that invariant
/// exists to prevent — but this function does not assume every caller went through `new()` (a
/// struct literal still compiles) and so simply checks whatever it's given: an empty list here
/// contributes zero failures rather than a distinct error, exactly as an empty `for` loop does.
pub fn canary_check<R: DnsLookup + ?Sized>(resolver: &R, config: &CanaryConfig) -> CanaryResult {
    let mut failures = Vec::new();

    for domain in &config.alive_controls {
        let verdict = check(resolver, domain);
        if verdict != Verdict::Alive {
            failures.push(format!(
                "alive control {domain:?} expected Verdict::Alive, got {verdict:?}"
            ));
        }
    }

    for domain in &config.dead_controls {
        let verdict = check(resolver, domain);
        if verdict != Verdict::Dead {
            failures.push(format!(
                "dead control {domain:?} expected Verdict::Dead, got {verdict:?}"
            ));
        }
    }

    if failures.is_empty() {
        return CanaryResult::Passed;
    }

    if failures.len() > MAX_NAMED_HITS {
        let more = failures.len() - MAX_NAMED_HITS;
        failures.truncate(MAX_NAMED_HITS);
        failures.push(format!("...and {more} more"));
    }

    CanaryResult::Failed { failures }
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

        /// Both A and AAAA resolve for `domain`, to a fixed RFC 5737 §3 TEST-NET-1 documentation
        /// address (`192.0.2.1`) — the specific value never matters to any test built on this
        /// helper, since `combine` only asks whether a lookup resolved, not what it resolved to.
        fn with_alive(self, domain: &str) -> Self {
            let addr = IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, 1));
            self.with(domain, RecordType::A, LookupResult::Resolved(vec![addr]))
                .with(domain, RecordType::Aaaa, LookupResult::Resolved(vec![addr]))
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
                .cloned()
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
                .with(
                    "example.com",
                    RecordType::A,
                    LookupResult::Resolved(vec![IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, 1))]),
                )
                .with("example.com", RecordType::Aaaa, LookupResult::NxDomain);
            assert_eq!(check(&resolver, "example.com"), Verdict::Alive);
        }

        #[test]
        fn one_resolving_and_the_other_unknown_is_alive() {
            let resolver = FakeResolver::new()
                .with(
                    "example.com",
                    RecordType::A,
                    LookupResult::Resolved(vec![IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, 1))]),
                )
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
        fn both_families_nxdomain_via_cname_is_unknown_not_dead() {
            // RFC 6604 §3: the RCODE describes the CNAME chain's terminal target, not the queried
            // name, so a name that resolved via a chain whose target is now NXDOMAIN in both
            // families is not proven unregistered — it must not be pruned as Dead.
            let resolver = FakeResolver::new()
                .with("example.com", RecordType::A, LookupResult::NxDomainViaCname)
                .with(
                    "example.com",
                    RecordType::Aaaa,
                    LookupResult::NxDomainViaCname,
                );
            assert_eq!(
                check(&resolver, "example.com"),
                Verdict::Unknown(UnknownReason::CnameToDeadTarget)
            );
        }

        #[test]
        fn nxdomain_via_cname_paired_with_plain_nxdomain_is_unknown_not_dead() {
            // A direct NXDOMAIN on one family plus a CNAME-chain NXDOMAIN on the other is still
            // not two direct NXDOMAINs on the queried name — only that combination is Dead.
            let resolver = FakeResolver::new()
                .with("example.com", RecordType::A, LookupResult::NxDomainViaCname)
                .with("example.com", RecordType::Aaaa, LookupResult::NxDomain);
            assert_eq!(
                check(&resolver, "example.com"),
                Verdict::Unknown(UnknownReason::CnameToDeadTarget)
            );
        }

        #[test]
        fn nxdomain_via_cname_paired_with_resolved_is_alive() {
            // An alive family wins regardless of what the other side reports.
            let resolver = FakeResolver::new()
                .with("example.com", RecordType::A, LookupResult::NxDomainViaCname)
                .with(
                    "example.com",
                    RecordType::Aaaa,
                    LookupResult::Resolved(vec![IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, 1))]),
                );
            assert_eq!(check(&resolver, "example.com"), Verdict::Alive);
        }

        #[test]
        fn resolved_addresses_are_not_discarded() {
            // The whole point of LookupResult::Resolved carrying a Vec<IpAddr> rather than being
            // a unit variant: the addresses a lookup actually returned must still be readable by
            // a caller, even though combine()/check() only care whether the variant is Resolved
            // at all right now.
            let addr = IpAddr::V4(std::net::Ipv4Addr::new(198, 51, 100, 7));
            let resolver = FakeResolver::new().with(
                "example.com",
                RecordType::A,
                LookupResult::Resolved(vec![addr]),
            );
            match resolver.lookup("example.com", RecordType::A) {
                LookupResult::Resolved(addrs) => assert_eq!(addrs, vec![addr]),
                other => panic!("expected Resolved, got {other:?}"),
            }
        }

        #[test]
        fn resolved_with_no_addresses_still_combines_to_alive() {
            // combine() only asks whether a side is Resolved at all, never inspects the address
            // list itself — an empty Vec (a lookup that reports "resolved" but couldn't attach
            // any address, e.g. a synthetic or partially-decoded answer) must still count.
            assert_eq!(
                combine(
                    LookupResult::Resolved(vec![]),
                    LookupResult::Unknown(UnknownReason::Timeout)
                ),
                Verdict::Alive
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

    mod canary_config_tests {
        use super::*;

        #[test]
        fn empty_alive_controls_is_rejected() {
            let result = CanaryConfig::new(vec![], vec!["invalid.".to_string()]);
            assert_eq!(result, Err(CanaryConfigError::EmptyAliveControls));
        }

        #[test]
        fn empty_dead_controls_is_rejected() {
            let result = CanaryConfig::new(vec!["example.com".to_string()], vec![]);
            assert_eq!(result, Err(CanaryConfigError::EmptyDeadControls));
        }

        #[test]
        fn both_non_empty_succeeds() {
            let result = CanaryConfig::new(
                vec!["example.com".to_string()],
                vec!["invalid.".to_string()],
            );
            assert_eq!(
                result,
                Ok(CanaryConfig {
                    alive_controls: vec!["example.com".to_string()],
                    dead_controls: vec!["invalid.".to_string()],
                })
            );
        }
    }

    mod nonce_dead_control_tests {
        use super::*;

        #[test]
        fn builds_the_expected_label_format() {
            assert_eq!(
                nonce_dead_control("example.com", "abc123"),
                "hb-canary-abc123.example.com"
            );
        }

        #[test]
        fn different_nonces_produce_different_domains() {
            let first = nonce_dead_control("example.com", "aaaa");
            let second = nonce_dead_control("example.com", "bbbb");
            assert_ne!(first, second);
        }

        #[test]
        fn different_base_domains_produce_different_domains() {
            let first = nonce_dead_control("example.com", "abc123");
            let second = nonce_dead_control("example.org", "abc123");
            assert_ne!(first, second);
        }
    }

    mod canary_check_tests {
        use super::*;

        fn config() -> CanaryConfig {
            CanaryConfig::new(
                vec!["example.com".to_string(), "iana.org".to_string()],
                // RFC 2606 §2 reserves "invalid." as guaranteed never to resolve; RFC 6761 §6.2
                // reserves a "test." name the same way. Two dead controls, not one, so a resolver
                // that only mis-answers one reserved name still fails the canary.
                vec!["invalid.".to_string(), "example.test.".to_string()],
            )
            .expect("two non-empty control lists must build a valid CanaryConfig")
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
                CanaryResult::Failed { failures } => {
                    assert_eq!(failures.len(), 1);
                    assert!(failures[0].contains("example.com"));
                    assert!(failures[0].contains("Alive"));
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
                CanaryResult::Failed { failures } => {
                    assert_eq!(failures.len(), 2);
                    assert!(failures.iter().any(|f| f.contains("invalid.")));
                    assert!(failures.iter().any(|f| f.contains("example.test.")));
                    assert!(failures.iter().all(|f| f.contains("Dead")));
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
                CanaryResult::Failed { failures } => {
                    assert_eq!(failures.len(), 1);
                    assert!(failures[0].contains("example.test."));
                    assert!(failures[0].contains("Dead"));
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

        #[test]
        fn an_alive_control_failure_and_a_dead_control_failure_are_both_reported() {
            // The exact case the reviewer named: short-circuiting on the first failure would lose
            // the diagnosis between "resolver entirely down" and "content filter" — both must be
            // named when both are actually failing, not just the first one encountered.
            let resolver = FakeResolver::new()
                .with_dead("example.com")
                .with_alive("iana.org")
                .with_alive("invalid.")
                .with_dead("example.test.");
            match canary_check(&resolver, &config()) {
                CanaryResult::Failed { failures } => {
                    assert_eq!(failures.len(), 2);
                    assert!(
                        failures
                            .iter()
                            .any(|f| f.contains("example.com") && f.contains("Alive"))
                    );
                    assert!(
                        failures
                            .iter()
                            .any(|f| f.contains("invalid.") && f.contains("Dead"))
                    );
                }
                CanaryResult::Passed => panic!("expected the canary to fail"),
            }
        }
    }
}
