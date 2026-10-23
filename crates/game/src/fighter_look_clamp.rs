//! Last-stage joint clamp (AM24): the angular limiter caps each bone's turn,
//! but a three-bone leg chain can still carry its toe past the pop law's 10 cm
//! miss budget in one tick. This stage samples the fully presented pose (clip,
//! inertia, feet plant, throw) at the pop law's mid tick, caps each joint's
//! miss against the previous sample, and writes the capped model-space
//! position back into the skin matrices. Presentation only; the sim hash never
//! reads it.

use engine_anim::inertialize::{AngularLimiter, DEFAULT_ANG_LIMIT_DEG_PER_SEC};

use super::{FighterLook, GaitPhase, Skeleton, LAG_TICKS, MAX_LOOK_FIGHTERS, SEED_S, TICK_HZ};

/// Pop-law miss budget with 1 cm of float margin: `view_truth::POP_M` is
/// 10 cm, and a joint at exactly 10 cm does not count as a pop (`>`).
const LOOK_POP_CAP_M: f32 = 0.09;

/// One slot's angular-speed limiter lanes: the `engine-anim` limiter owns the
/// cap and the last accepted rotation per bone; this struct keeps this tick's
/// start and end rotations so a frame at any sub-tick `alpha` reads a pose
/// between them. It also keeps the previous and current mid-tick joint
/// positions for the last-stage position clamp. Built once per look slot at
/// load; `observe_tick` advances it. Presentation only; never hashed.
pub(super) struct LimitLane {
    /// Caps each bone's rotation change per tick at
    /// [`DEFAULT_ANG_LIMIT_DEG_PER_SEC`].
    pub(super) limiter: AngularLimiter,
    /// Rotations the previous tick's output held (this tick's start).
    from: Vec<[f32; 4]>,
    /// Rotations this tick's output holds (its end).
    pub(super) to: Vec<[f32; 4]>,
    /// False until the slot's first pose: a first sight takes the pose whole
    /// instead of ramping in from identity.
    seeded: bool,
    /// Parent link per joint, from the rig's skeleton.
    parents: Vec<Option<usize>>,
    /// Previous mid-tick presented joint positions.
    prev_pos: Vec<[f32; 3]>,
    /// Velocity into `prev_pos`, meters per second.
    prev_vel: Vec<[f32; 3]>,
    /// This tick's clamped mid-tick joint positions.
    cur_pos: Vec<[f32; 3]>,
    /// Ticks from `prev_pos` to `cur_pos`; one on a steady frame loop.
    span: u32,
    /// Look tick `cur_pos` was clamped on.
    cur_tick: u32,
    /// True once `cur_pos` holds a sample.
    pos_seeded: bool,
    /// True once a second tick gives the position clamp a prediction.
    pos_ready: bool,
    /// True when `cur_pos` must move into `prev_pos` at the next tick.
    shift_pending: bool,
}

impl LimitLane {
    /// A lane for one rig's joints at the look's fixed cap, unseeded.
    pub(super) fn new(skeleton: &Skeleton) -> Self {
        let joints = skeleton.len();
        Self {
            limiter: AngularLimiter::new(joints, DEFAULT_ANG_LIMIT_DEG_PER_SEC),
            from: vec![[0.0, 0.0, 0.0, 1.0]; joints],
            to: vec![[0.0, 0.0, 0.0, 1.0]; joints],
            seeded: false,
            parents: skeleton.joints.iter().map(|joint| joint.parent).collect(),
            prev_pos: vec![[0.0; 3]; joints],
            prev_vel: vec![[0.0; 3]; joints],
            cur_pos: vec![[0.0; 3]; joints],
            span: 1,
            cur_tick: 0,
            pos_seeded: false,
            pos_ready: false,
            shift_pending: false,
        }
    }
}

/// Shortest-arc normalized lerp between two unit quats by `t` in `0..=1`;
/// `t >= 1` reads `b` whole. A degenerate sum reads `a`. Pure.
fn rot_lerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let w = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    if w >= 1.0 {
        return b;
    }
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let sign = if dot < 0.0 { -1.0 } else { 1.0 };
    let mut q = [
        a[0] + (b[0] * sign - a[0]) * w,
        a[1] + (b[1] * sign - a[1]) * w,
        a[2] + (b[2] * sign - a[2]) * w,
        a[3] + (b[3] * sign - a[3]) * w,
    ];
    let norm = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if norm.is_finite() && norm > 0.0 {
        for lane in &mut q {
            *lane /= norm;
        }
        q
    } else {
        a
    }
}

