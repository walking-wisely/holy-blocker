//! Turning an arbitrary decoded image into the tensor the classifier expects.
//!
//! The model input is fixed at 224x224 NCHW f32. Getting there is not a free
//! choice: every number in `docs/components/machine-learning/results.md` was
//! measured under one specific transform, and deviating from it means the
//! published accuracy describes something other than what ships.
//!
//! That transform is `machine-learning/src/holy_blocker_ml/dataset.py`
//! (`build_transform(image_size, augment=False)`):
//!
//! ```text
//! Resize(int(224 * 1.14))   # shorter side to 255, aspect preserved, bilinear
//! CenterCrop(224)
//! ToTensor()                # HWC u8 -> CHW f32 in [0, 1]
//! Normalize(IMAGENET_MEAN, IMAGENET_STD)
//! ```
//!
//! Note this is *not* a squash to 224x224. `torchvision.transforms.Resize` with
//! a scalar argument resizes the **shorter** side and preserves aspect ratio;
//! the centre crop then takes the middle square. An earlier version of the
//! image-sandbox plan specified a direct resize to 224x224, which matches
//! neither the training nor the evaluation transform.
//!
//! Reference documents:
//! - torchvision `Resize` (scalar size resizes the shorter edge):
//!   <https://pytorch.org/vision/stable/generated/torchvision.transforms.Resize.html>
//! - torchvision `Normalize`:
//!   <https://pytorch.org/vision/stable/generated/torchvision.transforms.Normalize.html>

use image::{DynamicImage, imageops::FilterType};

/// Model input edge length. Pinned by `TrainingConfig.image_size` and baked
/// into the exported graph, which marks only the batch axis dynamic.
pub const INPUT_SIZE: u32 = 224;

/// Shorter side is resized to `INPUT_SIZE * RESIZE_RATIO` before cropping.
/// From `dataset.py`: `transforms.Resize(int(image_size * 1.14))` = 255.
pub const RESIZE_RATIO: f32 = 1.14;

/// ImageNet channel statistics. torchvision ships MobileNetV3 weights trained
/// with these, and the fine-tuned head inherits the expectation — mirrored from
/// `dataset.IMAGENET_MEAN` / `IMAGENET_STD`.
pub const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
pub const IMAGENET_STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Images with either side below this are not classified.
///
/// **Provisional.** `transforms.Resize` has no lower bound, so a 16x16 sprite
/// is upscaled to 255x255 and scored on evidence that is almost entirely
/// interpolation — and the corpus contains no such images, so nothing in
/// results.md says whether the score means anything. The real floor is being
/// measured by the scale sweep in `holy_blocker_ml.inputs`; until that lands,
/// 32 is a guess chosen to exclude favicons and tracking pixels while keeping
/// plausible thumbnails.
///
/// Note a floor is also a bypass: content served just under it is unfiltered.
pub const MIN_DIMENSION: u32 = 32;

/// Longest:shortest side ratio beyond which an image is not classified.
///
/// This is a resource guard, not a quality one. Resizing the shorter side of a
/// 1000x10 banner to 255 produces a 25500x255 intermediate — about 19.5 MB —
/// from which the centre crop then keeps a 224-pixel sliver. Bounding the ratio
/// bounds that intermediate.
pub const MAX_ASPECT_RATIO: f32 = 8.0;

/// Why an image was not turned into a tensor.
///
/// Every variant means "do not classify", and the caller allows the image.
/// Refusing to classify is not evidence of anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreprocessError {
    /// Below `MIN_DIMENSION` on at least one side.
    TooSmall { width: u32, height: u32 },
    /// Beyond `MAX_ASPECT_RATIO`.
    ExtremeAspect { width: u32, height: u32 },
    /// Zero-width or zero-height.
    Empty,
}

impl std::fmt::Display for PreprocessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooSmall { width, height } => {
                write!(f, "image {width}x{height} is below the {MIN_DIMENSION}px floor")
            }
            Self::ExtremeAspect { width, height } => {
                write!(f, "image {width}x{height} exceeds the {MAX_ASPECT_RATIO}:1 aspect limit")
            }
            Self::Empty => write!(f, "image has a zero dimension"),
        }
    }
}

impl std::error::Error for PreprocessError {}

/// Knobs that the measurement will set. Constructed from the constants above by
/// `Default`, and overridable so the crate does not need a rebuild when the
/// scale sweep produces a real floor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreprocessConfig {
    pub input_size: u32,
    pub resize_ratio: f32,
    pub min_dimension: u32,
    pub max_aspect_ratio: f32,
}

impl Default for PreprocessConfig {
    fn default() -> Self {
        Self {
            input_size: INPUT_SIZE,
            resize_ratio: RESIZE_RATIO,
            min_dimension: MIN_DIMENSION,
            max_aspect_ratio: MAX_ASPECT_RATIO,
        }
    }
}

