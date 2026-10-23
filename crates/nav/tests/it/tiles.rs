//! tiles acceptance: breach-to-path, identical bytes, legion steering.
//! Drives the public tiles API the way the arena match will: flip cells
//! for breaks, rebake dirty tiles on the tick, refresh goal fields, then
//! steer agents on the baked flow. Timing gates stay ignored here and
//! run under the release perf gate.

#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "timing test reads the wall clock; it never enters the tick"
)]

use std::time::{Duration, Instant};

use engine_nav::mesh::{BakeConfig, NavError, NavMesh, TiledBake};
use engine_nav::tiles::{BreakSet, ObstacleCache, TileError, TileId, TiledNav, TICKS_PER_SECOND};

fn walled(w: usize, h: usize, wall_x: usize, gap_y: Option<usize>) -> Vec<bool> {
    let mut blocked = vec![false; w * h];
    for y in 0..h {
        if Some(y) != gap_y {
            blocked[y * w + wall_x] = true;
        }
    }
    blocked
}

#[test]
fn broken_wall_opens_a_path_within_one_second_of_ticks() {
    let (w, h) = (24, 16);
    let start = (0usize, 8usize);
    let goal = (23usize, 8usize);
    let mut nav = TiledNav::bake(&walled(w, h, 12, None), w, h, 4).expect("walled grid bakes");
    nav.bake_goal(goal).expect("goal bakes");
    assert_eq!(nav.path_open(goal, start), Ok(false));

    // Tick zero: the wall breaks open one cell wide.
    nav.set_blocked(12, 8, false).expect("breach flips");
    let mut ticks = 0u32;
    loop {
        nav.rebake_budget(usize::MAX);
        nav.bake_goal(goal).expect("field refreshes");
        ticks += 1;
        if nav.path_open(goal, start).expect("field answers") {
            break;
        }
        assert!(ticks < TICKS_PER_SECOND, "path must open in one second");
    }
    assert!(ticks <= TICKS_PER_SECOND, "opened at tick {ticks}");

    let mut pos = start;
    for _ in 0..w * h {
        if pos == goal {
            break;
        }
        pos = nav
            .direction(goal, pos)
            .expect("field answers")
            .expect("downhill holds");
    }
    assert_eq!(pos, goal);
}

#[test]
fn shuffled_break_order_gives_identical_bytes_twice() {
    let (w, h) = (16, 12);
    let goal = (15usize, 11usize);
    let mut first = TiledNav::bake(&walled(w, h, 8, Some(5)), w, h, 4).expect("bakes");
    let mut second = TiledNav::bake(&walled(w, h, 8, Some(5)), w, h, 4).expect("bakes");
    // Two batches name the same tiles through different chunk orders.
    let one = BreakSet::from_chunks(3, &[30, 10, 20], 12, |c| Some(TileId(c % 12)))
        .expect("batch builds");
    let two = BreakSet::from_chunks(3, &[20, 30, 10], 12, |c| Some(TileId(c % 12)))
        .expect("batch builds");
    assert_eq!(one, two, "order never leaks into the set");
    assert_eq!(first.apply_breaks(&one), second.apply_breaks(&two));
    // Breaches land in opposite cell order on both grids.
    first.set_blocked(8, 4, false).expect("breach flips");
    first.set_blocked(8, 6, false).expect("breach flips");
    second.set_blocked(8, 6, false).expect("breach flips");
    second.set_blocked(8, 4, false).expect("breach flips");
    first.rebake_budget(usize::MAX);
    second.rebake_budget(usize::MAX);
    first.bake_goal(goal).expect("refreshes");
    second.bake_goal(goal).expect("refreshes");
    assert_eq!(first.to_bytes(), second.to_bytes(), "bytes x2");
    assert_eq!(first.digest(), second.digest());
    let back = TiledNav::from_bytes(&first.to_bytes()).expect("decodes");
    assert_eq!(back, first);
}

