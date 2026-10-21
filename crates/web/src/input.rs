//! Browser input mapped onto `engine-input` (WEB7): keyboard, mouse with pointer lock, and the
//! Gamepad API. Every decision here is pure (key codes, dead zones, delta accumulation, the
//! `input:` line) so it runs and tests natively; `browser_input` (wasm only) feeds it from DOM
//! events and polls the pads each frame.
//!
//! What comes out is the same thing the native window produces: a [`FighterInput`] built through
//! [`Bindings`] (so a rebind applies on the web too), plus a presentation-only look delta in
//! millidegrees for [`engine_input::look`]. Nothing here reads a clock, so the same script of
//! events gives the same tape bytes (see the tape test below).

use engine_input::look::MAX_DELTA_MILLI_DEG;
use engine_input::map::Key;
use engine_input::{
    apply_deadzone, Bindings, BoundAction, Buttons, FighterError, FighterId, FighterInput,
    PhysicalButton, DEFAULT_DEADZONE,
};

/// Radial dead zone of an analogue stick, in stick units (0..1). Below it the stick is at rest;
/// above it the deflection is rescaled so a stick just outside the zone starts near zero.
pub const STICK_DEADZONE: f64 = 0.18;

/// Size of the per-device held tables. Key codes are `u8` in `engine-input`; mouse buttons are
/// 0..=4 in the DOM; the Gamepad API "standard" layout has 17 buttons.
const TABLE: usize = 256;

/// The engine key a `KeyboardEvent.code` stands for, or `None` for a key the engine has no row
/// for. `code` is the physical key (layout independent), so WASD stays WASD on any keyboard.
#[must_use]
pub fn key_for_code(code: &str) -> Option<Key> {
    Some(match code {
        "Space" => Key::SPACE,
        "ShiftLeft" | "ShiftRight" => Key::SHIFT,
        "ControlLeft" | "ControlRight" => Key::CTRL,
        "KeyW" | "ArrowUp" => Key::W,
        "KeyA" | "ArrowLeft" => Key::A,
        "KeyS" | "ArrowDown" => Key::S,
        "KeyD" | "ArrowRight" => Key::D,
        "KeyE" => Key::E,
        "KeyF" => Key::F,
        "KeyQ" => Key::Q,
        "Escape" => Key::ESC,
        "Enter" | "NumpadEnter" => Key::ENTER,
        _ => return None,
    })
}

/// True for keys the page must not scroll or navigate with (space, the arrows) while the game
/// has them, so the handler calls `preventDefault` for exactly these.
#[must_use]
pub fn swallows_default(code: &str) -> bool {
    matches!(
        code,
        "Space" | "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight"
    )
}

/// What a mouse button does when the bindings table has no row for it: left attacks, right
/// blocks, middle dodges. `button` is `MouseEvent.button`.
#[must_use]
pub const fn mouse_action(button: u8) -> Option<BoundAction> {
    match button {
        0 => Some(BoundAction::Attack),
        2 => Some(BoundAction::Block),
        1 => Some(BoundAction::Dodge),
        _ => None,
    }
}

/// What a button of the Gamepad API "standard" layout does by default: face buttons dodge (A),
/// block (B) and attack (X, Y), bumpers and triggers attack and block.
#[must_use]
pub const fn pad_action(index: u8) -> Option<BoundAction> {
    match index {
        0 => Some(BoundAction::Dodge),
        1 | 4 | 6 => Some(BoundAction::Block),
        2 | 3 | 5 | 7 => Some(BoundAction::Attack),
        _ => None,
    }
}

/// Standard-layout d-pad buttons, which also steer.
const PAD_DPAD_UP: u8 = 12;
const PAD_DPAD_DOWN: u8 = 13;
const PAD_DPAD_LEFT: u8 = 14;
const PAD_DPAD_RIGHT: u8 = 15;

/// One pad as the Gamepad API reports it, reduced to what the mapping reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PadReading {
    /// `Gamepad.connected`.
    pub connected: bool,
    /// `Gamepad.axes`: left stick x, left stick y, right stick x, right stick y (-1..1, y down).
    pub axes: Vec<f64>,
    /// `GamepadButton.pressed` per button index.
    pub buttons: Vec<bool>,
}

/// Where pad readings come from: `navigator.getGamepads()` in the page, a script in tests.
pub trait PadSource {
    /// The current reading of every pad slot (an empty slot is a default, unconnected reading).
    fn read(&mut self) -> Vec<PadReading>;
}

