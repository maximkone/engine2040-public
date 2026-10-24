//! Derived inspector fields: the runtime half of `#[derive(Inspector)]`.
//!
//! Owner: the editor's tuning panel. The macro (`crates/editor-derive`) emits
//! one [`FieldSpec`] per drawn field plus `fn ui(&mut self, ui: &mut egui::Ui)`
//! `-> bool`, which hands the table to [`draw`]. Units: each value in its own
//! field type; `min`, `max` and `step` are the authored attribute literals,
//! carried as `f64` so one table serves `f32` and `f64` fields. Budget: UI
//! thread only, one widget per row, no allocation beyond egui's own. Errors: an
//! unsupported field type is a compile error in the macro, never a refusal here.

use engine_core::checked::f32_from_f64;

/// One field as the macro describes it: label, group, range and accessors.
pub struct FieldSpec<T> {
    /// Field name as written in the struct; the widget label.
    pub name: &'static str,
    /// Panel section; an empty group draws no heading.
    pub group: &'static str,
    /// Smallest accepted value, from `#[inspector(min = ..)]`.
    pub min: Option<f64>,
    /// Largest accepted value, from `#[inspector(max = ..)]`.
    pub max: Option<f64>,
    /// Slider step, from `#[inspector(step = ..)]`.
    pub step: Option<f64>,
    /// True when the panel shows the value and never edits it.
    pub readonly: bool,
    /// Typed read/write access to the field.
    pub access: FieldAccess<T>,
}

/// Typed access to one field: the macro's only per-type seam.
pub enum FieldAccess<T> {
    /// An `f32` field.
    F32 {
        /// Reads the field.
        get: fn(&T) -> f32,
        /// Writes the field.
        set: fn(&mut T, f32),
    },
    /// An `f64` field.
    F64 {
        /// Reads the field.
        get: fn(&T) -> f64,
        /// Writes the field.
        set: fn(&mut T, f64),
    },
    /// A `bool` field.
    Bool {
        /// Reads the field.
        get: fn(&T) -> bool,
        /// Writes the field.
        set: fn(&mut T, bool),
    },
}

impl<T> FieldAccess<T> {
    /// The value as panel text, for a read-only row.
    fn text(&self, value: &T) -> String {
        match self {
            Self::F32 { get, .. } => format!("{:.3}", get(value)),
            Self::F64 { get, .. } => format!("{:.3}", get(value)),
            Self::Bool { get, .. } => get(value).to_string(),
        }
    }
}

/// What one [`draw`] call did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawReport {
    /// True when any field value changed this frame.
    pub changed: bool,
    /// Widgets drawn, one per table row.
    pub widgets: usize,
}

/// Draw one widget per field in table order, one heading per group change.
pub fn draw<T>(fields: &[FieldSpec<T>], value: &mut T, ui: &mut egui::Ui) -> DrawReport {
    let mut report = DrawReport {
        changed: false,
        widgets: 0,
    };
    let mut group = "";
    for spec in fields {
        if !spec.group.is_empty() && spec.group != group {
            group = spec.group;
            ui.label(egui::RichText::new(group).strong());
        }
        report.changed |= row(spec, value, ui);
        report.widgets += 1;
    }
    report
}

/// One row: a read-only label, or the typed editor for the field.
fn row<T>(spec: &FieldSpec<T>, value: &mut T, ui: &mut egui::Ui) -> bool {
    if spec.readonly {
        ui.label(format!("{}: {}", spec.name, spec.access.text(value)));
        return false;
    }
    match &spec.access {
        FieldAccess::F32 { get, set } => numeric_row(
            spec,
            value,
            ui,
            *get,
            *set,
            spec.min.map(f32_from_f64),
            spec.max.map(f32_from_f64),
        ),
        FieldAccess::F64 { get, set } => {
            numeric_row(spec, value, ui, *get, *set, spec.min, spec.max)
        }
        FieldAccess::Bool { get, set } => bool_row(spec, value, ui, *get, *set),
    }
}

/// One numeric row: a slider over the authored range, else a drag value.
fn numeric_row<T, Num: egui::emath::Numeric>(
    spec: &FieldSpec<T>,
    value: &mut T,
    ui: &mut egui::Ui,
    get: fn(&T) -> Num,
    set: fn(&mut T, Num),
    min: Option<Num>,
    max: Option<Num>,
) -> bool {
    let mut next = get(value);
    let response = match (min, max) {
        (Some(min), Some(max)) => ui.add(slider(spec.name, &mut next, min, max, spec.step)),
        _ => {
            let drag = egui::DragValue::new(&mut next).speed(spec.step.unwrap_or(0.01));
            ui.add(drag)
        }
    };
    if !response.changed() {
        return false;
    }
    set(value, next);
    true
}

