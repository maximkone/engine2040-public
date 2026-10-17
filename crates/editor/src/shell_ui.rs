//! egui dock drawing over [`Shell`](crate::shell::Shell).
//!
//! The consumer is an editor window host: call [`ShellUi::show`]
//! once per frame with the frame's root `egui::Ui` (egui 0.36 panels draw
//! inside a `Ui`; the host owns the window, the present loop and its
//! central panel). This module draws the hierarchy, inspector, viewport and
//! log panels and routes every gesture back through [`Shell`] batch
//! entrypoints, so the command service stays the only write path. The
//! viewport panel offers click-to-pick plus a `transform-gizmo` session over
//! the selection; every gizmo result stages a batch (preview) before Apply
//! commits it. Windowed runs and captures stay with the window host.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use egui_dock::widgets::tab_viewer::OnCloseResponse;
use egui_dock::{DockArea, DockState, TabViewer};

use engine_ai::CURVE_LANES;
use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;

use crate::commands::Applied;
use crate::gizmo::{ui::GizmoSession, GizmoTool};
use crate::inspector::{
    self, InspectorGrid, InspectorPanel, NodeView, SchemaStep, VersionedComponent,
};
use crate::outline::{
    self, OutlinerHandles, OutlinerRow, OutlinerView, Selection as SharedSelection, SelectionBank,
};
use crate::shell::{Panel, Shell, ShellError};
use crate::templates::TemplateKind;
use crate::tuning::CombatField;
use crate::viewport::ViewportSize;

/// Dock layout plus per-frame drawing over a [`Shell`].
/// PORT(Adanos020/egui_dock@cdcdb359af4b74f8cc7921091db7217f0759e4f7 dock_state):
/// dock stays the layout owner; egui_tiles behavior plus simple example
/// informed the choice only and no tile code lands.
pub struct ShellUi {
    dock: DockState<Panel>,
    gizmo: GizmoSession,
    viewport_tex: Option<egui::TextureHandle>,
    /// Last fix preview per finding row, shown under its row until applied,
    /// refreshed or cleared. Preview output only, never a staged write.
    fix_preview: Option<(usize, crate::commands::Preview)>,
    /// Last replay-exactness verdict from the Tuning tab button.
    tune_replay: Option<bool>,
    /// Selected template kind for the Templates tab.
    template_kind: TemplateKind,
    /// Item name draft for the Templates tab.
    template_name: String,
    /// Last template preview, shown until created, undone or cleared.
    template_preview: Option<crate::commands::Preview>,
    /// Last template notice (created, refused, freed) for the tab.
    template_status: Option<String>,
    /// Outliner search text for the hierarchy tab (substring over row names).
    outliner_query: String,
    /// Outliner rows and hits cached per document revision: a quiet frame
    /// rebuilds nothing, whatever the scene size.
    outline: OutlinerView,
    /// Shared live selection over stable row ids; viewport, outliner and
    /// inspector agree on this one object.
    shared_selection: SharedSelection,
    /// Named selection sets for the hierarchy tab.
    selection_sets: SelectionBank,
    /// Draft set name for save and recall.
    set_name: String,
    /// Staged inspector edits over the grid (validate then emit).
    inspector_panel: InspectorPanel,
    /// Next idempotency key for inspector batches (never zero).
    inspector_key: u64,
}

impl ShellUi {
    /// Dock layout seeded from the shell's open panels.
    pub fn new(shell: &Shell) -> Self {
        let shared_selection = shell
            .selection
            .map(|id| outline::select_one(outline::stable_id(id)))
            .unwrap_or_default();
        Self {
            dock: DockState::new(shell.open_panels.clone()),
            gizmo: GizmoSession::new(),
            viewport_tex: None,
            fix_preview: None,
            tune_replay: None,
            template_kind: TemplateKind::default(),
            template_name: String::new(),
            template_preview: None,
            template_status: None,
            outliner_query: String::new(),
            outline: OutlinerView::new(),
            shared_selection,
            selection_sets: SelectionBank::new(),
            set_name: String::new(),
            inspector_panel: InspectorPanel::new(),
            inspector_key: 1,
        }
    }

    /// Current gizmo tool offered in the viewport panel.
    pub fn gizmo_tool(&self) -> GizmoTool {
        self.gizmo.tool()
    }

    /// Draw the dock area into the host's root [`egui::Ui`], without a live
    /// viewport texture (headless frames and tests). Tabs opened through
    /// [`Shell::open_panel`] are added to the first leaf; tab closes report
    /// back to [`Shell::close_panel`] so the shell stays authoritative.
    pub fn show(&mut self, shell: &mut Shell, ui: &mut egui::Ui) {
        self.viewport_tex = None;
        self.show_inner(shell, ui);
    }

    /// Draw the dock area with the live render-validated viewport texture shown
    /// at the top of the viewport panel. The texture is owned by the host
    /// (refreshed when the shell revision or panel size moves); the panel
    /// only borrows it for this frame.
    pub fn show_with_viewport(
        &mut self,
        shell: &mut Shell,
        ui: &mut egui::Ui,
        viewport: Option<&egui::TextureHandle>,
    ) {
        // `TextureHandle` clones cheaply (an id plus an Arc); the viewer
        // borrows it for the frame without taking ownership from the host.
        self.viewport_tex = viewport.cloned();
        self.show_inner(shell, ui);
        self.viewport_tex = None;
    }

    fn show_inner(&mut self, shell: &mut Shell, ui: &mut egui::Ui) {
        crate::shell::panels::ensure_builtin_panels(shell);
        // Borrow the open list: the loop only reads it while the dock takes
        // the missing tabs, so no per-frame Vec clone.
        for panel in shell.open_panels.iter().copied() {
            let present = self.dock.iter_all_tabs().any(|(_, tab)| *tab == panel);
            if !present {
                self.dock.push_to_first_leaf(panel);
            }
        }
        if let Ok((rows, live)) = self
            .outline
            .sync(shell.revision(), || live_outliner_rows(shell))
        {
            reconcile_shared_selection(shell, &mut self.shared_selection, &rows, &live);
        }
        let (gizmo, dock, viewport_tex, fix_preview, tune_replay) = (
            &mut self.gizmo,
            &mut self.dock,
            self.viewport_tex.clone(),
            &mut self.fix_preview,
            &mut self.tune_replay,
        );
        let (template_kind, template_name, template_preview, template_status) = (
            &mut self.template_kind,
            &mut self.template_name,
            &mut self.template_preview,
            &mut self.template_status,
        );
        let (outliner_query, outline, shared_selection, selection_sets, set_name) = (
            &mut self.outliner_query,
            &mut self.outline,
            &mut self.shared_selection,
            &mut self.selection_sets,
            &mut self.set_name,
        );
        let (inspector_panel, inspector_key) = (&mut self.inspector_panel, &mut self.inspector_key);
        let mut viewer = Viewer {
            shell,
            gizmo,
            viewport_tex,
            fix_preview,
            tune_replay,
            template_kind,
            template_name,
            template_preview,
            template_status,
            outliner_query,
            outline,
            shared_selection,
            selection_sets,
            set_name,
            inspector_panel,
            inspector_key,
        };
        DockArea::new(dock).show_inside(ui, &mut viewer);
    }
}

