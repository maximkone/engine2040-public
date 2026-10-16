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
use super::map::{Input, Key};

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

    /// Convert a [`map::Input`](super::map::Input) frame reading into a button.
    pub const fn from_input(input: Input) -> Self {
        match input {
            Input::Key(k) => Self::Key(k.0),
            Input::Mouse(b) => Self::Mouse(b),
            Input::Pad(b) => Self::Pad(b),
        }
    }

    /// Convert back to a [`map::Input`](super::map::Input) frame reading.
    pub const fn to_input(self) -> Input {
        match self {
            Self::Key(c) => Input::Key(Key(c)),
            Self::Mouse(b) => Input::Mouse(b),
            Self::Pad(b) => Input::Pad(b),
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
