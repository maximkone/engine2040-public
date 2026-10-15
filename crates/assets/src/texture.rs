//! texture cook: encoded bytes to [`CookedTexture`].
//!
//! Consumer: `engine-render` uploads the mip chain; identity stays with the
//! registry ([`content_hash`]). Units: texels in, linear `0..=1` floats out.
//! `BaseColor` sources are sRGB and are linearized on cook; every other slot
//! is linear data and is only normalized (`byte / 255`). The slot-to-encoding
//! map is exhaustive, so no color space is ever guessed. Mips are box-filter
//! averages down to 1x1. All input is bounded; failures are typed
//! [`CookError`], never panics.

use super::bc::srgb_byte_to_linear;
use super::cooked::{CookError, CookedTexture, Mip, TextureEncoding, TextureSlot};
use super::registry::content_hash;
use super::wire::bound_chars;

/// Max accepted encoded input (16 MiB); larger inputs are rejected bounded.
pub const MAX_TEXTURE_BYTES: usize = 16 * 1024 * 1024;
/// Max width or height in texels; larger decodes are rejected.
pub const MAX_TEXTURE_DIM: u32 = 4096;
/// Max texel count (`4096 * 4096`); larger decodes are rejected.
pub const MAX_TEXTURE_PIXELS: usize = 16_777_216;
/// Cap on mip levels (`4096` down to `1` is 13); longer chains are rejected.
pub const MAX_MIPS: usize = 13;
/// Bound for decoder detail strings carried in errors.
pub const MAX_DECODE_DETAIL_CHARS: usize = 256;

/// Stable texture identity for encoded bytes: the registry content-hash
/// canon, so identical bytes keep the same id across recooks.
pub fn texture_content_id(bytes: &[u8]) -> u64 {
    content_hash(bytes)
}

/// Truncate decoder detail to at most `MAX_DECODE_DETAIL_CHARS` chars.
fn bound_detail(text: &str) -> String {
    bound_chars(text, MAX_DECODE_DETAIL_CHARS)
}

/// Encoding for a slot. Exhaustive on purpose: adding a slot without
/// extending this map is a compile error, never a silent guess.
fn encoding_for(slot: TextureSlot) -> TextureEncoding {
    match slot {
        TextureSlot::BaseColor => TextureEncoding::Srgb,
        TextureSlot::MetallicRoughness => TextureEncoding::Linear,
        TextureSlot::Normal => TextureEncoding::Linear,
        TextureSlot::Occlusion => TextureEncoding::Linear,
        TextureSlot::Emissive => TextureEncoding::Srgb,
    }
}

/// Box-filter 2x2 downsample; odd edges average the texels that exist.
/// `src` is never empty, so the divisor is always 1..=4.
fn downsample(src: &Mip) -> Mip {
    let width = src.width.max(1);
    let height = src.height.max(1);
    let next_width = (width / 2).max(1);
    let next_height = (height / 2).max(1);
    let mut texels = Vec::with_capacity((next_width * next_height) as usize);
    for y in 0..next_height {
        for x in 0..next_width {
            let mut sum = [0.0f32; 4];
            let mut count = 0u32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let sx = x * 2 + dx;
                    let sy = y * 2 + dy;
                    if sx < width && sy < height {
                        let texel = src.texels[(sy * width + sx) as usize];
                        sum[0] += texel[0];
                        sum[1] += texel[1];
                        sum[2] += texel[2];
                        sum[3] += texel[3];
                        count += 1;
                    }
                }
            }
            let divisor = count.max(1) as f32;
            texels.push([
                sum[0] / divisor,
                sum[1] / divisor,
                sum[2] / divisor,
                sum[3] / divisor,
            ]);
        }
    }
    Mip {
        width: next_width,
        height: next_height,
        texels,
    }
}

