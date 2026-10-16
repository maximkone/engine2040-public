//! action-state input map for deterministic replay.
//!
//! Shape ported as std-only Rust from the `Leafwing-Studios/leafwing-input-manager`
//! action-state shape (Apache-2.0, reimplemented only, never the crate dep):
//! digital actions as pressed/held/released bits plus analog actions as
//! fixed-point thousandths, sampled once per tick. Same tape twice gives the
//! same bytes; held-vs-pressed never leaks across tick boundaries.
//!
//! Bindings are frozen (`bindings.rs` owns them): this module stores per-tick
//! state only, never key codes.

use engine_core::hash_bytes;

/// Digital action slots (`0..MAX_ACTIONS`, bit `i` = action `i`).
pub const MAX_ACTIONS: usize = 64;
/// Analog action slots (`0..MAX_ANALOG`), fixed-point thousandths.
pub const MAX_ANALOG: usize = 8;

/// One fixed tick of action state: digital bits plus analog axes.
///
/// Sample once per tick with [`ActionState::sample`]; edges compare the
/// current tick against the previous one only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ActionState {
    current: u64,
    previous: u64,
    analog: [i32; MAX_ANALOG],
    prev_analog: [i32; MAX_ANALOG],
    ticks: u64,
}

impl ActionState {
    /// Blank state: nothing held, no history.
    pub fn new() -> Self {
        Self {
            current: 0,
            previous: 0,
            analog: [0; MAX_ANALOG],
            prev_analog: [0; MAX_ANALOG],
            ticks: 0,
        }
    }

    /// Sample one fixed tick: `digital` bit `i` holds action `i`,
    /// `analog[i]` is axis `i` in thousandths. Previous becomes last tick's
    /// current, so edges never leak across boundaries.
    pub fn sample(&mut self, digital: u64, analog: [i32; MAX_ANALOG]) {
        self.previous = self.current;
        self.prev_analog = self.analog;
        self.current = digital;
        self.analog = analog;
        self.ticks = self.ticks.saturating_add(1);
    }

    /// Ticks sampled so far.
    pub fn ticks(self) -> u64 {
        self.ticks
    }

    /// True when action `i` is held this tick.
    pub fn pressed(self, action: u8) -> bool {
        let i = usize::from(action);
        i < MAX_ACTIONS && (self.current >> i) & 1 == 1
    }

    /// Alias for [`ActionState::pressed`] (held this tick).
    pub fn held(self, action: u8) -> bool {
        self.pressed(action)
    }

    /// Rising edge: held now, not held last tick.
    pub fn just_pressed(self, action: u8) -> bool {
        let i = usize::from(action);
        if i >= MAX_ACTIONS {
            return false;
        }
        (self.current >> i) & 1 == 1 && (self.previous >> i) & 1 == 0
    }

    /// Falling edge: released now, held last tick.
    pub fn just_released(self, action: u8) -> bool {
        let i = usize::from(action);
        if i >= MAX_ACTIONS {
            return false;
        }
        (self.current >> i) & 1 == 0 && (self.previous >> i) & 1 == 1
    }

    /// Current digital bits.
    pub fn bits(self) -> u64 {
        self.current
    }

    /// Previous tick digital bits.
    pub fn prev_bits(self) -> u64 {
        self.previous
    }

    /// Analog axis `i` in thousandths this tick (`0` out of range).
    pub fn analog(self, axis: u8) -> i32 {
        let i = usize::from(axis);
        if i < MAX_ANALOG {
            self.analog[i]
        } else {
            0
        }
    }

    /// Analog axis `i` last tick.
    pub fn prev_analog(self, axis: u8) -> i32 {
        let i = usize::from(axis);
        if i < MAX_ANALOG {
            self.prev_analog[i]
        } else {
            0
        }
    }

