//! limbs: severed limbs from cut events.
//!
//! Consumer: the arena match step hands the cuts of one tick to
//! [`spawn_limbs`]; the stump servo reads the recoil in each report and the
//! debris ledger frees what `debris_to_free` asks for. Data owner:
//! `engine-physics` owns [`LimbPool`] and the limb body; `engine-combat` owns
//! [`Cut`]; this module owns only the joint-to-volume table and the
//! translation. Units: volume in cm^3, density in kg/m^3, origin in mm,
//! impulse in milli-newton-seconds. Determinism: one limb per cut, spawned
//! through `LimbPool::spawn_batch` in canonical (swing, victim, joint) order,
//! so any input order gives the same pool bytes; integer only. Errors:
//! [`LimbError`]; a bad batch refuses whole and the pool stays unchanged.

use engine_combat::Cut;
use engine_destruction::bonded::BondedCollection;
use engine_destruction::budget::DebrisLedger;
use engine_destruction::snapshot::{self, SnapshotError};
use engine_physics::limb::{LimbError, LimbPool, LimbSpawn, SpawnReport};

/// Volume of the chain that leaves the body when a joint is cut, in cm^3,
/// in joint wire order (adult 75 kg, tissue near water). The pelvis root
/// never detaches, so its slot is zero.
pub const LIMB_VOLUME_CM3: [u32; 16] = [
    0,      // Pelvis
    35_000, // Waist: everything above the belt
    21_000, // Chest: head, arms and upper torso
    6_100,  // Neck: the head
    3_750,  // ShoulderL: whole arm
    3_750,  // ShoulderR
    1_650,  // ElbowL: forearm and hand
    1_650,  // ElbowR
    450,    // WristL: hand
    450,    // WristR
    12_075, // HipL: whole leg
    12_075, // HipR
    4_575,  // KneeL: shank and foot
    4_575,  // KneeR
    1_090,  // AnkleL: foot
    1_090,  // AnkleR
];

/// Tissue density in kg/m^3 (water).
const LIMB_DENSITY_KG_M3: u32 = 1000;
/// Contact radius against the ground in mm.
const LIMB_RADIUS_MM: u32 = 60;
/// Where a limb leaves the body until the fighter capsules feed real
/// positions, in mm.
const LIMB_ORIGIN_MM: [i32; 3] = [0, 1000, 0];
/// Blade impulse into the limb until the swing feeds a real one, in mN*s.
const LIMB_IMPULSE_MNS: [i32; 3] = [20_000, 5_000, 0];

/// The limb one cut detaches.
///
/// Errors: [`LimbError::BadSpawn`] for the pelvis root or a joint past the
/// table: no resolver output names either, so this is a corrupt event.
fn limb_of(cut: &Cut) -> Result<LimbSpawn, LimbError> {
    let volume_cm3 = match LIMB_VOLUME_CM3.get(usize::from(cut.joint)) {
        Some(&volume) if cut.joint != 0 => volume,
        _ => return Err(LimbError::BadSpawn),
    };
    Ok(LimbSpawn {
        swing: cut.swing,
        victim: cut.victim,
        joint: cut.joint,
        origin_mm: LIMB_ORIGIN_MM,
        parent_vel_mmps: [0; 3],
        impulse_mns: LIMB_IMPULSE_MNS,
        volume_cm3,
        density_kg_m3: LIMB_DENSITY_KG_M3,
        radius_mm: LIMB_RADIUS_MM,
    })
}

/// Spawn one limb per cut into `pool`, in canonical (swing, victim, joint)
/// order whatever the order of `cuts`, and return one report per limb in that
/// order. `debris_live` is the debris the ledger holds against the shared
/// body cap; the reports say how many debris bodies to free.
///
/// Errors: [`LimbError::BadSpawn`] for a pelvis or unknown joint, or the same
/// (swing, victim, joint) cut twice (one joint detaches once); both refuse
/// before anything spawns, so the pool is unchanged.
pub fn spawn_limbs(
    pool: &mut LimbPool,
    cuts: &[Cut],
    debris_live: usize,
) -> Result<Vec<SpawnReport>, LimbError> {
    let mut batch = Vec::with_capacity(cuts.len());
    for cut in cuts {
        batch.push(limb_of(cut)?);
    }
    batch.sort_unstable_by_key(|spawn| (spawn.swing, spawn.victim, spawn.joint));
    let key = |spawn: &LimbSpawn| (spawn.swing, spawn.victim, spawn.joint);
    if batch.windows(2).any(|pair| key(&pair[0]) == key(&pair[1])) {
        return Err(LimbError::BadSpawn);
    }
    pool.spawn_batch(&mut batch, debris_live)
}

