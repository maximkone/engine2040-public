//! SPF8, SPF9 and SPX7 at the door: role triage, verdict allow rows and
//! z-fight notes on `scene.spatial`, plus the node roles `scene.brief` folds by.
//!
//! Consumer: `scene_spatial` and `scene_brief`. The judging is `engine-spatial`'s
//! (`triage`, `verdict`, `triage::z_fights_all`); this module maps the door's
//! components to roles and the door's labels to actor ids, and prints the rows.
//! Units: whole cm in the notes, document revisions in the allow rows.
//! Determinism: same document, report and rows give the same lines in the same
//! order. With none of the new arguments the rows are the report's, byte for byte.

use std::collections::BTreeMap;

use engine_editor::commands::Document;
use engine_scene::component::ComponentKind;
use engine_scene::ids::ObjectId;
use engine_spatial::triage::{triage, z_fights_all, Role};
use engine_spatial::verdict::{judge, Allow, Basis, MAX_ALLOW};
use engine_spatial::{Actor, Finding, Report, ZFIGHT_EPS_CM};

use super::{parse_object_ref, McpToolError};

/// Widest z-fight tolerance a call may ask for, in cm.
const MAX_EPSILON_CM: i32 = 50;

/// Role of each staged node (actor id = table index) from its components:
/// a mesh draws, a collider-only node is a collider, a light or spawner is an
/// effect marker. Nodes with none of these have no entry.
pub(crate) fn staged_roles(doc: &Document, ids: &[ObjectId]) -> BTreeMap<u32, Role> {
    let mut roles = BTreeMap::new();
    for (index, id) in ids.iter().enumerate() {
        let Some(kinds) = doc.components.get(&id.to_string()) else {
            continue;
        };
        let has = |kind: ComponentKind| kinds.contains_key(&kind);
        let role = if has(ComponentKind::Mesh) {
            Role::Mesh
        } else if has(ComponentKind::Collider) {
            Role::Collider
        } else if has(ComponentKind::Light) || has(ComponentKind::Spawner) {
            Role::Marker
        } else {
            continue;
        };
        if let Ok(actor) = u32::try_from(index) {
            roles.insert(actor, role);
        }
    }
    roles
}

fn actor_of(field: &str, text: &str, ids: &[ObjectId]) -> Result<u32, McpToolError> {
    let id = parse_object_ref(field, text)?;
    ids.iter()
        .position(|staged| *staged == id)
        .and_then(|index| u32::try_from(index).ok())
        .ok_or_else(|| {
            McpToolError::new(
                "unknown_actor",
                format!("{field} `{text}` names no staged object; re-read scene.brief"),
            )
        })
}

/// Field names of an allow row, in order; the reason is the rest of the line.
const ALLOW_FIELDS: [&str; 6] = ["check", "a", "b", "verdict", "rev", "expiry"];

