//! Replays: record input tapes with keyframes, theater playback
//! with frame-step, plus first-divergence attribution.
//!
//! Consumer: the fight replay theater, proof bug bundles and the soak
//! divergence check. Bug-file tapes are [`FighterTape`]s (the bug-file owner
//! `playtest::agent` decodes its bytes to them); this module plays any such
//! tape directly, with no provider or transport import. Data owner: the
//! caller owns configs and tapes; [`ReplayTape`] owns its recorded copy.
//! Units: fixed ticks at [`engine_arena::TICK_HZ`]. Determinism scope:
//! same-machine reproducibility — the same config plus inputs twice gives the
//! identical tape (sorted iteration, deterministic [`engine_arena`] runs, no
//! wall clock). Errors: [`ReplayError`]; bad input rejects loudly and
//! changes nothing.
//!
//! Header identity: a tape pins the world it was recorded against.
//! [`ReplayTape::spec_digest`] is the [`engine_arena::MatchSpec::identity`] of
//! the config, rules and world that produced its checkpoints, and
//! [`ReplayTape::logic_version`] is [`SIM_LOGIC_VERSION`]. Playback refuses a
//! tape from another world or another tick logic by name
//! ([`ReplayError::SpecMismatch`], [`ReplayError::LogicMismatch`], both values
//! carried) instead of surfacing as a bare hash mismatch on a later tick.
//!
//! Budgets: record costs one arena run plus 12 bytes per tick of checkpoints
//! and one 12-byte keyframe per [`KEYFRAME_EVERY`] ticks. Theater
//! frame-step is O(1) with no allocation (counting-allocator test below).

use std::collections::BTreeMap;

use engine_arena::world::MatchWorld;
use engine_arena::{ArenaConfig, Checkpoint, FighterTape, MatchSpec};
use engine_core::diverge::first_divergence;
use engine_core::Fnv64;
use engine_input::FighterInput;

/// Replay envelope version; anything else refuses loudly.
pub const REPLAY_VERSION: u8 = 1;
/// Tick logic version of the arena step the recorded checkpoints came from.
/// Bump it when the step stops reproducing old checkpoints, so a tape made by
/// the old logic refuses as [`ReplayError::LogicMismatch`] instead of
/// diverging mid-playback. Like the envelope, it rides the header and is
/// checked before any tick runs.
pub const SIM_LOGIC_VERSION: u8 = 1;
/// Ticks between keyframes: tick 0, every multiple of this, plus the final
/// tick. Matches the multiplayer plan so theater seeks share one grid.
pub const KEYFRAME_EVERY: u32 = 600;
/// Upper bound on keyframes for the longest legal match.
pub const MAX_KEYFRAMES: usize = (engine_arena::MAX_TICKS / KEYFRAME_EVERY) as usize + 2;

/// One seek point: the authoritative hash at `tick`. Seeks resimulate from
/// the keyframe through [`engine_arena::run_prefix`] plus
/// [`engine_arena::resume`], so a keyframe is a proven resume point, not a
/// stored claim.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Keyframe {
    /// Zero-based fixed tick of this seek point.
    pub tick: u32,
    /// Authoritative state hash at `tick`.
    pub hash: u64,
}

/// Recorded replay: validated inputs plus one checkpoint per tick and the
/// keyframe grid over them.
///
/// The two identity fields below are the header of the steal: a tape says
/// which world and which tick logic its checkpoints belong to. They are not
/// folded into [`ReplayTape::tape_hash`], so every pre-existing recording and
/// the frozen tape digest stay byte-identical.
#[derive(Clone, PartialEq, Debug)]
pub struct ReplayTape {
    /// Must equal [`REPLAY_VERSION`].
    pub version: u8,
    /// Match configuration the checkpoints were recorded under.
    pub config: ArenaConfig,
    /// [`engine_arena::MatchSpec::identity`] of the config, rules and world
    /// the checkpoints were recorded against. `0` marks a tape that carries
    /// no identity (pre-header bytes): [`ReplayTape::check_against`] refuses
    /// it for every spec.
    pub spec_digest: u64,
    /// Must equal [`SIM_LOGIC_VERSION`].
    pub logic_version: u8,
    /// `(tick, fighter)`-sorted human inputs; empty slots were AI-filled.
    pub inputs: Vec<FighterInput>,
    /// One authoritative checkpoint per tick, `config.ticks` long.
    pub checkpoints: Vec<Checkpoint>,
    /// Seek grid: tick 0, every [`KEYFRAME_EVERY`] ticks, plus the final tick.
    pub keyframes: Vec<Keyframe>,
}

/// One attributed input: the probe entry at `(tick, fighter)` whose reset to
/// the baseline entry removes the divergence.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CauseInput {
    /// Fixed tick of the attributed input.
    pub tick: u32,
    /// Fighter slot of the attributed input.
    pub fighter: u8,
    /// The probe input at `(tick, fighter)`.
    pub input: FighterInput,
}

/// First-divergence verdict: the first tick where the probe run differs from
/// the recorded run, plus the 1-minimal cause set (every listed input is
/// individually necessary). `causes` is never empty: a divergent pair always
/// has at least one differing input at or before the divergent tick.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DivergenceReport {
    /// First tick where the probe diverges from the recording.
    pub divergent_tick: u32,
    /// Recorded hash at the divergent tick.
    pub expected: u64,
    /// Probe hash at the divergent tick.
    pub observed: u64,
    /// Minimal causes in `(tick, fighter)` order.
    pub causes: Vec<CauseInput>,
}

impl std::fmt::Display for DivergenceReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "diverged at tick {}: expected {:016x}, observed {:016x}, causes [",
            self.divergent_tick, self.expected, self.observed
        )?;
        for (index, cause) in self.causes.iter().enumerate() {
            if index > 0 {
                write!(f, ", ")?;
            }
            write!(
                f,
                "tick {} fighter {} mx {} mz {}",
                cause.tick, cause.fighter, cause.input.mx, cause.input.mz
            )?;
        }
        write!(f, "]")
    }
}

