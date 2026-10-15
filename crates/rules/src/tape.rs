//! The rules tape: inputs, the script hash(es) the match ran under, and checkpoints.
//!
//! The header carries the ABI id and, for every script installed during the match, the tick
//! and the hash of its text. Replay looks each hash up in a `ScriptLibrary`, so an old tape
//! keeps replaying with its old script after the live script has moved on. If the ABI id no
//! longer matches the engine (limits or host API changed), replay refuses with a typed error
//! instead of drifting.
//!
//! File layout, little-endian: magic `E2RL`, version u16, abi u64, seed u64, fighters u8,
//! ticks u32, checkpoint_every u32, script count u16, per script (tick u32, hash u64), input
//! count u32, per input 10 bytes, checkpoint count u32, per checkpoint u64.

use std::collections::BTreeMap;

use crate::error::RulesError;
use crate::host::{BTN_ATTACK, BTN_BLOCK, BTN_DODGE};
use crate::limits::{Limits, MAX_FIGHTERS};
use crate::rules::{rules_abi, Rules};
use crate::script::script_hash;
use crate::sim::{scripted_inputs, ScriptInput, Sim};

/// File magic.
pub const TAPE_MAGIC: &[u8; 4] = b"E2RL";
/// Tape format version.
pub const TAPE_VERSION: u16 = 1;
/// Longest match a tape may hold (ticks).
pub const MAX_TAPE_TICKS: u32 = 200_000;
/// Most inputs a tape may hold.
pub const MAX_TAPE_INPUTS: usize = 2_000_000;
/// Most script installs a tape may name.
pub const MAX_TAPE_SCRIPTS: usize = 256;

/// A script installed at the start of `tick`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptSwap {
    /// Tick the script is installed before.
    pub tick: u32,
    /// Hash of the script text.
    pub hash: u64,
}

/// Why a tape file was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TapeError {
    /// Wrong magic bytes.
    BadMagic,
    /// Unknown format version.
    Version(u16),
    /// Ended early.
    Truncated,
    /// Bytes after the last field.
    Trailing,
    /// A count above its cap, or an out-of-range field.
    Invalid(&'static str),
}

/// Why a replay failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// Tape fields inconsistent.
    Tape(TapeError),
    /// The tape was written under another ABI (limits or host API changed).
    AbiMismatch {
        /// ABI id in the tape.
        tape: u64,
        /// ABI id of this engine.
        engine: u64,
    },
    /// The tape names a script the library does not hold.
    MissingScript(u64),
    /// The library text does not hash to the name it was filed under.
    LibraryCorrupt(u64),
    /// A script in the tape was refused by this engine.
    Script(RulesError),
    /// The tick or the world failed.
    Tick(RulesError),
    /// A checkpoint differed.
    Divergence {
        /// Zero-based tick of the checkpoint.
        tick: u32,
        /// Digest in the tape.
        expected: u64,
        /// Digest replayed.
        actual: u64,
    },
}

/// Script texts by hash: the museum a tape's header points into.
#[derive(Clone, Debug, Default)]
pub struct ScriptLibrary {
    by_hash: BTreeMap<u64, String>,
}

impl ScriptLibrary {
    /// An empty library.
    pub fn new() -> Self {
        Self::default()
    }

    /// File a script under its own hash and return the hash.
    pub fn add(&mut self, source: &str) -> u64 {
        let hash = script_hash(source);
        self.by_hash.insert(hash, source.to_string());
        hash
    }

    /// Text filed under `hash`.
    pub fn get(&self, hash: u64) -> Option<&str> {
        self.by_hash.get(&hash).map(String::as_str)
    }

    /// Scripts held.
    pub fn len(&self) -> usize {
        self.by_hash.len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.by_hash.is_empty()
    }
}

