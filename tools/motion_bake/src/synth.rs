//! synth: the seed-driven SOMA-77 stagger the committed fixture bakes from.
//!
//! A teacher clip in Kimodo's NPZ layout, authored in code so the bake has a
//! stable source while no GPU is in the loop: `local_rot_mats` (frames, 77, 3,
//! 3), `root_positions` (frames, 3) and `posed_joints` (frames, 77, 3) at 30
//! fps. The clip is a stagger: weight rocks onto the left foot while the right
//! foot steps back twice, then a trailing tail lifts both feet and drifts. The
//! tail is deliberate: the bake must cut it at the last constrained frame.
//!
//! This is authored data, not a simulation: positions are laid out in world
//! space per frame and the local rotations are authored alongside, so bone
//! lengths are not solved. That is what a teacher export is to a bake: data.
//!
//! Determinism: SplitMix64 draws the seed's five amplitudes; every pose value
//! is `+ - * /` and `sqrt` only (no trig, see `rig::turn`). Same seed, same
//! bytes, on every run and every CPU.

use crate::npz::Npy;
use crate::rig::{joint_index, quat_to_mat3, rest_world, soma_offsets, soma_skeleton, turn};
use engine_assets::soma::SOMA77;

/// Teacher frame rate, Kimodo's 30 fps.
pub const SOURCE_FPS: f32 = 30.0;
/// Teacher frames: 35 at 30 fps is 1.1667 s, of which the bake keeps 1.0 s.
pub const SOURCE_FRAMES: usize = 36;
/// Last frame the teacher pins (both feet down); frames after it are tail.
pub const LAST_CONSTRAINED_FRAME: usize = 30;
/// World height over the rest ankle within which a foot counts as planted.
pub const PLANT_BAND: f32 = 0.02;
/// First tail frame: both feet leave the ground and drift.
const TAIL_START: usize = 31;

/// One synthetic teacher clip in Kimodo's array layout.
#[derive(Clone, Debug)]
pub struct TeacherClip {
    /// Frame rate of the clip.
    pub fps: f32,
    /// Seed the amplitudes were drawn from.
    pub seed: i64,
    /// Local rotations, `frames * 77` entries in joint-major order.
    pub local_rot: Vec<[f32; 4]>,
    /// Root (hips) world positions, one per frame.
    pub root_positions: Vec<[f32; 3]>,
    /// World joint positions, `frames * 77` entries in joint-major order.
    pub posed_joints: Vec<[f32; 3]>,
}

/// The seed's five amplitudes: sway, the two lift heights, the step back and
/// the torso lean. SplitMix64 gives the same draws on every run.
struct Amplitudes {
    sway: f32,
    left_lift: f32,
    right_lift: f32,
    step: f32,
    lean: f32,
}

/// SplitMix64: a seed expansion that never aliases two seeds onto one stream.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: i64) -> Self {
        Self { state: seed as u64 }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// The top 24 bits as a fraction in `[0, 1)`; exact in f32.
    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn amplitudes(seed: i64) -> Amplitudes {
    let mut rng = SplitMix64::new(seed);
    Amplitudes {
        sway: 0.08 + 0.04 * rng.unit(),
        left_lift: 0.09 + 0.02 * rng.unit(),
        right_lift: 0.10 + 0.02 * rng.unit(),
        step: 0.16 + 0.08 * rng.unit(),
        lean: 0.02 + 0.02 * rng.unit(),
    }
}

/// Smooth 0..1 ramp.
fn ease(s: f32) -> f32 {
    s * s * (3.0 - 2.0 * s)
}

/// A triangular bump: 0 at `a` and `b`, 1 halfway between, 0 outside.
fn bump(f: f32, a: f32, b: f32) -> f32 {
    if f <= a || f >= b {
        return 0.0;
    }
    let t = (f - a) / (b - a);
    1.0 - (2.0 * t - 1.0).abs()
}

/// Left foot world offset from rest `(y, z)`: one short lift, then planted.
fn left_foot(f: f32, amp: &Amplitudes) -> (f32, f32) {
    let y = amp.left_lift * bump(f, 15.0, 19.0);
    let z = if f <= 15.0 {
        0.0
    } else if f >= 19.0 {
        0.02
    } else {
        0.02 * (f - 15.0) / 4.0
    };
    (y, z)
}

