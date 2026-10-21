//! AI-vs-AI fork win rates over scripted policies.
//!
//! Consumer: [`crate::verify`] plus the generation loop and tournament-style
//! callers, which read [`Report`] for seed counts and tallies. Data owner: the caller
//! owns the [`crate::ArenaVariant`]; each match builds its own
//! [`engine_arena::ArenaConfig`] from the variant's extent, fighter count
//! and [`crate::BalanceConfig`]. Units: fixed ticks, permille win rates.
//! Determinism scope: match `i` replays seed `base_seed.wrapping_add(i)`
//! through `run_prefix` plus one [`Fork`](engine_arena::fork::Fork) with
//! per-fighter `SetAggression` morale proposals, so the same variant twice
//! gives the same report on the same machine. Error type: [`crate::Failure`]
//! rows; invalid configs fail loudly instead of simulating.
//!
//! Balance semantics: both fighters run the builtin
//! scripted utility policy (brains, no tapes, no learned nets) on the
//! default kinematic path. The winner is the highest fork `damage_dealt`,
//! ties draw. The fighter-0 rate is `(2 * wins + draws) * 1000 / (2 *
//! matches)` in integer math. Bounds are declared per variant in
//! [`crate::BalanceConfig`]; the reference arena holds `[350, 650]` around
//! the measured symmetric rate (see the `reference_arena` tests below for
//! the pinned measurement and seed count).
//!
//! Measurement note (2026-09-23, seeds 2040..2059, 600 ticks): open-arena
//! brain bouts usually end 0-0, so matchup levers barely move the rate —
//! symmetric morale scored 475, a maximal `[1000, 0]` split scored 525, and
//! a frozen foe scored 550. The verifier therefore enforces the declared
//! bounds exactly (a variant demanding dominance it does not have is
//! rejected) rather than pretending morale alone decides fights. A future
//! `engine-input` edge could run `SwapTape` attack scripts (the
//! fork-proven decisive signal); this crate stays within its two
//! authorized deps and does not guess that API.

use super::{ArenaVariant, Failure};
use engine_arena::fork::{Fork, ForkProposal};
use engine_arena::{ArenaConfig, FighterId, FighterInput, FighterTape};

/// This verifier's name in [`crate::Failure::verifier`].
pub const VERIFIER: &str = "ForkWinRates";

/// Upper bound on matches per `verify` call: one full-match fork per seed,
/// so the tournament stays a bounded content check, not an open loop.
pub const MAX_MATCHES: u32 = 100;

/// Smallest human timing noise in ticks at 60 Hz (about 33 ms).
pub const HUMAN_NOISE_MIN_TICKS: u32 = 2;
/// Largest human timing noise in ticks at 60 Hz (about 66 ms).
pub const HUMAN_NOISE_MAX_TICKS: u32 = 4;
/// Human action cadence in ticks: one committed action per 8 ticks.
pub const HUMAN_ACTION_TICKS: u32 = 8;

/// Seeded human-like error for scripted players: timing noise plus a hold cadence.
/// Disabled is the perfect player; enabled delays each decision by 2 to 4 ticks
/// and holds one action per 8 ticks. The same seed twice gives the same tape.
/// The 2 to 4 range is a house number: calibrate it on recorded human tapes
/// when any exist.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HumanError {
    /// True when the noise applies; false is the perfect player.
    pub enabled: bool,
    /// Seed mixed into the per-match delay; ignored when disabled.
    pub seed: u64,
}

impl HumanError {
    /// Perfect player: no delay, no hold.
    pub const fn off() -> Self {
        Self {
            enabled: false,
            seed: 0,
        }
    }
    /// Human-like player with seeded delay and hold cadence.
    pub const fn human(seed: u64) -> Self {
        Self {
            enabled: true,
            seed,
        }
    }
    /// Delay for `match_seed` in 2 to 4 ticks, replay-exact via the core stream.
    pub fn delay_for(&self, match_seed: u64) -> u32 {
        let mixed = engine_core::splitmix64(match_seed.wrapping_add(self.seed));
        let rem = mixed % 3;
        if rem == 0 {
            HUMAN_NOISE_MIN_TICKS
        } else if rem == 1 {
            HUMAN_NOISE_MIN_TICKS + 1
        } else {
            HUMAN_NOISE_MAX_TICKS
        }
    }
}

