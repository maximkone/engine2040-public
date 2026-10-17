//! CR2: structured divergence capture for replay tapes.
//!
//! Consumer: [`crate::seeded_bug::render_report`] posts [`capture_line`] into
//! the A7 evidence report; bug triage decodes the same 48 bytes with
//! [`DivergenceCapture::from_bytes`]. Data owner: the caller owns the config
//! and tapes; this module borrows and returns one fixed record. Units: fixed
//! ticks, fighter slots, integer hashes. Determinism: the same config plus
//! tapes twice gives the identical record (deterministic resimulation, the
//! diagnostic split of `engine_arena::sync`, no wall clock, no float).
//! Errors: [`CausalError`]; a pair that never diverges has nothing to capture.

use engine_arena::sync::{run_system_hashes, SystemHashes, SYSTEM_NAMES};
use engine_arena::{ArenaConfig, BodyTape};
use engine_core::hash_bytes;

use crate::causal::{attribute, CausalError};

/// Record layout version, byte 0 of every capture.
pub const CAPTURE_VERSION: u8 = 1;
/// Fixed record size: version, two ticks, slot, system, 5 input bytes and
/// four `u64` digests. Well under the 64 byte per-checkpoint budget.
pub const CAPTURE_BYTES: usize = 48;
/// `system` value when the five diagnostic splits agree but the authority
/// checkpoint still differs (state the split does not cover).
pub const SYSTEM_UNSPLIT: u8 = u8::MAX;

/// First divergence of a failing tape from its baseline: where the hashes
/// leave, which input caused it, and the digests on both sides.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DivergenceCapture {
    /// First tick whose checkpoint differs from the baseline.
    pub tick: u32,
    /// Tick of the attributed failing input (at or before `tick`).
    pub cause_tick: u32,
    /// Fighter slot of the attributed input.
    pub slot: u8,
    /// Index into [`SYSTEM_NAMES`] of the first diverging system, or
    /// [`SYSTEM_UNSPLIT`].
    pub system: u8,
    /// Forward wish of the attributed input, percent of top speed.
    pub mx: i8,
    /// Right wish of the attributed input, percent of top speed.
    pub mz: i8,
    /// Facing of the attributed input, centidegrees.
    pub facing: i16,
    /// Button bits of the attributed input.
    pub buttons: u8,
    /// Baseline digest of the diverging system (`0` when unsplit).
    pub system_expected: u64,
    /// Failing digest of the diverging system (`0` when unsplit).
    pub system_observed: u64,
    /// Baseline checkpoint hash at `tick`.
    pub expected: u64,
    /// Failing checkpoint hash at `tick`.
    pub observed: u64,
}

impl DivergenceCapture {
    /// Name of the diverging system, `"unsplit"` when no split differs.
    pub fn system_name(&self) -> &'static str {
        SYSTEM_NAMES
            .get(usize::from(self.system))
            .copied()
            .unwrap_or("unsplit")
    }

    /// Encode the fixed little-endian record.
    pub fn to_bytes(&self) -> [u8; CAPTURE_BYTES] {
        let mut out = [0u8; CAPTURE_BYTES];
        out[0] = CAPTURE_VERSION;
        out[1..5].copy_from_slice(&self.tick.to_le_bytes());
        out[5..9].copy_from_slice(&self.cause_tick.to_le_bytes());
        out[9] = self.slot;
        out[10] = self.system;
        out[11] = self.mx.to_le_bytes()[0];
        out[12] = self.mz.to_le_bytes()[0];
        out[13..15].copy_from_slice(&self.facing.to_le_bytes());
        out[15] = self.buttons;
        for (i, digest) in [
            self.system_expected,
            self.system_observed,
            self.expected,
            self.observed,
        ]
        .iter()
        .enumerate()
        {
            out[16 + i * 8..24 + i * 8].copy_from_slice(&digest.to_le_bytes());
        }
        out
    }

    /// Decode a record; `None` on a wrong length or version.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != CAPTURE_BYTES || bytes[0] != CAPTURE_VERSION {
            return None;
        }
        let word = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or([0; 8]));
        let tick = u32::from_le_bytes(bytes[1..5].try_into().ok()?);
        Some(Self {
            tick,
            cause_tick: u32::from_le_bytes(bytes[5..9].try_into().ok()?),
            slot: bytes[9],
            system: bytes[10],
            mx: i8::from_le_bytes([bytes[11]]),
            mz: i8::from_le_bytes([bytes[12]]),
            facing: i16::from_le_bytes(bytes[13..15].try_into().ok()?),
            buttons: bytes[15],
            system_expected: word(16),
            system_observed: word(24),
            expected: word(32),
            observed: word(40),
        })
    }
}

