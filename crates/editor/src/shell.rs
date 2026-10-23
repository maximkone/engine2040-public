//! Dock shell: hierarchy, transform inspector and log panels over the command service.
//!
//! The consumer is the editor dock layout (egui in `shell_ui`, headless tests
//! here). The data owner is [`Shell`]: panel visibility, hierarchy selection,
//! a staged inspector edit, the [`Document`] plus [`Journal`] from
//! [`commands`](crate::commands), and a bounded [`LogRing`]. Units follow
//! `engine-scene`: meters for positions, radians for rotations.
//!
//! Every mutation constructs a [`Batch`] and applies it through the command
//! service (`validate` → `apply_named`, `undo`, `redo`). This module never
//! writes a [`Document`] field directly: the only mutation paths are
//! [`commands::apply_named`], [`commands::undo`] and [`commands::redo`],
//! with gizmo drags grouped by [`commands::begin_group`]/[`end_group`].
//! A staged inspector edit is validated with [`commands::validate`] before
//! it can apply, so an invalid draft (NaN, unknown node, stale revision)
//! reports an error and changes nothing. Every acknowledged apply also
//! chains one audit receipt ([`AuditChain`](crate::audit::AuditChain)), so
//! the dock is the production owner of [`crate::audit`].
//!
//! The hierarchy panel never stores scene handles: [`snapshot_rows`] takes a
//! [`Hierarchy`] as a function argument and returns plain [`SceneRow`] data
//! (ids plus a depth for indentation). Viewport picking and gizmo staging
//! live in [`Viewport`](crate::viewport::Viewport) plus
//! [`gizmo`](crate::gizmo); only windowed runs stay out of scope here: `shell_ui`
//! (behind the `ui` feature) draws these panels, and acceptance here is
//! panel/viewport/gizmo-to-batch logic tests.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use engine_assets::{import_gltf_path, MeshImportError};
use engine_scene::component::{AssetName, Component, ComponentError, Mesh};
use engine_scene::ids::ObjectId;
use engine_scene::schema::SchemaRegistry;
use engine_scene::transform::{Hierarchy, Transform};

use crate::audit::AuditChain;
use crate::inspector_auto::{build_grid, AutoError, AutoGrid};

use crate::activity::{Activity, ActivityRing, Author};
use crate::browser::{AssetKind, BundleView};
use crate::capability::{apply_gated, Approval, ApprovalLedger, CapError, Capability};
use crate::commands::{
    self, Applied, Batch, BatchError, CommandMeta, Document, Journal, MaterialName, Op, Preview,
};
#[cfg(feature = "ui")]
use crate::findings::{FindingView, FindingsError, FindingsPanel};
use crate::gizmo::{drag_to_local, world_to_local, GizmoDrag, GizmoError};
use crate::onboard::{Checklist, Step};
use crate::prefab::{Prefab, PrefabError};
use crate::templates::{TemplateError, TemplateItem, TemplateKind, TemplateService};
#[cfg(feature = "ui")]
use crate::tuning::{CombatField, LoopbackChild, TuneError, TuneReceipt, TuningPanel};
use crate::viewport::Viewport;

/// Self-registering dock panel inventory over [`Panel`].
#[path = "panels.rs"]
pub mod panels;

/// Default message capacity for [`Shell`]'s log panel.
pub const LOG_CAP: usize = 200;

/// Dock panels. The order of [`Shell::open_panels`] is the render order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Panel {
    /// Scene tree rows from [`snapshot_rows`]; click a row to select it.
    Hierarchy,
    /// Transform fields for the selection, staged then applied as a batch.
    Inspector,
    /// Bounded command receipt and error feed.
    Log,
    /// Ray-cast picking plus the translate/rotate gizmo over the selection.
    Viewport,
    /// Spatial findings on the staged AI proposal, fixed through the command service.
    Findings,
    /// Live combat/AI numbers, tuned through validated swaps at tick boundaries.
    Tuning,
    /// New-item templates: arena, prop, weapon and fighter starters.
    Templates,
    /// Asset files in a folder, with import through the EI3 glTF path (W5).
    Browser,
    /// Activity feed tagged human or agent, plus the scene diff, so a person
    /// and an AI on the same scene see each other's edits.
    Activity,
    /// Visible undo history: every kept step newest first, with named versions
    /// and a jump back to any kept step.
    History,
}

impl Panel {
    /// Tab title shown in the dock.
    pub fn title(self) -> &'static str {
        match self {
            Self::Hierarchy => "Hierarchy",
            Self::Inspector => "Inspector",
            Self::Log => "Log",
            Self::Viewport => "Viewport",
            Self::Findings => "Findings",
            Self::Tuning => "Tuning",
            Self::Templates => "Templates",
            Self::Browser => "Browser",
            Self::Activity => "activity",
            Self::History => "History",
        }
    }

    /// Every panel, in default dock order.
    pub fn all() -> Vec<Self> {
        panels::builtin_registry().panels()
    }
}

impl fmt::Display for Panel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.title())
    }
}

/// One hierarchy row: plain snapshot data for the hierarchy panel.
///
/// Built by [`snapshot_rows`] from a [`Hierarchy`] passed as a function
/// argument; rows never borrow or store scene handles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneRow {
    /// The node this row selects.
    pub id: ObjectId,
    /// Parent node, or [`ObjectId::NIL`] for a root.
    pub parent: ObjectId,
    /// Depth-first depth, for panel indentation.
    pub depth: u32,
    /// The node's local pose (meters/radians) at snapshot time.
    pub local: Transform,
}

/// Snapshot a hierarchy into deterministic panel rows.
///
/// Roots come first sorted by id, each followed depth-first by its
/// children (also sorted), so the same hierarchy always yields the same rows.
/// Takes scene data as an argument and returns owned rows; nothing is stored.
pub fn snapshot_rows(hierarchy: &Hierarchy) -> Vec<SceneRow> {
    let mut rows = Vec::with_capacity(hierarchy.len());
    // One pass builds every child list (sorted by id); asking the hierarchy
    // per node would scan all nodes each time, quadratic at siege scale.
    let kids_of = hierarchy.child_map();
    let mut stack: Vec<(ObjectId, u32)> = kids_of
        .get(&ObjectId::NIL)
        .into_iter()
        .flatten()
        .rev()
        .map(|id| (*id, 0))
        .collect();
    // Iterative depth-first walk; children pushed reversed so the smallest id
    // pops first. Depth saturates rather than overflowing on long chains.
    while let Some((id, depth)) = stack.pop() {
        let (parent, local) = match (hierarchy.parent(id), hierarchy.local(id)) {
            (Some(parent), Some(local)) => (parent, local),
            _ => continue,
        };
        rows.push(SceneRow {
            id,
            parent,
            depth,
            local,
        });
        for kid in kids_of.get(&id).into_iter().flatten().rev() {
            stack.push((*kid, depth.saturating_add(1)));
        }
    }
    rows
}

/// Bounded message ring for the log panel: oldest messages drop first.
#[derive(Clone, Debug)]
pub struct LogRing {
    messages: VecDeque<String>,
    cap: usize,
    dropped: u64,
}

impl LogRing {
    /// Empty ring keeping at most `cap` messages (`cap` clamps to at least 1).
    pub fn new(cap: usize) -> Self {
        Self {
            messages: VecDeque::new(),
            cap: cap.max(1),
            dropped: 0,
        }
    }

    /// Push a message, dropping the oldest past capacity.
    pub fn push(&mut self, message: impl Into<String>) {
        self.messages.push_back(message.into());
        if self.messages.len() > self.cap {
            self.messages.pop_front();
            self.dropped += 1;
        }
    }

    /// Messages oldest-first.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.messages.iter().map(String::as_str)
    }

    /// Number of kept messages.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// True when no message is kept.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Capacity of the ring.
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Messages dropped past capacity over this ring's lifetime.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Forget every kept message (the dropped counter is kept).
    pub fn clear(&mut self) {
        self.messages.clear();
    }
}

/// Which AI curve lane a panel edit targets (the Tuning tab's lane rows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(feature = "ui")]
pub enum TuneLane {
    /// Morale-to-attack-bias lane.
    Aggression,
    /// Morale-to-caution lane.
    Caution,
}

/// Cap on nodes one dropped file may add; a larger file is refused whole.
pub const MAX_IMPORT_NODES: usize = 64;

/// Why a `.glb`/`.gltf` import refused. Nothing is staged on any variant, so
/// the document stays byte-equal after a bad file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    /// The glTF parser or one of its bounds refused the file.
    Mesh(MeshImportError),
    /// The file held no triangle primitive.
    Empty,
    /// One file would add more nodes than [`MAX_IMPORT_NODES`].
    TooMany {
        /// Primitive count the file asked for.
        count: usize,
    },
    /// A name derived from the path failed the scene's asset-name rules.
    Name(ComponentError),
    /// The drop point was NaN or infinity.
    NonFinite,
    /// The command service refused the batch; the document is unchanged.
    Command(BatchError),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mesh(error) => write!(f, "glTF refused: {error}"),
            Self::Empty => f.write_str("the file has no triangle primitive"),
            Self::TooMany { count } => write!(
                f,
                "the file has {count} primitives; one drop adds at most {MAX_IMPORT_NODES}"
            ),
            Self::Name(error) => write!(f, "asset name refused: {error}"),
            Self::NonFinite => f.write_str("drop point is not finite"),
            Self::Command(error) => write!(f, "command refused: {error}"),
        }
    }
}

impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Mesh(error) => Some(error),
            Self::Name(error) => Some(error),
            Self::Command(error) => Some(error),
            Self::Empty | Self::TooMany { .. } | Self::NonFinite => None,
        }
    }
}

/// What rejected a shell edit. Command failures name the culprit node.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellError {
    /// An inspector edit needs a selected node first.
    NoSelection,
    /// Apply needs a staged edit first.
    NothingStaged,
    /// The selected node is gone from the document.
    UnknownNode(ObjectId),
    /// The command service refused the batch; the document is unchanged.
    Command(BatchError),
    /// The gizmo drag was invalid; nothing was staged or applied.
    Gizmo(GizmoError),
    /// A prefab capture or placement was refused; nothing changed.
    Prefab(PrefabError),
    /// A dropped file was refused; nothing changed.
    Import(ImportError),
}

impl fmt::Display for ShellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSelection => f.write_str("no node selected"),
            Self::NothingStaged => f.write_str("no staged edit to apply"),
            Self::UnknownNode(id) => write!(f, "selected node {id} is gone"),
            Self::Command(error) => write!(f, "command refused: {error}"),
            Self::Gizmo(error) => write!(f, "gizmo refused: {error}"),
            Self::Prefab(error) => write!(f, "prefab refused: {error}"),
            Self::Import(error) => write!(f, "import refused: {error}"),
        }
    }
}