/// Cook encoded texture bytes into linear texels plus a mip chain.
///
/// Decodes with `image`, validates bounds, converts to linear `RGBA`
/// floats (`BaseColor` RGB linearized, alpha and linear slots normalized),
/// then box-filters down to 1x1. Deterministic: same bytes plus slot give
/// bit-identical texels.
pub fn cook_texture(bytes: &[u8], slot: TextureSlot) -> Result<CookedTexture, CookError> {
    if bytes.is_empty() {
        return Err(CookError::TextureDecode("empty input".to_string()));
    }
    if bytes.len() > MAX_TEXTURE_BYTES {
        return Err(CookError::TextureTooLarge { count: bytes.len() });
    }
    let encoding = encoding_for(slot);
    let image = image::load_from_memory(bytes)
        .map_err(|e| CookError::TextureDecode(bound_detail(&format!("{e:?}"))))?;
    let rgba = image.to_rgba8();
    let (width, height) = (rgba.width(), rgba.height());
    if width == 0 || height == 0 || width > MAX_TEXTURE_DIM || height > MAX_TEXTURE_DIM {
        return Err(CookError::BadTextureDimensions { width, height });
    }
    let pixels = width as usize * height as usize;
    if pixels > MAX_TEXTURE_PIXELS {
        return Err(CookError::TextureTooLarge { count: pixels });
    }
    let srgb = encoding == TextureEncoding::Srgb;
    let mut base = Vec::with_capacity(pixels);
    for pixel in rgba.pixels() {
        let [r, g, b, a] = pixel.0;
        let (r, g, bl) = if srgb {
            (
                srgb_byte_to_linear(r),
                srgb_byte_to_linear(g),
                srgb_byte_to_linear(b),
            )
        } else {
            (
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
            )
        };
        base.push([r, g, bl, f32::from(a) / 255.0]);
    }
    let mut mips = vec![Mip {
        width,
        height,
        texels: base,
    }];
    while mips.last().is_some_and(|m| m.width > 1 || m.height > 1) {
        if mips.len() >= MAX_MIPS {
            return Err(CookError::TextureTooLarge {
                count: mips.len() + 1,
            });
        }
        let next = downsample(mips.last().expect("chain never empty"));
        mips.push(next);
    }
    Ok(CookedTexture {
        width,
        height,
        mips,
        slot,
        encoding,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode flat RGBA rows as PNG bytes via the `image` workspace dep.
    fn png_bytes(width: u32, height: u32, texel: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let image: image::RgbaImage =
            image::ImageBuffer::from_fn(width, height, |x, y| image::Rgba(texel(x, y)));
        let dynamic = image::DynamicImage::ImageRgba8(image);
        let mut out = Vec::new();
        {
            let mut cursor = std::io::Cursor::new(&mut out);
            dynamic
                .write_to(&mut cursor, image::ImageFormat::Png)
                .expect("test png must encode");
        }
        out
    }

    #[test]
    fn srgb_vs_linear_slot_tagging() {
        // Mid-grey separates the curves: sRGB 128 linearizes to ~0.216,
        // linear data normalizes to ~0.502. White/black agree by construction.
        let bytes = png_bytes(2, 2, |_, _| [128, 128, 128, 255]);
        let srgb = cook_texture(&bytes, TextureSlot::BaseColor).expect("srgb cooks");
        let linear = cook_texture(&bytes, TextureSlot::MetallicRoughness).expect("linear cooks");
        let normal = cook_texture(&bytes, TextureSlot::Normal).expect("normal cooks");
        assert_eq!(srgb.encoding, TextureEncoding::Srgb);
        assert_eq!(linear.encoding, TextureEncoding::Linear);
        assert_eq!(normal.encoding, TextureEncoding::Linear);
        assert_eq!(srgb.slot, TextureSlot::BaseColor);
        assert_eq!(linear.slot, TextureSlot::MetallicRoughness);
        assert_eq!(normal.slot, TextureSlot::Normal);
        let s = srgb.mips[0].texels[0][0];
        let l = linear.mips[0].texels[0][0];
        assert!(
            (l - 128.0 / 255.0).abs() < 1e-6,
            "linear slot normalizes, got {l}"
        );
        assert!(
            (s - 0.215_86).abs() < 1e-3,
            "sRGB slot linearizes 128 to ~0.216, got {s}"
        );
        assert!(l - s > 0.2, "curves must differ: linear {l} srgb {s}");
        // Alpha is never linearized.
        assert!((srgb.mips[0].texels[0][3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn mip_chain_halves_to_1x1() {
        // Non-power-of-two exercises odd-edge averaging: 5x3 -> 2x1 -> 1x1.
        let bytes = png_bytes(5, 3, |x, y| [(x * 51) as u8, (y * 127) as u8, 64, 255]);
        let cooked = cook_texture(&bytes, TextureSlot::Normal).expect("mips cook");
        assert_eq!((cooked.width, cooked.height), (5, 3));
        assert_eq!(cooked.mips.len(), 3);
        let dims: Vec<(u32, u32)> = cooked.mips.iter().map(|m| (m.width, m.height)).collect();
        assert_eq!(dims, vec![(5, 3), (2, 1), (1, 1)]);
        for mip in &cooked.mips {
            assert_eq!(
                mip.texels.len(),
                mip.width as usize * mip.height as usize,
                "mip {}x{} texel count",
                mip.width,
                mip.height
            );
            assert!(mip.texels.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
        }
        assert!(cooked.mips.len() <= MAX_MIPS);
        // Power-of-two chain halves exactly: 8x4 -> 4x2 -> 2x1 -> 1x1.
        let bytes = png_bytes(8, 4, |x, y| [x as u8 * 32, y as u8 * 63, 0, 255]);
        let cooked = cook_texture(&bytes, TextureSlot::BaseColor).expect("pow2 cooks");
        let dims: Vec<(u32, u32)> = cooked.mips.iter().map(|m| (m.width, m.height)).collect();
        assert_eq!(dims, vec![(8, 4), (4, 2), (2, 1), (1, 1)]);
        // Box average is exact on a 2x2 linear ramp: corners 0/255 average 0.5.
        let bytes = png_bytes(2, 2, |x, y| match (x, y) {
            (0, 0) => [0, 0, 0, 255],
            (1, 0) => [255, 0, 0, 255],
            (0, 1) => [0, 255, 0, 255],
            _ => [255, 255, 0, 255],
        });
        let cooked = cook_texture(&bytes, TextureSlot::Normal).expect("ramp cooks");
        assert_eq!(cooked.mips.len(), 2);
        let top = cooked.mips[1].texels[0];
        assert!((top[0] - 0.5).abs() < 1e-6, "red averages, got {}", top[0]);
        assert!(
            (top[1] - 0.5).abs() < 1e-6,
            "green averages, got {}",
            top[1]
        );
        assert!(top[2].abs() < 1e-6);
        assert!((top[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn malformed_and_oversize_rejected() {
        // Malformed inputs name the failure without panicking.
        for (label, bytes) in [
            ("empty", Vec::new()),
            ("garbage", b"not an image at all".to_vec()),
            ("truncated-png", vec![0x89, b'P', b'N', b'G', 0, 1, 2, 3]),
        ] {
            let err = cook_texture(&bytes, TextureSlot::BaseColor)
                .expect_err(&format!("{label} must fail"));
            assert!(
                matches!(err, CookError::TextureDecode(_)),
                "{label} wants TextureDecode, got {err:?}"
            );
            if let CookError::TextureDecode(detail) = &err {
                assert!(detail.chars().count() <= MAX_DECODE_DETAIL_CHARS);
            }
        }
        // Absurd encoded size is rejected before decoding.
        let huge = vec![0u8; MAX_TEXTURE_BYTES + 1];
        assert_eq!(
            cook_texture(&huge, TextureSlot::Normal),
            Err(CookError::TextureTooLarge { count: huge.len() })
        );
        // Absurd dimensions are rejected after decode (narrow strip, small
        // file, over-wide to trigger the dim bound without a huge alloc).
        let wide = png_bytes(MAX_TEXTURE_DIM + 1, 1, |_, _| [1, 2, 3, 255]);
        assert_eq!(
            cook_texture(&wide, TextureSlot::Normal),
            Err(CookError::BadTextureDimensions {
                width: MAX_TEXTURE_DIM + 1,
                height: 1
            })
        );
    }

    #[test]
    fn recook_stable_and_identity_is_content_hash() {
        let bytes = png_bytes(4, 4, |x, y| [(x * 64) as u8, (y * 64) as u8, 128, 255]);
        let first = cook_texture(&bytes, TextureSlot::BaseColor).expect("first cook");
        let second = cook_texture(&bytes, TextureSlot::BaseColor).expect("second cook");
        assert_eq!(first, second, "same bytes cook bit-identical");
        // Texture identity reuses the registry canon, not a second hash.
        assert_eq!(texture_content_id(&bytes), content_hash(&bytes));
        assert_eq!(texture_content_id(&bytes), texture_content_id(&bytes));
        // Distinct bytes cook distinctly.
        let other = png_bytes(4, 4, |x, y| [(x * 64 + 1) as u8, (y * 64) as u8, 128, 255]);
        assert_ne!(texture_content_id(&other), texture_content_id(&bytes));
        assert_ne!(
            cook_texture(&other, TextureSlot::BaseColor).expect("other cooks"),
            first
        );
    }

    #[test]
    fn five_slots_round_trip() {
        use super::super::cooked::{validate_material, CookedMaterial, TextureRef};
        // One 4x4 image per slot cooks bit-identical twice.
        for slot in [
            TextureSlot::BaseColor,
            TextureSlot::MetallicRoughness,
            TextureSlot::Normal,
            TextureSlot::Occlusion,
            TextureSlot::Emissive,
        ] {
            let bytes = png_bytes(4, 4, |x, y| [(x * 64) as u8, (y * 64) as u8, 128, 255]);
            let first = cook_texture(&bytes, slot).expect("slot cooks");
            let second = cook_texture(&bytes, slot).expect("slot recooks");
            assert_eq!(first, second, "{slot:?} cooks byte-equal x2");
        }
        // Emissive linearizes like BaseColor; Occlusion only normalizes.
        let bytes = png_bytes(4, 4, |_, _| [128, 128, 128, 255]);
        let emissive = cook_texture(&bytes, TextureSlot::Emissive).expect("emissive cooks");
        assert_eq!(emissive.encoding, TextureEncoding::Srgb);
        let e = emissive.mips[0].texels[0][0];
        assert!(
            (e - 0.215_86).abs() < 1e-3,
            "emissive 128 linearizes to ~0.216, got {e}"
        );
        let occlusion = cook_texture(&bytes, TextureSlot::Occlusion).expect("occlusion cooks");
        assert_eq!(occlusion.encoding, TextureEncoding::Linear);
        let o = occlusion.mips[0].texels[0][0];
        assert!(
            (o - 128.0 / 255.0).abs() < 1e-6,
            "occlusion normalizes without a curve, got {o}"
        );
        // A duplicate slot in one material is still refused.
        let dupe = CookedMaterial {
            textures: vec![
                TextureRef {
                    asset: 1,
                    slot: TextureSlot::Occlusion,
                },
                TextureRef {
                    asset: 2,
                    slot: TextureSlot::Occlusion,
                },
            ],
            ..CookedMaterial::default()
        };
        assert_eq!(
            validate_material(&dupe),
            Err(CookError::DuplicateSlot(TextureSlot::Occlusion))
        );
    }
}
