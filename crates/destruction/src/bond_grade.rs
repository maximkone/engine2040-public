//! bond_grade: graded per-bond breaking strain for baked bonds.
//!
//! Consumer: the DX14 calibration corpus reads graded tables offline;
//! the arena crate-prop path stays frozen so the golden hash never moves.
//! Caller (production): `crates/destruction/src/lib.rs` re-exports this
//! module; UNLOCKS DX14 `crates/destruction/tests/grade_calib.rs`.
//! Units: meters, kg-ish mass units, Q16 strain (`65536 == 1.0`), ticks.
//! Determinism: index-order loops, canonical bond order, `Rng::fork`
//! jitter, `+ - * /` and `sqrt` only, no hash maps, no wall clock.
//! Errors: [`GradeError`]. Budget: bake-only; grading scans at most 4096
//! bonds once, density cuts clip one shell per site pair, only the
//! returned vecs allocate, never the tick.

use crate::bake3d::Chunk3;
use crate::bonded::{Bond, BondedDesc, BreakError, NO_LAYER};
use crate::material::{MaterialError, MaterialId, MaterialTable};
use crate::nav_dirty::{DirtySetError, DirtySetRecord};
use engine_core::checked::saturating_usize_to_u32;
use engine_core::{Fnv64, Rng};

/// Round a finite positive `f64` to `u64`, saturating without an `as` cast.
///
/// Bit-decodes the rounded value like the core time kernel, so the bake
/// stays deterministic on every PC. Non-finite and non-positive inputs
/// saturate to `0` or `u64::MAX` instead of wrapping.
fn f64_round_to_u64_saturating(v: f64) -> u64 {
    if !v.is_finite() {
        if v.is_nan() {
            return 0;
        }
        return if v > 0.0 { u64::MAX } else { 0 };
    }
    let r = v.round();
    if r <= 0.0 {
        return 0;
    }
    if r >= 1.844_674_407_370_955_2e19 {
        return u64::MAX;
    }
    let bits = r.to_bits();
    let exp_raw = (bits >> 52) & 0x7ff;
    if exp_raw == 0 {
        return 0;
    }
    let exp_u16 = u16::try_from(exp_raw).unwrap_or(u16::MAX);
    let exp = i32::from(exp_u16).checked_sub(1023).unwrap_or(i32::MIN);
    if exp < 0 {
        return 0;
    }
    let mant = (bits & 0x000f_ffff_ffff_ffff) | 0x0010_0000_0000_0000;
    if exp <= 52 {
        let sub = 52i32.checked_sub(exp).unwrap_or(0);
        let shift = u32::try_from(sub).unwrap_or(52);
        mant >> shift
    } else {
        let sub = exp.checked_sub(52).unwrap_or(64);
        let shift = u32::try_from(sub).unwrap_or(64);
        if shift >= 64 || mant > u64::MAX >> shift {
            return u64::MAX;
        }
        mant << shift
    }
}

/// Q16 strain word back to `f32` without an `as` cast.
///
/// Splits the word into `u16` limbs that widen with `From`, so the bake
/// never truncates through a float cast.
fn q16_to_strain_f32(strain_q16: u64) -> f32 {
    let whole = strain_q16 / STRAIN_Q16_ONE;
    let frac = strain_q16 % STRAIN_Q16_ONE;
    let frac_u16 = u16::try_from(frac).unwrap_or(u16::MAX);
    let frac_f32 = f32::from(frac_u16) / 65_536.0;
    let hi = whole >> 16;
    let lo = whole & 0xffff;
    let hi_u16 = u16::try_from(hi).unwrap_or(u16::MAX);
    let lo_u16 = u16::try_from(lo).unwrap_or(u16::MAX);
    f32::from(hi_u16) * 65_536.0 + f32::from(lo_u16) + frac_f32
}

/// Version of the grading scheme; baked bytes carry it first.
pub const GRADE_VERSION_V1: u32 = 1;
/// One whole strain unit in Q16.
pub const STRAIN_Q16_ONE: u64 = 65_536;
/// Largest chunk count grading accepts; mirrors the bonded bound.
pub const MAX_GRADE_CHUNKS: usize = 1024;
/// Largest bond count grading accepts; mirrors the bonded bound.
pub const MAX_GRADE_BONDS: usize = 4096;
/// Reference contact area in Q16 (`0.0625 m2`): a 25 cm square face.
pub const AREA_REF_Q16: u64 = 4096;
/// Reference chunk mass in Q16 (`0.125` units): a 25 cm cube.
pub const MASS_REF_Q16: u64 = 8192;
/// Default jitter span in Q16 (`0.125` strain); `GradeParams` may narrow it.
pub const DEFAULT_JITTER_SPAN_Q16: u32 = 8192;
/// Largest jitter span grading accepts in Q16 (`0.5` strain).
pub const MAX_JITTER_SPAN_Q16: u32 = 32_768;
/// Sliver volume below which a chunk is dropped; cubic meters.
pub const SLIVER_VOL_M3: f32 = 1.0e-9;
/// Sliver half-extent below which a chunk is dropped; meters.
pub const SLIVER_MIN_HALF_M: f32 = 2.0e-4;
/// Draw span for density offsets: `[-1, 1)` in 1/1024 steps.
const OFFSET_SPAN: u64 = 2048;
/// Share of density sites clustered near the hit, in eighths.
const NEAR_EIGHTHS: usize = 5;

