//! Strict bounded little-endian input-tape codec; no native layout or IO.

use crate::{
    validate, Checkpoint, Config, Error, Input, Tape, FIXTURE_VERSION, MAX_INPUTS, MAX_TICKS,
};
use std::path::Path;

const MAGIC: &[u8; 8] = b"E2040RP\0";
/// Tape envelope version, independent of simulation fixture rules.
pub const FORMAT_VERSION: u16 = 1;
const HEADER_BYTES: usize = 28;
/// Maximum accepted file length, checked before allocation by IO consumers.
pub const MAX_TAPE_BYTES: usize = HEADER_BYTES + MAX_INPUTS * 16 + MAX_TICKS as usize * 16;

impl Tape {
    /// Encode validated input data and exactly one checkpoint per tick.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        validate(self.config, &self.inputs)?;
        if self.checkpoints.len() != self.config.ticks as usize {
            return Err(Error::Invalid("checkpoint count does not match ticks"));
        }
        let mut bytes =
            Vec::with_capacity(HEADER_BYTES + self.inputs.len() * 16 + self.checkpoints.len() * 16);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&FIXTURE_VERSION.to_le_bytes());
        bytes.extend_from_slice(&self.config.seed.to_le_bytes());
        bytes.extend_from_slice(&self.config.ticks.to_le_bytes());
        bytes.extend_from_slice(&(self.inputs.len() as u32).to_le_bytes());
        for input in &self.inputs {
            input.append_bytes(&mut bytes);
        }
        for checkpoint in &self.checkpoints {
            bytes.extend_from_slice(&checkpoint.legacy.to_le_bytes());
            bytes.extend_from_slice(&checkpoint.state.to_le_bytes());
        }
        Ok(bytes)
    }

    /// Decode without trusting sizes, float values, versions or trailing bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < HEADER_BYTES || bytes.len() > MAX_TAPE_BYTES {
            return Err(Error::Invalid("tape size is outside bounds"));
        }
        let mut reader = Reader { remaining: bytes };
        if &reader.take::<8>()? != MAGIC {
            return Err(Error::Invalid("bad tape magic"));
        }
        let format = u16::from_le_bytes(reader.take()?);
        let fixture = u16::from_le_bytes(reader.take()?);
        if format != FORMAT_VERSION || fixture != FIXTURE_VERSION {
            return Err(Error::Version { format, fixture });
        }
        let seed = u64::from_le_bytes(reader.take()?);
        let ticks = u32::from_le_bytes(reader.take()?);
        let count = u32::from_le_bytes(reader.take()?) as usize;
        let config = Config { seed, ticks };
        validate(config, &[])?;
        if count > MAX_INPUTS || reader.remaining.len() != count * 16 + ticks as usize * 16 {
            return Err(Error::Invalid("tape counts or length mismatch"));
        }
        let mut inputs = Vec::with_capacity(count);
        for _ in 0..count {
            inputs.push(Input {
                tick: u32::from_le_bytes(reader.take()?),
                body: u32::from_le_bytes(reader.take()?),
                vx: f32::from_le_bytes(reader.take()?),
                vy: f32::from_le_bytes(reader.take()?),
            });
        }
        validate(config, &inputs)?;
        let mut checkpoints = Vec::with_capacity(ticks as usize);
        for _ in 0..ticks {
            checkpoints.push(Checkpoint {
                legacy: u64::from_le_bytes(reader.take()?),
                state: u64::from_le_bytes(reader.take()?),
            });
        }
        Ok(Self {
            config,
            inputs,
            checkpoints,
        })
    }

    /// Verify file bytes beside the frozen codec; any refusal names the file.
    /// Owner: engine-proof; consumer: [`crate::replay`] after verify.
    /// Units: file bytes plus path display; determinism: same bytes plus path
    /// gives the same tape or the same refusal.
    /// Errors: [`FileVerifyError`] naming `path` with the wrapped decode
    /// reason; the in-memory [`Tape::decode`] contract is unchanged.
    pub fn verify_file(path: &Path, bytes: &[u8]) -> Result<Self, FileVerifyError> {
        Self::decode(bytes).map_err(|error| FileVerifyError {
            path: path.display().to_string(),
            reason: error.to_string(),
        })
    }
}

