//! On-device image classification for the Phase 4 network hook.
//!
//! Decodes an intercepted image response, reshapes it into the tensor the
//! MobileNetV3 classifier expects, and returns an allow/block verdict.
//!
//! ## The geometry
//!
//! An image is **tiled**, not centre-cropped: the shorter side goes to 224 and
//! overlapping 224 windows slide along the longer one, each scored
//! independently, and the **maximum** is the verdict score. The evaluation
//! centre crop discards ~23% of a 3:1 image, and measurement showed that costs
//! real coverage — on off-centre composites it caught 41% of explicit content
//! against tiling's 62%, at no cost on ordinary near-square images, which yield
//! a single tile. See
//! `docs/components/machine-learning/experiments/input-handling.md`.
//!
//! `preprocess` still implements the centre crop, because every published
//! figure was measured under it and `tests/parity.rs` pins it against
//! torchvision. `preprocess_tiles` is what ships.
//!
//! ## What is not here yet
//!
//! - **Perceptual hashing and the SQLite blocklist.** The original plan built
//!   these first, but no hash database exists or can be populated in-repo, so
//!   the path blocks nothing on day one. It is a cache in front of the model,
//!   not a prerequisite for it.
//! - **A fully convolutional equivalent.** MobileNetV3's
//!   `AdaptiveAvgPool2d(1) → Linear → Linear` head converts mechanically to 1x1
//!   convolutions with identical weights, turning N tile passes into one pass
//!   over a larger input whose spatial max equals the tiled max. That would also
//!   yield a coarse heatmap — the localisation the screen-capture path needs.

pub mod classifier;
pub mod preprocess;
pub mod raw;
pub mod sandbox;

pub use classifier::{ClassifierError, ClassifyResult, ImageClassifier, explicit_score};
pub use preprocess::{
    PreprocessConfig, PreprocessError, preprocess, preprocess_tiles, tile_starts,
};
pub use raw::{PixelLayout, RawFrameError, image_from_raw};
pub use sandbox::{
    DEFAULT_EXPLICIT_THRESHOLD, ImageSandbox, ImageVerdict, SandboxConfig, ScoredVerdict,
    reduce_tile_scores,
};
