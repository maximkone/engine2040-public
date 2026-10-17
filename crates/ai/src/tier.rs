//! Duel enemy tiers T1-T4: the skill-first ladder for the arena duel.
//!
//! Consumer: the balance tournament hook (reads [`SHIPPED_TIERS`],
//! [`WIN_BAR_PERMILLE`], [`TIER_DUELS`] and [`skill_score`]) and the arena
//! duel (installs a row on the depth brain). Data owner:
//! [`TierRow`](crate::depth::TierRow) in `depth.rs` owns parsing and storage;
//! this module is the narrow door over it: shipped rows, the win bar and the
//! skill score. Units: permille shares, ticks at 60 Hz. Determinism: same
//! rows give the same scores; integer math only. Errors:
//! [`TierError`](crate::depth::TierError) carries the 1-based line. Budget:
//! parsing runs once at match build; scoring is a few integer ops with no
//! allocation.

pub use crate::depth::{parse_tiers, TierError, TierRow, SHIPPED_TIERS, TIER_RANGE_MM};

/// Win bar: a tier beats the tier below in at least this share (permille) of
/// seeded duels, and loses to the tier above by the same bar.
pub const WIN_BAR_PERMILLE: u16 = 600;

/// Seeded duels per tier pair in the tier check.
pub const TIER_DUELS: u32 = 200;

/// Reaction climb for a reaction time: `(4 - reaction) * 100`, saturating
/// at zero past the `1..=4` table so out-of-range input never underflows
/// (plain `-` panics in debug and wraps in release). Integer only.
/// Plain `fn` (not `const`): exact widening via `From` is not const-stable
/// on this toolchain, and every caller runs at match build or in tests.
fn reaction_climb(reaction: u8) -> u32 {
    4u32.saturating_sub(u32::from(reaction)) * 100
}

/// Skill score for one tier row: nerve plus the reaction climb, so higher
/// tiers read the foe faster and press harder while health stays in its
/// narrow band. Reaction ticks today are 1..=4 (4 slowest), hence the flip.
pub fn skill_score(row: TierRow) -> u32 {
    u32::from(row.aggression_permille()) + reaction_climb(row.reaction_ticks())
}

/// Fewest wins out of [`TIER_DUELS`] that clears [`WIN_BAR_PERMILLE`].
pub fn bar_wins() -> u32 {
    u32::from(WIN_BAR_PERMILLE) * TIER_DUELS / 1_000
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped designer file parses to the shipped rows plus the primus,
    /// twice alike.
    #[test]
    fn shipped_file_matches_shipped_rows_twice() {
        let text = include_str!("../../../content/design/enemies.rows");
        let first = parse_tiers(text).expect("shipped tiers parse");
        let second = parse_tiers(text).expect("shipped tiers replay");
        assert_eq!(first[..3], SHIPPED_TIERS);
        assert_eq!(first[3], crate::depth::PRIMUS_TIER);
        assert_eq!(second, first);
    }

    /// Skill climbs every rung while health stays in its narrow band: tiers
    /// are skill first, never stat walls.
    #[test]
    fn skill_climbs_while_health_stays_narrow() {
        let scores = [
            skill_score(SHIPPED_TIERS[0]),
            skill_score(SHIPPED_TIERS[1]),
            skill_score(SHIPPED_TIERS[2]),
        ];
        assert!(
            scores[0] < scores[1] && scores[1] < scores[2],
            "skill must climb T1 < T2 < T3, got {scores:?}"
        );
        let mut low = u16::MAX;
        let mut high = u16::MIN;
        for row in SHIPPED_TIERS {
            assert!(
                (900..=1_200).contains(&row.hp_permille),
                "tier {} hp {} leaves the narrow band",
                row.tier,
                row.hp_permille
            );
            low = low.min(row.hp_permille);
            high = high.max(row.hp_permille);
        }
        assert!(
            high - low <= 150,
            "hp spread {high} - {low} is a stat wall, not skill"
        );
    }

    /// The bar arithmetic the hook runs: 600 permille of 200 duels is 120.
    #[test]
    fn bar_needs_120_of_200() {
        assert_eq!(bar_wins(), 120);
    }

    /// Skill score goldens per tier arm: nerve plus the reaction climb.
    #[test]
    fn skill_score_goldens_per_arm() {
        assert_eq!(skill_score(SHIPPED_TIERS[0]), 400);
        assert_eq!(skill_score(SHIPPED_TIERS[1]), 700);
        assert_eq!(skill_score(SHIPPED_TIERS[2]), 1_000);
    }

    /// Reaction climb saturates past the table: the `4 - reaction` flip
    /// must never underflow on out-of-range input.
    #[test]
    fn reaction_climb_saturates_beyond_table() {
        assert_eq!(reaction_climb(1), 300);
        assert_eq!(reaction_climb(4), 0);
        assert_eq!(
            reaction_climb(5),
            0,
            "past the table saturates, never wraps"
        );
        assert_eq!(reaction_climb(u8::MAX), 0);
    }

    /// Bad rows refuse through this door with the 1-based line.
    #[test]
    fn bad_rows_refuse_with_the_line() {
        let float = "table enemies version 1\ntier_1 hp_permille=950.0 tell_ticks=15 stagger_permille=400 counter=1\ntier_2 hp_permille=1000 tell_ticks=15 stagger_permille=500 counter=2\ntier_3 hp_permille=1050 tell_ticks=15 stagger_permille=600 counter=1";
        assert_eq!(parse_tiers(float), Err(TierError::Float { line: 2 }));
        let thin = "table enemies version 1\ntier_1 hp_permille=500 tell_ticks=15 stagger_permille=400 counter=1\ntier_2 hp_permille=1000 tell_ticks=15 stagger_permille=500 counter=2\ntier_3 hp_permille=1050 tell_ticks=15 stagger_permille=600 counter=1";
        assert_eq!(
            parse_tiers(thin),
            Err(TierError::OutOfFence {
                line: 2,
                field: "hp_permille",
                value: 500
            })
        );
    }
}
