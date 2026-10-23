//! The LF5 motion target's previous-frame state (the compact position stream
//! and the previous view-projection the forward pass subtracts) plus LF5b's
//! Halton(2,3) camera jitter, which rides the camera uniform but never a
//! motion vector.
//!
//! Owner: render lane. Units: meters for positions, the camera matrix bytes
//! the shader reads, pixels for the jitter. Determinism: pure copies of what
//! the last frame held plus the integer frame number; no clocks, no IO, no
//! locking. Errors: none (a missing mesh is a no-op).

use wgpu::util::DeviceExt as _;

use super::submit::SKIN_BLOCK_JOINTS;

/// The previous-position stream of a mesh's bind pose: the same compact rows
/// a refresh rewrites, allocated on the cold upload path.
pub(super) fn create_prev_buffer(device: &wgpu::Device, positions: &[[f32; 3]]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("ra1-mesh-prev-vb"),
        contents: &pack_positions(positions),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
    })
}

/// Write `last` into `prev`, allocating its own bytes: the upload path's
/// scratch already holds the new vertex image.
pub(super) fn write_prev_positions_alloc(
    queue: &wgpu::Queue,
    prev: &wgpu::Buffer,
    last: &[[f32; 3]],
) {
    queue.write_buffer(prev, 0, &pack_positions(last));
}

/// Compact `vec3` rows of `positions`: the LF5 previous-position stream
/// bytes, matching [`super::submit::PREV_POSITION_STRIDE`]. Pure.
pub(super) fn pack_positions(positions: &[[f32; 3]]) -> Vec<u8> {
    let stride = usize::try_from(super::submit::PREV_POSITION_STRIDE).unwrap_or(12);
    let mut bytes = Vec::with_capacity(positions.len() * stride);
    for position in positions {
        for value in position {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    bytes
}

/// Write `last` into the previous-position stream `prev`, reusing `scratch`.
/// `scratch` must be free at the call: a refresh publishes the old pose
/// before packing the new one, and an upload packs before it reuses the key.
pub(super) fn write_prev_positions(
    queue: &wgpu::Queue,
    scratch: &mut Vec<u8>,
    prev: &wgpu::Buffer,
    last: &[[f32; 3]],
) {
    scratch.clear();
    for position in last {
        for component in position {
            scratch.extend_from_slice(&component.to_le_bytes());
        }
    }
    queue.write_buffer(prev, 0, scratch);
}

/// The previous palette bytes for `matrices` at `prev[base..]`: each row is
/// the matrix the block held last time, or today's on its first write, so a
/// fresh body reports zero motion instead of a jump from the bind pose.
pub(super) fn prev_skin_bytes(
    base: usize,
    matrices: &[[[f32; 4]; 4]],
    prev: &mut [[[f32; 4]; 4]],
    seen: bool,
) -> [u8; SKIN_BLOCK_JOINTS * 64] {
    let mut bytes = [0u8; SKIN_BLOCK_JOINTS * 64];
    for (joint, matrix) in matrices.iter().enumerate() {
        let old = if seen { prev[base + joint] } else { *matrix };
        let at = joint * 64;
        for (value, chunk) in old.iter().flatten().zip(bytes[at..].chunks_exact_mut(4)) {
            chunk.copy_from_slice(&value.to_le_bytes());
        }
        prev[base + joint] = *matrix;
    }
    bytes
}

/// LF5b probe: the camera images one frame wrote, for tests and receipts.
/// `staged` is what the frame packed, `cam` what `cam_buf` took (the staged
/// bytes plus the jitter in the clip w column), `steady` the un-jittered
/// matrix binding 23 holds and `prev` the previous camera binding 21 took.
#[derive(Clone, Copy)]
pub(crate) struct JitterProbe {
    /// The jitter this frame's projection carried, in pixels; zero when off.
    pub(crate) px: [f32; 2],
    /// The 80 camera-uniform bytes the frame staged, before any jitter.
    pub(crate) staged: [u8; 80],
    /// The 80 bytes `cam_buf` took this frame.
    pub(crate) cam: [u8; 80],
    /// The un-jittered 64-byte view-projection binding 23 holds.
    pub(crate) steady: [u8; 64],
    /// The 64-byte previous view-projection binding 21 took.
    pub(crate) prev: [u8; 64],
}

impl JitterProbe {
    /// A probe before any frame: zero images, zero jitter.
    const EMPTY: Self = Self {
        px: [0.0; 2],
        staged: [0; 80],
        cam: [0; 80],
        steady: [0; 64],
        prev: [0; 64],
    };
}

/// LF5b camera jitter state: the switch (off by default), the un-jittered
/// matrix binding 23 hands the motion entries, and the images the last
/// frame wrote.
pub(crate) struct JitterState {
    /// Halton(2,3) projection jitter; off until the TAA resolve lands (LF5c).
    pub(crate) on: bool,
    /// The un-jittered view-projection, written every frame.
    pub(crate) steady_buf: wgpu::Buffer,
    /// The camera images the last frame wrote (tests and receipts).
    pub(crate) probe: JitterProbe,
}

impl JitterState {
    /// Cold init: the switch off, a zeroed steady matrix, an empty probe.
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let steady_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ra1-u-steady-camera"),
            contents: &[0u8; 64],
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        Self {
            on: false,
            steady_buf,
            probe: JitterProbe::EMPTY,
        }
    }
}

/// One little-endian `f32` at `at` in a camera-uniform image.
fn f32_at(bytes: &[u8], at: usize) -> f32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[at..at + 4]);
    f32::from_le_bytes(word)
}

