//! spot: the encode-only arm of the shadowed-spot bake.
//!
//! Owner: render lane. The live window frame records the spot pass into its
//! own one command buffer, so the frame keeps its no-wait shape (no poll, no
//! map, no submit of its own). The bake is the same one
//! [`super::submit_spot_atlas`] records; only the submit differs.
//! Presentation only: nothing here feeds the tick or the hash.
//!
//! Bounds: the same ones the submit arm holds - a triangle with any vertex
//! at or behind the spot near plane is dropped, more spots than atlas slots
//! and more triangles than `MAX_OCCLUDER_TRIS` degrade with a recorded
//! clamp, and bad input fails loudly with the lane's `LightError`.

#![forbid(unsafe_code)]

use super::{
    build_slot_clips, build_slot_geometry, encode_atlas_pass, upload_slots, LightError,
    OccluderTri, SpotAtlasConfig, SpotAtlasGpu, SpotLight, GATE_PASS, MAX_OCCLUDER_TRIS,
    SPOT_NEAR_M,
};

/// Encode-only receipt for the live frame's spot pass: the atlas bake was
/// recorded into the frame's own command buffer, so the pass rides the
/// frame's single submit and the frame keeps its no-wait shape (no poll, no
/// map, no submit of its own). The bake is the same one
/// [`super::submit_spot_atlas`] records; only the submit differs.
/// Presentation only, never hashed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpotEncode {
    /// Spots fed to the bake.
    pub spots: usize,
    /// Spots that baked a shadow tile.
    pub shadowed: usize,
    /// Atlas texels resident.
    pub texels: usize,
    /// Real wgpu passes recorded into the encoder (always one).
    pub passes_encoded: usize,
    /// Triangles dropped for crossing a spot near plane, as the CPU
    /// reference drops them too.
    pub dropped_tris: u32,
    /// True when a budget clamp fired before the encode.
    pub degraded: bool,
    /// Wall time of the whole call in microseconds: the CPU cost the live
    /// frame pays for the bake (evidence only).
    pub encode_us: u64,
}

impl SpotEncode {
    /// True when the bake reached the device (one pass recorded).
    pub fn ran(&self) -> bool {
        self.passes_encoded == 1
    }

    /// Pass name for the frame report and the live-frame gate.
    pub fn pass(&self) -> &'static str {
        GATE_PASS
    }
}

/// What the bake needs before the pass can be encoded: the accepted spot
/// count, the shadowed count, the dropped triangles and the degrade flag.
pub(super) struct AtlasPrep {
    /// Spots accepted (past the slot count the tail is dropped).
    pub(super) spots: usize,
    /// Spots that baked a shadow tile.
    pub(super) shadowed: usize,
    /// Triangles dropped for crossing a spot near plane.
    pub(super) dropped_tris: u32,
    /// True when a budget clamp fired before the bake.
    pub(super) degraded: bool,
}

/// Validate the frame, build one clip matrix per slot, build the accepted
/// geometry and upload both to the device. Shared by the standalone submit
/// and the live frame's encode: neither path submits or waits here, so the
/// live frame's bake rides its own command buffer.
pub(super) fn prepare_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    spots: &[SpotLight],
    tris: &[OccluderTri],
    config: SpotAtlasConfig,
    submitter: &mut SpotAtlasGpu,
) -> Result<AtlasPrep, LightError> {
    config.validate()?;
    let degraded = spots.len() > config.slots() || tris.len() > MAX_OCCLUDER_TRIS;
    let spots = &spots[..spots.len().min(config.slots())];
    let tris = &tris[..tris.len().min(MAX_OCCLUDER_TRIS)];
    let shadowed = build_slot_clips(spots, config.slots(), &mut submitter.clips)?;
    let dropped = build_slot_geometry(
        &mut submitter.vert_scratch,
        &mut submitter.runs,
        &submitter.clips,
        tris,
        SPOT_NEAR_M,
    );
    submitter.ensure_atlas(device, config)?;
    upload_slots(device, queue, submitter)?;
    Ok(AtlasPrep {
        spots: spots.len(),
        shadowed,
        dropped_tris: dropped,
        degraded,
    })
}

/// Encode the depth atlas into `encoder` without submitting it: the live
/// window frame records the spot pass in its own one command buffer, so the
/// frame keeps its no-wait shape (no poll, no map, no submit of its own).
/// The atlas texture, the slot uniforms and the accepted geometry are the
/// ones [`super::submit_spot_atlas`] uses, so both paths rasterize the same
/// depth. The caller owns the submit: the pass lands on the device with the
/// frame. Fails loudly with typed errors, same shape as
/// [`super::submit_spot_atlas`].
pub fn encode_spot_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    spots: &[SpotLight],
    tris: &[OccluderTri],
    config: SpotAtlasConfig,
    submitter: &mut SpotAtlasGpu,
    encoder: &mut wgpu::CommandEncoder,
) -> Result<SpotEncode, LightError> {
    let started = engine_clock::now();
    let prep = prepare_atlas(device, queue, spots, tris, config, submitter)?;
    encode_atlas_pass(submitter, config, encoder)?;
    Ok(SpotEncode {
        spots: prep.spots,
        shadowed: prep.shadowed,
        texels: config.texels(),
        passes_encoded: 1,
        dropped_tris: prep.dropped_tris,
        degraded: prep.degraded,
        encode_us: u64::try_from(engine_clock::since(started).as_micros()).unwrap_or(u64::MAX),
    })
}