/// [`TabViewer`] borrowing the shell and the gizmo session; one method per panel.
struct Viewer<'a> {
    shell: &'a mut Shell,
    gizmo: &'a mut GizmoSession,
    viewport_tex: Option<egui::TextureHandle>,
    fix_preview: &'a mut Option<(usize, crate::commands::Preview)>,
    tune_replay: &'a mut Option<bool>,
    template_kind: &'a mut TemplateKind,
    template_name: &'a mut String,
    template_preview: &'a mut Option<crate::commands::Preview>,
    template_status: &'a mut Option<String>,
    outliner_query: &'a mut String,
    outline: &'a mut OutlinerView,
    shared_selection: &'a mut SharedSelection,
    selection_sets: &'a mut SelectionBank,
    set_name: &'a mut String,
    inspector_panel: &'a mut InspectorPanel,
    inspector_key: &'a mut u64,
}

impl TabViewer for Viewer<'_> {
    type Tab = Panel;

    fn id(&mut self, tab: &mut Self::Tab) -> egui::Id {
        egui::Id::new(*tab)
    }

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        egui::WidgetText::from(tab.title())
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Self::Tab) {
        match tab {
            Panel::Hierarchy => self.hierarchy(ui),
            Panel::Inspector => self.inspector(ui),
            Panel::Viewport => self.viewport_panel(ui),
            Panel::Findings => self.findings_panel(ui),
            Panel::Tuning => self.tuning_panel(ui),
            Panel::Templates => self.templates_panel(ui),
            Panel::Log => self.log(ui),
        }
    }

    fn on_close(&mut self, tab: &mut Self::Tab) -> OnCloseResponse {
        self.shell.close_panel(*tab);
        OnCloseResponse::Close
    }
}

