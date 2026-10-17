
use super::AaScaleError;

/// SMAA compute shader: luma edge detect over the 4-neighborhood,
/// threshold ramp weight, blend toward the cross mean. Mirrors
/// [`super::smaa_resolve`] texel for texel.
pub const SMAA_COMPUTE_WGSL: &str = r#"
struct SmaaU {
    dims: vec4<u32>,
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: SmaaU;
@group(0) @binding(1) var<storage, read> src: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> dst: array<vec4<f32>>;
fn luma(p: vec3<f32>) -> f32 {
    return dot(p, vec3<f32>(0.2126, 0.7152, 0.0722));
}
fn src_at(x: i32, y: i32) -> vec3<f32> {
    let w = i32(u.dims.x);
    let h = i32(u.dims.y);
    let cx = u32(clamp(x, 0, w - 1));
    let cy = u32(clamp(y, 0, h - 1));
    return src[cy * u.dims.x + cx].rgb;
}
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = u.dims.x;
    let h = u.dims.y;
    if (gid.x >= w || gid.y >= h) {
        return;
    }
    let idx = gid.y * w + gid.x;
    let c = src[idx].rgb;
    let t0 = src_at(i32(gid.x) - 1, i32(gid.y));
    let t1 = src_at(i32(gid.x) + 1, i32(gid.y));
    let t2 = src_at(i32(gid.x), i32(gid.y) - 1);
    let t3 = src_at(i32(gid.x), i32(gid.y) + 1);
    let lc = luma(c);
    var lo = lc;
    var hi = lc;
    lo = min(lo, luma(t0));
    hi = max(hi, luma(t0));
    lo = min(lo, luma(t1));
    hi = max(hi, luma(t1));
    lo = min(lo, luma(t2));
    hi = max(hi, luma(t2));
    lo = min(lo, luma(t3));
    hi = max(hi, luma(t3));
    let thr = u.params.x;
    let blend = u.params.y;
    let ramp = clamp((hi - lo - thr) / thr, 0.0, 1.0);
    let weight = clamp(blend * ramp, 0.0, 1.0);
    let mean = (t0 + t1 + t2 + t3) * 0.25;
    dst[idx] = vec4<f32>(mix(c, mean, weight), 1.0);
}
"#;

/// Posted CPU/GPU parity tolerance for the SMAA pass (pure `+ - * /`
/// plus clamp, so 1-ulp drift only).
pub const GPU_PARITY_TOL_SMAA: f32 = 3e-4;

/// One GPU SMAA resolve: pixels plus the validation outcome plus ms.
#[derive(Clone, Debug)]
pub struct GpuSmaa {
    /// Settled HDR pixels, row-major `width * height`.
    pub pixels: Vec<[f32; 3]>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// wgpu validation errors observed; green means 0.
    pub validation_errors: u32,
    /// Wall time for encode plus submit plus readback in ms.
    pub submit_ms: f64,
}

