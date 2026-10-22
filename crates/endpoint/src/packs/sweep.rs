//! `play.sweep`: seeded bots play N headless bouts and answer one envelope.
//! `tune.sweep`: one knob, a range, fixed seeds; the balance curve comes back
//! as rows and as CSV bytes.
//!
//! Consumer: a playtest agent that wants one set of numbers for "does a batch
//! of bouts crash, drift or freeze" (`play.sweep`) and one set for "which
//! value of this knob keeps the win rate in the band" (`tune.sweep`).
//! Data owner for the simulation is `engine-arena` ([`GymLive`], whose brains
//! decide every slot) for `play.sweep`, and `engine-playtest` (the balance
//! tournament and its difficulty curve) for `tune.sweep`; this file owns the
//! verb DTOs, the sweep loops and the evidence file. Units: bouts, ticks,
//! fighter-ticks, permille win rates. Every bout runs twice from its own seed:
//! a bout the arena refuses or that stops early is a crash, a bout whose two
//! runs disagree on any tick digest is a hash divergence, and a stuck tick is
//! one live fighter moving slower than [`STUCK_SPEED_MPS`]. The knob sweep
//! replays one tournament per knob value over a fixed seed list, so the same
//! knob and seeds give the same curve twice. No clock is read. Evidence: the
//! `play.sweep` envelope is also written to `target/reports/sweep-<seed>.json`;
//! the knob sweep answers with its rows and its CSV bytes. DTO docs off.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use engine_arena::fork::STUCK_SPEED_MPS;
use engine_arena::gym::GymLive;
use engine_arena::{ArenaConfig, BoutTape, MAX_PARTICIPANTS};
use engine_core::splitmix64;
use engine_playtest::tournament::{
    render_sweep_csv, standard_difficulties, sweep_points, SweepPoint, TournamentConfig,
    MAX_MATCH_TICKS, STANDARD_TICKS, SWEEP_HUMAN, SWEEP_KNOB, SWEEP_MAX_VALUES, SWEEP_PERFECT,
    SWEEP_VALUES, WIN_MAX_PERMILLE, WIN_MIN_PERMILLE,
};
use engine_playtest::winrates::HumanError;
use rmcp::schemars::JsonSchema;
// `self` for schemars derive with `serde(with)`.
use rmcp::serde::{self, Deserialize, Serialize};

use crate::mcp::{McpServer, McpToolError};
use crate::packs::physics_settings::Knob as SettingKnob;
use crate::packs::{bad_args, PackDef, PackError, VerbSpec};
use crate::play::play_session::dec_u64;
use crate::play::{build_tape, PlayInputDto};
use crate::tool_capabilities::CapabilityCard;

/// Verb: seeded bots play N bouts, one envelope back.
pub const TOOL_PLAY_SWEEP: &str = "play.sweep";
/// Verb: sweep one knob over a range with fixed seeds, the curve comes back.
pub const TOOL_TUNE_SWEEP: &str = "tune.sweep";
/// Most bouts one sweep plays.
pub const MAX_SWEEP_BOUTS: u32 = 20;
/// Bouts when the call names none.
pub const DEFAULT_SWEEP_BOUTS: u32 = 5;
/// Shortest bout in ticks.
pub const MIN_SWEEP_TICKS: u32 = 60;
/// Longest bout in ticks (the `play.run` bound).
pub const MAX_SWEEP_TICKS: u32 = crate::play::MAX_PLAY_TICKS;
/// Bout length when the call names none.
pub const DEFAULT_SWEEP_TICKS: u32 = 600;
/// Sweep seed when the call names none.
pub const DEFAULT_SWEEP_SEED: u64 = 2040;
/// Arena half-extent of every swept bout, meters.
const SWEEP_HALF_EXTENT_M: f32 = 20.0;
/// Fixed seeds one knob sweep replays per difficulty when the call names none.
pub const DEFAULT_TUNE_SEEDS: u32 = 5;
/// Most fixed seeds one knob sweep may replay per difficulty.
pub const MAX_TUNE_SEEDS: u32 = 20;
/// First seed of a knob sweep when the call names none.
pub const DEFAULT_TUNE_BASE_SEED: u64 = 2040;
/// Fight length of a knob sweep in ticks when the call names none.
pub const DEFAULT_TUNE_TICKS: u32 = STANDARD_TICKS;
/// Ticks of the spotlight probe one knob sweep runs per value.
const TUNE_SPOTLIGHT_TICKS: u32 = 240;

