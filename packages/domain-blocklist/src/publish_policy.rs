//! Pure policy decisions the pipeline binary needs before it can build and publish an artifact:
//! which `SourceSnapshot` provenance string to publish per source, and which license the whole
//! build inherits. Pulled out of `main.rs` so both are testable modules rather than functions
//! buried in `main` — per this repo's own "keep policy logic in testable modules" rule.

use anyhow::{Result, bail};

use domain_blocklist::{Category, LicenseId, SourceId, SourceSnapshot};

/// `license_gate` (module 5) is keyed one-`SourceSnapshot`-per-`SourceId` — correctly, since a
/// second entry for the same source is ambiguous license coverage in the general case. This
/// pipeline's own `SourceId::Ut1` is the one place that invariant needs help from the caller: UT1
/// is fetched as three separate per-category tarballs (adult/gambling/dating), each producing its
/// own snapshot, but they all carry the identical `SourceId::Ut1`. This collapses any such group
/// into one snapshot per `SourceId` — asserting every grouped snapshot agrees on license (a
/// genuine per-category license split for one source would be a real anomaly worth stopping the
/// build for, not silently picking one) and joining their individually-verified revisions. Each
/// revision is tagged with the `Category` it was fetched under (`Adult=W/"abc";Gambling=W/"def"`),
/// so the composite is self-describing and mechanically splittable on `;` then `=` — the bare
/// `;`-join it replaces lost which category each revision belonged to and was not splittable if any
/// single revision string itself contained `;`. The category is only needed to build that
/// self-describing string; it is not part of the published `SourceSnapshot` type.
pub fn collapse_snapshots_by_source(
    snapshots: Vec<(Category, SourceSnapshot)>,
) -> Result<Vec<SourceSnapshot>> {
    let mut by_source: std::collections::BTreeMap<SourceId, Vec<(Category, SourceSnapshot)>> =
        Default::default();
    for (category, snapshot) in snapshots {
        by_source.entry(snapshot.source).or_default().push((category, snapshot));
    }
    by_source
        .into_values()
        .map(|group| {
            let first = group.first().expect("BTreeMap group is never empty").clone();
            if group.iter().any(|(_, s)| !s.license.spdx_matches(&first.1.license)) {
                bail!(
                    "source {:?} reported different licenses across its per-category fetches: {:?} \
                     — refusing to silently pick one",
                    first.1.source,
                    group.iter().map(|(_, s)| &s.license).collect::<Vec<_>>()
                );
            }
            let revision = group
                .iter()
                .map(|(category, s)| format!("{category:?}={}", s.revision))
                .collect::<Vec<_>>()
                .join(";");
            // Deliberately the minimum across the group, which understates two of UT1's three
            // category fetch times — a fabricated-but-safe-direction value (staleness reads too
            // pessimistic, never too optimistic), kept simple rather than carrying three separate
            // timestamps into one manifest field.
            let fetched_at = group
                .iter()
                .map(|(_, s)| s.fetched_at)
                .min()
                .unwrap_or(first.1.fetched_at);
            Ok(SourceSnapshot {
                source: first.1.source,
                revision,
                license: first.1.license,
                fetched_at,
            })
        })
        .collect()
}

