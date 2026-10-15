//! crack: siege-wall baked fracture plus crack spread on the bonded path.
//!
//! Consumer: siege walls in the arena match: one [`CrackWall::bake`]
//! offline, one [`CrackWall::strike`] per hit. Caller:
//! [`FractureCache::crack_strike`](crate::fracture::FractureCache::crack_strike)
//! runs the per-event leg so cache and spread agree in one place. Data
//! owner: `engine-destruction`. Units: meters, Q16 strain, ticks.
//! Determinism: table reads plus integer-only spread; the same bake, hits
//! and seeds give the same bytes and state hash on every PC. Errors:
//! [`CrackError`] wraps the fracture, spread and bonded refusals. Budget:
//! one cache read plus one bounded spread per strike; only the event vec
//! allocates.

use crate::bonded::{event_bytes, wire, BondedCollection, BondedDesc, BreakError, BreakEvent};
use crate::csg::{dist2, PropMesh};
use crate::fracture::{CacheKey, FractureCache, FractureError, FractureSpec};
use crate::spread::{
    spread_cracks, strain_to_fixed, SpreadBond, SpreadError, SpreadEvent, SpreadGraph,
};
use engine_core::Fnv64;

/// Why a crack strike was refused; user input never panics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CrackError {
    /// The baked cache, the cold probe or the bonded conversion refused.
    Fracture(FractureError),
    /// The fixed-point spread refused.
    Spread(SpreadError),
    /// The bonded descriptor or collection refused.
    Bonded(BreakError),
}

impl core::fmt::Display for CrackError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CrackError::Fracture(e) => write!(f, "crack fracture refused: {e}"),
            CrackError::Spread(e) => write!(f, "crack spread refused: {e}"),
            CrackError::Bonded(e) => write!(f, "crack bonded shape refused: {e}"),
        }
    }
}

impl std::error::Error for CrackError {}

impl From<FractureError> for CrackError {
    fn from(e: FractureError) -> Self {
        CrackError::Fracture(e)
    }
}

impl From<SpreadError> for CrackError {
    fn from(e: SpreadError) -> Self {
        CrackError::Spread(e)
    }
}

impl From<BreakError> for CrackError {
    fn from(e: BreakError) -> Self {
        CrackError::Bonded(e)
    }
}

/// What one wall strike observably produced: the bonded-path events plus
/// the integer receipts that pin them.
#[derive(Clone, Debug, PartialEq)]
pub struct CrackOutcome {
    /// Bond snaps then chunk frees, in canonical order, at the strike tick.
    pub events: Vec<BreakEvent>,
    /// Index of the cell holding the hit, into the cached fracture.
    pub hit_chunk: u32,
    /// Chunk count of the cached fracture.
    pub chunks: u32,
    /// Digest of the cached fracture bytes served for this hit.
    pub fracture_digest: u64,
    /// Digest of the spread event bytes.
    pub spread_digest: u64,
    /// Fold of the strike tick plus both digests; the event receipt.
    pub digest: u64,
    /// Wall hash after this strike; a stateless call repeats `digest`.
    pub state: u64,
}

/// Digest over the canonical bonded event bytes; equal events hash equal.
fn event_digest(events: &[BreakEvent]) -> u64 {
    let mut h = Fnv64::new();
    for event in events {
        h.write(&event_bytes(event));
    }
    h.finish()
}

/// Bonded form of one spread event; the shapes are field-identical, so the
/// conversion renames variants without touching payloads.
fn spread_to_break(event: &SpreadEvent) -> BreakEvent {
    match *event {
        SpreadEvent::BondBroken { tick, bond, strain } => {
            BreakEvent::BondBroken { tick, bond, strain }
        }
        SpreadEvent::ChunkFreed { tick, chunk } => BreakEvent::ChunkFreed { tick, chunk },
    }
}

