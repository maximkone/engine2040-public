//! Level session: revision-checked preview/apply/undo over the level actor list.
//! Consumer is `build_level` (`level.build` door via re-export) and `build_game`
//! (offline replay of a recorded level). Owner is `engine-endpoint`; units are
//! meters in, integer cm inside via the parent solver. Budget and cost mirror
//! the parent build. Errors reuse the parent `LevelError`; stale revisions,
//! duplicate keys and undone keys write nothing. Determinism: same brief text
//! plus seed gives the same preview, receipt and digests.
use std::collections::{BTreeMap, BTreeSet};

use engine_arena::world::MatchWorld;
use engine_spatial::Actor;

use super::{
    actors_digest, envelope, room_shell, BuildReport, CostInfo, EditOutcome, LevelError,
    LevelRefusal, Placement, RelationEdit, SpineWrite,
};

/// One SpineWrite back as a level actor on the shell: the exact inverse of
/// the edit's `writes_of`, so a landed relation-diff edit reproduces the
/// actor list the solve produced (and its digest).
fn actor_of(write: &SpineWrite) -> Actor {
    Actor {
        id: write.id,
        name: write.name.clone(),
        x: write.pos[0],
        y: write.pos[1],
        z: write.pos[2],
        sx: write.scale[0],
        sy: write.scale[1],
        sz: write.scale[2],
        parent: None,
        physical: true,
        vx: 0.0,
        vz: 0.0,
        yaw: write.yaw,
    }
}

/// The room shell plus one actor per edit write, in write order (prop-id
/// order), the level spine a landed edit installs.
fn actors_of(edit: &RelationEdit) -> Vec<Actor> {
    let mut actors = room_shell();
    actors.extend(edit.writes.iter().map(actor_of));
    actors
}

/// Dry-run preview: the full chain with no writes.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionPreview {
    /// Envelope brief id.
    pub brief_id: u64,
    /// Shared LW6 envelope record line (`BriefEnvelope::record_line`).
    /// Every preview carries it so the write path traces end to end.
    pub record_line: String,
    /// Placements in prop-id order.
    pub placements: Vec<Placement>,
    /// Gate finding lines (empty on a pass).
    pub findings: Vec<String>,
    /// Relation delta lines the edit would make (ARC12).
    pub relation_diff: Vec<String>,
    /// SP5 conflict-set lines a refusal names (empty on a pass).
    pub conflict_set: Vec<String>,
    /// Repair rounds consumed.
    pub repairs_used: u8,
    /// `"pass"` or `"refused"`.
    pub verdict: String,
    /// Revision an apply would publish.
    pub resulting_revision: u64,
    /// FNV over the placement table.
    pub level_digest: u64,
    /// AB4 input line on a pass, empty on a refusal.
    pub next_ab4: String,
    /// Simulated cost.
    pub cost: CostInfo,
}

/// Landed apply receipt.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    /// Session revision after the apply.
    pub revision: u64,
    /// Idempotency key this receipt answers.
    pub key: u64,
    /// Stable undo identity: pass back to `undo`.
    pub undo_token: u64,
    /// Shared LW6 envelope record line (`BriefEnvelope::record_line`).
    /// Every landed write carries it so the audit traces end to end.
    pub record_line: String,
    /// Relation delta lines this edit made (`engine_spatial::describe` order).
    pub relation_diff: Vec<String>,
    /// Passing report.
    pub report: BuildReport,
}

/// Apply outcome: placed with a receipt, or refused with the refusal.
/// Refusals write nothing: revision and history are untouched.
#[derive(Debug, Clone, PartialEq)]
pub enum ApplyOutcome {
    /// Level placed.
    Placed(Applied),
    /// Budget exhausted: the refusal plus the unchanged revision.
    Refused {
        /// Why the build refused, with conflicts and findings.
        refusal: LevelRefusal,
        /// Unchanged session revision.
        revision: u64,
    },
}

/// Undo receipt for a reverted apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Undone {
    /// Session revision after the undo.
    pub revision: u64,
    /// Key of the batch that was undone.
    pub undone_key: u64,
}

#[derive(Debug, Clone)]
struct SessionEntry {
    key: u64,
    prev: Vec<Actor>,
}

/// Level session: revision-checked, idempotency-keyed preview/apply/undo
/// over the level actor list. Mirrors the `tx.*` door semantics at the
/// level layer: stale revisions, duplicate keys and undone keys write
/// nothing, and undo restores the exact previous list as a new revision.
/// The EA1a-e assistant loop is the behavioral contract; this session is
/// the level-shaped instance of it.
#[derive(Debug, Clone)]
pub struct LevelSession {
    revision: u64,
    current: Vec<Actor>,
    history: Vec<SessionEntry>,
    keys: BTreeMap<u64, (u64, Applied)>,
    undone: BTreeSet<u64>,
}

impl LevelSession {
    /// Fresh session over the room shell at revision zero.
    pub fn new() -> Self {
        Self {
            revision: 0,
            current: room_shell(),
            history: Vec::new(),
            keys: BTreeMap::new(),
            undone: BTreeSet::new(),
        }
    }

