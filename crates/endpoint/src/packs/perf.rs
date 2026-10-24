//! `perf` pack: quiet perf runs as typed rows, plus the `doctor` preflight.
//!
//! The first read-only verb, `perf.table`, returns the rows of the newest
//! quiet perf csv as typed integer rows, so an agent reads p50 and p99 per
//! scenario through `api.call` instead of opening a file.
//!
//! The second read-only verb, `doctor`, answers what is broken before a model
//! starts: one line per check (GPU adapter, content root, bundle frames,
//! installed bridge age, last proof per crate), each with a status, what was
//! found and the fix. It never panics on a missing thing: a missing folder,
//! report or proof log is a `broken` or `skip` line with its recovery. Probes
//! stay cheap: no GPU device is opened, a bundle header and its frame are
//! walked without hashing, and one small receipt file names the adapter.
//!
//! Units: integer microseconds for `p50_us` and `p99_us`, chars for paths,
//! hours or minutes for ages. Determinism: the csv is read as bytes; the same
//! file always parses to the same rows in file order, and the newest file is
//! the lexicographically greatest `perf-*.csv` name. The doctor reads the tree
//! only; the same tree and clock give the same receipt. Errors: [`McpToolError`]
//! with `bad_args` for a bad `csv` path, `bad_csv` for a missing or malformed
//! csv, `no_perf_csv` when no csv exists; `doctor` answers `broken` lines
//! instead of errors.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::bad_args;
use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;

/// Pack name.
pub const PACK: &str = "perf";
/// Verb: quiet perf rows as typed integers.
pub const TOOL_PERF_TABLE: &str = "perf.table";
/// Verb: the preflight receipt, one line per check with its fix.
pub const TOOL_DOCTOR: &str = "perf.doctor";
/// Verb: one capture written as a Chrome-trace JSON document.
pub const TOOL_PERF_TRACE: &str = "perf.trace";

/// Longest csv argument in chars.
const MAX_CSV_CHARS: usize = 512;
/// Largest csv file in bytes.
const MAX_CSV_BYTES: usize = 64 * 1024;
/// Largest csv file in bytes as `u64` for metadata checks.
const MAX_CSV_BYTES_U64: u64 = 64 * 1024;

/// The perf pack: quiet runs as typed rows, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(PACK, "quiet perf runs as typed rows", false);
    pack.add(VerbSpec::of::<PerfTableTool>(
        TOOL_PERF_TABLE,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<DoctorTool>(
        TOOL_DOCTOR,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<PerfTraceTool>(
        TOOL_PERF_TRACE,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

// ---------------------------------------------------------------------------
// Transport DTOs. Field docs are omitted on purpose: every doc line inflates
// the advertised schemas, and the discovery budget is a gate.
// ---------------------------------------------------------------------------

/// Perf table request: an optional csv path with the newest quiet csv as default.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PerfTableArgs {
    #[serde(default)]
    pub csv: Option<String>,
}

/// One perf row: scenario with integer microsecond p50, p99 and the final hash.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PerfRow {
    pub scenario: String,
    pub p50_us: u64,
    pub p99_us: u64,
    pub final_hash: String,
}

/// Perf table answer: source file plus typed rows in file order.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PerfTableOutput {
    pub source: String,
    pub rows: Vec<PerfRow>,
}

fn bad_csv(message: String) -> McpToolError {
    McpToolError::new("bad_csv", message)
}

fn no_csv() -> McpToolError {
    McpToolError::new(
        "no_perf_csv",
        "no perf csv found under target/reports (perf-*.csv); \
         re-run cargo run --release -p engine-perf --bin perf to record one",
    )
}

/// A csv argument must be a non-empty relative path of plain names ending in `.csv`.
fn check_csv_path(field: &str, raw: &str) -> Result<(), McpToolError> {
    let plain = !raw.is_empty()
        && raw.chars().count() <= MAX_CSV_CHARS
        && !raw.contains(['\0', '\\', ':'])
        && raw.ends_with(".csv")
        && Path::new(raw)
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir));
    if plain {
        return Ok(());
    }
    Err(bad_args(format!(
        "{field} `{}` is not a relative csv path of plain names ending in `.csv` \
         (1..={MAX_CSV_CHARS} chars, `/` separators, no `..`, no drive, colon, backslash \
         or leading slash); re-run with a path such as target/reports/perf-2026-10-10T1826Z-1.csv",
        raw.chars().take(80).collect::<String>()
    )))
}

/// Parse `ms` text to integer microseconds without floats.
fn ms_to_us(cell: &str, line_no: usize) -> Result<u64, McpToolError> {
    let bad = |why: &str| {
        bad_csv(format!(
            "line {line_no}: `{cell}` is not ms ({why}); re-run after fixing the csv"
        ))
    };
    if cell.is_empty()
        || cell.bytes().any(|b| !(b.is_ascii_digit() || b == b'.'))
        || cell.bytes().filter(|b| *b == b'.').count() > 1
    {
        return Err(bad("digits with at most one dot"));
    }
    let (int_part, frac_part) = match cell.split_once('.') {
        Some((head, tail)) => (head, tail),
        None => (cell, ""),
    };
    if cell.contains('.') && (int_part.is_empty() || frac_part.is_empty()) {
        return Err(bad("digits with at most one dot"));
    }
    if !int_part.bytes().all(|b| b.is_ascii_digit())
        || !frac_part.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(bad("digits with at most one dot"));
    }
    let int_val: u64 = int_part
        .parse()
        .map_err(|_| bad("digits with at most one dot"))?;
    let int_us = int_val
        .checked_mul(1000)
        .ok_or_else(|| bad("value is too large"))?;
    let mut frac3 = String::with_capacity(3);
    for ch in frac_part.chars().take(3) {
        frac3.push(ch);
    }
    while frac3.len() < 3 {
        frac3.push('0');
    }
    let frac_us: u64 = if frac3.is_empty() {
        0
    } else {
        frac3
            .parse()
            .map_err(|_| bad("digits with at most one dot"))?
    };
    int_us
        .checked_add(frac_us)
        .ok_or_else(|| bad("value is too large"))
}

