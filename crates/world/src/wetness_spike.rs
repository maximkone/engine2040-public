//! wetness_spike: EXPERIMENT integer virtual-pipes wetness on a Q16 sand tile.
//! Owner: engine-world (spike only, never the tick). Units: Q16 fixed point
//! for ground height and wetness, permille for traction. Determinism: Jacobi
//! snapshot plus fixed neighbour order, so any evaluation permutation gives
//! byte-equal heights and flux. Errors: none, all math saturates, out-of-tile
//! neighbours are skipped. Budget: 0.2 ms per step on the 16x16 fixture.
//!
//! Hypothesis: a saturated-flux pipe layer spreads blood-source wetness and
//! moves traction while staying order-stable and cheap. Ablation: with the
//! layer off the ground is exactly the crate lattice bake.

use engine_core::hash::Fnv64;

/// Tile side in texels for the spike fixture.
pub const TILE_SIDE: usize = 16;
/// Cell count for the spike fixture.
pub const TILE_CELLS: usize = TILE_SIDE * TILE_SIDE;
/// Spike seed, shared with the golden game hash scope.
pub const SPIKE_SEED: u64 = 2040;
/// Ticks per spike run.
pub const SPIKE_TICKS: u32 = 600;
/// Step budget in nanoseconds (0.2 ms).
pub const STEP_BUDGET_NS: u128 = 200_000;
/// Blood-source cell: the crater centre, so water ponds before it spreads.
pub const SOURCE_IDX: usize = 8 * TILE_SIDE + 8;
/// Canonical neighbour order: east, south, west, north.
pub const CANON_ORDER: [usize; 4] = [0, 1, 2, 3];

const CRATER_C: usize = 8;
const CRATER_R: usize = 2;
const CRATER_STEP_Q16: u32 = 1 << 16;
const SOURCE_RATE_Q16: u32 = 1 << 12;
const MIN_HEAD_Q16: u64 = 1 << 12;
const FLOW_SHIFT: u32 = 2;
const WET_CAP_Q16: u32 = 4 << 16;
// Sand rides at 1/1024 of water flow: the crater survives 600 ticks while
// the digest still records every grain moved.
const SED_SHIFT: u32 = 10;
/// Dry traction in permille.
pub const TRACTION_BASE: u16 = 1000;
const TRACTION_SHIFT: u32 = 9;

/// Q16 ground plus Q16 wetness plus per-cell directed flux for one sand tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WetTile {
    /// Ground height in Q16, row-major.
    pub ground: [u32; TILE_CELLS],
    /// Standing wetness in Q16, row-major.
    pub wet: [u32; TILE_CELLS],
    /// Outflow per cell per canonical direction, row-major cells by [`CANON_ORDER`].
    pub flux: [u32; TILE_CELLS * 4],
}

/// Per-step scratch space. Lives beside the tile so a step never allocates.
#[derive(Clone, Debug)]
pub struct Scratch {
    level: [u64; TILE_CELLS],
    wet0: [u32; TILE_CELLS],
    wet1: [u32; TILE_CELLS],
    inflow: [u32; TILE_CELLS],
    sed_in: [u32; TILE_CELLS],
    sed_out: [u32; TILE_CELLS],
}

impl Scratch {
    /// Fresh zeroed scratch space, no heap use.
    #[must_use]
    pub fn new() -> Self {
        Self {
            level: [0; TILE_CELLS],
            wet0: [0; TILE_CELLS],
            wet1: [0; TILE_CELLS],
            inflow: [0; TILE_CELLS],
            sed_in: [0; TILE_CELLS],
            sed_out: [0; TILE_CELLS],
        }
    }
}

impl Default for Scratch {
    fn default() -> Self {
        Self::new()
    }
}

/// Widen a small tile index to the signed lattice coordinate. Tile indices
/// stay far below the `i32` range, so the fallback never fires in practice.
fn to_i32(v: usize) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