/// Replay failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReplayError {
    /// Arena config invalid (`ticks`, `fighters` or extent out of range).
    BadConfig,
    /// An input failed range/order/slot validation, or names a tick or slot
    /// outside the match.
    BadInput,
    /// Stored tape inconsistent: wrong version, checkpoint count mismatch,
    /// broken keyframe grid, or a keyframe hash the checkpoints disagree with.
    BadTape,
    /// Playback asked past the final tick (`upto` and `ticks` carried).
    BeyondEnd {
        /// Requested tick.
        upto: u32,
        /// Ticks in the recording.
        ticks: u32,
    },
    /// No keyframe at `index` (`index` and keyframe count carried).
    UnknownKeyframe {
        /// Requested keyframe index.
        index: usize,
        /// Keyframes in the recording.
        keyframes: usize,
    },
    /// Resimulation disagrees with the recording at `tick`; authoritative
    /// (`expected`) and replayed (`observed`) hashes carried.
    Diverged {
        /// First tick where resimulation disagrees.
        tick: u32,
        /// Recorded hash.
        expected: u64,
        /// Replayed hash.
        observed: u64,
    },
    /// The tape names another match world: `recorded` is the identity in the
    /// header, `expected` the identity of the spec playback was asked for.
    /// `tick` is `0` for the header gate (before any tick runs) and the
    /// playhead tick when the gate is reached mid-playback.
    SpecMismatch {
        /// Identity in the tape header.
        recorded: u64,
        /// Identity of the spec playback was asked for.
        expected: u64,
        /// Tick the refusal was raised at.
        tick: u32,
    },
    /// The tape's checkpoints came from another tick logic: `recorded` is the
    /// tape's version, `expected` this build's [`SIM_LOGIC_VERSION`]. `tick`
    /// is `0` for the header gate and the playhead tick mid-playback.
    LogicMismatch {
        /// Logic version in the tape header.
        recorded: u8,
        /// This build's [`SIM_LOGIC_VERSION`].
        expected: u8,
        /// Tick the refusal was raised at.
        tick: u32,
    },
    /// Baseline and probe runs agree: nothing to attribute.
    NoFailure,
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::BadConfig => write!(f, "invalid arena config"),
            Self::BadInput => write!(f, "invalid replay input"),
            Self::BadTape => write!(f, "replay tape inconsistent with its config"),
            Self::BeyondEnd { upto, ticks } => {
                write!(f, "tick {upto} past final tick {}", ticks - 1)
            }
            Self::UnknownKeyframe { index, keyframes } => {
                write!(f, "keyframe {index} out of {keyframes}")
            }
            Self::Diverged {
                tick,
                expected,
                observed,
            } => write!(
                f,
                "replay diverged at tick {tick}: expected {expected:016x}, got {observed:016x}"
            ),
            Self::SpecMismatch {
                recorded,
                expected,
                tick,
            } => write!(
                f,
                "replay spec mismatch at tick {tick}: recorded {recorded:016x}, expected {expected:016x}"
            ),
            Self::LogicMismatch {
                recorded,
                expected,
                tick,
            } => write!(
                f,
                "replay logic version mismatch at tick {tick}: recorded {recorded}, expected {expected}"
            ),
            Self::NoFailure => write!(f, "probe matches the recording"),
        }
    }
}

impl std::error::Error for ReplayError {}

/// Validate one input tape against the match bounds: per-input ranges plus
/// order (the tape contract) and arena bounds (every tick below `ticks`,
/// every slot below `fighters`), mirroring [`engine_arena::run`.
fn validate_inputs(config: &ArenaConfig, inputs: &[FighterInput]) -> Result<(), ReplayError> {
    if config.validate().is_err() {
        return Err(ReplayError::BadConfig);
    }
    if FighterTape(inputs.to_vec()).validate().is_err() {
        return Err(ReplayError::BadInput);
    }
    if inputs
        .iter()
        .any(|input| input.tick >= config.ticks || input.fighter.0 >= config.fighters)
    {
        return Err(ReplayError::BadInput);
    }
    Ok(())
}

/// Keyframe ticks for `ticks`: 0, every [`KEYFRAME_EVERY`] multiple, plus the
/// final tick when it is not already on the grid.
fn keyframe_ticks(ticks: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut tick = 0u32;
    while tick < ticks {
        out.push(tick);
        tick += KEYFRAME_EVERY;
    }
    if out.last() != Some(&(ticks - 1)) {
        out.push(ticks - 1);
    }
    out
}

/// The match [`record_tape`] resimulates under: the config plus the rules it
/// selects on open sand, exactly what [`engine_arena::run`] builds. Its
/// identity is the world digest every recorded tape pins.
fn recorded_spec(config: ArenaConfig) -> MatchSpec {
    MatchSpec::new(config, config.rules(), MatchWorld::empty())
}

/// Resimulate `inputs` under `spec`'s config, rules and world; `None` when the
/// spec cannot build its live match. The frozen arena (`spec.rules` equal to
/// the rules its config selects, open sand, no spear) takes the
/// [`engine_arena::run`] path every existing tape used; any other rules or
/// world steps a [`MatchSpec::live`] match tick by tick.
fn run_under_spec(spec: &MatchSpec, inputs: &[FighterInput]) -> Option<Vec<Checkpoint>> {
    if spec.rules == spec.config.rules() && spec.world_digest() == 0 && !spec.spear {
        return Some(engine_arena::run(
            &spec.config,
            &FighterTape(inputs.to_vec()),
        ));
    }
    let mut live = spec.live()?;
    let mut checkpoints = Vec::new();
    let mut tick = 0u32;
    while tick < spec.config.ticks {
        let tick_inputs: Vec<FighterInput> = inputs
            .iter()
            .copied()
            .filter(|input| input.tick == tick)
            .collect();
        checkpoints.push(live.step(&tick_inputs)?.checkpoint);
        tick += 1;
    }
    Some(checkpoints)
}

/// Assemble one tape: checkpoints plus the keyframe grid over them, stamped
/// with the envelope version, the world identity and the tick logic version.
fn assemble(
    config: ArenaConfig,
    spec_digest: u64,
    inputs: &[FighterInput],
    checkpoints: Vec<Checkpoint>,
) -> ReplayTape {
    let keyframes = keyframe_ticks(config.ticks)
        .into_iter()
        .map(|tick| Keyframe {
            tick,
            hash: checkpoints[tick as usize].hash,
        })
        .collect();
    ReplayTape {
        version: REPLAY_VERSION,
        config,
        spec_digest,
        logic_version: SIM_LOGIC_VERSION,
        inputs: inputs.to_vec(),
        checkpoints,
        keyframes,
    }
}

/// Record one replay: run the authoritative match over `inputs`, keep every
/// checkpoint, and lay the keyframe grid over them. The tape pins the identity
/// of the frozen arena the checkpoints come from, so a later playback under
/// another world refuses by name. Same config plus inputs twice gives the
/// identical tape.
pub fn record_tape(
    config: ArenaConfig,
    inputs: &[FighterInput],
) -> Result<ReplayTape, ReplayError> {
    record_tape_under(&recorded_spec(config), inputs)
}

/// [`record_tape`] under an explicit match `spec`: same shape, but the tape is
/// resimulated through the spec's rules and world and pins that identity.
/// Replaying it later needs the same spec ([`play_tape_under`]).
pub fn record_tape_under(
    spec: &MatchSpec,
    inputs: &[FighterInput],
) -> Result<ReplayTape, ReplayError> {
    validate_inputs(&spec.config, inputs)?;
    let checkpoints = run_under_spec(spec, inputs).ok_or(ReplayError::BadConfig)?;
    if checkpoints.len() != spec.config.ticks as usize {
        return Err(ReplayError::BadConfig);
    }
    Ok(assemble(spec.config, spec.identity(), inputs, checkpoints))
}

