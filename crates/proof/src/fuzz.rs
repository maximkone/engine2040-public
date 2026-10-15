//! Deterministic game fuzzing over the arena match spine.
//!
//! Consumer: the FZ1 gate and future playtest agents. Data owner: this module
//! owns fuzz corpora and repro bytes; `engine-arena` owns match state, and the
//! fighter tape types come from `engine-input` through the arena re-export.
//! Units: meters, radians, fixed ticks at the arena tick rate. Determinism
//! scope: the same `(seed, ticks, fighters)` always builds the same tape on
//! every machine, and every replay goes through [`engine_arena::run`].
//! Error type: [`FuzzError`] for repro IO and decoding; invariant failures are
//! values ([`InvariantFail`]), never errors.
//!
//! The position oracle below mirrors the arena's documented kinematics (spawn
//! mapping, wish steps, wall clamp) so the in-bounds and NaN rules can observe
//! positions that checkpoints alone do not carry. It is a same-machine test
//! oracle only; authoritative state stays in `engine-arena`.

use engine_arena::seed::expand as expand_seed;
use engine_arena::{
    ArenaConfig, Checkpoint as ArenaCheckpoint, FighterId, FighterInput, FighterTape,
    FIGHTER_RADIUS_M, MAX_PARTICIPANTS, TICK_HZ, TOP_SPEED_MPS,
};
use engine_core::hash_bytes;
use std::fmt;
use std::path::{Path, PathBuf};

/// Upper bound on ticks per [`gen_tape`] call, keeping corpora small and fast.
pub const MAX_FUZZ_TICKS: u32 = 512;
/// Most random inputs one [`gen_tape`] call emits; the stream budget follows.
const MAX_GEN_INPUTS: usize = 16;
/// Stream words per [`gen_tape`] call: one count word plus six per input.
const GEN_WORDS: usize = 1 + 6 * MAX_GEN_INPUTS;
/// Fixed tick count of every [`adversarial`] script.
pub const ADVERSARIAL_TICKS: u32 = 64;
/// Fixed fighter count of every [`adversarial`] script.
pub const ADVERSARIAL_FIGHTERS: u8 = 2;
/// Script names in [`adversarial`] order: wall-rush, spin, mash, max-wish.
pub const ADVERSARIAL_NAMES: [&str; 4] = ["wall-rush", "spin-in-place", "button-mash", "max-wish"];
/// Wish axis range, mirroring the `engine-input` fighter contract.
const WISH_RANGE: i8 = 100;
/// Facing range in centidegrees, mirroring the fighter contract.
const FACING_RANGE: i16 = 18_000;
/// Known button bits (`ATTACK | BLOCK | DODGE`); anything else is rejected.
const KNOWN_BUTTON_BITS: u8 = 0x07;

/// Repro envelope version, independent of arena match rules.
pub const REPRO_FORMAT: u16 = 1;
/// Fixed header bytes: magic, format, seed, ticks, fighters, extent, count.
const REPRO_HEADER: usize = 32;
/// Most inputs one repro file holds; checked before allocation.
pub const MAX_REPRO_INPUTS: usize = 100_000;
/// Largest accepted repro file; checked before allocation.
pub const MAX_REPRO_BYTES: usize = REPRO_HEADER + MAX_REPRO_INPUTS * 10;

/// Set the button bitset on one fighter input without naming its type.
///
/// `Buttons` is not re-exported through `engine-arena`, and this crate takes
/// no direct `engine-input` edge under the FZ1 packet, so the type is never
/// named here. Construction starts from the default (empty) bitset, whose
/// type unifies with the `buttons` field, and this helper then writes the
/// public raw-bits field through the concrete input type. Every `u8` pattern
/// is a valid bitset; the mask keeps only the contract's known button bits
/// (`ATTACK | BLOCK | DODGE = 0x07`), so tape validation always passes.
fn set_buttons(mut input: FighterInput, bits: u8) -> FighterInput {
    input.buttons.0 = bits & KNOWN_BUTTON_BITS;
    input
}

