//! Balance tournament over AI-vs-AI forks (a proof gate measured at
//! 3 difficulties).
//!
//! Consumer: the proof quality gate plus DESIGN move-table packets, which
//! read [`TournamentReport`] for win rates, kill shares, stuck counts and
//! exchange reachability. Data owner: the caller owns the config; every
//! match builds its own [`ArenaConfig`](engine_arena::ArenaConfig) from it.
//! Units: fixed ticks, permille rates, raw button bytes for moves.
//! Determinism: seeded and replay-exact (same config twice gives the same
//! report; difficulty `d` replays `base_seed + d * SEED_GAP + i`).
//! Errors: [`Failure`] rows; invalid configs fail before any simulation.

use super::agent::template_input;
use super::norm_score::{self, NormScore, TaskReturns};
use super::{
    winrates::{self, apply_morale, human_tape_for, HumanError},
    BalanceConfig, Failure,
};
use engine_arena::fork::{
    Fork, ForkMetrics, ForkProposal, TierRow, SHIPPED_TIERS, STUCK_SPEED_MPS,
};
use engine_arena::recorder::Dataset;
use engine_arena::zones::{shipped_gate_anchor, zone_at, ZoneTally};
use engine_arena::{ArenaConfig, FighterId, FighterInput, FighterTape, TICK_HZ};

/// This harness's name in [`Failure::verifier`].
pub const VERIFIER: &str = "BalanceTournament";

/// Standard matches per difficulty: the P3 proof size.
pub const MATCHES_PER_DIFFICULTY: u32 = 200;
/// Upper bound on matches per difficulty (== the standard size).
pub const MAX_MATCHES: u32 = 200;
/// Standard fixed ticks per tournament match.
pub const STANDARD_TICKS: u32 = 600;
/// Upper bound on difficulties per tournament (the game-ai ladder rung count).
pub const MAX_DIFFICULTIES: usize = 8;
/// Upper bound on fixed ticks per match inside one tournament.
pub const MAX_MATCH_TICKS: u32 = 3600;
/// Seed gap between difficulties: above [`MAX_MATCHES`], so difficulty
/// seed ranges never overlap and stay replay-exact.
pub const SEED_GAP: u64 = 1_000_003;
/// P3 bar: lowest acceptable fighter-0 win rate in permille.
pub const WIN_MIN_PERMILLE: u16 = 400;
/// P3 bar: highest acceptable fighter-0 win rate in permille.
pub const WIN_MAX_PERMILLE: u16 = 600;
/// P3 bar: no single move takes more than this share of kills (permille).
pub const KILL_SHARE_MAX_PERMILLE: u16 = 350;
/// P3 bar: no fighter holds below fork stuck speed for 3 s at 60 Hz (the
/// stuck definition frozen in the game-ai plan). Trips come from a
/// consecutive-stillness scan over recorded poses (post-kill corpse holds
/// excluded), never from the cumulative fork counter. Single owner is
/// `engine_core::STUCK_WINDOW_TICKS`.
pub const STUCK_WINDOW_TICKS: u32 = engine_core::STUCK_WINDOW_TICKS;
/// Fixed ticks per exchange-reachability probe match.
pub const EXCHANGE_TICKS: u32 = 240;
/// Spotlight matches per discovered move (the overpowered-move detector).
pub const SPOTLIGHT_MATCHES: u32 = 20;
/// Fixed ticks per spotlight match.
pub const SPOTLIGHT_TICKS: u32 = 600;
/// Ticks for the button-discovery probe windows.
const DISCOVERY_TICKS: u32 = 120;

/// One tournament difficulty: a name plus per-fighter morale in permille
/// (`0..=1000`, 500 neutral). Morale travels as fork `SetAggression`
/// proposals — the same scoring input the game-ai depth paces through — so a
/// difficulty changes pacing pressure, never stats or hidden information.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Difficulty {
    /// Stable name, for reasons and report rows only.
    pub name: String,
    /// Per-fighter morale; the tournament fences duels (`len == 2`).
    pub morale: Vec<u16>,
}

impl Difficulty {
    /// Even mirror match: symmetric morale, so any deviation from 500
    /// permille is mirror-match bias, not imbalance.
    pub fn even() -> Self {
        Self {
            name: "even".to_string(),
            morale: vec![500, 500],
        }
    }

    /// Pressure: one brave fighter against one cautious fighter.
    pub fn pressure() -> Self {
        Self {
            name: "pressure".to_string(),
            morale: vec![700, 300],
        }
    }

    /// Cauldron: maximal morale split (pacing extremes, same rules).
    pub fn cauldron() -> Self {
        Self {
            name: "cauldron".to_string(),
            morale: vec![1000, 0],
        }
    }
}

/// Standard difficulty ladder for the P3 proof.
pub fn standard_difficulties() -> Vec<Difficulty> {
    vec![
        Difficulty::even(),
        Difficulty::pressure(),
        Difficulty::cauldron(),
    ]
}

/// Balance-tournament configuration: one AI-vs-AI fork tournament per
/// difficulty, a spotlight kill-rate probe per discovered move, and
/// scripted exchange-reachability probes.
#[derive(Clone, PartialEq, Debug)]
pub struct TournamentConfig {
    /// First seed of the whole tournament.
    pub base_seed: u64,
    /// Fork matches per difficulty (`1..=MAX_MATCHES`).
    pub matches: u32,
    /// Fixed ticks per tournament match (`1..=MAX_MATCH_TICKS`).
    pub match_ticks: u32,
    /// Arena half-extent in meters (`5.0..=100.0`, the arena range).
    pub half_extent_m: f32,
    /// Difficulties under test (`1..=MAX_DIFFICULTIES`).
    pub difficulties: Vec<Difficulty>,
    /// Spotlight matches per discovered move (`1..=MAX_MATCHES`).
    pub spotlight_matches: u32,
    /// Fixed ticks per spotlight match (`1..=MAX_MATCH_TICKS`).
    pub spotlight_ticks: u32,
}

impl TournamentConfig {
    /// The P3 proof shape: 200 matches per standard difficulty at 600
    /// ticks, with a 20-match spotlight per discovered move.
    pub fn standard() -> Self {
        Self {
            base_seed: 2040,
            matches: MATCHES_PER_DIFFICULTY,
            match_ticks: STANDARD_TICKS,
            half_extent_m: 20.0,
            difficulties: standard_difficulties(),
            spotlight_matches: SPOTLIGHT_MATCHES,
            spotlight_ticks: SPOTLIGHT_TICKS,
        }
    }

    /// First seed of difficulty `index`.
    pub fn difficulty_seed(&self, index: usize) -> u64 {
        self.base_seed
            .wrapping_add((index as u64).wrapping_mul(SEED_GAP))
    }
}

/// One move's kill share: kills are attributed to the killer's button byte
/// at or before the killing tick (recorded fork rows), so a "move" is an
/// observed byte, never a named input constant. Sorted by byte.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KillShare {
    /// Observed button byte of the killing blow.
    pub move_byte: u8,
    /// Matches where this byte landed the kill.
    pub kills: u32,
    /// `kills * 1000 / kills_total`.
    pub share_permille: u16,
}

/// Exchange reachability: each exchange answer must appear in the scripted
/// probe matches. Parry is the timed-block negation (fresh press negates,
/// late press eats the full hit); riposte is the parrying fighter landing
/// the answer; clash is the simultaneous-strike script resolving
/// deterministically (ID-ordered single landing, replay-exact — the
/// bind/poise resolution awaits the exchange-resolver arena wiring, and
/// this same script becomes its assertion when it lands).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ExchangeReach {
    /// A fresh block press negated a strike that a late press ate whole.
    pub parry_seen: bool,
    /// The parrying fighter answered a negated strike with a landing hit.
    pub riposte_seen: bool,
    /// Simultaneous strikes resolved ID-ordered (one landing, no double).
    pub clash_seen: bool,
}

/// Spotlight kill-rate for one discovered move: the move spammed every tick
/// by fighter 0 against a brain-driven fighter 1. The seeded-overpowered
/// detector: a strike lever kills nearly every match (the fork decisive
/// signal), so a rate above [`KILL_SHARE_MAX_PERMILLE`] flags the move for
/// the DESIGN move-table packets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Spotlight {
    /// Discovered button byte under test.
    pub move_byte: u8,
    /// Spotlight matches played.
    pub matches: u32,
    /// Matches where fighter 0 landed the kill.
    pub kills: u32,
    /// `kills * 1000 / matches`.
    pub kill_rate_permille: u16,
}

/// One difficulty's tournament outcome: tallies plus permille rate, kill
/// attribution and stuck trips. `seeds` is the exact seed list, so a
/// report replays exactly and reviewers see the seed count at a glance.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DifficultyReport {
    /// Difficulty name.
    pub name: String,
    /// Matches played (== `seeds.len()`).
    pub matches: u32,
    /// Fixed ticks per match.
    pub match_ticks: u32,
    /// Exact seeds replayed, in match order.
    pub seeds: Vec<u64>,
    /// Matches fighter 0 won outright (top damage).
    pub wins: u32,
    /// Matches tied for top damage.
    pub draws: u32,
    /// Matches another fighter won outright.
    pub losses: u32,
    /// `(2 * wins + draws) * 1000 / (2 * matches)`.
    pub rate_permille: u16,
    /// One-sigma half-width of the rate in permille (win-rate uncertainty).
    pub uncertainty_permille: u16,
    /// Matches with any kill (`ticks_to_kill.is_some()`).
    pub kills_total: u32,
    /// Kill attribution per observed killing byte, sorted by byte. Empty
    /// when the tournament drew no blood (vacuous, never a flag).
    pub kill_shares: Vec<KillShare>,
    /// `(slot, matches tripped)` for fighters slow 180+ ticks, slot order.
    pub stuck_trips: Vec<(u8, u32)>,
    /// Tripped match seeds per slot (slot order, match order): the exact
    /// repro handoff for the game-ai lane (wall-grind freezes, seed 2043).
    pub stuck_seeds: Vec<(u8, Vec<u64>)>,
    /// Total slow ticks across every fighter and match.
    pub stuck_ticks_total: u64,
    /// GP36 zone *time* shares in permille `[centre, ring, wall, gate]` over
    /// every recorded fighter tick, summing to exactly 1000.
    pub zone_permille: [u32; 4],
    /// Gate-mouth *time* share in permille (the GP36 bar keeps it under 250):
    /// a share of fighter ticks, never the level-check floor *area* share.
    /// Equals `zone_permille[3]`.
    pub entrance_share: u32,
    /// Kills whose killing-tick killer pose sat in each zone, in
    /// `[centre, ring, wall, gate]` order.
    pub kills_per_zone: [u32; 4],
    /// Fighter 0's damage dealt in each match, in match order: the agent's
    /// per-episode return that the norm score resamples.
    pub agent_damage: Vec<i64>,
}

/// Full balance-tournament report: one row per difficulty plus the move
/// spotlight and the exchange probes. Deterministic: the same config twice
/// gives the same report.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TournamentReport {
    /// Per-difficulty outcomes, in config order.
    pub per_difficulty: Vec<DifficultyReport>,
    /// Spotlight kill-rates per discovered move, sorted by byte.
    pub spotlight: Vec<Spotlight>,
    /// Button bytes discovered by damage probing, sorted.
    pub discovered_moves: Vec<u8>,
    /// Scripted exchange probes (difficulty-independent scripts).
    pub exchange: ExchangeReach,
    /// Oracle-normalized agent score over the difficulties: the agent is
    /// fighter 0 on the ladder, the anchor is the best spam policy per task.
    pub norm: NormScore,
}

/// PQ9 line: the difficulty tally scored as fighter 0 vs fighter 1 Elo,
/// naming the entrants that sit `OP_GAP` or more above the start rating.
fn elo_line(row: &DifficultyReport) -> String {
    match crate::rating::table_from_tally(0, 1, row.wins, row.draws, row.losses) {
        Ok(table) => format!(
            "  elo fighter0={:.0} fighter1={:.0} op_flagged={:?}\n",
            table.rating(0).unwrap_or(crate::rating::START_RATING),
            table.rating(1).unwrap_or(crate::rating::START_RATING),
            crate::rating::flagged(&table),
        ),
        Err(error) => format!("  elo refused: {error}\n"),
    }
}

