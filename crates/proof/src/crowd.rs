//! crowd: the 20,000-agent crowd determinism harness.
//!
//! Consumer: `cargo test -p engine-proof --lib crowd` on every runner and PC;
//! two machines that print the same [`CrowdReport::hash`] agree on the whole
//! crowd. Data owner: this module owns the fixture and the report; the
//! physics crate owns bodies, islands and the sleep rules it calls. Units:
//! ticks (`u32`), metres and seconds in `f32` physics bodies. Determinism:
//! slot order everywhere, one seeded [`Rng`], fixed [`FIXED_DT`], plain `f32`
//! add and multiply only (no libm), so the hash is the same on x86 and Arm.
//! Errors: [`Error::Invalid`] for a bad shape or a missed wake; nothing
//! panics.
//!
//! The crowd is `agents` far grounded capsules in rows of [`ROW`], every
//! neighbour in a row in contact, so each row is one sleep island. It falls
//! asleep in one tick; then a seeded fresh row is shoved every
//! [`SHOVE_EVERY`] ticks from tick [`SHOVE_FIRST`]. A shove must wake its
//! whole row the same tick, and no body at or above the wake speed may ever
//! sleep. Shoves, wakes, the sleeper count and a digest every
//! [`CHECKPOINT_EVERY`] ticks go into the report.

use engine_core::{Fnv64, Rng};
use engine_physics::islands::{build_islands, update_sleep};
use engine_physics::lod::{mark_mixed_sleep, LodConfig, Tier};
use engine_physics::rigid::{state_hash, step_body, FIXED_DT};
use engine_physics::Body;

use crate::Error;

/// Agents in the canonical crowd.
pub const AGENTS: usize = 20_000;
/// Ticks in the canonical tape.
pub const TICKS: u32 = 600;
/// Agents per row; a row is one island.
pub const ROW: usize = 10;
/// Tick of the first shove.
pub const SHOVE_FIRST: u32 = 10;
/// Ticks between shoves.
pub const SHOVE_EVERY: u32 = 20;
/// Ticks between digests in [`CrowdReport::checkpoints`].
pub const CHECKPOINT_EVERY: u32 = 100;
/// Most agents one crowd holds: positions are exact in `f32` metres below it.
pub const MAX_AGENTS: usize = 60_000;

/// What one crowd tape did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrowdReport {
    /// Agents in the crowd.
    pub agents: usize,
    /// Ticks run.
    pub ticks: u32,
    /// Shoves delivered.
    pub shoves: u32,
    /// Shoves that woke their whole row on the tick they landed.
    pub woke: u32,
    /// Agents asleep after the last tick.
    pub asleep: usize,
    /// Digest after every [`CHECKPOINT_EVERY`] ticks; the first unequal
    /// entry names the block where two machines diverged.
    pub checkpoints: Vec<u64>,
    /// One hash over the checkpoints and the final state.
    pub hash: u64,
}

/// Far grounded capsules, one per slot, shoulder to shoulder along x.
fn bodies(agents: usize) -> Result<Vec<Body>, Error> {
    (0..agents)
        .map(|i| {
            let x = u16::try_from(i).map_err(|_| Error::Invalid("crowd agents exceed 65,535"))?;
            Ok(Body::new(f32::from(x), 0.5, 0.0, 0.0, 0.5, 0.5))
        })
        .collect()
}

