//! Module 5 of the domain-blocklist plan — the publish gates: pure functions over a built
//! artifact and the previous published build's manifest that stand between a bad build and a
//! signed one. See `docs/components/domain-blocklist/plan.md`'s "gates" section.
//!
//! Every gate here only decides pass/fail; refusing publication on a `Fail` and requiring
//! explicit human sign-off to override it is the pipeline's job (module 7 `cli`, unbuilt), not
//! this module's. Built ahead of `sources`/`liveness`/`fst_build` (modules 1, 3, 4 — all still
//! unbuilt) per the plan's implementation order, which places this module third precisely because
//! it is cheap, pure, and is what stops every category of bad build — leaving it for last would
//! mean the first real run has no guardrail.

use std::collections::BTreeSet;

use crate::types::{LicenseId, MergedEntry, SourceSnapshot};

/// The pass/fail decision from a single gate. `Fail` carries the reason as data, not just a
/// boolean, so a refused build names the evidence a human needs to review rather than requiring
/// them to re-derive it from raw counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateResult {
    Pass,
    Fail(String),
}

impl GateResult {
    pub fn is_pass(&self) -> bool {
        matches!(self, GateResult::Pass)
    }
}

/// Fails when `new_count` drops more than `max_drop_pct` below `prev_count`, or falls below
/// `absolute_floor`. Catches a bad liveness sweep the canary missed, a source that silently
/// emptied, and parser regressions.
///
/// `max_drop_pct` is a fraction in `[0, 1]` (the plan's starting value is `0.10` for 10%), not a
/// percent out of 100 — callers own that conversion so this function carries no implicit scale.
/// A `prev_count` of zero (nothing published yet) skips the percentage check entirely, since
/// there is no prior build to shrink against; only `absolute_floor` can gate a first build.
pub fn shrinkage_gate(
    prev_count: u64,
    new_count: u64,
    max_drop_pct: f64,
    absolute_floor: u64,
) -> GateResult {
    if new_count < absolute_floor {
        return GateResult::Fail(format!(
            "new entry count {new_count} is below the absolute floor of {absolute_floor}"
        ));
    }
    if prev_count == 0 || new_count >= prev_count {
        return GateResult::Pass;
    }

    let dropped = prev_count - new_count;
    let drop_pct = dropped as f64 / prev_count as f64;
    if drop_pct > max_drop_pct {
        GateResult::Fail(format!(
            "entry count dropped {:.2}% ({dropped} of {prev_count}), exceeding the {:.2}% limit",
            drop_pct * 100.0,
            max_drop_pct * 100.0
        ))
    } else {
        GateResult::Pass
    }
}

/// Fails when the entries present in `new_keys` but absent from `prev_keys` exceed `max_add_pct`
/// of `prev_keys`'s size. Catches a compromised or mis-bumped upstream injecting bulk entries —
/// the mirror image of [`shrinkage_gate`], and deliberately a separate gate since the two catch
/// opposite failures.
///
/// A `prev_keys` of zero (nothing published yet) always passes: there is no prior build to grow
/// against, and refusing a first build for having "100% growth" over nothing would be absurd.
pub fn growth_gate(
    prev_keys: &BTreeSet<String>,
    new_keys: &BTreeSet<String>,
    max_add_pct: f64,
) -> GateResult {
    if prev_keys.is_empty() {
        return GateResult::Pass;
    }

    let added = new_keys.difference(prev_keys).count();
    let add_pct = added as f64 / prev_keys.len() as f64;
    if add_pct > max_add_pct {
        GateResult::Fail(format!(
            "{added} new entries added, {:.2}% of the previous {} entries, exceeding the {:.2}% limit",
            add_pct * 100.0,
            prev_keys.len(),
            max_add_pct * 100.0
        ))
    } else {
        GateResult::Pass
    }
}

/// One control-set domain the merged list would incorrectly block, and the source(s) responsible
/// — carried separately from [`GateResult`]'s message so a caller can log it as build metrics
/// (the plan requires a regression to name the source that caused it, not just report a bare
/// rate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalsePositiveHit {
    pub domain: String,
    pub sources: Vec<crate::types::SourceId>,
}

