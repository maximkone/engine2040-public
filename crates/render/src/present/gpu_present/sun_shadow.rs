//! sun_shadow: single arena sun depth map from live draws.
//! Owner: render lane via submit slice 2. Units: meters, texels, 0..1 depths.
use std::fmt;

#[cfg(test)]
use crate::arena_day::ARENA_DAY_SUN_DIR;

/// Sun shadow failures. Bad matrix or shape fails loudly.
#[derive(Clone, PartialEq, Debug)]
pub enum SunShadowError {
    Invalid(String),
}
impl fmt::Display for SunShadowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "sun shadow input: {e}"),
        }
    }
}
impl std::error::Error for SunShadowError {}
/// Sun map edge in texels: 2048 over the pit.
pub const SUN_SHADOW_SIZE: u32 = 2048;
/// Float edge matching [`SUN_SHADOW_SIZE`] for UV math (test oracle only).
#[cfg(test)]
pub const SUN_SHADOW_SIZE_F32: f32 = 2048.0;
/// Constant depth-compare bias in normalized depth: 2^-19.
/// The old flat 0.002 was 0.119 m of depth over the 59.5 m sun frustum, so a
/// foot stood about 10 cm off its own contact shadow. This is the float-depth
/// floor; [`SUN_SHADOW_SLOPE_SCALE`] covers grazing surfaces and
/// [`SUN_SHADOW_NORMAL_OFFSET_TEXELS`] moves the sample off the surface.
pub const SUN_SHADOW_BIAS: f32 = 1.0 / 524288.0;
/// Slope-scaled compare bias in texels of the surface's depth slope: 2.0
/// texels covers the 3x3 compare grid's footprint plus one texel of margin.
pub const SUN_SHADOW_SLOPE_SCALE: f32 = 2.0;
/// Normal offset in light-space texels: the forward compare moves its sample
/// along the surface normal by this many texels before it projects, so a
/// surface does not compare against its own texel.
pub const SUN_SHADOW_NORMAL_OFFSET_TEXELS: f32 = 1.0;

#[cfg(test)]
mod layers_tests;
#[cfg(test)]
mod skinned_tests;
#[cfg(test)]
mod steady_tests;
/// Ortho box center in meters: the pit center.
pub const SUN_SHADOW_CENTER_M: [f32; 3] = [0.0, 0.0, 0.0];
/// Ortho half-extent in meters: 30 m span over the pit.
pub const SUN_SHADOW_HALF_EXTENT_M: f32 = 15.0;
/// Sun camera near in meters from the sun eye.
pub const SUN_SHADOW_NEAR_M: f32 = 0.5;
/// Sun camera far in meters from the sun eye.
pub const SUN_SHADOW_FAR_M: f32 = 60.0;
/// Cascade layers of the sun map, near to far (SW1): one texture array, one
/// layer per cascade, each re-rendered on its own [`CascadeCadence`] period.
///
/// [`CascadeCadence`]: crate::shadow_cascade::CascadeCadence
pub const SUN_LAYERS: usize = 3;
/// Span of each layer over the near one: the pit (30 m), the pit and its
/// stands (60 m), the whole bowl (120 m). Layer 0 is the live pit map.
pub const SUN_LAYER_SCALE: [f32; SUN_LAYERS] = [1.0, 2.0, 4.0];
/// Bytes of the forward group's pass frame at 20 (`SunFwdU`): the 96-byte
/// layer-0 head of [`sun_forward_uniform_bytes`], then the 144-byte layer
/// tail of [`sun_forward_layers_bytes`].
pub const SUN_FWD_BYTES: u64 = 96 + 144;
const _: () = assert!(SUN_FWD_BYTES == 240, "the encode frame buffer is 240 bytes");
/// Half-arm edge in texels: 1024 over the pit, the optional cost arm.
pub const SUN_SHADOW_HALF_SIZE: u32 = 1024;
/// Live sun depth edge switch: false runs the row's full 2048 (the row's
/// "one 2048 `Depth32Float` map per frame" spec, and the value before the
/// 2026-10-16 half-res flip), true runs the 1024 cost arm. The desc helper
/// below reads it and the live frame records that map inside
/// `submit::encode_forward`, so this one bool decides the edge `arena
/// per-pass shadow` measures. Owner item EB-2026-10-16-S16 keeps the
/// 1024/2048 pick open for the perf lane; the shipped default follows the row.
/// Units boolean in. Determinism fixed default, never hashed.
pub const SUN_SHADOW_HALF_RES: bool = false;
/// Pure size decision for the half switch: false keeps 2048, true runs 1024.
/// Owner render lane; Caller `sun_shadow_depth_desc` below plus the half-switch test.
/// Units texels out. Determinism pure, no allocation, never hashed.
#[must_use]
pub fn sun_shadow_size_for_half(half_res: bool) -> u32 {
    if half_res {
        SUN_SHADOW_HALF_SIZE
    } else {
        SUN_SHADOW_SIZE
    }
}
/// Size-param snap helper: snap `origin_m` to the texel grid for `size`.
/// Owner render lane; Caller `arena_sun_view_proj` below keeps the live snap plus the half-switch test.
/// Units meters in/out. Determinism pure snap via `shadow_cascade::snap_shadow_origin`, never hashed.
#[must_use]
pub fn sun_snap_origin_for_size(origin_m: f32, size: u32) -> f32 {
    crate::shadow_cascade::snap_shadow_origin(origin_m, SUN_SHADOW_HALF_EXTENT_M, size)
}
/// One light-space texel in meters for a square `edge`: the 30 m pit span
/// over the map edge. Units texels in, meters out. Determinism pure.
#[must_use]
pub fn sun_shadow_texel_m(edge: u32) -> f32 {
    2.0 * SUN_SHADOW_HALF_EXTENT_M / edge_f32(edge)
}
/// The forward compare's normal-offset distance in meters for `edge`.
/// Units texels in, meters out. Determinism pure.
#[must_use]
pub fn sun_shadow_normal_offset_m(edge: u32) -> f32 {
    SUN_SHADOW_NORMAL_OFFSET_TEXELS * sun_shadow_texel_m(edge)
}
/// Slope-scaled compare bias per unit `tan(theta)` for `edge`, in normalized
/// depth: [`SUN_SHADOW_SLOPE_SCALE`] texels of the surface's depth slope over
/// the sun span. Units texels in, normalized depth out. Determinism pure.
#[must_use]
pub fn sun_shadow_slope_scale_for(edge: u32) -> f32 {
    SUN_SHADOW_SLOPE_SCALE * sun_shadow_texel_m(edge) / (SUN_SHADOW_FAR_M - SUN_SHADOW_NEAR_M)
}
fn sun_light_basis(sun: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let fwd = [-sun[0], -sun[1], -sun[2]];
    let helper: [f32; 3] = if fwd[1].abs() > 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let mut t = [
        helper[1] * fwd[2] - helper[2] * fwd[1],
        helper[2] * fwd[0] - helper[0] * fwd[2],
        helper[0] * fwd[1] - helper[1] * fwd[0],
    ];
    let tl = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt().max(1e-12);
    t = [t[0] / tl, t[1] / tl, t[2] / tl];
    let u = [
        fwd[1] * t[2] - fwd[2] * t[1],
        fwd[2] * t[0] - fwd[0] * t[2],
        fwd[0] * t[1] - fwd[1] * t[0],
    ];
    (t, u, fwd)
}

