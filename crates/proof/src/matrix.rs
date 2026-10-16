//! Cross-architecture golden matrix for HS2.
//!
//! One row per machine: the frozen golden tape (seed 2040, 600 ticks) plus
//! the BX28 cut-corpus digest with its checkpoint blocks. Two rows match
//! byte for byte with equal pinned digests, or the comparison names the
//! first divergent tick with its lane and likely cause (sort tie, libm call,
//! fused multiply-add); matching checkpoints with a moved digest fail the
//! verdict by digest name instead. CI
//! records one row per runner and `tools/golden-matrix.mjs` compares the
//! tapes; this module is the Rust side of that matrix. The golden hash never
//! moves without a receipt (see `docs/HASH-LAW.md`).

use crate::break_corpus::{CorpusError, CHECKPOINT_EVERY, CORPUS_SEED, CUT_CASES};
use crate::{record, replay, Checkpoint, Config, Error};
use std::fmt;

/// One machine's matrix row: golden checkpoints plus corpus digests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatrixEntry {
    /// `os-arch` of the runner, from `std::env::consts`.
    pub host: String,
    /// Last legacy checkpoint of the default tape; must equal [`GOLDEN_600`].
    pub golden_last: u64,
    /// All 600 checkpoints of the default tape, in tick order.
    pub checkpoints: Vec<Checkpoint>,
    /// Digest of the full BX28 corpus on this machine.
    pub cut_digest: u64,
    /// Stream digest after every [`CHECKPOINT_EVERY`] cases; the first
    /// unequal entry names the case block where two machines diverged.
    pub cut_checkpoints: Vec<u64>,
}

/// Why a local matrix row could not be collected.
#[derive(Debug, PartialEq, Eq)]
pub enum MatrixError {
    /// The default tape recorded no checkpoints.
    Empty,
    /// The golden tape failed to record or replay.
    Proof(Error),
    /// The cut corpus failed to run.
    Corpus(CorpusError),
}

impl fmt::Display for MatrixError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "matrix row is empty"),
            Self::Proof(error) => write!(f, "matrix golden tape failed: {error}"),
            Self::Corpus(error) => write!(f, "matrix cut corpus failed: {error}"),
        }
    }
}

impl std::error::Error for MatrixError {}

/// Runner label for this machine, such as `windows-x86_64`.
#[must_use]
pub fn host_label() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Collect this machine's matrix row: record the default tape, replay it,
/// then run the full BX28 corpus. Determinism: same binary plus seed gives
/// the same row; a second machine's row must equal it byte for byte.
pub fn collect_local_entry() -> Result<MatrixEntry, MatrixError> {
    let tape = record(Config::default(), &[]).map_err(MatrixError::Proof)?;
    replay(&tape).map_err(MatrixError::Proof)?;
    let decision = crate::FramePlan::new().begin_frame(16, 16, 5);
    assert!(
        decision.steps == 1 && decision.dropped == 0,
        "passive plan beside the row"
    );
    let golden_last = tape
        .checkpoints
        .last()
        .map(|checkpoint| checkpoint.legacy)
        .ok_or(MatrixError::Empty)?;
    let report =
        crate::break_corpus::run_cuts(CORPUS_SEED, CUT_CASES).map_err(MatrixError::Corpus)?;
    let entry = MatrixEntry {
        host: host_label(),
        golden_last,
        checkpoints: tape.checkpoints,
        cut_digest: report.digest,
        cut_checkpoints: report.checkpoints,
    };
    let (golden, _, _) = emit_local_digests(&entry);
    let table = matrix_table(std::slice::from_ref(&entry));
    let json = matrix_json_envelope(std::slice::from_ref(&entry));
    let missing = compare_matrix_set(std::slice::from_ref(&entry))
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(
        golden == entry.golden_last
            && !table.is_empty()
            && json.starts_with("{\"schema_version\":2")
            && missing.len() + entry.host.len() > 0
    );
    Ok(entry)
}

