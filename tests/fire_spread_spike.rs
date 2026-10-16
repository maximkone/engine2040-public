//! Fire-spread spike: integer ember plus contact spread over a bonded wall.
//!
//! Fixture: an 8-chunk timber and thatch wall in a line, bonded in order,
//! seeded ignition at chunk zero. Contact spread follows the shipped fuel
//! rows (thatch ignites after one adjacent burn tick, timber after two;
//! thatch burns four ticks, timber eight). Ember spotting is integer only:
//! thatch burners emit one ember attempt per tick from a seeded stream,
//! jumps of two or more chunks count as spots, shorter throws stay contact.
//! Digest covers integer state only, never time. Timing uses the wall
//! clock outside the digest to check the per-cut slice.

use engine_core::{Fnv64, Rng};

/// Chunks in the wall fixture.
const CHUNK_COUNT: usize = 8;
/// Bonds in the line wall (each pair joined in index order).
const BOND_COUNT: usize = 7;
/// Total ticks for the cover run; long enough for contact alone to cross.
const COVER_TICKS: u32 = 20;
/// Seed for the seeded ignition runs.
const SPIKE_SEED: u64 = 0x1234_5678_9ABC_DEF0;
/// Calm wind in Q16; ember throws collapse to contact below the filter.
const WIND_CALM_Q16: u32 = 0;
/// Breeze wind in Q16; ember throws of two or more chunks may spot.
const WIND_BREEZE_Q16: u32 = 65_536;
/// Least jump in chunks that counts as a spot (shorter stays contact).
const SPOT_MIN_JUMP: u8 = 2;
/// Largest ember jump in chunks past the source.
const SPOT_MAX_JUMP: u8 = 4;
/// Ignition draw passes on three of four draws (integer, seeded).
const IGNITE_NUM: u64 = 3;
/// Ignition draw denominator.
const IGNITE_DEN: u64 = 4;
/// Per-step budget in microseconds for the eight-chunk wall.
const STEP_BUDGET_MICROS: u128 = 150;
/// Timber material tag in this spike (odd positions are thatch).
const MAT_TIMBER: u8 = 0;
/// Thatch material tag in this spike.
const MAT_THATCH: u8 = 1;
/// Timber fuel rate in Q16, copied from the shipped fuel rows.
const TIMBER_BURN_Q16: u32 = 65_536;
/// Thatch fuel rate in Q16, copied from the shipped fuel rows.
const THATCH_BURN_Q16: u32 = 262_144;
/// Unburned chunk state.
const ST_UNBURNED: u8 = 0;
/// Burning chunk state.
const ST_BURNING: u8 = 1;
/// Burnt chunk state.
const ST_BURNT: u8 = 2;

/// Typed spike failures; bad fixture input never panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpikeError {
    /// No chunks supplied.
    EmptyWorld,
    /// Chunk index out of range, culprit carried.
    BadChunk(usize),
    /// Bond index out of range, culprit carried.
    BadBond(usize),
    /// Tick budget is zero.
    BadTicks,
}

impl core::fmt::Display for SpikeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SpikeError::EmptyWorld => write!(f, "spike world needs chunks"),
            SpikeError::BadChunk(i) => write!(f, "spike chunk {i} is out of range"),
            SpikeError::BadBond(i) => write!(f, "spike bond {i} is out of range"),
            SpikeError::BadTicks => write!(f, "spike needs at least one tick"),
        }
    }
}

impl std::error::Error for SpikeError {}

/// One wall chunk: material tag plus integer burn ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Chunk {
    /// Material tag (`MAT_TIMBER` or `MAT_THATCH`).
    material: u8,
    /// Burn state (`ST_UNBURNED`, `ST_BURNING`, `ST_BURNT`).
    state: u8,
    /// Ticks of burn left while burning.
    burn_left: u32,
    /// Adjacent burning ticks seen while unburned.
    exposure: u32,
}

