//! Named replay scenarios: one per system under test.
//!
//! Every file under `content/playtest/replay` uses the fixture shape
//! [`crate::scenario_shape`] reads, plus a `bout` block and a recorded
//! `digest` assertion. The corpus is embedded at build time, so a replay
//! reads no working directory and no runtime file.
//!
//! A replay builds one [`BodyTape`] from the ordered steps, runs the
//! arena twice (checkpoints for the digest, events for the counts) and folds
//! the config, the canonical tape bytes and every checkpoint into one FNV-64
//! digest. Any changed input bit moves the tape bytes, so the digest moves
//! too: a scenario that no longer matches its recorded digest fails.
//!
//! Units: fixed ticks and integer slots; the digest is a unitless `u64`.
//! Determinism: same file, same digest, twice in a row; no wall clock, no
//! hash-map order.

use crate::agent::BodyError;
use crate::scenario_shape::{parse, Bout, Scenario, ShapeError, Step};
use engine_arena::{
    run, run_events, ArenaConfig, BodyId, BodyInput, BodyTape, Checkpoint, TickEvents,
};
use engine_combat::FightEvent;
use engine_core::hash::FNV64_OFFSET;
use engine_core::Fnv64;
use engine_input::fighter::{Buttons, ATTACK, BLOCK, DODGE, JUMP, THROW_SPEAR};

/// The committed corpus as `(name, text)`, sorted by name.
///
/// The name repeats the file's `name` field so a lookup never parses the
/// whole corpus first; a test holds the two equal.
pub const SCENARIOS: [(&str, &str); 6] = [
    (
        "combat.dodge",
        include_str!("../../../content/playtest/replay/combat.dodge.json"),
    ),
    (
        "combat.knockdown",
        include_str!("../../../content/playtest/replay/combat.knockdown.json"),
    ),
    (
        "crowd.march",
        include_str!("../../../content/playtest/replay/crowd.march.json"),
    ),
    (
        "duel.exchange",
        include_str!("../../../content/playtest/replay/duel.exchange.json"),
    ),
    (
        "parry.window",
        include_str!("../../../content/playtest/replay/parry.window.json"),
    ),
    (
        "stamina.mash",
        include_str!("../../../content/playtest/replay/stamina.mash.json"),
    ),
];

/// Corpus names in table order.
#[must_use]
pub fn names() -> Vec<&'static str> {
    SCENARIOS.iter().map(|(name, _)| *name).collect()
}

/// Fixture text for one corpus name, or `None` when the name is unknown.
#[must_use]
pub fn find(name: &str) -> Option<&'static str> {
    SCENARIOS
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, text)| *text)
}

/// Combat outcomes counted over one replay, one field per [`FightEvent`].
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct EventCounts {
    /// Clean hits that landed.
    pub hits: u32,
    /// Hits soaked by a held block.
    pub blocked: u32,
    /// Hits caught inside the parry window.
    pub parried: u32,
    /// Hits negated by dodge i-frames.
    pub dodged: u32,
    /// Victims dropped to zero hp.
    pub knockdowns: u32,
    /// Actions refused for lack of stamina.
    pub stamina_out: u32,
}

impl EventCounts {
    /// Total outcomes counted.
    #[must_use]
    pub const fn total(self) -> u32 {
        self.hits + self.blocked + self.parried + self.dodged + self.knockdowns + self.stamina_out
    }

    /// Count one event into its field.
    fn add(&mut self, event: FightEvent) {
        match event {
            FightEvent::Hit { .. } => self.hits = self.hits.saturating_add(1),
            FightEvent::Blocked { .. } => self.blocked = self.blocked.saturating_add(1),
            FightEvent::Parried { .. } => self.parried = self.parried.saturating_add(1),
            FightEvent::Dodged { .. } => self.dodged = self.dodged.saturating_add(1),
            FightEvent::Knockdown { .. } => self.knockdowns = self.knockdowns.saturating_add(1),
            FightEvent::StaminaOut { .. } => self.stamina_out = self.stamina_out.saturating_add(1),
        }
    }
}