impl Viewer<'_> {
    /// Cached outliner rows and live ids for the shell's current revision.
    fn outline_handles(&mut self) -> Result<OutlinerHandles, outline::OutlineError> {
        self.outline
            .sync(self.shell.revision(), || live_outliner_rows(self.shell))
    }

    /// Hierarchy tab over [`outline`]: search plus named sets over one selection.
    ///
    /// Rows and hits come from the revision-cached [`OutlinerView`]; clicks
    /// join [`SharedSelection`]; sets persist in [`SelectionBank`]. The hit
    /// list draws last and only the rows in view, so the frame costs the same
    /// at 20 rows and at 2,000. Add and remove go through shell batches,
    /// never direct writes.
    fn hierarchy(&mut self, ui: &mut egui::Ui) {
        let (rows, live) = match self.outline_handles() {
            Ok(handles) => handles,
            Err(error) => {
                ui.label(format!("outliner refused: {error}"));
                return;
            }
        };
        outline::prune(self.shared_selection, &live);
        ui.horizontal(|ui| {
            ui.label("search");
            ui.text_edit_singleline(self.outliner_query);
        });
        let hits = match self.outline.search(self.outliner_query) {
            Ok(hits) => hits,
            Err(error) => {
                ui.label(format!("search refused: {error}"));
                return;
            }
        };
        if rows.is_empty() {
            ui.label("Empty scene: Add root to begin.");
        }
        self.hierarchy_sets(ui, &rows, &live);
        self.hierarchy_add_remove(ui);
        ui.separator();
        let row_height = ui.spacing().interact_size.y;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_height, hits.len(), |ui, range| {
                for hit in &hits[range] {
                    let selected = self.shared_selection.ids.contains(&hit.id);
                    let indent = f32::from(u16::try_from(hit.depth.min(16)).unwrap_or(16)) * 16.0;
                    ui.horizontal(|ui| {
                        ui.add_space(indent);
                        if ui.selectable_label(selected, hit.name.clone()).clicked() {
                            *self.shared_selection = outline::select_one(hit.id);
                            sync_shell_to_shared(self.shell, self.shared_selection, &rows);
                        }
                        if ui.button("toggle").clicked() {
                            if let Ok(next) =
                                outline::select_toggle(self.shared_selection, hit.id, &live)
                            {
                                *self.shared_selection = next;
                                sync_shell_to_shared(self.shell, self.shared_selection, &rows);
                            }
                        }
                    });
                }
            });
    }

    /// Named selection sets over the shared selection; refusals land in the log.
    fn hierarchy_sets(&mut self, ui: &mut egui::Ui, rows: &[OutlinerRow], live: &BTreeSet<u64>) {
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("set");
            ui.text_edit_singleline(self.set_name);
            if ui.button("save").clicked() {
                let name = self.set_name.clone();
                if let Err(error) =
                    self.selection_sets
                        .save(&name, self.shared_selection.clone(), live)
                {
                    self.shell
                        .log_message(format!("ERR set save `{name}`: {error}"));
                }
            }
            if ui.button("recall").clicked() {
                let name = self.set_name.clone();
                match self.selection_sets.recall(&name, self.shared_selection) {
                    Ok(next) => {
                        *self.shared_selection = next;
                        sync_shell_to_shared(self.shell, self.shared_selection, rows);
                    }
                    Err(error) => {
                        self.shell
                            .log_message(format!("ERR set recall `{name}`: {error}"));
                    }
                }
            }
            if ui.button("drop").clicked() {
                let name = self.set_name.clone();
                if !self.selection_sets.remove(&name) {
                    self.shell
                        .log_message(format!("ERR set drop `{name}`: unknown set"));
                }
            }
        });
        ui.label(format!("sets: {}", self.selection_sets.names().join(", ")));
    }

    /// Add and remove through shell batches; the parent follows the shared primary.
    fn hierarchy_add_remove(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Add root").clicked() {
                let parent = self
                    .shell
                    .selection
                    .filter(|id| self.shell.document().hierarchy.contains(*id))
                    .unwrap_or(ObjectId::NIL);
                let _ = self
                    .shell
                    .insert_node(ObjectId::new(), parent, Transform::identity());
            }
            if ui.button("Remove selected").clicked() {
                if let Some(id) = self.shell.selection {
                    let _ = remove_closing_drag(self.shell, id);
                }
            }
        });
    }

    /// Inspector tab over the [`inspector`] grid for the shared selection.
    ///
    /// Views come from [`node_views_for_rows`], rows from
    /// [`inspector::build_grid`]; pose cells stage through the panel and
    /// fold into the command batch, so viewport, outliner and inspector
    /// agree on the identical live selection object.
    fn inspector(&mut self, ui: &mut egui::Ui) {
        let (rows, live) = match self.outline_handles() {
            Ok(handles) => handles,
            Err(error) => {
                ui.label(format!("outliner refused: {error}"));
                return;
            }
        };
        outline::prune(self.shared_selection, &live);
        if self.shared_selection.ids.is_empty() {
            ui.label("Select a row in Hierarchy.");
            return;
        }
        if self.inspector_panel.is_empty() {
            self.inspector_panel.rebase(self.shell.revision());
        }
        let views = node_views_for_rows(self.shell, &rows);
        let baseline: BTreeMap<(u64, String, String), String> = BTreeMap::new();
        let grid = match inspector::build_grid(&views, self.shared_selection, &baseline) {
            Ok(grid) => grid,
            Err(error) => {
                ui.label(format!("inspector refused: {error}"));
                return;
            }
        };
        ui.label(format!(
            "grid {} rows ({} overrides)",
            grid.len(),
            grid.overrides().len()
        ));
        self.inspector_grid_rows(ui, &grid);
        self.inspector_apply_bar(ui, &grid, &rows);
    }

    /// Grid rows with per-cell staging; read-only cells show their reason.
    fn inspector_grid_rows(&mut self, ui: &mut egui::Ui, grid: &InspectorGrid) {
        let staged = self.inspector_panel.staged_edits();
        let staged_map: BTreeMap<(u64, String, String), String> = staged.into_iter().collect();
        for row in grid.rows() {
            let key = (row.node, row.component.clone(), row.field.clone());
            let badge = row.badge().unwrap_or("-");
            ui.horizontal(|ui| {
                ui.label(format!(
                    "{}.{}.{} [{}]",
                    row.component, row.field, row.node, badge
                ));
                if row.kind.is_read_only() {
                    ui.label(row.value.clone());
                    return;
                }
                let current = staged_map
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| row.value.clone());
                let mut draft = current.clone();
                ui.text_edit_singleline(&mut draft);
                if draft != current {
                    let _ = self.inspector_panel.stage(
                        grid,
                        row.node,
                        &row.component,
                        &row.field,
                        &draft,
                    );
                }
            });
        }
    }

    /// Preview plus Apply and Discard over the staged grid edits.
    fn inspector_apply_bar(
        &mut self,
        ui: &mut egui::Ui,
        grid: &InspectorGrid,
        rows: &[OutlinerRow],
    ) {
        match self.inspector_panel.preview(grid) {
            Ok(preview) => {
                for line in &preview.effects {
                    ui.label(line);
                }
            }
            Err(error) => {
                if !self.inspector_panel.is_empty() {
                    ui.label(format!("preview refused: {error}"));
                }
            }
        }
        ui.horizontal(|ui| {
            let staged = !self.inspector_panel.is_empty();
            if ui.add_enabled(staged, egui::Button::new("Apply")).clicked() {
                let poses = poses_for_rows(self.shell, rows);
                let key = (*self.inspector_key).max(1);
                *self.inspector_key = key.wrapping_add(1).max(1);
                match self
                    .inspector_panel
                    .emit_command(grid, key, self.shell.revision(), &poses)
                {
                    Ok(batch) => {
                        if let Err(error) = self.shell.apply_ops(batch.ops) {
                            self.shell
                                .log_message(format!("ERR inspector apply: {error}"));
                        }
                        let _ = self.inspector_panel.refresh();
                    }
                    Err(error) => {
                        self.shell
                            .log_message(format!("ERR inspector apply: {error}"));
                    }
                }
            }
            if ui
                .add_enabled(staged, egui::Button::new("Discard"))
                .clicked()
            {
                let _ = self.inspector_panel.refresh();
            }
        });
    }

    /// Viewport panel: live viewport texture, click-to-pick plus the gizmo.
    ///
    /// The live texture (when the host supplies one) draws first at the
    /// panel width, keeping its aspect. Clicking the canvas picks against
    /// the shell's viewport rows and selects the hit. The gizmo edits the
    /// selection's *world* pose through the `transform-gizmo` session; each
    /// result commits a drag tick at once inside the viewport drag session
    /// (see [`tick_viewport_drag`]), and pointer release folds the ticks
    /// into one undo step, so a real drag is one journal entry.
    fn viewport_panel(&mut self, ui: &mut egui::Ui) {
        let camera = self.shell.viewport.camera;
        ui.label(format!(
            "view {}x{} fov {:.0} eye [{:.1}, {:.1}, {:.1}]",
            self.shell.viewport.size.width,
            self.shell.viewport.size.height,
            camera.fov_y_deg,
            camera.position[0],
            camera.position[1],
            camera.position[2],
        ));
        if let Some(tex) = self.viewport_tex.clone() {
            let avail = ui.available_width().max(1.0);
            let size = tex.size_vec2();
            let aspect = if size.y > 0.0 {
                size.x / size.y
            } else {
                16.0 / 9.0
            };
            ui.image(egui::load::SizedTexture::new(
                tex.id(),
                [avail, avail / aspect],
            ));
        }
        ui.horizontal(|ui| {
            ui.label("tool");
            for tool in [GizmoTool::Translate, GizmoTool::Rotate] {
                let active = self.gizmo.tool() == tool;
                if ui.selectable_label(active, tool.title()).clicked() {
                    self.gizmo.set_tool(tool);
                }
            }
        });

        // The canvas owns the panel size: keep the shell viewport in sync so
        // pick pixels match what the user clicked.
        let canvas = ui.allocate_response(ui.available_size(), egui::Sense::click());
        if let Ok(size) = ViewportSize::new(canvas.rect.width(), canvas.rect.height()) {
            self.shell.viewport.size = size;
        }
        if canvas.clicked() {
            if let Some(cursor) = ui.input(|input| input.pointer.hover_pos()) {
                let x = cursor.x - canvas.rect.min.x;
                let y = cursor.y - canvas.rect.min.y;
                let pick = self.shell.pick(x, y);
                apply_viewport_pick(self.shell, self.shared_selection, pick);
            }
        }

        let Some(selected) = self.shell.selection else {
            ui.label("Click the canvas to pick a node.");
            return;
        };
        let Some(world) = self.shell.document().hierarchy.world(selected) else {
            // Defensive: the target is gone, so the session can never tick
            // again; abandon the merge and keep landed steps individually
            // undoable rather than sealing one entry for a dead target.
            cancel_viewport_drag(self.shell);
            ui.label("Selected node is gone.");
            return;
        };
        ui.label(format!("node {selected}"));
        self.gizmo.sync_camera(&self.shell.viewport);
        if let Some(edited_world) = self.gizmo.interact(ui, world) {
            // Live drag tick: the session opens on first movement and each
            // tick commits at once, so the node follows the pointer; release
            // folds the ticks into one undo step below. A rejected tick
            // writes nothing and keeps the session open for the next tick.
            if !self.shell.gizmo_drag_open() {
                let _ = begin_viewport_drag(self.shell);
            }
            let _ = tick_viewport_drag(self.shell, edited_world);
        } else if self.shell.gizmo_drag_open()
            && ui.input(|input| !input.pointer.button_down(egui::PointerButton::Primary))
        {
            // Pointer released with no gizmo movement: fold the drag ticks
            // into one significant `gizmo drag` entry (one undo step).
            let _ = end_viewport_drag(self.shell);
        }
        if self.shell.gizmo_drag_open() {
            ui.label("dragging: release to commit one undo step");
        } else {
            ui.label("drag the gizmo: one undo step per drag");
        }
    }

    /// Findings panel: spatial rows on the staged AI proposal with severity, one
    /// preview/apply pair per automatic fix. Every fix goes through the shell
    /// command batches (preview, apply, undo); stale, manual and refused rows
    /// never write. Staging itself lives in the shell API (the assistant panel
    /// owns proposals); this tab only reviews what is staged.
    fn findings_panel(&mut self, ui: &mut egui::Ui) {
        let stale = self.shell.findings_stale();
        let count = self.shell.findings_len();
        ui.label(format!("staged AI proposal: {count} findings"));
        ui.horizontal(|ui| {
            if ui.button("Refresh").clicked() {
                self.shell.refresh_findings();
                *self.fix_preview = None;
            }
        });
        if count == 0 {
            ui.label("Stage an AI proposal through the shell to list its spatial findings.");
            return;
        }
        if stale {
            ui.label("Stale: the document moved on. Refresh and re-stage; fixes never apply while stale.");
        }
        for index in 0..count {
            let Some(view) = self.shell.finding(index) else {
                continue;
            };
            ui.separator();
            let short_a: String = view.a.to_string().chars().take(8).collect();
            let pair = match view.b {
                Some(id) => {
                    let short_b: String = id.to_string().chars().take(8).collect();
                    format!("{short_a} + {short_b}")
                }
                None => short_a,
            };
            ui.label(format!(
                "#{} {} {} ({})",
                view.index, view.check, view.severity, pair
            ));
            ui.label(format!("fix: {}", view.fix));
            ui.horizontal(|ui| {
                if ui.button("Preview").clicked() {
                    match self.shell.preview_fix(index) {
                        Ok(preview) => *self.fix_preview = Some((index, preview)),
                        Err(error) => self
                            .shell
                            .log_message(format!("ERR fix preview #{index}: {error}")),
                    }
                }
                let can_apply = !stale && view.fix.is_auto();
                if ui
                    .add_enabled(can_apply, egui::Button::new("Apply fix"))
                    .clicked()
                {
                    *self.fix_preview = None;
                    let _ = self.shell.apply_fix(index);
                }
            });
            if let Some((cached, preview)) = self.fix_preview.as_ref() {
                if *cached == index {
                    for line in &preview.effects {
                        ui.label(line);
                    }
                }
            }
        }
    }

    /// Tuning tab: combat/AI drafts as validated swap batches at tick
    /// boundaries, journaled into the tape. Every edit routes through the
    /// shell setters (validate, then stage, then apply); refusals land in
    /// the log panel and change nothing.
    fn tuning_panel(&mut self, ui: &mut egui::Ui) {
        let cursor = self.shell.tune_cursor();
        let panel = self.shell.tuning_panel();
        let combat_version = panel.tape().combat_version_for_tick(cursor);
        let ai_version = panel.tape().ai_version_for_tick(cursor);
        ui.label(format!(
            "child tick {cursor}: combat v{combat_version}, ai v{ai_version}"
        ));
        match panel.staged() {
            None => {
                ui.label("no tune batch staged");
            }
            Some(staged) => {
                if staged.base_cursor() == cursor {
                    ui.label(format!(
                        "staged @tick {}: combat v{}, ai v{}",
                        staged.base_cursor(),
                        staged.combat_version(),
                        staged.ai_version()
                    ));
                } else {
                    ui.label(format!(
                        "STALE: staged @tick {}, child at {cursor} — refresh first",
                        staged.base_cursor()
                    ));
                }
            }
        }
        if let Some(receipt) = panel.last_receipt() {
            ui.label(format!("last: {receipt}"));
        }
        ui.horizontal(|ui| {
            if ui.button("Preview").clicked() {
                let _ = self.shell.tune_preview();
            }
            if ui.button("Stage").clicked() {
                let _ = self.shell.tune_stage();
            }
            let can_apply = self
                .shell
                .tuning_panel()
                .staged()
                .is_some_and(|staged| staged.base_cursor() == self.shell.tune_cursor());
            if ui
                .add_enabled(can_apply, egui::Button::new("Apply @tick"))
                .clicked()
            {
                let _ = self.shell.tune_apply();
            }
            if ui.button("Refresh").clicked() {
                self.shell.tune_refresh();
            }
            if ui.button("Tick +1").clicked() {
                self.shell.tune_tick();
            }
        });
        ui.separator();
        ui.label("combat (HR3 rows, ticks)");
        for field in CombatField::all() {
            let mut value = self.shell.tuning_panel().combat_value(field) as i32;
            ui.horizontal(|ui| {
                ui.label(field.name());
                if ui.add(egui::DragValue::new(&mut value).speed(1)).changed() {
                    if value < 0 {
                        let (min, max) = self.shell.tuning_panel().combat_range(field);
                        self.shell.log_message(format!(
                            "ERR tune combat: combat {} value {value} outside {min}..={max}",
                            field.name()
                        ));
                    } else {
                        let _ = self.shell.tune_set_combat(field, value as u32);
                    }
                }
            });
        }
        ui.separator();
        ui.label("ai curves (HR4 lanes 0..=7, values 0..=1000)");
        for lane in 0..CURVE_LANES {
            let mut aggression = self.shell.tuning_panel().ai_curves().aggression[lane] as i32;
            let mut caution = self.shell.tuning_panel().ai_curves().caution[lane] as i32;
            ui.horizontal(|ui| {
                ui.label(format!("lane {lane}"));
                if ui
                    .add(
                        egui::DragValue::new(&mut aggression)
                            .speed(5)
                            .prefix("atk "),
                    )
                    .changed()
                {
                    Self::push_lane(
                        self.shell,
                        lane,
                        aggression,
                        crate::shell::TuneLane::Aggression,
                    );
                }
                if ui
                    .add(egui::DragValue::new(&mut caution).speed(5).prefix("cxx "))
                    .changed()
                {
                    Self::push_lane(self.shell, lane, caution, crate::shell::TuneLane::Caution);
                }
            });
        }
        let mut engage = self.shell.tuning_panel().ai_curves().engage_range_cm as i32;
        let mut morale = self.shell.tuning_panel().ai_morale();
        ui.horizontal(|ui| {
            ui.label("engage cm");
            if ui.add(egui::DragValue::new(&mut engage).speed(5)).changed() {
                if engage < 0 {
                    self.shell.log_message(format!(
                        "ERR tune ai: engage range {engage} cm outside 50..=6000"
                    ));
                } else {
                    let _ = self.shell.tune_set_engage(engage as u32);
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("morale");
            if ui.add(egui::DragValue::new(&mut morale).speed(5)).changed() {
                let _ = self.shell.tune_set_morale(morale);
            }
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Verify replay").clicked() {
                *self.tune_replay = Some(self.shell.tune_replay_exact());
            }
            match self.tune_replay {
                None => ui.label("replay not verified yet"),
                Some(true) => ui.label("replay exact x2"),
                Some(false) => ui.label("REPLAY DIVERGED"),
            }
        });
    }

    /// Push one edited AI lane through the shell setter, logging sign slips
    /// the `u16` setters cannot spell.
    fn push_lane(shell: &mut Shell, lane: usize, value: i32, kind: crate::shell::TuneLane) {
        if value < 0 || value > i32::from(u16::MAX) {
            shell.log_message(format!(
                "ERR tune ai: lane {lane} value {value} outside 0..=1000"
            ));
            return;
        }
        let _ = match kind {
            crate::shell::TuneLane::Aggression => shell.tune_set_aggression(lane, value as u16),
            crate::shell::TuneLane::Caution => shell.tune_set_caution(lane, value as u16),
        };
    }

    /// Templates tab: arena, prop, weapon and fighter starters as command
    /// batches. Preview stages a validated batch, Create commits it through
    /// the shell (reserving the name), Undo restores the document, Clean-up
    /// frees items whose nodes are all gone. Refusals land in the log panel
    /// and change nothing.
    fn templates_panel(&mut self, ui: &mut egui::Ui) {
        ui.label("new-item templates: grounded, disjoint, budget-checked");
        ui.horizontal(|ui| {
            ui.label("kind");
            for kind in TemplateKind::all() {
                let active = *self.template_kind == kind;
                if ui.selectable_label(active, kind.title()).clicked() {
                    *self.template_kind = kind;
                    *self.template_preview = None;
                }
            }
        });
        ui.label(self.template_kind.blurb());
        ui.horizontal(|ui| {
            ui.label("name");
            ui.text_edit_singleline(self.template_name);
            if ui.button("Suggest").clicked() {
                *self.template_name = self.shell.template_suggest(*self.template_kind);
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Preview").clicked() {
                match self
                    .shell
                    .template_preview(*self.template_kind, self.template_name)
                {
                    Ok(preview) => {
                        *self.template_preview = Some(preview);
                        *self.template_status = None;
                    }
                    Err(error) => {
                        *self.template_preview = None;
                        *self.template_status = Some(error.to_string());
                    }
                }
            }
            if ui.button("Create").clicked() {
                *self.template_preview = None;
                match self
                    .shell
                    .template_apply(*self.template_kind, self.template_name)
                {
                    Ok(applied) => {
                        *self.template_status = Some(format!("created r{}", applied.revision));
                    }
                    Err(error) => *self.template_status = Some(error.to_string()),
                }
            }
            if ui.button("Undo").clicked() {
                *self.template_preview = None;
                match self.shell.template_undo() {
                    Some(applied) => {
                        *self.template_status = Some(format!("undone r{}", applied.revision));
                    }
                    None => *self.template_status = Some("nothing to undo".to_string()),
                }
            }
            if ui.button("Clean-up").clicked() {
                *self.template_preview = None;
                let freed = self.shell.template_reconcile();
                *self.template_status = Some(if freed.is_empty() {
                    "nothing freed".to_string()
                } else {
                    format!("freed {}", freed.join(", "))
                });
            }
        });
        if let Some(preview) = self.template_preview.as_ref() {
            for line in &preview.effects {
                ui.label(line);
            }
        }
        if let Some(status) = self.template_status.as_ref() {
            ui.label(status.clone());
        }
        ui.separator();
        ui.label(format!("items ({})", self.shell.template_len()));
        for item in self.shell.template_items() {
            ui.label(format!(
                "{} `{}`: {} nodes, {} draws, {} tris",
                item.kind,
                item.name,
                item.nodes.len(),
                item.draws,
                item.tris
            ));
        }
    }

    /// Bounded receipt feed with its overflow count.
    fn log(&mut self, ui: &mut egui::Ui) {
        let dropped = self.shell.log().dropped();
        ui.horizontal(|ui| {
            ui.label(format!(
                "kept {} (dropped {dropped})",
                self.shell.log().len()
            ));
            if ui.button("Clear").clicked() {
                self.shell.clear_log();
            }
        });
        egui::ScrollArea::vertical().show(ui, |ui| {
            for line in self.shell.log().iter() {
                ui.label(line);
            }
        });
    }
}