/// A radial dead zone on a stick, as the wish pair the tape stores: `(mx, mz)` in -100..=100,
/// forward is stick up (negative y), right is positive x. Inside `deadzone` the answer is
/// `(0, 0)`; outside, the deflection is rescaled so the zone's edge is zero and a full stick
/// is 100, then the engine's own integer dead zone snaps what is left of noise.
#[must_use]
pub fn stick_wish(x: f64, y: f64, deadzone: f64) -> (i8, i8) {
    if !(x.is_finite() && y.is_finite()) {
        return (0, 0);
    }
    let dz = deadzone.clamp(0.0, 0.95);
    let mag = (x * x + y * y).sqrt();
    if mag <= dz {
        return (0, 0);
    }
    let scaled = (mag.min(1.0) - dz) / (1.0 - dz);
    let to_wish = |component: f64| -> i8 {
        let v = (component / mag * scaled * 100.0)
            .round()
            .clamp(-100.0, 100.0);
        // Rounded and clamped to -100..=100 above, so the cast cannot truncate.
        v as i8
    };
    apply_deadzone(to_wish(-y), to_wish(x), DEFAULT_DEADZONE)
}

/// Pointer-lock movement piled up between two ticks. Fractions carry over, so slow mouse
/// movement is not lost to rounding and a flick bigger than one tick's limit is spread over the
/// next ticks instead of clamped away.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LookAccum {
    x_px: f64,
    y_px: f64,
}

impl LookAccum {
    /// Add one `movementX/movementY` pair (pixels, y down). Non-finite input is dropped.
    pub fn push(&mut self, dx: f64, dy: f64) {
        if dx.is_finite() && dy.is_finite() {
            self.x_px += dx;
            self.y_px += dy;
        }
    }

    /// Drop what is pending (the lock was lost or taken).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The pending movement as `(dx, dy)` millidegrees for `LookState::apply_deltas` and keep
    /// the rest: yaw grows to the right, pitch grows upward (mouse up is negative `dy`). Each
    /// is limited to `engine_input::look::MAX_DELTA_MILLI_DEG`, which that call enforces.
    pub fn take_milli_deg(&mut self, milli_deg_per_px: f64) -> (i32, i32) {
        if !(milli_deg_per_px.is_finite() && milli_deg_per_px > 0.0) {
            return (0, 0);
        }
        let limit = f64::from(MAX_DELTA_MILLI_DEG);
        let take = |pending: &mut f64, sign: f64| -> i32 {
            let want = (*pending * milli_deg_per_px).trunc().clamp(-limit, limit);
            *pending -= want / milli_deg_per_px;
            // Truncated and clamped to +-MAX_DELTA_MILLI_DEG above.
            (want * sign) as i32
        };
        let dx = take(&mut self.x_px, 1.0);
        let dy = take(&mut self.y_px, -1.0);
        (dx, dy)
    }

    /// Pixels still pending, `(x, y)`; for tests and the receipt.
    #[must_use]
    pub fn pending(&self) -> (f64, f64) {
        (self.x_px, self.y_px)
    }
}

/// Centidegrees the seat turns per tick while a turn key is held: 90 degrees a second.
pub const TURN_CDEG_PER_TICK: i32 = 150;
/// Millidegrees of turn per pixel of pointer-lock movement.
pub const MILLI_DEG_PER_PX: f64 = 150.0;

/// The seat's absolute facing for the net duel, in centidegrees, turned by the mouse look and
/// the turn keys. The tape stores an absolute facing and WASD walk relative to it, so a seat
/// with no mouse needs keys to turn: Q turns left, E turns right.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facing {
    cdeg: i32,
}

impl Facing {
    /// Turn by `look_yaw_mdeg` (millidegrees, right positive) plus a tick of the turn keys and
    /// return the facing for this tick, wrapped into the contract range.
    pub fn turn(&mut self, look_yaw_mdeg: i32, left: bool, right: bool) -> i16 {
        let keys = TURN_CDEG_PER_TICK * (i32::from(right) - i32::from(left));
        let turned = self
            .cdeg
            .saturating_add(look_yaw_mdeg / 10)
            .saturating_add(keys);
        self.cdeg = (turned + 18_000).rem_euclid(36_000) - 18_000;
        i16::try_from(self.cdeg).unwrap_or(0)
    }
}

/// How many input events of each device the engine has taken in. These are the numbers of the
/// `input:` line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Key presses and releases of keys the engine knows (auto-repeat is not counted).
    pub kb: u32,
    /// Mouse button presses and releases, and pointer-lock movements.
    pub mouse: u32,
    /// Pad button presses and releases, and the stick leaving or re-entering its dead zone.
    pub pad: u32,
}

