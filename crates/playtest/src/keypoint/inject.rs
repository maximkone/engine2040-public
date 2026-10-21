//! Injected verification units: one gameplay claim checked in isolation.
//!
//! A unit states the runtime state the engine must hold when the bounded
//! interaction starts (the injection), the interaction itself, and the end
//! states that must not be mistaken for success. The verifier reaches the
//! declared state through the spec's scripted prefix, proves the arrival by
//! comparing the reached state with the declared one, then judges the
//! outcome against the spec's asserts.
//!
//! Donor: arXiv 2605.07442 (GameGen-Verifier) and its two skills:
//! `generative-state-construction` (a fuzzy claim becomes a concrete
//! prestate plus the near-miss states named) and
//! `short-interaction-verification` (inject the target precondition, run a
//! short interaction, judge the outcome). We port the shape, not the text:
//! our arena reaches a state by playing to it, so the unit states the state
//! and the verifier proves the arrival instead of assuming it.
//!
//! Consumer: the door's build verbs (the endpoint lane owns those) and the
//! proof harness. Data owner: the caller owns the unit; this module owns the
//! verdict.
//!
//! Units: fixed ticks, health in permille, positions in whole millimetres.
//! Determinism scope: same-machine reproducibility — the same unit twice
//! gives the identical verdict bytes; digests fold integers only, never
//! floats.
//!
//! Errors: a unit the fork track cannot judge fails as [`UnitError`]; a
//! wrong state is a failing verdict with a reason, never a panic.

use std::collections::BTreeMap;

use engine_arena::fork::{Fork, ForkProposal};
use engine_arena::{run_prefix, ArenaConfig, FighterInput};

use super::spec_parse::materialize;
use super::{
    check_asserts, Digester, KeypointAsserts, KeypointSpec, ScriptInput, MAX_PREFIX_TICKS,
    MAX_WINDOW_TICKS,
};

/// One slot's state the unit declares for the injection tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TargetState {
    /// Fighter slot, `0..fighters`.
    pub fighter: u8,
    /// Health in permille at the injection tick.
    pub hp: u16,
    /// Planar position `[x, z]` in whole millimetres at the injection tick.
    pub pos_mm: [i32; 2],
}

/// One end state the unit rejects as a look-alike success.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NearMiss {
    /// The end state must not carry this health for this slot.
    Hp {
        /// Fighter slot the value belongs to.
        fighter: u8,
        /// Health in permille.
        hp: u16,
    },
    /// The end state must not carry this planar position for this slot.
    Pose {
        /// Fighter slot the position belongs to.
        fighter: u8,
        /// Planar position `[x, z]` in whole millimetres.
        pos_mm: [i32; 2],
    },
    /// The end state must not have this slot knocked out.
    Down {
        /// Fighter slot.
        fighter: u8,
    },
}

/// One injected verification unit: a gameplay claim plus the runtime state
/// the engine must hold when the interaction starts.
///
/// The spec carries everything the fork track already judges: the id, the
/// claim, the seed, the fighter count, the extent, the injection prefix
/// (`prefix_ticks` plus `setup`), the bounded window (`window_ticks` plus
/// `window`) and the outcome asserts (`asserts`). This type adds the two
/// parts the fork track has no place for: the declared state the injection
/// must reach, and the near-miss end states that must not pass.
#[derive(Clone, PartialEq, Debug)]
pub struct Unit {
    /// The keypoint spec under test.
    pub spec: KeypointSpec,
    /// The state each slot must hold at the injection tick. Empty means the
    /// unit declares no prestate and only the outcome is judged.
    pub targets: Vec<TargetState>,
    /// End states that must not be mistaken for success.
    pub near_miss: Vec<NearMiss>,
}

/// The state the engine actually held at the injection tick.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InjectedState {
    /// Fighter slot.
    pub fighter: u8,
    /// Health in permille.
    pub hp: u16,
    /// Planar position `[x, z]` in whole millimetres.
    pub pos_mm: [i32; 2],
}

