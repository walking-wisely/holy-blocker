//! The lookup/verdict model: one [`DnsLookup::lookup`] call's outcome ([`LookupResult`]), how two
//! of them (A and AAAA) [`combine`] into a per-domain [`Verdict`], and [`check`] wiring both
//! together. See the parent module's doc comment for the reference documents this file's citations
//! draw from.

use std::net::IpAddr;

/// RFC 8914 §4 Extended DNS Error `INFO-CODE`s that mark a negative answer as policy-driven
/// rather than a fact about the registry — see [`LookupResult::NxDomain::extended_error`] and
/// [`is_filtering_ede`].
const EDE_FORGED_ANSWER: u16 = 4;
const EDE_DNSSEC_BOGUS: u16 = 6;
const EDE_BLOCKED: u16 = 15;
const EDE_CENSORED: u16 = 16;
const EDE_FILTERED: u16 = 17;

/// Whether an RFC 8914 Extended DNS Error code (if any) signals that a negative answer is
/// policy-driven — a resolver-side decision, not a fact about whether the name is registered.
/// [`combine`] treats any of these on either side of a paired `NxDomain`/`NxDomain` as proof the
/// resolver itself is not to be trusted for this domain, per that RFC's §4 definitions of Forged
/// Answer, DNSSEC Bogus, Blocked, Censored and Filtered.
fn is_filtering_ede(extended_error: Option<u16>) -> bool {
    matches!(
        extended_error,
        Some(EDE_FORGED_ANSWER)
            | Some(EDE_DNSSEC_BOGUS)
            | Some(EDE_BLOCKED)
            | Some(EDE_CENSORED)
            | Some(EDE_FILTERED)
    )
}

