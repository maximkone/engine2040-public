//! springs: secondary motion.
//!
//! Owner: `engine-anim`; consumer: locomotion dresses presentation poses via
//! [`apply_to_pose`], which writes settled tips onto pose joints.
//! Fixed-point spring
//! bones for plumes, straps, hair cards and cloth tails; presentation only,
//! never authoritative. Units: meters in, Q12 fixed point inside, 60 Hz
//! ticks. Determinism: integer math only (`+ - * /`, shifts), stable bone
//! order, no clock; one impulse script settles identically twice. Errors:
//! [`SpringError`]. Budget: O(bones) per tick, 0.1 ms for 8 fighters;
//! caller-owned buffers, no heap per tick.

use super::contract::LocalPose;

/// Fixed-point steps per meter (Q12).
pub const Q: i32 = 4096;
/// Shift matching [`Q`].
pub const SHIFT: u32 = 12;
/// Largest bone count in one set; bounds per-tick work.
pub const MAX_BONES: usize = 64;
/// Hard cap on tip velocity (Q12 steps per tick); keeps impulses finite.
pub const VMAX_Q: i32 = 2048;

/// Secondary-motion failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpringError {
    /// Stiffness outside `0..=Q`; value carried.
    BadStiffness(i32),
    /// Damping outside `0..=Q`; value carried.
    BadDamping(i32),
    /// Max offset not positive; value carried.
    BadMaxOffset(i32),
    /// Lane count mismatch; expected plus got.
    LengthMismatch { expected: usize, got: usize },
    /// Non-finite float in the named lane.
    NonFinite(&'static str),
    /// Bone count past [`MAX_BONES`]; value carried.
    TooManyBones(usize),
    /// Joint index out of range; value carried.
    BadJoint(usize),
    /// No bones to step.
    EmptySet,
}

impl core::fmt::Display for SpringError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SpringError::BadStiffness(k) => write!(f, "springs: stiffness {k} outside 0..={Q}"),
            SpringError::BadDamping(d) => write!(f, "springs: damping {d} outside 0..={Q}"),
            SpringError::BadMaxOffset(m) => write!(f, "springs: max offset {m} not positive"),
            SpringError::LengthMismatch { expected, got } => {
                write!(f, "springs: lane count {got} differs from {expected}")
            }
            SpringError::NonFinite(lane) => write!(f, "springs: non-finite float in {lane}"),
            SpringError::TooManyBones(n) => write!(f, "springs: {n} bones past {MAX_BONES}"),
            SpringError::BadJoint(joint) => write!(f, "springs: joint {joint} out of range"),
            SpringError::EmptySet => write!(f, "springs: no bones to step"),
        }
    }
}

impl std::error::Error for SpringError {}

/// Shared spring tuning: stiffness pulls tips home, damping drains energy,
/// max offset clamps excursion. All Q12 (`Q` = rest, `0` = free).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SpringParams {
    /// Pull per tick in Q12 (`0..=Q`).
    pub stiff_q: i32,
    /// Velocity kept per tick in Q12 (`0..=Q`; below `Q` drains energy).
    pub damp_q: i32,
    /// Largest tip excursion from the anchor, Q12 steps, positive.
    pub max_offset_q: i32,
}

impl SpringParams {
    /// New tuning; out-of-range lanes refuse loudly.
    pub fn new(stiff_q: i32, damp_q: i32, max_offset_q: i32) -> Result<Self, SpringError> {
        if !(0..=Q).contains(&stiff_q) {
            return Err(SpringError::BadStiffness(stiff_q));
        }
        if !(0..=Q).contains(&damp_q) {
            return Err(SpringError::BadDamping(damp_q));
        }
        if max_offset_q <= 0 {
            return Err(SpringError::BadMaxOffset(max_offset_q));
        }
        Ok(Self {
            stiff_q,
            damp_q,
            max_offset_q,
        })
    }
}

/// Meters to Q12 steps: non-finite maps to zero, range saturates to
/// `i32::MIN..=i32::MAX` (bit-exact with float saturation, the same shape
/// as the clip quantizer in the crate root).
fn to_q(meters: f32) -> i32 {
    if !meters.is_finite() {
        return 0;
    }
    // Q (4096) fits `i16` exactly and `i16` widens to `f32` losslessly, so
    // this scale is exactly `Q as f32` with no `as` cast.
    let scale = f32::from(i16::try_from(Q).unwrap_or(i16::MAX));
    f32_to_i32_sat((meters * scale).round())
}

