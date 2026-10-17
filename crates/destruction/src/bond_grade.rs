//! bond_grade: graded bonds, sliver reject and per-event break records.
//!
//! Consumer: the bake path (`graded_fracture` after `fracture_at`) and the
//! door's break verbs; nothing here runs in the tick. Data owner:
//! `engine-destruction`. Units: meters, strain units of `bonded`, tile
//! indices of `tile_m` meters. Determinism: bonds are visited in canonical
//! `(min, max)` pair order and the jitter is keyed to the pair, so any
//! insertion order of the same bonds grades to the same strains; floats use
//! `+ - * /` only. Errors: [`GradeError`]. Budget: one pass over the
//! bonds plus a hop flood from the anchors; bake only.
//!
//! Grade rule: a bond is stronger where its contact face is wide, where the
//! pair is heavy (it carries more) and where it sits few hops from an
//! anchor (load paths run to the ground). Each term is clamped so one
//! outlier cannot dominate; a seeded jitter breaks up the lattice look.

use crate::bake::CONTACT_MARGIN;
use crate::bonded::{
    slot, wire, Bond, BondedDesc, BreakError, BreakEvent, Chunk3, MAX_BOND_STRAIN,
};
use crate::csg::PropMesh;
use crate::fracture::{fracture_at, Fracture, FractureError, FractureSpec};
use engine_core::checked::{saturating_f64_to_u64, saturating_i64_to_i32};
use engine_core::Rng;

/// Weakest strain a graded bond may carry; keeps every bond breakable.
pub const MIN_GRADED_STRAIN: f32 = 1e-3;
/// Largest tile count one record may list; bounds the output up front.
pub const MAX_DIRTY_TILES: usize = 4096;
/// Contact-area term clamp, as a multiple of the mean contact area.
const AREA_TERM: (f32, f32) = (0.5, 2.0);
/// Pair-mass term clamp, as a multiple of the mean pair mass.
const MASS_TERM: (f32, f32) = (0.5, 2.0);
/// Draw span of the jitter: offsets are `[-1, 1)` in 1/1024 steps.
const JITTER_SPAN: u64 = 2048;
/// A chunk whose middle extent is under this fraction of its longest is a
/// needle sliver.
const SLIVER_NEEDLE: f32 = 0.125;

/// What one grading pass asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradeParams {
    /// Strain of a mean bond before the anchor term and jitter; positive.
    pub base_strain: f32,
    /// Extra strength of an anchor-adjacent bond, as a multiple of the base;
    /// it halves with every hop away. Zero turns the term off.
    pub anchor_gain: f32,
    /// Jitter span as a fraction of the strain; `0.0..=0.5`.
    pub jitter: f32,
    /// Seed of the jitter draws.
    pub seed: u64,
}

impl GradeParams {
    /// Siege-wall grade the calibration corpus (`tests/grade_calib.rs`)
    /// measures: the anchor term lifts a foot bond to seven times the strain
    /// the same bond would hold far from any anchor.
    pub const SIEGE: Self = Self {
        base_strain: 2.0,
        anchor_gain: 6.0,
        jitter: 0.2,
        seed: 0,
    };

    fn validate(&self) -> Result<(), GradeError> {
        let base_ok = self.base_strain.is_finite()
            && self.base_strain >= MIN_GRADED_STRAIN
            && self.base_strain <= MAX_BOND_STRAIN;
        let gain_ok = self.anchor_gain.is_finite() && (0.0..=64.0).contains(&self.anchor_gain);
        let jitter_ok = self.jitter.is_finite() && (0.0..=0.5).contains(&self.jitter);
        if base_ok && gain_ok && jitter_ok {
            Ok(())
        } else {
            Err(GradeError::BadParams)
        }
    }
}

