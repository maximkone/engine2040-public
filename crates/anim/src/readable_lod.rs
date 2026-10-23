//! readable_lod: facing arrow plus lean at crowd LOD.
//!
//! Owner: `engine-anim`; status: presentation overlay. Consumer: the arena HUD
//! and capture read [`ReadableCrowd::frame`] per fighter. The overlay drives
//! one [`Readable`] per LOD lane, re-evaluates a lane on the ticks its [`Tier`]
//! owns, and holds the drawn arrow plus lean on the ticks in between. That hold
//! is the readability rule: a far lane updates every 8 ticks, so a HUD that
//! read the overlay only on eval ticks would strobe the arrow to rest and back.
//!
//! Simulation never reads this module: it takes tiers and motion lanes by
//! reference and writes no lane back, so the tick bytes and the golden hash
//! cannot move. Units: meters per second, radians; fixed 60 Hz ticks.
//! Determinism: `+ - * /` plus `sqrt` (through [`Readable`]), agent order, no
//! clock; the same script twice draws identical frames. Errors:
//! [`ReadableLodError`].

use super::lod::{Tier, MAX_ACTORS};
use super::readable::{Readable, ReadableError, ARROW_MAX_TURN, LEAN_SLEW};
use core::fmt;

/// Lean (radians) that still reads as a forward lean at any distance: a moving
/// lane in the far ring must draw at least this.
pub const LEAN_VISIBLE_MIN: f32 = LEAN_SLEW;

/// Largest arrow turn one drawn tick may show: the [`ARROW_MAX_TURN`] slew
/// seen as a unit-arrow chord, which the Taylor step bounds by `d + d^3/4`.
pub const ARROW_DRAW_CHORD: f32 =
    ARROW_MAX_TURN + ARROW_MAX_TURN * ARROW_MAX_TURN * ARROW_MAX_TURN * 0.25;

/// One fighter's motion input for a tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MotionLane {
    /// Planar velocity `[x, z]` in meters per second.
    pub velocity: [f32; 2],
    /// Body facing, unit `[x, z]`.
    pub facing: [f32; 2],
}

/// The frame the HUD draws for one fighter this tick.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DrawnFrame {
    /// Unit `[x, z]` facing arrow.
    pub arrow: [f32; 2],
    /// Forward lean in radians.
    pub lean: f32,
    /// Tier this frame was drawn at.
    pub tier: Tier,
    /// True when this tick re-evaluated the lane; false when the tier held it.
    pub fresh: bool,
}

/// Bounded overlay failures: typed, never panics.
#[derive(Clone, PartialEq, Debug)]
pub enum ReadableLodError {
    /// Fighter count past [`MAX_ACTORS`].
    TooManyActors(usize),
    /// One lane count disagrees with the overlay's own.
    LengthMismatch {
        /// Overlay lane count.
        expected: usize,
        /// Lane count handed in.
        got: usize,
    },
    /// A motion lane was refused; the tick steps no lane.
    Readable(ReadableError),
}

impl fmt::Display for ReadableLodError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadableLodError::TooManyActors(n) => {
                write!(f, "readable lod: {n} fighters past {MAX_ACTORS}")
            }
            ReadableLodError::LengthMismatch { expected, got } => {
                write!(f, "readable lod: lane count {got} differs from {expected}")
            }
            ReadableLodError::Readable(error) => write!(f, "readable lod: {error}"),
        }
    }
}

impl std::error::Error for ReadableLodError {}

/// Facing arrow plus lean, one lane per LOD fighter.
#[derive(Clone, PartialEq, Debug)]
pub struct ReadableCrowd {
    lanes: Vec<Readable>,
    drawn: Vec<DrawnFrame>,
}

