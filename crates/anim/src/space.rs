//! space: 2D locomotion blend space over the animation graph banks.
//!
//! Owner: `engine-anim`; consumers: the arena match plus render
//! skinning read [`SpacePlayer::tick`] through the `engine-anim` API.
//! Units: Q10 speed (m/s), decidegrees angle, ticks, meters plus unit
//! quats out. Determinism: integer IDW weights, stable sample order,
//! `+ - * /` plus `sqrt` only, no clock. Errors: [`SpaceError`] at bake,
//! [`GraphError`] at tick. Edge: shares the motion-match `DECAY_DIV`/`Q` scale,
//! never forks it. Budget: tick is `samples * joints` lanes, at most 64
//! samples; `prepare` owns every buffer, ticks reuse them with zero
//! steady-state allocation (capacity-reuse test below).

use super::graph::{ClipBank, GraphError, Pose};
use super::inertialize::AngularLimiter;
use super::motion_match::{max_lane_abs, q as q10, DECAY_DIV, Q};
use engine_core::Fnv64;

/// Largest accepted sample count; bounds per-tick scan memory.
pub const MAX_SAMPLES: usize = 64;
/// Integer weight quantum: per-sample weights sum to exactly this.
pub const WEIGHT_SCALE: u32 = 65_536;
/// Fastest accepted speed in Q10 steps (12 m/s, the full-scale speed).
pub const MAX_SPEED_Q: i32 = 12 * Q;
/// Largest accepted heading magnitude in decidegrees (180 degrees).
pub const MAX_ANGLE_DECIDEG: i32 = 1800;
/// Angle-to-speed distance weight: one decidegree costs eight Q10 steps.
const ANGLE_W: i64 = 8;
/// Inverse-distance numerator; dwarfs any legal squared distance.
const INV_BIG: i64 = 1 << 40;
/// Identity quat for degenerate rotation sums.
const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// Blend-space authoring failures; every variant names its bound.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SpaceError {
    /// No samples to bake.
    EmptySpace,
    /// Sample count over [`MAX_SAMPLES`], count carried.
    TooManySamples(usize),
    /// A param is out of range; the lane names the problem.
    BadParam(&'static str),
    /// Two samples share one `(speed, angle)` point.
    DuplicateSample {
        /// Speed in Q10 steps.
        speed_q: i32,
        /// Heading in decidegrees.
        angle_q: i32,
    },
}

impl std::fmt::Display for SpaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpaceError::EmptySpace => write!(f, "space: no samples to bake"),
            SpaceError::TooManySamples(count) => {
                write!(f, "space: {count} samples exceeds {MAX_SAMPLES}")
            }
            SpaceError::BadParam(lane) => write!(f, "space: bad param ({lane})"),
            SpaceError::DuplicateSample { speed_q, angle_q } => {
                write!(f, "space: duplicate sample ({speed_q}, {angle_q})")
            }
        }
    }
}

impl std::error::Error for SpaceError {}

/// One range check for every `(speed, angle)` point, query or sample.
fn check_point(speed_q: i32, angle_q: i32) -> Result<(), SpaceError> {
    if !(0..=MAX_SPEED_Q).contains(&speed_q) {
        return Err(SpaceError::BadParam("speed"));
    }
    if !(-MAX_ANGLE_DECIDEG..=MAX_ANGLE_DECIDEG).contains(&angle_q) {
        return Err(SpaceError::BadParam("angle"));
    }
    Ok(())
}

/// One 2D blend query: forward speed plus heading offset from facing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SpaceParams {
    speed_q: i32,
    angle_q: i32,
}

impl SpaceParams {
    /// Build from authoring floats: `speed_mps` in `0..=12`, `angle_deg`
    /// in `-180..=180` (+right). Non-finite or out-of-range is refused.
    pub fn new(speed_mps: f32, angle_deg: f32) -> Result<Self, SpaceError> {
        if !(0.0..=12.0).contains(&speed_mps) {
            return Err(SpaceError::BadParam("speed"));
        }
        if !(-180.0..=180.0).contains(&angle_deg) {
            return Err(SpaceError::BadParam("angle"));
        }
        Ok(Self {
            speed_q: (speed_mps * Q as f32) as i32,
            angle_q: (angle_deg * 10.0) as i32,
        })
    }