/// Parse a quiet perf csv into typed rows. Header required, columns by name.
pub(crate) fn parse_perf_csv(text: &str) -> Result<Vec<PerfRow>, McpToolError> {
    let mut scenario_idx: Option<usize> = None;
    let mut p50_idx: Option<usize> = None;
    let mut p99_idx: Option<usize> = None;
    let mut hash_idx: Option<usize> = None;
    let mut width: usize = 0;
    let mut header_done = false;
    let mut header_line: usize = 1;
    let mut out: Vec<PerfRow> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line_no = index + 1;
        if raw.trim().is_empty() {
            continue;
        }
        let cells: Vec<&str> = raw.split(',').map(|cell| cell.trim()).collect();
        if !header_done {
            width = cells.len();
            header_line = line_no;
            for (pos, name) in cells.iter().enumerate() {
                match *name {
                    "scenario" => scenario_idx = Some(pos),
                    "p50_ms" => p50_idx = Some(pos),
                    "p99_ms" => p99_idx = Some(pos),
                    "final_hash" => hash_idx = Some(pos),
                    _ => {}
                }
            }
            if scenario_idx.is_none()
                || p50_idx.is_none()
                || p99_idx.is_none()
                || hash_idx.is_none()
            {
                return Err(bad_csv(format!(
                    "line {line_no}: header is missing scenario, p50_ms, p99_ms or final_hash; \
                     re-run after recording the quiet csv"
                )));
            }
            header_done = true;
            continue;
        }
        if cells.len() != width {
            return Err(bad_csv(format!(
                "line {line_no}: row has {} cells, header has {width} (line {header_line}); \
                 re-run after fixing the csv",
                cells.len()
            )));
        }
        let (s_idx, p50_i, p99_i, h_idx) = match (scenario_idx, p50_idx, p99_idx, hash_idx) {
            (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
            _ => {
                return Err(bad_csv(format!(
                    "line {line_no}: header is missing scenario, p50_ms, p99_ms or final_hash; \
                     re-run after recording the quiet csv"
                )));
            }
        };
        let scenario = cells[s_idx].to_owned();
        let p50_us = ms_to_us(cells[p50_i], line_no)?;
        let p99_us = ms_to_us(cells[p99_i], line_no)?;
        let final_hash = cells[h_idx].to_owned();
        out.push(PerfRow {
            scenario,
            p50_us,
            p99_us,
            final_hash,
        });
    }
    if !header_done {
        return Err(bad_csv(
            "line 1: header is missing scenario, p50_ms, p99_ms or final_hash; \
             re-run after recording the quiet csv"
                .to_owned(),
        ));
    }
    Ok(out)
}

/// Pick the newest quiet csv under `root/target/reports`.
fn newest_csv(root: &Path) -> Result<String, McpToolError> {
    let dir = root.join("target/reports");
    let entries = std::fs::read_dir(&dir).map_err(|_| no_csv())?;
    let mut best: Option<String> = None;
    for entry in entries {
        let entry = entry.map_err(|error| {
            bad_csv(format!(
                "target/reports is unreadable ({error}); re-run after fixing it"
            ))
        })?;
        let is_file = entry
            .file_type()
            .map_err(|error| {
                bad_csv(format!(
                    "target/reports is unreadable ({error}); re-run after fixing it"
                ))
            })?
            .is_file();
        if !is_file {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("perf-")
            && name.ends_with(".csv")
            && best.as_ref().is_none_or(|seen| name > *seen)
        {
            best = Some(name);
        }
    }
    best.map(|name| format!("target/reports/{name}"))
        .ok_or_else(no_csv)
}

/// Run one request: validate args, resolve the file under `root`, read and parse.
fn run_perf_table(args: &PerfTableArgs, root: &Path) -> Result<PerfTableOutput, McpToolError> {
    let rel = match &args.csv {
        Some(csv) => {
            check_csv_path("csv", csv)?;
            csv.clone()
        }
        None => newest_csv(root)?,
    };
    let path = root.join(&rel);
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > MAX_CSV_BYTES_U64 {
            return Err(bad_csv(format!(
                "`{rel}` is over 64 KiB; re-run with the quiet csv"
            )));
        }
    }
    let text = std::fs::read_to_string(&path).map_err(|error| {
        bad_csv(format!(
            "`{rel}` is unreadable ({error}); re-run after recording the quiet csv"
        ))
    })?;
    if text.len() > MAX_CSV_BYTES {
        return Err(bad_csv(format!(
            "`{rel}` is over 64 KiB; re-run with the quiet csv"
        )));
    }
    let rows = parse_perf_csv(&text)?;
    Ok(PerfTableOutput { source: rel, rows })
}

/// `perf.table` tool: quiet perf runs as typed rows, read-only.
pub struct PerfTableTool;

impl ToolBase for PerfTableTool {
    type Parameter = PerfTableArgs;
    type Output = PerfTableOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_PERF_TABLE.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Quiet perf runs as typed rows: p50 and p99 per scenario in integer microseconds \
             from the newest perf csv. Read-only."
                .into(),
        )
    }

    fn annotations() -> Option<rmcp::model::ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for PerfTableTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_PERF_TABLE, |_state| {
            run_perf_table(&param, Path::new("."))
        })
    }
}

// ---------------------------------------------------------------------------
// doctor: what is broken before a model starts.
// ---------------------------------------------------------------------------

/// Installed door binary the OpenCode session spawns (DOOR5).
const BRIDGE_INSTALLED: &str = ".opencode/bin/e2040-mcp.exe";
/// Release build the installed copy is refreshed from (DOOR5).
const BRIDGE_RELEASE: &str = "target/release/e2040-mcp.exe";
/// Per-crate proof records `tools/gh-check.mjs` keeps.
const PROOF_DIR: &str = "target/gh-check/results";
/// Longest proof record read, in bytes.
const MAX_PROOF_BYTES: u64 = 1024 * 1024;
/// A proof older than this many hours is stale, in milliseconds.
const PROOF_MAX_AGE_MS: u128 = 72 * 3_600_000;
/// Folder the arena window writes its reports into.
const REPORT_DIR: &str = "target/reports";
/// Newest arena reports read for the adapter name.
const MAX_ADAPTER_REPORTS: usize = 8;
/// The render crate's adapter override (RL11): a name with no device opened.
const GPU_ADAPTER_ENV: &str = "WGPU_ADAPTER_NAME";
/// Folder roots a cooked bundle sits under, relative to the project root.
const BUNDLE_ROOTS: [&str; 2] = ["content", "dist/web/assets"];
/// Bundle file suffix.
const BUNDLE_SUFFIX: &str = "e2bundle";
/// Bundle magic, owned by `engine-assets` (`crates/assets/src/bundle.rs`).
const BUNDLE_MAGIC: [u8; 8] = *b"E204BNDL";
/// Bundle frame version the runtime reads, owned by `engine-assets`.
const BUNDLE_VERSION: u32 = 1;
/// Most entries a bundle may declare, owned by `engine-assets`.
const MAX_BUNDLE_ENTRIES: u32 = 4096;
/// Largest bundle the loader accepts, owned by `engine-assets`.
const MAX_BUNDLE_BYTES: u64 = 512 * 1024 * 1024;
/// Highest entry kind byte the loader knows (clip), owned by `engine-assets`.
const MAX_BUNDLE_KIND: u8 = 5;
/// Longest crate name kept from a proof command line.
const MAX_CRATE_CHARS: usize = 64;
/// Longest detail fragment echoed in a receipt line.
const MAX_DETAIL_CHARS: usize = 160;

/// One check's verdict in the preflight receipt.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde", rename_all = "snake_case")]
#[schemars(crate = "rmcp::schemars")]
pub enum DoctorStatus {
    /// The check passed.
    Ok,
    /// The check failed; the line carries the fix.
    Broken,
    /// Nothing to check in this tree; the line says what would create it.
    Skip,
}

impl DoctorStatus {
    /// Lowercase wire name, the tag a receipt line prints.
    fn name(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Broken => "broken",
            Self::Skip => "skip",
        }
    }
}

