//! bindings: rebindable physical-button table mapping to [`FighterInput`].
//!
//! Consumer: the game/app layer polling devices each fixed tick and recording
//! the resulting [`FighterInput`] into a [`FighterTape`](super::fighter::FighterTape).
//! Data owner: [`Bindings`] owns one button per [`BoundAction`]; [`InputConfig`]
//! is the plain-fields save form the config layer persists (no serde here).
//! Units: wishes are quantized `-100..=100` percents of top speed, facing is
//! centidegrees `-18000..=18000`, ticks are zero-based fixed steps — the same
//! contract as [`FighterInput`]. Camera-relative wishes are quantized by the
//! CALLER into `mx`/`mz` before calling [`Bindings::to_fighter_input`]; this
//! module never touches a render camera, so replays stay camera-independent.
//! Determinism: every query is a pure function over fixed [`BoundAction::ALL`]
//! order; `pressed` slice order never affects the output. Errors reuse
//! [`FighterError`](super::fighter::FighterError); invalid wishes/facing fail
//! instead of clamping so bad callers are loud.

use super::fighter::{Buttons, FighterError, FighterId, FighterInput, ATTACK, BLOCK, DODGE};
use super::map::Key;

/// Count of rebindable actions (see [`BoundAction::ALL`]).
pub const ACTION_COUNT: usize = 7;
/// Default deadzone in wish units: wishes this small snap to zero.
pub const DEFAULT_DEADZONE: u8 = 8;
/// [`PhysicalButton`] kind code for keyboard keys.
pub const KIND_KEY: u8 = 0;
/// [`PhysicalButton`] kind code for mouse buttons.
pub const KIND_MOUSE: u8 = 1;
/// [`PhysicalButton`] kind code for gamepad buttons.
pub const KIND_PAD: u8 = 2;

/// Physical button as data codes (no device handles, no gilrs).
///
/// Codes are opaque `u8` rows: keyboard codes address [`Key`] storage
/// (`Key(code)`), mouse/pad codes are device button indices. Unknown codes
/// are ignored by [`Bindings`] lookups rather than rejected, so a focused
/// window keeps working when a device reports buttons nothing is bound to.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PhysicalButton {
    /// Keyboard key code.
    Key(u8),
    /// Mouse button index.
    Mouse(u8),
    /// Gamepad button index (data only; live gamepad polling is a later lane).
    Pad(u8),
}

impl PhysicalButton {
    /// Kind row for config save/restore (`KIND_KEY`/`KIND_MOUSE`/`KIND_PAD`).
    pub const fn kind(self) -> u8 {
        match self {
            Self::Key(_) => KIND_KEY,
            Self::Mouse(_) => KIND_MOUSE,
            Self::Pad(_) => KIND_PAD,
        }
    }

    /// Raw code within the kind row.
    pub const fn code(self) -> u8 {
        match self {
            Self::Key(c) | Self::Mouse(c) | Self::Pad(c) => c,
        }
    }

    /// Rebuild from a saved `(kind, code)` pair; `None` on unknown kinds so
    /// corrupt config rows fall back to defaults instead of panicking.
    pub const fn from_parts(kind: u8, code: u8) -> Option<Self> {
        match kind {
            KIND_KEY => Some(Self::Key(code)),
            KIND_MOUSE => Some(Self::Mouse(code)),
            KIND_PAD => Some(Self::Pad(code)),
            _ => None,
        }
    }
}

/// Rebindable action: three buttons plus four wish-axis directions.
///
/// The move variants name wish space (`mx` forward, `mz` right); the caller
/// maps them to camera-relative keys before quantizing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BoundAction {
    /// Attack button (sets `ATTACK`).
    Attack,
    /// Block button (sets `BLOCK`).
    Block,
    /// Dodge button (sets `DODGE`).
    Dodge,
    /// Forward wish (`mx = +100`).
    MoveForward,
    /// Back wish (`mx = -100`).
    MoveBack,
    /// Left wish (`mz = -100`).
    MoveLeft,
    /// Right wish (`mz = +100`).
    MoveRight,
}

impl BoundAction {
    /// Every action in fixed table order (also the config field order).
    pub const ALL: [Self; ACTION_COUNT] = [
        Self::Attack,
        Self::Block,
        Self::Dodge,
        Self::MoveForward,
        Self::MoveBack,
        Self::MoveLeft,
        Self::MoveRight,
    ];

    /// Table index of this action.
    pub const fn index(self) -> usize {
        match self {
            Self::Attack => 0,
            Self::Block => 1,
            Self::Dodge => 2,
            Self::MoveForward => 3,
            Self::MoveBack => 4,
            Self::MoveLeft => 5,
            Self::MoveRight => 6,
        }
    }

