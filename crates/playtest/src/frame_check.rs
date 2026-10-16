//! Frame-grounded checks: black/white screens, missing meshes, near-plane clipping.
//!
//! Consumer: the nightly playtest job and the visual gates call
//! [`check_scripted`] for scripted scenes and [`check_frame`] for raw RGBA8
//! from any offscreen path (the renderer's twin readback feeds the same shape).
//! The caller owns every byte; this module only borrows.
//! Units: pixels for sizes and coverage, plain counts for draws, permille
//! (`0..=1000`) for coverage fractions, counts for near-plane slices.
//! Determinism: the same frame twice gives the same verdict; digests are
//! FNV-1a hex with no wall clock or map order inside.
//! Errors: each detection is a [`FrameFault`] carrying [`Evidence`]
//! (capture digest plus metric); malformed inputs are `BadCapture`.

use std::fmt;

use engine_arena::seed as arena_seed;
use engine_core::hash::GOLDEN_GAMMA;
use engine_core::hash_bytes;
use engine_core::vec3::cross3;

/// Largest accepted frame dimension in pixels. Allocation bound for analysis
/// buffers (`2048 x 2048 x 4 B` = 16 MiB); the capture-gated test asserts it
/// equals the render capture ceiling so the two never drift.
pub const MAX_FRAME_DIM: u32 = 2048;

/// Hex content tag over capture bytes (FNV-1a 64, 16 lowercase hex chars).
/// Names captures inside evidence; not a cryptographic digest.
pub fn digest_hex(bytes: &[u8]) -> String {
    format!("{:016x}", hash_bytes(bytes))
}

/// Plain-data perspective camera for near-plane math. Meters, radians,
/// right-handed with `-Z` forward, mirroring the render camera layout.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FrameCamera {
    /// Camera position in meters.
    pub eye: [f32; 3],
    /// Look-at point in meters.
    pub target: [f32; 3],
    /// Up direction (need not be unit, must not parallel the view).
    pub up: [f32; 3],
    /// Vertical field of view in radians.
    pub fov_y: f32,
    /// Width over height.
    pub aspect: f32,
    /// Near clip in meters, `> 0`.
    pub near: f32,
    /// Far clip in meters, `> near`.
    pub far: f32,
}

/// Detection thresholds. Defaults catch a fully dark or fully bright frame,
/// demand 0.5% of pixels covered when geometry is expected, and refuse any
/// near-plane cut.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Thresholds {
    /// Largest per-pixel max channel still counted as black (`0..=255`).
    pub black_max: u8,
    /// Smallest per-pixel min channel still counted as white (`0..=255`).
    pub white_min: u8,
    /// Smallest per-pixel max channel counted as covered geometry.
    pub cover_min: u8,
    /// Smallest covered fraction in permille when draws are expected.
    pub min_cover_permille: u16,
    /// Largest accepted near-plane slice count.
    pub max_near_slices: u32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            black_max: 8,
            white_min: 247,
            cover_min: 32,
            min_cover_permille: 5,
            max_near_slices: 0,
        }
    }
}

/// Evidence carried by every [`FrameFault`]: which capture plus the metric.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Evidence {
    /// [`digest_hex`] over the capture header plus pixels.
    pub digest: String,
    /// Fault metric: max channel (black), min channel (white), covered
    /// pixels (missing) or slice count (clipping).
    pub metric: u64,
    /// Short human- and machine-readable detail with threshold and counts.
    pub detail: String,
}

/// Frame-grounded detections in check order (black, white, missing, clipping,
/// then the sequence-level frozen window, which never fires per frame).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FrameFault {
    /// Every pixel at or below `black_max`.
    BlackScreen(Evidence),
    /// Every channel of every pixel at or above `white_min`.
    WhiteScreen(Evidence),
    /// Geometry expected but the draw list is empty or nothing is covered.
    MissingMesh(Evidence),
    /// Near-plane slice count over the threshold.
    Clipping(Evidence),
    /// Three or more consecutive captures share one digest: the capture path
    /// re-emits identical bytes (frozen window; sequence-level only).
    Frozen(Evidence),
    /// Malformed capture (zero area, over bound, byte-length mismatch).
    BadCapture(String),
}

impl fmt::Display for FrameFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BlackScreen(e) => write!(f, "black screen {}: {}", e.digest, e.detail),
            Self::WhiteScreen(e) => write!(f, "white screen {}: {}", e.digest, e.detail),
            Self::MissingMesh(e) => write!(f, "missing mesh {}: {}", e.digest, e.detail),
            Self::Clipping(e) => write!(f, "clipping {}: {}", e.digest, e.detail),
            Self::Frozen(e) => write!(f, "frozen sequence {}: {}", e.digest, e.detail),
            Self::BadCapture(reason) => write!(f, "bad capture: {reason}"),
        }
    }
}

impl std::error::Error for FrameFault {}

/// One captured frame for [`check_frame`]: raw RGBA8 plus the draw and clip
/// counts the capture path reported. The renderer's twin readback maps directly.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CapturedFrame {
    /// Width in pixels, `1..=MAX_FRAME_DIM`.
    pub width: u32,
    /// Height in pixels, `1..=MAX_FRAME_DIM`.
    pub height: u32,
    /// Exactly `width * height * 4` bytes, row-major from the top.
    pub rgba: Vec<u8>,
    /// Draws submitted this frame.
    pub draws: u32,
    /// Draws the scene expected (geometry on screen when nonzero).
    pub expected_draws: u32,
    /// Triangles straddling the near plane (see [`count_near_slices`]).
    pub near_slices: u32,
}

