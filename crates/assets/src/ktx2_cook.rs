//! ktx2_cook: arena material maps cooked to block-compressed KTX2 with a full mip
//! chain, plus the reader that turns a cooked file back into RGBA8.
//!
//! Owner: `engine-assets`; consumers: the `arena_cook` bin (offline, `import`
//! feature) and `engine-game` `arena_kit` (loads cooked files, decodes level 0
//! into the render upload). Units: RGBA8 texels in, KTX2 bytes out. Std-only:
//! the block codecs are `bc` (BC7, BC5, BC1) and the fitted BC1 encoder below,
//! so there is no encoder dependency edge and the licence gate sees nothing new.
//! Determinism: index-order loops, no clock, no hash order; the same texels,
//! kind, shape and size cap give identical bytes.
//!
//! Shapes: the caller names the map kind and the block shape apart, so a lane
//! can pin the cheaper shape. The arena cook writes `BaseColor` as BC7 sRGB
//! (vkFormat 146) and `Orm` as BC7 linear (145, R occlusion, G roughness,
//! B metallic); `Normal` is BC5 linear (141, XY only, Z rebuilt at decode). A
//! lane on a wire budget names BC1 (132 sRGB, 131 linear) instead: the fighter
//! atlas does, inside the page's first bundle. The reader takes the shape from
//! the header, so a BC1 file still loads. The mip chain runs to 1x1; levels
//! below 4x4 pad to one block by edge clamp. Levels are stored smallest first,
//! aligned to 16 bytes, as the KTX2 spec asks; the level index lists the base
//! level first.

use crate::bc::{self, BcError, BcFormat, BcSlot, BLOCK_EDGE, BLOCK_TEXELS};
use crate::import::KTX2_MAGIC;
use std::fmt;

/// `VK_FORMAT_BC1_RGB_UNORM_BLOCK`.
pub const VK_BC1_RGB_UNORM: u32 = 131;
/// `VK_FORMAT_BC1_RGB_SRGB_BLOCK`.
pub const VK_BC1_RGB_SRGB: u32 = 132;
/// `VK_FORMAT_BC5_UNORM_BLOCK`.
pub const VK_BC5_UNORM: u32 = 141;
/// `VK_FORMAT_BC7_UNORM_BLOCK` (linear data such as ORM).
pub const VK_BC7_UNORM: u32 = 145;
/// `VK_FORMAT_BC7_SRGB_BLOCK` (colour).
pub const VK_BC7_SRGB: u32 = 146;

/// Byte alignment of every mip level and of the data region.
const LEVEL_ALIGN: usize = 16;
/// Fixed KTX2 header length in bytes.
const HEADER_LEN: usize = 80;
/// Level index entry length in bytes (offset, length, uncompressed length).
const LEVEL_ENTRY_LEN: usize = 24;

/// What a map holds: the transfer rule and the mip filter. The block shape is
/// named beside it at the cook, so one lane can pin BC1 and another take BC7
/// for the same kind.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MapKind {
    /// sRGB colour, mips averaged in linear light.
    BaseColor,
    /// Linear tangent-space normal (GL, +Y up), mips averaged then Z rebuilt.
    Normal,
    /// Linear packed occlusion/roughness/metallic, mips averaged per channel.
    Orm,
}

impl MapKind {
    /// Block shape the shared lane writes: BC7 for colour and ORM (one byte per
    /// texel, seven-bit endpoints) and BC5 for normals (XY, Z rebuilt). A lane
    /// on a wire budget pins BC1 through [`cook_ktx2_as`] instead.
    pub fn format(self) -> BcFormat {
        match self {
            Self::BaseColor | Self::Orm => BcFormat::Bc7,
            Self::Normal => BcFormat::Bc5,
        }
    }

    fn slot(self) -> BcSlot {
        match self {
            Self::BaseColor => BcSlot::BaseColor,
            Self::Normal => BcSlot::Normal,
            Self::Orm => BcSlot::MetallicRoughness,
        }
    }
}

/// Vulkan format number for a map kind and block shape; `None` when the pair is
/// not a legal map (a normal map in BC7, say).
fn vk_format(kind: MapKind, format: BcFormat) -> Option<u32> {
    match (kind, format) {
        (MapKind::BaseColor, BcFormat::Bc1) => Some(VK_BC1_RGB_SRGB),
        (MapKind::BaseColor, BcFormat::Bc7) => Some(VK_BC7_SRGB),
        (MapKind::Orm, BcFormat::Bc1) => Some(VK_BC1_RGB_UNORM),
        (MapKind::Orm, BcFormat::Bc7) => Some(VK_BC7_UNORM),
        (MapKind::Normal, BcFormat::Bc5) => Some(VK_BC5_UNORM),
        _ => None,
    }
}

/// Map kind and block shape named by a header format number; `None` for any
/// other format.
fn shape_of(vk: u32) -> Option<(MapKind, BcFormat)> {
    match vk {
        VK_BC1_RGB_SRGB => Some((MapKind::BaseColor, BcFormat::Bc1)),
        VK_BC1_RGB_UNORM => Some((MapKind::Orm, BcFormat::Bc1)),
        VK_BC5_UNORM => Some((MapKind::Normal, BcFormat::Bc5)),
        VK_BC7_SRGB => Some((MapKind::BaseColor, BcFormat::Bc7)),
        VK_BC7_UNORM => Some((MapKind::Orm, BcFormat::Bc7)),
        _ => None,
    }
}

