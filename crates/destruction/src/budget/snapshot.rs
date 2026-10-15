//! snapshot: canonical bytes of the debris ledger and the restore that
//! rebuilds it exactly, so a net rollback resims a break to byte-equal
//! debris.
//!
//! Consumer: the world snapshot (`snapshot.rs`) and the net rollback; the
//! ledger never reads these bytes on its own. Data owner:
//! `engine-destruction`. Units: the ledger's own (mm, mm/tick, ticks).
//! Determinism scope: the bytes are a pure function of the ledger state in
//! canonical `(island, id)` order, integers only, so a restored ledger
//! hashes (`state_hash`) and steps exactly like the original. Errors:
//! [`BudgetError::BadSnapshot`] for truncated, versioned wrong, out of
//! range, misordered or trailing bytes; nothing panics and a refused
//! restore changes nothing.

use super::{
    seed_in_range, BudgetError, DebrisBudget, DebrisLedger, Shard, ShardSeed, MAX_VEL_MM_PER_TICK,
};

use crate::cursor::Cursor;

/// Wire version of the ledger snapshot bytes.
pub const LEDGER_SNAPSHOT_V1: u32 = 1;
/// Bytes of one encoded shard.
const SHARD_LEN: usize = 4 + 4 + 12 + 12 + 4 + 1 + 1;

