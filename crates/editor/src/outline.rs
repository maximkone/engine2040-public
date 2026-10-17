//! Outliner: filtered rows plus named selection sets over one selection.
//!
//! Owner: the editor session; reads row snapshots, never owns the scene.
//! Units: stable row ids (`u64`), depths, query text. Budget: UI thread
//! only, search linear in rows (<= 50 ms at [`MAX_ROWS`]), a quiet panel frame
//! is O(1) through [`OutlinerView`]. Determinism:
//! same rows plus same query give the same hits in row order. Errors:
//! [`OutlineError`]; refusals stage nothing and mutate nothing. Consumer:
//! the editor outliner panel (hierarchy tab) via pure helpers here.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

use engine_scene::ids::ObjectId;

use crate::commands::bound_to;
use crate::shell::SceneRow;
#[cfg(test)]
use crate::shell::Shell;

// The shared selection lives in the inspector module; the outliner reuses
// it instead of minting a parallel type (one owner per type).
pub use crate::inspector::Selection;
pub use crate::inspector::MAX_SELECTION;

/// Most rows listed in one outliner snapshot: a 64-player plus 500-AI siege
/// scene fits with room to spare.
pub const MAX_ROWS: usize = 2048;
/// Longest search text in chars.
pub const MAX_QUERY_CHARS: usize = 128;
/// Most hits returned by one search; the full outliner fits exactly.
pub const MAX_RESULTS: usize = MAX_ROWS;
/// Most named selection sets kept in one bank.
pub const MAX_SETS: usize = 32;
/// Longest selection-set name in chars.
pub const MAX_SET_NAME_CHARS: usize = 64;
/// Bound for echoed text carried in errors.
pub const MAX_DETAIL_CHARS: usize = 128;

/// What rejected an outliner action. Refusals change nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutlineError {
    /// The search text is over the character bound.
    BadQuery {
        /// Static reason (never echoes unbounded input).
        reason: &'static str,
    },
    /// The snapshot is fuller than the row bound.
    TooManyRows {
        /// Row count refused.
        count: usize,
        /// Bound exceeded.
        max: usize,
    },
    /// The result set is fuller than the result bound.
    TooManyResults {
        /// Hit count refused.
        count: usize,
        /// Bound exceeded.
        max: usize,
    },
    /// No listed row carries this id.
    UnknownId {
        /// The requested stable id.
        id: u64,
    },
    /// The id is not live in the current snapshot; nothing changed.
    StaleHandle {
        /// The refused stable id.
        id: u64,
    },
    /// No selection-set carries this name.
    UnknownSet {
        /// The requested name (bounded).
        name: String,
    },
    /// The set name is empty or over the character cap.
    BadSetName {
        /// Static reason.
        reason: &'static str,
    },
    /// The bank already holds the set bound.
    TooManySets {
        /// Set count refused.
        count: usize,
        /// Bound exceeded.
        max: usize,
    },
    /// An action needs a selected row first.
    EmptySelection,
}

impl fmt::Display for OutlineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadQuery { reason } => write!(f, "search refused ({reason})"),
            Self::TooManyRows { count, max } => {
                write!(f, "outliner holds {count} rows over the {max} bound")
            }
            Self::TooManyResults { count, max } => {
                write!(f, "search found {count} hits over the {max} bound")
            }
            Self::UnknownId { id } => write!(f, "unknown outliner row {id}"),
            Self::StaleHandle { id } => write!(f, "stale outliner row {id}: refresh first"),
            Self::UnknownSet { name } => write!(f, "unknown selection set `{name}`"),
            Self::BadSetName { reason } => write!(f, "selection-set name refused ({reason})"),
            Self::TooManySets { count, max } => {
                write!(f, "bank holds {count} sets over the {max} bound")
            }
            Self::EmptySelection => f.write_str("no row selected"),
        }
    }
}

impl std::error::Error for OutlineError {}

/// Stable row id for a scene node: FNV-1a64 over a domain tag plus the 16
/// raw UUID bytes. Same node maps to the same id on every run; distinct
/// nodes map to distinct ids short of a 64-bit collision, which surfaces
/// as a duplicate row rather than a silent alias.
pub fn stable_id(id: ObjectId) -> u64 {
    let mut hash = engine_core::Fnv64::new();
    hash.write(b"outline:scene:");
    hash.write(id.as_uuid().as_bytes());
    hash.finish()
}

/// One outliner row: plain snapshot data for the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlinerRow {
    /// Stable id derived from the scene node ([`stable_id`]).
    pub id: u64,
    /// Scene node this row selects.
    pub source: ObjectId,
    /// Display name (caller label or shortened id).
    pub name: String,
    /// Depth-first depth, for panel indentation.
    pub depth: u32,
    /// Stable id of the parent row, if the parent is listed.
    pub parent: Option<u64>,
}

impl OutlinerRow {
    /// Short row text for the panel: indented name.
    pub fn label(&self) -> String {
        format!("{}{}", "  ".repeat(self.depth.min(16) as usize), self.name)
    }
}

/// Bound echoed text to [`MAX_DETAIL_CHARS`] on a char boundary.
fn bound_detail(text: &str) -> String {
    bound_to(text, MAX_DETAIL_CHARS)
}

/// Short fallback name for a node without a caller label.
fn short_name(id: ObjectId) -> String {
    id.to_string().chars().take(8).collect()
}

/// Build panel rows from a scene snapshot plus caller labels.
///
/// Row order follows the snapshot order (already deterministic); ids are
/// [`stable_id`] values so viewport picks map onto the same ids the
/// inspector grid uses. Missing labels fall back to the shortened node
/// id; overlong labels truncate deterministically instead of refusing,
/// so the panel never fails to list the scene.
pub fn build_rows(
    rows: &[SceneRow],
    names: &HashMap<ObjectId, String>,
) -> Result<Vec<OutlinerRow>, OutlineError> {
    if rows.len() > MAX_ROWS {
        return Err(OutlineError::TooManyRows {
            count: rows.len(),
            max: MAX_ROWS,
        });
    }
    let mut seen: BTreeSet<u64> = BTreeSet::new();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id = stable_id(row.id);
        if !seen.insert(id) {
            return Err(OutlineError::TooManyRows {
                count: rows.len(),
                max: MAX_ROWS,
            });
        }
        // The fallback id text is built only for rows without a label.
        let name = match names.get(&row.id) {
            Some(label) if !label.is_empty() => bound_detail(label),
            _ => bound_detail(&short_name(row.id)),
        };
        out.push(OutlinerRow {
            id,
            source: row.id,
            name,
            depth: row.depth,
            parent: if row.parent.is_nil() {
                None
            } else {
                Some(stable_id(row.parent))
            },
        });
    }
    Ok(out)
}

