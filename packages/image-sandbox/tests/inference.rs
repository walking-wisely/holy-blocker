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

use image_sandbox::{ImageClassifier, ImageSandbox, ImageVerdict, PixelLayout, SandboxConfig};
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

// --- the raw framebuffer path ---------------------------------------------

/// Decodes a fixture and re-emits it as the tightly packed BGRA buffer the
/// macOS daemon's `CapturedFrame` carries, so the two entry points can be
/// compared on identical pixels.
fn as_bgra(bytes: &[u8]) -> (Vec<u8>, u32, u32) {
    let image = image::load_from_memory(bytes).expect("fixture decodes").to_rgb8();
    let (width, height) = image.dimensions();
    let mut pixels = Vec::with_capacity((width * height) as usize * 4);
    for pixel in image.pixels() {
        pixels.extend_from_slice(&[pixel.0[2], pixel.0[1], pixel.0[0], 0xFF]);
    }
    (pixels, width, height)
}

#[test]
fn a_raw_frame_scores_identically_to_the_same_image_encoded() {
    // The claim the whole screen path rests on: `check_raw` is `check` with a
    // different decoder in front, not a second geometry. If these ever diverge,
    // every threshold measured through `check` stops describing what the daemon
    // actually does — and the divergence would be a channel swap or a row-order
    // mistake, both of which still produce a plausible-looking score.
    let Some(sandbox) = sandbox_or_skip() else { return };
    let bytes = include_bytes!("fixtures/gradient.png");
    let (pixels, width, height) = as_bgra(bytes);

    assert_eq!(
        sandbox.check_raw(&pixels, width, height, PixelLayout::Bgra).verdict,
        sandbox.check(bytes)
    );
}

#[test]
fn a_wide_raw_frame_takes_the_same_tiled_path() {
    // The wide fixture is what exercises multi-tile geometry, which is the case
    // a screen frame is always in.
    let Some(sandbox) = sandbox_or_skip() else { return };
    let bytes = include_bytes!("fixtures/wide.png");
    let (pixels, width, height) = as_bgra(bytes);

    assert_eq!(
        sandbox.check_raw(&pixels, width, height, PixelLayout::Bgra).verdict,
        sandbox.check(bytes)
    );
}

#[test]
fn reading_a_bgra_frame_as_rgba_changes_the_score() {
    // Proves the channel order is load-bearing rather than cosmetic: if this
    // ever stops holding, the equivalence test above would pass with a broken
    // layout parameter.
    let path = model_path();
    if !path.is_file() {
        eprintln!("skipping: no model at {}", path.display());
        return;
    }
    let bytes = include_bytes!("fixtures/gradient.png");
    let (pixels, width, height) = as_bgra(bytes);
    // Threshold 0.0 blocks everything, so both verdicts carry their score and
    // can be compared numerically rather than only as allow/block.
    let sandbox = ImageSandbox::new(
        ImageClassifier::load(&path, 224).unwrap(),
        SandboxConfig { explicit_threshold: 0.0, ..SandboxConfig::default() },
    );

    let correct = sandbox.check_raw(&pixels, width, height, PixelLayout::Bgra);
    let swapped = sandbox.check_raw(&pixels, width, height, PixelLayout::Rgba);

    assert_ne!(correct.score, swapped.score);
}