#[test]
fn legion_of_500_steps_on_one_shared_field() {
    let (w, h) = (32, 32);
    let mut nav = TiledNav::bake(&vec![false; w * h], w, h, 8).expect("open grid bakes");
    let goal = (31usize, 31usize);
    nav.bake_goal(goal).expect("goal bakes");
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut agents = Vec::with_capacity(500);
    while agents.len() < 500 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let x = (state >> 33) as usize % w;
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let y = (state >> 33) as usize % h;
        if (x, y) == goal {
            continue;
        }
        agents.push((x, y));
    }
    let mut out = vec![(0usize, 0usize); agents.len()];
    let moved = nav
        .steer_legion(goal, &agents, &mut out)
        .expect("legion steps");
    assert!(moved > 0, "the field carries the legion");
    for (agent, stepped) in agents.iter().zip(out.iter()) {
        match nav.direction(goal, *agent).expect("field answers") {
            Some(next) => assert_eq!(next, *stepped, "batch matches single steps"),
            None => assert_eq!(*agent, *stepped, "holders hold"),
        }
    }
    let mut short = vec![(0usize, 0usize); agents.len() - 1];
    assert_eq!(
        nav.steer_legion(goal, &agents, &mut short),
        Err(TileError::MismatchedLanes {
            agents: 500,
            slots: 499
        }),
        "lane mismatch refuses"
    );
}

#[test]
#[ignore = "timing: release perf gate measures the 1 ms legion budget"]
fn timing_legion_500_steps_under_1ms() {
    let (w, h) = (64, 64);
    let mut nav = TiledNav::bake(&vec![false; w * h], w, h, 8).expect("open grid bakes");
    let goal = (63usize, 63usize);
    nav.bake_goal(goal).expect("goal bakes");
    let agents: Vec<(usize, usize)> = (0..500).map(|i| ((i * 37) % w, (i * 53 + 7) % h)).collect();
    let mut out = vec![(0usize, 0usize); agents.len()];
    // Warmup once outside the measured window.
    nav.steer_legion(goal, &agents, &mut out)
        .expect("legion steps");
    let mut worst = Duration::ZERO;
    for _ in 0..50 {
        let begin = Instant::now();
        nav.steer_legion(goal, &agents, &mut out)
            .expect("legion steps");
        worst = worst.max(begin.elapsed());
    }
    assert!(
        worst < Duration::from_millis(1),
        "500 agents under 1 ms, worst {worst:?}"
    );
    eprintln!("legion500 worst over 50 reps: {worst:?}");
}

#[test]
#[ignore = "timing: release perf gate measures the 50 ms rebake budget"]
fn timing_rebake_tile_p95_under_50ms() {
    let (w, h) = (256, 256);
    let mut nav = TiledNav::bake(&vec![false; w * h], w, h, 16).expect("grid bakes");
    let goal = (255usize, 255usize);
    nav.bake_goal(goal).expect("goal bakes");
    let mut samples = Vec::with_capacity(21);
    for round in 0..21 {
        let x = (round * 37) % w;
        let y = (round * 53 + 11) % h;
        nav.set_blocked(x, y, true).expect("wall rises");
        let begin = Instant::now();
        nav.rebake_budget(usize::MAX);
        nav.bake_goal(goal).expect("field refreshes");
        samples.push(begin.elapsed());
        nav.set_blocked(x, y, false).expect("breach flips");
        nav.rebake_budget(usize::MAX);
        nav.bake_goal(goal).expect("field refreshes");
    }
    samples.sort();
    let p95 = samples[(samples.len() * 95 / 100).min(samples.len() - 1)];
    assert!(
        p95 < Duration::from_millis(50),
        "dirty tile rebake p95 under 50 ms, p95 {p95:?}"
    );
    eprintln!(
        "rebake p50 {:?} p95 {p95:?} max {:?}",
        samples[samples.len() / 2],
        samples[samples.len() - 1]
    );
}

