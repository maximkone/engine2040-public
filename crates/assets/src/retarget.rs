//! retarget: engine-native animation retarget cook.
//!
//! Consumer: the locomotion bake retargets [`RetargetedClip`]s onto the hero
//! skeleton for the motion database; `engine-arena` (tick-root intent) and
//! `engine-render` (skinning) consume the cooked [`RootTable`] and
//! [`ContactTable`] through the anim contract. Data owner: `engine-assets`;
//! `engine-anim` owns the contract types and never imports assets back.
//! Units: meters, seconds, radians in; the same out.
//!
//! Pipeline: [`BoneMap::build`] resolves target-name/source-name bone pairs
//! against both skeletons; [`retarget_clip`] re-expresses one source
//! [`ImportedClip`] in the target bind frame; [`retarget_cook`] additionally
//! extracts root motion and foot contacts through the shared skin-import
//! table cooks and attaches [`ClipTags`] parsed from a sidecar JSON doc (see
//! [`parse_sidecar`]); [`retarget_digest`] folds one cook into a stable `u64`
//! so the bake can prove byte-identical output.
//!
//! Retarget math: rotation keys keep their bind-relative delta, so the target
//! key is `normalize(source_key * conj(source_bind) * target_bind)`; position
//! keys keep their bind-relative offset scaled by the bind bone-length ratio
//! `|target_bone| / |source_bone|`, where a bone length is the bind distance
//! from the joint to its parent. The root always carries world travel 1:1
//! (stature scaling is the bake profile's job, not a guess here).
//!
//! Determinism: joint iteration is always target order, maps are ordered
//! pairs, sidecar objects keep document order, and the math uses only
//! `+ - * /` and `sqrt`. The same plan plus tags cooks the same bytes twice.
//!
//! Documented fallbacks (never guesses): target joints with no map entry hold
//! the target bind pose on a single key; degenerate bone lengths (at or below
//! [`BONE_FLOOR`]) retarget unscaled; JSON `null` reads as a missing sidecar
//! field; unknown sidecar fields are ignored. Rejected instead: map names
//! absent from either skeleton, duplicate target entries, non-finite or
//! non-unit binds, degenerate or unordered keys, malformed sidecars and
//! oversized input. Every failure is a typed [`RetargetError`], never a panic.
//!
//! Fixture provenance: every skeleton/clip fixture in the test module is
//! synthetic data authored in code in the style of the Quaternius Universal
//! Animation Library (CC0) humanoid clips — idle, walk and run — with no donor
//! bytes and no hero O7 content. Real Quaternius `.glb` files enter through
//! [`import_skinned_bytes`](super::skin_import::import_skinned_bytes) plus
//! [`import_clip`](super::skin_import::import_clip) at bake time.

use super::cooked::len_word;
use super::skin_import::{
    bound_clip_name, cook_clip_tables, CookConfig, ImportedClip, SkinImportError, QUAT_NORM_EPS,
};
use super::wire::bound_chars;
use engine_anim::contract::{
    validate_skeleton, AnimError, ContactTable, RootTable, Skeleton, NAME_MAX_LEN,
};
use engine_anim::pose::QuatTrack;
use engine_anim::{Clip, Track};
use engine_core::{quat_mul, Fnv64};
use std::fmt;

/// Max sidecar JSON bytes accepted; larger files are refused, never truncated.
pub const MAX_SIDECAR_BYTES: usize = 65_536;
/// Max tags carried per clip sidecar.
pub const MAX_TAGS: usize = 32;
/// Max chars per tag, socket or edge token.
pub const MAX_TAG_CHARS: usize = 64;
/// Max combat hit windows per clip sidecar.
pub const MAX_HIT_WINDOWS: usize = 32;
/// Max traversal edge markers per clip sidecar.
pub const MAX_TRAVERSAL: usize = 32;
/// Max chars of parser detail carried in [`RetargetError::BadSidecar`].
pub const MAX_SIDECAR_DETAIL_CHARS: usize = 128;
/// Max JSON nesting depth accepted in a sidecar.
pub const MAX_JSON_DEPTH: u32 = 32;
/// Bone lengths at or below this (meters) retarget unscaled, never divide.
const BONE_FLOOR: f32 = 1e-6;

/// Why a retarget cook failed; every malformed input maps to one variant.
#[derive(Clone, Debug, PartialEq)]
pub enum RetargetError {
    /// A bone-map name is absent from its skeleton; the name is carried.
    MissingBone {
        /// Truncated bone name.
        name: String,
    },
    /// Two map pairs name the same target joint; the name is carried.
    DuplicateBone {
        /// Truncated bone name.
        name: String,
    },
    /// A skeleton failed anim-contract validation; joint index carried.
    BadSkeleton {
        /// Joint index, or zero when the skeleton is empty.
        joint: usize,
    },
    /// A hand-built map disagrees with the target joint count.
    MapMismatch {
        /// Target joint count.
        expected: usize,
        /// Map entry count.
        got: usize,
    },
    /// A bind pose disagrees with its skeleton joint count.
    BindMismatch {
        /// Skeleton joint count.
        expected: usize,
        /// Bind array length.
        got: usize,
    },
    /// A bind rotation is not unit length; joint index carried.
    BadBind {
        /// Joint index in skeleton order.
        joint: usize,
    },
    /// A source key track has mismatched, unsorted or missing data.
    BadClip {
        /// Joint index in source order.
        joint: usize,
    },
    /// Source clip track counts differ from the source joint count.
    ClipLengthMismatch {
        /// Expected track count (the source joint count).
        expected: usize,
        /// Fewer tracks actually present.
        got: usize,
    },
    /// A source rotation key is not unit length.
    BadQuat {
        /// Joint index in source order.
        joint: usize,
        /// Key index within the track.
        key: usize,
    },
    /// A sidecar JSON doc is malformed or violates the tag schema.
    BadSidecar {
        /// Truncated parser message.
        detail: String,
    },
    /// A cooked root table failed validation; delta index carried.
    BadRoot {
        /// Delta index.
        index: usize,
    },
    /// A cooked contact table failed validation; sample index carried.
    BadContact {
        /// Sample index.
        index: usize,
    },
    /// A cook config names a joint past the target skeleton.
    UnknownJoint {
        /// Offending joint index.
        joint: usize,
    },
    /// An input exceeds a budget (sidecar bytes, tags, windows, markers).
    TooLarge {
        /// Offending count.
        count: usize,
    },
    /// Non-finite float in the named lane.
    NonFinite(&'static str),
}

impl fmt::Display for RetargetError {
    /// Diagnostics stay bounded: carried strings render truncated to
    /// [`MAX_SIDECAR_DETAIL_CHARS`] chars no matter how they were built.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBone { name } => {
                write!(f, "retarget missing bone {}", bound_detail(name))
            }
            Self::DuplicateBone { name } => {
                write!(f, "retarget duplicate bone {}", bound_detail(name))
            }
            Self::BadSkeleton { joint } => write!(f, "retarget bad skeleton joint {joint}"),
            Self::MapMismatch { expected, got } => {
                write!(f, "retarget wants {expected} map rows, found {got}")
            }
            Self::BindMismatch { expected, got } => {
                write!(f, "retarget wants {expected} bind rows, found {got}")
            }
            Self::BadBind { joint } => write!(f, "retarget bad bind joint {joint}"),
            Self::BadClip { joint } => write!(f, "retarget bad clip track {joint}"),
            Self::ClipLengthMismatch { expected, got } => {
                write!(f, "retarget wants {expected} tracks, found {got}")
            }
            Self::BadQuat { joint, key } => {
                write!(f, "retarget joint {joint} key {key} is not a unit quat")
            }
            Self::BadSidecar { detail } => {
                write!(f, "retarget bad sidecar: {}", bound_detail(detail))
            }
            Self::BadRoot { index } => write!(f, "retarget bad root delta {index}"),
            Self::BadContact { index } => write!(f, "retarget bad contact {index}"),
            Self::UnknownJoint { joint } => write!(f, "retarget unknown joint {joint}"),
            Self::TooLarge { count } => write!(f, "retarget too large: {count}"),
            Self::NonFinite(field) => write!(f, "retarget non-finite {field}"),
        }
    }
}

impl std::error::Error for RetargetError {}

/// Truncate a carried diagnostic to [`MAX_SIDECAR_DETAIL_CHARS`] chars.
fn bound_detail(text: &str) -> String {
    bound_chars(text, MAX_SIDECAR_DETAIL_CHARS)
}

/// Truncate a bone name to the anim contract's [`NAME_MAX_LEN`] chars.
fn bound_bone_name(name: &str) -> String {
    bound_chars(name, NAME_MAX_LEN)
}

/// Map an anim-contract skeleton failure onto the joint it names; the remaining variants
/// cannot come out of [`validate_skeleton`] and map to joint zero.
fn map_skeleton_error(error: AnimError) -> RetargetError {
    match error {
        AnimError::EmptySkeleton => RetargetError::BadSkeleton { joint: 0 },
        AnimError::TooManyJoints(count) => RetargetError::TooLarge { count },
        AnimError::BadName(joint) | AnimError::BadParent(joint) => {
            RetargetError::BadSkeleton { joint }
        }
        _ => RetargetError::BadSkeleton { joint: 0 },
    }
}

/// Bind pose: one local offset (meters) plus one unit quaternion per joint in
/// skeleton order, mirroring the glTF node transforms the skin import reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BindPose {
    /// Local offsets in meters, one per joint.
    pub positions: Vec<[f32; 3]>,
    /// Unit quaternions `[x, y, z, w]`, one per joint.
    pub rotations: Vec<[f32; 4]>,
}

/// Validate a bind pose against its skeleton: row counts, finite offsets and
/// unit rotations.
pub fn validate_bind(skeleton: &Skeleton, bind: &BindPose) -> Result<(), RetargetError> {
    validate_skeleton(skeleton).map_err(map_skeleton_error)?;
    if bind.positions.len() != skeleton.len() {
        return Err(RetargetError::BindMismatch {
            expected: skeleton.len(),
            got: bind.positions.len(),
        });
    }
    if bind.rotations.len() != skeleton.len() {
        return Err(RetargetError::BindMismatch {
            expected: skeleton.len(),
            got: bind.rotations.len(),
        });
    }
    if bind.positions.iter().flatten().any(|v| !v.is_finite()) {
        return Err(RetargetError::NonFinite("bind.positions"));
    }
    for (joint, quat) in bind.rotations.iter().enumerate() {
        if quat.iter().any(|v| !v.is_finite()) {
            return Err(RetargetError::NonFinite("bind.rotations"));
        }
        let norm = quat.iter().map(|v| v * v).sum::<f32>().sqrt();
        if (norm - 1.0).abs() > QUAT_NORM_EPS {
            return Err(RetargetError::BadBind { joint });
        }
    }
    Ok(())
}

/// Bone map: per target joint the source joint index, resolved by name at
/// [`BoneMap::build`] time. `None` rows hold the target bind pose (the
/// documented fallback for joints the source rig has no counterpart for).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoneMap {
    target_to_source: Vec<Option<usize>>,
}

