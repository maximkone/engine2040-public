//! motion_net: learned motion-matching pose path over integer nets.
//!
//! Consumer: future net-driven animation reads [`MotionNet`] poses
//! as presentation; baseline locomotion (`locomotion.rs`) stays net-free and
//! never imports this module. Data owner: `engine-anim` owns the validated
//! [`MotionNet`] handle; the caller owns the `.e2nn` bytes. Units: features
//! are unitless `i8` lanes; poses are meters plus unit quaternions on
//! [`loco_skeleton`](crate::locomotion::loco_skeleton), one net output step is
//! [`POS_SCALE`] meters.
//!
//! Determinism: integer-only net inference plus an exact power-of-two decode,
//! no transcendentals, no wall clock, stable joint order. Same bytes plus
//! same features give the same pose on every machine; short feature slices
//! zero-pad and long ones truncate (net totality), so inference never panics.
//! Invalid bytes or wrong dims return [`MotionNetError`]; a missing net falls
//! back to [`scripted_fallback_pose`] through [`infer_pose_or_fallback`].
//! No training here (the training tools own it); tests hand-build tiny `.e2nn`
//! fixtures from the documented byte layout, never shipped weights.

use super::contract::LocalPose;
use super::locomotion::{LocoState, Locomotion, LOCO_JOINT_COUNT, MAX_WISH_SPEED};
use super::motion_match::quant_symmetric;
use engine_nn::Model;

/// Quantized gait feature count: the only accepted net input dim.
pub const FEATURE_DIM: usize = 8;
/// Net output lanes decoded into a pose: three per [`LOCO_JOINT_COUNT`] joint.
pub const POSE_OUT_DIM: usize = LOCO_JOINT_COUNT * 3;
/// Meters per net output step; a power of two, so decode is bit-exact.
pub const POS_SCALE: f32 = 1.0 / 64.0;
/// Full-scale planar speed (m/s) for the speed feature lanes.
pub const FEATURE_FULL_SPEED: f32 = MAX_WISH_SPEED;

/// Feature lane order: gait phase in `[0, 1)`.
pub const LANE_PHASE: usize = 1;
/// Feature lane order: planar speed (m/s).
pub const LANE_SPEED: usize = 4;
/// Feature lane order: wish speed (m/s).
pub const LANE_WISH_SPEED: usize = 5;

/// Plain gait snapshot feeding [`features_from`]: locomotion state plus the
/// presentation scalars a learned net conditions on. Built by hand or by
/// [`GaitSnapshot::from_locomotion`]; never aliases machine internals.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GaitSnapshot {
    /// Current locomotion state (quantized to a small code).
    pub state: LocoState,
    /// Gait phase in `[0, 1)`.
    pub phase: f32,
    /// Facing as a unit `[x, z]` vector.
    pub facing: [f32; 2],
    /// Planar speed (m/s) applied on the previous tick.
    pub speed: f32,
}

impl GaitSnapshot {
    /// Snapshot the presentation scalars of a live machine. Reads public
    /// getters only; the machine keeps owning its authoritative state.
    pub fn from_locomotion(machine: &Locomotion) -> Self {
        Self {
            state: machine.state,
            phase: machine.phase(),
            facing: machine.facing(),
            speed: machine.speed(),
        }
    }
}

/// Plain planar wish feeding [`features_from`]: speed in m/s plus the
/// world-space `[x, z]` wish direction.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct WishInput {
    /// Wish speed in m/s.
    pub speed: f32,
    /// Wish direction `[x, z]` (clamped per lane, never normalized here).
    pub dir: [f32; 2],
}

/// Learned-net handle: a validated `.e2nn` model whose input dim is
/// [`FEATURE_DIM`] and whose output dim is [`POSE_OUT_DIM`]. Build with
/// [`MotionNet::load_e2nn`]; the dims check keeps the baseline feature/pose
/// sizes and the net in agreement, so [`MotionNet::infer_pose`] is total.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MotionNet {
    model: Model,
}

