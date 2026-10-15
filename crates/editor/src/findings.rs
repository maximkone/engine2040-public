//! Findings panel: spatial findings on a staged AI proposal, fixes via commands.
//!
//! Consumer is the editor session reviewing assistant proposals (shell wiring
//! plus the `shell_ui` Findings tab). The data owner is [`FindingsPanel`]:
//! one staged [`Batch`] plus its spatial delta and per-finding repair proposals.
//! Units are findings (rows) and command batches. Determinism scope: same
//! document plus same batch gives the same rows in the same order. Errors are
//! typed [`FindingsError`]; a stale panel refreshes, never applies.

use std::fmt;

use engine_scene::ids::ObjectId;
use engine_scene::transform::{Hierarchy, HierarchyError, Transform};
use engine_spatial::{check_staged, suggest, Actor, Proposal, RepairKind};

use crate::capability::op_target;
use crate::commands::{self, Applied, Batch, BatchError, Document, Journal, Op, Preview};
use crate::gizmo::world_to_local;

/// Actor name for one table slot. Grammar-clean (`obj_000007`) so UUID text
/// never trips the spatial grammar check; index is the union-table position.
fn actor_name(index: usize) -> String {
    format!("obj_{index:06}")
}

/// Integer centimeters to meters.
fn cm_to_m(value: i32) -> f32 {
    value as f32 / 100.0
}

/// What rejected a panel action. Refusals change neither document nor journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindingsError {
    /// No proposal is staged; stage one first.
    NoProposal,
    /// The document moved on; refresh (clear plus re-stage) instead of applying.
    StaleProposal {
        /// Current document revision.
        expected: u64,
        /// Revision the proposal was staged against.
        got: u64,
    },
    /// No finding carries this index.
    UnknownFinding {
        /// The requested row.
        index: usize,
    },
    /// A repair names an actor outside the panel's table.
    UnknownTarget {
        /// The spatial actor id.
        id: u32,
    },
    /// The repair target is gone from the document.
    UnknownNode(ObjectId),
    /// The finding has no geometry fix (rename, bake or velocity work).
    NoAutoFix {
        /// The finding's check name.
        check: &'static str,
    },
    /// Spatial refused the repair; resolve the conflict set by hand.
    Refused {
        /// Human-readable reason plus the ids to resolve.
        reason: String,
    },
    /// The fixed pose crosses a near-zero scale; nothing was staged.
    DegenerateTarget(ObjectId),
    /// The command service refused the batch; nothing ran.
    Command(BatchError),
}

impl fmt::Display for FindingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoProposal => f.write_str("no AI proposal staged"),
            Self::StaleProposal { expected, got } => write!(
                f,
                "stale proposal: document is at {expected}, proposal staged at {got}; refresh first"
            ),
            Self::UnknownFinding { index } => write!(f, "no finding at row {index}"),
            Self::UnknownTarget { id } => write!(f, "repair targets unknown actor {id}"),
            Self::UnknownNode(id) => write!(f, "repair target {id} is gone"),
            Self::NoAutoFix { check } => write!(
                f,
                "finding `{check}` has no automatic fix: resolve it by hand, then recheck"
            ),
            Self::Refused { reason } => write!(f, "repair refused: {reason}"),
            Self::DegenerateTarget(id) => {
                write!(f, "fixed pose for {id} sits across a degenerate scale")
            }
            Self::Command(error) => write!(f, "command refused: {error}"),
        }
    }
}

impl std::error::Error for FindingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Command(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BatchError> for FindingsError {
    fn from(error: BatchError) -> Self {
        Self::Command(error)
    }
}

/// What the panel offers for one finding row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FixStatus {
    /// A geometry repair stages as proposal ops plus one `SetLocal`.
    Auto(RepairKind),
    /// No scene edit exists; the note says what to do by hand.
    Manual(&'static str),
    /// Spatial refused; the reason names the conflict set.
    Refused(String),
}

impl FixStatus {
    /// True when [`FindingsPanel::preview_fix`] can build a command batch.
    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto(_))
    }
}

