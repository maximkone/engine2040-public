//! mcap: std-only MCAP writer and reader for replay recordings.
//!
//! Consumer: desync hunts and the proof trace export write a recording that
//! Foxglove or Rerun opens as is (both read MCAP; no SDK is linked here), and
//! the reader gives the same messages back for tests and for
//! [`first_divergent_tick`]. Data owner: [`ReplayRecording`] owns the four
//! topics (`/tick/hash`, `/fighters/N/pose`, `/events`, `/debug/lines`);
//! [`McapWriter`] and [`read`] own the container format (magic, header,
//! schemas, channels, messages, data end, summary, footer). Units: ticks
//! (`u64`), millimetres and deci-degrees as in [`FpsPose`]. Message time is
//! `tick * STEP_NS`, so a viewer's time axis is sim time. Payloads are JSON
//! text built from integers and hex strings only. Determinism: no clock, no
//! float, no random; the same calls give the same bytes. Errors: typed
//! [`McapError`]; a damaged file fails, it never panics. Budget: about 150
//! bytes per fighter-tick, so a 600-tick two-fighter bout stays far under
//! 400 KB. Chunking, compression and CRCs are left out on purpose; MCAP
//! readers accept a file without them (a zero CRC means "not computed").

use std::fmt;

use engine_core::time::STEP_NS;

use crate::fps::FpsPose;

/// The eight magic bytes that open and close every MCAP file.
const MAGIC: [u8; 8] = [0x89, b'M', b'C', b'A', b'P', b'0', b'\r', b'\n'];
const OP_HEADER: u8 = 0x01;
const OP_FOOTER: u8 = 0x02;
const OP_SCHEMA: u8 = 0x03;
const OP_CHANNEL: u8 = 0x04;
const OP_MESSAGE: u8 = 0x05;
const OP_CHUNK: u8 = 0x06;
const OP_STATISTICS: u8 = 0x0B;
const OP_DATA_END: u8 = 0x0F;

/// Topic that carries one state hash per tick.
pub const TOPIC_HASH: &str = "/tick/hash";
/// Topic that carries free-text sim events.
pub const TOPIC_EVENTS: &str = "/events";
/// Topic that carries debug line segments.
pub const TOPIC_LINES: &str = "/debug/lines";

const SCHEMA_ENCODING: &str = "jsonschema";
const MESSAGE_ENCODING: &str = "json";
const SCHEMA_HASH: &str =
    r#"{"type":"object","properties":{"tick":{"type":"integer"},"hash":{"type":"string"}}}"#;
const SCHEMA_POSE: &str = r#"{"type":"object","properties":{"tick":{"type":"integer"},"x_mm":{"type":"integer"},"y_mm":{"type":"integer"},"z_mm":{"type":"integer"},"yaw_ddeg":{"type":"integer"}}}"#;
const SCHEMA_EVENT: &str =
    r#"{"type":"object","properties":{"tick":{"type":"integer"},"text":{"type":"string"}}}"#;
const SCHEMA_LINE: &str = r#"{"type":"object","properties":{"tick":{"type":"integer"},"a":{"type":"array","items":{"type":"integer"}},"b":{"type":"array","items":{"type":"integer"}}}}"#;

/// Typed MCAP failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McapError {
    /// The file does not start and end with the MCAP magic.
    BadMagic,
    /// A record or field runs past the end of the bytes.
    Truncated,
    /// A string field is not UTF-8.
    BadUtf8,
    /// The file has no footer record before the closing magic.
    MissingFooter,
    /// A message names a channel that was never declared.
    UnknownChannel(u16),
    /// The file uses chunks, which this reader does not decode.
    Chunked,
    /// The statistics record disagrees with the messages actually read.
    CountMismatch {
        /// Message count the statistics record claims.
        stated: u64,
        /// Messages found in the data section.
        found: u64,
    },
    /// More than `u16::MAX` channels or schemas, or text longer than `u32::MAX`.
    TooLarge,
}

