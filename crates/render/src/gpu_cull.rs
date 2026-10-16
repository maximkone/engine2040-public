//! gpu_cull: GPU frustum cull plus its CPU oracle.
//!
//! Owner: render lane. Units: meters for centers and radii, counts for
//! instances. Determinism: the oracle uses `+ - *` in fixed plane order,
//! so the same bytes give the same flags twice. Errors are typed
//! [`CullError`] on the CPU path and [`CullSubmitError`] on the window
//! dispatch. Budget: the oracle is O(n) over caller-owned slices with no
//! allocation; the dispatch allocates its transient GPU buffers once per
//! call and reuses the compiled pipelines. Consumer: the frame-graph
//! executor binds this pass next; the window parity test compares the
//! device flags against [`cull_cpu`] today.

use core::fmt;

/// Workgroup width the shader is compiled for; the dispatch sizes from it.
pub const CULL_WORKGROUP: u32 = 64;

/// Compute shader source for the cull pass with the args write.
pub const CULL_WGSL: &str = include_str!("shaders/cull.wgsl");

/// One bounding sphere in meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sphere {
    /// Center in meters.
    pub center_m: [f32; 3],
    /// Radius in meters, finite and non-negative.
    pub radius_m: f32,
}

/// Cull input failures; every variant names what to fix.
#[derive(Clone, Debug, PartialEq)]
pub enum CullError {
    /// One sphere carries NaN or a negative radius.
    BadSphere {
        /// Sphere index in the caller's slice.
        index: usize,
        /// What failed validation.
        reason: String,
    },
    /// One plane carries NaN.
    BadPlane {
        /// Plane index, 0 through 5.
        index: usize,
        /// What failed validation.
        reason: String,
    },
    /// Hi-Z mip input missing; the caller must supply the depth pyramid.
    MissingHiz {
        /// What failed validation.
        reason: String,
    },
}

impl fmt::Display for CullError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSphere { index, reason } => {
                write!(f, "bad sphere {index}: {reason}")
            }
            Self::BadPlane { index, reason } => {
                write!(f, "bad plane {index}: {reason}")
            }
            Self::MissingHiz { reason } => {
                write!(f, "missing hiz: {reason}")
            }
        }
    }
}

impl std::error::Error for CullError {}

/// Re-exported 5-u32 indirect args: 36 indices plus the visible count.
pub use crate::draw::indirect_args_5;
/// Re-exported count over flag bytes: 1 visible, anything else culled.
pub use crate::draw::visible_count;

fn all_finite3(v: [f32; 3]) -> bool {
    v.iter().all(|x| x.is_finite())
}

fn validate_sphere(sphere: &Sphere, index: usize) -> Result<(), CullError> {
    if !all_finite3(sphere.center_m) {
        return Err(CullError::BadSphere {
            index,
            reason: "center_m is not finite".to_string(),
        });
    }
    if !sphere.radius_m.is_finite() {
        return Err(CullError::BadSphere {
            index,
            reason: "radius_m is not finite".to_string(),
        });
    }
    if sphere.radius_m < 0.0 {
        return Err(CullError::BadSphere {
            index,
            reason: "radius_m is negative".to_string(),
        });
    }
    Ok(())
}

fn validate_planes(planes: &[[f32; 4]; 6]) -> Result<(), CullError> {
    let mut i = 0;
    while i < 6 {
        let p = planes[i];
        let mut j = 0;
        while j < 4 {
            if !p[j].is_finite() {
                return Err(CullError::BadPlane {
                    index: i,
                    reason: "plane word is not finite".to_string(),
                });
            }
            j += 1;
        }
        i += 1;
    }
    Ok(())
}

/// True when the sphere survives all six planes with the `-radius` rule:
/// inside where `dot(n, center) + dist >= -radius`. Touching counts as
/// visible. Validation is loud; the math is `+ - *` in plane order.
/// The `index` names the caller slot in errors; the CPU fold [`cull_cpu`]
/// reads through here so indexed faults name the culprit sphere.
/// Units meters, no allocation, stable order.
pub fn sphere_visible_at(
    sphere: &Sphere,
    planes: &[[f32; 4]; 6],
    index: usize,
) -> Result<bool, CullError> {
    validate_sphere(sphere, index)?;
    validate_planes(planes)?;
    Ok(crate::draw::sphere_visible_cpu(
        sphere.center_m,
        sphere.radius_m,
        planes,
    ))
}

/// Flags for `spheres` in input order: 1 visible, 0 culled. Writes at most
/// `flags.len()` and returns the count written. No allocation; the caller
/// reuses its buffer across frames. Stable: the same bytes give the same
/// flags twice.
pub fn cull_cpu(
    spheres: &[Sphere],
    planes: &[[f32; 4]; 6],
    flags: &mut [u8],
) -> Result<usize, CullError> {
    validate_planes(planes)?;
    if flags.len() < spheres.len() {
        return Err(CullError::BadSphere {
            index: flags.len(),
            reason: "flags shorter than spheres".to_string(),
        });
    }
    let mut i = 0;
    while i < spheres.len() {
        let hit = sphere_visible_at(&spheres[i], planes, i)?;
        flags[i] = if hit { 1 } else { 0 };
        i += 1;
    }
    Ok(spheres.len())
}

