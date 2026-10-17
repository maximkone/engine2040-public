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

/// Ticks one initiative window lasts: one duelist presses, the other guards.
pub(crate) const INITIATIVE_WINDOW_TICKS: u32 = 24;
/// Ticks held within [`STILL_MM`] of one spot before a tier brain steps off.
pub(crate) const STILL_TICKS: u32 = 60;
/// Distance in millimetres a fighter must travel to count as having moved.
const STILL_MM: i64 = 400;
/// Ticks a tier brain backs away once stillness trips.
pub(crate) const DISENGAGE_TICKS: u32 = 20;

/// Attack-lane lean in score points a tier takes up in the window where it
/// presses and gives back where it guards: `1000 - 200 * tier`, floor 200.
/// Each rung leans less, so the climb keeps its guard up.
pub fn press_lean(row: TierRow) -> i32 {
    (1_000 - 200 * i32::from(row.tier)).max(200)
}

/// From tier 3 the brain re-scores the tick the foe's swing ends, so its
/// reply lands inside the stagger.
pub const fn punishes(row: TierRow) -> bool {
    row.tier >= 3
}

/// Per-duel play of a tier brain: the first-percept seed (tick and duel key,
/// equal for both duelists) plus the stillness clock. Integers only.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TierPlay {
    born: Option<u32>,
    key: u64,
    anchor: [i32; 2],
    still: u32,
    away: u32,
}

impl TierPlay {
    /// Seed once from the first percept; the key hashes the unordered pair
    /// of public start positions in millimetres.
    pub(crate) fn seed(&mut self, tick: u32, me: [i32; 2], foe: [i32; 2]) {
        if self.born.is_some() {
            return;
        }
        let (lo, hi) = if me <= foe { (me, foe) } else { (foe, me) };
        let word = |v: i32| u64::from_ne_bytes(i64::from(v).to_ne_bytes());
        self.born = Some(tick);
        self.key = engine_core::hash_u64s(&[word(lo[0]), word(lo[1]), word(hi[0]), word(hi[1])]);
        self.anchor = me;
    }

    /// Initiative lean of fighter `slot` at `tick`. A duel alternates who
    /// presses: each window a coin keyed on the duel names one slot to press
    /// (`+lean`) and the other to guard (`-lean`), so equal tiers never swing
    /// in lockstep and the same-tick id order decides nothing. Public start
    /// positions and the tick are its only inputs. `lifted` (a swing in
    /// flight, the riposte window) zeroes it: a blow is always answered.
    pub(crate) fn lean(&self, row: TierRow, slot: u8, tick: u32, lifted: bool) -> i32 {
        if lifted {
            return 0;
        }
        let window = tick.saturating_sub(self.born.unwrap_or(tick)) / INITIATIVE_WINDOW_TICKS;
        let coin = engine_core::CounterRng::new(self.key).sample(u64::from(window)) & 1;
        if coin == u64::from(slot & 1) {
            press_lean(row)
        } else {
            -press_lean(row)
        }
    }

    /// Stillness clock, one call per tick with the fighter's own position.
    /// Held within [`STILL_MM`] of one spot for [`STILL_TICKS`] ticks, the
    /// brain backs away for [`DISENGAGE_TICKS`] (`true`) with its buttons up.
    /// That ends a mutual-parry lock, where both duelists swing and guard in
    /// step, before it becomes a standing idle.
    pub(crate) fn backing_away(&mut self, me: [i32; 2]) -> bool {
        let moved = |axis: usize| i64::from(me[axis] - self.anchor[axis]).pow(2);
        if moved(0) + moved(1) > STILL_MM * STILL_MM {
            self.anchor = me;
            self.still = 0;
        } else {
            self.still += 1;
        }
        if self.away == 0 && self.still >= STILL_TICKS {
            self.away = DISENGAGE_TICKS;
            self.still = 0;
        }
        let backing = self.away > 0;
        self.away = self.away.saturating_sub(1);
        backing
    }
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

    /// Two duelists of one tier get complementary leans each window: one
    /// presses, one guards, and the coin is fair rather than an id order.
    #[test]
    fn initiative_names_one_presser_per_window() {
        let row = SHIPPED_TIERS[1];
        let (mut a, mut b) = (TierPlay::default(), TierPlay::default());
        a.seed(0, [0, 0], [9_000, 0]);
        b.seed(0, [9_000, 0], [0, 0]);
        let mut presses = 0;
        for window in 0..40 {
            let tick = window * INITIATIVE_WINDOW_TICKS;
            let (first, second) = (a.lean(row, 0, tick, false), b.lean(row, 1, tick, false));
            assert_eq!((first, first.abs()), (-second, press_lean(row)));
            presses += i32::from(first > 0);
        }
        assert!((8..=32).contains(&presses), "a fair coin: {presses}");
        let leans = SHIPPED_TIERS.map(press_lean);
        assert!(leans[0] > leans[1] && leans[1] > leans[2], "{leans:?}");
    }

    /// Held still, a duelist backs away once the stillness window fills, for
    /// the disengage span; one that keeps walking never does.
    #[test]
    fn stillness_backs_away_and_walking_never_does() {
        let (mut held, mut walker) = (TierPlay::default(), TierPlay::default());
        held.seed(0, [0, 0], [3_000, 0]);
        walker.seed(0, [0, 0], [3_000, 0]);
        let calls: Vec<bool> = (0..STILL_TICKS + DISENGAGE_TICKS)
            .map(|_| held.backing_away([0, 0]))
            .collect();
        let first = calls.iter().position(|b| *b);
        let span = calls.iter().filter(|b| **b).count();
        assert_eq!(first, Some(STILL_TICKS as usize - 1));
        assert_eq!(span, DISENGAGE_TICKS as usize);
        assert!((0..300).all(|n: i32| !walker.backing_away([n * 500, 0])));
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