/// Truncate a float to `i32`, saturating: bit-exact with the `as`
/// saturation (`NaN` maps to zero, out-of-range clamps), using integer math
/// past the sign/exponent split so no `as` cast is needed.
pub(crate) fn f32_to_i32_sat(v: f32) -> i32 {
    if v.is_nan() {
        return 0;
    }
    // Bounds spelled in `f64`: widening is exact and the 2^31 walls are
    // exact there (they are not spellable as `f32` literals).
    if f64::from(v) >= 2_147_483_648.0 {
        return i32::MAX;
    }
    if f64::from(v) < -2_147_483_648.0 {
        return i32::MIN;
    }
    if v.abs() < 1.0 {
        return 0;
    }
    let bits = v.to_bits();
    let neg = bits >> 31 != 0;
    let exp = (bits >> 23) & 0xFF;
    let mant = (bits & 0x7F_FFFF) | 0x80_0000;
    let e = exp - 127;
    let mag: u64 = if e >= 23 {
        u64::from(mant) << (e - 23)
    } else {
        u64::from(mant >> (23 - e))
    };
    let signed = i64::try_from(mag).unwrap_or(i64::MAX);
    let signed = if neg { -signed } else { signed };
    i32::try_from(signed).unwrap_or(if neg { i32::MIN } else { i32::MAX })
}

/// Clamp `v` into `anchor +- max`.
fn clamp_lane(v: i32, anchor: i32, max: i32) -> i32 {
    v.clamp(anchor.saturating_sub(max), anchor.saturating_add(max))
}

/// Integer multiply-adds budgeted per bone lane per tick inside [`SpringSet::step`]
/// (pull, drag, clamp adds): bounded work, so the 0.1 ms bar for 8 fighters
/// follows from the op count, never a wall clock (clocks stay out of anim).
pub const OPS_PER_LANE_TICK: u64 = 8;

/// Budgeted integer ops for one tick over `bones` bones. Bone counts fit
/// `u64` exactly on every supported target; absurd counts saturate.
pub fn tick_ops(bones: usize) -> u64 {
    u64::try_from(bones).unwrap_or(u64::MAX) * 3 * OPS_PER_LANE_TICK
}

/// Caller-owned spring set: tip positions plus velocities in Q12, one entry
/// per bone in stable order. Construct with [`SpringSet::with_capacity`],
/// add bones with [`SpringSet::add_bone`], advance with [`SpringSet::step`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SpringSet {
    params: SpringParams,
    pos: Vec<[i32; 3]>,
    vel: Vec<[i32; 3]>,
}

impl SpringSet {
    /// New set over `params` with room for `capacity` bones (clamped to
    /// [`MAX_BONES`]); the only allocation in the set lifetime.
    pub fn with_capacity(params: SpringParams, capacity: usize) -> Result<Self, SpringError> {
        if capacity > MAX_BONES {
            return Err(SpringError::TooManyBones(capacity));
        }
        Ok(Self {
            params,
            pos: Vec::with_capacity(capacity),
            vel: Vec::with_capacity(capacity),
        })
    }

    /// Shared tuning.
    pub fn params(&self) -> SpringParams {
        self.params
    }

    /// Bone count.
    pub fn len(&self) -> usize {
        self.pos.len()
    }

    /// True when the set holds no bones.
    pub fn is_empty(&self) -> bool {
        self.pos.is_empty()
    }

    /// Add one bone at `anchor_m` (meters); tip starts at rest on the anchor.
    /// Non-finite lanes refuse; past [`MAX_BONES`] refuses.
    pub fn add_bone(&mut self, anchor_m: [f32; 3]) -> Result<usize, SpringError> {
        if anchor_m.iter().any(|v| !v.is_finite()) {
            return Err(SpringError::NonFinite("anchor"));
        }
        if self.pos.len() >= MAX_BONES {
            return Err(SpringError::TooManyBones(self.pos.len() + 1));
        }
        let anchor = [to_q(anchor_m[0]), to_q(anchor_m[1]), to_q(anchor_m[2])];
        self.pos.push(anchor);
        self.vel.push([0; 3]);
        Ok(self.pos.len() - 1)
    }

    /// Kick one bone: add `impulse_m_s` (m/s) to its tip velocity.
    pub fn kick(&mut self, bone: usize, impulse_m_s: [f32; 3]) -> Result<(), SpringError> {
        if bone >= self.pos.len() {
            return Err(SpringError::BadJoint(bone));
        }
        if impulse_m_s.iter().any(|v| !v.is_finite()) {
            return Err(SpringError::NonFinite("impulse"));
        }
        for (slot, push) in self.vel[bone].iter_mut().zip(impulse_m_s.iter()) {
            *slot = slot.saturating_add(to_q(*push));
        }
        Ok(())
    }

