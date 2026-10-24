//! The realistic body family in the look: Rocketbox bodies, each with its own clips.
//!
//! The CT7 bodies share one clip set that the look retargets onto each body
//! by name. A Rocketbox body cannot take that set: its Biped bones carry
//! other bind rotations, and the shared retarget compares local offsets
//! across conventions. So each Rocketbox variant ships its own six clips,
//! already on its own skeleton (`tools/blender/rocketbox_fighter.py`, from
//! Mixamo mocap), and the look's retarget of a clip onto the very skeleton it
//! was cooked on is the identity.
//!
//! Data owner: [`crate::fighter_bind`] binds the bodies (CT7 and Rocketbox);
//! this file holds every look constructor: the CT7 load, the Rocketbox load
//! (each variant's clips in the look's slot order) and the shared tail
//! (`from_rigs`) with the body-units guard. Errors: [`LookError`], loud.

use super::*;

use engine_anim::pose::QuatTrack;
use engine_anim::{Clip, Track};
use engine_assets::ktx2_cook::{decode_level_rgba, parse_ktx2, MapKind};
use engine_assets::retarget::BindPose;

/// The band a person's pelvis sits in above the ground at bind, in metres.
/// Every body the game ships stands at 0.89 m; a body outside the band was
/// cooked in other units or with an unapplied scale, and drawing it puts the
/// camera inside a giant (2026-10-22) or loses the fighter under the sand.
pub(super) const PELVIS_BAND_M: (f32, f32) = (0.6, 1.4);

/// Refuse a body whose bind pelvis is not at a person's height, loudly and
/// by name, instead of drawing it at the wrong size.
pub(super) fn check_body_units(variant: &str, pelvis: [f32; 3]) -> Result<(), LookError> {
    let height = pelvis[1];
    let (low, high) = PELVIS_BAND_M;
    if height.is_finite() && (low..=high).contains(&height) {
        return Ok(());
    }
    Err(LookError::Import(format!(
        "{variant}: pelvis bind height {height} m is not a person's ({low}..{high} m):          the body was cooked in other units or with an unapplied object scale"
    )))
}

/// The six acts in the look's clip order, each held for this many seconds.
/// The cooked lane carries no clip GLBs yet, so every act draws the bind
/// pose instead of refusing the whole rig. A clips bundle can replace these
/// holds without touching the body or texture path.
const COOKED_HOLD_CLIPS: [(&str, f32); 6] = [
    ("idle", 1.0),
    ("run", 0.8),
    ("attack_0", 0.5),
    ("attack_1", 0.5),
    ("attack_2", 0.5),
    ("down", 1.0),
];

/// One hold clip: every joint keeps its bind local transform at both ends.
/// The target skeleton is the body's own, so the look's retarget is the
/// identity and the drawn pose is the bind pose.
fn hold_clip(name: &str, duration: f32, bind: &BindPose) -> ImportedClip {
    let times = vec![0.0, duration];
    let positions = Clip {
        tracks: bind
            .positions
            .iter()
            .map(|position| Track {
                times: times.clone(),
                values: vec![*position, *position],
            })
            .collect(),
    };
    let rotations = bind
        .rotations
        .iter()
        .map(|rotation| QuatTrack {
            times: times.clone(),
            values: vec![*rotation, *rotation],
        })
        .collect();
    ImportedClip {
        name: name.to_string(),
        duration,
        positions,
        rotations,
    }
}

/// The six hold clips as clip sources on the body's own skeleton and bind.
fn hold_sources(body: &SkinnedBody) -> Vec<ClipSource> {
    COOKED_HOLD_CLIPS
        .iter()
        .map(|(name, duration)| ClipSource {
            skeleton: body.skeleton.clone(),
            bind: body.bind.clone(),
            clips: vec![hold_clip(name, *duration, &body.bind)],
        })
        .collect()
}

