//! arena_winit: winit edge for the `arena` binary.
//!
//! Consumer: the binary event handler maps platform keys before feeding
//! the match. Data owner: this module owns the winit-to-engine input bridge
//! ([`from_winit_key`], [`from_winit_mouse`]); the engine codes live in
//! [`engine_game::input`]. Units: button codes. Determinism scope: pure
//! maps, no I/O. Errors: none; unmapped keys yield `None` and are ignored by
//! the bindings lookup.
//!
//! Budget: O(1) time, no allocation per call.

use engine_game::input::{KeyCode, MouseButton};

/// Map one winit key code onto its engine [`KeyCode`]. Keys outside the
/// default table (function keys, arrows, numpad) yield `None` and are ignored
/// by the bindings lookup, so a focused window keeps working. Pure. The winit
/// edge lives in this binary, never in the game API.
pub fn from_winit_key(code: winit::keyboard::KeyCode) -> Option<KeyCode> {
    use winit::keyboard::KeyCode as C;
    Some(match code {
        C::KeyW => KeyCode::W,
        C::KeyA => KeyCode::A,
        C::KeyS => KeyCode::S,
        C::KeyD => KeyCode::D,
        C::KeyF => KeyCode::F,
        C::KeyE => KeyCode::E,
        C::KeyQ => KeyCode::Q,
        C::Space => KeyCode::Space,
        C::ShiftLeft | C::ShiftRight => KeyCode::Shift,
        C::ControlLeft | C::ControlRight => KeyCode::Control,
        C::Escape => KeyCode::Escape,
        C::Enter => KeyCode::Enter,
        _ => return None,
    })
}

/// Map one winit mouse button onto its engine [`MouseButton`]. Pure; same
/// winit-edge note as [`from_winit_key`].
pub fn from_winit_mouse(button: winit::event::MouseButton) -> MouseButton {
    use winit::event::MouseButton as B;
    match button {
        B::Left => MouseButton::Left,
        B::Right => MouseButton::Right,
        B::Middle => MouseButton::Middle,
        B::Back => MouseButton::Back,
        B::Forward => MouseButton::Forward,
        B::Other(index) => MouseButton::Other(index),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_game::input::{map_key, map_mouse};
    use engine_input::{map::Key as K, PhysicalButton};

    #[test]
    fn winit_edge_maps_bound_keys_and_rejects_arrows() {
        use winit::keyboard::KeyCode as C;
        // Bound keys reach their engine codes; modifiers merge sides.
        assert_eq!(from_winit_key(C::KeyW), Some(KeyCode::W));
        assert_eq!(from_winit_key(C::KeyF), Some(KeyCode::F));
        assert_eq!(from_winit_key(C::Space), Some(KeyCode::Space));
        assert_eq!(from_winit_key(C::ShiftRight), Some(KeyCode::Shift));
        assert_eq!(from_winit_key(C::ControlLeft), Some(KeyCode::Control));
        // Arrows and function keys are ignored, so focus keeps working.
        assert_eq!(from_winit_key(C::ArrowUp), None);
        assert_eq!(from_winit_key(C::F1), None);
        // The engine leg maps without winit: spot-check the bridge output.
        assert_eq!(
            map_key(from_winit_key(C::KeyW).expect("W maps")),
            PhysicalButton::Key(K::W.0)
        );
        assert_eq!(
            map_mouse(from_winit_mouse(winit::event::MouseButton::Left)),
            PhysicalButton::Mouse(0)
        );
    }
}
