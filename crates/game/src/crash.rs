//! Crash reports: version, adapter, tape tail and checkpoint to a file.
//!
//! Owner: game crate; consumer is the windowed binary, which writes one
//! report then exits loudly (wiring: `pub mod crash;` in `lib.rs`). Units:
//! bytes for every size bound. Std-only with zero crate imports, so the
//! crash path never depends on the subsystem that just failed. Determinism:
//! formatting is pure over the report bytes; no wall clock, no RNG. Errors:
//! [`CrashError`] at every boundary; the writer never panics and the exit
//! code is never silent (0 is never emitted).

use std::collections::VecDeque;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError, TryLockError};
use std::time::{SystemTime, UNIX_EPOCH};

/// Max version string length in bytes (`"0.1.0+git-sha"` fits easily).
pub const MAX_VERSION_BYTES: usize = 256;
/// Max adapter description length in bytes.
pub const MAX_ADAPTER_BYTES: usize = 512;
/// Max retained input-tape tail in bytes (the last ~600 ticks of input log).
pub const MAX_TAPE_TAIL_BYTES: usize = 64 * 1024;
/// Max checkpoint snapshot in bytes.
pub const MAX_CHECKPOINT_BYTES: usize = 64 * 1024;
/// Max echoed bytes of an unknown mode token in [`CrashError::UnknownMode`].
pub const MAX_MODE_ECHO_BYTES: usize = 32;
/// Max display-mode echo bytes a report carries (`"borderless"` fits easily).
pub const MAX_DISPLAY_BYTES: usize = 32;
/// Max echoed bytes of an unknown field name in [`CrashError::UnknownField`].
pub const MAX_FIELD_ECHO_BYTES: usize = 32;
/// Max metric token bytes in a warning bundle directory name.
pub const MAX_METRIC_BYTES: usize = 64;

/// Base of the loud exit-code range; 0 (silent success) is never emitted.
const EXIT_CODE_BASE: i32 = 101;
/// Drill payload: tape-tail bytes written by [`drill_report`].
const DRILL_TAIL_LEN: usize = 1024;
/// Drill payload: checkpoint bytes written by [`drill_report`].
const DRILL_CHECKPOINT_LEN: usize = 64;

/// Game state the crash interrupted; matches the report schema.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CrashMode {
    /// Menus / settings / lobby screens.
    Menu,
    /// A live fight (fixed-tick match).
    Fight,
    /// Multiplayer lobby.
    Lobby,
}

impl CrashMode {
    /// Schema token for the `mode` row (`menu` | `fight` | `lobby`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Menu => "menu",
            Self::Fight => "fight",
            Self::Lobby => "lobby",
        }
    }

    /// Parse a schema token back; anything else fails loudly.
    pub fn parse(text: &str) -> Result<Self, CrashError> {
        match text {
            "menu" => Ok(Self::Menu),
            "fight" => Ok(Self::Fight),
            "lobby" => Ok(Self::Lobby),
            _ => Err(CrashError::UnknownMode(clip_prefix(
                text,
                MAX_MODE_ECHO_BYTES,
            ))),
        }
    }
}

/// Crash-report failures: bounded diagnostics, never panics.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum CrashError {
    /// Version string is empty; a report without a version teaches nothing.
    EmptyVersion,
    /// Version string exceeds [`MAX_VERSION_BYTES`]; lengths are bytes.
    VersionTooLong { len: usize, max: usize },
    /// Adapter string exceeds [`MAX_ADAPTER_BYTES`]; lengths are bytes.
    AdapterTooLong { len: usize, max: usize },
    /// Tape tail exceeds [`MAX_TAPE_TAIL_BYTES`]; lengths are bytes.
    TapeTailTooLarge { len: usize, max: usize },
    /// Checkpoint exceeds [`MAX_CHECKPOINT_BYTES`]; lengths are bytes.
    CheckpointTooLarge { len: usize, max: usize },
    /// Display-mode echo exceeds [`MAX_DISPLAY_BYTES`]; lengths are bytes.
    DisplayTooLong { len: usize, max: usize },
    /// The settings the caller holds are from another run; the report carries
    /// the digest it was written under (`want`), the caller offered `got`.
    SettingsDigestMismatch { want: u64, got: u64 },
    /// `mode` row is not `menu` | `fight` | `lobby`; echo clipped to
    /// [`MAX_MODE_ECHO_BYTES`] bytes at a char boundary.
    UnknownMode(String),
    /// Report carries a field outside the `[crash]` schema; echo clipped to
    /// [`MAX_FIELD_ECHO_BYTES`] bytes at a char boundary.
    UnknownField(String),
    /// Report misses a required schema row; row name carried.
    MissingField(&'static str),
    /// Report row is present but malformed; row name carried.
    BadValue(&'static str),
    /// Filesystem failure, kind carried (paths stay in the caller).
    Io(std::io::ErrorKind),
}

impl fmt::Display for CrashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyVersion => write!(f, "crash report needs a version"),
            Self::VersionTooLong { len, max } => {
                write!(f, "crash version too long ({len} bytes, max {max})")
            }
            Self::AdapterTooLong { len, max } => {
                write!(f, "crash adapter too long ({len} bytes, max {max})")
            }
            Self::TapeTailTooLarge { len, max } => {
                write!(f, "crash tape tail too large ({len} bytes, max {max})")
            }
            Self::CheckpointTooLarge { len, max } => {
                write!(f, "crash checkpoint too large ({len} bytes, max {max})")
            }
            Self::DisplayTooLong { len, max } => {
                write!(f, "crash display echo too long ({len} bytes, max {max})")
            }
            Self::SettingsDigestMismatch { want, got } => write!(
                f,
                "crash settings digest mismatch (report has {want:016x}, caller has {got:016x})"
            ),
            Self::UnknownMode(echo) => write!(f, "unknown crash mode ({echo})"),
            Self::UnknownField(field) => write!(f, "unknown crash field ({field})"),
            Self::MissingField(row) => write!(f, "crash report missing field ({row})"),
            Self::BadValue(row) => write!(f, "crash report bad value ({row})"),
            Self::Io(kind) => write!(f, "crash report io error ({kind:?})"),
        }
    }
}

impl std::error::Error for CrashError {}

/// Plain-data crash bundle: version, adapter, tape tail and checkpoint.
/// The caller owns truncation (see [`tail_window`]); the writer refuses
/// oversized bundles loudly instead of silently trimming them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CrashReport {
    /// Build version, e.g. `"0.1.0+git-sha"`; non-empty, bytes bounded.
    pub version: String,
    /// Adapter description, e.g. `"NVIDIA RTX 3050 6GB Laptop (driver x.y)"`.
    pub adapter: String,
    /// Game state interrupted by the crash.
    pub mode: CrashMode,
    /// Fixed-tick cursor at the crash.
    pub tick: u32,
    /// Digest of the settings tape in force at the crash (`engine_ui`'s
    /// `SettingsTape::digest`, recorded by [`note_settings`]); `0` when the
    /// caller carried no settings.
    pub settings_digest: u64,
    /// Display-mode echo, e.g. `"borderless"`; bytes bounded by
    /// [`MAX_DISPLAY_BYTES`], may be empty.
    pub display: String,
    /// Last input-log bytes (a suffix window of the tape).
    pub tape_tail: Vec<u8>,
    /// Checkpoint snapshot bytes at [`CrashReport::tick`].
    pub checkpoint: Vec<u8>,
}

impl CrashReport {
    /// Build and validate a bundle; oversized fields fail loudly.
    pub fn new(
        version: String,
        adapter: String,
        mode: CrashMode,
        tick: u32,
        tape_tail: Vec<u8>,
        checkpoint: Vec<u8>,
    ) -> Result<Self, CrashError> {
        let report = Self {
            version,
            adapter,
            mode,
            tick,
            settings_digest: 0,
            display: String::new(),
            tape_tail,
            checkpoint,
        };
        report.validate().map(|()| report)
    }

    /// The same bundle with the settings state beside it: the settings tape's
    /// digest and the display-mode echo. Validates after the change, so an
    /// oversized echo fails loudly and nothing reaches the disk.
    ///
    /// Errors: [`CrashError::DisplayTooLong`] when the echo passes
    /// [`MAX_DISPLAY_BYTES`].
    pub fn with_settings(
        mut self,
        settings_digest: u64,
        display: String,
    ) -> Result<Self, CrashError> {
        self.settings_digest = settings_digest;
        self.display = display;
        self.validate().map(|()| self)
    }

