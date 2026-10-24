//! impostor_live: the presenter's door to the octahedral impostors (RL64).
//!
//! Owner: render lane. The device work is `present/impostor_gpu.rs`; this file
//! only lets the game bake a figure once, stage instances, and lets the live
//! frame bind the presenter's own camera, sun, fog and ambient buffers to the
//! draw it records inside the forward pass. Units: meters, radians, linear
//! colour. Presentation only, never hashed. Budget: one bake per mesh id and
//! layout at load; per frame one bind-group rebuild per live batch.

use super::super::PresentError;
use super::presenter::Presenter;
use crate::impostor::AtlasLayout;
use crate::impostor_gpu::{BakeMesh, FrameBuffers, ImpostorInstance, ImpostorStats, PassShape};

fn refuse(err: impl std::fmt::Display) -> PresentError {
    PresentError::Validation(format!("rl64 impostor: {err}"))
}

impl Presenter {
    /// Bake `mesh` into an octahedral atlas of `layout` under `key`, unless
    /// that key and layout are already resident (the cache key). `lit` shades
    /// the draw with the frame's sun and ambient; false returns the albedo
    /// alone. Returns true when a bake ran. Cold path: it waits for the device.
    pub fn bake_impostor(
        &mut self,
        key: u64,
        mesh: &BakeMesh,
        layout: AtlasLayout,
        lit: bool,
    ) -> Result<bool, PresentError> {
        self.impostor_sub
            .bake(&self.device, &self.queue, key, mesh, layout, lit)
            .map_err(refuse)
    }

    /// Replace the instances drawn for the mesh baked under `key`; an empty
    /// list clears them. The batch keeps them until the next call, so a still
    /// crowd stages once. View only, never hashed.
    pub fn set_impostor_instances(
        &mut self,
        key: u64,
        list: &[ImpostorInstance],
    ) -> Result<(), PresentError> {
        self.impostor_sub
            .set_instances(&self.device, &self.queue, key, list)
            .map_err(refuse)
    }

    /// Meshes baked, instances staged, draws recorded and atlas bytes.
    #[must_use]
    pub fn impostor_stats(&self) -> ImpostorStats {
        self.impostor_sub.stats()
    }

    /// The sphere `(centre, radius)` the figure under `key` was fitted to, in
    /// the figure's own space, if it is resident.
    #[must_use]
    pub fn impostor_figure(&self, key: u64) -> Option<([f32; 3], f32)> {
        self.impostor_sub
            .atlas(key)
            .map(|atlas| (atlas.centre_m, atlas.radius_m))
    }

    /// Wall milliseconds the bake of `key` took, if it is resident.
    #[must_use]
    pub fn impostor_bake_ms(&self, key: u64) -> Option<f64> {
        self.impostor_sub.atlas(key).map(|atlas| atlas.bake_ms)
    }

    /// Bind the frame's buffers to the staged batches and build the pipelines
    /// of both forward shapes (1x and 4x), so `encode_forward` records the
    /// draw without touching the device. A frame with nothing staged does
    /// nothing. Caller: the live frame, after its uniforms are written.
    pub(super) fn prepare_impostors(&mut self) {
        let bufs = FrameBuffers {
            cam: &self.cam_buf,
            light: &self.light_buf,
            fog: &self.fog_fwd_buf,
            sh: &self.ibl_sh_buf,
            prev_cam: &self.prev_cam_buf,
            steady_cam: &self.jitter.steady_buf,
        };
        self.impostor_sub.prepare(
            &self.device,
            &bufs,
            &[PassShape::Forward1x, PassShape::ForwardMsaa],
        );
    }
}
