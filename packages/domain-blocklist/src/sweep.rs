//! The real, qps-paced, corroborated, canary-gated DNS liveness sweep — the piece module 3's plan
//! section deliberately deferred to `cli` ("the real network resolver ... and the qps-paced sweep
//! loop are left to `cli` (module 7, unbuilt)"). Feature-gated behind `net`, same as
//! `liveness::net`, since it exists only to drive that module's real [`HickoryDnsLookup`].
//!
//! This module does **not** reimplement any policy `liveness/` already owns — it drives
//! `liveness`'s own pure functions (`combine`, `corroborate`, `should_prune_with_hysteresis`,
//! `due_for_check`) with real, async I/O behind them. The one piece of logic duplicated rather than
//! reused is `canary::canary_check`/`lookup::check`'s pass/fail loop itself: both are written in
//! terms of the crate's *synchronous* `DnsLookup::lookup`, and calling `HickoryDnsLookup`'s sync
//! trait method (which internally `block_on`s its own runtime) from **inside** this module's
//! already-running async context would panic — "cannot start a runtime from within a runtime" is
//! documented tokio behavior, and the fetcher modules hit and recorded the identical trap for their
//! own sync/async bridge. [`check_async`]/[`canary_check_async`] are therefore async-native
//! re-implementations that call [`HickoryDnsLookup::lookup_async`] directly and otherwise apply the
//! *exact* same evaluation rules (`combine`'s three-step order; canary's alive/dead-control
//! equality check) — kept intentionally tiny (a handful of lines each) so the duplication is easy
//! to eyeball against the original rather than a second copy of real policy.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use domain_blocklist::liveness::{HickoryDnsLookup, ResolverConfig};
use domain_blocklist::{
    CacheEntry, CanaryConfig, LookupResult, MergedEntry, RecordType, Timestamp, UnknownReason,
    Verdict, combine as combine_verdict, corroborate, due_for_check, should_prune_with_hysteresis,
};
use futures::stream::{self, StreamExt};

/// The injection seam this module needs to be testable without real UDP/53: an async single-QTYPE
/// lookup, the same contract [`domain_blocklist::DnsLookup`] states for its synchronous
/// `lookup(&self, ...)`, minus the sync/async bridging that trait's tests-need-no-runtime design
/// requires. [`HickoryDnsLookup`] already has an inherent `lookup_async` with exactly this
/// signature (see its own doc comment for why it's async-native rather than going through the sync
/// trait) — this trait exists only so [`run_sweep_with_resolvers`] can be generic over it and a
/// test double can stand in for a real resolver.
pub trait AsyncDnsLookup: Send + Sync {
    fn lookup_async(
        &self,
        domain: &str,
        record: RecordType,
    ) -> impl std::future::Future<Output = LookupResult> + Send;
}

impl AsyncDnsLookup for HickoryDnsLookup {
    fn lookup_async(
        &self,
        domain: &str,
        record: RecordType,
    ) -> impl std::future::Future<Output = LookupResult> + Send {
        HickoryDnsLookup::lookup_async(self, domain, record)
    }
}

/// Pacing, corroboration, and gating knobs for one sweep run. Every threshold here is the plan's
/// own starting value, not a defended constant — see `docs/decisions/domain-blocklist-sourcing.md`
/// ("Sweep pacing") and `liveness/cache.rs`'s hysteresis doc comment for where each number comes
/// from.
#[derive(Debug, Clone)]
pub struct SweepConfig {
    pub primary: ResolverConfig,
    pub secondary: ResolverConfig,
    /// Target domain-starts per second. The plan derives ~23 qps from "~1,000,000 domains, ~2
    /// queries each, over a 24-hour window"; this module paces domain *dispatch*, not raw query
    /// count, since each domain costs 2–4 queries (A, sometimes AAAA, times two resolvers) — a
    /// caller wanting the plan's exact raw-query qps should divide accordingly when setting this.
    pub qps: f64,
    /// Max domain-checks in flight at once. Bounds memory/socket use independent of `qps` — see
    /// this module's doc comment on why pacing (via `qps`) and concurrency (via this) are two
    /// separate levers.
    pub concurrency: usize,
    /// How many domains to process between canary re-checks. The plan: "interleave through a long
    /// sweep... call it periodically (e.g. every few thousand queries)".
    pub canary_every: usize,
    pub ttl_seconds: u64,
    pub quarantine_seconds: u64,
}