/// Evaluates `merged` against `control_set` (e.g. a pinned Tranco top-N release) minus
/// `exclusions` (a reviewed, checked-in file of control domains that are legitimately adult) and
/// returns every control domain the merged list would block.
///
/// A control domain matches when it is present in `merged` under an identical normalized domain
/// string. This is a **direct-membership** check, not the subdomain-covering lookup an `Apex`
/// scope gets at query time (`fst_build`/`net-shield`, modules 4/6, both downstream of this pure
/// gate and neither built yet) — appropriate to what this module has to work with before the FST
/// exists, and a control set of top-level sites makes the distinction unlikely to matter in
/// practice.
pub fn false_positive_hits(
    merged: &[MergedEntry],
    control_set: &[&str],
    exclusions: &[&str],
) -> Vec<FalsePositiveHit> {
    let excluded: BTreeSet<&str> = exclusions.iter().copied().collect();
    control_set
        .iter()
        .filter(|domain| !excluded.contains(*domain))
        .filter_map(|domain| {
            merged
                .iter()
                .find(|entry| entry.domain == *domain)
                .map(|entry| FalsePositiveHit {
                    domain: domain.to_string(),
                    sources: entry.sources.clone(),
                })
        })
        .collect()
}

/// Fails when the false-positive rate against `control_set` — hits from [`false_positive_hits`]
/// divided by `control_set.len()` after `exclusions` are removed — exceeds `max_fp_rate`.
///
/// `max_fp_rate` is a fraction in `[0, 1]` (the plan's starting value is `0.005` for 0.5%), not a
/// percent out of 100, matching [`shrinkage_gate`]/[`growth_gate`]'s convention. An empty
/// `control_set` (after exclusions) always passes — there is nothing to measure a rate against.
pub fn false_positive_gate(
    merged: &[MergedEntry],
    control_set: &[&str],
    exclusions: &[&str],
    max_fp_rate: f64,
) -> GateResult {
    let excluded: BTreeSet<&str> = exclusions.iter().copied().collect();
    let checked = control_set
        .iter()
        .filter(|domain| !excluded.contains(*domain))
        .count();
    if checked == 0 {
        return GateResult::Pass;
    }

    let hits = false_positive_hits(merged, control_set, exclusions);
    let fp_rate = hits.len() as f64 / checked as f64;
    if fp_rate > max_fp_rate {
        let named: Vec<String> = hits
            .iter()
            .map(|hit| format!("{} (sources: {:?})", hit.domain, hit.sources))
            .collect();
        GateResult::Fail(format!(
            "false-positive rate {:.4}% ({} of {checked} control domains) exceeds the {:.4}% limit; hits: {}",
            fp_rate * 100.0,
            hits.len(),
            max_fp_rate * 100.0,
            named.join(", ")
        ))
    } else {
        GateResult::Pass
    }
}

/// Fails when `artifact_bytes` exceeds `ceiling`. The plan's starting ceiling is 32 MiB for the
/// `.fst` file, taking `image-sandbox`'s 15 MB model budget as precedent for stating a ceiling at
/// all rather than trusting the merged entry count to bound the artifact's size.
pub fn size_gate(artifact_bytes: u64, ceiling: u64) -> GateResult {
    if artifact_bytes > ceiling {
        GateResult::Fail(format!(
            "artifact is {artifact_bytes} bytes, exceeding the {ceiling}-byte ceiling"
        ))
    } else {
        GateResult::Pass
    }
}