/// The verdict of one unit. `pass` is true exactly when `failures` is
/// empty, and every failure line names the keypoint it belongs to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UnitVerdict {
    /// The keypoint id the verdict names.
    pub id: String,
    /// True when the injection landed and every check held.
    pub pass: bool,
    /// One line per failed check, sorted; each names the keypoint.
    pub failures: Vec<String>,
    /// The state the engine actually held at the injection tick.
    pub injected: Vec<InjectedState>,
    /// FNV digest over the integer run evidence.
    pub digest: u64,
}

impl UnitVerdict {
    /// Canonical text: the keypoint id, the verdict and every reason, one
    /// per line. The same unit twice gives the same bytes.
    pub fn to_text(&self) -> String {
        let mut text = format!(
            "keypoint={} pass={} digest={:016x} injected={}\n",
            self.id,
            self.pass,
            self.digest,
            self.injected.len()
        );
        for failure in &self.failures {
            text.push_str("fail ");
            text.push_str(failure);
            text.push('\n');
        }
        text
    }
}

/// Unit failures: a unit the fork track cannot judge, or an arena that
/// refused the built tapes. Both name the keypoint.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum UnitError {
    /// The unit carries something this track cannot judge.
    BadUnit {
        /// The keypoint id.
        id: String,
        /// What was wrong with it.
        reason: String,
    },
    /// The arena refused the built tapes.
    ArenaRefused {
        /// The keypoint id.
        id: String,
    },
}

impl core::fmt::Display for UnitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadUnit { id, reason } => write!(f, "unit `{id}` cannot run: {reason}"),
            Self::ArenaRefused { id } => write!(f, "arena refused unit `{id}`"),
        }
    }
}

impl std::error::Error for UnitError {}

/// Judge one unit: reach the declared state, run the bounded window, then
/// check the outcome, the declared prestate and the near-miss set.
///
/// The verdict can be wrong: a state the engine did not reach, a failed
/// assert or a rejected near-miss each add a reason naming the keypoint.
pub fn verify_unit(unit: &Unit) -> Result<UnitVerdict, UnitError> {
    let spec = &unit.spec;
    check_shape(unit)?;
    let run = run_unit(unit)?;

    let mut failures = Vec::new();
    failures.extend(check_injection(unit, &run.injected));
    for line in check_asserts(
        spec,
        spec.prefix_ticks,
        spec.window_ticks,
        &run.dataset,
        &run.metrics,
    ) {
        failures.push(format!("{}: {line}", spec.id));
    }
    failures.extend(check_near_misses(unit, &run.settle));

    let pass = failures.is_empty();
    failures.sort();
    let digest = fold_digest(spec, &run.injected, &run.metrics, pass);
    Ok(UnitVerdict {
        id: spec.id.clone(),
        pass,
        failures,
        injected: run.injected,
        digest,
    })
}

/// Refuse a unit this track cannot judge, before it costs a run.
fn check_shape(unit: &Unit) -> Result<(), UnitError> {
    let spec = &unit.spec;
    let bad = |reason: &str| UnitError::BadUnit {
        id: spec.id.clone(),
        reason: reason.to_owned(),
    };
    if spec.spine {
        return Err(bad("the injected unit judges the fork track only"));
    }
    if !spec.walls.is_empty() {
        return Err(bad("walls need the spine track"));
    }
    if !spec.freeze.is_empty() {
        return Err(bad("a frozen slot is not judged by the injected unit"));
    }
    if spec.asserts.reads_spine() {
        return Err(bad("spine-only asserts need the spine track"));
    }
    if spec.fighters < 2 {
        return Err(bad("a unit needs at least two fighters"));
    }
    if spec.prefix_ticks > MAX_PREFIX_TICKS {
        return Err(bad("the injection prefix is over its bound"));
    }
    if spec.window_ticks == 0 || spec.window_ticks > MAX_WINDOW_TICKS {
        return Err(bad("the window is outside its bound"));
    }
    for target in &unit.targets {
        if target.fighter >= spec.fighters {
            return Err(bad("a target names a slot outside the fight"));
        }
    }
    for near in &unit.near_miss {
        let slot = match near {
            NearMiss::Hp { fighter, .. } | NearMiss::Pose { fighter, .. } => *fighter,
            NearMiss::Down { fighter } => *fighter,
        };
        if slot >= spec.fighters {
            return Err(bad("a near-miss names a slot outside the fight"));
        }
    }
    if unit.targets.is_empty() && unit.near_miss.is_empty() && asserts_empty(&spec.asserts) {
        return Err(bad("a unit with no check is never a pass"));
    }
    Ok(())
}

