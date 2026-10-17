//! Ragdoll tape of the golden matrix (WEB3): one fixed fall of a 17-bone
//! humanoid ragdoll onto a floor box, 300 fixed steps, every body transform
//! digested every tick.
//!
//! Consumer: the `ragdoll_line` bin prints [`RagdollTape::rows`]; the
//! golden-matrix workflow runs it on every runner and
//! `tools/golden-matrix.mjs --ragdoll` compares the files, naming the first
//! tick and body that differ. Data owner: this module owns the fixture (rig,
//! capsules, spawn, one hit). Units: meters, seconds, newton-seconds, ticks at
//! [`engine_core::FIXED_DT`]. The digest is [`engine_core::Fnv64`] over
//! `f32::to_bits`, so a one-ULP difference in any transform moves it. The tape
//! only calls the public [`Ragdoll`] API; it never touches simulation code.

use engine_anim::contract::{Joint, LocalPose, Skeleton};
use engine_core::{Fnv64, FIXED_DT};
use engine_physics::ragdoll::{BoneShape, Error as RagdollError, Ragdoll};
use std::fmt::Write as _;

/// Fixed steps of the fall (5 s at 60 Hz: drop, hit, tumble, rest).
pub const TICKS: u32 = 300;
/// Step on which the single hit lands, while the body is already on the floor.
pub const HIT_TICK: u32 = 40;
/// Bone the hit lands on: the chest.
pub const HIT_BONE: usize = 2;
/// Capsule radius of every bone, meters.
pub const BONE_RADIUS: f32 = 0.06;
/// Capsule cylinder half-height of every bone, meters.
pub const BONE_HALF_HEIGHT: f32 = 0.1;

/// [`RagdollTape::digest`] of the standard fall. x86-64 Linux, x86-64 Windows,
/// aarch64 Linux and wasm32-wasip1 all print it (the golden-matrix workflow
/// compares them). It moves only with a Rapier or ragdoll change, and a new
/// value needs a fresh four-machine run.
pub const GOLDEN_RAGDOLL: u64 = 0x8097_ccaf_b674_0acb;

/// The rig: parent, name (`knee`/`elbow` become hinges) and local offset in
/// the parent frame. The pelvis offset is replaced by the spawn position.
const RIG: [(Option<usize>, &str, [f32; 3]); 17] = [
    (None, "pelvis", [0.0, 0.0, 0.0]),
    (Some(0), "spine", [0.0, 0.2, 0.0]),
    (Some(1), "chest", [0.0, 0.25, 0.0]),
    (Some(2), "neck", [0.0, 0.25, 0.0]),
    (Some(3), "head", [0.0, 0.12, 0.0]),
    (Some(2), "shoulder_l", [-0.2, 0.2, 0.0]),
    (Some(5), "elbow_l", [0.0, -0.3, 0.0]),
    (Some(6), "wrist_l", [0.0, -0.28, 0.0]),
    (Some(2), "shoulder_r", [0.2, 0.2, 0.0]),
    (Some(8), "elbow_r", [0.0, -0.3, 0.0]),
    (Some(9), "wrist_r", [0.0, -0.28, 0.0]),
    (Some(0), "hip_l", [-0.1, -0.05, 0.0]),
    (Some(11), "knee_l", [0.0, -0.45, 0.0]),
    (Some(12), "ankle_l", [0.0, -0.42, 0.0]),
    (Some(0), "hip_r", [0.1, -0.05, 0.0]),
    (Some(14), "knee_r", [0.0, -0.45, 0.0]),
    (Some(15), "ankle_r", [0.0, -0.42, 0.0]),
];

/// Pelvis tilt at spawn: 45 degrees about Z, so the fall lands on one side and
/// every limb meets the floor and its neighbours. Normalized by the ragdoll.
const SPAWN_TILT: [f32; 4] = [0.0, 0.0, 0.382_683_43, 0.923_879_5];

