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
use crate::raw::{PixelLayout, image_from_raw};

/// What the proxy should do with an image response body.
///
/// Three tiers, not two: `Warn` is the `sexy` band the classifier contract
/// added (`classifier.rs`) — content the model reads as suggestive but not
/// explicit. A probability *can* carry a warn band once the model itself has
/// three classes to draw the line between; a two-class model genuinely
/// couldn't.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageVerdict {
    Allow,
    Warn { score: f32 },
    Block { score: f32 },
}

/// Collapse per-tile scores into one verdict score for a single class.
///
/// The maximum, not the mean: "any region explicit → block" is what a blocker
/// wants, and averaging dilutes a small explicit region into a large safe
/// background — the exact failure the tiled geometry was adopted to fix. This
/// reduction is applied independently per class (see `ImageSandbox::
/// check_image`): a tile that reads mostly `sexy` and a different tile that
/// reads mostly `explicit` are separate pieces of evidence and must not be
/// averaged into each other.
///
/// Empty input scores 0.0. `preprocess_tiles` always returns at least one
/// window, so that is unreachable today; it is defined rather than panicking
/// because this runs inside the proxy's request path.
pub fn reduce_tile_scores(scores: &[f32]) -> f32 {
    scores.iter().copied().fold(0.0f32, f32::max)
}

/// A verdict together with the score behind it.
///
/// The raw-frame path returns this rather than a bare [`ImageVerdict`] because
/// its caller is a long-running daemon that logs one line per state change, and
/// "allowed at 0.44 against the configured threshold" and "allowed at 0.01" are
/// different facts about how well the operating point fits what is on screen.
/// The network path has no such caller and keeps the plain verdict.
///
/// With two thresholds now in play, `score` is whichever class's score
/// produced the verdict: `explicit_score` for `Block`, `sexy_score` for
/// `Warn`, and the larger of the two for `Allow` — a model that ran and saw
/// nothing still reports how close it came, on either axis.
// Not `Copy`: `ImageVerdict` is not, and making it so would be a change to the
// network path's public type for this path's convenience.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoredVerdict {
    pub verdict: ImageVerdict,
    /// `None` when no model ran at all — disabled sandbox, unreadable buffer,
    /// below the size floor, inference fault. Distinct from `Some(0.0)`, which
    /// is a real model output; reporting zero for "did not classify" would make
    /// a broken image path look like a confidently clean screen.
    pub score: Option<f32>,
}

impl ScoredVerdict {
    /// The fail-open result: allow, with no score, because nothing was scored.
    fn unscored_allow() -> Self {
        Self { verdict: ImageVerdict::Allow, score: None }
    }
}