/// Register `play.sweep` and `tune.sweep` on the play pack under construction.
pub fn add_verb(pack: &mut PackDef) -> Result<(), PackError> {
    pack.add(VerbSpec::of::<PlaySweepTool>(
        TOOL_PLAY_SWEEP,
        super::PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<TuneSweepTool>(
        TOOL_TUNE_SWEEP,
        super::PACK,
        CapabilityCard::Observe,
    )?);
    Ok(())
}

/// Request: sweep seed, bout count, bout length and headcount.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlaySweepArgs {
    #[serde(default, with = "dec_u64::opt")]
    #[schemars(with = "Option<String>")]
    pub seed: Option<u64>,
    #[serde(default)]
    pub bouts: Option<u32>,
    #[serde(default)]
    pub ticks: Option<u32>,
    #[serde(default, alias = "participants")]
    pub fighters: Option<u8>,
}

/// The envelope: counts over all bouts, plus where the evidence file went.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PlaySweepOutput {
    #[serde(with = "dec_u64")]
    #[schemars(with = "String")]
    pub seed: u64,
    pub bouts: u32,
    pub ticks: u32,
    pub crashes: u32,
    pub hash_divergences: u32,
    pub stuck_ticks: u64,
    pub evidence: String,
}

/// A validated sweep.
#[derive(Debug, Clone, Copy)]
struct Spec {
    seed: u64,
    bouts: u32,
    ticks: u32,
    fighters: u8,
}

fn spec_of(args: &PlaySweepArgs) -> Result<Spec, McpToolError> {
    let bouts = args.bouts.unwrap_or(DEFAULT_SWEEP_BOUTS);
    if !(1..=MAX_SWEEP_BOUTS).contains(&bouts) {
        return Err(bad_args(format!(
            "bouts {bouts} is outside 1..={MAX_SWEEP_BOUTS}; re-run with a bout count in range"
        )));
    }
    let ticks = args.ticks.unwrap_or(DEFAULT_SWEEP_TICKS);
    if !(MIN_SWEEP_TICKS..=MAX_SWEEP_TICKS).contains(&ticks) {
        return Err(bad_args(format!(
            "ticks {ticks} is outside {MIN_SWEEP_TICKS}..={MAX_SWEEP_TICKS}; re-run with a bout length in range"
        )));
    }
    let fighters = args.fighters.unwrap_or(2);
    if !(2..=MAX_PARTICIPANTS).contains(&fighters) {
        return Err(bad_args(format!(
            "fighters {fighters} is outside 2..={MAX_PARTICIPANTS}; re-run with a headcount in range"
        )));
    }
    Ok(Spec {
        seed: args.seed.unwrap_or(DEFAULT_SWEEP_SEED),
        bouts,
        ticks,
        fighters,
    })
}

/// What one bout run saw: the digest after every tick and the stuck count.
struct Run {
    digests: Vec<u64>,
    stuck: u64,
}

/// Play one bout headless, the bots deciding every slot, plus the `extra`
/// inputs of tick 0. `None` when the arena refuses the config or stops early.
fn play_bout(config: &ArenaConfig, extra: &BoutTape) -> Option<Run> {
    let mut env = GymLive::new(config)?;
    let still_sq = STUCK_SPEED_MPS * STUCK_SPEED_MPS;
    let mut run = Run {
        digests: Vec::with_capacity(usize::try_from(config.ticks).unwrap_or(0)),
        stuck: 0,
    };
    for tick in 0..config.ticks {
        let pinned = if tick == 0 { extra.0.as_slice() } else { &[] };
        env.step(pinned)?;
        let snapshot = env.snapshot();
        run.digests.push(snapshot.digest);
        for fighter in &snapshot.fighters {
            let [vx, _, vz] = fighter.velocity;
            if fighter.hp > 0 && vx * vx + vz * vz < still_sq {
                run.stuck += 1;
            }
        }
    }
    Some(run)
}

/// A bout run behind a panic fence: a panic is a crash, never an abort.
fn fenced(config: &ArenaConfig, extra: &BoutTape) -> Option<Run> {
    catch_unwind(AssertUnwindSafe(|| play_bout(config, extra))).unwrap_or(None)
}

/// Play the sweep. `tamper` names a bout whose replay gets one extra input at
/// tick 0, the way a nondeterministic step would drift: the seam the
/// divergence test uses; the verb passes `None`.
fn sweep(spec: &Spec, tamper: Option<u32>) -> PlaySweepOutput {
    let (mut crashes, mut hash_divergences, mut stuck_ticks) = (0u32, 0u32, 0u64);
    for bout in 0..spec.bouts {
        let config = ArenaConfig {
            seed: splitmix64(spec.seed.wrapping_add(u64::from(bout))),
            ticks: spec.ticks,
            fighters: spec.fighters,
            half_extent_m: SWEEP_HALF_EXTENT_M,
            table_version: 1,
        };
        let drift = if tamper == Some(bout) {
            let shove = PlayInputDto {
                tick: 0,
                fighter: 0,
                mx: -100,
                mz: 0,
                facing: 0,
                buttons: 0x01,
            };
            build_tape(&[shove], config.ticks, config.fighters).unwrap_or_default()
        } else {
            BoutTape::default()
        };
        match (
            fenced(&config, &BoutTape::default()),
            fenced(&config, &drift),
        ) {
            (Some(first), Some(replay)) => {
                hash_divergences += u32::from(first.digests != replay.digests);
                stuck_ticks += first.stuck;
            }
            _ => crashes += 1,
        }
    }
    PlaySweepOutput {
        seed: spec.seed,
        bouts: spec.bouts,
        ticks: spec.ticks,
        crashes,
        hash_divergences,
        stuck_ticks,
        evidence: String::new(),
    }
}

