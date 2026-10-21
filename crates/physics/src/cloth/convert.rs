//! convert: every crossing between the number worlds a cloth touches, in one place.
//!
//! The sheet is f32 metres, the hashed tick's swept cast is Q16 fixed point
//! ([`engine_core::fixed::Fixed`]) and the spatial hash indexes with `usize`. Each direction has
//! one named function here. The float-to-fixed path multiplies and rounds in f32 exactly as the
//! old inline cast did, so the fixed value is bit for bit the same.

use engine_core::checked;
use engine_core::fixed::Fixed;

/// f32 metres as Q16 fixed point, the unit the swept cast works in.
///
/// Rounds half away from zero in f32 before narrowing, so the result is bit for bit what an
/// inline multiply-and-round gave. NaN becomes zero and a value past `i64` saturates.
pub(in crate::cloth) fn to_fixed(value: f32) -> Fixed {
    let rounded = (value * 65_536.0).round();
    let magnitude =
        i64::try_from(checked::saturating_f64_to_u64(f64::from(rounded.abs()))).unwrap_or(i64::MAX);
    Fixed::from_bits(if rounded < 0.0 { -magnitude } else { magnitude })
}

/// A point as Q16 fixed point.
pub(in crate::cloth) fn to_fixed_point(p: [f32; 3]) -> [Fixed; 3] {
    [to_fixed(p[0]), to_fixed(p[1]), to_fixed(p[2])]
}

/// Q16 fixed point as f32 metres: a power-of-two scale, exact inside the world limit.
pub(in crate::cloth) fn from_fixed(v: Fixed) -> f32 {
    let metres = checked::f32_from_u64(v.to_bits().unsigned_abs()) / 65_536.0;
    if v.to_bits() < 0 {
        -metres
    } else {
        metres
    }
}

/// A float coordinate as a hash cell index: truncating toward zero, with anything negative or
/// NaN in cell zero, which is what the bare cast did.
pub(in crate::cloth) fn cell_index(v: f32) -> usize {
    usize::try_from(checked::saturating_f64_to_u64(f64::from(v))).unwrap_or(usize::MAX)
}

/// A `u32` hash count or cursor as a `usize` index. Cells and cursors never pass a few thousand,
/// so the fallback only covers a 16-bit target.
pub(in crate::cloth) fn index(v: u32) -> usize {
    usize::try_from(v).unwrap_or(0)
}

/// How many hash cells of `edge` metres an extent of `extent` metres covers, rounding up: the old
/// ceil-then-cast, with a negative or NaN extent in zero cells and a huge one saturating.
pub(in crate::cloth) fn cell_count(extent: f32, edge: f32) -> usize {
    let cells = (extent / edge).ceil();
    usize::try_from(checked::saturating_f64_to_u64(f64::from(cells))).unwrap_or(usize::MAX)
}