/// Live stable ids of the current snapshot, for membership checks.
pub fn live_set(rows: &[OutlinerRow]) -> BTreeSet<u64> {
    rows.iter().map(|row| row.id).collect()
}

/// Shared handles to one revision's outliner rows and their live id set.
pub type OutlinerHandles = (Arc<[OutlinerRow]>, Arc<BTreeSet<u64>>);

/// Outliner rows, live ids and the last search, cached per document revision.
///
/// The document revision grows by one per apply, undo and redo, so it names
/// the row set exactly. A quiet frame (same revision, same query) rebuilds
/// nothing and clones two `Arc` handles; only an edit rebuilds the rows, and
/// only a new query or new rows re-run the search. Handles are `Arc` so a
/// panel keeps drawing from one while it stages edits on the shell.
#[derive(Clone, Debug)]
pub struct OutlinerView {
    revision: Option<u64>,
    rows: Arc<[OutlinerRow]>,
    live: Arc<BTreeSet<u64>>,
    hits: Option<(String, Arc<[OutlinerHit]>)>,
}

impl Default for OutlinerView {
    fn default() -> Self {
        Self::new()
    }
}

impl OutlinerView {
    /// Empty view: the first [`sync`](Self::sync) builds the rows.
    pub fn new() -> Self {
        Self {
            revision: None,
            rows: Arc::from(Vec::new()),
            live: Arc::new(BTreeSet::new()),
            hits: None,
        }
    }

    /// Rows and live ids for document `revision`. `build` runs only when the
    /// revision moved since the last call; a failed build leaves the view
    /// empty so no stale rows hide behind a new revision.
    pub fn sync(
        &mut self,
        revision: u64,
        build: impl FnOnce() -> Result<Vec<OutlinerRow>, OutlineError>,
    ) -> Result<OutlinerHandles, OutlineError> {
        if self.revision != Some(revision) {
            *self = Self::new();
            let rows = build()?;
            self.live = Arc::new(live_set(&rows));
            self.rows = Arc::from(rows);
            self.revision = Some(revision);
        }
        Ok((Arc::clone(&self.rows), Arc::clone(&self.live)))
    }

    /// Hits of `query` over the synced rows, searched again only when the
    /// text or the rows changed.
    pub fn search(&mut self, query: &str) -> Result<Arc<[OutlinerHit]>, OutlineError> {
        if let Some((text, hits)) = &self.hits {
            if text == query {
                return Ok(Arc::clone(hits));
            }
        }
        let hits: Arc<[OutlinerHit]> = Arc::from(search(&self.rows, &OutlinerQuery::new(query))?);
        self.hits = Some((query.to_string(), Arc::clone(&hits)));
        Ok(hits)
    }
}

/// One search query: substring over row names; empty matches every row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlinerQuery {
    /// Substring over the row name; empty matches every name.
    pub text: String,
}

impl OutlinerQuery {
    /// Build one query without validation; [`search`] gates entry.
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
        }
    }
}

/// One search hit: the stable id plus the row name, in row order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlinerHit {
    /// Stable row id.
    pub id: u64,
    /// Row name.
    pub name: String,
    /// Row depth.
    pub depth: u32,
}

impl OutlinerHit {
    /// Row text for the panel: indented name.
    pub fn label(&self) -> String {
        format!("{}{}", "  ".repeat(self.depth.min(16) as usize), self.name)
    }
}

/// Search the outliner: substring over names, hits in row order.
///
/// Same rows plus same query always yield the same hits. Empty text
/// matches every row.
pub fn search(
    rows: &[OutlinerRow],
    query: &OutlinerQuery,
) -> Result<Vec<OutlinerHit>, OutlineError> {
    if query.text.chars().count() > MAX_QUERY_CHARS {
        return Err(OutlineError::BadQuery {
            reason: "search text exceeds the 128-character bound",
        });
    }
    let needle = query.text.to_lowercase();
    let mut hits = Vec::new();
    for row in rows {
        if !needle.is_empty() && !row.name.to_lowercase().contains(&needle) {
            continue;
        }
        hits.push(OutlinerHit {
            id: row.id,
            name: row.name.clone(),
            depth: row.depth,
        });
        if hits.len() > MAX_RESULTS {
            return Err(OutlineError::TooManyResults {
                count: hits.len(),
                max: MAX_RESULTS,
            });
        }
    }
    Ok(hits)
}

/// Single pick: replaces the selection, focusing the picked row.
pub fn select_one(id: u64) -> Selection {
    Selection {
        ids: vec![id],
        primary: Some(id),
    }
}

/// Add one live row to the selection (dedupes). Refuses stale ids and
/// the selection bound without mutating the input on error.
pub fn select_add(
    selection: &Selection,
    id: u64,
    live: &BTreeSet<u64>,
) -> Result<Selection, OutlineError> {
    if !live.contains(&id) {
        return Err(OutlineError::StaleHandle { id });
    }
    if selection.ids.contains(&id) {
        return Ok(selection.clone());
    }
    if selection.ids.len() >= MAX_SELECTION {
        return Err(OutlineError::BadQuery {
            reason: "selection exceeds the node bound",
        });
    }
    let mut next = selection.clone();
    next.ids.push(id);
    if next.primary.is_none() {
        next.primary = Some(id);
    }
    Ok(next)
}

/// Remove one row from the selection, repairing the focus when it leaves.
/// Removing an absent id is a no-op.
pub fn select_remove(selection: &Selection, id: u64) -> Selection {
    let mut next = selection.clone();
    next.ids.retain(|known| *known != id);
    if next.primary == Some(id) {
        next.primary = next.ids.first().copied();
    }
    next
}

/// Toggle one live row: present rows leave, absent rows join. Refuses
/// stale ids without mutating the input on error.
pub fn select_toggle(
    selection: &Selection,
    id: u64,
    live: &BTreeSet<u64>,
) -> Result<Selection, OutlineError> {
    if !live.contains(&id) {
        return Err(OutlineError::StaleHandle { id });
    }
    if selection.ids.contains(&id) {
        Ok(select_remove(selection, id))
    } else {
        select_add(selection, id, live)
    }
}

