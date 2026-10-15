//! Tape-to-dataset export: action-conditioned shards from any replay.
//!
//! Owner: engine-proof; consumer: provider training and eval harnesses read
//! the [`Shard`] bytes (state, action, observation, hash per sampled tick)
//! plus the one-line [`ShardManifest`] receipt. Units: ticks and LE bytes;
//! the observation triple carries FNV-1a64 digests, never floats.
//! Determinism: same tape plus stride gives identical bytes; replay never
//! calls a model. Errors: [`Error::Invalid`] for bad manifests, strides and
//! row counts, [`Error::Version`] for envelope mismatch,
//! [`Error::Divergence`] for the first replay mismatch. License shippability
//! stays with the distill provenance gate; the manifest only stamps an
//! explicit license string, and empty strings are refused. Budget: export and
//! verify replay every sampled tick from genesis, so tick-steps scale as rows
//! times ticks; hour tapes need a follow-up incremental pass and export here
//! with a wide stride.

use crate::{
    record, system_split_at, tape::Reader, validate, Checkpoint, Config, Error, Input, SystemSplit,
    MAX_INPUTS,
};
use engine_core::hash_bytes;
use std::fmt;

/// Shard envelope version, independent of the tape format version.
pub const SHARD_VERSION: u16 = 1;
/// Most rows one shard carries; larger exports raise the stride.
pub const MAX_ROWS: usize = 4096;
/// Largest accepted capture blob per row, in bytes.
pub const MAX_CAPTURE_BYTES: usize = 1 << 20;
/// Largest accepted capture total per shard, in bytes.
pub const MAX_CAPTURE_TOTAL_BYTES: usize = 16 << 20;
/// Largest accepted shard encoding, in bytes: covers full captures, rows and
/// inputs with headroom, checked before allocation by decode.
pub const MAX_SHARD_BYTES: usize = 20 << 20;
/// Longest manifest source id accepted, in chars.
pub const MAX_SHARD_SOURCE_CHARS: usize = 256;
/// Longest manifest license string accepted, in chars.
pub const MAX_SHARD_LICENSE_CHARS: usize = 64;
/// Longest training-purpose note accepted, in chars.
pub const MAX_SHARD_PURPOSE_CHARS: usize = 128;

const MAGIC: &[u8; 8] = b"E2040DS\0";

/// One sampled tick: the inputs applied before it, the observation triple
/// there, and both checkpoint scopes (state digest plus trajectory hash).
#[derive(Clone, Debug, PartialEq)]
pub struct ShardRow {
    /// Zero-based tick; strictly increasing within a shard.
    pub tick: u32,
    /// Inputs applied before this tick, in tape order.
    pub actions: Vec<Input>,
    /// World/space/clock digests at the post-step state.
    pub observation: SystemSplit,
    /// Both checkpoint scopes at the post-step state.
    pub checkpoint: Checkpoint,
    /// Optional capture blob (for example a PNG); opaque bytes, never hashed
    /// into the tick. `None` in headless fixture exports.
    pub capture: Option<Vec<u8>>,
}

/// Provider-agnostic receipt stapled to every shard: where the rows came
/// from, under what license, which content digest binds them, what they may
/// train, and the final tape hashes binding the shard to its replay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShardManifest {
    /// Collector id for the rows (tape range or teacher name).
    pub source: String,
    /// License the rows ship under; must be explicit, never empty.
    pub license: String,
    /// Content digest stamped at export; zero means unstamped and refused.
    pub digest: u64,
    /// What these rows may train (policy family or study name).
    pub trains: String,
    /// Row count; must match the shard.
    pub rows: usize,
    /// Final tape legacy hash binding the shard to its replay.
    pub tape_legacy: u64,
    /// Final tape state hash binding the shard to its replay.
    pub tape_state: u64,
}

/// Action-conditioned shard of one replay: the tape inputs plus one row per
/// sampled tick, bound by the manifest digest.
#[derive(Clone, Debug, PartialEq)]
pub struct Shard {
    /// Procedural initial-state configuration of the replayed tape.
    pub config: Config,
    /// Ordered, validated assignments of the replayed tape.
    pub inputs: Vec<Input>,
    /// Sampled per-tick rows in strictly increasing tick order.
    pub rows: Vec<ShardRow>,
    /// Receipt binding rows, license and final tape hashes.
    pub manifest: ShardManifest,
}

impl ShardManifest {
    /// Validate the fields and pack the manifest. Never panics.
    pub fn new(
        source: &str,
        license: &str,
        digest: u64,
        trains: &str,
        rows: usize,
        tape_legacy: u64,
        tape_state: u64,
    ) -> Result<Self, Error> {
        let manifest = Self {
            source: source.to_string(),
            license: license.to_string(),
            digest,
            trains: trains.to_string(),
            rows,
            tape_legacy,
            tape_state,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// Re-check a manifest built by any path (struct literal included):
    /// explicit source, license and purpose shapes, a stamped digest, and a
    /// row count inside the shard bound. Pure; never panics.
    pub fn validate(&self) -> Result<(), Error> {
        let source_chars = self.source.chars().count();
        if source_chars == 0 || source_chars > MAX_SHARD_SOURCE_CHARS {
            return Err(Error::Invalid("shard source id is empty or too long"));
        }
        let license_chars = self.license.chars().count();
        if license_chars == 0 || license_chars > MAX_SHARD_LICENSE_CHARS {
            return Err(Error::Invalid("shard license is empty or too long"));
        }
        if self.digest == 0 {
            return Err(Error::Invalid("shard digest is zero (unstamped)"));
        }
        let purpose_chars = self.trains.chars().count();
        if purpose_chars == 0 || purpose_chars > MAX_SHARD_PURPOSE_CHARS {
            return Err(Error::Invalid("shard purpose is empty or too long"));
        }
        if self.rows == 0 || self.rows > MAX_ROWS {
            return Err(Error::Invalid("shard row count is outside bounds"));
        }
        Ok(())
    }
}

impl fmt::Display for ShardManifest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "dataset {} {} rows {} digest {:016x} tape {:016x}/{:016x} for {}",
            self.source,
            self.license,
            self.rows,
            self.digest,
            self.tape_legacy,
            self.tape_state,
            self.trains
        )
    }
}

/// Format tag pinned in every manifest JSON document; a provider harness
/// checks this string before trusting any other field.
pub const MANIFEST_JSON_FORMAT: &str = "e2040-dataset-shard/1";
/// Longest manifest JSON document accepted, in bytes: the three text fields
/// at four bytes per char plus keys, hex and punctuation with headroom.
const MAX_MANIFEST_JSON_BYTES: usize = 4096;

/// One lowercase hex digit for the low nibble; total over all words, so no
/// conversion or failure case exists. Pure.
fn hex_char(nibble: u64) -> char {
    match nibble & 0xf {
        0 => '0',
        1 => '1',
        2 => '2',
        3 => '3',
        4 => '4',
        5 => '5',
        6 => '6',
        7 => '7',
        8 => '8',
        9 => '9',
        10 => 'a',
        11 => 'b',
        12 => 'c',
        13 => 'd',
        14 => 'e',
        _ => 'f',
    }
}