/// What one sweep run produced: the updated cache (every entry it touched) and which domains, of
/// those the caller asked about, are now pruned.
#[derive(Debug)]
pub struct SweepOutcome {
    pub cache: HashMap<String, CacheEntry>,
    pub pruned_domains: std::collections::BTreeSet<String>,
    pub checked_count: usize,
}

/// Why a sweep was abandoned. Per the plan, **every** verdict from an aborted sweep is discarded —
/// not merged, not written back to the cache — because there is no way to know at which query a
/// lying resolver started lying. The caller (`cli`'s pipeline) must not persist [`SweepOutcome`] on
/// this path; it never receives one.
#[derive(Debug, thiserror::Error)]
pub enum SweepError {
    #[error("initial canary check failed against the {resolver} resolver: {detail}")]
    InitialCanaryFailed { resolver: &'static str, detail: String },
    #[error(
        "canary check failed mid-sweep (after {checked_before_failure} domains) against the {resolver} resolver: {detail} — the whole sweep's results are discarded, per the plan's \"a partial sweep is not salvaged\" rule"
    )]
    MidSweepCanaryFailed {
        resolver: &'static str,
        checked_before_failure: usize,
        detail: String,
    },
    #[error("failed to start the DNS client: {0}")]
    Startup(#[from] std::io::Error),
}

/// Async re-implementation of [`domain_blocklist::liveness::check`], driving
/// [`HickoryDnsLookup::lookup_async`] directly rather than the sync `DnsLookup::lookup` — see this
/// module's doc comment for why. Applies the identical short-circuit `combine` itself documents: A
/// resolving skips the AAAA query entirely, since no AAAA answer can change an already-`Alive`
/// verdict.
async fn check_async<L: AsyncDnsLookup>(resolver: &L, domain: &str) -> Verdict {
    let a = resolver.lookup_async(domain, RecordType::A).await;
    if matches!(a, LookupResult::Resolved(_)) {
        return Verdict::Alive;
    }
    let aaaa = resolver.lookup_async(domain, RecordType::Aaaa).await;
    combine_verdict(a, aaaa)
}

/// Async re-implementation of [`domain_blocklist::check_corroborated`] over two resolvers, run
/// concurrently.
async fn check_corroborated_async<L: AsyncDnsLookup>(
    primary: &L,
    secondary: &L,
    domain: &str,
) -> Verdict {
    let (v1, v2) = tokio::join!(check_async(primary, domain), check_async(secondary, domain));
    corroborate(v1, v2)
}

