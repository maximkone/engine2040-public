//! touch: the touch lane. Fingers on the glass drive the same virtual pad
//! buttons a physical pad drives, so one binding table serves both.
//!
//! Consumer: the platform layer (Windows first: the shell's pointer and
//! touch messages) scales its pixel coordinates into permille of the screen
//! with [`TouchPoint::from_pixels`] and hands the lane one [`TouchEvent`]
//! list per fixed tick; the game layer reads the lane's [`TouchFrame`] and
//! records the same [`FighterInput`](crate::fighter::FighterInput) it records
//! for a pad. Data owner: [`TouchLane`] owns the live fingers. Units:
//! screen coordinates in permille (`0..=1000`), ticks are zero-based fixed
//! steps. Determinism: integer only, no clock, no float, no heap; the same
//! event script gives the same buttons on every machine. Errors: never
//! fails; a point off the screen, a finger id past [`MAX_FINGERS`] and a
//! finger that was never down are ignored.
//!
//! The vocabulary, in the order the row names it:
//!
//! | gesture | what the lane does | buttons |
//! |---|---|---|
//! | tap | a finger inside an action zone holds that button | [`PAD_ATTACK`] or [`PAD_BLOCK`] |
//! | drag | a finger inside the stick zone holds the direction buttons | [`PAD_UP`], [`PAD_DOWN`], [`PAD_LEFT`], [`PAD_RIGHT`] |
//! | two fingers | two fingers on the glass hold the dodge button | [`PAD_DODGE`] |
//!
//! Every button is a pad code, so a bindings table bound to
//! [`PhysicalButton::Pad`] rows reads a finger and a pad the same way: the
//! same action bits, the same tape, the same hash.

use crate::bindings::PhysicalButton;

/// Screen coordinate bound: permille of the screen edge.
pub const SCREEN_PERMILLE: i16 = 1000;

/// Live fingers the lane tracks at once.
pub const MAX_FINGERS: usize = 5;

/// Virtual pad code for the attack button.
pub const PAD_ATTACK: u8 = 0;
/// Virtual pad code for the block button.
pub const PAD_BLOCK: u8 = 1;
/// Virtual pad code for the dodge button.
pub const PAD_DODGE: u8 = 2;
/// Virtual pad code for the up direction.
pub const PAD_UP: u8 = 3;
/// Virtual pad code for the down direction.
pub const PAD_DOWN: u8 = 4;
/// Virtual pad code for the left direction.
pub const PAD_LEFT: u8 = 5;
/// Virtual pad code for the right direction.
pub const PAD_RIGHT: u8 = 6;

/// The virtual pad codes, in table order: the codes a physical pad sends.
pub const TOUCH_BUTTONS: [u8; 7] = [
    PAD_ATTACK,
    PAD_BLOCK,
    PAD_DODGE,
    PAD_UP,
    PAD_DOWN,
    PAD_LEFT,
    PAD_RIGHT,
];

/// Count of virtual pad buttons the lane holds.
pub const TOUCH_BUTTON_COUNT: usize = TOUCH_BUTTONS.len();

/// Stick deadzone in permille: a drag shorter than this is rest.
pub const STICK_DEADZONE_PERMILLE: i16 = 120;

/// One finger on the glass: an id and a point in permille of the screen.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TouchPoint {
    /// Finger id: one id is one finger until it lifts.
    pub id: u8,
    /// Horizontal position, permille of the screen width.
    pub x: i16,
    /// Vertical position, permille of the screen height, growing downward.
    pub y: i16,
}

impl TouchPoint {
    /// Point for a finger id at a permille position.
    pub const fn new(id: u8, x: i16, y: i16) -> Self {
        Self { id, x, y }
    }

    /// Point from pixel coordinates and a screen size.
    ///
    /// The Windows bridge: the shell knows pixels, the lane knows permille.
    /// Integer scaling only, so the same pixels give the same point on every
    /// machine. A pixel outside the screen clamps to the edge, and a screen
    /// size of zero or less gives the origin.
    pub fn from_pixels(id: u8, x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            id,
            x: scale_permille(x, width),
            y: scale_permille(y, height),
        }
    }
}