/// Why grading or recording was refused; input never panics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GradeError {
    /// A grade knob is out of range or not finite.
    BadParams,
    /// The tile size is not a positive finite number of meters.
    BadTile,
    /// The record would list more than [`MAX_DIRTY_TILES`] tiles.
    TooManyTiles,
    /// An event names a bond or chunk outside the descriptor.
    BadEvent,
    /// The cut left a sliver chunk after every retry.
    Sliver(usize),
    /// The descriptor was refused.
    Bonded(BreakError),
    /// The fracture was refused.
    Fracture(FractureError),
}

impl core::fmt::Display for GradeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GradeError::BadParams => write!(f, "grade knob out of range"),
            GradeError::BadTile => write!(f, "tile size must be positive and finite"),
            GradeError::TooManyTiles => write!(f, "more than {MAX_DIRTY_TILES} dirty tiles"),
            GradeError::BadEvent => write!(f, "event outside the descriptor"),
            GradeError::Sliver(n) => write!(f, "cut kept sliver chunk {n}"),
            GradeError::Bonded(e) => write!(f, "graded descriptor refused: {e}"),
            GradeError::Fracture(e) => write!(f, "graded fracture refused: {e}"),
        }
    }
}

impl std::error::Error for GradeError {}

impl From<BreakError> for GradeError {
    fn from(e: BreakError) -> Self {
        GradeError::Bonded(e)
    }
}

impl From<FractureError> for GradeError {
    fn from(e: FractureError) -> Self {
        GradeError::Fracture(e)
    }
}

/// Overlap of two boxes along each axis; negative inside the contact margin.
fn overlaps(a: &Chunk3, b: &Chunk3) -> [f32; 3] {
    let lo = |c: &Chunk3, k: usize| [c.x, c.y, c.z][k] - [c.hx, c.hy, c.hz][k];
    let hi = |c: &Chunk3, k: usize| [c.x, c.y, c.z][k] + [c.hx, c.hy, c.hz][k];
    let mut out = [0.0f32; 3];
    for (k, slot) in out.iter_mut().enumerate() {
        *slot = hi(a, k).min(hi(b, k)) - lo(a, k).max(lo(b, k));
    }
    out
}

/// Contact face area of two touching boxes: the product of the two widest
/// overlaps, the thinnest axis being the one they touch along. Meters^2.
#[must_use]
pub fn contact_area(a: &Chunk3, b: &Chunk3) -> f32 {
    let mut o = overlaps(a, b).map(|v| v.max(0.0));
    let thin = if o[0] <= o[1] && o[0] <= o[2] {
        0
    } else if o[1] <= o[2] {
        1
    } else {
        2
    };
    o[thin] = 1.0;
    o[0] * o[1] * o[2]
}

/// Bond hops from the nearest anchor to every chunk, `u32::MAX` when no
/// bond path reaches it. Breadth-first over the bond list, so index order
/// never changes the answer.
fn anchor_hops(chunks: usize, bonds: &[Bond], anchors: &[u32]) -> Vec<u32> {
    let mut hops = vec![u32::MAX; chunks];
    let mut frontier: Vec<usize> = Vec::new();
    for &a in anchors {
        if let Some(h) = hops.get_mut(slot(a)) {
            *h = 0;
            frontier.push(slot(a));
        }
    }
    let mut depth = 0u32;
    while !frontier.is_empty() {
        depth += 1;
        let mut next = Vec::new();
        for bond in bonds {
            let (a, b) = (slot(bond.a), slot(bond.b));
            for (from, to) in [(a, b), (b, a)] {
                if hops[from] == depth - 1 && hops[to] == u32::MAX {
                    hops[to] = depth;
                    next.push(to);
                }
            }
        }
        frontier = next;
    }
    hops
}

/// Seeded jitter factor `1 +- jitter` keyed to the chunk pair, so it moves
/// with the bond when the bond list is reordered.
fn jitter_factor(params: &GradeParams, a: u32, b: u32) -> f32 {
    let key = (u64::from(a.min(b)) << 32) | u64::from(a.max(b));
    let mut rng = Rng::new(params.seed).fork_named("bond-grade").fork(key);
    let draw = u16::try_from(rng.next_range(0, JITTER_SPAN)).unwrap_or(u16::MAX);
    1.0 + (f32::from(draw) / 1024.0 - 1.0) * params.jitter
}