/// Live outliner rows for the hierarchy tab.
///
/// Builds [`OutlinerRow`]s from the shell snapshot with fallback short names;
/// same snapshot gives the same rows in row order. Errors when the snapshot
/// exceeds the row bound; the caller shows the refusal and stages nothing.
pub(crate) fn live_outliner_rows(shell: &Shell) -> Result<Vec<OutlinerRow>, outline::OutlineError> {
    let rows = shell.rows();
    let names: HashMap<ObjectId, String> = HashMap::new();
    outline::build_rows(&rows, &names)
}

/// Node views for the inspector grid: one per live outliner row.
///
/// Identity carries the row name as the prefab key, the pose carries the
/// live local transform, and one `label` component at v1 current feeds the
/// grid label and schema-current rows. Same rows give the same views.
pub(crate) fn node_views_for_rows(shell: &Shell, rows: &[OutlinerRow]) -> Vec<NodeView> {
    let mut by_id: HashMap<u64, &OutlinerRow> = HashMap::new();
    for row in rows {
        by_id.insert(row.id, row);
    }
    let mut views = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(local) = shell.document().hierarchy.local(row.source) else {
            continue;
        };
        let parent_key = row
            .parent
            .and_then(|id| by_id.get(&id).map(|parent| parent.name.clone()));
        views.push(NodeView {
            id: row.id,
            key: Some(row.name.clone()),
            parent_key,
            pose: local,
            components: vec![VersionedComponent::new("label", 1, &row.name)],
            schema: vec![SchemaStep {
                component: "label",
                version: 1,
                is_current: true,
            }],
            museum: false,
        });
    }
    views
}

