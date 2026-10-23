//! editor: headless-first dock shell. Layout is data: serializes, hashes x2, viewport last.

/// Recorded editable-scene session (gate evidence): headless import, place,
/// edit, save and reopen through the shell, browser and save codec.
pub mod a2_session;
/// Recorded editor run (session evidence): viewport, picking, hierarchy,
/// inspector, gizmo, undo/redo, logs, isolated play and crash recovery.
pub mod a3_session;
/// Activity feed tagged human or agent plus a scene diff, so a person and an
/// AI on the same scene see each other's edits. Std-only: a bounded activity
/// ring and a total snapshot diff, consumed by the `activity` dock panel
/// ([`shell::Panel::Activity`]) and the future door verbs `session.activity`
/// and `scene.diff`.
pub mod activity;
/// Editor executable wiring (winit + egui + render snapshots + play child).
#[cfg(feature = "ui")]
pub mod app;
/// Digest-chained audit receipts over the crash-safe journal (rotation-capped).
pub mod audit;
/// Asset browser over asset bundles: search, drag-to-place intents,
/// place-rule refusal. Std-only; consumed by the shell and the recorded
/// sessions (`shell.rs`, `a2_session.rs`, `a3_session.rs`).
pub mod browser;
#[cfg(feature = "ui")]
pub mod browser_tab;
pub mod capability;
pub mod commands;
pub mod durable;
/// Findings panel: spatial findings on a staged AI proposal, fixes via commands.
#[cfg(feature = "ui")]
pub mod findings;
/// Fracture authoring: paint one bond through the command service, preview its
/// strength and replay a break without touching the document. Std-only.
pub mod fracture_tool;
/// Geometry-only batch staging shared by the CAP1 spatial gate and the
/// findings panel: same staged poses and components a commit would produce.
mod geometry;
pub mod gizmo;
/// Data-only node graph for material and VFX graphs; wired to Document.
pub mod graph;
/// Schema inspector over registry descriptors: override badging, batched
/// command edits. Std-only; consumed by the dock panel (`shell_ui.rs`
/// renders [`inspector::build_grid`]).
pub mod inspector;
/// Automatic inspector rows from registry schemas: min/max/step/dropdown
/// and read-only hints with undoable command-batch edits. Std-only.
pub mod inspector_auto;
pub mod instant;
/// First-run checklist over templates and play-in-editor: tutorial-1 steps, hints-only
/// guidance and the 15-minute time-to-first-fight budget. Std-only; the
/// headless proof boot flow drives it (`crates/proof/src/fb_ship.rs`
/// calls note_template / note_first_fight).
pub mod onboard;
/// Outliner: filtered rows plus named selection sets over the shared
/// inspector selection. Std-only; consumed by the editor hierarchy tab
/// (`shell_ui.rs`).
pub mod outline;
/// Command palette: one fuzzy index over the dock panels, door verbs and
/// listed assets, ranked with highlight ranges for the palette panel.
/// Std-only.
pub mod palette;
pub mod play;
/// Play-in-editor: game-window-equivalent fixed-tick loop in the viewport,
/// pause/step/restart over play snapshots. Behind `ui` with the engine-app
/// lifecycle edge so headless stays std-only. The user path is `--record-a3`
/// straight into [`play_in::PlayInSession`]; the `EditorApp` in-viewport
/// facade was cut 2026-10-16 (recover via `git log --follow -- crates/editor/src/app.rs`).
#[cfg(feature = "ui")]
pub mod play_in;
/// Pose shelf (W4): named poses over recorded [`pose_tape::PoseTape`]s, with a
/// caller-rendered swatch. Std-only; the shelf never records or replays.
pub mod pose_shelf;
/// Mouse-to-joint pose recorder: quantized joint targets as a bounded integer
/// tape, validated and replayed deterministically. The UI layer solves IK in
/// floats and hands the milli-turn angles here; this core carries no float and
/// never touches sim state. Std-only.
pub mod pose_tape;
/// Prefabs: capture a selection subtree and instantiate it as new nodes.
/// Std-only; the shell owns the command batch and undo.
pub mod prefab;
pub mod save;
/// Multi-select set over scene objects plus the object clipboard: copy, paste
/// with an offset and duplicate with caller-minted ids. Std-only; the gizmo
/// reads the set through [`gizmo::drag_set`].
pub mod select;
pub mod shell;
#[cfg(feature = "ui")]
pub mod shell_ui;
/// One scene -> spatial adapter: real collider extents, the labelled fallback
/// and the only owner of "how big is this node" (CAP1 gate, findings, door,
/// play). Std-only.
pub mod spatial_view;
/// Spine inspector: spine actor columns as read-only inspector nodes through
/// the RC2 extraction bridge and the RC4 schemas (consumed by `shell_ui.rs`).
pub mod spine_inspect;
/// New-item templates: arena, prop, weapon and fighter starters.
pub mod templates;
/// Editor theme: dark warm palette, bronze accent, spacing and text scale.
/// Behind `ui` with egui; headless builds stay std-only.
#[cfg(feature = "ui")]
pub mod theme;
/// Asset thumbnails: 256x256 RGBA8 images keyed by content digest, LRU-bounded
/// at a caller byte cap, with the requested-but-absent list. Std-only; the
/// caller supplies the pixels from the capture path.
pub mod thumbnail;
/// Live tuning panel: combat/AI drafts as previewed batches,
/// applied at tick boundaries through the practice child seam, journaled in a tape.
#[cfg(feature = "ui")]
pub mod tuning;
pub mod viewport;
/// Live viewport texture (CPU raster, render-validated uploads).
/// Behind `ui` with the rest of the render wiring so headless stays std-only.
#[cfg(feature = "ui")]
pub mod viewport_image;
/// Crash-safe journal: write-ahead log over the command service.
pub mod wal;
/// Reimport watch (E10): file snapshots polled at tick boundaries with a
/// coalescing window, so a DCC save costs one reimport. Std-only, no threads.
pub mod watch;

