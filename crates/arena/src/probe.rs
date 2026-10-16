//! probe: exploit-sweep forward-model helpers over the public fork API.
//!
//! Consumer: the playtest exploit sweep (`engine-playtest`), which must stay
//! free of genre tokens in production code, so every helper that names the
//! bout input types lives here in the exempt arena crate. Data owner: the
//! caller owns the config; every sim builds its own [`ArenaConfig`] from it.
//! Units: fixed ticks, raw button bytes, fork damage counts. Determinism:
//! seeded and replay-exact (same config twice gives the same metrics; arm
//! ties break by lowest byte, never by thread timing or hash order). Errors:
//! [`ProbeError`]: `Refused` on a contract break, `Crashed` when a fork worker
//! panics; never a zeroed run, never a panic here. Budget: one fork sim per call.

use crate::fork::{Fork, ForkCrashed, ForkMetrics, ForkProposal};
use crate::{ArenaConfig, FighterId, FighterInput, FighterTape};

/// Why a probe run gave no metrics. A crashed fork is never read as a clean
/// run: its zeros would look like a fight nobody won.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProbeError {
    /// A contract break before any fork ran (template, tick conversion,
    /// fork refusal).
    Refused,
    /// The fork worker panicked; its metrics are not a run.
    Crashed(ForkCrashed),
}

/// True when `config` records a template input (the tick-0 brain decision
/// both seeded tapes derive from). The sweep maps `false` to its
/// no-template error before any simulation.
pub fn template_ready(config: &ArenaConfig) -> bool {
    template_input(config).is_some()
}

/// Run the scripted baseline (empty tape, both brains) once. Pair with
/// [`leader_kills`] to read the baseline verdict; a crashed baseline is
/// `Crashed`, never a zeroed verdict.
pub fn baseline_metrics(config: &ArenaConfig) -> Result<ForkMetrics, ProbeError> {
    run_tape(config, &FighterTape::default())
}

/// Run one spam-byte fork: every tick for slot 0 carries `byte` (slot 1
/// stays brain-driven, exactly like the tournament spotlight). `Refused` on
/// any contract break (bad template, bad tick conversion, fork refusal);
/// `Crashed` when the fork worker panics.
pub fn spam_metrics(config: &ArenaConfig, ticks: u32, byte: u8) -> Result<ForkMetrics, ProbeError> {
    let template = template_input(config).ok_or(ProbeError::Refused)?;
    let tape = spam_tape(&template, ticks, byte).ok_or(ProbeError::Refused)?;
    run_tape(config, &tape)
}

/// Run the prepared fork on its worker; a panic becomes `Crashed`.
fn run_tape(config: &ArenaConfig, tape: &FighterTape) -> Result<ForkMetrics, ProbeError> {
    prepare_fork(config, tape)
        .ok_or(ProbeError::Refused)?
        .try_run_n(config.ticks)
        .map_err(ProbeError::Crashed)
}

/// Slot-0 damage in finished `metrics` (0 when the ledger is empty).
pub fn damage_of(metrics: &ForkMetrics) -> u16 {
    metrics
        .damage_dealt
        .iter()
        .find(|(slot, _)| *slot == 0)
        .map(|(_, dealt)| *dealt)
        .unwrap_or(0)
}

/// True when slot 0 both ends the match and tops damage.
pub fn leader_kills(metrics: &ForkMetrics) -> bool {
    metrics.ticks_to_kill.is_some() && killer_of(metrics) == 0
}

/// Killer slot of finished `metrics`: top damage, ties to the lowest slot.
/// Shared contract with the playtest tournament helper of the same name.
pub fn killer_of(metrics: &ForkMetrics) -> u8 {
    let (killer, _) =
        metrics
            .damage_dealt
            .iter()
            .fold((0u8, 0u16), |(slot, top), (candidate, dealt)| {
                if *dealt > top {
                    (*candidate, *dealt)
                } else {
                    (slot, top)
                }
            });
    killer
}

/// One validated template input recorded on `config` with an empty tape:
/// the tick-0 brain decision both seeded tapes derive from. `None` on
/// invalid configs only (valid configs always record at least one row).
fn template_input(config: &ArenaConfig) -> Option<FighterInput> {
    crate::run_dataset(config, &FighterTape::default())?
        .rows
        .into_iter()
        .next()
        .map(|row| row.action.input)
}

/// Idle input for `slot` at `tick`: zero wish, zero facing, zeroed buttons.
/// The template carries no idle state (every bout field is overwritten),
/// so only its slot validation scope matters here.
fn idle_input(_template: &FighterInput, slot: u8, tick: u32) -> Option<FighterInput> {
    Some(FighterInput {
        tick,
        fighter: FighterId::new(slot)?,
        mx: 0,
        mz: 0,
        facing: 0,
        buttons: engine_input::Buttons::NONE,
    })
}

/// Buttoned input for `slot` at `tick`: idle wish with `byte` as the raw
/// button byte.
fn buttoned_input(template: &FighterInput, slot: u8, tick: u32, byte: u8) -> Option<FighterInput> {
    let mut input = idle_input(template, slot, tick)?;
    input.buttons.0 = byte;
    Some(input)
}

/// Spam tape: every tick for slot 0 carries `byte`.
fn spam_tape(template: &FighterInput, ticks: u32, byte: u8) -> Option<FighterTape> {
    let capacity = usize::try_from(ticks).ok()?;
    let mut inputs = Vec::with_capacity(capacity);
    for tick in 0..ticks {
        inputs.push(buttoned_input(template, 0, tick, byte)?);
    }
    Some(FighterTape(inputs))
}

/// Build one full-match fork, ready to run: prefix at tick 0, neutral morale
/// per slot, then each tape slice rides as a `SwapTape` proposal so brain
/// slots stay live exactly like the real tick. `None` on any contract break.
fn prepare_fork(config: &ArenaConfig, tape: &FighterTape) -> Option<Fork> {
    let (snapshot, _) = crate::run_prefix(config, tape, 0)?;
    let mut fork = Fork::from_snapshot(snapshot, 0)?;
    for slot in 0..config.fighters {
        fork.apply_proposal(ForkProposal::SetAggression {
            fighter: slot,
            permille: 500,
        })
        .ok()?;
    }
    // Single pass over the tape into per-slot slices: one scan, one
    // allocation per slot, never one filter plus collect per slot.
    let slots = usize::from(config.fighters);
    let per_tick = tape.0.len().max(1);
    let mut per_slot: Vec<Vec<FighterInput>> = Vec::with_capacity(slots);
    for _ in 0..slots {
        per_slot.push(Vec::with_capacity(per_tick / slots.max(1)));
    }
    for input in tape.0.iter() {
        let at = usize::from(input.fighter.0);
        if at < slots {
            per_slot[at].push(*input);
        }
    }
    for (slot, slice) in per_slot.into_iter().enumerate() {
        if slice.is_empty() {
            continue;
        }
        let fighter = u8::try_from(slot).ok()?;
        fork.apply_proposal(ForkProposal::SwapTape {
            fighter,
            inputs: slice,
        })
        .ok()?;
    }
    Some(fork)
}