/// Noisy tape for one fighter from perfect brain inputs: delay by `delay` ticks
/// then hold one action per 8 ticks. Pure and replay-exact; every tick carries
/// one input so the fork never falls back to the live brain for this slot.
pub fn human_tape_for(
    perfect: &[FighterInput],
    fighter: FighterId,
    ticks: u32,
    delay: u32,
    template: &FighterInput,
) -> FighterTape {
    let cap = usize::try_from(ticks).unwrap_or(0);
    let mut out = Vec::with_capacity(cap);
    let mut held = *template;
    held.tick = 0;
    held.fighter = fighter;
    for tick in 0..ticks {
        let source = tick.saturating_sub(delay);
        let idx = usize::try_from(source).unwrap_or(usize::MAX);
        let base = perfect.get(idx).copied().unwrap_or(*template);
        if tick % HUMAN_ACTION_TICKS == 0 {
            let mut cur = base;
            cur.tick = tick;
            cur.fighter = fighter;
            held = cur;
        } else {
            held.tick = tick;
            held.fighter = fighter;
        }
        out.push(held);
    }
    FighterTape(out)
}

/// Apply per-fighter morale as fork `SetAggression` proposals.
/// `None` when a slot index cannot name a fighter (defensive: configs
/// validate first, so this fires only on an internal contract break).
pub(crate) fn apply_morale(fork: &mut Fork, morale: &[u16]) -> Option<()> {
    for (slot, permille) in morale.iter().enumerate() {
        let fighter = u8::try_from(slot).unwrap_or(u8::MAX);
        if fighter == u8::MAX {
            return None;
        }
        fork.apply_proposal(ForkProposal::SetAggression {
            fighter,
            permille: *permille,
        })
        .ok()?;
    }
    Some(())
}

/// Play one fork with optional human error on slot zero.
/// Disabled replays [`fork_damage`]; enabled records the perfect brain tape
/// once, delays and holds it for slot zero, then replays with that tape.
pub(crate) fn fork_damage_with_error(
    config: &ArenaConfig,
    morale: &[u16],
    error: HumanError,
) -> Option<Vec<(u8, u16)>> {
    if !error.enabled {
        return fork_damage(config, morale);
    }
    let (snapshot, _) = engine_arena::run_prefix(config, &FighterTape::default(), 0)?;
    let mut perfect = Fork::from_snapshot(snapshot, 0)?;
    apply_morale(&mut perfect, morale)?;
    let (_, dataset) = perfect.run_recorded(config.ticks);
    let mut perfect_zero: Vec<FighterInput> = Vec::new();
    for row in &dataset.rows {
        if row.action.fighter == 0 {
            perfect_zero.push(row.action.input);
        }
    }
    perfect_zero.sort_by_key(|input| input.tick);
    let template = perfect_zero.first().copied()?;
    let fighter_zero = FighterId::new(0)?;
    let delay = error.delay_for(config.seed);
    let noisy = human_tape_for(&perfect_zero, fighter_zero, config.ticks, delay, &template);
    let (snapshot_two, _) = engine_arena::run_prefix(config, &FighterTape::default(), 0)?;
    let mut second = Fork::from_snapshot(snapshot_two, 0)?;
    apply_morale(&mut second, morale)?;
    second
        .apply_proposal(ForkProposal::SwapTape {
            fighter: 0,
            inputs: noisy.0.clone(),
        })
        .ok()?;
    Some(second.run_n(config.ticks).damage_dealt)
}

/// Fork-tournament outcome: tallies plus the permille rate. `seeds` is the
/// exact seed list (`base_seed.wrapping_add(i)`), so a report replays
/// exactly and reviewers see the seed count at a glance. Seed count and
/// uncertainty travel with the report so seeded invariants (trigger and
/// balance callers) can quote them without re-running the tournament.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Report {
    /// Matches played (== `seeds.len()`).
    pub matches: u32,
    /// Fixed ticks per match.
    pub match_ticks: u32,
    /// Exact seeds replayed, in match order.
    pub seeds: Vec<u64>,
    /// Matches fighter 0 won outright.
    pub wins: u32,
    /// Matches tied for top damage.
    pub draws: u32,
    /// Matches another fighter won outright.
    pub losses: u32,
    /// `(2 * wins + draws) * 1000 / (2 * matches)`.
    pub rate_permille: u16,
}

impl Report {
    /// Seeds replayed: always `matches` on a completed tournament.
    /// The trigger's `JobDone.seed_count` copies this so consumers never recount.
    pub fn seed_count(&self) -> u32 {
        self.matches
    }

