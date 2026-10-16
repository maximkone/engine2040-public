//! Tape-decode one-harness fuzz (Bolero `check!` shape, HS23 kernel).
//! Kernel: fighter-tape decode via the repro envelope; bound: `REPRO_FORMAT`
//! plus `MAX_REPRO_BYTES`. Consumer: `cargo test` one-harness (proof-only,
//! outside the tick; wired via the `tape_fuzz` mod line in lib.rs).
//! PORT(camshaft/bolero@7642943b6c1f8a9b5e824dd1cd29b7c8e56068b8:README):
//! shape only, zero donor bytes, no new dependency.

use crate::fuzz::{
    adversarial, gen_tape, ADVERSARIAL_FIGHTERS, ADVERSARIAL_TICKS, MAX_REPRO_BYTES, REPRO_FORMAT,
};
use engine_arena::ArenaConfig;
use engine_core::hash_bytes;
use engine_net::session::{BodyId, BodyInput, BodyTape};
use std::fmt;

const CORPUS_SEEDS: [u64; 4] = [7, 9, 19, 42];
const MAGIC: &[u8; 8] = b"E2040FZ\0";
const HEADER: usize = 32;
const RECORD: usize = 10;

/// Harness failure: caps, envelope, or the replay-hash oracle.
#[derive(Debug, PartialEq, Eq)]
pub enum TapeFuzzError {
    /// Longer than [`MAX_REPRO_BYTES`]; refused before allocation.
    TooLarge(usize),
    /// Magic, version, range, or length check failed.
    BadEnvelope(&'static str),
    /// Masked vs raw decodes of one input replayed apart.
    OracleDiverged { first: u64, second: u64 },
}

impl fmt::Display for TapeFuzzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge(len) => write!(f, "tape fuzz input too large: {len} bytes"),
            Self::BadEnvelope(reason) => write!(f, "bad tape fuzz envelope: {reason}"),
            Self::OracleDiverged { first, second } => {
                write!(f, "diverged {first:016x} {second:016x}")
            }
        }
    }
}

impl std::error::Error for TapeFuzzError {}

/// One repro input, button bits raw: `(tick, fighter, mx, mz, facing, buttons)`.
struct RawInput(u32, u8, i8, i8, i16, u8);

/// Bytes name a case: `selector % 5 == 0` is a gen tape, else adversarial.
fn base_case(seed: u64, selector: u8) -> Result<(ArenaConfig, BodyTape), TapeFuzzError> {
    if selector % 5 == 0 {
        let config = ArenaConfig::try_new(seed, 32, 2, 20.0)
            .map_err(|_| TapeFuzzError::BadEnvelope("bad config"))?;
        Ok((config, gen_tape(seed, 32, 2)))
    } else {
        let config = ArenaConfig::try_new(seed, ADVERSARIAL_TICKS, ADVERSARIAL_FIGHTERS, 10.0)
            .map_err(|_| TapeFuzzError::BadEnvelope("bad config"))?;
        let scripts = adversarial(seed);
        Ok((
            config,
            scripts[(usize::from(selector) - 1) % scripts.len()].clone(),
        ))
    }
}

/// Replay digest: ordered `(tick, hash)` bytes of one `run`, folded once.
fn digest_of(config: &ArenaConfig, tape: &BodyTape) -> u64 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&config.seed.to_le_bytes());
    bytes.extend_from_slice(&config.ticks.to_le_bytes());
    bytes.push(config.fighters);
    for checkpoint in engine_arena::run(config, tape) {
        bytes.extend_from_slice(&checkpoint.tick.to_le_bytes());
        bytes.extend_from_slice(&checkpoint.hash.to_le_bytes());
    }
    hash_bytes(&bytes)
}

