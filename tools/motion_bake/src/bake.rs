//! bake: one teacher NPZ to one `.am5p` plus its receipt.
//!
//! Pipeline, all off-tick: read the Kimodo arrays, lay the teacher's world
//! joints on the SOMA-77 rig, retarget them onto the fighter-65 rig through
//! `BoneMap::build` + `retarget_cook` over the one name table
//! (`soma_pairs()`), take the length from the last constrained frame
//! (`engine_anim::motion_bake::bake_len`), sample the two foot lanes at the
//! bake rate and pack them through the AM5P home (`bake_pack`). The receipt
//! names every number the gate reads: `qclip_hash`, the track count, the tick
//! count, `avg_slip` per foot, the seed and the source file's sha256.
//!
//! Translation convention: the teacher's positions are world joint positions
//! (Kimodo's `posed_joints`), so the retarget transfers the world delta onto
//! each fighter joint's local bind. That is exactly the lane the contact and
//! slip meters read, and it is the convention the hero bake uses for feet.
//! Rotations ride as bind-relative local deltas, which is what the retarget
//! correction expects.

use crate::npz::{Npz, NpzError};
use crate::rig::{fighter_bind, fighter_skeleton, joint_index, soma_bind, soma_skeleton};
use engine_anim::clip::{avg_slip, bake_clip, bake_track, qclip_hash, TICK_DT};
use engine_anim::motion_bake::{
    bake_len, bake_pack_with_rots, bake_unpack, last_constrained, BakeError,
};
use engine_anim::pose::{sample_quat_track, QuatTrack};
use engine_anim::{sample, Clip, Track};
use engine_assets::fighter::RigError;
use engine_assets::retarget::{retarget_cook, retarget_digest, RetargetError};
use engine_assets::soma::{soma_pairs, SOMA77};
use engine_assets::{BoneMap, ClipTags, CookConfig, ImportedClip, RetargetPlan};
use sha2::{Digest, Sha256};

/// Bake rate: the fixed tick the clip is sampled at, frames per second.
pub const BAKE_FPS: f32 = 60.0;
/// A teacher root (`root_positions`) and the same joint inside `posed_joints`
/// must agree this closely, meters; a larger gap means the file disagrees
/// with itself.
pub const ROOT_AGREEMENT_M: f32 = 0.01;

/// Why a bake run was refused; every variant names the fault.
#[derive(Clone, Debug, PartialEq)]
pub enum BakeRunError {
    /// The `.npz` could not be read; the cause is carried.
    Npz(NpzError),
    /// An array has a shape the bake cannot use; the array is named.
    Shape(&'static str),
    /// A named joint is missing from the rig it was asked of.
    MissingJoint(&'static str),
    /// `root_positions` and `posed_joints` disagree; frame and gap carried.
    RootMismatch {
        /// Frame index.
        frame: usize,
        /// Gap in meters.
        gap: f32,
    },
    /// A lane carries a non-finite number; the lane is named.
    NonFinite(&'static str),
    /// The retarget cook refused the clip.
    Retarget(RetargetError),
    /// The endpoint rule or the pack refused the bake.
    Bake(BakeError),
    /// The rig could not be built: a joint or a parent is missing.
    Rig(RigError),
}

impl core::fmt::Display for BakeRunError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Npz(cause) => write!(f, "{cause}"),
            Self::Shape(array) => write!(f, "bake: {array} has the wrong shape"),
            Self::MissingJoint(name) => write!(f, "bake: no joint {name}"),
            Self::RootMismatch { frame, gap } => {
                write!(f, "bake: root disagrees at frame {frame} by {gap} m")
            }
            Self::NonFinite(lane) => write!(f, "bake: non-finite {lane}"),
            Self::Retarget(cause) => write!(f, "{cause}"),
            Self::Bake(cause) => write!(f, "{cause}"),
            Self::Rig(cause) => write!(f, "rig: {cause}"),
        }
    }
}

impl std::error::Error for BakeRunError {}

impl From<NpzError> for BakeRunError {
    fn from(cause: NpzError) -> Self {
        Self::Npz(cause)
    }
}

impl From<RetargetError> for BakeRunError {
    fn from(cause: RetargetError) -> Self {
        Self::Retarget(cause)
    }
}