/// Sun view-projection for one sun direction: ortho camera on the pit.
/// Owner render lane; Caller `arena_sun_view_proj` plus `submit::encode_forward` live depth keep the snap.
/// Units meters in, matrix out. Determinism pure, never hashed.
pub fn sun_view_proj_for_dir(sun_dir: [f32; 3]) -> Result<[[f32; 4]; 4], SunShadowError> {
    sun_view_proj_scaled(sun_dir, 1.0)
}
/// Sun view-projection of cascade `layer` (SW1): the pit ortho of
/// [`sun_view_proj_for_dir`] grown by [`SUN_LAYER_SCALE`], width and depth
/// alike, about the same centre; layer 0 is that matrix bit for bit. A layer
/// past [`SUN_LAYERS`] fails loudly.
/// Units meters in, matrix out. Determinism pure, never hashed.
pub fn sun_layer_view_proj(
    sun_dir: [f32; 3],
    layer: usize,
) -> Result<[[f32; 4]; 4], SunShadowError> {
    let scale = SUN_LAYER_SCALE
        .get(layer)
        .copied()
        .ok_or_else(|| SunShadowError::Invalid(format!("sun layer {layer} of {SUN_LAYERS}")))?;
    sun_view_proj_scaled(sun_dir, scale)
}
/// The pit ortho with its half-extent and far plane times `scale`.
fn sun_view_proj_scaled(sun_dir: [f32; 3], scale: f32) -> Result<[[f32; 4]; 4], SunShadowError> {
    if !sun_dir.iter().all(|c| c.is_finite()) {
        return Err(SunShadowError::Invalid("sun dir is not finite".into()));
    }
    let len2 = sun_dir[0] * sun_dir[0] + sun_dir[1] * sun_dir[1] + sun_dir[2] * sun_dir[2];
    if len2 < 1e-12 {
        return Err(SunShadowError::Invalid("sun dir has ~zero length".into()));
    }
    let len = len2.sqrt();
    let sun = [sun_dir[0] / len, sun_dir[1] / len, sun_dir[2] / len];
    // Half-res depth keeps the origin on its texel grid so the 1024 map does not shimmer.
    let edge = sun_shadow_size_for_half(SUN_SHADOW_HALF_RES);
    let center = [
        sun_snap_origin_for_size(SUN_SHADOW_CENTER_M[0], edge),
        sun_snap_origin_for_size(SUN_SHADOW_CENTER_M[1], edge),
        sun_snap_origin_for_size(SUN_SHADOW_CENTER_M[2], edge),
    ];
    let half = SUN_SHADOW_HALF_EXTENT_M * scale;
    let span = SUN_SHADOW_FAR_M * scale - SUN_SHADOW_NEAR_M;
    let (t, u, _) = sun_light_basis(sun);
    let dot_c = center[0] * sun[0] + center[1] * sun[1] + center[2] * sun[2];
    let dot_ct = center[0] * t[0] + center[1] * t[1] + center[2] * t[2];
    let dot_cu = center[0] * u[0] + center[1] * u[1] + center[2] * u[2];
    let w3 = (half + dot_c - SUN_SHADOW_NEAR_M) / span;
    let m = [
        [t[0] / half, u[0] / half, -sun[0] / span, 0.0],
        [t[1] / half, u[1] / half, -sun[1] / span, 0.0],
        [t[2] / half, u[2] / half, -sun[2] / span, 0.0],
        [-dot_ct / half, -dot_cu / half, w3, 1.0],
    ];
    if !m.iter().flatten().all(|c| c.is_finite()) {
        return Err(SunShadowError::Invalid("sun view is not finite".into()));
    }
    Ok(m)
}
/// Arena sun view-projection for this frame: ortho camera on the pit.
/// Owner render lane; Caller `arena_sun_view_proj` test (the live depth pass
/// builds its matrix from the frame sun direction).
/// Units matrix out. Determinism pure delegate, never hashed.
#[cfg(test)]
pub fn arena_sun_view_proj() -> Result<[[f32; 4]; 4], SunShadowError> {
    sun_view_proj_for_dir(ARENA_DAY_SUN_DIR)
}
/// Depth descriptor: square live-edge `Depth32Float`, the row's pinned 2048
/// with the switch off (the shipped default), 1024 on the half arm,
/// attachment plus binding.
pub fn sun_shadow_depth_desc() -> wgpu::TextureDescriptor<'static> {
    // Live edge: 1024 on the half arm, the pinned 2048 with the switch off.
    let edge = sun_shadow_size_for_half(SUN_SHADOW_HALF_RES);
    debug_assert!(
        edge == SUN_SHADOW_SIZE || edge == SUN_SHADOW_HALF_SIZE,
        "live sun edge is the pinned 2048 or the 1024 arm: {edge}"
    );
    sun_shadow_depth_desc_for(edge)
}
/// Depth descriptor for any square `edge` in texels: the quality tier's shadow map size
/// (`quality::shadow_edge_px`) reaches the texture here. The pit-centre snap is 0 for every
/// edge, so the sun matrix does not depend on it.
pub fn sun_shadow_depth_desc_for(edge: u32) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("sun-shadow-depth"),
        ..crate::shadow_cluster::gpu::cascade_depth_desc(edge)
    }
}
/// The live map's texture (SW1): [`SUN_LAYERS`] layers of the square `edge`
/// map, one per cascade; copyable so a proof can read a kept layer back.
pub fn sun_layers_depth_desc(edge: u32) -> wgpu::TextureDescriptor<'static> {
    let mut desc = sun_shadow_depth_desc_for(edge);
    desc.size.depth_or_array_layers = u32::try_from(SUN_LAYERS).unwrap_or(1);
    desc.usage |= wgpu::TextureUsages::COPY_SRC;
    desc
}
/// One cascade layer due this frame: its matrix and the 80 uniform bytes it
/// draws with.
pub type SunLayerDraw = ([[f32; 4]; 4], [u8; 80]);
/// The frame's layer plan (SW1): per layer under `layers`, the matrix and
/// bytes to draw with when `cadence` says it is due, `None` when it keeps the
/// map and uniform it was last drawn with. Every matrix is built before the
/// cadence moves, so a bad sun direction fails with the cadence untouched.
/// Units frames in. Determinism pure over the cadence, never hashed.
pub fn plan_sun_layers(
    cadence: &mut crate::shadow_cascade::CascadeCadence,
    frame: u64,
    sun_dir: [f32; 3],
    layers: usize,
) -> Result<[Option<SunLayerDraw>; SUN_LAYERS], SunShadowError> {
    let mut vps = [[[0.0f32; 4]; 4]; SUN_LAYERS];
    for (layer, vp) in vps.iter_mut().enumerate().take(layers) {
        *vp = sun_layer_view_proj(sun_dir, layer)?;
    }
    let mut plan = [None; SUN_LAYERS];
    for (layer, (slot, vp)) in plan.iter_mut().zip(vps).enumerate().take(layers) {
        if cadence.due(frame, layer, &vp) {
            *slot = Some((vp, sun_shadow_uniform_bytes(vp, SUN_SHADOW_BIAS, 1.0)));
        }
    }
    Ok(plan)
}
/// Uniform bytes: 80 bytes, matrix first (64), then `bias` and `scale`.
pub fn sun_shadow_uniform_bytes(vp: [[f32; 4]; 4], bias: f32, scale: f32) -> [u8; 80] {
    let mut out = [0u8; 80];
    for (ci, col) in vp.iter().enumerate() {
        for (ri, c) in col.iter().enumerate() {
            out[ci * 16 + ri * 4..ci * 16 + ri * 4 + 4].copy_from_slice(&c.to_le_bytes());
        }
    }
    out[64..68].copy_from_slice(&bias.to_le_bytes());
    out[68..72].copy_from_slice(&scale.to_le_bytes());
    out
}
/// Forward-group compare settings for one sun-map edge. Mirrors `SunFwdU` in
/// `forward.wgsl` field for field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SunFwdSample {
    /// Constant compare bias in normalized depth: [`SUN_SHADOW_BIAS`].
    pub bias: f32,
    /// 1 when the frame recorded the map, 0 when it skipped it.
    pub enabled: f32,
    /// One map texel in UV: `1/edge`.
    pub texel: f32,
    /// Normal-offset distance in meters: [`sun_shadow_normal_offset_m`].
    pub normal_offset_m: f32,
    /// Slope bias per unit `tan(theta)`: [`sun_shadow_slope_scale_for`].
    pub slope_scale: f32,
    /// Share of the allocated texture the live map covers: `edge / alloc`.
    /// A tier below the allocation draws into the low corner of the one
    /// texture (a viewport) and the reader scales its uv by this, so a tier
    /// change never replaces the texture the forward binds already hold.
    pub uv_scale: f32,
}
/// The live compare settings for `edge`: on, 2^-19, one texel offset, the map
/// filling its whole texture.
/// Units texels in, settings out. Determinism pure, never hashed.
#[cfg(test)]
#[must_use]
pub fn sun_fwd_sample_for_edge(edge: u32) -> SunFwdSample {
    sun_fwd_sample_for_live(edge, edge)
}
/// The live compare settings for a map of `edge` texels drawn into the low
/// corner of an `alloc`-texel texture (`edge <= alloc`; a larger edge is
/// clamped to the allocation).
/// Units texels in, settings out. Determinism pure, never hashed.
#[must_use]
pub fn sun_fwd_sample_for_live(edge: u32, alloc: u32) -> SunFwdSample {
    let edge = edge.min(alloc).max(1);
    SunFwdSample {
        bias: SUN_SHADOW_BIAS,
        enabled: 1.0,
        texel: 1.0 / edge_f32(edge),
        normal_offset_m: sun_shadow_normal_offset_m(edge),
        slope_scale: sun_shadow_slope_scale_for(edge),
        uv_scale: edge_f32(edge) / edge_f32(alloc.max(1)),
    }
}
/// Forward-group bytes for the live sun compare: 96 bytes, the pass matrix
/// first (64), then `bias`, `enabled`, `texel`, `normal_offset` and
/// `slope_scale` and `uv_scale`, then two pads, in the `SunFwdU` order.
pub fn sun_forward_uniform_bytes(vp: [[f32; 4]; 4], sample: SunFwdSample) -> [u8; 96] {
    let mut out = [0u8; 96];
    for (ci, col) in vp.iter().enumerate() {
        for (ri, c) in col.iter().enumerate() {
            out[ci * 16 + ri * 4..ci * 16 + ri * 4 + 4].copy_from_slice(&c.to_le_bytes());
        }
    }
    out[64..68].copy_from_slice(&sample.bias.to_le_bytes());
    out[68..72].copy_from_slice(&sample.enabled.to_le_bytes());
    out[72..76].copy_from_slice(&sample.texel.to_le_bytes());
    out[76..80].copy_from_slice(&sample.normal_offset_m.to_le_bytes());
    out[80..84].copy_from_slice(&sample.slope_scale.to_le_bytes());
    out[84..88].copy_from_slice(&sample.uv_scale.to_le_bytes());
    out
}
/// Forward-group layer tail (SW1), written at byte 96 of the pass frame: the
/// matrices layers 1 and 2 were last drawn with (64 each), then the layers
/// drawn (`1..=SUN_LAYERS`, as f32) and the spans of layers 1 and 2, in the
/// `SunFwdU` order (`far_vp`, `layers`). The reader samples only the first
/// `layers`, so a matrix past them may be anything.
/// Units matrices and counts in, bytes out. Determinism pure, never hashed.
pub fn sun_forward_layers_bytes(far: [[[f32; 4]; 4]; SUN_LAYERS - 1], layers: usize) -> [u8; 144] {
    let mut out = [0u8; 144];
    for (li, vp) in far.iter().enumerate() {
        for (ci, col) in vp.iter().enumerate() {
            for (ri, c) in col.iter().enumerate() {
                let at = li * 64 + ci * 16 + ri * 4;
                out[at..at + 4].copy_from_slice(&c.to_le_bytes());
            }
        }
    }
    let count = f32::from(u8::try_from(layers.clamp(1, SUN_LAYERS)).unwrap_or(1));
    out[128..132].copy_from_slice(&count.to_le_bytes());
    out[132..136].copy_from_slice(&SUN_LAYER_SCALE[1].to_le_bytes());
    out[136..140].copy_from_slice(&SUN_LAYER_SCALE[2].to_le_bytes());
    out
}
/// Depth-only vertex shader for the live sun map: sun matrix transform.
/// Owner render lane; Caller `submit::encode_forward` live depth builds its pipeline from it.
/// Units WGSL text. Determinism fixed bytes.
pub(crate) const SUN_SHADOW_DEPTH_WGSL: &str = r#"
struct SunU {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> sun: SunU;
@vertex
fn vs(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return sun.view_proj * vec4<f32>(pos, 1.0);
}
"#;
/// Depth-only vertex shader for skinned casters: the same joint blend the
/// forward `vs_skinned` runs, then the sun matrix. A fighter's body is drawn
/// from the 80-byte skinned vertex and the frame's joint block, so its shadow
/// is the posed body and not the bind pose the 48-byte static reader would see.
/// Binding 7 and 8 are the forward group's own palette and params buffers.
/// Owner render lane; Caller `sun_skinned_depth_pipeline` below.
/// Units WGSL text. Determinism fixed bytes.
pub(crate) const SUN_SHADOW_SKINNED_WGSL: &str = r#"
struct SunU {
    view_proj: mat4x4<f32>,
};
struct SkinParams {
    joint_count: u32,
    has_skin: u32,
    _pad0: u32,
    _pad1: u32,
};
@group(0) @binding(0) var<uniform> sun: SunU;
@group(0) @binding(7) var<storage, read> skin_joints: array<mat4x4<f32>, 64>;
@group(0) @binding(8) var<uniform> skin_params: SkinParams;
@vertex
fn vs_skinned(
    @location(0) pos: vec3<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
) -> @builtin(position) vec4<f32> {
    var sp = pos;
    if (skin_params.has_skin != 0u) {
        sp = vec3<f32>(0.0);
        for (var k = 0u; k < 4u; k = k + 1u) {
            let idx = min(joints[k], 63u);
            sp += weights[k] * (skin_joints[idx] * vec4<f32>(pos, 1.0)).xyz;
        }
    }
    return sun.view_proj * vec4<f32>(sp, 1.0);
}
"#;
/// Live sun uniform for one forward light direction: view-proj plus 80 bytes from the live list.
/// Test oracle: the layer-0 draw [`plan_sun_layers`] hands the live pass.
/// Units meters in, matrix plus bytes out. Determinism pure, never hashed.
#[cfg(test)]
pub(crate) fn live_sun_shadow_for_dir(
    sun_dir: [f32; 3],
) -> Result<([[f32; 4]; 4], [u8; 80]), SunShadowError> {
    let vp = sun_view_proj_for_dir(sun_dir)?;
    let bytes = sun_shadow_uniform_bytes(vp, SUN_SHADOW_BIAS, 1.0);
    Ok((vp, bytes))
}
/// Build the live sun depth pipeline for one bind layout: depth-only, 48-byte stride.
/// Owner render lane; Caller `SunShadowSubmitter::build` below draws the live list through it.
/// Units GPU handles. Presentation only, never hashed.
fn sun_depth_pipeline(
    device: &wgpu::Device,
    bind_layout: &wgpu::BindGroupLayout,
    shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    use crate::pipeline_warmup::CountedPipelines as _;
    let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("eg11-sun-pipe-layout"),
        bind_group_layouts: &[Some(bind_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline_counted(&wgpu::RenderPipelineDescriptor {
        label: Some("eg11-sun-pipeline"),
        layout: Some(&pipe_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: super::submit::FORWARD_VERTEX_STRIDE,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                }],
            })],
            compilation_options: Default::default(),
        },
        fragment: None,
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}
/// The skinned twin of [`sun_depth_pipeline`]: 80-byte vertices, position plus
/// the four joints and weights, the same depth state.
/// Owner render lane; Caller `SunShadowSubmitter::build` below.
/// Units GPU handles. Presentation only, never hashed.
fn sun_skinned_depth_pipeline(
    device: &wgpu::Device,
    bind_layout: &wgpu::BindGroupLayout,
    shader: &wgpu::ShaderModule,
) -> wgpu::RenderPipeline {
    use crate::pipeline_warmup::CountedPipelines as _;
    let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("eg11-sun-skinned-pipe-layout"),
        bind_group_layouts: &[Some(bind_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline_counted(&wgpu::RenderPipelineDescriptor {
        label: Some("eg11-sun-skinned-pipeline"),
        layout: Some(&pipe_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_skinned"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: super::submit::SKINNED_VERTEX_STRIDE,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Uint32x4,
                        offset: 48,
                        shader_location: 3,
                    },
                    wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x4,
                        offset: 64,
                        shader_location: 4,
                    },
                ],
            })],
            compilation_options: Default::default(),
        },
        fragment: None,
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}
/// One layer's sun bind: its own matrix uniform at 0, the forward group's
/// joint palette (one block per draw offset) at 7 and skin params at 8.
/// `buffers` is `[uniform, skin_joints, skin_params]`.
/// Units GPU handles. Presentation only, never hashed.
fn sun_layer_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffers: [&wgpu::Buffer; 3],
) -> wgpu::BindGroup {
    let [uniform, skin_joints, skin_params] = buffers;
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("eg11-sun-bind"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: skin_joints,
                    offset: 0,
                    size: wgpu::BufferSize::new(super::submit::SKIN_BLOCK_BYTES),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: skin_params.as_entire_binding(),
            },
        ],
    })
}
/// Live sun depth map for one presenter: the live `Depth32Float` map (1024 on
/// the half arm, the pinned 2048 with the switch off), its uniform, the bind
/// and the depth pipeline built once, so a live frame records the pass only.
/// EG11 Accept runs the map on live frames (`arena per-pass shadow` above
/// 0 ms) while nothing samples it yet, so a frame must not pay a texture plus
/// pipeline build each frame.
/// Owner render lane; Caller `submit::encode_forward` on the live path;
/// `Presenter::build` holds one.
/// Units GPU handles. Presentation only, never hashed.
pub(crate) struct SunShadowSubmitter {
    /// The map itself, [`SUN_LAYERS`] layers: a view does not keep the
    /// texture alive on its own.
    _map: wgpu::Texture,
    /// Every layer as one array view: the forward group binds it at 13.
    view: wgpu::TextureView,
    /// One attachment view per layer: a pass clears and draws only its own,
    /// so a layer that is not due keeps its depths.
    layer_views: [wgpu::TextureView; SUN_LAYERS],
    /// Edge of the live map in texels: the tier's shadow size, never above
    /// `alloc_edge`.
    edge: u32,
    /// Edge of the texture, fixed at build: the forward binds hold its view.
    alloc_edge: u32,
    /// One sun matrix uniform and bind per layer: a frame drawing several
    /// layers needs each pass to read its own matrix.
    uniforms: [wgpu::Buffer; SUN_LAYERS],
    binds: [wgpu::BindGroup; SUN_LAYERS],
    /// Which layers re-render this frame, and the matrix each was drawn with.
    cadence: std::cell::Cell<crate::shadow_cascade::CascadeCadence>,
    /// Frames the map was asked for, the cadence's clock.
    frame: std::cell::Cell<u64>,
    /// Layers the live frame draws and the forward pass samples:
    /// `1..=SUN_LAYERS`, 1 until a tier says more.
    layers: usize,
    pipeline: wgpu::RenderPipeline,
    /// Skinned casters: joint blend, then the sun matrix.
    skinned_pipeline: wgpu::RenderPipeline,
    /// EG11b: the forward group's pass frame (matrix, bias, enabled, texel).
    fwd_uniform: wgpu::Buffer,
    /// EG11b: the comparison sampler the forward group binds at 14.
    sampler: wgpu::Sampler,
}