/// Append one JSON string with its quotes: `"` and `\` escape, characters
/// below the space character travel as `\u00XX`, all other Unicode passes
/// through raw. Pure; writes only into the caller's buffer.
fn append_json_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if u32::from(c) < 0x20 => {
                let code = u64::from(u32::from(c));
                out.push_str("\\u00");
                out.push(hex_char(code >> 4));
                out.push(hex_char(code));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Append a 16-digit lowercase hex digest. JSON numbers lose u64 precision
/// on some platforms, so digests travel as strings. Pure.
fn append_hex16(out: &mut String, mut value: u64) {
    let mut buf = ['0'; 16];
    for slot in buf.iter_mut().rev() {
        *slot = hex_char(value);
        value >>= 4;
    }
    for digit in buf {
        out.push(digit);
    }
}

/// Byte cursor over one manifest JSON document: strict canonical order, no
/// whitespace, no trailing bytes. Pure parsing; every failure names its
/// piece as [`Error::Invalid`].
struct JsonCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl JsonCursor<'_> {
    fn expect_byte(&mut self, want: u8, what: &'static str) -> Result<(), Error> {
        match self.bytes.get(self.pos) {
            Some(seen) if *seen == want => {
                self.pos += 1;
                Ok(())
            }
            _ => Err(Error::Invalid(what)),
        }
    }

    fn expect_key(&mut self, key: &[u8]) -> Result<(), Error> {
        if self.bytes.get(self.pos..self.pos + key.len()) == Some(key) {
            self.pos += key.len();
            self.expect_byte(b':', "manifest JSON needs a colon after its key")
        } else {
            Err(Error::Invalid(
                "manifest JSON key is missing or out of order",
            ))
        }
    }

    fn take_string(&mut self) -> Result<String, Error> {
        self.expect_byte(b'"', "manifest JSON must quote its strings")?;
        let mut raw = Vec::new();
        loop {
            let byte = *self
                .bytes
                .get(self.pos)
                .ok_or(Error::Invalid("truncated manifest JSON"))?;
            self.pos += 1;
            match byte {
                b'"' => break,
                b'\\' => self.take_escape(&mut raw)?,
                0x00..=0x1f => return Err(Error::Invalid("raw control in manifest JSON")),
                _ => raw.push(byte),
            }
        }
        String::from_utf8(raw).map_err(|_| Error::Invalid("manifest JSON is not UTF-8"))
    }

    fn take_escape(&mut self, raw: &mut Vec<u8>) -> Result<(), Error> {
        let esc = *self
            .bytes
            .get(self.pos)
            .ok_or(Error::Invalid("truncated manifest JSON"))?;
        self.pos += 1;
        match esc {
            b'"' => raw.push(b'"'),
            b'\\' => raw.push(b'\\'),
            b'n' => raw.push(b'\n'),
            b'r' => raw.push(b'\r'),
            b't' => raw.push(b'\t'),
            b'b' => raw.push(0x08),
            b'f' => raw.push(0x0c),
            b'u' => {
                let end = self
                    .pos
                    .checked_add(4)
                    .filter(|end| *end <= self.bytes.len())
                    .ok_or(Error::Invalid("truncated manifest JSON"))?;
                let digits = self
                    .bytes
                    .get(self.pos..end)
                    .ok_or(Error::Invalid("truncated manifest JSON"))?;
                let text = std::str::from_utf8(digits)
                    .map_err(|_| Error::Invalid("manifest JSON escape is not ASCII"))?;
                let code = u32::from_str_radix(text, 16)
                    .map_err(|_| Error::Invalid("manifest JSON escape is not hex"))?;
                let ch = char::from_u32(code)
                    .ok_or(Error::Invalid("manifest JSON escape is not a char"))?;
                let mut buf = [0u8; 4];
                raw.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                self.pos = end;
            }
            _ => return Err(Error::Invalid("unknown manifest JSON escape")),
        }
        Ok(())
    }

    fn take_hex16(&mut self) -> Result<u64, Error> {
        self.expect_byte(b'"', "manifest JSON must quote its digests")?;
        let end = self
            .pos
            .checked_add(16)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(Error::Invalid("truncated manifest JSON"))?;
        let digits = self
            .bytes
            .get(self.pos..end)
            .ok_or(Error::Invalid("truncated manifest JSON"))?;
        let text = std::str::from_utf8(digits)
            .map_err(|_| Error::Invalid("manifest JSON digest is not ASCII"))?;
        let value = u64::from_str_radix(text, 16)
            .map_err(|_| Error::Invalid("manifest JSON digest is not hex"))?;
        self.pos = end;
        self.expect_byte(b'"', "manifest JSON must close its digests")?;
        Ok(value)
    }

    fn take_rows(&mut self) -> Result<usize, Error> {
        let start = self.pos;
        while self
            .bytes
            .get(self.pos)
            .is_some_and(|byte| byte.is_ascii_digit())
        {
            self.pos += 1;
        }
        let digits = self
            .bytes
            .get(start..self.pos)
            .filter(|digits| !digits.is_empty())
            .ok_or(Error::Invalid("manifest JSON row count is not a number"))?;
        let text = std::str::from_utf8(digits)
            .map_err(|_| Error::Invalid("manifest JSON row count is not ASCII"))?;
        text.parse::<usize>()
            .map_err(|_| Error::Invalid("manifest JSON row count is past the word range"))
    }
}

impl ShardManifest {
    /// Render the receipt as canonical JSON with fixed key order, so any
    /// training harness (Rust, Python or otherwise) reads the manifest
    /// without the Rust wire codec. Digests travel as 16-hex strings and
    /// `rows` as a decimal count; the document carries no whitespace.
    /// Determinism: same manifest gives identical bytes. Errors: none;
    /// every storable manifest renders. Allocates once for the receipt;
    /// offline export only, never in the tick.
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(256);
        out.push_str("{\"format\":");
        append_json_string(&mut out, MANIFEST_JSON_FORMAT);
        out.push_str(",\"source\":");
        append_json_string(&mut out, &self.source);
        out.push_str(",\"license\":");
        append_json_string(&mut out, &self.license);
        out.push_str(",\"digest\":\"");
        append_hex16(&mut out, self.digest);
        out.push_str("\",\"trains\":");
        append_json_string(&mut out, &self.trains);
        out.push_str(",\"rows\":");
        out.push_str(&self.rows.to_string());
        out.push_str(",\"tape_legacy\":\"");
        append_hex16(&mut out, self.tape_legacy);
        out.push_str("\",\"tape_state\":\"");
        append_hex16(&mut out, self.tape_state);
        out.push_str("\"}");
        out
    }

    /// Parse the canonical JSON back into a manifest and re-validate every
    /// shape through [`ShardManifest::new`]: unknown format tags, reordered
    /// keys, bad hex, empty text and trailing bytes all fail as
    /// [`Error::Invalid`]. Pure; the digest still needs its replay check in
    /// [`verify_shard`].
    pub fn parse_json(text: &str) -> Result<Self, Error> {
        let bytes = text.as_bytes();
        if bytes.is_empty() || bytes.len() > MAX_MANIFEST_JSON_BYTES {
            return Err(Error::Invalid("manifest JSON length is outside bounds"));
        }
        let mut cursor = JsonCursor { bytes, pos: 0 };
        cursor.expect_byte(b'{', "manifest JSON must open with {")?;
        cursor.expect_key(b"\"format\"")?;
        let format = cursor.take_string()?;
        if format != MANIFEST_JSON_FORMAT {
            return Err(Error::Invalid(
                "manifest JSON format is not e2040-dataset-shard/1",
            ));
        }
        cursor.expect_byte(b',', "manifest JSON needs commas between fields")?;
        cursor.expect_key(b"\"source\"")?;
        let source = cursor.take_string()?;
        cursor.expect_byte(b',', "manifest JSON needs commas between fields")?;
        cursor.expect_key(b"\"license\"")?;
        let license = cursor.take_string()?;
        cursor.expect_byte(b',', "manifest JSON needs commas between fields")?;
        cursor.expect_key(b"\"digest\"")?;
        let digest = cursor.take_hex16()?;
        cursor.expect_byte(b',', "manifest JSON needs commas between fields")?;
        cursor.expect_key(b"\"trains\"")?;
        let trains = cursor.take_string()?;
        cursor.expect_byte(b',', "manifest JSON needs commas between fields")?;
        cursor.expect_key(b"\"rows\"")?;
        let rows = cursor.take_rows()?;
        cursor.expect_byte(b',', "manifest JSON needs commas between fields")?;
        cursor.expect_key(b"\"tape_legacy\"")?;
        let tape_legacy = cursor.take_hex16()?;
        cursor.expect_byte(b',', "manifest JSON needs commas between fields")?;
        cursor.expect_key(b"\"tape_state\"")?;
        let tape_state = cursor.take_hex16()?;
        cursor.expect_byte(b'}', "manifest JSON must close with }")?;
        if cursor.pos != bytes.len() {
            return Err(Error::Invalid("manifest JSON has trailing bytes"));
        }
        Self::new(
            &source,
            &license,
            digest,
            &trains,
            rows,
            tape_legacy,
            tape_state,
        )
    }
}

