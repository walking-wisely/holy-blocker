//! ONNX inference over the exported classifier.
//!
//! Behind the `onnx` feature. Without it the crate still compiles and the
//! sandbox degrades to allowing everything, so a build without an ONNX Runtime
//! is a functioning build rather than a broken one.
//!
//! ## The score contract
//!
//! **Three classes, not two.** The binary `safe`/`explicit` contract missed a
//! real failure mode measured live: a model with no middle tier either blocks
//! ordinary photos it keys on the wrong cue for, or misses suggestive content
//! it was never trained to name, with no dial in between. `docs/decisions/
//! classifier-operating-point.md` records the model swap this generalises for.
//!
//! - the graph output `logits` has shape `[batch, 3]`;
//! - index 0 is `safe`, index 1 is `sexy`, index 2 is `explicit`
//!   (`SAFE_INDEX`/`SEXY_INDEX`/`EXPLICIT_INDEX`, named rather than derived
//!   from sorted directory names precisely so the order cannot silently
//!   invert);
//! - scores are `softmax(logits)`, one probability per class, summing to 1;
//! - `sandbox.rs` compares the `sexy` and `explicit` probabilities against
//!   their own thresholds independently — a tile can be simultaneously "not
//!   explicit enough to block" and "sexy enough to warn".
//!
//! Inventing a different reduction here — argmax, a raw logit, index 0 — would
//! produce numbers that look like probabilities and mean something else.

/// A classifier verdict for one image. All three are `softmax(logits)` at
/// their respective index, so `safe_score + sexy_score + explicit_score == 1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClassifyResult {
    pub safe_score: f32,
    pub sexy_score: f32,
    pub explicit_score: f32,
}

/// Output index of the `safe` class.
pub const SAFE_INDEX: usize = 0;
/// Output index of the `sexy` (suggestive, non-explicit) class.
pub const SEXY_INDEX: usize = 1;
/// Output index of the `explicit` class.
pub const EXPLICIT_INDEX: usize = 2;

/// Number of classes the exported head emits.
pub const CLASS_COUNT: usize = 3;

/// Turn a row of logits into one probability per class.
///
/// Pure, so the contract above is testable without a model file. Subtracts the
/// row maximum before exponentiating, which is the standard guard against
/// overflow on large logits and changes no result.
pub fn class_scores(logits: &[f32]) -> ClassifyResult {
    debug_assert_eq!(logits.len(), CLASS_COUNT);
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exponentiated: Vec<f32> = logits.iter().map(|v| (v - max).exp()).collect();
    let total: f32 = exponentiated.iter().sum();
    ClassifyResult {
        safe_score: exponentiated[SAFE_INDEX] / total,
        sexy_score: exponentiated[SEXY_INDEX] / total,
        explicit_score: exponentiated[EXPLICIT_INDEX] / total,
    }
}

#[derive(Debug)]
pub enum ClassifierError {
    #[cfg(feature = "onnx")]
    Session(ort::Error),
    /// The graph did not produce the `[batch, 3]` output the contract requires.
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
            Ok(class_scores(&logits[..CLASS_COUNT]))
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
    fn equal_logits_split_the_probability_evenly_across_three_classes() {
        let result = class_scores(&[0.0, 0.0, 0.0]);
        assert!((result.safe_score - 1.0 / 3.0).abs() < 1e-6);
        assert!((result.sexy_score - 1.0 / 3.0).abs() < 1e-6);
        assert!((result.explicit_score - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn a_dominant_explicit_logit_scores_near_one_on_explicit_only() {
        let result = class_scores(&[0.0, 0.0, 10.0]);
        assert!(result.explicit_score > 0.999);
        assert!(result.sexy_score < 0.001);
        assert!(result.safe_score < 0.001);
    }

    #[test]
    fn a_dominant_sexy_logit_scores_near_one_on_sexy_only() {
        // The tier this contract exists to add: high on sexy without also
        // reading as explicit.
        let result = class_scores(&[0.0, 10.0, 0.0]);
        assert!(result.sexy_score > 0.999);
        assert!(result.explicit_score < 0.001);
    }

    #[test]
    fn a_dominant_safe_logit_scores_near_zero_on_the_others() {
        let result = class_scores(&[10.0, 0.0, 0.0]);
        assert!(result.safe_score > 0.999);
        assert!(result.sexy_score < 0.001);
        assert!(result.explicit_score < 0.001);
    }

    #[test]
    fn the_three_scores_always_sum_to_one() {
        for logits in [[0.0, 1.0, 2.0], [-5.0, 3.0, 0.5], [10.0, -10.0, 0.0]] {
            let result = class_scores(&logits);
            let total = result.safe_score + result.sexy_score + result.explicit_score;
            assert!((total - 1.0).abs() < 1e-5, "{logits:?} summed to {total}");
        }
    }

    #[test]
    fn class_order_is_safe_then_sexy_then_explicit_not_alphabetical() {
        // Alphabetical would put "explicit" before "safe" before "sexy". If the
        // index constants were ever derived that way every verdict would
        // silently point at the wrong class. Assert the direction, not just
        // that scores differ.
        let result = class_scores(&[0.0, 1.0, 2.0]);
        assert!(result.explicit_score > result.sexy_score);
        assert!(result.sexy_score > result.safe_score);
    }

    #[test]
    fn large_logits_do_not_overflow_to_nan() {
        let result = class_scores(&[1000.0, 1000.5, 1001.0]);
        assert!(result.safe_score.is_finite());
        assert!(result.sexy_score.is_finite());
        assert!(result.explicit_score.is_finite());
        assert!(result.explicit_score > result.safe_score);
    }

    #[test]
    fn every_score_stays_within_the_unit_interval() {
        for logits in [[-50.0, 0.0, 50.0], [50.0, 0.0, -50.0], [0.0, 0.0, 0.0], [3.5, -2.1, 1.0]] {
            let result = class_scores(&logits);
            for score in [result.safe_score, result.sexy_score, result.explicit_score] {
                assert!((0.0..=1.0).contains(&score), "{logits:?} gave {score}");
            }
        }
    }

    #[cfg(not(feature = "onnx"))]
    #[test]
    fn loading_without_the_feature_reports_unavailable_rather_than_pretending() {
        let result = ImageClassifier::load(std::path::Path::new("/nonexistent.onnx"), 224);

        assert!(matches!(result, Err(ClassifierError::Unavailable)));
    }
}
