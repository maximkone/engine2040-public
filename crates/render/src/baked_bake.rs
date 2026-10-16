//! baked_bake: arena bake + validity mask slice of the baked-GI pass.
//!
//! Owner: render lane; consumer `crate::baked_gi` (re-exports) and
//! `crate::present::look` (`bake_arena_fixture` per frame).
//! Units: texels for the lightmap, meters for probes and boxes, ms in
//! receipts. Determinism: fixed row-major bake order; same complete inputs
//! give bit-identical bytes x2. Errors are typed [`BakedGiError`].

#![forbid(unsafe_code)]

use std::fmt;
use std::time::Instant;

use crate::gi::{BakeScene, Lightmap, ProbeGrid, SurfaceSample};

// ---------------------------------------------------------------------------
// Bounds: every external size is validated before allocation.
// ---------------------------------------------------------------------------

/// Largest accepted lightmap edge in texels (bakes stay LUT-sized).
pub const BAKE_MAX_DIM: u32 = 256;
/// Largest accepted probe axis (keeps the grid inside [`PROBE_MAX_COUNT`]).
pub const PROBE_MAX_AXIS: u32 = 16;
/// Largest accepted probe count (a 16x4x16 arena grid fits with room).
pub const PROBE_MAX_COUNT: usize = 1024;
/// Canonical arena fixture lightmap width in texels.
pub const ARENA_LIGHTMAP_W: u32 = 64;
/// Canonical arena fixture lightmap height in texels.
pub const ARENA_LIGHTMAP_H: u32 = 64;

// ---------------------------------------------------------------------------
// Errors: typed; bad input fails loudly, never silent success.
// ---------------------------------------------------------------------------

/// Baked-GI failures. Every refusal names the bad input; the CPU bake here
/// is the reference and the frame bind in `baked_gi` always validates first.
#[derive(Clone, PartialEq, Debug)]
pub enum BakedGiError {
    /// Caller input is unusable; reason carried.
    Invalid(String),
}

impl fmt::Display for BakedGiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "baked-gi input: {e}"),
        }
    }
}

impl std::error::Error for BakedGiError {}

impl From<crate::ibl::IblError> for BakedGiError {
    fn from(e: crate::ibl::IblError) -> Self {
        Self::Invalid(format!("baked-gi sh eval: {e}"))
    }
}

// ---------------------------------------------------------------------------
// Sealed boxes: gate-tunnel regions where trilinear probes would leak.
// ---------------------------------------------------------------------------

/// Axis-aligned sealed region in meters (the gate tunnel): fighters inside
/// sample the nearest probe instead of the trilinear blend, so sunlit sand
/// outside never bleeds into the tunnel.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SealedBox {
    /// Minimum corner in meters, must be finite.
    pub min: [f32; 3],
    /// Maximum corner in meters, must be finite with `min <= max`.
    pub max: [f32; 3],
}

impl SealedBox {
    /// Validate corners before any bake trusts this box.
    pub fn validate(&self) -> Result<(), BakedGiError> {
        for (name, corner) in [("min", self.min), ("max", self.max)] {
            if !corner.iter().all(|c| c.is_finite()) {
                return Err(BakedGiError::Invalid(format!(
                    "baked-gi sealed box {name} {corner:?} is not finite"
                )));
            }
        }
        for a in 0..3 {
            if self.min[a] > self.max[a] {
                return Err(BakedGiError::Invalid(format!(
                    "baked-gi sealed box min {:?} exceeds max {:?}",
                    self.min, self.max
                )));
            }
        }
        Ok(())
    }

    /// True while `pos` sits inside (edges count as inside, so a fighter
    /// exactly on the tunnel mouth takes the sealed path).
    pub fn contains(&self, pos: [f32; 3]) -> bool {
        (0..3).all(|a| pos[a] >= self.min[a] && pos[a] <= self.max[a])
    }
}

// ---------------------------------------------------------------------------
// Probe spec: grid shape for the fighter-probe bake.
// ---------------------------------------------------------------------------