    /// Action for a table index; `None` when out of range.
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Attack),
            1 => Some(Self::Block),
            2 => Some(Self::Dodge),
            3 => Some(Self::MoveForward),
            4 => Some(Self::MoveBack),
            5 => Some(Self::MoveLeft),
            6 => Some(Self::MoveRight),
            _ => None,
        }
    }

    /// Button bit this action sets, if it is a button action at all.
    /// Move variants return `None`: they feed wishes, not bits.
    pub const fn button_bit(self) -> Option<u8> {
        match self {
            Self::Attack => Some(ATTACK),
            Self::Block => Some(BLOCK),
            Self::Dodge => Some(DODGE),
            Self::MoveForward | Self::MoveBack | Self::MoveLeft | Self::MoveRight => None,
        }
    }
}

/// Action set: which layer owns the buttons this tick.
///
/// Duel carries the fight; Menu and Pause carry UI. Only the top of the
/// [`ActionSetStack`] is active, so menu keys never reach the duel tape.
/// Units: unitless enum. Determinism: fixed order in [`ActionSet::ALL`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ActionSet {
    /// Fight layer; the only set that writes duel button bits.
    Duel,
    /// Menu layer; duel evaluation is gated to zero while active.
    Menu,
    /// Pause layer; duel evaluation is gated to zero while active.
    Pause,
}

impl ActionSet {
    /// Every set in fixed stack order (also the index order).
    pub const ALL: [Self; 3] = [Self::Duel, Self::Menu, Self::Pause];

    /// Table index of this set.
    pub const fn index(self) -> usize {
        match self {
            Self::Duel => 0,
            Self::Menu => 1,
            Self::Pause => 2,
        }
    }

    /// The digest byte of this set: its table index as a byte.
    ///
    /// A named conversion, never a numeric cast: the layer digest folds one
    /// byte per layer, and the table has three entries, so the byte is exact.
    pub const fn byte(self) -> u8 {
        match self {
            Self::Duel => 0,
            Self::Menu => 1,
            Self::Pause => 2,
        }
    }

    /// Set for a table index; `None` when out of range.
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Duel),
            1 => Some(Self::Menu),
            2 => Some(Self::Pause),
            _ => None,
        }
    }
}

/// Stack of action sets; the top is active.
///
/// Push Menu over Duel to suspend the fight; pop to resume. Fixed capacity
/// 3, no heap, no clock. No live consumer yet: only in-crate tests drive
/// [`Bindings::button_bits_for_set`]; v2 only, the v1 [`Bindings::button_bits`]
/// path never reads the stack.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ActionSetStack {
    slots: [Option<ActionSet>; 3],
    len: usize,
}

impl ActionSetStack {
    /// Blank stack with Duel active.
    pub fn new() -> Self {
        Self {
            slots: [Some(ActionSet::ALL[0]), None, None],
            len: 1,
        }
    }

    /// Active set; `None` only when the stack was fully popped.
    pub fn active(self) -> Option<ActionSet> {
        if self.len == 0 {
            None
        } else {
            self.slots[self.len - 1]
        }
    }

    /// Count of pushed sets.
    pub fn len(self) -> usize {
        self.len
    }

    /// True when no set is pushed.
    pub fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Push a set; returns false and keeps the stack when full.
    pub fn push(&mut self, set: ActionSet) -> bool {
        if self.len >= 3 {
            return false;
        }
        self.slots[self.len] = Some(set);
        self.len += 1;
        true
    }

    /// Pop the top; `None` when empty.
    pub fn pop(&mut self) -> Option<ActionSet> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        let out = self.slots[self.len];
        self.slots[self.len] = None;
        out
    }

    /// True when the active set is Duel, so fight input flows.
    pub fn is_duel_active(self) -> bool {
        matches!(self.active(), Some(ActionSet::Duel))
    }
}

impl Default for ActionSetStack {
    /// Blank stack with Duel active (see [`ActionSetStack::new`]).
    fn default() -> Self {
        Self::new()
    }
}

/// One table row: which button drives an action.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Binding {
    /// Action this row drives.
    pub action: BoundAction,
    /// Button bound to it.
    pub button: PhysicalButton,
}

/// Rebindable button table plus deadzone.
///
/// One button per action; [`Bindings::rebind`] overwrites the row. Binding
/// two actions to the same button is allowed and deterministic (both fire);
/// callers wanting exclusivity rebind the other row first.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Bindings {
    buttons: [PhysicalButton; ACTION_COUNT],
    deadzone: u8,
}