/// Fails when any `snapshots` entry's license is not in `allowlist`. Module 1 enforces this at
/// fetch time already (per the plan: "a source whose current license is not on the allowlist
/// fails the build"); this is the same check re-run here so the published artifact can never
/// carry a snapshot the allowlist doesn't cover, even if module 1's own check is ever bypassed or
/// weakened between fetch and publish.
pub fn license_gate(snapshots: &[SourceSnapshot], allowlist: &[LicenseId]) -> GateResult {
    let disallowed: Vec<&SourceSnapshot> = snapshots
        .iter()
        .filter(|snapshot| !allowlist.contains(&snapshot.license))
        .collect();
    if disallowed.is_empty() {
        return GateResult::Pass;
    }

    let named: Vec<String> = disallowed
        .iter()
        .map(|snapshot| format!("{:?} ({:?})", snapshot.source, snapshot.license))
        .collect();
    GateResult::Fail(format!(
        "{} source(s) carry a license not on the allowlist: {}",
        disallowed.len(),
        named.join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use domain_normalize::RuleScope;

    use super::*;
    use crate::types::SourceId;

    fn merged(domain: &str, sources: Vec<SourceId>) -> MergedEntry {
        MergedEntry {
            domain: domain.to_string(),
            scope: RuleScope::Apex,
            sources,
            categories: vec![crate::types::Category::Adult],
        }
    }

    mod shrinkage_gate_tests {
        use super::*;

        #[test]
        fn no_change_passes() {
            assert_eq!(shrinkage_gate(1000, 1000, 0.10, 0), GateResult::Pass);
        }

        #[test]
        fn growth_passes() {
            assert_eq!(shrinkage_gate(1000, 1500, 0.10, 0), GateResult::Pass);
        }

        #[test]
        fn a_drop_within_the_limit_passes() {
            assert_eq!(shrinkage_gate(1000, 950, 0.10, 0), GateResult::Pass);
        }

        #[test]
        fn a_drop_at_exactly_the_limit_passes() {
            assert_eq!(shrinkage_gate(1000, 900, 0.10, 0), GateResult::Pass);
        }

        #[test]
        fn a_drop_past_the_limit_fails() {
            assert!(matches!(
                shrinkage_gate(1000, 899, 0.10, 0),
                GateResult::Fail(_)
            ));
        }

        #[test]
        fn falling_below_the_absolute_floor_fails_even_with_no_prior_build() {
            assert!(matches!(
                shrinkage_gate(0, 5, 0.10, 10),
                GateResult::Fail(_)
            ));
        }

        #[test]
        fn a_first_build_above_the_floor_passes_with_no_percentage_check() {
            // prev_count == 0: there is nothing to compute a drop percentage against.
            assert_eq!(shrinkage_gate(0, 5, 0.10, 0), GateResult::Pass);
        }

        #[test]
        fn the_absolute_floor_is_checked_even_when_the_percentage_would_pass() {
            // A 5% drop is within a 10% limit, but the absolute floor still applies.
            assert!(matches!(
                shrinkage_gate(1000, 950, 0.10, 1000),
                GateResult::Fail(_)
            ));
        }
    }

    mod growth_gate_tests {
        use super::*;

        fn keys(domains: &[&str]) -> BTreeSet<String> {
            domains.iter().map(|d| d.to_string()).collect()
        }

        #[test]
        fn no_new_keys_passes() {
            let prev = keys(&["a.com", "b.com"]);
            assert_eq!(growth_gate(&prev, &prev, 0.10), GateResult::Pass);
        }

        #[test]
        fn shrinking_the_key_set_passes() {
            let prev = keys(&["a.com", "b.com"]);
            let new = keys(&["a.com"]);
            assert_eq!(growth_gate(&prev, &new, 0.10), GateResult::Pass);
        }

        #[test]
        fn growth_within_the_limit_passes() {
            let prev: BTreeSet<String> = (0..100).map(|i| format!("d{i}.com")).collect();
            let mut new = prev.clone();
            new.insert("new1.com".to_string());
            new.insert("new2.com".to_string());
            // 2 added / 100 previous = 2%, within a 10% limit.
            assert_eq!(growth_gate(&prev, &new, 0.10), GateResult::Pass);
        }

        #[test]
        fn growth_past_the_limit_fails() {
            let prev = keys(&["a.com", "b.com"]);
            let new = keys(&["a.com", "b.com", "c.com", "d.com"]);
            // 2 added / 2 previous = 100%, well past a 10% limit.
            assert!(matches!(
                growth_gate(&prev, &new, 0.10),
                GateResult::Fail(_)
            ));
        }

        #[test]
        fn an_empty_previous_key_set_always_passes() {
            let prev = BTreeSet::new();
            let new = keys(&["a.com", "b.com", "c.com"]);
            assert_eq!(growth_gate(&prev, &new, 0.10), GateResult::Pass);
        }

        #[test]
        fn replacing_one_key_with_another_of_the_same_size_counts_only_the_addition() {
            let prev = keys(&["a.com", "b.com"]);
            let new = keys(&["a.com", "c.com"]);
            // 1 added / 2 previous = 50%.
            assert!(matches!(
                growth_gate(&prev, &new, 0.10),
                GateResult::Fail(_)
            ));
            assert_eq!(growth_gate(&prev, &new, 0.60), GateResult::Pass);
        }
    }

    mod false_positive_tests {
        use super::*;

        #[test]
        fn no_hits_against_the_control_set_passes() {
            let list = vec![merged("blocked.example", vec![SourceId::StevenBlack])];
            let control = vec!["google.com", "wikipedia.org"];
            assert_eq!(
                false_positive_gate(&list, &control, &[], 0.005),
                GateResult::Pass
            );
            assert!(false_positive_hits(&list, &control, &[]).is_empty());
        }

        #[test]
        fn a_control_domain_present_in_the_merged_list_is_a_hit() {
            let list = vec![merged("google.com", vec![SourceId::Ut1])];
            let control = vec!["google.com", "wikipedia.org"];
            let hits = false_positive_hits(&list, &control, &[]);
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].domain, "google.com");
            assert_eq!(hits[0].sources, vec![SourceId::Ut1]);
        }

        #[test]
        fn an_excluded_control_domain_is_never_a_hit() {
            let list = vec![merged("legitimately-adult.example", vec![SourceId::Hagezi])];
            let control = vec!["legitimately-adult.example"];
            let hits = false_positive_hits(&list, &control, &["legitimately-adult.example"]);
            assert!(hits.is_empty());
        }

        #[test]
        fn a_rate_over_the_limit_fails() {
            let list = vec![merged("one.example", vec![SourceId::StevenBlack])];
            // 1 hit / 100 checked = 1%, over a 0.5% limit.
            let mut control = vec!["one.example"];
            let rest: Vec<String> = (0..99).map(|i| format!("safe{i}.example")).collect();
            control.extend(rest.iter().map(String::as_str));
            assert!(matches!(
                false_positive_gate(&list, &control, &[], 0.005),
                GateResult::Fail(_)
            ));
        }

        #[test]
        fn a_rate_at_exactly_the_limit_passes() {
            let list = vec![merged("one.example", vec![SourceId::StevenBlack])];
            let mut control = vec!["one.example"];
            let rest: Vec<String> = (0..199).map(|i| format!("safe{i}.example")).collect();
            control.extend(rest.iter().map(String::as_str));
            // 1 / 200 = 0.5%.
            assert_eq!(
                false_positive_gate(&list, &control, &[], 0.005),
                GateResult::Pass
            );
        }

        #[test]
        fn an_empty_control_set_after_exclusions_passes() {
            let list = vec![merged("one.example", vec![SourceId::StevenBlack])];
            assert_eq!(
                false_positive_gate(&list, &["one.example"], &["one.example"], 0.0),
                GateResult::Pass
            );
        }

        #[test]
        fn the_failure_message_names_the_hit_and_its_sources() {
            let list = vec![merged(
                "one.example",
                vec![SourceId::StevenBlack, SourceId::Ut1],
            )];
            let result = false_positive_gate(&list, &["one.example"], &[], 0.0);
            match result {
                GateResult::Fail(message) => {
                    assert!(message.contains("one.example"));
                    assert!(message.contains("StevenBlack"));
                }
                GateResult::Pass => panic!("expected a failure"),
            }
        }
    }

    mod size_gate_tests {
        use super::*;

        #[test]
        fn under_the_ceiling_passes() {
            assert_eq!(size_gate(10_000_000, 32_000_000), GateResult::Pass);
        }

        #[test]
        fn exactly_the_ceiling_passes() {
            assert_eq!(size_gate(32_000_000, 32_000_000), GateResult::Pass);
        }

        #[test]
        fn over_the_ceiling_fails() {
            assert!(matches!(
                size_gate(32_000_001, 32_000_000),
                GateResult::Fail(_)
            ));
        }
    }

    mod license_gate_tests {
        use super::*;

        fn snapshot(source: SourceId, license: &str) -> SourceSnapshot {
            SourceSnapshot {
                source,
                revision: "v1".to_string(),
                license: LicenseId(license.to_string()),
                fetched_at: 0,
            }
        }

        #[test]
        fn every_snapshot_on_the_allowlist_passes() {
            let snapshots = vec![
                snapshot(SourceId::StevenBlack, "MIT"),
                snapshot(SourceId::Hagezi, "CC0-1.0"),
            ];
            let allowlist = vec![
                LicenseId("MIT".to_string()),
                LicenseId("CC0-1.0".to_string()),
            ];
            assert_eq!(license_gate(&snapshots, &allowlist), GateResult::Pass);
        }

        #[test]
        fn a_snapshot_off_the_allowlist_fails() {
            let snapshots = vec![
                snapshot(SourceId::StevenBlack, "MIT"),
                snapshot(SourceId::Ut1, "GPL-3.0"),
            ];
            let allowlist = vec![LicenseId("MIT".to_string())];
            assert!(matches!(
                license_gate(&snapshots, &allowlist),
                GateResult::Fail(_)
            ));
        }

        #[test]
        fn the_failure_message_names_the_source_and_its_license() {
            let snapshots = vec![snapshot(SourceId::Ut1, "GPL-3.0")];
            let result = license_gate(&snapshots, &[]);
            match result {
                GateResult::Fail(message) => {
                    assert!(message.contains("Ut1"));
                    assert!(message.contains("GPL-3.0"));
                }
                GateResult::Pass => panic!("expected a failure"),
            }
        }

        #[test]
        fn an_empty_snapshot_list_passes() {
            assert_eq!(license_gate(&[], &[]), GateResult::Pass);
        }
    }
}