/// One row `check a b|- verdict rev expiry reason` (`a`, `b` bare UUIDs; `rev`
/// and `expiry` 0 = none; the reason runs to the end of the line).
fn parse_row(i: usize, row: &str, ids: &[ObjectId]) -> Result<Allow, McpToolError> {
    let refuse = |field: &str, why: &str| {
        McpToolError::new(
            "bad_allow",
            format!(
                "allow[{i}].{field}: {why}; row grammar: check a b|- verdict rev expiry reason"
            ),
        )
    };
    let mut rest = row.trim();
    let mut fields = [""; 6];
    for (slot, name) in fields.iter_mut().zip(ALLOW_FIELDS) {
        let (head, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        if head.is_empty() {
            return Err(refuse(name, "missing"));
        }
        *slot = head;
        rest = tail.trim_start();
    }
    let [check, a, b, verdict, rev, expiry] = fields;
    let number = |name: &str, text: &str| {
        text.parse::<u64>()
            .map_err(|_| refuse(name, "not a number"))
    };
    let a_id = actor_of(&format!("allow[{i}].a"), a, ids)?;
    let b_id = (b != "-")
        .then(|| actor_of(&format!("allow[{i}].b"), b, ids))
        .transpose()?;
    Allow::new(
        check,
        a_id,
        b_id,
        verdict,
        rest,
        number("rev", rev)?,
        number("expiry", expiry)?,
    )
    .map_err(|e| refuse(e.field, &e.message))
}

fn parse_allow(rows: &[String], ids: &[ObjectId]) -> Result<Vec<Allow>, McpToolError> {
    if rows.len() > MAX_ALLOW {
        return Err(McpToolError::new(
            "too_many_allow",
            format!("allow holds {} rows; at most {MAX_ALLOW}", rows.len()),
        ));
    }
    rows.iter()
        .enumerate()
        .map(|(i, row)| parse_row(i, row, ids))
        .collect()
}

/// Z-fight tolerance in cm from the call, default 1; `bad_epsilon` outside 0..=50.
pub(crate) fn check_epsilon(epsilon_cm: Option<i32>) -> Result<i32, McpToolError> {
    match epsilon_cm {
        None => Ok(ZFIGHT_EPS_CM),
        Some(cm) if (0..=MAX_EPSILON_CM).contains(&cm) => Ok(cm),
        Some(cm) => Err(McpToolError::new(
            "bad_epsilon",
            format!("epsilon_cm {cm} is outside 0..={MAX_EPSILON_CM} (default {ZFIGHT_EPS_CM})"),
        )),
    }
}

/// The findings a `scene.spatial` answer shows, after triage, z-fight notes
/// and verdicts.
pub(crate) struct View {
    /// Findings that still count, severities as triaged.
    pub(crate) kept: Vec<Finding>,
    notes: BTreeMap<(&'static str, u32, u32), String>,
    /// Suppressed finding lines, each with verdict, basis and reason, then the
    /// learner cells that relaxed.
    pub(crate) suppressed: Vec<String>,
}

impl View {
    /// One finding line: the brief row, then its note (intent or z-fight fix).
    pub(crate) fn line(&self, f: &Finding) -> String {
        let note = self
            .notes
            .get(&(f.check, f.a, f.b))
            .map_or("", String::as_str);
        format!("{}{note}\n", f.row())
    }

    /// `red` when a kept finding is red, `clean` with none left to worry about
    /// (info rows do not count), else `yellow`.
    pub(crate) fn verdict(&self) -> &'static str {
        if self.kept.iter().any(|f| f.severity == "red") {
            "red"
        } else if self.kept.iter().any(|f| f.severity == "yellow") {
            "yellow"
        } else {
            "clean"
        }
    }
}

/// What the call asked of the report.
pub(crate) struct Asks<'a> {
    pub(crate) triage: bool,
    pub(crate) allow: &'a [String],
    /// Checks that ran, for the z-fight notes.
    pub(crate) run: &'a [&'a str],
    pub(crate) epsilon_cm: i32,
    /// Live document revision.
    pub(crate) revision: u64,
}

/// Build the view of `report` for `asks`. Absent every ask the kept rows are
/// the report's findings, in its order, with no notes.
pub(crate) fn view(
    actors: &[Actor],
    ids: &[ObjectId],
    doc: &Document,
    report: &Report,
    asks: &Asks<'_>,
) -> Result<View, McpToolError> {
    let allow = parse_allow(asks.allow, ids)?;
    let mut notes: BTreeMap<(&'static str, u32, u32), String> = BTreeMap::new();
    let findings: Vec<Finding> = if asks.triage {
        let roles = staged_roles(doc, ids);
        triage(actors, &roles, &report.findings)
            .into_iter()
            .map(|t| {
                let key = (t.finding.check, t.finding.a, t.finding.b);
                notes.insert(key, format!(" {}", t.intent.word()));
                Finding {
                    severity: t.severity,
                    ..t.finding
                }
            })
            .collect()
    } else {
        report.findings.clone()
    };
    if asks.run.contains(&"zfight") {
        for z in z_fights_all(actors, asks.epsilon_cm) {
            let note = notes.entry(("zfight", z.a, z.b)).or_default();
            note.push_str(&format!(" | {}", z.note()));
        }
    }
    let judged = judge(&findings, actors, &allow, asks.revision);
    let set_aside = judged.suppressed.iter().map(|s| {
        let basis = match s.basis {
            Basis::Allowed => "allow".to_owned(),
            Basis::Learned(level) => format!("learned level {level}"),
        };
        format!(
            "{} suppressed {} ({basis}): {}",
            s.finding.row(),
            s.verdict.word(),
            s.reason
        )
    });
    let learned = judged.cells.iter().filter(|c| c.level > 0).map(|c| {
        format!(
            "learned {} x {}: level {} ({} verdicts, last {})",
            c.check,
            c.family,
            c.level,
            c.count,
            c.last.word()
        )
    });
    let suppressed = set_aside.chain(learned).collect();
    Ok(View {
        kept: judged.kept,
        notes,
        suppressed,
    })
}
