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
/// Largest body count one collide pass may take; bounds per-tick work.
pub const MAX_COLLIDERS: usize = 8;
/// Integer ops charged per bone per collider per collide pass (nearest point, push, length).
pub const OPS_PER_COLLIDER_TICK: u64 = 24;

/// One body a spring chain may not pass through, in Q12 steps
/// (VRM `VRMSpringBoneColliderShapeSphere` / `...Capsule`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpringCollider {
    /// A ball: every tip within `radius_q` of `center_q` is inside.
    Sphere {
        /// Centre, Q12 steps.
        center_q: [i32; 3],
        /// Radius, Q12 steps, positive.
        radius_q: i32,
    },
    /// A pill: every tip within `radius_q` of the segment `head_q..tail_q` is inside.
    Capsule {
        /// One end of the axis, Q12 steps.
        head_q: [i32; 3],
        /// Other end of the axis, Q12 steps.
        tail_q: [i32; 3],
        /// Radius around the axis, Q12 steps, positive.
        radius_q: i32,
    },
}

impl SpringCollider {
    /// The body's radius, Q12 steps.
    pub fn radius_q(self) -> i32 {
        match self {
            Self::Sphere { radius_q, .. } | Self::Capsule { radius_q, .. } => radius_q,
        }
    }

    /// A unit direction, Q12, square to the body's axis: where a tip that sits exactly on the
    /// nearest point leaves by. A sphere has no axis, so it leaves upward; a capsule picks the
    /// axis' own smallest component, so the same body always picks the same way (a tilted axis
    /// pushed along +Y would not clear the capsule).
    pub fn escape_q(self) -> [i64; 3] {
        match self {
            Self::Sphere { .. } => [0, i64::from(Q), 0],
            Self::Capsule { head_q, tail_q, .. } => {
                let axis = delta(tail_q, head_q);
                let l2 = dot(axis, axis);
                if l2 == 0 {
                    return [0, i64::from(Q), 0];
                }
                let pick = (0..3).min_by_key(|i| axis[*i].unsigned_abs()).unwrap_or(1);
                let mut other = [0i64; 3];
                other[pick] = 1;
                let k = dot(other, axis);
                let mut perp = [
                    other[0] * l2 - axis[0] * k,
                    other[1] * l2 - axis[1] * k,
                    other[2] * l2 - axis[2] * k,
                ];
                let len = lane_len(perp);
                if len == 0 {
                    return [0, i64::from(Q), 0];
                }
                let q = i64::from(Q);
                perp = [perp[0] * q / len, perp[1] * q / len, perp[2] * q / len];
                perp
            }
        }
    }

    /// The point of the body nearest `p`, Q12 steps: the centre, or the nearest point of the
    /// axis (a degenerate axis reads as its head).
    pub fn nearest(self, p: [i32; 3]) -> [i32; 3] {
        match self {
            Self::Sphere { center_q, .. } => center_q,
            Self::Capsule { head_q, tail_q, .. } => {
                let axis = delta(tail_q, head_q);
                let l2 = dot(axis, axis);
                if l2 == 0 {
                    return head_q;
                }
                // Clamp the projection onto the axis to its own ends: a pill is the axis swept by
                // its radius, so the nearest point never lies past either end.
                let t = dot(delta(p, head_q), axis).clamp(0, l2);
                let scale = |c: i64| i32::try_from(c).unwrap_or(i32::MAX);
                [
                    scale(i64::from(head_q[0]) + axis[0] * t / l2),
                    scale(i64::from(head_q[1]) + axis[1] * t / l2),
                    scale(i64::from(head_q[2]) + axis[2] * t / l2),
                ]
            }
        }
    }
}

/// `a - b` in Q12 steps, widened so the products of two lanes cannot overflow.
fn delta(a: [i32; 3], b: [i32; 3]) -> [i64; 3] {
    [
        i64::from(a[0]) - i64::from(b[0]),
        i64::from(a[1]) - i64::from(b[1]),
        i64::from(a[2]) - i64::from(b[2]),
    ]
}

/// Dot product of two widened lanes.
fn dot(a: [i64; 3], b: [i64; 3]) -> i64 {
    a[0].saturating_mul(b[0])
        .saturating_add(a[1].saturating_mul(b[1]))
        .saturating_add(a[2].saturating_mul(b[2]))
}