/// Truncated or corrupt tape file refused, naming the partial file.
/// Owner: engine-proof; consumer: proof CLI `read_tape` via
/// [`Tape::verify_file`] (EV19.b); file IO only, never the fixed tick.
/// Units: file-system path display plus the wrapped decode reason;
/// determinism: same path plus bytes gives the same refusal.
/// Errors: display always names `path`, so a kill-9-style partial file is
/// diagnosable without trusting sizes, versions or trailing bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileVerifyError {
    /// Display form of the refused file path.
    pub path: String,
    /// Wrapped [`Tape::decode`] reason.
    pub reason: String,
}

impl std::fmt::Display for FileVerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "partial tape file '{}': {}", self.path, self.reason)
    }
}

impl std::error::Error for FileVerifyError {}

/// Shared bounded little-endian wire reader for the proof codecs. The tape
/// codec and the dataset shard codec both take fixed arrays through this one
/// body, so there is exactly one bounds check to keep correct.
pub(crate) struct Reader<'a> {
    pub(crate) remaining: &'a [u8],
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        self.take_named("truncated tape")
    }

    /// Take `N` bytes, blaming `what` when the wire ends early. Sibling
    /// codecs pass their own label so truncated inputs stay diagnosable.
    pub(crate) fn take_named<const N: usize>(
        &mut self,
        what: &'static str,
    ) -> Result<[u8; N], Error> {
        let bytes = self.remaining.get(..N).ok_or(Error::Invalid(what))?;
        let mut value = [0; N];
        value.copy_from_slice(bytes);
        self.remaining = &self.remaining[N..];
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{record, replay};

    fn recorded() -> Tape {
        record(
            Config { seed: 99, ticks: 8 },
            &[Input {
                tick: 2,
                body: 9,
                vx: -5.25,
                vy: 8.0,
            }],
        )
        .unwrap()
    }

    #[test]
    fn nonempty_tape_roundtrips_and_replays() {
        let original = recorded();
        let bytes = original.encode().unwrap();
        let decoded = Tape::decode(&bytes).unwrap();
        assert_eq!(original, decoded);
        assert_eq!(bytes, decoded.encode().unwrap());
        replay(&decoded).unwrap();
    }

    #[test]
    fn every_truncation_and_trailing_data_is_rejected() {
        let mut bytes = recorded().encode().unwrap();
        for end in 0..bytes.len() {
            assert!(Tape::decode(&bytes[..end]).is_err(), "length {end}");
        }
        bytes.push(0);
        assert!(Tape::decode(&bytes).is_err());
    }

    #[test]
    fn truncated_file_verify_refuses_naming_partial_file() {
        let bytes = recorded().encode().unwrap();
        let path = std::path::Path::new("out/partial.e2040");
        let label = "out/partial.e2040";
        for end in 0..bytes.len() {
            let error = Tape::verify_file(path, &bytes[..end]).unwrap_err();
            assert!(
                format!("{error}").contains(label),
                "prefix {end} refusal {error} names {label}"
            );
            assert_eq!(error.path, label);
            let plain = Tape::decode(&bytes[..end]).unwrap_err();
            assert!(
                !format!("{plain}").contains(label),
                "prefix {end} decode must stay path-free"
            );
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        let error = Tape::verify_file(path, &trailing).unwrap_err();
        assert!(format!("{error}").contains(label));
        let tape = Tape::verify_file(path, &bytes).unwrap();
        replay(&tape).unwrap();
    }

    #[test]
    fn malformed_versions_counts_and_float_are_rejected() {
        let bytes = recorded().encode().unwrap();
        for position in [0, 8, 10] {
            let mut bad = bytes.clone();
            bad[position] ^= 0xff;
            assert!(Tape::decode(&bad).is_err());
        }
        for position in [20, 24] {
            let mut bad = bytes.clone();
            bad[position..position + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(Tape::decode(&bad).is_err());
        }
        let mut bad = bytes;
        bad[HEADER_BYTES + 8..HEADER_BYTES + 12].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(Tape::decode(&bad).is_err());
    }

    #[test]
    fn altered_input_is_detected_by_actual_simulation() {
        let mut tape = recorded();
        tape.inputs[0].vx = 19.0;
        let decoded = Tape::decode(&tape.encode().unwrap()).unwrap();
        assert!(matches!(replay(&decoded), Err(Error::Divergence { .. })));
    }
}