impl FighterLook {
    /// Advance every seen slot's angular limiter to this tick's end pose, then
    /// advance its position clamp to the mid-tick sample the pop law reads.
    /// The wanted pose is the clip plus inertia pose at the tick's end (the
    /// very pose [`slot_sample`](Self::slot_sample) samples before the feet
    /// plant and the throw layer, which keep their own bounded motion); the
    /// limiter caps each bone's change at [`DEFAULT_ANG_LIMIT_DEG_PER_SEC`]
    /// over `elapsed` ticks. A slot first seen, unseen, ragdolled (physics owns
    /// its bones), under a forced balance pose (a fall or a kneel lands whole)
    /// or refused drops to unseeded, so its next pose lands whole instead of
    /// ramping in from stale or identity rotations. Presentation only; the sim
    /// hash never reads it. Caller: [`observe_tick`](Self::observe_tick) last.
    pub(super) fn limit_tick(&mut self, elapsed: u32) {
        let dt = f32::from(u16::try_from(elapsed).unwrap_or(u16::MAX)) / TICK_HZ;
        for index in 0..MAX_LOOK_FIGHTERS {
            let slot = self.slots[index];
            // Move last tick's clamped sample into the previous slot before
            // this tick takes a new one. A reset below clears the shift.
            if elapsed > 0 {
                self.shift_limit_positions(index);
            }
            // Read the tick a hair before its end, as the switch fold does: at
            // exactly 1.0 a swing clip that runs out would sample the *next*
            // clip a tick before the inertial offsets take over that cut.
            let wanted = match (slot.seen, self.ragdolled(index)) {
                (true, false) => self
                    .rigs
                    .get(self.rig_for_slot(index))
                    .and_then(|rig| self.inertial_pose(rig, index, &slot, 1.0 - LAG_TICKS).ok()),
                _ => None,
            };
            // A balance verdict (a fall or a kneel) is a forced pose the look
            // lands at once, so it takes the whole cap budget instead of
            // ramping: the limiter guards clip motion, never a floor verdict.
            // A slot with nothing to pose drops to unseeded too.
            let ramp = slot.seen
                && !self.ragdolled(index)
                && matches!(slot.balance, GaitPhase::Steady | GaitPhase::Stumbling);
            if !ramp || wanted.is_none() {
                if let Some(lane) = self.limit.get_mut(index) {
                    lane.seeded = false;
                    lane.pos_seeded = false;
                    lane.pos_ready = false;
                    lane.shift_pending = false;
                }
            }
            if let Some(pose) = &wanted {
                self.advance_limit(index, &pose.rotations, dt);
            }
            if !ramp || wanted.is_none() {
                continue;
            }
            // A repeated observe at the same tick keeps the sample it has.
            if elapsed == 0 && self.limit.get(index).is_some_and(|lane| lane.pos_seeded) {
                continue;
            }
            // Sample the final presented pose at the pop law's mid tick; the
            // feet plant and the throw layer have already run in it.
            let raw = {
                let Some(rig) = self.rigs.get(self.rig_for_slot(index)) else {
                    continue;
                };
                self.slot_sample_raw(rig, index, &slot, 0.5, false)
                    .ok()
                    .map(|sample| {
                        sample
                            .world
                            .iter()
                            .map(|m| [m[3][0], m[3][1], m[3][2]])
                            .collect::<Vec<_>>()
                    })
            };
            if let Some(raw) = raw {
                self.advance_limit_positions(index, raw, elapsed);
            }
        }
    }

    /// Step one slot's limiter toward `wanted` and keep this tick's start and
    /// end rotations. An unseeded lane takes `wanted` whole: a full second of
    /// the cap accepts any angle, so the first frame shows the clip rather
    /// than ramping in from identity. Steady state allocates nothing.
    pub(super) fn advance_limit(&mut self, index: usize, wanted: &[[f32; 4]], dt: f32) {
        let Some(lane) = self.limit.get_mut(index) else {
            return;
        };
        if lane.to.len() != wanted.len() {
            return;
        }
        if !lane.seeded {
            lane.from.copy_from_slice(wanted);
            lane.limiter.step(wanted, SEED_S, &mut lane.to);
            lane.seeded = true;
            return;
        }
        lane.from.copy_from_slice(&lane.to);
        lane.limiter.step(wanted, dt, &mut lane.to);
    }

