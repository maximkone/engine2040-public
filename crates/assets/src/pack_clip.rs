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
//! source clip on the fighter rig, and transfers that motion onto the CT7
//! body through [`crate::ct7_map::ct7_map`], which is the one place the two
//! naming schemes are bridged. The result is an [`ImportedClip`]: exactly
//! the shape the game already plays, so the look needs no new branch.
//!
//! The transfer is its own, not [`crate::retarget::retarget_clip`], and the
//! reason matters: the two rigs are different shapes with different joint
//! sets. A retarget that re-expresses a local offset against a local offset
//! — scaled by a bone-length ratio — is only meaningful when the two rigs
//! have the same bones and the same bind frame. The CT7 has 44 joints to the
//! fighter's 65, so its head bone covers the fighter's neck *and* head
//! together, and a shared retarget either inflates that difference or drops
//! the body through the floor.
//!
//! So this module transplants the motion in the one frame both rigs share —
//! the **world** — and lets the CT7's own frame carry the shape:
//!
//! 1. the pack's rotation lanes compose down the fighter chain into the
//!    teacher's **world** rotations, one per joint per key;
//! 2. the CT7's bind rotations compose down the CT7 chain into its own
//!    **bind world** rotations;
//! 3. a joint's posed world rotation is the teacher's world rotation
//!    composed with the CT7's bind world rotation: the CT7's own rest frame
//!    turned by the teacher's own motion. The teacher's rest is the identity
//!    (its bind rotations are the identity), so at the rest the CT7 holds
//!    its bind pose;
//! 4. the CT7's local rotation is the parent's posed world rotation turned
//!    back out, so the game's skinning composes the same world rotation
//!    again;
//! 5. the local offset is the CT7's **own bind offset**, one lane per key:
//!    the pose rides the rotations, and the CT7's own proportions (its
//!    shorter limbs, its single head bone) decide where a turned bone ends
//!    up, so the head stays near the CT7's head height and the feet near
//!    its ground;
//! 6. the one lane that carries translation is the **body root**: the
//!    teacher's world travel (read from the pack's root lane) sits on the
//!    CT7's own root bind position, so a lunge steps the body forward on
//!    the CT7's stance.
//!//! A joint the CT7 has no counterpart for holds its bind pose; a pack track
//! with no CT7 counterpart is dropped, exactly as the map documents.
//!
//! Units: the pack carries metres and the fighter rig's bind is in metres;
//! the CT7 body is in metres too. Times are seconds.
//!
//! Determinism: the same bytes give the same clip. Nothing here reads a
//! clock, a random source, or the environment.
//!
//! Errors: [`PackClipError`] at every boundary; a pack that does not fit the
//! body fails loud, never plays wrong motion.

use engine_anim::contract::Skeleton;
use engine_anim::motion_bake::bake_unpack_rots;
use engine_anim::pack::{unpack_qclip, PackedClip};
use engine_anim::pack_file::PackFileError;
// `load_pack` reads the pack from disk, so it is native-only; the error type
// beside it stays ungated because `PackClipError` carries it on every target.
// Importing the pair together broke every wasm build: a browser gets its bytes
// from a fetched bundle through `clip_from_pack_bytes`.
#[cfg(not(target_arch = "wasm32"))]
use engine_anim::pack_file::load_pack;
use engine_anim::pose::QuatTrack;
use engine_anim::{Clip, Track};
use engine_core::quat_mul;

use crate::fighter::fighter_skeleton;
use crate::retarget::BindPose;
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
    /// The body has no skeleton to play on.
    NoBody,
    /// The pack's tracks do not fit the fighter rig's joint count.
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

impl From<crate::retarget::RetargetError> for PackClipError {
    fn from(cause: crate::retarget::RetargetError) -> Self {
        Self::Rig(cause.to_string())
    }
}

