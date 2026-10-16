//! Level session: revision-checked preview/apply/undo over the level actor list.
//! Consumer is `build_level` (`level.build` door via re-export) and `build_game`
//! (offline replay of a recorded level). Owner is `engine-endpoint`; units are
//! meters in, integer cm inside via the parent solver. Budget and cost mirror
//! the parent build. Errors reuse the parent `LevelError`; stale revisions,
//! duplicate keys and undone keys write nothing. Determinism: same brief text
//! plus seed gives the same preview, receipt and digests.
use std::collections::{BTreeMap, BTreeSet};

use engine_spatial::Actor;

use super::{
    actors_digest, build, envelope, room_shell, BuildOutcome, BuildReport, CostInfo, LevelError,
    LevelRefusal, Placement,
};

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

    /// Dry run: validate, solve, gate and report with no writes.
    pub fn preview(&self, text: &str, seed: u64) -> Result<SessionPreview, LevelError> {
        let env = envelope(text, seed)?;
        let resulting_revision = self.revision + 1;
        match build(&env)? {
            BuildOutcome::Placed(level) => Ok(SessionPreview {
                brief_id: level.report.brief_id,
                record_line: env.record_line(),
                placements: level.report.placements.clone(),
                findings: Vec::new(),
                repairs_used: level.report.repairs_used,
                verdict: "pass".to_owned(),
                resulting_revision,
                level_digest: level.report.level_digest,
                next_ab4: level.report.next_ab4.clone(),
                cost: level.report.cost.clone(),
            }),
            BuildOutcome::Refused(refusal) => Ok(SessionPreview {
                brief_id: refusal.brief_id,
                record_line: env.record_line(),
                placements: Vec::new(),
                findings: refusal.findings.clone(),
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

    /// Atomic apply: stale revisions, duplicate keys and undone keys
    /// write nothing; a refused build writes nothing; a placed build
    /// lands exactly one revision. Same key plus same payload replays the
    /// recorded receipt.
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
        match build(&env)? {
            BuildOutcome::Refused(refusal) => Ok(ApplyOutcome::Refused {
                refusal,
                revision: self.revision,
            }),
            BuildOutcome::Placed(level) => {
                let revision = self.revision + 1;
                let applied = Applied {
                    revision,
                    key,
                    undo_token: key,
                    record_line: env.record_line(),
                    report: level.report.clone(),
                };
                self.history.push(SessionEntry {
                    key,
                    prev: std::mem::replace(&mut self.current, level.actors),
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