/// Invariant rule violated by one [`InvariantFail`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Rule {
    /// Checkpoint ticks ran backwards.
    TicksOrdered,
    /// A tape with effectful inputs hashed exactly like the no-input run.
    HashResponds,
    /// A fighter center left the wall-clamped arena.
    InBounds,
    /// A fighter position held a NaN, detected by bit pattern.
    NoNan,
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::TicksOrdered => "ticks-ordered",
            Self::HashResponds => "hash-responds",
            Self::InBounds => "in-bounds",
            Self::NoNan => "no-nan",
        };
        write!(f, "{name}")
    }
}

/// One invariant violation at one zero-based tick.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct InvariantFail {
    /// Zero-based tick where the rule broke.
    pub tick: u32,
    /// Which rule broke.
    pub rule: Rule,
}

/// Repro save/load failure: invalid bytes or an IO error.
#[derive(Debug)]
pub enum FuzzError {
    /// Invalid data, with a bounded diagnostic.
    Invalid(&'static str),
    /// Filesystem failure while saving or loading a repro.
    Io(std::io::Error),
}

impl fmt::Display for FuzzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(f, "invalid fuzz repro: {reason}"),
            Self::Io(error) => write!(f, "fuzz repro io: {error}"),
        }
    }
}

impl std::error::Error for FuzzError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Invalid(_) => None,
            Self::Io(error) => Some(error),
        }
    }
}

impl From<std::io::Error> for FuzzError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Build one validated fighter input; `None` only for an out-of-range slot.
fn make_input(
    tick: u32,
    fighter: u8,
    mx: i8,
    mz: i8,
    facing: i16,
    buttons: u8,
) -> Option<FighterInput> {
    let slot = FighterId::new(fighter)?;
    let base = FighterInput {
        tick,
        fighter: slot,
        mx,
        mz,
        facing,
        buttons: Default::default(),
    };
    Some(set_buttons(base, buttons))
}

/// True when an input can move the match hash at all.
fn is_effectful(input: &FighterInput) -> bool {
    input.mx != 0 || input.mz != 0 || input.facing != 0 || input.buttons.0 != 0
}

/// Seeded random valid inputs over the arena, via [`engine_arena::seed::expand`].
///
/// The tape is `(tick, fighter)`-sorted, duplicate-free, holds no no-op
/// inputs (so the hash-responds rule stays sound), and always passes tape
/// validation. Out-of-range `ticks`/`fighters` clamp into the valid arena
/// ranges. The same `(seed, ticks, fighters)` always builds the same tape.
pub fn gen_tape(seed: u64, ticks: u32, fighters: u8) -> FighterTape {
    let ticks = ticks.clamp(1, MAX_FUZZ_TICKS);
    let fighters = fighters.clamp(2, MAX_PARTICIPANTS);
    let words = expand_seed(seed, GEN_WORDS);
    let count = (words[0] % (MAX_GEN_INPUTS as u64 + 1)) as usize;
    let mut inputs = Vec::with_capacity(count);
    for chunk in words[1..].chunks_exact(6).take(count) {
        let tick = (chunk[0] % u64::from(ticks)) as u32;
        let fighter = (chunk[1] % u64::from(fighters)) as u8;
        let mx = ((chunk[2] % u64::from(WISH_RANGE as u8 * 2 + 1)) as i16 - 100) as i8;
        let mz = ((chunk[3] % u64::from(WISH_RANGE as u8 * 2 + 1)) as i16 - 100) as i8;
        let facing = ((chunk[4] % u64::from(FACING_RANGE as u16 * 2 + 1)) as i32 - 18_000) as i16;
        let buttons = (chunk[5] % u64::from(KNOWN_BUTTON_BITS + 1)) as u8;
        if let Some(input) = make_input(tick, fighter, mx, mz, facing, buttons) {
            if is_effectful(&input) {
                inputs.push(input);
            }
        }
    }
    inputs.sort_by_key(|input| (input.tick, input.fighter.0));
    inputs.dedup_by_key(|input| (input.tick, input.fighter.0));
    let tape = FighterTape(inputs);
    debug_assert!(tape.validate().is_ok());
    tape
}

