//! gtao: half-res GTAO ambient occlusion plus the shared look bind shape.
//!
//! Owner: render lane; consumers WIN1/ED1 lit frame through the forward pass
//! (`ao_full` below), ED1 viewport, Q1 captures; RL4/RL6/RP2 reuse
//! [`LookFrame`] and [`LookError`]. Units: pixels for sizes and radii, meters
//! for depth and falloff, ms in receipts. Determinism: row-major integer
//! orders, fixed tap tables, f64 accumulation on the CPU oracle; same inputs
//! give bit-identical bytes. Errors are typed [`LookError`]; the headless
//! twin names the missing device bind while the window build binds for real.
//!
//! Window GPU edge (documented, same crate): the submitter reuses
//! `crate::graph::look_frame_uniform_bytes` so the 32-byte look image has
//! one writer (GX1 owner); the graph tests own the shared device fixture.

use std::fmt;
use std::time::Instant;

use crate::fog::normalize3;

// ---------------------------------------------------------------------------
// Bounds: every external size is validated before allocation.
// ---------------------------------------------------------------------------

/// Largest accepted look dimension in pixels (mirrors the RA1 frame bound).
pub const MAX_LOOK_DIM: u32 = 2048;
/// Full-to-half resolution divisor for the AO buffer.
pub const GTAO_HALF_DIV: u32 = 2;
/// Fixed GTAO slice axes (Jimenez 2016 school, reimplemented, no donor code).
pub const GTAO_SLICES: usize = 4;
/// Taps per slice side; each slice samples both directions.
pub const GTAO_TAPS: usize = 3;
/// Largest accepted GTAO radius in pixels.
pub const GTAO_RADIUS_MAX_PX: f32 = 64.0;
/// Largest accepted GTAO depth falloff in meters.
pub const GTAO_FALLOFF_MAX_M: f32 = 32.0;
/// Posted GPU budget at 1080p on the reference GPU (PF2 enforces).
pub const GTAO_BUDGET_MS: f64 = 0.8;
/// Bytes per AO texel on the device (R32 float, half and full alike).
pub const GTAO_AO_BYTES_PER_TEXEL: u64 = 4;
/// Largest half-res AO transient the RA2 aliasing plan must hold in bytes.
/// 1080p packs 960x540x4 = 2,073,600 bytes; the forward pass upsamples from
/// the half buffer in-shader, so no full-res transient is ever planned.
pub const GTAO_HALF_TRANSIENT_BUDGET_BYTES: u64 = 4_200_000;
/// Posted CPU/GPU parity tolerance: worst absolute AO difference allowed
/// between the f64 CPU oracle and the f32 device passes (llvmpipe gate).
pub const GTAO_GPU_PARITY_TOL: f32 = 0.002;

/// Graph pass name for the half-res GTAO compute.
pub const GTAO_HALF_PASS: &str = "gtao-half";
/// Graph pass name for the bilateral upsample compute.
pub const GTAO_UPSAMPLE_PASS: &str = "gtao-upsample";
/// Graph resource name for the half-res AO buffer (forward reads via full).
pub const GTAO_HALF_RESOURCE: &str = "ao_half";
/// Graph resource name for the full-res AO buffer the forward pass reads.
pub const GTAO_FULL_RESOURCE: &str = "ao_full";

/// Slice axes in pixel steps: +x, +y and the two diagonals.
const SLICE_DIRS: [(i32, i32); GTAO_SLICES] = [(1, 0), (0, 1), (1, 1), (1, -1)];

// ---------------------------------------------------------------------------
// Errors: typed; bad input fails loudly, the headless twin names the bind
// it cannot run without a device, the window build never takes that path.
// ---------------------------------------------------------------------------

/// Look pass failures. `NoDevice` covers headless-twin calls with no wgpu
/// device in hand; `Device` covers device-side failures on the window build.
#[derive(Clone, PartialEq, Debug)]
pub enum LookError {
    /// Caller input is unusable; reason carried.
    Invalid(String),
    /// No device behind this call; what to build first, carried.
    NoDevice(String),
    /// A device, queue or map call failed; message carried.
    Device(String),
}

impl fmt::Display for LookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "look input: {e}"),
            Self::NoDevice(e) => write!(f, "look bind needs a device: {e}"),
            Self::Device(e) => write!(f, "look device: {e}"),
        }
    }
}

impl std::error::Error for LookError {}

// ---------------------------------------------------------------------------
// LookFrame: the shared plain-data frame uniforms every look pass binds.
// Minimal and stable: RL4/RL6/RP2 build on these four fields.
// ---------------------------------------------------------------------------

/// Frame uniforms shared by all look passes: frame counter, target size,
/// exposure anchor and sun direction. Plain data; validate before binding.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LookFrame {
    /// Frame counter for jitter and history indexing.
    pub frame_index: u64,
    /// Render target size in pixels, both components `1..=MAX_LOOK_DIM`.
    pub viewport_px: [u32; 2],
    /// Exposure anchor in EV100, must be finite.
    pub exposure_ev100: f32,
    /// Sun direction (any nonzero length), must be finite.
    pub sun_dir: [f32; 3],
}

impl LookFrame {
    /// Validate viewport, exposure and sun direction before any bind.
    pub fn validate(&self) -> Result<(), LookError> {
        if self.viewport_px[0] == 0 || self.viewport_px[1] == 0 {
            return Err(LookError::Invalid(format!(
                "look viewport {}x{} is zero",
                self.viewport_px[0], self.viewport_px[1]
            )));
        }
        if self.viewport_px[0] > MAX_LOOK_DIM || self.viewport_px[1] > MAX_LOOK_DIM {
            return Err(LookError::Invalid(format!(
                "look viewport {}x{} exceeds {MAX_LOOK_DIM}",
                self.viewport_px[0], self.viewport_px[1]
            )));
        }
        if !self.exposure_ev100.is_finite() {
            return Err(LookError::Invalid(
                "look exposure_ev100 is not finite".into(),
            ));
        }
        if !self.sun_dir.iter().all(|c| c.is_finite()) {
            return Err(LookError::Invalid("look sun_dir is not finite".into()));
        }
        let len2 = self.sun_dir[0] * self.sun_dir[0]
            + self.sun_dir[1] * self.sun_dir[1]
            + self.sun_dir[2] * self.sun_dir[2];
        if len2 < 1e-12 {
            return Err(LookError::Invalid("look sun_dir has ~zero length".into()));
        }
        Ok(())
    }

    /// Half-resolution target size in pixels (ceil divide by [`GTAO_HALF_DIV`]).
    pub fn half_size(&self) -> [u32; 2] {
        [
            self.viewport_px[0].div_ceil(GTAO_HALF_DIV),
            self.viewport_px[1].div_ceil(GTAO_HALF_DIV),
        ]
    }
}

/// Shared bind shape for every look pass: validates the frame first. The
/// headless twin then names the missing device bind (never silent); the
/// window build accepts the frame here while the per-pass uniform write in
/// the device submitter below is the bind.
#[cfg(not(feature = "window"))]
pub fn bind_look(frame: &LookFrame) -> Result<(), LookError> {
    frame.validate()?;
    Err(LookError::NoDevice(
        "look GPU bind needs the RA1 forward pass bind group; the CPU bake is the reference".into(),
    ))
}

/// Shared bind shape for every look pass: validates the frame. The device
/// submitter writes the packed look uniform and sets the bind group per
/// pass, so validation here is the bind contract on this build.
#[cfg(feature = "window")]
pub fn bind_look(frame: &LookFrame) -> Result<(), LookError> {
    frame.validate()
}

// ---------------------------------------------------------------------------
// GTAO CPU reference: half-res AO from depth plus normals, bilateral upsample.
// ---------------------------------------------------------------------------

/// GTAO configuration: full-res frame size, screen-space radius in pixels,
/// depth falloff in meters and occlusion strength. Validated up front.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GtaoConfig {
    /// Full-res frame width in pixels, `1..=MAX_LOOK_DIM`.
    pub width_px: u32,
    /// Full-res frame height in pixels, `1..=MAX_LOOK_DIM`.
    pub height_px: u32,
    /// Screen-space radius in pixels, `(0, GTAO_RADIUS_MAX_PX]`.
    pub radius_px: f32,
    /// Depth falloff in meters, `(0, GTAO_FALLOFF_MAX_M]`.
    pub falloff_m: f32,
    /// Occlusion strength, `0..=1`.
    pub intensity: f32,
}

impl GtaoConfig {
    /// Validate dims and kernel parameters before any allocation.
    pub fn validate(&self) -> Result<(), LookError> {
        if self.width_px == 0 || self.height_px == 0 {
            return Err(LookError::Invalid(format!(
                "gtao frame {}x{} is zero",
                self.width_px, self.height_px
            )));
        }
        if self.width_px > MAX_LOOK_DIM || self.height_px > MAX_LOOK_DIM {
            return Err(LookError::Invalid(format!(
                "gtao frame {}x{} exceeds {MAX_LOOK_DIM}",
                self.width_px, self.height_px
            )));
        }
        if !self.radius_px.is_finite()
            || self.radius_px <= 0.0
            || self.radius_px > GTAO_RADIUS_MAX_PX
        {
            return Err(LookError::Invalid(format!(
                "gtao radius {} outside (0, {GTAO_RADIUS_MAX_PX}]",
                self.radius_px
            )));
        }
        if !self.falloff_m.is_finite()
            || self.falloff_m <= 0.0
            || self.falloff_m > GTAO_FALLOFF_MAX_M
        {
            return Err(LookError::Invalid(format!(
                "gtao falloff {} outside (0, {GTAO_FALLOFF_MAX_M}]",
                self.falloff_m
            )));
        }
        if !self.intensity.is_finite() || self.intensity < 0.0 || self.intensity > 1.0 {
            return Err(LookError::Invalid(format!(
                "gtao intensity {} outside 0..=1",
                self.intensity
            )));
        }
        Ok(())
    }

    /// Half-res width in pixels (ceil divide).
    pub fn half_width(&self) -> usize {
        self.width_px.div_ceil(GTAO_HALF_DIV) as usize
    }

    /// Half-res height in pixels (ceil divide).
    pub fn half_height(&self) -> usize {
        self.height_px.div_ceil(GTAO_HALF_DIV) as usize
    }

    /// Half-res texel count.
    pub fn half_texels(&self) -> usize {
        self.half_width() * self.half_height()
    }

    /// Full-res texel count.
    pub fn full_texels(&self) -> usize {
        self.width_px as usize * self.height_px as usize
    }

    /// Integer tap step in pixels from the radius: `ceil(radius / TAPS)`, at
    /// least 1. Deterministic per config.
    pub fn tap_step_px(&self) -> i32 {
        (self.radius_px / GTAO_TAPS as f32).ceil().max(1.0) as i32
    }
}

