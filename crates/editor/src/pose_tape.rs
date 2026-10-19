//! Mouse-to-joint pose recorder: the integer tape a dragged pose becomes.
//!
//! Purpose: record quantized joint *targets* — never sim state — validate them
//! and replay them deterministically. The UI layer solves IK in floats and
//! hands the resulting milli-turn angles here; an IK solve stays out of scope,
//! so this core carries no float, no NaN and no rig.
//! Consumer: the editor pose-drag panel stages a [`PoseTape`] and replays it
//! into the command service; the headless tests here are the acceptance.
//! Units: angles are milli-turns (`mt`), one full turn is [`TURNS_MT`]
//! (360_000), bounded by [`MAX_ANGLE_MT`]; `joints` is a joint count.
//! Determinism: the tape is pure integers, joints are fixed at construction,
//! and [`PoseTape::digest`] is FNV-1a over (version, joints, frame count,
//! angles in frame order), so the same tape always gives the same digest.
//! Quantize rule: [`PoseTape::record`] takes milli-turn angles already
//! quantized by the caller and refuses any whose absolute value exceeds
//! [`MAX_ANGLE_MT`], so the tape is bounded and cannot carry NaN (integers
//! cannot).
//! Errors: [`PoseError`]. A refused `record` keeps nothing and leaves the tape
//! unchanged; a refused [`PoseTape::trimmed`] leaves the source tape unchanged.
//! Budget: O(joints) per recorded frame, O(frames) per digest and replay, far
//! under one panel frame at [`POSE_MAX_JOINTS`].
//!
//! Source (ideas only): web: Die by the Sword (1998) Move Editor, read
//! 2026-10-19 — frame trims over a recorded move, rebuilt as a bounded integer
//! tape instead of that game's joint targets.

use std::fmt;

use engine_core::Fnv64;

/// Most joints one [`PoseTape`] carries.
pub const POSE_MAX_JOINTS: usize = 64;
/// Tape format version stamped into the digest.
pub const POSE_VERSION_V1: u32 = 1;
/// Angle unit: milli-turns, one full turn is 360_000.
pub const TURNS_MT: i32 = 360_000;
/// Largest legal milli-turn magnitude; `|angle| == MAX_ANGLE_MT` is legal,
/// anything beyond is refused by [`PoseTape::record`].
pub const MAX_ANGLE_MT: i32 = TURNS_MT;

/// One frame: quantized joint angles, milli-turns, one per rig joint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoseFrame {
    angles_mt: Vec<i32>,
}

impl PoseFrame {
    /// The frame's angles, one per joint, in joint order.
    #[must_use]
    pub fn angles_mt(&self) -> &[i32] {
        &self.angles_mt
    }
}

/// A bounded integer pose tape: a fixed joint count plus recorded frames.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PoseTape {
    joints: usize,
    frames: Vec<PoseFrame>,
}

impl PoseTape {
    /// Empty tape for a rig of `joints` joints (`1..=POSE_MAX_JOINTS`).
    ///
    /// # Errors
    /// [`PoseError::ZeroJoints`] for `0`, [`PoseError::TooManyJoints`] above
    /// [`POSE_MAX_JOINTS`].
    pub fn new(joints: usize) -> Result<Self, PoseError> {
        if joints == 0 {
            return Err(PoseError::ZeroJoints);
        }
        if joints > POSE_MAX_JOINTS {
            return Err(PoseError::TooManyJoints { joints });
        }
        Ok(Self {
            joints,
            frames: Vec::new(),
        })
    }

    /// Joint count, fixed at construction.
    #[must_use]
    pub fn joints(&self) -> usize {
        self.joints
    }

    /// Recorded frame count.
    #[must_use]
    pub fn frames(&self) -> usize {
        self.frames.len()
    }

    /// One frame by index, oldest first.
    #[must_use]
    pub fn frame(&self, index: usize) -> Option<&PoseFrame> {
        self.frames.get(index)
    }

    /// Record one frame of quantized milli-turn angles (one per joint).
    ///
    /// # Errors
    /// [`PoseError::LengthMismatch`] when `angles_mt` is not exactly
    /// [`joints`](Self::joints) long, [`PoseError::AngleOutOfRange`] for any
    /// angle whose absolute value exceeds [`MAX_ANGLE_MT`]. On refusal nothing
    /// is kept and the tape is unchanged.
    pub fn record(&mut self, angles_mt: &[i32]) -> Result<(), PoseError> {
        if angles_mt.len() != self.joints {
            return Err(PoseError::LengthMismatch {
                expected: self.joints,
                got: angles_mt.len(),
            });
        }
        let bound = MAX_ANGLE_MT.unsigned_abs();
        for (joint, &angle_mt) in angles_mt.iter().enumerate() {
            if angle_mt.unsigned_abs() > bound {
                return Err(PoseError::AngleOutOfRange { joint, angle_mt });
            }
        }
        self.frames.push(PoseFrame {
            angles_mt: angles_mt.to_vec(),
        });
        Ok(())
    }

