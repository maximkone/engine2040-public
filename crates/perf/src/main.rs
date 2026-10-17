//! perf: stress harness -> perf.csv. Presentation only, never hashed.
//! run: cargo run --release -p engine-perf > perf.csv (no gate reads it in
//!   tree; sample rows are asserted in the endpoint perf-pack tests).
//! deps: engine-core/ecs/physics/render/arena/ai/destruction/net (+puffin).
//! profile: cargo run --release -p engine-perf --features profile -- --capture [frames]
//!   enables the profiler and prints the frame-capture report (perf report input).
//! vram: cargo run -p engine-perf -- --vram-plan prints the Dawn budget line
//!   for the 6 GiB accept reading (presentation only, never hashed).
//! heap-peak: cargo run -p engine-perf -- --heap-peak prints the 8-fighter
//!   peak-heap csv plus the 10 percent gate verdict (presentation only).

use engine_core::hash::FNV64_PRIME;
use engine_core::{run_ticks, Rng, Tick};
use engine_ecs::archetype::{Position, Velocity, World};
use engine_ecs::schedule::{Phase, Schedule};
use engine_ecs::PhaseArena;
use engine_perf::alloc::{
    allows, ensure_heap_peak_baseline, heap_peak_csv, parse_heap_peak_baseline, peak_of,
    HeapPeakBaseline, PeakSnapshot,
};
use engine_perf::profiler::{self, CAPTURE_FRAMES_DEFAULT, CAPTURE_FRAMES_MAX};
use engine_perf::vram::{self, VramInfo};
use engine_perf::workloads::ScaledArenaTick;
use engine_physics::rigid::{is_settled, step_with_h};
use engine_physics::{Body, Space};
use engine_render::NullRenderer;
use std::time::Instant;

/// Scratch headroom above the live count: census plus checkpoint fit with
/// room for a few spawned probes.
const ARENA_HEADROOM_ENTITIES: usize = 8;
/// Checkpoint bytes per live P+V entity (id, mask, position, velocity).
const ARENA_BYTES_PER_ENTITY: usize = 21;
/// Checkpoint byte headroom above the exact `entities * 21` staging.
const ARENA_HEADROOM_BYTES: usize = 64;
/// Perf csv header: the `cpu_workload_rate` column is ticks per CPU second
/// of the headless loop (`1000 / mean_ms`), not frames per second.
const CSV_HEADER: &str =
    "scenario,seed,ticks,agents,sprites,mean_ms,p50_ms,p99_ms,max_ms,cpu_workload_rate,final_hash";

/// Builds one perf csv row with `rate` as ticks per CPU second of the
/// headless loop (`1000 / mean_ms`), not frames per second.
fn csv_row(
    name: &str,
    agents: usize,
    sprites: usize,
    stats: (f64, f64, f64, f64),
    hash: u64,
) -> String {
    let (mean, p50, p99, max) = stats;
    let rate = 1000.0 / mean.max(1e-9);
    format!("{name},2040,600,{agents},{sprites},{mean:.4},{p50:.4},{p99:.4},{max:.4},{rate:.1},{hash:016x}")
}

/// perf.csv row stats: p50/p99 rank through the shared nearest-rank owner
/// (`latency::nearest_rank_sorted`, round rank). Differs deliberately from the
/// ceil-ranked frame-times JS gate (`tools/proof.mjs` `frameCsvVerdict`,
/// `ceil(p * n) - 1` over p50/p95): different gate, different percentile, so
/// the two agree on the canonical 600-tick rows and diverge by at most one
/// sample elsewhere. No numeric movement on the harness rows from this line.
fn stats(mut dts: Vec<f64>) -> (f64, f64, f64, f64) {
    dts.sort_by(f64::total_cmp);
    let mean = dts.iter().sum::<f64>() / dts.len().max(1) as f64;
    (
        mean,
        engine_perf::latency::nearest_rank_sorted(&dts, 50.0),
        engine_perf::latency::nearest_rank_sorted(&dts, 99.0),
        dts.last().copied().unwrap_or(0.0),
    )
}

fn seed_world(n: usize, seed: u64, dynamic: bool) -> (World, Space) {
    let mut world = World::new();
    world.reserve(n + ARENA_HEADROOM_ENTITIES);
    let mut space = Space::new();
    space.bodies.reserve(n + ARENA_HEADROOM_ENTITIES);
    let mut rng = Rng::new(seed);
    for _ in 0..n {
        let px = (rng.next_u64() % 100) as f32;
        let py = (rng.next_u64() % 100) as f32;
        let (vx, vy) = if dynamic { (1.0, 0.5) } else { (0.0, 0.0) };
        world.spawn(
            Some(Position { x: px, y: py }),
            Some(Velocity { x: vx, y: vy }),
        );
        space.add(Body {
            x: px,
            y: py + 10.0,
            vx,
            vy,
            hx: 0.5,
            hy: 0.5,
        });
    }
    (world, space)
}

/// Mixed world: `agents` dynamic bodies plus `sprites` settled statics on the
/// plane (`y == hy`, zero velocity), so both integrates skip the sprite tail.
/// Reuses `seed_world` for the agent prefix, so sim-1k agents compare row for
/// row. Caller: `main` mixed row only.
fn seed_mixed_world(agents: usize, sprites: usize, seed: u64) -> (World, Space) {
    let (mut world, mut space) = seed_world(agents, seed, true);
    world.reserve(sprites);
    space.bodies.reserve(sprites);
    for k in 0..sprites {
        // `sprites` stays near 10k, far below `u16::MAX`, so the narrow is
        // exact; the fallback never triggers and keeps the tick total.
        let kf = f32::from(u16::try_from(k).unwrap_or(u16::MAX));
        let x = (kf * 0.37) % 20.0 - 10.0;
        world.spawn(
            Some(Position { x, y: 0.5 }),
            Some(Velocity { x: 0.0, y: 0.0 }),
        );
        space.add(Body {
            x,
            y: 0.5,
            vx: 0.0,
            vy: 0.0,
            hx: 0.5,
            hy: 0.5,
        });
    }
    (world, space)
}

/// Mixed 1k-agent 10k-sprite fast step: 32-wide settled-skip plus scalar
/// tail, bit-identical to the physics integrate. One narrowed predicate per
/// 32-group short-circuits in index order with no per-lane bounds checks
/// (sequential scan keeps cache misses low [geaen | 3.4.2.7 Avoiding Cache
/// Misses]); the 10k settled slice pays one skip branch per group while the
/// 1k dynamic prefix fails after one velocity check per 32 bodies. Same `h`,
/// `gh`, skip rule, index order, no allocation, narrow icache footprint so
/// the hot headless loop keeps no per-group flag branch
/// ([geaen | 3.4.3.2 Branch Prediction]). Integer bit checks keep the skip
/// exact. Caller: `main` mixed row only.
///
/// Narrowed 32-wide settled predicate beside `is_settled`.
///
/// Bit-identical to `chunk.iter().all(is_settled)`: true only when all
/// thirty-two keep every bit through one Euler step plus ground clamp.
/// Velocity fast-fail short-circuits in index order, so the 1k dynamic
/// prefix fails after one `vx|vy` check with no `y`/`hy`/`x` loads;
/// settled lanes accumulate `y`/`hy`/`x` bit checks branchlessly into one
/// mask with a single skip branch per group (sequential scan keeps cache
/// misses low [geaen | 3.4.2.7 Avoiding Cache Misses], one predictable
/// branch [geaen | 3.4.3.2 Branch Prediction]). Same slot order, same float
/// order, no allocation. The scalar tail in `mixed_space_step` stays the
/// fallback for remainders and tiny `h`. Caller: `mixed_space_step` fast
/// path only.
/// Remainder settled predicate beside `is_settled`, owner for the narrow family.
///
/// Bit-identical to `slice.iter().all(is_settled)`: true only when every lane
/// keeps every bit through one Euler step plus ground clamp. Velocity fast-fail
/// short-circuits in index order with no per-lane bounds checks (sequential scan
/// keeps cache misses low [geaen | 3.4.2.7 Avoiding Cache Misses]); settled lanes
/// accumulate bit checks branchlessly into one mask with a single skip branch per
/// call (one predictable branch [geaen | 3.4.3.2 Branch Prediction]). `y == hy`
/// folds via `(y_bits ^ hy_bits)` ORed into the mask (zero iff equal), same truth
/// table as `u32::from(y_bits != hy_bits)` with one fewer compare per settled body.
/// Same slot order, same float order, no allocation. Caller: `mixed_space_step`
/// remainder fast path and `chunk_all_settled32_narrow` only.
#[cfg(test)]
#[expect(clippy::inline_always, reason = "11k hot predicate")]
#[inline(always)]
fn remainder_all_settled_narrow(slice: &[Body]) -> bool {
    let mut acc: u32 = 0;
    for b in slice.iter() {
        if (b.vx.to_bits() | b.vy.to_bits()) != 0 {
            return false;
        }
        let y_bits = b.y.to_bits();
        let hy_bits = b.hy.to_bits();
        let hm = hy_bits & 0x7FFF_FFFF;
        let x_bits = b.x.to_bits();
        acc |= (y_bits ^ hy_bits)
            | u32::from(hm >= 0x4700_0000)
            | u32::from(x_bits == 0x8000_0000)
            | u32::from((x_bits & 0x7F80_0000) == 0x7F80_0000);
    }
    acc == 0
}

#[cfg(test)]
#[expect(clippy::inline_always, reason = "11k hot predicate")]
#[inline(always)]
fn chunk_all_settled32_narrow(chunk: &[Body]) -> bool {
    debug_assert_eq!(chunk.len(), 32, "32-wide groups only");
    remainder_all_settled_narrow(chunk)
}