/// Occlusion normalizer: sum of `1/(1+t)` over both sides of every slice.
/// One writer for the CPU bake and the device uniform below.
fn gtao_weight_sum() -> f64 {
    let mut weight_sum = 0.0f64;
    for _ in 0..GTAO_SLICES {
        for _ in 0..2 {
            for t in 1..=GTAO_TAPS {
                weight_sum += 1.0 / (1.0 + t as f64);
            }
        }
    }
    weight_sum
}

/// Half-resolution ambient occlusion: `1.0` unoccluded, `0.0` fully
/// occluded. Row-major, `half_width * half_height` values.
#[derive(Clone, PartialEq, Debug)]
pub struct GtaoAo {
    /// Half-res width in pixels.
    pub width_half: usize,
    /// Half-res height in pixels.
    pub height_half: usize,
    /// Row-major AO values in `0..=1`.
    pub values: Vec<f32>,
}

impl GtaoAo {
    /// Texel count.
    pub fn texels(&self) -> usize {
        self.values.len()
    }

    /// Read one half-res texel; bounds-checked for callers outside the bake.
    pub fn texel(&self, x: usize, y: usize) -> Result<f32, LookError> {
        if x >= self.width_half || y >= self.height_half {
            return Err(LookError::Invalid(format!(
                "gtao texel ({x},{y}) outside {}x{}",
                self.width_half, self.height_half
            )));
        }
        Ok(self.values[y * self.width_half + x])
    }