/// The one line the page prints: `input: kb=<n> mouse=<n> pad=<n>`.
#[must_use]
pub fn input_line(c: Counts) -> String {
    format!("input: kb={} mouse={} pad={}", c.kb, c.mouse, c.pad)
}

/// The held state of every device, the look accumulator and the counters.
#[derive(Clone, Debug)]
pub struct InputState {
    keys: [bool; TABLE],
    mouse: [bool; 8],
    pad_buttons: [bool; 32],
    pad_wish: (i8, i8),
    pad_stick_active: bool,
    locked: bool,
    look: LookAccum,
    counts: Counts,
    bindings: Bindings,
}

impl Default for InputState {
    fn default() -> Self {
        Self::new(Bindings::new())
    }
}

impl InputState {
    /// Nothing held, nothing counted, the given bindings table.
    #[must_use]
    pub fn new(bindings: Bindings) -> Self {
        Self {
            keys: [false; TABLE],
            mouse: [false; 8],
            pad_buttons: [false; 32],
            pad_wish: (0, 0),
            pad_stick_active: false,
            locked: false,
            look: LookAccum::default(),
            counts: Counts::default(),
            bindings,
        }
    }

    /// The counters so far.
    #[must_use]
    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// Whether the pointer is locked to the canvas.
    #[must_use]
    pub fn pointer_locked(&self) -> bool {
        self.locked
    }

    /// A `keydown` (`down`) or `keyup` for `KeyboardEvent.code`. True when the engine knows the
    /// key and its state changed; auto-repeat of a held key and unknown keys change nothing.
    pub fn key_event(&mut self, code: &str, down: bool) -> bool {
        let Some(key) = key_for_code(code) else {
            return false;
        };
        let slot = &mut self.keys[key.code()];
        if *slot == down {
            return false;
        }
        *slot = down;
        self.counts.kb = self.counts.kb.saturating_add(1);
        true
    }

    /// A `mousedown` or `mouseup` of `MouseEvent.button`. True when the state changed.
    pub fn mouse_button(&mut self, button: i16, down: bool) -> bool {
        let Ok(button) = usize::try_from(button) else {
            return false;
        };
        let Some(slot) = self.mouse.get_mut(button) else {
            return false;
        };
        if *slot == down {
            return false;
        }
        *slot = down;
        self.counts.mouse = self.counts.mouse.saturating_add(1);
        true
    }

    /// The page's `pointerlockchange`: lock taken or lost. Losing the lock drops the pending
    /// look and releases the mouse buttons (the matching `mouseup` may never arrive).
    pub fn set_pointer_lock(&mut self, locked: bool) {
        if self.locked && !locked {
            self.look.clear();
            self.mouse = [false; 8];
        }
        self.locked = locked;
    }

    /// A `mousemove` with `movementX/movementY`. Counts and accumulates only while the pointer
    /// is locked: a free cursor moving over the page is not look input.
    pub fn mouse_move(&mut self, dx: f64, dy: f64) -> bool {
        if !self.locked || !(dx.is_finite() && dy.is_finite()) || (dx == 0.0 && dy == 0.0) {
            return false;
        }
        self.look.push(dx, dy);
        self.counts.mouse = self.counts.mouse.saturating_add(1);
        true
    }

    /// Read every pad once: buttons are the OR over connected pads, the stick is the pad with the
    /// biggest deflection. Counts each button edge and each stick leaving or entering its zone.
    pub fn poll_pads(&mut self, source: &mut dyn PadSource) {
        let readings = source.read();
        let mut buttons = [false; 32];
        let mut best = (0_i8, 0_i8);
        let mut best_mag = 0_i32;
        for pad in readings.iter().filter(|p| p.connected) {
            for (i, pressed) in pad.buttons.iter().enumerate().take(buttons.len()) {
                buttons[i] |= *pressed;
            }
            let axis = |i: usize| pad.axes.get(i).copied().unwrap_or(0.0);
            let wish = stick_wish(axis(0), axis(1), STICK_DEADZONE);
            let mag = i32::from(wish.0).pow(2) + i32::from(wish.1).pow(2);
            if mag > best_mag {
                best_mag = mag;
                best = wish;
            }
        }
        // The d-pad steers at full deflection when no stick does.
        if best == (0, 0) {
            let held = |i: u8| buttons[usize::from(i)];
            let mx = i8::from(held(PAD_DPAD_UP)) * 100 - i8::from(held(PAD_DPAD_DOWN)) * 100;
            let mz = i8::from(held(PAD_DPAD_RIGHT)) * 100 - i8::from(held(PAD_DPAD_LEFT)) * 100;
            best = (mx, mz);
        }
        let edges = buttons
            .iter()
            .zip(self.pad_buttons.iter())
            .filter(|(now, before)| now != before)
            .count();
        let stick_active = best_mag > 0;
        let stick_edge = usize::from(stick_active != self.pad_stick_active);
        let total = u32::try_from(edges + stick_edge).unwrap_or(u32::MAX);
        self.counts.pad = self.counts.pad.saturating_add(total);
        self.pad_buttons = buttons;
        self.pad_wish = best;
        self.pad_stick_active = stick_active;
    }

