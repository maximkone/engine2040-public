//! motion_match: motion matching v2.
//!
//! Consumer: arena locomotion picks a clip per tick and the traversal
//! chooser ranks approach clips; both read [`MatchResult`] plus the per-tick
//! [`SearchReceipt`] cost. Data owner: `engine-anim`; this module never
//! imports assets or arena. Units: meters, seconds, ticks at 60 Hz.
//!
//! Determinism: the authority path (quantize, cost, search, prediction,
//! blend decay) is integer-only with stable index order, so the same DB
//! plus the same query picks the same index twice. Errors are [`MatchError`].

use super::contract::LocalPose;
use engine_core::Fnv64;
use engine_nn::Model;

/// Future trajectory horizons in seconds.
pub const TRAJ_N: usize = 3;
/// Prediction horizons: 0.25 s, 0.5 s, 1.0 s ahead of the current root.
pub const HORIZONS_S: [f32; TRAJ_N] = [0.25, 0.5, 1.0];
/// Pose lanes: hip planar velocity (2) plus root-relative foot offsets (6).
pub const POSE_LANES: usize = 8;
/// Trajectory lanes: [`TRAJ_N`] root-relative future points (x, z).
pub const TRAJ_LANES: usize = TRAJ_N * 2;
/// Full feature width: pose lanes followed by trajectory lanes.
pub const FEAT_N: usize = POSE_LANES + TRAJ_LANES;
/// Fixed steps per meter (and per m/s): Q10.
pub const Q: i32 = 1024;
/// Quantize clamp: +-8192 m is far past any arena, keeps squares in `i64`.
pub const Q_MAX: i32 = 8_388_608;
/// Largest accepted per-lane weight; keeps lane products in `i64`.
pub const MAX_WEIGHT: i32 = 4096;
/// Largest accepted penalty; far below `i64::MAX` so the first entry seeds.
pub const PENALTY_MAX: i64 = i64::MAX / 8;
/// Largest baked database; bounds per-tick scan memory.
pub const MAX_ENTRIES: usize = 4096;
/// No-history marker for [`Query::curr`].
pub const NO_HIST: u32 = u32::MAX;
/// Trajectory-net input lanes: quantized vel (2) plus facing (2).
pub const TRAJ_NET_IN_DIM: usize = 4;
/// Trajectory-net output lanes: [`TRAJ_N`] planar offsets.
pub const TRAJ_NET_OUT_DIM: usize = TRAJ_N * 2;
/// Meters per trajectory-net output step; a power of two, so decode is exact.
pub const TRAJ_NET_STEP: f32 = 1.0 / 64.0;
/// Full-scale planar speed (m/s) for the velocity encoder lanes.
pub const TRAJ_FULL_SPEED: f32 = 12.0;
/// Blend decay divisor: offsets shrink by `1/8` per tick, minimum one step.
pub const DECAY_DIV: i32 = 8;

/// Motion-match failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatchError {
    /// The database holds no entries.
    EmptyDatabase,
    /// A query input is out of bounds; the lane names the problem.
    BadQuery(&'static str),
    /// A baked entry is out of bounds; the lane names the problem.
    BadDatabase(&'static str),
    /// A trajectory net has the wrong input/output dims; expected + got.
    BadNetDims {
        /// Required dim.
        expected: usize,
        /// The net's declared dim.
        got: usize,
    },
    /// The trajectory-net bytes are malformed; the net loader's cause is carried.
    Net(engine_nn::NnErr),
}

impl core::fmt::Display for MatchError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MatchError::EmptyDatabase => write!(f, "motion match: empty database"),
            MatchError::BadQuery(lane) => write!(f, "motion match: bad query ({lane})"),
            MatchError::BadDatabase(lane) => write!(f, "motion match: bad database ({lane})"),
            MatchError::BadNetDims { expected, got } => {
                write!(f, "motion match: net dim {got} differs from {expected}")
            }
            MatchError::Net(cause) => write!(f, "motion match: {cause}"),
        }
    }
}

impl std::error::Error for MatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MatchError::Net(cause) => Some(cause),
            _ => None,
        }
    }
}

impl From<engine_nn::NnErr> for MatchError {
    fn from(cause: engine_nn::NnErr) -> Self {
        MatchError::Net(cause)
    }
}

/// Current-body pose sample feeding the feature schema: planar hip velocity
/// in m/s, root-relative foot offsets in meters, planted-feet contact mask
/// (`bit0` left, `bit1` right).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PoseSample {
    /// Hip planar velocity `[x, z]` in m/s.
    pub hip_vel: [f32; 2],
    /// Left foot offset `[x, y, z]` in meters, root-relative.
    pub foot_l: [f32; 3],
    /// Right foot offset `[x, y, z]` in meters, root-relative.
    pub foot_r: [f32; 3],
    /// Contact mask: `bit0` left planted, `bit1` right planted.
    pub contact: u8,
}

/// Predicted future root path: [`TRAJ_N`] root-relative points in meters at
/// [`HORIZONS_S`]. Built by [`predict_linear`] or [`TrajNet::predict`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Trajectory {
    /// Future points `[x, z]` in meters, root-relative, horizon order.
    pub points: [[f32; 2]; TRAJ_N],
}

/// Sanitize then fix: non-finite maps to zero, range clamps to [`Q_MAX`].
pub(crate) fn q(value: f32) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    ((value * Q as f32) as i32).clamp(-Q_MAX, Q_MAX)
}