/// The least-permissive input license, per `fst_build::build`'s `output_license` contract. A small,
/// explicit rank table over this build's expected license set (most to least permissive); a
/// license not in the table maps to a distinct `UNKNOWN` sentinel rather than a guess, so an
/// unrecognized license shows up in the manifest rather than silently picking a rank for it.
///
/// The relative ordering below — especially CC-BY-SA-4.0 vs GPL-3.0 — is a starting point that has
/// **not been legally reviewed** and MUST be ratified by someone qualified before this manifest's
/// `output_license` field is treated as authoritative for redistribution decisions (`net-shield`
/// and any redistributor rely on it).
pub fn pick_output_license(snapshots: &[SourceSnapshot]) -> LicenseId {
    const RANK: &[&str] = &["CC0-1.0", "MIT", "CC-BY-SA-4.0", "GPL-3.0"];
    let worst = snapshots
        .iter()
        .map(|s| {
            RANK.iter()
                .position(|r| LicenseId(r.to_string()).spdx_matches(&s.license))
                .unwrap_or(usize::MAX)
        })
        .max();
    match worst {
        Some(idx) if idx < RANK.len() => LicenseId(RANK[idx].to_string()),
        _ => LicenseId("UNKNOWN".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(source: SourceId, revision: &str, license: &str, fetched_at: u64) -> SourceSnapshot {
        SourceSnapshot {
            source,
            revision: revision.to_string(),
            license: LicenseId(license.to_string()),
            fetched_at,
        }
    }

    #[test]
    fn collapse_single_source_tags_its_lone_revision_with_its_category() {
        let snapshots = vec![(Category::Adult, snapshot(SourceId::StevenBlack, "W/\"abc\"", "MIT", 100))];
        let collapsed = collapse_snapshots_by_source(snapshots).expect("collapse must succeed");
        assert_eq!(collapsed.len(), 1);
        assert_eq!(collapsed[0].revision, "Adult=W/\"abc\"");
    }

    #[test]
    fn collapse_ut1_group_joins_revisions_tagged_by_category() {
        let snapshots = vec![
            (Category::Adult, snapshot(SourceId::Ut1, "W/\"a\"", "CC-BY-SA-4.0", 200)),
            (Category::Gambling, snapshot(SourceId::Ut1, "W/\"g\"", "CC-BY-SA-4.0", 100)),
            (Category::Dating, snapshot(SourceId::Ut1, "W/\"d\"", "CC-BY-SA-4.0", 300)),
        ];
        let collapsed = collapse_snapshots_by_source(snapshots).expect("collapse must succeed");
        assert_eq!(collapsed.len(), 1);
        // Splittable on `;` then `=`, per the doc comment's contract.
        let mut parts: Vec<(&str, &str)> = collapsed[0]
            .revision
            .split(';')
            .map(|p| p.split_once('=').expect("each part must be `Category=revision`"))
            .collect();
        parts.sort();
        assert_eq!(
            parts,
            vec![("Adult", "W/\"a\""), ("Dating", "W/\"d\""), ("Gambling", "W/\"g\"")]
        );
    }

    #[test]
    fn collapse_ut1_group_takes_the_minimum_fetched_at() {
        let snapshots = vec![
            (Category::Adult, snapshot(SourceId::Ut1, "W/\"a\"", "CC-BY-SA-4.0", 200)),
            (Category::Gambling, snapshot(SourceId::Ut1, "W/\"g\"", "CC-BY-SA-4.0", 50)),
        ];
        let collapsed = collapse_snapshots_by_source(snapshots).expect("collapse must succeed");
        assert_eq!(collapsed[0].fetched_at, 50);
    }

    #[test]
    fn collapse_ut1_group_with_disagreeing_licenses_fails() {
        let snapshots = vec![
            (Category::Adult, snapshot(SourceId::Ut1, "W/\"a\"", "CC-BY-SA-4.0", 100)),
            (Category::Gambling, snapshot(SourceId::Ut1, "W/\"g\"", "MIT", 100)),
        ];
        assert!(collapse_snapshots_by_source(snapshots).is_err());
    }

    #[test]
    fn pick_output_license_picks_the_least_permissive_of_the_rank_table() {
        let snapshots = vec![
            snapshot(SourceId::StevenBlack, "r1", "MIT", 1),
            snapshot(SourceId::Hagezi, "r2", "GPL-3.0", 1),
        ];
        assert_eq!(pick_output_license(&snapshots), LicenseId("GPL-3.0".to_string()));
    }

    #[test]
    fn pick_output_license_unranked_license_maps_to_unknown() {
        let snapshots = vec![snapshot(SourceId::StevenBlack, "r1", "Some-Weird-License", 1)];
        assert_eq!(pick_output_license(&snapshots), LicenseId("UNKNOWN".to_string()));
    }

    #[test]
    fn pick_output_license_empty_snapshots_maps_to_unknown() {
        assert_eq!(pick_output_license(&[]), LicenseId("UNKNOWN".to_string()));
    }
}
