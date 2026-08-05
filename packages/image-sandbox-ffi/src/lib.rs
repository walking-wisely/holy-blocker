//! UniFFI surface over `image-sandbox`.
//!
//! Exists for the same reason `text-policy-ffi` does: an edge that is not Rust
//! — here the macOS daemon's Swift render path — needs the classifier without
//! reimplementing its geometry, its reduction, or its threshold. Everything in
//! this crate is type translation; no classification decision lives here.
//!
//! ## Why the raw-frame entry point and not `check`
//!
//! The screen path never holds an encoded image. `ScreenCaptureKit` delivers a
//! tightly packed BGRA framebuffer, so this crate exposes
//! `ImageSandbox::check_raw` and deliberately does *not* expose `check`: giving
//! Swift an encoded-bytes entry point would invite a caller to PNG-encode a
//! frame just to have it decoded again.
//!
//! ## The buffer crosses the boundary by copy
//!
//! UniFFI maps `Vec<u8>` onto Swift `Data` by copying. At a 1512x982 Retina
//! frame that is ~5.9 MB per call, on the daemon's ~500 ms image cadence. That
//! cost is accepted rather than designed around — a zero-copy surface would
//! mean handing Rust a raw pointer with a lifetime Swift cannot express, which
//! is a much worse trade for a memcpy measured in milliseconds.
//!
//! ## The runtime split this crate sits on one side of
//!
//! Per `docs/decisions/learning-from-feedback.md`, ONNX Runtime is the desktop
//! half and LiteRT is the Android half. This crate is therefore for macOS and
//! Windows only — `apps/mobile` must not be wired to it.

use std::path::PathBuf;
use std::sync::Arc;

use image_sandbox::{
    DEFAULT_EXPLICIT_THRESHOLD, ImageClassifier, ImageSandbox, ImageVerdict, PixelLayout,
    PreprocessConfig, SandboxConfig, ScoredVerdict,
};

uniffi::setup_scaffolding!();

/// Channel order of the caller's framebuffer. Mirrors `image_sandbox::PixelLayout`.
///
/// The macOS daemon is always `Bgra` — it sets `kCVPixelFormatType_32BGRA` on
/// its `SCStreamConfiguration` explicitly, because the macOS 26.5 default is
/// biplanar `420v` and cannot be read as packed pixels at all. `Rgba` exists so
/// the parameter is a real choice at the boundary rather than an assumption
/// buried on one side of it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, uniffi::Enum)]
pub enum FramePixelLayout {
    Bgra,
    Rgba,
}

impl From<FramePixelLayout> for PixelLayout {
    fn from(value: FramePixelLayout) -> Self {
        match value {
            FramePixelLayout::Bgra => PixelLayout::Bgra,
            FramePixelLayout::Rgba => PixelLayout::Rgba,
        }
    }
}

/// What the caller should do with the frame it just handed over.
///
/// An enum with fields rather than a record with a boolean, so the generated
/// Swift is an enum with associated values and the two cases cannot be confused
/// at a call site.
///
/// **`Allow` carries an optional score, and the option is load-bearing.**
/// `None` means nothing was classified at all — no model, an unreadable buffer,
/// a frame below the size floor, an inference fault. `Some(0.02)` means the
/// model ran and saw nothing. Collapsing those to a single "allow" would make a
/// silently broken image path indistinguishable from a clean screen, which is
/// exactly the failure the macOS daemon's first live pass spent a session on
/// with its capture path.
#[derive(Clone, Copy, Debug, PartialEq, uniffi::Enum)]
pub enum ImageOutcome {
    Allow { score: Option<f32> },
    Block { score: f32 },
}

impl From<ScoredVerdict> for ImageOutcome {
    fn from(value: ScoredVerdict) -> Self {
        match value.verdict {
            ImageVerdict::Allow => Self::Allow { score: value.score },
            ImageVerdict::Block { score } => Self::Block { score },
        }
    }
}