    /// One-sigma half-width of the fighter-0 score in permille, from the
    /// match tallies. Each match scores 1.0 (win), 0.5 (draw) or 0.0
    /// (loss); the mean is `rate_permille / 1000` and the standard error
    /// is `sqrt(var / n)` with the empirical score variance. Integer
    /// tallies in, rounded permille out; deterministic on one machine
    /// (uses `sqrt` only, no wall clock or hash order).
    pub fn uncertainty_permille(&self) -> u16 {
        if self.matches == 0 {
            return 0;
        }
        let n = f64::from(self.matches);
        let p = f64::from(self.rate_permille) / 1000.0;
        let variance = (f64::from(self.wins) * (1.0 - p) * (1.0 - p)
            + f64::from(self.draws) * (0.5 - p) * (0.5 - p)
            + f64::from(self.losses) * p * p)
            / n;
        if variance <= 0.0 || !variance.is_finite() {
            return 0;
        }
        let half_width = (variance / n).sqrt() * 1000.0;
        if !half_width.is_finite() {
            return 0;
        }
        half_width.round().clamp(0.0, 1000.0) as u16
    }
}

fn fail(reason: String) -> Failure {
    Failure {
        verifier: VERIFIER,
        reason,
    }
}

/// Play one full-match fork and return per-slot damage in slot order.
/// `None` when the snapshot or fork rejects (defensive: configs validate
/// first, so this fires only on an internal contract break).
pub(crate) fn fork_damage(config: &ArenaConfig, morale: &[u16]) -> Option<Vec<(u8, u16)>> {
    let (snapshot, _) = engine_arena::run_prefix(config, &FighterTape::default(), 0)?;
    let mut fork = Fork::from_snapshot(snapshot, 0)?;
    apply_morale(&mut fork, morale)?;
    Some(fork.run_n(config.ticks).damage_dealt)
}

/// How fighter 0 fared in one match.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SlotZero {
    /// Fighter 0 dealt strictly the most damage.
    Win,
    /// Fighter 0 tied for the most damage.
    Draw,
    /// Another fighter dealt strictly more damage.
    Loss,
}

/// Judge fighter 0 from per-slot damage rows: the highest damage wins, a
/// shared top is a draw for every tied slot.
pub(crate) fn slot_zero_result(damage: &[(u8, u16)]) -> SlotZero {
    let top = damage.iter().map(|(_, dealt)| *dealt).max().unwrap_or(0);
    let leaders: Vec<u8> = damage
        .iter()
        .filter(|(_, dealt)| *dealt == top)
        .map(|(slot, _)| *slot)
        .collect();
    if leaders.len() == 1 && leaders[0] == 0 {
        SlotZero::Win
    } else if leaders.contains(&0) {
        SlotZero::Draw
    } else {
        SlotZero::Loss
    }
}

/// Run the tournament and tally fighter-0 wins/draws/losses. Fails without
/// simulating when the fighter count, morale rows, match count or arena
/// config is invalid.
pub fn report(variant: &ArenaVariant) -> Result<Report, Failure> {
    let fighters = variant.morale.len();
    if fighters < 2 || fighters > usize::from(engine_arena::MAX_PARTICIPANTS) {
        return Err(fail(format!(
            "unsupported fighter count {fighters}: morale rows must number 2..={}",
            engine_arena::MAX_PARTICIPANTS
        )));
    }
    for (slot, permille) in variant.morale.iter().enumerate() {
        if *permille > 1000 {
            return Err(fail(format!(
                "invalid morale {permille} for fighter {slot}: must be 0..=1000"
            )));
        }
    }
    let balance = &variant.balance;
    if balance.matches == 0 || balance.matches > MAX_MATCHES {
        return Err(fail(format!(
            "invalid match count {}: must be 1..={MAX_MATCHES}",
            balance.matches
        )));
    }
    let config = ArenaConfig {
        seed: balance.base_seed,
        ticks: balance.match_ticks,
        fighters: fighters as u8,
        half_extent_m: variant.half_extent_m,
        // TABLES_VERSION_V1 keeps the golden frozen on v1 rules.
        table_version: 1,
    };
    if let Err(error) = config.validate() {
        return Err(fail(format!("invalid match config: {error:?}")));
    }

    let mut wins = 0u32;
    let mut draws = 0u32;
    let mut losses = 0u32;
    let mut seeds = Vec::with_capacity(balance.matches as usize);
    for index in 0..balance.matches {
        let seed = balance.base_seed.wrapping_add(u64::from(index));
        seeds.push(seed);
        let match_config = ArenaConfig { seed, ..config };
        let Some(damage) =
            fork_damage_with_error(&match_config, &variant.morale, HumanError::off())
        else {
            return Err(fail(format!(
                "fork rejected match {index} (seed {seed}): internal contract break"
            )));
        };
        match slot_zero_result(&damage) {
            SlotZero::Win => wins += 1,
            SlotZero::Draw => draws += 1,
            SlotZero::Loss => losses += 1,
        }
    }
    let rate = ((2 * wins + draws) * 1000 / (2 * balance.matches)) as u16;
    Ok(Report {
        matches: balance.matches,
        match_ticks: balance.match_ticks,
        seeds,
        wins,
        draws,
        losses,
        rate_permille: rate,
    })
}

