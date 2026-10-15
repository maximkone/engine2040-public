//! input: engine-owned window input codes (game-agnostic).
//!
//! Consumer: the windowed leg and the `arena` binary feed the match
//! through here. Data owner: this module owns the engine [`KeyCode`] and
//! [`MouseButton`] types plus their maps onto [`PhysicalButton`]. Units:
//! button codes (keyboard codes address [`Key`] storage, mouse codes are
//! device button indices). Determinism scope: pure maps, no I/O. Errors:
//! none; every engine code maps, so unknown physical keys are rejected at
//! the winit edge (the binary's `from_winit_*` shims), never here.
//!
//! Budget: O(1) time, no allocation per call.

use engine_input::{map::Key, PhysicalButton};

/// Engine-owned keyboard code: exactly the keys the game binds (WASD move,
/// QE strafe, F fire, E interact, space/shift/ctrl actions, escape/enter
/// menu). Left/right modifiers share one code: the bindings treat both sides
/// identically, so handedness never reaches the match.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum KeyCode {
    /// Move-forward key.
    W,
    /// Move-left key.
    A,
    /// Move-back key.
    S,
    /// Move-right key.
    D,
    /// Interact key.
    E,
    /// Fire key.
    F,
    /// Strafe key.
    Q,
    /// Jump/action key.
    Space,
    /// Modifier key (either shift side).
    Shift,
    /// Modifier key (either control side).
    Control,
    /// Cancel/menu key.
    Escape,
    /// Confirm/menu key.
    Enter,
}

/// Engine-owned mouse button: the buttons the game binds, without naming any
/// window backend. `Other` carries the raw extra-button index.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MouseButton {
    /// Primary button.
    Left,
    /// Secondary button.
    Right,
    /// Middle button.
    Middle,
    /// Back side button.
    Back,
    /// Forward side button.
    Forward,
    /// Extra button by raw backend index (clamped to `u8` on map).
    Other(u16),
}

/// Map one engine key code onto its [`PhysicalButton`]. Total by
/// construction: every engine code names a bound key. Pure.
pub fn map_key(code: KeyCode) -> PhysicalButton {
    let key = match code {
        KeyCode::W => Key::W,
        KeyCode::A => Key::A,
        KeyCode::S => Key::S,
        KeyCode::D => Key::D,
        KeyCode::E => Key::E,
        KeyCode::F => Key::F,
        KeyCode::Q => Key::Q,
        KeyCode::Space => Key::SPACE,
        KeyCode::Shift => Key::SHIFT,
        KeyCode::Control => Key::CTRL,
        KeyCode::Escape => Key::ESC,
        KeyCode::Enter => Key::ENTER,
    };
    PhysicalButton::Key(key.0)
}

/// Map one engine mouse button onto its [`PhysicalButton`]. Extra buttons
/// saturate into the last `u8` slot, so backends with wide indices keep
/// working. Pure.
pub fn map_mouse(button: MouseButton) -> PhysicalButton {
    match button {
        MouseButton::Left => PhysicalButton::Mouse(0),
        MouseButton::Right => PhysicalButton::Mouse(1),
        MouseButton::Middle => PhysicalButton::Mouse(2),
        MouseButton::Back => PhysicalButton::Mouse(3),
        MouseButton::Forward => PhysicalButton::Mouse(4),
        MouseButton::Other(index) => match u8::try_from(index) {
            Ok(narrow) => PhysicalButton::Mouse(narrow),
            Err(_) => PhysicalButton::Mouse(u8::MAX),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_maps_to_its_bound_button() {
        // Full table: one row per variant, so an added variant without a row
        // fails here rather than aliasing another key silently.
        for (code, want) in [
            (KeyCode::W, Key::W.0),
            (KeyCode::A, Key::A.0),
            (KeyCode::S, Key::S.0),
            (KeyCode::D, Key::D.0),
            (KeyCode::E, Key::E.0),
            (KeyCode::F, Key::F.0),
            (KeyCode::Q, Key::Q.0),
            (KeyCode::Space, Key::SPACE.0),
            (KeyCode::Shift, Key::SHIFT.0),
            (KeyCode::Control, Key::CTRL.0),
            (KeyCode::Escape, Key::ESC.0),
            (KeyCode::Enter, Key::ENTER.0),
        ] {
            assert_eq!(map_key(code), PhysicalButton::Key(want), "{code:?}");
        }
    }

    #[test]
    fn mouse_maps_buttons_and_clamps_extras() {
        assert_eq!(map_mouse(MouseButton::Left), PhysicalButton::Mouse(0));
        assert_eq!(map_mouse(MouseButton::Right), PhysicalButton::Mouse(1));
        assert_eq!(map_mouse(MouseButton::Middle), PhysicalButton::Mouse(2));
        assert_eq!(map_mouse(MouseButton::Back), PhysicalButton::Mouse(3));
        assert_eq!(map_mouse(MouseButton::Forward), PhysicalButton::Mouse(4));
        assert_eq!(map_mouse(MouseButton::Other(7)), PhysicalButton::Mouse(7));
        // Wide backend indices share the last slot instead of wrapping.
        assert_eq!(
            map_mouse(MouseButton::Other(300)),
            PhysicalButton::Mouse(255)
        );
    }
}
