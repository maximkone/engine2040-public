//! A baked combat pack, played on the game's CT7 body.
//!
//! Why this file exists: the engine bakes a teacher's clip into an `.am5p`
//! pack (see `engine_anim::motion_bake`). That pack is written against the
//! engine's **fighter rig** (65 SOMA-named joints). The game plays a **CT7**
//! body (44 Mixamo-named joints), and it loads its clips as glTF clip
//! sources, not as packs. So a baked combat pack had no way into the game:
//! the loader was missing, not the motion.
//!
//! This module is that loader. It reads a pack, lifts its tracks into a
//! source clip on the fighter rig, and retargets that clip onto the CT7 body
//! through [`crate::ct7_map::ct7_map`], which is the one place the two
//! naming schemes are bridged. The result is an [`ImportedClip`]: exactly
//! the shape the game already plays, so the look needs no new branch.
//!
//! Units: the pack carries metres and the fighter rig's bind is in metres;
//! the CT7 body is in metres too. Times are seconds.
//!
//! Determinism: the same bytes give the same clip. Nothing here reads a
//! clock, a random source, or the environment.
//!
//! Errors: [`PackClipError`] at every boundary; a pack that does not fit the
//! body fails loud, never plays wrong motion.

use engine_anim::clip::QTrack;
use engine_anim::contract::Skeleton;
use engine_anim::motion_bake::bake_unpack_rots;
use engine_anim::pack::{unpack_qclip, PackedClip};
use engine_anim::pack_file::{load_pack, PackFileError};
use engine_anim::pose::QuatTrack;
use engine_anim::{Clip, Track};

use crate::fighter::{fighter_bind, fighter_skeleton};
use crate::retarget::{retarget_clip, BindPose, RetargetError};
use crate::skin_import::ImportedClip;

/// The pack loader's name for a clip, used when the pack carries no better one.
pub const DEFAULT_CLIP_NAME: &str = "combat";

/// Why a pack could not be played on a body.
#[derive(Debug)]
pub enum PackClipError {
    /// The pack file could not be read or decoded.
    Pack(PackFileError),
    /// The fighter rig could not be built; reason carried.
    Rig(String),
    /// The pack does not fit the fighter rig; reason carried.
    Retarget(RetargetError),
    /// The body has no skeleton to play on.
    NoBody,
    /// The pack's tracks do not dequantize into the fighter rig's joint count.
    TrackCount {
        /// Tracks the pack holds.
        got: usize,
        /// Joints the fighter rig has.
        expected: usize,
    },
}

impl std::fmt::Display for PackClipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pack(why) => write!(f, "combat pack: {why}"),
            Self::Rig(why) => write!(f, "combat pack: the fighter rig is not usable: {why}"),
            Self::Retarget(why) => write!(f, "combat pack: retarget onto the body refused: {why:?}"),
            Self::NoBody => write!(f, "combat pack: the body has no skeleton to play on"),
            Self::TrackCount { got, expected } => write!(
                f,
                "combat pack: the pack holds {got} tracks, the fighter rig has {expected} joints, it was baked for another rig"
            ),
        }
    }
}

impl std::error::Error for PackClipError {}

impl From<PackFileError> for PackClipError {
    fn from(cause: PackFileError) -> Self {
        Self::Pack(cause)
    }
}

impl From<RetargetError> for PackClipError {
    fn from(cause: RetargetError) -> Self {
        Self::Retarget(cause)
    }
}

/// Read a baked pack and play it on the given body skeleton.
///
/// The body is the CT7 skeleton (44 sanitized Mixamo names). The pack is
/// retargeted through [`crate::ct7_map::ct7_map`], so a CT7 joint with no
/// fighter counterpart keeps its bind pose and a pack track with no CT7
/// counterpart is dropped, as the retarget documents.
///
/// Native only: the pack is read from disk. A browser gets its bytes from a
/// fetched bundle and uses [`clip_from_pack_bytes`].
#[cfg(not(target_arch = "wasm32"))]
pub fn clip_from_pack_file(
    path: &std::path::Path,
    body: &Skeleton,
    body_bind: &BindPose,
) -> Result<ImportedClip, PackClipError> {
    let loaded = load_pack(path)?;
    clip_from_pack_bytes(&loaded.bytes, body, body_bind)
}