/// Scale a pixel coordinate into permille of the screen extent.
fn scale_permille(value: i32, extent: i32) -> i16 {
    if extent <= 0 {
        return 0;
    }
    let clamped = value.clamp(0, extent);
    let permille = (i64::from(clamped) * i64::from(SCREEN_PERMILLE)) / i64::from(extent);
    i16::try_from(permille).unwrap_or(0)
}

/// What a finger did on one tick.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TouchEvent {
    /// The finger touched the glass.
    Down(TouchPoint),
    /// The finger moved.
    Move(TouchPoint),
    /// The finger lifted.
    Up(TouchPoint),
}

/// A screen rectangle in permille: the layout is data, not code.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TouchZone {
    /// Left edge, permille.
    pub x0: i16,
    /// Top edge, permille.
    pub y0: i16,
    /// Right edge, permille.
    pub x1: i16,
    /// Bottom edge, permille.
    pub y1: i16,
}

impl TouchZone {
    /// Zone from its four edges.
    pub const fn new(x0: i16, y0: i16, x1: i16, y1: i16) -> Self {
        Self { x0, y0, x1, y1 }
    }

    /// True when the point is inside the zone, edges included.
    pub fn contains(self, point: TouchPoint) -> bool {
        point.x >= self.x0 && point.x <= self.x1 && point.y >= self.y0 && point.y <= self.y1
    }

    /// Middle of the zone, permille.
    fn center(self) -> (i32, i32) {
        (
            (i32::from(self.x0) + i32::from(self.x1)) / 2,
            (i32::from(self.y0) + i32::from(self.y1)) / 2,
        )
    }
}

/// The screen layout: which zone drives which button.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TouchLayout {
    /// The virtual stick: a drag inside it drives the direction buttons.
    pub stick: TouchZone,
    /// The attack button.
    pub attack: TouchZone,
    /// The block button.
    pub block: TouchZone,
}

impl TouchLayout {
    /// The shipped layout: the stick on the left, two action zones on the
    /// right, dodge on two fingers.
    pub const fn shipped() -> Self {
        Self {
            stick: TouchZone::new(0, 0, 400, 1000),
            attack: TouchZone::new(700, 500, 1000, 1000),
            block: TouchZone::new(700, 0, 1000, 500),
        }
    }
}

impl Default for TouchLayout {
    /// The shipped layout (see [`TouchLayout::shipped`]).
    fn default() -> Self {
        Self::shipped()
    }
}

/// One tick of the touch lane: the virtual pad buttons it holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct TouchFrame {
    buttons: [bool; TOUCH_BUTTON_COUNT],
}

impl TouchFrame {
    /// True when the pad code is held this tick.
    pub fn held(self, code: u8) -> bool {
        match row_of(code) {
            Some(row) => self.buttons[row],
            None => false,
        }
    }

    /// The held pad codes, in table order.
    pub fn pressed(self) -> impl Iterator<Item = u8> {
        let buttons = self.buttons;
        buttons
            .into_iter()
            .enumerate()
            .filter(|(_, held)| *held)
            .map(|(row, _)| TOUCH_BUTTONS[row])
    }

    /// The held buttons as pad bindings, in table order.
    pub fn pressed_buttons(self) -> impl Iterator<Item = PhysicalButton> {
        self.pressed().map(PhysicalButton::Pad)
    }
}

/// Table row of a pad code; `None` when the code is not a touch button.
fn row_of(code: u8) -> Option<usize> {
    TOUCH_BUTTONS.iter().position(|&c| c == code)
}

/// The touch lane: the live fingers and the buttons they hold.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TouchLane {
    layout: TouchLayout,
    fingers: [Option<TouchPoint>; MAX_FINGERS],
    buttons: [bool; TOUCH_BUTTON_COUNT],
}

impl TouchLane {
    /// Lane with the shipped layout and no finger down.
    pub fn new() -> Self {
        Self::with_layout(TouchLayout::shipped())
    }