/// Clean-frame metrics returned by [`check_frame`] on a pass.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FrameMetrics {
    /// [`digest_hex`] over the capture header plus pixels.
    pub digest: String,
    /// Covered pixel count (any channel at or above `cover_min`).
    pub covered_pixels: u64,
    /// Covered fraction in permille.
    pub cover_permille: u64,
    /// Draws submitted.
    pub draws: u32,
    /// Near-plane slice count.
    pub near_slices: u32,
}

/// Largest visible per-pixel channel value over the frame, `0..=255`.
/// Alpha never counts: captures are opaque and fault semantics are about
/// visible light, so a dark frame with full alpha is still black.
pub fn max_channel(rgba: &[u8]) -> u8 {
    let mut max = 0u8;
    for pixel in rgba.chunks_exact(4) {
        let peak = pixel[0].max(pixel[1]).max(pixel[2]);
        if peak > max {
            max = peak;
            if max == 255 {
                break;
            }
        }
    }
    max
}

/// Smallest visible per-pixel channel value over the frame, `0..=255`.
/// Alpha never counts: a bright frame with zero alpha is still white.
/// Empty input reports 255 so only [`check_frame`] decides malformed input.
pub fn min_channel(rgba: &[u8]) -> u8 {
    let mut min = 255u8;
    for pixel in rgba.chunks_exact(4) {
        let trough = pixel[0].min(pixel[1]).min(pixel[2]);
        if trough < min {
            min = trough;
            if min == 0 {
                break;
            }
        }
    }
    min
}

/// Pixels with any channel at or above `cover_min`: geometry over the dark
/// backdrop. Counts pixels, so `rgba.len()` must be a multiple of 4.
pub fn covered_pixels(rgba: &[u8], cover_min: u8) -> usize {
    let mut covered = 0usize;
    for pixel in rgba.chunks_exact(4) {
        if pixel[0] >= cover_min || pixel[1] >= cover_min || pixel[2] >= cover_min {
            covered += 1;
        }
    }
    covered
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(v: [f32; 3]) -> Option<[f32; 3]> {
    let len2 = dot(v, v);
    if !len2.is_finite() || len2 < 1e-18 {
        return None;
    }
    let len = len2.sqrt();
    if !len.is_finite() {
        return None;
    }
    Some([v[0] / len, v[1] / len, v[2] / len])
}

/// View depth of `point` along the camera forward axis, positive in front.
fn view_depth(eye: [f32; 3], forward: [f32; 3], point: [f32; 3]) -> Option<f32> {
    if !point.iter().all(|c| c.is_finite()) {
        return None;
    }
    let d = dot(sub(point, eye), forward);
    if d.is_finite() {
        Some(d)
    } else {
        None
    }
}

/// Triangles straddling the near plane: smallest depth below `near` while
/// the largest is at or past it, with the triangle in front of the eye.
/// Degenerate cameras, bad clip ranges and bad triangles count zero (camera
/// validity is the capture path's job; this never faults on garbage).
/// Iteration is index order with a saturating sum, so results repeat.
pub fn count_near_slices(camera: &FrameCamera, positions: &[[f32; 3]], indices: &[u32]) -> u32 {
    if !camera.near.is_finite()
        || camera.near <= 0.0
        || !camera.far.is_finite()
        || camera.far <= camera.near
    {
        return 0;
    }
    if !camera.eye.iter().all(|c| c.is_finite())
        || !camera.target.iter().all(|c| c.is_finite())
        || !camera.up.iter().all(|c| c.is_finite())
    {
        return 0;
    }
    let view = sub(camera.target, camera.eye);
    let (Some(forward), Some(_side)) = (norm(view), norm(cross3(view, camera.up))) else {
        return 0;
    };
    let mut slices = 0u32;
    for tri in indices.chunks_exact(3) {
        let mut depths = [0.0f32; 3];
        let mut valid = true;
        for (slot, index) in depths.iter_mut().zip(tri.iter()) {
            let Some(point) = positions.get(*index as usize) else {
                valid = false;
                break;
            };
            let Some(depth) = view_depth(camera.eye, forward, *point) else {
                valid = false;
                break;
            };
            *slot = depth;
        }
        if !valid {
            continue;
        }
        let lo = depths[0].min(depths[1]).min(depths[2]);
        let hi = depths[0].max(depths[1]).max(depths[2]);
        if lo < camera.near && hi >= camera.near {
            slices = slices.saturating_add(1);
        }
    }
    slices
}

/// Run every frame check in fixed order (black, white, missing, clipping) and
/// return the clean metrics or the first fault, each carrying the capture
/// digest plus its metric.
pub fn check_frame(
    frame: &CapturedFrame,
    thresholds: &Thresholds,
) -> Result<FrameMetrics, FrameFault> {
    if frame.width == 0 || frame.height == 0 {
        return Err(FrameFault::BadCapture(format!(
            "zero area {}x{}",
            frame.width, frame.height
        )));
    }
    if frame.width > MAX_FRAME_DIM || frame.height > MAX_FRAME_DIM {
        return Err(FrameFault::BadCapture(format!(
            "size {}x{} exceeds {MAX_FRAME_DIM}",
            frame.width, frame.height
        )));
    }
    let pixels = u64::from(frame.width) * u64::from(frame.height);
    if frame.rgba.len() as u64 != pixels * 4 {
        return Err(FrameFault::BadCapture(format!(
            "byte length {} differs from {}x{} RGBA8",
            frame.rgba.len(),
            frame.width,
            frame.height
        )));
    }
    let mut header = Vec::with_capacity(20);
    header.extend_from_slice(&frame.width.to_le_bytes());
    header.extend_from_slice(&frame.height.to_le_bytes());
    header.extend_from_slice(&frame.draws.to_le_bytes());
    header.extend_from_slice(&frame.expected_draws.to_le_bytes());
    header.extend_from_slice(&frame.near_slices.to_le_bytes());
    let mut tagged = header;
    tagged.extend_from_slice(&frame.rgba);
    let digest = digest_hex(&tagged);

    let max = max_channel(&frame.rgba);
    if max <= thresholds.black_max {
        return Err(FrameFault::BlackScreen(Evidence {
            digest,
            metric: u64::from(max),
            detail: format!(
                "max channel {max} at or below {} over {pixels} pixels",
                thresholds.black_max
            ),
        }));
    }
    let min = min_channel(&frame.rgba);
    if min >= thresholds.white_min {
        return Err(FrameFault::WhiteScreen(Evidence {
            digest: digest.clone(),
            metric: u64::from(min),
            detail: format!(
                "min channel {min} at or above {} over {pixels} pixels",
                thresholds.white_min
            ),
        }));
    }
    let covered = covered_pixels(&frame.rgba, thresholds.cover_min) as u64;
    let cover_permille = covered * 1000 / pixels;
    if frame.expected_draws > 0
        && (frame.draws == 0 || cover_permille < u64::from(thresholds.min_cover_permille))
    {
        return Err(FrameFault::MissingMesh(Evidence {
            digest,
            metric: covered,
            detail: format!(
                "draws {}/{} expected, {covered} covered pixels ({cover_permille} permille, need {})",
                frame.draws, frame.expected_draws, thresholds.min_cover_permille
            ),
        }));
    }
    if frame.near_slices > thresholds.max_near_slices {
        return Err(FrameFault::Clipping(Evidence {
            digest,
            metric: u64::from(frame.near_slices),
            detail: format!(
                "{} near-plane slices over threshold {}",
                frame.near_slices, thresholds.max_near_slices
            ),
        }));
    }
    Ok(FrameMetrics {
        digest,
        covered_pixels: covered,
        cover_permille,
        draws: frame.draws,
        near_slices: frame.near_slices,
    })
}

/// Scripted scene under test: one seeded fault per variant plus the clean
/// reference. Seeds vary the fault presentation, never its detectability.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Script {
    /// Lit geometry over a dark backdrop with draws submitted.
    Clean,
    /// Fully dark frame with draws submitted.
    Black,
    /// Fully bright frame with draws submitted.
    White,
    /// Dark backdrop with geometry expected (empty or uncovered draws).
    MissingMesh,
    /// Lit geometry with near-plane slices over the threshold.
    Clipped,
}