/// Typed cook and read refusals; never a guessed texture.
#[derive(Clone, PartialEq, Debug)]
pub enum Ktx2Error {
    /// Pixel buffer length disagrees with `width * height * 4`.
    Length {
        /// `width * height * 4`.
        expected: usize,
        /// Bytes supplied.
        got: usize,
    },
    /// Width or height is zero or above the cook bound.
    Dimensions {
        /// Width in texels.
        width: u32,
        /// Height in texels.
        height: u32,
    },
    /// The block encoder refused.
    Codec(BcError),
    /// Not a KTX2 file or cut short; the reason names where.
    Malformed(&'static str),
    /// A valid KTX2 this reader does not decode (format or supercompression).
    Unsupported(&'static str),
    /// The map kind and block shape given to the cook do not go together.
    Shape,
}

impl fmt::Display for Ktx2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length { expected, got } => {
                write!(f, "ktx2_cook: expected {expected} RGBA bytes, got {got}")
            }
            Self::Dimensions { width, height } => {
                write!(f, "ktx2_cook: bad dimensions {width}x{height}")
            }
            Self::Codec(err) => write!(f, "ktx2_cook: {err}"),
            Self::Malformed(why) => write!(f, "ktx2_cook: malformed ktx2 ({why})"),
            Self::Unsupported(why) => write!(f, "ktx2_cook: unsupported ktx2 ({why})"),
            Self::Shape => write!(f, "ktx2_cook: map kind and block shape disagree"),
        }
    }
}

impl std::error::Error for Ktx2Error {}

impl From<BcError> for Ktx2Error {
    fn from(err: BcError) -> Self {
        Self::Codec(err)
    }
}

/// A parsed cooked texture: header facts plus the raw block bytes of every level.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Ktx2Texture {
    /// Base level width in texels.
    pub width: u32,
    /// Base level height in texels.
    pub height: u32,
    /// Map kind recovered from the header format.
    pub kind: MapKind,
    /// Block shape recovered from the header format.
    pub format: BcFormat,
    /// Block bytes per level, base level first.
    pub levels: Vec<Vec<u8>>,
}

/// Width and height of mip `level` (never below 1).
pub fn level_dims(width: u32, height: u32, level: usize) -> (u32, u32) {
    let shift = u32::try_from(level).unwrap_or(31).min(31);
    ((width >> shift).max(1), (height >> shift).max(1))
}

/// Mip count down to 1x1.
pub fn level_count(width: u32, height: u32) -> usize {
    let top = width.max(height).max(1);
    (u32::BITS - top.leading_zeros()) as usize
}

fn level_bytes(format: BcFormat, width: u32, height: u32) -> usize {
    let blocks = width.div_ceil(BLOCK_EDGE) as usize * height.div_ceil(BLOCK_EDGE) as usize;
    blocks * bc::block_bytes(format)
}

fn align_up(value: usize, align: usize) -> usize {
    value.div_ceil(align) * align
}

// ---------------------------------------------------------------------------
// Mip filter.
// ---------------------------------------------------------------------------

fn linear_to_srgb_byte(linear: f32) -> u8 {
    let v = linear.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5) as u8
}

/// Half-size box filter. Colour averages in linear light; data maps average bytes.
fn downsample(rgba: &[u8], width: u32, height: u32, kind: MapKind) -> (u32, u32, Vec<u8>) {
    let (nw, nh) = level_dims(width, height, 1);
    let (w, h) = (width as usize, height as usize);
    let lut: Vec<f32> = (0..=255u8).map(bc::srgb_byte_to_linear).collect();
    let mut out = vec![255u8; nw as usize * nh as usize * 4];
    for y in 0..nh as usize {
        for x in 0..nw as usize {
            let (x0, y0) = ((2 * x).min(w - 1), (2 * y).min(h - 1));
            let (x1, y1) = ((2 * x + 1).min(w - 1), (2 * y + 1).min(h - 1));
            let taps = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)];
            for c in 0..3 {
                let value = |&(tx, ty): &(usize, usize)| rgba[(ty * w + tx) * 4 + c];
                out[(y * nw as usize + x) * 4 + c] = if kind == MapKind::BaseColor {
                    let sum: f32 = taps.iter().map(|t| lut[usize::from(value(t))]).sum();
                    linear_to_srgb_byte(sum / 4.0)
                } else {
                    let sum: u32 = taps.iter().map(|t| u32::from(value(t))).sum();
                    ((sum + 2) / 4) as u8
                };
            }
        }
    }
    (nw, nh, out)
}

// ---------------------------------------------------------------------------
// Fitted BC1 encoder: principal axis, inset, 565 quantise, least-squares refit.
// ---------------------------------------------------------------------------

type Rgb = [f32; 3];

fn nearest_entry(texel: Rgb, palette: &[[u8; 3]; 4], w: Rgb) -> (u32, f32) {
    let mut best = (0u32, f32::MAX);
    for (i, entry) in palette.iter().enumerate() {
        let mut d = 0.0f32;
        for c in 0..3 {
            let e = texel[c] - f32::from(entry[c]);
            d += w[c] * e * e;
        }
        if d < best.1 {
            best = (i as u32, d);
        }
    }
    best
}

/// Pack two float endpoints into an opaque-path block (`c0 > c1`, or equal for a flat block),
/// pick indices against the unpacked palette; returns the block bytes and its error.
fn bc1_finish(texels: &[Rgb; BLOCK_TEXELS], e0: Rgb, e1: Rgb, w: Rgb) -> ([u8; 8], f32) {
    let q = |e: Rgb| {
        let b = |v: f32| v.round().clamp(0.0, 255.0) as u8;
        bc::pack_rgb565(b(e[0]), b(e[1]), b(e[2]))
    };
    let (mut c0, mut c1) = (q(e0), q(e1));
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
    }
    let palette = bc::bc1_palette(bc::unpack_rgb565(c0), bc::unpack_rgb565(c1));
    let mut indices = 0u32;
    let mut error = 0.0f32;
    for (i, texel) in texels.iter().enumerate() {
        let (index, d) = nearest_entry(*texel, &palette, w);
        indices |= index << (2 * i);
        error += d;
    }
    let mut out = [0u8; 8];
    out[0..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    out[4..8].copy_from_slice(&indices.to_le_bytes());
    (out, error)
}

