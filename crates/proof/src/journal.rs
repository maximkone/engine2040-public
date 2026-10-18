//! Crash-safe append-only tick journal, the streaming sibling of the
//! whole-blob [`Tape`] codec in [`crate::tape`].
//!
//! The tape codec writes one blob after the run, so a crash mid-run keeps
//! nothing. A journal writes a small header, then one self-delimiting record
//! per tick and flushes after each, so a crash keeps every finished tick.
//! The reader keeps every whole record, drops only a truncated trailing one
//! and fails hard on any damaged record before the end.
//!
//! Owner: engine-proof. Consumers: `engine2040 --journal` records through
//! [`record_journal`]; `engine2040 --replay` reads journals through
//! [`read_journal`]. The tape codec and its `FORMAT_VERSION` are untouched:
//! [`JournalRead::into_tape`] yields a normal [`Tape`] once every planned tick
//! is present.
//!
//! Layout, all little-endian, hashes are FNV-1a 64 from `engine_core`:
//! - header (32 bytes): magic `E2040JN\0`, journal version `u16`, fixture
//!   version `u16`, seed `u64`, planned ticks `u32`, hash of those 24 bytes.
//! - record, one per tick in order: body length `u32`, tick `u32`, hash of
//!   those 8 bytes (`u64`), then the body (16 bytes per input of this tick,
//!   then the 16 byte checkpoint), then a hash of the whole record so far.
//!
//! The head carries its own hash so a record whose length or tick bits flipped
//! is corruption, never mistaken for a short file. A cut can only hit the head,
//! the body or the closing hash, so a short file is a torn tail and a complete
//! record that fails its hash is damage. Scope: this survives process death
//! (the writer hands each record to the OS in one `write_all` plus `flush`);
//! surviving power loss needs a `Write` whose `flush` syncs the disk.
//! Determinism: the same config, inputs and checkpoints give the same bytes.
//! Bounds: sizes are checked against [`max_journal_bytes`], `MAX_TICKS` and
//! `MAX_INPUTS` before anything is allocated.

use crate::{validate, Checkpoint, Config, Error, Input, Tape, BODY_COUNT, MAX_INPUTS, MAX_TICKS};
use engine_core::hash_bytes;
use std::fmt;
use std::io::{self, Write};

mod read;
pub use read::{read_journal, JournalRead};
#[cfg(test)]
mod tests;

/// Journal magic, distinct from the tape magic `E2040RP\0`.
pub const MAGIC: [u8; 8] = *b"E2040JN\0";
/// Journal envelope version, independent of the tape and fixture versions.
pub const JOURNAL_VERSION: u16 = 1;
/// Header bytes: magic, two versions, seed, planned ticks, header hash.
pub const HEADER_BYTES: usize = 32;
/// Record head bytes: body length, tick, head hash.
pub(crate) const HEAD_BYTES: usize = 16;
/// Closing hash bytes of a record.
pub(crate) const SUM_BYTES: usize = 8;
/// One input row and the checkpoint are both 16 bytes.
pub(crate) const ROW_BYTES: usize = 16;

/// Largest accepted journal file in bytes: the header, one framed record per
/// planned tick and every input row the format allows. Readers check this
/// before allocating.
#[must_use]
pub fn max_journal_bytes() -> usize {
    let ticks = usize::try_from(MAX_TICKS).unwrap_or(usize::MAX);
    ticks
        .saturating_mul(HEAD_BYTES + ROW_BYTES + SUM_BYTES)
        .saturating_add(MAX_INPUTS.saturating_mul(ROW_BYTES))
        .saturating_add(HEADER_BYTES)
}

/// Most inputs one tick can carry: one per body slot.
pub(crate) fn max_tick_inputs() -> usize {
    usize::try_from(BODY_COUNT).unwrap_or(usize::MAX)
}

/// Why a journal could not be written or read.
#[derive(Debug, PartialEq, Eq)]
pub enum JournalError {
    /// Bounds, ranges, version or golden refusal from the proof layer.
    Proof(Error),
    /// The header is cut or is not a journal header.
    Header(&'static str),
    /// The record for `tick` is damaged, out of sequence or out of bounds.
    Corrupt {
        /// Zero-based tick of the record being read.
        tick: u32,
        /// Bounded diagnostic.
        reason: &'static str,
    },
    /// Fewer ticks than planned: `missing` of `planned` are absent.
    Incomplete {
        /// Ticks planned but absent.
        missing: u32,
        /// Ticks the header planned.
        planned: u32,
        /// A partial trailing record was dropped while reading.
        torn_tail: bool,
    },
    /// The sink failed; the file holds a readable prefix at most.
    Io(io::ErrorKind),
    /// The writer takes no more ticks.
    Closed(&'static str),
}

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Proof(error) => write!(f, "{error}"),
            Self::Header(reason) => write!(f, "journal header: {reason}"),
            Self::Corrupt { tick, reason } => {
                write!(f, "journal corrupt at tick {tick}: {reason}")
            }
            Self::Incomplete {
                missing,
                planned,
                torn_tail,
            } => {
                write!(
                    f,
                    "journal incomplete: {missing} of {planned} ticks missing"
                )?;
                if *torn_tail {
                    f.write_str(" (a torn trailing record was dropped)")?;
                }
                Ok(())
            }
            Self::Io(kind) => write!(f, "journal write failed: {kind}"),
            Self::Closed(reason) => write!(f, "journal closed: {reason}"),
        }
    }
}

