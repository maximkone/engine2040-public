//! png_out: encode raw 8-bit texture bytes as a PNG file, deterministically.
//!
//! Consumer: the door's `geo.bake` and `asset.cook` verbs write a normal map
//! beside the cooked bundle; the bytes must repeat for the same pixels, so the
//! caller hands row-major texels and gets a complete PNG byte string. Owner:
//! `engine-assets` (the crate that already carries the `image` codec). Units:
//! texels in, bytes out. Errors: [`PngError`], never a panic.

use std::fmt;

/// Byte length of one RGB texel.
const RGB_TEXELS: usize = 3;

/// Why a PNG encode failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PngError {
    /// Width or height is 0.
    EmptySize,
    /// Width or height exceeds [`MAX_EDGE`].
    TooLarge {
        /// Offending width in texels.
        width: u32,
        /// Offending height in texels.
        height: u32,
    },
    /// The texel buffer length is not `width * height * 3`.
    BadLength {
        /// Length the size asks for.
        expected: usize,
        /// Length the caller passed.
        got: usize,
    },
    /// The codec refused the pixels; bounded detail.
    Encode(String),
}

/// Largest map edge this writer accepts, texels.
pub const MAX_EDGE: u32 = 4096;

impl fmt::Display for PngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySize => write!(f, "png: width and height must be at least 1"),
            Self::TooLarge { width, height } => {
                write!(f, "png: {width}x{height} exceeds {MAX_EDGE} texels")
            }
            Self::BadLength { expected, got } => {
                write!(f, "png: buffer holds {got} bytes, size needs {expected}")
            }
            Self::Encode(detail) => write!(f, "png: encoder refused: {detail}"),
        }
    }
}

impl std::error::Error for PngError {}

/// Encode row-major `texture_channel = 3` texels into PNG bytes.
///
/// Same width, height and texels give the same bytes. Errors: [`PngError`] on a
/// zero or oversized size, a buffer length that is not `width * height * 3`, or
/// a codec refusal.
pub fn encode_png_rgb8(width: u32, height: u32, rgb: &[u8]) -> Result<Vec<u8>, PngError> {
    if width == 0 || height == 0 {
        return Err(PngError::EmptySize);
    }
    if width > MAX_EDGE || height > MAX_EDGE {
        return Err(PngError::TooLarge { width, height });
    }
    let expected = usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|pixels| pixels.checked_mul(RGB_TEXELS))
        .ok_or(PngError::TooLarge { width, height })?;
    if rgb.len() != expected {
        return Err(PngError::BadLength {
            expected,
            got: rgb.len(),
        });
    }
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::{ExtendedColorType, ImageEncoder};
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Default, FilterType::Adaptive)
        .write_image(rgb, width, height, ExtendedColorType::Rgb8)
        .map_err(|e| PngError::Encode(e.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_map_encodes_to_the_same_bytes_twice() {
        let texels = vec![128u8; 4 * 4 * 3];
        let first = encode_png_rgb8(4, 4, &texels).expect("encodes");
        let second = encode_png_rgb8(4, 4, &texels).expect("encodes again");
        assert_eq!(first, second, "same texels, same bytes");
        assert_eq!(&first[1..4], b"PNG", "a PNG signature is written");
    }

    #[test]
    fn bad_sizes_and_lengths_refuse_typed() {
        assert_eq!(encode_png_rgb8(0, 4, &[]), Err(PngError::EmptySize));
        assert_eq!(
            encode_png_rgb8(MAX_EDGE + 1, 1, &[]),
            Err(PngError::TooLarge {
                width: MAX_EDGE + 1,
                height: 1
            })
        );
        assert_eq!(
            encode_png_rgb8(2, 2, &[0; 11]),
            Err(PngError::BadLength {
                expected: 12,
                got: 11
            })
        );
    }
}