/// Bond of `desc` whose chunk-center midpoint lies nearest `hit`; ties go
/// to the lowest bond index, `None` when the descriptor holds no bonds. A
/// non-finite hit compares false everywhere, so it deterministically takes
/// bond 0; cache hits are validated finite before this runs.
#[must_use]
pub fn nearest_bond(desc: &BondedDesc, hit: [f32; 3]) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_d = f32::INFINITY;
    for (i, bond) in desc.bonds.iter().enumerate() {
        // Endpoints resolved when the spread graph accepted this descriptor;
        // anything else is skipped so user input never panics.
        let (Ok(a), Ok(b)) = (usize::try_from(bond.a), usize::try_from(bond.b)) else {
            continue;
        };
        if a >= desc.chunks.len() || b >= desc.chunks.len() {
            continue;
        }
        let (x, y) = (&desc.chunks[a], &desc.chunks[b]);
        let mid = [(x.x + y.x) * 0.5, (x.y + y.y) * 0.5, (x.z + y.z) * 0.5];
        let d = dist2(&mid, &hit);
        // Strict `<` keeps the lowest index on ties.
        if d < best_d {
            best_d = d;
            best = Some(i);
        }
    }
    // A non-finite hit compares false everywhere; take bond 0 then.
    best.or_else(|| (!desc.bonds.is_empty()).then_some(0))
}

/// Spread graph over a bonded descriptor: the same pairs in canonical
/// order, limits converted to Q16. Rejects what the bonded shape rejects.
pub fn spread_graph(desc: &BondedDesc) -> Result<SpreadGraph, CrackError> {
    let mut bonds = Vec::with_capacity(desc.bonds.len());
    for (i, bond) in desc.bonds.iter().enumerate() {
        let threshold_q16 = strain_to_fixed(bond.breaking_strain)
            .map_err(|_| CrackError::Spread(SpreadError::BadBond(i)))?;
        bonds.push(SpreadBond {
            a: bond.a,
            b: bond.b,
            threshold_q16,
        });
    }
    SpreadGraph::new(desc.chunks.len(), bonds, desc.anchors.clone()).map_err(CrackError::Spread)
}