/// Why grading refused; user data never panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GradeError {
    /// No chunks supplied.
    EmptyChunks,
    /// More than `MAX_GRADE_CHUNKS` chunks supplied.
    TooManyChunks,
    /// More than `MAX_GRADE_BONDS` bonds supplied.
    TooManyBonds,
    /// Bond endpoint, duplicate or limit at the carried bond index.
    BadBond(usize),
    /// Bond material array length or byte at the carried index.
    BadBondMaterial(usize),
    /// Jitter span past `MAX_JITTER_SPAN_Q16`.
    BadJitter,
    /// Mesh or cut refused by the CSG kernel.
    Csg,
    /// Sliver reject dropped every chunk; nothing left to ship.
    SliveredAway,
}

impl core::fmt::Display for GradeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyChunks => write!(f, "grade needs chunks"),
            Self::TooManyChunks => write!(f, "grade exceeds MAX_GRADE_CHUNKS"),
            Self::TooManyBonds => write!(f, "grade exceeds MAX_GRADE_BONDS"),
            Self::BadBond(i) => write!(f, "grade bond {i} is out of bounds"),
            Self::BadBondMaterial(i) => write!(f, "grade bond material {i} is unknown"),
            Self::BadJitter => write!(f, "grade jitter span is out of bounds"),
            Self::Csg => write!(f, "grade mesh cut was refused"),
            Self::SliveredAway => write!(f, "sliver reject dropped every chunk"),
        }
    }
}

impl std::error::Error for GradeError {}

impl From<BreakError> for GradeError {
    fn from(_: BreakError) -> Self {
        Self::BadBond(usize::MAX)
    }
}

impl From<MaterialError> for GradeError {
    fn from(_: MaterialError) -> Self {
        Self::BadBondMaterial(usize::MAX)
    }
}

/// Bake knobs for grading: seed plus jitter span.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GradeParams {
    /// Seed of the per-bond jitter draws.
    pub seed: u64,
    /// Half-width of the uniform jitter in Q16; `0` disables jitter.
    pub jitter_span_q16: u32,
}

impl GradeParams {
    /// Bake knobs with the default jitter span.
    #[must_use]
    pub const fn with_seed(seed: u64) -> Self {
        Self {
            seed,
            jitter_span_q16: DEFAULT_JITTER_SPAN_Q16,
        }
    }

    /// Range-checks the knobs without grading.
    pub fn validate(self) -> Result<(), GradeError> {
        if self.jitter_span_q16 > MAX_JITTER_SPAN_Q16 {
            return Err(GradeError::BadJitter);
        }
        Ok(())
    }
}

/// Contact face area of two boxes in m2; `+ - *` only.
///
/// Overlaps per axis are clamped at zero; the separation axis (smallest
/// overlap) is dropped and the other two multiply. Touching faces give
/// the face area, disjoint boxes give zero.
#[must_use]
pub fn contact_area_m2(a: &Chunk3, b: &Chunk3) -> f32 {
    let ox = a.hx + b.hx - (a.x - b.x).abs();
    let oy = a.hy + b.hy - (a.y - b.y).abs();
    let oz = a.hz + b.hz - (a.z - b.z).abs();
    if ox <= 0.0 || oy <= 0.0 || oz <= 0.0 {
        return 0.0;
    }
    if ox <= oy && ox <= oz {
        oy * oz
    } else if oy <= oz {
        ox * oz
    } else {
        ox * oy
    }
}

/// Contact area in Q16 load units, saturated; disjoint pairs read zero.
#[must_use]
pub fn contact_area_q16(a: &Chunk3, b: &Chunk3) -> u64 {
    let area = f64::from(contact_area_m2(a, b)) * 65_536.0;
    if area <= 0.0 {
        return 0;
    }
    if area >= 1.8e19 {
        return u64::MAX;
    }
    f64_round_to_u64_saturating(area)
}

/// Chunk mass in Q16 load units, at least one so every chunk counts.
#[must_use]
pub fn mass_to_q16(mass: f32) -> u64 {
    let scaled = f64::from(mass) * 65_536.0;
    if scaled <= 1.0 {
        return 1;
    }
    if scaled >= 1.8e19 {
        return u64::MAX;
    }
    f64_round_to_u64_saturating(scaled).max(1)
}