/// Check one stored tape without resimulating: version, tick logic version,
/// input validity and bounds, checkpoint count, and the keyframe grid (ticks
/// plus hashes).
fn check_consistency(tape: &ReplayTape) -> Result<(), ReplayError> {
    if tape.version != REPLAY_VERSION {
        return Err(ReplayError::BadTape);
    }
    tape.check_logic(0)?;
    validate_inputs(&tape.config, &tape.inputs)?;
    if tape.checkpoints.len() != tape.config.ticks as usize {
        return Err(ReplayError::BadTape);
    }
    let want = keyframe_ticks(tape.config.ticks);
    if tape.keyframes.len() != want.len() {
        return Err(ReplayError::BadTape);
    }
    for (keyframe, tick) in tape.keyframes.iter().zip(want) {
        if keyframe.tick != tick {
            return Err(ReplayError::BadTape);
        }
        if tape.checkpoints[tick as usize].hash != keyframe.hash {
            return Err(ReplayError::BadTape);
        }
    }
    Ok(())
}

/// Compare `fresh` against the stored checkpoints on the prefix through
/// `upto` and return the recorded checkpoint at `upto`. The first mismatch
/// refuses as [`ReplayError::Diverged`], carrying its tick.
fn compare_prefix(
    tape: &ReplayTape,
    fresh: &[Checkpoint],
    upto: u32,
) -> Result<Checkpoint, ReplayError> {
    if fresh.len() != tape.checkpoints.len() {
        return Err(ReplayError::BadTape);
    }
    for (index, (recorded, replayed)) in tape.checkpoints.iter().zip(fresh.iter()).enumerate() {
        if recorded != replayed {
            return Err(ReplayError::Diverged {
                tick: index as u32,
                expected: recorded.hash,
                observed: replayed.hash,
            });
        }
        if index as u32 == upto {
            return Ok(*recorded);
        }
    }
    Err(ReplayError::BadTape)
}

/// Play one stored tape up to and including `upto`: resimulate the match,
/// compare every checkpoint on the prefix, and return the checkpoint at
/// `upto`. The frozen arena only: a tape recorded under another world refuses
/// as [`ReplayError::SpecMismatch`], play it with [`play_tape_under`]. The
/// first hash mismatch fails as [`ReplayError::Diverged`].
pub fn play_tape(tape: &ReplayTape, upto: u32) -> Result<Checkpoint, ReplayError> {
    check_consistency(tape)?;
    tape.check_frozen()?;
    if upto >= tape.config.ticks {
        return Err(ReplayError::BeyondEnd {
            upto,
            ticks: tape.config.ticks,
        });
    }
    let fresh = engine_arena::run(&tape.config, &FighterTape(tape.inputs.clone()));
    compare_prefix(tape, &fresh, upto)
}

/// [`play_tape`] under an explicit match `spec`: first the world and logic
/// identity gate (a tape from another world refuses as
/// [`ReplayError::SpecMismatch`] before tick 0), then the tape is resimulated
/// through the spec's own rules and world. A hash that moved mid-playback
/// still refuses as [`ReplayError::Diverged`] with its tick.
pub fn play_tape_under(
    spec: &MatchSpec,
    tape: &ReplayTape,
    upto: u32,
) -> Result<Checkpoint, ReplayError> {
    check_consistency(tape)?;
    tape.check_against(spec)?;
    if upto >= tape.config.ticks {
        return Err(ReplayError::BeyondEnd {
            upto,
            ticks: tape.config.ticks,
        });
    }
    let fresh = run_under_spec(spec, &tape.inputs).ok_or(ReplayError::BadTape)?;
    compare_prefix(tape, &fresh, upto)
}

/// Attribute one probe tape against a recorded baseline under `config`:
/// resimulate both, name the first divergent tick, then reset each differing
/// input at or before that tick to baseline one at a time and keep the ones
/// whose reset removes the divergence. Deterministic in the report.
pub fn attribute(
    config: &ArenaConfig,
    baseline: &[FighterInput],
    probe: &[FighterInput],
) -> Result<DivergenceReport, ReplayError> {
    validate_inputs(config, baseline)?;
    validate_inputs(config, probe)?;
    let baseline_run = engine_arena::run(config, &FighterTape(baseline.to_vec()));
    let probe_run = engine_arena::run(config, &FighterTape(probe.to_vec()));
    if baseline_run.len() != config.ticks as usize || probe_run.len() != config.ticks as usize {
        return Err(ReplayError::BadTape);
    }
    let Some(divergent_tick) = first_divergence(&baseline_run, &probe_run) else {
        return Err(ReplayError::NoFailure);
    };
    let expected = baseline_run[divergent_tick as usize].hash;
    let observed = probe_run[divergent_tick as usize].hash;
    // Candidates are exactly the keys where probe differs from baseline:
    // identical tapes resimulate identically, so a divergent pair always has
    // at least one, and only keys at or before the divergent tick can have
    // caused it (the sim reads the past, never the future).
    let baseline_by_key: BTreeMap<(u32, u8), FighterInput> = baseline
        .iter()
        .map(|input| ((input.tick, input.fighter.0), *input))
        .collect();
    let mut causes = Vec::new();
    for input in probe {
        if input.tick > divergent_tick {
            continue;
        }
        let key = (input.tick, input.fighter.0);
        if baseline_by_key.get(&key) == Some(input) {
            continue;
        }
        let mut perturbed = Vec::with_capacity(probe.len());
        for candidate in probe {
            if (candidate.tick, candidate.fighter.0) == key {
                if let Some(base) = baseline_by_key.get(&key) {
                    perturbed.push(*base);
                }
            } else {
                perturbed.push(*candidate);
            }
        }
        if engine_arena::run(config, &FighterTape(perturbed)) == baseline_run {
            causes.push(CauseInput {
                tick: input.tick,
                fighter: input.fighter.0,
                input: *input,
            });
        }
    }
    if causes.is_empty() {
        // Unreachable for a divergent pair (the full reset equals baseline),
        // but a changed contract must fail loudly, never with an empty claim.
        return Err(ReplayError::BadTape);
    }
    Ok(DivergenceReport {
        divergent_tick,
        expected,
        observed,
        causes,
    })
}

impl ReplayTape {
    /// Re-check this tape without resimulating (version, tick logic version,
    /// inputs, checkpoint count, keyframe grid).
    pub fn check(&self) -> Result<(), ReplayError> {
        check_consistency(self)
    }

    /// Refuse this tape unless its checkpoints came from this build's tick
    /// logic. `recorded` is the tape's version, `expected` this build's
    /// [`SIM_LOGIC_VERSION`]; a header refusal names tick `0`. Every playback
    /// path runs it through [`check_consistency`].
    fn check_logic(&self, tick: u32) -> Result<(), ReplayError> {
        if self.logic_version != SIM_LOGIC_VERSION {
            return Err(ReplayError::LogicMismatch {
                recorded: self.logic_version,
                expected: SIM_LOGIC_VERSION,
                tick,
            });
        }
        Ok(())
    }

    /// Refuse this tape unless it was recorded against `spec`'s world. The
    /// tape's identity is `recorded`, the spec's [`engine_arena::MatchSpec::identity`]
    /// is `expected`; a tape that carries no identity (`spec_digest` `0`)
    /// refuses for every spec. Pairs with [`play_tape_under`] and
    /// [`Theater::new_under`], which run it before the first tick.
    pub fn check_against(&self, spec: &MatchSpec) -> Result<(), ReplayError> {
        self.check_logic(0)?;
        let expected = spec.identity();
        if self.spec_digest != expected {
            return Err(ReplayError::SpecMismatch {
                recorded: self.spec_digest,
                expected,
                tick: 0,
            });
        }
        Ok(())
    }

