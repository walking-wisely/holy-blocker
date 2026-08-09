//! The canary: a fixed set of domains with a known-expected [`Verdict`] used to sanity check a
//! resolver before trusting anything it says about the real sweep — [`CanaryConfig`],
//! [`nonce_dead_control`], and [`canary_check`] itself. See the parent module's doc comment for
//! the reference documents this file's citations draw from.

use super::lookup::{DnsLookup, Verdict, check};

/// A control-set failure message stops naming individual mismatches past this many, mirroring
/// `gates::MAX_NAMED_HITS` — canary control sets are expected to be tiny (a handful of domains),
/// so this cap will rarely matter in practice, but it keeps the convention consistent across the
/// crate rather than assuming it away here.
const MAX_NAMED_HITS: usize = 20;

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