/// Four adversarial scripts in [`ADVERSARIAL_NAMES`] order, all over two
/// fighters and [`ADVERSARIAL_TICKS`] ticks: wall-rush (max wishes into
/// opposite walls), spin-in-place (sweeping facing, no translation),
/// button-mash (cycling button bits, no translation) and max-wish (extreme
/// wishes alternating per tick). The seed picks wall sides, sweep phase, mash
/// phase and wish signs. Every script validates and replays without panic.
pub fn adversarial(seed: u64) -> Vec<FighterTape> {
    const TICKS: u32 = ADVERSARIAL_TICKS;
    const FIGHTERS: u8 = ADVERSARIAL_FIGHTERS;
    let words = expand_seed(seed, 8);
    let sign = |word: u64| {
        if word & 1 == 0 {
            1
        } else {
            -1
        }
    };
    let (sweep, mash) = (words[2], words[3]);
    let (rush_x, rush_z) = (sign(words[0]), sign(words[1]));
    let mut wall = Vec::new();
    let mut spin = Vec::new();
    let mut buttons = Vec::new();
    let mut wishes = Vec::new();
    for tick in 0..TICKS {
        for fighter in 0..FIGHTERS {
            let side: i8 = if fighter == 0 { 1 } else { -1 };
            if let Some(input) = make_input(
                tick,
                fighter,
                side * rush_x * WISH_RANGE,
                side * rush_z * WISH_RANGE,
                0,
                0,
            ) {
                wall.push(input);
            }
            let facing = (((u64::from(tick) * 937 + sweep) % u64::from(FACING_RANGE as u16 * 2 + 1))
                as i32
                - 18_000) as i16;
            if let Some(input) = make_input(tick, fighter, 0, 0, facing, 0) {
                spin.push(input);
            }
            let chord = ((u64::from(tick) + u64::from(fighter) * 3 + mash)
                % u64::from(KNOWN_BUTTON_BITS + 1)) as u8;
            if let Some(input) = make_input(tick, fighter, 0, 0, 0, chord) {
                buttons.push(input);
            }
            let bit = |word: u64| {
                if (word >> (tick % 64)) & 1 == 0 {
                    WISH_RANGE
                } else {
                    -WISH_RANGE
                }
            };
            let slot = usize::from(fighter);
            if let Some(input) = make_input(
                tick,
                fighter,
                bit(words[4 + slot % 2]),
                bit(words[6 + slot % 2]),
                0,
                0,
            ) {
                wishes.push(input);
            }
        }
    }
    [wall, spin, buttons, wishes]
        .into_iter()
        .map(FighterTape)
        .collect()
}

/// Normalize signed zero so `0.0` and `-0.0` compare identically.
fn canon(value: f32) -> f32 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}

/// Map one stream word uniformly into `[0, 1]`; mirrors the arena spawn map.
fn unit_f32(word: u64) -> f32 {
    (word as f64 / u64::MAX as f64) as f32
}

/// True for NaN by bit pattern: all exponent bits set, mantissa nonzero.
/// Infinities have a zero mantissa and are not NaN.
fn is_nan_bits(value: f32) -> bool {
    let bits = value.to_bits();
    bits & 0x7F80_0000 == 0x7F80_0000 && bits & 0x007F_FFFF != 0
}