/// Async re-implementation of [`domain_blocklist::canary_check`]'s pass/fail evaluation against one
/// resolver — same alive/dead-control equality rule, no policy divergence, just async-native.
async fn canary_pass_async<L: AsyncDnsLookup>(
    resolver: &L,
    canary: &CanaryConfig,
) -> Result<(), String> {
    let mut failures = Vec::new();
    for domain in &canary.alive_controls {
        let v = check_async(resolver, domain).await;
        if v != Verdict::Alive {
            failures.push(format!("alive control {domain:?} expected Alive, got {v:?}"));
        }
    }
    for domain in &canary.dead_controls {
        let v = check_async(resolver, domain).await;
        if v != Verdict::Dead {
            failures.push(format!("dead control {domain:?} expected Dead, got {v:?}"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

async fn canary_pass_both<L: AsyncDnsLookup>(
    primary: &L,
    secondary: &L,
    canary: &CanaryConfig,
) -> Result<(), (&'static str, String)> {
    canary_pass_async(primary, canary)
        .await
        .map_err(|d| ("primary", d))?;
    canary_pass_async(secondary, canary)
        .await
        .map_err(|d| ("secondary", d))?;
    Ok(())
}

/// Runs one full sweep over `entries`, checking only the domains [`due_for_check`] says are due
/// against `cache`/`ttl_seconds`, gated by an initial canary and re-checked every `canary_every`
/// domains. On any canary failure (initial or mid-sweep) this returns `Err` and the caller must
/// discard everything — no partial cache update, no partial prune list — per the plan.
///
/// Domains not due this run are still subject to [`should_prune_with_hysteresis`] against their
/// existing cached verdict, so a domain already inside its quarantine window (or already pruned
/// with an established `first_dead_at` streak) stays correctly pruned/kept without needing a fresh
/// lookup every run.
pub async fn run_sweep(
    entries: &[MergedEntry],
    cache: HashMap<String, CacheEntry>,
    canary: &CanaryConfig,
    config: &SweepConfig,
    now: Timestamp,
) -> Result<SweepOutcome, SweepError> {
    let primary = Arc::new(HickoryDnsLookup::new(config.primary)?);
    let secondary = Arc::new(HickoryDnsLookup::new(config.secondary)?);
    run_sweep_with_resolvers(entries, cache, canary, config, now, primary, secondary).await
}

/// The real body of [`run_sweep`], generic over [`AsyncDnsLookup`] so a test can drive the canary
/// gate, the mid-sweep abort, the `first_dead_at` streak update, and the
/// `should_prune_with_hysteresis` pass with no UDP/53 — the rules that decide whether a domain
/// leaves a signed artifact, per this module's own doc comment on why an aborted sweep discards
/// everything. `run_sweep` is the thin wrapper that constructs the real [`HickoryDnsLookup`]
/// resolvers `main.rs` actually calls.
pub async fn run_sweep_with_resolvers<L: AsyncDnsLookup>(
    entries: &[MergedEntry],
    mut cache: HashMap<String, CacheEntry>,
    canary: &CanaryConfig,
    config: &SweepConfig,
    now: Timestamp,
    primary: Arc<L>,
    secondary: Arc<L>,
) -> Result<SweepOutcome, SweepError> {
    if let Err((resolver, detail)) = canary_pass_both(&*primary, &*secondary, canary).await {
        return Err(SweepError::InitialCanaryFailed { resolver, detail });
    }

    let due_domains: Vec<String> = entries
        .iter()
        .filter(|e| due_for_check(cache.get(&e.domain), now, config.ttl_seconds))
        .map(|e| e.domain.clone())
        .collect();

    let mut checked = 0usize;
    let canary_every = config.canary_every.max(1);

    for chunk in due_domains.chunks(canary_every) {
        let interval = Duration::from_secs_f64(1.0 / config.qps.max(0.001));
        let start = tokio::time::Instant::now();
        let results: Vec<(String, Verdict)> = stream::iter(chunk.iter().cloned().enumerate())
            .map(|(idx, domain)| {
                let primary = Arc::clone(&primary);
                let secondary = Arc::clone(&secondary);
                async move {
                    let deadline = start + interval * (idx as u32);
                    tokio::time::sleep_until(deadline).await;
                    let verdict = check_corroborated_async(&*primary, &*secondary, &domain).await;
                    (domain, verdict)
                }
            })
            .buffer_unordered(config.concurrency.max(1))
            .collect()
            .await;

        for (domain, verdict) in results {
            let entry_before = cache.get(&domain).copied().unwrap_or(CacheEntry {
                last_checked: 0,
                verdict: Verdict::Unknown(UnknownReason::NoData),
                first_dead_at: None,
            });
            let first_dead_at = match verdict {
                Verdict::Dead => Some(entry_before.first_dead_at.unwrap_or(now)),
                _ => None,
            };
            cache.insert(
                domain,
                CacheEntry {
                    last_checked: now,
                    verdict,
                    first_dead_at,
                },
            );
            checked += 1;
        }

        if let Err((resolver, detail)) = canary_pass_both(&*primary, &*secondary, canary).await {
            return Err(SweepError::MidSweepCanaryFailed {
                resolver,
                checked_before_failure: checked,
                detail,
            });
        }
    }

    let mut pruned_domains = std::collections::BTreeSet::new();
    for entry in entries {
        if let Some(cached) = cache.get(&entry.domain)
            && should_prune_with_hysteresis(
                cached,
                &entry.domain,
                cached.verdict,
                now,
                config.quarantine_seconds,
            )
        {
            pruned_domains.insert(entry.domain.clone());
        }
    }

    Ok(SweepOutcome {
        cache,
        pruned_domains,
        checked_count: checked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain_normalize::RuleScope;
    use std::collections::HashMap as StdHashMap;
    use std::sync::Mutex;

    /// A fake [`AsyncDnsLookup`] with no network access: answers a fixed [`Verdict`]-shaped
    /// [`LookupResult`] pair for every domain it's told about via [`FakeResolver::alive`]/
    /// [`FakeResolver::dead`], and panics on an unconfigured domain — a mid-sweep call this test
    /// didn't expect is a test bug, not a silently-Unknown result that would mask it.
    #[derive(Default)]
    struct FakeResolver {
        answers: Mutex<StdHashMap<String, LookupResult>>,
    }

    impl FakeResolver {
        fn new() -> Self {
            Self::default()
        }

        fn with(self, domain: &str, result: LookupResult) -> Self {
            self.answers.lock().unwrap().insert(domain.to_string(), result);
            self
        }

        fn alive(self, domain: &str) -> Self {
            self.with(
                domain,
                LookupResult::Resolved(vec!["203.0.113.1".parse().unwrap()]),
            )
        }

        fn dead(self, domain: &str) -> Self {
            self.with(
                domain,
                LookupResult::NxDomain { authenticated: false, extended_error: None },
            )
        }
    }

    impl AsyncDnsLookup for FakeResolver {
        async fn lookup_async(&self, domain: &str, _record: RecordType) -> LookupResult {
            self.answers
                .lock()
                .unwrap()
                .get(domain)
                .cloned()
                .unwrap_or_else(|| panic!("FakeResolver has no configured answer for {domain:?}"))
        }
    }

    fn dummy_resolver_config() -> ResolverConfig {
        ResolverConfig {
            addr: "127.0.0.1:53".parse().unwrap(),
            timeout: Duration::from_millis(1),
        }
    }

    fn config() -> SweepConfig {
        SweepConfig {
            primary: dummy_resolver_config(),
            secondary: dummy_resolver_config(),
            qps: 10_000.0, // effectively no pacing delay in a test
            concurrency: 8,
            canary_every: 1000,
            ttl_seconds: 90 * 24 * 60 * 60,
            quarantine_seconds: 180 * 24 * 60 * 60,
        }
    }

    fn entry(domain: &str) -> MergedEntry {
        MergedEntry {
            domain: domain.to_string(),
            scope: RuleScope::ExactHost,
            sources: Vec::new(),
            categories: Vec::new(),
        }
    }

    fn canary() -> CanaryConfig {
        CanaryConfig::new(vec!["alive.invalid".to_string()], vec!["dead.invalid".to_string()])
            .expect("non-empty control lists")
    }

    #[tokio::test]
    async fn initial_canary_failure_returns_initial_canary_failed_and_no_cache() {
        // The canary's own alive control resolves Dead — a resolver that can't even pass its own
        // sanity check must never reach the sweep loop at all.
        let resolver = Arc::new(FakeResolver::new().dead("alive.invalid").dead("dead.invalid"));
        let result = run_sweep_with_resolvers(
            &[entry("example.com")],
            HashMap::new(),
            &canary(),
            &config(),
            1_000_000,
            Arc::clone(&resolver),
            Arc::clone(&resolver),
        )
        .await;

        assert!(matches!(result, Err(SweepError::InitialCanaryFailed { .. })));
    }

    /// A resolver whose answer for `dead.invalid` flips from `Dead` to `Alive` after
    /// [`FLIP_AFTER`] `A`-record queries for it — simulating a dead control that stops answering
    /// NXDOMAIN partway through a sweep (a resolver starting to filter, a control domain getting
    /// accidentally registered). Every other configured domain answers a fixed result throughout.
    /// Counting only `A`-record queries to `dead.invalid` (never `AAAA`, never any other domain)
    /// makes the flip point depend on how many times *that specific check* ran, not on incidental
    /// concurrent-task interleaving from `buffer_unordered`.
    struct FlippingDeadControl {
        alive_control: String,
        dead_control: String,
        due_domain: String,
        due_domain_answer: LookupResult,
        dead_control_a_queries: std::sync::atomic::AtomicUsize,
    }

    const FLIP_AFTER: usize = 2;

    impl AsyncDnsLookup for FlippingDeadControl {
        async fn lookup_async(&self, domain: &str, record: RecordType) -> LookupResult {
            if domain == self.alive_control {
                return LookupResult::Resolved(vec!["203.0.113.1".parse().unwrap()]);
            }
            if domain == self.dead_control {
                if record == RecordType::A {
                    let seen = self
                        .dead_control_a_queries
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if seen >= FLIP_AFTER {
                        return LookupResult::Resolved(vec!["203.0.113.2".parse().unwrap()]);
                    }
                }
                return LookupResult::NxDomain { authenticated: false, extended_error: None };
            }
            if domain == self.due_domain {
                return self.due_domain_answer.clone();
            }
            panic!("FlippingDeadControl has no configured answer for {domain:?}");
        }
    }

    #[tokio::test]
    async fn mid_sweep_canary_failure_returns_mid_sweep_canary_failed_and_no_partial_prune_list() {
        // canary_every = 1 forces a re-check immediately after the single due domain is
        // processed — the initial canary (2 A-queries to the dead control, one per resolver)
        // passes under FLIP_AFTER = 2, and the mid-sweep re-check's first A-query (the 3rd) is
        // what trips the flip.
        let resolver = Arc::new(FlippingDeadControl {
            alive_control: "alive.invalid".to_string(),
            dead_control: "dead.invalid".to_string(),
            due_domain: "flaky.example".to_string(),
            due_domain_answer: LookupResult::NxDomain { authenticated: false, extended_error: None },
            dead_control_a_queries: std::sync::atomic::AtomicUsize::new(0),
        });

        let mut cfg = config();
        cfg.canary_every = 1;

        let result = run_sweep_with_resolvers(
            &[entry("flaky.example")],
            HashMap::new(),
            &canary(),
            &cfg,
            1_000_000,
            Arc::clone(&resolver),
            Arc::clone(&resolver),
        )
        .await;

        match result {
            Err(SweepError::MidSweepCanaryFailed { checked_before_failure, .. }) => {
                assert_eq!(checked_before_failure, 1, "the one due domain was checked before the mid-sweep canary tripped");
            }
            other => panic!("expected MidSweepCanaryFailed, got {other:?}"),
        }
        // The plan's "a partial sweep is not salvaged" rule: an Err carries no SweepOutcome at
        // all, so there is no cache update and no prune list to inspect — the type itself is the
        // guarantee, this assertion just makes that explicit for the reader.
    }

    #[tokio::test]
    async fn a_dead_verdict_inside_its_quarantine_window_is_not_pruned() {
        let now: Timestamp = 1_000_000;
        let mut cache = HashMap::new();
        cache.insert(
            "recently-dead.dropped-domain.net".to_string(),
            CacheEntry {
                last_checked: now,
                verdict: Verdict::Dead,
                first_dead_at: Some(now - 10), // well inside a long quarantine window
            },
        );
        let mut cfg = config();
        cfg.quarantine_seconds = 1_000;
        cfg.ttl_seconds = u64::MAX / 2; // never due again this run — exercises the not-due path
        // The initial canary still runs regardless of whether any entry is due for a fresh
        // lookup, so its two controls must be configured even though this test's own domain
        // never triggers a real lookup.
        let resolver = Arc::new(FakeResolver::new().alive("alive.invalid").dead("dead.invalid"));

        let outcome = run_sweep_with_resolvers(
            &[entry("recently-dead.dropped-domain.net")],
            cache,
            &canary(),
            &cfg,
            now,
            Arc::clone(&resolver),
            Arc::clone(&resolver),
        )
        .await
        .expect("no domain is due, so no lookup happens and no canary runs");

        assert!(outcome.pruned_domains.is_empty());
        assert_eq!(outcome.checked_count, 0);
    }

    #[tokio::test]
    async fn a_dead_verdict_past_its_quarantine_window_is_pruned() {
        let now: Timestamp = 1_000_000;
        let mut cache = HashMap::new();
        cache.insert(
            "long-dead.dropped-domain.net".to_string(),
            CacheEntry {
                last_checked: now,
                verdict: Verdict::Dead,
                first_dead_at: Some(now - 10_000), // outside a short quarantine window
            },
        );
        let mut cfg = config();
        cfg.quarantine_seconds = 1_000;
        cfg.ttl_seconds = u64::MAX / 2;
        let resolver = Arc::new(FakeResolver::new().alive("alive.invalid").dead("dead.invalid"));

        let outcome = run_sweep_with_resolvers(
            &[entry("long-dead.dropped-domain.net")],
            cache,
            &canary(),
            &cfg,
            now,
            Arc::clone(&resolver),
            Arc::clone(&resolver),
        )
        .await
        .expect("no domain is due, so no lookup happens and no canary runs");

        assert_eq!(outcome.pruned_domains, ["long-dead.dropped-domain.net".to_string()].into());
    }

    /// Live smoke test: runs one real sweep against the actual `1.1.1.1`/`8.8.8.8` resolvers over
    /// real UDP/53, for a handful of domains, at a modest `qps`. Exists to answer one question
    /// before ever committing to an all-day production sweep — does the pacing, corroboration and
    /// canary machinery actually work against real infrastructure — without waiting 24 hours to
    /// find out. `#[ignore]`d, per this module's own doc comment ("exercised only by `#[ignore]`d
    /// smoke tests a caller opts into explicitly") and `liveness/net.rs`'s identical note — opt in
    /// with `cargo test --features net --release -- --ignored live_sweep_smoke_test`.
    ///
    /// Controls are chosen to avoid the two traps `liveness/canary.rs`'s own doc comments record:
    /// alive controls are well-known, definitely-non-adult, definitely-up domains (never
    /// `example.com`, whose *subdomains* answer NODATA rather than NXDOMAIN — irrelevant to an
    /// alive control, but avoided anyway to keep this test's domain list boringly uncontroversial);
    /// the dead control is a fresh label under the RFC 2606/6761-reserved `.invalid` TLD, which
    /// sits directly under the root zone (plain NSEC, no NSEC3 opt-out) and so NXDOMAINs cleanly on
    /// every honest resolver, per module 3's own measured finding.
    #[tokio::test]
    #[ignore = "touches real UDP/53 against public resolvers — run explicitly, not part of `cargo test --features net`"]
    async fn live_sweep_smoke_test_against_real_resolvers() {
        let config = SweepConfig {
            primary: ResolverConfig {
                addr: "1.1.1.1:53".parse().unwrap(),
                timeout: Duration::from_millis(3000),
            },
            secondary: ResolverConfig {
                addr: "8.8.8.8:53".parse().unwrap(),
                timeout: Duration::from_millis(3000),
            },
            // The knob under test, not a placeholder — this is the same `--qps` flag `main.rs`
            // exposes, deliberately set low here rather than the plan's ~11.5 production default
            // so this smoke test stays a few seconds long instead of racing real infrastructure.
            qps: 5.0,
            concurrency: 4,
            canary_every: 1000, // fewer domains than this, so exactly one canary check runs, at the end
            ttl_seconds: 90 * 24 * 60 * 60,
            quarantine_seconds: 180 * 24 * 60 * 60,
        };

        let canary = CanaryConfig::new(
            vec!["cloudflare.com".to_string(), "wikipedia.org".to_string()],
            vec!["holy-blocker-smoke-test-canary.invalid".to_string()],
        )
        .expect("non-empty control lists");

        let entries = vec![
            entry("mozilla.org"),
            entry("iana.org"),
            entry("holy-blocker-smoke-test-dead.invalid"),
        ];

        let start = std::time::Instant::now();
        let outcome = run_sweep(&entries, HashMap::new(), &canary, &config, 1_000_000)
            .await
            .expect("live sweep against real resolvers should complete without a canary failure");
        let elapsed = start.elapsed();

        assert_eq!(outcome.checked_count, entries.len());
        assert_eq!(outcome.cache.get("mozilla.org").map(|e| e.verdict), Some(Verdict::Alive));
        assert_eq!(outcome.cache.get("iana.org").map(|e| e.verdict), Some(Verdict::Alive));
        assert_eq!(
            outcome.cache.get("holy-blocker-smoke-test-dead.invalid").map(|e| e.verdict),
            Some(Verdict::Dead)
        );

        // The whole point of running this instead of the full pipeline: a handful of domains at
        // 5 qps should take a few seconds, not 24 hours. A blown budget here means something is
        // wrong with pacing/timeouts, not with the resolvers.
        assert!(
            elapsed < Duration::from_secs(30),
            "smoke sweep took {elapsed:?} — investigate before trusting this as a quick check"
        );
    }
}
