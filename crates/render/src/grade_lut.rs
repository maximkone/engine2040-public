//! SW152: one film grade as a 16-cube colour LUT plus an 8-bit dither, so the
//! dust, torch and blood in a frame land in one palette and the potato tier
//! stops banding in the sky.
//!
//! Consumer: the display stage (`crate::display_grade`, the post preset) picks
//! a [`GradeLut`] and the present shader samples it (`shaders/grade_lut.wgsl`,
//! `sampler3D` on device); this module is the CPU twin the accept measures:
//! trilinear lookup over `[[u8; 3]; 4096]`, 12,288 bytes, mip-free.
//!
//! The neutral LUT is built from [`neutral_sample`], the display stage's own
//! PBR Neutral curve, so a frame with no grade keeps today's bytes at every
//! grid point. Units: display `0..=1` in, bytes out; LUT bytes for the budget.
//! Determinism: pure integer and `f32` math on a fixed grid, no clock, no RNG.
//! Presentation only: nothing here feeds the tick or the hash.

use crate::quality::Quality;

/// Grid edge of the shipped LUT: 16 samples per axis.
pub const LUT_EDGE: usize = 16;
/// Entries in the shipped cube (`16^3`).
pub const LUT_ENTRIES: usize = LUT_EDGE * LUT_EDGE * LUT_EDGE;
/// Bytes the shipped cube costs (`4096 * 3`), mip-free.
pub const LUT_BYTES: usize = LUT_ENTRIES * 3;
/// Largest edge a LUT may ask for; `32^3` is the row's budget probe.
pub const LUT_EDGE_MAX: usize = 32;
/// Bytes a [`LUT_EDGE_MAX`] cube costs (`32^3 * 3`).
pub const LUT_BYTES_32: usize = LUT_EDGE_MAX * LUT_EDGE_MAX * LUT_EDGE_MAX * 3;
/// Largest LUT the potato tier loads: the shipped 16-cube, 12,288 bytes.
pub const POTATO_LUT_MAX_BYTES: usize = LUT_BYTES;

/// Why a LUT was refused before it reached a frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LutError {
    /// The edge is not one of the shipped sizes.
    BadEdge {
        /// The edge asked for.
        edge: usize,
    },
    /// The cube is bigger than the tier pays for.
    TooBigForTier {
        /// Tier that refused it.
        tier: Quality,
        /// Bytes the cube would cost.
        bytes: usize,
    },
}

impl core::fmt::Display for LutError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadEdge { edge } => write!(
                f,
                "LUT edge {edge} is not a shipped size; re-run with {LUT_EDGE} or {LUT_EDGE_MAX}"
            ),
            Self::TooBigForTier { tier, bytes } => write!(
                f,
                "a {bytes}-byte LUT is over the {tier:?} tier's {} bytes; re-run with the \
                 {LUT_EDGE}-cube",
                POTATO_LUT_MAX_BYTES
            ),
        }
    }
}

impl std::error::Error for LutError {}

/// One graded 16-cube: display `0..=1` in, display `0..=1` out, trilinear.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GradeLut {
    edge: usize,
    entries: Vec<[u8; 3]>,
}

impl GradeLut {
    /// Build a cube of `edge` samples per axis from `sample`, refusing an edge
    /// that is not one of the shipped sizes.
    pub fn from_sample(
        edge: usize,
        sample: impl Fn([f32; 3]) -> [f32; 3],
    ) -> Result<Self, LutError> {
        if edge != LUT_EDGE && edge != LUT_EDGE_MAX {
            return Err(LutError::BadEdge { edge });
        }
        Ok(Self::build(edge, sample))
    }

    /// Fill a cube of a `const` edge: infallible, so the shipped builders have no
    /// panic site; `from_sample` is the checked door for an edge from elsewhere.
    fn build(edge: usize, sample: impl Fn([f32; 3]) -> [f32; 3]) -> Self {
        let last = Self::edge_minus_one(edge);
        let mut entries = Vec::with_capacity(edge * edge * edge);
        for b in 0..edge {
            for g in 0..edge {
                for r in 0..edge {
                    let written = sample([
                        Self::grid_axis(r) / last,
                        Self::grid_axis(g) / last,
                        Self::grid_axis(b) / last,
                    ]);
                    entries.push([
                        quantize8(written[0]),
                        quantize8(written[1]),
                        quantize8(written[2]),
                    ]);
                }
            }
        }
        Self { edge, entries }
    }

    /// The neutral cube: the display stage's own PBR Neutral curve on the grid,
    /// so a frame with no grade keeps today's bytes at every grid point.
    pub fn neutral() -> Self {
        Self::build(LUT_EDGE, crate::display_grade::pbr_neutral)
    }

    /// A red-gain cube at the shipped edge: `gain_milli` thousandths applied to
    /// the red channel only, so a warm grade lifts red without moving green or
    /// blue.
    pub fn red_gain(gain_milli: u16) -> Self {
        let gain = f32::from(gain_milli) / 1000.0;
        Self::build(LUT_EDGE, |rgb| {
            let base = crate::display_grade::pbr_neutral(rgb);
            [base[0] * gain, base[1], base[2]]
        })
    }