/// Encode one case in the envelope (header plus raw 10-byte records).
fn encode_repro(config: &ArenaConfig, tape: &BodyTape) -> Result<Vec<u8>, TapeFuzzError> {
    let count =
        u32::try_from(tape.0.len()).map_err(|_| TapeFuzzError::BadEnvelope("too many inputs"))?;
    let mut bytes = Vec::with_capacity(HEADER + tape.0.len() * RECORD);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&REPRO_FORMAT.to_le_bytes());
    bytes.extend_from_slice(&config.seed.to_le_bytes());
    bytes.extend_from_slice(&config.ticks.to_le_bytes());
    bytes.push(config.fighters);
    bytes.push(0);
    bytes.extend_from_slice(&config.half_extent_m.to_bits().to_le_bytes());
    bytes.extend_from_slice(&count.to_le_bytes());
    for input in &tape.0 {
        bytes.extend_from_slice(&input.tick.to_le_bytes());
        bytes.push(input.fighter.0);
        bytes.push(input.mx.to_le_bytes()[0]);
        bytes.push(input.mz.to_le_bytes()[0]);
        bytes.extend_from_slice(&input.facing.to_le_bytes());
        bytes.push(input.buttons.0);
    }
    Ok(bytes)
}

/// Decode envelope bytes; sizes check before any input allocation.
fn decode_raw(bytes: &[u8]) -> Result<(ArenaConfig, Vec<RawInput>), TapeFuzzError> {
    if bytes.len() > MAX_REPRO_BYTES {
        return Err(TapeFuzzError::TooLarge(bytes.len()));
    }
    if bytes.len() < HEADER {
        return Err(TapeFuzzError::BadEnvelope("short repro"));
    }
    if !bytes.starts_with(MAGIC) {
        return Err(TapeFuzzError::BadEnvelope("bad magic"));
    }
    if u16::from_le_bytes([bytes[8], bytes[9]]) != REPRO_FORMAT {
        return Err(TapeFuzzError::BadEnvelope("bad format"));
    }
    let word =
        |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    let mut seed = [0u8; 8];
    seed.copy_from_slice(&bytes[10..18]);
    let config = ArenaConfig::try_new(
        u64::from_le_bytes(seed),
        word(18),
        bytes[22],
        f32::from_bits(word(24)),
    )
    .map_err(|_| TapeFuzzError::BadEnvelope("bad config"))?;
    let count = usize::try_from(word(28)).map_err(|_| TapeFuzzError::BadEnvelope("bad count"))?;
    let want = HEADER
        .checked_add(
            count
                .checked_mul(RECORD)
                .ok_or(TapeFuzzError::BadEnvelope("bad count"))?,
        )
        .ok_or(TapeFuzzError::BadEnvelope("bad count"))?;
    if bytes.len() != want {
        return Err(TapeFuzzError::BadEnvelope("length mismatch"));
    }
    let mut inputs = Vec::with_capacity(count);
    for k in 0..count {
        let at = HEADER + k * RECORD;
        inputs.push(RawInput(
            word(at),
            bytes[at + 4],
            i8::from_le_bytes([bytes[at + 5]]),
            i8::from_le_bytes([bytes[at + 6]]),
            i16::from_le_bytes([bytes[at + 7], bytes[at + 8]]),
            bytes[at + 9],
        ));
    }
    Ok((config, inputs))
}