/// Hi-Z wall sample for pass 2 (voidin second stage, reimplemented).
/// Caller: `cull_cpu_hiz` in this file.
/// PORT(pannapudi/voidin@36e84bb4e6c1bf4619df076cd2acdbeba1e63306:src/bin/model.rs)
pub struct HizPyramid {
    /// Wall depth in meters from the fixed camera.
    pub occluder_depth_m: f32,
    /// Wall half size in x/y in meters.
    pub half_extent_m: f32,
    /// Resident mip levels; 0 means missing and refuses loudly.
    pub levels: u32,
}

/// Fixed-size submit image in bytes: one 5-u32 args write, flat in n.
pub const SUBMIT_ARGS_BYTES: u64 = 20;

/// True when the sphere hides fully behind the Hi-Z wall level.
pub fn hiz_hidden(sphere: &Sphere, hiz: &HizPyramid) -> Result<bool, CullError> {
    if hiz.levels == 0 {
        return Err(CullError::MissingHiz {
            reason: "hiz mip input missing".to_string(),
        });
    }
    if !hiz.occluder_depth_m.is_finite() || !hiz.half_extent_m.is_finite() {
        return Err(CullError::MissingHiz {
            reason: "hiz sample is not finite".to_string(),
        });
    }
    validate_sphere(sphere, 0)?;
    let behind = -sphere.center_m[2] - sphere.radius_m > hiz.occluder_depth_m;
    let inside = sphere.center_m[0].abs() <= hiz.half_extent_m
        && sphere.center_m[1].abs() <= hiz.half_extent_m;
    Ok(behind && inside)
}

/// Pass 2 over pass-1 survivors only; 1 kept, 0 culled. No allocation.
pub fn cull_cpu_hiz(
    spheres: &[Sphere],
    pass1: &[u8],
    hiz: &HizPyramid,
    flags: &mut [u8],
) -> Result<usize, CullError> {
    if hiz.levels == 0 {
        return Err(CullError::MissingHiz {
            reason: "hiz mip input missing".to_string(),
        });
    }
    if pass1.len() < spheres.len() || flags.len() < spheres.len() {
        return Err(CullError::BadSphere {
            index: flags.len().min(pass1.len()),
            reason: "hiz flags shorter than spheres".to_string(),
        });
    }
    let mut i = 0;
    while i < spheres.len() {
        flags[i] = if pass1[i] != 1 {
            0
        } else if hiz_hidden(&spheres[i], hiz)? {
            0
        } else {
            1
        };
        i += 1;
    }
    Ok(spheres.len())
}

/// Next resident capacity for `need` instances (voidin ResizableBuffer
/// grow, reimplemented, zero bytes vendored). Doubling from 64, no
/// allocation, same need gives same capacity twice.
/// Caller: `ResidentCull::ensure_capacity` in this file.
/// PORT(pannapudi/voidin@36e84bb4e6c1bf4619df076cd2acdbeba1e63306:src/bin/model.rs)
/// Units counts for instances. Determinism: integer math only.
pub fn resident_grown_capacity(need_instances: usize) -> usize {
    let mut cap = 64;
    while cap < need_instances {
        cap = cap.saturating_mul(2);
        if cap == usize::MAX {
            break;
        }
    }
    cap
}

/// Resident cull storage: caller-owned scratch for the two-pass prepass
/// plus the dense visible-index list the single indirect draw consumes.
/// Grows by doubling (voidin moving-instances pattern, reimplemented) and
/// never shrinks, so steady-state frames allocate nothing once warm.
/// Caller: `cull_cpu_two_pass` in this file.
/// PORT(pannapudi/voidin@36e84bb4e6c1bf4619df076cd2acdbeba1e63306:src/bin/model.rs)
/// PORT(pannapudi/voidin@36e84bb4e6c1bf4619df076cd2acdbeba1e63306:src/bin/bvh_gpu.rs)
/// Units counts for instances; flags are 1 visible else 0. Determinism:
/// same bytes give same flags and indices twice. Errors: typed
/// [`CullError`] when an index exceeds `u32`.
pub struct ResidentCull {
    pass1: Vec<u8>,
    pass2: Vec<u8>,
    indices: Vec<u32>,
    cap: usize,
}

impl ResidentCull {
    /// Empty storage with no allocation.
    pub fn new() -> Self {
        Self {
            pass1: Vec::new(),
            pass2: Vec::new(),
            indices: Vec::new(),
            cap: 0,
        }
    }

    /// Instances the storage currently holds without reallocating.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Grow to hold `need` instances, doubling, never shrinking. No-op
    /// once warm; the caller reuses the same storage across frames.
    pub fn ensure_capacity(&mut self, need: usize) {
        if need <= self.cap {
            return;
        }
        let grown = resident_grown_capacity(need);
        if self.pass1.len() < grown {
            self.pass1.resize(grown, 0);
        }
        if self.pass2.len() < grown {
            self.pass2.resize(grown, 0);
        }
        if self.indices.len() < grown {
            self.indices.resize(grown, 0);
        }
        self.cap = grown;
    }

    /// Pass-2 flags in input order after [`cull_cpu_two_pass`] runs.
    pub fn pass2(&self) -> &[u8] {
        &self.pass2
    }