impl fmt::Display for FixStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auto(kind) => write!(f, "auto fix: {kind:?}"),
            Self::Manual(note) => write!(f, "manual: {note}"),
            Self::Refused(reason) => write!(f, "refused: {reason}"),
        }
    }
}

/// One panel row: owned data, so the viewer holds no spatial borrow.
#[derive(Clone, Debug, PartialEq)]
pub struct FindingView {
    /// Row index into the panel (argument for preview/apply).
    pub index: usize,
    /// Spatial check name (`overlap`, `floating`, ...).
    pub check: &'static str,
    /// `red` or `yellow`.
    pub severity: &'static str,
    /// First actor involved.
    pub a: ObjectId,
    /// Second actor, if the finding names a pair.
    pub b: Option<ObjectId>,
    /// Overlap percent or headcount, per the check.
    pub pct: f32,
    /// What the panel offers for this row.
    pub fix: FixStatus,
}

/// Hand note for a repair kind with no scene delta.
fn manual_note(kind: RepairKind) -> &'static str {
    match kind {
        RepairKind::WidenDoor => "door width lives in the nav bake: widen it by hand",
        _ => "resolve by hand (rename, reparent, velocity or bake), then recheck",
    }
}

/// One staged AI proposal plus its spatial review.
struct StagedProposal {
    batch: Batch,
    base_revision: u64,
    /// Union of before/staged node ids, sorted by id text; position is the actor id.
    order: Vec<ObjectId>,
    findings: Vec<engine_spatial::Finding>,
    proposals: Vec<Proposal>,
}

/// Editor findings panel over the command service.
///
/// `stage` validates a proposal batch (no mutation) and lists the spatial delta
/// findings with severity; each automatic fix previews and applies as the
/// proposal ops plus one repairing `SetLocal` through [`commands`], so undo
/// restores the pre-proposal document. Manual and refused rows never touch
/// the document or the journal. A panel whose document moved on is stale:
/// previews and applies fail with [`FindingsError::StaleProposal`] until the
/// caller refreshes (clear) and stages a fresh proposal.
pub struct FindingsPanel {
    proposal: Option<StagedProposal>,
}

impl FindingsPanel {
    /// Empty panel: no proposal, no findings.
    pub fn new() -> Self {
        Self { proposal: None }
    }

    /// Rows currently listed.
    pub fn len(&self) -> usize {
        self.proposal
            .as_ref()
            .map_or(0, |staged| staged.findings.len())
    }

    /// True when no proposal is staged.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Revision the staged proposal was built against, if any.
    pub fn base_revision(&self) -> Option<u64> {
        self.proposal.as_ref().map(|staged| staged.base_revision)
    }

    /// True when there is nothing to apply: no proposal, or the document
    /// moved past the staged revision.
    pub fn is_stale(&self, doc: &Document) -> bool {
        self.proposal
            .as_ref()
            .is_none_or(|staged| staged.base_revision != doc.revision)
    }

    /// Drop the staged proposal. Returns the dropped finding count.
    pub fn refresh(&mut self) -> usize {
        let dropped = self.len();
        self.proposal = None;
        dropped
    }

    /// Stage an AI proposal: validate it, snapshot before/staged actor sets
    /// and list the spatial delta. Returns the finding count. Stores nothing on
    /// error: a stale or invalid batch leaves the previous state in place.
    pub fn stage(&mut self, doc: &Document, batch: &Batch) -> Result<usize, FindingsError> {
        if batch.base_revision != doc.revision {
            return Err(FindingsError::StaleProposal {
                expected: doc.revision,
                got: batch.base_revision,
            });
        }
        commands::validate(doc, batch).map_err(FindingsError::Command)?;
        let staged_tree = stage_tree(&doc.hierarchy, &batch.ops)?;
        let mut order = doc.hierarchy.propagation_order();
        order.extend(staged_tree.propagation_order());
        order.sort_by_key(|id| id.to_string());
        order.dedup();
        let before = snapshot_actors(&doc.hierarchy, &order);
        let staged = snapshot_actors(&staged_tree, &order);
        let delta = check_staged(&before, &staged);
        let proposals = suggest(&delta.findings, &staged, doc.revision);
        let count = delta.findings.len();
        self.proposal = Some(StagedProposal {
            batch: batch.clone(),
            base_revision: doc.revision,
            order,
            findings: delta.findings,
            proposals,
        });
        Ok(count)
    }