    /// Lane with one layout and no finger down.
    pub fn with_layout(layout: TouchLayout) -> Self {
        Self {
            layout,
            fingers: [None; MAX_FINGERS],
            buttons: [false; TOUCH_BUTTON_COUNT],
        }
    }

    /// The layout this lane reads.
    pub fn layout(self) -> TouchLayout {
        self.layout
    }

    /// Drop every finger and clear the buttons.
    pub fn reset(&mut self) {
        self.fingers = [None; MAX_FINGERS];
        self.buttons = [false; TOUCH_BUTTON_COUNT];
    }

    /// Advance one fixed tick with this tick's events, and report the
    /// buttons the fingers hold.
    ///
    /// The events are read in the order given, so a script replays exactly.
    /// A finger id past [`MAX_FINGERS`] is ignored: the lane tracks the
    /// fingers it can hold, never more.
    pub fn tick(&mut self, events: &[TouchEvent]) -> TouchFrame {
        self.buttons = [false; TOUCH_BUTTON_COUNT];
        for event in events {
            match *event {
                TouchEvent::Down(point) => self.set_finger(point),
                TouchEvent::Move(point) => self.set_finger(point),
                TouchEvent::Up(point) => self.clear_finger(point.id),
            }
        }
        self.update_buttons();
        TouchFrame {
            buttons: self.buttons,
        }
    }

    /// Store a finger's position; a new id joins, a known id moves.
    fn set_finger(&mut self, point: TouchPoint) {
        if let Some(slot) = self.slot_of(point.id) {
            self.fingers[slot] = Some(point);
        }
    }

    /// Drop a finger that lifted.
    fn clear_finger(&mut self, id: u8) {
        if let Some(slot) = self.slot_of(id) {
            self.fingers[slot] = None;
        }
    }

    /// The slot a finger id owns, if the lane tracks it.
    fn slot_of(&self, id: u8) -> Option<usize> {
        let slot = usize::from(id);
        if slot < MAX_FINGERS {
            Some(slot)
        } else {
            None
        }
    }

    /// Read the live fingers into the button set.
    fn update_buttons(&mut self) {
        let mut live = 0usize;
        for point in self.fingers.into_iter().flatten() {
            live = live.saturating_add(1);
            self.apply_finger(point);
        }
        if live >= 2 {
            // Two fingers on the glass: the dodge gesture.
            self.set(PAD_DODGE);
        }
    }

    /// One finger's contribution: the stick or an action zone.
    fn apply_finger(&mut self, finger: TouchPoint) {
        if self.layout.stick.contains(finger) {
            self.apply_stick(finger);
            return;
        }
        if self.layout.attack.contains(finger) {
            self.set(PAD_ATTACK);
        }
        if self.layout.block.contains(finger) {
            self.set(PAD_BLOCK);
        }
    }

    /// The stick: a drag from the zone's middle holds the directions it
    /// leaves the deadzone towards.
    fn apply_stick(&mut self, finger: TouchPoint) {
        let (center_x, center_y) = self.layout.stick.center();
        let dx = i32::from(finger.x) - center_x;
        let dy = i32::from(finger.y) - center_y;
        let dead = i32::from(STICK_DEADZONE_PERMILLE);
        if dx <= -dead {
            self.set(PAD_LEFT);
        }
        if dx >= dead {
            self.set(PAD_RIGHT);
        }
        if dy <= -dead {
            self.set(PAD_UP);
        }
        if dy >= dead {
            self.set(PAD_DOWN);
        }
    }

    /// Hold one virtual pad button.
    fn set(&mut self, code: u8) {
        if let Some(row) = row_of(code) {
            self.buttons[row] = true;
        }
    }
}

impl Default for TouchLane {
    /// Lane with the shipped layout (see [`TouchLane::new`]).
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::{Bindings, BoundAction};
    use crate::fighter::{FighterId, FighterTape};
    use engine_core::hash_bytes;

