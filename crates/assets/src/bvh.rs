//! bvh: BVH motion intake. A hand-written parser for the text format (no
//! dependency, no `unsafe`) plus the retarget seam that lands one file's motion
//! on an engine skeleton as a clip the anim crate plays.
//!
//! Consumer: the offline motion bake and any intake that reads a free mocap or
//! text-to-motion BVH file. Data owner: `engine-assets`; the clip types belong
//! to `engine-anim`. Input units are the format's own: meters in `OFFSET` and
//! in translation channels, degrees in rotation channels. Output units are the
//! anim contract's: meters, radians, seconds.
//!
//! Format rules read (assimp's BVH loader is the spec only, no donor bytes):
//! one `HIERARCHY` with a single `ROOT`, nested `JOINT` blocks, an `End Site`
//! leaf that carries no channels and becomes no joint, `OFFSET x y z`,
//! `CHANNELS n <names>`, then `MOTION` with `Frames:` and `Frame Time:` and one
//! number per channel per frame in joint order.
//!
//! Evaluation: a joint's local position is its `OFFSET` plus its translation
//! channels, and its local rotation is the channel-order product of the Euler
//! rotations. BVH applies each written rotation about the already-turned axes,
//! so `Zrotation Xrotation Yrotation` composes `Rz * Rx * Ry`. That is exactly
//! the anim contract's [`LocalPose`](engine_anim::contract::LocalPose): one offset in the parent frame, composed
//! parent-first by `skinning_matrices`.
//!
//! Retarget: a source joint name finds the anim contract's
//! [`BoneRole`](engine_anim::contract::BoneRole) table,
//! so `Hips`, `pelvis` and `mixamorig:Hips` all land on the same bone. A
//! translation key keeps its bind-relative offset scaled by the bind
//! bone-length ratio, and a rotation key carries over as authored because BVH
//! has no rest rotations. Source bones with no role and required roles with no
//! source bone are listed in the [`BvhRetarget`](crate::bvh::BvhRetarget) receipt, never dropped
//! silently.
//!
//! Determinism: f32 `+ - * /`, `sqrt`, and `sin`/`cos` for the Euler decode
//! only, over stable file order. The same bytes parse to the same clip and the
//! same digest twice.

use engine_anim::contract::{
    bone_role_for_name, validate_skeleton, AnimError, BoneMapping, BoneRole, LocalPose, Skeleton,
    FULL_COVERAGE_PERMILLE, MAX_JOINTS, REQUIRED_BONE_COUNT, REQUIRED_BONE_ROLES,
};
use engine_anim::pose::{sample_pose_with_rots, QuatTrack};
use engine_anim::{Clip, Track};
use engine_core::{quat_mul, Fnv64};
use std::fmt;

/// Largest frame count accepted in one file; a longer cut is a corpus job.
pub const MAX_FRAMES: usize = 100_000;

/// Largest channel count accepted for one joint: BVH defines six.
const MAX_CHANNELS: usize = 6;

/// Bone lengths at or below this (meters) retarget unscaled, never divide.
const BONE_FLOOR: f32 = 1e-6;

/// Match confidence for a source bone that only shares a role with the target.
const ALIAS_CONFIDENCE: u16 = 900;

/// The rotation of a joint with no rotation channels.
const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// One BVH channel: a translation or a rotation about one axis, in written order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BvhChannel {
    /// Translation along X, meters.
    XPosition,
    /// Translation along Y, meters.
    YPosition,
    /// Translation along Z, meters.
    ZPosition,
    /// Rotation about X, degrees.
    XRotation,
    /// Rotation about Y, degrees.
    YRotation,
    /// Rotation about Z, degrees.
    ZRotation,
}

impl BvhChannel {
    /// Channel by its written name, `None` for an unknown word.
    fn parse(name: &str) -> Option<Self> {
        match name {
            "Xposition" => Some(Self::XPosition),
            "Yposition" => Some(Self::YPosition),
            "Zposition" => Some(Self::ZPosition),
            "Xrotation" => Some(Self::XRotation),
            "Yrotation" => Some(Self::YRotation),
            "Zrotation" => Some(Self::ZRotation),
            _ => None,
        }
    }

    /// Axis a translation channel adds to, `None` for a rotation channel.
    fn position_axis(self) -> Option<usize> {
        match self {
            Self::XPosition => Some(0),
            Self::YPosition => Some(1),
            Self::ZPosition => Some(2),
            Self::XRotation | Self::YRotation | Self::ZRotation => None,
        }
    }
}

/// One BVH joint: name, parent index, rest offset and its channels in order.
#[derive(Clone, PartialEq, Debug)]
pub struct BvhJoint {
    /// Name as authored (`Hips`, `LeftUpLeg`).
    pub name: String,
    /// Parent joint index, `None` for the root.
    pub parent: Option<usize>,
    /// Rest offset in the parent frame, meters.
    pub offset: [f32; 3],
    /// Channels in the order the file writes their values.
    pub channels: Vec<BvhChannel>,
}

/// One parsed BVH file: its joint tree plus one local pose per frame.
#[derive(Clone, PartialEq, Debug)]
pub struct BvhMotion {
    /// Joints in file order, parents before children.
    pub joints: Vec<BvhJoint>,
    /// Seconds between frames (`Frame Time`).
    pub frame_time: f32,
    /// One local pose per frame, joint order as [`Self::joints`].
    pub frames: Vec<LocalPose>,
}