/// Anchor distance per chunk: BFS layers out of the anchors over the
/// bond list, smallest-index parents first; `NO_LAYER` when unsupported.
///
/// Index-order flood with per-layer sorts, so the same pairs give the
/// same layers whatever order they arrived in. Allocates: bake only.
#[must_use]
pub fn anchor_layers(chunk_count: usize, bonds: &[Bond], anchors: &[u32]) -> Vec<u32> {
    let mut adj: Vec<Vec<u32>> = vec![Vec::new(); chunk_count];
    for bond in bonds {
        let a = usize::try_from(bond.a).unwrap_or(usize::MAX);
        let b = usize::try_from(bond.b).unwrap_or(usize::MAX);
        if a < chunk_count && b < chunk_count && a != b {
            adj[a].push(saturating_usize_to_u32(b));
            adj[b].push(saturating_usize_to_u32(a));
        }
    }
    let mut layer = vec![NO_LAYER; chunk_count];
    let mut work: Vec<usize> = Vec::new();
    for anchor in anchors {
        let i = usize::try_from(*anchor).unwrap_or(usize::MAX);
        if i < chunk_count && layer[i] == NO_LAYER {
            layer[i] = 0;
            work.push(i);
        }
    }
    let mut start = 0;
    let mut depth: u32 = 0;
    while start < work.len() {
        let end = work.len();
        work[start..end].sort_unstable();
        for k in start..end {
            let current = work[k];
            let mut next: Vec<u32> = adj[current].clone();
            next.sort_unstable();
            for n in next {
                let j = usize::try_from(n).unwrap_or(usize::MAX);
                if layer[j] == NO_LAYER {
                    layer[j] = depth + 1;
                    work.push(j);
                }
            }
        }
        start = end;
        depth = depth.saturating_add(1);
    }
    layer
}

/// Deterministic jitter of one canonical bond in Q16.
///
/// Uniform in `[-span, span]` drawn from `seed` forked by canonical
/// rank, so canonical order decides the draw and insertion order cannot.
#[must_use]
pub fn jitter_for_bond(seed: u64, canonical_rank: usize, span_q16: u32) -> i64 {
    if span_q16 == 0 {
        return 0;
    }
    let span = u64::from(span_q16);
    let width = span * 2 + 1;
    let rank = u64::try_from(canonical_rank).unwrap_or(u64::MAX);
    let draw = Rng::new(seed).fork(rank).next_u64() % width;
    let draw_i64 = i64::try_from(draw).unwrap_or(i64::MAX);
    let span_i64 = i64::try_from(span).unwrap_or(i64::MAX);
    draw_i64 - span_i64
}

/// One graded strain limit in Q16: material toughness scaled by contact
/// area and mass, weakened with anchor distance, plus seeded jitter.
///
/// Larger faces and heavier chunks hold harder; deeper layers (farther
/// from an anchor) fail sooner. Saturated to at least one so no bond is
/// free, and capped at `1e6` in Q16 like the bonded limit.
#[must_use]
pub fn grade_one_q16(
    base_q16: u32,
    area_q16: u64,
    chunk_mass_q16: u64,
    layer: u32,
    jitter_q16: i64,
) -> u64 {
    const CAP_Q16: u64 = 65_536_000_000;
    let area_num = STRAIN_Q16_ONE + area_q16.min(AREA_REF_Q16 * 4) / 4;
    let mass_num = STRAIN_Q16_ONE + chunk_mass_q16.min(MASS_REF_Q16 * 4) / 8;
    let depth = if layer == NO_LAYER { 8 } else { layer.min(8) };
    let anchor_den = STRAIN_Q16_ONE + u64::from(depth) * (STRAIN_Q16_ONE / 4);
    let scaled = (u128::from(base_q16) * u128::from(area_num) / u128::from(STRAIN_Q16_ONE))
        .saturating_mul(u128::from(mass_num))
        / u128::from(STRAIN_Q16_ONE);
    let weakened = (scaled * u128::from(STRAIN_Q16_ONE)) / u128::from(anchor_den);
    let jittered = i128::try_from(weakened).unwrap_or(i128::MAX) + i128::from(jitter_q16);
    if jittered <= 1 {
        return 1;
    }
    let as_u128 = u128::try_from(jittered).unwrap_or(u128::MAX);
    let capped = as_u128.min(u128::from(CAP_Q16));
    u64::try_from(capped).unwrap_or(CAP_Q16)
}

