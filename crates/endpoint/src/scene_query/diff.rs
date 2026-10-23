//! The semantic change between two document revisions (AX23): which objects
//! moved and which relation edges appeared or vanished, plus the content
//! digest pair. `tx.apply` and `scene.query kind diff` answer with this one
//! list; a revision diff replays the journal on clones, so the live document,
//! budget and ledger never move.
//!
//! Owner: endpoint. Consumers: `mcp::tx_apply`, `ServerState::scene_query`.
//! Determinism: ids sort by UUID text and edges keep the engine-spatial frozen
//! `(a, relation, b)` order, so equal pairs give equal bytes.

use std::collections::{BTreeMap, BTreeSet};

use engine_core::Fnv64;
use engine_editor::commands::{self, Applied, Document, Journal};
use engine_spatial::diff::diff_graph;

use super::super::mcp::{ChangeListDto, McpToolError};
use super::super::scene_tools::{change_list, snapshot, Snapshot};
use super::{Actor, QueryError, QueryKind, SceneQueryArgs, SceneQueryOutput};
use crate::receipts::ChangeList;

/// The semantic part of a change list: touched object ids in id-text order and
/// the relation edges gained and lost, one row per edge.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SemanticChange {
    /// Object ids the change names, in id-text order.
    pub ids: Vec<String>,
    /// Edges only on the after side: `+ relation <a> <rel> <b>`.
    pub edges_added: Vec<String>,
    /// Edges only on the before side: `- relation <a> <rel> <b>`.
    pub edges_removed: Vec<String>,
}

/// True when a world pose lane differs; a NaN lane counts as moved, the same
/// rule the change list uses for its moved rows.
fn lanes_moved(before: &Actor, after: &Actor) -> bool {
    let lanes = |actor: &Actor| {
        [
            actor.x, actor.y, actor.z, actor.yaw, actor.sx, actor.sy, actor.sz,
        ]
    };
    lanes(before) != lanes(after)
}

/// Changed object ids: present on one side only, or with a moved world pose.
fn changed_ids(pre: &Snapshot, post: &Snapshot) -> BTreeSet<String> {
    let before: BTreeMap<String, &Actor> = pre
        .ids
        .iter()
        .zip(&pre.actors)
        .map(|(id, actor)| (id.to_string(), actor))
        .collect();
    let after: BTreeMap<String, &Actor> = post
        .ids
        .iter()
        .zip(&post.actors)
        .map(|(id, actor)| (id.to_string(), actor))
        .collect();
    let mut ids = BTreeSet::new();
    for (key, actor) in &after {
        match before.get(key) {
            None => {
                ids.insert(key.clone());
            }
            Some(old) if lanes_moved(old, actor) => {
                ids.insert(key.clone());
            }
            Some(_) => {}
        }
    }
    for key in before.keys() {
        if !after.contains_key(key) {
            ids.insert(key.clone());
        }
    }
    ids
}

/// Both snapshots renumbered by position in the sorted union of their ids, so
/// one node keeps one graph id on both sides, plus the id text per number.
/// Snapshot ids are positions in each document's own id list and shift when a
/// write inserts or removes a node, so the raw pair would invent edges.
fn stable_pair(pre: &Snapshot, post: &Snapshot) -> (Vec<Actor>, Vec<Actor>, Vec<String>) {
    let keys: BTreeSet<String> = pre
        .ids
        .iter()
        .chain(&post.ids)
        .map(ToString::to_string)
        .collect();
    let keys: Vec<String> = keys.into_iter().collect();
    let numbered = |snap: &Snapshot| -> Vec<Actor> {
        let mut numbers: BTreeMap<&str, u32> = BTreeMap::new();
        for (index, key) in keys.iter().enumerate() {
            if let Ok(number) = u32::try_from(index) {
                numbers.insert(key.as_str(), number);
            }
        }
        snap.ids
            .iter()
            .zip(&snap.actors)
            .filter_map(|(id, actor)| {
                let text = id.to_string();
                let number = *numbers.get(text.as_str())?;
                Some(Actor {
                    id: number,
                    ..actor.clone()
                })
            })
            .collect()
    };
    (numbered(pre), numbered(post), keys)
}

