//! Module 2 of the domain-blocklist plan — normalize, scope, union, and category-filter
//! [`RawEntry`] values from every source into deduplicated [`MergedEntry`] values.
//!
//! Pure and I/O-free, per `docs/components/domain-blocklist/plan.md`: it consumes
//! `domain_normalize::normalize`/`classify_scope` (module 0) and produces the input to `gates`
//! (module 3) and `fst_build` (module 4), both still unbuilt.

use std::collections::BTreeMap;

use domain_normalize::{RuleScope, classify_scope, normalize};

use crate::types::{Category, MergedEntry, RawEntry, ScopeHint};

/// Resolves one entry's final [`RuleScope`], combining the parser's [`ScopeHint`] with
/// `domain_normalize::classify_scope`'s PSL-eligibility answer.
///
/// `normalized` must already be the output of [`domain_normalize::normalize`] — this function
/// does no normalization of its own, matching `classify_scope`'s own contract.
///
/// - `classify_scope` returning `None` (a public suffix, or on `shared_hosting_denylist`) refuses
///   the entry outright, regardless of `scope_hint`: a wildcard cannot conjure apex coverage the
///   PSL says isn't the tenant's, and a plain entry naming a public suffix is exactly the
///   shared-hosting black hole this design refuses to create.
/// - Only a wildcard (`scope_hint == Apex`) can ever produce `RuleScope::Apex`, and only when the
///   base it names is itself eTLD+1. A plain entry that happens to literally be a registrable
///   domain is still scoped `ExactHost` — a source that names `example.com` without a wildcard
///   only ever meant that one host, and widening it to cover every subdomain the source never
///   listed would be exactly the kind of silent over-block this plan's "no `www.`-stripping"
///   section warns about for the mirror case.
pub fn resolve_scope(
    normalized: &str,
    scope_hint: ScopeHint,
    shared_hosting_denylist: &[&str],
) -> Option<RuleScope> {
    match classify_scope(normalized, shared_hosting_denylist) {
        None => None,
        Some(RuleScope::Apex) if scope_hint == ScopeHint::Apex => Some(RuleScope::Apex),
        Some(_) => Some(RuleScope::ExactHost),
    }
}

/// Counts of entries `merge` dropped, one counter per reason — the plan requires every drop to be
/// counted and named rather than silently absorbed, so a parser or PSL regression shows up as one
/// counter exploding instead of a quietly shrinking list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergeReport {
    /// `domain_normalize::normalize` rejected the entry (malformed IDN, over-length after A-label
    /// expansion, ...). Module 1's parsers are expected to have already dropped most of these at
    /// fetch time; this counter catches anything that reached `merge` anyway.
    pub dropped_normalization_failed: u64,
    /// `classify_scope` returned `None` — the entry is itself a public suffix, or is on the
    /// shared-hosting denylist. This is the counter that would otherwise black-hole an entire
    /// hosting provider if it went unnoticed.
    pub dropped_public_suffix_or_denylisted: u64,
}

/// The result of a `merge` run: the deduplicated entries plus the drop counters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeOutput {
    pub entries: Vec<MergedEntry>,
    pub report: MergeReport,
}