impl Bindings {
    /// Default table: F attack, Shift block, Space dodge, WASD move.
    pub fn new() -> Self {
        Self {
            buttons: [
                PhysicalButton::Key(Key::F.0),
                PhysicalButton::Key(Key::SHIFT.0),
                PhysicalButton::Key(Key::SPACE.0),
                PhysicalButton::Key(Key::W.0),
                PhysicalButton::Key(Key::S.0),
                PhysicalButton::Key(Key::A.0),
                PhysicalButton::Key(Key::D.0),
            ],
            deadzone: DEFAULT_DEADZONE,
        }
    }

    /// Row for one action.
    pub fn binding(self, action: BoundAction) -> Binding {
        Binding {
            action,
            button: self.buttons[action.index()],
        }
    }

    /// Button currently bound to an action.
    pub fn button(self, action: BoundAction) -> PhysicalButton {
        self.buttons[action.index()]
    }

    /// Rebind one action to a new button, overwriting the old row.
    pub fn rebind(&mut self, action: BoundAction, button: PhysicalButton) {
        self.buttons[action.index()] = button;
    }

    /// Current deadzone in wish units.
    pub fn deadzone(self) -> u8 {
        self.deadzone
    }

    /// Set the deadzone applied to wishes in [`Bindings::to_fighter_input`].
    pub fn set_deadzone(&mut self, deadzone: u8) {
        self.deadzone = deadzone;
    }

    /// Button bits for the currently pressed buttons. Pressed buttons with
    /// no binding (unknown codes, move keys) contribute nothing.
    pub fn button_bits(self, pressed: &[PhysicalButton]) -> Buttons {
        let mut bits: u8 = 0;
        for action in BoundAction::ALL {
            if let Some(bit) = action.button_bit() {
                let bound = self.buttons[action.index()];
                if pressed.contains(&bound) {
                    bits |= bit;
                }
            }
        }
        Buttons(bits)
    }

    /// Full-deflection wish from the pressed movement buttons: forward
    /// `+100`, back `-100` on `mx`; right `+100`, left `-100` on `mz`.
    /// Opposed pairs cancel to zero. Callers quantize camera-relative axes
    /// with this or their own device math, then pass the result on.
    pub fn wish_from_pressed(self, pressed: &[PhysicalButton]) -> (i8, i8) {
        let fwd = pressed.contains(&self.buttons[BoundAction::MoveForward.index()]);
        let back = pressed.contains(&self.buttons[BoundAction::MoveBack.index()]);
        let left = pressed.contains(&self.buttons[BoundAction::MoveLeft.index()]);
        let right = pressed.contains(&self.buttons[BoundAction::MoveRight.index()]);
        let mx = i8::from(fwd) * 100 - i8::from(back) * 100;
        let mz = i8::from(right) * 100 - i8::from(left) * 100;
        (mx, mz)
    }

    /// Build one validated [`FighterInput`]: button bits from `pressed`,
    /// caller-quantized `wish` through this table's deadzone, `facing`
    /// passed through. Out-of-range wishes/facing fail with [`FighterError`].
    pub fn to_fighter_input(
        self,
        tick: u32,
        fighter: FighterId,
        pressed: &[PhysicalButton],
        wish: (i8, i8),
        facing: i16,
    ) -> Result<FighterInput, FighterError> {
        let (mx, mz) = apply_deadzone(wish.0, wish.1, self.deadzone);
        let input = FighterInput {
            tick,
            fighter,
            mx,
            mz,
            facing,
            buttons: self.button_bits(pressed),
        };
        input.validate()?;
        Ok(input)
    }
}

impl Default for Bindings {
    /// Default table (see [`Bindings::new`]).
    fn default() -> Self {
        Self::new()
    }
}

/// Snap wishes with small magnitude to zero: when `mx^2 + mz^2 <= deadzone^2`
/// the stick/keyboard noise is treated as rest. Integer math only, exact.
pub fn apply_deadzone(mx: i8, mz: i8, deadzone: u8) -> (i8, i8) {
    let magnitude2 = i32::from(mx) * i32::from(mx) + i32::from(mz) * i32::from(mz);
    let radius2 = i32::from(deadzone) * i32::from(deadzone);
    if magnitude2 <= radius2 {
        (0, 0)
    } else {
        (mx, mz)
    }
}

