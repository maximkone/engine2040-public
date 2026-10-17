//! gtao_live: GTAO on the forward-stored depth, on the device, in the live frame (LF3).
//!
//! Owner: render lane, consumer `present_frame_live` (one compute pass after
//! the forward and sky passes, before the blit). Units: pixels for sizes,
//! meters for the kernel, occlusion `0..=1` with 1.0 open. The pass is two
//! dispatches inside one compute pass: the half-res kernel reads the depth
//! texture directly (a world point per tap, a facing normal per center) and
//! writes a half-res buffer, then the bilateral upsample writes the full-res
//! `ao_tex` the blit multiplies in. The kernel is the plane-aware one of
//! [`crate::gtao::gtao_live_oracle`]; that CPU oracle and these shaders share
//! one set of constants, and a device test pins their parity.
//! Determinism: fixed tap order, f32 on the device; presentation only, never
//! hashed. Budget: `GTAO_BUDGET_MS`; transient storage is the half-res AO
//! buffer alone (a quarter of one frame of f32), no depth or normal copy.
//! Errors: a frame the kernel cannot size (zero or over `MAX_LOOK_DIM`) gets
//! no pass and `ao_tex` stays 1.0.
use super::super::*;
use super::frame::Targets;
use super::submit::{inverse_view_proj_cols, FrameCamera};
use super::Presenter;
use crate::decals::{compute_storage_entry, compute_uniform_entry};
use crate::gtao::{gtao_weight_sum_f32, GTAO_LIVE_SIN_BIAS};

/// Bytes of the kernel uniform (`LiveAoU` in both shaders).
const LIVE_AO_UNIFORM_BYTES: u64 = 112;

/// Workgroup edge of both dispatches, in pixels.
const GROUP_EDGE: u32 = 8;

/// Uniform, depth texture and the world-point reads both shaders share.
const GTAO_LIVE_COMMON_WGSL: &str = r#"
struct LiveAoU {
    inv_vp: mat4x4<f32>,
    eye: vec3<f32>,
    falloff: f32,
    full_wh: vec2<u32>,
    half_wh: vec2<u32>,
    step: u32,
    intensity: f32,
    weight_sum: f32,
    sin_bias: f32,
};
@group(0) @binding(0) var<uniform> u: LiveAoU;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
fn clamp_px(p: vec2<i32>) -> vec2<i32> {
    return clamp(p, vec2<i32>(0), vec2<i32>(u.full_wh) - vec2<i32>(1));
}
fn depth_at(p: vec2<i32>) -> f32 {
    return textureLoad(depth_tex, clamp_px(p), 0);
}
fn world_at(p: vec2<i32>) -> vec3<f32> {
    let c = clamp_px(p);
    let d = textureLoad(depth_tex, c, 0);
    let nx = 2.0 * (f32(c.x) + 0.5) / f32(u.full_wh.x) - 1.0;
    let ny = 1.0 - 2.0 * (f32(c.y) + 0.5) / f32(u.full_wh.y);
    let q = u.inv_vp * vec4<f32>(nx, ny, d, 1.0);
    return q.xyz / q.w;
}
"#;