/// Why a single-QTYPE lookup could not be read as a definite answer. Never used to prune a
/// domain on its own — see [`Verdict::Unknown`] and the plan's "a resolver hiccup or a transient
/// SERVFAIL must never be able to silently shrink the blocklist" rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownReason {
    /// A response with no error but no matching record (RCODE 0, empty answer) — the NODATA case,
    /// whose semantics come from RFC 2308 §2.2, not RFC 1035 §4.1.1 (which defines only the RCODE
    /// field, not what an empty-answer/RCODE-0 response means).
    ///
    /// **For an apex-scoped entry, this is an expected, benign, and often *permanent* state, not
    /// a sign of trouble.** A zone that only publishes addresses at `www` (with MX/TXT/etc. at the
    /// apex) is an empty non-terminal or NODATA at the apex per [RFC 4592
    /// §2.2.2](https://www.rfc-editor.org/rfc/rfc4592#section-2.2.2) — that domain is alive and
    /// serving content, and will report `NoData` on the apex A/AAAA lookup on every sweep forever.
    /// This module has no visibility into apex-vs-exact-host scope (that lives in
    /// `domain-normalize`/`merge.rs`, a different crate), so it cannot special-case this itself —
    /// but a caller tracking "domains stuck in `Unknown` across repeated sweeps" (per the plan)
    /// should not treat a persistent `NoData` the same as a persistent `Timeout` or `ServFail`:
    /// the latter two indicate resolver or network trouble worth investigating, while `NoData` at
    /// an apex is exactly what a healthy `www`-only zone looks like and needs no attention. See
    /// [`combine`]'s doc comment for the two unexploited levers (a `www.` secondary probe, and RFC
    /// 8020 zero-query NXDOMAIN inference) that would resolve this class of entry rather than just
    /// characterize it, both left as follow-up work outside this module's scope.
    NoData,
    /// RFC 1035 §4.1.1 RCODE 2.
    ServFail,
    /// RFC 1035 §4.1.1 RCODE 5 — the resolver declined to answer at all.
    Refused,
    /// RFC 1035 §4.1.1 RCODE 1 — the query itself couldn't be interpreted (a malformed request, or
    /// a middlebox mangling it in flight). Distinct from [`UnknownReason::Malformed`], which is
    /// about an unparseable *response*: this is the resolver's own claim that the *request* made no
    /// sense, and conflating the two would misreport which end of the exchange the evidence points
    /// at.
    FormErr,
    /// RFC 1035 §4.1.1 RCODE 4 — the resolver understood the query but does not support the
    /// requested kind of lookup. Some middleboxes and older resolvers answer AAAA this way instead
    /// of a clean NODATA or NXDOMAIN; kept distinct from [`UnknownReason::Refused`] (RCODE 5, a
    /// deliberate policy refusal) since a log line naming the RCODE should say which one fired.
    NotImp,
    /// No response arrived within the resolver client's deadline.
    Timeout,
    /// A response arrived but could not be parsed as a valid DNS message. **Never the right home
    /// for an unretried TC=1 (truncated) response** — see [`DnsLookup::lookup`]'s doc comment: an
    /// implementation that doesn't retry over TCP per RFC 1035 §4.2.1 / RFC 7766 must not surface
    /// truncation this way, since it permanently strands the entry in `Verdict::Unknown`.
    Malformed,
    /// The queried name resolved via a CNAME (RFC 1035 §3.3.1) or DNAME (RFC 6672) chain whose
    /// terminal target came back NXDOMAIN — see [`LookupResult::NxDomainViaChain`]. Named so a log
    /// line stating this reason says exactly what evidence was seen, not just that the answer was
    /// inconclusive. DNAME redirection carries the identical "the RCODE describes the target, not
    /// the queried name" property CNAME does (RFC 6604 §3), plus its own RFC 6672 §2.2 negative
    /// answer, YXDOMAIN (RCODE 6), returned when expanding the substitution would exceed 255
    /// octets — that case is folded into this same variant rather than given its own, since the
    /// evidentiary status ("not proof the queried name is dead") is identical either way.
    ChainToDeadTarget,
    /// Both sides answered `NxDomain`, but at least one carried an RFC 8914 Extended DNS Error
    /// code signalling the negative answer is policy-driven rather than a fact about the registry
    /// (4 Forged Answer, 6 DNSSEC Bogus, 15 Blocked, 16 Censored, 17 Filtered). One filtered
    /// answer means the resolver filters, so this must never combine into [`Verdict::Dead`] — and
    /// per the plan, the caller should treat it as a reason to abort the sweep entirely, not just
    /// skip this one domain.
    FilteredByResolver,
    /// Both sides answered `NxDomain` with no EDE-signalled filtering, but at least one was not
    /// DNSSEC-authenticated (the AD bit, RFC 4035 §3.2.3, was not set). An unauthenticated
    /// NXDOMAIN is exactly what a hijacking resolver forges to evade the canary — see
    /// [`LookupResult::NxDomain`] — so it is not proof of non-registration on its own.
    UnauthenticatedNxDomain,
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
    /// 3) — no CNAME chain was involved. This is the only `LookupResult` [`combine`] can ever read
    /// as proof of non-registration, and even then only conditionally — a bare NXDOMAIN carries no
    /// evidence of *why* it should be trusted, and two real mechanisms exist to supply that
    /// evidence (RFC 8914 §4):
    ///
    /// - `authenticated` is the resolver's DNSSEC AD bit (RFC 4035 §3.2.3) on this answer. `.com`,
    ///   `.net`, `.org` and most ccTLDs are signed, so authenticated denial-of-existence via
    ///   NSEC/NSEC3 is available for the overwhelming majority of entries even though the domains
    ///   *themselves* are unsigned — a validating resolver setting `AD=1` on an NXDOMAIN is
    ///   cryptographic proof the name is not delegated. [`combine`] requires `authenticated: true`
    ///   on **both** sides before it will ever produce [`Verdict::Dead`]; otherwise the result is
    ///   [`UnknownReason::UnauthenticatedNxDomain`]. This makes the canary-evasion attack of a
    ///   resolver simply forging NXDOMAIN for gTLD names essentially impossible.
    /// - `extended_error` is the raw RFC 8914 EDE `INFO-CODE`, if the answer carried one. Codes 4
    ///   (Forged Answer), 6 (DNSSEC Bogus), 15 (Blocked), 16 (Censored) and 17 (Filtered) are
    ///   exactly the "this negative answer is policy, not fact" signal this module needs; public
    ///   resolvers including Cloudflare emit them. [`combine`] treats any of those codes on either
    ///   side as [`UnknownReason::FilteredByResolver`], never [`Verdict::Dead`].
    NxDomain {
        authenticated: bool,
        extended_error: Option<u16>,
    },
    /// The queried name resolved via a CNAME ([RFC 1035
    /// §3.3.1](https://www.rfc-editor.org/rfc/rfc1035#section-3.3.1)) or DNAME ([RFC
    /// 6672](https://www.rfc-editor.org/rfc/rfc6672)) chain, but the chain's **terminal target**
    /// came back NXDOMAIN (or, for a DNAME substitution overflowing 255 octets, RFC 6672 §2.2's own
    /// YXDOMAIN). Per [RFC 6604 §3](https://www.rfc-editor.org/rfc/rfc6604#section-3) — a property
    /// DNAME redirection shares identically, since a DNAME is itself resolved via a synthesized
    /// CNAME — the RCODE in that response describes the last name in the chain, not the queried
    /// name itself, so this is evidence the *target* doesn't currently resolve, not that the
    /// queried name is unregistered. The queried name can still be a live registration that gets
    /// repointed tomorrow, so this must never combine into [`Verdict::Dead`] the way a direct
    /// [`LookupResult::NxDomain`] can.
    NxDomainViaChain,
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
/// 2. Otherwise, if both sides are `NxDomain` **on the queried name's own RCODE**:
///    - if either side's [`LookupResult::NxDomain::extended_error`] is an RFC 8914 code that
///      signals policy-driven filtering (see [`is_filtering_ede`]), the result is
///      `Unknown(FilteredByResolver)` — one filtered answer means the resolver filters, never
///      that the domain is dead;
///    - otherwise, `Dead` requires **both** sides to be DNSSEC-`authenticated`; if either is not,
///      the result is `Unknown(UnauthenticatedNxDomain)`, since an unauthenticated NXDOMAIN is
///      exactly what a hijacking resolver forges;
///    - only when both conditions above are satisfied — no filtering EDE, both authenticated —
///      is the result `Dead`, the only case that proves non-registration.
/// 3. Otherwise, if either side is [`LookupResult::NxDomainViaChain`], the result is
///    `Unknown(ChainToDeadTarget)`, never `Dead`. Per [RFC 6604
///    §3](https://www.rfc-editor.org/rfc/rfc6604#section-3) (and identically for DNAME redirection,
///    [RFC 6672](https://www.rfc-editor.org/rfc/rfc6672)), an NXDOMAIN reached through a CNAME or
///    DNAME chain describes the chain's terminal target, not the queried name — so a queried name
///    that resolved via a dead-ended CNAME or DNAME chain (whether paired with a plain `NxDomain`,
///    another `NxDomainViaChain`, or an `Unknown`) is not proven unregistered and must be kept, per
///    this module's "false negatives are the budget, false positives are the price" rule.
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
///
/// # A known-large, mostly-benign `Unknown` cohort
///
/// Case 4 above folds plain apex NODATA (see [`UnknownReason::NoData`]'s doc comment — a `www`-only
/// zone reports this on every sweep, forever, per [RFC 4592
/// §2.2.2](https://www.rfc-editor.org/rfc/rfc4592#section-2.2.2)) into the same `Unknown` bucket as
/// every other inconclusive cause. This module has no visibility into apex-vs-exact-host scope to
/// do anything about that here — that lives in `domain-normalize`/`merge.rs`, a different crate —
/// so two real levers exist but are deliberately **not implemented** in this function:
///
/// - TODO: an apex-scoped entry that comes back NODATA could be resolved (not just characterized)
///   by a secondary `www.<domain>` probe — a live `www` host would prove the apex domain is alive
///   even though the apex itself publishes no address records. This would change `check`'s
///   signature/behavior (it would need scope information this module doesn't have) and needs a
///   design decision, not a guess, before it's built.
/// - TODO: [RFC 8020](https://www.rfc-editor.org/rfc/rfc8020) ("NXDOMAIN really means there is
///   nothing underneath") is an unexploited lever in the other direction — an authenticated
///   NXDOMAIN at an apex proves every subdomain of it is also dead, with zero additional queries.
///   Not implemented here either; noted as a related possibility, not a plan.
pub fn combine(a: LookupResult, aaaa: LookupResult) -> Verdict {
    match (a, aaaa) {
        (LookupResult::Resolved(_), _) | (_, LookupResult::Resolved(_)) => Verdict::Alive,
        (
            LookupResult::NxDomain {
                authenticated: a_authenticated,
                extended_error: a_ede,
            },
            LookupResult::NxDomain {
                authenticated: aaaa_authenticated,
                extended_error: aaaa_ede,
            },
        ) => {
            if is_filtering_ede(a_ede) || is_filtering_ede(aaaa_ede) {
                Verdict::Unknown(UnknownReason::FilteredByResolver)
            } else if a_authenticated && aaaa_authenticated {
                Verdict::Dead
            } else {
                Verdict::Unknown(UnknownReason::UnauthenticatedNxDomain)
            }
        }
        (LookupResult::NxDomainViaChain, _) | (_, LookupResult::NxDomainViaChain) => {
            Verdict::Unknown(UnknownReason::ChainToDeadTarget)
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

/// One DNS lookup, behind a trait so [`check`] and [`canary_check`](super::canary_check) need no
/// live network access to test. The pipeline binary (module 7, unbuilt) wires in the real client
/// — an explicitly configured resolver (defaulting to `1.1.1.1` / `2606:4700:4700::1111`, never
/// the filtering `1.1.1.2`/`1.1.1.3` variants, and never the host's system resolver), per the
/// plan.
///
/// # Implementation contract
///
/// This trait's return type cannot enforce the following on its own — they are properties of how
/// `lookup` talks to the wire, not of the [`LookupResult`] it hands back — so a real implementation
/// (module 7, unbuilt) MUST satisfy them, or the invariants the rest of this module is built on
/// silently stop holding:
///
/// - **Truncation MUST be retried over TCP before returning.** A `TC=1` response (RFC 1035
///   §4.2.1) means the UDP answer was cut short, and [RFC 7766](https://www.rfc-editor.org/rfc/rfc7766)
///   requires a compliant client to redo the query over TCP rather than trust the truncated
///   payload. An implementation that returns on the truncated UDP response anyway must not surface
///   that as [`UnknownReason::Malformed`] — a `Malformed` verdict is exactly as untrustworthy as any
///   other `Unknown` and so never prunes on its own, but an entry that is *always* truncated for
///   this resolver would then be permanently stuck in `Verdict::Unknown` on every sweep, forever,
///   with nothing to say why. Retry over TCP first; only report `Malformed` for a response that
///   still can't be parsed as a valid DNS message after that retry.
/// - **CNAME/DNAME chain following MUST detect loops and bound depth.** Nothing in [`LookupResult`]
///   stops an implementation from following an indirection chain (CNAME, RFC 1035 §3.3.1; DNAME,
///   RFC 6672) forever if two names point at each other, or from following an attacker-lengthened
///   chain until it overflows some internal buffer. An implementation MUST detect a chain loop and
///   MUST bound how many indirections it will follow, returning [`UnknownReason::Malformed`] (or,
///   when the terminal answer is itself evidence — e.g. the chain's last hop came back NXDOMAIN —
///   [`LookupResult::NxDomainViaChain`]) rather than looping forever or overflowing.
pub trait DnsLookup {
    fn lookup(&self, domain: &str, record: RecordType) -> LookupResult;
}

/// Looks up `domain`'s liveness: one A and one AAAA query, combined per [`combine`]'s ordering.
/// Relies on the caller's [`DnsLookup`] impl to distinguish a direct NXDOMAIN on `domain` itself
/// from one reached via a dead-ended CNAME or DNAME chain ([`LookupResult::NxDomainViaChain`], RFC
/// 6604 §3 / RFC 6672) — only the former can ever combine into [`Verdict::Dead`]. See
/// [`DnsLookup`]'s doc comment for the truncation-retry and chain-loop/depth-limit contract a real
/// implementation must satisfy.
pub fn check<R: DnsLookup + ?Sized>(resolver: &R, domain: &str) -> Verdict {
    let a = resolver.lookup(domain, RecordType::A);
    let aaaa = resolver.lookup(domain, RecordType::Aaaa);
    combine(a, aaaa)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::liveness::test_support::FakeResolver;

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
            .with(
                "example.com",
                RecordType::Aaaa,
                LookupResult::NxDomain {
                    authenticated: true,
                    extended_error: None,
                },
            );
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
    fn both_sides_authenticated_nxdomain_is_dead() {
        // Named separately from both_nxdomain_is_dead to pin the exact rule down explicitly:
        // Dead requires DNSSEC authentication on BOTH sides, not just "both NxDomain".
        let resolver = FakeResolver::new().with_dead_as("example.invalid", true, None);
        assert_eq!(check(&resolver, "example.invalid"), Verdict::Dead);
    }

    #[test]
    fn both_sides_unauthenticated_nxdomain_is_unknown_not_dead() {
        // An unauthenticated NXDOMAIN on both families is exactly what a hijacking resolver
        // forges to evade the canary — it must never combine into Dead on its own.
        let resolver = FakeResolver::new().with_dead_as("example.invalid", false, None);
        assert_eq!(
            check(&resolver, "example.invalid"),
            Verdict::Unknown(UnknownReason::UnauthenticatedNxDomain)
        );
    }

    #[test]
    fn one_authenticated_and_one_unauthenticated_nxdomain_is_unknown_not_dead() {
        // Dead requires BOTH sides authenticated; one side being honest does not vouch for
        // the other.
        let resolver = FakeResolver::new()
            .with(
                "example.invalid",
                RecordType::A,
                LookupResult::NxDomain {
                    authenticated: true,
                    extended_error: None,
                },
            )
            .with(
                "example.invalid",
                RecordType::Aaaa,
                LookupResult::NxDomain {
                    authenticated: false,
                    extended_error: None,
                },
            );
        assert_eq!(
            check(&resolver, "example.invalid"),
            Verdict::Unknown(UnknownReason::UnauthenticatedNxDomain)
        );
    }

    #[test]
    fn nxdomain_with_blocked_ede_is_unknown_regardless_of_authentication() {
        // RFC 8914 EDE 15 (Blocked) on an otherwise-authenticated NXDOMAIN must still read as
        // resolver-side filtering, never as proof of non-registration — the filtering check
        // takes priority over the authentication check.
        let resolver = FakeResolver::new().with_dead_as("example.invalid", true, Some(15));
        assert_eq!(
            check(&resolver, "example.invalid"),
            Verdict::Unknown(UnknownReason::FilteredByResolver)
        );
    }

    #[test]
    fn nxdomain_with_blocked_ede_and_unauthenticated_is_still_filtered_not_unauthenticated() {
        // When both an EDE-filtering signal and a missing authentication are present, the
        // more specific "the resolver is filtering" diagnosis wins over the generic
        // "unauthenticated" one.
        let resolver = FakeResolver::new().with_dead_as("example.invalid", false, Some(15));
        assert_eq!(
            check(&resolver, "example.invalid"),
            Verdict::Unknown(UnknownReason::FilteredByResolver)
        );
    }

    #[test]
    fn nxdomain_with_censored_ede_on_only_one_side_is_unknown() {
        // Only one side needs to carry a filtering EDE code for the pair to be untrusted —
        // "one filtered answer means the resolver filters."
        let resolver = FakeResolver::new()
            .with(
                "example.invalid",
                RecordType::A,
                LookupResult::NxDomain {
                    authenticated: true,
                    extended_error: Some(16), // RFC 8914 §4 — Censored.
                },
            )
            .with(
                "example.invalid",
                RecordType::Aaaa,
                LookupResult::NxDomain {
                    authenticated: true,
                    extended_error: None,
                },
            );
        assert_eq!(
            check(&resolver, "example.invalid"),
            Verdict::Unknown(UnknownReason::FilteredByResolver)
        );
    }

    #[test]
    fn nxdomain_with_a_non_filtering_ede_code_still_requires_authentication() {
        // A non-filtering EDE code (e.g. 3, Stale Answer) must not be mistaken for a
        // filtering one — the authentication rule still applies normally.
        let resolver = FakeResolver::new().with_dead_as("example.invalid", false, Some(3));
        assert_eq!(
            check(&resolver, "example.invalid"),
            Verdict::Unknown(UnknownReason::UnauthenticatedNxDomain)
        );
    }

    #[test]
    fn nxdomain_paired_with_unknown_is_unknown_not_dead() {
        // The named mixed-verdict case from the plan: one family cleanly failing to resolve
        // while the other is merely inconclusive must never be conflated with both families
        // agreeing the domain doesn't exist.
        let resolver = FakeResolver::new()
            .with(
                "example.com",
                RecordType::A,
                LookupResult::NxDomain {
                    authenticated: true,
                    extended_error: None,
                },
            )
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
    fn formerr_is_unknown_not_malformed() {
        // RFC 1035 §4.1.1 RCODE 1 — a middlebox that mangles or refuses AAAA still returns
        // FORMERR, which has its own home distinct from a wire-parse failure (Malformed) or a
        // deliberate policy refusal (Refused).
        let resolver = FakeResolver::new()
            .with(
                "example.com",
                RecordType::A,
                LookupResult::Unknown(UnknownReason::FormErr),
            )
            .with(
                "example.com",
                RecordType::Aaaa,
                LookupResult::Unknown(UnknownReason::FormErr),
            );
        assert_eq!(
            check(&resolver, "example.com"),
            Verdict::Unknown(UnknownReason::FormErr)
        );
    }

    #[test]
    fn notimp_is_unknown_not_refused() {
        // RFC 1035 §4.1.1 RCODE 4 — "not implemented" is a different diagnosis than a deliberate
        // RCODE 5 refusal, and a log line naming the RCODE must be able to say which one fired.
        let resolver = FakeResolver::new()
            .with(
                "example.com",
                RecordType::A,
                LookupResult::Unknown(UnknownReason::NotImp),
            )
            .with(
                "example.com",
                RecordType::Aaaa,
                LookupResult::Unknown(UnknownReason::NotImp),
            );
        assert_eq!(
            check(&resolver, "example.com"),
            Verdict::Unknown(UnknownReason::NotImp)
        );
    }

    #[test]
    fn formerr_paired_with_notimp_keeps_the_a_sides_reason() {
        // Mirrors nxdomain_paired_with_unknown_is_unknown / unknown_paired_with_unknown_is_unknown:
        // two distinct Unknown reasons on either side still combine to Unknown, keeping the A
        // side's reason per combine()'s documented tie-break.
        let resolver = FakeResolver::new()
            .with(
                "example.com",
                RecordType::A,
                LookupResult::Unknown(UnknownReason::FormErr),
            )
            .with(
                "example.com",
                RecordType::Aaaa,
                LookupResult::Unknown(UnknownReason::NotImp),
            );
        assert_eq!(
            check(&resolver, "example.com"),
            Verdict::Unknown(UnknownReason::FormErr)
        );
    }

    #[test]
    fn both_families_nxdomain_via_cname_is_unknown_not_dead() {
        // RFC 6604 §3: the RCODE describes the CNAME chain's terminal target, not the queried
        // name, so a name that resolved via a chain whose target is now NXDOMAIN in both
        // families is not proven unregistered — it must not be pruned as Dead.
        let resolver = FakeResolver::new()
            .with("example.com", RecordType::A, LookupResult::NxDomainViaChain)
            .with(
                "example.com",
                RecordType::Aaaa,
                LookupResult::NxDomainViaChain,
            );
        assert_eq!(
            check(&resolver, "example.com"),
            Verdict::Unknown(UnknownReason::ChainToDeadTarget)
        );
    }

    #[test]
    fn nxdomain_via_cname_paired_with_plain_nxdomain_is_unknown_not_dead() {
        // A direct NXDOMAIN on one family plus a CNAME-chain NXDOMAIN on the other is still
        // not two direct NXDOMAINs on the queried name — only that combination is Dead.
        let resolver = FakeResolver::new()
            .with("example.com", RecordType::A, LookupResult::NxDomainViaChain)
            .with(
                "example.com",
                RecordType::Aaaa,
                LookupResult::NxDomain {
                    authenticated: true,
                    extended_error: None,
                },
            );
        assert_eq!(
            check(&resolver, "example.com"),
            Verdict::Unknown(UnknownReason::ChainToDeadTarget)
        );
    }

    #[test]
    fn nxdomain_via_cname_paired_with_resolved_is_alive() {
        // An alive family wins regardless of what the other side reports.
        let resolver = FakeResolver::new()
            .with("example.com", RecordType::A, LookupResult::NxDomainViaChain)
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
