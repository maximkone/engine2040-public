//! vat: vertex animation texture bake for crowd clips.
//!
//! Consumer: a later shader row samples [`Vat`] bytes as a texture with
//! per-seat offsets; this slice only bakes and decodes on the CPU. Data
//! owner: `engine-assets`. Units: meters in, integer centimeters in the
//! header, unorm16 in the payload. Determinism scope: the same frames bake
//! byte-identical output (index-order loops, one integer box, no clock, no
//! hash-map iteration). Errors: none; the bake is infallible and out-of-range
//! sampler reads fall back to the box centre.
//!
//! A mesh is positions plus index triplets into shared verts, so the bake
//! walks position frames only. One shared integer-centimeter box covers every
//! frame; each position maps to unorm16 RGB with a reserved zero alpha lane
//! (8 bytes per vertex per frame). Axes with zero extent encode the box
//! centre and decode back to the shared bound.

/// Header length in bytes: two `u32` counts plus six `i32` box corners.
const HEADER_LEN: usize = 32;
/// Payload bytes of one vertex in one frame: four `u16` lanes (RGBA).
const SAMPLE_LEN: usize = 8;
/// Unorm16 code for the box centre, used on zero-extent axes.
const CENTER_U16: u16 = 0x8000;
/// Reserved alpha lane: always zero, the sampler reads RGB only.
const ALPHA_U16: u16 = 0;
/// Centimeters per meter; the header box is integer centimeters.
const CM_PER_M: f32 = 100.0;
/// Payload scale: unorm16 spans `0.0..=1.0` over `0..=65535`.
const UNORM_LEVELS: f32 = 65535.0;
/// Two to the thirty-second as an exact `f32`: the first power of two above
/// `u32::MAX`, so every integral input at or above it saturates. Built from
/// exact small literals because the decimal `4_294_967_295.0` is not exactly
/// representable as an `f32`.
const TWO_POW_32_F32: f32 = 65_536.0 * 65_536.0;
/// Two to the thirty-first as an exact `f32`: the `i32` saturation edge, built
/// from exact small literals for the same reason.
const TWO_POW_31_F32: f32 = 65_536.0 * 32_768.0;

/// Integer-centimeter bounds plus counts of one baked clip. The box is the
/// floor/ceil of the clip bounds in centimeters, so every finite sample sits
/// inside it; no float and no clock ever enters the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VatHeader {
    /// Vertex count per frame (first frame's length; saturates past `u32::MAX`).
    pub verts_u32: u32,
    /// Frame count (saturates past `u32::MAX`).
    pub frames_u32: u32,
    /// Shared box minimum corner in integer centimeters.
    pub box_min_cm: [i32; 3],
    /// Shared box maximum corner in integer centimeters.
    pub box_max_cm: [i32; 3],
}

/// One baked clip: header plus payload at 8 bytes per vertex per frame,
/// frame-major. A 5,000-vert 30-frame crowd clip is 1,200,000 payload bytes
/// plus the 32-byte header. The same frames bake the same bytes twice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vat {
    /// Counts plus the shared integer-centimeter box.
    pub header: VatHeader,
    /// Payload: frame-major unorm16 RGBA in little-endian order, alpha zero.
    pub bytes: Vec<u8>,
}

/// Total serialized length in bytes of a `verts` by `frames` bake: payload
/// plus the 32-byte header. Saturates instead of overflowing on absurd sizes.
pub fn byte_len(verts: usize, frames: usize) -> usize {
    verts
        .saturating_mul(frames)
        .saturating_mul(SAMPLE_LEN)
        .saturating_add(HEADER_LEN)
}