/// Grades the bonds of `desc`: same pairs, same order, new strains from
/// contact area, pair mass and anchor distance with seeded jitter. Bakes
/// only; the result validates before it returns.
pub fn regrade(desc: &BondedDesc, params: &GradeParams) -> Result<BondedDesc, GradeError> {
    params.validate()?;
    desc.validate()?;
    // Canonical pair order fixes the float sums behind the means.
    let mut order: Vec<usize> = (0..desc.bonds.len()).collect();
    order.sort_by_key(|&i| {
        (
            desc.bonds[i].a.min(desc.bonds[i].b),
            desc.bonds[i].a.max(desc.bonds[i].b),
        )
    });
    let pair = |i: usize| {
        let bond = &desc.bonds[i];
        (&desc.chunks[slot(bond.a)], &desc.chunks[slot(bond.b)])
    };
    let (mut area_sum, mut mass_sum) = (0.0f32, 0.0f32);
    for &i in &order {
        let (a, b) = pair(i);
        area_sum += contact_area(a, b);
        mass_sum += a.mass + b.mass;
    }
    let count = f32::from(u16::try_from(order.len().max(1)).unwrap_or(u16::MAX));
    let (area_mean, mass_mean) = ((area_sum / count).max(1e-9), (mass_sum / count).max(1e-9));
    let hops = anchor_hops(desc.chunks.len(), &desc.bonds, &desc.anchors);
    let mut graded = desc.clone();
    for &i in &order {
        let (a, b) = pair(i);
        let area_term = (contact_area(a, b) / area_mean).clamp(AREA_TERM.0, AREA_TERM.1);
        let mass_term = ((a.mass + b.mass) / mass_mean).clamp(MASS_TERM.0, MASS_TERM.1);
        let near = hops[slot(desc.bonds[i].a)].min(hops[slot(desc.bonds[i].b)]);
        // Halving per hop: past 24 hops the term is zero in f32 anyway.
        let anchor_term = 1.0 + params.anchor_gain / f32::from(1u16 << near.min(15));
        let jitter = jitter_factor(params, desc.bonds[i].a, desc.bonds[i].b);
        let strain = params.base_strain * area_term * mass_term * anchor_term * jitter;
        graded.bonds[i].breaking_strain = strain.clamp(MIN_GRADED_STRAIN, MAX_BOND_STRAIN);
    }
    graded.validate()?;
    Ok(graded)
}

/// Chunk indices that are slivers: a needle whose middle extent is under an
/// eighth of its longest, or a speck whose longest half extent is under the
/// contact margin (it would bond to everything near it). Plates are fine: a
/// shell patch is thin by nature. Index order.
#[must_use]
pub fn sliver_chunks(chunks: &[Chunk3]) -> Vec<u32> {
    let mut out = Vec::new();
    for (i, c) in chunks.iter().enumerate() {
        let mut e = [c.hx, c.hy, c.hz];
        e.sort_by(f32::total_cmp);
        if e[1] < e[2] * SLIVER_NEEDLE || e[2] < CONTACT_MARGIN {
            out.push(wire(i));
        }
    }
    out
}

/// One graded cut: the Voronoi fracture at `hit` (the existing split code),
/// converted to a bonded descriptor and graded. A cut that leaves a sliver
/// chunk retries with the next seed, up to `tries` cuts, then refuses with
/// the first sliver of the last cut. Returns the fracture that was kept.
pub fn graded_fracture(
    mesh: &PropMesh,
    hit: [f32; 3],
    spec: &FractureSpec,
    break_threshold: f32,
    params: &GradeParams,
    tries: u32,
) -> Result<(Fracture, BondedDesc), GradeError> {
    let mut last = 0usize;
    for attempt in 0..tries.max(1) {
        let try_spec = FractureSpec {
            seed: spec.seed.wrapping_add(u64::from(attempt)),
            ..*spec
        };
        let fracture = fracture_at(mesh, hit, &try_spec)?;
        let desc = fracture.to_desc(params.base_strain, break_threshold)?;
        if let Some(&first) = sliver_chunks(&desc.chunks).first() {
            last = slot(first);
            continue;
        }
        let graded = regrade(&desc, params)?;
        return Ok((fracture, graded));
    }
    Err(GradeError::Sliver(last))
}