/// Fill every pixel with `level` (opaque): shared by the seeded black/white
/// faults so both stay the same shape with mirrored levels.
fn paint_solid(rgba: &mut [u8], level: u8) {
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[0] = level;
        pixel[1] = level;
        pixel[2] = level;
        pixel[3] = 255;
    }
}

/// Deterministic synthetic capture for `script`: dark-slate backdrop, a
/// copper-toned centered box for lit variants, seeded jitter on the box
/// origin and fault depth. Test helper, so sizes clamp to `8..=256`.
/// Even `MissingMesh` seeds submit zero draws; odd seeds submit draws with
/// zero covered pixels, covering both detector arms.
pub fn scripted(script: Script, width: u32, height: u32, seed: u64) -> CapturedFrame {
    let width = width.clamp(8, 256);
    let height = height.clamp(8, 256);
    let w = width as usize;
    let h = height as usize;
    let mut rng = seed.wrapping_add(GOLDEN_GAMMA);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        let t = y as f32 / (h.max(2) - 1) as f32;
        let bg = [
            (7.0 + 9.0 * t) as u8,
            (8.0 + 9.0 * t) as u8,
            (15.0 + 11.0 * t) as u8,
        ];
        for x in 0..w {
            let o = (y * w + x) * 4;
            rgba[o] = bg[0];
            rgba[o + 1] = bg[1];
            rgba[o + 2] = bg[2];
            rgba[o + 3] = 255;
        }
    }
    let mut paint_box = |rgba: &mut [u8], color: [u8; 3]| {
        let jx = (arena_seed::next(&mut rng) % (w as u64 / 8 + 1)) as usize;
        let jy = (arena_seed::next(&mut rng) % (h as u64 / 8 + 1)) as usize;
        let x0 = (w / 4).saturating_sub(jx / 2);
        let y0 = (h / 4).saturating_sub(jy / 2);
        for y in y0..(y0 + w / 2).min(h) {
            for x in x0..(x0 + w / 2).min(w) {
                let o = (y * w + x) * 4;
                rgba[o] = color[0];
                rgba[o + 1] = color[1];
                rgba[o + 2] = color[2];
            }
        }
    };
    match script {
        Script::Clean => {
            paint_box(&mut rgba, [184, 115, 71]);
            CapturedFrame {
                width,
                height,
                rgba,
                draws: 2,
                expected_draws: 2,
                near_slices: 0,
            }
        }
        Script::Black => {
            let level = (seed % 4) as u8;
            paint_solid(&mut rgba, level);
            CapturedFrame {
                width,
                height,
                rgba,
                draws: 2,
                expected_draws: 2,
                near_slices: 0,
            }
        }
        Script::White => {
            let residue = u8::try_from(seed % 4).unwrap_or(3);
            paint_solid(&mut rgba, 255u8.saturating_sub(residue));
            CapturedFrame {
                width,
                height,
                rgba,
                draws: 2,
                expected_draws: 2,
                near_slices: 0,
            }
        }
        Script::MissingMesh => CapturedFrame {
            width,
            height,
            rgba,
            draws: if seed.is_multiple_of(2) { 0 } else { 2 },
            expected_draws: 2,
            near_slices: 0,
        },
        Script::Clipped => {
            paint_box(&mut rgba, [184, 115, 71]);
            CapturedFrame {
                width,
                height,
                rgba,
                draws: 2,
                expected_draws: 2,
                near_slices: 1 + (seed % 4) as u32,
            }
        }
    }
}