impl BvhMotion {
    /// Clip length in seconds: the last frame starts this long after the first.
    pub fn duration(&self) -> f32 {
        match self.frames.len() {
            0 | 1 => 0.0,
            len => frame_times(self.frame_time, len)
                .last()
                .copied()
                .unwrap_or(0.0),
        }
    }
}

/// Why a BVH intake refused its input; every malformed file maps to one variant.
#[derive(Clone, PartialEq, Debug)]
pub enum BvhError {
    /// A token is not the keyword the format wants here; byte carried.
    BadToken {
        /// Byte offset the token starts at.
        at: usize,
    },
    /// A number is missing or does not parse as `f32`; byte carried.
    BadNumber {
        /// Byte offset the token starts at.
        at: usize,
    },
    /// A joint's channel count is not in `0..=MAX_CHANNELS`.
    BadChannels {
        /// Joint index in file order.
        joint: usize,
    },
    /// A channel name is not one of the six BVH channels.
    BadChannel {
        /// Joint index in file order.
        joint: usize,
    },
    /// More joints than the anim contract allows.
    TooManyJoints {
        /// Offending joint count.
        count: usize,
    },
    /// More frames than [`MAX_FRAMES`].
    TooManyFrames {
        /// Offending frame count.
        count: usize,
    },
    /// The motion block declares no frames.
    NoFrames,
    /// `Frame Time` is zero, negative or not finite.
    BadFrameTime,
    /// A frame ended before its declared channel width was read.
    FrameWidth {
        /// Frame index in file order.
        frame: usize,
        /// Channel values the header asks for.
        expected: usize,
        /// Values read before the file ended.
        got: usize,
    },
    /// A non-finite float in the named lane.
    NonFinite(&'static str),
    /// The target bind row count differs from the target joint count.
    BindMismatch {
        /// Target joint count.
        expected: usize,
        /// Bind row count.
        got: usize,
    },
    /// The target skeleton failed anim-contract validation.
    Target(AnimError),
}

impl fmt::Display for BvhError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadToken { at } => write!(f, "bvh bad token at byte {at}"),
            Self::BadNumber { at } => write!(f, "bvh bad number at byte {at}"),
            Self::BadChannels { joint } => write!(f, "bvh bad channel count on joint {joint}"),
            Self::BadChannel { joint } => write!(f, "bvh unknown channel on joint {joint}"),
            Self::TooManyJoints { count } => write!(f, "bvh too many joints: {count}"),
            Self::TooManyFrames { count } => write!(f, "bvh too many frames: {count}"),
            Self::NoFrames => write!(f, "bvh motion declares no frames"),
            Self::BadFrameTime => write!(f, "bvh frame time must be positive"),
            Self::FrameWidth {
                frame,
                expected,
                got,
            } => write!(f, "bvh frame {frame} wants {expected} values, read {got}"),
            Self::NonFinite(lane) => write!(f, "bvh non-finite {lane}"),
            Self::BindMismatch { expected, got } => {
                write!(f, "bvh wants {expected} bind rows, found {got}")
            }
            Self::Target(error) => write!(f, "bvh target skeleton: {error:?}"),
        }
    }
}

impl std::error::Error for BvhError {}

// ---------------------------------------------------------------------------
// Token cursor and the HIERARCHY / MOTION reader.
// ---------------------------------------------------------------------------

/// Whitespace token cursor over the file text; every refusal names its byte.
struct Tokens<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Tokens<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, pos: 0 }
    }

    /// Byte offset the next token would start at.
    fn at(&self) -> usize {
        self.pos
    }

    /// Next whitespace-separated token, `None` at the end of the file.
    fn next(&mut self) -> Option<&'a str> {
        let bytes = self.text.as_bytes();
        while self.pos < bytes.len() && bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        let start = self.pos;
        while self.pos < bytes.len() && !bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        if start == self.pos {
            None
        } else {
            self.text.get(start..self.pos)
        }
    }

    /// The next token without consuming it.
    fn peek(&self) -> Option<&'a str> {
        let mut probe = Self {
            text: self.text,
            pos: self.pos,
        };
        probe.next()
    }

    /// Next token as a finite `f32`.
    fn number(&mut self) -> Result<f32, BvhError> {
        let at = self.at();
        let token = self.next().ok_or(BvhError::BadNumber { at })?;
        let value = token
            .parse::<f32>()
            .map_err(|_| BvhError::BadNumber { at })?;
        if value.is_finite() {
            Ok(value)
        } else {
            Err(BvhError::NonFinite("channel"))
        }
    }
}

/// Consume the exact keyword the format wants next.
fn expect(toks: &mut Tokens<'_>, keyword: &str) -> Result<(), BvhError> {
    let at = toks.at();
    match toks.next() {
        Some(word) if word == keyword => Ok(()),
        _ => Err(BvhError::BadToken { at }),
    }
}

/// Read `OFFSET x y z`.
fn read_offset(toks: &mut Tokens<'_>) -> Result<[f32; 3], BvhError> {
    Ok([toks.number()?, toks.number()?, toks.number()?])
}

/// Open one `ROOT` or `JOINT` block: name, `{`, then the joint row.
fn push_joint(
    toks: &mut Tokens<'_>,
    joints: &mut Vec<BvhJoint>,
    open: &mut Vec<usize>,
    parent: Option<usize>,
) -> Result<(), BvhError> {
    let at = toks.at();
    let name = toks.next().ok_or(BvhError::BadToken { at })?.to_string();
    expect(toks, "{")?;
    if joints.len() >= MAX_JOINTS {
        return Err(BvhError::TooManyJoints {
            count: joints.len() + 1,
        });
    }
    joints.push(BvhJoint {
        name,
        parent,
        offset: [0.0; 3],
        channels: Vec::new(),
    });
    open.push(joints.len() - 1);
    Ok(())
}

