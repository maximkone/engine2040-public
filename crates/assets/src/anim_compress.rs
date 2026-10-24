//! anim_compress: animation key compression — segment, quantize, error-loop.
//!
//! Consumer: the animation cook stores [`CompressedClip`](crate::anim_compress::CompressedClip) bytes beside a
//! cooked clip, and the runtime decodes them into caller buffers with no
//! allocation ([`decode_clip`](crate::anim_compress::decode_clip)). Data owner: `engine-assets`; `engine-anim`
//! owns the track contract and never imports assets back.
//! Units: meters and seconds in; positions decode in meters, rotation error
//! is measured in degrees, and the payload header carries both measured worst
//! errors so a consumer can read the bound without decoding.
//!
//! Pipeline (ideas from ACL, nfrechette/acl, MIT): [`compress_clip`](crate::anim_compress::compress_clip) splits
//! every track into linear segments whose interior keys stay inside half the
//! error budget, quantizes the segment endpoints (8-bit position codes and
//! 8-bit rotation components, 16-bit when a range needs it) and then decodes
//! its own work to measure the real worst error. A track that misses the
//! budget retries with 16-bit codes, then with tighter segments; a track that
//! still misses is a typed error, never a quiet overrun.
//!
//! Determinism: joint and key loops run in input order, every float is
//! written bit-exactly little-endian, and no clock or map iteration enters
//! the cook; the same clip and budget cook the same bytes twice.
//!
//! Payload: a 24-byte header (magic, version, flags, joint count, duration and
//! the measured worst position and rotation errors), then one block per joint:
//! position key count/bits, rotation key count/bits, the position base and
//! extent lanes, the key times (u16 over the clip duration, raw f32 only when
//! two times would collide) and the packed codes.
//!
//! Fixture provenance: the test clip is synthetic data authored in code (a
//! 12-joint smooth idle-style loop); no donor bytes.

use super::skin_import::{ImportedClip, QUAT_NORM_EPS};
use super::wire::{push_u16, push_u32, Cursor, Short};
use std::fmt;

/// Payload magic, `ANC1` in little-endian bytes.
pub const MAGIC: u32 = 0x3143_4E41;
/// Payload version this cook writes and [`decode_clip`] accepts.
pub const VERSION: u16 = 1;
/// Serialized header length in bytes.
pub const HEADER_LEN: usize = 24;
/// Largest key count one track may carry (the on-wire count is a `u16`).
pub const MAX_KEYS: usize = 65_535;
/// Default position budget: one centimeter.
pub const DEFAULT_POSITION_CM: f32 = 1.0;
/// Default rotation budget: one degree.
pub const DEFAULT_ROTATION_DEG: f32 = 1.0;
/// Position coordinates per key.
const POS_DIM: usize = 3;
/// Rotation components per key.
const ROT_DIM: usize = 4;
/// Code levels at 8 bits per component.
const LEVELS_8: u16 = 255;
/// Code levels at 16 bits per component.
const LEVELS_16: u16 = u16::MAX;
/// Time code levels: a key time is a fraction of the clip duration.
const TIME_LEVELS: u16 = u16::MAX;
/// Error-loop attempts: 8-bit, then 16-bit with halving segment tolerance.
const ATTEMPTS: usize = 6;
/// Segment tolerance per attempt, as a fraction of the error budget.
const TOLERANCE: [f32; ATTEMPTS] = [0.5, 0.5, 0.25, 0.125, 0.0625, 0.0];
/// Centimeters per meter for error reporting.
const CM_PER_M: f32 = 100.0;
/// Radians to degrees for the rotation error.
const RAD_TO_DEG: f32 = 57.295_78;

/// Why a compression cook or a decode failed; every malformed input maps to
/// one variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnimCompressError {
    /// The clip carries no joints.
    NoJoints,
    /// The rotation track count differs from the position track count.
    TrackCount {
        /// Position track count.
        expected: usize,
        /// Rotation track count.
        got: usize,
    },
    /// A track is empty, ragged, unordered, before zero or past the duration.
    BadTrack {
        /// Joint index in skeleton order.
        joint: usize,
    },
    /// A rotation key is not near unit length.
    BadRotation {
        /// Joint index in skeleton order.
        joint: usize,
    },
    /// A float lane is not finite; the lane is named.
    NonFinite(&'static str),
    /// A track holds more keys than the on-wire count can express.
    TooManyKeys {
        /// Joint index in skeleton order.
        joint: usize,
    },
    /// The error budget is not finite or not positive.
    BadBudget,
    /// No segment precision met the budget; the plane is named.
    BudgetUnreachable {
        /// Joint index in skeleton order.
        joint: usize,
        /// True for the rotation lane.
        rotation: bool,
    },
    /// A caller decode buffer is smaller than the payload needs.
    OutTooSmall {
        /// Slots needed.
        need: usize,
        /// Slots present.
        got: usize,
    },
    /// The payload header or body is malformed.
    BadPayload,
}

impl fmt::Display for AnimCompressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoJoints => write!(f, "anim compress wants at least one joint"),
            Self::TrackCount { expected, got } => {
                write!(f, "anim compress wants {expected} rot tracks, found {got}")
            }
            Self::BadTrack { joint } => write!(f, "anim compress bad track joint {joint}"),
            Self::BadRotation { joint } => write!(f, "anim compress bad rotation joint {joint}"),
            Self::NonFinite(lane) => write!(f, "anim compress non-finite {lane}"),
            Self::TooManyKeys { joint } => {
                write!(f, "anim compress track {joint} is over {MAX_KEYS} keys")
            }
            Self::BadBudget => write!(f, "anim compress budget wants finite positive values"),
            Self::BudgetUnreachable { joint, rotation } => {
                let plane = if *rotation { "rotation" } else { "position" };
                write!(f, "anim compress {plane} joint {joint} missed the budget")
            }
            Self::OutTooSmall { need, got } => {
                write!(f, "anim compress wants {need} decode slots, found {got}")
            }
            Self::BadPayload => write!(f, "anim compress bad payload"),
        }
    }
}