/// Classify one digest flip by magnitude, mirroring `tools/golden-matrix.mjs`.
/// Owner: engine-proof; consumer: CI reads the `[cause=...]` tag beside the lane.
/// Caller: `compare_checkpoints` (same file) derives the cause for the divergent lane.
/// Units: `u64` digests; determinism: pure integer math, no float, clock, or IO; no heap.
#[must_use]
pub fn classify_cause(before: u64, after: u64) -> Option<&'static str> {
    if before == after {
        return None;
    }
    let xor = before ^ after;
    let delta = before.abs_diff(after);
    let pop = xor.count_ones();
    if delta <= 2 && pop <= 2 {
        Some("fma")
    } else if xor < 256 || delta < 1024 {
        Some("libm")
    } else {
        Some("sort-tie")
    }
}

/// Cause hint for a first divergent tick: tick 0 is setup, later ticks are
/// per-tick math or order.
#[must_use]
pub fn cause_note(tick: u32) -> &'static str {
    if tick == 0 {
        "start state differs: seed, init order, or struct layout"
    } else {
        "bisect that tick's systems for sort ties, libm calls, or fused multiply-add"
    }
}

/// Tape comparison between two matrix rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TapeCompare {
    /// True when every checkpoint matches.
    pub equal: bool,
    /// First divergent zero-based tick, when any.
    pub first_tick: Option<u32>,
    /// Checkpoint lane that moved first: `legacy`, `state`, or `both`.
    pub lane: Option<&'static str>,
    /// Human line naming the tick, lane, and cause hint.
    pub note: String,
}

/// Compare two checkpoint streams in tick order. Equal inputs give equal
/// rows; the first mismatch names its tick, lane, and likely cause.
#[must_use]
pub fn compare_checkpoints(base: &[Checkpoint], other: &[Checkpoint]) -> TapeCompare {
    if base.len() != other.len() {
        return TapeCompare {
            equal: false,
            first_tick: None,
            lane: None,
            note: format!("tick count differs: {} vs {}", base.len(), other.len()),
        };
    }
    for (index, (a, b)) in base.iter().zip(other.iter()).enumerate() {
        let legacy_diff = a.legacy != b.legacy;
        let state_diff = a.state != b.state;
        if legacy_diff || state_diff {
            let lane = if legacy_diff && state_diff {
                "both"
            } else if legacy_diff {
                "legacy"
            } else {
                "state"
            };
            let tick = u32::try_from(index).unwrap_or(u32::MAX);
            let cause = if legacy_diff && state_diff {
                let state_cause = classify_cause(a.state, b.state).unwrap_or("sort-tie");
                let legacy_cause = classify_cause(a.legacy, b.legacy).unwrap_or("sort-tie");
                if state_cause == "sort-tie" || legacy_cause == "sort-tie" {
                    "sort-tie"
                } else if state_cause == "libm" || legacy_cause == "libm" {
                    "libm"
                } else {
                    "fma"
                }
            } else if legacy_diff {
                classify_cause(a.legacy, b.legacy).unwrap_or("sort-tie")
            } else {
                classify_cause(a.state, b.state).unwrap_or("sort-tie")
            };
            return TapeCompare {
                equal: false,
                first_tick: Some(tick),
                lane: Some(lane),
                note: format!(
                    "first bad tick {tick} ({lane} lane) [cause={cause}]: {}",
                    cause_note(tick)
                ),
            };
        }
    }
    TapeCompare {
        equal: true,
        first_tick: None,
        lane: None,
        note: String::new(),
    }
}

/// Corpus checkpoint comparison between two matrix rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorpusCompare {
    /// True when every block digest matches.
    pub equal: bool,
    /// First divergent block index, when any.
    pub first_block: Option<usize>,
    /// First case of that block, when any.
    pub first_case: Option<u32>,
    /// Human line naming the block and its case range.
    pub note: String,
}

/// Compare two corpus checkpoint streams. Block `i` covers cases
/// `i * CHECKPOINT_EVERY .. (i + 1) * CHECKPOINT_EVERY`. Names the cause
/// via [`classify_cause`], mirroring [`compare_checkpoints`], so CI reads
/// the `[cause=...]` tag beside the block range.
#[must_use]
pub fn compare_corpus(base: &[u64], other: &[u64]) -> CorpusCompare {
    if base.len() != other.len() {
        return CorpusCompare {
            equal: false,
            first_block: None,
            first_case: None,
            note: format!(
                "corpus block count differs: {} vs {}",
                base.len(),
                other.len()
            ),
        };
    }
    for (block, (a, b)) in base.iter().zip(other.iter()).enumerate() {
        if a != b {
            let first = u32::try_from(block)
                .unwrap_or(u32::MAX)
                .saturating_mul(CHECKPOINT_EVERY);
            let last = first.saturating_add(CHECKPOINT_EVERY - 1);
            let cause = classify_cause(*a, *b).unwrap_or("sort-tie");
            return CorpusCompare {
                equal: false,
                first_block: Some(block),
                first_case: Some(first),
                note: format!(
                    "first bad corpus block {block} (cases {first}..={last}) [cause={cause}]: \
                     bisect that block for sort ties, libm calls, or fused multiply-add"
                ),
            };
        }
    }
    CorpusCompare {
        equal: true,
        first_block: None,
        first_case: None,
        note: String::new(),
    }
}