    /// Refuse this tape unless its world is the frozen arena this module
    /// resimulates with ([`engine_arena::run`] over `config`, open sand and
    /// the rules the config selects). Every helper that cannot take a spec —
    /// [`play_tape`], [`ReplayTape::suffix_from_keyframe`],
    /// [`ReplayTape::attribute_probe`] — gates on it, so a tape from another
    /// world stops by name instead of diverging late.
    fn check_frozen(&self) -> Result<(), ReplayError> {
        let expected = recorded_spec(self.config).identity();
        if self.spec_digest != expected {
            return Err(ReplayError::SpecMismatch {
                recorded: self.spec_digest,
                expected,
                tick: 0,
            });
        }
        Ok(())
    }

    /// Deterministic FNV-1a hash over version, config, inputs and
    /// checkpoints: the same tape twice gives the same hash. Integer math
    /// only, folded little-endian in order through the shared [`Fnv64`]
    /// owner; a regression checksum, not an integrity proof.
    pub fn tape_hash(&self) -> u64 {
        let mut fold = Fnv64::new();
        fold.write(&[self.version]);
        fold.write(&self.config.seed.to_le_bytes());
        fold.write(&self.config.ticks.to_le_bytes());
        fold.write(&[self.config.fighters]);
        fold.write(&self.config.half_extent_m.to_bits().to_le_bytes());
        let mut input_bytes = Vec::new();
        for input in &self.inputs {
            input.append_bytes(&mut input_bytes);
        }
        fold.write(&input_bytes);
        for checkpoint in &self.checkpoints {
            fold.write(&checkpoint.tick.to_le_bytes());
            fold.write(&checkpoint.hash.to_le_bytes());
        }
        fold.finish()
    }

    /// Resimulate from the keyframe at `index` through the arena snapshot
    /// path and return the suffix checkpoints from that tick on. The suffix
    /// must equal the stored tail exactly, or the tape fails loudly. The
    /// snapshot path is the frozen arena, so a tape from another world refuses
    /// as [`ReplayError::SpecMismatch`] (use [`play_tape_under`] for those).
    pub fn suffix_from_keyframe(&self, index: usize) -> Result<Vec<Checkpoint>, ReplayError> {
        check_consistency(self)?;
        self.check_frozen()?;
        let Some(keyframe) = self.keyframes.get(index) else {
            return Err(ReplayError::UnknownKeyframe {
                index,
                keyframes: self.keyframes.len(),
            });
        };
        let tape = FighterTape(self.inputs.clone());
        let Some((snapshot, prefix)) = engine_arena::run_prefix(&self.config, &tape, keyframe.tick)
        else {
            return Err(ReplayError::BadTape);
        };
        // The snapshot resumes from the keyframe tick, so its prefix holds
        // exactly the earlier ticks; the joined comparison below proves the
        // hashes.
        if prefix.len() != keyframe.tick as usize {
            return Err(ReplayError::BadTape);
        }
        let suffix = engine_arena::resume(snapshot, &self.config, &tape);
        let mut joined = prefix;
        joined.extend_from_slice(&suffix);
        if joined != self.checkpoints {
            return Err(ReplayError::BadTape);
        }
        Ok(suffix)
    }

    /// Attribute one probe tape against this recording (first divergent tick
    /// plus the minimal input cause set). Attribution resimulates the frozen
    /// arena, so a tape from another world refuses as
    /// [`ReplayError::SpecMismatch`].
    pub fn attribute_probe(&self, probe: &[FighterInput]) -> Result<DivergenceReport, ReplayError> {
        check_consistency(self)?;
        self.check_frozen()?;
        attribute(&self.config, &self.inputs, probe)
    }
}

/// Headless theater: play/pause/frame-step over one recording plus full
/// verification and divergence attribution. Frame-step is O(1) with no
/// allocation: checkpoints are recorded once, stepping only moves a cursor.
pub struct Theater {
    tape: ReplayTape,
    cursor: u32,
    playing: bool,
    /// The match the theater was opened under, when one was named: every
    /// resimulating step then plays that spec's rules and world instead of
    /// the frozen default.
    spec: Option<MatchSpec>,
}

impl Theater {
    /// Open one recording of the frozen arena; inconsistent tapes and tapes
    /// from another world refuse loudly and hold nothing. A tape recorded
    /// under another world opens with [`Theater::new_under`].
    pub fn new(tape: ReplayTape) -> Result<Self, ReplayError> {
        Self::open(tape, None)
    }

    /// [`Theater::new`] under an explicit match `spec`: the tape must carry
    /// that spec's identity ([`ReplayTape::check_against`]) and every
    /// resimulating step ([`Theater::play_to`], [`Theater::verify`]) plays the
    /// spec's own rules and world.
    pub fn new_under(spec: &MatchSpec, tape: ReplayTape) -> Result<Self, ReplayError> {
        Self::open(tape, Some(spec.clone()))
    }

    /// Shared open: consistency, then the identity gate when a spec is named.
    fn open(tape: ReplayTape, spec: Option<MatchSpec>) -> Result<Self, ReplayError> {
        check_consistency(&tape)?;
        match spec.as_ref() {
            Some(spec) => tape.check_against(spec)?,
            None => tape.check_frozen()?,
        }
        Ok(Self {
            tape,
            cursor: 0,
            playing: false,
            spec,
        })
    }

    /// The open recording.
    pub fn tape(&self) -> &ReplayTape {
        &self.tape
    }

    /// Ticks in the recording.
    pub fn len(&self) -> u32 {
        self.tape.config.ticks
    }

    /// True while there is at least one tick to show.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Current tick under the playhead.
    pub fn cursor(&self) -> u32 {
        self.cursor
    }

    /// Checkpoint under the playhead.
    pub fn current(&self) -> Checkpoint {
        self.tape.checkpoints[self.cursor as usize]
    }

    /// True while playing (frame-step pauses).
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Start playback.
    pub fn play(&mut self) {
        self.playing = true;
    }

    /// Freeze the playhead.
    pub fn pause(&mut self) {
        self.playing = false;
    }

    /// Back to the first tick, still paused.
    pub fn restart(&mut self) {
        self.cursor = 0;
        self.playing = false;
    }

    /// Advance one tick; pauses playback. `None` on the final tick.
    pub fn step_forward(&mut self) -> Option<Checkpoint> {
        if self.cursor + 1 >= self.len() {
            return None;
        }
        self.cursor += 1;
        self.playing = false;
        Some(self.tape.checkpoints[self.cursor as usize])
    }