/// Build the fixed-point feature: pose lanes then trajectory lanes.
/// Rejects non-finite lanes and contact masks past `0b11`.
pub fn make_feature(pose: &PoseSample, traj: &Trajectory) -> Result<[i32; FEAT_N], MatchError> {
    if pose.hip_vel.iter().any(|v| !v.is_finite())
        || pose.foot_l.iter().any(|v| !v.is_finite())
        || pose.foot_r.iter().any(|v| !v.is_finite())
    {
        return Err(MatchError::BadQuery("pose"));
    }
    if traj.points.iter().flatten().any(|v| !v.is_finite()) {
        return Err(MatchError::BadQuery("trajectory"));
    }
    if pose.contact > 3 {
        return Err(MatchError::BadQuery("contact"));
    }
    let mut feat = [0i32; FEAT_N];
    feat[0] = q(pose.hip_vel[0]);
    feat[1] = q(pose.hip_vel[1]);
    for (lane, v) in pose.foot_l.iter().enumerate() {
        feat[2 + lane] = q(*v);
    }
    for (lane, v) in pose.foot_r.iter().enumerate() {
        feat[5 + lane] = q(*v);
    }
    for (k, point) in traj.points.iter().enumerate() {
        feat[POSE_LANES + k * 2] = q(point[0]);
        feat[POSE_LANES + k * 2 + 1] = q(point[1]);
    }
    Ok(feat)
}

/// Default weights: pose `1.0`, trajectory `2.0` in integer units.
pub fn default_weights() -> [i32; FEAT_N] {
    let mut w = [1; FEAT_N];
    for x in w[POSE_LANES..].iter_mut() {
        *x = 2;
    }
    w
}

/// Convert square meters to integer cost units for jump/contact penalties.
pub fn cost_units(square_meters: f32) -> i64 {
    if !square_meters.is_finite() || square_meters <= 0.0 {
        return 0;
    }
    let units = square_meters * Q as f32 * Q as f32;
    if units >= (i64::MAX / 8) as f32 {
        PENALTY_MAX
    } else {
        units as i64
    }
}

/// One baked motion frame: fixed feature, source clip/tick, contact mask.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    /// Fixed feature: pose lanes then trajectory lanes.
    pub feat: [i32; FEAT_N],
    /// Source clip id.
    pub clip: u16,
    /// Tick inside the clip.
    pub tick: u16,
    /// Contact mask: `bit0` left, `bit1` right.
    pub contact: u8,
}

/// Baked database in canonical `(clip, tick)` order with one ascending
/// contact bucket per mask, so the accelerated search skips nothing blindly.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct MotionDb {
    /// Entries in canonical order.
    pub entries: Vec<Entry>,
    /// Per-mask ascending entry indices.
    buckets: [Vec<u32>; 4],
}

/// Bake entries into canonical order; rejects oversize DBs and bad masks.
pub fn bake(mut entries: Vec<Entry>) -> Result<MotionDb, MatchError> {
    if entries.len() > MAX_ENTRIES {
        return Err(MatchError::BadDatabase("too many entries"));
    }
    if entries.iter().any(|e| e.contact > 3) {
        return Err(MatchError::BadDatabase("contact"));
    }
    entries.sort_by_key(|e| (e.clip, e.tick));
    let mut buckets: [Vec<u32>; 4] = Default::default();
    for (index, entry) in entries.iter().enumerate() {
        buckets[usize::from(entry.contact)].push(index as u32);
    }
    Ok(MotionDb { entries, buckets })
}

/// One search query: fixed feature, contact intent, continuity cursor,
/// per-lane weights and integer penalties. Penalties share the squared
/// fixed-point cost units of [`cost_units`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Query {
    /// Desired fixed feature.
    pub feat: [i32; FEAT_N],
    /// Desired contact mask.
    pub contact: u8,
    /// Current DB index, or [`NO_HIST`] with no continuity bias.
    pub curr: u32,
    /// Per-lane weights in `[0, MAX_WEIGHT]`.
    pub weights: [i32; FEAT_N],
    /// Penalty for non-sequential jumps.
    pub jump_cost: i64,
    /// Penalty for contact-mask mismatch (anti-slide gate).
    pub contact_cost: i64,
}

/// Build a query from a pose sample plus its predicted trajectory.
pub fn build_query(
    pose: &PoseSample,
    traj: &Trajectory,
    curr: u32,
    weights: &[i32; FEAT_N],
    jump_cost: i64,
    contact_cost: i64,
) -> Result<Query, MatchError> {
    let feat = make_feature(pose, traj)?;
    if weights.iter().any(|w| *w < 0 || *w > MAX_WEIGHT) {
        return Err(MatchError::BadQuery("weights"));
    }
    if jump_cost < 0 || contact_cost < 0 {
        return Err(MatchError::BadQuery("penalty"));
    }
    if jump_cost > PENALTY_MAX || contact_cost > PENALTY_MAX {
        return Err(MatchError::BadQuery("penalty range"));
    }
    Ok(Query {
        feat,
        contact: pose.contact,
        curr,
        weights: *weights,
        jump_cost,
        contact_cost,
    })
}

/// Validate a hand-built query (same bounds [`build_query`] enforces).
fn validate_query(query: &Query) -> Result<(), MatchError> {
    if query.contact > 3 {
        return Err(MatchError::BadQuery("contact"));
    }
    if query.weights.iter().any(|w| *w < 0 || *w > MAX_WEIGHT) {
        return Err(MatchError::BadQuery("weights"));
    }
    if query.jump_cost < 0 || query.contact_cost < 0 {
        return Err(MatchError::BadQuery("penalty"));
    }
    if query.jump_cost > PENALTY_MAX || query.contact_cost > PENALTY_MAX {
        return Err(MatchError::BadQuery("penalty range"));
    }
    Ok(())
}