    /// The pre-crash tick this bundle regenerates, once `settings_digest` is
    /// the digest the report was written under: the checkpoint's own
    /// `(tick, hash)` record must agree with the report's `tick`.
    ///
    /// The caller holds the settings; a digest from another run is refused
    /// loudly instead of being replayed against the wrong settings. Only a real
    /// checkpoint record regenerates: a short record, the drill's synthetic
    /// payload and a record whose tick disagrees all fail loudly.
    ///
    /// Errors: [`CrashError::SettingsDigestMismatch`] for a foreign digest,
    /// [`CrashError::BadValue`]`("checkpoint")` for a checkpoint that cannot
    /// regenerate the tick.
    pub fn regenerate_tick(&self, settings_digest: u64) -> Result<u32, CrashError> {
        if settings_digest != self.settings_digest {
            return Err(CrashError::SettingsDigestMismatch {
                want: self.settings_digest,
                got: settings_digest,
            });
        }
        if self.checkpoint.len() != CHECKPOINT_RECORD_BYTES {
            return Err(CrashError::BadValue("checkpoint"));
        }
        let mut tick_bytes = [0u8; 4];
        tick_bytes.copy_from_slice(&self.checkpoint[..4]);
        let tick = u32::from_le_bytes(tick_bytes);
        if tick != self.tick {
            return Err(CrashError::BadValue("checkpoint"));
        }
        Ok(tick)
    }

    /// Check every bound; the writer runs this before touching the disk.
    pub fn validate(&self) -> Result<(), CrashError> {
        if self.version.is_empty() {
            return Err(CrashError::EmptyVersion);
        }
        if self.version.len() > MAX_VERSION_BYTES {
            return Err(CrashError::VersionTooLong {
                len: self.version.len(),
                max: MAX_VERSION_BYTES,
            });
        }
        if self.adapter.len() > MAX_ADAPTER_BYTES {
            return Err(CrashError::AdapterTooLong {
                len: self.adapter.len(),
                max: MAX_ADAPTER_BYTES,
            });
        }
        if self.tape_tail.len() > MAX_TAPE_TAIL_BYTES {
            return Err(CrashError::TapeTailTooLarge {
                len: self.tape_tail.len(),
                max: MAX_TAPE_TAIL_BYTES,
            });
        }
        if self.checkpoint.len() > MAX_CHECKPOINT_BYTES {
            return Err(CrashError::CheckpointTooLarge {
                len: self.checkpoint.len(),
                max: MAX_CHECKPOINT_BYTES,
            });
        }
        if self.display.len() > MAX_DISPLAY_BYTES {
            return Err(CrashError::DisplayTooLong {
                len: self.display.len(),
                max: MAX_DISPLAY_BYTES,
            });
        }
        Ok(())
    }
}

/// Kind of crash, for the loud exit-code mapping.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CrashKind {
    /// Operator/CI forced-crash drill (acceptance: a readable report lands).
    ForcedDrill,
    /// Unhandled panic caught at the top-level hook.
    Panic,
    /// Simulation state failed validation; continuing would desync.
    StateCorrupt,
    /// The report itself could not be written; stderr still says so.
    ReportFailed,
}

/// Loud exit code for a crash kind; never 0, distinct per kind.
#[must_use]
pub fn exit_code(kind: CrashKind) -> i32 {
    match kind {
        CrashKind::ForcedDrill => EXIT_CODE_BASE,
        CrashKind::Panic => EXIT_CODE_BASE + 1,
        CrashKind::StateCorrupt => EXIT_CODE_BASE + 2,
        CrashKind::ReportFailed => EXIT_CODE_BASE + 3,
    }
}

/// Loud exit code for a report failure; never 0, distinct per failure.
#[must_use]
pub fn error_exit_code(err: &CrashError) -> i32 {
    match err {
        CrashError::EmptyVersion | CrashError::VersionTooLong { .. } => EXIT_CODE_BASE + 10,
        CrashError::AdapterTooLong { .. } => EXIT_CODE_BASE + 11,
        CrashError::TapeTailTooLarge { .. } => EXIT_CODE_BASE + 12,
        CrashError::CheckpointTooLarge { .. } => EXIT_CODE_BASE + 13,
        CrashError::UnknownMode(_) => EXIT_CODE_BASE + 14,
        CrashError::UnknownField(_) => EXIT_CODE_BASE + 16,
        CrashError::MissingField(_) => EXIT_CODE_BASE + 17,
        CrashError::BadValue(_) => EXIT_CODE_BASE + 18,
        CrashError::DisplayTooLong { .. } => EXIT_CODE_BASE + 19,
        CrashError::SettingsDigestMismatch { .. } => EXIT_CODE_BASE + 20,
        CrashError::Io(_) => EXIT_CODE_BASE + 15,
    }
}

/// Suffix window of `bytes` capped at `max` bytes; never panics.
/// Units are bytes on both sides.
#[must_use]
pub fn tail_window(bytes: &[u8], max: usize) -> &[u8] {
    let start = bytes.len().saturating_sub(max);
    &bytes[start..]
}

/// Deterministic drill bundle: same inputs give the same bytes, so the
/// forced-crash drill is reproducible. Tape tail and checkpoint are
/// synthetic tick-derived patterns, not a real match.
pub fn drill_report(
    version: &str,
    adapter: &str,
    mode: CrashMode,
    tick: u32,
) -> Result<CrashReport, CrashError> {
    let mut tape_tail = Vec::with_capacity(DRILL_TAIL_LEN);
    for i in 0..DRILL_TAIL_LEN {
        tape_tail.push((tick.wrapping_add(i as u32) as u8) ^ 0x5A);
    }
    let mut checkpoint = Vec::with_capacity(DRILL_CHECKPOINT_LEN);
    for i in 0..DRILL_CHECKPOINT_LEN {
        checkpoint.push((tick.wrapping_add(i as u32) as u8) ^ 0xA5);
    }
    CrashReport::new(
        version.to_string(),
        adapter.to_string(),
        mode,
        tick,
        tape_tail,
        checkpoint,
    )
}

/// Render the `[crash]` TOML document; validates first, never panics.
pub fn format_report(report: &CrashReport) -> Result<String, CrashError> {
    report.validate()?;
    let mut out = String::new();
    out.push_str("[crash]\nversion = ");
    append_toml_string(&mut out, &report.version);
    out.push_str("\nadapter = ");
    append_toml_string(&mut out, &report.adapter);
    out.push_str("\nmode = \"");
    out.push_str(report.mode.as_str());
    out.push_str("\"\ndisplay = ");
    append_toml_string(&mut out, &report.display);
    out.push_str("\ntick = ");
    out.push_str(&report.tick.to_string());
    out.push_str("\nsettings_digest = \"");
    out.push_str(&format!("{:016x}", report.settings_digest));
    out.push_str("\"\ncheckpoint = \"");
    append_hex(&mut out, &report.checkpoint);
    out.push_str("\"\ntape_tail = \"");
    append_hex(&mut out, &report.tape_tail);
    out.push_str("\"\n");
    Ok(out)
}