/// What one break event did: the bonds that snapped, the chunks that came
/// free and the nav tiles whose walkable surface changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BreakRecord {
    /// Snapped bond ids, ascending.
    pub bonds: Vec<u32>,
    /// Freed chunk ids, ascending.
    pub freed: Vec<u32>,
    /// Dirty `[tile_x, tile_z]` indices, sorted and unique.
    pub dirty_tiles: Vec<[i32; 2]>,
}

/// Floor of a coordinate ratio as `i32`, saturating at +-1e9; the shift keeps
/// the saturating unsigned conversion exact for negatives.
pub(crate) fn floor_to_i32(v: f64) -> i32 {
    const SHIFT: f64 = 1e9;
    let shifted = saturating_f64_to_u64(v.floor().clamp(-SHIFT, SHIFT) + SHIFT);
    saturating_i64_to_i32(i64::try_from(shifted).unwrap_or(0) - 1_000_000_000)
}

/// Tile index range of the ground footprint of a box; floor division so
/// negative coordinates tile consistently.
fn footprint_tiles(c: &Chunk3, tile_m: f32, out: &mut Vec<[i32; 2]>) -> Result<(), GradeError> {
    let tile = |v: f32| floor_to_i32(f64::from(v) / f64::from(tile_m));
    let (x0, x1) = (tile(c.x - c.hx), tile(c.x + c.hx));
    let (z0, z1) = (tile(c.z - c.hz), tile(c.z + c.hz));
    let cells = (i64::from(x1) - i64::from(x0) + 1) * (i64::from(z1) - i64::from(z0) + 1);
    let room = i64::try_from(MAX_DIRTY_TILES - out.len()).unwrap_or(0);
    if cells > room {
        return Err(GradeError::TooManyTiles);
    }
    for x in x0..=x1 {
        for z in z0..=z1 {
            out.push([x, z]);
        }
    }
    Ok(())
}

