//! crowd_stands: the crowd on four sides of the sand, each stand its own
//! seats, excitement, travel delay and pan.
//!
//! The arena crowd used to be one global state ([`crate::crowd::Crowd`]):
//! one `bed_gain`, one reaction per event, no place. A knockdown at the north
//! wall sounded identical to one at the south wall, and the far stand cheered
//! in the same sample as the near one. This module gives the four stands
//! (north, east, south, west) their own life:
//!
//! - **Seats**: a seeded share of the crowd per stand in Q16, fixed at
//!   construction (the same seed gives the same house).
//! - **Excitement**: a cue at a stand adds its level there, and `step` leaks
//!   [`LEAK_DIVISOR`] of every stand's excitement to its two neighbours each
//!   tick, so a cheer rolls around the bowl.
//! - **Travel**: the sound of a cue at one stand reaches another after the
//!   sand between them at the speed of sound ([`travel_ms`]): 80 m from the
//!   north wall to the south stand is 233 ms, [`TRAVEL_TICKS`] = 14 ticks at
//!   60 Hz. Nothing is heard before that tick.
//! - **Pan and lift**: the near stand reads +6 dB ([`NEAR_LIFT_Q16`]) and each
//!   stand feeds the existing stereo pan by its side, so a north cheer leans
//!   north.
//!
//! Units: cm and whole ms on the wire, excitement and gains in Q16, ticks at
//! [`TICK_HZ`]. Deterministic: fixed arrays, no heap in `step`, no clocks, so
//! the same seed and the same cue order replay the same reaction times.

use crate::gunfire::SPEED_OF_SOUND_CM_PER_S;

/// Four stands: north, east, south, west.
pub const STANDS: usize = 4;
/// Tick rate the travel ticks are cut at (the game's 60 Hz).
pub const TICK_HZ: u32 = 60;
/// A stand's wall sits this far from the centre of the sand: 40 m.
pub const STAND_CM: i64 = 4_000;
/// Share of a stand's excitement that leaks to each neighbour per tick.
pub const LEAK_DIVISOR: i32 = 10;
/// +6 dB in Q16: 1.99526 * 65536, the near stand's lift.
pub const NEAR_LIFT_Q16: i32 = 130_770;
/// Unity in Q16.
pub const UNITY_Q16: i32 = 65_536;

/// North, east, south, west, in that order.
pub const NAMES: [&str; STANDS] = ["north", "east", "south", "west"];

/// Stand centre in cm on the sand's axes, north first: (x, z).
#[must_use]
pub fn stand_pos_cm(stand: usize) -> [i64; 2] {
    match stand % STANDS {
        0 => [0, -STAND_CM],
        1 => [STAND_CM, 0],
        2 => [0, STAND_CM],
        _ => [-STAND_CM, 0],
    }
}

/// Distance between two stands in cm: 8,000 north to south, 5,656 across the
/// diagonal.
#[must_use]
pub fn stand_distance_cm(a: usize, b: usize) -> i64 {
    let [ax, az] = stand_pos_cm(a);
    let [bx, bz] = stand_pos_cm(b);
    let dx = ax - bx;
    let dz = az - bz;
    // Integer square root keeps this deterministic and cast-free.
    isqrt(u64::try_from(dx * dx + dz * dz).unwrap_or(0))
}

/// Whole milliseconds for sound to cross `distance_cm`.
#[must_use]
pub fn travel_ms(distance_cm: i64) -> u32 {
    let cm = u64::try_from(distance_cm.max(0)).unwrap_or(0);
    u32::try_from(cm * 1_000 / u64::from(SPEED_OF_SOUND_CM_PER_S)).unwrap_or(u32::MAX)
}

/// Whole ticks (rounded up) for sound to cross `distance_cm` at [`TICK_HZ`].
#[must_use]
pub fn travel_ticks(distance_cm: i64) -> u32 {
    let ms = u64::from(travel_ms(distance_cm));
    let ticks = (ms * u64::from(TICK_HZ)).div_ceil(1_000);
    u32::try_from(ticks).unwrap_or(u32::MAX)
}

/// Integer square root, floor.
fn isqrt(value: u64) -> i64 {
    if value == 0 {
        return 0;
    }
    let mut x = value;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + value / x) / 2;
    }
    i64::try_from(x).unwrap_or(i64::MAX)
}

/// What one stand plays on a tick: its excitement as a gain in Q16 and the
/// stereo pan (-Q16 left .. +Q16 right) for its side of the bowl.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StandVoice {
    /// Gain in Q16, already carrying the near-stand lift.
    pub gain_q16: i32,
    /// Pan in Q16: -65536 hard left (west), +65536 hard right (east).
    pub pan_q16: i32,
}