/// Read a baked pack and play it on the given body.
///
/// The body is the CT7 skeleton (44 sanitized Mixamo names) and its bind
/// pose. The pack is transferred through [`crate::ct7_map::ct7_map`], so a
/// CT7 joint with no fighter counterpart keeps its bind pose and a pack
/// track with no CT7 counterpart is dropped, as the map documents.
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
    if body.joints.is_empty() {
        return Err(PackClipError::NoBody);
    }
    let rig = fighter_skeleton().map_err(|err| PackClipError::Rig(err.to_string()))?;
    let rig_bind =
        crate::fighter::fighter_bind().map_err(|err| PackClipError::Rig(err.to_string()))?;
    let (clip, rotations, duration) = unpack_pack(bytes, &rig)?;
    transfer(
        &clip, &rotations, duration, &rig, &rig_bind, body, body_bind,
    )
}

/// The pack's tracks: its local positions, its local rotations and its
/// duration, all on the fighter rig.
///
/// The pack is decoded twice on purpose: the positions from the packed clip
/// and the rotations from the pack's own rotation lanes. Both decoders
/// refuse a pack that does not re-encode to the same bytes, so a cut or
/// edited file never plays.
fn unpack_pack(bytes: &[u8], rig: &Skeleton) -> Result<(Clip, Vec<QuatTrack>, f32), PackClipError> {
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
    Ok((Clip { tracks: positions }, rot_tracks, duration))
}

/// Transfer the fighter-rig motion onto the target body, joint by joint.
///
/// The CT7 body keeps its **own** skeleton: its own bone lengths, its own
/// bind pose and its own ground. The teacher's motion rides on it in the one
/// frame both rigs share — the **world**.
///
/// Why world space and not the shared retarget's local math: the two rigs
/// have different shapes and different bind frames. The fighter rig's bind
/// rotations are the identity and its offsets are world-axis bone
/// directions, so a local offset means one thing there ("+Y is up the
/// world") and another on the CT7 ("+Y is up the bone"). The shared retarget
/// compares the two directly and scales the CT7's torso by 36x.
///
/// So the transfer is, per joint:
///
/// 1. the **bone** takes the teacher's bone direction: the world vector
///    from the teacher's parent joint to the teacher's joint, scaled to the
///    **CT7's own bone length**. The direction carries the pose, and the
///    CT7's own length decides how far the bone reaches — so the head stays
///    near the CT7's head height and the feet near its ground. A bone that
///    spans more than one teacher bone (the CT7's head covers the fighter's
///    neck and head together) keeps its span, because the teacher's parent
///    joint for that bone is the one that drives the CT7's parent;
/// 2. the **frame** is the CT7's own bind frame turned onto the teacher's
///    bone direction, so the mesh's bone stays straight between its joints;
/// 3. the **roll** about the bone comes from the teacher: how much the
///    teacher's own joint rolled about that bone since the clip's first key
///    (a sword in the hand points where the teacher pointed it);
/// 4. the local rotation is the parent's world rotation turned back out, so
///    the game's skinning composes the same world rotation again;
/// 5. the one lane that carries translation is the body root: the
///    teacher's world travel (read from the pack's root lane) sits on the
///    CT7's own root bind position, so a lunge steps the body forward on
///    the CT7's stance.
///
/// A joint the CT7 has no counterpart for holds its bind pose and rides its
/// parent rigidly; a pack track with no CT7 counterpart is dropped, exactly
/// as the map documents.
fn transfer(
    clip: &Clip,
    source_rots: &[QuatTrack],
    duration: f32,
    rig: &Skeleton,
    rig_bind: &BindPose,
    body: &Skeleton,
    body_bind: &BindPose,
) -> Result<ImportedClip, PackClipError> {
    let map = crate::ct7_map::ct7_map(body)?;
    // The teacher's world positions on the fighter rig: the pack's own lanes
    // composed down the fighter chain.
    let (posed, _posed_rots) = posed_world(clip, source_rots, rig);
    // The CT7's own bind world pose: the world positions and world rotations
    // its bind offsets and rotations compose to.
    let (bind_world_pos, bind_world_rot) = body_bind_world(body, body_bind);
    // The key count: one moment for every joint, the pack's own.
    let keys = clip.tracks[0].values.len();
    // The fighter's root joint: the pack's root lane holds the teacher's
    // world root position, and it is the one lane that carries travel.
    let fighter_root = rig
        .joints
        .iter()
        .position(|joint| joint.parent.is_none())
        .ok_or_else(|| PackClipError::Rig("the fighter rig has no root".to_string()))?;
    let root_target = (0..body.len()).find(|target| map.source_of(*target) == Some(fighter_root));
    // The teacher's travel from the clip's first key on.
    let travel: Vec<[f32; 3]> = (0..keys)
        .map(|key| {
            let now = clip.tracks[fighter_root]
                .values
                .get(key)
                .copied()
                .unwrap_or([0.0; 3]);
            let first = clip.tracks[fighter_root]
                .values
                .first()
                .copied()
                .unwrap_or([0.0; 3]);
            [now[0] - first[0], now[1] - first[1], now[2] - first[2]]
        })
        .collect();

    let mut positions = Vec::with_capacity(body.len());
    let mut rotations = Vec::with_capacity(body.len());
    // The CT7's posed world rotations, filled as the loop walks the body
    // parent-first: a child's local rotation is the parent's world rotation
    // turned back out.
    let mut world_rots: Vec<Vec<[f32; 4]>> = vec![Vec::new(); body.len()];
    for target in 0..body.len() {
        let joint = JointInputs {
            posed: &posed,
            rotations: source_rots,
            rig_bind,
            bind_world_pos: &bind_world_pos,
            bind_world_rot: &bind_world_rot,
            body_bind,
            travel: &travel,
            map: &map,
            clip,
            target,
            parent: body.joints[target].parent,
            source: map.source_of(target),
            is_root: root_target == Some(target),
            keys,
        };
        let (position, rotation, world) = transplant(&joint, &world_rots);
        world_rots[target] = world;
        positions.push(position);
        rotations.push(rotation);
    }
    Ok(ImportedClip {
        name: DEFAULT_CLIP_NAME.to_string(),
        duration,
        positions: Clip { tracks: positions },
        rotations,
    })
}

