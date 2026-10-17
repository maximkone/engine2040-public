//! pose_hold: which fighter slots freeze while sim hitstop holds.
//!
//! Consumer: [`crate::App`] folds the per-tick hold mask from the live tick
//! events; the look (`crate::fighter_look`, next hunk) freezes those slots'
//! clip clocks so impacts read on screen. Data owner: combat owns the freeze
//! authority; this module owns only the read-only slot pick. Units: fighter
//! slots (`0..fighters`), fixed ticks. Determinism: pure integer logic,
//! presentation only, never hashed. Errors: none; every function is total and
//! writes into caller buffers, never allocating.
//!
//! Caller: `crates/game/src/lib.rs` (`App::simulate_tick`, `App::rebuild_live`).

use engine_combat::FightEvent;

/// True when `slot` must freeze this tick: the sim holds clip clocks
/// (`holding`) and the slot dealt or took the impact.
#[must_use]
pub fn should_hold(holding: bool, slot: u8, attacker: u8, victim: u8) -> bool {
    holding && (slot == attacker || slot == victim)
}

/// Fill `out` with the per-slot hold flags for one tick: the attacker and the
/// victim freeze while `holding`, every other slot advances. Writes every
/// entry of `out`; entries at or above `fighters` read false, so bad slots
/// named by the impact never freeze.
pub fn hold_mask(fighters: u8, attacker: u8, victim: u8, holding: bool, out: &mut [bool]) {
    let live = usize::from(fighters);
    let attacker = usize::from(attacker);
    let victim = usize::from(victim);
    for (index, flag) in out.iter_mut().enumerate() {
        *flag = holding && index < live && (index == attacker || index == victim);
    }
}

/// The impact's slots from one tick's combat events: the first Hit or Parried
/// event's `(attacker, victim)`. `None` on a quiet tick (Blocked, Dodged and
/// empty count down, never freeze).
#[must_use]
pub fn impact_slots(events: &[FightEvent]) -> Option<(u8, u8)> {
    events.iter().find_map(|event| match event {
        FightEvent::Hit {
            attacker, victim, ..
        } => Some((*attacker, *victim)),
        FightEvent::Parried { attacker, victim } => Some((*attacker, *victim)),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(attacker: u8, victim: u8) -> FightEvent {
        FightEvent::Hit {
            attacker,
            victim,
            damage: 120,
        }
    }

    #[test]
    fn hitstop_hold_freezes_attacker_and_victim_only() {
        assert!(should_hold(true, 0, 0, 1));
        assert!(should_hold(true, 1, 0, 1));
        assert!(!should_hold(true, 2, 0, 1));
        assert!(!should_hold(false, 0, 0, 1));
        assert!(!should_hold(false, 1, 0, 1));
    }

    #[test]
    fn hitstop_hold_phases_stay_put_while_others_advance() {
        // Feel LF10: during hitstop the two impact slots' phases stay put
        // while the rest advance; afterwards all advance.
        let mut phases = [0u32; 4];
        
        let mut mask = [false; 4];
        // Tick 0: impact on slots 0 and 1, hitstop holds for 2 ticks.
        hold_mask(4, 0, 1, true, &mut mask);
        assert_eq!(mask, [true, true, false, false]);
        let frozen: [u32; 4] = phases;
        for (slot, phase) in phases.iter_mut().enumerate() {
            if !mask[slot] {
                *phase += 1;
            }
        }
        assert_eq!(
            [phases[0], phases[1]],
            [frozen[0], frozen[1]],
            "held phases stay put"
        );
        assert_eq!([phases[2], phases[3]], [1, 1], "free slots advance");
        // Tick 1: still holding, same picture.
        hold_mask(4, 0, 1, true, &mut mask);
        for (slot, phase) in phases.iter_mut().enumerate() {
            if !mask[slot] {
                *phase += 1;
            }
        }
        assert_eq!([phases[0], phases[1]], [0, 0], "held phases stay put");
        assert_eq!([phases[2], phases[3]], [2, 2], "free slots advance");
        // Tick 2: freeze over, every slot advances.
        hold_mask(4, 0, 1, false, &mut mask);
        assert_eq!(mask, [false; 4]);
        for phase in phases.iter_mut() {
            *phase += 1;
        }
        assert_eq!(phases, [1, 1, 3, 3], "all advance afterwards");
    }

    #[test]
    fn hitstop_hold_ignores_slots_above_the_match() {
        let mut mask = [true; 4];
        hold_mask(2, 5, 9, true, &mut mask);
        assert_eq!(mask, [false; 4], "bad slots never freeze");
        let mut short = [true; 1];
        hold_mask(4, 0, 1, true, &mut short);
        assert_eq!(short, [true], "short buffers fill their prefix");
    }

    #[test]
    fn hitstop_hold_finds_first_impact_slots() {
        let blocked = FightEvent::Blocked {
            attacker: 2,
            victim: 3,
            chip: 10,
        };
        let dodged = FightEvent::Dodged {
            attacker: 2,
            victim: 3,
        };
        assert_eq!(impact_slots(&[]), None, "quiet ticks hold nothing");
        assert_eq!(impact_slots(&[blocked]), None, "blocked never freezes");
        assert_eq!(impact_slots(&[dodged]), None, "dodged never freezes");
        assert_eq!(
            impact_slots(&[blocked, hit(0, 1)]),
            Some((0, 1)),
            "first impact wins"
        );
        let parried = FightEvent::Parried {
            attacker: 1,
            victim: 0,
        };
        assert_eq!(
            impact_slots(&[parried, hit(0, 1)]),
            Some((1, 0)),
            "parry freezes too"
        );
    }
}
