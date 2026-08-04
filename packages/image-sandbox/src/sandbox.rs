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
use crate::preprocess::{PreprocessConfig, preprocess_tiles};

/// What the proxy should do with an image response body.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageVerdict {
    Allow,
    Block { score: f32 },
}

/// Score at or above which an image is blocked.
///
/// **Measured**, for the shipped full-unfreeze checkpoint *under the tile-max
/// geometry*: the threshold achieving the 5% miss budget, at a cost of 10.09%
/// over-blocking. From
/// `docs/components/machine-learning/experiments/input-handling.md`, recorded in
/// `docs/decisions/classifier-operating-point.md`.
///
/// A threshold belongs to a model **and** a geometry, and both halves have
/// already caused an error here. The same checkpoint under a centre crop
/// operates at 0.2717; the superseded unfreeze-3 checkpoint operated at 0.20,
/// which is what this constant wrongly held before the corpus was available.
/// Taking a max over overlapping tiles shifts the whole score distribution
/// upward, so reusing a centre-crop threshold would over-block by roughly half
/// again (14.73% against 10.09%) with nothing failing to indicate it.
pub const DEFAULT_EXPLICIT_THRESHOLD: f32 = 0.4650;

/// Collapse per-tile scores into one verdict score.
///
/// The maximum, not the mean: "any region explicit → block" is what a blocker
/// wants, and averaging dilutes a small explicit region into a large safe
/// background — the exact failure the tiled geometry was adopted to fix.
///
/// Empty input scores 0.0. `preprocess_tiles` always returns at least one
/// window, so that is unreachable today; it is defined rather than panicking
/// because this runs inside the proxy's request path.
pub fn reduce_tile_scores(scores: &[f32]) -> f32 {
    scores.iter().copied().fold(0.0f32, f32::max)
}

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

        let tiles = match preprocess_tiles(&image, &self.config.preprocess) {
            Ok(tiles) => tiles,
            Err(reason) => {
                tracing::debug!("image not classified, allowing: {reason}");
                return ImageVerdict::Allow;
            }
        };

        let mut scores = Vec::with_capacity(tiles.len());
        for tile in &tiles {
            match classifier.classify(tile) {
                Ok(ClassifyResult { explicit_score }) => scores.push(explicit_score),
                Err(error) => {
                    // An inference failure is a fault in us, not evidence about
                    // the image. Warn, because unlike a decode failure it should
                    // not happen on healthy traffic.
                    //
                    // Abandon the whole image rather than reducing over the
                    // tiles that did succeed: a max over a subset is a score for
                    // a different image, and it would silently under-block
                    // exactly when something is already wrong.
                    tracing::warn!("image classification failed, allowing: {error}");
                    return ImageVerdict::Allow;
                }
            }
        }

        let explicit_score = reduce_tile_scores(&scores);
        if explicit_score >= self.config.explicit_threshold {
            ImageVerdict::Block { score: explicit_score }
        } else {
            ImageVerdict::Allow
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
    fn the_default_threshold_is_the_measured_tile_max_operating_point() {
        // Guards two separate mistakes. 0.5 is argmax's default rather than a
        // measured point, and the operating-point decision rejects it outright.
        // 0.20 and 0.2717 are also wrong here but far more plausible-looking:
        // the first belongs to the superseded unfreeze-3 model, the second to
        // this model under the *centre-crop* geometry. A threshold is only
        // valid for one model-and-geometry pairing.
        assert_eq!(SandboxConfig::default().explicit_threshold, 0.4650);
    }

    #[test]
    fn the_threshold_is_configurable_so_it_can_be_re_derived() {
        let config = SandboxConfig { explicit_threshold: 0.44, ..SandboxConfig::default() };

        assert_eq!(config.explicit_threshold, 0.44);
    }

    // --- reducing tile scores ---------------------------------------------

    #[test]
    fn the_reduction_is_the_maximum_over_tiles() {
        // "Any region explicit -> block" is the semantics the geometry was
        // adopted under.
        assert_eq!(reduce_tile_scores(&[0.01, 0.92, 0.03]), 0.92);
    }

    #[test]
    fn a_mean_reduction_would_dilute_a_single_explicit_tile() {
        // The failure tiling exists to fix, stated as a test: one explicit tile
        // in a wide safe banner. The mean is 0.24 and would clear no sensible
        // threshold; the max is 0.95 and blocks.
        let scores = [0.02, 0.01, 0.95, 0.03, 0.01];
        let mean = scores.iter().sum::<f32>() / scores.len() as f32;

        assert!(mean < SandboxConfig::default().explicit_threshold);
        assert!(reduce_tile_scores(&scores) >= SandboxConfig::default().explicit_threshold);
    }

    #[test]
    fn a_single_tile_reduces_to_itself() {
        // Near-square images take this path, and it must not perturb the score
        // — the measured plain-image AUC of 0.9806 assumes exactly this.
        assert_eq!(reduce_tile_scores(&[0.37]), 0.37);
    }

    #[test]
    fn no_tiles_scores_zero_rather_than_panicking() {
        // Unreachable via preprocess_tiles, which always yields at least one
        // window — but the reduction must not be the thing that panics inside
        // the proxy's request path if that ever stops being true.
        assert_eq!(reduce_tile_scores(&[]), 0.0);
    }
}