    /// Dense visible indices after [`cull_cpu_two_pass`] runs.
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }
}

impl Default for ResidentCull {
    fn default() -> Self {
        Self::new()
    }
}

/// Dense visible-index list over pass-2 flags, in input order. Writes at
/// most `indices_out.len()` and returns the visible count. No allocation;
/// the caller reuses its buffer across frames. Stable: same bytes give
/// the same list twice.
/// Caller: `cull_cpu_two_pass` in this file.
/// Units counts for instances. Determinism: integer order only.
pub fn compact_visible(pass2: &[u8], indices_out: &mut [u32]) -> Result<u32, CullError> {
    if indices_out.len() < pass2.len() {
        return Err(CullError::BadSphere {
            index: indices_out.len(),
            reason: "indices shorter than flags".to_string(),
        });
    }
    let mut count: u32 = 0;
    let mut i = 0;
    while i < pass2.len() {
        if pass2[i] == 1 {
            let slot = u32::try_from(i).map_err(|_| CullError::BadSphere {
                index: i,
                reason: "index exceeds u32".to_string(),
            })?;
            let at = usize::try_from(count).map_err(|_| CullError::BadSphere {
                index: i,
                reason: "count exceeds usize".to_string(),
            })?;
            indices_out[at] = slot;
            count = count.saturating_add(1);
        }
        i += 1;
    }
    Ok(count)
}

/// Two-pass visibility prepass (voidin Visibility-plus-compact, fused for
/// the CPU oracle): pass 1 frustum writes flags, pass 2 Hi-Z re-tests only
/// survivors, then compacts into the dense list plus one
/// [`indirect_args_5`] args buffer the draw consumes with a single
/// indirect dispatch. Caller-owned slices in and out, storage reused
/// across frames, submit image stays [`SUBMIT_ARGS_BYTES`] bytes.
/// Caller: `CullSubmitter::run_hiz_on_device` under `window` packs the
/// same `HizPyramid` for the `main_hiz` dispatch; consumer is
/// `crate::draw::DrawList::build`, which reads the args count for its one
/// instanced draw.
/// PORT(pannapudi/voidin@36e84bb4e6c1bf4619df076cd2acdbeba1e63306:src/bin/model.rs)
/// Units meters for spheres, counts for instances. Determinism: `+ - *`
/// in fixed plane order plus integer compact order. Errors: typed
/// [`CullError`], missing Hi-Z refuses loudly and never guesses.
pub fn cull_cpu_two_pass(
    spheres: &[Sphere],
    planes: &[[f32; 4]; 6],
    hiz: &HizPyramid,
    resident: &mut ResidentCull,
    args_out: &mut [u32; 5],
) -> Result<u32, CullError> {
    resident.ensure_capacity(spheres.len());
    let n = spheres.len();
    cull_cpu(spheres, planes, &mut resident.pass1[..n])?;
    cull_cpu_hiz(spheres, &resident.pass1[..n], hiz, &mut resident.pass2[..n])?;
    let kept = compact_visible(&resident.pass2[..n], &mut resident.indices[..n])?;
    let args = indirect_args_5(kept);
    args_out.copy_from_slice(&args);
    Ok(kept)
}

/// Window dispatch failures; every variant names what to fix.
#[cfg(feature = "window")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CullSubmitError {
    /// Caller input is unusable; reason carried.
    BadInput(String),
    /// A device or queue call failed; message carried.
    DeviceFault(String),
    /// A validation scope fired or a flag came back wrong; detail carried.
    FlagMismatch(String),
}

#[cfg(feature = "window")]
impl fmt::Display for CullSubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (kind, detail) = match self {
            Self::BadInput(detail) => ("refused", detail),
            Self::DeviceFault(detail) => ("device", detail),
            Self::FlagMismatch(detail) => ("flags", detail),
        };
        write!(f, "cull {kind} fault: {detail}")
    }
}

#[cfg(feature = "window")]
impl std::error::Error for CullSubmitError {}

/// Workgroups covering `n` spheres at the compiled width.
#[cfg(feature = "window")]
pub fn workgroups_for(n: usize) -> Result<u32, CullSubmitError> {
    let count = u32::try_from(n)
        .map_err(|_| CullSubmitError::BadInput("cull sphere count exceeds u32".to_string()))?;
    if count == 0 {
        return Err(CullSubmitError::BadInput(
            "cull submit holds no spheres".to_string(),
        ));
    }
    Ok(count.div_ceil(CULL_WORKGROUP))
}

/// Reusable device state for the cull pass: one bind layout plus a
/// pipeline per entry point. Built once per frame loop; steady-state
/// dispatches create no pipelines.
#[cfg(feature = "window")]
pub struct CullSubmitter {
    layout: wgpu::BindGroupLayout,
    pipe_cull: wgpu::ComputePipeline,
    pipe_args: wgpu::ComputePipeline,
    pipe_hiz: wgpu::ComputePipeline,
}