/// One replay outcome: what ran, what it hashed to and whether it matched.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReplayReport {
    /// Scenario name.
    pub name: String,
    /// System under test named by the scenario.
    pub system: String,
    /// Fixed ticks the bout ran.
    pub ticks: u32,
    /// Digest this replay observed.
    pub digest: u64,
    /// Digest the scenario records.
    pub recorded: u64,
    /// Event counts this replay observed.
    pub events: EventCounts,
    /// True when every assertion matched.
    pub pass: bool,
    /// One line per assertion that did not match.
    pub mismatches: Vec<String>,
}

impl ReplayReport {
    /// One compact receipt line, the CLI's only output for a scenario.
    #[must_use]
    pub fn receipt(&self) -> String {
        let verdict = if self.pass { "PASS" } else { "FAIL" };
        let name = &self.name;
        let ticks = self.ticks;
        let digest = self.digest;
        let recorded = self.recorded;
        let counts = &self.events;
        let hits = counts.hits;
        let blocked = counts.blocked;
        let parried = counts.parried;
        let dodged = counts.dodged;
        let knockdowns = counts.knockdowns;
        let stamina_out = counts.stamina_out;
        format!(
            "{verdict} {name} ticks={ticks} digest={digest:016x} recorded={recorded:016x} \
             hits={hits} blocked={blocked} parried={parried} dodged={dodged} \
             knockdowns={knockdowns} stamina_out={stamina_out}"
        )
    }
}

/// What stopped a replay. Every message names the legal shape.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CorpusError {
    /// No corpus file has this name.
    UnknownScenario(String),
    /// The fixture text does not parse.
    Shape(ShapeError),
    /// The scenario has no `bout`, so it cannot be replayed.
    MissingBout,
    /// A step input name is not one of the mapped actions.
    UnknownInput {
        /// 1-based step index.
        step: usize,
        /// The name as written.
        name: String,
    },
    /// A step names a slot outside the bout.
    BadSlot {
        /// 1-based step index.
        step: usize,
        /// The slot as written.
        slot: u8,
    },
    /// A step would write ticks past the bout end.
    StepPastBout {
        /// 1-based step index.
        step: usize,
        /// First tick the step writes.
        at: u32,
        /// Ticks the step holds.
        ticks: u32,
    },
    /// The arena refused the built tape.
    Tape(BodyError),
    /// No step has an input, so there is nothing to replay.
    NoInputs,
    /// The run produced fewer checkpoints than the bout asks for.
    ShortRun {
        /// Checkpoints the run produced.
        got: usize,
        /// Ticks the bout asks for.
        want: u32,
    },
    /// The digest is the FNV offset basis: nothing was folded in.
    VacuousDigest,
    /// The scenario records no `digest` assertion.
    NoRecordedDigest,
    /// An assertion names a check the replay does not own.
    UnknownCheck(String),
    /// An assertion value is not a decimal or `0x` integer.
    BadExpect {
        /// Assertion name.
        check: String,
        /// Value as written.
        expect: String,
    },
}

impl core::fmt::Display for CorpusError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownScenario(name) => {
                write!(f, "unknown scenario `{name}`: run --list for the corpus names")
            }
            Self::Shape(error) => write!(f, "scenario shape refused: {error}"),
            Self::MissingBout => write!(
                f,
                "scenario has no `bout`: need ticks, fighters, half_extent_m and table_version"
            ),
            Self::UnknownInput { step, name } => write!(
                f,
                "step {step} input `{name}` is not one of neutral, attack, block, dodge, jump, throw"
            ),
            Self::BadSlot { step, slot } => write!(
                f,
                "step {step} slot {slot} is not inside the bout"
            ),
            Self::StepPastBout { step, at, ticks } => write!(
                f,
                "step {step} writes ticks {at}..={} past the bout end",
                at.saturating_add(*ticks)
            ),
            Self::Tape(error) => write!(f, "tape refused: {error:?}"),
            Self::NoInputs => write!(f, "scenario has no input step: need at least one"),
            Self::ShortRun { got, want } => {
                write!(f, "run produced {got} checkpoints, bout asks {want}")
            }
            Self::VacuousDigest => write!(
                f,
                "digest equals the FNV offset basis: the run folded nothing"
            ),
            Self::NoRecordedDigest => write!(
                f,
                "scenario records no `digest` assertion: record one from a passing run"
            ),
            Self::UnknownCheck(name) => write!(
                f,
                "assertion `{name}` is not one of digest, ticks, hits, blocked, parried, \
                 dodged, knockdowns, stamina_out"
            ),
            Self::BadExpect { check, expect } => write!(
                f,
                "assertion `{check}` expect `{expect}` is not a decimal or 0x integer"
            ),
        }
    }
}