/// Everything one body joint's transplant reads, gathered once per joint:
/// what the teacher did, what the body holds at bind and where the joint
/// sits in both.
struct JointInputs<'a> {
    /// The teacher's world positions per joint per key, on the fighter rig.
    posed: &'a [Vec<[f32; 3]>],
    /// The teacher's own local rotations per joint, straight from the pack.
    rotations: &'a [QuatTrack],
    /// The fighter rig's bind offsets, world-axis: the roll's axis.
    rig_bind: &'a BindPose,
    /// The CT7's bind world positions: its own bone lengths and heights.
    bind_world_pos: &'a [[f32; 3]],
    /// The CT7's bind world rotations: its own frame, joint by joint.
    bind_world_rot: &'a [[f32; 4]],
    /// The CT7's own bind pose: offsets and rotations, in skeleton order.
    body_bind: &'a BindPose,
    /// The teacher's world travel per key, relative to the clip's first key.
    travel: &'a [[f32; 3]],
    /// The name map between the two rigs: the one bridge there is.
    map: &'a crate::retarget::BoneMap,
    /// The pack's own position tracks, for each lane's own time grid.
    clip: &'a Clip,
    /// The CT7 joint being transplanted.
    target: usize,
    /// Its parent on the CT7, when it has one.
    parent: Option<usize>,
    /// The fighter joint that drives it, `None` when the map has no row.
    source: Option<usize>,
    /// True for the body root: the one joint that carries translation.
    is_root: bool,
    /// The key count: one moment for every joint, the pack's own.
    keys: usize,
}

/// A joint the map does not bind: it holds its bind pose and rides its
/// parent rigidly. Its world rotation is the parent's composed with the
/// bind rotation, its local is the bind one, and it carries no lane.
fn held_bind_pose(
    joint: &JointInputs<'_>,
    world_rots: &[Vec<[f32; 4]>],
) -> (Track, QuatTrack, Vec<[f32; 4]>) {
    let bind_rotation = joint.body_bind.rotations[joint.target];
    let world: Vec<[f32; 4]> = (0..joint.keys)
        .map(|index| {
            compose(
                parent_world_rot(world_rots, joint.parent, index),
                bind_rotation,
            )
        })
        .collect();
    (
        Track {
            times: vec![0.0],
            values: vec![joint.body_bind.positions[joint.target]],
        },
        QuatTrack {
            times: vec![0.0],
            values: vec![bind_rotation],
        },
        world,
    )
}

