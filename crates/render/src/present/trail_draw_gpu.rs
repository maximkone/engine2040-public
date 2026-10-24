//! trail_draw_gpu: staged draw bytes and device builders for the sword ribbon trail.
//!
//! Owner: render lane; split from `trail`, which keeps the CPU ring. Window build only.
//! Units: bytes in the WGSL layout, meters in the vert positions. Determinism: fixed
//! field order, no wall clock. Errors: none here; device validation surfaces through
//! the submitter. Budget: stack bytes for packing; the builders run once per draw state.

use crate::draw::TrailStripVert;
use crate::vfx_soft::{vfx_depth_layout, vfx_pack_frame, VfxFrame, VFX_FRAME_BYTES};

// ---------------------------------------------------------------------------
// Staged draw bytes (behind `window`): the CPU image of
// `shaders/trail.wgsl`, one uniform per frame plus the strip storage buffer.
// The packing plus the device builders live with the frame pass in
// `present::gpu_present::submit`; this module owns the layout contract.
// ---------------------------------------------------------------------------

/// Draw-uniform byte size for `shaders/trail.wgsl` (`TrailU`): one `mat4`
/// view-projection, the camera right and up axes, the width scale and the live
/// vert count (96 bytes, every `vec3` member on a 16-byte bound), then the
/// 32-byte [`VfxFrame`] block (depth constants, depth-valid word, light).
pub const TRAIL_DRAW_UNIFORM_BYTES: usize = 128;

/// Byte offset of the [`VfxFrame`] block inside the draw uniform.
const TRAIL_VFX_AT: usize = TRAIL_DRAW_UNIFORM_BYTES - VFX_FRAME_BYTES;

/// [`TRAIL_VFX_AT`] and the depth-valid word eight bytes in, as buffer
/// addresses for the queue writes (a test pins both to the layout).
const TRAIL_VFX_ADDR: u64 = 96;
const TRAIL_DEPTH_ON_ADDR: u64 = 104;

/// One staged vert: the `StripVert` image of `shaders/trail.wgsl` (position
/// plus alpha). 16 bytes per vert, so the storage buffer is 16 * vert_count.
pub const TRAIL_DRAW_VERT_BYTES: usize = 16;

/// Kenney ribbon draw shader over the fixed 16-sample ring; pool bounds and
/// the seeded stream are untouched. The draw path is the `shaders/trail.wgsl`
/// file; `TrailSubmitter` stages the live strip and the frame records it.
/// Engine-owned WGSL, fixed math only, no wall clock.
pub const TRAIL_DRAW_WGSL: &str = include_str!("../shaders/trail.wgsl");

/// Append one little-endian `f32` at `*at`; stack bytes only.
fn trail_put_f32(out: &mut [u8], at: &mut usize, v: f32) {
    out[*at..*at + 4].copy_from_slice(&v.to_le_bytes());
    *at += 4;
}

/// Pack the `TrailU` image: the frame view-projection in column-major order
/// (the image the forward camera uniform carries), the camera right and up
/// axes, the width scale, the staged vert count and the [`VfxFrame`] block.
/// Deterministic: fixed field order, no wall clock.
pub fn trail_pack_uniform(
    view_proj: [[f32; 4]; 4],
    cam_right: [f32; 3],
    cam_up: [f32; 3],
    width_scale: f32,
    vert_count: u32,
    vfx: &VfxFrame,
) -> [u8; TRAIL_DRAW_UNIFORM_BYTES] {
    let mut out = [0u8; TRAIL_DRAW_UNIFORM_BYTES];
    let mut at = 0usize;
    for col in view_proj {
        for v in col {
            trail_put_f32(&mut out, &mut at, v);
        }
    }
    for v in cam_right {
        trail_put_f32(&mut out, &mut at, v);
    }
    trail_put_f32(&mut out, &mut at, width_scale);
    for v in cam_up {
        trail_put_f32(&mut out, &mut at, v);
    }
    // The `vec3` up axis pads to 16 bytes; the last 4 bytes are the live count.
    out[at..at + 4].copy_from_slice(&vert_count.to_le_bytes());
    at += 4;
    debug_assert_eq!(at, TRAIL_VFX_AT);
    vfx_pack_frame(vfx, &mut out[at..]);
    out
}

