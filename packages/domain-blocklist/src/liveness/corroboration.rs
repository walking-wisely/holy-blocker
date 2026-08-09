//! Two-resolver corroboration — [`corroborate`] and [`check_corroborated`] — the actual defence
//! this module now relies on to trust a [`Verdict::Dead`] enough to prune a domain, replacing an
//! earlier design that tried to establish that trust from DNSSEC authentication on a single
//! resolver's answer. See [`LookupResult::NxDomain`](super::lookup::LookupResult::NxDomain)'s doc
//! comment for the full history of why that design was abandoned.
//!
//! # Why corroboration, not DNSSEC authentication
//!
//! The property this module needs is: a single resolver, compromised, hijacked, or configured to
//! lie (a captive portal, a filtering appliance, a malicious upstream), must not be able to
//! silently prune real, live domains from the blocklist by forging NXDOMAIN. DNSSEC authentication
//! was the first attempt at supplying that property and failed for a structural reason unrelated
//! to any bug: NSEC3 opt-out ([RFC 5155 §6](https://www.rfc-editor.org/rfc/rfc5155#section-6)),
//! used by `.com`/`.net`/`.org`/`.xxx` and most large TLDs, makes authenticated denial-of-existence
//! *unavailable* for the overwhelming majority of unsigned delegations regardless of how honest
//! the resolver is — so requiring it did not distinguish an honest resolver from a lying one, it
//! just made `Dead` nearly unreachable for both.
//!
//! Corroboration supplies the same property a different way: require **two independently
//! configured resolvers** — different operators, different anycast networks, ideally different
//! transport paths — to each independently produce [`Verdict::Dead`] for the same domain before a
//! caller trusts it enough to prune. A resolver that lies about one domain's NXDOMAIN is a much
//! smaller attack than a resolver that lies about *every* domain (which the canary already
//! catches, per `canary::CanaryConfig`'s doc comment) — but corroboration means even a single-
//! domain forgery has to land on **both** configured resolvers at once, which is a categorically
//! higher bar than compromising or hijacking one. This does not depend on the swept domain's TLD
//! happening to sign without NSEC3 opt-out, so it does not reintroduce this module's namesake
//! showstopper.
//!
//! This also incidentally covers a class of failure hysteresis alone cannot: a **deterministic**
//! false-NXDOMAIN cause specific to one resolver's own implementation (e.g. an over-aggressive
//! QNAME-minimization implementation misbehaving against an authoritative server that predates
//! [RFC 8020](https://www.rfc-editor.org/rfc/rfc8020)) reproduces identically on every sweep
//! against that one resolver, so a time-bounded quarantine window
//! ([`should_prune_with_hysteresis`](super::cache::should_prune_with_hysteresis)) never resolves
//! it — the domain would still read `Dead` on every retry, no matter how long the wait. A second,
//! differently-implemented and differently-configured resolver has no reason to share the first
//! resolver's specific minimization bug, so `corroborate` naturally fails such a domain back to
//! `Unknown` rather than letting it eventually age past the quarantine window. See
//! [`corroborate`]'s test module for a regression test pinning this scenario down directly.
//!
//! **`should_prune`/`should_prune_with_hysteresis` (in [`cache`](super::cache)) now must be called
//! with the *corroborated* `Verdict` from [`check_corroborated`] (or `corroborate` applied to two
//! independent [`check`](super::lookup::check) calls), never a single resolver's own `check()`
//! result** — see those functions' doc comments, which were updated to say so explicitly.

use super::lookup::{DnsLookup, UnknownReason, Verdict, check};

/// Combines two independent [`Verdict`]s for the same domain — each the result of a full
/// [`check`](super::lookup::check) against a *different*, independently-configured [`DnsLookup`]
/// resolver — into one corroborated verdict. See this module's doc comment for why this, rather
/// than DNSSEC authentication, is what this module actually relies on before trusting a `Dead`
/// verdict enough to prune a domain.
///
/// - Either resolver reporting `Alive` wins outright, regardless of the other — the same "erring
///   toward keeping an entry" direction [`combine`](super::lookup::combine) takes for a single
///   resolver's two address families, applied one layer up across two resolvers. A domain a
///   second resolver can actually reach is alive no matter what the first one claims.
/// - Both resolvers agreeing `Dead` is the only combination this function ever calls `Dead` —
///   the whole point of this function existing.
/// - `Dead` from one resolver paired with `Unknown` from the other is **not** treated as `Dead`
///   by a tie-break toward the more decisive answer — that would silently readmit the exact
///   single-resolver-trust problem this function exists to close, since `Unknown` from the second
///   resolver is not disagreement, it is simply *no corroboration*. The result is
///   `Unknown(UncorroboratedDead)`, a distinct reason from every other `Unknown` cause so a build
///   log can tell "second resolver actively disagreed or was inconclusive" apart from "first
///   resolver alone was already inconclusive."
/// - Both resolvers `Unknown` (with different reasons) keeps the primary (first-argument)
///   resolver's reason — the same arbitrary-but-deterministic tie-break convention
///   [`combine`](super::lookup::combine) uses when both of *its* inputs are `Unknown`.
pub fn corroborate(primary: Verdict, secondary: Verdict) -> Verdict {
    match (primary, secondary) {
        (Verdict::Alive, _) | (_, Verdict::Alive) => Verdict::Alive,
        (Verdict::Dead, Verdict::Dead) => Verdict::Dead,
        (Verdict::Dead, Verdict::Unknown(_)) | (Verdict::Unknown(_), Verdict::Dead) => {
            Verdict::Unknown(UnknownReason::UncorroboratedDead)
        }
        (Verdict::Unknown(reason), Verdict::Unknown(_)) => Verdict::Unknown(reason),
    }
}