/// Per-tick fighter `[x, y, z]` frames mirroring the arena kinematics.
///
/// Returns no frames for invalid config or tape (validation owns those
/// inputs); otherwise one frame per tick in fighter-slot order. Same-machine
/// oracle for the in-bounds and NaN rules only.
fn oracle_positions(config: &ArenaConfig, tape: &FighterTape) -> Vec<Vec<[f32; 3]>> {
    if config.validate().is_err() || tape.validate().is_err() {
        return Vec::new();
    }
    let fighters = usize::from(config.fighters);
    if tape
        .0
        .iter()
        .any(|input| input.tick >= config.ticks || usize::from(input.fighter.0) >= fighters)
    {
        return Vec::new();
    }
    let limit = (config.half_extent_m - FIGHTER_RADIUS_M).max(0.0);
    let words = expand_seed(config.seed, fighters * 2);
    let mut pos: Vec<[f32; 3]> = (0..fighters)
        .map(|slot| {
            let x = unit_f32(words[slot * 2]) * 2.0 - 1.0;
            let z = unit_f32(words[slot * 2 + 1]) * 2.0 - 1.0;
            [canon(x * limit), 0.0, canon(z * limit)]
        })
        .collect();
    let dt = 1.0_f32 / TICK_HZ as f32;
    let mut wishes = [[0i8; 2]; MAX_PARTICIPANTS as usize];
    let mut cursor = 0_usize;
    let mut frames = Vec::with_capacity(config.ticks as usize);
    for tick in 0..config.ticks {
        while cursor < tape.0.len() && tape.0[cursor].tick == tick {
            let slot = usize::from(tape.0[cursor].fighter.0);
            if slot < fighters {
                wishes[slot] = [tape.0[cursor].mx, tape.0[cursor].mz];
            }
            cursor += 1;
        }
        for (index, feet) in pos.iter_mut().enumerate().take(fighters) {
            let step_x = wishes[index][0] as f32 / 100.0 * TOP_SPEED_MPS * dt;
            let step_z = wishes[index][1] as f32 / 100.0 * TOP_SPEED_MPS * dt;
            feet[0] = canon(feet[0] + step_x).clamp(-limit, limit);
            feet[2] = canon(feet[2] + step_z).clamp(-limit, limit);
            feet[0] = canon(feet[0]);
            feet[2] = canon(feet[2]);
        }
        frames.push(pos.clone());
    }
    frames
}

/// Check every fuzz invariant over one replay.
///
/// Rules: checkpoint ticks never decrease; a tape with effectful inputs never
/// hashes exactly like the no-input run; every fighter center stays inside
/// the wall-clamped arena; no position is NaN by bit pattern. Needs the
/// config and tape because checkpoints alone carry no positions. Invalid
/// config or tape yields no findings; validation owns those inputs.
pub fn check_invariants(
    config: &ArenaConfig,
    tape: &FighterTape,
    checkpoints: &[ArenaCheckpoint],
) -> Vec<InvariantFail> {
    let mut fails = Vec::new();
    for pair in checkpoints.windows(2) {
        if pair[1].tick < pair[0].tick {
            fails.push(InvariantFail {
                tick: pair[1].tick,
                rule: Rule::TicksOrdered,
            });
        }
    }
    if config.validate().is_err()
        || tape.validate().is_err()
        || checkpoints.is_empty()
        || tape.0.iter().any(|input| {
            input.tick >= config.ticks
                || usize::from(input.fighter.0) >= usize::from(config.fighters)
        })
    {
        return fails;
    }
    if let Some(first) = tape.0.iter().find(|input| is_effectful(input)) {
        let baseline = engine_arena::run(config, &FighterTape::default());
        if baseline.as_slice() == checkpoints {
            fails.push(InvariantFail {
                tick: first.tick,
                rule: Rule::HashResponds,
            });
        }
    }
    let limit = (config.half_extent_m - FIGHTER_RADIUS_M).max(0.0);
    for (tick, frame) in oracle_positions(config, tape).iter().enumerate() {
        let tick = tick as u32;
        let mut nan = false;
        let mut bounds = false;
        for feet in frame {
            for axis in [feet[0], feet[2]] {
                if is_nan_bits(axis) {
                    nan = true;
                }
                if !(-limit..=limit).contains(&axis) {
                    bounds = true;
                }
            }
        }
        if nan {
            fails.push(InvariantFail {
                tick,
                rule: Rule::NoNan,
            });
        }
        if bounds {
            fails.push(InvariantFail {
                tick,
                rule: Rule::InBounds,
            });
        }
    }
    fails
}

