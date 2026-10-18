//! Scene-to-actor adapter plus staged before/after gate.
//!
//! Consumer: the command service stages immutable scene snapshots; this
//! module adapts plain rows (meters) into [`Actor`] and gates the staged
//! set against the before set. Data owner is the caller; units are meters
//! in, integer-cm inside via the checks. Determinism scope: same actor
//! sets in any input order give the same findings order and hash.
//! Failure mode: added overlap (red) or floating (yellow) is reported as
//! new findings in the returned delta; empty (clean verdict) means staged
//! adds nothing new.
//!
//! Std-only. Never imports editor or engine-scene types.

use super::checks::hash_findings;
use super::index::{pos_known, size_known, to_cm, Box3};
use super::place::{escape, overlap_depth_cm};
use super::{Actor, Finding, Report};
use std::collections::{BTreeMap, BTreeSet};

/// One plain snapshot row: `(id, name, x, y, z, sx, sy, sz, physical)`.
/// Positions and full-extent sizes are in meters.
pub type SnapshotRow<'a> = (u32, &'a str, f32, f32, f32, f32, f32, f32, bool);

/// Plain scene snapshot row in meters. No scene/editor types cross here.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedActor {
    pub id: u32,
    pub name: String,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub sx: f32,
    pub sy: f32,
    pub sz: f32,
    pub physical: bool,
}

impl StagedActor {
    /// One snapshot row. Sizes are full extents in meters.
    /// Nine args mirror [`SnapshotRow`]; callers with tuples should use
    /// [`actors_from_snapshot`] instead of this constructor.
    #[expect(
        clippy::too_many_arguments,
        reason = "wide signature kept until its inputs move into a params struct"
    )]
    pub fn new(
        id: u32,
        name: &str,
        x: f32,
        y: f32,
        z: f32,
        sx: f32,
        sy: f32,
        sz: f32,
        physical: bool,
    ) -> Self {
        Self {
            id,
            name: name.to_string(),
            x,
            y,
            z,
            sx,
            sy,
            sz,
            physical,
        }
    }

    /// Adapter to the spatial actor. Parent/velocity/yaw default out.
    pub fn to_actor(&self) -> Actor {
        Actor {
            id: self.id,
            name: self.name.clone(),
            x: self.x,
            y: self.y,
            z: self.z,
            sx: self.sx,
            sy: self.sy,
            sz: self.sz,
            parent: None,
            physical: self.physical,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }
}

/// Adapter for [`StagedActor`] rows. Sorted by id for frozen order.
pub fn staged_to_actors(rows: &[StagedActor]) -> Vec<Actor> {
    let mut out: Vec<Actor> = rows.iter().map(StagedActor::to_actor).collect();
    out.sort_by_key(|a| a.id);
    out
}

/// Adapter from a plain tuple snapshot to spatial actors.
///
/// Each row is `(id, name, x, y, z, sx, sy, sz, physical)` in meters;
/// sizes are full extents. Output is sorted by id so the same rows in
/// any input order give the same vector, findings order and hash.
pub fn actors_from_snapshot(rows: &[SnapshotRow<'_>]) -> Vec<Actor> {
    let staged: Vec<StagedActor> = rows
        .iter()
        .map(|(id, name, x, y, z, sx, sy, sz, physical)| {
            StagedActor::new(*id, name, *x, *y, *z, *sx, *sy, *sz, *physical)
        })
        .collect();
    staged_to_actors(&staged)
}

/// Staged gate: findings present in `staged` but not in `before`.
///
/// Runs the full offline catalog on both sets, then returns the delta
/// (new findings only) in frozen order with a stable hash. Rejection
/// rule for the caller: reject when the delta has verdict `red`
/// (added overlap or another red check) or when it holds new
/// `overlap`/`floating` findings versus before. An empty delta
/// (verdict `clean`) means staged adds nothing new, even if `before`
/// was already dirty. Order-invariant: same sets any order, same hash.
pub fn check_staged(before: &[Actor], staged: &[Actor]) -> Report {
    let (delta, _) = split_reports(before, staged);
    let hash = hash_findings(&delta);
    Report {
        findings: delta,
        hash,
    }
}

/// Findings only in `staged` (new) and only in `before` (resolved), both in
/// the frozen order of their own report. A finding is the same finding when
/// check name and both ids match; its percent may move.
fn split_reports(before: &[Actor], staged: &[Actor]) -> (Vec<Finding>, Vec<Finding>) {
    split_with(super::run(before), super::run(staged))
}

fn split_with(before_rep: Report, staged_rep: Report) -> (Vec<Finding>, Vec<Finding>) {
    let key = |f: &Finding| (f.check, f.a, f.b);
    let before_keys: BTreeSet<_> = before_rep.findings.iter().map(key).collect();
    let staged_keys: BTreeSet<_> = staged_rep.findings.iter().map(key).collect();
    let new = staged_rep
        .findings
        .into_iter()
        .filter(|f| !before_keys.contains(&key(f)))
        .collect();
    let resolved = before_rep
        .findings
        .into_iter()
        .filter(|f| !staged_keys.contains(&key(f)))
        .collect();
    (new, resolved)
}

/// The one move that clears a finding (SPX3): which object, which way, how far.
///
/// `mover` is the object to move. `other` is the second object of a pair
/// finding (`u32::MAX` when the finding names one object). `dir` is a door word
/// (`-x`, `+x`, `-z`, `+z`, `up`, `down`) and `cm` the distance along it.
/// `depth_cm` is how deep a pair overlaps in plan view, 0 for single findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fix {
    pub mover: u32,
    pub other: u32,
    pub dir: &'static str,
    pub cm: i32,
    pub depth_cm: i32,
}

