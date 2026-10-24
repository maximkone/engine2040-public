//! Gamepad sticks and buttons through one shared bindings table.
//!
//! Owner: this module owns the stick snapshot plus the merge into the
//! caller's pressed list; the input crate owns the bindings table, the
//! deadzone shape and the validated fighter command. Units: stick
//! deflection is a finite 32-bit float in `-1.0..=1.0` per axis, wishes
//! are `-100..=100` percent of top speed, deadzones are wish units,
//! ticks are zero-based fixed steps. Determinism: the same snapshot plus
//! the same table gives the same command twice; iteration is index order,
//! never hash order, with no wall clock. Errors: bad fighter slots fail
//! through the shared fighter error; non-finite deflection rests at zero.

use engine_input::fighter::WISH_RANGE;
use engine_input::{
    apply_deadzone, Bindings, BoundAction, FighterError, FighterId, FighterInput, PhysicalButton,
};

use crate::fps_view::pad_rumble::{focus_lost, rumble_for_cue, RumbleRow, MOTORS_OFF_MS};
use engine_audio::FightCue;

/// Last device that acted on the input map: it decides which prompt glyph set the HUD draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DeviceKind {
    /// Keyboard and mouse.
    #[default]
    Keyboard,
    /// Gamepad.
    Pad,
}

/// The prompt glyph set the HUD draws for the last device: letter keys for the keyboard, face
/// buttons for the pad. One set at a time, so a prompt never mixes `E` with `A`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GlyphSet {
    /// Keyboard letters.
    #[default]
    Keyboard,
    /// Pad face buttons.
    Pad,
}

impl DeviceKind {
    /// The glyph set this device draws.
    #[must_use]
    pub fn glyphs(self) -> GlyphSet {
        match self {
            Self::Keyboard => GlyphSet::Keyboard,
            Self::Pad => GlyphSet::Pad,
        }
    }
}

/// One input event the window folds into [`PadState`]. Pure data: the winit and gilrs glue
/// translates its own events into these and nothing else.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PadEvent {
    /// The window lost focus (`WindowEvent::Focused(false)`).
    FocusLost,
    /// The window holds focus again.
    FocusGained,
    /// A pad was plugged in (or answered for the first time).
    PadConnected,
    /// The active pad went away: the Steam Deck rule pauses the local duel.
    PadDisconnected,
    /// A pad button went down: the pad becomes the prompt device.
    PadPress,
    /// A keyboard key went down: the keyboard becomes the prompt device.
    KeyPress,
    /// One fight cue wants to rumble.
    Cue(FightCue),
}

/// What one [`PadState::step`] decided: the motors to play, when they must be off, the prompt
/// glyph set and whether a local duel is paused.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PadStep {
    /// Motor row for this tick; a silent row means nothing to play.
    pub motors: RumbleRow,
    /// Event-time deadline the motors must be off by (ms), when step asked for a cut.
    pub motors_off_at_ms: Option<u32>,
    /// Prompt glyph set after the event.
    pub glyphs: GlyphSet,
    /// True while a pad disconnect holds a local duel paused.
    pub paused: bool,
}

/// Pure pad policy: focus, connection, last device and the pause rule as one step function.
/// The window feeds it its events and plays what it returns; nothing here touches a device.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PadState {
    focused: bool,
    last: DeviceKind,
    paused: bool,
    motors: RumbleRow,
}

impl Default for PadState {
    fn default() -> Self {
        Self::new()
    }
}

impl PadState {
    /// A focused window on the keyboard with no pad and no rumble playing.
    #[must_use]
    pub fn new() -> Self {
        Self {
            focused: true,
            last: DeviceKind::Keyboard,
            paused: false,
            motors: RumbleRow::default(),
        }
    }

    /// Fold one event at event time `at_ms` into the state and return the decision. Focus loss
    /// asks for the motors off inside [`MOTORS_OFF_MS`]; a disconnect pauses at once (within the
    /// same tick) and a cue plays its row unchanged while focused and unpaused.
    pub fn step(&mut self, event: PadEvent, at_ms: u32) -> PadStep {
        let mut deadline = None;
        match event {
            PadEvent::FocusLost => {
                self.focused = false;
                self.motors = focus_lost();
                deadline = Some(at_ms.saturating_add(MOTORS_OFF_MS));
            }
            PadEvent::FocusGained => {
                self.focused = true;
                self.motors = RumbleRow::default();
            }
            PadEvent::PadConnected => {
                self.paused = false;
            }
            PadEvent::PadDisconnected => {
                // Steam Deck rule: the duel pauses on the tick the pad leaves.
                self.paused = true;
                self.motors = RumbleRow::default();
                deadline = Some(at_ms);
            }
            PadEvent::PadPress => self.last = DeviceKind::Pad,
            PadEvent::KeyPress => self.last = DeviceKind::Keyboard,
            PadEvent::Cue(cue) => {
                self.motors = rumble_for_cue(cue, self.focused && !self.paused);
                if self.motors.ms > 0 {
                    deadline = Some(at_ms.saturating_add(self.motors.ms));
                }
            }
        }
        PadStep {
            motors: self.motors,
            motors_off_at_ms: deadline,
            glyphs: self.last.glyphs(),
            paused: self.paused,
        }
    }