/// Reusable SMAA device state for the WIN1 TAA-off loop: one compute
/// pipeline plus its bind layout. Built once; per-submit buffers are
/// driver resources and sit outside any reuse guarantee.
pub struct SmaaSubmitter {
    pipe: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl SmaaSubmitter {
    /// Compile the SMAA compute pipeline plus its bind layout.
    /// No frame-size state: buffers size per submit.
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rp4-smaa-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rp4-smaa"),
            source: wgpu::ShaderSource::Wgsl(SMAA_COMPUTE_WGSL.into()),
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rp4-smaa"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("rp4-smaa"),
            layout: Some(&pipe_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { pipe, layout }
    }

    /// GPU SMAA fallback over one frame. One compute dispatch plus
    /// readback inside a validation scope; 0 scope hits means green.
    pub fn resolve(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hdr: &[[f32; 3]],
        width: u32,
        height: u32,
        params: &super::SmaaParams,
    ) -> Result<GpuSmaa, AaScaleError> {
        params.validate()?;
        super::check_image(hdr, width, height)?;
        let texels = width as usize * height as usize;
        let started = std::time::Instant::now();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut uni = Vec::with_capacity(32);
        uni.extend_from_slice(&width.to_le_bytes());
        uni.extend_from_slice(&height.to_le_bytes());
        uni.extend_from_slice(&0u32.to_le_bytes());
        uni.extend_from_slice(&0u32.to_le_bytes());
        uni.extend_from_slice(&params.edge_threshold.to_le_bytes());
        uni.extend_from_slice(&params.max_blend.to_le_bytes());
        uni.extend_from_slice(&0f32.to_le_bytes());
        uni.extend_from_slice(&0f32.to_le_bytes());
        use wgpu::util::DeviceExt;
        let (u_buf, src_buf, dst_buf) = (
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("rp4-smaa-u"),
                contents: &uni,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            }),
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("rp4-smaa-src"),
                contents: &pack_pixels(hdr),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            }),
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rp4-smaa-dst"),
                size: texels as u64 * 16,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rp4-smaa-bind"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: u_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: src_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: dst_buf.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rp4-smaa"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rp4-smaa-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipe);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
        queue.submit(Some(encoder.finish()));
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| AaScaleError::Device(format!("poll: {e}")))?;
        let scope_err = pollster::block_on(scope.pop());
        if let Some(err) = scope_err {
            return Err(AaScaleError::Validation(format!("{err:?}")));
        }
        let back = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rp4-smaa-readback"),
            size: texels as u64 * 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rp4-smaa-copy"),
        });
        encoder.copy_buffer_to_buffer(&dst_buf, 0, &back, 0, texels as u64 * 16);
        queue.submit(Some(encoder.finish()));
        let slice = back.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| AaScaleError::Device(format!("poll: {e}")))?;
        rx.recv()
            .map_err(|e| AaScaleError::Device(format!("map channel: {e}")))?
            .map_err(|e| AaScaleError::Device(format!("map: {e}")))?;
        let mut pixels = Vec::with_capacity(texels);
        {
            let mapped = slice
                .get_mapped_range()
                .map_err(|e| AaScaleError::Device(format!("mapped range: {e}")))?;
            for chunk in mapped.chunks_exact(16) {
                let x = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                let y = f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
                let z = f32::from_le_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]);
                if !x.is_finite() || !y.is_finite() || !z.is_finite() {
                    return Err(AaScaleError::Validation(
                        "gpu smaa pixels are not finite".into(),
                    ));
                }
                pixels.push([x, y, z]);
            }
        }
        back.unmap();
        Ok(GpuSmaa {
            pixels,
            width,
            height,
            validation_errors: 0,
            submit_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}

fn pack_pixels(pixels: &[[f32; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len() * 16);
    for p in pixels {
        out.extend_from_slice(&p[0].to_le_bytes());
        out.extend_from_slice(&p[1].to_le_bytes());
        out.extend_from_slice(&p[2].to_le_bytes());
        out.extend_from_slice(&1f32.to_le_bytes());
    }
    out
}

/// Max per-channel abs diff between two frames (CPU/GPU parity figure).
pub fn max_abs_diff(a: &[[f32; 3]], b: &[[f32; 3]]) -> f32 {
    let mut max = 0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        for c in 0..3 {
            max = max.max((x[c] - y[c]).abs());
        }
    }
    max
}

/// Predicated SMAA compute shader: luma edge detect over the
/// 4-neighborhood plus a depth-step ramp over the same taps; the
/// stronger weight wins and blends toward the cross mean. Mirrors
/// [`super::smaa_resolve_with_depth`] texel for texel.
pub const SMAA_PREDICATED_COMPUTE_WGSL: &str = r#"
struct SmaaPredU {
    dims: vec4<u32>,
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: SmaaPredU;
@group(0) @binding(1) var<storage, read> src: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> depth: array<f32>;
@group(0) @binding(3) var<storage, read_write> dst: array<vec4<f32>>;
fn luma(p: vec3<f32>) -> f32 {
    return dot(p, vec3<f32>(0.2126, 0.7152, 0.0722));
}
fn src_at(x: i32, y: i32) -> vec3<f32> {
    let w = i32(u.dims.x);
    let h = i32(u.dims.y);
    let cx = u32(clamp(x, 0, w - 1));
    let cy = u32(clamp(y, 0, h - 1));
    return src[cy * u.dims.x + cx].rgb;
}
fn depth_at(x: i32, y: i32) -> f32 {
    let w = i32(u.dims.x);
    let h = i32(u.dims.y);
    let cx = u32(clamp(x, 0, w - 1));
    let cy = u32(clamp(y, 0, h - 1));
    return depth[cy * u.dims.x + cx];
}
@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let w = u.dims.x;
    let h = u.dims.y;
    if (gid.x >= w || gid.y >= h) {
        return;
    }
    let idx = gid.y * w + gid.x;
    let c = src[idx].rgb;
    let dc = depth[idx];
    let t0 = src_at(i32(gid.x) - 1, i32(gid.y));
    let t1 = src_at(i32(gid.x) + 1, i32(gid.y));
    let t2 = src_at(i32(gid.x), i32(gid.y) - 1);
    let t3 = src_at(i32(gid.x), i32(gid.y) + 1);
    let d0 = depth_at(i32(gid.x) - 1, i32(gid.y));
    let d1 = depth_at(i32(gid.x) + 1, i32(gid.y));
    let d2 = depth_at(i32(gid.x), i32(gid.y) - 1);
    let d3 = depth_at(i32(gid.x), i32(gid.y) + 1);
    let lc = luma(c);
    var lo = lc;
    var hi = lc;
    lo = min(lo, luma(t0));
    hi = max(hi, luma(t0));
    lo = min(lo, luma(t1));
    hi = max(hi, luma(t1));
    lo = min(lo, luma(t2));
    hi = max(hi, luma(t2));
    lo = min(lo, luma(t3));
    hi = max(hi, luma(t3));
    var dlo = dc;
    var dhi = dc;
    dlo = min(dlo, d0);
    dhi = max(dhi, d0);
    dlo = min(dlo, d1);
    dhi = max(dhi, d1);
    dlo = min(dlo, d2);
    dhi = max(dhi, d2);
    dlo = min(dlo, d3);
    dhi = max(dhi, d3);
    let thr = u.params.x;
    let blend = u.params.y;
    let dthr = u.params.z;
    let l_weight = clamp(blend * clamp((hi - lo - thr) / thr, 0.0, 1.0), 0.0, 1.0);
    let d_weight = clamp(blend * clamp((dhi - dlo - dthr) / dthr, 0.0, 1.0), 0.0, 1.0);
    let weight = max(l_weight, d_weight);
    let mean = (t0 + t1 + t2 + t3) * 0.25;
    dst[idx] = vec4<f32>(mix(c, mean, weight), 1.0);
}
"#;

/// Posted CPU/GPU parity tolerance for the predicated pass (pure
/// `+ - * /` plus clamp and max, so 1-ulp drift only).
pub const GPU_PARITY_TOL_PREDICATED: f32 = 3e-4;

fn predicated_uniform_bytes(
    width: u32,
    height: u32,
    params: &super::SmaaPredicatedParams,
) -> Vec<u8> {
    let mut uni = Vec::with_capacity(32);
    uni.extend_from_slice(&width.to_le_bytes());
    uni.extend_from_slice(&height.to_le_bytes());
    uni.extend_from_slice(&0u32.to_le_bytes());
    uni.extend_from_slice(&0u32.to_le_bytes());
    uni.extend_from_slice(&params.base.edge_threshold.to_le_bytes());
    uni.extend_from_slice(&params.base.max_blend.to_le_bytes());
    uni.extend_from_slice(&params.depth_threshold_m.to_le_bytes());
    uni.extend_from_slice(&0f32.to_le_bytes());
    uni
}

fn copy_to_readback_and_parse(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    src: &wgpu::Buffer,
    texels: u64,
) -> Result<Vec<[f32; 3]>, AaScaleError> {
    let bytes = texels * 16;
    let back = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rp7-pred-readback"),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("rp7-pred-copy"),
    });
    encoder.copy_buffer_to_buffer(src, 0, &back, 0, bytes);
    queue.submit(Some(encoder.finish()));
    let slice = back.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| AaScaleError::Device(format!("poll: {e}")))?;
    rx.recv()
        .map_err(|e| AaScaleError::Device(format!("map channel: {e}")))?
        .map_err(|e| AaScaleError::Device(format!("map: {e}")))?;
    let count = usize::try_from(texels)
        .map_err(|_| AaScaleError::Invalid("predicated texel count does not fit".into()))?;
    let mut pixels = Vec::with_capacity(count);
    {
        let mapped = slice
            .get_mapped_range()
            .map_err(|e| AaScaleError::Device(format!("mapped range: {e}")))?;
        for chunk in mapped.chunks_exact(16) {
            let x = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            let y = f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
            let z = f32::from_le_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]);
            if !x.is_finite() || !y.is_finite() || !z.is_finite() {
                return Err(AaScaleError::Validation(
                    "gpu predicated pixels are not finite".into(),
                ));
            }
            pixels.push([x, y, z]);
        }
    }
    back.unmap();
    Ok(pixels)
}