/// One finding a write added or cleared, with its fix when one exists.
#[derive(Debug, Clone, PartialEq)]
pub struct DeltaFinding {
    pub finding: Finding,
    pub fix: Option<Fix>,
}

/// What one write changed in the spatial report, scoped to the objects it
/// touched (SPX3). `touched` is sorted and deduplicated.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedDelta {
    pub touched: Vec<u32>,
    pub new_findings: Vec<DeltaFinding>,
    pub resolved: Vec<DeltaFinding>,
}

fn same_row(a: &Actor, b: &Actor) -> bool {
    let lanes = |r: &Actor| [r.x, r.y, r.z, r.sx, r.sy, r.sz, r.vx, r.vz, r.yaw].map(f32::to_bits);
    a.name == b.name && a.physical == b.physical && a.parent == b.parent && lanes(a) == lanes(b)
}

/// Ids whose row differs between the two snapshots, or exists on one side only.
/// A moved parent's children change without being named by any edit, so this
/// finds them. Sorted, deduplicated.
pub fn changed_ids(before: &[Actor], staged: &[Actor]) -> Vec<u32> {
    let old_rows: BTreeMap<u32, &Actor> = before.iter().map(|a| (a.id, a)).collect();
    let new_ids: BTreeSet<u32> = staged.iter().map(|a| a.id).collect();
    let mut out: Vec<u32> = Vec::new();
    for row in staged {
        match old_rows.get(&row.id) {
            Some(old) if same_row(old, row) => {}
            _ => out.push(row.id),
        }
    }
    out.extend(
        before
            .iter()
            .map(|a| a.id)
            .filter(|id| !new_ids.contains(id)),
    );
    out.sort_unstable();
    out.dedup();
    out
}

fn involves(finding: &Finding, touched: &[u32]) -> bool {
    touched.binary_search(&finding.a).is_ok()
        || (finding.b != u32::MAX && touched.binary_search(&finding.b).is_ok())
}

fn plan_box(a: &Actor) -> Option<Box3> {
    if !pos_known(a) || !size_known(a) {
        return None;
    }
    let (hx, hy, hz) = (
        to_cm(a.sx).abs() / 2,
        to_cm(a.sy).abs() / 2,
        to_cm(a.sz).abs() / 2,
    );
    let (cx, cy, cz) = (to_cm(a.x), to_cm(a.y), to_cm(a.z));
    Some(Box3 {
        id: a.id,
        min_x: cx - hx,
        max_x: cx + hx,
        min_y: cy - hy,
        max_y: cy + hy,
        min_z: cz - hz,
        max_z: cz + hz,
    })
}

/// The one-move fix for `finding` over `actors`, preferring to move a touched
/// object. `overlap` pairs push along the shortest plan axis; `floating`,
/// `support` and `sunken` move straight down or up to the floor; every other
/// check has no single-move fix and reads `None`.
pub fn fix_for(finding: &Finding, actors: &[Actor], touched: &[u32]) -> Option<Fix> {
    let find = |id: u32| actors.iter().find(|a| a.id == id);
    match finding.check {
        "overlap" => {
            let a_touched = touched.binary_search(&finding.a).is_ok();
            let b_touched = touched.binary_search(&finding.b).is_ok();
            let (mover, other) = if !a_touched && b_touched {
                (finding.b, finding.a)
            } else {
                (finding.a, finding.b)
            };
            let moving = plan_box(find(mover)?)?;
            let fixed = plan_box(find(other)?)?;
            let push = escape(&moving, &fixed, false);
            Some(Fix {
                mover,
                other,
                dir: push.dir,
                cm: push.cm,
                depth_cm: overlap_depth_cm(&moving, &fixed, false),
            })
        }
        "floating" | "support" => {
            let b = plan_box(find(finding.a)?)?;
            (b.min_y > 0).then_some(Fix {
                mover: finding.a,
                other: u32::MAX,
                dir: "down",
                cm: b.min_y,
                depth_cm: 0,
            })
        }
        "sunken" => {
            let b = plan_box(find(finding.a)?)?;
            (b.min_y < 0).then_some(Fix {
                mover: finding.a,
                other: u32::MAX,
                dir: "up",
                cm: -b.min_y,
                depth_cm: 0,
            })
        }
        _ => None,
    }
}

/// Staged gate scoped to the objects a write touched: new findings and cleared
/// findings that involve `touched` (plus every row the write changed, such as
/// the children of a moved parent), each new one with its one-move fix.
///
/// Runs the same full offline catalog as [`check_staged`] on both sets and
/// filters by id, so a pre-existing overlap elsewhere never shows and a clean
/// move reads empty. Order-invariant: same sets in any order, same delta.
pub fn check_staged_ids(before: &[Actor], staged: &[Actor], touched: &[u32]) -> StagedDelta {
    let (new, resolved) = split_reports(before, staged);
    scoped(new, resolved, staged, touched, before)
}

/// [`check_staged_ids`] over a named subset of the catalog, for callers whose
/// snapshot cannot drive every check (the door withholds the ones that need
/// real sizes or names). Unknown names come back as the legal list.
pub fn check_staged_with(
    before: &[Actor],
    staged: &[Actor],
    touched: &[u32],
    checks: &[&str],
) -> Result<StagedDelta, Vec<&'static str>> {
    let (new, resolved) = split_with(
        super::run_checks(before, checks)?,
        super::run_checks(staged, checks)?,
    );
    Ok(scoped(new, resolved, staged, touched, before))
}

