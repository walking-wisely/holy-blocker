use image_sandbox::{
    DEFAULT_EXPLICIT_THRESHOLD, ImageClassifier, ImageSandbox, ImageVerdict, SandboxConfig,
};
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

/// Classify an intercepted image body.
///
/// The score is `softmax(logits)[explicit]` from the MobileNetV3 classifier;
/// `image-sandbox` owns the threshold comparison. Scores are floats in [0, 1]
/// while `ScanResult::Block` carries an integer, so it is scaled to 0..100 —
/// the field is a diagnostic for logs, not an input to any decision.
///
/// A sandbox with no model loaded allows everything, which is what runs when
/// `--image-model` is not passed.
pub fn scan_image(sandbox: &ImageSandbox, bytes: &[u8]) -> ScanResult {
    match sandbox.check(bytes) {
        ImageVerdict::Block { score } => ScanResult::Block { score: (score * 100.0) as u32 },
        ImageVerdict::Allow => ScanResult::Allow,
    }
}

/// Load the image classifier, or fall back to a sandbox that allows everything.
///
/// A missing or unreadable model is reported and then tolerated rather than
/// being fatal: the proxy's other phases still work, and refusing to start
/// would take out URL and body filtering too.
pub fn build_image_sandbox(model_path: Option<&std::path::Path>, threshold: f32) -> ImageSandbox {
    let Some(path) = model_path else {
        return ImageSandbox::disabled();
    };
    match ImageClassifier::load(path, image_sandbox::preprocess::INPUT_SIZE) {
        Ok(classifier) => {
            tracing::info!(
                "image classifier loaded from {} (threshold {threshold})",
                path.display()
            );
            let config = SandboxConfig { explicit_threshold: threshold, ..SandboxConfig::default() };
            ImageSandbox::new(classifier, config)
        }
        Err(e) => {
            tracing::warn!("image classifier unavailable ({e}); images will not be scanned");
            ImageSandbox::disabled()
        }
    }
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
    fn scan_image_allows_truncated_jpeg_bytes() {
        let sandbox = build_image_sandbox(None, DEFAULT_EXPLICIT_THRESHOLD);
        assert!(matches!(scan_image(&sandbox, &[0xFF, 0xD8, 0xFF]), ScanResult::Allow));
    }

    #[test]
    fn scan_image_allows_empty_input() {
        let sandbox = build_image_sandbox(None, DEFAULT_EXPLICIT_THRESHOLD);
        assert!(matches!(scan_image(&sandbox, &[]), ScanResult::Allow));
    }

    #[test]
    fn without_a_model_path_images_are_not_scanned() {
        // The no-`--image-model` case must behave exactly as the proxy did
        // before this phase existed, so an operator who does not configure a
        // model sees no change at all.
        let sandbox = build_image_sandbox(None, DEFAULT_EXPLICIT_THRESHOLD);
        assert!(matches!(scan_image(&sandbox, b"anything at all"), ScanResult::Allow));
    }

    #[test]
    fn a_missing_model_file_degrades_to_allow_rather_than_panicking() {
        // A wrong path is an operator mistake. Taking down URL and body
        // filtering as well would turn it into an outage.
        let sandbox = build_image_sandbox(Some(std::path::Path::new("/nonexistent/model.onnx")), DEFAULT_EXPLICIT_THRESHOLD);
        assert!(matches!(scan_image(&sandbox, &[0xFF, 0xD8, 0xFF]), ScanResult::Allow));
    }
}