impl fmt::Display for McapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => write!(f, "mcap: bad magic"),
            Self::Truncated => write!(f, "mcap: file is truncated"),
            Self::BadUtf8 => write!(f, "mcap: string is not utf-8"),
            Self::MissingFooter => write!(f, "mcap: no footer"),
            Self::UnknownChannel(id) => write!(f, "mcap: message on unknown channel {id}"),
            Self::Chunked => write!(f, "mcap: chunked files are not supported"),
            Self::CountMismatch { stated, found } => {
                write!(f, "mcap: statistics say {stated} messages, found {found}")
            }
            Self::TooLarge => write!(f, "mcap: too many channels or text too long"),
        }
    }
}

impl std::error::Error for McapError {}

/// One declared channel: a topic plus the schema its payloads follow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McapChannel {
    /// Channel id, unique in the file.
    pub id: u16,
    /// Schema id, `0` when the channel has none.
    pub schema_id: u16,
    /// Topic name such as `/tick/hash`.
    pub topic: String,
    /// Payload encoding such as `json`.
    pub message_encoding: String,
}

/// One recorded message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McapMessage {
    /// Channel the message belongs to.
    pub channel: u16,
    /// Per-channel sequence number, from zero.
    pub sequence: u32,
    /// Log time in nanoseconds.
    pub log_time: u64,
    /// Payload bytes.
    pub data: Vec<u8>,
}

/// A whole file read back by [`read`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McapFile {
    /// Header profile string.
    pub profile: String,
    /// Header library string.
    pub library: String,
    /// Channels in declaration order.
    pub channels: Vec<McapChannel>,
    /// Messages in file order.
    pub messages: Vec<McapMessage>,
}

impl McapFile {
    /// Messages on `topic`, in file order.
    pub fn on_topic(&self, topic: &str) -> impl Iterator<Item = &McapMessage> {
        let ids: Vec<u16> = self
            .channels
            .iter()
            .filter(|channel| channel.topic == topic)
            .map(|channel| channel.id)
            .collect();
        self.messages
            .iter()
            .filter(move |message| ids.contains(&message.channel))
    }
}

fn put_str(out: &mut Vec<u8>, text: &str) -> Result<(), McapError> {
    let len = u32::try_from(text.len()).map_err(|_| McapError::TooLarge)?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(text.as_bytes());
    Ok(())
}

/// Byte count as a record length; a slice cannot exceed `u64::MAX` bytes.
fn len64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

fn put_record(out: &mut Vec<u8>, opcode: u8, body: &[u8]) {
    out.push(opcode);
    out.extend_from_slice(&len64(body.len()).to_le_bytes());
    out.extend_from_slice(body);
}

/// Streaming MCAP writer into memory: declare channels, add messages, then
/// [`McapWriter::finish`] for the bytes.
#[derive(Debug, Clone)]
pub struct McapWriter {
    out: Vec<u8>,
    // Schema and channel records again, for the summary section.
    summary: Vec<u8>,
    schemas: Vec<&'static str>,
    sequences: Vec<u32>,
    counts: Vec<u64>,
    first_time: Option<u64>,
    last_time: u64,
}

impl McapWriter {
    /// Start a file with the magic and a header record.
    ///
    /// # Errors
    ///
    /// Returns [`McapError::TooLarge`] when a string exceeds `u32::MAX` bytes.
    pub fn new(profile: &str, library: &str) -> Result<Self, McapError> {
        let mut out = MAGIC.to_vec();
        let mut body = Vec::new();
        put_str(&mut body, profile)?;
        put_str(&mut body, library)?;
        put_record(&mut out, OP_HEADER, &body);
        Ok(Self {
            out,
            summary: Vec::new(),
            schemas: Vec::new(),
            sequences: Vec::new(),
            counts: Vec::new(),
            first_time: None,
            last_time: 0,
        })
    }

