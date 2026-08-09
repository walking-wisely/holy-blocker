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
//!   [`LookupResult::NxDomainViaChain`] exists and is never treated as proof the queried name itself
//!   is unregistered.
//! - [RFC 6672](https://www.rfc-editor.org/rfc/rfc6672) — DNAME redirection, which shares RFC
//!   6604 §3's "the RCODE describes the target" property identically (a DNAME is itself resolved
//!   via a synthesized CNAME) and so is folded into the same [`LookupResult::NxDomainViaChain`]
//!   variant; §2.2 also defines YXDOMAIN (RCODE 6), returned when a DNAME substitution would exceed
//!   255 octets.
//! - [RFC 8914](https://www.rfc-editor.org/rfc/rfc8914) — Extended DNS Errors. §4 defines the code
//!   registry [`LookupResult::NxDomain::extended_error`] stores raw values from; codes 4 (Forged
//!   Answer), 6 (DNSSEC Bogus), 15 (Blocked), 16 (Censored) and 17 (Filtered) are "this negative
//!   answer is policy, not fact" signals [`combine`] reads as proof the resolver itself is not to
//!   be trusted for this domain, never as proof of non-registration. `authenticated` on the same
//!   variant reflects the DNSSEC AD bit (a validating resolver's own claim that NSEC/NSEC3 proved
//!   the name doesn't exist) rather than a spec this module parses a wire format against — the AD
//!   bit itself is defined by [RFC 4035 §3.2.3](https://www.rfc-editor.org/rfc/rfc4035#section-3.2.3).
//! - [RFC 1035 §4.2.1](https://www.rfc-editor.org/rfc/rfc1035#section-4.2.1) and [RFC
//!   7766](https://www.rfc-editor.org/rfc/rfc7766) — TC=1 truncation and the mandatory TCP retry; see
//!   [`DnsLookup`]'s doc comment for the implementation contract this module cannot enforce through
//!   its return type alone.

mod cache;
mod canary;
mod lookup;

pub use cache::{CacheEntry, due_for_check, is_special_use_domain, should_prune};
pub use canary::{CanaryConfig, CanaryConfigError, CanaryResult, canary_check, nonce_dead_control};
pub use lookup::{DnsLookup, LookupResult, RecordType, UnknownReason, Verdict, check, combine};

/// Test-only fixtures shared by [`lookup`]'s and [`canary`]'s test modules — kept here, one level
/// up from both, since neither concern owns the other and duplicating the fixture would risk the
/// two copies drifting apart.
#[cfg(test)]
mod test_support {
    use super::lookup::{DnsLookup, LookupResult, RecordType, UnknownReason};
    use std::collections::HashMap;

    /// A resolver whose answers are configured per `(domain, RecordType)`. Any combination not
    /// explicitly configured answers `Unknown(NoData)` — deliberately not `NxDomain`, so a test
    /// fixture typo (an unconfigured domain) never silently reads as a legitimate negative answer.
    pub(super) struct FakeResolver {
        answers: HashMap<(String, RecordType), LookupResult>,
    }

    impl FakeResolver {
        pub(super) fn new() -> Self {
            FakeResolver {
                answers: HashMap::new(),
            }
        }

        pub(super) fn with(
            mut self,
            domain: &str,
            record: RecordType,
            result: LookupResult,
        ) -> Self {
            self.answers.insert((domain.to_string(), record), result);
            self
        }

        /// Both A and AAAA resolve for `domain`, to a fixed RFC 5737 §3 TEST-NET-1 documentation
        /// address (`192.0.2.1`) — the specific value never matters to any test built on this
        /// helper, since `combine` only asks whether a lookup resolved, not what it resolved to.
        pub(super) fn with_alive(self, domain: &str) -> Self {
            let addr = std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, 1));
            self.with(domain, RecordType::A, LookupResult::Resolved(vec![addr]))
                .with(domain, RecordType::Aaaa, LookupResult::Resolved(vec![addr]))
        }

        /// Both A and AAAA come back a clean, DNSSEC-authenticated NXDOMAIN for `domain` — an
        /// honest resolver's answer, and the only combination [`combine`] will ever call `Dead`.
        pub(super) fn with_dead(self, domain: &str) -> Self {
            self.with_dead_as(domain, true, None)
        }

        /// Both A and AAAA come back NXDOMAIN for `domain`, with the given DNSSEC-authentication
        /// and RFC 8914 Extended DNS Error fields — the general form the authentication/EDE tests
        /// build on, and that `with_dead` is just the "honest resolver" special case of.
        pub(super) fn with_dead_as(
            self,
            domain: &str,
            authenticated: bool,
            extended_error: Option<u16>,
        ) -> Self {
            self.with(
                domain,
                RecordType::A,
                LookupResult::NxDomain {
                    authenticated,
                    extended_error,
                },
            )
            .with(
                domain,
                RecordType::Aaaa,
                LookupResult::NxDomain {
                    authenticated,
                    extended_error,
                },
            )
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
}