    /// Let go of everything held (the window lost focus, so no key-up will arrive). Counters and
    /// the pointer-lock flag stay.
    pub fn release_all(&mut self) {
        self.keys = [false; TABLE];
        self.mouse = [false; 8];
        self.pad_buttons = [false; 32];
        self.pad_wish = (0, 0);
        self.pad_stick_active = false;
    }

    /// Everything held right now as the buttons the bindings table understands.
    fn pressed(&self) -> Vec<PhysicalButton> {
        let mut out = Vec::new();
        for (code, held) in self.keys.iter().enumerate() {
            if *held {
                if let Ok(code) = u8::try_from(code) {
                    out.push(PhysicalButton::Key(code));
                }
            }
        }
        for (code, held) in self.mouse.iter().enumerate() {
            if *held {
                if let Ok(code) = u8::try_from(code) {
                    out.push(PhysicalButton::Mouse(code));
                }
            }
        }
        for (code, held) in self.pad_buttons.iter().enumerate() {
            if *held {
                if let Ok(code) = u8::try_from(code) {
                    out.push(PhysicalButton::Pad(code));
                }
            }
        }
        out
    }

    /// The button bits the mouse and pad defaults add on top of the bindings table.
    fn default_bits(&self) -> u8 {
        let bit =
            |action: Option<BoundAction>| action.and_then(BoundAction::button_bit).unwrap_or(0);
        let mut bits = 0;
        for (code, held) in self.mouse.iter().enumerate() {
            if *held {
                bits |= bit(u8::try_from(code).ok().and_then(mouse_action));
            }
        }
        for (code, held) in self.pad_buttons.iter().enumerate() {
            if *held {
                bits |= bit(u8::try_from(code).ok().and_then(pad_action));
            }
        }
        bits
    }

    /// The tape record for this tick: buttons from the bindings table plus the mouse and pad
    /// defaults, the keyboard wish (or the pad's when no movement key is held), `facing` as
    /// given. Fails loudly on a bad slot or facing, like [`Bindings::to_fighter_input`].
    ///
    /// # Errors
    /// [`FighterError`] for an unknown fighter slot or a facing outside the contract range.
    pub fn fighter_input(
        &self,
        tick: u32,
        fighter: FighterId,
        facing: i16,
    ) -> Result<FighterInput, FighterError> {
        let pressed = self.pressed();
        let keyboard = self.bindings.wish_from_pressed(&pressed);
        let wish = if keyboard == (0, 0) {
            self.pad_wish
        } else {
            keyboard
        };
        let mut input = self
            .bindings
            .to_fighter_input(tick, fighter, &pressed, wish, facing)?;
        input.buttons = Buttons(input.buttons.0 | self.default_bits());
        input.validate()?;
        Ok(input)
    }

    /// The tape record of slot 0 facing 0 at `tick`: what the input tape samples (WEB7).
    ///
    /// # Errors
    /// Never for slot 0 and facing 0; the type is [`Self::fighter_input`]'s.
    pub fn slot0_record(&self, tick: u32) -> Result<FighterInput, FighterError> {
        self.fighter_input(tick, FighterId(0), 0)
    }

    /// Whether `key` is held right now.
    #[must_use]
    pub fn held(&self, key: Key) -> bool {
        self.keys[key.code()]
    }

    /// Take the pending look as millidegrees (see [`LookAccum::take_milli_deg`]).
    pub fn take_look(&mut self, milli_deg_per_px: f64) -> (i32, i32) {
        self.look.take_milli_deg(milli_deg_per_px)
    }

