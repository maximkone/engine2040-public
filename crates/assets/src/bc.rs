//! bc: native BC1/BC3/BC4/BC5/BC7 block codec for in-cook texture compression.
//!
//! Owner: `engine-assets`; consumer: the texture cooker binds cooked mip
//! levels through [`encode_image`] (`texture_cook::cook_image` calls it). Units:
//! pixels in, bytes out. Every 4x4 block encodes alone, in row-major order, so the same
//! pixels plus format plus slot always emit identical bytes at any thread count. No third-party
//! encoder is vendored here; this file is std-only with zero crate imports
//! so it also compiles under standalone `rustc --test`. Refusals are typed
//! [`BcError`], never silent fallbacks or guessed formats.

use std::fmt;

/// Block edge in texels; every BC block covers 4x4 texels.
pub const BLOCK_EDGE: u32 = 4;
/// Texels per block (`4 * 4`).
pub const BLOCK_TEXELS: usize = 16;
/// Encoded bytes per BC1 block (two RGB565 endpoints + 32-bit indices).
pub const BC1_BLOCK_BYTES: usize = 8;
/// Encoded bytes per BC3 block (8-byte alpha block + 8-byte color block).
pub const BC3_BLOCK_BYTES: usize = 16;
/// Encoded bytes per BC4 block (single-channel 8-entry palette).
pub const BC4_BLOCK_BYTES: usize = 8;
/// Encoded bytes per BC5 block (two BC4 blocks for R and G).
pub const BC5_BLOCK_BYTES: usize = 16;
/// Encoded bytes per BC7 block (128 bits: mode 6 holds both endpoints and 16 indices).
pub const BC7_BLOCK_BYTES: usize = 16;
/// Max width or height in texels; mirrors the texture cook bound.
pub const MAX_BC_DIM: u32 = 4096;
/// Max texel count (`4096 * 4096`); mirrors the texture cook bound.
pub const MAX_BC_TEXELS: usize = 16_777_216;

/// BC shape shipped by the cook. No auto-detection: the caller names it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BcFormat {
    /// Opaque RGB, 8 bytes per 4x4 block, 4-color palette (`c0 > c1`).
    Bc1,
    /// RGBA, 16 bytes per block: explicit-alpha block plus a BC1 color block.
    Bc3,
    /// Single-channel data, 8 bytes per block (R through an 8-entry palette).
    Bc4,
    /// Dual-channel data, 16 bytes per block (R and G as two BC4 blocks).
    Bc5,
    /// RGBA high quality, 16 bytes per block: BC7 mode 6 (one subset, 7-bit endpoints).
    Bc7,
}

/// Material slot driving the transfer rule. Local mirror of the texture
/// cook's `TextureSlot` map (kept dependency-free so this file stays std-only):
/// `BaseColor` is sRGB, every other slot is linear data.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BcSlot {
    /// sRGB albedo; error metric runs on linearized values.
    BaseColor,
    /// Linear metallic-roughness data; bytes normalized as `byte / 255`.
    MetallicRoughness,
    /// Linear normal-map data; bytes normalized as `byte / 255`.
    Normal,
}

/// Typed cook refusals; invalid input returns these, never a guessed output.
#[derive(Clone, PartialEq, Debug)]
pub enum BcError {
    /// Image has no texels at all.
    EmptyImage,
    /// Width or height is zero, exceeds [`MAX_BC_DIM`], or is not a
    /// multiple of [`BLOCK_EDGE`]; offending dims carried.
    BadDimensions {
        /// Width in texels.
        width: u32,
        /// Height in texels.
        height: u32,
    },
    /// Pixel buffer length disagrees with `width * height * 4`; lengths carried.
    LengthMismatch {
        /// `width * height * 4`.
        expected: usize,
        /// Bytes actually supplied.
        got: usize,
    },
    /// Texel count exceeds [`MAX_BC_TEXELS`]; count carried.
    TooLarge {
        /// `width * height` texel count.
        count: usize,
    },
    /// A BC7 block uses a mode other than 6, the only one this codec writes and reads.
    Bc7Mode,
    /// BC1 carries no alpha: any texel with alpha below opaque was refused
    /// instead of silently dropping the channel; culprit texel carried.
    AlphaInBc1 {
        /// Zero-based texel index holding the transparent texel.
        texel: usize,
        /// Offending alpha byte.
        alpha: u8,
    },
}

impl fmt::Display for BcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyImage => write!(f, "bc: empty image"),
            Self::BadDimensions { width, height } => write!(
                f,
                "bc: bad dimensions {width}x{height} (must be nonzero multiples of 4, at most {MAX_BC_DIM})"
            ),
            Self::LengthMismatch { expected, got } => {
                write!(f, "bc: length mismatch (expected {expected} bytes, got {got})")
            }
            Self::TooLarge { count } => {
                write!(f, "bc: image too large ({count} texels)")
            }
            Self::Bc7Mode => write!(f, "bc: BC7 block is not mode 6"),
            Self::AlphaInBc1 { texel, alpha } => write!(
                f,
                "bc: transparent texel {texel} (alpha {alpha}) refused in BC1; use BC3"
            ),
        }
    }
}

impl std::error::Error for BcError {}

/// Encoded image: base dims, format, and concatenated block bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BcImage {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Block shape used.
    pub format: BcFormat,
    /// `block_count * block_bytes` encoded bytes in row-major block order.
    pub bytes: Vec<u8>,
}

/// True for the sRGB albedo slot; mirrors the texture cook's slot-to-encoding map.
/// Exhaustive on purpose: a new slot is a compile error, never a guess.
pub fn is_srgb(slot: BcSlot) -> bool {
    match slot {
        BcSlot::BaseColor => true,
        BcSlot::MetallicRoughness | BcSlot::Normal => false,
    }
}

/// Bytes per block for a format.
pub fn block_bytes(format: BcFormat) -> usize {
    match format {
        BcFormat::Bc1 => BC1_BLOCK_BYTES,
        BcFormat::Bc3 => BC3_BLOCK_BYTES,
        BcFormat::Bc4 => BC4_BLOCK_BYTES,
        BcFormat::Bc5 => BC5_BLOCK_BYTES,
        BcFormat::Bc7 => BC7_BLOCK_BYTES,
    }
}

