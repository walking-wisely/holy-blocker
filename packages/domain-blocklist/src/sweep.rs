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
    /// Checkpoints only fire at a canary-validated chunk boundary (see the loop in
    /// `run_sweep_with_resolvers`), so this rounds up to the next multiple of `canary_every`.
    /// `0` disables checkpointing.
    pub checkpoint_every: usize,
}

/// Persistence seam for mid-sweep checkpoints, so this module doesn't do file I/O directly.
/// A failure aborts the sweep rather than logging and continuing — silently degrading to
/// "no longer actually checkpointing" would defeat the point of adding this.
pub trait CheckpointSink: Send {
    fn checkpoint(
        &mut self,
        cache: &HashMap<String, CacheEntry>,
    ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;
}

#[cfg_attr(not(test), allow(dead_code))]
pub struct NoCheckpoint;

impl CheckpointSink for NoCheckpoint {
    async fn checkpoint(&mut self, _cache: &HashMap<String, CacheEntry>) -> anyhow::Result<()> {
        Ok(())
    }
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
    InitialCanaryFailed {
        resolver: &'static str,
        detail: String,
    },
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
    #[error("checkpoint failed after {checked_before_failure} domains: {detail}")]
    CheckpointFailed {
        checked_before_failure: usize,
        detail: String,
    },
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
            failures.push(format!(
                "alive control {domain:?} expected Alive, got {v:?}"
            ));
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
    checkpoint: &mut impl CheckpointSink,
) -> Result<SweepOutcome, SweepError> {
    let primary = Arc::new(HickoryDnsLookup::new(config.primary)?);
    let secondary = Arc::new(HickoryDnsLookup::new(config.secondary)?);
    run_sweep_with_resolvers(
        entries, cache, canary, config, now, primary, secondary, checkpoint,
    )
    .await
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
    checkpoint: &mut impl CheckpointSink,
) -> Result<SweepOutcome, SweepError> {
    if let Err((resolver, detail)) = canary_pass_both(&*primary, &*secondary, canary).await {
        return Err(SweepError::InitialCanaryFailed { resolver, detail });
    }

    // Indices, not cloned domain strings: holding a second full-corpus `Vec<String>` alongside
    // `entries` for the length of the sweep roughly doubles the resident set at multi-million-entry
    // scale. Only one chunk's worth of domains is ever cloned at a time, below.
    let due_indices: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| due_for_check(cache.get(&e.domain), now, config.ttl_seconds))
        .map(|(i, _)| i)
        .collect();

    let mut checked = 0usize;
    let mut since_checkpoint = 0usize;
    let canary_every = config.canary_every.max(1);