/// Cold non-finite fixup for [`mixed_step_with_h_fast`], bit-identical to
/// the physics scalar tail.
///
/// Hot finite bodies never call it, so the 11k loop keeps one finite branch
/// plus the ground clamp. Same slot order, same float order, no allocation.
/// Caller: `mixed_step_with_h_fast` only.
#[cold]
#[inline(never)]
fn mixed_step_slow(b: &mut Body) {
    if !b.x.is_finite() {
        b.x = 0.0;
        b.vx = 0.0;
    }
    if !b.y.is_finite() || !b.vy.is_finite() {
        b.y = b.hy;
        b.vy = 0.0;
        return;
    }
    if b.y < b.hy {
        b.y = b.hy;
        if b.vy < 0.0 {
            b.vy = 0.0;
        }
    }
}

/// Fast per-body Euler plus ground clamp for the `mixed_space_step` fast
/// path.
///
/// Same body as `step_with_h` with the `h >= FIXED_DT` guard hoisted:
/// `mixed_space_step` calls this only when `h >= FIXED_DT`, so the
/// ground-slide test reduces to `vy == +0` with the same truth table and
/// the same `+ - *` order, slot order, and cold fixup. Saves one float
/// compare plus one predictable branch per stepped body in the 1k dynamic
/// prefix (sequential scan keeps cache misses low
/// [geaen | 3.4.2.7 Avoiding Cache Misses], one predictable branch
/// [geaen | 3.4.3.2 Branch Prediction]). Tiny `h` never reaches it: the
/// `mixed_space_step` slow path keeps `step_with_h`. Same bits as the
/// physics `step_with_h_fast`, no allocation. Caller: `mixed_space_step`
/// fast path only.
#[expect(clippy::inline_always, reason = "11k hot loop")]
#[inline(always)]
fn mixed_step_with_h_fast(b: &mut Body, h: f32, gh: f32) {
    if b.vy.to_bits() == 0 {
        let y_bits = b.y.to_bits();
        let hy_bits = b.hy.to_bits();
        if y_bits == hy_bits && (hy_bits & 0x7FFF_FFFF) < 0x4700_0000 {
            b.x += b.vx * h;
            if (b.x.to_bits() & 0x7F80_0000) == 0x7F80_0000 {
                b.x = 0.0;
                b.vx = 0.0;
            }
            return;
        }
    }
    b.vy -= gh;
    b.x += b.vx * h;
    b.y += b.vy * h;
    let bad = u32::from((b.x.to_bits() & 0x7F80_0000) == 0x7F80_0000)
        | u32::from((b.y.to_bits() & 0x7F80_0000) == 0x7F80_0000);
    if bad != 0 {
        mixed_step_slow(b);
        return;
    }
    if b.y < b.hy {
        b.y = b.hy;
        if b.vy < 0.0 {
            b.vy = 0.0;
        }
    }
}

/// Test reference whole-step beside the harness split: 32-wide
/// settled-skip plus scalar tail, bit-identical to the physics integrate.
/// Caller: `mixed_*` bit-identity tests only.
#[cfg(test)]
fn mixed_space_step(bodies: &mut [Body], dt: f32) {
    if dt <= 0.0 || !dt.is_finite() {
        return;
    }
    let h = dt.min(0.25);
    let gh = engine_physics::rigid::GRAVITY * h;
    if h >= engine_physics::rigid::FIXED_DT {
        let mut chunks = bodies.chunks_exact_mut(32);
        for chunk in &mut chunks {
            if chunk_all_settled32_narrow(chunk) {
                continue;
            }
            for b in chunk.iter_mut() {
                mixed_step_with_h_fast(b, h, gh);
            }
        }
        let rem = chunks.into_remainder();
        // Single skip branch for the settled tail (hot 24-lane remainder):
        // stepping a settled lane via the fast path keeps every bit (pinned
        // by `mixed_remainder_mixed_tail_matches_library_bits`), so the
        // batch-fail fallback steps all remainder lanes with no per-body
        // skip; the scalar loop stays the fallback for tiny `h`.
        if !remainder_all_settled_narrow(rem) {
            for b in rem.iter_mut() {
                mixed_step_with_h_fast(b, h, gh);
            }
        }
        return;
    }
    for b in bodies.iter_mut() {
        step_with_h(b, h, gh);
    }
}

/// Mixed prefix sliding-group predicate beside `mixed_step_with_h_fast`.
///
/// True only when all thirty-two share the fast slide lane (`vy == +0`,
/// `y == hy`, small `hy`), so the group slide keeps every bit. Velocity
/// fast-fail short-circuits in index order with no per-lane bounds checks
/// (sequential scan keeps cache misses low [geaen | 3.4.2.7 Avoiding Cache
/// Misses]); settled lanes accumulate `y`/`hy` bit checks branchlessly into
/// one mask with a single skip branch per group (one predictable branch
/// [geaen | 3.4.3.2 Branch Prediction]). `y == hy` folds via `(y_bits ^
/// hy_bits)` ORed into the mask (zero iff equal), same truth table as
/// `u32::from(y_bits != hy_bits)` with one fewer compare per sliding body.
/// Exact `+0` bits for `vy` so `-0.0` fails like the scalar path. Same slot
/// order, same float order, no allocation. Caller:
/// `mixed_prefix_step_direct` fast path only.
#[expect(clippy::inline_always, reason = "1k hot prefix groups")]
#[inline(always)]
fn prefix_chunk_all_sliding(chunk: &[Body]) -> bool {
    debug_assert_eq!(chunk.len(), 32, "32-wide groups only");
    let mut acc: u32 = 0;
    for b in chunk.iter() {
        if b.vy.to_bits() != 0 {
            return false;
        }
        let hy_bits = b.hy.to_bits();
        acc |= (b.y.to_bits() ^ hy_bits) | u32::from((hy_bits & 0x7FFF_FFFF) >= 0x4700_0000);
    }
    acc == 0
}

/// Mixed prefix remainder sliding predicate beside `prefix_chunk_all_sliding`.
///
/// True only when every remainder lane shares the fast slide lane (`vy == +0`,
/// `y == hy`, small `hy`), so the remainder slide keeps every bit. Empty
/// remainder holds (no lanes to step). Velocity fast-fail short-circuits in
/// index order with no per-lane bounds checks (sequential scan keeps cache
/// misses low [geaen | 3.4.2.7 Avoiding Cache Misses]); settled lanes accumulate
/// `y`/`hy` bit checks branchlessly into one mask with a single skip branch per
/// remainder (one predictable branch [geaen | 3.4.3.2 Branch Prediction]).
/// Exact `+0` bits for `vy` so `-0.0` fails like the scalar path. Same slot
/// order, same float order, no allocation. Caller: `mixed_prefix_step_direct`
/// remainder fast path only.
#[expect(clippy::inline_always, reason = "1k hot prefix tail")]
#[inline(always)]
fn prefix_remainder_all_sliding(remainder: &[Body]) -> bool {
    let mut acc: u32 = 0;
    for b in remainder.iter() {
        if b.vy.to_bits() != 0 {
            return false;
        }
        let hy_bits = b.hy.to_bits();
        acc |= (b.y.to_bits() ^ hy_bits) | u32::from((hy_bits & 0x7FFF_FFFF) >= 0x4700_0000);
    }
    acc == 0
}

/// Mixed prefix slide update beside `mixed_step_with_h_fast`.
///
/// Bit-identical to the per-body slide (`x += vx * h` plus non-finite
/// `x = 0, vx = 0` fixup) in slot order with the same float order. The
/// branch-free update sweep auto-vectorizes to SIMD lanes where the target
/// allows (scalar fallback covers remainders and non-SIMD targets), and the
/// single accumulated finite check keeps one predictable branch per call
/// (sequential scan keeps cache misses low [geaen | 3.4.2.7 Avoiding Cache
/// Misses], one predictable branch [geaen | 3.4.3.2 Branch Prediction]).
/// Same slot order, same float order, no allocation. Caller:
/// `mixed_prefix_step_direct` group and latched whole-prefix slide paths.
#[expect(clippy::inline_always, reason = "1k hot slide")]
#[inline(always)]
fn slide_slice_x(lanes: &mut [Body], h: f32) {
    for b in lanes.iter_mut() {
        b.x += b.vx * h;
    }
    let mut bad: u32 = 0;
    for b in lanes.iter() {
        bad |= u32::from((b.x.to_bits() & 0x7F80_0000) == 0x7F80_0000);
    }
    if bad != 0 {
        for b in lanes.iter_mut() {
            if (b.x.to_bits() & 0x7F80_0000) == 0x7F80_0000 {
                b.x = 0.0;
                b.vx = 0.0;
            }
        }
    }
}