/// Bake position frames into a vertex animation texture: one shared
/// integer-centimeter box over all frames, each position scaled into unorm16
/// RGB with a zero alpha lane. Ragged frames read the first frame's vertex
/// count; short frames fill the box centre and long frames truncate.
/// Non-finite components never enter the box and encode as the box centre.
pub fn bake_vat(frames: &[Vec<[f32; 3]>]) -> Vat {
    let vert_count = frames.first().map_or(0, Vec::len);
    let (box_min_cm, box_max_cm) = shared_box_cm(frames);
    let header = VatHeader {
        verts_u32: u32_sat(vert_count),
        frames_u32: u32_sat(frames.len()),
        box_min_cm,
        box_max_cm,
    };
    let mut bytes = Vec::with_capacity(
        vert_count
            .saturating_mul(frames.len())
            .saturating_mul(SAMPLE_LEN),
    );
    for frame in frames {
        for vert in 0..vert_count {
            let sample = frame.get(vert).copied().unwrap_or([f32::NAN; 3]);
            for (axis, position) in sample.iter().enumerate() {
                let (Some(min_cm), Some(max_cm)) = (box_min_cm.get(axis), box_max_cm.get(axis))
                else {
                    continue;
                };
                let code = encode_axis(*position, *min_cm, *max_cm);
                bytes.extend_from_slice(&code.to_le_bytes());
            }
            bytes.extend_from_slice(&ALPHA_U16.to_le_bytes());
        }
    }
    Vat { header, bytes }
}

/// Decode one baked sample back to meters, reversing the box scale of
/// [`bake_vat`]. Out-of-range frame or vertex reads return the exact box
/// centre, never a panic.
pub fn decode_sample(vat: &Vat, frame: usize, vert: usize) -> [f32; 3] {
    let verts = usize::try_from(vat.header.verts_u32).unwrap_or(usize::MAX);
    let code_of = |axis: usize| -> Option<u16> {
        let slot = frame
            .checked_mul(verts)?
            .checked_add(vert)?
            .checked_mul(SAMPLE_LEN)?
            .checked_add(axis.checked_mul(2)?)?;
        let pair = vat.bytes.get(slot..slot.checked_add(2)?)?;
        let (Some(lo), Some(hi)) = (pair.first(), pair.get(1)) else {
            return None;
        };
        Some(u16::from_le_bytes([*lo, *hi]))
    };
    let mut out = [0.0; 3];
    for (axis, slot) in out.iter_mut().enumerate() {
        let (Some(min_cm), Some(max_cm)) = (
            vat.header.box_min_cm.get(axis),
            vat.header.box_max_cm.get(axis),
        ) else {
            continue;
        };
        *slot = match code_of(axis) {
            Some(code) => decode_axis(code, *min_cm, *max_cm),
            None => axis_centre_m(*min_cm, *max_cm),
        };
    }
    out
}