    /// Prompt glyph set for the last device.
    #[must_use]
    pub fn glyphs(&self) -> GlyphSet {
        self.last.glyphs()
    }

    /// True while a pad disconnect holds a local duel paused.
    #[must_use]
    pub fn paused(&self) -> bool {
        self.paused
    }

    /// The row playing now (silent when nothing plays).
    #[must_use]
    pub fn motors(&self) -> RumbleRow {
        self.motors
    }

    /// True when the window holds focus.
    #[must_use]
    pub fn focused(&self) -> bool {
        self.focused
    }
}

/// One gamepad snapshot as plain data: no device handles, no threads.
///
/// The window layer fills one of these per fixed tick from whatever pad
/// backend it uses; this module only reads the snapshot. The button bits
/// cover every pad code the shared button type can name. Sticks rest at
/// zero deflection; non-finite readings also rest at zero once quantized.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PadFrame {
    /// Left stick sideways deflection, `-1.0` left to `1.0` right.
    pub lx: f32,
    /// Left stick forward deflection, `-1.0` back to `1.0` forward.
    pub ly: f32,
    /// Held pad buttons as bits: bit `i` means pad code `i` is down.
    pub buttons: [u64; 4],
}

impl PadFrame {
    /// Empty snapshot: sticks centered, no buttons held.
    ///
    /// Units: none. Ownership: caller-owned value. Threading: `Send + Sync`,
    /// safe on any thread. Determinism: bitwise equal snapshots compare equal.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            lx: 0.0,
            ly: 0.0,
            buttons: [0, 0, 0, 0],
        }
    }

    /// Snapshot from raw readings: stick deflection plus the held pad codes.
    ///
    /// Units: sticks in `-1.0..=1.0` (values outside clamp at quantize time),
    /// codes are pad button indices. Ownership: caller-owned value.
    /// Threading: `Send + Sync`. Determinism: the same inputs give the same
    /// bits twice; duplicate codes collapse to one bit. Errors: none; every
    /// input maps to some snapshot, never panics.
    #[must_use]
    pub fn snapshot(lx: f32, ly: f32, pressed: &[u8]) -> Self {
        let mut buttons = [0u64, 0u64, 0u64, 0u64];
        for code in pressed {
            let word = usize::from(*code / 64);
            let bit = *code % 64;
            // The word index always fits the four-word table by construction.
            if let Some(slot) = buttons.get_mut(word) {
                *slot |= 1u64 << bit;
            }
        }
        Self { lx, ly, buttons }
    }

    /// True when pad code `index` is held in this snapshot.
    ///
    /// Units: button index. Ownership: borrows the snapshot. Threading:
    /// `Send + Sync`. Determinism: pure bit read.
    #[must_use]
    pub fn is_pressed(self, index: u8) -> bool {
        let word = usize::from(index / 64);
        let bit = index % 64;
        self.buttons
            .get(word)
            .is_some_and(|slot| slot & (1u64 << bit) != 0)
    }

    /// Stick wish for this snapshot: sideways becomes right wish, forward
    /// becomes forward wish, quantized then passed through `deadzone`.
    ///
    /// Units: wishes in `-100..=100`, deadzone in wish units. Ownership:
    /// borrows the snapshot. Threading: `Send + Sync`. Determinism: pure
    /// over the two deflections plus the deadzone; non-finite deflection
    /// rests at zero.
    #[must_use]
    pub fn stick_wish(self, deadzone: u8) -> (i8, i8) {
        stick_wish(self.lx, self.ly, deadzone)
    }

    /// Append held pad buttons to the caller's pressed list in index order,
    /// skipping codes already present so one button never fires twice.
    ///
    /// Units: button indices. Ownership: borrows the snapshot, extends the
    /// caller's buffer which keeps its capacity. Threading: caller thread
    /// only, no locks. Determinism: index order `0..=255`; the same snapshot
    /// plus the same incoming list appends the same codes twice. Budget:
    /// at most one scan of the button table; no heap use beyond the
    /// caller's buffer growth.
    pub fn append_pressed(self, out: &mut Vec<PhysicalButton>) {
        for index in 0..=u8::MAX {
            if self.is_pressed(index) {
                let button = PhysicalButton::Pad(index);
                if !out.contains(&button) {
                    out.push(button);
                }
            }
        }
    }
}

/// Quantize one stick axis to wish units: finite deflection scales by one
/// hundred and rounds, non-finite deflection rests at zero.
///
/// Units: input in `-1.0..=1.0` (outside values clamp), output in
/// `-100..=100`. Ownership: pure value function. Threading: `Send + Sync`.
/// Determinism: the same float gives the same byte twice; the float total
/// order decides rounding, never platform state. Errors: none; every float
/// maps to some wish, never panics.
#[must_use]
pub fn quantize_axis(value: f32) -> i8 {
    if !value.is_finite() {
        return 0;
    }
    let clamped = value.clamp(-1.0, 1.0);
    let scaled = (clamped * 100.0).round();
    // The rounded scale is an exact integer float in range, so it equals
    // exactly one wish converted back; the search converts integers to
    // floats (exact here) instead of floats to integers.
    for candidate in -100i8..=100i8 {
        if f32::from(candidate) == scaled {
            return candidate;
        }
    }
    0
}