/// Read `CHANNELS n <name>...` into the joint that is open now.
fn read_channels(
    toks: &mut Tokens<'_>,
    joints: &mut [BvhJoint],
    open: &[usize],
) -> Result<(), BvhError> {
    let at = toks.at();
    let joint = *open.last().ok_or(BvhError::BadToken { at })?;
    let count: usize = toks
        .next()
        .and_then(|token| token.parse().ok())
        .ok_or(BvhError::BadToken { at })?;
    if count > MAX_CHANNELS {
        return Err(BvhError::BadChannels { joint });
    }
    let mut channels = Vec::with_capacity(count);
    while channels.len() < count {
        let word = toks.peek().unwrap_or("");
        let channel = BvhChannel::parse(word).ok_or(BvhError::BadChannel { joint })?;
        let _ = toks.next();
        channels.push(channel);
    }
    joints[joint].channels = channels;
    Ok(())
}

/// Skip an `End Site { OFFSET x y z }` leaf: it carries no channels and its tip
/// offset is a display length, not a joint.
fn skip_end_site(toks: &mut Tokens<'_>) -> Result<(), BvhError> {
    expect(toks, "{")?;
    loop {
        let at = toks.at();
        match toks.next().ok_or(BvhError::BadToken { at })? {
            "}" => return Ok(()),
            "OFFSET" => {
                let _ = read_offset(toks)?;
            }
            _ => return Err(BvhError::BadToken { at }),
        }
    }
}

/// Read the joint tree, up to the matching brace of the root.
fn read_hierarchy(toks: &mut Tokens<'_>) -> Result<Vec<BvhJoint>, BvhError> {
    expect(toks, "ROOT")?;
    let mut joints: Vec<BvhJoint> = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    push_joint(toks, &mut joints, &mut open, None)?;
    loop {
        let at = toks.at();
        match toks.next().ok_or(BvhError::BadToken { at })? {
            "}" => {
                open.pop();
                if open.is_empty() {
                    return Ok(joints);
                }
            }
            "OFFSET" => {
                let joint = *open.last().ok_or(BvhError::BadToken { at })?;
                joints[joint].offset = read_offset(toks)?;
            }
            "CHANNELS" => read_channels(toks, &mut joints, &open)?,
            "JOINT" => {
                let parent = *open.last().ok_or(BvhError::BadToken { at })?;
                push_joint(toks, &mut joints, &mut open, Some(parent))?;
            }
            "End" => {
                expect(toks, "Site")?;
                skip_end_site(toks)?;
            }
            _ => return Err(BvhError::BadToken { at }),
        }
    }
}

/// Read `Frames: n`.
fn read_frame_count(toks: &mut Tokens<'_>) -> Result<usize, BvhError> {
    let at = toks.at();
    match toks.next().ok_or(BvhError::BadToken { at })? {
        "Frames:" | "Frames" => {}
        _ => return Err(BvhError::BadToken { at }),
    }
    toks.next()
        .and_then(|token| token.parse().ok())
        .ok_or(BvhError::BadNumber { at })
}

/// Read `Frame Time: seconds`; a non-positive step is refused.
fn read_frame_time(toks: &mut Tokens<'_>) -> Result<f32, BvhError> {
    let at = toks.at();
    match toks.next().ok_or(BvhError::BadToken { at })? {
        "Frame Time:" => {}
        "Frame" => expect(toks, "Time:")?,
        _ => return Err(BvhError::BadToken { at }),
    }
    let time = toks.number()?;
    if time <= 0.0 {
        return Err(BvhError::BadFrameTime);
    }
    Ok(time)
}

/// One joint's local pose from the channel values of one frame.
fn channel_pose(channels: &[BvhChannel], values: &[f32], offset: [f32; 3]) -> ([f32; 3], [f32; 4]) {
    let mut position = offset;
    let mut rotation = IDENTITY;
    for (channel, value) in channels.iter().zip(values.iter()) {
        match channel.position_axis() {
            Some(axis) => position[axis] += *value,
            None => rotation = quat_mul(rotation, euler_quat(*channel, *value)),
        }
    }
    (position, rotation)
}

/// Unit quaternion for one Euler channel, degrees in.
fn euler_quat(channel: BvhChannel, degrees: f32) -> [f32; 4] {
    let half = 0.5 * degrees.to_radians();
    let (sin, cos) = (half.sin(), half.cos());
    match channel {
        BvhChannel::XRotation => [sin, 0.0, 0.0, cos],
        BvhChannel::YRotation => [0.0, sin, 0.0, cos],
        BvhChannel::ZRotation => [0.0, 0.0, sin, cos],
        BvhChannel::XPosition | BvhChannel::YPosition | BvhChannel::ZPosition => IDENTITY,
    }
}

