//! Raw framebuffer input, for callers that never had an encoded image.
//!
//! [`crate::sandbox::ImageSandbox::check`] takes encoded bytes because its first
//! caller is `mitm-proxy`, which is holding an HTTP response body. The screen
//! path has the opposite problem: `ScreenCaptureKit` hands the macOS daemon a
//! tightly packed BGRA framebuffer several times a second, and there is no
//! encoded form anywhere in that pipeline.
//!
//! Encoding a frame to PNG purely so the sandbox can decode it again would cost
//! a compress/decompress round trip on a ~12 MB buffer at the image cadence, to
//! recover bytes the caller already had. This module skips both halves.
//!
//! ## Alpha is dropped, deliberately
//!
//! The classifier's input tensor is RGB — `preprocess`'s `to_tensor` calls
//! `to_rgb8` and normalises three channels. A screen framebuffer is opaque, so
//! its alpha channel carries no information; discarding it here rather than
//! building an `RgbaImage` and converting later avoids one full-frame copy.

use image::{DynamicImage, RgbImage};

/// Channel order of a 4-bytes-per-pixel framebuffer.
///
/// `Bgra` is what `ScreenCaptureKit` delivers under
/// `kCVPixelFormatType_32BGRA`, which the macOS daemon sets explicitly — the
/// macOS 26.5 default is biplanar `420v`, whose stride is the Y plane's and
/// cannot be read as packed pixels at all. See
/// `docs/components/mac-daemon/plan.md`, the first live e2e pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelLayout {
    Bgra,
    Rgba,
}

impl PixelLayout {
    /// Byte offsets of red, green and blue within one pixel.
    fn channel_offsets(self) -> (usize, usize, usize) {
        match self {
            PixelLayout::Bgra => (2, 1, 0),
            PixelLayout::Rgba => (0, 1, 2),
        }
    }
}

/// Why a raw buffer could not be read as an image.
///
/// Both variants mean the caller's geometry disagrees with its buffer, which is
/// a fault in us rather than evidence about the content — so every caller
/// treats these as allow, per the sandbox's fail-open rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawFrameError {
    /// Zero width or height. The macOS daemon's `CapturedFrame.empty` is
    /// exactly this, and it is the ordinary state before the first frame
    /// arrives rather than a malfunction.
    Empty,
    BufferTooSmall { expected: usize, actual: usize },
}

impl std::fmt::Display for RawFrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RawFrameError::Empty => write!(f, "raw frame has zero width or height"),
            RawFrameError::BufferTooSmall { expected, actual } => write!(
                f,
                "raw frame buffer is {actual} bytes, need at least {expected}"
            ),
        }
    }
}

impl std::error::Error for RawFrameError {}

/// A 4-channel framebuffer: one byte each for blue, green, red and alpha (or
/// the RGBA permutation of the same).
pub const BYTES_PER_PIXEL: usize = 4;

