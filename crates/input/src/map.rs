//! map: action map. One action = any-of chords; one chord = all-of inputs. Missing remote ticks repeat last.

use engine_core::hash_bytes;

/// Physical keyboard key, addressed by small code (`0..256`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Key(pub u8);

impl Key {
    /// Space bar.
    pub const SPACE: Self = Self(0);
    /// Shift modifier.
    pub const SHIFT: Self = Self(1);
    /// Control modifier.
    pub const CTRL: Self = Self(2);
    /// Move-forward key.
    pub const W: Self = Self(3);
    /// Move-left key.
    pub const A: Self = Self(4);
    /// Move-back key.
    pub const S: Self = Self(5);
    /// Move-right key.
    pub const D: Self = Self(6);
    /// Interact key.
    pub const E: Self = Self(7);
    /// Fire key.
    pub const F: Self = Self(8);
    /// Strafe-left key.
    pub const Q: Self = Self(9);
    /// Cancel key.
    pub const ESC: Self = Self(10);
    /// Confirm key.
    pub const ENTER: Self = Self(11);
    /// Raw code addressing [`Frame`] storage.
    pub const fn code(self) -> usize {
        self.0 as usize
    }
}

/// Physical input source: keyboard key, pad button, or mouse button.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Input {
    /// Keyboard key.
    Key(Key),
    /// Gamepad button index.
    Pad(u8),
    /// Mouse button index.
    Mouse(u8),
}

/// All-of binding: every listed input must be held; empty never matches.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chord {
    inputs: Vec<Input>,
}

impl Chord {
    /// Build a chord from borrowed physical inputs.
    pub fn new(inputs: &[Input]) -> Self {
        Self {
            inputs: inputs.to_vec(),
        }
    }
    /// Borrow the chord inputs.
    pub fn inputs(&self) -> &[Input] {
        &self.inputs
    }
    /// Count bound inputs.
    pub fn len(&self) -> usize {
        self.inputs.len()
    }
    /// True when no inputs are bound (never matches, by design).
    pub fn is_empty(&self) -> bool {
        self.inputs.is_empty()
    }
    /// True when every input is held (current or previous tick).
    fn all(&self, frame: &Frame, prev: bool) -> bool {
        if self.inputs.is_empty() {
            return false;
        }
        self.inputs
            .iter()
            .all(|&i| if prev { frame.was(i) } else { frame.held(i) })
    }
    /// True when every input is held this tick.
    pub fn satisfied(&self, frame: &Frame) -> bool {
        self.all(frame, false)
    }
}

/// Game action (user enum). Registration order in [`InputMap`] defines bits.
pub trait Action: Copy + Clone + PartialEq + Eq + core::fmt::Debug {
    /// Stable label for logs and replay tapes.
    fn name(self) -> &'static str;
}

/// One fixed tick of physical input plus the previous tick for edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    keys: [bool; 256],
    pads: [bool; 256],
    mice: [bool; 256],
    prev_keys: [bool; 256],
    prev_pads: [bool; 256],
    prev_mice: [bool; 256],
}

impl Frame {
    /// Blank frame: nothing held, no history.
    pub fn new() -> Self {
        Self {
            keys: [false; 256],
            pads: [false; 256],
            mice: [false; 256],
            prev_keys: [false; 256],
            prev_pads: [false; 256],
            prev_mice: [false; 256],
        }
    }
    /// Advance one fixed tick: current becomes previous, `held` is current.
    pub fn step(&mut self, held: &[Input]) {
        self.prev_keys = self.keys;
        self.prev_pads = self.pads;
        self.prev_mice = self.mice;
        self.keys = [false; 256];
        self.pads = [false; 256];
        self.mice = [false; 256];
        for &i in held {
            match i {
                Input::Key(k) => self.keys[k.code()] = true,
                Input::Pad(b) => self.pads[usize::from(b)] = true,
                Input::Mouse(b) => self.mice[usize::from(b)] = true,
            }
        }
    }
    /// True when the physical input is held this tick.
    pub fn held(&self, input: Input) -> bool {
        match input {
            Input::Key(k) => self.keys[k.code()],
            Input::Pad(b) => self.pads[usize::from(b)],
            Input::Mouse(b) => self.mice[usize::from(b)],
        }
    }
    /// True when the physical input was held last tick.
    fn was(&self, input: Input) -> bool {
        match input {
            Input::Key(k) => self.prev_keys[k.code()],
            Input::Pad(b) => self.prev_pads[usize::from(b)],
            Input::Mouse(b) => self.prev_mice[usize::from(b)],
        }
    }
}