    /// Read the limiter on a frame at sub-tick `alpha`: each bone takes the
    /// shortest-arc blend from this tick's start rotation to its capped end
    /// rotation, so the drawn pose never moves faster than the cap and needs
    /// no state of its own. A slot with no seeded lane, a ragdolled one, or
    /// one under a forced balance pose (`ramp` false) keeps `rots` as the pose
    /// gave it: a floor verdict lands whole, never masked by a stale lane.
    /// Pure.
    pub(super) fn limit_pose(&self, index: usize, rots: &mut [[f32; 4]], alpha: f32, ramp: bool) {
        let Some(lane) = self.limit.get(index) else {
            return;
        };
        if !ramp || !lane.seeded || self.ragdolled(index) {
            return;
        }
        for ((rot, from), to) in rots.iter_mut().zip(&lane.from).zip(&lane.to) {
            *rot = rot_lerp(*from, *to, alpha);
        }
    }

    /// Move last tick's clamped positions into the previous slot and keep the
    /// velocity into them. The first shift only seeds that velocity to zero:
    /// the pop law checks no prediction on its first two samples.
    fn shift_limit_positions(&mut self, index: usize) {
        let Some(lane) = self.limit.get_mut(index) else {
            return;
        };
        if !lane.shift_pending {
            return;
        }
        let dt = f32::from(u16::try_from(lane.span.max(1)).unwrap_or(u16::MAX)) / TICK_HZ;
        if lane.pos_ready {
            for ((velocity, cur), prev) in lane
                .prev_vel
                .iter_mut()
                .zip(lane.cur_pos.iter())
                .zip(&lane.prev_pos)
            {
                *velocity = [
                    (cur[0] - prev[0]) / dt,
                    (cur[1] - prev[1]) / dt,
                    (cur[2] - prev[2]) / dt,
                ];
            }
        } else {
            lane.prev_vel.fill([0.0; 3]);
            lane.pos_ready = true;
        }
        lane.prev_pos.copy_from_slice(&lane.cur_pos);
        lane.shift_pending = false;
    }

    /// Clamp one slot's mid-tick joint positions against the previous sample:
    /// a first sample lands whole, every later one caps its miss at
    /// [`LOOK_POP_CAP_M`] with the same constant-velocity prediction the pop
    /// law uses. The next frame reads `cur_pos` whole at alpha 0.5.
    fn advance_limit_positions(&mut self, index: usize, mut raw: Vec<[f32; 3]>, elapsed: u32) {
        let Some(lane) = self.limit.get_mut(index) else {
            return;
        };
        if !lane.pos_seeded {
            lane.cur_pos.copy_from_slice(&raw);
            lane.span = elapsed.max(1);
            lane.cur_tick = self.tick;
            lane.pos_seeded = true;
            lane.shift_pending = true;
            return;
        }
        if lane.prev_pos.len() != raw.len() {
            return;
        }
        let dt = f32::from(u16::try_from(elapsed.max(1)).unwrap_or(u16::MAX)) / TICK_HZ;
        clamp_world_joints(
            &mut raw,
            Some(lane.parents.as_slice()),
            &lane.prev_pos,
            &lane.prev_vel,
            dt,
            LOOK_POP_CAP_M,
        );
        lane.cur_pos.copy_from_slice(&raw);
        lane.span = elapsed.max(1);
        lane.cur_tick = self.tick;
        lane.shift_pending = true;
    }

    /// Write this tick's final joint positions into `world`: the exact clamped
    /// mid sample at alpha 0.5 (what the pop law reads), or a clamped
    /// prediction at another sub-tick alpha so the drawn frames stay inside
    /// the same budget. A stale lane, a forced pose or a ragdoll keeps the
    /// pose as it is. View only.
    pub(super) fn clamp_sample_positions(
        &self,
        index: usize,
        world: &mut [[[f32; 4]; 4]],
        alpha: f32,
    ) {
        let Some(slot) = self.slots.get(index) else {
            return;
        };
        if !matches!(slot.balance, GaitPhase::Steady | GaitPhase::Stumbling) {
            return;
        }
        if self.ragdolled(index) {
            return;
        }
        let Some(lane) = self.limit.get(index) else {
            return;
        };
        if !lane.pos_seeded || !lane.pos_ready || lane.cur_tick != self.tick {
            return;
        }
        let alpha = if alpha.is_finite() {
            alpha.clamp(0.0, 1.0)
        } else {
            0.5
        };
        if (alpha - 0.5).abs() <= 1e-6 {
            for (matrix, position) in world.iter_mut().zip(&lane.cur_pos) {
                matrix[3][0] = position[0];
                matrix[3][1] = position[1];
                matrix[3][2] = position[2];
            }
            return;
        }
        let mut positions: Vec<[f32; 3]> =
            world.iter().map(|m| [m[3][0], m[3][1], m[3][2]]).collect();
        let span = f32::from(u16::try_from(lane.span.max(1)).unwrap_or(u16::MAX));
        let ticks = (span - 0.5 + alpha).max(0.0);
        clamp_world_joints(
            &mut positions,
            Some(lane.parents.as_slice()),
            &lane.prev_pos,
            &lane.prev_vel,
            ticks / TICK_HZ,
            LOOK_POP_CAP_M,
        );
        for (matrix, position) in world.iter_mut().zip(&positions) {
            matrix[3][0] = position[0];
            matrix[3][1] = position[1];
            matrix[3][2] = position[2];
        }
    }
}