impl std::error::Error for AnimCompressError {}

/// Error budget for one compressed clip: the worst position distance in
/// centimeters and the worst rotation angle in degrees a decode may be off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompressBudget {
    /// Allowed position error in centimeters.
    pub position_cm: f32,
    /// Allowed rotation error in degrees.
    pub rotation_deg: f32,
}

impl Default for CompressBudget {
    fn default() -> Self {
        Self {
            position_cm: DEFAULT_POSITION_CM,
            rotation_deg: DEFAULT_ROTATION_DEG,
        }
    }
}

impl CompressBudget {
    /// Refuse a budget that is not finite and positive on both lanes.
    pub fn checked(self) -> Result<Self, AnimCompressError> {
        if !self.position_cm.is_finite()
            || !self.rotation_deg.is_finite()
            || self.position_cm <= 0.0
            || self.rotation_deg <= 0.0
        {
            return Err(AnimCompressError::BadBudget);
        }
        Ok(self)
    }
}

/// Header every payload starts with: what the clip holds and how far a decode
/// can be from it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClipHeader {
    /// Joint count the payload carries.
    pub joints: usize,
    /// Clip duration in seconds.
    pub duration: f32,
    /// Measured worst position error in centimeters.
    pub max_error_cm: f32,
    /// Measured worst rotation error in degrees.
    pub max_error_deg: f32,
}

/// One compressed clip: the serialized payload. [`compress_clip`] cooks it;
/// [`CompressedClip::from_bytes`] opens bytes read back from disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompressedClip {
    bytes: Vec<u8>,
}
impl CompressedClip {
    /// Serialized payload, ready for a bundle or a disk write.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Serialized payload length in bytes.
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }

    /// Open a payload read back from disk; a bad magic, version or short
    /// header is refused with [`AnimCompressError::BadPayload`].
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, AnimCompressError> {
        let clip = Self { bytes };
        parse_header(clip.as_bytes())?;
        Ok(clip)
    }

    /// Header fields: joint count, duration and the measured error bounds.
    pub fn header(&self) -> Result<ClipHeader, AnimCompressError> {
        parse_header(&self.bytes)
    }
}

/// Caller-owned decode buffers for one clip. Each lane holds `max_keys` slots
/// per joint, joint-major; [`decode_clip`] writes the first `count` slots of
/// every joint and never reallocates them.
pub struct ClipBuffers<'a> {
    /// Slots per joint in every lane; at least the largest track key count.
    pub max_keys: usize,
    /// Position key times, `joints * max_keys` slots.
    pub position_times: &'a mut [f32],
    /// Decoded positions in meters, `joints * max_keys` slots.
    pub positions: &'a mut [[f32; 3]],
    /// Rotation key times, `joints * max_keys` slots.
    pub rotation_times: &'a mut [f32],
    /// Decoded unit rotations, `joints * max_keys` slots.
    pub rotations: &'a mut [[f32; 4]],
}

/// Key counts [`decode_clip`] wrote for one joint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DecodedCounts {
    /// Position keys written.
    pub position_keys: usize,
    /// Rotation keys written.
    pub rotation_keys: usize,
}

/// Slots one lane of a [`ClipBuffers`] needs for `joints` joints at
/// `max_keys` keys each.
pub fn required_slots(joints: usize, max_keys: usize) -> usize {
    joints.saturating_mul(max_keys)
}

/// Uncompressed f32 footprint of one clip: 4 bytes per key time plus 12 per
/// position and 16 per rotation value. The compression ratio is measured
/// against this size.
pub fn raw_clip_bytes(clip: &ImportedClip) -> usize {
    let mut total = 0usize;
    for track in &clip.positions.tracks {
        total = total
            .saturating_add(track.times.len().saturating_mul(4))
            .saturating_add(track.values.len().saturating_mul(12));
    }
    for track in &clip.rotations {
        total = total
            .saturating_add(track.times.len().saturating_mul(4))
            .saturating_add(track.values.len().saturating_mul(16));
    }
    total
}

/// `raw / compressed` size ratio of one cook; zero when either side is empty.
pub fn compression_ratio(clip: &ImportedClip, compressed: &CompressedClip) -> f64 {
    let raw = raw_clip_bytes(clip);
    let packed = compressed.byte_len();
    if raw == 0 || packed == 0 {
        return 0.0;
    }
    let raw_f = f64::from(u32::try_from(raw).unwrap_or(u32::MAX));
    let packed_f = f64::from(u32::try_from(packed).unwrap_or(u32::MAX));
    raw_f / packed_f
}

/// Which lane of a joint a track belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plane {
    Position,
    Rotation,
}

impl Plane {
    /// Components per key.
    fn dim(self) -> usize {
        match self {
            Self::Position => POS_DIM,
            Self::Rotation => ROT_DIM,
        }
    }
}

/// One planned track: the kept keys, their quantized times and codes, the
/// position base/extent lanes and the measured worst error.
struct PlannedTrack {
    bits: u8,
    time_bits: u8,
    times16: Vec<u16>,
    times32: Vec<f32>,
    base: [f32; 3],
    extent: [f32; 3],
    codes: Vec<u16>,
    error: f32,
}

impl PlannedTrack {
    fn key_count(&self, plane: Plane) -> usize {
        self.codes.len() / plane.dim()
    }
}