/// Tick-counted gesture conditions before the tape, v2 only.
///
/// Shape reimplemented from the `bevy_enhanced_input` condition shape
/// (tap plus hold-and-release) and the `daInput` action-set shape: a
/// condition is a small state machine over `(pressed, ticks held)` counted
/// in integer fixed ticks. Consumer: [`Bindings::gesture_bits_for_tick`]
/// feeds one `pressed` bool per fixed tick and ORs the emissions into
/// [`GestureBits`](conditions::GestureBits); the game layer records the
/// resulting [`FighterInput`] into a tape. Data owner:
/// [`ConditionTracker`](conditions::ConditionTracker) owns its tick
/// counters. Units: ticks (integer fixed steps). Determinism: pure tick
/// function, no alloc, no clock, same script gives same emissions. Errors:
/// never fails; out-of-window presses yield `None`. V2 only:
/// [`GestureBits`](conditions::GestureBits) never enter [`FighterInput`]
/// bytes, so v1 tapes stay byte-equal and the golden hash never moves.
pub mod conditions {
    /// Minimum held ticks for a tap (shorter is noise, ignored).
    pub const TAP_MIN_HELD_TICKS: u32 = 2;
    /// Maximum held ticks for a tap (longer is a hold candidate or dead zone).
    pub const TAP_MAX_HELD_TICKS: u32 = 12;
    /// Minimum held ticks for a hold, emitted while still held.
    pub const HOLD_MIN_HELD_TICKS: u32 = 30;
    /// Minimum held ticks for a hold-and-release, emitted on release.
    pub const HOLD_RELEASE_MIN_HELD_TICKS: u32 = 30;

    const _: () = assert!(
        TAP_MIN_HELD_TICKS <= TAP_MAX_HELD_TICKS
            && TAP_MAX_HELD_TICKS < HOLD_MIN_HELD_TICKS
            && TAP_MIN_HELD_TICKS > 0
    );

    /// Tick-window gesture emitted by [`ConditionTracker::tick`].
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
    pub enum GestureKind {
        /// Short press released inside the tap window.
        Tap,
        /// Long press reaching the hold threshold while still held.
        Hold,
        /// Long press released after the hold threshold.
        HoldAndRelease,
        /// Two buttons held on the same tick.
        Chord,
    }

    /// V2-only gesture bitset; never enters [`FighterInput`](super::fighter::FighterInput) bytes.
    ///
    /// The tape keeps the v1 button bits only, so v1 bytes stay equal.
    /// Units: bit per [`GestureKind`] (see [`gesture_bit`]). Determinism:
    /// pure OR in fixed order (tracker emission, then chord).
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    pub struct GestureBits(pub u8);

    impl GestureBits {
        /// Empty bitset.
        pub const NONE: Self = Self(0);
    }