/// Block count for validated dims.
pub fn block_count(width: u32, height: u32) -> usize {
    (width / BLOCK_EDGE) as usize * (height / BLOCK_EDGE) as usize
}

/// Encoded length for validated dims plus format.
pub fn encoded_len(width: u32, height: u32, format: BcFormat) -> usize {
    block_count(width, height) * block_bytes(format)
}

/// Validate dims loudly: nonzero, multiples of 4, within budget.
pub fn validate_dims(width: u32, height: u32) -> Result<(), BcError> {
    if width == 0
        || height == 0
        || width > MAX_BC_DIM
        || height > MAX_BC_DIM
        || !width.is_multiple_of(BLOCK_EDGE)
        || !height.is_multiple_of(BLOCK_EDGE)
    {
        return Err(BcError::BadDimensions { width, height });
    }
    let count = width as usize * height as usize;
    if count > MAX_BC_TEXELS {
        return Err(BcError::TooLarge { count });
    }
    Ok(())
}

/// sRGB byte to linear float (alpha never passes through here). The texture
/// cook, the block encoder and the game's crowd tint share this one curve.
pub fn srgb_byte_to_linear(byte: u8) -> f32 {
    let v = f32::from(byte) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Channel weight for the endpoint/index error metric: linearized for
/// sRGB albedo, plain normalization for linear slots (texture cook rules).
fn channel_weight(byte: u8, srgb: bool) -> f32 {
    if srgb {
        srgb_byte_to_linear(byte)
    } else {
        f32::from(byte) / 255.0
    }
}

/// Pack 8-bit RGB into RGB565, rounding to nearest.
pub(crate) fn pack_rgb565(r: u8, g: u8, b: u8) -> u16 {
    let r5 = (u16::from(r) * 31 + 127) / 255;
    let g6 = (u16::from(g) * 63 + 127) / 255;
    let b5 = (u16::from(b) * 31 + 127) / 255;
    (r5 << 11) | (g6 << 5) | b5
}

/// Unpack RGB565 to 8-bit RGB (bit replication).
pub(crate) fn unpack_rgb565(packed: u16) -> [u8; 3] {
    let r5 = u32::from((packed >> 11) & 31);
    let g6 = u32::from((packed >> 5) & 63);
    let b5 = u32::from(packed & 31);
    let r = ((r5 * 255 + 15) / 31) as u8;
    let g = ((g6 * 255 + 31) / 63) as u8;
    let b = ((b5 * 255 + 15) / 31) as u8;
    [r, g, b]
}

/// Four-entry BC1 palette from unpacked endpoints (`c0 > c1` opaque path).
pub(crate) fn bc1_palette(c0: [u8; 3], c1: [u8; 3]) -> [[u8; 3]; 4] {
    let mut palette = [c0, c1, [0, 0, 0], [0, 0, 0]];
    for channel in 0..3 {
        let (a, b) = (u16::from(c0[channel]), u16::from(c1[channel]));
        palette[2][channel] = ((2 * a + b + 1) / 3) as u8;
        palette[3][channel] = ((a + 2 * b + 1) / 3) as u8;
    }
    palette
}

/// Squared distance between a texel and a palette entry under the slot rule.
fn palette_distance(texel: [u8; 4], entry: [u8; 3], srgb: bool) -> f32 {
    let mut sum = 0.0f32;
    for channel in 0..3 {
        let a = channel_weight(texel[channel], srgb);
        let b = channel_weight(entry[channel], srgb);
        let d = a - b;
        sum += d * d;
    }
    sum
}

/// Nearest palette index for one texel; ties break to the lower index so
/// the choice is deterministic.
fn nearest_index(texel: [u8; 4], palette: &[[u8; 3]; 4], srgb: bool) -> u32 {
    let mut best = 0u32;
    let mut best_dist = palette_distance(texel, palette[0], srgb);
    for (index, entry) in palette.iter().enumerate().skip(1) {
        let dist = palette_distance(texel, *entry, srgb);
        if dist < best_dist {
            best_dist = dist;
            best = index as u32;
        }
    }
    best
}

/// Encode one 4x4 RGBA block as BC1 (8 bytes). Refuses any texel with
/// alpha below opaque instead of silently dropping the channel.
pub fn encode_bc1_block(
    texels: &[[u8; 4]; BLOCK_TEXELS],
    slot: BcSlot,
) -> Result<[u8; 8], BcError> {
    for (index, texel) in texels.iter().enumerate() {
        if texel[3] != 255 {
            return Err(BcError::AlphaInBc1 {
                texel: index,
                alpha: texel[3],
            });
        }
    }
    let srgb = is_srgb(slot);
    // Deterministic endpoints: per-channel min/max over the block, then
    // order so packed c0 > packed c1 (opaque 4-color path). Ties fall to
    // the channel-order pick below, never to a hash or clock.
    let mut lo = [255u8; 3];
    let mut hi = [0u8; 3];
    for texel in texels {
        for channel in 0..3 {
            if texel[channel] < lo[channel] {
                lo[channel] = texel[channel];
            }
            if texel[channel] > hi[channel] {
                hi[channel] = texel[channel];
            }
        }
    }
    let mut c0 = pack_rgb565(hi[0], hi[1], hi[2]);
    let mut c1 = pack_rgb565(lo[0], lo[1], lo[2]);
    if c0 <= c1 {
        if c0 == c1 {
            // Flat block: nudge one endpoint so the opaque path survives
            // with a nonzero span; direction is fixed, never guessed.
            if c0 > 0 {
                c1 -= 1;
            } else {
                c0 += 1;
            }
        } else {
            core::mem::swap(&mut c0, &mut c1);
        }
    }
    let palette = bc1_palette(unpack_rgb565(c0), unpack_rgb565(c1));
    let mut indices = 0u32;
    for (i, texel) in texels.iter().enumerate() {
        indices |= nearest_index(*texel, &palette, srgb) << (2 * i);
    }
    let mut out = [0u8; 8];
    out[0..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    out[4..8].copy_from_slice(&indices.to_le_bytes());
    Ok(out)
}

/// Decode one BC1 block to 16 RGBA texels (alpha always opaque here).
pub fn decode_bc1_block(block: &[u8; 8]) -> [[u8; 4]; BLOCK_TEXELS] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    let palette = bc1_palette(unpack_rgb565(c0), unpack_rgb565(c1));
    let mut out = [[0u8; 4]; BLOCK_TEXELS];
    for (i, texel) in out.iter_mut().enumerate() {
        let entry = palette[((indices >> (2 * i)) & 3) as usize];
        *texel = [entry[0], entry[1], entry[2], 255];
    }
    out
}

/// Eight-entry alpha palette from the explicit endpoints.
fn alpha_palette(a0: u8, a1: u8) -> [u8; 8] {
    let mut palette = [0u8; 8];
    palette[0] = a0;
    palette[1] = a1;
    if a0 > a1 {
        for i in 2u16..8 {
            palette[usize::from(i)] =
                (((8 - i) * u16::from(a0) + (i - 1) * u16::from(a1) + 3) / 7) as u8;
        }
    } else {
        for i in 2u16..6 {
            palette[usize::from(i)] =
                (((6 - i) * u16::from(a0) + (i - 1) * u16::from(a1) + 2) / 5) as u8;
        }
        palette[6] = 0;
        palette[7] = 255;
    }
    palette
}

/// Encode one 4x4 block as BC3 (16 bytes): explicit-alpha block plus color.
pub fn encode_bc3_block(texels: &[[u8; 4]; BLOCK_TEXELS], slot: BcSlot) -> [u8; 16] {
    let mut lo = 255u8;
    let mut hi = 0u8;
    for texel in texels {
        if texel[3] < lo {
            lo = texel[3];
        }
        if texel[3] > hi {
            hi = texel[3];
        }
    }
    let (a0, a1) = (hi, lo);
    let palette = alpha_palette(a0, a1);
    let mut packed: u64 = 0;
    for (i, texel) in texels.iter().enumerate() {
        let mut best = 0u64;
        let mut best_dist = i32::from(u16::MAX);
        for (index, entry) in palette.iter().enumerate() {
            let dist = (i32::from(texel[3]) - i32::from(*entry)).abs();
            if dist < best_dist {
                best_dist = dist;
                best = index as u64;
            }
        }
        packed |= best << (3 * i);
    }
    // Color half reuses the opaque BC1 path on RGB with alpha forced opaque.
    let mut rgb = [[0u8; 4]; BLOCK_TEXELS];
    for (dst, src) in rgb.iter_mut().zip(texels.iter()) {
        *dst = [src[0], src[1], src[2], 255];
    }
    // All RGB channels survive (alpha was split off), so this cannot fail.
    let color = encode_bc1_block(&rgb, slot).unwrap_or_default();
    let mut out = [0u8; 16];
    out[0] = a0;
    out[1] = a1;
    let alpha_bytes = packed.to_le_bytes();
    out[2..8].copy_from_slice(&alpha_bytes[0..6]);
    out[8..16].copy_from_slice(&color);
    out
}

/// Decode one BC3 block to 16 RGBA texels.
pub fn decode_bc3_block(block: &[u8; 16]) -> [[u8; 4]; BLOCK_TEXELS] {
    let palette = alpha_palette(block[0], block[1]);
    let mut packed: u64 = 0;
    for (i, byte) in block[2..8].iter().enumerate() {
        packed |= u64::from(*byte) << (8 * i);
    }
    let mut color = [0u8; 8];
    color.copy_from_slice(&block[8..16]);
    let mut out = decode_bc1_block(&color);
    for (i, texel) in out.iter_mut().enumerate() {
        texel[3] = palette[((packed >> (3 * i)) & 7) as usize];
    }
    out
}

/// Encode one 4x4 block as BC4 (8 bytes): R through an 8-entry palette.
/// Reuses the BC3 alpha path, so endpoint picks stay sorted and indexed.
pub fn encode_bc4_block(texels: &[[u8; 4]; BLOCK_TEXELS], slot: BcSlot) -> [u8; 8] {
    let mut moved = [[0u8; 4]; BLOCK_TEXELS];
    for (dst, src) in moved.iter_mut().zip(texels.iter()) {
        *dst = [src[1], src[2], src[3], src[0]];
    }
    let full = encode_bc3_block(&moved, slot);
    let mut out = [0u8; 8];
    out.copy_from_slice(&full[0..8]);
    out
}

/// Decode one BC4 block: R from the palette, G/B zero, A opaque.
pub fn decode_bc4_block(block: &[u8; 8]) -> [[u8; 4]; BLOCK_TEXELS] {
    let mut full = [0u8; 16];
    full[0..8].copy_from_slice(block);
    let back = decode_bc3_block(&full);
    let mut out = [[0u8; 4]; BLOCK_TEXELS];
    for (dst, src) in out.iter_mut().zip(back.iter()) {
        *dst = [src[3], 0, 0, 255];
    }
    out
}

/// Encode one 4x4 block as BC5 (16 bytes): R and G as two BC4 blocks.
pub fn encode_bc5_block(texels: &[[u8; 4]; BLOCK_TEXELS], slot: BcSlot) -> [u8; 16] {
    let red = encode_bc4_block(texels, slot);
    let mut swapped = [[0u8; 4]; BLOCK_TEXELS];
    for (dst, src) in swapped.iter_mut().zip(texels.iter()) {
        *dst = [src[1], src[0], src[2], src[3]];
    }
    let green = encode_bc4_block(&swapped, slot);
    let mut out = [0u8; 16];
    out[0..8].copy_from_slice(&red);
    out[8..16].copy_from_slice(&green);
    out
}

/// Decode one BC5 block: R/G from the halves, B neutral, A opaque.
pub fn decode_bc5_block(block: &[u8; 16]) -> [[u8; 4]; BLOCK_TEXELS] {
    let mut first = [0u8; 8];
    let mut second = [0u8; 8];
    first.copy_from_slice(&block[0..8]);
    second.copy_from_slice(&block[8..16]);
    let red = decode_bc4_block(&first);
    let green = decode_bc4_block(&second);
    let mut out = [[0u8; 4]; BLOCK_TEXELS];
    for i in 0..BLOCK_TEXELS {
        out[i] = [red[i][0], green[i][0], 128, 255];
    }
    out
}

/// BC7 mode 6 weights for its 4-bit indices (Khronos data format spec, table of 16 weights).
const BC7_WEIGHTS: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];
/// Mode 6 starts with six zero bits and a one: the low byte of every block is `0x40`.
const BC7_MODE6: u128 = 1 << 6;
/// Refinement rounds of the endpoint fit; more rounds cost time and gain little.
const BC7_FIT_ROUNDS: usize = 4;