/// The four stands: seats, excitement, the waves still travelling and the
/// tick they land on.
#[derive(Clone, Debug)]
pub struct Stands {
    /// Seeded seats per stand in Q16 (sums to [`UNITY_Q16`]).
    seats_q16: [i32; STANDS],
    /// Excitement per stand in Q16.
    excite_q16: [i32; STANDS],
    /// Level (Q16) of the wave still crossing to each stand.
    pending_q16: [i32; STANDS],
    /// Tick the pending wave lands on; 0 means none.
    pending_at: [u32; STANDS],
    /// Tick the last cue was placed at.
    tick: u32,
    /// The stand the last cue was placed at (the near stand: it answers at
    /// once and carries the +6 dB lift).
    near_stand: usize,
    /// The tick that cue landed on.
    near_tick: u32,
}

impl Stands {
    /// Four stands with a seeded house: stand weights in Q16 that sum to
    /// unity, so the bowl holds one crowd however it is split.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut rng = seed | 1;
        let mut shares = [0_i32; STANDS];
        let mut total = 0_i32;
        for share in &mut shares {
            // xorshift64: deterministic, no clock, no heap.
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let weight = i32::try_from((rng >> 40) % 1_000).unwrap_or(0) + 1;
            *share = weight;
            total += weight;
        }
        let mut seats = [0_i32; STANDS];
        let mut given = 0_i32;
        for stand in 0..STANDS {
            let share = if stand + 1 == STANDS {
                UNITY_Q16 - given
            } else {
                shares[stand] * UNITY_Q16 / total
            };
            seats[stand] = share.max(0);
            given += seats[stand];
        }
        Self {
            seats_q16: seats,
            excite_q16: [0; STANDS],
            pending_q16: [0; STANDS],
            pending_at: [0; STANDS],
            tick: 0,
            near_stand: 0,
            near_tick: 0,
        }
    }

    /// Seeded seat share per stand in Q16.
    #[must_use]
    pub fn seats_q16(&self) -> [i32; STANDS] {
        self.seats_q16
    }

    /// Current excitement per stand in Q16.
    #[must_use]
    pub fn excite_q16(&self) -> [i32; STANDS] {
        self.excite_q16
    }

    /// The tick the last cue was placed at.
    #[must_use]
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// A cue at `stand` on `tick`: the stand itself answers at once, every
    /// other stand hears the wave after the sand between them, and the
    /// excitement lands on the stand that heard it.
    pub fn cue(&mut self, stand: usize, level_q16: i32, tick: u32) {
        let from = stand % STANDS;
        self.tick = tick;
        self.near_stand = from;
        self.near_tick = tick;
        self.excite_q16[from] = (self.excite_q16[from] + level_q16).max(0);
        for to in 0..STANDS {
            if to == from {
                continue;
            }
            self.pending_q16[to] = (self.pending_q16[to] + level_q16).max(0);
            self.pending_at[to] = tick + travel_ticks(stand_distance_cm(from, to));
        }
    }

    /// Advance one tick: leak a tenth of every stand's excitement to its two
    /// neighbours, decay a little, and return what the four stands play this
    /// tick (a wave that lands here now, scaled by the seats; the near stand
    /// of a fresh cue carries the +6 dB lift). Fixed arrays, no heap.
    pub fn step(&mut self, tick: u32) -> [StandVoice; STANDS] {
        self.tick = tick;
        let mut out = [StandVoice::default(); STANDS];
        for (stand, voice) in out.iter_mut().enumerate() {
            if self.pending_at[stand] == tick && self.pending_q16[stand] > 0 {
                let level = self.pending_q16[stand];
                self.excite_q16[stand] = (self.excite_q16[stand] + level).max(0);
                self.pending_q16[stand] = 0;
                self.pending_at[stand] = 0;
                voice.gain_q16 = self.voice_gain_q16(stand, level, false);
                voice.pan_q16 = pan_q16(stand);
            }
            if stand == self.near_stand && tick == self.near_tick {
                // The stand the blow landed at: the loudest of the four.
                let level = self.excite_q16[stand].max(0);
                voice.gain_q16 += self.voice_gain_q16(stand, level, true);
                voice.pan_q16 = pan_q16(stand);
            }
        }
        // 10 % of each stand rolls into both neighbours, and a sixty-fourth
        // decays away; every new value is cut from the pre-tick excitement so
        // one stand's leak cannot land on another's already-updated total.
        let before = self.excite_q16;
        let mut next = [0_i32; STANDS];
        for stand in 0..STANDS {
            let left = (stand + STANDS - 1) % STANDS;
            let right = (stand + 1) % STANDS;
            let leaked_out = 2 * (before[stand] / LEAK_DIVISOR);
            let leaked_in = before[left] / LEAK_DIVISOR + before[right] / LEAK_DIVISOR;
            next[stand] = (before[stand] - leaked_out + leaked_in - before[stand] / 64).max(0);
        }
        self.excite_q16 = next;
        out
    }

    /// Gain of a stand's voice in Q16: the wave level scaled by the seats it
    /// lands on, with the +6 dB near-stand lift when this is the cued stand.
    fn voice_gain_q16(&self, stand: usize, level_q16: i32, near: bool) -> i32 {
        let seats = i64::from(self.seats_q16[stand]);
        let level = i64::from(level_q16.max(0));
        let lift = if near {
            i64::from(NEAR_LIFT_Q16)
        } else {
            65_536
        };
        // One product, then one division: rounding twice loses a per cent.
        i32::try_from(level * seats * lift / (65_536 * 65_536)).unwrap_or(i32::MAX)
    }
}