fn scoped(
    new: Vec<Finding>,
    resolved: Vec<Finding>,
    staged: &[Actor],
    touched: &[u32],
    before: &[Actor],
) -> StagedDelta {
    let mut ids: Vec<u32> = touched.to_vec();
    ids.extend(changed_ids(before, staged));
    ids.sort_unstable();
    ids.dedup();
    let new_findings = new
        .into_iter()
        .filter(|f| involves(f, &ids))
        .map(|finding| DeltaFinding {
            fix: fix_for(&finding, staged, &ids),
            finding,
        })
        .collect();
    let resolved = resolved
        .into_iter()
        .filter(|f| involves(f, &ids))
        .map(|finding| DeltaFinding { finding, fix: None })
        .collect();
    StagedDelta {
        new_findings,
        resolved,
        touched: ids,
    }
}

/// Merge window for [`MergeScope::Ends`], in logical ticks.
///
/// Consumer: [`StagedJournal::push`]. Units are caller ticks, never wall
/// time, so merges replay identically. Determinism scope: same push
/// sequence gives the same entry list and [`StagedJournal::version`].
pub const MERGE_ENDS: u64 = 4;
/// Merge window for [`MergeScope::All`], in logical ticks.
///
/// Wider than [`MERGE_ENDS`]: batched inspector drags coalesce while
/// discrete picks stay separate. Same units and determinism as
/// [`MERGE_ENDS`].
pub const MERGE_ALL: u64 = 16;
/// Cap on journaled undo steps.
///
/// Consumer: [`StagedJournal`]. Units are entries; older steps drop from
/// the head once the cap is exceeded. Determinism scope: same pushes give
/// the same retained window.
pub const MAX_STEPS: usize = 64;
/// Grammar-retry budget for [`place_retried`]: at most two relaxed
/// re-solves after the exact attempt, else the caller keeps the refusal.
///
/// Consumer: `scene.place` readers. Units are attempts; geometry stays
/// integer-cm inside [`super::place`], so no float reaches the digest.
pub const MAX_RETRIES: u32 = 2;

/// Per-field modification bitset for staged rows.
///
/// Consumer: prefab-style inherit in this module; the caller owns the data.
/// Units: none, one bit per field group. Determinism scope: pure value,
/// order-free. Errors: none; unknown bits are ignored by [`try_inherit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modified(u8);

impl Modified {
    /// Position `x` was edited locally.
    pub const POS_X: u8 = 0b00_0001;
    /// Position `y` was edited locally.
    pub const POS_Y: u8 = 0b00_0010;
    /// Position `z` was edited locally.
    pub const POS_Z: u8 = 0b00_0100;
    /// Any size axis was edited locally.
    pub const SIZE: u8 = 0b00_1000;
    /// Display name was edited locally.
    pub const NAME: u8 = 0b01_0000;
    /// Physical flag was edited locally.
    pub const PHYSICAL: u8 = 0b10_0000;
    /// No local edits: inherit everything.
    pub const NONE: u8 = 0;
    /// Every tracked field edited: inherit nothing.
    pub const ALL: u8 = 0b11_1111;

    /// Wrap raw bits. Unknown bits ride along and are ignored on read.
    pub fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// Raw bits, for journals and tests.
    pub fn bits(self) -> u8 {
        self.0
    }

    /// Empty set: nothing modified.
    pub fn empty() -> Self {
        Self(Self::NONE)
    }

    /// True when `flag` is set.
    pub fn contains(self, flag: u8) -> bool {
        self.0 & flag != 0
    }

    /// Union of two sets.
    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Inherit-only-if-clean: fields without a [`Modified`] bit come from
/// `base`, edited fields stay on `over`.
///
/// Consumer: staged compose over prefab instances; the caller owns both
/// rows. Units are meters passthrough (no math, so no rounding). `id`
/// always comes from `over`. Determinism scope: pure per-field pick, same
/// inputs give the same row. Errors: none; mismatched ids still merge with
/// `over` winning identity, never a panic.
pub fn try_inherit(base: &StagedActor, over: &StagedActor, modified: Modified) -> StagedActor {
    StagedActor {
        id: over.id,
        name: pick_str(&base.name, &over.name, modified.contains(Modified::NAME)),
        x: pick_f32(base.x, over.x, modified.contains(Modified::POS_X)),
        y: pick_f32(base.y, over.y, modified.contains(Modified::POS_Y)),
        z: pick_f32(base.z, over.z, modified.contains(Modified::POS_Z)),
        sx: pick_size(base.sx, over.sx, modified),
        sy: pick_size(base.sy, over.sy, modified),
        sz: pick_size(base.sz, over.sz, modified),
        physical: pick_bool(
            base.physical,
            over.physical,
            modified.contains(Modified::PHYSICAL),
        ),
    }
}

fn pick_f32(base: f32, over: f32, edited: bool) -> f32 {
    if edited {
        over
    } else {
        base
    }
}

fn pick_bool(base: bool, over: bool, edited: bool) -> bool {
    if edited {
        over
    } else {
        base
    }
}

fn pick_str(base: &str, over: &str, edited: bool) -> String {
    if edited {
        over.to_string()
    } else {
        base.to_string()
    }
}

fn pick_size(base: f32, over: f32, modified: Modified) -> f32 {
    pick_f32(base, over, modified.contains(Modified::SIZE))
}

/// Merge-window scope for [`StagedJournal::push`].
///
/// Consumer: staged undo batching. Units: window in logical ticks
/// ([`MERGE_ENDS`] vs [`MERGE_ALL`]). Determinism scope: same scope and
/// ticks give the same merge decision. Errors: none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeScope {
    /// Tip-only coalescing inside [`MERGE_ENDS`] ticks with the same name.
    Ends,
    /// Same rule with the wider [`MERGE_ALL`] window for drag batches.
    All,
}

