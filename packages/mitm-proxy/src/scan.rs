use text_policy::{
    evaluator::Thresholds,
    lexicon::{Category, Dictionary, DictionaryTerm, LexiconBuilder, MatchMode, Severity},
    normalize::Language,
    policy::PolicyEngine,
    scorer::SourceKind,
    verdict::Action,
};

/// Outcome of a scan operation.
pub enum ScanResult {
    Allow,
    Block { score: u32 },
}

/// Build a `PolicyEngine` with a starter dictionary.
///
/// Terms are representative placeholders; a real implementation would load
/// dictionaries from a config file or embedded asset.
pub fn build_default_engine() -> PolicyEngine {
    let matcher = LexiconBuilder::new(Language::English)
        .add_dictionary(Dictionary::new(
            "adult-platforms",
            vec![
                DictionaryTerm::new(
                    "adult platform",
                    Category::AdultPlatform,
                    Severity::High,
                    vec![MatchMode::ExactPhrase, MatchMode::TokenSequence, MatchMode::UrlTokenSequence],
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
        .expect("built-in dictionary must be valid");

    PolicyEngine::new(matcher, Thresholds::default())
}

fn verdict_to_result(action: Action, score: u32) -> ScanResult {
    match action {
        Action::Block => ScanResult::Block { score },
        Action::Warn | Action::Blur | Action::Log | Action::Allow => ScanResult::Allow,
    }
}

pub fn scan_url(engine: &PolicyEngine, url: &str) -> ScanResult {
    let verdict = engine.evaluate(url, SourceKind::BrowserUrl);
    verdict_to_result(verdict.action, verdict.score)
}

pub fn scan_body(engine: &PolicyEngine, html: &str) -> ScanResult {
    let verdict = engine.evaluate(html, SourceKind::BrowserTitle);
    verdict_to_result(verdict.action, verdict.score)
}

/// Stub — returns `Allow` for every image until `packages/image-sandbox` is ready.
pub fn scan_image(_bytes: &[u8]) -> ScanResult {
    ScanResult::Allow
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> PolicyEngine {
        build_default_engine()
    }

    #[test]
    fn clean_url_is_allowed() {
        assert!(matches!(scan_url(&engine(), "https://example.com/path"), ScanResult::Allow));
    }

    #[test]
    fn clean_body_is_allowed() {
        assert!(matches!(scan_body(&engine(), "<html>hello world</html>"), ScanResult::Allow));
    }

    #[test]
    fn high_severity_url_term_blocks() {
        let e = engine();
        // "adult platform" with UrlTokenSequence should fire at BrowserUrl confidence
        let result = scan_url(&e, "https://adult-platform.example.com/");
        assert!(matches!(result, ScanResult::Block { .. }));
    }

    #[test]
    fn high_severity_body_term_blocks() {
        let e = engine();
        let result = scan_body(&e, "<html>explicit act shown here</html>");
        assert!(matches!(result, ScanResult::Block { .. }));
    }

    #[test]
    fn scan_image_allows_jpeg_bytes() {
        assert!(matches!(scan_image(&[0xFF, 0xD8, 0xFF]), ScanResult::Allow));
    }

    #[test]
    fn scan_image_allows_empty_input() {
        assert!(matches!(scan_image(&[]), ScanResult::Allow));
    }
}