/// A recorded rules match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RulesTape {
    /// ABI id the match ran under.
    pub abi: u64,
    /// Match seed.
    pub seed: u64,
    /// Actors in the match.
    pub fighters: u8,
    /// Ticks run.
    pub ticks: u32,
    /// A checkpoint is taken after every `checkpoint_every` ticks and after the last.
    pub checkpoint_every: u32,
    /// Scripts installed, by tick (strictly increasing). Empty means rules were off.
    pub scripts: Vec<ScriptSwap>,
    /// Inputs sorted by `(tick, fighter)`.
    pub inputs: Vec<ScriptInput>,
    /// Combined sim and rules-memory digests at each checkpoint.
    pub checkpoints: Vec<u64>,
}

/// Checkpoints a tape of `ticks` with `every` must hold.
pub fn checkpoint_count(ticks: u32, every: u32) -> usize {
    if every == 0 {
        return 0;
    }
    let full = usize::try_from(ticks / every).unwrap_or(usize::MAX);
    full + usize::from(!ticks.is_multiple_of(every))
}

impl RulesTape {
    /// The header bytes: everything except inputs and checkpoints. What the game's own tape
    /// header embeds (`header_digest`) to bind a match to its rules.
    pub fn header_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(32 + self.scripts.len() * 12);
        bytes.extend_from_slice(TAPE_MAGIC);
        bytes.extend_from_slice(&TAPE_VERSION.to_le_bytes());
        bytes.extend_from_slice(&self.abi.to_le_bytes());
        bytes.extend_from_slice(&self.seed.to_le_bytes());
        bytes.push(self.fighters);
        bytes.extend_from_slice(&self.ticks.to_le_bytes());
        bytes.extend_from_slice(&self.checkpoint_every.to_le_bytes());
        bytes.extend_from_slice(
            &u16::try_from(self.scripts.len())
                .unwrap_or(u16::MAX)
                .to_le_bytes(),
        );
        for swap in &self.scripts {
            bytes.extend_from_slice(&swap.tick.to_le_bytes());
            bytes.extend_from_slice(&swap.hash.to_le_bytes());
        }
        bytes
    }

    /// FNV-1a 64 of `header_bytes`.
    pub fn header_digest(&self) -> u64 {
        engine_core::hash_bytes(&self.header_bytes())
    }

    /// The whole tape as bytes.
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = self.header_bytes();
        bytes.extend_from_slice(
            &u32::try_from(self.inputs.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        for input in &self.inputs {
            input.append_bytes(&mut bytes);
        }
        bytes.extend_from_slice(
            &u32::try_from(self.checkpoints.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        for checkpoint in &self.checkpoints {
            bytes.extend_from_slice(&checkpoint.to_le_bytes());
        }
        bytes
    }

    /// Decode and validate a tape without trusting any size, count or trailing data.
    pub fn decode(bytes: &[u8]) -> Result<Self, TapeError> {
        let mut r = Reader { bytes, at: 0 };
        if r.take(4)? != TAPE_MAGIC {
            return Err(TapeError::BadMagic);
        }
        let version = r.u16()?;
        if version != TAPE_VERSION {
            return Err(TapeError::Version(version));
        }
        let abi = r.u64()?;
        let seed = r.u64()?;
        let fighters = r.u8()?;
        let ticks = r.u32()?;
        let checkpoint_every = r.u32()?;
        let script_count = usize::from(r.u16()?);
        if usize::from(fighters) == 0 || usize::from(fighters) > MAX_FIGHTERS {
            return Err(TapeError::Invalid("fighter count"));
        }
        if ticks == 0 || ticks > MAX_TAPE_TICKS || checkpoint_every == 0 {
            return Err(TapeError::Invalid("ticks or checkpoint cadence"));
        }
        if script_count > MAX_TAPE_SCRIPTS {
            return Err(TapeError::Invalid("too many scripts"));
        }
        let mut scripts = Vec::with_capacity(script_count);
        for _ in 0..script_count {
            scripts.push(ScriptSwap {
                tick: r.u32()?,
                hash: r.u64()?,
            });
        }
        if scripts.windows(2).any(|pair| pair[0].tick >= pair[1].tick)
            || scripts.iter().any(|s| s.tick >= ticks)
        {
            return Err(TapeError::Invalid(
                "script ticks must be increasing and inside the match",
            ));
        }
        let input_count = usize::try_from(r.u32()?).unwrap_or(usize::MAX);
        if input_count > MAX_TAPE_INPUTS {
            return Err(TapeError::Invalid("too many inputs"));
        }
        let mut inputs = Vec::with_capacity(input_count.min(r.remaining() / ScriptInput::BYTES));
        for _ in 0..input_count {
            let input = ScriptInput {
                tick: r.u32()?,
                fighter: r.u8()?,
                mx: i8::from_le_bytes([r.u8()?]),
                mz: i8::from_le_bytes([r.u8()?]),
                facing: i16::from_le_bytes([r.u8()?, r.u8()?]),
                buttons: r.u8()?,
            };
            let legal = input.tick < ticks
                && input.fighter < fighters
                && (-100..=100).contains(&input.mx)
                && (-100..=100).contains(&input.mz)
                && (-18_000..=18_000).contains(&input.facing)
                && input.buttons & !(BTN_ATTACK | BTN_BLOCK | BTN_DODGE) == 0;
            if !legal {
                return Err(TapeError::Invalid("input out of range"));
            }
            inputs.push(input);
        }
        if inputs
            .windows(2)
            .any(|p| (p[0].tick, p[0].fighter) >= (p[1].tick, p[1].fighter))
        {
            return Err(TapeError::Invalid(
                "inputs must be unique and sorted by (tick, fighter)",
            ));
        }
        let checkpoint_total = usize::try_from(r.u32()?).unwrap_or(usize::MAX);
        if checkpoint_total != checkpoint_count(ticks, checkpoint_every) {
            return Err(TapeError::Invalid("checkpoint count"));
        }
        let mut checkpoints = Vec::with_capacity(checkpoint_total);
        for _ in 0..checkpoint_total {
            checkpoints.push(r.u64()?);
        }
        if r.remaining() != 0 {
            return Err(TapeError::Trailing);
        }
        Ok(Self {
            abi,
            seed,
            fighters,
            ticks,
            checkpoint_every,
            scripts,
            inputs,
            checkpoints,
        })
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len() - self.at
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], TapeError> {
        let end = self
            .at
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(TapeError::Truncated)?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, TapeError> {
        Ok(self.take(1)?[0])
    }

    fn le<const N: usize>(&mut self) -> Result<[u8; N], TapeError> {
        self.take(N)?.try_into().map_err(|_| TapeError::Truncated)
    }

    fn u16(&mut self) -> Result<u16, TapeError> {
        self.le().map(u16::from_le_bytes)
    }

    fn u32(&mut self) -> Result<u32, TapeError> {
        self.le().map(u32::from_le_bytes)
    }

    fn u64(&mut self) -> Result<u64, TapeError> {
        self.le().map(u64::from_le_bytes)
    }
}
/// Combined digest of the sim and the rules memory: the checkpoint value.
fn checkpoint(sim: &Sim, rules: &Rules) -> u64 {
    engine_core::hash_u64s(&[sim.digest(), rules.state_digest()])
}

/// What `record` runs.
#[derive(Clone, Copy, Debug)]
pub struct RecordSpec<'a> {
    /// Actors, 1..=64.
    pub fighters: usize,
    /// Match seed.
    pub seed: u64,
    /// Ticks to run.
    pub ticks: u32,
    /// Checkpoint cadence in ticks (non-zero).
    pub checkpoint_every: u32,
    /// Scripts to install, as `(tick, source)`.
    pub installs: &'a [(u32, &'a str)],
    /// Sandbox limits.
    pub limits: Limits,
}