    /// Tick-counted tracker for tap, hold and hold-and-release.
    ///
    /// Feed one `pressed` bool per fixed tick with [`ConditionTracker::tick`];
    /// it returns one emission at most per tick. Budget: O(1) per tick, no heap.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct ConditionTracker {
        held: u32,
        hold_fired: bool,
    }

    impl ConditionTracker {
        /// Blank tracker: nothing held, no hold fired.
        pub fn new() -> Self {
            Self {
                held: 0,
                hold_fired: false,
            }
        }

        /// Clear counters to the blank state.
        pub fn reset(&mut self) {
            self.held = 0;
            self.hold_fired = false;
        }

        /// Advance one fixed tick; `pressed` is held this tick.
        ///
        /// Returns [`GestureKind::Hold`] on the tick held reaches
        /// [`HOLD_MIN_HELD_TICKS`], [`GestureKind::HoldAndRelease`] on release
        /// after that threshold, [`GestureKind::Tap`] on release inside
        /// `TAP_MIN..=TAP_MAX`. Noise and dead-zone presses return `None`.
        pub fn tick(&mut self, pressed: bool) -> Option<GestureKind> {
            if pressed {
                self.held = self.held.saturating_add(1);
                if self.held == HOLD_MIN_HELD_TICKS && !self.hold_fired {
                    self.hold_fired = true;
                    return Some(GestureKind::Hold);
                }
                return None;
            }
            if self.held > 0 {
                let held = self.held;
                self.held = 0;
                if self.hold_fired {
                    self.hold_fired = false;
                    if held >= HOLD_RELEASE_MIN_HELD_TICKS {
                        return Some(GestureKind::HoldAndRelease);
                    }
                    return None;
                }
                if (TAP_MIN_HELD_TICKS..=TAP_MAX_HELD_TICKS).contains(&held) {
                    return Some(GestureKind::Tap);
                }
                return None;
            }
            None
        }
    }

    impl Default for ConditionTracker {
        /// Blank tracker (see [`ConditionTracker::new`]).
        fn default() -> Self {
            Self::new()
        }
    }

    /// All-of chord: true only when both sides are held this same tick.
    ///
    /// Units: bools. Determinism: pure AND. Errors: never fails.
    pub fn chord_fires(first_held: bool, second_held: bool) -> bool {
        first_held && second_held
    }

    /// Bit for one gesture kind in [`GestureBits`].
    ///
    /// Units: `u8` bit. Determinism: fixed match. Errors: never fails.
    pub const fn gesture_bit(kind: GestureKind) -> u8 {
        match kind {
            GestureKind::Tap => 0x01,
            GestureKind::Hold => 0x02,
            GestureKind::HoldAndRelease => 0x04,
            GestureKind::Chord => 0x08,
        }
    }

    /// Gesture bits for one fixed tick from a tracker plus a chord pair.
    ///
    /// Calls [`ConditionTracker::tick`] for tap/hold/hold-and-release and
    /// [`chord_fires`] for the chord, ORed in fixed order. Units: ticks.
    /// Determinism: same inputs give same bits. Errors: never fails.
    /// Budget: O(1), no heap.
    pub fn bits_for_tick(
        tracker: &mut ConditionTracker,
        pressed: bool,
        chord_first: bool,
        chord_second: bool,
    ) -> GestureBits {
        let mut bits: u8 = 0;
        if let Some(kind) = tracker.tick(pressed) {
            bits |= gesture_bit(kind);
        }
        if chord_fires(chord_first, chord_second) {
            bits |= gesture_bit(GestureKind::Chord);
        }
        GestureBits(bits)
    }

    #[cfg(test)]
    mod tests {
        use super::super::super::bindings::{
            ActionSet, ActionSetStack, Bindings, BoundAction, ACTION_COUNT,
        };
        use super::super::super::fighter::{ATTACK, BLOCK, DODGE, KNOWN_BUTTONS};
        use super::*;

        fn run_hold(held_ticks: u32) -> Vec<GestureKind> {
            let mut tracker = ConditionTracker::new();
            let mut out = Vec::new();
            for _ in 0..held_ticks {
                if let Some(kind) = tracker.tick(true) {
                    out.push(kind);
                }
            }
            if let Some(kind) = tracker.tick(false) {
                out.push(kind);
            }
            out
        }

        #[test]
        fn conditions_tap_fires_within_window_and_ignores_noise() {
            assert_eq!(run_hold(2), vec![GestureKind::Tap]);
            assert_eq!(run_hold(5), vec![GestureKind::Tap]);
            assert_eq!(run_hold(12), vec![GestureKind::Tap]);
            assert!(run_hold(1).is_empty(), "1-tick blip is noise");
            assert!(run_hold(13).is_empty(), "13 ticks is past the tap window");
            assert!(run_hold(15).is_empty(), "dead zone emits nothing");
        }

        #[test]
        fn conditions_hold_fires_once_at_threshold_while_held() {
            let mut tracker = ConditionTracker::new();
            let mut hits: Vec<u32> = Vec::new();
            for tick in 0..35u32 {
                if tracker.tick(true) == Some(GestureKind::Hold) {
                    hits.push(tick);
                }
            }
            assert_eq!(hits.len(), 1, "hold fires exactly once, got {hits:?}");
            assert_eq!(hits[0], 29, "30th held tick is index 29");
            let mut short = ConditionTracker::new();
            for _ in 0..29 {
                assert_ne!(
                    short.tick(true),
                    Some(GestureKind::Hold),
                    "29 held ticks must not hold yet"
                );
            }
        }

        #[test]
        fn conditions_hold_and_release_fires_on_release_after_hold() {
            assert_eq!(
                run_hold(30).len(),
                2,
                "hold then release gives two emissions"
            );
            assert_eq!(run_hold(30)[1], GestureKind::HoldAndRelease);
            assert_eq!(run_hold(35)[1], GestureKind::HoldAndRelease);
            assert_ne!(
                run_hold(5).first(),
                Some(&GestureKind::HoldAndRelease),
                "short tap is never a hold-release"
            );
            assert!(
                run_hold(29)
                    .iter()
                    .all(|k| *k != GestureKind::HoldAndRelease),
                "29 ticks must not hold-release"
            );
            let mut tracker = ConditionTracker::new();
            for _ in 0..30 {
                tracker.tick(true);
            }
            tracker.reset();
            assert_eq!(tracker.tick(false), None, "reset clears the hold");
        }

        #[test]
        fn conditions_chord_needs_both_buttons_same_tick() {
            assert!(chord_fires(true, true));
            assert!(!chord_fires(true, false));
            assert!(!chord_fires(false, true));
            assert!(!chord_fires(false, false));
            let mut tracker = ConditionTracker::new();
            let both = bits_for_tick(&mut tracker, false, true, true);
            assert_eq!(both, GestureBits(gesture_bit(GestureKind::Chord)));
            let mut single = ConditionTracker::new();
            let one = bits_for_tick(&mut single, false, true, false);
            assert_eq!(one, GestureBits::NONE, "single side is no chord");
        }

        #[test]
        fn conditions_menu_presses_never_reach_duel() {
            let bindings = Bindings::new();
            let attack = bindings.button(BoundAction::Attack);
            let pressed = [attack];
            let duel_bits = bindings.button_bits_for_set(&pressed, ActionSet::Duel);
            assert_ne!(duel_bits.0, 0, "duel must see the attack");
            let menu_bits = bindings.button_bits_for_set(&pressed, ActionSet::Menu);
            let pause_bits = bindings.button_bits_for_set(&pressed, ActionSet::Pause);
            assert_eq!(menu_bits, super::super::super::fighter::Buttons::NONE);
            assert_eq!(pause_bits, super::super::super::fighter::Buttons::NONE);
            let mut menu_tracker = ConditionTracker::new();
            let menu_gestures = Bindings::gesture_bits_for_tick(
                &mut menu_tracker,
                true,
                true,
                true,
                ActionSet::Menu,
            );
            assert_eq!(menu_gestures, GestureBits::NONE, "menu leaks 0 gestures");
            let mut stack = ActionSetStack::new();
            assert!(stack.is_duel_active());
            assert!(stack.push(ActionSet::Menu));
            assert!(!stack.is_duel_active());
            assert_eq!(stack.active(), Some(ActionSet::Menu));
            assert_eq!(stack.pop(), Some(ActionSet::Menu));
            assert!(stack.is_duel_active());
            let menu_leaks = i32::from(!(menu_bits.0 == 0 && menu_gestures.0 == 0));
            assert_eq!(menu_leaks, 0, "0 menu presses leak into the duel");
        }

        #[test]
        fn conditions_same_script_gives_same_tape_bytes_x2() {
            assert_eq!(ACTION_COUNT, 7, "7 rebindable actions stay");
            assert_eq!(KNOWN_BUTTONS, ATTACK | BLOCK | DODGE);
            assert_eq!(KNOWN_BUTTONS, 0x07, "v1 keeps 3 bits");
            let bindings = Bindings::new();
            let attack = bindings.button(BoundAction::Attack);
            let run = || {
                let mut bytes = Vec::new();
                let fighter = super::super::super::fighter::FighterId::new(0).unwrap();
                let script: Vec<Vec<super::super::PhysicalButton>> =
                    vec![vec![attack], vec![attack], vec![], vec![attack], vec![]];
                for (i, pressed) in script.iter().enumerate() {
                    let tick = u32::try_from(i).unwrap();
                    let input = bindings
                        .to_fighter_input(tick, fighter, pressed, (0, 0), 0)
                        .unwrap();
                    input.append_bytes(&mut bytes);
                }
                bytes
            };
            let first = run();
            let second = run();
            assert_eq!(first, second, "same script gives same tape bytes x2");
            assert_eq!(first.len(), 5 * 10, "v1 record stays 10 bytes");
            let duel = bindings.button_bits_for_set(&[attack], ActionSet::Duel);
            assert_eq!(duel, bindings.button_bits(&[attack]));
        }
    }
}