impl ReadableCrowd {
    /// New overlay for `len` fighters (at most [`MAX_ACTORS`]); every arrow
    /// starts on `facing`, every lean at rest.
    pub fn new(len: usize, facing: [f32; 2]) -> Result<Self, ReadableLodError> {
        if len > MAX_ACTORS {
            return Err(ReadableLodError::TooManyActors(len));
        }
        let lanes: Vec<Readable> = (0..len).map(|_| Readable::new(facing)).collect();
        let arrow = Readable::new(facing).arrow();
        let drawn = vec![
            DrawnFrame {
                arrow,
                lean: 0.0,
                tier: Tier::Full,
                fresh: false,
            };
            len
        ];
        Ok(Self { lanes, drawn })
    }

    /// Fighter count.
    pub fn len(&self) -> usize {
        self.lanes.len()
    }

    /// True when no lane is driven.
    pub fn is_empty(&self) -> bool {
        self.lanes.is_empty()
    }

    /// The frame to draw for `agent`; `None` past the lane count.
    pub fn frame(&self, agent: usize) -> Option<DrawnFrame> {
        self.drawn.get(agent).copied()
    }

    /// One presentation tick: `tiers` says how often each lane re-evaluates,
    /// `motion` carries one lane per fighter. A lane whose tier does not
    /// update this tick keeps the arrow and lean it drew last tick, so the far
    /// ring never strobes. A refused motion lane rejects the whole tick and
    /// draws nothing new: the crowd never half-steps.
    pub fn tick(
        &mut self,
        tick: u64,
        tiers: &[Tier],
        motion: &[MotionLane],
    ) -> Result<(), ReadableLodError> {
        let len = self.lanes.len();
        if tiers.len() != len {
            return Err(ReadableLodError::LengthMismatch {
                expected: len,
                got: tiers.len(),
            });
        }
        if motion.len() != len {
            return Err(ReadableLodError::LengthMismatch {
                expected: len,
                got: motion.len(),
            });
        }
        for lane in motion {
            Readable::checked_speed(lane.velocity[0], lane.velocity[1], lane.facing)
                .map_err(ReadableLodError::Readable)?;
        }
        for (agent, slot) in self.drawn.iter_mut().enumerate() {
            let tier = tiers[agent];
            slot.tier = tier;
            slot.fresh = tier.updates_this_tick(tick, agent);
            if !slot.fresh {
                continue;
            }
            let lane = motion[agent];
            // Validated above, so this cannot refuse; the hold keeps the frame
            // rather than panicking on an impossible path.
            if let Ok(frame) =
                self.lanes[agent].update(lane.velocity[0], lane.velocity[1], lane.facing)
            {
                slot.arrow = frame.arrow;
                slot.lean = frame.lean;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crowd_scale::tape_hash;
    use crate::lod::assign_tiers;
    use crate::readable::{arrow_chord, LEAN_MAX};
    use core::f32::consts::FRAC_1_SQRT_2;

    const DIAGONAL: f32 = FRAC_1_SQRT_2;

    /// Eight unit headings, so a scripted crowd turns without transcendentals.
    const HEADINGS: [[f32; 2]; 8] = [
        [1.0, 0.0],
        [DIAGONAL, DIAGONAL],
        [0.0, 1.0],
        [-DIAGONAL, DIAGONAL],
        [-1.0, 0.0],
        [-DIAGONAL, -DIAGONAL],
        [0.0, -1.0],
        [DIAGONAL, -DIAGONAL],
    ];

    /// The 64-fighter tape with tiers on: 8 near (full), 20 half, 20 quarter,
    /// 16 far, all small on screen so distance decides the tier.
    fn tape() -> (Vec<f32>, Vec<f32>, Vec<Tier>) {
        let mut distances = Vec::with_capacity(MAX_ACTORS);
        for agent in 0..MAX_ACTORS {
            let index = f32::from(u16::try_from(agent).unwrap_or(u16::MAX));
            let distance = match agent {
                0..=7 => 5.0,
                8..=27 => 15.0 + index,
                28..=47 => 35.0 + index,
                _ => 65.0 + index,
            };
            distances.push(distance);
        }
        let screens = vec![0.002f32; MAX_ACTORS];
        let mut tiers = vec![Tier::Far; MAX_ACTORS];
        assign_tiers(&distances, &screens, &mut tiers).expect("the 64-fighter tape assigns");
        (distances, screens, tiers)
    }

    /// Scripted crowd motion: every agent runs 4 to 7 m/s and takes one new
    /// heading every 16 ticks, so the arrow has to steer and the lean has to
    /// settle in every tier.
    fn lanes(tick: u64) -> Vec<MotionLane> {
        let step = usize::try_from(tick / 16).unwrap_or(0);
        (0..MAX_ACTORS)
            .map(|agent| {
                let heading = HEADINGS[(step + agent) % HEADINGS.len()];
                let speed = 4.0 + f32::from(u8::try_from(agent % 4).unwrap_or(0));
                MotionLane {
                    velocity: [heading[0] * speed, heading[1] * speed],
                    facing: heading,
                }
            })
            .collect()
    }

    /// Every drawn frame for one scripted run, in tick then agent order.
    fn run(tiers: &[Tier], ticks: u64) -> Vec<DrawnFrame> {
        let mut crowd = ReadableCrowd::new(MAX_ACTORS, [0.0, 1.0]).expect("64 lanes fit");
        let mut frames = Vec::with_capacity(MAX_ACTORS * usize::try_from(ticks).unwrap_or(0));
        for tick in 0..ticks {
            crowd
                .tick(tick, tiers, &lanes(tick))
                .expect("scripted lanes are accepted");
            for agent in 0..MAX_ACTORS {
                frames.push(crowd.frame(agent).expect("every lane is drawn"));
            }
        }
        frames
    }

    /// AM22 accept: with tiers on, the far ring still draws a unit arrow and a
    /// visible lean every tick, and neither pops between draws.
    #[test]
    fn lod_readable_arrow_and_lean_visible_in_the_far_ring() {
        let (_distances, _screens, tiers) = tape();
        let mut seen = [false; 4];
        for tier in &tiers {
            seen[match tier {
                Tier::Full => 0,
                Tier::Half => 1,
                Tier::Quarter => 2,
                Tier::Far => 3,
            }] = true;
        }
        assert!(seen.iter().all(|&crossed| crossed), "tiers on: {seen:?}");
        let frames = run(&tiers, 96);
        let mut far_turns = 0.0f32;
        for (index, frame) in frames.iter().enumerate() {
            let tick = u64::try_from(index / MAX_ACTORS).unwrap_or(0);
            let agent = index % MAX_ACTORS;
            let len2 = frame.arrow[0] * frame.arrow[0] + frame.arrow[1] * frame.arrow[1];
            assert!(
                (len2 - 1.0).abs() < 1e-3,
                "agent {agent} arrow is unit at tick {tick}"
            );
            assert!(
                (0.0..=LEAN_MAX).contains(&frame.lean),
                "agent {agent} lean {} is out of range at tick {tick}",
                frame.lean
            );
            if index < MAX_ACTORS {
                continue;
            }
            let previous = frames[index - MAX_ACTORS];
            let step = arrow_chord(previous.arrow, frame.arrow);
            assert!(
                step <= ARROW_DRAW_CHORD + 1e-4,
                "agent {agent} arrow popped {step} at tick {tick}"
            );
            assert!(
                (frame.lean - previous.lean).abs() <= LEAN_SLEW + 1e-6,
                "agent {agent} lean jumped at tick {tick}"
            );
            if tiers[agent] != Tier::Far {
                continue;
            }
            let due = tiers[agent].updates_this_tick(tick, agent);
            assert_eq!(
                frame.fresh, due,
                "far agent {agent} freshness follows its tier at tick {tick}"
            );
            far_turns += step;
            if !due {
                assert_eq!(
                    frame.arrow, previous.arrow,
                    "a held arrow is the same arrow"
                );
                assert_eq!(frame.lean, previous.lean, "a held lean is the same lean");
            }
            if tick >= 40 {
                assert!(
                    frame.lean >= LEAN_VISIBLE_MIN,
                    "far agent {agent} draws lean {} at tick {tick}",
                    frame.lean
                );
            }
        }
        assert!(
            far_turns > 0.5,
            "far arrows steer with the crowd, turned {far_turns} of chord"
        );
    }

    /// AM22 accept: the nearest eight draw exactly what a full-rate overlay
    /// would draw, every tick.
    #[test]
    fn lod_readable_nearest_eight_match_full_rate_poses() {
        let (_distances, _screens, tiers) = tape();
        let mut crowd = ReadableCrowd::new(MAX_ACTORS, [0.0, 1.0]).expect("64 lanes fit");
        let mut reference: Vec<Readable> =
            (0..MAX_ACTORS).map(|_| Readable::new([0.0, 1.0])).collect();
        let mut matched = 0usize;
        for tick in 0..64u64 {
            let script = lanes(tick);
            crowd
                .tick(tick, &tiers, &script)
                .expect("scripted lanes are accepted");
            for (agent, state) in reference.iter_mut().enumerate() {
                let lane = script[agent];
                let full = state
                    .update(lane.velocity[0], lane.velocity[1], lane.facing)
                    .expect("scripted lane is accepted");
                if tiers[agent] != Tier::Full {
                    continue;
                }
                let drawn = crowd.frame(agent).expect("every lane is drawn");
                assert_eq!(
                    drawn.arrow, full.arrow,
                    "nearest agent {agent} arrow matches full rate at tick {tick}"
                );
                assert_eq!(
                    drawn.lean, full.lean,
                    "nearest agent {agent} lean matches full rate at tick {tick}"
                );
                assert!(drawn.fresh, "a full-rate lane draws fresh every tick");
                matched += 1;
            }
        }
        assert_eq!(matched, 64 * 8, "eight nearest over 64 ticks");
    }

    /// AM22 accept, sim bytes x2: the same script draws the same frames twice,
    /// and the overlay writes no sim lane.
    #[test]
    fn lod_readable_draws_x2_and_leaves_the_sim_bytes_alone() {
        let (distances, screens, tiers) = tape();
        let before = tape_hash(&distances, &screens);
        let first = run(&tiers, 64);
        let second = run(&tiers, 64);
        assert_eq!(first, second, "the same script draws the same frames twice");
        assert_eq!(
            tape_hash(&distances, &screens),
            before,
            "the overlay writes no crowd lane"
        );
    }

    /// A bad motion lane refuses the whole tick: no lane half-steps.
    #[test]
    fn lod_readable_refuses_a_bad_lane_before_it_steps_any() {
        let (_distances, _screens, tiers) = tape();
        let mut crowd = ReadableCrowd::new(MAX_ACTORS, [0.0, 1.0]).expect("64 lanes fit");
        for tick in 0..8u64 {
            crowd
                .tick(tick, &tiers, &lanes(tick))
                .expect("scripted lanes are accepted");
        }
        let before: Vec<DrawnFrame> = (0..MAX_ACTORS)
            .map(|agent| crowd.frame(agent).expect("every lane is drawn"))
            .collect();
        let mut bad = lanes(8);
        bad[MAX_ACTORS - 1].velocity[0] = f32::NAN;
        assert!(matches!(
            crowd.tick(8, &tiers, &bad),
            Err(ReadableLodError::Readable(ReadableError::NonFiniteInput(
                "mx"
            )))
        ));
        let after: Vec<DrawnFrame> = (0..MAX_ACTORS)
            .map(|agent| crowd.frame(agent).expect("every lane is drawn"))
            .collect();
        assert_eq!(after, before, "a refused tick draws nothing new");
        assert!(matches!(
            crowd.tick(9, &tiers, &bad[..MAX_ACTORS - 1]),
            Err(ReadableLodError::LengthMismatch { .. })
        ));
        assert!(matches!(
            ReadableCrowd::new(MAX_ACTORS + 1, [0.0, 1.0]),
            Err(ReadableLodError::TooManyActors(_))
        ));
    }
}