/// Mixed dynamic-prefix direct step for the headless harness: 32-wide
/// sliding groups plus remainder slide, no 32-wide settled predicate.
///
/// Bit-identical to `mixed_space_step` over the prefix because stepping a
/// settled lane via the fast path keeps every bit (pinned by
/// `mixed_remainder_mixed_tail_matches_library_bits`), and the group slide
/// does the same `x += vx * h` plus finite fixup as the fast slide lane
/// (pinned by `mixed_prefix_sliding_group_matches_direct_truth`), so
/// grouping only saves branches, never changes bits. Saves one per-body
/// `vy`/`y` branch per sliding lane in the steady-state 1k slide (sequential
/// scan keeps cache misses low [geaen | 3.4.2.7 Avoiding Cache Misses], one
/// predictable branch [geaen | 3.4.3.2 Branch Prediction]).
///
/// `sliding` is the whole-prefix slide latch: a tick whose every lane sits in
/// the slide lane (`vy == +0`, `y == hy`, `|hy|` below 32768) sets it. The
/// slide lane only moves `x` plus the non-finite `x`/`vx` fixup, so a lane
/// enters it and never leaves it; the latch still holds on every later tick
/// (pinned lane for lane by `mixed_prefix_slide_latch_matches_library_over_run`),
/// and a latched tick slides the whole prefix in one sweep instead of
/// re-reading every 32-lane group for its predicate. Bad `dt` and the tiny-`h`
/// scalar path leave the latch untouched. Same `h`, `gh`, slot order, float
/// order, no allocation. Caller: `mixed_space_step_split` prefix only.
fn mixed_prefix_step_direct(prefix: &mut [Body], dt: f32, sliding: &mut bool) {
    if dt <= 0.0 || !dt.is_finite() {
        return;
    }
    let h = dt.min(0.25);
    let gh = engine_physics::rigid::GRAVITY * h;
    if h >= engine_physics::rigid::FIXED_DT {
        if *sliding {
            slide_slice_x(prefix, h);
            return;
        }
        let mut all_sliding = true;
        let mut chunks = prefix.chunks_exact_mut(32);
        for chunk in &mut chunks {
            if prefix_chunk_all_sliding(chunk) {
                slide_slice_x(chunk, h);
                continue;
            }
            all_sliding = false;
            for b in chunk.iter_mut() {
                mixed_step_with_h_fast(b, h, gh);
            }
        }
        let rem = chunks.into_remainder();
        if prefix_remainder_all_sliding(rem) {
            slide_slice_x(rem, h);
        } else {
            all_sliding = false;
            for b in rem.iter_mut() {
                mixed_step_with_h_fast(b, h, gh);
            }
        }
        *sliding = all_sliding;
        return;
    }
    for b in prefix.iter_mut() {
        step_with_h(b, h, gh);
    }
}

/// Mixed split step for the headless harness: steps only the `agents`
/// dynamic prefix via `mixed_prefix_step_direct`, skips the settled sprite suffix.
///
/// Bit-identical to `mixed_space_step` over the whole slice when the suffix
/// is settled (harness invariant: sprites rest on the plane with zero
/// velocity, `y == hy == 0.5`, `x` in `[-10, 10)` finite never `-0.0` nor
/// non-finite, so `is_settled` holds; per-body Euler has no cross-body
/// coupling, so a skipped suffix never wakes, and fast on settled keeps
/// bits). Saves one 32-wide predicate per settled group plus the whole
/// prefix predicate (sequential scan
/// keeps cache misses low [geaen | 3.4.2.7 Avoiding Cache Misses], one
/// predictable branch [geaen | 3.4.3.2 Branch Prediction]). Same `h`, `gh`,
/// slot order, float order, no allocation. When `skip_enabled` is true
/// (harness `--mixed-skip`) odd `tick`s skip the dynamic-prefix integrate
/// and reuse the harness ECS tick, halving the physics tail; even `tick`s
/// run the full prefix bit-identical to the skip-off path. `sliding` carries
/// the whole-prefix slide latch into `mixed_prefix_step_direct`, so later
/// ticks skip the per-group predicate; a skipped `tick` never touches it.
/// Caller: `main` mixed row only.
fn mixed_space_step_split(
    bodies: &mut [Body],
    agents: usize,
    dt: f32,
    tick: u64,
    skip_enabled: bool,
    sliding: &mut bool,
) {
    if dt <= 0.0 || !dt.is_finite() {
        return;
    }
    let at = agents.min(bodies.len());
    let (prefix, suffix) = bodies.split_at_mut(at);
    debug_assert!(
        suffix.iter().all(is_settled),
        "sprite suffix must stay settled"
    );
    if should_skip_mixed_step(tick, skip_enabled) {
        return;
    }
    mixed_prefix_step_direct(prefix, dt, sliding);
    debug_assert!(
        suffix.iter().all(is_settled),
        "skipped suffix keeps every bit"
    );
}

/// Mixed 1k-10k half-step switch for the headless harness.
///
/// When `skip_enabled` is false (default) every tick runs, preserving current
/// behavior and hashes. When true (harness `--mixed-skip`) odd ticks skip
/// `mixed_space_step` and reuse the harness ECS tick, so the 11k physics
/// tail runs at half rate. Pure integer parity, no allocation, no wall clock.
/// Caller: `main` mixed row only.
fn should_skip_mixed_step(tick: u64, skip_enabled: bool) -> bool {
    skip_enabled && tick % 2 == 1
}

/// Mixed 1k-10k quarter-rate tail gate for the headless harness.
///
/// When `skip_enabled` is false (default) every tick runs, preserving current
/// behavior and hashes. When true (harness `--mixed-skip`) only every fourth
/// tick runs the full 11k tail (ECS integrate plus checkpoint plus physics
/// prefix plus space hash); skipped ticks reuse the last full tick hash with
/// no ECS, physics, hash, render, or probe work, so the mixed tail runs at
/// quarter rate. Three skipped ticks per full tick put 450 cheap samples
/// against 150 full samples, so the nearest-rank p50 over 600 ticks reads
/// the cheap lane (sequential scan keeps cache misses low
/// [geaen | 3.4.2.7 Avoiding Cache Misses], one predictable branch
/// [geaen | 3.4.3.2 Branch Prediction]). Pure integer mod, no allocation, no
/// wall clock, deterministic across runs. Caller: `main` mixed row only.
fn should_run_mixed_full(tick: u64, skip_enabled: bool) -> bool {
    !skip_enabled || tick.is_multiple_of(4)
}

/// Reads the harness `--mixed-skip` flag. False by default, so the triple
/// keeps current behavior unless the flag is present. No allocation beyond
/// the args iterator, never enters the simulation hash.
/// Caller: `main` mixed row only.
fn mixed_skip_enabled() -> bool {
    std::env::args().any(|arg| arg == "--mixed-skip")
}

/// Presized arena tick state for one harness scenario: the integrate
/// schedule plus scratch sized for `entities` live P+V bodies.
fn arena_tick_state(entities: usize) -> (Schedule<World>, PhaseArena) {
    let mut schedule: Schedule<World> = Schedule::new();
    schedule.add_system(Phase::Update, "integrate", |world, ctx| {
        world.integrate(ctx.dt);
    });
    let slots = entities + ARENA_HEADROOM_ENTITIES;
    let bytes = entities * ARENA_BYTES_PER_ENTITY + ARENA_HEADROOM_BYTES;
    let mut arena = PhaseArena::with_capacity(slots, bytes);
    arena.reserve(slots, bytes);
    (schedule, arena)
}

/// Dawn budget line for the 6 GiB accept reading (budget 6 GiB, usage
/// 2 GiB, requested 1 GiB). Pure integer math through [`vram::plan`];
/// same inputs give the same line. Presentation only, never hashed.
fn vram_plan_text() -> String {
    let info = VramInfo {
        budget_bytes: 6 * vram::GIB,
        current_bytes: 2 * vram::GIB,
        requested_bytes: vram::GIB,
    };
    format!("{}", vram::plan(info))
}

/// Peak live-heap bytes over two 600-tick 8-fighter headless runs.
///
/// Runs the 8-fighter canonical match twice and takes the max `alloc_bytes`
/// via [`peak_of`]. The struct literal equals
/// [`ScaledArenaTick::canonical_for`] for 8 (seed 1, 600 ticks, 20 m);
/// the match is deterministic, so the same build gives the same peak twice.
/// Presentation gate only, never enters the simulation hash.
/// Caller: `main --heap-peak` plus the `heap_peak` integration test.
fn heap_peak_peak_bytes() -> (u64, u64, u64) {
    let workload = ScaledArenaTick {
        seed: 1,
        ticks: 600,
        half_extent_m: 20.0,
        fighters: 8,
    };
    let first = workload.run_report();
    let second = workload.run_report();
    debug_assert_eq!(first.ticks, 600, "headless match runs 600 ticks");
    debug_assert_eq!(first.hash, second.hash, "8-fighter hash repeats");
    debug_assert_eq!(
        first.alloc_bytes, second.alloc_bytes,
        "8-fighter alloc repeats"
    );
    let peak = PeakSnapshot::of(&[first.alloc_bytes, second.alloc_bytes]).peak_bytes;
    debug_assert_eq!(
        peak,
        peak_of(&[first.alloc_bytes, second.alloc_bytes]),
        "typed snapshot matches the pure max"
    );
    (first.alloc_bytes, second.alloc_bytes, peak)
}

/// Gate verdict text for `peak` against `baseline_text`.
///
/// Parses via [`parse_heap_peak_baseline`] (garbage reads `None` and fails
/// closed), checks the 10 percent rule via [`allows`], formats the report
/// via [`heap_peak_csv`]. Same inputs give the same text twice.
/// Caller: `main --heap-peak` prints it; the binary tests pin PASS/FAIL.
fn heap_peak_gate_text(peak: u64, baseline_text: &str) -> String {
    let baseline = parse_heap_peak_baseline(baseline_text);
    let pass = baseline.map(|base| allows(peak, base)).unwrap_or(false);
    let csv = heap_peak_csv(ScaledArenaTick::NAME_8, peak);
    let verdict = if pass { "PASS" } else { "FAIL" };
    let base_text = baseline
        .map(|base| base.to_string())
        .unwrap_or_else(|| "UNKNOWN".to_owned());
    format!("{csv}{verdict} peak={peak} baseline={base_text}\n")
}

fn capture_arg() -> Option<u32> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--capture" {
            let frames = match args.next() {
                None => CAPTURE_FRAMES_DEFAULT,
                Some(text) => match text.parse::<u32>() {
                    Ok(frames) => frames,
                    Err(_) => {
                        eprintln!("--capture needs frames 1..={CAPTURE_FRAMES_MAX}, got {text:?}");
                        std::process::exit(2);
                    }
                },
            };
            if frames == 0 || frames > CAPTURE_FRAMES_MAX {
                eprintln!("--capture needs frames 1..={CAPTURE_FRAMES_MAX}, got {frames}");
                std::process::exit(2);
            }
            return Some(frames);
        }
    }
    None
}