/// Record of one break event against the baked `desc`: which bonds and
/// chunks changed, and which `tile_m` nav tiles the broken boxes cover (the
/// freed chunk, or both ends of a snapped bond). Feeds NV3's tile rebuilds.
pub fn break_record(
    desc: &BondedDesc,
    events: &[BreakEvent],
    tile_m: f32,
) -> Result<BreakRecord, GradeError> {
    if !tile_m.is_finite() || tile_m <= 0.0 {
        return Err(GradeError::BadTile);
    }
    let (mut bonds, mut freed, mut tiles) = (Vec::new(), Vec::new(), Vec::new());
    for event in events {
        match *event {
            BreakEvent::BondBroken { bond, .. } => {
                let ends = desc.bonds.get(slot(bond)).ok_or(GradeError::BadEvent)?;
                for end in [ends.a, ends.b] {
                    footprint_tiles(&desc.chunks[slot(end)], tile_m, &mut tiles)?;
                }
                bonds.push(bond);
            }
            BreakEvent::ChunkFreed { chunk, .. } => {
                let c = desc.chunks.get(slot(chunk)).ok_or(GradeError::BadEvent)?;
                footprint_tiles(c, tile_m, &mut tiles)?;
                freed.push(chunk);
            }
        }
    }
    bonds.sort_unstable();
    freed.sort_unstable();
    tiles.sort_unstable();
    tiles.dedup();
    Ok(BreakRecord {
        bonds,
        freed,
        dirty_tiles: tiles,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::BREAKING_THRESHOLD;
    use crate::bonded::BondedCollection;
    use crate::budget::{arena_default_budget, DebrisLedger, ShardSeed};
    use crate::crack::spread_graph;
    use crate::spread::spread_cracks;
    use crate::structure::{building, BuildingSpec};

    fn params() -> GradeParams {
        GradeParams {
            base_strain: 2.0,
            anchor_gain: 2.0,
            jitter: 0.2,
            seed: 7,
        }
    }

    /// Siege wall shell, `[-2, 2] x [0, 3] x [-0.2, 0.2]`, meters.
    fn wall() -> PropMesh {
        let (x, y, z) = (2.0f32, 3.0f32, 0.2f32);
        PropMesh {
            positions: vec![
                [-x, 0.0, -z],
                [x, 0.0, -z],
                [x, y, -z],
                [-x, y, -z],
                [-x, 0.0, z],
                [x, 0.0, z],
                [x, y, z],
                [-x, y, z],
            ],
            indices: vec![
                0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 4, 7, 0, 7, 3, 1, 2, 6, 1, 6, 5, 0, 1, 5, 0,
                5, 4, 3, 7, 6, 3, 6, 2,
            ],
        }
    }

    fn frame() -> BondedDesc {
        let spec = BuildingSpec {
            stories: 2,
            bays_x: 3,
            bays_z: 2,
            knockouts: 0,
            ..BuildingSpec::three_story(1)
        };
        building(&spec).expect("frame builds").desc
    }

    fn strains(desc: &BondedDesc) -> Vec<u32> {
        desc.bonds
            .iter()
            .map(|b| b.breaking_strain.to_bits())
            .collect()
    }

    #[test]
    fn grading_repeats_and_the_seed_matters() {
        let desc = frame();
        let first = regrade(&desc, &params()).expect("grades");
        assert_eq!(
            strains(&first),
            strains(&regrade(&desc, &params()).expect("grades"))
        );
        let other = GradeParams {
            seed: 8,
            ..params()
        };
        assert_ne!(
            strains(&first),
            strains(&regrade(&desc, &other).expect("grades"))
        );
        assert!(first
            .bonds
            .iter()
            .all(|b| b.breaking_strain >= MIN_GRADED_STRAIN));
        let uniform: Vec<u32> = desc
            .bonds
            .iter()
            .map(|b| b.breaking_strain.to_bits())
            .collect();
        assert_ne!(strains(&first), uniform, "grading must change the strains");
    }

    #[test]
    fn strain_follows_anchor_distance_and_area() {
        let desc = frame();
        let with = |gain: f32| {
            let p = GradeParams {
                anchor_gain: gain,
                jitter: 0.0,
                ..params()
            };
            regrade(&desc, &p).expect("grades")
        };
        let (off, on) = (with(0.0), with(2.0));
        let hops = anchor_hops(desc.chunks.len(), &desc.bonds, &desc.anchors);
        let mut seen = [false; 4];
        for (i, bond) in desc.bonds.iter().enumerate() {
            let near = hops[bond.a as usize].min(hops[bond.b as usize]);
            let ratio = on.bonds[i].breaking_strain / off.bonds[i].breaking_strain;
            let want = 1.0 + 2.0 / f32::from(1u16 << near.min(15));
            assert!(
                (ratio - want).abs() < 1e-3 * want,
                "hop {near}: {ratio} vs {want}"
            );
            if let Some(slot) = seen.get_mut(near as usize) {
                *slot = true;
            }
        }
        assert!(
            seen[0] && seen[1] && seen[2],
            "the frame spans several anchor distances"
        );
        let wide = Chunk3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            hx: 1.0,
            hy: 1.0,
            hz: 1.0,
            mass: 8.0,
        };
        let on_top = Chunk3 { y: 2.0, ..wide };
        let small = Chunk3 {
            y: 1.5,
            hx: 0.25,
            hz: 0.25,
            hy: 0.5,
            ..wide
        };
        assert!(contact_area(&wide, &on_top) > contact_area(&wide, &small));
    }

    #[test]
    fn graded_foot_bonds_hold_a_strain_that_breaks_uniform_ones() {
        let desc = frame();
        let graded = regrade(
            &desc,
            &GradeParams {
                base_strain: 3.0,
                anchor_gain: 3.0,
                jitter: 0.2,
                seed: 1,
            },
        )
        .expect("grades");
        let uniform = BondedDesc {
            bonds: desc
                .bonds
                .iter()
                .map(|b| Bond {
                    breaking_strain: 1.0,
                    ..*b
                })
                .collect(),
            ..desc.clone()
        };
        let foot: Vec<usize> = (0..desc.bonds.len())
            .filter(|&i| {
                desc.anchors.contains(&desc.bonds[i].a) || desc.anchors.contains(&desc.bonds[i].b)
            })
            .collect();
        assert!(!foot.is_empty());
        let snapped = |d: &BondedDesc, strain: f32| {
            let mut field = BondedCollection::new(d).expect("collection");
            for &bond in &foot {
                field.add_strain(bond, strain, 1).expect("strain");
            }
            let broken = field.broken_bonds().len();
            let anchors_hold = desc.anchors.iter().all(|&a| field.is_anchored(a as usize));
            (broken, anchors_hold)
        };
        assert_eq!(
            snapped(&uniform, 1.5).0,
            foot.len(),
            "uniform 1.0 loses every foot bond"
        );
        assert_eq!(
            snapped(&graded, 1.5).0,
            0,
            "graded foot bonds hold until their strain fails"
        );
        let (broken, anchors_hold) = snapped(&graded, 1e4);
        assert_eq!(broken, foot.len());
        assert!(anchors_hold, "an anchor never frees, whatever its bonds do");
    }

    #[test]
    fn eight_insertion_orders_grade_and_spread_equal() {
        let desc = frame();
        let reference = regrade(&desc, &params()).expect("grades");
        let spread_of = |d: &BondedDesc| {
            let graph = spread_graph(d).expect("graph");
            let hit = graph.bond_count() / 2;
            let events = spread_cracks(&graph, hit, 3 << 16, 5).expect("spread");
            (graph.bonds().to_vec(), events)
        };
        let want = spread_of(&reference);
        for k in 1..=8usize {
            let mut shuffled = desc.clone();
            let n = shuffled.bonds.len();
            shuffled.bonds.rotate_left(k * 7 % n);
            if k % 2 == 0 {
                shuffled.bonds.reverse();
            }
            let got = regrade(&shuffled, &params()).expect("grades");
            let mut left: Vec<_> = got
                .bonds
                .iter()
                .map(|b| (b.a.min(b.b), b.a.max(b.b), b.breaking_strain.to_bits()))
                .collect();
            let mut right: Vec<_> = reference
                .bonds
                .iter()
                .map(|b| (b.a.min(b.b), b.a.max(b.b), b.breaking_strain.to_bits()))
                .collect();
            left.sort_unstable();
            right.sort_unstable();
            assert_eq!(left, right, "order {k}: strains follow the pair");
            let (bonds, events) = spread_of(&got);
            assert_eq!(bonds, want.0, "order {k}: spread graph");
            assert_eq!(events.len(), want.1.len(), "order {k}: events");
        }
    }

    #[test]
    fn graded_cut_rejects_slivers_and_replays() {
        let spec = FractureSpec {
            cells: 16,
            radius_m: 0.8,
            seed: 3,
        };
        let (fracture, desc) = graded_fracture(
            &wall(),
            [0.3, 1.2, 0.0],
            &spec,
            BREAKING_THRESHOLD,
            &params(),
            8,
        )
        .expect("graded cut");
        assert!(sliver_chunks(&desc.chunks).is_empty());
        assert_eq!(fracture.chunks.len(), desc.chunks.len());
        let (again, desc2) = graded_fracture(
            &wall(),
            [0.3, 1.2, 0.0],
            &spec,
            BREAKING_THRESHOLD,
            &params(),
            8,
        )
        .expect("graded cut");
        assert_eq!(fracture.digest(), again.digest());
        assert_eq!(strains(&desc), strains(&desc2));
        let flat = Chunk3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            hx: 1.0,
            hy: 1.0,
            hz: 1.0,
            mass: 8.0,
        };
        let needle = Chunk3 {
            hy: 0.01,
            hz: 0.01,
            ..flat
        };
        assert_eq!(sliver_chunks(&[flat, flat, flat, needle]), vec![3]);
        assert_eq!(
            graded_fracture(
                &wall(),
                [0.0; 3],
                &spec,
                BREAKING_THRESHOLD,
                &GradeParams {
                    jitter: 0.9,
                    ..params()
                },
                1
            )
            .unwrap_err(),
            GradeError::BadParams
        );
    }

    #[test]
    fn break_record_lists_what_broke_and_the_dirty_tiles() {
        let desc = frame();
        let graded = regrade(&desc, &params()).expect("grades");
        let mut field = BondedCollection::new(&graded).expect("collection");
        let strike = desc.chunks.len() / 2;
        field.apply_impact(strike, 1e5, 1).expect("impact");
        field.step(0.016);
        let events = field.drain_events();
        assert!(!events.is_empty());
        let record = break_record(&graded, &events, 2.0).expect("record");
        assert_eq!(record, break_record(&graded, &events, 2.0).expect("record"));
        assert!(!record.bonds.is_empty());
        assert!(record.bonds.windows(2).all(|w| w[0] < w[1]));
        assert!(record.dirty_tiles.windows(2).all(|w| w[0] < w[1]));
        let c = &graded.chunks[strike];
        let tile = [(c.x / 2.0).floor() as i32, (c.z / 2.0).floor() as i32];
        assert!(
            record.dirty_tiles.contains(&tile),
            "the struck footprint is dirty"
        );
        assert_eq!(
            break_record(&graded, &events, 0.0).unwrap_err(),
            GradeError::BadTile
        );
        let bad = [BreakEvent::ChunkFreed {
            tick: 1,
            chunk: 9999,
        }];
        assert_eq!(
            break_record(&graded, &bad, 2.0).unwrap_err(),
            GradeError::BadEvent
        );
        assert_eq!(
            break_record(&graded, &events, 1e-5).unwrap_err(),
            GradeError::TooManyTiles
        );
    }

    /// Eight fighters, ten graded cuts each: the 16 per event and 1024 total
    /// debris caps hold, and the cuts that overflow are counted, not kept.
    #[test]
    fn the_eight_fighter_soak_holds_the_debris_caps() {
        let cfg = arena_default_budget();
        let mut ledger = DebrisLedger::new(&cfg).expect("ledger");
        let mut next_id = 0u32;
        for event in 0..80u32 {
            let spec = FractureSpec {
                cells: 24,
                radius_m: 0.8,
                seed: u64::from(event),
            };
            let hit = [(event % 7) as f32 * 0.4 - 1.2, 1.0, 0.0];
            let (_, desc) = graded_fracture(&wall(), hit, &spec, BREAKING_THRESHOLD, &params(), 8)
                .expect("graded cut");
            let seeds: Vec<ShardSeed> = desc
                .chunks
                .iter()
                .map(|c| {
                    next_id += 1;
                    ShardSeed {
                        id: next_id,
                        island: event,
                        pos_mm: [
                            (c.x * 1000.0) as i32,
                            (c.y * 1000.0) as i32,
                            (c.z * 1000.0) as i32,
                        ],
                        vel_mm_per_tick: [0, 20, 0],
                    }
                })
                .collect();
            let report = ledger.admit(event, &seeds).expect("admit");
            assert!(report.admitted <= cfg.per_event_cap);
            assert!(ledger.len() <= cfg.max_shards as usize);
            ledger.step([0, 1500, 6000]);
        }
        assert_eq!(
            ledger.len(),
            cfg.max_shards as usize,
            "the soak fills the ledger"
        );
        assert!(ledger.dropped_total() > 0);
    }
}
