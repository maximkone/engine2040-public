//! capture: offline clip capture — tape-bound PNGs plus a manifest.
//!
//! Consumer: the `arena` window binary (`--capture-dir/--capture-every/--tape`)
//! and the clip stitcher (PNGs to video/GIF). Data owner: this module owns the
//! capture flags, the tape file codec, the std-only PNG writer and the frame
//! sink. Units: fixed ticks at 60 Hz, pixels, bytes. Determinism: the same
//! config plus tape gives byte-identical PNGs and manifest twice (no wall
//! clock, no absolute paths, fixed field order). Errors: typed
//! [`CaptureError`]; bad flags or tapes fail before any frame is written.
//!
//! Budgets: one RGBA frame plus one PNG in memory at a time (about 17 MB at
//! 2048x2048); all filesystem IO runs on the present path, never the tick.
use std::path::{Path, PathBuf};

use engine_arena::seed as arena_seed;
use engine_core::adler::adler32;
use engine_core::hash::{FNV64_OFFSET, GOLDEN_GAMMA};
use engine_core::{hash_bytes, Fnv64};
use engine_input::{Buttons, FighterId, FighterInput, FighterTape};
use engine_render::present::SizeClass;

use super::director::{cutlog_digest, cutlog_json, verify_cutlog_json, Cut, Shot};
use super::{App, GameConfig, GameError, HUMAN_SLOT, MAX_TICKS};

/// Tape file envelope version; anything else refuses loudly.
pub const CAPTURE_VERSION: u16 = 1;
/// Renderer label filed in the manifest for the offscreen presenter twin:
/// frames drawn by `engine_render::present::Presenter::init_offscreen` (crates/render).
pub const TWIN_RENDERER: &str = "crates/render";
/// Renderer label filed in the manifest for the headless synth path.
pub const HEADLESS_RENDERER: &str = "headless-synth";
/// Longest accepted flag path in bytes (dirs and tape files alike).
pub const MAX_PATH_BYTES: usize = 4096;
/// Zero-padded width of `frame_{:06}.png` (100 000 ticks max, so 6 fits).
pub const FRAME_NAME_WIDTH: usize = 6;
/// Manifest file name written beside the PNGs.
pub const MANIFEST_NAME: &str = "manifest.json";
/// Cut-log file name written beside the PNGs plus the manifest: the
/// canonical [`crate::director::cutlog_json`] bytes the manifest digest binds.
pub const CUTLOG_NAME: &str = "cutlog.json";
/// Most manifest or cut-log bytes the binding check trusts before parsing.
pub const MAX_RECEIPT_BYTES: usize = 1 << 20;

/// Tape file magic (`E2040CAP`, 8 bytes), then version, count, inputs.
const TAPE_MAGIC: &[u8; 8] = b"E2040CAP";
/// One canonical [`FighterInput`] is 10 bytes (see `append_bytes`).
const INPUT_BYTES: usize = 10;
/// Tape header bytes: magic plus version plus count.
const HEADER_BYTES: usize = 14;
/// Most inputs in one tape file (one per tick is the dense ceiling).
const MAX_CAPTURE_INPUTS: usize = MAX_TICKS as usize;
/// Most accepted tape file bytes, checked before trusting the count.
const MAX_TAPE_FILE_BYTES: usize = HEADER_BYTES + MAX_CAPTURE_INPUTS * INPUT_BYTES;
/// Largest stored-deflate block in the PNG writer (zlib ceiling).
const PNG_BLOCK_MAX: usize = 65535;

/// Capture failures: bounded diagnostics, never panics.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CaptureError {
    /// CLI row rejected (unknown capture flag, missing value, `every` of 0,
    /// dir without cadence); culprit carried.
    BadArgs(String),
    /// Config row outside its range, row name carried.
    BadConfig(&'static str),
    /// Tape file malformed (magic, version, length, order, ranges); reason
    /// carried.
    BadTape(String),
    /// Frame bytes are not a PNG (twin plumbing defect); reason carried.
    BadPng(String),
    /// Cut-log binding defect (manifest digest, cut-log bytes or their
    /// receipt rows disagree); reason carried.
    BadCutlog(String),
    /// Filesystem failure; path plus kind carried (paths stay out of rows).
    Io {
        /// File the operation targeted (lossy; diagnostics only).
        path: String,
        /// IO failure kind.
        kind: std::io::ErrorKind,
    },
}

impl core::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadArgs(row) => write!(f, "bad capture arg ({row})"),
            Self::BadConfig(row) => write!(f, "bad capture config ({row})"),
            Self::BadTape(reason) => write!(f, "bad capture tape ({reason})"),
            Self::BadPng(reason) => write!(f, "bad capture frame ({reason})"),
            Self::BadCutlog(reason) => write!(f, "bad cut log ({reason})"),
            Self::Io { path, kind } => write!(f, "capture io ({path}: {kind:?})"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// Bound a diagnostic so errors stay small and printable (single owner; `windowed` re-exports it).
pub fn cap_message(msg: &str) -> String {
    const MAX: usize = 256;
    let out: String = msg.chars().take(MAX).collect();
    if out.len() < msg.len() {
        format!("{out}...")
    } else {
        out
    }
}

fn io_err(path: &Path, err: std::io::Error) -> CaptureError {
    CaptureError::Io {
        path: cap_message(&path.to_string_lossy()),
        kind: err.kind(),
    }
}

// ---------------------------------------------------------------------------
// Flags: `--capture-dir` plus `--capture-every` plus optional `--tape`.
// Parsed before `WinArgs` so unknown flags still fail in exactly one place.
// ---------------------------------------------------------------------------

/// Parsed capture CLI: a directory plus a frame cadence plus an input tape.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CaptureFlags {
    /// Directory the PNG sequence plus `manifest.json` go into.
    pub dir: PathBuf,
    /// One captured frame per this many presented frames (`>= 1`).
    pub every: u32,
    /// Playtest-agent-shape input tape driving the human slot (`None` idles).
    pub tape: Option<PathBuf>,
    /// Board rows the clip evidences (one built-in row when the flag is
    /// absent); the stitch validator requires non-empty row-ID strings.
    pub rows: Vec<String>,
    /// Build SHA hex (7-128 chars, required): carried into the manifest so
    /// the stitch validator can bind the clip to its build. No default —
    /// a capture without a build identity fails loudly.
    pub sha: String,
    /// Playback fps for the stitched clip (`1..=120`, default 30).
    pub fps: u32,
}

/// One board-row ID: 1-4 uppercase letters, 1-3 digits, optional lowercase
/// suffix. Mirrors the stitch validator's ROW_RE.
fn check_row_id(raw: &str, flag: &str) -> Result<String, CaptureError> {
    let id = raw.trim();
    let letters = id.chars().take_while(|c| c.is_ascii_uppercase()).count();
    let rest = &id[letters..];
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    let tail = &rest[digits..];
    let ok = (1..=4).contains(&letters)
        && (1..=3).contains(&digits)
        && (tail.is_empty()
            || (tail.len() == 1 && tail.chars().next().is_some_and(|c| c.is_ascii_lowercase())))
        && id.len() == letters + digits + tail.len();
    if !ok {
        return Err(CaptureError::BadArgs(format!(
            "{flag} has a bad board row ID: {raw}"
        )));
    }
    Ok(id.to_string())
}

/// Build SHA hex for the manifest: 7-128 hex chars, no `0x`, no whitespace.
fn check_sha(raw: &str, flag: &str) -> Result<String, CaptureError> {
    let sha = raw.trim();
    let ok = (7..=128).contains(&sha.len()) && sha.chars().all(|c| c.is_ascii_hexdigit());
    if !ok {
        return Err(CaptureError::BadArgs(format!(
            "{flag} needs 7-128 hex chars (build SHA)"
        )));
    }
    Ok(sha.to_string())
}

/// Split capture flags out of `raw`, passing everything else through in
/// order for [`WinArgs`](super::windowed::WinArgs). Unknown `--capture*`
/// flags, missing values, `every` of 0, duplicates and a cadence or tape
/// without a directory fail loudly; non-capture flags (known or not) are
/// never touched here, so `WinArgs` still refuses unknowns exactly as before.
pub fn split_capture_args(
    raw: &[String],
) -> Result<(Option<CaptureFlags>, Vec<String>), CaptureError> {
    let mut dir: Option<String> = None;
    let mut every: Option<String> = None;
    let mut tape: Option<String> = None;
    let mut rows: Option<String> = None;
    let mut sha: Option<String> = None;
    let mut fps: Option<String> = None;
    let mut rest = Vec::with_capacity(raw.len());
    let mut index = 0usize;
    while index < raw.len() {
        match raw[index].as_str() {
            "--capture-dir" => store_flag(&mut dir, raw, &mut index, "--capture-dir")?,
            "--capture-every" => store_flag(&mut every, raw, &mut index, "--capture-every")?,
            "--tape" => store_flag(&mut tape, raw, &mut index, "--tape")?,
            "--rows" => store_flag(&mut rows, raw, &mut index, "--rows")?,
            "--sha" => store_flag(&mut sha, raw, &mut index, "--sha")?,
            "--fps" => store_flag(&mut fps, raw, &mut index, "--fps")?,
            other if other.starts_with("--capture") => {
                return Err(CaptureError::BadArgs(other.to_string()));
            }
            _ => rest.push(raw[index].clone()),
        }
        index += 1;
    }
    let dir = match dir {
        Some(dir) => dir,
        None => {
            // Every other capture flag needs the directory, first one wins.
            let present = [
                (every.is_some(), "--capture-every"),
                (tape.is_some(), "--tape"),
                (rows.is_some(), "--rows"),
                (sha.is_some(), "--sha"),
                (fps.is_some(), "--fps"),
            ];
            if let Some((_, flag)) = present.iter().find(|(given, _)| *given) {
                return Err(CaptureError::BadArgs(format!("{flag} needs --capture-dir")));
            }
            return Ok((None, rest));
        }
    };
    let every = match every {
        Some(raw) => parse_every(&raw)?,
        None => {
            return Err(CaptureError::BadArgs(
                "--capture-dir needs --capture-every".to_string(),
            ));
        }
    };
    let rows = match rows {
        Some(raw) => raw
            .split(',')
            .map(|id| check_row_id(id, "--rows"))
            .collect::<Result<Vec<_>, _>>()?,
        None => vec!["SHOW1".to_string()],
    };
    if rows.is_empty() {
        return Err(CaptureError::BadArgs(
            "--rows needs at least one board row ID".to_string(),
        ));
    }
    let sha = match sha {
        Some(raw) => check_sha(&raw, "--sha")?,
        None => {
            return Err(CaptureError::BadArgs(
                "--capture-dir needs --sha (build SHA hex, 7-128 chars)".to_string(),
            ));
        }
    };
    let fps = match fps {
        Some(raw) => {
            let parsed: u32 = raw
                .trim()
                .parse()
                .map_err(|_| CaptureError::BadArgs(format!("--fps needs 1-120, got {raw}")))?;
            if !(1..=120).contains(&parsed) {
                return Err(CaptureError::BadArgs(format!(
                    "--fps needs 1-120, got {raw}"
                )));
            }
            parsed
        }
        None => 30,
    };
    Ok((
        Some(CaptureFlags {
            dir: check_path(&dir, "--capture-dir")?,
            every,
            tape: tape.map(|path| check_path(&path, "--tape")).transpose()?,
            rows,
            sha,
            fps,
        }),
        rest,
    ))
}