/// One body joint transplanted: its position track, its rotation track and
/// the posed world rotation the parent-first walk composes the rest from.
fn transplant(
    joint: &JointInputs<'_>,
    world_rots: &[Vec<[f32; 4]>],
) -> (Track, QuatTrack, Vec<[f32; 4]>) {
    let keys = joint.keys;
    let parent = joint.parent;
    let Some(source) = joint.source else {
        return held_bind_pose(joint, world_rots);
    };
    // The teacher's bone for this joint: from the joint that drives the
    // CT7's parent to the joint that drives this one. The CT7's bone keeps
    // the CT7's own length; the direction is the teacher's.
    let source_parent = parent.and_then(|parent| joint.map.source_of(parent));
    // The world rotation: the CT7's own bind frame turned onto the
    // teacher's bone direction, with the teacher's roll about that bone on
    // top.
    let world = swung_world_rotations(
        keys,
        &SwingInputs {
            posed: joint.posed,
            rotations: joint.rotations,
            source,
            source_parent,
            own_bone: own_bind_direction(
                joint.target,
                parent,
                joint.bind_world_pos,
                joint.bind_world_rot,
            ),
            // The teacher's roll about its own bone since the clip's first
            // key: the change of the teacher's own joint rotation, measured
            // about the bone the fighter rig holds (its own bind offset,
            // world-axis).
            roll_axis: unit(joint.rig_bind.positions[source]),
            // The teacher's rotation at the clip's first key, the roll's
            // reference.
            teacher_first: joint.rotations[source]
                .values
                .first()
                .copied()
                .unwrap_or([0.0, 0.0, 0.0, 1.0]),
        },
    );
    // The local rotation: the parent's world rotation turned back out, so
    // the game's skinning composes the same world rotation again.
    let local: Vec<[f32; 4]> = (0..keys)
        .map(|index| {
            compose(
                conj(parent_world_rot(world_rots, parent, index)),
                world[index],
            )
        })
        .collect();
    // The position: the CT7's own bone, pointed the way the teacher's bone
    // points and scaled to the CT7's own bone length.
    let length = joint.body_bind.positions[joint.target]
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    let values = placed_positions(
        keys,
        &PlaceInputs {
            posed: joint.posed,
            source,
            source_parent,
            length,
            bind_offset: joint.body_bind.positions[joint.target],
            is_root: joint.is_root,
            travel: joint.travel,
        },
        world_rots,
        parent,
    );
    (
        Track {
            times: joint.clip.tracks[source].times.clone(),
            values,
        },
        QuatTrack {
            times: joint.rotations[source].times.clone(),
            values: local,
        },
        world,
    )
}

/// The CT7's own bone direction at bind, in the joint's own frame: the
/// frame the swing turns. The root, which has no parent, takes the world
/// axis its bind pose stands on.
fn own_bind_direction(
    target: usize,
    parent: Option<usize>,
    bind_world_pos: &[[f32; 3]],
    bind_world_rot: &[[f32; 4]],
) -> [f32; 3] {
    let Some(parent) = parent else {
        return [0.0, 0.0, 1.0];
    };
    let bone = [
        bind_world_pos[target][0] - bind_world_pos[parent][0],
        bind_world_pos[target][1] - bind_world_pos[parent][1],
        bind_world_pos[target][2] - bind_world_pos[parent][2],
    ];
    unit(rotate(bone, conj(bind_world_rot[target])))
}

/// One joint's swing inputs, read once per clip: what the teacher did and
/// what the body holds at bind.
struct SwingInputs<'a> {
    /// The teacher's world positions per joint per key, on the fighter rig.
    posed: &'a [Vec<[f32; 3]>],
    /// The teacher's own local rotations per joint, straight from the pack.
    rotations: &'a [QuatTrack],
    /// The fighter joint that drives this one.
    source: usize,
    /// The fighter joint that drives the CT7's parent, when it has one.
    source_parent: Option<usize>,
    /// The CT7's own bone direction at bind, in this joint's own frame.
    own_bone: [f32; 3],
    /// The axis the teacher's roll is measured about: world-axis, its own
    /// bind offset.
    roll_axis: [f32; 3],
    /// The teacher's rotation at the clip's first key: the roll's reference.
    teacher_first: [f32; 4],
}

