//! look: presentation-only look channel beside the tape.
//!
//! `engine-input` owns the accumulated eye pose fed by per-tick look deltas; it
//! is presentation state and never enters the
//! [`FighterInput`](crate::fighter::FighterInput) record, the action tape or a
//! tick hash. Units: yaw and pitch in centidegrees, deltas in millidegrees; yaw
//! runs `0..36000` (0..360 deg) and pitch stays inside `+/-` the clamp.
//! Integer math only, no float, no heap: one delta track replays
//! byte-identically everywhere.

use engine_core::hash_bytes;

/// Row group a refused look parameter names (this crate's typed-refusal style).
const ROW: &str = "FV2";

/// Accepted per-tick delta half-range, millidegrees (6 deg). A delta outside it
/// refuses, never clamps: the table rate is a gain, so the bounded delta is what
/// bounds the yaw rate.
pub const MAX_DELTA_MILLI_DEG: i32 = 6_000;

/// Yaw wrap width in centidegrees: a full turn, `0..YAW_WRAP_CENTIDEG`.
pub const YAW_WRAP_CENTIDEG: i32 = 36_000;

/// Look tuning keys: one row set per table.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LookRates {
    /// Yaw gain in permille of the raw delta, applied per tick.
    pub yaw_permille_per_tick: i32,
    /// Pitch clamp in centidegrees, either side of level.
    pub pitch_clamp_centideg: i32,
}

/// The fps-view look table as it starts. Owner: the parsed
/// `content/design/fps-view.rows` table takes these keys over once that file
/// lands; until then this const is their single owner.
pub const LOOK_RATES: LookRates = LookRates {
    yaw_permille_per_tick: 40,
    pitch_clamp_centideg: 8_000,
};

/// Accumulated presentation look pose for one eye.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LookState {
    /// Eye yaw in centidegrees, wrapped into `0..YAW_WRAP_CENTIDEG`.
    pub yaw_centideg: i32,
    /// Eye pitch in centidegrees, clamped to the table's pitch clamp.
    pub pitch_centideg: i32,
}

impl LookState {
    /// Neutral pose: dead ahead, yaw 0 and pitch 0.
    pub const fn zero() -> Self {
        Self {
            yaw_centideg: 0,
            pitch_centideg: 0,
        }
    }

    /// Neutral pose once the rates are proven good; the pose keeps no copy of
    /// them (`LOOK_RATES` owns them, [`LookState::rates`] reads that).
    /// Errors: [`LookError::BadParam`] naming the refused key.
    pub fn new(rates: LookRates) -> Result<Self, LookError> {
        validate_rates(rates)?;
        Ok(Self::zero())
    }

    /// Fold one tick of look deltas into the pose.
    ///
    /// `dx_milli_deg` turns the eye right (yaw grows, wrapping at a full turn);
    /// `dy_milli_deg` raises it (pitch grows, clamped at the table bound). Both
    /// refuse with [`LookError::BadParam`] outside
    /// `+/-`[`MAX_DELTA_MILLI_DEG`] and leave the pose untouched.
    pub fn apply_deltas(&mut self, dx_milli_deg: i32, dy_milli_deg: i32) -> Result<(), LookError> {
        let rates = validate_rates(LOOK_RATES)?;
        let dx = bounded_delta(dx_milli_deg, "dx_milli_deg")?;
        let dy = bounded_delta(dy_milli_deg, "dy_milli_deg")?;
        // |delta| <= 6000 and rate <= 1000, so neither product can overflow.
        let yaw_step = dx * rates.yaw_permille_per_tick / 1000;
        let pitch_step = dy * rates.yaw_permille_per_tick / 1000;
        let clamp = rates.pitch_clamp_centideg;
        self.pitch_centideg = (self.pitch_centideg + pitch_step).clamp(-clamp, clamp);
        self.yaw_centideg = (self.yaw_centideg + yaw_step).rem_euclid(YAW_WRAP_CENTIDEG);
        Ok(())
    }

    /// The table this channel scales by: [`LOOK_RATES`] until the parsed one lands.
    pub fn rates(&self) -> LookRates {
        LOOK_RATES
    }

    /// Canonical little-endian pose bytes: yaw then pitch, 8 bytes, no heap.
    pub fn to_bytes(&self) -> [u8; 8] {
        let mut bytes = [0u8; 8];
        bytes[0..4].copy_from_slice(&self.yaw_centideg.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.pitch_centideg.to_le_bytes());
        bytes
    }

    /// Deterministic replay digest: `engine-core` hashing over [`LookState::to_bytes`].
    pub fn digest(&self) -> u64 {
        hash_bytes(&self.to_bytes())
    }
}

/// Table rows stay inside the ranges the channel's integer math is proven for.
fn validate_rates(rates: LookRates) -> Result<LookRates, LookError> {
    if !(1..=1000).contains(&rates.yaw_permille_per_tick) {
        return Err(bad("yaw_permille_per_tick"));
    }
    if !(1..=18_000).contains(&rates.pitch_clamp_centideg) {
        return Err(bad("pitch_clamp_centideg"));
    }
    Ok(rates)
}

/// Range-gated raw delta, passed through untouched when it is legal.
fn bounded_delta(delta: i32, what: &'static str) -> Result<i32, LookError> {
    if !(-MAX_DELTA_MILLI_DEG..=MAX_DELTA_MILLI_DEG).contains(&delta) {
        return Err(bad(what));
    }
    Ok(delta)
}

/// One refusal, with this crate's row group attached.
fn bad(what: &'static str) -> LookError {
    LookError::BadParam { row: ROW, what }
}