/// `0` when `to` continues `from` in the same clip, else the jump penalty.
/// History-free or out-of-range cursors cost nothing.
pub(crate) fn jump_add(db: &MotionDb, from: u32, to: usize, penalty: i64) -> i64 {
    if from == NO_HIST {
        return 0;
    }
    let prev = from as usize;
    if prev >= db.entries.len() || to >= db.entries.len() {
        return 0;
    }
    if to == prev + 1 && db.entries[to].clip == db.entries[prev].clip {
        0
    } else {
        penalty
    }
}

/// Winning pick: DB index plus its source frame and exact integer cost.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MatchResult {
    /// Winning DB index (lowest on ties).
    pub index: usize,
    /// Source clip id.
    pub clip: u16,
    /// Tick inside the clip.
    pub tick: u16,
    /// Exact integer cost of the winner.
    pub cost: i64,
}

/// Per-search receipt: what the accelerated scan evaluated, skipped and
/// spent. `cost_per_character` is integer lane ops for this one-character
/// query; crowd ticks sum one receipt per character.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SearchReceipt {
    /// Entries in the DB.
    pub total: usize,
    /// Entries costed over every lane.
    pub full_evals: usize,
    /// Entries cut by the running-best bound (partial or penalty bound).
    pub early_cuts: usize,
    /// Preferred-bucket entries reused from the seeding pass, not rescanned.
    pub reuse_skips: usize,
    /// Integer lane multiply-accumulates actually executed.
    pub lane_ops: u64,
}

impl SearchReceipt {
    /// Search cost for this character: integer lane ops in the authority
    /// path. Deterministic: the same DB plus the same query spends the
    /// same ops twice.
    pub fn cost_per_character(&self) -> u64 {
        self.lane_ops
    }
}

/// Capped entry cost: `None` once the partial sum reaches `best`, which is
/// safe because only a strictly lower cost replaces the winner. Counts each
/// executed lane op in the receipt.
pub(crate) fn cost_capped(
    db: &MotionDb,
    entry: &Entry,
    to: usize,
    query: &Query,
    best: i64,
    receipt: &mut SearchReceipt,
) -> Option<i64> {
    let mut sum = query
        .contact_cost
        .saturating_mul(i64::from(entry.contact != query.contact))
        .saturating_add(jump_add(db, query.curr, to, query.jump_cost));
    if sum >= best {
        return None;
    }
    for lane in 0..FEAT_N {
        let diff = i64::from(entry.feat[lane]) - i64::from(query.feat[lane]);
        sum = sum.saturating_add(
            diff.saturating_mul(diff)
                .saturating_mul(i64::from(query.weights[lane])),
        );
        receipt.lane_ops += 1;
        if sum >= best {
            return None;
        }
    }
    Some(sum)
}

/// Deterministic accelerated search: seed the running best from the
/// contact-matching bucket in ascending order, then scan every entry in
/// ascending index order with the running-best bound. Strict `<`
/// replacement keeps the lowest index on ties, so the pick equals a full
/// brute-force scan while the receipt records the skipped work.
pub fn search(db: &MotionDb, query: &Query) -> Result<(MatchResult, SearchReceipt), MatchError> {
    search_penalized(db, query, |_| 0)
}

/// [`search`] with `penalty` added to the cost of every entry (style and
/// terrain dials, see `motion_style`). Same scan order, same bound and the
/// same ties: a penalty of zero everywhere is `search` exactly. Penalties are
/// non-negative and at most [`PENALTY_MAX`].
pub(crate) fn search_penalized(
    db: &MotionDb,
    query: &Query,
    penalty: impl Fn(&Entry) -> i64,
) -> Result<(MatchResult, SearchReceipt), MatchError> {
    if db.entries.is_empty() {
        return Err(MatchError::EmptyDatabase);
    }
    validate_query(query)?;
    let mut receipt = SearchReceipt {
        total: db.entries.len(),
        ..SearchReceipt::default()
    };
    let mut best = i64::MAX;
    let mut best_index = 0usize;
    let mut consider = |to: usize, entry: &Entry, receipt: &mut SearchReceipt| {
        let extra = penalty(entry);
        let capped = if extra >= best {
            None
        } else {
            cost_capped(db, entry, to, query, best - extra, receipt)
        };
        match capped {
            Some(cost) => {
                if cost + extra < best {
                    best = cost + extra;
                    best_index = to;
                }
                receipt.full_evals += 1;
            }
            None => receipt.early_cuts += 1,
        }
    };
    for &packed in &db.buckets[usize::from(query.contact)] {
        let to = packed as usize;
        consider(to, &db.entries[to], &mut receipt);
    }
    for (to, entry) in db.entries.iter().enumerate() {
        if entry.contact == query.contact {
            receipt.reuse_skips += 1;
            continue;
        }
        consider(to, entry, &mut receipt);
    }
    let winner = &db.entries[best_index];
    Ok((
        MatchResult {
            index: best_index,
            clip: winner.clip,
            tick: winner.tick,
            cost: best,
        },
        receipt,
    ))
}