    /// One row's owned view. None when no proposal is staged, the index is
    /// past the end, or the finding names an actor outside the table.
    pub fn finding(&self, index: usize) -> Option<FindingView> {
        let staged = self.proposal.as_ref()?;
        let found = staged.findings.get(index)?;
        let a = staged.order.get(found.a as usize).copied()?;
        let b = if found.b == u32::MAX {
            None
        } else {
            Some(staged.order.get(found.b as usize).copied()?)
        };
        let fix = match staged.proposals.get(index) {
            Some(Proposal::Repair(repair)) => match repair.kind {
                RepairKind::NudgeOut | RepairKind::SnapToGrid | RepairKind::ResizeCollider => {
                    FixStatus::Auto(repair.kind)
                }
                kind => FixStatus::Manual(manual_note(kind)),
            },
            Some(Proposal::Refused(refused)) => FixStatus::Refused(refused.reason.clone()),
            None => FixStatus::Refused("missing repair proposal".to_string()),
        };
        Some(FindingView {
            index,
            check: found.check,
            severity: found.severity,
            a,
            b,
            pct: found.pct,
            fix,
        })
    }

    /// Preview one automatic fix without side effects: the proposal ops plus
    /// the repairing `SetLocal`, validated against the live document.
    pub fn preview_fix(&self, doc: &Document, index: usize) -> Result<Preview, FindingsError> {
        let key = self
            .proposal
            .as_ref()
            .ok_or(FindingsError::NoProposal)?
            .batch
            .idempotency_key;
        let fix = self.fix_batch(doc, index, key)?;
        commands::validate(doc, &fix).map_err(FindingsError::Command)
    }

    /// Apply one automatic fix atomically (preview, then commit) through the
    /// command service. Undo restores the pre-proposal document via the journal.
    pub fn apply_fix(
        &self,
        doc: &mut Document,
        journal: &mut Journal,
        index: usize,
        idempotency_key: u64,
    ) -> Result<Applied, FindingsError> {
        let fix = self.fix_batch(doc, index, idempotency_key)?;
        // Preview first so a fix that stopped validating reports before commit.
        commands::validate(doc, &fix).map_err(FindingsError::Command)?;
        commands::apply(doc, journal, &fix).map_err(FindingsError::Command)
    }

    /// Build the fix batch for one row: the staged proposal ops plus one
    /// repairing `SetLocal` for the spatial delta. Pure: stages on a private
    /// copy, never touches the caller's document.
    fn fix_batch(
        &self,
        doc: &Document,
        index: usize,
        idempotency_key: u64,
    ) -> Result<Batch, FindingsError> {
        let staged = self.proposal.as_ref().ok_or(FindingsError::NoProposal)?;
        if doc.revision != staged.base_revision {
            return Err(FindingsError::StaleProposal {
                expected: doc.revision,
                got: staged.base_revision,
            });
        }
        let found = staged
            .findings
            .get(index)
            .ok_or(FindingsError::UnknownFinding { index })?;
        let repair = match staged.proposals.get(index) {
            Some(Proposal::Repair(repair)) => repair,
            Some(Proposal::Refused(refused)) => {
                return Err(FindingsError::Refused {
                    reason: refused.reason.clone(),
                });
            }
            None => {
                return Err(FindingsError::Refused {
                    reason: "missing repair proposal".to_string(),
                });
            }
        };
        match repair.kind {
            RepairKind::NudgeOut | RepairKind::SnapToGrid | RepairKind::ResizeCollider => {}
            _ => {
                return Err(FindingsError::NoAutoFix { check: found.check });
            }
        }
        let target = staged.order.get(repair.target_id as usize).copied().ok_or(
            FindingsError::UnknownTarget {
                id: repair.target_id,
            },
        )?;
        let tree = stage_tree(&doc.hierarchy, &staged.batch.ops)?;
        let world = tree
            .world(target)
            .ok_or(FindingsError::UnknownNode(target))?;
        let parent = tree
            .parent(target)
            .ok_or(FindingsError::UnknownNode(target))?;
        let parent_world = if parent.is_nil() {
            Transform::identity()
        } else {
            tree.world(parent)
                .ok_or(FindingsError::UnknownNode(parent))?
        };
        let mut posed = world;
        posed.position = [
            world.position[0] + cm_to_m(repair.delta_cm[0]),
            world.position[1] + cm_to_m(repair.delta_cm[1]),
            world.position[2] + cm_to_m(repair.delta_cm[2]),
        ];
        if repair.kind == RepairKind::ResizeCollider {
            let Some(sizes) = repair.size_cm else {
                return Err(FindingsError::Refused {
                    reason: format!("malformed-repair: resize for {target} carries no sizes"),
                });
            };
            posed.scale = [cm_to_m(sizes[0]), cm_to_m(sizes[1]), cm_to_m(sizes[2])];
        }
        let local =
            world_to_local(parent_world, posed).ok_or(FindingsError::DegenerateTarget(target))?;
        let mut ops = staged.batch.ops.clone();
        ops.push(Op::SetLocal { id: target, local });
        Ok(Batch {
            ops,
            base_revision: staged.base_revision,
            idempotency_key,
            provenance: None,
        })
    }
}