/// Grade every bond of a descriptor from the BX12 material table.
///
/// Bonds sort into canonical `(min, max)` order first, so shuffled
/// insertion grades identically; jitter draws by canonical rank.
/// Each bond reads `table` toughness through its material byte, scaled
/// by contact area, mean chunk mass and anchor layer. Returns the
/// descriptor with graded `breaking_strain` values, same chunks,
/// anchors and threshold. Bake only.
pub fn grade_desc(
    desc: &BondedDesc,
    bond_materials: &[u8],
    table: MaterialTable,
    params: GradeParams,
) -> Result<BondedDesc, GradeError> {
    params.validate()?;
    table
        .validate()
        .map_err(|_| GradeError::BadBondMaterial(usize::MAX))?;
    desc.validate()
        .map_err(|_| GradeError::BadBond(usize::MAX))?;
    if desc.chunks.is_empty() {
        return Err(GradeError::EmptyChunks);
    }
    if desc.chunks.len() > MAX_GRADE_CHUNKS {
        return Err(GradeError::TooManyChunks);
    }
    if desc.bonds.len() > MAX_GRADE_BONDS {
        return Err(GradeError::TooManyBonds);
    }
    if bond_materials.len() != desc.bonds.len() {
        let bad = bond_materials.len().min(desc.bonds.len());
        return Err(GradeError::BadBondMaterial(bad));
    }
    crate::material::validate_bond_materials(bond_materials).map_err(|e| match e {
        crate::material::MaterialError::BadBondMaterial { index, .. } => {
            GradeError::BadBondMaterial(index)
        }
        _ => GradeError::BadBondMaterial(usize::MAX),
    })?;
    let layers = anchor_layers(desc.chunks.len(), &desc.bonds, &desc.anchors);
    let masses: Vec<u64> = desc.chunks.iter().map(|c| mass_to_q16(c.mass)).collect();
    let mut order: Vec<usize> = (0..desc.bonds.len()).collect();
    order.sort_by(|&l, &r| {
        let (bl, br) = (&desc.bonds[l], &desc.bonds[r]);
        (bl.a.min(bl.b), bl.a.max(bl.b), l).cmp(&(br.a.min(br.b), br.a.max(br.b), r))
    });
    let mut graded: Vec<Bond> = Vec::with_capacity(desc.bonds.len());
    let mut mats: Vec<u8> = Vec::with_capacity(desc.bonds.len());
    for (rank, &i) in order.iter().enumerate() {
        let bond = &desc.bonds[i];
        let id =
            MaterialId::try_from(bond_materials[i]).map_err(|_| GradeError::BadBondMaterial(i))?;
        let base = table.breaking_strain_q16(id);
        let a = usize::try_from(bond.a).unwrap_or(usize::MAX);
        let b = usize::try_from(bond.b).unwrap_or(usize::MAX);
        let area = contact_area_q16(&desc.chunks[a], &desc.chunks[b]);
        let mean_mass = masses[a].saturating_add(masses[b]) / 2;
        let layer = layers[a].min(layers[b]).min(8);
        let jitter = jitter_for_bond(params.seed, rank, params.jitter_span_q16);
        let strain_q16 = grade_one_q16(base, area, mean_mass.max(1), layer, jitter);
        let strain = q16_to_strain_f32(strain_q16);
        let clamped = if strain.is_finite() && strain > 0.0 {
            strain.min(1.0e6)
        } else {
            1.0
        };
        graded.push(Bond {
            a: bond.a.min(bond.b),
            b: bond.a.max(bond.b),
            breaking_strain: clamped,
        });
        mats.push(bond_materials[i]);
    }
    let _ = mats;
    let out = BondedDesc {
        chunks: desc.chunks.clone(),
        bonds: graded,
        anchors: {
            let mut anchors = desc.anchors.clone();
            anchors.sort_unstable();
            anchors.dedup();
            anchors
        },
        break_threshold: desc.break_threshold,
    };
    out.validate()
        .map_err(|_| GradeError::BadBond(usize::MAX))?;
    Ok(out)
}

/// Seeded density sites over a shell box: site 0 is the hit, then
/// `NEAR_EIGHTHS / 8` cluster near it (denser where impacts land) and
/// the rest spread over the whole box. Every site stays inside the box.
///
/// Integer `Rng` draws turned into floats with `+ - * /` only; the same
/// box, hit, count and seed give the same sites on every PC.
#[must_use]
pub fn density_sites(
    min: &[f32; 3],
    max: &[f32; 3],
    hit: [f32; 3],
    cells: usize,
    radius_m: f32,
    seed: u64,
) -> Vec<[f32; 3]> {
    let mut out = Vec::new();
    if cells == 0 {
        return out;
    }
    out.push(hit);
    if cells == 1 {
        return out;
    }
    let near = (cells - 1) * NEAR_EIGHTHS / 8;
    let mut rng = Rng::new(seed).fork_named("grade-density");
    for k in 1..cells {
        let mut p = [0.0f32; 3];
        for axis in 0..3 {
            let draw = rng.next_range(0, OFFSET_SPAN);
            let draw_u16 = u16::try_from(draw).unwrap_or(u16::MAX);
            let unit = f32::from(draw_u16) / 1024.0 - 1.0;
            p[axis] = if k <= near {
                let fall = unit * unit.abs();
                (hit[axis] + fall * radius_m).clamp(min[axis], max[axis])
            } else {
                min[axis] + (unit * 0.5 + 0.5) * (max[axis] - min[axis])
            };
        }
        out.push(p);
    }
    out
}

/// Voronoi-density cells of `sites` over `mesh`, clipped by the existing
/// CSG split kernel (`clip_back_reused` over bisector half-spaces,
/// nearest site first, ties by site index). Empty cells drop.
///
/// This is the density leg DX9 owns: sites come from [`density_sites`],
/// clipping stays the shipped `csg` code, never a second splitter.
/// Bake only; allocates the cells.
pub fn cut_density_cells(
    mesh: &crate::csg::PropMesh,
    sites: &[[f32; 3]],
) -> Vec<crate::csg::PropMesh> {
    use crate::csg::{clip_back_reused, dist2, plane_between, ClipScratch};
    let mut pair: Vec<f32> = Vec::with_capacity(sites.len() * sites.len());
    for a in sites {
        for b in sites {
            pair.push(dist2(a, b));
        }
    }
    let mut scratch = ClipScratch::default();
    let mut cells = Vec::new();
    for (i, site) in sites.iter().enumerate() {
        let mut order: Vec<usize> = (0..sites.len()).filter(|j| *j != i).collect();
        order.sort_by(|a, b| {
            pair[i * sites.len() + *a]
                .total_cmp(&pair[i * sites.len() + *b])
                .then(a.cmp(b))
        });
        let mut cell = mesh.clone();
        for j in order {
            let Some(plane) = plane_between(site, &sites[j]) else {
                if j < i {
                    cell.indices.clear();
                    break;
                }
                continue;
            };
            clip_back_reused(&mut cell, &plane, &mut scratch);
            if cell.indices.is_empty() {
                break;
            }
        }
        if !cell.indices.is_empty() {
            cells.push(cell);
        }
    }
    cells
}