impl From<BakeError> for BakeRunError {
    fn from(cause: BakeError) -> Self {
        Self::Bake(cause)
    }
}
impl From<RigError> for BakeRunError {
    fn from(cause: RigError) -> Self {
        Self::Rig(cause)
    }
}

/// Everything one baked clip is: the bytes to ship and the receipt to prove.
#[derive(Clone, Debug)]
pub struct Baked {
    /// `.am5p` bytes.
    pub bytes: Vec<u8>,
    /// The receipt the gate reads.
    pub receipt: Receipt,
}

/// The receipt fields, in the order the writer prints them.
#[derive(Clone, Debug)]
pub struct Receipt {
    /// Clip name.
    pub clip: String,
    /// Teacher array layout name.
    pub source_kind: &'static str,
    /// sha256 of the teacher bytes, hex.
    pub source_sha256: String,
    /// Teacher frame count.
    pub source_frames: usize,
    /// Teacher frame rate.
    pub source_fps: f32,
    /// Seed the teacher recorded, when it recorded one.
    pub seed: Option<i64>,
    /// Last frame the teacher pinned, in bake ticks.
    pub last_constrained_tick: u32,
    /// Baked tick count.
    pub ticks: u32,
    /// Time of the last constrained tick, seconds.
    pub dur_s: f32,
    /// Track names in clip order: one per packed track, the fighter's own joint
    /// order.
    pub tracks: Vec<String>,
    /// Keys per track.
    pub keys: usize,
    /// The wire version the pack was written in: 1 for a position-only pack,
    /// 2 for a pack that also carries the joint rotations.
    pub pack_version: u16,
    /// Bytes on the wire.
    pub packed_bytes: usize,
    /// sha256 of the pack bytes, hex.
    pub bytes_sha256: String,
    /// `qclip_hash` of the clip the shipped bytes decode to.
    pub qclip_hash: u64,
    /// Digest of the whole retargeted cook.
    pub retarget_digest: u64,
    /// `avg_slip` of the two feet, left then right: the gate reads these.
    pub avg_slip: Vec<f32>,
    /// Plant windows per foot inside the baked length.
    pub plant_windows: Vec<u32>,
}

impl Receipt {
    /// Deterministic JSON: fixed key order, six decimals on floats.
    pub fn to_json(&self) -> String {
        let tracks: Vec<String> = self.tracks.iter().map(|t| format!("\"{t}\"")).collect();
        let seed = match self.seed {
            Some(seed) => seed.to_string(),
            None => "null".to_string(),
        };
        let slips: Vec<String> = self.avg_slip.iter().map(|v| format!("{v:.6}")).collect();
        let windows: Vec<String> = self.plant_windows.iter().map(u32::to_string).collect();
        format!(
            "{{\n  \"tool\": \"motion_bake\",\n  \"clip\": \"{}\",\n  \"source_kind\": \"{}\",\n  \"source_sha256\": \"{}\",\n  \"source_frames\": {},\n  \"source_fps\": {},\n  \"seed\": {},\n  \"endpoint_rule\": \"partigen-last-constrained-frame\",\n  \"last_constrained_tick\": {},\n  \"ticks\": {},\n  \"dur_s\": {:.6},\n  \"tracks\": [{}],\n  \"track_count\": {},\n  \"keys_per_track\": {},\n  \"pack_magic\": \"AM5P\",\n  \"pack_version\": {},\n  \"packed_bytes\": {},\n  \"bytes_sha256\": \"{}\",\n  \"qclip_hash\": \"0x{:016x}\",\n  \"retarget_digest\": \"0x{:016x}\",\n  \"avg_slip\": {{\"leftfoot\": {}, \"rightfoot\": {}}},\n  \"plant_windows\": {{\"leftfoot\": {}, \"rightfoot\": {}}}\n}}\n",
            self.clip,
            self.source_kind,
            self.source_sha256,
            self.source_frames,
            self.source_fps,
            seed,
            self.last_constrained_tick,
            self.ticks,
            self.dur_s,
            tracks.join(", "),
            self.tracks.len(),
            self.keys,
            self.pack_version,
            self.packed_bytes,
            self.bytes_sha256,
            self.qclip_hash,
            self.retarget_digest,
            slips[0],
            slips[1],
            windows[0],
            windows[1],
        )
    }
}

