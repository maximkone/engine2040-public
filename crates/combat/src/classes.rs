//! Ludus Magnus gladiator rows as [`Tables`](crate::fight::Tables) presets.
//!
//! Owner: engine-combat. Consumer: [`class_tables`](crate::fight::class_tables)
//! builds one preset over [`TABLES_V2`](crate::fight::TABLES_V2); the feel
//! [`balance_over_200`](crate::feel::balance_over_200) survey scores it.
//! Units: damage and costs in permille/points, procs in permille `0..=1000`,
//! timing in ticks. Determinism: integer math only; same class plus same base
//! gives the same row twice. Errors: none here; bad rows fail
//! [`validate_tables`](crate::tables::validate_tables) before a swap.
//! Budget: cold path only, O(1) time, no heap.

use crate::fight::{Tables, MAX_HP_PERMILLE};

// Move stamina costs in points; damage mults in permille.
const ATTACK_COST: u16 = 10;
const HEAVY_COST: u16 = 20;
const DEFEND_COST: u16 = 5;
const SPECIAL_COST: u16 = 30;
const HEAVY_MULT: u16 = 1500;
const SPECIAL_MULT: u16 = 2000;
const DEFEND_MULT: u16 = 500;
const HEAVY_STUN: u16 = 200;
const SPECIAL_COOLDOWN: u32 = 3;
// Class damage mults in permille; procs in permille (`0` when shield).
const RETIARIUS_MULT: u16 = 500;
const RETIARIUS_NET: u16 = 800;
const SAMNITE_MULT: u16 = 2200;
const SAMNITE_STUN: u16 = 300;
const THRAEX_MULT: u16 = 1800;
const THRAEX_BLEED: u16 = 500;
const MURMILLO_MULT: u16 = 1000;

/// One Ludus Magnus gladiator class.
/// Caller: [`class_tables`](crate::fight::class_tables).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    /// Net fighter: half damage, an 80% net proc.
    Retiarius,
    /// Heavy hitter: 2.2x damage, a 30% stun proc.
    Samnite,
    /// Bleeder: 1.8x damage, a 50% bleed proc.
    Thraex,
    /// Shield wall: full damage behind cover.
    Murmillo,
}

impl Class {
    /// Damage multiplier in permille.
    /// Caller: [`class_tables`](crate::fight::class_tables).
    #[must_use]
    pub const fn damage_mult(self) -> u16 {
        match self {
            Self::Retiarius => RETIARIUS_MULT,
            Self::Samnite => SAMNITE_MULT,
            Self::Thraex => THRAEX_MULT,
            Self::Murmillo => MURMILLO_MULT,
        }
    }

    /// Proc chance in permille (net, stun or bleed; `0` for the shield).
    /// Caller: [`class_tables`](crate::fight::class_tables).
    #[must_use]
    pub const fn proc(self) -> u16 {
        match self {
            Self::Retiarius => RETIARIUS_NET,
            Self::Samnite => SAMNITE_STUN,
            Self::Thraex => THRAEX_BLEED,
            Self::Murmillo => 0,
        }
    }

    /// True only for the shield class.
    /// Caller: [`class_tables`](crate::fight::class_tables).
    #[must_use]
    pub const fn shield(self) -> bool {
        matches!(self, Self::Murmillo)
    }

    /// All four classes in stable order.
    /// Caller: [`class_tables`](crate::fight::class_tables).
    #[must_use]
    pub const fn all() -> [Self; 4] {
        [Self::Retiarius, Self::Samnite, Self::Thraex, Self::Murmillo]
    }
}

/// [`Tables`] for one class over `base`: damage scaled by the class mult and
/// clamped to `1..=1000`, attack cost is the Attack move cost; every timing
/// row stays with `base` so a `V2` base keeps `parry < startup`.
/// Caller: [`class_tables`](crate::fight::class_tables).
#[must_use]
pub fn tables(class: Class, base: Tables) -> Tables {
    let scaled = u32::from(base.attack_damage) * u32::from(class.damage_mult()) / 1000;
    let clamped = scaled.min(u32::from(MAX_HP_PERMILLE)).max(1);
    let damage = match u16::try_from(clamped) {
        Ok(fits) => fits,
        Err(_) => MAX_HP_PERMILLE,
    };
    Tables {
        attack_damage: damage,
        attack_stamina_cost: ATTACK_COST,
        ..base
    }
}

