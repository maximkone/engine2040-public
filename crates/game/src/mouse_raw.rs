//! Raw mouse deltas for the free camera: winit's unaccelerated
//! `DeviceEvent::MouseMotion` deltas sum between frames and one `take` per
//! presented frame turns the look. Presentation only: the sim never reads it
//! and no hash sees it.

use engine_core::checked::f32_from_f64;

/// The most pixels one axis may turn in a single frame; a pointer warp, a
/// driver spike or a debugger pause cannot snap the view past this.
pub const MAX_MOUSE_PX: f32 = 2_000.0;

/// Per-frame sum of raw mouse deltas while the cursor is grabbed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MouseRaw {
    grabbed: bool,
    dx_px: f32,
    dy_px: f32,
}

impl MouseRaw {
    /// A fresh accumulator; nothing is grabbed until the window says so.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether pushes are currently collected.
    pub fn is_grabbed(&self) -> bool {
        self.grabbed
    }

    /// Grab or release. A release also drops any pending sum, so a delta
    /// collected before the release cannot turn the camera after it.
    pub fn set_grabbed(&mut self, grabbed: bool) {
        self.grabbed = grabbed;
        if !grabbed {
            self.dx_px = 0.0;
            self.dy_px = 0.0;
        }
    }

    /// Add one winit `MouseMotion` delta. A push while not grabbed and a
    /// non-finite delta change nothing; the sum saturates at
    /// [`MAX_MOUSE_PX`] per axis.
    pub fn push(&mut self, dx_px: f64, dy_px: f64) {
        if !self.grabbed {
            return;
        }
        let (dx, dy) = (f32_from_f64(dx_px), f32_from_f64(dy_px));
        if !dx.is_finite() || !dy.is_finite() {
            return;
        }
        self.dx_px = (self.dx_px + dx).clamp(-MAX_MOUSE_PX, MAX_MOUSE_PX);
        self.dy_px = (self.dy_px + dy).clamp(-MAX_MOUSE_PX, MAX_MOUSE_PX);
    }

    /// The frame's sum, clamped per axis, and a zeroed accumulator.
    pub fn take(&mut self) -> (f32, f32) {
        let out = (
            self.dx_px.clamp(-MAX_MOUSE_PX, MAX_MOUSE_PX),
            self.dy_px.clamp(-MAX_MOUSE_PX, MAX_MOUSE_PX),
        );
        self.dx_px = 0.0;
        self.dy_px = 0.0;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_pushes_take_their_sum_once() {
        let mut mouse = MouseRaw::new();
        mouse.set_grabbed(true);
        mouse.push(3.0, -1.0);
        mouse.push(3.0, -1.0);
        assert_eq!(mouse.take(), (6.0, -2.0));
        assert_eq!(mouse.take(), (0.0, 0.0));
    }

    #[test]
    fn a_push_while_not_grabbed_changes_nothing() {
        let mut mouse = MouseRaw::new();
        mouse.push(3.0, -1.0);
        assert_eq!(mouse.take(), (0.0, 0.0));
        mouse.set_grabbed(true);
        mouse.set_grabbed(false);
        mouse.push(5.0, 5.0);
        assert_eq!(mouse.take(), (0.0, 0.0));
        assert!(!mouse.is_grabbed());
    }

    #[test]
    fn non_finite_deltas_are_dropped() {
        let mut mouse = MouseRaw::new();
        mouse.set_grabbed(true);
        mouse.push(f64::NAN, 1.0);
        mouse.push(1.0, f64::INFINITY);
        assert_eq!(mouse.take(), (0.0, 0.0));
        mouse.push(2.0, 3.0);
        assert_eq!(mouse.take(), (2.0, 3.0));
    }

    #[test]
    fn a_spike_takes_the_cap_on_its_axis_only() {
        let mut mouse = MouseRaw::new();
        mouse.set_grabbed(true);
        mouse.push(9_000.0, -4.0);
        assert_eq!(mouse.take(), (MAX_MOUSE_PX, -4.0));
        mouse.push(1_500.0, 0.0);
        mouse.push(1_500.0, 0.0);
        assert_eq!(mouse.take(), (MAX_MOUSE_PX, 0.0));
    }

    #[test]
    fn a_release_drops_the_pending_sum() {
        let mut mouse = MouseRaw::new();
        mouse.set_grabbed(true);
        mouse.push(4.0, 4.0);
        mouse.set_grabbed(false);
        assert_eq!(mouse.take(), (0.0, 0.0));
    }
}
