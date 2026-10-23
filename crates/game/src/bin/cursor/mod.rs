//! cursor: the free camera's raw mouse source and pointer grab.
//!
//! The window hands winit's unaccelerated `DeviceEvent::MouseMotion` deltas to
//! [`MouseRaw`], which sums them until the presented frame takes them.
//! Presentation only: no sim input, no hash.

use engine_game::camera::DirectorParams;
use engine_game::fps_view::LookState;
use engine_game::mouse_raw::MouseRaw;
use winit::event::DeviceEvent;
use winit::window::{CursorGrabMode, Window};

/// Grab the pointer for free-fly (locked, else confined) and release it for the
/// director. View only: a platform that refuses the grab leaves the arrow keys
/// in charge.
pub fn set_grabbed(raw: &mut MouseRaw, window: Option<&Window>, grabbed: bool) {
    raw.set_grabbed(grabbed);
    let Some(window) = window else {
        return;
    };
    let result = if grabbed {
        window
            .set_cursor_grab(CursorGrabMode::Locked)
            .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined))
    } else {
        window.set_cursor_grab(CursorGrabMode::None)
    };
    // A refused grab is not fatal: the free camera still flies on keys.
    let _ = result;
    window.set_cursor_visible(!grabbed);
}

/// Sum one device event into the frame's mouse delta; non-motion events and a
/// released grab are ignored by [`MouseRaw`].
pub fn push_event(raw: &mut MouseRaw, event: &DeviceEvent) {
    if let DeviceEvent::MouseMotion { delta } = event {
        raw.push(delta.0, delta.1);
    }
}

/// Apply the frame's summed pixels to the free camera's look state.
pub fn take_look(raw: &mut MouseRaw, cam: &mut DirectorParams) {
    let (dx, dy) = raw.take();
    if dx == 0.0 && dy == 0.0 {
        return;
    }
    let mut look = LookState {
        yaw_rad: cam.free_yaw_rad,
        pitch_rad: cam.free_pitch_rad,
    };
    look.apply_mouse(dx, dy);
    cam.free_yaw_rad = look.yaw_rad;
    cam.free_pitch_rad = look.pitch_rad;
}
