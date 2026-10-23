//! soak: the crowd soak harness for the fixed tick. Three readings over the
//! same integer fixtures: the 500-agent step cost, the seeded corridor drill
//! (does every agent reach the far end?), and the byte-for-byte replay.
//!
//! Fixtures are seeded and integer-only, so a rerun builds the same crowds and
//! the same state bytes on every machine. The cost bar is wall clock and stays
//! out of the default suite: it is `#[ignore]` and runs release-only on a quiet
//! PC (no other cargo build, one test thread). It times [`CROWD_SAMPLES`]
//! single `Orca::step` calls after discarding [`CROWD_WARMUP`] warm steps, then
//! judges the median and the p95, never one sample. The other two tests are
//! deterministic and run in the default suite.
//!
//! Units: millimetres and millimetres per second; one step is
//! [`engine_nav::orca::Params::dt_ms`] long.

#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "the cost bar reads the wall clock; the step it times never does"
)]

use std::time::Instant;

use engine_core::fixed::isqrt;
use engine_core::hash::{hex16, Fnv64};
use engine_core::hash_hex;
use engine_nav::orca::{Agent, Orca, Params, Wall};

/// Step tuning the arena crowd uses.
const PARAMS: Params = Params::ARENA;
/// Agents in the cost bar.
const CROWD_AGENTS: usize = 500;
/// Half-width of the cost crowd's arena in mm. The area grows with the crowd,
/// so 500 agents stand at the density of the 50-agent world of the same shape.
const CROWD_HALF_MM: i32 = 31_623;
/// Single steps timed by the cost bar.
const CROWD_SAMPLES: usize = 513;
/// Steps thrown away first, so the grid lanes are sized and the caches warm.
const CROWD_WARMUP: usize = 32;
/// The bar: 1 ms per 500-agent step, in nanoseconds.
const STEP_BUDGET_NS: u128 = 1_000_000;
/// Seeds `0..CORRIDOR_SEEDS` are the corridor corpus; same seed, same crowd.
const CORRIDOR_SEEDS: u64 = 1_000;
/// Ticks one corridor run lasts: 60 s at a 50 ms step.
const CORRIDOR_TICKS: usize = 1_200;
/// A stuck agent makes no progress for this many ticks (10 s at a 50 ms step).
const STUCK_WINDOW_TICKS: usize = 200;
/// Progress that clears the stuck window: this many mm closer to the goal.
const PROGRESS_MM: i64 = 100;
/// Within this distance of the goal an agent has arrived: one body radius plus
/// one step, plus slack for the truncating millimetre move.
const ARRIVE_MM: i64 = 400;
/// Corridor half-width between the walls in mm: a 4 m hall.
const CORRIDOR_HALF_MM: i32 = 2_000;
/// Body radius of every soak agent in mm.
const RADIUS_MM: i32 = 300;

/// Distance between two points in mm, from the integer square root of the
/// squared gap.
fn distance_mm(a: [i32; 2], b: [i32; 2]) -> i64 {
    let dx = i64::from(a[0]) - i64::from(b[0]);
    let dy = i64::from(a[1]) - i64::from(b[1]);
    let squared = u64::try_from(dx * dx + dy * dy).expect("bounded mm keep the square in u64");
    i64::try_from(isqrt(u128::from(squared))).expect("a distance under 2^31 mm fits i64")
}

/// Full-speed preferred velocity toward `goal`, slowing to land on it.
fn preferred_toward(pos: [i32; 2], goal: [i32; 2]) -> [i32; 2] {
    let delta = [
        i64::from(goal[0]) - i64::from(pos[0]),
        i64::from(goal[1]) - i64::from(pos[1]),
    ];
    let dist = distance_mm(pos, goal);
    if dist == 0 {
        return [0, 0];
    }
    let speed = (dist * 1000 / i64::from(PARAMS.dt_ms)).min(i64::from(PARAMS.max_speed_mm_s));
    [
        i32::try_from(delta[0] * speed / dist).expect("a clamped speed fits i32"),
        i32::try_from(delta[1] * speed / dist).expect("a clamped speed fits i32"),
    ]
}