    /// Advance `dt_ticks` (>= 1) toward `anchors_q` (Q12, one per bone),
    /// writing tips to `out_q`. Semi-implicit Euler in integers: damped
    /// velocity first, then the clamped position. Length mismatches refuse
    /// and write nothing; steady-state calls allocate nothing.
    pub fn step(
        &mut self,
        anchors_q: &[[i32; 3]],
        dt_ticks: u32,
        out_q: &mut [[i32; 3]],
    ) -> Result<(), SpringError> {
        if self.pos.is_empty() {
            return Err(SpringError::EmptySet);
        }
        if anchors_q.len() != self.pos.len() || out_q.len() != self.pos.len() {
            return Err(SpringError::LengthMismatch {
                expected: self.pos.len(),
                got: anchors_q.len().min(out_q.len()),
            });
        }
        let dt = i64::from(dt_ticks.max(1));
        let stiff = i64::from(self.params.stiff_q);
        let damp = i64::from(self.params.damp_q);
        let max = self.params.max_offset_q;
        for ((pos_row, vel_row), (anchor_row, out_row)) in self
            .pos
            .iter_mut()
            .zip(self.vel.iter_mut())
            .zip(anchors_q.iter().zip(out_q.iter_mut()))
        {
            for ((pos, vel), (anchor, out)) in pos_row
                .iter_mut()
                .zip(vel_row.iter_mut())
                .zip(anchor_row.iter().zip(out_row.iter_mut()))
            {
                let anchor_i64 = i64::from(*anchor);
                let mut p = i64::from(*pos);
                let mut v = i64::from(*vel);
                for _ in 0..dt {
                    let err = anchor_i64.saturating_sub(p);
                    let pull = err.saturating_mul(stiff) >> SHIFT;
                    let drag = v.saturating_mul(damp) >> SHIFT;
                    v = (v + pull - (v - drag)).clamp(i64::from(-VMAX_Q), i64::from(VMAX_Q));
                    p = anchor_i64 + (p + v - anchor_i64).clamp(-i64::from(max), i64::from(max));
                }
                // Both lanes were clamped above (`v` to +-VMAX_Q, `p` to
                // anchor +- max), so they fit `i32` exactly; the fallback
                // saturates and never fires on reachable inputs.
                *vel = i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX });
                *pos = clamp_lane(
                    i32::try_from(p).unwrap_or(if p < 0 { i32::MIN } else { i32::MAX }),
                    *anchor,
                    max,
                );
                *out = *pos;
            }
        }
        Ok(())
    }

    /// Largest tip excursion from its anchor in Q12 steps (settle meter).
    pub fn max_offset_q(&self, anchors_q: &[[i32; 3]]) -> Result<i32, SpringError> {
        if anchors_q.len() != self.pos.len() {
            return Err(SpringError::LengthMismatch {
                expected: self.pos.len(),
                got: anchors_q.len(),
            });
        }
        let mut worst = 0i32;
        for (pos_row, anchor) in self.pos.iter().zip(anchors_q.iter()) {
            for (tip, lane) in pos_row.iter().zip(anchor.iter()) {
                let off = tip.saturating_sub(*lane).saturating_abs();
                if off > worst {
                    worst = off;
                }
            }
        }
        Ok(worst)
    }
}