impl BoneMap {
    /// Resolve `(target_name, source_name)` pairs against both skeletons in
    /// pair order. Unknown names fail [`RetargetError::MissingBone`], a
    /// repeated target fails [`RetargetError::DuplicateBone`].
    pub fn build(
        source: &Skeleton,
        target: &Skeleton,
        pairs: &[(&str, &str)],
    ) -> Result<Self, RetargetError> {
        validate_skeleton(source).map_err(map_skeleton_error)?;
        validate_skeleton(target).map_err(map_skeleton_error)?;
        let mut rows = vec![None; target.len()];
        for (target_name, source_name) in pairs {
            let slot =
                joint_index(target, target_name).ok_or_else(|| RetargetError::MissingBone {
                    name: bound_bone_name(target_name),
                })?;
            let origin =
                joint_index(source, source_name).ok_or_else(|| RetargetError::MissingBone {
                    name: bound_bone_name(source_name),
                })?;
            if rows[slot].is_some() {
                return Err(RetargetError::DuplicateBone {
                    name: bound_bone_name(target_name),
                });
            }
            rows[slot] = Some(origin);
        }
        Ok(Self {
            target_to_source: rows,
        })
    }

    /// Row count; always the target joint count for built maps.
    pub fn len(&self) -> usize {
        self.target_to_source.len()
    }

    /// True when the map carries no rows.
    pub fn is_empty(&self) -> bool {
        self.target_to_source.is_empty()
    }

    /// Source joint for a target joint, or `None` when the joint holds bind
    /// (or the index is out of range, which never panics).
    pub fn source_of(&self, target_joint: usize) -> Option<usize> {
        self.target_to_source.get(target_joint).copied().flatten()
    }
}

/// Joint index by name, `None` when the skeleton has no such joint.
fn joint_index(skeleton: &Skeleton, name: &str) -> Option<usize> {
    skeleton.joints.iter().position(|joint| joint.name == name)
}

/// Quaternion conjugate: the inverse rotation for unit quaternions.
fn quat_conj(quat: &[f32; 4]) -> [f32; 4] {
    [-quat[0], -quat[1], -quat[2], quat[3]]
}

/// Normalize a quaternion; `None` on degenerate input (never panics).
fn quat_normalize(quat: [f32; 4]) -> Option<[f32; 4]> {
    let norm_squared =
        quat[0] * quat[0] + quat[1] * quat[1] + quat[2] * quat[2] + quat[3] * quat[3];
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

/// Bind bone length: distance from the joint to its parent (meters); the root
/// and unknown joints measure zero so the caller falls back to unit scale.
fn bone_len(bind: &BindPose, skeleton: &Skeleton, joint: usize) -> f32 {
    let parent = skeleton.joints.get(joint).and_then(|entry| entry.parent);
    let Some(origin) = parent else {
        return 0.0;
    };
    let (tip, base) = (bind.positions[joint], bind.positions[origin]);
    let delta = [tip[0] - base[0], tip[1] - base[1], tip[2] - base[2]];
    (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt()
}

/// Per-joint translation scale: the target/source bind bone-length ratio. The
/// target root always scales 1.0 so world travel carries over exactly;
/// degenerate bones scale 1.0 instead of dividing.
fn joint_scale(
    source: &Skeleton,
    source_bind: &BindPose,
    target: &Skeleton,
    target_bind: &BindPose,
    target_joint: usize,
    source_joint: usize,
) -> f32 {
    if target.joints[target_joint].parent.is_none() {
        return 1.0;
    }
    let from = bone_len(source_bind, source, source_joint);
    let to = bone_len(target_bind, target, target_joint);
    if !from.is_finite() || !to.is_finite() || from <= BONE_FLOOR || to <= BONE_FLOOR {
        1.0
    } else {
        to / from
    }
}

/// Validate one source key-time lane: non-empty, finite, strictly increasing.
fn check_times(times: &[f32], joint: usize) -> Result<(), RetargetError> {
    if times.is_empty() {
        return Err(RetargetError::BadClip { joint });
    }
    if times.iter().any(|t| !t.is_finite()) {
        return Err(RetargetError::NonFinite("clip.times"));
    }
    let mut prev = times[0];
    for time in &times[1..] {
        if *time <= prev {
            return Err(RetargetError::BadClip { joint });
        }
        prev = *time;
    }
    Ok(())
}

/// Re-express one source clip on the target skeleton: mapped joints carry
/// bind-relative deltas through the correction math, unmapped joints hold the
/// target bind pose on a single key. Track counts, name and duration follow
/// the source clip.
pub fn retarget_clip(
    source_clip: &ImportedClip,
    source: &Skeleton,
    source_bind: &BindPose,
    target: &Skeleton,
    target_bind: &BindPose,
    map: &BoneMap,
) -> Result<ImportedClip, RetargetError> {
    validate_skeleton(source).map_err(map_skeleton_error)?;
    validate_skeleton(target).map_err(map_skeleton_error)?;
    validate_bind(source, source_bind)?;
    validate_bind(target, target_bind)?;
    if map.len() != target.len() {
        return Err(RetargetError::MapMismatch {
            expected: target.len(),
            got: map.len(),
        });
    }
    if source_clip.positions.tracks.len() != source.len()
        || source_clip.rotations.len() != source.len()
    {
        return Err(RetargetError::ClipLengthMismatch {
            expected: source.len(),
            got: source_clip
                .positions
                .tracks
                .len()
                .min(source_clip.rotations.len()),
        });
    }
    if !source_clip.duration.is_finite() || source_clip.duration < 0.0 {
        return Err(RetargetError::NonFinite("clip.duration"));
    }
    // Validate every source lane first so no partial cook can escape.
    for (joint, (positions, rotations)) in source_clip
        .positions
        .tracks
        .iter()
        .zip(source_clip.rotations.iter())
        .enumerate()
    {
        check_times(&positions.times, joint)?;
        check_times(&rotations.times, joint)?;
        if positions.times.len() != positions.values.len() {
            return Err(RetargetError::BadClip { joint });
        }
        if rotations.times.len() != rotations.values.len() {
            return Err(RetargetError::BadClip { joint });
        }
        if positions.values.iter().flatten().any(|v| !v.is_finite()) {
            return Err(RetargetError::NonFinite("clip.values"));
        }
        for (key, quat) in rotations.values.iter().enumerate() {
            if quat.iter().any(|v| !v.is_finite()) {
                return Err(RetargetError::NonFinite("clip.rotations"));
            }
            let norm = quat.iter().map(|v| v * v).sum::<f32>().sqrt();
            if (norm - 1.0).abs() > QUAT_NORM_EPS {
                return Err(RetargetError::BadQuat { joint, key });
            }
        }
    }
    // One lane per target joint: correction quat plus translation scale.
    struct Lane {
        origin: usize,
        scale: f32,
        correction: [f32; 4],
    }
    let lanes: Vec<Option<Lane>> = (0..target.len())
        .map(|slot| {
            map.source_of(slot).map(|origin| Lane {
                origin,
                scale: joint_scale(source, source_bind, target, target_bind, slot, origin),
                correction: quat_mul(
                    quat_conj(&source_bind.rotations[origin]),
                    target_bind.rotations[slot],
                ),
            })
        })
        .collect();
    let mut positions = Vec::with_capacity(target.len());
    let mut rotations = Vec::with_capacity(target.len());
    for (slot, lane) in lanes.iter().enumerate() {
        let Some(lane) = lane else {
            positions.push(Track {
                times: vec![0.0],
                values: vec![target_bind.positions[slot]],
            });
            rotations.push(QuatTrack {
                times: vec![0.0],
                values: vec![target_bind.rotations[slot]],
            });
            continue;
        };
        let source_positions = &source_clip.positions.tracks[lane.origin];
        let source_rotations = &source_clip.rotations[lane.origin];
        let source_base = source_bind.positions[lane.origin];
        let target_base = target_bind.positions[slot];
        let mut target_values = Vec::with_capacity(source_positions.values.len());
        for value in &source_positions.values {
            let moved = [
                target_base[0] + lane.scale * (value[0] - source_base[0]),
                target_base[1] + lane.scale * (value[1] - source_base[1]),
                target_base[2] + lane.scale * (value[2] - source_base[2]),
            ];
            if moved.iter().any(|v| !v.is_finite()) {
                return Err(RetargetError::NonFinite("clip.values"));
            }
            target_values.push(moved);
        }
        positions.push(Track {
            times: source_positions.times.clone(),
            values: target_values,
        });
        let mut target_quats = Vec::with_capacity(source_rotations.values.len());
        for (key, quat) in source_rotations.values.iter().enumerate() {
            let turned =
                quat_normalize(quat_mul(*quat, lane.correction)).ok_or(RetargetError::BadQuat {
                    joint: lane.origin,
                    key,
                })?;
            target_quats.push(turned);
        }
        rotations.push(QuatTrack {
            times: source_rotations.times.clone(),
            values: target_quats,
        });
    }
    Ok(ImportedClip {
        name: bound_clip_name(&source_clip.name),
        duration: source_clip.duration,
        positions: Clip { tracks: positions },
        rotations,
    })
}

// ---------------------------------------------------------------------------
// Sidecar JSON: clip tags without new dependencies (std-only parser).
// ---------------------------------------------------------------------------

/// Sidecar value: objects keep document order so cooks stay deterministic.
#[derive(Clone, Debug, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

/// Why the sidecar JSON parser refused its input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JsonErrorKind {
    BadLiteral,
    NestingTooDeep,
    WantValue,
    WantKey,
    DuplicateKey,
    WantColon,
    WantComma,
    OpenString,
    OpenEscape,
    LoneSurrogate,
    BadCodepoint,
    BadEscape,
    ControlChar,
    StringTooLong,
    BadUtf8,
    OpenUnicode,
    BadUnicode,
    BadNumber,
    TrailingBytes,
}

impl JsonErrorKind {
    fn what(self) -> &'static str {
        match self {
            Self::BadLiteral => "bad literal",
            Self::NestingTooDeep => "nesting too deep",
            Self::WantValue => "want value",
            Self::WantKey => "want key",
            Self::DuplicateKey => "duplicate key",
            Self::WantColon => "want colon",
            Self::WantComma => "want comma",
            Self::OpenString => "open string",
            Self::OpenEscape => "open escape",
            Self::LoneSurrogate => "lone surrogate",
            Self::BadCodepoint => "bad codepoint",
            Self::BadEscape => "bad escape",
            Self::ControlChar => "control char",
            Self::StringTooLong => "string too long",
            Self::BadUtf8 => "bad utf-8",
            Self::OpenUnicode => "open unicode",
            Self::BadUnicode => "bad unicode",
            Self::BadNumber => "bad number",
            Self::TrailingBytes => "trailing bytes",
        }
    }
}

