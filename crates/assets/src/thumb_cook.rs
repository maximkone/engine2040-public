//! thumb_cook: one 128 px RGBA8 thumbnail cooked beside an import.
//!
//! Owner: `engine-assets`; consumer: `engine-editor`'s asset browser, which
//! caches the pixels per content digest and draws one per listed row. The
//! import cooks it, so a fresh import has its thumbnail with no separate
//! editor-side pass.
//! Units: source texels in, [`THUMB_BYTES`] RGBA8 bytes out. The box average is
//! integer only and walks the source in row-major order, so the same pixels
//! give the same thumbnail on every run and machine; no clock, no RNG.
//! Errors: [`ThumbError`]; a refused cook returns no pixels.

use std::fmt;

/// Thumbnail width and height in pixels: the budget WG14 names.
pub const THUMB_EDGE: u32 = 128;
/// Bytes of one RGBA8 thumbnail (`128 * 128 * 4`).
pub const THUMB_BYTES: usize = 128 * 128 * 4;
/// Largest source edge a cook accepts, pixels.
pub const MAX_SOURCE_EDGE: u32 = 8192;
/// RGBA channels per texel.
const CHANNELS: usize = 4;

/// One cooked thumbnail: 128 px RGBA8 pixels plus their content digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CookedThumb {
    /// Pixels, [`THUMB_BYTES`] long, rows top-down, RGBA8.
    pub rgba: Vec<u8>,
    /// [`crate::registry::content_hash`] of `rgba`, so a reader keys the
    /// pixels by their own bytes.
    pub digest: u64,
}

impl CookedThumb {
    /// Pixel width (always [`THUMB_EDGE`]).
    pub fn width(&self) -> u32 {
        THUMB_EDGE
    }

    /// Pixel height (always [`THUMB_EDGE`]).
    pub fn height(&self) -> u32 {
        THUMB_EDGE
    }
}

/// What refused a cook. Nothing was cooked on error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThumbError {
    /// Width or height is 0.
    EmptySize {
        /// Offending width.
        width: u32,
        /// Offending height.
        height: u32,
    },
    /// An edge went over [`MAX_SOURCE_EDGE`], which the arithmetic bounds.
    TooLarge {
        /// Offending width.
        width: u32,
        /// Offending height.
        height: u32,
    },
    /// The texel buffer length is not `width * height * 4`.
    BadLength {
        /// Length the size asks for.
        expected: usize,
        /// Length the caller passed.
        got: usize,
    },
    /// The codec refused the encoded image; bounded detail.
    Decode(String),
}

impl fmt::Display for ThumbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySize { width, height } => {
                write!(f, "thumb: {width}x{height} needs a size of at least 1")
            }
            Self::TooLarge { width, height } => write!(
                f,
                "thumb: {width}x{height} exceeds the {MAX_SOURCE_EDGE} px source edge"
            ),
            Self::BadLength { expected, got } => {
                write!(f, "thumb: buffer holds {got} bytes, size needs {expected}")
            }
            Self::Decode(detail) => write!(f, "thumb: codec refused: {detail}"),
        }
    }
}

impl std::error::Error for ThumbError {}

/// Texel bytes a `width * height` RGBA8 buffer holds; `None` when the size
/// overflows this machine's `usize`.
fn pixel_bytes(width: u32, height: u32) -> Option<usize> {
    let width = usize::try_from(width).ok()?;
    let height = usize::try_from(height).ok()?;
    width.checked_mul(height)?.checked_mul(CHANNELS)
}

/// One source span of a destination pixel: `[start, end)` along one axis.
fn span(dest: u32, src: u32) -> (usize, usize) {
    let edge = THUMB_EDGE;
    let start = u64::from(dest) * u64::from(src) / u64::from(edge);
    let mut end = u64::from(dest + 1) * u64::from(src) / u64::from(edge);
    if end <= start {
        end = start + 1;
    }
    let end = end.min(u64::from(src));
    (
        usize::try_from(start).unwrap_or(0),
        usize::try_from(end).unwrap_or(1),
    )
}