/// One check of the preflight: status, what was found and the fix.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct DoctorLine {
    pub check: String,
    pub status: DoctorStatus,
    pub detail: String,
    pub fix: String,
}

impl DoctorLine {
    /// One receipt line: `[status] check: detail | fix: ...`.
    pub fn line(&self) -> String {
        if self.fix.is_empty() {
            format!("[{}] {}: {}", self.status.name(), self.check, self.detail)
        } else {
            format!(
                "[{}] {}: {} | fix: {}",
                self.status.name(),
                self.check,
                self.detail,
                self.fix
            )
        }
    }

    fn ok(check: &'static str, detail: String) -> Self {
        Self {
            check: check.to_owned(),
            status: DoctorStatus::Ok,
            detail,
            fix: String::new(),
        }
    }

    fn broken(check: &'static str, detail: String, fix: String) -> Self {
        Self {
            check: check.to_owned(),
            status: DoctorStatus::Broken,
            detail,
            fix,
        }
    }

    fn skip(check: &'static str, detail: String, fix: String) -> Self {
        Self {
            check: check.to_owned(),
            status: DoctorStatus::Skip,
            detail,
            fix,
        }
    }
}

/// Answer to `doctor`: the root checked, the receipt text and its typed lines.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct DoctorOutput {
    pub root: String,
    pub ok: bool,
    pub broken: usize,
    pub skipped: usize,
    pub receipt: String,
    pub lines: Vec<DoctorLine>,
}

/// Doctor request: the project root to check.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct DoctorArgs {
    #[serde(default)]
    pub root: Option<String>,
}

/// Milliseconds since the Unix epoch; zero when the clock is before it.
fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0)
}

/// Replace control chars with spaces and cap the echo, so one check stays one line.
fn one_line(text: &str, max: usize) -> String {
    let clean: String = text
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    let trimmed = clean.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_owned();
    }
    let mut out: String = trimmed.chars().take(max.saturating_sub(3)).collect();
    out.push_str("...");
    out
}

/// A span as whole minutes under an hour and whole hours above it.
fn span_ms(ms: u128) -> String {
    let minutes = ms / 60_000;
    if minutes < 60 {
        format!("{minutes} min")
    } else {
        format!("{} h", minutes / 60)
    }
}

/// Run every check, cheapest first, and fold the lines into one receipt.
fn run_doctor(root: &Path, now_ms: u128, adapter: Option<&str>) -> DoctorOutput {
    let lines = vec![
        gpu_line(root, adapter),
        content_line(root),
        bundles_line(root),
        bridge_line(root),
        proof_line(root, now_ms),
    ];
    let broken = lines
        .iter()
        .filter(|line| line.status == DoctorStatus::Broken)
        .count();
    let skipped = lines
        .iter()
        .filter(|line| line.status == DoctorStatus::Skip)
        .count();
    let receipt = lines
        .iter()
        .map(DoctorLine::line)
        .collect::<Vec<_>>()
        .join("\n");
    DoctorOutput {
        root: root.display().to_string(),
        ok: broken == 0,
        broken,
        skipped,
        receipt,
        lines,
    }
}

/// (a) The adapter a run would use: the RL11 override, else the newest arena report.
fn gpu_line(root: &Path, adapter: Option<&str>) -> DoctorLine {
    if let Some(name) = adapter.map(str::trim).filter(|name| !name.is_empty()) {
        return DoctorLine::ok(
            "gpu",
            format!(
                "adapter \"{}\" from {GPU_ADAPTER_ENV}",
                one_line(name, MAX_DETAIL_CHARS)
            ),
        );
    }
    match report_adapter(root) {
        Some((file, name)) => DoctorLine::ok(
            "gpu",
            format!(
                "adapter \"{}\" from {file} (last arena run)",
                one_line(&name, MAX_DETAIL_CHARS)
            ),
        ),
        None => DoctorLine::broken(
            "gpu",
            format!(
                "no adapter named: {GPU_ADAPTER_ENV} is unset and no {REPORT_DIR}/arena-*.txt \
                 names `adapter=\"...\"`"
            ),
            "run the arena window once (cargo run -p engine-game --bin arena -- --frames 2 --exit) \
             or set WGPU_ADAPTER_NAME"
                .to_owned(),
        ),
    }
}

/// The adapter named by the newest arena report that carries one.
fn report_adapter(root: &Path) -> Option<(String, String)> {
    let mut names: Vec<String> = std::fs::read_dir(root.join(REPORT_DIR))
        .ok()?
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with("arena-") && name.ends_with(".txt"))
        .collect();
    names.sort_unstable();
    for name in names.iter().rev().take(MAX_ADAPTER_REPORTS) {
        let relative = format!("{REPORT_DIR}/{name}");
        let Ok(text) = std::fs::read_to_string(root.join(&relative)) else {
            continue;
        };
        if let Some(adapter) = adapter_in(&text) {
            return Some((relative, adapter));
        }
    }
    None
}

/// The text between `adapter="` and the next quote, when a report carries it.
fn adapter_in(text: &str) -> Option<String> {
    let start = text.find("adapter=\"")? + "adapter=\"".len();
    let rest = text.get(start..)?;
    let end = rest.find('"')?;
    let name = rest.get(..end)?.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// (b) The content root exists and is readable.
fn content_line(root: &Path) -> DoctorLine {
    match std::fs::read_dir(root.join("content")) {
        Ok(entries) => {
            let folders = entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                .count();
            DoctorLine::ok(
                "content",
                format!("content/ is readable, {folders} folders"),
            )
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => DoctorLine::broken(
            "content",
            format!("content/ is missing under {}", root.display()),
            "restore it: git checkout -- content".to_owned(),
        ),
        Err(error) => DoctorLine::broken(
            "content",
            format!("content/ is unreadable ({error})"),
            "fix the folder permissions or restore it: git checkout -- content".to_owned(),
        ),
    }
}

/// (c) Every cooked bundle parses as a current frame with sane digests.
fn bundles_line(root: &Path) -> DoctorLine {
    let mut found: Vec<PathBuf> = Vec::new();
    for relative in BUNDLE_ROOTS {
        collect_bundles(&root.join(relative), 1, &mut found);
    }
    found.sort();
    if found.is_empty() {
        return DoctorLine::skip(
            "bundles",
            "no .e2bundle under content/ or dist/web/assets".to_owned(),
            "cook one: cargo run -p engine-assets --bin e2bundle -- pack <dir> <out.e2bundle>"
                .to_owned(),
        );
    }
    let mut entries = 0u32;
    for path in &found {
        match bundle_entries(path) {
            Ok(count) => entries = entries.saturating_add(count),
            Err(fault) => {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(path)
                    .display()
                    .to_string();
                return DoctorLine::broken(
                    "bundles",
                    format!("{relative} {}", fault.detail()),
                    format!(
                        "re-cook it: cargo run -p engine-assets --bin e2bundle -- pack <its folder> \
                         {relative}"
                    ),
                );
            }
        }
    }
    DoctorLine::ok(
        "bundles",
        format!(
            "{} bundles parse as v{BUNDLE_VERSION}, {entries} entries",
            found.len()
        ),
    )
}

/// Collect `*.e2bundle` files under `dir`, descending `depth` folders.
fn collect_bundles(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            if depth > 0 {
                collect_bundles(&path, depth - 1, out);
            }
        } else if kind.is_file()
            && path.extension().and_then(|ext| ext.to_str()) == Some(BUNDLE_SUFFIX)
        {
            out.push(path);
        }
    }
}