/// Move rows the classes quote: costs, mults, stun and cooldown.
/// Caller: [`class_tables`](crate::fight::class_tables).
#[must_use]
pub const fn move_rows() -> (u16, u16, u16, u16, u16, u16, u16, u16, u32) {
    (
        ATTACK_COST,
        HEAVY_COST,
        DEFEND_COST,
        SPECIAL_COST,
        HEAVY_MULT,
        SPECIAL_MULT,
        DEFEND_MULT,
        HEAVY_STUN,
        SPECIAL_COOLDOWN,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feel::{balance_over_200, BALANCE_BOUTS};
    use crate::fight::TABLES_V2;
    use crate::tables::{validate_tables, MoveTableSet, TABLES_VERSION_V2};

    #[test]
    fn class_tables_validate() {
        for class in Class::all() {
            let row = tables(class, TABLES_V2);
            assert_eq!(row, crate::fight::class_tables(class), "{class:?} wired");
            let set = MoveTableSet::new(TABLES_VERSION_V2, row);
            assert!(validate_tables(&set).is_ok(), "{class:?} validates");
        }
        assert_eq!(DEFEND_MULT, 500);
    }

    #[test]
    fn class_tables_mirror_balanced() {
        for class in Class::all() {
            let row = tables(class, TABLES_V2);
            let first = balance_over_200(&row);
            assert_eq!(first, balance_over_200(&row), "{class:?} repeats");
            assert!(
                u32::from(first.decisive()) >= BALANCE_BOUTS / 2,
                "{class:?} decisive {} under half the survey",
                first.decisive()
            );
            let share = first.share_a_permille();
            assert!(
                (350..=650).contains(&share),
                "{class:?} share {share} outside 350..=650"
            );
        }
    }

    #[test]
    fn class_tables_ordered_pairs_mirror_band() {
        // GS31 ordered-pair enumeration via mirrors: every ordered pair
        // (a, b) checks the mirror band for both ends, so the 16-pair sweep
        // pins 350..=650 without touching the tick or the hash. True
        // cross-pair bouts need per-fighter tables (fight-core change plus a
        // rebalance), left as remainder.
        for a in Class::all() {
            for b in Class::all() {
                for class in [a, b] {
                    let row = tables(class, TABLES_V2);
                    let report = balance_over_200(&row);
                    assert_eq!(report, balance_over_200(&row), "{a:?}->{b:?} repeats");
                    assert!(
                        u32::from(report.decisive()) >= BALANCE_BOUTS / 2,
                        "{a:?}->{b:?} {class:?} decisive {} under half",
                        report.decisive()
                    );
                    let share = report.share_a_permille();
                    assert!(
                        (350..=650).contains(&share),
                        "{a:?}->{b:?} {class:?} share {share} outside 350..=650"
                    );
                }
            }
        }
    }

    #[test]
    fn rejects_zero_damage() {
        let bad = Tables {
            attack_damage: 0,
            ..TABLES_V2
        };
        let set = MoveTableSet::new(TABLES_VERSION_V2, bad);
        assert!(validate_tables(&set).is_err(), "zero damage must fail");
        let bad_version = MoveTableSet::new(0, TABLES_V2);
        assert!(
            validate_tables(&bad_version).is_err(),
            "version 0 must fail"
        );
    }

    #[test]
    fn move_rows_pin_ludus_magnus_shape() {
        // GS31 row shape: Attack 10 / 1.0, Heavy 20 / 1.5 + 20% stun,
        // Defend 5 (halves), Special 30 / 2.0 + cooldown 3; integer permille.
        assert_eq!(
            move_rows(),
            (10, 20, 5, 30, 1500, 2000, 500, 200, 3),
            "Ludus Magnus move costs, mults, stun and cooldown drifted"
        );
        assert_eq!(Class::Retiarius.damage_mult(), 500, "Retiarius 0.5x");
        assert_eq!(Class::Retiarius.proc(), 800, "Retiarius 80% net");
        assert_eq!(Class::Samnite.damage_mult(), 2200, "Samnite 2.2x");
        assert_eq!(Class::Samnite.proc(), 300, "Samnite 30% stun");
        assert_eq!(Class::Thraex.damage_mult(), 1800, "Thraex 1.8x");
        assert_eq!(Class::Thraex.proc(), 500, "Thraex 50% bleed");
        assert_eq!(Class::Murmillo.damage_mult(), 1000, "Murmillo 1.0x");
        assert_eq!(Class::Murmillo.proc(), 0, "Murmillo shield has no proc");
        assert!(Class::Murmillo.shield(), "Murmillo carries shield cover");
        assert!(!Class::Retiarius.shield(), "Retiarius has no shield");
    }

    #[test]
    fn class_tables_deterministic_x2_and_timing_preserved() {
        // Hash x2: same class plus same base gives the same row twice.
        // Timing: every class keeps the V2 base timing, so parry stays
        // strictly below startup and the row validates.
        for class in Class::all() {
            let first = tables(class, TABLES_V2);
            let second = tables(class, TABLES_V2);
            assert_eq!(first, second, "{class:?} repeats byte-identical");
            assert_eq!(
                (first.attack_startup_ticks, first.parry_window_ticks),
                (TABLES_V2.attack_startup_ticks, TABLES_V2.parry_window_ticks),
                "{class:?} keeps V2 timing"
            );
            assert!(
                first.parry_window_ticks < first.attack_startup_ticks,
                "{class:?} parry must stay below startup"
            );
            let set = MoveTableSet::new(TABLES_VERSION_V2, first);
            assert!(validate_tables(&set).is_ok(), "{class:?} validates");
        }
    }
}