    /// Build from fixed point: Q10 speed plus decidegree heading.
    pub fn from_fixed(speed_q: i32, angle_q: i32) -> Result<Self, SpaceError> {
        let params = Self { speed_q, angle_q };
        params.check()?;
        Ok(params)
    }

    fn check(&self) -> Result<(), SpaceError> {
        check_point(self.speed_q, self.angle_q)
    }

    /// Speed in Q10 steps (m/s).
    pub fn speed_q(&self) -> i32 {
        self.speed_q
    }

    /// Heading in decidegrees (+right of facing).
    pub fn angle_q(&self) -> i32 {
        self.angle_q
    }
}

/// One blend sample: a fixed `(speed, angle)` point plus its clip key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SpaceSample {
    speed_q: i32,
    angle_q: i32,
    clip: u32,
}

impl SpaceSample {
    /// Build one sample; out-of-range points are refused loudly.
    pub fn new(speed_q: i32, angle_q: i32, clip: u32) -> Result<Self, SpaceError> {
        let sample = Self {
            speed_q,
            angle_q,
            clip,
        };
        sample.check()?;
        Ok(sample)
    }

    fn check(&self) -> Result<(), SpaceError> {
        check_point(self.speed_q, self.angle_q)
    }

    /// Speed in Q10 steps.
    pub fn speed_q(&self) -> i32 {
        self.speed_q
    }

    /// Heading in decidegrees.
    pub fn angle_q(&self) -> i32 {
        self.angle_q
    }

    /// Clip-bank key sampled at this point.
    pub fn clip(&self) -> u32 {
        self.clip
    }
}

/// Baked 2D space: samples in canonical `(speed, angle, clip)` order.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BlendSpace {
    samples: Vec<SpaceSample>,
}

impl BlendSpace {
    /// Sample count.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// True when the space holds no samples (unreachable via [`bake`]).
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Samples in canonical order.
    pub fn samples(&self) -> &[SpaceSample] {
        &self.samples
    }
}

/// Bake samples into canonical order; rejects empties, oversize sets,
/// out-of-range points and duplicate `(speed, angle)` pairs.
pub fn bake(mut samples: Vec<SpaceSample>) -> Result<BlendSpace, SpaceError> {
    if samples.is_empty() {
        return Err(SpaceError::EmptySpace);
    }
    if samples.len() > MAX_SAMPLES {
        return Err(SpaceError::TooManySamples(samples.len()));
    }
    for sample in &samples {
        sample.check()?;
    }
    samples.sort_by_key(|sample| (sample.speed_q, sample.angle_q, sample.clip));
    for window in samples.windows(2) {
        if window[0].speed_q == window[1].speed_q && window[0].angle_q == window[1].angle_q {
            return Err(SpaceError::DuplicateSample {
                speed_q: window[1].speed_q,
                angle_q: window[1].angle_q,
            });
        }
    }
    Ok(BlendSpace { samples })
}

/// FNV-1a64 over the canonical samples (speed, angle, clip).
pub fn space_hash(space: &BlendSpace) -> u64 {
    let mut hash = Fnv64::new();
    hash.write_len(space.len());
    for sample in &space.samples {
        hash.write(&sample.speed_q.to_le_bytes());
        hash.write(&sample.angle_q.to_le_bytes());
        hash.write(&sample.clip.to_le_bytes());
    }
    hash.finish()
}

/// Fixed back to float meters.
fn deq10(steps: i32) -> f32 {
    steps as f32 / Q as f32
}

/// Squared 2D distance with the angle lane scaled into Q10 shape.
fn dist2(sample: &SpaceSample, params: SpaceParams) -> i64 {
    let dspeed = i64::from(sample.speed_q) - i64::from(params.speed_q);
    let dangle = (i64::from(sample.angle_q) - i64::from(params.angle_q)) * ANGLE_W;
    dspeed * dspeed + dangle * dangle
}

/// Error-path sample label; success ticks never format.
fn sample_name(index: usize) -> String {
    format!("space[{index}]")
}