/// Encode one repro: versioned header plus config, then every input.
fn encode_repro(config: &ArenaConfig, tape: &FighterTape) -> Result<Vec<u8>, FuzzError> {
    if config.validate().is_err() {
        return Err(FuzzError::Invalid("invalid arena config"));
    }
    if tape.validate().is_err() {
        return Err(FuzzError::Invalid("invalid fighter tape"));
    }
    if tape.0.iter().any(|input| {
        input.tick >= config.ticks || usize::from(input.fighter.0) >= usize::from(config.fighters)
    }) {
        return Err(FuzzError::Invalid("tape escapes its arena config"));
    }
    if tape.0.len() > MAX_REPRO_INPUTS {
        return Err(FuzzError::Invalid("repro holds too many inputs"));
    }
    let mut bytes = Vec::with_capacity(REPRO_HEADER + tape.0.len() * 10);
    bytes.extend_from_slice(b"E2040FZ\0");
    bytes.extend_from_slice(&REPRO_FORMAT.to_le_bytes());
    bytes.extend_from_slice(&config.seed.to_le_bytes());
    bytes.extend_from_slice(&config.ticks.to_le_bytes());
    bytes.push(config.fighters);
    bytes.push(0);
    bytes.extend_from_slice(&config.half_extent_m.to_bits().to_le_bytes());
    bytes.extend_from_slice(&(tape.0.len() as u32).to_le_bytes());
    for input in &tape.0 {
        bytes.extend_from_slice(&input.tick.to_le_bytes());
        bytes.push(input.fighter.0);
        bytes.push(input.mx as u8);
        bytes.push(input.mz as u8);
        bytes.extend_from_slice(&input.facing.to_le_bytes());
        bytes.push(input.buttons.0);
    }
    Ok(bytes)
}

/// Take exactly `N` bytes or fail with a bounded diagnostic.
fn take<const N: usize>(bytes: &[u8], pos: &mut usize) -> Result<[u8; N], FuzzError> {
    let next = pos
        .checked_add(N)
        .ok_or(FuzzError::Invalid("repro offset overflows"))?;
    let slice = bytes
        .get(*pos..next)
        .ok_or(FuzzError::Invalid("truncated repro"))?;
    *pos = next;
    let mut value = [0; N];
    value.copy_from_slice(slice);
    Ok(value)
}

/// Decode one repro, validating versions, ranges, bounds and exact length.
fn decode_repro(bytes: &[u8]) -> Result<(ArenaConfig, FighterTape), FuzzError> {
    if bytes.len() < REPRO_HEADER || bytes.len() > MAX_REPRO_BYTES {
        return Err(FuzzError::Invalid("repro size is outside bounds"));
    }
    let mut pos = 0;
    if take::<8>(bytes, &mut pos)? != *b"E2040FZ\0" {
        return Err(FuzzError::Invalid("bad repro magic"));
    }
    if u16::from_le_bytes(take(bytes, &mut pos)?) != REPRO_FORMAT {
        return Err(FuzzError::Invalid("unsupported repro version"));
    }
    let seed = u64::from_le_bytes(take(bytes, &mut pos)?);
    let ticks = u32::from_le_bytes(take(bytes, &mut pos)?);
    let fighters = take::<1>(bytes, &mut pos)?[0];
    let _reserved: [u8; 1] = take(bytes, &mut pos)?;
    let half_extent_m = f32::from_bits(u32::from_le_bytes(take(bytes, &mut pos)?));
    let config = ArenaConfig {
        seed,
        ticks,
        fighters,
        half_extent_m,
    };
    if config.validate().is_err() {
        return Err(FuzzError::Invalid("invalid arena config"));
    }
    let count = u32::from_le_bytes(take(bytes, &mut pos)?) as usize;
    if count > MAX_REPRO_INPUTS {
        return Err(FuzzError::Invalid("repro holds too many inputs"));
    }
    let want = REPRO_HEADER
        .checked_add(
            count
                .checked_mul(10)
                .ok_or(FuzzError::Invalid("repro too large"))?,
        )
        .ok_or(FuzzError::Invalid("repro too large"))?;
    if bytes.len() != want {
        return Err(FuzzError::Invalid("repro counts or length mismatch"));
    }
    let mut inputs = Vec::with_capacity(count);
    for _ in 0..count {
        let tick = u32::from_le_bytes(take(bytes, &mut pos)?);
        let fighter = take::<1>(bytes, &mut pos)?[0];
        let mx = take::<1>(bytes, &mut pos)?[0] as i8;
        let mz = take::<1>(bytes, &mut pos)?[0] as i8;
        let facing = i16::from_le_bytes(take(bytes, &mut pos)?);
        let raw_buttons = take::<1>(bytes, &mut pos)?[0];
        let slot = FighterId::new(fighter).ok_or(FuzzError::Invalid("unknown fighter"))?;
        let base = FighterInput {
            tick,
            fighter: slot,
            mx,
            mz,
            facing,
            buttons: Default::default(),
        };
        // Keep raw bits: tape validation below rejects unknown buttons.
        let mut input = base;
        input.buttons.0 = raw_buttons;
        inputs.push(input);
    }
    let tape = FighterTape(inputs);
    if tape.validate().is_err() {
        return Err(FuzzError::Invalid("invalid fighter tape"));
    }
    if tape.0.iter().any(|input| {
        input.tick >= config.ticks || usize::from(input.fighter.0) >= usize::from(config.fighters)
    }) {
        return Err(FuzzError::Invalid("tape escapes its arena config"));
    }
    Ok((config, tape))
}

