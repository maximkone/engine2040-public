//! Versioned trace export for the eval harness: one line per report row.
//!
//! Consumer: offline tuning passes read this text; the harness suite
//! round-trips it. Data owner: the caller owns the report; this module
//! borrows it for export and builds a fresh one on import. Units:
//! milliseconds for latency, minor cost units (`unknown` when unreported).
//! Determinism: export is pure in the report; import resolves labels to the
//! canonical static strings, so export-import-export is byte-identical.
//! Errors: typed [`EvalError`](super::EvalError) variants only.

use super::{
    backend_label, Cost, EvalError, EvalReport, EvalRow, ExternalPass, AUTHORING_TASKS,
    MAX_DETAIL_CHARS, PLAY_TASKS,
};
use crate::agent::Backend;

/// Trace format magic heading every export line 1.
pub const TRACE_MAGIC: &str = "e2040-ev1-trace";
/// Trace format version this build writes and reads.
pub const TRACE_VERSION: u32 = 1;
/// Column header for row lines: tab-separated field names in export order.
const TRACE_COLUMNS: &str =
    "task\tflow\tbackend\tcompleted\trejected\tlatency_ms\tcost\tprivacy\tdetail";
/// Largest accepted row count in one trace; the harness writes 75.
const TRACE_MAX_ROWS: usize = 1024;

/// Format (every line ends with `\n`, UTF-8):
/// line 1: `e2040-ev1-trace v1 rows=<n>` where `<n>` counts the row lines.
/// line 2: the tab-separated column header.
/// next `<n>` lines: one row per report row in report order, tab-separated
/// `task flow backend completed rejected latency_ms cost privacy detail`.
/// Each row carries the task id, the transcript detail (fork rows record
/// their fork metrics there: stuck ticks and damage sums), and the verdict
/// (`completed`, `rejected`).
/// last line: `external-mcp` plus tab-separated `completed found latency_ms
/// tools_listed notes`.
/// Booleans render `true`/`false`, cost renders `unknown` or decimal minor
/// units, integers render decimal. Free text (`detail`, `notes`) escapes
/// backslash as `\\`, tab as `\t`, carriage return as `\r` and newline as
/// `\n`; any other escape or a dangling backslash is a parse error. A
/// trailing carriage return per line is tolerated (import normalizes it)
/// but never written, so re-exports stay byte-identical.
fn escape_trace_field(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out
}

/// Inverse of [`escape_trace_field`]; unknown escapes fail typed.
///
/// Errors: [`EvalError::TraceParse`] naming the line and the legal escapes.
fn unescape_trace_field(text: &str, line_no: usize) -> Result<String, EvalError> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            _ => {
                return Err(EvalError::TraceParse {
                    reason: format!(
                        "line {line_no}: bad escape in free-text field; legal escapes are \\\\ \\t \\r \\n"
                    ),
                });
            }
        }
    }
    Ok(out)
}