/// One call for scripted scenes: build the seeded capture, run every check.
/// The nightly job runs each [`Script`] fault plus [`Script::Clean`] through
/// this and files a bug on any unexpected verdict.
pub fn check_scripted(
    script: Script,
    width: u32,
    height: u32,
    seed: u64,
    thresholds: &Thresholds,
) -> Result<FrameMetrics, FrameFault> {
    check_frame(&scripted(script, width, height, seed), thresholds)
}

/// Name under which proof invokes the white-screen gate. The orchestrator
/// runs the workspace test legs, which execute [`white_gate`]; the name lives
/// here (playtest side) so the gate keeps its name even before proof wires it.
pub const WHITE_GATE_NAME: &str = "qa4-white-screen-gate";

/// Proof-gate outcome: clean digest plus the caught fault digests. Empty
/// failures exactly when the gate passes; any unexpected verdict returns the
/// fault so proof fails on a seeded fault instead of passing silently.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WhiteGateOutcome {
    /// Clean reference digest (seed 7 at 96x64).
    pub clean_digest: String,
    /// Caught white-fault digests across seeds `0..8`.
    pub white_digests: Vec<String>,
    /// Caught black-fault digests across seeds `0..8`.
    pub black_digests: Vec<String>,
}

/// Proof gate: seeded white plus black faults are each caught while clean
/// arena/twin/play-step sized frames pass. Runs on synthetic captures so the
/// default (std-only) build proves it; the `capture` tests prove the same
/// faults stay distinct on real offscreen PNGs.
pub fn white_gate() -> Result<WhiteGateOutcome, FrameFault> {
    let thresholds = Thresholds::default();
    let clean = check_scripted(Script::Clean, 96, 64, 7, &thresholds)?.digest;
    for (width, height) in [(160, 120), (128, 96), (96, 64)] {
        check_scripted(Script::Clean, width, height, 7, &thresholds)?;
    }
    let mut white_digests = Vec::with_capacity(8);
    for seed in 0..8u64 {
        match check_scripted(Script::White, 96, 64, seed, &thresholds) {
            Err(FrameFault::WhiteScreen(evidence)) => white_digests.push(evidence.digest),
            Err(fault) => return Err(fault),
            Ok(metrics) => {
                return Err(FrameFault::BadCapture(format!(
                    "white seed {seed} passed as {}",
                    metrics.digest
                )));
            }
        }
    }
    let mut black_digests = Vec::with_capacity(8);
    for seed in 0..8u64 {
        match check_scripted(Script::Black, 96, 64, seed, &thresholds) {
            Err(FrameFault::BlackScreen(evidence)) => black_digests.push(evidence.digest),
            Err(fault) => return Err(fault),
            Ok(metrics) => {
                return Err(FrameFault::BadCapture(format!(
                    "black seed {seed} passed as {}",
                    metrics.digest
                )));
            }
        }
    }
    Ok(WhiteGateOutcome {
        clean_digest: clean,
        white_digests,
        black_digests,
    })
}

/// Frozen-sequence gate: a stuck capture path re-emits identical
/// bytes while the sim advances, so no single frame can name it frozen
/// (one frozen frame is pixel-identical to one clean frame) — the digest
/// window over the capture sequence does. Presentation/playtest only: the
/// tick never calls this; inputs are the FNV-1a hex digests [`check_frame`]
/// reports, integer-only, no wall clock and no map order inside.
pub const FROZEN_WINDOW: usize = 3;

/// Name under which proof invokes the frozen-sequence gate, mirroring
/// [`WHITE_GATE_NAME`] so the gate keeps its name even before proof wires it.
pub const FROZEN_GATE_NAME: &str = "qa14-frozen-sequence-gate";

/// Proof-gate outcome: the caught frozen digest plus its run length and the
/// motion-sequence unique count. Empty failures exactly when the gate
/// passes; a frozen sequence that passes as motion (or a motion sequence
/// that faults) returns the fault so proof fails loudly either way.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FrozenGateOutcome {
    /// The repeated digest of the 12-frame frozen seed-7 sequence.
    pub frozen_digest: String,
    /// Consecutive identical digests in the frozen sequence (12).
    pub frozen_frames: u64,
    /// Unique digests across the 12-frame motion sequence.
    pub motion_unique: u64,
}

/// Slide the 3-identical-digest window over per-frame digests in order and
/// fault on the first frozen window, carrying the repeated digest plus the
/// full run length. Shorter than the window, or motion with no 3-window,
/// passes: the gate faults on stuck captures, never on live ones.
pub fn check_digest_window(digests: &[String]) -> Result<(), FrameFault> {
    if digests.len() < FROZEN_WINDOW {
        return Ok(());
    }
    for (start, run) in digests.windows(FROZEN_WINDOW).enumerate() {
        if run[0] == run[1] && run[1] == run[2] {
            let mut length = FROZEN_WINDOW;
            while digests
                .get(start + length)
                .is_some_and(|next| *next == run[0])
            {
                length += 1;
            }
            let metric = u64::try_from(length).map_err(|_| {
                FrameFault::BadCapture(format!("frozen run length {length} overflows u64"))
            })?;
            return Err(FrameFault::Frozen(Evidence {
                digest: run[0].clone(),
                metric,
                detail: format!(
                    "{length} consecutive identical digests from frame {start} (window {FROZEN_WINDOW})"
                ),
            }));
        }
    }
    Ok(())
}