#[test]
fn obstacle_rebuild_touches_only_its_tiles() {
    use engine_nav::tiles::{ObstacleCache, TileId};
    let (w, h) = (24usize, 16usize);
    let (start, goal) = ((0usize, 8usize), (23usize, 8usize));
    let mut nav = TiledNav::bake(&walled(w, h, 12, Some(8)), w, h, 8).expect("door bakes");
    nav.bake_goal(goal).expect("goal bakes");
    assert!(nav.find_path(start, goal).expect("query").is_some());
    let mut cache = ObstacleCache::default();
    let door = [12, 8, 12, 8];
    let rubble = cache.add_box_obstacle(&nav, door).expect("queues");
    assert_eq!(nav.dirty_count(), 0, "add only queues");
    let want = [nav.tile_of(12, 8).expect("door cell")];
    let before: Vec<u32> = (0..nav.tile_count() as u32)
        .map(|t| nav.version(TileId(t)).expect("v"))
        .collect();
    assert_eq!(nav.update_budget(&mut cache, 1), want.len());
    assert_eq!(nav.update_budget(&mut cache, 1), 0, "queue drained");
    nav.bake_goal(goal).expect("field refreshes");
    let sealed = nav.find_path(start, goal).expect("query");
    assert!(sealed.is_none(), "rubble seals");
    for (i, v) in before.iter().enumerate() {
        let now = nav.version(TileId(i as u32)).expect("v");
        let next = v + u32::from(want.contains(&TileId(i as u32)));
        assert_eq!(now, next, "only touched tiles bump");
    }
    assert_eq!(cache.remove_obstacle(rubble), Ok(()));
    let mut ticks = 0u32;
    while nav.find_path(start, goal).expect("query").is_none() {
        nav.update_budget(&mut cache, 1);
        nav.bake_goal(goal).expect("refresh");
        ticks += 1;
        assert!(ticks < TICKS_PER_SECOND, "breach opens in one second");
    }
    let walk = nav.find_path(start, goal).expect("query");
    let walk = walk.expect("breach connects");
    assert!(walk.cells().contains(&(12, 8)));
    assert_eq!(nav.to_bytes(), nav.to_bytes(), "digest x2");
    let stale = cache.remove_obstacle(rubble);
    assert!(matches!(stale, Err(TileError::UnknownObstacle(..))));
}

#[test]
fn obstacle_requests_refuse_loudly() {
    use engine_nav::tiles::{ObstacleCache, MAX_OBSTACLE_REQUESTS};
    let (w, h) = (32usize, 32usize);
    let nav = TiledNav::bake(&vec![false; w * h], w, h, 4).expect("grid bakes");
    let mut cache = ObstacleCache::default();
    let e = cache.add_box_obstacle(&nav, [5, 5, 2, 2]);
    assert_eq!(e, Err(TileError::BadBounds));
    let e = cache.add_box_obstacle(&nav, [w + 1, 0, w + 9, 3]);
    assert_eq!(e, Err(TileError::BadBounds));
    let big = cache.add_box_obstacle(&nav, [0, 0, w - 1, h - 1]);
    assert!(matches!(big, Err(TileError::TooManyTiles { .. })));
    for _ in 0..MAX_OBSTACLE_REQUESTS {
        assert!(cache.add_box_obstacle(&nav, [0, 0, 0, 0]).is_ok());
    }
    let full = cache.add_box_obstacle(&nav, [1, 1, 1, 1]);
    assert_eq!(full, Err(TileError::QueueFull));
}

/// Sorted tile indexes a request touches, for readable asserts.
fn tile_numbers(tiles: &[TileId]) -> Vec<u32> {
    tiles.iter().map(|t| t.0).collect()
}

/// Settled 32x32 edge-8 grid with a rubble box over the middle crossing, the
/// state the NV13 breach starts from.
fn settled_rubble() -> (
    TiledNav,
    engine_nav::tiles::ObstacleCache,
    engine_nav::tiles::ObstacleRef,
) {
    use engine_nav::tiles::ObstacleCache;
    let mut nav = TiledNav::bake(&vec![false; 32 * 32], 32, 32, 8).expect("grid bakes");
    let mut cache = ObstacleCache::default();
    let rubble = cache
        .add_box_obstacle(&nav, [14, 14, 17, 17])
        .expect("rubble queues");
    assert!(!nav.up_to_date(&cache), "a queued request is not converged");
    nav.update_budget(&mut cache, usize::MAX);
    assert!(nav.up_to_date(&cache), "settled before the breach");
    (nav, cache, rubble)
}

