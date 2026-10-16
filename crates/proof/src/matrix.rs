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
    let golden_last = tape
        .checkpoints
        .last()
        .map(|checkpoint| checkpoint.legacy)
        .ok_or(MatrixError::Empty)?;
    let report =
        crate::break_corpus::run_cuts(CORPUS_SEED, CUT_CASES).map_err(MatrixError::Corpus)?;
    Ok(MatrixEntry {
        host: host_label(),
        golden_last,
        checkpoints: tape.checkpoints,
        cut_digest: report.digest,
        cut_checkpoints: report.checkpoints,
    })
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
/// rows; the first mismatch names its tick and lane.
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
            return TapeCompare {
                equal: false,
                first_tick: Some(tick),
                lane: Some(lane),
                note: format!("first bad tick {tick} ({lane} lane): {}", cause_note(tick)),
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
/// `i * CHECKPOINT_EVERY .. (i + 1) * CHECKPOINT_EVERY`.
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
            return CorpusCompare {
                equal: false,
                first_block: Some(block),
                first_case: Some(first),
                note: format!(
                    "first bad corpus block {block} (cases {first}..={last}): \
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
}