/// One endpoint pair as 8-bit RGBA, the value the decoder reconstructs.
type Bc7Endpoints = [[u8; 4]; 2];

/// Interpolate one channel between two 8-bit endpoint values with a BC7 weight.
fn bc7_lerp(e0: u8, e1: u8, weight: u32) -> u8 {
    let mixed = (64 - weight) * u32::from(e0) + weight * u32::from(e1) + 32;
    u8::try_from(mixed >> 6).unwrap_or(u8::MAX)
}

/// The 16 colors an endpoint pair spans, in index order.
fn bc7_palette(endpoints: &Bc7Endpoints) -> [[u8; 4]; 16] {
    let mut palette = [[0u8; 4]; 16];
    for (entry, weight) in palette.iter_mut().zip(BC7_WEIGHTS) {
        for (channel, value) in entry.iter_mut().enumerate() {
            *value = bc7_lerp(endpoints[0][channel], endpoints[1][channel], weight);
        }
    }
    palette
}

/// Squared byte-space distance between two RGBA values. BC7 is judged on the stored
/// bytes (what PSNR measures), for every slot.
fn bc7_distance(a: [u8; 4], b: [u8; 4]) -> u32 {
    (0..4)
        .map(|c| (i32::from(a[c]) - i32::from(b[c])).unsigned_abs().pow(2))
        .sum()
}