/// Bounded load failures: malformed bytes name the loader cause, wrong dims
/// name expected vs got. Never panics, never blocks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MotionNetError {
    /// Bytes are not a valid `.e2nn` file; the loader cause is carried.
    Nn(engine_nn::NnErr),
    /// Net input dim differs from [`FEATURE_DIM`]; expected + got carried.
    BadInputDim {
        /// Required input dim ([`FEATURE_DIM`]).
        expected: usize,
        /// Net's declared input dim.
        got: usize,
    },
    /// Net output dim differs from [`POSE_OUT_DIM`]; expected + got carried.
    BadOutputDim {
        /// Required output dim ([`POSE_OUT_DIM`]).
        expected: usize,
        /// Net's declared output dim.
        got: usize,
    },
}

impl core::fmt::Display for MotionNetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MotionNetError::Nn(cause) => write!(f, "motion net: {cause}"),
            MotionNetError::BadInputDim { expected, got } => write!(
                f,
                "motion net: input dim {got} differs from features {expected}"
            ),
            MotionNetError::BadOutputDim { expected, got } => write!(
                f,
                "motion net: output dim {got} differs from pose lanes {expected}"
            ),
        }
    }
}

impl std::error::Error for MotionNetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            MotionNetError::Nn(cause) => Some(cause),
            _ => None,
        }
    }
}

impl From<engine_nn::NnErr> for MotionNetError {
    fn from(cause: engine_nn::NnErr) -> Self {
        MotionNetError::Nn(cause)
    }
}

/// Small state code per [`LocoState`]: `Idle` 0, `Walk` 1, `Run` 2, `Strafe`
/// 3, `Start` 4, `Stop` 5, `Pivot` 6, `Air` 7, `Land` 8.
pub fn state_code(state: LocoState) -> i8 {
    match state {
        LocoState::Idle => 0,
        LocoState::Walk => 1,
        LocoState::Run => 2,
        LocoState::Strafe => 3,
        LocoState::Start => 4,
        LocoState::Stop => 5,
        LocoState::Pivot => 6,
        LocoState::Air => 7,
        LocoState::Land => 8,
    }
}

/// Quantize one gait snapshot plus one wish into [`FEATURE_DIM`] `i8` lanes
/// in lane order (`LANE_*`). Pure, total and bit-exact: same inputs twice
/// give the same lanes on every machine.
pub fn features_from(gait: &GaitSnapshot, wish: &WishInput) -> Vec<i8> {
    vec![
        state_code(gait.state),
        quant_symmetric(gait.phase, 1.0),
        quant_symmetric(gait.facing[0], 1.0),
        quant_symmetric(gait.facing[1], 1.0),
        quant_symmetric(gait.speed, FEATURE_FULL_SPEED),
        quant_symmetric(wish.speed, FEATURE_FULL_SPEED),
        quant_symmetric(wish.dir[0], 1.0),
        quant_symmetric(wish.dir[1], 1.0),
    ]
}

impl MotionNet {
    /// Load and validate an `.e2nn` net: the runtime parses the bytes, then
    /// the dims are checked against [`FEATURE_DIM`] and [`POSE_OUT_DIM`]. Same
    /// bytes twice give equal handles.
    pub fn load_e2nn(bytes: &[u8]) -> Result<Self, MotionNetError> {
        let model = engine_nn::load(bytes)?;
        if model.input_dim() != FEATURE_DIM {
            return Err(MotionNetError::BadInputDim {
                expected: FEATURE_DIM,
                got: model.input_dim(),
            });
        }
        if model.output_dim() != POSE_OUT_DIM {
            return Err(MotionNetError::BadOutputDim {
                expected: POSE_OUT_DIM,
                got: model.output_dim(),
            });
        }
        Ok(Self { model })
    }

    /// Accepted feature count (always [`FEATURE_DIM`]).
    pub fn input_dim(&self) -> usize {
        self.model.input_dim()
    }

    /// Decoded pose lane count (always [`POSE_OUT_DIM`]).
    pub fn output_dim(&self) -> usize {
        self.model.output_dim()
    }