    /// Keep frames `begin..end` (half-open, like the Move Editor frame trims).
    ///
    /// # Errors
    /// [`PoseError::BadTrim`] when `begin > end` or `end > frames()`; the
    /// source tape is unchanged either way.
    pub fn trimmed(&self, begin: usize, end: usize) -> Result<Self, PoseError> {
        if begin > end || end > self.frames.len() {
            return Err(PoseError::BadTrim { begin, end });
        }
        Ok(Self {
            joints: self.joints,
            frames: self.frames[begin..end].to_vec(),
        })
    }

    /// The recorded frames as angle slices, oldest first.
    #[must_use = "an iterator does nothing until it is consumed"]
    pub fn replay(&self) -> impl Iterator<Item = &[i32]> {
        self.frames.iter().map(PoseFrame::angles_mt)
    }

    /// FNV-1a 64 over (version, joints, frame count, angles in frame order).
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_u32(POSE_VERSION_V1);
        hasher.write_len(self.joints);
        hasher.write_len(self.frames.len());
        for frame in &self.frames {
            for &angle_mt in &frame.angles_mt {
                hasher.write_i32(angle_mt);
            }
        }
        hasher.finish()
    }
}

/// What refused a pose-tape action. A refusal changes nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoseError {
    /// The joint count was zero.
    ZeroJoints,
    /// The joint count was over [`POSE_MAX_JOINTS`].
    TooManyJoints {
        /// Joint count refused.
        joints: usize,
    },
    /// The frame's angle count did not match the tape's joint count.
    LengthMismatch {
        /// Joints the tape was built with.
        expected: usize,
        /// Angles the frame carried.
        got: usize,
    },
    /// An angle's absolute value exceeded [`MAX_ANGLE_MT`].
    AngleOutOfRange {
        /// Joint index that was out of range.
        joint: usize,
        /// The refused angle in milli-turns.
        angle_mt: i32,
    },
    /// A trim window with `begin > end` or `end` past the last frame.
    BadTrim {
        /// First frame index asked for.
        begin: usize,
        /// Frame index past the last wanted frame.
        end: usize,
    },
}

impl fmt::Display for PoseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroJoints => f.write_str("pose tape needs at least one joint"),
            Self::TooManyJoints { joints } => write!(
                f,
                "pose tape joints {joints} over the {POSE_MAX_JOINTS} joint bound"
            ),
            Self::LengthMismatch { expected, got } => {
                write!(f, "pose frame carries {got} angles, expected {expected}")
            }
            Self::AngleOutOfRange { joint, angle_mt } => write!(
                f,
                "joint {joint} angle {angle_mt} mt over the {MAX_ANGLE_MT} mt bound"
            ),
            Self::BadTrim { begin, end } => {
                write!(f, "pose trim {begin}..{end} is out of range")
            }
        }
    }
}