/// Take the value after a flag: missing values and values starting a new
/// flag fail loudly instead of being swallowed.
fn take_value(raw: &[String], index: &mut usize, flag: &str) -> Result<String, CaptureError> {
    *index += 1;
    match raw.get(*index) {
        Some(value) if !value.starts_with("--") => Ok(value.clone()),
        _ => Err(CaptureError::BadArgs(format!("{flag} needs a value"))),
    }
}

/// Store one flag's value in `slot`: a repeat fails as a duplicate, a missing
/// value fails through [`take_value`].
fn store_flag(
    slot: &mut Option<String>,
    raw: &[String],
    index: &mut usize,
    flag: &str,
) -> Result<(), CaptureError> {
    if slot.is_some() {
        return Err(CaptureError::BadArgs(format!("duplicate {flag}")));
    }
    *slot = Some(take_value(raw, index, flag)?);
    Ok(())
}

/// Parse the frame cadence: a number in `1..=MAX_TICKS`, never 0.
fn parse_every(raw: &str) -> Result<u32, CaptureError> {
    let value: u32 = raw
        .parse()
        .map_err(|_| CaptureError::BadArgs(format!("--capture-every {raw}")))?;
    if value == 0 || value > MAX_TICKS {
        return Err(CaptureError::BadArgs(format!("--capture-every {raw}")));
    }
    Ok(value)
}

/// Validate a flag path: non-empty and bounded (the filesystem owns the rest
/// and reports it through [`CaptureError::Io`]).
fn check_path(raw: &str, flag: &str) -> Result<PathBuf, CaptureError> {
    if raw.is_empty() || raw.len() > MAX_PATH_BYTES {
        return Err(CaptureError::BadArgs(format!("{flag} path rejected")));
    }
    Ok(PathBuf::from(raw))
}

// ---------------------------------------------------------------------------
// Tape files: magic plus version plus count plus canonical 10-byte inputs.
// ---------------------------------------------------------------------------

/// A tape file on disk: validated inputs plus file identity for the manifest.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LoadedTape {
    /// `(tick, fighter)`-sorted human inputs driving the capture.
    pub inputs: Vec<FighterInput>,
    /// File name only (never the absolute path, so manifests compare).
    pub file_name: String,
    /// FNV-1a hex of the exact file bytes (binds tape identity to frames).
    pub fnv: String,
}

/// Encode tape inputs for [`load_tape_file`] (tests plus future recorders).
#[must_use]
pub fn encode_tape_bytes(inputs: &[FighterInput]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER_BYTES + inputs.len() * INPUT_BYTES);
    bytes.extend_from_slice(TAPE_MAGIC);
    bytes.extend_from_slice(&CAPTURE_VERSION.to_le_bytes());
    bytes.extend_from_slice(
        &u32::try_from(inputs.len() & 0xffff_ffff)
            .unwrap_or(0)
            .to_le_bytes(),
    );
    for input in inputs {
        input.append_bytes(&mut bytes);
    }
    bytes
}

/// Load and validate a tape file: magic, version, exact length, per-input
/// ranges plus tape order. Trailing bytes and truncations both fail.
pub fn load_tape_file(path: &Path) -> Result<LoadedTape, CaptureError> {
    let mut file = std::fs::File::open(path).map_err(|err| io_err(path, err))?;
    let mut bytes = Vec::new();
    {
        use std::io::Read as _;
        file.by_ref()
            .take(u64::try_from(MAX_TAPE_FILE_BYTES).unwrap_or(u64::MAX) + 1)
            .read_to_end(&mut bytes)
            .map_err(|err| io_err(path, err))?;
    }
    if bytes.len() > MAX_TAPE_FILE_BYTES {
        return Err(CaptureError::BadTape("tape file too large".to_string()));
    }
    let inputs = decode_tape_bytes(&bytes)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CaptureError::BadTape("tape file name must be UTF-8".to_string()))?
        .to_string();
    Ok(LoadedTape {
        inputs,
        file_name,
        fnv: fnv_hex(&bytes),
    })
}

/// Decode tape bytes without trusting sizes, versions or trailing data.
fn decode_tape_bytes(bytes: &[u8]) -> Result<Vec<FighterInput>, CaptureError> {
    if bytes.len() < HEADER_BYTES {
        return Err(CaptureError::BadTape("truncated tape header".to_string()));
    }
    if bytes[0..8] != TAPE_MAGIC[..] {
        return Err(CaptureError::BadTape("bad tape magic".to_string()));
    }
    let version = u16::from_le_bytes(
        bytes[8..10]
            .try_into()
            .map_err(|_| CaptureError::BadTape("truncated tape header".to_string()))?,
    );
    if version != CAPTURE_VERSION {
        return Err(CaptureError::BadTape(format!(
            "unsupported tape version {version}"
        )));
    }
    let count =
        usize::try_from(u32::from_le_bytes(bytes[10..14].try_into().map_err(
            |_| CaptureError::BadTape("truncated tape header".to_string()),
        )?))
        .unwrap_or(usize::MAX);
    if count > MAX_CAPTURE_INPUTS {
        return Err(CaptureError::BadTape("too many tape inputs".to_string()));
    }
    if bytes.len() != HEADER_BYTES + count * INPUT_BYTES {
        return Err(CaptureError::BadTape(
            "tape counts or length mismatch".to_string(),
        ));
    }
    let mut inputs = Vec::with_capacity(count);
    for index in 0..count {
        let chunk = &bytes[HEADER_BYTES + index * INPUT_BYTES..][..INPUT_BYTES];
        let tick = u32::from_le_bytes(
            chunk[0..4]
                .try_into()
                .map_err(|_| CaptureError::BadTape(format!("truncated tape input {index}")))?,
        );
        let fighter = FighterId::new(chunk[4])
            .ok_or_else(|| CaptureError::BadTape(format!("tape input {index} names no fighter")))?;
        let facing = i16::from_le_bytes(
            chunk[7..9]
                .try_into()
                .map_err(|_| CaptureError::BadTape(format!("truncated tape input {index}")))?,
        );
        let input = FighterInput {
            tick,
            fighter,
            mx: i8::from_ne_bytes([chunk[5]]),
            mz: i8::from_ne_bytes([chunk[6]]),
            facing,
            buttons: Buttons(chunk[9]),
        };
        input.validate().map_err(|err| {
            CaptureError::BadTape(format!("tape input {index} invalid ({err:?})"))
        })?;
        inputs.push(input);
    }
    FighterTape(inputs.clone())
        .validate()
        .map_err(|err| CaptureError::BadTape(format!("tape order invalid ({err:?})")))?;
    Ok(inputs)
}

/// Check tape inputs against one capture run: every entry names the human
/// slot and lands inside the session, or the run refuses before writing.
pub fn validate_tape_for_run(
    inputs: &[FighterInput],
    human_slot: u8,
    ticks: u32,
) -> Result<(), CaptureError> {
    for input in inputs {
        if input.fighter.get() != human_slot {
            return Err(CaptureError::BadTape(format!(
                "tape input at tick {} names fighter {}, capture runs slot {human_slot}",
                input.tick,
                input.fighter.get(),
            )));
        }
        if input.tick >= ticks {
            return Err(CaptureError::BadTape(format!(
                "tape input at tick {} past session end {ticks}",
                input.tick,
            )));
        }
    }
    Ok(())
}

/// Cursor over sorted tape inputs: ticks ascend, so one pointer serves the
/// whole run with no search and no allocation.
#[derive(Clone, Debug, Default)]
pub struct TapeInputs {
    inputs: Vec<FighterInput>,
    cursor: usize,
}

impl TapeInputs {
    /// Wrap sorted tape inputs (empty idles every tick).
    #[must_use]
    pub fn new(inputs: Vec<FighterInput>) -> Self {
        Self { inputs, cursor: 0 }
    }

    /// The staged input for `tick`, if the tape names it (exact match; ticks
    /// are queried in order, so a stale entry is unreachable).
    pub fn input_for_tick(&mut self, tick: u32) -> Option<FighterInput> {
        if self.cursor < self.inputs.len() && self.inputs[self.cursor].tick == tick {
            let input = self.inputs[self.cursor];
            self.cursor += 1;
            Some(input)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// PNG writer: std-only, stored deflate, deterministic bytes for one input.
// ---------------------------------------------------------------------------

/// FNV-1a hex of bytes (non-crypto checksum for manifest rows, tape identity
/// and package digests; never secrets).
///
/// Single owner for the crate's FNV-1a hex digest: the packager and the
/// `staged_fight` binary reuse this instead of keeping local copies.
pub fn fnv_hex(bytes: &[u8]) -> String {
    format!("{:016x}", hash_bytes(bytes))
}

fn crc32_table() -> &'static [u32; 256] {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (index, cell) in table.iter_mut().enumerate() {
            let mut c = u32::try_from(index & 0xffff_ffff).unwrap_or(0);
            for _ in 0..8 {
                c = if c & 1 == 1 {
                    0xEDB88320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *cell = c;
        }
        table
    })
}

fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    for byte in data {
        crc = crc32_table()[usize::try_from((crc ^ u32::from(*byte)) & 0xFF).unwrap_or(usize::MAX)]
            ^ (crc >> 8);
    }
    crc
}

