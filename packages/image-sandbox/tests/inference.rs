//! End-to-end inference against the real exported model.
//!
//! Skipped unless the `onnx` feature is on *and* the artifact exists, because
//! `data/models/` is gitignored — a fresh clone has no model and must still
//! have a green test run.
//!
//! Export it with, from the repository root:
//!
//! ```text
//! holy-blocker-export --checkpoint machine-learning/artifacts/unfreeze-full/finetuned-v0.pt \
//!   --out data/models/baseline-v0.onnx
//! ```

#![cfg(feature = "onnx")]

use image_sandbox::{ImageClassifier, ImageSandbox, ImageVerdict, SandboxConfig};
use std::path::{Path, PathBuf};

fn model_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/models/baseline-v0.onnx")
}

fn sandbox_or_skip() -> Option<ImageSandbox> {
    let path = model_path();
    if !path.is_file() {
        eprintln!("skipping: no model at {}", path.display());
        return None;
    }
    let classifier = ImageClassifier::load(&path, 224).expect("model loads");
    Some(ImageSandbox::new(classifier, SandboxConfig::default()))
}

#[test]
fn the_exported_model_loads_and_scores_a_real_image() {
    let Some(sandbox) = sandbox_or_skip() else { return };

    // The parity fixture is a synthetic gradient — the assertion is that a
    // verdict comes back at all, not what it is. What the model says about a
    // sine pattern is not a claim this crate should be making.
    let bytes = include_bytes!("fixtures/gradient.png");

    let verdict = sandbox.check(bytes);

    match verdict {
        ImageVerdict::Allow => {}
        ImageVerdict::Block { score } => {
            assert!((0.0..=1.0).contains(&score), "score out of range: {score}");
        }
    }
}

#[test]
fn scores_are_deterministic_across_calls() {
    let Some(sandbox) = sandbox_or_skip() else { return };
    let bytes = include_bytes!("fixtures/gradient.png");

    assert_eq!(sandbox.check(bytes), sandbox.check(bytes));
}

#[test]
fn an_image_below_the_size_floor_is_allowed_without_reaching_the_model() {
    let Some(sandbox) = sandbox_or_skip() else { return };

    // A 1x1 transparent PNG — the tracking pixel the floor exists to skip.
    let pixel: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    assert_eq!(sandbox.check(pixel), ImageVerdict::Allow);
}

#[test]
fn a_threshold_of_zero_blocks_and_of_one_allows() {
    // Pins the comparison direction end to end: with a real model in the loop,
    // an impossible-to-clear threshold must allow and an always-cleared one
    // must block. An inverted comparison passes every unit test in isolation.
    let path = model_path();
    if !path.is_file() {
        eprintln!("skipping: no model at {}", path.display());
        return;
    }
    let bytes = include_bytes!("fixtures/gradient.png");

    let block_all = ImageSandbox::new(
        ImageClassifier::load(&path, 224).unwrap(),
        SandboxConfig { explicit_threshold: 0.0, ..SandboxConfig::default() },
    );
    let allow_all = ImageSandbox::new(
        ImageClassifier::load(&path, 224).unwrap(),
        SandboxConfig { explicit_threshold: 1.1, ..SandboxConfig::default() },
    );

    assert!(matches!(block_all.check(bytes), ImageVerdict::Block { .. }));
    assert_eq!(allow_all.check(bytes), ImageVerdict::Allow);
}