/// True when a baked box is a sliver: paper-thin or near-zero volume.
///
/// Slivers shatter the solver into one-triangle shards, so the bake
/// drops them before bonds are built. Pure `+ - *` comparisons only.
#[must_use]
pub fn is_sliver(c: &Chunk3) -> bool {
    if !c.hx.is_finite() || !c.hy.is_finite() || !c.hz.is_finite() || !c.mass.is_finite() {
        return true;
    }
    if c.hx < SLIVER_MIN_HALF_M || c.hy < SLIVER_MIN_HALF_M || c.hz < SLIVER_MIN_HALF_M {
        return true;
    }
    let vol = f64::from(c.hx) * f64::from(c.hy) * f64::from(c.hz) * 8.0;
    vol < f64::from(SLIVER_VOL_M3)
}

/// Retained chunk indices after sliver reject, ascending.
#[must_use]
pub fn reject_slivers(chunks: &[Chunk3]) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, c) in chunks.iter().enumerate() {
        if !is_sliver(c) {
            out.push(i);
        }
    }
    out
}

/// Drop sliver chunks from a descriptor, remapping bonds and anchors.
///
/// Bonds touching a dropped chunk go with it; survivors reindex through
/// the retained map and sort canonical. Errors when nothing survives.
/// Bake only; allocates the cleaned descriptor.
pub fn apply_sliver_reject(desc: &BondedDesc) -> Result<BondedDesc, GradeError> {
    if desc.chunks.is_empty() {
        return Err(GradeError::EmptyChunks);
    }
    let kept = reject_slivers(&desc.chunks);
    if kept.is_empty() {
        return Err(GradeError::SliveredAway);
    }
    let mut remap = vec![usize::MAX; desc.chunks.len()];
    for (at, &i) in kept.iter().enumerate() {
        remap[i] = at;
    }
    let chunks: Vec<Chunk3> = kept.iter().map(|&i| desc.chunks[i]).collect();
    let mut bonds = Vec::new();
    for bond in &desc.bonds {
        let a = usize::try_from(bond.a).unwrap_or(usize::MAX);
        let b = usize::try_from(bond.b).unwrap_or(usize::MAX);
        if a < remap.len() && b < remap.len() && remap[a] != usize::MAX && remap[b] != usize::MAX {
            let x = saturating_usize_to_u32(remap[a]);
            let y = saturating_usize_to_u32(remap[b]);
            bonds.push(Bond {
                a: x.min(y),
                b: x.max(y),
                breaking_strain: bond.breaking_strain,
            });
        }
    }
    bonds.sort_by(|l, r| (l.a, l.b).cmp(&(r.a, r.b)));
    bonds.dedup_by(|a, b| a.a == b.a && a.b == b.b);
    let mut anchors = Vec::new();
    for anchor in &desc.anchors {
        let i = usize::try_from(*anchor).unwrap_or(usize::MAX);
        if i < remap.len() && remap[i] != usize::MAX {
            anchors.push(saturating_usize_to_u32(remap[i]));
        }
    }
    anchors.sort_unstable();
    anchors.dedup();
    let out = BondedDesc {
        chunks,
        bonds,
        anchors,
        break_threshold: desc.break_threshold,
    };
    out.validate()
        .map_err(|_| GradeError::BadBond(usize::MAX))?;
    Ok(out)
}

/// Per-event break record for one drained batch: the freed chunks that
/// batch freed, sorted and deduped, ready to map to nav tiles.
///
/// Bond snaps ride the log but never dirty tiles alone, so only freed
/// chunks record. A batch freeing nothing refuses with `EmptySet` and
/// the caller skips invalidation. Thin bake-time wrapper over
/// [`DirtySetRecord::from_events`]; the arena maps chunks to tiles.
pub fn record_breaks(
    tick: u32,
    events: &[crate::bonded::BreakEvent],
    chunk_count: usize,
) -> Result<DirtySetRecord, DirtySetError> {
    let record = DirtySetRecord::from_events(tick, events, chunk_count)?;
    debug_assert_eq!(record.tick(), tick);
    Ok(record)
}