impl std::error::Error for ShellError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Command(error) => Some(error),
            Self::Gizmo(error) => Some(error),
            Self::Prefab(error) => Some(error),
            Self::Import(error) => Some(error),
            Self::NoSelection | Self::NothingStaged | Self::UnknownNode(_) => None,
        }
    }
}

impl From<BatchError> for ShellError {
    fn from(error: BatchError) -> Self {
        Self::Command(error)
    }
}

/// A validated inspector draft waiting for apply.
#[derive(Clone, Debug)]
struct StagedEdit {
    target: ObjectId,
    draft: Transform,
    base_revision: u64,
    key: u64,
    preview: Preview,
    /// Undo-panel name the commit carries (`inspector edit`, `gizmo drag`).
    name: &'static str,
}

/// Dock shell state over the command service.
///
/// `open_panels` (ordered) and `selection` are the panel state; the document
/// and journal change only through [`commands::apply_named`],
/// [`commands::undo`] and [`commands::redo`], each applied batch logging its
/// receipt lines. A gizmo drag holds one open group (`gizmo_group`) so its
/// ticks commit as insignificant steps and release merges them into a single
/// significant entry; the merged entry keeps journal growth to one entry per
/// drag no matter the tick count.
pub struct Shell {
    /// Open panels in render order.
    pub open_panels: Vec<Panel>,
    /// Selected node, if its row was picked in the hierarchy panel.
    pub selection: Option<ObjectId>,
    /// Viewport camera and panel size for picking and the gizmo.
    pub viewport: Viewport,
    doc: Document,
    journal: Journal,
    log: LogRing,
    staged: Option<StagedEdit>,
    next_key: u64,
    /// Digest-chained audit receipts over acknowledged applies: one receipt
    /// per committed [`Shell::apply_staged`] batch, so the `audit` module's
    /// chain has a production owner instead of only tests.
    audit: AuditChain,
    /// Monotonic acknowledge counter. The dock has no fixed tick, so each
    /// receipt stamps the next value into its `tick` field.
    apply_tick: u64,
    /// Open gizmo-drag group, if a drag session is live.
    gizmo_group: Option<u64>,
    /// Template reservations: item names, node ids and cost totals.
    templates: TemplateService,
    /// Bounded activity ring: human and agent edits in one place (ED20).
    activity: ActivityRing,
    /// First-run checklist (tutorial-1): templates and first fight (ED18).
    onboard: Checklist,
    /// Saved prefabs in creation order (in-memory; not in the save file yet).
    prefabs: Vec<Prefab>,
    /// Review state for one staged AI proposal (behind `ui` with spatial).
    #[cfg(feature = "ui")]
    findings: FindingsPanel,
    /// Live tuning drafts plus the staged batch and tape journal.
    #[cfg(feature = "ui")]
    tuning: TuningPanel,
    /// Headless practice-faithful child the tuning batches apply through.
    #[cfg(feature = "ui")]
    tune_child: LoopbackChild,
}

impl Shell {
    /// Empty document at revision zero with every panel open.
    pub fn new() -> Self {
        Self {
            open_panels: Panel::all(),
            selection: None,
            viewport: Viewport::default_panel(),
            doc: Document::new(),
            journal: Journal::new(),
            log: LogRing::new(LOG_CAP),
            staged: None,
            next_key: 1,
            audit: AuditChain::new(),
            apply_tick: 0,
            gizmo_group: None,
            templates: TemplateService::new(),
            activity: ActivityRing::new(),
            onboard: Checklist::new(),
            prefabs: Vec::new(),
            #[cfg(feature = "ui")]
            findings: FindingsPanel::new(),
            #[cfg(feature = "ui")]
            tuning: TuningPanel::new(),
            #[cfg(feature = "ui")]
            tune_child: LoopbackChild::new(),
        }
    }

    /// Open a panel (dedupes, keeps render order).
    pub fn open_panel(&mut self, panel: Panel) {
        if !self.open_panels.contains(&panel) {
            self.open_panels.push(panel);
        }
    }

    /// Close a panel. False when it was not open.
    pub fn close_panel(&mut self, panel: Panel) -> bool {
        if let Some(i) = self.open_panels.iter().position(|p| *p == panel) {
            self.open_panels.remove(i);
            true
        } else {
            false
        }
    }

    /// True when the panel is open.
    pub fn is_open(&self, panel: Panel) -> bool {
        self.open_panels.contains(&panel)
    }

    /// The authoring document (read-only: edits go through batches).
    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// The undo/redo journal (read-only).
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Current document revision.
    pub fn revision(&self) -> u64 {
        self.doc.revision
    }

    /// The log panel feed.
    pub fn log(&self) -> &LogRing {
        &self.log
    }

    /// Push a panel message (used by hosts for notes, not scene edits).
    pub fn log_message(&mut self, message: impl Into<String>) {
        self.log.push(message);
    }

    /// Reopen a saved document: swap the document, reset the journal, clear
    /// selection and staging, and log the receipt. Used by Open and startup;
    /// the caller keeps the previous document on error (this takes a decoded
    /// [`Document`], so it cannot fail).
    pub fn open_document(&mut self, doc: Document) {
        let revision = doc.revision;
        let nodes = doc.hierarchy.len();
        self.doc = doc;
        self.journal = Journal::new();
        self.selection = None;
        self.staged = None;
        // A fresh journal drops every open group with it.
        self.gizmo_group = None;
        self.log
            .push(format!("opened document r{revision} ({nodes} nodes)"));
    }

    /// Forget every kept log message.
    pub fn clear_log(&mut self) {
        self.log.clear();
    }

    /// Current hierarchy rows for the hierarchy panel.
    pub fn rows(&self) -> Vec<SceneRow> {
        snapshot_rows(&self.doc.hierarchy)
    }

    /// Select a node (`nil` clears). True when the selection changed: the id
    /// must be nil or name a known node, and any staged edit is discarded.
    pub fn select(&mut self, id: ObjectId) -> bool {
        if id.is_nil() {
            self.selection = None;
            self.staged = None;
            return true;
        }
        if self.doc.hierarchy.contains(id) {
            self.selection = Some(id);
            self.staged = None;
            true
        } else {
            false
        }
    }

    /// The inspector draft: the staged pose when one targets the selection,
    /// else the selection's live local pose, else none.
    pub fn inspector_local(&self) -> Option<Transform> {
        let selected = self.selection?;
        if let Some(staged) = &self.staged {
            if staged.target == selected {
                return Some(staged.draft);
            }
        }
        self.doc.hierarchy.local(selected)
    }

    /// The staged preview, if an edit passed validation and waits for apply.
    pub fn staged_preview(&self) -> Option<&Preview> {
        self.staged.as_ref().map(|staged| &staged.preview)
    }

    /// Automatic inspector grid for the selection (ED9 wave-32 caller).
    ///
    /// Builds the 20-row [`AutoGrid`] via [`build_grid`] over the
    /// selection's live pose lanes (`pose.px` from `position[0]`,
    /// `pose.sx` from `scale[0]`, `pose.rx` from `rotation[0]`); every
    /// other cell falls back to its schema default. None when nothing is
    /// selected or the selection is gone. Pure query: never mutates
    /// selection, staging, the document or the journal.
    pub fn auto_grid(&self) -> Option<AutoGrid> {
        let selected = self.selection?;
        let live = self.doc.hierarchy.local(selected)?;
        let mut values = BTreeMap::new();
        values.insert(
            ("pose".to_string(), "pose.px".to_string()),
            live.position[0].to_string(),
        );
        values.insert(
            ("pose".to_string(), "pose.sx".to_string()),
            live.scale[0].to_string(),
        );
        values.insert(
            ("pose".to_string(), "pose.rx".to_string()),
            live.rotation[0].to_string(),
        );
        Some(build_grid(&SchemaRegistry::builtin(), &values))
    }

    /// Edit one automatic-inspector cell for the selection through the
    /// command service (validate, atomic apply, journal, undo).
    ///
    /// Drives [`build_grid`] for the selection first, then routes the edit
    /// through [`crate::inspector_auto::apply_pose_edit_on_shell`], so the
    /// batch commits via [`commands::apply_named`] like any inspector edit
    /// and undoes/redoes with [`undo`](Self::undo)/[`redo`](Self::redo).
    /// Out-of-range values refuse as [`AutoError::BadValue`] quoting the
    /// `min..=max` range and write nothing (logged); no selection or a
    /// gone selection refuses as `UnknownField` and writes nothing.
    pub fn auto_edit(
        &mut self,
        component: &str,
        field: &str,
        value: &str,
    ) -> Result<Applied, AutoError> {
        let target = match self.selection {
            Some(selected) => selected,
            None => {
                return Err(AutoError::UnknownField {
                    component: component.to_string(),
                    field: field.to_string(),
                });
            }
        };
        let grid = match self.auto_grid() {
            Some(grid) => grid,
            None => {
                return Err(AutoError::UnknownField {
                    component: component.to_string(),
                    field: field.to_string(),
                });
            }
        };
        match crate::inspector_auto::apply_pose_edit_on_shell(
            self, target, &grid, component, field, value,
        ) {
            Ok(applied) => Ok(applied),
            Err(error) => {
                self.log
                    .push(format!("ERR auto {component}.{field} `{value}`: {error}"));
                Err(error)
            }
        }
    }

    /// Stage a transform edit for the selection: validates a single-op batch
    /// and stores the draft only when validation passes. An invalid draft
    /// reports the error and leaves any earlier staged edit in place.
    pub fn stage_transform(&mut self, draft: Transform) -> Result<Preview, ShellError> {
        let target = self.selection.ok_or(ShellError::NoSelection)?;
        self.stage_local(target, draft, "stage", "inspector edit")
    }

    /// Stage a gizmo drag for the selection: applies the parent-frame delta
    /// to the selection's local pose, then validates the batch as a preview.
    /// An invalid drag reports the error and leaves any earlier staged edit
    /// in place.
    pub fn stage_gizmo_drag(&mut self, drag: &GizmoDrag) -> Result<Preview, ShellError> {
        let target = self.selection.ok_or(ShellError::NoSelection)?;
        let current = self
            .doc
            .hierarchy
            .local(target)
            .ok_or(ShellError::UnknownNode(target))?;
        let draft = match drag_to_local(current, drag) {
            Ok(draft) => draft,
            Err(error) => {
                self.log.push(format!("ERR gizmo {target}: {error}"));
                return Err(ShellError::Gizmo(error));
            }
        };
        self.stage_local(target, draft, "gizmo", "gizmo drag")
    }