/// Construction failures. Classification itself is infallible across this
/// boundary by design — `image-sandbox` fails open internally, and turning that
/// into a thrown error would make every caller reimplement the fail-open rule.
#[derive(Debug, uniffi::Error)]
pub enum ImageGuardError {
    /// The model file could not be loaded: missing, unreadable, not a valid
    /// ONNX graph, or this crate was built without the `onnx` feature.
    Unavailable { reason: String },
}

// Written out rather than derived, so this crate adds no dependency the other
// FFI wrappers do not have — `image-sandbox` spells its errors the same way.
impl std::fmt::Display for ImageGuardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable { reason } => write!(f, "classifier unavailable: {reason}"),
        }
    }
}

impl std::error::Error for ImageGuardError {}

/// Handle held by the foreign caller for the process lifetime.
///
/// Construction loads and warms an ONNX session, which is expensive — build one
/// and reuse it, exactly as the Android service holds a single `PolicyEngine`
/// rather than one per event.
#[derive(uniffi::Object)]
pub struct ImageGuard {
    sandbox: ImageSandbox,
}

#[uniffi::export]
impl ImageGuard {
    /// Loads `model_path` and classifies against it.
    ///
    /// `threshold` of `None` uses the measured operating point for the shipped
    /// checkpoint under the tile-max geometry. Pass a value only to re-derive
    /// it: a threshold belongs to a model **and** a geometry, and reusing one
    /// across either change has already caused an error in this project twice.
    /// See `docs/decisions/classifier-operating-point.md`.
    #[uniffi::constructor]
    pub fn with_model(
        model_path: String,
        threshold: Option<f32>,
    ) -> Result<Arc<Self>, ImageGuardError> {
        let preprocess = PreprocessConfig::default();
        let classifier = ImageClassifier::load(&PathBuf::from(&model_path), preprocess.input_size)
            .map_err(|error| ImageGuardError::Unavailable {
            reason: format!("{model_path}: {error}"),
        })?;

        let config = SandboxConfig {
            explicit_threshold: threshold.unwrap_or(DEFAULT_EXPLICIT_THRESHOLD),
            preprocess,
        };
        Ok(Arc::new(Self {
            sandbox: ImageSandbox::new(classifier, config),
        }))
    }

    /// A guard with no model, which allows everything.
    ///
    /// Not a test double: it is what the daemon runs when no model has been
    /// provisioned, and it keeps behaviour identical to a build with no image
    /// path at all. The alternative — refusing to start — would take the whole
    /// daemon down, including the text path, over a missing optional file.
    #[uniffi::constructor]
    pub fn disabled() -> Arc<Self> {
        Arc::new(Self {
            sandbox: ImageSandbox::disabled(),
        })
    }

    /// Classify one tightly packed framebuffer.
    ///
    /// `pixels` must be `width * height * 4` bytes with **no row padding**. On
    /// macOS that is what `PixelBufferCopy.depad` produces; handing over a
    /// CoreVideo buffer with its native `bytesPerRow` still classifies as
    /// *something* rather than failing, which is why the de-padding stays on
    /// the Swift side where the stride is known.
    ///
    /// Never fails. Every internal error path — a short buffer, a frame below
    /// the size floor, an inference fault — returns `Allow` with **no score**,
    /// because none of them is evidence about what is on screen and none of
    /// them produced a number. A caller that logs the score can tell those
    /// apart from a frame the model actually looked at.
    pub fn classify_frame(
        &self,
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        layout: FramePixelLayout,
    ) -> ImageOutcome {
        self.sandbox
            .check_raw(&pixels, width, height, layout.into())
            .into()
    }

    /// The threshold this guard is operating at, so the caller can log the
    /// operating point beside a score instead of assuming the default.
    pub fn threshold(&self) -> f32 {
        self.sandbox.config().explicit_threshold
    }
}

