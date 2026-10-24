//! fighter_look_crowd: the figure the stands crowd is baked from.
//!
//! Consumer: [`crate::crowd_stands`] (bakes it once at load through the
//! presenter's impostor atlas). Data owner: the rigs the look already loaded;
//! this file only poses one of them at a clip time and hands its parts to the
//! baker. Units: meters, clip seconds. Determinism: presentation only; the
//! same rig and time give the same mesh bytes twice. Errors: typed
//! [`LookError`]. Budget: one skin per part at load, never per frame.

use engine_render::impostor_gpu::{BakeMesh, BakePart, BakeTexture};

use super::*;

impl FighterLook {
    fn crowd_rig(&self, variant: &str) -> Result<&Rig, LookError> {
        self.rigs
            .iter()
            .find(|rig| rig.variant == variant)
            .or_else(|| self.rigs.first())
            .ok_or_else(|| LookError::Skin("no rig to bake the crowd from".to_string()))
    }

    /// One rig posed at `time_s` of clip `clip` (0 is the idle guard), in its
    /// own space with the feet on the ground: the joint blend every part skins
    /// through.
    fn crowd_pose(
        rig: &Rig,
        clip: usize,
        time_s: f32,
    ) -> Result<engine_render::v1_skin::SkinningUniform, LookError> {
        let clip = rig
            .clips
            .get(clip)
            .ok_or_else(|| LookError::Skin("crowd clip missing".to_string()))?;
        let mut pose: LocalPose =
            sample_pose_with_rots(&rig.skeleton, &clip.positions, &clip.rotations, time_s)
                .map_err(|err| LookError::Skin(format!("{err:?}")))?;
        pose.positions[rig.pelvis][0] = rig.bind_pelvis[0];
        pose.positions[rig.pelvis][2] = rig.bind_pelvis[2];
        let world = skinning_matrices(&rig.skeleton, &pose)
            .map_err(|err| LookError::Skin(format!("{err:?}")))?;
        let matrices: Vec<[[f32; 4]; 4]> = world
            .iter()
            .zip(rig.inverse_binds.iter())
            .map(|(joint, inverse)| mul(inverse, joint))
            .collect();
        rig.pipe
            .skinning_uniform(&matrices)
            .map_err(|err| LookError::Skin(format!("{err:?}")))
    }

    /// The crowd figure: the `variant` body (the first rig when the name is
    /// unknown) skinned at `time_s` of clip `clip`, one [`BakePart`] per body
    /// part. `tunic` lists the part indices the stands recolour per seat;
    /// `skip` the parts left out of the figure (weapons). Cold path.
    pub fn crowd_figure(
        &self,
        variant: &str,
        clip: usize,
        time_s: f32,
        tunic: &[usize],
        skip: &[usize],
    ) -> Result<BakeMesh, LookError> {
        let rig = self.crowd_rig(variant)?;
        let uniform = Self::crowd_pose(rig, clip, time_s)?;
        let mut parts = Vec::with_capacity(rig.parts.len());
        for (index, part) in rig.parts.iter().enumerate() {
            if skip.contains(&index) {
                continue;
            }
            let posed = self.skin_part(rig, part, None, &uniform)?;
            parts.push(BakePart {
                positions: posed.positions,
                normals: posed.normals,
                uvs: part.mesh.uvs.clone(),
                indices: part.mesh.indices.clone(),
                base_color: part.material.base_color,
                texture: part.texture.as_ref().map(|(_, upload)| BakeTexture {
                    width: upload.width,
                    height: upload.height,
                    rgba: upload.rgba.clone(),
                }),
                tunic: tunic.contains(&index),
            });
        }
        Ok(BakeMesh { parts })
    }

    /// One line per part of the `variant` body posed at `time_s` of clip
    /// `clip`: index, vertex and triangle counts, the posed bounds, the map
    /// size and the factor. The stands read it to choose tunic and skip parts.
    pub fn crowd_figure_report(
        &self,
        variant: &str,
        clip: usize,
        time_s: f32,
    ) -> Result<Vec<String>, LookError> {
        let rig = self.crowd_rig(variant)?;
        let uniform = Self::crowd_pose(rig, clip, time_s)?;
        let mut lines = vec![format!(
            "crowd figure {} clips {}",
            rig.variant,
            rig.clips.len()
        )];
        for (index, part) in rig.parts.iter().enumerate() {
            let posed = self.skin_part(rig, part, None, &uniform)?;
            let mut lo = [f32::MAX; 3];
            let mut hi = [f32::MIN; 3];
            for p in &posed.positions {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
            let map = part.texture.as_ref().map_or_else(
                || "none".to_string(),
                |(_, up)| format!("{}x{}", up.width, up.height),
            );
            lines.push(format!(
                "part {index}: verts {} tris {} lo {lo:.2?} hi {hi:.2?} map {map} factor {:.2?}",
                posed.positions.len(),
                part.mesh.indices.len() / 3,
                part.material.base_color,
            ));
        }
        Ok(lines)
    }
}