/// Parse a `[crash]` TOML document back into a [`CrashReport`].
///
/// Strict schema: exactly `version`, `adapter`, `mode`, `display`, `tick`,
/// `settings_digest`, `checkpoint` and `tape_tail` under one `[crash]` header,
/// in any order.
/// Unknown fields fail with [`CrashError::UnknownField`], missing rows with
/// [`CrashError::MissingField`], malformed rows with [`CrashError::BadValue`]
/// (bad `mode` keeps [`CrashError::UnknownMode`]) and oversized bundles keep
/// the [`CrashReport::validate`] errors. Units: bytes for every size bound.
/// Determinism: pure over the text bytes; no wall clock, no RNG. Errors:
/// [`CrashError`] at every boundary; never panics.
pub fn parse_report(text: &str) -> Result<CrashReport, CrashError> {
    let mut version: Option<String> = None;
    let mut adapter: Option<String> = None;
    let mut mode_text: Option<String> = None;
    let mut display_text: Option<String> = None;
    let mut tick_text: Option<String> = None;
    let mut settings_digest_text: Option<String> = None;
    let mut checkpoint_text: Option<String> = None;
    let mut tape_tail_text: Option<String> = None;
    let mut header_seen = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line == "[crash]" {
            header_seen = true;
            continue;
        }
        if !header_seen {
            return Err(CrashError::BadValue("header"));
        }
        let (key, value) = split_row(line)?;
        match key {
            "version" => take_string(&mut version, value, "version")?,
            "adapter" => take_string(&mut adapter, value, "adapter")?,
            "mode" => take_string(&mut mode_text, value, "mode")?,
            "display" => take_string(&mut display_text, value, "display")?,
            "settings_digest" => {
                take_string(&mut settings_digest_text, value, "settings_digest")?;
            }
            "tick" => {
                if tick_text.is_some() {
                    return Err(CrashError::BadValue("tick"));
                }
                tick_text = Some(value.to_string());
            }
            "checkpoint" => take_string(&mut checkpoint_text, value, "checkpoint")?,
            "tape_tail" => take_string(&mut tape_tail_text, value, "tape_tail")?,
            _ => {
                return Err(CrashError::UnknownField(clip_prefix(
                    key,
                    MAX_FIELD_ECHO_BYTES,
                )));
            }
        }
    }
    if !header_seen {
        return Err(CrashError::BadValue("header"));
    }
    let version = version.ok_or(CrashError::MissingField("version"))?;
    let adapter = adapter.ok_or(CrashError::MissingField("adapter"))?;
    let mode_text = mode_text.ok_or(CrashError::MissingField("mode"))?;
    let display = display_text.ok_or(CrashError::MissingField("display"))?;
    let tick_text = tick_text.ok_or(CrashError::MissingField("tick"))?;
    let settings_text = settings_digest_text.ok_or(CrashError::MissingField("settings_digest"))?;
    let checkpoint_text = checkpoint_text.ok_or(CrashError::MissingField("checkpoint"))?;
    let tape_tail_text = tape_tail_text.ok_or(CrashError::MissingField("tape_tail"))?;
    let mode = CrashMode::parse(&mode_text)?;
    let tick: u32 = tick_text
        .trim()
        .parse()
        .map_err(|_| CrashError::BadValue("tick"))?;
    let settings_digest = u64::from_str_radix(&settings_text, 16)
        .map_err(|_| CrashError::BadValue("settings_digest"))?;
    let checkpoint = decode_hex(&checkpoint_text, "checkpoint")?;
    let tape_tail = decode_hex(&tape_tail_text, "tape_tail")?;
    CrashReport::new(version, adapter, mode, tick, tape_tail, checkpoint)?
        .with_settings(settings_digest, display)
}

/// Split one `key = value` row; surrounding whitespace is trimmed.
fn split_row(line: &str) -> Result<(&str, &str), CrashError> {
    let (key, value) = line.split_once('=').ok_or(CrashError::BadValue("row"))?;
    Ok((key.trim(), value.trim()))
}

/// Record one string row into its slot: the first copy is parsed, a second is
/// a [`CrashError::BadValue`] naming the row.
fn take_string(
    slot: &mut Option<String>,
    value: &str,
    row: &'static str,
) -> Result<(), CrashError> {
    if slot.is_some() {
        return Err(CrashError::BadValue(row));
    }
    *slot = Some(parse_toml_string(value, row)?);
    Ok(())
}

/// Unquote one TOML basic string, inverting [`append_toml_string`].
fn parse_toml_string(value: &str, row: &'static str) -> Result<String, CrashError> {
    let inner = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .ok_or(CrashError::BadValue(row))?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let esc = chars.next().ok_or(CrashError::BadValue(row))?;
        match esc {
            '"' => out.push('"'),
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'u' => {
                let mut code: u32 = 0;
                for _ in 0..4 {
                    let digit = chars.next().ok_or(CrashError::BadValue(row))?;
                    let byte = u8::try_from(digit).map_err(|_| CrashError::BadValue(row))?;
                    let val = hex_byte(byte).ok_or(CrashError::BadValue(row))?;
                    code = code
                        .checked_mul(16)
                        .and_then(|acc| acc.checked_add(u32::from(val)))
                        .ok_or(CrashError::BadValue(row))?;
                }
                let decoded = char::from_u32(code).ok_or(CrashError::BadValue(row))?;
                out.push(decoded);
            }
            _ => return Err(CrashError::BadValue(row)),
        }
    }
    Ok(out)
}

/// Decode lowercase or uppercase hex into raw bytes; odd length fails.
fn decode_hex(text: &str, row: &'static str) -> Result<Vec<u8>, CrashError> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err(CrashError::BadValue(row));
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut index = 0;
    while index < bytes.len() {
        let hi = hex_byte(bytes[index]).ok_or(CrashError::BadValue(row))?;
        let lo = hex_byte(bytes[index + 1]).ok_or(CrashError::BadValue(row))?;
        out.push(hi.wrapping_shl(4).wrapping_add(lo));
        index += 2;
    }
    Ok(out)
}

/// One hex digit value for an ASCII byte; `None` for anything else.
fn hex_byte(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte.wrapping_sub(b'0')),
        b'a'..=b'f' => Some(byte.wrapping_sub(b'a').wrapping_add(10)),
        b'A'..=b'F' => Some(byte.wrapping_sub(b'A').wrapping_add(10)),
        _ => None,
    }
}

/// Write the TOML report to a caller-provided path. Validates before
/// touching the disk (a bad bundle creates no file); IO failures map to
/// [`CrashError::Io`]. No panic paths.
pub fn write_report(path: &Path, report: &CrashReport) -> Result<(), CrashError> {
    let text = format_report(report)?;
    std::fs::write(path, text.as_bytes()).map_err(|err| CrashError::Io(err.kind()))?;
    Ok(())
}

/// Facts one warning bundle carries; formatted by the two private writers.
struct BundleFacts<'a> {
    metric: &'a str,
    stamp: &'a str,
    seed: u64,
    ticks: u32,
    ticks_done: u32,
    fighters: u8,
    view_line: &'a str,
    warnings: &'a [String],
    adapter: &'a str,
    display: &'a str,
    checkpoint: Option<(u32, u64)>,
    tape_bytes: usize,
}

/// One token that is safe as a directory name: lowercase ASCII, digits and
/// `_`, non-empty and bounded by [`MAX_METRIC_BYTES`].
fn safe_metric(metric: &str) -> bool {
    !metric.is_empty()
        && metric.len() <= MAX_METRIC_BYTES
        && metric
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// The bundle's `report.toml`: a `[repro]` table with the exact view receipt,
/// every warning line, the run's seed and ticks, adapter, display and the
/// checkpoint record, plus the tape's name and size.
fn format_bundle_report(facts: &BundleFacts<'_>) -> String {
    let mut out = String::from("[repro]\nmetric = ");
    append_toml_string(&mut out, facts.metric);
    out.push_str("\nutc = ");
    append_toml_string(&mut out, facts.stamp);
    out.push_str(&format!(
        "\nseed = {}\nticks = {}\nticks_done = {}\nfighters = {}\n",
        facts.seed, facts.ticks, facts.ticks_done, facts.fighters
    ));
    out.push_str("view = ");
    append_toml_string(&mut out, facts.view_line);
    out.push_str("\nwarnings = [");
    for (index, warning) in facts.warnings.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        append_toml_string(&mut out, warning);
    }
    out.push_str("]\nadapter = ");
    append_toml_string(&mut out, facts.adapter);
    out.push_str("\ndisplay = ");
    append_toml_string(&mut out, facts.display);
    out.push_str("\ncheckpoint = ");
    match facts.checkpoint {
        Some((tick, hash)) => out.push_str(&format!("\"{tick}:{hash:016x}\"")),
        None => out.push_str("\"none\""),
    }
    out.push_str(&format!(
        "\ntape = \"tape.bin\"\ntape_bytes = {}\n",
        facts.tape_bytes
    ));
    out
}

/// The bundle's `README.txt`: what each file holds and the rerun line.
fn format_bundle_readme(facts: &BundleFacts<'_>) -> String {
    let checkpoint = match facts.checkpoint {
        Some((tick, hash)) => format!("tick {tick}, hash {hash:016x}"),
        None => "none (no checkpoint recorded)".to_string(),
    };
    let mut out = format!(
        "engine2040 view-warning repro bundle\nmetric: {}\nutc: {}\nseed: {}\nticks: {} (ran {})\nfighters: {}\n\nview: {}\n\nwarnings:\n",
        facts.metric, facts.stamp, facts.seed, facts.ticks, facts.ticks_done, facts.fighters,
        facts.view_line,
    );
    for warning in facts.warnings {
        out.push_str("  ");
        out.push_str(warning);
        out.push('\n');
    }
    out.push_str(&format!(
        "\ncheckpoint: {checkpoint}\ntape: tape.bin ({} bytes; the crash tape tail, raw canonical 10-byte FighterInput records, oldest first)\n\nrerun the same bout:\n  arena.exe --seed {} --ticks {} --fighters {} --frames {} --exit\n\nThe crash-report equivalent: report.toml carries these facts; tape.bin is the\nsame bytes a crash-<UTC>.toml carries as tape_tail hex.\n",
        facts.tape_bytes, facts.seed, facts.ticks, facts.fighters, facts.ticks,
    ));
    out
}