impl std::error::Error for PoseError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn tape(joints: usize) -> PoseTape {
        PoseTape::new(joints).expect("tape builds")
    }

    fn recorded(angles: &[&[i32]]) -> PoseTape {
        let mut tape = tape(angles.first().map_or(1, |frame| frame.len()));
        for frame in angles {
            tape.record(frame).expect("frame records");
        }
        tape
    }

    #[test]
    fn record_three_frames_then_replay_twice_is_identical() {
        let mut tape = tape(3);
        tape.record(&[0, 1_000, -1_000]).expect("frame 0");
        tape.record(&[90_000, -90_000, 180_000]).expect("frame 1");
        tape.record(&[TURNS_MT, -TURNS_MT, 0]).expect("frame 2");
        assert_eq!(tape.frames(), 3);
        assert_eq!(tape.joints(), 3);

        let first: Vec<Vec<i32>> = tape.replay().map(<[i32]>::to_vec).collect();
        let second: Vec<Vec<i32>> = tape.replay().map(<[i32]>::to_vec).collect();
        assert_eq!(first, second);
        assert_eq!(first.len(), 3);
        assert_eq!(first[1], vec![90_000, -90_000, 180_000]);
        assert_eq!(tape.frame(0).unwrap().angles_mt(), &[0, 1_000, -1_000]);
        assert_eq!(
            tape.frame(2).unwrap().angles_mt(),
            &[TURNS_MT, -TURNS_MT, 0]
        );
        assert!(tape.frame(3).is_none());
        // Replay yields exactly `frames` slices.
        assert_eq!(tape.replay().count(), tape.frames());
    }

    #[test]
    fn trimmed_keeps_the_half_open_window_and_leaves_the_source() {
        let tape = recorded(&[&[0, 0], &[1, -1], &[2, -2], &[3, -3]]);
        let cut = tape.trimmed(1, 3).expect("window is valid");
        assert_eq!(cut.frames(), 2);
        assert_eq!(cut.joints(), 2);
        assert_eq!(cut.frame(0).unwrap().angles_mt(), &[1, -1]);
        assert_eq!(cut.frame(1).unwrap().angles_mt(), &[2, -2]);
        assert_eq!(cut.frame(2), None);
        // The source tape is unchanged.
        assert_eq!(tape.frames(), 4);
        assert_eq!(tape.trimmed(0, 0).expect("empty window").frames(), 0);
        assert_eq!(tape.trimmed(4, 4).expect("tail window").frames(), 0);
        assert_eq!(tape.trimmed(0, 4).expect("whole tape"), tape);
    }

    #[test]
    fn trimmed_refuses_bad_bounds_and_keeps_the_tape() {
        let tape = recorded(&[&[0], &[1]]);
        assert_eq!(
            tape.trimmed(2, 1).unwrap_err(),
            PoseError::BadTrim { begin: 2, end: 1 }
        );
        assert_eq!(
            tape.trimmed(0, 3).unwrap_err(),
            PoseError::BadTrim { begin: 0, end: 3 }
        );
        assert_eq!(tape.frames(), 2);
        assert_eq!(tape.frame(1).unwrap().angles_mt(), &[1]);
    }

    #[test]
    fn record_refuses_wrong_joint_count_and_keeps_frames_at_zero() {
        let mut tape = tape(3);
        assert_eq!(
            tape.record(&[0, 0]).unwrap_err(),
            PoseError::LengthMismatch {
                expected: 3,
                got: 2
            }
        );
        assert_eq!(tape.frames(), 0);
        assert_eq!(
            tape.record(&[0, 0, 0, 0]).unwrap_err(),
            PoseError::LengthMismatch {
                expected: 3,
                got: 4
            }
        );
        assert_eq!(tape.frames(), 0);
        assert!(tape.replay().next().is_none());
    }

    #[test]
    fn record_refuses_angle_over_max_and_keeps_frames_at_zero() {
        let mut tape = tape(2);
        let over = MAX_ANGLE_MT + 1;
        assert_eq!(
            tape.record(&[0, over]).unwrap_err(),
            PoseError::AngleOutOfRange {
                joint: 1,
                angle_mt: over
            }
        );
        assert_eq!(tape.frames(), 0);
        let under = -MAX_ANGLE_MT - 1;
        assert_eq!(
            tape.record(&[under, 0]).unwrap_err(),
            PoseError::AngleOutOfRange {
                joint: 0,
                angle_mt: under
            }
        );
        assert_eq!(tape.frames(), 0);
        // The bound itself is legal, so one full turn round-trips.
        tape.record(&[MAX_ANGLE_MT, -MAX_ANGLE_MT])
            .expect("bound is legal");
        assert_eq!(tape.frames(), 1);
    }

    #[test]
    fn new_refuses_zero_and_too_many_joints() {
        assert_eq!(PoseTape::new(0).unwrap_err(), PoseError::ZeroJoints);
        let over = POSE_MAX_JOINTS + 1;
        assert_eq!(
            PoseTape::new(over).unwrap_err(),
            PoseError::TooManyJoints { joints: over }
        );
        assert!(PoseTape::new(1).is_ok());
        assert!(PoseTape::new(POSE_MAX_JOINTS).is_ok());
    }

    #[test]
    fn digest_moves_when_one_angle_changes_and_is_stable_across_identical_tapes() {
        let first = recorded(&[&[1_000, -2_000], &[3_000, -4_000]]);
        let second = recorded(&[&[1_000, -2_000], &[3_000, -4_000]]);
        assert_eq!(first.digest(), second.digest());

        let one_angle = recorded(&[&[1_000, -2_000], &[3_000, -2_001]]);
        assert_ne!(first.digest(), one_angle.digest());

        // A joint-count change alone moves the digest too.
        let wider = recorded(&[&[1_000, -2_000, 0], &[3_000, -4_000, 0]]);
        assert_ne!(first.digest(), wider.digest());

        // An empty tape of the same rig is a different digest.
        assert_ne!(first.digest(), tape(2).digest());
    }

    #[test]
    fn errors_display_a_reason() {
        for error in [
            PoseError::ZeroJoints,
            PoseError::TooManyJoints { joints: 65 },
            PoseError::LengthMismatch {
                expected: 3,
                got: 2,
            },
            PoseError::AngleOutOfRange {
                joint: 1,
                angle_mt: 360_001,
            },
            PoseError::BadTrim { begin: 2, end: 1 },
        ] {
            assert!(!error.to_string().is_empty(), "{error:?} explains itself");
        }
    }
}