/// Live poses keyed by stable id for the inspector command fold.
///
/// Same rows give the same map; missing locals are skipped because the grid
/// already refuses them as unknown nodes.
pub(crate) fn poses_for_rows(
    shell: &Shell,
    rows: &[OutlinerRow],
) -> BTreeMap<u64, (ObjectId, Transform)> {
    let mut poses = BTreeMap::new();
    for row in rows {
        if let Some(local) = shell.document().hierarchy.local(row.source) {
            poses.insert(row.id, (row.source, local));
        }
    }
    poses
}

/// Reconcile the shared selection with the shell before each frame.
///
/// Prunes stale ids, adopts the shell selection when the shared one is empty
/// and pushes the shared primary otherwise, so viewport, outliner and
/// inspector agree on the identical live object. Never stages batches beyond
/// closing a live gizmo drag through the sync below.
pub(crate) fn reconcile_shared_selection(
    shell: &mut Shell,
    shared: &mut SharedSelection,
    rows: &[OutlinerRow],
    live: &BTreeSet<u64>,
) {
    outline::prune(shared, live);
    if shared.primary.is_none() {
        if let Some(id) = shell.selection.map(outline::stable_id) {
            if live.contains(&id) {
                *shared = outline::select_one(id);
            } else {
                select_closing_drag(shell, ObjectId::NIL);
            }
        }
        return;
    }
    sync_shell_to_shared(shell, shared, rows);
}