/// One joint's placed-position inputs, read once per clip.
struct PlaceInputs<'a> {
    /// The teacher's world positions per joint per key, on the fighter rig.
    posed: &'a [Vec<[f32; 3]>],
    /// The fighter joint that drives this one.
    source: usize,
    /// The fighter joint that drives the CT7's parent, when it has one.
    source_parent: Option<usize>,
    /// The CT7's own bone length at bind: how far the bone reaches.
    length: f32,
    /// The CT7's own bind offset: the held base the travel rides on.
    bind_offset: [f32; 3],
    /// True for the body root, the one joint that carries travel.
    is_root: bool,
    /// The teacher's world travel per key, relative to the clip's first key.
    travel: &'a [[f32; 3]],
}

/// The CT7's posed world rotations per key for one joint: its own bind
/// frame turned onto the teacher's bone direction, with the teacher's roll
/// about that bone on top.
fn swung_world_rotations(keys: usize, inputs: &SwingInputs<'_>) -> Vec<[f32; 4]> {
    let mut world = Vec::with_capacity(keys);
    for index in 0..keys {
        let tip = inputs.posed[inputs.source]
            .get(index)
            .copied()
            .unwrap_or([0.0; 3]);
        let base = inputs
            .source_parent
            .and_then(|parent| inputs.posed[parent].get(index).copied())
            .unwrap_or([0.0; 3]);
        let direction = unit([tip[0] - base[0], tip[1] - base[1], tip[2] - base[2]]);
        // The teacher's own joint rotation change since the clip's first
        // key, measured as the roll it makes about its own bone.
        let teacher_now = inputs.rotations[inputs.source]
            .values
            .get(index)
            .copied()
            .unwrap_or(inputs.teacher_first);
        let delta = compose(teacher_now, conj(inputs.teacher_first));
        let roll = twist_angle(delta, inputs.roll_axis);
        let swung = swing(inputs.own_bone, direction);
        world.push(compose(twist(direction, roll), swung));
    }
    world
}

/// The CT7's local offsets per key for one joint: its own bone, pointed the
/// way the teacher's bone points and scaled to its own length, written in
/// the parent's frame. The body root is the one lane that carries travel.
fn placed_positions(
    keys: usize,
    inputs: &PlaceInputs<'_>,
    world_rots: &[Vec<[f32; 4]>],
    parent: Option<usize>,
) -> Vec<[f32; 3]> {
    let mut values = Vec::with_capacity(keys);
    for index in 0..keys {
        let tip = inputs.posed[inputs.source]
            .get(index)
            .copied()
            .unwrap_or([0.0; 3]);
        let base = inputs
            .source_parent
            .and_then(|joint| inputs.posed[joint].get(index).copied())
            .unwrap_or([0.0; 3]);
        let direction = unit([tip[0] - base[0], tip[1] - base[1], tip[2] - base[2]]);
        let bone = [
            direction[0] * inputs.length,
            direction[1] * inputs.length,
            direction[2] * inputs.length,
        ];
        // The bone is written in the CT7's parent frame: the game's
        // skinning turns it by the parent's world rotation, so the inverse
        // turns it back.
        let turned = rotate(bone, conj(parent_world_rot(world_rots, parent, index)));
        if inputs.is_root {
            // The body root: the CT7's own bind offset plus the teacher's
            // world travel, turned into the parent's frame the same way.
            let travel = inputs.travel.get(index).copied().unwrap_or([0.0; 3]);
            let carry = rotate(travel, conj(parent_world_rot(world_rots, parent, index)));
            values.push([
                inputs.bind_offset[0] + carry[0],
                inputs.bind_offset[1] + carry[1],
                inputs.bind_offset[2] + carry[2],
            ]);
        } else {
            values.push(turned);
        }
    }
    values
}

/// A joint's parent world rotation at one key, or the identity for a joint
/// with no parent (the body root, whose parent frame is the world).
fn parent_world_rot(world_rots: &[Vec<[f32; 4]>], parent: Option<usize>, index: usize) -> [f32; 4] {
    parent
        .and_then(|parent| {
            world_rots
                .get(parent)
                .and_then(|values| values.get(index))
                .copied()
        })
        .unwrap_or([0.0, 0.0, 0.0, 1.0])
}