/// Add the row-order span between `anchor` and `target` (both inclusive)
/// to the selection, for shift-click. Refuses unknown endpoints and the
/// selection bound without mutating the input on error.
pub fn select_range(
    selection: &Selection,
    anchor: u64,
    target: u64,
    order: &[u64],
    live: &BTreeSet<u64>,
) -> Result<Selection, OutlineError> {
    let lo = order
        .iter()
        .position(|id| *id == anchor)
        .ok_or(OutlineError::UnknownId { id: anchor })?;
    let hi = order
        .iter()
        .position(|id| *id == target)
        .ok_or(OutlineError::UnknownId { id: target })?;
    let (from, to) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    let mut next = selection.clone();
    for id in &order[from..=to] {
        if !live.contains(id) {
            return Err(OutlineError::StaleHandle { id: *id });
        }
        if !next.ids.contains(id) {
            if next.ids.len() >= MAX_SELECTION {
                return Err(OutlineError::BadQuery {
                    reason: "selection exceeds the node bound",
                });
            }
            next.ids.push(*id);
        }
    }
    if next.primary.is_none() {
        next.primary = next.ids.first().copied();
    }
    Ok(next)
}

/// Focus one selected row. Refuses rows outside the selection.
pub fn set_primary(selection: &Selection, id: u64) -> Result<Selection, OutlineError> {
    if !selection.ids.contains(&id) {
        return Err(OutlineError::UnknownId { id });
    }
    let mut next = selection.clone();
    next.primary = Some(id);
    Ok(next)
}

/// Drop ids that left the snapshot; repairs the focus to the first kept
/// row when it leaves. Returns true when the selection changed.
pub fn prune(selection: &mut Selection, live: &BTreeSet<u64>) -> bool {
    let before = selection.ids.len();
    let focus = selection.primary;
    selection.ids.retain(|id| live.contains(id));
    if let Some(primary) = selection.primary {
        if !selection.ids.contains(&primary) {
            selection.primary = selection.ids.first().copied();
        }
    }
    before != selection.ids.len() || focus != selection.primary
}

/// Union of two selections: every id of either, focus prefers `a`.
pub fn union(a: &Selection, b: &Selection) -> Result<Selection, OutlineError> {
    let mut ids = a.ids.clone();
    for id in &b.ids {
        if !ids.contains(id) {
            if ids.len() >= MAX_SELECTION {
                return Err(OutlineError::BadQuery {
                    reason: "selection exceeds the node bound",
                });
            }
            ids.push(*id);
        }
    }
    let primary = a
        .primary
        .or(b.primary)
        .or_else(|| ids.first().copied())
        .filter(|focus| ids.contains(focus));
    Ok(Selection { ids, primary })
}

/// Intersection of two selections: ids of both, focus kept when shared.
pub fn intersect(a: &Selection, b: &Selection) -> Selection {
    let ids: Vec<u64> = a
        .ids
        .iter()
        .filter(|id| b.ids.contains(id))
        .copied()
        .collect();
    let primary = a
        .primary
        .filter(|focus| ids.contains(focus))
        .or_else(|| ids.first().copied());
    Selection { ids, primary }
}

/// Difference of two selections: ids of `a` missing from `b`.
pub fn difference(a: &Selection, b: &Selection) -> Selection {
    let ids: Vec<u64> = a
        .ids
        .iter()
        .filter(|id| !b.ids.contains(id))
        .copied()
        .collect();
    let primary = a
        .primary
        .filter(|focus| ids.contains(focus))
        .or_else(|| ids.first().copied());
    Selection { ids, primary }
}

/// Route one viewport pick into the shared selection.
///
/// `None` clears a replacing click and keeps an additive one. `Some(id)`
/// maps through [`stable_id`]: stale picks refuse without mutating the
/// input; additive picks toggle the row while replacing picks focus it
/// alone. Toggle wins multi-select conflicts deterministically.
pub fn apply_pick(
    selection: &Selection,
    pick: Option<ObjectId>,
    additive: bool,
    live: &BTreeSet<u64>,
) -> Result<Selection, OutlineError> {
    match pick {
        None => {
            if additive {
                Ok(selection.clone())
            } else {
                Ok(Selection {
                    ids: Vec::new(),
                    primary: None,
                })
            }
        }
        Some(node) => {
            let id = stable_id(node);
            if !live.contains(&id) {
                return Err(OutlineError::StaleHandle { id });
            }
            if additive {
                select_toggle(selection, id, live)
            } else {
                Ok(select_one(id))
            }
        }
    }
}

/// How a recalled set merges into the current selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecallMode {
    /// The set becomes the selection.
    Replace,
    /// Every set row joins the selection.
    Union,
    /// Only rows in both stay selected.
    Intersect,
    /// Set rows leave the selection.
    Subtract,
}

/// Named selection sets over the shared [`Selection`].
///
/// The bank owns no scene state: callers pass live snapshots for
/// membership, and [`prune`] drops stale rows before saving or after
/// loading. Insertion order never matters: names sort as text.
#[derive(Clone, Debug, Default)]
pub struct SelectionBank {
    sets: BTreeMap<String, Selection>,
}

impl SelectionBank {
    /// Empty bank: no named sets.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set count.
    pub fn len(&self) -> usize {
        self.sets.len()
    }

    /// True when no set is stored.
    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }

    /// Stored set names in text order.
    pub fn names(&self) -> Vec<&str> {
        self.sets.keys().map(String::as_str).collect()
    }

    /// One stored set by name.
    pub fn load(&self, name: &str) -> Result<&Selection, OutlineError> {
        self.sets.get(name).ok_or_else(|| OutlineError::UnknownSet {
            name: bound_detail(name),
        })
    }

    /// Store the selection under `name`, replacing any older set there.
    /// Prunes stale rows first so banks never keep dead handles.
    pub fn save(
        &mut self,
        name: &str,
        mut selection: Selection,
        live: &BTreeSet<u64>,
    ) -> Result<(), OutlineError> {
        validate_set_name(name)?;
        prune(&mut selection, live);
        if !self.sets.contains_key(name) && self.sets.len() >= MAX_SETS {
            return Err(OutlineError::TooManySets {
                count: self.sets.len() + 1,
                max: MAX_SETS,
            });
        }
        self.sets.insert(name.to_string(), selection);
        Ok(())
    }

    /// Drop one stored set. True when one was stored.
    pub fn remove(&mut self, name: &str) -> bool {
        self.sets.remove(name).is_some()
    }

    /// Merge one stored set into the current selection.
    pub fn recall(&self, name: &str, current: &Selection) -> Result<Selection, OutlineError> {
        self.recall_mode(name, current, RecallMode::Replace)
    }

    /// Merge one stored set into the current selection under `mode`.
    pub fn recall_mode(
        &self,
        name: &str,
        current: &Selection,
        mode: RecallMode,
    ) -> Result<Selection, OutlineError> {
        let stored = self.load(name)?;
        match mode {
            RecallMode::Replace => Ok(stored.clone()),
            RecallMode::Union => union(current, stored),
            RecallMode::Intersect => Ok(intersect(current, stored)),
            RecallMode::Subtract => Ok(difference(current, stored)),
        }
    }
}