/// Run the reference arena over `inputs` with scripts installed at the given ticks, and write
/// down the tape. Every script goes through `Rules::set` (smoke run included) and into
/// `library`.
pub fn record(
    spec: &RecordSpec<'_>,
    inputs: Vec<ScriptInput>,
    library: &mut ScriptLibrary,
) -> Result<RulesTape, RulesError> {
    let RecordSpec {
        fighters,
        seed,
        ticks,
        checkpoint_every,
        installs,
        limits,
    } = *spec;
    let mut rules = Rules::new(limits);
    let mut sim = Sim::new(fighters, seed);
    let mut scripts = Vec::new();
    let mut checkpoints = Vec::new();
    let mut cursor = 0;
    for tick in 0..ticks {
        for (at, source) in installs.iter().filter(|(at, _)| *at == tick) {
            let receipt = rules.set(source, *at)?;
            library.add(source);
            scripts.push(ScriptSwap {
                tick: *at,
                hash: receipt.hash,
            });
        }
        let end = cursor
            + inputs[cursor..]
                .iter()
                .take_while(|input| input.tick == tick)
                .count();
        sim.step(&inputs[cursor..end], &mut rules)?;
        cursor = end;
        if (tick + 1).is_multiple_of(checkpoint_every) || tick + 1 == ticks {
            checkpoints.push(checkpoint(&sim, &rules));
        }
    }
    Ok(RulesTape {
        abi: rules.abi(),
        seed,
        fighters: u8::try_from(fighters).unwrap_or(u8::MAX),
        ticks,
        checkpoint_every,
        scripts,
        inputs,
        checkpoints,
    })
}

