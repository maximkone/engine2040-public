//! Wound paint intake and ordering for the per-character budget store.
//!
//! Validates normalized-UV rects, gates backface paints, and orders paints
//! by `(tick, char_id)`. Owner: `wound_map`, the only consumer. Units: UV
//! 0..1 floats, texels for [`DILATE_PX`]. Determinism: pure functions of
//! the arguments; the order key is integers only. Invalid rects are
//! dropped (`None`/`false`), never a panic.
// PORT(naelstrof/SkinnedMeshDecals): per-fighter decal maps with a capped
// per-frame drain, adapted from the donor's skinned-mesh decal projection
// into a CPU budget queue; GPU projection lands with the atlas bind.

use std::cmp::Ordering;

/// Seam fill ring width in texels around each drained rect, applied at bind.
///
/// Recorded here so the bind step dilates the same rects it drains; this
/// module only queues and orders rects.
pub const DILATE_PX: u32 = 1;

/// Wound rect in normalized atlas UV: `[u0, u1] x [v0, v1]`, each 0..1.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct UvRect {
    /// Left edge in UV.
    pub u0: f32,
    /// Top edge in UV.
    pub v0: f32,
    /// Right edge in UV.
    pub u1: f32,
    /// Bottom edge in UV.
    pub v1: f32,
}

/// Wound paint kind: cut, bruise, or burn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WoundKind {
    /// Open cut.
    Cut,
    /// Closed bruise.
    Bruise,
    /// Burn mark.
    Burn,
}

/// One bounding paint for one hit on one body.
///
/// `char_id` is the owning body: a paint never leaks onto another body
/// because eviction and drain both keep this tag.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Paint {
    /// Tick of the hit at 60 Hz; drain orders by this first.
    pub tick: u32,
    /// Owning character id; drain orders by this second.
    pub char_id: u32,
    /// Clamped wound rect in atlas UV.
    pub rect: UvRect,
    /// Wound paint kind.
    pub kind: WoundKind,
}

/// Clamp a rect into 0..1 UV, ordering corners; `None` when nothing remains.
///
/// Non-finite inputs and empty results are dropped, mirroring the atlas
/// reject for air or buried prints: a paint outside the map paints nothing.
pub fn clamp_rect(rect: UvRect) -> Option<UvRect> {
    if !(rect.u0.is_finite() && rect.v0.is_finite() && rect.u1.is_finite() && rect.v1.is_finite()) {
        return None;
    }
    let clamped = UvRect {
        u0: rect.u0.min(rect.u1).clamp(0.0, 1.0),
        v0: rect.v0.min(rect.v1).clamp(0.0, 1.0),
        u1: rect.u0.max(rect.u1).clamp(0.0, 1.0),
        v1: rect.v0.max(rect.v1).clamp(0.0, 1.0),
    };
    if clamped.u0 >= clamped.u1 || clamped.v0 >= clamped.v1 {
        return None;
    }
    Some(clamped)
}

/// True when a paint faces the camera: cosine of surface normal and view
/// direction above zero. Non-finite input never faces.
pub fn is_front_facing(facing: f32) -> bool {
    facing.is_finite() && facing > 0.0
}

/// Drain order for two paints: `tick` first, then `char_id`.
pub fn paint_order(a: &Paint, b: &Paint) -> Ordering {
    (a.tick, a.char_id).cmp(&(b.tick, b.char_id))
}

/// Expand a rect by [`DILATE_PX`] texels on a square `map_px` map, clamped.
///
/// Gives the seam-fill ring so neighboring UV islands share texels. A zero
/// map size returns the rect unchanged (no bind target yet).
pub fn dilate_uv(rect: UvRect, map_px: u32) -> UvRect {
    if map_px == 0 {
        return rect;
    }
    let step = DILATE_PX as f32 / map_px as f32;
    UvRect {
        u0: (rect.u0 - step).clamp(0.0, 1.0),
        v0: (rect.v0 - step).clamp(0.0, 1.0),
        u1: (rect.u1 + step).clamp(0.0, 1.0),
        v1: (rect.v1 + step).clamp(0.0, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_orders_corners_and_clamps_edges() {
        let rect = UvRect {
            u0: 0.4,
            v0: 1.7,
            u1: -0.2,
            v1: 0.9,
        };
        assert_eq!(
            clamp_rect(rect),
            Some(UvRect {
                u0: 0.0,
                v0: 0.9,
                u1: 0.4,
                v1: 1.0,
            })
        );
    }

    #[test]
    fn clamp_drops_fully_outside_and_nan() {
        let outside = UvRect {
            u0: 1.5,
            v0: 0.2,
            u1: 2.0,
            v1: 0.4,
        };
        assert_eq!(clamp_rect(outside), None);
        let nan = UvRect {
            u0: f32::NAN,
            v0: 0.0,
            u1: 0.5,
            v1: 0.5,
        };
        assert_eq!(clamp_rect(nan), None);
    }

    #[test]
    fn facing_gate_drops_backface_and_nonfinite() {
        assert!(is_front_facing(0.5));
        assert!(!is_front_facing(0.0));
        assert!(!is_front_facing(-1.0));
        assert!(!is_front_facing(f32::NAN));
    }

    #[test]
    fn dilate_expands_one_texel_and_clamps_at_border() {
        let rect = UvRect {
            u0: 0.5,
            v0: 0.5,
            u1: 0.75,
            v1: 0.75,
        };
        let step: f32 = 1.0 / 256.0;
        assert_eq!(
            dilate_uv(rect, 256),
            UvRect {
                u0: 0.5 - step,
                v0: 0.5 - step,
                u1: 0.75 + step,
                v1: 0.75 + step,
            }
        );
        let edge = UvRect {
            u0: 0.0,
            v0: 0.0,
            u1: 0.1,
            v1: 0.1,
        };
        let grown = dilate_uv(edge, 256);
        assert_eq!(grown.u0, 0.0);
        assert_eq!(grown.v0, 0.0);
        assert_eq!(dilate_uv(rect, 0), rect);
    }

    #[test]
    fn order_keys_tick_then_char() {
        let paint = |tick: u32, char_id: u32| Paint {
            tick,
            char_id,
            rect: UvRect {
                u0: 0.0,
                v0: 0.0,
                u1: 0.5,
                v1: 0.5,
            },
            kind: WoundKind::Cut,
        };
        assert_eq!(paint_order(&paint(1, 9), &paint(1, 3)), Ordering::Greater);
        assert_eq!(paint_order(&paint(0, 9), &paint(1, 3)), Ordering::Less);
        assert_eq!(paint_order(&paint(2, 2), &paint(2, 2)), Ordering::Equal);
    }
}