/// The NV13 breach: two cylinder adds plus one box removal, each touching 4
/// tiles (cap 8), rebuilt `budget` tiles per tick. Returns the rebuilds per
/// tick, the stale walks after each tick, and the final digest.
fn run_breach(budget: usize) -> (Vec<usize>, Vec<Vec<usize>>, u64) {
    use engine_nav::tiles::MAX_TOUCHED_TILES;
    let (mut nav, mut cache, rubble) = settled_rubble();
    // One resting walk per tile the breach will dirty.
    let dirty = [0u32, 1, 4, 5, 6, 9, 10, 11, 14, 15];
    let walks: Vec<_> = dirty
        .iter()
        .map(|t| {
            let cell = ((*t as usize % 4) * 8 + 3, (*t as usize / 4) * 8 + 3);
            nav.find_path(cell, cell)
                .expect("cell is free")
                .expect("walk")
        })
        .collect();
    let west = cache
        .add_cylinder_obstacle(&nav, (8, 8), 2)
        .expect("queues");
    let east = cache
        .add_cylinder_obstacle(&nav, (24, 24), 2)
        .expect("queues");
    let touched = [west, east, rubble].map(|r| cache.touched(&nav, r).expect("live"));
    assert_eq!(
        touched.each_ref().map(|t| t.len()),
        [4, 4, 4],
        "touched 4/4/4"
    );
    assert!(
        touched.iter().all(|t| t.len() <= MAX_TOUCHED_TILES),
        "cap 8"
    );
    assert_eq!(tile_numbers(&touched[0]), [0, 1, 4, 5]);
    assert_eq!(tile_numbers(&touched[1]), [10, 11, 14, 15]);
    assert_eq!(tile_numbers(&touched[2]), [5, 6, 9, 10]);
    cache.remove_obstacle(rubble).expect("removal queues");
    assert_eq!(cache.pending_count(), 3);
    assert_eq!(nav.dirty_count(), 0, "requests only queue");

    let (mut rebuilt, mut stale) = (Vec::new(), Vec::new());
    while !nav.up_to_date(&cache) {
        rebuilt.push(nav.update_budget(&mut cache, budget));
        stale.push(nav.stale_paths(&walks));
    }
    assert_eq!(cache.pending_count(), 0);
    assert_eq!(
        nav.update_budget(&mut cache, budget),
        0,
        "converged is idle"
    );
    (rebuilt, stale, nav.digest())
}

#[test]
fn four_tile_breach_rebuilds_one_tile_per_tick_and_converges() {
    let (rebuilt, stale, digest) = run_breach(1);
    assert_eq!(rebuilt, [1; 10], "1 rebuild per tick over 10 ticks");
    for (tick, now) in stale.iter().enumerate() {
        assert_eq!(
            *now,
            (0..=tick).collect::<Vec<_>>(),
            "stale exact at tick {tick}"
        );
    }
    assert_eq!(stale.last().map(Vec::len), Some(10), "stale exact 10/10");
    assert_eq!(digest, run_breach(1).2, "digest x2");
    assert_eq!(
        digest, 8_032_266_843_827_844_966,
        "pinned digest of the converged breach"
    );
}

/// Ablation: draining the whole burst in one tick breaks the per-tick bound
/// the budget keeps, yet lands on the same converged grid.
#[test]
fn burst_rebuild_breaks_the_per_tick_budget() {
    let (rebuilt, _, digest) = run_breach(usize::MAX);
    assert_eq!(rebuilt, [10], "burst rebuilds all 10 in one tick");
    assert!(
        rebuilt.iter().any(|n| *n > 1),
        "the 1-per-tick gate catches it"
    );
    assert_eq!(digest, run_breach(1).2, "same grid once converged");
}