/// Sidecar JSON refusal: the byte offset it happened at plus the reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct JsonError {
    at: usize,
    kind: JsonErrorKind,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "byte {}: {}", self.at, self.kind.what())
    }
}

impl std::error::Error for JsonError {}

/// Byte cursor for the sidecar parser; errors name the byte offset.
struct JsonCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonCursor<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn fail(&self, kind: JsonErrorKind) -> JsonError {
        JsonError {
            at: self.pos.min(self.bytes.len()),
            kind,
        }
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, JsonError> {
        let end = self.pos + word.len();
        if self.bytes.get(self.pos..end) != Some(word.as_bytes()) {
            return Err(self.fail(JsonErrorKind::BadLiteral));
        }
        if let Some(next) = self.bytes.get(end) {
            if next.is_ascii_alphanumeric() || *next == b'_' {
                return Err(self.fail(JsonErrorKind::BadLiteral));
            }
        }
        self.pos = end;
        Ok(value)
    }

    fn value(&mut self, depth: u32) -> Result<Json, JsonError> {
        if depth > MAX_JSON_DEPTH {
            return Err(self.fail(JsonErrorKind::NestingTooDeep));
        }
        self.ws();
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => Ok(Json::Num(self.number()?)),
            _ => Err(self.fail(JsonErrorKind::WantValue)),
        }
    }

    fn object(&mut self, depth: u32) -> Result<Json, JsonError> {
        self.pos += 1;
        let mut fields = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Obj(fields));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(self.fail(JsonErrorKind::WantKey));
            }
            let key = self.string()?;
            if fields.iter().any(|(seen, _)| *seen == key) {
                return Err(self.fail(JsonErrorKind::DuplicateKey));
            }
            self.ws();
            if self.peek() != Some(b':') {
                return Err(self.fail(JsonErrorKind::WantColon));
            }
            self.pos += 1;
            let value = self.value(depth + 1)?;
            fields.push((key, value));
            self.ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Obj(fields));
                }
                _ => return Err(self.fail(JsonErrorKind::WantComma)),
            }
        }
    }

    fn array(&mut self, depth: u32) -> Result<Json, JsonError> {
        self.pos += 1;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            let value = self.value(depth + 1)?;
            items.push(value);
            self.ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Arr(items));
                }
                _ => return Err(self.fail(JsonErrorKind::WantComma)),
            }
        }
    }

    /// Raw string bytes; control chars and bad escapes are refused here.
    fn string(&mut self) -> Result<String, JsonError> {
        self.pos += 1;
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let byte = self
                .peek()
                .ok_or_else(|| self.fail(JsonErrorKind::OpenString))?;
            match byte {
                b'"' => {
                    self.pos += 1;
                    break;
                }
                b'\\' => {
                    self.pos += 1;
                    match self
                        .peek()
                        .ok_or_else(|| self.fail(JsonErrorKind::OpenEscape))?
                    {
                        b'"' => {
                            buf.push(b'"');
                            self.pos += 1;
                        }
                        b'\\' => {
                            buf.push(b'\\');
                            self.pos += 1;
                        }
                        b'/' => {
                            buf.push(b'/');
                            self.pos += 1;
                        }
                        b'b' => {
                            buf.push(0x08);
                            self.pos += 1;
                        }
                        b'f' => {
                            buf.push(0x0C);
                            self.pos += 1;
                        }
                        b'n' => {
                            buf.push(b'\n');
                            self.pos += 1;
                        }
                        b'r' => {
                            buf.push(b'\r');
                            self.pos += 1;
                        }
                        b't' => {
                            buf.push(b'\t');
                            self.pos += 1;
                        }
                        b'u' => {
                            self.pos += 1;
                            let point = self.escape_point()?;
                            let glyph = if (0xD800..0xDC00).contains(&point) {
                                if self.bytes.get(self.pos..self.pos + 2) != Some(b"\\u".as_slice())
                                {
                                    return Err(self.fail(JsonErrorKind::LoneSurrogate));
                                }
                                self.pos += 2;
                                let low = self.escape_point()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return Err(self.fail(JsonErrorKind::LoneSurrogate));
                                }
                                let full = 0x10000 + ((point - 0xD800) << 10) + (low - 0xDC00);
                                char::from_u32(full)
                                    .ok_or_else(|| self.fail(JsonErrorKind::BadCodepoint))?
                            } else if (0xDC00..0xE000).contains(&point) {
                                return Err(self.fail(JsonErrorKind::LoneSurrogate));
                            } else {
                                char::from_u32(point)
                                    .ok_or_else(|| self.fail(JsonErrorKind::BadCodepoint))?
                            };
                            let mut scratch = [0u8; 4];
                            buf.extend_from_slice(glyph.encode_utf8(&mut scratch).as_bytes());
                        }
                        _ => return Err(self.fail(JsonErrorKind::BadEscape)),
                    }
                }
                0x00..=0x1F => return Err(self.fail(JsonErrorKind::ControlChar)),
                _ => {
                    buf.push(byte);
                    self.pos += 1;
                }
            }
            if buf.len() > MAX_SIDECAR_BYTES {
                return Err(self.fail(JsonErrorKind::StringTooLong));
            }
        }
        String::from_utf8(buf).map_err(|_| self.fail(JsonErrorKind::BadUtf8))
    }

    fn escape_point(&mut self) -> Result<u32, JsonError> {
        let digits = self
            .bytes
            .get(self.pos..self.pos + 4)
            .ok_or_else(|| self.fail(JsonErrorKind::OpenUnicode))?;
        let value =
            engine_core::parse_hex4(digits).ok_or_else(|| self.fail(JsonErrorKind::BadUnicode))?;
        self.pos += 4;
        Ok(value)
    }

    /// Strict JSON number: no leading `+`, no leading zeros, no `NaN`.
    fn number(&mut self) -> Result<f64, JsonError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.pos += 1;
                }
            }
            _ => return Err(self.fail(JsonErrorKind::BadNumber)),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.fail(JsonErrorKind::BadNumber));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.fail(JsonErrorKind::BadNumber));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos])
            .map_err(|_| self.fail(JsonErrorKind::BadNumber))?;
        text.parse::<f64>()
            .map_err(|_| self.fail(JsonErrorKind::BadNumber))
    }
}

/// Parse one sidecar doc; trailing bytes are refused, never ignored.
fn parse_json(bytes: &[u8]) -> Result<Json, JsonError> {
    let mut cursor = JsonCursor { bytes, pos: 0 };
    let value = cursor.value(0)?;
    cursor.ws();
    if cursor.pos != bytes.len() {
        return Err(cursor.fail(JsonErrorKind::TrailingBytes));
    }
    Ok(value)
}

/// One combat hit window: a socket struck between two clip times (seconds).
/// The combat step reads these (socket plus start/end); locomotion ignores
/// them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HitWindow {
    /// Socket name (`SOCKET_<name>` convention), token charset.
    pub socket: String,
    /// Window start in seconds, at or after zero.
    pub start_s: f32,
    /// Window end in seconds, after the start.
    pub end_s: f32,
}

/// Edge markers the link layer reads, closed so the fight-data audit has one
/// vocabulary (`finisher` is the marker the traversal cook stamps on finisher
/// clips). New edges extend this list with their consumer, never as
/// free-form sidecar text.
pub const TRAVERSAL_EDGES: [&str; 6] = ["vault", "hurdle", "mantle", "climb", "drop", "finisher"];

/// True when the edge names a link-layer traversal the audit accepts.
pub fn is_traversal_edge(edge: &str) -> bool {
    TRAVERSAL_EDGES.contains(&edge)
}

/// One traversal edge marker: a named edge at one clip time (seconds).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraversalMarker {
    /// Edge name from [`TRAVERSAL_EDGES`]; the traversal link reads these.
    pub edge: String,
    /// Clip time in seconds, at or after zero.
    pub time_s: f32,
}

/// Clip tags from a sidecar JSON doc: loop and root-motion flags, locomotion
/// tags, an optional contact threshold override, foot names, combat hit
/// windows and traversal edge markers. Consumers split by field: combat reads
/// `hit_windows`, locomotion reads `looped`, `root_motion` and the cooked
/// contact tables, the traversal link reads `traversal`. Unknown fields are
/// ignored so older cooks read newer sidecars; every known field is
/// validated strictly.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipTags {
    /// Clip name from the sidecar, empty when the doc names none.
    pub clip: String,
    /// True when the clip tiles seamlessly.
    pub looped: bool,
    /// True when the clip drives root motion (metadata for locomotion).
    pub root_motion: bool,
    /// Locomotion tags (`walk`, `strafe_left`, ...), token charset.
    pub tags: Vec<String>,
    /// Contact threshold override in meters; `None` keeps the cook config.
    pub ground_y: Option<f32>,
    /// `(left, right)` foot joint names; `None` keeps the cook config.
    pub feet: Option<(String, String)>,
    /// Combat hit windows in document order.
    pub hit_windows: Vec<HitWindow>,
    /// Traversal edge markers in document order.
    pub traversal: Vec<TraversalMarker>,
}

/// Refuse a sidecar with a bounded message.
fn sidecar_fail(what: &str) -> RetargetError {
    RetargetError::BadSidecar {
        detail: bound_detail(what),
    }
}

/// One object field by key; document order decides nothing here.
fn obj_field<'a>(fields: &'a [(String, Json)], key: &str) -> Option<&'a Json> {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

/// Optional bool field; `null` and missing read as the default.
fn opt_bool(fields: &[(String, Json)], key: &str, default: bool) -> Result<bool, RetargetError> {
    match obj_field(fields, key) {
        None | Some(Json::Null) => Ok(default),
        Some(Json::Bool(flag)) => Ok(*flag),
        Some(_) => Err(sidecar_fail(&format!("{key}: want bool"))),
    }
}

/// Optional finite number field; `null` and missing read as `None`.
fn opt_f32(fields: &[(String, Json)], key: &str) -> Result<Option<f32>, RetargetError> {
    match obj_field(fields, key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Num(number)) => {
            if !number.is_finite() {
                return Err(sidecar_fail(&format!("{key}: want finite")));
            }
            let value = *number as f32;
            if value.is_finite() {
                Ok(Some(value))
            } else {
                Err(sidecar_fail(&format!("{key}: want finite")))
            }
        }
        Some(_) => Err(sidecar_fail(&format!("{key}: want number"))),
    }
}

/// Required finite number field.
fn req_f32(value: Option<&Json>, context: &str) -> Result<f32, RetargetError> {
    match value {
        Some(Json::Num(number)) if number.is_finite() => {
            let value = *number as f32;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(sidecar_fail(&format!("{context}: want finite")))
            }
        }
        _ => Err(sidecar_fail(&format!("{context}: want number"))),
    }
}