/// Right foot world offset from rest `(y, z)`: two lifts, stepping back
/// [`Amplitudes::step`] each time.
fn right_foot(f: f32, amp: &Amplitudes) -> (f32, f32) {
    let y = amp.right_lift * bump(f, 8.0, 12.0) + amp.right_lift * bump(f, 20.0, 24.0);
    let z = if f <= 8.0 {
        0.0
    } else if f <= 12.0 {
        -amp.step * (f - 8.0) / 4.0
    } else if f <= 20.0 {
        -amp.step
    } else if f <= 24.0 {
        -amp.step - amp.step * (f - 20.0) / 4.0
    } else {
        -2.0 * amp.step
    };
    (y, z)
}

/// The tail lift in meters: 0 while the teacher is constrained, then a lift
/// that starts clear of the plant band, so the bake has a tail to cut.
fn tail(f: f32) -> f32 {
    if (f as usize) < TAIL_START {
        return 0.0;
    }
    0.08 + 0.07 * ease(((f - TAIL_START as f32) / 4.0).min(1.0))
}

/// The tail's drift phase: 0 before the tail, 0..1 across it.
fn tail_phase(f: f32) -> f32 {
    if (f as usize) < TAIL_START {
        return 0.0;
    }
    ease(((f - TAIL_START as f32) / 4.0).min(1.0))
}

/// Bone chains that follow a parent offset, by SOMA joint name: the torso and
/// both arms. The feet are authored in world space instead.
fn follow(name: &str) -> Option<f32> {
    Some(match name {
        "spine1" => 0.98,
        "spine2" => 0.96,
        "chest" => 0.94,
        "neck1" => 0.92,
        "neck2" | "head" | "headend" | "jaw" | "lefteye" | "righteye" => 0.90,
        "leftshoulder" | "rightshoulder" => 0.94,
        _ if name.starts_with("leftarm")
            || name.starts_with("leftforearm")
            || name.starts_with("lefthand") =>
        {
            0.94
        }
        _ if name.starts_with("rightarm")
            || name.starts_with("rightforearm")
            || name.starts_with("righthand") =>
        {
            0.94
        }
        _ => return None,
    })
}

/// True for the arm-and-hand chain of one side: the bones a walk swings.
fn is_arm(name: &str, side: &str) -> bool {
    name.starts_with(side) && (name.contains("arm") || name.contains("hand"))
}

/// The authored local rotation of one joint at frame `f`: a small set of
/// bones turn, everything else holds identity.
fn rotation(name: &str, f: f32, amp: &Amplitudes) -> [f32; 4] {
    let stagger = bump(f, 6.0, 30.0);
    match name {
        "hips" => turn([0.0, 1.0, 0.0], 0.10 * ease(f / (SOURCE_FRAMES - 1) as f32)),
        "spine1" => turn([1.0, 0.0, 0.0], 0.06 * stagger),
        "chest" => turn([0.0, 0.0, 1.0], 0.05 * stagger * (amp.lean / 0.03)),
        "leftleg" => turn([1.0, 0.0, 0.0], -0.16 * bump(f, 10.0, 22.0)),
        "leftshin" => turn([1.0, 0.0, 0.0], 0.24 * bump(f, 10.0, 22.0)),
        "leftfoot" => turn([1.0, 0.0, 0.0], -0.08 * bump(f, 10.0, 22.0)),
        "rightleg" => turn([1.0, 0.0, 0.0], -0.12 * bump(f, 6.0, 26.0)),
        "rightshin" => turn([1.0, 0.0, 0.0], 0.20 * bump(f, 6.0, 26.0)),
        "rightfoot" => turn([1.0, 0.0, 0.0], -0.06 * bump(f, 6.0, 26.0)),
        "leftarm" => turn([1.0, 0.0, 0.0], 0.10 * bump(f, 2.0, 18.0)),
        "rightarm" => turn([1.0, 0.0, 0.0], -0.08 * bump(f, 10.0, 28.0)),
        _ => [0.0, 0.0, 0.0, 1.0],
    }
}