/// What one run of a unit produced: the recorded dataset, the fork metrics
/// and the states at the injection tick and at the end of the window.
struct UnitRun {
    /// Every recorded tick of the run.
    dataset: engine_arena::recorder::Dataset,
    /// The fork metrics of the window.
    metrics: engine_arena::fork::ForkMetrics,
    /// The state the engine held at the injection tick.
    injected: Vec<InjectedState>,
    /// The state the engine held at the end of the window.
    settle: Vec<InjectedState>,
}

/// Reach the declared state and run the bounded window.
fn run_unit(unit: &Unit) -> Result<UnitRun, UnitError> {
    let spec = &unit.spec;
    let prefix = spec.prefix_ticks;
    let window = spec.window_ticks;
    let total = prefix.saturating_add(window).saturating_add(1);
    let config = ArenaConfig {
        seed: spec.seed,
        ticks: total,
        fighters: spec.fighters,
        half_extent_m: spec.half_extent_m,
        table_version: 1,
    };
    let by_key = |inputs: &[ScriptInput]| -> BTreeMap<(u32, u8), ScriptInput> {
        inputs
            .iter()
            .map(|entry| ((entry.tick, entry.fighter), *entry))
            .collect()
    };
    // Full coverage: every `(tick, slot)` runs the scripted override or
    // idle, so brains are never consulted and the run is exact.
    let setup_tape =
        materialize(&by_key(&spec.setup), 0, prefix, spec.fighters).ok_or_else(|| {
            UnitError::ArenaRefused {
                id: spec.id.clone(),
            }
        })?;
    let window_tape = materialize(
        &by_key(&spec.window),
        prefix,
        prefix.saturating_add(window).saturating_add(1),
        spec.fighters,
    )
    .ok_or_else(|| UnitError::ArenaRefused {
        id: spec.id.clone(),
    })?;

    let (snapshot, _) =
        run_prefix(&config, &setup_tape, prefix).ok_or_else(|| UnitError::ArenaRefused {
            id: spec.id.clone(),
        })?;
    let mut fork =
        Fork::from_snapshot(snapshot, prefix).ok_or_else(|| UnitError::ArenaRefused {
            id: spec.id.clone(),
        })?;
    let mut window_full: BTreeMap<u8, Vec<FighterInput>> = BTreeMap::new();
    for input in &window_tape.0 {
        window_full.entry(input.fighter.0).or_default().push(*input);
    }
    for slot in 0..spec.fighters {
        let inputs = window_full
            .remove(&slot)
            .ok_or_else(|| UnitError::ArenaRefused {
                id: spec.id.clone(),
            })?;
        fork.apply_proposal(ForkProposal::SwapTape {
            fighter: slot,
            inputs,
        })
        .map_err(|_| UnitError::ArenaRefused {
            id: spec.id.clone(),
        })?;
    }
    let metrics = fork.run_n(window);
    let (_, dataset) = fork.run_recorded(window.saturating_add(1));

    let injected = state_at(&dataset, prefix);
    let settle = state_at(&dataset, prefix.saturating_add(window));
    Ok(UnitRun {
        dataset,
        metrics,
        injected,
        settle,
    })
}