/// Pack one staged [`TrailStripVert`] into the `StripVert` image: `pos` and
/// `alpha`. Deterministic: fixed order.
pub fn trail_pack_vert(vert: &TrailStripVert, out: &mut [u8]) {
    debug_assert_eq!(out.len(), TRAIL_DRAW_VERT_BYTES);
    let mut at = 0usize;
    for v in vert.pos_m {
        trail_put_f32(out, &mut at, v);
    }
    trail_put_f32(out, &mut at, vert.alpha);
    debug_assert_eq!(at, TRAIL_DRAW_VERT_BYTES);
}

/// Pack every staged vert into a contiguous byte buffer; one [`TRAIL_DRAW_VERT_BYTES`]
/// block per vert, in order. Deterministic: fixed order.
pub fn trail_pack_strip(verts: &[TrailStripVert]) -> Vec<u8> {
    let mut out = vec![0u8; verts.len() * TRAIL_DRAW_VERT_BYTES];
    for (i, vert) in verts.iter().enumerate() {
        let slice = &mut out[i * TRAIL_DRAW_VERT_BYTES..(i + 1) * TRAIL_DRAW_VERT_BYTES];
        trail_pack_vert(vert, slice);
    }
    out
}

/// Draw bind-group layout: the view uniform and the strip storage buffer.
/// Fixed shape, never rebuilt.
pub fn trail_draw_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let storage = || wgpu::BufferBindingType::Storage { read_only: true };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("rl53-trail-draw-layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                // The fragment reads `trail.light` and `trail.depth_on` too.
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(TRAIL_DRAW_UNIFORM_BYTES as u64),
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: storage(),
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(TRAIL_DRAW_VERT_BYTES as u64),
                },
                count: None,
            },
        ],
    })
}

/// Draw pipeline layout: the uniform and strip bind group, then the scene
/// depth the soft contact reads (`vfx_soft`, group 1).
pub fn trail_draw_pipeline_layout(
    device: &wgpu::Device,
    bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::PipelineLayout {
    let depth = vfx_depth_layout(device);
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("rl53-trail-draw-pipeline-layout"),
        bind_group_layouts: &[Some(bind_group_layout), Some(&depth)],
        immediate_size: 0,
    })
}

/// Draw render pipeline: triangle strip topology, one vertex shader reading
/// the strip storage, one fragment shader shaping the soft streak. The colour
/// target blends premultiplied (the shader's alpha word keeps part of the light
/// additive), and there is no depth attachment: the shader fades against the
/// stored depth itself, so the pass keeps the colour-only shape.
pub fn trail_draw_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    pipeline_layout: &wgpu::PipelineLayout,
    target_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("rl53-trail-draw-pipeline"),
        layout: Some(pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("trail_vs"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("trail_fs"),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// One device buffer with `usage` plus the copy destination the batch uploads
/// need.
#[cfg(feature = "gpu")]
pub fn trail_draw_buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage,
        mapped_at_creation: false,
    })
}

// ---------------------------------------------------------------------------
// The staged draw: one uniform, one max-size strip buffer and the strip
// pipeline, recorded into the HDR target after the lit draws. The particle
// submitter is the model: built on the first non-empty strip, rewritten in
// place after, never mapped or waited on.
// ---------------------------------------------------------------------------

/// Most verts one staged frame holds: eight slots (`MAX_TRAIL_SLOTS`) times
/// twelve pairs (`TRAIL_LEN`) times two verts a pair.
pub const TRAIL_DRAW_MAX_VERTS: usize = 8 * 12 * 2;

/// Ribbon width share of the blade length: the shader scales the half width
/// by the base-to-tip distance, so a long blade draws a wider streak.
pub const TRAIL_WIDTH_SCALE: f32 = 0.35;