/// Look-channel contract failures: bounded `&'static str` diagnostics, never
/// panics. Determinism: `Copy` payloads, no state, stable across platforms.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum LookError {
    /// Look parameter outside its table range.
    BadParam {
        /// Table row group the parameter belongs to.
        row: &'static str,
        /// Refused key or delta name.
        what: &'static str,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fighter::{Buttons, FighterId, FighterInput, ATTACK, BLOCK};

    /// 600 ticks: yaw sweeps both ways and wraps, pitch ramps then unwinds.
    fn track_600() -> Vec<(i32, i32)> {
        let dy = |t: i32| if t % 3 == 0 { 4_000 } else { -1_200 };
        (0..600).map(|t| ((t % 120) * 50 - 3_000, dy(t))).collect()
    }

    /// One pose driven over a track, with every tick's bytes kept.
    fn drive(track: &[(i32, i32)]) -> (LookState, Vec<[u8; 8]>) {
        let mut look = LookState::new(LOOK_RATES).expect("table rates validate");
        let mut bytes = Vec::with_capacity(track.len());
        for &(dx, dy) in track {
            look.apply_deltas(dx, dy).expect("scripted delta in range");
            bytes.push(look.to_bytes());
        }
        (look, bytes)
    }

    #[test]
    fn look_channel_same_tape_replays_byte_identical_x2() {
        let track = track_600();
        let (first, bytes) = drive(&track);
        let (second, again) = drive(&track);
        assert_eq!(bytes, again, "byte track diverged between runs");
        assert_eq!(first.to_bytes(), second.to_bytes(), "pose bytes diverged");
        assert_eq!(first.digest(), second.digest(), "pose digest diverged");
        // Receipt: the u64 both 600-tick runs land on (replay evidence).
        println!("look digest x2: {:x} {:x}", first.digest(), second.digest());
        // The compares above only mean something if the pose moved.
        assert!(
            first.yaw_centideg != 0 && first.pitch_centideg != 0,
            "pose never moved: {first:?}"
        );
        assert_ne!(first.digest(), drive(&[(999, -999); 600]).0.digest());
    }

    #[test]
    fn look_deltas_clamp_and_wrap_at_table_rates() {
        let full = MAX_DELTA_MILLI_DEG;
        let mut look = LookState::new(LOOK_RATES).expect("table rates validate");
        assert_eq!(look.rates(), LOOK_RATES);
        // Table gain: 1000 millideg is 40 centideg, a full-stick delta 240, so
        // 150 ticks turn the eye once and one more tick starts the next turn.
        look.apply_deltas(1_000, 0).expect("in range");
        assert_eq!((look.yaw_centideg, look.pitch_centideg), (40, 0));
        // A full-stick tick turns the eye 240 centideg and lifts it 240, so
        // pitch pins at the table bound while yaw keeps turning.
        let mut turn = LookState::zero();
        let mut seen = Vec::new();
        for _ in 0..151 {
            turn.apply_deltas(full, full).expect("in range");
            seen.push((turn.yaw_centideg, turn.pitch_centideg));
        }
        assert_eq!(seen[0], (240, 240));
        assert_eq!(seen[33], (8_160, 8_000), "pitch pins at the table bound");
        assert_eq!(seen[149], (0, 8_000), "yaw wraps after 150 ticks");
        assert_eq!(seen[150], (240, 8_000), "the next turn starts at 240");
        turn.apply_deltas(-full, -full).expect("in range");
        assert_eq!(turn.yaw_centideg, 0, "yaw must wrap back down to zero");
        turn.apply_deltas(-full, -full).expect("in range");
        assert_eq!(turn.yaw_centideg, 35_760, "yaw must wrap below zero too");
        for _ in 0..80 {
            turn.apply_deltas(0, -full).expect("in range");
        }
        assert_eq!(turn.pitch_centideg, -8_000, "pitch must pin the other way");
        // A delta past the accepted range refuses and leaves the pose alone.
        let before = turn.to_bytes();
        for (dx, dy, what) in [
            (full + 1, 0, "dx_milli_deg"),
            (-full - 1, 0, "dx_milli_deg"),
            (0, full + 1, "dy_milli_deg"),
            (i32::MIN, i32::MIN, "dx_milli_deg"),
        ] {
            assert_eq!(turn.apply_deltas(dx, dy), Err(bad(what)));
        }
        assert_eq!(turn.to_bytes(), before, "a refused delta moved the pose");
        // A table outside the proven ranges refuses before any tick runs.
        let table = |yaw, clamp| LookRates {
            yaw_permille_per_tick: yaw,
            pitch_clamp_centideg: clamp,
        };
        for (rates, what) in [
            (table(0, 8_000), "yaw_permille_per_tick"),
            (table(1_001, 8_000), "yaw_permille_per_tick"),
            (table(40, 0), "pitch_clamp_centideg"),
            (table(40, 18_001), "pitch_clamp_centideg"),
        ] {
            assert_eq!(LookState::new(rates), Err(bad(what)));
        }
    }

    #[test]
    fn look_state_never_touches_fighter_input_bytes() {
        let input = FighterInput {
            tick: 7,
            fighter: FighterId::new(1).expect("slot inside the cap"),
            mx: 100,
            mz: -100,
            facing: 9_000,
            buttons: Buttons(ATTACK | BLOCK),
        };
        let input_bytes = || {
            let mut bytes = Vec::new();
            input.append_bytes(&mut bytes);
            bytes
        };
        let before = input_bytes();
        assert_eq!(before.len(), 10, "the fighter record stays 10 bytes");
        let (look, _) = drive(&track_600());
        assert_eq!(input_bytes(), before, "look moved the record bytes");
        // The pose is its own value: no field and no byte of the record path.
        assert_eq!(std::mem::size_of::<LookState>(), 8);
        assert_ne!(look.to_bytes(), [0u8; 8]);
    }
}