/// Build the fixture tile: lattice heights in Q16, one crater, dry.
#[must_use]
pub fn tile_init(seed: u64, with_crater: bool) -> WetTile {
    let mut ground = [0u32; TILE_CELLS];
    for (idx, cell) in ground.iter_mut().enumerate() {
        let x = idx % TILE_SIDE;
        let y = idx / TILE_SIDE;
        let mut g = u32::from(crate::height(seed, to_i32(x), to_i32(y))) << 16;
        if with_crater {
            let r = x.abs_diff(CRATER_C).max(y.abs_diff(CRATER_C));
            if r <= CRATER_R {
                let steps = u32::try_from(CRATER_R + 1 - r).unwrap_or(0);
                g = g.saturating_sub(steps * CRATER_STEP_Q16);
            }
        }
        *cell = g;
    }
    WetTile {
        ground,
        wet: [0; TILE_CELLS],
        flux: [0; TILE_CELLS * 4],
    }
}

const fn neighbour(i: usize, d: usize) -> Option<usize> {
    let x = i % TILE_SIDE;
    let y = i / TILE_SIDE;
    // Canonical pipes: 0 east, 1 south, 2 west, 3 north. An underflow
    // wraps to `usize::MAX`, which the bound check below rejects.
    let (nx, ny) = match d {
        0 => (x + 1, y),
        1 => (x, y + 1),
        2 => (x.wrapping_sub(1), y),
        _ => (x, y.wrapping_sub(1)),
    };
    if nx >= TILE_SIDE || ny >= TILE_SIDE {
        None
    } else {
        Some(ny * TILE_SIDE + nx)
    }
}

/// Advance one wetness step. All outflows derive from the snapshot, so any
/// neighbour evaluation `order` (a permutation of 0..4) gives the same tile.
/// With `flux_on` false the tile is untouched: the dry-ground ablation case.
pub fn step(tile: &mut WetTile, scratch: &mut Scratch, order: &[usize; 4], flux_on: bool) {
    if !flux_on {
        return;
    }
    // Blood-source emission: the fixture's only wetness inlet.
    tile.wet[SOURCE_IDX] = tile.wet[SOURCE_IDX].saturating_add(SOURCE_RATE_Q16);
    // Snapshot levels and water; every decision below reads these only.
    for i in 0..TILE_CELLS {
        scratch.level[i] = u64::from(tile.ground[i]) + u64::from(tile.wet[i]);
        scratch.wet0[i] = tile.wet[i];
        scratch.inflow[i] = 0;
        scratch.sed_in[i] = 0;
        scratch.sed_out[i] = 0;
    }
    for i in 0..TILE_CELLS {
        // Desired outflow per canonical direction from the snapshot.
        let mut want = [0u32; 4];
        let mut slope = [0u64; 4];
        let mut total_want: u64 = 0;
        let mut total_slope: u64 = 0;
        for &d in order.iter() {
            debug_assert!(d < 4, "step order must permute 0..4");
            let Some(j) = neighbour(i, d) else {
                continue;
            };
            if scratch.level[i] > scratch.level[j] {
                let drop = scratch.level[i] - scratch.level[j];
                if drop > MIN_HEAD_Q16 {
                    slope[d] = drop;
                    total_slope += drop;
                    let w = u32::try_from(drop >> FLOW_SHIFT).unwrap_or(u32::MAX);
                    want[d] = w;
                    total_want += u64::from(w);
                }
            }
        }
        // Cap total outflow at the water on hand, scaled from the snapshot.
        let avail = u64::from(scratch.wet0[i]);
        let mut out = [0u32; 4];
        let mut out_sum: u64 = 0;
        if total_want > 0 && avail > 0 {
            if total_want <= avail {
                out = want;
                out_sum = total_want;
            } else {
                for (slot, &w) in out.iter_mut().zip(want.iter()) {
                    *slot = u32::try_from(u64::from(w) * avail / total_want).unwrap_or(u32::MAX);
                    out_sum += u64::from(*slot);
                }
            }
        }
        // Spill rule: water above the cap sheets downhill by slope share.
        let mut remain = avail - out_sum;
        if remain > u64::from(WET_CAP_Q16) && total_slope > 0 {
            let excess = remain - u64::from(WET_CAP_Q16);
            let mut spilled: u64 = 0;
            for (slot, &share) in out.iter_mut().zip(slope.iter()) {
                if share > 0 {
                    let g = excess * share / total_slope;
                    *slot = slot.saturating_add(u32::try_from(g).unwrap_or(u32::MAX));
                    spilled += g;
                }
            }
            out_sum += spilled;
            remain = avail - out_sum;
        }
        scratch.wet1[i] = u32::try_from(remain).unwrap_or(u32::MAX);
        for (d, (slot, &f)) in tile.flux[i * 4..i * 4 + 4]
            .iter_mut()
            .zip(out.iter())
            .enumerate()
        {
            *slot = f;
            if f == 0 {
                continue;
            }
            if let Some(j) = neighbour(i, d) {
                scratch.inflow[j] = scratch.inflow[j].saturating_add(f);
                let s = f >> SED_SHIFT;
                scratch.sed_out[i] = scratch.sed_out[i].saturating_add(s);
                scratch.sed_in[j] = scratch.sed_in[j].saturating_add(s);
            }
        }
    }
    // Jacobi commit: inflows and sediment land after every cell decided.
    for i in 0..TILE_CELLS {
        tile.wet[i] = scratch.wet1[i].saturating_add(scratch.inflow[i]);
        tile.ground[i] = tile.ground[i]
            .saturating_sub(scratch.sed_out[i])
            .saturating_add(scratch.sed_in[i]);
    }
}