use std::collections::VecDeque;

use crate::activity::Author;

/// Why [`Tree::decode`] refused a layout text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeError {
    /// No active-index line.
    Empty,
    /// The active index is not a number or points past the last tab.
    BadActive,
}

impl std::fmt::Display for TreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Empty => "layout has no active-index line",
            Self::BadActive => "layout active index is not a tab",
        })
    }
}

impl std::error::Error for TreeError {}

/// Dock tree: ordered tabs + active index. Order = render order (deterministic).
pub struct Tree {
    tabs: Vec<String>,
    active: usize,
}

impl Tree {
    /// Empty layout.
    pub fn new() -> Self {
        Self {
            tabs: vec![],
            active: 0,
        }
    }
    /// Open a tab (dedupes, activates it).
    pub fn open(&mut self, name: &str) {
        if let Some(i) = self.tabs.iter().position(|t| t == name) {
            self.active = i;
        } else {
            self.tabs.push(name.to_string());
            self.active = self.tabs.len() - 1;
        }
    }
    /// Close a tab. False when absent.
    pub fn close(&mut self, name: &str) -> bool {
        if let Some(i) = self.tabs.iter().position(|t| t == name) {
            self.tabs.remove(i);
            self.active = self.active.min(self.tabs.len().saturating_sub(1));
            true
        } else {
            false
        }
    }
    /// Encode layout: `active_idx \n tab...` (roundtrips via `decode`).
    pub fn encode(&self) -> String {
        let mut out = format!("{}\n", self.active);
        out.push_str(&self.tabs.join("\n"));
        out
    }
    /// Decode layout. Err on malformed active index.
    pub fn decode(s: &str) -> Result<Self, TreeError> {
        let mut lines = s.split('\n');
        let active: usize = lines
            .next()
            .ok_or(TreeError::Empty)?
            .parse()
            .map_err(|_| TreeError::BadActive)?;
        let tabs: Vec<String> = lines.map(|l| l.to_string()).collect();
        let n = tabs.len();
        if n > 0 && active >= n {
            return Err(TreeError::BadActive);
        }
        Ok(Self {
            tabs,
            active: if n == 0 { 0 } else { active },
        })
    }
    /// FNV-1a64 over the encoded layout (layout-hash gate).
    pub fn layout_hash(&self) -> u64 {
        engine_core::hash_bytes(self.encode().as_bytes())
    }
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

/// Bounded, visible undo history: layout snapshots with their scene position.
///
/// The scene undo itself stays the command journal's
/// ([`commands::undo`](crate::commands::undo)); this ring is what the history
/// panel shows and what [`History::jump`] reads when a person clicks an older
/// row. The ring cap style follows [`ActivityRing`](crate::activity::ActivityRing):
/// the oldest entry leaves first and [`History::dropped`] counts it.
pub struct History {
    entries: VecDeque<HistoryEntry>,
    dropped: u64,
    cap: usize,
    /// Named versions: name plus entry index, oldest name first.
    names: Vec<(String, usize)>,
}

impl History {
    /// Construct with a cap (use [`HISTORY_CAP`] for the dock).
    pub fn new(cap: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            dropped: 0,
            cap: cap.max(1),
            names: Vec::new(),
        }
    }