/// Least-squares endpoints for the indices already chosen in `block`.
fn bc1_refit(texels: &[Rgb; BLOCK_TEXELS], block: &[u8; 8]) -> Option<(Rgb, Rgb)> {
    const T: [f32; 4] = [1.0, 0.0, 2.0 / 3.0, 1.0 / 3.0];
    let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    let (mut aa, mut bb, mut ab) = (0.0f32, 0.0f32, 0.0f32);
    let (mut ax, mut bx) = ([0.0f32; 3], [0.0f32; 3]);
    for (i, texel) in texels.iter().enumerate() {
        let a = T[((indices >> (2 * i)) & 3) as usize];
        let b = 1.0 - a;
        aa += a * a;
        bb += b * b;
        ab += a * b;
        for c in 0..3 {
            ax[c] += a * texel[c];
            bx[c] += b * texel[c];
        }
    }
    let det = aa * bb - ab * ab;
    if det.abs() < 1e-4 {
        return None;
    }
    let mut e0 = [0.0f32; 3];
    let mut e1 = [0.0f32; 3];
    for c in 0..3 {
        e0[c] = (bb * ax[c] - ab * bx[c]) / det;
        e1[c] = (aa * bx[c] - ab * ax[c]) / det;
    }
    Some((e0, e1))
}

/// Encode one opaque 4x4 block as BC1. `w` weights the channels in the error.
fn encode_bc1_fitted(block: &[[u8; 4]; BLOCK_TEXELS], w: Rgb) -> [u8; 8] {
    let mut texels = [[0.0f32; 3]; BLOCK_TEXELS];
    let mut mean = [0.0f32; 3];
    for (t, src) in texels.iter_mut().zip(block.iter()) {
        for c in 0..3 {
            t[c] = f32::from(src[c]);
            mean[c] += t[c] / 16.0;
        }
    }
    let mut cov = [[0.0f32; 3]; 3];
    for t in &texels {
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += (t[i] - mean[i]) * (t[j] - mean[j]);
            }
        }
    }
    let mut axis = [0.577_35f32; 3];
    for _ in 0..8 {
        let mut next = [0.0f32; 3];
        for i in 0..3 {
            for j in 0..3 {
                next[i] += cov[i][j] * axis[j];
            }
        }
        let len = (next[0] * next[0] + next[1] * next[1] + next[2] * next[2]).sqrt();
        if len < 1e-6 {
            break;
        }
        axis = [next[0] / len, next[1] / len, next[2] / len];
    }
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for t in &texels {
        let p =
            (t[0] - mean[0]) * axis[0] + (t[1] - mean[1]) * axis[1] + (t[2] - mean[2]) * axis[2];
        lo = lo.min(p);
        hi = hi.max(p);
    }
    // Inset by 1/16 of the span: palette ends sit just inside the extremes.
    let inset = (hi - lo) / 16.0;
    let at = |p: f32| {
        [
            mean[0] + axis[0] * p,
            mean[1] + axis[1] * p,
            mean[2] + axis[2] * p,
        ]
    };
    let (mut best, mut best_err) = bc1_finish(&texels, at(hi - inset), at(lo + inset), w);
    for _ in 0..2 {
        let Some((e0, e1)) = bc1_refit(&texels, &best) else {
            break;
        };
        let (cand, err) = bc1_finish(&texels, e0, e1, w);
        if err < best_err {
            best = cand;
            best_err = err;
        } else {
            break;
        }
    }
    best
}

// ---------------------------------------------------------------------------
// Level encode and decode.
// ---------------------------------------------------------------------------

fn encode_level(
    rgba: &[u8],
    width: u32,
    height: u32,
    kind: MapKind,
    format: BcFormat,
) -> Result<Vec<u8>, Ktx2Error> {
    let (pw, ph) = (
        align_up(width as usize, BLOCK_EDGE as usize),
        align_up(height as usize, BLOCK_EDGE as usize),
    );
    // Edge-clamp pad to whole blocks (only the sub-4 mips actually pad).
    let padded: Vec<u8>;
    let src: &[u8] = if (pw, ph) == (width as usize, height as usize) {
        rgba
    } else {
        let mut buf = vec![0u8; pw * ph * 4];
        for y in 0..ph {
            for x in 0..pw {
                let s =
                    (y.min(height as usize - 1) * width as usize + x.min(width as usize - 1)) * 4;
                buf[(y * pw + x) * 4..][..4].copy_from_slice(&rgba[s..s + 4]);
            }
        }
        padded = buf;
        &padded
    };
    let (pw32, ph32) = (pw as u32, ph as u32);
    if format != BcFormat::Bc1 {
        return Ok(bc::encode_image(src, pw32, ph32, format, kind.slot())?.bytes);
    }
    // BC1 by the fitted encoder: the lanes that pin BC1 live on a wire budget.
    let weights = if kind == MapKind::BaseColor {
        [0.299 * 3.0, 0.587 * 3.0, 0.114 * 3.0]
    } else {
        [1.0, 1.0, 1.0]
    };
    let mut out = Vec::with_capacity(level_bytes(format, width, height));
    for by in 0..ph / 4 {
        for bx in 0..pw / 4 {
            let mut block = [[0u8; 4]; BLOCK_TEXELS];
            for (i, texel) in block.iter_mut().enumerate() {
                let s = ((by * 4 + i / 4) * pw + bx * 4 + i % 4) * 4;
                *texel = [src[s], src[s + 1], src[s + 2], 255];
            }
            out.extend_from_slice(&encode_bc1_fitted(&block, weights));
        }
    }
    Ok(out)
}