/// Bondless fallback: sever the chunk nearest `hit` through the bonded
/// collection, then free every chunk without an anchor path. With no bonds
/// the support flood reaches anchors only, so every unanchored chunk frees:
/// the same set the first step would free, computed here because the wall
/// owns no stepping collection.
fn sever_isolated(
    desc: &BondedDesc,
    hit: [f32; 3],
    tick: u32,
) -> Result<(Vec<BreakEvent>, u64), CrackError> {
    let mut field = BondedCollection::new(desc).map_err(CrackError::Bonded)?;
    let mut best = 0usize;
    let mut best_d = f32::INFINITY;
    for (i, chunk) in field.chunks().iter().enumerate() {
        let d = dist2(&[chunk.x, chunk.y, chunk.z], &hit);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    field.sever_chunk(best, tick).map_err(CrackError::Bonded)?;
    let mut events = field.drain_events();
    // Validated chunk counts sit far below u32::MAX.
    for i in 0..field.len() {
        if !field.is_anchored(i) {
            events.push(BreakEvent::ChunkFreed {
                tick,
                chunk: wire(i),
            });
        }
    }
    let digest = event_digest(&events);
    Ok((events, digest))
}

/// Crack spread over a bonded descriptor: the fixed-point flood from the
/// bond nearest `hit`, converted to bonded-path events plus their digest.
/// Bondless shapes sever the nearest chunk instead, so every validated
/// descriptor yields events through the same bonded path.
pub fn spread_for_desc(
    desc: &BondedDesc,
    hit: [f32; 3],
    seed_strain_q16: i64,
    tick: u32,
) -> Result<(Vec<BreakEvent>, u64), CrackError> {
    desc.validate().map_err(CrackError::Bonded)?;
    if desc.bonds.is_empty() {
        return sever_isolated(desc, hit, tick);
    }
    let graph = spread_graph(desc)?;
    // Bonds are non-empty here and `nearest_bond` falls back to bond 0 when
    // a non-finite hit compares false everywhere.
    let bond = nearest_bond(desc, hit).unwrap_or(0);
    let spread = spread_cracks(&graph, bond, seed_strain_q16, tick).map_err(CrackError::Spread)?;
    let events: Vec<BreakEvent> = spread.iter().map(spread_to_break).collect();
    let digest = event_digest(&events);
    Ok((events, digest))
}

/// Integer hash fold of one strike: previous state, tick and both digests
/// feed FNV-1a64, so a replay of the same strikes from the same bake lands
/// on the same state on every PC.
#[must_use]
pub fn fold_state(prev: u64, tick: u32, fracture_digest: u64, spread_digest: u64) -> u64 {
    let mut h = Fnv64::new();
    h.write_u64(prev);
    h.write_u32(tick);
    h.write_u64(fracture_digest);
    h.write_u64(spread_digest);
    h.finish()
}

/// Siege wall: one baked fracture cache plus the strains of the bonded
/// path, the strike tick and the chained integer hash state. Bake once
/// offline; a strike is one cache read plus one bounded spread.
#[derive(Clone, Debug)]
pub struct CrackWall {
    cache: FractureCache,
    breaking_strain: f32,
    break_threshold: f32,
    tick: u32,
    state: u64,
}

impl CrackWall {
    /// Bakes the wall cache and pins the strains against real baked chunks:
    /// one cached fracture converts to the bonded shape, so a bad strain or
    /// threshold refuses here, never in the match.
    pub fn bake(
        mesh: &PropMesh,
        spec: &FractureSpec,
        grid: u32,
        breaking_strain: f32,
        break_threshold: f32,
    ) -> Result<Self, CrackError> {
        let cache = FractureCache::bake(mesh, spec, grid).map_err(CrackError::Fracture)?;
        // Grid validation guarantees at least one cell, so key (0, 0, 0) resolves.
        let probe = cache
            .cold(
                mesh,
                CacheKey {
                    ix: 0,
                    iy: 0,
                    iz: 0,
                },
            )
            .map_err(CrackError::Fracture)?;
        probe
            .to_desc(breaking_strain, break_threshold)
            .map_err(CrackError::Fracture)?;
        let state = cache.digest();
        Ok(Self {
            cache,
            breaking_strain,
            break_threshold,
            tick: 0,
            state,
        })
    }

    /// Runtime strike at `hit` with `seed_strain_q16` of Q16 seed strain:
    /// the cached slice of the hit cell plus the fixed-point spread, then
    /// the wall state chains forward. No clip runs.
    pub fn strike(
        &mut self,
        hit: [f32; 3],
        seed_strain_q16: i64,
    ) -> Result<CrackOutcome, CrackError> {
        // Ticks count served strikes; saturation keeps the counter total
        // past 4B strikes without wrapping the hash input, and a refused
        // strike leaves the tick and the state untouched.
        let tick = self.tick.saturating_add(1);
        let mut outcome = self.cache.crack_strike(
            hit,
            self.breaking_strain,
            self.break_threshold,
            seed_strain_q16,
            tick,
        )?;
        outcome.state = fold_state(
            self.state,
            tick,
            outcome.fracture_digest,
            outcome.spread_digest,
        );
        self.tick = tick;
        self.state = outcome.state;
        Ok(outcome)
    }

    /// Strikes served so far.
    #[must_use]
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// Chained integer hash of the bake plus every strike served.
    #[must_use]
    pub fn state_hash(&self) -> u64 {
        self.state
    }

    /// Baked cache behind this wall, for cold-path comparisons.
    #[must_use]
    pub fn cache(&self) -> &FractureCache {
        &self.cache
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bake::BREAKING_THRESHOLD;

    /// Siege wall shell: a closed box `[-2, 2] x [0, 3] x [-0.2, 0.2]`, 12
    /// triangles, meters.
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

    fn spec(cells: u32, seed: u64) -> FractureSpec {
        FractureSpec {
            cells,
            radius_m: 0.8,
            seed,
        }
    }

    fn baked_wall() -> CrackWall {
        CrackWall::bake(&wall(), &spec(16, 3), 2, 1.0, BREAKING_THRESHOLD).expect("wall bakes")
    }

    fn q(strain: f32) -> i64 {
        strain_to_fixed(strain).expect("test strain must be valid")
    }

    #[test]
    fn strike_replays_identical_twice() {
        let mut first = baked_wall();
        let mut second = baked_wall();
        let a = first.strike([0.5, 1.5, 0.2], q(8.0)).expect("strikes");
        let b = second.strike([0.5, 1.5, 0.2], q(8.0)).expect("strikes");
        assert_eq!(a, b);
        assert_eq!(a.digest, b.digest);
        assert_eq!(first.state_hash(), second.state_hash());
        assert!(a.chunks >= 2, "a strike splits the wall: {a:?}");
        assert!(!a.events.is_empty(), "seed strain snaps bonds: {a:?}");
        assert_eq!(first.tick(), 1);
    }

    #[test]
    fn runtime_slice_matches_cold_cut() {
        let mesh = wall();
        let mut wall =
            CrackWall::bake(&mesh, &spec(16, 3), 2, 1.0, BREAKING_THRESHOLD).expect("wall bakes");
        let hit = [0.5, 1.5, 0.2];
        let key = wall.cache().key_of(hit).expect("hit inside");
        let cold = wall.cache().cold(&mesh, key).expect("cold cut");
        let cached = wall.cache().cut(hit).expect("cache hit");
        assert_eq!(
            cached.bytes(),
            cold.bytes(),
            "table read equals the cold cut"
        );
        let (cached_digest, cached_hit) = (cached.digest(), cached.hit_chunk);
        let outcome = wall.strike(hit, q(8.0)).expect("strikes");
        assert_eq!(outcome.fracture_digest, cached_digest);
        assert_eq!(outcome.hit_chunk, cached_hit);
    }

    #[test]
    fn spread_shape_bond_events_lead_at_the_strike_tick() {
        let mut wall = baked_wall();
        let outcome = wall.strike([0.5, 1.5, 0.2], q(8.0)).expect("strikes");
        let events = &outcome.events;
        assert!(!events.is_empty(), "seed strain past every limit snaps");
        assert!(
            events.iter().all(|e| match *e {
                BreakEvent::BondBroken { tick, .. } | BreakEvent::ChunkFreed { tick, .. } =>
                    tick == 1,
            }),
            "every event carries the strike tick: {events:?}"
        );
        let first_free = events
            .iter()
            .position(|e| matches!(e, BreakEvent::ChunkFreed { .. }))
            .unwrap_or(events.len());
        let (snaps, frees) = (&events[..first_free], &events[first_free..]);
        assert!(!snaps.is_empty(), "the hit bond snaps: {events:?}");
        assert!(
            snaps
                .iter()
                .all(|e| matches!(e, BreakEvent::BondBroken { .. })),
            "snaps lead in canonical bond order: {events:?}"
        );
        assert!(
            frees
                .iter()
                .all(|e| matches!(e, BreakEvent::ChunkFreed { .. })),
            "frees trail in chunk order: {events:?}"
        );
        let mut bonds: Vec<u32> = Vec::new();
        for e in snaps {
            if let BreakEvent::BondBroken { bond, .. } = *e {
                bonds.push(bond);
            }
        }
        let mut sorted = bonds.clone();
        sorted.sort_unstable();
        assert_eq!(bonds, sorted, "snaps run in ascending bond order");
    }

    #[test]
    fn another_hit_or_seed_another_digest() {
        let mut wall = baked_wall();
        let a = wall.strike([0.5, 1.5, 0.2], q(8.0)).expect("strikes");
        let mut wall = baked_wall();
        let b = wall.strike([-1.2, 2.0, 0.2], q(8.0)).expect("strikes");
        assert_ne!(a.digest, b.digest, "another hit, another cut");
        let mut wall = baked_wall();
        let c = wall.strike([0.5, 1.5, 0.2], q(9.0)).expect("strikes");
        assert_ne!(
            a.spread_digest, c.spread_digest,
            "another seed, another spread"
        );
    }

    #[test]
    fn bad_inputs_refuse() {
        let mesh = wall();
        assert_eq!(
            CrackWall::bake(&mesh, &spec(16, 3), 0, 1.0, BREAKING_THRESHOLD).unwrap_err(),
            CrackError::Fracture(FractureError::BadGrid)
        );
        assert_eq!(
            CrackWall::bake(&mesh, &spec(1, 3), 2, 1.0, BREAKING_THRESHOLD).unwrap_err(),
            CrackError::Fracture(FractureError::BadCells)
        );
        assert!(matches!(
            CrackWall::bake(&mesh, &spec(16, 3), 2, f32::NAN, BREAKING_THRESHOLD).unwrap_err(),
            CrackError::Fracture(_)
        ));
        let mut wall = baked_wall();
        assert_eq!(
            wall.strike([9.0, 0.0, 0.0], q(8.0)).unwrap_err(),
            CrackError::Fracture(FractureError::BadHit)
        );
        assert_eq!(
            wall.strike([0.5, 1.5, 0.2], -1).unwrap_err(),
            CrackError::Spread(SpreadError::BadStrain)
        );
        assert_eq!(wall.tick(), 0, "refused strikes serve nothing: {wall:?}");
    }

    #[test]
    fn bondless_fracture_severs_through_bonded() {
        use crate::bonded::Chunk3;
        fn chunk(x: f32) -> Chunk3 {
            Chunk3 {
                x,
                y: 2.0,
                z: 0.0,
                hx: 0.5,
                hy: 0.5,
                hz: 0.5,
                mass: 1.0,
            }
        }
        // Two disjoint chunks, no bonds: the hit chunk still frees through
        // the bonded sever path instead of refusing.
        let desc = BondedDesc {
            chunks: vec![chunk(0.0), chunk(10.0)],
            bonds: Vec::new(),
            anchors: vec![0],
            break_threshold: BREAKING_THRESHOLD,
        };
        let (events, digest) =
            spread_for_desc(&desc, [10.0, 2.0, 0.0], q(8.0), 4).expect("bondless severs");
        assert_eq!(
            events,
            vec![BreakEvent::ChunkFreed { tick: 4, chunk: 1 }],
            "only the struck chunk frees"
        );
        assert_eq!(digest, event_digest(&events));
        assert_eq!(nearest_bond(&desc, [10.0, 2.0, 0.0]), None);
    }

    #[test]
    fn state_chains_across_strikes() {
        let mut first = baked_wall();
        let a = first.strike([0.5, 1.5, 0.2], q(8.0)).expect("strikes");
        let b = first.strike([-1.2, 2.0, 0.2], q(8.0)).expect("strikes");
        assert_ne!(a.state, b.state, "the second strike chains forward");
        assert_eq!(b.state, first.state_hash());
        assert_eq!(first.tick(), 2);
        let mut second = baked_wall();
        second.strike([0.5, 1.5, 0.2], q(8.0)).expect("strikes");
        second.strike([-1.2, 2.0, 0.2], q(8.0)).expect("strikes");
        assert_eq!(
            first.state_hash(),
            second.state_hash(),
            "same strikes chain equal"
        );
    }

    #[test]
    fn fracture_leg_serves_the_same_outcome() {
        let mut wall = baked_wall();
        let hit = [0.5, 1.5, 0.2];
        let leg = wall
            .cache()
            .crack_strike(hit, 1.0, BREAKING_THRESHOLD, q(8.0), 1)
            .expect("leg strikes");
        let owned = wall.strike(hit, q(8.0)).expect("wall strikes");
        assert_eq!(leg.events, owned.events);
        assert_eq!(leg.digest, owned.digest);
        assert_eq!(leg.fracture_digest, owned.fracture_digest);
        assert_eq!(leg.spread_digest, owned.spread_digest);
    }

    #[test]
    fn nonfinite_hit_takes_bond_zero() {
        use crate::bonded::{Bond, BondedDesc, Chunk3};
        fn chunk(x: f32) -> Chunk3 {
            Chunk3 {
                x,
                y: 2.0,
                z: 0.0,
                hx: 0.5,
                hy: 0.5,
                hz: 0.5,
                mass: 1.0,
            }
        }
        let desc = BondedDesc {
            chunks: vec![chunk(0.0), chunk(10.0)],
            bonds: vec![Bond {
                a: 0,
                b: 1,
                breaking_strain: 1.0,
            }],
            anchors: vec![0],
            break_threshold: BREAKING_THRESHOLD,
        };
        // NaN compares false everywhere, so the documented rule takes bond 0.
        assert_eq!(nearest_bond(&desc, [f32::NAN, 2.0, 0.0]), Some(0));
    }

    #[test]
    fn nonfinite_hit_spreads_without_panic() {
        use crate::bonded::{Bond, BondedDesc, Chunk3};
        fn chunk(x: f32) -> Chunk3 {
            Chunk3 {
                x,
                y: 2.0,
                z: 0.0,
                hx: 0.5,
                hy: 0.5,
                hz: 0.5,
                mass: 1.0,
            }
        }
        let desc = BondedDesc {
            chunks: vec![chunk(0.0), chunk(10.0)],
            bonds: vec![Bond {
                a: 0,
                b: 1,
                breaking_strain: 1.0,
            }],
            anchors: vec![0],
            break_threshold: BREAKING_THRESHOLD,
        };
        assert!(
            spread_for_desc(&desc, [f32::NAN, 2.0, 0.0], q(8.0), 4).is_ok(),
            "a non-finite hit still spreads from bond 0"
        );
    }
}