    /// Retreat one tick; pauses playback. `None` on the first tick.
    pub fn step_back(&mut self) -> Option<Checkpoint> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        self.playing = false;
        Some(self.tape.checkpoints[self.cursor as usize])
    }

    /// Play to `tick`: resimulate the prefix, compare every checkpoint, and
    /// move the playhead. The first mismatch fails as
    /// [`ReplayError::Diverged`] and the playhead does not move.
    pub fn play_to(&mut self, tick: u32) -> Result<Checkpoint, ReplayError> {
        let point = self.resimulate(tick)?;
        self.cursor = tick;
        self.playing = true;
        Ok(point)
    }

    /// Resimulate the prefix through the open spec (or the frozen arena when
    /// none was named).
    fn resimulate(&self, tick: u32) -> Result<Checkpoint, ReplayError> {
        match self.spec.as_ref() {
            Some(spec) => play_tape_under(spec, &self.tape, tick),
            None => play_tape(&self.tape, tick),
        }
    }

    /// Jump the playhead to the keyframe at `index` (no resimulation; the
    /// checkpoints are already verified by [`Self::verify`]).
    pub fn seek_keyframe(&mut self, index: usize) -> Result<Checkpoint, ReplayError> {
        let Some(keyframe) = self.tape.keyframes.get(index) else {
            return Err(ReplayError::UnknownKeyframe {
                index,
                keyframes: self.tape.keyframes.len(),
            });
        };
        self.cursor = keyframe.tick;
        self.playing = false;
        Ok(self.current())
    }

    /// Keyframe grid of the open recording.
    pub fn keyframes(&self) -> &[Keyframe] {
        &self.tape.keyframes
    }

    /// Full verification: resimulate the whole match and compare every
    /// checkpoint. `Ok` means the recording replays exactly.
    pub fn verify(&self) -> Result<(), ReplayError> {
        self.resimulate(self.len() - 1)?;
        Ok(())
    }

    /// Attribute one probe tape against the open recording: first divergent
    /// tick plus the minimal input cause set (tick plus fighter).
    pub fn attribute_probe(&self, probe: &[FighterInput]) -> Result<DivergenceReport, ReplayError> {
        self.tape.attribute_probe(probe)
    }
}