impl TournamentReport {
    /// Reproducible text report: one line per difficulty plus spotlight and
    /// exchange rows, so reviewers quote numbers without re-running.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("PQ2 balance tournament: AI-vs-AI forks per difficulty\n");
        for row in &self.per_difficulty {
            out.push_str(&format!(
                "difficulty {} matches={} ticks={} seeds={}..{} wins={} draws={} losses={} rate={} +-{} kills_total={} stuck_trips=[{}] stuck_ticks_total={}\n",
                row.name,
                row.matches,
                row.match_ticks,
                row.seeds.first().copied().unwrap_or(0),
                row.seeds.last().copied().unwrap_or(0),
                row.wins,
                row.draws,
                row.losses,
                row.rate_permille,
                row.uncertainty_permille,
                row.kills_total,
                row.stuck_trips
                    .iter()
                    .map(|(slot, n)| format!("{slot}:{n}"))
                    .collect::<Vec<_>>()
                    .join(","),
                row.stuck_ticks_total,
            ));
            out.push_str(&format!(
                "  zone share centre={} ring={} wall={} gate={} (fighter-tick time permille, sum 1000)\n",
                row.zone_permille[0],
                row.zone_permille[1],
                row.zone_permille[2],
                row.zone_permille[3],
            ));
            out.push_str(&elo_line(row));
            out.push_str(&format!(
                "  entrance_share={} (gate time share permille: a fighter-tick share, not the level-check floor-area share)\n",
                row.entrance_share,
            ));
            out.push_str(&format!(
                "  kills_per_zone=[{},{},{},{}] (centre,ring,wall,gate)\n",
                row.kills_per_zone[0],
                row.kills_per_zone[1],
                row.kills_per_zone[2],
                row.kills_per_zone[3],
            ));
            for share in &row.kill_shares {
                out.push_str(&format!(
                    "  kill_share move_byte={} kills={} share={}\n",
                    share.move_byte, share.kills, share.share_permille,
                ));
            }
            for (slot, seeds) in &row.stuck_seeds {
                out.push_str(&format!("  stuck slot={slot} seeds={seeds:?}\n"));
            }
        }
        out.push_str(&format!("{}\n", self.norm.line()));
        out.push_str(&format!(
            "exchange parry={} riposte={} clash={} discovered_moves={:?}\n",
            self.exchange.parry_seen,
            self.exchange.riposte_seen,
            self.exchange.clash_seen,
            self.discovered_moves,
        ));
        let charter_line =
            crate::charter::run_c1(crate::charter::first_bout_log(self.exchange.parry_seen))
                .map_or_else(|error| format!("FAIL c1: {error}"), |verdict| verdict.line);
        out.push_str(&format!("charter_c1 {charter_line}\n"));
        for spot in &self.spotlight {
            out.push_str(&format!(
                "spotlight move_byte={} matches={} kills={} kill_rate={}\n",
                spot.move_byte, spot.matches, spot.kills, spot.kill_rate_permille,
            ));
        }
        out
    }
}

fn fail(reason: String) -> Failure {
    Failure {
        verifier: VERIFIER,
        reason,
    }
}

/// Idle input for `slot` at `tick`: zero wish, zero facing, zeroed buttons.
/// Zero is the universal idle, not a mirrored contract value.
fn idle_input(template: &FighterInput, slot: u8, tick: u32) -> Option<FighterInput> {
    let id = FighterId::new(slot)?;
    let mut input = *template;
    input.tick = tick;
    input.fighter = id;
    input.mx = 0;
    input.mz = 0;
    input.facing = 0;
    input.buttons.0 = 0;
    Some(input)
}

/// Buttoned input for `slot` at `tick`: idle wish with `byte` as the raw
/// button byte. Callers pass only bytes discovered by damage probing.
fn buttoned_input(template: &FighterInput, slot: u8, tick: u32, byte: u8) -> Option<FighterInput> {
    let mut input = idle_input(template, slot, tick)?;
    input.buttons.0 = byte;
    Some(input)
}

/// Spam tape: every tick for `slot` carries `byte`.
fn spam_tape(template: &FighterInput, slot: u8, ticks: u32, byte: u8) -> Option<FighterTape> {
    let mut inputs = Vec::with_capacity(ticks as usize);
    for tick in 0..ticks {
        inputs.push(buttoned_input(template, slot, tick, byte)?);
    }
    Some(FighterTape(inputs))
}

/// Single-shot tape: `byte` at `at`, idle everywhere else.
fn single_tape(
    template: &FighterInput,
    slot: u8,
    ticks: u32,
    at: u32,
    byte: u8,
) -> Option<FighterTape> {
    let mut inputs = Vec::with_capacity(ticks as usize);
    for tick in 0..ticks {
        if tick == at {
            inputs.push(buttoned_input(template, slot, tick, byte)?);
        } else {
            inputs.push(idle_input(template, slot, tick)?);
        }
    }
    Some(FighterTape(inputs))
}

/// Hold-from tape: idle before `from`, `byte` from `from` on.
fn hold_from_tape(
    template: &FighterInput,
    slot: u8,
    ticks: u32,
    from: u32,
    byte: u8,
) -> Option<FighterTape> {
    let mut inputs = Vec::with_capacity(ticks as usize);
    for tick in 0..ticks {
        if tick >= from {
            inputs.push(buttoned_input(template, slot, tick, byte)?);
        } else {
            inputs.push(idle_input(template, slot, tick)?);
        }
    }
    Some(FighterTape(inputs))
}

/// Multi-shot tape: `shots` carry their bytes, idle everywhere else.
/// Shots must name distinct ticks (duplicates would fail validation).
fn shots_tape(
    template: &FighterInput,
    slot: u8,
    ticks: u32,
    shots: &[(u32, u8)],
) -> Option<FighterTape> {
    let mut inputs = Vec::with_capacity(ticks as usize);
    for tick in 0..ticks {
        match shots.iter().find(|(at, _)| *at == tick) {
            Some((_, byte)) => inputs.push(buttoned_input(template, slot, tick, *byte)?),
            None => inputs.push(idle_input(template, slot, tick)?),
        }
    }
    Some(FighterTape(inputs))
}

/// Merge per-slot tapes into one `(tick, fighter)`-sorted tape.
fn merge_tapes(tapes: &[FighterTape]) -> FighterTape {
    let mut inputs = Vec::new();
    for tape in tapes {
        inputs.extend(tape.0.iter().copied());
    }
    inputs.sort_by_key(|input| (input.tick, input.fighter.0));
    FighterTape(inputs)
}

/// Install an optional shipped tier row on every brain of a duel fork: a
/// difficulty is one skill rung over the whole field, never a per-slot stat
/// wall. `None` on an internal contract break (configs validate first).
fn apply_tier(fork: &mut Fork, fighters: u8, tier: Option<TierRow>) -> Option<()> {
    let Some(row) = tier else {
        return Some(());
    };
    for fighter in 0..fighters {
        fork.apply_proposal(ForkProposal::SetTier { fighter, tier: row })
            .ok()?;
    }
    Some(())
}

/// Run one full-match fork: prefix at tick 0, `SetAggression` morale per
/// slot (the game-ai pacing input), then each tape slice rides as a `SwapTape`
/// proposal so brain slots stay live exactly like the real tick.
/// `None` on any internal contract break (configs validate first).
fn fork_metrics(config: &ArenaConfig, morale: &[u16], tape: &FighterTape) -> Option<ForkMetrics> {
    let (snapshot, _) = engine_arena::run_prefix(config, tape, 0)?;
    let mut fork = Fork::from_snapshot(snapshot, 0)?;
    for (slot, permille) in morale.iter().enumerate() {
        fork.apply_proposal(ForkProposal::SetAggression {
            fighter: slot as u8,
            permille: *permille,
        })
        .ok()?;
    }
    for slot in 0..config.fighters {
        let slice: Vec<FighterInput> = tape
            .0
            .iter()
            .filter(|input| input.fighter.0 == slot)
            .copied()
            .collect();
        if slice.is_empty() {
            continue;
        }
        fork.apply_proposal(ForkProposal::SwapTape {
            fighter: slot,
            inputs: slice,
        })
        .ok()?;
    }
    Some(fork.run_n(config.ticks))
}

/// Recorded fork run: metrics plus per-fighter rows for kill attribution.
/// Metrics equal [`fork_metrics`] over the same fork by construction, plus
/// the optional difficulty tier rung installed on every brain.
fn fork_recorded(
    config: &ArenaConfig,
    morale: &[u16],
    tier: Option<TierRow>,
    tape: &FighterTape,
) -> Option<(ForkMetrics, Dataset)> {
    let (snapshot, _) = engine_arena::run_prefix(config, tape, 0)?;
    let mut fork = Fork::from_snapshot(snapshot, 0)?;
    for (slot, permille) in morale.iter().enumerate() {
        fork.apply_proposal(ForkProposal::SetAggression {
            fighter: slot as u8,
            permille: *permille,
        })
        .ok()?;
    }
    apply_tier(&mut fork, config.fighters, tier)?;
    for slot in 0..config.fighters {
        let slice: Vec<FighterInput> = tape
            .0
            .iter()
            .filter(|input| input.fighter.0 == slot)
            .copied()
            .collect();
        if slice.is_empty() {
            continue;
        }
        fork.apply_proposal(ForkProposal::SwapTape {
            fighter: slot,
            inputs: slice,
        })
        .ok()?;
    }
    Some(fork.run_recorded(config.ticks))
}

/// Recorded fork run with seeded human-like error on slot zero: the perfect
/// live-brain tape records once, then slot zero replays it delayed by
/// [`HumanError::delay_for`] ticks with one held action per
/// [`winrates::HUMAN_ACTION_TICKS`] ticks while slot one stays live.
/// Disabled replays [`fork_recorded`]; `None` on any internal contract break.
fn fork_recorded_with_error(
    config: &ArenaConfig,
    morale: &[u16],
    tier: Option<TierRow>,
    error: HumanError,
) -> Option<(ForkMetrics, Dataset)> {
    if !error.enabled {
        return fork_recorded(config, morale, tier, &FighterTape::default());
    }
    let (_, dataset) = fork_recorded(config, morale, tier, &FighterTape::default())?;
    let mut perfect_zero: Vec<FighterInput> = dataset
        .rows
        .iter()
        .filter(|row| row.action.fighter == 0)
        .map(|row| row.action.input)
        .collect();
    perfect_zero.sort_by_key(|input| input.tick);
    let template = perfect_zero.first().copied()?;
    let fighter_zero = FighterId::new(0)?;
    let delay = error.delay_for(config.seed);
    let noisy = human_tape_for(&perfect_zero, fighter_zero, config.ticks, delay, &template);
    let (snapshot, _) = engine_arena::run_prefix(config, &FighterTape::default(), 0)?;
    let mut fork = Fork::from_snapshot(snapshot, 0)?;
    apply_morale(&mut fork, morale)?;
    apply_tier(&mut fork, config.fighters, tier)?;
    fork.apply_proposal(ForkProposal::SwapTape {
        fighter: 0,
        inputs: noisy.0.clone(),
    })
    .ok()?;
    Some(fork.run_recorded(config.ticks))
}

/// Damage dealt by fighter 0 into a frozen fighter 1 when spamming `byte`.
/// The frozen-foe reference: full damage lands, nothing answers back.
fn spam_into_frozen(
    config: &ArenaConfig,
    template: &FighterInput,
    byte: u8,
    ticks: u32,
) -> Option<u16> {
    let probe = ArenaConfig { ticks, ..*config };
    let spam = spam_tape(template, 0, ticks, byte)?;
    let (snapshot, _) = engine_arena::run_prefix(&probe, &spam, 0)?;
    let mut fork = Fork::from_snapshot(snapshot, 0)?;
    fork.apply_proposal(ForkProposal::SwapTape {
        fighter: 0,
        inputs: spam.0.clone(),
    })
    .ok()?;
    fork.apply_proposal(ForkProposal::FreezeFighter { fighter: 1 })
        .ok()?;
    Some(
        fork.run_n(ticks)
            .damage_dealt
            .first()
            .map(|(_, dealt)| *dealt)
            .unwrap_or(0),
    )
}

/// Discover the strike byte: the first low button byte that deals damage
/// into a frozen foe. Behavior-discovered per config, never a named input
/// constant; the report quotes the byte so reviewers see what ran.
fn discover_strike(config: &ArenaConfig, template: &FighterInput) -> Option<u8> {
    for byte in 1u8..=7u8 {
        if spam_into_frozen(config, template, byte, DISCOVERY_TICKS)? > 0 {
            return Some(byte);
        }
    }
    None
}

/// Damage dealt by fighter 0's two-strike script (`byte` single shots at
/// ticks 0 and 60) against fighter 1 holding `hold` from tick 0, plus the
/// holder's own damage. The guard/evade discriminator: guards soak both
/// impacts into chip (small nonzero total, holder deals nothing), evades
/// negate outright or eat full hits late, strikers answer back.
fn hold_two_strike(
    config: &ArenaConfig,
    template: &FighterInput,
    strike: u8,
    hold: u8,
    ticks: u32,
) -> Option<(u16, u16)> {
    let probe = ArenaConfig { ticks, ..*config };
    let mut striker = shots_tape(template, 0, ticks, &[(0, strike), (60, strike)])?.0;
    striker.sort_by_key(|input| (input.tick, input.fighter.0));
    let holder = hold_from_tape(template, 1, ticks, 0, hold)?;
    let tape = merge_tapes(&[FighterTape(striker), holder]);
    let metrics = fork_metrics(&probe, &[500, 500], &tape)?;
    let dealt = |slot: u8| {
        metrics
            .damage_dealt
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|(_, d)| *d)
            .unwrap_or(0)
    };
    Some((dealt(0), dealt(1)))
}