/// Read a baked pack's bytes and play them on the given body skeleton.
///
/// This is the browser door: the same bytes, the same clip, no file system.
pub fn clip_from_pack_bytes(
    bytes: &[u8],
    body: &Skeleton,
    body_bind: &BindPose,
) -> Result<ImportedClip, PackClipError> {
    let rig = fighter_skeleton().map_err(|err| PackClipError::Rig(err.to_string()))?;
    let rig_bind = fighter_bind().map_err(|err| PackClipError::Rig(err.to_string()))?;
    let source = source_clip(bytes, &rig)?;
    let map = crate::ct7_map::ct7_map(body)?;
    retarget_clip(&source, &rig, &rig_bind, body, body_bind, &map).map_err(PackClipError::from)
}

/// Lift a pack's bytes into a source clip on the fighter rig.
///
/// The pack holds one track per rig joint, in the rig's own joint order, so
/// the source clip needs no remapping. The rotations come out of the pack's
/// own rotation lanes: that is the whole point of the version-2 wire, and a
/// version-1 pack (no rotations) is refused rather than played on bind
/// rotations, which would slide the arms.
fn source_clip(bytes: &[u8], rig: &Skeleton) -> Result<ImportedClip, PackClipError> {
    // The pack is decoded twice on purpose: the positions come from the
    // packed clip (the same decode the anim crate's own sampler uses) and the
    // rotations from the pack's rotation lanes. Both decoders refuse a pack
    // that does not re-encode to the same bytes, so a cut or edited file
    // never plays.
    let packed =
        PackedClip::from_bytes(bytes).map_err(|err| PackClipError::Rig(err.to_string()))?;
    let clip = unpack_qclip(&packed);
    let rotations = bake_unpack_rots(bytes).map_err(|err| PackClipError::Rig(err.to_string()))?;
    if clip.tracks.len() != rig.len() {
        return Err(PackClipError::TrackCount {
            got: clip.tracks.len(),
            expected: rig.len(),
        });
    }
    if rotations.len() != rig.len() {
        return Err(PackClipError::TrackCount {
            got: rotations.len(),
            expected: rig.len(),
        });
    }
    let mut positions = Vec::with_capacity(rig.len());
    for track in &clip.tracks {
        positions.push(Track {
            times: track.times.clone(),
            values: dequantize(track),
        });
    }
    let mut duration = clip.dur;
    let mut rot_tracks = Vec::with_capacity(rig.len());
    for track in &rotations {
        rot_tracks.push(QuatTrack {
            times: track.times.clone(),
            values: track.values.clone(),
        });
        if let Some(end) = track.times.last() {
            if *end > duration {
                duration = *end;
            }
        }
    }
    Ok(ImportedClip {
        name: DEFAULT_CLIP_NAME.to_string(),
        duration,
        positions: Clip { tracks: positions },
        rotations: rot_tracks,
    })
}

/// The position values of one packed track, dequantized.
///
/// The wire stores each lane as a 16-bit key with the track's own min and
/// max, so the value is `min + (max - min) * key / 65535`. The same formula
/// the anim crate's own sampler uses, so the clip this builds samples to the
/// same numbers the pack holds.
fn dequantize(track: &QTrack) -> Vec<[f32; 3]> {
    track
        .keys
        .iter()
        .map(|key| {
            [
                lane(track, key[0], 0),
                lane(track, key[1], 1),
                lane(track, key[2], 2),
            ]
        })
        .collect()
}

/// One lane of one key: the dequantized value.
fn lane(track: &QTrack, key: u16, axis: usize) -> f32 {
    let min = track.min[axis];
    let max = track.max[axis];
    min + (max - min) * (f32::from(key) / 65535.0)
}
