// Idea pattern from rr-debugger/rr (MIT, verified): snapshot at T, advance N,
// restore, re-advance, per-tick hash compare. Fresh write against the
// engine-ecs snapshot API (`crate::snapshot::{snapshot, restore}`); no donor
// bytes copied.
//
// What this proves: a snapshot taken at tick T restores the exact world, so
// re-running the same fixed-dt ticks reproduces every per-tick hash, and
// restoring snapshots in reverse (end, then start) time-travels backwards
// exactly. Mismatch count over all seeds must be 0.

use engine_ecs::snapshot::{restore, snapshot};
use engine_ecs::{Position, Velocity, World};

/// Fixed step used by every run below (seconds).
const DT: f32 = 0.016;
/// Warmup ticks before the snapshot at T.
const WARMUP_TICKS: u32 = 16;
/// Ticks advanced, then re-advanced after restore.
const RESIM_TICKS: u32 = 32;
/// Seeds swept by the main test; every seed must report 0 mismatches.
const SEEDS: [u64; 5] = [1, 7, 42, 1234, 2040];

/// splitmix64: deterministic stream expander, one state per seed.
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Map 64 raw bits to an f32 in roughly [-8, 8) with exact binary fractions
/// (multiples of 1/256), so integrate steps stay on benign magnitudes.
fn bits_to_coord(bits: u64) -> f32 {
    let mag = (bits & 0x7FF) as f32 / 256.0;
    if bits & 0x800 == 0 {
        mag
    } else {
        -mag
    }
}

/// Deterministic world from a seed: 24 slots, staged kills on every 5th slot,
/// then one integrate so dead slots and moved positions are both covered.
fn seeded_world(seed: u64) -> World {
    let mut rng = seed | 1;
    let mut world = World::new();
    for _ in 0..24 {
        let px = bits_to_coord(splitmix64(&mut rng));
        let py = bits_to_coord(splitmix64(&mut rng));
        let vx = bits_to_coord(splitmix64(&mut rng));
        let vy = bits_to_coord(splitmix64(&mut rng));
        world.spawn(Position { x: px, y: py }, Velocity { x: vx, y: vy });
    }
    for id in (0..24).step_by(5) {
        assert!(world.kill(id), "seeded kill must stage");
    }
    world.integrate(DT);
    world
}

/// Advance `n` fixed ticks, returning the snapshot hash after each tick.
fn advance_hashes(world: &mut World, n: u32) -> Vec<u64> {
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        world.integrate(DT);
        out.push(snapshot(world).hash);
    }
    out
}

/// Live-state hash of the world a snapshot was captured from, decoded back
/// through `restore` so the comparison goes through bytes both ways.
fn world_state_at(snap: &engine_ecs::snapshot::Snapshot) -> u64 {
    restore(&snap.bytes)
        .expect("snapshot just taken must decode")
        .state_hash()
}

/// Snapshot at T, advance N, restore, re-advance: returns the mismatch count.
/// Also exercises the reverse direction (restore end, then restore start).
fn restore_resimulate_mismatches(seed: u64) -> usize {
    let mut world = seeded_world(seed);
    for _ in 0..WARMUP_TICKS {
        world.integrate(DT);
    }

    let snap_t = snapshot(&world);
    let mut mismatches = 0;

    // Forward run from T.
    let hashes_a = advance_hashes(&mut world, RESIM_TICKS);
    let snap_end_a = snapshot(&world);

    // Restore at T must reproduce the exact snapshot bytes and live state.
    let mut replay = restore(&snap_t.bytes).expect("snapshot at T must decode");
    let snap_t_again = snapshot(&replay);
    if snap_t_again.bytes != snap_t.bytes || snap_t_again.hash != snap_t.hash {
        mismatches += 1;
    }
    if replay.state_hash() != world_state_at(&snap_t) {
        mismatches += 1;
    }

    // Re-advance from the restore; every per-tick hash must match.
    let hashes_b = advance_hashes(&mut replay, RESIM_TICKS);
    for (step, (a, b)) in hashes_a.iter().zip(hashes_b.iter()).enumerate() {
        if a != b {
            mismatches += 1;
            eprintln!("seed {seed} tick {step}: forward {a:#x} != replay {b:#x}");
        }
    }
    let snap_end_b = snapshot(&replay);
    if snap_end_a.bytes != snap_end_b.bytes || snap_end_a.hash != snap_end_b.hash {
        mismatches += 1;
    }

    // Reverse direction: restore the end snapshot, then travel back to T.
    let back_end = restore(&snap_end_a.bytes).expect("end snapshot must decode");
    if snapshot(&back_end).hash != snap_end_a.hash {
        mismatches += 1;
    }
    let back_start = restore(&snap_t.bytes).expect("start snapshot must decode");
    if snapshot(&back_start).hash != snap_t.hash {
        mismatches += 1;
    }
    // One tick forward from the reversed restore must equal the replay head.
    let mut back_fwd = restore(&snap_t.bytes).expect("start snapshot must decode");
    back_fwd.integrate(DT);
    if snapshot(&back_fwd).hash != hashes_b[0] {
        mismatches += 1;
    }

    mismatches
}

#[test]
fn restore_resimulate_reverse_zero_mismatches() {
    let mut total = 0usize;
    for seed in SEEDS {
        let m = restore_resimulate_mismatches(seed);
        assert_eq!(m, 0, "seed {seed} reported {m} mismatches");
        total += m;
    }
    assert_eq!(total, 0, "expected 0 mismatches over seeds");
    // Non-vacuous: the run must actually move state across ticks.
    let mut world = seeded_world(SEEDS[0]);
    let before = snapshot(&world).hash;
    world.integrate(DT);
    assert_ne!(
        snapshot(&world).hash,
        before,
        "script never moved the snapshot hash"
    );
}

#[test]
fn corrupt_snapshot_rejected_loudly() {
    let mut world = seeded_world(SEEDS[0]);
    for _ in 0..WARMUP_TICKS {
        world.integrate(DT);
    }
    let snap = snapshot(&world);
    // Empty input is refused, never panics.
    assert!(restore(&[]).is_err(), "empty bytes must be refused");
    // Bad version is refused.
    let mut bad = snap.bytes.clone();
    bad[0] = 0xff;
    bad[1] = 0xff;
    assert!(restore(&bad).is_err(), "bad version must be refused");
    // Truncation is refused.
    let cut = snap.bytes.len() / 2;
    assert!(
        restore(&snap.bytes[..cut]).is_err(),
        "truncated bytes must be refused"
    );
    // A perturbed world must NOT match the replay hashes, so a zero count
    // proves comparison rather than a constant.
    let mut perturbed = restore(&snap.bytes).expect("snapshot must decode");
    perturbed.pos[1].x += 0.5;
    perturbed.integrate(DT);
    let mut clean = restore(&snap.bytes).expect("snapshot must decode");
    clean.integrate(DT);
    assert_ne!(
        snapshot(&perturbed).hash,
        snapshot(&clean).hash,
        "perturbation must move the replay hash"
    );
}