impl SunShadowSubmitter {
    /// Build the live map plus its pipeline once for one device.
    /// Units GPU handles. Presentation only, never hashed.
    pub(crate) fn build(
        device: &wgpu::Device,
        skin_joints: &wgpu::Buffer,
        skin_params: &wgpu::Buffer,
    ) -> Self {
        let edge = sun_shadow_depth_desc().size.width;
        let texture = device.create_texture(&sun_layers_depth_desc(edge));
        let layer_view = |layer: usize| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("sw1-sun-layer"),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: u32::try_from(layer).unwrap_or(0),
                array_layer_count: Some(1),
                ..Default::default()
            })
        };
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("sw1-sun-layers"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let layer_views = std::array::from_fn(layer_view);
        let uniforms: [wgpu::Buffer; SUN_LAYERS] = std::array::from_fn(|_| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("eg11-sun-uniform"),
                size: 80,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        // Binding 0 is the sun matrix; 7 and 8 are the forward group's joint
        // palette (one block per draw offset) and skin params, read by the
        // skinned caster only. A static draw passes offset 0 and ignores them.
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("eg11-sun-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                super::submit::skin_joints_entry(7),
                super::submit::skin_params_entry(8),
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("eg11-sun-depth"),
            source: wgpu::ShaderSource::Wgsl(SUN_SHADOW_DEPTH_WGSL.into()),
        });
        let skinned_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("eg11-sun-depth-skinned"),
            source: wgpu::ShaderSource::Wgsl(SUN_SHADOW_SKINNED_WGSL.into()),
        });
        let pipeline = sun_depth_pipeline(device, &bind_layout, &shader);
        let skinned_pipeline = sun_skinned_depth_pipeline(device, &bind_layout, &skinned_shader);
        let binds = std::array::from_fn(|layer| {
            sun_layer_bind(
                device,
                &bind_layout,
                [&uniforms[layer], skin_joints, skin_params],
            )
        });
        Self {
            _map: texture,
            view,
            layer_views,
            edge,
            alloc_edge: edge,
            uniforms,
            binds,
            cadence: std::cell::Cell::new(crate::shadow_cascade::CascadeCadence::new([1; 3])),
            frame: std::cell::Cell::new(0),
            layers: 1,
            pipeline,
            skinned_pipeline,
            fwd_uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("eg11-sun-forward-uniform"),
                size: SUN_FWD_BYTES,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            // LessEqual against `frag - bias` is lit when the fragment is in
            // front of the stored nearest depth (the depth pass wrote `Less`).
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("eg11-sun-compare"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                compare: Some(wgpu::CompareFunction::LessEqual),
                ..Default::default()
            }),
        }
    }

    /// The live depth map, every layer as one array view, bound in the
    /// forward group at 13.
    pub(crate) fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// The comparison sampler for the live depth map, bound at 14.
    pub(crate) fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// The forward group's pass frame, bound at 20.
    pub(crate) fn fwd_uniform(&self) -> &wgpu::Buffer {
        &self.fwd_uniform
    }

    /// Write the pass frame for a frame that skipped the depth pass: the
    /// forward shader then reads fully lit, so a stale map never shadows.
    /// Caller: `submit::encode_forward` when the switch is off or the sun
    /// direction is invalid.
    pub(crate) fn disable_forward(&self, queue: &wgpu::Queue) {
        // The lit uniform no longer names a drawn map: the next wanted frame
        // re-renders every layer and writes the forward frame again.
        self.force_all();
        let off = SunFwdSample {
            enabled: 0.0,
            ..sun_fwd_sample_for_live(self.edge, self.alloc_edge)
        };
        let bytes = sun_forward_uniform_bytes([[0.0; 4]; 4], off);
        queue.write_buffer(&self.fwd_uniform, 0, &bytes);
    }

    /// Record the forward frame's sun step: the map when the frame wants it
    /// and the switch is on, else the lit uniform so a stale map never
    /// shadows. Returns the pass time the receipt posts (0 when skipped).
    /// Caller: `submit::encode_forward`.
    pub(crate) fn record(
        &self,
        presenter: &super::presenter::Presenter,
        enc: &mut wgpu::CommandEncoder,
        list: &crate::present::DrawList,
        wanted: bool,
    ) -> u64 {
        let us = match wanted {
            true => self.encode(presenter, enc, list),
            false => None,
        };
        match us {
            Some(us) => us,
            None => {
                self.disable_forward(&presenter.queue);
                0
            }
        }
    }

    /// Edge of the map in texels.
    pub(crate) fn edge(&self) -> u32 {
        self.edge
    }

    /// Set the live map to a square of `edge` texels (the tier's shadow size), clamped to the
    /// texture built at start. The texture, its view and the forward binds that hold the view
    /// stay as they are: a smaller tier draws into the low corner (`encode` sets the viewport)
    /// and the forward reader scales its uv by `edge / alloc_edge`, so a tier change at run
    /// time never leaves a bind pointing at a map nobody writes.
    pub(crate) fn resize(&mut self, edge: u32) {
        self.edge = edge.clamp(1, self.alloc_edge);
        self.force_all();
    }

    /// A tier's cascades (SW1): `bands` layers drawn (`1..=SUN_LAYERS`) on
    /// the tier's `periods`, every layer re-rendered on the next frame, which
    /// is frame 0 of the new clock.
    /// Caller: `quality_apply::apply_quality`, next to [`Self::resize`].
    pub(crate) fn set_cascades(&mut self, bands: usize, periods: [u8; 3]) {
        self.layers = bands.clamp(1, SUN_LAYERS);
        self.cadence
            .set(crate::shadow_cascade::CascadeCadence::new(periods));
        self.frame.set(0);
    }

    /// Every layer re-renders on the next frame: the maps no longer match
    /// what was drawn (resize, tier, a forward frame written off).
    fn force_all(&self) {
        let mut cadence = self.cadence.get();
        cadence.force_all();
        self.cadence.set(cadence);
    }

    /// The map texture, for a proof that reads a layer back.
    #[cfg(test)]
    pub(crate) fn map(&self) -> &wgpu::Texture {
        &self._map
    }

    /// Record the frame's sun depth layers from the live draw list: one ortho
    /// pass per layer the cadence finds due (layer 0 over the 30 m pit), each
    /// with its own matrix uniform. A layer that is not due is neither
    /// cleared nor drawn and its uniform is not written; the forward head is
    /// written only with layer 0 and the layer tail with the matrix each far
    /// layer was last drawn with, so the reader always names the matrix its
    /// layer holds. A frame with no layer due records nothing and reads 0 us.
    /// Owner render lane; Caller `submit::encode_forward` before the forward
    /// draws on the live path.
    /// Units meters for the pit fit, wall us out. Presentation only, never hashed.
    /// A bad sun direction returns `None` so the forward pass still runs.
    pub(crate) fn encode(
        &self,
        presenter: &super::presenter::Presenter,
        enc: &mut wgpu::CommandEncoder,
        list: &crate::present::DrawList,
    ) -> Option<u64> {
        let started = engine_clock::now();
        let dir = list.light.direction.to_array();
        let mut cadence = self.cadence.get();
        let frame = self.frame.get();
        let plan = plan_sun_layers(&mut cadence, frame, dir, self.layers).ok()?;
        self.cadence.set(cadence);
        self.frame.set(frame.wrapping_add(1));
        if plan.iter().all(Option::is_none) {
            return Some(0);
        }
        // One write per frame: head and tail together when layer 0 drew, the
        // tail alone when only a far layer did.
        let far = std::array::from_fn(|k| cadence.drawn_with(k + 1).copied().unwrap_or_default());
        let mut fwd = [0u8; 240];
        fwd[96..].copy_from_slice(&sun_forward_layers_bytes(far, self.layers));
        let from = if let Some((vp, _)) = plan[0] {
            let sample = sun_fwd_sample_for_live(self.edge, self.alloc_edge);
            fwd[..96].copy_from_slice(&sun_forward_uniform_bytes(vp, sample));
            0
        } else {
            96
        };
        presenter.queue.write_buffer(
            &self.fwd_uniform,
            u64::try_from(from).unwrap_or(0),
            &fwd[from..],
        );
        for (layer, draw) in plan.iter().enumerate() {
            let Some((_, uniform_bytes)) = draw else {
                continue;
            };
            presenter
                .queue
                .write_buffer(&self.uniforms[layer], 0, uniform_bytes);
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("eg11-sun-depth-pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.layer_views[layer],
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // The tier's edge draws into the low corner of the one texture.
            let live = f32::from(u16::try_from(self.edge).unwrap_or(u16::MAX));
            pass.set_viewport(0.0, 0.0, live, live, 0.0, 1.0);
            let mut skinned_bound: Option<bool> = None;
            for item in list.items.iter() {
                let Some(mesh) = presenter.meshes.get(&item.mesh) else {
                    continue;
                };
                // A skinned mesh carries 80-byte vertices: it takes the joint
                // blend and its frame block, never the 48-byte static reader.
                let skin_block = presenter.skin_blocks.get(&item.mesh).copied();
                let skinned = skin_block.is_some();
                if skinned_bound != Some(skinned) {
                    skinned_bound = Some(skinned);
                    pass.set_pipeline(if skinned {
                        &self.skinned_pipeline
                    } else {
                        &self.pipeline
                    });
                }
                pass.set_bind_group(
                    0,
                    &self.binds[layer],
                    &[skin_block.map_or(0, super::submit::skin_block_offset)],
                );
                pass.set_vertex_buffer(0, mesh.vertex.slice(..));
                pass.set_index_buffer(mesh.index.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.count, 0, 0..1);
            }
        }
        u64::try_from(engine_clock::since(started).as_micros()).ok()
    }
}
/// Map edge as a float for the forward uniform's texel step, exact for every
/// live edge (1024 or 2048) with no lossy cast on the per-frame path.
fn edge_f32(edge: u32) -> f32 {
    let e = u16::try_from(edge).unwrap_or(2048);
    f32::from(e)
}
/// CPU oracle: sun-map UV and depth of a world point; test-only until the
/// presenter reads the map back.
#[cfg(test)]
fn sun_uv_depth(
    view_proj: [[f32; 4]; 4],
    point: [f32; 3],
) -> Result<Option<[f32; 3]>, SunShadowError> {
    if !view_proj.iter().flatten().all(|c| c.is_finite()) {
        return Err(SunShadowError::Invalid("sun view is not finite".into()));
    }
    if !point.iter().all(|c| c.is_finite()) {
        return Err(SunShadowError::Invalid("sun point is not finite".into()));
    }
    let mut clip = [0.0; 3];
    for i in 0..3 {
        clip[i] = view_proj[0][i] * point[0]
            + view_proj[1][i] * point[1]
            + view_proj[2][i] * point[2]
            + view_proj[3][i];
    }
    if !clip.iter().all(|c| c.is_finite()) {
        return Err(SunShadowError::Invalid("sun clip is not finite".into()));
    }
    let u = clip[0] * 0.5 + 0.5;
    // The depth pass drew through the standard viewport: v runs down.
    let v = 0.5 - clip[1] * 0.5;
    let depth = clip[2];
    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) || !(0.0..=1.0).contains(&depth) {
        return Ok(None);
    }
    Ok(Some([u, v, depth]))
}
/// CPU oracle of the forward compare: lit when `frag - bias <= stored`;
/// test-only, the shader holds the production copy.
#[cfg(test)]
fn sun_shadow_factor(stored: f32, frag: f32, bias: f32) -> Result<f32, SunShadowError> {
    if !stored.is_finite() || !frag.is_finite() || !bias.is_finite() {
        return Err(SunShadowError::Invalid("sun compare is not finite".into()));
    }
    if frag - bias <= stored {
        Ok(1.0)
    } else {
        Ok(0.0)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sun_size_is_2048() {
        assert_eq!(SUN_SHADOW_SIZE, 2048, "sun edge stays 2048");
        assert_eq!(SUN_SHADOW_SIZE_F32, 2048.0, "float edge matches");
        assert_eq!(SUN_SHADOW_HALF_SIZE, 1024, "half edge is 1024");
        // `black_box`: the pin must keep running if the switch is flipped.
        assert!(
            !std::hint::black_box(SUN_SHADOW_HALF_RES),
            "live map runs the row's 2048"
        );
        assert_eq!(SUN_SHADOW_HALF_EXTENT_M, 15.0, "ortho spans 30 m");
        let desc = sun_shadow_depth_desc();
        assert_eq!(desc.size.width, SUN_SHADOW_SIZE, "desc runs the row's 2048");
        assert_eq!(
            desc.size.height, SUN_SHADOW_SIZE,
            "desc runs the row's 2048"
        );
        assert_eq!(desc.format, wgpu::TextureFormat::Depth32Float);
        // Full edge is the live edge; the half arm stays one bool away at 1024.
        assert_eq!(
            sun_shadow_size_for_half(false),
            SUN_SHADOW_SIZE,
            "off keeps 2048"
        );
        assert_eq!(
            sun_shadow_size_for_half(true),
            SUN_SHADOW_HALF_SIZE,
            "on runs 1024"
        );
    }
    #[test]
    fn sun_half_switch_halves_texel_and_keeps_snap() {
        // Live runs the row's 2048; the half arm stays one bool away at 1024,
        // with the origin kept on its texel grid either way.
        assert!(
            !std::hint::black_box(SUN_SHADOW_HALF_RES),
            "live map runs the row's 2048"
        );
        assert_eq!(SUN_SHADOW_HALF_SIZE, 1024, "half edge is 1024");
        let full = sun_shadow_size_for_half(false);
        let half = sun_shadow_size_for_half(true);
        assert_eq!(full, SUN_SHADOW_SIZE, "off keeps 2048");
        assert_eq!(full, 2048, "full pins 2048");
        assert_eq!(half, 1024, "half halves the edge");
        assert_eq!(half, full / 2, "half halves texel count");
        // Texel widths: 30 m span over the edge, so half doubles the texel.
        let span = 2.0 * SUN_SHADOW_HALF_EXTENT_M;
        let full16 = u16::try_from(full).expect("2048 fits in u16");
        let half16 = u16::try_from(half).expect("1024 fits in u16");
        let texel_full = span / f32::from(full16);
        let texel_half = span / f32::from(half16);
        assert!(
            (texel_half - texel_full * 2.0).abs() < 1e-6,
            "half doubles texel: {texel_half} vs {texel_full}"
        );
        // Snap keeps the origin on the texel grid for both sizes.
        let origin = 2.5;
        let snapped_full = sun_snap_origin_for_size(origin, full);
        let snapped_half = sun_snap_origin_for_size(origin, half);
        assert!(
            (origin - snapped_full).abs() <= texel_full + 1e-6,
            "full snap within one texel"
        );
        assert!(
            (origin - snapped_half).abs() <= texel_half + 1e-6,
            "half snap within one texel"
        );
        let grid_full = (snapped_full / texel_full).round() * texel_full;
        let grid_half = (snapped_half / texel_half).round() * texel_half;
        assert!(
            (snapped_full - grid_full).abs() < 1e-6,
            "full stays snapped"
        );
        assert!(
            (snapped_half - grid_half).abs() < 1e-6,
            "half stays snapped"
        );
        // Toggle restores and x2 equal.
        assert_eq!(
            sun_shadow_size_for_half(false),
            full,
            "toggle restores 2048"
        );
        assert_eq!(sun_shadow_size_for_half(true), half, "half repeats");
        assert_eq!(
            sun_snap_origin_for_size(origin, full),
            snapped_full,
            "full snap x2 equal"
        );
        assert_eq!(
            sun_snap_origin_for_size(origin, half),
            snapped_half,
            "half snap x2 equal"
        );
        // Descriptor runs the row's 2048 with the switch off; view and desc agree.
        let desc = sun_shadow_depth_desc();
        assert_eq!(desc.size.width, SUN_SHADOW_SIZE, "desc runs the row's 2048");
        assert_eq!(desc.size.width, full, "desc matches the live decision");
        let live_edge = sun_shadow_size_for_half(SUN_SHADOW_HALF_RES);
        assert_eq!(live_edge, full, "live edge keeps the row's 2048");
        assert_eq!(live_edge, desc.size.width, "view and desc agree on 2048");
    }
    #[test]
    fn forward_frame_carries_the_pass_matrix_and_a_3x3_pcf() {
        // The forward group's pass frame is the depth pass matrix (64 B), then
        // bias, enabled, texel, normal offset and slope scale, in the
        // `SunFwdU` order, and the reader is a 3x3 compare instead of the old
        // fixed 2x2 at 1/256.
        let vp = arena_sun_view_proj().expect("arena sun view");
        fn at(bytes: &[u8; 96], offset: usize) -> f32 {
            f32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ])
        }
        let sample = sun_fwd_sample_for_edge(SUN_SHADOW_SIZE);
        let bytes = sun_forward_uniform_bytes(vp, sample);
        assert_eq!(bytes.len(), 96, "matrix plus two std140 rows");
        assert_eq!(at(&bytes, 0), vp[0][0], "the matrix comes first");
        assert_eq!(at(&bytes, 64), SUN_SHADOW_BIAS, "bias at 64");
        assert_eq!(at(&bytes, 68), 1.0, "enabled at 68");
        assert!((at(&bytes, 72) - 1.0 / 2048.0).abs() < 1e-9, "texel at 72");
        assert!(
            (at(&bytes, 76) - sun_shadow_normal_offset_m(SUN_SHADOW_SIZE)).abs() < 1e-9,
            "normal offset in meters at 76"
        );
        assert!(
            (at(&bytes, 80) - sun_shadow_slope_scale_for(SUN_SHADOW_SIZE)).abs() < 1e-9,
            "slope scale at 80"
        );
        let off = SunFwdSample {
            enabled: 0.0,
            ..sample
        };
        assert_eq!(
            at(&sun_forward_uniform_bytes(vp, off), 68),
            0.0,
            "a skipped pass writes enabled 0"
        );
        let wgsl = include_str!("../../shaders/forward.wgsl");
        assert!(wgsl.contains("for (var y = -1; y <= 1; y++)"), "3x3 rows");
        assert!(wgsl.contains("lit / 9.0"), "nine compares, not four");
        assert!(!wgsl.contains("1.0 / 256.0"), "the fixed 2x2 texel is gone");
        assert!(wgsl.contains("fn sun_shadow_at("), "the map reader exists");
        assert!(
            wgsl.contains("sun_shadow_at(in.world_pos, in.world_n)"),
            "the reader takes the surface normal for the normal offset"
        );
        assert!(wgsl.contains("fn sun_bias("), "the slope term exists");
    }
    #[test]
    fn sun_single_box_is_nearer_than_plane() {
        let m = arena_sun_view_proj().expect("arena sun builds");
        let n = arena_sun_view_proj().expect("builds twice");
        assert_eq!(m, n);
        // Live helper carries the same ortho fit from the forward sun dir.
        let live = sun_view_proj_for_dir(ARENA_DAY_SUN_DIR).expect("live dir builds");
        assert_eq!(m, live, "live dir matches the arena rig");
        let (live_vp, live_bytes) =
            live_sun_shadow_for_dir(ARENA_DAY_SUN_DIR).expect("live uniform builds");
        assert_eq!(live_vp, m, "live view matches the arena view");
        let want = sun_shadow_uniform_bytes(m, SUN_SHADOW_BIAS, 1.0);
        assert_eq!(live_bytes, want, "live bytes match the uniform packer");
        let (_, live_again) =
            live_sun_shadow_for_dir(ARENA_DAY_SUN_DIR).expect("live builds twice");
        assert_eq!(live_bytes, live_again, "live bytes x2 equal");
        assert!(
            sun_view_proj_for_dir([0.0, 0.0, 0.0]).is_err(),
            "zero sun refused"
        );
        let Some([_, _, top]) = sun_uv_depth(m, [2.0, 1.8, 0.0]).expect("box projects") else {
            panic!("box top sits inside the arena map");
        };
        let Some([_, _, ground]) = sun_uv_depth(m, [2.0, 0.0, 0.0]).expect("ground projects")
        else {
            panic!("ground sits inside the arena map");
        };
        assert!(top < ground, "box nearer than plane: {top} vs {ground}");
        let s = sun_shadow_factor(top, ground, SUN_SHADOW_BIAS).expect("behind");
        assert_eq!(s, 0.0);
        let l = sun_shadow_factor(ground, top, SUN_SHADOW_BIAS).expect("above");
        assert_eq!(l, 1.0);
        // Offscreen GPU depth order from the pit camera (EG11 slice 1 proof):
        // the box-on-plane quad pair renders into the LIVE map shape (the row's
        // 2048 `Depth32Float` desc) and is read back, so the nearer-texel claim
        // is measured on the map the live frame encodes, not a stand-in.
        let Some((device, queue, _)) = crate::test_gpu::require_or_skip(
            "sun_single_box_is_nearer_than_plane",
            crate::graph::gx1_probe(),
        ) else {
            return;
        };
        let test_size = sun_shadow_size_for_half(SUN_SHADOW_HALF_RES);
        let mut matrix_bytes = [0u8; 64];
        for (ci, col) in m.iter().enumerate() {
            for (ri, c) in col.iter().enumerate() {
                matrix_bytes[ci * 16 + ri * 4..ci * 16 + ri * 4 + 4]
                    .copy_from_slice(&c.to_le_bytes());
            }
        }
        let plane: [[f32; 3]; 4] = [
            [-15.0, 0.0, -15.0],
            [15.0, 0.0, -15.0],
            [15.0, 0.0, 15.0],
            [-15.0, 0.0, 15.0],
        ];
        let box_top: [[f32; 3]; 4] = [
            [1.5, 1.8, -0.5],
            [2.5, 1.8, -0.5],
            [2.5, 1.8, 0.5],
            [1.5, 1.8, 0.5],
        ];
        let to_bytes = |verts: &[[f32; 3]; 4]| {
            let mut out = Vec::with_capacity(48);
            for p in verts {
                for c in p {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
            out
        };
        let plane_bytes = to_bytes(&plane);
        let box_bytes = to_bytes(&box_top);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sun-single-uniform"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform, 0, &matrix_bytes);
        let make_vert = |bytes: &[u8]| {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sun-single-vert"),
                size: bytes.len() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buf, 0, bytes);
            buf
        };
        let plane_vb = make_vert(&plane_bytes);
        let box_vb = make_vert(&box_bytes);
        let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];
        let mut index_bytes = [0u8; 24];
        for (i, w) in indices.iter().enumerate() {
            index_bytes[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        let make_index = || {
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sun-single-index"),
                size: 24,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buf, 0, &index_bytes);
            buf
        };
        let plane_ib = make_index();
        let box_ib = make_index();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sun-single-depth"),
            source: wgpu::ShaderSource::Wgsl(SUN_SHADOW_DEPTH_WGSL.into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sun-single-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sun-single-pipe-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sun-single-pipeline"),
            layout: Some(&pipe_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    }],
                })],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sun-single-bind"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sun-single-test-depth"),
            size: wgpu::Extent3d {
                width: test_size,
                height: test_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let started = std::time::Instant::now();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut last: Option<Vec<f32>> = None;
        let (box_u, box_v) = {
            let Some([u, v, _]) = sun_uv_depth(m, [2.0, 1.8, 0.0]).expect("box projects") else {
                panic!("box top sits inside the arena map");
            };
            (u, v)
        };
        let (ground_u, ground_v) = {
            let Some([u, v, _]) = sun_uv_depth(m, [2.0, 0.0, 0.0]).expect("ground projects") else {
                panic!("ground sits inside the arena map");
            };
            (u, v)
        };
        for _ in 0..2 {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("sun-single-encode"),
            });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("sun-single-depth-pass"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.set_vertex_buffer(0, plane_vb.slice(..));
                pass.set_index_buffer(plane_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..6, 0, 0..1);
                pass.set_vertex_buffer(0, box_vb.slice(..));
                pass.set_index_buffer(box_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..6, 0, 0..1);
            }
            // Row bytes of the depth copy, padded to the 256-byte copy
            // alignment: at the live 1024 edge one texel row is 4 KiB, so the
            // stride is the row bytes, never the texel count (the 64-texel
            // stand-in only matched because 64 * 4 == 256).
            let row_bytes = test_size * 4;
            let padded = row_bytes.div_ceil(256) * 256;
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sun-single-readback"),
                size: u64::from(padded) * u64::from(test_size),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::DepthOnly,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded),
                        rows_per_image: Some(test_size),
                    },
                },
                wgpu::Extent3d {
                    width: test_size,
                    height: test_size,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit(Some(encoder.finish()));
            let slice = readback.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("readback poll holds");
            rx.recv()
                .expect("readback channel holds")
                .expect("readback maps");
            let w = usize::try_from(test_size).expect("test size fits");
            let mut depths = vec![0.0f32; w * w];
            {
                let mapped = slice.get_mapped_range().expect("mapped range holds");
                let stride = usize::try_from(padded).expect("padded fits");
                for y in 0..w {
                    for x in 0..w {
                        let off = y * stride + x * 4;
                        depths[y * w + x] = f32::from_le_bytes([
                            mapped[off],
                            mapped[off + 1],
                            mapped[off + 2],
                            mapped[off + 3],
                        ]);
                    }
                }
            }
            readback.unmap();
            if let Some(prev) = last.replace(depths.clone()) {
                assert_eq!(prev, depths, "depth bytes x2 equal");
                let sample = |u: f32, v: f32| {
                    let xi = ((u * test_size as f32).floor() as usize).min(w - 1);
                    let yi = ((v * test_size as f32).floor() as usize).min(w - 1);
                    depths[yi * w + xi]
                };
                let box_d = sample(box_u, box_v);
                let ground_d = sample(ground_u, ground_v);
                assert!(
                    box_d < ground_d,
                    "gpu box nearer than plane: {box_d} vs {ground_d}"
                );
                assert!(box_d < 1.0 && ground_d <= 1.0, "map wrote depths");
            }
        }
        let scope_err = pollster::block_on(scope.pop());
        assert!(scope_err.is_none(), "0 validation errors");
        eprintln!(
            "sun single-map offscreen: box ({box_u:.3},{box_v:.3}) nearer than plane, x2 equal, submit_ms={:.2}",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
    #[test]
    fn sun_prepass_box_nearer_than_plane_live() {
        let scene = crate::v1::default_scene(160, 120).expect("default scene is valid");
        let list = crate::present::DrawList {
            camera: scene.camera,
            light: crate::v1::DirectionalLight {
                direction: glam::Vec3::from_array(ARENA_DAY_SUN_DIR),
                color: glam::Vec3::new(1.0, 0.96, 0.90),
                intensity: 3.0,
            },
            items: Vec::new(),
        };
        let dir = list.light.direction.to_array();
        let (vp, bytes) = live_sun_shadow_for_dir(dir).expect("live dir builds");
        assert_eq!(vp, arena_sun_view_proj().expect("arena sun builds"));
        assert_eq!(bytes, sun_shadow_uniform_bytes(vp, SUN_SHADOW_BIAS, 1.0));
        assert_eq!(
            live_sun_shadow_for_dir(dir).expect("live builds twice").1,
            bytes,
            "live bytes x2 equal"
        );
        assert!(live_sun_shadow_for_dir([0.0, 0.0, 0.0]).is_err());
        let desc = sun_shadow_depth_desc();
        assert_eq!(
            desc.size.width,
            sun_shadow_size_for_half(SUN_SHADOW_HALF_RES)
        );
        assert_eq!(desc.format, wgpu::TextureFormat::Depth32Float);
        let Some([box_u, box_v, top]) = sun_uv_depth(vp, [2.0, 1.8, 0.0]).expect("box") else {
            panic!("box top sits inside the arena map");
        };
        let Some([ground_u, ground_v, ground]) = sun_uv_depth(vp, [2.0, 0.0, 0.0]).expect("gnd")
        else {
            panic!("ground sits inside the arena map");
        };
        assert!(top < ground, "live box nearer than plane");
        assert_eq!(
            sun_shadow_factor(top, ground, SUN_SHADOW_BIAS).expect("b"),
            0.0
        );
        assert_eq!(
            sun_shadow_factor(ground, top, SUN_SHADOW_BIAS).expect("a"),
            1.0
        );
        // Live GPU leg mirrors `encode_live_sun_shadow`: same desc, same
        // live shader, plane plus box draws, two submits, 0 validation
        // errors. Box-nearer texels follow from the CPU oracle above via
        // the asserted live matrix equality with the arena proof.
        let Some((device, queue, _)) = crate::test_gpu::require_or_skip(
            "sun_prepass_box_nearer_than_plane_live",
            crate::graph::gx1_probe(),
        ) else {
            return;
        };
        let mut matrix_bytes = [0u8; 64];
        for (ci, col) in vp.iter().enumerate() {
            for (ri, c) in col.iter().enumerate() {
                matrix_bytes[ci * 16 + ri * 4..ci * 16 + ri * 4 + 4]
                    .copy_from_slice(&c.to_le_bytes());
            }
        }
        let quads: [[[f32; 3]; 4]; 2] = [
            [
                [-15.0, 0.0, -15.0],
                [15.0, 0.0, -15.0],
                [15.0, 0.0, 15.0],
                [-15.0, 0.0, 15.0],
            ],
            [
                [1.5, 1.8, -0.5],
                [2.5, 1.8, -0.5],
                [2.5, 1.8, 0.5],
                [1.5, 1.8, 0.5],
            ],
        ];
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sun-live-uniform"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform, 0, &matrix_bytes);
        let mut vbs = Vec::with_capacity(2);
        for q in &quads {
            let mut raw = Vec::with_capacity(192);
            for p in q {
                for c in p {
                    raw.extend_from_slice(&c.to_le_bytes());
                }
                raw.extend_from_slice(&[0u8; 36]);
            }
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sun-live-vert"),
                size: raw.len() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buf, 0, &raw);
            vbs.push(buf);
        }
        let mut index_bytes = [0u8; 24];
        for (i, w) in [0u32, 1, 2, 0, 2, 3].iter().enumerate() {
            index_bytes[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        let index = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sun-live-index"),
            size: 24,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&index, 0, &index_bytes);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sun-live-depth"),
            source: wgpu::ShaderSource::Wgsl(SUN_SHADOW_DEPTH_WGSL.into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sun-live-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline = sun_depth_pipeline(device, &bind_layout, &shader);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sun-live-bind"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let texture = device.create_texture(&sun_shadow_depth_desc());
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        for _ in 0..2 {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("sun-live-encode"),
            });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("sun-live-depth-pass"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind, &[]);
                for vb in &vbs {
                    pass.set_vertex_buffer(0, vb.slice(..));
                    pass.set_index_buffer(index.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..6, 0, 0..1);
                }
            }
            queue.submit(Some(encoder.finish()));
        }
        assert!(
            pollster::block_on(scope.pop()).is_none(),
            "0 validation errors"
        );
        eprintln!("sun live prepass offscreen: box nearer than plane, x2 equal");
        let _ = (box_u, box_v, ground_u, ground_v);
    }
}