impl std::error::Error for CorpusError {}

impl From<ShapeError> for CorpusError {
    fn from(error: ShapeError) -> Self {
        Self::Shape(error)
    }
}

/// Replay one named scenario from the embedded corpus.
///
/// Errors: [`CorpusError::UnknownScenario`] for a name outside the corpus;
/// every other variant names what the fixture or the run got wrong.
pub fn replay(name: &str) -> Result<ReplayReport, CorpusError> {
    let text = find(name).ok_or_else(|| CorpusError::UnknownScenario(name.to_string()))?;
    replay_text(text)
}

/// Replay one scenario text (the corpus files and the tamper test both use it).
fn replay_text(text: &str) -> Result<ReplayReport, CorpusError> {
    let scenario = parse(text)?;
    let bout = scenario.bout.ok_or(CorpusError::MissingBout)?;
    let config = config_of(scenario.seed, &bout);
    let tape = build_tape(&scenario, &bout)?;
    let checkpoints = run(&config, &tape);
    if checkpoints.len() != usize::try_from(bout.ticks).unwrap_or(usize::MAX) {
        return Err(CorpusError::ShortRun {
            got: checkpoints.len(),
            want: bout.ticks,
        });
    }
    let events = count_events(&run_events(&config, &tape));
    let digest = digest_of(&config, &tape, &checkpoints);
    if digest == FNV64_OFFSET {
        return Err(CorpusError::VacuousDigest);
    }
    let checked = check_assertions(&scenario, bout.ticks, events, digest)?;
    Ok(ReplayReport {
        name: scenario.name,
        system: scenario.system,
        ticks: bout.ticks,
        digest,
        recorded: checked.recorded,
        events,
        pass: checked.pass,
        mismatches: checked.mismatches,
    })
}

/// Arena settings for one bout.
fn config_of(seed: u64, bout: &Bout) -> ArenaConfig {
    ArenaConfig {
        seed,
        ticks: bout.ticks,
        fighters: bout.fighters,
        half_extent_m: f32::from(bout.half_extent_m),
        table_version: bout.table_version,
    }
}

/// The buttons one step name holds; `None` for a name outside the map.
fn buttons_of(name: &str) -> Option<Buttons> {
    let bits = match name {
        "neutral" => 0,
        "attack" | "jab" => ATTACK,
        "block" | "guard" => BLOCK,
        "dodge" => DODGE,
        "jump" => JUMP,
        "throw" | "throw_spear" => THROW_SPEAR,
        _ => return None,
    };
    Some(Buttons(bits))
}