    /// Max kept entries.
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Push one step, evicting the oldest past the cap. Named versions follow
    /// their entry: a name on the evicted step is dropped and every kept name
    /// shifts down by one index.
    pub fn push(&mut self, entry: HistoryEntry) {
        self.entries.push_back(entry);
        if self.entries.len() > self.cap {
            self.entries.pop_front();
            self.dropped = self.dropped.saturating_add(1);
            self.names.retain(|(_, at)| *at > 0);
            for (_, at) in &mut self.names {
                *at = at.saturating_sub(1);
            }
        }
    }

    /// Give one kept entry a name (`W7` named versions).
    ///
    /// The newest [`MAX_NAMES`] names stay; naming past that drops the oldest
    /// name. An empty or duplicate name and an index past the ring refuse,
    /// changing nothing.
    pub fn name(&mut self, index: usize, name: &str) -> Result<(), NameError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(NameError::Empty);
        }
        if self.entries.get(index).is_none() {
            return Err(NameError::OutOfRange {
                index,
                len: self.entries.len(),
            });
        }
        if self.names.iter().any(|(known, _)| known == name) {
            return Err(NameError::Duplicate {
                name: name.to_string(),
            });
        }
        self.names.push((name.to_string(), index));
        if self.names.len() > MAX_NAMES {
            self.names.remove(0);
        }
        Ok(())
    }

    /// The entry a name points at, when the name is still kept.
    pub fn version(&self, name: &str) -> Option<&HistoryEntry> {
        let (_, index) = self.names.iter().find(|(known, _)| known == name)?;
        self.entries.get(*index)
    }

    /// Kept names oldest first, with the entry index each points at.
    pub fn names(&self) -> Vec<(&str, usize)> {
        self.names
            .iter()
            .map(|(name, index)| (name.as_str(), *index))
            .collect()
    }

    /// One panel row per kept entry, newest first.
    pub fn rows_newest_first(&self) -> Vec<HistoryRow> {
        let mut rows: Vec<HistoryRow> = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| HistoryRow {
                index,
                label: entry.label.clone(),
                revision: entry.revision,
                tick: entry.tick,
                name: self
                    .names
                    .iter()
                    .find(|(_, at)| *at == index)
                    .map(|(name, _)| name.clone()),
            })
            .collect();
        rows.reverse();
        rows
    }

    /// Activate one history-panel row (newest first): the jump the panel hands
    /// the scene owner. Row `0` is the newest step.
    pub fn activate(&self, row: usize) -> Result<HistoryJump, HistoryError> {
        let len = self.entries.len();
        let index = len
            .checked_sub(row.saturating_add(1))
            .ok_or(HistoryError::OutOfRange { index: row, len })?;
        self.jump(index)
    }

    /// Depth.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many entries the ring dropped so far.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// One kept entry by index, oldest first.
    pub fn entry(&self, index: usize) -> Option<&HistoryEntry> {
        self.entries.get(index)
    }

    /// Kept entries oldest first (panel render order).
    pub fn entries(&self) -> impl Iterator<Item = &HistoryEntry> + '_ {
        self.entries.iter()
    }

    /// The newest kept entry.
    pub fn latest(&self) -> Option<&HistoryEntry> {
        self.entries.back()
    }

    /// Jump to one kept entry: restore its layout and report the scene move.
    ///
    /// The layout comes back byte-for-byte. `ops` is the journal distance from
    /// the newest entry to the target, and `backward` says which way the scene
    /// owner walks it (`true` = undo that many journal entries, `false` = redo).
    /// A jump does not mutate the ring: the caller applies the scene move
    /// first, then its own layout restore decides what the next push records.
    pub fn jump(&self, index: usize) -> Result<HistoryJump, HistoryError> {
        let target = self.entries.get(index).ok_or(HistoryError::OutOfRange {
            index,
            len: self.entries.len(),
        })?;
        let newest = self.entries.back().ok_or(HistoryError::OutOfRange {
            index,
            len: self.entries.len(),
        })?;
        Ok(HistoryJump {
            label: target.label.clone(),
            layout: target.layout.clone(),
            revision: target.revision,
            undo_depth: target.undo_depth,
            ops: newest.undo_depth.abs_diff(target.undo_depth),
            backward: index + 1 < self.entries.len(),
        })
    }
}