    /// The bindings a pad would use: every action on a virtual pad code.
    fn pad_bindings() -> Bindings {
        let mut bindings = Bindings::new();
        bindings.rebind(BoundAction::Attack, PhysicalButton::Pad(PAD_ATTACK));
        bindings.rebind(BoundAction::Block, PhysicalButton::Pad(PAD_BLOCK));
        bindings.rebind(BoundAction::Dodge, PhysicalButton::Pad(PAD_DODGE));
        bindings.rebind(BoundAction::MoveForward, PhysicalButton::Pad(PAD_UP));
        bindings.rebind(BoundAction::MoveBack, PhysicalButton::Pad(PAD_DOWN));
        bindings.rebind(BoundAction::MoveLeft, PhysicalButton::Pad(PAD_LEFT));
        bindings.rebind(BoundAction::MoveRight, PhysicalButton::Pad(PAD_RIGHT));
        bindings
    }

    /// One tick of the script: the touch events and the pad buttons a
    /// physical pad sends on the same tick.
    struct Step {
        events: Vec<TouchEvent>,
        pad: Vec<PhysicalButton>,
    }

    fn down(id: u8, x: i16, y: i16) -> TouchEvent {
        TouchEvent::Down(TouchPoint::new(id, x, y))
    }

    fn move_to(id: u8, x: i16, y: i16) -> TouchEvent {
        TouchEvent::Move(TouchPoint::new(id, x, y))
    }

    fn up(id: u8, x: i16, y: i16) -> TouchEvent {
        TouchEvent::Up(TouchPoint::new(id, x, y))
    }

    /// The script: a tap on attack, a drag up then right, a tap on block and
    /// a two-finger dodge. The pad column is what a physical pad sends.
    fn script() -> Vec<Step> {
        vec![
            Step {
                events: vec![down(0, 800, 700)],
                pad: vec![PhysicalButton::Pad(PAD_ATTACK)],
            },
            Step {
                events: vec![up(0, 800, 700)],
                pad: vec![],
            },
            Step {
                events: vec![down(1, 200, 500)],
                pad: vec![],
            },
            Step {
                events: vec![move_to(1, 200, 200)],
                pad: vec![PhysicalButton::Pad(PAD_UP)],
            },
            Step {
                events: vec![move_to(1, 400, 500)],
                pad: vec![PhysicalButton::Pad(PAD_RIGHT)],
            },
            Step {
                events: vec![up(1, 400, 500)],
                pad: vec![],
            },
            Step {
                events: vec![down(2, 800, 200)],
                pad: vec![PhysicalButton::Pad(PAD_BLOCK)],
            },
            Step {
                events: vec![up(2, 800, 200)],
                pad: vec![],
            },
            Step {
                events: vec![down(0, 800, 700), down(3, 600, 500)],
                pad: vec![PhysicalButton::Pad(PAD_ATTACK), PhysicalButton::Pad(PAD_DODGE)],
            },
            Step {
                events: vec![up(0, 800, 700), up(3, 600, 500)],
                pad: vec![],
            },
        ]
    }

    /// Hash of a tape: the same bytes the sim hashes.
    fn tape_hash(tape: &FighterTape) -> u64 {
        let mut bytes = Vec::new();
        for input in &tape.0 {
            input.append_bytes(&mut bytes);
        }
        hash_bytes(&bytes)
    }

    /// Replay the script through a fresh lane and hash the tape.
    fn replay(steps: &[Step], bindings: &Bindings) -> u64 {
        let fighter = FighterId::new(0).expect("slot 0");
        let mut lane = TouchLane::new();
        let mut tape = FighterTape(Vec::new());
        for (tick, step) in steps.iter().enumerate() {
            let frame = lane.tick(&step.events);
            let pressed: Vec<PhysicalButton> = frame.pressed_buttons().collect();
            let tick_index = u32::try_from(tick).unwrap_or(0);
            let input = bindings
                .to_fighter_input(tick_index, fighter, &pressed, (0, 0), 0)
                .expect("touch input");
            tape.0.push(input);
        }
        tape_hash(&tape)
    }