    /// Current session revision.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Current actor list (shell plus placed props).
    pub fn current(&self) -> &[Actor] {
        &self.current
    }

    /// Digest of the current actor list.
    pub fn current_digest(&self) -> u64 {
        actors_digest(&self.current)
    }

    /// The arena spine world this level installs: the boxes the spine step
    /// plays around. Empty until a write lands, and empty again after the
    /// undo, so the level sits on the spine exactly as long as it is applied.
    /// Errors: [`LevelError::SpineWorld`] when the level holds a box the
    /// arena cannot play.
    pub fn spine_world(&self) -> Result<MatchWorld, LevelError> {
        super::spine_world(&self.current)
    }

    /// Dry run: validate, solve the relation-diff edit, gate and report with
    /// no writes. The relation delta the edit would make rides the preview so
    /// a caller reviews relations, not coordinates.
    pub fn preview(&self, text: &str, seed: u64) -> Result<SessionPreview, LevelError> {
        let env = envelope(text, seed)?;
        let resulting_revision = self.revision + 1;
        match super::relation_edit(&env)? {
            EditOutcome::Edited(edit) => Ok(SessionPreview {
                brief_id: edit.report.brief_id,
                record_line: env.record_line(),
                placements: edit.report.placements.clone(),
                findings: Vec::new(),
                relation_diff: edit.changes,
                conflict_set: Vec::new(),
                repairs_used: edit.report.repairs_used,
                verdict: "pass".to_owned(),
                resulting_revision,
                level_digest: edit.report.level_digest,
                next_ab4: edit.report.next_ab4.clone(),
                cost: edit.report.cost.clone(),
            }),
            EditOutcome::Refused(refusal) => Ok(SessionPreview {
                brief_id: refusal.brief_id,
                record_line: env.record_line(),
                placements: Vec::new(),
                findings: refusal.findings.clone(),
                relation_diff: Vec::new(),
                conflict_set: refusal.conflict_set.clone(),
                repairs_used: refusal.repairs_used,
                verdict: "refused".to_owned(),
                resulting_revision,
                level_digest: 0,
                next_ab4: String::new(),
                cost: CostInfo {
                    solve_calls: 0,
                    nav_tiles: 0,
                    repair_rounds: refusal.repairs_used,
                    units: u64::from(refusal.repairs_used) * 500,
                },
            }),
        }
    }

    /// Atomic apply through the relation-diff edit path (ARC12): stale
    /// revisions, duplicate keys and undone keys write nothing; a refused
    /// build writes nothing; a placed build lands exactly one revision and
    /// its writes install the level spine. Same key plus same payload
    /// replays the recorded receipt.
    pub fn apply(
        &mut self,
        text: &str,
        seed: u64,
        expected_revision: u64,
        key: u64,
    ) -> Result<ApplyOutcome, LevelError> {
        if expected_revision != self.revision {
            return Err(LevelError::StaleRevision {
                expected: self.revision,
                got: expected_revision,
            });
        }
        let env = envelope(text, seed)?;
        if self.undone.contains(&key) {
            return Err(LevelError::UndoneKey { key });
        }
        if let Some((payload, applied)) = self.keys.get(&key) {
            if *payload == env.brief_id() {
                return Ok(ApplyOutcome::Placed(applied.clone()));
            }
            return Err(LevelError::DuplicateKey { key });
        }
        match super::relation_edit(&env)? {
            EditOutcome::Refused(refusal) => Ok(ApplyOutcome::Refused {
                refusal,
                revision: self.revision,
            }),
            EditOutcome::Edited(edit) => {
                let revision = self.revision + 1;
                let applied = Applied {
                    revision,
                    key,
                    undo_token: key,
                    record_line: env.record_line(),
                    relation_diff: edit.changes.clone(),
                    report: edit.report.clone(),
                };
                self.history.push(SessionEntry {
                    key,
                    prev: std::mem::replace(&mut self.current, actors_of(&edit)),
                });
                self.keys.insert(key, (env.brief_id(), applied.clone()));
                self.revision = revision;
                Ok(ApplyOutcome::Placed(applied))
            }
        }
    }

    /// Undo the newest applied batch as a new revision. A given token
    /// must match the newest batch key.
    pub fn undo(&mut self, token: Option<u64>) -> Result<Undone, LevelError> {
        let Some(newest) = self.history.last() else {
            return Err(LevelError::NothingToUndo);
        };
        if let Some(token) = token {
            if newest.key != token {
                return Err(LevelError::UndoMismatch {
                    expected: newest.key,
                    got: token,
                });
            }
        }
        let entry = match self.history.pop() {
            Some(entry) => entry,
            None => return Err(LevelError::NothingToUndo),
        };
        self.current = entry.prev;
        self.revision += 1;
        self.undone.insert(entry.key);
        Ok(Undone {
            revision: self.revision,
            undone_key: entry.key,
        })
    }
}

impl Default for LevelSession {
    fn default() -> Self {
        Self::new()
    }
}

/// Fresh session over the room shell at revision zero.
/// Test-only entry so session tests keep one construction site.
#[cfg(test)]
pub fn fresh_session() -> LevelSession {
    LevelSession::new()
}