/// Actor probe grid shape: axis counts plus origin and spacing in meters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ProbeSpec {
    /// Probes on x, `1..=PROBE_MAX_AXIS`.
    pub nx: u32,
    /// Probes on y, `1..=PROBE_MAX_AXIS`.
    pub ny: u32,
    /// Probes on z, `1..=PROBE_MAX_AXIS`.
    pub nz: u32,
    /// Grid origin (probe 0,0,0 position) in meters, must be finite.
    pub min: [f32; 3],
    /// Spacing per axis in meters, must be finite and positive.
    pub step: [f32; 3],
}

impl ProbeSpec {
    /// Canonical arena grid: 6x3x6 over a 20x6x20 m floor with the tunnel.
    pub fn arena() -> Self {
        Self {
            nx: 6,
            ny: 3,
            nz: 6,
            min: [-10.0, 0.25, -10.0],
            step: [4.0, 2.0, 4.0],
        }
    }

    /// Validate axes, origin and spacing before any bake allocates.
    pub fn validate(&self) -> Result<(), BakedGiError> {
        for (name, n) in [("nx", self.nx), ("ny", self.ny), ("nz", self.nz)] {
            if n == 0 || n > PROBE_MAX_AXIS {
                return Err(BakedGiError::Invalid(format!(
                    "baked-gi probe axis {name} {n} outside 1..={PROBE_MAX_AXIS}"
                )));
            }
        }
        let count = self.nx as usize * self.ny as usize * self.nz as usize;
        if count > PROBE_MAX_COUNT {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi probe count {count} exceeds {PROBE_MAX_COUNT}"
            )));
        }
        if !self.min.iter().all(|c| c.is_finite()) {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi probe min {:?} is not finite",
                self.min
            )));
        }
        if !self.step.iter().all(|c| c.is_finite()) || self.step.iter().any(|c| *c <= 0.0) {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi probe step {:?} must be finite positive",
                self.step
            )));
        }
        Ok(())
    }

    /// Probe count for this shape.
    pub fn count(&self) -> usize {
        self.nx as usize * self.ny as usize * self.nz as usize
    }
}

// ---------------------------------------------------------------------------
// Arena bake: lightmap plus validity mask plus sealed fighter probes.
// ---------------------------------------------------------------------------

/// Baked arena GI: a `gi` hemisphere lightmap with a per-texel validity mask
/// plus a `gi` probe grid with per-probe tunnel seals. The SH coefficients
/// are evaluated through `ibl`, never re-derived here.
pub struct ArenaBake {
    /// Hemisphere lightmap over the arena floor and walls.
    pub map: Lightmap,
    /// Per-texel validity mask, row-major like the map: 1 means the bake
    /// input was sound, 0 means a degenerate sample the consumer must skip.
    pub valid: Vec<u8>,
    /// Actor probe grid over the arena volume.
    pub probes: ProbeGrid,
    /// Per-probe tunnel seal in grid order: 1 means the probe sits inside a
    /// sealed box and only serves nearest (never blended) lookups.
    pub sealed: Vec<u8>,
    /// Sealed boxes kept for the runtime inside test.
    pub boxes: Vec<SealedBox>,
}

impl ArenaBake {
    /// Texel count of the lightmap.
    pub fn texels(&self) -> usize {
        self.map.tex.len()
    }

    /// Valid texel count (mask entries equal to 1).
    pub fn valid_count(&self) -> usize {
        self.valid.iter().filter(|&&v| v == 1).count()
    }

    /// Probe count.
    pub fn probe_count(&self) -> usize {
        self.probes.probe_count()
    }

    /// Sealed probe count.
    pub fn sealed_count(&self) -> usize {
        self.sealed.iter().filter(|&&v| v == 1).count()
    }

    /// True while texel `(x, y)` baked from a sound sample.
    pub fn is_valid(&self, x: u32, y: u32) -> bool {
        if x >= self.map.w || y >= self.map.h {
            return false;
        }
        let i = y as usize * self.map.w as usize + x as usize;
        self.valid.get(i).copied().unwrap_or(0) == 1
    }

    /// Exact resident bytes: HDR texels plus indices plus mask plus probe
    /// SH plus seals plus boxes. The frame receipt must agree with this.
    pub fn resident_bytes(&self) -> u64 {
        self.map.tex.len() as u64 * 12
            + self.map.idx.len() as u64
            + self.valid.len() as u64
            + self.probes.sh.len() as u64 * 9 * 12
            + self.sealed.len() as u64
            + self.boxes.len() as u64 * 24
    }