/// sha256 of bytes, lower-case hex: the receipt's source and pack identity.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Read a shape, dropping one leading batch axis of 1 (Kimodo writes a
/// one-sample batch as `(1, N, J, 3, 3)`).
fn shape_without_batch(mut shape: Vec<usize>) -> Vec<usize> {
    if shape.len() > 1 && shape[0] == 1 {
        shape.remove(0);
    }
    shape
}

/// Bake one teacher clip: `name` labels the clip in the receipt.
pub fn bake(bytes: &[u8], name: &str) -> Result<Baked, BakeRunError> {
    let npz = Npz::from_bytes(bytes)?;
    let mats = npz.get("local_rot_mats")?;
    let shape = shape_without_batch(mats.shape.clone());
    if shape.len() != 4 || shape[2] != 3 || shape[3] != 3 {
        return Err(BakeRunError::Shape("local_rot_mats"));
    }
    let joints = SOMA77.len();
    if shape[1] != joints {
        return Err(BakeRunError::Shape("local_rot_mats"));
    }
    let frames = shape[0];
    if frames < 2 {
        return Err(BakeRunError::Shape("local_rot_mats"));
    }
    let posed = npz.get("posed_joints")?;
    let posed_shape = shape_without_batch(posed.shape.clone());
    if posed_shape != vec![frames, joints, 3] {
        return Err(BakeRunError::Shape("posed_joints"));
    }
    let roots = npz.get("root_positions")?;
    let root_shape = shape_without_batch(roots.shape.clone());
    if root_shape != vec![frames, 3] {
        return Err(BakeRunError::Shape("root_positions"));
    }
    let fps = match npz.get("fps") {
        Ok(array) => round(fps_of(array)?),
        Err(_) => crate::synth::SOURCE_FPS,
    };
    if !fps.is_finite() || fps <= 0.0 {
        return Err(BakeRunError::NonFinite("fps"));
    }

    let mats = mats.as_f32()?;
    let posed = posed.as_f32()?;
    let roots = roots.as_f32()?;
    if mats.iter().any(|v| !v.is_finite()) {
        return Err(BakeRunError::NonFinite("local_rot_mats"));
    }
    if posed.iter().any(|v| !v.is_finite()) {
        return Err(BakeRunError::NonFinite("posed_joints"));
    }
    if roots.iter().any(|v| !v.is_finite()) {
        return Err(BakeRunError::NonFinite("root_positions"));
    }

    // One key per teacher frame: local rotations from the matrices, world
    // positions from posed_joints, with root_positions checked against them.
    let source = soma_skeleton()?;
    let source_bind = soma_bind()?;
    let root = joint_index(&source, "hips").ok_or(BakeRunError::MissingJoint("hips"))?;
    for frame in 0..frames {
        let at = frame * 3;
        let gap = [
            posed[frame * joints * 3 + root * 3] - roots[at],
            posed[frame * joints * 3 + root * 3 + 1] - roots[at + 1],
            posed[frame * joints * 3 + root * 3 + 2] - roots[at + 2],
        ];
        let distance = (gap[0] * gap[0] + gap[1] * gap[1] + gap[2] * gap[2]).sqrt();
        if distance > ROOT_AGREEMENT_M {
            return Err(BakeRunError::RootMismatch {
                frame,
                gap: distance,
            });
        }
    }
    let mut positions = Vec::with_capacity(joints);
    let mut rotations = Vec::with_capacity(joints);
    for joint in 0..joints {
        let mut times = Vec::with_capacity(frames);
        let mut values = Vec::with_capacity(frames);
        for frame in 0..frames {
            times.push(frame as f32 / fps);
            let at = (frame * joints + joint) * 3;
            let mut value = [posed[at], posed[at + 1], posed[at + 2]];
            if joint == root {
                value = [roots[frame * 3], roots[frame * 3 + 1], roots[frame * 3 + 2]];
            }
            values.push(value);
        }
        positions.push(Track {
            times: times.clone(),
            values,
        });
        let mut quats = Vec::with_capacity(frames);
        for frame in 0..frames {
            let at = (frame * joints + joint) * 9;
            let m = [
                [mats[at], mats[at + 1], mats[at + 2]],
                [mats[at + 3], mats[at + 4], mats[at + 5]],
                [mats[at + 6], mats[at + 7], mats[at + 8]],
            ];
            quats.push(crate::rig::mat3_to_quat(m));
        }
        rotations.push(QuatTrack {
            times,
            values: quats,
        });
    }
    let source_clip = ImportedClip {
        name: name.to_string(),
        duration: (frames - 1) as f32 / fps,
        positions: Clip { tracks: positions },
        rotations,
    };

    // The fighter rig and the one name table.
    let target = fighter_skeleton()?;
    let target_bind = fighter_bind()?;
    let pairs = soma_pairs();
    let map = BoneMap::build(&source, &target, &pairs)?;
    let root_joint = joint_index(&target, "hips").ok_or(BakeRunError::MissingJoint("hips"))?;
    let left_foot =
        joint_index(&target, "leftfoot").ok_or(BakeRunError::MissingJoint("leftfoot"))?;
    let right_foot =
        joint_index(&target, "rightfoot").ok_or(BakeRunError::MissingJoint("rightfoot"))?;
    let ground_y = target_bind.positions[left_foot][1] + crate::synth::PLANT_BAND;
    let config = CookConfig {
        root_joint,
        left_foot,
        right_foot,
        ground_y,
        start_tick: 0,
    };
    let plan = RetargetPlan {
        source_clip: &source_clip,
        source: &source,
        source_bind: &source_bind,
        target: &target,
        target_bind: &target_bind,
        map: &map,
        config: &config,
    };
    let cooked = retarget_cook(&plan, ClipTags::default())?;

    // The endpoint rule: the teacher's own planted frames decide the length.
    let source_ticks = ((frames - 1) as f32 / fps / TICK_DT).round() as usize + 1;
    let soma_rest = &source_bind.positions;
    let soma_left =
        joint_index(&source, "leftfoot").ok_or(BakeRunError::MissingJoint("leftfoot"))?;
    let soma_right =
        joint_index(&source, "rightfoot").ok_or(BakeRunError::MissingJoint("rightfoot"))?;
    let mut constrained = Vec::with_capacity(source_ticks);
    for tick in 0..source_ticks {
        let t = tick as f32 * TICK_DT;
        let left = sample(&source_clip.positions, soma_left, t);
        let right = sample(&source_clip.positions, soma_right, t);
        let left_pinned = left[1] - soma_rest[soma_left][1] <= crate::synth::PLANT_BAND;
        let right_pinned = right[1] - soma_rest[soma_right][1] <= crate::synth::PLANT_BAND;
        constrained.push(left_pinned || right_pinned);
    }
    let last = last_constrained(&constrained).ok_or(BakeError::NoConstraint)?;
    let len = bake_len(i64::from(last), BAKE_FPS)?;

    // The whole fighter body at the bake rate, packed through the one AM5P
    // home: one track per fighter joint, in the fighter's own joint order, so
    // the tick can play the pack without asking the bake anything.
    let fighter_joints = target.len();
    let mut tracks = Vec::with_capacity(fighter_joints);
    for joint in 0..fighter_joints {
        let times: Vec<f32> = (0..len.ticks).map(|k| k as f32 * TICK_DT).collect();
        let values: Vec<[f32; 3]> = times
            .iter()
            .map(|t| {
                if !t.is_finite() {
                    return [0.0; 3];
                }
                sample(&cooked.clip.positions, joint, *t)
            })
            .collect();
        tracks.push(bake_track(times, values));
    }
    let clip = bake_clip(tracks);
    // The rotations ride with the positions: a combat clip that carries only
    // the feet would slide the arms on their bind rotations, so the swing
    // would never show. The pack becomes a version-2 wire.
    let times: Vec<f32> = (0..len.ticks).map(|k| k as f32 * TICK_DT).collect();
    let mut rot_tracks = Vec::with_capacity(fighter_joints);
    for joint in 0..fighter_joints {
        let values: Vec<[f32; 4]> = times
            .iter()
            .map(|t| {
                if !t.is_finite() {
                    return [0.0, 0.0, 0.0, 1.0];
                }
                sample_quat_at(&cooked.clip.rotations, joint, *t)
            })
            .collect();
        rot_tracks.push(QuatTrack {
            times: times.clone(),
            values,
        });
    }
    let pack = bake_pack_with_rots(&clip, &rot_tracks)?;
    let shipped = bake_unpack(&pack)?;

    let mut windows = [0u32; 2];
    let mut down = [false; 2];
    for contact in &cooked.contacts.contacts {
        if contact.tick >= len.ticks {
            continue;
        }
        let foot = usize::from(contact.foot);
        if foot < 2 && contact.down && !down[foot] {
            windows[foot] += 1;
        }
        if foot < 2 {
            down[foot] = contact.down;
        }
    }
    let seed = match npz.get("seed") {
        Ok(array) => array
            .as_i64()
            .ok()
            .and_then(|values| values.first().copied()),
        Err(_) => None,
    };
    let receipt = Receipt {
        clip: name.to_string(),
        source_kind: "kimodo-npz",
        source_sha256: sha256_hex(bytes),
        source_frames: frames,
        source_fps: fps,
        seed,
        last_constrained_tick: last,
        ticks: len.ticks,
        dur_s: len.dur,
        tracks: target
            .joints
            .iter()
            .map(|joint| joint.name.clone())
            .collect(),
        keys: shipped.tracks.first().map_or(0, |track| track.times.len()),
    pack_version: u16::from_le_bytes([pack[4], pack[5]]),
        packed_bytes: pack.len(),
        bytes_sha256: sha256_hex(&pack),
        qclip_hash: qclip_hash(&shipped),
        retarget_digest: retarget_digest(&cooked),
        avg_slip: vec![
            avg_slip(&shipped, left_foot, len.ticks),
            avg_slip(&shipped, right_foot, len.ticks),
        ],
        plant_windows: windows.to_vec(),
    };
    Ok(Baked {
        bytes: pack,
        receipt,
    })
}

