//! texture_cook: offline BC cook over `bc` plus KTX2 emit.
//! Owner: `engine-assets`; consumer: `crates/assets/tests/it/blockcomp_cook.rs`
//! cook-twice gate (current byte-identical proof).
//! Units: texels in, KTX2 bytes out. Same pixels plus slot emit identical
//! bytes via index-order loops and sorted picks, no clock. Typed [`BcError`].
//! PORT(hasenbanck/block_compression@f81ea7551b8fff47fb4e16d02a3077c0aba20a90): block decisions.
//! PORT(BinomialLLC/basis_universal@99f52d63aa6799cbdaecfe977111dc5ec3b31d47): KTX2 shape.

use crate::bc::{encode_image, BcError, BcFormat, BcSlot};
use crate::import::KTX2_MAGIC;
use crate::registry::content_hash;

/// Cooked texture: picked shape, raw blocks, KTX2 bytes, digest of KTX2.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CookedImage {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Picked BC shape.
    pub format: BcFormat,
    /// Block bytes in row-major order.
    pub blocks: Vec<u8>,
    /// KTX2 container (80-byte header plus blocks).
    pub ktx2: Vec<u8>,
    /// `content_hash` of `ktx2`, x2 identical.
    pub digest: u64,
}

/// Pick the BC shape for slots whose shape never depends on alpha:
/// normals to BC5, metallic-roughness to BC4. BaseColor returns `None`
/// so the caller fuses its alpha scan into the BC1 encode attempt.
fn pick_format(slot: BcSlot) -> Option<BcFormat> {
    match slot {
        BcSlot::BaseColor => None,
        BcSlot::Normal => Some(BcFormat::Bc5),
        BcSlot::MetallicRoughness => Some(BcFormat::Bc4),
    }
}

/// Wrap blocks in our KTX2 container (80-byte header, under 256 bytes).
fn emit_ktx2(width: u32, height: u32, blocks: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; 80];
    out[0..12].copy_from_slice(&KTX2_MAGIC);
    out[20..24].copy_from_slice(&width.to_le_bytes());
    out[24..28].copy_from_slice(&height.to_le_bytes());
    out[36..40].copy_from_slice(&1u32.to_le_bytes());
    out[40..44].copy_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(blocks);
    out
}

/// Cook RGBA (`width * height * 4`, row-major) by slot into blocks plus KTX2.
/// Loud on empty, bad dims, length mismatch, oversize, or BC1 alpha.
/// BaseColor picks its shape with a fused scan: try BC1 first (its
/// per-block alpha check is the scan), fall back to BC7 only on
/// transparent texels, so opaque images encode once with byte-identical
/// bytes to the old pre-scan pick.
pub fn cook_image(
    rgba: &[u8],
    width: u32,
    height: u32,
    slot: BcSlot,
) -> Result<CookedImage, BcError> {
    if rgba.is_empty() {
        return Err(BcError::EmptyImage);
    }
    if let Some(format) = pick_format(slot) {
        let encoded = encode_image(rgba, width, height, format, slot)?;
        let ktx2 = emit_ktx2(width, height, &encoded.bytes);
        let digest = content_hash(&ktx2);
        return Ok(CookedImage {
            width,
            height,
            format,
            blocks: encoded.bytes,
            ktx2,
            digest,
        });
    }
    match encode_image(rgba, width, height, BcFormat::Bc1, slot) {
        Ok(encoded) => {
            let ktx2 = emit_ktx2(width, height, &encoded.bytes);
            let digest = content_hash(&ktx2);
            Ok(CookedImage {
                width,
                height,
                format: BcFormat::Bc1,
                blocks: encoded.bytes,
                ktx2,
                digest,
            })
        }
        Err(BcError::AlphaInBc1 { .. }) => {
            let encoded = encode_image(rgba, width, height, BcFormat::Bc7, slot)?;
            let ktx2 = emit_ktx2(width, height, &encoded.bytes);
            let digest = content_hash(&ktx2);
            Ok(CookedImage {
                width,
                height,
                format: BcFormat::Bc7,
                blocks: encoded.bytes,
                ktx2,
                digest,
            })
        }
        Err(other) => Err(other),
    }
}