/// Half-res plane-aware kernel: one thread per half texel. Mirrors
/// `gtao_live_oracle` step for step (same tap order, same bias and fade).
const GTAO_LIVE_HALF_WGSL: &str = r#"
@group(0) @binding(2) var<storage, read_write> ao_half: array<f32>;
fn facing_normal(c: vec2<i32>, here: vec3<f32>) -> vec3<f32> {
    let tx = world_at(c + vec2<i32>(1, 0)) - world_at(c + vec2<i32>(-1, 0));
    let ty = world_at(c + vec2<i32>(0, 1)) - world_at(c + vec2<i32>(0, -1));
    var n = cross(tx, ty);
    let to_eye = u.eye - here;
    if (dot(n, to_eye) < 0.0) {
        n = -n;
    }
    let l2 = dot(n, n);
    if (l2 < 1e-24) {
        return to_eye / max(length(to_eye), 1e-12);
    }
    return n / sqrt(l2);
}
@compute @workgroup_size(8, 8)
fn half_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= u.half_wh.x || gid.y >= u.half_wh.y) {
        return;
    }
    let cx = min(gid.x * 2u, u.full_wh.x - 1u);
    let cy = min(gid.y * 2u, u.full_wh.y - 1u);
    let c = vec2<i32>(i32(cx), i32(cy));
    let slot = gid.y * u.half_wh.x + gid.x;
    if (depth_at(c) >= 1.0) {
        ao_half[slot] = 1.0;
        return;
    }
    let pc = world_at(c);
    let nc = facing_normal(c, pc);
    var occ = 0.0;
    for (var s = 0u; s < 4u; s = s + 1u) {
        var dir = vec2<i32>(1, 0);
        if (s == 1u) {
            dir = vec2<i32>(0, 1);
        } else if (s == 2u) {
            dir = vec2<i32>(1, 1);
        } else if (s == 3u) {
            dir = vec2<i32>(1, -1);
        }
        for (var t = 1u; t <= 3u; t = t + 1u) {
            let off = i32(t * u.step);
            let w_tap = 1.0 / (1.0 + f32(t));
            for (var side = 0; side < 2; side = side + 1) {
                let sgn = select(-1, 1, side == 0);
                let p = clamp_px(c + dir * off * sgn);
                if (depth_at(p) >= 1.0) {
                    continue;
                }
                let v = world_at(p) - pc;
                let len = length(v);
                if (len < 1e-5) {
                    continue;
                }
                let sine = dot(v, nc) / len;
                let elevation = clamp((sine - u.sin_bias) / (1.0 - u.sin_bias), 0.0, 1.0);
                let fade = clamp(1.0 - len / u.falloff, 0.0, 1.0);
                occ = occ + elevation * fade * w_tap;
            }
        }
    }
    ao_half[slot] = clamp(1.0 - u.intensity * occ / u.weight_sum, 0.0, 1.0);
}
"#;

/// Bilateral upsample: one thread per full pixel, the 2x2 half neighbors
/// weighted by position and by eye-distance similarity, written straight into
/// the full-res `ao_tex`; sky pixels write 1.0. Mirrors `upsample_bilateral`.
const GTAO_LIVE_UP_WGSL: &str = r#"
@group(0) @binding(2) var<storage, read> ao_half: array<f32>;
@group(0) @binding(3) var ao_out: texture_storage_2d<r32float, write>;
fn dist_at(p: vec2<i32>) -> f32 {
    return length(world_at(p) - u.eye);
}
@compute @workgroup_size(8, 8)
fn up_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    let fw = u.full_wh.x;
    let fh = u.full_wh.y;
    if (x >= fw || y >= fh) {
        return;
    }
    let xy = vec2<i32>(i32(x), i32(y));
    if (depth_at(xy) >= 1.0) {
        textureStore(ao_out, xy, vec4<f32>(1.0, 0.0, 0.0, 0.0));
        return;
    }
    let hw = u.half_wh.x;
    let hh = u.half_wh.y;
    let hx = x / 2u;
    let hy = y / 2u;
    let fx = f32(x) / 2.0 - f32(hx);
    let fy = f32(y) / 2.0 - f32(hy);
    let hx1 = min(hx + 1u, hw - 1u);
    let hy1 = min(hy + 1u, hh - 1u);
    let d = dist_at(xy);
    let ax0 = min(hx * 2u, fw - 1u);
    let ax1 = min(hx1 * 2u, fw - 1u);
    let ay0 = min(hy * 2u, fh - 1u);
    let ay1 = min(hy1 * 2u, fh - 1u);
    let w00 = (1.0 - fx) * (1.0 - fy);
    let w10 = fx * (1.0 - fy);
    let w01 = (1.0 - fx) * fy;
    let w11 = fx * fy;
    let r00 = 1.0 / (1.0 + abs(d - dist_at(vec2<i32>(i32(ax0), i32(ay0)))) / u.falloff);
    let r10 = 1.0 / (1.0 + abs(d - dist_at(vec2<i32>(i32(ax1), i32(ay0)))) / u.falloff);
    let r01 = 1.0 / (1.0 + abs(d - dist_at(vec2<i32>(i32(ax0), i32(ay1)))) / u.falloff);
    let r11 = 1.0 / (1.0 + abs(d - dist_at(vec2<i32>(i32(ax1), i32(ay1)))) / u.falloff);
    let acc = ao_half[hy * hw + hx] * w00 * r00
        + ao_half[hy * hw + hx1] * w10 * r10
        + ao_half[hy1 * hw + hx] * w01 * r01
        + ao_half[hy1 * hw + hx1] * w11 * r11;
    let wsum = w00 * r00 + w10 * r10 + w01 * r01 + w11 * r11;
    let ao = select(1.0, acc / wsum, wsum > 0.0);
    textureStore(ao_out, xy, vec4<f32>(ao, 0.0, 0.0, 0.0));
}
"#;