impl Default for Frame {
    /// Blank frame (see [`Frame::new`]).
    fn default() -> Self {
        Self::new()
    }
}

/// Maps actions to any-of chords, evaluated per [`Frame`]. Same registration + same frames = same bits.
#[derive(Clone, Debug)]
pub struct InputMap<A> {
    actions: Vec<A>,
    chords: Vec<Vec<Chord>>,
}

impl<A: Action> InputMap<A> {
    /// Empty map (no actions bound).
    pub fn new() -> Self {
        Self {
            actions: Vec::new(),
            chords: Vec::new(),
        }
    }
    /// Bind one all-of chord to an action (any-of across binds).
    pub fn bind(&mut self, action: A, chord: Chord) {
        match self.position(action) {
            Some(i) => self.chords[i].push(chord),
            None => {
                self.actions.push(action);
                self.chords.push(vec![chord]);
            }
        }
    }
    /// Bind borrowed raw inputs as one all-of chord.
    pub fn bind_keys(&mut self, action: A, inputs: &[Input]) {
        self.bind(action, Chord::new(inputs));
    }
    /// Count registered actions.
    pub fn len(&self) -> usize {
        self.actions.len()
    }
    /// True when no actions are registered.
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
    /// True when the action has any binding.
    pub fn contains(&self, action: A) -> bool {
        self.position(action).is_some()
    }
    /// Registration index of an action (linear scan, deterministic).
    fn position(&self, action: A) -> Option<usize> {
        self.actions.iter().position(|&a| a == action)
    }
    /// True when any chord of the action holds (current or previous tick).
    fn eval(&self, frame: &Frame, action: A, prev: bool) -> bool {
        match self.position(action) {
            Some(i) => self.chords[i].iter().any(|c| c.all(frame, prev)),
            None => false,
        }
    }
    /// True when the action is held this tick.
    pub fn pressed(&self, frame: &Frame, action: A) -> bool {
        self.eval(frame, action, false)
    }
    /// Rising edge: held now, not held last tick.
    pub fn just_pressed(&self, frame: &Frame, action: A) -> bool {
        self.eval(frame, action, false) && !self.eval(frame, action, true)
    }
    /// Falling edge: released now, held last tick.
    pub fn just_released(&self, frame: &Frame, action: A) -> bool {
        !self.eval(frame, action, false) && self.eval(frame, action, true)
    }
    /// Bit `i` = registered action `i` pressed (first 64 registrations).
    pub fn action_bits(&self, frame: &Frame) -> u64 {
        let mut bits: u64 = 0;
        for (i, &a) in self.actions.iter().enumerate().take(64) {
            if self.eval(frame, a, false) {
                bits |= 1u64 << i;
            }
        }
        bits
    }
    /// Deterministic tick hash over action bits (canon FNV-1a via core).
    pub fn hash_frame(&self, frame: &Frame) -> u64 {
        hash_bits(self.action_bits(frame))
    }
}

impl<A: Action> Default for InputMap<A> {
    /// Empty map (see [`InputMap::new`]).
    fn default() -> Self {
        Self::new()
    }
}

/// FNV-1a 64 over action-bit bytes (canon engine-core recipe, not a copy).
pub fn hash_bits(bits: u64) -> u64 {
    hash_bytes(&bits.to_le_bytes())
}

/// Repeat-last predictor: the rollback hook for inputs-only net.
///
/// Net calls [`RepeatLast::confirm`] on each present remote tick and
/// [`RepeatLast::predict`] (last confirmed, 0 before first) on loss;
/// rollback resimulates on mispredict (GGPO shape, MaxRollback 8 in net).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RepeatLast {
    last: u64,
    has: bool,
}