/// Read every frame of the motion block, in file order.
fn read_frames(
    toks: &mut Tokens<'_>,
    joints: &[BvhJoint],
    count: usize,
) -> Result<Vec<LocalPose>, BvhError> {
    if count == 0 {
        return Err(BvhError::NoFrames);
    }
    if count > MAX_FRAMES {
        return Err(BvhError::TooManyFrames { count });
    }
    let width: usize = joints.iter().map(|joint| joint.channels.len()).sum();
    let mut frames = Vec::with_capacity(count);
    for frame in 0..count {
        let mut positions = Vec::with_capacity(joints.len());
        let mut rotations = Vec::with_capacity(joints.len());
        let mut read = 0usize;
        for joint in joints {
            let mut values = Vec::with_capacity(joint.channels.len());
            while values.len() < joint.channels.len() {
                if toks.peek().is_none() {
                    return Err(BvhError::FrameWidth {
                        frame,
                        expected: width,
                        got: read,
                    });
                }
                values.push(toks.number()?);
            }
            read += values.len();
            let (position, rotation) = channel_pose(&joint.channels, &values, joint.offset);
            positions.push(position);
            rotations.push(rotation);
        }
        frames.push(LocalPose {
            positions,
            rotations,
        });
    }
    Ok(frames)
}

/// Parse one BVH file: `HIERARCHY`, then `MOTION`, then every frame.
///
/// Refuses a malformed header, an unknown channel, a truncated frame and any
/// trailing token; the tree and the frames otherwise come out in file order.
pub fn parse_bvh(text: &str) -> Result<BvhMotion, BvhError> {
    let mut toks = Tokens::new(text);
    expect(&mut toks, "HIERARCHY")?;
    let joints = read_hierarchy(&mut toks)?;
    expect(&mut toks, "MOTION")?;
    let count = read_frame_count(&mut toks)?;
    let frame_time = read_frame_time(&mut toks)?;
    let frames = read_frames(&mut toks, &joints, count)?;
    let at = toks.at();
    if toks.peek().is_some() {
        return Err(BvhError::BadToken { at });
    }
    Ok(BvhMotion {
        joints,
        frame_time,
        frames,
    })
}

// ---------------------------------------------------------------------------
// Retarget: source joint names to the anim contract's bone roles.
// ---------------------------------------------------------------------------

/// A clip on the target skeleton: one position track plus one rotation track
/// per target joint, in target order.
#[derive(Clone, Debug)]
pub struct BvhClip {
    /// Clip name carried from the intake (`walk_seed` for a BVH file).
    pub name: String,
    /// Clip length in seconds.
    pub duration: f32,
    /// Position tracks in the anim crate's clip form.
    pub positions: Clip,
    /// Rotation tracks, one per target joint.
    pub rotations: Vec<QuatTrack>,
}

impl BvhClip {
    /// Sample one pose at `time` seconds on `skeleton` (the target rig).
    pub fn sample(&self, skeleton: &Skeleton, time: f32) -> Result<LocalPose, AnimError> {
        sample_pose_with_rots(skeleton, &self.positions, &self.rotations, time)
    }

    /// FNV-1a64 over name, duration, times and every key of every lane: the
    /// bytes a cook-twice proof compares.
    pub fn digest(&self) -> u64 {
        let mut hash = Fnv64::new();
        hash.write_str(&self.name);
        hash.write_f32(self.duration);
        hash.write_len(self.positions.tracks.len());
        for track in &self.positions.tracks {
            write_lane(&mut hash, &track.times, &track.values);
        }
        hash.write_len(self.rotations.len());
        for track in &self.rotations {
            write_lane(&mut hash, &track.times, &track.values);
        }
        hash.finish()
    }
}

impl PartialEq for BvhClip {
    /// Structural equality: the same lanes, times and key bits.
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.duration.to_bits() == other.duration.to_bits()
            && self.positions.tracks.len() == other.positions.tracks.len()
            && self
                .positions
                .tracks
                .iter()
                .zip(other.positions.tracks.iter())
                .all(|(left, right)| {
                    same_bits(&left.times, &left.values, &right.times, &right.values)
                })
            && self.rotations.len() == other.rotations.len()
            && self
                .rotations
                .iter()
                .zip(other.rotations.iter())
                .all(|(left, right)| {
                    same_bits(&left.times, &left.values, &right.times, &right.values)
                })
    }
}

/// Hash one lane: key count, times, then every component.
fn write_lane<const N: usize>(hash: &mut Fnv64, times: &[f32], values: &[[f32; N]]) {
    hash.write_len(times.len());
    for time in times {
        hash.write_f32(*time);
    }
    for value in values {
        for component in value {
            hash.write_f32(*component);
        }
    }
}

/// True when two lane lists carry the same times and key bits.
fn same_bits<const N: usize>(
    a_times: &[f32],
    a_values: &[[f32; N]],
    b_times: &[f32],
    b_values: &[[f32; N]],
) -> bool {
    a_times.len() == b_times.len()
        && a_times
            .iter()
            .zip(b_times.iter())
            .all(|(x, y)| x.to_bits() == y.to_bits())
        && a_values.len() == b_values.len()
        && a_values.iter().zip(b_values.iter()).all(|(x, y)| {
            x.iter()
                .zip(y.iter())
                .all(|(p, q)| p.to_bits() == q.to_bits())
        })
}

/// One retargeted clip plus the receipt of what mapped and what did not.
#[derive(Clone, PartialEq, Debug)]
pub struct BvhRetarget {
    /// Clip on the target skeleton.
    pub clip: BvhClip,
    /// One row per mapped target joint, in target order.
    pub mapped: Vec<BoneMapping>,
    /// Source bone names with no known role, in file order.
    pub source_unknown: Vec<String>,
    /// Target bone names with no source joint, in target order.
    pub target_unmapped: Vec<String>,
    /// Required roles the source rig does not carry, in coverage order.
    pub missing_roles: Vec<BoneRole>,
    /// Covered required roles in per-mille (`1000` equals all of them).
    pub coverage_permille: u16,
}

