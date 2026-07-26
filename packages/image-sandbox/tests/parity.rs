//! Cross-language parity: does the Rust preprocessing match torchvision?
//!
//! The unit tests in `preprocess.rs` check the module against itself — shape,
//! layout, crop position, refusals. None of them can catch the failure that
//! matters most: producing a *self-consistent* tensor that differs from the one
//! the model was trained and evaluated on. A wrong resample filter or an
//! off-by-one crop yields a perfectly plausible tensor and a silently different
//! score, and no amount of Rust-only testing sees it.
//!
//! So the expected values come from Python. `tests/fixtures/gradient.expected.txt`
//! is the output of `dataset.build_transform(224, augment=False)` applied to
//! `tests/fixtures/gradient.png`, a deterministic synthetic image chosen to be
//! asymmetric (a wrong crop position shows) and high-frequency (a wrong
//! resample filter shows).
//!
//! Regenerate with, from the repository root:
//!
//! ```text
//! PYTHONPATH=machine-learning/src python - <<'PY'
//! import numpy as np
//! from pathlib import Path
//! from PIL import Image
//! from holy_blocker_ml.dataset import build_transform
//! out = Path("packages/image-sandbox/tests/fixtures")
//! image = Image.open(out / "gradient.png").convert("RGB")
//! t = build_transform(224, augment=False)(image).numpy().reshape(-1)
//! idx = list(range(0, t.size, t.size // 64))[:64]
//! lines = ["shape 3 224 224", f"mean {t.mean():.8f}", f"std {t.std():.8f}",
//!          f"min {t.min():.8f}", f"max {t.max():.8f}"]
//! lines += [f"probe {i} {t[i]:.8f}" for i in idx]
//! (out / "gradient.expected.txt").write_text("\n".join(lines) + "\n")
//! PY
//! ```
//!
//! ## On the tolerance
//!
//! Exact equality is not achievable and not the goal. Pillow's bilinear
//! resampling and the `image` crate's `FilterType::Triangle` are both
//! antialiased bilinear filters but differ in support width and rounding, so
//! individual pixels drift. What must hold is that the *distribution* the model
//! sees is the same one it was measured on — hence tight bounds on the
//! aggregate statistics and a looser per-pixel bound.

use image_sandbox::preprocess::{PreprocessConfig, preprocess};

struct Expected {
    mean: f32,
    std: f32,
    min: f32,
    max: f32,
    probes: Vec<(usize, f32)>,
}

fn load_expected() -> Expected {
    let text = include_str!("fixtures/gradient.expected.txt");
    let mut expected = Expected { mean: 0.0, std: 0.0, min: 0.0, max: 0.0, probes: Vec::new() };
    for line in text.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            ["mean", v] => expected.mean = v.parse().unwrap(),
            ["std", v] => expected.std = v.parse().unwrap(),
            ["min", v] => expected.min = v.parse().unwrap(),
            ["max", v] => expected.max = v.parse().unwrap(),
            ["probe", i, v] => expected.probes.push((i.parse().unwrap(), v.parse().unwrap())),
            ["shape", "3", "224", "224"] => {}
            other => panic!("unrecognised fixture line: {other:?}"),
        }
    }
    assert!(!expected.probes.is_empty(), "fixture carried no probe values");
    expected
}

fn rust_tensor() -> Vec<f32> {
    let image = image::load_from_memory(include_bytes!("fixtures/gradient.png"))
        .expect("fixture png decodes");
    preprocess(&image, &PreprocessConfig::default()).expect("fixture is admissible")
}

#[test]
fn tensor_length_matches_the_python_shape() {
    assert_eq!(rust_tensor().len(), 3 * 224 * 224);
}

#[test]
fn aggregate_statistics_match_torchvision() {
    let tensor = rust_tensor();
    let expected = load_expected();

    let n = tensor.len() as f32;
    let mean = tensor.iter().sum::<f32>() / n;
    let variance = tensor.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n;
    let std = variance.sqrt();

    // Aggregates average out per-pixel resampling differences, so they are the
    // strict check: a wrong normalisation, channel order, or crop region moves
    // these well beyond 1e-3.
    assert!(
        (mean - expected.mean).abs() < 1e-3,
        "mean {mean} vs torchvision {}",
        expected.mean
    );
    assert!((std - expected.std).abs() < 1e-3, "std {std} vs torchvision {}", expected.std);
}

#[test]
fn value_range_matches_torchvision() {
    let tensor = rust_tensor();
    let expected = load_expected();

    let min = tensor.iter().copied().fold(f32::INFINITY, f32::min);
    let max = tensor.iter().copied().fold(f32::NEG_INFINITY, f32::max);

    // Range is set by the ImageNet constants and the extremes present in the
    // image; resampling can only pull extremes inward, never past them.
    assert!((min - expected.min).abs() < 0.05, "min {min} vs {}", expected.min);
    assert!((max - expected.max).abs() < 0.05, "max {max} vs {}", expected.max);
}

#[test]
fn sampled_values_match_torchvision_within_resampling_tolerance() {
    let tensor = rust_tensor();
    let expected = load_expected();

    let mut worst = 0.0f32;
    let mut worst_at = 0usize;
    for &(index, want) in &expected.probes {
        let delta = (tensor[index] - want).abs();
        if delta > worst {
            worst = delta;
            worst_at = index;
        }
    }

    // Per-pixel drift between two bilinear implementations on a high-frequency
    // fixture. If this fails the fixture is deliberately hostile: check the
    // aggregate test first, which isolates a systematic error from noise.
    assert!(
        worst < 0.25,
        "worst per-pixel delta {worst} at index {worst_at}; aggregates should be checked first"
    );
}

#[test]
fn a_squashed_resize_would_fail_this_fixture() {
    // Guards the guard. The image-sandbox plan originally specified a direct
    // resize to 224x224; this confirms the fixture actually distinguishes that
    // from the shorter-side-plus-crop transform, rather than passing anything.
    let image = image::load_from_memory(include_bytes!("fixtures/gradient.png")).unwrap();
    let squashed = image.resize_exact(224, 224, image::imageops::FilterType::Triangle);
    let squashed_tensor =
        preprocess(&squashed, &PreprocessConfig::default()).expect("224x224 is admissible");

    let expected = load_expected();
    let n = squashed_tensor.len() as f32;
    let mean = squashed_tensor.iter().sum::<f32>() / n;

    assert!(
        (mean - expected.mean).abs() > 1e-3,
        "squashing produced the same mean as the correct transform, so this \
         fixture cannot tell them apart"
    );
}