/// One-harness body: envelope bytes decode two ways (masked vs raw button
/// bits) and both replay; equal digests pass. Other bytes name a case that
/// must replay x2 equal. Oversize is refused before allocation.
pub fn check_tape_bytes(bytes: &[u8]) -> Result<u64, TapeFuzzError> {
    if bytes.len() > MAX_REPRO_BYTES {
        return Err(TapeFuzzError::TooLarge(bytes.len()));
    }
    if bytes.len() >= HEADER && bytes.starts_with(MAGIC) {
        let (config, raw) = decode_raw(bytes)?;
        let mut masked = Vec::with_capacity(raw.len());
        let mut kept = Vec::with_capacity(raw.len());
        for row in &raw {
            let Some(slot) = BodyId::new(row.1) else {
                return Err(TapeFuzzError::BadEnvelope("unknown fighter"));
            };
            let base = BodyInput {
                tick: row.0,
                fighter: slot,
                mx: row.2,
                mz: row.3,
                facing: row.4,
                buttons: Default::default(),
            };
            let (mut a, mut b) = (base, base);
            a.buttons.0 = row.5 & 0x07;
            b.buttons.0 = row.5;
            masked.push(a);
            kept.push(b);
        }
        let first = digest_of(&config, &BodyTape(masked));
        let second = digest_of(&config, &BodyTape(kept));
        return if first == second {
            Ok(first)
        } else {
            Err(TapeFuzzError::OracleDiverged { first, second })
        };
    }
    let n = bytes.len().min(8);
    let mut seed = [0u8; 8];
    seed[..n].copy_from_slice(&bytes[..n]);
    let (config, tape) = base_case(
        u64::from_le_bytes(seed),
        bytes.first().copied().unwrap_or(0),
    )?;
    let first = digest_of(&config, &tape);
    let second = digest_of(&config, &tape);
    if first == second {
        Ok(first)
    } else {
        Err(TapeFuzzError::OracleDiverged { first, second })
    }
}

/// Mutation mode: same body, byte flips, no libFuzzer. Unknown-bits each
/// input in turn; returns cases run plus the first failing bytes.
pub fn mutate_until_flag(limit: usize) -> Option<(usize, Vec<u8>)> {
    let (config, tape) = base_case(9, 3).ok()?;
    let base = encode_repro(&config, &tape).ok()?;
    for k in 0..tape.0.len().min(limit) {
        let mut mutant = base.clone();
        mutant[HEADER + k * RECORD + 9] |= 0x08;
        if matches!(
            check_tape_bytes(&mutant),
            Err(TapeFuzzError::OracleDiverged { .. })
        ) {
            return Some((k + 1, mutant));
        }
    }
    None
}

/// Shrink a failure to its minimal failing prefix, re-emitted enveloped.
pub fn shrink_repro(bytes: &[u8]) -> Vec<u8> {
    let Ok((_, raw)) = decode_raw(bytes) else {
        return bytes.to_vec();
    };
    let Some(end) = raw.iter().position(|row| row.5 & !0x07 != 0) else {
        return bytes.to_vec();
    };
    let Ok(n) = u32::try_from(end + 1) else {
        return bytes.to_vec();
    };
    let mut out = bytes[..HEADER + (end + 1) * RECORD].to_vec();
    out[28..32].copy_from_slice(&n.to_le_bytes());
    if matches!(
        check_tape_bytes(&out),
        Err(TapeFuzzError::OracleDiverged { .. })
    ) {
        out
    } else {
        bytes.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tape_corpus_runs_green_and_reproduces() {
        for seed in CORPUS_SEEDS {
            for selector in 0..5 {
                let (config, tape) = base_case(seed, selector).expect("corpus case builds");
                let bytes = encode_repro(&config, &tape).expect("corpus case encodes");
                assert!(check_tape_bytes(&bytes).is_ok(), "{seed}:{selector}");
            }
        }
        let digest = || {
            let mut bytes = Vec::new();
            for seed in CORPUS_SEEDS {
                for selector in 0..5 {
                    let (config, tape) = base_case(seed, selector).expect("corpus case builds");
                    bytes.extend_from_slice(&digest_of(&config, &tape).to_le_bytes());
                }
            }
            hash_bytes(&bytes)
        };
        assert_eq!(digest(), digest());
        assert!(matches!(
            check_tape_bytes(&vec![0u8; MAX_REPRO_BYTES + 1]),
            Err(TapeFuzzError::TooLarge(_))
        ));
    }

    #[test]
    fn tape_fuzz_oracle_flags_seeded_divergence() {
        let (cases, mutant) =
            mutate_until_flag(1000).expect("oracle must flag the button-bits divergence");
        assert!(cases <= 1000, "{cases}");
        let shrunk = shrink_repro(&mutant);
        assert!(shrunk.len() <= 64, "{}", shrunk.len());
        assert!(matches!(
            check_tape_bytes(&shrunk),
            Err(TapeFuzzError::OracleDiverged { .. })
        ));
    }
}