    /// Apply a gizmo drag immediately through the command service: stage the
    /// preview first, then commit it, so a rejected drag writes nothing.
    pub fn apply_gizmo_drag(&mut self, drag: &GizmoDrag) -> Result<Applied, ShellError> {
        self.stage_gizmo_drag(drag)?;
        self.apply_staged()
    }

    /// Stage an absolute *world* pose from the gizmo session for the
    /// selection, converting it to the parent frame first. Err before staging
    /// when the pose is degenerate or the parent chain is unresolvable.
    pub fn stage_gizmo_world(&mut self, world: Transform) -> Result<Preview, ShellError> {
        let target = self.selection.ok_or(ShellError::NoSelection)?;
        if !self.doc.hierarchy.contains(target) {
            return Err(ShellError::UnknownNode(target));
        }
        let parent = self
            .doc
            .hierarchy
            .parent(target)
            .ok_or(ShellError::UnknownNode(target))?;
        let parent_world = if parent.is_nil() {
            Transform::identity()
        } else {
            self.doc
                .hierarchy
                .world(parent)
                .ok_or(ShellError::UnknownNode(parent))?
        };
        let draft = match world_to_local(parent_world, world) {
            Some(draft) => draft,
            None => {
                let error = GizmoError::DegenerateTransform;
                self.log.push(format!("ERR gizmo {target}: {error}"));
                return Err(ShellError::Gizmo(error));
            }
        };
        self.stage_local(target, draft, "gizmo", "gizmo drag")
    }

    /// Apply an absolute gizmo world pose immediately through the command
    /// service; a rejected pose writes nothing.
    pub fn apply_gizmo_world(&mut self, world: Transform) -> Result<Applied, ShellError> {
        self.stage_gizmo_world(world)?;
        self.apply_staged()
    }

    /// Pick against the current rows with the shell's viewport. Pure query:
    /// never mutates selection, staging or the document.
    pub fn pick(&self, x: f32, y: f32) -> Option<ObjectId> {
        self.viewport.pick(x, y, &self.rows())
    }

    /// Validate one `SetLocal` draft and store it as the staged edit.
    /// `origin` names the caller in the log line (`stage`, `gizmo`); `name`
    /// is the undo-panel name the later commit carries.
    fn stage_local(
        &mut self,
        target: ObjectId,
        draft: Transform,
        origin: &str,
        name: &'static str,
    ) -> Result<Preview, ShellError> {
        if !self.doc.hierarchy.contains(target) {
            return Err(ShellError::UnknownNode(target));
        }
        let batch = Batch {
            ops: vec![Op::SetLocal {
                id: target,
                local: draft,
            }],
            base_revision: self.doc.revision,
            idempotency_key: self.take_key(),
            provenance: None,
        };
        match commands::validate(&self.doc, &batch) {
            Ok(preview) => {
                let staged = StagedEdit {
                    target,
                    draft,
                    base_revision: batch.base_revision,
                    key: batch.idempotency_key,
                    preview: preview.clone(),
                    name,
                };
                self.staged = Some(staged);
                Ok(preview)
            }
            Err(error) => {
                self.log.push(format!("ERR {origin} {target}: {error}"));
                Err(ShellError::Command(error))
            }
        }
    }

    /// Apply the staged inspector edit through the command service as a
    /// significant grouped commit: the entry carries its staging name
    /// (`inspector edit`, `gizmo drag`) and seals in the durable journal.
    pub fn apply_staged(&mut self) -> Result<Applied, ShellError> {
        let staged = self.staged.clone().ok_or(ShellError::NothingStaged)?;
        let batch = Batch {
            ops: vec![Op::SetLocal {
                id: staged.target,
                local: staged.draft,
            }],
            base_revision: staged.base_revision,
            idempotency_key: staged.key,
            provenance: None,
        };
        let meta = CommandMeta::significant(staged.name);
        match commands::apply_named(&mut self.doc, &mut self.journal, &batch, &meta) {
            Ok(applied) => {
                self.record_audit(&batch, &meta, &applied);
                self.log_applied(&applied);
                self.note_activity(Author::Human, "batch.apply", None);
                self.staged = None;
                Ok(applied)
            }
            Err(error) => {
                self.log.push(format!("ERR apply staged: {error}"));
                Err(ShellError::Command(error))
            }
        }
    }

    /// Chain one acknowledged apply in the audit receipts: the entry covers
    /// the stamped acknowledge counter, the published revision, the batch key,
    /// the command name and the batch ops (see
    /// [`AuditChain::record_applied`]). Call only after the command service
    /// acknowledged the batch. A receipt that cannot fit the module's size
    /// bounds logs its refusal instead of panicking: the document and the
    /// journal are committed either way.
    fn record_audit(&mut self, batch: &Batch, meta: &CommandMeta, applied: &Applied) {
        let tick = self.apply_tick;
        self.apply_tick = self.apply_tick.saturating_add(1);
        if let Err(error) = self.audit.record_applied(tick, batch, meta, applied) {
            self.log.push(format!("ERR audit receipt: {error}"));
        }
    }

    /// Open a gizmo-drag session for the selection: later ticks commit as
    /// insignificant steps tagged with the returned group id, and
    /// [`end_gizmo_drag`](Self::end_gizmo_drag) merges them into one
    /// significant `gizmo drag` entry. Idempotent while a session is live
    /// (returns the open group). Needs a live selection first.
    pub fn begin_gizmo_drag(&mut self) -> Result<u64, ShellError> {
        let target = self.selection.ok_or(ShellError::NoSelection)?;
        if !self.doc.hierarchy.contains(target) {
            return Err(ShellError::UnknownNode(target));
        }
        if let Some(open) = self.gizmo_group {
            return Ok(open);
        }
        let key = self.take_key();
        let group = commands::begin_group(
            &mut self.journal,
            &CommandMeta::significant("gizmo drag"),
            key,
        );
        self.gizmo_group = Some(group);
        Ok(group)
    }

    /// True while a gizmo-drag session holds its group open.
    pub fn gizmo_drag_open(&self) -> bool {
        self.gizmo_group.is_some()
    }

    /// Apply one gizmo-drag tick inside the session: the parent-frame delta
    /// commits immediately as an insignificant step (undoable, skipped by the
    /// durable journal) so every viewport tick stays responsive and the later
    /// release folds them into one undo step. Opens a session when none is
    /// live, so a bare tick still groups. An invalid drag reports its error
    /// and writes nothing; the session stays open for the next tick.
    pub fn tick_gizmo_drag(&mut self, drag: &GizmoDrag) -> Result<Applied, ShellError> {
        if self.gizmo_group.is_none() {
            self.begin_gizmo_drag()?;
        }
        let target = self.selection.ok_or(ShellError::NoSelection)?;
        let current = self
            .doc
            .hierarchy
            .local(target)
            .ok_or(ShellError::UnknownNode(target))?;
        let draft = match drag_to_local(current, drag) {
            Ok(draft) => draft,
            Err(error) => {
                self.log.push(format!("ERR gizmo {target}: {error}"));
                return Err(ShellError::Gizmo(error));
            }
        };
        let batch = Batch {
            ops: vec![Op::SetLocal {
                id: target,
                local: draft,
            }],
            base_revision: self.doc.revision,
            idempotency_key: self.take_key(),
            provenance: None,
        };
        let meta = CommandMeta::insignificant("gizmo drag step");
        match commands::apply_named(&mut self.doc, &mut self.journal, &batch, &meta) {
            Ok(applied) => {
                self.log_applied(&applied);
                Ok(applied)
            }
            Err(error) => {
                self.log.push(format!("ERR gizmo tick {target}: {error}"));
                Err(ShellError::Command(error))
            }
        }
    }

    /// Release the gizmo-drag session: the session's steps merge into one
    /// significant `gizmo drag` entry (one undo step, one durable seal).
    /// None when no session is live, or when it held no steps. A single
    /// failing tick never blocks the release: the steps that did land still
    /// merge and undo/redo bit-exactly.
    pub fn end_gizmo_drag(&mut self) -> Option<Applied> {
        self.gizmo_group.take()?;
        let merged = commands::end_group(&mut self.journal);
        if let Some(applied) = &merged {
            self.log_applied(applied);
        }
        merged
    }

    /// Abandon the gizmo-drag session without merging: its steps stay
    /// individual undo entries. True when a session was live.
    pub fn cancel_gizmo_drag(&mut self) -> bool {
        if self.gizmo_group.take().is_none() {
            return false;
        }
        commands::cancel_group(&mut self.journal)
    }

    /// Insert a node through the command service.
    pub fn insert_node(
        &mut self,
        id: ObjectId,
        parent: ObjectId,
        local: Transform,
    ) -> Result<Applied, ShellError> {
        self.apply_ops(vec![Op::Insert { id, parent, local }])
    }

    /// Remove a node through the command service. A removed selection clears
    /// the selection and any staged edit with it.
    pub fn remove_node(&mut self, id: ObjectId) -> Result<Applied, ShellError> {
        let applied = self.apply_ops(vec![Op::Remove { id }])?;
        if self.selection == Some(id) && !self.doc.hierarchy.contains(id) {
            self.selection = None;
            self.staged = None;
        }
        Ok(applied)
    }

    /// Reparent a node through the command service, keeping its world pose.
    pub fn reparent_node(
        &mut self,
        id: ObjectId,
        new_parent: ObjectId,
    ) -> Result<Applied, ShellError> {
        self.apply_ops(vec![Op::Reparent { id, new_parent }])
    }

    /// The material bound to a node, if any. Pure query.
    pub fn material_of(&self, id: ObjectId) -> Option<MaterialName> {
        self.doc.materials.get(&id.to_string()).copied()
    }

    /// Bind a browser material asset to a node through the command service:
    /// validate the asset against the bundle, then commit a journaled
    /// `SetMaterial` batch (undoable, durable, saved in the scene bytes).
    /// An unknown asset, a non-material asset or an unknown node refuses and
    /// writes nothing (logged).
    pub fn bind_material(
        &mut self,
        id: ObjectId,
        asset: &str,
        bundle: &BundleView,
    ) -> Result<Applied, ShellError> {
        if !self.doc.hierarchy.contains(id) {
            return Err(ShellError::UnknownNode(id));
        }
        let known = bundle
            .find(asset)
            .is_some_and(|entry| entry.kind == AssetKind::Material);
        if !known {
            let error = ShellError::Command(BatchError::BadMaterial { id });
            self.log
                .push(format!("ERR material {id} `{asset}`: {error}"));
            return Err(error);
        }
        let name = match MaterialName::try_from(asset) {
            Ok(name) => name,
            Err(error) => {
                let refused = ShellError::Command(BatchError::BadMaterial { id });
                self.log
                    .push(format!("ERR material {id} `{asset}`: {refused} ({error})"));
                return Err(refused);
            }
        };
        self.apply_ops(vec![Op::SetMaterial {
            id,
            material: Some(name),
        }])
    }