/// Cook one 128 px RGBA8 thumbnail from row-major RGBA8 source texels.
///
/// Each destination texel is the integer average of the source rectangle that
/// maps onto it, so a flat source stays that colour and a two-tone source keeps
/// both sides. Same pixels, same thumbnail.
pub fn cook_thumbnail(rgba: &[u8], width: u32, height: u32) -> Result<CookedThumb, ThumbError> {
    if width == 0 || height == 0 {
        return Err(ThumbError::EmptySize { width, height });
    }
    if width > MAX_SOURCE_EDGE || height > MAX_SOURCE_EDGE {
        return Err(ThumbError::TooLarge { width, height });
    }
    let expected = pixel_bytes(width, height).ok_or(ThumbError::TooLarge { width, height })?;
    if rgba.len() != expected {
        return Err(ThumbError::BadLength {
            expected,
            got: rgba.len(),
        });
    }
    let out_axis = usize::try_from(THUMB_EDGE).unwrap_or(0);
    let mut out = vec![0u8; THUMB_BYTES];
    for y in 0..THUMB_EDGE {
        let (y0, y1) = span(y, height);
        for x in 0..THUMB_EDGE {
            let (x0, x1) = span(x, width);
            let mut sum = [0u64; CHANNELS];
            let mut count = 0u64;
            for row in y0..y1 {
                let base = row * usize::try_from(width).unwrap_or(0) * CHANNELS;
                for col in x0..x1 {
                    let at = base + col * CHANNELS;
                    for (channel, total) in sum.iter_mut().enumerate() {
                        *total += u64::from(rgba[at + channel]);
                    }
                    count += 1;
                }
            }
            let count = count.max(1);
            let out_y = usize::try_from(y).unwrap_or(0);
            let out_x = usize::try_from(x).unwrap_or(0);
            let at = (out_y * out_axis + out_x) * CHANNELS;
            for (channel, total) in sum.iter().enumerate() {
                let mean = total / count;
                out[at + channel] = u8::try_from(mean).unwrap_or(u8::MAX);
            }
        }
    }
    Ok(finish(out))
}

/// Cook the thumbnail of one encoded PNG or JPEG image, as an import carries it.
///
/// # Errors
/// [`ThumbError::Decode`] when the codec refuses the bytes, plus the same
/// refusals as [`cook_thumbnail`].
pub fn cook_thumbnail_image(bytes: &[u8]) -> Result<CookedThumb, ThumbError> {
    let decoded = image::load_from_memory(bytes)
        .map_err(|err| ThumbError::Decode(err.to_string()))?
        .to_rgba8();
    let (width, height) = decoded.dimensions();
    cook_thumbnail(decoded.as_raw(), width, height)
}

/// Seal cooked pixels with their content digest.
fn finish(rgba: Vec<u8>) -> CookedThumb {
    let digest = crate::registry::content_hash(&rgba);
    CookedThumb { rgba, digest }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A source split left/right: red on the left half, green on the right.
    fn split(width: u32, height: u32) -> Vec<u8> {
        let texels = usize::try_from(width * height).expect("a small source");
        let mut rgba = vec![0u8; texels * CHANNELS];
        for y in 0..height {
            for x in 0..width {
                let at = usize::try_from(y * width + x).expect("a small source") * CHANNELS;
                let left = x < width / 2;
                rgba[at] = if left { 255 } else { 0 };
                rgba[at + 1] = if left { 0 } else { 255 };
                rgba[at + 3] = 255;
            }
        }
        rgba
    }

    #[test]
    fn a_cook_gives_the_128_px_budget_twice_identical() {
        let source = split(512, 512);
        let first = cook_thumbnail(&source, 512, 512).expect("cooks");
        assert_eq!(first.rgba.len(), THUMB_BYTES);
        assert_eq!((first.width(), first.height()), (128, 128));
        assert_eq!(first.digest, crate::registry::content_hash(&first.rgba));
        let second = cook_thumbnail(&source, 512, 512).expect("cooks again");
        assert_eq!(first, second, "same texels, same thumbnail");
        // Both halves survive: the leftmost texel is red, the rightmost green.
        assert!(first.rgba[0] > 200 && first.rgba[1] < 55);
        let last = (127 * 128 + 127) * 4;
        assert!(first.rgba[last + 1] > 200 && first.rgba[last] < 55);
    }

    #[test]
    fn a_small_source_upsamples_and_stays_flat() {
        let source = [200u8, 100, 50, 255].repeat(4);
        let thumb = cook_thumbnail(&source, 2, 2).expect("cooks");
        assert_eq!(thumb.rgba.len(), THUMB_BYTES);
        for texel in thumb.rgba.chunks_exact(4) {
            assert_eq!(texel, [200, 100, 50, 255]);
        }
    }

    #[test]
    fn bad_sources_refuse_typed() {
        assert_eq!(
            cook_thumbnail(&[], 0, 8),
            Err(ThumbError::EmptySize {
                width: 0,
                height: 8
            })
        );
        assert_eq!(
            cook_thumbnail(&[], MAX_SOURCE_EDGE + 1, 1),
            Err(ThumbError::TooLarge {
                width: MAX_SOURCE_EDGE + 1,
                height: 1
            })
        );
        assert_eq!(
            cook_thumbnail(&[0; 11], 2, 2),
            Err(ThumbError::BadLength {
                expected: 16,
                got: 11
            })
        );
        let err = cook_thumbnail_image(b"not an image").unwrap_err();
        assert!(matches!(err, ThumbError::Decode(_)), "{err}");
    }
}