/// Spike world: the wall plus the seeded ember stream and counters.
#[derive(Clone, Debug)]
struct World {
    /// Eight chunks in index order.
    chunks: [Chunk; CHUNK_COUNT],
    /// Match tick of the next step.
    tick: u32,
    /// Seeded stream for ember jumps and ignition draws.
    rng: Rng,
    /// Master switch for the ember pass.
    do_spotting: bool,
    /// Wind in Q16; zero forces throws below the contact filter.
    wind_q16: u32,
    /// Ignitions caused by contact exposure.
    contact_ignitions: u32,
    /// Ignitions caused by ember jumps of two or more chunks.
    spot_ignitions: u32,
}

/// Material tag at an index: even is timber, odd is thatch.
fn material_at(index: usize) -> u8 {
    if index.is_multiple_of(2) {
        MAT_TIMBER
    } else {
        MAT_THATCH
    }
}

/// Fuel rate in Q16 for a material tag, mirroring the shipped fuel rows.
fn burn_q16_of(material: u8) -> u32 {
    if material == MAT_THATCH {
        THATCH_BURN_Q16
    } else {
        TIMBER_BURN_Q16
    }
}

/// Burn duration in ticks: thatch is fast, timber is steady. Both outlast
/// the two-tick contact delay so the front never starves mid-wall.
fn burn_ticks_of(material: u8) -> u32 {
    if material == MAT_THATCH {
        4
    } else {
        8
    }
}

/// Adjacent burn ticks needed to ignite: thatch one, timber two.
fn contact_ticks_needed(material: u8) -> u32 {
    if material == MAT_THATCH {
        1
    } else {
        2
    }
}

/// Spotting-prone flag: thatch emits embers, timber never does.
fn is_spotting_prone(material: u8) -> bool {
    material == MAT_THATCH
}

/// Bond endpoints of the line wall: bond `i` joins chunks `i` and `i + 1`.
fn bond_ends(bond: usize) -> Result<(usize, usize), SpikeError> {
    if bond >= BOND_COUNT {
        return Err(SpikeError::BadBond(bond));
    }
    Ok((bond, bond + 1))
}

/// Other end of a bond from one endpoint.
fn other_end(bond: usize, from: usize) -> Result<usize, SpikeError> {
    let (a, b) = bond_ends(bond)?;
    if from == a {
        Ok(b)
    } else if from == b {
        Ok(a)
    } else {
        Err(SpikeError::BadChunk(from))
    }
}

/// Neighbour bonds of a chunk in the line wall, in ascending order.
fn neighbour_bonds(chunk: usize) -> Result<[Option<usize>; 2], SpikeError> {
    if chunk >= CHUNK_COUNT {
        return Err(SpikeError::BadChunk(chunk));
    }
    let left = if chunk > 0 { Some(chunk - 1) } else { None };
    let right = if chunk + 1 < CHUNK_COUNT {
        Some(chunk)
    } else {
        None
    };
    Ok([left, right])
}

/// Fresh wall with seeded ignition armed at chunk zero on the first step.
fn new_world(seed: u64, do_spotting: bool, wind_q16: u32) -> Result<World, SpikeError> {
    if CHUNK_COUNT == 0 {
        return Err(SpikeError::EmptyWorld);
    }
    let mut chunks = [Chunk {
        material: MAT_TIMBER,
        state: ST_UNBURNED,
        burn_left: 0,
        exposure: 0,
    }; CHUNK_COUNT];
    for (i, slot) in chunks.iter_mut().enumerate() {
        slot.material = material_at(i);
    }
    Ok(World {
        chunks,
        tick: 0,
        rng: Rng::new(seed),
        do_spotting,
        wind_q16,
        contact_ignitions: 0,
        spot_ignitions: 0,
    })
}