/// One `0 | 1` row: a checkbox.
fn bool_row<T>(
    spec: &FieldSpec<T>,
    value: &mut T,
    ui: &mut egui::Ui,
    get: fn(&T) -> bool,
    set: fn(&mut T, bool),
) -> bool {
    let mut next = get(value);
    let response = ui.checkbox(&mut next, spec.name);
    if !response.changed() {
        return false;
    }
    set(value, next);
    true
}

/// A slider over `min..=max` with the authored step and the field name.
fn slider<'a, Num: egui::emath::Numeric>(
    name: &'static str,
    value: &'a mut Num,
    min: Num,
    max: Num,
    step: Option<f64>,
) -> egui::Slider<'a> {
    let mut slider = egui::Slider::new(value, min..=max).text(name);
    if let Some(step) = step {
        slider = slider.step_by(step);
    }
    slider
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspector::FeelTuning;

    /// Three fields: one attributed, one plain, one skipped.
    #[derive(engine_editor_derive::Inspector)]
    struct Probe {
        #[inspector(min = 0.0, max = 1.0, step = 0.01, group = "probe")]
        pitch: f32,
        gain: f64,
        #[inspector(skip)]
        hidden: u32,
    }

    fn probe() -> Probe {
        Probe {
            pitch: 0.5,
            gain: 1.0,
            hidden: 7,
        }
    }

    /// One headless egui frame: no window, no GPU, a fixed screen rectangle.
    /// The font texture is never uploaded, so its delta is cleared here.
    fn frame(mut run: impl FnMut(&mut egui::Ui)) {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(480.0, 320.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| run(ui));
        output.textures_delta.clear();
    }

    #[test]
    fn inspector_probe_draws_two_widgets_with_their_ranges() {
        assert_eq!(
            Probe::INSPECTOR_FIELDS.len(),
            2,
            "the skipped field draws nothing"
        );
        let names: Vec<&str> = Probe::INSPECTOR_FIELDS.iter().map(|s| s.name).collect();
        assert_eq!(names, ["pitch", "gain"]);
        let pitch = &Probe::INSPECTOR_FIELDS[0];
        assert_eq!(
            (pitch.min, pitch.max, pitch.step),
            (Some(0.0), Some(1.0), Some(0.01))
        );
        assert_eq!(pitch.group, "probe");
        assert!(!pitch.readonly);
        let gain = &Probe::INSPECTOR_FIELDS[1];
        assert_eq!((gain.min, gain.max, gain.step), (None, None, None));
        assert_eq!(gain.group, "");

        let mut value = probe();
        let mut widgets = 0;
        frame(|ui| {
            widgets = draw(Probe::INSPECTOR_FIELDS, &mut value, ui).widgets;
        });
        assert_eq!(widgets, 2);

        // The derived `fn ui` is the seam the tuning panel calls: a fresh
        // frame moves nothing and reports no change.
        let mut again = probe();
        let mut changed = true;
        frame(|ui| {
            changed = Probe::ui(&mut again, ui);
        });
        assert!(!changed);
        assert_eq!(again.pitch, 0.5);
        assert_eq!(again.gain, 1.0);
        assert_eq!(again.hidden, 7, "a skipped field is not a widget");
    }

    #[test]
    fn inspector_feel_table_draws_its_knobs_and_holds_the_read_only_one() {
        let fields = FeelTuning::INSPECTOR_FIELDS;
        assert_eq!(fields.len(), 3, "three knobs, the revision stays hidden");
        assert!(fields.iter().all(|spec| spec.group == "feel"));
        let shake = &fields[0];
        assert_eq!(
            (shake.min, shake.max, shake.step),
            (Some(0.0), Some(2.0), Some(0.05))
        );
        let pitch = &fields[2];
        assert_eq!(pitch.name, "pitch_variation");
        assert!(pitch.readonly);

        let mut feel = FeelTuning {
            shake_scale: 1.0,
            flash_lift: 1.0,
            pitch_variation: 0.0,
            revision: 0,
        };
        let mut changed = true;
        frame(|ui| {
            changed = feel.ui(ui);
        });
        assert!(!changed);
        // Read-only means drawn and never edited.
        assert_eq!(feel.pitch_variation, 0.0);
    }
}