/// The device half of live GTAO: both compute pipelines and their layouts.
/// Built once with the presenter; the frame-size resources live in
/// [`GtaoSized`] beside the targets they read and write.
pub(crate) struct GtaoGpu {
    half: wgpu::ComputePipeline,
    up: wgpu::ComputePipeline,
    half_layout: wgpu::BindGroupLayout,
    up_layout: wgpu::BindGroupLayout,
    /// The `ao_tex` holds live GTAO output, not the neutral 1.0: it must be
    /// reset once before a frame without the pass can trust it.
    pub(crate) wrote_ao: bool,
}

/// Frame-size resources of live GTAO: the kernel uniform, the half-res
/// buffer behind both bind groups, built with the targets they bind.
pub(crate) struct GtaoSized {
    uniform: wgpu::Buffer,
    half_bind: wgpu::BindGroup,
    up_bind: wgpu::BindGroup,
    /// Uniform image with every field but the camera filled in.
    template: [u8; 112],
    full: (u32, u32),
    half: (u32, u32),
}

/// One bind group entry.
fn bind_entry<'a>(binding: u32, resource: wgpu::BindingResource<'a>) -> wgpu::BindGroupEntry<'a> {
    wgpu::BindGroupEntry { binding, resource }
}

/// The depth texture entry both layouts share (binding 1).
fn depth_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 1,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// Pack the kernel uniform: the inverse view-projection (column-major), the
/// eye and falloff, both frame sizes, the tap step, strength, normalizer and
/// elevation bias. Pure bytes; the camera part is rewritten every frame.
fn live_ao_uniform_bytes(
    inv_cols: [[f32; 4]; 4],
    eye: [f32; 3],
    full: (u32, u32),
    half: (u32, u32),
    config: &crate::gtao::GtaoConfig,
) -> [u8; 112] {
    let mut out = [0u8; 112];
    let mut put = |at: usize, word: [u8; 4]| out[at..at + 4].copy_from_slice(&word);
    for (c, col) in inv_cols.iter().enumerate() {
        for (r, v) in col.iter().enumerate() {
            put((c * 4 + r) * 4, v.to_le_bytes());
        }
    }
    for (i, v) in eye.iter().enumerate() {
        put(64 + i * 4, v.to_le_bytes());
    }
    put(76, config.falloff_m.to_le_bytes());
    put(80, full.0.to_le_bytes());
    put(84, full.1.to_le_bytes());
    put(88, half.0.to_le_bytes());
    put(92, half.1.to_le_bytes());
    put(
        96,
        u32::try_from(config.tap_step_px())
            .unwrap_or(1)
            .to_le_bytes(),
    );
    put(100, config.intensity.to_le_bytes());
    put(104, gtao_weight_sum_f32().to_le_bytes());
    put(108, GTAO_LIVE_SIN_BIAS.to_le_bytes());
    out
}