/// Discover guard and evade bytes from the non-strike bytes: a guard soaks
/// the two-strike script into chip (holder deals nothing, striker total
/// strictly below one full hit); an evade negates or eats full hits late
/// (holder deals nothing, striker total zero or at/above one full hit).
/// Returns `(guard, evade)`; each `None` when no byte answers that way.
fn discover_answers(
    config: &ArenaConfig,
    template: &FighterInput,
    strike: u8,
    frozen_full: u16,
) -> (Option<u8>, Option<u8>) {
    let (mut guard, mut evade) = (None, None);
    for byte in 1u8..=7u8 {
        if byte == strike {
            continue;
        }
        let Some((striker_dealt, holder_dealt)) =
            hold_two_strike(config, template, strike, byte, DISCOVERY_TICKS)
        else {
            continue;
        };
        if holder_dealt > 0 {
            continue;
        }
        if guard.is_none() && striker_dealt > 0 && striker_dealt < frozen_full {
            guard = Some(byte);
        }
        if evade.is_none() && (striker_dealt == 0 || striker_dealt >= frozen_full) {
            evade = Some(byte);
        }
        if guard.is_some() && evade.is_some() {
            break;
        }
    }
    (guard, evade)
}

/// Discovered move bytes for a duel config: strike plus any guard/evade
/// answers, sorted. `None` when no strike byte exists (nothing can kill).
fn discover_moves(config: &ArenaConfig) -> Option<Vec<u8>> {
    let template_config = ArenaConfig {
        ticks: DISCOVERY_TICKS,
        ..*config
    };
    let template = template_input(&template_config)?;
    let strike = discover_strike(&template_config, &template)?;
    let frozen_full = spam_into_frozen(&template_config, &template, strike, DISCOVERY_TICKS)?;
    let (guard, evade) = discover_answers(&template_config, &template, strike, frozen_full);
    let mut moves = vec![strike];
    for byte in [guard, evade].into_iter().flatten() {
        if !moves.contains(&byte) {
            moves.push(byte);
        }
    }
    moves.sort_unstable();
    Some(moves)
}

/// Exchange probes over [`EXCHANGE_TICKS`] ticks with the discovered bytes:
/// parry (fresh press negates what a late press eats whole), riposte (the
/// parrying fighter answers with a landing hit) and clash (mutual full
/// damage off one opening script). Pure fork damage ledgers, deterministic.
fn probe_exchange(
    config: &ArenaConfig,
    template: &FighterInput,
    strike: u8,
    guard: u8,
) -> Option<ExchangeReach> {
    let probe = ArenaConfig {
        seed: config.seed,
        ticks: EXCHANGE_TICKS,
        fighters: 2,
        half_extent_m: config.half_extent_m,
        table_version: 1,
    };
    let morale = [500, 500];
    // Parry: single strike at tick 0; fresh press at tick 4 negates the
    // impact (block age 0 at resolve), a press at tick 5 eats it whole.
    let early = merge_tapes(&[
        single_tape(template, 0, probe.ticks, 0, strike)?,
        single_tape(template, 1, probe.ticks, 4, guard)?,
    ]);
    let late = merge_tapes(&[
        single_tape(template, 0, probe.ticks, 0, strike)?,
        single_tape(template, 1, probe.ticks, 5, guard)?,
    ]);
    let early_dmg = fork_metrics(&probe, &morale, &early)?
        .damage_dealt
        .first()
        .map(|(_, d)| *d)
        .unwrap_or(0);
    let late_dmg = fork_metrics(&probe, &morale, &late)?
        .damage_dealt
        .first()
        .map(|(_, d)| *d)
        .unwrap_or(0);
    let parry_seen = early_dmg == 0 && late_dmg > 0;
    // Riposte: the parrying fighter answers at tick 20 (stagger and
    // hit-stop clear) into the idling striker and lands.
    let answer = merge_tapes(&[
        single_tape(template, 0, probe.ticks, 0, strike)?,
        shots_tape(template, 1, probe.ticks, &[(4, guard), (20, strike)])?,
    ]);
    let answer_metrics = fork_metrics(&probe, &morale, &answer)?;
    let dealt = |slot: u8| {
        answer_metrics
            .damage_dealt
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|(_, d)| *d)
            .unwrap_or(0)
    };
    let riposte_seen = dealt(0) == 0 && dealt(1) > 0;
    // Clash: both strike off the same opening tick. Fight rules cancel the
    // victim's in-flight strike through hit-react, so exactly the lower ID
    // lands: the probe pins that deterministic single landing (no double
    // hit, no dropped exchange), replay-exact on every seed.
    let duel = merge_tapes(&[
        single_tape(template, 0, probe.ticks, 0, strike)?,
        single_tape(template, 1, probe.ticks, 0, strike)?,
    ]);
    let duel_metrics = fork_metrics(&probe, &morale, &duel)?;
    let duel_dealt = |slot: u8| {
        duel_metrics
            .damage_dealt
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|(_, d)| *d)
            .unwrap_or(0)
    };
    let clash_seen = duel_dealt(0) > 0 && duel_dealt(1) == 0;
    Some(ExchangeReach {
        parry_seen,
        riposte_seen,
        clash_seen,
    })
}

/// Killer slot of finished `metrics`: top damage, ties to the lowest slot.
pub(crate) fn killer_of(metrics: &ForkMetrics) -> u8 {
    let mut killer = 0u8;
    let mut top = 0u16;
    for (slot, dealt) in &metrics.damage_dealt {
        if *dealt > top {
            top = *dealt;
            killer = *slot;
        }
    }
    killer
}

/// Killing move: the killer's button byte on the killing tick, falling back
/// to its most recent nonzero byte at or before that tick (startup delay
/// means the killing tick itself often holds idle), then to zero.
fn killing_byte(dataset: &Dataset, killer: u8, kill_tick: u32) -> u8 {
    let mut idle_byte = None;
    let mut falling: Option<u8> = None;
    for row in &dataset.rows {
        if row.action.fighter != killer || row.observation.tick > kill_tick {
            continue;
        }
        let byte = row.action.input.buttons.0;
        if row.observation.tick == kill_tick {
            idle_byte = Some(byte);
        } else if byte != 0 {
            falling = Some(byte);
        }
    }
    match idle_byte {
        Some(byte) if byte != 0 => byte,
        _ => falling.or(idle_byte).unwrap_or(0),
    }
}

/// Validate a tournament config without simulating. Duels only: the
/// discovery and exchange probes script slots 0 and 1 (4-fighter squads
/// are Tier B per the game-ai plan and stay out of this harness).
fn validate(config: &TournamentConfig) -> Result<(), Failure> {
    if config.matches == 0 || config.matches > MAX_MATCHES {
        return Err(fail(format!(
            "invalid match count {}: must be 1..={MAX_MATCHES}",
            config.matches
        )));
    }
    if config.match_ticks == 0 || config.match_ticks > MAX_MATCH_TICKS {
        return Err(fail(format!(
            "invalid match ticks {}: must be 1..={MAX_MATCH_TICKS}",
            config.match_ticks
        )));
    }
    if config.spotlight_matches == 0 || config.spotlight_matches > MAX_MATCHES {
        return Err(fail(format!(
            "invalid spotlight matches {}: must be 1..={MAX_MATCHES}",
            config.spotlight_matches
        )));
    }
    if config.spotlight_ticks == 0 || config.spotlight_ticks > MAX_MATCH_TICKS {
        return Err(fail(format!(
            "invalid spotlight ticks {}: must be 1..={MAX_MATCH_TICKS}",
            config.spotlight_ticks
        )));
    }
    if config.difficulties.is_empty() || config.difficulties.len() > MAX_DIFFICULTIES {
        return Err(fail(format!(
            "invalid difficulty count {}: must be 1..={MAX_DIFFICULTIES}",
            config.difficulties.len()
        )));
    }
    let probe = ArenaConfig {
        seed: config.base_seed,
        ticks: config.match_ticks,
        fighters: 2,
        half_extent_m: config.half_extent_m,
        table_version: 1,
    };
    if let Err(error) = probe.validate() {
        return Err(fail(format!("invalid match config: {error:?}")));
    }
    for difficulty in &config.difficulties {
        if difficulty.morale.len() != 2 {
            return Err(fail(format!(
                "difficulty {} needs exactly 2 morale rows (duels only), got {}",
                difficulty.name,
                difficulty.morale.len()
            )));
        }
        for (slot, permille) in difficulty.morale.iter().enumerate() {
            if *permille > 1000 {
                return Err(fail(format!(
                    "difficulty {} invalid morale {permille} for fighter {slot}: must be 0..=1000",
                    difficulty.name
                )));
            }
        }
    }
    Ok(())
}

/// Recorded pose of `slot` at `tick` in a dataset row-major by tick then
/// slot, `None` when the shape is short. Pure: never panics.
fn pose_at(dataset: &Dataset, fighters: usize, tick: u32, slot: usize) -> Option<[f32; 3]> {
    let base = usize::try_from(tick).ok()?.checked_mul(fighters)?;
    let row = dataset.rows.get(base.checked_add(slot)?)?;
    row.observation
        .fighters
        .get(slot)
        .map(|fighter| fighter.pos)
}

/// Consecutive-stillness scan over recorded poses (the game-ai stuck
/// definition): speed sample `t` spans poses `t` and `t+1`, and a fighter
/// trips on [`STUCK_WINDOW_TICKS`] consecutive sub-threshold samples.
/// Post-kill ticks are corpse holds, so kill matches scan only ticks before
/// the killing tick. Returns tripped slots in order, at most once each.
/// Shape mismatches scan nothing (deterministic, never a panic); the fork
/// cumulative slow-tick total is reported separately and never gated.
fn scan_stuck_consecutive(
    dataset: &Dataset,
    fighters: usize,
    ticks: u32,
    kill_tick: Option<u32>,
) -> Vec<u8> {
    if fighters == 0 || dataset.rows.len() != ticks as usize * fighters {
        return Vec::new();
    }
    for (index, row) in dataset.rows.iter().enumerate() {
        if row.observation.tick != index as u32 / fighters as u32
            || usize::from(row.action.fighter) != index % fighters
            || row.observation.fighters.len() != fighters
        {
            return Vec::new();
        }
    }
    let step_m = STUCK_SPEED_MPS / TICK_HZ as f32;
    let stuck_sq = step_m * step_m;
    // Samples run while both endpoint poses are pre-kill: sample `t` spans
    // poses `t` and `t + 1`, and poses at the killing tick are still alive.
    let last_sample = kill_tick.unwrap_or_else(|| ticks.saturating_sub(1));
    let mut tripped = Vec::new();
    for slot in 0..fighters {
        let mut run = 0u32;
        for tick in 0..last_sample {
            let (a, b) = (
                dataset.rows[tick as usize * fighters + slot]
                    .observation
                    .fighters[slot]
                    .pos,
                dataset.rows[(tick as usize + 1) * fighters + slot]
                    .observation
                    .fighters[slot]
                    .pos,
            );
            let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
            if dx * dx + dz * dz < stuck_sq {
                run += 1;
                if run >= STUCK_WINDOW_TICKS {
                    tripped.push(slot as u8);
                    break;
                }
            } else {
                run = 0;
            }
        }
    }
    tripped
}

/// Kill share per killing button byte, ordered by byte, in permille of all
/// kills (at least one, so an empty tournament divides by one).
fn kill_shares_of(mut kill_counts: Vec<(u8, u32)>, kills_total: u32) -> Vec<KillShare> {
    kill_counts.sort_by_key(|(byte, _)| *byte);
    kill_counts
        .into_iter()
        .map(|(byte, kills)| KillShare {
            move_byte: byte,
            kills,
            share_permille: (kills * 1000 / kills_total.max(1)) as u16,
        })
        .collect()
}

/// Running per-difficulty tallies across one tournament's matches: wins,
/// kill attribution, stuck trips and the GP36 zone metric, folded one
/// recorded match at a time so [`run_difficulty`] stays a short driver.
struct DifficultyTotals {
    /// Matches fighter 0 won outright (top damage).
    wins: u32,
    /// Matches tied for top damage.
    draws: u32,
    /// Matches another fighter won outright.
    losses: u32,
    /// Matches with any kill.
    kills_total: u32,
    /// Kill counts per observed killing byte.
    kill_counts: Vec<(u8, u32)>,
    /// Per-slot stuck match counts.
    stuck: Vec<(u8, u32)>,
    /// Tripped match seeds per slot.
    stuck_seeds: Vec<(u8, Vec<u64>)>,
    /// Total slow ticks across every fighter and match.
    stuck_ticks_total: u64,
    /// GP36 zone time tally over every recorded fighter tick.
    zones: ZoneTally,
    /// GP36 kills whose killing-tick killer pose sat in each zone.
    kills_per_zone: [u32; 4],
}

