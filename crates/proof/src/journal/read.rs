//! Journal reader: whole records kept, a truncated tail dropped, damage fatal.

use super::{
    max_journal_bytes, max_tick_inputs, JournalError, HEADER_BYTES, HEAD_BYTES, JOURNAL_VERSION,
    MAGIC, ROW_BYTES, SUM_BYTES,
};
use crate::tape::Reader;
use crate::{validate, Checkpoint, Config, Error, Input, Tape, FIXTURE_VERSION, MAX_INPUTS};
use engine_core::hash_bytes;

/// What a journal file holds: its config and every complete tick record.
#[derive(Clone, Debug, PartialEq)]
pub struct JournalRead {
    /// Seed and planned tick count from the header.
    pub config: Config,
    /// Inputs of the complete ticks, in `(tick, body)` order.
    pub inputs: Vec<Input>,
    /// One checkpoint per complete tick, from tick 0 without gaps.
    pub checkpoints: Vec<Checkpoint>,
    /// A partial trailing record was found and dropped.
    pub torn_tail: bool,
}

impl JournalRead {
    /// Complete ticks read.
    #[must_use]
    pub fn ticks_present(&self) -> u32 {
        u32::try_from(self.checkpoints.len()).unwrap_or(u32::MAX)
    }

    /// Planned ticks the file does not hold.
    #[must_use]
    pub fn missing_ticks(&self) -> u32 {
        self.config.ticks.saturating_sub(self.ticks_present())
    }

    /// The normal tape of a journal that holds every planned tick.
    ///
    /// Errors: [`JournalError::Incomplete`] names the missing tick count when
    /// the run stopped early; nothing is padded or guessed.
    pub fn into_tape(self) -> Result<Tape, JournalError> {
        let missing = self.missing_ticks();
        if missing != 0 {
            return Err(JournalError::Incomplete {
                missing,
                planned: self.config.ticks,
                torn_tail: self.torn_tail,
            });
        }
        Ok(Tape {
            config: self.config,
            inputs: self.inputs,
            checkpoints: self.checkpoints,
        })
    }
}

/// Read journal bytes, keeping every complete record.
///
/// A cut in the middle of the last record sets `torn_tail` and keeps the
/// prefix; a cut exactly between records is a clean shorter prefix. A cut
/// header, a bad hash, a gap, a wrong tick or bytes after the last planned
/// tick are errors. Allocation follows the bytes present, never the header's
/// claimed tick count.
pub fn read_journal(bytes: &[u8]) -> Result<JournalRead, JournalError> {
    if bytes.len() > max_journal_bytes() {
        return Err(Error::Invalid("journal size is outside bounds").into());
    }
    let (config, body) = read_header(bytes)?;
    let planned = usize::try_from(config.ticks).unwrap_or(usize::MAX);
    let room = bytes.len() / (HEAD_BYTES + ROW_BYTES + SUM_BYTES);
    let mut read = JournalRead {
        config,
        inputs: Vec::new(),
        checkpoints: Vec::with_capacity(planned.min(room)),
        torn_tail: false,
    };
    let mut rest = body;
    while !rest.is_empty() {
        let tick = read.ticks_present();
        if tick >= config.ticks {
            return Err(corrupt(tick, "bytes after the last planned tick"));
        }
        match read_record(rest, tick, config, &mut read.inputs)? {
            Some((checkpoint, after)) => {
                read.checkpoints.push(checkpoint);
                rest = after;
            }
            None => {
                read.torn_tail = true;
                break;
            }
        }
    }
    Ok(read)
}

fn corrupt(tick: u32, reason: &'static str) -> JournalError {
    JournalError::Corrupt { tick, reason }
}