/// Write this frame's camera uniform: the staged bytes, plus the Halton(2,3)
/// jitter when the switch is on. The jitter is a projection shift, so on the
/// view-projection product it subtracts `ndc` times the clip w row from rows
/// 0 and 1 (all four columns); that moves every clip x by `-ndc.x * clip.w`,
/// the constant NDC offset TAA wants. The un-jittered matrix goes to binding
/// 23 and to the previous-camera history (binding 21), so a motion vector
/// never carries the jitter. Off writes the staged bytes unchanged, so a
/// frame stays byte-equal to the pre-jitter path.
pub(super) fn publish_camera(presenter: &mut crate::present::Presenter, cam: &[u8]) {
    let mut staged = [0u8; 80];
    if let Some(head) = cam.get(..staged.len()) {
        staged.copy_from_slice(head);
    }
    let (width, height) = presenter
        .targets
        .as_ref()
        .map_or((0, 0), |targets| (targets.width, targets.height));
    let on = presenter.jitter.on && width > 0 && height > 0;
    let px = if on {
        crate::post::jitter_offset(presenter.frames_presented)
    } else {
        [0.0; 2]
    };
    let mut written = staged;
    if on {
        let ndc = crate::post::jitter_ndc(presenter.frames_presented, width, height);
        // Column-major: (row r, column c) sits at float index `c * 4 + r`,
        // byte `c * 16 + r * 4`. Row 3 (the clip w row) stays as staged, so
        // read it before either write; rows 2 and 3 are never touched.
        for column in 0..4 {
            let base = column * 16;
            let w = f32_at(&staged, base + 12);
            let x = f32_at(&staged, base) - ndc[0] * w;
            let y = f32_at(&staged, base + 4) - ndc[1] * w;
            written[base..base + 4].copy_from_slice(&x.to_le_bytes());
            written[base + 4..base + 8].copy_from_slice(&y.to_le_bytes());
        }
    }
    let mut steady = [0u8; 64];
    steady.copy_from_slice(&staged[..64]);
    let previous = presenter.prev_cam_bytes.unwrap_or(steady);
    presenter
        .queue
        .write_buffer(&presenter.cam_buf, 0, &written);
    presenter
        .queue
        .write_buffer(&presenter.jitter.steady_buf, 0, &steady);
    presenter.write_prev_camera(&steady);
    presenter.jitter.probe = JitterProbe {
        px,
        staged,
        cam: written,
        steady,
        prev: previous,
    };
}

impl crate::present::Presenter {
    /// Stage this frame's camera: publish the previous view-projection first,
    /// so the motion target subtracts it, then keep this frame's bytes for
    /// the next one. The first frame publishes itself, so a fresh presenter
    /// reports zero camera motion.
    pub(super) fn write_prev_camera(&mut self, cam: &[u8]) {
        let Some(head) = cam.get(..64) else {
            return;
        };
        let mut current = [0u8; 64];
        current.copy_from_slice(head);
        let previous = self.prev_cam_bytes.unwrap_or(current);
        self.queue.write_buffer(&self.prev_cam_buf, 0, &previous);
        self.prev_cam_bytes = Some(current);
    }

    /// LF5b: switch the Halton(2,3) projection jitter on. Off by default:
    /// until the TAA resolve consumes the jitter (LF5c), an off frame is
    /// byte-equal to the pre-jitter path.
    pub fn set_taa_jitter_enabled(&mut self, on: bool) {
        self.jitter.on = on;
    }

    /// Whether the projection jitter is on ([`Self::set_taa_jitter_enabled`]).
    pub fn taa_jitter_enabled(&self) -> bool {
        self.jitter.on
    }

    /// The projection jitter this frame carried, in pixels; zero when off.
    pub fn applied_jitter_px(&self) -> [f32; 2] {
        self.jitter.probe.px
    }

    /// The camera-uniform images the last frame wrote: the staged 80 bytes,
    /// the 80 `cam_buf` took, the 64-byte un-jittered matrix binding 23
    /// holds and the 64-byte previous matrix binding 21 took. A receipt for
    /// tests and look captures; no pass reads it.
    pub fn last_camera_images(&self) -> ([u8; 80], [u8; 80], [u8; 64], [u8; 64]) {
        let probe = &self.jitter.probe;
        (probe.staged, probe.cam, probe.steady, probe.prev)
    }
}