/// Proof gate: a 12-frame frozen seed-7 sequence (one seeded capture
/// re-emitted 12 times, as a stuck twin/play-step path would) is caught
/// exactly once, while a 12-frame motion sequence (seeds `0..12`) passes
/// with no frozen window. Runs on synthetic captures so the default
/// (std-only) build proves it.
pub fn frozen_gate() -> Result<FrozenGateOutcome, FrameFault> {
    let thresholds = Thresholds::default();
    let frozen: Vec<String> = (0..12)
        .map(|_| {
            check_scripted(Script::Clean, 96, 64, 7, &thresholds)
                .map(|report| report.digest)
                .map_err(|fault| {
                    FrameFault::BadCapture(format!("clean seed 7 fails per frame: {fault}"))
                })
        })
        .collect::<Result<Vec<String>, FrameFault>>()?;
    let frozen_digest = frozen[0].clone();
    if frozen.iter().any(|digest| *digest != frozen_digest) {
        return Err(FrameFault::BadCapture(format!(
            "frozen seed-7 source varies across 12 frames: {frozen:?}"
        )));
    }
    match check_digest_window(&frozen) {
        Err(FrameFault::Frozen(evidence)) => {
            if evidence.metric != 12 {
                return Err(FrameFault::BadCapture(format!(
                    "frozen run length {} is not 12",
                    evidence.metric
                )));
            }
        }
        Err(fault) => return Err(fault),
        Ok(()) => {
            return Err(FrameFault::BadCapture(
                "12-frame frozen seed-7 passed as motion".to_string(),
            ));
        }
    }
    let motion: Vec<String> = (0..12u64)
        .map(|seed| {
            check_scripted(Script::Clean, 96, 64, seed, &thresholds)
                .map(|report| report.digest)
                .map_err(|fault| {
                    FrameFault::BadCapture(format!("clean motion seed fails per frame: {fault}"))
                })
        })
        .collect::<Result<Vec<String>, FrameFault>>()?;
    check_digest_window(&motion)?;
    let mut unique = motion.clone();
    unique.sort();
    unique.dedup();
    let motion_unique = u64::try_from(unique.len()).map_err(|_| {
        FrameFault::BadCapture(format!("motion unique {} overflows u64", unique.len()))
    })?;
    Ok(FrozenGateOutcome {
        frozen_digest,
        frozen_frames: 12,
        motion_unique,
    })
}

/// Real offscreen scenes behind the `capture` feature: scripted variants of
/// the render default scene run through the CPU offscreen capture, sharing
/// [`count_near_slices`] and [`digest_hex`] with the synthetic path.
#[cfg(feature = "capture")]
pub mod capture {
    use super::{count_near_slices, digest_hex, FrameCamera};
    use engine_render::v1::{self, CaptureError, Manifest, Scene};