/// Entry verdict combining the tape and corpus compares for one row pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatrixVerdict {
    /// True only when both the tape and the corpus compare equal.
    pub equal: bool,
    /// Tape comparison over both `checkpoints`, in tick order.
    pub tape: TapeCompare,
    /// Corpus comparison over both `cut_checkpoints`, in block order.
    pub corpus: CorpusCompare,
    /// Empty when equal, else the tape note when the tape moved, else the corpus note.
    pub note: String,
}

/// Compare two collected rows: tape first, corpus blocks second, pinned
/// digests last, sharing one wording for CI plus `tools/golden-matrix.mjs`.
/// HS2 accepts only when the digests are equal: matching checkpoints with a
/// moved digest still fails, naming the digest instead of a tick.
/// Determinism: same rows give the same verdict.
#[must_use]
pub fn compare_entries(base: &MatrixEntry, other: &MatrixEntry) -> MatrixVerdict {
    let tape = compare_checkpoints(&base.checkpoints, &other.checkpoints);
    let corpus = compare_corpus(&base.cut_checkpoints, &other.cut_checkpoints);
    let digest_note = if base.golden_last != other.golden_last {
        Some(format!(
            "golden digest differs: {:016x} vs {:016x}",
            base.golden_last, other.golden_last
        ))
    } else if base.cut_digest != other.cut_digest {
        Some(format!(
            "corpus digest differs: {:016x} vs {:016x}",
            base.cut_digest, other.cut_digest
        ))
    } else {
        None
    };
    let equal = tape.equal && corpus.equal && digest_note.is_none();
    let note = if equal {
        String::new()
    } else if !tape.equal {
        tape.note.clone()
    } else if !corpus.equal {
        corpus.note.clone()
    } else {
        digest_note.clone().unwrap_or_default()
    };
    MatrixVerdict {
        equal,
        tape,
        corpus,
        note,
    }
}

/// Stable CI row for one collected entry: host plus golden plus corpus
/// digests plus block count, in that order, lower-hex padded to 16.
/// Owner: engine-proof; consumer: CI stores one line per runner.
/// Caller: [`MatrixEntry`]'s [`fmt::Display`] (same file) formats this line,
/// so `format!("{entry}")` stays byte-equal to this helper.
/// Units: hex digests and block count; determinism: pure over the entry
/// with no clock, float, or IO; heap only the returned string.
#[must_use]
pub fn matrix_digest_line(entry: &MatrixEntry) -> String {
    format!(
        "{} golden={:016x} corpus={:016x} blocks={}",
        entry.host,
        entry.golden_last,
        entry.cut_digest,
        entry.cut_checkpoints.len()
    )
}

/// Human CI row for a collected entry: the same stable line as
/// [`matrix_digest_line`], so runners print one line per machine.
/// Determinism: pure over the entry; no clock, float, or IO beyond the
/// formatter.
impl fmt::Display for MatrixEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&matrix_digest_line(self))
    }
}

/// Parsed CI row: the fields of one [`matrix_digest_line`] line.
/// Owner: engine-proof; consumer: a later compare job reads stored rows.
/// Units: hex digests plus block count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedMatrixRow {
    /// Runner label, such as `windows-x86_64`; never empty, never whitespace.
    pub host: String,
    /// Golden digest from `golden=<16hex>`.
    pub golden_last: u64,
    /// Corpus digest from `corpus=<16hex>`.
    pub cut_digest: u64,
    /// Block count from `blocks=<n>`.
    pub blocks: usize,
}