/// Token charset: ASCII alphanumerics plus `_`, within [`MAX_TAG_CHARS`].
fn valid_token(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_TAG_CHARS
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Bone charset: the token rule plus the anim contract's no-leading-digit rule.
fn valid_bone(text: &str) -> bool {
    valid_token(text) && !text.as_bytes()[0].is_ascii_digit()
}

/// Required token field.
fn req_token(value: &Json, context: &str) -> Result<String, RetargetError> {
    match value {
        Json::Str(text) if valid_token(text) => Ok(text.clone()),
        _ => Err(sidecar_fail(&format!("{context}: want token"))),
    }
}

/// Required bone-name field.
fn req_bone(value: &Json, context: &str) -> Result<String, RetargetError> {
    match value {
        Json::Str(text) if valid_bone(text) => Ok(text.clone()),
        _ => Err(sidecar_fail(&format!("{context}: want bone"))),
    }
}

/// Parse a sidecar JSON doc into validated [`ClipTags`]. Malformed JSON, wrong
/// types, out-of-range windows and over-budget arrays are refused; unknown
/// fields are ignored.
pub fn parse_sidecar(text: &str) -> Result<ClipTags, RetargetError> {
    if text.len() > MAX_SIDECAR_BYTES {
        return Err(RetargetError::TooLarge { count: text.len() });
    }
    let json = parse_json(text.as_bytes()).map_err(|error| RetargetError::BadSidecar {
        detail: bound_detail(&error.to_string()),
    })?;
    let Json::Obj(fields) = &json else {
        return Err(sidecar_fail("want object"));
    };
    let clip = match obj_field(fields, "clip") {
        None | Some(Json::Null) => String::new(),
        Some(Json::Str(name)) => bound_clip_name(name),
        Some(_) => return Err(sidecar_fail("clip: want string")),
    };
    let looped = opt_bool(fields, "loop", false)?;
    let root_motion = opt_bool(fields, "root_motion", false)?;
    let mut tags = Vec::new();
    match obj_field(fields, "tags") {
        None | Some(Json::Null) => {}
        Some(Json::Arr(items)) => {
            if items.len() > MAX_TAGS {
                return Err(RetargetError::TooLarge { count: items.len() });
            }
            for (index, item) in items.iter().enumerate() {
                let tag = req_token(item, &format!("tags[{index}]"))?;
                if tags.contains(&tag) {
                    return Err(sidecar_fail(&format!("tags[{index}]: duplicate")));
                }
                tags.push(tag);
            }
        }
        Some(_) => return Err(sidecar_fail("tags: want array")),
    }
    let ground_y = opt_f32(fields, "ground_y")?;
    let feet = match obj_field(fields, "feet") {
        None | Some(Json::Null) => None,
        Some(Json::Obj(pair)) => {
            let left = obj_field(pair, "left")
                .ok_or_else(|| sidecar_fail("feet: wants left and right"))?;
            let right = obj_field(pair, "right")
                .ok_or_else(|| sidecar_fail("feet: wants left and right"))?;
            Some((req_bone(left, "feet.left")?, req_bone(right, "feet.right")?))
        }
        Some(_) => return Err(sidecar_fail("feet: want object")),
    };
    let mut hit_windows = Vec::new();
    match obj_field(fields, "hit_windows") {
        None | Some(Json::Null) => {}
        Some(Json::Arr(items)) => {
            if items.len() > MAX_HIT_WINDOWS {
                return Err(RetargetError::TooLarge { count: items.len() });
            }
            for (index, item) in items.iter().enumerate() {
                let context = format!("hit_windows[{index}]");
                let Json::Obj(window) = item else {
                    return Err(sidecar_fail(&format!("{context}: want object")));
                };
                let socket = req_token(
                    obj_field(window, "socket")
                        .ok_or_else(|| sidecar_fail(&format!("{context}: wants socket")))?,
                    &format!("{context}.socket"),
                )?;
                let start_s = req_f32(obj_field(window, "start_s"), &format!("{context}.start_s"))?;
                let end_s = req_f32(obj_field(window, "end_s"), &format!("{context}.end_s"))?;
                if start_s < 0.0 || end_s <= start_s {
                    return Err(sidecar_fail(&format!("{context}: want 0 <= start < end")));
                }
                hit_windows.push(HitWindow {
                    socket,
                    start_s,
                    end_s,
                });
            }
        }
        Some(_) => return Err(sidecar_fail("hit_windows: want array")),
    }
    let mut traversal = Vec::new();
    match obj_field(fields, "traversal") {
        None | Some(Json::Null) => {}
        Some(Json::Arr(items)) => {
            if items.len() > MAX_TRAVERSAL {
                return Err(RetargetError::TooLarge { count: items.len() });
            }
            for (index, item) in items.iter().enumerate() {
                let context = format!("traversal[{index}]");
                let Json::Obj(marker) = item else {
                    return Err(sidecar_fail(&format!("{context}: want object")));
                };
                let edge = req_token(
                    obj_field(marker, "edge")
                        .ok_or_else(|| sidecar_fail(&format!("{context}: wants edge")))?,
                    &format!("{context}.edge"),
                )?;
                let time_s = req_f32(obj_field(marker, "time_s"), &format!("{context}.time_s"))?;
                if !is_traversal_edge(&edge) {
                    return Err(sidecar_fail(&format!(
                        "{context}: want vault|hurdle|mantle|climb|drop|finisher"
                    )));
                }
                if time_s < 0.0 {
                    return Err(sidecar_fail(&format!("{context}: want time at or after 0")));
                }
                traversal.push(TraversalMarker { edge, time_s });
            }
        }
        Some(_) => return Err(sidecar_fail("traversal: want array")),
    }
    Ok(ClipTags {
        clip,
        looped,
        root_motion,
        tags,
        ground_y,
        feet,
        hit_windows,
        traversal,
    })
}

/// Parse a sidecar from raw bytes: size-bounded first, then UTF-8, then JSON.
pub fn parse_sidecar_bytes(bytes: &[u8]) -> Result<ClipTags, RetargetError> {
    if bytes.len() > MAX_SIDECAR_BYTES {
        return Err(RetargetError::TooLarge { count: bytes.len() });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| sidecar_fail("want utf-8"))?;
    parse_sidecar(text)
}

// ---------------------------------------------------------------------------
// Cook: retarget plus root/contacts tables plus tags, with a stable digest.
// ---------------------------------------------------------------------------

/// One retargeted clip: the target-skeleton clip plus its authoritative tick
/// tables and sidecar tags, ready for the locomotion bake.
#[derive(Clone, Debug, PartialEq)]
pub struct RetargetedClip {
    /// Clip on the target skeleton (name and duration follow the source).
    pub clip: ImportedClip,
    /// Per-tick planar root deltas plus yaw, cooked from the target clip.
    pub root: RootTable,
    /// Per-tick foot plant/lift samples, cooked from the target clip.
    pub contacts: ContactTable,
    /// Validated sidecar tags for this clip.
    pub tags: ClipTags,
}

/// Everything one cook needs; all references outlive the call. Skeletons,
/// binds and the map are shared across clips, so the bake builds them once and
/// reuses the plan per source clip.
#[derive(Clone, Copy, Debug)]
pub struct RetargetPlan<'a> {
    /// Source clip on the source skeleton.
    pub source_clip: &'a ImportedClip,
    /// Source skeleton the clip was authored on.
    pub source: &'a Skeleton,
    /// Source bind pose (local offsets plus rest rotations).
    pub source_bind: &'a BindPose,
    /// Target skeleton (the hero rig at bake time).
    pub target: &'a Skeleton,
    /// Target bind pose.
    pub target_bind: &'a BindPose,
    /// Bone map from target names to source names.
    pub map: &'a BoneMap,
    /// Table cook config: target root/foot joints, threshold, first tick.
    pub config: &'a CookConfig,
}

/// Table-cook failures map onto the matching retarget variant. The container
/// arm is defensive: cooking an already-validated clip cannot produce
/// container errors, but the match stays exhaustive instead of panicking.
fn map_cook_error(error: SkinImportError) -> RetargetError {
    match error {
        SkinImportError::NoSkeleton => RetargetError::BadSkeleton { joint: 0 },
        SkinImportError::BadJoint { joint } => RetargetError::BadSkeleton { joint },
        SkinImportError::UnknownJoint { joint } => RetargetError::UnknownJoint { joint },
        SkinImportError::BadClip { joint } | SkinImportError::EmptyTrack { joint } => {
            RetargetError::BadClip { joint }
        }
        SkinImportError::ClipLengthMismatch { expected, got } => {
            RetargetError::ClipLengthMismatch { expected, got }
        }
        SkinImportError::BadQuat { joint, key } => RetargetError::BadQuat { joint, key },
        SkinImportError::BadRoot { index } => RetargetError::BadRoot { index },
        SkinImportError::BadContact { index } => RetargetError::BadContact { index },
        SkinImportError::TooLarge { count } => RetargetError::TooLarge { count },
        SkinImportError::NonFinite(field) => RetargetError::NonFinite(field),
        _ => RetargetError::NonFinite("cook tables"),
    }
}

/// Cook one retarget: clip re-expressed on the target skeleton, root motion
/// and foot contacts extracted through the shared skin-import cooks, tags
/// attached.
/// The config's root/foot joints index the target skeleton and `ground_y`
/// samples local foot heights (the skin-import threshold rule).
pub fn retarget_cook(
    plan: &RetargetPlan<'_>,
    tags: ClipTags,
) -> Result<RetargetedClip, RetargetError> {
    let clip = retarget_clip(
        plan.source_clip,
        plan.source,
        plan.source_bind,
        plan.target,
        plan.target_bind,
        plan.map,
    )?;
    if plan.config.root_joint >= plan.target.len() {
        return Err(RetargetError::UnknownJoint {
            joint: plan.config.root_joint,
        });
    }
    if plan.config.left_foot >= plan.target.len() {
        return Err(RetargetError::UnknownJoint {
            joint: plan.config.left_foot,
        });
    }
    if plan.config.right_foot >= plan.target.len() {
        return Err(RetargetError::UnknownJoint {
            joint: plan.config.right_foot,
        });
    }
    if !plan.config.ground_y.is_finite() {
        return Err(RetargetError::NonFinite("ground_y"));
    }
    let (root, contacts) =
        cook_clip_tables(&clip, plan.target, plan.config).map_err(map_cook_error)?;
    Ok(RetargetedClip {
        clip,
        root,
        contacts,
        tags,
    })
}

/// Cook one retarget with tags read from a sidecar doc. A sidecar `ground_y`
/// overrides the plan threshold and sidecar `feet` names resolve against the
/// target skeleton (unknown names fail [`RetargetError::MissingBone`]); the
/// `loop` and `root_motion` flags ride along as locomotion metadata.
pub fn retarget_cook_with_sidecar(
    plan: &RetargetPlan<'_>,
    sidecar: &str,
) -> Result<RetargetedClip, RetargetError> {
    let tags = parse_sidecar(sidecar)?;
    let mut config = *plan.config;
    if let Some(ground_y) = tags.ground_y {
        config.ground_y = ground_y;
    }
    if let Some((left, right)) = &tags.feet {
        config.left_foot =
            joint_index(plan.target, left).ok_or_else(|| RetargetError::MissingBone {
                name: bound_bone_name(left),
            })?;
        config.right_foot =
            joint_index(plan.target, right).ok_or_else(|| RetargetError::MissingBone {
                name: bound_bone_name(right),
            })?;
    }
    let overridden = RetargetPlan {
        config: &config,
        ..*plan
    };
    retarget_cook(&overridden, tags)
}

