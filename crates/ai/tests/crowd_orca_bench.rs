// Attribution: ORCA half-plane formulation inspired by Jur van den Berg's
// RVO2 library by Snape et al. (Apache-2.0); neighbour search follows the
// classic deterministic kd-tree range-query shape. No donor code is copied:
// every line below is a fresh integer fixed-point implementation written for
// engine2040.
//!
//! Crowd avoidance bench: 512-agent ORCA step, naive O(n^2) neighbour scan
//! (before) versus kd-tree neighbour query (after) with an identical
//! half-plane solve, so both paths land bit-identical velocities.
//!
//! Units: millimetres for length, millimetres per second for velocity,
//! milliseconds for time. All state is `i32`/`i64` integers: the step is
//! bit-deterministic for one complete input world. Wall-clock timing wraps
//! the steps only for the bench report and never enters simulation state.
//! Reported numbers are per-step ms over `STEPS` timed steps, best of three
//! fresh runs each way.

#[expect(
    clippy::disallowed_types,
    reason = "bench wall-clock report only, never sim state"
)]
use std::time::Instant;

/// Fixed-point scale is implicit: positions are mm, velocities are mm/s.
const AGENTS: usize = 512;
/// Timed steps per run; amortises scheduling noise over the query gap.
const STEPS: usize = 20;
/// Neighbour radius in mm. Kept tight so the kd-tree prune beats the scan.
const NEIGHBOUR_DIST_MM: i64 = 2_000;
/// Time horizon for collision anticipation in ms.
const HORIZON_MS: i64 = 2_000;
/// Step length in ms.
const DT_MS: i64 = 100;
/// Agent radius in mm.
const RADIUS_MM: i32 = 300;
/// Top speed in mm/s.
const MAX_SPEED_MM_S: i64 = 1_500;

/// One crowd agent. Plain integers: `Copy` so worlds clone exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Agent {
    px_mm: i32,
    py_mm: i32,
    vx_mm_s: i32,
    vy_mm_s: i32,
    pref_vx_mm_s: i32,
    pref_vy_mm_s: i32,
}

