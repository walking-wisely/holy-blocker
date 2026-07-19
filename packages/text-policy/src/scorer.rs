use std::collections::HashMap;

use crate::lexicon::{Category, LexiconMatch, MatchMode, Severity};
use crate::verdict::EvidenceItem;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceKind {
    BrowserTitle,
    BrowserUrl,
    AccessibilityTree,
    OcrHigh,
    OcrMedium,
    OcrLow,
}

pub struct ScoreResult {
    pub score: u32,
    pub evidence: Vec<EvidenceItem>,
}

fn base_score(severity: Severity) -> u32 {
    match severity {
        Severity::High => 80,
        Severity::Medium => 35,
        Severity::Low => 15,
    }
}

fn match_multiplier(mode: MatchMode) -> f32 {
    match mode {
        MatchMode::ExactPhrase => 1.00,
        MatchMode::TokenSequence => 0.80,
        MatchMode::Compact => 0.75,
        // Full confidence, unlike the prose TokenSequence it otherwise
        // resembles: a term appearing in a host or path has no surrounding
        // context that could make it innocent, so there is nothing for a
        // discount to hedge against. A URL also never matches ExactPhrase —
        // the phrase is not contiguous across separators — so discounting
        // this mode would put a block permanently out of reach for URLs.
        MatchMode::UrlTokenSequence => 1.00,
    }
}

fn source_multiplier(source: SourceKind) -> f32 {
    match source {
        SourceKind::BrowserTitle | SourceKind::BrowserUrl | SourceKind::AccessibilityTree => 1.00,
        SourceKind::OcrHigh => 0.90,
        SourceKind::OcrMedium => 0.70,
        SourceKind::OcrLow => 0.40,
    }
}

fn is_exception(category: Category) -> bool {
    matches!(
        category,
        Category::EducationException | Category::MedicalException | Category::SafetyException
    )
}

/// How many times a term occurred, and the best quality it was matched at.
struct TermHits<'a> {
    /// Match at the highest-quality mode seen. Carries the term metadata, which
    /// every match for the term shares, and the span used as evidence.
    best: &'a LexiconMatch,
    /// Distinct occurrences of the term in the text.
    occurrences: u32,
}

/// Collapses raw matches into one entry per term.
///
/// The matcher emits one `LexiconMatch` per match mode, so a single occurrence
/// of a term listing both `ExactPhrase` and `Compact` arrives here twice. Those
/// are one occurrence seen through two normalization views, not two hits, and
/// summing them would let the number of modes an author happens to list drive
/// the score instead of the term's severity.
///
/// Spans cannot disambiguate this: they are offsets into different views (the
/// normalized and leet-compact texts), so the same occurrence has different
/// spans per mode. Occurrences are therefore counted within a single mode, and
/// the term is credited with the most occurrences any one mode saw, at the
/// quality of the best mode that fired.
///
/// Returned in first-seen order so evidence is deterministic.
fn group_by_term<'a>(matches: &'a [LexiconMatch]) -> Vec<TermHits<'a>> {
    let mut order: Vec<usize> = Vec::new();
    let mut by_term: HashMap<usize, HashMap<MatchMode, (u32, &'a LexiconMatch)>> = HashMap::new();

    for m in matches {
        let modes = by_term.entry(m.term_id).or_insert_with(|| {
            order.push(m.term_id);
            HashMap::new()
        });
        modes
            .entry(m.mode)
            .and_modify(|(count, _)| *count += 1)
            .or_insert((1, m));
    }

    order
        .into_iter()
        .map(|term_id| {
            let modes = &by_term[&term_id];
            let best = modes
                .iter()
                .max_by(|(a, _), (b, _)| {
                    match_multiplier(**a)
                        .partial_cmp(&match_multiplier(**b))
                        .expect("match multipliers are finite")
                })
                .map(|(_, (_, m))| *m)
                .expect("term id came from an existing match");

            TermHits {
                best,
                occurrences: modes.values().map(|(count, _)| *count).max().unwrap_or(1),
            }
        })
        .collect()
}

