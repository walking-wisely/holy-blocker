//! ONNX inference over the exported MobileNetV3 classifier.
//!
//! Behind the `onnx` feature. Without it the crate still compiles and the
//! sandbox degrades to allowing everything, so a build without an ONNX Runtime
//! is a functioning build rather than a broken one.
//!
//! ## The score contract
//!
//! Pinned by `machine-learning/src/holy_blocker_ml/labels.py` and
//! `eval.py:collect_predictions`, and every published threshold was derived
//! under it:
//!
//! - the graph output `logits` has shape `[batch, 2]`;
//! - index 0 is `safe`, index 1 is `explicit` (`BINARY_LABELS`, pinned there
//!   rather than derived from sorted directory names precisely so it cannot
//!   silently invert);
//! - the score is `softmax(logits)[1]`;
//! - block when that score is `>= threshold`.
//!
//! Inventing a different reduction here — argmax, a raw logit, index 0 — would
//! produce numbers that look like probabilities and mean something else.

/// A classifier verdict for one image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClassifyResult {
    /// `softmax(logits)[EXPLICIT_INDEX]`, in [0, 1].
    pub explicit_score: f32,
}

/// Output index of the `explicit` class. Mirrors `labels.POSITIVE_INDEX`.
pub const EXPLICIT_INDEX: usize = 1;

/// Number of classes the exported head emits. Mirrors `labels.BINARY_LABELS`.
pub const CLASS_COUNT: usize = 2;

/// Turn a row of logits into the positive-class probability.
///
/// Pure, so the contract above is testable without a model file. Subtracts the
/// row maximum before exponentiating, which is the standard guard against
/// overflow on large logits and changes no result.
pub fn explicit_score(logits: &[f32]) -> f32 {
    debug_assert_eq!(logits.len(), CLASS_COUNT);
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exponentiated: Vec<f32> = logits.iter().map(|v| (v - max).exp()).collect();
    let total: f32 = exponentiated.iter().sum();
    exponentiated[EXPLICIT_INDEX] / total
}

#[derive(Debug)]
pub enum ClassifierError {
    #[cfg(feature = "onnx")]
    Session(ort::Error),
    /// The graph did not produce the `[batch, 2]` output the contract requires.
    UnexpectedOutput(String),
    /// Built without the `onnx` feature.
    Unavailable,
}

impl std::fmt::Display for ClassifierError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(feature = "onnx")]
            Self::Session(e) => write!(f, "onnx runtime: {e}"),
            Self::UnexpectedOutput(what) => write!(f, "unexpected model output: {what}"),
            Self::Unavailable => {
                write!(f, "built without the `onnx` feature, so no model can be loaded")
            }
        }
    }
}

impl std::error::Error for ClassifierError {}

#[cfg(feature = "onnx")]
mod backend {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;

    /// A loaded ONNX session.
    ///
    /// `ort::Session::run` takes `&mut self`, so the session sits behind a
    /// mutex. That serialises inference across connections, which is the right
    /// default for a single small model: ONNX Runtime already parallelises
    /// within a call, and letting every connection build its own session would
    /// multiply the memory by the connection count.
    pub struct ImageClassifier {
        session: Mutex<ort::session::Session>,
        input_size: u32,
    }

    impl ImageClassifier {
        pub fn load(model_path: &Path, input_size: u32) -> Result<Self, ClassifierError> {
            let session = ort::session::Session::builder()
                .map_err(ClassifierError::Session)?
                .commit_from_file(model_path)
                .map_err(ClassifierError::Session)?;
            Ok(Self { session: Mutex::new(session), input_size })
        }

        /// Run one NCHW tensor of `3 * input_size * input_size` floats.
        pub fn classify(&self, tensor: &[f32]) -> Result<ClassifyResult, ClassifierError> {
            let expected = 3 * (self.input_size * self.input_size) as usize;
            if tensor.len() != expected {
                return Err(ClassifierError::UnexpectedOutput(format!(
                    "input has {} floats, expected {expected}",
                    tensor.len()
                )));
            }

            let shape = [1i64, 3, self.input_size as i64, self.input_size as i64];
            let value = ort::value::Tensor::from_array((shape, tensor.to_vec()))
                .map_err(ClassifierError::Session)?;

            let mut session = self.session.lock().expect("classifier mutex poisoned");
            let outputs = session
                .run(ort::inputs!["image" => value])
                .map_err(ClassifierError::Session)?;

            let (_, logits) = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(ClassifierError::Session)?;
            if logits.len() < CLASS_COUNT {
                return Err(ClassifierError::UnexpectedOutput(format!(
                    "{} values, expected {CLASS_COUNT}",
                    logits.len()
                )));
            }
            Ok(ClassifyResult { explicit_score: explicit_score(&logits[..CLASS_COUNT]) })
        }
    }
}

#[cfg(not(feature = "onnx"))]
mod backend {
    use super::*;
    use std::path::Path;

    /// Stub for builds without the `onnx` feature.
    pub struct ImageClassifier;

    impl ImageClassifier {
        pub fn load(_model_path: &Path, _input_size: u32) -> Result<Self, ClassifierError> {
            Err(ClassifierError::Unavailable)
        }

        pub fn classify(&self, _tensor: &[f32]) -> Result<ClassifyResult, ClassifierError> {
            Err(ClassifierError::Unavailable)
        }
    }
}

pub use backend::ImageClassifier;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_logits_split_the_probability_evenly() {
        assert!((explicit_score(&[0.0, 0.0]) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_dominant_explicit_logit_scores_near_one() {
        assert!(explicit_score(&[0.0, 10.0]) > 0.999);
    }

    #[test]
    fn a_dominant_safe_logit_scores_near_zero() {
        assert!(explicit_score(&[10.0, 0.0]) < 0.001);
    }

    #[test]
    fn the_positive_class_is_index_one_not_zero() {
        // If BINARY_LABELS were ever derived from sorted names, "explicit"
        // would sort to index 0 and every verdict would invert. This asserts
        // the direction, not just the arithmetic.
        assert!(explicit_score(&[0.0, 1.0]) > explicit_score(&[1.0, 0.0]));
    }

    #[test]
    fn large_logits_do_not_overflow_to_nan() {
        let score = explicit_score(&[1000.0, 1001.0]);

        assert!(score.is_finite(), "got {score}");
        assert!(score > 0.5);
    }

    #[test]
    fn scores_stay_within_the_unit_interval() {
        for pair in [[-50.0, 50.0], [50.0, -50.0], [0.0, 0.0], [3.5, -2.1]] {
            let score = explicit_score(&pair);
            assert!((0.0..=1.0).contains(&score), "{pair:?} gave {score}");
        }
    }

    #[cfg(not(feature = "onnx"))]
    #[test]
    fn loading_without_the_feature_reports_unavailable_rather_than_pretending() {
        let result = ImageClassifier::load(std::path::Path::new("/nonexistent.onnx"), 224);

        assert!(matches!(result, Err(ClassifierError::Unavailable)));
    }
}
