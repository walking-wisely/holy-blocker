use crate::lexicon::{Category, MatchSpan, Severity};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Block,
    Blur,
    Warn,
    Log,
    Allow,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EvidenceItem {
    pub rule_id: String,
    pub category: Category,
    pub severity: Severity,
    pub span: MatchSpan,
    pub base_score: u32,
    pub multiplier: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Verdict {
    pub action: Action,
    pub score: u32,
    pub evidence: Vec<EvidenceItem>,
}