/// Reads a tightly packed 4-channel framebuffer as an RGB image.
///
/// `pixels` must be **row-major with no row padding** — `width * height * 4`
/// bytes. Stride is the caller's problem, and on the macOS side it is already
/// solved: `PixelBufferCopy.depad` strips CoreVideo's row alignment before the
/// frame ever reaches this crate. Accepting a `bytesPerRow` here would put a
/// second implementation of that same de-padding on the other side of an FFI
/// boundary.
///
/// A buffer **longer** than the geometry requires is accepted and its trailing
/// bytes ignored, matching `PixelBufferCopy.depad`'s own `>=` guard: an
/// over-allocated buffer is a normal thing for a caller reusing scratch space,
/// and the leading `width * height * 4` bytes are unambiguous either way.
pub fn image_from_raw(
    pixels: &[u8],
    width: u32,
    height: u32,
    layout: PixelLayout,
) -> Result<DynamicImage, RawFrameError> {
    if width == 0 || height == 0 {
        return Err(RawFrameError::Empty);
    }

    let expected = width as usize * height as usize * BYTES_PER_PIXEL;
    if pixels.len() < expected {
        return Err(RawFrameError::BufferTooSmall {
            expected,
            actual: pixels.len(),
        });
    }

    let (red, green, blue) = layout.channel_offsets();
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for pixel in pixels[..expected].chunks_exact(BYTES_PER_PIXEL) {
        rgb.push(pixel[red]);
        rgb.push(pixel[green]);
        rgb.push(pixel[blue]);
    }

    // Cannot fail: the buffer was sized from the same width and height.
    let buffer = RgbImage::from_raw(width, height, rgb)
        .expect("rgb buffer is width * height * 3 by construction");
    Ok(DynamicImage::ImageRgb8(buffer))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One pixel, in the byte order a BGRA framebuffer stores pure red.
    const RED_BGRA: [u8; 4] = [0x00, 0x00, 0xFF, 0xFF];

    #[test]
    fn bgra_channels_are_reordered_not_copied_through() {
        // The whole reason this module has a layout parameter. Reading BGRA as
        // RGBA swaps red and blue, which is invisible in any test using grey
        // and would quietly change what the classifier sees on skin tones.
        let image = image_from_raw(&RED_BGRA, 1, 1, PixelLayout::Bgra).expect("valid");

        assert_eq!(image.to_rgb8().get_pixel(0, 0).0, [0xFF, 0x00, 0x00]);
    }

    #[test]
    fn rgba_channels_pass_through_in_order() {
        let image = image_from_raw(&RED_BGRA, 1, 1, PixelLayout::Rgba).expect("valid");

        assert_eq!(image.to_rgb8().get_pixel(0, 0).0, [0x00, 0x00, 0xFF]);
    }

    #[test]
    fn alpha_is_dropped_rather_than_multiplied_into_the_colour() {
        // A fully transparent red pixel still reads as red: the classifier's
        // tensor is RGB, and a screen framebuffer's alpha is meaningless.
        let transparent_red = [0x00, 0x00, 0xFF, 0x00];
        let image = image_from_raw(&transparent_red, 1, 1, PixelLayout::Bgra).expect("valid");

        assert_eq!(image.to_rgb8().get_pixel(0, 0).0, [0xFF, 0x00, 0x00]);
    }

    #[test]
    fn pixels_land_in_row_major_order() {
        // Guards a transposed read, which would score a real frame as *something*
        // rather than failing — the same silent class of bug as a sheared copy.
        // 2x1: first pixel red, second blue.
        let mut pixels = Vec::new();
        pixels.extend_from_slice(&RED_BGRA);
        pixels.extend_from_slice(&[0xFF, 0x00, 0x00, 0xFF]);

        let image = image_from_raw(&pixels, 2, 1, PixelLayout::Bgra).expect("valid");
        let rgb = image.to_rgb8();

        assert_eq!(rgb.dimensions(), (2, 1));
        assert_eq!(rgb.get_pixel(0, 0).0, [0xFF, 0x00, 0x00]);
        assert_eq!(rgb.get_pixel(1, 0).0, [0x00, 0x00, 0xFF]);
    }

    #[test]
    fn a_zero_dimension_frame_is_empty_not_a_panic() {
        // `CapturedFrame.empty()` is the ordinary pre-first-frame state on the
        // macOS side, so this path is hit in normal operation.
        assert_eq!(image_from_raw(&[], 0, 0, PixelLayout::Bgra), Err(RawFrameError::Empty));
        assert_eq!(image_from_raw(&RED_BGRA, 1, 0, PixelLayout::Bgra), Err(RawFrameError::Empty));
        assert_eq!(image_from_raw(&RED_BGRA, 0, 1, PixelLayout::Bgra), Err(RawFrameError::Empty));
    }

    #[test]
    fn a_short_buffer_is_refused_rather_than_read_past() {
        // Reading past the end would be unsound; taking a partial frame would
        // classify a torn image. Refusing is the only safe answer, and the
        // caller allows on it.
        let two_pixels_worth = [0u8; 8];

        assert_eq!(
            image_from_raw(&two_pixels_worth, 2, 2, PixelLayout::Bgra),
            Err(RawFrameError::BufferTooSmall { expected: 16, actual: 8 })
        );
    }

    #[test]
    fn a_longer_buffer_is_accepted_and_its_tail_ignored() {
        // Matches PixelBufferCopy.depad's `>=` guard: an over-allocated scratch
        // buffer is normal and the leading bytes are unambiguous.
        let mut pixels = RED_BGRA.to_vec();
        pixels.extend_from_slice(&[0xAB; 64]);

        let image = image_from_raw(&pixels, 1, 1, PixelLayout::Bgra).expect("valid");

        assert_eq!(image.to_rgb8().dimensions(), (1, 1));
    }

    #[test]
    fn a_realistic_frame_round_trips_at_full_size() {
        // 64x32 rather than 1x1, so a stride or row-order mistake has somewhere
        // to show up. Every pixel's green channel encodes its row.
        let (width, height) = (64u32, 32u32);
        let mut pixels = Vec::with_capacity((width * height) as usize * BYTES_PER_PIXEL);
        for row in 0..height {
            for _ in 0..width {
                pixels.extend_from_slice(&[0x00, row as u8, 0x00, 0xFF]);
            }
        }

        let rgb = image_from_raw(&pixels, width, height, PixelLayout::Bgra)
            .expect("valid")
            .to_rgb8();

        assert_eq!(rgb.dimensions(), (width, height));
        for row in 0..height {
            assert_eq!(rgb.get_pixel(0, row).0[1], row as u8, "row {row}");
            assert_eq!(rgb.get_pixel(width - 1, row).0[1], row as u8, "row {row}");
        }
    }
}