/// Write the envelope under `root/target/reports` and return its relative path.
fn write_evidence(root: &Path, out: &PlaySweepOutput) -> Result<String, McpToolError> {
    let rel = format!("target/reports/sweep-{}.json", out.seed);
    let dir = root.join("target/reports");
    let failed = |why: std::io::Error| {
        McpToolError::new(
            "evidence_unwritten",
            format!("could not write {rel} ({why}); re-run after freeing target/reports"),
        )
    };
    std::fs::create_dir_all(&dir).map_err(failed)?;
    let body = rmcp::serde_json::to_string_pretty(out).unwrap_or_default();
    std::fs::write(root.join(&rel), body).map_err(failed)?;
    Ok(rel)
}

/// Run one request: validate, sweep, write the evidence under `root`.
fn run_sweep(args: &PlaySweepArgs, root: &Path) -> Result<PlaySweepOutput, McpToolError> {
    let mut out = sweep(&spec_of(args)?, None);
    out.evidence = write_evidence(root, &out)?;
    Ok(out)
}

/// `play.sweep` tool: seeded bots play N bouts, one envelope back.
pub struct PlaySweepTool;

impl rmcp::handler::server::router::tool::ToolBase for PlaySweepTool {
    type Parameter = PlaySweepArgs;
    type Output = PlaySweepOutput;
    type Error = McpToolError;

    fn name() -> std::borrow::Cow<'static, str> {
        TOOL_PLAY_SWEEP.into()
    }

    fn description() -> Option<std::borrow::Cow<'static, str>> {
        Some(
            "Seeded bots play N headless bouts twice each: returns bouts, crashes, \
             hash_divergences and stuck_ticks, and writes target/reports/sweep-<seed>.json. \
             Read-only on the scene."
                .into(),
        )
    }

    fn annotations() -> Option<rmcp::model::ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl rmcp::handler::server::router::tool::SyncTool<McpServer> for PlaySweepTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_PLAY_SWEEP, |_state| run_sweep(&param, Path::new(".")))
    }
}

// ---------------------------------------------------------------------------
// tune.sweep: one knob, a range, fixed seeds, the curve back.
// ---------------------------------------------------------------------------

/// The knob one sweep varies: the fight length (the balance knob the
/// tournament reads) or one door setting from the `settings.get` table. The
/// settings knobs are the door's knob vocabulary, so the winning value of a
/// sweep lands with `settings.set` and comes back with `settings.undo`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TuneKnob {
    /// Fight length in fixed ticks (`TournamentConfig::match_ticks`).
    MatchTicks,
    /// A door setting (the `settings.get` knob table).
    Setting(SettingKnob),
}

impl TuneKnob {
    /// Every knob the sweep takes: the balance knob first, then the
    /// `settings.get` table in its own order.
    fn all() -> Vec<TuneKnob> {
        let mut knobs = vec![TuneKnob::MatchTicks];
        knobs.extend(SettingKnob::ALL.into_iter().map(TuneKnob::Setting));
        knobs
    }

    /// The knob named `word` (trimmed, any letter case).
    fn from_name(word: &str) -> Option<Self> {
        let trimmed = word.trim();
        if trimmed.eq_ignore_ascii_case(SWEEP_KNOB) {
            return Some(Self::MatchTicks);
        }
        SettingKnob::from_name(trimmed).map(Self::Setting)
    }

    /// The knob's word, as the CSV and the refusal name it.
    fn name(self) -> &'static str {
        match self {
            Self::MatchTicks => SWEEP_KNOB,
            Self::Setting(knob) => knob.name(),
        }
    }

    /// The knob's unit, as the `settings.get` table prints it.
    fn unit(self) -> &'static str {
        match self {
            Self::MatchTicks => "ticks",
            Self::Setting(knob) => knob.unit(),
        }
    }

    /// The smallest legal value.
    fn min(self) -> u32 {
        match self {
            Self::MatchTicks => 1,
            Self::Setting(knob) => u32::try_from(knob.min()).unwrap_or(0),
        }
    }

    /// The largest legal value.
    fn max(self) -> u32 {
        match self {
            Self::MatchTicks => MAX_MATCH_TICKS,
            Self::Setting(knob) => u32::try_from(knob.max()).unwrap_or(u32::MAX),
        }
    }

    /// The values a sweep runs when the caller names no range: the standard
    /// curve for the fight length, the range's three points for a setting.
    fn default_values(self) -> Vec<u32> {
        match self {
            Self::MatchTicks => SWEEP_VALUES.to_vec(),
            Self::Setting(_) => {
                let min = self.min();
                let max = self.max();
                vec![min, min.saturating_add(max.saturating_sub(min) / 2), max]
            }
        }
    }

    /// The setting behind this knob, when it is one.
    fn setting(self) -> Option<SettingKnob> {
        match self {
            Self::MatchTicks => None,
            Self::Setting(knob) => Some(knob),
        }
    }
}