    /// Apply any op list atomically through the command service as a
    /// significant grouped commit (`edit`, durable like the legacy plain
    /// apply). Failures apply nothing and are logged.
    pub fn apply_ops(&mut self, ops: Vec<Op>) -> Result<Applied, ShellError> {
        let batch = Batch {
            ops,
            base_revision: self.doc.revision,
            idempotency_key: self.take_key(),
            provenance: None,
        };
        let meta = CommandMeta::significant("edit");
        match commands::apply_named(&mut self.doc, &mut self.journal, &batch, &meta) {
            Ok(applied) => {
                self.log_applied(&applied);
                Ok(applied)
            }
            Err(error) => {
                self.log.push(format!("ERR apply: {error}"));
                Err(ShellError::Command(error))
            }
        }
    }

    /// Apply an AI/MCP proposal through the shared capability gate:
    /// idempotent replay, staged spatial delta, single-use authorization,
    /// then the atomic command apply. Manual inspector/gizmo edits stay
    /// human-source direct; every token-bearing batch shares this path, so
    /// no transport can skip spatial validation. Failures apply nothing and
    /// are logged.
    pub fn apply_proposal(
        &mut self,
        batch: &Batch,
        cap: &mut Capability,
        approval: &Approval,
        ledger: &mut ApprovalLedger,
    ) -> Result<Applied, CapError> {
        match apply_gated(
            &mut self.doc,
            &mut self.journal,
            batch,
            cap,
            approval,
            ledger,
        ) {
            Ok(applied) => {
                self.log_applied(&applied);
                Ok(applied)
            }
            Err(error) => {
                self.log.push(format!("ERR proposal: {error}"));
                Err(error)
            }
        }
    }

    /// Stage an AI proposal for spatial review: validate it and list its
    /// delta findings. Returns the finding count; failures store nothing.
    /// Behind `ui` with the spatial edge, so headless shells never see it.
    #[cfg(feature = "ui")]
    pub fn stage_proposal(&mut self, batch: &Batch) -> Result<usize, FindingsError> {
        match self.findings.stage(&self.doc, batch) {
            Ok(count) => {
                self.log.push(format!(
                    "staged proposal r{} ({} findings)",
                    self.doc.revision, count
                ));
                Ok(count)
            }
            Err(error) => {
                self.log.push(format!("ERR stage proposal: {error}"));
                Err(error)
            }
        }
    }

    /// Rows currently listed on the staged proposal.
    #[cfg(feature = "ui")]
    pub fn findings_len(&self) -> usize {
        self.findings.len()
    }

    /// One findings row, if the index is live.
    #[cfg(feature = "ui")]
    pub fn finding(&self, index: usize) -> Option<FindingView> {
        self.findings.finding(index)
    }

    /// True when there is nothing appliable: no proposal, or the document
    /// moved past the staged revision.
    #[cfg(feature = "ui")]
    pub fn findings_stale(&self) -> bool {
        self.findings.is_stale(&self.doc)
    }

    /// Drop the staged proposal. Returns the dropped finding count.
    #[cfg(feature = "ui")]
    pub fn refresh_findings(&mut self) -> usize {
        self.findings.refresh()
    }

    /// Preview one automatic fix without side effects.
    #[cfg(feature = "ui")]
    pub fn preview_fix(&self, index: usize) -> Result<Preview, FindingsError> {
        self.findings.preview_fix(&self.doc, index)
    }

    /// Apply one automatic fix through the command service: the proposal ops
    /// plus the repairing move, committed atomically and logged. Stale,
    /// manual and refused rows change nothing and report their typed error.
    #[cfg(feature = "ui")]
    pub fn apply_fix(&mut self, index: usize) -> Result<Applied, FindingsError> {
        let key = self.take_key();
        match self
            .findings
            .apply_fix(&mut self.doc, &mut self.journal, index, key)
        {
            Ok(applied) => {
                self.log_applied(&applied);
                Ok(applied)
            }
            Err(error) => {
                self.log.push(format!("ERR fix {index}: {error}"));
                Err(error)
            }
        }
    }

    /// The live tuning panel: combat/AI drafts, the staged batch and tape.
    #[cfg(feature = "ui")]
    pub fn tuning_panel(&self) -> &TuningPanel {
        &self.tuning
    }

    /// Child tick cursor the next staged batch would apply at.
    #[cfg(feature = "ui")]
    pub fn tune_cursor(&self) -> u32 {
        self.tune_child.cursor()
    }

    /// True when a tune batch is staged against a cursor that moved on.
    #[cfg(feature = "ui")]
    pub fn tune_stale(&self) -> bool {
        self.tuning.is_stale(self.tune_child.cursor())
    }

    /// Advance the tune child one fixed tick (the frame boundary proposals
    /// stage against).
    #[cfg(feature = "ui")]
    pub fn tune_tick(&mut self) {
        self.tune_child.tick();
    }

    /// Edit one combat number through the panel. Invalid values are refused
    /// with their legal range and logged; the draft is unchanged.
    #[cfg(feature = "ui")]
    pub fn tune_set_combat(&mut self, field: CombatField, value: u32) -> Result<(), TuneError> {
        match self.tuning.set_combat(field, value) {
            Ok(()) => {
                self.log.push(format!("tuned combat {field} = {value}"));
                Ok(())
            }
            Err(error) => {
                self.log.push(format!("ERR tune combat: {error}"));
                Err(error)
            }
        }
    }

    /// Edit one aggression lane (`0..=7`, `0..=1000`).
    #[cfg(feature = "ui")]
    pub fn tune_set_aggression(&mut self, lane: usize, value: u16) -> Result<(), TuneError> {
        match self.tuning.set_aggression(lane, value) {
            Ok(()) => {
                self.log
                    .push(format!("tuned ai aggression[{lane}] = {value}"));
                Ok(())
            }
            Err(error) => {
                self.log.push(format!("ERR tune ai: {error}"));
                Err(error)
            }
        }
    }

    /// Edit one caution lane (`0..=7`, `0..=1000`).
    #[cfg(feature = "ui")]
    pub fn tune_set_caution(&mut self, lane: usize, value: u16) -> Result<(), TuneError> {
        match self.tuning.set_caution(lane, value) {
            Ok(()) => {
                self.log.push(format!("tuned ai caution[{lane}] = {value}"));
                Ok(())
            }
            Err(error) => {
                self.log.push(format!("ERR tune ai: {error}"));
                Err(error)
            }
        }
    }

    /// Edit the strike-commit range in centimetres (`50..=6000`).
    #[cfg(feature = "ui")]
    pub fn tune_set_engage(&mut self, cm: u32) -> Result<(), TuneError> {
        match self.tuning.set_engage_range(cm) {
            Ok(()) => {
                self.log.push(format!("tuned ai engage = {cm} cm"));
                Ok(())
            }
            Err(error) => {
                self.log.push(format!("ERR tune ai: {error}"));
                Err(error)
            }
        }
    }

    /// Edit the starting morale (`0..=1000`).
    #[cfg(feature = "ui")]
    pub fn tune_set_morale(&mut self, morale: i32) -> Result<(), TuneError> {
        match self.tuning.set_morale(morale) {
            Ok(()) => {
                self.log.push(format!("tuned ai morale = {morale}"));
                Ok(())
            }
            Err(error) => {
                self.log.push(format!("ERR tune ai: {error}"));
                Err(error)
            }
        }
    }

    /// Preview both drafts without side effects: the validated combat and AI
    /// snapshots for the next versions, logged as one summary line.
    #[cfg(feature = "ui")]
    pub fn tune_preview(&mut self) -> Result<String, TuneError> {
        let combat = match self.tuning.preview_combat() {
            Ok(swap) => swap,
            Err(error) => {
                self.log.push(format!("ERR tune preview: {error}"));
                return Err(error);
            }
        };
        let ai = match self.tuning.preview_ai() {
            Ok(config) => config,
            Err(error) => {
                self.log.push(format!("ERR tune preview: {error}"));
                return Err(error);
            }
        };
        let summary = format!(
            "tune preview: combat v{} (damage {}), ai v{} (engage {} cm, morale {})",
            combat.version(),
            combat.tables().attack_damage,
            ai.curves.version,
            ai.curves.engage_range_cm,
            ai.morale_default,
        );
        self.log.push(summary.clone());
        Ok(summary)
    }

    /// Stage the previewed drafts as a batch against the child cursor,
    /// minting the next rules versions. Failures stage nothing.
    #[cfg(feature = "ui")]
    pub fn tune_stage(&mut self) -> Result<TuneReceipt, TuneError> {
        // The receipt shape is reused as a stage summary (versions minted,
        // cursor recorded); the real apply receipt lands in `tune_apply`.
        let cursor = self.tune_child.cursor();
        match self.tuning.stage(cursor) {
            Ok(staged) => {
                let summary = TuneReceipt {
                    at_tick: staged.base_cursor(),
                    combat_version: staged.combat_version(),
                    ai_version: staged.ai_version(),
                };
                self.log.push(format!(
                    "staged tune @tick {}: combat v{}, ai v{}",
                    summary.at_tick, summary.combat_version, summary.ai_version
                ));
                Ok(summary)
            }
            Err(error) => {
                self.log.push(format!("ERR tune stage: {error}"));
                Err(error)
            }
        }
    }

    /// Apply the staged batch at the child cursor (tick boundary) and
    /// journal it into the tape. A moved cursor refuses with
    /// [`TuneError::StaleChild`]: refresh and re-stage instead.
    #[cfg(feature = "ui")]
    pub fn tune_apply(&mut self) -> Result<TuneReceipt, TuneError> {
        match self.tuning.apply(&mut self.tune_child) {
            Ok(receipt) => {
                self.log.push(format!("applied {receipt}"));
                Ok(receipt)
            }
            Err(error) => {
                self.log.push(format!("ERR tune apply: {error}"));
                Err(error)
            }
        }
    }

    /// Drop the staged tune batch (the stale-child recovery). Returns true
    /// when one was waiting; drafts and the tape are kept.
    #[cfg(feature = "ui")]
    pub fn tune_refresh(&mut self) -> bool {
        let dropped = self.tuning.refresh();
        if dropped {
            self.log.push("tune batch refreshed".to_string());
        }
        dropped
    }

    /// Replay-exactness check over the tune tape: the rules timeline replays
    /// identically twice and the AI decisions under it compare identically
    /// twice.
    #[cfg(feature = "ui")]
    pub fn tune_replay_exact(&self) -> bool {
        self.tuning.replay_exact()
    }