    /// Run the net and decode a [`LocalPose`] on
    /// [`loco_skeleton`](crate::locomotion::loco_skeleton): each output lane
    /// is an `i8`-range step of [`POS_SCALE`] meters into joint positions in
    /// skeleton order, rotations stay identity. Total: short feature slices
    /// zero-pad, long ones truncate; the result always validates.
    pub fn infer_pose(&self, features: &[i8]) -> LocalPose {
        decode_pose(&engine_nn::infer(&self.model, features))
    }
}

/// Decode raw net outputs into a pose: lane `3 * joint + lane` sets
/// `positions[joint][lane]`, rotations stay identity. Out-of-range lanes
/// clamp to `i8` range first (belt-and-braces: the runtime already
/// saturates), so a corrupt-but-loaded model still yields a finite pose,
/// never a panic.
fn decode_pose(raw: &[i32]) -> LocalPose {
    let mut positions = Vec::with_capacity(LOCO_JOINT_COUNT);
    for joint in 0..LOCO_JOINT_COUNT {
        let mut point = [0.0f32; 3];
        for (lane, slot) in point.iter_mut().enumerate() {
            let step = raw.get(joint * 3 + lane).copied().unwrap_or(0);
            *slot = step.clamp(-128, 127) as f32 * POS_SCALE;
        }
        positions.push(point);
    }
    LocalPose {
        positions,
        rotations: vec![[0.0, 0.0, 0.0, 1.0]; LOCO_JOINT_COUNT],
    }
}

/// Scripted fallback pose: the idle stance a fresh machine emits at
/// rest (root `0.95 - 0.02` keeps the exact `base_pose` subtraction order).
/// Baseline locomotion stays net-free; this is what an absent net degrades
/// to, byte-tied to the scripted path by test.
pub fn scripted_fallback_pose() -> LocalPose {
    LocalPose {
        positions: vec![
            [0.0, 0.95 - 0.02, 0.0],
            [0.0, 0.22, 0.0],
            [0.0, 0.30, 0.0],
            [0.11, -0.90, 0.0],
            [-0.11, -0.90, 0.0],
            [0.32, -0.05, 0.0],
            [-0.32, -0.05, 0.0],
        ],
        rotations: vec![[0.0, 0.0, 0.0, 1.0]; LOCO_JOINT_COUNT],
    }
}