/// Largest fighter slot named by the input contract, for error-path tests.
#[cfg(test)]
fn invalid_fighter() -> engine_input::FighterId {
    engine_input::FighterId(engine_input::MAX_PARTICIPANTS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_arena::TICK_HZ;
    use engine_input::fighter::Buttons;
    use engine_input::FighterId;

    fn config(ticks: u32) -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks,
            fighters: 2,
            half_extent_m: 20.0,
            // TABLES_VERSION_V1 keeps the golden frozen on v1 rules.
            table_version: 1,
        }
    }

    fn wish(tick: u32, slot: u8, mx: i8) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(slot).expect("slot"),
            mx,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        }
    }

    #[test]
    fn record_plays_back_with_keyframes_and_hash_x2() {
        let cfg = config(64);
        let inputs: Vec<FighterInput> = (0..64)
            .map(|tick| wish(tick, 0, if tick % 20 < 10 { 100 } else { -100 }))
            .collect();
        let first = record_tape(cfg, &inputs).expect("record");
        let second = record_tape(cfg, &inputs).expect("record again");
        // Deterministic hash x2: same complete input twice, identical tape.
        assert_eq!(first, second);
        assert_eq!(first.tape_hash(), second.tape_hash());
        assert_ne!(first.tape_hash(), 0);
        assert_eq!(first.checkpoints.len(), 64);
        // Short match: keyframes at tick 0 plus the final tick.
        assert_eq!(
            first
                .keyframes
                .iter()
                .map(|key| key.tick)
                .collect::<Vec<_>>(),
            vec![0, 63]
        );
        assert_eq!(first.keyframes[0].hash, first.checkpoints[0].hash);
        assert_eq!(first.keyframes[1].hash, first.checkpoints[63].hash);
        // Every prefix tick plays with its recorded hash.
        for upto in [0, 1, 31, 63] {
            let point = play_tape(&first, upto).expect("play");
            assert_eq!(point, first.checkpoints[upto as usize]);
        }
        assert_eq!(
            play_tape(&first, 64),
            Err(ReplayError::BeyondEnd {
                upto: 64,
                ticks: 64
            })
        );
    }

    #[test]
    fn long_match_lays_keyframes_every_600_ticks() {
        let cfg = config(1250);
        let tape = record_tape(cfg, &[]).expect("record");
        assert_eq!(
            tape.keyframes
                .iter()
                .map(|key| key.tick)
                .collect::<Vec<_>>(),
            vec![0, 600, 1200, 1249]
        );
        assert!(tape.keyframes.len() <= MAX_KEYFRAMES);
        for keyframe in &tape.keyframes {
            assert_eq!(tape.checkpoints[keyframe.tick as usize].hash, keyframe.hash);
        }
        // Every keyframe is a proven resume point through the arena snapshot
        // path: prefix plus suffix equals the stored checkpoints exactly.
        for index in 0..tape.keyframes.len() {
            let suffix = tape.suffix_from_keyframe(index).expect("resume");
            let tick = tape.keyframes[index].tick as usize;
            assert_eq!(suffix, tape.checkpoints[tick..]);
        }
        assert_eq!(
            tape.suffix_from_keyframe(tape.keyframes.len()),
            Err(ReplayError::UnknownKeyframe {
                index: 4,
                keyframes: 4
            })
        );
    }

    #[test]
    fn recorded_tape_pins_the_world_identity_and_round_trips_under_it() {
        let cfg = config(64);
        let inputs: Vec<FighterInput> = (0..64).map(|tick| wish(tick, 0, 100)).collect();
        let tape = record_tape(cfg, &inputs).expect("record");
        let spec = recorded_spec(cfg);
        // The header names the world and the tick logic the checkpoints came
        // from.
        assert_eq!(tape.spec_digest, spec.identity());
        assert_ne!(tape.spec_digest, 0);
        assert_eq!(tape.logic_version, SIM_LOGIC_VERSION);
        assert_eq!(tape.check_against(&spec), Ok(()));
        // Under its own spec the tape plays exactly the plain frozen path.
        for upto in [0, 1, 31, 63] {
            assert_eq!(play_tape_under(&spec, &tape, upto), play_tape(&tape, upto));
        }
        let mut theater = Theater::new_under(&spec, tape.clone()).expect("open");
        assert_eq!(theater.play_to(63), Ok(tape.checkpoints[63]));
        assert_eq!(theater.verify(), Ok(()));
    }

    #[test]
    fn another_world_refuses_by_name_naming_both_digests() {
        let cfg = config(32);
        let tape = record_tape(cfg, &[]).expect("record");
        // The kit duel is another world (rules v2 on the kit walls) for the
        // same config: the tape does not belong to it.
        let other = MatchSpec::kit_duel(cfg).expect("kit duel");
        let recorded = tape.spec_digest;
        let expected = other.identity();
        assert_ne!(recorded, expected);
        let refuse = ReplayError::SpecMismatch {
            recorded,
            expected,
            tick: 0,
        };
        assert_eq!(play_tape_under(&other, &tape, 31), Err(refuse));
        assert_eq!(tape.check_against(&other), Err(refuse));
        assert_eq!(Theater::new_under(&other, tape.clone()).err(), Some(refuse));
        // The refusal names both digests in its text.
        let text = refuse.to_string();
        assert!(text.contains(&format!("{recorded:016x}")), "{text}");
        assert!(text.contains(&format!("{expected:016x}")), "{text}");
    }

    #[test]
    fn tape_recorded_under_a_wall_world_replays_only_under_that_spec() {
        let cfg = config(64);
        let kit = MatchSpec::kit_duel(cfg).expect("kit duel");
        let tape = record_tape_under(&kit, &[]).expect("record under the walls");
        assert_eq!(tape.spec_digest, kit.identity());
        assert_ne!(
            tape.checkpoints,
            record_tape(cfg, &[]).expect("frozen").checkpoints,
            "the kit walls move the bout"
        );
        // Recorded under the walls, it replays through the wall world only.
        assert_eq!(play_tape_under(&kit, &tape, 63), Ok(tape.checkpoints[63]));
        assert_eq!(
            Theater::new_under(&kit, tape.clone())
                .expect("open")
                .verify(),
            Ok(())
        );
        // The frozen helpers cannot play another world: they refuse by name
        // instead of diverging on a later tick.
        match play_tape(&tape, 63) {
            Err(ReplayError::SpecMismatch {
                recorded,
                expected,
                tick,
            }) => {
                assert_eq!((recorded, tick), (kit.identity(), 0));
                assert_ne!(expected, recorded);
            }
            other => panic!("the frozen path must refuse another world: {other:?}"),
        }
        assert!(tape.suffix_from_keyframe(0).is_err());
        assert!(Theater::new(tape.clone()).is_err());
        assert!(play_tape_under(&recorded_spec(cfg), &tape, 63).is_err());
    }

    #[test]
    fn another_tick_logic_refuses_by_name_naming_both_versions() {
        let cfg = config(16);
        let tape = record_tape(cfg, &[]).expect("record");
        let mut old = tape.clone();
        old.logic_version = SIM_LOGIC_VERSION.wrapping_add(1);
        let refuse = ReplayError::LogicMismatch {
            recorded: old.logic_version,
            expected: SIM_LOGIC_VERSION,
            tick: 0,
        };
        assert_eq!(old.check(), Err(refuse));
        assert_eq!(play_tape(&old, 15), Err(refuse));
        assert_eq!(old.check_against(&recorded_spec(cfg)), Err(refuse));
        assert!(Theater::new(old.clone()).is_err());
        assert!(Theater::new_under(&recorded_spec(cfg), old.clone()).is_err());
        let text = refuse.to_string();
        assert!(
            text.contains(&format!("recorded {}", old.logic_version)),
            "{text}"
        );
        assert!(
            text.contains(&format!("expected {}", SIM_LOGIC_VERSION)),
            "{text}"
        );
    }

    #[test]
    fn mid_playback_refusals_carry_their_tick() {
        let cfg = config(32);
        let mut tape = record_tape(cfg, &[]).expect("record");
        // Tick 7 is off the keyframe grid, so the cheap check passes and the
        // resimulating paths name the forged tick.
        tape.checkpoints[7].hash ^= 1;
        let spec = recorded_spec(cfg);
        let diverged = ReplayError::Diverged {
            tick: 7,
            expected: tape.checkpoints[7].hash,
            observed: tape.checkpoints[7].hash ^ 1,
        };
        assert_eq!(play_tape(&tape, 31), Err(diverged));
        assert_eq!(play_tape_under(&spec, &tape, 31), Err(diverged));
        // The header gate passes (the world and logic match), so the refusal
        // comes only when the playhead resimulates, and it carries the tick.
        let mut theater = Theater::new_under(&spec, tape).expect("the header opens");
        assert_eq!(theater.play_to(31), Err(diverged));
        assert_eq!(theater.verify(), Err(diverged));
    }

    #[test]
    fn theater_frame_steps_both_ways_with_play_pause() {
        let cfg = config(8);
        let inputs: Vec<FighterInput> = (0..8).map(|tick| wish(tick, 1, 100)).collect();
        let tape = record_tape(cfg, &inputs).expect("record");
        let mut theater = Theater::new(tape).expect("open");
        assert_eq!(theater.len(), 8);
        assert!(!theater.is_empty());
        assert_eq!((theater.cursor(), theater.is_playing()), (0, false));
        assert_eq!(theater.current(), theater.tape().checkpoints[0]);
        assert_eq!(theater.step_back(), None);
        theater.play();
        assert!(theater.is_playing());
        // Frame-step pauses and walks the recorded checkpoints exactly.
        for want in 1..8 {
            assert_eq!(theater.step_forward().map(|point| point.tick), Some(want));
            assert!(!theater.is_playing());
            assert_eq!(theater.cursor(), want);
            assert_eq!(theater.current(), theater.tape().checkpoints[want as usize]);
        }
        assert_eq!(theater.step_forward(), None);
        for want in (0..7).rev() {
            assert_eq!(theater.step_back().map(|point| point.tick), Some(want));
            assert_eq!(theater.cursor(), want);
        }
        // Resimulating play moves the playhead and resumes playing.
        let point = theater.play_to(5).expect("play");
        assert_eq!((theater.cursor(), theater.is_playing()), (5, true));
        assert_eq!(point, theater.current());
        theater.pause();
        assert!(!theater.is_playing());
        // Keyframe seeks land on the grid with matching hashes.
        let keyframes = theater.keyframes().len();
        for index in 0..keyframes {
            let tick = theater.tape().keyframes[index].tick;
            let point = theater.seek_keyframe(index).expect("seek");
            assert_eq!((theater.cursor(), point.tick), (tick, tick));
            assert_eq!(point.hash, theater.tape().keyframes[index].hash);
        }
        assert_eq!(
            theater.seek_keyframe(keyframes),
            Err(ReplayError::UnknownKeyframe {
                index: keyframes,
                keyframes
            })
        );
        theater.restart();
        assert_eq!((theater.cursor(), theater.is_playing()), (0, false));
        theater.verify().expect("recording replays exactly");
    }

    #[test]
    fn frame_step_allocates_nothing() {
        let cfg = config(32);
        let inputs: Vec<FighterInput> = (0..32).map(|tick| wish(tick, 0, -100)).collect();
        let mut theater = Theater::new(record_tape(cfg, &inputs).expect("record")).expect("open");
        let ((), heap) = engine_core::alloc_probe::measure(|| {
            for _ in 0..31 {
                let _ = theater.step_forward();
            }
            for _ in 0..31 {
                let _ = theater.step_back();
            }
        });
        assert_eq!(heap.calls, 0, "frame-step must not allocate");
        assert_eq!(theater.cursor(), 0);
    }

    #[test]
    fn alloc_probe_sees_a_real_allocation() {
        let (_, heap) = engine_core::alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert_eq!(
            heap.calls, 1,
            "the test allocator counts, so zeros are real"
        );
    }

    #[test]
    fn qa1_divert_tape_plays_and_attributes_to_its_input() {
        // Brain-driven control plus one diverted probe input at tick 50 for
        // fighter 1 (the seeded-bug case, built here as plain `FighterTape`
        // rows so the replay owner stays dependency-free).
        let cfg = config(400);
        let control: Vec<FighterInput> = Vec::new();
        let mut probe = control.clone();
        probe.push(FighterInput {
            tick: 50,
            fighter: FighterId::new(1).expect("slot 1"),
            mx: 100,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        });
        let recording = record_tape(cfg, &control).expect("record control");
        // Any bug-file tape plays: the probe resimulates through the same gate.
        let probe_tape = record_tape(cfg, &probe).expect("record probe");
        assert_ne!(recording.checkpoints, probe_tape.checkpoints);
        // Seeded bug attributes to its input: tick plus fighter.
        let report = recording.attribute_probe(&probe).expect("attributes");
        assert_eq!(report.causes.len(), 1, "one cause: {report}");
        assert_eq!(
            (report.causes[0].tick, report.causes[0].fighter),
            (50, 1),
            "cause is the diverted input: {report}"
        );
        assert_eq!(report.causes[0].input.mx, 100);
        assert!(report.divergent_tick >= 50);
        assert_ne!(report.expected, report.observed);
        // The free function agrees, and the report is deterministic.
        assert_eq!(attribute(&cfg, &control, &probe), Ok(report.clone()));
        assert_eq!(recording.attribute_probe(&probe), Ok(report.clone()));
        assert_eq!(
            Theater::new(recording.clone())
                .expect("open")
                .attribute_probe(&probe),
            Ok(report.clone())
        );
        println!("DIVERGENCE EXAMPLE: {report}");
        // A clean probe matches: no failure to attribute.
        assert_eq!(
            recording.attribute_probe(&control),
            Err(ReplayError::NoFailure)
        );
        assert_eq!(
            attribute(&cfg, &control, &control),
            Err(ReplayError::NoFailure)
        );
    }

    #[test]
    fn corrupted_checkpoints_diverge_at_the_first_bad_tick() {
        let cfg = config(16);
        let inputs: Vec<FighterInput> = (0..16).map(|tick| wish(tick, 0, 100)).collect();
        let mut tape = record_tape(cfg, &inputs).expect("record");
        tape.checkpoints[7].hash ^= 1;
        // Keyframe grid still matches, so the cheap check passes and the
        // resimulating play names the forged tick.
        assert_eq!(
            play_tape(&tape, 15),
            Err(ReplayError::Diverged {
                tick: 7,
                expected: tape.checkpoints[7].hash,
                observed: tape.checkpoints[7].hash ^ 1,
            })
        );
        assert!(Theater::new(tape).expect("open").verify().is_err());
    }

    #[test]
    fn bad_configs_inputs_and_tapes_refuse_loudly() {
        // Zero ticks never opens a recording.
        assert_eq!(record_tape(config(0), &[]), Err(ReplayError::BadConfig));
        // Out-of-range wishes, unordered tapes, ticks past the match, and
        // slots past the match all refuse without recording.
        let cfg = config(8);
        let mut wild = wish(0, 0, 100);
        wild.mx = 101;
        assert_eq!(record_tape(cfg, &[wild]), Err(ReplayError::BadInput));
        assert_eq!(
            record_tape(cfg, &[wish(1, 0, 0), wish(0, 0, 0)]),
            Err(ReplayError::BadInput)
        );
        assert_eq!(
            record_tape(cfg, &[wish(8, 0, 0)]),
            Err(ReplayError::BadInput)
        );
        let mut foreign = wish(0, 0, 0);
        foreign.fighter = invalid_fighter();
        assert_eq!(record_tape(cfg, &[foreign]), Err(ReplayError::BadInput));
        // Stored-tape surgery refuses: wrong version, short checkpoints,
        // forged keyframe hash, off-grid keyframes.
        let tape = record_tape(cfg, &[]).expect("record");
        let mut bad_version = tape.clone();
        bad_version.version = REPLAY_VERSION + 1;
        assert_eq!(bad_version.check(), Err(ReplayError::BadTape));
        assert!(Theater::new(bad_version).is_err());
        let mut short = tape.clone();
        short.checkpoints.pop();
        assert_eq!(short.check(), Err(ReplayError::BadTape));
        let mut forged = tape.clone();
        forged.keyframes[0].hash ^= 1;
        assert_eq!(forged.check(), Err(ReplayError::BadTape));
        let mut off_grid = tape.clone();
        off_grid.keyframes[0].tick = 1;
        assert_eq!(off_grid.check(), Err(ReplayError::BadTape));
        // Error strings name the fault.
        assert_eq!(ReplayError::BadConfig.to_string(), "invalid arena config");
        assert_eq!(ReplayError::BadInput.to_string(), "invalid replay input");
        assert_eq!(
            ReplayError::NoFailure.to_string(),
            "probe matches the recording"
        );
        assert_eq!(TICK_HZ, 60);
    }

    /// Per-tape theater proof: frame-step every tick both ways, every
    /// keyframe seeks and resumes through the arena snapshot codec, and the
    /// divergence report names the causal tick (the first differing checkpoint,
    /// restated here so the net owner stays dependency-free).
    fn check_mp7_tape(tape: &ReplayTape, probe: &[FighterInput], cause: (u32, u8)) {
        let mut theater = Theater::new(tape.clone()).expect("open");
        for want in 1..tape.config.ticks {
            assert_eq!(
                theater.step_forward().map(|point| point.tick),
                Some(want),
                "frame-step forward lands on tick {want}"
            );
        }
        assert_eq!(theater.step_forward(), None);
        for want in (0..tape.config.ticks - 1).rev() {
            assert_eq!(
                theater.step_back().map(|point| point.tick),
                Some(want),
                "frame-step back lands on tick {want}"
            );
        }
        assert_eq!(theater.cursor(), 0);
        for index in 0..tape.keyframes.len() {
            let tick = tape.keyframes[index].tick;
            let point = theater.seek_keyframe(index).expect("seek");
            assert_eq!(
                (theater.cursor(), point.tick, point.hash),
                (tick, tick, tape.keyframes[index].hash)
            );
            let suffix = tape.suffix_from_keyframe(index).expect("resume");
            assert_eq!(suffix, tape.checkpoints[tick as usize..]);
        }
        theater.verify().expect("recording replays exactly");
        let report = tape.attribute_probe(probe).expect("attributes");
        let baseline_run = engine_arena::run(&tape.config, &FighterTape(tape.inputs.clone()));
        let probe_run = engine_arena::run(&tape.config, &FighterTape(probe.to_vec()));
        let mut causal_tick = None;
        for (index, (left, right)) in baseline_run.iter().zip(probe_run.iter()).enumerate() {
            if left != right {
                causal_tick = Some(index as u32);
                break;
            }
        }
        assert_eq!(
            Some(report.divergent_tick),
            causal_tick,
            "matches the causal tick"
        );
        assert_eq!(report.causes.len(), 1, "one cause: {report}");
        assert_eq!(
            (report.causes[0].tick, report.causes[0].fighter),
            cause,
            "cause is the perturbed input: {report}"
        );
        assert_ne!(report.expected, report.observed);
    }

    #[test]
    fn mp7_three_tapes_frame_step_keyframe_resume_cr1_parity_and_seeded_corruption() {
        // Three tapes: short fight, long soak, tuned fight (the seeded
        // divert at tick 50 for fighter 1).
        let short_cfg = config(64);
        let short_inputs: Vec<FighterInput> = (0..64)
            .map(|tick| wish(tick, 0, if tick % 20 < 10 { 100 } else { -100 }))
            .collect();
        let mut short_probe = short_inputs.clone();
        short_probe[7] = wish(7, 0, -100);
        let soak_cfg = config(1250);
        let soak_tick = engine_core::Rng::new(2040).fork(7).next_range(0, 1250) as u32;
        let soak_probe = vec![wish(soak_tick, 1, 100)];
        let tuned_cfg = config(400);
        let tuned_probe = vec![FighterInput {
            tick: 50,
            fighter: FighterId::new(1).expect("slot 1"),
            mx: 100,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        }];

        let short = record_tape(short_cfg, &short_inputs).expect("record short");
        let soak = record_tape(soak_cfg, &[]).expect("record soak");
        let tuned = record_tape(tuned_cfg, &[]).expect("record tuned");

        check_mp7_tape(&short, &short_probe, (7, 0));
        check_mp7_tape(&soak, &soak_probe, (soak_tick, 1));
        check_mp7_tape(&tuned, &tuned_probe, (50, 1));
        println!(
            "THEATER: short={} soak={} tuned={} ticks; keyframes {}/{}/{}",
            short.config.ticks,
            soak.config.ticks,
            tuned.config.ticks,
            short.keyframes.len(),
            soak.keyframes.len(),
            tuned.keyframes.len()
        );

        // Seeded corruption refuses loudly on every tape: forged checkpoint
        // bits diverge at the seeded tick, a forged keyframe hash and a wrong
        // envelope version fail the cheap check.
        let mut forged_short = short.clone();
        // Short keyframes sit at ticks 0 and 63, so the seeded forgery stays
        // strictly inside to reach the resimulating gate, not the grid check.
        let bad_tick = engine_core::Rng::new(2040).fork(71).next_range(1, 63) as usize;
        forged_short.checkpoints[bad_tick].hash ^= 1;
        assert_eq!(
            play_tape(&forged_short, short.config.ticks - 1),
            Err(ReplayError::Diverged {
                tick: bad_tick as u32,
                expected: forged_short.checkpoints[bad_tick].hash,
                observed: forged_short.checkpoints[bad_tick].hash ^ 1,
            })
        );
        assert!(Theater::new(forged_short).expect("open").verify().is_err());
        let mut forged_soak = soak.clone();
        let bad_key = engine_core::Rng::new(2040)
            .fork(72)
            .next_range(0, soak.keyframes.len() as u64) as usize;
        forged_soak.keyframes[bad_key].hash ^= 1;
        assert_eq!(forged_soak.check(), Err(ReplayError::BadTape));
        assert!(Theater::new(forged_soak).is_err());
        let mut forged_tuned = tuned.clone();
        let bump = 1 + engine_core::Rng::new(2040).fork(73).next_range(0, 5) as u8;
        forged_tuned.version = REPLAY_VERSION.wrapping_add(bump);
        assert_eq!(forged_tuned.check(), Err(ReplayError::BadTape));
        assert!(Theater::new(forged_tuned).is_err());
        assert_eq!(TICK_HZ, 60);
    }

    #[test]
    fn ten_seeded_bugs_attribute_to_exact_tick_and_input() {
        let cfg = config(200);
        let baseline: Vec<FighterInput> = Vec::new();
        let recording = record_tape(cfg, &baseline).expect("record baseline");
        let seeds = [
            FighterInput {
                tick: 5,
                fighter: FighterId::new(0).expect("slot fits"),
                mx: 100,
                mz: 0,
                facing: 0,
                buttons: Buttons::NONE,
            },
            FighterInput {
                tick: 17,
                fighter: FighterId::new(1).expect("slot fits"),
                mx: -100,
                mz: 0,
                facing: 0,
                buttons: Buttons::NONE,
            },
            FighterInput {
                tick: 29,
                fighter: FighterId::new(0).expect("slot fits"),
                mx: 0,
                mz: 100,
                facing: 0,
                buttons: Buttons::NONE,
            },
            FighterInput {
                tick: 41,
                fighter: FighterId::new(1).expect("slot fits"),
                mx: 0,
                mz: -100,
                facing: 0,
                buttons: Buttons::NONE,
            },
            FighterInput {
                tick: 53,
                fighter: FighterId::new(0).expect("slot fits"),
                mx: 0,
                mz: 0,
                facing: 9000,
                buttons: Buttons::NONE,
            },
            FighterInput {
                tick: 67,
                fighter: FighterId::new(1).expect("slot fits"),
                mx: 0,
                mz: 0,
                facing: -9000,
                buttons: Buttons::NONE,
            },
            FighterInput {
                tick: 79,
                fighter: FighterId::new(0).expect("slot fits"),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Buttons(1),
            },
            FighterInput {
                tick: 93,
                fighter: FighterId::new(1).expect("slot fits"),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Buttons(2),
            },
            FighterInput {
                tick: 107,
                fighter: FighterId::new(0).expect("slot fits"),
                mx: 50,
                mz: 50,
                facing: 4500,
                buttons: Buttons(4),
            },
            FighterInput {
                tick: 129,
                fighter: FighterId::new(1).expect("slot fits"),
                mx: -50,
                mz: 50,
                facing: -4500,
                buttons: Buttons(3),
            },
        ];
        let mut first = Vec::new();
        for seed in &seeds {
            let probe = vec![*seed];
            let report = recording.attribute_probe(&probe).expect("probe diverges");
            assert_eq!(report.divergent_tick, seed.tick, "tick: {report}");
            assert_eq!(report.causes.len(), 1, "one cause: {report}");
            assert_eq!(
                (report.causes[0].tick, report.causes[0].fighter),
                (seed.tick, seed.fighter.0),
                "cause: {report}"
            );
            assert_eq!(report.causes[0].input, *seed, "input: {report}");
            assert_ne!(report.expected, report.observed, "digests: {report}");
            first.push(report);
        }
        let mut second = Vec::new();
        for seed in &seeds {
            let probe = vec![*seed];
            let report = recording.attribute_probe(&probe).expect("probe diverges");
            second.push(report);
        }
        assert_eq!(first, second, "sweep repeats byte for byte");
    }

    #[test]
    fn capture_fits_64_bytes_per_checkpoint() {
        let per_tick = std::mem::size_of::<Checkpoint>();
        assert!(per_tick <= 64, "checkpoint {per_tick} bytes over 64 cap");
        let per_keyframe = std::mem::size_of::<Keyframe>();
        assert!(
            per_keyframe <= 64,
            "keyframe {per_keyframe} bytes over 64 cap"
        );
        let tape = record_tape(config(64), &[]).expect("record");
        assert_eq!(tape.checkpoints.len(), 64);
        let stored = std::mem::size_of_val(&tape.checkpoints[0]);
        assert!(stored <= 64, "stored checkpoint {stored} bytes over 64 cap");
    }
}