impl MergeScope {
    /// Window in logical ticks for this scope.
    pub fn window(self) -> u64 {
        match self {
            Self::Ends => MERGE_ENDS,
            Self::All => MERGE_ALL,
        }
    }
}

/// One journaled staged edit: snapshots around a named change.
///
/// Consumer: [`StagedJournal`]; the caller owns the rows. Units are meters
/// in the rows, logical ticks in `tick`. Determinism scope: snapshots keep
/// caller order; equality is by value.
#[derive(Debug, Clone, PartialEq)]
pub struct JournalEntry {
    /// Stable edit label (`drag`, `place`, ...); merges need equal names.
    pub name: String,
    /// Logical tick of the last push folded into this entry.
    pub tick: u64,
    /// Rows before the entry.
    pub before: Vec<StagedActor>,
    /// Rows after the entry.
    pub after: Vec<StagedActor>,
}

/// Merge-window undo journal with a redo cursor and a version counter.
///
/// Consumer: the command service staging rows through [`check_staged`];
/// the caller owns the rows. Units are meters in rows, logical ticks at
/// push time. Determinism scope: same pushes in order give the same entries,
/// cursor and [`StagedJournal::version`]; no wall clock is read. Budget:
/// at most [`MAX_STEPS`] entries (older steps drop); each push clones the
/// staged rows once. Staged-gate path only, never the fixed tick.
#[derive(Debug, Clone)]
pub struct StagedJournal {
    entries: Vec<JournalEntry>,
    cursor: usize,
    version: u64,
    max_steps: usize,
}

impl StagedJournal {
    /// Empty journal retaining at most `max_steps` entries.
    ///
    /// A `max_steps` of zero keeps one step so undo stays total.
    pub fn new(max_steps: usize) -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
            version: 0,
            max_steps: max_steps.max(1),
        }
    }

    /// Monotonic version: bumps on every push, merge, undo and redo.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Applied entry count (redo cursor position).
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Retained entry count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no entry is retained.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Record `after` over `before` under `name` at logical `tick`.
    ///
    /// Merges into the tip entry when the name matches and the tick gap fits
    /// `scope`; otherwise truncates the redo tail and appends. Caps at the
    /// construction limit by dropping the oldest entry. Returns the new
    /// [`StagedJournal::version`].
    pub fn push(
        &mut self,
        name: &str,
        tick: u64,
        before: &[StagedActor],
        after: &[StagedActor],
        scope: MergeScope,
    ) -> u64 {
        if self.cursor < self.entries.len() {
            self.entries.truncate(self.cursor);
        }
        let mergeable = self.entries.last().and_then(|tip| {
            if tip.name == name && tick.saturating_sub(tip.tick) <= scope.window() {
                Some(tip.tick)
            } else {
                None
            }
        });
        if mergeable.is_some() {
            if let Some(tip) = self.entries.last_mut() {
                tip.after = after.to_vec();
                tip.tick = tick;
            }
            self.version = self.version.saturating_add(1);
            return self.version;
        }
        self.entries.push(JournalEntry {
            name: name.to_string(),
            tick,
            before: before.to_vec(),
            after: after.to_vec(),
        });
        if self.entries.len() > self.max_steps {
            let drop = self.entries.len() - self.max_steps;
            self.entries.drain(..drop);
        }
        self.cursor = self.entries.len();
        self.version = self.version.saturating_add(1);
        self.version
    }

    /// Step back one entry: returns the rows to restore, or `None` at the
    /// bottom. Bumps the version on success.
    pub fn undo(&mut self) -> Option<Vec<StagedActor>> {
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        self.version = self.version.saturating_add(1);
        Some(self.entries[self.cursor].before.clone())
    }

    /// Step forward one entry: returns the rows to restore, or `None` at
    /// the tip. Bumps the version on success.
    pub fn redo(&mut self) -> Option<Vec<StagedActor>> {
        if self.cursor >= self.entries.len() {
            return None;
        }
        let rows = self.entries[self.cursor].after.clone();
        self.cursor += 1;
        self.version = self.version.saturating_add(1);
        Some(rows)
    }
}

/// Longest kept staged name, in chars (mirrors the grammar check limit).
pub const STAGED_NAME_MAX: usize = 64;

