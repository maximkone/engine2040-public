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

#[cfg(feature = "gpu")]
use crate::decals::{compute_storage_entry, compute_uniform_entry};

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
/// Tap depth (m) at or beyond which a GTAO tap is background, never an
/// occluder: both cameras clip past it, so a cleared (far-plane) texel
/// decodes at or past it. Never hashed. Caller: `gtao_half_res` (same
/// file) and `GTAO_HALF_WGSL` gate their tap term with the same figure.
pub const GTAO_BACKGROUND_M: f32 = 100.0;
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

/// Live half switch: on (default) routes the live upsample through half
/// size, off opts out to full neutral. Owner render, consumer the live bind
/// below. Units boolean flag. Determinism flag only, no wall clock, no
/// allocation. Presentation only, never hashed.
/// Caller: `gtao_live_full_for_forward` (same file) wires the live bind
/// through here.
static GTAO_LIVE_HALF_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

/// Read the live half switch: true routes the live upsample through half
/// size (default), false opts out to full. Units boolean out. Determinism
/// flag only. Presentation only, never hashed.
pub fn gtao_live_half_enabled() -> bool {
    GTAO_LIVE_HALF_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Set the live half switch: true halves the live upsample (default),
/// false opts out to full. Units boolean in. Determinism flag only.
/// Presentation only, never hashed.
pub fn set_gtao_live_half_enabled(enabled: bool) {
    GTAO_LIVE_HALF_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Live half size for a full frame: half (ceil) when the switch is on,
/// full when it is off. Pure, no allocation. Zero area passes through for
/// the suspend path. Presentation only, never hashed.
#[must_use]
pub fn gtao_live_half_size(width_px: u32, height_px: u32) -> [u32; 2] {
    if !gtao_live_half_enabled() {
        return [width_px, height_px];
    }
    if width_px == 0 || height_px == 0 {
        return [width_px, height_px];
    }
    [
        width_px.div_ceil(GTAO_HALF_DIV).max(1),
        height_px.div_ceil(GTAO_HALF_DIV).max(1),
    ]
}

/// Live upsample through half size: half on (default) lifts the half buffer
/// with the bilateral snap plus range weights (`upsample_bilateral`); full
/// opt-out keeps the probe-only neutral (`GTAO_OPEN`). Units pixels for
/// sizes, meters for depth. Determinism same slice plus flag gives same
/// field. Errors [`LookError::Invalid`] on bad shapes or feeds.
/// Caller: `gtao_live_full_for_forward` (same file) wires the live bind
/// through here.
pub fn gtao_live_upsample_for_forward(
    half: &GtaoAo,
    depth_full: &[f32],
    config: &GtaoConfig,
) -> Result<Vec<f32>, LookError> {
    if !gtao_live_half_enabled() {
        check_depth_buffer(depth_full, config.full_texels(), "upsample depth")?;
        return Ok(vec![GTAO_OPEN; config.full_texels()]);
    }
    let want = gtao_live_half_size(config.width_px, config.height_px);
    let want_w = usize::try_from(want[0])
        .map_err(|_| LookError::Invalid(format!("live half width {} exceeds usize", want[0])))?;
    let want_h = usize::try_from(want[1])
        .map_err(|_| LookError::Invalid(format!("live half height {} exceeds usize", want[1])))?;
    if half.width_half != want_w || half.height_half != want_h {
        return Err(LookError::Invalid(format!(
            "live half {}x{} mismatches live size {}x{}",
            half.width_half, half.height_half, want[0], want[1]
        )));
    }
    upsample_bilateral(half, depth_full, config)
}

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
#[cfg(not(feature = "gpu"))]
pub fn bind_look(frame: &LookFrame) -> Result<(), LookError> {
    frame.validate()?;
    Err(LookError::NoDevice(
        "look GPU bind needs the RA1 forward pass bind group; the CPU bake is the reference".into(),
    ))
}

/// Shared bind shape for every look pass: validates the frame. The device
/// submitter writes the packed look uniform and sets the bind group per
/// pass, so validation here is the bind contract on this build.
#[cfg(feature = "gpu")]
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
        crate::accel::f32_le_bytes(&self.values)
    }

    /// Bind seam for the RA1 forward AO bind on the headless twin: validates
    /// the frame and the half shape, then names the missing device bind.
    /// The window build next to it binds the baked buffer for real.
    #[cfg(not(feature = "gpu"))]
    pub fn submit_gpu(&self, frame: &LookFrame) -> Result<AoReceipt, LookError> {
        frame.validate()?;
        let half = frame.half_size();
        if half[0] as usize != self.width_half || half[1] as usize != self.height_half {
            let (want_w, want_h) = (self.width_half, self.height_half);
            let (fw, fh) = (half[0], half[1]);
            let reason = format!("gtao half {want_w}x{want_h} mismatches frame half {fw}x{fh}"); // alloc-ok: cold refusal, never a steady frame
            return Err(LookError::Invalid(reason));
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
    #[cfg(feature = "gpu")]
    pub fn submit_gpu(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &LookFrame,
    ) -> Result<AoReceipt, LookError> {
        frame.validate()?;
        let half = frame.half_size();
        if half[0] as usize != self.width_half || half[1] as usize != self.height_half {
            let (want_w, want_h) = (self.width_half, self.height_half);
            let (fw, fh) = (half[0], half[1]);
            let reason = format!("gtao half {want_w}x{want_h} mismatches frame half {fw}x{fh}"); // alloc-ok: cold refusal, never a steady frame
            return Err(LookError::Invalid(reason));
        }
        let texels = self.texels();
        let count = u32::try_from(texels)
            .map_err(|_| LookError::Invalid(format!("gtao texels {texels} exceed u32")))?; // alloc-ok: cold refusal, never a steady frame
        let byte_len = texels as u64 * GTAO_AO_BYTES_PER_TEXEL;
        let started = Instant::now();
        let probe = build_bind_probe(device, byte_len);
        queue.write_buffer(
            &probe.look_buf,
            0,
            &crate::graph::look_frame_uniform_bytes(frame),
        );
        let mut src_bytes = Vec::with_capacity(texels * 4); // alloc-ok: submit_gpu probe staging with readback, not the window steady frame
        for v in &self.values {
            src_bytes.extend_from_slice(&v.to_le_bytes());
        }
        queue.write_buffer(&probe.src_buf, 0, &src_bytes);

        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gtao-ao-bind"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gtao-ao-bind-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&probe.pipeline);
            pass.set_bind_group(0, &probe.bind, &[]);
            pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&probe.dst_buf, 0, &probe.readback, 0, byte_len);
        queue.submit(Some(encoder.finish()));
        map_and_wait(device, &probe.readback)?;
        let back = readback_f32(&probe.readback, texels)?;
        probe.readback.unmap();
        if let Some(err) = pollster::block_on(scope.pop()) {
            let reason = format!("gtao bind validation: {err:?}"); // alloc-ok: cold device-loss
            return Err(LookError::Device(reason));
        }
        for (i, (got, want)) in back.iter().zip(self.values.iter()).enumerate() {
            if got.to_bits() != want.to_bits() {
                let reason = format!("gtao bind roundtrip texel {i} differs: {got} vs {want}"); // alloc-ok: cold device-loss path, never a steady frame
                return Err(LookError::Device(reason));
            }
        }
        Ok(AoReceipt {
            half_texels: texels as u64,
            full_texels: u64::from(frame.viewport_px[0]) * u64::from(frame.viewport_px[1]),
            ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}

/// Open-field AO: the blit composite neutral (`ao_tex` stays 1.0 live).
/// CPU mirror only; the bake below must darken the valley against this.
pub const GTAO_OPEN: f32 = 1.0;

/// Read valley and open probes from a full-res AO field. Pixels for sizes
/// and coords, row-major `width * height` values in `0..=1`. Pure, std-only;
/// caller owns `full`, this sizes nothing. Errors: [`LookError::Invalid`]
/// on zero/oversize dims, length mismatch or out-of-range coords.
/// Determinism: same slice plus coords give the same pair.
pub fn ao_on_off_stats(
    full: &[f32],
    width_px: u32,
    height_px: u32,
    valley_xy: (usize, usize),
    open_xy: (usize, usize),
) -> Result<(f32, f32), LookError> {
    if width_px == 0 || height_px == 0 {
        return Err(LookError::Invalid(format!(
            "gtao on/off frame {width_px}x{height_px} is zero"
        )));
    }
    if width_px > MAX_LOOK_DIM || height_px > MAX_LOOK_DIM {
        return Err(LookError::Invalid(format!(
            "gtao on/off frame {width_px}x{height_px} exceeds {MAX_LOOK_DIM}"
        )));
    }
    let width = usize::try_from(width_px)
        .map_err(|_| LookError::Invalid(format!("gtao on/off width {width_px} exceeds usize")))?;
    let height = usize::try_from(height_px)
        .map_err(|_| LookError::Invalid(format!("gtao on/off height {height_px} exceeds usize")))?;
    let want = width * height;
    if full.len() != want {
        return Err(LookError::Invalid(format!(
            "gtao on/off length {} mismatches frame {want}",
            full.len()
        )));
    }
    if valley_xy.0 >= width || valley_xy.1 >= height {
        return Err(LookError::Invalid(format!(
            "gtao valley ({},{}) outside {width_px}x{height_px}",
            valley_xy.0, valley_xy.1
        )));
    }
    if open_xy.0 >= width || open_xy.1 >= height {
        return Err(LookError::Invalid(format!(
            "gtao open ({},{}) outside {width_px}x{height_px}",
            open_xy.0, open_xy.1
        )));
    }
    Ok((
        full[valley_xy.1 * width + valley_xy.0],
        full[open_xy.1 * width + open_xy.0],
    ))
}

/// Guard the live blit upload: the full-res AO field behind `ao_tex` must
/// match the frame and hold finite `0..=1` occlusion. Pixels for dims,
/// row-major `width * height` values. Pure, std-only; caller owns `full`.
/// Errors: [`LookError::Invalid`] on zero/oversize dims, length mismatch,
/// non-finite or out-of-range texels. Called by the live fog tranche before
/// the `ao_texture` queue write; tests own the step-bake acceptance plus
/// the loud refusals. Determinism: same slice plus dims give the same verdict.
pub fn check_ao_full_for_blit(
    full: &[f32],
    width_px: u32,
    height_px: u32,
) -> Result<(), LookError> {
    if width_px == 0 || height_px == 0 {
        return Err(LookError::Invalid(format!(
            "gtao blit AO frame {width_px}x{height_px} is zero"
        )));
    }
    if width_px > MAX_LOOK_DIM || height_px > MAX_LOOK_DIM {
        return Err(LookError::Invalid(format!(
            "gtao blit AO frame {width_px}x{height_px} exceeds {MAX_LOOK_DIM}"
        )));
    }
    let width = usize::try_from(width_px)
        .map_err(|_| LookError::Invalid(format!("gtao blit AO width {width_px} exceeds usize")))?;
    let height = usize::try_from(height_px).map_err(|_| {
        LookError::Invalid(format!("gtao blit AO height {height_px} exceeds usize"))
    })?;
    let want = width * height;
    if full.len() != want {
        return Err(LookError::Invalid(format!(
            "gtao blit AO length {} mismatches frame {want}",
            full.len()
        )));
    }
    for (i, v) in full.iter().enumerate() {
        if !v.is_finite() {
            return Err(LookError::Invalid(format!(
                "gtao blit AO[{i}] value {v} is not finite"
            )));
        }
        if !(0.0..=1.0).contains(v) {
            return Err(LookError::Invalid(format!(
                "gtao blit AO[{i}] value {v} escapes 0..=1"
            )));
        }
    }
    Ok(())
}

/// Stale AO reuse gate for the live frame: true only when the cached
/// full-res field behind `ao_tex` still matches the frame and holds finite
/// `0..=1` occlusion. Pixels for dims, row-major `width * height` values.
/// Pure, std-only; caller owns `full`. Determinism same slice plus dims give
/// same verdict. No allocation, no device, no map, no poll.
/// Caller: `crates/render/src/present/gpu_present/live.rs` `present_frame_live`
/// reuses the last probe tranche through here before the `ao_texture` write.
#[must_use]
pub fn stale_ao_should_reuse(full: &[f32], width_px: u32, height_px: u32) -> bool {
    check_ao_full_for_blit(full, width_px, height_px).is_ok()
}

/// Live GTAO bind for one forward-stored depth feed: half-res oracle plus
/// the bilateral upsample into the full-res field the blit `ao_tex` samples.
/// Units: pixels in `config`, meters for depth, ms out (evidence only, never
/// hashed). Determinism: row-major integer order with f64 accumulation in the
/// oracle, so the same feed gives the same field. Errors: [`LookError::Invalid`]
/// on bad sizes or feed values; empty or zero-size is skip (caller keeps the
/// neutral 1.0, no extra work). CPU-only; no WGSL, no map, no poll.
/// Caller: `crates/render/src/present/gpu_present/live.rs` `present_frame_live`.
pub fn gtao_live_full_for_forward(
    depth_full: &[f32],
    normals_full: &[[f32; 3]],
    config: &GtaoConfig,
) -> Result<(Vec<f32>, f64), LookError> {
    let started = Instant::now();
    let (half, _) = gtao_half_res(depth_full, normals_full, config)?;
    let full = gtao_live_upsample_for_forward(&half, depth_full, config)?;
    check_ao_full_for_blit(&full, config.width_px, config.height_px)?;
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok((full, ms))
}

/// Occluder elevation floor of the live kernel, as a sine over the tangent
/// plane: a neighbor whose direction sits below it never occludes, so open
/// flat ground reads exactly 1.0 at any view angle (a depth-difference
/// kernel darkens a floor seen at a grazing angle against itself).
pub const GTAO_LIVE_SIN_BIAS: f32 = 0.1;

/// Taps per slice side as the signed step count the live kernel loops over.
const GTAO_LIVE_TAPS: i32 = 3;
const _: () = assert!(GTAO_TAPS == 3, "the live loop counts the same taps");

/// The one narrowing of the live oracle's f64 accumulation to the f32 field
/// it stores.
fn narrow(v: f64) -> f32 {
    v as f32
}

/// The occlusion normalizer as the f32 the live device uniform carries.
#[cfg(feature = "gpu")]
pub(crate) fn gtao_weight_sum_f32() -> f32 {
    narrow(gtao_weight_sum())
}

/// A pixel coordinate as an index; the validated frame bound keeps it small.
fn index_of(v: u32) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// Unproject every depth texel to a world point (f64, row-major). Errors on a
/// depth outside `0..=1` or a degenerate unprojection. Used by the live
/// oracle only; the device shaders redo this per tap from the depth texture.
fn live_world_points(
    raw_depth: &[f32],
    config: &GtaoConfig,
    inv_cols: [[f32; 4]; 4],
) -> Result<Vec<[f64; 3]>, LookError> {
    let (w, h) = (config.width_px, config.height_px);
    let mut world = vec![[0.0f64; 3]; index_of(w) * index_of(h)];
    for y in 0..h {
        for x in 0..w {
            let i = index_of(y) * index_of(w) + index_of(x);
            let d = raw_depth[i];
            if !d.is_finite() || !(0.0..=1.0).contains(&d) {
                return Err(LookError::Invalid(format!(
                    "gtao live depth[{i}] {d} outside 0..=1"
                )));
            }
            let clip = [
                2.0 * (f64::from(x) + 0.5) / f64::from(w) - 1.0,
                1.0 - 2.0 * (f64::from(y) + 0.5) / f64::from(h),
                f64::from(d),
                1.0,
            ];
            let mut q = [0.0f64; 4];
            for (r, slot) in q.iter_mut().enumerate() {
                *slot = (0..4).map(|c| f64::from(inv_cols[c][r]) * clip[c]).sum();
            }
            if !q.iter().all(|v| v.is_finite()) || q[3] <= 0.0 {
                return Err(LookError::Invalid(format!(
                    "gtao live depth[{i}] unprojects degenerately"
                )));
            }
            world[i] = [q[0] / q[3], q[1] / q[3], q[2] / q[3]];
        }
    }
    Ok(world)
}

/// Facing normal of texel `(x, y)` from central differences of the world
/// grid (neighbors clamped at the frame edge), oriented toward the eye; a
/// degenerate neighborhood falls back to the view ray.
fn live_facing_normal(
    world: &[[f64; 3]],
    (w, h): (u32, u32),
    (x, y): (u32, u32),
    eye: [f64; 3],
) -> [f64; 3] {
    let at = |px: u32, py: u32| world[index_of(py) * index_of(w) + index_of(px)];
    let (xl, xr) = (x.saturating_sub(1), (x + 1).min(w - 1));
    let (yt, yb) = (y.saturating_sub(1), (y + 1).min(h - 1));
    let (a, b, c, e) = (at(xr, y), at(xl, y), at(x, yb), at(x, yt));
    let tx = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let ty = [c[0] - e[0], c[1] - e[1], c[2] - e[2]];
    let mut n = [
        tx[1] * ty[2] - tx[2] * ty[1],
        tx[2] * ty[0] - tx[0] * ty[2],
        tx[0] * ty[1] - tx[1] * ty[0],
    ];
    let here = at(x, y);
    let to_eye = [eye[0] - here[0], eye[1] - here[1], eye[2] - here[2]];
    if n[0] * to_eye[0] + n[1] * to_eye[1] + n[2] * to_eye[2] < 0.0 {
        n = [-n[0], -n[1], -n[2]];
    }
    let len2 = n[0] * n[0] + n[1] * n[1] + n[2] * n[2];
    let (v, len) = if len2 < 1e-24 {
        (
            to_eye,
            (to_eye[0] * to_eye[0] + to_eye[1] * to_eye[1] + to_eye[2] * to_eye[2]).sqrt(),
        )
    } else {
        (n, len2.sqrt())
    };
    [v[0] / len, v[1] / len, v[2] / len]
}

/// CPU oracle of the live GTAO frame: the plane-aware kernel the device runs
/// on the forward-stored depth. Half-res centers sit on the top-left pixel of
/// each 2x2 block; four slices times three taps times two sides sample the
/// world point of each tap and count it as an occluder by its elevation over
/// the center's tangent plane (sine above [`GTAO_LIVE_SIN_BIAS`], scaled to
/// 0..1) faded linearly to nothing at `falloff_m` of world distance; sky
/// texels (depth exactly 1.0) never occlude and read 1.0. The half result is
/// upsampled with [`upsample_bilateral`] over eye distance. Units: depth is
/// raw `0..=1` forward depth, `inv_view_proj_cols` the column-major inverse
/// of the frame's view-projection, `eye` and falloff in meters. Determinism:
/// fixed integer order, f64 accumulation. Errors: [`LookError::Invalid`] on a
/// bad config, a length mismatch or depth outside `0..=1`.
pub fn gtao_live_oracle(
    raw_depth: &[f32],
    inv_view_proj_cols: [[f32; 4]; 4],
    eye: [f32; 3],
    config: &GtaoConfig,
) -> Result<Vec<f32>, LookError> {
    config.validate()?;
    let (w, h) = (config.width_px, config.height_px);
    let wu = index_of(w);
    if raw_depth.len() != wu * index_of(h) {
        return Err(LookError::Invalid(format!(
            "gtao live depth length {} mismatches frame {}",
            raw_depth.len(),
            wu * index_of(h)
        )));
    }
    let world = live_world_points(raw_depth, config, inv_view_proj_cols)?;
    let eye64 = eye.map(f64::from);
    let dist: Vec<f32> = world
        .iter()
        .map(|p| {
            let d = [p[0] - eye64[0], p[1] - eye64[1], p[2] - eye64[2]];
            narrow((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt())
        })
        .collect();
    let (hw, hh) = (w.div_ceil(GTAO_HALF_DIV), h.div_ceil(GTAO_HALF_DIV));
    let (wi, hi) = (
        i32::try_from(w).unwrap_or(i32::MAX),
        i32::try_from(h).unwrap_or(i32::MAX),
    );
    let step = config.tap_step_px();
    let falloff = f64::from(config.falloff_m);
    let bias = f64::from(GTAO_LIVE_SIN_BIAS);
    let weight_sum = gtao_weight_sum();
    let mut values = vec![1.0f32; index_of(hw) * index_of(hh)];
    for hy in 0..hh {
        for hx in 0..hw {
            let cx = (hx * GTAO_HALF_DIV).min(w - 1);
            let cy = (hy * GTAO_HALF_DIV).min(h - 1);
            let ci = index_of(cy) * wu + index_of(cx);
            if raw_depth[ci] >= 1.0 {
                continue;
            }
            let pc = world[ci];
            let nc = live_facing_normal(&world, (w, h), (cx, cy), eye64);
            let (cxi, cyi) = (
                i32::try_from(cx).unwrap_or(0),
                i32::try_from(cy).unwrap_or(0),
            );
            let mut occ = 0.0f64;
            for (dx, dy) in SLICE_DIRS {
                for t in 1..=GTAO_LIVE_TAPS {
                    let off = t * step;
                    let w_tap = 1.0 / (1.0 + f64::from(t));
                    for side in [1i32, -1i32] {
                        let nx = (cxi + dx * off * side).clamp(0, wi - 1);
                        let ny = (cyi + dy * off * side).clamp(0, hi - 1);
                        let ni = index_of(u32::try_from(ny).unwrap_or(0)) * wu
                            + index_of(u32::try_from(nx).unwrap_or(0));
                        if raw_depth[ni] >= 1.0 {
                            continue;
                        }
                        let p = world[ni];
                        let v = [p[0] - pc[0], p[1] - pc[1], p[2] - pc[2]];
                        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                        if len < 1e-5 {
                            continue;
                        }
                        let sine = (v[0] * nc[0] + v[1] * nc[1] + v[2] * nc[2]) / len;
                        let elevation = ((sine - bias) / (1.0 - bias)).clamp(0.0, 1.0);
                        let fade = (1.0 - len / falloff).clamp(0.0, 1.0);
                        occ += elevation * fade * w_tap;
                    }
                }
            }
            let ao = 1.0 - f64::from(config.intensity) * occ / weight_sum;
            values[index_of(hy) * index_of(hw) + index_of(hx)] = narrow(ao.clamp(0.0, 1.0));
        }
    }
    let half = GtaoAo {
        width_half: index_of(hw),
        height_half: index_of(hh),
        values,
    };
    let mut full = upsample_bilateral(&half, &dist, config)?;
    for (ao, d) in full.iter_mut().zip(raw_depth) {
        if *d >= 1.0 {
            *ao = 1.0;
        }
    }
    Ok(full)
}

/// Device resources for the baked-AO bind probe: one compute pipeline plus
/// its four buffers and bind group. Built per `submit_gpu` call; the caller
/// owns the resources for exactly one probe dispatch.
#[cfg(feature = "gpu")]
struct BindProbe {
    pipeline: wgpu::ComputePipeline,
    look_buf: wgpu::Buffer,
    src_buf: wgpu::Buffer,
    dst_buf: wgpu::Buffer,
    readback: wgpu::Buffer,
    bind: wgpu::BindGroup,
}

/// Build the bind-probe pipeline, buffers and bind group for `byte_len`
/// bytes of half AO. Same labels, layouts and usages the inline submitter
/// used; extracted so `submit_gpu` stays under the line budget. No
/// validation here; the caller validated shapes before sizing anything.
#[cfg(feature = "gpu")]
fn build_bind_probe(device: &wgpu::Device, byte_len: u64) -> BindProbe {
    use crate::pipeline_warmup::CountedPipelines as _;
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("gtao-ao-bind"),
        source: wgpu::ShaderSource::Wgsl(GTAO_BIND_WGSL.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("gtao-ao-bind-layout"),
        entries: &[
            compute_uniform_entry(0),
            compute_storage_entry(1, true),
            compute_storage_entry(2, false),
        ],
    });
    let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("gtao-ao-bind-pipe-layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline_counted(&wgpu::ComputePipelineDescriptor {
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
    BindProbe {
        pipeline,
        look_buf,
        src_buf,
        dst_buf,
        readback,
        bind,
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
                        if nd >= f64::from(GTAO_BACKGROUND_M) {
                            continue;
                        }
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
    for (i, v) in half.values.iter().enumerate() {
        if !v.is_finite() {
            return Err(LookError::Invalid(format!(
                "gtao half AO[{i}] value {v} is not finite"
            )));
        }
        if !(0.0..=1.0).contains(v) {
            return Err(LookError::Invalid(format!(
                "gtao half AO[{i}] value {v} escapes 0..=1"
            )));
        }
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
#[cfg(feature = "gpu")]
pub const GTAO_CFG_UNIFORM_BYTES: u64 = 40;

/// Half-res GTAO compute: one thread per half texel, fixed slice taps in
/// shader order mirroring the CPU oracle. The shared look uniform stays
/// bound and live without touching AO math.
#[cfg(feature = "gpu")]
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
// Cleared far-plane texels are background and never occlude (GTAO_BACKGROUND_M).
const GATE_M: f32 = 100.0;
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
                if (nd >= GATE_M) { continue; }
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
#[cfg(feature = "gpu")]
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
#[cfg(feature = "gpu")]
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

/// Pack the kernel uniform: full/half sizes, tap step, falloff, intensity
/// and the occlusion normalizer. Pure bytes; validation stays on the caller.
#[cfg(feature = "gpu")]
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
#[cfg(feature = "gpu")]
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
#[cfg(feature = "gpu")]
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
#[cfg(feature = "gpu")]
fn readback_f32(buffer: &wgpu::Buffer, count: usize) -> Result<Vec<f32>, LookError> {
    let mut out = vec![0.0f32; count];
    readback_f32_into(buffer, &mut out)?;
    Ok(out)
}

/// Validate config, frame and every input value before the submitter sizes
/// anything, so a refusal sizes nothing. No allocation: values are checked
/// in place; normalization stays in the fill step.
#[cfg(feature = "gpu")]
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
#[cfg(feature = "gpu")]
fn frame_config_mismatch(frame: &LookFrame, config: &GtaoConfig) -> LookError {
    LookError::Invalid(format!(
        "gtao frame {}x{} mismatches config {}x{}",
        frame.viewport_px[0], frame.viewport_px[1], config.width_px, config.height_px
    ))
}

/// Refusal for sized buffers that vanished mid-submit. Cold path only.
#[cfg(feature = "gpu")]
fn lost_buffers() -> LookError {
    LookError::Device("gtao submitter lost its sized buffers".into())
}

/// Device refusal text for a validation-scope hit. Cold path only.
#[cfg(feature = "gpu")]
fn validation_refusal(err: wgpu::Error) -> LookError {
    LookError::Device(format!("gtao submit validation: {err:?}"))
}

/// Normalize validated directions into caller-owned storage, growing it
/// only when the shape outgrows the current capacity. Values passed
/// [`validate_normals`], so `None` is unreachable; it still fails loudly
/// through a typed error instead of panicking. `pub(crate)` for the
/// steady-state allocation probe.
#[cfg(feature = "gpu")]
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
#[cfg(feature = "gpu")]
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
#[cfg(feature = "gpu")]
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
#[cfg(feature = "gpu")]
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

#[cfg(feature = "gpu")]
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
#[cfg(feature = "gpu")]
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

#[cfg(feature = "gpu")]
impl GtaoSubmitter {
    /// Compile both compute pipelines and the kernel plus look bind layouts;
    /// allocate the two uniform buffers. No frame-size state yet: storage
    /// buffers size on the first submit and resize only on shape change.
    pub fn new(device: &wgpu::Device) -> Self {
        use crate::pipeline_warmup::CountedPipelines as _;
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
                compute_uniform_entry(0),
                compute_uniform_entry(1),
                compute_storage_entry(2, true),
                compute_storage_entry(3, true),
                compute_storage_entry(4, false),
            ],
        });
        let up_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gtao-upsample-layout"),
            entries: &[
                compute_uniform_entry(0),
                compute_uniform_entry(1),
                compute_storage_entry(2, true),
                compute_storage_entry(3, true),
                compute_storage_entry(4, false),
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
        let half_pipeline =
            device.create_compute_pipeline_counted(&wgpu::ComputePipelineDescriptor {
                label: Some("gtao-half-pipeline"),
                layout: Some(&half_pipe_layout),
                module: &half_shader,
                entry_point: Some("half_main"),
                compilation_options: Default::default(),
                cache: None,
            });
        let up_pipeline =
            device.create_compute_pipeline_counted(&wgpu::ComputePipelineDescriptor {
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
        #[cfg(not(feature = "gpu"))]
        bind_look(&f).expect_err("headless twin names the missing device bind");
        #[cfg(feature = "gpu")]
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
        // Live half (default on) routes through half size and keeps the
        // flat identity; full opt-out keeps the probe-only neutral.
        let was_live_half = gtao_live_half_enabled();
        set_gtao_live_half_enabled(true);
        assert!(gtao_live_half_enabled(), "live half reads on");
        assert_eq!(
            gtao_live_half_size(6, 6),
            [3, 3],
            "half halves the live size"
        );
        let live =
            gtao_live_upsample_for_forward(&half, &depth, &config).expect("live half upsamples");
        for v in &live {
            assert!((*v - 0.35).abs() < 1e-6, "live half keeps flat, got {v}");
        }
        set_gtao_live_half_enabled(false);
        assert!(!gtao_live_half_enabled(), "full opt-out reads back");
        assert_eq!(gtao_live_half_size(6, 6), [6, 6], "full keeps full size");
        let full_opt =
            gtao_live_upsample_for_forward(&half, &depth, &config).expect("full opts out");
        assert!(
            full_opt.iter().all(|v| *v == GTAO_OPEN),
            "full opt-out stays neutral"
        );
        set_gtao_live_half_enabled(true);
        assert_eq!(
            gtao_live_upsample_for_forward(&half, &depth, &config).expect("half restores"),
            live,
            "toggle restores half"
        );
        set_gtao_live_half_enabled(was_live_half);
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
    fn upsample_rejects_bad_half_texels() {
        // Caller: `gtao_live_full_for_forward` (same file) plus the live
        // frame fresh path (`present/gpu_present/live.rs`
        // `live_ao_upload_for_frame`) through it into the blit `ao_tex`.
        // Behavior: a half buffer carrying a non-finite or out-of-range
        // texel refuses loudly so the caller keeps `ao_tex` at 1.0 instead
        // of uploading NaN; a valid half still upsamples to a finite
        // 0..=1 field. Same bad half gives the same refusal twice.
        let (depth, normals) = flat_inputs(8, 8, 5.0);
        let (good, _) = gtao_half_res(&depth, &normals, &config_8x8()).expect("flat bakes");
        let full = upsample_bilateral(&good, &depth, &config_8x8()).expect("valid half upsamples");
        assert_eq!(full.len(), 64, "8x8 upsamples to 64");
        assert!(
            full.iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
            "valid half stays finite 0..=1"
        );
        for bad in [f32::NAN, f32::INFINITY, 1.5, -0.5] {
            let mut half = good.clone();
            half.values[3] = bad;
            assert!(
                matches!(
                    upsample_bilateral(&half, &depth, &config_8x8()),
                    Err(LookError::Invalid(_))
                ),
                "half texel {bad} refuses loudly so ao_tex keeps 1.0"
            );
            assert!(
                matches!(
                    upsample_bilateral(&half, &depth, &config_8x8()),
                    Err(LookError::Invalid(_))
                ),
                "same bad half refuses the same twice"
            );
        }
    }

    #[test]
    #[cfg(not(feature = "gpu"))]
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
    #[cfg(feature = "gpu")]
    fn gtao_device() -> Result<
        (
            &'static wgpu::Device,
            &'static wgpu::Queue,
            &'static wgpu::AdapterInfo,
        ),
        crate::test_gpu::NoAdapter,
    > {
        crate::graph::gx1_probe()
    }

    #[cfg(feature = "gpu")]
    fn worst_diff(a: &[f32], b: &[f32]) -> f32 {
        a.iter()
            .zip(b.iter())
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max)
    }

    #[test]
    #[cfg(feature = "gpu")]
    fn gpu_half_and_upsample_match_cpu_within_tolerance() {
        let Some((device, queue, info)) = crate::test_gpu::require_or_skip(
            "gpu_half_and_upsample_match_cpu_within_tolerance",
            gtao_device(),
        ) else {
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
    #[cfg(feature = "gpu")]
    fn gpu_baked_bind_roundtrip_is_exact() {
        let Some((device, queue, _)) =
            crate::test_gpu::require_or_skip("gpu_baked_bind_roundtrip_is_exact", gtao_device())
        else {
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
    #[cfg(feature = "gpu")]
    fn gpu_refuses_bad_input_before_sizing() {
        let Some((device, queue, _)) =
            crate::test_gpu::require_or_skip("gpu_refuses_bad_input_before_sizing", gtao_device())
        else {
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
    #[cfg(feature = "gpu")]
    fn gpu_reuses_buffers_on_matching_shape() {
        let Some((device, queue, _)) =
            crate::test_gpu::require_or_skip("gpu_reuses_buffers_on_matching_shape", gtao_device())
        else {
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
    #[cfg(feature = "gpu")]
    fn gpu_steady_state_grows_no_host_scratch() {
        // Steady-state submits grow nothing on the host: every scratch
        // capacity stays flat across submits on a matching shape, and the
        // reused outputs still match the CPU oracle within the posted
        // parity tolerance. Sizing happens once, on the first submit;
        // refusals size nothing (see the refusal test).
        let Some((device, queue, _)) = crate::test_gpu::require_or_skip(
            "gpu_steady_state_grows_no_host_scratch",
            gtao_device(),
        ) else {
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
    #[cfg(feature = "gpu")]
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
    #[cfg(feature = "gpu")]
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
    #[cfg(feature = "gpu")]
    #[ignore = "timing: 1080p gtao budget probe, run under --perf"]
    fn gpu_1080p_budget_probe_reports_ms() {
        let Some((device, queue, info)) =
            crate::test_gpu::require_or_skip("gpu_1080p_budget_probe_reports_ms", gtao_device())
        else {
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

    #[test]
    fn ao_on_off_darkens_valley_keeps_open_sand() {
        // 8x8 step: left near 2 m, right far 6 m, flat up normals.
        let mut depth = vec![0.0f32; 64];
        for y in 0..8 {
            for x in 0..8 {
                depth[y * 8 + x] = if x < 4 { 2.0 } else { 6.0 };
            }
        }
        let normals = vec![[0.0, 0.0, 1.0]; 64];
        let config = config_8x8();
        let (half, _) = gtao_half_res(&depth, &normals, &config).expect("step bakes");
        let full = upsample_bilateral(&half, &depth, &config).expect("step upsamples");
        let (valley, open) = ao_on_off_stats(&full, 8, 8, (4, 1), (0, 1)).expect("probes read");
        assert!(valley < GTAO_OPEN, "valley must darken, got {valley}");
        assert!(
            GTAO_OPEN - valley > 1e-6,
            "on differs from off above epsilon, got {valley}"
        );
        assert!(
            (open - GTAO_OPEN).abs() < 0.02,
            "open sand stays within 2%, got {open}"
        );
    }

    #[test]
    fn far_plane_background_never_occludes_itself() {
        // Cleared texels decode to the far plane: never an occluder.
        let normals = vec![[0.0, 0.0, 1.0]; 64];
        let config = config_8x8();
        let mut far = vec![GTAO_BACKGROUND_M; 64];
        for (i, d) in far.iter_mut().enumerate() {
            if i % 8 < 4 {
                *d += 6.0;
            }
        }
        let (half, _) = gtao_half_res(&far, &normals, &config).expect("background bakes");
        assert!(
            half.values.iter().all(|v| (v - GTAO_OPEN).abs() < 0.02),
            "background stays open"
        );
        let step: Vec<f32> = (0..64).map(|i| if i % 8 < 4 { 2.0 } else { 6.0 }).collect();
        let (hs, _) = gtao_half_res(&step, &normals, &config).expect("step bakes");
        let full = upsample_bilateral(&hs, &step, &config).expect("step upsamples");
        let (v, o) = ao_on_off_stats(&full, 8, 8, (4, 1), (0, 1)).expect("probes read");
        assert!(v < GTAO_OPEN - 1e-6, "step darkens: {v}");
        assert!((o - GTAO_OPEN).abs() < 0.02, "sand open: {o}");
    }

    #[test]
    fn ao_on_off_is_deterministic_x2() {
        let mut depth = vec![0.0f32; 64];
        for y in 0..8 {
            for x in 0..8 {
                depth[y * 8 + x] = if x < 4 { 2.0 } else { 6.0 };
            }
        }
        let normals = vec![[0.0, 0.0, 1.0]; 64];
        let config = config_8x8();
        let bake = |depth: &[f32]| {
            let (half, _) = gtao_half_res(depth, &normals, &config).expect("step bakes");
            upsample_bilateral(&half, depth, &config).expect("step upsamples")
        };
        let first = bake(&depth);
        let second = bake(&depth);
        assert_eq!(first, second, "bake is bit-identical twice");
        let a = ao_on_off_stats(&first, 8, 8, (4, 1), (0, 1)).expect("probes read");
        let b = ao_on_off_stats(&second, 8, 8, (4, 1), (0, 1)).expect("probes read");
        assert_eq!(a.0.to_bits(), b.0.to_bits(), "valley probe is exact x2");
        assert_eq!(a.1.to_bits(), b.1.to_bits(), "open probe is exact x2");
    }

    #[test]
    fn ao_on_off_flat_is_open() {
        assert_eq!(GTAO_OPEN, 1.0, "open documents the blit neutral");
        let full = vec![GTAO_OPEN; 64];
        let (valley, open) =
            ao_on_off_stats(&full, 8, 8, (4, 1), (0, 1)).expect("flat probes read");
        assert_eq!((valley, open), (1.0, 1.0), "flat field stays open");
    }

    #[test]
    fn ao_on_off_refuses_short_slice() {
        let short = vec![GTAO_OPEN; 63];
        assert!(
            matches!(
                ao_on_off_stats(&short, 8, 8, (4, 1), (0, 1)),
                Err(LookError::Invalid(_))
            ),
            "short slice must fail loudly"
        );
        let full = vec![GTAO_OPEN; 64];
        assert!(
            matches!(
                ao_on_off_stats(&full, 0, 8, (0, 0), (0, 0)),
                Err(LookError::Invalid(_))
            ),
            "zero dims must fail loudly"
        );
    }

    #[test]
    fn ao_on_off_refuses_out_of_range_coords() {
        let full = vec![GTAO_OPEN; 64];
        assert!(
            matches!(
                ao_on_off_stats(&full, 8, 8, (8, 1), (0, 1)),
                Err(LookError::Invalid(_))
            ),
            "valley outside must fail loudly"
        );
        assert!(
            matches!(
                ao_on_off_stats(&full, 8, 8, (4, 1), (0, 8)),
                Err(LookError::Invalid(_))
            ),
            "open outside must fail loudly"
        );
    }

    #[test]
    fn ao_full_for_blit_accepts_step_bake() {
        let mut depth = vec![0.0f32; 64];
        for y in 0..8 {
            for x in 0..8 {
                depth[y * 8 + x] = if x < 4 { 2.0 } else { 6.0 };
            }
        }
        let normals = vec![[0.0, 0.0, 1.0]; 64];
        let config = config_8x8();
        let (half, _) = gtao_half_res(&depth, &normals, &config).expect("step bakes");
        let full = upsample_bilateral(&half, &depth, &config).expect("step upsamples");
        check_ao_full_for_blit(&full, 8, 8).expect("step bake feeds the blit");
        let (valley, open) = ao_on_off_stats(&full, 8, 8, (4, 1), (0, 1)).expect("probes read");
        assert!(valley < GTAO_OPEN, "valley must darken, got {valley}");
        assert!(
            (open - GTAO_OPEN).abs() < 0.02,
            "open sand stays within 2%, got {open}"
        );
    }

    #[test]
    fn ao_full_for_blit_rejects_bad_fields() {
        let full = vec![GTAO_OPEN; 64];
        assert!(
            matches!(
                check_ao_full_for_blit(&full[..63], 8, 8),
                Err(LookError::Invalid(_))
            ),
            "short slice must fail loudly"
        );
        let mut nan = vec![GTAO_OPEN; 64];
        nan[10] = f32::NAN;
        assert!(
            matches!(
                check_ao_full_for_blit(&nan, 8, 8),
                Err(LookError::Invalid(_))
            ),
            "NaN AO must fail loudly"
        );
        let mut over = vec![GTAO_OPEN; 64];
        over[20] = 1.5;
        assert!(
            matches!(
                check_ao_full_for_blit(&over, 8, 8),
                Err(LookError::Invalid(_))
            ),
            "AO above 1 must fail loudly"
        );
    }

    #[test]
    fn gtao_live_binds_ao_after_forward() {
        // Forward-stored step depth (left near 2 m, right far 6 m): the live
        // helper must darken the valley while open sand stays open; empty or
        // zero-size stays skip (caller keeps neutral 1.0, no extra work).
        let mut depth = vec![0.0f32; 64];
        for y in 0..8 {
            for x in 0..8 {
                depth[y * 8 + x] = if x < 4 { 2.0 } else { 6.0 };
            }
        }
        let normals = vec![[0.0, 0.0, 1.0]; 64];
        let config = config_8x8();
        let (full, ms) = gtao_live_full_for_forward(&depth, &normals, &config).expect("step binds");
        assert_eq!(full.len(), 64, "full field covers the frame");
        assert!(ms >= 0.0, "live bind records ms");
        let (valley, open) = ao_on_off_stats(&full, 8, 8, (4, 1), (0, 1)).expect("probes read");
        assert!(valley < GTAO_OPEN, "valley must darken, got {valley}");
        assert!(
            (open - GTAO_OPEN).abs() < 0.02,
            "open sand stays within 2%, got {open}"
        );
        let zero = GtaoConfig {
            width_px: 0,
            ..config
        };
        assert!(
            matches!(
                gtao_live_full_for_forward(&depth, &normals, &zero),
                Err(LookError::Invalid(_))
            ),
            "zero-size stays skip with a loud refusal"
        );
        let neutral = vec![GTAO_OPEN; 64];
        assert!(
            neutral.iter().all(|v| *v == 1.0),
            "skipped live bind keeps ao_tex at 1.0"
        );
    }

    #[test]
    fn stale_ao_reuse_darkens_valley_keeps_open_sand() {
        // Caller: `stale_ao_should_reuse` gate plus `check_ao_full_for_blit`.
        // Behavior: cached step field validates, valley darkens, open stays.
        let mut depth = vec![0.0f32; 64];
        for y in 0..8 {
            for x in 0..8 {
                depth[y * 8 + x] = if x < 4 { 2.0 } else { 6.0 };
            }
        }
        let normals = vec![[0.0, 0.0, 1.0]; 64];
        let config = config_8x8();
        let (first, _) = gtao_live_full_for_forward(&depth, &normals, &config).expect("step binds");
        let (second, _) =
            gtao_live_full_for_forward(&depth, &normals, &config).expect("step binds again");
        assert_eq!(first, second, "same feed gives same field twice");
        assert!(stale_ao_should_reuse(&first, 8, 8), "step bake reuses");
        let (valley, open) = ao_on_off_stats(&first, 8, 8, (4, 1), (0, 1)).expect("probes read");
        assert!(valley < GTAO_OPEN, "valley must darken, got {valley}");
        assert!(
            (open - GTAO_OPEN).abs() < 0.02,
            "open sand stays within 2%, got {open}"
        );
        assert!(
            !stale_ao_should_reuse(&first[..63], 8, 8),
            "short slice stays skip"
        );
        assert!(!stale_ao_should_reuse(&first, 0, 8), "zero size stays skip");
    }

    #[test]
    fn live_helper_routes_through_live_bind() {
        // Caller: `crates/render/src/present/gpu_present/submit.rs` `live_gtao_ao`
        // after `encode_forward` before `encode_blit`.
        // Behavior: the helper routes through `gtao_live_full_for_forward`
        // (half plus bilateral upsample), BUT the only production caller
        // (`submit_ibl_chart`) passes empty `&[]`/`&[]`, so the gate always
        // skips and the receipt stays `None`. This pins both halves: the
        // routing inside the helper and the empty-feed skip at the call site.
        let src = include_str!("present/gpu_present/submit.rs");
        let ao_fn = src.find("fn live_gtao_ao").expect("live helper exists");
        let ao_end = src[ao_fn..]
            .find("fn live_ao_upload_for_frame")
            .map(|i| ao_fn + i)
            .unwrap_or(src.len());
        assert!(
            src[ao_fn..ao_end].contains("gtao_live_full_for_forward"),
            "live helper routes through the live bind after forward"
        );
        assert!(
            src.contains("let depth: &[f32] = &[];"),
            "production caller passes an empty feed, so the live bind skips"
        );
    }

    #[test]
    fn live_fresh_ao_darkens_valley_with_stored_depth() {
        // Caller: `crates/render/src/present/gpu_present/live.rs`
        // `present_frame_live` fresh path after `encode_forward`, through
        // `gtao_live_full_for_forward` (half-res oracle plus bilateral
        // upsample) into the full-res field the blit samples as `ao_tex`.
        // Behavior: empty feed refuses loudly so the caller keeps `ao_tex`
        // at 1.0 (skip, no extra work); the forward-stored step depth (left
        // near 2 m, right far 6 m) darkens the valley while open sand stays
        // within 2%; same feed gives the same field twice; ms is evidence
        // only. Slice 1 keeps the whole-HDR multiply (darkens direct sun
        // too); ambient-only moves to the slice-2 prepass.
        let config = config_8x8();
        let empty_depth: &[f32] = &[];
        let empty_normals: &[[f32; 3]] = &[];
        assert!(
            matches!(
                gtao_live_full_for_forward(empty_depth, empty_normals, &config),
                Err(LookError::Invalid(_))
            ),
            "empty feed refuses loudly so ao_tex keeps 1.0"
        );
        let neutral = vec![GTAO_OPEN; 64];
        assert!(
            neutral.iter().all(|v| *v == 1.0),
            "skipped fresh bind keeps ao_tex at 1.0"
        );
        let mut depth = vec![0.0f32; 64];
        for y in 0..8 {
            for x in 0..8 {
                depth[y * 8 + x] = if x < 4 { 2.0 } else { 6.0 };
            }
        }
        let normals = vec![[0.0, 0.0, 1.0]; 64];
        let (first, ms) =
            gtao_live_full_for_forward(&depth, &normals, &config).expect("stored depth binds");
        let (second, _) = gtao_live_full_for_forward(&depth, &normals, &config)
            .expect("stored depth binds again");
        assert_eq!(first, second, "same stored feed gives same field twice");
        assert!(ms >= 0.0, "live bind records ms as evidence only");
        check_ao_full_for_blit(&first, 8, 8).expect("stored bake guards clean for the blit");
        let (valley, open) = ao_on_off_stats(&first, 8, 8, (4, 1), (0, 1)).expect("probes read");
        assert!(valley < GTAO_OPEN, "valley must darken, got {valley}");
        assert!(
            (open - GTAO_OPEN).abs() < 0.02,
            "open sand stays within 2%, got {open}"
        );
        // Live wiring: the fresh bake runs after forward and before the
        // blit, with the stale reuse kept as fallback and the empty gate
        // kept as skip.
        let src = include_str!("present/gpu_present/live.rs");
        let live_fn = src
            .find("fn present_frame_live")
            .expect("live frame exists");
        let live_end = src[live_fn..]
            .find("fn present_frame_probed")
            .map(|i| live_fn + i)
            .unwrap_or(src.len());
        let live_body = &src[live_fn..live_end];
        for token in [
            "encode_forward",
            "live_ao_upload_for_frame",
            "stale_ao_reuse_upload",
            "encode_blit",
            "live_ao_should_bind",
        ] {
            assert!(
                live_body.contains(token),
                "live frame wires {token} after forward before the blit"
            );
        }
        let fwd_at = live_body
            .find("encode_forward")
            .expect("forward sits in live frame");
        let helper_at = live_body
            .find("live_ao_upload_for_frame")
            .expect("fresh helper sits in live frame");
        let blit_at = live_body
            .find("encode_blit")
            .expect("blit sits in live frame");
        assert!(
            fwd_at < helper_at && helper_at < blit_at,
            "fresh bake runs after forward and before the blit"
        );
    }
}

/// Live-kernel oracle fixtures: a pinhole camera over analytic planes, so the
/// depth the oracle reads is exact and no device or mesh is involved.
#[cfg(test)]
mod live_oracle_tests {
    use super::*;

    const W: u32 = 96;
    const H: u32 = 72;
    const EYE: [f32; 3] = [0.0, 1.7, 0.0];
    const FOV_Y: f32 = 0.9;
    const NEAR: f32 = 0.1;
    const FAR: f32 = 100.0;

    fn config() -> GtaoConfig {
        GtaoConfig {
            width_px: W,
            height_px: H,
            radius_px: 8.0,
            falloff_m: 2.0,
            intensity: 1.0,
        }
    }

    fn aspect() -> f32 {
        W as f32 / H as f32
    }

    /// Column-major inverse view-projection of a pinhole at `EYE` looking
    /// down -Z with the DirectX depth range: the unprojection of one NDC
    /// point is `inv_proj * clip` then a translation by the eye.
    fn inv_cols() -> [[f32; 4]; 4] {
        let f = 1.0 / (FOV_Y * 0.5).tan();
        let a = FAR / (NEAR - FAR);
        let b = NEAR * FAR / (NEAR - FAR);
        [
            [aspect() / f, 0.0, 0.0, 0.0],
            [0.0, 1.0 / f, 0.0, 0.0],
            [EYE[0] / b, EYE[1] / b, EYE[2] / b, 1.0 / b],
            [EYE[0] * a / b, EYE[1] * a / b, -1.0 + EYE[2] * a / b, a / b],
        ]
    }

    /// What one pixel's ray hits.
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Hit {
        Floor([f32; 3]),
        Wall([f32; 3]),
        Sky,
    }

    /// Ray-cast the floor `y = 0` and, when `wall` is set, a wall at
    /// `z = -6` that stands 3 m tall; the nearest hit wins.
    fn cast(wall: bool) -> Vec<(Hit, f32)> {
        let tan = (FOV_Y * 0.5).tan();
        let a = FAR / (NEAR - FAR);
        let b = NEAR * FAR / (NEAR - FAR);
        let mut out = Vec::new();
        for y in 0..H {
            for x in 0..W {
                let nx = 2.0 * (x as f32 + 0.5) / W as f32 - 1.0;
                let ny = 1.0 - 2.0 * (y as f32 + 0.5) / H as f32;
                let dir = [nx * aspect() * tan, ny * tan, -1.0];
                let mut best: Option<(f32, Hit)> = None;
                if dir[1] < 0.0 {
                    let t = -EYE[1] / dir[1];
                    let p = [EYE[0] + t * dir[0], 0.0, EYE[2] - t];
                    best = Some((t, Hit::Floor(p)));
                }
                if wall {
                    let t = EYE[2] + 6.0;
                    let p = [EYE[0] + t * dir[0], EYE[1] + t * dir[1], -6.0];
                    let nearer = best.is_none_or(|(tb, _)| t < tb);
                    if (0.0..=3.0).contains(&p[1]) && nearer {
                        best = Some((t, Hit::Wall(p)));
                    }
                }
                out.push(match best {
                    // Past the far plane the rasterizer clips: that is sky.
                    Some((t, hit)) if (a * -t + b) / t <= 1.0 => (hit, (a * -t + b) / t),
                    _ => (Hit::Sky, 1.0),
                });
            }
        }
        out
    }

    fn depth_of(scene: &[(Hit, f32)]) -> Vec<f32> {
        scene.iter().map(|(_, d)| *d).collect()
    }

    fn run(scene: &[(Hit, f32)]) -> Vec<f32> {
        gtao_live_oracle(&depth_of(scene), inv_cols(), EYE, &config()).expect("oracle runs")
    }

    #[test]
    fn open_floor_and_sky_read_exactly_open() {
        // A floor seen at a grazing angle is flat: no tap sits above its
        // tangent plane, so every texel is 1.0 (a depth-difference kernel
        // darkened it against itself).
        let scene = cast(false);
        assert!(scene.iter().any(|(h, _)| matches!(h, Hit::Floor(_))));
        assert!(scene.iter().any(|(h, _)| *h == Hit::Sky), "sky is in view");
        let ao = run(&scene);
        for (i, v) in ao.iter().enumerate() {
            assert!(
                (*v - 1.0).abs() < 1e-6,
                "texel {i} reads {v}, an open floor never occludes itself"
            );
        }
    }

    #[test]
    fn a_wall_base_darkens_both_surfaces_and_leaves_open_ground_alone() {
        let scene = cast(true);
        let ao = run(&scene);
        let mut base_floor = Vec::new();
        let mut base_wall = Vec::new();
        for (i, (hit, _)) in scene.iter().enumerate() {
            match hit {
                Hit::Floor(p) if p[2] < -5.65 => base_floor.push(ao[i]),
                Hit::Wall(p) if p[1] < 0.35 => base_wall.push(ao[i]),
                Hit::Floor(p) if p[2] > -4.0 => assert!(
                    (ao[i] - 1.0).abs() < 1e-6,
                    "open floor at z {} reads {}",
                    p[2],
                    ao[i]
                ),
                Hit::Floor(_) => {}
                Hit::Wall(p) if p[1] > 2.0 => {
                    assert!((ao[i] - 1.0).abs() < 1e-6, "high wall reads {}", ao[i]);
                }
                Hit::Wall(_) => {}
                Hit::Sky => assert_eq!(ao[i], 1.0, "sky never occludes"),
            }
        }
        assert!(
            base_floor.len() > 20 && base_wall.len() > 10,
            "both sides of the corner are in view: {} / {}",
            base_floor.len(),
            base_wall.len()
        );
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        assert!(
            mean(&base_floor) < 0.85,
            "floor at the base: {}",
            mean(&base_floor)
        );
        assert!(
            mean(&base_wall) < 0.85,
            "wall at the base: {}",
            mean(&base_wall)
        );
        assert!(ao.iter().all(|v| (0.0..=1.0).contains(v)));
    }

    #[test]
    fn the_oracle_repeats_and_refuses_bad_input() {
        let scene = cast(true);
        let depth = depth_of(&scene);
        let first = gtao_live_oracle(&depth, inv_cols(), EYE, &config()).expect("runs");
        let second = gtao_live_oracle(&depth, inv_cols(), EYE, &config()).expect("runs again");
        assert_eq!(first, second, "same depth, same field");
        let short = &depth[..depth.len() - 1];
        assert!(gtao_live_oracle(short, inv_cols(), EYE, &config()).is_err());
        let mut bad = depth.clone();
        bad[5] = 1.5;
        assert!(gtao_live_oracle(&bad, inv_cols(), EYE, &config()).is_err());
        bad[5] = f32::NAN;
        assert!(gtao_live_oracle(&bad, inv_cols(), EYE, &config()).is_err());
        let zero = GtaoConfig {
            width_px: 0,
            ..config()
        };
        assert!(gtao_live_oracle(&depth, inv_cols(), EYE, &zero).is_err());
        let mut singular = inv_cols();
        singular[3][3] = 0.0;
        singular[2][3] = 0.0;
        assert!(
            gtao_live_oracle(&depth, singular, EYE, &config()).is_err(),
            "a degenerate unprojection fails loudly"
        );
    }
}