/// Request: the knob, the range of values, the fixed seeds and the fight
/// length the sweep runs.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TuneSweepArgs {
    /// The knob to sweep: `match_ticks` or any name in the `settings.get` table.
    #[serde(default)]
    pub knob: Option<String>,
    /// The values to sweep, as a list.
    #[serde(default)]
    pub values: Option<Vec<u32>>,
    /// Smallest value of the range (with `max` and `step`).
    #[serde(default)]
    pub min: Option<u32>,
    /// Largest value of the range (with `min` and `step`).
    #[serde(default)]
    pub max: Option<u32>,
    /// Step of the range (default 1).
    #[serde(default)]
    pub step: Option<u32>,
    /// Fixed seeds replayed per difficulty (default 5).
    #[serde(default)]
    pub seeds: Option<u32>,
    /// First seed of the sweep (default 2040).
    #[serde(default, with = "dec_u64::opt")]
    #[schemars(with = "Option<String>")]
    pub base_seed: Option<u64>,
    /// Fight length in ticks for a setting knob (default 600).
    #[serde(default)]
    pub ticks: Option<u32>,
}

/// One kill share of a curve row: the observed killing byte and its share.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TuneSweepShare {
    pub move_byte: u8,
    pub kills: u32,
    pub share_permille: u16,
}

/// One curve row: a difficulty's outcome at one knob value.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TuneSweepRow {
    pub value: u32,
    pub difficulty: String,
    pub player: String,
    pub wins: u32,
    pub draws: u32,
    pub losses: u32,
    pub win_rate_permille: u16,
    pub kill_shares: Vec<TuneSweepShare>,
    pub stuck_count: u32,
    pub trials: u32,
}

/// One skipped variant: nobody finished it, so it carries no win rate.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TuneSweepSkip {
    pub value: u32,
    pub difficulty: String,
    pub reason: String,
}

/// The answer: the curve rows, the curve as CSV bytes and the winning value.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct TuneSweepOutput {
    /// The knob the sweep varied.
    pub knob: String,
    /// The knob's unit.
    pub unit: String,
    /// The swept values, in order.
    pub values: Vec<u32>,
    /// The fixed seeds replayed per difficulty (match `i` uses
    /// `base_seed + i`; difficulty `d` replays `base_seed + d * gap + i`).
    pub seeds: u32,
    /// The first seed of the sweep.
    #[serde(with = "dec_u64")]
    #[schemars(with = "String")]
    pub base_seed: u64,
    /// The curve rows, in sweep order.
    pub rows: Vec<TuneSweepRow>,
    /// The variants the curve skipped, with the reason.
    pub skipped: Vec<TuneSweepSkip>,
    /// The curve as CSV bytes: `knob,value,difficulty,player,win_rate_permille,kill_shares,stuck_count,trials`.
    pub csv: String,
    /// The winning value: the one whose win rate sits closest to the middle
    /// of the legal band. `None` when the curve carries no row at all.
    pub best: Option<u32>,
    /// True when the curve moved with the knob. A knob the balance harness
    /// cannot see (a door setting) gives a flat curve, and the sweep says so
    /// instead of pretending the knob changed the fight.
    pub curve_moves: bool,
    /// How the winning value lands, undoably.
    pub landing: String,
}

/// A validated sweep request.
#[derive(Clone, PartialEq, Eq, Debug)]
struct TuneSpec {
    knob: TuneKnob,
    values: Vec<u32>,
    seeds: u32,
    base_seed: u64,
    ticks: u32,
}