/// Why one bundle frame refused, in the words the doctor's line prints.
enum BundleFault {
    Unreadable(std::io::Error),
    Header(u64),
    OverCap(u64),
    Magic,
    Stale(u32),
    Empty,
    Entries(u32),
    Kind(u8),
    Overflow,
    Payload(u64),
    Digest,
    Past { at: u64, len: u64 },
    Trailing(u64),
    Truncated(u64),
}

impl BundleFault {
    /// The detail printed after the bundle path.
    fn detail(&self) -> String {
        match self {
            Self::Unreadable(error) => format!("is unreadable ({error})"),
            Self::Header(len) => format!("is {len} bytes, under the 16-byte header"),
            Self::OverCap(len) => {
                format!("is {len} bytes, over the {MAX_BUNDLE_BYTES}-byte cap")
            }
            Self::Magic => "has magic other than E204BNDL".to_owned(),
            Self::Stale(version) => {
                format!("is a stale v{version} frame (the runtime reads v{BUNDLE_VERSION})")
            }
            Self::Empty => "holds no entries (a stub)".to_owned(),
            Self::Entries(count) => {
                format!("declares {count} entries, over the {MAX_BUNDLE_ENTRIES} cap")
            }
            Self::Kind(kind) => format!("has an unknown entry kind {kind}"),
            Self::Overflow => "frame offsets overflow".to_owned(),
            Self::Payload(payload) => {
                format!("has a {payload}-byte entry, over the {MAX_BUNDLE_BYTES}-byte cap")
            }
            Self::Digest => "has an entry digest that is not 64 hex chars".to_owned(),
            Self::Past { at, len } => format!("runs past its {len} bytes at offset {at}"),
            Self::Trailing(extra) => format!("has {extra} trailing bytes after the frame"),
            Self::Truncated(at) => format!("is truncated at offset {at}"),
        }
    }
}

/// Walk one bundle frame without hashing: header, entry framing and digest shape.
///
/// Returns the entry count. The runtime loader stays the authority on payload
/// digests; this answers whether the file is a current, walkable frame.
fn bundle_entries(path: &Path) -> Result<u32, BundleFault> {
    let mut file = File::open(path).map_err(BundleFault::Unreadable)?;
    let len = file.metadata().map_err(BundleFault::Unreadable)?.len();
    if len < 16 {
        return Err(BundleFault::Header(len));
    }
    if len > MAX_BUNDLE_BYTES {
        return Err(BundleFault::OverCap(len));
    }
    let mut header = [0u8; 16];
    file.read_exact(&mut header)
        .map_err(BundleFault::Unreadable)?;
    if header[..8] != BUNDLE_MAGIC {
        return Err(BundleFault::Magic);
    }
    let version = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
    if version != BUNDLE_VERSION {
        return Err(BundleFault::Stale(version));
    }
    let count = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
    if count == 0 {
        return Err(BundleFault::Empty);
    }
    if count > MAX_BUNDLE_ENTRIES {
        return Err(BundleFault::Entries(count));
    }
    let mut at = 16u64;
    for _ in 0..count {
        let mut kind = [0u8; 1];
        read_at(&mut file, at, &mut kind, len)?;
        if !(1..=MAX_BUNDLE_KIND).contains(&kind[0]) {
            return Err(BundleFault::Kind(kind[0]));
        }
        at = at.checked_add(1).ok_or(BundleFault::Overflow)?;
        let mut name_len = [0u8; 2];
        read_at(&mut file, at, &mut name_len, len)?;
        at = at
            .checked_add(2 + u64::from(u16::from_le_bytes(name_len)))
            .ok_or(BundleFault::Overflow)?;
        let mut payload_len = [0u8; 4];
        read_at(&mut file, at, &mut payload_len, len)?;
        at = at.checked_add(4).ok_or(BundleFault::Overflow)?;
        let payload = u64::from(u32::from_le_bytes(payload_len));
        if payload > MAX_BUNDLE_BYTES {
            return Err(BundleFault::Payload(payload));
        }
        at = at.checked_add(payload).ok_or(BundleFault::Overflow)?;
        let mut digest = [0u8; 64];
        read_at(&mut file, at, &mut digest, len)?;
        if !digest.iter().all(u8::is_ascii_hexdigit) {
            return Err(BundleFault::Digest);
        }
        at = at.checked_add(64).ok_or(BundleFault::Overflow)?;
        if at > len {
            return Err(BundleFault::Past { at, len });
        }
    }
    if at != len {
        return Err(BundleFault::Trailing(len.saturating_sub(at)));
    }
    Ok(count)
}

/// Read exactly `buffer.len()` bytes at `at`, refusing a read past `len`.
fn read_at(file: &mut File, at: u64, buffer: &mut [u8], len: u64) -> Result<(), BundleFault> {
    let want = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
    if at.checked_add(want).is_none_or(|end| end > len) {
        return Err(BundleFault::Truncated(at));
    }
    file.seek(SeekFrom::Start(at))
        .and_then(|_| file.read_exact(buffer))
        .map_err(BundleFault::Unreadable)
}

/// (d) The installed bridge is as new as the release build (DOOR5).
fn bridge_line(root: &Path) -> DoctorLine {
    let installed_path = root.join(BRIDGE_INSTALLED);
    let installed = match std::fs::metadata(&installed_path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return DoctorLine::skip(
                "bridge",
                format!("{BRIDGE_INSTALLED} is not installed"),
                "install it: node tools/mcp-install.mjs".to_owned(),
            );
        }
        Err(error) => {
            return DoctorLine::broken(
                "bridge",
                format!("{BRIDGE_INSTALLED} is unreadable ({error})"),
                "refresh it: node tools/mcp-install.mjs".to_owned(),
            );
        }
    };
    let release = match std::fs::metadata(root.join(BRIDGE_RELEASE)) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return DoctorLine::skip(
                "bridge",
                format!(
                    "{BRIDGE_INSTALLED} is installed, but there is no {BRIDGE_RELEASE} to compare"
                ),
                "build it (cargo build --release -p engine-endpoint --bin e2040-mcp), then run \
                 node tools/mcp-install.mjs"
                    .to_owned(),
            );
        }
        Err(error) => {
            return DoctorLine::broken(
                "bridge",
                format!("{BRIDGE_RELEASE} is unreadable ({error})"),
                "rebuild it (cargo build --release -p engine-endpoint --bin e2040-mcp)".to_owned(),
            );
        }
    };
    if installed.len() != release.len() {
        return DoctorLine::broken(
            "bridge",
            format!(
                "{BRIDGE_INSTALLED} is {} bytes, {BRIDGE_RELEASE} is {}",
                installed.len(),
                release.len()
            ),
            "refresh it: node tools/mcp-install.mjs".to_owned(),
        );
    }
    match (installed.modified(), release.modified()) {
        (Ok(installed_at), Ok(release_at)) => match installed_at.duration_since(release_at) {
            Ok(_) => DoctorLine::ok(
                "bridge",
                format!("{BRIDGE_INSTALLED} is current, not older than {BRIDGE_RELEASE}"),
            ),
            Err(behind) => DoctorLine::broken(
                "bridge",
                format!(
                    "{BRIDGE_INSTALLED} is older than {BRIDGE_RELEASE} by {}",
                    span_ms(behind.duration().as_millis())
                ),
                "refresh it: node tools/mcp-install.mjs".to_owned(),
            ),
        },
        _ => DoctorLine::skip(
            "bridge",
            format!("{BRIDGE_INSTALLED} or {BRIDGE_RELEASE} has no readable age"),
            "refresh it: node tools/mcp-install.mjs".to_owned(),
        ),
    }
}