/// The declared prestate: the engine must have held it at the injection
/// tick. A state the engine never held is the failure the unit exists to
/// catch.
fn check_injection(unit: &Unit, injected: &[InjectedState]) -> Vec<String> {
    let spec = &unit.spec;
    let mut failures = Vec::new();
    for target in &unit.targets {
        let Some(reached) = injected
            .iter()
            .find(|state| state.fighter == target.fighter)
        else {
            failures.push(format!(
                "{}: injection fighter {}: no observation",
                spec.id, target.fighter
            ));
            continue;
        };
        if reached.hp != target.hp {
            failures.push(format!(
                "{}: injection hp fighter {}: expected {} observed {}",
                spec.id, target.fighter, target.hp, reached.hp
            ));
        }
        if reached.pos_mm != target.pos_mm {
            failures.push(format!(
                "{}: injection pos_mm fighter {}: expected {:?} observed {:?}",
                spec.id, target.fighter, target.pos_mm, reached.pos_mm
            ));
        }
    }
    failures
}

/// The near-miss set: a look-alike end state is not success.
fn check_near_misses(unit: &Unit, settle: &[InjectedState]) -> Vec<String> {
    let spec = &unit.spec;
    let mut failures = Vec::new();
    for near in &unit.near_miss {
        match *near {
            NearMiss::Hp { fighter, hp } => {
                if let Some(state) = settle.iter().find(|state| state.fighter == fighter) {
                    if state.hp == hp {
                        failures.push(format!(
                            "{}: near miss hp fighter {}: {} is not success",
                            spec.id, fighter, hp
                        ));
                    }
                }
            }
            NearMiss::Pose { fighter, pos_mm } => {
                if let Some(state) = settle.iter().find(|state| state.fighter == fighter) {
                    if state.pos_mm == pos_mm {
                        failures.push(format!(
                            "{}: near miss pos_mm fighter {}: {:?} is not success",
                            spec.id, fighter, pos_mm
                        ));
                    }
                }
            }
            NearMiss::Down { fighter } => {
                if let Some(state) = settle.iter().find(|state| state.fighter == fighter) {
                    if state.hp == 0 {
                        failures.push(format!(
                            "{}: near miss fighter {}: down is not success",
                            spec.id, fighter
                        ));
                    }
                }
            }
        }
    }
    failures
}

/// Judge every unit and keep the order. One bad unit does not stop the
/// rest: it becomes a failing verdict with its reason, so a batch never
/// loses a keypoint to a neighbour's refusal.
pub fn verify_all(units: &[Unit]) -> Vec<UnitVerdict> {
    units
        .iter()
        .map(|unit| match verify_unit(unit) {
            Ok(verdict) => verdict,
            Err(error) => {
                let id = unit.spec.id.clone();
                let mut digester = Digester::new();
                digester.text(&id);
                digester.u8(0);
                UnitVerdict {
                    id: id.clone(),
                    pass: false,
                    failures: vec![format!("{id}: {error}")],
                    injected: Vec::new(),
                    digest: digester.finish(),
                }
            }
        })
        .collect()
}

/// True when the assert set carries no field at all.
fn asserts_empty(asserts: &KeypointAsserts) -> bool {
    asserts.hp.is_empty()
        && asserts.damage.is_empty()
        && asserts.stuck.is_empty()
        && asserts.killed.is_none()
        && asserts.damage_ticks.is_none()
        && asserts.ticks_to_kill.is_none()
        && asserts.state.is_empty()
        && asserts.stamina.is_empty()
        && asserts.pos_mm.is_empty()
}