/// Check an image is worth classifying, without decoding cost beyond dimensions.
pub fn admissible(width: u32, height: u32, config: &PreprocessConfig) -> Result<(), PreprocessError> {
    if width == 0 || height == 0 {
        return Err(PreprocessError::Empty);
    }
    if width < config.min_dimension || height < config.min_dimension {
        return Err(PreprocessError::TooSmall { width, height });
    }
    let long = width.max(height) as f32;
    let short = width.min(height) as f32;
    if long / short > config.max_aspect_ratio {
        return Err(PreprocessError::ExtremeAspect { width, height });
    }
    Ok(())
}

/// Shorter side to `input_size * resize_ratio`, preserving aspect.
///
/// Rounding matches Python's `int(...)` truncation in `dataset.py` and
/// torchvision's own shorter-edge rule, so the target edge is identical.
fn resize_shorter_side(image: &DynamicImage, target: u32) -> DynamicImage {
    let (width, height) = (image.width(), image.height());
    let (new_width, new_height) = if width <= height {
        (target, ((target as f32) * height as f32 / width as f32).round() as u32)
    } else {
        (((target as f32) * width as f32 / height as f32).round() as u32, target)
    };
    // Triangle is the antialiased bilinear filter, matching torchvision's
    // default `InterpolationMode.BILINEAR` with antialias enabled for PIL input.
    image.resize_exact(new_width.max(1), new_height.max(1), FilterType::Triangle)
}

/// Left/top offset of a centred crop, matching torchvision's rounding.
///
/// `transforms.CenterCrop` computes `int(round((extent - crop) / 2.0))`, which
/// rounds a half-pixel remainder up rather than truncating it. Truncating
/// instead shifts the crop one pixel left and up whenever the difference is
/// odd — which for a 595-wide resize is exactly what happens, and it moved the
/// tensor mean by 0.0085 against the Python fixture. Small, systematic, and
/// invisible to any test that only checks Rust against itself.
///
/// Reference: torchvision `CenterCrop`,
/// <https://pytorch.org/vision/stable/generated/torchvision.transforms.CenterCrop.html>
fn centre_offset(extent: u32, crop: u32) -> u32 {
    if extent <= crop {
        return 0;
    }
    (f64::from(extent - crop) / 2.0).round() as u32
}

