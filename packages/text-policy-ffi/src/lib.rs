//! UniFFI surface over `text-policy`.
//!
//! Exists so non-Rust edges — the Android service first — reach one policy
//! engine instead of reimplementing normalization and scoring per platform.
//! Everything here is a thin mapping layer: no policy decisions live in this
//! crate, only type translation across the FFI boundary.

use std::sync::Arc;

use text_policy::{
    evaluator::Thresholds,
    lexicon::{Category, Dictionary, DictionaryTerm, LexiconBuilder, MatchMode, Severity},
    normalize::Language,
    policy, scorer, verdict,
};

uniffi::setup_scaffolding!();

/// Provenance of the text, which scales the score by how much the source can
/// be trusted. `AccessibilityTree` is the Android text path.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum SourceKind {
    BrowserTitle,
    BrowserUrl,
    AccessibilityTree,
    OcrHigh,
    OcrMedium,
    OcrLow,
}

impl From<SourceKind> for scorer::SourceKind {
    fn from(value: SourceKind) -> Self {
        match value {
            SourceKind::BrowserTitle => Self::BrowserTitle,
            SourceKind::BrowserUrl => Self::BrowserUrl,
            SourceKind::AccessibilityTree => Self::AccessibilityTree,
            SourceKind::OcrHigh => Self::OcrHigh,
            SourceKind::OcrMedium => Self::OcrMedium,
            SourceKind::OcrLow => Self::OcrLow,
        }
    }
}

/// What the caller should do. Mirrors `text_policy::verdict::Action`.
///
/// Per the formation model this describes *content*, never the person: there
/// is deliberately no variant that names a fault.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum Action {
    Block,
    Blur,
    Warn,
    Log,
    Allow,
}

impl From<verdict::Action> for Action {
    fn from(value: verdict::Action) -> Self {
        match value {
            verdict::Action::Block => Self::Block,
            verdict::Action::Blur => Self::Blur,
            verdict::Action::Warn => Self::Warn,
            verdict::Action::Log => Self::Log,
            verdict::Action::Allow => Self::Allow,
        }
    }
}

/// Evidence is intentionally not carried across the boundary: the edge needs
/// the action to act on and the score to log, and matched phrases are exactly
/// the kind of content this project does not move around.
#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct Verdict {
    pub action: Action,
    pub score: u32,
}

/// Starter dictionary, mirroring `mitm-proxy`'s `build_default_engine`.
///
/// Terms are representative placeholders; a real implementation would load
/// dictionaries from a config file or embedded asset.
fn builtin_matcher() -> text_policy::lexicon::LexiconMatcher {
    LexiconBuilder::new(Language::English)
        .add_dictionary(Dictionary::new(
            "adult-platforms",
            vec![
                DictionaryTerm::new(
                    "adult platform",
                    Category::AdultPlatform,
                    Severity::High,
                    vec![
                        MatchMode::ExactPhrase,
                        MatchMode::TokenSequence,
                        MatchMode::UrlTokenSequence,
                    ],
                ),
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
            ],
        ))
        .add_dictionary(Dictionary::new(
            "exceptions",
            vec![DictionaryTerm::new(
                "medical anatomy",
                Category::MedicalException,
                Severity::Low,
                vec![MatchMode::ExactPhrase],
            )],
        ))
        .build()
        .expect("built-in dictionary must be valid")
}

/// Handle held by the foreign caller for the process lifetime. Construction
/// compiles the lexicon automaton, so build it once and reuse it — the Android
/// service holds a single instance rather than one per event.
#[derive(uniffi::Object)]
pub struct PolicyEngine {
    inner: policy::PolicyEngine,
}

#[uniffi::export]
impl PolicyEngine {
    /// Builds an engine over the built-in starter dictionary and default
    /// thresholds (block 80, warn 50).
    #[uniffi::constructor]
    pub fn with_builtin_dictionary() -> Arc<Self> {
        Arc::new(Self {
            inner: policy::PolicyEngine::new(builtin_matcher(), Thresholds::default()),
        })
    }