/// Check every lane of a clip before any byte is written.
fn validate_clip(clip: &ImportedClip) -> Result<(), AnimCompressError> {
    let joints = clip.positions.tracks.len();
    if joints == 0 {
        return Err(AnimCompressError::NoJoints);
    }
    if clip.rotations.len() != joints {
        return Err(AnimCompressError::TrackCount {
            expected: joints,
            got: clip.rotations.len(),
        });
    }
    if !clip.duration.is_finite() || clip.duration < 0.0 {
        return Err(AnimCompressError::NonFinite("clip.duration"));
    }
    for (joint, (positions, rotations)) in clip
        .positions
        .tracks
        .iter()
        .zip(clip.rotations.iter())
        .enumerate()
    {
        check_track(
            joint,
            &positions.times,
            positions.values.len(),
            clip.duration,
        )?;
        check_track(
            joint,
            &rotations.times,
            rotations.values.len(),
            clip.duration,
        )?;
        if positions
            .values
            .iter()
            .flatten()
            .any(|value| !value.is_finite())
        {
            return Err(AnimCompressError::NonFinite("clip.values"));
        }
        for quat in &rotations.values {
            if quat.iter().any(|value| !value.is_finite()) {
                return Err(AnimCompressError::NonFinite("clip.rotations"));
            }
            let norm = quat.iter().map(|value| value * value).sum::<f32>().sqrt();
            if (norm - 1.0).abs() > QUAT_NORM_EPS {
                return Err(AnimCompressError::BadRotation { joint });
            }
        }
    }
    Ok(())
}

/// One track's timing rules: non-empty, paired, finite, at or after zero,
/// strictly increasing and at or before the clip duration.
fn check_track(
    joint: usize,
    times: &[f32],
    values: usize,
    duration: f32,
) -> Result<(), AnimCompressError> {
    if times.is_empty() || times.len() != values {
        return Err(AnimCompressError::BadTrack { joint });
    }
    if times.len() > MAX_KEYS {
        return Err(AnimCompressError::TooManyKeys { joint });
    }
    let mut prev = -1.0f32;
    for time in times {
        if !time.is_finite() || *time < 0.0 || *time <= prev || *time > duration {
            return Err(AnimCompressError::BadTrack { joint });
        }
        prev = *time;
    }
    Ok(())
}

/// Cook one clip: segment, quantize, measure, retry until the budget holds.
/// The same clip plus budget always yields the same bytes.
pub fn compress_clip(
    clip: &ImportedClip,
    budget: &CompressBudget,
) -> Result<CompressedClip, AnimCompressError> {
    let budget = budget.checked()?;
    validate_clip(clip)?;
    let joints = clip.positions.tracks.len();
    let mut out = Vec::new();
    push_u32(&mut out, MAGIC);
    push_u16(&mut out, VERSION);
    push_u16(&mut out, 0);
    push_u32(&mut out, u32_of(joints));
    push_f32(&mut out, clip.duration);
    push_f32(&mut out, 0.0);
    push_f32(&mut out, 0.0);
    let mut worst_cm = 0.0f32;
    let mut worst_deg = 0.0f32;
    let budget_m = budget.position_cm / CM_PER_M;
    for joint in 0..joints {
        let positions = &clip.positions.tracks[joint];
        let rotations = &clip.rotations[joint];
        let position_values: Vec<[f32; 4]> = positions
            .values
            .iter()
            .map(|value| [value[0], value[1], value[2], 0.0])
            .collect();
        let planned = plan_track(
            joint,
            &positions.times,
            &position_values,
            Plane::Position,
            budget_m,
            clip.duration,
        )?;
        worst_cm = worst_cm.max(planned.error * CM_PER_M);
        write_track(&mut out, &planned, Plane::Position);
        let planned = plan_track(
            joint,
            &rotations.times,
            &rotations.values,
            Plane::Rotation,
            budget.rotation_deg,
            clip.duration,
        )?;
        worst_deg = worst_deg.max(planned.error);
        write_track(&mut out, &planned, Plane::Rotation);
    }
    out[HEADER_LEN - 8..HEADER_LEN - 4].copy_from_slice(&worst_cm.to_bits().to_le_bytes());
    out[HEADER_LEN - 4..HEADER_LEN].copy_from_slice(&worst_deg.to_bits().to_le_bytes());
    Ok(CompressedClip { bytes: out })
}

/// Plan one track: try 8-bit codes, then 16-bit codes with halving segment
/// tolerance until a self-measured decode stays inside `budget`.
fn plan_track(
    joint: usize,
    times: &[f32],
    values: &[[f32; 4]],
    plane: Plane,
    budget: f32,
    duration: f32,
) -> Result<PlannedTrack, AnimCompressError> {
    for (attempt, share) in TOLERANCE.iter().enumerate() {
        let bits = if attempt == 0 { 8u8 } else { 16u8 };
        let keep = segment_keys(times, values, plane, budget * share);
        let mut planned = encode_keys(times, values, plane, bits, &keep, duration);
        planned.error = measure_error(times, values, plane, &planned, duration);
        if planned.error <= budget {
            return Ok(planned);
        }
    }
    Err(AnimCompressError::BudgetUnreachable {
        joint,
        rotation: plane == Plane::Rotation,
    })
}

/// Greedy longest-run segmentation: keep key zero, then always the farthest
/// key whose interior keys stay within `tolerance` of the endpoint line.
fn segment_keys(times: &[f32], values: &[[f32; 4]], plane: Plane, tolerance: f32) -> Vec<usize> {
    let count = times.len();
    let mut keep = Vec::new();
    if count == 0 {
        return keep;
    }
    keep.push(0);
    let mut start = 0usize;
    while start + 1 < count {
        let mut end = start + 1;
        while end + 1 < count && segment_ok(times, values, plane, start, end + 1, tolerance) {
            end += 1;
        }
        keep.push(end);
        start = end;
    }
    keep
}