impl DifficultyTotals {
    /// Empty tallies for a two-slot duel field.
    fn new() -> Self {
        Self {
            wins: 0,
            draws: 0,
            losses: 0,
            kills_total: 0,
            kill_counts: Vec::new(),
            stuck: vec![(0, 0), (1, 0)],
            stuck_seeds: vec![(0, Vec::new()), (1, Vec::new())],
            stuck_ticks_total: 0,
            zones: ZoneTally::default(),
            kills_per_zone: [0; 4],
        }
    }

    /// Fold one recorded match into every tally: the result, kill
    /// attribution, stuck trips, and the GP36 zone metric (every pose plus
    /// the kill's zone at the killing tick). The gate anchor is data from
    /// the shipped rows, never a constant.
    fn observe(
        &mut self,
        seed: u64,
        dataset: &Dataset,
        metrics: &ForkMetrics,
        fighters: usize,
        ticks: u32,
    ) {
        match winrates::slot_zero_result(&metrics.damage_dealt) {
            winrates::SlotZero::Win => self.wins += 1,
            winrates::SlotZero::Draw => self.draws += 1,
            winrates::SlotZero::Loss => self.losses += 1,
        }
        let gate = shipped_gate_anchor();
        let gate_mouth = [gate[0], gate[1]];
        let gate_half_m = gate[2];
        for tick in 0..ticks {
            for slot in 0..fighters {
                if let Some(pos) = pose_at(dataset, fighters, tick, slot) {
                    self.zones
                        .add(zone_at(pos[0], pos[2], gate_mouth, gate_half_m));
                }
            }
        }
        if let Some(kill_in) = metrics.ticks_to_kill {
            self.kills_total += 1;
            let killer = killer_of(metrics);
            let byte = killing_byte(dataset, killer, kill_in.saturating_sub(1));
            match self.kill_counts.iter_mut().find(|(b, _)| *b == byte) {
                Some(entry) => entry.1 += 1,
                None => self.kill_counts.push((byte, 1)),
            }
            if let Some(pos) = pose_at(
                dataset,
                fighters,
                kill_in.saturating_sub(1),
                usize::from(killer),
            ) {
                let zone = zone_at(pos[0], pos[2], gate_mouth, gate_half_m);
                self.kills_per_zone[zone.index()] += 1;
            }
        }
        for (_, slow) in &metrics.stuck_ticks {
            self.stuck_ticks_total += u64::from(*slow);
        }
        let kill_tick = metrics.ticks_to_kill.map(|kill| kill.saturating_sub(1));
        for slot in scan_stuck_consecutive(dataset, fighters, ticks, kill_tick) {
            if let Some(entry) = self.stuck.iter_mut().find(|(s, _)| *s == slot) {
                entry.1 += 1;
            }
            if let Some(entry) = self.stuck_seeds.iter_mut().find(|(s, _)| *s == slot) {
                entry.1.push(seed);
            }
        }
    }
}

/// Run one difficulty's AI-vs-AI tournament: `matches` recorded forks with
/// per-slot morale and the difficulty's shipped tier rung, tallying wins
/// plus kill attribution and stuck trips.
fn run_difficulty(
    config: &TournamentConfig,
    index: usize,
    difficulty: &Difficulty,
    error: HumanError,
) -> Result<DifficultyReport, Failure> {
    let base = config.difficulty_seed(index);
    let fighters = difficulty.morale.len();
    // The shipped ladder's production install: rung `index` reads its row
    // straight from `engine_ai::tier` (no copy of the numbers here). Rungs
    // past the shipped table stay untiered, matching a custom ladder.
    let tier = SHIPPED_TIERS.get(index).copied();
    let mut seeds = Vec::with_capacity(config.matches as usize);
    let mut agent_damage = Vec::with_capacity(config.matches as usize);
    let mut totals = DifficultyTotals::new();
    for i in 0..config.matches {
        let seed = base.wrapping_add(u64::from(i));
        seeds.push(seed);
        let match_config = ArenaConfig {
            seed,
            ticks: config.match_ticks,
            fighters: 2,
            half_extent_m: config.half_extent_m,
            table_version: 1,
        };
        let Some((metrics, dataset)) =
            fork_recorded_with_error(&match_config, &difficulty.morale, tier, error)
        else {
            return Err(fail(format!(
                "difficulty {} fork rejected match {i} (seed {seed}): internal contract break",
                difficulty.name
            )));
        };
        agent_damage.push(slot_zero_damage(&metrics));
        totals.observe(seed, &dataset, &metrics, fighters, config.match_ticks);
    }
    let kill_shares = kill_shares_of(totals.kill_counts, totals.kills_total);
    let win_permille = (2 * totals.wins + totals.draws) * 1000 / (2 * config.matches);
    let rate = u16::try_from(win_permille).unwrap_or(u16::MAX);
    let win_report = winrates::Report {
        matches: config.matches,
        match_ticks: config.match_ticks,
        seeds: seeds.clone(),
        wins: totals.wins,
        draws: totals.draws,
        losses: totals.losses,
        rate_permille: rate,
    };
    let stuck_trips: Vec<(u8, u32)> = totals.stuck.into_iter().filter(|(_, n)| *n > 0).collect();
    let stuck_seeds: Vec<(u8, Vec<u64>)> = totals
        .stuck_seeds
        .into_iter()
        .filter(|(_, seeds)| !seeds.is_empty())
        .collect();
    Ok(DifficultyReport {
        name: difficulty.name.clone(),
        matches: config.matches,
        match_ticks: config.match_ticks,
        seeds,
        wins: totals.wins,
        draws: totals.draws,
        losses: totals.losses,
        rate_permille: rate,
        uncertainty_permille: win_report.uncertainty_permille(),
        kills_total: totals.kills_total,
        kill_shares,
        stuck_trips,
        stuck_seeds,
        stuck_ticks_total: totals.stuck_ticks_total,
        zone_permille: totals.zones.shares(),
        entrance_share: totals.zones.entrance_share(),
        kills_per_zone: totals.kills_per_zone,
        agent_damage,
    })
}

/// Fighter 0's damage dealt in one match: the per-episode return the norm
/// score reads. Zero when the slot is absent (forks always carry both slots).
fn slot_zero_damage(metrics: &ForkMetrics) -> i64 {
    metrics
        .damage_dealt
        .iter()
        .find(|(slot, _)| *slot == 0)
        .map_or(0, |(_, dealt)| i64::from(*dealt))
}

/// One norm episode for a fixed policy: fighter 0 carries `byte` on every
/// tick (`0` is the idle policy) while fighter 1 stays brain-driven on the
/// difficulty's morale and tier rung. Returns fighter 0's damage dealt.
fn scripted_return(
    match_config: &ArenaConfig,
    difficulty: &Difficulty,
    tier: Option<TierRow>,
    template: &FighterInput,
    byte: u8,
) -> Result<i64, Failure> {
    let tape = spam_tape(template, 0, match_config.ticks, byte)
        .ok_or_else(|| fail(format!("norm tape for byte {byte} refused")))?;
    let (metrics, _) =
        fork_recorded(match_config, &difficulty.morale, tier, &tape).ok_or_else(|| {
            fail(format!(
                "difficulty {} norm fork rejected seed {} (byte {byte}): internal contract break",
                difficulty.name, match_config.seed
            ))
        })?;
    Ok(slot_zero_damage(&metrics))
}

/// Norm task for one difficulty. The agent's episodes are the tournament
/// row's per-match damage. The idle baseline and each discovered move spammed
/// by fighter 0 run on the first `spotlight_matches` seeds of the same
/// difficulty: one fork per seed per policy, the spotlight's own bound.
fn norm_task(
    config: &TournamentConfig,
    index: usize,
    difficulty: &Difficulty,
    row: &DifficultyReport,
    template: &FighterInput,
    moves: &[u8],
) -> Result<TaskReturns, Failure> {
    let tier = SHIPPED_TIERS.get(index).copied();
    let take = usize::try_from(config.spotlight_matches)
        .unwrap_or(usize::MAX)
        .min(row.seeds.len());
    let mut baseline = Vec::with_capacity(take);
    let mut scripted: Vec<(String, Vec<i64>)> = moves
        .iter()
        .map(|byte| (format!("spam:{byte}"), Vec::with_capacity(take)))
        .collect();
    for seed in row.seeds.iter().take(take) {
        let match_config = ArenaConfig {
            seed: *seed,
            ticks: config.match_ticks,
            fighters: 2,
            half_extent_m: config.half_extent_m,
            table_version: 1,
        };
        baseline.push(scripted_return(
            &match_config,
            difficulty,
            tier,
            template,
            0,
        )?);
        for ((_, returns), byte) in scripted.iter_mut().zip(moves) {
            returns.push(scripted_return(
                &match_config,
                difficulty,
                tier,
                template,
                *byte,
            )?);
        }
    }
    Ok(TaskReturns {
        name: difficulty.name.clone(),
        agent: row.agent_damage.clone(),
        baseline,
        scripted,
    })
}

/// Run the spotlight for one discovered move: fighter 0 spams the byte
/// against a brain-driven fighter 1 over `spotlight_matches` matches.
/// A kill counts only when fighter 0 both ends the match and tops damage.
fn run_spotlight(
    config: &TournamentConfig,
    template: &FighterInput,
    move_byte: u8,
) -> Result<Spotlight, Failure> {
    let mut kills = 0u32;
    for i in 0..config.spotlight_matches {
        let seed = config
            .base_seed
            .wrapping_add(SEED_GAP.wrapping_mul(config.difficulties.len() as u64 + 1))
            .wrapping_add(u64::from(i));
        let match_config = ArenaConfig {
            seed,
            ticks: config.spotlight_ticks,
            fighters: 2,
            half_extent_m: config.half_extent_m,
            table_version: 1,
        };
        let spam = spam_tape(template, 0, config.spotlight_ticks, move_byte).ok_or_else(|| {
            fail(format!(
                "spotlight move_byte={move_byte}: tape build refused"
            ))
        })?;
        let Some(metrics) = fork_metrics(&match_config, &[500, 500], &spam) else {
            return Err(fail(format!(
                "spotlight move_byte={move_byte} fork rejected match {i} (seed {seed}): internal contract break"
            )));
        };
        if metrics.ticks_to_kill.is_some() && killer_of(&metrics) == 0 {
            kills += 1;
        }
    }
    Ok(Spotlight {
        move_byte,
        matches: config.spotlight_matches,
        kills,
        kill_rate_permille: (kills * 1000 / config.spotlight_matches.max(1)) as u16,
    })
}

/// Run the whole balance tournament: discovery, one tournament per
/// difficulty, the move spotlight and the exchange probes. Deterministic:
/// the same config twice gives the same report.
pub fn run(config: &TournamentConfig) -> Result<TournamentReport, Failure> {
    run_with_error(config, HumanError::off())
}

/// Same tournament with seeded human-like error on slot zero of the
/// per-difficulty duel rows (perfect player when disabled). Discovery,
/// spotlight and exchange probes stay perfect brains: they pin mechanisms,
/// not difficulty. Deterministic: the same config and error twice give the
/// same report.
fn run_with_error(
    config: &TournamentConfig,
    error: HumanError,
) -> Result<TournamentReport, Failure> {
    validate(config)?;
    let duel = ArenaConfig {
        seed: config.base_seed,
        ticks: DISCOVERY_TICKS,
        fighters: 2,
        half_extent_m: config.half_extent_m,
        table_version: 1,
    };
    let template = template_input(&duel)
        .ok_or_else(|| fail("template record refused: internal contract break".to_string()))?;
    let moves = discover_moves(&duel)
        .ok_or_else(|| fail("no strike byte discovered: nothing can kill".to_string()))?;
    let strike = moves[0];
    let frozen_full = spam_into_frozen(&duel, &template, strike, DISCOVERY_TICKS)
        .ok_or_else(|| fail("strike reference refused: internal contract break".to_string()))?;
    let (guard, _) = discover_answers(&duel, &template, strike, frozen_full);
    let mut per_difficulty = Vec::with_capacity(config.difficulties.len());
    for (index, difficulty) in config.difficulties.iter().enumerate() {
        per_difficulty.push(run_difficulty(config, index, difficulty, error)?);
    }
    let mut spotlight = Vec::with_capacity(moves.len());
    for byte in &moves {
        spotlight.push(run_spotlight(config, &template, *byte)?);
    }
    let mut tasks = Vec::with_capacity(config.difficulties.len());
    for (index, (difficulty, row)) in config.difficulties.iter().zip(&per_difficulty).enumerate() {
        tasks.push(norm_task(
            config, index, difficulty, row, &template, &moves,
        )?);
    }
    let norm = norm_score::score(
        &tasks,
        norm_score::BOOTSTRAP_RESAMPLES,
        norm_score::BOOTSTRAP_SEED,
    )
    .map_err(|error| fail(format!("norm refused: {error}")))?;
    let exchange = match guard {
        Some(byte) => probe_exchange(&duel, &template, strike, byte)
            .ok_or_else(|| fail("exchange probes refused: internal contract break".to_string()))?,
        None => ExchangeReach::default(),
    };
    Ok(TournamentReport {
        per_difficulty,
        spotlight,
        discovered_moves: moves,
        exchange,
        norm,
    })
}