#[test]
fn cylinder_obstacles_refuse_loudly_and_block_only_the_disc() {
    use engine_nav::tiles::ObstacleCache;
    let mut nav = TiledNav::bake(&vec![false; 32 * 32], 32, 32, 8).expect("grid bakes");
    let mut cache = ObstacleCache::default();
    assert_eq!(
        cache.add_cylinder_obstacle(&nav, (32, 0), 1),
        Err(TileError::BadBounds)
    );
    assert!(matches!(
        cache.add_cylinder_obstacle(&nav, (16, 16), 20),
        Err(TileError::TooManyTiles { .. })
    ));
    let disc = cache
        .add_cylinder_obstacle(&nav, (8, 8), 2)
        .expect("queues");
    nav.update_budget(&mut cache, usize::MAX);
    assert!(nav.find_path((8, 8), (8, 8)).is_err(), "centre is blocked");
    assert!(
        nav.find_path((10, 8), (10, 8)).is_err(),
        "edge cell is blocked"
    );
    assert!(
        nav.find_path((10, 10), (10, 10)).is_ok(),
        "box corner stays open"
    );
    cache.remove_obstacle(disc).expect("removal queues");
    nav.update_budget(&mut cache, usize::MAX);
    assert!(nav.up_to_date(&cache));
    assert!(
        nav.find_path((8, 8), (8, 8)).is_ok(),
        "removal clears the disc"
    );
}

/// Arena-shaped 48x48 floor: a round wall with an east gate, four pillars,
/// and one 2x2 plug at the corner where four 8-cell tiles meet.
fn arena_floor() -> Vec<bool> {
    let (w, h) = (48usize, 48usize);
    let mut blocked = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as i32 - 24, y as i32 - 24);
            let d2 = dx * dx + dy * dy;
            let gate = dx > 0 && dy.abs() <= 1;
            let pillar = [(-8, -8), (8, -8), (-8, 8), (8, 8)]
                .iter()
                .any(|(px, py)| (dx - px).abs() <= 1 && (dy - py).abs() <= 1);
            blocked[y * w + x] = d2 > 22 * 22 || (d2 >= 20 * 20 && !gate) || pillar;
        }
    }
    for (x, y) in [(15, 15), (16, 15), (15, 16), (16, 16)] {
        blocked[y * w + x] = true;
    }
    blocked
}

fn cell_bytes(cells: &[(usize, usize)]) -> Vec<u8> {
    cells
        .iter()
        .flat_map(|(x, y)| [*x as u32, *y as u32])
        .flat_map(u32::to_le_bytes)
        .collect()
}

/// Diffs the whole-arena bake against the tiled bake on seeded cross-tile
/// queries: grid walks, smooth paths and flow steps must match byte for
/// byte. Returns the tiled side's bytes so a rerun can be compared.
fn stitch_diff(whole: &TiledNav, tiled: &TiledNav) -> Vec<u8> {
    let free: Vec<(usize, usize)> = (0..48)
        .flat_map(|y| (0..48).map(move |x| (x, y)))
        .filter(|cell| whole.find_path(*cell, *cell).is_ok())
        .collect();
    let mut state = 0x2040_u64;
    let mut pick = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        free[(state >> 33) as usize % free.len()]
    };
    let (mut report, mut crossing) = (Vec::new(), 0);
    for _ in 0..150 {
        let (start, goal) = (pick(), pick());
        let want = whole.find_path(start, goal).expect("whole query");
        let got = tiled.find_path(start, goal).expect("tiled query");
        assert_eq!(
            want.as_ref().map(|p| cell_bytes(p.cells())),
            got.as_ref().map(|p| cell_bytes(p.cells())),
            "walk {start:?}->{goal:?}"
        );
        let smooth_want = engine_nav::funnel::smooth_path(whole, start, goal).expect("whole");
        let smooth_got = engine_nav::funnel::smooth_path(tiled, start, goal).expect("tiled");
        assert_eq!(
            smooth_want.as_ref().map(|p| p.points().to_vec()),
            smooth_got.as_ref().map(|p| p.points().to_vec()),
            "smooth {start:?}->{goal:?}"
        );
        crossing += usize::from(tiled.tile_of(start.0, start.1) != tiled.tile_of(goal.0, goal.1));
        if let Some(p) = got {
            report.extend(cell_bytes(p.cells()));
        }
    }
    assert!(
        crossing > 100,
        "only {crossing} of 150 queries cross a tile"
    );
    let goal = (24, 24);
    for (x, y) in &free {
        assert_eq!(
            whole.direction(goal, (*x, *y)),
            tiled.direction(goal, (*x, *y)),
            "flow step at ({x}, {y})"
        );
    }
    report
}