/// True when every interior key of `start..=end` sits within `tolerance` of
/// the linear interpolation of the two endpoint values.
fn segment_ok(
    times: &[f32],
    values: &[[f32; 4]],
    plane: Plane,
    start: usize,
    end: usize,
    tolerance: f32,
) -> bool {
    let span = times[end] - times[start];
    if span <= 0.0 {
        return false;
    }
    for key in start + 1..end {
        let fraction = clamp01((times[key] - times[start]) / span);
        let guess = interp(plane, values[start], values[end], fraction);
        if plane_distance(plane, values[key], guess) > tolerance {
            return false;
        }
    }
    true
}

/// Encode the kept keys at one precision: quantized times, position
/// base/extent lanes and per-component codes.
fn encode_keys(
    times: &[f32],
    values: &[[f32; 4]],
    plane: Plane,
    bits: u8,
    keep: &[usize],
    duration: f32,
) -> PlannedTrack {
    let levels = levels_of(bits);
    let dim = plane.dim();
    let mut base = [-1.0f32; 3];
    let mut extent = [0.0f32; 3];
    if plane == Plane::Position {
        for (axis, (base_slot, extent_slot)) in base.iter_mut().zip(extent.iter_mut()).enumerate() {
            let mut low = f32::INFINITY;
            let mut high = f32::NEG_INFINITY;
            for &key in keep {
                let value = values[key][axis];
                if value < low {
                    low = value;
                }
                if value > high {
                    high = value;
                }
            }
            *base_slot = low;
            *extent_slot = high - low;
        }
    }
    let mut times16 = Vec::with_capacity(keep.len());
    let mut ordered = true;
    for &key in keep {
        let code = time_code(times[key], duration);
        if let Some(previous) = times16.last() {
            if code <= *previous {
                ordered = false;
            }
        }
        times16.push(code);
    }
    let (time_bits, times16, times32) = if ordered {
        (16u8, times16, Vec::new())
    } else {
        (
            32u8,
            Vec::new(),
            keep.iter().map(|key| times[*key]).collect(),
        )
    };
    let mut codes = Vec::with_capacity(keep.len().saturating_mul(dim));
    for &key in keep {
        let mut axis = 0usize;
        while axis < dim {
            let unit = if plane == Plane::Rotation {
                (values[key][axis] + 1.0) * 0.5
            } else if extent[axis] > 0.0 {
                (values[key][axis] - base[axis]) / extent[axis]
            } else {
                0.0
            };
            codes.push(unit_to_code(unit, levels));
            axis += 1;
        }
    }
    PlannedTrack {
        bits,
        time_bits,
        times16,
        times32,
        base,
        extent,
        codes,
        error: 0.0,
    }
}

/// Decode a planned track back to f32 keys; the measurement walks these.
fn decoded_keys(planned: &PlannedTrack, plane: Plane, duration: f32) -> (Vec<f32>, Vec<[f32; 4]>) {
    let levels = levels_of(planned.bits);
    let dim = plane.dim();
    let count = planned.key_count(plane);
    let mut times = Vec::with_capacity(count);
    for index in 0..count {
        let time = if planned.time_bits == 16 {
            f32::from(planned.times16[index]) / f32::from(TIME_LEVELS) * duration
        } else {
            planned.times32[index]
        };
        times.push(time);
    }
    let mut values = Vec::with_capacity(count);
    for index in 0..count {
        let mut value = [0.0f32; 4];
        for (axis, lane) in value.iter_mut().enumerate().take(dim) {
            let unit = code_to_unit(planned.codes[index * dim + axis], levels);
            *lane = if plane == Plane::Rotation {
                unit * 2.0 - 1.0
            } else {
                planned.base[axis] + unit * planned.extent[axis]
            };
        }
        if plane == Plane::Rotation {
            value = normalize4(value).unwrap_or([0.0, 0.0, 0.0, 1.0]);
        }
        values.push(value);
    }
    (times, values)
}

/// Worst error of a planned track against every original key, measured with
/// the consumer's interpolation (lerp for positions, normalized lerp for
/// rotations).
fn measure_error(
    times: &[f32],
    values: &[[f32; 4]],
    plane: Plane,
    planned: &PlannedTrack,
    duration: f32,
) -> f32 {
    let (decoded_times, decoded_values) = decoded_keys(planned, plane, duration);
    let mut worst = 0.0f32;
    for (key, time) in times.iter().enumerate() {
        let guess = sample_keys(plane, &decoded_times, &decoded_values, *time);
        let error = plane_distance(plane, values[key], guess);
        if error > worst {
            worst = error;
        }
    }
    worst
}

/// Sample a decoded key list at `time`; clamps outside the key range.
fn sample_keys(plane: Plane, times: &[f32], values: &[[f32; 4]], time: f32) -> [f32; 4] {
    if times.is_empty() || values.is_empty() {
        return rest(plane);
    }
    let last = times.len() - 1;
    if time <= times[0] {
        return values[0];
    }
    if time >= times[last] {
        return values[last];
    }
    let mut low = 0usize;
    while low + 1 < last && times[low + 1] <= time {
        low += 1;
    }
    let span = times[low + 1] - times[low];
    let fraction = if span > 0.0 {
        clamp01((time - times[low]) / span)
    } else {
        0.0
    };
    interp(plane, values[low], values[low + 1], fraction)
}

/// Interpolate one key pair the way the anim sampler does.
fn interp(plane: Plane, a: [f32; 4], b: [f32; 4], fraction: f32) -> [f32; 4] {
    match plane {
        Plane::Position => [
            a[0] + (b[0] - a[0]) * fraction,
            a[1] + (b[1] - a[1]) * fraction,
            a[2] + (b[2] - a[2]) * fraction,
            0.0,
        ],
        Plane::Rotation => nlerp4(a, b, fraction),
    }
}