/// Nearest palette entry per texel (ties to the lower index) and the summed error.
fn bc7_assign(texels: &[[u8; 4]; BLOCK_TEXELS], palette: &[[u8; 4]; 16]) -> ([u8; 16], u32) {
    let mut indices = [0u8; 16];
    let mut total = 0u32;
    for (slot, texel) in indices.iter_mut().zip(texels) {
        let (mut best, mut best_distance) = (0u8, u32::MAX);
        for (index, entry) in (0u8..).zip(palette) {
            let distance = bc7_distance(*texel, *entry);
            if distance < best_distance {
                best = index;
                best_distance = distance;
            }
        }
        *slot = best;
        total += best_distance;
    }
    (indices, total)
}

/// `v` rounded to the nearest integer in `0..=max` (halves up), by binary search: no cast.
fn round_to_u8(v: f64, max: u8) -> u8 {
    let (mut lo, mut hi) = (0u8, max);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if v < f64::from(mid) + 0.5 {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo
}

/// Quantize a float endpoint to a 7-bit value per channel plus the shared p-bit that
/// gives the least error; returns the 7-bit values, the p-bit and the 8-bit endpoint.
fn bc7_quantize(endpoint: [f64; 4]) -> ([u8; 4], u8, [u8; 4]) {
    let mut best = ([0u8; 4], 0u8, [0u8; 4]);
    let mut best_error = f64::INFINITY;
    for pbit in 0..2u8 {
        let (mut q7, mut full, mut error) = ([0u8; 4], [0u8; 4], 0.0f64);
        for c in 0..4 {
            let q = round_to_u8((endpoint[c] - f64::from(pbit)) / 2.0, 127);
            q7[c] = q;
            full[c] = (q << 1) | pbit;
            error += (f64::from(full[c]) - endpoint[c]).powi(2);
        }
        if error < best_error {
            best_error = error;
            best = (q7, pbit, full);
        }
    }
    best
}

/// Principal axis of the block's RGBA cloud by a fixed number of power iterations.
fn bc7_axis(texels: &[[u8; 4]; BLOCK_TEXELS], mean: [f64; 4]) -> [f64; 4] {
    let mut cov = [[0.0f64; 4]; 4];
    for texel in texels {
        let d: [f64; 4] = std::array::from_fn(|c| f64::from(texel[c]) - mean[c]);
        for (i, row) in cov.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell += d[i] * d[j];
            }
        }
    }
    let mut axis = [0.5f64; 4];
    for _ in 0..8 {
        let next: [f64; 4] = std::array::from_fn(|i| (0..4).map(|j| cov[i][j] * axis[j]).sum());
        let length = next.iter().map(|v| v * v).sum::<f64>().sqrt();
        if length < 1e-9 {
            break;
        }
        axis = next.map(|v| v / length);
    }
    axis
}

/// Least-squares endpoints for the given index weights; `None` when every weight is the same
/// (the system is singular).
fn bc7_solve(texels: &[[u8; 4]; BLOCK_TEXELS], indices: &[u8; 16]) -> Option<([f64; 4], [f64; 4])> {
    let (mut aa, mut ab, mut bb) = (0.0f64, 0.0f64, 0.0f64);
    let (mut ta, mut tb) = ([0.0f64; 4], [0.0f64; 4]);
    for (texel, &index) in texels.iter().zip(indices) {
        let b = f64::from(BC7_WEIGHTS[usize::from(index)]) / 64.0;
        let a = 1.0 - b;
        aa += a * a;
        ab += a * b;
        bb += b * b;
        for c in 0..4 {
            ta[c] += a * f64::from(texel[c]);
            tb[c] += b * f64::from(texel[c]);
        }
    }
    let det = aa * bb - ab * ab;
    if det.abs() < 1e-9 {
        return None;
    }
    let e0 = std::array::from_fn(|c| ((bb * ta[c] - ab * tb[c]) / det).clamp(0.0, 255.0));
    let e1 = std::array::from_fn(|c| ((aa * tb[c] - ab * ta[c]) / det).clamp(0.0, 255.0));
    Some((e0, e1))
}