    /// Grid edge of this cube.
    pub fn edge(&self) -> usize {
        self.edge
    }

    /// Bytes this cube costs in memory and on the GPU.
    pub fn bytes(&self) -> usize {
        self.entries.len() * 3
    }

    /// One axis coordinate as the grid cell under it: the one cast site. The
    /// sample lattice is `k / (edge - 1)`, so the cell is `floor(v * (edge - 1))`
    /// and `frac` is what is left over; a wrong spacing left a 49-code hole at
    /// 0.875 in this module's own pre-check.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the coordinate is clamped to 0..=1 and multiplied by the grid span (15 or 31), so it is a small non-negative integer"
    )]
    fn grid_cell(axis: f32, span: f32, top: usize) -> usize {
        let index = axis.clamp(0.0, 1.0) * span;
        (index as usize).min(top)
    }

    /// A grid index back as an `f32` sample coordinate in `0..=1`.
    #[expect(
        clippy::cast_precision_loss,
        reason = "the index is bounded by the LUT edge (16 or 32), exactly representable"
    )]
    fn grid_axis(index: usize) -> f32 {
        index as f32
    }

    /// `edge - 1` as an `f32`: the last sample coordinate.
    fn edge_minus_one(edge: usize) -> f32 {
        Self::grid_axis(edge) - 1.0
    }

    /// One entry by grid coordinate.
    pub fn entry(&self, r: usize, g: usize, b: usize) -> [u8; 3] {
        let edge = self.edge;
        self.entries[(b * edge + g) * edge + r]
    }

    /// Trilinear lookup: the shader's `sampler3D` read, linear in three axes.
    pub fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
        let edge = self.edge;
        let last = Self::edge_minus_one(edge);
        let axis = [
            rgb[0].clamp(0.0, 1.0) * last,
            rgb[1].clamp(0.0, 1.0) * last,
            rgb[2].clamp(0.0, 1.0) * last,
        ];
        let lo = [
            Self::grid_cell(rgb[0], last, edge - 1),
            Self::grid_cell(rgb[1], last, edge - 1),
            Self::grid_cell(rgb[2], last, edge - 1),
        ];
        let frac = [
            axis[0] - Self::grid_axis(lo[0]),
            axis[1] - Self::grid_axis(lo[1]),
            axis[2] - Self::grid_axis(lo[2]),
        ];
        let mut out = [0.0_f32; 3];
        for (channel, slot) in out.iter_mut().enumerate() {
            let mut value = 0.0_f32;
            for b in 0..2 {
                for g in 0..2 {
                    for r in 0..2 {
                        let index = [
                            if r == 0 {
                                lo[0]
                            } else {
                                (lo[0] + 1).min(edge - 1)
                            },
                            if g == 0 {
                                lo[1]
                            } else {
                                (lo[1] + 1).min(edge - 1)
                            },
                            if b == 0 {
                                lo[2]
                            } else {
                                (lo[2] + 1).min(edge - 1)
                            },
                        ];
                        let weight = (if r == 0 { 1.0 - frac[0] } else { frac[0] })
                            * (if g == 0 { 1.0 - frac[1] } else { frac[1] })
                            * (if b == 0 { 1.0 - frac[2] } else { frac[2] });
                        value +=
                            f32::from(self.entry(index[0], index[1], index[2])[channel]) * weight;
                    }
                }
            }
            *slot = value / 255.0;
        }
        out
    }

    /// Load a cube for a tier: a 32-cube is refused on the potato row, which
    /// pays only [`POTATO_LUT_MAX_BYTES`].
    pub fn load_for(edge: usize, tier: Quality) -> Result<Self, LutError> {
        let cube = Self::from_sample(edge, crate::display_grade::pbr_neutral)?;
        if cube.bytes() > POTATO_LUT_MAX_BYTES && tier == Quality::Potato {
            return Err(LutError::TooBigForTier {
                tier,
                bytes: cube.bytes(),
            });
        }
        Ok(cube)
    }
}

/// One pixel's 4x4 ordered dither offset, half an 8-bit step either way: the
/// LUT stage's small offset, which cannot invert a one-unit ramp.
pub fn dither_4x4(px: [f32; 2]) -> [f32; 3] {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the coordinate is reduced to a 0..3 cell before the cast"
    )]
    let x = (px[0].abs() as u32) & 3;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the coordinate is reduced to a 0..3 cell before the cast"
    )]
    let y = (px[1].abs() as u32) & 3;
    // Bayer 4x4, 0..15 in the standard order.
    const BAYER: [u8; 16] = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
    let cell = usize::try_from(y * 4 + x).unwrap_or(0);
    let step = (f32::from(BAYER[cell]) + 0.5) / 16.0 - 0.5;
    [step / 255.0; 3]
}

/// Quantize one display value to the 8-bit store.
fn quantize8(value: f32) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the value is clamped to 0..=255 and rounded before the cast"
    )]
    let code = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    code
}

#[cfg(test)]
#[path = "grade_lut_tests.rs"]
mod tests;