impl DebrisLedger {
    /// Canonical bytes: version, the budget knobs, tick counters and totals,
    /// then every shard in `(island, id)` order.
    #[must_use]
    pub fn snapshot_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(48 + self.shards.len() * SHARD_LEN);
        out.extend_from_slice(&LEDGER_SNAPSHOT_V1.to_le_bytes());
        out.extend_from_slice(&self.cfg.per_event_cap.to_le_bytes());
        out.extend_from_slice(&self.cfg.max_shards.to_le_bytes());
        out.extend_from_slice(&self.cfg.sleep_ticks.to_le_bytes());
        out.extend_from_slice(&self.cfg.lod_distance_mm.to_le_bytes());
        out.extend_from_slice(&self.cfg.sleep_speed_mm_per_tick.to_le_bytes());
        out.extend_from_slice(&self.tick.to_le_bytes());
        out.extend_from_slice(&self.last_admit_tick.to_le_bytes());
        out.extend_from_slice(&self.admitted_total.to_le_bytes());
        out.extend_from_slice(&self.dropped_total.to_le_bytes());
        out.extend_from_slice(&self.lod_swaps.to_le_bytes());
        out.extend_from_slice(
            &u32::try_from(self.shards.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        for s in &self.shards {
            out.extend_from_slice(&s.id.to_le_bytes());
            out.extend_from_slice(&s.island.to_le_bytes());
            for v in s.pos_mm.iter().chain(s.vel_mm_per_tick.iter()) {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&s.still_ticks.to_le_bytes());
            out.push(u8::from(s.sleeping));
            out.push(u8::from(s.low_lod));
        }
        out
    }

    /// Rebuilds a ledger from [`Self::snapshot_bytes`]; the budget knobs ride
    /// in the bytes, so a restore needs no live ledger.
    pub fn from_snapshot_bytes(bytes: &[u8]) -> Result<Self, BudgetError> {
        let mut r = Cursor::new(bytes, BudgetError::BadSnapshot);
        if r.read_u32()? != LEDGER_SNAPSHOT_V1 {
            return Err(BudgetError::BadSnapshot);
        }
        let cfg = DebrisBudget {
            per_event_cap: r.read_u32()?,
            max_shards: r.read_u32()?,
            sleep_ticks: r.read_u32()?,
            lod_distance_mm: r.read_i32()?,
            sleep_speed_mm_per_tick: r.read_i32()?,
        };
        let mut ledger = Self::new(&cfg).map_err(|_| BudgetError::BadSnapshot)?;
        ledger.tick = r.read_u32()?;
        ledger.last_admit_tick = r.read_u32()?;
        ledger.admitted_total = r.read_u64()?;
        ledger.dropped_total = r.read_u64()?;
        ledger.lod_swaps = r.read_u64()?;
        let count = usize::try_from(r.read_u32()?).map_err(|_| BudgetError::BadSnapshot)?;
        if count > usize::try_from(cfg.max_shards).unwrap_or(0) {
            return Err(BudgetError::BadSnapshot);
        }
        let mut last: Option<(u32, u32)> = None;
        for _ in 0..count {
            let id = r.read_u32()?;
            let island = r.read_u32()?;
            let mut pos = [0i32; 3];
            for p in &mut pos {
                *p = r.read_i32()?;
            }
            let mut vel = [0i32; 3];
            for v in &mut vel {
                *v = r.read_i32()?;
            }
            let still_ticks = r.read_u32()?;
            let sleeping = r.read_flag()?;
            let low_lod = r.read_flag()?;
            let seed = ShardSeed {
                id,
                island,
                pos_mm: pos,
                vel_mm_per_tick: vel,
            };
            if !seed_in_range(&seed)
                || vel.iter().any(|v| v.abs() > MAX_VEL_MM_PER_TICK)
                || last.is_some_and(|prev| prev >= (island, id))
            {
                return Err(BudgetError::BadSnapshot);
            }
            last = Some((island, id));
            ledger.shards.push(Shard {
                id,
                island,
                pos_mm: pos,
                vel_mm_per_tick: vel,
                still_ticks,
                sleeping,
                low_lod,
            });
        }
        if !r.is_done() {
            return Err(BudgetError::BadSnapshot);
        }
        Ok(ledger)
    }

    /// Replaces this ledger's state with the snapshot's. On error nothing
    /// changed.
    pub fn restore_bytes(&mut self, bytes: &[u8]) -> Result<(), BudgetError> {
        *self = Self::from_snapshot_bytes(bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::budget::arena_default_budget;

    fn seeds(n: u32, island: u32) -> Vec<ShardSeed> {
        (0..n)
            .map(|i| ShardSeed {
                id: island * 100 + i,
                island,
                pos_mm: [i32::try_from(i).expect("small") * 300, 2000, 0],
                vel_mm_per_tick: [40, 10, -20],
            })
            .collect()
    }

    /// A break-then-rollback tape: admit on tick 3 and 9, step each tick.
    fn run(ledger: &mut DebrisLedger, from: u32, to: u32) {
        for tick in from..to {
            if tick == 3 {
                ledger.admit(tick, &seeds(12, 1)).expect("admits");
            }
            if tick == 9 {
                ledger.admit(tick, &seeds(9, 2)).expect("admits");
            }
            ledger.step([0, 0, 0]);
        }
    }

    #[test]
    fn debris_snapshot_break_then_rollback_resims_byte_equal() {
        let mut live = DebrisLedger::new(&arena_default_budget()).expect("ledger");
        run(&mut live, 0, 12);
        let bytes = live.snapshot_bytes();
        assert_eq!(bytes, live.snapshot_bytes(), "bytes x2");
        run(&mut live, 12, 200);
        let end = live.state_hash();
        let end_bytes = live.snapshot_bytes();
        // Roll back to the checkpoint and resim the same tape.
        let mut back = DebrisLedger::from_snapshot_bytes(&bytes).expect("restores");
        assert_eq!(back.snapshot_bytes(), bytes, "restore then snapshot");
        run(&mut back, 12, 200);
        assert_eq!(back.state_hash(), end);
        assert_eq!(back.snapshot_bytes(), end_bytes);
        // Restore in place into a ledger that moved on.
        live.restore_bytes(&bytes).expect("restores");
        run(&mut live, 12, 200);
        assert_eq!(live.state_hash(), end);
    }

    #[test]
    fn debris_snapshot_refuses_hostile_bytes_and_changes_nothing() {
        let mut ledger = DebrisLedger::new(&arena_default_budget()).expect("ledger");
        run(&mut ledger, 0, 12);
        let good = ledger.snapshot_bytes();
        let before = ledger.state_hash();
        for cut in 0..good.len() {
            assert!(
                DebrisLedger::from_snapshot_bytes(&good[..cut]).is_err(),
                "cut {cut}"
            );
            assert!(ledger.restore_bytes(&good[..cut]).is_err());
        }
        assert_eq!(ledger.state_hash(), before, "refusals changed nothing");
        let mut trailing = good.clone();
        trailing.push(1);
        assert!(DebrisLedger::from_snapshot_bytes(&trailing).is_err());
        let mut bad_version = good.clone();
        bad_version[0] = 7;
        assert!(DebrisLedger::from_snapshot_bytes(&bad_version).is_err());
        // Swap two shards' order: not canonical, refused.
        let head = 4 + 20 + 4 + 4 + 8 + 8 + 8 + 4;
        let mut swapped = good.clone();
        let (a, b) = (head, head + SHARD_LEN);
        for k in 0..SHARD_LEN {
            swapped.swap(a + k, b + k);
        }
        assert!(DebrisLedger::from_snapshot_bytes(&swapped).is_err());
        // A flag byte that is neither 0 nor 1 is refused.
        let mut flag = good.clone();
        flag[head + SHARD_LEN - 1] = 2;
        assert!(DebrisLedger::from_snapshot_bytes(&flag).is_err());
        assert!(DebrisLedger::from_snapshot_bytes(&good).is_ok());
    }
}