impl FighterLook {
    /// Bind the CT7 bundles under `content_dir`, re-hash every body against
    /// its pin, import bodies plus clips, retarget the clips per rig and
    /// decode the base-color maps. Cold path; loud on any refusal.
    /// WEB6: this is the native door into [`FighterLook::load_from`]; the
    /// browser has no file system and reads the same bytes out of a
    /// fetched bundle.
    pub fn load(content_dir: &Path) -> Result<Self, LookError> {
        Self::load_from(&FsSource::new(content_dir))
    }

    /// The same bind out of a source: the manifest, the body GLBs and the
    /// baked clips are read by logical path, so a browser and the native
    /// window see the same bytes. Cold path; loud on any refusal.
    pub fn load_from(source: &dyn AssetSource) -> Result<Self, LookError> {
        let bindings =
            fighter_bind::bind_all_from(source).map_err(|err| LookError::Bind(err.to_string()))?;
        let mut sources = Vec::with_capacity(CLIP_FILES.len());
        for file in CLIP_FILES {
            let path = clip_logical_path(file);
            let bytes = source
                .read(&path)
                .map_err(|err| LookError::Import(format!("{file}: {err}")))?;
            sources.push(
                import_clip_source(&bytes)
                    .map_err(|err| LookError::Import(format!("{file}: {err}")))?,
            );
        }
        let mut rigs = Vec::with_capacity(bindings.len());
        for (index, binding) in bindings.iter().enumerate() {
            rigs.push(load_rig(binding, index, &sources, source)?);
        }
        Self::from_rigs(
            rigs,
            bindings.into_iter().map(|b| b.bundle_sha256).collect(),
        )
    }

    /// The cooked lane the page fetches: the four bodies and their atlas
    /// quadrants by the names `engine_assets::rig_cook` writes. The bodies
    /// are pinned by SHA; each act holds the bind pose because the lane
    /// carries no clip GLBs yet. Cold path; loud on any refusal.
    pub fn load_cooked_from(source: &dyn AssetSource) -> Result<Self, LookError> {
        let mut rigs = Vec::with_capacity(fighter_bind::VARIANTS.len());
        let mut shas = Vec::with_capacity(fighter_bind::VARIANTS.len());
        for (index, variant) in fighter_bind::VARIANTS.iter().enumerate() {
            let body_name = format!("{}/{variant}_body.glb", engine_assets::rig_cook::COOKED_DIR);
            let bytes = source
                .read(&body_name)
                .map_err(|err| LookError::Bind(format!("{body_name}: {err}")))?;
            let body_sha = sha_hex(&bytes);
            if body_sha.as_str() != fighter_bind::EXPECTED_COOKED_BODY_SHA256[index] {
                return Err(LookError::ShaMismatch((*variant).to_string()));
            }
            // Import once for the hold clips; `load_rig` imports the same
            // bytes for the draw. Cold path, so the second parse is cheap
            // enough and keeps one body-import path for every family.
            let body = import_skinned_body(&bytes)
                .map_err(|err| LookError::Import(format!("{variant}: {err}")))?;
            // One cooked part per atlas quadrant. `load_rig` appends the hero's
            // graft and Roman kit after them (`fighter_gear::arm_hero`,
            // `fighter_kit_body::dress`); those carry their own generated maps.
            if body.parts.len() != engine_assets::rig_cook::QUADRANTS {
                return Err(LookError::Import(format!(
                    "{variant}: {} cooked parts, the cooked lane has {} atlas quadrants",
                    body.parts.len(),
                    engine_assets::rig_cook::QUADRANTS
                )));
            }
            let hold = hold_sources(&body);
            let binding = FighterBinding {
                variant: (*variant).to_string(),
                bundle_sha256: body_sha.clone(),
                body_glb_sha256: body_sha.clone(),
                body_glb: body_name,
                joints: u32::try_from(body.skeleton.len())
                    .map_err(|_| LookError::Import(format!("{variant}: joint count over u32")))?,
                tris_lod0: 0,
                clips: u32::try_from(hold.len())
                    .map_err(|_| LookError::Import(format!("{variant}: clip count over u32")))?,
            };
            let mut rig = load_rig(&binding, index, &hold, source)?;
            let quadrants = engine_assets::rig_cook::QUADRANTS;
            // The cooked parts come first; the parts `load_rig` appends keep
            // the maps it gave them.
            for (part_index, part) in rig.parts.iter_mut().take(quadrants).enumerate() {
                let atlas = format!(
                    "{}/{variant}_q{part_index}.ktx2",
                    engine_assets::rig_cook::COOKED_DIR
                );
                let bytes = source
                    .read(&atlas)
                    .map_err(|err| LookError::Import(format!("{atlas}: {err}")))?;
                let texture = parse_ktx2(&bytes)
                    .map_err(|err| LookError::Import(format!("{atlas}: {err:?}")))?;
                if texture.kind != MapKind::BaseColor {
                    return Err(LookError::Import(format!("{atlas}: not a base-color map")));
                }
                let (width, height, rgba) = decode_level_rgba(&texture, 0)
                    .map_err(|err| LookError::Import(format!("{atlas}: {err:?}")))?;
                let upload = TextureUpload {
                    width,
                    height,
                    rgba,
                };
                upload
                    .validate()
                    .map_err(|err| LookError::Import(format!("{atlas}: {err}")))?;
                let id = LOOK_TEXTURE_ID_BASE
                    + u64::try_from(index * MAX_PARTS + part_index)
                        .map_err(|_| LookError::Import("texture id".to_string()))?;
                part.texture = Some((TextureKey::new(id, digest32(&bytes)), upload));
            }
            shas.push(body_sha);
            rigs.push(rig);
        }
        Self::from_rigs(rigs, shas)
    }