/// Adopt `vel` and advance every position by one step.
fn advance(agents: &mut [Agent], vel: &[[i32; 2]]) {
    let step = i64::from(PARAMS.dt_ms);
    for (agent, vel) in agents.iter_mut().zip(vel) {
        agent.vel = *vel;
        let dx = i64::from(vel[0]) * step / 1000;
        let dy = i64::from(vel[1]) * step / 1000;
        agent.pos[0] = i32::try_from(i64::from(agent.pos[0]) + dx).expect("x stays inside i32 mm");
        agent.pos[1] = i32::try_from(i64::from(agent.pos[1]) + dy).expect("y stays inside i32 mm");
    }
}

/// `n` agents scattered over a square arena of half-width `half` mm, each
/// wanting the mirrored point. Seeded, so one seed is one world.
fn crossing_world(n: usize, seed: u64, half: i32) -> Vec<Agent> {
    let span = u64::try_from(2 * i64::from(half) + 1).expect("half is positive");
    let lane = |hash: u64| {
        let scaled = i64::try_from(hash % span).expect("a lane roll fits i64");
        i32::try_from(scaled - i64::from(half)).expect("a lane fits i32")
    };
    (0..n)
        .map(|i| {
            let i = u64::try_from(i).expect("an agent index fits u64");
            let hash = engine_core::splitmix64(seed ^ i.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let pos = [lane(hash), lane(hash.rotate_left(17))];
            Agent {
                pos,
                vel: [0, 0],
                pref: preferred_toward(pos, [-pos[0], -pos[1]]),
                radius_mm: RADIUS_MM,
            }
        })
        .collect()
}

/// What one corridor run left behind.
struct CorridorEnd {
    agents: Vec<Agent>,
    goals: Vec<[i32; 2]>,
    stuck: Vec<StuckAgent>,
}

/// One agent that went [`STUCK_WINDOW_TICKS`] ticks without getting
/// [`PROGRESS_MM`] closer to its goal while still short of it.
#[derive(Debug)]
struct StuckAgent {
    agent: usize,
    pos: [i32; 2],
    goal: [i32; 2],
    best_mm: i64,
    at_tick: usize,
}

/// The two hard walls of the corridor: free space between them.
fn corridor_walls() -> [Wall; 2] {
    [
        Wall::new([0, -CORRIDOR_HALF_MM], [0, 1]),
        Wall::new([0, CORRIDOR_HALF_MM], [0, -1]),
    ]
}

/// A 4 m x 12 m corridor where two four-agent files swap ends: group A starts
/// at the west end and wants the east goal, group B the other way round. Every
/// start is jittered by up to 150 mm from `seed`, so the corpus is 1,000
/// distinct crowds on the same floor and the same goals.
fn corridor_world(seed: u64) -> (Vec<Agent>, Vec<[i32; 2]>) {
    let jitter = |row: u64, salt: u64| {
        let roll = i64::try_from(engine_core::splitmix64(seed ^ (row << 8) ^ salt) % 301)
            .expect("a jitter roll fits i64");
        i32::try_from(roll - 150).expect("jitter fits i32")
    };
    let mut agents = Vec::with_capacity(8);
    let mut goals = Vec::with_capacity(8);
    for (row, y) in [-1_050, -350, 350, 1_050].into_iter().enumerate() {
        let row = u64::try_from(row).expect("four rows fit u64");
        let offset = 700 * i32::try_from(row).expect("a row index fits i32");
        agents.push(Agent {
            pos: [600 + offset + jitter(row, 1), y + jitter(row, 2)],
            vel: [0, 0],
            pref: [0, 0],
            radius_mm: RADIUS_MM,
        });
        goals.push([11_400, y]);
        agents.push(Agent {
            pos: [11_400 - offset + jitter(row, 3), y + jitter(row, 4)],
            vel: [0, 0],
            pref: [0, 0],
            radius_mm: RADIUS_MM,
        });
        goals.push([600, y]);
    }
    (agents, goals)
}

/// Step one corridor to its end, watching every agent for a stall. An agent is
/// stuck when it is still more than [`ARRIVE_MM`] from its goal and no tick in
/// the last [`STUCK_WINDOW_TICKS`] brought it [`PROGRESS_MM`] closer. A `trace`
/// sink sees every tick's state, so a replay compares the whole run.
fn run_corridor(seed: u64, orca: &mut Orca, mut trace: Option<&mut Fnv64>) -> CorridorEnd {
    let (mut agents, goals) = corridor_world(seed);
    let walls = corridor_walls();
    let mut vel = vec![[0; 2]; agents.len()];
    let mut mark: Vec<i64> = agents
        .iter()
        .zip(&goals)
        .map(|(agent, goal)| distance_mm(agent.pos, *goal))
        .collect();
    let mut mark_at = vec![0usize; agents.len()];
    let mut reported = vec![false; agents.len()];
    let mut stuck = Vec::new();
    for tick in 0..CORRIDOR_TICKS {
        for (agent, goal) in agents.iter_mut().zip(&goals) {
            agent.pref = preferred_toward(agent.pos, *goal);
        }
        orca.step(&agents, &walls, &PARAMS, &mut vel)
            .expect("the corridor crowd steps");
        advance(&mut agents, &vel);
        if let Some(trace) = trace.as_deref_mut() {
            trace_state(trace, &agents);
        }
        for (k, goal) in goals.iter().enumerate() {
            let now = distance_mm(agents[k].pos, *goal);
            if now <= mark[k] - PROGRESS_MM {
                mark[k] = now;
                mark_at[k] = tick;
            }
            if !reported[k] && now > ARRIVE_MM && tick - mark_at[k] >= STUCK_WINDOW_TICKS {
                reported[k] = true;
                stuck.push(StuckAgent {
                    agent: k,
                    pos: agents[k].pos,
                    goal: *goal,
                    best_mm: mark[k],
                    at_tick: tick,
                });
            }
        }
    }
    CorridorEnd {
        agents,
        goals,
        stuck,
    }
}

/// Little-endian bytes of every agent's position and velocity, in index order:
/// the state the sim hash sees.
fn state_bytes(agents: &[Agent]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(agents.len() * 16);
    for agent in agents {
        for word in [agent.pos[0], agent.pos[1], agent.vel[0], agent.vel[1]] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
    }
    bytes
}

/// Mix one tick of state into `trace`: the same four words per agent, in the
/// same order, that [`state_bytes`] lays out.
fn trace_state(trace: &mut Fnv64, agents: &[Agent]) {
    for agent in agents {
        trace.write_i32(agent.pos[0]);
        trace.write_i32(agent.pos[1]);
        trace.write_i32(agent.vel[0]);
        trace.write_i32(agent.vel[1]);
    }
}

/// The cost bar: 500 agents, one integer ORCA step, median and p95 under 1 ms.
#[test]
#[ignore = "timing: wall clock bar; release only, one thread, quiet PC (cargo test -p engine-nav --release --test it soak -- --ignored --test-threads=1)"]
fn five_hundred_agents_step_under_one_millisecond() {
    // Positions are fixed across the samples, so every sample does the same
    // work: one grid build, 500 neighbour feeds, 500 linear programs.
    let agents = crossing_world(CROWD_AGENTS, 2040, CROWD_HALF_MM);
    let mut orca = Orca::new();
    let mut vel = vec![[0; 2]; CROWD_AGENTS];
    for _ in 0..CROWD_WARMUP {
        orca.step(&agents, &[], &PARAMS, &mut vel)
            .expect("the warm step holds");
    }
    let mut samples = Vec::with_capacity(CROWD_SAMPLES);
    for _ in 0..CROWD_SAMPLES {
        let begin = Instant::now();
        orca.step(&agents, &[], &PARAMS, &mut vel)
            .expect("the timed step holds");
        samples.push(begin.elapsed().as_nanos());
    }
    samples.sort_unstable();
    let median = samples[CROWD_SAMPLES / 2];
    let p95 = samples[CROWD_SAMPLES * 95 / 100];
    let cpu = std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| String::from("unknown"));
    eprintln!(
        "machine: {} {} / {cpu} / {} logical cpus",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(0, |n| n.get()),
    );
    eprintln!(
        "500 agents x {CROWD_SAMPLES} steps after {CROWD_WARMUP} warmups: median {median} ns, p95 {p95} ns, min {} ns, max {} ns",
        samples[0],
        samples[CROWD_SAMPLES - 1],
    );
    assert!(
        median < STEP_BUDGET_NS,
        "median {median} ns is over the {STEP_BUDGET_NS} ns bar"
    );
    assert!(
        p95 < STEP_BUDGET_NS,
        "p95 {p95} ns is over the {STEP_BUDGET_NS} ns bar"
    );
}