/// The values the call asks for: the explicit list, the numeric range, or the
/// knob's own default values. Every value is range-checked against the knob.
fn tune_values(args: &TuneSweepArgs, knob: TuneKnob) -> Result<Vec<u32>, McpToolError> {
    let values = match &args.values {
        Some(list) => list.clone(),
        None if args.min.is_none() && args.max.is_none() && args.step.is_none() => {
            knob.default_values()
        }
        None => {
            let min = args.min.unwrap_or_else(|| knob.min());
            let max = args.max.unwrap_or_else(|| knob.max());
            let step = args.step.unwrap_or(1).max(1);
            if min > max {
                return Err(bad_args(format!(
                    "range {min}..={max} is empty; re-run with min <= max"
                )));
            }
            let span = u64::from(max) - u64::from(min);
            let count = span / u64::from(step) + 1;
            let cap = u64::try_from(SWEEP_MAX_VALUES).unwrap_or(u64::MAX);
            if count > cap {
                return Err(bad_args(format!(
                    "range {min}..={max} step {step} gives {count} values; the sweep takes at most {SWEEP_MAX_VALUES}; re-run with a bigger step"
                )));
            }
            let mut walked = Vec::new();
            let mut value = min;
            for _ in 0..count {
                walked.push(value);
                value = value.saturating_add(step);
            }
            walked
        }
    };
    if values.is_empty() || values.len() > SWEEP_MAX_VALUES {
        return Err(bad_args(format!(
            "values {} is outside 1..={SWEEP_MAX_VALUES}; re-run with a range in that size",
            values.len()
        )));
    }
    for value in &values {
        if *value < knob.min() || *value > knob.max() {
            return Err(bad_args(format!(
                "value {value} is outside {}..={} {}; re-run with a value in range",
                knob.min(),
                knob.max(),
                knob.unit()
            )));
        }
    }
    Ok(values)
}

/// Validate one request: the knob, the range, the seed count and the fight
/// length. Every refusal names the legal values.
fn tune_spec(args: &TuneSweepArgs) -> Result<TuneSpec, McpToolError> {
    let named = args.knob.as_deref().unwrap_or(SWEEP_KNOB);
    let knob = TuneKnob::from_name(named).ok_or_else(|| {
        let legal: Vec<&str> = TuneKnob::all().iter().map(|knob| knob.name()).collect();
        bad_args(format!(
            "knob `{named}` is unknown; legal knobs: [{}]; re-run with one of those names",
            legal.join(", ")
        ))
    })?;
    let values = tune_values(args, knob)?;
    let seeds = args.seeds.unwrap_or(DEFAULT_TUNE_SEEDS);
    if seeds == 0 || seeds > MAX_TUNE_SEEDS {
        return Err(bad_args(format!(
            "seeds {seeds} is outside 1..={MAX_TUNE_SEEDS}; re-run with a seed count in range"
        )));
    }
    let ticks = args.ticks.unwrap_or(DEFAULT_TUNE_TICKS);
    if ticks == 0 || ticks > MAX_MATCH_TICKS {
        return Err(bad_args(format!(
            "ticks {ticks} is outside 1..={MAX_MATCH_TICKS}; re-run with a fight length in range"
        )));
    }
    Ok(TuneSpec {
        knob,
        values,
        seeds,
        base_seed: args.base_seed.unwrap_or(DEFAULT_TUNE_BASE_SEED),
        ticks,
    })
}

/// The tournament every value runs: the shipped difficulty ladder, the
/// caller's seeds, a short spotlight probe. Small on purpose: a sweep runs
/// one tournament per value.
fn tune_base(spec: &TuneSpec) -> TournamentConfig {
    TournamentConfig {
        base_seed: spec.base_seed,
        matches: spec.seeds,
        match_ticks: spec.ticks,
        half_extent_m: 20.0,
        difficulties: standard_difficulties(),
        spotlight_matches: 1,
        spotlight_ticks: TUNE_SPOTLIGHT_TICKS,
    }
}

/// The mean win rate of one value over its rows, in permille.
fn tune_mean_rate(rows: &[TuneSweepRow], value: u32) -> Option<u32> {
    let rates: Vec<u32> = rows
        .iter()
        .filter(|row| row.value == value)
        .map(|row| u32::from(row.win_rate_permille))
        .collect();
    if rates.is_empty() {
        return None;
    }
    let count = u32::try_from(rates.len()).unwrap_or(u32::MAX);
    Some(rates.iter().copied().sum::<u32>() / count.max(1))
}

/// The winning value: the one whose win rate sits closest to the middle of
/// the legal band. Ties keep the lowest value, so the answer never depends
/// on the order the rows arrived in.
fn tune_best(rows: &[TuneSweepRow], values: &[u32]) -> Option<u32> {
    let middle = u32::from(WIN_MIN_PERMILLE + WIN_MAX_PERMILLE) / 2;
    values
        .iter()
        .filter_map(|value| {
            tune_mean_rate(rows, *value).map(|rate| (rate.abs_diff(middle), *value))
        })
        .min()
        .map(|(_, value)| value)
}

/// True when the values sit on different win rates: the knob moved the curve.
fn tune_curve_moves(rows: &[TuneSweepRow], values: &[u32]) -> bool {
    let rates: Vec<u32> = values
        .iter()
        .filter_map(|value| tune_mean_rate(rows, *value))
        .collect();
    rates.windows(2).any(|pair| pair[0] != pair[1])
}