/// Most undo steps one dock [`History`] keeps; the oldest is evicted first.
pub const HISTORY_CAP: usize = 100;

/// Most named versions one [`History`] keeps; the oldest name leaves first.
pub const MAX_NAMES: usize = 32;

/// One history-panel row: a kept step in display order (newest first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryRow {
    /// Ring index of the entry, oldest first (`0` = oldest kept).
    pub index: usize,
    /// Step label, e.g. `scene.move`.
    pub label: String,
    /// Scene revision the step ended at.
    pub revision: u64,
    /// Fixed tick the step happened on.
    pub tick: u64,
    /// Named version pointing at this entry, when it has one.
    pub name: Option<String>,
}

impl HistoryRow {
    /// One panel line: `scene.move rev=3 t0042` plus ` name=approved` when the
    /// step carries a named version.
    pub fn text(&self) -> String {
        let base = format!("{} rev={} t{:04}", self.label, self.revision, self.tick);
        match &self.name {
            Some(name) => format!("{base} name={name}"),
            None => base,
        }
    }
}

/// Why [`History::name`] refused a named version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameError {
    /// The name held no text.
    Empty,
    /// The name already points at a step.
    Duplicate {
        /// The repeated name.
        name: String,
    },
    /// The index is past the kept entries.
    OutOfRange {
        /// Index asked for.
        index: usize,
        /// Kept entries.
        len: usize,
    },
}

impl std::fmt::Display for NameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("version name holds no text; pass 1 to 64 chars"),
            Self::Duplicate { name } => write!(
                f,
                "version name `{name}` is taken; pick a new name or jump to that version"
            ),
            Self::OutOfRange { index, len } => write!(
                f,
                "history has no entry {index}: it keeps {len} entries (index 0..{len}); name a kept index"
            ),
        }
    }
}

impl std::error::Error for NameError {}

/// One visible undo step: its label, the layout it left and the scene position
/// it ended at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    /// Short action name shown in the panel, e.g. `scene.move`.
    pub label: String,
    /// Layout text current when the step was recorded.
    pub layout: String,
    /// Scene revision the step ended at.
    pub revision: u64,
    /// Fixed tick the step happened on.
    pub tick: u64,
    /// Who did it (a person at the dock or an AI through the door).
    pub author: Author,
    /// Journal undo depth after the step; a jump applies the difference.
    pub undo_depth: usize,
}

impl HistoryEntry {
    /// Build one entry without validation; [`History::push`] keeps it.
    pub fn new(
        label: &str,
        layout: &str,
        revision: u64,
        tick: u64,
        author: Author,
        undo_depth: usize,
    ) -> Self {
        Self {
            label: label.to_string(),
            layout: layout.to_string(),
            revision,
            tick,
            author,
            undo_depth,
        }
    }

    /// One panel row: `t0042 human scene.move rev=3 depth=2`.
    pub fn text(&self) -> String {
        format!(
            "t{:04} {} {} rev={} depth={}",
            self.tick,
            self.author.name(),
            self.label,
            self.revision,
            self.undo_depth
        )
    }
}

/// What a [`History::jump`] asks the scene owner to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryJump {
    /// Label of the target step.
    pub label: String,
    /// Layout string to restore byte-for-byte.
    pub layout: String,
    /// Scene revision recorded at the target step.
    pub revision: u64,
    /// Journal undo depth recorded at the target step.
    pub undo_depth: usize,
    /// Journal entries to walk between the newest step and the target.
    pub ops: usize,
    /// True when the target is older than the newest step.
    pub backward: bool,
}