/// The corridor drill: 1,000 seeded crowds (seeds 0 through 999) swap ends of
/// the hall, and nobody may stall or end short of the far goal.
#[test]
fn thousand_seeded_corridors_leave_nobody_stuck() {
    let mut orca = Orca::new();
    let mut stuck = Vec::new();
    let mut short = Vec::new();
    for seed in 0..CORRIDOR_SEEDS {
        let end = run_corridor(seed, &mut orca, None);
        stuck.extend(end.stuck.into_iter().map(|agent| (seed, agent)));
        for (k, (agent, goal)) in end.agents.iter().zip(&end.goals).enumerate() {
            let left = distance_mm(agent.pos, *goal);
            if left > ARRIVE_MM {
                short.push((seed, k, agent.pos, *goal, left));
            }
        }
    }
    let stalled: Vec<String> = stuck
        .iter()
        .take(3)
        .map(|(seed, agent)| {
            format!(
                "seed {seed} agent {} at {:?} wanted {:?}: last best {} mm from tick {}",
                agent.agent, agent.pos, agent.goal, agent.best_mm, agent.at_tick
            )
        })
        .collect();
    assert!(
        stuck.is_empty(),
        "{} agents stalled for {STUCK_WINDOW_TICKS} ticks without {PROGRESS_MM} mm of progress: {stalled:?}",
        stuck.len()
    );
    assert!(
        short.is_empty(),
        "{} agents ended over {ARRIVE_MM} mm short of the far goal: {:?}",
        short.len(),
        &short[..short.len().min(3)]
    );
}