/// Cap each joint's miss against `prev + vel * dt` at `cap`, parent first, and
/// carry a capped parent's delta down its chain so a segment that need not
/// shrink keeps its length. `parents` is `None` for a flat list. Pure.
fn clamp_world_joints(
    joints: &mut [[f32; 3]],
    parents: Option<&[Option<usize>]>,
    prev: &[[f32; 3]],
    vel: &[[f32; 3]],
    dt_s: f32,
    cap: f32,
) {
    let n = joints.len().min(prev.len()).min(vel.len());
    let mut deltas = vec![[0.0f32; 3]; n];
    for joint in 0..n {
        let original = joints[joint];
        let mut want = original;
        if let Some(parent) = parents.and_then(|links| links.get(joint).copied().flatten()) {
            if let Some(delta) = deltas.get(parent) {
                want[0] += delta[0];
                want[1] += delta[1];
                want[2] += delta[2];
            }
        }
        let predicted = [
            prev[joint][0] + vel[joint][0] * dt_s,
            prev[joint][1] + vel[joint][1] * dt_s,
            prev[joint][2] + vel[joint][2] * dt_s,
        ];
        let miss = [
            want[0] - predicted[0],
            want[1] - predicted[1],
            want[2] - predicted[2],
        ];
        let distance = (miss[0] * miss[0] + miss[1] * miss[1] + miss[2] * miss[2]).sqrt();
        let capped = if distance > cap {
            let scale = cap / distance;
            [
                predicted[0] + miss[0] * scale,
                predicted[1] + miss[1] * scale,
                predicted[2] + miss[2] * scale,
            ]
        } else {
            want
        };
        deltas[joint] = [
            capped[0] - original[0],
            capped[1] - original[1],
            capped[2] - original[2],
        ];
        joints[joint] = capped;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_anim::contract::Joint;

    fn two_joints() -> Skeleton {
        Skeleton {
            joints: vec![
                Joint {
                    name: "pelvis".to_string(),
                    parent: None,
                },
                Joint {
                    name: "toes_r".to_string(),
                    parent: Some(0),
                },
            ],
        }
    }

    fn parents(skeleton: &Skeleton) -> Vec<Option<usize>> {
        skeleton.joints.iter().map(|joint| joint.parent).collect()
    }

    #[test]
    fn clamp_world_caps_the_miss_to_the_pop_budget() {
        let skeleton = two_joints();
        let links = parents(&skeleton);
        let mut joints = [[0.0, 0.0, 0.0], [0.15, 0.0, 0.0]];
        let prev = [[0.0; 3]; 2];
        let vel = [[0.0; 3]; 2];
        clamp_world_joints(
            &mut joints,
            Some(links.as_slice()),
            &prev,
            &vel,
            1.0 / TICK_HZ,
            LOOK_POP_CAP_M,
        );
        assert!(joints[0][0].abs() < 1e-6, "root stays put");
        assert!((joints[1][0] - LOOK_POP_CAP_M).abs() < 1e-6, "{joints:?}");
        assert!(joints[1][0] < 0.10, "under the pop law's 10 cm");
    }

    #[test]
    fn clamp_world_keeps_a_step_inside_the_budget() {
        let skeleton = two_joints();
        let links = parents(&skeleton);
        let mut joints = [[0.0, 0.0, 0.0], [0.05, 0.0, 0.0]];
        let prev = [[0.0; 3]; 2];
        let vel = [[0.0; 3]; 2];
        clamp_world_joints(
            &mut joints,
            Some(links.as_slice()),
            &prev,
            &vel,
            1.0 / TICK_HZ,
            LOOK_POP_CAP_M,
        );
        assert!((joints[1][0] - 0.05).abs() < 1e-6, "{joints:?}");
    }
}