/// FNV-1a64 over entries (clip, tick, contact, fixed lanes).
pub fn db_hash(db: &MotionDb) -> u64 {
    let mut h = Fnv64::new();
    h.write_len(db.entries.len());
    for e in &db.entries {
        h.write_u16(e.clip);
        h.write_u16(e.tick);
        h.write_u8(e.contact);
        for lane in e.feat.iter() {
            h.write_i32(*lane);
        }
    }
    h.finish()
}

/// Linear trajectory rollout: `points[k] = pos + vel * HORIZONS_S[k]` with
/// `+ *` only. Rejects non-finite inputs; the learned [`TrajNet`] path
/// refines this fallback, never the replay.
pub fn predict_linear(pos: [f32; 2], vel: [f32; 2]) -> Result<Trajectory, MatchError> {
    if pos.iter().any(|v| !v.is_finite()) || vel.iter().any(|v| !v.is_finite()) {
        return Err(MatchError::BadQuery("trajectory input"));
    }
    let mut points = [[0.0f32; 2]; TRAJ_N];
    for (k, h) in HORIZONS_S.iter().enumerate() {
        points[k] = [pos[0] + vel[0] * h, pos[1] + vel[1] * h];
    }
    Ok(Trajectory { points })
}

/// Symmetric quantization to `i8`: `value / full` clamped to `[-1, 1]`
/// times 127. Non-finite values and non-positive scales map to `0`.
pub(crate) fn quant_symmetric(value: f32, full: f32) -> i8 {
    if !value.is_finite() || !full.is_finite() || full <= 0.0 {
        return 0;
    }
    ((value / full).clamp(-1.0, 1.0) * 127.0) as i8
}

/// Learned trajectory predictor over the integer net runtime: a validated
/// `.e2nn` net with [`TRAJ_NET_IN_DIM`] inputs and [`TRAJ_NET_OUT_DIM`]
/// outputs. Integer inference, exact power-of-two decode, total on short
/// or long feature slices (the runtime pads/truncates).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TrajNet {
    model: Model,
}

impl TrajNet {
    /// Load and validate trajectory-net bytes: the runtime parses, then dims are
    /// checked against [`TRAJ_NET_IN_DIM`] and [`TRAJ_NET_OUT_DIM`].
    pub fn load(bytes: &[u8]) -> Result<Self, MatchError> {
        let model = engine_nn::load(bytes)?;
        if model.input_dim() != TRAJ_NET_IN_DIM {
            return Err(MatchError::BadNetDims {
                expected: TRAJ_NET_IN_DIM,
                got: model.input_dim(),
            });
        }
        if model.output_dim() != TRAJ_NET_OUT_DIM {
            return Err(MatchError::BadNetDims {
                expected: TRAJ_NET_OUT_DIM,
                got: model.output_dim(),
            });
        }
        Ok(Self { model })
    }

    /// Predict the future root path: quantize velocity plus facing into four
    /// `i8` lanes, run the integer net, decode each output lane as a
    /// [`TRAJ_NET_STEP`]-meter offset from `pos`. Rejects non-finite input.
    pub fn predict(
        &self,
        pos: [f32; 2],
        vel: [f32; 2],
        facing: [f32; 2],
    ) -> Result<Trajectory, MatchError> {
        if pos.iter().any(|v| !v.is_finite())
            || vel.iter().any(|v| !v.is_finite())
            || facing.iter().any(|v| !v.is_finite())
        {
            return Err(MatchError::BadQuery("trajectory input"));
        }
        let features = [
            quant_symmetric(vel[0], TRAJ_FULL_SPEED),
            quant_symmetric(vel[1], TRAJ_FULL_SPEED),
            quant_symmetric(facing[0], 1.0),
            quant_symmetric(facing[1], 1.0),
        ];
        let raw = engine_nn::infer(&self.model, &features);
        let mut points = [[0.0f32; 2]; TRAJ_N];
        for (k, slot) in points.iter_mut().enumerate() {
            let dx = raw.get(k * 2).copied().unwrap_or(0).clamp(-128, 127) as f32;
            let dz = raw.get(k * 2 + 1).copied().unwrap_or(0).clamp(-128, 127) as f32;
            *slot = [pos[0] + dx * TRAJ_NET_STEP, pos[1] + dz * TRAJ_NET_STEP];
        }
        Ok(Trajectory { points })
    }
}

/// Inertialized transition state: fixed-point joint offsets (Q10, meters)
/// decaying by `1/8` per tick with a guaranteed one-step minimum, so every
/// switch settles exactly. Rotations cut to the target; positions carry the
/// decay. Capture with [`InertialBlend::capture_pose`], advance one tick
/// per [`InertialBlend::step_pose`].
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct InertialBlend {
    /// Per-joint position offsets in Q10 meters.
    off: Vec<[i32; 3]>,
}

impl InertialBlend {
    /// New zero state for `n` joints.
    pub fn new(n: usize) -> Self {
        Self {
            off: vec![[0; 3]; n],
        }
    }

    /// Joint count.
    pub fn len(&self) -> usize {
        self.off.len()
    }

    /// True when there are no joints.
    pub fn is_empty(&self) -> bool {
        self.off.is_empty()
    }

    /// Capture the offset between two poses; the first [`InertialBlend::step_pose`]
    /// output then equals `src` exactly, decaying toward later targets.
    pub fn capture_pose(&mut self, src: &LocalPose, dst: &LocalPose) {
        let shared = self.len().min(src.positions.len()).min(dst.positions.len());
        for (index, slot) in self.off.iter_mut().enumerate() {
            *slot = if index < shared {
                [
                    q(src.positions[index][0]).saturating_sub(q(dst.positions[index][0])),
                    q(src.positions[index][1]).saturating_sub(q(dst.positions[index][1])),
                    q(src.positions[index][2]).saturating_sub(q(dst.positions[index][2])),
                ]
            } else {
                [0; 3]
            };
        }
    }

