//! eqs_bind: live EQS cover bind for one arena fighter.
//!
//! Owner: engine-arena. Consumer: `run_controlled` via [`SlotController`].
//! Units: millimetres, steer `-100..=100`, centidegrees, ticks.
//! Determinism: integer math only; same view gives same steer twice.
//! Errors: query refusal holds still, never panics, never fails validation.

use engine_ai::eqs::{CoverBoundary, EqContext, EqPoint, Tactics, Vec3Mm};

use crate::{ControlView, FighterId, FighterInput, SlotController};
use engine_input::Buttons;

/// Cover standoff behind each object, millimetres.
const STANDOFF_MM: i32 = 2_000;
/// Travel budget from the querier, millimetres.
const MAX_DIST_MM: i32 = 80_000;

/// One live EQS cover bind: aims the claimed slot at the ranked cover spot.
/// Holds the cover set, slot and tick-boundary dedupe; `request_id` is the
/// tick, so replays apply once. Fresh bounds stay silent (hash-neutral).
pub struct EqsCoverController {
    slot: u8,
    covers: Vec<Vec3Mm>,
    boundary: CoverBoundary,
    scratch: Vec<EqPoint>,
    calls: u32,
    first_self: Option<Vec3Mm>,
    first_winner: Option<Vec3Mm>,
    first_steer: Option<(i8, i8)>,
    last_self: Option<Vec3Mm>,
}

impl EqsCoverController {
    /// New bind for one slot over fixed covers (millimetres, `0..16`).
    pub fn new(slot: u8, covers: Vec<Vec3Mm>) -> Self {
        Self {
            slot,
            covers,
            boundary: CoverBoundary::new(),
            scratch: Vec::new(),
            calls: 0,
            first_self: None,
            first_winner: None,
            first_steer: None,
            last_self: None,
        }
    }

    fn hold(row: &mut [FighterInput], idx: usize, facing: i16) {
        if let Some(entry) = row.get_mut(idx) {
            entry.mx = 0;
            entry.mz = 0;
            entry.facing = facing.clamp(-18_000, 18_000);
            entry.buttons = Buttons::NONE;
        }
    }
}

impl SlotController for EqsCoverController {
    fn controls(&self, slot: u8) -> bool {
        slot == self.slot
    }

    fn control(&mut self, view: &ControlView<'_>, row: &mut [FighterInput]) {
        self.calls = self.calls.saturating_add(1);
        let idx = usize::from(self.slot);
        if idx >= row.len() || idx >= view.slots.len() {
            return;
        }
        let seen = view.slots[idx];
        let self_pos = Vec3Mm::new(seen.x_mm, 0, seen.z_mm);
        self.last_self = Some(self_pos);
        if self.first_self.is_none() {
            self.first_self = Some(self_pos);
        }
        let mut foe: Option<Vec3Mm> = None;
        let mut best: i128 = i128::MAX;
        for (other, slot) in view.slots.iter().enumerate() {
            if other == idx {
                continue;
            }
            let dx = i128::from(slot.x_mm) - i128::from(seen.x_mm);
            let dz = i128::from(slot.z_mm) - i128::from(seen.z_mm);
            let dist = dx * dx + dz * dz;
            if dist < best {
                best = dist;
                foe = Some(Vec3Mm::new(slot.x_mm, 0, slot.z_mm));
            }
        }
        let Some(foe_pos) = foe else {
            Self::hold(row, idx, seen.facing_cdeg);
            return;
        };
        let ctx = EqContext::new(self_pos, foe_pos);
        let Ok(query) = Tactics::build_cover_query(&self.covers, STANDOFF_MM, MAX_DIST_MM, 0)
        else {
            Self::hold(row, idx, seen.facing_cdeg);
            return;
        };
        let Ok(winner) = query.best_into(&ctx, &mut self.scratch) else {
            Self::hold(row, idx, seen.facing_cdeg);
            return;
        };
        let Some(winner) = winner else {
            Self::hold(row, idx, seen.facing_cdeg);
            return;
        };
        let Some(fighter) = FighterId::new(self.slot) else {
            Self::hold(row, idx, seen.facing_cdeg);
            return;
        };
        if self.first_winner.is_none() {
            self.first_winner = Some(winner.pos);
        }
        let Some(applied) = Tactics::bound_cover_input(
            winner,
            self_pos,
            view.tick,
            fighter,
            seen.facing_cdeg,
            u64::from(view.tick),
            &mut self.boundary,
        ) else {
            Self::hold(row, idx, seen.facing_cdeg);
            return;
        };
        if self.first_steer.is_none() {
            self.first_steer = Some((applied.mx, applied.mz));
        }
        if let Some(entry) = row.get_mut(idx) {
            entry.mx = applied.mx;
            entry.mz = applied.mz;
            entry.facing = applied.facing;
            entry.buttons = applied.buttons;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{run, run_controlled, ArenaConfig, FighterTape};

    fn covers() -> Vec<Vec3Mm> {
        vec![Vec3Mm::new(6_000, 0, 0), Vec3Mm::new(-6_000, 0, 0)]
    }

    #[test]
    fn eqs_bind_drives_fighter_to_cover_and_holds_x2() {
        let config = ArenaConfig::try_new(7, 180, 2, 20.0).expect("valid config");
        let tape = FighterTape::default();
        let plain = run(&config, &tape);
        let mut first = EqsCoverController::new(0, covers());
        let driven = run_controlled(&config, &tape, &mut first);
        let mut second = EqsCoverController::new(0, covers());
        let again = run_controlled(&config, &tape, &mut second);
        assert_eq!(driven.len(), 180);
        assert_eq!(driven, again, "replay x2 identical");
        assert_ne!(driven, plain, "cover steer changes the run");
        assert_eq!((first.calls, second.calls), (180, 180));
        let (self_pos, winner, steer) = (
            first.first_self.expect("saw tick 0"),
            first.first_winner.expect("ranked a spot"),
            first.first_steer.expect("bound one input"),
        );
        let dx = i64::from(winner.x) - i64::from(self_pos.x);
        let dz = i64::from(winner.z) - i64::from(self_pos.z);
        if dx == 0 && dz == 0 {
            assert_eq!(steer, (0, 0), "already on cover holds still");
        } else {
            assert!(steer.0 != 0 || steer.1 != 0, "steers when away");
            let dot = i64::from(steer.0) * dx + i64::from(steer.1) * dz;
            assert!(dot > 0, "steer points toward the winner");
        }
        let last = first.last_self.expect("saw the last tick");
        let init = dx * dx + dz * dz;
        let fdx = i64::from(winner.x) - i64::from(last.x);
        let fdz = i64::from(winner.z) - i64::from(last.z);
        assert!(
            fdx * fdx + fdz * fdz <= init,
            "moves toward the ranked spot"
        );
    }

    #[test]
    fn eqs_bind_never_queried_matches_run() {
        let config = ArenaConfig::try_new(7, 60, 2, 20.0).expect("valid config");
        let tape = FighterTape::default();
        let plain = run(&config, &tape);
        let mut silent = EqsCoverController::new(9, covers());
        let driven = run_controlled(&config, &tape, &mut silent);
        assert_eq!(driven, plain, "claiming nothing keeps the legacy run");
        assert_eq!(silent.boundary.last_applied(), None);
    }
}