/// Build the seed's stagger: world poses plus local rotations for every frame.
pub fn stagger(seed: i64) -> TeacherClip {
    assert_eq!(
        TAIL_START,
        LAST_CONSTRAINED_FRAME + 1,
        "the tail starts right after the last pinned frame"
    );
    let amp = amplitudes(seed);
    let skeleton = soma_skeleton().expect("the teacher table is parent-first");
    let rest = rest_world(&skeleton, &soma_offsets());
    let hips_at = joint_index(&skeleton, "hips").expect("soma has hips");
    let mut local_rot = Vec::with_capacity(SOURCE_FRAMES * SOMA77.len());
    let mut posed = Vec::with_capacity(SOURCE_FRAMES * SOMA77.len());
    let mut roots = Vec::with_capacity(SOURCE_FRAMES);
    for frame in 0..SOURCE_FRAMES {
        let f = frame as f32;
        let span = (SOURCE_FRAMES - 1) as f32;
        let s = f / span;
        let hips = [amp.sway * ease(s), -0.03 * bump(f, 6.0, 30.0), 0.0];
        let (left_y, left_z) = left_foot(f, &amp);
        let (right_y, right_z) = right_foot(f, &amp);
        let up = tail(f);
        let drift = tail_phase(f);
        let left = [0.0, left_y + up, left_z + 0.03 * drift];
        let right = [0.0, right_y + up, right_z - 0.05 * drift];
        let lean = amp.lean * bump(f, 6.0, 30.0);
        let swing_l = 0.05 * bump(f, 2.0, 18.0);
        let swing_r = -0.04 * bump(f, 10.0, 28.0);
        for (slot, &(name, _)) in SOMA77.iter().enumerate() {
            let base = rest[slot];
            let offset = match name {
                "hips" => hips,
                "leftfoot" | "lefttoebase" | "lefttoeend" => left,
                "rightfoot" | "righttoebase" | "righttoeend" => right,
                "leftleg" => [
                    0.5 * hips[0] + 0.35 * left[0],
                    0.5 * hips[1] + 0.35 * left[1],
                    0.5 * hips[2] + 0.35 * left[2],
                ],
                "leftshin" => [
                    0.7 * hips[0] + 0.7 * left[0],
                    0.7 * hips[1] + 0.7 * left[1],
                    0.7 * hips[2] + 0.7 * left[2],
                ],
                "rightleg" => [
                    0.5 * hips[0] + 0.35 * right[0],
                    0.5 * hips[1] + 0.35 * right[1],
                    0.5 * hips[2] + 0.35 * right[2],
                ],
                "rightshin" => [
                    0.7 * hips[0] + 0.7 * right[0],
                    0.7 * hips[1] + 0.7 * right[1],
                    0.7 * hips[2] + 0.7 * right[2],
                ],
                _ => {
                    let follow = follow(name).unwrap_or(0.0);
                    let mut moved = [follow * hips[0], follow * hips[1], follow * hips[2]];
                    if name == "chest" {
                        moved[2] += lean;
                    }
                    if is_arm(name, "left") {
                        moved[2] += swing_l;
                    }
                    if is_arm(name, "right") {
                        moved[2] += swing_r;
                    }
                    moved
                }
            };
            posed.push([
                base[0] + offset[0],
                base[1] + offset[1],
                base[2] + offset[2],
            ]);
            local_rot.push(rotation(name, f, &amp));
        }
        roots.push([
            rest[hips_at][0] + hips[0],
            rest[hips_at][1] + hips[1],
            rest[hips_at][2] + hips[2],
        ]);
    }
    TeacherClip {
        fps: SOURCE_FPS,
        seed,
        local_rot,
        root_positions: roots,
        posed_joints: posed,
    }
}

impl TeacherClip {
    /// Frame count.
    pub fn frames(&self) -> usize {
        self.root_positions.len()
    }