/// Shortest-arc normalized lerp between two unit quats; identity on a
/// degenerate result.
fn nlerp4(a: [f32; 4], b: [f32; 4], fraction: f32) -> [f32; 4] {
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let sign = if dot < 0.0 { -1.0 } else { 1.0 };
    let mixed = [
        a[0] + (sign * b[0] - a[0]) * fraction,
        a[1] + (sign * b[1] - a[1]) * fraction,
        a[2] + (sign * b[2] - a[2]) * fraction,
        a[3] + (sign * b[3] - a[3]) * fraction,
    ];
    normalize4(mixed).unwrap_or([0.0, 0.0, 0.0, 1.0])
}

/// Normalize a quaternion; `None` on degenerate input.
fn normalize4(quat: [f32; 4]) -> Option<[f32; 4]> {
    let norm_squared = quat.iter().map(|value| value * value).sum::<f32>();
    if !norm_squared.is_finite() || norm_squared <= 0.0 {
        return None;
    }
    let inverse = 1.0 / norm_squared.sqrt();
    Some([
        quat[0] * inverse,
        quat[1] * inverse,
        quat[2] * inverse,
        quat[3] * inverse,
    ])
}

/// Distance in one plane's own unit: meters for positions, degrees for
/// rotations.
fn plane_distance(plane: Plane, a: [f32; 4], b: [f32; 4]) -> f32 {
    match plane {
        Plane::Position => {
            let dx = a[0] - b[0];
            let dy = a[1] - b[1];
            let dz = a[2] - b[2];
            (dx * dx + dy * dy + dz * dz).sqrt()
        }
        Plane::Rotation => {
            let left = normalize4(a).unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let right = normalize4(b).unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let dot =
                (left[0] * right[0] + left[1] * right[1] + left[2] * right[2] + left[3] * right[3])
                    .abs()
                    .clamp(0.0, 1.0);
            2.0 * dot.acos() * RAD_TO_DEG
        }
    }
}

/// The neutral value of one plane: the origin for positions, identity for
/// rotations.
fn rest(plane: Plane) -> [f32; 4] {
    match plane {
        Plane::Position => [0.0, 0.0, 0.0, 0.0],
        Plane::Rotation => [0.0, 0.0, 0.0, 1.0],
    }
}

/// Code levels for one precision.
fn levels_of(bits: u8) -> u16 {
    if bits == 8 {
        LEVELS_8
    } else {
        LEVELS_16
    }
}

/// `unit` clamped into `0.0..=1.0`.
fn clamp01(unit: f32) -> f32 {
    if unit.is_nan() {
        0.0
    } else {
        unit.clamp(0.0, 1.0)
    }
}

/// Round a unit value onto `0..=levels` without a cast; NaN, negative and
/// out-of-range values clamp to the ends.
fn unit_to_code(unit: f32, levels: u16) -> u16 {
    if unit.is_nan() || unit <= 0.0 {
        return 0;
    }
    if unit >= 1.0 {
        return levels;
    }
    let top = f32::from(levels);
    let scaled = (unit * top).round().clamp(0.0, top);
    integral_to_u16(scaled).min(levels)
}

/// A non-negative whole `f32` at or below `65535.0` as `u16`, rebuilt from the
/// exponent and mantissa so the conversion is a named shift, never a cast.
fn integral_to_u16(integral: f32) -> u16 {
    if !integral.is_finite() || integral <= 0.0 {
        return 0;
    }
    if integral >= 65_535.0 {
        return u16::MAX;
    }
    let bits = integral.to_bits();
    let exponent = (bits >> 23) & 0xFF;
    if exponent > 143 {
        return u16::MAX;
    }
    let mantissa = (bits & 0x007F_FFFF) | 0x0080_0000;
    let shift = 150u32.saturating_sub(exponent);
    u16::try_from(mantissa >> shift).unwrap_or(u16::MAX)
}

/// `code / levels` as `0.0..=1.0`.
fn code_to_unit(code: u16, levels: u16) -> f32 {
    f32::from(code) / f32::from(levels)
}

/// Quantized key time: a fraction of the clip duration.
fn time_code(time: f32, duration: f32) -> u16 {
    let unit = if duration > 0.0 { time / duration } else { 0.0 };
    unit_to_code(unit, TIME_LEVELS)
}

/// Append one `f32` bit-exactly.
fn push_f32(out: &mut Vec<u8>, value: f32) {
    push_u32(out, value.to_bits());
}

/// Append one planned track: counts and precisions, position lanes, times and
/// codes.
fn write_track(out: &mut Vec<u8>, planned: &PlannedTrack, plane: Plane) {
    let count = planned.key_count(plane);
    push_u16(out, u16::try_from(count).unwrap_or(u16::MAX));
    out.push(planned.bits);
    out.push(planned.time_bits);
    if plane == Plane::Position {
        for value in planned.base {
            push_f32(out, value);
        }
        for value in planned.extent {
            push_f32(out, value);
        }
    }
    if planned.time_bits == 16 {
        for code in &planned.times16 {
            push_u16(out, *code);
        }
    } else {
        for time in &planned.times32 {
            push_f32(out, *time);
        }
    }
    if planned.bits == 8 {
        for code in &planned.codes {
            out.push(u8::try_from(*code).unwrap_or(u8::MAX));
        }
    } else {
        for code in &planned.codes {
            push_u16(out, *code);
        }
    }
}

/// Parse the fixed header; refuses a short, foreign or future payload.
fn parse_header(bytes: &[u8]) -> Result<ClipHeader, AnimCompressError> {
    if bytes.len() < HEADER_LEN {
        return Err(AnimCompressError::BadPayload);
    }
    let mut cursor = Cursor::new(bytes);
    let magic = read_u32(&mut cursor)?;
    if magic != MAGIC {
        return Err(AnimCompressError::BadPayload);
    }
    let version = cursor.u16().map_err(short_read)?;
    if version != VERSION {
        return Err(AnimCompressError::BadPayload);
    }
    let _flags = cursor.u16().map_err(short_read)?;
    let joints = usize::try_from(read_u32(&mut cursor)?).unwrap_or(usize::MAX);
    let duration = read_f32(&mut cursor)?;
    let max_error_cm = read_f32(&mut cursor)?;
    let max_error_deg = read_f32(&mut cursor)?;
    if joints == 0 || !duration.is_finite() || duration < 0.0 {
        return Err(AnimCompressError::BadPayload);
    }
    Ok(ClipHeader {
        joints,
        duration,
        max_error_cm,
        max_error_deg,
    })
}

