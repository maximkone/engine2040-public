//! Graph, guided-path and perf surfaces: state plus their draw code.
//!
//! Three surfaces with no evaluation of their own. The material graph tab draws
//! [`Canvas`](crate::graph_canvas::Canvas) nodes and routes edits through the
//! validated `connect`/`move_node` doors, so a refusal shows as a toast row and
//! never mutates the canvas. The guided path paints
//! [`panel_rows`](crate::art_overlay::RungRow) as a top-left overlay and hands
//! `N` to [`advance_on_key`](crate::art_overlay::advance_on_key). The perf
//! overlay paints rows from the host's [`PerfSource`](crate::perf_panel::PerfSource).
//!
//! Split out of `shell_ui.rs` so that file keeps its quality pin: one
//! [`Surfaces`] field carries all three states.
//! Units: canvas positions are integer pixels, zoom thousandths, pan pixels.
//! Determinism: the draw order is the canvas's node order and the ladder order.

use super::{ShellUi, Viewer};
use crate::art_path::{ArtPath, ArtStep};
use crate::graph_canvas::{Canvas, CanvasView, Link, PortKind};
use crate::graph_eval::MaterialReport;
use crate::perf_panel::{Band, Budget, PerfPanel, PerfSource};

/// Node box width in canvas pixels.
pub const NODE_W: i32 = 160;
/// Node box height in canvas pixels.
pub const NODE_H: i32 = 72;
/// Grid a dragged node snaps to, in canvas pixels.
pub const SNAP_PX: i32 = 8;
/// Radius of a port dot, in canvas pixels.
pub const PORT_R: i32 = 6;
/// One wheel notch of zoom, in thousandths.
pub const ZOOM_NOTCH_MILLI: i32 = 250;

/// State of the three surfaces: material graph, guided ladder and perf overlay.
pub struct Surfaces {
    /// Nodes and links the Graph tab edits.
    pub graph: Canvas,
    /// Pan and zoom of the graph view.
    pub view: CanvasView,
    /// Node the click selected.
    pub pick: Option<u32>,
    /// Node Enter handed to the host through [`ShellUi::take_graph_pick`].
    pub graph_pick: Option<u32>,
    /// First half of a pending link: `(node, output port)`.
    pub link: Option<(u32, u8)>,
    /// Last graph refusal or confirmation row.
    pub toast: Option<String>,
    /// The guided ladder.
    pub art: ArtPath,
    /// Last guided-path line (`advanced to Material` or a refusal).
    pub art_notice: Option<String>,
    /// Perf overlay visibility.
    pub perf: PerfPanel,
    /// Budget the overlay bands every row with, in each row's own unit.
    pub perf_budget: Budget,
    /// The host's numbers, when it filled them.
    pub perf_source: Option<Box<dyn PerfSource>>,
    /// Last `E` evaluation: the status row and the verdict.
    pub material: Option<MaterialReport>,
}

impl Default for Surfaces {
    fn default() -> Self {
        Self {
            graph: Canvas::new(),
            view: CanvasView::new(),
            pick: None,
            graph_pick: None,
            link: None,
            toast: None,
            art: ArtPath::new(),
            art_notice: None,
            perf: PerfPanel::new(),
            perf_budget: Budget::new(u64::MAX, u64::MAX),
            perf_source: None,
            material: None,
        }
    }
}

impl ShellUi {
    /// The material graph the Graph tab draws.
    pub fn graph(&self) -> &Canvas {
        &self.surfaces.graph
    }

    /// Replace the graph (a host load); selection and pending link clear.
    pub fn set_graph(&mut self, graph: Canvas) {
        self.surfaces.graph = graph;
        self.surfaces.pick = None;
        self.surfaces.link = None;
        self.surfaces.toast = None;
    }

    /// Take the node id Enter handed over. The host binds the material.
    pub fn take_graph_pick(&mut self) -> Option<u32> {
        self.surfaces.graph_pick.take()
    }

    /// Take the last `E` evaluation: its status row and, when it baked, the
    /// verdict. Nothing is written to a material here.
    pub fn take_material_verdict(&mut self) -> Option<MaterialReport> {
        self.surfaces.material.take()
    }

    /// The guided ladder as the host last set it.
    pub fn art(&self) -> &ArtPath {
        &self.surfaces.art
    }