/// Stick deflection to wish: sideways maps to right wish, forward maps to
/// forward wish, quantized then snapped through `deadzone` by the shared
/// deadzone owner.
///
/// Units: deflection in `-1.0..=1.0` per axis, wishes in `-100..=100`,
/// deadzone in wish units. Ownership: pure value function. Threading:
/// `Send + Sync`. Determinism: pure over the two floats plus the deadzone.
/// Errors: none; every pair maps to some wish, never panics.
#[must_use]
pub fn stick_wish(lx: f32, ly: f32, deadzone: u8) -> (i8, i8) {
    let mz = quantize_axis(lx);
    let mx = quantize_axis(ly);
    apply_deadzone(mx, mz, deadzone)
}

/// Add a button wish and a stick wish with saturation to the wish range.
///
/// Units: wishes in `-100..=100` per axis. Ownership: pure value function.
/// Threading: `Send + Sync`. Determinism: pure integer math; opposed full
/// deflections clamp rather than wrap. Errors: none; every pair maps to a
/// wish in range, never panics.
#[must_use]
pub fn combine_wish(button: (i8, i8), stick: (i8, i8)) -> (i8, i8) {
    let mx = i16::from(button.0) + i16::from(stick.0);
    let mz = i16::from(button.1) + i16::from(stick.1);
    let lo = i16::from(-WISH_RANGE);
    let hi = i16::from(WISH_RANGE);
    let mx = mx.clamp(lo, hi);
    let mz = mz.clamp(lo, hi);
    // Both sums clamp to the wish range above, so the fallible step always
    // succeeds; the fallback only satisfies the total function contract.
    let mx = i8::try_from(mx).unwrap_or(0);
    let mz = i8::try_from(mz).unwrap_or(0);
    (mx, mz)
}

/// One recorded pad event in backend-neutral form: the gilrs poll call
/// will emit these once per frame, and tests feed them by hand.
///
/// Units: button codes are pad indices `0..=255`, axes in `-1.0..=1.0`.
/// Ownership: caller-owned values. Threading: `Send + Sync`.
/// Determinism: the same list folds to the same snapshot twice.
/// Errors: none; every event maps to some state, never panics.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RecordedPadEvent {
    /// Pad code went down.
    ButtonDown(u8),
    /// Pad code went up.
    ButtonUp(u8),
    /// Stick pair replaced wholesale; the last axis event wins.
    Axis { lx: f32, ly: f32 },
}

/// Fold a recorded event list into one snapshot: downs set bits, ups clear
/// them, the last axis event sets the sticks. Button codes above 255 cannot
/// exist (`u8`), so every code maps to a bit; non-finite axes rest at zero
/// once quantized downstream.
///
/// Units: as in [`RecordedPadEvent`]. Ownership: borrows the list, returns
/// an owned snapshot. Threading: caller thread only, no locks.
/// Determinism: pure fold in list order; the same list gives the same bytes
/// twice. Budget: one pass over the list, no heap. Errors: none, never
/// panics.
#[must_use]
pub fn snapshot_from_recorded(events: &[RecordedPadEvent]) -> PadFrame {
    let mut lx = 0.0f32;
    let mut ly = 0.0f32;
    let mut buttons = [0u64, 0u64, 0u64, 0u64];
    for event in events {
        match *event {
            RecordedPadEvent::ButtonDown(code) => {
                let word = usize::from(code / 64);
                let bit = code % 64;
                if let Some(slot) = buttons.get_mut(word) {
                    *slot |= 1u64 << bit;
                }
            }
            RecordedPadEvent::ButtonUp(code) => {
                let word = usize::from(code / 64);
                let bit = code % 64;
                if let Some(slot) = buttons.get_mut(word) {
                    *slot &= !(1u64 << bit);
                }
            }
            RecordedPadEvent::Axis { lx: x, ly: y } => {
                lx = x;
                ly = y;
            }
        }
    }
    let mut frame = PadFrame::snapshot(lx, ly, &[]);
    frame.buttons = buttons;
    frame
}

/// Fold one device poll into a snapshot: the backend drains its pending
/// events into a list once per frame, and this folds that list through
/// [`snapshot_from_recorded`].
///
/// Units: as in [`RecordedPadEvent`]. Ownership: takes the drained list by
/// value (the caller moves its per-frame drain in) and returns an owned
/// snapshot. Threading: caller thread only, no locks, no threads.
/// Determinism: pure over the drained list; the same list gives the same
/// bytes twice, and stick clamping happens at quantize time through
/// [`PadFrame::stick_wish`] exactly like [`PadFrame::snapshot`], so an
/// out-of-range or non-finite axis rests the same way. Budget: one pass
/// over the list, no heap beyond the moved-in drain. Errors: none, never
/// panics.
#[must_use]
pub fn poll_gilrs_once(read: impl FnOnce() -> Vec<RecordedPadEvent>) -> PadFrame {
    let events = read();
    snapshot_from_recorded(&events)
}