    /// Declare a JSON channel on `topic` whose payloads follow the JSON
    /// schema text `schema`. Schemas with the same `name` are declared once.
    /// Returns the channel id (the first channel is `0`).
    ///
    /// # Errors
    ///
    /// Returns [`McapError::TooLarge`] past `u16::MAX` channels or schemas.
    pub fn channel(
        &mut self,
        topic: &str,
        name: &'static str,
        schema: &str,
    ) -> Result<u16, McapError> {
        let schema_id = match self.schemas.iter().position(|known| *known == name) {
            Some(index) => index + 1,
            None => {
                self.schemas.push(name);
                let id = u16::try_from(self.schemas.len()).map_err(|_| McapError::TooLarge)?;
                let mut body = id.to_le_bytes().to_vec();
                put_str(&mut body, name)?;
                put_str(&mut body, SCHEMA_ENCODING)?;
                put_str(&mut body, schema)?;
                put_record(&mut self.out, OP_SCHEMA, &body);
                put_record(&mut self.summary, OP_SCHEMA, &body);
                self.schemas.len()
            }
        };
        let id = u16::try_from(self.sequences.len()).map_err(|_| McapError::TooLarge)?;
        let schema_id = u16::try_from(schema_id).map_err(|_| McapError::TooLarge)?;
        let mut body = Vec::new();
        body.extend_from_slice(&id.to_le_bytes());
        body.extend_from_slice(&schema_id.to_le_bytes());
        put_str(&mut body, topic)?;
        put_str(&mut body, MESSAGE_ENCODING)?;
        body.extend_from_slice(&0u32.to_le_bytes());
        put_record(&mut self.out, OP_CHANNEL, &body);
        put_record(&mut self.summary, OP_CHANNEL, &body);
        self.sequences.push(0);
        self.counts.push(0);
        Ok(id)
    }

    /// Append one message. An unknown `channel` is refused.
    ///
    /// # Errors
    ///
    /// Returns [`McapError::UnknownChannel`] for an undeclared channel.
    pub fn message(&mut self, channel: u16, log_time: u64, data: &[u8]) -> Result<(), McapError> {
        let slot = usize::from(channel);
        let sequence = *self
            .sequences
            .get(slot)
            .ok_or(McapError::UnknownChannel(channel))?;
        let mut body = Vec::with_capacity(22 + data.len());
        body.extend_from_slice(&channel.to_le_bytes());
        body.extend_from_slice(&sequence.to_le_bytes());
        body.extend_from_slice(&log_time.to_le_bytes());
        body.extend_from_slice(&log_time.to_le_bytes());
        body.extend_from_slice(data);
        put_record(&mut self.out, OP_MESSAGE, &body);
        self.sequences[slot] = sequence.wrapping_add(1);
        self.counts[slot] += 1;
        self.first_time.get_or_insert(log_time);
        self.last_time = self.last_time.max(log_time);
        Ok(())
    }

    /// Close the data section, write the summary (schemas, channels,
    /// statistics) and the footer, and return the file bytes.
    #[must_use]
    pub fn finish(mut self) -> Vec<u8> {
        put_record(&mut self.out, OP_DATA_END, &0u32.to_le_bytes());
        let summary_start = len64(self.out.len());
        // Sizes below are bounded by the u16 channel and schema ids that
        // `channel` enforces, so the saturating conversions never clamp.
        let mut stats = Vec::new();
        stats.extend_from_slice(&self.counts.iter().sum::<u64>().to_le_bytes());
        stats.extend_from_slice(
            &u16::try_from(self.schemas.len())
                .unwrap_or(u16::MAX)
                .to_le_bytes(),
        );
        stats.extend_from_slice(
            &u32::try_from(self.counts.len())
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        stats.extend_from_slice(&[0u8; 12]);
        stats.extend_from_slice(&self.first_time.unwrap_or(0).to_le_bytes());
        stats.extend_from_slice(&self.last_time.to_le_bytes());
        let map_len = u32::try_from(self.counts.len() * 10).unwrap_or(u32::MAX);
        stats.extend_from_slice(&map_len.to_le_bytes());
        for (id, count) in (0u16..).zip(&self.counts) {
            stats.extend_from_slice(&id.to_le_bytes());
            stats.extend_from_slice(&count.to_le_bytes());
        }
        put_record(&mut self.summary, OP_STATISTICS, &stats);
        self.out.extend_from_slice(&self.summary);
        let mut footer = Vec::new();
        footer.extend_from_slice(&summary_start.to_le_bytes());
        footer.extend_from_slice(&0u64.to_le_bytes());
        footer.extend_from_slice(&0u32.to_le_bytes());
        put_record(&mut self.out, OP_FOOTER, &footer);
        self.out.extend_from_slice(&MAGIC);
        self.out
    }
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], McapError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.data.len())
            .ok_or(McapError::Truncated)?;
        let bytes = &self.data[self.pos..end];
        self.pos = end;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], McapError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn u16(&mut self) -> Result<u16, McapError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, McapError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, McapError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn len32(&mut self) -> Result<usize, McapError> {
        usize::try_from(self.u32()?).map_err(|_| McapError::Truncated)
    }

    fn string(&mut self) -> Result<String, McapError> {
        let len = self.len32()?;
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| McapError::BadUtf8)
    }

    fn rest(&mut self) -> &'a [u8] {
        let bytes = &self.data[self.pos..];
        self.pos = self.data.len();
        bytes
    }
}

