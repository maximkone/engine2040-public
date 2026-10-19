//! Editor theme: one dark, warm palette for the whole shell.
//!
//! Consumer is [`EditorApp::new`](crate::app::EditorApp) through [`apply`];
//! the dock, panels, viewport border and menus all read the same egui
//! [`Style`], so one call paints the shell. Data owner is this module: the
//! palette constants and the spacing/text scale. Units are egui points; the
//! colors are sRGB triads. Determinism does not apply (presentation only),
//! and no clock or I/O is read.

use egui::{
    Color32, CornerRadius, FontId, Stroke, Style, TextStyle, Theme, ThemePreference, Visuals,
};

/// Dock and panel fill, one step above the window background.
const PANEL: Color32 = Color32::from_rgb(22, 19, 16);
/// Floating window fill.
const WINDOW: Color32 = Color32::from_rgb(27, 23, 19);
/// Grid stripes and barely-there fills.
const FAINT: Color32 = Color32::from_rgb(34, 29, 24);
/// Text edits, scroll bars and other recessed surfaces.
const EXTREME: Color32 = Color32::from_rgb(11, 9, 8);
/// Primary text: warm off-white, never pure white.
const TEXT: Color32 = Color32::from_rgb(236, 229, 218);
/// Secondary text and hints.
const WEAK: Color32 = Color32::from_rgb(166, 154, 137);
/// Bronze accent: selection, focus, active strokes.
const ACCENT: Color32 = Color32::from_rgb(217, 164, 65);
/// Deep bronze behind selected text.
const ACCENT_DEEP: Color32 = Color32::from_rgb(90, 62, 20);
/// Hairline borders.
const EDGE: Color32 = Color32::from_rgb(58, 50, 43);
/// Hovered widget fill.
const HOVER: Color32 = Color32::from_rgb(51, 42, 34);
/// Pressed widget fill.
const ACTIVE: Color32 = Color32::from_rgb(74, 58, 40);
/// Error text.
const ERROR: Color32 = Color32::from_rgb(224, 108, 90);
/// Warning text.
const WARN: Color32 = Color32::from_rgb(224, 163, 74);

/// Apply the editor palette and spacing to one egui context.
///
/// Both the dark and the light theme slots receive the same style and the
/// preference is pinned to dark, so the shell looks the same on a machine
/// set to light mode. Call once, before the first frame.
pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.dark_mode = true;
    v.override_text_color = Some(TEXT);
    v.weak_text_color = Some(WEAK);
    v.panel_fill = PANEL;
    v.window_fill = WINDOW;
    v.faint_bg_color = FAINT;
    v.extreme_bg_color = EXTREME;
    v.code_bg_color = EXTREME;
    v.window_stroke = Stroke::new(1.0, EDGE);
    v.selection.bg_fill = ACCENT_DEEP;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARN;
    v.error_fg_color = ERROR;
    v.window_corner_radius = CornerRadius::same(8);
    v.menu_corner_radius = CornerRadius::same(6);

    let corner = CornerRadius::same(6);
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.noninteractive.weak_bg_fill = PANEL;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, EDGE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.noninteractive.corner_radius = corner;

    v.widgets.inactive.bg_fill = FAINT;
    v.widgets.inactive.weak_bg_fill = FAINT;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, EDGE);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.corner_radius = corner;

    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.corner_radius = corner;

    v.widgets.active.bg_fill = ACTIVE;
    v.widgets.active.weak_bg_fill = ACTIVE;
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.corner_radius = corner;

    v.widgets.open.bg_fill = HOVER;
    v.widgets.open.weak_bg_fill = HOVER;
    v.widgets.open.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.open.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.open.corner_radius = corner;

    v.button_frame = true;
    v.striped = true;
    v.indent_has_left_vline = true;

    let mut style = (*ctx.style_of(Theme::Dark)).clone();
    style.visuals = v;
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(9.0, 5.0);
    style.spacing.interact_size = egui::vec2(26.0, 22.0);
    style.spacing.indent = 18.0;
    style.spacing.slider_width = 140.0;
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(13.5));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(13.5));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(11.5));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(17.0));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(12.5));

    ctx.set_style_of(Theme::Dark, style.clone());
    ctx.set_style_of(Theme::Light, style);
    ctx.set_theme(ThemePreference::Dark);
}

/// The one style the shell uses after [`apply`]; exposed for tests and for
/// panels that need to paint a color outside the widget path.
#[must_use]
pub fn style(ctx: &egui::Context) -> Style {
    (*ctx.style_of(Theme::Dark)).clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_is_dark_warm_and_bronze() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let style = style(&ctx);
        assert!(style.visuals.dark_mode);
        assert_eq!(style.visuals.panel_fill, PANEL);
        assert_eq!(style.visuals.selection.bg_fill, ACCENT_DEEP);
        assert_eq!(style.visuals.selection.stroke.color, ACCENT);
        assert_eq!(style.visuals.widgets.hovered.bg_stroke.color, ACCENT);
        assert_eq!(
            style.visuals.widgets.hovered.corner_radius,
            CornerRadius::same(6)
        );
        assert!(style.spacing.interact_size.y >= 22.0);
        assert_eq!(
            style.text_styles.get(&TextStyle::Body),
            Some(&FontId::proportional(13.5))
        );
        // Both theme slots carry the same style, so a light-mode OS cannot
        // flip the editor to a white shell.
        let light = ctx.style_of(Theme::Light);
        assert_eq!(light.visuals.panel_fill, PANEL);
    }
}