/// Check every shape a hand-built shard could lie about: tape config and
/// input order, row bound and strictly increasing ticks inside the tape,
/// action and capture totals, and the manifest against its row count. Pure.
fn check_shapes(shard: &Shard) -> Result<(), Error> {
    validate(shard.config, &shard.inputs)?;
    shard.manifest.validate()?;
    if shard.rows.len() != shard.manifest.rows {
        return Err(Error::Invalid("shard manifest row count does not match"));
    }
    let mut actions = 0usize;
    let mut captures = 0usize;
    let mut previous: Option<u32> = None;
    for row in &shard.rows {
        if row.tick >= shard.config.ticks {
            return Err(Error::Invalid("shard tick is past the tape end"));
        }
        if previous.is_some_and(|tick| tick >= row.tick) {
            return Err(Error::Invalid("shard ticks are not strictly increasing"));
        }
        previous = Some(row.tick);
        actions = actions
            .checked_add(row.actions.len())
            .filter(|total| *total <= MAX_INPUTS)
            .ok_or(Error::Invalid("shard carries too many actions"))?;
        if let Some(blob) = &row.capture {
            if blob.len() > MAX_CAPTURE_BYTES {
                return Err(Error::Invalid("shard capture blob is too large"));
            }
            captures = captures
                .checked_add(blob.len())
                .filter(|total| *total <= MAX_CAPTURE_TOTAL_BYTES)
                .ok_or(Error::Invalid("shard capture total is too large"))?;
        }
    }
    Ok(())
}

fn append_input(bytes: &mut Vec<u8>, input: &Input) {
    bytes.extend_from_slice(&input.tick.to_le_bytes());
    bytes.extend_from_slice(&input.body.to_le_bytes());
    bytes.extend_from_slice(&input.vx.to_le_bytes());
    bytes.extend_from_slice(&input.vy.to_le_bytes());
}

fn append_row(bytes: &mut Vec<u8>, row: &ShardRow) -> Result<(), Error> {
    bytes.extend_from_slice(&row.tick.to_le_bytes());
    let actions = u32::try_from(row.actions.len())
        .map_err(|_| Error::Invalid("shard action count is past the u32 range"))?;
    bytes.extend_from_slice(&actions.to_le_bytes());
    for action in &row.actions {
        append_input(bytes, action);
    }
    bytes.extend_from_slice(&row.observation.world.to_le_bytes());
    bytes.extend_from_slice(&row.observation.space.to_le_bytes());
    bytes.extend_from_slice(&row.observation.clock.to_le_bytes());
    bytes.extend_from_slice(&row.checkpoint.legacy.to_le_bytes());
    bytes.extend_from_slice(&row.checkpoint.state.to_le_bytes());
    match &row.capture {
        None => bytes.extend_from_slice(&0u32.to_le_bytes()),
        Some(blob) => {
            let len = u32::try_from(blob.len())
                .map_err(|_| Error::Invalid("shard capture blob is past the u32 range"))?;
            // Length zero is reserved for `None`, so an empty blob travels
            // as one zero byte and decodes back to `Some(empty)`.
            bytes.extend_from_slice(&(len + 1).to_le_bytes());
            bytes.extend_from_slice(blob);
        }
    }
    Ok(())
}

/// Canonical content bytes: config, inputs and rows in encode order. The
/// manifest digest is the engine hash of exactly these bytes, so any edit to
/// a row, an action or the config breaks the digest.
fn append_content_parts(
    config: Config,
    inputs: &[Input],
    rows: &[ShardRow],
    bytes: &mut Vec<u8>,
) -> Result<(), Error> {
    bytes.extend_from_slice(&config.seed.to_le_bytes());
    bytes.extend_from_slice(&config.ticks.to_le_bytes());
    let count = u32::try_from(inputs.len())
        .map_err(|_| Error::Invalid("shard input count is past the u32 range"))?;
    bytes.extend_from_slice(&count.to_le_bytes());
    for input in inputs {
        append_input(bytes, input);
    }
    let count = u32::try_from(rows.len())
        .map_err(|_| Error::Invalid("shard row count is past the u32 range"))?;
    bytes.extend_from_slice(&count.to_le_bytes());
    for row in rows {
        append_row(bytes, row)?;
    }
    Ok(())
}

fn append_text(bytes: &mut Vec<u8>, text: &str) -> Result<(), Error> {
    let len = u32::try_from(text.len())
        .map_err(|_| Error::Invalid("shard text field is past the u32 range"))?;
    bytes.extend_from_slice(&len.to_le_bytes());
    bytes.extend_from_slice(text.as_bytes());
    Ok(())
}

/// Decode one wire input (tick, body, velocity pair). Pure wire parsing.
fn decode_input(reader: &mut Reader<'_>) -> Result<Input, Error> {
    Ok(Input {
        tick: u32::from_le_bytes(reader.take_named("truncated shard")?),
        body: u32::from_le_bytes(reader.take_named("truncated shard")?),
        vx: f32::from_le_bytes(reader.take_named("truncated shard")?),
        vy: f32::from_le_bytes(reader.take_named("truncated shard")?),
    })
}

/// Decode the envelope head plus tape config and inputs. Pure wire parsing.
fn decode_head(reader: &mut Reader<'_>) -> Result<(Config, Vec<Input>), Error> {
    if reader.take_named::<8>("truncated shard")? != *MAGIC {
        return Err(Error::Invalid("bad shard magic"));
    }
    let version = u16::from_le_bytes(reader.take_named("truncated shard")?);
    if version != SHARD_VERSION {
        return Err(Error::Version {
            format: version,
            fixture: crate::FIXTURE_VERSION,
        });
    }
    let seed = u64::from_le_bytes(reader.take_named("truncated shard")?);
    let ticks = u32::from_le_bytes(reader.take_named("truncated shard")?);
    let config = Config { seed, ticks };
    let input_count = usize::try_from(u32::from_le_bytes(reader.take_named("truncated shard")?))
        .map_err(|_| Error::Invalid("shard input count is past the word range"))?;
    if input_count > MAX_INPUTS {
        return Err(Error::Invalid("shard carries too many inputs"));
    }
    let mut inputs = Vec::with_capacity(input_count);
    for _ in 0..input_count {
        inputs.push(decode_input(reader)?);
    }
    Ok((config, inputs))
}

/// Decode sampled rows with their action lists and optional captures.
fn decode_rows(reader: &mut Reader<'_>) -> Result<Vec<ShardRow>, Error> {
    let row_count = usize::try_from(u32::from_le_bytes(reader.take_named("truncated shard")?))
        .map_err(|_| Error::Invalid("shard row count is past the word range"))?;
    if row_count == 0 || row_count > MAX_ROWS {
        return Err(Error::Invalid("shard row count is outside bounds"));
    }
    let mut rows = Vec::with_capacity(row_count);
    for _ in 0..row_count {
        let tick = u32::from_le_bytes(reader.take_named("truncated shard")?);
        let action_count =
            usize::try_from(u32::from_le_bytes(reader.take_named("truncated shard")?))
                .map_err(|_| Error::Invalid("shard action count is past the word range"))?;
        if action_count > MAX_INPUTS {
            return Err(Error::Invalid("shard row carries too many actions"));
        }
        let mut actions = Vec::with_capacity(action_count);
        for _ in 0..action_count {
            actions.push(decode_input(reader)?);
        }
        let observation = SystemSplit {
            world: u64::from_le_bytes(reader.take_named("truncated shard")?),
            space: u64::from_le_bytes(reader.take_named("truncated shard")?),
            clock: u64::from_le_bytes(reader.take_named("truncated shard")?),
        };
        let checkpoint = Checkpoint {
            legacy: u64::from_le_bytes(reader.take_named("truncated shard")?),
            state: u64::from_le_bytes(reader.take_named("truncated shard")?),
        };
        let capture_len =
            usize::try_from(u32::from_le_bytes(reader.take_named("truncated shard")?))
                .map_err(|_| Error::Invalid("shard capture length is past the word range"))?;
        // Wire length is one past the blob length (zero means None).
        let capture = match capture_len {
            0 => None,
            len => Some(reader.take_bytes(len - 1)?.to_vec()),
        };
        rows.push(ShardRow {
            tick,
            actions,
            observation,
            checkpoint,
            capture,
        });
    }
    Ok(rows)
}