/// Fewest verts that close one triangle; shorter strips draw nothing.
const TRAIL_MIN_STRIP_VERTS: u32 = 3;

/// Refused stage, named so the caller can report what it carried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrailDrawError {
    /// More verts than one strip buffer holds.
    OverCap {
        /// Verts the caller staged.
        requested: usize,
        /// Verts the buffer holds.
        allowed: usize,
    },
    /// The per-slot counts do not add up to the vert list.
    LenMismatch {
        /// Verts staged.
        verts: usize,
        /// Sum of the per-slot counts.
        summed: usize,
    },
    /// A camera axis, a strip position or an alpha is not finite.
    NotFinite,
}

impl std::fmt::Display for TrailDrawError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OverCap { requested, allowed } => {
                write!(f, "trail stage holds {requested} verts, cap {allowed}")
            }
            Self::LenMismatch { verts, summed } => {
                write!(f, "trail strip counts sum to {summed}, not {verts}")
            }
            Self::NotFinite => f.write_str("trail stage carried a non-finite value"),
        }
    }
}

/// Device state for the staged draw: the strip pipeline, the 96-byte uniform,
/// the max-size strip buffer and the one bind group that ties them together.
/// Built once, on the first non-empty strip; every later frame rewrites the
/// same buffers.
struct TrailDrawState {
    pipeline: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    strip: wgpu::Buffer,
    /// Staged strip bytes, reused across frames (cleared, never reallocated).
    packed: Vec<u8>,
    /// One `(first vert, vert count)` range per busy slot, so one draw never
    /// bridges two fighters.
    runs: Vec<(u32, u32)>,
}

/// Validate one frame's strip and fold it into per-slot draw ranges: one
/// `(first vert, vert count)` range per slot that closes a triangle, so one
/// draw never bridges two fighters. Over-cap frames, a count list that does
/// not add up, and short strips are folded here; only whole pairs draw, so an
/// odd trailing vert is dropped from its slot's range. Pure: no device, no
/// clock, one small bounded allocation.
pub fn trail_strip_runs(
    verts: &[TrailStripVert],
    strip_lens: &[u32],
) -> Result<Vec<(u32, u32)>, TrailDrawError> {
    if verts.len() > TRAIL_DRAW_MAX_VERTS {
        return Err(TrailDrawError::OverCap {
            requested: verts.len(),
            allowed: TRAIL_DRAW_MAX_VERTS,
        });
    }
    let summed = strip_lens.iter().fold(0usize, |sum, count| {
        sum.saturating_add(usize::try_from(*count).unwrap_or(usize::MAX))
    });
    if summed != verts.len() {
        return Err(TrailDrawError::LenMismatch {
            verts: verts.len(),
            summed,
        });
    }
    let mut runs = Vec::with_capacity(strip_lens.len());
    let mut start = 0u32;
    for count in strip_lens {
        let pairs = *count & !1;
        if pairs >= TRAIL_MIN_STRIP_VERTS {
            runs.push((start, pairs));
        }
        start = start.saturating_add(*count);
    }
    Ok(runs)
}

/// Reusable window submit state for the sword ribbon: the strip pipeline, the
/// frame uniform the camera fills, and the staged verts. Built once per frame
/// loop; a steady frame rewrites the same two buffers and creates nothing.
/// Never maps and never waits on the device.
pub struct TrailSubmitter {
    view: [u8; TRAIL_DRAW_UNIFORM_BYTES],
    draw: Option<TrailDrawState>,
    /// The frame constants the next stage packs behind the camera.
    vfx: VfxFrame,
    /// Group 1: the frame's stored depth, rebuilt with the targets. Without it
    /// the strip cannot draw (the pipeline layout asks for it).
    depth_bind: Option<wgpu::BindGroup>,
}

impl Default for TrailSubmitter {
    fn default() -> Self {
        Self::new()
    }
}