/// One corridor run as the replay bar sees it: the trace over every tick of the
/// run, and the end-state bytes.
fn replay(seed: u64) -> (u64, Vec<u8>) {
    let mut trace = Fnv64::new();
    let end = run_corridor(seed, &mut Orca::new(), Some(&mut trace));
    (trace.finish(), state_bytes(&end.agents))
}

/// The replay bar: the same corridor seed, run twice, gives the same state on
/// every tick and the same end bytes. The trace is what carries the seed: the
/// start jitter differs, while the final bodies all sit on their goals, so the
/// end bytes alone are the same for every seed. A neighbouring seed must move
/// the trace, so the gate can say no.
#[test]
fn the_same_corridor_replays_to_identical_state_bytes() {
    let (first_trace, first_end) = replay(2040);
    let (second_trace, second_end) = replay(2040);
    let (other_trace, other_end) = replay(2041);
    eprintln!(
        "seed 2040: run 1 trace {} end {} | run 2 trace {} end {} ; seed 2041: trace {} end {}",
        hex16(first_trace),
        hash_hex(&first_end),
        hex16(second_trace),
        hash_hex(&second_end),
        hex16(other_trace),
        hash_hex(&other_end),
    );
    assert_eq!(
        first_trace, second_trace,
        "the same corridor seed gave two different run traces"
    );
    assert_eq!(
        first_end, second_end,
        "the same corridor seed gave two different end states"
    );
    assert_ne!(
        first_trace, other_trace,
        "a different seed gave the same run trace"
    );
}