/// The near stand's lift: 2x (+6 dB) in Q16.
#[must_use]
pub fn near_lift_q16() -> i32 {
    NEAR_LIFT_Q16
}

/// Pan of a stand in Q16: north 0, east +half, south 0, west -half (the bowl
/// is front/back symmetric, so the sides carry the pan).
#[must_use]
pub fn pan_q16(stand: usize) -> i32 {
    match stand % STANDS {
        1 => UNITY_Q16 / 2,
        3 => -(UNITY_Q16 / 2),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SW111 F2P: a knockdown at the north wall is heard in the south stand
    /// 80 m away 233 ms later (8000 cm / 34,300 cm/s), and the near stand is
    /// the loud one (+6 dB).
    #[test]
    fn crowd_wave_arrives_at_the_far_stand_after_233_ms() {
        assert_eq!(stand_distance_cm(0, 2), 8_000, "north to south is 80 m");
        assert_eq!(travel_ms(8_000), 233);
        assert_eq!(travel_ticks(8_000), 14, "233 ms at 60 Hz cuts at 14 ticks");
        let mut stands = Stands::new(2040);
        let seats = stands.seats_q16();
        stands.cue(0, UNITY_Q16, 0);
        let at_cue = stands.step(0);
        assert!(at_cue[0].gain_q16 > 0, "the north stand answers at once");
        let near = i64::from(at_cue[0].gain_q16) * 1_000 / i64::from(seats[0].max(1));
        assert!(
            (1_960..=2_000).contains(&near),
            "the near stand reads +6 dB (x1.995), got {near} per mille"
        );
        for tick in 1..14 {
            let quiet = stands.step(tick);
            assert_eq!(quiet[2].gain_q16, 0, "the south stand is silent at {tick}");
        }
        let heard = stands.step(14);
        assert!(
            heard[2].gain_q16 > 0,
            "the south stand hears the wave at 14 ticks (233 ms)"
        );
        let far = i64::from(heard[2].gain_q16) * 1_000 / i64::from(seats[2].max(1));
        assert!(
            (990..=1_010).contains(&far),
            "the far stand has no lift, got {far} per mille"
        );
        assert_eq!(heard[2].pan_q16, pan_q16(2));
    }

    /// SW111 F2P: a cheer rolls around the bowl: a tenth of each stand leaks
    /// to both neighbours every tick.
    #[test]
    fn stand_excitement_leaks_to_neighbours() {
        let mut stands = Stands::new(7);
        stands.cue(0, 10_000, 0);
        let before = stands.excite_q16();
        assert_eq!(before[0], 10_000);
        assert_eq!(before[1], 0);
        let _ = stands.step(1);
        let after = stands.excite_q16();
        let leak = 10_000 / LEAK_DIVISOR;
        assert_eq!(after[1], leak, "east takes a tenth");
        assert_eq!(after[3], leak, "west takes a tenth");
        assert!(after[0] < before[0], "north loses what it leaked");
        assert_eq!(after[2], 0, "the far stand waits for the wave");
    }

    /// The house is seeded, the four stands sum to one crowd, and a tick
    /// returns a fixed array (no heap, so the audio thread stays allocation
    /// free).
    #[test]
    fn stands_are_seeded_and_step_returns_fixed_arrays() {
        let a = Stands::new(2040);
        let b = Stands::new(2040);
        assert_eq!(a.seats_q16(), b.seats_q16(), "same seed, same house");
        assert_ne!(Stands::new(1).seats_q16(), Stands::new(2).seats_q16());
        assert_eq!(
            a.seats_q16().iter().sum::<i32>(),
            UNITY_Q16,
            "the stands hold one crowd"
        );
        assert!(a.seats_q16().iter().all(|seats| *seats > 0));
        let mut stands = Stands::new(3);
        let voices: [StandVoice; STANDS] = stands.step(0);
        assert_eq!(voices.len(), STANDS);
        assert_eq!(pan_q16(0), 0, "north is centred");
        assert_eq!(pan_q16(1), UNITY_Q16 / 2, "east leans right");
        assert_eq!(pan_q16(3), -UNITY_Q16 / 2, "west leans left");
        assert_eq!(near_lift_q16(), NEAR_LIFT_Q16);
        assert_eq!(NEAR_LIFT_Q16 * 1_000 / UNITY_Q16, 1_995, "+6 dB is x1.995");
    }
}