/// Decode one cooked level to RGBA8 (`width * height * 4`). Normals rebuild Z from XY.
pub fn decode_level_rgba(
    tex: &Ktx2Texture,
    level: usize,
) -> Result<(u32, u32, Vec<u8>), Ktx2Error> {
    let data = tex
        .levels
        .get(level)
        .ok_or(Ktx2Error::Malformed("level out of range"))?;
    let (w, h) = level_dims(tex.width, tex.height, level);
    let rgba = decode_blocks_rgba(tex.format, w, h, data)?;
    Ok((w, h, rgba))
}

/// Decode one level of packed block bytes (`level_bytes(format, width, height)`
/// long, row-major blocks) to RGBA8 (`width * height * 4`). Normals rebuild Z
/// from XY, as the cook wrote XY only. A caller holding the blocks without the
/// container (the render upload path's RGBA8 fallback) decodes a level here.
pub fn decode_blocks_rgba(
    format: BcFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<u8>, Ktx2Error> {
    if width == 0 || height == 0 {
        return Err(Ktx2Error::Dimensions { width, height });
    }
    if data.len() != level_bytes(format, width, height) {
        return Err(Ktx2Error::Malformed("level length"));
    }
    let blocks_wide = width.div_ceil(BLOCK_EDGE) as usize;
    let step = bc::block_bytes(format);
    let mut rgba = vec![255u8; width as usize * height as usize * 4];
    for (index, chunk) in data.chunks_exact(step).enumerate() {
        let texels = match format {
            BcFormat::Bc1 => {
                let mut b = [0u8; 8];
                b.copy_from_slice(chunk);
                bc::decode_bc1_block(&b)
            }
            BcFormat::Bc7 => {
                let mut b = [0u8; 16];
                b.copy_from_slice(chunk);
                bc::decode_bc7_block(&b).ok_or(Ktx2Error::Malformed("bc7 block is not mode 6"))?
            }
            BcFormat::Bc5 => {
                let mut b = [0u8; 16];
                b.copy_from_slice(chunk);
                let mut t = bc::decode_bc5_block(&b);
                for texel in &mut t {
                    texel[2] = rebuild_z(texel[0], texel[1]);
                }
                t
            }
            BcFormat::Bc3 | BcFormat::Bc4 => {
                return Err(Ktx2Error::Unsupported("block shape"));
            }
        };
        let (bx, by) = ((index % blocks_wide) * 4, (index / blocks_wide) * 4);
        for (i, texel) in texels.iter().enumerate() {
            let (x, y) = (bx + i % 4, by + i / 4);
            if x < width as usize && y < height as usize {
                rgba[(y * width as usize + x) * 4..][..4].copy_from_slice(texel);
            }
        }
    }
    Ok(rgba)
}

/// Z byte of a unit normal from its XY bytes (0..255 maps to -1..1).
fn rebuild_z(x: u8, y: u8) -> u8 {
    let fx = f32::from(x) / 127.5 - 1.0;
    let fy = f32::from(y) / 127.5 - 1.0;
    let z = (1.0 - fx * fx - fy * fy).max(0.0).sqrt();
    ((z * 0.5 + 0.5) * 255.0 + 0.5) as u8
}

// ---------------------------------------------------------------------------
// KTX2 container.
// ---------------------------------------------------------------------------

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Basic data format descriptor (spec section 7) for the kind and shape.
fn dfd_for(kind: MapKind, format: BcFormat) -> Vec<u8> {
    // KHR_DF_MODEL_BC1A (128), _BC5 (132), _BC7 (134); one sample per 8-byte
    // block half, or one 128-bit sample for a BC7 block.
    let (model, samples, bytes_plane, top_bit): (u32, u32, u32, u32) = match format {
        BcFormat::Bc5 => (132, 2, 16, 63),
        BcFormat::Bc7 => (134, 1, 16, 127),
        _ => (128, 1, 8, 63),
    };
    let transfer: u32 = if kind == MapKind::BaseColor { 2 } else { 1 };
    let block_size = 24 + 16 * samples;
    let mut d = Vec::new();
    put_u32(&mut d, 4 + block_size);
    put_u32(&mut d, 0);
    put_u32(&mut d, 2 | (block_size << 16));
    put_u32(&mut d, model | (1 << 8) | (transfer << 16));
    put_u32(&mut d, 3 | (3 << 8));
    put_u32(&mut d, bytes_plane);
    put_u32(&mut d, 0);
    for s in 0..samples {
        // bitOffset 64 * s, bitLength (top_bit + 1), channel id s (red, then green).
        put_u32(&mut d, (64 * s) | (top_bit << 16) | (s << 24));
        put_u32(&mut d, 0);
        put_u32(&mut d, 0);
        put_u32(&mut d, u32::MAX);
    }
    d
}

/// Cook RGBA8 (`width * height * 4`, row-major) in the kind's own shape
/// ([`MapKind::format`]) into a KTX2 file with mips. `max_dim` caps the longest
/// edge by halving first (0 means no cap).
pub fn cook_ktx2(
    rgba: &[u8],
    width: u32,
    height: u32,
    kind: MapKind,
    max_dim: u32,
) -> Result<Vec<u8>, Ktx2Error> {
    cook_ktx2_as(rgba, width, height, kind, kind.format(), max_dim)
}

/// Cook in a pinned block shape: the fighter atlas and other wire-budget lanes
/// pass BC1. A kind and shape that do not go together refuse.
pub fn cook_ktx2_as(
    rgba: &[u8],
    width: u32,
    height: u32,
    kind: MapKind,
    format: BcFormat,
    max_dim: u32,
) -> Result<Vec<u8>, Ktx2Error> {
    let vk = vk_format(kind, format).ok_or(Ktx2Error::Shape)?;
    if width == 0 || height == 0 || width > bc::MAX_BC_DIM || height > bc::MAX_BC_DIM {
        return Err(Ktx2Error::Dimensions { width, height });
    }
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(Ktx2Error::Length {
            expected,
            got: rgba.len(),
        });
    }
    let (mut w, mut h, mut cur) = (width, height, rgba.to_vec());
    while max_dim > 0 && w.max(h) > max_dim && w.max(h) > 1 {
        let (nw, nh, next) = downsample(&cur, w, h, kind);
        (w, h, cur) = (nw, nh, next);
    }
    let count = level_count(w, h);
    let mut levels: Vec<Vec<u8>> = Vec::with_capacity(count);
    let (mut lw, mut lh, mut lrgba) = (w, h, cur);
    for level in 0..count {
        levels.push(encode_level(&lrgba, lw, lh, kind, format)?);
        if level + 1 < count {
            let (nw, nh, next) = downsample(&lrgba, lw, lh, kind);
            (lw, lh, lrgba) = (nw, nh, next);
        }
    }
    let dfd = dfd_for(kind, format);
    let dfd_offset = HEADER_LEN + LEVEL_ENTRY_LEN * count;
    let mut cursor = align_up(dfd_offset + dfd.len(), LEVEL_ALIGN);
    // Smallest level first in the file; the index below lists the base level first.
    let mut entries = vec![(0usize, 0usize); count];
    for level in (0..count).rev() {
        entries[level] = (cursor, levels[level].len());
        cursor = align_up(cursor + levels[level].len(), LEVEL_ALIGN);
    }
    let mut out = Vec::with_capacity(cursor);
    out.extend_from_slice(&KTX2_MAGIC);
    put_u32(&mut out, vk);
    put_u32(&mut out, 1);
    put_u32(&mut out, w);
    put_u32(&mut out, h);
    put_u32(&mut out, 0);
    put_u32(&mut out, 0);
    put_u32(&mut out, 1);
    put_u32(&mut out, count as u32);
    put_u32(&mut out, 0);
    put_u32(&mut out, dfd_offset as u32);
    put_u32(&mut out, dfd.len() as u32);
    out.extend_from_slice(&[0u8; 8 + 16]);
    for (offset, len) in &entries {
        out.extend_from_slice(&(*offset as u64).to_le_bytes());
        out.extend_from_slice(&(*len as u64).to_le_bytes());
        out.extend_from_slice(&(*len as u64).to_le_bytes());
    }
    out.extend_from_slice(&dfd);
    for level in (0..count).rev() {
        out.resize(entries[level].0, 0);
        out.extend_from_slice(&levels[level]);
    }
    out.resize(cursor, 0);
    Ok(out)
}