/// Canonical graded-bake bytes: version plus chunk boxes plus bonds
/// plus anchors, all bit-exact and in canonical order.
#[must_use]
pub fn graded_bytes(desc: &BondedDesc) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&GRADE_VERSION_V1.to_le_bytes());
    let chunk_len = u64::try_from(desc.chunks.len()).unwrap_or(u64::MAX);
    out.extend_from_slice(&chunk_len.to_le_bytes());
    for c in &desc.chunks {
        for w in [
            c.x.to_bits(),
            c.y.to_bits(),
            c.z.to_bits(),
            c.hx.to_bits(),
            c.hy.to_bits(),
            c.hz.to_bits(),
            c.mass.to_bits(),
        ] {
            out.extend_from_slice(&w.to_le_bytes());
        }
    }
    let bond_len = u64::try_from(desc.bonds.len()).unwrap_or(u64::MAX);
    out.extend_from_slice(&bond_len.to_le_bytes());
    for b in &desc.bonds {
        out.extend_from_slice(&b.a.to_le_bytes());
        out.extend_from_slice(&b.b.to_le_bytes());
        out.extend_from_slice(&b.breaking_strain.to_bits().to_le_bytes());
    }
    let anchor_len = u64::try_from(desc.anchors.len()).unwrap_or(u64::MAX);
    out.extend_from_slice(&anchor_len.to_le_bytes());
    for a in &desc.anchors {
        out.extend_from_slice(&a.to_le_bytes());
    }
    out
}

/// Digest over [`graded_bytes`]; equal bakes digest equal twice.
#[must_use]
pub fn graded_digest(desc: &BondedDesc) -> u64 {
    let mut h = Fnv64::new();
    h.write(&graded_bytes(desc));
    h.finish()
}

/// Bake entry: cut `mesh` with `cuts`, drop slivers, wire contact
/// bonds plus bottom-layer anchors, then grade with `table`.
///
/// Reuses the shipped `csg` bake and `in_contact` split results; this
/// file owns only grading plus sliver reject. Bake only.
pub fn bake_graded(
    mesh: &crate::csg::PropMesh,
    cuts: &[crate::csg::CutPlane],
    bond_materials: &[u8],
    table: MaterialTable,
    break_threshold: f32,
    params: GradeParams,
) -> Result<BondedDesc, GradeError> {
    let chunks = crate::csg::bake(mesh, cuts).map_err(|_| GradeError::Csg)?;
    let boxes: Vec<Chunk3> = chunks.iter().map(|c| c.to_chunk3()).collect();
    let kept = reject_slivers(&boxes);
    if kept.is_empty() {
        return Err(GradeError::SliveredAway);
    }
    let mut remap = vec![usize::MAX; boxes.len()];
    for (at, &i) in kept.iter().enumerate() {
        remap[i] = at;
    }
    let clean: Vec<Chunk3> = kept.iter().map(|&i| boxes[i]).collect();
    let mut bonds = Vec::new();
    for a in 0..clean.len() {
        for b in (a + 1)..clean.len() {
            if crate::shards3d::in_contact(&clean[a], &clean[b]) {
                bonds.push(Bond {
                    a: saturating_usize_to_u32(a),
                    b: saturating_usize_to_u32(b),
                    breaking_strain: 1.0,
                });
            }
        }
    }
    let bottom = clean
        .iter()
        .map(|c| c.y - c.hy)
        .reduce(f32::min)
        .unwrap_or(0.0);
    let mut anchors = Vec::new();
    for (i, c) in clean.iter().enumerate() {
        if (c.y - c.hy) - bottom <= 1e-3 {
            anchors.push(saturating_usize_to_u32(i));
        }
    }
    if bond_materials.len() != bonds.len() {
        return Err(GradeError::BadBondMaterial(
            bond_materials.len().min(bonds.len()),
        ));
    }
    // Material bytes arrive in contact-pair order `(a, b)` ascending
    // because the loops above emit that order; grading canonicalizes.
    let raw = BondedDesc {
        chunks: clean,
        bonds,
        anchors,
        break_threshold,
    };
    let graded = grade_desc(&raw, bond_materials, table, params)?;
    apply_sliver_reject(&graded)
}

/// Density bake entry: `cells` sites around `hit`, clipped by the
/// shipped split kernel, boxed and sliver-cleaned.
///
/// Returns kept boxes in site order with empties dropped. Bake only.
pub fn density_boxes(
    mesh: &crate::csg::PropMesh,
    hit: [f32; 3],
    cells: usize,
    radius_m: f32,
    seed: u64,
) -> Vec<Chunk3> {
    let (min, max) = (mesh_min(mesh), mesh_max(mesh));
    let sites = density_sites(&min, &max, hit, cells, radius_m, seed);
    let mut out = Vec::new();
    for cell in cut_density_cells(mesh, &sites) {
        let (lo, hi) = (mesh_min(&cell), mesh_max(&cell));
        let half = [
            ((hi[0] - lo[0]) * 0.5).max(1e-4),
            ((hi[1] - lo[1]) * 0.5).max(1e-4),
            ((hi[2] - lo[2]) * 0.5).max(1e-4),
        ];
        let c = Chunk3 {
            x: (lo[0] + hi[0]) * 0.5,
            y: (lo[1] + hi[1]) * 0.5,
            z: (lo[2] + hi[2]) * 0.5,
            hx: half[0],
            hy: half[1],
            hz: half[2],
            mass: half[0] * half[1] * half[2] * 8.0,
        };
        if !is_sliver(&c) {
            out.push(c);
        }
    }
    out
}