/// Why [`History::jump`] refused an index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryError {
    /// The index is past the kept entries.
    OutOfRange {
        /// Index asked for.
        index: usize,
        /// Kept entries.
        len: usize,
    },
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfRange { index, len } => write!(
                f,
                "history has no entry {index}: it keeps {len} entries (index 0..{len}); jump to a kept index"
            ),
        }
    }
}

impl std::error::Error for HistoryError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_close_activate() {
        let mut t = Tree::new();
        t.open("hierarchy");
        t.open("inspector");
        t.open("hierarchy");
        assert_eq!(t.encode(), "0\nhierarchy\ninspector");
        assert!(t.close("hierarchy"));
        assert!(!t.close("nope"));
        assert_eq!(t.encode(), "0\ninspector");
    }

    #[test]
    fn encode_decode_roundtrip_x2() {
        let mut t = Tree::new();
        t.open("a");
        t.open("b");
        let s = t.encode();
        assert_eq!(Tree::decode(&s).unwrap().encode(), s);
        assert_eq!(t.layout_hash(), Tree::decode(&s).unwrap().layout_hash());
        assert_eq!(Tree::decode("9\na\nb").err(), Some(TreeError::BadActive));
        assert_eq!(Tree::decode("x\na").err(), Some(TreeError::BadActive));
    }

    fn entry(label: &str, layout: &str, revision: u64, tick: u64, depth: usize) -> HistoryEntry {
        HistoryEntry::new(label, layout, revision, tick, Author::Human, depth)
    }

    #[test]
    fn history_records_three_labelled_edits() {
        let mut h = History::new(HISTORY_CAP);
        assert!(h.is_empty());
        h.push(entry("scene.place", "0\nhierarchy", 1, 10, 1));
        h.push(entry("scene.move", "1\nhierarchy\ninspector", 2, 20, 2));
        h.push(entry("scene.rename", "1\nhierarchy\nbrowser", 3, 30, 3));
        assert_eq!(h.len(), 3);
        assert_eq!(h.dropped(), 0);
        let labels: Vec<&str> = h.entries().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, vec!["scene.place", "scene.move", "scene.rename"]);
        assert_eq!(h.entry(1).unwrap().revision, 2);
        assert_eq!(h.entry(3), None);
        assert_eq!(
            h.latest().unwrap().text(),
            "t0030 human scene.rename rev=3 depth=3"
        );
    }

    #[test]
    fn jump_restores_the_first_layout_byte_for_byte() {
        let mut h = History::new(HISTORY_CAP);
        let first = "0\nhierarchy\ninspector\nviewport";
        h.push(entry("scene.place", first, 1, 10, 1));
        h.push(entry("scene.move", "1\nhierarchy", 2, 20, 2));
        h.push(entry("scene.move", "2\nhierarchy\nbrowser", 3, 30, 3));

        let jump = h.jump(0).unwrap();
        assert_eq!(jump.layout, first);
        assert_eq!(jump.layout.as_bytes(), first.as_bytes());
        assert_eq!(jump.revision, 1);
        assert_eq!(jump.undo_depth, 1);
        assert_eq!(jump.ops, 2);
        assert!(jump.backward);
        assert_eq!(jump.label, "scene.place");
        // A jump is a report: the ring keeps every entry and its order.
        assert_eq!(h.len(), 3);
        assert_eq!(h.entry(0).unwrap().layout, first);
        // Jumping to the newest step walks nothing.
        let newest = h.jump(2).unwrap();
        assert_eq!(newest.ops, 0);
        assert!(!newest.backward);

        assert_eq!(
            h.jump(3),
            Err(HistoryError::OutOfRange { index: 3, len: 3 })
        );
        let text = HistoryError::OutOfRange { index: 9, len: 3 }.to_string();
        assert!(text.contains("jump to a kept index"), "{text}");
    }

    #[test]
    fn history_ring_drops_the_oldest_and_counts_it() {
        let mut h = History::new(HISTORY_CAP);
        for i in 0..120u64 {
            h.push(entry(
                "scene.move",
                &format!("L{i}"),
                i,
                i,
                usize::try_from(i).unwrap(),
            ));
        }
        assert_eq!(h.len(), HISTORY_CAP);
        assert_eq!(h.dropped(), 20);
        // Entry 0 is the 21st push; the first 20 left.
        assert_eq!(h.entry(0).unwrap().layout, "L20");
        assert_eq!(h.latest().unwrap().layout, "L119");
        // A cap-3 ring drops from the front and keeps render order.
        let mut small = History::new(3);
        for label in ["a", "b", "c", "d", "e"] {
            small.push(entry(label, label, 1, 1, 1));
        }
        assert_eq!(small.len(), 3);
        assert_eq!(small.dropped(), 2);
        let kept: Vec<&str> = small.entries().map(|row| row.label.as_str()).collect();
        assert_eq!(kept, vec!["c", "d", "e"]);
    }

    #[test]
    fn named_versions_roundtrip_and_refuse_three_ways() {
        let mut h = History::new(HISTORY_CAP);
        h.push(entry("scene.place", "0\nhierarchy", 1, 10, 1));
        h.push(entry("scene.move", "1\nhierarchy", 2, 20, 2));
        assert_eq!(h.name(0, "approved"), Ok(()));
        assert_eq!(h.version("approved").unwrap().layout, "0\nhierarchy");
        assert_eq!(h.names(), vec![("approved", 0)]);
        assert_eq!(h.name(1, "  "), Err(NameError::Empty));
        assert_eq!(
            h.name(1, "approved"),
            Err(NameError::Duplicate {
                name: "approved".to_string()
            })
        );
        assert_eq!(
            h.name(9, "later"),
            Err(NameError::OutOfRange { index: 9, len: 2 })
        );
        assert!(h.version("later").is_none());
        // Newest names stay: name 33 entries and the first one leaves.
        for i in 0..MAX_NAMES + 1 {
            h.push(entry("scene.move", "x", 3, 30, 3));
            let at = h.len() - 1;
            let _ = h.name(at, &format!("v{i}"));
        }
        assert!(h.version("approved").is_none(), "the oldest name left");
        assert_eq!(h.names().len(), MAX_NAMES);
        assert!(h.version(&format!("v{MAX_NAMES}")).is_some());
    }

    #[test]
    fn history_rows_run_newest_first_and_activate_returns_the_jump() {
        let mut h = History::new(HISTORY_CAP);
        let first = "0\nhierarchy\ninspector";
        h.push(entry("scene.place", first, 1, 10, 1));
        h.push(entry("scene.move", "1\nhierarchy", 2, 20, 2));
        h.push(entry("scene.move", "2\nhierarchy\nbrowser", 3, 30, 3));
        h.name(1, "wip").unwrap();

        let rows = h.rows_newest_first();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].label, "scene.move");
        assert_eq!(rows[0].revision, 3);
        assert_eq!(rows[0].tick, 30);
        assert_eq!(rows[0].index, 2);
        assert_eq!(rows[1].name.as_deref(), Some("wip"));
        assert_eq!(rows[2].text(), "scene.place rev=1 t0010");

        let jump = h.activate(2).unwrap();
        assert_eq!(jump.layout, first);
        assert_eq!(jump.layout.as_bytes(), first.as_bytes());
        assert_eq!(jump.ops, 2);
        assert!(jump.backward);
        assert_eq!(h.activate(0).unwrap().ops, 0);
        assert_eq!(
            h.activate(3),
            Err(HistoryError::OutOfRange { index: 3, len: 3 })
        );
    }

    #[test]
    fn named_versions_follow_entries_when_the_ring_drops() {
        let mut h = History::new(2);
        h.push(entry("a", "A", 1, 1, 1));
        h.push(entry("b", "B", 2, 2, 2));
        h.name(1, "keep-b").unwrap();
        h.push(entry("c", "C", 3, 3, 3));
        // Entry `a` left; the name shifted down and still points at `b`.
        assert_eq!(h.version("keep-b").unwrap().layout, "B");
        h.name(1, "keep-c").unwrap();
        h.push(entry("d", "D", 4, 4, 4));
        // `b` left with `keep-b`; the newer name shifted down and survived.
        assert!(h.version("keep-b").is_none());
        assert_eq!(h.version("keep-c").unwrap().layout, "C");
        assert_eq!(h.names(), vec![("keep-c", 0)]);
    }
}