fn write_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(
        &u32::try_from(data.len() & 0xffff_ffff)
            .unwrap_or(0)
            .to_be_bytes(),
    );
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut crc = crc32_update(0xFFFF_FFFF, tag);
    crc = crc32_update(crc, data);
    out.extend_from_slice(&(crc ^ 0xFFFF_FFFF).to_be_bytes());
}

/// Encode RGBA bytes as a PNG (8-bit, filter 0, stored deflate): the same
/// pixels always give the same bytes on every machine.
fn encode_png_rgba8(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, CaptureError> {
    if SizeClass::classify(width, height) != SizeClass::Sizes {
        return Err(CaptureError::BadArgs(format!(
            "capture size {width}x{height} rejected"
        )));
    }
    let stride = usize::try_from(width).unwrap_or(usize::MAX) * 4;
    if pixels.len() != stride * usize::try_from(height).unwrap_or(usize::MAX) {
        return Err(CaptureError::BadPng("pixel length mismatch".to_string()));
    }
    let mut raw = Vec::with_capacity(pixels.len() + usize::try_from(height).unwrap_or(usize::MAX));
    for row in pixels.chunks_exact(stride) {
        raw.push(0u8);
        raw.extend_from_slice(row);
    }
    let mut zlib = Vec::with_capacity(raw.len() + 16);
    zlib.extend_from_slice(&[0x78, 0x01]);
    let mut blocks = raw.chunks(PNG_BLOCK_MAX).peekable();
    while let Some(block) = blocks.next() {
        let last = blocks.peek().is_none();
        zlib.push(u8::from(last));
        let len = u16::try_from(block.len() & 0xffff).unwrap_or(0);
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = Vec::with_capacity(8 + 25 + zlib.len() + 16 + 12);
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 6;
    write_chunk(&mut out, b"IHDR", &ihdr);
    write_chunk(&mut out, b"IDAT", &zlib);
    write_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

/// IEND chunk bytes: every PNG ends with exactly these (empty chunk CRC).
const PNG_IEND_TAIL: &[u8; 12] = &[0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130];

/// Reject truncated PNGs: the file must end with the IEND chunk (true for
/// both this writer and the twin encoder, which both emit standard PNGs).
fn check_png_complete(png: &[u8]) -> Result<(), CaptureError> {
    if png.len() < 45 || png[png.len() - 12..] != PNG_IEND_TAIL[..] {
        return Err(CaptureError::BadPng("truncated PNG".to_string()));
    }
    Ok(())
}

/// Read `(width, height)` out of PNG bytes, rejecting non-PNGs loudly.
fn parse_png_dims(png: &[u8]) -> Result<(u32, u32), CaptureError> {
    if png.len() < 33
        || png[0..8] != [137, 80, 78, 71, 13, 10, 26, 10]
        || png[12..16] != *b"IHDR"
        || u32::from_be_bytes(
            png[8..12]
                .try_into()
                .map_err(|_| CaptureError::BadPng("truncated PNG header".to_string()))?,
        ) != 13
    {
        return Err(CaptureError::BadPng("not a capture PNG".to_string()));
    }
    let width = u32::from_be_bytes(
        png[16..20]
            .try_into()
            .map_err(|_| CaptureError::BadPng("truncated PNG header".to_string()))?,
    );
    let height = u32::from_be_bytes(
        png[20..24]
            .try_into()
            .map_err(|_| CaptureError::BadPng("truncated PNG header".to_string()))?,
    );
    if SizeClass::classify(width, height) != SizeClass::Sizes {
        return Err(CaptureError::BadPng(format!(
            "PNG size {width}x{height} rejected"
        )));
    }
    Ok((width, height))
}

/// Headless frame bound to the sim: SplitMix64 pixels seeded by session seed,
/// checkpoint hash and tick, so the same tape gives the same PNGs twice and
/// a different tape gives different PNGs. The window build uses twin bytes
/// instead; this path only serves headless machines with no adapter.
fn headless_frame_png(
    seed: u64,
    checkpoint_hash: u64,
    tick: u32,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, CaptureError> {
    if SizeClass::classify(width, height) != SizeClass::Sizes {
        return Err(CaptureError::BadArgs(format!(
            "capture size {width}x{height} rejected"
        )));
    }
    let count = usize::try_from(width).unwrap_or(usize::MAX)
        * usize::try_from(height).unwrap_or(usize::MAX);
    let mut pixels = vec![0u8; count * 4];
    let mut state = seed
        .wrapping_add(checkpoint_hash.rotate_left(7))
        .wrapping_add(u64::from(tick).wrapping_mul(GOLDEN_GAMMA));
    // Per-frame stream from sim state: no wall clock, no pointer values.
    for pixel in pixels.chunks_exact_mut(4) {
        let word = arena_seed::next(&mut state);
        let word_bytes = word.to_le_bytes();
        pixel[0] = word_bytes[0];
        pixel[1] = word_bytes[1];
        pixel[2] = word_bytes[2];
        pixel[3] = 255;
    }
    encode_png_rgba8(&pixels, width, height)
}

// ---------------------------------------------------------------------------
// Sink: PNG bytes in, `frame_{:06}.png` plus `manifest.json` on disk.
// ---------------------------------------------------------------------------

/// One manifest row: which tick produced which file with which hashes.
#[derive(Clone, PartialEq, Eq, Debug)]
struct CaptureRow {
    tick: u32,
    file: String,
    width: u32,
    height: u32,
    checkpoint: String,
    png_fnv: String,
    sha256: String,
    bytes: u64,
}

/// Manifest identity for the stitch contract (`tools/showcase.mjs`):
/// board rows evidenced, build SHA hex, playback fps. Bundled so the sink
/// constructor stays under the argument-count lint.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ManifestMeta {
    /// Board rows the clip evidences (non-empty valid IDs).
    pub rows: Vec<String>,
    /// Build SHA hex (7-128 chars).
    pub sha: String,
    /// Playback fps (`1..=120`).
    pub fps: u32,
}

/// Window look tap: the window host owns one tap and notes each validated
/// capture PNG once; refusals count as faults, never as calls.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct LookTap {
    calls: u32,
    bytes: u64,
    faults: u32,
}

impl LookTap {
    /// Empty tap with no notes yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            calls: 0,
            bytes: 0,
            faults: 0,
        }
    }

    /// Note one validated PNG: one call plus its byte count, saturating.
    /// Refusals (short magic, empty, truncated) count one fault and leave
    /// calls plus bytes untouched; validated frames still count once each.
    pub fn note(&mut self, png: &[u8]) {
        if png.len() < 8 || png[0..8] != [137, 80, 78, 71, 13, 10, 26, 10] {
            self.faults = self.faults.saturating_add(1);
            return;
        }
        self.calls = self.calls.saturating_add(1);
        self.bytes = self
            .bytes
            .saturating_add(u64::try_from(png.len()).unwrap_or(u64::MAX));
    }

    /// Notes so far (one per validated frame).
    #[must_use]
    pub fn calls(&self) -> u32 {
        self.calls
    }

    /// Refusals so far (one per non-PNG note, never a call).
    #[must_use]
    pub fn faults(&self) -> u32 {
        self.faults
    }
}

/// Frame sink: created up front (bad dirs fail fast), frames appended in
/// order, the manifest sealed by [`CaptureSink::finish`].
#[derive(Debug)]
pub struct CaptureSink {
    dir: PathBuf,
    every: u32,
    seed: u64,
    tape_name: Option<String>,
    tape_fnv: Option<String>,
    renderer: &'static str,
    frames: u32,
    sequence: u64,
    first_size: Option<(u32, u32)>,
    rows: Vec<CaptureRow>,
    board_rows: Vec<String>,
    build_sha: String,
    fps: u32,
    /// Spectator-camera digest hex, noted by the headless run before sealing.
    director_digest: String,
    /// Spectator-camera cut count, noted by the headless run before sealing.
    director_cuts: u32,
    /// Canonical cut-log bytes filed beside the manifest, if the run noted
    /// its cuts through [`CaptureSink::note_director_cuts`].
    cutlog_json: Option<String>,
}