impl Default for FindingsPanel {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot one hierarchy into spatial actors: world position plus world
/// scale as the 1 m-cube extents, grammar-clean names by table index. Nodes
/// missing from `tree` (removed by the proposal) are skipped; every other
/// node maps, even degenerate ones, so spatial reports them instead of the
/// adapter silently dropping them.
fn snapshot_actors(tree: &Hierarchy, order: &[ObjectId]) -> Vec<Actor> {
    let mut out = Vec::new();
    for (index, id) in order.iter().enumerate() {
        let Some(world) = tree.world(*id) else {
            continue;
        };
        out.push(Actor {
            id: index as u32,
            name: actor_name(index),
            x: world.position[0],
            y: world.position[1],
            z: world.position[2],
            sx: world.scale[0].abs(),
            sy: world.scale[1].abs(),
            sz: world.scale[2].abs(),
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        });
    }
    out
}

/// Stage batch ops on a private hierarchy copy. The caller commits nothing:
/// failures name the culprit and leave the input untouched.
fn stage_tree(tree: &Hierarchy, ops: &[Op]) -> Result<Hierarchy, BatchError> {
    let mut staged = tree.clone();
    for op in ops {
        let result = match *op {
            Op::Insert { id, parent, local } => staged.insert(id, parent, local),
            Op::Remove { id } => staged.remove(id),
            Op::SetLocal { id, local } => staged.set_local(id, local),
            Op::Reparent { id, new_parent } => staged.reparent(id, new_parent),
            // Bindings leave poses alone; the node must still exist.
            Op::SetMaterial { id, .. }
            | Op::SetComponent { id, .. }
            | Op::RemoveComponent { id, .. } => {
                if staged.contains(id) {
                    Ok(())
                } else {
                    Err(engine_scene::transform::HierarchyError::UnknownNode)
                }
            }
        };
        result.map_err(|error| {
            let id = op_target(*op);
            let parent = match *op {
                Op::Insert { parent, .. } => parent,
                Op::Reparent { new_parent, .. } => new_parent,
                Op::Remove { .. }
                | Op::SetLocal { .. }
                | Op::SetMaterial { .. }
                | Op::SetComponent { .. }
                | Op::RemoveComponent { .. } => ObjectId::NIL,
            };
            match error {
                HierarchyError::UnknownNode => BatchError::UnknownNode { id },
                HierarchyError::UnknownParent => BatchError::UnknownParent { id, parent },
                HierarchyError::Duplicate => BatchError::Duplicate { id },
                HierarchyError::Cycle => BatchError::Cycle {
                    id,
                    new_parent: parent,
                },
                HierarchyError::NonFinite => BatchError::NonFinite { id },
                HierarchyError::DegenerateScale => BatchError::DegenerateScale { id },
                HierarchyError::NilId => BatchError::NilId,
            }
        })?;
    }
    Ok(staged)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose_at(x: f32) -> Transform {
        Transform::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    fn pose_xyz(x: f32, y: f32, z: f32) -> Transform {
        Transform::new([x, y, z], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    fn insert_batch(id: ObjectId, parent: ObjectId, local: Transform, rev: u64, key: u64) -> Batch {
        Batch {
            ops: vec![Op::Insert { id, parent, local }],
            base_revision: rev,
            idempotency_key: key,
            provenance: None,
        }
    }

    /// Two grounded roots 10 m apart: the clean before-state every test builds on.
    fn seeded_doc() -> (Document, Journal, ObjectId, ObjectId) {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let a = ObjectId::new();
        let b = ObjectId::new();
        for (i, id) in [a, b].into_iter().enumerate() {
            let batch = insert_batch(
                id,
                ObjectId::NIL,
                pose_at(i as f32 * 10.0),
                doc.revision,
                100 + i as u64,
            );
            commands::apply(&mut doc, &mut journal, &batch).unwrap();
        }
        (doc, journal, a, b)
    }

    /// Proposal dragging `b` onto `a`: one red overlap finding.
    fn overlap_batch(b: ObjectId, rev: u64) -> Batch {
        Batch {
            ops: vec![Op::SetLocal {
                id: b,
                local: pose_at(0.0),
            }],
            base_revision: rev,
            idempotency_key: 700,
            provenance: None,
        }
    }

    #[test]
    fn clean_proposal_stages_with_no_findings() {
        let (doc, journal, a, _b) = seeded_doc();
        let mut panel = FindingsPanel::new();
        assert!(panel.is_empty());
        assert!(panel.is_stale(&doc));
        // A grounded nudge nearby stays clean: no rows, nothing to fix.
        let batch = Batch {
            ops: vec![Op::SetLocal {
                id: a,
                local: pose_at(2.0),
            }],
            base_revision: doc.revision,
            idempotency_key: 701,
            provenance: None,
        };
        assert_eq!(panel.stage(&doc, &batch).unwrap(), 0);
        assert!(panel.is_empty());
        assert!(!panel.is_stale(&doc));
        assert_eq!(panel.base_revision(), Some(doc.revision));
        assert!(panel.finding(0).is_none());
        assert_eq!(
            panel.preview_fix(&doc, 0).unwrap_err(),
            FindingsError::UnknownFinding { index: 0 }
        );
        assert_eq!(journal.past_len(), 2);
    }

    #[test]
    fn overlap_proposal_lists_one_red_finding() {
        let (doc, _journal, a, b) = seeded_doc();
        let mut panel = FindingsPanel::new();
        assert_eq!(
            panel.stage(&doc, &overlap_batch(b, doc.revision)).unwrap(),
            1
        );
        assert_eq!(panel.len(), 1);
        let view = panel.finding(0).unwrap();
        assert_eq!(view.index, 0);
        assert_eq!(view.check, "overlap");
        assert_eq!(view.severity, "red");
        assert!(view.pct > 0.0);
        // The pair names both proposal nodes, single rows carry no second id.
        let mut pair = [view.a, view.b.unwrap()];
        pair.sort_by_key(|id| id.to_string());
        let mut expected = [a, b];
        expected.sort_by_key(|id| id.to_string());
        assert_eq!(pair, expected);
        assert!(view.fix.is_auto());
        assert_eq!(view.fix, FixStatus::Auto(RepairKind::NudgeOut));
    }

    #[test]
    fn fix_preview_validates_without_mutating() {
        let (doc, journal, _a, b) = seeded_doc();
        let mut panel = FindingsPanel::new();
        panel.stage(&doc, &overlap_batch(b, doc.revision)).unwrap();
        let snapshot = doc.clone();
        let past = journal.past_len();
        let preview = panel.preview_fix(&doc, 0).unwrap();
        // Proposal op plus the repairing move, one revision ahead.
        assert_eq!(preview.effects.len(), 2);
        assert_eq!(preview.resulting_revision, doc.revision + 1);
        assert_eq!(doc, snapshot);
        assert_eq!(journal.past_len(), past);
    }

    #[test]
    fn fix_apply_moves_clear_and_undo_restores() {
        let (mut doc, mut journal, a, b) = seeded_doc();
        let before = doc.clone();
        let mut panel = FindingsPanel::new();
        panel.stage(&doc, &overlap_batch(b, doc.revision)).unwrap();
        let applied = panel.apply_fix(&mut doc, &mut journal, 0, 710).unwrap();
        assert_eq!(applied.revision, doc.revision);
        assert_eq!(applied.effects.len(), 2);
        // Penetration was a full meter, so the repair separates the pair by
        // that plus 1 cm; either node may move (spatial nudges the higher id).
        let gap = (doc.hierarchy.world(a).unwrap().position[0]
            - doc.hierarchy.world(b).unwrap().position[0])
            .abs();
        assert!((gap - 1.01).abs() <= 1e-4, "gap {gap}");
        assert_eq!(doc.hierarchy.world(b).unwrap().position[1], 0.0);
        // The panel went stale with the revision bump; undo restores the exact document.
        assert!(panel.is_stale(&doc));
        commands::undo(&mut doc, &mut journal).expect("fix batch is undoable");
        assert_eq!(doc.hierarchy, before.hierarchy);
        commands::redo(&mut doc, &mut journal).expect("fix batch is redoable");
        let gap = (doc.hierarchy.world(a).unwrap().position[0]
            - doc.hierarchy.world(b).unwrap().position[0])
            .abs();
        assert!((gap - 1.01).abs() <= 1e-4, "gap {gap}");
    }

    #[test]
    fn stale_panel_refreshes_never_applies() {
        let (mut doc, mut journal, _a, b) = seeded_doc();
        let mut panel = FindingsPanel::new();
        panel.stage(&doc, &overlap_batch(b, doc.revision)).unwrap();
        // A second writer lands first and moves the revision on.
        let newcomer = ObjectId::new();
        let bump = insert_batch(newcomer, ObjectId::NIL, pose_at(30.0), doc.revision, 720);
        commands::apply(&mut doc, &mut journal, &bump).unwrap();
        let staled = FindingsError::StaleProposal {
            expected: doc.revision,
            got: doc.revision - 1,
        };
        let snapshot = doc.clone();
        let past = journal.past_len();
        assert!(panel.is_stale(&doc));
        assert_eq!(panel.preview_fix(&doc, 0).unwrap_err(), staled);
        assert_eq!(
            panel.apply_fix(&mut doc, &mut journal, 0, 721).unwrap_err(),
            staled
        );
        assert_eq!(doc, snapshot, "stale fixes never apply");
        assert_eq!(journal.past_len(), past);
        // Refresh drops the stale rows; a fresh proposal stages against the new revision.
        assert_eq!(panel.refresh(), 1);
        assert!(panel.is_empty());
        assert_eq!(
            panel.stage(&doc, &overlap_batch(b, doc.revision)).unwrap(),
            1
        );
        assert!(!panel.is_stale(&doc));
    }

    #[test]
    fn invalid_proposal_reports_command_error_and_stores_nothing() {
        let (doc, _journal, _a, b) = seeded_doc();
        let mut panel = FindingsPanel::new();
        let bad = Batch {
            ops: vec![Op::SetLocal {
                id: b,
                local: Transform::new([f32::NAN, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            }],
            base_revision: doc.revision,
            idempotency_key: 730,
            provenance: None,
        };
        let error = panel.stage(&doc, &bad).unwrap_err();
        assert!(matches!(
            error,
            FindingsError::Command(BatchError::NonFinite { .. })
        ));
        assert!(panel.is_empty());
        // A stale base never even validates: the revision gate fires first.
        let stale = Batch {
            ops: vec![],
            base_revision: doc.revision + 5,
            idempotency_key: 731,
            provenance: None,
        };
        assert_eq!(
            panel.stage(&doc, &stale).unwrap_err(),
            FindingsError::StaleProposal {
                expected: doc.revision,
                got: doc.revision + 5
            }
        );
        assert!(panel.is_empty());
    }

    #[test]
    fn density_finding_is_manual_and_never_applies() {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        // Nine grounded cubes inside one meter cell: density fires (manual fix).
        let mut ops = Vec::new();
        for i in 0..9 {
            ops.push(Op::Insert {
                id: ObjectId::new(),
                parent: ObjectId::NIL,
                local: pose_xyz(i as f32 * 0.1, 0.5, 0.0),
            });
        }
        let batch = Batch {
            ops,
            base_revision: doc.revision,
            idempotency_key: 740,
            provenance: None,
        };
        let mut panel = FindingsPanel::new();
        let count = panel.stage(&doc, &batch).unwrap();
        assert!(count >= 2, "overlaps plus one density finding, got {count}");
        let density = (0..count)
            .map(|i| panel.finding(i).unwrap())
            .find(|view| view.check == "overlapDensity")
            .expect("density finding is listed");
        assert_eq!(density.severity, "yellow");
        assert!(!density.fix.is_auto());
        let snapshot = doc.clone();
        let past = journal.past_len();
        assert!(matches!(
            panel.preview_fix(&doc, density.index).unwrap_err(),
            FindingsError::NoAutoFix {
                check: "overlapDensity"
            }
        ));
        assert!(matches!(
            panel
                .apply_fix(&mut doc, &mut journal, density.index, 741)
                .unwrap_err(),
            FindingsError::NoAutoFix {
                check: "overlapDensity"
            }
        ));
        assert_eq!(doc, snapshot, "manual rows never apply");
        assert_eq!(journal.past_len(), past);
        assert_eq!(panel.len(), count, "panel keeps its rows after the refusal");
    }

    #[test]
    fn zero_scale_proposal_repairs_through_resize() {
        let (mut doc, mut journal, a, _b) = seeded_doc();
        let flat = Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0]);
        let batch = Batch {
            ops: vec![Op::SetLocal { id: a, local: flat }],
            base_revision: doc.revision,
            idempotency_key: 750,
            provenance: None,
        };
        let mut panel = FindingsPanel::new();
        panel.stage(&doc, &batch).unwrap();
        let scale = (0..panel.len())
            .map(|i| panel.finding(i).unwrap())
            .find(|view| view.check == "scale")
            .expect("scale finding is listed");
        assert_eq!(scale.severity, "red");
        assert_eq!(scale.fix, FixStatus::Auto(RepairKind::ResizeCollider));
        panel
            .apply_fix(&mut doc, &mut journal, scale.index, 751)
            .unwrap();
        // The repair grounds the unit cube: scale back to 1 m, resting on the plane.
        let world = doc.hierarchy.world(a).unwrap();
        assert!((world.scale[0] - 1.0).abs() <= 1e-4, "{world:?}");
        assert!((world.position[1] - 0.5).abs() <= 1e-4, "{world:?}");
        commands::undo(&mut doc, &mut journal).expect("resize fix is undoable");
        assert_eq!(doc.hierarchy.local(a), Some(pose_at(0.0)));
    }

    #[test]
    fn shell_roundtrip_stages_applies_and_undoes() {
        use crate::shell::Shell;

        let mut shell = Shell::new();
        let a = ObjectId::new();
        let b = ObjectId::new();
        shell.insert_node(a, ObjectId::NIL, pose_at(0.0)).unwrap();
        shell.insert_node(b, ObjectId::NIL, pose_at(10.0)).unwrap();
        let batch = Batch {
            ops: vec![Op::SetLocal {
                id: b,
                local: pose_at(0.0),
            }],
            base_revision: shell.revision(),
            idempotency_key: 760,
            provenance: None,
        };
        assert_eq!(shell.stage_proposal(&batch).unwrap(), 1);
        let view = shell.finding(0).unwrap();
        assert_eq!((view.check, view.severity), ("overlap", "red"));
        let preview = shell.preview_fix(0).unwrap();
        assert_eq!(preview.effects.len(), 2);
        shell.apply_fix(0).unwrap();
        let gap = (shell.document().hierarchy.world(a).unwrap().position[0]
            - shell.document().hierarchy.world(b).unwrap().position[0])
            .abs();
        assert!((gap - 1.01).abs() <= 1e-4, "gap {gap}");
        shell.undo().expect("shell undoes the fix batch");
        assert_eq!(shell.document().hierarchy.local(b), Some(pose_at(10.0)));
    }
}