/// The measured operating point, exposed so a caller can report it without
/// constructing a guard.
#[uniffi::export]
pub fn default_explicit_threshold() -> f32 {
    DEFAULT_EXPLICIT_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame large enough to clear the 96px size floor.
    fn frame(width: u32, height: u32) -> Vec<u8> {
        vec![0x80; (width * height) as usize * 4]
    }

    #[test]
    fn a_disabled_guard_allows_every_frame() {
        let guard = ImageGuard::disabled();

        assert_eq!(
            guard.classify_frame(frame(256, 256), 256, 256, FramePixelLayout::Bgra),
            ImageOutcome::Allow { score: None }
        );
    }

    #[test]
    fn a_missing_model_reports_why_rather_than_allowing_silently() {
        // The failure mode this constructor is fallible to prevent: a daemon
        // that reports "image scanning on" while classifying nothing.
        let Err(ImageGuardError::Unavailable { reason }) =
            ImageGuard::with_model("/nonexistent/model.onnx".into(), None)
        else {
            panic!("a missing model must not construct");
        };
        assert!(reason.contains("/nonexistent/model.onnx"), "reason: {reason}");
    }

    #[test]
    fn an_empty_frame_is_allowed_rather_than_crossing_the_boundary_as_an_error() {
        // The pre-first-frame state on the macOS side. Classification is
        // infallible across this boundary precisely so this needs no handling.
        let guard = ImageGuard::disabled();

        assert_eq!(
            guard.classify_frame(Vec::new(), 0, 0, FramePixelLayout::Bgra),
            ImageOutcome::Allow { score: None }
        );
    }

    #[test]
    fn a_disabled_guard_still_reports_the_measured_threshold() {
        // So a log line reads the same whether or not a model was provisioned.
        assert_eq!(ImageGuard::disabled().threshold(), DEFAULT_EXPLICIT_THRESHOLD);
    }

    #[test]
    fn the_exported_default_threshold_is_the_measured_tile_max_operating_point() {
        // Guards the same three wrong-but-plausible values as image-sandbox's
        // own test: 0.5 is argmax's default, 0.20 belongs to the superseded
        // unfreeze-3 checkpoint, 0.2717 to this checkpoint under a centre crop.
        assert_eq!(default_explicit_threshold(), 0.4650);
    }

    #[test]
    fn pixel_layout_maps_to_every_image_sandbox_variant() {
        // Guards against a variant being added on one side only.
        let cases = [
            (FramePixelLayout::Bgra, PixelLayout::Bgra),
            (FramePixelLayout::Rgba, PixelLayout::Rgba),
        ];
        for (ffi, native) in cases {
            assert_eq!(PixelLayout::from(ffi), native);
        }
    }

    #[test]
    fn outcome_maps_from_every_verdict_variant() {
        assert_eq!(
            ImageOutcome::from(ScoredVerdict {
                verdict: ImageVerdict::Block { score: 0.77 },
                score: Some(0.77)
            }),
            ImageOutcome::Block { score: 0.77 }
        );
    }

    #[test]
    fn an_allow_the_model_produced_keeps_its_score() {
        // The distinction the daemon logs: the model ran and saw 0.44, which is
        // just under the 0.4650 operating point. That margin is the only signal
        // that would show a threshold miscalibrated for screen content.
        assert_eq!(
            ImageOutcome::from(ScoredVerdict {
                verdict: ImageVerdict::Allow,
                score: Some(0.44)
            }),
            ImageOutcome::Allow { score: Some(0.44) }
        );
    }

    #[test]
    fn an_allow_that_never_reached_the_model_has_no_score() {
        // Not `Some(0.0)`. A broken image path must not read as a clean screen.
        assert_eq!(
            ImageOutcome::from(ScoredVerdict { verdict: ImageVerdict::Allow, score: None }),
            ImageOutcome::Allow { score: None }
        );
    }
}
