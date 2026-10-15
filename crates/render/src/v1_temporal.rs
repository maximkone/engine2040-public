//! Test-only temporal parity oracle: render sizes below 1080p,
//! R4-jitter uniforms, exact history buffers, bilinear reference.
//! Consumer: `post::taa_upsample` parity test plus the in-module oracle.
//! Owner `engine-render`; pixels only, never the simulation hash.
//! Units: pixels, NDC, linear RGB. Determinism: integer size math,
//! fixed tap order, same inputs give identical outputs. Errors typed.
//!
//! Live path is `post::taa_upsample`; this module is `#[cfg(test)]`
//! so the window build cannot reach it.

use std::fmt;

use crate::v1::MAX_CAPTURE_DIM;
use crate::v1_lighting::TaaJitter;

// ---------------------------------------------------------------------------
// Quality presets: plain render-scale fractions, not FSR code.
// ---------------------------------------------------------------------------

/// Render-scale preset: which fraction of the presentation size the GPU
/// renders at. Fractions are exact integer ratios so every run derives
/// identical target sizes from the same presentation size.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UpscalePreset {
    /// 77/100 of each axis (e.g. 1920x1080 -> 1478x831).
    Ultra,
    /// 2/3 of each axis (e.g. 1920x1080 -> 1280x720).
    Quality,
    /// 59/100 of each axis (e.g. 1920x1080 -> 1132x637).
    Balanced,
    /// 1/2 of each axis (e.g. 1920x1080 -> 960x540).
    Performance,
}

impl UpscalePreset {
    /// Exact render-scale ratio: always below 1 so the render target stays
    /// below the presentation size on every axis.
    pub fn ratio(self) -> (u32, u32) {
        match self {
            Self::Ultra => (77, 100),
            Self::Quality => (2, 3),
            Self::Balanced => (59, 100),
            Self::Performance => (1, 2),
        }
    }
}

// ---------------------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------------------

/// Temporal plumbing failures: typed, no panics on caller data.
#[derive(Clone, PartialEq, Debug)]
pub enum TemporalError {
    /// Zero-area presentation size.
    ZeroArea,
    /// Presentation dimension exceeds [`MAX_CAPTURE_DIM`].
    TooLarge { width: u32, height: u32 },
    /// Pixel count differs from `width * height`.
    LengthMismatch { expected: usize, got: usize },
    /// A texel is NaN/infinite; index carried.
    NonFinite(usize),
}

impl fmt::Display for TemporalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroArea => write!(f, "temporal presentation size is zero"),
            Self::TooLarge { width, height } => {
                write!(f, "temporal {width}x{height} exceeds {MAX_CAPTURE_DIM}")
            }
            Self::LengthMismatch { expected, got } => {
                write!(f, "temporal pixels {got} differ from dims {expected}")
            }
            Self::NonFinite(i) => write!(f, "temporal texel {i} is not finite"),
        }
    }
}

impl std::error::Error for TemporalError {}

// ---------------------------------------------------------------------------
// Targets: presentation size in, sub-presentation render size out.
// ---------------------------------------------------------------------------

/// Render/present target pair: the GPU renders at `render_*` and presents
/// at `present_*`. Render is strictly below present on both axes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TemporalTargets {
    /// Presentation width in pixels (e.g. 1920).
    pub present_w: u32,
    /// Presentation height in pixels (e.g. 1080).
    pub present_h: u32,
    /// Render width in pixels, derived from the preset ratio.
    pub render_w: u32,
    /// Render height in pixels, derived from the preset ratio.
    pub render_h: u32,
    /// Preset that derived the render size.
    pub preset: UpscalePreset,
}

impl TemporalTargets {
    /// Derive the render size from the presentation size and preset with
    /// integer floor math (`render = present * num / den`, at least 1).
    /// Rejects zero areas and oversized presentations before any math.
    pub fn derive(
        present_w: u32,
        present_h: u32,
        preset: UpscalePreset,
    ) -> Result<Self, TemporalError> {
        if present_w == 0 || present_h == 0 {
            return Err(TemporalError::ZeroArea);
        }
        if present_w > MAX_CAPTURE_DIM || present_h > MAX_CAPTURE_DIM {
            return Err(TemporalError::TooLarge {
                width: present_w,
                height: present_h,
            });
        }
        let (num, den) = preset.ratio();
        let scale = |p: u32| ((u64::from(p) * u64::from(num) / u64::from(den)) as u32).max(1);
        Ok(Self {
            present_w,
            present_h,
            render_w: scale(present_w),
            render_h: scale(present_h),
            preset,
        })
    }