    /// Pixels of look still pending.
    #[must_use]
    pub fn pending_look(&self) -> (f64, f64) {
        self.look.pending()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_input::fighter::{ATTACK, BLOCK, DODGE};
    use engine_input::MAX_PARTICIPANTS;

    fn fighter() -> FighterId {
        FighterId::new(0).expect("slot 0 exists")
    }

    /// A pad source that plays back a fixed list of frames, one per `read`.
    struct Script(Vec<Vec<PadReading>>);

    impl PadSource for Script {
        fn read(&mut self) -> Vec<PadReading> {
            if self.0.is_empty() {
                Vec::new()
            } else {
                self.0.remove(0)
            }
        }
    }

    fn pad(axes: &[f64], pressed: &[u8]) -> PadReading {
        let mut buttons = vec![false; 17];
        for &i in pressed {
            buttons[usize::from(i)] = true;
        }
        PadReading {
            connected: true,
            axes: axes.to_vec(),
            buttons,
        }
    }

    #[test]
    fn key_codes_map_to_the_engine_keys_whatever_the_layout() {
        let rows = [
            ("Space", Key::SPACE),
            ("ShiftLeft", Key::SHIFT),
            ("ShiftRight", Key::SHIFT),
            ("ControlLeft", Key::CTRL),
            ("KeyW", Key::W),
            ("KeyA", Key::A),
            ("KeyS", Key::S),
            ("KeyD", Key::D),
            ("ArrowUp", Key::W),
            ("ArrowLeft", Key::A),
            ("ArrowDown", Key::S),
            ("ArrowRight", Key::D),
            ("KeyE", Key::E),
            ("KeyF", Key::F),
            ("KeyQ", Key::Q),
            ("Escape", Key::ESC),
            ("Enter", Key::ENTER),
            ("NumpadEnter", Key::ENTER),
        ];
        for (code, key) in rows {
            assert_eq!(key_for_code(code), Some(key), "{code}");
        }
        // `code` is the physical key: the character a layout prints is never looked at.
        for unknown in ["", "KeyZ", "F5", "w", "Digit1", "Tab"] {
            assert_eq!(key_for_code(unknown), None, "{unknown:?}");
        }
    }

    #[test]
    fn only_space_and_the_arrows_are_kept_from_the_page() {
        for code in ["Space", "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"] {
            assert!(swallows_default(code), "{code}");
        }
        for code in ["KeyW", "F5", "Tab", "Escape", "ShiftLeft"] {
            assert!(!swallows_default(code), "{code}");
        }
    }

    #[test]
    fn auto_repeat_and_unknown_keys_do_not_count() {
        let mut s = InputState::default();
        assert!(s.key_event("KeyW", true));
        assert!(
            !s.key_event("KeyW", true),
            "browser auto-repeat sends keydown again"
        );
        assert!(!s.key_event("KeyZ", true), "no engine key");
        assert!(
            !s.key_event("KeyS", false),
            "release of a key never pressed"
        );
        assert!(s.key_event("KeyW", false));
        assert_eq!(
            s.counts(),
            Counts {
                kb: 2,
                mouse: 0,
                pad: 0
            }
        );
    }

    #[test]
    fn wasd_and_the_default_buttons_build_the_tape_record() {
        let mut s = InputState::default();
        s.key_event("KeyW", true);
        s.key_event("KeyD", true);
        s.key_event("KeyF", true);
        s.key_event("ShiftLeft", true);
        let i = s.fighter_input(7, fighter(), 450).expect("valid record");
        assert_eq!((i.mx, i.mz), (100, 100));
        assert_eq!(i.buttons, Buttons(ATTACK | BLOCK));
        assert_eq!((i.tick, i.facing), (7, 450));
        // Opposite keys cancel, like the native table.
        s.key_event("KeyS", true);
        let i = s.fighter_input(8, fighter(), 0).expect("valid record");
        assert_eq!((i.mx, i.mz), (0, 100));
    }

    #[test]
    fn mouse_and_pad_buttons_press_the_defaults_and_a_rebind_wins() {
        let mut s = InputState::default();
        assert!(s.mouse_button(0, true));
        assert!(!s.mouse_button(0, true), "second down of a held button");
        assert!(s.mouse_button(2, true));
        assert!(!s.mouse_button(9, true), "no such button");
        assert!(!s.mouse_button(-1, true));
        let i = s.fighter_input(0, fighter(), 0).expect("valid record");
        assert_eq!(i.buttons, Buttons(ATTACK | BLOCK));

        let mut pads = Script(vec![vec![pad(&[], &[0])]]);
        s.poll_pads(&mut pads);
        let i = s.fighter_input(1, fighter(), 0).expect("valid record");
        assert_eq!(i.buttons, Buttons(ATTACK | BLOCK | DODGE));

        // The table can move a row onto a mouse button; both paths then agree on the bit.
        let mut table = Bindings::new();
        table.rebind(BoundAction::Dodge, PhysicalButton::Mouse(4));
        let mut s = InputState::new(table);
        s.mouse_button(4, true);
        let i = s.fighter_input(0, fighter(), 0).expect("valid record");
        assert_eq!(i.buttons, Buttons(DODGE));
    }

    #[test]
    fn stick_dead_zone_rests_rescales_and_saturates() {
        let dz = STICK_DEADZONE;
        assert_eq!(stick_wish(0.0, 0.0, dz), (0, 0));
        assert_eq!(stick_wish(0.1, -0.1, dz), (0, 0), "inside the zone");
        assert_eq!(stick_wish(dz, 0.0, dz), (0, 0), "the edge is still rest");
        assert_eq!(
            stick_wish(0.2, 0.0, dz),
            (0, 0),
            "just outside: a few percent, snapped by the integer zone"
        );
        assert_eq!(stick_wish(0.0, -1.0, dz), (100, 0), "up is forward");
        assert_eq!(stick_wish(0.0, 1.0, dz), (-100, 0), "down is back");
        assert_eq!(stick_wish(1.0, 0.0, dz), (0, 100), "right");
        assert_eq!(stick_wish(-1.0, 0.0, dz), (0, -100), "left");
        // A square-gate corner tops out at full deflection in the same direction.
        let (mx, mz) = stick_wish(1.0, -1.0, dz);
        assert_eq!(mx, mz, "a diagonal keeps its direction");
        assert!(mx > 60 && mx <= 100, "{mx}");
        // Half the usable range is about half the wish, and it grows with deflection.
        let half = stick_wish(0.0, -(dz + (1.0 - dz) / 2.0), dz);
        assert!((49..=51).contains(&half.0), "{half:?}");
        let mut last = 0;
        for k in 0..=20 {
            let (mx, _) = stick_wish(0.0, -f64::from(k) / 20.0, dz);
            assert!(mx >= last, "monotonic at {k}");
            last = mx;
        }
        // Garbage in, rest out.
        assert_eq!(stick_wish(f64::NAN, 0.5, dz), (0, 0));
        assert_eq!(stick_wish(0.5, f64::INFINITY, dz), (0, 0));
    }

    #[test]
    fn pads_count_edges_and_steer_from_the_stick_or_the_dpad() {
        let mut s = InputState::default();
        let mut script = Script(vec![
            vec![pad(&[0.0, 0.0], &[])],
            vec![pad(&[0.0, -1.0], &[2])],
            vec![pad(&[0.0, -1.0], &[2])],
            vec![pad(&[0.05, 0.02], &[])],
            vec![pad(&[0.0, 0.0], &[PAD_DPAD_LEFT])],
            vec![PadReading::default(), pad(&[1.0, 0.0], &[])],
        ]);
        s.poll_pads(&mut script);
        assert_eq!(s.counts().pad, 0, "rest: no edge");
        s.poll_pads(&mut script);
        assert_eq!(s.counts().pad, 2, "stick out and X down");
        let i = s.fighter_input(0, fighter(), 0).expect("valid record");
        assert_eq!((i.mx, i.mz, i.buttons), (100, 0, Buttons(ATTACK)));
        s.poll_pads(&mut script);
        assert_eq!(s.counts().pad, 2, "a held button is not a new event");
        s.poll_pads(&mut script);
        assert_eq!(s.counts().pad, 4, "back to rest and X up");
        s.poll_pads(&mut script);
        assert_eq!(s.counts().pad, 5, "d-pad left");
        let i = s.fighter_input(1, fighter(), 0).expect("valid record");
        assert_eq!((i.mx, i.mz), (0, -100), "d-pad left");
        s.poll_pads(&mut script);
        let i = s.fighter_input(2, fighter(), 0).expect("valid record");
        assert_eq!(
            (i.mx, i.mz),
            (0, 100),
            "stick of the second slot, empty slot ignored"
        );
        // No pads at all is rest, not an error.
        s.poll_pads(&mut Script(vec![]));
        let i = s.fighter_input(3, fighter(), 0).expect("valid record");
        assert_eq!((i.mx, i.mz, i.buttons), (0, 0, Buttons::NONE));
    }

    #[test]
    fn a_disconnected_pad_reads_as_rest() {
        let mut s = InputState::default();
        let mut gone = pad(&[1.0, 1.0], &[0, 2]);
        gone.connected = false;
        s.poll_pads(&mut Script(vec![vec![gone]]));
        assert_eq!(s.counts().pad, 0);
        let i = s.fighter_input(0, fighter(), 0).expect("valid record");
        assert_eq!((i.mx, i.mz, i.buttons), (0, 0, Buttons::NONE));
    }

    #[test]
    fn keyboard_movement_beats_the_pad_but_the_pad_fills_in() {
        let mut s = InputState::default();
        s.poll_pads(&mut Script(vec![vec![pad(&[0.0, -1.0], &[])]]));
        s.key_event("KeyA", true);
        let i = s.fighter_input(0, fighter(), 0).expect("valid record");
        assert_eq!((i.mx, i.mz), (0, -100), "a held movement key wins");
        s.key_event("KeyA", false);
        let i = s.fighter_input(1, fighter(), 0).expect("valid record");
        assert_eq!((i.mx, i.mz), (100, 0));
    }

    #[test]
    fn pointer_lock_gates_and_sums_the_mouse_delta() {
        let mut s = InputState::default();
        assert!(!s.mouse_move(10.0, 5.0), "a free cursor is not look input");
        assert_eq!(s.pending_look(), (0.0, 0.0));
        assert_eq!(s.counts().mouse, 0);

        s.set_pointer_lock(true);
        assert!(s.pointer_locked());
        assert!(s.mouse_move(10.0, -4.0));
        assert!(s.mouse_move(5.0, -2.0));
        assert!(!s.mouse_move(0.0, 0.0), "no movement is no event");
        assert!(!s.mouse_move(f64::NAN, 1.0));
        assert_eq!(s.pending_look(), (15.0, -6.0));
        assert_eq!(s.counts().mouse, 2);

        // 20 millidegrees per pixel: 15 px right is +300, 6 px up is +120 (pitch grows upward).
        assert_eq!(s.take_look(20.0), (300, 120));
        assert_eq!(s.pending_look(), (0.0, 0.0));
        assert_eq!(s.take_look(20.0), (0, 0), "taken once");
    }

    #[test]
    fn look_fractions_carry_and_a_flick_spreads_over_ticks() {
        let mut look = LookAccum::default();
        // 0.1 px at 10 mdeg/px is 1 mdeg: thirty small events must not round away.
        let mut total = 0;
        for _ in 0..30 {
            look.push(0.1, 0.0);
            total += look.take_milli_deg(10.0).0;
        }
        assert!((29..=30).contains(&total), "{total}");

        // 1000 px at 10 mdeg/px is 10000 mdeg: more than one tick may carry.
        look.clear();
        look.push(1000.0, 0.0);
        let first = look.take_milli_deg(10.0);
        assert_eq!(first, (MAX_DELTA_MILLI_DEG, 0));
        let second = look.take_milli_deg(10.0);
        assert!((3_999..=4_000).contains(&second.0), "{second:?}");
        assert_eq!(look.take_milli_deg(10.0).0, 0);

        // A bad gain takes nothing and keeps the pending movement.
        look.push(5.0, 5.0);
        assert_eq!(look.take_milli_deg(0.0), (0, 0));
        assert_eq!(look.take_milli_deg(f64::NAN), (0, 0));
        assert_eq!(look.pending(), (5.0, 5.0));
    }

    #[test]
    fn losing_the_pointer_lock_drops_pending_look_and_mouse_buttons() {
        let mut s = InputState::default();
        s.set_pointer_lock(true);
        s.mouse_button(0, true);
        s.mouse_move(40.0, 40.0);
        s.set_pointer_lock(false);
        assert!(!s.pointer_locked());
        assert_eq!(s.pending_look(), (0.0, 0.0));
        let i = s.fighter_input(0, fighter(), 0).expect("valid record");
        assert_eq!(
            i.buttons,
            Buttons::NONE,
            "the mouseup never came, the button is released"
        );
    }

    #[test]
    fn losing_focus_lets_go_of_everything_but_keeps_the_counts() {
        let mut s = InputState::default();
        s.key_event("KeyW", true);
        s.mouse_button(0, true);
        s.release_all();
        let i = s.fighter_input(0, fighter(), 0).expect("valid record");
        assert_eq!((i.mx, i.mz, i.buttons), (0, 0, Buttons::NONE));
        assert_eq!(
            s.counts(),
            Counts {
                kb: 1,
                mouse: 1,
                pad: 0
            }
        );
        // The key-up that follows the refocus is a change from "not held": not counted.
        assert!(!s.key_event("KeyW", false));
    }

    #[test]
    fn bad_slot_or_facing_is_refused_not_clamped() {
        let s = InputState::default();
        // The cap is the engine's, not a number typed here: the last slot
        // inside it is legal and the first slot past it is refused, never
        // clamped onto the last one.
        let last = FighterId(MAX_PARTICIPANTS - 1);
        assert!(
            s.fighter_input(0, last, 0).is_ok(),
            "the last slot inside the cap is legal"
        );
        assert_eq!(
            s.fighter_input(0, FighterId(MAX_PARTICIPANTS), 0).unwrap_err(),
            FighterError::UnknownFighter(MAX_PARTICIPANTS)
        );
        assert_eq!(
            s.fighter_input(0, fighter(), 18_001).unwrap_err(),
            FighterError::FacingOutOfRange(18_001)
        );
    }

    /// The same script of device events gives the same tape bytes twice, and those bytes are the
    /// records `Bindings` builds directly from the same pressed buttons.
    #[test]
    fn a_scripted_run_gives_the_same_tape_bytes_twice() {
        fn run() -> Vec<u8> {
            let mut s = InputState::default();
            let mut bytes = Vec::new();
            let steps: [(&str, bool); 6] = [
                ("KeyW", true),
                ("KeyF", true),
                ("KeyF", false),
                ("KeyD", true),
                ("KeyW", false),
                ("KeyD", false),
            ];
            for (tick, (code, down)) in steps.into_iter().enumerate() {
                s.key_event(code, down);
                let tick = u32::try_from(tick).expect("small");
                s.fighter_input(tick, fighter(), 0)
                    .expect("valid record")
                    .append_bytes(&mut bytes);
            }
            bytes
        }
        let first = run();
        assert_eq!(first, run());
        assert_eq!(first.len(), 6 * 10, "v1 records are 10 bytes");

        let table = Bindings::new();
        let (w, d, f) = (
            PhysicalButton::Key(Key::W.0),
            PhysicalButton::Key(Key::D.0),
            PhysicalButton::Key(Key::F.0),
        );
        let held: [Vec<PhysicalButton>; 6] =
            [vec![w], vec![w, f], vec![w], vec![w, d], vec![d], vec![]];
        let mut expect = Vec::new();
        for (tick, pressed) in held.iter().enumerate() {
            let wish = table.wish_from_pressed(pressed);
            table
                .to_fighter_input(
                    u32::try_from(tick).expect("small"),
                    fighter(),
                    pressed,
                    wish,
                    0,
                )
                .expect("valid record")
                .append_bytes(&mut expect);
        }
        assert_eq!(first, expect);
    }

    #[test]
    fn the_input_line_has_the_page_shape() {
        assert_eq!(
            input_line(Counts {
                kb: 3,
                mouse: 2,
                pad: 0
            }),
            "input: kb=3 mouse=2 pad=0"
        );
        assert_eq!(input_line(Counts::default()), "input: kb=0 mouse=0 pad=0");
    }

    #[test]
    fn the_seat_turns_with_the_mouse_and_the_turn_keys_and_wraps() {
        let mut facing = Facing::default();
        assert_eq!(facing.turn(0, false, false), 0);
        // 20 degrees of mouse look (20000 millidegrees) is 2000 centidegrees.
        assert_eq!(facing.turn(20_000, false, false), 2_000);
        // E turns right and Q turns left by the same step per tick.
        assert_eq!(facing.turn(0, false, true), 2_150);
        assert_eq!(facing.turn(0, true, false), 2_000);
        assert_eq!(facing.turn(0, true, true), 2_000, "both keys cancel");
        // Past half a turn the facing wraps to the other side, always inside the contract.
        let mut wrap = Facing::default();
        assert_eq!(wrap.turn(170_000, false, false), 17_000);
        assert_eq!(wrap.turn(30_000, false, false), -16_000);
        for _ in 0..1_000 {
            let value = i32::from(wrap.turn(MAX_DELTA_MILLI_DEG, false, true));
            assert!((-18_000..=18_000).contains(&value));
        }
    }

    #[test]
    fn a_held_turn_key_is_visible_and_released_with_the_rest() {
        let mut state = InputState::default();
        assert!(!state.held(Key::E));
        state.key_event("KeyE", true);
        assert!(state.held(Key::E) && !state.held(Key::Q));
        state.release_all();
        assert!(!state.held(Key::E));
    }
}