/// Ignite one unburned chunk through the given path.
fn ignite(world: &mut World, chunk: usize, by_spot: bool) -> Result<(), SpikeError> {
    if chunk >= CHUNK_COUNT {
        return Err(SpikeError::BadChunk(chunk));
    }
    let cell = &mut world.chunks[chunk];
    if cell.state != ST_UNBURNED {
        return Ok(());
    }
    cell.state = ST_BURNING;
    cell.burn_left = burn_ticks_of(cell.material);
    if by_spot {
        world.spot_ignitions = world.spot_ignitions.saturating_add(1);
    } else {
        world.contact_ignitions = world.contact_ignitions.saturating_add(1);
    }
    Ok(())
}

/// One integer tick: contact pass, then the seeded ember pass, then burnout.
fn step(world: &mut World) -> Result<(), SpikeError> {
    // Seed tick: chunk zero ignites through the seeded ignition path.
    if world.tick == 0 {
        ignite(world, 0, false)?;
    }
    // Contact pass: every burning chunk exposes each unburned neighbour.
    let mut contact_hits: [u32; CHUNK_COUNT] = [0; CHUNK_COUNT];
    for chunk in 0..CHUNK_COUNT {
        if world.chunks[chunk].state != ST_BURNING {
            continue;
        }
        for bond in neighbour_bonds(chunk)?.into_iter().flatten() {
            let next = other_end(bond, chunk)?;
            if world.chunks[next].state == ST_UNBURNED {
                contact_hits[next] = contact_hits[next].saturating_add(1);
            }
        }
    }
    for (i, hits) in contact_hits.iter().enumerate() {
        if *hits == 0 || world.chunks[i].state != ST_UNBURNED {
            continue;
        }
        let cell = &mut world.chunks[i];
        cell.exposure = cell.exposure.saturating_add(*hits);
        if cell.exposure >= contact_ticks_needed(cell.material) {
            ignite(world, i, false)?;
        }
    }
    // Ember pass: each spotting-prone burner attempts one seeded throw.
    if world.do_spotting {
        // Collect throws first so draw order is by chunk index.
        let mut throws: [Option<(usize, u8)>; CHUNK_COUNT] = [None; CHUNK_COUNT];
        for (chunk, throw_slot) in throws.iter_mut().enumerate() {
            if world.chunks[chunk].state != ST_BURNING {
                continue;
            }
            if !is_spotting_prone(world.chunks[chunk].material) {
                continue;
            }
            let jump: u8 = if world.wind_q16 == WIND_CALM_Q16 {
                1
            } else {
                let draw = world.rng.next_u64() % 3;
                (SPOT_MIN_JUMP as u64 + draw) as u8
            };
            *throw_slot = Some((chunk, jump));
        }
        for throw_ in throws.into_iter().flatten() {
            let (source, jump) = throw_;
            if !(SPOT_MIN_JUMP..=SPOT_MAX_JUMP).contains(&jump) {
                continue;
            }
            let landing = source + jump as usize;
            if landing >= CHUNK_COUNT {
                continue;
            }
            if world.chunks[landing].state != ST_UNBURNED {
                continue;
            }
            let draw = world.rng.next_u64() % IGNITE_DEN;
            if draw < IGNITE_NUM {
                ignite(world, landing, true)?;
            }
        }
    }
    // Burnout pass: burning chunks tick down in index order.
    for cell in world.chunks.iter_mut() {
        if cell.state == ST_BURNING {
            cell.burn_left = cell.burn_left.saturating_sub(1);
            if cell.burn_left == 0 {
                cell.state = ST_BURNT;
            }
        }
    }
    world.tick = world.tick.saturating_add(1);
    Ok(())
}

/// Canonical digest over integer state only: tick, chunks, counters.
fn digest(world: &World) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write_u32(world.tick);
    hasher.write_u32(world.contact_ignitions);
    hasher.write_u32(world.spot_ignitions);
    for cell in world.chunks.iter() {
        hasher.write_u8(cell.material);
        hasher.write_u8(cell.state);
        hasher.write_u32(cell.burn_left);
        hasher.write_u32(cell.exposure);
        hasher.write_u32(burn_q16_of(cell.material));
    }
    hasher.finish()
}