/// Mirror the shared primary into the shell for gizmo compatibility.
///
/// True when the shell selection changed; closes a live drag first so ticks
/// fold into one entry before the target moves.
pub(crate) fn sync_shell_to_shared(
    shell: &mut Shell,
    shared: &SharedSelection,
    rows: &[OutlinerRow],
) -> bool {
    let wanted = shared
        .primary
        .and_then(|id| rows.iter().find(|row| row.id == id).map(|row| row.source));
    match wanted {
        Some(id) => {
            if shell.selection == Some(id) {
                false
            } else {
                select_closing_drag(shell, id)
            }
        }
        None => {
            if shell.selection.is_none() {
                false
            } else {
                select_closing_drag(shell, ObjectId::NIL)
            }
        }
    }
}

/// Route one viewport pick into the shared selection as a replacing click.
///
/// Stale picks refuse without mutating; misses clear. On success the shell
/// mirrors the shared primary for the gizmo session.
pub(crate) fn apply_viewport_pick(
    shell: &mut Shell,
    shared: &mut SharedSelection,
    pick: Option<ObjectId>,
) {
    let rows = match live_outliner_rows(shell) {
        Ok(rows) => rows,
        Err(_) => return,
    };
    let live = outline::live_set(&rows);
    outline::prune(shared, &live);
    match outline::apply_pick(shared, pick, false, &live) {
        Ok(next) => {
            *shared = next;
            sync_shell_to_shared(shell, shared, &rows);
        }
        Err(error) => {
            shell.log_message(format!("ERR viewport pick: {error}"));
        }
    }
}

/// Viewport drag session routing: the viewport panel drives the
/// shell's grouped gizmo-drag session with absolute world poses, so a real
/// drag commits live tick by tick and pointer release folds the ticks into
/// one significant `gizmo drag` journal entry (one undo step, one durable
/// seal). Selection changes and removals route through
/// [`select_closing_drag`]/[`remove_closing_drag`], which end-or-cancel the
/// session first, so a drag never leaks across
/// targets. Units follow `engine-scene`: meters, radians. Errors are the
/// shell's ([`ShellError`]); a rejected tick writes nothing and keeps the
/// session open for the next tick.
///
/// The consumer is the viewport panel above plus headless drag scripts in
/// tests; the shell owns the group, the document and the journal.
pub(crate) fn begin_viewport_drag(shell: &mut Shell) -> Result<u64, ShellError> {
    shell.begin_gizmo_drag()
}

/// Commit one absolute world-pose tick inside the session, opening it when
/// none is live. Each tick applies immediately through the command service
/// inside the open group, so the node follows the pointer; the later release
/// ([`end_viewport_drag`]) merges the ticks into one entry.
pub(crate) fn tick_viewport_drag(
    shell: &mut Shell,
    world: Transform,
) -> Result<Applied, ShellError> {
    if !shell.gizmo_drag_open() {
        shell.begin_gizmo_drag()?;
    }
    shell.apply_gizmo_world(world)
}

/// Release the session, merging its ticks into one undo step. None when no
/// session is live or it held no steps (the session still closes).
pub(crate) fn end_viewport_drag(shell: &mut Shell) -> Option<Applied> {
    shell.end_gizmo_drag()
}

/// Abandon the session without merging: its steps stay individual entries.
/// True when a session was live.
pub(crate) fn cancel_viewport_drag(shell: &mut Shell) -> bool {
    shell.cancel_gizmo_drag()
}

/// Select a node, ending-or-cancelling a live drag first so committed ticks
/// fold into one entry for the old target before the selection moves.
pub(crate) fn select_closing_drag(shell: &mut Shell, id: ObjectId) -> bool {
    if shell.gizmo_drag_open() {
        let _ = shell.end_gizmo_drag();
    }
    shell.select(id)
}