    /// Same as `with_builtin_dictionary` but with caller-chosen thresholds.
    #[uniffi::constructor]
    pub fn with_thresholds(block: u32, warn: u32) -> Arc<Self> {
        Arc::new(Self {
            inner: policy::PolicyEngine::new(builtin_matcher(), Thresholds { block, warn }),
        })
    }

    pub fn evaluate(&self, text: String, source: SourceKind) -> Verdict {
        let v = self.inner.evaluate(&text, source.into());
        Verdict {
            action: v.action.into(),
            score: v.score,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Arc<PolicyEngine> {
        PolicyEngine::with_builtin_dictionary()
    }

    #[test]
    fn clean_text_is_allowed() {
        let v = engine().evaluate("the quick brown fox".into(), SourceKind::AccessibilityTree);
        assert_eq!(v.action, Action::Allow);
        assert_eq!(v.score, 0);
    }

    #[test]
    fn high_severity_match_blocks_via_accessibility_source() {
        let v = engine().evaluate(
            "contains explicit act here".into(),
            SourceKind::AccessibilityTree,
        );
        assert_eq!(v.action, Action::Block);
    }

    #[test]
    fn url_source_matches_domain_tokens() {
        let v = engine().evaluate(
            "https://example.com/adult-platform/watch".into(),
            SourceKind::BrowserUrl,
        );
        assert_eq!(v.action, Action::Block);
    }

    #[test]
    fn custom_thresholds_are_honoured() {
        // "nudity" alone scores ~61 → Warn at defaults; a block cutoff of 60
        // must turn the same text into a Block.
        let strict = PolicyEngine::with_thresholds(60, 30);
        assert_eq!(
            strict
                .evaluate("mentions nudity once".into(), SourceKind::BrowserTitle)
                .action,
            Action::Block
        );
        assert_eq!(
            engine()
                .evaluate("mentions nudity once".into(), SourceKind::BrowserTitle)
                .action,
            Action::Warn
        );
    }

    #[test]
    fn source_kind_maps_to_every_text_policy_variant() {
        // Guards against a variant being added on one side only.
        let cases = [
            (SourceKind::BrowserTitle, scorer::SourceKind::BrowserTitle),
            (SourceKind::BrowserUrl, scorer::SourceKind::BrowserUrl),
            (
                SourceKind::AccessibilityTree,
                scorer::SourceKind::AccessibilityTree,
            ),
            (SourceKind::OcrHigh, scorer::SourceKind::OcrHigh),
            (SourceKind::OcrMedium, scorer::SourceKind::OcrMedium),
            (SourceKind::OcrLow, scorer::SourceKind::OcrLow),
        ];
        for (ffi, native) in cases {
            assert_eq!(scorer::SourceKind::from(ffi), native);
        }
    }

    #[test]
    fn action_maps_from_every_text_policy_variant() {
        let cases = [
            (verdict::Action::Block, Action::Block),
            (verdict::Action::Blur, Action::Blur),
            (verdict::Action::Warn, Action::Warn),
            (verdict::Action::Log, Action::Log),
            (verdict::Action::Allow, Action::Allow),
        ];
        for (native, ffi) in cases {
            assert_eq!(Action::from(native), ffi);
        }
    }

    #[test]
    fn lower_confidence_source_scores_below_accessibility_tree() {
        let e = engine();
        let high = e.evaluate("explicit act".into(), SourceKind::AccessibilityTree);
        let low = e.evaluate("explicit act".into(), SourceKind::OcrLow);
        assert!(low.score < high.score);
    }

    #[test]
    fn engine_is_reusable_across_calls() {
        let e = engine();
        assert_eq!(
            e.evaluate("explicit act".into(), SourceKind::AccessibilityTree)
                .action,
            Action::Block
        );
        assert_eq!(
            e.evaluate("harmless text".into(), SourceKind::AccessibilityTree)
                .action,
            Action::Allow
        );
    }
}