/// Sanitize one staged name into the frozen grammar shape
/// (`[a-z0-9_]`, 1..=64 chars, no leading digit).
///
/// Consumer: [`place_retried`]; the caller owns the string. Units: chars.
/// Determinism scope: pure char map, same input gives the same output.
/// Errors: none; empty input degrades to `_`.
pub fn sanitize_staged_name(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        let lower = ch.to_ascii_lowercase();
        let ok = lower.is_ascii_lowercase() || lower.is_ascii_digit() || lower == '_';
        if ok {
            out.push(lower);
        } else {
            out.push('_');
        }
        if out.len() >= STAGED_NAME_MAX {
            break;
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    if out.as_bytes()[0].is_ascii_digit() {
        out.insert(0, '_');
        while out.len() > STAGED_NAME_MAX {
            out.pop();
        }
    }
    out
}

/// Grammar-clean actor names in place; returns true when any row changed.
///
/// Consumer: [`place_retried`]. Units are actor meters passthrough.
/// Determinism scope: id order untouched, only names rewrite. Errors: none.
pub fn clean_actor_names(actors: &mut [Actor]) -> bool {
    let mut changed = false;
    for a in actors.iter_mut() {
        let fixed = sanitize_staged_name(&a.name);
        if fixed != a.name {
            a.name = fixed;
            changed = true;
        }
    }
    changed
}

fn drop_first_unmet(wants: &mut Vec<super::place::Want>, unmet: &[super::place::Want]) -> bool {
    let Some(first) = unmet.first() else {
        return false;
    };
    if let Some(pos) = wants.iter().position(|w| w == first) {
        wants.remove(pos);
        return true;
    }
    if wants.is_empty() {
        return false;
    }
    let last = wants.len() - 1;
    wants.remove(last);
    true
}

/// Solve with a bounded grammar retry: exact attempt first, then at most
/// [`MAX_RETRIES`] relaxed re-solves dropping one unmet want each round.
///
/// Consumer: `scene.place` readers placing through staged rows; geometry
/// delegates to [`super::place::solve`] on the frozen [`GRID_CM`](super::place::GRID_CM)
/// grid x [`YAW_STEPS`](super::place::YAW_STEPS) yaws with the frozen
/// [`Relation`](super::relation::Relation) vocabulary, so the digest stays
/// order-invariant. Actor names are grammar-cleaned up front (retry zero),
/// which never moves geometry. Units are actor meters in, integer
/// centimeters inside the solver. Determinism scope: same intent and actors
/// in any input order give the same placement or the same refusal plus the
/// same attempt count. Returns the attempt count (`1` for exact) beside the
/// solver outcome; a refusal means even the relaxed intents failed within
/// the retry budget.
pub fn place_retried(
    intent: &super::place::Intent,
    actors: &[Actor],
) -> (
    Result<Vec<super::place::TransformOut>, super::place::ConflictSet>,
    u32,
) {
    let mut cleaned: Vec<Actor> = actors.to_vec();
    clean_actor_names(&mut cleaned);
    let mut wants = intent.wants.clone();
    let mut tries: u32 = 0;
    let mut attempt = super::place::Intent {
        object: intent.object.clone(),
        wants: wants.clone(),
    };
    loop {
        tries += 1;
        match super::place::solve(&attempt, &cleaned) {
            ok @ Ok(_) => return (ok, tries),
            Err(conflict) => {
                if tries > MAX_RETRIES {
                    return (Err(conflict), tries);
                }
                if !drop_first_unmet(&mut wants, &conflict.unmet) {
                    return (Err(conflict), tries);
                }
                attempt.wants.clone_from(&wants);
            }
        }
    }
}

/// Test-only timing wrapper around [`super::run`]: full offline check plus
/// caller-owned elapsed micros. Production uses [`super::run`] directly and
/// its clock is engine_core::Tick; tests pass fixed values. Elapsed time is
/// returned untouched and never feeds the report hash.
#[cfg(test)]
pub fn full_check_timed(actors: &[Actor], elapsed_micros: u64) -> (Report, u64) {
    (super::run(actors), elapsed_micros)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resting(id: u32, name: &str, x: f32, z: f32) -> Actor {
        Actor {
            id,
            name: name.into(),
            x,
            y: 0.5,
            z,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    fn clean_pair() -> Vec<Actor> {
        actors_from_snapshot(&[
            (0, "crate_a", 0.0, 0.5, 0.0, 1.0, 1.0, 1.0, true),
            (1, "crate_b", 10.0, 0.5, 10.0, 1.0, 1.0, 1.0, true),
        ])
    }

    #[test]
    fn snapshot_adapter_maps_rows_sorted() {
        let rows = [
            (7, "prop_g", 1.0, 0.5, 2.0, 1.0, 1.0, 1.0, true),
            (3, "prop_f", 0.0, 0.5, 0.0, 2.0, 1.0, 0.5, false),
        ];
        let actors = actors_from_snapshot(&rows);
        assert_eq!(actors.len(), 2);
        assert_eq!(actors[0].id, 3);
        assert_eq!(actors[1].id, 7);
        assert_eq!(actors[0].name, "prop_f");
        assert!(!actors[0].physical);
        assert!(actors[1].physical);
        // Reversed input gives the same vector, findings and hash.
        let rev = actors_from_snapshot(&[rows[1], rows[0]]);
        assert_eq!(rev.iter().map(|a| a.id).collect::<Vec<_>>(), vec![3, 7]);
        assert_eq!(
            super::super::run(&actors).hash,
            super::super::run(&rev).hash
        );
    }

    #[test]
    fn staged_overlap_rejected_with_findings() {
        let before = clean_pair();
        assert_eq!(super::super::run(&before).verdict(), "clean");
        let staged = vec![
            resting(0, "crate_a", 0.0, 0.0),
            resting(1, "crate_b", 0.5, 0.0),
        ];
        let delta = check_staged(&before, &staged);
        assert!(delta.count("overlap") >= 1);
        assert_eq!(delta.verdict(), "red");
        // Before had no overlap, staged adds it: new findings exist.
        assert!(!delta.findings.is_empty());
    }

    #[test]
    fn staged_floating_rejected() {
        let before = vec![resting(0, "crate_a", 0.0, 0.0)];
        assert_eq!(check_staged(&before, &before).count("floating"), 0);
        let mut high = resting(0, "crate_a", 0.0, 0.0);
        high.y = 5.0;
        let delta = check_staged(&before, &[high]);
        assert!(delta.count("floating") >= 1);
        // Floating is yellow, so rejection is via new findings, not red.
        assert!(!delta.findings.is_empty());
    }

    #[test]
    fn clean_staged_passes() {
        let before = clean_pair();
        let staged = before.clone();
        let delta = check_staged(&before, &staged);
        assert!(delta.findings.is_empty());
        assert_eq!(delta.verdict(), "clean");
        // A moved-but-still-clean staged set also passes.
        let moved = vec![
            resting(0, "crate_a", 0.0, 0.0),
            resting(1, "crate_b", 20.0, 20.0),
        ];
        let delta2 = check_staged(&before, &moved);
        assert!(delta2.findings.is_empty());
        assert_eq!(delta2.verdict(), "clean");
    }

    #[test]
    fn timing_test_records_micros() {
        // Arena scale: 32x32 grid, 2 m pitch, all grounded and disjoint.
        let mut rows: Vec<SnapshotRow<'static>> = Vec::new();
        let mut id = 0u32;
        for gx in 0..32 {
            for gz in 0..32 {
                rows.push((
                    id,
                    "arena_prop",
                    gx as f32 * 2.0,
                    0.5,
                    gz as f32 * 2.0,
                    1.0,
                    1.0,
                    1.0,
                    true,
                ));
                id += 1;
            }
        }
        // Static names keep borrows simple.
        let actors: Vec<Actor> = rows
            .iter()
            .map(|(i, _, x, y, z, sx, sy, sz, p)| {
                let mut a = resting(*i, "arena_prop", *x, *z);
                a.y = *y;
                a.sx = *sx;
                a.sy = *sy;
                a.sz = *sz;
                a.physical = *p;
                a
            })
            .collect();
        assert_eq!(actors.len(), 1024);
        let (timed, micros) = full_check_timed(&actors, 7);
        let plain = super::super::run(&actors);
        assert_eq!(timed.hash, plain.hash);
        assert_eq!(timed.findings.len(), plain.findings.len());
        // Caller-owned elapsed passes through untouched, outside the hash.
        assert_eq!(micros, 7);
        // Same hash on repeat; elapsed differs by caller but the report is stable.
        let (timed2, micros2) = full_check_timed(&actors, 11);
        assert_eq!(timed.hash, timed2.hash);
        assert_eq!(micros2, 11);
    }

    #[test]
    fn order_invariance_preserved() {
        let a = vec![
            resting(0, "crate_a", 0.0, 0.0),
            resting(1, "crate_b", 0.5, 0.0),
        ];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(super::super::run(&a).hash, super::super::run(&b).hash);
        let before = clean_pair();
        let mut before_rev = before.clone();
        before_rev.reverse();
        let d1 = check_staged(&before, &a);
        let d2 = check_staged(&before_rev, &b);
        assert_eq!(d1.hash, d2.hash);
        assert_eq!(d1.findings, d2.findings);
        let (t1, micros_a) = full_check_timed(&a, 7);
        let (t2, micros_b) = full_check_timed(&b, 7);
        assert_eq!(t1.hash, t2.hash);
        assert_eq!((micros_a, micros_b), (7, 7));
    }

    #[test]
    fn staged_actor_roundtrip() {
        let row = StagedActor::new(9, "gate_prop", 3.0, 0.5, 4.0, 1.0, 2.0, 1.0, true);
        let actor = row.to_actor();
        assert_eq!(actor.id, 9);
        assert_eq!(actor.name, "gate_prop");
        assert_eq!((actor.x, actor.y, actor.z), (3.0, 0.5, 4.0));
        let back = staged_to_actors(&[row]);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].id, 9);
    }

    #[test]
    fn inherit_keeps_clean_fields_from_base() {
        let base = StagedActor::new(1, "base_crate", 1.0, 0.5, 1.0, 1.0, 1.0, 1.0, true);
        let over = StagedActor::new(1, "edited", 9.0, 9.5, 9.0, 2.0, 2.0, 2.0, false);
        let clean = try_inherit(&base, &over, Modified::empty());
        assert_eq!(clean.name, "base_crate");
        assert_eq!((clean.x, clean.y, clean.z), (1.0, 0.5, 1.0));
        assert!(clean.physical);
        let dirty = try_inherit(
            &base,
            &over,
            Modified::from_bits(Modified::POS_X | Modified::NAME),
        );
        assert_eq!(dirty.name, "edited");
        assert_eq!(dirty.x, 9.0);
        assert_eq!(dirty.z, 1.0);
        assert!(dirty.physical);
        let all = try_inherit(&base, &over, Modified::from_bits(Modified::ALL));
        assert_eq!(all.name, "edited");
        assert!(!all.physical);
    }

    #[test]
    fn journal_merges_window_and_replays_cursor() {
        let a = vec![StagedActor::new(
            0, "crate_a", 0.0, 0.5, 0.0, 1.0, 1.0, 1.0, true,
        )];
        let b = vec![StagedActor::new(
            0, "crate_a", 1.0, 0.5, 0.0, 1.0, 1.0, 1.0, true,
        )];
        let c = vec![StagedActor::new(
            0, "crate_a", 2.0, 0.5, 0.0, 1.0, 1.0, 1.0, true,
        )];
        let mut journal = StagedJournal::new(MAX_STEPS);
        let v0 = journal.version();
        journal.push("drag", 10, &a, &b, MergeScope::Ends);
        journal.push("drag", 12, &b, &c, MergeScope::Ends);
        assert_eq!(journal.len(), 1, "same name inside ENDS window merges");
        assert!(journal.version() > v0);
        let back = journal.undo().expect("undo replays");
        assert_eq!(back[0].x, 0.0);
        assert_eq!(journal.cursor(), 0);
        let fwd = journal.redo().expect("redo replays");
        assert_eq!(fwd[0].x, 2.0);
        assert_eq!(journal.cursor(), 1);
        journal.push("place", 13, &c, &a, MergeScope::Ends);
        assert_eq!(journal.len(), 2, "new name splits the window");
        journal.push("drag", 100, &a, &b, MergeScope::Ends);
        assert_eq!(journal.len(), 3, "far tick splits the window");
    }

    #[test]
    fn journal_caps_steps_and_versions() {
        let a = vec![StagedActor::new(
            0, "crate_a", 0.0, 0.5, 0.0, 1.0, 1.0, 1.0, true,
        )];
        let b = vec![StagedActor::new(
            0, "crate_a", 1.0, 0.5, 0.0, 1.0, 1.0, 1.0, true,
        )];
        let mut journal = StagedJournal::new(2);
        journal.push("a", 0, &a, &b, MergeScope::All);
        journal.push("b", 100, &b, &a, MergeScope::All);
        journal.push("c", 200, &a, &b, MergeScope::All);
        assert_eq!(journal.len(), 2, "max_steps cap drops the head");
        assert_eq!(journal.cursor(), 2);
        let v = journal.version();
        assert!(journal.undo().is_some());
        assert!(journal.version() > v);
    }

    fn wheel_room() -> Vec<Actor> {
        let wall = |id: u32, name: &str, x: f32, z: f32, sx: f32, sz: f32| Actor {
            id,
            name: name.into(),
            x,
            y: 1.0,
            z,
            sx,
            sy: 2.0,
            sz,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        };
        vec![
            wall(0, "wall_north", 0.0, -5.0, 9.8, 0.2),
            wall(1, "wall_south", 0.0, 5.0, 9.8, 0.2),
            wall(2, "wall_west", -5.0, 0.0, 0.2, 10.0),
            wall(3, "wall_east", 5.0, 0.0, 0.2, 10.0),
            wall(4, "gate", 3.0, -4.5, 1.0, 0.5),
            Actor {
                id: 5,
                name: "brazier".into(),
                x: 0.0,
                y: 0.5,
                z: 2.0,
                sx: 0.6,
                sy: 1.0,
                sz: 0.6,
                parent: None,
                physical: true,
                vx: 0.0,
                vz: 0.0,
                yaw: 0.0,
            },
        ]
    }

    fn wheel_conflict(seed: usize) -> super::super::place::Intent {
        use super::super::place::{Intent, ObjectRef, Target, Wall, Want};
        use super::super::relation::Relation;
        if seed < 10 {
            let hi_n = 30 + (seed as i32 % 3) * 10;
            let hi_s = 30 + ((seed as i32 + 1) % 3) * 10;
            Intent {
                object: ObjectRef::Id(5),
                wants: vec![
                    Want {
                        relation: Relation::NextTo,
                        target: Target::Wall(Wall::North),
                        distance_cm: Some((0, hi_n)),
                    },
                    Want {
                        relation: Relation::NextTo,
                        target: Target::Wall(Wall::South),
                        distance_cm: Some((0, hi_s)),
                    },
                ],
            }
        } else {
            let s = seed - 10;
            let near_hi = 40 + (s as i32 % 3) * 10;
            let far_lo = 300 + (s as i32 % 2) * 20;
            Intent {
                object: ObjectRef::Id(5),
                wants: vec![
                    Want {
                        relation: Relation::NextTo,
                        target: Target::Object(4),
                        distance_cm: Some((0, near_hi)),
                    },
                    Want {
                        relation: Relation::NextTo,
                        target: Target::Object(4),
                        distance_cm: Some((far_lo, far_lo + 100)),
                    },
                ],
            }
        }
    }

    #[test]
    fn retry_lifts_twenty_seeded_conflicts_without_new_violations() {
        use super::super::place::solve;
        let mut base_solved = 0usize;
        let mut retry_solved = 0usize;
        for seed in 0..20 {
            let actors = wheel_room();
            let intent = wheel_conflict(seed);
            let base = solve(&intent, &actors);
            assert!(
                base.is_err(),
                "seed {seed}: joint failure must refuse exact"
            );
            if base.is_ok() {
                base_solved += 1;
            }
            let (retried, tries) = place_retried(&intent, &actors);
            assert!(
                (1..=1 + MAX_RETRIES).contains(&tries),
                "seed {seed}: retry in budget"
            );
            assert!(tries <= 3, "seed {seed}: at most two retries");
            let outs = retried.expect("seeded conflict must repair within budget");
            retry_solved += 1;
            assert_eq!(outs.len(), 1);
            let placed: Vec<Actor> = actors
                .iter()
                .map(|a| {
                    if a.id == outs[0].id {
                        Actor {
                            x: outs[0].x,
                            y: outs[0].y,
                            z: outs[0].z,
                            yaw: outs[0].yaw,
                            ..a.clone()
                        }
                    } else {
                        a.clone()
                    }
                })
                .collect();
            let report = super::super::run(&placed);
            assert_eq!(report.count("overlap"), 0, "seed {seed}: no new overlap");
            assert!(
                !report
                    .findings
                    .iter()
                    .any(|f| f.check == "support" && f.a == outs[0].id),
                "seed {seed}: no new support violation"
            );
            let again = super::super::run(&placed);
            assert_eq!(report.hash, again.hash, "seed {seed}: digest stable x2");
            let (twice, tries2) = place_retried(&intent, &actors);
            let outs2 = twice.expect("seeded retry replays");
            assert_eq!(outs, outs2, "seed {seed}: placement stable x2");
            assert_eq!(tries, tries2, "seed {seed}: attempt count stable");
        }
        assert_eq!(base_solved, 0);
        assert_eq!(retry_solved, 20, "retry must lift every seeded conflict");
    }

    #[test]
    fn retry_refuses_impossible_within_budget() {
        use super::super::place::{Intent, ObjectRef, Target, Wall, Want};
        use super::super::relation::Relation;
        let actors = wheel_room();
        let intent = Intent {
            object: ObjectRef::Name("missing_prop".to_string()),
            wants: vec![Want {
                relation: Relation::NextTo,
                target: Target::Wall(Wall::North),
                distance_cm: None,
            }],
        };
        let (out, tries) = place_retried(&intent, &actors);
        assert!(out.is_err(), "unknown identity still refuses");
        assert!(tries <= 1 + MAX_RETRIES);
    }

    #[test]
    fn grammar_names_clean_without_moving_geometry() {
        let mut actors = vec![resting(0, "Bad Name!", 0.0, 0.0)];
        assert!(clean_actor_names(&mut actors));
        assert_eq!(actors[0].name, "bad_name_");
        assert_eq!((actors[0].x, actors[0].z), (0.0, 0.0));
        assert_eq!(sanitize_staged_name(""), "_");
        assert!(sanitize_staged_name("9lives").starts_with('_'));
    }
    /// A thin wall slab (20 cm across x) that nothing else touches.
    fn slab(id: u32) -> Actor {
        Actor {
            sx: 0.2,
            sy: 2.0,
            sz: 10.0,
            y: 1.0,
            ..resting(id, "wall", 0.0, 0.0)
        }
    }

    fn scene_with_crate_at(crate_x: f32) -> Vec<Actor> {
        // A pre-existing overlap far away (ids 8, 9) rides every scene.
        vec![
            slab(1),
            resting(2, "crate", crate_x, 0.0),
            resting(8, "old_a", 20.0, 20.0),
            resting(9, "old_b", 20.4, 20.0),
        ]
    }

    #[test]
    fn spatial_delta_names_both_ids_and_the_axis_fix() {
        let before = scene_with_crate_at(5.0);
        let after = scene_with_crate_at(0.0);
        let delta = check_staged_ids(&before, &after, &[2]);
        assert_eq!(delta.touched, vec![2]);
        assert_eq!(delta.new_findings.len(), 1, "{:?}", delta.new_findings);
        let found = &delta.new_findings[0];
        assert_eq!(
            (found.finding.check, found.finding.a, found.finding.b),
            ("overlap", 1, 2)
        );
        // The crate (100 cm) straddles the 20 cm slab: 60 cm out either side.
        assert_eq!(
            found.fix,
            Some(Fix {
                mover: 2,
                other: 1,
                dir: "-x",
                cm: 60,
                depth_cm: 20,
            })
        );
        assert!(delta.resolved.is_empty());
    }

    #[test]
    fn spatial_delta_hides_old_overlaps_and_clean_moves_read_empty() {
        let before = scene_with_crate_at(5.0);
        // Pre-existing overlap (8, 9) stays out of the delta even when asked about it.
        let clean = scene_with_crate_at(6.0);
        let delta = check_staged_ids(&before, &clean, &[2]);
        assert!(delta.new_findings.is_empty() && delta.resolved.is_empty());
        let named = check_staged_ids(&before, &clean, &[8, 9]);
        assert!(named.new_findings.is_empty(), "old overlap is not new");
        assert_eq!(named.touched, vec![2, 8, 9]);
        // A crate already inside the wall that moves out resolves the overlap.
        let inside = scene_with_crate_at(0.0);
        let out = check_staged_ids(&inside, &before, &[2]);
        assert!(out.new_findings.is_empty());
        assert_eq!(out.resolved.len(), 1);
        assert_eq!(out.resolved[0].finding.check, "overlap");
        assert!(out.resolved[0].fix.is_none());
    }

    #[test]
    fn spatial_delta_finds_moved_children_and_ignores_input_order() {
        let before = scene_with_crate_at(5.0);
        let mut after = scene_with_crate_at(0.0);
        // The caller names nothing: the changed row alone scopes the delta.
        let delta = check_staged_ids(&before, &after, &[]);
        assert_eq!(delta.touched, vec![2]);
        assert_eq!(delta.new_findings.len(), 1);
        let mut before_rev = before.clone();
        before_rev.reverse();
        after.reverse();
        assert_eq!(delta, check_staged_ids(&before_rev, &after, &[]));
        // Removing the crate clears its overlap; its id is touched by absence.
        let gone: Vec<Actor> = scene_with_crate_at(0.0)
            .into_iter()
            .filter(|a| a.id != 2)
            .collect();
        let removed = check_staged_ids(&scene_with_crate_at(0.0), &gone, &[2]);
        assert_eq!(removed.resolved.len(), 1);
        assert!(removed.new_findings.is_empty());
    }

    #[test]
    fn spatial_delta_floating_fix_lowers_to_the_floor() {
        let before = vec![resting(1, "lamp", 0.0, 0.0)];
        let mut lamp = before[0].clone();
        lamp.y = 2.5;
        let delta = check_staged_ids(&before, &[lamp], &[1]);
        let fix = delta
            .new_findings
            .iter()
            .find(|f| f.finding.check == "floating")
            .and_then(|f| f.fix)
            .expect("floating carries a fix");
        // Bottom at 2.0 m: lower 200 cm to rest on the floor.
        assert_eq!((fix.mover, fix.dir, fix.cm), (1, "down", 200));
    }
}