/// Buffers plus the bind group for one dispatch; built per call.
#[cfg(feature = "window")]
struct CullBind {
    flags_buf: wgpu::Buffer,
    args_buf: wgpu::Buffer,
    flags_read: wgpu::Buffer,
    args_read: wgpu::Buffer,
    indices_buf: wgpu::Buffer,
    indices_read: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

/// One GPU buffer for the cull dispatch.
#[cfg(feature = "window")]
fn cull_buffer(
    device: &wgpu::Device,
    label: &'static str,
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

#[cfg(feature = "window")]
impl CullSubmitter {
    /// Compile the cull plus args plus Hi-Z pipelines and the 7-binding layout.
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpu-cull"),
            source: wgpu::ShaderSource::Wgsl(CULL_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpu-cull-layout"),
            entries: &[
                crate::decals::compute_storage_entry(0, true),
                crate::decals::compute_storage_entry(1, true),
                crate::decals::compute_storage_entry(2, false),
                crate::decals::compute_storage_entry(3, false),
                crate::decals::compute_storage_entry(4, false),
                crate::decals::compute_storage_entry(5, true),
                crate::decals::compute_storage_entry(6, false),
            ],
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu-cull-pipe-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipe_cull = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gpu-cull-pipeline"),
            layout: Some(&pipe_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let pipe_args = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gpu-cull-args-pipeline"),
            layout: Some(&pipe_layout),
            module: &shader,
            entry_point: Some("write_args"),
            compilation_options: Default::default(),
            cache: None,
        });
        let pipe_hiz = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gpu-cull-hiz-pipeline"),
            layout: Some(&pipe_layout),
            module: &shader,
            entry_point: Some("main_hiz"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            layout,
            pipe_cull,
            pipe_args,
            pipe_hiz,
        }
    }

    /// Run the device cull over `spheres` and `planes`, writing `flags_out`
    /// (1 visible, 0 culled) plus the 5-u32 `args_out`, returning the
    /// visible count. Caller buffers are reused across frames.
    pub fn run_on_device(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        spheres: &[Sphere],
        planes: &[[f32; 4]; 6],
        flags_out: &mut [u32],
        args_out: &mut [u32; 5],
    ) -> Result<u32, CullSubmitError> {
        validate_planes(planes).map_err(|e| CullSubmitError::BadInput(e.to_string()))?;
        if spheres.is_empty() {
            return Err(CullSubmitError::BadInput(
                "cull submit holds no spheres".to_string(),
            ));
        }
        if flags_out.len() < spheres.len() {
            return Err(CullSubmitError::BadInput(
                "cull flags out shorter than spheres".to_string(),
            ));
        }
        for (i, s) in spheres.iter().enumerate() {
            validate_sphere(s, i).map_err(|e| CullSubmitError::BadInput(e.to_string()))?;
        }
        let n_u32 = u32::try_from(spheres.len())
            .map_err(|_| CullSubmitError::BadInput("cull sphere count exceeds u32".to_string()))?;
        let groups = n_u32.div_ceil(CULL_WORKGROUP);
        let mut sphere_bytes: Vec<u8> = Vec::new();
        for s in spheres {
            for v in [s.center_m[0], s.center_m[1], s.center_m[2], s.radius_m] {
                sphere_bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        let mut plane_bytes: Vec<u8> = Vec::new();
        for p in planes {
            for v in p {
                plane_bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        let sphere_len = u64::try_from(sphere_bytes.len())
            .map_err(|_| CullSubmitError::BadInput("cull sphere bytes exceed u64".to_string()))?;
        let flag_len = u64::try_from(spheres.len())
            .map_err(|_| CullSubmitError::BadInput("cull flag count exceeds u64".to_string()))?
            .saturating_mul(4);
        let bound = Self::bind_buffers(
            &self.layout,
            device,
            queue,
            &sphere_bytes,
            &plane_bytes,
            sphere_len,
            flag_len,
            &[0u8; 16],
        )?;
        self.dispatch_and_read(device, queue, &bound, flag_len, groups, flags_out, args_out)
    }

    /// Run the Hi-Z two-pass on device: `main_hiz` fuses frustum plus the
    /// wall reject, then compacts survivors into `indices_out` plus one
    /// `indirect_args_5` args buffer for a single indirect draw. Caller
    /// supplies the depth-pyramid sample as `hiz`; missing mips refuse
    /// loudly and never guess. Caller buffers are reused across frames.
    /// Caller: future VG11 indirect-draw wiring; consumer is
    /// `crate::draw::DrawList::build`, which reads the args count.
    /// Units meters for spheres, counts for instances. Errors: typed
    /// [`CullSubmitError`], missing Hi-Z is `BadInput`.
    #[cfg(feature = "window")]
    pub fn run_hiz_on_device(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        spheres: &[Sphere],
        planes: &[[f32; 4]; 6],
        hiz: &HizPyramid,
        flags_out: &mut [u32],
        indices_out: &mut [u32],
        args_out: &mut [u32; 5],
    ) -> Result<u32, CullSubmitError> {
        validate_planes(planes).map_err(|e| CullSubmitError::BadInput(e.to_string()))?;
        if hiz.levels == 0 {
            return Err(CullSubmitError::BadInput(
                "hiz mip input missing".to_string(),
            ));
        }
        if !hiz.occluder_depth_m.is_finite() || !hiz.half_extent_m.is_finite() {
            return Err(CullSubmitError::BadInput(
                "hiz sample is not finite".to_string(),
            ));
        }
        if spheres.is_empty() {
            return Err(CullSubmitError::BadInput(
                "cull submit holds no spheres".to_string(),
            ));
        }
        if flags_out.len() < spheres.len() || indices_out.len() < spheres.len() {
            return Err(CullSubmitError::BadInput(
                "cull hiz outs shorter than spheres".to_string(),
            ));
        }
        for (i, s) in spheres.iter().enumerate() {
            validate_sphere(s, i).map_err(|e| CullSubmitError::BadInput(e.to_string()))?;
        }
        let n_u32 = u32::try_from(spheres.len())
            .map_err(|_| CullSubmitError::BadInput("cull sphere count exceeds u32".to_string()))?;
        let groups = n_u32.div_ceil(CULL_WORKGROUP);
        let mut sphere_bytes: Vec<u8> = Vec::new();
        for s in spheres {
            for v in [s.center_m[0], s.center_m[1], s.center_m[2], s.radius_m] {
                sphere_bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        let mut plane_bytes: Vec<u8> = Vec::new();
        for p in planes {
            for v in p {
                plane_bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
        let sphere_len = u64::try_from(sphere_bytes.len())
            .map_err(|_| CullSubmitError::BadInput("cull sphere bytes exceed u64".to_string()))?;
        let flag_len = u64::try_from(spheres.len())
            .map_err(|_| CullSubmitError::BadInput("cull flag count exceeds u64".to_string()))?
            .saturating_mul(4);
        let mut hiz_bytes = [0u8; 16];
        hiz_bytes[0..4].copy_from_slice(&hiz.occluder_depth_m.to_le_bytes());
        hiz_bytes[4..8].copy_from_slice(&hiz.half_extent_m.to_le_bytes());
        hiz_bytes[8..12].copy_from_slice(&f32::from_bits(hiz.levels).to_le_bytes());
        let bound = Self::bind_buffers(
            &self.layout,
            device,
            queue,
            &sphere_bytes,
            &plane_bytes,
            sphere_len,
            flag_len,
            &hiz_bytes,
        )?;
        self.dispatch_hiz_and_read(
            device,
            queue,
            &bound,
            flag_len,
            groups,
            flags_out,
            indices_out,
            args_out,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one device dispatch needs its seven buffers plus sizes in one place"
    )]
    fn bind_buffers(
        layout: &wgpu::BindGroupLayout,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        sphere_bytes: &[u8],
        plane_bytes: &[u8],
        sphere_len: u64,
        flag_len: u64,
        hiz_bytes: &[u8; 16],
    ) -> Result<CullBind, CullSubmitError> {
        use wgpu::BufferUsages as U;
        let spheres_buf = cull_buffer(device, "cull-spheres", sphere_len, U::STORAGE | U::COPY_DST);
        let planes_buf = cull_buffer(device, "cull-planes", 96, U::STORAGE | U::COPY_DST);
        let flags_buf = cull_buffer(device, "cull-flags", flag_len, U::STORAGE | U::COPY_SRC);
        let count_buf = cull_buffer(
            device,
            "cull-count",
            4,
            U::STORAGE | U::COPY_DST | U::COPY_SRC,
        );
        let args_buf = cull_buffer(device, "cull-args", 20, U::STORAGE | U::COPY_SRC);
        let flags_read = cull_buffer(
            device,
            "cull-flags-read",
            flag_len,
            U::COPY_DST | U::MAP_READ,
        );
        let args_read = cull_buffer(device, "cull-args-read", 20, U::COPY_DST | U::MAP_READ);
        let hiz_buf = cull_buffer(device, "cull-hiz", 16, U::STORAGE | U::COPY_DST);
        let indices_buf = cull_buffer(device, "cull-indices", flag_len, U::STORAGE | U::COPY_SRC);
        let indices_read = cull_buffer(
            device,
            "cull-indices-read",
            flag_len,
            U::COPY_DST | U::MAP_READ,
        );
        queue.write_buffer(&spheres_buf, 0, sphere_bytes);
        queue.write_buffer(&planes_buf, 0, plane_bytes);
        queue.write_buffer(&count_buf, 0, &[0u8; 4]);
        queue.write_buffer(&hiz_buf, 0, hiz_bytes);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cull-bind"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: spheres_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: planes_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: flags_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: count_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: args_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: hiz_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: indices_buf.as_entire_binding(),
                },
            ],
        });
        Ok(CullBind {
            flags_buf,
            args_buf,
            flags_read,
            args_read,
            indices_buf,
            indices_read,
            bind,
        })
    }

    fn dispatch_and_read(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bound: &CullBind,
        flag_len: u64,
        groups: u32,
        flags_out: &mut [u32],
        args_out: &mut [u32; 5],
    ) -> Result<u32, CullSubmitError> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cull"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cull-main"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipe_cull);
            pass.set_bind_group(0, &bound.bind, &[]);
            pass.dispatch_workgroups(groups, 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cull-args"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipe_args);
            pass.set_bind_group(0, &bound.bind, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&bound.flags_buf, 0, &bound.flags_read, 0, flag_len);
        encoder.copy_buffer_to_buffer(&bound.args_buf, 0, &bound.args_read, 0, 20);
        queue.submit(Some(encoder.finish()));
        let n = flags_out.len();
        crate::accel::map_readback(device, &bound.flags_read, "cull")
            .map_err(CullSubmitError::DeviceFault)?;
        crate::accel::map_readback(device, &bound.args_read, "cull")
            .map_err(CullSubmitError::DeviceFault)?;
        let flags = crate::accel::readback_u32(&bound.flags_read, n, "cull", "flags")
            .map_err(CullSubmitError::DeviceFault)?;
        let args = crate::accel::readback_u32(&bound.args_read, 5, "cull", "args")
            .map_err(CullSubmitError::DeviceFault)?;
        bound.flags_read.unmap();
        bound.args_read.unmap();
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(CullSubmitError::FlagMismatch(format!("{err:?}")));
        }
        flags_out.copy_from_slice(&flags);
        let mut got = [0u32; 5];
        got.copy_from_slice(&args);
        args_out.copy_from_slice(&got);
        Ok(got[1])
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one Hi-Z dispatch needs its flags plus indices plus sizes in one place"
    )]
    fn dispatch_hiz_and_read(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bound: &CullBind,
        flag_len: u64,
        groups: u32,
        flags_out: &mut [u32],
        indices_out: &mut [u32],
        args_out: &mut [u32; 5],
    ) -> Result<u32, CullSubmitError> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("cull-hiz"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cull-hiz-main"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipe_hiz);
            pass.set_bind_group(0, &bound.bind, &[]);
            pass.dispatch_workgroups(groups, 1, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cull-hiz-args"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipe_args);
            pass.set_bind_group(0, &bound.bind, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&bound.flags_buf, 0, &bound.flags_read, 0, flag_len);
        encoder.copy_buffer_to_buffer(&bound.args_buf, 0, &bound.args_read, 0, 20);
        encoder.copy_buffer_to_buffer(&bound.indices_buf, 0, &bound.indices_read, 0, flag_len);
        queue.submit(Some(encoder.finish()));
        let n = flags_out.len();
        crate::accel::map_readback(device, &bound.flags_read, "cull-hiz")
            .map_err(CullSubmitError::DeviceFault)?;
        crate::accel::map_readback(device, &bound.args_read, "cull-hiz")
            .map_err(CullSubmitError::DeviceFault)?;
        crate::accel::map_readback(device, &bound.indices_read, "cull-hiz")
            .map_err(CullSubmitError::DeviceFault)?;
        let flags = crate::accel::readback_u32(&bound.flags_read, n, "cull-hiz", "flags")
            .map_err(CullSubmitError::DeviceFault)?;
        let args = crate::accel::readback_u32(&bound.args_read, 5, "cull-hiz", "args")
            .map_err(CullSubmitError::DeviceFault)?;
        let indices = crate::accel::readback_u32(&bound.indices_read, n, "cull-hiz", "indices")
            .map_err(CullSubmitError::DeviceFault)?;
        bound.flags_read.unmap();
        bound.args_read.unmap();
        bound.indices_read.unmap();
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(CullSubmitError::FlagMismatch(format!("{err:?}")));
        }
        flags_out.copy_from_slice(&flags);
        indices_out.copy_from_slice(&indices);
        let mut got = [0u32; 5];
        got.copy_from_slice(&args);
        args_out.copy_from_slice(&got);
        Ok(got[1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Rng;

    fn unit_box_planes() -> [[f32; 4]; 6] {
        [
            [1.0, 0.0, 0.0, 1.0],
            [-1.0, 0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0, 1.0],
            [0.0, -1.0, 0.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            [0.0, 0.0, -1.0, 1.0],
        ]
    }

    fn sphere(x: f32, y: f32, z: f32, r: f32) -> Sphere {
        Sphere {
            center_m: [x, y, z],
            radius_m: r,
        }
    }

    fn unit_from_bits(stream: &mut Rng, span: f32) -> f32 {
        let unit = f32::from_bits(0x3F80_0000 | (stream.next_u32() >> 9)) - 1.0;
        unit * span
    }

    fn fixture_10k() -> Vec<Sphere> {
        let base = Rng::new(2040);
        let mut out = Vec::with_capacity(10_008);
        let mut i: u64 = 0;
        while out.len() < 10_000 {
            let mut s = base.fork(i);
            let x = unit_from_bits(&mut s, 4.0) - 2.0;
            let y = unit_from_bits(&mut s, 4.0) - 2.0;
            let z = unit_from_bits(&mut s, 4.0) - 2.0;
            out.push(sphere(x, y, z, 0.1));
            i += 1;
        }
        out.push(sphere(0.0, 0.0, 0.0, 0.5));
        out.push(sphere(5.0, 0.0, 0.0, 0.5));
        out.push(sphere(-5.0, 0.0, 0.0, 0.5));
        out.push(sphere(0.0, 5.0, 0.0, 0.5));
        out.push(sphere(0.0, -5.0, 0.0, 0.5));
        out.push(sphere(0.0, 0.0, 5.0, 0.5));
        out.push(sphere(0.0, 0.0, -5.0, 0.5));
        out.push(sphere(1.0, 0.0, 0.0, 0.0));
        out
    }

    #[test]
    fn gpu_cull_cpu_boundary_keeps_and_culls() {
        let planes = unit_box_planes();
        let keep = [
            sphere(0.0, 0.0, 0.0, 0.5),
            sphere(1.0, 0.0, 0.0, 0.0),
            sphere(1.0, 0.0, 0.0, 0.5),
            sphere(0.0, 0.0, 0.0, 10.0),
        ];
        for s in keep {
            assert!(
                sphere_visible_at(&s, &planes, 0).expect("valid sphere stays"),
                "kept {s:?}"
            );
        }
        let cull = [
            sphere(5.0, 0.0, 0.0, 0.5),
            sphere(-5.0, 0.0, 0.0, 0.5),
            sphere(0.0, 0.0, 5.0, 0.1),
            sphere(f32::NAN, 0.0, 0.0, 0.5),
        ];
        for s in cull {
            let visible = match sphere_visible_at(&s, &planes, 0) {
                Ok(v) => v,
                Err(_) => false,
            };
            assert!(!visible, "culled {s:?}");
        }
        assert!(sphere_visible_at(&sphere(0.0, 0.0, 0.0, -1.0), &planes, 0).is_err());
        let mut bad = planes;
        bad[2] = [0.0, f32::NAN, 0.0, 1.0];
        assert!(sphere_visible_at(&sphere(0.0, 0.0, 0.0, 0.5), &bad, 0).is_err());
    }

    #[test]
    fn gpu_cull_cpu_flags_short_names_first_unwritten() {
        let planes = unit_box_planes();
        let spheres = [sphere(0.0, 0.0, 0.0, 0.5), sphere(5.0, 0.0, 0.0, 0.5)];
        let mut flags = [0u8; 1];
        match cull_cpu(&spheres, &planes, &mut flags) {
            Err(CullError::BadSphere { index, .. }) => {
                assert_eq!(index, flags.len(), "names the first unwritten slot")
            }
            _ => panic!("short flags must refuse with the unwritten index"),
        }
        let mut exact = [0u8; 2];
        assert_eq!(
            cull_cpu(&spheres, &planes, &mut exact).expect("exact fits"),
            2
        );
    }

    #[test]
    fn gpu_cull_cpu_10k_seeded_stable_x2() {
        let planes = unit_box_planes();
        let spheres = fixture_10k();
        assert_eq!(spheres.len(), 10_008);
        let mut a = vec![0u8; spheres.len()];
        let mut b = vec![0u8; spheres.len()];
        assert_eq!(
            cull_cpu(&spheres, &planes, &mut a).expect("culls"),
            spheres.len()
        );
        assert_eq!(
            cull_cpu(&spheres, &planes, &mut b).expect("culls"),
            spheres.len()
        );
        assert_eq!(a, b, "same bytes give same flags x2");
        assert_eq!(visible_count(&a), visible_count(&b));
        let mut manual: u32 = 0;
        for flag in &a {
            if *flag == 1 {
                manual = manual.saturating_add(1);
            }
        }
        assert_eq!(visible_count(&a), manual, "count equals manual sum");
        assert!(visible_count(&a) > 0, "some spheres stay visible");
        let len_u32 = u32::try_from(a.len()).expect("fixture fits u32");
        assert!(visible_count(&a) < len_u32, "some spheres cull away");
    }

    #[test]
    fn gpu_cull_occluded_crowd_two_pass_under_10pct() {
        let planes = [
            [1.0, 0.0, 0.0, 20.0],
            [-1.0, 0.0, 0.0, 20.0],
            [0.0, 1.0, 0.0, 20.0],
            [0.0, -1.0, 0.0, 20.0],
            [0.0, 0.0, -1.0, -0.5],
            [0.0, 0.0, 1.0, 60.0],
        ];
        let hiz = HizPyramid {
            occluder_depth_m: 5.0,
            half_extent_m: 6.0,
            levels: 4,
        };
        let base = Rng::new(2040);
        let mut spheres = Vec::with_capacity(10_000);
        for i in 0..10_000 {
            let mut s = base.fork(i as u64);
            let x = unit_from_bits(&mut s, 10.0) - 5.0;
            let y = unit_from_bits(&mut s, 10.0) - 5.0;
            let z = if (i as usize) < 900 {
                -(unit_from_bits(&mut s, 3.5) + 1.0)
            } else {
                -(unit_from_bits(&mut s, 24.0) + 6.0)
            };
            spheres.push(sphere(x, y, z, 0.3));
        }
        let mut p1a = vec![0u8; spheres.len()];
        let mut p1b = vec![0u8; spheres.len()];
        assert_eq!(
            cull_cpu(&spheres, &planes, &mut p1a).expect("pass1"),
            10_000
        );
        assert_eq!(
            cull_cpu(&spheres, &planes, &mut p1b).expect("pass1"),
            10_000
        );
        assert_eq!(p1a, p1b, "pass-1 oracle x2 identical");
        let mut p2 = vec![0u8; spheres.len()];
        assert_eq!(
            cull_cpu_hiz(&spheres, &p1a, &hiz, &mut p2).expect("pass2"),
            10_000
        );
        let kept = visible_count(&p2);
        assert!(kept <= 1_000, "at most 1000 survive, got {kept}");
        assert_eq!(kept, 900, "front row survives, wall hides the rest");
        let args = indirect_args_5(kept);
        assert_eq!(
            [
                args.len(),
                args[0] as usize,
                usize::try_from(args[1]).expect("count fits")
            ],
            [5, 36, kept as usize],
            "one draw carries the count"
        );
        assert_eq!(
            SUBMIT_ARGS_BYTES, 20,
            "fixed args write stays flat 1k to 10k"
        );
        assert!(matches!(
            cull_cpu_hiz(
                &spheres,
                &p1a,
                &HizPyramid {
                    occluder_depth_m: 5.0,
                    half_extent_m: 6.0,
                    levels: 0
                },
                &mut p2
            ),
            Err(CullError::MissingHiz { .. })
        ));
        assert_eq!(resident_grown_capacity(64), 64);
        assert_eq!(resident_grown_capacity(1_000), 1_024);
        assert_eq!(resident_grown_capacity(10_000), 16_384);
        let mut resident = ResidentCull::new();
        resident.ensure_capacity(1_000);
        let cap1k = resident.capacity();
        resident.ensure_capacity(10_000);
        assert!(resident.capacity() >= 10_000, "grows to hold 10k");
        assert!(resident.capacity() >= cap1k, "never shrinks");
        let mut args2 = [0u32; 5];
        let kept2 = cull_cpu_two_pass(&spheres, &planes, &hiz, &mut resident, &mut args2)
            .expect("two-pass compacts");
        assert_eq!(kept2, kept, "fused two-pass equals split passes");
        assert_eq!(args2, args, "one args buffer carries the count");
        assert_eq!(args2[1], kept, "args[1] is the visible count");
        let dense = &resident.indices()[..kept as usize];
        assert_eq!(dense.len(), kept as usize, "dense list holds survivors");
        let mut seen: u32 = 0;
        for (i, flag) in p2.iter().enumerate() {
            if *flag == 1 {
                let want = u32::try_from(i).expect("index fits");
                assert!(dense.contains(&want), "index {i} compacts");
                seen = seen.saturating_add(1);
            }
        }
        assert_eq!(seen, kept, "dense list covers every survivor");
        let cap_before = resident.capacity();
        let mut args3 = [0u32; 5];
        let kept3 = cull_cpu_two_pass(&spheres, &planes, &hiz, &mut resident, &mut args3)
            .expect("second frame reuses storage");
        assert_eq!((kept3, args3), (kept, args), "same bytes twice x2");
        assert_eq!(
            resident.capacity(),
            cap_before,
            "steady state allocates nothing"
        );
        let mut tiny = [0u32; 8];
        assert!(
            compact_visible(&p2, &mut tiny).is_err(),
            "short indices refuse"
        );
    }

    #[test]
    fn gpu_cull_indirect_args_are_5_u32() {
        assert_eq!(indirect_args_5(0), [36, 0, 0, 0, 0]);
        assert_eq!(indirect_args_5(7), [36, 7, 0, 0, 0]);
        assert_eq!(indirect_args_5(10_008).len(), 5);
        assert!(CULL_WGSL.contains("atomicAdd"), "shader counts visible");
        assert!(CULL_WGSL.contains("-radius"), "shader keeps the rule");
        assert!(CULL_WGSL.contains("write_args"), "shader writes args");
        assert!(CULL_WGSL.contains("main_hiz"), "shader fuses Hi-Z");
        assert!(
            CULL_WGSL.contains("hiz_params"),
            "shader fetches the pyramid"
        );
        assert!(
            CULL_WGSL.contains("visible_indices"),
            "shader compacts the list"
        );
        assert_eq!(CULL_WORKGROUP, 64);
        assert_eq!(workgroups_for_test(128), 2);
    }

    fn workgroups_for_test(n: usize) -> u32 {
        let count = u32::try_from(n).expect("count fits");
        count.div_ceil(64)
    }

    #[cfg(feature = "window")]
    #[test]
    fn gpu_cull_gpu_matches_cpu_when_adapter() {
        use crate::test_gpu::{require_or_skip, NoAdapter};
        let spheres = fixture_10k();
        let planes = unit_box_planes();
        let mut cpu8 = vec![0u8; spheres.len()];
        cull_cpu(&spheres, &planes, &mut cpu8).expect("cpu culls");
        let cpu_visible = visible_count(&cpu8);
        let cpu_args = indirect_args_5(cpu_visible);
        let probe = match crate::present::Presenter::init_offscreen() {
            Ok(p) => Ok(p),
            Err(crate::present::PresentError::NoAdapter) => {
                Err(NoAdapter("no wgpu adapter on this machine".into()))
            }
            Err(e) => panic!("presenter init failed (must fail loudly): {e}"),
        };
        let Some(presenter) = require_or_skip("gpu_cull_gpu_matches_cpu_when_adapter", probe)
        else {
            return;
        };
        let submitter = CullSubmitter::new(&presenter.device);
        let mut gpu_flags = vec![0u32; spheres.len()];
        let mut gpu_args = [0u32; 5];
        let got = submitter
            .run_on_device(
                &presenter.device,
                &presenter.queue,
                &spheres,
                &planes,
                &mut gpu_flags,
                &mut gpu_args,
            )
            .expect("device cull runs");
        assert_eq!(got, cpu_visible, "device count equals oracle");
        assert_eq!(gpu_args, cpu_args, "device args equal oracle args");
        for (i, (g, c)) in gpu_flags.iter().zip(cpu8.iter()).enumerate() {
            assert_eq!(*g, u32::from(*c), "flag {i} matches oracle");
        }
    }
}
