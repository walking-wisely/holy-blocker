//! The canary: a fixed set of domains with a known-expected [`Verdict`] used to sanity check a
//! resolver before trusting anything it says about the real sweep — [`CanaryConfig`],
//! [`nonce_dead_control`], and [`canary_check`] itself. See the parent module's doc comment for
//! the reference documents this file's citations draw from.

use super::lookup::{DnsLookup, Verdict, check};
use crate::types::Timestamp;

/// A control-set failure message stops naming individual mismatches past this many, mirroring
/// `gates::MAX_NAMED_HITS` — canary control sets are expected to be tiny (a handful of domains),
/// so this cap will rarely matter in practice, but it keeps the convention consistent across the
/// crate rather than assuming it away here.
const MAX_NAMED_HITS: usize = 20;

/// A fixed set of domains with a **known** expected [`Verdict`] (never `Unknown`), used to sanity
/// check the resolver itself before trusting anything it says about the real sweep.
///
/// # Known gap: this control set is structurally blind to a resolver that filters only this
/// # module's own category
///
/// **Read this before relying on this canary to catch the failure it exists for.** Every control
/// this type carries today is drawn from *outside* the category the real sweep is checking:
/// `alive_controls` must be "definitely-non-adult" domains, and `dead_controls` are reserved
/// names (`invalid.`, `test.`) or a per-run nonce. A resolver that misbehaves *indiscriminately* —
/// sinks everything, or NXDOMAIN-rewrites everything — trips one of these lists and the canary
/// does its job. A resolver that filters **only the category this sweep targets**, and answers
/// everything else honestly, passes every control in this set cleanly (none of them are in that
/// category) and then silently prunes the real sweep to near-zero. That is not a hypothetical: UK
/// ISP-level content filtering, Italy's AGCOM blocking regime, a CI provider's "family-safe"
/// network default, and Cloudflare's own `1.1.1.3` family-filter resolver variant all have exactly
/// this shape, and this is the single most dangerous failure mode the whole liveness design names
/// (see the "resolver, and the canary check that guards it" section of
/// `docs/decisions/domain-blocklist-sourcing.md`).
///
/// The correct fix is well understood and not applied here: an **in-category** alive control — a
/// domain a category-targeting filter would block, but which is safe to check into a public
/// repository (filtering vendors publish exactly this kind of test hostname for other categories,
/// e.g. Cisco/OpenDNS's malware/phishing test domains). This module cannot supply that value
/// itself: this project's own conventions forbid checking adult-content domains, real or
/// placeholder, into this public repository even as a test fixture, and a vendor-published test
/// domain must be sourced from that vendor's *current* documentation at deployment time, never
/// invented or recalled from memory. Both constraints make this a `cli`-level (module 7, unbuilt)
/// deployment-time data-sourcing responsibility, not something fixable here — see the decision
/// doc's "Open gap" subsection for the full reasoning and the recommended path forward.
///
/// **No type-shape change is needed to support the fix.** `alive_controls: Vec<String>` already
/// accepts a domain from any category — nothing about this struct privileges "non-adult" domains,
/// it is purely that no caller has ever supplied an in-category one. When `cli` is built, its
/// operator supplies one or more current, vendor-published, category-relevant test domains through
/// this exact parameter, alongside the existing non-adult controls.
///
/// The two lists are otherwise deliberately the same shape — both are "domains this build already
/// knows the answer for" — because a resolver can misbehave in either direction: a
/// content-filtering resolver NXDOMAINs real sites it blocks (caught by `alive_controls`, several
/// known-always-alive, definitely-non-adult domains, and — once sourced — the in-category control
/// above), and a wildcard-sink resolver resolves everything, including names that must never
/// resolve (caught by `dead_controls`). `dead_controls` takes more than one for the same reason
/// `alive_controls` does: a single control is one resolver quirk away from a false pass — e.g. a
/// resolver that only mis-answers under `test.` and not `invalid.` — and `canary_check` runs the
/// full set either way, so there is no cost to including more than one.
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
/// (matching this file's existing style, e.g. [`CacheEntry`](super::cache::CacheEntry)'s public
/// fields) because tests and other trusted call sites that already know a literal is valid need
/// to construct one directly — but a struct literal built by hand bypasses the invariant
/// entirely; treat that as a deliberate opt-out, not an oversight.
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