/// Check one finished [`Report`] against the declared bounds. Shared by
/// [`check`] (which runs the tournament) and the trigger (which
/// tallies incrementally and must not re-run forks at completion). The
/// failure reason always quotes the measured rate, bounds, seed range,
/// tallies, seed count and one-sigma uncertainty so seeded invariants are
/// diagnosed without a second run.
pub fn check_report(report: &Report, balance: &super::BalanceConfig) -> Vec<Failure> {
    if report.rate_permille < balance.min_win_permille
        || report.rate_permille > balance.max_win_permille
    {
        vec![fail(format!(
            "win rate {} permille outside [{}, {}] over {} matches, seeds {}..{} (wins {}, draws {}, losses {}, seed count {}, +-{} permille 1-sigma)",
            report.rate_permille,
            balance.min_win_permille,
            balance.max_win_permille,
            report.matches,
            balance.base_seed,
            balance
                .base_seed
                .wrapping_add(u64::from(report.matches) - 1),
            report.wins,
            report.draws,
            report.losses,
            report.seed_count(),
            report.uncertainty_permille(),
        ))]
    } else {
        Vec::new()
    }
}

/// Check the tournament rate against the declared bounds. Returns empty on a
/// pass, one [`Failure`] on a rejection.
pub fn check(variant: &ArenaVariant) -> Vec<Failure> {
    match report(variant) {
        Err(failure) => vec![failure],
        Ok(outcome) => check_report(&outcome, &variant.balance),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{reference_arena, BalanceConfig};
    use super::*;

    fn tiny_balance() -> BalanceConfig {
        BalanceConfig {
            matches: 2,
            match_ticks: 60,
            base_seed: 7,
            min_win_permille: 0,
            max_win_permille: 1000,
        }
    }

    #[test]
    fn report_counts_seeds_and_tallies() {
        let mut variant = reference_arena();
        variant.balance = tiny_balance();
        let outcome = report(&variant).expect("tiny config is valid");
        assert_eq!(outcome.matches, 2);
        assert_eq!(outcome.match_ticks, 60);
        assert_eq!(outcome.seeds, vec![7, 8]);
        assert_eq!(outcome.wins + outcome.draws + outcome.losses, 2);
        let expect = ((2 * outcome.wins + outcome.draws) * 1000 / 4) as u16;
        assert_eq!(outcome.rate_permille, expect);
        // Same variant twice: identical report (fork determinism).
        assert_eq!(report(&variant).expect("deterministic"), outcome);
    }

    #[test]
    fn reference_arena_passes_with_documented_seeds() {
        let variant = reference_arena();
        let outcome = report(&variant).expect("reference config is valid");
        assert_eq!(outcome.matches, 20);
        assert_eq!(
            outcome.seeds,
            (0..20).map(|i| 2040 + i).collect::<Vec<u64>>()
        );
        assert!(
            (350..=650).contains(&outcome.rate_permille),
            "symmetric rate must sit inside reference bounds: {outcome:?}"
        );
        assert!(check(&variant).is_empty());
    }

    #[test]
    fn demanding_bounds_are_rejected() {
        // The symmetric tournament scores ~475 (see measurement note): a
        // variant demanding fighter-0 dominance ([900, 1000]) is rejected
        // with the measured rate, bounds, seed range and tallies quoted.
        let mut variant = reference_arena();
        variant.balance.min_win_permille = 900;
        variant.balance.max_win_permille = 1000;
        let outcome = report(&variant).expect("bounds do not affect the run");
        assert!(
            !(900..=1000).contains(&outcome.rate_permille),
            "symmetric rate must miss demanding bounds: {outcome:?}"
        );
        let failures = check(&variant);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].verifier, VERIFIER);
        assert!(
            failures[0].reason.contains("win rate"),
            "{}",
            failures[0].reason
        );
        assert!(
            failures[0].reason.contains("20 matches"),
            "{}",
            failures[0].reason
        );
        assert!(
            failures[0].reason.contains("2040..2059"),
            "{}",
            failures[0].reason
        );
    }

    #[test]
    fn invalid_configs_fail_without_simulating() {
        let mut variant = reference_arena();
        variant.balance = tiny_balance();
        variant.morale = vec![500];
        assert!(report(&variant).is_err());
        let mut variant = reference_arena();
        variant.balance = tiny_balance();
        variant.morale = vec![500; usize::from(engine_arena::MAX_PARTICIPANTS) + 1];
        assert!(report(&variant).is_err());
        let mut variant = reference_arena();
        variant.balance = tiny_balance();
        variant.morale = vec![500, 1001];
        assert!(report(&variant).is_err());
        let mut variant = reference_arena();
        variant.balance = tiny_balance();
        variant.balance.matches = 0;
        assert!(report(&variant).is_err());
        let mut variant = reference_arena();
        variant.balance = tiny_balance();
        variant.balance.matches = MAX_MATCHES + 1;
        assert!(report(&variant).is_err());
        let mut variant = reference_arena();
        variant.balance = tiny_balance();
        variant.half_extent_m = 200.0;
        assert!(report(&variant).is_err());
        // Rejections surface through check as failures, never panics.
        assert_eq!(check(&variant).len(), 1);
    }

    #[test]
    fn human_error_defaults_off_with_documented_noise_and_cadence() {
        assert!(!HumanError::off().enabled, "default is the perfect player");
        assert!(HumanError::human(7).enabled);
        assert_eq!(HUMAN_NOISE_MIN_TICKS, 2, "timing noise starts at 2 ticks");
        assert_eq!(HUMAN_NOISE_MAX_TICKS, 4, "timing noise ends at 4 ticks");
        assert_eq!(HUMAN_ACTION_TICKS, 8, "one committed action per 8 ticks");
    }

    #[test]
    fn human_delay_stays_in_2_to_4_ticks_and_replays() {
        let error = HumanError::human(2040);
        for seed in 0..200u64 {
            let delay = error.delay_for(seed);
            assert!(
                (HUMAN_NOISE_MIN_TICKS..=HUMAN_NOISE_MAX_TICKS).contains(&delay),
                "seed {seed}: delay {delay} outside 2..=4 ticks",
            );
            assert_eq!(delay, error.delay_for(seed), "seed {seed} must replay");
        }
        // Disabled still answers deterministically (callers ignore it when off).
        assert_eq!(
            HumanError::off().delay_for(9),
            HumanError::off().delay_for(9),
        );
    }

    #[test]
    fn human_tape_holds_one_action_per_8_ticks() {
        let config = ArenaConfig {
            seed: 7,
            ticks: 60,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        };
        let Some(template) = crate::agent::template_input(&config) else {
            panic!("template records");
        };
        let Some(fighter) = FighterId::new(0) else {
            panic!("slot zero exists");
        };
        let mut perfect = Vec::new();
        for (tick, byte) in (0..24u32).zip([1u8, 2, 3].into_iter().cycle()) {
            let mut input = template;
            input.tick = tick;
            input.buttons.0 = byte;
            perfect.push(input);
        }
        let tape = human_tape_for(&perfect, fighter, 24, 3, &template);
        assert_eq!(tape.0.len(), 24);
        for (input, tick) in tape.0.iter().zip(0..24u32) {
            assert_eq!(input.tick, tick);
            assert_eq!(input.fighter.0, 0);
        }
        // Tick 8 latches perfect[8 - 3]; the hold window repeats that latch.
        assert_eq!(tape.0[8].buttons.0, perfect[5].buttons.0);
        for tick in 9..16usize {
            assert_eq!(
                tape.0[tick].buttons.0, tape.0[8].buttons.0,
                "tick {tick} must hold the tick-8 latch",
            );
        }
    }

    #[test]
    fn human_error_replays_on_a_tiny_fork_and_off_matches_plain() {
        let config = ArenaConfig {
            seed: 7,
            ticks: 60,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        };
        let morale = [500u16, 500u16];
        let error = HumanError::human(11);
        let first = fork_damage_with_error(&config, &morale, error).expect("tiny fork runs");
        let second = fork_damage_with_error(&config, &morale, error).expect("tiny fork replays");
        assert_eq!(first, second, "same error seed twice gives the same damage");
        assert_eq!(
            fork_damage_with_error(&config, &morale, HumanError::off()),
            fork_damage(&config, &morale),
            "disabled error is the perfect player",
        );
    }
}