    /// Reserved template items in name order.
    pub fn template_items(&self) -> impl Iterator<Item = &TemplateItem> {
        self.templates.items()
    }

    /// Reserved template item count.
    pub fn template_len(&self) -> usize {
        self.templates.len()
    }

    /// Bounded activity ring (human and agent edits), oldest first.
    pub fn activity(&self) -> &ActivityRing {
        &self.activity
    }

    /// Record one activity row; the tick is the current acknowledged-apply
    /// counter, so the panel orders edits without a wall clock.
    pub fn note_activity(&mut self, author: Author, verb: &'static str, subject: Option<u64>) {
        let tick = self.apply_tick;
        self.activity.push(Activity {
            author,
            verb,
            subject,
            tick,
        });
    }

    /// First-run checklist (tutorial-1), copied for the Templates and empty
    /// viewport bands; the shell owns the live state.
    pub fn onboard(&self) -> Checklist {
        self.onboard
    }

    /// Record one applied template and return the step it completed.
    pub fn note_template(&mut self, kind: TemplateKind) -> Option<Step> {
        self.onboard.note_template(kind)
    }

    /// Record a play start; `Ok(false)` when the scene is not play-ready.
    pub fn note_first_fight(&mut self) -> Result<bool, crate::onboard::OnboardError> {
        self.onboard
            .note_first_fight(TemplateService::play_ready(&self.doc))
    }

    /// Saved prefabs in creation order.
    pub fn prefabs(&self) -> &[Prefab] {
        &self.prefabs
    }

    /// Capture the selected node's subtree as a prefab; a same-name prefab
    /// is replaced. Refuses with no selection or an empty name.
    pub fn save_prefab(&mut self, name: impl Into<String>) -> Result<(), PrefabError> {
        let root = self.selection.ok_or(PrefabError::NoSelection)?;
        let prefab = Prefab::capture(&self.doc, root, name)?;
        let name = prefab.name.clone();
        let nodes = prefab.node_count();
        match self.prefabs.iter().position(|p| p.name == prefab.name) {
            Some(index) => self.prefabs[index] = prefab,
            None => self.prefabs.push(prefab),
        }
        self.log
            .push(format!("prefab `{name}` saved: {nodes} nodes"));
        Ok(())
    }

    /// Place a prefab under the selected node (or the root) offset by `at`
    /// meters. One command batch, so one undo removes the whole copy.
    pub fn place_prefab(&mut self, prefab: &Prefab, at: [f32; 3]) -> Result<Applied, ShellError> {
        let parent = self.selection.unwrap_or(ObjectId::NIL);
        let made = prefab.instantiate(parent, at).map_err(ShellError::Prefab)?;
        let batch = Batch {
            ops: made.ops,
            base_revision: self.doc.revision,
            idempotency_key: self.take_key(),
            provenance: None,
        };
        let meta = CommandMeta::significant(format!("prefab.instantiate {}", prefab.name));
        match commands::apply_named(&mut self.doc, &mut self.journal, &batch, &meta) {
            Ok(applied) => {
                self.record_audit(&batch, &meta, &applied);
                self.log_applied(&applied);
                self.note_activity(Author::Human, "prefab.instantiate", None);
                Ok(applied)
            }
            Err(error) => {
                self.log
                    .push(format!("ERR prefab `{}`: {error}", prefab.name));
                Err(ShellError::Command(error))
            }
        }
    }

    /// Import a `.glb`/`.gltf` file as one node per triangle primitive,
    /// fanned around `at` (the ground point under a drop). One batch, so one
    /// undo removes the whole import; a bad file stages nothing and the
    /// document stays byte-equal.
    pub fn import_glb(
        &mut self,
        path: &str,
        bytes: &[u8],
        at: [f32; 3],
    ) -> Result<Applied, ImportError> {
        let imported = import_gltf_path(path, bytes).map_err(ImportError::Mesh)?;
        if imported.meshes.is_empty() {
            return Err(ImportError::Empty);
        }
        if imported.meshes.len() > MAX_IMPORT_NODES {
            return Err(ImportError::TooMany {
                count: imported.meshes.len(),
            });
        }
        if !at.iter().all(|coordinate| coordinate.is_finite()) {
            return Err(ImportError::NonFinite);
        }
        let stem = import_stem(path);
        let total = imported.meshes.len();
        let mut ops = Vec::with_capacity(total * 2);
        let mut first = None;
        for (index, _mesh) in imported.meshes.iter().enumerate() {
            let name = if total == 1 {
                stem.clone()
            } else {
                format!("{stem}-{index}")
            };
            let asset = AssetName::try_from(name.as_str()).map_err(ImportError::Name)?;
            let id = ObjectId::new();
            if first.is_none() {
                first = Some(id);
            }
            let [offset_x, offset_z] = import_offset(index);
            ops.push(Op::Insert {
                id,
                parent: ObjectId::NIL,
                local: Transform::new(
                    [at[0] + offset_x, at[1], at[2] + offset_z],
                    [0.0, 0.0, 0.0, 1.0],
                    [1.0, 1.0, 1.0],
                ),
            });
            ops.push(Op::SetComponent {
                id,
                component: Component::Mesh(Mesh {
                    asset,
                    cast_shadows: true,
                }),
            });
        }
        let batch = Batch {
            ops,
            base_revision: self.doc.revision,
            idempotency_key: self.take_key(),
            provenance: None,
        };
        let meta = CommandMeta::significant(format!("import {stem}"));
        match commands::apply_named(&mut self.doc, &mut self.journal, &batch, &meta) {
            Ok(applied) => {
                self.record_audit(&batch, &meta, &applied);
                self.log_applied(&applied);
                self.log.push(format!(
                    "import `{path}`: {total} node(s) at [{:.1}, {:.1}]",
                    at[0], at[2]
                ));
                self.note_activity(Author::Human, "import.glb", None);
                if let Some(id) = first {
                    self.select(id);
                }
                Ok(applied)
            }
            Err(error) => {
                self.log.push(format!("ERR import `{path}`: {error}"));
                Err(ImportError::Command(error))
            }
        }
    }

    /// First free name for a kind (`arena`, `arena_2`, ...).
    pub fn template_suggest(&self, kind: TemplateKind) -> String {
        self.templates.default_name(kind)
    }

    /// Preview one template item without side effects: the staged batch
    /// validated against the live document.
    pub fn template_preview(
        &self,
        kind: TemplateKind,
        name: &str,
    ) -> Result<Preview, TemplateError> {
        self.templates.preview(&self.doc, kind, name)
    }

    /// Create one template item through the command service: preview first,
    /// then commit atomically and reserve the name. Failures apply nothing
    /// and are logged.
    pub fn template_apply(
        &mut self,
        kind: TemplateKind,
        name: &str,
    ) -> Result<Applied, TemplateError> {
        let key = self.take_key();
        match self
            .templates
            .apply(&mut self.doc, &mut self.journal, kind, name, key)
        {
            Ok((item, applied)) => {
                self.log.push(format!(
                    "template {kind} `{}`: {} nodes r{}",
                    item.name,
                    item.nodes.len(),
                    applied.revision
                ));
                self.log_applied(&applied);
                self.note_activity(Author::Human, "template.apply", None);
                self.onboard.note_template(kind);
                Ok(applied)
            }
            Err(error) => {
                self.log
                    .push(format!("ERR template {kind} `{name}`: {error}"));
                Err(error)
            }
        }
    }

    /// Undo the newest applied batch as a new revision. Template
    /// reservations survive (redo stays symmetric); call
    /// [`template_reconcile`](Self::template_reconcile) to free items
    /// whose nodes are all gone.
    pub fn template_undo(&mut self) -> Option<Applied> {
        self.undo()
    }

    /// Free template items whose nodes are all gone from the document.
    /// Returns the freed names in name order and logs them.
    pub fn template_reconcile(&mut self) -> Vec<String> {
        let freed = self.templates.reconcile(&self.doc);
        if !freed.is_empty() {
            self.log
                .push(format!("template cleanup freed {}", freed.join(", ")));
        }
        freed
    }

    /// Undo the newest applied batch as a new revision. None when empty.
    pub fn undo(&mut self) -> Option<Applied> {
        let receipt = commands::undo(&mut self.doc, &mut self.journal)?;
        self.log_applied(&receipt);
        Some(receipt)
    }

    /// Redo the newest undone batch as a new revision. None when empty.
    pub fn redo(&mut self) -> Option<Applied> {
        let receipt = commands::redo(&mut self.doc, &mut self.journal)?;
        self.log_applied(&receipt);
        Some(receipt)
    }

    /// Name of the batch the next undo would reverse, or `None`. The menu
    /// labels its button with this so the action is never anonymous (W8).
    pub fn undo_label(&self) -> Option<&str> {
        self.journal.past.last().map(|entry| entry.name.as_str())
    }

    /// Name of the batch the next redo would replay, or `None`.
    pub fn redo_label(&self) -> Option<&str> {
        self.journal.future.last().map(|entry| entry.name.as_str())
    }

    /// Fresh idempotency key for the next batch (never zero).
    fn take_key(&mut self) -> u64 {
        let key = self.next_key.max(1);
        self.next_key = key.wrapping_add(1).max(1);
        key
    }

    /// Feed one applied (or undone/redone) receipt into the log ring.
    fn log_applied(&mut self, applied: &Applied) {
        for line in &applied.effects {
            self.log.push(format!("r{} {line}", applied.revision));
        }
    }
}

impl Default for Shell {
    fn default() -> Self {
        Self::new()
    }
}

/// Sanitized asset stem from a dropped file path: the file name without its
/// `.glb`/`.gltf` extension, bytes outside the asset-name alphabet replaced
/// by `-`, capped so a per-mesh suffix still fits the name bound.
fn import_stem(path: &str) -> String {
    let file = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = match file.rsplit_once('.') {
        Some((stem, extension))
            if extension.eq_ignore_ascii_case("glb") || extension.eq_ignore_ascii_case("gltf") =>
        {
            stem
        }
        _ => file,
    };
    let mut out = String::with_capacity(stem.len());
    for ch in stem.chars() {
        if out.len() >= 48 {
            break;
        }
        let keep = ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.');
        out.push(if keep { ch } else { '-' });
    }
    let trimmed = out.trim_matches(|ch| ch == '-' || ch == '.').to_string();
    if trimmed.is_empty() {
        "import".to_string()
    } else {
        trimmed
    }
}