impl Bindings {
    /// Button bits gated by the active set: Duel flows, Menu/Pause give none.
    ///
    /// V2 only; the v1 [`Bindings::button_bits`] path is untouched. No live
    /// consumer yet: only in-crate tests pass [`ActionSetStack::active`].
    /// Determinism: pure over fixed [`BoundAction::ALL`] order. Errors: never fails.
    pub fn button_bits_for_set(self, pressed: &[PhysicalButton], set: ActionSet) -> Buttons {
        if set == ActionSet::Duel {
            self.button_bits(pressed)
        } else {
            Buttons::NONE
        }
    }

    /// Gesture bits for one fixed tick, gated by the active set.
    ///
    /// Feeds `pressed` plus the chord pair into
    /// [`conditions::bits_for_tick`] when Duel is active, else
    /// [`conditions::GestureBits::NONE`]. V2 only, never in tape bytes.
    /// No live consumer yet: only in-crate tests drive this per tick.
    /// Determinism: same inputs give same bits. Errors: never fails.
    /// Budget: O(1), no heap.
    pub fn gesture_bits_for_tick(
        tracker: &mut conditions::ConditionTracker,
        pressed: bool,
        chord_first: bool,
        chord_second: bool,
        set: ActionSet,
    ) -> conditions::GestureBits {
        if set != ActionSet::Duel {
            return conditions::GestureBits::NONE;
        }
        conditions::bits_for_tick(tracker, pressed, chord_first, chord_second)
    }
}