    /// Render pixel count without `u32` overflow (u64).
    pub fn render_pixels(&self) -> u64 {
        u64::from(self.render_w) * u64::from(self.render_h)
    }
}

// ---------------------------------------------------------------------------
// History buffers: exact-sized color + depth stores at the render size.
// ---------------------------------------------------------------------------

/// Bytes per RGBA32F texel (4 x f32 LE).
pub const HISTORY_COLOR_BPT: u64 = 16;
/// Bytes per R32F depth texel.
pub const HISTORY_DEPTH_BPT: u64 = 4;

/// Double-buffered temporal history at the render size: current and
/// previous linear-RGBA frames plus one depth plane. The real TAA resolve
/// reads `previous`; [`HistoryBuffer::swap`] promotes `current` after each
/// presented frame.
#[derive(Clone, PartialEq, Debug)]
pub struct HistoryBuffer {
    /// Render width in pixels.
    pub width: u32,
    /// Render height in pixels.
    pub height: u32,
    /// `width * height` linear-RGBA texels of the frame being built.
    pub current: Vec<[f32; 4]>,
    /// `width * height` linear-RGBA texels of the previous frame.
    pub previous: Vec<[f32; 4]>,
    /// `width * height` view-depth texels in meters.
    pub depth: Vec<f32>,
}

impl HistoryBuffer {
    /// Allocate zeroed history at the targets' render size. Lens are exactly
    /// `render_w * render_h`; sizes are bounded by [`MAX_CAPTURE_DIM`] via
    /// [`TemporalTargets::derive`], so allocation size is always finite.
    pub fn new(targets: &TemporalTargets) -> Self {
        let n = targets.render_pixels() as usize;
        Self {
            width: targets.render_w,
            height: targets.render_h,
            current: vec![[0.0; 4]; n],
            previous: vec![[0.0; 4]; n],
            depth: vec![0.0; n],
        }
    }

    /// Exact texel count (`width * height`).
    pub fn len(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Whether the buffer holds no texels (only when a dimension is 0,
    /// which [`TemporalTargets::derive`] never produces).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Exact color-plane byte size (`len * 16`).
    pub fn color_byte_len(&self) -> u64 {
        self.len() as u64 * HISTORY_COLOR_BPT
    }

    /// Exact depth-plane byte size (`len * 4`).
    pub fn depth_byte_len(&self) -> u64 {
        self.len() as u64 * HISTORY_DEPTH_BPT
    }

    /// Promote the built frame: swaps current/previous, keeping every texel.
    pub fn swap(&mut self) {
        std::mem::swap(&mut self.current, &mut self.previous);
    }

    /// Write one current-frame texel; out-of-range indices fail loudly.
    pub fn write(&mut self, index: usize, texel: [f32; 4]) -> Result<(), TemporalError> {
        if index >= self.current.len() {
            return Err(TemporalError::LengthMismatch {
                expected: self.current.len(),
                got: index + 1,
            });
        }
        if !texel.iter().all(|c| c.is_finite()) {
            return Err(TemporalError::NonFinite(index));
        }
        self.current[index] = texel;
        Ok(())
    }

    /// Read one previous-frame texel; out-of-range indices fail loudly.
    pub fn read_previous(&self, index: usize) -> Result<[f32; 4], TemporalError> {
        self.previous
            .get(index)
            .copied()
            .ok_or(TemporalError::LengthMismatch {
                expected: self.previous.len(),
                got: index + 1,
            })
    }
}

// ---------------------------------------------------------------------------
// Jitter uniform wiring: R4 Halton sequence at the render size.
// ---------------------------------------------------------------------------

/// Per-frame temporal uniform: the R4 [`TaaJitter`] offset evaluated at the
/// render size, in both pixel and NDC units, plus the projection hook the
/// `v1_gpu` pass consumes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TemporalUniform {
    /// Frame index this uniform was built for.
    pub frame: u64,
    /// Jitter in pixels, centered on `[-0.5, 0.5)` (== R4 offset).
    pub jitter_pixels: [f32; 2],
    /// Jitter in NDC (`pixels * 2 / res`, == R4 NDC at the render size).
    pub jitter_ndc: [f32; 2],
}

impl TemporalUniform {
    /// Build for `frame` at the render size. Zero render areas (suspended
    /// swap path) yield zero jitter, matching the R4 convention.
    pub fn for_frame(frame: u64, render_w: u32, render_h: u32) -> Self {
        Self {
            frame,
            jitter_pixels: TaaJitter::offset(frame),
            jitter_ndc: TaaJitter::ndc(frame, render_w, render_h),
        }
    }