    /// Backing check: dims match every buffer before a bind trusts the bake.
    pub fn check_backing(&self, what: &'static str) -> Result<(), BakedGiError> {
        let n = self.map.w as usize * self.map.h as usize;
        if self.map.tex.len() != n || self.map.idx.len() != n || self.valid.len() != n {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi {what} lightmap backing {} mismatches {}x{}",
                self.map.tex.len(),
                self.map.w,
                self.map.h
            )));
        }
        if self.valid.iter().any(|&v| v != 0 && v != 1) {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi {what} mask holds non-binary entries"
            )));
        }
        let want = self.probes.nx as usize * self.probes.ny as usize * self.probes.nz as usize;
        if self.probes.sh.len() != want || self.sealed.len() != want {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi {what} probe backing {} mismatches grid",
                self.probes.sh.len()
            )));
        }
        if self.sealed.iter().any(|&v| v != 0 && v != 1) {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi {what} seals hold non-binary entries"
            )));
        }
        Ok(())
    }

    /// Deterministic little-endian bytes: dims, HDR texels, indices, mask,
    /// probe grid shape and SH, seals, boxes. Same complete inputs give
    /// bit-identical bytes on every run.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.resident_bytes() as usize + 32);
        out.extend_from_slice(&self.map.w.to_le_bytes());
        out.extend_from_slice(&self.map.h.to_le_bytes());
        for t in &self.map.tex {
            for c in t {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        out.extend_from_slice(&self.map.idx);
        out.extend_from_slice(&self.valid);
        out.extend_from_slice(&self.probes.nx.to_le_bytes());
        out.extend_from_slice(&self.probes.ny.to_le_bytes());
        out.extend_from_slice(&self.probes.nz.to_le_bytes());
        for c in self.probes.min.iter().chain(self.probes.step.iter()) {
            out.extend_from_slice(&c.to_le_bytes());
        }
        for p in &self.probes.sh {
            for c in p {
                for v in c {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        out.extend_from_slice(&self.sealed);
        out.extend_from_slice(&(self.boxes.len() as u32).to_le_bytes());
        for b in &self.boxes {
            for c in b.min.iter().chain(b.max.iter()) {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        out
    }

    /// Rebake gate digest over [`Self::to_bytes`] via the engine-core hash
    /// owner (FNV-1a/64; same bytes give the same digest on every run).
    pub fn digest(&self) -> u64 {
        engine_core::hash_bytes(&self.to_bytes())
    }
}

pub(crate) fn sample_sound(s: &SurfaceSample) -> bool {
    if !s.pos.iter().all(|c| c.is_finite()) {
        return false;
    }
    if !s.normal.iter().all(|c| c.is_finite()) {
        return false;
    }
    let l2 = s.normal[0] * s.normal[0] + s.normal[1] * s.normal[1] + s.normal[2] * s.normal[2];
    if !(l2.is_finite() && l2 >= 1e-12) {
        return false;
    }
    if !s.albedo.iter().all(|c| c.is_finite() && *c >= 0.0) {
        return false;
    }
    if !s.occlusion.is_finite() {
        return false;
    }
    true
}

fn check_scene(scene: &BakeScene<'_>) -> Result<(), BakedGiError> {
    for (i, d) in scene.dirs.iter().enumerate() {
        if !d.dir.iter().all(|c| c.is_finite()) {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi dir light {i} direction is not finite"
            )));
        }
        let l2 = d.dir[0] * d.dir[0] + d.dir[1] * d.dir[1] + d.dir[2] * d.dir[2];
        if !(l2.is_finite() && l2 >= 1e-12) {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi dir light {i} direction has ~zero length"
            )));
        }
        if !d.color.iter().all(|c| c.is_finite() && *c >= 0.0) {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi dir light {i} color is not finite nonnegative"
            )));
        }
    }
    for (i, q) in scene.points.iter().enumerate() {
        if !q.pos.iter().all(|c| c.is_finite()) {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi point light {i} position is not finite"
            )));
        }
        if !q.color.iter().all(|c| c.is_finite() && *c >= 0.0) {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi point light {i} color is not finite nonnegative"
            )));
        }
        if !q.inv_range_sq.is_finite() || q.inv_range_sq < 0.0 {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi point light {i} inv_range_sq is not finite nonnegative"
            )));
        }
    }
    for (name, c) in [("sky", scene.sky.sky), ("ground", scene.sky.ground)] {
        if !c.iter().all(|v| v.is_finite() && *v >= 0.0) {
            return Err(BakedGiError::Invalid(format!(
                "baked-gi sky {name} is not finite nonnegative"
            )));
        }
    }
    Ok(())
}