/// Save form of [`Bindings`]: plain `u8` fields only, no serde dependency.
///
/// The config layer persists these fields however it stores numbers; unknown
/// kind codes load as the default button for that action so a corrupt row
/// can never brick input.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct InputConfig {
    /// Kind row of the attack binding (see `KIND_*`).
    pub attack_kind: u8,
    /// Code of the attack binding.
    pub attack_code: u8,
    /// Kind row of the block binding.
    pub block_kind: u8,
    /// Code of the block binding.
    pub block_code: u8,
    /// Kind row of the dodge binding.
    pub dodge_kind: u8,
    /// Code of the dodge binding.
    pub dodge_code: u8,
    /// Kind row of the forward-move binding.
    pub forward_kind: u8,
    /// Code of the forward-move binding.
    pub forward_code: u8,
    /// Kind row of the back-move binding.
    pub back_kind: u8,
    /// Code of the back-move binding.
    pub back_code: u8,
    /// Kind row of the left-move binding.
    pub left_kind: u8,
    /// Code of the left-move binding.
    pub left_code: u8,
    /// Kind row of the right-move binding.
    pub right_kind: u8,
    /// Code of the right-move binding.
    pub right_code: u8,
    /// Deadzone in wish units.
    pub deadzone: u8,
}

impl InputConfig {
    /// Capture a table into its save form.
    pub fn from_bindings(bindings: &Bindings) -> Self {
        let part = |action: BoundAction| {
            let button = bindings.button(action);
            (button.kind(), button.code())
        };
        let (attack_kind, attack_code) = part(BoundAction::Attack);
        let (block_kind, block_code) = part(BoundAction::Block);
        let (dodge_kind, dodge_code) = part(BoundAction::Dodge);
        let (forward_kind, forward_code) = part(BoundAction::MoveForward);
        let (back_kind, back_code) = part(BoundAction::MoveBack);
        let (left_kind, left_code) = part(BoundAction::MoveLeft);
        let (right_kind, right_code) = part(BoundAction::MoveRight);
        Self {
            attack_kind,
            attack_code,
            block_kind,
            block_code,
            dodge_kind,
            dodge_code,
            forward_kind,
            forward_code,
            back_kind,
            back_code,
            left_kind,
            left_code,
            right_kind,
            right_code,
            deadzone: bindings.deadzone(),
        }
    }

    /// Restore a table; unknown kind rows fall back to the default button.
    pub fn to_bindings(self) -> Bindings {
        let defaults = Bindings::new();
        let pick = |kind: u8, code: u8, action: BoundAction| {
            PhysicalButton::from_parts(kind, code).unwrap_or_else(|| defaults.button(action))
        };
        let mut bindings = Bindings::new();
        bindings.rebind(
            BoundAction::Attack,
            pick(self.attack_kind, self.attack_code, BoundAction::Attack),
        );
        bindings.rebind(
            BoundAction::Block,
            pick(self.block_kind, self.block_code, BoundAction::Block),
        );
        bindings.rebind(
            BoundAction::Dodge,
            pick(self.dodge_kind, self.dodge_code, BoundAction::Dodge),
        );
        bindings.rebind(
            BoundAction::MoveForward,
            pick(
                self.forward_kind,
                self.forward_code,
                BoundAction::MoveForward,
            ),
        );
        bindings.rebind(
            BoundAction::MoveBack,
            pick(self.back_kind, self.back_code, BoundAction::MoveBack),
        );
        bindings.rebind(
            BoundAction::MoveLeft,
            pick(self.left_kind, self.left_code, BoundAction::MoveLeft),
        );
        bindings.rebind(
            BoundAction::MoveRight,
            pick(self.right_kind, self.right_code, BoundAction::MoveRight),
        );
        bindings.set_deadzone(self.deadzone);
        bindings
    }
}

impl Default for InputConfig {
    /// Save form of the default table (see [`Bindings::new`]).
    fn default() -> Self {
        Self::from_bindings(&Bindings::new())
    }
}

#[cfg(test)]
mod tests {
    use super::super::fighter::{FACING_RANGE, MAX_PARTICIPANTS, WISH_RANGE};
    use super::*;

    fn fighter() -> FighterId {
        FighterId::new(0).unwrap()
    }

    #[test]
    fn attack_key_maps_to_attack_bit() {
        let bindings = Bindings::new();
        // The default attack binding is a keyboard key by contract.
        assert!(matches!(
            bindings.button(BoundAction::Attack),
            PhysicalButton::Key(_)
        ));
        let attack = bindings.button(BoundAction::Attack);
        let input = bindings
            .to_fighter_input(3, fighter(), &[attack], (0, 0), 0)
            .unwrap();
        assert_eq!(input.buttons, Buttons(ATTACK));
        assert_eq!(input.tick, 3);
    }

    #[test]
    fn block_and_dodge_keys_map_to_their_bits() {
        let bindings = Bindings::new();
        let pressed = [
            bindings.button(BoundAction::Block),
            bindings.button(BoundAction::Dodge),
        ];
        let bits = bindings.button_bits(&pressed);
        assert_eq!(bits, Buttons(BLOCK | DODGE));
    }