/// Decode the trailing provider-agnostic receipt. Pure wire parsing.
fn decode_manifest(reader: &mut Reader<'_>) -> Result<ShardManifest, Error> {
    Ok(ShardManifest {
        source: reader.take_text(MAX_SHARD_SOURCE_CHARS)?,
        license: reader.take_text(MAX_SHARD_LICENSE_CHARS)?,
        digest: u64::from_le_bytes(reader.take_named("truncated shard")?),
        trains: reader.take_text(MAX_SHARD_PURPOSE_CHARS)?,
        rows: usize::try_from(u32::from_le_bytes(reader.take_named("truncated shard")?))
            .map_err(|_| Error::Invalid("shard row count is past the word range"))?,
        tape_legacy: u64::from_le_bytes(reader.take_named("truncated shard")?),
        tape_state: u64::from_le_bytes(reader.take_named("truncated shard")?),
    })
}

impl Shard {
    /// Canonical content bytes for the manifest digest. Fails on counts that
    /// do not fit the u32 wire format.
    pub fn content_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut bytes = Vec::new();
        append_content_parts(self.config, &self.inputs, &self.rows, &mut bytes)?;
        Ok(bytes)
    }

    /// Encode the shard: magic, version, content, then the manifest. Fails on
    /// any shape [`check_shapes`] refuses.
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        check_shapes(self)?;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&SHARD_VERSION.to_le_bytes());
        append_content_parts(self.config, &self.inputs, &self.rows, &mut bytes)?;
        append_text(&mut bytes, &self.manifest.source)?;
        append_text(&mut bytes, &self.manifest.license)?;
        bytes.extend_from_slice(&self.manifest.digest.to_le_bytes());
        append_text(&mut bytes, &self.manifest.trains)?;
        let rows = u32::try_from(self.manifest.rows)
            .map_err(|_| Error::Invalid("shard row count is past the u32 range"))?;
        bytes.extend_from_slice(&rows.to_le_bytes());
        bytes.extend_from_slice(&self.manifest.tape_legacy.to_le_bytes());
        bytes.extend_from_slice(&self.manifest.tape_state.to_le_bytes());
        Ok(bytes)
    }

    /// Decode without trusting sizes, text, versions or trailing bytes.
    /// Recomputes the content digest, so any edited row fails here before
    /// the costlier replay check in [`verify_shard`].
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_SHARD_BYTES {
            return Err(Error::Invalid("shard encoding is past the byte cap"));
        }
        let mut reader = Reader { remaining: bytes };
        let (config, inputs) = decode_head(&mut reader)?;
        let rows = decode_rows(&mut reader)?;
        let manifest = decode_manifest(&mut reader)?;
        if !reader.remaining.is_empty() {
            return Err(Error::Invalid("shard has trailing bytes"));
        }
        let shard = Self {
            config,
            inputs,
            rows,
            manifest,
        };
        check_shapes(&shard)?;
        let digest = hash_bytes(&shard.content_bytes()?);
        if digest != shard.manifest.digest {
            return Err(Error::Invalid("shard digest does not match its rows"));
        }
        Ok(shard)
    }

    /// Staple one optional capture blob per row after export (for example a
    /// `play.step` PNG per sampled tick) and re-stamp the manifest digest, so
    /// the captures join the same content hash the replay verifies. Every
    /// blob keeps the per-blob and per-shard caps from [`check_shapes`]; a
    /// count mismatch, an oversized blob, or a zero re-stamp writes nothing
    /// and the shard keeps its old digest. Copies each blob once.
    pub fn attach_captures(&mut self, captures: Vec<Option<Vec<u8>>>) -> Result<(), Error> {
        if captures.len() != self.rows.len() {
            return Err(Error::Invalid("shard capture count does not match rows"));
        }
        let mut total = 0usize;
        for blob in captures.iter().flatten() {
            if blob.len() > MAX_CAPTURE_BYTES {
                return Err(Error::Invalid("shard capture blob is too large"));
            }
            total = total
                .checked_add(blob.len())
                .filter(|sum| *sum <= MAX_CAPTURE_TOTAL_BYTES)
                .ok_or(Error::Invalid("shard capture total is too large"))?;
        }
        let mut rows = self.rows.clone();
        for (row, capture) in rows.iter_mut().zip(captures) {
            row.capture = capture;
        }
        let mut probe = Vec::new();
        append_content_parts(self.config, &self.inputs, &rows, &mut probe)?;
        let digest = hash_bytes(&probe);
        if digest == 0 {
            return Err(Error::Invalid("shard digest is zero (unstamped)"));
        }
        self.rows = rows;
        self.manifest.digest = digest;
        Ok(())
    }
}

/// Shard-specific takes over the shared proof wire [`Reader`]. The fixed
/// array take lives once in the tape codec (called below with the shard
/// truncation label); only the capture-blob and text takes are shard-shaped.
impl Reader<'_> {
    fn take_bytes(&mut self, len: usize) -> Result<&[u8], Error> {
        if len > MAX_CAPTURE_BYTES {
            return Err(Error::Invalid("shard capture blob is too large"));
        }
        let bytes = self
            .remaining
            .get(..len)
            .ok_or(Error::Invalid("truncated shard"))?;
        self.remaining = &self.remaining[len..];
        Ok(bytes)
    }

    fn take_text(&mut self, max_chars: usize) -> Result<String, Error> {
        let len = usize::try_from(u32::from_le_bytes(self.take_named("truncated shard")?))
            .map_err(|_| Error::Invalid("shard text length is past the word range"))?;
        if len > max_chars * 4 {
            return Err(Error::Invalid("shard text field is too long"));
        }
        let bytes = self
            .remaining
            .get(..len)
            .ok_or(Error::Invalid("truncated shard"))?;
        self.remaining = &self.remaining[len..];
        let text = String::from_utf8(bytes.to_vec())
            .map_err(|_| Error::Invalid("shard text is not UTF-8"))?;
        if text.chars().count() > max_chars {
            return Err(Error::Invalid("shard text field is too long"));
        }
        Ok(text)
    }
}

/// Export one action-conditioned shard from any replay: record the tape,
/// sample ticks `0, stride, 2 * stride, ...` plus the final tick, and stamp
/// the manifest with the content digest and the final tape hashes under the
/// given explicit `license`. The simulation functions stay frozen; this only
/// reads the replay. Cost scales as rows times ticks (see the module docs).
pub fn export_shard(
    config: Config,
    inputs: &[Input],
    source: &str,
    license: &str,
    trains: &str,
    stride: u32,
) -> Result<Shard, Error> {
    if stride == 0 {
        return Err(Error::Invalid("shard stride must be nonzero"));
    }
    let tape = record(config, inputs)?;
    // `record` already refused `ticks == 0`, so `total - 1` cannot underflow.
    let total = u64::from(config.ticks);
    let step = u64::from(stride);
    let mut marks: Vec<u32> = Vec::new();
    let mut tick = 0u64;
    while tick < total {
        marks.push(
            u32::try_from(tick).map_err(|_| Error::Invalid("shard tick is past the u32 range"))?,
        );
        tick += step;
    }
    let last =
        u32::try_from(total - 1).map_err(|_| Error::Invalid("shard tick is past the u32 range"))?;
    if marks.last() != Some(&last) {
        marks.push(last);
    }
    if marks.len() > MAX_ROWS {
        return Err(Error::Invalid("shard needs a wider stride"));
    }
    let mut rows = Vec::with_capacity(marks.len());
    for mark in marks {
        // Row `mark` is the post-step state after tick `mark`, which the
        // split reaches after `mark + 1` steps; `mark` stays far below the
        // u32 range because tapes cap ticks at `MAX_TICKS`.
        let observation = system_split_at(config, inputs, mark + 1)
            .ok_or(Error::Invalid("shard observation replay failed"))?;
        let checkpoint = tape.checkpoints[usize::try_from(mark)
            .map_err(|_| Error::Invalid("shard tick is past the word range"))?];
        let actions = inputs
            .iter()
            .filter(|input| input.tick == mark)
            .copied()
            .collect();
        rows.push(ShardRow {
            tick: mark,
            actions,
            observation,
            checkpoint,
            capture: None,
        });
    }
    let last_checkpoint = *tape
        .checkpoints
        .last()
        .ok_or(Error::Invalid("tape has no checkpoints"))?;
    let mut content = Vec::new();
    append_content_parts(config, inputs, &rows, &mut content)?;
    let digest = hash_bytes(&content);
    if digest == 0 {
        return Err(Error::Invalid("shard digest is zero (unstamped)"));
    }
    let manifest = ShardManifest::new(
        source,
        license,
        digest,
        trains,
        rows.len(),
        last_checkpoint.legacy,
        last_checkpoint.state,
    )?;
    Ok(Shard {
        config,
        inputs: inputs.to_vec(),
        rows,
        manifest,
    })
}