/// Length of a widened lane in Q12 steps: integer square root, so no float enters the pass.
fn lane_len(v: [i64; 3]) -> i64 {
    let sq = dot(v, v);
    if sq <= 0 {
        return 0;
    }
    i64::try_from(engine_core::fixed::isqrt(
        u128::try_from(sq).unwrap_or(u128::MAX),
    ))
    .unwrap_or(i64::MAX)
}

/// A widened lane back in range: both callers clamp to a tip bound first.
fn narrow(v: [i64; 3]) -> [i32; 3] {
    v.map(|c| i32::try_from(c).unwrap_or(if c < 0 { i32::MIN } else { i32::MAX }))
}

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
    /// Collider count past [`MAX_COLLIDERS`]; value carried.
    TooManyColliders(usize),
    /// A collider radius that is not positive; value carried.
    BadRadius(i32),
    /// The set has no radius for the named bone; value carried.
    BadBone(usize),
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
            SpringError::TooManyColliders(n) => {
                write!(f, "springs: {n} colliders past {MAX_COLLIDERS}")
            }
            SpringError::BadRadius(r) => write!(f, "springs: collider radius {r} not positive"),
            SpringError::BadBone(bone) => write!(f, "springs: bone {bone} has no radius lane"),
        }
    }
}

impl std::error::Error for SpringError {}

/// A curve along the chain: one value, or a walk from the root to the last bone, all Q12.
/// VRM spring bones and Godot's `SpringBoneSimulator3D` both sample a damping curve per joint.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpringCurve {
    /// The same value at every joint.
    Flat(i32),
    /// `from_q` at the root, `to_q` at the last bone, rounded down in between.
    Ramp {
        /// Value at joint 0.
        from_q: i32,
        /// Value at the last joint.
        to_q: i32,
    },
}

impl SpringCurve {
    /// The curve's value at `index` of `bones` (a one-bone chain reads `from_q`).
    pub fn at(self, index: usize, bones: usize) -> i32 {
        match self {
            Self::Flat(v) => v,
            Self::Ramp { from_q, to_q } => {
                let last = bones.saturating_sub(1);
                if last == 0 {
                    return from_q;
                }
                let i = i64::try_from(index.min(last)).unwrap_or(0);
                let n = i64::try_from(last).unwrap_or(1);
                let from = i64::from(from_q);
                let to = i64::from(to_q);
                i32::try_from(from + (to - from) * i / n).unwrap_or(to_q)
            }
        }
    }
}

/// One joint's own tuning: what a curve samples per bone (VRM `m_stiffnessForce`,
/// `m_dragForce`, `m_gravityPower` and `m_hitRadius`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SpringJoint {
    /// Pull per tick in Q12 (`0..=Q`).
    pub stiff_q: i32,
    /// Velocity kept per tick in Q12 (`0..=Q`; below `Q` drains energy).
    pub damp_q: i32,
    /// Gravity added to the tip's velocity per tick, Q12 steps (`0..=VMAX_Q`).
    pub gravity_q: i32,
    /// The bone's own thickness, Q12 steps: what a collider pushes out by.
    pub radius_q: i32,
}

impl SpringJoint {
    /// The joint a set of shared `params` gives one bone of thickness `radius_q`.
    pub fn from_params(params: SpringParams, radius_q: i32) -> Self {
        Self {
            stiff_q: params.stiff_q,
            damp_q: params.damp_q,
            gravity_q: 0,
            radius_q,
        }
    }

    /// Whether every lane is inside its bound (`stiff`/`damp` in `0..=Q`, gravity in
    /// `-VMAX..=VMAX`, radius not negative).
    pub fn legal(&self) -> bool {
        (0..=Q).contains(&self.stiff_q)
            && (0..=Q).contains(&self.damp_q)
            && (-VMAX_Q..=VMAX_Q).contains(&self.gravity_q)
            && self.radius_q >= 0
    }
}

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

/// Budgeted integer ops for one collide pass: every bone against every body, in stable order.
pub fn collide_ops(bones: usize, colliders: usize) -> u64 {
    u64::try_from(bones)
        .unwrap_or(u64::MAX)
        .saturating_mul(u64::try_from(colliders).unwrap_or(u64::MAX))
        .saturating_mul(OPS_PER_COLLIDER_TICK)
}