/// Integer inverse-distance weights for `params`: exact hits take the full
/// quantum, otherwise every weight is `SCALE * inv / sum` with the leftover
/// dealt to the largest remainders in stable index order, so the row sums
/// to exactly [`WEIGHT_SCALE`]. `out` must span the samples.
pub fn weights_into(
    space: &BlendSpace,
    params: SpaceParams,
    out: &mut [u32],
) -> Result<(), GraphError> {
    if out.len() != space.len() {
        return Err(GraphError::LengthMismatch {
            expected: space.len(),
            got: out.len(),
        });
    }
    for (index, sample) in space.samples.iter().enumerate() {
        if sample.speed_q == params.speed_q && sample.angle_q == params.angle_q {
            for (slot, weight) in out.iter_mut().enumerate() {
                *weight = if slot == index { WEIGHT_SCALE } else { 0 };
            }
            return Ok(());
        }
    }
    let mut sum: i64 = 0;
    for sample in &space.samples {
        sum += INV_BIG / (dist2(sample, params) + 1);
    }
    if sum <= 0 {
        return Err(GraphError::NonFinite("space.weights"));
    }
    let scale = i64::from(WEIGHT_SCALE);
    let mut floored: i64 = 0;
    for (weight, sample) in out.iter_mut().zip(space.samples.iter()) {
        let inv = INV_BIG / (dist2(sample, params) + 1);
        *weight = (inv * scale / sum) as u32;
        floored += i64::from(*weight);
    }
    // Largest remainders, ties to the lowest index; stack flags only.
    let mut rest = scale - floored;
    let mut used = [false; MAX_SAMPLES];
    while rest > 0 {
        let mut best = 0usize;
        let mut best_rem = -1i64;
        for (index, sample) in space.samples.iter().enumerate() {
            if used[index] {
                continue;
            }
            let inv = INV_BIG / (dist2(sample, params) + 1);
            let rem = inv * scale % sum;
            if rem > best_rem {
                best_rem = rem;
                best = index;
            }
        }
        used[best] = true;
        out[best] += 1;
        rest -= 1;
    }
    Ok(())
}

/// Resize without reallocating unless the shape changed, then zero.
fn sized<T: Copy>(buf: &mut Vec<T>, count: usize, zero: T) {
    if buf.len() != count {
        buf.clear();
        buf.reserve(count);
        buf.resize(count, zero);
    } else {
        buf.fill(zero);
    }
}

/// Inertialized 2D space player: blends bank poses by integer weights and
/// carries interruptions on decaying Q10 offsets, so the first tick after
/// an interrupt equals the pre-interrupt pose exactly, then settles onto
/// the new target. Positions carry the motion-match `/8, minimum one step` decay;
/// rotations pass a last-stage angular-speed limiter (1080 deg/s, 18 deg per tick).
/// `prepare` owns every buffer; `tick` reuses them, allocating nothing.
#[derive(Clone, Debug, Default)]
pub struct SpacePlayer {
    weights: Vec<u32>,
    acc: Vec<[i64; 3]>,
    target: Vec<[i32; 3]>,
    off: Vec<[i32; 3]>,
    prev: Vec<[i32; 3]>,
    rot_limit: AngularLimiter,
    joints: usize,
    primed: bool,
}

impl SpacePlayer {
    /// Empty player; [`SpacePlayer::prepare`] before [`SpacePlayer::tick`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Size scratch for `joints` wide and `samples` many; resets offsets.
    /// Reallocates only when the shape changes; repeat prepares reuse.
    pub fn prepare(&mut self, joints: usize, samples: usize) {
        self.joints = joints;
        sized(&mut self.weights, samples, 0);
        sized(&mut self.acc, joints, [0; 3]);
        sized(&mut self.target, joints, [0; 3]);
        sized(&mut self.off, joints, [0; 3]);
        sized(&mut self.prev, joints, [0; 3]);
        self.rot_limit.prepare(joints);
        self.primed = false;
    }

    /// Prepared joint width.
    pub fn joints(&self) -> usize {
        self.joints
    }

    /// Prepared sample count.
    pub fn sample_count(&self) -> usize {
        self.weights.len()
    }

    /// Integer weights from the last [`SpacePlayer::tick`]: one per prepared
    /// sample in canonical order, summing to [`WEIGHT_SCALE`] after a tick.
    pub fn weights(&self) -> &[u32] {
        &self.weights
    }