fn read_header(bytes: &[u8]) -> Result<(Config, &[u8]), JournalError> {
    let Some((header, body)) = bytes.split_at_checked(HEADER_BYTES) else {
        return Err(JournalError::Header("journal header is cut"));
    };
    let mut reader = Reader { remaining: header };
    if reader.take_named::<8>("journal header is cut")? != MAGIC {
        return Err(JournalError::Header("bad journal magic"));
    }
    let format = u16::from_le_bytes(reader.take_named("journal header is cut")?);
    let fixture = u16::from_le_bytes(reader.take_named("journal header is cut")?);
    if format != JOURNAL_VERSION || fixture != FIXTURE_VERSION {
        return Err(Error::Version { format, fixture }.into());
    }
    let seed = u64::from_le_bytes(reader.take_named("journal header is cut")?);
    let ticks = u32::from_le_bytes(reader.take_named("journal header is cut")?);
    let sum = u64::from_le_bytes(reader.take_named("journal header is cut")?);
    if hash_bytes(&header[..HEADER_BYTES - SUM_BYTES]) != sum {
        return Err(JournalError::Header("header checksum mismatch"));
    }
    let config = Config { seed, ticks };
    validate(config, &[])?;
    Ok((config, body))
}

/// One record: `Ok(None)` when `rest` ends inside it (a torn tail), the
/// checkpoint and the bytes after it when whole and sound, an error otherwise.
fn read_record<'a>(
    rest: &'a [u8],
    tick: u32,
    config: Config,
    inputs: &mut Vec<Input>,
) -> Result<Option<(Checkpoint, &'a [u8])>, JournalError> {
    let Some((head, after_head)) = rest.split_at_checked(HEAD_BYTES) else {
        return Ok(None);
    };
    let mut reader = Reader { remaining: head };
    let field = "record head is cut";
    let body_len = u32::from_le_bytes(reader.take_named(field)?);
    let head_tick = u32::from_le_bytes(reader.take_named(field)?);
    let head_sum = u64::from_le_bytes(reader.take_named(field)?);
    if hash_bytes(&head[..HEAD_BYTES - SUM_BYTES]) != head_sum {
        return Err(corrupt(tick, "record head checksum mismatch"));
    }
    if head_tick != tick {
        return Err(corrupt(tick, "record is out of sequence"));
    }
    let body_len = usize::try_from(body_len).unwrap_or(usize::MAX);
    let rows = body_len / ROW_BYTES;
    if !body_len.is_multiple_of(ROW_BYTES) || rows == 0 || rows > max_tick_inputs() + 1 {
        return Err(corrupt(tick, "record length is outside bounds"));
    }
    let Some((body_and_sum, after)) = after_head.split_at_checked(body_len + SUM_BYTES) else {
        return Ok(None);
    };
    let (body, sum_bytes) = body_and_sum.split_at(body_len);
    let mut sum_reader = Reader {
        remaining: sum_bytes,
    };
    let sum = u64::from_le_bytes(sum_reader.take_named("record sum is cut")?);
    if hash_bytes(&rest[..HEAD_BYTES + body_len]) != sum {
        return Err(corrupt(tick, "record checksum mismatch"));
    }
    let count = rows - 1;
    if inputs.len().saturating_add(count) > MAX_INPUTS {
        return Err(corrupt(tick, "too many inputs"));
    }
    let first = inputs.len();
    let mut body_reader = Reader { remaining: body };
    for _ in 0..count {
        let row = "input row is cut";
        inputs.push(Input {
            tick: u32::from_le_bytes(body_reader.take_named(row)?),
            body: u32::from_le_bytes(body_reader.take_named(row)?),
            vx: f32::from_le_bytes(body_reader.take_named(row)?),
            vy: f32::from_le_bytes(body_reader.take_named(row)?),
        });
    }
    let own = &inputs[first..];
    if own.iter().any(|input| input.tick != tick) {
        return Err(corrupt(tick, "input belongs to another tick"));
    }
    validate(config, own).map_err(|error| match error {
        Error::Invalid(reason) => corrupt(tick, reason),
        other => JournalError::Proof(other),
    })?;
    let checkpoint = Checkpoint {
        legacy: u64::from_le_bytes(body_reader.take_named("checkpoint is cut")?),
        state: u64::from_le_bytes(body_reader.take_named("checkpoint is cut")?),
    };
    Ok(Some((checkpoint, after)))
}