    /// Fault variants of the default scene: clean reference, unlit material,
    /// overexposed light, camera turned away from the mesh, camera grazing
    /// the mesh surface.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum RealScript {
        /// Three-quarter view of the lit cube.
        Clean,
        /// Zero light: the frame carries no lit contribution.
        Black,
        /// Blown-out light: every pixel saturates toward white.
        White,
        /// Camera turned away: the mesh leaves the frustum.
        MissingMesh,
        /// Eye just off the cube face: the near plane cuts the mesh.
        Clipped,
    }

    /// Scripted variant of the default scene at `width` x `height`.
    pub fn real_scene(script: RealScript, width: u32, height: u32) -> Result<Scene, CaptureError> {
        let mut scene = v1::default_scene(width, height)?;
        match script {
            RealScript::Clean => {}
            RealScript::Black => {
                scene.light.intensity = 0.0;
            }
            RealScript::White => {
                scene.light.intensity = 40.0;
            }
            RealScript::MissingMesh => {
                let eye = scene.camera.eye.to_array();
                let target = [eye[0] * 3.0, eye[1] * 3.0 + 2.0, eye[2] * 3.0];
                let up = scene.camera.up.to_array();
                let mut camera = v1::Camera::new(eye, target, up, scene.camera.fov_y);
                camera.aspect = width as f32 / height as f32;
                scene.camera = camera;
            }
            RealScript::Clipped => {
                let mut camera = v1::Camera::new(
                    [0.0, 0.0, 0.55],
                    [0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0],
                    scene.camera.fov_y,
                );
                camera.aspect = width as f32 / height as f32;
                scene.camera = camera;
            }
        }
        Ok(scene)
    }

    /// Plain-data camera copied from a render scene (no render import leaks
    /// into the slice math: only arrays cross the boundary).
    pub fn scene_camera(scene: &Scene) -> FrameCamera {
        FrameCamera {
            eye: scene.camera.eye.to_array(),
            target: scene.camera.target.to_array(),
            up: scene.camera.up.to_array(),
            fov_y: scene.camera.fov_y,
            aspect: scene.camera.aspect,
            near: scene.camera.near,
            far: scene.camera.far,
        }
    }

    /// Near-plane slice count over the real scene mesh plus camera.
    pub fn scene_slices(scene: &Scene) -> u32 {
        count_near_slices(
            &scene_camera(scene),
            &scene.mesh.positions,
            &scene.mesh.indices,
        )
    }

    /// Run the CPU offscreen capture for a scripted real scene.
    pub fn capture_png(
        script: RealScript,
        width: u32,
        height: u32,
    ) -> Result<(Vec<u8>, Manifest), CaptureError> {
        let scene = real_scene(script, width, height)?;
        v1::capture_scene(&scene, width, height)
    }

    /// Content tag over real capture bytes for evidence rows.
    pub fn png_digest(png: &[u8]) -> String {
        digest_hex(png)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 96;
    const H: u32 = 64;

    fn thresholds() -> Thresholds {
        Thresholds::default()
    }

    #[test]
    fn clean_passes_with_cover_metrics() {
        let metrics =
            check_scripted(Script::Clean, W, H, 7, &thresholds()).expect("clean scene passes");
        assert!(metrics.covered_pixels > 0);
        assert!(metrics.cover_permille >= u64::from(Thresholds::default().min_cover_permille));
        assert_eq!(metrics.draws, 2);
        assert_eq!(metrics.near_slices, 0);
        assert_eq!(metrics.digest.len(), 16);
    }

    #[test]
    fn clean_passes_across_seeds() {
        for seed in 0..8u64 {
            let metrics = check_scripted(Script::Clean, W, H, seed, &thresholds());
            assert!(metrics.is_ok(), "clean seed {seed} passes: {metrics:?}");
        }
    }

    #[test]
    fn clean_passes_at_arena_twin_and_play_step_sizes() {
        for (width, height) in [(160, 120), (128, 96), (96, 64)] {
            let metrics = check_scripted(Script::Clean, width, height, 7, &thresholds());
            assert!(
                metrics.is_ok(),
                "clean {width}x{height} passes: {metrics:?}"
            );
        }
    }

    #[test]
    fn white_gate_passes_and_names_its_door() {
        assert_eq!(WHITE_GATE_NAME, "qa4-white-screen-gate");
        let outcome = white_gate().expect("gate passes on seeded suite");
        assert_eq!(outcome.clean_digest.len(), 16);
        assert_eq!(outcome.white_digests.len(), 8);
        assert_eq!(outcome.black_digests.len(), 8);
        let again = white_gate().expect("gate replays");
        assert_eq!(outcome, again, "same seeds repeat the gate");
    }

    #[test]
    fn seeded_faults_fail_the_gate_instead_of_passing() {
        assert!(matches!(
            check_scripted(Script::White, W, H, 0, &thresholds()),
            Err(FrameFault::WhiteScreen(_))
        ));
        assert!(matches!(
            check_scripted(Script::Black, W, H, 0, &thresholds()),
            Err(FrameFault::BlackScreen(_))
        ));
    }

    #[test]
    fn seeded_black_frames_are_caught() {
        for seed in 0..8u64 {
            match check_scripted(Script::Black, W, H, seed, &thresholds()) {
                Err(FrameFault::BlackScreen(evidence)) => {
                    assert!(evidence.metric <= u64::from(Thresholds::default().black_max));
                    assert_eq!(evidence.digest.len(), 16);
                }
                other => panic!("black seed {seed} must fault, got {other:?}"),
            }
        }
    }

    #[test]
    fn seeded_white_frames_are_caught() {
        for seed in 0..8u64 {
            match check_scripted(Script::White, W, H, seed, &thresholds()) {
                Err(FrameFault::WhiteScreen(evidence)) => {
                    assert!(evidence.metric >= u64::from(Thresholds::default().white_min));
                    assert_eq!(evidence.digest.len(), 16);
                }
                other => panic!("white seed {seed} must fault, got {other:?}"),
            }
        }
    }

    #[test]
    fn seeded_missing_meshes_are_caught_on_both_arms() {
        let mut empty_draws = 0;
        let mut uncovered_draws = 0;
        for seed in 0..8u64 {
            let frame = scripted(Script::MissingMesh, W, H, seed);
            if frame.draws == 0 {
                empty_draws += 1;
            } else {
                uncovered_draws += 1;
            }
            match check_frame(&frame, &thresholds()) {
                Err(FrameFault::MissingMesh(evidence)) => {
                    assert_eq!(evidence.metric, 0, "backdrop stays below cover_min");
                }
                other => panic!("missing seed {seed} must fault, got {other:?}"),
            }
        }
        assert!(empty_draws > 0 && uncovered_draws > 0, "both arms covered");
    }

    #[test]
    fn seeded_clipping_is_caught_with_its_count() {
        for seed in 0..8u64 {
            let want = 1 + (seed % 4) as u32;
            match check_scripted(Script::Clipped, W, H, seed, &thresholds()) {
                Err(FrameFault::Clipping(evidence)) => {
                    assert_eq!(evidence.metric, u64::from(want));
                }
                other => panic!("clipped seed {seed} must fault, got {other:?}"),
            }
        }
    }

    #[test]
    fn digests_differ_per_fault_and_seed() {
        let clean = check_scripted(Script::Clean, W, H, 1, &thresholds())
            .expect("clean passes")
            .digest;
        let black = scripted(Script::Black, W, H, 1);
        let white = scripted(Script::White, W, H, 1);
        let missing = scripted(Script::MissingMesh, W, H, 1);
        let clipped = scripted(Script::Clipped, W, H, 1);
        let digests = [
            clean,
            digest_hex(&black.rgba),
            digest_hex(&white.rgba),
            digest_hex(&missing.rgba),
            digest_hex(&clipped.rgba),
        ];
        for (i, a) in digests.iter().enumerate() {
            for (j, b) in digests.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "fault digests differ ({i} vs {j})");
                }
            }
        }
        let again = check_scripted(Script::Clean, W, H, 1, &thresholds())
            .expect("clean passes")
            .digest;
        assert_eq!(digests[0], again, "same seed repeats its digest");
    }

    #[test]
    fn black_boundary_faults_at_max_and_passes_above() {
        let fill = |level: u8| CapturedFrame {
            width: 16,
            height: 16,
            rgba: vec![level, level, level, 255]
                .into_iter()
                .cycle()
                .take(16 * 16 * 4)
                .collect(),
            draws: 0,
            expected_draws: 0,
            near_slices: 0,
        };
        assert!(matches!(
            check_frame(&fill(Thresholds::default().black_max), &thresholds()),
            Err(FrameFault::BlackScreen(_))
        ));
        assert!(
            check_frame(&fill(Thresholds::default().black_max + 1), &thresholds()).is_ok(),
            "one step above black_max with nothing expected passes"
        );
    }

    #[test]
    fn white_boundary_faults_at_min_and_passes_below() {
        let fill = |level: u8| CapturedFrame {
            width: 16,
            height: 16,
            rgba: vec![level, level, level, 255]
                .into_iter()
                .cycle()
                .take(16 * 16 * 4)
                .collect(),
            draws: 0,
            expected_draws: 0,
            near_slices: 0,
        };
        assert!(matches!(
            check_frame(&fill(Thresholds::default().white_min), &thresholds()),
            Err(FrameFault::WhiteScreen(_))
        ));
        assert!(
            check_frame(&fill(Thresholds::default().white_min - 1), &thresholds()).is_ok(),
            "one step below white_min with nothing expected passes"
        );
    }

    #[test]
    fn fault_priority_is_black_then_white_then_missing_then_clipping() {
        let worst = CapturedFrame {
            width: 16,
            height: 16,
            rgba: vec![0u8; 16 * 16 * 4],
            draws: 0,
            expected_draws: 2,
            near_slices: 4,
        };
        assert!(
            matches!(
                check_frame(&worst, &thresholds()),
                Err(FrameFault::BlackScreen(_))
            ),
            "black wins over missing plus clipping"
        );
        let mut white_and_clipped = scripted(Script::White, W, H, 1);
        white_and_clipped.near_slices = 4;
        assert!(
            matches!(
                check_frame(&white_and_clipped, &thresholds()),
                Err(FrameFault::WhiteScreen(_))
            ),
            "white wins over clipping"
        );
        let mut missing_and_clipped = scripted(Script::MissingMesh, W, H, 1);
        missing_and_clipped.near_slices = 4;
        assert!(
            matches!(
                check_frame(&missing_and_clipped, &thresholds()),
                Err(FrameFault::MissingMesh(_))
            ),
            "missing wins over clipping"
        );
    }

    #[test]
    fn malformed_captures_are_bad_capture() {
        let empty = CapturedFrame {
            width: 0,
            height: H,
            rgba: vec![],
            draws: 0,
            expected_draws: 0,
            near_slices: 0,
        };
        assert!(matches!(
            check_frame(&empty, &thresholds()),
            Err(FrameFault::BadCapture(_))
        ));
        let short = CapturedFrame {
            width: 16,
            height: 16,
            rgba: vec![0u8; 16 * 16 * 4 - 1],
            draws: 0,
            expected_draws: 0,
            near_slices: 0,
        };
        assert!(matches!(
            check_frame(&short, &thresholds()),
            Err(FrameFault::BadCapture(_))
        ));
        let huge = CapturedFrame {
            width: MAX_FRAME_DIM + 1,
            height: 8,
            rgba: vec![],
            draws: 0,
            expected_draws: 0,
            near_slices: 0,
        };
        assert!(matches!(
            check_frame(&huge, &thresholds()),
            Err(FrameFault::BadCapture(_))
        ));
    }

    #[test]
    fn every_fault_displays_with_its_digest() {
        let cases = [
            (
                "black screen",
                check_scripted(Script::Black, W, H, 3, &thresholds()).unwrap_err(),
            ),
            (
                "white screen",
                check_scripted(Script::White, W, H, 3, &thresholds()).unwrap_err(),
            ),
            (
                "missing mesh",
                check_scripted(Script::MissingMesh, W, H, 3, &thresholds()).unwrap_err(),
            ),
            (
                "clipping",
                check_scripted(Script::Clipped, W, H, 3, &thresholds()).unwrap_err(),
            ),
        ];
        for (name, fault) in cases {
            let text = format!("{fault}");
            assert!(text.starts_with(name), "display names the fault: {text}");
            assert!(
                text.len() > name.len() + 16,
                "display carries the digest: {text}"
            );
        }
    }

    #[test]
    fn near_slice_math_counts_only_straddlers() {
        let camera = FrameCamera {
            eye: [0.0, 0.0, 5.0],
            target: [0.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            fov_y: 0.7,
            aspect: 1.0,
            near: 1.0,
            far: 100.0,
        };
        // Triangle crossing depth 1.0 (one vertex at 0.5, two past it).
        let positions = [[0.0, 0.0, 4.5], [1.0, 0.0, 2.0], [-1.0, 0.0, 2.0]];
        assert_eq!(count_near_slices(&camera, &positions, &[0, 1, 2]), 1);
        // Fully past the near plane: no cut.
        let far = [[0.0, 0.0, 2.0], [1.0, 0.0, 2.0], [-1.0, 0.0, 2.0]];
        assert_eq!(count_near_slices(&camera, &far, &[0, 1, 2]), 0);
        // Fully behind the eye: no cut.
        let behind = [[0.0, 0.0, 6.0], [1.0, 0.0, 7.0], [-1.0, 0.0, 7.0]];
        assert_eq!(count_near_slices(&camera, &behind, &[0, 1, 2]), 0);
        // Bad triangles never fault.
        assert_eq!(count_near_slices(&camera, &far, &[0, 1, 9]), 0);
        assert_eq!(count_near_slices(&camera, &far, &[0, 1]), 0);
        // Degenerate camera and clip range count zero.
        let mut bad = camera;
        bad.target = bad.eye;
        assert_eq!(count_near_slices(&bad, &positions, &[0, 1, 2]), 0);
        let mut bad_clip = camera;
        bad_clip.near = 0.0;
        assert_eq!(count_near_slices(&bad_clip, &positions, &[0, 1, 2]), 0);
    }

    #[test]
    fn faults_report_through_std_error() {
        let fault: Box<dyn std::error::Error> =
            Box::new(check_scripted(Script::Black, W, H, 0, &thresholds()).unwrap_err());
        assert!(format!("{fault}").starts_with("black screen"));
    }

    #[test]
    fn frozen_gate_passes_and_names_its_door() {
        assert_eq!(FROZEN_GATE_NAME, "qa14-frozen-sequence-gate");
        let outcome = frozen_gate().expect("gate catches frozen, passes motion");
        assert_eq!(outcome.frozen_digest.len(), 16);
        assert_eq!(outcome.frozen_frames, 12);
        assert!(
            outcome.motion_unique >= 2,
            "motion varies: {}",
            outcome.motion_unique
        );
        let again = frozen_gate().expect("gate replays");
        assert_eq!(outcome, again, "same seeds repeat the gate");
    }

    #[test]
    fn frozen_window_needs_three_identical() {
        let pair = ["aa".to_string(), "aa".to_string(), "bb".to_string()];
        assert!(
            check_digest_window(&pair).is_ok(),
            "two identical frames are motion, not frozen"
        );
        let empty: [String; 0] = [];
        assert!(check_digest_window(&empty).is_ok(), "empty passes");
        let short = ["aa".to_string()];
        assert!(
            check_digest_window(&short).is_ok(),
            "shorter than window passes"
        );
    }

    #[test]
    fn frozen_window_faults_with_run_length_and_digest() {
        let stuck = vec!["cc".to_string(); 12];
        match check_digest_window(&stuck) {
            Err(FrameFault::Frozen(evidence)) => {
                assert_eq!(evidence.digest, "cc");
                assert_eq!(evidence.metric, 12);
                let text = format!("{}", FrameFault::Frozen(evidence));
                assert!(
                    text.starts_with("frozen sequence"),
                    "display names it: {text}"
                );
            }
            other => panic!("12 identical digests must fault, got {other:?}"),
        }
    }

    #[test]
    fn per_frame_checks_still_pass_frozen_frames_singly() {
        // One frozen frame is pixel-identical to one clean frame: the
        // per-frame gate must pass it (only the window may fault).
        let metrics = check_scripted(Script::Clean, W, H, 7, &thresholds());
        assert!(metrics.is_ok(), "single frozen frame passes: {metrics:?}");
    }
}