    /// The row's tape test: a touch tap and a swipe drive the same action
    /// bits as the pad mapping, hash the same tape, and two replays are
    /// equal.
    #[test]
    fn a_touch_tap_hashes_the_same_tape_as_the_pad() {
        let bindings = pad_bindings();
        let steps = script();
        let fighter = FighterId::new(0).expect("slot 0");
        let mut lane = TouchLane::new();
        let mut touch_tape = FighterTape(Vec::new());
        let mut pad_tape = FighterTape(Vec::new());
        for (tick, step) in steps.iter().enumerate() {
            let frame = lane.tick(&step.events);
            let touch: Vec<PhysicalButton> = frame.pressed_buttons().collect();
            assert_eq!(
                bindings.button_bits(&touch),
                bindings.button_bits(&step.pad),
                "tick {tick}: the touch bits differ from the pad bits"
            );
            assert_eq!(
                bindings.wish_from_pressed(&touch),
                bindings.wish_from_pressed(&step.pad),
                "tick {tick}: the touch wish differs from the pad wish"
            );
            let tick_index = u32::try_from(tick).unwrap_or(0);
            let touch_input = bindings
                .to_fighter_input(tick_index, fighter, &touch, (0, 0), 0)
                .expect("touch input");
            let pad_input = bindings
                .to_fighter_input(tick_index, fighter, &step.pad, (0, 0), 0)
                .expect("pad input");
            assert_eq!(touch_input, pad_input, "tick {tick}: the inputs differ");
            touch_tape.0.push(touch_input);
            pad_tape.0.push(pad_input);
        }
        assert_eq!(
            tape_hash(&touch_tape),
            tape_hash(&pad_tape),
            "the touch tape must hash like the pad tape"
        );
        assert_eq!(
            replay(&steps, &bindings),
            replay(&steps, &bindings),
            "two replays of one script are equal"
        );
    }

    /// Two fingers on the glass hold dodge, and the button clears when one
    /// of them lifts.
    #[test]
    fn two_fingers_hold_the_dodge_button() {
        let mut lane = TouchLane::new();
        let frame = lane.tick(&[down(0, 600, 500), down(1, 620, 500)]);
        assert!(frame.held(PAD_DODGE), "two fingers are the dodge gesture");
        let frame = lane.tick(&[up(1, 620, 500)]);
        assert!(!frame.held(PAD_DODGE), "one finger left is not a dodge");
        assert!(!frame.held(PAD_ATTACK), "and it holds no action button");
    }

    /// A drag inside the stick zone holds the directions it leaves the
    /// deadzone towards, and a drag at the middle holds nothing.
    #[test]
    fn a_drag_holds_the_directions_it_leaves_the_deadzone_towards() {
        let mut lane = TouchLane::new();
        let frame = lane.tick(&[down(0, 200, 500)]);
        assert!(!frame.held(PAD_UP) && !frame.held(PAD_DOWN));
        let frame = lane.tick(&[move_to(0, 200, 200)]);
        assert!(frame.held(PAD_UP), "up is held");
        assert!(!frame.held(PAD_DOWN), "down is not");
        let frame = lane.tick(&[move_to(0, 200, 800)]);
        assert!(frame.held(PAD_DOWN), "down is held");
        assert!(!frame.held(PAD_UP), "up is not");
    }

    /// A point off the screen clamps to the edge, and a zero screen size
    /// gives the origin: the Windows pixel bridge never fails.
    #[test]
    fn pixels_scale_into_permille() {
        let point = TouchPoint::from_pixels(0, 1920, 1080, 3840, 2160);
        assert_eq!(point.x, 500);
        assert_eq!(point.y, 500);
        let outside = TouchPoint::from_pixels(0, 4000, -20, 3840, 2160);
        assert_eq!(outside.x, 1000, "a point past the edge clamps");
        assert_eq!(outside.y, 0, "a point before the edge clamps");
        let empty = TouchPoint::from_pixels(0, 10, 10, 0, 0);
        assert_eq!((empty.x, empty.y), (0, 0));
    }

    /// A finger outside every zone holds nothing, and a finger id past the
    /// lane's cap is ignored.
    #[test]
    fn points_outside_every_zone_hold_nothing() {
        let mut lane = TouchLane::new();
        let frame = lane.tick(&[down(0, 500, 500), down(200, 800, 700)]);
        assert!(frame.pressed().next().is_none(), "no zone, no button");
    }
}
