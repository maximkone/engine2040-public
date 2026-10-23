//! Palette overlay and History tab drawing for [`ShellUi`](super::ShellUi).
//!
//! Split out of `shell_ui.rs` so that file stays under the size ratchet. Both
//! methods only read keys and paint; the state machines they drive live in
//! [`crate::palette`] and [`crate::history_panel`], which are headless and
//! tested without a window.

use super::{ShellUi, Viewer};

impl ShellUi {
    /// Palette overlay: Ctrl+K or Ctrl+P toggles it, Up/Down/Enter/Escape go
    /// through [`PaletteState::key`](crate::palette::PaletteState::key), and a
    /// dispatched row is stored for the host. Nothing here calls the command
    /// service, so the overlay stays a pure state machine plus a draw list.
    pub(super) fn palette_frame(&mut self, ui: &mut egui::Ui) {
        use crate::palette::PaletteKey;
        let ctrl = ui.input(|input| input.modifiers.command || input.modifiers.ctrl);
        let toggle = ctrl
            && ui.input(|input| input.key_pressed(egui::Key::K) || input.key_pressed(egui::Key::P));
        let key = if toggle {
            Some(PaletteKey::Toggle)
        } else if !self.palette.is_open() {
            None
        } else if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            Some(PaletteKey::Escape)
        } else if ui.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
            Some(PaletteKey::MoveDown)
        } else if ui.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
            Some(PaletteKey::MoveUp)
        } else if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            Some(PaletteKey::Enter)
        } else {
            None
        };
        if let Some(key) = key {
            match self.palette.key(key, &mut self.palette_index) {
                Ok(Some(action)) => self.palette_action = Some(action),
                Ok(None) => {}
                Err(error) => self.palette_error = Some(error.to_string()),
            }
        }
        if !self.palette.is_open() {
            return;
        }
        let mut draft = self.palette.query().to_string();
        let rows = match self.palette.draw_rows(&self.palette_index) {
            Ok(rows) => rows,
            Err(error) => {
                self.palette_error = Some(error.to_string());
                Vec::new()
            }
        };
        let error = self.palette_error.clone();
        egui::Window::new("Palette")
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label("query");
                    ui.text_edit_singleline(&mut draft);
                });
                for row in &rows {
                    let text = format!("{}  [{}]", row.label, row.tier.label());
                    let _row = ui.selectable_label(row.selected, text);
                }
                if let Some(error) = &error {
                    ui.colored_label(egui::Color32::RED, error);
                }
            });
        if draft != self.palette.query() {
            self.palette.set_query(&draft);
        }
    }
}

impl Viewer<'_> {
    /// History tab: kept steps newest first (label, revision, tick, version
    /// name), Up/Down moves the cursor, Enter stores the jump for the host.
    pub(super) fn history_tab(&mut self, ui: &mut egui::Ui) {
        let rows = self.history_panel.rows(self.history);
        if rows.is_empty() {
            ui.weak("No undo steps yet. Each acknowledged apply lands here.");
            return;
        }
        if ui.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
            self.history_panel.move_cursor(true, rows.len());
        }
        if ui.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
            self.history_panel.move_cursor(false, rows.len());
        }
        let cursor = self.history_panel.cursor();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (row, entry) in rows.iter().enumerate() {
                let text = match &entry.name {
                    Some(name) => format!(
                        "{}  rev={}  t{:04}  [{name}]",
                        entry.label, entry.revision, entry.tick
                    ),
                    None => format!(
                        "{}  rev={}  t{:04}",
                        entry.label, entry.revision, entry.tick
                    ),
                };
                let _row = ui.selectable_label(row == cursor, text);
            }
        });
        if ui.input(|input| input.key_pressed(egui::Key::Enter)) {
            if let Ok(jump) = self.history_panel.activate(self.history) {
                *self.history_jump = Some(jump);
            }
        }
    }
}