impl TrailSubmitter {
    /// Build the submitter: the draw camera starts at identity (right +X,
    /// up +Y) and no device state exists until the first non-empty strip.
    pub fn new() -> Self {
        Self {
            view: trail_pack_uniform(
                [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                TRAIL_WIDTH_SCALE,
                0,
                &VfxFrame::default(),
            ),
            draw: None,
            vfx: VfxFrame::default(),
            depth_bind: None,
        }
    }

    /// Hand the strip the frame's stored depth (the presenter rebuilds this
    /// with its targets). Cheap: one bind group.
    pub fn bind_depth(&mut self, depth_bind: wgpu::BindGroup) {
        self.depth_bind = Some(depth_bind);
    }

    /// True once a depth bind is in place.
    pub fn has_depth_bind(&self) -> bool {
        self.depth_bind.is_some()
    }

    /// The frame constants the next stage packs.
    pub fn vfx(&self) -> VfxFrame {
        self.vfx
    }

    /// Take the frame's depth constants and light. A non-finite block is
    /// refused (the last good one stays). Rewrites the uniform tail in place.
    pub fn set_vfx(&mut self, queue: &wgpu::Queue, vfx: VfxFrame) -> bool {
        if !crate::vfx_soft::vfx_frame_finite(&vfx) {
            return false;
        }
        self.vfx = vfx;
        vfx_pack_frame(&vfx, &mut self.view[TRAIL_VFX_AT..]);
        if let Some(draw) = self.draw.as_ref() {
            queue.write_buffer(&draw.uniform, TRAIL_VFX_ADDR, &self.view[TRAIL_VFX_AT..]);
        }
        true
    }

    /// Say whether the depth bind holds this frame's depth (the frame decides
    /// at record time: the 4x path keeps none). Rewrites one word of the live
    /// uniform; `stage` repacks the tail from the last [`Self::set_vfx`].
    pub fn set_depth_valid(&self, queue: &wgpu::Queue, valid: bool) {
        if let Some(draw) = self.draw.as_ref() {
            queue.write_buffer(
                &draw.uniform,
                TRAIL_DEPTH_ON_ADDR,
                &u32::from(valid).to_le_bytes(),
            );
        }
    }

    /// Buffers created so far (probe for the reuse test: flat across frames
    /// that stage the same shape).
    pub fn created_buffers(&self) -> u64 {
        if self.draw.is_some() {
            2
        } else {
            0
        }
    }

    /// Verts in the strip staged on the device; 0 draws nothing.
    pub fn staged_count(&self) -> u32 {
        self.draw
            .as_ref()
            .map_or(0, |draw| draw.runs.iter().map(|(_, count)| *count).sum())
    }

    /// True when the staged frame has at least one strip to draw.
    pub fn has_draw(&self) -> bool {
        self.draw.as_ref().is_some_and(|draw| !draw.runs.is_empty())
    }

    /// Stage one frame: the flat vert list plus the vert count of each slot's
    /// strip. Over-cap frames and non-finite values are refused before any
    /// device work; an empty frame clears the staged runs, so a calm frame
    /// issues no draw call. One draw range per slot keeps a strip from
    /// bridging two fighters. This path never maps and never waits.
    #[expect(
        clippy::too_many_arguments,
        reason = "device, queue, strip slices and the frame camera are the draw's inputs"
    )]
    pub fn stage(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        verts: &[TrailStripVert],
        strip_lens: &[u32],
        view_proj: [[f32; 4]; 4],
        cam_right: [f32; 3],
        cam_up: [f32; 3],
    ) -> Result<u32, TrailDrawError> {
        let runs = trail_strip_runs(verts, strip_lens)?;
        let finite = view_proj.iter().flatten().all(|c| c.is_finite())
            && cam_right.iter().chain(cam_up.iter()).all(|c| c.is_finite())
            && verts
                .iter()
                .all(|vert| vert.pos_m.iter().all(|c| c.is_finite()) && vert.alpha.is_finite());
        if !finite {
            return Err(TrailDrawError::NotFinite);
        }
        let total = u32::try_from(verts.len()).unwrap_or(u32::MAX);
        self.view = trail_pack_uniform(
            view_proj,
            cam_right,
            cam_up,
            TRAIL_WIDTH_SCALE,
            total,
            &self.vfx,
        );
        if verts.is_empty() {
            if let Some(draw) = self.draw.as_mut() {
                draw.runs.clear();
            }
            return Ok(0);
        }
        if self.draw.is_none() {
            self.draw = Some(TrailDrawState::build(device));
        }
        let Some(draw) = self.draw.as_mut() else {
            return Ok(0);
        };
        draw.packed.clear();
        draw.packed.resize(verts.len() * TRAIL_DRAW_VERT_BYTES, 0);
        for (index, vert) in verts.iter().enumerate() {
            let at = index * TRAIL_DRAW_VERT_BYTES;
            trail_pack_vert(vert, &mut draw.packed[at..at + TRAIL_DRAW_VERT_BYTES]);
        }
        queue.write_buffer(&draw.uniform, 0, &self.view);
        queue.write_buffer(&draw.strip, 0, &draw.packed);
        draw.runs = runs;
        Ok(total)
    }

    /// Record the staged strips into `pass`: one triangle-strip draw per busy
    /// slot, or nothing when the frame staged none, so a calm frame issues no
    /// draw call at all. The pass must carry one HDR colour target and no
    /// depth attachment, the shape the pipeline declares; the depth the soft
    /// contact reads rides group 1 ([`Self::bind_depth`]).
    pub fn encode_draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let (Some(draw), Some(depth)) = (self.draw.as_ref(), self.depth_bind.as_ref()) else {
            return;
        };
        if draw.runs.is_empty() {
            return;
        }
        pass.set_pipeline(&draw.pipeline);
        pass.set_bind_group(0, &draw.bind, &[]);
        pass.set_bind_group(crate::vfx_soft::VFX_DEPTH_GROUP, depth, &[]);
        for (start, count) in &draw.runs {
            pass.draw(*start..start + count, 0..1);
        }
    }
}