#[test]
fn stitched_tiles_match_the_whole_arena_before_and_after_a_breach() {
    let run = || {
        let floor = arena_floor();
        let mut whole = TiledNav::bake(&floor, 48, 48, 64).expect("whole bakes");
        let mut tiled = TiledNav::bake(&floor, 48, 48, 8).expect("tiled bakes");
        assert_eq!((whole.tile_count(), tiled.tile_count()), (1, 36));
        whole.bake_goal((24, 24)).expect("goal");
        tiled.bake_goal((24, 24)).expect("goal");
        let mut report = stitch_diff(&whole, &tiled);

        // A walk across the plug, stamped before the breach.
        let across = ((8, 24), (40, 24));
        let held = tiled
            .find_path(across.0, across.1)
            .expect("query")
            .expect("open");
        for (x, y) in [(15, 15), (16, 15), (15, 16), (16, 16)] {
            whole.set_blocked(x, y, false).expect("breach flips");
            tiled.set_blocked(x, y, false).expect("breach flips");
        }
        assert_eq!(
            whole.rebake_budget(usize::MAX),
            1,
            "one tile on the whole bake"
        );
        assert_eq!(
            tiled.rebake_budget(usize::MAX),
            4,
            "the plug touches 4 tiles"
        );
        whole.bake_goal((24, 24)).expect("refresh");
        tiled.bake_goal((24, 24)).expect("refresh");
        report.extend(stitch_diff(&whole, &tiled));
        assert_eq!(tiled.path_is_stale(&held), Ok(false), "a far walk survives");
        report
    };
    let first = run();
    assert!(!first.is_empty());
    assert_eq!(first, run(), "stitch diff bytes x2");
}

/// 16 m square floor in 1 m slabs: four pillars on the 4 m tile crossings,
/// sparse 1 m holes straddling the x = 8 m and z = 8 m borders, plus a
/// scatter so polygons are not one rectangle. At 0.25 m cells this is a
/// 65 x 65 grid, so 16-cell tiles form a 5 x 5 layout.
fn mesh_arena() -> Vec<[[f32; 3]; 3]> {
    let mut tris = Vec::new();
    for z in 0..16usize {
        for x in 0..16usize {
            let pillar = [(4usize, 4usize), (12, 4), (4, 12), (12, 12)].contains(&(x, z));
            let seam = (x == 7 || x == 8 || z == 7 || z == 8) && (x + z) % 5 == 0;
            let scatter = x % 5 == 2 && z % 7 == 3;
            if pillar || seam || scatter {
                continue;
            }
            let (x0, x1) = (x as f32, x as f32 + 1.0);
            let (z0, z1) = (z as f32, z as f32 + 1.0);
            tris.push([[x0, 0.0, z0], [x0, 0.0, z1], [x1, 0.0, z1]]);
            tris.push([[x0, 0.0, z0], [x1, 0.0, z1], [x1, 0.0, z0]]);
        }
    }
    tris
}

fn mesh_config() -> BakeConfig {
    BakeConfig {
        cell_size: 0.25,
        walkable_normal_y: 0.7,
    }
}

fn path_bytes(path: &[[f32; 3]]) -> Vec<u8> {
    path.iter()
        .flat_map(|point| point.iter().flat_map(|value| value.to_le_bytes()))
        .collect()
}

