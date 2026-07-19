use crate::evaluator::{MlClassifier, Thresholds, evaluate};
use crate::lexicon::LexiconMatcher;
use crate::normalize::{NormalizationPipeline, NormalizedText};
use crate::scorer::{SourceKind, score};
use crate::verdict::Verdict;

pub struct PolicyEngine {
    matcher: LexiconMatcher,
    thresholds: Thresholds,
    ml: Option<Box<dyn MlClassifier>>,
}

impl PolicyEngine {
    pub fn new(matcher: LexiconMatcher, thresholds: Thresholds) -> Self {
        Self { matcher, thresholds, ml: None }
    }

    pub fn with_ml(mut self, classifier: Box<dyn MlClassifier>) -> Self {
        self.ml = Some(classifier);
        self
    }

    /// Normalize `text` then run the full pipeline.
    pub fn evaluate(&self, text: &str, source: SourceKind) -> Verdict {
        let pipeline = NormalizationPipeline::for_language(self.matcher.language());
        let views = pipeline.normalize_views(text);
        self.evaluate_normalized(&views, source)
    }

    /// Hot path for callers that already hold pre-normalized views.
    pub fn evaluate_normalized(&self, views: &NormalizedText, source: SourceKind) -> Verdict {
        let matches = self.matcher.matches_normalized(views);
        let result = score(&matches, source);
        evaluate(
            result.score,
            result.evidence,
            &self.thresholds,
            Some(views),
            self.ml.as_deref(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluator::Thresholds;
    use crate::lexicon::{
        Category, Dictionary, DictionaryTerm, LexiconBuilder, MatchMode, Severity,
    };
    use crate::normalize::Language;
    use crate::scorer::SourceKind;
    use crate::verdict::Action;

    fn engine() -> PolicyEngine {
        let matcher = LexiconBuilder::new(Language::English)
            .add_dictionary(Dictionary::new(
                "core",
                vec![
                    DictionaryTerm::new(
                        "explicit act",
                        Category::ExplicitAct,
                        Severity::High,
                        vec![MatchMode::ExactPhrase, MatchMode::TokenSequence],
                    ),
                    DictionaryTerm::new(
                        "nudity",
                        Category::Nudity,
                        Severity::Medium,
                        vec![MatchMode::ExactPhrase, MatchMode::Compact],
                    ),
                    DictionaryTerm::new(
                        "medical anatomy",
                        Category::MedicalException,
                        Severity::Low,
                        vec![MatchMode::ExactPhrase],
                    ),
                ],
            ))
            .build()
            .expect("valid test dictionary");

        PolicyEngine::new(matcher, Thresholds::default())
    }

    #[test]
    fn clean_text_is_allowed() {
        let v = engine().evaluate("the quick brown fox", SourceKind::BrowserTitle);
        assert_eq!(v.action, Action::Allow);
        assert_eq!(v.score, 0);
        assert!(v.evidence.is_empty());
    }

    #[test]
    fn high_severity_exact_match_blocks() {
        // One occurrence, matched at its best mode (ExactPhrase, x1.00): 80.
        // The term also fires TokenSequence, but that is the same occurrence
        // seen through another view and must not be added on top.
        let v = engine().evaluate("contains explicit act here", SourceKind::BrowserTitle);
        assert_eq!(v.action, Action::Block);
        assert_eq!(v.score, 80);
    }

    #[test]
    fn medium_severity_occurrences_accumulate_without_blocking() {
        // Two occurrences x 35 = 70 → Warn. A Medium term must not reach the
        // Block band on repetition alone; that band is for High severity.
        let v = engine().evaluate("nudity nudity here", SourceKind::BrowserTitle);
        assert_eq!(v.action, Action::Warn);
        assert_eq!(v.score, 70);
    }

    #[test]
    fn single_medium_severity_scores_below_warn_band() {
        // One occurrence of a Medium term is 35, under the warn threshold of
        // 50 — a single ambiguous word on its own says nothing.
        let v = engine().evaluate("mentions nudity once", SourceKind::BrowserTitle);
        assert_eq!(v.score, 35);
        assert_eq!(v.action, Action::Allow);
    }

    #[test]
    fn exception_reduces_score() {
        // high match fires ExactPhrase + TokenSequence = 144, then -40 from exception → 104 → clamped 100 → Block
        // Use a source with lower confidence to show exception still reduces
        // OcrLow: ExactPhrase 80*0.4=32, TokenSequence 80*0.8*0.4=25.6 → 57.6 → 58; exception: -40 → 18 → Allow
        let v = engine().evaluate(
            "explicit act alongside medical anatomy context",
            SourceKind::OcrLow,
        );
        assert_eq!(v.action, Action::Allow);
        assert!(v.score < 50);
    }

    #[test]
    fn evaluate_normalized_produces_same_result_as_evaluate() {
        let e = engine();
        let text = "explicit act example";
        let pipeline = NormalizationPipeline::for_language(e.matcher.language());
        let views = pipeline.normalize_views(text);

        let v1 = e.evaluate(text, SourceKind::BrowserTitle);
        let v2 = e.evaluate_normalized(&views, SourceKind::BrowserTitle);
        assert_eq!(v1, v2);
    }

    #[test]
    fn low_confidence_ocr_lowers_score() {
        // 80 x 1.00 (best mode) x 0.40 (OcrLow) = 32 → Allow. Text this
        // unreliably extracted should not drive a user-visible action.
        let v = engine().evaluate("explicit act", SourceKind::OcrLow);
        assert_eq!(v.score, 32);
        assert_eq!(v.action, Action::Allow);
    }

    #[test]
    fn compact_leet_match_is_detected() {
        let v = engine().evaluate("n-u-d-i-t-y content", SourceKind::BrowserTitle);
        // compact mode: 35 * 0.75 = 26 → Allow
        assert_eq!(v.action, Action::Allow);
        assert!(v.score > 0);
    }

    #[test]
    fn evidence_propagates_to_verdict() {
        let v = engine().evaluate("explicit act found", SourceKind::BrowserTitle);
        assert!(!v.evidence.is_empty());
        assert_eq!(v.evidence[0].category, Category::ExplicitAct);
    }

    #[test]
    fn custom_thresholds_change_action() {
        let matcher = LexiconBuilder::new(Language::English)
            .add_dictionary(Dictionary::new(
                "core",
                vec![DictionaryTerm::new(
                    "explicit act",
                    Category::ExplicitAct,
                    Severity::High,
                    vec![MatchMode::ExactPhrase],
                )],
            ))
            .build()
            .expect("valid");

        // Raise block threshold so score 80 only warns
        let engine = PolicyEngine::new(matcher, Thresholds { block: 90, warn: 50 });
        let v = engine.evaluate("explicit act", SourceKind::BrowserTitle);
        assert_eq!(v.action, Action::Warn);
    }
}
