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
pub mod play;
/// Play-in-editor: game-window-equivalent fixed-tick loop in the viewport,
/// pause/step/restart over play snapshots. Behind `ui` with the engine-app
/// lifecycle edge so headless stays std-only. The user path is `--record-a3`
/// straight into [`play_in::PlayInSession`]; the `EditorApp` in-viewport
/// facade was cut 2026-10-16 (recover via `git log --follow -- crates/editor/src/app.rs`).
#[cfg(feature = "ui")]
pub mod play_in;
/// Mouse-to-joint pose recorder: quantized joint targets as a bounded integer
/// tape, validated and replayed deterministically. The UI layer solves IK in
/// floats and hands the milli-turn angles here; this core carries no float and
/// never touches sim state. Std-only.
pub mod pose_tape;
/// Prefabs: capture a selection subtree and instantiate it as new nodes.
/// Std-only; the shell owns the command batch and undo.
pub mod prefab;
pub mod save;
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

/// Bounded undo history of layout strings (cap-100 gate seed).
pub struct History {
    stack: Vec<String>,
    /// Max kept entries.
    pub cap: usize,
}

impl History {
    /// Construct with cap (use 100 per gate).
    pub fn new(cap: usize) -> Self {
        Self {
            stack: vec![],
            cap: cap.max(1),
        }
    }
    /// Push a layout snapshot, evicting oldest past cap.
    pub fn push(&mut self, layout: String) {
        self.stack.push(layout);
        if self.stack.len() > self.cap {
            self.stack.remove(0);
        }
    }
    /// Pop the latest snapshot (undo). None when empty.
    pub fn undo(&mut self) -> Option<String> {
        self.stack.pop()
    }
    /// Depth.
    pub fn len(&self) -> usize {
        self.stack.len()
    }
    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }
}

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

    #[test]
    fn history_cap_100() {
        let mut h = History::new(100);
        for i in 0..120 {
            h.push(format!("L{i}"));
        }
        assert_eq!(h.len(), 100);
        assert_eq!(h.undo().unwrap(), "L119");
        assert_eq!(h.len(), 99);
    }
}