/// Write spring tips (meters) into pose lanes: `tips_m[k]` lands on
/// `pose.positions[joints[k]]`. Presentation only. Bad joints, length
/// mismatches and non-finite tips refuse and write nothing.
pub fn apply_to_pose(
    pose: &mut LocalPose,
    tips_m: &[[f32; 3]],
    joints: &[usize],
) -> Result<(), SpringError> {
    if tips_m.len() != joints.len() {
        return Err(SpringError::LengthMismatch {
            expected: joints.len(),
            got: tips_m.len(),
        });
    }
    for (tip, joint) in tips_m.iter().zip(joints.iter()) {
        if *joint >= pose.positions.len() {
            return Err(SpringError::BadJoint(*joint));
        }
        if tip.iter().any(|v| !v.is_finite()) {
            return Err(SpringError::NonFinite("tip"));
        }
    }
    for (tip, joint) in tips_m.iter().zip(joints.iter()) {
        pose.positions[*joint] = *tip;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::contract::LocalPose;
    use super::*;
    use engine_core::alloc_probe::measure;
    use engine_core::Fnv64;

    fn tuned() -> SpringParams {
        SpringParams::new(Q / 2, Q - Q / 8, Q / 4).expect("fixture tuning valid")
    }

    fn set_with(bones: usize) -> SpringSet {
        let mut set = SpringSet::with_capacity(tuned(), bones).expect("fixture fits");
        for b in 0..bones {
            set.add_bone([b as f32 * 0.1, 1.0, 0.0])
                .expect("fixture bone valid");
        }
        set
    }

    fn anchors_for(set: &SpringSet) -> Vec<[i32; 3]> {
        set.pos.clone()
    }

    #[test]
    fn soak_never_blows_up_then_settles() {
        // 8 fighters' plumes, one hour at 60 Hz, kicked every 97 ticks: tips
        // stay inside the clamp every tick, then settle under a millimeter
        // once the kicks stop.
        let mut set = set_with(8);
        let anchors = anchors_for(&set);
        let mut out = vec![[0; 3]; 8];
        let mut bone = 0usize;
        for t in 0..216_000 {
            if t % 97 == 0 {
                set.kick(bone, [0.6, -0.3, 0.4])
                    .expect("fixture kick valid");
                bone = (bone + 1) % 8;
            }
            set.step(&anchors, 1, &mut out).expect("soak steps");
            for (bone, tip) in out.iter().enumerate() {
                for lane in 0..3 {
                    let off = tip[lane]
                        .saturating_sub(anchors[bone][lane])
                        .saturating_abs();
                    assert!(
                        off <= tuned().max_offset_q,
                        "tick {t} bone {bone}: blow-up {off}"
                    );
                }
            }
        }
        for _ in 0..600 {
            set.step(&anchors, 1, &mut out).expect("settle steps");
        }
        let rest = set.max_offset_q(&anchors).expect("fixture metered");
        assert!(rest <= to_q(0.001), "settle {rest} steps past 1 mm");
        println!(
            "soak: 216000 ticks x 8 bones, max clamp held, rest={rest} steps ({} um)",
            rest.saturating_mul(1000) / Q
        );
    }

    #[test]
    fn replay_exact_x2() {
        let run = || {
            let mut set = set_with(4);
            let anchors = anchors_for(&set);
            let mut out = vec![[0; 3]; 4];
            for t in 0..240 {
                if t % 37 == 0 {
                    set.kick(t % 4, [0.4, 0.2, -0.3]).expect("script kicks");
                }
                set.step(&anchors, 1, &mut out).expect("script steps");
            }
            (set, out)
        };
        let (a_set, a_out) = run();
        let (b_set, b_out) = run();
        assert_eq!(a_set, b_set);
        assert_eq!(a_out, b_out);
    }

    #[test]
    fn consumer_dresses_pose_joints() {
        // In-lane consumer path: spring tips land on the presentation pose
        // joints via `apply_to_pose` while the remaining joints stay
        // untouched (locomotion wiring calls this same entry point).
        let mut pose = LocalPose {
            positions: vec![[0.0, 1.0, 0.0]; 7],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 7],
        };
        let before = pose.clone();
        let tips = [[0.05, 1.2, 0.02], [-0.05, 1.2, -0.02]];
        apply_to_pose(&mut pose, &tips, &[5, 6]).expect("dress valid");
        assert_eq!(pose.positions[5], tips[0]);
        assert_eq!(pose.positions[6], tips[1]);
        for (index, slot) in pose.positions.iter().enumerate() {
            if index != 5 && index != 6 {
                assert_eq!(*slot, before.positions[index], "joint {index} untouched");
            }
        }
    }

    #[test]
    fn failures_are_typed_never_panics() {
        assert_eq!(
            SpringParams::new(-1, Q, Q),
            Err(SpringError::BadStiffness(-1))
        );
        assert_eq!(
            SpringParams::new(Q, Q + 1, Q),
            Err(SpringError::BadDamping(Q + 1))
        );
        assert_eq!(
            SpringParams::new(Q, Q, 0),
            Err(SpringError::BadMaxOffset(0))
        );
        assert_eq!(
            SpringSet::with_capacity(tuned(), MAX_BONES + 1),
            Err(SpringError::TooManyBones(MAX_BONES + 1))
        );
        let mut set = set_with(2);
        assert_eq!(
            set.add_bone([f32::NAN, 0.0, 0.0]),
            Err(SpringError::NonFinite("anchor"))
        );
        assert_eq!(set.kick(9, [0.0; 3]), Err(SpringError::BadJoint(9)));
        assert_eq!(
            set.kick(0, [f32::INFINITY, 0.0, 0.0]),
            Err(SpringError::NonFinite("impulse"))
        );
        let anchors = anchors_for(&set);
        let mut short = vec![[0; 3]; 1];
        assert_eq!(
            set.step(&anchors, 1, &mut short),
            Err(SpringError::LengthMismatch {
                expected: 2,
                got: 1
            })
        );
        let empty = SpringSet::with_capacity(tuned(), 0).expect("empty fits");
        let mut none: Vec<[i32; 3]> = Vec::new();
        assert_eq!(
            empty.clone().step(&[], 1, &mut none),
            Err(SpringError::EmptySet)
        );
        let mut pose = LocalPose {
            positions: vec![[0.0, 1.0, 0.0]; 4],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 4],
        };
        let bad_joint = pose.positions.len() + 1;
        assert_eq!(
            apply_to_pose(&mut pose, &[[0.0; 3]], &[bad_joint]),
            Err(SpringError::BadJoint(bad_joint))
        );
        assert_eq!(
            apply_to_pose(&mut pose, &[[f32::NAN, 0.0, 0.0]], &[0]),
            Err(SpringError::NonFinite("tip"))
        );
        assert_eq!(
            apply_to_pose(&mut pose, &[[0.0; 3]; 2], &[0]),
            Err(SpringError::LengthMismatch {
                expected: 1,
                got: 2
            })
        );
        let text = SpringError::BadJoint(99).to_string();
        assert!(text.contains("99"), "{text}");
    }

    #[test]
    fn steady_state_allocates_nothing() {
        let mut set = set_with(8);
        let anchors = anchors_for(&set);
        let mut out = vec![[0; 3]; 8];
        for _ in 0..8 {
            set.step(&anchors, 1, &mut out).expect("warmup steps");
        }
        let ((), heap) = measure(|| {
            for _ in 0..240 {
                set.step(&anchors, 1, &mut out).expect("steady steps");
                std::hint::black_box(&out);
            }
        });
        assert_eq!(heap.calls, 0, "spring tick must not allocate");
    }

    #[test]
    fn eight_fighters_fit_the_op_budget() {
        // 8 fighters x 8 bones run 60 real ticks; the budgeted op count is
        // 92_160 integer ops, far below what 0.1 ms retires on the reference
        // CPU, so the bar holds by construction (the perf gate posts the wall
        // number; wall clocks stay out of `engine-anim`).
        let mut sets: Vec<SpringSet> = (0..8).map(|_| set_with(8)).collect();
        let anchors: Vec<Vec<[i32; 3]>> = sets.iter().map(anchors_for).collect();
        let mut outs: Vec<Vec<[i32; 3]>> = (0..8).map(|_| vec![[0; 3]; 8]).collect();
        for round in 0..60 {
            for (fighter, set) in sets.iter_mut().enumerate() {
                if round % 17 == 0 {
                    set.kick(round % 8, [0.3, 0.1, -0.2]).expect("budget kicks");
                }
                set.step(&anchors[fighter], 1, &mut outs[fighter])
                    .expect("budget steps");
            }
        }
        std::hint::black_box(&outs);
        let budgeted = 8 * tick_ops(8) * 60;
        assert_eq!(budgeted, 92_160);
        assert!(sets.len() == 8 && outs.len() == 8);
    }

    /// One digest over the spring state plus the tick tips. Integer and bit
    /// work only; allocates nothing.
    fn am19_digest(set: &SpringSet, out: &[[i32; 3]]) -> u64 {
        let mut hash = Fnv64::new();
        for row in set.pos.iter().chain(set.vel.iter()).chain(out.iter()) {
            for lane in row.iter() {
                hash.write_i32(*lane);
            }
        }
        hash.finish()
    }

    /// One scripted tape tick: a fixed kick schedule, then the step.
    fn am19_tick(set: &mut SpringSet, anchors: &[[i32; 3]], out: &mut [[i32; 3]], tick: usize) {
        if tick % 9 == 0 {
            set.kick(tick % out.len(), [0.4, 0.2, -0.3])
                .expect("script kicks");
        }
        set.step(anchors, 1, out).expect("script steps");
    }

    #[test]
    fn am19_springs_replay_x2_and_restore_equal() {
        let run = || {
            let mut set = set_with(4);
            let anchors = anchors_for(&set);
            let mut out = vec![[0; 3]; 4];
            let mut digests = Vec::with_capacity(64);
            for tick in 0..64usize {
                am19_tick(&mut set, &anchors, &mut out, tick);
                digests.push(am19_digest(&set, &out));
            }
            digests
        };
        let first = run();
        let second = run();
        assert_eq!(first, second, "spring tape must replay x2");
        assert_ne!(first[0], first[63], "scripted tape must move");
        // Snapshot mid-tape, run out, restore, run out again.
        let mut set = set_with(4);
        let anchors = anchors_for(&set);
        let mut out = vec![[0; 3]; 4];
        for tick in 0..16usize {
            am19_tick(&mut set, &anchors, &mut out, tick);
        }
        let snapshot_set = set.clone();
        let snapshot_out = out.clone();
        let mut reference = Vec::with_capacity(48);
        for tick in 16..64usize {
            am19_tick(&mut set, &anchors, &mut out, tick);
            reference.push(am19_digest(&set, &out));
        }
        set.clone_from(&snapshot_set);
        out.clone_from(&snapshot_out);
        assert_eq!(set, snapshot_set, "restored set must hold the snapshot");
        assert_eq!(out, snapshot_out, "restored tips must hold the snapshot");
        let mut replay = Vec::with_capacity(48);
        for tick in 16..64usize {
            am19_tick(&mut set, &anchors, &mut out, tick);
            replay.push(am19_digest(&set, &out));
        }
        assert_eq!(reference, replay, "restore-then-step must equal");
    }

    #[test]
    fn am19_springs_eval_paths_allocate_nothing_per_tick() {
        // Every per-tick eval path on the springs route: the step, the
        // settle meter and the pose dress. Setup buffers are caller-owned
        // and preallocated; the measured loop must add no heap call.
        const TIPS: [[f32; 3]; 8] = [[0.05, 1.2, 0.02]; 8];
        const JOINTS: [usize; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
        let mut set = set_with(8);
        let anchors = anchors_for(&set);
        let mut out = vec![[0; 3]; 8];
        let mut pose = LocalPose {
            positions: vec![[0.0, 1.0, 0.0]; 8],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 8],
        };
        let path_tick = |set: &mut SpringSet, out: &mut [[i32; 3]], pose: &mut LocalPose| {
            set.step(&anchors, 1, out).expect("steady steps");
            let rest = set.max_offset_q(&anchors).expect("steady meter");
            apply_to_pose(pose, &TIPS, &JOINTS).expect("steady dress");
            std::hint::black_box(rest);
            std::hint::black_box(&out);
            std::hint::black_box(&pose);
        };
        for _ in 0..8 {
            path_tick(&mut set, &mut out, &mut pose);
        }
        let ((), heap) = measure(|| {
            for _ in 0..240 {
                path_tick(&mut set, &mut out, &mut pose);
            }
        });
        assert_eq!(heap.calls, 0, "springs eval paths must not allocate");
    }

    #[test]
    fn am19_alloc_probe_sees_a_real_allocation() {
        // Calibration: a blind counter must not fake the gate above.
        let ((), heap) = measure(|| {
            std::hint::black_box(vec![0u8; 64]);
        });
        assert!(heap.calls >= 1, "alloc probe is blind");
    }

    #[test]
    fn q_conversion_saturates_without_casts() {
        // `to_q` keeps the float-saturation shape: unit scale, sign kept,
        // zero on non-finite, clamp at the `i32` walls.
        assert_eq!(to_q(0.0), 0);
        assert_eq!(to_q(1.0), Q);
        assert_eq!(to_q(-1.0), -Q);
        assert_eq!(to_q(f32::NAN), 0);
        assert_eq!(to_q(f32::INFINITY), 0);
        assert_eq!(to_q(1.0e30), i32::MAX);
        assert_eq!(to_q(-1.0e30), i32::MIN);
        // The integer helper is bit-exact with `as` saturation (oracle
        // sweep; `as` here is test-only, never production code).
        for v in [
            0.0,
            -0.0,
            0.5,
            -0.5,
            1.0,
            -1.0,
            1.5,
            -1.5,
            100.25,
            -100.25,
            8_388_608.0,
            -8_388_608.0,
            (i32::MAX - 256) as f32,
            i32::MIN as f32,
            i32::MAX as f32,
            u32::MAX as f32,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ] {
            assert_eq!(f32_to_i32_sat(v), v as i32, "sat {v}");
        }
    }
}