/// Remove a node, ending-or-cancelling a live drag first so the removal
/// applies outside the drag group and never joins its merged entry.
pub(crate) fn remove_closing_drag(shell: &mut Shell, id: ObjectId) -> Result<Applied, ShellError> {
    if shell.gizmo_drag_open() {
        let _ = shell.end_gizmo_drag();
    }
    shell.remove_node(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Headless dock frame: no window, no GPU, just one egui pass over all
    /// four panels (hierarchy, inspector, viewport with its gizmo session,
    /// log). The windowed present loop stays with the window host; this only proves the
    /// panels draw without panicking and route through shell batches.
    #[test]
    fn headless_dock_frame_smoke() {
        let mut shell = Shell::new();
        let id = ObjectId::new();
        shell
            .insert_node(id, ObjectId::NIL, Transform::identity())
            .unwrap();
        assert!(shell.select(id));
        let mut ui_state = ShellUi::new(&shell);
        assert_eq!(ui_state.gizmo_tool(), GizmoTool::Translate);

        // `run_ui` drives one frame with no window and no GPU. The frame
        // produces paint output (including one font texture delta) that a
        // real host would upload; headless we just clear it.
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui_state.show(&mut shell, ui);
        });
        output.textures_delta.clear();

        // The frame changed no document state: drawing only reads.
        assert_eq!(shell.selection, Some(id));
        assert!(shell.staged_preview().is_none());
        assert_eq!(shell.document().hierarchy.len(), 1);
    }

    /// Viewport, outliner and inspector agree on the identical live selection.
    ///
    /// The viewport pick joins the shared object, an outliner search hit
    /// toggles it, a saved set recalls it, and the inspector grids the same
    /// ids; the shell mirrors the primary for the gizmo.
    #[test]
    fn live_panels_share_one_selection() {
        use std::any::TypeId;

        use crate::inspector as inspector_mod;
        use crate::outline as outline_mod;

        assert_eq!(
            TypeId::of::<SharedSelection>(),
            TypeId::of::<inspector_mod::Selection>()
        );
        let mut shell = Shell::new();
        let root = ObjectId::new();
        let child = ObjectId::new();
        shell
            .insert_node(root, ObjectId::NIL, Transform::identity())
            .unwrap();
        shell
            .insert_node(child, root, Transform::identity())
            .unwrap();
        let rows = live_outliner_rows(&shell).unwrap();
        assert_eq!(rows.len(), 2);
        let live = outline_mod::live_set(&rows);
        let view = shell.viewport;
        let hit = shell.pick(view.size.width / 2.0, view.size.height / 2.0);
        assert_eq!(hit, Some(root));
        let mut shared = SharedSelection::new();
        apply_viewport_pick(&mut shell, &mut shared, hit);
        assert_eq!(shared.primary, Some(outline_mod::stable_id(root)));
        assert_eq!(shell.selection, Some(root));
        let query = outline_mod::OutlinerQuery::new("");
        let hits = outline_mod::search(&rows, &query).unwrap();
        assert_eq!(hits.len(), 2);
        let other = hits
            .iter()
            .find(|item| item.id != shared.primary.expect("primary is set"))
            .expect("second row lists")
            .id;
        shared = outline_mod::select_toggle(&shared, other, &live).unwrap();
        sync_shell_to_shared(&mut shell, &shared, &rows);
        assert_eq!(shared.ids.len(), 2);
        assert_eq!(shell.selection, Some(root));
        let mut bank = SelectionBank::new();
        bank.save("pair", shared.clone(), &live).unwrap();
        let recalled = bank.recall("pair", &SharedSelection::new()).unwrap();
        assert_eq!(recalled, shared);
        let views = node_views_for_rows(&shell, &rows);
        let baseline: BTreeMap<(u64, String, String), String> = BTreeMap::new();
        let grid = inspector_mod::build_grid(&views, &shared, &baseline).unwrap();
        assert!(grid
            .rows()
            .iter()
            .any(|row| row.node == outline_mod::stable_id(root)));
        assert!(grid
            .rows()
            .iter()
            .any(|row| row.node == outline_mod::stable_id(child)));
        let mut ui_state = ShellUi::new(&shell);
        ui_state.shared_selection = shared.clone();
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui_state.show(&mut shell, ui);
        });
        output.textures_delta.clear();
        assert_eq!(shell.selection, Some(root));
        assert_eq!(ui_state.shared_selection, shared);
    }

    /// The 2,000-row siege scene draws only the rows in view, and a quiet
    /// frame reuses the cached outliner rows instead of rebuilding them.
    #[test]
    fn siege_hierarchy_frame_draws_only_the_visible_rows() {
        let (mut shell, _) = outline::siege_scene();
        let mut ui_state = ShellUi::new(&shell);
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0));
        let mut frame = |shell: &mut Shell| {
            let input = egui::RawInput {
                screen_rect: Some(screen),
                ..egui::RawInput::default()
            };
            let mut output = ctx.run_ui(input, |ui| ui_state.show(shell, ui));
            output.textures_delta.clear();
            output.shapes.len()
        };
        // The first frame loads fonts and builds the 2,000 rows once.
        frame(&mut shell);
        let mut shapes = 0;
        let mut times: Vec<std::time::Duration> = (0..31)
            .map(|_| {
                let start = std::time::Instant::now();
                shapes = frame(&mut shell);
                start.elapsed()
            })
            .collect();
        times.sort();
        let median = times[times.len() / 2];
        eprintln!("ED19 siege dock frame: median {median:?}, {shapes} shapes");
        assert!(
            shapes < 600,
            "{shapes} shapes: the list must virtualize, not draw 2,000 rows"
        );
        assert!(median < std::time::Duration::from_millis(5), "{median:?}");
        assert_eq!(shell.document().hierarchy.len(), outline::SIEGE_ROWS);
    }

    /// The Findings tab draws staged rows without panicking and without
    /// writing: drawing only reads the shell, fixes stay on their buttons.
    #[test]
    fn findings_tab_draws_staged_rows() {
        use crate::commands::{Batch, Op};

        let mut shell = Shell::new();
        let a = ObjectId::new();
        let b = ObjectId::new();
        let pose = |x: f32| Transform::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]);
        shell.insert_node(a, ObjectId::NIL, pose(0.0)).unwrap();
        shell.insert_node(b, ObjectId::NIL, pose(10.0)).unwrap();
        let batch = Batch {
            ops: vec![Op::SetLocal {
                id: b,
                local: pose(0.0),
            }],
            base_revision: shell.revision(),
            idempotency_key: 4242,
            provenance: None,
        };
        assert_eq!(shell.stage_proposal(&batch).unwrap(), 1);
        assert!(shell.is_open(Panel::Findings));

        let mut ui_state = ShellUi::new(&shell);
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui_state.show(&mut shell, ui);
        });
        output.textures_delta.clear();

        // Drawing changed no review state: the finding row is still staged.
        assert_eq!(shell.findings_len(), 1);
        assert!(!shell.findings_stale());
        assert_eq!(shell.finding(0).unwrap().check, "overlap");
    }

    /// The Tuning tab draws drafts plus a staged batch without panicking and
    /// without writing: drawing only reads the shell, tunes stay on their
    /// buttons.
    #[test]
    fn tuning_tab_draws_drafts_and_staged_batch() {
        use crate::tuning::CombatField;

        let mut shell = Shell::new();
        assert!(shell.is_open(Panel::Tuning));
        shell
            .tune_set_combat(CombatField::AttackDamage, 240)
            .unwrap();
        shell
            .tune_set_combat(CombatField::ParryWindowTicks, 3)
            .unwrap();
        shell.tune_set_engage(300).unwrap();
        shell.tune_stage().unwrap();
        assert!(shell.tuning_panel().staged().is_some());

        let mut ui_state = ShellUi::new(&shell);
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui_state.show(&mut shell, ui);
        });
        output.textures_delta.clear();

        // Drawing changed no tuning state: the batch is still staged at the
        // same versions, the tape is still empty.
        let staged = shell.tuning_panel().staged().expect("batch kept");
        assert_eq!((staged.combat_version(), staged.ai_version()), (2, 2));
        assert_eq!(shell.tuning_panel().tape().command_len(), 0);
        assert!(!shell.tune_stale());
    }

    /// The Templates tab draws kinds plus reserved items without panicking
    /// and without writing: drawing only reads the shell, creation stays
    /// on its buttons.
    #[test]
    fn templates_tab_draws_kinds_and_reserved_items() {
        let mut shell = Shell::new();
        assert!(shell.is_open(Panel::Templates));
        shell
            .template_apply(TemplateKind::Prop, "prop")
            .expect("prop template applies");
        assert_eq!(shell.template_len(), 1);

        let mut ui_state = ShellUi::new(&shell);
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui_state.show(&mut shell, ui);
        });
        output.textures_delta.clear();

        // Drawing changed no template state: the item is still reserved
        // with its node, the document untouched.
        assert_eq!(shell.template_len(), 1);
        assert_eq!(shell.document().hierarchy.len(), 1);
        assert_eq!(
            shell.template_suggest(TemplateKind::Prop),
            "prop_2".to_string()
        );
    }

    fn drag_world(x: f32) -> Transform {
        Transform::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    fn drag_shell() -> (Shell, ObjectId) {
        let mut shell = Shell::new();
        let id = ObjectId::new();
        shell
            .insert_node(id, ObjectId::NIL, Transform::identity())
            .unwrap();
        assert!(shell.select(id));
        (shell, id)
    }

    /// A viewport drag across ticks commits live and folds into one journal
    /// entry on release: one undo step, one durable seal, the end pose kept,
    /// exact undo/redo.
    #[test]
    fn viewport_drag_is_one_entry_with_live_ticks() {
        use crate::commands::durable_count;

        let (mut shell, id) = drag_shell();
        let start = shell.document().hierarchy.clone();
        let (base_past, base_durable) =
            (shell.journal().past_len(), durable_count(shell.journal()));

        begin_viewport_drag(&mut shell).unwrap();
        assert!(shell.gizmo_drag_open());
        for x in [1.0, 2.0, 3.0, 4.0, 5.0] {
            tick_viewport_drag(&mut shell, drag_world(x)).unwrap();
            // Live commit: the document follows the pointer mid-drag.
            let local = shell.document().hierarchy.local(id).unwrap();
            assert!((local.position[0] - x).abs() <= 1e-5);
        }
        assert_eq!(shell.journal().past_len(), base_past + 5);

        let merged = end_viewport_drag(&mut shell).expect("drag commits one entry");
        assert!(!shell.gizmo_drag_open());
        assert_eq!(shell.journal().past_len(), base_past + 1);
        assert_eq!(durable_count(shell.journal()), base_durable + 1);
        let entry = shell.journal().past.last().expect("merged entry");
        assert_eq!(entry.name, "gizmo drag");
        assert!(entry.significant);
        assert_eq!(merged.key, entry.key);
        // Releasing twice commits nothing; nothing left to cancel.
        assert!(end_viewport_drag(&mut shell).is_none());
        assert!(!cancel_viewport_drag(&mut shell));

        // Same end pose as one single-shot apply of the last world pose.
        let (mut plain, plain_id) = drag_shell();
        plain.apply_gizmo_world(drag_world(5.0)).unwrap();
        assert_eq!(
            shell.document().hierarchy.local(id).map(|t| t.position),
            plain
                .document()
                .hierarchy
                .local(plain_id)
                .map(|t| t.position)
        );

        // One undo restores the drag start; redo replays it exactly.
        shell.undo().expect("drag undoes in one step");
        assert_eq!(shell.document().hierarchy, start);
        shell.redo().expect("drag redoes in one step");
        assert_eq!(
            shell.document().hierarchy.local(id).map(|t| t.position),
            Some([5.0, 0.0, 0.0])
        );
    }

    /// A bare tick without an explicit begin still opens, groups and merges.
    #[test]
    fn viewport_tick_auto_opens_session() {
        let (mut shell, id) = drag_shell();
        let base = shell.journal().past_len();
        tick_viewport_drag(&mut shell, drag_world(2.0)).unwrap();
        assert!(shell.gizmo_drag_open());
        end_viewport_drag(&mut shell).expect("auto-opened drag commits");
        assert_eq!(shell.journal().past_len(), base + 1);
        let local = shell.document().hierarchy.local(id).unwrap();
        assert!((local.position[0] - 2.0).abs() <= 1e-5);
    }

    /// Drag routing without a live selection fails clean: no session, no
    /// journal growth, no document change.
    #[test]
    fn viewport_drag_without_selection_fails_clean() {
        use crate::commands::durable_count;

        let mut shell = Shell::new();
        assert_eq!(
            begin_viewport_drag(&mut shell).unwrap_err(),
            ShellError::NoSelection
        );
        assert_eq!(
            tick_viewport_drag(&mut shell, drag_world(1.0)).unwrap_err(),
            ShellError::NoSelection
        );
        assert!(!shell.gizmo_drag_open());
        assert!(shell.journal().is_empty());
        assert_eq!(durable_count(shell.journal()), 0);
        assert!(end_viewport_drag(&mut shell).is_none());
    }

    /// A degenerate world pose is refused without staging or applying, the
    /// session survives it, and the later good ticks still merge into one
    /// entry whose undo restores the start.
    #[test]
    fn viewport_degenerate_tick_writes_nothing_and_session_survives() {
        use crate::commands::durable_count;

        let (mut shell, id) = drag_shell();
        let snapshot = shell.document().clone();
        let (base_past, base_durable) =
            (shell.journal().past_len(), durable_count(shell.journal()));
        begin_viewport_drag(&mut shell).unwrap();

        // Zero-scale world pose: not invertible back to the parent frame.
        let flat = Transform::new([9.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0]);
        let error = tick_viewport_drag(&mut shell, flat).unwrap_err();
        assert!(matches!(error, ShellError::Gizmo(_)));
        assert_eq!(*shell.document(), snapshot);
        assert_eq!(shell.journal().past_len(), base_past);
        assert_eq!(durable_count(shell.journal()), base_durable);
        assert!(shell.gizmo_drag_open());

        tick_viewport_drag(&mut shell, drag_world(3.0)).unwrap();
        end_viewport_drag(&mut shell).expect("good ticks still merge");
        assert_eq!(shell.journal().past_len(), base_past + 1);
        shell.undo().expect("merged drag undoes");
        assert_eq!(shell.document().hierarchy, snapshot.hierarchy);
        let _ = id;
    }

    /// Finding 1: changing the selection ends the live drag as one entry
    /// for the old target before the selection moves.
    #[test]
    fn select_ends_open_drag_as_one_entry() {
        let (mut shell, id) = drag_shell();
        let other = ObjectId::new();
        shell
            .insert_node(other, ObjectId::NIL, Transform::identity())
            .unwrap();
        assert!(shell.select(id));
        let base = shell.journal().past_len();

        begin_viewport_drag(&mut shell).unwrap();
        tick_viewport_drag(&mut shell, drag_world(1.0)).unwrap();
        tick_viewport_drag(&mut shell, drag_world(2.0)).unwrap();
        assert!(shell.gizmo_drag_open());

        assert!(select_closing_drag(&mut shell, other));
        assert!(!shell.gizmo_drag_open());
        assert_eq!(shell.selection, Some(other));
        // Two ticks folded into one entry for the old target.
        assert_eq!(shell.journal().past_len(), base + 1);
        assert_eq!(shell.journal().past.last().unwrap().name, "gizmo drag");
        let local = shell.document().hierarchy.local(id).unwrap();
        assert!((local.position[0] - 2.0).abs() <= 1e-5);
        // The drag still undoes in one step after the selection moved.
        shell.undo().expect("drag undoes in one step");
        let back = shell.document().hierarchy.local(id).unwrap();
        assert_eq!(back.position, [0.0, 0.0, 0.0]);
    }

    /// Finding 2: removing a node ends the live drag first, so the removal
    /// applies outside the drag group and the drag keeps its single entry.
    #[test]
    fn remove_ends_open_drag_outside_the_group() {
        let (mut shell, id) = drag_shell();
        let base = shell.journal().past_len();

        begin_viewport_drag(&mut shell).unwrap();
        tick_viewport_drag(&mut shell, drag_world(1.0)).unwrap();
        tick_viewport_drag(&mut shell, drag_world(2.0)).unwrap();
        assert!(shell.gizmo_drag_open());

        remove_closing_drag(&mut shell, id).unwrap();
        assert!(!shell.gizmo_drag_open());
        // One merged drag entry plus the removal itself.
        assert_eq!(shell.journal().past_len(), base + 2);
        assert_eq!(shell.journal().past[base].name, "gizmo drag");
        assert!(!shell.document().hierarchy.contains(id));
        assert_eq!(shell.selection, None);
    }

    /// The cancel half of ends-or-cancels: abandoning keeps the landed steps
    /// as individual entries, each undoing on its own.
    #[test]
    fn cancel_viewport_drag_keeps_steps_individual() {
        let (mut shell, id) = drag_shell();
        let base = shell.journal().past_len();
        begin_viewport_drag(&mut shell).unwrap();
        tick_viewport_drag(&mut shell, drag_world(1.0)).unwrap();
        tick_viewport_drag(&mut shell, drag_world(2.0)).unwrap();
        assert!(cancel_viewport_drag(&mut shell));
        assert!(!shell.gizmo_drag_open());
        assert_eq!(shell.journal().past_len(), base + 2);
        shell.undo().unwrap();
        let local = shell.document().hierarchy.local(id).unwrap();
        assert!((local.position[0] - 1.0).abs() <= 1e-5);
    }
}