/// Merge keyboard and pad buttons into one pressed list with no duplicates.
///
/// The caller holds `out` across ticks: clear it, copy the keyboard slice in
/// order, then append pad codes in index order skipping repeats. There is a
/// single input state per tick, never a pad table beside the keyboard table.
///
/// Units: button codes. Ownership: borrows both inputs, reuses the caller's
/// buffer which keeps its capacity. Threading: caller thread only, no locks.
/// Determinism: keyboard order then pad index order; the same inputs merge
/// to the same list twice. Budget: one pass over each input; no heap use
/// beyond the caller's buffer growth.
pub fn merge_pressed(keyboard: &[PhysicalButton], pad: &PadFrame, out: &mut Vec<PhysicalButton>) {
    out.clear();
    out.extend_from_slice(keyboard);
    pad.append_pressed(out);
}

/// Build one validated fighter command from merged buttons plus stick wish.
///
/// Button wishes come from the shared table over `pressed`, the stick wish
/// adds on with saturation, facing stays pinned at zero like the keyboard
/// path, and the table validates the result. Rebinding any action row to a
/// pad code routes that pad button through this same call.
///
/// Units: wishes in `-100..=100`, facing in centidegrees (always zero here),
/// ticks are zero-based. Ownership: borrows the pressed slice and the table.
/// Threading: caller thread only, no locks. Determinism: pure over the
/// inputs plus the table; the same row gives the same command twice.
/// Errors: [`FighterError`] when the fighter slot or the combined wish is
/// out of range; the table refuses loudly, never clamps silently.
pub fn fighter_input_from_pad(
    tick: u32,
    fighter: FighterId,
    pressed: &[PhysicalButton],
    stick: (i8, i8),
    bindings: &Bindings,
) -> Result<FighterInput, FighterError> {
    let button = bindings.wish_from_pressed(pressed);
    let wish = combine_wish(button, stick);
    bindings.to_fighter_input(tick, fighter, pressed, wish, 0)
}

/// FPS input map: WASD movement, mouse fire/aim, everything rebindable.
///
/// Starts from the default table (the move rows are WASD already) and moves
/// the two aim rows onto the mouse: the primary button fires (attack), the
/// secondary aims (block). Dodge and the deadzone keep their defaults. Look
/// itself stays presentation-side in the camera director: facing is pinned
/// to zero here exactly like the keyboard path, so replays never depend on
/// a camera angle. Every row stays rebindable through [`Bindings::rebind`].
///
/// Units: buttons as [`PhysicalButton`] codes. Ownership: returns an owned
/// table. Threading: caller thread only, no locks. Determinism: the same
/// call builds the same table twice. Errors: none; every row names a valid
/// button, never panics.
#[must_use]
pub fn fps_bindings() -> Bindings {
    let mut bindings = Bindings::new();
    bindings.rebind(BoundAction::Attack, PhysicalButton::Mouse(0));
    bindings.rebind(BoundAction::Block, PhysicalButton::Mouse(1));
    bindings
}