/// Net pose with scripted fallback: `Some` net infers, `None` clones the
/// given fallback (usually [`scripted_fallback_pose`]). Never panics, never
/// blocks; the baseline stays net-free.
pub fn infer_pose_or_fallback(
    net: Option<&MotionNet>,
    features: &[i8],
    fallback: &LocalPose,
) -> LocalPose {
    match net {
        Some(net) => net.infer_pose(features),
        None => fallback.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::contract::validate_pose;
    use super::super::locomotion::loco_skeleton;
    use super::*;

    fn w_u16(value: u16, out: &mut Vec<u8>) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn w_i32(value: i32, out: &mut Vec<u8>) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn header(input_dim: u16, layers: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&engine_nn::MAGIC);
        bytes.push(engine_nn::VERSION);
        bytes.push(layers);
        w_u16(input_dim, &mut bytes);
        bytes
    }

    /// One dense layer with identity requant and no activation: row `k`
    /// carries a single `1` at column `k % in_dim`, so output `k` gathers
    /// `features[k % in_dim]`. Built from the net byte layout, no weights
    /// ship with the crate.
    fn gather_net_bytes(in_dim: u16, out_dim: u16) -> Vec<u8> {
        let mut bytes = header(in_dim, 1);
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

    fn fixture_bytes() -> Vec<u8> {
        gather_net_bytes(FEATURE_DIM as u16, POSE_OUT_DIM as u16)
    }

    /// Weight byte offset of row `row`, column `col` in the fixture layout:
    /// header 8 + tag 1 + out_dim 2 + mult 4 + shift 1 + act 1 + bias
    /// `POSE_OUT_DIM * 4`, then row-major weights.
    fn weight_offset(row: usize, col: usize) -> usize {
        8 + 9 + POSE_OUT_DIM * 4 + row * FEATURE_DIM + col
    }

    #[test]
    fn hand_built_net_golden_pose() {
        let net = MotionNet::load_e2nn(&fixture_bytes()).expect("fixture loads");
        assert_eq!(net.input_dim(), FEATURE_DIM);
        assert_eq!(net.output_dim(), POSE_OUT_DIM);
        // Gather: outputs repeat the 8 features across the 21 pose lanes.
        let features: Vec<i8> = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let pose = net.infer_pose(&features);
        let step = |v: f32| v / 64.0;
        let expect = LocalPose {
            positions: vec![
                [step(1.0), step(2.0), step(3.0)],
                [step(4.0), step(5.0), step(6.0)],
                [step(7.0), step(8.0), step(1.0)],
                [step(2.0), step(3.0), step(4.0)],
                [step(5.0), step(6.0), step(7.0)],
                [step(8.0), step(1.0), step(2.0)],
                [step(3.0), step(4.0), step(5.0)],
            ],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]; LOCO_JOINT_COUNT],
        };
        assert_eq!(pose, expect);
        validate_pose(&loco_skeleton(), &pose).unwrap();
    }

    #[test]
    fn bit_exact_on_two_runs_and_reload() {
        let bytes = fixture_bytes();
        let net = MotionNet::load_e2nn(&bytes).expect("fixture loads");
        let gait = GaitSnapshot {
            state: LocoState::Walk,
            phase: 0.25,
            facing: [0.0, 1.0],
            speed: 1.5,
        };
        let wish = WishInput {
            speed: 1.5,
            dir: [0.0, 1.0],
        };
        let features = features_from(&gait, &wish);
        assert_eq!(features.len(), FEATURE_DIM);
        let first = net.infer_pose(&features);
        let second = net.infer_pose(&features);
        assert_eq!(first, second);
        let reloaded = MotionNet::load_e2nn(&bytes).expect("reloads");
        assert_eq!(reloaded, net);
        assert_eq!(reloaded.infer_pose(&features), first);
        // Same snapshot twice quantizes identically too.
        assert_eq!(features_from(&gait, &wish), features);
    }

    #[test]
    fn one_changed_weight_changes_the_pose() {
        let mut bytes = fixture_bytes();
        let at = weight_offset(0, 0);
        assert_eq!(bytes[at], 1u8);
        bytes[at] = 2u8;
        let net = MotionNet::load_e2nn(&bytes).expect("edited net still loads");
        let before = MotionNet::load_e2nn(&fixture_bytes()).expect("fixture loads");
        let features: Vec<i8> = vec![4, 0, 0, 0, 0, 0, 0, 0];
        let edited = net.infer_pose(&features);
        let golden = before.infer_pose(&features);
        assert_ne!(edited, golden);
        // Row 0 gathers `2 * features[0] = 8`; the rest is untouched.
        assert_eq!(edited.positions[0][0], 8.0 / 64.0);
        assert_eq!(edited.positions[0][1], golden.positions[0][1]);
        assert_eq!(edited.positions[6], golden.positions[6]);
    }

    #[test]
    fn malformed_net_rejected() {
        assert_eq!(
            MotionNet::load_e2nn(&[]),
            Err(MotionNetError::Nn(engine_nn::NnErr::Truncated))
        );
        let mut bad = fixture_bytes();
        bad[0] = b'Z';
        assert_eq!(
            MotionNet::load_e2nn(&bad),
            Err(MotionNetError::Nn(engine_nn::NnErr::BadMagic))
        );
        let bad = &fixture_bytes()[..fixture_bytes().len() - 1];
        assert_eq!(
            MotionNet::load_e2nn(bad),
            Err(MotionNetError::Nn(engine_nn::NnErr::Truncated))
        );
        // A valid net with the wrong input dim is rejected by lane.
        let narrow = gather_net_bytes(7, POSE_OUT_DIM as u16);
        engine_nn::load(&narrow).expect("narrow net is valid NN1");
        assert_eq!(
            MotionNet::load_e2nn(&narrow),
            Err(MotionNetError::BadInputDim {
                expected: FEATURE_DIM,
                got: 7
            })
        );
        // A valid net with the wrong output dim is rejected by lane.
        let short = gather_net_bytes(FEATURE_DIM as u16, 5);
        engine_nn::load(&short).expect("short net is valid NN1");
        assert_eq!(
            MotionNet::load_e2nn(&short),
            Err(MotionNetError::BadOutputDim {
                expected: POSE_OUT_DIM,
                got: 5
            })
        );
    }

    #[test]
    fn scripted_fallback_engages_on_absent_net() {
        let fallback = scripted_fallback_pose();
        validate_pose(&loco_skeleton(), &fallback).unwrap();
        // The fallback is the pose a fresh machine emits at rest, lane for
        // lane: the net-free baseline stays the authority.
        let mut machine = Locomotion::new();
        let idle = machine
            .update(0.0, [0.0, 1.0], 0, None)
            .expect("idle update cannot fail");
        assert_eq!(fallback, idle.pose);
        // Absent net clones the fallback; a present net infers instead.
        let features: Vec<i8> = vec![1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(infer_pose_or_fallback(None, &features, &fallback), fallback);
        let net = MotionNet::load_e2nn(&fixture_bytes()).expect("fixture loads");
        assert_eq!(
            infer_pose_or_fallback(Some(&net), &features, &fallback),
            net.infer_pose(&features)
        );
        assert_ne!(
            infer_pose_or_fallback(Some(&net), &features, &fallback),
            fallback
        );
    }

    #[test]
    fn features_from_goldens_and_nan_safety() {
        // Idle at rest facing +Z: state 0, zero lanes, full-scale unit lanes.
        let gait = GaitSnapshot {
            state: LocoState::Idle,
            phase: 0.0,
            facing: [0.0, 1.0],
            speed: 0.0,
        };
        let wish = WishInput {
            speed: 0.0,
            dir: [0.0, 1.0],
        };
        assert_eq!(
            features_from(&gait, &wish),
            vec![0, 0, 0, 127, 0, 0, 0, 127]
        );
        // Every state maps to a distinct small code.
        let codes = [
            LocoState::Idle,
            LocoState::Walk,
            LocoState::Run,
            LocoState::Strafe,
            LocoState::Start,
            LocoState::Stop,
            LocoState::Pivot,
            LocoState::Air,
            LocoState::Land,
        ]
        .map(state_code);
        assert_eq!(codes, [0, 1, 2, 3, 4, 5, 6, 7, 8]);
        // Non-finite lanes degrade to zero, never NaN out.
        let poisoned = GaitSnapshot {
            state: LocoState::Run,
            phase: f32::NAN,
            facing: [f32::INFINITY, 0.0],
            speed: f32::NAN,
        };
        let bad_wish = WishInput {
            speed: f32::NAN,
            dir: [0.0, f32::NEG_INFINITY],
        };
        assert_eq!(
            features_from(&poisoned, &bad_wish),
            vec![2, 0, 0, 0, 0, 0, 0, 0]
        );
        // Full-scale speed hits full-scale lanes; overspeed clamps.
        let fast = GaitSnapshot {
            state: LocoState::Run,
            phase: 0.5,
            facing: [0.0, 1.0],
            speed: MAX_WISH_SPEED,
        };
        let fast_wish = WishInput {
            speed: MAX_WISH_SPEED * 2.0,
            dir: [1.0, 0.0],
        };
        let lanes = features_from(&fast, &fast_wish);
        assert_eq!(lanes[LANE_SPEED], 127);
        assert_eq!(lanes[LANE_WISH_SPEED], 127);
        assert_eq!(lanes[LANE_PHASE], 63);
    }

    #[test]
    fn short_and_long_feature_slices_stay_total() {
        let net = MotionNet::load_e2nn(&fixture_bytes()).expect("fixture loads");
        // Short pads with zero: only lane 0 gathers `features[0] = 9`.
        let short = net.infer_pose(&[9]);
        assert_eq!(short.positions[0][0], 9.0 / 64.0);
        assert_eq!(short.positions[0][1], 0.0);
        validate_pose(&loco_skeleton(), &short).unwrap();
        // Long truncates: extra lanes change nothing.
        let base: Vec<i8> = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let mut long = base.clone();
        long.extend_from_slice(&[100, -100, 50]);
        assert_eq!(net.infer_pose(&long), net.infer_pose(&base));
    }
}