/// Build the sorted tape the steps describe.
///
/// A step starts at its absolute `at` when written, else at the running
/// cursor; its `ticks` ticks are written for its slot, and the cursor moves
/// past them. Steps may overlap only on different slots.
fn build_tape(scenario: &Scenario, bout: &Bout) -> Result<BodyTape, CorpusError> {
    let mut cursor = 0u32;
    let mut inputs = Vec::new();
    for (index, step) in scenario.steps.iter().enumerate() {
        let row = index + 1;
        let (start, ticks) = match step {
            Step::Wait { ticks } => (cursor, *ticks),
            Step::Input { at, ticks, .. } => (at.unwrap_or(cursor), *ticks),
        };
        let end = start.checked_add(ticks).ok_or(CorpusError::StepPastBout {
            step: row,
            at: start,
            ticks,
        })?;
        if end > bout.ticks {
            return Err(CorpusError::StepPastBout {
                step: row,
                at: start,
                ticks,
            });
        }
        if let Step::Input {
            name,
            fighter,
            mx,
            mz,
            facing,
            ..
        } = step
        {
            let buttons = buttons_of(name).ok_or_else(|| CorpusError::UnknownInput {
                step: row,
                name: name.clone(),
            })?;
            if *fighter >= bout.fighters {
                return Err(CorpusError::BadSlot {
                    step: row,
                    slot: *fighter,
                });
            }
            let slot = BodyId(*fighter);
            for tick in start..end {
                inputs.push(BodyInput {
                    tick,
                    fighter: slot,
                    mx: *mx,
                    mz: *mz,
                    facing: *facing,
                    buttons,
                });
            }
        }
        cursor = end;
    }
    if inputs.is_empty() {
        return Err(CorpusError::NoInputs);
    }
    inputs.sort_by_key(|input| (input.tick, input.fighter.get()));
    let tape = BodyTape(inputs);
    tape.validate().map_err(CorpusError::Tape)?;
    Ok(tape)
}

/// Count every combat event over the run.
fn count_events(rows: &[TickEvents]) -> EventCounts {
    let mut counts = EventCounts::default();
    for row in rows {
        for event in &row.events {
            counts.add(*event);
        }
    }
    counts
}

/// Fold the config, the canonical tape bytes and every checkpoint into one
/// digest. Any changed input bit or tick state moves it.
fn digest_of(config: &ArenaConfig, tape: &BodyTape, checkpoints: &[Checkpoint]) -> u64 {
    let mut hash = Fnv64::new();
    hash.write_u64(config.seed);
    hash.write_u32(config.ticks);
    hash.write_u8(config.fighters);
    hash.write_f32(config.half_extent_m);
    hash.write_u32(config.table_version);
    hash.write_len32(tape.0.len());
    let mut bytes = Vec::with_capacity(tape.0.len() * 10);
    for input in &tape.0 {
        input.append_bytes(&mut bytes);
    }
    hash.write(&bytes);
    for checkpoint in checkpoints {
        hash.write_u32(checkpoint.tick);
        hash.write_u64(checkpoint.hash);
    }
    hash.finish()
}

/// Assertion outcomes: the recorded digest plus whether every row matched.
struct Checked {
    recorded: u64,
    pass: bool,
    mismatches: Vec<String>,
}

/// Compare every assertion against what the run observed.
fn check_assertions(
    scenario: &Scenario,
    ticks: u32,
    events: EventCounts,
    digest: u64,
) -> Result<Checked, CorpusError> {
    let mut recorded = None;
    let mut pass = true;
    let mut mismatches = Vec::new();
    for assertion in &scenario.assertions {
        let expect = parse_expect(&assertion.expect).ok_or_else(|| CorpusError::BadExpect {
            check: assertion.name.clone(),
            expect: assertion.expect.clone(),
        })?;
        let observed = match assertion.name.as_str() {
            "digest" => {
                recorded = Some(expect);
                digest
            }
            "ticks" => u64::from(ticks),
            "hits" => u64::from(events.hits),
            "blocked" => u64::from(events.blocked),
            "parried" => u64::from(events.parried),
            "dodged" => u64::from(events.dodged),
            "knockdowns" => u64::from(events.knockdowns),
            "stamina_out" => u64::from(events.stamina_out),
            other => return Err(CorpusError::UnknownCheck(other.to_string())),
        };
        if observed != expect {
            pass = false;
            mismatches.push(format!(
                "{}: recorded {expect} observed {observed}",
                assertion.name
            ));
        }
    }
    let recorded = recorded.ok_or(CorpusError::NoRecordedDigest)?;
    Ok(Checked {
        recorded,
        pass,
        mismatches,
    })
}