/// Feed one sampled track: key count, key times, then every value component.
fn feed_track<const N: usize>(state: &mut Fnv64, times: &[f32], values: &[[f32; N]]) {
    state.write_u64(len_word(times.len()));
    for time in times {
        state.write_f32(*time);
    }
    for value in values {
        for component in value {
            state.write_f32(*component);
        }
    }
}

/// Stable `u64` over one cook: names, every key bit, every table bit and every
/// tag. Compares digests to prove identical bakes; any input bit flip
/// changes the digest.
pub fn retarget_digest(cooked: &RetargetedClip) -> u64 {
    let mut state = Fnv64::new();
    state.write_str(&cooked.clip.name);
    state.write_u8(0xFF);
    state.write_f32(cooked.clip.duration);
    for track in &cooked.clip.positions.tracks {
        feed_track(&mut state, &track.times, &track.values);
    }
    state.write_u8(0xFE);
    for track in &cooked.clip.rotations {
        feed_track(&mut state, &track.times, &track.values);
    }
    state.write_u8(0xFD);
    for delta in &cooked.root.deltas {
        state.write(&delta.tick.to_le_bytes());
        state.write_f32(delta.dp[0]);
        state.write_f32(delta.dp[1]);
        state.write_f32(delta.dyaw);
    }
    state.write_u8(0xFC);
    for contact in &cooked.contacts.contacts {
        state.write(&contact.tick.to_le_bytes());
        state.write_u8(contact.foot);
        state.write_bool(contact.down);
    }
    state.write_u8(0xFB);
    state.write_bool(cooked.tags.looped);
    state.write_bool(cooked.tags.root_motion);
    for tag in &cooked.tags.tags {
        state.write_str(tag);
        state.write_u8(0);
    }
    match cooked.tags.ground_y {
        None => state.write_u8(0),
        Some(threshold) => {
            state.write_u8(1);
            state.write_f32(threshold);
        }
    }
    match &cooked.tags.feet {
        None => state.write_u8(0),
        Some((left, right)) => {
            state.write_u8(1);
            state.write_str(left);
            state.write_u8(0);
            state.write_str(right);
            state.write_u8(0);
        }
    }
    for window in &cooked.tags.hit_windows {
        state.write_str(&window.socket);
        state.write_u8(0);
        state.write_f32(window.start_s);
        state.write_f32(window.end_s);
    }
    for marker in &cooked.tags.traversal {
        state.write_str(&marker.edge);
        state.write_u8(0);
        state.write_f32(marker.time_s);
    }
    state.finish()
}

/// Bake one combat batch through the intake over this retarget path.
///
/// Consumer: the combat clip bake builds per-clip plans from caller-owned
/// clips plus one shared rig template and cooks them in order through
/// [`super::clip_intake::cook_batch`], which dedupes by digest. Layering:
/// the bake assembles, the intake cooks batches, [`retarget_cook`] cooks
/// single clips. Budget: one batch entry per input clip at offline bake
/// time, never the tick. Mismatched slice lengths are refused as `TooLarge`
/// with the widest length; licence, retarget and digest-dupe failures pass
/// through with the offending clip named.
pub fn bake_combat_batch(
    rig: &RetargetPlan<'_>,
    clips: &[&ImportedClip],
    tags: &[ClipTags],
    spdx: &[&str],
    seen: &mut Vec<u64>,
) -> Result<Vec<u64>, super::clip_intake::IntakeError> {
    let widest = clips.len().max(tags.len()).max(spdx.len());
    if tags.len() != clips.len() || spdx.len() != clips.len() {
        return Err(super::clip_intake::IntakeError::TooLarge { count: widest });
    }
    let mut entries = Vec::with_capacity(clips.len());
    for index in 0..clips.len() {
        entries.push(super::clip_intake::BatchClip {
            plan: RetargetPlan {
                source_clip: clips[index],
                ..*rig
            },
            tags: tags[index].clone(),
            spdx: spdx[index],
        });
    }
    super::clip_intake::cook_batch(&entries, seen)
}

/// Bake one traversal batch through the intake over this retarget path.
///
/// Consumer: the traversal clip bake builds per-clip plans from caller-owned
/// clips plus one shared rig template and cooks them in order through
/// [`super::traversal_cook::cook_traversal_batch`], which enforces one edge
/// marker per clip and audits to zero sidecar findings. Layering: the bake
/// assembles, the intake cooks batches, [`retarget_cook_with_sidecar`] cooks
/// single clips. Budget: one batch entry per input clip at offline bake
/// time, never the tick. Mismatched slice lengths are refused as
/// [`super::traversal_cook::TraversalError::LengthMismatch`]; licence,
/// retarget, marker and digest-dupe failures pass through with the offending
/// clip named.
pub fn bake_traversal_batch(
    rig: &RetargetPlan<'_>,
    clips: &[&ImportedClip],
    sidecars: &[&str],
    spdx: &[&str],
    seen: &mut Vec<u64>,
) -> Result<Vec<u64>, super::traversal_cook::TraversalError> {
    let mut plans = Vec::with_capacity(clips.len());
    for clip in clips {
        plans.push(RetargetPlan {
            source_clip: *clip,
            ..*rig
        });
    }
    super::traversal_cook::cook_traversal_batch(&plans, sidecars, spdx, seen)
}

#[cfg(test)]
mod tests {
    use super::super::skin_import::{
        import_clip_doc, import_skeleton_doc, ClipDoc, JointDoc, RotationKey, SkeletonDoc,
        TranslationKey,
    };
    use super::*;
    use engine_anim::contract::{validate_contacts, validate_root};

    // Synthetic Quaternius-style fixtures: a 15-joint humanoid source rig and
    // a hero target rig with renamed palms, longer legs (x1.1), longer arms
    // (x1.05) and a rest-pose twist on the left forearm. No donor bytes.
    const JOINTS: usize = 15;
    const ROOT: usize = 0;
    const SPINE: usize = 1;
    const FOOT_L: usize = 11;
    const FOOT_R: usize = 14;

    fn source_names() -> [&'static str; JOINTS] {
        [
            "root",
            "spine",
            "head",
            "upperarm_l",
            "lowerarm_l",
            "hand_l",
            "upperarm_r",
            "lowerarm_r",
            "hand_r",
            "thigh_l",
            "shin_l",
            "foot_l",
            "thigh_r",
            "shin_r",
            "foot_r",
        ]
    }

    fn target_names() -> [&'static str; JOINTS] {
        [
            "root",
            "spine",
            "head",
            "upperarm_l",
            "lowerarm_l",
            "palm_l",
            "upperarm_r",
            "lowerarm_r",
            "palm_r",
            "thigh_l",
            "shin_l",
            "foot_l",
            "thigh_r",
            "shin_r",
            "foot_r",
        ]
    }

    fn parents() -> [Option<usize>; JOINTS] {
        [
            None,
            Some(0),
            Some(1),
            Some(1),
            Some(3),
            Some(4),
            Some(1),
            Some(6),
            Some(7),
            Some(0),
            Some(9),
            Some(10),
            Some(0),
            Some(12),
            Some(13),
        ]
    }

    fn source_offsets() -> [[f32; 3]; JOINTS] {
        [
            [0.0, 0.96, 0.0],
            [0.0, 0.10, 0.0],
            [0.0, 0.24, 0.0],
            [0.20, 0.06, 0.0],
            [0.26, 0.0, 0.0],
            [0.24, 0.0, 0.0],
            [-0.20, 0.06, 0.0],
            [-0.26, 0.0, 0.0],
            [-0.24, 0.0, 0.0],
            [0.10, -0.06, 0.0],
            [0.0, -0.44, 0.0],
            [0.0, -0.44, 0.06],
            [-0.10, -0.06, 0.0],
            [0.0, -0.44, 0.0],
            [0.0, -0.44, 0.06],
        ]
    }

    fn target_offsets() -> [[f32; 3]; JOINTS] {
        let limbs = [3, 4, 5, 6, 7, 8];
        let legs = [9, 10, 11, 12, 13, 14];
        let mut offsets = source_offsets();
        for joint in limbs {
            for component in &mut offsets[joint] {
                *component *= 1.05;
            }
        }
        for joint in legs {
            for component in &mut offsets[joint] {
                *component *= 1.1;
            }
        }
        offsets
    }

    fn identity_rotations() -> [[f32; 4]; JOINTS] {
        [[0.0, 0.0, 0.0, 1.0]; JOINTS]
    }

    fn target_rotations() -> [[f32; 4]; JOINTS] {
        // Rest-pose twist on the left forearm exercises rotation correction.
        let mut rotations = identity_rotations();
        rotations[4] = [0.0, 0.0, 0.0872, 0.9962];
        rotations
    }

    fn skeleton_fixture(names: [&str; JOINTS]) -> Skeleton {
        let parents = parents();
        import_skeleton_doc(&SkeletonDoc {
            joints: names
                .iter()
                .enumerate()
                .map(|(index, name)| JointDoc {
                    name: (*name).to_string(),
                    parent: parents[index],
                })
                .collect(),
        })
        .expect("synthetic skeleton must import")
    }

    fn bind_fixture(offsets: [[f32; 3]; JOINTS], rotations: [[f32; 4]; JOINTS]) -> BindPose {
        BindPose {
            positions: offsets.to_vec(),
            rotations: rotations.to_vec(),
        }
    }