/// `usize` count as `u32`; saturates past `u32::MAX` (live clips never reach
/// it). Named conversion instead of an `as` cast.
fn u32_sat(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// Shared integer-centimeter box over every finite component of every frame.
/// Axes with no finite sample stay degenerate at zero; floor/ceil keep every
/// finite sample inside the box.
fn shared_box_cm(frames: &[Vec<[f32; 3]>]) -> ([i32; 3], [i32; 3]) {
    let mut min_m = [f32::INFINITY; 3];
    let mut max_m = [f32::NEG_INFINITY; 3];
    for frame in frames {
        for sample in frame {
            for (axis, value) in sample.iter().enumerate() {
                if !value.is_finite() {
                    continue;
                }
                if let Some(lo) = min_m.get_mut(axis) {
                    if *value < *lo {
                        *lo = *value;
                    }
                }
                if let Some(hi) = max_m.get_mut(axis) {
                    if *value > *hi {
                        *hi = *value;
                    }
                }
            }
        }
    }
    let mut box_min_cm = [0; 3];
    let mut box_max_cm = [0; 3];
    for axis in 0..3 {
        let (Some(lo), Some(hi)) = (min_m.get(axis), max_m.get(axis)) else {
            continue;
        };
        if lo.is_finite() && hi.is_finite() {
            if let (Some(dst_min), Some(dst_max)) =
                (box_min_cm.get_mut(axis), box_max_cm.get_mut(axis))
            {
                *dst_min = f32_to_i32_sat((*lo * CM_PER_M).floor());
                *dst_max = f32_to_i32_sat((*hi * CM_PER_M).ceil());
            }
        }
    }
    (box_min_cm, box_max_cm)
}

/// Encode one axis position into unorm16 against the integer-centimeter box.
/// Zero-extent axes and non-finite inputs encode the box centre.
fn encode_axis(position_m: f32, min_cm: i32, max_cm: i32) -> u16 {
    let extent_cm = max_cm.saturating_sub(min_cm);
    if extent_cm <= 0 || !position_m.is_finite() {
        return CENTER_U16;
    }
    let min_m = i32_to_f32(min_cm);
    let extent_m = i32_to_f32(extent_cm);
    unit_to_u16((position_m - min_m) / extent_m)
}

/// Reverse [`encode_axis`]: unorm16 back to meters through the same box.
/// Zero-extent axes return the shared bound.
fn decode_axis(code: u16, min_cm: i32, max_cm: i32) -> f32 {
    let min_m = i32_to_f32(min_cm);
    let extent_m = i32_to_f32(max_cm.saturating_sub(min_cm));
    if extent_m <= 0.0 {
        return min_m;
    }
    min_m + f32::from(code) / UNORM_LEVELS * extent_m
}

/// Box-centre position of one axis in meters: the out-of-range fallback of
/// [`decode_sample`].
fn axis_centre_m(min_cm: i32, max_cm: i32) -> f32 {
    (i32_to_f32(min_cm) + i32_to_f32(max_cm)) * 0.5
}

/// Map a unit `0.0..=1.0` value onto unorm16, rounding half away from zero
/// and clamping outside values into range.
fn unit_to_u16(unit: f32) -> u16 {
    if unit.is_nan() || unit <= 0.0 {
        return 0;
    }
    if unit >= 1.0 {
        return u16::MAX;
    }
    let integral = (unit * UNORM_LEVELS).round();
    if integral >= 65535.0 {
        return u16::MAX;
    }
    let mag = f32_integral_to_u32_sat(integral);
    u16::try_from(mag).unwrap_or(u16::MAX)
}

/// Convert an integral `f32` in `0.0..=4_294_967_295.0` to `u32`, saturating
/// at or above 2^32 and mapping NaN to zero. The sign, exponent and mantissa below
/// rebuild the integer bit-exactly: a named conversion, never an `as` cast.
fn f32_integral_to_u32_sat(integral: f32) -> u32 {
    if integral.is_nan() || integral <= 0.0 {
        return 0;
    }
    if integral >= TWO_POW_32_F32 {
        return u32::MAX;
    }
    let bits: u32 = integral.to_bits();
    let exp: u32 = (bits >> 23) & 0xFF;
    if exp < 127 {
        return 0;
    }
    let shift: u32 = exp.wrapping_sub(127);
    let mut mag: u32 = (bits & 0x7F_FF_FF) | 0x80_00_00;
    if shift >= 23 {
        mag = mag.wrapping_shl(shift.wrapping_sub(23));
    } else {
        mag >>= 23_u32.wrapping_sub(shift);
    }
    mag
}

/// Truncate an integral `f32` toward zero, saturating to the `i32` range;
/// NaN maps to zero. Exact on every finite input.
fn f32_to_i32_sat(integral: f32) -> i32 {
    if integral.is_nan() {
        return 0;
    }
    if integral >= TWO_POW_31_F32 {
        return i32::MAX;
    }
    if integral <= -TWO_POW_31_F32 {
        return i32::MIN;
    }
    if integral < 0.0 {
        let mag = f32_integral_to_u32_sat(-integral);
        i32::from_ne_bytes(mag.to_ne_bytes()).wrapping_neg()
    } else {
        let mag = f32_integral_to_u32_sat(integral);
        i32::from_ne_bytes(mag.to_ne_bytes())
    }
}

/// Round an `i32` to the nearest `f32`, ties to even (the same rounding a
/// hardware conversion uses). Exact below 2^24. Named conversion instead of
/// an `as` cast: the magnitude below is normalized bit by bit.
fn i32_to_f32(value: i32) -> f32 {
    if value == 0 {
        return 0.0;
    }
    let neg = value < 0;
    let mag: u32 = u32::from_ne_bytes(value.wrapping_abs().to_ne_bytes());
    let zeros: u32 = mag.leading_zeros();
    let norm: u32 = mag.wrapping_shl(zeros);
    let biased: u32 = 31_u32.wrapping_sub(zeros).wrapping_add(127);
    let sign: u32 = u32::from(neg);
    let mut mant: u32 = (norm >> 8) & 0x7F_FF_FF;
    let round_bit: u32 = (norm >> 7) & 1;
    let sticky: u32 = norm & 0x7F;
    if round_bit == 1 && (sticky != 0 || mant & 1 == 1) {
        mant = mant.wrapping_add(1);
        if mant > 0x7F_FF_FF {
            return f32::from_bits((sign << 31) | (biased.wrapping_add(1) << 23));
        }
    }
    f32::from_bits((sign << 31) | (biased << 23) | mant)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two frames of a small quad: frame one lifts the second frame by a
    /// quarter meter so every axis carries a live extent.
    fn quad_frames() -> Vec<Vec<[f32; 3]>> {
        vec![
            vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
            vec![
                [0.0, 0.0, 0.25],
                [1.0, 0.0, 0.25],
                [1.0, 1.0, 0.25],
                [0.0, 1.0, 0.25],
            ],
        ]
    }

    #[test]
    fn vat_decode_within_1e_minus_3() {
        let frames = quad_frames();
        let vat = bake_vat(&frames);
        for (frame, positions) in frames.iter().enumerate() {
            for (vert, source) in positions.iter().enumerate() {
                let decoded = decode_sample(&vat, frame, vert);
                for axis in 0..3 {
                    let drift = (decoded[axis] - source[axis]).abs();
                    assert!(
                        drift <= 1e-3,
                        "frame {frame} vert {vert} axis {axis} drifted {drift}"
                    );
                }
            }
        }
    }

    #[test]
    fn vat_bytes_are_shape_plus_header() {
        let vat = bake_vat(&quad_frames());
        let header_len = byte_len(0, 0);
        assert_eq!(vat.header.verts_u32, 4);
        assert_eq!(vat.header.frames_u32, 2);
        assert_eq!(vat.bytes.len() + header_len, byte_len(4, 2));
        assert_eq!(vat.bytes.len(), 4 * 2 * 8);
        assert_eq!(byte_len(5_000, 30), 1_200_000 + header_len);
    }

    #[test]
    fn vat_stable_x2() {
        let frames = quad_frames();
        let first = bake_vat(&frames);
        let second = bake_vat(&frames);
        assert_eq!(first.header, second.header);
        assert_eq!(first.bytes, second.bytes);
    }

    #[test]
    fn vat_out_of_range_reads_box_centre() {
        let vat = bake_vat(&quad_frames());
        let centre = [
            (i32_to_f32(vat.header.box_min_cm[0]) + i32_to_f32(vat.header.box_max_cm[0])) * 0.5,
            (i32_to_f32(vat.header.box_min_cm[1]) + i32_to_f32(vat.header.box_max_cm[1])) * 0.5,
            (i32_to_f32(vat.header.box_min_cm[2]) + i32_to_f32(vat.header.box_max_cm[2])) * 0.5,
        ];
        for sample in [decode_sample(&vat, 9, 0), decode_sample(&vat, 0, 9)] {
            assert_eq!(sample, centre);
        }
        let empty = bake_vat(&[]);
        assert_eq!(empty.header.verts_u32, 0);
        assert_eq!(empty.header.frames_u32, 0);
        assert!(empty.bytes.is_empty());
        assert_eq!(decode_sample(&empty, 0, 0), [0.0, 0.0, 0.0]);
    }
}
