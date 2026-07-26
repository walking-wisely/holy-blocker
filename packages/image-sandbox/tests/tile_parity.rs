//! Cross-language parity for the **deployed** geometry: does Rust tiling match
//! `holy_blocker_ml.inputs.tile_geometry`?
//!
//! `parity.rs` does this for the centre-crop transform every published figure
//! was measured under. This file does it for the transform that actually ships,
//! which is the one that can silently diverge without any number moving on a
//! results page. The same argument applies as there, one step stronger: a wrong
//! stride, a missing flush-to-edge window, or resizing the shorter side to 255
//! instead of 224 all produce a perfectly plausible stack of tiles and a
//! quietly different verdict.
//!
//! The fixture is `wide.png`, 900x300, chosen so that each failure mode is
//! observable:
//!
//! - **high-frequency** in red (`sin(x/7)`), so a wrong resample filter shows;
//! - **a left-to-right green ramp**, so tile *position* is visible in each
//!   tile's mean — the means are strictly increasing, and any mis-ordered,
//!   mis-strided, or missing tile breaks that;
//! - **3:1**, so it produces five tiles rather than the trivial one.
//!
//! Regenerate with, from the repository root:
//!
//! ```text
//! PYTHONPATH=machine-learning/src python - <<'PY'
//! from pathlib import Path
//! from PIL import Image
//! from holy_blocker_ml.inputs import tile_geometry
//! out = Path("packages/image-sandbox/tests/fixtures")
//! image = Image.open(out / "wide.png").convert("RGB")
//! tiles = tile_geometry(224)(image).numpy()
//! lines = [f"tiles {tiles.shape[0]} 3 224 224"]
//! for i, t in enumerate(tiles):
//!     f = t.reshape(-1)
//!     lines.append(f"tile {i} mean {f.mean():.8f} std {f.std():.8f} "
//!                  f"min {f.min():.8f} max {f.max():.8f}")
//!     idx = list(range(0, f.size, f.size // 16))[:16]
//!     lines += [f"probe {i} {j} {f[j]:.8f}" for j in idx]
//! (out / "wide.expected.txt").write_text("\n".join(lines) + "\n")
//! PY
//! ```
//!
//! Tolerances follow `parity.rs`: tight on aggregates, which average out
//! per-pixel resampling differences between Pillow and the `image` crate, and
//! loose per-pixel.

use image_sandbox::preprocess::{PreprocessConfig, preprocess_tiles};

struct ExpectedTile {
    mean: f32,
    std: f32,
    probes: Vec<(usize, f32)>,
}

fn load_expected() -> (usize, Vec<ExpectedTile>) {
    let text = include_str!("fixtures/wide.expected.txt");
    let mut count = 0usize;
    let mut tiles: Vec<ExpectedTile> = Vec::new();

    for line in text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.as_slice() {
            ["tiles", n, "3", "224", "224"] => count = n.parse().unwrap(),
            ["tile", i, "mean", mean, "std", std, "min", _, "max", _] => {
                assert_eq!(i.parse::<usize>().unwrap(), tiles.len(), "tiles out of order");
                tiles.push(ExpectedTile {
                    mean: mean.parse().unwrap(),
                    std: std.parse().unwrap(),
                    probes: Vec::new(),
                });
            }
            ["probe", i, j, v] => {
                let tile: usize = i.parse().unwrap();
                tiles[tile].probes.push((j.parse().unwrap(), v.parse().unwrap()));
            }
            other => panic!("unrecognised fixture line: {other:?}"),
        }
    }

    assert_eq!(tiles.len(), count, "fixture header disagrees with its own rows");
    assert!(count > 1, "fixture must be wide enough to actually tile");
    (count, tiles)
}

fn rust_tiles() -> Vec<Vec<f32>> {
    let image =
        image::load_from_memory(include_bytes!("fixtures/wide.png")).expect("fixture png decodes");
    preprocess_tiles(&image, &PreprocessConfig::default()).expect("fixture is admissible")
}

fn mean_of(tile: &[f32]) -> f32 {
    tile.iter().sum::<f32>() / tile.len() as f32
}

#[test]
fn the_tile_count_matches_torchvision() {
    // The single most likely divergence: a stride off by a factor, or a missing
    // flush-to-far-edge window, changes only this number.
    let (count, _) = load_expected();

    assert_eq!(rust_tiles().len(), count);
}

#[test]
fn every_tile_has_the_shape_the_graph_declares() {
    assert!(rust_tiles().iter().all(|t| t.len() == 3 * 224 * 224));
}

#[test]
fn per_tile_aggregates_match_torchvision() {
    let tiles = rust_tiles();
    let (_, expected) = load_expected();

    for (index, (tile, want)) in tiles.iter().zip(&expected).enumerate() {
        let mean = mean_of(tile);
        let variance =
            tile.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / tile.len() as f32;
        let std = variance.sqrt();

        assert!(
            (mean - want.mean).abs() < 1e-3,
            "tile {index} mean {mean} vs torchvision {}",
            want.mean
        );
        assert!(
            (std - want.std).abs() < 1e-3,
            "tile {index} std {std} vs torchvision {}",
            want.std
        );
    }
}

#[test]
fn tile_means_increase_left_to_right_exactly_as_in_python() {
    // The fixture's green channel ramps across the image, so each tile's mean
    // encodes where it was taken from. This is what distinguishes "five tiles
    // of the right shape" from "five tiles from the right places" — a reversed
    // or duplicated tile passes every other test in this file.
    let tiles = rust_tiles();
    let (_, expected) = load_expected();

    let means: Vec<f32> = tiles.iter().map(|t| mean_of(t)).collect();
    assert!(
        means.windows(2).all(|w| w[0] < w[1]),
        "tile means must increase with position, got {means:?}"
    );

    let want: Vec<f32> = expected.iter().map(|t| t.mean).collect();
    assert!(
        want.windows(2).all(|w| w[0] < w[1]),
        "the fixture itself must be position-sensitive, or this test proves nothing"
    );
}

#[test]
fn sampled_values_match_torchvision_within_resampling_tolerance() {
    let tiles = rust_tiles();
    let (_, expected) = load_expected();

    let mut worst = 0.0f32;
    let mut worst_at = (0usize, 0usize);
    for (index, (tile, want)) in tiles.iter().zip(&expected).enumerate() {
        for &(offset, value) in &want.probes {
            let delta = (tile[offset] - value).abs();
            if delta > worst {
                worst = delta;
                worst_at = (index, offset);
            }
        }
    }

    // Per-pixel drift between two bilinear implementations on a deliberately
    // high-frequency fixture. Check the aggregate test first if this trips: it
    // separates a systematic error from resampling noise.
    assert!(
        worst < 0.25,
        "worst per-pixel delta {worst} at tile/offset {worst_at:?}; check aggregates first"
    );
}

#[test]
fn resizing_to_the_crop_ratio_would_fail_this_fixture() {
    // Guards the guard. tile_geometry resizes the shorter side to 224, not to
    // 224*1.14 as the centre-crop path does. Confirm the fixture can tell those
    // apart, rather than passing whatever it is handed.
    let image = image::load_from_memory(include_bytes!("fixtures/wide.png")).unwrap();
    let wrong = PreprocessConfig { input_size: 255, ..PreprocessConfig::default() };

    let tiles = preprocess_tiles(&image, &wrong).unwrap();
    let (count, _) = load_expected();

    assert_ne!(
        tiles.len(),
        count,
        "the fixture cannot distinguish the two resize targets, so the parity \
         check above is not testing what it claims"
    );
}