/// How the winning value lands, undoably: one `settings.set` writes it and
/// one `settings.undo` takes it back, so the tape stays replayable.
fn tune_landing(knob: TuneKnob, best: Option<u32>) -> String {
    match (knob.setting(), best) {
        (Some(setting), Some(value)) => format!(
            "settings.set knob={} value={value} at_tick=0, then settings.undo at_tick=1 takes it back",
            setting.name()
        ),
        (None, Some(value)) => format!(
            "the fight length lands in the tournament config (match_ticks={value}); \
             no door verb writes it"
        ),
        (_, None) => "the curve carries no row: nothing to land".to_string(),
    }
}

/// Run one knob sweep: one tournament per value, the curve as rows and CSV.
fn run_tune_sweep(args: &TuneSweepArgs) -> Result<TuneSweepOutput, McpToolError> {
    let spec = tune_spec(args)?;
    let base = tune_base(&spec);
    let points: Vec<SweepPoint> = spec
        .values
        .iter()
        .map(|value| SweepPoint {
            value: *value,
            config: match spec.knob {
                TuneKnob::MatchTicks => TournamentConfig {
                    match_ticks: *value,
                    ..base.clone()
                },
                TuneKnob::Setting(_) => base.clone(),
            },
        })
        .collect();
    let report = sweep_points(spec.knob.name(), &points, HumanError::off())
        .map_err(|error| bad_args(format!("sweep refused: {}", error.reason)))?;
    let rows: Vec<TuneSweepRow> = report
        .rows
        .iter()
        .map(|row| TuneSweepRow {
            value: row.value,
            difficulty: row.difficulty.clone(),
            player: if row.human {
                SWEEP_HUMAN.to_owned()
            } else {
                SWEEP_PERFECT.to_owned()
            },
            wins: row.wins,
            draws: row.draws,
            losses: row.losses,
            win_rate_permille: row.rate_permille,
            kill_shares: row
                .kill_shares
                .iter()
                .map(|share| TuneSweepShare {
                    move_byte: share.move_byte,
                    kills: share.kills,
                    share_permille: share.share_permille,
                })
                .collect(),
            stuck_count: row.stuck_count,
            trials: row.trials,
        })
        .collect();
    let skipped: Vec<TuneSweepSkip> = report
        .skipped
        .iter()
        .map(|skip| TuneSweepSkip {
            value: skip.value,
            difficulty: skip.difficulty.clone(),
            reason: skip.reason.clone(),
        })
        .collect();
    let best = tune_best(&rows, &spec.values);
    Ok(TuneSweepOutput {
        knob: spec.knob.name().to_owned(),
        unit: spec.knob.unit().to_owned(),
        values: spec.values.clone(),
        seeds: spec.seeds,
        base_seed: spec.base_seed,
        curve_moves: tune_curve_moves(&rows, &spec.values),
        best,
        landing: tune_landing(spec.knob, best),
        csv: render_sweep_csv(&report),
        rows,
        skipped,
    })
}

/// `tune.sweep` handler: the knob sweep answers with its rows and its CSV.
fn tune_sweep(_service: &McpServer, args: &TuneSweepArgs) -> Result<TuneSweepOutput, McpToolError> {
    run_tune_sweep(args)
}

