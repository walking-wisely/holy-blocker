use crate::normalize::NormalizedText;
use crate::verdict::{Action, EvidenceItem, Verdict};

pub struct Thresholds {
    pub block: u32,
    pub warn: u32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self { block: 80, warn: 50 }
    }
}

/// Hook point for an optional ML classifier applied in the uncertain band.
///
/// When `score` falls in `[warn, block)` a classifier can raise or lower the
/// final action. Wire to `None` until a real eval set exists.
pub trait MlClassifier: Send + Sync {
    /// Returns `(probability 0.0–1.0, label)`. Higher probability means the
    /// content is more likely to match the blocked class.
    fn classify(&self, views: &NormalizedText, evidence: &[EvidenceItem]) -> (f32, &'static str);
}

/// Maps a `score` and its evidence to a [`Verdict`] using configurable thresholds.
///
/// Pass `ml` as `None` to skip the ML hook (the rule-based path is correct without it).
pub fn evaluate(
    score: u32,
    evidence: Vec<EvidenceItem>,
    thresholds: &Thresholds,
    _views: Option<&NormalizedText>,
    _ml: Option<&dyn MlClassifier>,
) -> Verdict {
    let action = if score >= thresholds.block {
        Action::Block
    } else if score >= thresholds.warn {
        Action::Warn
    } else {
        Action::Allow
    };

    Verdict { action, score, evidence }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexicon::{Category, MatchSpan, Severity};
    use crate::verdict::EvidenceItem;

    fn thresholds() -> Thresholds {
        Thresholds::default()
    }

    fn evidence() -> Vec<EvidenceItem> {
        vec![EvidenceItem {
            rule_id: "test:word".into(),
            category: Category::ExplicitAct,
            severity: Severity::High,
            span: MatchSpan::Bytes { start: 0, end: 4 },
            base_score: 80,
            multiplier: 1.0,
        }]
    }

    #[test]
    fn score_at_block_threshold_blocks() {
        let v = evaluate(80, evidence(), &thresholds(), None, None);
        assert_eq!(v.action, Action::Block);
        assert_eq!(v.score, 80);
    }

    #[test]
    fn score_above_block_threshold_blocks() {
        let v = evaluate(100, evidence(), &thresholds(), None, None);
        assert_eq!(v.action, Action::Block);
    }

    #[test]
    fn score_at_warn_threshold_warns() {
        let v = evaluate(50, vec![], &thresholds(), None, None);
        assert_eq!(v.action, Action::Warn);
        assert_eq!(v.score, 50);
    }

    #[test]
    fn score_between_warn_and_block_warns() {
        let v = evaluate(65, vec![], &thresholds(), None, None);
        assert_eq!(v.action, Action::Warn);
    }

    #[test]
    fn score_just_below_warn_allows() {
        let v = evaluate(49, vec![], &thresholds(), None, None);
        assert_eq!(v.action, Action::Allow);
    }

    #[test]
    fn score_zero_allows() {
        let v = evaluate(0, vec![], &thresholds(), None, None);
        assert_eq!(v.action, Action::Allow);
        assert_eq!(v.score, 0);
    }

    #[test]
    fn evidence_is_passed_through() {
        let ev = evidence();
        let v = evaluate(80, ev.clone(), &thresholds(), None, None);
        assert_eq!(v.evidence, ev);
    }

    #[test]
    fn custom_block_threshold_scores_below_it_warn() {
        // score 85 is above the default block (80) but below custom block (90) → Warn
        let t = Thresholds { block: 90, warn: 60 };
        let v = evaluate(85, vec![], &t, None, None);
        assert_eq!(v.action, Action::Warn);
    }

    #[test]
    fn custom_block_threshold_score_at_boundary_blocks() {
        let t = Thresholds { block: 90, warn: 60 };
        let v = evaluate(90, vec![], &t, None, None);
        assert_eq!(v.action, Action::Block);
    }

    #[test]
    fn custom_warn_threshold_score_below_it_allows() {
        let t = Thresholds { block: 90, warn: 60 };
        let v = evaluate(59, vec![], &t, None, None);
        assert_eq!(v.action, Action::Allow);
    }

    #[test]
    fn equal_block_and_warn_thresholds_score_at_boundary_blocks() {
        // degenerate: warn == block collapses the warn band; at the threshold it blocks
        let t = Thresholds { block: 50, warn: 50 };
        let v = evaluate(50, vec![], &t, None, None);
        assert_eq!(v.action, Action::Block);
    }

    #[test]
    fn equal_block_and_warn_thresholds_score_below_boundary_allows() {
        let t = Thresholds { block: 50, warn: 50 };
        let v = evaluate(49, vec![], &t, None, None);
        assert_eq!(v.action, Action::Allow);
    }
}