    /// Host readiness for one rung (a file landed, a light exists, ...).
    pub fn set_step_ready(&mut self, step: ArtStep, ready: bool) {
        self.surfaces.art.set_ready(step, ready);
    }

    /// Take the last guided-path line for the host to echo.
    pub fn take_art_notice(&mut self) -> Option<String> {
        self.surfaces.art_notice.take()
    }

    /// True when the perf overlay draws.
    pub fn perf_shown(&self) -> bool {
        self.surfaces.perf.shown()
    }

    /// Hand the host's measurements to the perf overlay.
    pub fn set_perf_source(&mut self, source: Box<dyn PerfSource>) {
        self.surfaces.perf_source = Some(source);
    }

    /// Budget every perf row is banded with, in that row's unit.
    pub fn set_perf_budget(&mut self, budget: Budget) {
        self.surfaces.perf_budget = budget;
    }

    /// Toggle the perf overlay, as the `P` key does.
    pub fn toggle_perf(&mut self) -> bool {
        self.surfaces.perf.toggle()
    }

    /// Both overlays, drawn over the dock: the guided path top-left and the
    /// perf rows under it. Keys `N` and `P` are read here.
    pub fn overlays(&mut self, ui: &mut egui::Ui) {
        self.art_overlay(ui);
        self.perf_overlay(ui);
    }

    /// Guided path overlay: five rungs, the current one strong with its hint
    /// under it, `N` taking the current rung.
    fn art_overlay(&mut self, ui: &mut egui::Ui) {
        if ui.input(|input| input.key_pressed(egui::Key::N)) {
            let line = match crate::art_overlay::advance_on_key(&mut self.surfaces.art) {
                Ok(step) => format!("advanced to {}", step.name()),
                Err(error) => error.to_string(),
            };
            self.surfaces.art_notice = Some(line);
        }
        let rows = self.surfaces.art.panel_rows();
        let notice = self.surfaces.art_notice.clone();
        let ctx = ui.ctx().clone();
        egui::Area::new(egui::Id::new("art-path-overlay"))
            .fixed_pos(egui::pos2(8.0, 8.0))
            .show(&ctx, |ui| {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_max_width(280.0);
                    ui.label("Guided art path  (N takes the next ready rung)");
                    for row in &rows {
                        if row.current {
                            ui.strong(&row.label);
                        } else {
                            ui.label(&row.label);
                        }
                        if let Some(hint) = row.hint {
                            ui.weak(hint);
                        }
                    }
                    if let Some(notice) = notice.as_deref() {
                        ui.colored_label(egui::Color32::YELLOW, notice);
                    }
                });
            });
    }

    /// Perf overlay: one row per host sample, banded by the caller budget,
    /// `P` toggling the whole overlay.
    fn perf_overlay(&mut self, ui: &mut egui::Ui) {
        if ui.input(|input| input.key_pressed(egui::Key::P)) {
            self.surfaces.perf.toggle();
        }
        if !self.surfaces.perf.shown() {
            return;
        }
        let budget = self.surfaces.perf_budget;
        let rows = match self.surfaces.perf_source.as_ref() {
            Some(source) => self.surfaces.perf.rows(source.as_ref(), |_| budget),
            None => Vec::new(),
        };
        let ctx = ui.ctx().clone();
        egui::Area::new(egui::Id::new("perf-overlay"))
            .fixed_pos(egui::pos2(8.0, 240.0))
            .show(&ctx, |ui| {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_max_width(320.0);
                    if rows.is_empty() {
                        ui.weak("perf: no source (the window host fills it)");
                        return;
                    }
                    for row in &rows {
                        let color = match row.band {
                            Band::Pass => egui::Color32::LIGHT_GREEN,
                            Band::Warn => egui::Color32::YELLOW,
                            Band::Fail => egui::Color32::LIGHT_RED,
                        };
                        ui.colored_label(color, row.text());
                    }
                });
            });
    }
}

/// Integer pixels to points. The i16 clamp keeps the conversion total.
fn pt(value: i32) -> f32 {
    f32::from(i16::try_from(value).unwrap_or(0))
}

/// Points back to integer pixels, for pointer movement.
#[expect(
    clippy::cast_possible_truncation,
    reason = "pointer deltas are whole points inside a window narrower than 32767 px"
)]
fn points(value: f32) -> i32 {
    value.round() as i32
}