/// Decoded image -> normalised NCHW f32 buffer of `3 * input_size * input_size`.
pub fn preprocess(
    image: &DynamicImage,
    config: &PreprocessConfig,
) -> Result<Vec<f32>, PreprocessError> {
    admissible(image.width(), image.height(), config)?;

    let target = (config.input_size as f32 * config.resize_ratio) as u32;
    let resized = resize_shorter_side(image, target);

    let size = config.input_size;
    let left = centre_offset(resized.width(), size);
    let top = centre_offset(resized.height(), size);
    let cropped = resized.crop_imm(left, top, size, size).to_rgb8();

    // NCHW: all of R, then all of G, then all of B — the layout the exported
    // graph declares for its `image` input.
    let pixel_count = (size * size) as usize;
    let mut tensor = vec![0.0f32; 3 * pixel_count];
    for (index, pixel) in cropped.pixels().enumerate() {
        for channel in 0..3 {
            let scaled = pixel.0[channel] as f32 / 255.0;
            tensor[channel * pixel_count + index] =
                (scaled - IMAGENET_MEAN[channel]) / IMAGENET_STD[channel];
        }
    }
    Ok(tensor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    fn solid(width: u32, height: u32, colour: [u8; 3]) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_pixel(width, height, Rgb(colour)))
    }

    #[test]
    fn tensor_has_the_shape_the_exported_graph_declares() {
        let tensor = preprocess(&solid(300, 300, [128, 128, 128]), &PreprocessConfig::default())
            .expect("a 300x300 image is admissible");

        assert_eq!(tensor.len(), 3 * 224 * 224);
    }

    #[test]
    fn normalisation_matches_the_imagenet_statistics() {
        // A solid image survives resize and crop unchanged, so every element of
        // a channel must equal (value/255 - mean) / std exactly.
        let tensor = preprocess(&solid(300, 300, [255, 0, 0]), &PreprocessConfig::default())
            .unwrap();

        let pixels = 224 * 224;
        let red = (1.0 - IMAGENET_MEAN[0]) / IMAGENET_STD[0];
        let green = (0.0 - IMAGENET_MEAN[1]) / IMAGENET_STD[1];
        assert!((tensor[0] - red).abs() < 1e-5, "got {}", tensor[0]);
        assert!((tensor[pixels] - green).abs() < 1e-5, "got {}", tensor[pixels]);
    }

    #[test]
    fn channels_are_planar_not_interleaved() {
        // NCHW means the first `pixels` elements are all red. Interleaved
        // (NHWC) output would put green at index 1 and still have the right
        // length, which is exactly the bug a length assertion misses.
        let tensor = preprocess(&solid(300, 300, [255, 0, 0]), &PreprocessConfig::default())
            .unwrap();

        let pixels = 224 * 224;
        let red = (1.0 - IMAGENET_MEAN[0]) / IMAGENET_STD[0];
        assert!((tensor[1] - red).abs() < 1e-5, "index 1 should still be red");
        assert!(tensor[pixels] < 0.0, "green channel should be below its mean");
    }

    #[test]
    fn the_shorter_side_is_resized_not_the_longer_one() {
        // 600x300 -> shorter side 300 becomes 255, so the result is 510x255,
        // and the crop takes 224 from the middle. Squashing to 224x224 instead
        // would keep content the centre crop must discard.
        let resized = resize_shorter_side(&solid(600, 300, [1, 2, 3]), 255);

        assert_eq!(resized.height(), 255);
        assert_eq!(resized.width(), 510);
    }

    #[test]
    fn a_portrait_image_resizes_its_width() {
        let resized = resize_shorter_side(&solid(300, 600, [1, 2, 3]), 255);

        assert_eq!(resized.width(), 255);
        assert_eq!(resized.height(), 510);
    }

    #[test]
    fn the_crop_keeps_the_centre_of_a_wide_image() {
        // Left third red, middle third green, right third blue. The centre crop
        // of a 3:1 image must be entirely green.
        let mut image = RgbImage::new(900, 300);
        for (x, _, pixel) in image.enumerate_pixels_mut() {
            *pixel = match x {
                0..300 => Rgb([255, 0, 0]),
                300..600 => Rgb([0, 255, 0]),
                _ => Rgb([0, 0, 255]),
            };
        }

        let tensor =
            preprocess(&DynamicImage::ImageRgb8(image), &PreprocessConfig::default()).unwrap();

        let pixels = 224 * 224;
        let green = (1.0 - IMAGENET_MEAN[1]) / IMAGENET_STD[1];
        let centre = 112 * 224 + 112;
        assert!(
            (tensor[pixels + centre] - green).abs() < 0.1,
            "centre pixel should be green, got {}",
            tensor[pixels + centre]
        );
    }

    #[test]
    fn centre_offset_rounds_a_half_pixel_up_like_torchvision() {
        // 595 -> 224 leaves 371, half of which is 185.5. Truncating gives 185
        // and shifts the whole crop one pixel left.
        assert_eq!(centre_offset(595, 224), 186);
        assert_eq!(centre_offset(255, 224), 16);
    }

    #[test]
    fn centre_offset_is_zero_when_there_is_nothing_to_crop() {
        assert_eq!(centre_offset(224, 224), 0);
        assert_eq!(centre_offset(100, 224), 0);
    }

    #[test]
    fn an_image_below_the_floor_is_refused() {
        let result = preprocess(&solid(16, 16, [0, 0, 0]), &PreprocessConfig::default());

        assert_eq!(result, Err(PreprocessError::TooSmall { width: 16, height: 16 }));
    }

    #[test]
    fn the_floor_applies_to_either_side() {
        let result = preprocess(&solid(400, 8, [0, 0, 0]), &PreprocessConfig::default());

        assert!(matches!(result, Err(PreprocessError::TooSmall { .. })));
    }

    #[test]
    fn an_extreme_aspect_ratio_is_refused_before_allocating() {
        // 1000x40 is 25:1. Accepting it would resize to 6375x255 first.
        let result = preprocess(&solid(1000, 40, [0, 0, 0]), &PreprocessConfig::default());

        assert_eq!(result, Err(PreprocessError::ExtremeAspect { width: 1000, height: 40 }));
    }

    #[test]
    fn an_aspect_ratio_at_the_limit_is_accepted() {
        let result = preprocess(&solid(800, 100, [10, 10, 10]), &PreprocessConfig::default());

        assert!(result.is_ok(), "8:1 is the limit, not past it");
    }

    #[test]
    fn a_zero_dimension_image_is_refused_rather_than_panicking() {
        assert_eq!(admissible(0, 100, &PreprocessConfig::default()), Err(PreprocessError::Empty));
    }

    #[test]
    fn the_floor_is_configurable_without_a_rebuild() {
        // The measured floor is not in yet; a caller must be able to set it.
        let config = PreprocessConfig { min_dimension: 8, ..PreprocessConfig::default() };

        assert!(preprocess(&solid(16, 16, [9, 9, 9]), &config).is_ok());
    }
}