    /// Deterministic little-endian bytes: row-major f32 values. Same inputs
    /// give bit-identical bytes on every run.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.values.len() * 4);
        for v in &self.values {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    /// Bind seam for the RA1 forward AO bind on the headless twin: validates
    /// the frame and the half shape, then names the missing device bind.
    /// The window build next to it binds the baked buffer for real.
    #[cfg(not(feature = "window"))]
    pub fn submit_gpu(&self, frame: &LookFrame) -> Result<AoReceipt, LookError> {
        frame.validate()?;
        let half = frame.half_size();
        if half[0] as usize != self.width_half || half[1] as usize != self.height_half {
            return Err(LookError::Invalid(format!(
                "gtao half {}x{} mismatches frame half {}x{}",
                self.width_half, self.height_half, half[0], half[1]
            )));
        }
        Err(LookError::NoDevice(
            "gtao GPU bind needs the RA1 forward pass bind group; the CPU bake is the reference"
                .into(),
        ))
    }

    /// Bind the baked half AO on the device: uploads the half buffer, runs
    /// one bind-group probe pass over it and reads it back exactly, proving
    /// the forward bind validates with 0 errors. Fails loudly (never silent,
    /// never the twin refusal) on bad shapes or any device fault.
    #[cfg(feature = "window")]
    pub fn submit_gpu(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &LookFrame,
    ) -> Result<AoReceipt, LookError> {
        frame.validate()?;
        let half = frame.half_size();
        if half[0] as usize != self.width_half || half[1] as usize != self.height_half {
            return Err(LookError::Invalid(format!(
                "gtao half {}x{} mismatches frame half {}x{}",
                self.width_half, self.height_half, half[0], half[1]
            )));
        }
        let texels = self.texels();
        let count = u32::try_from(texels)
            .map_err(|_| LookError::Invalid(format!("gtao texels {texels} exceed u32")))?;
        let byte_len = texels as u64 * GTAO_AO_BYTES_PER_TEXEL;
        let started = Instant::now();

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gtao-ao-bind"),
            source: wgpu::ShaderSource::Wgsl(GTAO_BIND_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gtao-ao-bind-layout"),
            entries: &[
                uniform_entry(0),
                storage_entry(1, true),
                storage_entry(2, false),
            ],
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gtao-ao-bind-pipe-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gtao-ao-bind-pipeline"),
            layout: Some(&pipe_layout),
            module: &shader,
            entry_point: Some("bind_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let look_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gtao-ao-bind-look"),
            size: crate::graph::LOOK_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let src_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gtao-ao-bind-src"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let dst_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gtao-ao-bind-dst"),
            size: byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gtao-ao-bind-readback"),
            size: byte_len,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gtao-ao-bind-group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: look_buf.as_entire_binding(),
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
        queue.write_buffer(&look_buf, 0, &crate::graph::look_frame_uniform_bytes(frame));
        let mut src_bytes = Vec::with_capacity(texels * 4);
        for v in &self.values {
            src_bytes.extend_from_slice(&v.to_le_bytes());
        }
        queue.write_buffer(&src_buf, 0, &src_bytes);

        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gtao-ao-bind"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gtao-ao-bind-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&dst_buf, 0, &readback, 0, byte_len);
        queue.submit(Some(encoder.finish()));
        map_and_wait(device, &readback)?;
        let back = readback_f32(&readback, texels)?;
        readback.unmap();
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(LookError::Device(format!("gtao bind validation: {err:?}")));
        }
        for (i, (got, want)) in back.iter().zip(self.values.iter()).enumerate() {
            if got.to_bits() != want.to_bits() {
                return Err(LookError::Device(format!(
                    "gtao bind roundtrip texel {i} differs: {got} vs {want}"
                )));
            }
        }
        Ok(AoReceipt {
            half_texels: texels as u64,
            full_texels: u64::from(frame.viewport_px[0]) * u64::from(frame.viewport_px[1]),
            ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}

/// One GTAO frame receipt: buffer sizes plus wall-clock bake ms for the
/// frame report (ms is evidence only, never hashed).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AoReceipt {
    /// Half-res AO texels resident.
    pub half_texels: u64,
    /// Full-res pixels upsampled.
    pub full_texels: u64,
    /// Wall-clock bake time in ms.
    pub ms: f64,
}

impl AoReceipt {
    /// Posted GPU budget this receipt is charged against.
    pub fn budget_ms(&self) -> f64 {
        GTAO_BUDGET_MS
    }

    /// True while the bake stayed inside the posted budget.
    pub fn within_budget(&self) -> bool {
        self.ms <= GTAO_BUDGET_MS
    }
}

fn check_depth_buffer(depth: &[f32], want: usize, what: &'static str) -> Result<(), LookError> {
    if depth.len() != want {
        return Err(LookError::Invalid(format!(
            "gtao {what} length {} mismatches frame {want}",
            depth.len()
        )));
    }
    for (i, d) in depth.iter().enumerate() {
        if !d.is_finite() || *d <= 0.0 {
            return Err(LookError::Invalid(format!(
                "gtao {what}[{i}] depth {d} is not positive finite"
            )));
        }
    }
    Ok(())
}

/// Validate normal directions in place: finite and nonzero, indexed for a
/// loud refusal. Shared by the CPU baker (which then normalizes into its
/// own buffer) and the device submit (which normalizes into its scratch),
/// so both stages enforce the same rule. `pub(crate)` for the steady-state
/// allocation probe.
pub(crate) fn validate_normals(normals: &[[f32; 3]], want: usize) -> Result<(), LookError> {
    if normals.len() != want {
        return Err(LookError::Invalid(format!(
            "gtao normals length {} mismatches frame {want}",
            normals.len()
        )));
    }
    for (i, n) in normals.iter().enumerate() {
        if normalize3(*n).is_none() {
            return Err(LookError::Invalid(format!(
                "gtao normals[{i}] {n:?} is not a finite nonzero direction"
            )));
        }
    }
    Ok(())
}

fn normalized_normals(normals: &[[f32; 3]], want: usize) -> Result<Vec<[f32; 3]>, LookError> {
    validate_normals(normals, want)?;
    let mut out = Vec::with_capacity(want);
    for n in normals {
        match normalize3(*n) {
            Some(v) => out.push(v),
            None => {
                return Err(LookError::Invalid("gtao normal failed revalidation".into()));
            }
        }
    }
    Ok(out)
}

/// Half-res GTAO from full-res depth (view distance in meters) and normals.
/// Center texel per half pixel is the top-left full pixel `(2hx, 2hy)`;
/// fixed slice taps in integer order accumulate horizon occlusion scaled by
/// normal alignment. Returns the AO buffer plus wall-clock ms.
pub fn gtao_half_res(
    depth_full: &[f32],
    normals_full: &[[f32; 3]],
    config: &GtaoConfig,
) -> Result<(GtaoAo, f64), LookError> {
    config.validate()?;
    let w = config.width_px as usize;
    let h = config.height_px as usize;
    let full = config.full_texels();
    check_depth_buffer(depth_full, full, "depth")?;
    let normals = normalized_normals(normals_full, full)?;
    let started = Instant::now();
    let hw = config.half_width();
    let hh = config.half_height();
    let step = config.tap_step_px();
    let falloff = config.falloff_m as f64;
    let intensity = config.intensity as f64;
    // Sum of 1/(1+t) over both sides, all slices: the occlusion normalizer.
    let weight_sum = gtao_weight_sum();
    let mut values = vec![1.0f32; hw * hh];
    for hy in 0..hh {
        for hx in 0..hw {
            // Integer center: top-left full pixel of the 2x2 block, clamped.
            let cx = (hx * GTAO_HALF_DIV as usize).min(w - 1);
            let cy = (hy * GTAO_HALF_DIV as usize).min(h - 1);
            let ci = cy * w + cx;
            let cd = depth_full[ci] as f64;
            let nc = normals[ci];
            let mut occ = 0.0f64;
            for (dx, dy) in SLICE_DIRS {
                for t in 1..=GTAO_TAPS {
                    let off = t as i32 * step;
                    let w_tap = 1.0 / (1.0 + t as f64);
                    for side in [1i32, -1i32] {
                        let nx = (cx as i32 + dx * off * side).clamp(0, w as i32 - 1) as usize;
                        let ny = (cy as i32 + dy * off * side).clamp(0, h as i32 - 1) as usize;
                        let ni = ny * w + nx;
                        let nd = depth_full[ni] as f64;
                        let nn = normals[ni];
                        let diff = ((cd - nd) / falloff).clamp(0.0, 1.0);
                        let dot = (nc[0] * nn[0] + nc[1] * nn[1] + nc[2] * nn[2]).max(0.0) as f64;
                        occ += diff * w_tap * dot;
                    }
                }
            }
            let ao = (1.0 - intensity * (occ / weight_sum)).clamp(0.0, 1.0);
            values[hy * hw + hx] = ao as f32;
        }
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok((
        GtaoAo {
            width_half: hw,
            height_half: hh,
            values,
        },
        ms,
    ))
}

/// Depth-aware bilateral upsample from half-res AO to full-res AO. Each full
/// pixel blends its 2x2 half neighbors with bilinear spatial weights times a
/// depth-similarity range weight, so flat areas reproduce the half buffer
/// while depth edges keep the near side. Integer pixel order, no float keys.
pub fn upsample_bilateral(
    half: &GtaoAo,
    depth_full: &[f32],
    config: &GtaoConfig,
) -> Result<Vec<f32>, LookError> {
    config.validate()?;
    if half.width_half != config.half_width() || half.height_half != config.half_height() {
        return Err(LookError::Invalid(format!(
            "gtao half {}x{} mismatches config half {}x{}",
            half.width_half,
            half.height_half,
            config.half_width(),
            config.half_height()
        )));
    }
    if half.values.len() != half.width_half * half.height_half {
        return Err(LookError::Invalid(format!(
            "gtao half length {} mismatches {}x{}",
            half.values.len(),
            half.width_half,
            half.height_half
        )));
    }
    let w = config.width_px as usize;
    let h = config.height_px as usize;
    check_depth_buffer(depth_full, config.full_texels(), "upsample depth")?;
    let falloff = config.falloff_m as f64;
    let mut out = vec![1.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let hx = x / GTAO_HALF_DIV as usize;
            let hy = y / GTAO_HALF_DIV as usize;
            // Fractional position inside the half texel: 0.0 on even pixels
            // (exact center), 0.5 on odd pixels (midway to the next texel).
            let fx = x as f64 / GTAO_HALF_DIV as f64 - hx as f64;
            let fy = y as f64 / GTAO_HALF_DIV as f64 - hy as f64;
            let hx1 = (hx + 1).min(half.width_half - 1);
            let hy1 = (hy + 1).min(half.height_half - 1);
            let corners = [
                (hx, hy, (1.0 - fx) * (1.0 - fy)),
                (hx1, hy, fx * (1.0 - fy)),
                (hx, hy1, (1.0 - fx) * fy),
                (hx1, hy1, fx * fy),
            ];
            let pix_depth = depth_full[y * w + x] as f64;
            let mut acc = 0.0f64;
            let mut wsum = 0.0f64;
            for (qx, qy, spatial) in corners {
                // Half texel depth anchor: its top-left full pixel's depth.
                let ax = (qx * GTAO_HALF_DIV as usize).min(w - 1);
                let ay = (qy * GTAO_HALF_DIV as usize).min(h - 1);
                let anchor = depth_full[ay * w + ax] as f64;
                let range = 1.0 / (1.0 + ((pix_depth - anchor).abs() / falloff));
                let weight = spatial * range;
                acc += half.values[qy * half.width_half + qx] as f64 * weight;
                wsum += weight;
            }
            out[y * w + x] = if wsum > 0.0 { (acc / wsum) as f32 } else { 1.0 };
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Graph passes as data: half-res GTAO plus bilateral upsample feeding the
// WIN1 forward pass. WIN1 wires `half` -> `upsample` -> `forward` with the
// forward pass declaring `.input("ao_full")`; RA2 owns order and aliasing.
// ---------------------------------------------------------------------------

/// GTAO graph wiring for one config: pass names, resources and transient
/// bytes. The half buffer is the only production transient (the forward
/// pass upsamples from it in-shader); the full buffer below sizes the
/// parity readback the device submit runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GtaoPassPlan {
    /// Graph pass running the half-res GTAO compute.
    pub half_pass: &'static str,
    /// Graph pass running the bilateral upsample compute.
    pub upsample_pass: &'static str,
    /// Half-res AO resource the half pass writes.
    pub half_resource: &'static str,
    /// Full-res AO resource the forward pass reads.
    pub full_resource: &'static str,
    /// Half-res AO transient bytes (`half texels * 4`).
    pub half_bytes: u64,
    /// Full-res AO bytes (`full texels * 4`, parity path only).
    pub full_bytes: u64,
}

/// Validate the config, then describe the two graph passes plus the AO
/// bytes the RA2 aliasing plan must hold. Pure data, no allocation.
pub fn gtao_pass_plan(config: &GtaoConfig) -> Result<GtaoPassPlan, LookError> {
    config.validate()?;
    Ok(GtaoPassPlan {
        half_pass: GTAO_HALF_PASS,
        upsample_pass: GTAO_UPSAMPLE_PASS,
        half_resource: GTAO_HALF_RESOURCE,
        full_resource: GTAO_FULL_RESOURCE,
        half_bytes: config.half_texels() as u64 * GTAO_AO_BYTES_PER_TEXEL,
        full_bytes: config.full_texels() as u64 * GTAO_AO_BYTES_PER_TEXEL,
    })
}

// ---------------------------------------------------------------------------
// Window GPU passes: half-res GTAO compute plus bilateral upsample compute
// over the shared look bind group. Engine-owned WGSL, no donor code.
// ---------------------------------------------------------------------------

/// Byte size of the packed GTAO kernel uniform (full/half sizes, tap step,
/// falloff, intensity, occlusion normalizer, 16-byte aligned fields).
#[cfg(feature = "window")]
pub const GTAO_CFG_UNIFORM_BYTES: u64 = 40;

/// Half-res GTAO compute: one thread per half texel, fixed slice taps in
/// shader order mirroring the CPU oracle. The shared look uniform stays
/// bound and live without touching AO math.
#[cfg(feature = "window")]
const GTAO_HALF_WGSL: &str = r#"
struct GtaoCfg {
    full_wh: vec2<u32>,
    half_wh: vec2<u32>,
    step: u32,
    _pad0: u32,
    falloff: f32,
    intensity: f32,
    weight_sum: f32,
    _pad1: f32,
};
struct LookU {
    frame_idx: u32,
    width_px: u32,
    height_px: u32,
    exposure: f32,
    sun_dir: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0) var<uniform> cfg: GtaoCfg;
@group(0) @binding(1) var<uniform> look: LookU;
@group(0) @binding(2) var<storage, read> depth: array<f32>;
@group(0) @binding(3) var<storage, read> normals: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> ao_half: array<f32>;
@compute @workgroup_size(8, 8)
fn half_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let hx = gid.x;
    let hy = gid.y;
    if (hx >= cfg.half_wh.x || hy >= cfg.half_wh.y) { return; }
    let fw = cfg.full_wh.x;
    let fh = cfg.full_wh.y;
    let cx = min(hx * 2u, fw - 1u);
    let cy = min(hy * 2u, fh - 1u);
    let ci = cy * fw + cx;
    let cd = depth[ci];
    let nc = normals[ci].xyz;
    var occ = 0.0;
    for (var s = 0u; s < 4u; s = s + 1u) {
        var dx: i32 = 0;
        var dy: i32 = 0;
        if (s == 0u) { dx = 1; }
        else if (s == 1u) { dy = 1; }
        else if (s == 2u) { dx = 1; dy = 1; }
        else { dx = 1; dy = -1; }
        for (var t = 1u; t <= 3u; t = t + 1u) {
            let off = i32(t * cfg.step);
            let w_tap = 1.0 / (1.0 + f32(t));
            for (var side: i32 = 0; side < 2; side = side + 1) {
                let sgn = select(-1i, 1i, side == 1);
                let nx = clamp(i32(cx) + dx * off * sgn, 0, i32(fw) - 1);
                let ny = clamp(i32(cy) + dy * off * sgn, 0, i32(fh) - 1);
                let ni = u32(ny) * fw + u32(nx);
                let nd = depth[ni];
                let nn = normals[ni].xyz;
                let diff = clamp((cd - nd) / cfg.falloff, 0.0, 1.0);
                let align = max(dot(nc, nn), 0.0);
                occ = occ + diff * w_tap * align;
            }
        }
    }
    let ao = clamp(1.0 - cfg.intensity * (occ / cfg.weight_sum), 0.0, 1.0);
    ao_half[hy * cfg.half_wh.x + hx] = ao + look.exposure * 0.0;
}
"#;

/// Bilateral upsample compute: one thread per full pixel, 2x2 half
/// neighbors with bilinear spatial weights times the depth-similarity
/// range weight, mirroring the CPU oracle. Look uniform bound and live.
#[cfg(feature = "window")]
const GTAO_UPSAMPLE_WGSL: &str = r#"
struct GtaoCfg {
    full_wh: vec2<u32>,
    half_wh: vec2<u32>,
    step: u32,
    _pad0: u32,
    falloff: f32,
    intensity: f32,
    weight_sum: f32,
    _pad1: f32,
};
struct LookU {
    frame_idx: u32,
    width_px: u32,
    height_px: u32,
    exposure: f32,
    sun_dir: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0) var<uniform> cfg: GtaoCfg;
@group(0) @binding(1) var<uniform> look: LookU;
@group(0) @binding(2) var<storage, read> ao_half: array<f32>;
@group(0) @binding(3) var<storage, read> depth: array<f32>;
@group(0) @binding(4) var<storage, read_write> ao_full: array<f32>;
@compute @workgroup_size(8, 8)
fn up_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    let fw = cfg.full_wh.x;
    let fh = cfg.full_wh.y;
    if (x >= fw || y >= fh) { return; }
    let hw = cfg.half_wh.x;
    let hh = cfg.half_wh.y;
    let hx = x / 2u;
    let hy = y / 2u;
    let fx = f32(x) / 2.0 - f32(hx);
    let fy = f32(y) / 2.0 - f32(hy);
    let hx1 = min(hx + 1u, hw - 1u);
    let hy1 = min(hy + 1u, hh - 1u);
    let d = depth[y * fw + x];
    let w00 = (1.0 - fx) * (1.0 - fy);
    let w10 = fx * (1.0 - fy);
    let w01 = (1.0 - fx) * fy;
    let w11 = fx * fy;
    let ax0 = min(hx * 2u, fw - 1u);
    let ax1 = min(hx1 * 2u, fw - 1u);
    let ay0 = min(hy * 2u, fh - 1u);
    let ay1 = min(hy1 * 2u, fh - 1u);
    let r00 = 1.0 / (1.0 + abs(d - depth[ay0 * fw + ax0]) / cfg.falloff);
    let r10 = 1.0 / (1.0 + abs(d - depth[ay0 * fw + ax1]) / cfg.falloff);
    let r01 = 1.0 / (1.0 + abs(d - depth[ay1 * fw + ax0]) / cfg.falloff);
    let r11 = 1.0 / (1.0 + abs(d - depth[ay1 * fw + ax1]) / cfg.falloff);
    let acc = ao_half[hy * hw + hx] * w00 * r00
        + ao_half[hy * hw + hx1] * w10 * r10
        + ao_half[hy1 * hw + hx] * w01 * r01
        + ao_half[hy1 * hw + hx1] * w11 * r11;
    let wsum = w00 * r00 + w10 * r10 + w01 * r01 + w11 * r11;
    ao_full[y * fw + x] = acc / wsum + look.exposure * 0.0;
}
"#;

/// Bind probe over a baked half AO: copies the bound buffer to prove the
/// forward bind-group path validates. Look uniform bound and live.
#[cfg(feature = "window")]
const GTAO_BIND_WGSL: &str = r#"
struct LookU {
    frame_idx: u32,
    width_px: u32,
    height_px: u32,
    exposure: f32,
    sun_dir: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0) var<uniform> look: LookU;
@group(0) @binding(1) var<storage, read> src: array<f32>;
@group(0) @binding(2) var<storage, read_write> dst: array<f32>;
@compute @workgroup_size(64)
fn bind_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&src)) { return; }
    dst[i] = src[i] + look.exposure * 0.0;
}
"#;