/// Drop one trailing carriage return so CRLF-authored traces read the same
/// as LF ones; the export itself only writes `\n`.
fn strip_trace_cr(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

/// Parse line 1 into the declared row count; foreign versions fail typed.
///
/// Errors: [`EvalError::TraceParse`] for a malformed header,
/// [`EvalError::TraceVersion`] for a version this build cannot read.
fn parse_trace_header(line: &str) -> Result<usize, EvalError> {
    let line = strip_trace_cr(line);
    let bad = || EvalError::TraceParse {
        reason: format!("line 1: want `{TRACE_MAGIC} v{TRACE_VERSION} rows=<n>`; got `{line}`"),
    };
    let rest = line.strip_prefix(TRACE_MAGIC).ok_or_else(bad)?;
    let rest = rest.strip_prefix(' ').ok_or_else(bad)?;
    let (version_text, rows_text) = rest.split_once(' ').ok_or_else(bad)?;
    let version_digits = version_text.strip_prefix('v').ok_or_else(bad)?;
    if version_digits != TRACE_VERSION.to_string() {
        return Err(EvalError::TraceVersion {
            want: format!("v{TRACE_VERSION}"),
            got: version_text.to_owned(),
        });
    }
    let count_text = rows_text.strip_prefix("rows=").ok_or_else(bad)?;
    let count: usize = count_text.parse().map_err(|_| bad())?;
    if count > TRACE_MAX_ROWS {
        return Err(EvalError::TraceParse {
            reason: format!("line 1: rows={count} exceeds the {TRACE_MAX_ROWS}-row bound"),
        });
    }
    Ok(count)
}

/// Parse one `true`/`false` field; anything else fails typed.
///
/// Errors: [`EvalError::TraceParse`] naming the line and field.
fn parse_trace_bool(text: &str, line_no: usize, field: &str) -> Result<bool, EvalError> {
    match text {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(EvalError::TraceParse {
            reason: format!("line {line_no}: field `{field}` wants true|false; got `{text}`"),
        }),
    }
}

/// Parse one decimal integer field; signs, spaces and junk fail typed.
///
/// Errors: [`EvalError::TraceParse`] naming the line and field.
fn parse_trace_u64(text: &str, line_no: usize, field: &str) -> Result<u64, EvalError> {
    text.parse::<u64>().map_err(|_| EvalError::TraceParse {
        reason: format!("line {line_no}: field `{field}` wants a decimal integer; got `{text}`"),
    })
}

/// Parse the cost field: `unknown` or decimal minor units.
///
/// Errors: [`EvalError::TraceParse`] naming the line.
fn parse_trace_cost(text: &str, line_no: usize) -> Result<Cost, EvalError> {
    if text == "unknown" {
        Ok(Cost::Unknown)
    } else {
        parse_trace_u64(text, line_no, "cost").map(Cost::Units)
    }
}

/// Parse one tab-separated row line, resolving task, flow and backend text
/// back to the canonical static strings so no parsed bytes become
/// long-lived references.
///
/// Errors: [`EvalError::TraceParse`] naming the line and the legal shape.
fn parse_trace_row(line: &str, line_no: usize) -> Result<EvalRow, EvalError> {
    let line = strip_trace_cr(line);
    let bad = |what: &str| EvalError::TraceParse {
        reason: format!("line {line_no}: {what}"),
    };
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != 9 {
        return Err(bad(
            "row wants 9 tab-separated fields; see the column header",
        ));
    }
    let task = AUTHORING_TASKS
        .iter()
        .chain(PLAY_TASKS.iter())
        .find(|task| task.id == fields[0])
        .ok_or_else(|| bad("unknown task id; run the 20 authoring plus 5 play ids"))?;
    if fields[1] != task.flow.as_str() {
        return Err(bad("flow label does not match the task table"));
    }
    let backend = match fields[2] {
        "mock" => backend_label(Backend::Mock),
        "local" => backend_label(Backend::Local),
        "remote" => backend_label(Backend::Remote),
        _ => return Err(bad("backend wants mock|local|remote")),
    };
    let completed = parse_trace_bool(fields[3], line_no, "completed")?;
    let rejected = parse_trace_bool(fields[4], line_no, "rejected")?;
    let latency_ms = parse_trace_u64(fields[5], line_no, "latency_ms")?;
    let cost = parse_trace_cost(fields[6], line_no)?;
    let privacy_failure = parse_trace_bool(fields[7], line_no, "privacy")?;
    let detail = unescape_trace_field(fields[8], line_no)?;
    if detail.chars().count() > MAX_DETAIL_CHARS {
        return Err(bad("detail exceeds the bounded length"));
    }
    Ok(EvalRow {
        task: task.id,
        flow: task.flow.as_str(),
        backend,
        completed,
        rejected,
        latency_ms,
        cost,
        privacy_failure,
        detail,
    })
}

/// Parse the trailing `external-mcp` line.
///
/// Errors: [`EvalError::TraceParse`] naming the line and the legal shape.
fn parse_trace_external(line: &str, line_no: usize) -> Result<ExternalPass, EvalError> {
    let line = strip_trace_cr(line);
    let bad = |what: &str| EvalError::TraceParse {
        reason: format!("line {line_no}: {what}"),
    };
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != 6 || fields[0] != "external-mcp" {
        return Err(bad(
            "last line wants `external-mcp` plus 5 tab-separated fields",
        ));
    }
    let completed = parse_trace_bool(fields[1], line_no, "completed")?;
    let found = parse_trace_bool(fields[2], line_no, "found")?;
    let latency_ms = parse_trace_u64(fields[3], line_no, "latency_ms")?;
    let tools_listed = u32::try_from(parse_trace_u64(fields[4], line_no, "tools_listed")?)
        .map_err(|_| bad("tools_listed is out of range"))?;
    let notes = unescape_trace_field(fields[5], line_no)?;
    if notes.chars().count() > MAX_DETAIL_CHARS {
        return Err(bad("notes exceed the bounded length"));
    }
    Ok(ExternalPass {
        completed,
        found,
        latency_ms,
        tools_listed,
        notes,
    })
}

/// Render one report row in column order with the free text escaped.
fn export_trace_row(row: &EvalRow) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        row.task,
        row.flow,
        row.backend,
        row.completed,
        row.rejected,
        row.latency_ms,
        row.cost,
        row.privacy_failure,
        escape_trace_field(&row.detail),
    )
}