/// FNV-1a digest over ground, wetness and flux in canonical order.
#[must_use]
pub fn digest(tile: &WetTile) -> u64 {
    let mut h = Fnv64::new();
    for &g in tile.ground.iter() {
        h.write_u32(g);
    }
    for &w in tile.wet.iter() {
        h.write_u32(w);
    }
    for &f in tile.flux.iter() {
        h.write_u32(f);
    }
    h.finish()
}

/// Traction in permille: dry ground grips at 1000, wet ground slips.
#[must_use]
pub fn traction_permille(tile: &WetTile, idx: usize) -> u16 {
    let cut = (tile.wet[idx] >> TRACTION_SHIFT).min(u32::from(TRACTION_BASE));
    TRACTION_BASE - u16::try_from(cut).unwrap_or(TRACTION_BASE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[global_allocator]
    static TEST_ALLOC: engine_core::alloc_probe::CountingAlloc =
        engine_core::alloc_probe::CountingAlloc;

    const ORDER_A: [usize; 4] = CANON_ORDER;
    const ORDER_B: [usize; 4] = [3, 2, 1, 0];
    const ORDER_C: [usize; 4] = [1, 3, 0, 2];

    fn run(seed: u64, crater: bool, flux_on: bool, order: &[usize; 4]) -> (WetTile, u64, u128) {
        let mut tile = tile_init(seed, crater);
        let mut scratch = Scratch::new();
        let start = Instant::now();
        for _ in 0..SPIKE_TICKS {
            step(&mut tile, &mut scratch, order, flux_on);
        }
        let avg_ns = start.elapsed().as_nanos() / u128::from(SPIKE_TICKS);
        let d = digest(&tile);
        (tile, d, avg_ns)
    }

    fn wet_total(tile: &WetTile) -> u64 {
        tile.wet.iter().map(|&w| u64::from(w)).sum()
    }

    #[test]
    fn wetness_pipes_x2_byte_equal() {
        let (a, digest_a, _) = run(SPIKE_SEED, true, true, &ORDER_A);
        let (b, digest_b, _) = run(SPIKE_SEED, true, true, &ORDER_A);
        assert_eq!(digest_a, digest_b, "same seed must give the same digest");
        assert_eq!(a.ground, b.ground, "heights must match byte for byte");
        assert_eq!(a.wet, b.wet, "wetness must match byte for byte");
        assert_eq!(a.flux, b.flux, "flux must match byte for byte");
        assert!(wet_total(&a) > 0, "the blood source must wet the tile");
        assert_ne!(
            a.ground,
            tile_init(SPIKE_SEED, true).ground,
            "wet flow must move sand"
        );
        println!(
            "wetness_pipes x2 digest={digest_a:016x} wet_total={}",
            wet_total(&a)
        );
    }

    #[test]
    fn wetness_pipes_shuffled_order_stable() {
        let (_, digest_a, _) = run(SPIKE_SEED, true, true, &ORDER_A);
        let (_, digest_b, _) = run(SPIKE_SEED, true, true, &ORDER_B);
        let (_, digest_c, _) = run(SPIKE_SEED, true, true, &ORDER_C);
        assert_eq!(digest_a, digest_b, "reversed order must not drift");
        assert_eq!(digest_a, digest_c, "shuffled order must not drift");
        println!("wetness_pipes shuffled digest={digest_a:016x} stable x3 orders");
    }

    #[test]
    fn wetness_pipes_ablation_flux_off_matches_bx17() {
        let (tile, _, _) = run(SPIKE_SEED, true, false, &ORDER_A);
        let init = tile_init(SPIKE_SEED, true);
        assert_eq!(tile.ground, init.ground, "dry heights must not move");
        assert_eq!(tile.wet, init.wet, "dry tile stays dry");
        assert_eq!(tile.flux, init.flux, "dry tile holds no flux");
        // The fixture ground is exactly the lattice bake plus the crater.
        let bare = tile_init(SPIKE_SEED, false);
        for y in 0..TILE_SIDE {
            for x in 0..TILE_SIDE {
                let want = u32::from(crate::height(SPIKE_SEED, x as i32, y as i32)) << 16;
                assert_eq!(bare.ground[y * TILE_SIDE + x], want);
            }
        }
        println!("wetness_pipes ablation flux-off equals BX17 heights exactly");
    }

    #[test]
    fn wetness_pipes_step_budget_and_zero_alloc() {
        let mut tile = tile_init(SPIKE_SEED, true);
        let mut scratch = Scratch::new();
        for _ in 0..10 {
            step(&mut tile, &mut scratch, &ORDER_A, true);
        }
        let ((), allocs) =
            engine_core::alloc_probe::measure(|| step(&mut tile, &mut scratch, &ORDER_A, true));
        assert_eq!(allocs.calls, 0, "a step must not touch the heap");
        let (_, _, avg_ns) = run(SPIKE_SEED, true, true, &ORDER_A);
        assert!(
            avg_ns <= STEP_BUDGET_NS,
            "step over budget: {avg_ns}ns > {}ns",
            STEP_BUDGET_NS
        );
        println!(
            "wetness_pipes step_ns={avg_ns} allocs=0 budget_ns={}",
            STEP_BUDGET_NS
        );
    }

    #[test]
    fn wetness_pipes_traction_slip_delta() {
        let (tile, _, _) = run(SPIKE_SEED, true, true, &ORDER_A);
        let dry = tile_init(SPIKE_SEED, true);
        assert_eq!(traction_permille(&dry, SOURCE_IDX), TRACTION_BASE);
        // Water is conserved (source only adds), so 2,457,600 Q16 units sit
        // on 256 cells: some cell holds 9,600+, a traction delta of 18+.
        let mut min = TRACTION_BASE;
        for i in 0..TILE_CELLS {
            min = min.min(traction_permille(&tile, i));
        }
        let delta = TRACTION_BASE - min;
        assert!(min < TRACTION_BASE, "wetness must cut traction somewhere");
        assert!(delta >= 10, "slip effect too small to matter: {delta}");
        println!(
            "wetness_pipes traction dry=1000 min={min} center={} delta={delta}",
            traction_permille(&tile, SOURCE_IDX)
        );
    }
}