/// Uniform bind-group entry over the compute stage.
#[cfg(feature = "window")]
fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Storage bind-group entry over the compute stage.
#[cfg(feature = "window")]
fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Pack the kernel uniform: full/half sizes, tap step, falloff, intensity
/// and the occlusion normalizer. Pure bytes; validation stays on the caller.
#[cfg(feature = "window")]
fn gtao_uniform_bytes(config: &GtaoConfig) -> [u8; 40] {
    let mut out = [0u8; 40];
    out[0..4].copy_from_slice(&config.width_px.to_le_bytes());
    out[4..8].copy_from_slice(&config.height_px.to_le_bytes());
    out[8..12].copy_from_slice(&(config.half_width() as u32).to_le_bytes());
    out[12..16].copy_from_slice(&(config.half_height() as u32).to_le_bytes());
    out[16..20].copy_from_slice(&(config.tap_step_px() as u32).to_le_bytes());
    out[24..28].copy_from_slice(&config.falloff_m.to_le_bytes());
    out[28..32].copy_from_slice(&config.intensity.to_le_bytes());
    out[32..36].copy_from_slice(&(gtao_weight_sum() as f32).to_le_bytes());
    out
}

/// Map a readback buffer and wait for it. One poll covers every pending map.
#[cfg(feature = "window")]
fn map_and_wait(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Result<(), LookError> {
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| LookError::Device(format!("gtao poll: {e}")))?;
    rx.recv()
        .map_err(|e| LookError::Device(format!("gtao map channel: {e}")))?
        .map_err(|e| LookError::Device(format!("gtao map: {e}")))?;
    Ok(())
}

/// Decode `out.len()` little-endian f32 values from a mapped readback
/// buffer into caller-owned storage. The submit path reuses its output
/// buffers through this helper, so steady-state submits grow nothing; the
/// one-shot bind probe keeps [`readback_f32`].
#[cfg(feature = "window")]
fn readback_f32_into(buffer: &wgpu::Buffer, out: &mut [f32]) -> Result<(), LookError> {
    let mapped = buffer
        .slice(..)
        .get_mapped_range()
        .map_err(|e| LookError::Device(format!("gtao mapped range: {e}")))?;
    let want = out.len() * 4;
    if mapped.len() < want {
        return Err(LookError::Device(format!(
            "gtao readback {} bytes holds fewer than {} f32 values",
            mapped.len(),
            out.len()
        )));
    }
    for (slot, chunk) in out.iter_mut().zip(mapped[..want].chunks_exact(4)) {
        *slot = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    Ok(())
}

/// Read `count` little-endian f32 values from a mapped readback buffer.
/// One-shot path for the baked bind probe; the submit path reuses its
/// outputs through [`readback_f32_into`] instead.
#[cfg(feature = "window")]
fn readback_f32(buffer: &wgpu::Buffer, count: usize) -> Result<Vec<f32>, LookError> {
    let mut out = vec![0.0f32; count];
    readback_f32_into(buffer, &mut out)?;
    Ok(out)
}

/// Validate config, frame and every input value before the submitter sizes
/// anything, so a refusal sizes nothing. No allocation: values are checked
/// in place; normalization stays in the fill step.
#[cfg(feature = "window")]
fn check_submit_inputs(
    config: &GtaoConfig,
    frame: &LookFrame,
    depth_full: &[f32],
    normals_full: &[[f32; 3]],
) -> Result<(), LookError> {
    config.validate()?;
    frame.validate()?;
    if frame.viewport_px != [config.width_px, config.height_px] {
        return Err(frame_config_mismatch(frame, config));
    }
    let full_n = config.full_texels();
    check_depth_buffer(depth_full, full_n, "depth")?;
    validate_normals(normals_full, full_n)?;
    Ok(())
}

/// Refusal text for a frame/config viewport mismatch. Cold path only.
#[cfg(feature = "window")]
fn frame_config_mismatch(frame: &LookFrame, config: &GtaoConfig) -> LookError {
    LookError::Invalid(format!(
        "gtao frame {}x{} mismatches config {}x{}",
        frame.viewport_px[0], frame.viewport_px[1], config.width_px, config.height_px
    ))
}

/// Refusal for sized buffers that vanished mid-submit. Cold path only.
#[cfg(feature = "window")]
fn lost_buffers() -> LookError {
    LookError::Device("gtao submitter lost its sized buffers".into())
}

/// Device refusal text for a validation-scope hit. Cold path only.
#[cfg(feature = "window")]
fn validation_refusal(err: wgpu::Error) -> LookError {
    LookError::Device(format!("gtao submit validation: {err:?}"))
}

/// Normalize validated directions into caller-owned storage, growing it
/// only when the shape outgrows the current capacity. Values passed
/// [`validate_normals`], so `None` is unreachable; it still fails loudly
/// through a typed error instead of panicking. `pub(crate)` for the
/// steady-state allocation probe.
#[cfg(feature = "window")]
pub(crate) fn fill_normal_dirs(src: &[[f32; 3]], out: &mut Vec<[f32; 3]>) -> Result<(), LookError> {
    out.clear();
    if out.capacity() < src.len() {
        out.reserve(src.len() - out.capacity());
    }
    for (i, n) in src.iter().enumerate() {
        match normalize3(*n) {
            Some(v) => out.push(v),
            None => {
                return Err(LookError::Invalid(format!(
                    "gtao normals[{i}] {n:?} failed revalidation"
                )));
            }
        }
    }
    Ok(())
}

/// Pack depth values as little-endian bytes into caller-owned storage,
/// which must hold `depth.len() * 4` bytes. The submitter sizes it on shape
/// change, so steady-state packing grows nothing. `pub(crate)` for the
/// steady-state allocation probe.
#[cfg(feature = "window")]
pub(crate) fn pack_depth_bytes(depth: &[f32], out: &mut [u8]) {
    debug_assert_eq!(out.len(), depth.len() * 4);
    for (d, chunk) in depth.iter().zip(out.chunks_exact_mut(4)) {
        chunk.copy_from_slice(&d.to_le_bytes());
    }
}

/// Pack normal directions plus pad as little-endian bytes (`x, y, z, 0`
/// per direction) into caller-owned storage, which must hold
/// `normals.len() * 16` bytes. Same caller-owned reuse rule as
/// [`pack_depth_bytes`]. `pub(crate)` for the steady-state allocation
/// probe.
#[cfg(feature = "window")]
pub(crate) fn pack_normal_bytes(normals: &[[f32; 3]], out: &mut [u8]) {
    debug_assert_eq!(out.len(), normals.len() * 16);
    let zero = 0f32.to_le_bytes();
    for (n, chunk) in normals.iter().zip(out.chunks_exact_mut(16)) {
        chunk[0..4].copy_from_slice(&n[0].to_le_bytes());
        chunk[4..8].copy_from_slice(&n[1].to_le_bytes());
        chunk[8..12].copy_from_slice(&n[2].to_le_bytes());
        chunk[12..16].copy_from_slice(&zero);
    }
}

/// One device submit: AO texel counts, the validation outcome and the
/// wall-clock encode + submit + readback ms (evidence only, never hashed).
/// `upload_ms` splits out the staging upload so the budget probe can tell
/// host transfer from device execution.
#[cfg(feature = "window")]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GtaoGpuReceipt {
    /// Half-res AO texels written.
    pub half_texels: u64,
    /// Full-res pixels upsampled.
    pub full_texels: u64,
    /// wgpu validation errors observed; green means 0.
    pub validation_errors: u32,
    /// Wall time for the staging upload in ms (evidence only).
    pub upload_ms: f64,
    /// Wall time for encode + submit + readback in ms (evidence only).
    pub submit_ms: f64,
}

#[cfg(feature = "window")]
impl GtaoGpuReceipt {
    /// Posted GPU budget this submit is charged against.
    pub fn budget_ms(&self) -> f64 {
        GTAO_BUDGET_MS
    }

    /// True while the submit stayed inside the posted budget.
    pub fn within_budget(&self) -> bool {
        self.submit_ms <= GTAO_BUDGET_MS
    }

    /// True when the submit observed zero validation errors.
    pub fn is_clean(&self) -> bool {
        self.validation_errors == 0
    }
}

/// Reusable device state for the two GTAO passes: both compute pipelines
/// plus the kernel and look bind layouts over two uniform buffers, with
/// shape-keyed storage buffers and bind groups. Built once per frame loop;
/// steady-state submits on a matching shape create no buffers or bind
/// groups (the reuse test proves the count stays flat) and grow nothing on
/// the host either: uploads pack into the owned scratch below and outputs
/// decode into the owned buffers behind [`GtaoSubmitter::half_ao`] and
/// [`GtaoSubmitter::full_ao`] (the scratch test proves the capacities stay
/// flat). Per-submit encoders and the uniform rewrites are driver work, same
/// as the graph submitter.
#[cfg(feature = "window")]
pub struct GtaoSubmitter {
    half_pipeline: wgpu::ComputePipeline,
    up_pipeline: wgpu::ComputePipeline,
    half_layout: wgpu::BindGroupLayout,
    up_layout: wgpu::BindGroupLayout,
    cfg_buf: wgpu::Buffer,
    look_buf: wgpu::Buffer,
    cached_shape: Option<(u32, u32)>,
    depth_buf: Option<wgpu::Buffer>,
    normal_buf: Option<wgpu::Buffer>,
    half_buf: Option<wgpu::Buffer>,
    full_buf: Option<wgpu::Buffer>,
    half_rb: Option<wgpu::Buffer>,
    full_rb: Option<wgpu::Buffer>,
    half_bind: Option<wgpu::BindGroup>,
    up_bind: Option<wgpu::BindGroup>,
    created_buffers: u64,
    /// Packed depth staging (`full texels * 4` bytes), sized on shape
    /// change and refilled per submit.
    depth_scratch: Vec<u8>,
    /// Packed normal staging (`full texels * 16` bytes), same reuse rule.
    normal_scratch: Vec<u8>,
    /// Normalized normal directions (`full texels`), same reuse rule.
    normal_dirs: Vec<[f32; 3]>,
    /// Last half-res occlusion the device wrote back.
    half_out: GtaoAo,
    /// Last full-res upsample the device wrote back.
    full_out: Vec<f32>,
}