/// The knobs of the fall. [`Fixture::STANDARD`] is what the matrix runs; the
/// tests move one knob at a time to show the digest follows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fixture {
    /// World position of the pelvis at spawn, meters.
    pub spawn: [f32; 3],
    /// Impulse of the hit on [`HIT_BONE`] at [`HIT_TICK`], newton-seconds. The
    /// bones weigh a few grams, so a few hundredths is already a hard shove.
    pub hit: [f32; 3],
}

impl Fixture {
    /// The fall the golden matrix compares across machines.
    pub const STANDARD: Self = Self {
        spawn: [0.0, 2.0, 0.0],
        hit: [0.02, 0.01, -0.015],
    };
}

/// What one fall leaves behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RagdollTape {
    /// Digest of every body after each step, skeleton order: `bodies[tick][bone]`.
    pub bodies: Vec<Vec<u64>>,
    /// Contact pairs the narrow phase reported over the whole fall.
    pub contacts: u64,
    /// [`Ragdoll::state_hash`] after the last step: it also folds velocities
    /// and the solver counters, which the transform digests do not.
    pub final_state: u64,
}

impl RagdollTape {
    /// One digest for the whole fall: every body digest in tick order, then the
    /// contact count and the final state hash.
    #[must_use]
    pub fn digest(&self) -> u64 {
        let mut hash = Fnv64::new();
        for tick in &self.bodies {
            for body in tick {
                hash.write_u64(*body);
            }
        }
        hash.write_u64(self.contacts);
        hash.write_u64(self.final_state);
        hash.finish()
    }

    /// The text `ragdoll_line` prints for `host`: a summary line, then one line
    /// per tick listing the body digests in skeleton order. This is the format
    /// `tools/golden-matrix.mjs --ragdoll` reads:
    ///
    /// ```text
    /// <host> ragdoll=<16hex> ticks=<n> bodies=<n> contacts=<n> final_state=<16hex>
    /// <host> tick=<t> bodies=<16hex>,<16hex>,...
    /// ```
    #[must_use]
    pub fn rows(&self, host: &str) -> String {
        let bodies = self.bodies.first().map_or(0, Vec::len);
        let mut text = String::new();
        // Writing into a `String` cannot fail.
        let _ = writeln!(
            text,
            "{host} ragdoll={:016x} ticks={} bodies={bodies} contacts={} final_state={:016x}",
            self.digest(),
            self.bodies.len(),
            self.contacts,
            self.final_state
        );
        for (tick, row) in self.bodies.iter().enumerate() {
            let list: Vec<String> = row.iter().map(|d| format!("{d:016x}")).collect();
            let _ = writeln!(text, "{host} tick={tick} bodies={}", list.join(","));
        }
        text
    }
}

fn skeleton() -> Skeleton {
    Skeleton {
        joints: RIG
            .iter()
            .map(|&(parent, name, _)| Joint {
                name: name.into(),
                parent,
            })
            .collect(),
    }
}

fn pose(spawn: [f32; 3]) -> LocalPose {
    let mut positions: Vec<[f32; 3]> = RIG.iter().map(|&(_, _, offset)| offset).collect();
    positions[0] = spawn;
    let mut rotations = vec![[0.0, 0.0, 0.0, 1.0]; RIG.len()];
    rotations[0] = SPAWN_TILT;
    LocalPose {
        positions,
        rotations,
    }
}

/// Digest of each body: its world center, then its blended-out local position
/// and rotation, every component by bit pattern.
fn body_digests(ragdoll: &Ragdoll) -> Result<Vec<u64>, RagdollError> {
    let local = ragdoll.bone_poses();
    let mut digests = Vec::with_capacity(RIG.len());
    for (bone, (position, rotation)) in local.positions.iter().zip(&local.rotations).enumerate() {
        let world = ragdoll
            .bone_position(bone)
            .ok_or(RagdollError::BadBone(bone))?;
        let mut hash = Fnv64::new();
        for value in world.iter().chain(position).chain(rotation) {
            hash.write_f32(*value);
        }
        digests.push(hash.finish());
    }
    Ok(digests)
}