/// Grid offsets around the drop point: eight columns by eight rows, 0.6 m
/// apart, so a large file fans out instead of stacking every mesh on a pixel.
fn import_offset(index: usize) -> [f32; 2] {
    const STEPS: [f32; 8] = [-2.1, -1.5, -0.9, -0.3, 0.3, 0.9, 1.5, 2.1];
    [STEPS[index % 8], STEPS[(index / 8) % 8]]
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn hierarchy_select_inspector_edit_apply_undo_restores() {
        let (mut shell, _root, child) = seeded_shell();
        let before = shell.document().clone();

        // Hierarchy panel: two rows, child nested one deep.
        let rows = shell.rows();
        assert_eq!(rows.len(), 2);
        let child_row = rows.iter().find(|row| row.id == child).unwrap();
        assert_eq!(child_row.depth, 1);

        // Select the row, edit the inspector draft, stage and apply.
        assert!(shell.select(child));
        assert_eq!(shell.inspector_local(), Some(pose_at(1.0)));
        let preview = shell.stage_transform(pose_at(4.0)).unwrap();
        assert_eq!(preview.effects.len(), 1);
        assert_eq!(preview.resulting_revision, shell.revision() + 1);
        assert_eq!(shell.staged_preview().unwrap().effects, preview.effects);
        let applied = shell.apply_staged().unwrap();
        assert_eq!(applied.revision, shell.revision());
        assert_eq!(shell.document().hierarchy.local(child), Some(pose_at(4.0)));
        // The staged edit clears on apply, and the receipt reached the log.
        assert!(shell.staged_preview().is_none());
        assert!(!shell.log().is_empty());
        assert!(shell.apply_staged().unwrap_err() == ShellError::NothingStaged);

        // Journal undo restores the exact hierarchy; redo reapplies the edit.
        shell.undo().expect("journal holds the inspector batch");
        assert_eq!(shell.document().hierarchy, before.hierarchy);
        assert_eq!(shell.document().hierarchy.local(child), Some(pose_at(1.0)));
        shell.redo().expect("redo holds the inspector batch");
        assert_eq!(shell.document().hierarchy.local(child), Some(pose_at(4.0)));
    }

    #[test]
    fn apply_staged_records_one_audit_receipt_per_acknowledged_apply() {
        let (mut shell, _root, child) = seeded_shell();
        assert!(shell.select(child));
        shell.stage_transform(pose_at(4.0)).unwrap();
        assert!(shell.audit.is_empty());

        // One acknowledged apply chains exactly one receipt.
        let applied = shell.apply_staged().unwrap();
        assert_eq!(applied.revision, shell.revision());
        assert_eq!(shell.audit.len(), 1);
        assert_eq!(shell.audit.total_appended(), 1);
        shell.audit.verify().expect("chain verifies");
        let head = shell.audit.head().expect("receipt retained");
        shell
            .audit
            .verify_pinned(head, 1)
            .expect("pinned head verifies");

        // Undo/redo are journal authority, not acknowledged applies.
        shell.undo().expect("undo holds the inspector batch");
        assert_eq!(shell.audit.total_appended(), 1);

        // A refused apply (nothing staged) appends nothing.
        assert!(shell.apply_staged().is_err());
        assert_eq!(shell.audit.total_appended(), 1);
        // A second acknowledged apply extends the chain, head moves.
        assert!(shell.select(child));
        shell.stage_transform(pose_at(6.0)).unwrap();
        shell.apply_staged().unwrap();
        assert_eq!(shell.audit.total_appended(), 2);
        assert_ne!(shell.audit.head().expect("receipt retained"), head);
        shell.audit.verify().expect("chain still verifies");
    }

    #[test]
    fn staged_invalid_edit_rejected_before_apply() {
        let (mut shell, _root, child) = seeded_shell();
        assert!(shell.select(child));
        let snapshot = shell.document().clone();
        let past = shell.journal().past_len();

        // NaN never reaches the document: staging rejects it first.
        let bad = Transform::new([f32::NAN, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        let error = shell.stage_transform(bad).unwrap_err();
        assert!(matches!(
            error,
            ShellError::Command(BatchError::NonFinite { .. })
        ));
        assert_eq!(*shell.document(), snapshot);
        assert_eq!(shell.journal().past_len(), past);
        // Nothing valid was staged, so there is nothing to apply.
        assert_eq!(shell.apply_staged().unwrap_err(), ShellError::NothingStaged);
        assert_eq!(*shell.document(), snapshot);

        // Unknown selection also stages nothing.
        assert!(!shell.select(ObjectId::new()));
    }

    #[test]
    fn staged_edit_goes_stale_after_another_apply() {
        let (mut shell, _root, child) = seeded_shell();
        assert!(shell.select(child));
        shell.stage_transform(pose_at(7.0)).unwrap();
        // A second writer lands first and moves the revision on.
        shell
            .apply_ops(vec![Op::SetLocal {
                id: child,
                local: pose_at(2.0),
            }])
            .unwrap();
        let snapshot = shell.document().clone();
        let error = shell.apply_staged().unwrap_err();
        assert!(matches!(
            error,
            ShellError::Command(BatchError::StaleRevision { .. })
        ));
        assert_eq!(*shell.document(), snapshot);
    }

    #[test]
    fn gizmo_drag_stages_batch_with_expected_delta_then_undo_restores() {
        use crate::gizmo::GizmoDrag;

        let (mut shell, _root, child) = seeded_shell();
        let before = shell.document().clone();
        assert!(shell.select(child));

        // Translate the selection +3 m on x: the staged preview names it.
        let preview = shell
            .stage_gizmo_drag(&GizmoDrag::Translate {
                delta: [3.0, 0.0, 0.0],
            })
            .unwrap();
        assert_eq!(preview.effects.len(), 1);
        assert_eq!(preview.resulting_revision, shell.revision() + 1);
        let applied = shell.apply_staged().unwrap();
        assert_eq!(applied.revision, shell.revision());
        assert_eq!(shell.document().hierarchy.local(child), Some(pose_at(4.0)));

        // Undo restores the exact hierarchy; redo reapplies the drag.
        shell.undo().expect("journal holds the gizmo batch");
        assert_eq!(shell.document().hierarchy, before.hierarchy);
        shell.redo().expect("redo holds the gizmo batch");
        assert_eq!(shell.document().hierarchy.local(child), Some(pose_at(4.0)));

        // A quarter Y spin premultiplies the local rotation.
        let half = std::f32::consts::FRAC_PI_4;
        shell
            .apply_gizmo_drag(&GizmoDrag::Rotate {
                axis: [0.0, 1.0, 0.0],
                angle_rad: std::f32::consts::FRAC_PI_2,
            })
            .unwrap();
        let spun = shell.document().hierarchy.local(child).unwrap();
        assert!((spun.rotation[1] - half.sin()).abs() <= 1e-5);
        assert!((spun.rotation[3] - half.cos()).abs() <= 1e-5);
        assert!((spun.position[0] - 4.0).abs() <= 1e-5);
        shell.undo().expect("journal holds the spin batch");
        assert_eq!(shell.document().hierarchy.local(child), Some(pose_at(4.0)));
    }

    #[test]
    fn invalid_gizmo_drag_rejected_before_apply() {
        use crate::gizmo::{GizmoDrag, GizmoError};

        let (mut shell, _root, child) = seeded_shell();
        assert!(shell.select(child));
        let snapshot = shell.document().clone();
        let past = shell.journal().past_len();

        for drag in [
            GizmoDrag::Translate {
                delta: [f32::NAN, 0.0, 0.0],
            },
            GizmoDrag::Rotate {
                axis: [0.0, 0.0, 0.0],
                angle_rad: 0.5,
            },
            GizmoDrag::Rotate {
                axis: [0.0, 1.0, 0.0],
                angle_rad: f32::INFINITY,
            },
        ] {
            let error = shell.stage_gizmo_drag(&drag).unwrap_err();
            assert!(
                matches!(error, ShellError::Gizmo(_)),
                "drag {drag:?} rejected as gizmo error"
            );
        }
        assert_eq!(*shell.document(), snapshot);
        assert_eq!(shell.journal().past_len(), past);
        assert_eq!(shell.apply_staged().unwrap_err(), ShellError::NothingStaged);

        // A degenerate absolute world pose is refused the same way.
        let flat = Transform::new([0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [f32::NAN, 1.0, 1.0]);
        assert_eq!(
            shell.stage_gizmo_world(flat).unwrap_err(),
            ShellError::Gizmo(GizmoError::DegenerateTransform)
        );
        assert_eq!(*shell.document(), snapshot);

        // Without a selection the gizmo has no target.
        shell.select(ObjectId::NIL);
        assert_eq!(
            shell
                .stage_gizmo_drag(&GizmoDrag::Translate {
                    delta: [1.0, 0.0, 0.0]
                })
                .unwrap_err(),
            ShellError::NoSelection
        );
    }

    #[test]
    fn gizmo_world_pose_roundtrips_for_nested_selection() {
        let (mut shell, _root, child) = seeded_shell();
        assert!(shell.select(child));
        // Child world starts at x=1; push its world pose +2 on x.
        let mut world = shell.document().hierarchy.world(child).unwrap();
        world.position[0] += 2.0;
        shell.stage_gizmo_world(world).unwrap();
        shell.apply_staged().unwrap();
        let local = shell.document().hierarchy.local(child).unwrap();
        assert!((local.position[0] - 3.0).abs() <= 1e-5);
        // The world pose matches what the gizmo asked for.
        let landed = shell.document().hierarchy.world(child).unwrap();
        assert!((landed.position[0] - world.position[0]).abs() <= 1e-4);
    }

    #[test]
    fn viewport_pick_queries_current_rows_without_mutating() {
        let (shell, root, _child) = seeded_shell();
        let revision = shell.revision();
        let view = shell.viewport;
        let hit = shell.pick(view.size.width / 2.0, view.size.height / 2.0);
        // The root sits at the origin on the center ray; the child at x=1
        // stands a full meter off it, outside its pick sphere.
        assert_eq!(hit, Some(root));
        assert_eq!(shell.revision(), revision);
        assert_eq!(shell.selection, None);
        assert!(shell.staged_preview().is_none());
        assert_eq!(shell.pick(-5.0, -5.0), None);
    }

    #[test]
    fn gated_proposal_applies_and_logs_while_rejected_proposal_changes_nothing() {
        use crate::capability::{approve, OpMask};

        let (mut shell, _root, child) = seeded_shell();
        let before = shell.document().clone();
        let mut cap = Capability::new(7, OpMask::ALL, vec![child.to_string()], 4);
        let mut ledger = ApprovalLedger::new();
        let batch = Batch {
            ops: vec![Op::SetLocal {
                id: child,
                local: pose_at(6.0),
            }],
            base_revision: shell.revision(),
            idempotency_key: 770,
            provenance: None,
        };
        let approval = approve(7, &batch, shell.revision(), shell.revision() + 4);
        let log_before = shell.log().len();
        let applied = shell
            .apply_proposal(&batch, &mut cap, &approval, &mut ledger)
            .unwrap();
        assert_eq!(applied.revision, shell.revision());
        assert_eq!(shell.document().hierarchy.local(child), Some(pose_at(6.0)));
        assert!(shell.log().len() > log_before);
        assert_eq!(cap.remaining(), 3);

        // Out-of-scope target through the same shell path: rejected, logged,
        // document unchanged. The stranger exists (a fresh root) so staging
        // passes and the scope gate is the one that refuses.
        let stranger = ObjectId::new();
        shell
            .insert_node(stranger, ObjectId::NIL, pose_at(20.0))
            .unwrap();
        let bad = Batch {
            ops: vec![Op::SetLocal {
                id: stranger,
                local: pose_at(21.0),
            }],
            base_revision: shell.revision(),
            idempotency_key: 771,
            provenance: None,
        };
        let bad_approval = approve(7, &bad, shell.revision(), shell.revision() + 4);
        let snapshot = shell.document().clone();
        assert_eq!(
            shell.apply_proposal(&bad, &mut cap, &bad_approval, &mut ledger),
            Err(CapError::OutOfScope { id: stranger })
        );
        assert_eq!(*shell.document(), snapshot);
        assert_ne!(*shell.document(), before);
        assert_eq!(cap.remaining(), 3);
    }

    #[test]
    fn log_cap_drops_oldest() {
        let mut ring = LogRing::new(3);
        for i in 0..5 {
            ring.push(format!("m{i}"));
        }
        assert_eq!(ring.len(), 3);
        assert_eq!(ring.dropped(), 2);
        let kept: Vec<&str> = ring.iter().collect();
        assert_eq!(kept, vec!["m2", "m3", "m4"]);
        ring.clear();
        assert!(ring.is_empty());
        assert_eq!(ring.len(), 0);
    }

    #[test]
    fn open_document_resets_journal_and_clears_selection() {
        let (mut shell, _root, child) = seeded_shell();
        assert!(shell.select(child));
        shell.stage_transform(pose_at(9.0)).unwrap();
        let log_before = shell.log().len();

        let mut replacement = Document::new();
        replacement.revision = 41;
        shell.open_document(replacement.clone());
        assert_eq!(*shell.document(), replacement);
        assert_eq!(shell.journal().past_len(), 0);
        assert_eq!(shell.selection, None);
        assert!(shell.staged_preview().is_none());
        assert!(shell.log().len() > log_before);
        assert!(shell.undo().is_none(), "fresh journal has nothing to undo");
    }

    #[test]
    fn panels_open_close_and_remove_clears_selection() {
        let (mut shell, root, _child) = seeded_shell();
        assert!(shell.is_open(Panel::Inspector));
        assert!(shell.close_panel(Panel::Inspector));
        assert!(!shell.is_open(Panel::Inspector));
        assert!(!shell.close_panel(Panel::Inspector));
        shell.open_panel(Panel::Inspector);
        shell.open_panel(Panel::Inspector);
        assert!(shell.is_open(Panel::Inspector));

        assert!(shell.select(root));
        shell.remove_node(root).unwrap();
        assert_eq!(shell.selection, None);
        assert!(shell.staged_preview().is_none());
    }

    #[test]
    fn snapshot_rows_are_plain_sorted_data() {
        let mut shell = Shell::new();
        let b = ObjectId::new();
        let a = ObjectId::new();
        // Insert in reverse id-string order; rows still sort deterministically.
        let (first, second) = if a.to_string() < b.to_string() {
            (a, b)
        } else {
            (b, a)
        };
        shell
            .insert_node(second, ObjectId::NIL, pose_at(2.0))
            .unwrap();
        shell
            .insert_node(first, ObjectId::NIL, pose_at(1.0))
            .unwrap();
        let rows = shell.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, first);
        assert_eq!(rows[1].id, second);
        assert!(rows.iter().all(|row| row.depth == 0));
        // Re-snapshotting the same document gives the same rows.
        assert_eq!(shell.rows(), rows);
    }

    #[test]
    fn templates_create_through_the_command_service_with_log_and_undo() {
        use crate::templates::TemplateError;

        let mut shell = Shell::new();
        assert_eq!(shell.template_len(), 0);
        assert_eq!(
            shell.template_suggest(TemplateKind::Arena),
            "arena".to_string()
        );

        // Preview stages a validated batch without side effects.
        let before = shell.document().clone();
        let preview = shell
            .template_preview(TemplateKind::Arena, "arena")
            .unwrap();
        assert_eq!(preview.effects.len(), 6);
        assert_eq!(*shell.document(), before);

        // Create commits through the command service and logs the receipt.
        let log_before = shell.log().len();
        let applied = shell.template_apply(TemplateKind::Arena, "arena").unwrap();
        assert_eq!(applied.revision, shell.revision());
        assert_eq!(shell.template_len(), 1);
        assert_eq!(shell.document().hierarchy.len(), 6);
        assert!(shell.log().len() > log_before);
        shell.template_apply(TemplateKind::Prop, "prop").unwrap();
        assert_eq!(shell.document().hierarchy.len(), 7);

        // A taken name refuses with its suggestion and changes nothing.
        let snapshot = shell.document().clone();
        let error = shell
            .template_apply(TemplateKind::Prop, "prop")
            .unwrap_err();
        assert_eq!(
            error,
            TemplateError::NameTaken {
                name: "prop".to_string(),
                suggested: "prop_2".to_string(),
            }
        );
        assert_eq!(*shell.document(), snapshot);

        // Undo restores the prop nodes; the reservation survives for redo
        // until an explicit cleanup frees it.
        shell.template_undo().expect("template batch undoes");
        assert_eq!(shell.document().hierarchy.len(), 6);
        assert_eq!(shell.template_len(), 2);
        assert_eq!(shell.template_reconcile(), vec!["prop".to_string()]);
        assert_eq!(shell.template_len(), 1);
        assert!(shell.template_reconcile().is_empty());
        shell.redo().expect("redo holds the prop batch");
        assert_eq!(shell.document().hierarchy.len(), 7);
    }

    /// The empty-scene band's one click: arena, fighter and weapon applied in
    /// order leave a play-ready scene, tick the first three checklist steps,
    /// and the first fight ticks the fourth.
    #[test]
    fn one_click_starters_reach_a_play_ready_scene() {
        let mut shell = Shell::new();
        assert_eq!(shell.onboard().done_count(), 0);
        for kind in [
            TemplateKind::Arena,
            TemplateKind::Fighter,
            TemplateKind::Weapon,
        ] {
            let name = shell.template_suggest(kind);
            shell.template_apply(kind, &name).unwrap();
        }
        assert!(TemplateService::play_ready(shell.document()));
        assert_eq!(shell.onboard().done_count(), 3);
        assert!(!shell.onboard().is_complete());
        assert!(shell.note_first_fight().unwrap());
        assert!(shell.onboard().is_complete());
    }

    /// A prefab copies the subtree's poses, components and material binding;
    /// placing it is one batch, and one undo removes the whole copy.
    #[test]
    fn prefab_round_trip_copies_components_and_undoes() {
        use engine_scene::component::{Component, ComponentKind, Light, LightKind};

        let mut shell = Shell::new();
        shell.template_apply(TemplateKind::Arena, "arena").unwrap();
        let root = shell
            .template_items()
            .find(|item| item.name == "arena")
            .expect("arena reservation")
            .nodes[0];
        assert!(shell.select(root));
        shell
            .doc
            .components
            .entry(root.to_string())
            .or_default()
            .insert(
                ComponentKind::Light,
                Component::Light(Light {
                    kind: LightKind::Point,
                    color: [1.0, 0.5, 0.25],
                    intensity: 3.0,
                    range: 6.0,
                }),
            );
        let root_local = shell.doc.hierarchy.local(root).expect("root pose");
        shell.save_prefab("lamp").expect("captures the selection");
        assert_eq!(shell.prefabs().len(), 1);
        let prefab = shell.prefabs()[0].clone();
        let nodes = prefab.node_count();

        let before = shell.doc.hierarchy.len();
        let before_ids: std::collections::BTreeSet<_> = shell
            .doc
            .hierarchy
            .world_all()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        shell
            .place_prefab(&prefab, [10.0, 0.0, 0.0])
            .expect("places the prefab");
        assert_eq!(shell.doc.hierarchy.len(), before + nodes);

        let new_root = shell
            .doc
            .hierarchy
            .world_all()
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| !before_ids.contains(id))
            .find(|id| {
                shell.doc.hierarchy.local(*id).is_some_and(|local| {
                    (local.position[0] - (root_local.position[0] + 10.0)).abs() < 1e-5
                        && (local.position[1] - root_local.position[1]).abs() < 1e-5
                })
            })
            .expect("new root carries the offset");
        let copied = shell
            .doc
            .components
            .get(&new_root.to_string())
            .and_then(|table| table.get(&ComponentKind::Light));
        assert!(matches!(
            copied,
            Some(Component::Light(light)) if light.intensity == 3.0
        ));

        // Editing one copy leaves the source untouched.
        if let Some(table) = shell.doc.components.get_mut(&new_root.to_string()) {
            if let Some(Component::Light(light)) = table.get_mut(&ComponentKind::Light) {
                light.intensity = 9.0;
            }
        }
        let source = shell
            .doc
            .components
            .get(&root.to_string())
            .and_then(|table| table.get(&ComponentKind::Light));
        assert!(matches!(
            source,
            Some(Component::Light(light)) if light.intensity == 3.0
        ));

        shell.undo().expect("one undo removes the whole copy");
        assert_eq!(shell.doc.hierarchy.len(), before);
        assert!(!shell.doc.hierarchy.contains(new_root));
    }

    /// EI3: a real glTF becomes nodes at the drop point in one batch; a bad
    /// file leaves the document byte-equal.
    #[test]
    fn dropped_gltf_imports_nodes_and_a_bad_file_changes_nothing() {
        use engine_scene::component::ComponentKind;

        const TRIANGLE: &[u8] = include_bytes!("../../../fixtures/khronos/Triangle.gltf");
        let mut shell = Shell::new();
        let applied = shell
            .import_glb("triangle.gltf", TRIANGLE, [3.0, 0.0, 4.0])
            .expect("the fixture imports");
        assert_eq!(shell.document().hierarchy.len(), 1);
        assert_eq!(applied.revision, 1);
        let selected = shell.selection.expect("import selects its first node");
        let key = selected.to_string();
        let mesh = shell
            .document()
            .components
            .get(&key)
            .and_then(|table| table.get(&ComponentKind::Mesh));
        assert!(matches!(
            mesh,
            Some(Component::Mesh(mesh)) if mesh.asset.as_str() == "triangle"
        ));
        let local = shell
            .document()
            .hierarchy
            .local(selected)
            .expect("node pose");
        assert_eq!(local.position[0], 3.0 + import_offset(0)[0]);
        assert_eq!(local.position[2], 4.0 + import_offset(0)[1]);

        let after_import = shell.document().clone();
        let refused = shell.import_glb("broken.glb", b"not a gltf", [0.0, 0.0, 0.0]);
        assert!(matches!(
            refused,
            Err(ImportError::Mesh(MeshImportError::Parse { .. }))
        ));
        assert_eq!(*shell.document(), after_import, "bad file changes nothing");

        shell.undo().expect("one undo removes the import");
        assert!(shell.document().hierarchy.is_empty());
    }

    /// Fixed ids so two shells can compare end digests bit-exactly: random
    /// ids would differ by construction.
    fn fixed_shell() -> (Shell, ObjectId, ObjectId) {
        let mut shell = Shell::new();
        let root =
            ObjectId::parse("11111111-2222-3333-4444-555555555555").expect("fixture id parses");
        let child =
            ObjectId::parse("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").expect("fixture id parses");
        shell
            .insert_node(root, ObjectId::NIL, pose_at(0.0))
            .unwrap();
        shell.insert_node(child, root, pose_at(1.0)).unwrap();
        (shell, root, child)
    }

    #[test]
    fn gizmo_drag_session_is_one_entry_with_same_digest() {
        use crate::commands::{durable_count, needs_persist, NO_GROUP};
        use crate::gizmo::GizmoDrag;

        const STEPS: usize = 20;
        const STEP_M: f32 = 0.25;

        let (mut shell, _root, child) = fixed_shell();
        assert!(shell.select(child));
        let start = shell.document().hierarchy.clone();
        let (base_past, base_durable) =
            (shell.journal().past_len(), durable_count(shell.journal()));

        // Twenty viewport ticks inside one session.
        let group = shell.begin_gizmo_drag().unwrap();
        assert_ne!(group, NO_GROUP);
        assert!(shell.gizmo_drag_open());
        // Re-beginning while live returns the open group, not a new one.
        assert_eq!(shell.begin_gizmo_drag().unwrap(), group);
        for _ in 0..STEPS {
            shell
                .tick_gizmo_drag(&GizmoDrag::Translate {
                    delta: [STEP_M, 0.0, 0.0],
                })
                .unwrap();
        }
        // Mid-drag every tick is undoable but seals nothing durable.
        assert_eq!(shell.journal().past_len(), base_past + STEPS);
        assert_eq!(durable_count(shell.journal()), base_durable);
        assert!(
            shell
                .journal()
                .past
                .iter()
                .skip(base_past)
                .all(|entry| !needs_persist(entry)),
            "drag steps skip the durable journal"
        );

        // Release folds the drag into one significant entry.
        let merged = shell.end_gizmo_drag().expect("drag commits one entry");
        assert!(!shell.gizmo_drag_open());
        assert_eq!(shell.journal().past_len(), base_past + 1);
        assert_eq!(durable_count(shell.journal()), base_durable + 1);
        let entry = shell.journal().past.last().expect("merged entry");
        assert_eq!(entry.name, "gizmo drag");
        assert!(entry.significant && needs_persist(entry));
        assert_eq!(merged.key, entry.key);
        let landed = shell.document().hierarchy.local(child).unwrap();
        assert!((landed.position[0] - 6.0).abs() <= 1e-5);
        // Releasing twice commits nothing.
        assert!(shell.end_gizmo_drag().is_none());
        assert!(!shell.cancel_gizmo_drag());

        // Reference: the same drag as ungrouped single-shot applies.
        let (mut plain, _root, plain_child) = fixed_shell();
        assert!(plain.select(plain_child));
        for _ in 0..STEPS {
            plain
                .apply_gizmo_drag(&GizmoDrag::Translate {
                    delta: [STEP_M, 0.0, 0.0],
                })
                .unwrap();
        }
        // Was N entries; the session keeps one (95% fewer here).
        assert_eq!(plain.journal().past_len(), base_past + STEPS);
        assert_eq!(shell.document().hierarchy, plain.document().hierarchy);

        // One undo restores the drag start; redo replays it exactly.
        shell.undo().expect("drag undoes in one step");
        assert_eq!(shell.document().hierarchy, start);
        shell.redo().expect("drag redoes in one step");
        assert_eq!(shell.document().hierarchy, plain.document().hierarchy);
    }

    #[test]
    fn inspector_staged_commit_is_significant_and_exact() {
        use crate::commands::{durable_count, needs_persist};

        let (mut shell, _root, child) = fixed_shell();
        assert!(shell.select(child));
        let before = shell.document().hierarchy.clone();
        let (base_past, base_durable) =
            (shell.journal().past_len(), durable_count(shell.journal()));

        shell.stage_transform(pose_at(4.0)).unwrap();
        let applied = shell.apply_staged().unwrap();
        assert_eq!(applied.revision, shell.revision());
        assert_eq!(shell.journal().past_len(), base_past + 1);
        assert_eq!(durable_count(shell.journal()), base_durable + 1);
        let entry = shell.journal().past.last().expect("inspector entry");
        assert_eq!(entry.name, "inspector edit");
        assert!(entry.significant && needs_persist(entry));
        assert_eq!(shell.document().hierarchy.local(child), Some(pose_at(4.0)));

        shell.undo().expect("inspector batch undoes");
        assert_eq!(shell.document().hierarchy, before);
        shell.redo().expect("inspector batch redoes");
        assert_eq!(shell.document().hierarchy.local(child), Some(pose_at(4.0)));
    }

    #[test]
    fn gizmo_session_failures_write_nothing_and_release_anyway() {
        use crate::commands::durable_count;
        use crate::gizmo::GizmoDrag;

        // A session needs a live selection first.
        let mut shell = Shell::new();
        assert_eq!(
            shell.begin_gizmo_drag().unwrap_err(),
            ShellError::NoSelection
        );

        let (mut shell, _root, child) = fixed_shell();
        assert!(shell.select(child));
        let snapshot = shell.document().clone();
        let (base_past, base_durable) =
            (shell.journal().past_len(), durable_count(shell.journal()));
        shell.begin_gizmo_drag().unwrap();

        // A NaN tick is refused and the session survives it.
        let error = shell
            .tick_gizmo_drag(&GizmoDrag::Translate {
                delta: [f32::NAN, 0.0, 0.0],
            })
            .unwrap_err();
        assert!(matches!(error, ShellError::Gizmo(_)));
        assert_eq!(*shell.document(), snapshot);
        assert_eq!(shell.journal().past_len(), base_past);
        assert_eq!(durable_count(shell.journal()), base_durable);
        assert!(shell.gizmo_drag_open());

        // The good tick still lands and releases into one entry.
        shell
            .tick_gizmo_drag(&GizmoDrag::Translate {
                delta: [2.0, 0.0, 0.0],
            })
            .unwrap();
        let merged = shell.end_gizmo_drag().expect("steps still merge");
        assert_eq!(shell.journal().past_len(), base_past + 1);
        assert_eq!(merged.key, shell.journal().past.last().unwrap().key);
        shell.undo().expect("merged drag undoes");
        assert_eq!(shell.document().hierarchy, snapshot.hierarchy);

        // A bare tick without an explicit begin still groups.
        assert!(shell.select(child));
        let base = shell.journal().past_len();
        shell
            .tick_gizmo_drag(&GizmoDrag::Translate {
                delta: [1.0, 0.0, 0.0],
            })
            .unwrap();
        assert!(shell.gizmo_drag_open());
        shell.end_gizmo_drag().expect("auto-opened drag commits");
        assert_eq!(shell.journal().past_len(), base + 1);
    }

    #[test]
    fn cancelled_drag_keeps_steps_individual() {
        use crate::gizmo::GizmoDrag;

        let (mut shell, _root, child) = fixed_shell();
        assert!(shell.select(child));
        let base = shell.journal().past_len();
        shell.begin_gizmo_drag().unwrap();
        for _ in 0..2 {
            shell
                .tick_gizmo_drag(&GizmoDrag::Translate {
                    delta: [1.0, 0.0, 0.0],
                })
                .unwrap();
        }
        assert!(shell.cancel_gizmo_drag());
        assert!(!shell.gizmo_drag_open());
        assert_eq!(shell.journal().past_len(), base + 2);
        assert_eq!(shell.document().hierarchy.local(child), Some(pose_at(3.0)));
        // Each step still undoes on its own.
        shell.undo().unwrap();
        assert_eq!(shell.document().hierarchy.local(child), Some(pose_at(2.0)));
    }

    #[test]
    fn shell_caller_drives_auto_grid_with_undo_and_range_refusals() {
        use crate::inspector_auto::AutoError;

        // No selection: no grid, edits refuse without writing.
        let mut shell = Shell::new();
        assert!(shell.auto_grid().is_none());
        assert!(matches!(
            shell.auto_edit("pose", "pose.px", "1.0").unwrap_err(),
            AutoError::UnknownField { .. }
        ));

        // Selected object: the shell builds all 20 auto rows itself.
        let (mut shell, _root, child) = seeded_shell();
        assert!(shell.select(child));
        let grid = shell.auto_grid().expect("selection builds the auto grid");
        assert_eq!(grid.len(), 20);
        assert!(grid.covers_all_kinds());
        let lane = grid
            .rows()
            .iter()
            .find(|row| row.component == "pose" && row.field == "pose.px")
            .expect("grid renders the pose lane");
        assert_eq!((lane.min.as_str(), lane.max.as_str()), ("-10000", "10000"));

        // In-range edit routes through command batches with undo/redo.
        let applied = shell.auto_edit("pose", "pose.px", "4.5").unwrap();
        assert_eq!(applied.revision, shell.revision());
        let local = shell.document().hierarchy.local(child).unwrap();
        assert!((local.position[0] - 4.5).abs() <= 1e-5);
        shell.undo().expect("journal holds the auto batch");
        assert_eq!(shell.document().hierarchy.local(child), Some(pose_at(1.0)));
        shell.redo().expect("redo holds the auto batch");
        assert!((shell.document().hierarchy.local(child).unwrap().position[0] - 4.5).abs() <= 1e-5);

        // Out-of-range edit refuses naming the range and writes nothing.
        let before = shell.document().clone();
        let log_before = shell.log().len();
        let error = shell.auto_edit("pose", "pose.px", "20000").unwrap_err();
        assert!(matches!(error, AutoError::BadValue { .. }));
        assert!(
            error.to_string().contains("-10000..=10000"),
            "refusal quotes the range, got: {error}"
        );
        assert_eq!(*shell.document(), before);
        assert!(shell.log().len() > log_before);
    }
}
