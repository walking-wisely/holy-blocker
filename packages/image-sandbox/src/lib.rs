//! On-device image classification for the Phase 4 network hook.
//!
//! Decodes an intercepted image response, reshapes it into the tensor the
//! MobileNetV3 classifier expects, and returns an allow/block verdict.
//!
//! ## What is not here yet
//!
//! - **Perceptual hashing and the SQLite blocklist.** The original plan built
//!   these first, but no hash database exists or can be populated in-repo, so
//!   the path blocks nothing on day one. It is a cache in front of the model,
//!   not a prerequisite for it.
//! - **A measured size floor and geometry.** `preprocess` carries a provisional
//!   32px floor and reproduces the evaluation centre-crop. Both are being
//!   measured by `holy_blocker_ml.inputs`; the centre crop in particular
//!   discards ~23% of a wide image, so content in the side of a banner is
//!   currently never seen.
//! - **A model-specific threshold.** See `sandbox::DEFAULT_EXPLICIT_THRESHOLD`.

pub mod classifier;
pub mod preprocess;
pub mod sandbox;

pub use classifier::{ClassifierError, ClassifyResult, ImageClassifier, explicit_score};
pub use preprocess::{PreprocessConfig, PreprocessError, preprocess};
pub use sandbox::{DEFAULT_EXPLICIT_THRESHOLD, ImageSandbox, ImageVerdict, SandboxConfig};