/// Reusable predicated SMAA device state for the TAA-off loop: one
/// compute pipeline plus its bind layout. Built once; per-submit
/// buffers are driver resources and sit outside any reuse guarantee.
pub struct SmaaPredicatedSubmitter {
    pipe: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl SmaaPredicatedSubmitter {
    /// Compile the predicated compute pipeline plus its bind layout.
    /// No frame-size state: buffers size per submit.
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rp7-pred-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rp7-pred"),
            source: wgpu::ShaderSource::Wgsl(SMAA_PREDICATED_COMPUTE_WGSL.into()),
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rp7-pred"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("rp7-pred"),
            layout: Some(&pipe_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { pipe, layout }
    }

    /// GPU predicated SMAA over one frame plus its depth buffer. One
    /// compute dispatch plus readback inside a validation scope; 0
    /// scope hits means green. `size` is the `(width, height)` frame
    /// size in pixels, shared by the color and depth buffers.
    pub fn resolve_with_depth(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hdr: &[[f32; 3]],
        depth: &[f32],
        size: (u32, u32),
        params: &super::SmaaPredicatedParams,
    ) -> Result<GpuSmaa, AaScaleError> {
        let (width, height) = size;
        params.validate()?;
        super::check_image(hdr, width, height)?;
        super::check_depth(depth, width, height)?;
        let texels = u64::from(width) * u64::from(height);
        let started = std::time::Instant::now();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        use wgpu::util::DeviceExt;
        let (u_buf, src_buf, depth_buf, dst_buf) = (
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("rp7-pred-u"),
                contents: &predicated_uniform_bytes(width, height, params),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            }),
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("rp7-pred-src"),
                contents: &pack_pixels(hdr),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            }),
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("rp7-pred-depth"),
                contents: &crate::accel::f32_le_bytes(depth),
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
            }),
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rp7-pred-dst"),
                size: texels * 16,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rp7-pred-bind"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: u_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: src_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: depth_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: dst_buf.as_entire_binding(),
                },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rp7-pred"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rp7-pred-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipe);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
        }
        queue.submit(Some(encoder.finish()));
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| AaScaleError::Device(format!("poll: {e}")))?;
        let scope_err = pollster::block_on(scope.pop());
        if let Some(err) = scope_err {
            return Err(AaScaleError::Validation(format!("{err:?}")));
        }
        let pixels = copy_to_readback_and_parse(device, queue, &dst_buf, texels)?;
        Ok(GpuSmaa {
            pixels,
            width,
            height,
            validation_errors: 0,
            submit_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}