    for chunk in due_indices.chunks(canary_every) {
        let interval = Duration::from_secs_f64(1.0 / config.qps.max(0.001));
        let start = tokio::time::Instant::now();
        let results: Vec<(String, Verdict)> = stream::iter(chunk.iter().copied().enumerate())
            .map(|(pos, idx)| {
                let domain = entries[idx].domain.clone();
                let primary = Arc::clone(&primary);
                let secondary = Arc::clone(&secondary);
                async move {
                    let deadline = start + interval * (pos as u32);
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
            since_checkpoint += 1;
        }

        if let Err((resolver, detail)) = canary_pass_both(&*primary, &*secondary, canary).await {
            return Err(SweepError::MidSweepCanaryFailed {
                resolver,
                checked_before_failure: checked,
                detail,
            });
        }

        if config.checkpoint_every > 0 && since_checkpoint >= config.checkpoint_every {
            checkpoint
                .checkpoint(&cache)
                .await
                .map_err(|e| SweepError::CheckpointFailed {
                    checked_before_failure: checked,
                    detail: e.to_string(),
                })?;
            since_checkpoint = 0;
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
            self.answers
                .lock()
                .unwrap()
                .insert(domain.to_string(), result);
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
                LookupResult::NxDomain {
                    authenticated: false,
                    extended_error: None,
                },
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

    #[derive(Default)]
    struct FakeCheckpointSink {
        calls: Vec<usize>,
        fail_on_call: Option<usize>,
    }

    impl CheckpointSink for FakeCheckpointSink {
        async fn checkpoint(&mut self, cache: &HashMap<String, CacheEntry>) -> anyhow::Result<()> {
            self.calls.push(cache.len());
            if self.fail_on_call == Some(self.calls.len()) {
                anyhow::bail!("simulated checkpoint failure");
            }
            Ok(())
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
            checkpoint_every: 0,
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
        CanaryConfig::new(
            vec!["alive.invalid".to_string()],
            vec!["dead.invalid".to_string()],
        )
        .expect("non-empty control lists")
    }

    #[tokio::test]
    async fn initial_canary_failure_returns_initial_canary_failed_and_no_cache() {
        // The canary's own alive control resolves Dead — a resolver that can't even pass its own
        // sanity check must never reach the sweep loop at all.
        let resolver = Arc::new(
            FakeResolver::new()
                .dead("alive.invalid")
                .dead("dead.invalid"),
        );
        let result = run_sweep_with_resolvers(
            &[entry("example.com")],
            HashMap::new(),
            &canary(),
            &config(),
            1_000_000,
            Arc::clone(&resolver),
            Arc::clone(&resolver),
            &mut NoCheckpoint,
        )
        .await;

        assert!(matches!(
            result,
            Err(SweepError::InitialCanaryFailed { .. })
        ));
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
                return LookupResult::NxDomain {
                    authenticated: false,
                    extended_error: None,
                };
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
            due_domain_answer: LookupResult::NxDomain {
                authenticated: false,
                extended_error: None,
            },
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
            &mut NoCheckpoint,
        )
        .await;

        match result {
            Err(SweepError::MidSweepCanaryFailed {
                checked_before_failure,
                ..
            }) => {
                assert_eq!(
                    checked_before_failure, 1,
                    "the one due domain was checked before the mid-sweep canary tripped"
                );
            }
            other => panic!("expected MidSweepCanaryFailed, got {other:?}"),
        }
        // The plan's "a partial sweep is not salvaged" rule: an Err carries no SweepOutcome at
        // all, so there is no cache update and no prune list to inspect — the type itself is the
        // guarantee, this assertion just makes that explicit for the reader.
    }

    #[tokio::test]
    async fn checkpoint_fires_at_the_next_canary_chunk_boundary_after_the_threshold() {
        let resolver = Arc::new(
            FakeResolver::new()
                .alive("alive.invalid")
                .dead("dead.invalid")
                .alive("d1.example")
                .alive("d2.example")
                .alive("d3.example")
                .alive("d4.example")
                .alive("d5.example")
                .alive("d6.example"),
        );
        let entries: Vec<MergedEntry> = [
            "d1.example",
            "d2.example",
            "d3.example",
            "d4.example",
            "d5.example",
            "d6.example",
        ]
        .iter()
        .map(|d| entry(d))
        .collect();

        let mut cfg = config();
        cfg.canary_every = 2;
        cfg.checkpoint_every = 4;
        let mut sink = FakeCheckpointSink::default();

        let outcome = run_sweep_with_resolvers(
            &entries,
            HashMap::new(),
            &canary(),
            &cfg,
            1_000_000,
            Arc::clone(&resolver),
            Arc::clone(&resolver),
            &mut sink,
        )
        .await
        .expect("all-alive resolver never fails the canary");

        assert_eq!(outcome.checked_count, 6);
        // 3 chunks of 2; the threshold of 4 is only crossed after the 2nd chunk, so exactly one
        // checkpoint fires, holding the 4 domains checked by then.
        assert_eq!(sink.calls, vec![4]);
    }

    #[tokio::test]
    async fn a_checkpoint_failure_aborts_the_sweep() {
        let resolver = Arc::new(
            FakeResolver::new()
                .alive("alive.invalid")
                .dead("dead.invalid")
                .alive("d1.example")
                .alive("d2.example"),
        );
        let entries = vec![entry("d1.example"), entry("d2.example")];

        let mut cfg = config();
        cfg.canary_every = 2;
        cfg.checkpoint_every = 1;
        let mut sink = FakeCheckpointSink {
            fail_on_call: Some(1),
            ..Default::default()
        };

        let result = run_sweep_with_resolvers(
            &entries,
            HashMap::new(),
            &canary(),
            &cfg,
            1_000_000,
            Arc::clone(&resolver),
            Arc::clone(&resolver),
            &mut sink,
        )
        .await;

        assert!(matches!(
            result,
            Err(SweepError::CheckpointFailed {
                checked_before_failure: 2,
                ..
            })
        ));
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
        let resolver = Arc::new(
            FakeResolver::new()
                .alive("alive.invalid")
                .dead("dead.invalid"),
        );

        let outcome = run_sweep_with_resolvers(
            &[entry("recently-dead.dropped-domain.net")],
            cache,
            &canary(),
            &cfg,
            now,
            Arc::clone(&resolver),
            Arc::clone(&resolver),
            &mut NoCheckpoint,
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
        let resolver = Arc::new(
            FakeResolver::new()
                .alive("alive.invalid")
                .dead("dead.invalid"),
        );

        let outcome = run_sweep_with_resolvers(
            &[entry("long-dead.dropped-domain.net")],
            cache,
            &canary(),
            &cfg,
            now,
            Arc::clone(&resolver),
            Arc::clone(&resolver),
            &mut NoCheckpoint,
        )
        .await
        .expect("no domain is due, so no lookup happens and no canary runs");

        assert_eq!(
            outcome.pruned_domains,
            ["long-dead.dropped-domain.net".to_string()].into()
        );
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
            checkpoint_every: 0,
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
        let outcome = run_sweep(
            &entries,
            HashMap::new(),
            &canary,
            &config,
            1_000_000,
            &mut NoCheckpoint,
        )
        .await
        .expect("live sweep against real resolvers should complete without a canary failure");
        let elapsed = start.elapsed();

        assert_eq!(outcome.checked_count, entries.len());
        assert_eq!(
            outcome.cache.get("mozilla.org").map(|e| e.verdict),
            Some(Verdict::Alive)
        );
        assert_eq!(
            outcome.cache.get("iana.org").map(|e| e.verdict),
            Some(Verdict::Alive)
        );
        assert_eq!(
            outcome
                .cache
                .get("holy-blocker-smoke-test-dead.invalid")
                .map(|e| e.verdict),
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

    /// Answers deterministically from the domain string itself (no stored per-domain table), so
    /// this resolver costs no memory of its own regardless of corpus size — the point of the stress
    /// test below is to isolate RAM growth to `entries`/`cache`, not incidentally add a second
    /// multi-million-entry map to account for.
    struct DeterministicResolver;

    impl AsyncDnsLookup for DeterministicResolver {
        async fn lookup_async(&self, domain: &str, _record: RecordType) -> LookupResult {
            if domain == "alive.invalid" {
                return LookupResult::Resolved(vec!["203.0.113.1".parse().unwrap()]);
            }
            if domain == "dead.invalid" {
                return LookupResult::NxDomain {
                    authenticated: false,
                    extended_error: None,
                };
            }
            let hash = domain
                .bytes()
                .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
            if hash % 100 == 0 {
                LookupResult::NxDomain {
                    authenticated: false,
                    extended_error: None,
                }
            } else {
                LookupResult::Resolved(vec!["203.0.113.1".parse().unwrap()])
            }
        }
    }

    fn current_rss_mb() -> f64 {
        let pid = std::process::id().to_string();
        let output = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &pid])
            .output()
            .expect("`ps` should be available on macOS/Linux");
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<f64>()
            .unwrap_or(0.0)
            / 1024.0
    }

    struct MeasuringCheckpointSink {
        path: std::path::PathBuf,
        started: std::time::Instant,
        checkpoints: usize,
    }

    impl CheckpointSink for MeasuringCheckpointSink {
        async fn checkpoint(&mut self, cache: &HashMap<String, CacheEntry>) -> anyhow::Result<()> {
            self.checkpoints += 1;
            crate::cache_store::save(&self.path, cache)?;
            let file_bytes = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
            println!(
                "[stress] checkpoint #{:>3}  t={:>7.1}s  cached={:>9}  rss={:>7.1} MB  cache_file={:>7.1} MB",
                self.checkpoints,
                self.started.elapsed().as_secs_f64(),
                cache.len(),
                current_rss_mb(),
                file_bytes as f64 / (1024.0 * 1024.0),
            );
            Ok(())
        }
    }

    /// No real network, no real ~4.75M-domain corpus fetch — this stands in for both with
    /// [`DeterministicResolver`] and a synthetic `entries` vec, so it runs in seconds against
    /// exactly the code path the real ~14h sweep runs, and reports RSS at every checkpoint. Answers
    /// the question the change in this commit was made for: does the checkpointed cache actually
    /// stay bounded, and does dropping the `due_domains` full-corpus clone actually show up in RSS.
    #[tokio::test]
    #[ignore = "long-running, memory-focused — run explicitly with `cargo test --bin domain-blocklist \
                --features cli,net --release -- --ignored --nocapture stress_test_measures_memory_at_corpus_scale`"]
    async fn stress_test_measures_memory_at_corpus_scale() {
        const N: usize = 4_800_000; // matches the measured real merged-corpus size

        println!(
            "[stress] rss before building entries: {:.1} MB",
            current_rss_mb()
        );
        let entries: Vec<MergedEntry> = (0..N)
            .map(|i| entry(&format!("domain-{i}.example")))
            .collect();
        println!(
            "[stress] rss after building {N} entries: {:.1} MB",
            current_rss_mb()
        );

        let dir = tempfile::tempdir().unwrap();
        let mut cfg = config();
        cfg.qps = 2_000_000.0;
        cfg.concurrency = 1000;
        cfg.canary_every = 100_000;
        cfg.checkpoint_every = 400_000;

        let resolver = Arc::new(DeterministicResolver);
        let mut sink = MeasuringCheckpointSink {
            path: dir.path().join("cache.bin"),
            started: std::time::Instant::now(),
            checkpoints: 0,
        };

        let outcome = run_sweep_with_resolvers(
            &entries,
            HashMap::new(),
            &canary(),
            &cfg,
            1_000_000,
            Arc::clone(&resolver),
            Arc::clone(&resolver),
            &mut sink,
        )
        .await
        .expect("DeterministicResolver's canary controls always answer correctly");

        println!(
            "[stress] done: checked={} pruned={} rss_final={:.1} MB checkpoints={}",
            outcome.checked_count,
            outcome.pruned_domains.len(),
            current_rss_mb(),
            sink.checkpoints,
        );
        assert_eq!(outcome.checked_count, N);
    }

    /// Runs the same corpus through two sweeps sharing one cache file — the second constructed the
    /// way a restart after a crash would be, by `cache_store::load`ing what the first checkpointed —
    /// and asserts the second does zero DNS lookups. This is what "resume is free" actually means:
    /// not a special resume code path, just `due_for_check`'s ordinary TTL logic seeing every domain
    /// as already checked.
    #[tokio::test]
    #[ignore = "long-running — run explicitly with `cargo test --bin domain-blocklist --features cli,net \
                --release -- --ignored --nocapture a_resumed_sweep_after_checkpointing_does_no_redundant_lookups`"]
    async fn a_resumed_sweep_after_checkpointing_does_no_redundant_lookups() {
        const N: usize = 500_000;
        let entries: Vec<MergedEntry> = (0..N)
            .map(|i| entry(&format!("domain-{i}.example")))
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let cache_path = dir.path().join("cache.bin");
        let resolver = Arc::new(DeterministicResolver);
        let mut cfg = config();
        cfg.qps = 2_000_000.0;
        cfg.concurrency = 1000;
        cfg.canary_every = 100_000;
        cfg.checkpoint_every = 100_000;
        let now = 1_000_000;

        let mut sink = MeasuringCheckpointSink {
            path: cache_path.clone(),
            started: std::time::Instant::now(),
            checkpoints: 0,
        };
        let first = run_sweep_with_resolvers(
            &entries,
            HashMap::new(),
            &canary(),
            &cfg,
            now,
            Arc::clone(&resolver),
            Arc::clone(&resolver),
            &mut sink,
        )
        .await
        .expect("first sweep should complete cleanly");
        assert_eq!(first.checked_count, N);

        // Simulates a restart: load exactly what the first run's last checkpoint persisted, and
        // sweep the same corpus again at the same `now` — nothing should be due yet.
        let resumed_cache = crate::cache_store::load(&cache_path).unwrap();
        let mut resume_sink = FakeCheckpointSink::default();
        let second = run_sweep_with_resolvers(
            &entries,
            resumed_cache,
            &canary(),
            &cfg,
            now,
            Arc::clone(&resolver),
            Arc::clone(&resolver),
            &mut resume_sink,
        )
        .await
        .expect("resumed sweep should complete cleanly with nothing due");

        assert_eq!(second.checked_count, 0);
    }
}