/// Validate one selection-set name ahead of storing.
fn validate_set_name(name: &str) -> Result<(), OutlineError> {
    if name.is_empty() {
        return Err(OutlineError::BadSetName { reason: "empty" });
    }
    if name.chars().count() > MAX_SET_NAME_CHARS {
        return Err(OutlineError::BadSetName {
            reason: "over-64-characters",
        });
    }
    Ok(())
}

/// True when display text looks like a scene path string.
///
/// Path strings join names with separators (`/` or `\`) or name relative
/// steps (`.` or `..`). They break when any segment renames or moves, so
/// panels must never store them as references: handles are [`stable_id`]
/// values over the persistent node id, which rename and move leave intact.
/// This lint names the trap; [`lint_names`] reports every occurrence.
pub fn is_path_string(text: &str) -> bool {
    text.contains('/') || text.contains('\\') || text == "." || text == ".."
}

/// One path-string lint finding: the node whose display label must stay
/// display-only, plus the offending text bounded to [`MAX_DETAIL_CHARS`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathLint {
    /// Node carrying the path-shaped label.
    pub source: ObjectId,
    /// Offending label text, bounded for error display.
    pub text: String,
}

/// Scan display labels for path-string traps, in node-text order.
///
/// Labels are display-only: a flagged label still lists (see [`build_rows`])
/// but no caller may resolve it as a reference. Empty findings mean the
/// panel stores handles only. Determinism: same map gives the same findings
/// in the same order; no clock, no iteration over hash order.
pub fn lint_names(names: &HashMap<ObjectId, String>) -> Vec<PathLint> {
    let mut findings: Vec<PathLint> = names
        .iter()
        .filter(|(_, label)| is_path_string(label))
        .map(|(source, label)| PathLint {
            source: *source,
            text: bound_detail(label),
        })
        .collect();
    findings.sort_by_key(|finding| finding.source);
    findings
}

/// Count selection ids missing from the live snapshot.
///
/// Zero means every referenced row still lists; any other count names
/// dangling references the caller must prune before use.
pub fn dangling_count(selection: &Selection, live: &BTreeSet<u64>) -> usize {
    selection.ids.iter().filter(|id| !live.contains(id)).count()
}

/// Remap a selection through a copy map, dropping unmapped ids.
///
/// Copying a subtree mints fresh node ids; the caller passes the old to new
/// [`stable_id`] map and the selection follows it. Rename and move need no
/// map: ids persist, so the selection passes through untouched. The primary
/// follows its mapping when kept, else falls back to the first kept row.
/// Returns the remapped selection plus the count of dropped ids.
pub fn remap_selection(selection: &Selection, map: &BTreeMap<u64, u64>) -> (Selection, usize) {
    let mut ids: Vec<u64> = Vec::with_capacity(selection.ids.len());
    let mut dropped: usize = 0;
    for id in &selection.ids {
        match map.get(id) {
            Some(mapped) => {
                if !ids.contains(mapped) {
                    ids.push(*mapped);
                }
            }
            None => {
                dropped = dropped.saturating_add(1);
            }
        }
    }
    let primary = selection
        .primary
        .and_then(|focus| map.get(&focus).copied())
        .filter(|focus| ids.contains(focus))
        .or_else(|| ids.first().copied());
    (Selection { ids, primary }, dropped)
}

/// Rows in the siege scale fixture.
#[cfg(test)]
pub(crate) const SIEGE_ROWS: usize = 2000;