/// Read a whole unchunked MCAP file: header, channels and every message,
/// checking the magic at both ends, the footer and the statistics count.
///
/// # Errors
///
/// Returns [`McapError`] for a damaged, chunked or inconsistent file.
pub fn read(bytes: &[u8]) -> Result<McapFile, McapError> {
    let body = bytes
        .strip_prefix(&MAGIC)
        .and_then(|rest| rest.strip_suffix(&MAGIC))
        .ok_or(McapError::BadMagic)?;
    let mut cursor = Cursor { data: body, pos: 0 };
    let mut file = McapFile {
        profile: String::new(),
        library: String::new(),
        channels: Vec::new(),
        messages: Vec::new(),
    };
    let mut stated = None;
    let mut footer = false;
    // The summary section repeats the channels; only the data section counts.
    let mut in_summary = false;
    while cursor.pos < body.len() {
        let opcode = cursor.take(1)?[0];
        let len = usize::try_from(cursor.u64()?).map_err(|_| McapError::Truncated)?;
        let mut record = Cursor {
            data: cursor.take(len)?,
            pos: 0,
        };
        match opcode {
            OP_HEADER => {
                file.profile = record.string()?;
                file.library = record.string()?;
            }
            OP_CHANNEL if !in_summary => {
                let id = record.u16()?;
                let schema_id = record.u16()?;
                let topic = record.string()?;
                let message_encoding = record.string()?;
                file.channels.push(McapChannel {
                    id,
                    schema_id,
                    topic,
                    message_encoding,
                });
            }
            OP_MESSAGE => {
                let channel = record.u16()?;
                if !file.channels.iter().any(|known| known.id == channel) {
                    return Err(McapError::UnknownChannel(channel));
                }
                let sequence = record.u32()?;
                let log_time = record.u64()?;
                let _publish_time = record.u64()?;
                file.messages.push(McapMessage {
                    channel,
                    sequence,
                    log_time,
                    data: record.rest().to_vec(),
                });
            }
            OP_STATISTICS => stated = Some(record.u64()?),
            OP_DATA_END => in_summary = true,
            OP_FOOTER => footer = true,
            OP_CHUNK => return Err(McapError::Chunked),
            // Schemas are read by id only; unknown records are skipped, as
            // the format asks of every reader.
            _ => {}
        }
    }
    if !footer {
        return Err(McapError::MissingFooter);
    }
    let found = len64(file.messages.len());
    match stated {
        Some(stated) if stated != found => Err(McapError::CountMismatch { stated, found }),
        _ => Ok(file),
    }
}

/// JSON string body for `text`: quotes, backslashes and control characters
/// escaped, everything else kept as UTF-8.
fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out
}

/// The replay recording: four fixed topics over one [`McapWriter`]. The
/// `/tick/hash` channel is declared first, so a hunt can find it at once.
#[derive(Debug, Clone)]
pub struct ReplayRecording {
    writer: McapWriter,
    hash: u16,
    events: u16,
    lines: u16,
    poses: Vec<Option<u16>>,
}