/// Export a full report as versioned trace text for offline tuners: every
/// task row (task, transcript detail with fork metrics, verdict) plus the
/// external pass. Pure in the report: the same report twice gives the same
/// bytes. This path allocates the output string; it never runs in the tick.
pub fn export_trace(report: &EvalReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{TRACE_MAGIC} v{TRACE_VERSION} rows={}\n",
        report.rows.len()
    ));
    out.push_str(TRACE_COLUMNS);
    out.push('\n');
    for row in &report.rows {
        out.push_str(&export_trace_row(row));
    }
    out.push_str(&format!(
        "external-mcp\t{}\t{}\t{}\t{}\t{}\n",
        report.external.completed,
        report.external.found,
        report.external.latency_ms,
        report.external.tools_listed,
        escape_trace_field(&report.external.notes),
    ));
    out
}

/// Re-import text from [`export_trace`]: the header version must match
/// [`TRACE_VERSION`], the column header must match exactly, and the body
/// must hold exactly the declared row count plus the external line.
/// Unknown versions fail typed [`EvalError::TraceVersion`] before any row
/// is touched; anything else malformed fails typed
/// [`EvalError::TraceParse`]. Task, flow and backend text resolve to the
/// canonical static strings, so the round trip is byte-identical.
///
/// Errors: [`EvalError::TraceVersion`] for foreign headers,
/// [`EvalError::TraceParse`] for malformed bodies.
pub fn import_trace(text: &str) -> Result<EvalReport, EvalError> {
    let Some(body) = text.strip_suffix('\n') else {
        return Err(EvalError::TraceParse {
            reason: "trace is truncated: missing final newline; the export ends every line with a newline"
                .to_owned(),
        });
    };
    let lines: Vec<&str> = body.split('\n').collect();
    if lines.len() < 3 {
        return Err(EvalError::TraceParse {
            reason: "trace is truncated: want header plus columns plus rows".to_owned(),
        });
    }
    let count = parse_trace_header(lines[0])?;
    if strip_trace_cr(lines[1]) != TRACE_COLUMNS {
        return Err(EvalError::TraceParse {
            reason: "line 2: column header does not match the export shape".to_owned(),
        });
    }
    if lines.len() != 3 + count {
        return Err(EvalError::TraceParse {
            reason: format!(
                "header rows={count} but the body holds {} row lines",
                lines.len().saturating_sub(3)
            ),
        });
    }
    let mut rows = Vec::with_capacity(count);
    for (offset, line) in lines[2..2 + count].iter().enumerate() {
        rows.push(parse_trace_row(line, offset + 3)?);
    }
    let external = parse_trace_external(lines[2 + count], count + 3)?;
    Ok(EvalReport { rows, external })
}