/// The `fps` scalar as a number, whichever numeric type the teacher used.
fn fps_of(array: &crate::npz::Npy) -> Result<f32, BakeRunError> {
    let values = array.as_f32()?;
    values.first().copied().ok_or(BakeRunError::Shape("fps"))
}

/// Round a rate to a whole number of frames per second when it is already
/// one (Kimodo writes 30.0; a float rounding error must not change times).
fn round(fps: f32) -> f32 {
    let whole = fps.round();
    if (fps - whole).abs() < 1e-3 {
        whole
    } else {
        fps
    }
}

/// The mtime-free helper the tests use: bake the committed fixture bytes.
/// One joint's rotation at `time` seconds, read from the retargeted clip.
///
/// A joint with no rotation lane (or a lane that holds nothing usable) reads
/// as the identity, so the pack always carries a unit quaternion per joint.
fn sample_quat_at(rotations: &[QuatTrack], joint: usize, time: f32) -> [f32; 4] {
    let Some(track) = rotations.get(joint) else {
        return [0.0, 0.0, 0.0, 1.0];
    };
    sample_quat_track(track, time).unwrap_or([0.0, 0.0, 0.0, 1.0])
}

#[cfg(test)]
pub fn committed() -> (Vec<u8>, String) {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let source = std::fs::read(format!("{root}/fixtures/clips/stagger.npz")).expect("npz");
    let baked = bake(&source, "stagger").expect("bakes");
    (baked.bytes, baked.receipt.to_json())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_committed_fixture_is_this_bake_run_twice() {
        let (first, receipt) = committed();
        let (second, again) = committed();
        assert_eq!(first, second, "two bakes of one source, one byte stream");
        assert_eq!(receipt, again, "and one receipt");
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let on_disk = std::fs::read(format!("{root}/fixtures/clips/stagger.am5p"))
            .expect("committed fixture");
        let on_disk_receipt =
            std::fs::read_to_string(format!("{root}/fixtures/clips/stagger.receipt.json"))
                .expect("committed receipt");
        assert_eq!(first, on_disk, "the fixture is the tool's own bytes");
        assert_eq!(receipt, on_disk_receipt, "the receipt is the tool's text");
        assert!(receipt.contains("\"qclip_hash\": \"0x"), "a receipt hash");
        assert_eq!(first.len(), 54813, "the committed pack size");
    }
}