/// Decode one payload into caller buffers with no heap use at all. Returns the
/// header so the caller reads the duration and the measured bounds for free.
pub fn decode_clip(
    compressed: &CompressedClip,
    buffers: &mut ClipBuffers<'_>,
    counts: &mut [DecodedCounts],
) -> Result<ClipHeader, AnimCompressError> {
    let header = parse_header(compressed.as_bytes())?;
    let joints = header.joints;
    if counts.len() < joints {
        return Err(AnimCompressError::OutTooSmall {
            need: joints,
            got: counts.len(),
        });
    }
    if buffers.max_keys == 0 {
        return Err(AnimCompressError::OutTooSmall { need: 1, got: 0 });
    }
    let need = required_slots(joints, buffers.max_keys);
    for got in [
        buffers.position_times.len(),
        buffers.positions.len(),
        buffers.rotation_times.len(),
        buffers.rotations.len(),
    ] {
        if got < need {
            return Err(AnimCompressError::OutTooSmall { need, got });
        }
    }
    let max_keys = buffers.max_keys;
    let mut cursor = Cursor::at(compressed.as_bytes(), HEADER_LEN);
    for (joint, slot) in counts.iter_mut().enumerate().take(joints) {
        let offset = joint.saturating_mul(max_keys);
        let base = read_track_header(&mut cursor, Plane::Position)?;
        if base.count > max_keys {
            return Err(AnimCompressError::OutTooSmall {
                need: base.count,
                got: max_keys,
            });
        }
        let (position_times, positions) = (
            &mut buffers.position_times[offset..offset + base.count],
            &mut buffers.positions[offset..offset + base.count],
        );
        decode_positions(
            &mut cursor,
            &base,
            position_times,
            positions,
            header.duration,
        )?;
        let rotations_head = read_track_header(&mut cursor, Plane::Rotation)?;
        if rotations_head.count > max_keys {
            return Err(AnimCompressError::OutTooSmall {
                need: rotations_head.count,
                got: max_keys,
            });
        }
        let (rotation_times, rotations) = (
            &mut buffers.rotation_times[offset..offset + rotations_head.count],
            &mut buffers.rotations[offset..offset + rotations_head.count],
        );
        decode_rotations(
            &mut cursor,
            &rotations_head,
            rotation_times,
            rotations,
            header.duration,
        )?;
        *slot = DecodedCounts {
            position_keys: base.count,
            rotation_keys: rotations_head.count,
        };
    }
    if cursor.pos() != compressed.byte_len() {
        return Err(AnimCompressError::BadPayload);
    }
    Ok(header)
}

/// One track's on-wire header: key count, code/time precisions and the
/// position lanes (rotations keep the fixed `-1..=1` component range).
#[derive(Clone, Copy, Debug)]
struct TrackHeader {
    count: usize,
    bits: u8,
    time_bits: u8,
    base: [f32; 3],
    extent: [f32; 3],
}

/// Read one track header; the position lanes are present for `Position` and
/// stay zero for `Rotation`.
fn read_track_header(
    cursor: &mut Cursor<'_>,
    plane: Plane,
) -> Result<TrackHeader, AnimCompressError> {
    let count = usize::from(cursor.u16().map_err(short_read)?);
    let bits = cursor.u8().map_err(short_read)?;
    let time_bits = cursor.u8().map_err(short_read)?;
    if count == 0 || !matches!(bits, 8 | 16) || !matches!(time_bits, 16 | 32) {
        return Err(AnimCompressError::BadPayload);
    }
    let mut base = [0.0f32; 3];
    let mut extent = [0.0f32; 3];
    if plane == Plane::Position {
        for slot in base.iter_mut() {
            *slot = read_f32(cursor)?;
        }
        for slot in extent.iter_mut() {
            *slot = read_f32(cursor)?;
        }
        for value in base.iter().chain(extent.iter()) {
            if !value.is_finite() {
                return Err(AnimCompressError::BadPayload);
            }
        }
    }
    Ok(TrackHeader {
        count,
        bits,
        time_bits,
        base,
        extent,
    })
}

/// Read one track's key times into `out`; refuses an unordered lane.
fn read_times(
    cursor: &mut Cursor<'_>,
    head: &TrackHeader,
    out: &mut [f32],
    duration: f32,
) -> Result<(), AnimCompressError> {
    let mut prev = -1.0f32;
    for slot in out.iter_mut().take(head.count) {
        let time = if head.time_bits == 16 {
            code_to_unit(cursor.u16().map_err(short_read)?, TIME_LEVELS) * duration
        } else {
            read_f32(cursor)?
        };
        if !time.is_finite() || time < 0.0 || time <= prev {
            return Err(AnimCompressError::BadPayload);
        }
        prev = time;
        *slot = time;
    }
    Ok(())
}

/// Read one track's next code at the header's precision.
fn read_code(cursor: &mut Cursor<'_>, bits: u8) -> Result<u16, AnimCompressError> {
    if bits == 8 {
        cursor.u8().map(u16::from).map_err(short_read)
    } else {
        cursor.u16().map_err(short_read)
    }
}