#[cfg(feature = "window")]
impl GtaoSubmitter {
    /// Compile both compute pipelines and the kernel plus look bind layouts;
    /// allocate the two uniform buffers. No frame-size state yet: storage
    /// buffers size on the first submit and resize only on shape change.
    pub fn new(device: &wgpu::Device) -> Self {
        let half_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gtao-half"),
            source: wgpu::ShaderSource::Wgsl(GTAO_HALF_WGSL.into()),
        });
        let up_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gtao-upsample"),
            source: wgpu::ShaderSource::Wgsl(GTAO_UPSAMPLE_WGSL.into()),
        });
        let half_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gtao-half-layout"),
            entries: &[
                uniform_entry(0),
                uniform_entry(1),
                storage_entry(2, true),
                storage_entry(3, true),
                storage_entry(4, false),
            ],
        });
        let up_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gtao-upsample-layout"),
            entries: &[
                uniform_entry(0),
                uniform_entry(1),
                storage_entry(2, true),
                storage_entry(3, true),
                storage_entry(4, false),
            ],
        });
        let half_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gtao-half-pipe-layout"),
            bind_group_layouts: &[Some(&half_layout)],
            immediate_size: 0,
        });
        let up_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gtao-upsample-pipe-layout"),
            bind_group_layouts: &[Some(&up_layout)],
            immediate_size: 0,
        });
        let half_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gtao-half-pipeline"),
            layout: Some(&half_pipe_layout),
            module: &half_shader,
            entry_point: Some("half_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let up_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gtao-upsample-pipeline"),
            layout: Some(&up_pipe_layout),
            module: &up_shader,
            entry_point: Some("up_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let cfg_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gtao-cfg-uniform"),
            size: GTAO_CFG_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let look_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gtao-look-uniform"),
            size: crate::graph::LOOK_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            half_pipeline,
            up_pipeline,
            half_layout,
            up_layout,
            cfg_buf,
            look_buf,
            cached_shape: None,
            depth_buf: None,
            normal_buf: None,
            half_buf: None,
            full_buf: None,
            half_rb: None,
            full_rb: None,
            half_bind: None,
            up_bind: None,
            created_buffers: 0,
            depth_scratch: Vec::new(),
            normal_scratch: Vec::new(),
            normal_dirs: Vec::new(),
            half_out: GtaoAo {
                width_half: 0,
                height_half: 0,
                values: Vec::new(),
            },
            full_out: Vec::new(),
        }
    }

    /// Sized storage plus readback buffers created so far (probe for the
    /// reuse test: flat across matching-shape submits).
    pub fn created_buffers(&self) -> u64 {
        self.created_buffers
    }

    /// Host scratch capacities in bytes and elements, in field order
    /// (depth, normals, directions, half outputs, full outputs). Probe for
    /// the scratch test: flat across matching-shape submits.
    pub fn scratch_capacities(&self) -> (usize, usize, usize, usize, usize) {
        (
            self.depth_scratch.capacity(),
            self.normal_scratch.capacity(),
            self.normal_dirs.capacity(),
            self.half_out.values.capacity(),
            self.full_out.capacity(),
        )
    }

    /// Last half-res occlusion the device wrote back (empty before the
    /// first submit).
    pub fn half_ao(&self) -> &GtaoAo {
        &self.half_out
    }

    /// Last full-res upsample the device wrote back (empty before the
    /// first submit).
    pub fn full_ao(&self) -> &[f32] {
        &self.full_out
    }

    /// Run the half-res GTAO pass plus the bilateral upsample pass on the
    /// device and read both buffers back into submitter-owned outputs (see
    /// [`GtaoSubmitter::half_ao`] and [`GtaoSubmitter::full_ao`]).
    /// Validates every input before touching the device, so a refusal sizes
    /// nothing. Fails loudly (never silent, never the twin refusal) on bad
    /// inputs or any device fault. Steady-state submits allocate nothing on
    /// the host: uniforms are fixed arrays and uploads pack into owned
    /// scratch.
    pub fn submit(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        depth_full: &[f32],
        normals_full: &[[f32; 3]],
        config: &GtaoConfig,
        frame: &LookFrame,
    ) -> Result<GtaoGpuReceipt, LookError> {
        check_submit_inputs(config, frame, depth_full, normals_full)?;
        let started = Instant::now();
        self.ensure_storage(device, config)?;
        let upload_start = Instant::now();
        self.write_submit_inputs(queue, config, frame, depth_full, normals_full)?;
        let upload_ms = upload_start.elapsed().as_secs_f64() * 1000.0;
        self.run_passes(device, queue, config)?;
        Ok(GtaoGpuReceipt {
            half_texels: u64::try_from(config.half_texels())
                .map_err(|_| LookError::Invalid("gtao half texels exceed u64".into()))?,
            full_texels: u64::try_from(config.full_texels())
                .map_err(|_| LookError::Invalid("gtao full texels exceed u64".into()))?,
            validation_errors: 0,
            upload_ms,
            submit_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// Size storage plus readback buffers and host scratch on shape change;
    /// matching shapes reuse everything. Runs after validation, so refusals
    /// still size nothing.
    fn ensure_storage(
        &mut self,
        device: &wgpu::Device,
        config: &GtaoConfig,
    ) -> Result<(), LookError> {
        let (w, h) = (config.width_px, config.height_px);
        if self.cached_shape == Some((w, h)) {
            return Ok(());
        }
        let full_n = config.full_texels();
        let half_n = config.half_texels();
        let full_bytes = u64::try_from(full_n)
            .map_err(|_| LookError::Invalid("gtao full texels exceed u64".into()))?
            * GTAO_AO_BYTES_PER_TEXEL;
        let half_bytes = u64::try_from(half_n)
            .map_err(|_| LookError::Invalid("gtao half texels exceed u64".into()))?
            * GTAO_AO_BYTES_PER_TEXEL;
        let normal_bytes = u64::try_from(full_n)
            .map_err(|_| LookError::Invalid("gtao full texels exceed u64".into()))?
            * 16;
        self.depth_scratch.resize(full_n * 4, 0);
        self.normal_scratch.resize(full_n * 16, 0);
        if self.normal_dirs.capacity() < full_n {
            self.normal_dirs
                .reserve(full_n - self.normal_dirs.capacity());
        }
        self.half_out = GtaoAo {
            width_half: config.half_width(),
            height_half: config.half_height(),
            values: vec![1.0f32; half_n],
        };
        self.full_out.resize(full_n, 1.0);
        let depth_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gtao-depth"),
            size: full_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let normal_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gtao-normals"),
            size: normal_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let half_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gtao-half-ao"),
            size: half_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let full_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gtao-full-ao"),
            size: full_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let half_rb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gtao-half-readback"),
            size: half_bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let full_rb = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gtao-full-readback"),
            size: full_bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        self.create_binds(device, &depth_buf, &normal_buf, &half_buf, &full_buf);
        self.depth_buf = Some(depth_buf);
        self.normal_buf = Some(normal_buf);
        self.half_buf = Some(half_buf);
        self.full_buf = Some(full_buf);
        self.half_rb = Some(half_rb);
        self.full_rb = Some(full_rb);
        self.cached_shape = Some((w, h));
        self.created_buffers = self.created_buffers.saturating_add(6);
        Ok(())
    }

    /// Build both pass bind groups over sized storage buffers. Runs only on
    /// shape change, right after the buffers land.
    fn create_binds(
        &mut self,
        device: &wgpu::Device,
        depth_buf: &wgpu::Buffer,
        normal_buf: &wgpu::Buffer,
        half_buf: &wgpu::Buffer,
        full_buf: &wgpu::Buffer,
    ) {
        let half_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gtao-half-bind"),
            layout: &self.half_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.cfg_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.look_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: depth_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: normal_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: half_buf.as_entire_binding(),
                },
            ],
        });
        let up_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gtao-upsample-bind"),
            layout: &self.up_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.cfg_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.look_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: half_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: depth_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: full_buf.as_entire_binding(),
                },
            ],
        });
        self.half_bind = Some(half_bind);
        self.up_bind = Some(up_bind);
    }

    /// Rewrite uniforms plus the staged depth/normal uploads for this
    /// submit. Uniform images are fixed arrays and scratch is
    /// submitter-owned, so this performs no host allocation; the queue
    /// stages the bytes to the device.
    fn write_submit_inputs(
        &mut self,
        queue: &wgpu::Queue,
        config: &GtaoConfig,
        frame: &LookFrame,
        depth_full: &[f32],
        normals_full: &[[f32; 3]],
    ) -> Result<(), LookError> {
        queue.write_buffer(&self.cfg_buf, 0, &gtao_uniform_bytes(config));
        queue.write_buffer(
            &self.look_buf,
            0,
            &crate::graph::look_frame_uniform_bytes(frame),
        );
        fill_normal_dirs(normals_full, &mut self.normal_dirs)?;
        pack_depth_bytes(depth_full, &mut self.depth_scratch);
        pack_normal_bytes(&self.normal_dirs, &mut self.normal_scratch);
        let (depth_buf, normal_buf) = match (&self.depth_buf, &self.normal_buf) {
            (Some(a), Some(b)) => (a, b),
            _ => return Err(lost_buffers()),
        };
        queue.write_buffer(depth_buf, 0, &self.depth_scratch);
        queue.write_buffer(normal_buf, 0, &self.normal_scratch);
        Ok(())
    }

    /// Record both compute passes, submit once and decode both readbacks
    /// into the owned outputs. Readback buffers are driver resources (same
    /// carve-out as the graph submitter); host outputs are reused.
    fn run_passes(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: &GtaoConfig,
    ) -> Result<(), LookError> {
        let hw = u32::try_from(config.half_width())
            .map_err(|_| LookError::Invalid("gtao half width exceeds u32".into()))?;
        let hh = u32::try_from(config.half_height())
            .map_err(|_| LookError::Invalid("gtao half height exceeds u32".into()))?;
        let (w, h) = (config.width_px, config.height_px);
        let half_bytes = u64::try_from(config.half_texels())
            .map_err(|_| LookError::Invalid("gtao half texels exceed u64".into()))?
            * GTAO_AO_BYTES_PER_TEXEL;
        let full_bytes = u64::try_from(config.full_texels())
            .map_err(|_| LookError::Invalid("gtao full texels exceed u64".into()))?
            * GTAO_AO_BYTES_PER_TEXEL;
        let (half_buf, full_buf, half_rb, full_rb, half_bind, up_bind) = match (
            &self.half_buf,
            &self.full_buf,
            &self.half_rb,
            &self.full_rb,
            &self.half_bind,
            &self.up_bind,
        ) {
            (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)) => (a, b, c, d, e, f),
            _ => return Err(lost_buffers()),
        };
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gtao-frame"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gtao-half-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.half_pipeline);
            pass.set_bind_group(0, half_bind, &[]);
            pass.dispatch_workgroups(hw.div_ceil(8), hh.div_ceil(8), 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gtao-upsample-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.up_pipeline);
            pass.set_bind_group(0, up_bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        encoder.copy_buffer_to_buffer(half_buf, 0, half_rb, 0, half_bytes);
        encoder.copy_buffer_to_buffer(full_buf, 0, full_rb, 0, full_bytes);
        queue.submit(Some(encoder.finish()));
        map_and_wait(device, half_rb)?;
        map_and_wait(device, full_rb)?;
        readback_f32_into(half_rb, &mut self.half_out.values)?;
        readback_f32_into(full_rb, &mut self.full_out)?;
        half_rb.unmap();
        full_rb.unmap();
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(validation_refusal(err));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests: LookFrame validation, flat/step/normal fixtures, determinism x2,
// upsample edges, graph wiring into the forward pass, loud failures, the
// headless twin refusal plus the window device passes.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> LookFrame {
        LookFrame {
            frame_index: 7,
            viewport_px: [16, 12],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        }
    }

    fn config_8x8() -> GtaoConfig {
        GtaoConfig {
            width_px: 8,
            height_px: 8,
            radius_px: 4.0,
            falloff_m: 2.0,
            intensity: 1.0,
        }
    }

    fn flat_inputs(w: usize, h: usize, depth: f32) -> (Vec<f32>, Vec<[f32; 3]>) {
        (vec![depth; w * h], vec![[0.0, 0.0, 1.0]; w * h])
    }

    /// Analytic fixture shared by the determinism and device parity tests:
    /// planar ramp plus a sine ridge with tilted normals.
    fn ridge_inputs(w: usize, h: usize) -> (Vec<f32>, Vec<[f32; 3]>) {
        let mut depth = Vec::with_capacity(w * h);
        let mut normals = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let ridge = (((x as f32 * 0.7) + (y as f32 * 0.3)).sin() * 0.5 + 0.5) * 1.5;
                depth.push(3.0 + x as f32 * 0.1 + y as f32 * 0.05 + ridge);
                let tilt = ((x as f32 * 0.35) + (y as f32 * 0.2)).sin() * 0.4;
                normals.push([tilt, 0.0, 1.0]);
            }
        }
        (depth, normals)
    }

    #[test]
    fn look_frame_accepts_sane_frame() {
        let f = frame();
        f.validate().expect("sane frame validates");
        assert_eq!(f.half_size(), [8, 6], "half size ceil-divides by 2");
        assert_eq!(
            LookFrame {
                viewport_px: [15, 7],
                ..f
            }
            .half_size(),
            [8, 4],
            "odd dims round up so no pixel is dropped"
        );
        #[cfg(not(feature = "window"))]
        bind_look(&f).expect_err("headless twin names the missing device bind");
        #[cfg(feature = "window")]
        bind_look(&f).expect("window build validates the look frame");
    }

    #[test]
    fn look_frame_rejects_zero_or_oversize_viewport() {
        for viewport_px in [[0, 12], [16, 0], [0, 0]] {
            let bad = LookFrame {
                viewport_px,
                ..frame()
            };
            assert!(
                matches!(bad.validate(), Err(LookError::Invalid(_))),
                "zero viewport {viewport_px:?} must fail loudly"
            );
            assert!(matches!(bind_look(&bad), Err(LookError::Invalid(_))));
        }
        let big = LookFrame {
            viewport_px: [4096, 12],
            ..frame()
        };
        assert!(matches!(big.validate(), Err(LookError::Invalid(_))));
    }

    #[test]
    fn look_frame_rejects_non_finite_or_zero_direction() {
        for bad in [
            LookFrame {
                exposure_ev100: f32::NAN,
                ..frame()
            },
            LookFrame {
                exposure_ev100: f32::INFINITY,
                ..frame()
            },
            LookFrame {
                sun_dir: [f32::NAN, 0.0, 1.0],
                ..frame()
            },
            LookFrame {
                sun_dir: [0.0, 0.0, 0.0],
                ..frame()
            },
            LookFrame {
                sun_dir: [1.0, 0.0, f32::INFINITY],
                ..frame()
            },
        ] {
            assert!(
                matches!(bad.validate(), Err(LookError::Invalid(_))),
                "bad frame {bad:?} must fail loudly"
            );
        }
        frame().validate().expect("sane frame validates");
    }

    #[test]
    fn errors_name_cause_loudly() {
        for err in [
            LookError::Invalid("bad viewport".into()),
            LookError::NoDevice("needs the forward bind group".into()),
            LookError::Device("poll failed".into()),
        ] {
            let text = format!("{err}");
            assert!(!text.is_empty(), "errors are never silent");
            let debug = format!("{err:?}");
            assert!(!debug.is_empty(), "debug names the variant");
        }
        assert!(format!("{:?}", LookError::NoDevice("x".into())).contains("NoDevice"));
    }

    #[test]
    fn flat_plane_is_unoccluded() {
        // Constant depth plus constant normals: every tap sees zero height
        // difference, so every half texel stays exactly 1.0.
        let (depth, normals) = flat_inputs(8, 8, 5.0);
        let (ao, ms) = gtao_half_res(&depth, &normals, &config_8x8()).expect("flat bakes");
        assert_eq!((ao.width_half, ao.height_half), (4, 4));
        for v in &ao.values {
            assert_eq!(*v, 1.0, "flat plane is fully unoccluded");
        }
        assert!(ms >= 0.0, "bake records pass ms");
        let receipt = AoReceipt {
            half_texels: ao.texels() as u64,
            full_texels: config_8x8().full_texels() as u64,
            ms,
        };
        assert_eq!((receipt.half_texels, receipt.full_texels), (16, 64));
        assert_eq!(receipt.budget_ms(), GTAO_BUDGET_MS);
    }

    #[test]
    fn depth_step_occludes_the_valley_side() {
        // Left half near (2 m), right half far (6 m): far pixels beside the
        // edge see nearer occluders, near pixels see nothing behind them.
        let mut depth = vec![0.0f32; 64];
        for y in 0..8 {
            for x in 0..8 {
                depth[y * 8 + x] = if x < 4 { 2.0 } else { 6.0 };
            }
        }
        let normals = vec![[0.0, 0.0, 1.0]; 64];
        let (ao, _) = gtao_half_res(&depth, &normals, &config_8x8()).expect("step bakes");
        // Half texel (0,1) covers full x=0..1 (plateau); (2,1) covers x=4..5
        // (valley just right of the edge).
        let plateau = ao.texel(0, 1).expect("plateau reads");
        let valley = ao.texel(2, 1).expect("valley reads");
        assert_eq!(plateau, 1.0, "plateau sees no nearer occluder");
        assert!(
            valley < 1.0,
            "valley beside the step must occlude, got {valley}"
        );
        assert!(
            valley > 0.0,
            "partial occlusion stays inside 0..1, got {valley}"
        );
        // Far from the edge the step is out of radius: full occlusion fades.
        let far = ao.texel(3, 1).expect("far valley reads");
        assert!(
            far >= valley,
            "occlusion fades with distance: near {valley}, far {far}"
        );
    }

    #[test]
    fn normals_scale_the_occlusion() {
        // Same depth step twice: aligned normals occlude, orthogonal neighbor
        // normals (dot 0) contribute nothing, so AO must differ.
        let mut depth = vec![0.0f32; 64];
        for y in 0..8 {
            for x in 0..8 {
                depth[y * 8 + x] = if x < 4 { 2.0 } else { 6.0 };
            }
        }
        let aligned = vec![[0.0, 0.0, 1.0]; 64];
        let mut tilted = vec![[0.0, 0.0, 1.0]; 64];
        for y in 0..8 {
            for x in 0..4 {
                tilted[y * 8 + x] = [1.0, 0.0, 0.0];
            }
        }
        let (a, _) = gtao_half_res(&depth, &aligned, &config_8x8()).expect("aligned bakes");
        let (b, _) = gtao_half_res(&depth, &tilted, &config_8x8()).expect("tilted bakes");
        assert_ne!(a.values, b.values, "normals must feed the AO");
        let va = a.texel(2, 1).expect("aligned valley reads");
        let vb = b.texel(2, 1).expect("tilted valley reads");
        assert!(
            vb > va,
            "orthogonal occluders contribute less: aligned {va}, tilted {vb}"
        );
    }

    #[test]
    fn bake_is_deterministic_with_stable_golden_bytes() {
        // Analytic fixture: planar ramp plus a sine ridge, tilted normals.
        // Same complete inputs twice give bit-identical values and bytes.
        let w = 16usize;
        let h = 12usize;
        let config = GtaoConfig {
            width_px: w as u32,
            height_px: h as u32,
            radius_px: 5.0,
            falloff_m: 1.5,
            intensity: 0.9,
        };
        let (depth, normals) = ridge_inputs(w, h);
        let (a, _) = gtao_half_res(&depth, &normals, &config).expect("first bakes");
        let (b, _) = gtao_half_res(&depth, &normals, &config).expect("second bakes");
        assert_eq!(a, b, "bake is bit-identical twice");
        assert_eq!(a.to_bytes(), b.to_bytes(), "golden bytes are stable");
        assert_eq!(a.to_bytes().len(), a.texels() * 4);
        assert_eq!((a.width_half, a.height_half), (8, 6));
        for v in &a.values {
            assert!((0.0..=1.0).contains(v), "AO stays in range, got {v}");
        }
        assert!(matches!(a.texel(8, 0), Err(LookError::Invalid(_))));
    }

    #[test]
    fn upsample_flat_is_identity() {
        // Flat half AO plus flat depth: every range weight is 1, so the
        // upsample reproduces the half value at every full pixel.
        let config = GtaoConfig {
            width_px: 6,
            height_px: 6,
            radius_px: 4.0,
            falloff_m: 2.0,
            intensity: 1.0,
        };
        let half = GtaoAo {
            width_half: 3,
            height_half: 3,
            values: vec![0.35; 9],
        };
        let (depth, _) = flat_inputs(6, 6, 4.0);
        let full = upsample_bilateral(&half, &depth, &config).expect("flat upsamples");
        assert_eq!(full.len(), 36);
        for v in &full {
            assert!(
                (*v - 0.35).abs() < 1e-6,
                "flat upsample reproduces the half value, got {v}"
            );
        }
    }

    #[test]
    fn upsample_respects_the_depth_edge() {
        // Half AO steps 1->0 left to right; full depth steps 2->6 at x=2.
        // Pixel x=1 sits between half texels 0 (AO 1) and 1 (AO 0) on the
        // near side: bilateral weights favor the near anchor, so the result
        // must sit above the plain bilinear midpoint.
        let config = GtaoConfig {
            width_px: 4,
            height_px: 2,
            radius_px: 4.0,
            falloff_m: 1.0,
            intensity: 1.0,
        };
        let half = GtaoAo {
            width_half: 2,
            height_half: 1,
            values: vec![1.0, 0.0],
        };
        let depth = vec![2.0, 2.0, 6.0, 6.0, 2.0, 2.0, 6.0, 6.0];
        let full = upsample_bilateral(&half, &depth, &config).expect("edge upsamples");
        let edge = full[1];
        assert!(
            edge > 0.6,
            "near-side edge pixel favors the near anchor, got {edge}"
        );
        assert_eq!(full[0], 1.0, "even pixels sit on texel centers");
        assert!(
            full[3] < 0.5,
            "far-side edge pixel favors the far anchor, got {}",
            full[3]
        );
        assert!((0.0..=1.0).contains(&edge));
    }

    #[test]
    fn pass_plan_names_graph_order_and_budgets() {
        // The plan is pure data: pass names, resources and transient bytes.
        let plan = gtao_pass_plan(&config_8x8()).expect("8x8 plans");
        assert_eq!(
            (plan.half_pass, plan.upsample_pass),
            (GTAO_HALF_PASS, GTAO_UPSAMPLE_PASS)
        );
        assert_eq!(
            (plan.half_resource, plan.full_resource),
            (GTAO_HALF_RESOURCE, GTAO_FULL_RESOURCE)
        );
        assert_eq!((plan.half_bytes, plan.full_bytes), (16 * 4, 64 * 4));
        // 1080p half transient stays inside the RA2 aliasing budget; the
        // forward pass upsamples from the half buffer in-shader, so no
        // full-res transient is ever planned.
        let arena = GtaoConfig {
            width_px: 1920,
            height_px: 1080,
            radius_px: 8.0,
            falloff_m: 2.0,
            intensity: 1.0,
        };
        let arena_plan = gtao_pass_plan(&arena).expect("arena plans");
        assert_eq!(arena_plan.half_bytes, 960 * 540 * 4);
        assert!(
            arena_plan.half_bytes <= GTAO_HALF_TRANSIENT_BUDGET_BYTES,
            "half transient {} over budget {GTAO_HALF_TRANSIENT_BUDGET_BYTES}",
            arena_plan.half_bytes
        );
        // The plan wires into the frame graph exactly as WIN1 runs it:
        // half writes ao_half, upsample reads it and writes ao_full, the
        // forward pass reads ao_full.
        let mut graph = crate::graph::FrameGraph::new();
        let half = graph.add_pass(
            crate::graph::PassDesc::new(plan.half_pass)
                .output(plan.half_resource)
                .transient(plan.half_resource, plan.half_bytes),
        );
        let up = graph.add_pass(
            crate::graph::PassDesc::new(plan.upsample_pass)
                .input(plan.half_resource)
                .output(plan.full_resource)
                .transient(plan.full_resource, plan.full_bytes),
        );
        let forward = graph.add_pass(
            crate::graph::PassDesc::new("forward")
                .input(plan.full_resource)
                .input("hdr")
                .output("hdr")
                .work(256, 1_800_000),
        );
        graph.add_edge(half, up).expect("dense pass ids");
        graph.add_edge(up, forward).expect("dense pass ids");
        let mut compiled = graph.compile().expect("gtao chain is acyclic");
        assert_eq!(compiled.order(), &[half, up, forward]);
        assert!(
            compiled.barriers().iter().any(|b| {
                b.resource == plan.half_resource
                    && b.kind == crate::graph::BarrierKind::Transition
                    && b.from == half
                    && b.to == up
            }),
            "half -> upsample hands over ao_half"
        );
        let receipt = compiled.execute(&crate::graph::FrameInfo {
            frame_index: 3,
            width_px: 8,
            height_px: 8,
        });
        assert_eq!(receipt.draws, 256, "forward draws survive the chain");
        assert_eq!(receipt.per_pass_cpu_ms.len(), 3);
    }

    #[test]
    fn bad_gtao_inputs_fail_loudly() {
        let (depth, normals) = flat_inputs(8, 8, 5.0);
        let zero = GtaoConfig {
            width_px: 0,
            ..config_8x8()
        };
        assert!(matches!(
            gtao_half_res(&depth, &normals, &zero),
            Err(LookError::Invalid(_))
        ));
        let bad_radius = GtaoConfig {
            radius_px: f32::NAN,
            ..config_8x8()
        };
        assert!(matches!(
            gtao_half_res(&depth, &normals, &bad_radius),
            Err(LookError::Invalid(_))
        ));
        let bad_intensity = GtaoConfig {
            intensity: 2.0,
            ..config_8x8()
        };
        assert!(matches!(
            gtao_half_res(&depth, &normals, &bad_intensity),
            Err(LookError::Invalid(_))
        ));
        assert!(matches!(
            gtao_half_res(&depth[..10], &normals, &config_8x8()),
            Err(LookError::Invalid(_))
        ));
        let mut bad_depth = depth.clone();
        bad_depth[3] = f32::NAN;
        assert!(matches!(
            gtao_half_res(&bad_depth, &normals, &config_8x8()),
            Err(LookError::Invalid(_))
        ));
        let mut zero_depth = depth.clone();
        zero_depth[3] = 0.0;
        assert!(matches!(
            gtao_half_res(&zero_depth, &normals, &config_8x8()),
            Err(LookError::Invalid(_))
        ));
        let mut bad_normal = normals.clone();
        bad_normal[5] = [0.0, 0.0, 0.0];
        assert!(matches!(
            gtao_half_res(&depth, &bad_normal, &config_8x8()),
            Err(LookError::Invalid(_))
        ));
        let (ao, _) = gtao_half_res(&depth, &normals, &config_8x8()).expect("flat bakes");
        assert!(matches!(
            upsample_bilateral(&ao, &depth[..10], &config_8x8()),
            Err(LookError::Invalid(_))
        ));
        let wrong_half = GtaoAo {
            width_half: 9,
            height_half: 9,
            values: vec![1.0; 81],
        };
        assert!(matches!(
            upsample_bilateral(&wrong_half, &depth, &config_8x8()),
            Err(LookError::Invalid(_))
        ));
        assert!(gtao_pass_plan(&zero).is_err(), "plan validates too");
        assert_eq!(
            config_8x8().tap_step_px(),
            2,
            "radius 4 over 3 taps steps 2"
        );
    }

    #[test]
    #[cfg(not(feature = "window"))]
    fn gpu_submit_without_device_refuses_loudly() {
        let (depth, normals) = flat_inputs(16, 12, 5.0);
        let config = GtaoConfig {
            width_px: 16,
            height_px: 12,
            radius_px: 4.0,
            falloff_m: 2.0,
            intensity: 1.0,
        };
        let (ao, _) = gtao_half_res(&depth, &normals, &config).expect("bakes");
        match ao.submit_gpu(&frame()) {
            Err(LookError::NoDevice(detail)) => {
                assert!(!detail.is_empty(), "never silent");
                assert!(detail.contains("RA1"), "names the missing bind: {detail}");
            }
            other => panic!("headless bind must name the missing device, got {other:?}"),
        }
        let bad_frame = LookFrame {
            viewport_px: [0, 12],
            ..frame()
        };
        assert!(
            matches!(ao.submit_gpu(&bad_frame), Err(LookError::Invalid(_))),
            "invalid frames fail validation before the twin refusal"
        );
        let other = LookFrame {
            viewport_px: [8, 8],
            ..frame()
        };
        assert!(
            matches!(ao.submit_gpu(&other), Err(LookError::Invalid(_))),
            "half/frame mismatch fails loudly"
        );
    }

    // ----- Window device tests (behind `window`; skip loudly without one) -----

    /// Device shared with the GX1 suite: one device for the run, so parallel
    /// tests never exhaust adapters. `None` is skip-not-green, never a pass.
    #[cfg(feature = "window")]
    fn gtao_device() -> Option<(
        &'static wgpu::Device,
        &'static wgpu::Queue,
        &'static wgpu::AdapterInfo,
    )> {
        crate::graph::gx1_test_device()
    }

    #[cfg(feature = "window")]
    fn worst_diff(a: &[f32], b: &[f32]) -> f32 {
        a.iter()
            .zip(b.iter())
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max)
    }

    #[test]
    #[cfg(feature = "window")]
    fn gpu_half_and_upsample_match_cpu_within_tolerance() {
        let Some((device, queue, info)) = gtao_device() else {
            eprintln!("SKIPPED gtao parity: no wgpu adapter on this machine");
            return;
        };
        let config = GtaoConfig {
            width_px: 32,
            height_px: 24,
            radius_px: 5.0,
            falloff_m: 1.5,
            intensity: 0.9,
        };
        let frame = LookFrame {
            frame_index: 5,
            viewport_px: [32, 24],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        };
        let (depth, normals) = ridge_inputs(32, 24);
        let (cpu_half, _) = gtao_half_res(&depth, &normals, &config).expect("cpu half bakes");
        let cpu_full = upsample_bilateral(&cpu_half, &depth, &config).expect("cpu upsample bakes");
        let mut submitter = GtaoSubmitter::new(device);
        let receipt = submitter
            .submit(device, queue, &depth, &normals, &config, &frame)
            .expect("device passes run clean");
        assert!(receipt.is_clean(), "0 validation errors");
        let half = submitter.half_ao();
        let full = submitter.full_ao();
        assert_eq!(
            (half.width_half, half.height_half),
            (cpu_half.width_half, cpu_half.height_half)
        );
        assert_eq!(full.len(), cpu_full.len());
        let half_diff = worst_diff(&half.values, &cpu_half.values);
        let full_diff = worst_diff(full, &cpu_full);
        assert!(
            half_diff <= GTAO_GPU_PARITY_TOL,
            "half parity {half_diff} over tol {GTAO_GPU_PARITY_TOL}"
        );
        assert!(
            full_diff <= GTAO_GPU_PARITY_TOL,
            "full parity {full_diff} over tol {GTAO_GPU_PARITY_TOL}"
        );
        assert_eq!(receipt.budget_ms(), GTAO_BUDGET_MS);
        eprintln!(
            "GTAO parity on {}: half_diff={half_diff:.6} full_diff={full_diff:.6} upload_ms={:.2} submit_ms={:.2}",
            info.name, receipt.upload_ms, receipt.submit_ms
        );
    }

    #[test]
    #[cfg(feature = "window")]
    fn gpu_baked_bind_roundtrip_is_exact() {
        let Some((device, queue, _)) = gtao_device() else {
            eprintln!("SKIPPED gtao bind: no wgpu adapter on this machine");
            return;
        };
        let (depth, normals) = flat_inputs(16, 12, 5.0);
        let config = GtaoConfig {
            width_px: 16,
            height_px: 12,
            radius_px: 4.0,
            falloff_m: 2.0,
            intensity: 1.0,
        };
        let (ao, _) = gtao_half_res(&depth, &normals, &config).expect("bakes");
        let receipt = ao
            .submit_gpu(device, queue, &frame())
            .expect("baked bind runs clean");
        assert_eq!(
            (receipt.half_texels, receipt.full_texels),
            (ao.texels() as u64, 16 * 12)
        );
        assert_eq!(receipt.budget_ms(), GTAO_BUDGET_MS);
        // Shape mismatches still fail loudly on the device build.
        let other = LookFrame {
            viewport_px: [8, 8],
            ..frame()
        };
        assert!(matches!(
            ao.submit_gpu(device, queue, &other),
            Err(LookError::Invalid(_))
        ));
    }

    #[test]
    #[cfg(feature = "window")]
    fn gpu_refuses_bad_input_before_sizing() {
        let Some((device, queue, _)) = gtao_device() else {
            eprintln!("SKIPPED gtao refusals: no wgpu adapter on this machine");
            return;
        };
        let config = GtaoConfig {
            width_px: 16,
            height_px: 12,
            radius_px: 4.0,
            falloff_m: 2.0,
            intensity: 1.0,
        };
        let frame = LookFrame {
            frame_index: 5,
            viewport_px: [16, 12],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        };
        let (depth, normals) = flat_inputs(16, 12, 5.0);
        let mut submitter = GtaoSubmitter::new(device);
        assert_eq!(submitter.created_buffers(), 0);
        // Zero viewport, mismatched frame, short depth, bad depth value and
        // degenerate normals all refuse before any buffer is sized.
        let bad_frame = LookFrame {
            viewport_px: [0, 12],
            ..frame
        };
        assert!(matches!(
            submitter.submit(device, queue, &depth, &normals, &config, &bad_frame),
            Err(LookError::Invalid(_))
        ));
        let other_frame = LookFrame {
            viewport_px: [8, 8],
            ..frame
        };
        assert!(matches!(
            submitter.submit(device, queue, &depth, &normals, &config, &other_frame),
            Err(LookError::Invalid(_))
        ));
        assert!(matches!(
            submitter.submit(device, queue, &depth[..10], &normals, &config, &frame),
            Err(LookError::Invalid(_))
        ));
        let mut bad_depth = depth.clone();
        bad_depth[3] = 0.0;
        assert!(matches!(
            submitter.submit(device, queue, &bad_depth, &normals, &config, &frame),
            Err(LookError::Invalid(_))
        ));
        let mut bad_normals = normals.clone();
        bad_normals[5] = [0.0, 0.0, 0.0];
        assert!(matches!(
            submitter.submit(device, queue, &depth, &bad_normals, &config, &frame),
            Err(LookError::Invalid(_))
        ));
        let bad_config = GtaoConfig {
            intensity: 2.0,
            ..config
        };
        assert!(matches!(
            submitter.submit(device, queue, &depth, &normals, &bad_config, &frame),
            Err(LookError::Invalid(_))
        ));
        assert_eq!(
            submitter.created_buffers(),
            0,
            "refused submits size nothing"
        );
    }

    #[test]
    #[cfg(feature = "window")]
    fn gpu_reuses_buffers_on_matching_shape() {
        let Some((device, queue, _)) = gtao_device() else {
            eprintln!("SKIPPED gtao reuse: no wgpu adapter on this machine");
            return;
        };
        let config = GtaoConfig {
            width_px: 32,
            height_px: 24,
            radius_px: 5.0,
            falloff_m: 1.5,
            intensity: 0.9,
        };
        let frame = LookFrame {
            frame_index: 5,
            viewport_px: [32, 24],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        };
        let (depth, normals) = ridge_inputs(32, 24);
        let mut submitter = GtaoSubmitter::new(device);
        let first = submitter
            .submit(device, queue, &depth, &normals, &config, &frame)
            .expect("first submits");
        let shape = submitter.created_buffers();
        assert!(shape > 0, "first submit sizes the buffers");
        let first_half = submitter.half_ao().values.clone();
        let first_full = submitter.full_ao().to_vec();
        let second = submitter
            .submit(
                device,
                queue,
                &depth,
                &normals,
                &config,
                &LookFrame {
                    frame_index: 6,
                    ..frame
                },
            )
            .expect("second submits");
        assert_eq!(
            submitter.created_buffers(),
            shape,
            "steady-state submit creates no new buffers"
        );
        assert!(first.is_clean() && second.is_clean());
        assert_eq!(first_half, submitter.half_ao().values);
        assert_eq!(first_full, submitter.full_ao());
    }

    #[test]
    #[cfg(feature = "window")]
    fn gpu_steady_state_grows_no_host_scratch() {
        // Steady-state submits grow nothing on the host: every scratch
        // capacity stays flat across submits on a matching shape, and the
        // reused outputs still match the CPU oracle within the posted
        // parity tolerance. Sizing happens once, on the first submit;
        // refusals size nothing (see the refusal test).
        let Some((device, queue, _)) = gtao_device() else {
            eprintln!("SKIPPED gtao scratch: no wgpu adapter on this machine");
            return;
        };
        let config = GtaoConfig {
            width_px: 32,
            height_px: 24,
            radius_px: 5.0,
            falloff_m: 1.5,
            intensity: 0.9,
        };
        let frame = LookFrame {
            frame_index: 5,
            viewport_px: [32, 24],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        };
        let (depth, normals) = ridge_inputs(32, 24);
        let (cpu_half, _) = gtao_half_res(&depth, &normals, &config).expect("cpu half bakes");
        let cpu_full = upsample_bilateral(&cpu_half, &depth, &config).expect("cpu upsample bakes");
        let mut submitter = GtaoSubmitter::new(device);
        submitter
            .submit(device, queue, &depth, &normals, &config, &frame)
            .expect("first submits");
        let caps = submitter.scratch_capacities();
        assert!(
            caps.0 == 32 * 24 * 4 && caps.1 == 32 * 24 * 16,
            "first submit sizes staging exactly, got {caps:?}"
        );
        for index in 6..9 {
            submitter
                .submit(
                    device,
                    queue,
                    &depth,
                    &normals,
                    &config,
                    &LookFrame {
                        frame_index: index,
                        ..frame
                    },
                )
                .expect("steady state submits");
            assert_eq!(
                submitter.scratch_capacities(),
                caps,
                "steady-state submit grows no host scratch"
            );
        }
        // Reuse keeps the oracle agreement: never bit-exact, because the
        // device f32 passes may round the last ulp differently than the CPU
        // oracle (observed 0.7119923 vs 0.7119924, about two ulps at 0.71),
        // so steady-state outputs share the `GTAO_GPU_PARITY_TOL` worst-diff
        // bound the parity test posts. Flat scratch capacities above prove
        // reuse sizes nothing; this proves reuse drifts nothing.
        let half_diff = worst_diff(&submitter.half_ao().values, &cpu_half.values);
        let full_diff = worst_diff(submitter.full_ao(), &cpu_full);
        assert!(
            half_diff <= GTAO_GPU_PARITY_TOL,
            "reused half {half_diff} over tol {GTAO_GPU_PARITY_TOL}"
        );
        assert!(
            full_diff <= GTAO_GPU_PARITY_TOL,
            "reused full {full_diff} over tol {GTAO_GPU_PARITY_TOL}"
        );
    }

    #[test]
    #[cfg(feature = "window")]
    fn look_uniform_matches_graph_packing() {
        // One writer for the 32-byte look image: the graph packer. The GTAO
        // passes bind the same bytes, so any drift fails here, not on device.
        let bytes = crate::graph::look_frame_uniform_bytes(&frame());
        assert_eq!(bytes.len(), 32);
        assert_eq!(&bytes[0..4], &(7u32.to_le_bytes()));
        assert_eq!(&bytes[4..8], &(16u32.to_le_bytes()));
        assert_eq!(&bytes[8..12], &(12u32.to_le_bytes()));
        assert_eq!(&bytes[12..16], &10.0f32.to_le_bytes());
    }

    #[test]
    #[cfg(feature = "window")]
    fn gpu_receipt_budget_accessors_are_exact() {
        // Deterministic budget logic (no wall clock): at, under and over
        // the posted budget plus the validation flag.
        let at = GtaoGpuReceipt {
            half_texels: 16,
            full_texels: 64,
            validation_errors: 0,
            upload_ms: 0.1,
            submit_ms: GTAO_BUDGET_MS,
        };
        assert_eq!(at.budget_ms(), GTAO_BUDGET_MS);
        assert!(at.within_budget(), "exactly at budget is within");
        assert!(at.is_clean());
        let over = GtaoGpuReceipt {
            submit_ms: GTAO_BUDGET_MS + 0.5,
            validation_errors: 1,
            ..at
        };
        assert!(!over.within_budget(), "over budget fails loudly");
        assert!(!over.is_clean());
    }

    #[test]
    #[cfg(feature = "window")]
    #[ignore = "timing: 1080p gtao budget probe, run under --perf"]
    fn gpu_1080p_budget_probe_reports_ms() {
        let Some((device, queue, info)) = gtao_device() else {
            eprintln!("SKIPPED gtao 1080p probe: no wgpu adapter on this machine");
            return;
        };
        // Flat 1080p fixture: every tap agrees, so parity is exact 1.0 and
        // the receipt reports the arena-size encode + submit + readback ms
        // the 0.8 ms budget is charged against (PF2 enforces on release).
        let config = GtaoConfig {
            width_px: 1920,
            height_px: 1080,
            radius_px: 8.0,
            falloff_m: 2.0,
            intensity: 1.0,
        };
        let frame = LookFrame {
            frame_index: 1,
            viewport_px: [1920, 1080],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        };
        let (depth, normals) = flat_inputs(1920, 1080, 5.0);
        let mut submitter = GtaoSubmitter::new(device);
        let receipt = submitter
            .submit(device, queue, &depth, &normals, &config, &frame)
            .expect("1080p submits");
        assert!(receipt.is_clean(), "0 validation errors at 1080p");
        for v in submitter.half_ao().values.iter() {
            assert_eq!(*v, 1.0, "flat plane stays unoccluded at 1080p");
        }
        eprintln!(
            "GTAO 1080p probe on {}: upload_ms={:.2} submit_ms={:.2} budget_ms={} (oracle path stages full depth+normals from the host; the budget row judges device execution)",
            info.name,
            receipt.upload_ms,
            receipt.submit_ms,
            receipt.budget_ms()
        );
    }
}
