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
    CacheEntry, CanaryConfig, MergedEntry, RecordType, Timestamp, UnknownReason, Verdict,
    combine as combine_verdict, corroborate, due_for_check, should_prune_with_hysteresis,
};
use futures::stream::{self, StreamExt};

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
async fn check_async(resolver: &HickoryDnsLookup, domain: &str) -> Verdict {
    let a = resolver.lookup_async(domain, RecordType::A).await;
    if matches!(a, domain_blocklist::LookupResult::Resolved(_)) {
        return Verdict::Alive;
    }
    let aaaa = resolver.lookup_async(domain, RecordType::Aaaa).await;
    combine_verdict(a, aaaa)
}

/// Async re-implementation of [`domain_blocklist::check_corroborated`] over two
/// [`HickoryDnsLookup`]s, run concurrently.
async fn check_corroborated_async(
    primary: &HickoryDnsLookup,
    secondary: &HickoryDnsLookup,
    domain: &str,
) -> Verdict {
    let (v1, v2) = tokio::join!(check_async(primary, domain), check_async(secondary, domain));
    corroborate(v1, v2)
}

/// Async re-implementation of [`domain_blocklist::canary_check`]'s pass/fail evaluation against one
/// resolver — same alive/dead-control equality rule, no policy divergence, just async-native.
async fn canary_pass_async(resolver: &HickoryDnsLookup, canary: &CanaryConfig) -> Result<(), String> {
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

async fn canary_pass_both(
    primary: &HickoryDnsLookup,
    secondary: &HickoryDnsLookup,
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
    mut cache: HashMap<String, CacheEntry>,
    canary: &CanaryConfig,
    config: &SweepConfig,
    now: Timestamp,
) -> Result<SweepOutcome, SweepError> {
    let primary = Arc::new(HickoryDnsLookup::new(config.primary)?);
    let secondary = Arc::new(HickoryDnsLookup::new(config.secondary)?);

    if let Err((resolver, detail)) = canary_pass_both(&primary, &secondary, canary).await {
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
                    let verdict = check_corroborated_async(&primary, &secondary, &domain).await;
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

        if let Err((resolver, detail)) = canary_pass_both(&primary, &secondary, canary).await {
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