/// Decode one position track into meters.
fn decode_positions(
    cursor: &mut Cursor<'_>,
    head: &TrackHeader,
    times: &mut [f32],
    out: &mut [[f32; 3]],
    duration: f32,
) -> Result<(), AnimCompressError> {
    read_times(cursor, head, times, duration)?;
    let levels = levels_of(head.bits);
    for slot in out.iter_mut().take(head.count) {
        for (axis, lane) in slot.iter_mut().enumerate() {
            let code = read_code(cursor, head.bits)?;
            let unit = code_to_unit(code, levels);
            *lane = head.base[axis] + unit * head.extent[axis];
        }
    }
    Ok(())
}

/// Decode one rotation track into unit quats.
fn decode_rotations(
    cursor: &mut Cursor<'_>,
    head: &TrackHeader,
    times: &mut [f32],
    out: &mut [[f32; 4]],
    duration: f32,
) -> Result<(), AnimCompressError> {
    read_times(cursor, head, times, duration)?;
    let levels = levels_of(head.bits);
    for slot in out.iter_mut().take(head.count) {
        let mut quat = [0.0f32; 4];
        for lane in quat.iter_mut() {
            let code = read_code(cursor, head.bits)?;
            *lane = code_to_unit(code, levels) * 2.0 - 1.0;
        }
        *slot = normalize4(quat).ok_or(AnimCompressError::BadPayload)?;
    }
    Ok(())
}

/// Read one little-endian `u32`.
fn read_u32(cursor: &mut Cursor<'_>) -> Result<u32, AnimCompressError> {
    cursor.u32().map_err(short_read)
}

/// Read one bit-exact `f32`.
fn read_f32(cursor: &mut Cursor<'_>) -> Result<f32, AnimCompressError> {
    read_u32(cursor).map(f32::from_bits)
}

/// Map a short read onto the payload error.
fn short_read(_: Short) -> AnimCompressError {
    AnimCompressError::BadPayload
}