impl RepeatLast {
    /// Empty predictor (predicts 0 until first confirm).
    pub fn new() -> Self {
        Self {
            last: 0,
            has: false,
        }
    }
    /// Store present remote bits; returns them unchanged.
    pub fn confirm(&mut self, bits: u64) -> u64 {
        self.last = bits;
        self.has = true;
        bits
    }
    /// Repeat last confirmed bits (0 before the first confirm).
    pub fn predict(&self) -> u64 {
        self.last
    }
    /// True once any present input has arrived.
    pub fn has(&self) -> bool {
        self.has
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test action set: move on W, jump on the Space+Shift chord.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Act {
        Move,
        Jump,
    }

    impl Action for Act {
        fn name(self) -> &'static str {
            match self {
                Act::Move => "move",
                Act::Jump => "jump",
            }
        }
    }

    /// Map under test: Move=[W], Jump=[Space+Shift].
    fn map() -> InputMap<Act> {
        let mut m = InputMap::new();
        m.bind_keys(Act::Move, &[Input::Key(Key::W)]);
        m.bind_keys(Act::Jump, &[Input::Key(Key::SPACE), Input::Key(Key::SHIFT)]);
        m
    }

    #[test]
    fn chord_requires_all_keys() {
        let m = map();
        let mut f = Frame::new();
        f.step(&[Input::Key(Key::SPACE)]);
        assert!(!m.pressed(&f, Act::Jump));
        f.step(&[Input::Key(Key::SPACE), Input::Key(Key::SHIFT)]);
        assert!(m.pressed(&f, Act::Jump));
        assert!(!m.pressed(&f, Act::Move));
    }

    #[test]
    fn hold_stays_pressed_edge_once() {
        let m = map();
        let mut f = Frame::new();
        f.step(&[Input::Key(Key::W)]);
        assert!(m.pressed(&f, Act::Move) && m.just_pressed(&f, Act::Move));
        f.step(&[Input::Key(Key::W)]);
        assert!(m.pressed(&f, Act::Move) && !m.just_pressed(&f, Act::Move));
        f.step(&[Input::Key(Key::W)]);
        assert!(m.pressed(&f, Act::Move) && !m.just_released(&f, Act::Move));
    }

    #[test]
    fn release_clears_and_reports_once() {
        let m = map();
        let mut f = Frame::new();
        f.step(&[Input::Key(Key::W)]);
        f.step(&[]);
        assert!(!m.pressed(&f, Act::Move));
        assert!(m.just_released(&f, Act::Move));
        f.step(&[]);
        assert!(!m.just_released(&f, Act::Move));
    }

    #[test]
    fn hash_same_inputs_same_hash_x2() {
        let script: Vec<Vec<Input>> = vec![
            vec![Input::Key(Key::W)],
            vec![Input::Key(Key::W)],
            vec![Input::Key(Key::SPACE), Input::Key(Key::SHIFT)],
            vec![],
        ];
        let run = || {
            let m = map();
            let mut f = Frame::new();
            let mut h = 0u64;
            for held in &script {
                f.step(held);
                h ^= m.hash_frame(&f);
            }
            h
        };
        assert_eq!(run(), run());
        let m = map();
        let mut f = Frame::new();
        f.step(&[Input::Key(Key::W)]);
        let hw = m.hash_frame(&f);
        f.step(&[Input::Key(Key::F)]);
        let hf = m.hash_frame(&f);
        assert_ne!(hw, hf);
    }

    #[test]
    fn repeat_last_predicts_on_loss() {
        let mut r = RepeatLast::new();
        assert_eq!(r.predict(), 0);
        assert!(!r.has());
        assert_eq!(r.confirm(0b101), 0b101);
        assert!(r.has());
        assert_eq!(r.predict(), 0b101);
        assert_eq!(r.confirm(0b010), 0b010);
        assert_eq!(r.predict(), 0b010);
    }

    #[test]
    fn replay_same_inputs_same_bits() {
        let script: Vec<Vec<Input>> = vec![
            vec![Input::Key(Key::W)],
            vec![Input::Key(Key::SPACE)],
            vec![Input::Key(Key::SPACE), Input::Key(Key::SHIFT)],
            vec![],
            vec![Input::Pad(0)],
        ];
        let play = || {
            let m = map();
            let mut f = Frame::new();
            script
                .iter()
                .map(|held| {
                    f.step(held);
                    m.action_bits(&f)
                })
                .collect::<Vec<u64>>()
        };
        assert_eq!(play(), play());
    }
}