/// Re-verify a shard by hash twice: the manifest digest over its rows, two
/// fresh replays against every sampled checkpoint and observation, the
/// actions against the tape inputs, and the final tape hashes against the
/// manifest binding. The first mismatch names its tick.
pub fn verify_shard(shard: &Shard) -> Result<(), Error> {
    check_shapes(shard)?;
    let digest = hash_bytes(&shard.content_bytes()?);
    if digest != shard.manifest.digest {
        return Err(Error::Invalid("shard digest does not match its rows"));
    }
    let first = record(shard.config, &shard.inputs)?;
    let second = record(shard.config, &shard.inputs)?;
    if first.checkpoints.len() != second.checkpoints.len() {
        return Err(Error::Invalid("shard replays disagree in length"));
    }
    for (index, (a, b)) in first
        .checkpoints
        .iter()
        .zip(second.checkpoints.iter())
        .enumerate()
    {
        if a != b {
            let tick = u32::try_from(index)
                .map_err(|_| Error::Invalid("shard tick is past the u32 range"))?;
            return Err(Error::Divergence {
                tick,
                system: "dataset",
                expected: *b,
                actual: *a,
            });
        }
    }
    for row in &shard.rows {
        // Decode and encode already bound `tick` inside the tape.
        let replayed = first.checkpoints[usize::try_from(row.tick)
            .map_err(|_| Error::Invalid("shard tick is past the word range"))?];
        if replayed != row.checkpoint {
            return Err(Error::Divergence {
                tick: row.tick,
                system: "dataset",
                expected: row.checkpoint,
                actual: replayed,
            });
        }
        // The observation folds the same canonical bytes as the state hash,
        // so equality there implies equality here; the recheck still guards
        // a hand-built shard that paired a foreign observation with intact
        // checkpoints.
        let observed = system_split_at(shard.config, &shard.inputs, row.tick + 1)
            .ok_or(Error::Invalid("shard observation replay failed"))?;
        if observed != row.observation {
            return Err(Error::Divergence {
                tick: row.tick,
                system: "dataset",
                expected: row.checkpoint,
                actual: replayed,
            });
        }
        let wanted: Vec<Input> = shard
            .inputs
            .iter()
            .filter(|input| input.tick == row.tick)
            .copied()
            .collect();
        if wanted != row.actions {
            return Err(Error::Invalid("shard actions do not match the tape inputs"));
        }
    }
    let last = first
        .checkpoints
        .last()
        .ok_or(Error::Invalid("tape has no checkpoints"))?;
    if last.legacy != shard.manifest.tape_legacy || last.state != shard.manifest.tape_state {
        let tick = shard
            .rows
            .last()
            .map(|row| row.tick)
            .ok_or(Error::Invalid("shard has no rows"))?;
        return Err(Error::Divergence {
            tick,
            system: "dataset",
            expected: Checkpoint {
                legacy: shard.manifest.tape_legacy,
                state: shard.manifest.tape_state,
            },
            actual: *last,
        });
    }
    Ok(())
}

/// One shard's binding inside an hour-scale receipt: the content digest a
/// training harness re-hashes before touching the rows, plus the final tape
/// hashes tying the shard to its replay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatasetEntry {
    /// Collector id copied from the shard manifest.
    pub source: String,
    /// Content digest copied from the shard manifest.
    pub digest: u64,
    /// Row count copied from the shard manifest.
    pub rows: usize,
    /// Final tape legacy hash copied from the shard manifest.
    pub tape_legacy: u64,
    /// Final tape state hash copied from the shard manifest.
    pub tape_state: u64,
}

/// Provider-agnostic receipt over a full export run (for example one hour
/// of arena tapes): one entry per shard plus the totals a training harness
/// needs before downloading any bytes. The run digest is the engine hash
/// over the entries in order, so a dropped or edited shard breaks it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatasetReceipt {
    /// Collector id for the whole run (tape range or season name).
    pub source: String,
    /// License every shard ships under; must be explicit, never empty.
    pub license: String,
    /// What these rows may train (policy family or study name).
    pub trains: String,
    /// One entry per shard, in export order.
    pub entries: Vec<DatasetEntry>,
    /// Total rows across all shards.
    pub rows: usize,
    /// Total tape ticks across all shards (at the fixed tick rate this is
    /// the run length in time).
    pub ticks: u64,
    /// Engine hash over the entries; zero is never stamped.
    pub digest: u64,
}

impl fmt::Display for DatasetReceipt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "dataset {} {} shards {} rows {} ticks {} digest {:016x} for {}",
            self.source,
            self.license,
            self.entries.len(),
            self.rows,
            self.ticks,
            self.digest,
            self.trains
        )
    }
}

impl DatasetReceipt {
    /// Canonical bytes for the run digest: the run fields plus every entry
    /// in export order. Fails only on counts past the u32 wire range.
    pub fn content_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut bytes = Vec::new();
        append_text(&mut bytes, &self.source)?;
        append_text(&mut bytes, &self.license)?;
        append_text(&mut bytes, &self.trains)?;
        let shards = u32::try_from(self.entries.len())
            .map_err(|_| Error::Invalid("dataset shard count is past the u32 range"))?;
        bytes.extend_from_slice(&shards.to_le_bytes());
        for entry in &self.entries {
            append_text(&mut bytes, &entry.source)?;
            bytes.extend_from_slice(&entry.digest.to_le_bytes());
            let rows = u32::try_from(entry.rows)
                .map_err(|_| Error::Invalid("dataset row count is past the u32 range"))?;
            bytes.extend_from_slice(&rows.to_le_bytes());
            bytes.extend_from_slice(&entry.tape_legacy.to_le_bytes());
            bytes.extend_from_slice(&entry.tape_state.to_le_bytes());
        }
        Ok(bytes)
    }
}