/// Integer square root (Newton iterations, deterministic on every target).
fn isqrt(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Scale `(vx, vy)` down to `MAX_SPEED_MM_S`. The divisor `len + 1` strictly
/// shrinks any non-zero component, so the loop terminates; truncation only
/// ever shortens, so the result always fits the cap. Inputs must fit `i32`.
fn clamp_speed(vx: i64, vy: i64) -> (i32, i32) {
    let max_sq = (MAX_SPEED_MM_S * MAX_SPEED_MM_S) as u64;
    let (mut x, mut y) = (vx, vy);
    loop {
        let len_sq = (x * x + y * y) as u64;
        if len_sq <= max_sq {
            break;
        }
        let div = isqrt(len_sq) as i64 + 1;
        x = x * MAX_SPEED_MM_S / div;
        y = y * MAX_SPEED_MM_S / div;
    }
    (x as i32, y as i32)
}

/// Build the deterministic 512-agent crossing world: a sparse grid whose
/// preferred velocities point at the mirrored side, so crossing agents must
/// avoid each other at the centre.
fn build_world() -> Vec<Agent> {
    let mut agents = Vec::with_capacity(AGENTS);
    for i in 0..AGENTS {
        let gx = (i % 32) as i64;
        let gy = (i / 32) as i64;
        let px = (gx * 1_200 - 18_600) as i32;
        let py = (gy * 1_200 - 9_000) as i32;
        // Preferred velocity: towards the mirrored point, at top speed.
        let dx = -2 * i64::from(px);
        let dy = -2 * i64::from(py);
        let dist_sq = (dx * dx + dy * dy) as u64;
        let (pref_vx, pref_vy) = if dist_sq == 0 {
            (0, 0)
        } else {
            let dist = isqrt(dist_sq) as i64;
            // `isqrt` floors, so the raw vector can sit a hair above the cap.
            clamp_speed(dx * MAX_SPEED_MM_S / dist, dy * MAX_SPEED_MM_S / dist)
        };
        agents.push(Agent {
            px_mm: px,
            py_mm: py,
            vx_mm_s: 0,
            vy_mm_s: 0,
            pref_vx_mm_s: pref_vx,
            pref_vy_mm_s: pref_vy,
        });
    }
    agents
}

/// One ORCA half-plane: `dot(v, n) <= bound_mm_s`, normal `n` unit-scaled.
#[derive(Clone, Copy)]
struct Constraint {
    nx: i64,
    ny: i64,
    norm: i64,
    bound_mm_s: i64,
}

/// Solve the new velocity for `agent` given neighbour indices into `agents`.
/// Shared by both neighbour-query paths so timings compare query cost only.
fn solve_agent(agent: &Agent, neighbours: &[usize], agents: &[Agent]) -> (i32, i32) {
    let mut constraints = [Constraint {
        nx: 0,
        ny: 0,
        norm: 1,
        bound_mm_s: 0,
    }; 64];
    let mut count = 0usize;

    for &j in neighbours {
        let other = &agents[j];
        let rx = i64::from(other.px_mm - agent.px_mm);
        let ry = i64::from(other.py_mm - agent.py_mm);
        let dist_sq = (rx * rx + ry * ry) as u64;
        let combined = i64::from(RADIUS_MM) * 2;
        // Time to closest approach along relative velocity.
        let rel_vx = i64::from(agent.vx_mm_s - other.vx_mm_s);
        let rel_vy = i64::from(agent.vy_mm_s - other.vy_mm_s);
        let dist = isqrt(dist_sq) as i64;
        // Deterministic fallback normal when agents coincide: derived from
        // the neighbour index so the order of constraints stays stable.
        let (nx, ny, nlen) = if dist == 0 {
            let k = (j % 8) as i64 + 1;
            (k * 1_000, (8 - k) * 1_000, 8_000)
        } else {
            (rx * 1_000 / dist, ry * 1_000 / dist, 1_000)
        };
        // Approaching speed along the normal (mm/s, scaled by 1/1000*nlen).
        let approaching = -(rel_vx * nx + rel_vy * ny) / nlen.max(1);
        if approaching <= 0 {
            continue;
        }
        // Milliseconds until the combined radius is breached.
        let gap = dist - combined;
        let ttc_ms = if gap <= 0 {
            0
        } else {
            gap * 1_000 / approaching.max(1)
        };
        if ttc_ms > HORIZON_MS {
            continue;
        }
        // Half responsibility: each agent corrects half the needed rate.
        let need_mm_s = if gap <= 0 {
            combined * 1_000 / HORIZON_MS
        } else {
            (combined - gap.min(combined)) * 1_000 / HORIZON_MS
        };
        let mid_vx = (i64::from(agent.vx_mm_s) + i64::from(other.vx_mm_s)) / 2;
        let mid_vy = (i64::from(agent.vy_mm_s) + i64::from(other.vy_mm_s)) / 2;
        let mid_along = (mid_vx * nx + mid_vy * ny) / nlen.max(1);
        if count < constraints.len() {
            constraints[count] = Constraint {
                nx,
                ny,
                norm: nlen.max(1),
                bound_mm_s: mid_along - need_mm_s / 2,
            };
            count += 1;
        }
    }

    // Project the preferred velocity onto every half-plane, two passes
    // (the standard RVO2 linear-program sweep shape, integer arithmetic).
    let mut nvx = i64::from(agent.pref_vx_mm_s);
    let mut nvy = i64::from(agent.pref_vy_mm_s);
    for _ in 0..2 {
        for c in constraints.iter().take(count) {
            let along = (nvx * c.nx + nvy * c.ny) / c.norm;
            if along > c.bound_mm_s {
                let excess = along - c.bound_mm_s;
                nvx -= c.nx * excess / c.norm;
                nvy -= c.ny * excess / c.norm;
            }
        }
    }

    // Clamp to top speed (also covers a pref vector already at the cap).
    clamp_speed(nvx, nvy)
}

/// Naive O(n^2) neighbour scan. Returns the integrated world.
fn step_naive(agents: &mut [Agent]) {
    let r_sq = (NEIGHBOUR_DIST_MM * NEIGHBOUR_DIST_MM) as u64;
    let mut new_v = vec![(0i32, 0i32); agents.len()];
    for i in 0..agents.len() {
        let mut neighbours = Vec::new();
        for j in 0..agents.len() {
            if i == j {
                continue;
            }
            let dx = i64::from(agents[j].px_mm - agents[i].px_mm);
            let dy = i64::from(agents[j].py_mm - agents[i].py_mm);
            let d_sq = (dx * dx + dy * dy) as u64;
            if d_sq <= r_sq {
                neighbours.push(j);
            }
        }
        // Index order is already ascending: deterministic constraint order.
        new_v[i] = solve_agent(&agents[i], &neighbours, agents);
    }
    for (a, v) in agents.iter_mut().zip(new_v) {
        a.vx_mm_s = v.0;
        a.vy_mm_s = v.1;
        a.px_mm += (i64::from(v.0) * DT_MS / 1_000) as i32;
        a.py_mm += (i64::from(v.1) * DT_MS / 1_000) as i32;
    }
}

/// Deterministic kd-tree over agent positions for range queries.
struct KdNode {
    point: usize,
    axis: u8,
    left: Option<Box<KdNode>>,
    right: Option<Box<KdNode>>,
}

fn kd_build(sorted: &mut [usize], depth: usize, agents: &[Agent]) -> Option<Box<KdNode>> {
    if sorted.is_empty() {
        return None;
    }
    let axis = (depth % 2) as u8;
    sorted.sort_unstable_by_key(|&i| {
        (
            if axis == 0 {
                agents[i].px_mm
            } else {
                agents[i].py_mm
            },
            i,
        )
    });
    let mid = sorted.len() / 2;
    let (left, rest) = sorted.split_at_mut(mid);
    let point = rest[0];
    let right = if rest.len() > 1 {
        &mut rest[1..]
    } else {
        &mut []
    };
    Some(Box::new(KdNode {
        point,
        axis,
        left: kd_build(left, depth + 1, agents),
        right: kd_build(right, depth + 1, agents),
    }))
}

fn kd_query(
    node: &Option<Box<KdNode>>,
    qx: i32,
    qy: i32,
    r_sq: u64,
    agents: &[Agent],
    out: &mut Vec<usize>,
) {
    let Some(n) = node else { return };
    let a = &agents[n.point];
    let dx = i64::from(a.px_mm - qx);
    let dy = i64::from(a.py_mm - qy);
    if (dx * dx + dy * dy) as u64 <= r_sq {
        out.push(n.point);
    }
    let diff = if n.axis == 0 {
        i64::from(qx - a.px_mm)
    } else {
        i64::from(qy - a.py_mm)
    };
    if diff * diff <= r_sq as i64 {
        kd_query(&n.left, qx, qy, r_sq, agents, out);
        kd_query(&n.right, qx, qy, r_sq, agents, out);
    } else if diff < 0 {
        kd_query(&n.left, qx, qy, r_sq, agents, out);
    } else {
        kd_query(&n.right, qx, qy, r_sq, agents, out);
    }
}

/// kd-tree neighbour query path. Returns the integrated world.
fn step_kdtree(agents: &mut [Agent]) {
    let r_sq = (NEIGHBOUR_DIST_MM * NEIGHBOUR_DIST_MM) as u64;
    let mut order: Vec<usize> = (0..agents.len()).collect();
    let tree = kd_build(&mut order, 0, agents);
    let mut new_v = vec![(0i32, 0i32); agents.len()];
    let mut scratch = Vec::new();
    for i in 0..agents.len() {
        scratch.clear();
        kd_query(
            &tree,
            agents[i].px_mm,
            agents[i].py_mm,
            r_sq,
            agents,
            &mut scratch,
        );
        scratch.retain(|&j| j != i);
        // Sort so constraint order matches the naive path exactly.
        scratch.sort_unstable();
        new_v[i] = solve_agent(&agents[i], &scratch, agents);
    }
    for (a, v) in agents.iter_mut().zip(new_v) {
        a.vx_mm_s = v.0;
        a.vy_mm_s = v.1;
        a.px_mm += (i64::from(v.0) * DT_MS / 1_000) as i32;
        a.py_mm += (i64::from(v.1) * DT_MS / 1_000) as i32;
    }
}

fn speed_sq_mm_s(vx: i32, vy: i32) -> u64 {
    (i64::from(vx) * i64::from(vx) + i64::from(vy) * i64::from(vy)) as u64
}

#[test]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "bench wall-clock report only, never sim state"
)]
fn orca_512_step_ms() {
    assert_eq!(build_world().len(), AGENTS, "world must hold 512 agents");

    // Warm up both paths off the clock, then keep the best per-step ms over
    // STEPS timed steps across three fresh runs each way; the minimum is
    // robust against scheduling noise.
    step_naive(&mut build_world());
    step_kdtree(&mut build_world());
    let mut before_ms = f64::INFINITY;
    let mut before_world = build_world();
    for _ in 0..3 {
        let mut w = build_world();
        let t0 = Instant::now();
        for _ in 0..STEPS {
            step_naive(&mut w);
        }
        let per_step = t0.elapsed().as_secs_f64() * 1_000.0 / STEPS as f64;
        before_ms = before_ms.min(per_step);
        before_world = w;
    }
    let mut after_ms = f64::INFINITY;
    let mut after_world = build_world();
    for _ in 0..3 {
        let mut w = build_world();
        let t1 = Instant::now();
        for _ in 0..STEPS {
            step_kdtree(&mut w);
        }
        let per_step = t1.elapsed().as_secs_f64() * 1_000.0 / STEPS as f64;
        after_ms = after_ms.min(per_step);
        after_world = w;
    }

    // Wall-clock report only; state below is integer-deterministic.
    println!("crowd_orca_512 before_ms={before_ms:.3} after_ms={after_ms:.3}");

    // Same query radius and sorted constraint order: bit-identical worlds.
    assert_eq!(
        before_world, after_world,
        "kd-tree path must match the naive path exactly"
    );

    // Determinism: a fresh run lands on the same integers.
    let mut rerun = build_world();
    for _ in 0..STEPS {
        step_kdtree(&mut rerun);
    }
    assert_eq!(after_world, rerun, "steps must be deterministic");

    // Every velocity respects the speed cap and positions stay in the arena.
    let max_sq = (MAX_SPEED_MM_S * MAX_SPEED_MM_S) as u64;
    for a in &after_world {
        assert!(
            speed_sq_mm_s(a.vx_mm_s, a.vy_mm_s) <= max_sq,
            "velocity exceeds top speed: {a:?}"
        );
        assert!(
            a.px_mm.abs() < 30_000 && a.py_mm.abs() < 30_000,
            "agent left the arena: {a:?}"
        );
    }
}