impl TrailDrawState {
    /// Build the draw state once: the pipeline over [`TRAIL_DRAW_WGSL`], the
    /// uniform, the max-size strip buffer and the bind group. Two buffers are
    /// what the create counter charges.
    fn build(device: &wgpu::Device) -> Self {
        let layout = trail_draw_layout(device);
        let pipe_layout = trail_draw_pipeline_layout(device, &layout);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lf14-trail-draw"),
            source: wgpu::ShaderSource::Wgsl(TRAIL_DRAW_WGSL.into()),
        });
        let pipeline =
            trail_draw_pipeline(device, &shader, &pipe_layout, crate::present::HDR_FORMAT);
        // Buffer sizes are layout constants; the `usize` to `u64` step is
        // total on every target the engine builds for.
        let uniform_bytes = u64::try_from(TRAIL_DRAW_UNIFORM_BYTES).unwrap_or(u64::MAX);
        let strip_bytes =
            u64::try_from(TRAIL_DRAW_MAX_VERTS * TRAIL_DRAW_VERT_BYTES).unwrap_or(u64::MAX);
        let uniform = trail_draw_buffer(
            device,
            "lf14-trail-uniform",
            uniform_bytes,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let strip = trail_draw_buffer(
            device,
            "lf14-trail-strip",
            strip_bytes,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lf14-trail-bind"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: strip.as_entire_binding(),
                },
            ],
        });
        Self {
            pipeline,
            bind,
            uniform,
            strip,
            packed: Vec::with_capacity(TRAIL_DRAW_MAX_VERTS * TRAIL_DRAW_VERT_BYTES),
            runs: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trail_pack_uniform_is_deterministic() {
        let view_proj = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let cam_right = [1.0, 0.0, 0.0];
        let cam_up = [0.0, 1.0, 0.0];
        let width_scale = 1.5;
        let vfx = VfxFrame::default();
        let a = trail_pack_uniform(view_proj, cam_right, cam_up, width_scale, 24, &vfx);
        let b = trail_pack_uniform(view_proj, cam_right, cam_up, width_scale, 24, &vfx);
        assert_eq!(a, b, "the same inputs pack the same uniform bytes");
        assert_eq!(a.len(), TRAIL_DRAW_UNIFORM_BYTES);
        assert_eq!(
            &a[TRAIL_VFX_AT - 4..TRAIL_VFX_AT],
            &24u32.to_le_bytes(),
            "the word before the frame block is the live vert count"
        );
        let mut tail = [0u8; VFX_FRAME_BYTES];
        vfx_pack_frame(&vfx, &mut tail);
        assert_eq!(
            &a[TRAIL_VFX_AT..],
            &tail,
            "the frame block closes the uniform"
        );
    }

    #[test]
    fn trail_pack_strip_is_deterministic() {
        let verts = vec![
            TrailStripVert {
                pos_m: [0.0, 0.0, 0.0],
                alpha: 0.0,
            },
            TrailStripVert {
                pos_m: [1.0, 0.0, 0.0],
                alpha: 1.0,
            },
        ];
        let a = trail_pack_strip(&verts);
        let b = trail_pack_strip(&verts);
        assert_eq!(a, b, "the same verts pack the same strip bytes");
        assert_eq!(a.len(), verts.len() * TRAIL_DRAW_VERT_BYTES);
    }

    fn strip(count: usize) -> Vec<TrailStripVert> {
        (0..count)
            .map(|i| TrailStripVert {
                pos_m: [i as f32 * 0.1, 1.0, 0.0],
                alpha: 1.0,
            })
            .collect()
    }

    #[test]
    fn trail_strip_runs_split_slots_and_drop_short_strips() {
        // Two busy slots and one single-pair slot: one range per drawable
        // slot, the pair short of a triangle draws nothing, and the second
        // range starts after the skipped slot's verts.
        let verts = strip(26);
        let runs = trail_strip_runs(&verts, &[24, 2]).expect("counts add up");
        assert_eq!(runs, vec![(0, 24)]);
        let runs = trail_strip_runs(&verts, &[4, 22]).expect("counts add up");
        assert_eq!(runs, vec![(0, 4), (4, 22)], "a strip never bridges slots");
    }

    #[test]
    fn trail_strip_runs_refuse_over_cap_and_mismatch() {
        let verts = strip(TRAIL_DRAW_MAX_VERTS + 2);
        assert_eq!(
            trail_strip_runs(&verts, &[TRAIL_DRAW_MAX_VERTS as u32 + 2]),
            Err(TrailDrawError::OverCap {
                requested: TRAIL_DRAW_MAX_VERTS + 2,
                allowed: TRAIL_DRAW_MAX_VERTS,
            })
        );
        let verts = strip(4);
        assert_eq!(
            trail_strip_runs(&verts, &[2]),
            Err(TrailDrawError::LenMismatch {
                verts: 4,
                summed: 2
            })
        );
    }

    #[test]
    fn trail_strip_runs_drop_an_odd_tail() {
        let verts = strip(5);
        let runs = trail_strip_runs(&verts, &[5]).expect("counts add up");
        assert_eq!(runs, vec![(0, 4)], "an odd last vert cannot close a pair");
    }

    #[test]
    fn trail_strip_runs_accept_an_empty_frame() {
        let runs = trail_strip_runs(&[], &[]).expect("an idle frame is valid");
        assert!(runs.is_empty(), "an idle frame draws nothing");
    }

    #[test]
    #[cfg(feature = "gpu")]
    fn trail_draw_stages_and_records_without_validation_errors() {
        let Some((device, queue, _info)) = crate::test_gpu::require_or_skip(
            "trail_draw_stages_and_records_without_validation_errors",
            crate::graph::gx1_probe(),
        ) else {
            return;
        };
        let mut submitter = TrailSubmitter::new();
        assert_eq!(submitter.created_buffers(), 0);
        assert!(!submitter.has_draw(), "a fresh submitter draws nothing");
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lf14-trail-draw-depth"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::present::DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor::default());
        submitter.bind_depth(crate::vfx_soft::vfx_depth_bind(device, &depth_view));
        assert!(submitter.has_depth_bind());
        let verts = strip(24);
        let staged = submitter
            .stage(
                device,
                queue,
                &verts,
                &[24],
                [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
            )
            .expect("the strip uploads");
        assert_eq!(staged, 24);
        assert_eq!(submitter.created_buffers(), 2, "two buffers, built once");
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let hdr = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lf14-trail-draw-target"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::present::HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = hdr.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("lf14-trail-draw-frame"),
        });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lf14-trail-draw-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            submitter.encode_draw(&mut pass);
        }
        queue.submit(Some(enc.finish()));
        assert!(
            pollster::block_on(scope.pop()).is_none(),
            "the staged strip must record a valid draw"
        );
        // A calm frame clears the staged runs, so nothing draws and nothing
        // new is created.
        assert_eq!(
            submitter
                .stage(
                    device,
                    queue,
                    &[],
                    &[],
                    [
                        [1.0, 0.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0, 0.0],
                        [0.0, 0.0, 0.0, 1.0],
                    ],
                    [1.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                )
                .expect("an idle frame stages nothing"),
            0
        );
        assert!(!submitter.has_draw());
        assert_eq!(
            submitter.created_buffers(),
            2,
            "a calm frame creates nothing"
        );
    }

    /// The packer writes each field at the offset naga gives it, so the frame
    /// uniform the shader reads is the image the CPU staged.
    #[cfg(feature = "shader-validate")]
    #[test]
    fn trail_uniform_layout_matches_the_packer() {
        let (span, fields) =
            engine_material::compile::uniform_struct_layout(TRAIL_DRAW_WGSL, "TrailU")
                .expect("trail.wgsl declares TrailU");
        assert_eq!(
            span,
            u32::try_from(TRAIL_DRAW_UNIFORM_BYTES).expect("small")
        );
        let offset = |name: &str| {
            fields
                .iter()
                .find(|(field, _)| field == name)
                .map(|(_, offset)| *offset)
                .unwrap_or_else(|| panic!("TrailU has no {name}"))
        };
        assert_eq!(offset("view_proj"), 0);
        assert_eq!(offset("cam_right"), 64);
        assert_eq!(offset("width_scale"), 76);
        assert_eq!(offset("cam_up"), 80);
        assert_eq!(offset("vert_count"), 92);
        let at = |bytes: usize| u32::try_from(bytes).expect("small");
        assert_eq!(offset("zparams"), at(TRAIL_VFX_AT));
        assert_eq!(offset("depth_on"), at(TRAIL_VFX_AT) + 8);
        assert_eq!(offset("light"), at(TRAIL_VFX_AT) + 16);
        assert_eq!(usize::try_from(TRAIL_VFX_ADDR), Ok(TRAIL_VFX_AT));
        assert_eq!(usize::try_from(TRAIL_DEPTH_ON_ADDR), Ok(TRAIL_VFX_AT + 8));

        let packed = trail_pack_uniform(
            [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            [0.25, 0.5, 0.75],
            [0.125, 0.375, 0.625],
            TRAIL_WIDTH_SCALE,
            42,
            &VfxFrame::default(),
        );
        let f32_at =
            |at: usize| f32::from_le_bytes(packed[at..at + 4].try_into().expect("4 bytes"));
        let u32_at =
            |at: usize| u32::from_le_bytes(packed[at..at + 4].try_into().expect("4 bytes"));
        let width_scale = usize::try_from(offset("width_scale")).expect("small");
        let cam_up = usize::try_from(offset("cam_up")).expect("small");
        let vert_count = usize::try_from(offset("vert_count")).expect("small");
        assert_eq!(f32_at(width_scale), TRAIL_WIDTH_SCALE);
        assert_eq!(
            [f32_at(cam_up), f32_at(cam_up + 4), f32_at(cam_up + 8)],
            [0.125, 0.375, 0.625]
        );
        assert_eq!(u32_at(vert_count), 42);
    }
}