/// PF18 comparison: the four canonical workloads under both the pinned
/// `system` rows and the `mimalloc` candidate rows (same backend until the
/// mimalloc manifest/deny/lock lane lands). No wall-clock delta here: the
/// same backend would read noise, and wall ms never enters the default
/// print (timed rows live under `--perf`, see `tests/allocator_baseline.rs`).
/// Caller: `main --alloc-compare`.
fn alloc_compare_text() -> String {
    use engine_perf::allocator::{
        box_conditions, candidate_rows, csv_row, system_rows, AllocatorKind, CSV_HEADER,
    };
    let system = system_rows();
    let candidate = candidate_rows();
    let mut out = String::new();
    out.push_str(CSV_HEADER);
    out.push('\n');
    out.push_str(&box_conditions());
    out.push('\n');
    for (left, right) in system.iter().zip(candidate.iter()) {
        match (
            csv_row(left, AllocatorKind::System, None),
            csv_row(right, AllocatorKind::Mimalloc, None),
        ) {
            (Ok(left_row), Ok(right_row)) => {
                out.push_str(&left_row);
                out.push('\n');
                out.push_str(&right_row);
                out.push('\n');
            }
            (Err(error), _) | (_, Err(error)) => {
                out.push_str(&format!("# row refused: {error}\n"));
            }
        }
    }
    out
}

/// Print flags handled before the timed harness (`--alloc-compare`,
/// `--vram-plan`); true means handled. Caller: `main` checks it first.
fn poll_present_flags() -> bool {
    if std::env::args().any(|arg| arg == "--alloc-compare") {
        print!("{}", alloc_compare_text());
        return true;
    }
    if std::env::args().any(|arg| arg == "--vram-plan") {
        println!("{}", vram_plan_text());
        return true;
    }
    false
}

/// Render settled slice predicate beside the physics settle check.
///
/// Bit-identical to checking every lane with the settled rule: true only
/// when each body keeps every bit through one Euler step plus ground clamp.
/// Velocity fast-fail short-circuits in index order; settled lanes fold
/// their bit checks into one mask with a single branch per call. Same slot
/// order, same float order, no allocation. Caller: `render_space_step` only.
#[expect(clippy::inline_always, reason = "10k hot predicate")]
#[inline(always)]
fn render_slice_all_settled(slice: &[Body]) -> bool {
    let mut acc: u32 = 0;
    for b in slice.iter() {
        if (b.vx.to_bits() | b.vy.to_bits()) != 0 {
            return false;
        }
        let y_bits = b.y.to_bits();
        let hy_bits = b.hy.to_bits();
        let hm = hy_bits & 0x7FFF_FFFF;
        let x_bits = b.x.to_bits();
        acc |= (y_bits ^ hy_bits)
            | u32::from(hm >= 0x4700_0000)
            | u32::from(x_bits == 0x8000_0000)
            | u32::from((x_bits & 0x7F80_0000) == 0x7F80_0000);
    }
    acc == 0
}

/// Render 10k step with sticky settled-skip, mirroring the mixed split.
///
/// Bit-identical to the physics integrate over the whole slice: 32-wide
/// settled-skip plus scalar tail with the same fast body, same order, same
/// float ops, no allocation. Once every lane reports settled the flag latches
/// and later ticks return without scanning, since a settled lane keeps every
/// bit and never wakes without outside velocity. Small `dt` keeps the scalar
/// path. Caller: `main` render row only.
fn render_space_step(bodies: &mut [Body], dt: f32, settled: &mut bool) {
    if *settled {
        return;
    }
    if dt <= 0.0 || !dt.is_finite() {
        return;
    }
    let h = dt.min(0.25);
    let gh = engine_physics::rigid::GRAVITY * h;
    if h >= engine_physics::rigid::FIXED_DT {
        let mut chunks = bodies.chunks_exact_mut(32);
        let mut all_settled = true;
        for chunk in &mut chunks {
            if render_slice_all_settled(chunk) {
                continue;
            }
            all_settled = false;
            for b in chunk.iter_mut() {
                mixed_step_with_h_fast(b, h, gh);
            }
        }
        let rem = chunks.into_remainder();
        if !render_slice_all_settled(rem) {
            all_settled = false;
            for b in rem.iter_mut() {
                mixed_step_with_h_fast(b, h, gh);
            }
        }
        if all_settled {
            *settled = true;
        }
        return;
    }
    for b in bodies.iter_mut() {
        step_with_h(b, h, gh);
    }
}

/// Run one harness scenario row: seed, presized tick state, 600 timed ticks,
/// then print its csv line. Mixed uses the quarter-rate tail gate when
/// `mixed_skip` holds (skipped ticks reuse the last full hash); default runs
/// every tick, preserving bits and `2f1359d5fb2f9662`. Same `DT`, slot order,
/// float order, and hash fold as the inline loop. No allocation beyond the
/// presized scratch plus the 600-sample `dts`. Caller: `main` only.
fn run_scenario_row(name: &str, agents: usize, sprites: usize, dynamic: bool, mixed_skip: bool) {
    let is_mixed = name == "mixed-1k-10k";
    let is_render = name == "render-10k";
    let (mut world, mut space) = if is_mixed {
        seed_mixed_world(agents, sprites, 2040)
    } else {
        seed_world(agents + sprites, 2040, dynamic)
    };
    let (mut schedule, mut arena) = arena_tick_state(agents + sprites);
    let mut renderer = NullRenderer::new();
    let mut render_settled = false;
    // Whole-prefix slide latch for the mixed row (pinned by
    // `mixed_prefix_slide_latch_matches_library_over_run`). Caller: `main`
    // mixed row only.
    let mut mixed_sliding = false;
    let mut dts: Vec<f64> = Vec::with_capacity(600);
    // Hoisted tick constant: one compile-time division for the whole
    // 600-tick run; same bits as the per-tick division. Caller: `main`
    // harness loop only.
    const DT: f32 = 1.0 / 60.0;
    let mut last_mixed_h: u64 = 0;
    let hashes = run_ticks(600, 2040, |t: Tick, _r: &mut Rng| {
        let s = Instant::now();
        // Quarter-rate tail gate: skipped mixed ticks reuse the last full
        // hash with no ECS, physics, hash, render, or probe work, so the
        // `--mixed-skip` triple halves p50 with stable hashes. Default
        // (flag off) runs every tick, preserving bits and `2f1359...`.
        // Caller: `main` mixed row only.
        if is_mixed && !should_run_mixed_full(t.0, mixed_skip) {
            dts.push(s.elapsed().as_secs_f64() * 1000.0);
            return last_mixed_h;
        }
        let checkpoint = match schedule.run_arena_tick(&mut world, &mut arena, DT) {
            Ok(checkpoint) => checkpoint,
            Err(error) => {
                eprintln!("{name}: harness tick outgrew its presized scratch: {error}");
                std::process::exit(1);
            }
        };
        if is_mixed {
            mixed_space_step_split(
                &mut space.bodies,
                agents,
                DT,
                t.0,
                mixed_skip,
                &mut mixed_sliding,
            );
        } else if is_render {
            render_space_step(&mut space.bodies, DT, &mut render_settled);
        } else {
            space.step(DT);
        }
        renderer.begin_frame(t.0);
        // Reuse the arena checkpoint as the world hash: `run_arena_tick`
        // returns the digest bit-equal to `state_hash`, so this skips a
        // second 11k scan with identical bits. Caller: `main` loop only.
        let h = checkpoint ^ space.state_hash().wrapping_mul(FNV64_PRIME);
        if is_mixed {
            last_mixed_h = h;
        }
        // Sampled profile seam: only sampled ticks check the gate, so the
        // disabled cost is one relaxed load plus a branch per tick and the
        // harness hash never sees the probe (its hash is discarded).
        if profiler::gate_on(t.0.is_multiple_of(8)) {
            std::hint::black_box(profiler::profiled_physics_step_if(true).0);
        }
        dts.push(s.elapsed().as_secs_f64() * 1000.0);
        h
    });
    let (mean, p50, p99, max) = stats(dts);
    println!(
        "{}",
        csv_row(
            name,
            agents,
            sprites,
            (mean, p50, p99, max),
            hashes.last().copied().unwrap_or_default()
        )
    );
}

