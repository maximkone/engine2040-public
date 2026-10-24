//! Plain-data values shared by the frame graph and the look passes.
//!
//! These cannot belong to any single pass: the graph packs a `LookFrame` into
//! its uniform image, and every pass validates the same one before binding.
//! Keeping them here is what lets a pass move to its own crate without pulling
//! the graph along.

/// Largest accepted look dimension in pixels (mirrors the RA1 frame bound).
pub const MAX_LOOK_DIM: u32 = 2048;

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

impl std::fmt::Display for LookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "look input: {e}"),
            Self::NoDevice(e) => write!(f, "look bind needs a device: {e}"),
            Self::Device(e) => write!(f, "look device: {e}"),
        }
    }
}

impl std::error::Error for LookError {}

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

/// FNV-1a 64-bit over bytes. A stable, allocation-free digest for receipts,
/// shared by the frame graph and the shadow pass so neither owns it.
pub fn fnv1a64(b: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for x in b {
        h ^= *x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
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
}