/// Screen position of a canvas point under the view.
fn screen(rect: egui::Rect, view: CanvasView, x: i32, y: i32) -> egui::Pos2 {
    let zoom = pt(view.zoom_milli) / 1000.0;
    egui::pos2(
        rect.left() + pt(x.saturating_add(view.pan_x)) * zoom,
        rect.top() + pt(y.saturating_add(view.pan_y)) * zoom,
    )
}

/// Canvas point under a screen position.
fn to_canvas(rect: egui::Rect, view: CanvasView, pos: egui::Pos2) -> (i32, i32) {
    let zoom = pt(view.zoom_milli) / 1000.0;
    let scale = 1000.0 / zoom;
    (
        points((pos.x - rect.left()) * scale) - view.pan_x,
        points((pos.y - rect.top()) * scale) - view.pan_y,
    )
}

/// Snap one canvas coordinate to the [`SNAP_PX`] grid.
fn snapped(value: i32) -> i32 {
    value - value.rem_euclid(SNAP_PX)
}

impl Viewer<'_> {
    /// Graph tab: node boxes with titles and port dots, middle-drag pan, wheel
    /// zoom, click to select, drag to move on the 8 px grid, and a two-click
    /// link gesture that goes through [`Canvas::connect`]. A refusal is a toast
    /// row; Enter hands the selected id to the host, and `E` evaluates the
    /// canvas through the material pack as one status row. Nothing is
    /// evaluated or written on the draw path otherwise.
    pub(super) fn graph_tab(&mut self, ui: &mut egui::Ui) {
        let selected = self
            .surfaces
            .pick
            .map_or_else(|| "none".to_string(), |id| id.to_string());
        ui.label(format!(
            "graph  nodes={}  links={}  zoom={}  selected={selected}",
            self.surfaces.graph.nodes().len(),
            self.surfaces.graph.links().len(),
            self.surfaces.view.zoom_milli
        ));
        if let Some(toast) = self.surfaces.toast.as_deref() {
            ui.colored_label(egui::Color32::YELLOW, toast);
        }
        if let Some(report) = self.surfaces.material.as_ref() {
            ui.monospace(&report.line);
        }
        if let Some((id, port)) = self.surfaces.link {
            ui.weak(format!("linking from node {id} out {port}"));
        }
        let (response, painter) = ui.allocate_painter(
            ui.available_size().max(egui::vec2(64.0, 64.0)),
            egui::Sense::click_and_drag(),
        );
        let rect = response.rect;
        if response.dragged_by(egui::PointerButton::Middle) {
            let drag = response.drag_delta();
            self.surfaces.view.pan_by(points(drag.x), points(drag.y));
        }
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll > 0.0 {
                self.surfaces.view.zoom_by(ZOOM_NOTCH_MILLI);
            } else if scroll < 0.0 {
                self.surfaces.view.zoom_by(-ZOOM_NOTCH_MILLI);
            }
        }
        if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            if let Some(id) = self.surfaces.pick {
                self.surfaces.graph_pick = Some(id);
            }
        }
        if ui.input(|input| input.key_pressed(egui::Key::E)) {
            self.surfaces.material = Some(MaterialReport::of(self.surfaces.graph.evaluate()));
        }
        self.graph_draw(&painter, rect);
        if let Some(pos) = response.interact_pointer_pos() {
            if response.clicked() {
                self.graph_click(rect, pos);
            }
            if response.dragged_by(egui::PointerButton::Primary) {
                self.graph_drag(rect, pos);
            }
        }
    }

    /// Paint one box per node with its title and one dot per port.
    fn graph_draw(&self, painter: &egui::Painter, rect: egui::Rect) {
        let view = self.surfaces.view;
        let zoom = pt(view.zoom_milli) / 1000.0;
        for node in self.surfaces.graph.nodes() {
            let min = screen(rect, view, node.x, node.y);
            let size = egui::vec2(pt(NODE_W) * zoom, pt(NODE_H) * zoom);
            let box_rect = egui::Rect::from_min_size(min, size);
            let fill = if self.surfaces.pick == Some(node.id) {
                egui::Color32::from_rgb(60, 90, 130)
            } else {
                egui::Color32::from_rgb(30, 34, 40)
            };
            painter.rect_filled(box_rect, egui::CornerRadius::same(3), fill);
            painter.text(
                box_rect.min + egui::vec2(8.0, 6.0),
                egui::Align2::LEFT_TOP,
                format!("node {}", node.id),
                egui::FontId::proportional(12.0),
                egui::Color32::WHITE,
            );
            for (port, kind) in node.outputs.iter().enumerate() {
                let center = egui::pos2(
                    box_rect.left() + pt(port_x(0, port)) * zoom,
                    box_rect.top() + 4.0,
                );
                painter.circle_filled(center, pt(PORT_R) * zoom, port_color(*kind));
            }
            for (port, kind) in node.inputs.iter().enumerate() {
                let center = egui::pos2(
                    box_rect.left() + pt(port_x(0, port)) * zoom,
                    box_rect.bottom() - 4.0,
                );
                painter.circle_filled(center, pt(PORT_R) * zoom, port_color(*kind));
            }
        }
    }

    /// Node under a screen point, with the output or input port dot it hit.
    fn graph_hit(
        &self,
        rect: egui::Rect,
        pos: egui::Pos2,
    ) -> Option<(u32, Option<u8>, Option<u8>)> {
        let (cx, cy) = to_canvas(rect, self.surfaces.view, pos);
        let node = self.surfaces.graph.nodes().iter().rev().find(|node| {
            cx >= node.x && cx < node.x + NODE_W && cy >= node.y && cy < node.y + NODE_H
        })?;
        let out = (0..node.outputs.len()).find(|port| {
            (cx - (node.x + port_x(0, *port))).abs() <= PORT_R && (cy - node.y).abs() <= PORT_R
        });
        let inp = (0..node.inputs.len()).find(|port| {
            (cx - (node.x + port_x(0, *port))).abs() <= PORT_R
                && (cy - (node.y + NODE_H)).abs() <= PORT_R
        });
        Some((
            node.id,
            out.and_then(|port| u8::try_from(port).ok()),
            inp.and_then(|port| u8::try_from(port).ok()),
        ))
    }

    /// A click: an output dot starts a link, an input dot completes one, and a
    /// box body selects the node.
    fn graph_click(&mut self, rect: egui::Rect, pos: egui::Pos2) {
        match self.graph_hit(rect, pos) {
            None => self.surfaces.pick = None,
            Some((id, Some(port), _)) => {
                self.surfaces.link = Some((id, port));
                self.surfaces.toast = Some(format!("linking from node {id} output {port}"));
            }
            Some((id, None, Some(port))) => self.finish_link(id, port),
            Some((id, None, None)) => self.surfaces.pick = Some(id),
        }
    }

    /// A primary drag moves the selected node to the 8 px grid.
    fn graph_drag(&mut self, rect: egui::Rect, pos: egui::Pos2) {
        let Some(id) = self.surfaces.pick else {
            return;
        };
        let (cx, cy) = to_canvas(rect, self.surfaces.view, pos);
        if let Err(error) = self.surfaces.graph.move_node(id, snapped(cx), snapped(cy)) {
            self.surfaces.toast = Some(error.to_string());
        }
    }

    /// Complete a pending link into `(node, port)`, or report why it cannot.
    fn finish_link(&mut self, to: u32, to_port: u8) {
        let Some((from, from_port)) = self.surfaces.link.take() else {
            self.surfaces.toast = Some("no link in flight: click an output dot first".to_string());
            return;
        };
        match self.surfaces.graph.connect(Link {
            from,
            from_port,
            to,
            to_port,
        }) {
            Ok(()) => {
                self.surfaces.toast = Some(format!("linked {from}:{from_port} -> {to}:{to_port}"));
            }
            Err(error) => self.surfaces.toast = Some(error.to_string()),
        }
    }
}

/// x of a port dot inside its node box, in canvas pixels.
fn port_x(origin: i32, port: usize) -> i32 {
    origin + 8 + i32::try_from(port).unwrap_or(0) * 14
}

/// Dot colour of one port kind, so a kind mismatch is visible before the click.
fn port_color(kind: PortKind) -> egui::Color32 {
    match kind {
        PortKind::Color => egui::Color32::from_rgb(230, 140, 60),
        PortKind::Scalar => egui::Color32::from_rgb(120, 200, 120),
        PortKind::Normal => egui::Color32::from_rgb(120, 160, 230),
        PortKind::Mask => egui::Color32::from_rgb(200, 120, 200),
    }
}