/// Why a combined restore was refused. On every error nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimbsSnapshotError {
    /// The bytes are truncated, mis-sized or carry trailing data.
    Bad,
    /// The destruction part was refused.
    World(SnapshotError),
    /// The limb part was refused.
    Limbs(LimbError),
}

impl core::fmt::Display for LimbsSnapshotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Bad => write!(f, "limbs snapshot bytes are not valid"),
            Self::World(error) => write!(f, "limbs snapshot world part refused: {error}"),
            Self::Limbs(error) => write!(f, "limbs snapshot limb part refused: {error}"),
        }
    }
}

impl std::error::Error for LimbsSnapshotError {}

/// One rollback snapshot of everything a break touches: the destruction
/// bytes (`engine_destruction::snapshot::capture`), then the limb pool's
/// bytes, each part prefixed with its length as a little-endian `u64`.
/// Equal states give equal bytes on every PC.
#[must_use]
pub fn capture_with_limbs(
    collections: &[&BondedCollection],
    ledger: &DebrisLedger,
    pool: &LimbPool,
) -> Vec<u8> {
    let world = snapshot::capture(collections, ledger);
    let limbs = pool.snapshot();
    let mut out = Vec::with_capacity(16 + world.len() + limbs.len());
    for part in [&world, &limbs] {
        out.extend_from_slice(&u64::try_from(part.len()).unwrap_or(u64::MAX).to_le_bytes());
        out.extend_from_slice(part);
    }
    out
}

/// One length-prefixed part and the bytes after it.
fn take_part(bytes: &[u8]) -> Result<(&[u8], &[u8]), LimbsSnapshotError> {
    let (head, rest) = bytes
        .split_first_chunk::<8>()
        .ok_or(LimbsSnapshotError::Bad)?;
    let len = usize::try_from(u64::from_le_bytes(*head)).map_err(|_| LimbsSnapshotError::Bad)?;
    if len > rest.len() {
        return Err(LimbsSnapshotError::Bad);
    }
    Ok(rest.split_at(len))
}