impl GtaoGpu {
    /// Compile both kernels and their layouts. Cold path, once per presenter.
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        use crate::pipeline_warmup::CountedPipelines as _;
        let module = |label: &'static str, body: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(format!("{GTAO_LIVE_COMMON_WGSL}{body}").into()),
            })
        };
        let half_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gtao-live-half-layout"),
            entries: &[
                compute_uniform_entry(0),
                depth_entry(),
                compute_storage_entry(2, false),
            ],
        });
        let up_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gtao-live-up-layout"),
            entries: &[
                compute_uniform_entry(0),
                depth_entry(),
                compute_storage_entry(2, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::R32Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let pipeline = |label: &'static str,
                        layout: &wgpu::BindGroupLayout,
                        module: &wgpu::ShaderModule,
                        entry: &'static str| {
            let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline_counted(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pipe_layout),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let half_module = module("gtao-live-half", GTAO_LIVE_HALF_WGSL);
        let up_module = module("gtao-live-up", GTAO_LIVE_UP_WGSL);
        Self {
            half: pipeline("gtao-live-half", &half_layout, &half_module, "half_main"),
            up: pipeline("gtao-live-up", &up_layout, &up_module, "up_main"),
            half_layout,
            up_layout,
            wrote_ao: false,
        }
    }

    /// Size the frame resources over this frame's depth and `ao_tex` views:
    /// the half-res buffer, the uniform and both bind groups. `None` when the
    /// kernel cannot size the frame (zero or over the look bound), so such a
    /// frame runs no pass. Cold path: runs with the targets, never per frame.
    pub(crate) fn sized(
        &self,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
        ao_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> Option<GtaoSized> {
        let config = Presenter::gtao_config_for_frame(width, height).ok()?;
        let half = (width.div_ceil(2), height.div_ceil(2));
        let half_bytes = u64::from(half.0) * u64::from(half.1) * 4;
        let ao_half = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gtao-live-half-ao"),
            size: half_bytes,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gtao-live-uniform"),
            size: LIVE_AO_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let half_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gtao-live-half-bind"),
            layout: &self.half_layout,
            entries: &[
                bind_entry(0, uniform.as_entire_binding()),
                bind_entry(1, wgpu::BindingResource::TextureView(depth_view)),
                bind_entry(2, ao_half.as_entire_binding()),
            ],
        });
        let up_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gtao-live-up-bind"),
            layout: &self.up_layout,
            entries: &[
                bind_entry(0, uniform.as_entire_binding()),
                bind_entry(1, wgpu::BindingResource::TextureView(depth_view)),
                bind_entry(2, ao_half.as_entire_binding()),
                bind_entry(3, wgpu::BindingResource::TextureView(ao_view)),
            ],
        });
        let template =
            live_ao_uniform_bytes([[0.0; 4]; 4], [0.0; 3], (width, height), half, &config);
        Some(GtaoSized {
            uniform,
            half_bind,
            up_bind,
            template,
            full: (width, height),
            half,
        })
    }
}

impl Presenter {
    /// Decide whether this live frame runs GTAO and keep `ao_tex` honest: the
    /// pass runs when the row is on, the targets carry its resources and the
    /// forward pass is not multisampled (the 4x depth is not resolved);
    /// otherwise a texture the pass wrote earlier is reset to neutral once.
    /// Caller: `present_frame_live`, before the forward encode (the forward
    /// pass must store depth when this returns true).
    pub(super) fn gtao_live_plan(&mut self, row_on: bool, width: u32, height: u32) -> bool {
        let runs = row_on
            && self.targets.as_ref().is_some_and(|targets| {
                let (_, resolves) =
                    super::msaa::msaa_forward_choice(&self.msaa, targets.msaa.is_some());
                targets.gtao.is_some() && !resolves
            });
        if !runs && self.gtao_gpu.wrote_ao {
            let _rows = self.clear_fog_targets(width, height);
            self.gtao_gpu.wrote_ao = false;
        }
        self.gtao_gpu.wrote_ao |= runs;
        runs
    }