/// Every slot's state at one tick, in slot order.
fn state_at(dataset: &engine_arena::recorder::Dataset, tick: u32) -> Vec<InjectedState> {
    dataset
        .rows
        .iter()
        .find(|row| row.observation.tick == tick)
        .map(|row| {
            row.observation
                .fighters
                .iter()
                .map(|entry| InjectedState {
                    fighter: entry.id,
                    hp: entry.hp,
                    pos_mm: [
                        super::spine::mm(entry.pos[0]),
                        super::spine::mm(entry.pos[2]),
                    ],
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Fold the integer run evidence: the spec header, the reached state and
/// the fork metrics. Never a float, so the digest is stable.
fn fold_digest(
    spec: &KeypointSpec,
    injected: &[InjectedState],
    metrics: &engine_arena::fork::ForkMetrics,
    pass: bool,
) -> u64 {
    let mut digester = Digester::new();
    digester.text(&spec.id);
    digester.u64(spec.seed);
    digester.u8(spec.fighters);
    digester.u32(spec.prefix_ticks);
    digester.u32(spec.window_ticks);
    digester.u8(u8::from(pass));
    for state in injected {
        digester.u8(state.fighter);
        digester.u16(state.hp);
        digester.bytes(&state.pos_mm[0].to_le_bytes());
        digester.bytes(&state.pos_mm[1].to_le_bytes());
    }
    for (slot, damage) in &metrics.damage_dealt {
        digester.u8(*slot);
        digester.u16(*damage);
    }
    for (slot, stuck) in &metrics.stuck_ticks {
        digester.u8(*slot);
        digester.u32(*stuck);
    }
    digester.u32(metrics.ticks_to_kill.unwrap_or(u32::MAX));
    digester.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A two-fighter unit: both slots idle, so the state at the injection
    /// tick is the state the fight starts in.
    fn unit(id: &str) -> Unit {
        Unit {
            spec: KeypointSpec {
                spine: false,
                walls: Vec::new(),
                id: id.to_owned(),
                claim: "the fight holds its breath".to_owned(),
                seed: 2040,
                fighters: 2,
                half_extent_m: 20.0,
                prefix_ticks: 4,
                setup: Vec::new(),
                window_ticks: 8,
                window: Vec::new(),
                freeze: Vec::new(),
                asserts: KeypointAsserts {
                    hp: vec![(0, 1000), (1, 1000)],
                    ..KeypointAsserts::default()
                },
            },
            targets: Vec::new(),
            near_miss: Vec::new(),
        }
    }

    /// The state a unit reaches, read back from a first run so a later
    /// target can state it exactly.
    fn reached(id: &str) -> Vec<InjectedState> {
        let verdict = verify_unit(&unit(id)).expect("unit runs");
        verdict.injected
    }

    #[test]
    fn injected_unit_returns_a_verdict() {
        let verdict = verify_unit(&unit("kp-inject-idle")).expect("unit runs");
        assert_eq!(verdict.id, "kp-inject-idle");
        assert!(verdict.pass, "idle fight holds: {:?}", verdict.failures);
        assert_eq!(verdict.injected.len(), 2, "both slots observed");
        assert!(verdict.failures.is_empty());
        println!("{}", verdict.to_text());
    }

    #[test]
    fn injected_state_that_matches_passes() {
        let mut unit = unit("kp-inject-match");
        unit.targets = reached("kp-inject-match")
            .iter()
            .map(|state| TargetState {
                fighter: state.fighter,
                hp: state.hp,
                pos_mm: state.pos_mm,
            })
            .collect();
        let verdict = verify_unit(&unit).expect("unit runs");
        assert!(
            verdict.pass,
            "declared state reached: {:?}",
            verdict.failures
        );
    }

    #[test]
    fn wrong_pose_fails_with_the_reason_and_the_keypoint_named() {
        let mut unit = unit("kp-inject-wrong-pose");
        unit.targets = reached("kp-inject-wrong-pose")
            .iter()
            .map(|state| TargetState {
                fighter: state.fighter,
                hp: state.hp,
                pos_mm: [state.pos_mm[0] + 1000, state.pos_mm[1]],
            })
            .collect();
        let verdict = verify_unit(&unit).expect("unit runs");
        assert!(!verdict.pass, "a pose the fight never held must fail");
        let reason = verdict
            .failures
            .iter()
            .find(|line| line.contains("pos_mm"))
            .expect("the pose failure names itself");
        assert!(
            reason.contains("kp-inject-wrong-pose"),
            "the reason names the keypoint: {reason}"
        );
        assert!(
            reason.contains("expected") && reason.contains("observed"),
            "the reason says what was expected and what was seen: {reason}"
        );
        let text = verdict.to_text();
        assert!(text.contains("kp-inject-wrong-pose"), "text names it");
        println!("{text}");
    }

    #[test]
    fn verdict_bytes_are_the_same_twice() {
        let mut unit = unit("kp-inject-twice");
        unit.targets = reached("kp-inject-twice")
            .iter()
            .map(|state| TargetState {
                fighter: state.fighter,
                hp: state.hp,
                pos_mm: state.pos_mm,
            })
            .collect();
        let first = verify_unit(&unit).expect("first run");
        let second = verify_unit(&unit).expect("second run");
        assert_eq!(first, second, "the same unit gives the same verdict");
        assert_eq!(first.to_text(), second.to_text(), "same bytes");
        assert_eq!(first.digest, second.digest, "same digest");
    }

    #[test]
    fn near_miss_end_state_is_not_success() {
        let mut unit = unit("kp-inject-near-miss");
        let state = reached("kp-inject-near-miss")
            .first()
            .copied()
            .expect("state");
        unit.near_miss = vec![NearMiss::Hp {
            fighter: state.fighter,
            hp: state.hp,
        }];
        let verdict = verify_unit(&unit).expect("unit runs");
        assert!(!verdict.pass, "a near-miss end state must fail");
        let reason = verdict
            .failures
            .iter()
            .find(|line| line.contains("near miss"))
            .expect("the near-miss names itself");
        assert!(reason.contains("kp-inject-near-miss"), "{reason}");
    }

    #[test]
    fn a_unit_with_no_check_is_refused() {
        let mut unit = unit("kp-inject-empty");
        unit.spec.asserts = KeypointAsserts::default();
        let error = verify_unit(&unit).expect_err("an empty unit is refused");
        assert!(
            matches!(error, UnitError::BadUnit { .. }),
            "the refusal is typed: {error}"
        );
    }

    #[test]
    fn a_spine_unit_is_refused_by_name() {
        let mut unit = unit("kp-inject-spine");
        unit.spec.spine = true;
        let error = verify_unit(&unit).expect_err("a spine unit is refused");
        assert!(error.to_string().contains("kp-inject-spine"), "{error}");
    }

    #[test]
    fn one_bad_unit_does_not_stop_the_batch() {
        let mut bad = unit("kp-inject-bad");
        bad.spec.spine = true;
        let good = unit("kp-inject-good");
        let verdicts = verify_all(&[bad, good]);
        assert_eq!(verdicts.len(), 2);
        assert!(!verdicts[0].pass, "the bad unit fails");
        assert!(verdicts[1].pass, "the good unit still runs");
        assert!(
            verdicts[0].failures[0].contains("kp-inject-bad"),
            "the reason names the keypoint"
        );
    }

    #[test]
    fn the_unit_makes_no_model_call() {
        // The production half of this file must not reach a model: the
        // unit is judged by the arena alone.
        let source = include_str!("inject.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("test module marker");
        for banned in [
            "provider",
            "Provider",
            "llm",
            "Llm",
            "anthropic",
            "openai",
            "ollama",
            "gemini",
            "model",
            "Model",
        ] {
            assert!(
                !production.contains(banned),
                "the injected unit must not name `{banned}`"
            );
        }
    }
}