    /// True when every inertial offset lane reached zero.
    pub fn settled(&self) -> bool {
        self.off.iter().flatten().all(|lane| *lane == 0)
    }

    /// Largest absolute offset lane in Q10 steps (settle meter).
    pub fn max_offset_q(&self) -> i32 {
        max_lane_abs(&self.off)
    }

    /// Evaluate one fixed tick: blend the space at `params` from `clips`,
    /// inertialize when `interrupt` is set, write the pose to `out`.
    /// Steady-state calls allocate nothing; bank and shape failures are
    /// loud [`GraphError`]s naming the sample at fault.
    pub fn tick(
        &mut self,
        space: &BlendSpace,
        clips: &ClipBank,
        params: SpaceParams,
        interrupt: bool,
        out: &mut Pose,
    ) -> Result<(), GraphError> {
        self.check_tick_shape(space, out)?;
        let joints = self.joints;
        weights_into(space, params, &mut self.weights)?;
        for slot in self.acc.iter_mut() {
            *slot = [0; 3];
        }
        for slot in out.rotations.iter_mut() {
            *slot = [0.0; 4];
        }
        self.accumulate_samples(space, clips, out)?;
        normalize_quats(&mut out.rotations);
        self.rot_limit
            .limit_in_place(&mut out.rotations, 1.0 / 60.0);
        let scale = i64::from(WEIGHT_SCALE);
        for joint in 0..joints {
            self.target[joint][0] = (self.acc[joint][0] / scale) as i32;
            self.target[joint][1] = (self.acc[joint][1] / scale) as i32;
            self.target[joint][2] = (self.acc[joint][2] / scale) as i32;
        }
        if interrupt && self.primed {
            // Capture src-minus-dst so this tick still shows the old pose.
            for joint in 0..joints {
                self.off[joint][0] = self.prev[joint][0].saturating_sub(self.target[joint][0]);
                self.off[joint][1] = self.prev[joint][1].saturating_sub(self.target[joint][1]);
                self.off[joint][2] = self.prev[joint][2].saturating_sub(self.target[joint][2]);
            }
        } else if !self.primed {
            for slot in self.off.iter_mut() {
                *slot = [0; 3];
            }
        }
        for joint in 0..joints {
            for lane in 0..3 {
                let mixed = self.target[joint][lane].saturating_add(self.off[joint][lane]);
                self.prev[joint][lane] = mixed;
                out.positions[joint][lane] = deq10(mixed);
                // Motion-match decay: eighths per tick with a one-step minimum.
                let decay = self.off[joint][lane] / DECAY_DIV;
                let step = if decay == 0 {
                    self.off[joint][lane].signum()
                } else {
                    decay
                };
                self.off[joint][lane] = self.off[joint][lane].saturating_sub(step);
            }
        }
        self.primed = true;
        Ok(())
    }

    /// Shape gate for one tick: prepared, one weight per sample, `out` as wide
    /// as the prepared joint count.
    fn check_tick_shape(&self, space: &BlendSpace, out: &Pose) -> Result<(), GraphError> {
        if self.joints == 0 {
            return Err(GraphError::NotPrepared);
        }
        if self.weights.len() != space.len() {
            return Err(GraphError::LengthMismatch {
                expected: self.weights.len(),
                got: space.len(),
            });
        }
        if out.positions.len() != self.joints || out.rotations.len() != self.joints {
            let got = if out.positions.len() != self.joints {
                out.positions.len()
            } else {
                out.rotations.len()
            };
            return Err(GraphError::LengthMismatch {
                expected: self.joints,
                got,
            });
        }
        Ok(())
    }