/// Parse one [`matrix_digest_line`] row back into fields.
/// Accepts exactly `host golden=<16hex> corpus=<16hex> blocks=<n>`: host
/// non-empty without whitespace, both hex fields exactly 16 lower-hex
/// digits, blocks decimal `usize`. Returns `None` on any shape, hex, or
/// number mismatch.
/// Determinism: pure over the string; no clock, float, or IO; heap only
/// the returned host string.
#[must_use]
pub fn parse_matrix_digest_line(line: &str) -> Option<ParsedMatrixRow> {
    let mut parts = line.split(' ');
    let host = parts.next()?;
    let golden_part = parts.next()?;
    let corpus_part = parts.next()?;
    let blocks_part = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if host.is_empty() || host.chars().any(char::is_whitespace) {
        return None;
    }
    let golden_hex = golden_part.strip_prefix("golden=")?;
    let corpus_hex = corpus_part.strip_prefix("corpus=")?;
    let blocks_dec = blocks_part.strip_prefix("blocks=")?;
    if golden_hex.len() != 16 || corpus_hex.len() != 16 {
        return None;
    }
    if !golden_hex
        .bytes()
        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return None;
    }
    if !corpus_hex
        .bytes()
        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return None;
    }
    if blocks_dec.is_empty() || !blocks_dec.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let golden_last = u64::from_str_radix(golden_hex, 16).ok()?;
    let cut_digest = u64::from_str_radix(corpus_hex, 16).ok()?;
    let blocks = blocks_dec.parse::<usize>().ok()?;
    Some(ParsedMatrixRow {
        host: host.to_owned(),
        golden_last,
        cut_digest,
        blocks,
    })
}

/// Versioned determinism envelope version; the `--json` envelope leads with it.
/// Caller: `collect_local_entry` (same file) asserts the envelope leads with 2.
pub const SCHEMA_VERSION: u32 = 2;
/// Bound HS2 target set: hosts must match exactly, digests match each other, never a pinned constant.
/// Caller: `compare_matrix_set` (same file) checks this set.
pub const TARGETS: &[&str] = &["windows-x86_64", "linux-x86_64"];
/// Optional third HS2 host when an aarch64 runner exists: the two bound
/// x86-64 rows plus one `*-aarch64` row, digests still equal.
/// Caller: `compare_matrix_set` (same file) allows this third row.
/// Units: host labels; determinism: pure string match, no clock or IO.
#[must_use]
pub fn is_optional_aarch64_host(host: &str) -> bool {
    matches!(host, "linux-aarch64" | "windows-aarch64" | "macos-aarch64")
}
/// Emit this target's digests beside `collect_local_entry`, with a trace round-trip.
/// Caller: `collect_local_entry` (same file) proves file-vs-memory equality per row.
pub fn emit_local_digests(entry: &MatrixEntry) -> (u64, u64, usize) {
    let bytes = crate::encode_trace_inputs(&[]);
    let back = crate::decode_trace_inputs(Config::default(), &bytes).unwrap_or_default();
    assert!(back.is_empty(), "empty trace must round-trip");
    (
        entry.golden_last,
        entry.cut_digest,
        entry.cut_checkpoints.len(),
    )
}
/// Why a matrix set comparison failed: host cover or row divergence.
/// Caller: `compare_matrix_set` (same file) returns this instead of a string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MatrixSetError {
    /// Host labels differ from [`TARGETS`]; carries the labels seen.
    Hosts { got: Vec<String> },
    /// A row pair diverged; carries the verdict note naming tick and lane.
    Diverged { note: String },
}

impl fmt::Display for MatrixSetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hosts { got } => write!(f, "hosts differ: {got:?} vs {TARGETS:?}"),
            Self::Diverged { note } => write!(f, "{note}"),
        }
    }
}

impl std::error::Error for MatrixSetError {}