/// Write one self-contained repro bundle for the first broken view floor:
/// `<dir>/<metric>-<utc-stamp>/` holding `report.toml` (seed, planned and run
/// ticks, the exact `arena view:` line, every `arena WARN:` line, adapter,
/// display echo, checkpoint tick and hash), `tape.bin` (the crash tape tail,
/// raw canonical 10-byte `FighterInput` records, oldest first) and
/// `README.txt` (what each file holds plus the rerun line).
///
/// View only: nothing here reaches the simulation or its hash. The caller
/// writes once, when the first warning is known, so a run leaves one bundle.
/// Errors: [`CrashError::BadValue`]`("metric")` for an unsafe directory token,
/// [`CrashError::Io`] for a failed directory or file; the caller keeps its own
/// exit code and only logs the refusal.
#[expect(
    clippy::too_many_arguments,
    reason = "the bundle's fields mirror report.toml one to one"
)]
pub fn write_warning_bundle(
    dir: &Path,
    metric: &str,
    seed: u64,
    ticks: u32,
    ticks_done: u32,
    fighters: u8,
    view_line: &str,
    warnings: &[String],
) -> Result<PathBuf, CrashError> {
    if !safe_metric(metric) {
        return Err(CrashError::BadValue("metric"));
    }
    let stamp = utc_stamp(SystemTime::now());
    let (adapter, display, checkpoint, tape) = match try_lock_or_recover(&STATE) {
        Some(state) => {
            let adapter = if state.adapter.is_empty() {
                "no GPU adapter (window not presented)".to_string()
            } else {
                state.adapter.clone()
            };
            let checkpoint = if state.checkpoint.len() == CHECKPOINT_RECORD_BYTES {
                let mut tick_bytes = [0u8; 4];
                tick_bytes.copy_from_slice(&state.checkpoint[..4]);
                let mut hash_bytes = [0u8; 8];
                hash_bytes.copy_from_slice(&state.checkpoint[4..12]);
                Some((
                    u32::from_le_bytes(tick_bytes),
                    u64::from_le_bytes(hash_bytes),
                ))
            } else {
                None
            };
            (
                adapter,
                state.display.clone(),
                checkpoint,
                state.tape.iter().copied().collect::<Vec<u8>>(),
            )
        }
        None => (
            "unknown (snapshot busy at warning)".to_string(),
            String::new(),
            None,
            Vec::new(),
        ),
    };
    let facts = BundleFacts {
        metric,
        stamp: &stamp,
        seed,
        ticks,
        ticks_done,
        fighters,
        view_line,
        warnings,
        adapter: &adapter,
        display: &display,
        checkpoint,
        tape_bytes: tape.len(),
    };
    let bundle = dir.join(format!("{metric}-{stamp}"));
    std::fs::create_dir_all(&bundle).map_err(|err| CrashError::Io(err.kind()))?;
    std::fs::write(bundle.join("report.toml"), format_bundle_report(&facts))
        .map_err(|err| CrashError::Io(err.kind()))?;
    std::fs::write(bundle.join("tape.bin"), &tape).map_err(|err| CrashError::Io(err.kind()))?;
    std::fs::write(bundle.join("README.txt"), format_bundle_readme(&facts))
        .map_err(|err| CrashError::Io(err.kind()))?;
    Ok(bundle)
}

// PORT(EmbarkStudios/crash-handling@e7d5067:crash-handler, minidumper): fault
// hook writes one stub below; the monitor owns the real minidump (`std` only).

/// Sibling `.dmp` path for a `.toml` report path. Pure.
fn minidump_path(report_path: &Path) -> PathBuf {
    report_path.with_extension("dmp")
}

/// Write the fault-path stub dump beside the report (magic, tick, version
/// length) in one write call; a bad bundle leaves neither file.
fn write_minidump_stub(path: &Path, report: &CrashReport) -> Result<PathBuf, CrashError> {
    report.validate()?;
    let dump = minidump_path(path);
    let mut header = Vec::with_capacity(16);
    header.extend_from_slice(b"MDMP");
    header.extend_from_slice(&report.tick.to_le_bytes());
    let version_len = u32::try_from(report.version.len()).unwrap_or(u32::MAX);
    header.extend_from_slice(&version_len.to_le_bytes());
    std::fs::write(&dump, &header).map_err(|err| CrashError::Io(err.kind()))?;
    Ok(dump)
}

/// Char-boundary-safe byte prefix of `text` capped at `max` bytes.
fn clip_prefix(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// TOML basic-string escaping for one value; control rows use `\uXXXX`.
fn append_toml_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if u32::from(ch) < 0x20 || u32::from(ch) == 0x7F => {
                out.push_str(&format!("\\u{:04X}", u32::from(ch)));
            }
            _ => out.push(ch),
        }
    }
    out.push('"');
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// Lowercase hex of raw bytes; indexes are 4-bit masked, so no panic.
fn append_hex(out: &mut String, bytes: &[u8]) {
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0F)]));
    }
}

/// Crate version stamped into every crash report.
const BUILD_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Name of the process main thread; only its panics are fatal.
const MAIN_THREAD: &str = "main";
/// Bytes of one checkpoint record: tick (u32, little-endian) then hash (u64).
const CHECKPOINT_RECORD_BYTES: usize = 12;
/// Env var that overrides the run-log and crash-report directory.
pub const LOG_DIR_ENV: &str = "E2040_LOG_DIR";

/// Paths of one run; both carry the run's UTC start stamp.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunPaths {
    /// Status log: every line passed to [`mirror_line`], appended as it happens.
    pub log: PathBuf,
    /// Crash report the hook writes if this run panics.
    pub report: PathBuf,
}

/// Sim facts the hook reads at crash time. Written only by the tick path
/// through [`record_fight_tick`] and [`set_adapter`]; never read by the sim.
struct HookState {
    adapter: String,
    mode: CrashMode,
    tick: u32,
    settings_digest: u64,
    display: String,
    tape: VecDeque<u8>,
    checkpoint: Vec<u8>,
}

static STATE: Mutex<HookState> = Mutex::new(HookState {
    adapter: String::new(),
    mode: CrashMode::Menu,
    tick: 0,
    settings_digest: 0,
    display: String::new(),
    tape: VecDeque::new(),
    checkpoint: Vec::new(),
});
static LOG: Mutex<Option<File>> = Mutex::new(None);
static RUN: OnceLock<RunPaths> = OnceLock::new();

/// Install the crash hook, then open this run's log file. Call first in
/// `main`. The hook is set before anything can fail, so a log that cannot be
/// opened still leaves panics loud (the report then goes to the temp dir).
/// Errors: [`CrashError::Io`] for the directory or log, `BadValue("install")`
/// on a second call. Never panics.
pub fn install_run_hook(dir: &Path) -> Result<RunPaths, CrashError> {
    if RUN.get().is_some() {
        return Err(CrashError::BadValue("install"));
    }
    std::panic::set_hook(Box::new(on_panic));
    std::fs::create_dir_all(dir).map_err(|err| CrashError::Io(err.kind()))?;
    let stamp = utc_stamp(SystemTime::now());
    let paths = RunPaths {
        log: dir.join(format!("arena-{stamp}.log")),
        report: dir.join(format!("crash-{stamp}.toml")),
    };
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.log)
        .map_err(|err| CrashError::Io(err.kind()))?;
    *LOG.lock().unwrap_or_else(PoisonError::into_inner) = Some(file);
    let _ = RUN.set(paths.clone());
    Ok(paths)
}

/// Where logs and crash reports go: `E2040_LOG_DIR` when set, else
/// `target/logs` when the working directory has a `target/`, else
/// `%LOCALAPPDATA%` (or `$HOME`) `/engine2040/logs`.
#[must_use]
pub fn default_log_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(LOG_DIR_ENV) {
        return PathBuf::from(dir);
    }
    if Path::new("target").is_dir() {
        return Path::new("target").join("logs");
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .map_or_else(std::env::temp_dir, PathBuf::from);
    base.join("engine2040").join("logs")
}