/// Encode one 4x4 RGBA block as BC7 mode 6 (16 bytes): one subset, 7-bit RGBA endpoints with
/// a p-bit each, 4-bit indices. The endpoints start at the extremes along the block's
/// principal axis, then a few least-squares rounds move them; the round with the least
/// error after quantization wins. Pure integer output of a fixed-order fit, so the same
/// texels always give the same bytes. `slot` does not change the metric (stored bytes).
pub fn encode_bc7_block(texels: &[[u8; 4]; BLOCK_TEXELS], _slot: BcSlot) -> [u8; 16] {
    let mean: [f64; 4] =
        std::array::from_fn(|c| texels.iter().map(|t| f64::from(t[c])).sum::<f64>() / 16.0);
    let axis = bc7_axis(texels, mean);
    let along = |t: &[u8; 4]| {
        (0..4)
            .map(|c| (f64::from(t[c]) - mean[c]) * axis[c])
            .sum::<f64>()
    };
    let (mut lo, mut hi) = (0usize, 0usize);
    for (i, texel) in texels.iter().enumerate() {
        if along(texel) < along(&texels[lo]) {
            lo = i;
        }
        if along(texel) > along(&texels[hi]) {
            hi = i;
        }
    }
    let fit = |e0: [f64; 4], e1: [f64; 4]| {
        let ((q0, p0, f0), (q1, p1, f1)) = (bc7_quantize(e0), bc7_quantize(e1));
        let (indices, error) = bc7_assign(texels, &bc7_palette(&[f0, f1]));
        (error, [q0, q1], [p0, p1], indices)
    };
    let mut best = fit(texels[lo].map(f64::from), texels[hi].map(f64::from));
    let mut next = bc7_solve(texels, &best.3);
    for _ in 0..BC7_FIT_ROUNDS {
        let Some((e0, e1)) = next else { break };
        let candidate = fit(e0, e1);
        next = bc7_solve(texels, &candidate.3);
        if candidate.0 < best.0 {
            best = candidate;
        }
    }
    let (_, mut q, mut p, mut indices) = best;
    // The first index drops its top bit: when it is set, swap the endpoints and flip all indices.
    if indices[0] >= 8 {
        q.swap(0, 1);
        p.swap(0, 1);
        indices = indices.map(|i| 15 - i);
    }
    let mut bits: u128 = BC7_MODE6;
    for (c, (&a, &b)) in q[0].iter().zip(&q[1]).enumerate() {
        bits |= u128::from(a) << (7 + 14 * c);
        bits |= u128::from(b) << (14 + 14 * c);
    }
    bits |= u128::from(p[0]) << 63;
    bits |= u128::from(p[1]) << 64;
    bits |= u128::from(indices[0]) << 65;
    for (i, &index) in indices.iter().enumerate().skip(1) {
        bits |= u128::from(index) << (68 + 4 * (i - 1));
    }
    bits.to_le_bytes()
}

/// Decode one BC7 block. Only mode 6, the one the encoder writes, is read; any other mode
/// returns `None` instead of a guess.
pub fn decode_bc7_block(block: &[u8; 16]) -> Option<[[u8; 4]; BLOCK_TEXELS]> {
    let bits = u128::from_le_bytes(*block);
    if bits & 0x7f != BC7_MODE6 {
        return None;
    }
    let field = |at: usize, width: usize| {
        u8::try_from((bits >> at) & ((1u128 << width) - 1)).unwrap_or(u8::MAX)
    };
    // Endpoint e holds channel c at bit 7 + 7 e + 14 c and shares the p-bit at 63 + e.
    let endpoints: Bc7Endpoints = [0, 1]
        .map(|e| std::array::from_fn(|c| (field(7 + 7 * e + 14 * c, 7) << 1) | field(63 + e, 1)));
    let palette = bc7_palette(&endpoints);
    let mut out = [[0u8; 4]; BLOCK_TEXELS];
    for (i, texel) in out.iter_mut().enumerate() {
        let index = if i == 0 {
            field(65, 3)
        } else {
            field(68 + 4 * (i - 1), 4)
        };
        *texel = palette[usize::from(index)];
    }
    Some(out)
}

/// Encode a full RGBA image (`width * height * 4` bytes, row-major) into
/// BC blocks in row-major block order. Loud on bad dims, length mismatch,
/// oversize input, or transparent texels under BC1; byte-identical x2.
pub fn encode_image(
    rgba: &[u8],
    width: u32,
    height: u32,
    format: BcFormat,
    slot: BcSlot,
) -> Result<BcImage, BcError> {
    if rgba.is_empty() {
        return Err(BcError::EmptyImage);
    }
    validate_dims(width, height)?;
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(BcError::LengthMismatch {
            expected,
            got: rgba.len(),
        });
    }
    let blocks_wide = (width / BLOCK_EDGE) as usize;
    let blocks_high = (height / BLOCK_EDGE) as usize;
    let mut bytes = Vec::with_capacity(blocks_wide * blocks_high * block_bytes(format));
    let stride = width as usize * 4;
    for by in 0..blocks_high {
        for bx in 0..blocks_wide {
            let mut block = [[0u8; 4]; BLOCK_TEXELS];
            for ty in 0..BLOCK_EDGE as usize {
                let row = (by * BLOCK_EDGE as usize + ty) * stride;
                for tx in 0..BLOCK_EDGE as usize {
                    let src = row + (bx * BLOCK_EDGE as usize + tx) * 4;
                    block[ty * 4 + tx] = [rgba[src], rgba[src + 1], rgba[src + 2], rgba[src + 3]];
                }
            }
            match format {
                BcFormat::Bc1 => bytes.extend_from_slice(&encode_bc1_block(&block, slot)?),
                BcFormat::Bc3 => bytes.extend_from_slice(&encode_bc3_block(&block, slot)),
                BcFormat::Bc4 => bytes.extend_from_slice(&encode_bc4_block(&block, slot)),
                BcFormat::Bc5 => bytes.extend_from_slice(&encode_bc5_block(&block, slot)),
                BcFormat::Bc7 => bytes.extend_from_slice(&encode_bc7_block(&block, slot)),
            }
        }
    }
    Ok(BcImage {
        width,
        height,
        format,
        bytes,
    })
}

