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

use engine_nav::tiles::{BreakSet, TileError, TileId, TiledNav, TICKS_PER_SECOND};

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
