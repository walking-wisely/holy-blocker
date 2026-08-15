//! Groups a liveness sweep's cache by *why* the sweep didn't trust a domain — every `Dead`
//! verdict, and every `Unknown` verdict split by its exact [`UnknownReason`] — so a human can
//! review the actual domains behind an aggregate count, rather than only the count itself. This
//! exists because `cli`'s own `run_liveness` previously logged only `checked`/`pruned` totals: a
//! 10% `Unknown` rate at qps-scale (see `docs/decisions/domain-blocklist-sourcing.md`'s "Measured
//! 2026-08-15/16" section) is a number worth knowing, but *which* domains and *which reason* is
//! what turns it into something actionable — a `Timeout`/`UncorroboratedDead` spike is a resolver-
//! load signal to watch, while a persistent `NoData` on an apex-scoped entry is the benign,
//! expected case module 3's own `UnknownReason::NoData` doc comment already names.
//!
//! `Alive` entries are deliberately excluded — this answers "what didn't the sweep trust, and
//! why", not "dump the whole cache".

use std::collections::{BTreeMap, HashMap};

use super::{CacheEntry, Verdict};

/// Every domain a sweep placed in a negative (non-`Alive`) category, grouped for review.
/// `unknown_by_reason` is keyed by the reason's `Debug` label (`UnknownReason` derives neither
/// `Hash` nor `Ord`, and a string label is exactly what a human-reviewed report wants regardless)
/// rather than the enum itself, and both the key set and every domain list are produced in sorted
/// order so the report is reproducible across runs over an unordered `HashMap` cache.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NegativeOutcomeReport {
    pub dead: Vec<String>,
    pub unknown_by_reason: BTreeMap<String, Vec<String>>,
}

impl NegativeOutcomeReport {
    /// Total domains across every negative category — `dead.len()` plus every reason bucket's
    /// length. Never counts a domain twice: a cache entry has exactly one `Verdict`.
    pub fn total(&self) -> usize {
        self.dead.len() + self.unknown_by_reason.values().map(Vec::len).sum::<usize>()
    }
}

/// Builds a [`NegativeOutcomeReport`] from a sweep's finished cache. Pure — takes the already-
/// materialized `HashMap<String, CacheEntry>` a real sweep (`sweep::run_sweep`) or a fixture
/// produces, so this is testable with no DNS client and no `net` feature.
pub fn negative_outcome_report(cache: &HashMap<String, CacheEntry>) -> NegativeOutcomeReport {
    let mut report = NegativeOutcomeReport::default();
    for (domain, entry) in cache {
        match entry.verdict {
            Verdict::Alive => {}
            Verdict::Dead => report.dead.push(domain.clone()),
            Verdict::Unknown(reason) => {
                report
                    .unknown_by_reason
                    .entry(format!("{reason:?}"))
                    .or_default()
                    .push(domain.clone());
            }
        }
    }
    report.dead.sort();
    for domains in report.unknown_by_reason.values_mut() {
        domains.sort();
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::lookup::UnknownReason;

    fn entry(verdict: Verdict) -> CacheEntry {
        CacheEntry { last_checked: 0, verdict, first_dead_at: None }
    }

    #[test]
    fn alive_entries_are_excluded() {
        let cache = HashMap::from([("alive.example".to_string(), entry(Verdict::Alive))]);
        let report = negative_outcome_report(&cache);
        assert_eq!(report, NegativeOutcomeReport::default());
        assert_eq!(report.total(), 0);
    }

    #[test]
    fn dead_entries_are_collected_and_sorted() {
        let cache = HashMap::from([
            ("z-dead.example".to_string(), entry(Verdict::Dead)),
            ("a-dead.example".to_string(), entry(Verdict::Dead)),
            ("alive.example".to_string(), entry(Verdict::Alive)),
        ]);
        let report = negative_outcome_report(&cache);
        assert_eq!(report.dead, vec!["a-dead.example", "z-dead.example"]);
        assert!(report.unknown_by_reason.is_empty());
        assert_eq!(report.total(), 2);
    }

    #[test]
    fn unknown_entries_are_grouped_by_exact_reason() {
        let cache = HashMap::from([
            ("timeout1.example".to_string(), entry(Verdict::Unknown(UnknownReason::Timeout))),
            ("timeout2.example".to_string(), entry(Verdict::Unknown(UnknownReason::Timeout))),
            ("servfail.example".to_string(), entry(Verdict::Unknown(UnknownReason::ServFail))),
            (
                "uncorroborated.example".to_string(),
                entry(Verdict::Unknown(UnknownReason::UncorroboratedDead)),
            ),
        ]);
        let report = negative_outcome_report(&cache);
        assert!(report.dead.is_empty());
        assert_eq!(
            report.unknown_by_reason.get("Timeout").cloned().unwrap_or_default(),
            vec!["timeout1.example", "timeout2.example"]
        );
        assert_eq!(
            report.unknown_by_reason.get("ServFail").cloned().unwrap_or_default(),
            vec!["servfail.example"]
        );
        assert_eq!(
            report.unknown_by_reason.get("UncorroboratedDead").cloned().unwrap_or_default(),
            vec!["uncorroborated.example"]
        );
        assert_eq!(report.total(), 4);
    }

    #[test]
    fn a_domain_appears_in_exactly_one_bucket() {
        // Reproduces this module's own "never counts a domain twice" doc claim on `total()`: one
        // domain, one verdict, one bucket — not silently duplicated across dead/unknown.
        let cache = HashMap::from([("dead.example".to_string(), entry(Verdict::Dead))]);
        let report = negative_outcome_report(&cache);
        assert_eq!(report.dead.len(), 1);
        assert_eq!(report.unknown_by_reason.values().map(Vec::len).sum::<usize>(), 0);
    }

    #[test]
    fn empty_cache_produces_an_empty_report() {
        let report = negative_outcome_report(&HashMap::new());
        assert_eq!(report, NegativeOutcomeReport::default());
        assert_eq!(report.total(), 0);
    }
}
