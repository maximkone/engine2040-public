//! Spawn/gate reachability over plain scene data.
//!
//! Consumer: [`crate::verify`] (the generation loop treats a rejection as a
//! compile error with the reason as the diagnostic). Data owner: the caller owns the
//! [`crate::ArenaVariant`]; this module copies what it needs into integer
//! centimeters and never retains references. Units: meters in, integer cm on
//! a 50 cm tile grid. Determinism scope: same variant in any input order
//! gives the same failures — inputs sort by id, sets are `BTreeSet`, BFS
//! order is frozen. Error type: [`crate::Failure`] rows, never panics.

use super::{ArenaVariant, Failure, Prop};
use std::collections::{BTreeSet, VecDeque};

/// This verifier's name in [`crate::Failure::verifier`].
pub const VERIFIER: &str = "SpawnReachability";

/// Tile edge in centimeters. 50 cm keeps the reference grid at 80x80 while
/// staying well under a fighter capsule diameter (80 cm).
const TILE_CM: i64 = 50;

/// Fighter walk band in centimeters: only props intersecting `[0, 180)`
/// block tiles, so chandeliers and banners never seal a gate.
const FIGHTER_HEIGHT_CM: i64 = 180;

/// Round meters to integer centimeters. Call only after a finiteness check.
fn to_cm(value: f32) -> i64 {
    (f64::from(value) * 100.0).round() as i64
}

/// Tile coordinate for a centimeter point (floors toward negative infinity).
fn tile_of(cm: i64) -> i64 {
    cm.div_euclid(TILE_CM)
}

/// One validated physical blocker footprint in centimeters.
struct Block {
    min_x: i64,
    max_x: i64,
    min_z: i64,
    max_z: i64,
}

fn fail(reason: String) -> Failure {
    Failure {
        verifier: VERIFIER,
        reason,
    }
}

/// True when the tile square overlaps the block square with nonzero area.
/// Edge-touching tiles stay walkable (flush walls nest, they do not seal).
fn tile_hits_block(tx: i64, tz: i64, block: &Block) -> bool {
    let (x0, x1) = (tx * TILE_CM, tx * TILE_CM + TILE_CM);
    let (z0, z1) = (tz * TILE_CM, tz * TILE_CM + TILE_CM);
    x0 < block.max_x && block.min_x < x1 && z0 < block.max_z && block.min_z < z1
}

/// True when the tile square overlaps the arena square with nonzero area.
fn tile_in_arena(tx: i64, tz: i64, bound_cm: i64) -> bool {
    let (x0, x1) = (tx * TILE_CM, tx * TILE_CM + TILE_CM);
    let (z0, z1) = (tz * TILE_CM, tz * TILE_CM + TILE_CM);
    x0 < bound_cm && -bound_cm < x1 && z0 < bound_cm && -bound_cm < z1
}

/// Tiles reachable from `start` over walkable in-arena tiles (4-neighborhood:
/// diagonal corner squeezes do not count as paths).
fn flood(start: (i64, i64), bound_cm: i64, blocked: &BTreeSet<(i64, i64)>) -> BTreeSet<(i64, i64)> {
    let mut seen = BTreeSet::new();
    if blocked.contains(&start) || !tile_in_arena(start.0, start.1, bound_cm) {
        return seen;
    }
    let mut queue = VecDeque::from([start]);
    seen.insert(start);
    while let Some((tx, tz)) = queue.pop_front() {
        for (nx, nz) in [(tx + 1, tz), (tx - 1, tz), (tx, tz + 1), (tx, tz - 1)] {
            if seen.contains(&(nx, nz))
                || blocked.contains(&(nx, nz))
                || !tile_in_arena(nx, nz, bound_cm)
            {
                continue;
            }
            seen.insert((nx, nz));
            queue.push_back((nx, nz));
        }
    }
    seen
}

/// True when the prop row has a finite center and a finite, strictly
/// positive size.
fn prop_valid(prop: &Prop) -> bool {
    let finite = prop.x.is_finite()
        && prop.y.is_finite()
        && prop.z.is_finite()
        && prop.sx.is_finite()
        && prop.sy.is_finite()
        && prop.sz.is_finite();
    finite && !(prop.sx <= 0.0 || prop.sy <= 0.0 || prop.sz <= 0.0)
}

/// Validated blockers: physical, valid rows, intersecting the walk band.
fn blockers(props: &[Prop]) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    for prop in props {
        if !prop.physical || !prop_valid(prop) {
            continue;
        }
        let min_y = to_cm(prop.y - prop.sy / 2.0);
        let max_y = to_cm(prop.y + prop.sy / 2.0);
        if max_y <= 0 || min_y >= FIGHTER_HEIGHT_CM {
            continue;
        }
        blocks.push(Block {
            min_x: to_cm(prop.x - prop.sx / 2.0),
            max_x: to_cm(prop.x + prop.sx / 2.0),
            min_z: to_cm(prop.z - prop.sz / 2.0),
            max_z: to_cm(prop.z + prop.sz / 2.0),
        });
    }
    blocks
}

