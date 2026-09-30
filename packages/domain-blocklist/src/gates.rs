//! Module 5 of the domain-blocklist plan — the publish gates: pure functions over a built
//! artifact and the previous published build's manifest that stand between a bad build and a
//! signed one. See `docs/components/domain-blocklist/plan.md`'s "gates" section and the decision
//! doc's "A measured false-positive gate on every build" / "cross-source corroboration" sections
//! for the false-positive design specifically.
//!
//! Every gate here only decides pass/fail; refusing publication on a `Fail` and requiring
//! explicit human sign-off to override it is the pipeline's job (module 7 `cli`, unbuilt), not
//! this module's. Built ahead of `sources`/`liveness`/`fst_build` (modules 1, 3, 4 — all still
//! unbuilt) per the plan's implementation order, which places this module third precisely because
//! it is cheap, pure, and is what stops every category of bad build — leaving it for last would
//! mean the first real run has no guardrail.

use std::collections::{BTreeSet, HashMap};

use domain_normalize::{RuleScope, normalize};

use crate::types::{LicenseId, MergedEntry, SourceId, SourceSnapshot};

/// A control-set/comparison result's failure message stops naming individual hits past this many
/// — an unbounded `Fail` message over a large control set would allocate megabytes in exactly the
/// path where an allocation surprise is least wanted.
const MAX_NAMED_HITS: usize = 20;

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

/// A threshold outside `[0, 1]` is meaningless, and a `NaN` threshold makes every `>` comparison
/// against it silently `false` — which would make the gate carrying it pass unconditionally on a
/// config mistake instead of refusing to run. Every threshold-taking gate below validates with
/// this before comparing anything against it.
fn valid_fraction(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

/// Whether a previous published build was available to compare against. Distinguishing this from
/// a bare `0` count or an empty key set is the point: a caller that failed to load the previous
/// manifest and a caller correctly reporting a genuine first build used to look identical to
/// `shrinkage_gate`/`growth_gate`, and both silently skipped the percentage check either way — an
/// adversarial review of this module found that a manifest-fetch failure and "nothing published
/// yet" must never be able to collapse onto the same input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviousBuild<T> {
    /// Nothing has ever been published — a genuine first build.
    None,
    /// The previous published build's count/key set, successfully loaded and verified.
    Existing(T),
}