/// Poll one frame of pad events and build the tick input: the once-per-frame
/// backend poll folds `pad_events` into a [`PadFrame`], pad codes merge with
/// the keyboard `pressed` list in fixed order, the stick wish adds onto the
/// button wish with saturation, and the table validates the result.
///
/// `pad_events` is the frame's drained backend event slice: empty while no
/// device backend is linked (sticks centered, no buttons held), one backend
/// drain per frame once the gilrs poll lands. Facing stays pinned at zero
/// like the keyboard path, so the same events give the same input twice.
/// The windowed leg calls this once per tick and reuses its pressed buffer.
///
/// Units: wishes in `-100..=100`, facing in centidegrees (always zero here),
/// ticks are zero-based. Ownership: borrows both inputs plus the table and
/// reuses the caller's `scratch` buffer, which keeps its capacity.
/// Threading: caller thread only, no locks. Determinism: pure over the
/// inputs plus the table; index order, never hash order. Budget: one pass
/// over each input; no heap use beyond the caller's buffer growth. Errors:
/// [`FighterError`] when the fighter slot is bad; the table refuses loudly,
/// never clamps silently.
pub fn tick_input_with_pad(
    tick: u32,
    fighter: FighterId,
    keyboard: &[PhysicalButton],
    pad_events: &[RecordedPadEvent],
    bindings: &Bindings,
    scratch: &mut Vec<PhysicalButton>,
) -> Result<FighterInput, FighterError> {
    let pad: PadFrame = snapshot_from_recorded(pad_events);
    merge_pressed(keyboard, &pad, scratch);
    let stick = pad.stick_wish(bindings.deadzone());
    fighter_input_from_pad(tick, fighter, scratch, stick, bindings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe;
    use engine_input::{
        fighter::{ATTACK, BLOCK},
        Bindings, BoundAction, Buttons,
    };

    fn fighter() -> FighterId {
        FighterId::new(0).expect("slot zero is always valid")
    }

    #[test]
    fn axes_quantize_full_half_and_rest() {
        assert_eq!(quantize_axis(1.0), 100);
        assert_eq!(quantize_axis(-1.0), -100);
        assert_eq!(quantize_axis(0.0), 0);
        assert_eq!(quantize_axis(0.5), 50);
        assert_eq!(quantize_axis(-0.25), -25);
        // Outside deflection clamps instead of escaping the wish range.
        assert_eq!(quantize_axis(2.0), 100);
        assert_eq!(quantize_axis(-3.0), -100);
        // Non-finite readings rest at zero so no bad float reaches the hash.
        assert_eq!(quantize_axis(f32::NAN), 0);
        assert_eq!(quantize_axis(f32::INFINITY), 0);
        assert_eq!(quantize_axis(f32::NEG_INFINITY), 0);
    }

    #[test]
    fn deadzone_snaps_noise_but_keeps_drive() {
        let bindings = Bindings::new();
        let dead = bindings.deadzone();
        // Stick noise inside the deadzone rests.
        assert_eq!(PadFrame::snapshot(0.02, 0.01, &[]).stick_wish(dead), (0, 0));
        assert_eq!(stick_wish(0.0, 0.0, dead), (0, 0));
        // Full deflection drives full wish through the same deadzone.
        assert_eq!(stick_wish(1.0, 1.0, dead), (100, 100));
        assert_eq!(stick_wish(-1.0, 0.5, dead), (50, -100));
        // The frame helper agrees with the free function on the same inputs.
        let frame = PadFrame::snapshot(0.5, -0.5, &[]);
        assert_eq!(frame.stick_wish(dead), stick_wish(0.5, -0.5, dead));
    }

    #[test]
    fn rebound_pad_button_fires_and_old_key_stops() {
        let mut bindings = Bindings::new();
        let old = bindings.button(BoundAction::Attack);
        bindings.rebind(BoundAction::Attack, PhysicalButton::Pad(5));
        let pressed = [PhysicalButton::Pad(5)];
        let input = fighter_input_from_pad(0, fighter(), &pressed, (0, 0), &bindings)
            .expect("pad attack validates");
        assert_eq!(input.buttons, Buttons(ATTACK));
        // The old key no longer attacks once its row moves to the pad.
        let stale = fighter_input_from_pad(0, fighter(), &[old], (0, 0), &bindings)
            .expect("stale key validates");
        assert_eq!(stale.buttons, Buttons::NONE);
        // Every action row accepts a pad code, so a full fight fits on pads.
        for action in BoundAction::ALL {
            let code = u8::try_from(action.index())
                .expect("action table fits in a pad code")
                .saturating_add(10);
            bindings.rebind(action, PhysicalButton::Pad(code));
        }
        let config = engine_input::InputConfig::from_bindings(&bindings);
        let restored = config.to_bindings();
        assert_eq!(restored, bindings);
    }

    #[test]
    fn sticks_add_to_keys_with_saturation() {
        let bindings = Bindings::new();
        // Keyboard forward plus stick right gives a diagonal.
        let fwd = bindings.button(BoundAction::MoveForward);
        let right = bindings.button(BoundAction::MoveRight);
        let pressed = [fwd];
        let input = fighter_input_from_pad(7, fighter(), &pressed, (0, 100), &bindings)
            .expect("diagonal validates");
        assert_eq!((input.mx, input.mz), (100, 100));
        assert_eq!(input.tick, 7);
        // Opposed full drives clamp instead of wrapping.
        assert_eq!(combine_wish((100, 100), (100, 100)), (100, 100));
        assert_eq!(combine_wish((-100, -100), (-100, 100)), (-100, 0));
        assert_eq!(combine_wish((100, 0), (-100, 0)), (0, 0));
        // The unused move rows never set button bits.
        let moves = [fwd, right];
        assert_eq!(bindings.button_bits(&moves), Buttons::NONE);
    }

    #[test]
    fn merge_keeps_one_state_in_fixed_order() {
        let pad = PadFrame::snapshot(0.0, 0.0, &[3, 7, 3]);
        let keyboard = [PhysicalButton::Key(9), PhysicalButton::Pad(3)];
        let mut merged = Vec::with_capacity(8);
        merge_pressed(&keyboard, &pad, &mut merged);
        // Keyboard order first, then pad index order, each code once.
        assert_eq!(
            merged,
            vec![
                PhysicalButton::Key(9),
                PhysicalButton::Pad(3),
                PhysicalButton::Pad(7),
            ]
        );
        // The same inputs merge to the same list twice.
        let mut again = Vec::with_capacity(8);
        merge_pressed(&keyboard, &pad, &mut again);
        assert_eq!(merged, again);
        // Duplicate pad codes collapse in the snapshot itself.
        assert!(pad.is_pressed(3));
        assert!(pad.is_pressed(7));
        assert!(!pad.is_pressed(4));
        assert_eq!(PadFrame::empty(), PadFrame::snapshot(0.0, 0.0, &[]));
    }

    #[test]
    fn pad_gilrs_recorded_events_give_equal_bytes() {
        use RecordedPadEvent::{Axis, ButtonDown, ButtonUp};
        let events = [
            Axis { lx: 0.5, ly: -0.25 },
            ButtonDown(0),
            ButtonDown(7),
            ButtonDown(7),
            ButtonUp(0),
            ButtonDown(3),
            Axis { lx: 1.0, ly: 0.5 },
        ];
        let first = snapshot_from_recorded(&events);
        let second = snapshot_from_recorded(&events);
        // Byte equality: axis bits plus button words match exactly.
        assert_eq!(first.lx.to_bits(), second.lx.to_bits());
        assert_eq!(first.ly.to_bits(), second.ly.to_bits());
        assert_eq!(first.buttons, second.buttons);
        assert_eq!(first, second);
        // Press-then-release clears; duplicates collapse; last axis wins.
        assert!(!first.is_pressed(0));
        assert!(first.is_pressed(3));
        assert!(first.is_pressed(7));
        assert_eq!((first.lx, first.ly), (1.0, 0.5));
        // Same tape through `snapshot` gives the same button bytes.
        let direct = PadFrame::snapshot(1.0, 0.5, &[3, 7]);
        assert_eq!(first.buttons, direct.buttons);
        // Once-per-frame drain is pure: the same recorded list folds to
        // the same bytes twice through the poll helper.
        let polled_first = poll_gilrs_once(|| events.to_vec());
        let polled_second = poll_gilrs_once(|| events.to_vec());
        assert_eq!(polled_first.lx.to_bits(), polled_second.lx.to_bits());
        assert_eq!(polled_first.ly.to_bits(), polled_second.ly.to_bits());
        assert_eq!(polled_first.buttons, polled_second.buttons);
        assert_eq!(polled_first, polled_second);
        assert_eq!(polled_first, first);
        // Empty drain rests at the empty snapshot.
        assert_eq!(poll_gilrs_once(Vec::new), PadFrame::empty());
        assert_eq!(snapshot_from_recorded(&[]), PadFrame::empty());
    }

    #[test]
    fn fps_map_fires_aims_moves_and_rebinds() {
        let bindings = fps_bindings();
        // WASD movement rows keep their defaults; only the aim rows move.
        for action in [
            BoundAction::MoveForward,
            BoundAction::MoveBack,
            BoundAction::MoveLeft,
            BoundAction::MoveRight,
        ] {
            assert_eq!(bindings.button(action), Bindings::new().button(action));
        }
        // Primary button fires, secondary aims.
        assert_eq!(
            bindings.button(BoundAction::Attack),
            PhysicalButton::Mouse(0)
        );
        assert_eq!(
            bindings.button(BoundAction::Block),
            PhysicalButton::Mouse(1)
        );
        assert_eq!(
            bindings.button_bits(&[PhysicalButton::Mouse(0)]),
            Buttons(ATTACK)
        );
        assert_eq!(
            bindings.button_bits(&[PhysicalButton::Mouse(1)]),
            Buttons(BLOCK)
        );
        // Every row stays rebindable: move fire to a pad code and the
        // mouse button stops firing through the same table.
        let mut rebound = bindings;
        rebound.rebind(BoundAction::Attack, PhysicalButton::Pad(9));
        assert_eq!(
            rebound.button_bits(&[PhysicalButton::Mouse(0)]),
            Buttons::NONE
        );
        assert_eq!(
            rebound.button_bits(&[PhysicalButton::Pad(9)]),
            Buttons(ATTACK)
        );
        // The same call builds the same table twice.
        assert_eq!(fps_bindings(), bindings);
    }

    #[test]
    fn pad_poll_merges_stick_and_buttons_deterministically() {
        let mut bindings = fps_bindings();
        bindings.rebind(BoundAction::Attack, PhysicalButton::Pad(0));
        let human = FighterId::new(0).expect("slot zero is always valid");
        let keyboard = [bindings.button(BoundAction::MoveForward)];
        let events = [
            RecordedPadEvent::Axis { lx: 1.0, ly: 0.0 },
            RecordedPadEvent::ButtonDown(0),
            RecordedPadEvent::ButtonDown(0),
            RecordedPadEvent::ButtonUp(1),
        ];
        let mut scratch = Vec::with_capacity(16);
        let first = tick_input_with_pad(0, human, &keyboard, &events, &bindings, &mut scratch)
            .expect("scripted poll input validates");
        let second = tick_input_with_pad(0, human, &keyboard, &events, &bindings, &mut scratch)
            .expect("scripted poll input validates");
        // Same events give the same input twice: bits hash into the tick.
        assert_eq!(first, second);
        first.validate().expect("polled input validates");
        // Keyboard forward plus full-right stick: a saturated diagonal.
        assert_eq!((first.mx, first.mz), (100, 100));
        // The rebound pad code fires through the poll path; the released
        // code and the duplicate press leave no trace.
        assert_eq!(first.buttons, Buttons(ATTACK));
        assert_eq!(first.tick, 0);
        // Empty frame emits exactly the idle input the headless leg
        // replays tick for tick, so the golden never moves.
        let idle = tick_input_with_pad(9, human, &[], &[], &bindings, &mut scratch)
            .expect("idle poll input validates");
        assert_eq!(
            idle,
            FighterInput {
                tick: 9,
                fighter: human,
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Buttons::NONE,
            }
        );
        // A bad fighter slot fails loudly instead of stamping a bad input.
        let bad = tick_input_with_pad(
            0,
            FighterId(99),
            &keyboard,
            &events,
            &bindings,
            &mut scratch,
        );
        assert!(bad.is_err());
    }

    #[test]
    fn pad_poll_reuses_scratch_without_growing() {
        // Calibration: the shared probe must see a real allocation first.
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "allocation probe is blind");

        let bindings = fps_bindings();
        let human = FighterId::new(0).expect("slot zero is always valid");
        let keyboard = [bindings.button(BoundAction::MoveForward)];
        let events = [RecordedPadEvent::Axis { lx: 0.5, ly: -0.25 }];
        let mut scratch = Vec::with_capacity(16);
        // Warmup sizes the buffer; the measured pass must not grow it.
        tick_input_with_pad(0, human, &keyboard, &events, &bindings, &mut scratch)
            .expect("warmup poll input validates");
        let (input, steady) = alloc_probe::measure(|| {
            tick_input_with_pad(0, human, &keyboard, &events, &bindings, &mut scratch)
        });
        assert!(input.is_ok());
        assert_eq!(
            steady.calls, 0,
            "pad poll plus build must not allocate per tick"
        );
    }

    #[test]
    fn bad_fighter_is_refused_loudly() {
        let bindings = Bindings::new();
        let bad = FighterId(99);
        assert_eq!(
            fighter_input_from_pad(0, bad, &[], (0, 0), &bindings),
            Err(FighterError::UnknownFighter(99))
        );
    }

    #[test]
    fn pad_driven_session_advances_and_replays() {
        let config = crate::GameConfig {
            seed: 41,
            ticks: 64,
            fighters: 2,
            half_extent_m: 12.0,
        };
        let run_once = || {
            let mut app = crate::App::new(config).expect("test config validates");
            let mut bindings = Bindings::new();
            bindings.rebind(BoundAction::Attack, PhysicalButton::Pad(0));
            let mut pressed = Vec::with_capacity(8);
            let mut outcomes = Vec::with_capacity(64);
            for tick in 0..64u32 {
                // Circle on the stick, pad attack every fifth tick.
                let phase = tick % 4;
                let (lx, ly) = match phase {
                    0 => (1.0, 0.0),
                    1 => (0.0, 1.0),
                    2 => (-1.0, 0.0),
                    _ => (0.0, -1.0),
                };
                let pad = if tick % 5 == 0 {
                    PadFrame::snapshot(lx, ly, &[0])
                } else {
                    PadFrame::snapshot(lx, ly, &[])
                };
                merge_pressed(&[], &pad, &mut pressed);
                let stick = pad.stick_wish(bindings.deadzone());
                let input = fighter_input_from_pad(tick, fighter(), &pressed, stick, &bindings)
                    .expect("scripted pad input validates");
                outcomes.push(app.tick(input));
            }
            (app.checkpoints().to_vec(), outcomes)
        };
        let (first_checks, first_outcomes) = run_once();
        let (second_checks, second_outcomes) = run_once();
        assert_eq!(first_checks.len(), 64);
        assert_eq!(first_checks, second_checks);
        assert_eq!(first_outcomes, second_outcomes);
        assert!(first_outcomes.iter().all(|outcome| outcome.input_accepted));
        // The scripted bout moves rather than idling on one hash.
        let mut hashes: Vec<u64> = first_checks.iter().map(|check| check.hash).collect();
        hashes.sort_unstable();
        hashes.dedup();
        assert!(hashes.len() > 1, "pad bout never moved");
    }

    #[test]
    fn merged_path_allocates_nothing_at_steady_state() {
        // Calibration: the shared probe must see a real allocation first.
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "allocation probe is blind");

        let bindings = Bindings::new();
        let pad = PadFrame::snapshot(0.5, -0.25, &[1, 2]);
        let keyboard = [PhysicalButton::Key(3)];
        let mut pressed = Vec::with_capacity(16);
        merge_pressed(&keyboard, &pad, &mut pressed);
        let stick = pad.stick_wish(bindings.deadzone());
        // Warmup sizes the buffer; the measured pass must not grow it.
        let (input, steady) = alloc_probe::measure(|| {
            merge_pressed(&keyboard, &pad, &mut pressed);
            fighter_input_from_pad(0, fighter(), &pressed, stick, &bindings)
        });
        assert!(input.is_ok());
        assert_eq!(
            steady.calls, 0,
            "pad merge plus build must not allocate per tick"
        );
    }

    /// The seven cue rows, one cue each, with the row it must reach.
    fn cue_rows() -> [(FightCue, usize); 7] {
        use crate::fps_view::pad_rumble::{
            BLOCKED_ROW, DODGED_ROW, HEAVY_HIT_ROW, KNOCKDOWN_ROW, LIGHT_HIT_ROW, PARRIED_ROW,
            STAMINA_OUT_ROW,
        };
        [
            (
                FightCue::Hit {
                    attacker: 1,
                    victim: 0,
                    damage: 1,
                },
                LIGHT_HIT_ROW,
            ),
            (
                FightCue::Hit {
                    attacker: 1,
                    victim: 0,
                    damage: u16::MAX,
                },
                HEAVY_HIT_ROW,
            ),
            (
                FightCue::Blocked {
                    attacker: 1,
                    victim: 0,
                    chip: 20,
                },
                BLOCKED_ROW,
            ),
            (
                FightCue::Parried {
                    attacker: 1,
                    victim: 0,
                },
                PARRIED_ROW,
            ),
            (
                FightCue::Dodged {
                    attacker: 1,
                    victim: 0,
                },
                DODGED_ROW,
            ),
            (FightCue::StaminaOut { fighter: 0 }, STAMINA_OUT_ROW),
            (
                FightCue::Knockdown {
                    attacker: 1,
                    victim: 0,
                },
                KNOCKDOWN_ROW,
            ),
        ]
    }

    #[test]
    fn pad_state_focus_loss_cuts_motors_within_ten_ms() {
        use crate::fps_view::pad_rumble::KNOCKDOWN_ROW;
        let mut state = PadState::new();
        let played = state.step(
            PadEvent::Cue(FightCue::Knockdown {
                attacker: 1,
                victim: 0,
            }),
            1_000,
        );
        assert_eq!(
            played.motors,
            crate::fps_view::pad_rumble::RUMBLE_TABLE[KNOCKDOWN_ROW]
        );
        assert_eq!(played.motors_off_at_ms, Some(1_260), "the row's own length");
        let cut = state.step(PadEvent::FocusLost, 1_100);
        let motors = cut.motors;
        assert_eq!(
            (motors.low, motors.high, motors.trigger_l, motors.trigger_r),
            (0, 0, 0, 0),
            "focus loss leaves no motor on"
        );
        let off_at = cut.motors_off_at_ms.expect("focus loss asks for a cut");
        assert!(
            off_at <= 1_100 + MOTORS_OFF_MS,
            "motors off at {off_at} ms, deadline {} ms",
            1_100 + MOTORS_OFF_MS
        );
        assert!(!state.focused());
        // A cue while the window is away stays silent.
        let away = state.step(
            PadEvent::Cue(FightCue::Blocked {
                attacker: 1,
                victim: 0,
                chip: 1,
            }),
            1_200,
        );
        assert_eq!(away.motors, RumbleRow::default());
        assert_eq!(away.motors_off_at_ms, None);
        // Back again: the next cue plays.
        state.step(PadEvent::FocusGained, 1_300);
        let back = state.step(
            PadEvent::Cue(FightCue::Blocked {
                attacker: 1,
                victim: 0,
                chip: 1,
            }),
            1_400,
        );
        assert!(back.motors.low > 0, "a focused pad rumbles again");
    }

    #[test]
    fn pad_state_disconnect_pauses_the_duel_within_one_tick() {
        let mut state = PadState::new();
        state.step(PadEvent::PadPress, 0);
        assert!(!state.paused());
        let gone = state.step(PadEvent::PadDisconnected, 16);
        assert!(gone.paused, "the disconnect pauses at once");
        assert_eq!(
            gone.motors_off_at_ms,
            Some(16),
            "motors cut on the same tick"
        );
        assert!(state.paused());
        // Reconnecting resumes, and a pad already acted, so pad glyphs stay.
        let back = state.step(PadEvent::PadConnected, 32);
        assert!(!back.paused);
        assert_eq!(back.glyphs, GlyphSet::Pad);
    }

    #[test]
    fn pad_state_glyphs_follow_the_last_device() {
        let mut state = PadState::new();
        assert_eq!(state.glyphs(), GlyphSet::Keyboard, "the keyboard starts");
        assert_eq!(state.step(PadEvent::PadPress, 0).glyphs, GlyphSet::Pad);
        assert_eq!(
            state.step(PadEvent::KeyPress, 16).glyphs,
            GlyphSet::Keyboard,
            "a key press flips the prompts back"
        );
        assert_eq!(state.step(PadEvent::PadPress, 32).glyphs, GlyphSet::Pad);
        assert_eq!(DeviceKind::Pad.glyphs(), GlyphSet::Pad);
        assert_eq!(DeviceKind::Keyboard.glyphs(), GlyphSet::Keyboard);
    }

    #[test]
    fn pad_state_seven_cue_rows_reach_the_motors_unchanged() {
        use crate::fps_view::pad_rumble::RUMBLE_TABLE;
        let mut state = PadState::new();
        for (cue, row) in cue_rows() {
            let step = state.step(PadEvent::Cue(cue), 500);
            assert_eq!(step.motors, RUMBLE_TABLE[row], "{cue:?} plays its own row");
            let want_deadline = if RUMBLE_TABLE[row].ms == 0 {
                None
            } else {
                Some(500 + RUMBLE_TABLE[row].ms)
            };
            assert_eq!(step.motors_off_at_ms, want_deadline, "{cue:?} deadline");
        }
    }
}