    fn map_fixture(source: &Skeleton, target: &Skeleton) -> BoneMap {
        let pairs: Vec<(String, String)> = target_names()
            .iter()
            .zip(source_names().iter())
            .map(|(t, s)| ((*t).to_string(), (*s).to_string()))
            .collect();
        let borrowed: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(t, s)| (t.as_str(), s.as_str()))
            .collect();
        BoneMap::build(source, target, &borrowed).expect("full map must build")
    }

    /// Full rig: source/target skeletons, binds and a complete bone map.
    fn rig() -> (Skeleton, Skeleton, BindPose, BindPose, BoneMap) {
        let source = skeleton_fixture(source_names());
        let target = skeleton_fixture(target_names());
        let source_bind = bind_fixture(source_offsets(), identity_rotations());
        let target_bind = bind_fixture(target_offsets(), target_rotations());
        let map = map_fixture(&source, &target);
        (source, target, source_bind, target_bind, map)
    }

    fn key3(time: f32, value: [f32; 3]) -> TranslationKey {
        TranslationKey { time, value }
    }

    fn key4(time: f32, value: [f32; 4]) -> RotationKey {
        RotationKey { time, value }
    }

    fn held(offset: [f32; 3]) -> Vec<TranslationKey> {
        vec![key3(0.0, offset)]
    }

    fn held_rot() -> Vec<RotationKey> {
        vec![key4(0.0, [0.0, 0.0, 0.0, 1.0])]
    }

    fn blank_doc(name: &str, offsets: &[[f32; 3]; JOINTS]) -> ClipDoc {
        ClipDoc {
            name: name.to_string(),
            translations: offsets.iter().map(|offset| held(*offset)).collect(),
            rotations: (0..JOINTS).map(|_| held_rot()).collect(),
        }
    }

    /// Three Quaternius-style clips on the source rig: idle sway, a walk with
    /// alternating foot lifts and root yaw, and a run with a flight phase.
    fn clip_doc(name: &str) -> ClipDoc {
        let offsets = source_offsets();
        let mut doc = blank_doc(name, &offsets);
        let foot_bind_l = offsets[FOOT_L];
        let foot_bind_r = offsets[FOOT_R];
        match name {
            "idle" => {
                doc.translations[SPINE] = vec![
                    key3(0.0, offsets[SPINE]),
                    key3(0.5, [0.0, 0.105, 0.0]),
                    key3(1.0, offsets[SPINE]),
                ];
            }
            "walk" => {
                doc.translations[ROOT] = vec![
                    key3(0.0, offsets[ROOT]),
                    key3(0.5, [0.6, 0.96, 0.0]),
                    key3(1.0, [1.2, 0.96, 0.0]),
                ];
                doc.rotations[ROOT] = vec![
                    key4(0.0, [0.0, 0.0, 0.0, 1.0]),
                    key4(0.5, [0.0, 0.025, 0.0, 0.9997]),
                    key4(1.0, [0.0, 0.0, 0.0, 1.0]),
                ];
                let lift_l = [foot_bind_l[0], foot_bind_l[1] + 0.12, foot_bind_l[2] + 0.10];
                let lift_r = [foot_bind_r[0], foot_bind_r[1] + 0.12, foot_bind_r[2] + 0.10];
                doc.translations[FOOT_L] = vec![
                    key3(0.0, foot_bind_l),
                    key3(0.5, lift_l),
                    key3(1.0, foot_bind_l),
                ];
                doc.translations[FOOT_R] =
                    vec![key3(0.0, lift_r), key3(0.5, foot_bind_r), key3(1.0, lift_r)];
            }
            "run" => {
                doc.translations[ROOT] = vec![
                    key3(0.0, offsets[ROOT]),
                    key3(0.4, [1.5, 0.98, 0.0]),
                    key3(0.8, [3.0, 0.96, 0.0]),
                ];
                let flight_l = [foot_bind_l[0], foot_bind_l[1] + 0.20, foot_bind_l[2] + 0.14];
                let flight_r = [foot_bind_r[0], foot_bind_r[1] + 0.20, foot_bind_r[2] + 0.14];
                doc.translations[FOOT_L] = vec![
                    key3(0.0, foot_bind_l),
                    key3(0.4, flight_l),
                    key3(0.8, foot_bind_l),
                ];
                doc.translations[FOOT_R] = vec![
                    key3(0.0, foot_bind_r),
                    key3(0.4, flight_r),
                    key3(0.8, foot_bind_r),
                ];
            }
            _ => panic!("unknown fixture clip"),
        }
        doc
    }

    fn cook_config() -> CookConfig {
        CookConfig {
            root_joint: ROOT,
            left_foot: FOOT_L,
            right_foot: FOOT_R,
            ground_y: -0.38,
            start_tick: 100,
        }
    }

    fn cook_fixture(name: &str) -> RetargetedClip {
        let (source, target, source_bind, target_bind, map) = rig();
        let clip = import_clip_doc(&clip_doc(name), &source).expect("fixture clip must import");
        let config = cook_config();
        let plan = RetargetPlan {
            source_clip: &clip,
            source: &source,
            source_bind: &source_bind,
            target: &target,
            target_bind: &target_bind,
            map: &map,
            config: &config,
        };
        retarget_cook(&plan, ClipTags::default()).expect("fixture cook must pass")
    }

    /// Bind-relative rotation delta of one key, for tolerance comparison.
    fn relative_delta(key: &[f32; 4], bind: &[f32; 4]) -> [f32; 4] {
        quat_mul(*key, quat_conj(bind))
    }

    /// Rotation deltas survive within 1e-4 per component and scaled offsets
    /// within 1e-5 per component on every mapped joint and key.
    fn assert_within_tolerance(
        source_clip: &ImportedClip,
        source: &Skeleton,
        source_bind: &BindPose,
        cooked: &ImportedClip,
        target: &Skeleton,
        target_bind: &BindPose,
        map: &BoneMap,
    ) {
        assert_eq!(cooked.positions.tracks.len(), target.len());
        assert_eq!(cooked.rotations.len(), target.len());
        assert_eq!(cooked.name, source_clip.name);
        assert_eq!(cooked.duration, source_clip.duration);
        for slot in 0..target.len() {
            let Some(origin) = map.source_of(slot) else {
                continue;
            };
            let scale = joint_scale(source, source_bind, target, target_bind, slot, origin);
            let source_positions = &source_clip.positions.tracks[origin];
            let target_positions = &cooked.positions.tracks[slot];
            assert_eq!(target_positions.times, source_positions.times);
            for (key, (from, to)) in source_positions
                .values
                .iter()
                .zip(target_positions.values.iter())
                .enumerate()
            {
                let base = source_bind.positions[origin];
                let anchor = target_bind.positions[slot];
                for axis in 0..3 {
                    let want = anchor[axis] + scale * (from[axis] - base[axis]);
                    assert!(
                        (to[axis] - want).abs() <= 1e-5,
                        "clip {} joint {slot} key {key}: {to:?} vs {want:?}",
                        source_clip.name
                    );
                }
            }
            let source_rots = &source_clip.rotations[origin];
            let target_rots = &cooked.rotations[slot];
            assert_eq!(target_rots.times, source_rots.times);
            for (key, (from, to)) in source_rots
                .values
                .iter()
                .zip(target_rots.values.iter())
                .enumerate()
            {
                let want = relative_delta(from, &source_bind.rotations[origin]);
                let got = relative_delta(to, &target_bind.rotations[slot]);
                for axis in 0..4 {
                    assert!(
                        (got[axis] - want[axis]).abs() <= 1e-4,
                        "clip {} joint {slot} key {key}: {got:?} vs {want:?}",
                        source_clip.name
                    );
                }
            }
        }
        let _ = source;
    }

    #[test]
    fn three_clips_retarget_within_tolerance() {
        let (source, target, source_bind, target_bind, map) = rig();
        for name in ["idle", "walk", "run"] {
            let clip = import_clip_doc(&clip_doc(name), &source).expect("fixture clip must import");
            let cooked = retarget_clip(&clip, &source, &source_bind, &target, &target_bind, &map)
                .expect("retarget must pass");
            assert_within_tolerance(
                &clip,
                &source,
                &source_bind,
                &cooked,
                &target,
                &target_bind,
                &map,
            );
        }
    }

    #[test]
    fn cook_tables_validate_and_carry_travel() {
        let walk = cook_fixture("walk");
        validate_root(&walk.root).expect("cooked root passes C4");
        validate_contacts(&walk.contacts).expect("cooked contacts pass C4");
        assert_eq!(walk.root.deltas.len(), 60);
        assert_eq!(walk.root.deltas[0].tick, 100);
        // Loop-aware travel: the first n-1 deltas sum to last-minus-first.
        let travel_x: f32 = walk.root.deltas[..59].iter().map(|delta| delta.dp[0]).sum();
        assert!((travel_x - 1.18).abs() < 1e-4, "travel {travel_x}");
        assert_eq!(walk.contacts.contacts.len(), 120);
        for foot in [0u8, 1u8] {
            let down = walk
                .contacts
                .contacts
                .iter()
                .filter(|contact| contact.foot == foot && contact.down)
                .count();
            assert!(
                down > 0 && down < 60,
                "walk foot {foot} alternates: {down}/60"
            );
        }

        let idle = cook_fixture("idle");
        assert_eq!(idle.root.deltas.len(), 60);
        let idle_travel: f32 = idle.root.deltas[..59].iter().map(|delta| delta.dp[0]).sum();
        assert!(idle_travel.abs() < 1e-6, "idle travel {idle_travel}");
        assert!(
            idle.contacts.contacts.iter().all(|contact| contact.down),
            "idle feet never lift"
        );

        let run = cook_fixture("run");
        assert_eq!(run.root.deltas.len(), 48);
        let run_travel: f32 = run.root.deltas[..47].iter().map(|delta| delta.dp[0]).sum();
        assert!(
            (run_travel - 2.9375).abs() < 1e-3,
            "run travel {run_travel}"
        );
    }

    #[test]
    fn cook_identical_twice() {
        for name in ["idle", "walk", "run"] {
            let first = cook_fixture(name);
            let second = cook_fixture(name);
            assert_eq!(first, second, "{name} cooks identically twice");
            assert_eq!(
                retarget_digest(&first),
                retarget_digest(&second),
                "{name} digest stable"
            );
        }
    }

    #[test]
    fn digest_sensitive_to_input() {
        let base = cook_fixture("walk");
        let digest = retarget_digest(&base);
        let mut touched = base.clone();
        touched.clip.positions.tracks[ROOT].values[0][0] += 0.001;
        assert_ne!(
            retarget_digest(&touched),
            digest,
            "position bit flips digest"
        );
        let mut tagged = base.clone();
        tagged.tags.looped = true;
        assert_ne!(retarget_digest(&tagged), digest, "tag bit flips digest");
    }

    #[test]
    fn unmapped_joints_hold_bind() {
        let (source, target, source_bind, target_bind, _) = rig();
        let pairs: Vec<(String, String)> = target_names()
            .iter()
            .zip(source_names().iter())
            .filter(|(name, _)| **name != "head")
            .map(|(t, s)| ((*t).to_string(), (*s).to_string()))
            .collect();
        let borrowed: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(t, s)| (t.as_str(), s.as_str()))
            .collect();
        let map = BoneMap::build(&source, &target, &borrowed).expect("partial map must build");
        assert_eq!(map.source_of(2), None, "head holds bind");
        let clip = import_clip_doc(&clip_doc("walk"), &source).expect("fixture clip must import");
        let cooked = retarget_clip(&clip, &source, &source_bind, &target, &target_bind, &map)
            .expect("partial retarget must pass");
        assert_eq!(cooked.positions.tracks[2].times, vec![0.0]);
        assert_eq!(
            cooked.positions.tracks[2].values,
            vec![target_bind.positions[2]]
        );
        assert_eq!(cooked.rotations[2].times, vec![0.0]);
        assert_eq!(cooked.rotations[2].values, vec![target_bind.rotations[2]]);
    }

    #[test]
    fn missing_and_duplicate_bones_refused() {
        let (source, target, _, _, _) = rig();
        assert_eq!(
            BoneMap::build(&source, &target, &[("nope", "root")]),
            Err(RetargetError::MissingBone {
                name: "nope".to_string()
            })
        );
        assert_eq!(
            BoneMap::build(&source, &target, &[("head", "nope_src")]),
            Err(RetargetError::MissingBone {
                name: "nope_src".to_string()
            })
        );
        assert_eq!(
            BoneMap::build(&source, &target, &[("head", "head"), ("head", "spine")]),
            Err(RetargetError::DuplicateBone {
                name: "head".to_string()
            })
        );
    }

    #[test]
    fn bad_binds_refused() {
        let (source, target, source_bind, target_bind, map) = rig();
        let clip = import_clip_doc(&clip_doc("idle"), &source).expect("fixture clip must import");
        let mut finite = source_bind.clone();
        finite.positions[3][0] = f32::NAN;
        assert_eq!(
            retarget_clip(&clip, &source, &finite, &target, &target_bind, &map),
            Err(RetargetError::NonFinite("bind.positions"))
        );
        let mut flat = target_bind.clone();
        flat.rotations[4] = [0.0, 0.0, 0.0, 0.0];
        assert_eq!(
            retarget_clip(&clip, &source, &source_bind, &target, &flat, &map),
            Err(RetargetError::BadBind { joint: 4 })
        );
        let short = BindPose {
            positions: vec![[0.0; 3]],
            rotations: vec![[0.0, 0.0, 0.0, 1.0]],
        };
        assert_eq!(
            retarget_clip(&clip, &source, &short, &target, &target_bind, &map),
            Err(RetargetError::BindMismatch {
                expected: JOINTS,
                got: 1
            })
        );
        let mut renamed = target.clone();
        renamed.joints[0].name = "Bad Name".to_string();
        assert_eq!(
            retarget_clip(&clip, &source, &source_bind, &renamed, &target_bind, &map),
            Err(RetargetError::BadSkeleton { joint: 0 })
        );
        let short_map = BoneMap {
            target_to_source: vec![Some(0)],
        };
        assert_eq!(
            retarget_clip(
                &clip,
                &source,
                &source_bind,
                &target,
                &target_bind,
                &short_map
            ),
            Err(RetargetError::MapMismatch {
                expected: JOINTS,
                got: 1
            })
        );
    }

    #[test]
    fn bad_source_clips_refused() {
        let (source, target, source_bind, target_bind, map) = rig();
        let clip = import_clip_doc(&clip_doc("idle"), &source).expect("fixture clip must import");
        let mut flat = clip.clone();
        flat.rotations[3].values[0] = [0.0, 0.0, 0.0, 0.0];
        assert_eq!(
            retarget_clip(&flat, &source, &source_bind, &target, &target_bind, &map),
            Err(RetargetError::BadQuat { joint: 3, key: 0 })
        );
        let mut short = clip.clone();
        short.positions.tracks.pop();
        assert_eq!(
            retarget_clip(&short, &source, &source_bind, &target, &target_bind, &map),
            Err(RetargetError::ClipLengthMismatch {
                expected: JOINTS,
                got: JOINTS - 1
            })
        );
        let mut unsorted = clip.clone();
        unsorted.positions.tracks[SPINE].times[1] = 0.0;
        assert_eq!(
            retarget_clip(
                &unsorted,
                &source,
                &source_bind,
                &target,
                &target_bind,
                &map
            ),
            Err(RetargetError::BadClip { joint: SPINE })
        );
        let mut wild = clip;
        wild.duration = f32::NAN;
        assert_eq!(
            retarget_clip(&wild, &source, &source_bind, &target, &target_bind, &map),
            Err(RetargetError::NonFinite("clip.duration"))
        );
    }

    #[test]
    fn bad_cook_configs_refused() {
        let (source, target, source_bind, target_bind, map) = rig();
        let clip = import_clip_doc(&clip_doc("idle"), &source).expect("fixture clip must import");
        let base = cook_config();
        for config in [
            CookConfig {
                root_joint: 99,
                ..base
            },
            CookConfig {
                left_foot: 99,
                ..base
            },
            CookConfig {
                right_foot: 99,
                ..base
            },
            CookConfig {
                ground_y: f32::NAN,
                ..base
            },
        ] {
            let plan = RetargetPlan {
                source_clip: &clip,
                source: &source,
                source_bind: &source_bind,
                target: &target,
                target_bind: &target_bind,
                map: &map,
                config: &config,
            };
            assert!(
                retarget_cook(&plan, ClipTags::default()).is_err(),
                "bad config refused"
            );
        }
        assert_eq!(
            retarget_cook(
                &RetargetPlan {
                    source_clip: &clip,
                    source: &source,
                    source_bind: &source_bind,
                    target: &target,
                    target_bind: &target_bind,
                    map: &map,
                    config: &CookConfig {
                        root_joint: 99,
                        ..base
                    },
                },
                ClipTags::default()
            ),
            Err(RetargetError::UnknownJoint { joint: 99 })
        );
    }

    fn full_sidecar() -> &'static str {
        r#"{
            "clip": "walk",
            "loop": true,
            "root_motion": true,
            "tags": ["locomotion", "walk"],
            "ground_y": -0.30,
            "feet": {"left": "foot_l", "right": "foot_r"},
            "hit_windows": [{"socket": "SOCKET_hand_r", "start_s": 0.2, "end_s": 0.35}],
            "traversal": [{"edge": "vault", "time_s": 0.5}],
            "future_field": [1, 2, 3]
        }"#
    }

    #[test]
    fn sidecar_full_parse() {
        let tags = parse_sidecar(full_sidecar()).expect("full sidecar must parse");
        assert_eq!(tags.clip, "walk");
        assert!(tags.looped);
        assert!(tags.root_motion);
        assert_eq!(
            tags.tags,
            vec!["locomotion".to_string(), "walk".to_string()]
        );
        assert_eq!(tags.ground_y, Some(-0.30));
        assert_eq!(
            tags.feet,
            Some(("foot_l".to_string(), "foot_r".to_string()))
        );
        assert_eq!(
            tags.hit_windows,
            vec![HitWindow {
                socket: "SOCKET_hand_r".to_string(),
                start_s: 0.2,
                end_s: 0.35,
            }]
        );
        assert_eq!(
            tags.traversal,
            vec![TraversalMarker {
                edge: "vault".to_string(),
                time_s: 0.5,
            }]
        );
        // Byte and str entry points agree; minimal docs take defaults.
        assert_eq!(
            tags,
            parse_sidecar_bytes(full_sidecar().as_bytes()).expect("bytes parse")
        );
        let minimal = parse_sidecar("{}").expect("empty object takes defaults");
        assert_eq!(minimal, ClipTags::default());
        let nulled = parse_sidecar(r#"{"tags": null, "feet": null}"#).expect("nulls are missing");
        assert_eq!(nulled, ClipTags::default());
    }

    #[test]
    fn sidecar_cook_applies_overrides() {
        let (source, target, source_bind, target_bind, map) = rig();
        let clip = import_clip_doc(&clip_doc("walk"), &source).expect("fixture clip must import");
        let config = cook_config();
        let plan = RetargetPlan {
            source_clip: &clip,
            source: &source,
            source_bind: &source_bind,
            target: &target,
            target_bind: &target_bind,
            map: &map,
            config: &config,
        };
        let plain = retarget_cook(&plan, ClipTags::default()).expect("plain cook must pass");
        let plain_down = plain
            .contacts
            .contacts
            .iter()
            .filter(|contact| contact.foot == 0 && contact.down)
            .count();
        assert!(
            plain_down > 0 && plain_down < 60,
            "plain walk alternates: {plain_down}/60"
        );
        // The sidecar threshold (-0.30) sits above the lifted foot (-0.352),
        // so every sample reads planted.
        let tagged =
            retarget_cook_with_sidecar(&plan, full_sidecar()).expect("sidecar cook must pass");
        let tagged_down = tagged
            .contacts
            .contacts
            .iter()
            .filter(|contact| contact.foot == 0 && contact.down)
            .count();
        assert_eq!(tagged_down, 60, "override threshold plants every sample");
        assert!(tagged.tags.looped);
        assert_eq!(tagged.tags.ground_y, Some(-0.30));
        // Unknown sidecar feet resolve against the target skeleton, strictly.
        let err =
            retarget_cook_with_sidecar(&plan, r#"{"feet": {"left": "toe_l", "right": "foot_r"}}"#)
                .expect_err("unknown sidecar foot must fail");
        assert_eq!(
            err,
            RetargetError::MissingBone {
                name: "toe_l".to_string()
            }
        );
    }

    #[test]
    fn sidecar_traversal_vocabulary() {
        for edge in TRAVERSAL_EDGES {
            assert!(is_traversal_edge(edge), "{edge} is a link-layer edge");
            let doc = format!(r#"{{"traversal": [{{"edge": "{edge}", "time_s": 0.5}}]}}"#);
            let tags = parse_sidecar(&doc).expect("closed-vocabulary edge must parse");
            assert_eq!(
                tags.traversal,
                vec![TraversalMarker {
                    edge: edge.to_string(),
                    time_s: 0.5,
                }]
            );
        }
        assert!(!is_traversal_edge("mantle_low"));
        assert!(!is_traversal_edge(""));
        assert!(
            parse_sidecar(r#"{"traversal": [{"edge": "mantle_low", "time_s": 0.5}]}"#).is_err(),
            "free-form edge refused"
        );
    }

    #[test]
    fn sidecar_consumer_combat_reads_windows() {
        // Dual-wield shape: two ordered, disjoint strike windows on one socket.
        let tags = parse_sidecar(
            r#"{"clip": "attack_dual_1", "hit_windows": [
            {"socket": "SOCKET_wep_R", "start_s": 0.08, "end_s": 0.22},
            {"socket": "SOCKET_wep_R", "start_s": 0.24, "end_s": 0.34}]}"#,
        )
        .expect("dual windows must parse");
        assert_eq!(tags.hit_windows.len(), 2);
        let mut prev_end = 0.0;
        for window in &tags.hit_windows {
            assert!(
                window.socket.starts_with("SOCKET_"),
                "combat reads sockets, got {}",
                window.socket
            );
            assert!(
                window.start_s >= prev_end && window.start_s < window.end_s,
                "windows ordered and disjoint"
            );
            assert!(
                window.end_s - window.start_s >= 1.0f32 / 60.0f32,
                "window covers a 60 Hz tick"
            );
            prev_end = window.end_s;
        }
        // A strike doc with no windows carries nothing for the combat step.
        let bare = parse_sidecar(r#"{"clip": "attack_probe"}"#).expect("winless doc parses");
        assert!(bare.hit_windows.is_empty());
    }

    #[test]
    fn sidecar_consumer_loco_reads_contacts() {
        let (source, target, source_bind, target_bind, map) = rig();
        let clip = import_clip_doc(&clip_doc("walk"), &source).expect("fixture clip must import");
        let config = cook_config();
        let plan = RetargetPlan {
            source_clip: &clip,
            source: &source,
            source_bind: &source_bind,
            target: &target,
            target_bind: &target_bind,
            map: &map,
            config: &config,
        };
        let cooked =
            retarget_cook_with_sidecar(&plan, full_sidecar()).expect("sidecar cook must pass");
        // Locomotion shape: loop plus root-motion flags ride along, tables validate.
        assert!(cooked.tags.looped);
        assert!(cooked.tags.root_motion);
        validate_contacts(&cooked.contacts).expect("cooked contacts validate");
        validate_root(&cooked.root).expect("cooked root validates");
        let mut feet: Vec<u8> = cooked
            .contacts
            .contacts
            .iter()
            .map(|contact| contact.foot)
            .collect();
        feet.sort_unstable();
        feet.dedup();
        assert_eq!(feet, vec![0, 1], "both feet sampled");
        let ticks: Vec<u32> = cooked
            .contacts
            .contacts
            .iter()
            .map(|contact| contact.tick)
            .collect();
        assert!(
            ticks.windows(2).all(|pair| pair[0] <= pair[1]),
            "contact ticks ordered"
        );
    }

    #[test]
    fn sidecar_canonical_audit() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/fighters/sidecars");
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .expect("sidecar dir must list")
            .map(|entry| {
                entry
                    .expect("dir entry must read")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.ends_with(".json"))
            .collect();
        names.sort();
        assert!(names.len() >= 14, "canonical sidecar set, got {names:?}");
        let (source, target, source_bind, target_bind, map) = rig();
        let clip = import_clip_doc(&clip_doc("walk"), &source).expect("fixture clip must import");
        let config = cook_config();
        let plan = RetargetPlan {
            source_clip: &clip,
            source: &source,
            source_bind: &source_bind,
            target: &target,
            target_bind: &target_bind,
            map: &map,
            config: &config,
        };
        for name in &names {
            let bytes = std::fs::read(dir.join(name)).expect("sidecar must read");
            let text = std::str::from_utf8(&bytes).expect("sidecar is utf-8");
            let tags = parse_sidecar(text).expect("canonical sidecar must parse");
            assert_eq!(tags, parse_sidecar(text).expect("parse repeats"));
            let stem = name.strip_suffix(".json").expect("json suffix");
            assert_eq!(tags.clip, stem, "sidecar names its clip");
            if stem.starts_with("attack") {
                assert!(!tags.hit_windows.is_empty(), "{stem} carries windows");
                let mut prev_end = 0.0;
                for window in &tags.hit_windows {
                    assert!(window.socket.starts_with("SOCKET_"), "{stem} socket shape");
                    assert!(
                        window.start_s >= prev_end && window.start_s < window.end_s,
                        "{stem} windows ordered"
                    );
                    prev_end = window.end_s;
                }
            }
            for marker in &tags.traversal {
                assert!(is_traversal_edge(&marker.edge), "closed edge vocabulary");
                assert!(marker.time_s >= 0.0, "marker time at or after 0");
            }
            let first =
                retarget_cook_with_sidecar(&plan, text).expect("canonical sidecar must cook");
            let second = retarget_cook_with_sidecar(&plan, text).expect("cook repeats");
            assert_eq!(
                retarget_digest(&first),
                retarget_digest(&second),
                "{stem} cook bytes x2"
            );
        }
    }

    #[test]
    fn sidecar_refusals() {
        for (label, doc) in [
            ("empty", ""),
            ("garbage", "not json at all"),
            ("array", "[1, 2]"),
            ("trailing", "{} {}"),
            ("bad-bool", r#"{"loop": 1}"#),
            ("bad-tags", r#"{"tags": "walk"}"#),
            ("bad-token", r#"{"tags": ["Bad-Name!"]}"#),
            ("dupe-tag", r#"{"tags": ["walk", "walk"]}"#),
            (
                "too-many-tags",
                &format!("{{\"tags\": [{}]}}", r#""t","#.repeat(33)),
            ),
            ("bad-ground", r#"{"ground_y": "high"}"#),
            ("half-feet", r#"{"feet": {"left": "foot_l"}}"#),
            (
                "bad-foot",
                r#"{"feet": {"left": "Foot L", "right": "foot_r"}}"#,
            ),
            (
                "bad-window",
                r#"{"hit_windows": [{"socket": "s", "start_s": 0.5, "end_s": 0.2}]}"#,
            ),
            (
                "window-no-socket",
                r#"{"hit_windows": [{"start_s": 0.1, "end_s": 0.2}]}"#,
            ),
            (
                "bad-edge",
                r#"{"traversal": [{"edge": "vault", "time_s": -1}]}"#,
            ),
            (
                "bad-edge-name",
                r#"{"traversal": [{"edge": "mantle_low", "time_s": 0.5}]}"#,
            ),
            ("edge-no-time", r#"{"traversal": [{"edge": "vault"}]}"#),
            ("dupe-key", r#"{"loop": true, "loop": false}"#),
            ("unclosed", r#"{"tags": ["#),
            ("bad-number", r#"{"ground_y": 01}"#),
            ("bad-escape", "{\"tags\": [\"wa\\x\"]}"),
        ] {
            let doc: &str = doc;
            assert!(
                parse_sidecar(doc).is_err(),
                "{label} sidecar must be refused"
            );
        }
        let oversize = " ".repeat(MAX_SIDECAR_BYTES + 1);
        assert_eq!(
            parse_sidecar(&oversize),
            Err(RetargetError::TooLarge {
                count: MAX_SIDECAR_BYTES + 1
            })
        );
        assert_eq!(
            parse_sidecar_bytes(&[0xFFu8; 8]),
            Err(RetargetError::BadSidecar {
                detail: "want utf-8".to_string()
            })
        );
        assert!(
            parse_sidecar_bytes(full_sidecar().as_bytes()).is_ok(),
            "bytes entry parses"
        );
    }

    #[test]
    fn bake_combat_batch_cooks_through_intake() {
        let (source, target, source_bind, target_bind, map) = rig();
        let config = cook_config();
        let walk = import_clip_doc(&clip_doc("walk"), &source).expect("fixture clip must import");
        let run = import_clip_doc(&clip_doc("run"), &source).expect("fixture clip must import");
        let rig_plan = RetargetPlan {
            source_clip: &walk,
            source: &source,
            source_bind: &source_bind,
            target: &target,
            target_bind: &target_bind,
            map: &map,
            config: &config,
        };
        let clips = [&walk, &run];
        let tags = [ClipTags::default(), ClipTags::default()];
        let spdx = [
            super::super::clip_intake::EXPECTED_SPDX,
            super::super::clip_intake::EXPECTED_SPDX,
        ];
        let mut seen = Vec::new();
        let digests =
            bake_combat_batch(&rig_plan, &clips, &tags, &spdx, &mut seen).expect("bake must cook");
        assert_eq!(digests.len(), 2);
        assert_ne!(digests[0], digests[1]);
        assert_eq!(seen, digests);
        // Same batch against the same seen dedupes by digest, first clip named.
        let mut replay = seen.clone();
        assert!(matches!(
            bake_combat_batch(&rig_plan, &clips, &tags, &spdx, &mut replay),
            Err(super::super::clip_intake::IntakeError::Duplicate { .. })
        ));
        // Mismatched slices are refused, never half-cooked.
        assert!(matches!(
            bake_combat_batch(&rig_plan, &clips[..1], &tags, &spdx, &mut Vec::new()),
            Err(super::super::clip_intake::IntakeError::TooLarge { .. })
        ));
    }

    #[test]
    fn bake_traversal_batch_cooks_eighteen_with_zero_findings() {
        use super::super::traversal_cook::{
            audit_sidecars, traversal_clip_names, traversal_sidecar, TraversalError,
            EXPECTED_SPDX as TRAVERSAL_SPDX,
        };
        let names = traversal_clip_names();
        assert_eq!(names.len(), 18);
        let (source, target, source_bind, target_bind, map) = rig();
        let config = cook_config();
        let offsets = source_offsets();
        let mut owned = Vec::with_capacity(names.len());
        for name in &names {
            owned.push(
                import_clip_doc(&blank_doc(name, &offsets), &source)
                    .expect("fixture clip must import"),
            );
        }
        let first = import_clip_doc(&clip_doc("walk"), &source).expect("fixture clip must import");
        let rig_plan = RetargetPlan {
            source_clip: &first,
            source: &source,
            source_bind: &source_bind,
            target: &target,
            target_bind: &target_bind,
            map: &map,
            config: &config,
        };
        let clips: Vec<&ImportedClip> = owned.iter().collect();
        let sidecars: Vec<String> = names
            .iter()
            .enumerate()
            .map(|(slot, name)| traversal_sidecar(name, slot))
            .collect();
        let borrowed: Vec<&str> = sidecars.iter().map(String::as_str).collect();
        let spdx = [TRAVERSAL_SPDX; 18];
        let mut seen = Vec::new();
        let digests = bake_traversal_batch(&rig_plan, &clips, &borrowed, &spdx, &mut seen)
            .expect("traversal bake must cook");
        assert_eq!(digests.len(), 18);
        assert_eq!(seen, digests);
        assert!(
            audit_sidecars(&borrowed).is_empty(),
            "traversal bake audits to 0 findings"
        );
        let mut replay = seen.clone();
        assert!(matches!(
            bake_traversal_batch(&rig_plan, &clips, &borrowed, &spdx, &mut replay),
            Err(TraversalError::Duplicate { .. })
        ));
        assert!(matches!(
            bake_traversal_batch(&rig_plan, &clips[..1], &borrowed, &spdx, &mut Vec::new()),
            Err(TraversalError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn errors_display_bounded() {
        for error in [
            RetargetError::MissingBone {
                name: "toe_l".to_string(),
            },
            RetargetError::DuplicateBone {
                name: "head".to_string(),
            },
            RetargetError::BadSkeleton { joint: 0 },
            RetargetError::MapMismatch {
                expected: 15,
                got: 1,
            },
            RetargetError::BindMismatch {
                expected: 15,
                got: 1,
            },
            RetargetError::BadBind { joint: 4 },
            RetargetError::BadClip { joint: 1 },
            RetargetError::ClipLengthMismatch {
                expected: 15,
                got: 14,
            },
            RetargetError::BadQuat { joint: 1, key: 0 },
            RetargetError::BadSidecar {
                detail: "byte 3: want value".to_string(),
            },
            RetargetError::BadRoot { index: 4 },
            RetargetError::BadContact { index: 4 },
            RetargetError::UnknownJoint { joint: 99 },
            RetargetError::TooLarge { count: 1 },
            RetargetError::NonFinite("clip.values"),
        ] {
            assert!(!format!("{error}").is_empty());
        }
        let long = "x".repeat(4096);
        let error = RetargetError::BadSidecar {
            detail: long.clone(),
        };
        assert!(format!("{error}").chars().count() <= MAX_SIDECAR_DETAIL_CHARS + 64);
        let error = RetargetError::MissingBone { name: long };
        assert!(format!("{error}").chars().count() <= MAX_SIDECAR_DETAIL_CHARS + 64);
    }
}