    /// The arrays a bake reads, in the order [`crate::npz::write_npz`] writes
    /// them: `fps`, `seed`, `local_rot_mats`, `root_positions`, `posed_joints`.
    pub fn arrays(&self) -> Vec<(&'static str, Npy)> {
        let frames = self.frames();
        let joints = SOMA77.len();
        let mut mats = Vec::with_capacity(frames * joints * 9);
        for quat in &self.local_rot {
            let m = quat_to_mat3(*quat);
            for row in m {
                mats.extend_from_slice(&row);
            }
        }
        let mut posed = Vec::with_capacity(frames * joints * 3);
        for position in &self.posed_joints {
            posed.extend_from_slice(position);
        }
        let mut roots = Vec::with_capacity(frames * 3);
        for position in &self.root_positions {
            roots.extend_from_slice(position);
        }
        vec![
            ("fps", crate::npz::f32_scalar(self.fps)),
            ("seed", crate::npz::i64_scalar(self.seed)),
            (
                "local_rot_mats",
                crate::npz::f32_array(&mats, vec![frames, joints, 3, 3]),
            ),
            (
                "root_positions",
                crate::npz::f32_array(&roots, vec![frames, 3]),
            ),
            (
                "posed_joints",
                crate::npz::f32_array(&posed, vec![frames, joints, 3]),
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rig::soma_index;

    #[test]
    fn the_seed_drives_the_clip_and_repeats_exactly() {
        let first = stagger(7);
        let second = stagger(7);
        assert_eq!(first.posed_joints, second.posed_joints);
        assert_eq!(first.local_rot, second.local_rot);
        assert_eq!(first.root_positions, second.root_positions);
        assert_ne!(stagger(8).posed_joints, first.posed_joints, "seeds differ");
        assert_eq!(first.frames(), SOURCE_FRAMES);
        assert_eq!(
            first.posed_joints.len(),
            SOURCE_FRAMES * SOMA77.len(),
            "one world pose per frame"
        );
        // The arrays a bake reads keep Kimodo's shapes.
        let arrays = first.arrays();
        let names: Vec<&str> = arrays.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            vec![
                "fps",
                "seed",
                "local_rot_mats",
                "root_positions",
                "posed_joints"
            ]
        );
        assert_eq!(arrays[2].1.shape, vec![SOURCE_FRAMES, 77, 3, 3]);
        assert_eq!(arrays[3].1.shape, vec![SOURCE_FRAMES, 3]);
        assert_eq!(arrays[4].1.shape, vec![SOURCE_FRAMES, 77, 3]);
    }

    #[test]
    fn the_stagger_plants_both_feet_and_tails_off_the_ground() {
        let clip = stagger(7);
        let left = soma_index("leftfoot").expect("the foot is in the table");
        let right = soma_index("rightfoot").expect("the foot is in the table");
        let rest = rest_world(
            &soma_skeleton().expect("the teacher table is parent-first"),
            &soma_offsets(),
        );
        let planted = |frame: usize, joint: usize| {
            clip.posed_joints[frame * SOMA77.len() + joint][1] - rest[joint][1] <= PLANT_BAND
        };
        // The last pinned frame holds both feet; the tail does not.
        assert!(planted(LAST_CONSTRAINED_FRAME, left));
        assert!(planted(LAST_CONSTRAINED_FRAME, right));
        for frame in LAST_CONSTRAINED_FRAME + 1..SOURCE_FRAMES {
            assert!(!planted(frame, left), "left tail frame {frame}");
            assert!(!planted(frame, right), "right tail frame {frame}");
        }
        // Several plant windows: left lifts once, right lifts twice.
        let mut windows = 0;
        for joint in [left, right] {
            let mut down = false;
            for frame in 0..=LAST_CONSTRAINED_FRAME {
                let now = planted(frame, joint);
                if now && !down {
                    windows += 1;
                }
                down = now;
            }
        }
        assert_eq!(windows, 5, "five plant windows");
        // The right foot steps back: the world z falls by twice the step.
        let step = amplitudes(7).step;
        let last = (SOURCE_FRAMES - 1) * SOMA77.len() + right;
        let landed = clip.posed_joints[last][2] - rest[right][2];
        assert!(
            (landed + 2.0 * step + 0.05).abs() < 1e-5,
            "right foot ends at {landed} after two {step} steps plus the tail drift"
        );
    }
}