/// Semantic change between two snapshots: objects added, removed or moved and
/// the relation edges gained or lost, every id rendered as its object id. Ids
/// come out in id-text order; edges keep the engine-spatial frozen
/// `(a, relation, b)` order, so the same pair of documents gives the same
/// bytes.
pub(crate) fn semantic_change(pre: &Snapshot, post: &Snapshot) -> SemanticChange {
    let (before_actors, after_actors, keys) = stable_pair(pre, post);
    let diff = diff_graph(
        &engine_spatial::relation::build(&before_actors),
        &engine_spatial::relation::build(&after_actors),
    );
    let mut ids = changed_ids(pre, post);
    let uuid = |number: u32| -> Option<&str> {
        usize::try_from(number)
            .ok()
            .and_then(|index| keys.get(index))
            .map(String::as_str)
    };
    let mut edges_added = Vec::with_capacity(diff.added.len());
    for (a, rel, b) in &diff.added {
        let (Some(a), Some(b)) = (uuid(*a), uuid(*b)) else {
            continue;
        };
        edges_added.push(format!("+ relation {a} {} {b}", rel.name()));
        ids.insert(a.to_owned());
        ids.insert(b.to_owned());
    }
    let mut edges_removed = Vec::with_capacity(diff.removed.len());
    for (a, rel, b) in &diff.removed {
        let (Some(a), Some(b)) = (uuid(*a), uuid(*b)) else {
            continue;
        };
        edges_removed.push(format!("- relation {a} {} {b}", rel.name()));
        ids.insert(a.to_owned());
        ids.insert(b.to_owned());
    }
    SemanticChange {
        ids: ids.into_iter().collect(),
        edges_added,
        edges_removed,
    }
}

/// What one revision pair changed: the bounded object list, the semantic part
/// and the document digests on both sides.
pub(crate) struct RevisionDiff {
    /// Objects added, removed and moved, clipped to the receipt cap.
    pub changes: ChangeList,
    /// Touched ids and the relation edges gained or lost.
    pub semantics: SemanticChange,
    /// Content digest of the before state.
    pub before_digest: [u8; 32],
    /// Content digest of the after state.
    pub after_digest: [u8; 32],
}

/// Change list and digest pair between two revisions the journal holds,
/// rebuilt on staged clones. `revision_a` is the before side; the live
/// document, journal, budget and ledger never move.
pub(crate) fn revision_diff(
    doc: &Document,
    journal: &Journal,
    revision_a: u64,
    revision_b: u64,
) -> Result<RevisionDiff, McpToolError> {
    let before = state_at(doc, journal, revision_a)
        .ok_or_else(|| unknown_revision(doc, journal, revision_a))?;
    let after = state_at(doc, journal, revision_b)
        .ok_or_else(|| unknown_revision(doc, journal, revision_b))?;
    let (pre, post) = (snapshot(&before), snapshot(&after));
    Ok(RevisionDiff {
        changes: change_list(&pre, &post, revision_a, revision_b),
        semantics: semantic_change(&pre, &post),
        before_digest: commands::document_digest(&before),
        after_digest: commands::document_digest(&after),
    })
}

/// The change list `tx.apply` answers: the live before/after pair, or the
/// original pair rebuilt from the journal when the key replayed a landed batch.
/// A landed write bumps the revision, so `applied.revision == pre_revision`
/// means the retained receipt answered and nothing landed now; a pair the
/// journal no longer holds falls back to the live one, as before.
pub(crate) fn applied_change(
    doc: &Document,
    journal: &Journal,
    applied: &Applied,
    pre_revision: u64,
    before: &Snapshot,
    after: &Snapshot,
) -> ChangeListDto {
    let replay = if applied.revision == pre_revision {
        revision_diff(
            doc,
            journal,
            applied.revision.saturating_sub(1),
            applied.revision,
        )
        .ok()
    } else {
        None
    };
    match replay {
        Some(diff) => {
            let mut change = ChangeListDto::from(diff.changes);
            change.digest_before = crate::play::hex_bytes(&diff.before_digest);
            change.digest_after = crate::play::hex_bytes(&diff.after_digest);
            change.with_semantics(diff.semantics)
        }
        None => {
            let mut change = ChangeListDto::from(change_list(
                before,
                after,
                applied.revision.saturating_sub(1),
                applied.revision,
            ));
            change.digest_before = crate::play::hex_bytes(&applied.before_digest);
            change.digest_after = crate::play::hex_bytes(&applied.after_digest);
            change.with_semantics(semantic_change(before, after))
        }
    }
}

