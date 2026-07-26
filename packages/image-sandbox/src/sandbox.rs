//! Decode -> preprocess -> classify -> verdict.
//!
//! The single entry point `mitm-proxy` calls at its Phase 4 hook.
//!
//! ## Fail open, everywhere
//!
//! Every failure path returns `Allow`: an undecodable buffer, an image below
//! the size floor, an extreme aspect ratio, a missing model, an inference
//! error. None of those are evidence that an image is explicit, and blocking on
//! them would turn a malformed GIF or a missing file into a broken page. The
//! cost is that a bug in this crate degrades to "no filtering" rather than
//! "filter everything", which is the right direction for something sitting in
//! the path of all browser traffic.

use crate::classifier::{ClassifyResult, ImageClassifier};
use crate::preprocess::{PreprocessConfig, preprocess};

/// What the proxy should do with an image response body.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageVerdict {
    Allow,
    Block { score: f32 },
}

/// Score at or above which an image is blocked.
///
/// **Provisional, and known to be derived from a different model.** 0.20 is the
/// 5%-miss-budget operating point recorded in
/// `docs/decisions/classifier-operating-point.md`, measured on the *unfreeze-3*
/// checkpoint. The shipped artifact is the full-unfreeze model, whose 5% budget
/// costs 10.09% over-blocking — but the threshold that achieves it was never
/// recorded, and that decision doc states plainly that the threshold is
/// model-specific and must be re-derived.
///
/// Re-deriving it needs the corpus. Until then this is a starting point, not a
/// measured operating point, which is why `SandboxConfig` carries it as a field
/// rather than the code reading the constant directly.
pub const DEFAULT_EXPLICIT_THRESHOLD: f32 = 0.20;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SandboxConfig {
    pub explicit_threshold: f32,
    pub preprocess: PreprocessConfig,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            explicit_threshold: DEFAULT_EXPLICIT_THRESHOLD,
            preprocess: PreprocessConfig::default(),
        }
    }
}

pub struct ImageSandbox {
    classifier: Option<ImageClassifier>,
    config: SandboxConfig,
}

impl ImageSandbox {
    /// A sandbox with no classifier. Allows everything.
    ///
    /// Not a placeholder to be removed: it is what runs when no model path is
    /// configured, and it keeps the proxy's behaviour identical to today's.
    pub fn disabled() -> Self {
        Self { classifier: None, config: SandboxConfig::default() }
    }

    pub fn new(classifier: ImageClassifier, config: SandboxConfig) -> Self {
        Self { classifier: Some(classifier), config }
    }

    pub fn config(&self) -> &SandboxConfig {
        &self.config
    }

    /// Decode `bytes`, classify, and decide.
    pub fn check(&self, bytes: &[u8]) -> ImageVerdict {
        let Some(classifier) = &self.classifier else {
            return ImageVerdict::Allow;
        };

        let image = match image::load_from_memory(bytes) {
            Ok(image) => image,
            Err(error) => {
                // Truncated, unsupported, or not an image at all. Common enough
                // on real traffic that this is debug, not warn.
                tracing::debug!("image decode failed, allowing: {error}");
                return ImageVerdict::Allow;
            }
        };

        let tensor = match preprocess(&image, &self.config.preprocess) {
            Ok(tensor) => tensor,
            Err(reason) => {
                tracing::debug!("image not classified, allowing: {reason}");
                return ImageVerdict::Allow;
            }
        };

        match classifier.classify(&tensor) {
            Ok(ClassifyResult { explicit_score }) => {
                if explicit_score >= self.config.explicit_threshold {
                    ImageVerdict::Block { score: explicit_score }
                } else {
                    ImageVerdict::Allow
                }
            }
            Err(error) => {
                // An inference failure is a fault in us, not evidence about the
                // image. Warn, because unlike a decode failure it should not
                // happen on healthy traffic.
                tracing::warn!("image classification failed, allowing: {error}");
                ImageVerdict::Allow
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sandbox_without_a_model_allows_everything() {
        let sandbox = ImageSandbox::disabled();

        assert_eq!(sandbox.check(&[0xFF, 0xD8, 0xFF]), ImageVerdict::Allow);
        assert_eq!(sandbox.check(&[]), ImageVerdict::Allow);
    }

    #[test]
    fn undecodable_bytes_are_allowed_not_blocked() {
        let sandbox = ImageSandbox::disabled();

        assert_eq!(sandbox.check(b"<html>not an image</html>"), ImageVerdict::Allow);
    }

    #[test]
    fn the_default_threshold_is_the_recorded_miss_budget_operating_point() {
        // Guards against someone "fixing" this to 0.5, which the operating
        // point decision explicitly rejects: 0.5 is argmax's default, not a
        // measured point, and it misses far more than the budget allows.
        assert_eq!(SandboxConfig::default().explicit_threshold, 0.20);
    }

    #[test]
    fn the_threshold_is_configurable_so_it_can_be_re_derived() {
        let config = SandboxConfig { explicit_threshold: 0.44, ..SandboxConfig::default() };

        assert_eq!(config.explicit_threshold, 0.44);
    }
}
