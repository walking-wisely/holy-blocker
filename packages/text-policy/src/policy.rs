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
        // ExactPhrase (80) + TokenSequence (80*0.8=64) → 144 → clamped to 100 → Block
        let v = engine().evaluate("contains explicit act here", SourceKind::BrowserTitle);
        assert_eq!(v.action, Action::Block);
        assert_eq!(v.score, 100);
    }

    #[test]
    fn medium_severity_matches_accumulate() {
        // Each "nudity" fires ExactPhrase (35) + Compact (35*0.75≈26); two occurrences → >80 → Block
        let v = engine().evaluate("nudity nudity here", SourceKind::BrowserTitle);
        assert_eq!(v.action, Action::Block);
        assert!(v.score >= 80);
    }

    #[test]
    fn single_medium_severity_warns() {
        // ExactPhrase (35) + Compact (≈26) = ≈61 → Warn
        let v = engine().evaluate("mentions nudity once", SourceKind::BrowserTitle);
        assert_eq!(v.action, Action::Warn);
        assert!(v.score >= 50 && v.score < 80);
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
        // ExactPhrase 80*0.4=32, TokenSequence 80*0.8*0.4=25.6 → 57 → Warn (not Block)
        let v = engine().evaluate("explicit act", SourceKind::OcrLow);
        assert_eq!(v.action, Action::Warn);
        assert!(v.score < 80);
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