    /// Jitter the projection matrix for this frame (R4 perspective terms).
    pub fn apply_to_proj(&self, proj: glam::Mat4) -> glam::Mat4 {
        TaaJitter::apply_to_proj(proj, self.jitter_ndc)
    }

    /// GPU uniform `[jx_px, jy_px, jx_ndc, jy_ndc]` for the resolve pass.
    pub fn to_uniform(&self) -> [f32; 4] {
        [
            self.jitter_pixels[0],
            self.jitter_pixels[1],
            self.jitter_ndc[0],
            self.jitter_ndc[1],
        ]
    }

    /// 16-byte little-endian image of [`to_uniform`](Self::to_uniform).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16);
        for c in self.to_uniform() {
            out.extend_from_slice(&c.to_le_bytes());
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Pipeline: targets + history + frame counter in one deterministic handle.
// ---------------------------------------------------------------------------

/// Frame assembly handle for PA1/GM1: owns the targets and history, hands
/// out the per-frame jitter uniform, and swaps history on present.
#[derive(Clone, PartialEq, Debug)]
pub struct TemporalPipeline {
    /// Render/present sizes.
    pub targets: TemporalTargets,
    /// Color + depth history at the render size.
    pub history: HistoryBuffer,
    /// Next frame index (monotonic; jitter depends only on the index).
    pub frame: u64,
}

impl TemporalPipeline {
    /// Build from the presentation size and preset with zeroed history.
    pub fn new(
        present_w: u32,
        present_h: u32,
        preset: UpscalePreset,
    ) -> Result<Self, TemporalError> {
        let targets = TemporalTargets::derive(present_w, present_h, preset)?;
        let history = HistoryBuffer::new(&targets);
        Ok(Self {
            targets,
            history,
            frame: 0,
        })
    }

    /// Jitter uniform for the current frame at the render size.
    pub fn begin_frame(&self) -> TemporalUniform {
        TemporalUniform::for_frame(self.frame, self.targets.render_w, self.targets.render_h)
    }

    /// Present: promote history and advance the frame index.
    pub fn end_frame(&mut self) {
        self.history.swap();
        self.frame = self.frame.wrapping_add(1);
    }
}

// ---------------------------------------------------------------------------
// Bilinear reference the live path must match exactly.
// ---------------------------------------------------------------------------

/// Test-only bilinear reference: what `post::taa_upsample` matches.
/// Clamp-to-edge; same inputs give bit-identical outputs.
pub fn upscale_bilinear(
    src: &[[f32; 3]],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Result<Vec<[f32; 3]>, TemporalError> {
    for (dim, name) in [
        (src_w, "src_w"),
        (src_h, "src_h"),
        (dst_w, "dst_w"),
        (dst_h, "dst_h"),
    ] {
        if dim == 0 {
            let _ = name;
            return Err(TemporalError::ZeroArea);
        }
        if dim > MAX_CAPTURE_DIM {
            return Err(TemporalError::TooLarge {
                width: src_w.max(dst_w),
                height: src_h.max(dst_h),
            });
        }
    }
    if src.len() != src_w as usize * src_h as usize {
        return Err(TemporalError::LengthMismatch {
            expected: src_w as usize * src_h as usize,
            got: src.len(),
        });
    }
    for (i, p) in src.iter().enumerate() {
        if !p.iter().all(|c| c.is_finite()) {
            return Err(TemporalError::NonFinite(i));
        }
    }
    let sw = src_w as usize;
    let sh = src_h as usize;
    let sx = src_w as f32 / dst_w as f32;
    let sy = src_h as f32 / dst_h as f32;
    let mut out = Vec::with_capacity(dst_w as usize * dst_h as usize);
    for y in 0..dst_h as usize {
        for x in 0..dst_w as usize {
            // Pixel-center mapping with clamp-to-edge: exact identity when
            // src and dst sizes match (weight collapses to one texel).
            let fx = (x as f32 + 0.5) * sx - 0.5;
            let fy = (y as f32 + 0.5) * sy - 0.5;
            let x0 = (fx.floor() as i32).clamp(0, src_w as i32 - 1) as usize;
            let y0 = (fy.floor() as i32).clamp(0, src_h as i32 - 1) as usize;
            let x1 = (x0 + 1).min(sw - 1);
            let y1 = (y0 + 1).min(sh - 1);
            let tx = (fx - x0 as f32).clamp(0.0, 1.0);
            let ty = (fy - y0 as f32).clamp(0.0, 1.0);
            let a = src[y0 * sw + x0];
            let b = src[y0 * sw + x1];
            let c = src[y1 * sw + x0];
            let d = src[y1 * sw + x1];
            let mix = |p: [f32; 3], q: [f32; 3], t: f32| {
                [
                    p[0] + (q[0] - p[0]) * t,
                    p[1] + (q[1] - p[1]) * t,
                    p[2] + (q[2] - p[2]) * t,
                ]
            };
            out.push(mix(mix(a, b, tx), mix(c, d, tx), ty));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests: sizes derive below present, R4 jitter wiring, exact history
// sizing, determinism x2, stub behavior, failure cases.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_sizes_derive_below_present() {
        // Exact assertion: integer floor math from the preset ratios, so the
        // reference GPU always renders below 1080p and presents 1080p.
        let cases = [
            (UpscalePreset::Ultra, 1478u32, 831u32),
            (UpscalePreset::Quality, 1280, 720),
            (UpscalePreset::Balanced, 1132, 637),
            (UpscalePreset::Performance, 960, 540),
        ];
        for (preset, want_w, want_h) in cases {
            let t = TemporalTargets::derive(1920, 1080, preset).expect("1080p derives");
            assert_eq!(
                (t.render_w, t.render_h),
                (want_w, want_h),
                "preset {preset:?} must derive exactly"
            );
            assert!(
                t.render_w < t.present_w && t.render_h < t.present_h,
                "render {w}x{h} must stay below present 1920x1080",
                w = t.render_w,
                h = t.render_h
            );
            assert_eq!(
                t.render_pixels(),
                u64::from(want_w) * u64::from(want_h),
                "pixel count is the exact product"
            );
        }
        // Smaller presentations scale the same way (Quality 2/3).
        let small =
            TemporalTargets::derive(960, 540, UpscalePreset::Quality).expect("small derives");
        assert_eq!((small.render_w, small.render_h), (640, 360));
        // Same inputs twice give identical targets (derivation determinism).
        assert_eq!(
            TemporalTargets::derive(1920, 1080, UpscalePreset::Quality),
            TemporalTargets::derive(1920, 1080, UpscalePreset::Quality)
        );
    }

    #[test]
    fn jitter_integrates_with_r4_halton_sequence() {
        // Exact assertion: the uniform IS the R4 sequence at the render
        // size — pixels equal TaaJitter::offset, NDC equals TaaJitter::ndc,
        // and projection wiring touches only the perspective terms.
        for frame in [0u64, 1, 2, 7, 15, 10_000] {
            let u = TemporalUniform::for_frame(frame, 1280, 720);
            assert_eq!(u.frame, frame);
            assert_eq!(
                u.jitter_pixels,
                TaaJitter::offset(frame),
                "frame {frame} pixels reuse the R4 offset"
            );
            assert_eq!(
                u.jitter_ndc,
                TaaJitter::ndc(frame, 1280, 720),
                "frame {frame} NDC reuses the R4 scaling at render size"
            );
            let proj = glam::Mat4::IDENTITY;
            assert_eq!(
                u.apply_to_proj(proj),
                TaaJitter::apply_to_proj(proj, u.jitter_ndc),
                "frame {frame} projection wiring matches R4"
            );
            assert_eq!(
                u.to_uniform(),
                [
                    u.jitter_pixels[0],
                    u.jitter_pixels[1],
                    u.jitter_ndc[0],
                    u.jitter_ndc[1]
                ]
            );
            assert_eq!(u.to_bytes().len(), 16, "uniform image is 16 bytes");
        }
        // Suspended swap path (zero render area) yields zero NDC jitter.
        let suspended = TemporalUniform::for_frame(3, 0, 720);
        assert_eq!(suspended.jitter_ndc, [0.0, 0.0]);
    }

    #[test]
    fn history_buffer_sizing_is_exact() {
        // Exact assertion: lens equal render_w * render_h and byte sizes
        // equal len * bytes-per-texel — the allocation the pass binds.
        let targets =
            TemporalTargets::derive(1920, 1080, UpscalePreset::Quality).expect("targets derive");
        let mut h = HistoryBuffer::new(&targets);
        assert_eq!((h.width, h.height), (1280, 720));
        assert_eq!(h.len(), 1280 * 720);
        assert_eq!(h.current.len(), h.len());
        assert_eq!(h.previous.len(), h.len());
        assert_eq!(h.depth.len(), h.len());
        assert_eq!(h.color_byte_len(), h.len() as u64 * HISTORY_COLOR_BPT);
        assert_eq!(h.depth_byte_len(), h.len() as u64 * HISTORY_DEPTH_BPT);
        // Swap promotes the built frame bit-exactly.
        h.current[7] = [1.0, 0.5, 0.25, 1.0];
        h.swap();
        assert_eq!(h.previous[7], [1.0, 0.5, 0.25, 1.0]);
        assert_eq!(
            h.read_previous(7).expect("reads back"),
            [1.0, 0.5, 0.25, 1.0]
        );
        // Write/read round trip on the current plane.
        h.write(9, [0.1, 0.2, 0.3, 1.0]).expect("writes");
        h.swap();
        assert_eq!(
            h.read_previous(9).expect("reads back"),
            [0.1, 0.2, 0.3, 1.0]
        );
    }

    #[test]
    fn temporal_pipeline_is_deterministic_x2() {
        // Same complete inputs twice give bit-identical uniforms, history
        // and stub outputs (x2).
        let run = || {
            let mut p =
                TemporalPipeline::new(1920, 1080, UpscalePreset::Quality).expect("pipeline builds");
            let mut uniform_bytes = Vec::new();
            for (i, texel) in [[0.4, 0.3, 0.2, 1.0], [0.9, 0.1, 0.5, 1.0]]
                .iter()
                .enumerate()
            {
                let u = p.begin_frame();
                uniform_bytes.extend_from_slice(&u.to_bytes());
                p.history.write(i, *texel).expect("writes");
                p.end_frame();
            }
            let src = vec![[0.5, 0.25, 0.125]; 1280 * 720];
            let up = upscale_bilinear(&src, 1280, 720, 1920, 1080).expect("upscales");
            (p, uniform_bytes, up)
        };
        let (p1, b1, u1) = run();
        let (p2, b2, u2) = run();
        assert_eq!(p1, p2, "pipeline state is bit-identical twice");
        assert_eq!(b1, b2, "uniform bytes are bit-identical twice");
        assert_eq!(u1, u2, "stub upscale output is bit-identical twice");
        assert_eq!(p1.frame, 2, "two presented frames advance the counter");
    }

    #[test]
    fn stub_upscaler_behaves_exactly() {
        // Same-size upscale is the bit-exact identity (weights collapse).
        let src = vec![[0.3, 0.6, 0.9]; 4 * 3];
        let same = upscale_bilinear(&src, 4, 3, 4, 3).expect("same size upscales");
        assert_eq!(same, src, "same-size stub upscale is the identity");
        // Solid colors survive any upscale unchanged (no invented detail).
        let solid = vec![[0.7, 0.2, 0.1]; 2 * 2];
        let big = upscale_bilinear(&solid, 2, 2, 8, 6).expect("solid upscales");
        assert_eq!(big.len(), 8 * 6);
        assert!(
            big.iter().all(|p| *p == [0.7, 0.2, 0.1]),
            "solid color must survive the stub unchanged"
        );
        // 1x1 replicates to every destination texel.
        let one = upscale_bilinear(&[[1.0, 0.5, 0.0]], 1, 1, 3, 2).expect("1x1 upscales");
        assert_eq!(one, vec![[1.0, 0.5, 0.0]; 6]);
        // Output length is exactly dst_w * dst_h with finite texels.
        let grad: Vec<[f32; 3]> = (0..64).map(|i| [i as f32 * 0.01, 0.2, 0.05]).collect();
        let out = upscale_bilinear(&grad, 8, 8, 16, 16).expect("grad upscales");
        assert_eq!(out.len(), 256);
        assert!(out.iter().all(|p| p.iter().all(|c| c.is_finite())));
    }

    #[test]
    fn invalid_inputs_fail_loudly() {
        assert_eq!(
            TemporalTargets::derive(0, 1080, UpscalePreset::Quality),
            Err(TemporalError::ZeroArea)
        );
        assert_eq!(
            TemporalTargets::derive(1920, 0, UpscalePreset::Quality),
            Err(TemporalError::ZeroArea)
        );
        assert!(matches!(
            TemporalTargets::derive(4096, 1080, UpscalePreset::Quality),
            Err(TemporalError::TooLarge { .. })
        ));
        assert!(matches!(
            TemporalPipeline::new(0, 0, UpscalePreset::Performance),
            Err(TemporalError::ZeroArea)
        ));
        let mut h = HistoryBuffer::new(
            &TemporalTargets::derive(64, 64, UpscalePreset::Performance).expect("targets derive"),
        );
        assert!(matches!(
            h.write(h.len(), [1.0; 4]),
            Err(TemporalError::LengthMismatch { .. })
        ));
        assert!(matches!(
            h.write(0, [f32::NAN, 0.0, 0.0, 1.0]),
            Err(TemporalError::NonFinite(0))
        ));
        assert!(matches!(
            h.read_previous(h.len()),
            Err(TemporalError::LengthMismatch { .. })
        ));
        let src = vec![[0.5, 0.5, 0.5]; 4 * 4];
        assert_eq!(
            upscale_bilinear(&src, 4, 4, 0, 4),
            Err(TemporalError::ZeroArea)
        );
        assert_eq!(
            upscale_bilinear(&src[..15], 4, 4, 8, 8),
            Err(TemporalError::LengthMismatch {
                expected: 16,
                got: 15
            })
        );
        let mut bad = src.clone();
        bad[3] = [f32::INFINITY, 0.0, 0.0];
        assert_eq!(
            upscale_bilinear(&bad, 4, 4, 8, 8),
            Err(TemporalError::NonFinite(3))
        );
    }
}

#[cfg(test)]
mod sl3_gate {
    //! Parity oracle that stays green now the module is test-only.
    use super::{upscale_bilinear, TemporalPipeline, TemporalTargets, UpscalePreset};

    #[test]
    fn sl3_r6_stub_parity_oracle_stays_green() {
        // 1920x1080 Quality derives exactly 1280x720 (integer floor math).
        let t = TemporalTargets::derive(1920, 1080, UpscalePreset::Quality).expect("derives");
        assert_eq!((t.render_w, t.render_h), (1280, 720));
        assert_eq!(t.render_pixels(), 1280 * 720);
        // Same-size upscale is the bit-exact identity.
        let src = vec![[0.3, 0.6, 0.9]; 4 * 3];
        assert_eq!(upscale_bilinear(&src, 4, 3, 4, 3).expect("same"), src);
        // Solid colors survive any upscale unchanged (48 texels out).
        let solid = vec![[0.7, 0.2, 0.1]; 2 * 2];
        let big = upscale_bilinear(&solid, 2, 2, 8, 6).expect("solid");
        assert_eq!(big.len(), 48);
        assert!(big.iter().all(|p| *p == [0.7, 0.2, 0.1]));
        // One pipeline run is bit-identical twice; one frame advances it.
        let run = || {
            let mut p =
                TemporalPipeline::new(1920, 1080, UpscalePreset::Quality).expect("pipeline");
            let bytes = p.begin_frame().to_bytes();
            p.end_frame();
            (p.frame, bytes)
        };
        assert_eq!(run(), run());
        assert_eq!(run().0, 1);
    }
}