/// Read an assertion value as a decimal or `0x` integer.
fn parse_expect(text: &str) -> Option<u64> {
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16).ok(),
        None => text.parse::<u64>().ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing(name: &str) -> ReplayReport {
        let report = replay(name).expect("scenario must replay");
        assert!(report.pass, "{name} mismatches: {:?}", report.mismatches);
        report
    }

    #[test]
    fn every_committed_scenario_replays_to_its_digest_twice() {
        let mut systems: Vec<String> = Vec::new();
        for (name, text) in SCENARIOS {
            let scenario = parse(text).expect("corpus file must parse");
            assert_eq!(scenario.name, name, "name field must match its key");
            assert!(!scenario.system.is_empty(), "{name} needs a system");
            assert!(
                !systems.contains(&scenario.system),
                "{name} repeats system {}",
                scenario.system
            );
            systems.push(scenario.system.clone());

            let first = passing(name);
            // Printed with --nocapture: the corpus receipt is the CLI's output.
            println!("{}", first.receipt());
            let second = passing(name);
            assert_eq!(first.digest, second.digest, "{name} digest moved");
            assert_eq!(first.receipt(), second.receipt(), "{name} receipt moved");
            assert_eq!(
                first.recorded, first.digest,
                "{name} must record its digest"
            );
            // Anti-vacuity: real ticks ran and real state was folded.
            assert!(first.ticks > 0, "{name} must run ticks");
            assert_ne!(first.digest, FNV64_OFFSET, "{name} folded nothing");
            if first.system != "locomotion" {
                assert!(
                    first.events.total() > 0,
                    "{name} must fire its signature event"
                );
            }
        }
    }

    #[test]
    fn each_system_fires_the_outcome_it_names() {
        assert!(passing("duel.exchange").events.hits > 0);
        assert_eq!(passing("parry.window").events.parried, 1);
        assert!(passing("parry.window").events.blocked > 0);
        assert!(passing("combat.dodge").events.dodged > 0);
        assert_eq!(passing("combat.knockdown").events.knockdowns, 1);
        assert!(passing("stamina.mash").events.stamina_out > 0);
        assert_eq!(passing("crowd.march").events.total(), 0);
    }

    #[test]
    fn one_changed_input_bit_fails_the_recorded_digest() {
        let text = find("duel.exchange").expect("duel.exchange is committed");
        let tampered = text.replacen("\"ticks\": 1", "\"ticks\": 2", 1);
        assert_ne!(tampered, text, "tamper must change a step");
        let report = replay_text(&tampered).expect("tampered text still replays");
        assert!(!report.pass, "tampered input must fail the recorded digest");
        assert_ne!(report.digest, report.recorded);
        assert!(report.receipt().starts_with("FAIL duel.exchange"));
    }

    #[test]
    fn missing_name_is_a_typed_error() {
        let error = replay("no.such.scenario").expect_err("unknown name must fail");
        assert_eq!(
            error,
            CorpusError::UnknownScenario("no.such.scenario".to_string())
        );
        let message = error.to_string();
        assert!(message.contains("no.such.scenario"));
        assert!(message.contains("--list"));
    }

    #[test]
    fn bad_fixtures_name_what_is_wrong() {
        let base = find("duel.exchange").expect("duel.exchange is committed");
        let no_bout = base.replace("\"bout\"", "\"bout_removed\"");
        assert_eq!(replay_text(&no_bout), Err(CorpusError::MissingBout));
        let bad_name = base.replace("\"input\": \"attack\"", "\"input\": \"dance\"");
        assert!(matches!(
            replay_text(&bad_name),
            Err(CorpusError::UnknownInput { .. })
        ));
        let past_end = base.replace("\"ticks\": 1,", "\"ticks\": 600,");
        assert!(matches!(
            replay_text(&past_end),
            Err(CorpusError::StepPastBout { .. })
        ));
    }
}