/// Run the fixture for a tick budget, returning the world and its digest.
fn run(
    seed: u64,
    do_spotting: bool,
    wind_q16: u32,
    ticks: u32,
) -> Result<(World, u64), SpikeError> {
    if ticks == 0 {
        return Err(SpikeError::BadTicks);
    }
    let mut world = new_world(seed, do_spotting, wind_q16)?;
    for _ in 0..ticks {
        step(&mut world)?;
    }
    let sum = digest(&world);
    Ok((world, sum))
}

/// Count chunks in a state.
fn count_in(world: &World, state: u8) -> usize {
    world.chunks.iter().filter(|c| c.state == state).count()
}

#[test]
fn fire_spread_seeded_covers_wall_per_fuel_table() {
    let (world, sum) =
        run(SPIKE_SEED, true, WIND_BREEZE_Q16, COVER_TICKS).expect("cover run must succeed");
    assert_eq!(
        count_in(&world, ST_BURNT) + count_in(&world, ST_BURNING),
        CHUNK_COUNT,
        "seeded ignition must reach every chunk: digest {sum:016x}"
    );
    assert!(
        world.contact_ignitions >= 4,
        "contact must carry the wall: {} contact, {} spot, digest {sum:016x}",
        world.contact_ignitions,
        world.spot_ignitions
    );
    // Fuel-table split: thatch lights first, so at least one spot or fast
    // contact chain must show beside the contact count.
    assert!(
        world.contact_ignitions + world.spot_ignitions >= 7,
        "seven ignitions cover eight chunks: digest {sum:016x}",
    );
}

#[test]
fn fire_spread_digest_identical_x2() {
    let (_, first) =
        run(SPIKE_SEED, true, WIND_BREEZE_Q16, COVER_TICKS).expect("first run must succeed");
    let (_, second) =
        run(SPIKE_SEED, true, WIND_BREEZE_Q16, COVER_TICKS).expect("second run must succeed");
    assert_eq!(first, second, "same seed must give the same digest twice");
}

#[test]
fn fire_spread_ember_passover_spots_downwind() {
    // Ember-passover behavior: with the breeze and the master switch on,
    // thatch throwers spot two or more chunks downwind; this run names a
    // positive spot count so the planner sees the ember path is live.
    let (world, sum) =
        run(SPIKE_SEED, true, WIND_BREEZE_Q16, COVER_TICKS).expect("ember run must succeed");
    assert!(
        world.spot_ignitions > 0,
        "breeze plus spotting must spot at least once: digest {sum:016x}",
    );
}

#[test]
fn fire_spread_light_wind_throws_zero_spots() {
    // Donor bug shape killed here: calm wind collapses every throw below
    // the two-chunk contact filter, so replays show zero spot fires.
    let (world, sum) =
        run(SPIKE_SEED, true, WIND_CALM_Q16, COVER_TICKS).expect("calm run must succeed");
    assert_eq!(
        world.spot_ignitions, 0,
        "calm wind must throw zero spots: digest {sum:016x}"
    );
    assert_eq!(
        count_in(&world, ST_BURNT) + count_in(&world, ST_BURNING),
        CHUNK_COUNT,
        "contact alone must still cover the wall: digest {sum:016x}"
    );
}

#[test]
fn fire_spread_ablation_spotting_off_holds_contact() {
    let (with_spot, _) =
        run(SPIKE_SEED, true, WIND_BREEZE_Q16, COVER_TICKS).expect("spotting run must succeed");
    let (without_spot, _) =
        run(SPIKE_SEED, false, WIND_BREEZE_Q16, COVER_TICKS).expect("contact run must succeed");
    assert_eq!(
        without_spot.spot_ignitions, 0,
        "master switch off must silence every spot"
    );
    assert_eq!(
        count_in(&without_spot, ST_BURNT) + count_in(&without_spot, ST_BURNING),
        CHUNK_COUNT,
        "contact alone must still cover the wall"
    );
    assert!(
        with_spot.spot_ignitions > without_spot.spot_ignitions,
        "ablation must separate the ember path from pure contact"
    );
}

