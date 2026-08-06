use image_sandbox::{ImageClassifier, ImageSandbox, ImageVerdict, SandboxConfig};
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
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

/// Controls how scan verdicts are translated into proxy actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtectionMode {
    /// Block verdicts sever the connection.
    Full,
    /// Scans run and events are emitted, but verdicts are downgraded to Allow.
    WarnOnly,
    /// Scan closures short-circuit before calling the engine.
    Off,
}

impl ProtectionMode {
    const FULL: u8 = 0;
    const WARN_ONLY: u8 = 1;
    const OFF: u8 = 2;

    pub fn as_u8(self) -> u8 {
        match self {
            Self::Full => Self::FULL,
            Self::WarnOnly => Self::WARN_ONLY,
            Self::Off => Self::OFF,
        }
    }

    pub fn to_atomic(self) -> Arc<AtomicU8> {
        Arc::new(AtomicU8::new(self.as_u8()))
    }

    pub fn from_atomic(cell: &AtomicU8) -> Self {
        match cell.load(Ordering::Relaxed) {
            Self::WARN_ONLY => Self::WarnOnly,
            Self::OFF => Self::Off,
            _ => Self::Full,
        }
    }

    /// Store a new mode into a shared atomic cell.
    /// Accepted by IPC handlers so they don't duplicate the u8 encoding.
    pub fn store(cell: &AtomicU8, mode: Self) {
        cell.store(mode.as_u8(), Ordering::Relaxed);
    }
}

/// Map an engine `Action` + score to a `ScanResult` according to the active mode.
pub fn apply_mode(mode: ProtectionMode, action: Action, score: u32) -> ScanResult {
    match (mode, action) {
        (ProtectionMode::Full, Action::Block) => ScanResult::Block { score },
        _ => ScanResult::Allow,
    }
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

pub fn scan_url(engine: &PolicyEngine, url: &str, mode: ProtectionMode) -> ScanResult {
    if mode == ProtectionMode::Off {
        return ScanResult::Allow;
    }
    let verdict = engine.evaluate(url, SourceKind::BrowserUrl);
    apply_mode(mode, verdict.action, verdict.score)
}

pub fn scan_body(engine: &PolicyEngine, html: &str, mode: ProtectionMode) -> ScanResult {
    if mode == ProtectionMode::Off {
        return ScanResult::Allow;
    }
    let verdict = engine.evaluate(html, SourceKind::BrowserTitle);
    apply_mode(mode, verdict.action, verdict.score)
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
            let config = SandboxConfig {
                explicit_threshold: threshold,
                preprocess: image_sandbox::PreprocessConfig::default(),
            };
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
        assert!(matches!(
            scan_url(&engine(), "https://example.com/path", ProtectionMode::Full),
            ScanResult::Allow
        ));
    }

    #[test]
    fn clean_body_is_allowed() {
        assert!(matches!(
            scan_body(&engine(), "<html>hello world</html>", ProtectionMode::Full),
            ScanResult::Allow
        ));
    }

    #[test]
    fn high_severity_url_term_blocks() {
        let e = engine();
        // "adult platform" with UrlTokenSequence should fire at BrowserUrl confidence
        let result = scan_url(&e, "https://adult-platform.example.com/", ProtectionMode::Full);
        assert!(matches!(result, ScanResult::Block { .. }));
    }

    #[test]
    fn high_severity_body_term_blocks() {
        let e = engine();
        let result = scan_body(&e, "<html>explicit act shown here</html>", ProtectionMode::Full);
        assert!(matches!(result, ScanResult::Block { .. }));
    }

    // A placeholder, not a measured value: every case below has model_path
    // as None, or a missing file, so the threshold is never read.
    const UNUSED_THRESHOLD: f32 = 0.0;

    #[test]
    fn scan_image_allows_truncated_jpeg_bytes() {
        let sandbox = build_image_sandbox(None, UNUSED_THRESHOLD);
        assert!(matches!(scan_image(&sandbox, &[0xFF, 0xD8, 0xFF]), ScanResult::Allow));
    }

    #[test]
    fn scan_image_allows_empty_input() {
        let sandbox = build_image_sandbox(None, UNUSED_THRESHOLD);
        assert!(matches!(scan_image(&sandbox, &[]), ScanResult::Allow));
    }

    #[test]
    fn without_a_model_path_images_are_not_scanned() {
        // The no-`--image-model` case must behave exactly as the proxy did
        // before this phase existed, so an operator who does not configure a
        // model sees no change at all.
        let sandbox = build_image_sandbox(None, UNUSED_THRESHOLD);
        assert!(matches!(scan_image(&sandbox, b"anything at all"), ScanResult::Allow));
    }

    #[test]
    fn a_missing_model_file_degrades_to_allow_rather_than_panicking() {
        // A wrong path is an operator mistake. Taking down URL and body
        // filtering as well would turn it into an outage.
        let sandbox = build_image_sandbox(
            Some(std::path::Path::new("/nonexistent/model.onnx")),
            UNUSED_THRESHOLD,
        );
        assert!(matches!(scan_image(&sandbox, &[0xFF, 0xD8, 0xFF]), ScanResult::Allow));
    }

    // apply_mode tests
    #[test]
    fn apply_mode_full_block() {
        assert!(matches!(
            apply_mode(ProtectionMode::Full, Action::Block, 90),
            ScanResult::Block { score: 90 }
        ));
    }

    #[test]
    fn apply_mode_warn_only_downgrades_block() {
        assert!(matches!(
            apply_mode(ProtectionMode::WarnOnly, Action::Block, 90),
            ScanResult::Allow
        ));
    }

    #[test]
    fn apply_mode_off_downgrades_block() {
        assert!(matches!(
            apply_mode(ProtectionMode::Off, Action::Block, 90),
            ScanResult::Allow
        ));
    }

    #[test]
    fn apply_mode_full_warn_is_allow() {
        assert!(matches!(
            apply_mode(ProtectionMode::Full, Action::Warn, 60),
            ScanResult::Allow
        ));
    }

    #[test]
    fn warn_only_url_does_not_block() {
        let e = engine();
        let result = scan_url(&e, "https://adult-platform.example.com/", ProtectionMode::WarnOnly);
        assert!(matches!(result, ScanResult::Allow));
    }

    #[test]
    fn off_mode_short_circuits_url_scan() {
        let e = engine();
        let result = scan_url(&e, "https://adult-platform.example.com/", ProtectionMode::Off);
        assert!(matches!(result, ScanResult::Allow));
    }

    #[test]
    fn protection_mode_atomic_roundtrip() {
        let cell = ProtectionMode::WarnOnly.to_atomic();
        assert_eq!(ProtectionMode::from_atomic(&cell), ProtectionMode::WarnOnly);
        ProtectionMode::store(&cell, ProtectionMode::Off);
        assert_eq!(ProtectionMode::from_atomic(&cell), ProtectionMode::Off);
    }

    #[test]
    fn store_updates_shared_cell() {
        let cell = ProtectionMode::Full.to_atomic();
        ProtectionMode::store(&cell, ProtectionMode::WarnOnly);
        assert_eq!(ProtectionMode::from_atomic(&cell), ProtectionMode::WarnOnly);
        ProtectionMode::store(&cell, ProtectionMode::Off);
        assert_eq!(ProtectionMode::from_atomic(&cell), ProtectionMode::Off);
        ProtectionMode::store(&cell, ProtectionMode::Full);
        assert_eq!(ProtectionMode::from_atomic(&cell), ProtectionMode::Full);
    }
}