impl CaptureSink {
    /// Open the sink, creating the directory. `every` of 0 and an empty
    /// renderer label fail; the meta's rows must be non-empty valid IDs,
    /// the build SHA 7-128 hex chars, fps 1-120; IO failures carry path
    /// plus kind.
    pub fn create(
        dir: &Path,
        every: u32,
        seed: u64,
        tape: Option<&LoadedTape>,
        renderer: &'static str,
        meta: &ManifestMeta,
    ) -> Result<Self, CaptureError> {
        if every == 0 || every > MAX_TICKS {
            return Err(CaptureError::BadArgs(format!("--capture-every {every}")));
        }
        if renderer.is_empty() {
            return Err(CaptureError::BadArgs("capture renderer unset".to_string()));
        }
        if meta.rows.is_empty() {
            return Err(CaptureError::BadArgs(
                "capture rows needs board row IDs".to_string(),
            ));
        }
        for row in &meta.rows {
            check_row_id(row, "rows")?;
        }
        let build_sha = check_sha(&meta.sha, "sha")?;
        if !(1..=120).contains(&meta.fps) {
            return Err(CaptureError::BadArgs(format!(
                "capture fps {} needs 1-120",
                meta.fps
            )));
        }
        if dir.as_os_str().is_empty() {
            return Err(CaptureError::BadArgs(
                "--capture-dir path rejected".to_string(),
            ));
        }
        std::fs::create_dir_all(dir).map_err(|err| io_err(dir, err))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            every,
            seed,
            tape_name: tape.map(|loaded| loaded.file_name.clone()),
            tape_fnv: tape.map(|loaded| loaded.fnv.clone()),
            renderer,
            frames: 0,
            sequence: FNV64_OFFSET,
            first_size: None,
            rows: Vec::new(),
            board_rows: meta.rows.clone(),
            build_sha,
            fps: meta.fps,
            director_digest: String::new(),
            director_cuts: 0,
            cutlog_json: None,
        })
    }

    /// True when presented frame `presented` (1-based, after increment) is
    /// due: the first frame plus every `every`-th after it.
    #[must_use]
    pub fn should_capture(&self, presented: u32) -> bool {
        presented > 0 && (presented - 1).is_multiple_of(self.every)
    }

    /// Frames captured so far.
    #[must_use]
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// Note the session's spectator-camera cuts plus held shot before
    /// sealing: the sink encodes the canonical cut log itself and files the
    /// digest over those exact bytes, so a caller can never file a digest
    /// for cuts it did not serve. The headless run calls this after its
    /// last tick; [`CaptureSink::finish`] writes the bytes as `cutlog.json`.
    pub fn note_director_cuts(&mut self, cuts: &[Cut], shot: Shot) {
        self.director_digest = format!("{:016x}", cutlog_digest(cuts, shot));
        self.director_cuts = u32::try_from(cuts.len()).unwrap_or(u32::MAX);
        self.cutlog_json = Some(cutlog_json(cuts, shot));
    }

    /// Write one presented frame: validates the PNG, files it as
    /// `frame_{:06}.png` and records its row. Non-PNG bytes fail loudly.
    pub fn write_frame(
        &mut self,
        tick: u32,
        checkpoint_hash: u64,
        png: &[u8],
    ) -> Result<(), CaptureError> {
        self.write_frame_with_hook(tick, checkpoint_hash, png, None)
    }

    /// Write one frame then notify `hook` once with the validated bytes.
    /// Units: ticks, bytes. Errors: [`CaptureError::BadPng`] or `Io`;
    /// refusal never calls the hook. Determinism: same inputs, same files.
    pub fn write_frame_with_hook(
        &mut self,
        tick: u32,
        checkpoint_hash: u64,
        png: &[u8],
        hook: Option<&mut dyn FnMut(&[u8])>,
    ) -> Result<(), CaptureError> {
        use sha2::{Digest, Sha256};
        let (width, height) = parse_png_dims(png)?;
        check_png_complete(png)?;
        if self.first_size.is_none() {
            self.first_size = Some((width, height));
        }
        let file = format!(
            "frame_{:0width$}.png",
            self.frames,
            width = FRAME_NAME_WIDTH
        );
        std::fs::write(self.dir.join(&file), png)
            .map_err(|err| io_err(&self.dir.join(&file), err))?;
        let mut sequence = Fnv64::with_state(self.sequence);
        sequence.write(png);
        self.sequence = sequence.finish();
        let mut digest = Sha256::new();
        digest.update(png);
        self.rows.push(CaptureRow {
            tick,
            file,
            width,
            height,
            checkpoint: format!("{checkpoint_hash:016x}"),
            png_fnv: fnv_hex(png),
            sha256: format!("{:x}", digest.finalize()),
            bytes: u64::try_from(png.len()).unwrap_or(u64::MAX),
        });
        self.frames += 1;
        if let Some(hook) = hook {
            hook(png);
        }
        Ok(())
    }

    /// Seal the run: write `manifest.json` (seed, tape, rows, SHA) beside
    /// the PNGs, plus `cutlog.json` when the run noted its cuts, and report
    /// the receipt.
    pub fn finish(self) -> Result<CaptureReport, CaptureError> {
        let manifest = manifest_json(&self);
        let path = self.dir.join(MANIFEST_NAME);
        std::fs::write(&path, manifest.as_bytes()).map_err(|err| io_err(&path, err))?;
        if let Some(cutlog) = &self.cutlog_json {
            let cutlog_path = self.dir.join(CUTLOG_NAME);
            std::fs::write(&cutlog_path, cutlog.as_bytes())
                .map_err(|err| io_err(&cutlog_path, err))?;
        }
        Ok(CaptureReport {
            frames: self.frames,
            manifest: path,
            sequence_fnv: format!("{:016x}", self.sequence),
            director_digest: self.director_digest,
            director_cuts: self.director_cuts,
        })
    }
}

/// Capture receipt: frame count, manifest path and the sequence checksum.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CaptureReport {
    /// Frames written.
    pub frames: u32,
    /// Manifest file written beside the PNGs.
    pub manifest: PathBuf,
    /// FNV-1a hex over the PNG bytes in order (the sequence SHA row).
    pub sequence_fnv: String,
    /// Spectator-camera cut-log digest hex noted before sealing.
    pub director_digest: String,
    /// Spectator-camera cuts noted before sealing.
    pub director_cuts: u32,
}

fn json_escape_into(out: &mut String, text: &str) {
    for char in text.chars() {
        match char {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ if u32::from(char) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", u32::from(char)));
            }
            _ => out.push(char),
        }
    }
}

/// Manifest beside the PNGs: the stitch-validator contract
/// (`tools/showcase.mjs`: uint seed, non-empty tape string, board-ID rows,
/// build SHA hex, fps, per-file frames with sha256) plus the spectator-camera
/// digest and cut count. Fixed field order, no
/// timestamps, file names only: the same run gives byte-identical manifests
/// twice. A run with no tape file records `"live"` as its tape identity.
fn manifest_json(sink: &CaptureSink) -> String {
    let mut out = String::from("{\"schema\":1");
    out.push_str(",\"engine_version\":\"");
    out.push_str(env!("CARGO_PKG_VERSION"));
    out.push_str("\",\"renderer\":\"");
    out.push_str(sink.renderer);
    out.push_str("\",\"seed\":");
    out.push_str(&sink.seed.to_string());
    out.push_str(",\"tape\":\"");
    match &sink.tape_name {
        Some(name) => json_escape_into(&mut out, name),
        None => out.push_str("live"),
    }
    out.push_str("\",\"tape_fnv\":");
    match &sink.tape_fnv {
        Some(hex) => {
            out.push('"');
            out.push_str(hex);
            out.push('"');
        }
        None => out.push_str("null"),
    }
    out.push_str(",\"capture_every\":");
    out.push_str(&sink.every.to_string());
    out.push_str(",\"fps\":");
    out.push_str(&sink.fps.to_string());
    out.push_str(",\"width\":");
    match sink.first_size {
        Some((width, _)) => out.push_str(&width.to_string()),
        None => out.push_str("null"),
    }
    out.push_str(",\"height\":");
    match sink.first_size {
        Some((_, height)) => out.push_str(&height.to_string()),
        None => out.push_str("null"),
    }
    out.push_str(",\"sequence_fnv\":\"");
    out.push_str(&format!("{:016x}", sink.sequence));
    out.push_str("\",\"director_digest\":\"");
    json_escape_into(&mut out, &sink.director_digest);
    out.push_str("\",\"director_cuts\":");
    out.push_str(&sink.director_cuts.to_string());
    out.push_str(",\"sha\":\"");
    out.push_str(&sink.build_sha);
    out.push_str("\",\"rows\":[");
    for (index, row) in sink.board_rows.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('"');
        out.push_str(row);
        out.push('"');
    }
    out.push_str("],\"frames\":[");
    for (index, row) in sink.rows.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"tick\":");
        out.push_str(&row.tick.to_string());
        out.push_str(",\"file\":\"");
        out.push_str(&row.file);
        out.push_str("\",\"width\":");
        out.push_str(&row.width.to_string());
        out.push_str(",\"height\":");
        out.push_str(&row.height.to_string());
        out.push_str(",\"checkpoint\":\"");
        out.push_str(&row.checkpoint);
        out.push_str("\",\"png_fnv\":\"");
        out.push_str(&row.png_fnv);
        out.push_str("\",\"sha256\":\"");
        out.push_str(&row.sha256);
        out.push_str("\",\"bytes\":");
        out.push_str(&row.bytes.to_string());
        out.push('}');
    }
    out.push_str("]}");
    out
}

// ---------------------------------------------------------------------------
// Headless run: config plus tape in, PNG sequence plus manifest on disk.
// ---------------------------------------------------------------------------

/// Headless capture request: where, how often, how big, how many ticks.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HeadlessRequest {
    /// Directory the PNG sequence plus `manifest.json` go into.
    pub dir: PathBuf,
    /// One captured frame per this many ticks (`>= 1`).
    pub every: u32,
    /// Frame width in pixels (`1..=MAX_FRAME_DIM`).
    pub width: u32,
    /// Frame height in pixels (`1..=MAX_FRAME_DIM`).
    pub height: u32,
    /// Ticks to simulate (`1..=config.ticks`).
    pub frames: u32,
    /// Board rows the clip evidences (non-empty valid IDs).
    pub rows: Vec<String>,
    /// Build SHA hex (7-128 chars).
    pub sha: String,
    /// Playback fps (`1..=120`).
    pub fps: u32,
}

/// Capture scene id the review path selects for the arena bundle.
#[cfg(test)]
pub const ARENA_CAPTURE_SCENE: &str = "arena";
/// Fighter count for the 8-fighter capture (arena plus 8).
#[cfg(test)]
pub const ARENA8_FIGHTERS: u8 = 8;
/// Arena half-extent for the capture scene in meters.
#[cfg(test)]
pub const ARENA_CAPTURE_EXTENT_M: f32 = 20.0;

/// Build the arena scene config the capture path can select: `scene` must
/// name the arena and `fighters` must sit in `2..=MAX_PARTICIPANTS` (the
/// 8-fighter capture passes 8). Bad scenes and counts fail loudly before any frame.
#[cfg(test)]
pub fn arena_capture_config(
    scene: &str,
    seed: u64,
    ticks: u32,
    fighters: u8,
) -> Result<GameConfig, CaptureError> {
    if scene != ARENA_CAPTURE_SCENE {
        return Err(CaptureError::BadArgs(format!("--scene {scene}")));
    }
    let config = GameConfig {
        seed,
        ticks,
        fighters,
        half_extent_m: ARENA_CAPTURE_EXTENT_M,
    };
    config.validate().map_err(map_config_error)?;
    Ok(config)
}

/// Canonical 8-fighter arena config for review captures.
#[cfg(test)]
pub fn arena8_capture_config(seed: u64, ticks: u32) -> Result<GameConfig, CaptureError> {
    arena_capture_config(ARENA_CAPTURE_SCENE, seed, ticks, ARENA8_FIGHTERS)
}

fn map_config_error(err: GameError) -> CaptureError {
    match err {
        GameError::BadConfig(row) => CaptureError::BadConfig(row),
        other => CaptureError::BadArgs(format!("bad capture config ({other:?})")),
    }
}