/// Fails when `new_count` drops more than `max_drop_pct` below `prev_count`, or falls below
/// `absolute_floor`. Catches a bad liveness sweep the canary missed, a source that silently
/// emptied, and parser regressions.
///
/// `max_drop_pct` is a fraction in `[0, 1]` (the plan's starting value is `0.10` for 10%), not a
/// percent out of 100 — callers own that conversion so this function carries no implicit scale.
/// `PreviousBuild::None` (nothing published yet) skips the percentage check entirely, since there
/// is no prior build to shrink against; only `absolute_floor` can gate a first build. A caller
/// that couldn't load the previous manifest must not pass `None` to mean that — see
/// [`PreviousBuild`].
pub fn shrinkage_gate(
    prev_count: PreviousBuild<u64>,
    new_count: u64,
    max_drop_pct: f64,
    absolute_floor: u64,
) -> GateResult {
    if !valid_fraction(max_drop_pct) {
        return GateResult::Fail(format!(
            "max_drop_pct {max_drop_pct} is not a finite fraction in [0, 1]"
        ));
    }
    if new_count < absolute_floor {
        return GateResult::Fail(format!(
            "new entry count {new_count} is below the absolute floor of {absolute_floor}"
        ));
    }

    let prev_count = match prev_count {
        PreviousBuild::None => return GateResult::Pass,
        PreviousBuild::Existing(count) => count,
    };
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
/// `PreviousBuild::None` (nothing published yet) always passes: there is no prior build to grow
/// against, and refusing a first build for "100% growth" over nothing would be absurd. Same
/// `None`-means-first-build caution as [`shrinkage_gate`].
pub fn growth_gate(
    prev_keys: PreviousBuild<&BTreeSet<String>>,
    new_keys: &BTreeSet<String>,
    max_add_pct: f64,
) -> GateResult {
    if !valid_fraction(max_add_pct) {
        return GateResult::Fail(format!(
            "max_add_pct {max_add_pct} is not a finite fraction in [0, 1]"
        ));
    }

    let prev_keys = match prev_keys {
        PreviousBuild::None => return GateResult::Pass,
        PreviousBuild::Existing(keys) => keys,
    };
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

/// One control-set domain the merged blocklist blocks, and every source that independently
/// flagged it. This is the *full* hit list, before corroboration or manual review are applied —
/// see [`review_queue`] for the bounded set that actually needs a human, and
/// [`false_positive_gate`] for what's counted toward the published rate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalsePositiveHit {
    pub domain: String,
    pub scope: RuleScope,
    pub sources: Vec<SourceId>,
}

impl FalsePositiveHit {
    /// Two or more independently-maintained sources agreeing a domain belongs in a sensitive
    /// category is treated as corroboration the classification is correct — see the decision
    /// doc's cross-source-corroboration section. A single source's tag could be that source's own
    /// curation mistake (it happens at the file level: an entire opt-in "porn"/"NSFW" list or
    /// category directory is trusted wholesale, not verified per domain — see the plan's module 1
    /// input-shape notes); two separately-run projects making the identical mistake about the
    /// identical domain is far less likely, *unless* they share upstream provenance for that
    /// entry, which this heuristic cannot detect and does not claim to — an accepted limitation,
    /// not a guarantee of independence.
    ///
    /// The fixed count of 2 is deliberately not a fraction of `SourceId`'s current cardinality —
    /// at today's 3 sources it *is* effectively "most sources agree," but the two facts happen to
    /// coincide rather than one deriving from the other. If `SourceId` grows past roughly 5–10
    /// variants, revisit this as a percentage of the total source count instead: 2 agreeing out of
    /// 15 is much weaker corroboration than 2 out of 3, and a fixed count stops tracking that.
    pub fn is_corroborated(&self) -> bool {
        self.sources.len() >= 2
    }
}

/// Every `control_set` domain that the merged list blocks, with its supporting sources attached.
/// `control_set` is normalized the same way `merged`'s own keys already are (`merged` is always
/// `merge()`'s output, already `domain_normalize::normalize`'d) — an entry that fails to
/// normalize is dropped rather than compared, since it cannot sensibly equal a normalized key
/// either way. Skipping this normalization was a real, confirmed bug in an earlier version of
/// this function: an unnormalized control set (mixed case, a trailing dot, a U-label IDN) matched
/// nothing and reported a 0% false-positive rate against a list that was blocking 100% of it.
///
/// A control domain matches when it equals a merged entry's domain exactly. This works because
/// Tranco (the control set the plan specifies) ranks registrable, pay-level domains, so a control
/// entry and an `Apex`-scoped merged entry for the same site share the identical key — it is
/// **not** a general subdomain-covering lookup (that's `fst_build`/`net-shield`, modules 4/6,
/// neither built yet), and would stop being a safe approximation against a control set that
/// instead ranks hostnames.
///
/// Both `control_set` and `merged` are indexed/scanned once each — O(control + merged), not
/// O(control × merged). The earlier linear-scan version of this function was confirmed to cost
/// roughly 10¹² comparisons at realistic list sizes (a multi-million-entry merged list against a
/// million-entry control set).
pub fn false_positive_hits(merged: &[MergedEntry], control_set: &[&str]) -> Vec<FalsePositiveHit> {
    let by_domain: HashMap<&str, &MergedEntry> = merged
        .iter()
        .map(|entry| (entry.domain.as_str(), entry))
        .collect();

    control_set
        .iter()
        .filter_map(|raw| normalize(raw).ok())
        .filter_map(|domain| {
            by_domain
                .get(domain.as_str())
                .map(|entry| FalsePositiveHit {
                    domain,
                    scope: entry.scope,
                    sources: entry.sources.clone(),
                })
        })
        .collect()
}

/// The hits that actually need a human to look at: present in `merged`, backed by only **one**
/// source (not [corroborated](FalsePositiveHit::is_corroborated)), and not already covered by
/// `exclusions` — a small, reviewed, checked-in list of domains a human has confirmed are
/// legitimately sensitive despite lacking corroboration. `exclusions` is normalized the same way
/// `control_set` is, for the same reason (a hand-typed file is exactly where a stray case or
/// trailing-dot mismatch silently fails to match).
///
/// This is the bounded set the decision doc's corroboration mechanism exists to produce — not
/// "every adult site in the control set" (measurably ~250 at a Tranco top-10k control size, which
/// does not scale to manual review and is not what a false positive even means here), but
/// specifically the domains neither the pipeline's own cross-source agreement nor a past review
/// has already vouched for.
pub fn review_queue(
    merged: &[MergedEntry],
    control_set: &[&str],
    exclusions: &[&str],
) -> Vec<FalsePositiveHit> {
    let excluded: BTreeSet<String> = exclusions
        .iter()
        .filter_map(|raw| normalize(raw).ok())
        .collect();

    false_positive_hits(merged, control_set)
        .into_iter()
        .filter(|hit| !hit.is_corroborated() && !excluded.contains(&hit.domain))
        .collect()
}

/// Fails when the rate of [`review_queue`]'s output — hits neither corroborated by a second
/// independent source nor already reviewed — exceeds `max_fp_rate` against the *checked* control
/// set size (`control_set.len()` after normalization failures are dropped).
///
/// `max_fp_rate` is a fraction in `[0, 1]` (the plan's starting value is `0.005` for 0.5%), not a
/// percent out of 100, matching [`shrinkage_gate`]/[`growth_gate`]'s convention — and, per
/// [`valid_fraction`], a non-finite or out-of-range value fails the gate rather than silently
/// passing it.
///
/// `checked` below `min_control_size` fails outright. An empty or truncated control-set fetch (a
/// parse bug, a truncated download, a maintenance page instead of the real list) previously
/// looked identical to "measured, and clean" — this is what makes that distinguishable. There is
/// no default; the caller states what a trustworthy measurement requires.
pub fn false_positive_gate(
    merged: &[MergedEntry],
    control_set: &[&str],
    exclusions: &[&str],
    max_fp_rate: f64,
    min_control_size: usize,
) -> GateResult {
    if !valid_fraction(max_fp_rate) {
        return GateResult::Fail(format!(
            "max_fp_rate {max_fp_rate} is not a finite fraction in [0, 1]"
        ));
    }

    let checked = control_set
        .iter()
        .filter_map(|raw| normalize(raw).ok())
        .count();
    if checked < min_control_size {
        return GateResult::Fail(format!(
            "control set only had {checked} valid entries after normalization, below the required minimum of {min_control_size}"
        ));
    }

    let flagged = review_queue(merged, control_set, exclusions);
    let fp_rate = flagged.len() as f64 / checked as f64;
    if fp_rate > max_fp_rate {
        let mut named: Vec<String> = flagged
            .iter()
            .take(MAX_NAMED_HITS)
            .map(|hit| format!("{} (sources: {:?})", hit.domain, hit.sources))
            .collect();
        if flagged.len() > MAX_NAMED_HITS {
            named.push(format!("...and {} more", flagged.len() - MAX_NAMED_HITS));
        }
        GateResult::Fail(format!(
            "false-positive rate {:.4}% ({} of {checked} control domains, uncorroborated and unreviewed) exceeds the {:.4}% limit; hits: {}",
            fp_rate * 100.0,
            flagged.len(),
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

/// Fails when either (a) a source that contributed entries to `merged` has no corresponding
/// snapshot in `snapshots` at all, or (b) any snapshot present carries a license not on
/// `allowlist`. (a) is the cheaper mistake the license-only version of this gate used to miss
/// entirely: a *missing* snapshot passed trivially, even though the published artifact would
/// still carry entries from a source with no license record at all — module 1 enforces the same
/// check at fetch time (per the plan: "a source whose current license is not on the allowlist
/// fails the build"), and this is that check re-run here so it can never be bypassed or weakened
/// between fetch and publish.
///
/// Also fails when `snapshots` carries more than one entry for the same `SourceId` — two
/// snapshots for one source make license coverage ambiguous (which one governs?), and are always
/// either a build bug or two genuinely conflicting fetches worth surfacing either way.
///
/// License comparison is case-insensitive and trims whitespace, matching SPDX's own
/// identifier-comparison rule — see [`LicenseId::spdx_matches`]. An earlier exact-string version
/// of this check failed *closed* on a case/whitespace mismatch (correctly refusing to publish),
/// but produced a confusing failure that invited "just add the lowercase spelling to the
/// allowlist too," which would have quietly widened matching rather than fixing the comparison.
pub fn license_gate(
    merged: &[MergedEntry],
    snapshots: &[SourceSnapshot],
    allowlist: &[LicenseId],
) -> GateResult {
    let mut seen_sources: BTreeSet<SourceId> = BTreeSet::new();
    let mut duplicated: Vec<SourceId> = Vec::new();
    for snapshot in snapshots {
        if !seen_sources.insert(snapshot.source) {
            duplicated.push(snapshot.source);
        }
    }
    if !duplicated.is_empty() {
        return GateResult::Fail(format!(
            "{} source(s) have more than one snapshot, making license coverage ambiguous: {duplicated:?}",
            duplicated.len()
        ));
    }

    let contributing_sources: BTreeSet<SourceId> = merged
        .iter()
        .flat_map(|entry| entry.sources.iter().copied())
        .collect();
    let missing: Vec<SourceId> = contributing_sources
        .difference(&seen_sources)
        .copied()
        .collect();
    if !missing.is_empty() {
        return GateResult::Fail(format!(
            "{} source(s) contributed entries but have no license snapshot at all: {missing:?}",
            missing.len()
        ));
    }

    let disallowed: Vec<&SourceSnapshot> = snapshots
        .iter()
        .filter(|snapshot| {
            !allowlist
                .iter()
                .any(|allowed| allowed.spdx_matches(&snapshot.license))
        })
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
    use super::*;
    use crate::types::{Category, SourceId};

    fn merged(domain: &str, sources: Vec<SourceId>) -> MergedEntry {
        MergedEntry {
            domain: domain.to_string(),
            scope: RuleScope::Apex,
            sources,
            categories: vec![Category::Adult],
        }
    }

    mod shrinkage_gate_tests {
        use super::*;

        #[test]
        fn no_change_passes() {
            assert_eq!(
                shrinkage_gate(PreviousBuild::Existing(1000), 1000, 0.10, 0),
                GateResult::Pass
            );
        }

        #[test]
        fn growth_passes() {
            assert_eq!(
                shrinkage_gate(PreviousBuild::Existing(1000), 1500, 0.10, 0),
                GateResult::Pass
            );
        }

        #[test]
        fn a_drop_within_the_limit_passes() {
            assert_eq!(
                shrinkage_gate(PreviousBuild::Existing(1000), 950, 0.10, 0),
                GateResult::Pass
            );
        }

        #[test]
        fn a_drop_at_exactly_the_limit_passes() {
            assert_eq!(
                shrinkage_gate(PreviousBuild::Existing(1000), 900, 0.10, 0),
                GateResult::Pass
            );
        }

        #[test]
        fn a_drop_past_the_limit_fails() {
            assert!(matches!(
                shrinkage_gate(PreviousBuild::Existing(1000), 899, 0.10, 0),
                GateResult::Fail(_)
            ));
        }

        #[test]
        fn falling_below_the_absolute_floor_fails_even_with_no_prior_build() {
            assert!(matches!(
                shrinkage_gate(PreviousBuild::None, 5, 0.10, 10),
                GateResult::Fail(_)
            ));
        }

        #[test]
        fn a_first_build_above_the_floor_passes_with_no_percentage_check() {
            assert_eq!(
                shrinkage_gate(PreviousBuild::None, 5, 0.10, 0),
                GateResult::Pass
            );
        }

        #[test]
        fn the_absolute_floor_is_checked_even_when_the_percentage_would_pass() {
            assert!(matches!(
                shrinkage_gate(PreviousBuild::Existing(1000), 950, 0.10, 1000),
                GateResult::Fail(_)
            ));
        }

        #[test]
        fn a_nan_threshold_fails_instead_of_disabling_the_gate() {
            assert!(matches!(
                shrinkage_gate(PreviousBuild::Existing(1), 1, f64::NAN, 0),
                GateResult::Fail(_)
            ));
        }

        #[test]
        fn an_out_of_range_threshold_fails() {
            assert!(matches!(
                shrinkage_gate(PreviousBuild::Existing(1), 1, 1.5, 0),
                GateResult::Fail(_)
            ));
            assert!(matches!(
                shrinkage_gate(PreviousBuild::Existing(1), 1, -0.1, 0),
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
            assert_eq!(
                growth_gate(PreviousBuild::Existing(&prev), &prev, 0.10),
                GateResult::Pass
            );
        }

        #[test]
        fn shrinking_the_key_set_passes() {
            let prev = keys(&["a.com", "b.com"]);
            let new = keys(&["a.com"]);
            assert_eq!(
                growth_gate(PreviousBuild::Existing(&prev), &new, 0.10),
                GateResult::Pass
            );
        }

        #[test]
        fn growth_within_the_limit_passes() {
            let prev: BTreeSet<String> = (0..100).map(|i| format!("d{i}.com")).collect();
            let mut new = prev.clone();
            new.insert("new1.com".to_string());
            new.insert("new2.com".to_string());
            // 2 added / 100 previous = 2%, within a 10% limit.
            assert_eq!(
                growth_gate(PreviousBuild::Existing(&prev), &new, 0.10),
                GateResult::Pass
            );
        }

        #[test]
        fn growth_past_the_limit_fails() {
            let prev = keys(&["a.com", "b.com"]);
            let new = keys(&["a.com", "b.com", "c.com", "d.com"]);
            assert!(matches!(
                growth_gate(PreviousBuild::Existing(&prev), &new, 0.10),
                GateResult::Fail(_)
            ));
        }

        #[test]
        fn a_first_build_always_passes() {
            let new = keys(&["a.com", "b.com", "c.com"]);
            assert_eq!(
                growth_gate(PreviousBuild::None, &new, 0.10),
                GateResult::Pass
            );
        }

        #[test]
        fn an_empty_previous_key_set_always_passes() {
            let prev = BTreeSet::new();
            let new = keys(&["a.com", "b.com", "c.com"]);
            assert_eq!(
                growth_gate(PreviousBuild::Existing(&prev), &new, 0.10),
                GateResult::Pass
            );
        }

        #[test]
        fn replacing_one_key_with_another_of_the_same_size_counts_only_the_addition() {
            let prev = keys(&["a.com", "b.com"]);
            let new = keys(&["a.com", "c.com"]);
            assert!(matches!(
                growth_gate(PreviousBuild::Existing(&prev), &new, 0.10),
                GateResult::Fail(_)
            ));
            assert_eq!(
                growth_gate(PreviousBuild::Existing(&prev), &new, 0.60),
                GateResult::Pass
            );
        }

        #[test]
        fn a_nan_threshold_fails() {
            let prev = keys(&["a.com"]);
            let new = keys(&["a.com", "b.com"]);
            assert!(matches!(
                growth_gate(PreviousBuild::Existing(&prev), &new, f64::NAN),
                GateResult::Fail(_)
            ));
        }
    }

    mod false_positive_tests {
        use super::*;

        #[test]
        fn no_hits_against_the_control_set_passes() {
            let list = vec![merged("blocked.example", vec![SourceId::StevenBlack])];
            let control = vec!["google.com", "wikipedia.org"];
            assert_eq!(
                false_positive_gate(&list, &control, &[], 0.005, 0),
                GateResult::Pass
            );
            assert!(false_positive_hits(&list, &control).is_empty());
        }

        #[test]
        fn an_unnormalized_control_domain_still_matches_a_normalized_merged_entry() {
            // Confirmed real bug: before normalizing control_set, mixed case / a trailing dot /
            // an IDN U-label matched nothing, and a list blocking 100% of the control set scored
            // a 0% false-positive rate.
            let list = vec![merged("google.com", vec![SourceId::Ut1])];
            let control = vec!["GOOGLE.com."];
            let hits = false_positive_hits(&list, &control);
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].domain, "google.com");
        }

        #[test]
        fn a_single_source_hit_is_uncorroborated_and_lands_in_the_review_queue() {
            let list = vec![merged("one.example", vec![SourceId::StevenBlack])];
            let control = vec!["one.example"];
            let queue = review_queue(&list, &control, &[]);
            assert_eq!(queue.len(), 1);
            assert!(!queue[0].is_corroborated());
        }

        #[test]
        fn a_two_source_hit_is_corroborated_and_never_reaches_the_review_queue() {
            let list = vec![merged(
                "pornhub.example",
                vec![SourceId::StevenBlack, SourceId::Hagezi],
            )];
            let control = vec!["pornhub.example"];
            assert!(review_queue(&list, &control, &[]).is_empty());
            // Still visible in the full hit list, just not flagged for review.
            let hits = false_positive_hits(&list, &control);
            assert_eq!(hits.len(), 1);
            assert!(hits[0].is_corroborated());
        }

        #[test]
        fn corroboration_alone_clears_the_gate_with_no_exclusions_file_entry() {
            let list = vec![merged(
                "pornhub.example",
                vec![SourceId::StevenBlack, SourceId::Hagezi, SourceId::Ut1],
            )];
            let control = vec!["pornhub.example"];
            assert_eq!(
                false_positive_gate(&list, &control, &[], 0.0, 0),
                GateResult::Pass
            );
        }

        #[test]
        fn an_excluded_single_source_hit_is_never_a_review_candidate() {
            let list = vec![merged("legitimately-adult.example", vec![SourceId::Hagezi])];
            let control = vec!["legitimately-adult.example"];
            assert!(review_queue(&list, &control, &["legitimately-adult.example"]).is_empty());
        }

        #[test]
        fn exclusions_are_normalized_the_same_way_the_control_set_is() {
            let list = vec![merged("shared-host.example", vec![SourceId::Ut1])];
            let control = vec!["shared-host.example"];
            let queue = review_queue(&list, &control, &["Shared-Host.example."]);
            assert!(queue.is_empty());
        }

        #[test]
        fn a_rate_over_the_limit_fails() {
            let list = vec![merged("one.example", vec![SourceId::StevenBlack])];
            let mut control = vec!["one.example"];
            let rest: Vec<String> = (0..99).map(|i| format!("safe{i}.example")).collect();
            control.extend(rest.iter().map(String::as_str));
            // 1 uncorroborated hit / 100 checked = 1%, over a 0.5% limit.
            assert!(matches!(
                false_positive_gate(&list, &control, &[], 0.005, 0),
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
                false_positive_gate(&list, &control, &[], 0.005, 0),
                GateResult::Pass
            );
        }

        #[test]
        fn a_corroborated_hit_does_not_count_toward_the_rate_even_at_a_zero_threshold() {
            let list = vec![merged(
                "pornhub.example",
                vec![SourceId::StevenBlack, SourceId::Hagezi],
            )];
            let control = vec!["pornhub.example", "safe.example"];
            assert_eq!(
                false_positive_gate(&list, &control, &[], 0.0, 0),
                GateResult::Pass
            );
        }

        #[test]
        fn a_control_set_below_the_minimum_size_fails_regardless_of_rate() {
            let list: Vec<MergedEntry> = vec![];
            let control = vec!["a.example", "b.example"];
            assert!(matches!(
                false_positive_gate(&list, &control, &[], 0.005, 1000),
                GateResult::Fail(_)
            ));
        }

        #[test]
        fn an_empty_control_set_with_no_minimum_passes() {
            let list = vec![merged("one.example", vec![SourceId::StevenBlack])];
            assert_eq!(
                false_positive_gate(&list, &[], &[], 0.0, 0),
                GateResult::Pass
            );
        }

        #[test]
        fn a_nan_threshold_fails_instead_of_disabling_the_gate() {
            let list = vec![merged("one.example", vec![SourceId::StevenBlack])];
            let control = vec!["one.example"];
            assert!(matches!(
                false_positive_gate(&list, &control, &[], f64::NAN, 0),
                GateResult::Fail(_)
            ));
        }

        #[test]
        fn the_failure_message_names_the_hit_and_its_sources() {
            let list = vec![merged("one.example", vec![SourceId::StevenBlack])];
            let result = false_positive_gate(&list, &["one.example"], &[], 0.0, 0);
            match result {
                GateResult::Fail(message) => {
                    assert!(message.contains("one.example"));
                    assert!(message.contains("StevenBlack"));
                }
                GateResult::Pass => panic!("expected a failure"),
            }
        }

        #[test]
        fn the_failure_message_truncates_past_the_named_hit_cap() {
            let entries: Vec<MergedEntry> = (0..30)
                .map(|i| merged(&format!("hit{i}.example"), vec![SourceId::StevenBlack]))
                .collect();
            let control: Vec<String> = (0..30).map(|i| format!("hit{i}.example")).collect();
            let control_refs: Vec<&str> = control.iter().map(String::as_str).collect();
            let result = false_positive_gate(&entries, &control_refs, &[], 0.0, 0);
            match result {
                GateResult::Fail(message) => assert!(message.contains("and 10 more")),
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
        fn every_snapshot_on_the_allowlist_passes_with_no_contributing_sources() {
            let snapshots = vec![
                snapshot(SourceId::StevenBlack, "MIT"),
                snapshot(SourceId::Hagezi, "CC0-1.0"),
            ];
            let allowlist = vec![
                LicenseId("MIT".to_string()),
                LicenseId("CC0-1.0".to_string()),
            ];
            assert_eq!(license_gate(&[], &snapshots, &allowlist), GateResult::Pass);
        }

        #[test]
        fn a_snapshot_off_the_allowlist_fails() {
            let snapshots = vec![
                snapshot(SourceId::StevenBlack, "MIT"),
                snapshot(SourceId::Ut1, "GPL-3.0"),
            ];
            let allowlist = vec![LicenseId("MIT".to_string())];
            assert!(matches!(
                license_gate(&[], &snapshots, &allowlist),
                GateResult::Fail(_)
            ));
        }

        #[test]
        fn a_contributing_source_with_no_snapshot_at_all_fails() {
            // The bug the license-only version of this gate missed entirely: an omitted
            // snapshot, not a tampered one.
            let list = vec![merged("blocked.example", vec![SourceId::Ut1])];
            let snapshots: Vec<SourceSnapshot> = vec![];
            assert!(matches!(
                license_gate(&list, &snapshots, &[]),
                GateResult::Fail(_)
            ));
        }

        #[test]
        fn a_contributing_source_missing_among_several_snapshots_fails() {
            let list = vec![merged(
                "blocked.example",
                vec![SourceId::StevenBlack, SourceId::Ut1],
            )];
            let snapshots = vec![snapshot(SourceId::StevenBlack, "MIT")];
            let allowlist = vec![LicenseId("MIT".to_string())];
            match license_gate(&list, &snapshots, &allowlist) {
                GateResult::Fail(message) => assert!(message.contains("Ut1")),
                GateResult::Pass => panic!("expected a failure"),
            }
        }

        #[test]
        fn duplicate_snapshots_for_the_same_source_fail() {
            let snapshots = vec![
                snapshot(SourceId::StevenBlack, "MIT"),
                snapshot(SourceId::StevenBlack, "MIT"),
            ];
            let allowlist = vec![LicenseId("MIT".to_string())];
            assert!(matches!(
                license_gate(&[], &snapshots, &allowlist),
                GateResult::Fail(_)
            ));
        }

        #[test]
        fn license_comparison_is_case_and_whitespace_insensitive() {
            let list = vec![merged("blocked.example", vec![SourceId::StevenBlack])];
            let snapshots = vec![snapshot(SourceId::StevenBlack, " mit ")];
            let allowlist = vec![LicenseId("MIT".to_string())];
            assert_eq!(
                license_gate(&list, &snapshots, &allowlist),
                GateResult::Pass
            );
        }

        #[test]
        fn the_failure_message_names_the_source_and_its_license() {
            let snapshots = vec![snapshot(SourceId::Ut1, "GPL-3.0")];
            let result = license_gate(&[], &snapshots, &[]);
            match result {
                GateResult::Fail(message) => {
                    assert!(message.contains("Ut1"));
                    assert!(message.contains("GPL-3.0"));
                }
                GateResult::Pass => panic!("expected a failure"),
            }
        }

        #[test]
        fn an_empty_snapshot_list_with_no_contributing_sources_passes() {
            assert_eq!(license_gate(&[], &[], &[]), GateResult::Pass);
        }
    }
}