/// Decode an encoded image back to RGBA bytes (row-major).
/// Test-only quality gate comparing against the uncompressed source.
#[cfg(test)]
pub fn decode_image(image: &BcImage) -> Result<Vec<u8>, BcError> {
    validate_dims(image.width, image.height)?;
    if image.bytes.len() != encoded_len(image.width, image.height, image.format) {
        return Err(BcError::LengthMismatch {
            expected: encoded_len(image.width, image.height, image.format),
            got: image.bytes.len(),
        });
    }
    let width = image.width as usize;
    let height = image.height as usize;
    let blocks_wide = width / BLOCK_EDGE as usize;
    let step = block_bytes(image.format);
    let mut rgba = vec![0u8; width * height * 4];
    for (index, chunk) in image.bytes.chunks_exact(step).enumerate() {
        let bx = index % blocks_wide;
        let by = index / blocks_wide;
        let texels = match image.format {
            BcFormat::Bc1 => {
                let mut block = [0u8; 8];
                block.copy_from_slice(chunk);
                decode_bc1_block(&block)
            }
            BcFormat::Bc3 => {
                let mut block = [0u8; 16];
                block.copy_from_slice(chunk);
                decode_bc3_block(&block)
            }
            BcFormat::Bc4 => {
                let mut block = [0u8; 8];
                block.copy_from_slice(chunk);
                decode_bc4_block(&block)
            }
            BcFormat::Bc5 => {
                let mut block = [0u8; 16];
                block.copy_from_slice(chunk);
                decode_bc5_block(&block)
            }
            BcFormat::Bc7 => {
                let mut block = [0u8; 16];
                block.copy_from_slice(chunk);
                decode_bc7_block(&block).ok_or(BcError::Bc7Mode)?
            }
        };
        for ty in 0..BLOCK_EDGE as usize {
            for tx in 0..BLOCK_EDGE as usize {
                let dst =
                    ((by * BLOCK_EDGE as usize + ty) * width + (bx * BLOCK_EDGE as usize + tx)) * 4;
                let texel = texels[ty * 4 + tx];
                rgba[dst..dst + 4].copy_from_slice(&texel);
            }
        }
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_rgba(width: u32, height: u32, texel: [u8; 4]) -> Vec<u8> {
        let mut out = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..width as usize * height as usize {
            out.extend_from_slice(&texel);
        }
        out
    }

    #[test]
    fn dims_refused_loudly() {
        // Non-multiples of 4 fail even when tiny; zero and oversize fail too.
        for (w, h) in [(5, 4), (4, 5), (6, 6), (3, 3), (0, 4), (4, 0)] {
            assert_eq!(
                validate_dims(w, h),
                Err(BcError::BadDimensions {
                    width: w,
                    height: h
                }),
                "{w}x{h} must fail"
            );
            assert_eq!(
                encode_image(&[0u8; 64], w, h, BcFormat::Bc1, BcSlot::Normal),
                Err(BcError::BadDimensions {
                    width: w,
                    height: h
                })
            );
        }
        assert_eq!(
            validate_dims(MAX_BC_DIM + 4, 4),
            Err(BcError::BadDimensions {
                width: MAX_BC_DIM + 4,
                height: 4
            })
        );
        assert!(matches!(
            encode_image(&[], 4, 4, BcFormat::Bc1, BcSlot::Normal),
            Err(BcError::EmptyImage)
        ));
        assert_eq!(
            encode_image(&[0u8; 63], 4, 4, BcFormat::Bc1, BcSlot::Normal),
            Err(BcError::LengthMismatch {
                expected: 64,
                got: 63
            })
        );
    }

    #[test]
    fn alpha_palette_interpolates_both_endpoint_orders() {
        // a0 > a1: six interpolated entries between the endpoints.
        assert_eq!(alpha_palette(255, 0), [255, 0, 219, 182, 146, 109, 73, 36]);
        // a0 <= a1: four interpolated entries plus explicit 0 and 255.
        assert_eq!(alpha_palette(0, 255), [0, 255, 51, 102, 153, 204, 0, 255]);
        assert_eq!(alpha_palette(7, 7), [7, 7, 7, 7, 7, 7, 0, 255]);
    }

    #[test]
    fn rgb565_unpacks_by_bit_replication() {
        assert_eq!(unpack_rgb565(0x0000), [0, 0, 0]);
        assert_eq!(unpack_rgb565(0xFFFF), [255, 255, 255]);
        assert_eq!(unpack_rgb565(0xF800), [255, 0, 0]);
        assert_eq!(unpack_rgb565(0x07E0), [0, 255, 0]);
        assert_eq!(unpack_rgb565(0x001F), [0, 0, 255]);
        assert_eq!(unpack_rgb565(0x8410), [132, 130, 132]);
    }

    #[test]
    fn bc1_transparency_refused_not_dropped() {
        // One translucent texel must name the culprit, never encode opaque.
        let mut block = [[200u8, 100, 50, 255]; BLOCK_TEXELS];
        block[7] = [200, 100, 50, 128];
        assert_eq!(
            encode_bc1_block(&block, BcSlot::BaseColor),
            Err(BcError::AlphaInBc1 {
                texel: 7,
                alpha: 128
            })
        );
        let rgba = solid_rgba(4, 4, [10, 20, 30, 0]);
        assert!(matches!(
            encode_image(&rgba, 4, 4, BcFormat::Bc1, BcSlot::Normal),
            Err(BcError::AlphaInBc1 { .. })
        ));
        // BC3 accepts the same texels, alpha channel intact.
        let encoded = encode_image(&rgba, 4, 4, BcFormat::Bc3, BcSlot::Normal).expect("bc3 cooks");
        assert_eq!(encoded.bytes.len(), BC3_BLOCK_BYTES);
        let decoded = decode_image(&encoded).expect("bc3 decodes");
        assert!(decoded.chunks_exact(4).all(|t| t[3] == 0));
    }

    #[test]
    fn encode_identical_bytes_twice() {
        // Same pixels + format + slot give identical bytes on repeat cooks.
        let mut rgba = Vec::new();
        for y in 0..8u32 {
            for x in 0..8u32 {
                rgba.extend_from_slice(&[
                    (x * 37) as u8,
                    (y * 53) as u8,
                    ((x + y) * 29) as u8,
                    255,
                ]);
            }
        }
        for format in [BcFormat::Bc1, BcFormat::Bc3] {
            let first = encode_image(&rgba, 8, 8, format, BcSlot::BaseColor).expect("first");
            let second = encode_image(&rgba, 8, 8, format, BcSlot::BaseColor).expect("second");
            assert_eq!(first.bytes, second.bytes, "{format:?} must repeat x2");
            assert_eq!(first.bytes.len(), encoded_len(8, 8, format));
        }
        // Linear slots repeat too.
        let first = encode_image(&rgba, 8, 8, BcFormat::Bc1, BcSlot::Normal).expect("first");
        let second = encode_image(&rgba, 8, 8, BcFormat::Bc1, BcSlot::Normal).expect("second");
        assert_eq!(first.bytes, second.bytes);
    }

    #[test]
    fn srgb_rules_match_a2_slots() {
        // Exhaustive slot map: only albedo is sRGB, exactly like the texture cook.
        assert!(is_srgb(BcSlot::BaseColor));
        assert!(!is_srgb(BcSlot::MetallicRoughness));
        assert!(!is_srgb(BcSlot::Normal));
        // Mid-grey separates the curves the same way the texture cook does.
        let linear = channel_weight(128, false);
        let srgb = channel_weight(128, true);
        assert!((linear - 128.0 / 255.0).abs() < 1e-6);
        assert!((srgb - 0.215_86).abs() < 1e-3);
        assert!(linear - srgb > 0.2);
        // The metric follows the slot: a gradient block encodes identically
        // x2 per slot and the two slots disagree on mid-grey ramps.
        let mut block = [[0u8; 4]; BLOCK_TEXELS];
        for (i, texel) in block.iter_mut().enumerate() {
            let v = (i as u8) * 17;
            *texel = [v, v, v, 255];
        }
        let srgb_block = encode_bc1_block(&block, BcSlot::BaseColor).expect("srgb");
        let linear_block = encode_bc1_block(&block, BcSlot::Normal).expect("linear");
        assert_eq!(
            srgb_block,
            encode_bc1_block(&block, BcSlot::BaseColor).expect("x2")
        );
        assert_ne!(
            srgb_block, linear_block,
            "sRGB vs linear metric must split the ramp"
        );
    }

    #[test]
    fn roundtrip_shapes_hold() {
        // Solid red survives BC1 exactly (endpoints span the flat color).
        let rgba = solid_rgba(4, 4, [255, 0, 0, 255]);
        let encoded = encode_image(&rgba, 4, 4, BcFormat::Bc1, BcSlot::Normal).expect("bc1");
        assert_eq!(encoded.bytes.len(), BC1_BLOCK_BYTES);
        let decoded = decode_image(&encoded).expect("decode");
        assert_eq!(decoded, rgba);
        // Alpha ramp survives BC3 within one palette step per texel.
        let mut ramp = Vec::new();
        for i in 0..16u8 {
            ramp.extend_from_slice(&[64, 128, 192, i * 17]);
        }
        let encoded = encode_image(&ramp, 4, 4, BcFormat::Bc3, BcSlot::Normal).expect("bc3");
        let decoded = decode_image(&encoded).expect("decode");
        for (src, dst) in ramp.chunks_exact(4).zip(decoded.chunks_exact(4)) {
            // 8-entry interpolation spans 255 in 7 steps, so quantization
            // error caps at half a step (ceil(255 / 14) = 19).
            assert!((i32::from(src[3]) - i32::from(dst[3])).abs() <= 19);
        }
    }

    #[test]
    fn block_layout_counts_hold() {
        assert_eq!(block_bytes(BcFormat::Bc1), BC1_BLOCK_BYTES);
        assert_eq!(block_bytes(BcFormat::Bc3), BC3_BLOCK_BYTES);
        assert_eq!(block_count(4, 4), 1);
        assert_eq!(block_count(8, 4), 2);
        assert_eq!(encoded_len(8, 8, BcFormat::Bc1), 4 * BC1_BLOCK_BYTES);
        assert_eq!(encoded_len(8, 8, BcFormat::Bc3), 4 * BC3_BLOCK_BYTES);
        // RGB565 roundtrips its primaries through bit replication.
        assert_eq!(unpack_rgb565(pack_rgb565(255, 0, 0)), [255, 0, 0]);
        assert_eq!(unpack_rgb565(pack_rgb565(0, 255, 0)), [0, 255, 0]);
    }

    /// Hash of two lattice coordinates and a seed, well mixed.
    fn hash2(x: u32, y: u32, seed: u32) -> u32 {
        let mut h = x.wrapping_mul(0x9E37_79B1)
            ^ y.wrapping_mul(0x85EB_CA6B)
            ^ seed.wrapping_mul(0xC2B2_AE35);
        h ^= h >> 15;
        h = h.wrapping_mul(0x2C1B_3C6D);
        h ^= h >> 12;
        h = h.wrapping_mul(0x297A_2D39);
        h ^ (h >> 15)
    }

    /// Smooth value noise with lattice cells of `cell` texels, in 0.0..=1.0.
    fn value_noise(x: u32, y: u32, cell: u32, seed: u32) -> f32 {
        let (gx, gy) = (x / cell, y / cell);
        let (fx, fy) = (
            (x % cell) as f32 / cell as f32,
            (y % cell) as f32 / cell as f32,
        );
        let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
        let at = |dx: u32, dy: u32| (hash2(gx + dx, gy + dy, seed) & 255) as f32 / 255.0;
        let top = at(0, 0) + (at(1, 0) - at(0, 0)) * smooth(fx);
        let bottom = at(0, 1) + (at(1, 1) - at(0, 1)) * smooth(fx);
        top + (bottom - top) * smooth(fy)
    }

    /// A stone-wall albedo: running-bond bricks with mortar lines, per-brick tint, four
    /// octaves of colour noise (cells of 64, 16, 4 and 2 texels) and a +-4 code grain, so
    /// blocks hold real fine detail.
    fn albedo_fixture(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                let row = y / 64;
                let shifted = x + if row % 2 == 0 { 0 } else { 64 };
                let mortar = y % 64 < 4 || shifted % 128 < 4;
                let tint = (hash2(shifted / 128, row, 7) & 31) as f32 - 16.0;
                let noise = 0.45 * value_noise(x, y, 64, 1)
                    + 0.25 * value_noise(x, y, 16, 2)
                    + 0.15 * value_noise(x, y, 4, 3)
                    + 0.15 * value_noise(x, y, 2, 4);
                let base = if mortar {
                    70.0
                } else {
                    120.0 + tint + 70.0 * noise
                };
                // Grain is independent per channel, like sensor or sculpt noise in a real map.
                let channel = |k: f32, seed: u32| {
                    let grain = (hash2(x, y, seed) % 9) as f32 - 4.0;
                    (base * k + grain).clamp(0.0, 255.0) as u8
                };
                out.extend_from_slice(&[channel(1.0, 9), channel(0.85, 10), channel(0.7, 11), 255]);
            }
        }
        out
    }

    fn psnr_rgb(a: &[u8], b: &[u8]) -> f64 {
        let (mut sum, mut n) = (0.0f64, 0.0f64);
        for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
            for c in 0..3 {
                let d = f64::from(p[c]) - f64::from(q[c]);
                sum += d * d;
                n += 1.0;
            }
        }
        10.0 * (255.0f64 * 255.0 / (sum / n).max(1e-9)).log10()
    }

    #[test]
    fn bc7_packs_mode_6_fields_where_the_spec_puts_them() {
        // A solid white block: every 7-bit endpoint field is 127, both p-bits are 1 (127 * 2 + 1
        // = 255), every index is 0. The field starts are the spec's: mode 7 bits, then
        // R0 R1 G0 G1 B0 B1 A0 A1 at 7, 14, 21 ... 56, p-bits at 63 and 64, indices from 65.
        let block = encode_bc7_block(&[[255; 4]; BLOCK_TEXELS], BcSlot::BaseColor);
        let mut want: u128 = 1 << 6;
        for start in [7, 14, 21, 28, 35, 42, 49, 56] {
            want |= 127 << start;
        }
        want |= (1 << 63) | (1 << 64);
        assert_eq!(u128::from_le_bytes(block), want);
        assert_eq!(decode_bc7_block(&block), Some([[255; 4]; BLOCK_TEXELS]));
    }

    #[test]
    fn bc7_reads_only_mode_6() {
        let mut block = encode_bc7_block(&[[10, 20, 30, 255]; BLOCK_TEXELS], BcSlot::BaseColor);
        block[0] = 0x01; // mode 0
        assert_eq!(decode_bc7_block(&block), None);
        let image = BcImage {
            width: 4,
            height: 4,
            format: BcFormat::Bc7,
            bytes: block.to_vec(),
        };
        assert_eq!(decode_image(&image), Err(BcError::Bc7Mode));
    }

    #[test]
    fn bc7_keeps_solid_and_two_color_blocks_exact() {
        // One p-bit serves all four channels of an endpoint, so a solid color is exact when its
        // channels share a parity, and within one code per channel when they do not.
        for solid in [
            [0, 0, 0, 0],
            [255, 255, 255, 255],
            [200, 100, 50, 254],
            [1, 3, 5, 7],
        ] {
            let block = encode_bc7_block(&[solid; BLOCK_TEXELS], BcSlot::BaseColor);
            assert_eq!(
                decode_bc7_block(&block),
                Some([solid; BLOCK_TEXELS]),
                "{solid:?}"
            );
        }
        let mixed = [200, 100, 51, 255];
        let block = encode_bc7_block(&[mixed; BLOCK_TEXELS], BcSlot::BaseColor);
        let decoded = decode_bc7_block(&block).expect("mode 6");
        assert!(decoded
            .iter()
            .all(|t| (0..4).all(|c| t[c].abs_diff(mixed[c]) <= 1)));
        // Texel 0 is the bright end: the encoder must flip endpoints and indices to keep the
        // first index under 8, and the picture must not change.
        let (bright, dark) = ([250, 240, 230, 254], [10, 20, 30, 254]);
        let mut texels = [dark; BLOCK_TEXELS];
        texels[0] = bright;
        texels[5] = bright;
        let block = encode_bc7_block(&texels, BcSlot::BaseColor);
        assert_eq!(decode_bc7_block(&block), Some(texels));
    }

    #[test]
    fn bc7_gradient_block_stays_within_a_few_codes() {
        let mut texels = [[0u8; 4]; BLOCK_TEXELS];
        for (i, t) in texels.iter_mut().enumerate() {
            let v = (i * 16) as u8;
            *t = [v, 255 - v, v / 2 + 40, 255];
        }
        let decoded = decode_bc7_block(&encode_bc7_block(&texels, BcSlot::BaseColor)).unwrap();
        for (a, b) in texels.iter().zip(&decoded) {
            for c in 0..4 {
                assert!(a[c].abs_diff(b[c]) <= 6, "{a:?} vs {b:?}");
            }
        }
    }

    /// Encode with `threads` scoped threads, each taking a band of block rows, and join the
    /// bands in order.
    fn encode_bands(rgba: &[u8], width: u32, height: u32, threads: u32) -> Vec<u8> {
        let rows = height / BLOCK_EDGE;
        let band = rows.div_ceil(threads);
        let stride = width as usize * 4;
        std::thread::scope(|scope| {
            let mut jobs = Vec::new();
            for first in (0..rows).step_by(band as usize) {
                let last = (first + band).min(rows);
                let part = &rgba[first as usize * 4 * stride..last as usize * 4 * stride];
                let h = (last - first) * BLOCK_EDGE;
                jobs.push(scope.spawn(move || {
                    encode_image(part, width, h, BcFormat::Bc7, BcSlot::BaseColor)
                        .expect("band encodes")
                        .bytes
                }));
            }
            jobs.into_iter()
                .flat_map(|j| j.join().expect("band thread"))
                .collect()
        })
    }

    #[test]
    fn bc7_albedo_1024_reaches_40_db_is_4x_smaller_and_threads_agree() {
        let (w, h) = (1024, 1024);
        let rgba = albedo_fixture(w, h);
        let image = encode_image(&rgba, w, h, BcFormat::Bc7, BcSlot::BaseColor).expect("encodes");
        // 8 bits per texel: a quarter of RGBA8.
        assert_eq!(image.bytes.len() * 4, rgba.len());
        assert_eq!(image.bytes.len(), encoded_len(w, h, BcFormat::Bc7));
        let decoded = decode_image(&image).expect("decodes");
        let db = psnr_rgb(&rgba, &decoded);
        println!("bc7 albedo {w}x{h}: {db:.2} dB");
        // The old stand-in (BC3 with swapped halves) scores 38.98 dB on this fixture.
        assert!(db >= 40.0, "PSNR {db:.2} dB");
        // Bytes equal twice, and equal at 1 and 8 threads.
        let again = encode_image(&rgba, w, h, BcFormat::Bc7, BcSlot::BaseColor).expect("again");
        assert_eq!(image.bytes, again.bytes);
        assert_eq!(image.bytes, encode_bands(&rgba, w, h, 1));
        assert_eq!(image.bytes, encode_bands(&rgba, w, h, 8));
    }
}