/// Run the session headless over the tape (idle ticks where the tape is
/// silent) and capture every `every`-th tick through
/// [`headless_frame_png`]. Same config plus tape twice gives byte-identical
/// PNGs and manifest; a different tape gives different pixels.
pub fn run_headless_capture(
    config: GameConfig,
    tape: Option<&LoadedTape>,
    req: &HeadlessRequest,
) -> Result<CaptureReport, CaptureError> {
    config.validate().map_err(map_config_error)?;
    if req.frames == 0 || req.frames > config.ticks {
        return Err(CaptureError::BadArgs(format!(
            "capture frames {} past session end {}",
            req.frames, config.ticks,
        )));
    }
    if SizeClass::classify(req.width, req.height) != SizeClass::Sizes {
        return Err(CaptureError::BadArgs(format!(
            "capture size {}x{} rejected",
            req.width, req.height,
        )));
    }
    if req.every == 0 || req.every > MAX_TICKS {
        return Err(CaptureError::BadArgs(format!(
            "--capture-every {}",
            req.every
        )));
    }
    let inputs = tape.map(|loaded| loaded.inputs.as_slice()).unwrap_or(&[]);
    validate_tape_for_run(inputs, HUMAN_SLOT, config.ticks)?;
    let meta = ManifestMeta {
        rows: req.rows.clone(),
        sha: req.sha.clone(),
        fps: req.fps,
    };
    let mut sink = CaptureSink::create(
        &req.dir,
        req.every,
        config.seed,
        tape,
        HEADLESS_RENDERER,
        &meta,
    )?;
    // Host tap counts each validated PNG once; refusals are faults.
    let mut tap = LookTap::new();
    let mut app = App::new(config).map_err(map_config_error)?;
    let human = FighterId::new(HUMAN_SLOT).ok_or(CaptureError::BadConfig("fighters"))?;
    let mut cursor = TapeInputs::new(inputs.to_vec());
    for tick in 0..req.frames {
        let live = FighterInput {
            tick,
            fighter: human,
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        let input = cursor.input_for_tick(tick).unwrap_or(live);
        let outcome = app.tick(input);
        if !outcome.input_accepted {
            return Err(CaptureError::BadTape(format!("tick {tick} rejected")));
        }
        if sink.should_capture(tick + 1) {
            let hash = outcome.checkpoint.map(|point| point.hash).unwrap_or(0);
            let png = headless_frame_png(config.seed, hash, tick, req.width, req.height)?;
            sink.write_frame_with_hook(tick, hash, &png, Some(&mut |bytes| tap.note(bytes)))?;
        }
    }
    // Headless PNGs always validate, so any fault is a writer defect.
    if tap.faults() != 0 {
        return Err(CaptureError::BadPng(
            "headless tap refused a PNG".to_string(),
        ));
    }
    if tap.calls() != sink.frames() {
        return Err(CaptureError::BadPng(
            "headless tap missed a frame".to_string(),
        ));
    }
    // The session drove its spectator camera from the same ticks: file the
    // canonical cut log beside the frames, with the digest computed over
    // those exact cuts (never a caller-supplied string).
    sink.note_director_cuts(app.director_cuts(), app.director_shot());
    sink.finish()
}

/// Verify a sealed capture directory binds its cut log: the manifest's
/// `director_digest` plus `director_cuts` rows must match the digest over
/// the `cutlog.json` cuts filed beside the frames. A forged cut (any tick,
/// framing, focus, side or reason change), a resealed count, a missing
/// cut log or an unbound manifest (no digest rows, like the ADV52 fixture)
/// all fail with [`CaptureError::BadCutlog`]; clean captures pass.
pub fn verify_capture_binding(dir: &Path) -> Result<(), CaptureError> {
    let manifest_path = dir.join(MANIFEST_NAME);
    let manifest = read_receipt_file(&manifest_path)?;
    let digest_hex = manifest_string_field(&manifest, "\"director_digest\":\"")
        .ok_or_else(|| CaptureError::BadCutlog("manifest missing director_digest".to_string()))?;
    if digest_hex.is_empty() {
        return Err(CaptureError::BadCutlog(
            "manifest missing director_digest".to_string(),
        ));
    }
    let cuts = manifest_number_field(&manifest, "\"director_cuts\":")
        .ok_or_else(|| CaptureError::BadCutlog("manifest missing director_cuts".to_string()))?;
    let cutlog_path = dir.join(CUTLOG_NAME);
    if !cutlog_path.is_file() {
        return Err(CaptureError::BadCutlog(
            "capture missing cutlog.json".to_string(),
        ));
    }
    let cutlog = read_receipt_file(&cutlog_path)?;
    verify_cutlog_json(&cutlog, &digest_hex, cuts)
        .map_err(|err| CaptureError::BadCutlog(cap_message(&err.to_string())))
}

/// Read a receipt sidecar (manifest or cut log) with a size cap, so a
/// hostile directory cannot feed unbounded bytes into the parser.
fn read_receipt_file(path: &Path) -> Result<String, CaptureError> {
    let bytes = std::fs::read(path).map_err(|err| io_err(path, err))?;
    if bytes.len() > MAX_RECEIPT_BYTES {
        return Err(CaptureError::BadCutlog(
            "receipt file too large".to_string(),
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| CaptureError::BadCutlog("receipt file must be UTF-8".to_string()))
}

/// Borrow the bytes after `key` in a receipt sidecar.
fn receipt_field_rest<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let after = text.find(key)? + key.len();
    text.get(after..)
}

/// Borrow the string filed as `"key":"value"` in a receipt sidecar.
fn manifest_string_field(text: &str, key: &str) -> Option<String> {
    let rest = receipt_field_rest(text, key)?;
    let end = rest.find('"')?;
    rest.get(..end).map(str::to_string)
}

/// Read the decimal number filed as `"key":digits` in a receipt sidecar.
fn manifest_number_field(text: &str, key: &str) -> Option<u32> {
    let rest = receipt_field_rest(text, key)?;
    let len = rest.bytes().take_while(u8::is_ascii_digit).count();
    if len == 0 || len > 10 {
        return None;
    }
    let mut value: u32 = 0;
    for byte in rest.bytes().take(len) {
        value = value.checked_mul(10)?.checked_add(u32::from(byte - b'0'))?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(ticks: u32) -> GameConfig {
        GameConfig {
            seed: 2040,
            ticks,
            fighters: 2,
            half_extent_m: 20.0,
        }
    }

    fn idle_input(tick: u32) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(HUMAN_SLOT).unwrap(),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        }
    }

    /// Deterministic scripted tape: strafe cycle plus an attack every 12 ticks.
    fn scripted_inputs() -> Vec<FighterInput> {
        (0..48u32)
            .map(|tick| {
                let phase = (tick / 6) % 4;
                let (mx, mz) = match phase {
                    0 => (100, 0),
                    1 => (0, 100),
                    2 => (-100, 0),
                    _ => (0, -100),
                };
                FighterInput {
                    tick,
                    fighter: FighterId::new(HUMAN_SLOT).unwrap(),
                    mx,
                    mz,
                    facing: 0,
                    buttons: if tick.is_multiple_of(12) {
                        Buttons(engine_input::fighter::ATTACK)
                    } else {
                        Buttons::NONE
                    },
                }
            })
            .collect()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("e2040-capture-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn dir_files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn capture_request(dir: PathBuf, frames: u32) -> HeadlessRequest {
        HeadlessRequest {
            dir,
            every: 4,
            width: 64,
            height: 48,
            frames,
            rows: vec!["SHOW1".to_string()],
            sha: "deadbeef1234".to_string(),
            fps: 30,
        }
    }

    #[test]
    fn capture_same_tape_gives_byte_identical_pngs_twice() {
        let tape = LoadedTape {
            inputs: scripted_inputs(),
            file_name: "script.tape".to_string(),
            fnv: fnv_hex(&encode_tape_bytes(&scripted_inputs())),
        };
        let first_dir = temp_dir("twice-a");
        let second_dir = temp_dir("twice-b");
        let first = run_headless_capture(
            test_config(48),
            Some(&tape),
            &capture_request(first_dir.clone(), 48),
        )
        .unwrap();
        let second = run_headless_capture(
            test_config(48),
            Some(&tape),
            &capture_request(second_dir.clone(), 48),
        )
        .unwrap();
        assert_eq!(first.frames, 12);
        assert_eq!(second.frames, 12);
        assert_eq!(first.sequence_fnv, second.sequence_fnv);
        let first_files = dir_files(&first_dir);
        let second_files = dir_files(&second_dir);
        assert_eq!(first_files, second_files);
        // Twelve frames plus the manifest plus the bound cut log, all
        // byte-identical across runs (the loop below compares each file).
        assert_eq!(first_files.len(), 14);
        assert!(first_files.contains(&CUTLOG_NAME.to_string()));
        for name in &first_files {
            let a = std::fs::read(first_dir.join(name)).unwrap();
            let b = std::fs::read(second_dir.join(name)).unwrap();
            assert_eq!(a, b, "file differs across runs: {name}");
        }
        std::fs::remove_dir_all(&first_dir).unwrap();
        std::fs::remove_dir_all(&second_dir).unwrap();
    }

    #[test]
    fn capture_manifest_names_seed_tape_rows_and_sha() {
        let dir = temp_dir("manifest");
        let tape_path = dir.join("script.tape");
        std::fs::write(&tape_path, encode_tape_bytes(&scripted_inputs())).unwrap();
        let tape = load_tape_file(&tape_path).unwrap();
        let report = run_headless_capture(
            test_config(48),
            Some(&tape),
            &capture_request(dir.clone(), 48),
        )
        .unwrap();
        assert_eq!(report.frames, 12);
        let manifest = std::fs::read_to_string(&report.manifest).unwrap();
        // Stitch-validator contract mirror (tools/showcase.mjs): uint seed,
        // non-empty tape, board-ID rows, build SHA hex, fps, per-file frames
        // with sha256 + bytes. The real cross-check runs via --check-only.
        assert!(manifest.contains("\"seed\":2040"), "seed row:\n{manifest}");
        assert!(
            manifest.contains("\"tape\":\"script.tape\""),
            "tape row:\n{manifest}"
        );
        assert!(
            manifest.contains("\"capture_every\":4"),
            "cadence row:\n{manifest}"
        );
        assert!(manifest.contains("\"fps\":30"), "fps row:\n{manifest}");
        assert!(
            manifest.contains("\"rows\":[\"SHOW1\"]"),
            "board rows:\n{manifest}"
        );
        assert!(
            manifest.contains("\"sha\":\"deadbeef1234\""),
            "build sha:\n{manifest}"
        );
        assert!(
            manifest.contains(&format!("\"sequence_fnv\":\"{}\"", report.sequence_fnv)),
            "sha row:\n{manifest}"
        );
        assert!(
            manifest.contains("\"renderer\":\"headless-synth\""),
            "renderer:\n{manifest}"
        );
        assert_eq!(
            manifest.matches("\"tick\":").count(),
            12,
            "rows:\n{manifest}"
        );
        assert!(
            manifest.contains("\"file\":\"frame_000000.png\""),
            "row file:\n{manifest}"
        );
        assert_eq!(
            manifest.matches("\"sha256\":\"").count(),
            12,
            "per-file hashes:\n{manifest}"
        );
        // Every row file exists and parses as the manifest size.
        for index in 0..12u32 {
            let name = format!("frame_{index:06}.png");
            let png = std::fs::read(dir.join(&name)).unwrap();
            assert_eq!(parse_png_dims(&png).unwrap(), (64, 48), "{name}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn headless_capture_files_director_cuts() {
        let tape = LoadedTape {
            inputs: scripted_inputs(),
            file_name: "script.tape".to_string(),
            fnv: fnv_hex(&encode_tape_bytes(&scripted_inputs())),
        };
        let first_dir = temp_dir("director-a");
        let second_dir = temp_dir("director-b");
        let first = run_headless_capture(
            test_config(48),
            Some(&tape),
            &capture_request(first_dir.clone(), 48),
        )
        .unwrap();
        let second = run_headless_capture(
            test_config(48),
            Some(&tape),
            &capture_request(second_dir.clone(), 48),
        )
        .unwrap();
        // The session drove its camera from the same ticks: the receipt and
        // the manifest carry the cut-log digest and cut count, identically
        // twice.
        assert_eq!(first.director_digest, second.director_digest);
        assert_eq!(first.director_cuts, second.director_cuts);
        assert!(!first.director_digest.is_empty(), "digest must file");
        assert!(
            first.director_cuts > 0,
            "a scripted fight must earn cuts in capture"
        );
        for (dir, report) in [(&first_dir, &first), (&second_dir, &second)] {
            let manifest = std::fs::read_to_string(&report.manifest).unwrap();
            assert!(
                manifest.contains(&format!(
                    "\"director_digest\":\"{}\"",
                    report.director_digest
                )),
                "digest row:\n{manifest}"
            );
            assert!(
                manifest.contains(&format!("\"director_cuts\":{}", report.director_cuts)),
                "cuts row:\n{manifest}"
            );
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn capture_binding_passes_clean_and_fails_forged_cutlog() {
        let tape = LoadedTape {
            inputs: scripted_inputs(),
            file_name: "script.tape".to_string(),
            fnv: fnv_hex(&encode_tape_bytes(&scripted_inputs())),
        };
        let dir = temp_dir("binding");
        let report = run_headless_capture(
            test_config(48),
            Some(&tape),
            &capture_request(dir.clone(), 48),
        )
        .unwrap();
        assert!(report.director_cuts > 0, "the script must earn cuts");
        assert!(dir.join(CUTLOG_NAME).is_file(), "cuts must file");
        assert!(verify_capture_binding(&dir).is_ok(), "clean binds");
        // The ADV52 forgery: one cut tick bumped beside the sealed manifest.
        let cutlog_path = dir.join(CUTLOG_NAME);
        let cutlog = std::fs::read_to_string(&cutlog_path).unwrap();
        let key = "\"tick\":";
        let at = cutlog.find(key).expect("cuts file ticks");
        let digits: String = cutlog[at + key.len()..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .map(char::from)
            .collect();
        let tick: u32 = digits.parse().expect("tick parses");
        let forged = cutlog.replacen(&format!("{key}{tick}"), &format!("{key}{}", tick + 1), 1);
        assert_ne!(forged, cutlog);
        std::fs::write(&cutlog_path, forged.as_bytes()).unwrap();
        assert!(
            matches!(
                verify_capture_binding(&dir),
                Err(CaptureError::BadCutlog(_))
            ),
            "forged tick must fail the binding"
        );
        // A tampered manifest digest beside clean cuts fails the same way.
        std::fs::write(&cutlog_path, cutlog.as_bytes()).unwrap();
        assert!(verify_capture_binding(&dir).is_ok());
        let manifest_path = dir.join(MANIFEST_NAME);
        let manifest = std::fs::read_to_string(&manifest_path).unwrap();
        let tampered = manifest.replacen(&report.director_digest, "0000000000000000", 1);
        assert_ne!(tampered, manifest);
        std::fs::write(&manifest_path, tampered.as_bytes()).unwrap();
        assert!(
            matches!(
                verify_capture_binding(&dir),
                Err(CaptureError::BadCutlog(_))
            ),
            "tampered manifest digest must fail the binding"
        );
        // An unbound manifest (no digest rows, the ADV52 fixture shape)
        // beside a cut log still fails: the receipt is unbound, not clean.
        std::fs::write(&manifest_path, manifest.as_bytes()).unwrap();
        std::fs::remove_file(&cutlog_path).unwrap();
        assert!(
            matches!(
                verify_capture_binding(&dir),
                Err(CaptureError::BadCutlog(_))
            ),
            "missing cut log must fail the binding"
        );
        let unbound = temp_dir("binding-unbound");
        std::fs::write(unbound.join(MANIFEST_NAME), "{\"schema\":1,\"frames\":[]}").unwrap();
        std::fs::write(
            unbound.join(CUTLOG_NAME),
            "{\"cuts\":[{\"tick\":61}],\"digest\":\"forged\"}",
        )
        .unwrap();
        assert!(
            matches!(
                verify_capture_binding(&unbound),
                Err(CaptureError::BadCutlog(_))
            ),
            "unbound manifest must fail the binding"
        );
        std::fs::remove_dir_all(&unbound).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn capture_different_tapes_give_different_pixels() {
        let scripted = LoadedTape {
            inputs: scripted_inputs(),
            file_name: "script.tape".to_string(),
            fnv: fnv_hex(&encode_tape_bytes(&scripted_inputs())),
        };
        let idle = LoadedTape {
            inputs: Vec::new(),
            file_name: "idle.tape".to_string(),
            fnv: fnv_hex(&encode_tape_bytes(&[])),
        };
        let scripted_dir = temp_dir("bind-a");
        let idle_dir = temp_dir("bind-b");
        run_headless_capture(
            test_config(48),
            Some(&scripted),
            &capture_request(scripted_dir.clone(), 48),
        )
        .unwrap();
        run_headless_capture(
            test_config(48),
            Some(&idle),
            &capture_request(idle_dir.clone(), 48),
        )
        .unwrap();
        // Tape-bound frames: the scripted run must differ from idle somewhere.
        let mut differed = false;
        for index in 0..12u32 {
            let name = format!("frame_{index:06}.png");
            if std::fs::read(scripted_dir.join(&name)).unwrap()
                != std::fs::read(idle_dir.join(&name)).unwrap()
            {
                differed = true;
                break;
            }
        }
        assert!(differed, "tape inputs never reached the frames");
        // ... while the idle tape still matches a second idle run exactly.
        let idle_again = temp_dir("bind-c");
        run_headless_capture(
            test_config(48),
            Some(&idle),
            &capture_request(idle_again.clone(), 48),
        )
        .unwrap();
        for name in dir_files(&idle_dir) {
            assert_eq!(
                std::fs::read(idle_dir.join(&name)).unwrap(),
                std::fs::read(idle_again.join(&name)).unwrap(),
                "{name}"
            );
        }
        std::fs::remove_dir_all(&scripted_dir).unwrap();
        std::fs::remove_dir_all(&idle_dir).unwrap();
        std::fs::remove_dir_all(&idle_again).unwrap();
    }

    #[test]
    fn arena_scene_config_selects_8fighter_capture() {
        // The capture path can select the arena scene plus the
        // 8-fighter config review captures use.
        let config = arena8_capture_config(2040, 48).expect("arena8 builds");
        assert_eq!(
            (
                config.seed,
                config.ticks,
                config.fighters,
                config.half_extent_m
            ),
            (2040, 48, ARENA8_FIGHTERS, ARENA_CAPTURE_EXTENT_M)
        );
        assert_eq!(ARENA_CAPTURE_SCENE, "arena");
        let same = arena_capture_config("arena", 2040, 48, 8).expect("explicit 8 builds");
        assert_eq!((same.fighters, same.ticks), (8, 48));
        // The 8-fighter session runs the headless capture path end to end.
        let dir = temp_dir("arena8");
        let mut req = capture_request(dir.clone(), 16);
        req.rows = vec!["RL5".to_string()];
        let report = run_headless_capture(config, None, &req).expect("arena8 captures");
        assert_eq!(report.frames, 4);
        let manifest = std::fs::read_to_string(&report.manifest).unwrap();
        assert!(manifest.contains("\"rows\":[\"RL5\"]"), "rows:\n{manifest}");
        assert!(manifest.contains("\"seed\":2040"), "seed:\n{manifest}");
        // Failure edges: wrong scene and bad counts fail loudly.
        assert!(arena_capture_config("dungeon", 2040, 48, 8).is_err());
        assert!(arena_capture_config("arena", 2040, 48, 1).is_err());
        assert!(arena_capture_config("arena", 2040, 0, 8).is_err());
        assert!(arena_capture_config("arena", 2040, 48, 17).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn capture_rejects_malformed_tape_bytes() {
        let good = encode_tape_bytes(&scripted_inputs());
        assert_eq!(decode_tape_bytes(&good).unwrap(), scripted_inputs());
        // Empty tape is valid (idles every tick).
        assert!(decode_tape_bytes(&encode_tape_bytes(&[]))
            .unwrap()
            .is_empty());
        let mut bad_magic = good.clone();
        bad_magic[0] ^= 0xFF;
        assert!(decode_tape_bytes(&bad_magic).is_err());
        let mut bad_version = good.clone();
        bad_version[8] ^= 0xFF;
        assert!(decode_tape_bytes(&bad_version).is_err());
        assert!(decode_tape_bytes(&[]).is_err());
        assert!(decode_tape_bytes(&good[..HEADER_BYTES - 1]).is_err());
        assert!(decode_tape_bytes(&good[..good.len() - 1]).is_err());
        let mut trailing = good.clone();
        trailing.push(0);
        assert!(decode_tape_bytes(&trailing).is_err());
        // Unsorted and duplicate tapes fail.
        let mut unsorted = scripted_inputs();
        unsorted.swap(0, 5);
        assert!(decode_tape_bytes(&encode_tape_bytes(&unsorted)).is_err());
        let mut duplicate = scripted_inputs();
        duplicate[1] = duplicate[0];
        assert!(decode_tape_bytes(&encode_tape_bytes(&duplicate)).is_err());
        // Out-of-range wish and unknown button bits fail.
        let mut bad_wish = scripted_inputs();
        bad_wish[2].mx = 101;
        assert!(decode_tape_bytes(&encode_tape_bytes(&bad_wish)).is_err());
        let mut bad_buttons = scripted_inputs();
        bad_buttons[2].buttons = Buttons(0x08);
        assert!(decode_tape_bytes(&encode_tape_bytes(&bad_buttons)).is_err());
        // Unknown fighter slot fails.
        let mut bad_slot = good.clone();
        bad_slot[HEADER_BYTES + 4] = 16;
        assert!(decode_tape_bytes(&bad_slot).is_err());
        // Missing files fail with IO, never a panic.
        assert!(matches!(
            load_tape_file(Path::new("no-such-tape.tape")),
            Err(CaptureError::Io { .. })
        ));
    }

    #[test]
    fn capture_flags_split_and_reject_loudly() {
        let raw = [
            "arena",
            "--frames",
            "60",
            "--capture-dir",
            "out",
            "--capture-every",
            "4",
            "--sha",
            "deadbeef1234",
            "--exit",
        ]
        .iter()
        .map(|arg| (*arg).to_string())
        .collect::<Vec<_>>();
        let (flags, rest) = split_capture_args(&raw).unwrap();
        assert_eq!(
            flags,
            Some(CaptureFlags {
                dir: PathBuf::from("out"),
                every: 4,
                tape: None,
                rows: vec!["SHOW1".to_string()],
                sha: "deadbeef1234".to_string(),
                fps: 30,
            })
        );
        // WinArgs rows pass through untouched and in order.
        assert_eq!(rest, vec!["arena", "--frames", "60", "--exit"]);
        // Tape rides along from any position.
        let raw = [
            "--tape",
            "t.tape",
            "--capture-dir",
            "d",
            "--capture-every",
            "2",
            "--sha",
            "abc1234",
        ]
        .iter()
        .map(|arg| (*arg).to_string())
        .collect::<Vec<_>>();
        let (flags, rest) = split_capture_args(&raw).unwrap();
        assert_eq!(rest, Vec::<String>::new());
        assert_eq!(flags.unwrap().tape, Some(PathBuf::from("t.tape")));
        // No capture flags: nothing taken, nothing lost.
        let raw = ["arena", "--frames", "7"]
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>();
        assert_eq!(split_capture_args(&raw).unwrap(), (None, raw));
        // Loud rejections (never a silent default).
        for bad in [
            vec!["arena", "--capture-fps", "30"],
            vec!["arena", "--capture"],
            vec!["arena", "--capture-dir"],
            vec!["arena", "--capture-dir", "--frames", "60"],
            vec!["arena", "--capture-dir", ""],
            vec!["arena", "--capture-every", "4"],
            vec!["arena", "--tape", "t.tape"],
            vec!["arena", "--capture-dir", "d"],
            vec!["arena", "--capture-dir", "d", "--capture-every", "0"],
            vec!["arena", "--capture-dir", "d", "--capture-every", "abc"],
            vec!["arena", "--capture-dir", "d", "--capture-every", "100001"],
            vec![
                "arena",
                "--capture-dir",
                "a",
                "--capture-dir",
                "b",
                "--capture-every",
                "2",
            ],
            vec![
                "arena",
                "--capture-dir",
                "d",
                "--capture-every",
                "2",
                "--tape",
                "a",
                "--tape",
                "b",
            ],
            vec!["arena", "--capture-dir", "d", "--capture-every", "2"],
        ] {
            let bad: Vec<String> = bad.iter().map(|arg| (*arg).to_string()).collect();
            assert!(split_capture_args(&bad).is_err(), "args accepted: {bad:?}");
        }
        // Non-capture unknowns pass through: WinArgs still refuses them loudly.
        let raw = ["arena", "--nope"]
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>();
        let (flags, rest) = split_capture_args(&raw).unwrap();
        assert_eq!(flags, None);
        assert_eq!(rest, raw);
    }

    #[test]
    fn capture_headless_png_roundtrips_through_decoder() {
        let png = headless_frame_png(2040, 0x1234_5678_9ABC_DEF0, 7, 5, 3).unwrap();
        assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert_eq!(parse_png_dims(&png).unwrap(), (5, 3));
        let (width, height, pixels) = decode_test_png(&png).unwrap();
        assert_eq!((width, height), (5, 3));
        assert_eq!(pixels.len(), 5 * 3 * 4);
        // Opaque, non-black, high-variance frames (the play-session pixel check shape).
        assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
        let distinct: std::collections::BTreeSet<[u8; 3]> = pixels
            .chunks_exact(4)
            .map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .collect();
        assert!(distinct.len() > 10, "frames must vary: {}", distinct.len());
        assert!(
            distinct
                .iter()
                .any(|rgb| rgb[0] > 16 || rgb[1] > 16 || rgb[2] > 16),
            "frames must not be black"
        );
        // Same coordinates twice give the same bytes; a moved tick does not.
        assert_eq!(
            png,
            headless_frame_png(2040, 0x1234_5678_9ABC_DEF0, 7, 5, 3).unwrap()
        );
        assert_ne!(
            png,
            headless_frame_png(2040, 0x1234_5678_9ABC_DEF0, 8, 5, 3).unwrap()
        );
        // Bad sizes and pixel lengths fail instead of encoding garbage.
        assert!(headless_frame_png(2040, 0, 0, 0, 3).is_err());
        assert!(headless_frame_png(2040, 0, 0, 9999, 64).is_err());
        assert!(encode_png_rgba8(&[0u8; 7], 5, 3).is_err());
        assert!(parse_png_dims(b"not a png").is_err());
        assert!(parse_png_dims(&png[..20]).is_err());
    }

    /// Minimal decoder for the writer's own shape (stored blocks, filter 0):
    /// proves the PNGs are valid files, not just stable garbage.
    fn decode_test_png(png: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
        if png.len() < 33 || png[0..8] != [137, 80, 78, 71, 13, 10, 26, 10] {
            return Err("bad signature".to_string());
        }
        let (width, height) = parse_png_dims(png).map_err(|err| err.to_string())?;
        let mut cursor = 8usize;
        let mut idat = Vec::new();
        loop {
            if cursor + 8 > png.len() {
                return Err("truncated chunk".to_string());
            }
            let len = u32::from_be_bytes(png[cursor..cursor + 4].try_into().unwrap()) as usize;
            let tag = &png[cursor + 4..cursor + 8];
            let data = png
                .get(cursor + 8..cursor + 8 + len + 4)
                .ok_or_else(|| "truncated chunk data".to_string())?;
            if tag == b"IEND" {
                break;
            }
            if tag == b"IDAT" {
                idat.extend_from_slice(&data[..len]);
            }
            cursor += 8 + len + 4;
        }
        if idat.len() < 2 || idat[0..2] != [0x78, 0x01] {
            return Err("bad zlib header".to_string());
        }
        let mut raw = Vec::new();
        let mut at = 2usize;
        loop {
            if at + 5 > idat.len() {
                return Err("truncated block".to_string());
            }
            let last = idat[at] == 1;
            if idat[at] > 1 {
                return Err("compressed block unsupported".to_string());
            }
            let len = u16::from_le_bytes(idat[at + 1..at + 3].try_into().unwrap()) as usize;
            let nlen = u16::from_le_bytes(idat[at + 3..at + 5].try_into().unwrap());
            if nlen != !(len as u16) {
                return Err("bad block length".to_string());
            }
            raw.extend_from_slice(
                idat.get(at + 5..at + 5 + len)
                    .ok_or_else(|| "truncated block data".to_string())?,
            );
            at += 5 + len;
            if last {
                break;
            }
        }
        let want = adler32(&raw).to_be_bytes();
        if idat.get(at..at + 4) != Some(want.as_slice()) {
            return Err("bad adler".to_string());
        }
        let stride = width as usize * 4;
        let mut pixels = Vec::with_capacity(stride * height as usize);
        for row in raw.chunks_exact(stride + 1) {
            if row[0] != 0 {
                return Err("bad filter".to_string());
            }
            pixels.extend_from_slice(&row[1..]);
        }
        if pixels.len() != stride * height as usize {
            return Err("short pixels".to_string());
        }
        Ok((width, height, pixels))
    }

    #[test]
    fn capture_run_rejects_out_of_range_tape_before_writing() {
        // Ticks are validated before the directory is even created.
        let dir = std::env::temp_dir().join(format!("e2040-capture-never-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut over = scripted_inputs();
        over.push(FighterInput {
            tick: 48,
            ..idle_input(48)
        });
        let tape = LoadedTape {
            inputs: over,
            file_name: "over.tape".to_string(),
            fnv: "00".to_string(),
        };
        let req = capture_request(dir.clone(), 48);
        assert!(run_headless_capture(test_config(48), Some(&tape), &req).is_err());
        assert!(!dir.exists(), "rejected runs must write nothing");
        // Foreign slots fail the same way.
        let mut foreign = scripted_inputs();
        foreign[3].fighter = FighterId::new(1).unwrap();
        let tape = LoadedTape {
            inputs: foreign,
            file_name: "foreign.tape".to_string(),
            fnv: "00".to_string(),
        };
        assert!(run_headless_capture(test_config(48), Some(&tape), &req).is_err());
        assert!(!dir.exists(), "rejected runs must write nothing");
        // Frames past the session end fail too.
        let dir = temp_dir("past-end");
        let req = capture_request(dir.clone(), 49);
        assert!(run_headless_capture(test_config(48), None, &req).is_err());
        assert!(dir_files(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn capture_sink_finish_without_frames_writes_empty_manifest() {
        let dir = temp_dir("empty");
        let meta = ManifestMeta {
            rows: vec!["SHOW1".to_string()],
            sha: "deadbeef1234".to_string(),
            fps: 30,
        };
        let sink = CaptureSink::create(&dir, 2, 2040, None, HEADLESS_RENDERER, &meta).unwrap();
        assert!(!sink.should_capture(0));
        assert!(sink.should_capture(1));
        assert!(!sink.should_capture(2));
        assert!(sink.should_capture(3));
        let report = sink.finish().unwrap();
        assert_eq!(report.frames, 0);
        let manifest = std::fs::read_to_string(&report.manifest).unwrap();
        assert!(manifest.contains("\"frames\":[]"), "manifest:\n{manifest}");
        assert!(
            manifest.contains("\"tape\":\"live\""),
            "manifest:\n{manifest}"
        );
        assert_eq!(dir_files(&dir), vec!["manifest.json".to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn capture_manifest_names_the_renderer_that_drew_the_frames() {
        // The offscreen presenter is crates/render (GPU); the synth path is not.
        assert_eq!(TWIN_RENDERER, "crates/render");
        assert_ne!(TWIN_RENDERER, HEADLESS_RENDERER);
        let meta = ManifestMeta {
            rows: vec!["SHOW1".to_string()],
            sha: "deadbeef1234".to_string(),
            fps: 30,
        };
        for (name, label) in [("twin", TWIN_RENDERER), ("synth", HEADLESS_RENDERER)] {
            let dir = temp_dir(&format!("renderer-{name}"));
            let sink = CaptureSink::create(&dir, 1, 2040, None, label, &meta).unwrap();
            let report = sink.finish().unwrap();
            let manifest = std::fs::read_to_string(&report.manifest).unwrap();
            assert!(
                manifest.contains(&format!("\"renderer\":\"{label}\"")),
                "manifest:\n{manifest}"
            );
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn capture_write_frame_rejects_non_png_bytes() {
        let dir = temp_dir("badframe");
        let meta = ManifestMeta {
            rows: vec!["SHOW1".to_string()],
            sha: "deadbeef1234".to_string(),
            fps: 30,
        };
        let mut sink = CaptureSink::create(&dir, 1, 2040, None, HEADLESS_RENDERER, &meta).unwrap();
        assert!(sink.write_frame(0, 0, b"not a png").is_err());
        let png = headless_frame_png(2040, 9, 0, 8, 8).unwrap();
        assert!(sink.write_frame(0, 0, &png[..png.len() - 1]).is_err());
        assert!(sink.write_frame(0, 9, &png).is_ok());
        assert_eq!(sink.frames(), 1);
        let report = sink.finish().unwrap();
        assert_eq!(report.frames, 1);
        assert!(matches!(
            CaptureSink::create(&dir, 0, 2040, None, HEADLESS_RENDERER, &meta),
            Err(CaptureError::BadArgs(_))
        ));
        assert!(matches!(
            CaptureSink::create(&dir, 1, 2040, None, "", &meta),
            Err(CaptureError::BadArgs(_))
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn capture_hook_runs_once_per_validated_frame() {
        let dir = temp_dir("hook-once");
        let meta = ManifestMeta {
            rows: vec!["SHOW1".to_string()],
            sha: "deadbeef1234".to_string(),
            fps: 30,
        };
        let mut sink = CaptureSink::create(&dir, 1, 2040, None, HEADLESS_RENDERER, &meta).unwrap();
        let mut calls = 0u32;
        for tick in 0..3u32 {
            let png = headless_frame_png(2040, 9, tick, 8, 8).unwrap();
            let mut hook = |_: &[u8]| calls += 1;
            sink.write_frame_with_hook(tick, 9, &png, Some(&mut hook))
                .unwrap();
            assert_eq!(calls, tick + 1);
            assert_eq!(sink.frames(), tick + 1);
        }
        let mut hook = |_: &[u8]| calls += 1;
        assert!(sink
            .write_frame_with_hook(3, 9, b"not a png", Some(&mut hook))
            .is_err());
        assert_eq!(calls, 3);
        assert_eq!(sink.frames(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn capture_look_tap_notes_once_per_validated_frame() {
        let dir = temp_dir("look-tap");
        let meta = ManifestMeta {
            rows: vec!["SHOW1".to_string()],
            sha: "deadbeef1234".to_string(),
            fps: 30,
        };
        let mut sink = CaptureSink::create(&dir, 1, 2040, None, HEADLESS_RENDERER, &meta).unwrap();
        let mut tap = LookTap::new();
        assert_eq!(tap.calls(), 0);
        for tick in 0..3u32 {
            let png = headless_frame_png(2040, 9, tick, 8, 8).unwrap();
            sink.write_frame_with_hook(tick, 9, &png, Some(&mut |bytes| tap.note(bytes)))
                .unwrap();
            assert_eq!(tap.calls(), tick + 1);
            assert_eq!(sink.frames(), tick + 1);
        }
        assert!(sink
            .write_frame_with_hook(3, 9, b"not a png", Some(&mut |bytes| tap.note(bytes)))
            .is_err());
        assert_eq!(tap.calls(), 3);
        assert_eq!(sink.frames(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn look_tap_refuses_non_png_bytes() {
        let mut tap = LookTap::new();
        tap.note(b"not a png");
        assert_eq!(tap.calls(), 0, "non-PNG must not count");
        tap.note(b"");
        assert_eq!(tap.calls(), 0, "empty must not count");
        tap.note(&[137, 80, 78, 71, 13, 10, 26]);
        assert_eq!(tap.calls(), 0, "truncated magic must not count");
        let png = headless_frame_png(2040, 9, 0, 8, 8).expect("valid PNG");
        tap.note(&png);
        assert_eq!(tap.calls(), 1, "validated PNG counts once");
    }

    #[test]
    fn look_tap_counts_faults_on_refused_png() {
        let mut tap = LookTap::new();
        assert_eq!((tap.calls(), tap.faults()), (0, 0));
        tap.note(b"not a png");
        assert_eq!(tap.calls(), 0, "refusal adds no call");
        assert_eq!(tap.faults(), 1, "non-PNG counts one fault");
        tap.note(b"");
        assert_eq!(tap.calls(), 0, "empty adds no call");
        assert_eq!(tap.faults(), 2, "empty counts a second fault");
        tap.note(&[137, 80, 78, 71, 13, 10, 26]);
        assert_eq!(tap.calls(), 0, "truncated magic adds no call");
        assert_eq!(tap.faults(), 3, "truncated magic counts a third fault");
        let png = headless_frame_png(2040, 9, 0, 8, 8).expect("valid PNG");
        tap.note(&png);
        assert_eq!((tap.calls(), tap.faults()), (1, 3));
        let again = headless_frame_png(2040, 9, 0, 8, 8).expect("valid PNG");
        tap.note(&again);
        assert_eq!((tap.calls(), tap.faults()), (2, 3));
    }

    #[test]
    fn capture_window_tap_counts_once_per_frame() {
        // Window pin: `capture_offscreen` hands each validated PNG to
        // `tap.note` once via `write_frame_with_hook`, same as headless.
        let dir = temp_dir("window-tap");
        let meta = ManifestMeta {
            rows: vec!["SHOW1".to_string()],
            sha: "deadbeef1234".to_string(),
            fps: 30,
        };
        let mut sink = CaptureSink::create(&dir, 1, 2040, None, TWIN_RENDERER, &meta).unwrap();
        let mut tap = LookTap::new();
        for tick in 0..3u32 {
            let png = headless_frame_png(2040, 9, tick, 8, 8).expect("valid PNG");
            sink.write_frame_with_hook(tick, 9, &png, Some(&mut |bytes| tap.note(bytes)))
                .unwrap();
            assert_eq!(tap.calls(), tick + 1, "one tap call per window frame");
            assert_eq!(tap.faults(), 0, "validated frames add no fault");
            assert_eq!(sink.frames(), tick + 1, "one sink frame per write");
            assert_eq!(tap.calls(), sink.frames(), "tap tracks sink");
        }
        assert!(sink
            .write_frame_with_hook(3, 9, b"not a png", Some(&mut |bytes| tap.note(bytes)))
            .is_err());
        assert_eq!(tap.calls(), 3, "refusal adds no call");
        assert_eq!(sink.frames(), 3, "refusal adds no frame");
        tap.note(b"not a png");
        assert_eq!(tap.calls(), 3, "non-PNG still adds no call");
        assert_eq!(tap.faults(), 1, "non-PNG counts one fault");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn capture_errors_print_without_panicking() {
        for err in [
            CaptureError::BadArgs("x".to_string()),
            CaptureError::BadConfig("ticks"),
            CaptureError::BadTape("magic".to_string()),
            CaptureError::BadPng("sig".to_string()),
            CaptureError::Io {
                path: "dir".to_string(),
                kind: std::io::ErrorKind::NotFound,
            },
        ] {
            assert!(!format!("{err}").is_empty());
        }
    }

    fn flag_args(flags: &[&str]) -> Vec<String> {
        flags.iter().map(|flag| flag.to_string()).collect()
    }

    #[test]
    fn capture_flags_rows_sha_fps_parsed_and_refused() {
        // Full flag set parses; rows default to the built-in row, fps to 30.
        let (parsed, rest) = split_capture_args(&flag_args(&[
            "--capture-dir",
            "d",
            "--capture-every",
            "2",
            "--tape",
            "t.tape",
            "--rows",
            "SHOW1,WIN1",
            "--sha",
            "deadbeef1234",
            "--fps",
            "60",
            "--frames",
            "10",
        ]))
        .unwrap();
        let flags = parsed.expect("capture armed");
        assert_eq!(flags.rows, vec!["SHOW1".to_string(), "WIN1".to_string()]);
        assert_eq!(flags.sha, "deadbeef1234");
        assert_eq!(flags.fps, 60);
        assert_eq!(rest, vec!["--frames".to_string(), "10".to_string()]);
        // Defaults: built-in rows, fps 30; sha has no default.
        let (parsed, _) = split_capture_args(&flag_args(&[
            "--capture-dir",
            "d",
            "--capture-every",
            "2",
            "--sha",
            "abc1234",
        ]))
        .unwrap();
        let flags = parsed.expect("capture armed");
        assert_eq!(flags.rows, vec!["SHOW1".to_string()]);
        assert_eq!(flags.fps, 30);
        // Bad board ID, bad SHA, bad fps, missing sha all fail loudly.
        for args in [
            vec![
                "--capture-dir",
                "d",
                "--capture-every",
                "2",
                "--rows",
                "BOGUS!!",
                "--sha",
                "abc1234",
            ],
            vec!["--capture-dir", "d", "--capture-every", "2", "--sha", "xyz"],
            vec![
                "--capture-dir",
                "d",
                "--capture-every",
                "2",
                "--sha",
                "abc1234",
                "--fps",
                "0",
            ],
            vec!["--capture-dir", "d", "--capture-every", "2", "--fps", "30"],
            vec![
                "--capture-dir",
                "d",
                "--capture-every",
                "2",
                "--sha",
                "abc1234",
                "--sha",
                "abc1234",
            ],
        ] {
            assert!(
                split_capture_args(&flag_args(&args)).is_err(),
                "must refuse: {args:?}"
            );
        }
    }
}