/// Fold one fight tick into the crash snapshot: the tape tail gains this
/// tick's canonical input bytes (oldest bytes drop past
/// [`MAX_TAPE_TAIL_BYTES`]) and the checkpoint becomes `(tick, hash)` when
/// the sim has one. `cursor` is the fixed-tick cursor after the tick. Called
/// once per tick on the sim thread; no per-tick allocation after the tape
/// first fills.
pub fn record_fight_tick(cursor: u32, input_bytes: &[u8], checkpoint: Option<(u32, u64)>) {
    let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    state.mode = CrashMode::Fight;
    state.tick = cursor;
    state.tape.extend(input_bytes.iter().copied());
    let excess = state.tape.len().saturating_sub(MAX_TAPE_TAIL_BYTES);
    state.tape.drain(..excess);
    if let Some((tick, hash)) = checkpoint {
        state.checkpoint.clear();
        state.checkpoint.extend_from_slice(&tick.to_le_bytes());
        state.checkpoint.extend_from_slice(&hash.to_le_bytes());
        debug_assert_eq!(state.checkpoint.len(), CHECKPOINT_RECORD_BYTES);
    }
}

/// Name the GPU adapter behind the window (`name (driver)`) once the
/// presenter is up. Clipped to [`MAX_ADAPTER_BYTES`] at a char boundary.
pub fn set_adapter(description: &str) {
    let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    state.adapter = clip_prefix(description, MAX_ADAPTER_BYTES);
}

/// The settings state a crash report carries: the settings tape's digest and
/// the display-mode echo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsEcho {
    /// Digest of the settings tape in force (`engine_ui`'s
    /// `SettingsTape::digest`); `0` when nothing was recorded.
    pub digest: u64,
    /// Display-mode echo, bytes bounded by [`MAX_DISPLAY_BYTES`].
    pub display: String,
}

/// Record the live settings state the next crash report carries: the settings
/// tape's digest plus a display-mode echo, clipped to [`MAX_DISPLAY_BYTES`] at
/// a char boundary. A caller that changes a setting calls this once; a run that
/// never calls it reports digest `0` and an empty echo.
pub fn note_settings(digest: u64, display: &str) {
    let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    state.settings_digest = digest;
    state.display = clip_prefix(display, MAX_DISPLAY_BYTES);
}

/// What a report written now would carry as settings state. The report path
/// reads the same snapshot; this is the read side for tests and the door.
#[must_use]
pub fn settings_echo() -> SettingsEcho {
    let state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    SettingsEcho {
        digest: state.settings_digest,
        display: state.display.clone(),
    }
}

/// Append one status line to the run log. The arena binary calls this for
/// every line it prints, so the log holds what the console showed. The file
/// is unbuffered and each line is one write call, so a crash keeps every
/// line already written. No-op before install; lock poisoning is recovered.
pub fn mirror_line(line: &str) {
    let mut log = LOG.lock().unwrap_or_else(PoisonError::into_inner);
    write_log_line(log.as_mut(), line);
}

/// Panic-path flavour of [`mirror_line`]: never blocks. A lock held at the
/// moment of the panic skips the log line; the stderr copy still lands.
fn mirror_line_from_hook(line: &str) {
    if let Some(mut log) = try_lock_or_recover(&LOG) {
        write_log_line(log.as_mut(), line);
    }
}

fn write_log_line(log: Option<&mut File>, line: &str) {
    let Some(file) = log else {
        return;
    };
    if let Err(err) = file.write_all(format!("{line}\n").as_bytes()) {
        stderr_line(&format!("arena: run log write failed ({:?})", err.kind()));
    }
}

/// One line to stderr without `eprintln!`: a failed print there would panic
/// inside the panic hook, so the write result is ignored instead.
fn stderr_line(line: &str) {
    let _ = writeln!(std::io::stderr(), "{line}");
}

/// Lock without blocking; a poisoned lock still yields its data. `None` only
/// when another holder has the lock right now.
fn try_lock_or_recover<T>(lock: &Mutex<T>) -> Option<MutexGuard<'_, T>> {
    match lock.try_lock() {
        Ok(guard) => Some(guard),
        Err(TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
        Err(TryLockError::WouldBlock) => None,
    }
}

/// The panic hook. Std-only, never calls into the sim or the GPU. On the main
/// thread it writes the report and exits with the crash code. On any other
/// thread it only logs and returns, so the panic unwinds to its owner, which
/// may recover it (a joined worker, a reader's `catch_unwind`, an ecs job).
fn on_panic(info: &PanicHookInfo<'_>) {
    let message = panic_text(info.payload());
    let place = match info.location() {
        Some(at) => format!("{}:{}:{}", at.file(), at.line(), at.column()),
        None => "unknown location".to_string(),
    };
    let thread = std::thread::current();
    if thread.name() != Some(MAIN_THREAD) {
        let who = match thread.name() {
            Some(name) => name.to_string(),
            None => format!("{:?}", thread.id()),
        };
        let line = format!("arena worker panic on {who} at {place}: {message}");
        stderr_line(&line);
        mirror_line_from_hook(&line);
        return;
    }
    finish_fatal(&format!("arena panic at {place}: {message}"));
}

/// Fatal exit: log `line` to stderr and the run log, write the crash report,
/// then exit with `Panic`, or `ReportFailed` when the report cannot be written.
fn finish_fatal(line: &str) -> ! {
    stderr_line(line);
    mirror_line_from_hook(line);
    let code = match write_crash_report() {
        Ok(path) => {
            let note = format!("arena crash report: {}", path.display());
            stderr_line(&note);
            mirror_line_from_hook(&note);
            exit_code(CrashKind::Panic)
        }
        Err(err) => {
            let note = format!("arena: crash report not written ({err})");
            stderr_line(&note);
            mirror_line_from_hook(&note);
            exit_code(CrashKind::ReportFailed)
        }
    };
    std::process::exit(code)
}

/// Fatal exit for a panic that reached `main` without the hook running: a
/// worker panic its owner re-raised with `resume_unwind`. `main` calls this
/// from a `catch_unwind` around its body, so every fatal path writes a report.
pub fn fatal_unwind(payload: Box<dyn std::any::Any + Send>) -> ! {
    let line = format!("arena panic unwound to main: {}", panic_text(&*payload));
    finish_fatal(&line)
}

/// Build the report from the snapshot and write it to the run's path (or a
/// stamped file in the temp dir when no run was installed). Returns the path.
fn write_crash_report() -> Result<PathBuf, CrashError> {
    let path = match RUN.get() {
        Some(run) => run.report.clone(),
        None => std::env::temp_dir().join(format!("crash-{}.toml", utc_stamp(SystemTime::now()))),
    };
    let report = match try_lock_or_recover(&STATE) {
        Some(state) => {
            let adapter = if state.adapter.is_empty() {
                "no GPU adapter (window not presented)".to_string()
            } else {
                state.adapter.clone()
            };
            CrashReport::new(
                BUILD_VERSION.to_string(),
                adapter,
                state.mode,
                state.tick,
                state.tape.iter().copied().collect(),
                state.checkpoint.clone(),
            )?
            .with_settings(state.settings_digest, state.display.clone())?
        }
        None => CrashReport::new(
            BUILD_VERSION.to_string(),
            "unknown (snapshot busy at crash)".to_string(),
            CrashMode::Menu,
            0,
            Vec::new(),
            Vec::new(),
        )?,
    };
    write_report(&path, &report)?;
    write_minidump_stub(&path, &report).ok(); // best-effort stub; the report is the acceptance.
    Ok(path)
}

/// Text of a panic payload: `&str` or `String`, else a fixed note.
fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        return (*text).to_string();
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    "non-string panic payload".to_string()
}

