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
        MatchMode::UrlTokenSequence => 0.80,
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

    for m in matches {
        if is_exception(m.category) {
            exception_reduction += 40.0;
            continue;
        }

        let base = base_score(m.severity);
        let multiplier = match_multiplier(m.mode) * src_mult;
        positive += base as f32 * multiplier;

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
        LexiconMatch {
            term_id: 0,
            dictionary: "test".into(),
            phrase: "word".into(),
            category,
            severity,
            mode,
            surface: "word".into(),
            span: MatchSpan::Bytes { start: 0, end: 4 },
        }
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
        let matches: Vec<LexiconMatch> = (0..5)
            .map(|_| make_match(Category::ExplicitAct, Severity::High, MatchMode::ExactPhrase))
            .collect();
        let result = score(&matches, SourceKind::BrowserTitle);
        assert_eq!(result.score, 100);
    }

    #[test]
    fn exception_category_reduces_score() {
        let positive = make_match(Category::ExplicitAct, Severity::High, MatchMode::ExactPhrase);
        let exception = make_match(Category::EducationException, Severity::Low, MatchMode::ExactPhrase);
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
        let positive = make_match(Category::Nudity, Severity::Medium, MatchMode::ExactPhrase);
        let exception = make_match(Category::SafetyException, Severity::Low, MatchMode::ExactPhrase);
        let result = score(&[positive, exception], SourceKind::BrowserTitle);
        assert_eq!(result.evidence.len(), 1);
        assert_eq!(result.evidence[0].category, Category::Nudity);
    }

    #[test]
    fn evidence_item_has_correct_rule_id() {
        let m = make_match(Category::Nudity, Severity::Medium, MatchMode::ExactPhrase);
        let result = score(&[m], SourceKind::BrowserTitle);
        assert_eq!(result.evidence[0].rule_id, "test:word");
    }

    #[test]
    fn multiple_positive_matches_accumulate() {
        let m1 = make_match(Category::ExplicitAct, Severity::High, MatchMode::ExactPhrase);
        let m2 = make_match(Category::Nudity, Severity::Medium, MatchMode::ExactPhrase);
        // 80 + 35 = 115 → clamped to 100
        let result = score(&[m1, m2], SourceKind::BrowserTitle);
        assert_eq!(result.score, 100);
        assert_eq!(result.evidence.len(), 2);
    }

    #[test]
    fn url_token_sequence_applies_080_multiplier() {
        // Medium severity URL token: 35 * 0.8 * 1.0 = 28
        let m = make_match(Category::Nudity, Severity::Medium, MatchMode::UrlTokenSequence);
        let result = score(&[m], SourceKind::BrowserUrl);
        assert_eq!(result.score, 28);
    }
}