/// Why [`nonce_dead_control`] refused to build a domain. Named per this module's "name the
/// evidence" convention rather than a bare unit error — see [`CanaryConfigError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonceDomainError {
    /// `nonce` was empty.
    EmptyNonce,
    /// `base_domain` was empty.
    EmptyBaseDomain,
    /// `nonce` contained a byte outside RFC 1035's LDH alphabet (letters, digits, hyphen) — see
    /// [`nonce_dead_control`]'s doc comment for why this is checked here rather than left to fail
    /// opaquely as an `Unknown` canary result.
    NonceNotLdh,
    /// A label of `base_domain` (split on `.`) contained a byte outside RFC 1035's LDH alphabet,
    /// or was empty (e.g. a `..` in the input).
    BaseDomainNotLdh,
    /// A single label — `nonce`, or one label of `base_domain` — exceeded the 63-octet cap RFC
    /// 1035 §2.3.4 places on every label.
    LabelTooLong,
    /// The fully constructed `<nonce>.<base_domain>` name exceeded the 255-octet cap RFC 1035
    /// §2.3.4 places on a whole domain name (label octets, label-length octets, and the
    /// terminating root label together).
    NameTooLong,
}

/// A label is valid per RFC 1035's LDH convention (letters, digits, hyphen) if it is non-empty
/// and every byte is ASCII alphanumeric or `-`. RFC 1035 itself defines the wire-format label
/// syntax more permissively (`§2.3.4`); the LDH restriction is the convention actual DNS
/// deployments — and any real client library — enforce for host names, and this label is about to
/// be sent as one, so validating against what a real client will actually accept (rather than the
/// bare wire-format grammar) is what avoids the "fails closed on a real client, but not until the
/// opaque `Unknown` canary result" failure this validation exists to move earlier.
fn is_ldh_label(label: &str) -> bool {
    !label.is_empty()
        && label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// RFC 1035 §2.3.4 caps a single label at this many octets.
const MAX_LABEL_OCTETS: usize = 63;

/// RFC 1035 §2.3.4 caps a whole domain name — label octets, one length octet per label, plus the
/// terminating zero-length root label — at this many octets.
const MAX_NAME_OCTETS: usize = 255;

/// Builds a nonce dead-control domain — a random label under a real, currently-registered base
/// domain, e.g. `<nonce>.example.com`. This function is **pure and deterministic**: it does not
/// generate the nonce itself. `cli` (module 7, unbuilt) is responsible for generating a fresh
/// random nonce per sweep run and calling this to build the actual dead-control domain string
/// before constructing a [`CanaryConfig`].
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
/// to a sink address is caught regardless of how it treats reserved TLDs. **It must be random per
/// run** — a fixed, checked-in nonce could simply be allowlisted by a sink operator, defeating the
/// control. This function deliberately emits the **bare nonce as the whole label**, with no fixed
/// project-identifying prefix: a stable, distinctive prefix (e.g. a checked-in `"hb-canary-"`) is
/// allowlistable by *pattern* almost as cheaply as a fixed nonce is allowlistable by value, and it
/// would additionally brand every probe — a project-identifying label sent, on every sweep, to a
/// third-party base domain's authoritative servers and to whatever resolver operator is between
/// them and the caller — which a local-first project has no reason to broadcast. A caller that
/// wants its own stable debugging convention can still prepend one to the `nonce` it passes in;
/// this function has no opinion on that, it only refuses to hardcode one.
///
/// This is the same general technique Chrome has long used for captive-portal/hijack detection —
/// with one structural difference that is what makes this design still viable where Chrome's
/// stopped being: Chrome's probes were **single-label** names with no registered parent, resolved
/// directly against the root/TLD servers, and Chrome removed that check in M87 because too many
/// enterprise/ISP resolvers legitimately intercept unqualified single-label lookups (e.g. to
/// support search-suggestion or intranet conventions) for the signal to stay reliable. This
/// function's nonce is instead a label **under a real, currently-registered zone**, resolved
/// against that zone's own authoritative servers, never the root — so it doesn't trigger the
/// single-label interception behavior Chrome's probes did, and the failure mode that killed
/// Chrome's version doesn't apply here.
///
/// `base_domain` must be a real, currently-registered domain the caller controls or trusts to
/// always answer honestly for random subdomains (i.e., a domain where an unregistered
/// `<nonce>.base_domain` genuinely NXDOMAINs). This module has no way to verify that property —
/// it's a `cli`-level configuration responsibility, same as [`CanaryConfig::new`]'s non-empty
/// invariant only enforces cardinality, never content.
///
/// **Trap for module 7 (`cli`, unbuilt): `base_domain` must never gain a wildcard DNS record.**
/// If it ever does, `<nonce>.base_domain` resolves for *every* nonce, so the dead control always
/// reports `Alive`, the canary fails on every run, and every sweep aborts — permanently, and
/// silently in the sense that nothing about the failure looks different from a genuinely hijacked
/// resolver. This is a fail-closed outcome, not a fail-open one, but it is a non-obvious
/// availability cliff: whoever configures `base_domain` must keep it wildcard-free for as long as
/// it's used this way.
///
/// # Validation
///
/// Returns `Err` rather than a bare `String` when the constructed name would violate [RFC 1035
/// §2.3.4](https://www.rfc-editor.org/rfc/rfc1035#section-2.3.4) (a label over [`MAX_LABEL_OCTETS`]
/// octets, or a whole name over [`MAX_NAME_OCTETS`] octets) or when `nonce`/`base_domain` contain a
/// byte outside RFC 1035's LDH alphabet (letters, digits, hyphen). A real DNS client library
/// rejects a label or name violating either constraint, so accepting arbitrary bytes here would
/// only defer that rejection to canary time — where it surfaces as an opaque `Verdict::Unknown` on
/// the dead control and the whole sweep aborts with no indication *why*. Catching it at
/// construction time turns that into an immediate, specific [`NonceDomainError`].
pub fn nonce_dead_control(base_domain: &str, nonce: &str) -> Result<String, NonceDomainError> {
    if nonce.is_empty() {
        return Err(NonceDomainError::EmptyNonce);
    }
    if base_domain.is_empty() {
        return Err(NonceDomainError::EmptyBaseDomain);
    }
    if !is_ldh_label(nonce) {
        return Err(NonceDomainError::NonceNotLdh);
    }
    if nonce.len() > MAX_LABEL_OCTETS {
        return Err(NonceDomainError::LabelTooLong);
    }
    for label in base_domain.split('.') {
        if !is_ldh_label(label) {
            return Err(NonceDomainError::BaseDomainNotLdh);
        }
        if label.len() > MAX_LABEL_OCTETS {
            return Err(NonceDomainError::LabelTooLong);
        }
    }

    let domain = format!("{nonce}.{base_domain}");

    // RFC 1035 §2.3.4: total wire-format length is every label's length octet plus its content
    // octets, plus the terminating zero-length root label — not just the presentation-format
    // string length, which omits the length octets and the root label entirely.
    let wire_octets: usize = domain
        .split('.')
        .map(|label| label.len() + 1)
        .sum::<usize>()
        + 1;
    if wire_octets > MAX_NAME_OCTETS {
        return Err(NonceDomainError::NameTooLong);
    }

    Ok(domain)
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
///
/// Both variants carry `checked_at`, the `now: Timestamp` [`canary_check`] was called with. A
/// single check at the start of a long sweep can't see a resolver that starts lying partway
/// through — see [`canary_check`]'s doc comment for why a caller is expected to call it
/// repeatedly across a sweep, not just once — and a caller doing that needs to know *when* each
/// result happened to build a "discard back to the last passing canary" recovery strategy,
/// without inventing its own timestamp-tracking wrapper around this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanaryResult {
    Passed {
        checked_at: Timestamp,
    },
    Failed {
        failures: Vec<String>,
        checked_at: Timestamp,
    },
}

/// Runs every control in `config` — both `alive_controls` and `dead_controls` — against `resolver`
/// and reports whether each answered as expected, stamping the result with `now`. Per the plan,
/// **any** canary result other than expected must abort the entire sweep — this function only
/// reports that; discarding the sweep's verdicts and refusing to write the cache back is the
/// caller's (`cli`, module 7, unbuilt) responsibility, since there is no way to know at which
/// query a lying resolver started lying and so no partial sweep to salvage.
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
///
/// # Interleave this through a sweep, not just once at t=0
///
/// The plan justifies gating a sweep on the canary with "there is no way to know at which query a
/// lying resolver started lying" — but that reasoning argues *for* calling this repeatedly through
/// a long sweep, not just once before it starts. A full sweep runs for roughly 24 hours (the
/// plan's qps budget), and over that span an anycast route can flip onto a filtering node, the
/// runner's own network can change, a DHCP lease can renew onto a captive portal, or the resolver
/// can start rate-limiting — none of which a single check at the start can see, since all of them
/// can happen strictly after it passed.
///
/// **This function does not do that scheduling itself — pacing/interleaving a sweep is a `cli`-
/// level (module 7, unbuilt) concern, per this module's existing split between pure decision logic
/// and the real network loop.** What this function contributes is making that safe to build: call
/// it periodically (e.g. every few thousand queries) through the sweep loop with the current
/// `now`, and use each [`CanaryResult::checked_at`](CanaryResult) to discard verdicts collected
/// back to the last passing canary rather than either the whole sweep or nothing.
pub fn canary_check<R: DnsLookup + ?Sized>(
    resolver: &R,
    config: &CanaryConfig,
    now: Timestamp,
) -> CanaryResult {
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
        return CanaryResult::Passed { checked_at: now };
    }

    if failures.len() > MAX_NAMED_HITS {
        let more = failures.len() - MAX_NAMED_HITS;
        failures.truncate(MAX_NAMED_HITS);
        failures.push(format!("...and {more} more"));
    }

    CanaryResult::Failed {
        failures,
        checked_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::liveness::test_support::FakeResolver;

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
        fn builds_the_expected_label_format_with_no_project_branding() {
            // The label is the bare nonce — no "hb-canary-" prefix, since a stable, distinctive
            // prefix is allowlistable by pattern almost as easily as a fixed nonce, and it would
            // brand every probe with a project-identifying label sent to a third-party zone.
            assert_eq!(
                nonce_dead_control("example.com", "abc123"),
                Ok("abc123.example.com".to_string())
            );
        }

        #[test]
        fn different_nonces_produce_different_domains() {
            let first = nonce_dead_control("example.com", "aaaa").unwrap();
            let second = nonce_dead_control("example.com", "bbbb").unwrap();
            assert_ne!(first, second);
        }

        #[test]
        fn different_base_domains_produce_different_domains() {
            let first = nonce_dead_control("example.com", "abc123").unwrap();
            let second = nonce_dead_control("example.org", "abc123").unwrap();
            assert_ne!(first, second);
        }

        #[test]
        fn empty_nonce_is_rejected() {
            assert_eq!(
                nonce_dead_control("example.com", ""),
                Err(NonceDomainError::EmptyNonce)
            );
        }

        #[test]
        fn empty_base_domain_is_rejected() {
            assert_eq!(
                nonce_dead_control("", "abc123"),
                Err(NonceDomainError::EmptyBaseDomain)
            );
        }

        #[test]
        fn a_nonce_with_non_ldh_characters_is_rejected() {
            // RFC 1035 labels are letters/digits/hyphen only; an underscore or a dot embedded in
            // the nonce would either be silently mangled by a real client or smuggle an extra
            // label boundary into what is supposed to be one opaque label.
            assert_eq!(
                nonce_dead_control("example.com", "abc_123"),
                Err(NonceDomainError::NonceNotLdh)
            );
            assert_eq!(
                nonce_dead_control("example.com", "abc.123"),
                Err(NonceDomainError::NonceNotLdh)
            );
        }

        #[test]
        fn a_base_domain_with_non_ldh_characters_is_rejected() {
            assert_eq!(
                nonce_dead_control("exa mple.com", "abc123"),
                Err(NonceDomainError::BaseDomainNotLdh)
            );
        }

        #[test]
        fn a_base_domain_with_an_empty_label_is_rejected() {
            // "example..com" splits into an empty middle label, which is not a valid LDH label.
            assert_eq!(
                nonce_dead_control("example..com", "abc123"),
                Err(NonceDomainError::BaseDomainNotLdh)
            );
        }

        #[test]
        fn a_label_over_63_octets_is_rejected() {
            // RFC 1035 §2.3.4 caps each label at 63 octets.
            let long_label = "a".repeat(64);
            assert_eq!(
                nonce_dead_control(&format!("{long_label}.com"), "abc123"),
                Err(NonceDomainError::LabelTooLong)
            );
            assert_eq!(
                nonce_dead_control("example.com", &long_label),
                Err(NonceDomainError::LabelTooLong)
            );
        }

        #[test]
        fn a_label_of_exactly_63_octets_is_accepted() {
            let max_label = "a".repeat(63);
            assert!(nonce_dead_control(&format!("{max_label}.com"), "abc123").is_ok());
        }

        #[test]
        fn a_name_over_255_octets_is_rejected() {
            // RFC 1035 §2.3.4 caps the whole name (label octets + label length octets, plus the
            // root label) at 255 octets — build a base domain made of several max-length labels
            // to push the constructed name over that limit.
            let label = "a".repeat(63);
            let base_domain = format!("{label}.{label}.{label}.{label}");
            assert_eq!(
                nonce_dead_control(&base_domain, "abc123"),
                Err(NonceDomainError::NameTooLong)
            );
        }
    }

    mod canary_check_tests {
        use super::*;
        use crate::liveness::lookup::{LookupResult, RecordType, UnknownReason};

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
            assert_eq!(
                canary_check(&resolver, &config(), 1_000),
                CanaryResult::Passed { checked_at: 1_000 }
            );
        }

        #[test]
        fn passed_and_failed_both_carry_the_now_they_were_checked_at() {
            // A caller interleaving canary_check through a long sweep needs each result's
            // timestamp to build a "discard back to the last passing canary" recovery strategy —
            // this pins that both variants carry the exact now passed in, not just Failed.
            let healthy = FakeResolver::new()
                .with_alive("example.com")
                .with_alive("iana.org")
                .with_dead("invalid.")
                .with_dead("example.test.");
            assert_eq!(
                canary_check(&healthy, &config(), 12_345),
                CanaryResult::Passed { checked_at: 12_345 }
            );

            let sink = FakeResolver::new()
                .with_alive("example.com")
                .with_alive("iana.org")
                .with_alive("invalid.")
                .with_alive("example.test.");
            match canary_check(&sink, &config(), 67_890) {
                CanaryResult::Failed { checked_at, .. } => assert_eq!(checked_at, 67_890),
                CanaryResult::Passed { .. } => panic!("expected the canary to fail"),
            }
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
            match canary_check(&resolver, &config(), 1_000) {
                CanaryResult::Failed { failures, .. } => {
                    assert_eq!(failures.len(), 1);
                    assert!(failures[0].contains("example.com"));
                    assert!(failures[0].contains("Alive"));
                }
                CanaryResult::Passed { .. } => panic!("expected the canary to fail"),
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
            match canary_check(&resolver, &config(), 1_000) {
                CanaryResult::Failed { failures, .. } => {
                    assert_eq!(failures.len(), 2);
                    assert!(failures.iter().any(|f| f.contains("invalid.")));
                    assert!(failures.iter().any(|f| f.contains("example.test.")));
                    assert!(failures.iter().all(|f| f.contains("Dead")));
                }
                CanaryResult::Passed { .. } => panic!("expected the canary to fail"),
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
            match canary_check(&resolver, &config(), 1_000) {
                CanaryResult::Failed { failures, .. } => {
                    assert_eq!(failures.len(), 1);
                    assert!(failures[0].contains("example.test."));
                    assert!(failures[0].contains("Dead"));
                }
                CanaryResult::Passed { .. } => panic!("expected the canary to fail"),
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
                canary_check(&resolver, &config(), 1_000),
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
            match canary_check(&resolver, &config(), 1_000) {
                CanaryResult::Failed { failures, .. } => {
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
                CanaryResult::Passed { .. } => panic!("expected the canary to fail"),
            }
        }
    }
}