    /// The look around loaded rigs: the swing length from the first rig's
    /// first attack clip, then the inertia, limit, ragdoll and cape layers.
    /// Shared by every body family.
    fn from_rigs(rigs: Vec<Rig>, bundle_shas: Vec<String>) -> Result<Self, LookError> {
        for rig in &rigs {
            fighter_look_family::check_body_units(&rig.variant, rig.bind_pelvis)?;
        }
        let swing_s = rigs
            .first()
            .and_then(|rig| rig.clips.get(2))
            .map_or(0.0, |clip| clip.duration);
        // Whole ticks covering the swing; clip lengths are a few seconds.
        let mut swing_ticks = 1u32;
        while f32::from(u16::try_from(swing_ticks).unwrap_or(u16::MAX)) < swing_s * TICK_HZ
            && swing_ticks < 600
        {
            swing_ticks += 1;
        }
        let inertia = fighter_look_inertia::slot_sets(&rigs)?;
        let limit = limit_lanes(&rigs)?;
        let ragdoll =
            Self::ragdoll_for_rigs(&rigs).map_err(|error| LookError::Import(error.to_string()))?;
        let capes = Capes::for_rigs(&rigs);
        Ok(Self {
            rigs,
            ring: None,
            slots: vec![SlotAnim::new(); MAX_LOOK_FIGHTERS],
            bridge: bridge_slots(),
            trails: TrailRing::new(),
            clip_events: ClipEventLane::new(),
            tick: 0,
            swing_ticks,
            bundle_shas,
            truth: Vec::new(),
            shake_trauma_permille: 0,
            uploaded_digest: empty_upload_digest(),
            inertia,
            limit,
            ragdoll,
            limbs: LimbLook::new(),
            capes,
            hero: 0,
            checkpoints: empty_checkpoints(),
            rewind_cursor: 0,
            correction: None,
        })
    }

    /// The Rocketbox family out of its own content tree
    /// (`content/fighters/rocketbox`).
    pub fn load_rocketbox(dir: &Path) -> Result<Self, LookError> {
        Self::load_rocketbox_from(&FsSource::new(dir))
    }

