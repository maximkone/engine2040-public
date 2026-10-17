//! Offline open-vocabulary label lift: a labeled capture row becomes candidate
//! tags for one scene node, plus a gold precision/recall pair. No model, no
//! weights, no float; the lexicon is a hand-editable table.
//!
//! Owner: spatial crate. Consumer: the door and brief path call
//! [`lift_captures`] / [`lift_label`] (SP46 slice 2). Units: meters and
//! tracked units never cross this module. Determinism: the same labeled rows
//! in give the same [`LiftReport`] bytes, the tick is never read and no digest
//! of it is touched. Tags are proposals for a human gold gate; the lift writes
//! no node, so [`LiftReport::auto_applied`] is 0 by construction. Errors:
//! typed [`LiftError`]; a reviewed node the report does not hold is refused.
//! Budget: O(rows * vocab) integer compares, offline only, never the tick.

use std::fmt;

/// The fixed open-vocabulary lexicon: lowercase ASCII word -> tag name. A
/// human edits this table; it is the whole "open vocabulary" today. Unknown
/// words yield no tag and never a guess.
pub const VOCAB: &[(&str, &str)] = &[
    ("trestle", "table"),
    ("desk", "table"),
    ("bench", "table"),
    ("buttress", "cover"),
    ("barricade", "cover"),
    ("parapet", "cover"),
    ("banner", "cloth"),
    ("pennant", "cloth"),
    ("drape", "cloth"),
    ("tapestry", "cloth"),
    ("torch", "light"),
    ("lantern", "light"),
    ("brazier", "light"),
    ("crate", "container"),
    ("barrel", "container"),
    ("chest", "container"),
    ("amphora", "container"),
    ("column", "pillar"),
    ("pillar", "pillar"),
    ("plinth", "pillar"),
    ("arch", "gate"),
    ("portcullis", "gate"),
    ("altar", "shrine"),
    ("statue", "monument"),
];

/// One labeled capture row: a scene node id and the free-text label a capture
/// gave it. Plain data; no meters and no units cross here.
#[derive(Debug, Clone)]
pub struct LabeledCapture<'a> {
    /// Scene node the capture points at.
    pub node: u32,
    /// Free-text label for that node.
    pub label: &'a str,
}

/// One human gold row: the tags a reviewer says the node truly has.
#[derive(Debug, Clone)]
pub struct GoldRow<'a> {
    /// Scene node this gold covers.
    pub node: u32,
    /// The label the gold was judged against.
    pub label: &'a str,
    /// Tags the reviewer accepted as correct for the node.
    pub gold: &'a [&'a str],
}

/// One lifted node: the proposals plus how the gold counted against them.
#[derive(Debug, Clone)]
pub struct LiftRow {
    /// Scene node this row lifts.
    pub node: u32,
    /// Proposed tags, [`VOCAB`] order, deduplicated.
    pub proposed: Vec<&'static str>,
    /// Gold tags that appear in `proposed`.
    pub gold_hit: u32,
    /// Gold tags that do not appear in `proposed`.
    pub gold_missed: u32,
}

/// Whole-lift report. `auto_applied` is always 0: the lift writes no node.
#[derive(Debug, Clone)]
pub struct LiftReport {
    /// One row per capture, in capture input order.
    pub rows: Vec<LiftRow>,
    /// Total proposals across all rows.
    pub proposed_total: u32,
    /// Total gold tags across rows that have a gold row.
    pub gold_total: u32,
    /// Total gold tags found in the proposals.
    pub hit_total: u32,
    /// Always 0.
    pub auto_applied: u32,
    /// `hit_total * 1000 / proposed_total`, 0 when the divisor is 0.
    pub precision_permille: u32,
    /// `hit_total * 1000 / gold_total`, 0 when the divisor is 0.
    pub recall_permille: u32,
}

/// Legal refusal from the lift. Every message names the legal values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiftError {
    /// A reviewed node the report does not hold. The legal values are the node
    /// ids in the report's rows.
    UnknownNode { node: u32 },
}

impl fmt::Display for LiftError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LiftError::UnknownNode { node } => write!(
                f,
                "unknown node {node}: legal values are the node ids in the report's rows"
            ),
        }
    }
}

impl std::error::Error for LiftError {}