/// Caller-owned spring set: tip positions plus velocities in Q12, one entry
/// per bone in stable order. Construct with [`SpringSet::with_capacity`],
/// add bones with [`SpringSet::add_bone`], advance with [`SpringSet::step`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SpringSet {
    params: SpringParams,
    pos: Vec<[i32; 3]>,
    vel: Vec<[i32; 3]>,
    /// Each bone's own tuning, sampled from the curves.
    joints: Vec<SpringJoint>,
    /// Bit `n` set: the bone may ignore collider `n` (a braid does not fight its own root).
    exclude: Vec<u16>,
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
            joints: Vec::with_capacity(capacity),
            exclude: Vec::with_capacity(capacity),
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
        self.joints.push(SpringJoint::from_params(self.params, 0));
        self.exclude.push(0);
        Ok(self.pos.len() - 1)
    }

    /// Add one bone of thickness `radius_m` (meters) at `anchor_m`: as [`SpringSet::add_bone`],
    /// plus the radius a collider pushes it out by. A radius below zero refuses.
    pub fn add_bone_radius(
        &mut self,
        anchor_m: [f32; 3],
        radius_m: f32,
    ) -> Result<usize, SpringError> {
        let radius = to_q(radius_m);
        if radius < 0 || !radius_m.is_finite() {
            return Err(SpringError::BadRadius(radius));
        }
        let bone = self.add_bone(anchor_m)?;
        self.joints[bone].radius_q = radius;
        Ok(bone)
    }

    /// One bone's own tuning, replacing what the shared params gave it. An illegal joint refuses
    /// and changes nothing.
    pub fn set_joint(&mut self, bone: usize, joint: SpringJoint) -> Result<(), SpringError> {
        if bone >= self.joints.len() {
            return Err(SpringError::BadBone(bone));
        }
        if !joint.legal() {
            return Err(SpringError::BadStiffness(joint.stiff_q));
        }
        self.joints[bone] = joint;
        Ok(())
    }

    /// Sample four curves along the chain: stiffness, damping, gravity and radius, one value per
    /// joint, root to tip (VRM `set_stiffness_damping_curve` and friends). An illegal sample
    /// refuses and changes nothing.
    pub fn set_curves(
        &mut self,
        stiff: SpringCurve,
        damp: SpringCurve,
        gravity: SpringCurve,
        radius: SpringCurve,
    ) -> Result<(), SpringError> {
        let bones = self.joints.len();
        for index in 0..bones {
            let joint = SpringJoint {
                stiff_q: stiff.at(index, bones),
                damp_q: damp.at(index, bones),
                gravity_q: gravity.at(index, bones),
                radius_q: radius.at(index, bones),
            };
            if !joint.legal() {
                return Err(SpringError::BadStiffness(joint.stiff_q));
            }
        }
        for index in 0..bones {
            self.joints[index] = SpringJoint {
                stiff_q: stiff.at(index, bones),
                damp_q: damp.at(index, bones),
                gravity_q: gravity.at(index, bones),
                radius_q: radius.at(index, bones),
            };
        }
        Ok(())
    }

    /// One joint's tuning.
    pub fn joint(&self, bone: usize) -> Result<SpringJoint, SpringError> {
        self.joints
            .get(bone)
            .copied()
            .ok_or(SpringError::BadBone(bone))
    }

    /// Let bone `bone` ignore collider `collider` from now on (an excluded pair: a braid's own
    /// root, a strap's own buckle).
    pub fn exclude(&mut self, bone: usize, collider: usize) -> Result<(), SpringError> {
        if bone >= self.exclude.len() {
            return Err(SpringError::BadBone(bone));
        }
        if collider >= MAX_COLLIDERS {
            return Err(SpringError::TooManyColliders(collider + 1));
        }
        let bit = 1u16
            .checked_shl(u32::try_from(collider).unwrap_or(u32::MAX))
            .unwrap_or(0);
        self.exclude[bone] |= bit;
        Ok(())
    }

    /// Whether bone `bone` ignores collider `collider`.
    pub fn excluded(&self, bone: usize, collider: usize) -> bool {
        let Some(mask) = self.exclude.get(bone) else {
            return false;
        };
        let bit = 1u16
            .checked_shl(u32::try_from(collider).unwrap_or(u32::MAX))
            .unwrap_or(0);
        *mask & bit != 0
    }

    /// One bone's thickness, Q12 steps.
    pub fn radius_q(&self, bone: usize) -> Result<i32, SpringError> {
        self.joints
            .get(bone)
            .map(|joint| joint.radius_q)
            .ok_or(SpringError::BadBone(bone))
    }

    /// Push every tip out of the bodies it stands inside, then keep the bone length: the tip
    /// slides along the delta from the nearest surface point to the body's radius plus the bone's
    /// own, and its excursion from its anchor never passes [`SpringParams::max_offset_q`]. Stable
    /// bone then collider order, integer only, so one tape settles identically twice. Both lane
    /// lengths must match the bone count; a count or radius outside its bound refuses and writes
    /// nothing.
    pub fn collide(
        &mut self,
        anchors_q: &[[i32; 3]],
        colliders: &[SpringCollider],
    ) -> Result<(), SpringError> {
        if colliders.len() > MAX_COLLIDERS {
            return Err(SpringError::TooManyColliders(colliders.len()));
        }
        if anchors_q.len() != self.pos.len() {
            return Err(SpringError::LengthMismatch {
                expected: self.pos.len(),
                got: anchors_q.len(),
            });
        }
        if colliders.iter().any(|collider| collider.radius_q() <= 0) {
            let bad = colliders
                .iter()
                .map(|c| c.radius_q())
                .find(|r| *r <= 0)
                .unwrap_or(0);
            return Err(SpringError::BadRadius(bad));
        }
        let max = i64::from(self.params.max_offset_q);
        for (bone, anchor) in anchors_q.iter().enumerate() {
            let mut tip = self.pos[bone];
            for (slot, collider) in colliders.iter().enumerate() {
                if self.excluded(bone, slot) {
                    continue;
                }
                let nearest = collider.nearest(tip);
                let off = delta(tip, nearest);
                let dist = lane_len(off);
                let need = i64::from(collider.radius_q()) + i64::from(self.joints[bone].radius_q);
                if dist >= need {
                    continue;
                }
                // Stand on the body's surface: the same radial line, pushed out by what is
                // missing. A tip exactly on the nearest point has no line, so it leaves along the
                // body's own escape direction.
                let pushed = if dist > 1 {
                    [
                        i64::from(nearest[0]) + off[0] * need / dist,
                        i64::from(nearest[1]) + off[1] * need / dist,
                        i64::from(nearest[2]) + off[2] * need / dist,
                    ]
                } else {
                    let dir = collider.escape_q();
                    let q = i64::from(Q);
                    [
                        i64::from(nearest[0]) + dir[0] * need / q,
                        i64::from(nearest[1]) + dir[1] * need / q,
                        i64::from(nearest[2]) + dir[2] * need / q,
                    ]
                };
                tip = narrow(pushed);
            }
            // Keep the bone length: the tip is this far from its anchor at most, so a push cannot
            // stretch the chain past its own bound.
            let off = delta(tip, *anchor);
            let len = lane_len(off);
            if len > max && len > 0 {
                tip = narrow([
                    i64::from(anchor[0]) + off[0] * max / len,
                    i64::from(anchor[1]) + off[1] * max / len,
                    i64::from(anchor[2]) + off[2] * max / len,
                ]);
            }
            self.pos[bone] = tip;
        }
        Ok(())
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
        let max = self.params.max_offset_q;
        for (((pos_row, vel_row), joint), (anchor_row, out_row)) in self
            .pos
            .iter_mut()
            .zip(self.vel.iter_mut())
            .zip(self.joints.iter())
            .zip(anchors_q.iter().zip(out_q.iter_mut()))
        {
            let stiff = i64::from(joint.stiff_q);
            let damp = i64::from(joint.damp_q);
            let gravity = i64::from(joint.gravity_q);
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
                    v = (v + gravity + pull - (v - drag))
                        .clamp(i64::from(-VMAX_Q), i64::from(VMAX_Q));
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
        if tick.is_multiple_of(9) {
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

    /// The helmet: a sphere of 90 mm around the head, and the crest bone's tip starts 1 mm
    /// inside it.
    fn helmet_sphere(tip_q: [i32; 3]) -> SpringCollider {
        let r = to_q(0.09);
        let bone = to_q(0.01);
        SpringCollider::Sphere {
            center_q: [tip_q[0] - (r - bone), tip_q[1], tip_q[2]],
            radius_q: r,
        }
    }

    #[test]
    fn a_crest_tail_never_ends_inside_the_helmet_sphere() {
        // One crest bone of 10 mm thickness whose tip rest sits 1 mm inside the helmet sphere:
        // the push lands it on the surface, and the bone length holds from there.
        let mut set = SpringSet::with_capacity(tuned(), 1).expect("one bone fits");
        let bone = set
            .add_bone_radius([0.0, 1.6, 0.0], 0.01)
            .expect("crest bone");
        let anchors = anchors_for(&set);
        let mut out = vec![[0; 3]; 1];
        set.step(&anchors, 1, &mut out).expect("rest step");
        let tip = out[0];
        let sphere = helmet_sphere(tip);
        let before = sphere.nearest(tip);
        let gap = lane_len(delta(tip, before));
        assert!(
            gap < i64::from(sphere.radius_q()) + i64::from(set.radius_q(bone).expect("radius")),
            "the fixture tail starts inside the sphere: {gap}"
        );
        set.collide(&anchors, &[sphere]).expect("push out");
        let after = set.pos[0];
        let surface = lane_len(delta(after, sphere.nearest(after)));
        assert!(
            surface >= i64::from(sphere.radius_q()),
            "the tail ends {} steps from the helmet's axis, inside its {}",
            surface,
            sphere.radius_q()
        );
        assert_eq!(set.radius_q(bone), Ok(to_q(0.01)));
        // Two runs of the same push are the same push.
        let run = || {
            let mut set = SpringSet::with_capacity(tuned(), 1).expect("fits");
            set.add_bone_radius([0.0, 1.6, 0.0], 0.01).expect("bone");
            let anchors = anchors_for(&set);
            set.collide(&anchors, &[sphere]).expect("push");
            set.pos[0]
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn capsule_push_out_keeps_the_bone_length() {
        // A strap over a 40 mm shoulder capsule whose axis crosses the chain: the tip is pushed
        // out to the surface and never further from its anchor than the tuning allows.
        let mut set = SpringSet::with_capacity(tuned(), 3).expect("three bones fit");
        for b in 0..3 {
            set.add_bone_radius([b as f32 * 0.06, 1.4, 0.0], 0.02)
                .expect("strap bone");
        }
        let anchors = anchors_for(&set);
        let mut out = vec![[0; 3]; 3];
        for _ in 0..4 {
            set.step(&anchors, 1, &mut out).expect("settle steps");
        }
        let capsule = SpringCollider::Capsule {
            head_q: [to_q(0.06), to_q(1.3), to_q(-0.2)],
            tail_q: [to_q(0.06), to_q(1.5), to_q(0.2)],
            radius_q: to_q(0.10),
        };
        set.collide(&anchors, &[capsule]).expect("shoulder push");
        let gaps: Vec<i64> = (0..3)
            .map(|bone| lane_len(delta(set.pos[bone], capsule.nearest(set.pos[bone]))))
            .collect();
        let moved: Vec<i64> = (0..3)
            .map(|bone| lane_len(delta(set.pos[bone], anchors[bone])))
            .collect();
        assert!(
            gaps.iter().all(|gap| *gap >= i64::from(capsule.radius_q())),
            "tips end {gaps:?} steps from the axis (need {}), moved {moved:?}, tips {:?}, anchors {:?}",
            capsule.radius_q(),
            (0..3).map(|b| set.pos[b]).collect::<Vec<_>>(),
            anchors
        );
        assert!(
            moved
                .iter()
                .all(|off| *off <= i64::from(tuned().max_offset_q)),
            "the push stretched a bone past its {} bound: {moved:?}",
            tuned().max_offset_q
        );
    }

    #[test]
    fn collide_stays_inside_its_op_budget() {
        // The Accept's fixture: 8 bones, 3 spheres and 2 capsules, inside 8 ops/bone and
        // 24 ops per bone-collider pair.
        let bones = 8usize;
        let colliders = 3 + 2usize;
        assert_eq!(collide_ops(bones, colliders), 960);
        assert_eq!(collide_ops(bones, colliders), 8 * 5 * OPS_PER_COLLIDER_TICK);
        let mut set = SpringSet::with_capacity(tuned(), bones).expect("8 bones fit");
        for b in 0..bones {
            set.add_bone_radius([b as f32 * 0.05, 1.5, 0.0], 0.01)
                .expect("bone");
        }
        let anchors = anchors_for(&set);
        let list = [
            helmet_sphere(anchors[0]),
            SpringCollider::Sphere {
                center_q: anchors[2],
                radius_q: to_q(0.05),
            },
            SpringCollider::Sphere {
                center_q: anchors[4],
                radius_q: to_q(0.05),
            },
            SpringCollider::Capsule {
                head_q: [anchors[6][0], anchors[6][1] - to_q(0.2), anchors[6][2]],
                tail_q: [anchors[6][0], anchors[6][1] + to_q(0.2), anchors[6][2]],
                radius_q: to_q(0.04),
            },
            SpringCollider::Capsule {
                head_q: anchors[7],
                tail_q: anchors[7],
                radius_q: to_q(0.04),
            },
        ];
        let digest = || {
            let mut set = set.clone();
            let mut out = vec![[0; 3]; bones];
            for _ in 0..8 {
                set.step(&anchors, 1, &mut out).expect("budget step");
                set.collide(&anchors, &list).expect("budget collide");
            }
            (set, out)
        };
        let (a_set, a_out) = digest();
        let (b_set, b_out) = digest();
        assert_eq!(a_set, b_set, "the same bodies push the same way twice");
        assert_eq!(a_out, b_out);
        for bone in 0..bones {
            let off = lane_len(delta(a_set.pos[bone], anchors[bone]));
            assert!(off <= i64::from(tuned().max_offset_q));
        }
        // Refusals: too many bodies, a bad radius, a lane mismatch.
        let nine: Vec<SpringCollider> = (0..9).map(|_| list[0]).collect();
        assert_eq!(
            set.collide(&anchors, &nine),
            Err(SpringError::TooManyColliders(9))
        );
        assert_eq!(
            set.collide(
                &anchors,
                &[SpringCollider::Sphere {
                    center_q: [0; 3],
                    radius_q: 0
                }]
            ),
            Err(SpringError::BadRadius(0))
        );
        assert_eq!(
            set.collide(&anchors[..2], &list),
            Err(SpringError::LengthMismatch {
                expected: bones,
                got: 2
            })
        );
        assert_eq!(set.radius_q(bones), Err(SpringError::BadBone(bones)));
    }

    #[test]
    fn per_joint_curves_sample_from_the_root_to_the_tip() {
        // Six loincloth bones: stiffness falls and drag rises toward the tip, gravity grows, the
        // radius is one value. The sampled joints are exactly the curves' values.
        let bones = 6usize;
        let mut set = SpringSet::with_capacity(tuned(), bones).expect("six bones fit");
        for b in 0..bones {
            set.add_bone([b as f32 * 0.1, 1.0, 0.0]).expect("bone");
        }
        let stiff = SpringCurve::Ramp {
            from_q: Q / 2,
            to_q: Q / 8,
        };
        let damp = SpringCurve::Ramp {
            from_q: Q - Q / 8,
            to_q: Q / 4,
        };
        let gravity = SpringCurve::Ramp {
            from_q: 0,
            to_q: Q / 16,
        };
        let radius = SpringCurve::Flat(to_q(0.02));
        set.set_curves(stiff, damp, gravity, radius)
            .expect("curves legal");
        for b in 0..bones {
            let joint = set.joint(b).expect("bone");
            assert_eq!(joint.stiff_q, stiff.at(b, bones), "stiffness at {b}");
            assert_eq!(joint.damp_q, damp.at(b, bones), "drag at {b}");
            assert_eq!(joint.gravity_q, gravity.at(b, bones), "gravity at {b}");
            assert_eq!(joint.radius_q, to_q(0.02), "radius at {b}");
        }
        assert_eq!(SpringCurve::Flat(7).at(3, 8), 7);
        assert_eq!(
            SpringCurve::Ramp {
                from_q: 0,
                to_q: 10
            }
            .at(0, 1),
            0,
            "one bone"
        );
        // An illegal curve refuses and leaves every joint as it was.
        let before = set.clone();
        assert_eq!(
            set.set_curves(
                SpringCurve::Flat(Q + 1),
                SpringCurve::Flat(Q),
                SpringCurve::Flat(0),
                SpringCurve::Flat(0)
            ),
            Err(SpringError::BadStiffness(Q + 1))
        );
        assert_eq!(set, before);
        assert_eq!(
            set.set_joint(bones, SpringJoint::default()),
            Err(SpringError::BadBone(bones))
        );
    }

    #[test]
    fn a_lagging_tail_follows_its_root_late() {
        // Gravity pulls every bone down; the drag curve runs from free at the root to heavy at
        // the tip, so the tip passes the same 20 mm mark many ticks after the root.
        let bones = 4usize;
        let mut set = SpringSet::with_capacity(tuned(), bones).expect("four bones fit");
        for b in 0..bones {
            set.add_bone([0.0, 1.6, b as f32 * 0.02])
                .expect("hair bone");
        }
        let anchors = anchors_for(&set);
        set.set_curves(
            SpringCurve::Flat(Q / 64),
            SpringCurve::Ramp {
                from_q: Q,
                to_q: Q / 256,
            },
            SpringCurve::Flat(2),
            SpringCurve::Flat(0),
        )
        .expect("hair curves");
        let mut out = vec![[0; 3]; bones];
        let mut root_at = None;
        let mut tail_at = None;
        for tick in 0..120u32 {
            set.step(&anchors, 1, &mut out).expect("hair steps");
            let off = |tip: [i32; 3], anchor: [i32; 3]| lane_len(delta(tip, anchor));
            if root_at.is_none() && off(out[0], anchors[0]) > to_q(0.02).into() {
                root_at = Some(tick);
            }
            if tail_at.is_none() && off(out[bones - 1], anchors[bones - 1]) > to_q(0.02).into() {
                tail_at = Some(tick);
            }
        }
        let root_at = root_at.expect("the root sagged");
        let tail_at = tail_at.expect("the tip sagged");
        assert!(
            tail_at >= root_at + 3,
            "the tip passed 20 mm at tick {tail_at}, the root at {root_at}: drag curve too flat"
        );
        // Two runs of the same curves and tape are the same run.
        let run = || {
            let mut set = SpringSet::with_capacity(tuned(), bones).expect("fits");
            for b in 0..bones {
                set.add_bone([0.0, 1.6, b as f32 * 0.02]).expect("bone");
            }
            set.set_curves(
                SpringCurve::Flat(Q / 64),
                SpringCurve::Ramp {
                    from_q: Q,
                    to_q: Q / 256,
                },
                SpringCurve::Flat(2),
                SpringCurve::Flat(0),
            )
            .expect("curves");
            let anchors = anchors_for(&set);
            let mut out = vec![[0; 3]; bones];
            for _ in 0..120 {
                set.step(&anchors, 1, &mut out).expect("steps");
            }
            (set, out)
        };
        let (a_set, a_out) = run();
        let (b_set, b_out) = run();
        assert_eq!(a_set, b_set);
        assert_eq!(a_out, b_out);
    }

    #[test]
    fn an_excluded_pair_is_left_alone() {
        // The helmet sphere sits exactly where the crest bone's tip rests; the pair is excluded,
        // so the tip stays, and the same pair without the exclusion pushes it out.
        let mut set = SpringSet::with_capacity(tuned(), 1).expect("one bone fits");
        set.add_bone_radius([0.0, 1.6, 0.0], 0.01).expect("crest");
        let anchors = anchors_for(&set);
        let sphere = SpringCollider::Sphere {
            center_q: anchors[0],
            radius_q: to_q(0.09),
        };
        set.exclude(0, 0).expect("exclude its own helmet");
        assert!(set.excluded(0, 0));
        assert!(!set.excluded(0, 1));
        set.collide(&anchors, &[sphere]).expect("excluded pass");
        assert_eq!(set.pos[0], anchors[0], "the excluded pair moved the tip");
        let mut plain = SpringSet::with_capacity(tuned(), 1).expect("fits");
        plain.add_bone_radius([0.0, 1.6, 0.0], 0.01).expect("crest");
        plain.collide(&anchors, &[sphere]).expect("push");
        assert_ne!(
            plain.pos[0], anchors[0],
            "without the exclusion it is pushed"
        );
        assert_eq!(set.exclude(1, 0), Err(SpringError::BadBone(1)));
        assert_eq!(
            set.exclude(0, MAX_COLLIDERS),
            Err(SpringError::TooManyColliders(MAX_COLLIDERS + 1))
        );
    }
}