/// Normalizes, scopes, and unions `raw_entries` from every source into one [`MergedEntry`] per
/// surviving normalized domain.
///
/// Two `RawEntry`s that normalize to the same key are merged: their `sources` and `categories`
/// sets are unioned (never overwritten — see the plan's "Don't blend categories silently"), and
/// the wider of their two scopes wins (`Apex` beats `ExactHost`, since one source naming the apex
/// is a stronger statement than another naming one host under it).
///
/// Output order is the sorted normalized-domain order — a side effect of using a `BTreeMap` to
/// dedupe, and convenient (not required) for `fst_build`'s own sorted-insertion need downstream.
pub fn merge(raw_entries: &[RawEntry], shared_hosting_denylist: &[&str]) -> MergeOutput {
    let mut by_domain: BTreeMap<String, MergedEntry> = BTreeMap::new();
    let mut report = MergeReport::default();

    for entry in raw_entries {
        let normalized = match normalize(&entry.domain) {
            Ok(normalized) => normalized,
            Err(_) => {
                report.dropped_normalization_failed += 1;
                continue;
            }
        };

        let scope = match resolve_scope(&normalized, entry.scope_hint, shared_hosting_denylist) {
            Some(scope) => scope,
            None => {
                report.dropped_public_suffix_or_denylisted += 1;
                continue;
            }
        };

        by_domain
            .entry(normalized.clone())
            .and_modify(|merged| {
                if !merged.sources.contains(&entry.source) {
                    merged.sources.push(entry.source);
                }
                if !merged.categories.contains(&entry.category) {
                    merged.categories.push(entry.category);
                }
                if scope == RuleScope::Apex {
                    merged.scope = RuleScope::Apex;
                }
            })
            .or_insert_with(|| MergedEntry {
                domain: normalized,
                scope,
                sources: vec![entry.source],
                categories: vec![entry.category],
            });
    }

    MergeOutput {
        entries: by_domain.into_values().collect(),
        report,
    }
}

/// Keeps only the entries carrying at least one category this build is configured to ship.
///
/// Per the plan: "an `adult`-only build still includes a domain that is also flagged `gambling`,
/// because the `adult` flag alone qualifies it" — this is an *any-of* filter over the whole
/// `categories` set, never a requirement that every category match.
pub fn filter_by_category(entries: &[MergedEntry], allowed: &[Category]) -> Vec<MergedEntry> {
    entries
        .iter()
        .filter(|entry| entry.categories.iter().any(|category| allowed.contains(category)))
        .cloned()
        .collect()
}