    /// Advance one tick toward `target` into the caller-owned `out`:
    /// positions carry the integer decay, rotations cut to the target. Same
    /// target twice gives the same pose; `out` keeps its capacity, so a pose
    /// that already fits allocates nothing.
    pub fn step_pose(&mut self, target: &LocalPose, out: &mut LocalPose) {
        out.rotations.clear();
        out.rotations.extend_from_slice(&target.rotations);
        out.positions.clear();
        for (index, p) in target.positions.iter().enumerate() {
            let mut lanes = [q(p[0]), q(p[1]), q(p[2])];
            if let Some(offset) = self.off.get_mut(index) {
                for (lane, slot) in lanes.iter_mut().zip(offset.iter_mut()) {
                    *lane = step_lane(slot, *lane);
                }
            }
            out.positions.push([
                lanes[0] as f32 / Q as f32,
                lanes[1] as f32 / Q as f32,
                lanes[2] as f32 / Q as f32,
            ]);
        }
    }

    /// True when every offset lane reached zero.
    pub fn settled(&self) -> bool {
        self.off.iter().flatten().all(|v| *v == 0)
    }

    /// Largest absolute offset lane in Q10 steps (settle meter).
    pub fn max_offset_q(&self) -> i32 {
        max_lane_abs(&self.off)
    }
}

/// One lane of the inertial step: the output is `target + offset`, then the
/// offset decays by `1/DECAY_DIV` with a one-step minimum so it always
/// reaches zero.
fn step_lane(offset: &mut i32, target: i32) -> i32 {
    let out = target.saturating_add(*offset);
    let decay = *offset / DECAY_DIV;
    let step = if decay == 0 { offset.signum() } else { decay };
    *offset = offset.saturating_sub(step);
    out
}