/// Save one failing case so it replays exactly.
///
/// Writes versioned `config + tape` bytes to `dir` (created as needed) under
/// a deterministic name derived from the case and its content hash, so the
/// same case always lands at the same path.
///
/// # Errors
///
/// Returns [`FuzzError`] for invalid config/tape or filesystem failures.
pub fn save_repro(
    dir: &Path,
    config: &ArenaConfig,
    tape: &FighterTape,
) -> Result<PathBuf, FuzzError> {
    let bytes = encode_repro(config, tape)?;
    std::fs::create_dir_all(dir)?;
    let digest = hash_bytes(&bytes);
    let (seed, ticks, fighters) = (config.seed, config.ticks, config.fighters);
    let name = format!("fuzz-{seed:016x}-{ticks}-{fighters}-{digest:016x}.bin");
    let path = dir.join(name);
    std::fs::write(&path, &bytes)?;
    Ok(path)
}

/// Load one saved repro.
///
/// # Errors
///
/// Returns [`FuzzError`] for filesystem failures, corrupt bytes, version
/// drift or any range violation.
pub fn load_repro(path: &Path) -> Result<(ArenaConfig, FighterTape), FuzzError> {
    let len = std::fs::metadata(path)?.len();
    if len > MAX_REPRO_BYTES as u64 {
        return Err(FuzzError::Invalid("repro size is outside bounds"));
    }
    let bytes = std::fs::read(path)?;
    decode_repro(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fuzz_config(seed: u64) -> ArenaConfig {
        ArenaConfig {
            seed,
            ticks: 32,
            fighters: 2,
            half_extent_m: 20.0,
        }
    }

    fn arena_config(seed: u64) -> ArenaConfig {
        ArenaConfig {
            seed,
            ticks: ADVERSARIAL_TICKS,
            fighters: ADVERSARIAL_FIGHTERS,
            half_extent_m: 10.0,
        }
    }

    #[test]
    fn same_seed_twice_gives_same_tape_corpus() {
        assert_eq!(gen_tape(7, 32, 2), gen_tape(7, 32, 2));
        assert_eq!(adversarial(7), adversarial(7));
        // Neighbouring seeds drive disjoint SplitMix64 streams, so corpora
        // from different seeds draw from different randomness.
        assert_ne!(expand_seed(7, 8), expand_seed(8, 8));
        gen_tape(7, 32, 2).validate().unwrap();
        for tape in adversarial(7) {
            tape.validate().unwrap();
        }
        assert_eq!(ADVERSARIAL_NAMES.len(), adversarial(7).len());
    }

    #[test]
    fn corpus_replays_identical_checkpoints_twice() {
        let config = fuzz_config(5);
        let tape = gen_tape(5, 32, 2);
        let (first, second) = (
            engine_arena::run(&config, &tape),
            engine_arena::run(&config, &tape),
        );
        assert_eq!(first.len(), 32);
        assert_eq!(first, second);
        let arena = arena_config(5);
        for tape in adversarial(5) {
            let (first, second) = (
                engine_arena::run(&arena, &tape),
                engine_arena::run(&arena, &tape),
            );
            assert_eq!(first.len(), ADVERSARIAL_TICKS as usize);
            assert_eq!(first, second);
        }
    }

    #[test]
    fn adversarial_scripts_replay_without_panic_and_hold_invariants() {
        let config = arena_config(9);
        for (name, tape) in ADVERSARIAL_NAMES.into_iter().zip(adversarial(9)) {
            let checkpoints = engine_arena::run(&config, &tape);
            assert_eq!(checkpoints.len(), ADVERSARIAL_TICKS as usize, "{name}");
            assert!(checkpoints
                .windows(2)
                .all(|pair| pair[0].tick <= pair[1].tick));
            assert!(
                check_invariants(&config, &tape, &checkpoints).is_empty(),
                "{name}"
            );
        }
    }

    #[test]
    fn swapped_checkpoints_are_caught_with_their_tick() {
        let config = fuzz_config(11);
        let tape = gen_tape(11, 16, 2);
        let live = engine_arena::run(&config, &tape);
        assert!(check_invariants(&config, &tape, &live).is_empty());
        let mut tampered = live.clone();
        tampered.swap(2, 3);
        let fails = check_invariants(&config, &tape, &tampered);
        assert!(
            fails
                .iter()
                .any(|fail| fail.tick == 2 && fail.rule == Rule::TicksOrdered),
            "{fails:?}"
        );
    }

    #[test]
    fn empty_run_checkpoints_fail_hash_responds_for_input_tapes() {
        let config = arena_config(13);
        let tapes = adversarial(13);
        let baseline = engine_arena::run(&config, &FighterTape::default());
        assert_eq!(baseline.len(), ADVERSARIAL_TICKS as usize);
        let fails = check_invariants(&config, &tapes[0], &baseline);
        assert!(
            fails
                .iter()
                .any(|fail| fail.tick == 0 && fail.rule == Rule::HashResponds),
            "{fails:?}"
        );
    }

    #[test]
    fn saved_repro_reloads_and_replays_to_same_checkpoints() {
        let config = fuzz_config(21);
        let tape = gen_tape(21, 24, 2);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("e2040-fz1-{pid}"));
        let path = save_repro(&dir, &config, &tape).unwrap();
        let (config_back, tape_back) = load_repro(&path).unwrap();
        assert_eq!((config, tape.clone()), (config_back, tape_back.clone()));
        // Same path for the same case: the name is content-derived.
        assert_eq!(save_repro(&dir, &config, &tape).unwrap(), path);
        let (first, second) = (
            engine_arena::run(&config_back, &tape_back),
            engine_arena::run(&config, &tape),
        );
        assert_eq!(first, second);
        assert_eq!(first.len(), 32);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_repros_are_rejected() {
        let config = fuzz_config(23);
        let tape = gen_tape(23, 8, 2);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("e2040-fz1-{pid}"));
        let path = save_repro(&dir, &config, &tape).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        assert!(decode_repro(&[]).is_err());
        assert!(decode_repro(&bytes[..bytes.len() - 1]).is_err());
        bytes.push(0);
        assert!(decode_repro(&bytes).is_err());
        let mut bad = std::fs::read(&path).unwrap();
        bad[0] ^= 0xff;
        assert!(decode_repro(&bad).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn nan_bit_check_ignores_zero_one_and_infinity() {
        assert!(is_nan_bits(f32::NAN));
        for finite in [0.0, -0.0, 1.5, -100.0, f32::MAX, f32::MIN] {
            assert!(!is_nan_bits(finite));
        }
        assert!(!is_nan_bits(f32::INFINITY));
        assert!(!is_nan_bits(f32::NEG_INFINITY));
    }
}