/// Siege-shaped scene for the scale proofs: an arena root, a players group
/// of 64, an AI group of 500 in five squads of 100, and props filling the
/// scene up to [`SIEGE_ROWS`]. Returns the shell plus a name for every node.
#[cfg(test)]
pub(crate) fn siege_scene() -> (Shell, HashMap<ObjectId, String>) {
    use crate::commands::Op;
    use engine_scene::transform::Transform;

    let mut ops = Vec::with_capacity(SIEGE_ROWS);
    let mut names = HashMap::new();
    let mut add = |name: String, parent: ObjectId| {
        let id = ObjectId::new();
        ops.push(Op::Insert {
            id,
            parent,
            local: Transform::identity(),
        });
        names.insert(id, name);
        id
    };
    let arena = add("arena".to_string(), ObjectId::NIL);
    let players = add("players".to_string(), arena);
    for index in 0..64 {
        add(format!("player {index:02}"), players);
    }
    let ai = add("ai".to_string(), arena);
    for squad in 0..5 {
        let group = add(format!("squad {squad}"), ai);
        for member in 0..100 {
            add(format!("ai {:03}", squad * 100 + member), group);
        }
    }
    let props = add("props".to_string(), arena);
    // Arena, players group, 64 players, ai group, 5 squads, 500 AI, props group.
    let placed = 1 + 1 + 64 + 1 + 5 + 500 + 1;
    for prop in 0..SIEGE_ROWS - placed {
        add(format!("prop {prop:04}"), props);
    }
    let mut shell = Shell::new();
    shell.apply_ops(ops).expect("siege scene applies");
    (shell, names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::TypeId;
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    use engine_scene::transform::Transform;

    use crate::inspector::{self, NodeView, VersionedComponent};
    use crate::shell::Shell;

    fn pose_at(x: f32) -> Transform {
        Transform::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    fn seeded_shell() -> (Shell, ObjectId, ObjectId) {
        let mut shell = Shell::new();
        let root = ObjectId::new();
        let child = ObjectId::new();
        shell
            .insert_node(root, ObjectId::NIL, pose_at(0.0))
            .unwrap();
        shell.insert_node(child, root, pose_at(1.0)).unwrap();
        (shell, root, child)
    }

    fn outline_two() -> (Shell, ObjectId, ObjectId, Vec<OutlinerRow>, BTreeSet<u64>) {
        let (shell, root, child) = seeded_shell();
        let mut names = HashMap::new();
        names.insert(root, "hall".to_string());
        names.insert(child, "hall/plinth".to_string());
        let rows = build_rows(&shell.rows(), &names).unwrap();
        assert_eq!(rows.len(), 2);
        let live = live_set(&rows);
        (shell, root, child, rows, live)
    }

    fn order_of(rows: &[OutlinerRow]) -> Vec<u64> {
        rows.iter().map(|row| row.id).collect()
    }

    #[test]
    fn stable_ids_are_deterministic_per_node() {
        let (shell, root, child, rows, _) = outline_two();
        assert_eq!(rows.len(), shell.rows().len());
        assert_ne!(stable_id(root), stable_id(child));
        assert_eq!(stable_id(root), stable_id(root));
        // Parent links survive the mapping: the child points at the root.
        let root_row = rows.iter().find(|row| row.source == root).unwrap();
        let child_row = rows.iter().find(|row| row.source == child).unwrap();
        assert_eq!(child_row.parent, Some(root_row.id));
        assert_eq!(root_row.parent, None);
    }

    #[test]
    fn search_matches_substrings_in_row_order() {
        let (_, _, _, rows, _) = outline_two();
        let all = search(&rows, &OutlinerQuery::new("")).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(
            all.iter().map(|hit| hit.id).collect::<Vec<_>>(),
            order_of(&rows)
        );
        let one = search(&rows, &OutlinerQuery::new("PLINTH")).unwrap();
        assert_eq!(one.len(), 1);
        assert!(one[0].name.contains("plinth"));
        // Same inputs twice give the same hits.
        assert_eq!(search(&rows, &OutlinerQuery::new("hall")).unwrap().len(), 2);
        assert_eq!(
            search(&rows, &OutlinerQuery::new("hall")).unwrap(),
            search(&rows, &OutlinerQuery::new("hall")).unwrap()
        );
    }

    #[test]
    fn overlong_search_refuses_and_stages_nothing() {
        let (_, _, _, rows, _) = outline_two();
        let long = "x".repeat(MAX_QUERY_CHARS + 1);
        assert!(matches!(
            search(&rows, &OutlinerQuery::new(&long)).unwrap_err(),
            OutlineError::BadQuery { .. }
        ));
    }

    #[test]
    fn toggle_add_remove_and_range_cover_multiselect() {
        let (_, _, _, rows, live) = outline_two();
        let order = order_of(&rows);
        let empty = Selection::new();
        // Toggle joins, toggling again leaves.
        let one = select_toggle(&empty, order[0], &live).unwrap();
        assert_eq!(one.primary, Some(order[0]));
        let two = select_toggle(&one, order[1], &live).unwrap();
        assert_eq!(two.ids.len(), 2);
        let back = select_toggle(&two, order[1], &live).unwrap();
        assert_eq!(back, one);
        // Shift-click spans the row order.
        let spanned = select_range(&empty, order[0], order[1], &order, &live).unwrap();
        assert_eq!(spanned.ids.len(), 2);
        // Focus moves only inside the selection.
        let focused = set_primary(&spanned, order[1]).unwrap();
        assert_eq!(focused.primary, Some(order[1]));
        assert!(matches!(
            set_primary(&spanned, 0xdead).unwrap_err(),
            OutlineError::UnknownId { .. }
        ));
    }

    #[test]
    fn stale_handles_refuse_then_prune_repairs_focus() {
        let (_, _, _, rows, live) = outline_two();
        let order = order_of(&rows);
        let ghost = 0x9e37_79b9_7f4a_7c15;
        assert!(!live.contains(&ghost));
        let empty = Selection::new();
        assert_eq!(
            select_toggle(&empty, ghost, &live).unwrap_err(),
            OutlineError::StaleHandle { id: ghost }
        );
        assert!(matches!(
            apply_pick(&empty, Some(ObjectId::new()), false, &live).unwrap_err(),
            OutlineError::StaleHandle { .. }
        ));
        // A selection holding a removed row prunes it and repairs focus.
        let mut stale = Selection {
            ids: vec![order[0], ghost],
            primary: Some(ghost),
        };
        assert!(prune(&mut stale, &live));
        assert_eq!(stale.ids, vec![order[0]]);
        assert_eq!(stale.primary, Some(order[0]));
        assert!(!prune(&mut stale, &live));
        let _ = rows;
    }

    #[test]
    fn set_algebra_matches_hand_computed_membership() {
        let (_, _, _, rows, _) = outline_two();
        let order = order_of(&rows);
        let a = Selection {
            ids: vec![order[0]],
            primary: Some(order[0]),
        };
        let b = Selection {
            ids: vec![order[1]],
            primary: Some(order[1]),
        };
        let both = union(&a, &b).unwrap();
        assert_eq!(both.ids.len(), 2);
        assert_eq!(both.primary, Some(order[0]));
        assert!(intersect(&a, &b).ids.is_empty());
        assert_eq!(intersect(&both, &b).ids, vec![order[1]]);
        assert_eq!(difference(&both, &b), a);
    }

    #[test]
    fn bank_saves_recalls_and_names_its_failures() {
        let (_, _, _, rows, live) = outline_two();
        let order = order_of(&rows);
        let mut bank = SelectionBank::new();
        assert!(bank.is_empty());
        bank.save("pair", select_one(order[0]), &live).unwrap();
        // Saving prunes stale rows instead of keeping dead handles.
        bank.save(
            "stale",
            Selection {
                ids: vec![order[0], 0xabcd],
                primary: Some(0xabcd),
            },
            &live,
        )
        .unwrap();
        assert_eq!(bank.load("stale").unwrap().ids, vec![order[0]]);
        let current = select_one(order[1]);
        let merged = bank
            .recall_mode("pair", &current, RecallMode::Union)
            .unwrap();
        assert_eq!(merged.ids.len(), 2);
        let kept = bank
            .recall_mode("pair", &merged, RecallMode::Intersect)
            .unwrap();
        assert_eq!(kept.ids, vec![order[0]]);
        let cut = bank
            .recall_mode("pair", &merged, RecallMode::Subtract)
            .unwrap();
        assert_eq!(cut.ids, vec![order[1]]);
        assert_eq!(bank.names(), vec!["pair", "stale"]);
        assert!(matches!(
            bank.load("missing").unwrap_err(),
            OutlineError::UnknownSet { .. }
        ));
        assert!(matches!(
            bank.save("", current.clone(), &live).unwrap_err(),
            OutlineError::BadSetName { .. }
        ));
        assert!(bank.remove("pair"));
        assert!(!bank.remove("pair"));
        let _ = rows;
    }

    #[test]
    fn bank_cap_holds_and_replace_never_grows_past_it() {
        let (_, _, _, _, live) = outline_two();
        let mut bank = SelectionBank::new();
        for i in 0..MAX_SETS {
            bank.save(&format!("set-{i}"), Selection::new(), &live)
                .unwrap();
        }
        assert_eq!(bank.len(), MAX_SETS);
        assert!(matches!(
            bank.save("overflow", Selection::new(), &live).unwrap_err(),
            OutlineError::TooManySets { .. }
        ));
        // Replacing a stored name under a full bank still fits.
        bank.save("set-0", Selection::new(), &live).unwrap();
        assert_eq!(bank.len(), MAX_SETS);
    }

    #[test]
    fn picks_replace_or_toggle_and_misses_clear_only_alone() {
        let (shell, root, _, rows, live) = outline_two();
        let root_id = stable_id(root);
        let empty = Selection::new();
        // Replacing click focuses the pick alone.
        let one = apply_pick(&empty, Some(root), false, &live).unwrap();
        assert_eq!(one, select_one(root_id));
        // Additive click on the same row toggles it off (conflict resolved).
        assert!(apply_pick(&one, Some(root), true, &live)
            .unwrap()
            .ids
            .is_empty());
        // A miss clears a replacing click but keeps an additive one.
        assert!(apply_pick(&one, None, false, &live).unwrap().ids.is_empty());
        assert_eq!(apply_pick(&one, None, true, &live).unwrap(), one);
        let _ = (shell, rows);
    }

    #[test]
    fn viewport_outliner_inspector_agree_on_one_selection_type() {
        // One owner per type: the outliner reuses the inspector selection.
        assert_eq!(
            TypeId::of::<Selection>(),
            TypeId::of::<inspector::Selection>()
        );
        let (shell, root, child) = seeded_shell();
        // Viewport picks the root at the panel center on the live rows.
        let view = shell.viewport;
        let hit = shell.pick(view.size.width / 2.0, view.size.height / 2.0);
        assert_eq!(hit, Some(root));
        // Outliner maps the same rows onto stable ids the pick joins.
        let mut names = HashMap::new();
        names.insert(root, "hall".to_string());
        names.insert(child, "hall/plinth".to_string());
        let rows = build_rows(&shell.rows(), &names).unwrap();
        let live = live_set(&rows);
        let mut selection = apply_pick(&Selection::new(), hit, false, &live).unwrap();
        assert_eq!(selection.primary, Some(stable_id(root)));
        // The inspector grids the same selection object over matching views.
        let views = vec![
            NodeView {
                id: stable_id(root),
                key: Some("hall".into()),
                parent_key: None,
                pose: inspector::LocalPose::IDENTITY,
                components: vec![VersionedComponent::new("label", 1, "Grand Hall")],
                schema: vec![inspector::SchemaStep {
                    component: "label",
                    version: 1,
                    is_current: true,
                }],
                museum: false,
            },
            NodeView {
                id: stable_id(child),
                key: Some("hall/plinth".into()),
                parent_key: Some("hall".into()),
                pose: inspector::LocalPose::IDENTITY,
                components: vec![VersionedComponent::new("label", 1, "Plinth")],
                schema: vec![inspector::SchemaStep {
                    component: "label",
                    version: 1,
                    is_current: true,
                }],
                museum: false,
            },
        ];
        let grid = inspector::build_grid(&views, &selection, &BTreeMap::new()).unwrap();
        assert!(grid.rows().iter().any(|row| row.node == stable_id(root)));
        // An outliner search hit joins the same object: all three agree.
        let hits = search(&rows, &OutlinerQuery::new("plinth")).unwrap();
        assert_eq!(hits.len(), 1);
        selection = select_toggle(&selection, hits[0].id, &live).unwrap();
        assert_eq!(selection.ids.len(), 2);
        let grid = inspector::build_grid(&views, &selection, &BTreeMap::new()).unwrap();
        assert!(grid.rows().iter().any(|row| row.node == stable_id(child)));
        // The viewport still picks the same root under the shared ids.
        assert_eq!(stable_id(hit.unwrap()), selection.ids[0]);
    }

    #[test]
    fn empty_names_fall_back_and_long_names_truncate() {
        let (shell, _, _) = seeded_shell();
        let rows = build_rows(&shell.rows(), &HashMap::new()).unwrap();
        assert!(rows.iter().all(|row| !row.name.is_empty()));
        let mut names = HashMap::new();
        for row in shell.rows() {
            names.insert(row.id, "n".repeat(MAX_DETAIL_CHARS + 40));
        }
        let capped = build_rows(&shell.rows(), &names).unwrap();
        assert!(capped.iter().all(|row| row.name.len() <= MAX_DETAIL_CHARS));
    }

    #[test]
    fn too_many_rows_refuse() {
        let (shell, _, _) = seeded_shell();
        let rows = shell.rows();
        let many = vec![rows[0]; MAX_ROWS + 1];
        assert!(matches!(
            build_rows(&many, &HashMap::new()).unwrap_err(),
            OutlineError::TooManyRows { .. }
        ));
    }

    #[test]
    fn path_strings_flag_but_never_become_handles() {
        assert!(is_path_string("hall/plinth"));
        assert!(is_path_string("a/b/c"));
        assert!(is_path_string("hall\\plinth"));
        assert!(is_path_string("."));
        assert!(is_path_string(".."));
        assert!(!is_path_string(""));
        assert!(!is_path_string("hall"));
        assert!(!is_path_string("plinth-1"));
        assert!(!is_path_string("hall plinth"));

        let first = ObjectId::parse("00000000-0000-4000-8000-000000000021")
            .expect("fixed fixture id parses");
        let second = ObjectId::parse("00000000-0000-4000-8000-000000000022")
            .expect("fixed fixture id parses");
        let third = ObjectId::parse("00000000-0000-4000-8000-000000000023")
            .expect("fixed fixture id parses");
        let mut names: HashMap<ObjectId, String> = HashMap::new();
        names.insert(first, "hall".to_string());
        names.insert(second, "hall/plinth".to_string());
        names.insert(third, "..".to_string());
        let findings = lint_names(&names);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings, lint_names(&names));
        assert!(findings.iter().all(|finding| !finding.text.is_empty()));
        assert!(findings
            .iter()
            .all(|finding| finding.text.len() <= MAX_DETAIL_CHARS));

        // Flagged labels still list as display-only with unchanged handles.
        let mut shell = Shell::new();
        shell
            .insert_node(first, ObjectId::NIL, pose_at(0.0))
            .expect("fixed tree inserts");
        shell
            .insert_node(second, first, pose_at(1.0))
            .expect("fixed tree inserts");
        let rows = build_rows(&shell.rows(), &names).expect("flagged labels list");
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert_eq!(row.id, stable_id(row.source));
        }
        let live = live_set(&rows);
        assert!(live.contains(&stable_id(first)));
        assert!(live.contains(&stable_id(second)));
    }

    #[test]
    fn remap_follows_copies_and_drops_dangling() {
        let first = stable_id(
            ObjectId::parse("00000000-0000-4000-8000-000000000031")
                .expect("fixed fixture id parses"),
        );
        let second = stable_id(
            ObjectId::parse("00000000-0000-4000-8000-000000000032")
                .expect("fixed fixture id parses"),
        );
        let ghost: u64 = 0x9e37_79b9_7f4a_7c15;
        let selection = Selection {
            ids: vec![first, second],
            primary: Some(second),
        };
        let live: BTreeSet<u64> = BTreeSet::from([first, second]);
        assert_eq!(dangling_count(&selection, &live), 0);
        let stale = Selection {
            ids: vec![first, ghost],
            primary: Some(ghost),
        };
        assert_eq!(dangling_count(&stale, &live), 1);

        let replacement = first.wrapping_add(1);
        let map: BTreeMap<u64, u64> = BTreeMap::from([(first, replacement)]);
        let (remapped, dropped) = remap_selection(&stale, &map);
        assert_eq!(dropped, 1);
        assert_eq!(remapped.ids, vec![replacement]);
        assert_eq!(remapped.primary, Some(replacement));

        let empty_map: BTreeMap<u64, u64> = BTreeMap::new();
        let (cleared, dropped_all) = remap_selection(&selection, &empty_map);
        assert_eq!(dropped_all, 2);
        assert!(cleared.ids.is_empty());
        assert_eq!(cleared.primary, None);
    }

    #[test]
    fn handles_survive_seeded_rename_and_move_ops() {
        let seed: u32 = 51;
        let root = ObjectId::parse("00000000-0000-4000-8000-000000000041")
            .expect("fixed fixture id parses");
        let node_a = ObjectId::parse("00000000-0000-4000-8000-000000000042")
            .expect("fixed fixture id parses");
        let node_b = ObjectId::parse("00000000-0000-4000-8000-000000000043")
            .expect("fixed fixture id parses");
        let node_c = ObjectId::parse("00000000-0000-4000-8000-000000000044")
            .expect("fixed fixture id parses");
        let nodes = [root, node_a, node_b, node_c];
        let pinned: Vec<u64> = nodes.iter().map(|id| stable_id(*id)).collect();

        let mut shell = Shell::new();
        shell
            .insert_node(root, ObjectId::NIL, pose_at(0.0))
            .expect("fixed tree inserts");
        shell
            .insert_node(node_a, root, pose_at(1.0))
            .expect("fixed tree inserts");
        shell
            .insert_node(node_b, root, pose_at(2.0))
            .expect("fixed tree inserts");
        shell
            .insert_node(node_c, node_a, pose_at(3.0))
            .expect("fixed tree inserts");

        let mut names: HashMap<ObjectId, String> = HashMap::new();
        names.insert(root, "root".to_string());
        names.insert(node_a, "alpha".to_string());
        names.insert(node_b, "beta".to_string());
        names.insert(node_c, "gamma".to_string());
        let selection = Selection {
            ids: vec![stable_id(node_a), stable_id(node_c)],
            primary: Some(stable_id(node_c)),
        };

        for step in 0..20 {
            let target = nodes[step % nodes.len()];
            let label = format!("label-{seed}-{step}");
            names.insert(target, label);
            match step % 4 {
                0 => {
                    shell
                        .reparent_node(node_c, root)
                        .expect("leaf move stays acyclic");
                }
                1 => {
                    shell
                        .reparent_node(node_c, node_a)
                        .expect("leaf move stays acyclic");
                }
                2 => {
                    shell
                        .reparent_node(node_b, node_a)
                        .expect("leaf move stays acyclic");
                }
                _ => {
                    shell
                        .reparent_node(node_b, root)
                        .expect("leaf move stays acyclic");
                }
            }
            let rows = build_rows(&shell.rows(), &names).expect("renames list");
            assert_eq!(rows.len(), nodes.len());
            for (node, pinned_id) in nodes.iter().zip(pinned.iter()) {
                assert_eq!(stable_id(*node), *pinned_id);
                let row = rows
                    .iter()
                    .find(|row| row.source == *node)
                    .expect("every node still lists");
                assert_eq!(row.id, *pinned_id);
            }
            let live = live_set(&rows);
            assert_eq!(dangling_count(&selection, &live), 0);
            let mut scrubbed = selection.clone();
            assert!(!prune(&mut scrubbed, &live));
            assert_eq!(scrubbed, selection);
            assert!(lint_names(&names).is_empty());
            assert_eq!(live.len(), nodes.len());
        }

        let rows = build_rows(&shell.rows(), &names).expect("final rows list");
        let live = live_set(&rows);
        let hits = search(&rows, &OutlinerQuery::new(&format!("label-{seed}-19")))
            .expect("renamed label searches");
        assert_eq!(hits.len(), 1);
        assert!(live.contains(&hits[0].id));
        assert!(selection.ids.iter().all(|id| live.contains(id)));
    }

    /// Median of `runs` timings of `work`.
    fn median_of(runs: usize, mut work: impl FnMut()) -> Duration {
        let mut times: Vec<Duration> = (0..runs)
            .map(|_| {
                let start = Instant::now();
                work();
                start.elapsed()
            })
            .collect();
        times.sort();
        times[runs / 2]
    }

    #[test]
    fn siege_scene_search_stays_under_50_ms() {
        let (shell, names) = siege_scene();
        let rows = build_rows(&shell.rows(), &names).unwrap();
        assert_eq!(rows.len(), SIEGE_ROWS);
        let count = |text: &str| search(&rows, &OutlinerQuery::new(text)).unwrap().len();
        assert_eq!(count("ai 4"), 100);
        assert_eq!(count("PLAYER"), 65, "64 players plus their group");
        assert_eq!(count(""), SIEGE_ROWS);
        assert_eq!(count("no such row"), 0);
        let mut worst = Duration::ZERO;
        for text in ["", "ai 4", "PROP 1", "squad"] {
            let start = Instant::now();
            search(&rows, &OutlinerQuery::new(text)).unwrap();
            worst = worst.max(start.elapsed());
        }
        eprintln!("ED19 siege: {SIEGE_ROWS} rows, worst search {worst:?}");
        assert!(worst < Duration::from_millis(50), "search took {worst:?}");
    }

    #[test]
    fn view_rebuilds_rows_only_when_the_revision_moves() {
        let (mut shell, names) = siege_scene();
        let builds = Cell::new(0u32);
        let build = |shell: &Shell| {
            builds.set(builds.get() + 1);
            build_rows(&shell.rows(), &names)
        };
        let mut view = OutlinerView::new();
        let (rows, live) = view.sync(shell.revision(), || build(&shell)).unwrap();
        assert_eq!(
            (builds.get(), rows.len(), live.len()),
            (1, SIEGE_ROWS, SIEGE_ROWS)
        );
        // A quiet frame shares the same handles and never calls the builder.
        let (again, _) = view.sync(shell.revision(), || build(&shell)).unwrap();
        assert!(Arc::ptr_eq(&rows, &again));
        assert_eq!(builds.get(), 1);
        let hits = view.search("ai 4").unwrap();
        assert_eq!(hits.len(), 100);
        assert!(Arc::ptr_eq(&hits, &view.search("ai 4").unwrap()));
        assert_eq!(view.search("squad").unwrap().len(), 5);
        // An edit moves the revision: rows rebuild and the search runs again.
        shell
            .insert_node(
                ObjectId::new(),
                ObjectId::NIL,
                engine_scene::transform::Transform::identity(),
            )
            .unwrap();
        let (grown, grown_live) = view.sync(shell.revision(), || build(&shell)).unwrap();
        assert_eq!((builds.get(), grown.len()), (2, SIEGE_ROWS + 1));
        assert!(grown_live.len() > live.len());
        assert_eq!(view.search("squad").unwrap().len(), 5);
        // A refused build leaves nothing stale behind the new revision.
        let refused = view.sync(shell.revision() + 1, || {
            Err(OutlineError::TooManyRows {
                count: MAX_ROWS + 1,
                max: MAX_ROWS,
            })
        });
        assert!(refused.is_err());
        assert!(view.search("").unwrap().is_empty());
    }

    #[test]
    fn selection_sets_over_hundreds_of_siege_rows() {
        let (shell, names) = siege_scene();
        let rows = build_rows(&shell.rows(), &names).unwrap();
        let live = live_set(&rows);
        let order = order_of(&rows);
        let ai: Vec<u64> = search(&rows, &OutlinerQuery::new("ai "))
            .unwrap()
            .iter()
            .map(|hit| hit.id)
            .collect();
        assert_eq!(ai.len(), 500);
        let start = Instant::now();
        // Two overlapping sets of 200 AI rows: rows 0..200 and 100..300.
        let pick = |ids: &[u64]| {
            ids.iter().fold(Selection::new(), |sel, id| {
                select_add(&sel, *id, &live).expect("under the selection bound")
            })
        };
        let (a, b) = (pick(&ai[..200]), pick(&ai[100..300]));
        let mut bank = SelectionBank::new();
        bank.save("squad-a", a.clone(), &live).unwrap();
        bank.save("squad-b", b.clone(), &live).unwrap();
        assert_eq!(intersect(&a, &b).ids.len(), 100);
        assert_eq!(difference(&a, &b).ids.len(), 100);
        let wide = bank.recall_mode("squad-b", &a, RecallMode::Union);
        assert!(wide.is_err(), "300 rows pass the {MAX_SELECTION} bound");
        // A shift-click span over 250 rows fits; one more row does not.
        let span = select_range(&Selection::new(), order[10], order[259], &order, &live).unwrap();
        assert_eq!(span.ids.len(), 250);
        assert!(select_range(&Selection::new(), order[10], order[266], &order, &live).is_err());
        // Fifty AI rows despawn: the bank's sets prune to the survivors.
        let gone: BTreeSet<u64> = ai[..50].iter().copied().collect();
        let alive: BTreeSet<u64> = live.difference(&gone).copied().collect();
        assert_eq!(dangling_count(&a, &alive), 50);
        let mut pruned = bank.recall("squad-a", &Selection::new()).unwrap();
        assert!(prune(&mut pruned, &alive));
        assert_eq!(pruned.ids.len(), 150);
        let took = start.elapsed();
        eprintln!("ED19 siege: two 200-row sets, algebra and prune took {took:?}");
        assert!(took < Duration::from_millis(50), "sets took {took:?}");
    }

    #[test]
    fn quiet_panel_frame_stays_under_one_millisecond_at_siege_scale() {
        let (shell, names) = siege_scene();
        let revision = shell.revision();
        let mut view = OutlinerView::new();
        let rebuild = median_of(9, || {
            view = OutlinerView::new();
            view.sync(revision, || build_rows(&shell.rows(), &names))
                .unwrap();
        });
        // A quiet frame: synced rows, memoized hits, labels for the window
        // the scroll area shows (40 rows).
        let mut labelled = 0usize;
        let quiet = median_of(101, || {
            view.sync(revision, || unreachable!("revision did not move"))
                .unwrap();
            let hits = view.search("").unwrap();
            labelled = hits.iter().take(40).map(|hit| hit.label().len()).sum();
        });
        assert!(labelled > 0);
        eprintln!("ED19 siege: panel frame quiet {quiet:?}, after an edit {rebuild:?}");
        assert!(
            quiet < Duration::from_millis(1),
            "quiet frame took {quiet:?}"
        );
        assert!(
            rebuild < Duration::from_millis(10),
            "rebuild took {rebuild:?}"
        );
    }
}