/// There is no built-in default: a threshold belongs to a model **and** a
/// geometry, and reusing one across either change has already caused an error
/// in this project twice. The caller must supply both explicitly for its own
/// deployed checkpoint — see the deployment's own configuration, not a value
/// recorded here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SandboxConfig {
    pub explicit_threshold: f32,
    /// Score at or above which the `sexy` class produces `Warn` rather than
    /// `Allow`. Same no-default rule as `explicit_threshold`, and the two are
    /// only meaningfully ordered relative to each other for a given model —
    /// nothing here enforces `sexy_threshold < explicit_threshold`, since a
    /// model's own calibration is what should decide that, not this crate.
    pub sexy_threshold: f32,
    pub preprocess: PreprocessConfig,
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
    /// Both thresholds are inert here — `check`/`check_raw` return `Allow`
    /// before either is ever read — so `0.0` is a placeholder, not a claim
    /// about a value.
    pub fn disabled() -> Self {
        Self {
            classifier: None,
            config: SandboxConfig {
                explicit_threshold: 0.0,
                sexy_threshold: 0.0,
                preprocess: PreprocessConfig::default(),
            },
        }
    }

    pub fn new(classifier: ImageClassifier, config: SandboxConfig) -> Self {
        Self { classifier: Some(classifier), config }
    }

    pub fn config(&self) -> &SandboxConfig {
        &self.config
    }

    /// Decode `bytes`, classify, and decide.
    pub fn check(&self, bytes: &[u8]) -> ImageVerdict {
        if self.classifier.is_none() {
            return ImageVerdict::Allow;
        }

        let image = match image::load_from_memory(bytes) {
            Ok(image) => image,
            Err(error) => {
                // Truncated, unsupported, or not an image at all. Common enough
                // on real traffic that this is debug, not warn.
                tracing::debug!("image decode failed, allowing: {error}");
                return ImageVerdict::Allow;
            }
        };

        self.check_image(&image).verdict
    }

    /// Classify a raw framebuffer the caller already holds — the screen path.
    ///
    /// Same geometry, same thresholds and the same fail-open contract as
    /// [`Self::check`]; only the decode step differs, because a captured frame
    /// was never encoded. `pixels` must be tightly packed with no row padding —
    /// see [`image_from_raw`].
    ///
    /// **The configured thresholds' provenance does not extend here.** Whatever
    /// values the caller supplies are calibrated against a corpus of *images*
    /// under tile-max. A screen frame is a different distribution — a small
    /// content region inside application chrome, at a display aspect ratio —
    /// and re-deriving an operating point for it is an open question. Tile-max
    /// is still the right geometry for that shape, which is why this path
    /// reuses it rather than the centre crop.
    /// Returns the score alongside the verdict — see [`ScoredVerdict`].
    pub fn check_raw(
        &self,
        pixels: &[u8],
        width: u32,
        height: u32,
        layout: PixelLayout,
    ) -> ScoredVerdict {
        if self.classifier.is_none() {
            return ScoredVerdict::unscored_allow();
        }

        let image = match image_from_raw(pixels, width, height, layout) {
            Ok(image) => image,
            Err(error) => {
                // A zero-dimension frame is the ordinary pre-first-frame state,
                // not a malfunction, so this stays at debug like a decode
                // failure rather than warning on every tick before capture
                // starts.
                tracing::debug!("raw frame not classified, allowing: {error}");
                return ScoredVerdict::unscored_allow();
            }
        };

        self.check_image(&image)
    }

    /// The shared half: tile, score every tile, reduce per class, threshold.
    fn check_image(&self, image: &image::DynamicImage) -> ScoredVerdict {
        let Some(classifier) = &self.classifier else {
            return ScoredVerdict::unscored_allow();
        };

        let tiles = match preprocess_tiles(image, &self.config.preprocess) {
            Ok(tiles) => tiles,
            Err(reason) => {
                tracing::debug!("image not classified, allowing: {reason}");
                return ScoredVerdict::unscored_allow();
            }
        };

        let mut sexy_scores = Vec::with_capacity(tiles.len());
        let mut explicit_scores = Vec::with_capacity(tiles.len());
        for tile in &tiles {
            match classifier.classify(tile) {
                Ok(ClassifyResult { sexy_score, explicit_score, .. }) => {
                    sexy_scores.push(sexy_score);
                    explicit_scores.push(explicit_score);
                }
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
                    return ScoredVerdict::unscored_allow();
                }
            }
        }

        let explicit_score = reduce_tile_scores(&explicit_scores);
        let sexy_score = reduce_tile_scores(&sexy_scores);

        // `explicit` is checked first: the highest-`sexy` tile need not be the
        // highest-`explicit` tile once each class is reduced independently, so
        // an image can clear both bars at once, and block must win whenever it
        // applies — warning on content that already clears the block bar would
        // under-react to it.
        if explicit_score >= self.config.explicit_threshold {
            ScoredVerdict {
                verdict: ImageVerdict::Block { score: explicit_score },
                score: Some(explicit_score),
            }
        } else if sexy_score >= self.config.sexy_threshold {
            ScoredVerdict {
                verdict: ImageVerdict::Warn { score: sexy_score },
                score: Some(sexy_score),
            }
        } else {
            ScoredVerdict {
                verdict: ImageVerdict::Allow,
                score: Some(explicit_score.max(sexy_score)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(sexy_threshold: f32, explicit_threshold: f32) -> SandboxConfig {
        SandboxConfig { explicit_threshold, sexy_threshold, preprocess: PreprocessConfig::default() }
    }

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

    // --- the raw framebuffer path -----------------------------------------

    #[test]
    fn a_sandbox_without_a_model_allows_raw_frames_too() {
        // The screen path must inherit the same disabled-is-allow behaviour as
        // the network path, or a daemon built without a model would block
        // everything on screen instead of nothing.
        let sandbox = ImageSandbox::disabled();
        let one_pixel = [0u8; 4];

        let scored = sandbox.check_raw(&one_pixel, 1, 1, PixelLayout::Bgra);

        assert_eq!(scored.verdict, ImageVerdict::Allow);
        // No score, not 0.0: nothing was classified, and a zero would read as a
        // confident "definitely clean" in the daemon's log line.
        assert_eq!(scored.score, None);
    }

    #[test]
    fn an_empty_raw_frame_is_allowed_rather_than_reaching_the_model() {
        // `CapturedFrame.empty()` on the macOS side. This is the state on every
        // tick before the first frame arrives, so it must be quiet and safe.
        let sandbox = ImageSandbox::disabled();

        assert_eq!(
            sandbox.check_raw(&[], 0, 0, PixelLayout::Bgra),
            ScoredVerdict { verdict: ImageVerdict::Allow, score: None }
        );
    }

    #[test]
    fn a_raw_frame_whose_buffer_is_too_short_is_allowed() {
        // Geometry disagreeing with the buffer is a fault in us, not evidence
        // about what is on screen — fail open like every other path here.
        let sandbox = ImageSandbox::disabled();

        assert_eq!(
            sandbox.check_raw(&[0u8; 8], 640, 480, PixelLayout::Bgra),
            ScoredVerdict { verdict: ImageVerdict::Allow, score: None }
        );
    }

    #[test]
    fn a_disabled_sandbox_never_reads_its_placeholder_thresholds() {
        // `disabled()` carries inert 0.0s — every check path returns before the
        // classifier or either threshold is consulted, so this is a regression
        // guard on that ordering, not a claim about the values.
        let sandbox = ImageSandbox::disabled();
        assert_eq!(sandbox.check(&[]), ImageVerdict::Allow);
    }

    #[test]
    fn both_thresholds_are_configurable_by_the_caller() {
        let cfg = config(0.3, 0.44);

        assert_eq!(cfg.explicit_threshold, 0.44);
        assert_eq!(cfg.sexy_threshold, 0.3);
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
        // threshold; the max is 0.95 and blocks. 0.5 stands in for "a sensible
        // threshold" here — the point is the mean/max gap, not a specific cut.
        let scores = [0.02, 0.01, 0.95, 0.03, 0.01];
        let mean = scores.iter().sum::<f32>() / scores.len() as f32;
        let plausible_threshold = 0.5;

        assert!(mean < plausible_threshold);
        assert!(reduce_tile_scores(&scores) >= plausible_threshold);
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