crate::e2_tool!(
    /// `tune.sweep`: one knob, a range, fixed seeds; the curve comes back.
    TuneSweepTool,
    name = TOOL_TUNE_SWEEP,
    description = "Sweep one knob over a range with fixed seeds and read the balance curve: knob is match_ticks or any settings.get knob, values is the range, seeds is the fixed seed count per difficulty. Answers the curve rows, the CSV bytes, the winning value and whether the knob moved the curve. Read-only.",
    readonly = true,
    args = TuneSweepArgs,
    output = TuneSweepOutput,
    invoke = tune_sweep
);

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    /// A knob sweep over the fight length: the curve comes back as rows and as
    /// CSV bytes, and the same knob and seeds give the same CSV twice.
    #[test]
    fn tune_sweep_answers_with_the_curve_and_replays_it_exactly() {
        let args = TuneSweepArgs {
            knob: Some(SWEEP_KNOB.to_owned()),
            values: Some(vec![300, 600]),
            seeds: Some(DEFAULT_TUNE_SEEDS),
            ticks: None,
            ..Default::default()
        };
        let first = run_tune_sweep(&args).expect("the sweep runs");
        let second = run_tune_sweep(&args).expect("the sweep replays");
        assert_eq!(
            first.csv, second.csv,
            "the same knob and seeds give one CSV"
        );
        assert_eq!(first.rows, second.rows);
        assert_eq!(first.skipped, second.skipped);
        assert_eq!(first.knob, SWEEP_KNOB);
        assert_eq!(first.unit, "ticks");
        assert_eq!(first.values, vec![300, 600]);
        assert_eq!(first.seeds, DEFAULT_TUNE_SEEDS);
        assert!(first.csv.starts_with(
            "knob,value,difficulty,player,win_rate_permille,kill_shares,stuck_count,trials\n"
        ));
        // Every value and difficulty lands in exactly one row or skip.
        assert_eq!(first.rows.len() + first.skipped.len(), 2 * 3);
        for row in &first.rows {
            assert_eq!(row.trials, DEFAULT_TUNE_SEEDS);
            assert_eq!(row.player, SWEEP_PERFECT);
            assert_eq!(row.wins + row.draws + row.losses, row.trials);
        }
    }

    /// Every knob in the `settings.get` table is a legal knob for the sweep.
    #[test]
    fn tune_sweep_takes_any_settings_knob_name() {
        for knob in SettingKnob::ALL {
            let code = u32::try_from(knob.default_code()).unwrap_or(0);
            let out = run_tune_sweep(&TuneSweepArgs {
                knob: Some(knob.name().to_owned()),
                values: Some(vec![code]),
                seeds: Some(1),
                ticks: Some(60),
                ..Default::default()
            })
            .expect("the knob table answers");
            assert_eq!(out.knob, knob.name());
            assert_eq!(out.unit, knob.unit());
            assert_eq!(out.values, vec![code]);
        }
    }

    /// A knob name outside the table refuses with the legal names.
    #[test]
    fn a_bad_knob_name_is_a_typed_refusal() {
        let refused = run_tune_sweep(&TuneSweepArgs {
            knob: Some("gore_level".to_owned()),
            ..Default::default()
        })
        .expect_err("an unknown knob refuses");
        assert_eq!(refused.code, "bad_args");
        assert!(refused.message.contains("gore_level"));
        for legal in [SWEEP_KNOB, "gravity", "render_scale"] {
            assert!(
                refused.message.contains(legal),
                "the refusal names {legal}: {}",
                refused.message
            );
        }
    }

    /// A value outside the knob's range refuses with the range in the reason.
    #[test]
    fn a_value_outside_the_knob_range_is_refused() {
        let refused = run_tune_sweep(&TuneSweepArgs {
            knob: Some("gravity".to_owned()),
            values: Some(vec![40_000]),
            seeds: Some(1),
            ticks: Some(60),
            ..Default::default()
        })
        .expect_err("an out of range value refuses");
        assert_eq!(refused.code, "bad_args");
        assert!(refused.message.contains("0..=32000"), "{}", refused.message);
    }

    /// The winning value lands undoably: one `settings.set` writes it and one
    /// `settings.undo` takes it back, so the tape stays replayable.
    #[test]
    fn the_winning_value_lands_with_one_settings_set_and_one_settings_undo() {
        use crate::packs::physics_settings::{
            SettingsGetArgs, SettingsGetTool, SettingsSetArgs, SettingsSetTool, SettingsUndoArgs,
            SettingsUndoTool,
        };
        let door = McpServer::new(7, Capability::unrestricted(7));
        // The largest seed count: the ladder needs a finished fight before the
        // curve carries a row, and the winning value comes from that row.
        let out = run_tune_sweep(&TuneSweepArgs {
            knob: Some("gravity".to_owned()),
            values: Some(vec![0, 9_810]),
            seeds: Some(MAX_TUNE_SEEDS),
            ticks: None,
            ..Default::default()
        })
        .expect("the sweep runs");
        let best = out.best.expect("the curve names a value");
        // The arena duel reads no door setting, so both values sit on one
        // rate and the tie keeps the lowest value.
        assert_eq!(best, 0, "the flat curve keeps the lowest value");
        assert!(!out.curve_moves, "a door setting does not move the curve");
        assert!(out.landing.contains("settings.set"), "{}", out.landing);
        // The sweep itself changed nothing in the session.
        let before = SettingsGetTool::invoke(&door, SettingsGetArgs { at_tick: None })
            .expect("the knob table reads");
        assert_eq!(before.tape_records, 0);
        // The winning value lands as one undoable write.
        SettingsSetTool::invoke(
            &door,
            SettingsSetArgs {
                at_tick: 0,
                knob: "gravity".to_owned(),
                value: i64::from(best),
            },
        )
        .expect("the value lands");
        let landed = SettingsGetTool::invoke(&door, SettingsGetArgs { at_tick: None })
            .expect("the knob table reads");
        assert_eq!(
            landed
                .knobs
                .iter()
                .find(|row| row.name == "gravity")
                .expect("gravity is listed")
                .value,
            i64::from(best)
        );
        // One undo takes it back and the tape stays replayable.
        SettingsUndoTool::invoke(&door, SettingsUndoArgs { at_tick: 1 }).expect("the undo lands");
        let after = SettingsGetTool::invoke(&door, SettingsGetArgs { at_tick: None })
            .expect("the knob table reads");
        assert_eq!(
            after
                .knobs
                .iter()
                .find(|row| row.name == "gravity")
                .expect("gravity is listed")
                .value,
            9_810
        );
        assert_eq!(after.tape_records, 2);
    }

    /// The verb is listed by `api.describe` and answers by exact name.
    #[test]
    fn the_tune_verb_is_listed_and_answers() {
        use crate::insight::mcp_tools::{ApiDescribeArgs, ApiDescribeTool};
        let server = McpServer::new(7, Capability::unrestricted(7));
        let entry = ApiDescribeTool::invoke(
            &server,
            ApiDescribeArgs {
                path: TOOL_TUNE_SWEEP.to_owned(),
            },
        )
        .expect("tune.sweep describes");
        assert!(entry.docs.contains("knob:"), "{}", entry.docs);
        let out = TuneSweepTool::invoke(
            &server,
            TuneSweepArgs {
                knob: Some(SWEEP_KNOB.to_owned()),
                values: Some(vec![120]),
                seeds: Some(1),
                ticks: Some(60),
                ..Default::default()
            },
        )
        .expect("tune.sweep answers");
        assert_eq!(out.knob, SWEEP_KNOB);
        assert!(!out.csv.is_empty());
    }

    fn spec(bouts: u32) -> Spec {
        Spec {
            seed: 7,
            bouts,
            ticks: 120,
            fighters: 2,
        }
    }

    /// A 5-bout sweep on seed 7 returns the same envelope twice and counts
    /// nothing wrong in a healthy arena.
    #[test]
    fn sweep_five_bouts_seed_7_repeats_its_envelope() {
        let first = sweep(&spec(5), None);
        let second = sweep(&spec(5), None);
        assert_eq!(first, second);
        assert_eq!((first.bouts, first.ticks), (5, 120));
        assert_eq!((first.crashes, first.hash_divergences), (0, 0));
        eprintln!("sweep seed 7: {first:?}");
    }

    /// One tampered replay is exactly one hash divergence.
    #[test]
    fn an_injected_divergence_reports_one_hash_divergence() {
        let out = sweep(&spec(5), Some(2));
        assert_eq!(out.hash_divergences, 1);
        assert_eq!(out.crashes, 0);
        assert_eq!(out.bouts, 5);
    }

    /// An arena the config check refuses is a crash, counted, not a panic.
    #[test]
    fn a_refused_bout_is_a_counted_crash() {
        let out = sweep(
            &Spec {
                ticks: 0,
                ..spec(3)
            },
            None,
        );
        assert_eq!((out.crashes, out.hash_divergences), (3, 0));
    }

    #[test]
    fn bad_numbers_refuse_with_the_legal_range() {
        let refuse = |args: PlaySweepArgs| spec_of(&args).expect_err("refused");
        let bouts = refuse(PlaySweepArgs {
            bouts: Some(21),
            ..Default::default()
        });
        assert!(bouts.message.contains("1..=20"), "{}", bouts.message);
        let ticks = refuse(PlaySweepArgs {
            ticks: Some(10),
            ..Default::default()
        });
        assert!(ticks.message.contains("60..=3600"), "{}", ticks.message);
        let fighters = refuse(PlaySweepArgs {
            fighters: Some(1),
            ..Default::default()
        });
        assert!(fighters.message.contains("2..="), "{}", fighters.message);
    }

    /// The verb is listed by `api.describe`, runs through the door and writes
    /// the evidence file next to the working directory it was given.
    #[test]
    fn the_verb_is_listed_and_writes_its_evidence() {
        use crate::insight::mcp_tools::{ApiDescribeArgs, ApiDescribeTool};
        let server = McpServer::new(7, Capability::unrestricted(7));
        let entry = ApiDescribeTool::invoke(
            &server,
            ApiDescribeArgs {
                path: TOOL_PLAY_SWEEP.to_owned(),
            },
        )
        .expect("play.sweep describes");
        assert!(entry.docs.contains("bouts:"), "{}", entry.docs);

        let root = std::env::temp_dir().join(format!("e2040-sweep-{}", std::process::id()));
        let args = PlaySweepArgs {
            seed: Some(7),
            bouts: Some(2),
            ticks: Some(60),
            fighters: None,
        };
        let out = run_sweep(&args, &root).expect("sweeps");
        assert_eq!(out.evidence, "target/reports/sweep-7.json");
        let text = std::fs::read_to_string(root.join(&out.evidence)).expect("evidence reads");
        assert!(text.contains("\"hash_divergences\": 0"), "{text}");
        assert!(text.contains("\"bouts\": 2"), "{text}");
        let again = run_sweep(&args, &root).expect("sweeps again");
        assert_eq!(out, again);
        let _ = std::fs::remove_dir_all(&root);
    }
}