/// The CT7's bind world pose: the world positions and world rotations its own
/// bind offsets and rotations compose to, in the engine's skinning
/// convention (a child's offset is turned by the parent's world rotation).
fn body_bind_world(body: &Skeleton, bind: &BindPose) -> (Vec<[f32; 3]>, Vec<[f32; 4]>) {
    let mut positions = vec![[0.0f32; 3]; body.len()];
    let mut rotations = vec![[0.0f32, 0.0, 0.0, 1.0]; body.len()];
    for (index, joint) in body.joints.iter().enumerate() {
        let (parent_pos, parent_rot) = match joint.parent {
            Some(parent) => (positions[parent], rotations[parent]),
            None => ([0.0; 3], [0.0, 0.0, 0.0, 1.0]),
        };
        let turned = rotate(bind.positions[index], parent_rot);
        positions[index] = [
            parent_pos[0] + turned[0],
            parent_pos[1] + turned[1],
            parent_pos[2] + turned[2],
        ];
        rotations[index] = compose(parent_rot, bind.rotations[index]);
    }
    (positions, rotations)
}

/// The shortest rotation taking one direction onto another.
fn swing(from: [f32; 3], to: [f32; 3]) -> [f32; 4] {
    let dot = from[0] * to[0] + from[1] * to[1] + from[2] * to[2];
    if dot >= 1.0 - 1.0e-6 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    if dot <= -1.0 + 1.0e-6 {
        // Opposite: any axis across them turns the bone the same way.
        let axis = perpendicular(from);
        return [axis[0], axis[1], axis[2], 0.0];
    }
    let axis = [
        from[1] * to[2] - from[2] * to[1],
        from[2] * to[0] - from[0] * to[2],
        from[0] * to[1] - from[1] * to[0],
    ];
    let half = ((1.0 + dot) * 2.0).sqrt();
    [axis[0] / half, axis[1] / half, axis[2] / half, half * 0.5]
}

/// The rotation about an axis by an angle, in radians.
fn twist(axis: [f32; 3], angle: f32) -> [f32; 4] {
    let half = angle * 0.5;
    let s = half.sin();
    [axis[0] * s, axis[1] * s, axis[2] * s, half.cos()]
}

/// How far a rotation rolls about an axis, in radians: the signed angle it
/// turns a direction across the axis.
fn twist_angle(rot: [f32; 4], axis: [f32; 3]) -> f32 {
    let from = perpendicular(axis);
    let to = rotate(from, rot);
    let from = project(from, axis);
    let to = project(to, axis);
    if from == [0.0; 3] || to == [0.0; 3] {
        return 0.0;
    }
    let cross = [
        from[1] * to[2] - from[2] * to[1],
        from[2] * to[0] - from[0] * to[2],
        from[0] * to[1] - from[1] * to[0],
    ];
    (cross[0] * axis[0] + cross[1] * axis[1] + cross[2] * axis[2])
        .atan2(from[0] * to[0] + from[1] * to[1] + from[2] * to[2])
}