    /// Record the GTAO compute pass over the stored forward depth into
    /// `ao_tex`: the camera uniform first (a queue write ordered before the
    /// submit), then the half-res kernel and the upsample in one pass bracketed
    /// by `timestamps`. Returns false when the targets carry no resources.
    /// Caller: `present_frame_live`, after the forward and sky passes.
    pub(super) fn encode_gtao(
        &self,
        enc: &mut wgpu::CommandEncoder,
        targets: &Targets,
        camera: &FrameCamera,
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) -> Result<bool, PresentError> {
        let Some(sized) = targets.gtao.as_ref() else {
            return Ok(false);
        };
        let inv = inverse_view_proj_cols(camera)?;
        let mut bytes = sized.template;
        for (c, col) in inv.iter().enumerate() {
            for (r, v) in col.iter().enumerate() {
                let at = (c * 4 + r) * 4;
                bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        for (i, v) in camera.eye.to_array().iter().enumerate() {
            bytes[64 + i * 4..68 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        self.queue.write_buffer(&sized.uniform, 0, &bytes);
        let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ra1-gtao"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(&self.gtao_gpu.half);
        pass.set_bind_group(0, &sized.half_bind, &[]);
        pass.dispatch_workgroups(
            sized.half.0.div_ceil(GROUP_EDGE),
            sized.half.1.div_ceil(GROUP_EDGE),
            1,
        );
        pass.set_pipeline(&self.gtao_gpu.up);
        pass.set_bind_group(0, &sized.up_bind, &[]);
        pass.dispatch_workgroups(
            sized.full.0.div_ceil(GROUP_EDGE),
            sized.full.1.div_ceil(GROUP_EDGE),
            1,
        );
        Ok(true)
    }

    /// The `gtao` cell of a live frame's per-pass row: the device time of the
    /// newest resolved GTAO pass (a few frames late, never waited for). 0
    /// without timestamp queries or before the first sample lands.
    pub(super) fn live_gtao_row_us(&self) -> u64 {
        let ms = self
            .gpu_timer
            .as_ref()
            .map_or(0.0, |timer| f64::from(timer.latest_gtao_ms()));
        Self::ms_to_us(ms)
    }
}

#[cfg(test)]
mod tests {
    use super::super::submit::{effective_camera, map_readback};
    use super::*;
    use crate::pass_truth::harness::fixtures::{arena_like_scene, init_or_skip, H, W};
    use crate::pass_truth::LookToggles;

    /// Read one whole `f32` texture (a depth texture through its depth
    /// aspect, or `ao_texture`) back to the host, row-major.
    fn read_f32(
        presenter: &Presenter,
        texture: &wgpu::Texture,
        aspect: wgpu::TextureAspect,
    ) -> Vec<f32> {
        let (w, h) = (W, H);
        let row = w * 4;
        let padded = row.div_ceil(256) * 256;
        let buffer = presenter.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test-readback"),
            size: u64::from(padded) * u64::from(h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = presenter
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        presenter.queue.submit(Some(enc.finish()));
        let mut waits = FrameWaits::default();
        let bytes = map_readback(&presenter.device, &mut waits, &buffer, row, padded, h)
            .expect("readback maps");
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    #[test]
    fn the_uniform_image_carries_the_kernel_where_the_shaders_read_it() {
        // Caller: `GtaoGpu::sized` and `encode_gtao`. Behavior: every field
        // of `LiveAoU` sits at the offset the WGSL struct reads (matrix at 0,
        // eye at 64, falloff in the eye pad word, sizes at 80 and 88, the tap
        // step, strength, normalizer and bias after them).
        let config = Presenter::gtao_config_for_frame(320, 240).expect("config");
        let mut cols = [[0.0f32; 4]; 4];
        for (c, col) in cols.iter_mut().enumerate() {
            for (r, v) in col.iter_mut().enumerate() {
                *v = f32::from(u8::try_from(c * 4 + r).unwrap_or(0)) + 0.5;
            }
        }
        let bytes = live_ao_uniform_bytes(cols, [1.0, 2.0, 3.0], (320, 240), (160, 120), &config);
        let f = |at: usize| {
            f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let u = |at: usize| {
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        assert_eq!(f(0), 0.5, "column 0 row 0");
        assert_eq!(f(4 * 5), 5.5, "column 1 row 1");
        assert_eq!(f(60), 15.5, "column 3 row 3");
        assert_eq!([f(64), f(68), f(72)], [1.0, 2.0, 3.0], "eye");
        assert_eq!(f(76), config.falloff_m, "falloff rides the eye pad");
        assert_eq!((u(80), u(84)), (320, 240), "full size");
        assert_eq!((u(88), u(92)), (160, 120), "half size");
        assert_eq!(u(96), 3, "tap step of the 8 px arena kernel");
        assert_eq!(f(100), config.intensity);
        assert!((f(104) - 8.6667).abs() < 1e-3, "normalizer {}", f(104));
        assert_eq!(f(108), GTAO_LIVE_SIN_BIAS);
        assert_eq!(bytes.len(), 112, "LiveAoU is 112 bytes");
    }

    #[test]
    fn the_live_pass_matches_the_cpu_oracle_on_the_stored_depth() {
        // Caller: the LF3 accept ("GTAO on the stored forward depth"). Behavior:
        // the device AO of one live frame equals `gtao_live_oracle` run over
        // the very depth that frame stored: sky texels read exactly 1.0, and
        // the rest agree within float tolerance (f32 device, f64 oracle).
        let Some(mut presenter) = init_or_skip("the_live_pass_matches_the_cpu_oracle") else {
            return;
        };
        presenter.resize(W, H).expect("valid size applies");
        let list = arena_like_scene(&mut presenter, W, H);
        presenter.render_live_offscreen(&list).expect("live frame");
        let targets = presenter.targets.as_ref().expect("targets exist");
        let depth = read_f32(
            &presenter,
            &targets.depth_texture,
            wgpu::TextureAspect::DepthOnly,
        );
        let got = read_f32(&presenter, &targets.ao_texture, wgpu::TextureAspect::All);
        let frame = effective_camera(&list, W, H).expect("camera builds");
        let inv = inverse_view_proj_cols(&frame).expect("inverse");
        let config = Presenter::gtao_config_for_frame(W, H).expect("config");
        let want = crate::gtao::gtao_live_oracle(&depth, inv, frame.eye.to_array(), &config)
            .expect("oracle runs");
        assert_eq!(got.len(), want.len());
        let (mut worst, mut sum, mut off_count, mut dark, mut covered) =
            (0.0f32, 0.0f32, 0usize, 0usize, 0usize);
        for ((g, w), d) in got.iter().zip(want.iter()).zip(depth.iter()) {
            if *d >= 1.0 {
                assert_eq!(*g, 1.0, "sky reads open on the device");
                continue;
            }
            let gap = (g - w).abs();
            covered += 1;
            worst = worst.max(gap);
            sum += gap;
            off_count += usize::from(gap > 0.02);
            dark += usize::from(*w < 0.99);
        }
        let mean = sum / f32::from(u16::try_from(covered).unwrap_or(u16::MAX)).max(1.0);
        eprintln!(
            "gtao device vs oracle: worst {worst:.4}, mean {mean:.6}, {off_count} texels over 0.02 of {covered} ({dark} darkened)"
        );
        assert!(
            dark > 500,
            "the frame has real occlusion to compare: {dark}"
        );
        assert!(mean < 0.001, "mean gap {mean}");
        assert!(
            off_count * 200 < covered,
            "{off_count} texels differ by more than 0.02"
        );
        assert!(worst < 0.25, "worst gap {worst}");
    }

    #[test]
    fn ao_tex_is_neutral_when_the_pass_is_off_and_resets_after_it_ran() {
        // Caller: `gtao_live_plan`. Behavior: a frame with the pass on writes
        // occlusion below 1.0; the next frame with the row off leaves `ao_tex`
        // exactly 1.0 everywhere (the reset runs once).
        let Some(mut presenter) = init_or_skip("ao_tex_is_neutral_when_the_pass_is_off") else {
            return;
        };
        presenter.resize(W, H).expect("valid size applies");
        let list = arena_like_scene(&mut presenter, W, H);
        let mut off = LookToggles::all_on();
        off.disable("gtao");
        presenter.render_live_offscreen(&list).expect("pass on");
        let lit = read_f32(
            &presenter,
            &presenter.targets.as_ref().expect("targets").ao_texture,
            wgpu::TextureAspect::All,
        );
        assert!(lit.iter().any(|v| *v < 0.9), "the pass writes occlusion");
        assert!(presenter.gtao_gpu.wrote_ao, "the texture is marked dirty");
        presenter
            .render_live_offscreen_with_toggles(&list, &off)
            .expect("pass off");
        let neutral = read_f32(
            &presenter,
            &presenter.targets.as_ref().expect("targets").ao_texture,
            wgpu::TextureAspect::All,
        );
        assert!(neutral.iter().all(|v| *v == 1.0), "reset to open");
        assert!(!presenter.gtao_gpu.wrote_ao, "the reset ran once");
    }

    #[test]
    fn a_multisampled_frame_runs_no_gtao_and_keeps_ao_open() {
        // Caller: `gtao_live_plan`. Behavior: the 4x depth is not resolved, so
        // the pass is skipped on a multisampled frame: `ao_tex` stays 1.0 and
        // the frame stays validation-clean. An adapter without 4x skips.
        let Some(mut presenter) = init_or_skip("a_multisampled_frame_runs_no_gtao") else {
            return;
        };
        presenter.set_msaa_enabled(true);
        if !presenter.msaa_enabled() {
            crate::test_gpu::note_skip("msaa_gtao", "adapter cannot build the 4x pair");
            return;
        }
        presenter.resize(W, H).expect("valid size applies");
        let list = arena_like_scene(&mut presenter, W, H);
        let receipt = presenter.render_live_offscreen(&list).expect("4x frame");
        assert!(receipt.is_clean(), "validation-clean");
        let ao = read_f32(
            &presenter,
            &presenter.targets.as_ref().expect("targets").ao_texture,
            wgpu::TextureAspect::All,
        );
        assert!(ao.iter().all(|v| *v == 1.0), "no pass, no occlusion");
        assert!(!presenter.gtao_gpu.wrote_ao);
    }
}