/// Blocked tiles over the arena span plus a one-tile margin.
fn blocked_tiles(blocks: &[Block], bound_cm: i64) -> BTreeSet<(i64, i64)> {
    let span = tile_of(bound_cm) + 1;
    let mut blocked: BTreeSet<(i64, i64)> = BTreeSet::new();
    for tx in -span..=span {
        for tz in -span..=span {
            if blocks.iter().any(|b| tile_hits_block(tx, tz, b)) {
                blocked.insert((tx, tz));
            }
        }
    }
    blocked
}

/// Validate one spawn or gate (`kind` names it in the reason): finite and
/// inside the arena. Returns its id plus tile, or records why it is unusable.
fn endpoint_tile(
    kind: &str,
    (id, x, z): (u32, f32, f32),
    half_extent_m: f32,
    out: &mut Vec<Failure>,
) -> Option<(u32, (i64, i64))> {
    if !x.is_finite() || !z.is_finite() {
        out.push(fail(format!(
            "invalid {kind} {id}: finite coordinates required"
        )));
        return None;
    }
    if x.abs() > half_extent_m || z.abs() > half_extent_m {
        out.push(fail(format!(
            "{kind} {id} at ({x}, {z}) lies outside the arena"
        )));
        return None;
    }
    Some((id, (tile_of(to_cm(x)), tile_of(to_cm(z)))))
}