fn walkable_cells(mesh: &NavMesh) -> Vec<(u32, u32)> {
    let (w, h) = mesh.grid_dims();
    let mut cells = Vec::new();
    for z in 0..h {
        for x in 0..w {
            if mesh.cell_height_mm([x, z]).is_some() {
                cells.push((x, z));
            }
        }
    }
    cells
}

fn seeded_pair(cells: &[(u32, u32)], state: &mut u64) -> [(u32, u32); 2] {
    let pick = |state: &mut u64| {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        cells[((*state >> 33) as usize) % cells.len()]
    };
    [pick(state), pick(state)]
}

#[test]
fn tiled_mesh_assembly_equals_the_whole_bake_and_a_four_tile_rebake() {
    let run = || {
        let tris = mesh_arena();
        let config = mesh_config();
        let whole = NavMesh::bake(&tris, &config).expect("whole bakes");
        let mut tiled = TiledBake::bake(&tris, &config, 16).expect("tiled bakes");
        assert_eq!(
            tiled.nav().to_bytes(),
            whole.to_bytes(),
            "tiled assembly is the whole bake byte for byte"
        );
        assert_eq!(tiled.tile_layout(), (16, 2, 5, 5));
        assert_eq!(tiled.tile_count(), 25);

        let before: Vec<Vec<u8>> = (0..tiled.tile_count())
            .map(|tile| tiled.tile_bytes(tile).expect("tile bytes"))
            .collect();

        // The 2 x 2 plug at cells 31/32 sits on four mesh tile corners.
        let dirty = [6u32, 7, 11, 12];
        assert_eq!(tiled.rebake(&dirty), Ok(4), "four tiles rebake");
        assert_eq!(
            tiled.nav().to_bytes(),
            whole.to_bytes(),
            "a subset rebake rebuilds the whole bake"
        );
        for tile in 0..tiled.tile_count() {
            assert_eq!(
                tiled.tile_bytes(tile).expect("tile bytes"),
                before[tile as usize],
                "tile {tile} bytes unchanged"
            );
        }
        let past = tiled.tile_count();
        assert_eq!(
            tiled.rebake(&[past]),
            Err(NavError::NotCanonical("tile id")),
            "a bad tile id refuses"
        );

        let cells = walkable_cells(&whole);
        let mut state = 0x2040_2026u64;
        let (mut crossing, mut report) = (0usize, Vec::new());
        for _ in 0..40 {
            let [start, goal] = seeded_pair(&cells, &mut state);
            let from = whole.cell_center_m([start.0, start.1]);
            let to = whole.cell_center_m([goal.0, goal.1]);
            let want = whole
                .query([from[0], 0.0, from[1]], [to[0], 0.0, to[1]])
                .expect("whole query");
            let got = tiled
                .nav()
                .query([from[0], 0.0, from[1]], [to[0], 0.0, to[1]])
                .expect("tiled query");
            assert_eq!(
                path_bytes(&got),
                path_bytes(&want),
                "cross-tile path {start:?} -> {goal:?}"
            );
            crossing += usize::from(start.0 / 16 != goal.0 / 16 || start.1 / 16 != goal.1 / 16);
            report.extend(path_bytes(&got));
        }
        assert!(
            crossing > 20,
            "only {crossing} of 40 queries cross a mesh tile"
        );

        // No seam gap: every cell on a tile border reads the same height.
        let (w, h) = whole.grid_dims();
        let mut seam = 0usize;
        for z in 0..h {
            for x in 0..w {
                if x % 16 == 0 || x % 16 == 15 || z % 16 == 0 || z % 16 == 15 {
                    seam += 1;
                    assert_eq!(
                        whole.cell_height_mm([x, z]),
                        tiled.nav().cell_height_mm([x, z]),
                        "seam cell ({x}, {z})"
                    );
                }
            }
        }
        assert!(seam > 100, "only {seam} seam cells counted");
        report.extend(before.concat());
        (whole.digest(), report)
    };
    let first = run();
    assert!(!first.1.is_empty());
    assert_eq!(first, run(), "tiled mesh digest and report x2");
}