/// (e) The last proof per crate, read from the gh-check proof log.
fn proof_line(root: &Path, now_ms: u128) -> DoctorLine {
    let entries = match std::fs::read_dir(root.join(PROOF_DIR)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return DoctorLine::skip(
                "proof",
                format!("no proof log at {PROOF_DIR}"),
                "record one: node tools/gh-check.mjs cargo test -p engine-endpoint --lib doctor"
                    .to_owned(),
            );
        }
        Err(error) => {
            return DoctorLine::broken(
                "proof",
                format!("{PROOF_DIR} is unreadable ({error})"),
                "fix the folder or delete it and record a proof again".to_owned(),
            );
        }
    };
    let mut newest: BTreeMap<String, ProofRecord> = BTreeMap::new();
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let Some(record) = read_proof(&entry.path()) else {
            continue;
        };
        for name in &record.crates {
            let stale = newest
                .get(name)
                .is_some_and(|seen| seen.at_ms >= record.at_ms);
            if !stale {
                newest.insert(name.clone(), record.clone());
            }
        }
    }
    if newest.is_empty() {
        return DoctorLine::skip(
            "proof",
            format!("{PROOF_DIR} holds no cargo -p record"),
            "record one: node tools/gh-check.mjs cargo test -p engine-endpoint --lib doctor"
                .to_owned(),
        );
    }
    let red = newest.iter().find(|(_, record)| record.code != 0);
    if let Some((name, record)) = red {
        return DoctorLine::broken(
            "proof",
            format!(
                "{name} is red {} ago ({}); {} crates proven",
                span_ms(now_ms.saturating_sub(record.at_ms)),
                one_line(&record.cmd, MAX_DETAIL_CHARS),
                newest.len()
            ),
            format!(
                "re-run it: node tools/gh-check.mjs {}",
                one_line(&record.cmd, MAX_DETAIL_CHARS)
            ),
        );
    }
    let stale = newest
        .iter()
        .find(|(_, record)| now_ms.saturating_sub(record.at_ms) > PROOF_MAX_AGE_MS);
    if let Some((name, record)) = stale {
        return DoctorLine::broken(
            "proof",
            format!(
                "{name} has no proof newer than {} (last {} ago, {}); {} crates proven",
                span_ms(PROOF_MAX_AGE_MS),
                span_ms(now_ms.saturating_sub(record.at_ms)),
                one_line(&record.cmd, MAX_DETAIL_CHARS),
                newest.len()
            ),
            format!(
                "re-run it: node tools/gh-check.mjs {}",
                one_line(&record.cmd, MAX_DETAIL_CHARS)
            ),
        );
    }
    match newest.iter().max_by_key(|(_, record)| record.at_ms) {
        Some((name, record)) => DoctorLine::ok(
            "proof",
            format!(
                "{} crates proven, newest {name} {} ago, all green",
                newest.len(),
                span_ms(now_ms.saturating_sub(record.at_ms))
            ),
        ),
        None => DoctorLine::skip(
            "proof",
            format!("{PROOF_DIR} holds no record"),
            String::new(),
        ),
    }
}

/// One gh-check result: its command, exit code, age and the crates it proved.
#[derive(Clone)]
struct ProofRecord {
    cmd: String,
    code: i64,
    at_ms: u128,
    crates: Vec<String>,
}

/// Read one proof record; `None` for a non-record, an oversize file or a command with no `-p`.
fn read_proof(path: &Path) -> Option<ProofRecord> {
    if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
        return None;
    }
    if std::fs::metadata(path).ok()?.len() > MAX_PROOF_BYTES {
        return None;
    }
    let value: rmcp::serde_json::Value =
        rmcp::serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let cmd = value.get("cmd")?.as_str()?.to_owned();
    let code = value.get("code")?.as_i64()?;
    let at_ms = u128::from(value.get("at")?.as_u64()?);
    let crates = proof_crates(&cmd);
    if crates.is_empty() {
        return None;
    }
    Some(ProofRecord {
        cmd,
        code,
        at_ms,
        crates,
    })
}

/// The crate names a `cargo ... -p <name>` line proves, in line order.
fn proof_crates(cmd: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut parts = cmd.split_whitespace();
    while let Some(part) = parts.next() {
        let name = match part {
            "-p" | "--package" => parts.next().map(|next| next.trim_matches('"')),
            _ => part
                .strip_prefix("--package=")
                .or_else(|| part.strip_prefix("-p="))
                .map(|next| next.trim_matches('"')),
        };
        let Some(name) = name else {
            continue;
        };
        if name.is_empty()
            || name.chars().count() > MAX_CRATE_CHARS
            || out.iter().any(|seen| seen == name)
        {
            continue;
        }
        out.push(name.to_owned());
    }
    out
}

/// `doctor` tool: the preflight receipt, read-only.
pub struct DoctorTool;

impl ToolBase for DoctorTool {
    type Parameter = DoctorArgs;
    type Output = DoctorOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_DOCTOR.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "What is broken before a model starts: GPU adapter, content root, bundle frames, \
             installed bridge age and the last proof per crate, one line per check with its fix. \
             Read-only."
                .into(),
        )
    }

    fn annotations() -> Option<rmcp::model::ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for DoctorTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_DOCTOR, |_state| {
            let root = super::codex::project_root(param.root.as_deref());
            let adapter = std::env::var(GPU_ADAPTER_ENV).ok();
            Ok(run_doctor(&root, unix_ms(), adapter.as_deref()))
        })
    }
}

// ---------------------------------------------------------------------------
// trace: one capture as a Chrome-trace document.
// ---------------------------------------------------------------------------

/// Frames one `perf.trace` capture may ask for; the writer's own cap is higher.
const MAX_TRACE_FRAMES: u32 = 8;

/// How many frames `perf.trace` captures.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PerfTraceArgs {
    /// Frames to capture, 1..=8; default 1.
    pub frames: Option<u32>,
}

/// What `perf.trace` answers: the trace document and what it carries.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PerfTraceOutput {
    /// Frames captured.
    pub frames: u32,
    /// `ph: "X"` slices in the document, one per measured phase per frame.
    pub slices: u32,
    /// Phase names the document carries, sorted: the tick systems and the GPU
    /// passes the capture measures.
    pub phases: Vec<String>,
    /// Document length in microseconds: the last slice end.
    pub duration_us: u64,
    /// FNV-1a over the document text, 16 hex digits.
    pub digest: String,
    /// The Chrome-trace JSON document, `process_name` metadata row first.
    pub trace: String,
}