/// UTC `YYYYMMDDTHHMMSSZ` for a wall time. Integer math (civil-from-days),
/// so no clock crate; times before 1970 read as the epoch.
fn utc_stamp(at: SystemTime) -> String {
    let secs = at
        .duration_since(UNIX_EPOCH)
        .map_or(0, |span| span.as_secs());
    let days = i64::try_from(secs / 86_400).unwrap_or(i64::MAX);
    let day_secs = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        day_secs / 3_600,
        day_secs % 3_600 / 60,
        day_secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: &str = "0.1.0+test";
    const ADAPTER: &str = "Test Adapter 1 (driver 0.1)";

    fn drill_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("e2040-crash-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir for crash drill");
        dir
    }

    fn sample(mode: CrashMode) -> CrashReport {
        drill_report(VERSION, ADAPTER, mode, 12_345).expect("drill bundle valid")
    }

    #[test]
    fn forced_crash_drill_leaves_readable_reports() {
        let dir = drill_dir("drill");
        for mode in [CrashMode::Menu, CrashMode::Fight, CrashMode::Lobby] {
            let report = sample(mode);
            let path = dir.join(format!("crash-{}.toml", mode.as_str()));
            write_report(&path, &report).expect("drill report writes");
            let text = std::fs::read_to_string(&path).expect("drill report readable");
            assert!(!text.is_empty(), "report must not be empty");
            assert!(text.contains("[crash]"), "missing table:\n{text}");
            assert!(text.contains(VERSION), "missing version:\n{text}");
            assert!(text.contains(ADAPTER), "missing adapter:\n{text}");
            assert!(text.contains(mode.as_str()), "missing mode token:\n{text}");
            assert!(text.contains("12345"), "missing tick:\n{text}");
            assert!(text.contains("checkpoint = \""), "missing row:\n{text}");
            assert!(text.contains("tape_tail = \""), "missing row:\n{text}");
            // Drill bytes are deterministic: same inputs, same file.
            let again = sample(mode);
            assert_eq!(format_report(&report), format_report(&again));
        }
        std::fs::remove_dir_all(&dir).expect("drill temp cleaned");
    }

    #[test]
    fn forced_dump_and_report_leave_one_each() {
        let dir = drill_dir("dump-pair");
        let report = sample(CrashMode::Fight);
        let toml = dir.join("crash-dump-pair.toml");
        write_report(&toml, &report).expect("report writes");
        let dump = write_minidump_stub(&toml, &report).expect("dump writes");
        assert!(toml.is_file() && dump.is_file(), "need 1 dump + 1 report");
        let bytes = std::fs::read(&dump).expect("dump readable");
        assert!(bytes.starts_with(b"MDMP"), "dump lacks magic");
        let bad_toml = dir.join("bad.toml");
        let bad = CrashReport {
            version: VERSION.to_string(),
            adapter: ADAPTER.to_string(),
            mode: CrashMode::Fight,
            tick: 1,
            settings_digest: 0,
            display: String::new(),
            tape_tail: Vec::new(),
            checkpoint: vec![0u8; MAX_CHECKPOINT_BYTES + 1],
        };
        assert!(write_minidump_stub(&bad_toml, &bad).is_err());
        assert!(!bad_toml.exists() && !minidump_path(&bad_toml).exists());
        std::fs::remove_dir_all(&dir).expect("drill temp cleaned");
    }

    #[test]
    fn exit_codes_are_loud_and_distinct() {
        let codes = [
            exit_code(CrashKind::ForcedDrill),
            exit_code(CrashKind::Panic),
            exit_code(CrashKind::StateCorrupt),
            exit_code(CrashKind::ReportFailed),
        ];
        for code in codes {
            assert_ne!(code, 0, "exit code must never be silent");
        }
        let mut sorted = codes.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len(), "kinds share a code");
        let errs = [
            CrashError::EmptyVersion,
            CrashError::VersionTooLong { len: 1, max: 0 },
            CrashError::AdapterTooLong { len: 1, max: 0 },
            CrashError::TapeTailTooLarge { len: 1, max: 0 },
            CrashError::CheckpointTooLarge { len: 1, max: 0 },
            CrashError::UnknownMode("x".to_string()),
            CrashError::Io(std::io::ErrorKind::NotFound),
        ];
        for err in &errs {
            assert_ne!(error_exit_code(err), 0, "error code silent: {err:?}");
            assert!(!err.to_string().is_empty(), "empty diagnostic");
        }
    }

    #[test]
    fn oversized_fields_fail_loudly_without_touching_disk() {
        let dir = drill_dir("bounds");
        let over_version = "v".repeat(MAX_VERSION_BYTES + 1);
        let err = CrashReport::new(
            over_version,
            ADAPTER.to_string(),
            CrashMode::Fight,
            0,
            Vec::new(),
            Vec::new(),
        )
        .expect_err("overlong version accepted");
        assert_eq!(
            err,
            CrashError::VersionTooLong {
                len: MAX_VERSION_BYTES + 1,
                max: MAX_VERSION_BYTES
            }
        );
        let err = CrashReport::new(
            VERSION.to_string(),
            "a".repeat(MAX_ADAPTER_BYTES + 1),
            CrashMode::Fight,
            0,
            Vec::new(),
            Vec::new(),
        )
        .expect_err("overlong adapter accepted");
        assert_eq!(
            err,
            CrashError::AdapterTooLong {
                len: MAX_ADAPTER_BYTES + 1,
                max: MAX_ADAPTER_BYTES
            }
        );
        let err = CrashReport::new(
            VERSION.to_string(),
            ADAPTER.to_string(),
            CrashMode::Fight,
            0,
            vec![0u8; MAX_TAPE_TAIL_BYTES + 1],
            Vec::new(),
        )
        .expect_err("oversized tape accepted");
        assert_eq!(
            err,
            CrashError::TapeTailTooLarge {
                len: MAX_TAPE_TAIL_BYTES + 1,
                max: MAX_TAPE_TAIL_BYTES
            }
        );
        let bad = CrashReport {
            version: VERSION.to_string(),
            adapter: ADAPTER.to_string(),
            mode: CrashMode::Lobby,
            tick: 7,
            settings_digest: 0,
            display: String::new(),
            tape_tail: Vec::new(),
            checkpoint: vec![0u8; MAX_CHECKPOINT_BYTES + 1],
        };
        let path = dir.join("must-not-exist.toml");
        assert!(write_report(&path, &bad).is_err());
        assert!(!path.exists(), "bad bundle touched the disk");
        assert!(format_report(&bad).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_version_refused() {
        let err = CrashReport::new(
            String::new(),
            ADAPTER.to_string(),
            CrashMode::Menu,
            0,
            Vec::new(),
            Vec::new(),
        )
        .expect_err("empty version accepted");
        assert_eq!(err, CrashError::EmptyVersion);
    }

    #[test]
    fn toml_specials_are_escaped_on_one_line_per_row() {
        let report = CrashReport::new(
            "v\"1\\\n2".to_string(),
            "a\tb".to_string(),
            CrashMode::Fight,
            1,
            vec![0xABu8, 0x00, 0xFF],
            vec![0x01u8],
        )
        .expect("specials fit the bounds");
        let text = format_report(&report).expect("specials format");
        assert!(text.contains("v\\\"1\\\\\\n2"), "not escaped:\n{text}");
        assert!(text.contains("a\\tb"), "tab not escaped:\n{text}");
        assert!(text.contains("ab00ff"), "tape hex wrong:\n{text}");
        // Eight rows plus the table header: an unescaped newline would add one.
        assert_eq!(text.lines().count(), 9, "row leaked a newline:\n{text}");
    }

    #[test]
    fn mode_parse_roundtrip_and_loud_refusal() {
        for mode in [CrashMode::Menu, CrashMode::Fight, CrashMode::Lobby] {
            assert_eq!(CrashMode::parse(mode.as_str()), Ok(mode));
        }
        for bad in ["MENU", "fight ", "", "arena"] {
            assert!(CrashMode::parse(bad).is_err(), "mode accepted: {bad:?}");
        }
        // Long hostile tokens echo bounded and on a char boundary.
        let long = "x".repeat(1024);
        let Err(CrashError::UnknownMode(echo)) = CrashMode::parse(&long) else {
            panic!("long mode accepted");
        };
        assert!(echo.len() <= MAX_MODE_ECHO_BYTES);
        let emoji = "🪂".repeat(32);
        let Err(CrashError::UnknownMode(echo)) = CrashMode::parse(&emoji) else {
            panic!("emoji mode accepted");
        };
        assert!(echo.len() <= MAX_MODE_ECHO_BYTES);
    }

    #[test]
    fn utc_stamp_is_calendar_utc() {
        // 1_700_000_000 s after the epoch is 2023-11-14 22:13:20 UTC.
        let at = UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        assert_eq!(utc_stamp(at), "20231114T221320Z");
        assert_eq!(utc_stamp(UNIX_EPOCH), "19700101T000000Z");
        // 2024-02-29 is a leap day: the day after is March 1.
        let leap = UNIX_EPOCH + std::time::Duration::from_secs(1_709_164_800);
        assert_eq!(utc_stamp(leap), "20240229T000000Z");
    }

    #[test]
    fn tail_window_keeps_the_suffix() {
        let bytes: Vec<u8> = (0..10).collect();
        assert_eq!(tail_window(&bytes, 4), &[6, 7, 8, 9]);
        assert_eq!(tail_window(&bytes, 0), &[] as &[u8]);
        assert_eq!(tail_window(&bytes, 100), bytes.as_slice());
        assert_eq!(tail_window(&[], 8), &[] as &[u8]);
    }

    #[test]
    fn missing_directory_fails_as_io_not_panic() {
        let dir = drill_dir("missing");
        let path = dir.join("no-such-dir").join("crash.toml");
        let report = sample(CrashMode::Fight);
        let err = write_report(&path, &report).expect_err("write into void accepted");
        assert!(matches!(err, CrashError::Io(_)), "wrong error: {err:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn crash_drill_reports_roundtrip_through_schema() {
        let dir = drill_dir("schema-roundtrip");
        for mode in [CrashMode::Menu, CrashMode::Fight, CrashMode::Lobby] {
            let report = sample(mode);
            let path = dir.join(format!("crash-{}.toml", mode.as_str()));
            write_report(&path, &report).expect("drill report writes");
            let text = std::fs::read_to_string(&path).expect("drill report readable");
            let parsed = parse_report(&text).expect("drill report parses");
            assert_eq!(parsed, report, "roundtrip diverged for {}", mode.as_str());
            assert_eq!(
                format_report(&parsed).expect("reformat works"),
                text,
                "bytes diverged for {}",
                mode.as_str()
            );
        }
        std::fs::remove_dir_all(&dir).expect("drill temp cleaned");
    }

    #[test]
    fn crash_schema_refuses_unknown_fields() {
        let report = sample(CrashMode::Fight);
        let text = format_report(&report).expect("valid report formats");
        // Trailing unknown row is refused with its clipped name.
        let bad = format!("{text}extra = \"evil\"\n");
        let Err(CrashError::UnknownField(field)) = parse_report(&bad) else {
            panic!("unknown trailing field accepted:\n{bad}");
        };
        assert_eq!(field, "extra");
        // Unknown row among known rows is refused too.
        let bad = text.replacen("mode =", "sponsor = \"evil\"\nmode =", 1);
        assert!(
            matches!(parse_report(&bad), Err(CrashError::UnknownField(_))),
            "inline unknown field accepted:\n{bad}"
        );
        // Long hostile field names echo bounded on a char boundary.
        let long = "y".repeat(1024);
        let bad = format!("{text}{long} = \"evil\"\n");
        let Err(CrashError::UnknownField(echo)) = parse_report(&bad) else {
            panic!("long field accepted");
        };
        assert!(echo.len() <= MAX_FIELD_ECHO_BYTES);
        // Missing, duplicate and malformed rows fail loudly.
        let missing = text
            .lines()
            .filter(|line| !line.starts_with("tick ="))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        assert_eq!(
            parse_report(&missing),
            Err(CrashError::MissingField("tick"))
        );
        let dup = format!("{text}tick = 1\n");
        assert_eq!(parse_report(&dup), Err(CrashError::BadValue("tick")));
        let bad_tick = text.replacen("tick = 12345", "tick = \"12345\"", 1);
        assert_eq!(parse_report(&bad_tick), Err(CrashError::BadValue("tick")));
        let bad_hex = text.replacen("checkpoint = \"", "checkpoint = \"zz", 1);
        assert_eq!(
            parse_report(&bad_hex),
            Err(CrashError::BadValue("checkpoint"))
        );
        let no_header = text.replacen("[crash]\n", "", 1);
        assert_eq!(
            parse_report(&no_header),
            Err(CrashError::BadValue("header"))
        );
        // Bad mode keeps the UnknownMode refusal, not a generic row error.
        let bad_mode = text.replacen("mode = \"fight\"", "mode = \"arena\"", 1);
        assert!(
            matches!(parse_report(&bad_mode), Err(CrashError::UnknownMode(_))),
            "bad mode not refused as UnknownMode:\n{bad_mode}"
        );
        // New schema errors are loud and distinct.
        let codes = [
            error_exit_code(&CrashError::UnknownField("x".to_string())),
            error_exit_code(&CrashError::MissingField("tick")),
            error_exit_code(&CrashError::BadValue("tick")),
        ];
        for code in codes {
            assert_ne!(code, 0, "schema error code silent");
            assert_ne!(
                code,
                error_exit_code(&CrashError::Io(std::io::ErrorKind::NotFound))
            );
        }
        assert!(!CrashError::UnknownField("x".to_string())
            .to_string()
            .is_empty());
        assert!(!CrashError::MissingField("tick").to_string().is_empty());
        assert!(!CrashError::BadValue("tick").to_string().is_empty());
    }

    #[test]
    fn settings_digest_and_display_roundtrip_through_schema() {
        let report = sample(CrashMode::Fight)
            .with_settings(0x0bad_c0de_1234_5678, "borderless".to_string())
            .expect("settings fit the bounds");
        let text = format_report(&report).expect("formats");
        assert!(
            text.contains("settings_digest = \"0badc0de12345678\""),
            "{text}"
        );
        assert!(text.contains("display = \"borderless\""), "{text}");
        let parsed = parse_report(&text).expect("parses");
        assert_eq!(parsed, report, "roundtrip lost the settings state");
        assert_eq!(format_report(&parsed).expect("reformats"), text);
        // The two rows are part of the strict schema: dropping one is loud.
        let missing = text
            .lines()
            .filter(|line| !line.starts_with("settings_digest ="))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        assert_eq!(
            parse_report(&missing),
            Err(CrashError::MissingField("settings_digest"))
        );
        let missing = text
            .lines()
            .filter(|line| !line.starts_with("display ="))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        assert_eq!(
            parse_report(&missing),
            Err(CrashError::MissingField("display"))
        );
        // A bad digest string is a bad value, never a silent zero.
        let bad = text.replacen("settings_digest = \"", "settings_digest = \"zz", 1);
        assert_eq!(
            parse_report(&bad),
            Err(CrashError::BadValue("settings_digest"))
        );
        // A bundle with no settings writes the zero digest and an empty echo.
        let plain = format_report(&sample(CrashMode::Menu)).expect("formats");
        assert!(
            plain.contains("settings_digest = \"0000000000000000\""),
            "{plain}"
        );
        assert!(plain.contains("display = \"\""), "{plain}");
    }

    #[test]
    fn custom_settings_regenerate_the_pre_crash_tick() {
        // A real checkpoint record: tick (u32, little-endian) then hash (u64).
        let tick = 12_345u32;
        let mut checkpoint = tick.to_le_bytes().to_vec();
        checkpoint.extend_from_slice(&0x5eed_5eed_5eed_5eedu64.to_le_bytes());
        let digest = 0x00d1_6e57_1234_0001u64;
        let report = CrashReport::new(
            VERSION.to_string(),
            ADAPTER.to_string(),
            CrashMode::Fight,
            tick,
            Vec::new(),
            checkpoint,
        )
        .expect("bundle valid")
        .with_settings(digest, "fullscreen".to_string())
        .expect("settings fit the bounds");
        assert_eq!(
            report.regenerate_tick(digest),
            Ok(tick),
            "report plus settings regenerate the pre-crash tick"
        );
        // The same inputs twice give the same bytes, and the tick still reads
        // back after a schema roundtrip.
        let text = format_report(&report).expect("formats");
        let parsed = parse_report(&text).expect("parses");
        assert_eq!(parsed.regenerate_tick(digest), Ok(tick));
        assert_eq!(format_report(&parsed).expect("reformats"), text);
    }

    #[test]
    fn settings_digest_mismatch_refuses_loudly() {
        let tick = 400u32;
        let mut checkpoint = tick.to_le_bytes().to_vec();
        checkpoint.extend_from_slice(&7u64.to_le_bytes());
        let report = CrashReport::new(
            VERSION.to_string(),
            ADAPTER.to_string(),
            CrashMode::Fight,
            tick,
            Vec::new(),
            checkpoint.clone(),
        )
        .expect("bundle valid")
        .with_settings(0xabc, "windowed".to_string())
        .expect("settings fit");
        let err = report
            .regenerate_tick(0xdef)
            .expect_err("settings from another run are refused");
        assert_eq!(
            err,
            CrashError::SettingsDigestMismatch {
                want: 0xabc,
                got: 0xdef
            }
        );
        assert!(
            err.to_string().contains("0abc"),
            "the refusal names both: {err}"
        );
        assert_ne!(error_exit_code(&err), 0);
        assert_ne!(
            error_exit_code(&err),
            error_exit_code(&CrashError::BadValue("checkpoint"))
        );
        // A matching digest with a checkpoint that cannot regenerate the tick
        // is a bad value, never a silent tick: the drill payload is synthetic.
        let drill = sample(CrashMode::Fight);
        assert!(matches!(
            drill.regenerate_tick(0),
            Err(CrashError::BadValue("checkpoint"))
        ));
        // A short record and a record whose tick disagrees both fail too.
        let short = CrashReport::new(
            VERSION.to_string(),
            ADAPTER.to_string(),
            CrashMode::Fight,
            tick,
            Vec::new(),
            tick.to_le_bytes().to_vec(),
        )
        .expect("bundle valid")
        .with_settings(0, String::new())
        .expect("no settings fit");
        assert!(matches!(
            short.regenerate_tick(0),
            Err(CrashError::BadValue("checkpoint"))
        ));
        let disagreeing = CrashReport::new(
            VERSION.to_string(),
            ADAPTER.to_string(),
            CrashMode::Fight,
            tick + 1,
            Vec::new(),
            checkpoint,
        )
        .expect("bundle valid")
        .with_settings(0, String::new())
        .expect("no settings fit");
        assert!(matches!(
            disagreeing.regenerate_tick(0),
            Err(CrashError::BadValue("checkpoint"))
        ));
    }

    #[test]
    fn oversized_display_fails_loudly_with_a_distinct_code() {
        let err = sample(CrashMode::Fight)
            .with_settings(1, "d".repeat(MAX_DISPLAY_BYTES + 1))
            .expect_err("overlong display echo accepted");
        assert_eq!(
            err,
            CrashError::DisplayTooLong {
                len: MAX_DISPLAY_BYTES + 1,
                max: MAX_DISPLAY_BYTES,
            }
        );
        assert_ne!(error_exit_code(&err), 0);
        assert_ne!(
            error_exit_code(&err),
            error_exit_code(&CrashError::UnknownMode("x".to_string()))
        );
        assert!(!err.to_string().is_empty());
        // The bound itself is legal and writes.
        let dir = drill_dir("display-bound");
        let report = sample(CrashMode::Fight)
            .with_settings(2, "d".repeat(MAX_DISPLAY_BYTES))
            .expect("the bound is legal");
        let path = dir.join("crash-display-bound.toml");
        write_report(&path, &report).expect("bound-sized echo writes");
        let text = std::fs::read_to_string(&path).expect("report readable");
        assert_eq!(parse_report(&text).expect("parses"), report);
        std::fs::remove_dir_all(&dir).expect("drill temp cleaned");
    }

    #[test]
    fn note_settings_records_the_live_settings_echo() {
        // A run that never records settings reports the defaults.
        note_settings(0, "");
        assert_eq!(
            settings_echo(),
            SettingsEcho {
                digest: 0,
                display: String::new()
            }
        );
        note_settings(0xc0ff_ee00_1234_5678, "borderless");
        assert_eq!(
            settings_echo(),
            SettingsEcho {
                digest: 0xc0ff_ee00_1234_5678,
                display: "borderless".to_string()
            }
        );
        // A hostile echo is clipped at a char boundary, never stored raw.
        note_settings(9, &"🪂".repeat(32));
        let echo = settings_echo();
        assert_eq!(echo.digest, 9);
        assert!(
            echo.display.len() <= MAX_DISPLAY_BYTES,
            "{:?}",
            echo.display
        );
        assert!(echo.display.is_char_boundary(echo.display.len()));
        // Leave the shared snapshot as the other tests expect to find it.
        note_settings(0, "");
    }

    #[test]
    fn crash_drill_preserves_previous_save_generation() {
        use engine_save::fallback::prev_name;
        for mode in [CrashMode::Menu, CrashMode::Fight, CrashMode::Lobby] {
            let dir = drill_dir(&format!("sv1-{}", mode.as_str()));
            let slot: u8 = 3;
            engine_save::save_generation(&dir, slot, 7, 70, b"gen-one")
                .expect("first generation saves");
            engine_save::save_generation(&dir, slot, 7, 71, b"gen-two")
                .expect("second generation saves");
            // Forced crash leaves its own schema-valid report beside the saves.
            let report = sample(mode);
            let path = dir.join(format!("crash-{}.toml", mode.as_str()));
            write_report(&path, &report).expect("drill report writes");
            let text = std::fs::read_to_string(&path).expect("report readable");
            assert_eq!(parse_report(&text).expect("report parses"), report);
            // Both generations stay loadable after the drill.
            let current = std::fs::read(dir.join(engine_save::slot_name(slot).expect("slot")))
                .expect("current save readable");
            assert_eq!(
                engine_save::open(&current).expect("current opens").payload,
                b"gen-two"
            );
            let prev = std::fs::read(dir.join(prev_name(slot).expect("prev name")))
                .expect("previous save readable");
            assert_eq!(
                engine_save::open(&prev).expect("previous opens").payload,
                b"gen-one"
            );
            // A crash that corrupts the current file still leaves the previous.
            let mut broken = current;
            broken[0] = b'Z';
            std::fs::write(
                dir.join(engine_save::slot_name(slot).expect("slot")),
                &broken,
            )
            .expect("corrupt current");
            let reread = std::fs::read(dir.join(engine_save::slot_name(slot).expect("slot")))
                .expect("broken current readable");
            assert!(
                engine_save::open(&reread).is_err(),
                "corrupt current opened"
            );
            let prev = std::fs::read(dir.join(prev_name(slot).expect("prev name")))
                .expect("previous survives");
            let env = engine_save::open(&prev).expect("previous still opens");
            assert_eq!(env.payload, b"gen-one");
            assert_eq!(env.tick, 70);
            std::fs::remove_dir_all(&dir).expect("drill temp cleaned");
        }
    }

    #[test]
    fn warning_bundle_writes_report_tape_and_readme() {
        let root = drill_dir("repro-bundle");
        set_adapter(ADAPTER);
        record_fight_tick(7, &[0xAA, 0xBB, 0xCC], Some((6, 0x5eed_5eed_5eed_5eedu64)));
        let warnings = vec![
            "arena WARN: view_sunk_cm_max value=12.0 floor=3.0".to_string(),
            "arena WARN: view_nonfinite value=1 floor=0".to_string(),
        ];
        let view = "arena view: frames=7 sim_ticks=0.0..6.0 bodies=14 sunk_cm_max=12.0";
        let bundle =
            write_warning_bundle(&root, "view_sunk_cm_max", 2040, 600, 7, 2, view, &warnings)
                .expect("bundle writes");
        assert!(bundle.is_dir(), "bundle dir: {}", bundle.display());
        let name = bundle
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        assert!(
            name.starts_with("view_sunk_cm_max-") && name.ends_with('Z'),
            "metric-stamped dir: {name}"
        );
        let report = std::fs::read_to_string(bundle.join("report.toml")).expect("report readable");
        for needle in [
            "[repro]",
            "metric = \"view_sunk_cm_max\"",
            "seed = 2040",
            "ticks = 600",
            "ticks_done = 7",
            "fighters = 2",
            view,
            warnings[0].as_str(),
            warnings[1].as_str(),
            ADAPTER,
            "checkpoint = \"6:5eed5eed5eed5eed\"",
            "tape = \"tape.bin\"",
            "tape_bytes = 3",
        ] {
            assert!(
                report.contains(needle),
                "report misses {needle:?}:\n{report}"
            );
        }
        assert_eq!(
            std::fs::read(bundle.join("tape.bin")).expect("tape readable"),
            vec![0xAA, 0xBB, 0xCC],
            "tape.bin is the crash tape tail"
        );
        let readme = std::fs::read_to_string(bundle.join("README.txt")).expect("readme readable");
        assert!(readme.contains("view_sunk_cm_max"), "{readme}");
        assert!(
            readme.contains("arena.exe --seed 2040 --ticks 600 --fighters 2 --frames 600 --exit"),
            "rerun line:\n{readme}"
        );
        assert!(
            readme.contains("checkpoint: tick 6, hash 5eed5eed5eed5eed"),
            "{readme}"
        );
        // A hostile metric never escapes the bundle root.
        let bad = write_warning_bundle(&root, "..\\escape", 2040, 1, 1, 2, view, &warnings);
        assert_eq!(bad, Err(CrashError::BadValue("metric")));
        assert!(
            !root.join("escape").exists(),
            "hostile metric wrote outside the root"
        );
        std::fs::remove_dir_all(&root).expect("repro temp cleaned");
    }

    #[test]
    #[ignore = "timing: crash report write budget 1 s, run under --perf in release"]
    fn crash_drill_write_within_one_second() {
        let dir = drill_dir("budget");
        for mode in [CrashMode::Menu, CrashMode::Fight, CrashMode::Lobby] {
            let report = sample(mode);
            let path = dir.join(format!("crash-{}.toml", mode.as_str()));
            let start = std::time::Instant::now();
            write_report(&path, &report).expect("drill report writes");
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_secs() < 1,
                "drill write too slow for {}: {elapsed:?}",
                mode.as_str()
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