    /// The Rocketbox family out of a source rooted at its tree: the bodies
    /// from the family manifest, each with its six clips in slot order.
    pub fn load_rocketbox_from(source: &dyn AssetSource) -> Result<Self, LookError> {
        let bindings = fighter_bind::bind_rocketbox_from(source)
            .map_err(|err| LookError::Bind(err.to_string()))?;
        let mut rigs = Vec::with_capacity(bindings.len());
        for (index, binding) in bindings.iter().enumerate() {
            let mut sources = Vec::with_capacity(fighter_bind::ROCKETBOX_CLIP_ROLES.len());
            for role in fighter_bind::ROCKETBOX_CLIP_ROLES {
                let path = fighter_bind::rocketbox_clip_path(&binding.variant, role);
                let bytes = source
                    .read(&path)
                    .map_err(|err| LookError::Import(format!("{path}: {err}")))?;
                sources.push(
                    import_clip_source(&bytes)
                        .map_err(|err| LookError::Import(format!("{path}: {err}")))?,
                );
            }
            rigs.push(load_rig(binding, index, &sources, source)?);
        }
        let shas = bindings.into_iter().map(|b| b.bundle_sha256).collect();
        Self::from_rigs(rigs, shas)
    }
}

/// Farthest a bind vertex sits from its nearest bind joint, in meters.
/// Pure `f32` in, same `f32` out; a non-finite vertex reads as infinity so
/// the caller skips it, never a panic.
fn nearest_joint_d(joints: &[[f32; 3]], vertex: [f32; 3]) -> f32 {
    let mut nearest = f32::MAX;
    for joint in joints.iter() {
        let dx = vertex[0] - joint[0];
        let dy = vertex[1] - joint[1];
        let dz = vertex[2] - joint[2];
        nearest = nearest.min(dx * dx + dy * dy + dz * dz);
    }
    nearest.sqrt()
}

/// Skin radius of one rig in meters: the farthest any bind vertex sits from
/// its nearest bind joint, plus 10 cm, floored at 0.5 m. The joints-only
/// pose pads placed joints by this radius so its bounds hold the skinned
/// vertex bounds. Cold path only (once per rig at load); pure `f32` in,
/// same `f32` out; a non-finite vertex never moves the radius.
pub(super) fn skin_radius_m(body: &SkinnedBody) -> f32 {
    let mut radius = 0.5f32;
    for part in body.parts.iter() {
        for vertex in part.mesh.positions.iter() {
            let d = nearest_joint_d(&body.bind.positions, *vertex);
            if d.is_finite() {
                radius = radius.max(d + 0.10);
            }
        }
    }
    radius
}