/// `perf.trace` tool: one capture written as a Chrome-trace JSON document.
pub struct PerfTraceTool;

impl ToolBase for PerfTraceTool {
    type Parameter = PerfTraceArgs;
    type Output = PerfTraceOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_PERF_TRACE.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Capture the profiled tick once more and answer it as one Chrome-trace JSON \
             document: a process_name metadata row, then one `ph: \"X\"` slice per measured \
             phase with ts and dur in microseconds, timestamps accumulated from zero so the \
             write reads no clock. Open it in a trace viewer. Read-only."
                .into(),
        )
    }

    fn annotations() -> Option<rmcp::model::ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for PerfTraceTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_PERF_TRACE, |_state| run_perf_trace(&param))
    }
}

/// Capture `frames` and fold the rows into the answer: one slice per span, the
/// distinct phase names, the document length and the document itself.
fn run_perf_trace(args: &PerfTraceArgs) -> Result<PerfTraceOutput, McpToolError> {
    let frames = args.frames.unwrap_or(1);
    if frames == 0 || frames > MAX_TRACE_FRAMES {
        return Err(McpToolError::new(
            "bad_frames",
            format!(
                "{frames} frames is outside 1..={MAX_TRACE_FRAMES}; recovery: ask for a smaller \
                 capture"
            ),
        ));
    }
    let file = engine_perf::trace_file::capture_trace(frames).map_err(|error| {
        McpToolError::new(
            "trace_failed",
            format!("the capture refused ({error}); recovery: re-run with fewer frames"),
        )
    })?;
    let mut phases: Vec<String> = Vec::new();
    let mut slices: u32 = 0;
    let mut duration_us: u64 = 0;
    for row in file.rows() {
        if row.cat() == engine_perf::trace_file::CAT_COUNTER {
            continue;
        }
        slices = slices.saturating_add(1);
        duration_us = duration_us.max(row.ts_us().saturating_add(row.dur_us()));
        let name = row.name().to_owned();
        if !phases.contains(&name) {
            phases.push(name);
        }
    }
    phases.sort();
    let trace = file.to_json_with_process();
    Ok(PerfTraceOutput {
        frames,
        slices,
        phases,
        duration_us,
        digest: format!("{:016x}", engine_core::hash_bytes(trace.as_bytes())),
        trace,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::mcp_tools::{ApiDescribeArgs, ApiDescribeTool};
    use crate::packs::PackRegistry;
    use engine_editor::capability::Capability;
    use std::time::Duration;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    const SAMPLE_CSV: &str = "scenario,seed,ticks,agents,sprites,mean_ms,p50_ms,p99_ms,max_ms,fps_est,final_hash\nsim-1k,2040,600,1000,0,0.1227,0.1048,0.2811,0.4514,8151.4,e2f9c8b4aad1a292\nrender-10k,2040,600,0,10000,1.2182,1.0960,2.2193,3.4160,820.9,de383c47b516dbf4\nmixed-1k-10k,2040,600,1000,10000,1.1598,1.1040,1.9442,3.0056,862.2,3b804bc8e928217d\n";

    #[test]
    fn perf_table_parses_the_quiet_csv_into_integer_rows() {
        let rows = parse_perf_csv(SAMPLE_CSV).expect("sample parses");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].scenario, "sim-1k");
        assert_eq!((rows[0].p50_us, rows[0].p99_us), (104, 281));
        assert_eq!(rows[0].final_hash, "e2f9c8b4aad1a292");
        assert_eq!(rows[1].scenario, "render-10k");
        assert_eq!((rows[1].p50_us, rows[1].p99_us), (1096, 2219));
        assert_eq!(rows[1].final_hash, "de383c47b516dbf4");
        assert_eq!(rows[2].scenario, "mixed-1k-10k");
        assert_eq!((rows[2].p50_us, rows[2].p99_us), (1104, 1944));
        assert_eq!(rows[2].final_hash, "3b804bc8e928217d");
    }

    #[test]
    fn perf_table_ignores_a_renamed_workload_column() {
        let renamed = SAMPLE_CSV.replacen("fps_est", "cpu_workload_rate", 1);
        assert!(renamed.contains("cpu_workload_rate"));
        let before = parse_perf_csv(SAMPLE_CSV).expect("sample parses");
        let after = parse_perf_csv(&renamed).expect("renamed parses");
        assert_eq!(before, after);
    }

    #[test]
    fn perf_table_refuses_bad_csv_and_bad_paths() {
        let missing = SAMPLE_CSV.replacen("p99_ms,", "", 1);
        let error = parse_perf_csv(&missing).expect_err("missing column refuses");
        assert_eq!(error.code, "bad_csv");
        let short = "scenario,p50_ms,p99_ms,final_hash\nsim-1k,0.1048,0.2811\n";
        let error = parse_perf_csv(short).expect_err("short row refuses");
        assert_eq!(error.code, "bad_csv");
        assert!(error.message.contains("line 2"));
        let bad_num = "scenario,p50_ms,p99_ms,final_hash\nsim-1k,abc,0.2811,e2f9c8b4aad1a292\n";
        let error = parse_perf_csv(bad_num).expect_err("abc refuses");
        assert_eq!(error.code, "bad_csv");
        for bad in ["", "..", "a/../b", "/abs", "C:x", "x.txt"] {
            let error = check_csv_path("csv", bad).expect_err(bad);
            assert_eq!(error.code, "bad_args", "{bad:?}");
            assert!(error.message.contains("re-run"), "{bad:?}");
        }
    }

    #[test]
    fn perf_table_picks_the_newest_csv_and_reads_twice_equal() {
        let base = std::env::temp_dir().join(format!("e2040-perf-table-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let reports = base.join("target/reports");
        std::fs::create_dir_all(&reports).expect("reports create");
        let old_name = "perf-2026-10-10T1826Z-1.csv";
        let new_name = "perf-2026-10-10T1827Z-1.csv";
        let old_csv = "scenario,seed,ticks,agents,sprites,mean_ms,p50_ms,p99_ms,max_ms,fps_est,final_hash\nold-1k,2040,600,1000,0,0.1227,1.0000,2.0000,0.4514,8151.4,aaaaaaaaaaaaaaaa\n";
        std::fs::write(reports.join(old_name), old_csv).expect("old writes");
        std::fs::write(reports.join(new_name), SAMPLE_CSV).expect("new writes");
        let args = PerfTableArgs { csv: None };
        let first = run_perf_table(&args, &base).expect("newest reads");
        assert!(
            first.source.contains(new_name),
            "greater name wins: {}",
            first.source
        );
        assert_eq!(first.rows.len(), 3);
        assert_eq!(first.rows[0].scenario, "sim-1k");
        let second = run_perf_table(&args, &base).expect("reads twice");
        assert_eq!(first, second, "two calls give equal output");
        let empty_base =
            std::env::temp_dir().join(format!("e2040-perf-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&empty_base);
        std::fs::create_dir_all(empty_base.join("target/reports")).expect("empty reports create");
        let error = run_perf_table(&args, &empty_base).expect_err("empty refuses");
        assert_eq!(error.code, "no_perf_csv");
        assert!(error
            .message
            .contains("cargo run --release -p engine-perf --bin perf"));
        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&empty_base);
    }

    #[test]
    fn perf_table_is_a_listed_read_only_verb_in_the_perf_pack() {
        let verb = PackRegistry::shipped()
            .verb(TOOL_PERF_TABLE)
            .expect("perf.table is a registered verb");
        assert_eq!(verb.pack(), PACK);
        assert_eq!(verb.card(), CapabilityCard::Observe);
        assert!(verb.readonly());
        assert!(PackRegistry::shipped()
            .pack(PACK)
            .expect("perf pack")
            .verb_names()
            .contains(&TOOL_PERF_TABLE));
        let described = ApiDescribeTool::invoke(
            &server(),
            ApiDescribeArgs {
                path: TOOL_PERF_TABLE.to_owned(),
            },
        )
        .expect("api.describe lists perf.table");
        assert_eq!(described.path, TOOL_PERF_TABLE);
        assert!(described.docs.contains("p50"));
        assert!(described.signature.contains("perf"));
    }

    // ---- doctor -----------------------------------------------------------

    /// A valid one-entry bundle digest (64 lowercase hex chars).
    const GOOD_DIGEST: &[u8; 64] =
        b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("e2040-doctor-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("fixture root creates");
        root
    }

    /// One bundle frame: magic, `version`, one entry per name with a good digest.
    fn write_bundle(path: &Path, version: u32, names: &[&str]) {
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(b"E204BNDL");
        bytes.extend_from_slice(&version.to_le_bytes());
        bytes.extend_from_slice(
            &u32::try_from(names.len())
                .expect("fixture count")
                .to_le_bytes(),
        );
        for name in names {
            bytes.push(1);
            bytes.extend_from_slice(
                &u16::try_from(name.len())
                    .expect("fixture name")
                    .to_le_bytes(),
            );
            bytes.extend_from_slice(name.as_bytes());
            bytes.extend_from_slice(&7u32.to_le_bytes());
            bytes.extend_from_slice(b"payload");
            bytes.extend_from_slice(GOOD_DIGEST);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("bundle folder");
        }
        std::fs::write(path, bytes).expect("bundle writes");
    }

    /// Move a file's mtime to `minutes_ago` before `now_ms`.
    fn set_age(path: &Path, now_ms: u128, minutes_ago: u64) {
        let when = UNIX_EPOCH
            + Duration::from_millis(u64::try_from(now_ms).expect("fixture clock"))
            - Duration::from_secs(minutes_ago * 60);
        std::fs::File::options()
            .write(true)
            .open(path)
            .expect("fixture file opens")
            .set_modified(when)
            .expect("fixture mtime sets");
    }

    /// One gh-check proof record.
    fn write_proof(path: &Path, code: i64, cmd: &str, at_ms: u128) {
        let at = u64::try_from(at_ms).expect("fixture clock");
        std::fs::write(
            path,
            format!("{{\"code\":{code},\"cmd\":\"{cmd}\",\"at\":{at}}}\n"),
        )
        .expect("proof writes");
    }

    /// A tree every doctor check passes on.
    fn doctor_fixture(name: &str) -> (PathBuf, u128) {
        let root = fixture(name);
        let now_ms = unix_ms();
        std::fs::create_dir_all(root.join("content/arena")).expect("content folder");
        write_bundle(
            &root.join("content/arena/kit.e2bundle"),
            BUNDLE_VERSION,
            &["kit"],
        );
        let reports = root.join(REPORT_DIR);
        std::fs::create_dir_all(&reports).expect("reports folder");
        std::fs::write(
            reports.join("arena-20260101T0000Z-1.txt"),
            "arena per-pass: adapter=\"Fixture GPU 9000\" driver=\"1.0\"\n",
        )
        .expect("report writes");
        std::fs::create_dir_all(root.join(".opencode/bin")).expect("bridge folder");
        std::fs::create_dir_all(root.join("target/release")).expect("release folder");
        std::fs::write(root.join(BRIDGE_INSTALLED), b"bridge").expect("installed writes");
        std::fs::write(root.join(BRIDGE_RELEASE), b"bridge").expect("release writes");
        set_age(&root.join(BRIDGE_INSTALLED), now_ms, 60);
        set_age(&root.join(BRIDGE_RELEASE), now_ms, 120);
        let proofs = root.join(PROOF_DIR);
        std::fs::create_dir_all(&proofs).expect("proof folder");
        write_proof(
            &proofs.join("green.json"),
            0,
            "cargo test -p engine-endpoint --lib doctor",
            now_ms - 3_600_000,
        );
        (root, now_ms)
    }

    fn check_line<'a>(output: &'a DoctorOutput, check: &str) -> &'a DoctorLine {
        output
            .lines
            .iter()
            .find(|line| line.check == check)
            .unwrap_or_else(|| panic!("no `{check}` line in {}", output.receipt))
    }

    #[test]
    fn doctor_answers_five_ok_lines_and_reads_twice_equal() {
        let (root, now_ms) = doctor_fixture("healthy");
        let first = run_doctor(&root, now_ms, None);
        assert!(first.ok, "healthy fixture:\n{}", first.receipt);
        assert_eq!(first.lines.len(), 5, "one line per check");
        assert_eq!(first.broken, 0);
        assert_eq!(first.skipped, 0);
        assert_eq!(first.receipt.lines().count(), 5, "one line per check");
        for line in &first.lines {
            assert_eq!(line.status, DoctorStatus::Ok, "{}", line.line());
            assert!(!line.line().contains('\n'), "{}", line.line());
        }
        assert!(first.receipt.contains("adapter \"Fixture GPU 9000\""));
        assert!(first.receipt.contains("bundles parse as v1"));
        assert!(first.receipt.contains("is current"));
        assert!(first.receipt.contains("all green"));
        let second = run_doctor(&root, now_ms, None);
        assert_eq!(first, second, "same tree and clock, same receipt");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Each check is read once as broken, with its fix on the same line.
    #[test]
    fn doctor_breaks_a_missing_content_root_with_its_fix() {
        let (root, now_ms) = doctor_fixture("no-content");
        let _ = std::fs::remove_dir_all(root.join("content"));
        let output = run_doctor(&root, now_ms, None);
        let line = check_line(&output, "content");
        assert_eq!(line.status, DoctorStatus::Broken, "{}", line.line());
        assert!(
            line.detail.contains("content/ is missing"),
            "{}",
            line.line()
        );
        assert!(
            line.fix.contains("git checkout -- content"),
            "{}",
            line.line()
        );
        assert!(!output.ok, "a broken check means not ok");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn doctor_breaks_a_stale_bundle_with_its_fix() {
        let (root, now_ms) = doctor_fixture("stale-bundle");
        write_bundle(&root.join("content/arena/kit.e2bundle"), 0, &["kit"]);
        let output = run_doctor(&root, now_ms, None);
        let line = check_line(&output, "bundles");
        assert_eq!(line.status, DoctorStatus::Broken, "{}", line.line());
        assert!(line.detail.contains("stale v0 frame"), "{}", line.line());
        assert!(line.fix.contains("e2bundle -- pack"), "{}", line.line());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn doctor_breaks_an_absent_adapter_report_with_its_fix() {
        let (root, now_ms) = doctor_fixture("no-adapter");
        let _ = std::fs::remove_dir_all(root.join(REPORT_DIR));
        let output = run_doctor(&root, now_ms, None);
        let line = check_line(&output, "gpu");
        assert_eq!(line.status, DoctorStatus::Broken, "{}", line.line());
        assert!(line.detail.contains("no adapter named"), "{}", line.line());
        assert!(line.fix.contains("--frames 2 --exit"), "{}", line.line());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn doctor_skips_a_missing_proof_log_and_breaks_a_red_or_stale_one() {
        let (root, now_ms) = doctor_fixture("proof-missing");
        let _ = std::fs::remove_dir_all(root.join(PROOF_DIR));
        let output = run_doctor(&root, now_ms, None);
        let line = check_line(&output, "proof");
        assert_eq!(line.status, DoctorStatus::Skip, "{}", line.line());
        assert!(line.fix.contains("gh-check.mjs"), "{}", line.line());
        let _ = std::fs::remove_dir_all(&root);

        let (root, now_ms) = doctor_fixture("proof-red");
        write_proof(
            &root.join(PROOF_DIR).join("red.json"),
            1,
            "cargo test -p engine-render --lib contact_shadow",
            now_ms - 600_000,
        );
        let output = run_doctor(&root, now_ms, None);
        let line = check_line(&output, "proof");
        assert_eq!(line.status, DoctorStatus::Broken, "{}", line.line());
        assert!(
            line.detail.contains("engine-render is red"),
            "{}",
            line.line()
        );
        assert!(
            line.fix
                .contains("node tools/gh-check.mjs cargo test -p engine-render"),
            "{}",
            line.line()
        );
        let _ = std::fs::remove_dir_all(&root);

        let (root, now_ms) = doctor_fixture("proof-stale");
        let _ = std::fs::remove_dir_all(root.join(PROOF_DIR));
        std::fs::create_dir_all(root.join(PROOF_DIR)).expect("proof folder");
        write_proof(
            &root.join(PROOF_DIR).join("old.json"),
            0,
            "cargo test -p engine-endpoint --lib doctor",
            now_ms - 100 * 3_600_000,
        );
        let output = run_doctor(&root, now_ms, None);
        let line = check_line(&output, "proof");
        assert_eq!(line.status, DoctorStatus::Broken, "{}", line.line());
        assert!(
            line.detail.contains("no proof newer than"),
            "{}",
            line.line()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn doctor_breaks_a_stale_installed_bridge_and_skips_an_absent_one() {
        let (root, now_ms) = doctor_fixture("stale-bridge");
        set_age(&root.join(BRIDGE_INSTALLED), now_ms, 180);
        set_age(&root.join(BRIDGE_RELEASE), now_ms, 60);
        let output = run_doctor(&root, now_ms, None);
        let line = check_line(&output, "bridge");
        assert_eq!(line.status, DoctorStatus::Broken, "{}", line.line());
        assert!(line.detail.contains("is older than"), "{}", line.line());
        assert!(
            line.fix.contains("node tools/mcp-install.mjs"),
            "{}",
            line.line()
        );
        let _ = std::fs::remove_file(root.join(BRIDGE_INSTALLED));
        let output = run_doctor(&root, now_ms, None);
        let line = check_line(&output, "bridge");
        assert_eq!(line.status, DoctorStatus::Skip, "{}", line.line());
        assert!(
            line.fix.contains("node tools/mcp-install.mjs"),
            "{}",
            line.line()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn doctor_is_a_listed_read_only_verb_in_the_perf_pack() {
        let verb = PackRegistry::shipped()
            .verb(TOOL_DOCTOR)
            .expect("doctor is a registered verb");
        assert_eq!(verb.pack(), PACK);
        assert_eq!(verb.card(), CapabilityCard::Observe);
        assert!(verb.readonly());
        assert!(PackRegistry::shipped()
            .pack(PACK)
            .expect("perf pack")
            .verb_names()
            .contains(&TOOL_DOCTOR));
        let described = ApiDescribeTool::invoke(
            &server(),
            ApiDescribeArgs {
                path: TOOL_DOCTOR.to_owned(),
            },
        )
        .expect("api.describe lists doctor");
        assert_eq!(described.path, TOOL_DOCTOR);
        assert!(described.docs.contains("broken"));
        assert!(described.signature.contains("perf"));
    }

    #[test]
    fn trace_writes_a_chrome_trace_with_a_process_row_and_phase_slices() {
        let out = PerfTraceTool::invoke(&server(), PerfTraceArgs { frames: Some(1) })
            .expect("one frame captures");
        assert_eq!(out.frames, 1);
        assert_eq!(out.digest.len(), 16);
        assert!(
            out.trace.starts_with("{\"traceEvents\":["),
            "{}",
            &out.trace[..40.min(out.trace.len())]
        );
        // One process_name metadata row, and the row is the first event.
        assert!(out.trace.contains("\"name\":\"process_name\""), "metadata");
        assert!(out.trace.contains("\"ph\":\"M\""), "metadata phase");
        assert!(out.trace.contains(engine_perf::trace_file::PROCESS_NAME));
        // One `ph: "X"` slice per measured phase, at least the eight phases of
        // a tick, and the document carries exactly the slices it reports.
        assert!(out.slices >= 8, "{} slices", out.slices);
        assert!(out.phases.len() >= 8, "{:?}", out.phases);
        let slices = out.trace.matches("\"ph\":\"X\"").count();
        assert_eq!(
            slices,
            usize::try_from(out.slices).expect("slices fit"),
            "{slices}"
        );
        // Timestamps run from zero and every duration is a whole number.
        assert!(out.trace.contains("\"ts\":0,"), "timestamps start at zero");
        for chunk in out.trace.split("\"dur\":").skip(1) {
            let digits: String = chunk.chars().take_while(char::is_ascii_digit).collect();
            assert!(!digits.is_empty(), "a duration is a number: {chunk}");
        }
        assert!(out.duration_us > 0, "the document has length");
        // A bad frame count refuses before any capture runs.
        let refused = PerfTraceTool::invoke(&server(), PerfTraceArgs { frames: Some(0) })
            .expect_err("zero frames refuses");
        assert_eq!(refused.code, "bad_frames");
    }
}

// WG2: this pack publishes its own row; `packs/mod.rs` keeps no hand list.
#[expect(
    unsafe_code,
    reason = "linkme publishes a pack row through a link section"
)]
#[linkme::distributed_slice(super::PACK_MODULES)]
static PACK_ROW: super::PackModule = super::PackModule {
    order: 11,
    name: PACK,
    default_loaded: false,
    tools: &["perf.table", "perf.doctor", "perf.trace"],
    build: pack,
};