/// Replay `tape` from its start, installing each named script from `library`, and compare every
/// checkpoint. Returns the final checkpoint.
pub fn replay(
    tape: &RulesTape,
    library: &ScriptLibrary,
    limits: Limits,
) -> Result<u64, ReplayError> {
    let engine_abi = rules_abi(&limits);
    if tape.abi != engine_abi {
        return Err(ReplayError::AbiMismatch {
            tape: tape.abi,
            engine: engine_abi,
        });
    }
    let mut rules = Rules::new(limits);
    let mut sim = Sim::new(usize::from(tape.fighters), tape.seed);
    let (mut cursor, mut next_check) = (0, 0);
    let mut last = 0;
    for tick in 0..tape.ticks {
        for swap in tape.scripts.iter().filter(|swap| swap.tick == tick) {
            let source = library
                .get(swap.hash)
                .ok_or(ReplayError::MissingScript(swap.hash))?;
            let receipt = rules
                .load_exact(source, tick)
                .map_err(ReplayError::Script)?;
            if receipt.hash != swap.hash {
                return Err(ReplayError::LibraryCorrupt(swap.hash));
            }
        }
        let end = cursor
            + tape.inputs[cursor..]
                .iter()
                .take_while(|input| input.tick == tick)
                .count();
        sim.step(&tape.inputs[cursor..end], &mut rules)
            .map_err(ReplayError::Tick)?;
        cursor = end;
        if (tick + 1).is_multiple_of(tape.checkpoint_every) || tick + 1 == tape.ticks {
            last = checkpoint(&sim, &rules);
            let expected = tape.checkpoints[next_check];
            if expected != last {
                return Err(ReplayError::Divergence {
                    tick,
                    expected,
                    actual: last,
                });
            }
            next_check += 1;
        }
    }
    Ok(last)
}

/// A ready-made tape for demos and tests: scripted inputs over `fighters`, one script from tick 0.
pub fn record_scripted(
    fighters: usize,
    seed: u64,
    ticks: u32,
    source: &str,
    library: &mut ScriptLibrary,
) -> Result<RulesTape, RulesError> {
    let spec = RecordSpec {
        fighters,
        seed,
        ticks,
        checkpoint_every: 100,
        installs: &[(0, source)],
        limits: Limits::DEFAULT,
    };
    record(&spec, scripted_inputs(fighters, ticks, seed), library)
}