/// Check a finished report against the P3 bars: 40–60% win rate per
/// difficulty (the win-rate verifier, reused), kill-share concentration
/// only when a real movelist exists (two or more distinct killing bytes —
/// the shipped rules have a single damaging answer, so single-byte
/// concentration is structural, while the spotlight below is the
/// overpowered-move detector), zero stuck fighters, and all three exchange
/// answers seen. Returns empty on a pass.
pub fn check(report: &TournamentReport, config: &TournamentConfig) -> Vec<Failure> {
    let mut out = Vec::new();
    for (index, row) in report.per_difficulty.iter().enumerate() {
        let balance = BalanceConfig {
            matches: row.matches,
            match_ticks: row.match_ticks,
            base_seed: config.difficulty_seed(index),
            min_win_permille: WIN_MIN_PERMILLE,
            max_win_permille: WIN_MAX_PERMILLE,
        };
        let win_report = winrates::Report {
            matches: row.matches,
            match_ticks: row.match_ticks,
            seeds: row.seeds.clone(),
            wins: row.wins,
            draws: row.draws,
            losses: row.losses,
            rate_permille: row.rate_permille,
        };
        for mut failure in winrates::check_report(&win_report, &balance) {
            failure.reason = format!("difficulty {}: {}", row.name, failure.reason);
            out.push(failure);
        }
        for share in &row.kill_shares {
            if row.kill_shares.len() >= 2 && share.share_permille > KILL_SHARE_MAX_PERMILLE {
                out.push(fail(format!(
                    "difficulty {}: move_byte={} takes {} permille of {} kills (cap {})",
                    row.name,
                    share.move_byte,
                    share.share_permille,
                    row.kills_total,
                    KILL_SHARE_MAX_PERMILLE,
                )));
            }
        }
        for (slot, tripped) in &row.stuck_trips {
            out.push(fail(format!(
                "difficulty {}: fighter {slot} stuck in {tripped} of {} matches ({}+ slow ticks)",
                row.name, row.matches, STUCK_WINDOW_TICKS,
            )));
        }
    }
    if !report.exchange.parry_seen {
        out.push(fail(
            "parry unreachable: no fresh-press negation observed".to_string(),
        ));
    }
    if !report.exchange.riposte_seen {
        out.push(fail(
            "riposte unreachable: no parry-then-answer observed".to_string(),
        ));
    }
    if !report.exchange.clash_seen {
        out.push(fail(
            "clash unreachable: no mutual full-damage impact observed".to_string(),
        ));
    }
    out
}

/// Check the move spotlight: any move killing above 35% of its spotlight
/// matches is overpowered and flagged for the move-table packets. The
/// strike lever is expected to fire (the fork decisive signal); that firing
/// is the seeded-overpowered proof, not a tournament failure.
pub fn check_spotlight(report: &TournamentReport) -> Vec<Failure> {
    let mut out = Vec::new();
    for spot in &report.spotlight {
        if spot.kill_rate_permille > KILL_SHARE_MAX_PERMILLE {
            out.push(fail(format!(
                "overpowered move_byte={}: {} kills in {} matches (rate {} permille, cap {})",
                spot.move_byte,
                spot.kills,
                spot.matches,
                spot.kill_rate_permille,
                KILL_SHARE_MAX_PERMILLE,
            )));
        }
    }
    out
}

/// Name under which the proof harness invokes this gate. The orchestrator
/// wires `tools/proof.mjs` to `engine_playtest::proof_gate` under this key;
/// the key lives here (playtest side) so the gate keeps its name even
/// before the proof crate grows the dependency edge.
pub const GATE_NAME: &str = "pq2-balance-tournament";

/// Proof-gate outcome: the full tournament report plus its P3-bar failures
/// from [`check`]. Spotlight overpowered flags ride inside
/// `report.spotlight` for the DESIGN move-table consumers and never fail
/// the gate (the strike lever is expected to fire).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GateOutcome {
    /// Full balance-tournament report (per-difficulty rows, spotlight,
    /// exchange reachability); `to_text()` is its byte-stable rendering.
    pub report: TournamentReport,
    /// P3 bars over the report (win rate, kill-share concentration, zero
    /// stuck, exchange reachability); empty exactly when the gate passes.
    pub failures: Vec<Failure>,
}

impl GateOutcome {
    /// True exactly when every P3 bar passed.
    pub fn pass(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Proof-gate entry point: run the standard tournament
/// ([`TournamentConfig::standard`]: 200 AI-vs-AI fork matches per
/// difficulty at 600 ticks) and check it against the P3 bars. Deterministic:
/// the same call twice gives the same report text. Cost is the standard
/// shape itself (600 tournament matches plus 60 spotlight matches at 600
/// ticks; about 19 s in a debug build on the dev machine). Errors only on
/// an internal contract break; the standard config always validates.
pub fn proof_gate() -> Result<GateOutcome, Failure> {
    let config = TournamentConfig::standard();
    let report = run(&config)?;
    let failures = check(&report, &config);
    Ok(GateOutcome { report, failures })
}

/// Difficulty-curve knob: the ONE [`BalanceConfig`](super::BalanceConfig)
/// field the sweep varies (`match_ticks`, fight length in fixed ticks).
/// Both configs carry it; longer fights let skill decide while short fights
/// compress to draws, so the curve proves the win-rate bands hold across
/// fight lengths instead of at one shape only.
#[cfg(test)]
const SWEEP_KNOB: &str = "match_ticks";
/// Standard sweep values: short, standard and long fights.
#[cfg(test)]
const SWEEP_VALUES: [u32; 3] = [300, 600, 1_200];
/// Upper bound on knob values per sweep (the knob cap): one short run each.
#[cfg(test)]
const SWEEP_MAX_VALUES: usize = 8;
/// Player label for the perfect tape in the curve CSV.
#[cfg(test)]
const SWEEP_PERFECT: &str = "perfect";
/// Player label for the human-error tape in the curve CSV.
#[cfg(test)]
const SWEEP_HUMAN: &str = "human";

/// Difficulty-curve sweep configuration: one 200-match tournament per knob
/// value, perfect player first, then the seeded human-like error when
/// enabled (default off). Owned by the enemy designer; the curve CSV is its
/// report to the balance log.
#[cfg(test)]
#[derive(Clone, PartialEq, Debug)]
struct SweepConfig {
    /// Base tournament; `matches` stays at the proof size, only the knob
    /// (`match_ticks`) varies per value.
    base: TournamentConfig,
    /// Knob values (`1..=MAX_MATCH_TICKS`, `1..=SWEEP_MAX_VALUES` entries).
    values: Vec<u32>,
    /// Seeded human-like error on slot zero; off is the perfect player.
    error: HumanError,
}

#[cfg(test)]
impl SweepConfig {
    /// Standard curve: the P3 proof shape at short, standard and long fight
    /// lengths with the perfect player.
    fn standard() -> Self {
        Self {
            base: TournamentConfig::standard(),
            values: SWEEP_VALUES.to_vec(),
            error: HumanError::off(),
        }
    }
}

/// One finished curve point: a difficulty's outcome at one knob value.
#[cfg(test)]
#[derive(Clone, PartialEq, Eq, Debug)]
struct SweepRow {
    /// Knob value (fight length in ticks).
    value: u32,
    /// Difficulty name, in config order.
    difficulty: String,
    /// True for the human-error replay, false for the perfect player.
    human: bool,
    /// Matches fighter 0 won outright.
    wins: u32,
    /// Matches tied for top damage.
    draws: u32,
    /// Matches another fighter won outright.
    losses: u32,
    /// `(2 * wins + draws) * 1000 / (2 * trials)`.
    rate_permille: u16,
    /// Kill attribution per observed killing byte, sorted by byte.
    kill_shares: Vec<KillShare>,
    /// Matches with a fighter slow 180+ ticks, summed over both slots.
    stuck_count: u32,
    /// Matches played (== the tournament size).
    trials: u32,
}

/// One skipped curve point: a variant nobody finished (zero kills), so it
/// carries no win-rate signal and the curve skips it instead of printing a
/// bloodless row as difficulty data.
#[cfg(test)]
#[derive(Clone, PartialEq, Eq, Debug)]
struct SweepSkip {
    /// Knob value (fight length in ticks).
    value: u32,
    /// Difficulty name, in config order.
    difficulty: String,
    /// Why the variant was skipped (always names the empty kill count).
    reason: String,
}

/// Whole difficulty curve: finished rows plus skips, in sweep order.
/// Deterministic: the same config twice gives the same report.
#[cfg(test)]
#[derive(Clone, PartialEq, Eq, Debug)]
struct SweepReport {
    /// Finished curve points, in `(value, difficulty, player)` order.
    rows: Vec<SweepRow>,
    /// Skipped variants, in `(value, difficulty)` order.
    skipped: Vec<SweepSkip>,
}

/// One curve row from a finished difficulty report. Pure.
#[cfg(test)]
fn sweep_row(value: u32, row: &DifficultyReport, human: bool) -> SweepRow {
    SweepRow {
        value,
        difficulty: row.name.clone(),
        human,
        wins: row.wins,
        draws: row.draws,
        losses: row.losses,
        rate_permille: row.rate_permille,
        kill_shares: row.kill_shares.clone(),
        stuck_count: row.stuck_trips.iter().map(|(_, n)| *n).sum(),
        trials: row.matches,
    }
}

/// Run the difficulty curve: per knob value the perfect-player tournament
/// first; a difficulty nobody finished (`kills_total == 0`) is skipped;
/// finished difficulties also replay under the seeded human error when
/// enabled. Discovery, spotlight and exchange probes stay perfect brains —
/// only duel rows take the error tape. Fails without simulating on a bad
/// knob list; per-value tournaments validate like [`run`].
#[cfg(test)]
fn sweep_difficulty_curve(sweep: &SweepConfig) -> Result<SweepReport, Failure> {
    if sweep.values.is_empty() || sweep.values.len() > SWEEP_MAX_VALUES {
        return Err(fail(format!(
            "invalid sweep values {}: must be 1..={SWEEP_MAX_VALUES}",
            sweep.values.len()
        )));
    }
    for value in &sweep.values {
        if *value == 0 || *value > MAX_MATCH_TICKS {
            return Err(fail(format!(
                "invalid sweep value {value}: must be 1..={MAX_MATCH_TICKS}"
            )));
        }
    }
    let mut rows = Vec::new();
    let mut skipped = Vec::new();
    for value in &sweep.values {
        let valued = TournamentConfig {
            match_ticks: *value,
            ..sweep.base.clone()
        };
        let perfect = run_with_error(&valued, HumanError::off())?;
        for (index, row) in perfect.per_difficulty.iter().enumerate() {
            if row.kills_total == 0 {
                skipped.push(SweepSkip {
                    value: *value,
                    difficulty: row.name.clone(),
                    reason: "no-finisher (kills_total=0)".to_string(),
                });
                continue;
            }
            rows.push(sweep_row(*value, row, false));
            if sweep.error.enabled {
                let difficulty = &valued.difficulties[index];
                let erred = run_difficulty(&valued, index, difficulty, sweep.error)?;
                rows.push(sweep_row(*value, &erred, true));
            }
        }
    }
    Ok(SweepReport { rows, skipped })
}

/// Render the curve as CSV: `knob,value,difficulty,player,win_rate_permille,
/// kill_shares,stuck_count,trials` data rows plus `# skipped ...` comment
/// lines. Kill shares print `byte:share` pairs joined by `;` (`none` when a
/// finished row drew no blood attribution). Pure and byte-stable: the same
/// report twice gives the same text (no clock inside; the UTC stamp names
/// the file only).
#[cfg(test)]
fn render_sweep_csv(report: &SweepReport) -> String {
    let mut out = String::from(
        "knob,value,difficulty,player,win_rate_permille,kill_shares,stuck_count,trials\n",
    );
    for row in &report.rows {
        let shares = if row.kill_shares.is_empty() {
            "none".to_string()
        } else {
            row.kill_shares
                .iter()
                .map(|share| format!("{}:{}", share.move_byte, share.share_permille))
                .collect::<Vec<_>>()
                .join(";")
        };
        out.push_str(&format!(
            "{SWEEP_KNOB},{},{},{},{},{},{},{}\n",
            row.value,
            row.difficulty,
            if row.human {
                SWEEP_HUMAN
            } else {
                SWEEP_PERFECT
            },
            row.rate_permille,
            shares,
            row.stuck_count,
            row.trials,
        ));
    }
    for skip in &report.skipped {
        out.push_str(&format!(
            "# skipped {SWEEP_KNOB}={} difficulty={} reason={}\n",
            skip.value, skip.difficulty, skip.reason,
        ));
    }
    out
}

/// UTC stamp `yyyymmddTHHMMZ` from unix seconds (integer civil-from-days,
/// Howard Hinnant's algorithm). The stamp names the `difficulty-<UTC>.csv`
/// file; it never enters the report or the CSV body, so the curve stays
/// replay-exact.
#[cfg(test)]
fn stamp_from_unix(secs: u64) -> String {
    let days = secs / 86_400;
    let secs_of_day = secs % 86_400;
    // Civil-from-days on unsigned math: the stamp is always post-1970, so
    // `z` stays non-negative and `/` plus `%` match the Euclidean ops.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}{:02}{:02}T{:02}{:02}Z",
        year,
        m,
        d,
        secs_of_day / 3_600,
        (secs_of_day % 3_600) / 60,
    )
}

/// Current UTC stamp for one sweep filename (wall clock names the file only).
#[cfg(test)]
fn utc_stamp_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    stamp_from_unix(secs)
}