/// Run the crowd tape: `agents` (a multiple of [`ROW`]) for `ticks` ticks,
/// shoves picked by `seed`.
///
/// # Errors
///
/// [`Error::Invalid`] for a shape that is not a whole number of rows, for
/// more than [`MAX_AGENTS`], for a shove that lands on a row that is not
/// asleep, and for any body that sleeps at or above the wake speed.
pub fn run(agents: usize, ticks: u32, seed: u64) -> Result<CrowdReport, Error> {
    if agents == 0 || !agents.is_multiple_of(ROW) || agents > MAX_AGENTS {
        return Err(Error::Invalid(
            "crowd agents must be whole rows up to 60,000",
        ));
    }
    let cfg = LodConfig::default();
    let mut crowd = bodies(agents)?;
    let pairs: Vec<(usize, usize)> = (0..agents)
        .filter(|i| i % ROW != ROW - 1)
        .map(|i| (i, i + 1))
        .collect();
    let islands = build_islands(agents, &pairs);
    let tiers = vec![Tier::Capsule; agents];
    let (mut timers, mut asleep) = (vec![0u32; agents], vec![false; agents]);
    let mut v2 = vec![0.0f32; agents];
    let mut rng = Rng::new(seed);
    let mut used = vec![false; agents / ROW];
    let (mut shoves, mut woke) = (0u32, 0u32);
    let mut checkpoints = Vec::new();
    let span = u64::try_from(agents).map_err(|_| Error::Invalid("crowd size"))?;
    for tick in 0..ticks {
        let mut shoved = None;
        if tick >= SHOVE_FIRST && (tick - SHOVE_FIRST).is_multiple_of(SHOVE_EVERY) {
            // A fresh row each time, so the row is asleep when the shove lands.
            for _ in 0..64 {
                let k = usize::try_from(rng.next_u64() % span).unwrap_or(0);
                if !used[k / ROW] {
                    used[k / ROW] = true;
                    shoved = Some(k);
                    break;
                }
            }
        }
        if let Some(k) = shoved {
            if !asleep[k] {
                return Err(Error::Invalid("crowd row was not asleep before its shove"));
            }
            crowd[k].vx = 3.0;
            shoves += 1;
        }
        for (v, b) in v2.iter_mut().zip(&crowd) {
            *v = b.vx * b.vx + b.vy * b.vy;
        }
        mark_mixed_sleep(&crowd, &tiers, &mut asleep, &mut timers, &cfg)
            .map_err(|_| Error::Invalid("crowd sleep arrays disagree"))?;
        update_sleep(agents, &islands, &v2, &mut timers, &mut asleep, cfg.sleep);
        for (b, &sleeping) in crowd.iter_mut().zip(&asleep) {
            if !sleeping {
                step_body(b, FIXED_DT);
            }
        }
        if v2
            .iter()
            .zip(&asleep)
            .any(|(&v, &sleeping)| sleeping && v >= cfg.sleep.wake_v2)
        {
            return Err(Error::Invalid("crowd body sleeps above the wake speed"));
        }
        if let Some(k) = shoved {
            let row = k / ROW * ROW;
            woke += u32::from(asleep[row..row + ROW].iter().all(|&sleeping| !sleeping));
        }
        if (tick + 1).is_multiple_of(CHECKPOINT_EVERY) {
            checkpoints.push(digest(&crowd, &asleep, tick));
        }
    }
    let asleep_count = asleep.iter().filter(|&&sleeping| sleeping).count();
    let mut hash = Fnv64::new();
    for checkpoint in &checkpoints {
        hash.write_u64(*checkpoint);
    }
    hash.write_u64(digest(&crowd, &asleep, ticks));
    Ok(CrowdReport {
        agents,
        ticks,
        shoves,
        woke,
        asleep: asleep_count,
        checkpoints,
        hash: hash.finish(),
    })
}

/// Digest of the crowd at `tick`: every body plus the sleeper count.
fn digest(crowd: &[Body], asleep: &[bool], tick: u32) -> u64 {
    let sleepers = asleep.iter().filter(|&&sleeping| sleeping).count();
    let mut hash = Fnv64::new();
    hash.write_u64(u64::from(tick));
    hash.write_u64(state_hash(crowd));
    hash.write_u64(u64::try_from(sleepers).unwrap_or(0));
    hash.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pinned on the Linux runner and re-checked on the Windows PC: the
    /// canonical tape hashes the same on both.
    const CROWD_20K_600: u64 = 0x9627_a111_9822_c2e0;

    #[test]
    fn crowd_20k_600_tape_hashes_x2_never_misses_a_wake_and_matches_the_pin() {
        let first = run(AGENTS, TICKS, 2_040).expect("canonical crowd runs");
        let second = run(AGENTS, TICKS, 2_040).expect("canonical crowd runs");
        assert_eq!(first, second, "same seed, same tape");
        assert_eq!(first.agents, 20_000);
        assert_eq!(first.shoves, 30);
        assert_eq!(first.woke, first.shoves, "wake recall must be 100%");
        // At most the 30 shoved rows stay awake; every other row sleeps through it all.
        assert!(
            first.asleep >= AGENTS - 30 * ROW,
            "only {} asleep",
            first.asleep
        );
        assert_eq!(first.checkpoints.len(), 6);
        assert_eq!(first.hash, CROWD_20K_600, "pinned hash moved");
    }

    #[test]
    fn crowd_another_seed_shoves_other_agents() {
        let a = run(AGENTS, TICKS, 2_040).expect("runs");
        let b = run(AGENTS, TICKS, 9_001).expect("runs");
        assert_ne!(a.hash, b.hash);
        assert_ne!(a.checkpoints, b.checkpoints);
    }

    #[test]
    fn crowd_refuses_bad_shapes() {
        for agents in [0, 15, ROW * 7 + 1, MAX_AGENTS + ROW] {
            assert!(run(agents, 10, 1).is_err(), "{agents} agents");
        }
        // Too few rows to find a fresh one: the shove is skipped, not looped on.
        let tiny = run(ROW, TICKS, 1).expect("one row");
        assert!(tiny.shoves <= 1);
    }
}