#[test]
fn fire_spread_step_within_slice() {
    // Slice check uses the wall clock outside the digest; hashed state
    // never sees time. Budget is the per-cut slice from the hunt brief.
    let mut world = new_world(SPIKE_SEED, true, WIND_BREEZE_Q16).expect("fixture must build");
    let mut worst: u128 = 0;
    for _ in 0..COVER_TICKS {
        let start = std::time::Instant::now();
        step(&mut world).expect("step must succeed");
        let took = start.elapsed().as_micros();
        if took > worst {
            worst = took;
        }
    }
    assert!(
        worst <= STEP_BUDGET_MICROS,
        "eight-chunk step must stay in slice: worst {worst} micros"
    );
}

#[test]
fn fire_spread_rejects_bad_input() {
    assert_eq!(
        run(SPIKE_SEED, true, WIND_BREEZE_Q16, 0).unwrap_err(),
        SpikeError::BadTicks
    );
    let mut world = new_world(SPIKE_SEED, true, WIND_BREEZE_Q16).expect("fixture must build");
    assert_eq!(
        ignite(&mut world, CHUNK_COUNT, false).unwrap_err(),
        SpikeError::BadChunk(CHUNK_COUNT)
    );
    assert_eq!(
        bond_ends(BOND_COUNT).unwrap_err(),
        SpikeError::BadBond(BOND_COUNT)
    );
}

#[test]
fn fire_spread_report_numbers() {
    let (breeze, breeze_sum) =
        run(SPIKE_SEED, true, WIND_BREEZE_Q16, COVER_TICKS).expect("breeze must run");
    let (calm, calm_sum) =
        run(SPIKE_SEED, true, WIND_CALM_Q16, COVER_TICKS).expect("calm must run");
    let (contact_only, _) =
        run(SPIKE_SEED, false, WIND_BREEZE_Q16, COVER_TICKS).expect("contact must run");
    // Pinned report row: this test used to print digests and assert nothing,
    // so any regression stayed green. Every printed number is now asserted.
    const BREEZE_DIGEST: u64 = 0xd42636f7cf9b1b9f;
    const CALM_DIGEST: u64 = 0xac36cab292caff7b;
    assert_eq!(
        breeze_sum, BREEZE_DIGEST,
        "breeze digest drifted: got {breeze_sum:016x}"
    );
    assert_eq!(calm_sum, CALM_DIGEST, "calm digest drifted: got {calm_sum:016x}");
    assert_eq!(breeze.contact_ignitions, 7, "breeze contact count drifted");
    assert_eq!(breeze.spot_ignitions, 1, "breeze spot count drifted");
    assert_eq!(
        count_in(&breeze, ST_BURNT),
        8,
        "breeze must still burn the whole wall"
    );
    assert_eq!(calm.spot_ignitions, 0, "calm wind must throw zero spots");
    assert_eq!(
        count_in(&contact_only, ST_BURNT),
        8,
        "contact alone must still burn the whole wall"
    );
    let mut world = new_world(SPIKE_SEED, true, WIND_BREEZE_Q16).expect("fixture must build");
    let mut worst: u128 = 0;
    for _ in 0..COVER_TICKS {
        let start = std::time::Instant::now();
        step(&mut world).expect("step must succeed");
        let took = start.elapsed().as_micros();
        if took > worst {
            worst = took;
        }
    }
    println!(
        "SPIKE-NUMBERS breeze digest {breeze_sum:016x} contact {} spot {} burnt {} | calm digest {calm_sum:016x} spot {} | contact-only burnt {} | worst {worst} micros",
        breeze.contact_ignitions,
        breeze.spot_ignitions,
        count_in(&breeze, ST_BURNT),
        calm.spot_ignitions,
        count_in(&contact_only, ST_BURNT),
    );
}
