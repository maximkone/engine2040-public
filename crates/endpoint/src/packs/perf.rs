//! `perf` pack: quiet perf runs as typed rows.
//!
//! One read-only verb `perf.table` returns the rows of the newest quiet perf
//! csv as typed integer rows, so an agent reads p50 and p99 per scenario
//! through `api.call` instead of opening a file.
//!
//! Units: integer microseconds for `p50_us` and `p99_us`, chars for paths.
//! Determinism: the csv is read as bytes; the same file always parses to the
//! same rows in file order, and the newest file is the lexicographically
//! greatest `perf-*.csv` name. Errors: [`McpToolError`] with `bad_args` for a
//! bad `csv` path, `bad_csv` for a missing or malformed csv, `no_perf_csv`
//! when no csv exists.

use std::borrow::Cow;
use std::path::{Component, Path};

use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;

/// Pack name.
pub const PACK: &str = "perf";
/// Verb: quiet perf rows as typed integers.
pub const TOOL_PERF_TABLE: &str = "perf.table";

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

fn bad_args(message: String) -> McpToolError {
    McpToolError::new("bad_args", message)
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
            && best.as_ref().map_or(true, |seen| name > *seen)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::mcp_tools::{ApiDescribeArgs, ApiDescribeTool};
    use crate::packs::PackRegistry;
    use engine_editor::capability::Capability;

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
}