    #[test]
    fn unbound_codes_are_ignored() {
        let bindings = Bindings::new();
        // Codes nothing is bound to: no default row uses them.
        let pressed = [
            PhysicalButton::Key(200),
            PhysicalButton::Mouse(77),
            PhysicalButton::Pad(77),
        ];
        assert_eq!(bindings.button_bits(&pressed), Buttons::NONE);
        let input = bindings
            .to_fighter_input(0, fighter(), &pressed, (10, 0), 0)
            .unwrap();
        assert_eq!(input.buttons, Buttons::NONE);
        assert_eq!((input.mx, input.mz), (10, 0));
    }

    #[test]
    fn deadzone_snaps_small_wish_to_zero() {
        assert_eq!(apply_deadzone(2, 1, DEFAULT_DEADZONE), (0, 0));
        assert_eq!(apply_deadzone(0, 0, 0), (0, 0));
        // Boundary magnitude snaps too.
        assert_eq!(apply_deadzone(8, 0, 8), (0, 0));
        // Large wishes pass through untouched.
        assert_eq!(apply_deadzone(100, 0, DEFAULT_DEADZONE), (100, 0));
        assert_eq!(apply_deadzone(0, -100, DEFAULT_DEADZONE), (0, -100));
        // The table applies its deadzone on the way to FighterInput.
        let bindings = Bindings::new();
        let input = bindings
            .to_fighter_input(0, fighter(), &[], (2, 1), 0)
            .unwrap();
        assert_eq!((input.mx, input.mz), (0, 0));
    }

    #[test]
    fn rebind_roundtrip_through_config_changes_output() {
        let mut bindings = Bindings::new();
        let old = bindings.button(BoundAction::Attack);
        bindings.rebind(BoundAction::Attack, PhysicalButton::Pad(3));
        let config = InputConfig::from_bindings(&bindings);
        let restored = config.to_bindings();
        assert_eq!(restored, bindings);
        assert_eq!(restored.button(BoundAction::Attack), PhysicalButton::Pad(3));

        // Old button no longer attacks; the rebound one does.
        assert_eq!(restored.button_bits(&[old]), Buttons::NONE);
        let input = restored
            .to_fighter_input(1, fighter(), &[PhysicalButton::Pad(3)], (0, 0), 0)
            .unwrap();
        assert_eq!(input.buttons, Buttons(ATTACK));
    }

    #[test]
    fn unknown_config_kinds_fall_back_to_defaults() {
        let config = InputConfig {
            attack_kind: 99,
            ..InputConfig::default()
        };
        let restored = config.to_bindings();
        assert_eq!(
            restored.button(BoundAction::Attack),
            Bindings::new().button(BoundAction::Attack)
        );
        assert_eq!(PhysicalButton::from_parts(99, 0), None);
    }

    #[test]
    fn invalid_wish_and_facing_are_rejected() {
        let bindings = Bindings::new();
        assert_eq!(
            bindings.to_fighter_input(0, fighter(), &[], (WISH_RANGE + 1, 0), 0),
            Err(FighterError::WishOutOfRange(WISH_RANGE + 1))
        );
        assert_eq!(
            bindings.to_fighter_input(0, fighter(), &[], (0, -WISH_RANGE - 1), 0),
            Err(FighterError::WishOutOfRange(-WISH_RANGE - 1))
        );
        assert_eq!(
            bindings.to_fighter_input(0, fighter(), &[], (0, 0), FACING_RANGE + 1),
            Err(FighterError::FacingOutOfRange(FACING_RANGE + 1))
        );
        assert_eq!(
            bindings.to_fighter_input(0, FighterId(MAX_PARTICIPANTS), &[], (0, 0), 0),
            Err(FighterError::UnknownFighter(MAX_PARTICIPANTS))
        );
    }

    #[test]
    fn movement_presses_build_full_wish() {
        let bindings = Bindings::new();
        let fwd = bindings.button(BoundAction::MoveForward);
        let back = bindings.button(BoundAction::MoveBack);
        let left = bindings.button(BoundAction::MoveLeft);
        let right = bindings.button(BoundAction::MoveRight);
        assert_eq!(bindings.wish_from_pressed(&[fwd]), (100, 0));
        assert_eq!(bindings.wish_from_pressed(&[left]), (0, -100));
        assert_eq!(bindings.wish_from_pressed(&[right]), (0, 100));
        // Opposed pairs cancel; move keys never set button bits.
        assert_eq!(bindings.wish_from_pressed(&[fwd, back]), (0, 0));
        assert_eq!(bindings.wish_from_pressed(&[left, right]), (0, 0));
        assert_eq!(
            bindings.button_bits(&[fwd, back, left, right]),
            Buttons::NONE
        );
    }
}