/// Tags proposed for one label: lowercase ASCII, split on non-alphanumerics,
/// one tag per matched lexicon word, [`VOCAB`] order, deduplicated.
pub fn lift_label(label: &str) -> Vec<&'static str> {
    let lower = label.to_ascii_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let mut out: Vec<&'static str> = Vec::new();
    for &(word, tag) in VOCAB {
        if !out.contains(&tag) && tokens.contains(&word) {
            out.push(tag);
        }
    }
    out
}

/// Lift every capture in input order. Gold is looked up by node; a node with no
/// gold row reads as no gold. The report writes no node.
pub fn lift_captures(captures: &[LabeledCapture<'_>], gold: &[GoldRow<'_>]) -> LiftReport {
    let mut rows = Vec::with_capacity(captures.len());
    let mut proposed_total = 0u32;
    let mut gold_total = 0u32;
    let mut hit_total = 0u32;
    for cap in captures {
        let proposed = lift_label(cap.label);
        let gold_tags: &[&str] = match gold.iter().find(|g| g.node == cap.node) {
            Some(g) => g.gold,
            None => &[],
        };
        let mut hit = 0u32;
        for tag in gold_tags {
            if proposed.iter().any(|p| p == tag) {
                hit += 1;
            }
        }
        let gold_len = as_u32(gold_tags.len());
        proposed_total += as_u32(proposed.len());
        gold_total += gold_len;
        hit_total += hit;
        rows.push(LiftRow {
            node: cap.node,
            proposed,
            gold_hit: hit,
            gold_missed: gold_len - hit,
        });
    }
    LiftReport {
        rows,
        proposed_total,
        gold_total,
        hit_total,
        auto_applied: 0,
        precision_permille: permille(hit_total, proposed_total),
        recall_permille: permille(hit_total, gold_total),
    }
}

/// Proposals for the nodes a human reviewed, in report order, deduplicated by
/// node. Every reviewed node must be a report row; an unknown node fails
/// loudly. Proposals are copied out, never applied to a node.
pub fn applies_for(
    report: &LiftReport,
    reviewed: &[u32],
) -> Result<Vec<(u32, Vec<&'static str>)>, LiftError> {
    for &node in reviewed {
        if !report.rows.iter().any(|r| r.node == node) {
            return Err(LiftError::UnknownNode { node });
        }
    }
    let out = report
        .rows
        .iter()
        .filter(|r| reviewed.contains(&r.node))
        .map(|r| (r.node, r.proposed.clone()))
        .collect();
    Ok(out)
}

/// Canonical bytes for the whole report: the x2 gate's one owner. Lengths and
/// counts are little-endian; no float reaches these bytes.
pub fn report_bytes(report: &LiftReport) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&as_u32(report.rows.len()).to_le_bytes());
    for row in &report.rows {
        out.extend_from_slice(&row.node.to_le_bytes());
        out.extend_from_slice(&row.gold_hit.to_le_bytes());
        out.extend_from_slice(&row.gold_missed.to_le_bytes());
        out.extend_from_slice(&as_u32(row.proposed.len()).to_le_bytes());
        for tag in &row.proposed {
            out.extend_from_slice(&as_u32(tag.len()).to_le_bytes());
            out.extend_from_slice(tag.as_bytes());
        }
    }
    for n in [
        report.proposed_total,
        report.gold_total,
        report.hit_total,
        report.auto_applied,
        report.precision_permille,
        report.recall_permille,
    ] {
        out.extend_from_slice(&n.to_le_bytes());
    }
    out
}