    /// Sum every sample's weighted Q10 positions into `acc` and its weighted
    /// quats into `out.rotations`, in sample index order; both start zeroed.
    fn accumulate_samples(
        &mut self,
        space: &BlendSpace,
        clips: &ClipBank,
        out: &mut Pose,
    ) -> Result<(), GraphError> {
        let joints = self.joints;
        let first = clips
            .get(space.samples[0].clip)
            .ok_or_else(|| GraphError::UnknownClip {
                node: sample_name(0),
                clip: space.samples[0].clip,
            })?;
        if first.positions.len() != joints || first.rotations.len() != joints {
            return Err(GraphError::LengthMismatch {
                expected: joints,
                got: first.positions.len(),
            });
        }
        for (index, sample) in space.samples.iter().enumerate() {
            let pose = clips
                .get(sample.clip)
                .ok_or_else(|| GraphError::UnknownClip {
                    node: sample_name(index),
                    clip: sample.clip,
                })?;
            if pose.positions.len() != joints || pose.rotations.len() != joints {
                return Err(GraphError::LengthMismatch {
                    expected: joints,
                    got: pose.positions.len(),
                });
            }
            let w_q = i64::from(self.weights[index]);
            let w_f = self.weights[index] as f32 / WEIGHT_SCALE as f32;
            for joint in 0..joints {
                let position = pose.positions[joint];
                self.acc[joint][0] += w_q * i64::from(q10(position[0]));
                self.acc[joint][1] += w_q * i64::from(q10(position[1]));
                self.acc[joint][2] += w_q * i64::from(q10(position[2]));
                // Shortest-arc alignment to the first sample keeps the
                // weighted quat sum on one hemisphere, in index order.
                let quat = pose.rotations[joint];
                let anchor = first.rotations[joint];
                let dot = quat[0] * anchor[0]
                    + quat[1] * anchor[1]
                    + quat[2] * anchor[2]
                    + quat[3] * anchor[3];
                let sign = if dot < 0.0 { -1.0 } else { 1.0 };
                out.rotations[joint][0] += w_f * sign * quat[0];
                out.rotations[joint][1] += w_f * sign * quat[1];
                out.rotations[joint][2] += w_f * sign * quat[2];
                out.rotations[joint][3] += w_f * sign * quat[3];
            }
        }
        Ok(())
    }
}