/// Filename for one sweep run: `difficulty-<UTC>.csv`.
#[cfg(test)]
fn sweep_csv_name(stamp: &str) -> String {
    format!("difficulty-{stamp}.csv")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_arena::fork::{TIER_DUELS, WIN_BAR_PERMILLE};

    fn tiny_config() -> TournamentConfig {
        TournamentConfig {
            base_seed: 7001,
            matches: 3,
            match_ticks: 60,
            half_extent_m: 20.0,
            difficulties: vec![Difficulty::even()],
            spotlight_matches: 2,
            spotlight_ticks: 600,
        }
    }

    fn duel_config() -> ArenaConfig {
        ArenaConfig {
            seed: 7001,
            ticks: DISCOVERY_TICKS,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        }
    }

    #[test]
    fn standard_shape_matches_the_p3_proof() {
        let config = TournamentConfig::standard();
        assert_eq!(config.matches, MATCHES_PER_DIFFICULTY);
        assert_eq!(config.matches, MAX_MATCHES);
        assert_eq!(config.match_ticks, STANDARD_TICKS);
        assert_eq!(config.difficulties.len(), 3);
        assert_eq!(config.spotlight_matches, SPOTLIGHT_MATCHES);
        assert_eq!(config.spotlight_ticks, SPOTLIGHT_TICKS);
        assert!(validate(&config).is_ok());
        // Difficulty seeds partition without overlap (gap above MAX_MATCHES).
        let first: Vec<u64> = (0..3).map(|d| config.difficulty_seed(d)).collect();
        assert_eq!(first, vec![2040, 2040 + SEED_GAP, 2040 + 2 * SEED_GAP]);
    }

    #[test]
    fn invalid_configs_fail_without_simulating() {
        let good = tiny_config();
        let mut bad = good.clone();
        bad.matches = 0;
        assert!(run(&bad).is_err());
        let mut bad = good.clone();
        bad.matches = MAX_MATCHES + 1;
        assert!(run(&bad).is_err());
        let mut bad = good.clone();
        bad.match_ticks = 0;
        assert!(run(&bad).is_err());
        let mut bad = good.clone();
        bad.spotlight_matches = 0;
        assert!(run(&bad).is_err());
        let mut bad = good.clone();
        bad.difficulties = Vec::new();
        assert!(run(&bad).is_err());
        let mut bad = good.clone();
        bad.difficulties = vec![Difficulty::even(); MAX_DIFFICULTIES + 1];
        assert!(run(&bad).is_err());
        let mut bad = good.clone();
        bad.difficulties[0].morale = vec![500];
        assert!(run(&bad).is_err());
        let mut bad = good.clone();
        bad.difficulties[0].morale = vec![500, 1001];
        assert!(run(&bad).is_err());
        let mut bad = good.clone();
        bad.half_extent_m = 200.0;
        assert!(run(&bad).is_err());
        // Rejections are verifier failures, never panics.
        for variant in [&bad, &good] {
            let _ = validate(variant);
        }
    }

    #[test]
    fn strike_discovery_finds_the_decisive_lever() {
        let config = duel_config();
        let template = template_input(&config).expect("valid config records");
        let strike = discover_strike(&config, &template).expect("a strike byte exists");
        // Spam into a frozen foe lands and kills, deterministically twice.
        let first = spam_into_frozen(&config, &template, strike, 600).expect("fork runs");
        let second = spam_into_frozen(&config, &template, strike, 600).expect("fork replays");
        assert!(first > 0, "strike spam must land damage");
        assert_eq!(first, second);
        let spam = spam_tape(&template, 0, 600, strike).expect("tape builds");
        let probe = ArenaConfig {
            ticks: 600,
            ..config
        };
        let (snapshot, _) = engine_arena::run_prefix(&probe, &spam, 0).expect("prefix builds");
        let mut fork = Fork::from_snapshot(snapshot, 0).expect("fork builds");
        fork.apply_proposal(ForkProposal::SwapTape {
            fighter: 0,
            inputs: spam.0.clone(),
        })
        .expect("spam validates");
        fork.apply_proposal(ForkProposal::FreezeFighter { fighter: 1 })
            .expect("foe freezes");
        let metrics = fork.run_n(600);
        assert!(
            metrics.ticks_to_kill.is_some(),
            "relentless spam kills: {metrics:?}"
        );
        assert_eq!(metrics, fork.run_n(600));
    }

    #[test]
    fn simultaneous_strikes_resolve_id_ordered() {
        // Both fighters strike off tick 0: hit-react cancels the victim's
        // in-flight strike, so exactly fighter 0 lands — replay-exact on
        // every seed, never a double hit, never a dropped exchange.
        let config = duel_config();
        let template = template_input(&config).expect("valid config records");
        let strike = discover_strike(&config, &template).expect("strike exists");
        let run_duel = || {
            let probe = ArenaConfig {
                ticks: EXCHANGE_TICKS,
                ..config
            };
            let duel = merge_tapes(&[
                single_tape(&template, 0, probe.ticks, 0, strike).expect("tape"),
                single_tape(&template, 1, probe.ticks, 0, strike).expect("tape"),
            ]);
            fork_metrics(&probe, &[500, 500], &duel).expect("fork runs")
        };
        let (first, second) = (run_duel(), run_duel());
        assert_eq!(first, second);
        // Structural pin (no restated table constants): exactly the lower
        // ID lands, the victim's strike cancels through hit-react.
        let dealt = |m: &ForkMetrics, slot: u8| {
            m.damage_dealt
                .iter()
                .find(|(s, _)| *s == slot)
                .map(|(_, d)| *d)
                .unwrap_or(0)
        };
        assert!(dealt(&first, 0) > 0);
        assert_eq!(dealt(&first, 1), 0);
    }

    #[test]
    fn guard_discovery_and_parry_edge_hold() {
        let config = duel_config();
        let template = template_input(&config).expect("valid config records");
        let strike = discover_strike(&config, &template).expect("strike exists");
        let frozen_full =
            spam_into_frozen(&config, &template, strike, DISCOVERY_TICKS).expect("reference runs");
        assert!(frozen_full > 0);
        let (guard, _) = discover_answers(&config, &template, strike, frozen_full);
        let guard = guard.expect("a guard byte exists");
        assert_ne!(guard, strike);
        // Fresh press at tick 4 negates; a press at tick 5 eats the hit.
        let probe = ArenaConfig {
            ticks: EXCHANGE_TICKS,
            ..config
        };
        let morale = [500, 500];
        let early = merge_tapes(&[
            single_tape(&template, 0, probe.ticks, 0, strike).expect("tape"),
            single_tape(&template, 1, probe.ticks, 4, guard).expect("tape"),
        ]);
        let late = merge_tapes(&[
            single_tape(&template, 0, probe.ticks, 0, strike).expect("tape"),
            single_tape(&template, 1, probe.ticks, 5, guard).expect("tape"),
        ]);
        let early_dmg = fork_metrics(&probe, &morale, &early)
            .expect("fork runs")
            .damage_dealt[0]
            .1;
        let late_dmg = fork_metrics(&probe, &morale, &late)
            .expect("fork runs")
            .damage_dealt[0]
            .1;
        assert_eq!(early_dmg, 0, "fresh press must negate");
        assert!(late_dmg > 0, "late press must eat the hit");
    }

    #[test]
    fn tiny_tournament_repeats_exactly_with_numbers() {
        let config = tiny_config();
        let first = run(&config).expect("tiny config runs");
        let second = run(&config).expect("tiny config replays");
        assert_eq!(first, second);
        assert_eq!(first.to_text(), second.to_text());
        assert_eq!(first.per_difficulty.len(), 1);
        let row = &first.per_difficulty[0];
        assert_eq!(row.matches, 3);
        assert_eq!(row.seeds, vec![7001, 7002, 7003]);
        assert_eq!(row.wins + row.draws + row.losses, 3);
        assert!(!first.discovered_moves.is_empty());
        assert!(!first.spotlight.is_empty());
        println!("{}", first.to_text());
    }

    #[test]
    fn norm_line_rides_the_tournament_report() {
        // Real path: run the tournament, read the printed report text.
        let report = run(&tiny_config()).expect("tiny config runs");
        let text = report.to_text();
        let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("norm=")).collect();
        assert_eq!(lines.len(), 1, "one norm line per agent: {text}");
        assert_eq!(lines[0], report.norm.line());
        let norm = &report.norm;
        assert!(norm.ci_lo <= norm.permille && norm.permille <= norm.ci_hi);
        assert_eq!(norm.resamples, norm_score::BOOTSTRAP_RESAMPLES);
        assert_eq!(norm.tasks.len(), 1);
        assert_eq!(norm.tasks[0].name, "even");
        let anchored = report
            .discovered_moves
            .iter()
            .any(|byte| norm.tasks[0].anchor == format!("spam:{byte}"));
        assert!(anchored, "anchor must be a discovered move: {text}");
        assert_eq!(report.per_difficulty[0].agent_damage.len(), 3);
        assert!(lines[0].contains(" anchor=even=spam:"), "{}", lines[0]);
    }

    #[test]
    fn zones_shares_sum_to_1000_and_entrance_stays_under_250() {
        // GP36: one metric over every recorded fighter tick. The gate anchor
        // is data (content/design/arena-zones.rows, gate_tunnel at 12 m on
        // the centre lane). The four shares always sum to 1000 permille and
        // the gate *time* share stays under the 250 bar; the record itself
        // does not move.
        let config = tiny_config();
        let first = run(&config).expect("tiny config runs");
        let second = run(&config).expect("tiny config replays");
        assert_eq!(first, second, "the zone tally must not disturb the record");
        assert_eq!(first.to_text(), second.to_text());
        assert!(!first.per_difficulty.is_empty());
        for row in &first.per_difficulty {
            let sum: u32 = row.zone_permille.iter().sum();
            assert_eq!(sum, 1000, "zone shares must sum to 1000: {row:?}");
            assert_eq!(row.entrance_share, row.zone_permille[3]);
            assert!(
                row.entrance_share < 250,
                "GP36 bar: entrance under 250 permille, got {}",
                row.entrance_share
            );
            let zone_kills: u32 = row.kills_per_zone.iter().sum();
            assert_eq!(zone_kills, row.kills_total, "every kill lands in one zone");
        }
        let text = first.to_text();
        assert!(text.contains("zone share centre="), "{text}");
        assert!(text.contains("entrance_share="), "{text}");
        assert!(text.contains("kills_per_zone=["), "{text}");
        let elo_rows = text.matches("  elo fighter0=").count();
        assert_eq!(
            elo_rows,
            first.per_difficulty.len(),
            "one PQ9 row per difficulty: {text}"
        );
        println!("{text}");
    }

    #[test]
    fn seeded_overpowered_move_is_flagged() {
        // The discovered strike lever spammed into brains must trip the
        // spotlight cap: this is the seeded-overpowered proof.
        let config = tiny_config();
        let report = run(&config).expect("tiny config runs");
        let strike = report.discovered_moves[0];
        let spot = report
            .spotlight
            .iter()
            .find(|s| s.move_byte == strike)
            .expect("strike has a spotlight row");
        assert!(
            spot.kill_rate_permille > KILL_SHARE_MAX_PERMILLE,
            "strike must flag as overpowered: {spot:?}"
        );
        let flags = check_spotlight(&report);
        assert!(
            flags.iter().any(|f| f.verifier == VERIFIER
                && f.reason.contains("overpowered")
                && f.reason.contains(&strike.to_string())),
            "flag must quote the seeded byte: {flags:?}"
        );
        // Replay flags the same byte.
        let again = run(&config).expect("replays");
        assert_eq!(check_spotlight(&again), flags);
    }

    #[test]
    fn wall_grind_mechanism_pinned() {
        // Wall-pinned fighter 1 grinding motionless with live wishes into
        // the wall clamp. Mechanism: DepthBrain steering re-aims off the blocked axis and
        // wall-escapes, so the fighter clears the wall or lands a kill
        // instead of grinding. STUCK_WINDOW_TICKS untouched (180).
        let cfg = ArenaConfig {
            seed: 2043,
            ticks: 600,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        };
        let Some((metrics, dataset)) =
            fork_recorded(&cfg, &[500, 500], None, &FighterTape::default())
        else {
            panic!("seed 2043 must fork");
        };
        let kill_tick = metrics.ticks_to_kill.map(|kill| kill.saturating_sub(1));
        assert_eq!(
            scan_stuck_consecutive(&dataset, 2, 600, kill_tick),
            Vec::<u8>::new(),
            "GA3: no wall grind on seed 2043"
        );
        // The fighter is alive and either kills or clears the wall (was:
        // pinned within a meter of the east wall for the whole back half).
        let mut live_wish = false;
        for tick in (200..380).step_by(20) {
            let input = dataset.rows[tick as usize * 2 + 1].action.input;
            live_wish = live_wish || input.mx != 0 || input.mz != 0;
        }
        assert!(live_wish, "escaped brain must keep wishing");
        let start = dataset.rows[300 * 2 + 1].observation.fighters[1].pos;
        let end = dataset.rows[599 * 2 + 1].observation.fighters[1].pos;
        let moved = ((end[0] - start[0]).powi(2) + (end[2] - start[2]).powi(2)).sqrt();
        assert!(
            metrics.ticks_to_kill.is_some() || moved > 1.0 || wall_min_m(&dataset, 1) > 1.0,
            "GA3: kill, motion or wall clearance on seed 2043"
        );
    }

    /// Replay one 600-tick duel exactly as the tournament does (empty tape,
    /// per-slot morale) and return its metrics plus recorded rows.
    fn replay_duel(seed: u64, morale: &[u16]) -> (ForkMetrics, Dataset) {
        let cfg = ArenaConfig {
            seed,
            ticks: 600,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        };
        fork_recorded(&cfg, morale, None, &FighterTape::default())
            .expect("stuck-leg seed must fork")
    }

    /// Ticks where `slot` issues a nonzero move wish (a live brain).
    fn live_wish_ticks(dataset: &Dataset, slot: usize) -> u32 {
        (0..600u32)
            .filter(|tick| {
                let input = dataset.rows[*tick as usize * 2 + slot].action.input;
                input.mx != 0 || input.mz != 0
            })
            .count() as u32
    }

    /// Closest any pose of `slot` comes to the arena wall, in meters.
    fn wall_min_m(dataset: &Dataset, slot: usize) -> f32 {
        (0..600u32)
            .map(|tick| {
                let pos = dataset.rows[tick as usize * 2 + slot].observation.fighters[slot].pos;
                20.0 - pos[0].abs().max(pos[2].abs())
            })
            .fold(f32::MAX, f32::min)
    }

    #[test]
    fn stuck_leg_even_slot0_drift_to_wall_grind() {
        // Drifted fighter 0 to the east wall on a frozen z lane, grounding
        // 0-0. Mechanism: re-aiming off the blocked axis fixes drift grinds; the
        // foe needs no change. STUCK_WINDOW_TICKS untouched (180).
        let (metrics, dataset) = replay_duel(2047, &[500, 500]);
        let kill_tick = metrics.ticks_to_kill.map(|kill| kill.saturating_sub(1));
        let tripped = scan_stuck_consecutive(&dataset, 2, 600, kill_tick);
        assert!(
            tripped.is_empty(),
            "GA3: no drift grind on seed 2047: {tripped:?}"
        );
        let lane0 = dataset.rows[0].observation.fighters[0].pos;
        let end0 = dataset.rows[599 * 2].observation.fighters[0].pos;
        let moved = ((end0[0] - lane0[0]).powi(2) + (end0[2] - lane0[2]).powi(2)).sqrt();
        assert!(
            metrics.ticks_to_kill.is_some() || moved > 1.0,
            "GA3: seed 2047 resolves by kill or motion"
        );
        assert!(
            live_wish_ticks(&dataset, 0) >= 500,
            "escaped brain must keep wishing"
        );
    }

    #[test]
    fn stuck_leg_pressure_slot0_grind_under_split_morale() {
        // Parked fighter 0 at the east wall 0-0 under split morale.
        // Mechanism: the morale split only changes pacing pressure, never
        // changes pacing pressure, never the blocked-axis wish — steering
        // fixes the wish itself, so pressure grinds clear with it.
        // STUCK_WINDOW_TICKS untouched (180).
        let (metrics, dataset) = replay_duel(1_002_065, &[700, 300]);
        let kill_tick = metrics.ticks_to_kill.map(|kill| kill.saturating_sub(1));
        let tripped = scan_stuck_consecutive(&dataset, 2, 600, kill_tick);
        assert!(
            !tripped.contains(&0),
            "GA3: fighter 0 must not grind under split morale: {tripped:?}"
        );
        assert!(
            live_wish_ticks(&dataset, 0) >= 500,
            "escaped brain must keep wishing"
        );
    }

    #[test]
    fn stuck_leg_pressure_slot1_mirror_grind() {
        // Low morale wishing into the wall on both extremes.
        // Mechanism: low morale still wished into the wall; one shared
        // wall-escape wish covers both morale extremes at once.
        // STUCK_WINDOW_TICKS untouched (180).
        let (metrics, dataset) = replay_duel(1_002_065, &[700, 300]);
        let kill_tick = metrics.ticks_to_kill.map(|kill| kill.saturating_sub(1));
        let tripped = scan_stuck_consecutive(&dataset, 2, 600, kill_tick);
        assert!(
            !tripped.contains(&1),
            "GA3: fighter 1 must not grind under split morale: {tripped:?}"
        );
        assert!(
            live_wish_ticks(&dataset, 1) >= 500,
            "escaped brain must keep wishing"
        );
    }

    #[test]
    fn stuck_leg_cauldron_slot0_spawn_pin() {
        // Spawn-pinned fighter 0 at x = 19.26 with the foe never
        // approaching. Mechanism: steering adds the opening-tick wall-escape plus the
        // passive foe's closing wish, so the pin breaks from both sides.
        // STUCK_WINDOW_TICKS untouched (180).
        let (metrics, dataset) = replay_duel(2_002_056, &[1000, 0]);
        let kill_tick = metrics.ticks_to_kill.map(|kill| kill.saturating_sub(1));
        let tripped = scan_stuck_consecutive(&dataset, 2, 600, kill_tick);
        assert_eq!(
            tripped,
            Vec::<u8>::new(),
            "GA3: no spawn pin on seed 2002056: {tripped:?}"
        );
        assert!(
            live_wish_ticks(&dataset, 0) >= 500,
            "escaped brain must keep wishing"
        );
        let spawn0 = dataset.rows[0].observation.fighters[0].pos;
        assert!(
            spawn0[0] > 19.0,
            "spawn still sits against the wall (placement unchanged): x={}",
            spawn0[0]
        );
        let end0 = dataset.rows[599 * 2].observation.fighters[0].pos;
        let fled = ((end0[0] - spawn0[0]).powi(2) + (end0[2] - spawn0[2]).powi(2)).sqrt();
        assert!(
            wall_min_m(&dataset, 0) > 1.0 || metrics.ticks_to_kill.is_some() || fled > 1.0,
            "GA3: the pin breaks by clearance, kill or flight (fled={fled:.2})"
        );
        let foe_mean: f32 = (0..600u32)
            .map(|tick| {
                let row = &dataset.rows[tick as usize * 2];
                let (p, q) = (
                    row.observation.fighters[0].pos,
                    row.observation.fighters[1].pos,
                );
                let (dx, dz) = (q[0] - p[0], q[2] - p[2]);
                (dx * dx + dz * dz).sqrt()
            })
            .sum::<f32>()
            / 600.0;
        // The 0-morale foe keeps range (7.32, was 9.30 pinned) rather than
        // closing: passivity holding distance is sane; full closing wishes
        // are follow-up, not this row. Bar guards the improvement direction.
        assert!(
            foe_mean < 9.0 || metrics.ticks_to_kill.is_some(),
            "GA3: foe no farther than the pinned 9.30: {foe_mean}"
        );
    }

    /// The two gate rows the shipped tier ladder posts today: even and
    /// cauldron kill a second move byte rarely, so byte 1 holds a dominant
    /// share of a two-move movelist. Win rates and stuck trips are clean.
    /// One table owns the strings so both pins agree.
    fn ga3_gate_rows() -> [&'static str; 2] {
        [
            "difficulty even: move_byte=1 takes 895 permille of 48 kills (cap 350)",
            "difficulty cauldron: move_byte=1 takes 833 permille of 24 kills (cap 350)",
        ]
    }

    #[test]
    fn proof_gate_reproduces_byte_identical_with_five_named_stuck_legs() {
        // The five named legs stay clear (see the five tests above) and the
        // full tournament replays byte-identical x2. At the shipped tier
        // ladder every difficulty sits inside the 400-600 band with zero
        // stuck trips; the only rows the gate still posts are the two
        // kill-share concentrations in `ga3_gate_rows`, for the move-table
        // owner to clear.
        // The proof harness calls this entry point by name (GATE_NAME)
        // through the crate-public API only — no test hooks, no private
        // helpers. If the gate were unwired (entry removed or renamed), this
        // test fails to build; if the report stops reproducing, the text
        // comparison fails.
        assert_eq!(GATE_NAME, "pq2-balance-tournament");
        let first = proof_gate().expect("standard gate must run");
        let second = proof_gate().expect("standard gate must replay");
        assert_eq!(first, second);
        assert_eq!(first.report.to_text(), second.report.to_text());
        assert!(
            !first.pass(),
            "gate posts the known edge: {:?}",
            first.failures
        );
        let rows = ga3_gate_rows();
        assert_eq!(
            first.failures.len(),
            rows.len(),
            "GA3: known kill-share rows: {:?}",
            first.failures
        );
        for needle in rows {
            assert!(
                first.failures.iter().any(|f| f.reason.contains(needle)),
                "missing gate row {needle}: {:?}",
                first.failures
            );
        }
        println!("{}", first.report.to_text());
    }

    #[test]
    fn tiny_bloodless_tournament_passes_the_p3_bars() {
        // Three 60-tick brain bouts draw 0-0: win rate centers, no kills to
        // share, nobody sits still 180 ticks, exchange answers all land.
        let config = tiny_config();
        let report = run(&config).expect("tiny config runs");
        assert!(report.exchange.parry_seen, "parry must answer");
        assert!(report.exchange.riposte_seen, "riposte must answer");
        assert!(report.exchange.clash_seen, "clash must answer");
        let failures = check(&report, &config);
        assert!(failures.is_empty(), "tiny proof must pass: {failures:?}");
    }

    #[test]
    fn kill_share_flags_only_multi_move_concentration() {
        // Synthetic rows: single-byte concentration (the shipped
        // single-strike ruleset) never flags; a two-move split with one
        // move at 900 permille does.
        let row = |shares: Vec<KillShare>| DifficultyReport {
            name: "synthetic".to_string(),
            matches: 10,
            match_ticks: 60,
            seeds: (0..10).collect(),
            wins: 5,
            draws: 0,
            losses: 5,
            rate_permille: 500,
            uncertainty_permille: 0,
            kills_total: 10,
            kill_shares: shares,
            stuck_trips: Vec::new(),
            stuck_seeds: Vec::new(),
            stuck_ticks_total: 0,
            zone_permille: [0; 4],
            entrance_share: 0,
            kills_per_zone: [0; 4],
            agent_damage: vec![0; 10],
        };
        let config = tiny_config();
        let single = TournamentReport {
            per_difficulty: vec![row(vec![KillShare {
                move_byte: 1,
                kills: 10,
                share_permille: 1000,
            }])],
            spotlight: Vec::new(),
            discovered_moves: vec![1],
            exchange: ExchangeReach {
                parry_seen: true,
                riposte_seen: true,
                clash_seen: true,
            },
            norm: NormScore {
                permille: 0,
                ci_lo: 0,
                ci_hi: 0,
                resamples: 0,
                tasks: Vec::new(),
            },
        };
        assert!(
            check(&single, &config)
                .iter()
                .all(|f| !f.reason.contains("takes")),
            "single-move concentration is structural: {:?}",
            check(&single, &config)
        );
        let split = TournamentReport {
            per_difficulty: vec![row(vec![
                KillShare {
                    move_byte: 1,
                    kills: 9,
                    share_permille: 900,
                },
                KillShare {
                    move_byte: 2,
                    kills: 1,
                    share_permille: 100,
                },
            ])],
            spotlight: Vec::new(),
            discovered_moves: vec![1, 2],
            exchange: ExchangeReach {
                parry_seen: true,
                riposte_seen: true,
                clash_seen: true,
            },
            norm: NormScore {
                permille: 0,
                ci_lo: 0,
                ci_hi: 0,
                resamples: 0,
                tasks: Vec::new(),
            },
        };
        let flags = check(&split, &config);
        assert!(
            flags
                .iter()
                .any(|f| f.reason.contains("move_byte=1 takes 900")),
            "dominant move in a movelist must flag: {flags:?}"
        );
    }

    #[test]
    fn report_text_prints_the_c1_charter_verdict_from_the_parry_probe() {
        let report = |parry_seen: bool| TournamentReport {
            per_difficulty: Vec::new(),
            spotlight: Vec::new(),
            discovered_moves: Vec::new(),
            exchange: ExchangeReach {
                parry_seen,
                riposte_seen: false,
                clash_seen: false,
            },
            norm: NormScore {
                permille: 0,
                ci_lo: 0,
                ci_hi: 0,
                resamples: 0,
                tasks: Vec::new(),
            },
        };
        assert!(report(true)
            .to_text()
            .contains("charter_c1 PASS c1: parry landed in first bout\n"));
        assert!(report(false)
            .to_text()
            .contains("charter_c1 FAIL c1: no parry in first bout (sev B)\n"));
    }

    #[test]
    fn p3_standard_posts_numbers_with_known_stuck_edge() {
        // Full P3 proof with steering brains — 200 AI-vs-AI fork matches
        // per difficulty. Determinism itself is proven by
        // `tiny_tournament_repeats_exactly`; this test posts the bars.
        // Golden receipt (same-machine scope per crate docs): at the shipped
        // tier ladder (`run_difficulty` installs `SHIPPED_TIERS[index]`) the
        // rates measure 490 / 482 / 472 permille, all inside the 400-600 band,
        // no difficulty posts a stuck trip, and the exchange answers all
        // land. Only the two kill-share rows in `ga3_gate_rows` trip the gate;
        // STUCK_WINDOW_TICKS untouched, zero sim change here. These numbers
        // moved off the old 997 / 2 / 507 receipt (191+191 and 181+181 stuck)
        // because the ladder's per-rung temperament now reaches a duel.
        let config = TournamentConfig::standard();
        let report = run(&config).expect("standard config runs");
        assert_eq!(report.per_difficulty.len(), 3);
        for row in &report.per_difficulty {
            assert_eq!(row.matches, MATCHES_PER_DIFFICULTY);
            assert_eq!(row.wins + row.draws + row.losses, MATCHES_PER_DIFFICULTY);
        }
        println!("{}", report.to_text());
        let even = &report.per_difficulty[0];
        let pressure = &report.per_difficulty[1];
        let cauldron = &report.per_difficulty[2];
        for (row, label) in [
            (even, "even"),
            (pressure, "pressure"),
            (cauldron, "cauldron"),
        ] {
            assert!(
                row.stuck_trips.is_empty() && row.stuck_seeds.is_empty(),
                "GA3: {label} clean: {:?} {:?}",
                row.stuck_trips,
                row.stuck_seeds
            );
        }
        // Golden receipt (DepthBrain + steering + tier ladder, same-machine
        // scope): rates 490/482/472, kills 48/22/24, exchange answers all land.
        assert_eq!(even.rate_permille, 490, "GA3 even rate");
        assert_eq!(pressure.rate_permille, 482, "GA3 pressure rate");
        assert_eq!(cauldron.rate_permille, 472, "GA3 cauldron rate");
        assert_eq!(even.kills_total, 48, "even kills");
        assert_eq!(pressure.kills_total, 22, "GA3 pressure kills");
        assert_eq!(cauldron.kills_total, 24, "GA3 cauldron kills");
        assert!(report.exchange.parry_seen);
        assert!(report.exchange.riposte_seen);
        assert!(report.exchange.clash_seen);
        // The gate posts exactly the two kill-share rows pinned in
        // `ga3_gate_rows` (byte-identical x2 below pins it).
        let failures = check(&report, &config);
        let rows = ga3_gate_rows();
        assert_eq!(
            failures.len(),
            rows.len(),
            "GA3: gate posts only the kill-share rows: {failures:?}"
        );
        for needle in rows {
            assert!(
                failures.iter().any(|f| f.reason.contains(needle)),
                "missing gate row {needle}: {failures:?}"
            );
        }
    }

    fn tiny_sweep(values: Vec<u32>, error: HumanError) -> SweepConfig {
        SweepConfig {
            base: tiny_config(),
            values,
            error,
        }
    }

    #[test]
    fn sweep_rejects_bad_knob_lists_without_simulating() {
        let good = tiny_sweep(vec![60], HumanError::off());
        assert!(sweep_difficulty_curve(&good).is_ok());
        let mut bad = tiny_sweep(Vec::new(), HumanError::off());
        assert!(sweep_difficulty_curve(&bad).is_err());
        bad = tiny_sweep(vec![60; SWEEP_MAX_VALUES + 1], HumanError::off());
        assert!(sweep_difficulty_curve(&bad).is_err());
        bad = tiny_sweep(vec![0], HumanError::off());
        assert!(sweep_difficulty_curve(&bad).is_err());
        bad = tiny_sweep(vec![MAX_MATCH_TICKS + 1], HumanError::off());
        assert!(sweep_difficulty_curve(&bad).is_err());
        // Rejections are verifier failures, never panics.
        for variant in [&bad, &good] {
            let _ = sweep_difficulty_curve(variant).is_ok();
        }
    }

    #[test]
    fn sweep_skips_bloodless_variants_and_replays_its_csv() {
        // One tick cannot finish anyone: the variant is skipped, the CSV
        // carries the skip line, and the same seed twice gives the same CSV.
        let sweep = tiny_sweep(vec![1], HumanError::off());
        let first = sweep_difficulty_curve(&sweep).expect("one-tick sweep runs");
        assert!(first.rows.is_empty(), "nothing finishes in one tick");
        assert_eq!(first.skipped.len(), 1);
        assert_eq!(first.skipped[0].value, 1);
        assert_eq!(first.skipped[0].difficulty, "even");
        let second = sweep_difficulty_curve(&sweep).expect("one-tick sweep replays");
        assert_eq!(first, second);
        let csv = render_sweep_csv(&first);
        assert_eq!(csv, render_sweep_csv(&second));
        let mut lines = csv.lines();
        assert_eq!(
            lines.next(),
            Some("knob,value,difficulty,player,win_rate_permille,kill_shares,stuck_count,trials"),
            "header names every CSV column",
        );
        assert!(lines
            .next()
            .unwrap_or("")
            .starts_with("# skipped match_ticks=1"));
        assert_eq!(csv.lines().count(), 2, "header plus one skip line");
        println!("{csv}");
    }

    #[test]
    fn sweep_rows_cover_every_difficulty_at_each_value() {
        let sweep = tiny_sweep(vec![60, 120], HumanError::off());
        let report = sweep_difficulty_curve(&sweep).expect("tiny sweep runs");
        // Every (value, difficulty) lands in exactly one of rows/skipped.
        assert_eq!(
            report.rows.len() + report.skipped.len(),
            2 * sweep.base.difficulties.len(),
        );
        for row in &report.rows {
            assert!(sweep.values.contains(&row.value));
            assert_eq!(row.trials, sweep.base.matches);
            assert_eq!(row.wins + row.draws + row.losses, row.trials);
        }
        let again = sweep_difficulty_curve(&sweep).expect("tiny sweep replays");
        assert_eq!(render_sweep_csv(&report), render_sweep_csv(&again));
    }

    #[test]
    fn error_path_replays_and_matches_perfect_when_off() {
        let config = tiny_config();
        let perfect = run(&config).expect("tiny config runs");
        let replayed = run_with_error(&config, HumanError::off()).expect("off replays run");
        assert_eq!(perfect, replayed, "disabled error is the perfect player");
        let error = HumanError::human(11);
        let first = run_with_error(&config, error).expect("human error runs");
        let second = run_with_error(&config, error).expect("human error replays");
        assert_eq!(first, second, "same error seed twice gives the same report");
        assert_eq!(first.per_difficulty.len(), config.difficulties.len());
    }

    #[test]
    fn sweep_stamp_pins_epoch_and_names_csv_files() {
        assert_eq!(stamp_from_unix(0), "19700101T0000Z");
        let stamp = utc_stamp_now();
        assert_eq!(stamp.len(), 14, "yyyymmddTHHMMZ: {stamp}");
        assert!(stamp.ends_with('Z') && stamp.contains('T'), "{stamp}");
        assert_eq!(sweep_csv_name(&stamp), format!("difficulty-{stamp}.csv"),);
    }

    #[test]
    #[ignore = "full 200-match difficulty curve: run alone with --ignored"]
    fn full_difficulty_sweep_writes_csv() {
        // Full 200-match curve (short, standard, long fights): run
        // explicitly to refresh `target/reports/difficulty-<UTC>.csv`.
        // Same-seed determinism is proven by the tiny sweep tests above
        // (each runs twice and compares CSV text); this run plants the
        // full-shape artifact. Run it twice and diff the two bodies to
        // prove the full curve replays byte-identical (stamps name the
        // files only, never enter the CSV body).
        let sweep = SweepConfig::standard();
        assert_eq!(sweep.values, SWEEP_VALUES.to_vec());
        let first = sweep_difficulty_curve(&sweep).expect("standard sweep runs");
        let csv = render_sweep_csv(&first);
        assert_eq!(
            first.rows.len() + first.skipped.len(),
            SWEEP_VALUES.len() * sweep.base.difficulties.len(),
            "every value x difficulty lands in rows or skips",
        );
        assert!(
            !first.rows.is_empty(),
            "the curve must finish somewhere: {:?}",
            first.skipped
        );
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/reports");
        std::fs::create_dir_all(&dir).expect("reports dir builds");
        let path = dir.join(sweep_csv_name(&utc_stamp_now()));
        std::fs::write(&path, &csv).expect("CSV writes");
        let data_rows = csv
            .lines()
            .filter(|line| !line.starts_with('#') && !line.starts_with("knob,"))
            .count();
        assert_eq!(data_rows, first.rows.len());
        println!("wrote {} with {data_rows} data rows", path.display());
    }

    /// One adjacent tier pair over [`TIER_DUELS`] seeded duels at the
    /// standard match length: slot 0 holds `higher`, slot 1 `lower`, morale
    /// even. `Ok(permille)` when the higher rung clears
    /// [`WIN_BAR_PERMILLE`]; `Err` naming the pair when it does not (the
    /// inverted-install failure case).
    fn tier_pair_verdict(higher: TierRow, lower: TierRow) -> Result<u16, String> {
        let base = TournamentConfig::standard().base_seed;
        let mut wins = 0u32;
        for i in 0..TIER_DUELS {
            let cfg = ArenaConfig {
                seed: base.wrapping_add(u64::from(i)),
                ticks: STANDARD_TICKS,
                fighters: 2,
                half_extent_m: 20.0,
                table_version: 1,
            };
            let (snapshot, _) = engine_arena::run_prefix(&cfg, &FighterTape::default(), 0)
                .ok_or("tier pair prefix refused")?;
            let mut fork = Fork::from_snapshot(snapshot, 0).ok_or("tier pair fork refused")?;
            for (slot, tier) in [higher, lower].iter().enumerate() {
                let fighter = u8::try_from(slot).map_err(|_| "tier pair slot overflow")?;
                fork.apply_proposal(ForkProposal::SetTier {
                    fighter,
                    tier: *tier,
                })
                .map_err(|error| format!("tier pair slot {slot} refused: {error:?}"))?;
            }
            if matches!(
                winrates::slot_zero_result(&fork.run_n(cfg.ticks).damage_dealt),
                winrates::SlotZero::Win
            ) {
                wins += 1;
            }
        }
        let permille = u16::try_from(wins * 1000 / TIER_DUELS).unwrap_or(u16::MAX);
        if permille < WIN_BAR_PERMILLE {
            return Err(format!(
                "tier pair tier_{} (higher) vs tier_{} (lower): {permille} permille of {TIER_DUELS} duels, below the {WIN_BAR_PERMILLE} bar",
                higher.tier, lower.tier
            ));
        }
        Ok(permille)
    }

    #[test]
    fn tier_bar_posts_600_plus_per_pair() {
        // GA13: the shipped tier table's production reader is the tournament
        // ladder (`run_difficulty` installs SHIPPED_TIERS[index]). Each
        // adjacent pair posts the higher rung clearing the bar over
        // TIER_DUELS seeded duels, and the pair replays exactly.
        for low in 0..SHIPPED_TIERS.len() - 1 {
            let (lower, higher) = (SHIPPED_TIERS[low], SHIPPED_TIERS[low + 1]);
            let first = tier_pair_verdict(higher, lower).expect("higher rung must clear the bar");
            let again = tier_pair_verdict(higher, lower).expect("pair replays");
            assert_eq!(
                first, again,
                "pair tier_{}/tier_{} must replay",
                higher.tier, lower.tier
            );
            println!(
                "tier pair tier_{} vs tier_{}: {first} permille of {TIER_DUELS} duels",
                higher.tier, lower.tier
            );
        }
        // Failure case: an inverted install puts tier_3 on the lowest rung,
        // so the nominally-higher rung holds tier_1 and posts below the bar;
        // the refusal names the pair.
        let inverted = tier_pair_verdict(SHIPPED_TIERS[0], SHIPPED_TIERS[2])
            .expect_err("inverted install must post below the bar");
        assert!(
            inverted.contains("tier_1") && inverted.contains("tier_3"),
            "refusal names the pair: {inverted}"
        );
        println!("inverted install refused: {inverted}");
    }
}