/// Re-verify a full export run by hash: every shard through [`verify_shard`]
/// (two fresh replays each), then fold the shard manifests into one receipt
/// stamped with the run digest. The first bad shard names its tick; an
/// empty run or a blank license is refused before any replay. Pure apart
/// from the replays, which never call a model.
pub fn verify_dataset(
    source: &str,
    license: &str,
    trains: &str,
    shards: &[Shard],
) -> Result<DatasetReceipt, Error> {
    if shards.is_empty() {
        return Err(Error::Invalid("dataset has no shards"));
    }
    if source.is_empty() || source.chars().count() > MAX_SHARD_SOURCE_CHARS {
        return Err(Error::Invalid("dataset source id is empty or too long"));
    }
    if license.is_empty() || license.chars().count() > MAX_SHARD_LICENSE_CHARS {
        return Err(Error::Invalid("dataset license is empty or too long"));
    }
    if trains.is_empty() || trains.chars().count() > MAX_SHARD_PURPOSE_CHARS {
        return Err(Error::Invalid("dataset purpose is empty or too long"));
    }
    let mut entries = Vec::with_capacity(shards.len());
    let mut rows = 0usize;
    let mut ticks = 0u64;
    for shard in shards {
        verify_shard(shard)?;
        rows = rows
            .checked_add(shard.rows.len())
            .ok_or(Error::Invalid("dataset row total is past the word range"))?;
        ticks = ticks
            .checked_add(u64::from(shard.config.ticks))
            .ok_or(Error::Invalid("dataset tick total is past the u64 range"))?;
        entries.push(DatasetEntry {
            source: shard.manifest.source.clone(),
            digest: shard.manifest.digest,
            rows: shard.rows.len(),
            tape_legacy: shard.manifest.tape_legacy,
            tape_state: shard.manifest.tape_state,
        });
    }
    let mut receipt = DatasetReceipt {
        source: source.to_string(),
        license: license.to_string(),
        trains: trains.to_string(),
        entries,
        rows,
        ticks,
        digest: 0,
    };
    let digest = hash_bytes(&receipt.content_bytes()?);
    if digest == 0 {
        return Err(Error::Invalid("dataset digest is zero (unstamped)"));
    }
    receipt.digest = digest;
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_inputs() -> [Input; 2] {
        [
            Input {
                tick: 5,
                body: 0,
                vx: -20.0,
                vy: 8.0,
            },
            Input {
                tick: 20,
                body: 1,
                vx: 2.0,
                vy: 6.0,
            },
        ]
    }

    fn fixture_shard() -> Shard {
        export_shard(
            Config { seed: 7, ticks: 24 },
            &fixture_inputs(),
            "arena-tapes",
            "CC0-1.0",
            "melee-policy",
            5,
        )
        .expect("fixture export fits its bounds")
    }

    #[test]
    fn shard_exports_rows_and_reverifies_twice() {
        let shard = fixture_shard();
        assert_eq!(shard.rows.first().expect("nonempty shard").tick, 0);
        assert_eq!(shard.rows.last().expect("nonempty shard").tick, 23);
        assert!(!shard.rows.is_empty());
        verify_shard(&shard).expect("fresh shard verifies");
        verify_shard(&shard).expect("shard verifies twice");
        let receipt = shard.manifest.to_string();
        assert!(receipt.contains("arena-tapes"), "{receipt}");
        assert!(receipt.contains("CC0-1.0"), "{receipt}");
    }

    #[test]
    fn actions_land_on_their_tick() {
        let shard = export_shard(
            Config { seed: 7, ticks: 24 },
            &fixture_inputs(),
            "arena-tapes",
            "CC0-1.0",
            "melee-policy",
            1,
        )
        .expect("stride-one export fits");
        assert_eq!(shard.rows.len(), 24);
        assert!(shard.rows[4].actions.is_empty());
        assert_eq!(shard.rows[5].actions, vec![fixture_inputs()[0]]);
        assert_eq!(shard.rows[20].actions, vec![fixture_inputs()[1]]);
    }

    #[test]
    fn stride_covers_zero_and_the_final_tick() {
        let shard = export_shard(
            Config {
                seed: 7,
                ticks: 100,
            },
            &[],
            "arena-tapes",
            "CC0-1.0",
            "melee-policy",
            30,
        )
        .expect("strided export fits");
        let ticks: Vec<u32> = shard.rows.iter().map(|row| row.tick).collect();
        assert_eq!(ticks, vec![0, 30, 60, 90, 99]);
    }

    #[test]
    fn row_cap_refuses_a_narrow_stride_before_any_observation() {
        let wide = export_shard(
            Config {
                seed: 7,
                ticks: 100,
            },
            &[],
            "arena-tapes",
            "CC0-1.0",
            "melee-policy",
            1,
        )
        .expect("100 rows fit the cap");
        assert_eq!(wide.rows.len(), 100);
        assert!(matches!(
            export_shard(
                Config {
                    seed: 7,
                    ticks: 5_000,
                },
                &[],
                "arena-tapes",
                "CC0-1.0",
                "melee-policy",
                1,
            ),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn manifest_shape_and_zero_stride_are_refused() {
        let config = Config { seed: 7, ticks: 24 };
        for (source, license, trains, stride) in [
            ("", "CC0-1.0", "melee-policy", 5),
            ("arena-tapes", "", "melee-policy", 5),
            ("arena-tapes", "CC0-1.0", "", 5),
            ("arena-tapes", "CC0-1.0", "melee-policy", 0),
        ] {
            assert!(
                export_shard(config, &fixture_inputs(), source, license, trains, stride).is_err(),
                "source {source:?} license {license:?} trains {trains:?} stride {stride}"
            );
        }
    }

    #[test]
    fn encoding_roundtrips_byte_exact_and_reverifies() {
        let shard = fixture_shard();
        let bytes = shard.encode().expect("shard encodes");
        assert_eq!(
            bytes,
            shard.encode().expect("encoding is deterministic"),
            "same shard gives identical bytes"
        );
        let decoded = Shard::decode(&bytes).expect("shard decodes");
        assert_eq!(shard, decoded);
        verify_shard(&decoded).expect("decoded shard verifies");
    }

    #[test]
    fn tampered_rows_fail_verify_or_decode() {
        // Flipped checkpoint with a re-stamped digest: the digest gate passes
        // and the replay names the row's tick.
        let mut shard = fixture_shard();
        let tick = shard.rows[2].tick;
        shard.rows[2].checkpoint.state ^= 1;
        shard.manifest.digest =
            hash_bytes(&shard.content_bytes().expect("re-stamp reads valid shapes"));
        let error = verify_shard(&shard).expect_err("tampered row must diverge");
        assert!(
            matches!(
                error,
                Error::Divergence {
                    tick: got,
                    system: "dataset",
                    ..
                } if got == tick
            ),
            "got {error:?}"
        );
        // Flipped content byte: the digest check fails at decode.
        let mut bytes = fixture_shard().encode().expect("shard encodes");
        bytes[20] ^= 0xff;
        assert!(
            matches!(Shard::decode(&bytes), Err(Error::Invalid(_))),
            "edited rows must break the digest"
        );
        // Flipped manifest binding: decodes (digest covers rows only) but the
        // final tape hashes no longer match, so verify refuses.
        let mut rebound = fixture_shard();
        rebound.manifest.tape_state ^= 1;
        assert!(verify_shard(&rebound).is_err());
    }

    #[test]
    fn truncated_and_trailing_bytes_are_rejected() {
        let bytes = fixture_shard().encode().expect("shard encodes");
        for end in [0, 8, 10, 24, bytes.len() / 2, bytes.len() - 1] {
            assert!(Shard::decode(&bytes[..end]).is_err(), "length {end}");
        }
        let mut tailed = bytes.clone();
        tailed.push(0);
        assert!(Shard::decode(&tailed).is_err());
    }

    #[test]
    fn attached_captures_restamp_and_stay_bound_to_the_digest() {
        let mut shard = fixture_shard();
        let before = shard.manifest.digest;
        let rows = shard.rows.len();
        let mut captures: Vec<Option<Vec<u8>>> = (0..rows).map(|_| None).collect();
        captures[1] = Some(vec![1, 2, 3]);
        captures[4] = Some(vec![0; 8]);
        shard
            .attach_captures(captures)
            .expect("captures fit their caps");
        assert_ne!(shard.manifest.digest, before, "captures move the digest");
        assert_eq!(shard.rows[1].capture, Some(vec![1, 2, 3]));
        assert_eq!(shard.rows[4].capture, Some(vec![0; 8]));
        let bytes = shard.encode().expect("shard with captures encodes");
        let decoded = Shard::decode(&bytes).expect("shard with captures decodes");
        assert_eq!(shard, decoded);
        verify_shard(&decoded).expect("shard with captures verifies");
        // A capture edited without a re-stamp breaks the digest gate.
        let mut edited = decoded.clone();
        edited.rows[1].capture = Some(vec![9, 9, 9]);
        assert!(verify_shard(&edited).is_err());
    }

    #[test]
    fn mismatched_and_oversized_captures_leave_the_shard_untouched() {
        let mut shard = fixture_shard();
        let before = shard.clone();
        assert!(
            shard.attach_captures(vec![None]).is_err(),
            "capture count must match the row count"
        );
        assert_eq!(shard, before, "count mismatch writes nothing");
        let rows = shard.rows.len();
        let mut too_big: Vec<Option<Vec<u8>>> = (0..rows).map(|_| None).collect();
        too_big[0] = Some(vec![0; MAX_CAPTURE_BYTES + 1]);
        assert!(
            shard.attach_captures(too_big).is_err(),
            "an oversized blob must fail"
        );
        assert_eq!(shard, before, "oversized blob writes nothing");
    }

    #[test]
    fn hour_of_tapes_exports_and_reverifies_twice() {
        // One hour at the 60 Hz fixed tick is 216,000 ticks, past the
        // single-tape cap, so three duel tapes cover the hour. Each shard
        // exports with a wide stride, then re-verifies by hash twice.
        const TICKS_PER_TAPE: u32 = 72_000;
        const STRIDE: u32 = 36_000;
        assert_eq!(
            3 * u64::from(TICKS_PER_TAPE),
            216_000,
            "three tapes cover one hour at 60 Hz"
        );
        let mut total = 0u64;
        for seed in [7u64, 19, 2040] {
            // Two bodies trade velocity assignments in (tick, body) order,
            // the same shape the arena duel records.
            let inputs = [
                Input {
                    tick: 1_000,
                    body: 0,
                    vx: -20.0,
                    vy: 8.0,
                },
                Input {
                    tick: 24_000,
                    body: 1,
                    vx: 2.0,
                    vy: 6.0,
                },
                Input {
                    tick: 48_000,
                    body: 0,
                    vx: 9.0,
                    vy: -4.0,
                },
                Input {
                    tick: 71_000,
                    body: 1,
                    vx: -6.0,
                    vy: 3.0,
                },
            ];
            let config = Config {
                seed,
                ticks: TICKS_PER_TAPE,
            };
            let shard = export_shard(
                config,
                &inputs,
                "arena-hour",
                "CC0-1.0",
                "melee-policy",
                STRIDE,
            )
            .expect("wide-stride hour tape fits its bounds");
            // Wide stride keeps the shard small: genesis, mid-tape, final.
            let ticks: Vec<u32> = shard.rows.iter().map(|row| row.tick).collect();
            assert_eq!(ticks, vec![0, 36_000, TICKS_PER_TAPE - 1]);
            // Provider-agnostic manifest: source, license, digest, purpose
            // and both final tape hashes travel with the rows.
            assert_eq!(shard.manifest.rows, shard.rows.len());
            assert_ne!(shard.manifest.digest, 0);
            let receipt = shard.manifest.to_string();
            assert!(receipt.contains("arena-hour"), "{receipt}");
            assert!(receipt.contains("CC0-1.0"), "{receipt}");
            // Bytes out and back, then the replay hash check twice.
            let decoded = Shard::decode(&shard.encode().expect("hour shard encodes"))
                .expect("hour shard decodes");
            assert_eq!(shard, decoded);
            verify_shard(&decoded).expect("hour shard verifies");
            verify_shard(&decoded).expect("hour shard verifies twice");
            total += u64::from(config.ticks);
        }
        assert_eq!(total, 216_000, "exported tapes sum to one hour");
    }

    #[test]
    fn hour_run_receipt_folds_three_shards_and_reverifies_twice() {
        // Run-level close of the hour Accept clause: the same three
        // 72,000-tick duel tapes fold through `verify_dataset` into one
        // receipt with `ticks == 216_000`, stable across two verifications.
        // Each shard travels the provider JSON wire plus the byte wire
        // before the fold, so the receipt binds what any harness reads.
        const TICKS_PER_TAPE: u32 = 72_000;
        const STRIDE: u32 = 36_000;
        assert_eq!(
            3 * u64::from(TICKS_PER_TAPE),
            216_000,
            "three tapes cover one hour at 60 Hz"
        );
        let mut shards = Vec::with_capacity(3);
        for (index, seed) in [7u64, 19, 2040].iter().enumerate() {
            // Two bodies trade velocity assignments in (tick, body) order,
            // the same shape the arena duel records.
            let inputs = [
                Input {
                    tick: 1_000,
                    body: 0,
                    vx: -20.0,
                    vy: 8.0,
                },
                Input {
                    tick: 24_000,
                    body: 1,
                    vx: 2.0,
                    vy: 6.0,
                },
                Input {
                    tick: 48_000,
                    body: 0,
                    vx: 9.0,
                    vy: -4.0,
                },
                Input {
                    tick: 71_000,
                    body: 1,
                    vx: -6.0,
                    vy: 3.0,
                },
            ];
            let config = Config {
                seed: *seed,
                ticks: TICKS_PER_TAPE,
            };
            let source = match index {
                0 => "arena-hour-0000",
                1 => "arena-hour-0001",
                _ => "arena-hour-0002",
            };
            let shard = export_shard(config, &inputs, source, "CC0-1.0", "melee-policy", STRIDE)
                .expect("wide-stride hour tape fits its bounds");
            // Wide stride keeps the shard small: genesis, mid-tape, final.
            let ticks: Vec<u32> = shard.rows.iter().map(|row| row.tick).collect();
            assert_eq!(ticks, vec![0, 36_000, TICKS_PER_TAPE - 1]);
            // Provider wire per shard before the fold: canonical JSON
            // roundtrips without the Rust byte codec, then bytes roundtrip.
            let text = shard.manifest.to_json();
            assert!(text.contains(source), "{text}");
            assert!(text.contains("\"license\":\"CC0-1.0\""), "{text}");
            let rebound = ShardManifest::parse_json(&text).expect("hour manifest JSON parses");
            assert_eq!(shard.manifest, rebound);
            let decoded = Shard::decode(&shard.encode().expect("hour shard encodes"))
                .expect("hour shard decodes");
            assert_eq!(shard, decoded);
            shards.push(decoded);
        }
        let receipt = verify_dataset("arena-hour-1", "CC0-1.0", "melee-policy", &shards)
            .expect("hour run verifies");
        assert_eq!(receipt.entries.len(), 3);
        assert_eq!(receipt.ticks, 216_000);
        assert_ne!(receipt.digest, 0);
        let rows: usize = shards.iter().map(|shard| shard.rows.len()).sum();
        assert_eq!(receipt.rows, rows);
        for (entry, shard) in receipt.entries.iter().zip(shards.iter()) {
            assert_eq!(entry.source, shard.manifest.source);
            assert_eq!(entry.digest, shard.manifest.digest);
            assert_eq!(entry.rows, shard.rows.len());
            assert_eq!(entry.tape_legacy, shard.manifest.tape_legacy);
            assert_eq!(entry.tape_state, shard.manifest.tape_state);
        }
        let line = receipt.to_string();
        assert!(line.contains("arena-hour-1"), "{line}");
        assert!(line.contains("CC0-1.0"), "{line}");
        let again = verify_dataset("arena-hour-1", "CC0-1.0", "melee-policy", &shards)
            .expect("hour run re-verifies twice");
        assert_eq!(receipt, again);
    }

    fn two_tape_run() -> [Shard; 2] {
        let first = export_shard(
            Config { seed: 7, ticks: 24 },
            &fixture_inputs(),
            "arena-tapes-0000",
            "CC0-1.0",
            "melee-policy",
            5,
        )
        .expect("first tape exports");
        let second = export_shard(
            Config { seed: 8, ticks: 24 },
            &fixture_inputs(),
            "arena-tapes-0001",
            "CC0-1.0",
            "melee-policy",
            5,
        )
        .expect("second tape exports");
        [first, second]
    }

    #[test]
    fn dataset_receipt_folds_two_tapes_and_reverifies() {
        let shards = two_tape_run();
        let receipt = verify_dataset("arena-hour-1", "CC0-1.0", "melee-policy", &shards)
            .expect("two fresh shards verify");
        assert_eq!(receipt.entries.len(), 2);
        assert_eq!(receipt.rows, shards[0].rows.len() + shards[1].rows.len());
        assert_eq!(receipt.ticks, 48);
        assert_ne!(receipt.digest, 0);
        assert_eq!(receipt.entries[0].digest, shards[0].manifest.digest);
        assert_eq!(receipt.entries[1].tape_state, shards[1].manifest.tape_state);
        let again = verify_dataset("arena-hour-1", "CC0-1.0", "melee-policy", &shards)
            .expect("run re-verifies twice");
        assert_eq!(receipt, again);
        let line = receipt.to_string();
        assert!(line.contains("arena-hour-1"), "{line}");
        assert!(line.contains("CC0-1.0"), "{line}");
        let rebuilt = DatasetReceipt {
            digest: 0,
            ..receipt.clone()
        };
        let stamp = hash_bytes(&rebuilt.content_bytes().expect("receipt encodes"));
        assert_eq!(stamp, receipt.digest);
    }

    #[test]
    fn dataset_receipt_refuses_empty_runs_and_a_tampered_shard() {
        let shards = two_tape_run();
        assert!(verify_dataset("arena-hour-1", "", "melee-policy", &shards).is_err());
        assert!(verify_dataset("arena-hour-1", "CC0-1.0", "melee-policy", &[]).is_err());
        let mut tampered = shards;
        let tick = tampered[1].rows[2].tick;
        tampered[1].rows[2].checkpoint.state ^= 1;
        tampered[1].manifest.digest = hash_bytes(
            &tampered[1]
                .content_bytes()
                .expect("re-stamp reads valid shapes"),
        );
        let error = verify_dataset("arena-hour-1", "CC0-1.0", "melee-policy", &tampered)
            .expect_err("tampered run must diverge");
        assert!(
            matches!(
                error,
                Error::Divergence {
                    tick: got,
                    system: "dataset",
                    ..
                } if got == tick
            ),
            "got {error:?}"
        );
    }

    #[test]
    fn manifest_json_roundtrips_and_reverifies_twice() {
        // Provider-agnostic receipt: any harness reads this JSON without the
        // Rust wire codec, then the shard re-verifies by hash twice.
        let shard = fixture_shard();
        let text = shard.manifest.to_json();
        assert!(
            text.contains("\"format\":\"e2040-dataset-shard/1\""),
            "{text}"
        );
        assert!(text.contains("\"license\":\"CC0-1.0\""), "{text}");
        assert_eq!(
            text,
            shard.manifest.to_json(),
            "manifest JSON is deterministic"
        );
        let back = ShardManifest::parse_json(&text).expect("canonical JSON parses");
        assert_eq!(shard.manifest, back);
        verify_shard(&shard).expect("shard verifies");
        verify_shard(&shard).expect("shard verifies twice");
        // A flipped digest hex still parses but no longer binds the rows.
        let key = "\"digest\":\"";
        let at = text.find(key).expect("digest field travels in the JSON") + key.len();
        let flip = if text.as_bytes()[at] == b'0' {
            '1'
        } else {
            '0'
        };
        let mut edited = text.clone();
        edited.replace_range(at..at + 1, &flip.to_string());
        let rebound = ShardManifest::parse_json(&edited).expect("flipped hex still parses");
        assert_ne!(rebound.digest, shard.manifest.digest);
        let rebound_shard = Shard {
            manifest: rebound,
            ..shard.clone()
        };
        assert!(verify_shard(&rebound_shard).is_err());
    }

    #[test]
    fn manifest_json_is_the_only_wire_format_and_refuses_unlicensed() {
        // Training jobs read the JSON receipt alone: no byte codec appears
        // in this path, yet the manifest roundtrips and still binds rows.
        let shard = fixture_shard();
        let text = shard.manifest.to_json();
        assert!(
            text.starts_with("{\"format\":\"e2040-dataset-shard/1\",\"source\":\""),
            "{text}"
        );
        assert!(text.contains("\"license\":\"CC0-1.0\""), "{text}");
        assert_eq!(
            text,
            shard.manifest.to_json(),
            "manifest JSON is deterministic"
        );
        let back = ShardManifest::parse_json(&text).expect("canonical JSON parses");
        assert_eq!(shard.manifest, back);
        let wired = Shard {
            manifest: back,
            ..shard.clone()
        };
        verify_shard(&wired).expect("JSON wired shard verifies");
        verify_shard(&wired).expect("JSON wired shard verifies twice");
        // Unlicensed shapes never stamp through the constructor.
        let good = &shard.manifest;
        assert!(
            ShardManifest::new(
                &good.source,
                "",
                good.digest,
                &good.trains,
                good.rows,
                good.tape_legacy,
                good.tape_state
            )
            .is_err(),
            "empty license is refused"
        );
        assert!(
            ShardManifest::new(
                &good.source,
                &good.license,
                good.digest,
                "",
                good.rows,
                good.tape_legacy,
                good.tape_state
            )
            .is_err(),
            "empty trains purpose is refused"
        );
        assert!(
            ShardManifest::new(
                &good.source,
                &good.license,
                0,
                &good.trains,
                good.rows,
                good.tape_legacy,
                good.tape_state
            )
            .is_err(),
            "zero digest is refused"
        );
        assert!(
            ShardManifest::new(
                &"s".repeat(MAX_SHARD_SOURCE_CHARS + 1),
                &good.license,
                good.digest,
                &good.trains,
                good.rows,
                good.tape_legacy,
                good.tape_state
            )
            .is_err(),
            "over-long source is refused"
        );
        assert!(
            ShardManifest::new(
                &good.source,
                &"l".repeat(MAX_SHARD_LICENSE_CHARS + 1),
                good.digest,
                &good.trains,
                good.rows,
                good.tape_legacy,
                good.tape_state
            )
            .is_err(),
            "over-long license is refused"
        );
        assert!(
            ShardManifest::new(
                &good.source,
                &good.license,
                good.digest,
                &"t".repeat(MAX_SHARD_PURPOSE_CHARS + 1),
                good.rows,
                good.tape_legacy,
                good.tape_state
            )
            .is_err(),
            "over-long trains purpose is refused"
        );
        // The same refusals hold through the JSON wire.
        assert!(
            ShardManifest::parse_json(&text.replace("\"license\":\"CC0-1.0\"", "\"license\":\"\""))
                .is_err(),
            "empty license JSON is refused"
        );
        assert!(
            ShardManifest::parse_json(
                &text.replace("\"trains\":\"melee-policy\"", "\"trains\":\"\"")
            )
            .is_err(),
            "empty trains JSON is refused"
        );
        let key = "\"digest\":\"";
        let at = text.find(key).expect("digest field travels in the JSON") + key.len();
        let mut zeroed = text.clone();
        zeroed.replace_range(at..at + 16, "0000000000000000");
        assert!(
            ShardManifest::parse_json(&zeroed).is_err(),
            "zero digest JSON is refused"
        );
        let long_license = "l".repeat(MAX_SHARD_LICENSE_CHARS + 1);
        let over_long = text.replace("CC0-1.0", &long_license);
        assert!(
            ShardManifest::parse_json(&over_long).is_err(),
            "over-long license JSON is refused"
        );
        // A flipped digest still parses but no longer binds the rows.
        let flip = if text.as_bytes()[at] == b'0' {
            '1'
        } else {
            '0'
        };
        let mut edited = text.clone();
        edited.replace_range(at..at + 1, &flip.to_string());
        let rebound = ShardManifest::parse_json(&edited).expect("flipped hex still parses");
        assert_ne!(rebound.digest, shard.manifest.digest);
        let rebound_shard = Shard {
            manifest: rebound,
            ..shard.clone()
        };
        assert!(verify_shard(&rebound_shard).is_err());
    }

    #[test]
    fn manifest_json_rejects_garbage_and_truncation() {
        let text = fixture_shard().manifest.to_json();
        for cut in [0, 1, 5, text.len() / 2, text.len() - 1] {
            let prefix = text.get(..cut).expect("fixture JSON is ASCII");
            assert!(ShardManifest::parse_json(prefix).is_err(), "cut at {cut}");
        }
        assert!(ShardManifest::parse_json("").is_err());
        assert!(ShardManifest::parse_json("{}").is_err());
        assert!(
            ShardManifest::parse_json(&text.replace("e2040-dataset-shard/1", "other/9")).is_err(),
            "foreign format tag is refused"
        );
        let mut tailed = text.clone();
        tailed.push(' ');
        assert!(
            ShardManifest::parse_json(&tailed).is_err(),
            "trailing bytes are refused"
        );
    }
}