/// One target lane: the source joint that drives it plus the bind scale.
#[derive(Clone, Copy, Debug)]
struct Lane {
    origin: usize,
    scale: f32,
}

/// Role of a source or target bone name, with one rig prefix (`mixamorig:`)
/// stripped: BVH files carry the DCC's own prefix, the role table does not.
fn role_of(name: &str) -> Option<BoneRole> {
    if let Ok(role) = bone_role_for_name(name) {
        return Some(role);
    }
    let tail = name.rsplit(':').next().unwrap_or(name);
    bone_role_for_name(tail).ok()
}

/// First source joint per required role, indexed by [`BoneRole::index`].
fn first_per_role(source: &[Option<BoneRole>]) -> [Option<usize>; REQUIRED_BONE_COUNT] {
    let mut slots: [Option<usize>; REQUIRED_BONE_COUNT] = [None; REQUIRED_BONE_COUNT];
    for (index, role) in source.iter().enumerate() {
        if let Some(role) = role {
            let slot = &mut slots[role.index()];
            if slot.is_none() {
                *slot = Some(index);
            }
        }
    }
    slots
}

/// Length of one bind offset in meters.
fn length(value: [f32; 3]) -> f32 {
    (value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt()
}

/// Bind bone length: the distance from a target joint's bind offset to its
/// parent's; the root measures zero so its travel carries over 1:1.
fn target_bone_len(skeleton: &Skeleton, bind: &[[f32; 3]], joint: usize) -> f32 {
    let Some(parent) = skeleton.joints.get(joint).and_then(|row| row.parent) else {
        return 0.0;
    };
    let (tip, base) = (bind[joint], bind[parent]);
    length([tip[0] - base[0], tip[1] - base[1], tip[2] - base[2]])
}

/// Per-lane translation scale: the target/source bind bone-length ratio, with
/// degenerate bones left at 1.0 instead of dividing.
fn bind_scale(
    motion: &BvhMotion,
    skeleton: &Skeleton,
    bind: &[[f32; 3]],
    joint: usize,
    origin: usize,
) -> f32 {
    if skeleton.joints[joint].parent.is_none() {
        return 1.0;
    }
    let from = length(motion.joints[origin].offset);
    let to = target_bone_len(skeleton, bind, joint);
    if from <= BONE_FLOOR || to <= BONE_FLOOR {
        1.0
    } else {
        to / from
    }
}

/// Clip times in seconds, one per frame, built by accumulation so a frame key
/// sits exactly where the sampler interpolates it.
fn frame_times(frame_time: f32, frames: usize) -> Vec<f32> {
    let mut times = Vec::with_capacity(frames);
    let mut time = 0.0f32;
    for _ in 0..frames {
        times.push(time);
        time += frame_time;
    }
    times
}

/// Covered required roles in per-mille, floored.
fn coverage_permille(covered: usize) -> u16 {
    let full = usize::from(FULL_COVERAGE_PERMILLE);
    u16::try_from(covered * full / REQUIRED_BONE_COUNT).unwrap_or(FULL_COVERAGE_PERMILLE)
}

/// Build one position lane plus one rotation lane per target joint: mapped
/// joints follow their source, unmapped ones hold the target bind and identity.
fn clip_tracks(
    motion: &BvhMotion,
    bind: &[[f32; 3]],
    lanes: &[Option<Lane>],
    times: &[f32],
) -> (Clip, Vec<QuatTrack>) {
    let mut positions = Vec::with_capacity(lanes.len());
    let mut rotations = Vec::with_capacity(lanes.len());
    for (slot, lane) in lanes.iter().enumerate() {
        let held = motion.frames.len();
        let Some(lane) = lane else {
            positions.push(Track {
                times: times.to_vec(),
                values: vec![bind[slot]; held],
            });
            rotations.push(QuatTrack {
                times: times.to_vec(),
                values: vec![IDENTITY; held],
            });
            continue;
        };
        let base = bind[slot];
        let source_base = motion.joints[lane.origin].offset;
        let values: Vec<[f32; 3]> = motion
            .frames
            .iter()
            .map(|frame| {
                let source = frame.positions[lane.origin];
                [
                    base[0] + lane.scale * (source[0] - source_base[0]),
                    base[1] + lane.scale * (source[1] - source_base[1]),
                    base[2] + lane.scale * (source[2] - source_base[2]),
                ]
            })
            .collect();
        positions.push(Track {
            times: times.to_vec(),
            values,
        });
        rotations.push(QuatTrack {
            times: times.to_vec(),
            values: motion
                .frames
                .iter()
                .map(|frame| frame.rotations[lane.origin])
                .collect(),
        });
    }
    (Clip { tracks: positions }, rotations)
}

/// Retarget one parsed BVH motion onto `target`, whose bind offsets are
/// `target_bind` (one local offset per joint, meters).
///
/// A target joint takes the first source joint with the same role; its
/// translations keep the bind-relative offset scaled by the bone-length ratio
/// and its rotations carry over as authored. A target joint with no source
/// holds `target_bind` and identity on every frame. The returned receipt names
/// the source bones with no role, the target bones with no source bone and the
/// required roles the source rig lacks; it never refuses a partial rig, so the
/// caller decides whether the coverage is playable.
pub fn retarget_motion(
    motion: &BvhMotion,
    target: &Skeleton,
    target_bind: &[[f32; 3]],
    name: &str,
) -> Result<BvhRetarget, BvhError> {
    validate_skeleton(target).map_err(BvhError::Target)?;
    if target_bind.len() != target.len() {
        return Err(BvhError::BindMismatch {
            expected: target.len(),
            got: target_bind.len(),
        });
    }
    if target_bind.iter().flatten().any(|value| !value.is_finite()) {
        return Err(BvhError::NonFinite("target_bind"));
    }
    let source: Vec<Option<BoneRole>> =
        motion.joints.iter().map(|row| role_of(&row.name)).collect();
    let slots = first_per_role(&source);
    let mut lanes: Vec<Option<Lane>> = Vec::with_capacity(target.len());
    let mut mapped = Vec::new();
    let mut target_unmapped = Vec::new();
    for (slot, row) in target.joints.iter().enumerate() {
        let role = role_of(&row.name);
        match role.and_then(|role| slots[role.index()].map(|origin| (role, origin))) {
            Some((role, origin)) => {
                let source_row = &motion.joints[origin];
                let (confidence_permille, method) = if source_row.name == row.name {
                    (FULL_COVERAGE_PERMILLE, "exact")
                } else {
                    (ALIAS_CONFIDENCE, "alias")
                };
                mapped.push(BoneMapping {
                    source: source_row.name.clone(),
                    role,
                    confidence_permille,
                    method,
                });
                lanes.push(Some(Lane {
                    origin,
                    scale: bind_scale(motion, target, target_bind, slot, origin),
                }));
            }
            None => {
                lanes.push(None);
                target_unmapped.push(row.name.clone());
            }
        }
    }
    let mut source_unknown = Vec::new();
    for (row, role) in motion.joints.iter().zip(source.iter()) {
        if role.is_none() {
            source_unknown.push(row.name.clone());
        }
    }
    let mut missing_roles = Vec::new();
    let mut covered = 0usize;
    for role in REQUIRED_BONE_ROLES {
        if slots[role.index()].is_some() {
            covered += 1;
        } else {
            missing_roles.push(role);
        }
    }
    let times = frame_times(motion.frame_time, motion.frames.len());
    let duration = times.last().copied().unwrap_or(0.0);
    let (positions, rotations) = clip_tracks(motion, target_bind, &lanes, &times);
    let bounded: String = name
        .chars()
        .take(engine_anim::contract::NAME_MAX_LEN)
        .collect();
    Ok(BvhRetarget {
        clip: BvhClip {
            name: bounded,
            duration,
            positions,
            rotations,
        },
        mapped,
        source_unknown,
        target_unmapped,
        missing_roles,
        coverage_permille: coverage_permille(covered),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_anim::clip::TICK_DT;
    use engine_anim::contract::{skinning_matrices, validate_pose, Joint};

    /// The hand-written walk seed: 17 joints, four frames, no donor bytes.
    const WALK_SEED: &str = include_str!("../../../fixtures/bvh/walk_seed.bvh");

    /// A partial rig: the pelvis and one spine joint, plus a tail the engine
    /// role table does not know.
    const TAIL_RIG: &str = "\
HIERARCHY
ROOT Hips
{
    OFFSET 0.00 0.95 0.00
    CHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation
    JOINT Spine
    {
        OFFSET 0.00 0.12 0.00
        CHANNELS 3 Zrotation Xrotation Yrotation
        JOINT Tail_01
        {
            OFFSET 0.00 0.10 0.00
            CHANNELS 3 Zrotation Xrotation Yrotation
        }
    }
}
MOTION
Frames: 2
Frame Time: 0.1
0.00 0.00 0.00 0.00 0.00 0.00
0.00 0.00 0.00
0.00 0.00 10.00
0.00 0.00 0.00 0.00 0.00 0.00
0.00 0.00 0.00
0.00 0.00 -10.00
";

    /// The engine rig in [`REQUIRED_BONE_ROLES`] order: engine bone names, one
    /// local bind offset per joint.
    fn engine_rig() -> (Skeleton, Vec<[f32; 3]>) {
        let rows: [(&str, Option<usize>, [f32; 3]); REQUIRED_BONE_COUNT] = [
            ("root", None, [0.0, 0.0, 0.0]),
            ("pelvis", Some(0), [0.0, 0.95, 0.0]),
            ("spine_lower", Some(1), [0.0, 0.12, 0.0]),
            ("spine_upper", Some(2), [0.0, 0.12, 0.0]),
            ("head", Some(3), [0.0, 0.20, 0.0]),
            ("upper_arm_l", Some(3), [0.16, 0.12, 0.0]),
            ("upper_arm_r", Some(3), [-0.16, 0.12, 0.0]),
            ("lower_arm_l", Some(5), [0.26, 0.0, 0.0]),
            ("lower_arm_r", Some(6), [-0.26, 0.0, 0.0]),
            ("hand_l", Some(7), [0.24, 0.0, 0.0]),
            ("hand_r", Some(8), [-0.24, 0.0, 0.0]),
            ("upper_leg_l", Some(1), [0.09, -0.05, 0.0]),
            ("upper_leg_r", Some(1), [-0.09, -0.05, 0.0]),
            ("lower_leg_l", Some(11), [0.0, -0.42, 0.0]),
            ("lower_leg_r", Some(12), [0.0, -0.42, 0.0]),
            ("foot_l", Some(13), [0.0, -0.42, 0.0]),
            ("foot_r", Some(14), [0.0, -0.42, 0.0]),
        ];
        let joints = rows
            .iter()
            .map(|(name, parent, _)| Joint {
                name: (*name).to_string(),
                parent: *parent,
            })
            .collect();
        let bind = rows.iter().map(|(_, _, offset)| *offset).collect();
        (Skeleton { joints }, bind)
    }

    /// One-frame document with the named channels: the smallest shape that
    /// proves a channel rule.
    fn one_joint(channels: &str, values: &str) -> String {
        let count = channels.split_whitespace().count();
        format!(
            "HIERARCHY\nROOT root\n{{\n    OFFSET 0 0 0\n    CHANNELS {count} {channels}\n}}\n\
             MOTION\nFrames: 1\nFrame Time: 0.1\n{values}\n"
        )
    }

    /// Rotation about one axis (0 = X, 1 = Y, 2 = Z) by degrees, the oracle the
    /// parser's Euler decode must match.
    fn axis_quat(axis: usize, degrees: f32) -> [f32; 4] {
        let half = 0.5 * degrees.to_radians();
        let (sin, cos) = (half.sin(), half.cos());
        match axis {
            0 => [sin, 0.0, 0.0, cos],
            1 => [0.0, sin, 0.0, cos],
            _ => [0.0, 0.0, sin, cos],
        }
    }

    #[test]
    fn parse_reads_hierarchy_channels_and_frames() {
        let motion = parse_bvh(WALK_SEED).expect("the fixture parses");
        assert_eq!(motion.joints.len(), 17, "End Site leaves become no joint");
        assert_eq!(motion.joints[0].name, "Root");
        assert_eq!(motion.joints[0].parent, None);
        assert_eq!(motion.joints[1].name, "Hips");
        assert_eq!(motion.joints[1].parent, Some(0));
        assert_eq!(motion.joints[1].offset, [0.0, 0.95, 0.0]);
        assert_eq!(motion.joints[0].channels.len(), 6);
        assert_eq!(
            motion.joints[2].channels,
            vec![
                BvhChannel::ZRotation,
                BvhChannel::XRotation,
                BvhChannel::YRotation
            ]
        );
        assert_eq!(motion.frame_time, 0.0333333);
        assert_eq!(motion.frames.len(), 4);
        // Frame 1: the root travels 5 cm, a translation channel adds to the
        // joint's OFFSET, and the hips yaw 2 degrees.
        assert_eq!(motion.frames[1].positions[0], [0.05, 0.0, 0.0]);
        assert_eq!(motion.frames[1].positions[1], [0.0, 0.96, 0.0]);
        let yaw = motion.frames[1].rotations[1];
        let half = 1.0f32.to_radians();
        assert!((yaw[1] - half.sin()).abs() < 1e-6, "{yaw:?}");
        assert!((yaw[3] - half.cos()).abs() < 1e-6, "{yaw:?}");
        assert!(
            (motion.duration() - 0.1).abs() < 1e-5,
            "{}",
            motion.duration()
        );

        // Refusals: no header, an unknown channel, a frame that ends early.
        assert_eq!(parse_bvh(""), Err(BvhError::BadToken { at: 0 }));
        let bad = WALK_SEED.replacen("CHANNELS 3 Zrotation", "CHANNELS 3 Qrotation", 1);
        assert!(
            matches!(parse_bvh(&bad), Err(BvhError::BadChannel { joint: 2 })),
            "an unknown channel is named"
        );
        let short = WALK_SEED
            .rsplit_once("0.00 0.00 0.00\n")
            .map(|(head, _)| head)
            .expect("the fixture ends with a frame line");
        assert!(
            matches!(
                parse_bvh(short),
                Err(BvhError::FrameWidth {
                    frame: 3,
                    expected: 57,
                    got: 54
                })
            ),
            "a truncated frame is named"
        );
    }

    #[test]
    fn channel_order_decides_the_local_rotation() {
        // Written order is applied order: Rz * Rx * Ry, never the reverse.
        let zxy = parse_bvh(&one_joint("Zrotation Xrotation Yrotation", "30 45 60"))
            .expect("the zxy doc parses");
        let got = zxy.frames[0].rotations[0];
        let want = quat_mul(
            quat_mul(axis_quat(2, 30.0), axis_quat(0, 45.0)),
            axis_quat(1, 60.0),
        );
        for lane in 0..4 {
            assert!(
                (got[lane] - want[lane]).abs() < 1e-6,
                "lane {lane}: {got:?} vs {want:?}"
            );
        }
        let yxz = parse_bvh(&one_joint("Yrotation Xrotation Zrotation", "60 45 30"))
            .expect("the yxz doc parses");
        let other = yxz.frames[0].rotations[0];
        assert!(
            got.iter()
                .zip(other.iter())
                .any(|(a, b)| (a - b).abs() > 1e-3),
            "the channel order made no difference: {got:?} {other:?}"
        );
        // Degrees in: 90 degrees about X is a quarter turn.
        let quarter = parse_bvh(&one_joint("Xrotation", "90")).expect("the one-axis doc parses");
        let q = quarter.frames[0].rotations[0];
        let eighth = 45.0f32.to_radians();
        assert!(
            (q[0] - eighth.sin()).abs() < 1e-6 && (q[3] - eighth.cos()).abs() < 1e-6,
            "{q:?}"
        );
        // Translation channels read on their own axes: Z X Y, values 1 2 3.
        let moved = parse_bvh(&one_joint("Zposition Xposition Yposition", "1 2 3"))
            .expect("the position doc parses");
        assert_eq!(moved.frames[0].positions[0], [2.0, 3.0, 1.0]);
    }

    #[test]
    fn retarget_covers_the_humanoid_roles() {
        let motion = parse_bvh(WALK_SEED).expect("the fixture parses");
        let (target, bind) = engine_rig();
        let report =
            retarget_motion(&motion, &target, &bind, "walk_seed").expect("the fixture retargets");
        assert_eq!(report.coverage_permille, FULL_COVERAGE_PERMILLE);
        assert_eq!(report.mapped.len(), REQUIRED_BONE_COUNT);
        assert!(report.missing_roles.is_empty());
        assert!(report.source_unknown.is_empty());
        assert!(report.target_unmapped.is_empty());
        assert_eq!(report.clip.positions.tracks.len(), target.len());
        assert_eq!(report.clip.rotations.len(), target.len());
        assert_eq!(report.clip.name, "walk_seed");
        // Hips finds the pelvis bone through the role table, not by name.
        let pelvis = report
            .mapped
            .iter()
            .find(|row| row.role == BoneRole::Pelvis)
            .expect("the pelvis maps");
        assert_eq!(pelvis.source, "Hips");
        assert_eq!(pelvis.method, "alias");
        assert_eq!(pelvis.confidence_permille, ALIAS_CONFIDENCE);
        // The source thigh and the target thigh are both 0.42 m, so the scale
        // is 1.0 and frame 0 holds the fixture's rest offset.
        assert_eq!(
            report.clip.positions.tracks[11].values[0],
            [0.09, -0.05, 0.0]
        );
        assert_eq!(report.clip.positions.tracks[1].values[0], [0.0, 0.95, 0.0]);
    }

    #[test]
    fn unknown_bones_are_reported_not_dropped() {
        let motion = parse_bvh(TAIL_RIG).expect("the tail rig parses");
        let (mut target, mut bind) = engine_rig();
        target.joints.push(Joint {
            name: "tail_extra".to_string(),
            parent: Some(0),
        });
        bind.push([0.0, 0.1, 0.0]);
        let report = retarget_motion(&motion, &target, &bind, "tail")
            .expect("a partial rig still retargets");
        assert_eq!(report.source_unknown, vec!["Tail_01".to_string()]);
        assert!(
            report.mapped.iter().all(|row| row.source != "Tail_01"),
            "an unknown source bone never drives a target joint"
        );
        assert!(report.mapped.iter().any(|row| row.role == BoneRole::Pelvis));
        assert!(report.missing_roles.contains(&BoneRole::FootLeft));
        assert!(report.target_unmapped.contains(&"foot_l".to_string()));
        assert!(report.target_unmapped.contains(&"tail_extra".to_string()));
        // Two of seventeen roles covered: 2000 / 17 floors to 117 per-mille.
        assert_eq!(report.coverage_permille, 117);
        // A DCC-prefixed source name still finds its role.
        let prefixed =
            parse_bvh(&TAIL_RIG.replace("ROOT Hips", "ROOT mixamorig:Hips")).expect("parses");
        let prefixed_report =
            retarget_motion(&prefixed, &target, &bind, "prefixed").expect("retargets");
        assert!(prefixed_report
            .mapped
            .iter()
            .any(|row| row.role == BoneRole::Pelvis));
    }

    #[test]
    fn bvh_intake_clip_retargets_and_plays_x2_hashed() {
        let (target, bind) = engine_rig();
        let first = retarget_motion(
            &parse_bvh(WALK_SEED).expect("the fixture parses"),
            &target,
            &bind,
            "walk_seed",
        )
        .expect("the fixture retargets");
        let second = retarget_motion(
            &parse_bvh(WALK_SEED).expect("the fixture parses again"),
            &target,
            &bind,
            "walk_seed",
        )
        .expect("the fixture retargets again");
        assert_eq!(
            first.clip, second.clip,
            "the same file cooks the same bytes"
        );
        assert_eq!(first.clip.digest(), second.clip.digest());

        // Play it: every sampled pose validates and skins to finite matrices.
        let duration = first.clip.duration;
        assert!(duration > 0.0);
        let mut ticks = 0usize;
        let mut time = 0.0f32;
        while time < duration {
            let pose = first.clip.sample(&target, time).expect("the clip samples");
            validate_pose(&target, &pose).expect("a sampled pose is playable");
            let matrices = skinning_matrices(&target, &pose).expect("the pose skins");
            assert!(matrices
                .iter()
                .flatten()
                .flatten()
                .all(|value| value.is_finite()));
            ticks += 1;
            time += TICK_DT;
        }
        assert!(ticks >= 5, "only {ticks} ticks played");

        // Frame 2 lands on its own key: the root has travelled 10 cm and the
        // left knee bends the 12 degrees the file authored.
        let pose = first
            .clip
            .sample(&target, 2.0 * 0.0333333)
            .expect("frame 2 samples");
        assert!(
            (pose.positions[0][0] - 0.10).abs() < 1e-4,
            "{:?}",
            pose.positions[0]
        );
        let knee = axis_quat(0, 12.0);
        let got = pose.rotations[13];
        for lane in 0..4 {
            assert!(
                (got[lane] - knee[lane]).abs() < 1e-5,
                "lane {lane}: {got:?} vs {knee:?}"
            );
        }
    }
}