impl ReplayRecording {
    /// Start an empty recording with the hash, event and line topics.
    ///
    /// # Errors
    ///
    /// Returns [`McapError`] only for an impossible size overflow.
    pub fn new() -> Result<Self, McapError> {
        let mut writer = McapWriter::new("e2040.replay", "engine2040")?;
        let hash = writer.channel(TOPIC_HASH, "e2040.TickHash", SCHEMA_HASH)?;
        let events = writer.channel(TOPIC_EVENTS, "e2040.Event", SCHEMA_EVENT)?;
        let lines = writer.channel(TOPIC_LINES, "e2040.Line", SCHEMA_LINE)?;
        Ok(Self {
            writer,
            hash,
            events,
            lines,
            poses: Vec::new(),
        })
    }

    /// Record the state hash of `tick`.
    ///
    /// # Errors
    ///
    /// Returns [`McapError`] only for an impossible size overflow.
    pub fn tick_hash(&mut self, tick: u64, hash: u64) -> Result<(), McapError> {
        let text = format!(r#"{{"tick":{tick},"hash":"{hash:016x}"}}"#);
        self.writer
            .message(self.hash, tick_time(tick), text.as_bytes())
    }

    /// Record one fighter pose on `/fighters/<n>/pose`; the topic is
    /// declared the first time that fighter appears.
    ///
    /// # Errors
    ///
    /// Returns [`McapError`] only for an impossible size overflow.
    pub fn pose(&mut self, pose: &FpsPose) -> Result<(), McapError> {
        let slot = usize::from(pose.fighter);
        if self.poses.len() <= slot {
            self.poses.resize(slot + 1, None);
        }
        let channel = match self.poses[slot] {
            Some(channel) => channel,
            None => {
                let topic = format!("/fighters/{}/pose", pose.fighter);
                let channel = self.writer.channel(&topic, "e2040.Pose", SCHEMA_POSE)?;
                self.poses[slot] = Some(channel);
                channel
            }
        };
        let text = format!(
            r#"{{"tick":{},"x_mm":{},"y_mm":{},"z_mm":{},"yaw_ddeg":{}}}"#,
            pose.tick, pose.x_mm, pose.y_mm, pose.z_mm, pose.yaw_ddeg
        );
        self.writer
            .message(channel, tick_time(pose.tick), text.as_bytes())
    }

    /// Record one free-text sim event at `tick`.
    ///
    /// # Errors
    ///
    /// Returns [`McapError`] only for an impossible size overflow.
    pub fn event(&mut self, tick: u64, text: &str) -> Result<(), McapError> {
        let text = format!(r#"{{"tick":{tick},"text":"{}"}}"#, json_escape(text));
        self.writer
            .message(self.events, tick_time(tick), text.as_bytes())
    }

    /// Record one debug line segment from `a` to `b` (millimetres).
    ///
    /// # Errors
    ///
    /// Returns [`McapError`] only for an impossible size overflow.
    pub fn debug_line(&mut self, tick: u64, a: [i32; 3], b: [i32; 3]) -> Result<(), McapError> {
        let text = format!(
            r#"{{"tick":{tick},"a":[{},{},{}],"b":[{},{},{}]}}"#,
            a[0], a[1], a[2], b[0], b[1], b[2]
        );
        self.writer
            .message(self.lines, tick_time(tick), text.as_bytes())
    }

    /// Close the recording and return the file bytes.
    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        self.writer.finish()
    }
}

/// Log time of `tick`: the tick index times the fixed step, saturating.
fn tick_time(tick: u64) -> u64 {
    tick.saturating_mul(STEP_NS)
}

/// First tick where the `/tick/hash` rows of two recordings disagree,
/// position by position. A row that differs returns the tick of `left`'s row;
/// when one recording is a strict prefix of the other, the first extra tick.
/// `None` when both hold the same rows.
#[must_use]
pub fn first_divergent_tick(left: &McapFile, right: &McapFile) -> Option<u64> {
    let mut a = left.on_topic(TOPIC_HASH);
    let mut b = right.on_topic(TOPIC_HASH);
    loop {
        match (a.next(), b.next()) {
            (Some(x), Some(y)) if x.log_time != y.log_time || x.data != y.data => {
                return Some(x.log_time / STEP_NS);
            }
            (Some(_), Some(_)) => {}
            (Some(x), None) | (None, Some(x)) => return Some(x.log_time / STEP_NS),
            (None, None) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A seeded 600-tick bout of two fighters. `flip` XORs the hash from
    /// that tick on, the way a real desync carries forward.
    fn bout(flip: Option<u64>) -> Vec<u8> {
        let mut rec = ReplayRecording::new().expect("channels fit");
        let mut state = 0x2040_u64;
        for tick in 0..600u64 {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let hash = if flip.is_some_and(|at| tick >= at) {
                state ^ 1
            } else {
                state
            };
            rec.tick_hash(tick, hash).expect("hash");
            for fighter in 0..2u8 {
                let sway = i32::try_from(tick).expect("small") * 7;
                let pose = FpsPose::try_new(
                    tick,
                    fighter,
                    1_000 * i32::from(fighter) + sway,
                    0,
                    -400,
                    900,
                )
                .expect("in range");
                rec.pose(&pose).expect("pose");
            }
            if tick % 50 == 0 {
                rec.event(tick, "hit \"fighter 1\"").expect("event");
            }
            if tick % 10 == 0 {
                rec.debug_line(tick, [0, 0, 0], [1_000, 0, 500])
                    .expect("line");
            }
        }
        rec.finish()
    }

    #[test]
    fn mcap_roundtrip_600_tick_bout_is_small_equal_and_complete() {
        let bytes = bout(None);
        assert!(bytes.len() <= 400 * 1024, "{} bytes", bytes.len());
        assert_eq!(bytes, bout(None), "same calls, same bytes");

        let file = read(&bytes).expect("reads back");
        assert_eq!(
            (file.profile.as_str(), file.library.as_str()),
            ("e2040.replay", "engine2040")
        );
        let topics: Vec<&str> = file.channels.iter().map(|c| c.topic.as_str()).collect();
        assert_eq!(
            topics,
            [
                "/tick/hash",
                "/events",
                "/debug/lines",
                "/fighters/0/pose",
                "/fighters/1/pose"
            ]
        );
        let counts: Vec<usize> = topics.iter().map(|t| file.on_topic(t).count()).collect();
        assert_eq!(counts, [600, 12, 60, 600, 600]);
        assert_eq!(file.messages.len(), 1_872, "every message comes back");
        let first = file.on_topic(TOPIC_HASH).next().expect("first hash");
        assert_eq!(first.sequence, 0);
        let first_hash = 0x2040_u64
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        assert_eq!(
            first.data,
            format!(r#"{{"tick":0,"hash":"{first_hash:016x}"}}"#).as_bytes()
        );
        let last = file.on_topic(TOPIC_HASH).last().expect("last hash");
        assert_eq!((last.sequence, last.log_time), (599, 599 * STEP_NS));
        let event = file.on_topic(TOPIC_EVENTS).next().expect("event");
        assert_eq!(event.data, br#"{"tick":0,"text":"hit \"fighter 1\""}"#);
    }

    #[test]
    fn mcap_roundtrip_seeded_desync_first_differs_at_its_tick() {
        let clean = read(&bout(None)).expect("clean");
        for at in [0, 1, 321, 599] {
            let moved = read(&bout(Some(at))).expect("moved");
            assert_eq!(first_divergent_tick(&clean, &moved), Some(at));
            assert_eq!(first_divergent_tick(&moved, &clean), Some(at));
        }
        assert_eq!(first_divergent_tick(&clean, &clean), None);

        let mut short = ReplayRecording::new().expect("channels");
        short.tick_hash(0, 7).expect("hash");
        let short = read(&short.finish()).expect("short");
        let mut long = ReplayRecording::new().expect("channels");
        long.tick_hash(0, 7).expect("hash");
        long.tick_hash(1, 8).expect("hash");
        let long = read(&long.finish()).expect("long");
        assert_eq!(
            first_divergent_tick(&short, &long),
            Some(1),
            "extra tail diverges"
        );
    }

    #[test]
    fn mcap_roundtrip_layout_matches_the_container_spec() {
        let bytes = bout(None);
        assert_eq!(&bytes[..8], &MAGIC);
        assert_eq!(&bytes[bytes.len() - 8..], &MAGIC);
        assert_eq!(bytes[8], OP_HEADER, "header record follows the magic");
        // Footer: opcode, length 20, then summary start, offsets, crc.
        let footer = bytes.len() - 8 - 29;
        assert_eq!(bytes[footer], OP_FOOTER);
        assert_eq!(
            u64::from_le_bytes(bytes[footer + 1..footer + 9].try_into().expect("8")),
            20
        );
        let summary_start = usize::try_from(u64::from_le_bytes(
            bytes[footer + 9..footer + 17].try_into().expect("8"),
        ))
        .expect("fits");
        assert_eq!(
            bytes[summary_start - 13],
            OP_DATA_END,
            "summary follows data end"
        );
        assert_eq!(bytes[summary_start], OP_SCHEMA);
    }

    #[test]
    fn mcap_roundtrip_refuses_damaged_files() {
        let bytes = bout(None);
        let mut bad_magic = bytes.clone();
        bad_magic[1] = b'X';
        assert_eq!(read(&bad_magic), Err(McapError::BadMagic));
        assert_eq!(read(&bytes[..bytes.len() - 1]), Err(McapError::BadMagic));
        assert_eq!(read(&[]), Err(McapError::BadMagic));
        // Cut inside the header record: the closing magic is intact, the record is not.
        let mut cut = bytes[..20].to_vec();
        cut.extend_from_slice(&MAGIC);
        assert_eq!(read(&cut), Err(McapError::Truncated));
        // A message on a channel nobody declared.
        let mut writer = McapWriter::new("p", "l").expect("header");
        assert_eq!(
            writer.message(3, 0, b"x"),
            Err(McapError::UnknownChannel(3))
        );
        let mut forged = MAGIC.to_vec();
        let mut body = vec![9, 0];
        body.extend_from_slice(&[0; 20]);
        put_record(&mut forged, OP_MESSAGE, &body);
        forged.extend_from_slice(&MAGIC);
        assert_eq!(read(&forged), Err(McapError::UnknownChannel(9)));
        // Chunks are not decoded, and no footer means no file.
        let mut chunked = MAGIC.to_vec();
        put_record(&mut chunked, OP_CHUNK, &[]);
        chunked.extend_from_slice(&MAGIC);
        assert_eq!(read(&chunked), Err(McapError::Chunked));
        let mut headless = MAGIC.to_vec();
        headless.extend_from_slice(&MAGIC);
        assert_eq!(read(&headless), Err(McapError::MissingFooter));
    }

    #[test]
    fn mcap_roundtrip_statistics_must_match_the_messages() {
        let mut writer = McapWriter::new("p", "l").expect("header");
        let id = writer.channel("/a", "s", "{}").expect("channel");
        writer.message(id, 5, b"one").expect("message");
        let mut bytes = writer.finish();
        let file = read(&bytes).expect("clean file reads");
        assert_eq!(file.messages.len(), 1);
        // The statistics record (9 header bytes, 56 body bytes for one channel)
        // sits just before the 29-byte footer and the closing magic; its first
        // field is the message count. Bump it and the reader notices.
        let at = bytes.len() - 8 - 29 - 65;
        assert_eq!(bytes[at], OP_STATISTICS);
        bytes[at + 9] = 2;
        assert_eq!(
            read(&bytes),
            Err(McapError::CountMismatch {
                stated: 2,
                found: 1
            })
        );
    }

    #[test]
    fn json_text_escapes_control_characters() {
        assert_eq!(json_escape("a\"b\\c\nd\u{1}e"), "a\\\"b\\\\c\\nd\\u0001e");
    }
}
