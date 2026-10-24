//! Dock tab viewer: one arm per panel, plus the tab id, title and close hook.
//!
//! Split out of `shell_ui.rs` when that file crossed the 1500-line quality
//! bound. [`Viewer`](super::Viewer) and its drawing methods stay with the
//! shell; this module only routes a tab to its method.

use egui_dock::widgets::tab_viewer::OnCloseResponse;
use egui_dock::TabViewer;

use super::{ShellUi, Viewer};
use crate::shell::Panel;

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
            Panel::Browser => self.browser.show(ui, self.shell),
            Panel::Activity => self.activity(ui),
            Panel::Log => self.log(ui),
            Panel::History => self.history_tab(ui),
            Panel::Timeline => self.timeline_tab(ui),
            Panel::Poses => self.poses_tab(ui),
            Panel::Graph => self.graph_tab(ui),
        }
    }

    fn on_close(&mut self, tab: &mut Self::Tab) -> OnCloseResponse {
        self.shell.close_panel(*tab);
        OnCloseResponse::Close
    }
}

impl ShellUi {
    /// Replace the animation sheet the Timeline tab edits.
    pub fn set_sheet(&mut self, sheet: crate::timeline::Sheet) {
        self.timeline = sheet;
        self.timeline_panel = crate::timeline::SheetPanel::new();
    }

    /// The sheet the Timeline tab edits.
    pub fn sheet(&self) -> &crate::timeline::Sheet {
        &self.timeline
    }

    /// Take the last timeline edit (`I` or Delete). The host applies it.
    pub fn take_timeline_edit(&mut self) -> Option<crate::timeline::SheetEdit> {
        self.timeline_edit.take()
    }

    /// The pose shelf the Poses tab lists.
    pub fn shelf(&self) -> &crate::pose_shelf::PoseShelf {
        &self.shelf
    }

    /// Hand the recorded tape to the Poses tab; `S` saves it.
    pub fn set_pose_tape(&mut self, tape: crate::pose_tape::PoseTape) {
        self.pose_tape = Some(tape);
    }

    /// Take the pose name Enter picked.
    pub fn take_pose_pick(&mut self) -> Option<String> {
        self.pose_pick.take()
    }
}