/// The document state at `revision`, rebuilt from the live document by undoing
/// the newer journal entries on a staged clone. `None` when the journal no
/// longer holds it.
fn state_at(doc: &Document, journal: &Journal, revision: u64) -> Option<Document> {
    if !held(doc, journal, revision) {
        return None;
    }
    let mut state = doc.clone();
    let mut staged = journal.clone();
    while staged
        .past
        .last()
        .is_some_and(|entry| entry.result_revision > revision)
    {
        commands::undo(&mut state, &mut staged)?;
    }
    Some(state)
}

/// True when `revision` is a state boundary the journal can rebuild: the live
/// revision, or the base/result of an applied entry still in `past`.
fn held(doc: &Document, journal: &Journal, revision: u64) -> bool {
    revision == doc.revision
        || journal
            .past
            .iter()
            .any(|entry| entry.base_revision == revision || entry.result_revision == revision)
}

/// Refusal for a revision the journal no longer holds: the held span, the
/// reason it is gone, and the read that reports the current revision.
fn unknown_revision(doc: &Document, journal: &Journal, revision: u64) -> McpToolError {
    let oldest = journal
        .past
        .first()
        .map_or(doc.revision, |entry| entry.base_revision);
    let current = doc.revision;
    McpToolError::new(
        "unknown_revision",
        format!(
            "revision {revision} is not held by the journal; it keeps {oldest}..={current} \
             (evicted and undone revisions are gone); hint: call scene.diff with revision_a \
             and revision_b inside {oldest}..={current}, or scene.read for the current \
             revision {current}"
        ),
    )
}

/// Door entry for `scene.query kind diff`: the card requires both revisions,
/// so the defaults only guard a direct `run_query` call.
pub(super) fn answer(
    doc: &Document,
    journal: &Journal,
    args: &SceneQueryArgs,
) -> Result<SceneQueryOutput, McpToolError> {
    scene_diff_output(
        doc,
        journal,
        args.revision_a.unwrap_or(0),
        args.revision_b.unwrap_or(0),
    )
}

/// Typed refusal if a diff ever reaches the actor-scan runner.
pub(super) fn needs_journal() -> QueryError {
    QueryError::new(
        "needs_journal",
        "kind `diff` compares two document revisions, not actor rows",
        "call scene.query kind diff with revision_a and revision_b; the door \
         answers it before the actor scan",
    )
}

/// `scene.query kind diff`: the change list between two held revisions, with
/// the digest pair, rendered as the same DTO `tx.apply` returns.
fn scene_diff_output(
    doc: &Document,
    journal: &Journal,
    revision_a: u64,
    revision_b: u64,
) -> Result<SceneQueryOutput, McpToolError> {
    let diff = revision_diff(doc, journal, revision_a, revision_b)?;
    let mut change = ChangeListDto::from(diff.changes);
    change.digest_before = crate::play::hex_bytes(&diff.before_digest);
    change.digest_after = crate::play::hex_bytes(&diff.after_digest);
    let change = change.with_semantics(diff.semantics);
    let text = rmcp::serde_json::to_string(&change).unwrap_or_default();
    let mut digest = Fnv64::new();
    digest.write(b"e2scene_diff1");
    digest.write(text.as_bytes());
    Ok(SceneQueryOutput {
        kind: QueryKind::Diff.name().to_owned(),
        digest: digest.finish(),
        bytes: u64::try_from(text.len()).unwrap_or(u64::MAX),
        truncated: change.truncated,
        count: change.ids_total,
        repeat_count: 0,
        hits: Vec::new(),
        ids: change.ids.clone(),
        conflicts: Vec::new(),
        change: Some(change),
    })
}