/// Personal-name triage: a pure, bounded heuristic that flags a *candidate* for the review queue
/// the plan's GDPR section requires, never a verdict change of its own.
///
/// Returns `true` when `second_level_label` decomposes into 2–4 alphabetic tokens separated by
/// `-`, `.`, or `_`, in any order. This is a triage filter, not a completeness guarantee: it
/// flags `red-panda` and misses `janedoe`, and per the plan both are the accepted, documented
/// behavior rather than bugs to fix.
pub fn flag_personal_name(second_level_label: &str) -> bool {
    let tokens: Vec<&str> = second_level_label
        .split(['-', '.', '_'])
        .filter(|token| !token.is_empty())
        .collect();

    (2..=4).contains(&tokens.len())
        && tokens
            .iter()
            .all(|token| !token.is_empty() && token.chars().all(|c| c.is_ascii_alphabetic()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(domain: &str, source: crate::types::SourceId, category: Category, hint: ScopeHint) -> RawEntry {
        RawEntry {
            domain: domain.to_string(),
            source,
            category,
            scope_hint: hint,
        }
    }

    mod resolve_scope_tests {
        use super::*;

        #[test]
        fn wildcard_base_that_is_registrable_becomes_apex() {
            assert_eq!(
                resolve_scope("example.com", ScopeHint::Apex, &[]),
                Some(RuleScope::Apex)
            );
        }

        #[test]
        fn wildcard_base_that_is_not_registrable_is_refused_widening() {
            // *.foo.example.com's extracted base is foo.example.com, which sits below eTLD+1 —
            // the wildcard cannot conjure apex coverage the PSL says isn't the tenant's.
            assert_eq!(
                resolve_scope("foo.example.com", ScopeHint::Apex, &[]),
                Some(RuleScope::ExactHost)
            );
        }

        #[test]
        fn plain_entry_that_is_registrable_is_still_exact_host_only() {
            // No wildcard means the source only ever named this one host, even though it happens
            // to be eTLD+1 — widening without an explicit wildcard is exactly the over-block this
            // design avoids for the mirror www. case.
            assert_eq!(
                resolve_scope("example.com", ScopeHint::None, &[]),
                Some(RuleScope::ExactHost)
            );
        }

        #[test]
        fn plain_subdomain_entry_is_exact_host() {
            assert_eq!(
                resolve_scope("www.example.com", ScopeHint::None, &[]),
                Some(RuleScope::ExactHost)
            );
        }

        #[test]
        fn public_suffix_is_refused_regardless_of_hint() {
            assert_eq!(resolve_scope("co.uk", ScopeHint::None, &[]), None);
            assert_eq!(resolve_scope("co.uk", ScopeHint::Apex, &[]), None);
        }

        #[test]
        fn shared_hosting_denylist_is_refused_regardless_of_hint() {
            assert_eq!(
                resolve_scope("example.com", ScopeHint::None, &["example.com"]),
                None
            );
            assert_eq!(
                resolve_scope("example.com", ScopeHint::Apex, &["example.com"]),
                None
            );
        }
    }

    mod merge_tests {
        use super::*;
        use crate::types::SourceId;

        #[test]
        fn single_entry_survives_as_is() {
            let out = merge(
                &[entry("example.com", SourceId::StevenBlack, Category::Adult, ScopeHint::Apex)],
                &[],
            );
            assert_eq!(out.entries.len(), 1);
            let merged = &out.entries[0];
            assert_eq!(merged.domain, "example.com");
            assert_eq!(merged.scope, RuleScope::Apex);
            assert_eq!(merged.sources, vec![SourceId::StevenBlack]);
            assert_eq!(merged.categories, vec![Category::Adult]);
            assert_eq!(out.report, MergeReport::default());
        }

        #[test]
        fn two_sources_for_the_same_domain_union_their_sources() {
            let out = merge(
                &[
                    entry("example.com", SourceId::StevenBlack, Category::Adult, ScopeHint::Apex),
                    entry("example.com", SourceId::Hagezi, Category::Adult, ScopeHint::Apex),
                ],
                &[],
            );
            assert_eq!(out.entries.len(), 1);
            assert_eq!(
                out.entries[0].sources,
                vec![SourceId::StevenBlack, SourceId::Hagezi]
            );
        }

        #[test]
        fn a_domain_flagged_under_two_categories_by_two_sources_keeps_both() {
            // Named test case from the plan: a domain classified as both `adult` and `gambling`
            // by two different sources must survive merge with both categories intact.
            let out = merge(
                &[
                    entry("example.com", SourceId::StevenBlack, Category::Adult, ScopeHint::Apex),
                    entry("example.com", SourceId::Ut1, Category::Gambling, ScopeHint::Apex),
                ],
                &[],
            );
            assert_eq!(out.entries.len(), 1);
            assert_eq!(
                out.entries[0].categories,
                vec![Category::Adult, Category::Gambling]
            );
        }

        #[test]
        fn merging_apex_and_exact_host_for_the_same_domain_takes_the_wider_scope() {
            // One source names the bare apex without a wildcard (ExactHost); another lists it as
            // a wildcard (Apex). The union is Apex — a stronger statement wins.
            let out = merge(
                &[
                    entry("example.com", SourceId::StevenBlack, Category::Adult, ScopeHint::None),
                    entry("example.com", SourceId::Hagezi, Category::Adult, ScopeHint::Apex),
                ],
                &[],
            );
            assert_eq!(out.entries.len(), 1);
            assert_eq!(out.entries[0].scope, RuleScope::Apex);
        }

        #[test]
        fn www_host_and_apex_stay_distinct_entries() {
            // The plan's central www. example: a bare Apex entry for example.com and a separate
            // ExactHost entry for www.example.com are two keys, never aliased onto one.
            let out = merge(
                &[
                    entry("example.com", SourceId::StevenBlack, Category::Adult, ScopeHint::Apex),
                    entry("www.example.com", SourceId::StevenBlack, Category::Adult, ScopeHint::None),
                ],
                &[],
            );
            assert_eq!(out.entries.len(), 2);
            let apex = out.entries.iter().find(|e| e.domain == "example.com").unwrap();
            let host = out.entries.iter().find(|e| e.domain == "www.example.com").unwrap();
            assert_eq!(apex.scope, RuleScope::Apex);
            assert_eq!(host.scope, RuleScope::ExactHost);
        }

        #[test]
        fn a_public_suffix_entry_is_dropped_and_counted() {
            let out = merge(
                &[entry("co.uk", SourceId::Ut1, Category::Adult, ScopeHint::None)],
                &[],
            );
            assert!(out.entries.is_empty());
            assert_eq!(out.report.dropped_public_suffix_or_denylisted, 1);
        }

        #[test]
        fn a_denylisted_shared_hosting_entry_is_dropped_and_counted() {
            let out = merge(
                &[entry(
                    "shared-host.example",
                    SourceId::Ut1,
                    Category::Adult,
                    ScopeHint::Apex,
                )],
                &["shared-host.example"],
            );
            assert!(out.entries.is_empty());
            assert_eq!(out.report.dropped_public_suffix_or_denylisted, 1);
        }

        #[test]
        fn an_unnormalizable_entry_is_dropped_and_counted_separately() {
            // A label over 63 bytes fails domain_normalize::normalize's post-conversion length
            // check (RFC 1035 §2.3.4), independent of the public-suffix/denylist counter.
            let over_long_label = "a".repeat(64);
            let out = merge(
                &[entry(
                    &format!("{over_long_label}.com"),
                    SourceId::Ut1,
                    Category::Adult,
                    ScopeHint::None,
                )],
                &[],
            );
            assert!(out.entries.is_empty());
            assert_eq!(out.report.dropped_normalization_failed, 1);
            assert_eq!(out.report.dropped_public_suffix_or_denylisted, 0);
        }

        #[test]
        fn output_is_empty_for_empty_input() {
            let out = merge(&[], &[]);
            assert_eq!(out, MergeOutput::default());
        }
    }

    mod filter_by_category_tests {
        use super::*;
        use crate::types::SourceId;

        fn merged(domain: &str, categories: Vec<Category>) -> MergedEntry {
            MergedEntry {
                domain: domain.to_string(),
                scope: RuleScope::Apex,
                sources: vec![SourceId::StevenBlack],
                categories,
            }
        }

        #[test]
        fn any_matching_category_qualifies_the_entry() {
            let entries = vec![merged("example.com", vec![Category::Adult, Category::Gambling])];
            let out = filter_by_category(&entries, &[Category::Adult]);
            assert_eq!(out.len(), 1);
        }

        #[test]
        fn no_matching_category_excludes_the_entry() {
            let entries = vec![merged("example.com", vec![Category::Dating])];
            let out = filter_by_category(&entries, &[Category::Adult, Category::Gambling]);
            assert!(out.is_empty());
        }

        #[test]
        fn empty_allowed_set_excludes_everything() {
            let entries = vec![merged("example.com", vec![Category::Adult])];
            let out = filter_by_category(&entries, &[]);
            assert!(out.is_empty());
        }
    }

    mod flag_personal_name_tests {
        use super::*;

        #[test]
        fn two_hyphenated_tokens_are_flagged() {
            // Named example from the plan: red-panda.com is expected to be flagged.
            assert!(flag_personal_name("red-panda"));
        }

        #[test]
        fn a_single_run_together_token_is_not_flagged() {
            // Named example from the plan: janedoe.com is expected to be missed — this is the
            // accepted false negative, not a bug.
            assert!(!flag_personal_name("janedoe"));
        }

        #[test]
        fn three_and_four_dot_separated_tokens_are_flagged() {
            assert!(flag_personal_name("john.q.public"));
            assert!(flag_personal_name("mary.jane.smith.jones"));
        }

        #[test]
        fn five_tokens_is_too_many_to_flag() {
            assert!(!flag_personal_name("a.b.c.d.e"));
        }

        #[test]
        fn a_token_with_digits_is_not_flagged() {
            assert!(!flag_personal_name("panda99-red"));
        }

        #[test]
        fn underscore_separated_tokens_are_flagged() {
            assert!(flag_personal_name("jane_doe"));
        }

        #[test]
        fn mixed_separators_are_flagged() {
            assert!(flag_personal_name("jane-doe.smith"));
        }

        #[test]
        fn empty_label_is_not_flagged() {
            assert!(!flag_personal_name(""));
        }
    }
}
