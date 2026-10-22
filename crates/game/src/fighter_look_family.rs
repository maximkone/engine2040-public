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
        let ragdoll = Self::ragdoll_for_rigs(&rigs);
        let capes = Capes::for_rigs(&rigs);
        Ok(Self {
            rigs,
            ring: None,
            slots: vec![SlotAnim::new(); MAX_LOOK_FIGHTERS],
            bridge: bridge_slots(),
            trails: TrailRing::new(),
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
}