/// Aggregates `LexiconMatch` results into a clamped `0..=100` score and
/// a list of evidence items that explain the decision.
///
/// Exception-category matches reduce the total score rather than raising it.
/// The source multiplier reflects confidence in the text extraction method.
pub fn score(matches: &[LexiconMatch], source: SourceKind) -> ScoreResult {
    let src_mult = source_multiplier(source);
    let mut positive: f32 = 0.0;
    let mut exception_reduction: f32 = 0.0;
    let mut evidence = Vec::new();

    for hits in group_by_term(matches) {
        let m = hits.best;
        let occurrences = hits.occurrences as f32;

        if is_exception(m.category) {
            exception_reduction += 40.0 * occurrences;
            continue;
        }

        let base = base_score(m.severity);
        let multiplier = match_multiplier(m.mode) * src_mult;
        positive += base as f32 * multiplier * occurrences;

        evidence.push(EvidenceItem {
            rule_id: format!("{}:{}", m.dictionary, m.phrase),
            category: m.category,
            severity: m.severity,
            span: m.span,
            base_score: base,
            multiplier,
        });
    }

    let raw = (positive - exception_reduction).max(0.0);
    let score = (raw.round() as u32).min(100);

    ScoreResult { score, evidence }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexicon::{MatchSpan, LexiconMatch};

    fn make_match(
        category: Category,
        severity: Severity,
        mode: MatchMode,
    ) -> LexiconMatch {
        make_term_match(0, category, severity, mode, 0)
    }

    /// A match with an explicit term id and span start, so tests can express
    /// "same term, several modes" apart from "same term, several occurrences".
    fn make_term_match(
        term_id: usize,
        category: Category,
        severity: Severity,
        mode: MatchMode,
        start: usize,
    ) -> LexiconMatch {
        LexiconMatch {
            term_id,
            dictionary: "test".into(),
            phrase: format!("word{term_id}"),
            category,
            severity,
            mode,
            surface: format!("word{term_id}"),
            span: MatchSpan::Bytes { start, end: start + 4 },
        }
    }

    // ── one occurrence matching several modes ──────────────────────────────
    // The matcher emits one LexiconMatch per mode, so a single occurrence of a
    // term listing both ExactPhrase and Compact arrives here twice. Those are
    // the same occurrence seen through two normalization views, not two hits.

    #[test]
    fn same_term_matching_two_modes_scores_once() {
        // Spans differ because they index different views (normalized vs
        // leet-compact) — this is exactly what the matcher produces for
        // "mentions nudity once".
        let exact = make_term_match(0, Category::Nudity, Severity::Medium, MatchMode::ExactPhrase, 9);
        let compact = make_term_match(0, Category::Nudity, Severity::Medium, MatchMode::Compact, 8);
        let result = score(&[exact, compact], SourceKind::BrowserTitle);
        // Medium base 35 at best (exact) quality — not 35 + 26 = 61.
        assert_eq!(result.score, 35);
        assert_eq!(result.evidence.len(), 1);
    }

    #[test]
    fn same_term_two_modes_uses_best_multiplier() {
        let exact = make_term_match(0, Category::ExplicitAct, Severity::High, MatchMode::ExactPhrase, 0);
        let compact = make_term_match(0, Category::ExplicitAct, Severity::High, MatchMode::Compact, 0);
        let result = score(&[exact, compact], SourceKind::BrowserTitle);
        // Best available quality is exact (x1.00), so 80 — not 80 + 60.
        assert_eq!(result.score, 80);
        assert_eq!(result.evidence[0].multiplier, 1.00);
    }

    #[test]
    fn term_matching_only_compact_keeps_compact_multiplier() {
        // Evasion spelling: only the compact view fires, so quality stays 0.75.
        let compact = make_term_match(0, Category::ExplicitAct, Severity::High, MatchMode::Compact, 0);
        let result = score(&[compact], SourceKind::BrowserTitle);
        assert_eq!(result.score, 60);
    }

    #[test]
    fn medium_term_with_three_modes_cannot_reach_block_band() {
        // The regression that mattered: mode count must not let a Medium term
        // cross a threshold meant for High severity.
        let exact = make_term_match(0, Category::Nudity, Severity::Medium, MatchMode::ExactPhrase, 0);
        let compact = make_term_match(0, Category::Nudity, Severity::Medium, MatchMode::Compact, 0);
        let tokens = make_term_match(0, Category::Nudity, Severity::Medium, MatchMode::TokenSequence, 0);
        let result = score(&[exact, compact, tokens], SourceKind::BrowserTitle);
        assert_eq!(result.score, 35);
    }

    // ── several occurrences of one term ────────────────────────────────────

    #[test]
    fn repeated_term_occurrences_still_accumulate() {
        let first = make_term_match(0, Category::Nudity, Severity::Medium, MatchMode::ExactPhrase, 0);
        let second = make_term_match(0, Category::Nudity, Severity::Medium, MatchMode::ExactPhrase, 40);
        let result = score(&[first, second], SourceKind::BrowserTitle);
        assert_eq!(result.score, 70);
    }

    #[test]
    fn repeated_term_across_modes_counts_occurrences_not_modes() {
        // Two occurrences, each firing in two modes → four matches, two hits.
        let matches = vec![
            make_term_match(0, Category::Nudity, Severity::Medium, MatchMode::ExactPhrase, 0),
            make_term_match(0, Category::Nudity, Severity::Medium, MatchMode::Compact, 0),
            make_term_match(0, Category::Nudity, Severity::Medium, MatchMode::ExactPhrase, 40),
            make_term_match(0, Category::Nudity, Severity::Medium, MatchMode::Compact, 40),
        ];
        let result = score(&matches, SourceKind::BrowserTitle);
        assert_eq!(result.score, 70);
    }

    // ── exceptions must collapse the same way ──────────────────────────────

    #[test]
    fn exception_term_matching_two_modes_reduces_once() {
        let positive = make_term_match(0, Category::ExplicitAct, Severity::High, MatchMode::ExactPhrase, 0);
        let exc_exact = make_term_match(1, Category::MedicalException, Severity::Low, MatchMode::ExactPhrase, 20);
        let exc_compact = make_term_match(1, Category::MedicalException, Severity::Low, MatchMode::Compact, 19);
        // 80 - 40 = 40, not 80 - 80 = 0.
        let result = score(&[positive, exc_exact, exc_compact], SourceKind::BrowserTitle);
        assert_eq!(result.score, 40);
    }

    #[test]
    fn distinct_terms_still_accumulate_separately() {
        let a = make_term_match(0, Category::Nudity, Severity::Medium, MatchMode::ExactPhrase, 0);
        let b = make_term_match(1, Category::AnatomyAmbiguous, Severity::Low, MatchMode::ExactPhrase, 40);
        // 35 + 15 = 50 — grouping is per term, not global.
        let result = score(&[a, b], SourceKind::BrowserTitle);
        assert_eq!(result.score, 50);
        assert_eq!(result.evidence.len(), 2);
    }

    #[test]
    fn empty_matches_score_zero() {
        let result = score(&[], SourceKind::BrowserTitle);
        assert_eq!(result.score, 0);
        assert!(result.evidence.is_empty());
    }

    #[test]
    fn high_severity_exact_browser_scores_80() {
        let m = make_match(Category::ExplicitAct, Severity::High, MatchMode::ExactPhrase);
        let result = score(&[m], SourceKind::BrowserTitle);
        assert_eq!(result.score, 80);
    }

    #[test]
    fn medium_severity_exact_browser_scores_35() {
        let m = make_match(Category::Nudity, Severity::Medium, MatchMode::ExactPhrase);
        let result = score(&[m], SourceKind::BrowserTitle);
        assert_eq!(result.score, 35);
    }

    #[test]
    fn low_severity_exact_browser_scores_15() {
        let m = make_match(Category::AnatomyAmbiguous, Severity::Low, MatchMode::ExactPhrase);
        let result = score(&[m], SourceKind::BrowserTitle);
        assert_eq!(result.score, 15);
    }

    #[test]
    fn compact_mode_applies_075_multiplier() {
        // High severity compact: 80 * 0.75 * 1.0 = 60
        let m = make_match(Category::ExplicitAct, Severity::High, MatchMode::Compact);
        let result = score(&[m], SourceKind::BrowserTitle);
        assert_eq!(result.score, 60);
    }

    #[test]
    fn low_confidence_ocr_reduces_score() {
        // High severity exact, low OCR: 80 * 1.0 * 0.4 = 32
        let m = make_match(Category::ExplicitAct, Severity::High, MatchMode::ExactPhrase);
        let result = score(&[m], SourceKind::OcrLow);
        assert_eq!(result.score, 32);
    }

    #[test]
    fn high_confidence_ocr_reduces_score_slightly() {
        // High severity exact, high OCR: 80 * 1.0 * 0.9 = 72
        let m = make_match(Category::ExplicitAct, Severity::High, MatchMode::ExactPhrase);
        let result = score(&[m], SourceKind::OcrHigh);
        assert_eq!(result.score, 72);
    }

    #[test]
    fn score_clamps_at_100() {
        // Five separate occurrences of one term: 5 x 80 = 400 → clamped.
        let matches: Vec<LexiconMatch> = (0..5)
            .map(|i| {
                make_term_match(
                    0,
                    Category::ExplicitAct,
                    Severity::High,
                    MatchMode::ExactPhrase,
                    i * 10,
                )
            })
            .collect();
        let result = score(&matches, SourceKind::BrowserTitle);
        assert_eq!(result.score, 100);
    }

    #[test]
    fn exception_category_reduces_score() {
        let positive = make_term_match(0, Category::ExplicitAct, Severity::High, MatchMode::ExactPhrase, 0);
        let exception = make_term_match(1, Category::EducationException, Severity::Low, MatchMode::ExactPhrase, 20);
        // 80 - 40 = 40
        let result = score(&[positive, exception], SourceKind::BrowserTitle);
        assert_eq!(result.score, 40);
    }

    #[test]
    fn exception_cannot_push_score_below_zero() {
        let exception = make_match(Category::MedicalException, Severity::Low, MatchMode::ExactPhrase);
        let result = score(&[exception], SourceKind::BrowserTitle);
        assert_eq!(result.score, 0);
    }

    #[test]
    fn exception_matches_excluded_from_evidence() {
        let positive = make_term_match(0, Category::Nudity, Severity::Medium, MatchMode::ExactPhrase, 0);
        let exception = make_term_match(1, Category::SafetyException, Severity::Low, MatchMode::ExactPhrase, 20);
        let result = score(&[positive, exception], SourceKind::BrowserTitle);
        assert_eq!(result.evidence.len(), 1);
        assert_eq!(result.evidence[0].category, Category::Nudity);
    }

    #[test]
    fn evidence_item_has_correct_rule_id() {
        let m = make_match(Category::Nudity, Severity::Medium, MatchMode::ExactPhrase);
        let result = score(&[m], SourceKind::BrowserTitle);
        assert_eq!(result.evidence[0].rule_id, "test:word0");
    }

    #[test]
    fn multiple_positive_matches_accumulate() {
        let m1 = make_term_match(0, Category::ExplicitAct, Severity::High, MatchMode::ExactPhrase, 0);
        let m2 = make_term_match(1, Category::Nudity, Severity::Medium, MatchMode::ExactPhrase, 20);
        // 80 + 35 = 115 → clamped to 100
        let result = score(&[m1, m2], SourceKind::BrowserTitle);
        assert_eq!(result.score, 100);
        assert_eq!(result.evidence.len(), 2);
    }

    #[test]
    fn url_token_sequence_applies_full_multiplier() {
        // Medium severity URL token: 35 * 1.0 * 1.0 = 35
        let m = make_match(Category::Nudity, Severity::Medium, MatchMode::UrlTokenSequence);
        let result = score(&[m], SourceKind::BrowserUrl);
        assert_eq!(result.score, 35);
    }

    #[test]
    fn high_severity_url_token_reaches_block_band() {
        // The point of the x1.00 URL multiplier: a High term in a host or path
        // must be able to reach 80. At x0.80 it capped at 64 and no URL could
        // ever block.
        let m = make_match(Category::AdultPlatform, Severity::High, MatchMode::UrlTokenSequence);
        let result = score(&[m], SourceKind::BrowserUrl);
        assert_eq!(result.score, 80);
    }

    #[test]
    fn url_token_outranks_token_sequence_for_same_occurrence() {
        // A hostname hit fires both modes; the better one must win.
        let tokens = make_term_match(0, Category::AdultPlatform, Severity::High, MatchMode::TokenSequence, 8);
        let url = make_term_match(0, Category::AdultPlatform, Severity::High, MatchMode::UrlTokenSequence, 8);
        let result = score(&[tokens, url], SourceKind::BrowserUrl);
        assert_eq!(result.score, 80);
        assert_eq!(result.evidence.len(), 1);
    }
}