/// Largest absolute lane in Q10 steps over a joint-offset table; empty reads 0.
pub(crate) fn max_lane_abs(offsets: &[[i32; 3]]) -> i32 {
    offsets
        .iter()
        .flatten()
        .map(|v| v.saturating_abs())
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::super::contract::{validate_pose, Joint, Skeleton};
    use super::super::locomotion::{loco_skeleton, Locomotion};
    use super::*;
    use engine_core::alloc_probe::measure;

    fn pose(contact: u8, hip: f32, foot: f32) -> PoseSample {
        PoseSample {
            hip_vel: [hip, 0.0],
            foot_l: [foot, 0.05, 0.0],
            foot_r: [-foot, 0.05, 0.0],
            contact,
        }
    }

    /// Trajectory fixture matching [`predict_linear`] from the origin at the
    /// given hip speed, so queries and entries share prediction semantics.
    fn traj(hip: f32) -> Trajectory {
        predict_linear([0.0, 0.0], [hip, 0.0]).expect("fixture velocity finite")
    }

    fn entry(clip: u16, tick: u16, hip: f32, foot: f32, contact: u8) -> Entry {
        Entry {
            feat: make_feature(&pose(contact, hip, foot), &traj(hip)).expect("fixture valid"),
            clip,
            tick,
            contact,
        }
    }

    fn query_for(hip: f32, foot: f32, contact: u8) -> Query {
        build_query(
            &pose(contact, hip, foot),
            &traj(hip),
            NO_HIST,
            &default_weights(),
            cost_units(0.01),
            cost_units(0.04),
        )
        .expect("fixture query valid")
    }

    #[test]
    fn bake_sorts_and_hashes_stably() {
        let unsorted = vec![
            entry(1, 2, 1.0, 0.1, 3),
            entry(0, 9, 2.0, 0.2, 1),
            entry(0, 1, 0.5, 0.0, 0),
        ];
        let db = bake(unsorted).expect("bake valid");
        assert_eq!(
            db.entries
                .iter()
                .map(|e| (e.clip, e.tick))
                .collect::<Vec<_>>(),
            vec![(0, 1), (0, 9), (1, 2)]
        );
        let reordered = bake(vec![
            entry(0, 1, 0.5, 0.0, 0),
            entry(1, 2, 1.0, 0.1, 3),
            entry(0, 9, 2.0, 0.2, 1),
        ])
        .expect("bake valid");
        assert_eq!(db_hash(&db), db_hash(&reordered));
    }

    #[test]
    fn search_picks_exact_and_repeats() {
        let db = bake(vec![
            entry(0, 0, 0.0, 0.0, 3),
            entry(0, 1, 1.5, 0.1, 1),
            entry(0, 2, 3.0, 0.2, 2),
        ])
        .expect("bake valid");
        let query = query_for(1.5, 0.1, 1);
        let first = search(&db, &query).expect("search hits");
        let second = search(&db, &query).expect("search hits");
        assert_eq!(first, second);
        assert_eq!(first.0.index, 1);
        assert_eq!((first.0.clip, first.0.tick), (0, 1));
        assert_eq!(first.0.cost, 0);
        assert_eq!(first.1.cost_per_character(), first.1.lane_ops);
        // Same query twice spends the same ops.
        assert_eq!(first.1.lane_ops, second.1.lane_ops);
    }

    /// Test-only brute force without bounds: the accelerated pick must equal it.
    fn brute(db: &MotionDb, query: &Query) -> (usize, i64) {
        let mut best = i64::MAX;
        let mut at = 0usize;
        for (to, e) in db.entries.iter().enumerate() {
            let mut sum = query
                .contact_cost
                .saturating_mul(i64::from(e.contact != query.contact))
                .saturating_add(jump_add(db, query.curr, to, query.jump_cost));
            for lane in 0..FEAT_N {
                let diff = e.feat[lane] as i64 - query.feat[lane] as i64;
                sum = sum.saturating_add(
                    diff.saturating_mul(diff)
                        .saturating_mul(query.weights[lane] as i64),
                );
            }
            if sum < best {
                best = sum;
                at = to;
            }
        }
        (at, best)
    }

    #[test]
    fn search_matches_brute_force_on_noise() {
        let mut state: u64 = 0x1234_5678_9abc_def1;
        let mut next = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            state
        };
        let lane = |next: &mut dyn FnMut() -> u64, span: f32| {
            ((next() % 2000) as f32 / 1000.0 - 1.0) * span
        };
        let mut entries = Vec::with_capacity(256);
        for i in 0..256 {
            let hip = lane(&mut next, 3.0);
            let foot = lane(&mut next, 0.4);
            let contact = (next() % 4) as u8;
            entries.push(entry((i / 32) as u16, (i % 32) as u16, hip, foot, contact));
        }
        let db = bake(entries).expect("bake valid");
        for round in 0..25 {
            let hip = lane(&mut next, 3.0);
            let foot = lane(&mut next, 0.4);
            let contact = (next() % 4) as u8;
            let query = query_for(hip, foot, contact);
            let (got, receipt) = search(&db, &query).expect("search hits");
            let (want_at, want_cost) = brute(&db, &query);
            assert_eq!((got.index, got.cost), (want_at, want_cost), "round {round}");
            // Receipt accounts for every entry: each entry is costed or cut
            // exactly once across both passes, and preferred-bucket entries
            // are reused (not rescanned) in the full scan.
            assert_eq!(receipt.full_evals + receipt.early_cuts, receipt.total);
            assert_eq!(
                receipt.reuse_skips,
                db.entries.iter().filter(|e| e.contact == contact).count()
            );
            // Identical picks twice on the same query.
            assert_eq!(search(&db, &query).expect("repeat").0, got);
        }
    }

    #[test]
    fn acceleration_skips_bounded_work() {
        // Exact match on entry 0 with heavy penalties: every mismatched
        // entry dies on the penalty bound with zero lane ops.
        let mut entries = vec![entry(0, 0, 1.0, 0.1, 3)];
        for i in 1..64 {
            entries.push(entry(0, i, 1.0 + i as f32, 0.3, 0));
        }
        let db = bake(entries).expect("bake valid");
        let query = build_query(
            &pose(3, 1.0, 0.1),
            &traj(1.0),
            NO_HIST,
            &default_weights(),
            cost_units(1.0),
            cost_units(1.0),
        )
        .expect("query valid");
        let (got, receipt) = search(&db, &query).expect("search hits");
        assert_eq!(got.index, 0);
        assert!(receipt.early_cuts > 0, "bound must cut: {receipt:?}");
        assert!(
            receipt.lane_ops < (receipt.total as u64) * (FEAT_N as u64),
            "accelerated ops {} below brute {}",
            receipt.lane_ops,
            receipt.total * FEAT_N
        );
    }

    #[test]
    fn ties_keep_lowest_index_and_contact_gates() {
        let db = bake(vec![
            entry(0, 0, 1.0, 0.1, 0),
            entry(0, 1, 1.0, 0.1, 0),
            entry(0, 2, 1.0, 0.1, 3),
        ])
        .expect("bake valid");
        let (got, _) = search(&db, &query_for(1.0, 0.1, 0)).expect("search hits");
        assert_eq!(got.index, 0, "ties keep the lowest index");
        // A heavy contact penalty holds the planted entry even when the
        // trajectory matches a lifted one exactly.
        let lifted = build_query(
            &pose(0, 1.0, 0.1),
            &traj(1.0),
            NO_HIST,
            &default_weights(),
            0,
            cost_units(10.0),
        )
        .expect("query valid");
        let (held, _) = search(&db, &lifted).expect("search hits");
        assert_eq!(held.index, 0);
    }

    #[test]
    fn continuity_bias_holds_the_sequence() {
        let db = bake(vec![
            entry(0, 0, 1.0, 0.1, 3),
            entry(0, 1, 1.0, 0.1, 3),
            entry(1, 0, 1.0, 0.1, 3),
        ])
        .expect("bake valid");
        // Entry 2 ties on features; without history the lowest index wins,
        // with history on entry 0 the sequential entry 1 wins over the jump.
        let plain = query_for(1.0, 0.1, 3);
        assert_eq!(search(&db, &plain).expect("search").0.index, 0);
        let biased = build_query(
            &pose(3, 1.0, 0.1),
            &traj(1.0),
            0,
            &default_weights(),
            cost_units(0.5),
            0,
        )
        .expect("query valid");
        assert_eq!(search(&db, &biased).expect("search").0.index, 1);
    }

    #[test]
    fn typed_errors_name_empty_db_and_bad_query() {
        let query = query_for(1.0, 0.1, 3);
        assert_eq!(
            search(&MotionDb::default(), &query),
            Err(MatchError::EmptyDatabase)
        );
        let mut weights = default_weights();
        weights[0] = -1;
        assert_eq!(
            build_query(&pose(3, 1.0, 0.1), &traj(1.0), NO_HIST, &weights, 0, 0),
            Err(MatchError::BadQuery("weights"))
        );
        weights[0] = MAX_WEIGHT + 1;
        assert_eq!(
            build_query(&pose(3, 1.0, 0.1), &traj(1.0), NO_HIST, &weights, 0, 0),
            Err(MatchError::BadQuery("weights"))
        );
        assert_eq!(
            build_query(
                &pose(3, 1.0, 0.1),
                &traj(1.0),
                NO_HIST,
                &default_weights(),
                -5,
                0
            ),
            Err(MatchError::BadQuery("penalty"))
        );
        assert_eq!(
            build_query(
                &pose(3, 1.0, 0.1),
                &traj(1.0),
                NO_HIST,
                &default_weights(),
                PENALTY_MAX + 1,
                0
            ),
            Err(MatchError::BadQuery("penalty range"))
        );
        assert_eq!(
            build_query(
                &pose(9, 1.0, 0.1),
                &traj(1.0),
                NO_HIST,
                &default_weights(),
                0,
                0
            ),
            Err(MatchError::BadQuery("contact"))
        );
        assert_eq!(
            build_query(
                &PoseSample {
                    hip_vel: [f32::NAN, 0.0],
                    foot_l: [0.0; 3],
                    foot_r: [0.0; 3],
                    contact: 3,
                },
                &traj(1.0),
                NO_HIST,
                &default_weights(),
                0,
                0
            ),
            Err(MatchError::BadQuery("pose"))
        );
        // Oversize DBs and bad masks are bake-time errors, never truncations.
        let big = (0..MAX_ENTRIES + 1)
            .map(|i| entry(0, i as u16, 0.0, 0.0, 0))
            .collect::<Vec<_>>();
        assert_eq!(bake(big), Err(MatchError::BadDatabase("too many entries")));
        let mut bad = entry(0, 0, 0.0, 0.0, 0);
        bad.contact = 7;
        assert_eq!(bake(vec![bad]), Err(MatchError::BadDatabase("contact")));
    }

    #[test]
    fn linear_prediction_golden_and_rejected() {
        let got = predict_linear([1.0, 2.0], [3.0, -1.0]).expect("valid input");
        assert_eq!(
            got,
            Trajectory {
                points: [[1.75, 1.75], [2.5, 1.5], [4.0, 1.0]],
            }
        );
        assert_eq!(
            got,
            predict_linear([1.0, 2.0], [3.0, -1.0]).expect("repeat")
        );
        assert_eq!(
            predict_linear([f32::INFINITY, 0.0], [0.0, 0.0]),
            Err(MatchError::BadQuery("trajectory input"))
        );
        assert_eq!(
            predict_linear([0.0, 0.0], [0.0, f32::NAN]),
            Err(MatchError::BadQuery("trajectory input"))
        );
    }

    fn w_u16(value: u16, out: &mut Vec<u8>) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn w_i32(value: i32, out: &mut Vec<u8>) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    /// Gather net: one dense layer, identity requant, no activation; row `k`
    /// carries a single `1` at column `k % in_dim`, so output `k` gathers
    /// `features[k % in_dim]`. Test fixture only, no shipped weights.
    fn gather_net_bytes(in_dim: u16, out_dim: u16) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&engine_nn::MAGIC);
        bytes.push(engine_nn::VERSION);
        bytes.push(1);
        w_u16(in_dim, &mut bytes);
        bytes.push(engine_nn::TAG_DENSE);
        w_u16(out_dim, &mut bytes);
        w_i32(1, &mut bytes);
        bytes.push(0);
        bytes.push(engine_nn::ACT_NONE);
        for _ in 0..out_dim {
            w_i32(0, &mut bytes);
        }
        for row in 0..out_dim as usize {
            for col in 0..in_dim as usize {
                bytes.push(u8::from(col == row % in_dim as usize));
            }
        }
        bytes
    }

    #[test]
    fn learned_prediction_golden_reload_and_reject() {
        let bytes = gather_net_bytes(TRAJ_NET_IN_DIM as u16, TRAJ_NET_OUT_DIM as u16);
        let net = TrajNet::load(&bytes).expect("fixture loads");
        // vel [12, 0] m/s hits full scale (127), facing +Z gives 127 on lane 3.
        let got = net
            .predict([1.0, 2.0], [12.0, 0.0], [0.0, 1.0])
            .expect("valid input");
        let step = TRAJ_NET_STEP;
        assert_eq!(
            got,
            Trajectory {
                points: [
                    [1.0 + 127.0 * step, 2.0],
                    [1.0, 2.0 + 127.0 * step],
                    [1.0 + 127.0 * step, 2.0],
                ],
            }
        );
        // Reloaded bytes predict identically; bad dims and bytes are typed out.
        assert_eq!(
            TrajNet::load(&bytes)
                .expect("reloads")
                .predict([1.0, 2.0], [12.0, 0.0], [0.0, 1.0])
                .expect("valid input"),
            got
        );
        assert_eq!(
            TrajNet::load(&gather_net_bytes(3, TRAJ_NET_OUT_DIM as u16)),
            Err(MatchError::BadNetDims {
                expected: TRAJ_NET_IN_DIM,
                got: 3
            })
        );
        assert_eq!(
            TrajNet::load(&gather_net_bytes(TRAJ_NET_IN_DIM as u16, 2)),
            Err(MatchError::BadNetDims {
                expected: TRAJ_NET_OUT_DIM,
                got: 2
            })
        );
        assert_eq!(
            TrajNet::load(&[]),
            Err(MatchError::Net(engine_nn::NnErr::Truncated))
        );
        assert_eq!(
            net.predict([f32::NAN, 0.0], [0.0, 0.0], [0.0, 1.0]),
            Err(MatchError::BadQuery("trajectory input"))
        );
    }

    fn skeleton() -> Skeleton {
        Skeleton {
            joints: vec![
                Joint {
                    name: "root".into(),
                    parent: None,
                },
                Joint {
                    name: "spine".into(),
                    parent: Some(0),
                },
            ],
        }
    }

    fn stance(height: f32) -> LocalPose {
        use super::super::contract::LocalPose;
        LocalPose {
            positions: vec![[0.0, height, 0.0], [0.0, 1.0, 0.0]],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; 2],
        }
    }

    /// Step into a fresh pose, for tests that compare whole poses.
    fn stepped(blend: &mut InertialBlend, target: &LocalPose) -> LocalPose {
        let mut out = LocalPose {
            positions: Vec::new(),
            rotations: Vec::new(),
        };
        blend.step_pose(target, &mut out);
        out
    }

    #[test]
    fn blend_starts_exact_decays_and_repeats() {
        let (src, dst) = (stance(1.0), stance(0.0));
        let mut first = InertialBlend::new(2);
        first.capture_pose(&src, &dst);
        assert_eq!(first.max_offset_q(), Q);
        // First output equals the source pose exactly.
        let out = stepped(&mut first, &dst);
        assert_eq!(out.positions[0], src.positions[0]);
        // A twin blend steps identically (deterministic decay).
        let mut second = InertialBlend::new(2);
        second.capture_pose(&src, &dst);
        assert_eq!(stepped(&mut second, &dst), out);
        // Sixty ticks settle a one-meter offset below a millimeter.
        for _ in 0..60 {
            stepped(&mut first, &dst);
        }
        assert!(first.settled(), "max={}", first.max_offset_q());
        assert!(first.max_offset_q() < 2, "max={}", first.max_offset_q());
        let landed = stepped(&mut first, &dst);
        assert_eq!(landed.positions[0], dst.positions[0]);
        validate_pose(&skeleton(), &landed).unwrap();
    }

    #[test]
    fn locomotion_consumer_path_end_to_end() {
        // Arena locomotion shapes the query: gait speed/facing plus the wish
        // drive the pose sample, the linear prediction feeds the trajectory
        // lanes, and the pick blends inertially toward the next pose.
        let mut machine = Locomotion::new();
        for t in 0..12 {
            machine.update(1.5, [0.0, 1.0], t, None).expect("walk wish");
        }
        let facing = machine.facing();
        let speed = machine.speed();
        let sample = PoseSample {
            hip_vel: [facing[0] * speed, facing[1] * speed],
            foot_l: [0.11, 0.05, 0.0],
            foot_r: [-0.11, 0.05, 0.0],
            contact: 3,
        };
        let traj = predict_linear([0.0, 0.0], sample.hip_vel).expect("finite wish");
        let db = bake(vec![
            entry(0, 0, 0.0, 0.0, 3),
            Entry {
                feat: make_feature(&sample, &traj).expect("sample valid"),
                clip: 0,
                tick: 1,
                contact: 3,
            },
            entry(1, 0, 4.0, 0.2, 1),
        ])
        .expect("bake valid");
        let query = build_query(
            &sample,
            &traj,
            NO_HIST,
            &default_weights(),
            cost_units(0.01),
            cost_units(0.04),
        )
        .expect("query valid");
        let (picked, receipt) = search(&db, &query).expect("search hits");
        assert_eq!((picked.clip, picked.tick), (0, 1));
        assert!(receipt.cost_per_character() > 0);
        // The traversal chooser reuses the same pick shape for
        // approach clips; the blend output is identical twice.
        let current = machine
            .update(1.5, [0.0, 1.0], 12, None)
            .expect("walk tick")
            .pose;
        let mut target = current.clone();
        target.positions[0][1] -= 0.02;
        let mut blend = InertialBlend::new(current.positions.len());
        blend.capture_pose(&current, &target);
        assert_eq!(stepped(&mut blend, &target), {
            let mut twin = InertialBlend::new(current.positions.len());
            twin.capture_pose(&current, &target);
            stepped(&mut twin, &target)
        });
        validate_pose(&loco_skeleton(), &stepped(&mut blend, &target)).unwrap();
    }

    #[test]
    fn step_pose_into_a_sized_pose_allocates_nothing() {
        let (src, dst) = (stance(1.0), stance(0.0));
        let mut blend = InertialBlend::new(2);
        blend.capture_pose(&src, &dst);
        let mut out = stepped(&mut blend, &dst);
        blend.capture_pose(&src, &dst);
        let ((), heap) = measure(|| {
            for _ in 0..60 {
                blend.step_pose(&dst, &mut out);
            }
        });
        assert_eq!(heap.calls, 0, "steady inertial steps must not allocate");
        assert!(blend.settled());
        assert_eq!(out.positions[0], dst.positions[0]);
    }

    #[test]
    fn short_offset_table_passes_extra_joints_through_quantized() {
        // Joints past the offset table are the quantized target, undecayed.
        let (src, dst) = (stance(1.0), stance(0.0));
        let mut blend = InertialBlend::new(1);
        blend.capture_pose(&src, &dst);
        let out = stepped(&mut blend, &dst);
        assert_eq!(out.positions[0], src.positions[0]);
        assert_eq!(out.positions[1], dst.positions[1]);
        assert_eq!(out.rotations, dst.rotations);
    }
}