#[test]
fn the_rebake_hook_hands_the_mesh_lane_exactly_the_dirty_tiles() {
    let (w, h) = (65usize, 65usize);
    let mut nav = TiledNav::bake(&vec![false; w * h], w, h, 16).expect("grid bakes");
    for (x, z) in [(31usize, 31usize), (32, 31), (31, 32), (32, 32)] {
        nav.set_blocked(x, z, true).expect("plug rises");
    }
    assert_eq!(nav.dirty_count(), 4, "the plug touches four tiles");
    let tris = mesh_arena();
    let config = mesh_config();
    let whole = NavMesh::bake(&tris, &config).expect("whole bakes");
    let mut tiled = TiledBake::bake(&tris, &config, 16).expect("tiled bakes");
    let mut seen = Vec::new();
    let done = nav.rebake_budget_with(usize::MAX, &mut |tile| {
        seen.push(tile.0);
        tiled.rebake(&[tile.0]).expect("mesh tile rebakes");
    });
    assert_eq!(done, 4);
    assert_eq!(seen, [6, 7, 11, 12], "ascending, exactly once each");
    assert_eq!(
        tiled.nav().to_bytes(),
        whole.to_bytes(),
        "hooked rebake keeps the whole-bake bytes"
    );
    assert!(nav.up_to_date(&ObstacleCache::default()), "queue drained");
    assert_eq!(nav.rebake_budget(usize::MAX), 0, "no dirty tile left");
}

#[test]
#[ignore = "timing: release perf gate posts four-tile rebake ms against the full bake"]
fn timing_four_tile_rebake_versus_full_bake() {
    // 64 m floor in 1 m slabs: 8192 triangles on a 257 x 257 cell grid, so
    // 17 x 17 mesh tiles of 16 cells. The four centre tiles rebake together.
    let mut tris = Vec::new();
    for z in 0..64usize {
        for x in 0..64usize {
            let (x0, x1) = (x as f32, x as f32 + 1.0);
            let (z0, z1) = (z as f32, z as f32 + 1.0);
            tris.push([[x0, 0.0, z0], [x0, 0.0, z1], [x1, 0.0, z1]]);
            tris.push([[x0, 0.0, z0], [x1, 0.0, z1], [x1, 0.0, z0]]);
        }
    }
    let config = mesh_config();
    let whole = NavMesh::bake(&tris, &config).expect("whole bakes");
    let mut tiled = TiledBake::bake(&tris, &config, 16).expect("tiled bakes");
    let (_, _, cols, _) = tiled.tile_layout();
    let centre = cols / 2 + (cols / 2) * cols;
    let dirty = [centre, centre + 1, centre + cols, centre + cols + 1];
    tiled.rebake(&dirty).expect("warmup rebake");
    assert_eq!(tiled.nav().to_bytes(), whole.to_bytes(), "warmup matches");
    let mut full = Vec::with_capacity(21);
    let mut dirty_ms = Vec::with_capacity(21);
    for _ in 0..21 {
        let begin = Instant::now();
        let baked = NavMesh::bake(&tris, &config).expect("full bakes");
        full.push(begin.elapsed());
        assert_eq!(baked.to_bytes(), whole.to_bytes(), "full bake x2");
        let begin = Instant::now();
        tiled.rebake(&dirty).expect("four tiles rebake");
        dirty_ms.push(begin.elapsed());
        assert_eq!(tiled.nav().to_bytes(), whole.to_bytes(), "rebake matches");
    }
    let stats = |mut samples: Vec<Duration>| {
        samples.sort();
        (
            samples[samples.len() / 2],
            samples[(samples.len() * 95 / 100).min(samples.len() - 1)],
        )
    };
    let (full_p50, full_p95) = stats(full);
    let (dirty_p50, dirty_p95) = stats(dirty_ms);
    assert!(
        dirty_p95 < Duration::from_millis(50),
        "four-tile rebake p95 {dirty_p95:?}"
    );
    eprintln!(
        "full bake p50 {full_p50:?} p95 {full_p95:?}; dirty-4 rebake p50 {dirty_p50:?} p95 {dirty_p95:?}"
    );
}