    /// Deterministic digest of the whole state (replay compare).
    pub fn digest(self) -> u64 {
        let mut bytes = [0u8; 8 + 8 + MAX_ANALOG * 4 * 2 + 8];
        bytes[0..8].copy_from_slice(&self.current.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.previous.to_le_bytes());
        for (n, v) in self.analog.iter().enumerate() {
            bytes[16 + n * 4..16 + n * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        let base = 16 + MAX_ANALOG * 4;
        for (n, v) in self.prev_analog.iter().enumerate() {
            bytes[base + n * 4..base + n * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        bytes[base + MAX_ANALOG * 4..].copy_from_slice(&self.ticks.to_le_bytes());
        hash_bytes(&bytes)
    }

    /// Byte-serial form of the state (replay tapes compare these).
    pub fn to_bytes(self) -> [u8; 8 + 8 + MAX_ANALOG * 4 * 2 + 8] {
        let mut bytes = [0u8; 8 + 8 + MAX_ANALOG * 4 * 2 + 8];
        bytes[0..8].copy_from_slice(&self.current.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.previous.to_le_bytes());
        for (n, v) in self.analog.iter().enumerate() {
            bytes[16 + n * 4..16 + n * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        let base = 16 + MAX_ANALOG * 4;
        for (n, v) in self.prev_analog.iter().enumerate() {
            bytes[base + n * 4..base + n * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        bytes[base + MAX_ANALOG * 4..].copy_from_slice(&self.ticks.to_le_bytes());
        bytes
    }
}

impl Default for ActionState {
    /// Blank state (see [`ActionState::new`]).
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZERO_ANALOG: [i32; MAX_ANALOG] = [0; MAX_ANALOG];

    fn tape_600() -> Vec<(u64, [i32; MAX_ANALOG])> {
        let mut tape = Vec::with_capacity(600);
        let mut analog = [0i32; MAX_ANALOG];
        for t in 0..600u64 {
            // Deterministic script: bits rotate, axis 0 ramps and holds.
            let bits = (t.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 11) & 0xFF;
            analog[0] = ((t % 200) as i32 - 100) * 10;
            analog[1] = if t % 3 == 0 { 1000 } else { -1000 };
            tape.push((bits, analog));
        }
        tape
    }

    fn run(tape: &[(u64, [i32; MAX_ANALOG])]) -> Vec<[u8; 88]> {
        let mut state = ActionState::new();
        tape.iter()
            .map(|&(bits, analog)| {
                state.sample(bits, analog);
                state.to_bytes()
            })
            .collect()
    }

    #[test]
    fn same_tape_replays_byte_identical_x2() {
        let tape = tape_600();
        assert_eq!(run(&tape), run(&tape));
    }

    #[test]
    fn tape_digest_matches_x2_over_600_ticks() {
        let tape = tape_600();
        let digest = |t: &[(u64, [i32; MAX_ANALOG])]| {
            let mut state = ActionState::new();
            for &(bits, analog) in t {
                state.sample(bits, analog);
            }
            state.digest()
        };
        assert_eq!(digest(&tape), digest(&tape));
        assert_eq!(run(&tape).len(), 600);
    }

    #[test]
    fn held_vs_pressed_never_leaks_across_ticks() {
        let mut s = ActionState::new();
        // Action 2 held for two ticks: edge fires once, release fires once,
        // then silence — nothing leaks into later ticks.
        s.sample(1 << 2, ZERO_ANALOG);
        assert!(s.held(2) && s.just_pressed(2) && !s.just_released(2));
        s.sample(1 << 2, ZERO_ANALOG);
        assert!(s.held(2) && !s.just_pressed(2) && !s.just_released(2));
        s.sample(0, ZERO_ANALOG);
        assert!(!s.held(2) && !s.just_pressed(2) && s.just_released(2));
        s.sample(0, ZERO_ANALOG);
        assert!(!s.held(2) && !s.just_pressed(2) && !s.just_released(2));
        // Analog history advances one tick only.
        let mut a = ActionState::new();
        a.sample(0, [500, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(a.analog(0), 500);
        assert_eq!(a.prev_analog(0), 0);
        a.sample(0, [700, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(a.analog(0), 700);
        assert_eq!(a.prev_analog(0), 500);
    }

    #[test]
    fn out_of_range_actions_are_quiet() {
        let mut s = ActionState::new();
        s.sample(u64::MAX, [i32::MAX; MAX_ANALOG]);
        assert!(!s.pressed(64) && !s.just_pressed(200) && !s.just_released(255));
        assert_eq!(s.analog(8), 0);
    }
}