/// Compare several targets' rows: hosts equal `TARGETS` exactly, or the two
/// bound hosts plus one optional aarch64 host; digests equal the first row.
/// Caller: `collect_local_entry` (same file) exercises this set check per row.
pub fn compare_matrix_set(entries: &[MatrixEntry]) -> Result<(), MatrixSetError> {
    let hosts: Vec<&str> = entries.iter().map(|item| item.host.as_str()).collect();
    let hosts_ok = if hosts == TARGETS {
        true
    } else if hosts.len() == TARGETS.len() + 1
        && hosts[0] == TARGETS[0]
        && hosts[1] == TARGETS[1]
        && is_optional_aarch64_host(hosts[2])
    {
        true
    } else {
        false
    };
    if !hosts_ok {
        return Err(MatrixSetError::Hosts {
            got: hosts.iter().map(|host| (*host).to_owned()).collect(),
        });
    }
    for other in &entries[1..] {
        let verdict = compare_entries(&entries[0], other);
        if !verdict.equal {
            return Err(MatrixSetError::Diverged {
                note: verdict.note.clone(),
            });
        }
    }
    Ok(())
}
/// One `matrix_digest_line` per entry in order; one row per bound target.
/// Caller: `matrix_json_envelope` (same file) posts this table beside the envelope.
pub fn matrix_table(entries: &[MatrixEntry]) -> String {
    entries
        .iter()
        .map(matrix_digest_line)
        .collect::<Vec<_>>()
        .join("\n")
}
/// `--json` envelope with `schema_version` leading; rows carry the per-target table.
/// Caller: `collect_local_entry` (same file) asserts the leading field.
pub fn matrix_json_envelope(entries: &[MatrixEntry]) -> String {
    let rows = entries
        .iter()
        .map(|item| format!("\"{}\"", matrix_digest_line(item)))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{\"schema_version\":{SCHEMA_VERSION},\"targets\":{TARGETS:?},\"rows\":[{rows}]}}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::break_corpus::{run_cuts, GOLDEN_CUT_DIGEST};
    use crate::GOLDEN_600;

    #[test]
    fn matrix_local_row_matches_pinned_digests_x2() {
        let first = collect_local_entry().expect("local row collects");
        let second = collect_local_entry().expect("local row replays");
        assert_eq!(first, second, "two rows on one machine must match");
        assert_eq!(first.golden_last, GOLDEN_600, "golden moved");
        assert_eq!(first.cut_digest, GOLDEN_CUT_DIGEST, "corpus digest moved");
        assert!(!first.host.is_empty(), "host label must name the runner");
        println!(
            "matrix host={} golden={:016x} corpus={:016x} blocks={}",
            first.host,
            first.golden_last,
            first.cut_digest,
            first.cut_checkpoints.len()
        );
        let mut other = first.clone();
        other.host = TARGETS[1].to_owned();
        let pair = [first.clone(), other];
        assert!(
            compare_matrix_set(&pair).is_ok(),
            "bound hosts plus equal digests pass"
        );
        assert!(
            compare_matrix_set(std::slice::from_ref(&first)).is_err(),
            "one row cannot cover two targets"
        );
        let mut moved = pair.clone();
        moved[1].checkpoints[41].legacy ^= 1;
        let error = compare_matrix_set(&moved).expect_err("moved tick must fail");
        let note = error.to_string();
        assert!(
            note.contains("41") && note.contains("legacy"),
            "note {note}"
        );
        let table = matrix_table(&pair);
        assert_eq!(table.lines().count(), TARGETS.len(), "one row per target");
        for line in table.lines() {
            assert!(
                parse_matrix_digest_line(line).is_some(),
                "row {line} parses"
            );
        }
        let json = matrix_json_envelope(&pair);
        assert!(
            json.starts_with("{\"schema_version\":2"),
            "envelope {json} leads with version"
        );
    }

    #[test]
    fn matrix_names_first_divergent_tick_and_lane() {
        let tape = record(Config::default(), &[]).expect("tape records");
        let base = tape.checkpoints.clone();
        let mut moved = base.clone();
        moved[41].legacy ^= 1;
        let compared = compare_checkpoints(&base, &moved);
        assert!(!compared.equal);
        assert_eq!(compared.first_tick, Some(41));
        assert_eq!(compared.lane, Some("legacy"));
        assert!(compared.note.contains("41"), "note {}", compared.note);
        assert!(compared.note.contains("legacy"), "note {}", compared.note);
        let mut state_moved = base.clone();
        state_moved[7].state ^= 1;
        let state_compared = compare_checkpoints(&base, &state_moved);
        assert_eq!(state_compared.first_tick, Some(7));
        assert_eq!(state_compared.lane, Some("state"));
        let same = compare_checkpoints(&base, &base);
        assert!(same.equal);
        assert_eq!(same.first_tick, None);
        let short = &base[..10];
        let count_compared = compare_checkpoints(&base, short);
        assert!(!count_compared.equal);
        assert!(
            count_compared.note.contains("tick count"),
            "{count_compared:?}"
        );
    }

    #[test]
    fn matrix_corpus_blocks_name_the_case_range() {
        let report = run_cuts(CORPUS_SEED, CHECKPOINT_EVERY * 2).expect("prefix runs");
        assert_eq!(report.checkpoints.len(), 2);
        let mut moved = report.checkpoints.clone();
        moved[1] ^= 1;
        let compared = compare_corpus(&report.checkpoints, &moved);
        assert!(!compared.equal);
        assert_eq!(compared.first_block, Some(1));
        assert_eq!(compared.first_case, Some(CHECKPOINT_EVERY));
        assert!(
            compared.note.contains("500..=999"),
            "note {}",
            compared.note
        );
        let same = compare_corpus(&report.checkpoints, &report.checkpoints);
        assert!(same.equal);
    }

    #[test]
    fn matrix_corpus_block_names_cause_like_tape() {
        // HS2 slice: the corpus block note names its cause via
        // `classify_cause`, mirroring `compare_checkpoints`, so CI reads
        // the `[cause=...]` tag beside the block range.
        let report = run_cuts(CORPUS_SEED, CHECKPOINT_EVERY * 2).expect("prefix runs");
        let base = report.checkpoints.clone();
        let mut fma_moved = base.clone();
        fma_moved[1] ^= 1;
        let fma_compared = compare_corpus(&base, &fma_moved);
        assert!(!fma_compared.equal);
        assert!(
            fma_compared.note.contains("[cause=fma]"),
            "fma block note {}",
            fma_compared.note
        );
        let mut libm_moved = base.clone();
        libm_moved[1] ^= 0x80;
        let libm_compared = compare_corpus(&base, &libm_moved);
        assert!(
            libm_compared.note.contains("[cause=libm]"),
            "libm block note {}",
            libm_compared.note
        );
        assert_ne!(
            fma_compared.note, libm_compared.note,
            "same block with different magnitudes must name different causes"
        );
        assert_eq!(
            compare_corpus(&base, &fma_moved).note,
            fma_compared.note,
            "same inputs give same note twice"
        );
    }

    #[test]
    fn matrix_entry_verdict_orders_tape_before_corpus() {
        let tape = record(Config::default(), &[]).expect("tape records");
        let report = run_cuts(CORPUS_SEED, CHECKPOINT_EVERY * 2).expect("prefix runs");
        let base = MatrixEntry {
            host: host_label(),
            golden_last: GOLDEN_600,
            checkpoints: tape.checkpoints.clone(),
            cut_digest: report.digest,
            cut_checkpoints: report.checkpoints.clone(),
        };
        let same = compare_entries(&base, &base);
        assert!(same.equal, "equal rows stay equal");
        assert!(same.note.is_empty(), "note {:?}", same.note);
        let mut both_moved = base.clone();
        both_moved.checkpoints[41].legacy ^= 1;
        both_moved.cut_checkpoints[1] ^= 1;
        let both = compare_entries(&base, &both_moved);
        assert!(!both.equal, "moved tape plus corpus must differ");
        assert_eq!(both.tape.first_tick, Some(41));
        assert!(both.note.contains("41"), "note {}", both.note);
        assert_eq!(both.note, both.tape.note, "tape names first");
        let mut corpus_moved = base.clone();
        corpus_moved.cut_checkpoints[1] ^= 1;
        let corpus_only = compare_entries(&base, &corpus_moved);
        assert!(!corpus_only.equal, "moved corpus must differ");
        assert!(corpus_only.tape.equal, "tape still matches");
        assert_eq!(corpus_only.corpus.first_block, Some(1));
        assert!(
            corpus_only.note.contains("first bad corpus block 1"),
            "note {}",
            corpus_only.note
        );
        assert_eq!(corpus_only.note, corpus_only.corpus.note);
    }

    #[test]
    fn matrix_verdict_requires_equal_digests() {
        let tape = record(Config::default(), &[]).expect("tape records");
        let report = run_cuts(CORPUS_SEED, CHECKPOINT_EVERY * 2).expect("prefix runs");
        let base = MatrixEntry {
            host: host_label(),
            golden_last: GOLDEN_600,
            checkpoints: tape.checkpoints.clone(),
            cut_digest: report.digest,
            cut_checkpoints: report.checkpoints.clone(),
        };
        // Same checkpoints on both sides, but the corpus digest moved: the
        // matrix must fail and name the digest, not the ticks.
        let mut moved = base.clone();
        moved.cut_digest ^= 1;
        let verdict = compare_entries(&base, &moved);
        assert!(!verdict.equal, "a moved corpus digest must fail the matrix");
        assert!(verdict.tape.equal && verdict.corpus.equal);
        assert!(
            verdict.note.contains("corpus digest differs"),
            "note {}",
            verdict.note
        );
        let mut golden_moved = base.clone();
        golden_moved.golden_last ^= 1;
        let golden_verdict = compare_entries(&base, &golden_moved);
        assert!(!golden_verdict.equal, "a moved golden must fail the matrix");
        assert!(
            golden_verdict.note.contains("golden digest differs"),
            "note {}",
            golden_verdict.note
        );
    }

    #[test]
    fn matrix_cause_note_separates_setup_from_tick_math() {
        assert!(cause_note(0).contains("start state"), "{}", cause_note(0));
        assert!(cause_note(9).contains("sort ties"), "{}", cause_note(9));
    }

    #[test]
    fn matrix_cause_note_names_libm_and_fma_lanes() {
        assert_eq!(
            classify_cause(0x1000, 0x1000),
            None,
            "equal digests have no cause"
        );
        assert_eq!(
            classify_cause(0x1000, 0x1001),
            Some("fma"),
            "single-bit drift is fused multiply-add"
        );
        assert_eq!(
            classify_cause(0x1000, 0x1080),
            Some("libm"),
            "one-byte drift is a libm call"
        );
        assert_eq!(
            classify_cause(0x1000, 0xFFFF_0000),
            Some("sort-tie"),
            "large drift is a sort tie"
        );
        let tape = record(Config::default(), &[]).expect("tape records");
        let base = tape.checkpoints.clone();
        let mut fma_moved = base.clone();
        fma_moved[9].legacy ^= 1;
        let fma_compared = compare_checkpoints(&base, &fma_moved);
        let mut libm_moved = base.clone();
        libm_moved[9].legacy ^= 0x80;
        let libm_compared = compare_checkpoints(&base, &libm_moved);
        assert!(
            fma_compared.note.contains("[cause=fma]"),
            "fma lane note {}",
            fma_compared.note
        );
        assert!(
            libm_compared.note.contains("[cause=libm]"),
            "libm lane note {}",
            libm_compared.note
        );
        assert_ne!(
            fma_compared.note, libm_compared.note,
            "same tick and lane with different magnitudes must name different causes"
        );
    }

    #[test]
    fn matrix_digest_line_is_stable_and_names_host() {
        let entry = collect_local_entry().expect("local row collects");
        let first = matrix_digest_line(&entry);
        let second = matrix_digest_line(&entry);
        assert_eq!(first, second, "digest line must be stable x2");
        assert!(
            first.contains(&entry.host),
            "line {first} must name host {}",
            entry.host
        );
        assert!(
            first.contains("golden=d57b5c8ae3096614"),
            "line {first} must pin the golden"
        );
        assert!(
            first.contains(&format!("corpus={GOLDEN_CUT_DIGEST:016x}")),
            "line {first} must pin the corpus"
        );
        assert!(
            first.contains("blocks=20"),
            "line {first} must pin the block count"
        );
        let mut moved = entry.clone();
        moved.cut_digest ^= 1;
        assert_ne!(
            matrix_digest_line(&moved),
            first,
            "a moved corpus digest must flip the line"
        );
        let verdict = compare_entries(&entry, &moved);
        assert!(!verdict.equal, "a moved corpus digest must fail the matrix");
        assert!(
            verdict.note.contains("corpus digest differs"),
            "note {}",
            verdict.note
        );
    }

    #[test]
    fn matrix_display_matches_digest_line() {
        let entry = collect_local_entry().expect("local row collects");
        assert_eq!(
            format!("{entry}"),
            matrix_digest_line(&entry),
            "Display must stay byte-equal to the digest line"
        );
        let mut moved = entry.clone();
        moved.cut_digest ^= 1;
        assert_ne!(
            format!("{moved}"),
            matrix_digest_line(&entry),
            "a moved corpus digest must flip the Display row"
        );
        assert_eq!(
            format!("{moved}"),
            matrix_digest_line(&moved),
            "Display must track the moved entry"
        );
    }

    #[test]
    fn matrix_set_error_is_typed() {
        let first = collect_local_entry().expect("local row collects");
        let mut other = first.clone();
        other.host = TARGETS[1].to_owned();
        let pair = [first.clone(), other];
        assert!(
            compare_matrix_set(&pair).is_ok(),
            "bound hosts plus equal digests pass"
        );
        let error = compare_matrix_set(std::slice::from_ref(&first))
            .expect_err("one row cannot cover two targets");
        assert!(
            matches!(error, MatrixSetError::Hosts { .. }),
            "short set names hosts, got {error:?}"
        );
        let mut moved = pair.clone();
        moved[1].checkpoints[41].legacy ^= 1;
        let error = compare_matrix_set(&moved).expect_err("moved tick must fail");
        assert!(
            matches!(error, MatrixSetError::Diverged { .. }),
            "moved tick names divergence, got {error:?}"
        );
        assert!(
            error.to_string().contains("41"),
            "note names the tick: {error}"
        );
    }

    #[test]
    fn matrix_digest_line_round_trips_through_parser() {
        let entry = collect_local_entry().expect("local row collects");
        let line = matrix_digest_line(&entry);
        let parsed = parse_matrix_digest_line(&line).expect("digest line parses");
        assert_eq!(parsed.host, entry.host, "host {line}");
        assert_eq!(parsed.golden_last, entry.golden_last, "golden {line}");
        assert_eq!(parsed.cut_digest, entry.cut_digest, "corpus {line}");
        assert_eq!(parsed.blocks, entry.cut_checkpoints.len(), "blocks {line}");
        let mut moved = entry.clone();
        moved.cut_digest ^= 1;
        let moved_line = matrix_digest_line(&moved);
        let moved_parsed = parse_matrix_digest_line(&moved_line).expect("moved line parses");
        assert_eq!(
            moved_parsed.cut_digest, moved.cut_digest,
            "moved {moved_line}"
        );
        assert!(parse_matrix_digest_line("").is_none(), "empty must fail");
        assert!(
            parse_matrix_digest_line(&format!("{} golden={:016x}", entry.host, entry.golden_last))
                .is_none(),
            "missing fields must fail"
        );
        assert!(
            parse_matrix_digest_line(&format!(
                "{} golden=abc corpus={:016x} blocks={}",
                entry.host,
                entry.cut_digest,
                entry.cut_checkpoints.len()
            ))
            .is_none(),
            "short hex must fail"
        );
        assert!(
            parse_matrix_digest_line(&format!(
                "{} golden={:016x} corpus={:016x} blocks=xx",
                entry.host, entry.golden_last, entry.cut_digest
            ))
            .is_none(),
            "bad blocks must fail"
        );
    }

    #[test]
    fn matrix_set_accepts_optional_aarch64_third_row() {
        let first = collect_local_entry().expect("local row collects");
        let mut second = first.clone();
        second.host = TARGETS[1].to_owned();
        let mut third = first.clone();
        third.host = "linux-aarch64".to_owned();
        let triple = [first.clone(), second.clone(), third.clone()];
        assert!(
            compare_matrix_set(&triple).is_ok(),
            "optional aarch64 third row passes when digests match"
        );
        let mut moved = triple.clone();
        moved[2].checkpoints[41].legacy ^= 1;
        let error = compare_matrix_set(&moved).expect_err("moved aarch64 tick must fail");
        assert!(
            error.to_string().contains("41"),
            "diverged aarch64 names the tick: {error}"
        );
        assert!(
            matches!(error, MatrixSetError::Diverged { .. }),
            "moved tick names divergence, got {error:?}"
        );
        let mut bad = triple.clone();
        bad[2].host = "freebsd-x86_64".to_owned();
        let error = compare_matrix_set(&bad).expect_err("unknown third host must fail");
        assert!(
            matches!(error, MatrixSetError::Hosts { .. }),
            "unknown host names hosts, got {error:?}"
        );
        assert!(is_optional_aarch64_host("linux-aarch64"));
        assert!(is_optional_aarch64_host("windows-aarch64"));
        assert!(!is_optional_aarch64_host("linux-x86_64"));
        assert!(!is_optional_aarch64_host("freebsd-x86_64"));
        assert!(!is_optional_aarch64_host(""));
    }
}