/// Runs [`check`](super::lookup::check) against `primary` and `secondary` — two *independently
/// configured* [`DnsLookup`] resolvers, per this module's doc comment (different operators,
/// different networks; querying the same resolver twice, or two resolvers behind the same
/// upstream, defeats the point entirely) — and [`corroborate`]s the two results.
///
/// `pub` so a caller (`cli`, module 7, unbuilt) issuing the two `check()` calls concurrently — the
/// obvious win this function itself doesn't take, since it calls them sequentially — can still
/// reuse [`corroborate`]'s exact combination rule rather than reimplementing it.
pub fn check_corroborated<P: DnsLookup + ?Sized, S: DnsLookup + ?Sized>(
    primary: &P,
    secondary: &S,
    domain: &str,
) -> Verdict {
    corroborate(check(primary, domain), check(secondary, domain))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::liveness::lookup::RecordType;
    use crate::liveness::test_support::FakeResolver;

    #[test]
    fn both_dead_corroborates_to_dead() {
        assert_eq!(corroborate(Verdict::Dead, Verdict::Dead), Verdict::Dead);
    }

    #[test]
    fn either_alive_wins_regardless_of_the_other() {
        assert_eq!(corroborate(Verdict::Alive, Verdict::Dead), Verdict::Alive);
        assert_eq!(corroborate(Verdict::Dead, Verdict::Alive), Verdict::Alive);
        assert_eq!(
            corroborate(Verdict::Alive, Verdict::Unknown(UnknownReason::Timeout)),
            Verdict::Alive
        );
    }

    #[test]
    fn dead_paired_with_unknown_is_uncorroborated_not_dead() {
        // The load-bearing rule: one resolver alone reporting Dead is never enough on its own.
        assert_eq!(
            corroborate(Verdict::Dead, Verdict::Unknown(UnknownReason::Timeout)),
            Verdict::Unknown(UnknownReason::UncorroboratedDead)
        );
        assert_eq!(
            corroborate(Verdict::Unknown(UnknownReason::ServFail), Verdict::Dead),
            Verdict::Unknown(UnknownReason::UncorroboratedDead)
        );
    }

    #[test]
    fn both_unknown_keeps_the_primary_sides_reason() {
        assert_eq!(
            corroborate(
                Verdict::Unknown(UnknownReason::Refused),
                Verdict::Unknown(UnknownReason::Timeout)
            ),
            Verdict::Unknown(UnknownReason::Refused)
        );
    }

    #[test]
    fn check_corroborated_agrees_dead_when_both_resolvers_agree() {
        let primary = FakeResolver::new().with_dead("example.invalid");
        let secondary = FakeResolver::new().with_dead("example.invalid");
        assert_eq!(
            check_corroborated(&primary, &secondary, "example.invalid"),
            Verdict::Dead
        );
    }

    #[test]
    fn check_corroborated_is_uncorroborated_when_only_one_resolver_agrees() {
        let primary = FakeResolver::new().with_dead("example.invalid");
        let secondary = FakeResolver::new().with(
            "example.invalid",
            RecordType::A,
            crate::liveness::lookup::LookupResult::Unknown(UnknownReason::Timeout),
        );
        assert_eq!(
            check_corroborated(&primary, &secondary, "example.invalid"),
            Verdict::Unknown(UnknownReason::UncorroboratedDead)
        );
    }

    /// The scenario named in this module's doc comment: a resolver-specific deterministic bug
    /// (e.g. a QNAME-minimization implementation that mishandles a non-RFC-8020-compliant
    /// authoritative server) reads a genuinely live domain as NXDOMAIN on *every* sweep against
    /// that one resolver, forever — a time-bounded hysteresis window never resolves it, since the
    /// same resolver produces the same wrong answer no matter how long the caller waits. A second,
    /// differently-implemented resolver with no reason to share that bug corroborates the domain
    /// back to Unknown on every single sweep, so it is never pruned no matter how many sweeps run.
    #[test]
    fn a_deterministic_single_resolver_bug_never_corroborates_across_any_number_of_sweeps() {
        let buggy_resolver = FakeResolver::new().with_dead("live-domain.example");
        let honest_resolver = FakeResolver::new().with_alive("live-domain.example");

        for _ in 0..50 {
            assert_eq!(
                check_corroborated(&buggy_resolver, &honest_resolver, "live-domain.example"),
                Verdict::Alive,
                "an honest second resolver reporting Alive must win outright, every sweep"
            );
        }
    }
}