/// Bake the arena: hemisphere lightmap with a validity mask plus fighter
/// probes with tunnel seals. Unsound samples (non-finite or degenerate
/// inputs) bake black and read 0 in the mask instead of poisoning the map.
/// Returns the bake plus wall-clock ms (evidence only, never hashed).
pub fn bake_arena(
    w: u32,
    h: u32,
    samples: &[SurfaceSample],
    scene: &BakeScene<'_>,
    probe: &ProbeSpec,
    boxes: &[SealedBox],
) -> Result<(ArenaBake, f64), BakedGiError> {
    if w == 0 || h == 0 {
        return Err(BakedGiError::Invalid(format!(
            "baked-gi lightmap {w}x{h} is zero"
        )));
    }
    if w > BAKE_MAX_DIM || h > BAKE_MAX_DIM {
        return Err(BakedGiError::Invalid(format!(
            "baked-gi lightmap {w}x{h} exceeds {BAKE_MAX_DIM}"
        )));
    }
    if samples.len() != w as usize * h as usize {
        return Err(BakedGiError::Invalid(format!(
            "baked-gi samples {} mismatch {}x{}",
            samples.len(),
            w,
            h
        )));
    }
    check_scene(scene)?;
    probe.validate()?;
    for b in boxes {
        b.validate()?;
    }
    let started = Instant::now();
    let mut cleaned = Vec::with_capacity(samples.len());
    let mut valid = Vec::with_capacity(samples.len());
    for s in samples {
        if sample_sound(s) {
            cleaned.push(*s);
            valid.push(1u8);
        } else {
            // Degenerate sample: bake black, flag invalid. The consumer
            // falls back to probe ambient for these texels.
            cleaned.push(SurfaceSample {
                pos: [0.0, 0.0, 0.0],
                normal: [0.0, 1.0, 0.0],
                albedo: [0.0, 0.0, 0.0],
                occlusion: 0.0,
            });
            valid.push(0u8);
        }
    }
    let map = Lightmap::bake(w, h, &cleaned, scene);
    let grid = ProbeGrid::bake(probe.nx, probe.ny, probe.nz, probe.min, probe.step, scene);
    let mut sealed = Vec::with_capacity(grid.sh.len());
    for iz in 0..probe.nz {
        for iy in 0..probe.ny {
            for ix in 0..probe.nx {
                let p = grid.probe_pos(ix as usize, iy as usize, iz as usize);
                let inside = boxes.iter().any(|b| b.contains(p));
                sealed.push(u8::from(inside));
            }
        }
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok((
        ArenaBake {
            map,
            valid,
            probes: grid,
            sealed,
            boxes: boxes.to_vec(),
        },
        ms,
    ))
}

// ---------------------------------------------------------------------------
// Canonical arena fixture: the bake the arena frame and captures share.
// ---------------------------------------------------------------------------

/// Canonical arena sun plus two gate-torch points and a sand-sky ambient:
/// the complete bake input the fixture, the tests and captures share.
pub fn arena_fixture_parts() -> (
    Vec<crate::gi::DirLight>,
    Vec<crate::gi::PointLight>,
    crate::gi::SkyAmbient,
) {
    use crate::gi::{DirLight, PointLight, SkyAmbient};
    (
        vec![DirLight {
            dir: [0.3, 1.0, 0.2],
            color: [2.2, 2.0, 1.8],
        }],
        vec![
            PointLight {
                pos: [-4.0, 2.5, 9.0],
                color: [6.0, 3.0, 1.2],
                inv_range_sq: 1.0 / 144.0,
            },
            PointLight {
                pos: [4.0, 2.5, 9.0],
                color: [6.0, 3.0, 1.2],
                inv_range_sq: 1.0 / 144.0,
            },
        ],
        SkyAmbient {
            sky: [0.35, 0.45, 0.6],
            ground: [0.22, 0.17, 0.12],
        },
    )
}

/// Canonical sand-floor samples: `w` by `h` texels over a 20x20 m floor at
/// `y = 0` with +Y normals, sand albedo and full occlusion, except a darker
/// worn strip down the middle (the fight lane) at 0.85 occlusion.
pub fn arena_floor_samples(w: u32, h: u32) -> Vec<SurfaceSample> {
    let mut out = Vec::with_capacity(w as usize * h as usize);
    for y in 0..h {
        for x in 0..w {
            let fx = (f64::from(x) + 0.5) / f64::from(w) * 20.0 - 10.0;
            let fz = (f64::from(y) + 0.5) / f64::from(h) * 20.0 - 10.0;
            let lane = if fx.abs() < 2.0 { 0.85 } else { 1.0 };
            out.push(SurfaceSample {
                pos: [fx as f32, 0.0, fz as f32],
                normal: [0.0, 1.0, 0.0],
                albedo: [0.62, 0.50, 0.36],
                occlusion: lane,
            });
        }
    }
    out
}

/// Canonical gate tunnel: a 6x3x4 m sealed box at the arena edge straddling
/// two probe columns; fighters inside take the sealed path.
pub fn arena_tunnel() -> SealedBox {
    SealedBox {
        min: [-3.0, 0.0, 8.0],
        max: [3.0, 3.0, 12.0],
    }
}

/// Bake the canonical arena fixture: 64x64 sand floor plus the 6x3x6 probe
/// grid with the gate tunnel sealed. The bytes the captures pin and the tests replay.
pub fn bake_arena_fixture() -> Result<(ArenaBake, f64), BakedGiError> {
    let (dirs, points, sky) = arena_fixture_parts();
    let scene = BakeScene {
        dirs: &dirs,
        points: &points,
        sky,
    };
    let samples = arena_floor_samples(ARENA_LIGHTMAP_W, ARENA_LIGHTMAP_H);
    bake_arena(
        ARENA_LIGHTMAP_W,
        ARENA_LIGHTMAP_H,
        &samples,
        &scene,
        &ProbeSpec::arena(),
        &[arena_tunnel()],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gi::{BakeScene, DirLight, PointLight, SkyAmbient};

    fn slice_small_samples(w: u32, h: u32) -> Vec<SurfaceSample> {
        arena_floor_samples(w, h)
    }

    #[test]
    fn bake_slice_bakes_identical_bytes_twice() {
        // Slice proof: a small bake through the new `baked_bake`
        // path gives bit-identical bytes plus one stable digest x2.
        let (dirs, points, sky) = arena_fixture_parts();
        let w = 8u32;
        let h = 8u32;
        let bake_once = |dirs: &[DirLight], points: &[PointLight], sky: SkyAmbient| {
            let scene = BakeScene { dirs, points, sky };
            bake_arena(
                w,
                h,
                &slice_small_samples(w, h),
                &scene,
                &ProbeSpec {
                    nx: 2,
                    ny: 2,
                    nz: 2,
                    min: [-10.0, 0.25, -10.0],
                    step: [20.0, 4.0, 20.0],
                },
                &[],
            )
            .expect("slice bake builds")
        };
        let (a, ms_a) = bake_once(&dirs, &points, sky);
        let (b, ms_b) = bake_once(&dirs, &points, sky);
        assert!(ms_a >= 0.0 && ms_b >= 0.0, "slice bake records pass ms");
        assert_eq!(a.to_bytes(), b.to_bytes(), "slice bake bytes stable x2");
        assert_eq!(a.digest(), b.digest(), "slice digest stable x2");
        assert_ne!(a.digest(), 0, "slice digest must actually hash");
        assert_eq!(a.valid_count(), a.texels(), "small slice fully valid");
    }
}
