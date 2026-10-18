//! SPX3: the spatial findings a write changed, returned on the write.
//!
//! Consumer: `tx.apply` and `scene.place` answers, so a weak model sees the
//! consequence of its edit (a crate sunk into a wall, a lamp left floating)
//! without another read. The staged check is `engine_spatial::staged::
//! check_staged_ids`: the full offline catalog on the snapshot before and after
//! the write, filtered to the objects the write touched, so a pre-existing
//! overlap elsewhere never shows and a clean move reads empty.
//!
//! Ids: snapshot actors are numbered by position in the sorted id list, which
//! shifts when a write inserts or removes a node, so both snapshots are
//! renumbered over the union of their ids before the check.
//! Units: centimeters in the lines. Determinism: same document and batch give
//! the same lines in the same order (red before yellow, then check and id
//! order). Budget: two snapshots plus two catalog runs; `delta=false` skips it.

use std::collections::BTreeMap;

use engine_editor::commands::{Batch, Document, Op};
use engine_scene::ids::ObjectId;
use engine_spatial::staged::{check_staged_ids, check_staged_with, DeltaFinding};
use engine_spatial::Actor;

use super::{object_label, partition, snapshot, Snapshot};
use crate::mcp::SpatialDeltaDto;

/// Most findings, new or cleared, a delta lists; the rest read as a total.
pub const DELTA_MAX_FINDINGS: usize = 5;
/// Most touched ids a delta lists; the rest read as a total.
pub const DELTA_MAX_IDS: usize = 16;

/// SPX3: the scene as it was, taken only when the call wants a delta
/// (`delta: false` skips both snapshots and the catalog runs).
pub(crate) fn delta_before(doc: &Document, flag: Option<bool>) -> Option<Snapshot> {
    (flag != Some(false)).then(|| snapshot(doc))
}

/// The delta of the batch that just landed on `doc`, when `before` was taken.
pub(crate) fn delta_after(
    before: Option<Snapshot>,
    doc: &Document,
    batch: &Batch,
) -> Option<SpatialDeltaDto> {
    before.map(|before| spatial_delta(&before, &snapshot(doc), &op_ids(batch)))
}

/// Every node a batch names, in op order, deduplicated.
fn op_ids(batch: &Batch) -> Vec<ObjectId> {
    let mut out: Vec<ObjectId> = Vec::new();
    for op in &batch.ops {
        let id = match op {
            Op::Insert { id, .. }
            | Op::Remove { id }
            | Op::SetLocal { id, .. }
            | Op::Reparent { id, .. }
            | Op::SetMaterial { id, .. }
            | Op::SetComponent { id, .. }
            | Op::RemoveComponent { id, .. } => *id,
        };
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

/// Snapshot actors renumbered by position in `keys` (the sorted union of ids).
fn renumbered(snap: &Snapshot, stable: &BTreeMap<&str, u32>) -> Vec<Actor> {
    snap.ids
        .iter()
        .zip(&snap.actors)
        .filter_map(|(id, actor)| {
            let number = *stable.get(id.to_string().as_str())?;
            Some(Actor {
                id: number,
                ..actor.clone()
            })
        })
        .collect()
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// One finding as a single sentence that names its objects by label and ends
/// with the one-move fix when the check has one.
fn finding_text(entry: &DeltaFinding, label: &dyn Fn(u32) -> String) -> String {
    let f = &entry.finding;
    let named = |id: u32| {
        if id == u32::MAX {
            String::new()
        } else {
            label(id)
        }
    };
    match (f.check, entry.fix) {
        ("overlap", Some(fix)) => format!(
            "overlap ({}): {} overlaps {} by {} cm in plan; fix: move {} {} cm {}",
            f.severity,
            named(fix.mover),
            named(fix.other),
            fix.depth_cm,
            named(fix.mover),
            fix.cm,
            fix.dir
        ),
        ("floating" | "support", Some(fix)) => format!(
            "{} ({}): {} floats above the floor; fix: move {} {} cm {}",
            f.check,
            f.severity,
            named(fix.mover),
            named(fix.mover),
            fix.cm,
            fix.dir
        ),
        ("sunken", Some(fix)) => format!(
            "sunken ({}): {} sits below the floor; fix: move {} {} cm {}",
            f.severity,
            named(fix.mover),
            named(fix.mover),
            fix.cm,
            fix.dir
        ),
        _ if f.b == u32::MAX => format!("{} ({}): {}", f.check, f.severity, named(f.a)),
        _ => format!(
            "{} ({}): {} and {}",
            f.check,
            f.severity,
            named(f.a),
            named(f.b)
        ),
    }
}

/// The spatial delta of one write: what `touched` (plus every changed row)
/// gained and lost between `before` and `after`.
pub(crate) fn spatial_delta(
    before: &Snapshot,
    after: &Snapshot,
    touched: &[ObjectId],
) -> SpatialDeltaDto {
    let mut keys: Vec<String> = before
        .ids
        .iter()
        .chain(&after.ids)
        .map(ToString::to_string)
        .collect();
    keys.sort();
    keys.dedup();
    let stable: BTreeMap<&str, u32> = keys
        .iter()
        .enumerate()
        .filter_map(|(number, key)| Some((key.as_str(), u32::try_from(number).ok()?)))
        .collect();
    let before_rows = renumbered(before, &stable);
    let after_rows = renumbered(after, &stable);
    let named: Vec<u32> = touched
        .iter()
        .filter_map(|id| stable.get(id.to_string().as_str()).copied())
        .collect();
    // The door's own check list: the ones its snapshot can drive, so a delta
    // never reports what `scene.spatial` would withhold.
    let (run, _skipped) = partition(None, before.missing_pose + after.missing_pose > 0);
    let delta = check_staged_with(&before_rows, &after_rows, &named, &run)
        .unwrap_or_else(|_| check_staged_ids(&before_rows, &after_rows, &named));

    // Labels come from the after snapshot, else the before one (removed nodes).
    let mut labels: BTreeMap<u32, String> = BTreeMap::new();
    for snap in [before, after] {
        for (id, actor) in snap.ids.iter().zip(&snap.actors) {
            if let Some(number) = stable.get(id.to_string().as_str()) {
                labels.insert(*number, object_label(&actor.name, id));
            }
        }
    }
    let label = |number: u32| {
        labels
            .get(&number)
            .cloned()
            .unwrap_or_else(|| format!("object {number}"))
    };

    // Red before yellow; the catalog's frozen order breaks ties.
    let mut fresh: Vec<&DeltaFinding> = delta.new_findings.iter().collect();
    fresh.sort_by_key(|entry| entry.finding.severity != "red");
    let new_findings: Vec<String> = fresh
        .iter()
        .take(DELTA_MAX_FINDINGS)
        .map(|entry| finding_text(entry, &label))
        .collect();
    let resolved: Vec<String> = delta
        .resolved
        .iter()
        .take(DELTA_MAX_FINDINGS)
        .map(|entry| format!("resolved {}", finding_text(entry, &label)))
        .collect();
    let listed = new_findings.len() + resolved.len();
    SpatialDeltaDto {
        ids: delta
            .touched
            .iter()
            .take(DELTA_MAX_IDS)
            .filter_map(|number| keys.get(usize::try_from(*number).ok()?).cloned())
            .collect(),
        new_findings,
        resolved,
        more: count(delta.new_findings.len() + delta.resolved.len() - listed),
    }
}