fn mesh_min(mesh: &crate::csg::PropMesh) -> [f32; 3] {
    let mut min = [f32::INFINITY; 3];
    for v in &mesh.positions {
        for k in 0..3 {
            if v[k] < min[k] {
                min[k] = v[k];
            }
        }
    }
    min
}

fn mesh_max(mesh: &crate::csg::PropMesh) -> [f32; 3] {
    let mut max = [f32::NEG_INFINITY; 3];
    for v in &mesh.positions {
        for k in 0..3 {
            if v[k] > max[k] {
                max[k] = v[k];
            }
        }
    }
    max
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::BREAKING_THRESHOLD;
    use crate::bake3d::fracture3;
    use crate::bonded::BondedCollection;

    fn test_desc() -> BondedDesc {
        BondedDesc {
            chunks: vec![
                Chunk3 {
                    x: 0.0,
                    y: 2.0,
                    z: 0.0,
                    hx: 0.5,
                    hy: 0.5,
                    hz: 0.5,
                    mass: 1.0,
                },
                Chunk3 {
                    x: 1.0,
                    y: 2.0,
                    z: 0.0,
                    hx: 0.5,
                    hy: 0.5,
                    hz: 0.5,
                    mass: 1.0,
                },
                Chunk3 {
                    x: 2.0,
                    y: 2.0,
                    z: 0.0,
                    hx: 0.5,
                    hy: 0.5,
                    hz: 0.5,
                    mass: 2.0,
                },
            ],
            bonds: vec![
                Bond {
                    a: 0,
                    b: 1,
                    breaking_strain: 1.0,
                },
                Bond {
                    a: 1,
                    b: 2,
                    breaking_strain: 1.0,
                },
            ],
            anchors: vec![0],
            break_threshold: BREAKING_THRESHOLD,
        }
    }

    fn stone_mats(n: usize) -> Vec<u8> {
        vec![MaterialId::Stone.as_u8(); n]
    }

    #[test]
    fn graded_bake_repeats_x2() {
        let desc = test_desc();
        let table = MaterialTable::siege_kit();
        let params = GradeParams::with_seed(7);
        let a = grade_desc(&desc, &stone_mats(2), table, params).expect("grades");
        let b = grade_desc(&desc, &stone_mats(2), table, params).expect("grades");
        assert_eq!(graded_bytes(&a), graded_bytes(&b));
        assert_eq!(graded_digest(&a), graded_digest(&b));
        assert_ne!(a.bonds[0].breaking_strain.to_bits(), 1.0f32.to_bits());
    }

    #[test]
    fn insertion_shuffled_8_ways_same_graded() {
        let desc = test_desc();
        let table = MaterialTable::siege_kit();
        let params = GradeParams::with_seed(11);
        let base = grade_desc(&desc, &stone_mats(2), table, params).expect("base");
        let mut orders: Vec<Vec<usize>> = vec![vec![0, 1], vec![1, 0]];
        for seed in [1u64, 2, 3, 7, 42, 99] {
            let mut v = vec![0usize, 1usize];
            let mut s = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let j = usize::try_from(s >> 33).unwrap_or(0) % 2;
            if j == 1 {
                v.swap(0, 1);
            }
            let _ = s;
            orders.push(v);
        }
        assert_eq!(orders.len(), 8);
        for order in orders {
            let mut shuffled = desc.clone();
            shuffled.bonds = order.iter().map(|&i| desc.bonds[i]).collect();
            let mats: Vec<u8> = order.iter().map(|_| MaterialId::Stone.as_u8()).collect();
            let graded = grade_desc(&shuffled, &mats, table, params).expect("shuffled");
            assert_eq!(graded.bonds, base.bonds);
            assert_eq!(graded_digest(&graded), graded_digest(&base));
        }
    }

    #[test]
    fn anchored_chunks_hold_until_strain_fails() {
        let desc = test_desc();
        let table = MaterialTable::siege_kit();
        let params = GradeParams::with_seed(5);
        let graded = grade_desc(&desc, &stone_mats(2), table, params).expect("grades");
        let mut field = BondedCollection::new(&graded).expect("valid");
        field.step(1.0 / 60.0);
        assert!(field.freed_chunks().is_empty());
        let limit = graded.bonds[1].breaking_strain;
        field.add_strain(1, limit + 0.5, 9).expect("strain");
        assert_eq!(field.freed_chunks(), vec![2]);
        assert!(!field.freed_chunks().contains(&0));
    }

    #[test]
    fn jitter_is_deterministic_and_seeded() {
        let a = jitter_for_bond(7, 0, 8192);
        let b = jitter_for_bond(7, 0, 8192);
        assert_eq!(a, b);
        assert!(a.abs() <= 8192);
        assert_eq!(jitter_for_bond(7, 0, 0), 0);
        let other = jitter_for_bond(8, 0, 8192);
        assert_ne!(a, other);
    }

    #[test]
    fn sliver_reject_drops_tiny_keeps_crate() {
        let fat = Chunk3 {
            x: 0.0,
            y: 0.5,
            z: 0.0,
            hx: 0.5,
            hy: 0.5,
            hz: 0.5,
            mass: 1.0,
        };
        let thin = Chunk3 {
            x: 0.0,
            y: 0.5,
            z: 0.0,
            hx: 1e-5,
            hy: 0.5,
            hz: 0.5,
            mass: 1e-9,
        };
        assert!(!is_sliver(&fat));
        assert!(is_sliver(&thin));
        assert_eq!(reject_slivers(&[fat, thin]), vec![0]);
        let baked = fracture3(0.0, 2.0, 0.0, 1.0, 1.0, 1.0, 6, 11);
        assert_eq!(reject_slivers(&baked).len(), baked.len());
    }

    #[test]
    fn density_cells_clip_and_repeat_x2() {
        use crate::csg::PropMesh;
        let mesh = PropMesh {
            positions: vec![
                [-0.5, -0.5, -0.5],
                [0.5, -0.5, -0.5],
                [0.5, 0.5, -0.5],
                [-0.5, 0.5, -0.5],
                [-0.5, -0.5, 0.5],
                [0.5, -0.5, 0.5],
                [0.5, 0.5, 0.5],
                [-0.5, 0.5, 0.5],
            ],
            indices: vec![
                0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5, 0, 1, 5, 0,
                5, 4, 3, 7, 6, 3, 6, 2,
            ],
        };
        let (min, max) = ([-0.5, -0.5, -0.5], [0.5, 0.5, 0.5]);
        let hit = [0.1, 0.0, 0.0];
        let s1 = density_sites(&min, &max, hit, 8, 0.4, 3);
        let s2 = density_sites(&min, &max, hit, 8, 0.4, 3);
        assert_eq!(s1, s2);
        assert_eq!(s1[0], hit);
        let c1 = cut_density_cells(&mesh, &s1);
        let c2 = cut_density_cells(&mesh, &s2);
        assert_eq!(c1, c2);
        assert!(c1.len() >= 2);
        let boxes = density_boxes(&mesh, hit, 8, 0.4, 3);
        assert_eq!(boxes.len(), c1.len());
    }

    #[test]
    fn records_name_freed_chunks_and_digest_x2() {
        let desc = test_desc();
        let table = MaterialTable::siege_kit();
        let params = GradeParams::with_seed(9);
        let graded = grade_desc(&desc, &stone_mats(2), table, params).expect("grades");
        let run = || {
            let mut field = BondedCollection::new(&graded).expect("valid");
            field
                .add_strain(1, graded.bonds[1].breaking_strain + 1.0, 9)
                .expect("snap");
            let events = field.drain_events();
            let record = record_breaks(9, &events, field.len()).expect("records");
            (events, record.to_bytes(), record.digest())
        };
        let (e1, b1, d1) = run();
        let (e2, b2, d2) = run();
        assert_eq!(e1, e2);
        assert_eq!(b1, b2);
        assert_eq!(d1, d2);
        assert!(e1
            .iter()
            .any(|e| matches!(e, crate::bonded::BreakEvent::ChunkFreed { chunk: 2, .. })));
    }

    #[test]
    fn caps_16_1024_hold_in_soak() {
        use crate::budget::{arena_default_budget, DebrisLedger, ShardSeed};
        let cfg = arena_default_budget();
        assert_eq!(cfg.per_event_cap, 16);
        assert_eq!(cfg.max_shards, 1024);
        let run = || {
            let mut ledger = DebrisLedger::new(&cfg).expect("valid");
            let mut next: u32 = 0;
            for tick in 0..30u32 {
                for fighter in 0..8u32 {
                    if (tick + fighter * 7) % 10 == 0 {
                        let batch: Vec<ShardSeed> = (0..20)
                            .map(|k| ShardSeed {
                                id: next + k,
                                island: next,
                                pos_mm: [0, 3000, 0],
                                vel_mm_per_tick: [0, 0, 0],
                            })
                            .collect();
                        let report = ledger.admit(tick, &batch).expect("admit");
                        assert!(report.admitted <= 16);
                        next += 20;
                    }
                }
                ledger.step([0, 1500, 0]);
                assert!(ledger.len() <= 1024);
            }
            ledger.state_hash()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn bad_inputs_refuse() {
        let table = MaterialTable::siege_kit();
        let params = GradeParams::with_seed(1);
        assert_eq!(
            grade_desc(&test_desc(), &[MaterialId::Stone.as_u8()], table, params).unwrap_err(),
            GradeError::BadBondMaterial(1)
        );
        assert_eq!(
            GradeParams {
                seed: 1,
                jitter_span_q16: MAX_JITTER_SPAN_Q16 + 1
            }
            .validate()
            .unwrap_err(),
            GradeError::BadJitter
        );
        assert!(apply_sliver_reject(&BondedDesc {
            chunks: vec![Chunk3 {
                x: 0.0,
                y: 0.5,
                z: 0.0,
                hx: 1e-5,
                hy: 1e-5,
                hz: 1e-5,
                mass: 1e-9,
            }],
            bonds: vec![],
            anchors: vec![],
            break_threshold: BREAKING_THRESHOLD,
        })
        .is_err());
    }
}