impl std::error::Error for JournalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Proof(error) => Some(error),
            _ => None,
        }
    }
}

impl From<Error> for JournalError {
    fn from(error: Error) -> Self {
        Self::Proof(error)
    }
}

impl From<io::Error> for JournalError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

/// Appends one flushed record per tick to any [`Write`] sink.
///
/// Owner: the caller owns the sink; the writer owns the tick counter and the
/// reused record buffer. A failed write closes the writer, because the sink may
/// hold half a record and a later record would bury it mid-file.
pub struct JournalWriter<W: Write> {
    out: W,
    config: Config,
    next_tick: u32,
    rows_written: usize,
    failed: bool,
    scratch: Vec<u8>,
}

impl<W: Write> JournalWriter<W> {
    /// Validate `config`, write and flush the header, and start at tick 0.
    pub fn begin(mut out: W, config: Config) -> Result<Self, JournalError> {
        validate(config, &[])?;
        let mut header = Vec::with_capacity(HEADER_BYTES);
        header.extend_from_slice(&MAGIC);
        header.extend_from_slice(&JOURNAL_VERSION.to_le_bytes());
        header.extend_from_slice(&crate::FIXTURE_VERSION.to_le_bytes());
        header.extend_from_slice(&config.seed.to_le_bytes());
        header.extend_from_slice(&config.ticks.to_le_bytes());
        let sum = hash_bytes(&header);
        header.extend_from_slice(&sum.to_le_bytes());
        out.write_all(&header)?;
        out.flush()?;
        Ok(Self {
            out,
            config,
            next_tick: 0,
            rows_written: 0,
            failed: false,
            scratch: Vec::with_capacity(HEAD_BYTES + ROW_BYTES + SUM_BYTES),
        })
    }

    /// Append the record for the next tick and flush it.
    ///
    /// `inputs` are this tick's inputs only, validated like tape inputs.
    /// Refused ticks write nothing; an I/O failure closes the writer.
    pub fn append_tick(
        &mut self,
        inputs: &[Input],
        checkpoint: Checkpoint,
    ) -> Result<(), JournalError> {
        if self.failed {
            return Err(JournalError::Closed("an earlier write failed"));
        }
        if self.is_complete() {
            return Err(JournalError::Closed("every planned tick is written"));
        }
        let tick = self.next_tick;
        validate(self.config, inputs)?;
        if inputs.iter().any(|input| input.tick != tick) {
            return Err(Error::Invalid("input is not for the tick being appended").into());
        }
        if self.rows_written.saturating_add(inputs.len()) > MAX_INPUTS {
            return Err(Error::Invalid("too many inputs").into());
        }
        encode_record(&mut self.scratch, tick, inputs, checkpoint)?;
        let wrote = self
            .out
            .write_all(&self.scratch)
            .and_then(|()| self.out.flush());
        if let Err(error) = wrote {
            self.failed = true;
            return Err(error.into());
        }
        self.rows_written += inputs.len();
        self.next_tick += 1;
        Ok(())
    }

    /// Tick the next [`append_tick`](Self::append_tick) call writes.
    #[must_use]
    pub fn next_tick(&self) -> u32 {
        self.next_tick
    }

    /// Every planned tick has been appended.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.next_tick >= self.config.ticks
    }

    /// Give the sink back.
    pub fn into_inner(self) -> W {
        self.out
    }
}

fn encode_record(
    out: &mut Vec<u8>,
    tick: u32,
    inputs: &[Input],
    checkpoint: Checkpoint,
) -> Result<(), JournalError> {
    let body_len = u32::try_from((inputs.len() + 1) * ROW_BYTES)
        .map_err(|_| Error::Invalid("tick record is too large"))?;
    out.clear();
    out.extend_from_slice(&body_len.to_le_bytes());
    out.extend_from_slice(&tick.to_le_bytes());
    let head_sum = hash_bytes(out);
    out.extend_from_slice(&head_sum.to_le_bytes());
    for input in inputs {
        input.append_bytes(out);
    }
    out.extend_from_slice(&checkpoint.legacy.to_le_bytes());
    out.extend_from_slice(&checkpoint.state.to_le_bytes());
    let sum = hash_bytes(out);
    out.extend_from_slice(&sum.to_le_bytes());
    Ok(())
}

/// Run the fixture like [`crate::record`], appending each tick to `out` as it
/// finishes, and return the same [`Tape`].
///
/// A crash or failure leaves a readable journal of the finished ticks. The
/// state checkpoint folds every input of the run, so the caller passes the
/// whole input list up front exactly as for `record`. Pass `&mut file` to keep
/// the sink. Errors: [`JournalError`]; the sink holds the prefix written so far.
pub fn record_journal<W: Write>(
    config: Config,
    inputs: &[Input],
    out: W,
) -> Result<Tape, JournalError> {
    validate(config, inputs)?;
    let mut writer = JournalWriter::begin(out, config)?;
    let checkpoints = crate::execute_with(config, inputs, None, |own, checkpoint| {
        writer.append_tick(own, checkpoint)
    })?;
    Ok(Tape {
        config,
        inputs: inputs.to_vec(),
        checkpoints,
    })
}