fn rd32(b: &[u8], o: usize) -> Result<u32, Ktx2Error> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(Ktx2Error::Malformed("header cut short"))
}

fn rd64(b: &[u8], o: usize) -> Result<usize, Ktx2Error> {
    let s = b
        .get(o..o + 8)
        .ok_or(Ktx2Error::Malformed("level index cut short"))?;
    let v = u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]);
    usize::try_from(v).map_err(|_| Ktx2Error::Malformed("level offset"))
}

/// Parse a KTX2 file this module cooked (BC1, BC5 or BC7, no supercompression,
/// 2D, one layer). The block shape comes from the header, not from the kind's
/// current default, so an older BC1 cook still loads.
pub fn parse_ktx2(bytes: &[u8]) -> Result<Ktx2Texture, Ktx2Error> {
    if bytes.len() < HEADER_LEN || bytes[0..12] != KTX2_MAGIC {
        return Err(Ktx2Error::Malformed("magic"));
    }
    let (kind, format) = shape_of(rd32(bytes, 12)?).ok_or(Ktx2Error::Unsupported("format"))?;
    let (width, height) = (rd32(bytes, 20)?, rd32(bytes, 24)?);
    if width == 0 || height == 0 || width > bc::MAX_BC_DIM || height > bc::MAX_BC_DIM {
        return Err(Ktx2Error::Dimensions { width, height });
    }
    if rd32(bytes, 28)? != 0 || rd32(bytes, 32)? > 1 || rd32(bytes, 36)? != 1 {
        return Err(Ktx2Error::Unsupported("not a plain 2D texture"));
    }
    if rd32(bytes, 44)? != 0 {
        return Err(Ktx2Error::Unsupported("supercompression"));
    }
    let count = rd32(bytes, 40)? as usize;
    if count != level_count(width, height) {
        return Err(Ktx2Error::Malformed("level count"));
    }
    let mut levels = Vec::with_capacity(count);
    for level in 0..count {
        let at = HEADER_LEN + level * LEVEL_ENTRY_LEN;
        let (offset, len) = (rd64(bytes, at)?, rd64(bytes, at + 8)?);
        let (lw, lh) = level_dims(width, height, level);
        if len != level_bytes(format, lw, lh) {
            return Err(Ktx2Error::Malformed("level length"));
        }
        let end = offset
            .checked_add(len)
            .ok_or(Ktx2Error::Malformed("level end"))?;
        levels.push(
            bytes
                .get(offset..end)
                .ok_or(Ktx2Error::Malformed("level past end of file"))?
                .to_vec(),
        );
    }
    Ok(Ktx2Texture {
        width,
        height,
        kind,
        format,
        levels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Texels that are exactly representable in a BC1 block: two 565 endpoints
    /// per 4x4 block (half the block each), so a lossless codec returns them.
    fn grid_fixture(w: u32, h: u32) -> Vec<u8> {
        let mut out = vec![255u8; (w * h * 4) as usize];
        for by in 0..h / 4 {
            for bx in 0..w / 4 {
                let seed = bx * 31 + by * 17;
                let a = bc::unpack_rgb565(((seed * 2711 + 5) & 0xFFFF) as u16);
                let b = bc::unpack_rgb565(((seed * 4099 + 9001) & 0xFFFF) as u16);
                for ty in 0..4 {
                    for tx in 0..4 {
                        let c = if (tx + ty) % 2 == 0 { a } else { b };
                        let o = (((by * 4 + ty) * w + bx * 4 + tx) * 4) as usize;
                        out[o..o + 3].copy_from_slice(&c);
                    }
                }
            }
        }
        out
    }

    /// Smooth, noisy stand-in for a photographic texture.
    fn natural_fixture(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        let mut state = 0x1234_5678u32;
        for y in 0..h {
            for x in 0..w {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let n = ((state >> 24) as i32 - 128) / 16;
                let base =
                    110 + ((x as f32 * 0.11).sin() * 40.0 + (y as f32 * 0.07).cos() * 30.0) as i32;
                let c = |k: i32| (base + k + n).clamp(0, 255) as u8;
                out.extend_from_slice(&[c(20), c(0), c(-25), 255]);
            }
        }
        out
    }

    fn max_diff(a: &[u8], b: &[u8]) -> u8 {
        a.chunks_exact(4)
            .zip(b.chunks_exact(4))
            .flat_map(|(p, q)| (0..3).map(move |c| p[c].abs_diff(q[c])))
            .max()
            .unwrap_or(0)
    }

    fn psnr(a: &[u8], b: &[u8]) -> f64 {
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
    fn ktx2_cook_transcode_fixture_within_two_codes() {
        let (w, h) = (64, 64);
        let src = grid_fixture(w, h);
        let shapes = [
            (MapKind::BaseColor, BcFormat::Bc7),
            (MapKind::Orm, BcFormat::Bc7),
            (MapKind::BaseColor, BcFormat::Bc1),
            (MapKind::Orm, BcFormat::Bc1),
        ];
        for (kind, format) in shapes {
            let file = cook_ktx2_as(&src, w, h, kind, format, 0).expect("fixture cooks");
            let tex = parse_ktx2(&file).expect("cooked file parses");
            assert_eq!(
                (tex.width, tex.height, tex.kind, tex.format),
                (w, h, kind, format)
            );
            let (dw, dh, rgba) = decode_level_rgba(&tex, 0).expect("level 0 transcodes");
            assert_eq!((dw, dh), (w, h));
            let diff = max_diff(&src, &rgba);
            assert!(
                diff <= 2,
                "{kind:?} {format:?}: worst channel error {diff} codes"
            );
        }
    }

    #[test]
    fn ktx2_cook_natural_texture_quality_floor() {
        let (w, h) = (128, 128);
        let src = natural_fixture(w, h);
        let file = cook_ktx2(&src, w, h, MapKind::BaseColor, 0).expect("cooks");
        let tex = parse_ktx2(&file).expect("parses");
        let (_, _, rgba) = decode_level_rgba(&tex, 0).expect("decodes");
        let db = psnr(&src, &rgba);
        println!("ktx2 bc7 natural {w}x{h}: {db:.2} dB");
        assert!(db > 38.0, "BC7 on a noisy natural fixture: {db:.1} dB");
    }

    #[test]
    fn ktx2_cook_normals_keep_direction() {
        let (w, h) = (64, 64);
        let mut src = vec![255u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let nx = ((x as f32 * 0.2).sin() * 0.5).clamp(-0.9, 0.9);
                let ny = ((y as f32 * 0.15).cos() * 0.4).clamp(-0.9, 0.9);
                let nz = (1.0f32 - nx * nx - ny * ny).sqrt();
                let o = ((y * w + x) * 4) as usize;
                src[o] = ((nx * 0.5 + 0.5) * 255.0 + 0.5) as u8;
                src[o + 1] = ((ny * 0.5 + 0.5) * 255.0 + 0.5) as u8;
                src[o + 2] = ((nz * 0.5 + 0.5) * 255.0 + 0.5) as u8;
            }
        }
        let file = cook_ktx2(&src, w, h, MapKind::Normal, 0).expect("cooks");
        assert_eq!(rd32(&file, 12).expect("header"), VK_BC5_UNORM);
        let tex = parse_ktx2(&file).expect("parses");
        let (_, _, rgba) = decode_level_rgba(&tex, 0).expect("decodes");
        let mut worst = 1.0f32;
        for (p, q) in src.chunks_exact(4).zip(rgba.chunks_exact(4)) {
            let v = |t: &[u8]| {
                let d = [
                    f32::from(t[0]) / 127.5 - 1.0,
                    f32::from(t[1]) / 127.5 - 1.0,
                    f32::from(t[2]) / 127.5 - 1.0,
                ];
                let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                [d[0] / l, d[1] / l, d[2] / l]
            };
            let (a, b) = (v(p), v(q));
            worst = worst.min(a[0] * b[0] + a[1] * b[1] + a[2] * b[2]);
        }
        assert!(worst > 0.9995, "worst normal agreement {worst}");
    }

    #[test]
    fn ktx2_cook_mip_chain_is_whole_and_in_spec_order() {
        let (w, h) = (64, 32);
        let src = natural_fixture(w, h);
        let file = cook_ktx2(&src, w, h, MapKind::Orm, 0).expect("cooks");
        let tex = parse_ktx2(&file).expect("parses");
        assert_eq!(tex.levels.len(), 7, "64x32 runs 64,32,16,8,4,2,1");
        assert_eq!(rd32(&file, 40).expect("header"), 7);
        let mut last_offset = usize::MAX;
        for level in 0..7 {
            let at = HEADER_LEN + level * LEVEL_ENTRY_LEN;
            let offset = rd64(&file, at).expect("index");
            assert_eq!(offset % LEVEL_ALIGN, 0, "level {level} aligned");
            assert!(
                offset < last_offset,
                "smaller levels sit earlier in the file"
            );
            last_offset = offset;
            let (lw, lh) = level_dims(w, h, level);
            let (dw, dh, px) = decode_level_rgba(&tex, level).expect("every level decodes");
            assert_eq!((dw, dh, px.len()), (lw, lh, (lw * lh * 4) as usize));
        }
        // The 1x1 tail is near the mean colour of the source.
        let (_, _, tail) = decode_level_rgba(&tex, 6).expect("1x1");
        let mean: f32 = src.chunks_exact(4).map(|p| f32::from(p[0])).sum::<f32>() / (w * h) as f32;
        assert!(
            (f32::from(tail[0]) - mean).abs() < 8.0,
            "tail {} mean {mean}",
            tail[0]
        );
    }

    #[test]
    fn ktx2_cook_is_byte_identical_twice_and_caps_size() {
        let src = natural_fixture(64, 64);
        let a = cook_ktx2(&src, 64, 64, MapKind::BaseColor, 0).expect("first");
        let b = cook_ktx2(&src, 64, 64, MapKind::BaseColor, 0).expect("second");
        assert_eq!(a, b);
        let small = cook_ktx2(&src, 64, 64, MapKind::BaseColor, 16).expect("capped");
        let tex = parse_ktx2(&small).expect("parses");
        assert_eq!((tex.width, tex.height), (16, 16));
    }

    #[test]
    fn ktx2_cook_set_is_under_half_of_rgba_with_mips() {
        let (w, h) = (256, 256);
        let rgba_set = 3 * (w * h * 4) as usize;
        let base = cook_ktx2(&natural_fixture(w, h), w, h, MapKind::BaseColor, 0).expect("base");
        let orm = cook_ktx2(&natural_fixture(w, h), w, h, MapKind::Orm, 0).expect("orm");
        let normal = cook_ktx2(&normal_fixture(w, h), w, h, MapKind::Normal, 0).expect("normal");
        let cooked = base.len() + orm.len() + normal.len();
        assert!(
            cooked * 2 <= rgba_set,
            "set {cooked} B (with full mips) vs {rgba_set} B of RGBA8 base levels"
        );
        // One byte per texel at level 0: a quarter of the RGBA8 the GPU holds today.
        let tex = parse_ktx2(&base).expect("parses");
        assert_eq!(tex.levels[0].len() * 4, (w * h * 4) as usize);
    }

    /// A seeded tangent-space normal map: three sine hills with slopes up to about 0.5 each,
    /// plus fine noise of +-0.03 on the slope, so every 4x4 block holds real detail. RGBA8
    /// with Z in blue, the way an artist's file stores it.
    fn normal_fixture(w: u32, h: u32) -> Vec<u8> {
        let mut state = 0x2040_beefu32;
        let mut next = || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) as f32 / 16_777_216.0
        };
        let waves: Vec<[f32; 4]> = (0..3)
            .map(|_| {
                [
                    0.03 + 0.07 * next(),
                    0.03 + 0.07 * next(),
                    std::f32::consts::TAU * next(),
                    2.0 + 3.0 * next(),
                ]
            })
            .collect();
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let (mut sx, mut sy) = (0.0f32, 0.0f32);
                for [fx, fy, phase, amp] in &waves {
                    let c = amp * (fx * x as f32 + fy * y as f32 + phase).cos();
                    sx += c * fx;
                    sy += c * fy;
                }
                sx += (next() - 0.5) * 0.06;
                sy += (next() - 0.5) * 0.06;
                let len = (sx * sx + sy * sy + 1.0).sqrt();
                let byte = |v: f32| ((v / len * 0.5 + 0.5) * 255.0 + 0.5) as u8;
                out.extend_from_slice(&[byte(-sx), byte(-sy), byte(1.0), 255]);
            }
        }
        out
    }

    #[test]
    fn bc5_normal_map_1024_cooks_to_one_mib_plus_mips_within_one_and_a_half_degrees() {
        let (w, h) = (1024, 1024);
        let src = normal_fixture(w, h);
        let file = cook_ktx2(&src, w, h, MapKind::Normal, 0).expect("cooks");
        assert_eq!(
            file,
            cook_ktx2(&src, w, h, MapKind::Normal, 0).expect("recooks"),
            "cook bytes equal x2"
        );
        let tex = parse_ktx2(&file).expect("parses");
        // BC5 is 16 bytes per 4x4 block: 1 MiB at 1024x1024, then the chain down to 1x1.
        assert_eq!(tex.levels.len(), 11);
        assert_eq!(tex.levels[0].len(), 1 << 20);
        let mips: usize = tex.levels[1..].iter().map(Vec::len).sum();
        assert_eq!(mips, 349_552, "512x512 down to one block per tiny level");
        let (_, _, rgba) = decode_level_rgba(&tex, 0).expect("decodes");
        let unit = |t: &[u8]| {
            let d = [0, 1, 2].map(|k| f64::from(t[k]) / 127.5 - 1.0);
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            d.map(|v| v / l)
        };
        let mut sum = 0.0f64;
        for (p, q) in src.chunks_exact(4).zip(rgba.chunks_exact(4)) {
            let (a, b) = (unit(p), unit(q));
            let dot = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]).clamp(-1.0, 1.0);
            sum += dot.acos().to_degrees();
        }
        let mean = sum / f64::from(w * h);
        println!("bc5 normal map {w}x{h}: mean angular error {mean:.3} degrees");
        assert!(mean <= 1.5, "mean angular error {mean} degrees");
    }

    #[test]
    fn ktx2_cook_refuses_loudly() {
        assert!(matches!(
            cook_ktx2(&[0u8; 12], 2, 2, MapKind::Orm, 0),
            Err(Ktx2Error::Length { .. })
        ));
        assert!(matches!(
            cook_ktx2(&[], 0, 4, MapKind::Orm, 0),
            Err(Ktx2Error::Dimensions { .. })
        ));
        // A normal map is two channels: BC7 would have to guess what to drop.
        assert!(matches!(
            cook_ktx2_as(&[0u8; 64], 4, 4, MapKind::Normal, BcFormat::Bc7, 0),
            Err(Ktx2Error::Shape)
        ));
        assert!(matches!(
            parse_ktx2(b"not a ktx2 file at all, but long enough to read a header from"),
            Err(Ktx2Error::Malformed(_))
        ));
        let file = cook_ktx2(&natural_fixture(16, 16), 16, 16, MapKind::Orm, 0).expect("cooks");
        assert!(matches!(
            parse_ktx2(&file[..file.len() - 8]),
            Err(Ktx2Error::Malformed(_))
        ));
        let mut other = file;
        other[12] = 37;
        assert!(matches!(parse_ktx2(&other), Err(Ktx2Error::Unsupported(_))));
    }

    /// The five committed arena sets and the level-0 dimensions the cook gave
    /// them (`content/arena/cooked/cook.json`).
    const ARENA_SETS: [(&str, u32, u32); 5] = [
        ("stone", 1024, 512),
        ("sand", 1024, 1024),
        ("wood", 1024, 512),
        ("iron", 1024, 1024),
        ("floor", 1024, 512),
    ];

    #[test]
    fn ktx2_cook_bc7_albedo_1024_reaches_40_db_and_cooks_equal_twice() {
        let (w, h) = (1024, 1024);
        let src = natural_fixture(w, h);
        let file = cook_ktx2(&src, w, h, MapKind::BaseColor, 0).expect("cooks");
        assert_eq!(
            file,
            cook_ktx2(&src, w, h, MapKind::BaseColor, 0).expect("recooks"),
            "cook bytes equal x2"
        );
        assert_eq!(rd32(&file, 12).expect("header"), VK_BC7_SRGB);
        let tex = parse_ktx2(&file).expect("parses");
        assert_eq!(
            (tex.width, tex.height, tex.kind),
            (w, h, MapKind::BaseColor)
        );
        assert_eq!(tex.format, BcFormat::Bc7);
        assert_eq!(tex.levels.len(), 11);
        // BC7 is one byte per texel: 1 MiB at 1024x1024, then the chain down to 1x1.
        assert_eq!(tex.levels[0].len(), 1 << 20);
        let mips: usize = tex.levels[1..].iter().map(Vec::len).sum();
        assert_eq!(mips, 349_552, "512x512 down to one block per tiny level");
        let (_, _, rgba) = decode_level_rgba(&tex, 0).expect("decodes");
        let db = psnr(&src, &rgba);
        println!(
            "ktx2 bc7 albedo {w}x{h}: {db:.2} dB, {} B with mips vs {} B of rgba8",
            file.len(),
            w as usize * h as usize * 4
        );
        assert!(db >= 40.0, "PSNR {db:.2} dB");
    }

    /// The arena set's cooked bytes against the RGBA8 the GPU holds today: the
    /// load path decodes level 0 into a four-byte-per-texel upload with no
    /// mips, so BC7 colour plus BC5 normal with mips is the win the 6 GB card
    /// wants. Prints both numbers.
    #[test]
    fn ktx2_cook_bc7_arena_set_fits_under_half_of_todays_vram() {
        let mut cooked = 0usize;
        let mut today = 0usize;
        for (name, w, h) in ARENA_SETS {
            let cook_set = |kind| {
                let src = if kind == MapKind::Normal {
                    normal_fixture(w, h)
                } else {
                    natural_fixture(w, h)
                };
                cook_ktx2(&src, w, h, kind, 0).expect("set cooks")
            };
            let base = cook_set(MapKind::BaseColor);
            let orm = cook_set(MapKind::Orm);
            let normal = cook_set(MapKind::Normal);
            let set = base.len() + orm.len() + normal.len();
            let rgba8 = 3 * w as usize * h as usize * 4;
            println!("arena {name} {w}x{h}: {set} B cooked vs {rgba8} B of rgba8 today");
            cooked += set;
            today += rgba8;
        }
        let percent = 100.0 * cooked as f64 / today as f64;
        println!(
            "arena set: {cooked} B cooked (BC7 colour, BC5 normal, full mips) vs {today} B \
             of RGBA8 base levels today: {percent:.1} percent"
        );
        assert!(
            cooked * 2 <= today,
            "the arena set is {cooked} B; half of today's {today} B is {}",
            today / 2
        );
        // The smallest set twice, so the whole path is byte-equal x2.
        let (w, h) = (ARENA_SETS[0].1, ARENA_SETS[0].2);
        let once = cook_ktx2(&natural_fixture(w, h), w, h, MapKind::Orm, 0).expect("orm");
        let twice = cook_ktx2(&natural_fixture(w, h), w, h, MapKind::Orm, 0).expect("again");
        assert_eq!(once, twice, "cook bytes equal x2");
    }
}