/// Run one fall.
///
/// # Errors
/// Forwards the [`RagdollError`] of any ragdoll call; the standard fixture never fails.
pub fn run(fixture: &Fixture) -> Result<RagdollTape, RagdollError> {
    let shape = BoneShape {
        radius: BONE_RADIUS,
        half_height: BONE_HALF_HEIGHT,
    };
    let mut ragdoll = Ragdoll::spawn(&skeleton(), &pose(fixture.spawn), &[shape; RIG.len()])?;
    ragdoll.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])?;
    ragdoll.blend_to_ragdoll(1.0)?;
    let mut bodies = Vec::new();
    for tick in 0..TICKS {
        if tick == HIT_TICK {
            ragdoll.apply_hit(HIT_BONE, fixture.hit, u64::from(tick))?;
        }
        ragdoll.step(FIXED_DT)?;
        bodies.push(body_digests(&ragdoll)?);
    }
    Ok(RagdollTape {
        bodies,
        contacts: ragdoll.contact_events(),
        final_state: ragdoll.state_hash(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standard() -> RagdollTape {
        run(&Fixture::STANDARD).expect("the standard fall runs")
    }

    #[test]
    fn the_fall_repeats_bit_for_bit() {
        let first = standard();
        assert_eq!(first, standard(), "two falls in one process must match");
        assert_eq!(first.bodies.len(), TICKS as usize);
        assert!(first.bodies.iter().all(|tick| tick.len() == RIG.len()));
    }

    #[test]
    fn the_standard_fall_keeps_its_four_machine_digest() {
        let tape = standard();
        assert_eq!(
            tape.digest(),
            GOLDEN_RAGDOLL,
            "the ragdoll fall moved: Rapier or the fixture changed"
        );
        assert!(
            tape.rows("h")
                .starts_with(&format!("h ragdoll={GOLDEN_RAGDOLL:016x} ")),
            "the printed row carries the pinned digest"
        );
    }

    #[test]
    fn the_fall_reaches_the_floor_and_the_hit_acts() {
        let tape = standard();
        assert!(
            tape.contacts > 0,
            "a fall onto a floor must report contacts"
        );
        let unhit = run(&Fixture {
            hit: [0.0; 3],
            ..Fixture::STANDARD
        })
        .expect("the unhit fall runs");
        // The hit lands on HIT_TICK: every earlier tick is the same fall.
        let hit_at = HIT_TICK as usize;
        assert_eq!(tape.bodies[..hit_at], unhit.bodies[..hit_at]);
        assert_ne!(
            tape.bodies[hit_at], unhit.bodies[hit_at],
            "the hit must move a body on the tick it lands"
        );
        assert_ne!(tape.digest(), unhit.digest());
    }

    #[test]
    fn the_digest_sees_one_ulp_of_the_spawn_height() {
        let base = standard();
        let nudged = run(&Fixture {
            spawn: [0.0, f32::from_bits(2.0f32.to_bits() + 1), 0.0],
            ..Fixture::STANDARD
        })
        .expect("the nudged fall runs");
        assert_ne!(
            base.bodies[0][0], nudged.bodies[0][0],
            "one ULP of height moves the pelvis digest on the first tick"
        );
        assert_ne!(base.digest(), nudged.digest());
    }

    #[test]
    fn rows_have_the_shape_the_matrix_tool_reads() {
        let tape = standard();
        let text = tape.rows("wasi-wasm32");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 1 + TICKS as usize);
        let summary = format!(
            "wasi-wasm32 ragdoll={:016x} ticks={TICKS} bodies={} contacts={} final_state={:016x}",
            tape.digest(),
            RIG.len(),
            tape.contacts,
            tape.final_state
        );
        assert_eq!(lines[0], summary);
        for (tick, line) in lines[1..].iter().enumerate() {
            let head = format!("wasi-wasm32 tick={tick} bodies=");
            let list = line
                .strip_prefix(&head)
                .unwrap_or_else(|| panic!("line {line} must start with {head}"));
            let digests: Vec<&str> = list.split(',').collect();
            assert_eq!(digests.len(), RIG.len(), "one digest per body: {line}");
            for digest in digests {
                assert!(
                    digest.len() == 16
                        && digest
                            .bytes()
                            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
                    "body digest {digest} must be 16 lower-hex digits"
                );
            }
        }
    }
}