/// Capture the first divergence of `failing` from `baseline` under `config`:
/// the divergent tick and attributed input from causal replay, the first
/// diverging system and its digests from the per-system split. Cold path:
/// one causal attribution plus two split runs.
///
/// # Errors
///
/// [`CausalError::NoFailureReproduces`] when the runs agree;
/// [`CausalError::CorruptTape`] for invalid configs or tapes.
pub fn capture(
    config: &ArenaConfig,
    baseline: &BodyTape,
    failing: &BodyTape,
) -> Result<DivergenceCapture, CausalError> {
    let report = attribute(config, baseline, failing)?;
    let cause = report.causes.first().ok_or(CausalError::CorruptTape)?;
    let split_a = run_system_hashes(config, baseline);
    let split_b = run_system_hashes(config, failing);
    let at = report.divergent_tick as usize;
    let (system, system_expected, system_observed) = match (split_a.get(at), split_b.get(at)) {
        (Some(a), Some(b)) => first_system(a, b),
        _ => (SYSTEM_UNSPLIT, 0, 0),
    };
    Ok(DivergenceCapture {
        tick: report.divergent_tick,
        cause_tick: cause.tick,
        slot: cause.fighter,
        system,
        mx: cause.input.mx,
        mz: cause.input.mz,
        facing: cause.input.facing,
        buttons: cause.input.buttons.0,
        system_expected,
        system_observed,
        expected: report.expected,
        observed: report.observed,
    })
}

/// First differing system index with both digests, in [`SYSTEM_NAMES`] order.
fn first_system(a: &SystemHashes, b: &SystemHashes) -> (u8, u64, u64) {
    let pairs = [
        (a.world, b.world),
        (a.space, b.space),
        (a.combat, b.combat),
        (a.rng, b.rng),
        (a.ai, b.ai),
    ];
    pairs
        .iter()
        .enumerate()
        .find(|(_, (x, y))| x != y)
        .and_then(|(i, (x, y))| Some((u8::try_from(i).ok()?, *x, *y)))
        .unwrap_or((SYSTEM_UNSPLIT, 0, 0))
}

/// Printable one-line capture: tick, cause, system and the record digest.
/// Same inputs twice print the same line.
pub fn capture_line(config: &ArenaConfig, baseline: &BodyTape, failing: &BodyTape) -> String {
    match capture(config, baseline, failing) {
        Ok(hit) => format!(
            "replay.capture tick={} cause_tick={} slot={} system={} bytes={} fnv={:016x}",
            hit.tick,
            hit.cause_tick,
            hit.slot,
            hit.system_name(),
            CAPTURE_BYTES,
            hash_bytes(&hit.to_bytes()),
        ),
        Err(CausalError::NoFailureReproduces) => {
            format!("replay.capture clean ticks={}", config.ticks)
        }
        Err(CausalError::CorruptTape) => "replay.capture corrupt".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks: 120,
            fighters: 2,
            half_extent_m: 20.0,
            table_version: 1,
        }
    }

    /// Ten seeded bugs: one wild input at (tick, slot) on top of the
    /// brain-driven baseline, varied over wish, facing and buttons.
    fn seeded(index: u32) -> (BodyTape, u32, u8) {
        let template = crate::agent::template_input(&config()).expect("template");
        let tick = 6 + index * 11;
        let slot = u8::try_from(index % 2).expect("slot");
        let mut input = template;
        input.tick = tick;
        input.fighter = engine_arena::BodyId::new(slot).expect("slot");
        match index % 5 {
            0 => input.mx = -100,
            1 => input.mz = 100,
            2 => input.facing = 17_000,
            3 => input.buttons.0 = 7,
            _ => {
                input.mx = 100;
                input.mz = -100;
            }
        }
        (BodyTape(vec![input]), tick, slot)
    }

    #[test]
    fn ten_seeded_bugs_attribute_to_exact_tick_and_input_x2() {
        let baseline = BodyTape::default();
        for index in 0..10 {
            let (failing, tick, slot) = seeded(index);
            let first = capture(&config(), &baseline, &failing).expect("diverges");
            let second = capture(&config(), &baseline, &failing).expect("replays");
            assert_eq!(first.to_bytes(), second.to_bytes(), "bug {index} x2");
            assert_eq!(
                (first.tick, first.cause_tick, first.slot),
                (tick, tick, slot)
            );
            let want = failing.0[0];
            assert_eq!(
                (first.mx, first.mz, first.facing, first.buttons),
                (want.mx, want.mz, want.facing, want.buttons.0),
                "bug {index} input"
            );
            assert_ne!(first.expected, first.observed);
            assert!(first.to_bytes().len() <= 64);
        }
    }

    #[test]
    fn record_round_trips_and_rejects_bad_bytes() {
        let (failing, _, _) = seeded(3);
        let hit = capture(&config(), &BodyTape::default(), &failing).expect("diverges");
        assert_eq!(DivergenceCapture::from_bytes(&hit.to_bytes()), Some(hit));
        assert!(DivergenceCapture::from_bytes(&hit.to_bytes()[1..]).is_none());
        let mut wrong = hit.to_bytes();
        wrong[0] = 9;
        assert!(DivergenceCapture::from_bytes(&wrong).is_none());
        assert_ne!(hit.system, SYSTEM_UNSPLIT, "the split names the system");
    }

    #[test]
    fn identical_tapes_capture_nothing() {
        let tape = BodyTape::default();
        assert_eq!(
            capture(&config(), &tape, &tape),
            Err(CausalError::NoFailureReproduces)
        );
        assert_eq!(
            capture_line(&config(), &tape, &tape),
            "replay.capture clean ticks=120"
        );
    }
}