/// A joint count as the on-wire `u32`, saturating past the count range.
fn u32_of(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: engine_core::alloc_probe::CountingAlloc =
    engine_core::alloc_probe::CountingAlloc;

#[cfg(test)]
mod tests {
    use super::*;
    use engine_anim::pose::QuatTrack;
    use engine_anim::{Clip, Track};

    /// Joints in the synthetic fixture clip.
    const JOINTS: usize = 12;
    /// Keys per track in the synthetic fixture clip.
    const KEYS: usize = 30;

    /// A small `usize` as `f32` without a cast.
    fn idx(value: usize) -> f32 {
        f32::from(u16::try_from(value).unwrap_or(u16::MAX))
    }

    /// Synthetic smooth loop: every joint swings on three axes and turns on
    /// one tilted axis, sampled 30 times over one second.
    fn smooth_clip() -> ImportedClip {
        let axis = normalize4([0.3, 1.0, 0.2, 0.0]).unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let mut positions = Vec::with_capacity(JOINTS);
        let mut rotations = Vec::with_capacity(JOINTS);
        for joint in 0..JOINTS {
            let phase = idx(joint) * 0.37;
            let swing = 0.06 + idx(joint) * 0.004;
            let turn = 0.25 + idx(joint) * 0.012;
            let mut position_times = Vec::with_capacity(KEYS);
            let mut position_values = Vec::with_capacity(KEYS);
            let mut rotation_times = Vec::with_capacity(KEYS);
            let mut rotation_values = Vec::with_capacity(KEYS);
            for key in 0..KEYS {
                let time = idx(key) / 29.0;
                let wave = (time * std::f32::consts::TAU + phase).sin();
                let side = (time * std::f32::consts::TAU * 2.0 + phase).sin();
                position_times.push(time);
                position_values.push([swing * wave, 0.9 + 0.02 * side, 0.4 + swing * 0.5 * wave]);
                rotation_times.push(time);
                let half = turn * wave * 0.5;
                rotation_values.push([
                    axis[0] * half.sin(),
                    axis[1] * half.sin(),
                    axis[2] * half.sin(),
                    half.cos(),
                ]);
            }
            positions.push(Track {
                times: position_times,
                values: position_values,
            });
            rotations.push(QuatTrack {
                times: rotation_times,
                values: rotation_values,
            });
        }
        ImportedClip {
            name: "idle_smooth".to_string(),
            duration: 1.0,
            positions: Clip { tracks: positions },
            rotations,
        }
    }

    /// Decoded clip in the module's own `[f32; 4]` lane shape.
    struct Decoded {
        position_times: Vec<Vec<f32>>,
        position_values: Vec<Vec<[f32; 4]>>,
        rotation_times: Vec<Vec<f32>>,
        rotation_values: Vec<Vec<[f32; 4]>>,
    }

    /// Decode a payload into fresh per-joint vectors.
    fn decode_all(clip: &CompressedClip, max_keys: usize) -> Decoded {
        let joints = clip.header().expect("header").joints;
        let slots = required_slots(joints, max_keys);
        let mut position_times = vec![0.0f32; slots];
        let mut positions = vec![[0.0f32; 3]; slots];
        let mut rotation_times = vec![0.0f32; slots];
        let mut rotations = vec![[0.0f32; 4]; slots];
        let mut counts = vec![DecodedCounts::default(); joints];
        {
            let mut buffers = ClipBuffers {
                max_keys,
                position_times: &mut position_times,
                positions: &mut positions,
                rotation_times: &mut rotation_times,
                rotations: &mut rotations,
            };
            decode_clip(clip, &mut buffers, &mut counts).expect("decode");
        }
        let mut out = Decoded {
            position_times: Vec::with_capacity(joints),
            position_values: Vec::with_capacity(joints),
            rotation_times: Vec::with_capacity(joints),
            rotation_values: Vec::with_capacity(joints),
        };
        for (joint, count) in counts.iter().enumerate() {
            let start = joint * max_keys;
            out.position_times
                .push(position_times[start..start + count.position_keys].to_vec());
            out.position_values.push(
                positions[start..start + count.position_keys]
                    .iter()
                    .map(|value| [value[0], value[1], value[2], 0.0])
                    .collect(),
            );
            out.rotation_times
                .push(rotation_times[start..start + count.rotation_keys].to_vec());
            out.rotation_values
                .push(rotations[start..start + count.rotation_keys].to_vec());
        }
        out
    }

    /// Worst decode error against every original key: centimeters and degrees.
    fn worst_errors(clip: &ImportedClip, decoded: &Decoded) -> (f32, f32) {
        let mut worst_cm = 0.0f32;
        let mut worst_deg = 0.0f32;
        for (joint, track) in clip.positions.tracks.iter().enumerate() {
            for (key, time) in track.times.iter().enumerate() {
                let original = track.values[key];
                let guess = sample_keys(
                    Plane::Position,
                    &decoded.position_times[joint],
                    &decoded.position_values[joint],
                    *time,
                );
                let error = plane_distance(
                    Plane::Position,
                    [original[0], original[1], original[2], 0.0],
                    guess,
                ) * CM_PER_M;
                worst_cm = worst_cm.max(error);
            }
        }
        for (joint, track) in clip.rotations.iter().enumerate() {
            for (key, time) in track.times.iter().enumerate() {
                let guess = sample_keys(
                    Plane::Rotation,
                    &decoded.rotation_times[joint],
                    &decoded.rotation_values[joint],
                    *time,
                );
                let error = plane_distance(Plane::Rotation, track.values[key], guess);
                worst_deg = worst_deg.max(error);
            }
        }
        (worst_cm, worst_deg)
    }

    #[test]
    fn compress_shrinks_a_smooth_clip_five_to_twenty_times() {
        let clip = smooth_clip();
        let compressed = compress_clip(&clip, &CompressBudget::default()).expect("cook");
        let ratio = compression_ratio(&clip, &compressed);
        assert!(
            (5.0..=20.0).contains(&ratio),
            "compression ratio {ratio} outside 5..=20 (raw {}, packed {})",
            raw_clip_bytes(&clip),
            compressed.byte_len()
        );
        assert!(compressed.byte_len() < raw_clip_bytes(&clip));
    }

    #[test]
    fn decode_stays_within_cm_and_degrees() {
        let clip = smooth_clip();
        let compressed = compress_clip(&clip, &CompressBudget::default()).expect("cook");
        let decoded = decode_all(&compressed, KEYS);
        let (worst_cm, worst_deg) = worst_errors(&clip, &decoded);
        assert!(worst_cm <= DEFAULT_POSITION_CM, "position {worst_cm} cm");
        assert!(
            worst_deg <= DEFAULT_ROTATION_DEG,
            "rotation {worst_deg} deg"
        );
        let header = compressed.header().expect("header");
        assert!(header.max_error_cm <= DEFAULT_POSITION_CM);
        assert!(header.max_error_deg <= DEFAULT_ROTATION_DEG);
    }

    #[test]
    fn decode_allocates_nothing() {
        let clip = smooth_clip();
        let compressed = compress_clip(&clip, &CompressBudget::default()).expect("cook");
        let joints = compressed.header().expect("header").joints;
        let slots = required_slots(joints, KEYS);
        let mut position_times = vec![0.0f32; slots];
        let mut positions = vec![[0.0f32; 3]; slots];
        let mut rotation_times = vec![0.0f32; slots];
        let mut rotations = vec![[0.0f32; 4]; slots];
        let mut counts = vec![DecodedCounts::default(); joints];
        let mut buffers = ClipBuffers {
            max_keys: KEYS,
            position_times: &mut position_times,
            positions: &mut positions,
            rotation_times: &mut rotation_times,
            rotations: &mut rotations,
        };
        let (result, heap) = engine_core::alloc_probe::measure(|| {
            decode_clip(&compressed, &mut buffers, &mut counts)
        });
        assert!(result.is_ok(), "decode must succeed");
        assert_eq!(heap.calls, 0, "decode allocates nothing");
    }

    #[test]
    fn cook_twice_is_identical() {
        let clip = smooth_clip();
        let budget = CompressBudget::default();
        let first = compress_clip(&clip, &budget).expect("first cook");
        let second = compress_clip(&clip, &budget).expect("second cook");
        assert_eq!(first.as_bytes(), second.as_bytes());
        let reopened = CompressedClip::from_bytes(first.as_bytes().to_vec()).expect("reopen");
        assert_eq!(reopened.as_bytes(), first.as_bytes());
        let left = decode_all(&first, KEYS);
        let right = decode_all(&reopened, KEYS);
        assert_eq!(left.position_times, right.position_times);
        assert_eq!(left.rotation_values, right.rotation_values);
    }

    #[test]
    fn truncated_payload_is_refused_not_panicking() {
        let clip = smooth_clip();
        let compressed = compress_clip(&clip, &CompressBudget::default()).expect("cook");
        let mut bytes = compressed.as_bytes().to_vec();
        let _ = bytes.pop();
        let truncated = CompressedClip::from_bytes(bytes).expect("header still reads");
        let joints = truncated.header().expect("header").joints;
        let slots = required_slots(joints, KEYS);
        let mut position_times = vec![0.0f32; slots];
        let mut positions = vec![[0.0f32; 3]; slots];
        let mut rotation_times = vec![0.0f32; slots];
        let mut rotations = vec![[0.0f32; 4]; slots];
        let mut counts = vec![DecodedCounts::default(); joints];
        let mut buffers = ClipBuffers {
            max_keys: KEYS,
            position_times: &mut position_times,
            positions: &mut positions,
            rotation_times: &mut rotation_times,
            rotations: &mut rotations,
        };
        assert_eq!(
            decode_clip(&truncated, &mut buffers, &mut counts),
            Err(AnimCompressError::BadPayload)
        );
        assert_eq!(
            CompressedClip::from_bytes(vec![0u8; HEADER_LEN]),
            Err(AnimCompressError::BadPayload)
        );
    }
}