#[test]
fn orca_separates_overlapping_pair() {
    let pair = vec![
        Agent {
            px_mm: 0,
            py_mm: 0,
            vx_mm_s: 0,
            vy_mm_s: 0,
            pref_vx_mm_s: 1_500,
            pref_vy_mm_s: 0,
        },
        Agent {
            px_mm: 0,
            py_mm: 0,
            vx_mm_s: 0,
            vy_mm_s: 0,
            pref_vx_mm_s: -1_500,
            pref_vy_mm_s: 0,
        },
    ];
    let mut world = pair.clone();
    step_kdtree(&mut world);
    // Coinciding agents must not share a position after the step.
    assert!(
        world[0].px_mm != world[1].px_mm || world[0].py_mm != world[1].py_mm,
        "overlapping agents did not separate: {world:?}"
    );
    // Failure case the solve must never produce: speeds above the cap.
    let max_sq = (MAX_SPEED_MM_S * MAX_SPEED_MM_S) as u64;
    for a in &world {
        assert!(
            speed_sq_mm_s(a.vx_mm_s, a.vy_mm_s) <= max_sq,
            "clamp failed: {a:?}"
        );
    }
}

#[test]
fn isqrt_handles_u64_max_without_overflow() {
    // Pins the Newton seed against overflow: `(u64::MAX + 1) / 2` wraps,
    // so the seed must use `div_ceil`. Small values pin the exact root.
    assert_eq!(isqrt(0), 0);
    assert_eq!(isqrt(1), 1);
    assert_eq!(isqrt(4), 2);
    assert_eq!(isqrt(15), 3);
    assert_eq!(isqrt(16), 4);
    assert_eq!(isqrt(u64::MAX), 4_294_967_295);
}