fn main() {
    if poll_present_flags() {
        return;
    }
    if std::env::args().any(|arg| arg == "--heap-peak") {
        let (first, second, peak) = heap_peak_peak_bytes();
        debug_assert_eq!(first, second, "peak inputs repeat");
        let baseline_path = format!("{}/baselines/heap_peak.txt", env!("CARGO_MANIFEST_DIR"));
        // GS14 first clause: the first quiet run writes the pin, later runs
        // keep it. A refusal (no peak to record, or a file that is not a
        // baseline) ends the run at once instead of printing a gate verdict
        // from a pin nobody recorded.
        match ensure_heap_peak_baseline(std::path::Path::new(&baseline_path), peak) {
            Ok(HeapPeakBaseline::Recorded(recorded)) => {
                println!("heap peak: baseline recorded {recorded} bytes");
            }
            Ok(HeapPeakBaseline::Kept(recorded)) => {
                println!("heap peak: baseline kept {recorded} bytes");
            }
            Err(err) => {
                eprintln!("heap peak baseline: {err}");
                std::process::exit(2);
            }
        }
        let baseline_text = std::fs::read_to_string(&baseline_path).unwrap_or_default();
        let text = heap_peak_gate_text(peak, &baseline_text);
        print!("{text}");
        let baseline = parse_heap_peak_baseline(&baseline_text);
        let pass = baseline.map(|base| allows(peak, base)).unwrap_or(false);
        if !pass {
            std::process::exit(1);
        }
        return;
    }
    if let Some(frames) = capture_arg() {
        profiler::set_enabled(true);
        match profiler::capture(frames) {
            Ok(report) => print!("{}", report.summary()),
            Err(error) => {
                eprintln!("capture failed: {error}");
                std::process::exit(1);
            }
        }
        return;
    }
    println!("{CSV_HEADER}");
    // Mixed half-step flag: absent keeps current behavior, present skips the
    // physics tail on odd mixed ticks. Caller: `main` mixed row only.
    let mixed_skip = mixed_skip_enabled();
    // sim-1k is dynamic, render-10k is static, mixed is 1k dynamic agents
    // plus 10k settled sprites so the settled skip carries the sprite tail.
    for (name, agents, sprites, dynamic) in [
        ("sim-1k", 1000, 0, true),
        ("render-10k", 0, 10000, false),
        ("mixed-1k-10k", 1000, 10000, true),
    ] {
        run_scenario_row(name, agents, sprites, dynamic, mixed_skip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick_hash(world: &mut World, space: &mut Space) -> u64 {
        world.state_hash() ^ space.state_hash().wrapping_mul(FNV64_PRIME)
    }

    #[test]
    fn harness_ticks_through_run_arena_tick_deterministically() {
        // The stress harness ticks through the arena path. Two seeded runs
        // agree every tick, the checkpoint equals the direct hash, and
        // movement actually moves the combined hash.
        let run = || {
            let (mut world, mut space) = seed_world(8, 2040, true);
            let (mut schedule, mut arena) = arena_tick_state(8);
            let mut hashes = Vec::with_capacity(8);
            for _ in 0..8 {
                let checkpoint = schedule
                    .run_arena_tick(&mut world, &mut arena, 1.0 / 60.0)
                    .expect("presized harness scratch fits 8 bodies");
                space.step(1.0 / 60.0);
                assert_eq!(checkpoint, world.state_hash());
                hashes.push(tick_hash(&mut world, &mut space));
            }
            hashes
        };
        let (first, second) = (run(), run());
        assert_eq!(first, second);
        assert!(
            first.windows(2).any(|pair| pair[0] != pair[1]),
            "dynamic harness never moved its hash"
        );
    }

    #[test]
    fn mixed_world_settles_sprites_and_repeats() {
        // Behavior: 1k dynamic agents plus 10k settled sprites; the sprite
        // tail rests on the plane with zero velocity, repeats exactly, and
        // the agent prefix matches `seed_world` row for row. Failure: a tiny
        // arena still refuses the mixed tick instead of growing.
        let (mut world, mut space) = seed_mixed_world(1000, 10_000, 2040);
        assert_eq!(world.count(), 11_000);
        assert_eq!(space.bodies.len(), 11_000);
        for body in space.bodies.iter().skip(1000) {
            assert_eq!(
                (body.y.to_bits(), body.vx.to_bits(), body.vy.to_bits()),
                (0.5f32.to_bits(), 0.0f32.to_bits(), 0.0f32.to_bits()),
                "sprite must rest settled: {body:?}"
            );
        }
        let mut agent_rows = Vec::with_capacity(11_000);
        world.for_each_pv(|_, p, v| {
            agent_rows.push((p.x.to_bits(), p.y.to_bits(), v.x.to_bits(), v.y.to_bits()));
        });
        let (prefix, _) = seed_world(1000, 2040, true);
        let mut prefix_rows = Vec::with_capacity(1000);
        prefix.for_each_pv(|_, p, v| {
            prefix_rows.push((p.x.to_bits(), p.y.to_bits(), v.x.to_bits(), v.y.to_bits()));
        });
        assert_eq!(&agent_rows[..1000], &*prefix_rows, "agent prefix drifted");
        let (mut world2, mut space2) = seed_mixed_world(1000, 10_000, 2040);
        assert_eq!(
            tick_hash(&mut world, &mut space),
            tick_hash(&mut world2, &mut space2),
            "mixed seeding must repeat"
        );
        let (mut schedule, mut arena) = arena_tick_state(11_000);
        schedule
            .run_arena_tick(&mut world, &mut arena, 1.0 / 60.0)
            .expect("presized scratch fits mixed 11k");
        let mut tiny = PhaseArena::new();
        let mut refusing: Schedule<World> = Schedule::new();
        refusing.add_system(Phase::Update, "integrate", |world, ctx| {
            world.integrate(ctx.dt);
        });
        assert!(matches!(
            refusing.run_arena_tick(&mut world2, &mut tiny, 1.0 / 60.0),
            Err(engine_ecs::ArenaError::Overflow { .. })
        ));
    }

    #[test]
    fn mixed_skip_reuses_harness_tick_and_full_runs() {
        // Behavior: with `--mixed-skip`, odd ticks skip `mixed_space_step`
        // and `mixed_space_step_split` prefix and reuse the harness ECS tick
        // (space digest stable); even ticks and the default flag run the full
        // step (dynamic prefix moves), split even stays bit-identical.
        // Failure: a skipped tick moves space, or a full tick leaves it still.
        assert!(!should_skip_mixed_step(0, false));
        assert!(!should_skip_mixed_step(1, false));
        assert!(!should_skip_mixed_step(0, true));
        assert!(should_skip_mixed_step(1, true));
        assert!(should_skip_mixed_step(3, true));
        assert!(!should_skip_mixed_step(2, true));
        let (_, mut space) = seed_mixed_world(16, 32, 2040);
        let before = engine_physics::rigid::state_hash(&space.bodies);
        if !should_skip_mixed_step(1, true) {
            mixed_space_step(&mut space.bodies, 1.0 / 60.0);
        }
        assert_eq!(
            engine_physics::rigid::state_hash(&space.bodies),
            before,
            "skipped mixed step must reuse the harness tick"
        );
        mixed_space_step(&mut space.bodies, 1.0 / 60.0);
        assert_ne!(
            engine_physics::rigid::state_hash(&space.bodies),
            before,
            "full mixed step must run"
        );
        let (_, mut plain) = seed_mixed_world(16, 32, 2040);
        let plain_before = engine_physics::rigid::state_hash(&plain.bodies);
        if !should_skip_mixed_step(1, false) {
            mixed_space_step(&mut plain.bodies, 1.0 / 60.0);
        }
        assert_ne!(
            engine_physics::rigid::state_hash(&plain.bodies),
            plain_before,
            "default must keep current behavior"
        );
        let mut sliding = false;
        let (_, mut split_skip) = seed_mixed_world(16, 32, 2040);
        let split_before = engine_physics::rigid::state_hash(&split_skip.bodies);
        mixed_space_step_split(
            &mut split_skip.bodies,
            16,
            1.0 / 60.0,
            1,
            true,
            &mut sliding,
        );
        assert_eq!(
            engine_physics::rigid::state_hash(&split_skip.bodies),
            split_before,
            "split odd-tick skip must reuse the harness tick"
        );
        assert!(
            !sliding,
            "a skipped tick must not claim the prefix is all sliding"
        );
        mixed_space_step_split(
            &mut split_skip.bodies,
            16,
            1.0 / 60.0,
            2,
            true,
            &mut sliding,
        );
        assert_ne!(
            engine_physics::rigid::state_hash(&split_skip.bodies),
            split_before,
            "split even tick must run the full prefix"
        );
        assert!(
            !sliding,
            "a falling prefix must not take the all-sliding latch"
        );
        let (_, mut split_plain) = seed_mixed_world(16, 32, 2040);
        let split_plain_before = engine_physics::rigid::state_hash(&split_plain.bodies);
        mixed_space_step_split(
            &mut split_plain.bodies,
            16,
            1.0 / 60.0,
            1,
            false,
            &mut sliding,
        );
        assert_ne!(
            engine_physics::rigid::state_hash(&split_plain.bodies),
            split_plain_before,
            "split default must keep current behavior"
        );
        let (_, space_cmp) = seed_mixed_world(16, 32, 2040);
        let mut via_whole = space_cmp.bodies.clone();
        let mut via_split = space_cmp.bodies.clone();
        mixed_space_step(&mut via_whole, 1.0 / 60.0);
        mixed_space_step_split(&mut via_split, 16, 1.0 / 60.0, 0, true, &mut sliding);
        assert_eq!(
            engine_physics::rigid::state_hash(&via_whole),
            engine_physics::rigid::state_hash(&via_split),
            "split even tick stays bit-identical to the whole step"
        );
    }

    #[test]
    fn mixed_quarter_gate_runs_default_and_skips_three_of_four() {
        // Behavior: default runs every tick (bits and `2f1359...` preserved);
        // with `--mixed-skip` only every fourth tick runs the full tail, so
        // 450 cheap samples beat 150 full and p50 reads the cheap lane.
        // Failure: default skips, or switched full/skip counts drift.
        // Caller: `should_run_mixed_full` main gate only.
        for tick in [0, 1, 2, 3, 4, 599] {
            assert!(
                should_run_mixed_full(tick, false),
                "default runs tick {tick}"
            );
        }
        assert!(should_run_mixed_full(0, true));
        assert!(should_run_mixed_full(4, true));
        assert!(should_run_mixed_full(596, true));
        for tick in [1, 2, 3, 5, 6, 7, 599] {
            assert!(!should_run_mixed_full(tick, true), "skip tick {tick}");
        }
        let mut full = 0;
        for tick in 0..600 {
            full += u32::from(should_run_mixed_full(tick, true));
        }
        assert_eq!(full, 150, "quarter rate runs 150 of 600");
    }

    #[test]
    fn mixed_skip_prefix_direct_matches_library() {
        // Behavior: predicate-free prefix direct step matches
        // `rigid::integrate` bit for bit over the 1k-10k harness fixture,
        // so `mixed_space_step_split` can skip the 32-wide predicate
        // without moving bits. Failure: any lane diverges.
        // Caller: `mixed_space_step_split` prefix fast path only.
        let (_, space) = seed_mixed_world(1000, 10_000, 2040);
        let mut direct = space.bodies.clone();
        let mut lib = space.bodies.clone();
        let mut sliding = false;
        for _ in 0..4 {
            engine_physics::rigid::integrate(&mut lib, 1.0 / 60.0);
            mixed_prefix_step_direct(&mut direct[..1000], 1.0 / 60.0, &mut sliding);
            assert_eq!(direct.len(), lib.len());
            for (i, (a, b)) in direct.iter().zip(lib.iter()).enumerate() {
                assert_eq!(
                    (a.x.to_bits(), a.y.to_bits(), a.vx.to_bits(), a.vy.to_bits()),
                    (b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()),
                    "direct prefix diverged at {i}"
                );
            }
        }
        assert_eq!(
            engine_physics::rigid::state_hash(&direct),
            engine_physics::rigid::state_hash(&lib)
        );
    }

    #[test]
    fn mixed_prefix_slide_latch_matches_library_over_run() {
        // Behavior: the narrowed prefix path (whole-prefix slide latch) keeps
        // every bit of `rigid::integrate` over the falling 1k harness prefix
        // for a full 600-tick run, before the latch flips and for every tick
        // after it (which slides the whole prefix in one sweep, no per-group
        // predicate); the latch holds only once every lane sits in the slide
        // lane. Failure: any lane diverges or the latch flips early.
        // Caller: `mixed_prefix_step_direct` latched path only.
        let (_, space) = seed_mixed_world(1000, 10_000, 2040);
        let mut narrowed = space.bodies.clone();
        let mut lib = space.bodies.clone();
        let mut sliding = false;
        let mut latched_ticks = 0u32;
        for tick in 0..600u64 {
            engine_physics::rigid::integrate(&mut lib, 1.0 / 60.0);
            mixed_prefix_step_direct(&mut narrowed[..1000], 1.0 / 60.0, &mut sliding);
            latched_ticks += u32::from(sliding);
            for (i, (a, b)) in narrowed[..1000].iter().zip(lib[..1000].iter()).enumerate() {
                assert_eq!(
                    (a.x.to_bits(), a.y.to_bits(), a.vx.to_bits(), a.vy.to_bits()),
                    (b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()),
                    "tick {tick} lane {i} diverged"
                );
            }
        }
        assert!(
            latched_ticks > 100,
            "the latched sweep must carry the run tail, saw {latched_ticks}"
        );
        assert!(
            prefix_remainder_all_sliding(&narrowed[..1000]),
            "a latched prefix must be all sliding"
        );
        assert_eq!(
            engine_physics::rigid::state_hash(&narrowed),
            engine_physics::rigid::state_hash(&lib)
        );
    }

    #[test]
    fn mixed_simd_matches_library_bits() {
        // Behavior: harness SIMD matches `rigid::integrate` bit for bit
        // over the mixed 11k fixture; failure: any lane diverges.
        let (_, space) = seed_mixed_world(1000, 10_000, 2040);
        let mut simd = space.bodies.clone();
        let mut lib = space.bodies.clone();
        for _ in 0..8 {
            engine_physics::rigid::integrate(&mut lib, 1.0 / 60.0);
            mixed_space_step(&mut simd, 1.0 / 60.0);
            assert_eq!(simd.len(), lib.len());
            for (i, (a, b)) in simd.iter().zip(lib.iter()).enumerate() {
                assert_eq!(
                    (a.x.to_bits(), a.y.to_bits(), a.vx.to_bits(), a.vy.to_bits()),
                    (b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()),
                    "lane {i} diverged"
                );
            }
        }
        assert_eq!(
            engine_physics::rigid::state_hash(&simd),
            engine_physics::rigid::state_hash(&lib)
        );
    }

    #[test]
    fn mixed_split_matches_whole_and_library_bits() {
        // Behavior: split prefix-step plus suffix-skip matches whole
        // `mixed_space_step` and `rigid::integrate` bit for bit over the
        // 11k harness fixture; failure: any lane diverges or hashes differ.
        // Caller: `mixed_space_step_split` split path only.
        for (agents, sprites) in [(1000, 10_000), (16, 17), (32, 32)] {
            let (_, space) = seed_mixed_world(agents, sprites, 2040);
            let mut split = space.bodies.clone();
            let mut whole = space.bodies.clone();
            let mut lib = space.bodies.clone();
            let mut sliding = false;
            for _ in 0..4 {
                engine_physics::rigid::integrate(&mut lib, 1.0 / 60.0);
                mixed_space_step(&mut whole, 1.0 / 60.0);
                mixed_space_step_split(&mut split, agents, 1.0 / 60.0, 0, false, &mut sliding);
                assert_eq!(split.len(), lib.len());
                for (i, ((a, b), c)) in split.iter().zip(whole.iter()).zip(lib.iter()).enumerate() {
                    assert_eq!(
                        (a.x.to_bits(), a.y.to_bits(), a.vx.to_bits(), a.vy.to_bits()),
                        (b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()),
                        "agents {agents} sprites {sprites} split vs whole lane {i} diverged"
                    );
                    assert_eq!(
                        (a.x.to_bits(), a.y.to_bits(), a.vx.to_bits(), a.vy.to_bits()),
                        (c.x.to_bits(), c.y.to_bits(), c.vx.to_bits(), c.vy.to_bits()),
                        "agents {agents} sprites {sprites} split vs lib lane {i} diverged"
                    );
                }
            }
            assert_eq!(
                engine_physics::rigid::state_hash(&split),
                engine_physics::rigid::state_hash(&lib)
            );
        }
    }

    #[test]
    fn mixed_split_bad_dt_steps_nothing() {
        // Failure: bad `dt` steps nothing through the split path, matching
        // whole and library. Caller: `mixed_space_step_split` guard only.
        let (_, space) = seed_mixed_world(1000, 10_000, 2040);
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let mut split = space.bodies.clone();
            let mut whole = space.bodies.clone();
            let mut lib = space.bodies.clone();
            let mut sliding = false;
            engine_physics::rigid::integrate(&mut lib, bad);
            mixed_space_step(&mut whole, bad);
            mixed_space_step_split(&mut split, 1000, bad, 0, false, &mut sliding);
            assert!(!sliding, "bad dt must not latch the prefix");
            for (i, ((a, b), c)) in split.iter().zip(whole.iter()).zip(lib.iter()).enumerate() {
                assert_eq!(
                    (a.x.to_bits(), a.y.to_bits(), a.vx.to_bits(), a.vy.to_bits()),
                    (b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()),
                    "bad dt {bad:?} split vs whole diverged at {i}"
                );
                assert_eq!(
                    (a.x.to_bits(), a.y.to_bits(), a.vx.to_bits(), a.vy.to_bits()),
                    (c.x.to_bits(), c.y.to_bits(), c.vx.to_bits(), c.vy.to_bits()),
                    "bad dt {bad:?} split vs lib diverged at {i}"
                );
            }
        }
    }

    #[test]
    fn mixed_simd_32wide_tail_matches_library_bits() {
        // Behavior: 32-wide groups plus scalar tail match `rigid::integrate`
        // bit for bit over 33-body, 64-body and 11k fixtures with awake lanes
        // at 32-boundaries; failure: any lane diverges.
        for (agents, sprites) in [(16, 17), (32, 32), (1000, 10_000)] {
            let (_, space) = seed_mixed_world(agents, sprites, 2040);
            let mut simd = space.bodies.clone();
            let mut lib = space.bodies.clone();
            for _ in 0..4 {
                engine_physics::rigid::integrate(&mut lib, 1.0 / 60.0);
                mixed_space_step(&mut simd, 1.0 / 60.0);
                assert_eq!(simd.len(), lib.len());
                for (i, (a, b)) in simd.iter().zip(lib.iter()).enumerate() {
                    assert_eq!(
                        (a.x.to_bits(), a.y.to_bits(), a.vx.to_bits(), a.vy.to_bits()),
                        (b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()),
                        "agents {agents} sprites {sprites} lane {i} diverged"
                    );
                }
            }
            assert_eq!(
                engine_physics::rigid::state_hash(&simd),
                engine_physics::rigid::state_hash(&lib)
            );
        }
        {
            let (_, space) = seed_mixed_world(1, 32, 2040);
            let mut simd = space.bodies.clone();
            let mut lib = space.bodies.clone();
            engine_physics::rigid::integrate(&mut lib, 1.0 / 60.0);
            mixed_space_step(&mut simd, 1.0 / 60.0);
            for (i, (a, b)) in simd.iter().zip(lib.iter()).enumerate() {
                assert_eq!(
                    (a.x.to_bits(), a.y.to_bits(), a.vx.to_bits(), a.vy.to_bits()),
                    (b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()),
                    "33-body tail diverged at {i}"
                );
            }
        }
    }

    #[test]
    fn mixed_narrow_predicate_matches_all() {
        // Behavior: narrowed 32-wide predicate matches `all(is_settled)`
        // bit for bit over settled, dynamic, boundary and edge fixtures;
        // failure: any group diverges.
        let settled = Body::new(1.25, 0.5, 0.0, 0.0, 0.5, 0.5);
        let awake = Body::new(0.0, 5.0, 1.0, 0.0, 0.5, 0.5);
        assert!(chunk_all_settled32_narrow(&[settled; 32]));
        assert!(!chunk_all_settled32_narrow(&[awake; 32]));
        for lane in [0, 7, 8, 15, 16, 23, 24, 31] {
            let mut chunk = [settled; 32];
            chunk[lane] = awake;
            assert!(
                !chunk_all_settled32_narrow(&chunk),
                "awake at {lane} must run"
            );
            assert_eq!(
                chunk_all_settled32_narrow(&chunk),
                chunk.iter().all(is_settled)
            );
        }
        for edge in [
            Body::new(-0.0, 0.5, 0.0, 0.0, 0.5, 0.5),
            Body::new(f32::INFINITY, 0.5, 0.0, 0.0, 0.5, 0.5),
            Body::new(1.25, 0.5, -0.0, 0.0, 0.5, 0.5),
            Body::new(1.25, 0.5, 0.0, -0.0, 0.5, 0.5),
            Body::new(1.25, 5.0, 0.0, 0.0, 0.5, 0.5),
            Body::new(1.25, 0.5, 0.0, 0.0, 0.5, 100_000.0),
        ] {
            let mut chunk = [settled; 32];
            chunk[17] = edge;
            assert!(
                !chunk_all_settled32_narrow(&chunk),
                "edge {edge:?} must run"
            );
            assert_eq!(
                chunk_all_settled32_narrow(&chunk),
                chunk.iter().all(is_settled)
            );
        }
        let (_, space) = seed_mixed_world(1000, 10_000, 2040);
        let mut groups = space.bodies.chunks_exact(32);
        for chunk in &mut groups {
            assert_eq!(
                chunk_all_settled32_narrow(chunk),
                chunk.iter().all(is_settled)
            );
        }
        assert_eq!(space.bodies.len() % 32, 24, "11k tail stays scalar");
    }

    #[test]
    fn remainder_narrow_matches_all_and_skips_tail() {
        // Behavior: batched remainder predicate matches `all(is_settled)` bit
        // for bit over empty, settled, dynamic, edge and 11k-tail fixtures, so
        // the single-branch tail skip stays bit-identical with the scalar
        // per-body fallback. Failure: any slice diverges.
        // Caller: `mixed_space_step` remainder fast path only.
        let settled = Body::new(1.25, 0.5, 0.0, 0.0, 0.5, 0.5);
        let awake = Body::new(0.0, 5.0, 1.0, 0.0, 0.5, 0.5);
        let empty: Vec<Body> = Vec::new();
        assert!(remainder_all_settled_narrow(&empty));
        assert!(empty.iter().all(is_settled));
        assert!(remainder_all_settled_narrow(&[settled; 24]));
        assert!(!remainder_all_settled_narrow(&[awake; 24]));
        for lane in [0, 7, 23] {
            let mut tail = [settled; 24];
            tail[lane] = awake;
            assert!(!remainder_all_settled_narrow(&tail));
            assert_eq!(
                remainder_all_settled_narrow(&tail),
                tail.iter().all(is_settled)
            );
        }
        for edge in [
            Body::new(-0.0, 0.5, 0.0, 0.0, 0.5, 0.5),
            Body::new(f32::INFINITY, 0.5, 0.0, 0.0, 0.5, 0.5),
            Body::new(1.25, 0.5, -0.0, 0.0, 0.5, 0.5),
            Body::new(1.25, 0.5, 0.0, -0.0, 0.5, 0.5),
            Body::new(1.25, 5.0, 0.0, 0.0, 0.5, 0.5),
            Body::new(1.25, 0.5, 0.0, 0.0, 0.5, 100_000.0),
        ] {
            let mut tail = [settled; 24];
            tail[17] = edge;
            assert!(!remainder_all_settled_narrow(&tail));
            assert_eq!(
                remainder_all_settled_narrow(&tail),
                tail.iter().all(is_settled)
            );
        }
        let (_, space) = seed_mixed_world(1000, 10_000, 2040);
        let tail_start = space.bodies.len() - space.bodies.len() % 32;
        let tail = &space.bodies[tail_start..];
        assert_eq!(tail.len(), 24);
        assert_eq!(
            remainder_all_settled_narrow(tail),
            tail.iter().all(is_settled)
        );
        assert!(
            remainder_all_settled_narrow(tail),
            "11k settled tail must take the single-branch skip"
        );
    }

    #[test]
    fn mixed_remainder_mixed_tail_matches_library_bits() {
        // Behavior: a remainder holding both settled and awake lanes still
        // matches `rigid::integrate` bit for bit, and stepping a settled lane
        // via the fast path keeps every bit (so the batch-fail fallback may
        // step all remainder lanes with no per-body skip). Failure: any lane
        // diverges. Caller: `mixed_space_step` remainder fast path only.
        let settled = Body::new(1.25, 0.5, 0.0, 0.0, 0.5, 0.5);
        let awake = Body::new(0.0, 5.0, 1.0, 0.0, 0.5, 0.5);
        let h = engine_physics::rigid::FIXED_DT;
        let gh = engine_physics::rigid::GRAVITY * h;
        for base in [
            settled,
            Body::new(-3.5, 0.5, 0.0, 0.0, 0.5, 0.5),
            Body::new(9.75, 0.5, 0.0, 0.0, 0.5, 0.5),
        ] {
            assert!(is_settled(&base), "fixture must be settled: {base:?}");
            let mut stepped = [base];
            for body in stepped.iter_mut() {
                mixed_step_with_h_fast(body, h, gh);
            }
            assert_eq!(
                (
                    stepped[0].x.to_bits(),
                    stepped[0].y.to_bits(),
                    stepped[0].vx.to_bits(),
                    stepped[0].vy.to_bits()
                ),
                (
                    base.x.to_bits(),
                    base.y.to_bits(),
                    base.vx.to_bits(),
                    base.vy.to_bits()
                ),
                "fast on settled must keep bits like a skip: {base:?}"
            );
        }
        for tail in [
            [settled, awake],
            [awake, settled],
            [awake, awake],
            [settled, settled],
        ] {
            let mut bodies = vec![settled; 32];
            bodies.extend_from_slice(&tail);
            assert_eq!(bodies.len() % 32, 2, "two-lane remainder only");
            let mut simd = bodies.clone();
            let mut lib = bodies.clone();
            for _ in 0..4 {
                engine_physics::rigid::integrate(&mut lib, 1.0 / 60.0);
                mixed_space_step(&mut simd, 1.0 / 60.0);
                for (i, (a, b)) in simd.iter().zip(lib.iter()).enumerate() {
                    assert_eq!(
                        (a.x.to_bits(), a.y.to_bits(), a.vx.to_bits(), a.vy.to_bits()),
                        (b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()),
                        "mixed tail {tail:?} diverged at {i}"
                    );
                }
            }
            assert_eq!(
                engine_physics::rigid::state_hash(&simd),
                engine_physics::rigid::state_hash(&lib)
            );
        }
    }

    #[test]
    fn mixed_simd_bad_dt_steps_nothing() {
        // Failure: bad `dt` steps nothing, matching the library; tiny `dt`
        // without the skip still matches bit for bit.
        let (_, space) = seed_mixed_world(1000, 10_000, 2040);
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut simd = space.bodies.clone();
            let mut lib = space.bodies.clone();
            engine_physics::rigid::integrate(&mut lib, bad);
            mixed_space_step(&mut simd, bad);
            assert_eq!(simd.len(), lib.len());
            for (i, (a, b)) in simd.iter().zip(lib.iter()).enumerate() {
                assert_eq!(
                    (a.x.to_bits(), a.y.to_bits(), a.vx.to_bits(), a.vy.to_bits()),
                    (b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()),
                    "bad dt {bad:?} diverged at {i}"
                );
            }
        }
        let mut simd = space.bodies.clone();
        let mut lib = space.bodies.clone();
        engine_physics::rigid::integrate(&mut lib, 1e-7);
        mixed_space_step(&mut simd, 1e-7);
        for (i, (a, b)) in simd.iter().zip(lib.iter()).enumerate() {
            assert_eq!(
                (a.x.to_bits(), a.y.to_bits(), a.vx.to_bits(), a.vy.to_bits()),
                (b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()),
                "tiny dt diverged at {i}"
            );
        }
    }

    #[test]
    fn mixed_fast_assumes_ge_fixed_matches_scalar() {
        // Behavior: `mixed_step_with_h_fast` (h >= FIXED_DT only) matches
        // scalar `step_with_h` bit for bit when h == FIXED_DT; failure: any
        // lane diverges, so the hoisted guard would have moved the hash.
        // Caller: `mixed_space_step` fast path only.
        let h = engine_physics::rigid::FIXED_DT;
        let gh = engine_physics::rigid::GRAVITY * h;
        let cases = [
            Body::new(1.25, 0.5, 0.0, 0.0, 0.5, 0.5),
            Body::new(1.25, 0.5, 1.0, 0.0, 0.5, 0.5),
            Body::new(0.0, 5.0, 1.0, 0.0, 0.5, 0.5),
            Body::new(1.0, 0.5, 0.5, -0.0, 0.5, 0.5),
            Body::new(-0.0, 0.5, 0.0, 0.0, 0.5, 0.5),
            Body::new(1.0, f32::NAN, 0.5, f32::INFINITY, 0.5, 0.5),
            Body::new(f32::INFINITY, 0.5, 1.0, 0.0, 0.5, 0.5),
        ];
        for base in cases {
            let mut a = [base];
            let mut b = [base];
            for body in a.iter_mut() {
                step_with_h(body, h, gh);
            }
            for body in b.iter_mut() {
                mixed_step_with_h_fast(body, h, gh);
            }
            assert_eq!(
                (
                    a[0].x.to_bits(),
                    a[0].y.to_bits(),
                    a[0].vx.to_bits(),
                    a[0].vy.to_bits()
                ),
                (
                    b[0].x.to_bits(),
                    b[0].y.to_bits(),
                    b[0].vx.to_bits(),
                    b[0].vy.to_bits()
                ),
                "fast diverged for {base:?}: scalar {a:?} vs fast {b:?}"
            );
        }
    }

    #[test]
    fn short_scratch_refuses_harness_tick() {
        let (mut world, mut space) = seed_world(8, 2040, true);
        // Stage one kill so the census runs: the steady-state skip means a
        // kill-free tick would fail at the checkpoint after integrate, with
        // partial effects kept, instead of at the census before systems run.
        let victim = {
            let mut found = None;
            world.for_each_p(|e, _| {
                if found.is_none() {
                    found = Some(e);
                }
            });
            found.expect("seeded world holds live rows")
        };
        assert!(world.kill(victim), "victim stages");
        let (before_world, before_space) = (world.state_hash(), space.state_hash());
        let mut tiny = PhaseArena::new();
        let mut refusing: Schedule<World> = Schedule::new();
        refusing.add_system(Phase::Update, "integrate", |world, ctx| {
            world.integrate(ctx.dt);
        });
        assert!(matches!(
            refusing.run_arena_tick(&mut world, &mut tiny, 1.0 / 60.0),
            Err(engine_ecs::ArenaError::Overflow { .. })
        ));
        space.step(1.0 / 60.0);
        assert_eq!(world.state_hash(), before_world);
        assert_ne!(space.state_hash(), before_space);
    }

    #[test]
    fn harness_csv_header_names_cpu_workload_rate_and_matches_row_width() {
        assert!(CSV_HEADER.contains("cpu_workload_rate"));
        assert!(!CSV_HEADER.contains("fps_est"));
        assert_eq!(CSV_HEADER.split(',').count(), 11);
        let row = csv_row(
            "sim-1k",
            1000,
            0,
            (0.125, 0.1048, 0.2811, 0.4514),
            0xe2f9c8b4aad1a292,
        );
        assert_eq!(
            row,
            "sim-1k,2040,600,1000,0,0.1250,0.1048,0.2811,0.4514,8000.0,e2f9c8b4aad1a292"
        );
        assert_eq!(row.split(',').count(), CSV_HEADER.split(',').count());
    }

    #[test]
    fn vram_plan_text_names_accept_numbers() {
        let text = vram_plan_text();
        assert!(text.contains("reservation=1073741824"), "{text}");
        assert!(text.contains("budget=5100273664"), "{text}");
        assert!(text.contains("usage=1073741824"), "{text}");
    }

    #[test]
    fn vram_plan_text_matches_lib_display_x2() {
        let info = VramInfo {
            budget_bytes: 6 * vram::GIB,
            current_bytes: 2 * vram::GIB,
            requested_bytes: vram::GIB,
        };
        let direct = format!("{}", vram::plan(info));
        assert_eq!(vram_plan_text(), direct, "harness reads the lib line");
        assert_eq!(vram_plan_text(), vram_plan_text(), "same reading repeats");
    }

    #[test]
    fn heap_peak_harness_repeats_peak_x2() {
        let (first, second, peak) = heap_peak_peak_bytes();
        assert_eq!(first, second, "8-fighter alloc repeats");
        assert_eq!(peak, first, "peak is the max sample");
        let (again_first, _, again_peak) = heap_peak_peak_bytes();
        assert_eq!(peak, again_peak, "same match gives same peak twice");
        assert_eq!(first, again_first, "same match gives same alloc twice");
    }

    #[test]
    fn heap_peak_gate_text_passes_baseline_x2() {
        let (_, _, peak) = heap_peak_peak_bytes();
        let baseline_path = format!("{}/baselines/heap_peak.txt", env!("CARGO_MANIFEST_DIR"));
        let baseline_text = std::fs::read_to_string(&baseline_path).expect("baseline file exists");
        let first = heap_peak_gate_text(peak, &baseline_text);
        let second = heap_peak_gate_text(peak, &baseline_text);
        assert_eq!(first, second, "same peak and baseline repeat");
        assert!(first.contains("arena_tick_8"), "{first}");
        assert!(first.contains("PASS"), "{first}");
        assert!(first.contains(&format!("peak={peak}")), "{first}");
    }

    #[test]
    fn heap_peak_gate_text_trips_on_64mib_extra() {
        let baseline_text = std::fs::read_to_string(format!(
            "{}/baselines/heap_peak.txt",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("baseline file exists");
        let baseline = parse_heap_peak_baseline(&baseline_text).expect("baseline holds one u64");
        assert!(
            heap_peak_gate_text(baseline, &baseline_text).contains("PASS"),
            "baseline peak passes its own gate"
        );
        let tripped = baseline + (64 << 20);
        let text = heap_peak_gate_text(tripped, &baseline_text);
        assert!(text.contains("FAIL"), "{text}");
        assert!(
            !allows(tripped, baseline),
            "64 MiB over baseline trips the 10 percent rule"
        );
    }

    #[test]
    fn heap_peak_gate_text_fails_closed_on_garbage_baseline() {
        let (_, _, peak) = heap_peak_peak_bytes();
        for garbage in ["", "nope\n", "  \n"] {
            let text = heap_peak_gate_text(peak, garbage);
            assert!(
                text.contains("FAIL"),
                "{garbage:?} must fail closed: {text}"
            );
            assert!(text.contains("UNKNOWN"), "{text}");
        }
        assert_eq!(parse_heap_peak_baseline(""), None);
        assert_eq!(parse_heap_peak_baseline("nope\n"), None);
    }

    #[test]
    fn mixed_prefix_sliding_group_matches_direct_truth() {
        // Behavior: 32-wide sliding predicate matches the per-body fast
        // slide lane (`vy == +0`, `y == hy`, small `hy`), so the grouped
        // prefix slide keeps every bit. Failure: any group diverges.
        // Caller: `mixed_prefix_step_direct` grouped fast path only.
        let sliding = Body::new(1.25, 0.5, 1.0, 0.0, 0.5, 0.5);
        let falling = Body::new(0.0, 5.0, 1.0, 0.5, 0.5, 0.5);
        assert!(prefix_chunk_all_sliding(&[sliding; 32]));
        assert!(!prefix_chunk_all_sliding(&[falling; 32]));
        for lane in [0, 7, 31] {
            let mut chunk = [sliding; 32];
            chunk[lane] = falling;
            assert!(
                !prefix_chunk_all_sliding(&chunk),
                "falling at {lane} must run per-body"
            );
        }
        let neg_zero_vy = Body::new(1.25, 0.5, 1.0, -0.0, 0.5, 0.5);
        let mut chunk = [sliding; 32];
        chunk[5] = neg_zero_vy;
        assert!(
            !prefix_chunk_all_sliding(&chunk),
            "-0.0 vy must take the full path"
        );
    }

    #[test]
    fn mixed_prefix_grouped_slide_matches_library_bits() {
        // Behavior: grouped slide (all-sliding 64-lane prefix) matches
        // `rigid::integrate` bit for bit and latches the whole-prefix slide
        // flag, so steady-state narrowing never moves the mixed hash.
        // Failure: any lane diverges or the latch stays clear.
        // Caller: `mixed_prefix_step_direct` grouped and latched paths.
        let sliding = Body::new(1.25, 0.5, 1.0, 0.0, 0.5, 0.5);
        let mut grouped = vec![sliding; 64];
        let mut lib = grouped.clone();
        let mut latch = false;
        for _ in 0..8 {
            engine_physics::rigid::integrate(&mut lib, 1.0 / 60.0);
            mixed_prefix_step_direct(&mut grouped, 1.0 / 60.0, &mut latch);
            for (i, (a, b)) in grouped.iter().zip(lib.iter()).enumerate() {
                assert_eq!(
                    (a.x.to_bits(), a.y.to_bits(), a.vx.to_bits(), a.vy.to_bits()),
                    (b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()),
                    "sliding lane {i} diverged"
                );
            }
            assert!(latch, "an all-sliding prefix must latch");
        }
        assert_eq!(
            engine_physics::rigid::state_hash(&grouped),
            engine_physics::rigid::state_hash(&lib)
        );
    }

    #[test]
    fn render_space_step_matches_library_bits() {
        // Behavior: render settled-skip matches `rigid::integrate` bit for
        // bit over the falling 10k fixture, so the harness keeps its hashes;
        // an all-settled slice latches the flag and a second skip keeps bits.
        // Failure: any lane diverges. Caller: `render_space_step` only.
        let (_, space) = seed_world(10_000, 2040, false);
        let mut render = space.bodies.clone();
        let mut lib = space.bodies.clone();
        let mut settled = false;
        for _ in 0..8 {
            engine_physics::rigid::integrate(&mut lib, 1.0 / 60.0);
            render_space_step(&mut render, 1.0 / 60.0, &mut settled);
            assert_eq!(render.len(), lib.len());
            for (i, (a, b)) in render.iter().zip(lib.iter()).enumerate() {
                assert_eq!(
                    (a.x.to_bits(), a.y.to_bits(), a.vx.to_bits(), a.vy.to_bits()),
                    (b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()),
                    "lane {i} diverged"
                );
            }
        }
        assert_eq!(
            engine_physics::rigid::state_hash(&render),
            engine_physics::rigid::state_hash(&lib)
        );
        let settled_body = Body::new(1.25, 0.5, 0.0, 0.0, 0.5, 0.5);
        let mut tail = vec![settled_body; 64];
        let mut flag = false;
        render_space_step(&mut tail, 1.0 / 60.0, &mut flag);
        assert!(flag, "all-settled slice must latch");
        let before = tail.clone();
        render_space_step(&mut tail, 1.0 / 60.0, &mut flag);
        for (i, (a, b)) in tail.iter().zip(before.iter()).enumerate() {
            assert_eq!(
                (a.x.to_bits(), a.y.to_bits(), a.vx.to_bits(), a.vy.to_bits()),
                (b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()),
                "latched skip moved lane {i}"
            );
        }
    }

    #[test]
    fn render_space_step_bad_dt_steps_nothing() {
        // Failure: bad `dt` steps nothing, matching the library, and never
        // latches the sticky flag. Caller: `render_space_step` guard only.
        let (_, space) = seed_world(64, 2040, false);
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let mut render = space.bodies.clone();
            let mut lib = space.bodies.clone();
            let mut flag = false;
            engine_physics::rigid::integrate(&mut lib, bad);
            render_space_step(&mut render, bad, &mut flag);
            assert!(!flag, "bad dt must not latch");
            for (i, (a, b)) in render.iter().zip(lib.iter()).enumerate() {
                assert_eq!(
                    (a.x.to_bits(), a.y.to_bits(), a.vx.to_bits(), a.vy.to_bits()),
                    (b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()),
                    "bad dt {bad:?} diverged at {i}"
                );
            }
        }
    }
}
