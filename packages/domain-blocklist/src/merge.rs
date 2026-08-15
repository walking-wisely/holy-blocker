//! Module 2 of the domain-blocklist plan — normalize, scope, union, and category-filter
//! [`RawEntry`] values from every source into deduplicated [`MergedEntry`] values.
//!
//! Pure and I/O-free, per `docs/components/domain-blocklist/plan.md`: it consumes
//! `domain_normalize::normalize`/`classify_scope` (module 0) and produces the input to `gates`
//! (module 5, done — see `gates.rs`) and `fst_build` (module 4, still unbuilt).

use std::collections::BTreeMap;
use std::net::IpAddr;

use domain_normalize::{RuleScope, classify_scope, normalize};

use crate::types::{Category, MergedEntry, RawEntry, ScopeHint};

/// Resolves one entry's final [`RuleScope`], combining the parser's [`ScopeHint`] with
/// `domain_normalize::classify_scope`'s PSL-eligibility answer.
///
/// `normalized` must already be the output of [`domain_normalize::normalize`] — this function
/// does no normalization of its own, matching `classify_scope`'s own contract.
///
/// - `classify_scope` returning `None` refuses the entry outright, regardless of `scope_hint` —
///   this now only happens for a `shared_hosting_denylist` match, a deliberate operator decision
///   to never make a rule targeting that exact domain at all.
/// - Only a wildcard (`scope_hint == Apex`) can ever produce `RuleScope::Apex`, and only when the
///   base it names is itself eTLD+1. A plain entry that happens to literally be a registrable
///   domain is still scoped `ExactHost` — a source that names `example.com` without a wildcard
///   only ever meant that one host, and widening it to cover every subdomain the source never
///   listed would be exactly the kind of silent over-block this plan's "no `www.`-stripping"
///   section warns about for the mirror case. The same downgrade-not-drop rule now applies when
///   `normalized` is itself a public suffix (`classify_scope` still refuses `Apex` there, but
///   returns `Some(ExactHost)` rather than `None` — see its own doc comment for why claiming the
///   literal suffix string is safe even though claiming everything under it would not be).
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
    /// `classify_scope` returned `None` — the normalized entry matches the caller-supplied
    /// `shared_hosting_denylist` exactly. Being itself a public suffix no longer lands here (see
    /// `classify_scope`'s own doc comment: that case downgrades to `ExactHost` now, since claiming
    /// the literal suffix string is safe even though claiming everything under it would not be) —
    /// this counter now only moves on a deliberate operator "never make a rule targeting this
    /// domain" decision, so a nonzero count is worth checking against that list, not the PSL.
    pub dropped_shared_hosting_denylisted: u64,
    /// The normalized domain parses as an IPv4 or IPv6 literal. The plan's module 1 input-shape
    /// table assigns dropping these to the source parsers ("IP-based blocking is out of scope for
    /// a domain-keyed artifact"), but that module doesn't exist yet — this is defense in depth so
    /// a mis-split hosts-file line (or any future parser bug) can never land an IP string as a key
    /// in a domain-keyed FST.
    pub dropped_ip_literal: u64,
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
/// `sources` and `categories` within each entry are sorted too, independent of the order
/// `raw_entries` arrived in — both are logical sets (see the plan's "categories is a set, not a
/// single value"), and an input-order-dependent `Vec` would make the eventual signed artifact
/// non-reproducible across a source fetch order CI makes no promises about.
///
/// `shared_hosting_denylist` is normalized once here, up front — `classify_scope`'s contract
/// assumes its `normalized` parameter is already `normalize()`'s output, and the denylist is a
/// hand-authored, checked-in file exactly where a stray trailing dot, capitalization, or a
/// Unicode-typed IDN would otherwise silently fail to match and reopen the shared-hosting hole
/// this parameter exists to close.
pub fn merge(raw_entries: &[RawEntry], shared_hosting_denylist: &[&str]) -> MergeOutput {
    let normalized_denylist: Vec<String> = shared_hosting_denylist
        .iter()
        .filter_map(|raw| normalize(raw).ok())
        .collect();
    let denylist_refs: Vec<&str> = normalized_denylist.iter().map(String::as_str).collect();

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

        if normalized.parse::<IpAddr>().is_ok() {
            // DNS labels are digit-legal (RFC 1035 §2.3.1), so an IP literal like "0.0.0.0"
            // survives normalize() as itself rather than erroring — it must be caught here.
            report.dropped_ip_literal += 1;
            continue;
        }

        let scope = match resolve_scope(&normalized, entry.scope_hint, &denylist_refs) {
            Some(scope) => scope,
            None => {
                report.dropped_shared_hosting_denylisted += 1;
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

    for merged in by_domain.values_mut() {
        merged.sources.sort();
        merged.categories.sort();
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
        .filter(|entry| {
            entry
                .categories
                .iter()
                .any(|category| allowed.contains(category))
        })
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
///
/// A label carrying the `xn--` ACE prefix (RFC 3492 §5) — what `normalize()` produces for any
/// non-ASCII label — is never flagged. Its hyphen-delimited fragments are punycode encoding
/// artifacts, not human-readable name parts, and treating them as tokens would systematically
/// false-positive across the entire IDN corpus (`xn--mller-kva`, i.e. `müller`, splits into the
/// three all-alphabetic tokens `xn`, `mller`, `kva` and would otherwise be flagged).
pub fn flag_personal_name(second_level_label: &str) -> bool {
    if second_level_label.starts_with("xn--") {
        return false;
    }

    let tokens: Vec<&str> = second_level_label
        .split(['-', '.', '_'])
        .filter(|token| !token.is_empty())
        .collect();

    (2..=4).contains(&tokens.len())
        && tokens
            .iter()
            .all(|token| token.chars().all(|c| c.is_ascii_alphabetic()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(
        domain: &str,
        source: crate::types::SourceId,
        category: Category,
        hint: ScopeHint,
    ) -> RawEntry {
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
        fn public_suffix_is_never_apex_regardless_of_hint_but_downgrades_to_exact_host() {
            assert_eq!(
                resolve_scope("co.uk", ScopeHint::None, &[]),
                Some(RuleScope::ExactHost)
            );
            assert_eq!(
                resolve_scope("co.uk", ScopeHint::Apex, &[]),
                Some(RuleScope::ExactHost)
            );
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
                &[entry(
                    "example.com",
                    SourceId::StevenBlack,
                    Category::Adult,
                    ScopeHint::Apex,
                )],
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
                    entry(
                        "example.com",
                        SourceId::StevenBlack,
                        Category::Adult,
                        ScopeHint::Apex,
                    ),
                    entry(
                        "example.com",
                        SourceId::Hagezi,
                        Category::Adult,
                        ScopeHint::Apex,
                    ),
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
                    entry(
                        "example.com",
                        SourceId::StevenBlack,
                        Category::Adult,
                        ScopeHint::Apex,
                    ),
                    entry(
                        "example.com",
                        SourceId::Ut1,
                        Category::Gambling,
                        ScopeHint::Apex,
                    ),
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
                    entry(
                        "example.com",
                        SourceId::StevenBlack,
                        Category::Adult,
                        ScopeHint::None,
                    ),
                    entry(
                        "example.com",
                        SourceId::Hagezi,
                        Category::Adult,
                        ScopeHint::Apex,
                    ),
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
                    entry(
                        "example.com",
                        SourceId::StevenBlack,
                        Category::Adult,
                        ScopeHint::Apex,
                    ),
                    entry(
                        "www.example.com",
                        SourceId::StevenBlack,
                        Category::Adult,
                        ScopeHint::None,
                    ),
                ],
                &[],
            );
            assert_eq!(out.entries.len(), 2);
            let apex = out
                .entries
                .iter()
                .find(|e| e.domain == "example.com")
                .unwrap();
            let host = out
                .entries
                .iter()
                .find(|e| e.domain == "www.example.com")
                .unwrap();
            assert_eq!(apex.scope, RuleScope::Apex);
            assert_eq!(host.scope, RuleScope::ExactHost);
        }

        #[test]
        fn a_public_suffix_entry_is_kept_as_exact_host_not_dropped() {
            // "co.uk" is itself a public suffix — never eligible for Apex (that would claim the
            // whole shared TLD) — but the literal string is still safe to name exactly, so it's
            // kept as ExactHost rather than dropped. See classify_scope's own doc comment.
            let out = merge(
                &[entry(
                    "co.uk",
                    SourceId::Ut1,
                    Category::Adult,
                    ScopeHint::None,
                )],
                &[],
            );
            assert_eq!(out.entries.len(), 1);
            assert_eq!(out.entries[0].domain, "co.uk");
            assert_eq!(out.entries[0].scope, RuleScope::ExactHost);
            assert_eq!(out.report.dropped_shared_hosting_denylisted, 0);
        }

        #[test]
        fn a_wildcard_hinted_public_suffix_entry_downgrades_to_exact_host_not_dropped() {
            // A wildcard source entry (*.co.uk) still can't get Apex over the whole TLD, but the
            // literal base is kept as ExactHost rather than discarded outright — a strict subset
            // of what the source asked for, not a widening.
            let out = merge(
                &[entry(
                    "co.uk",
                    SourceId::Ut1,
                    Category::Adult,
                    ScopeHint::Apex,
                )],
                &[],
            );
            assert_eq!(out.entries.len(), 1);
            assert_eq!(out.entries[0].scope, RuleScope::ExactHost);
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
            assert_eq!(out.report.dropped_shared_hosting_denylisted, 1);
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
            assert_eq!(out.report.dropped_shared_hosting_denylisted, 0);
        }

        #[test]
        fn output_is_empty_for_empty_input() {
            let out = merge(&[], &[]);
            assert_eq!(out, MergeOutput::default());
        }

        #[test]
        fn an_ip_literal_is_dropped_and_counted_separately() {
            // RFC 1035 labels are digit-legal, so "0.0.0.0" normalizes as itself rather than
            // erroring — it must be caught by its own check, not conflated with either drop
            // reason above.
            let out = merge(
                &[entry(
                    "0.0.0.0",
                    SourceId::StevenBlack,
                    Category::Adult,
                    ScopeHint::None,
                )],
                &[],
            );
            assert!(out.entries.is_empty());
            assert_eq!(out.report.dropped_ip_literal, 1);
            assert_eq!(out.report.dropped_normalization_failed, 0);
            assert_eq!(out.report.dropped_shared_hosting_denylisted, 0);
        }

        #[test]
        fn shared_hosting_denylist_matches_despite_case_and_a_trailing_dot() {
            // The denylist is a hand-authored, checked-in file — exactly where a stray
            // capitalization or trailing dot would otherwise silently fail to match and reopen
            // the shared-hosting hole this parameter exists to close.
            let out = merge(
                &[entry(
                    "shared-host.example",
                    SourceId::Ut1,
                    Category::Adult,
                    ScopeHint::Apex,
                )],
                &["Shared-Host.example."],
            );
            assert!(out.entries.is_empty());
            assert_eq!(out.report.dropped_shared_hosting_denylisted, 1);
        }

        #[test]
        fn a_domain_that_fails_normalization_and_would_also_be_denylisted_counts_only_once() {
            // Precedence: an entry that can't be normalized at all never reaches classify_scope,
            // so only the normalization counter moves, never both.
            let over_long_label = "a".repeat(64);
            let unnormalizable = format!("{over_long_label}.com");
            let out = merge(
                &[entry(
                    &unnormalizable,
                    SourceId::Ut1,
                    Category::Adult,
                    ScopeHint::None,
                )],
                &[unnormalizable.as_str()],
            );
            assert_eq!(out.report.dropped_normalization_failed, 1);
            assert_eq!(out.report.dropped_shared_hosting_denylisted, 0);
        }

        #[test]
        fn three_sources_for_the_same_domain_union_all_three() {
            let out = merge(
                &[
                    entry(
                        "example.com",
                        SourceId::StevenBlack,
                        Category::Adult,
                        ScopeHint::Apex,
                    ),
                    entry(
                        "example.com",
                        SourceId::Hagezi,
                        Category::Gambling,
                        ScopeHint::Apex,
                    ),
                    entry(
                        "example.com",
                        SourceId::Ut1,
                        Category::Dating,
                        ScopeHint::Apex,
                    ),
                ],
                &[],
            );
            assert_eq!(out.entries.len(), 1);
            assert_eq!(
                out.entries[0].sources,
                vec![SourceId::StevenBlack, SourceId::Hagezi, SourceId::Ut1]
            );
            assert_eq!(
                out.entries[0].categories,
                vec![Category::Adult, Category::Gambling, Category::Dating]
            );
        }

        #[test]
        fn the_same_source_and_category_repeated_three_times_dedupes_to_one() {
            let raw = entry(
                "example.com",
                SourceId::Ut1,
                Category::Adult,
                ScopeHint::Apex,
            );
            let out = merge(&[raw.clone(), raw.clone(), raw], &[]);
            assert_eq!(out.entries.len(), 1);
            assert_eq!(out.entries[0].sources, vec![SourceId::Ut1]);
            assert_eq!(out.entries[0].categories, vec![Category::Adult]);
        }

        #[test]
        fn mixed_case_and_a_trailing_dot_normalize_onto_the_same_merged_entry() {
            // This is the entire reason merge() normalizes before keying: two spellings of the
            // same domain must collapse into one MergedEntry, not survive as two.
            let out = merge(
                &[
                    entry(
                        "EXAMPLE.COM",
                        SourceId::Ut1,
                        Category::Adult,
                        ScopeHint::Apex,
                    ),
                    entry(
                        "example.com.",
                        SourceId::Hagezi,
                        Category::Gambling,
                        ScopeHint::Apex,
                    ),
                ],
                &[],
            );
            assert_eq!(out.entries.len(), 1);
            assert_eq!(out.entries[0].domain, "example.com");
            assert_eq!(
                out.entries[0].sources,
                vec![SourceId::Hagezi, SourceId::Ut1]
            );
            assert_eq!(
                out.entries[0].categories,
                vec![Category::Adult, Category::Gambling]
            );
        }

        #[test]
        fn scope_reconciliation_is_independent_of_which_entry_arrives_first() {
            // The mirror of merging_apex_and_exact_host_for_the_same_domain_takes_the_wider_scope
            // above: feeding the Apex-hinted (wildcard) entry FIRST must not let a later plain
            // entry downgrade it back to ExactHost.
            let out = merge(
                &[
                    entry(
                        "example.com",
                        SourceId::Hagezi,
                        Category::Adult,
                        ScopeHint::Apex,
                    ),
                    entry(
                        "example.com",
                        SourceId::StevenBlack,
                        Category::Adult,
                        ScopeHint::None,
                    ),
                ],
                &[],
            );
            assert_eq!(out.entries.len(), 1);
            assert_eq!(out.entries[0].scope, RuleScope::Apex);
        }

        #[test]
        fn merge_output_does_not_depend_on_raw_entry_order() {
            let forward = vec![
                entry(
                    "example.com",
                    SourceId::StevenBlack,
                    Category::Adult,
                    ScopeHint::Apex,
                ),
                entry(
                    "example.com",
                    SourceId::Hagezi,
                    Category::Gambling,
                    ScopeHint::Apex,
                ),
                entry(
                    "example.com",
                    SourceId::Ut1,
                    Category::Dating,
                    ScopeHint::Apex,
                ),
            ];
            let mut reversed = forward.clone();
            reversed.reverse();

            let forward_out = merge(&forward, &[]);
            let reversed_out = merge(&reversed, &[]);

            assert_eq!(forward_out, reversed_out);
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
            let entries = vec![merged(
                "example.com",
                vec![Category::Adult, Category::Gambling],
            )];
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

        #[test]
        fn an_entry_with_no_categories_is_never_included() {
            // Not reachable through merge() today (every RawEntry carries exactly one category),
            // but filter_by_category is pub — its own degenerate case should hold on its own.
            let entries = vec![merged("example.com", vec![])];
            let out = filter_by_category(
                &entries,
                &[Category::Adult, Category::Gambling, Category::Dating],
            );
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

        #[test]
        fn a_punycode_ace_label_is_never_flagged() {
            // "xn--mller-kva" is the A-label normalize() produces for "müller" — its hyphenated
            // fragments (xn, mller, kva) are encoding artifacts, not name tokens, and must never
            // be treated as if they were.
            assert!(!flag_personal_name("xn--mller-kva"));
            assert!(!flag_personal_name("xn--nxasmq6b")); // an ACE label with no interior hyphen
        }
    }
}