/// Check that every spawn reaches every gate. Returns one [`Failure`] per
/// invalid row or unreachable pair, id-sorted; empty means the variant is
/// walkable.
pub fn check(variant: &ArenaVariant) -> Vec<Failure> {
    let mut out: Vec<Failure> = Vec::new();
    if !variant.half_extent_m.is_finite() || variant.half_extent_m <= 0.0 {
        out.push(fail(format!(
            "invalid half extent {}: must be finite and positive",
            variant.half_extent_m
        )));
        return out;
    }
    let bound_cm = to_cm(variant.half_extent_m);

    if variant.spawns.is_empty() {
        out.push(fail(
            "no spawns: at least one spawn is required".to_string(),
        ));
    }
    if variant.gates.is_empty() {
        out.push(fail("no gates: at least one gate is required".to_string()));
    }

    // Id-sorted copies so input order never moves failures.
    let mut spawns = variant.spawns.clone();
    spawns.sort_by_key(|s| s.id);
    let mut gates = variant.gates.clone();
    gates.sort_by_key(|g| g.id);
    let mut props = variant.props.clone();
    props.sort_by_key(|p| p.id);

    for prop in &props {
        if !prop_valid(prop) {
            out.push(fail(format!(
                "invalid prop {} ({}): finite center and strictly positive size required",
                prop.id, prop.name
            )));
        }
    }

    let blocked = blocked_tiles(&blockers(&props), bound_cm);

    // Validated endpoints: finite, in bounds.
    let good_spawns: Vec<(u32, (i64, i64))> = spawns
        .iter()
        .filter_map(|s| endpoint_tile("spawn", (s.id, s.x, s.z), variant.half_extent_m, &mut out))
        .collect();
    let good_gates: Vec<(u32, (i64, i64))> = gates
        .iter()
        .filter_map(|g| endpoint_tile("gate", (g.id, g.x, g.z), variant.half_extent_m, &mut out))
        .collect();

    // Every spawn reaches every gate: one flood per spawn.
    for (spawn_id, start) in &good_spawns {
        if blocked.contains(start) {
            out.push(fail(format!("spawn {spawn_id} starts inside a blocker")));
            continue;
        }
        let reached = flood(*start, bound_cm, &blocked);
        for (gate_id, goal) in &good_gates {
            if !reached.contains(goal) {
                out.push(fail(format!(
                    "spawn {spawn_id} cannot reach gate {gate_id}"
                )));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::{
        reference_arena, ArenaVariant, BalanceConfig, BudgetCaps, Gate, Prop, Spawn,
    };
    use super::*;

    fn open_variant() -> ArenaVariant {
        ArenaVariant {
            name: "open".to_string(),
            half_extent_m: 10.0,
            spawns: vec![Spawn {
                id: 0,
                x: -5.0,
                z: 0.0,
            }],
            gates: vec![Gate {
                id: 1,
                x: 5.0,
                z: 0.0,
            }],
            props: Vec::new(),
            morale: vec![500, 500],
            balance: BalanceConfig {
                matches: 1,
                match_ticks: 60,
                base_seed: 7,
                min_win_permille: 0,
                max_win_permille: 1000,
            },
            budgets: BudgetCaps {
                max_draws: 64,
                max_triangles: 1_000_000,
                max_memory_bytes: 1_000_000,
            },
        }
    }

    /// Full-height wall at x = 0 spanning the arena: no path around it.
    fn sealed_wall(id: u32) -> Prop {
        Prop {
            id,
            name: "wall".to_string(),
            x: 0.0,
            y: 1.0,
            z: 0.0,
            sx: 1.0,
            sy: 2.0,
            sz: 20.0,
            physical: true,
            triangles: 12,
            memory_bytes: 64,
        }
    }

    #[test]
    fn reference_arena_is_walkable() {
        assert!(check(&reference_arena()).is_empty());
    }

    #[test]
    fn open_floor_reaches() {
        assert!(check(&open_variant()).is_empty());
    }

    #[test]
    fn sealed_gate_is_rejected() {
        let mut variant = open_variant();
        variant.props.push(sealed_wall(5));
        let failures = check(&variant);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].verifier, VERIFIER);
        assert!(
            failures[0].reason.contains("cannot reach"),
            "unexpected: {}",
            failures[0].reason
        );
    }

    #[test]
    fn every_spawn_must_reach_every_gate() {
        // Two spawns, two gates, one full-width divider at z = 2.5: each
        // spawn reaches its own side's gate but not the other side's. A
        // union flood would pass; per-spawn floods must fail both pairs.
        let mut variant = open_variant();
        variant.spawns.push(Spawn {
            id: 2,
            x: -5.0,
            z: 5.0,
        });
        variant.gates.push(Gate {
            id: 3,
            x: 5.0,
            z: 5.0,
        });
        variant.props.push(Prop {
            id: 5,
            name: "divider".to_string(),
            x: 0.0,
            y: 1.0,
            z: 2.5,
            sx: 20.0,
            sy: 2.0,
            sz: 1.0,
            physical: true,
            triangles: 12,
            memory_bytes: 64,
        });
        let failures = check(&variant);
        assert_eq!(failures.len(), 2, "{failures:?}");
        assert!(
            failures
                .iter()
                .any(|f| f.reason == "spawn 0 cannot reach gate 3"),
            "{failures:?}"
        );
        assert!(
            failures
                .iter()
                .any(|f| f.reason == "spawn 2 cannot reach gate 1"),
            "{failures:?}"
        );
    }

    #[test]
    fn spawn_inside_blocker_is_rejected() {
        let mut variant = open_variant();
        variant.props.push(Prop {
            id: 6,
            name: "crate".to_string(),
            x: -5.0,
            y: 0.5,
            z: 0.0,
            sx: 2.0,
            sy: 1.0,
            sz: 2.0,
            physical: true,
            triangles: 12,
            memory_bytes: 64,
        });
        let failures = check(&variant);
        assert!(
            failures
                .iter()
                .any(|f| f.reason.contains("inside a blocker")),
            "{failures:?}"
        );
    }

    #[test]
    fn floating_props_do_not_block() {
        // A banner 3 m up spans the arena but clears the walk band.
        let mut variant = open_variant();
        variant.props.push(Prop {
            id: 7,
            name: "banner".to_string(),
            x: 0.0,
            y: 3.0,
            z: 0.0,
            sx: 1.0,
            sy: 1.0,
            sz: 20.0,
            physical: true,
            triangles: 12,
            memory_bytes: 64,
        });
        assert!(check(&variant).is_empty());
    }

    #[test]
    fn bad_rows_fail_loud() {
        let mut variant = open_variant();
        variant.half_extent_m = f32::NAN;
        assert!(!check(&variant).is_empty());
        let mut variant = open_variant();
        variant.spawns = Vec::new();
        assert!(check(&variant)
            .iter()
            .any(|f| f.reason.contains("no spawns")));
        let mut variant = open_variant();
        variant.gates = Vec::new();
        assert!(check(&variant)
            .iter()
            .any(|f| f.reason.contains("no gates")));
        let mut variant = open_variant();
        variant.spawns[0].x = 50.0;
        assert!(check(&variant)
            .iter()
            .any(|f| f.reason.contains("outside the arena")));
        let mut variant = open_variant();
        variant.props.push(Prop {
            sx: 0.0,
            ..sealed_wall(8)
        });
        assert!(check(&variant)
            .iter()
            .any(|f| f.reason.contains("invalid prop 8")));
    }

    #[test]
    fn input_order_does_not_move_failures() {
        let mut variant = open_variant();
        variant.props.push(sealed_wall(5));
        variant.spawns.push(Spawn {
            id: 9,
            x: -5.0,
            z: 5.0,
        });
        let first = check(&variant);
        variant.spawns.reverse();
        variant.props.reverse();
        let second = check(&variant);
        assert_eq!(first, second);
    }
}