/// Normalize each accumulated quat; a non-finite or zero-length sum becomes
/// the identity.
fn normalize_quats(rotations: &mut [[f32; 4]]) {
    for quat in rotations.iter_mut() {
        let norm2 = quat[0] * quat[0] + quat[1] * quat[1] + quat[2] * quat[2] + quat[3] * quat[3];
        if norm2.is_finite() && norm2 > 0.0 {
            let inv = 1.0 / norm2.sqrt();
            quat[0] *= inv;
            quat[1] *= inv;
            quat[2] *= inv;
            quat[3] *= inv;
        } else {
            *quat = IDENTITY;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::pose_bits;

    fn pose2(p0: [f32; 3], p1: [f32; 3]) -> Pose {
        Pose {
            positions: vec![p0, p1],
            rotations: vec![IDENTITY, IDENTITY],
        }
    }

    fn bank_ab() -> ClipBank {
        let mut bank = ClipBank::new();
        bank.insert(10, pose2([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]))
            .unwrap();
        bank.insert(11, pose2([4.0, 0.0, 0.0], [0.0, 2.0, 0.0]))
            .unwrap();
        bank
    }

    fn space_ab() -> BlendSpace {
        bake(vec![
            SpaceSample::new(0, 0, 10).unwrap(),
            SpaceSample::new(2048, 0, 11).unwrap(),
        ])
        .unwrap()
    }

    fn params(speed_q: i32, angle_q: i32) -> SpaceParams {
        SpaceParams::from_fixed(speed_q, angle_q).unwrap()
    }

    fn player2() -> SpacePlayer {
        let mut player = SpacePlayer::new();
        player.prepare(2, 2);
        assert_eq!(player.joints(), 2);
        assert_eq!(player.sample_count(), 2);
        player
    }

    #[test]
    fn bake_sorts_and_rejects_loudly() {
        let space = bake(vec![
            SpaceSample::new(2048, 0, 11).unwrap(),
            SpaceSample::new(0, 0, 10).unwrap(),
        ])
        .unwrap();
        assert_eq!(space.len(), 2);
        assert!(!space.is_empty());
        assert_eq!(space, space_ab());
        assert_eq!(space_hash(&space), space_hash(&space_ab()));
        assert_eq!(bake(Vec::new()), Err(SpaceError::EmptySpace));
        let many = (0..MAX_SAMPLES + 1)
            .map(|i| SpaceSample::new((i * 10) as i32, 0, i as u32).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(bake(many), Err(SpaceError::TooManySamples(MAX_SAMPLES + 1)));
        assert_eq!(
            SpaceSample::new(-1, 0, 10),
            Err(SpaceError::BadParam("speed"))
        );
        assert_eq!(
            SpaceSample::new(MAX_SPEED_Q + 1, 0, 10),
            Err(SpaceError::BadParam("speed"))
        );
        assert_eq!(
            SpaceSample::new(0, MAX_ANGLE_DECIDEG + 1, 10),
            Err(SpaceError::BadParam("angle"))
        );
        let dup = bake(vec![
            SpaceSample::new(0, 0, 10).unwrap(),
            SpaceSample::new(0, 0, 11).unwrap(),
        ]);
        assert_eq!(
            dup,
            Err(SpaceError::DuplicateSample {
                speed_q: 0,
                angle_q: 0
            })
        );
        let text = dup.unwrap_err().to_string();
        assert!(text.contains("duplicate") && text.contains('0'), "{text}");
        assert!(SpaceError::EmptySpace.to_string().contains("no samples"));
        assert!(SpaceError::TooManySamples(65).to_string().contains("65"));
    }

    #[test]
    fn params_validate_ranges() {
        assert_eq!(
            SpaceParams::new(f32::NAN, 0.0),
            Err(SpaceError::BadParam("speed"))
        );
        assert_eq!(
            SpaceParams::new(-1.0, 0.0),
            Err(SpaceError::BadParam("speed"))
        );
        assert_eq!(
            SpaceParams::new(13.0, 0.0),
            Err(SpaceError::BadParam("speed"))
        );
        assert_eq!(
            SpaceParams::new(1.0, 200.0),
            Err(SpaceError::BadParam("angle"))
        );
        assert_eq!(
            SpaceParams::from_fixed(-5, 0),
            Err(SpaceError::BadParam("speed"))
        );
        assert_eq!(
            SpaceParams::from_fixed(0, 1801),
            Err(SpaceError::BadParam("angle"))
        );
        let mid = SpaceParams::new(2.0, 45.0).unwrap();
        assert_eq!((mid.speed_q(), mid.angle_q()), (2048, 450));
        assert_eq!(SpaceParams::new(0.0, -180.0).unwrap().angle_q(), -1800);
    }

    #[test]
    fn weights_exact_sum_and_stable() {
        let space = space_ab();
        let mut weights = vec![0u32; 2];
        weights_into(&space, params(0, 0), &mut weights).unwrap();
        assert_eq!(weights, vec![WEIGHT_SCALE, 0]);
        weights_into(&space, params(2048, 0), &mut weights).unwrap();
        assert_eq!(weights, vec![0, WEIGHT_SCALE]);
        // Off-grid rows sum to exactly one quantum and repeat identically.
        weights_into(&space, params(1024, 450), &mut weights).unwrap();
        assert_eq!(weights.iter().sum::<u32>(), WEIGHT_SCALE);
        let mut again = vec![0u32; 2];
        weights_into(&space, params(1024, 450), &mut again).unwrap();
        assert_eq!(weights, again);
        // Authoring order never changes the baked order or the weights.
        let flipped = bake(vec![
            SpaceSample::new(2048, 0, 11).unwrap(),
            SpaceSample::new(0, 0, 10).unwrap(),
        ])
        .unwrap();
        assert_eq!(space_hash(&flipped), space_hash(&space));
        weights_into(&flipped, params(1024, 450), &mut again).unwrap();
        assert_eq!(weights, again);
        // Symmetric midpoint splits the quantum evenly.
        weights_into(&space, params(1024, 0), &mut weights).unwrap();
        assert_eq!(weights, vec![WEIGHT_SCALE / 2, WEIGHT_SCALE / 2]);
        assert_eq!(
            weights_into(&space, params(0, 0), &mut [0u32; 1]).unwrap_err(),
            GraphError::LengthMismatch {
                expected: 2,
                got: 1
            }
        );
    }

    #[test]
    fn eval_matches_hand_rolled_midpoint() {
        let (space, bank) = (space_ab(), bank_ab());
        let mut player = player2();
        let mut out = Pose::zero(2);
        player
            .tick(&space, &bank, params(1024, 0), false, &mut out)
            .unwrap();
        // Hand-rolled baseline: symmetric 50/50 of [0,0,0]/[4,0,0] and
        // [0,0,0]/[0,2,0] per lane.
        assert_eq!(out.positions[0], [2.0, 0.0, 0.0]);
        assert_eq!(out.positions[1], [0.0, 1.0, 0.0]);
        assert_eq!(out.rotations, vec![IDENTITY, IDENTITY]);
        // Endpoints are exact: grid points reproduce their clips bit for bit.
        player
            .tick(&space, &bank, params(0, 0), false, &mut out)
            .unwrap();
        assert_eq!(out, bank.get(10).unwrap().clone());
        player
            .tick(&space, &bank, params(2048, 0), false, &mut out)
            .unwrap();
        assert_eq!(out, bank.get(11).unwrap().clone());
    }

    #[test]
    fn interrupt_has_no_pop_then_settles() {
        let (space, bank) = (space_ab(), bank_ab());
        let mut player = player2();
        let mut out = Pose::zero(2);
        for _ in 0..40 {
            player
                .tick(&space, &bank, params(0, 0), false, &mut out)
                .unwrap();
        }
        assert!(player.settled());
        let before = out.clone();
        // The interrupt tick still shows the old pose: zero velocity pop.
        player
            .tick(&space, &bank, params(2048, 0), true, &mut out)
            .unwrap();
        assert_eq!(out, before);
        assert!(player.max_offset_q() > 0);
        let peak = player.max_offset_q();
        for _ in 0..120 {
            player
                .tick(&space, &bank, params(2048, 0), false, &mut out)
                .unwrap();
        }
        assert!(player.settled(), "max={}", player.max_offset_q());
        assert_eq!(player.max_offset_q(), 0);
        assert!(peak > player.max_offset_q());
        // Settled output lands exactly on the uninterrupted target.
        let mut twin = player2();
        let mut want = Pose::zero(2);
        twin.tick(&space, &bank, params(2048, 0), false, &mut want)
            .unwrap();
        assert_eq!(out, want);
    }

    #[test]
    fn ang_limit_space_player_caps_180_deg_step() {
        use std::f32::consts::PI;
        #[expect(
            clippy::disallowed_methods,
            reason = "test-only angle oracle; production keeps polynomial helpers"
        )]
        fn angle_deg(a: [f32; 4], b: [f32; 4]) -> f32 {
            let mut dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
            if !dot.is_finite() {
                return 180.0;
            }
            if dot < 0.0 {
                dot = -dot;
            }
            if dot > 1.0 {
                dot = 1.0;
            }
            2.0 * dot.acos() * 180.0 / PI
        }
        const ID: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
        const HALF_Y: [f32; 4] = [0.0, 1.0, 0.0, 0.0];
        fn pose_pair(r0: [f32; 4]) -> Pose {
            Pose {
                positions: vec![[0.0; 3], [0.0; 3]],
                rotations: vec![r0, ID],
            }
        }
        let mut bank = ClipBank::new();
        bank.insert(20, pose_pair(ID)).unwrap();
        bank.insert(21, pose_pair(HALF_Y)).unwrap();
        let space = bake(vec![
            SpaceSample::new(0, 0, 20).unwrap(),
            SpaceSample::new(2048, 0, 21).unwrap(),
        ])
        .unwrap();
        let mut player = player2();
        let mut out = Pose::zero(2);
        for _ in 0..10 {
            player
                .tick(&space, &bank, params(0, 0), false, &mut out)
                .unwrap();
        }
        let start = out.rotations[0];
        assert!(angle_deg(start, ID) < 1.0, "primed {start:?}");
        player
            .tick(&space, &bank, params(2048, 0), false, &mut out)
            .unwrap();
        let first = angle_deg(start, out.rotations[0]);
        assert!(
            first <= 19.0,
            "first step pops {first} deg, want <= 19 (18 + eps)"
        );
        for _ in 0..9 {
            player
                .tick(&space, &bank, params(2048, 0), false, &mut out)
                .unwrap();
        }
        let done = angle_deg(out.rotations[0], HALF_Y);
        assert!(done <= 1.0, "after 10 ticks off {done} deg");
    }

    #[test]
    fn replay_is_identical_bytes_twice() {
        let (space, bank) = (space_ab(), bank_ab());
        let script = [
            (0, 0, false),
            (1024, 0, false),
            (2048, 0, true),
            (2048, 450, false),
            (0, -900, true),
            (1024, 0, false),
        ];
        let run = |rounds: usize| {
            let mut player = player2();
            let mut out = Pose::zero(2);
            let mut hashes = Vec::with_capacity(script.len() * rounds);
            for _ in 0..rounds {
                for (speed_q, angle_q, interrupt) in script {
                    player
                        .tick(&space, &bank, params(speed_q, angle_q), interrupt, &mut out)
                        .unwrap();
                    hashes.push(pose_bits(&out));
                }
            }
            hashes
        };
        let first = run(4);
        let second = run(4);
        assert_eq!(first, second);
        // A fresh twin replays the same bytes tick for tick.
        assert_eq!(run(1), first[..script.len()]);
    }

    #[test]
    fn failures_name_shape_and_bank() {
        let (space, bank) = (space_ab(), bank_ab());
        let mut out = Pose::zero(2);
        assert_eq!(
            SpacePlayer::new().tick(&space, &bank, params(0, 0), false, &mut out),
            Err(GraphError::NotPrepared)
        );
        let mut player = player2();
        let mut wide = Pose::zero(3);
        assert_eq!(
            player.tick(&space, &bank, params(0, 0), false, &mut wide),
            Err(GraphError::LengthMismatch {
                expected: 2,
                got: 3
            })
        );
        let trio = bake(vec![
            SpaceSample::new(0, 0, 10).unwrap(),
            SpaceSample::new(1024, 0, 10).unwrap(),
            SpaceSample::new(2048, 0, 11).unwrap(),
        ])
        .unwrap();
        assert_eq!(
            player.tick(&trio, &bank, params(0, 0), false, &mut out),
            Err(GraphError::LengthMismatch {
                expected: 2,
                got: 3
            })
        );
        let missing = bake(vec![SpaceSample::new(0, 0, 999).unwrap()]).unwrap();
        let mut solo = SpacePlayer::new();
        solo.prepare(2, 1);
        assert_eq!(
            solo.tick(&missing, &bank, params(0, 0), false, &mut out),
            Err(GraphError::UnknownClip {
                node: String::from("space[0]"),
                clip: 999
            })
        );
        let mut ragged = ClipBank::new();
        ragged
            .insert(
                10,
                Pose {
                    positions: vec![[0.0; 3]; 3],
                    rotations: vec![IDENTITY; 3],
                },
            )
            .unwrap();
        assert_eq!(
            player.tick(&space, &ragged, params(0, 0), false, &mut out),
            Err(GraphError::LengthMismatch {
                expected: 2,
                got: 3
            })
        );
    }

    #[test]
    fn prepare_reuses_scratch() {
        let (space, bank) = (space_ab(), bank_ab());
        let mut player = player2();
        let mut out = Pose::zero(2);
        let caps = (
            player.weights.capacity(),
            player.acc.capacity(),
            player.target.capacity(),
            player.off.capacity(),
            player.prev.capacity(),
        );
        for tick in 0..30 {
            let interrupt = tick == 15;
            player
                .tick(&space, &bank, params(1024, 0), interrupt, &mut out)
                .unwrap();
        }
        // Thirty ticks, one with an interrupt: no buffer grew or shrank.
        assert_eq!(
            (
                player.weights.capacity(),
                player.acc.capacity(),
                player.target.capacity(),
                player.off.capacity(),
                player.prev.capacity(),
            ),
            caps
        );
        assert_eq!(player.weights.len(), 2);
        player.prepare(2, 2);
        assert_eq!(
            (
                player.weights.capacity(),
                player.acc.capacity(),
                player.target.capacity(),
                player.off.capacity(),
                player.prev.capacity(),
            ),
            caps
        );
        // A new shape resizes the scratch and resets the offsets.
        player.prepare(3, 5);
        assert_eq!((player.joints(), player.sample_count()), (3, 5));
        assert!(player.settled());
    }
}