#[cfg(feature = "capture")]
#[cfg(test)]
mod capture_tests {
    use super::capture::*;
    use super::*;

    const W: u32 = 160;
    const H: u32 = 120;

    #[test]
    fn ceiling_matches_render() {
        assert_eq!(
            MAX_FRAME_DIM,
            engine_render::v1::MAX_CAPTURE_DIM,
            "analysis bound tracks the render ceiling"
        );
    }

    #[test]
    fn real_clean_capture_renders_with_clean_manifest() {
        let (png, manifest) = capture_png(RealScript::Clean, W, H).expect("clean scene captures");
        assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert!(manifest.is_clean(), "no validation errors: {manifest:?}");
        assert_eq!((manifest.width, manifest.height), (W, H));
        let again = png_digest(
            &capture_png(RealScript::Clean, W, H)
                .expect("clean scene captures")
                .0,
        );
        assert_eq!(png_digest(&png), again, "same scene repeats its digest");
        let scene = real_scene(RealScript::Clean, W, H).expect("clean scene builds");
        assert_eq!(scene_slices(&scene), 0, "clean view cuts nothing");
    }

    #[test]
    fn real_fault_scenes_capture_distinctly() {
        let clean = png_digest(&capture_png(RealScript::Clean, W, H).expect("clean").0);
        for script in [
            RealScript::Black,
            RealScript::White,
            RealScript::MissingMesh,
            RealScript::Clipped,
        ] {
            let (png, manifest) = capture_png(script, W, H).expect("fault scene captures");
            assert!(manifest.is_clean(), "{script:?} stays valid: {manifest:?}");
            assert_ne!(png_digest(&png), clean, "{script:?} differs from clean");
        }
        let clipped = real_scene(RealScript::Clipped, W, H).expect("clipped builds");
        let slices = scene_slices(&clipped);
        assert!(slices > 0, "grazing view cuts the mesh: {slices}");
    }

    #[test]
    fn real_slice_counts_drive_the_clipping_check() {
        let clipped = real_scene(RealScript::Clipped, W, H).expect("clipped builds");
        let slices = scene_slices(&clipped);
        assert!(slices > 0);
        let mut frame = scripted(Script::Clean, W, H, 11);
        frame.near_slices = slices;
        match check_frame(&frame, &Thresholds::default()) {
            Err(FrameFault::Clipping(evidence)) => {
                assert_eq!(evidence.metric, u64::from(slices));
                assert_eq!(evidence.digest.len(), 16);
            }
            other => panic!("real slice count must clip, got {other:?}"),
        }
        let clean = real_scene(RealScript::Clean, W, H).expect("clean builds");
        let mut frame = scripted(Script::Clean, W, H, 11);
        frame.near_slices = scene_slices(&clean);
        assert!(
            check_frame(&frame, &Thresholds::default()).is_ok(),
            "clean geometry passes with its own slice count"
        );
    }
}