/// Lossless usize -> u32, saturating only past the u32 range (never here).
fn as_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Integer permille `hit / total`, 0 when the divisor is 0.
fn permille(hit: u32, total: u32) -> u32 {
    if total == 0 {
        return 0;
    }
    u32::try_from(u64::from(hit) * 1000 / u64::from(total)).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: [(&str, &[&str]); 30] = [
        ("trestle", &["table"]),
        ("wooden desk", &["table"]),
        ("stone bench", &["table"]),
        ("buttress wall", &["cover"]),
        ("a barricade", &["cover"]),
        ("parapet", &["cover"]),
        ("banner", &["cloth"]),
        ("torn pennant", &["cloth"]),
        ("drape", &["cloth"]),
        ("tapestry", &["cloth"]),
        ("torch", &["light"]),
        ("lantern", &["light"]),
        ("brazier", &["light"]),
        ("crate", &["container"]),
        ("barrel", &["container"]),
        ("chest", &["container"]),
        ("amphora", &["container"]),
        ("column", &["pillar"]),
        ("pillar", &["pillar"]),
        ("plinth", &["pillar"]),
        ("arch", &["gate"]),
        ("portcullis", &["gate"]),
        ("altar", &["shrine"]),
        ("statue", &["monument"]),
        ("trestle with a banner", &["table", "cloth"]),
        ("barrel behind the parapet", &["container", "cover"]),
        ("torch on the pillar", &["light", "pillar"]),
        ("crate by the portcullis", &["container", "gate"]),
        ("statue near the altar", &["monument", "shrine"]),
        ("drape over the bench", &["cloth", "table"]),
    ];

    #[test]
    fn tag_lift_thirty_labeled_rows_hit_gold_and_never_auto_apply() {
        let captures: Vec<LabeledCapture<'_>> = FIXTURE
            .iter()
            .enumerate()
            .map(|(i, (label, _))| LabeledCapture {
                node: i as u32,
                label,
            })
            .collect();
        let gold: Vec<GoldRow<'_>> = FIXTURE
            .iter()
            .enumerate()
            .map(|(i, (label, tags))| GoldRow {
                node: i as u32,
                label,
                gold: tags,
            })
            .collect();
        let report = lift_captures(&captures, &gold);
        assert_eq!(report.rows.len(), 30);
        assert_eq!(report.auto_applied, 0);
        for (i, (_, tags)) in FIXTURE.iter().enumerate() {
            for tag in tags.iter() {
                assert!(
                    report.rows[i].proposed.contains(tag),
                    "gold {tag} missing from row {i} proposals"
                );
            }
            assert_eq!(report.rows[i].gold_missed, 0);
        }
        assert_eq!(report.precision_permille, 1000);
        assert_eq!(report.recall_permille, 1000);
        assert!(applies_for(&report, &[]).unwrap().is_empty());
        println!(
            "gold lift permille: precision={} recall={}",
            report.precision_permille, report.recall_permille
        );
    }

    #[test]
    fn tag_lift_unknown_word_proposes_nothing() {
        assert!(lift_label("zzz qwerty blorp").is_empty());
        let caps = [LabeledCapture {
            node: 7,
            label: "zzz qwerty",
        }];
        let gold = [GoldRow {
            node: 7,
            label: "zzz qwerty",
            gold: &["table"],
        }];
        let report = lift_captures(&caps, &gold);
        assert!(report.rows[0].proposed.is_empty());
        assert_eq!(report.rows[0].gold_missed, 1);
        assert_eq!(report.proposed_total, 0);
        assert_eq!(report.precision_permille, 0);
        assert_eq!(report.auto_applied, 0);
    }

    #[test]
    fn tag_lift_applies_only_reviewed_nodes() {
        let caps = [
            LabeledCapture {
                node: 1,
                label: "trestle",
            },
            LabeledCapture {
                node: 2,
                label: "banner",
            },
        ];
        let report = lift_captures(&caps, &[]);
        let got = applies_for(&report, &[2, 1]).unwrap();
        assert_eq!(got, vec![(1, vec!["table"]), (2, vec!["cloth"])]);
        let err = applies_for(&report, &[9]).unwrap_err();
        assert_eq!(err, LiftError::UnknownNode { node: 9 });
        assert!(format!("{err}").contains("legal"));
    }

    #[test]
    fn tag_lift_report_bytes_replay_twice() {
        let caps = [
            LabeledCapture {
                node: 0,
                label: "trestle with barrel",
            },
            LabeledCapture {
                node: 5,
                label: "statue",
            },
        ];
        let gold = [GoldRow {
            node: 0,
            label: "trestle with barrel",
            gold: &["table", "container"],
        }];
        let a = lift_captures(&caps, &gold);
        let b = lift_captures(&caps, &gold);
        assert_eq!(report_bytes(&a), report_bytes(&b));
        assert_eq!(report_bytes(&a), report_bytes(&a));
    }
}