/// Sole depth of one rig in meters: how far the lowest bind vertex sits under
/// the lowest bind joint, floored at 0. The joints-only pose bounds its floor
/// side by the lowest placed joint less this depth, not by
/// [`skin_radius_m`]: the radius pads every side by a whole arm's reach, so an
/// idle body there read a metre under the sand. Cold path only; a non-finite
/// point never moves it.
pub(super) fn skin_sole_m(body: &SkinnedBody) -> f32 {
    let lowest = |points: &mut dyn Iterator<Item = [f32; 3]>| {
        points
            .map(|point| point[1])
            .filter(|y| y.is_finite())
            .fold(f32::MAX, f32::min)
    };
    let joint = lowest(&mut body.bind.positions.iter().copied());
    let vertex = lowest(
        &mut body
            .parts
            .iter()
            .flat_map(|part| part.mesh.positions.iter().copied()),
    );
    if joint == f32::MAX || vertex == f32::MAX {
        return 0.0;
    }
    (joint - vertex).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn family_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters/rocketbox")
    }

    /// A body at a person's height passes; one cooked in centimetres (the
    /// pelvis at 89 m) or lost under the floor is refused by name.
    #[test]
    fn a_body_in_the_wrong_units_is_refused() {
        check_body_units("hero", [0.0, 0.892, 0.0]).expect("the CT7 hero stands at 0.89 m");
        let giant = check_body_units("rb_legionary", [0.0, 89.5, 0.0]).expect_err("cm units");
        assert!(giant.to_string().contains("rb_legionary"), "{giant}");
        assert!(giant.to_string().contains("units"), "{giant}");
        check_body_units("flat", [0.0, 0.009, 0.0]).expect_err("a hundredth scale");
        check_body_units("nan", [0.0, f32::NAN, 0.0]).expect_err("no height");
    }

    /// Every realistic body the family manifest names loads with its own
    /// clips, and the look finds the joints its layers need by the CT7 names
    /// the cook gave them. A new avatar cooked into the tree is held to the
    /// same bar with no edit here.
    #[test]
    fn the_rocketbox_family_loads_with_its_own_clips() {
        let look = FighterLook::load_rocketbox(&family_dir()).expect("the Rocketbox family loads");
        assert!(!look.rigs.is_empty(), "at least one realistic variant");
        for rig in &look.rigs {
            assert_eq!(rig.clips.len(), 6, "idle, run, three attacks, death");
            assert!(rig.legs.is_some(), "the legs are found by name");
            assert!(rig.throw_arm.is_some(), "the throwing arm is found by name");
            assert!(
                rig.skeleton.joints.len() >= 45,
                "the Biped carries at least the CT7 joints"
            );
            for clip in &rig.clips {
                assert!(clip.duration > 0.1, "{} has a length", clip.name);
            }
            // The slash is not the idle: the attack slot moves the right arm.
            let arm = rig
                .skeleton
                .joints
                .iter()
                .position(|joint| joint.name == "arm_upper_r")
                .expect("the right upper arm");
            let attack = &rig.clips[2];
            let first = attack.rotations[arm].values.first().copied().expect("keys");
            let turned = attack.rotations[arm].values.iter().any(|q| {
                (q[0] - first[0]).abs() + (q[1] - first[1]).abs() + (q[2] - first[2]).abs() > 0.2
            });
            assert!(turned, "the right arm swings in the attack clip");
        }
    }

    /// The shipped cooked lane opens by the names `rig_cook` writes: one rig
    /// per variant, four atlas quadrants per cooked body, six hold clips that
    /// sample the bind pose. The hero also carries the graft and kit that
    /// `load_rig` adds after its quadrants. The body pins are checked by the
    /// loader.
    #[test]
    fn the_shipped_cooked_lane_loads_four_rigs_with_bind_holds() {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters/rig.e2bundle");
        let bytes = std::fs::read(&path).expect("the shipped cooked lane");
        let source =
            engine_assets::asset_source::BundleSource::open(bytes).expect("the lane opens");
        let look = FighterLook::load_cooked_from(&source).expect("the cooked lane loads");
        assert_eq!(look.rigs.len(), fighter_bind::VARIANTS.len());
        for (index, rig) in look.rigs.iter().enumerate() {
            assert_eq!(rig.variant, fighter_bind::VARIANTS[index]);
            assert_eq!(rig.skeleton.joints.len(), 45, "the CT7 rig");
            assert_eq!(rig.clips.len(), 6, "one hold clip per act");
            // The hero's gear (`fighter_gear::arm_hero`: a gladius and a scutum from the
            // CC-BY table when its files load, else the graft's four parts) and kit
            // (`fighter_kit::KIT_PARTS`, 5bb4d402d) follow its quadrants.
            let quadrants = engine_assets::rig_cook::QUADRANTS;
            let added = if rig.variant == "hero" {
                super::fighter_gear::armed_parts(&super::fighter_gear::table::source_dir())
                    + fighter_kit::KIT_PARTS
            } else {
                0
            };
            assert_eq!(
                rig.parts.len(),
                quadrants + added,
                "one part per atlas quadrant, then the hero's added parts"
            );
            assert!(
                rig.parts.iter().all(|part| part.texture.is_some()),
                "every part carries a base-colour map"
            );
            let pelvis = rig
                .skeleton
                .joints
                .iter()
                .position(|joint| joint.name == "pelvis")
                .expect("the pelvis joint");
            let keys = &rig.clips[0].positions.tracks[pelvis].values;
            let first = keys.first().copied().expect("the first key");
            let last = keys.last().copied().expect("the last key");
            for axis in 0..3 {
                assert!(
                    (first[axis] - last[axis]).abs() < 1.0e-6,
                    "the hold clip stays on its bind position"
                );
            }
        }
    }
}