/// A unit vector across the given one.
fn perpendicular(axis: [f32; 3]) -> [f32; 3] {
    let other = if axis[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let cross = [
        axis[1] * other[2] - axis[2] * other[1],
        axis[2] * other[0] - axis[0] * other[2],
        axis[0] * other[1] - axis[1] * other[0],
    ];
    unit(cross)
}

/// The part of a vector across an axis.
fn project(vector: [f32; 3], axis: [f32; 3]) -> [f32; 3] {
    let along = vector[0] * axis[0] + vector[1] * axis[1] + vector[2] * axis[2];
    [
        vector[0] - along * axis[0],
        vector[1] - along * axis[1],
        vector[2] - along * axis[2],
    ]
}

/// A unit vector, or the zero vector when the input has no direction.
fn unit(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length <= 1.0e-6 || !length.is_finite() {
        return [0.0; 3];
    }
    [vector[0] / length, vector[1] / length, vector[2] / length]
}

/// The conjugate of a quaternion: its inverse, for a unit quaternion.
fn conj(quat: [f32; 4]) -> [f32; 4] {
    [-quat[0], -quat[1], -quat[2], quat[3]]
}

/// The pack's posed world positions per key: the local offsets composed
/// along the fighter rig's own chain with the teacher's rotations, exactly
/// the skinning convention, so the result indexes by joint then by key.
///
/// The result is indexed by joint and then by key. Every track in a pack
/// holds the same keys, so one index means one moment for every joint.
type PosedWorld = (Vec<Vec<[f32; 3]>>, Vec<Vec<[f32; 4]>>);

/// Posed world positions and rotations for one clip, joint then key.
fn posed_world(source: &Clip, source_rots: &[QuatTrack], rig: &Skeleton) -> PosedWorld {
    let mut positions: Vec<Vec<[f32; 3]>> = Vec::with_capacity(rig.len());
    let mut rotations: Vec<Vec<[f32; 4]>> = Vec::with_capacity(rig.len());
    for joint in 0..rig.len() {
        let track = &source.tracks[joint];
        let parent = rig.joints[joint].parent;
        let mut values = Vec::with_capacity(track.values.len());
        let mut rots = Vec::with_capacity(track.values.len());
        for (index, value) in track.values.iter().enumerate() {
            let time = track.times.get(index).copied().unwrap_or_default();
            // The parent's world rotation and position in this pose.
            let (parent_world, parent_rotation) = match parent {
                Some(parent) => (
                    positions
                        .get(parent)
                        .and_then(|keys| keys.get(index))
                        .copied()
                        .unwrap_or([0.0; 3]),
                    rotations
                        .get(parent)
                        .and_then(|keys| keys.get(index))
                        .copied()
                        .unwrap_or([0.0, 0.0, 0.0, 1.0]),
                ),
                None => ([0.0; 3], [0.0, 0.0, 0.0, 1.0]),
            };
            // The joint's own local rotation, and the world rotation it
            // makes with its parent.
            let local_rotation = source_rots
                .get(joint)
                .and_then(|track| {
                    let at = track
                        .times
                        .iter()
                        .position(|other| *other >= time)
                        .unwrap_or_else(|| track.times.len().saturating_sub(1));
                    track.values.get(at).copied()
                })
                .unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let world_rotation = compose(parent_rotation, local_rotation);
            // The local offset is turned by the parent's world rotation: the
            // pack's offsets are bone-axis, like the CT7's own, so a plain
            // sum would put the arm in the wrong place.
            let turned = rotate(*value, parent_rotation);
            values.push([
                parent_world[0] + turned[0],
                parent_world[1] + turned[1],
                parent_world[2] + turned[2],
            ]);
            rots.push(world_rotation);
        }
        positions.push(values);
        rotations.push(rots);
    }
    (positions, rotations)
}

/// The position values of one packed track, dequantized.
///
/// The wire stores each lane as a 16-bit key with the track's own min and
/// max, so the value is `min + (max - min) * key / 65535`. The same formula
/// the anim crate's own sampler uses, so the clip this builds samples to the
/// same numbers the pack holds.
fn dequantize(track: &engine_anim::clip::QTrack) -> Vec<[f32; 3]> {
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
fn lane(track: &engine_anim::clip::QTrack, key: u16, axis: usize) -> f32 {
    let min = track.min[axis];
    let max = track.max[axis];
    min + (max - min) * (f32::from(key) / 65535.0)
}

/// Rotate a vector by a quaternion.
///
/// [`quat_mul`] is the raw Hamilton product, without normalization: a
/// normalized product would turn every rotated offset into a unit vector, so
/// the bone lengths survive only while the product stays raw.
fn rotate(v: [f32; 3], quat: [f32; 4]) -> [f32; 3] {
    let conj = [-quat[0], -quat[1], -quat[2], quat[3]];
    let r = quat_mul(quat_mul(quat, [v[0], v[1], v[2], 0.0]), conj);
    [r[0], r[1], r[2]]
}

/// Compose two quaternions: `a` then `b`.
///
/// The quaternion is `x, y, z, w`.
fn compose(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let out = [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ];
    let norm = (out[0] * out[0] + out[1] * out[1] + out[2] * out[2] + out[3] * out[3]).sqrt();
    if norm <= 0.0 || !norm.is_finite() {
        return [0.0, 0.0, 0.0, 1.0];
    }
    [out[0] / norm, out[1] / norm, out[2] / norm, out[3] / norm]
}