/// Roll the collections, the debris ledger and the limb pool back to `bytes`
/// from [`capture_with_limbs`]. Atomic: both parts are parsed and rebuilt
/// before anything live is replaced, so a refused snapshot changes nothing.
///
/// Errors: [`LimbsSnapshotError`] for truncated, mis-sized or trailing bytes,
/// a refused destruction part (wrong collection count, bad bytes) or a
/// refused limb part.
pub fn restore_with_limbs(
    collections: &mut [&mut BondedCollection],
    ledger: &mut DebrisLedger,
    pool: &mut LimbPool,
    bytes: &[u8],
) -> Result<(), LimbsSnapshotError> {
    let (world, rest) = take_part(bytes)?;
    let (limbs, rest) = take_part(rest)?;
    if !rest.is_empty() {
        return Err(LimbsSnapshotError::Bad);
    }
    // The pool is rebuilt first and swapped last; the world restore is itself
    // atomic and the only step between, so no failure leaves half a rollback.
    let rebuilt = LimbPool::restore(limbs).map_err(LimbsSnapshotError::Limbs)?;
    snapshot::restore(collections, ledger, world).map_err(LimbsSnapshotError::World)?;
    *pool = rebuilt;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_combat::{
        resolve_swing, AnatomyTable, BladeSwing, JointId, JointTarget, CUT_V1, MAX_CUTS_PER_SWING,
    };
    use engine_core::{CounterCursor, CounterRng};
    use engine_destruction::bonded::BreakEvent;
    use engine_destruction::budget::{arena_default_budget, ShardSeed};
    use engine_destruction::structure::{analyze, building, BuildingSpec};

    fn cut(swing: u32, victim: u8, joint: u8) -> Cut {
        Cut {
            swing,
            attacker: victim.wrapping_add(1),
            victim,
            joint,
            flags: 0,
            energy_q16: 600_000,
            protection_q16: 589_824,
        }
    }

    /// 20 seeded cuts: swings 0..20, joints 1..=15, victims 0..8.
    fn seeded_cuts() -> Vec<Cut> {
        let mut draw = CounterCursor::from_rng(CounterRng::new(2040).fork(0xB37), 0);
        (0..20u32)
            .map(|swing| {
                let joint = u8::try_from(1 + draw.next_u64() % 15).unwrap_or(1);
                let victim = u8::try_from(draw.next_u64() % 8).unwrap_or(0);
                cut(swing, victim, joint)
            })
            .collect()
    }

    fn shuffled(cuts: &[Cut], seed: u64) -> Vec<Cut> {
        let mut out = cuts.to_vec();
        let mut draw = CounterCursor::from_rng(CounterRng::new(seed), 0);
        for top in (1..out.len()).rev() {
            let pick = draw.next_u64() % (u64::try_from(top).unwrap_or(0) + 1);
            out.swap(top, usize::try_from(pick).unwrap_or(0));
        }
        out
    }

    fn pool_after(cuts: &[Cut]) -> (LimbPool, Vec<SpawnReport>) {
        let mut pool = LimbPool::new(64).expect("pool");
        let reports = spawn_limbs(&mut pool, cuts, 0).expect("spawns");
        (pool, reports)
    }

    #[test]
    fn limbs_twenty_seeded_cuts_spawn_twenty_limbs_in_any_order_and_twice() {
        let cuts = seeded_cuts();
        let (pool, reports) = pool_after(&cuts);
        assert_eq!(pool.len(), 20);
        assert_eq!(reports.len(), 20);
        let hash = pool.state_hash();
        assert_eq!(pool_after(&cuts).0.state_hash(), hash, "run twice");
        for seed in [1, 2, 3, 99] {
            let (other, other_reports) = pool_after(&shuffled(&cuts, seed));
            assert_ne!(shuffled(&cuts, seed), cuts, "shuffle {seed} moved nothing");
            assert_eq!(other.state_hash(), hash, "shuffle {seed} changed the pool");
            assert_eq!(other_reports, reports);
        }
        // Limbs carry the cut's identity in canonical order.
        let mut want: Vec<(u32, u8, u8)> =
            cuts.iter().map(|c| (c.swing, c.victim, c.joint)).collect();
        want.sort_unstable();
        let got: Vec<(u8, u8)> = pool.limbs().iter().map(|l| (l.victim, l.joint)).collect();
        let want_pairs: Vec<(u8, u8)> = want.iter().map(|&(_, v, j)| (v, j)).collect();
        assert_eq!(got, want_pairs);
    }

    #[test]
    fn limbs_hash_sees_a_changed_cut() {
        let cuts = seeded_cuts();
        let base = pool_after(&cuts).0.state_hash();
        let mut other_joint = cuts.clone();
        other_joint[4].joint = if other_joint[4].joint == 1 { 2 } else { 1 };
        assert_ne!(pool_after(&other_joint).0.state_hash(), base);
        let mut other_victim = cuts.clone();
        other_victim[7].victim ^= 1;
        assert_ne!(pool_after(&other_victim).0.state_hash(), base);
        assert_ne!(pool_after(&cuts[..19]).0.state_hash(), base);
    }

    #[test]
    fn limbs_mass_follows_the_joint_and_the_chain_shrinks_toward_the_hand() {
        let mut pool = LimbPool::new(16).expect("pool");
        let cuts: Vec<Cut> = (1..=15u8)
            .map(|joint| cut(u32::from(joint), 0, joint))
            .collect();
        spawn_limbs(&mut pool, &cuts, 0).expect("spawns");
        for (limb, joint) in pool.limbs().iter().zip(1..=15usize) {
            assert_eq!(limb.mass_g, LIMB_VOLUME_CM3[joint], "joint {joint}");
            assert_eq!(limb.radius_mm, 60);
        }
        // A distal chain is always lighter than the chain it hangs from.
        let anatomy = AnatomyTable::rome_kit();
        for joint in JointId::all().iter().filter(|j| j.is_severable()) {
            let parent = usize::from(anatomy.parents[joint.index()]);
            if parent != 0 {
                assert!(
                    LIMB_VOLUME_CM3[joint.index()] < LIMB_VOLUME_CM3[parent],
                    "joint {} outweighs its parent {parent}",
                    joint.index()
                );
            }
            assert!(LIMB_VOLUME_CM3[joint.index()] > 0);
        }
        assert_eq!(LIMB_VOLUME_CM3[0], 0);
    }

    #[test]
    fn limbs_debris_at_the_cap_asks_the_ledger_for_one_body_per_limb() {
        let cuts = seeded_cuts();
        let mut pool = LimbPool::new(64).expect("pool");
        let cap = pool.cap();
        let reports = spawn_limbs(&mut pool, &cuts, cap).expect("spawns");
        assert_eq!(reports.len(), 20);
        assert!(reports.iter().all(|r| r.debris_to_free == 1));
        // Limbs outrank debris: the pool keeps the newest limb, older ones give way.
        assert_eq!(pool.len(), 1);
        assert_eq!(reports.iter().map(|r| r.evicted_limbs).sum::<u32>(), 19);
        // Room under the cap asks for nothing.
        let (_, roomy) = pool_after(&cuts);
        assert!(roomy
            .iter()
            .all(|r| r.debris_to_free == 0 && r.evicted_limbs == 0));
    }

    #[test]
    fn limbs_recoil_is_equal_and_opposite_and_names_the_victim() {
        let (_, reports) = pool_after(&[cut(3, 5, 7)]);
        let recoil = reports[0].recoil;
        assert_eq!((recoil.victim, recoil.joint), (5, 7));
        assert_eq!(recoil.impulse_mns, [-20_000, -5_000, 0]);
    }

    #[test]
    fn limbs_refuse_pelvis_unknown_joint_and_repeats_with_the_pool_unchanged() {
        let mut pool = LimbPool::new(8).expect("pool");
        let before = pool.state_hash();
        for bad in [
            vec![cut(1, 0, 3), cut(2, 0, 0)],
            vec![cut(1, 0, 3), cut(2, 0, 16)],
            vec![cut(1, 0, 3), cut(2, 0, 255)],
            vec![cut(1, 0, 3), cut(1, 0, 3)],
        ] {
            assert_eq!(spawn_limbs(&mut pool, &bad, 0), Err(LimbError::BadSpawn));
            assert_eq!(pool.state_hash(), before, "a refused batch spawned a limb");
        }
        // The same swing may cut two joints, and two victims may lose the same joint.
        let both = [cut(1, 0, 3), cut(1, 0, 7), cut(1, 1, 3)];
        assert_eq!(spawn_limbs(&mut pool, &both, 0).expect("spawns").len(), 3);
        assert!(spawn_limbs(&mut pool, &[], 0)
            .expect("empty is fine")
            .is_empty());
    }

    #[test]
    fn limbs_come_from_the_real_resolver_output() {
        // A hard fresh swing through every severable capsule of one victim.
        let anatomy = AnatomyTable::rome_kit();
        let targets: Vec<JointTarget> = JointId::all()
            .iter()
            .filter(|j| j.is_severable())
            .map(|&joint| JointTarget {
                joint,
                q0_mm: [-20, -80, 0],
                q1_mm: [20, 80, 0],
                radius_mm: 60,
            })
            .collect();
        let mut made: Vec<Cut> = Vec::new();
        for swing in 0..8u32 {
            let blade = BladeSwing {
                attacker: 0,
                victim: 1,
                swing,
                p0_mm: [-100, 0, 0],
                p1_mm: [100, 0, 0],
                blade_half_mm: 10,
                energy_q16: 2_097_152,
                sharpness_q16: 131_072,
                wear_q16: 0,
            };
            let mut out = [cut(0, 0, 1); MAX_CUTS_PER_SWING];
            let count = resolve_swing(&blade, &targets, &anatomy, 0, &CUT_V1, &mut out);
            made.extend_from_slice(&out[..count]);
        }
        assert!(made.len() > 20, "the resolver cut {} joints", made.len());
        let (pool, reports) = pool_after(&made);
        assert_eq!(pool.len(), made.len().min(64));
        assert_eq!(reports.len(), made.len());
        let (again, _) = pool_after(&shuffled(&made, 5));
        assert_eq!(again.state_hash(), pool.state_hash());
    }

    const DT: f32 = 1.0 / 60.0;

    /// Everything one break touches: the collapsing building, the debris it
    /// sheds and the limbs the cuts detach.
    struct Break {
        field: BondedCollection,
        ledger: DebrisLedger,
        pool: LimbPool,
    }

    fn fresh_break() -> (Break, usize) {
        let built = building(&BuildingSpec::three_story(0)).expect("valid building");
        let mut field = BondedCollection::new(&built.desc).expect("collection builds");
        let capacity = field.max_load_ratio_q16() * 5 / 4;
        field.set_capacity_q16(capacity);
        let weakest = usize::try_from(analyze(&field).weakest.expect("stands")).expect("small");
        let ledger = DebrisLedger::new(&arena_default_budget()).expect("ledger");
        let pool = LimbPool::new(64).expect("pool");
        (
            Break {
                field,
                ledger,
                pool,
            },
            weakest,
        )
    }

    /// One match tick: the structural cut at tick 2, the collapse step, freed
    /// chunks become debris, then the limbs whose cut is due spawn and the
    /// pool steps.
    fn tick(state: &mut Break, weakest: usize, cuts: &[Cut], t: u32) {
        if t == 2 {
            state.field.sever_chunk(weakest, t).expect("in range");
        }
        state.field.step(DT);
        let mut events = Vec::new();
        state.field.drain_into(&mut events);
        let seeds: Vec<ShardSeed> = events
            .iter()
            .filter_map(|event| match event {
                BreakEvent::ChunkFreed { chunk, .. } => Some(ShardSeed {
                    id: *chunk,
                    island: chunk / 64,
                    pos_mm: [i32::try_from(*chunk).unwrap_or(0) * 7, 2000, 0],
                    vel_mm_per_tick: [3, 0, -2],
                }),
                BreakEvent::BondBroken { .. } => None,
            })
            .collect();
        state.ledger.admit(t, &seeds).expect("admits");
        state.ledger.step([0, 0, 0]);
        let due: Vec<Cut> = cuts.iter().copied().filter(|c| c.swing == t).collect();
        spawn_limbs(&mut state.pool, &due, 0).expect("spawns");
        state.pool.step();
    }

    fn bytes_of(state: &Break) -> Vec<u8> {
        capture_with_limbs(&[&state.field], &state.ledger, &state.pool)
    }

    fn rollback(state: &mut Break, bytes: &[u8]) -> Result<(), LimbsSnapshotError> {
        restore_with_limbs(
            &mut [&mut state.field],
            &mut state.ledger,
            &mut state.pool,
            bytes,
        )
    }

    /// Straight run of 90 ticks with a checkpoint after tick 5.
    fn straight_run() -> (Break, usize, Vec<u8>, Vec<u8>) {
        let cuts = seeded_cuts();
        let (mut state, weakest) = fresh_break();
        let mut checkpoint = Vec::new();
        for t in 0..90 {
            tick(&mut state, weakest, &cuts, t);
            if t == 5 {
                checkpoint = bytes_of(&state);
            }
        }
        let end = bytes_of(&state);
        (state, weakest, checkpoint, end)
    }

    #[test]
    fn limbs_snapshot_rollback_across_a_collapse_with_limbs_is_byte_equal() {
        let (mut state, weakest, checkpoint, straight) = straight_run();
        let (_, _, checkpoint_again, straight_again) = straight_run();
        assert_eq!(checkpoint, checkpoint_again, "checkpoint bytes x2");
        assert_eq!(straight, straight_again, "end bytes x2");
        assert_eq!(state.field.len(), 427, "the building under test");
        assert!(
            !state.field.freed_chunks().is_empty(),
            "the building collapsed"
        );
        assert_eq!(state.pool.len(), 20, "all 20 limbs alive at tick 90");

        rollback(&mut state, &checkpoint).expect("restores");
        assert_eq!(bytes_of(&state), checkpoint, "restore then capture");
        assert_eq!(
            state.pool.len(),
            6,
            "swings 0..=5 had spawned by the checkpoint"
        );
        let cuts = seeded_cuts();
        for t in 6..90 {
            tick(&mut state, weakest, &cuts, t);
        }
        assert_eq!(
            bytes_of(&state),
            straight,
            "resim from the checkpoint equals the run"
        );
        assert_ne!(checkpoint, straight);
    }

    #[test]
    fn limbs_snapshot_layout_is_two_length_prefixed_parts() {
        let (state, _, _, bytes) = straight_run();
        let world = snapshot::capture(&[&state.field], &state.ledger);
        let limbs = state.pool.snapshot();
        let mut want = Vec::new();
        want.extend_from_slice(&u64::try_from(world.len()).unwrap_or(0).to_le_bytes());
        want.extend_from_slice(&world);
        want.extend_from_slice(&u64::try_from(limbs.len()).unwrap_or(0).to_le_bytes());
        want.extend_from_slice(&limbs);
        assert_eq!(bytes, want);
        // A limb moves the bytes even when the building does not.
        let mut more = state.pool.clone();
        spawn_limbs(&mut more, &[cut(99, 3, 5)], 0).expect("spawns");
        let bigger = capture_with_limbs(&[&state.field], &state.ledger, &more);
        assert_ne!(bigger, bytes);
    }

    #[test]
    fn limbs_snapshot_refuses_bad_bytes_and_changes_nothing() {
        let (mut state, _, checkpoint, _) = straight_run();
        let before = bytes_of(&state);
        assert_ne!(before, checkpoint, "the live state is not the checkpoint");
        let world_len = usize::try_from(u64::from_le_bytes(checkpoint[..8].try_into().expect("8")))
            .expect("fits");
        let limb_at = 8 + world_len + 8;

        // Each refusal: the error, and the live state exactly as it was.
        let expect_refused = |state: &mut Break, what: &str, bytes: &[u8]| {
            let result = rollback(state, bytes);
            assert_eq!(bytes_of(state), before, "{what} changed the live state");
            result.expect_err(what)
        };
        for cut_at in [
            0,
            1,
            7,
            8,
            9,
            15,
            16,
            100,
            checkpoint.len() / 2,
            checkpoint.len() - 1,
        ] {
            expect_refused(&mut state, "truncated", &checkpoint[..cut_at]);
        }
        let mut trailing = checkpoint.clone();
        trailing.push(0);
        assert_eq!(
            expect_refused(&mut state, "trailing", &trailing),
            LimbsSnapshotError::Bad
        );
        let mut lying = checkpoint.clone();
        lying[..8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert_eq!(
            expect_refused(&mut state, "lying prefix", &lying),
            LimbsSnapshotError::Bad
        );
        // A bad part beside a good one: the good part must not be applied.
        let mut bad_world = checkpoint.clone();
        bad_world[8] = 9;
        assert_eq!(
            expect_refused(&mut state, "bad world", &bad_world),
            LimbsSnapshotError::World(SnapshotError::Bad)
        );
        let mut bad_limbs = checkpoint.clone();
        bad_limbs[limb_at] = 9;
        assert_eq!(
            expect_refused(&mut state, "bad limbs", &bad_limbs),
            LimbsSnapshotError::Limbs(LimbError::BadSnapshot)
        );
        // A wrong collection count refuses after the limb part parsed fine.
        let mut other = state.field.clone();
        let result = restore_with_limbs(
            &mut [&mut state.field, &mut other],
            &mut state.ledger,
            &mut state.pool,
            &checkpoint,
        );
        assert_eq!(
            result,
            Err(LimbsSnapshotError::World(SnapshotError::CollectionCount {
                have: 1,
                want: 2
            }))
        );
        assert_eq!(bytes_of(&state), before);
        // The good bytes still restore after all those refusals.
        rollback(&mut state, &checkpoint).expect("good bytes restore");
        assert_eq!(bytes_of(&state), checkpoint);
        assert!(LimbsSnapshotError::Bad.to_string().contains("not valid"));
    }
}
