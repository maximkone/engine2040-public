//! Automatic inspector rows from registry schemas plus undoable batches.
//!
//! Owner: the editor session; reads registry schemas and caller values, owns
//! only its staged edits. Units: component/field text, revisions (`u64`).
//! Budget: UI thread only, row build linear in fields, never per-tick.
//! Determinism: row order is component then field; no clock, no hash order.
//! Errors: [`AutoError`]; refusals stage nothing and mutate nothing.
//!
//! Pattern: one row builder per schema type carrying
//! min/max/step/dropdown/read-only hints, without reflection.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use engine_scene::schema::SchemaRegistry;

use crate::commands::bound_to;
use crate::inspector::parse_bool;

/// Most rows kept in one auto grid.
pub const MAX_ROWS: usize = 128;
/// Most ops staged in one auto batch.
pub const MAX_OPS: usize = 64;
/// Longest text value kept in a cell.
pub const MAX_TEXT_LEN: usize = 512;

/// Every schema type the auto grid renders (20, in grid order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SchemaKind {
    /// Free text up to `max` chars.
    Text,
    /// Integer in `min..=max`.
    Integer,
    /// Integer slider in `min..=max` stepping `step`.
    IntegerSlider,
    /// Float in `min..=max`; NaN and infinity always refuse.
    Float,
    /// Float slider in `min..=max` stepping `step`.
    FloatSlider,
    /// Percentage `0..=100` stepping `1`.
    Percentage,
    /// Angle in degrees `-180..=180` stepping `0.1`.
    AngleDeg,
    /// Distance in meters `0..=10000` stepping `0.01`.
    DistanceM,
    /// Scale factor `0.001..=1000` stepping `0.001`.
    ScaleFactor,
    /// Quaternion lane `-1..=1` stepping `0.001`.
    QuatLane,
    /// Checkbox (`true`/`false`, also `1`/`0`, `yes`/`no`).
    Bool,
    /// One of `options[index]`; stored by name.
    Dropdown,
    /// Color `r,g,b` with each lane `0..=255`.
    ColorRgb,
    /// Three comma floats `x,y,z`, each in `min..=max`.
    Vec3,
    /// Entity reference (`u64` text, must name a live row).
    EntityRef,
    /// Asset path (non-empty text ending in a known suffix).
    AssetPath,
    /// Duration in seconds `0..=3600` stepping `0.01`.
    DurationSecs,
    /// Count `0..=1024` stepping `1`.
    Count,
    /// Component version pinned at `current`; read-only by construction.
    Version,
    /// Never editable; `reason` names why.
    ReadOnly,
}

impl SchemaKind {
    /// Every schema type, in grid order.
    pub const fn all() -> [Self; 20] {
        [
            Self::Text,
            Self::Integer,
            Self::IntegerSlider,
            Self::Float,
            Self::FloatSlider,
            Self::Percentage,
            Self::AngleDeg,
            Self::DistanceM,
            Self::ScaleFactor,
            Self::QuatLane,
            Self::Bool,
            Self::Dropdown,
            Self::ColorRgb,
            Self::Vec3,
            Self::EntityRef,
            Self::AssetPath,
            Self::DurationSecs,
            Self::Count,
            Self::Version,
            Self::ReadOnly,
        ]
    }

    /// Short grid section name for this kind.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Integer => "integer",
            Self::IntegerSlider => "int-slider",
            Self::Float => "float",
            Self::FloatSlider => "float-slider",
            Self::Percentage => "percentage",
            Self::AngleDeg => "angle-deg",
            Self::DistanceM => "distance-m",
            Self::ScaleFactor => "scale",
            Self::QuatLane => "quat-lane",
            Self::Bool => "bool",
            Self::Dropdown => "dropdown",
            Self::ColorRgb => "color-rgb",
            Self::Vec3 => "vec3",
            Self::EntityRef => "entity-ref",
            Self::AssetPath => "asset-path",
            Self::DurationSecs => "duration-s",
            Self::Count => "count",
            Self::Version => "version",
            Self::ReadOnly => "read-only",
        }
    }

    /// True for cells the panel never stages.
    pub const fn is_read_only(self) -> bool {
        matches!(self, Self::Version | Self::ReadOnly)
    }
}

impl fmt::Display for SchemaKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One field schema: a registry component field plus its inspector hints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldSchema {
    /// Component holding the field, e.g. `"pose"`.
    pub component: &'static str,
    /// Field within the component, e.g. `"pose.px"`.
    pub field: &'static str,
    /// Which row builder renders this field.
    pub kind: SchemaKind,
    /// Smallest accepted value (numeric kinds).
    pub min: f64,
    /// Largest accepted value (numeric kinds).
    pub max: f64,
    /// Step the slider/arrow keys move (slider kinds).
    pub step: f64,
    /// Allowed values in display order (dropdown only).
    pub options: &'static [&'static str],
    /// Default cell text when the caller holds no value.
    pub default: &'static str,
    /// Why the cell never edits (read-only kinds).
    pub read_only_reason: &'static str,
}

/// Dropdown options for the mode field.
pub const MODES: [&str; 3] = ["ring", "pit", "hall"];
/// Suffixes the asset-path row accepts.
pub const ASSET_SUFFIXES: [&str; 3] = [".glb", ".png", ".wav"];

/// The 20 field schemas the grid renders, one per [`SchemaKind`].
pub const fn field_schemas() -> [FieldSchema; 20] {
    [
        FieldSchema {
            component: "label",
            field: "text",
            kind: SchemaKind::Text,
            min: 0.0,
            max: 512.0,
            step: 1.0,
            options: &[],
            default: "",
            read_only_reason: "",
        },
        FieldSchema {
            component: "gadget",
            field: "level",
            kind: SchemaKind::Integer,
            min: 0.0,
            max: 99.0,
            step: 1.0,
            options: &[],
            default: "0",
            read_only_reason: "",
        },
        FieldSchema {
            component: "gadget",
            field: "tier",
            kind: SchemaKind::IntegerSlider,
            min: 0.0,
            max: 9.0,
            step: 1.0,
            options: &[],
            default: "0",
            read_only_reason: "",
        },
        FieldSchema {
            component: "pose",
            field: "weight",
            kind: SchemaKind::Float,
            min: -1000.0,
            max: 1000.0,
            step: 0.1,
            options: &[],
            default: "0.0000",
            read_only_reason: "",
        },
        FieldSchema {
            component: "pose",
            field: "pose.px",
            kind: SchemaKind::FloatSlider,
            min: -10_000.0,
            max: 10_000.0,
            step: 0.1,
            options: &[],
            default: "0.0000",
            read_only_reason: "",
        },
        FieldSchema {
            component: "tune",
            field: "mix",
            kind: SchemaKind::Percentage,
            min: 0.0,
            max: 100.0,
            step: 1.0,
            options: &[],
            default: "50",
            read_only_reason: "",
        },
        FieldSchema {
            component: "pose",
            field: "yaw_deg",
            kind: SchemaKind::AngleDeg,
            min: -180.0,
            max: 180.0,
            step: 0.1,
            options: &[],
            default: "0",
            read_only_reason: "",
        },
        FieldSchema {
            component: "pose",
            field: "range_m",
            kind: SchemaKind::DistanceM,
            min: 0.0,
            max: 10_000.0,
            step: 0.01,
            options: &[],
            default: "1",
            read_only_reason: "",
        },
        FieldSchema {
            component: "pose",
            field: "pose.sx",
            kind: SchemaKind::ScaleFactor,
            min: 0.001,
            max: 1000.0,
            step: 0.001,
            options: &[],
            default: "1.0000",
            read_only_reason: "",
        },
        FieldSchema {
            component: "pose",
            field: "pose.rx",
            kind: SchemaKind::QuatLane,
            min: -1.0,
            max: 1.0,
            step: 0.001,
            options: &[],
            default: "0.0000",
            read_only_reason: "",
        },
        FieldSchema {
            component: "gadget",
            field: "enabled",
            kind: SchemaKind::Bool,
            min: 0.0,
            max: 1.0,
            step: 1.0,
            options: &[],
            default: "false",
            read_only_reason: "",
        },
        FieldSchema {
            component: "arena",
            field: "mode",
            kind: SchemaKind::Dropdown,
            min: 0.0,
            max: 2.0,
            step: 1.0,
            options: &MODES,
            default: "ring",
            read_only_reason: "",
        },
        FieldSchema {
            component: "render",
            field: "tint",
            kind: SchemaKind::ColorRgb,
            min: 0.0,
            max: 255.0,
            step: 1.0,
            options: &[],
            default: "255,255,255",
            read_only_reason: "",
        },
        FieldSchema {
            component: "pose",
            field: "offset",
            kind: SchemaKind::Vec3,
            min: -1000.0,
            max: 1000.0,
            step: 0.1,
            options: &[],
            default: "0,0,0",
            read_only_reason: "",
        },
        FieldSchema {
            component: "scene",
            field: "target",
            kind: SchemaKind::EntityRef,
            min: 1.0,
            max: 4_294_967_295.0,
            step: 1.0,
            options: &[],
            default: "1",
            read_only_reason: "",
        },
        FieldSchema {
            component: "assets",
            field: "mesh",
            kind: SchemaKind::AssetPath,
            min: 1.0,
            max: 256.0,
            step: 1.0,
            options: &ASSET_SUFFIXES,
            default: "prop.glb",
            read_only_reason: "",
        },
        FieldSchema {
            component: "audio",
            field: "length_s",
            kind: SchemaKind::DurationSecs,
            min: 0.0,
            max: 3600.0,
            step: 0.01,
            options: &[],
            default: "1",
            read_only_reason: "",
        },
        FieldSchema {
            component: "spawn",
            field: "count",
            kind: SchemaKind::Count,
            min: 0.0,
            max: 1024.0,
            step: 1.0,
            options: &[],
            default: "1",
            read_only_reason: "",
        },
        FieldSchema {
            component: "label",
            field: "version",
            kind: SchemaKind::Version,
            min: 1.0,
            max: 1.0,
            step: 1.0,
            options: &[],
            default: "1",
            read_only_reason: "version pins at current and never edits",
        },
        FieldSchema {
            component: "museum",
            field: "note",
            kind: SchemaKind::ReadOnly,
            min: 0.0,
            max: 0.0,
            step: 1.0,
            options: &[],
            default: "museum-v1",
            read_only_reason: "museum fixture loads read-only",
        },
    ]
}

/// One automatic row: a built cell plus its inspector hints.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoRow {
    /// Component holding the field.
    pub component: String,
    /// Field within the component.
    pub field: String,
    /// Which builder produced this row.
    pub kind: SchemaKind,
    /// Live value text.
    pub value: String,
    /// Smallest accepted value, quoted in refusals.
    pub min: String,
    /// Largest accepted value, quoted in refusals.
    pub max: String,
    /// Slider/arrow step; `None` on non-slider rows.
    pub step: Option<String>,
    /// Allowed values in display order; `None` off dropdown rows.
    pub options: Option<&'static [&'static str]>,
    /// Why the cell never edits; `None` on editable rows.
    pub read_only_reason: Option<&'static str>,
    /// Registry current version for version rows; `None` elsewhere.
    pub pinned_version: Option<u32>,
}

impl AutoRow {
    /// True for cells the panel never stages.
    pub fn is_read_only(&self) -> bool {
        self.kind.is_read_only() || self.read_only_reason.is_some()
    }
}

/// Owned grid: rows in component, field order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutoGrid {
    rows: Vec<AutoRow>,
}

impl AutoGrid {
    /// Empty grid.
    pub fn new() -> Self {
        Self::default()
    }

    /// Row count.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True when no row rendered.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Rows in display order.
    pub fn rows(&self) -> &[AutoRow] {
        &self.rows
    }

    /// True when every [`SchemaKind`] renders at least one row.
    pub fn covers_all_kinds(&self) -> bool {
        SchemaKind::all()
            .iter()
            .all(|kind| self.rows.iter().any(|row| row.kind == *kind))
    }
}

/// What rejected an automatic inspector action. Refusals stage nothing.
#[derive(Clone, Debug, PartialEq)]
pub enum AutoError {
    /// No row matches this component and field.
    UnknownField {
        /// The requested component.
        component: String,
        /// The requested field.
        field: String,
    },
    /// The cell is read-only; `reason` names why.
    ReadOnly {
        /// Component holding the cell.
        component: String,
        /// Field holding the cell.
        field: String,
        /// Why the cell never edits.
        reason: String,
    },
    /// The value is outside the cell range; the range is carried to quote.
    BadValue {
        /// Component holding the cell.
        component: String,
        /// Field holding the cell.
        field: String,
        /// The refused text.
        value: String,
        /// Legal minimum quoted in the refusal.
        min: String,
        /// Legal maximum quoted in the refusal.
        max: String,
    },
    /// The value names no dropdown option; `options` quotes the legal set.
    BadOption {
        /// Component holding the cell.
        component: String,
        /// Field holding the cell.
        field: String,
        /// The refused text.
        value: String,
        /// Legal values in display order.
        options: &'static [&'static str],
    },
    /// The value is not finite; nothing was staged.
    NonFinite {
        /// Component holding the cell.
        component: String,
        /// Field holding the cell.
        field: String,
    },
    /// The batch base does not match the store revision; nothing ran.
    StaleRevision {
        /// Current store revision.
        expected: u64,
        /// Revision stamped on the batch.
        got: u64,
    },
    /// The key was already used by another payload; nothing ran.
    DuplicateKey {
        /// The reused key.
        key: u64,
    },
    /// The key belongs to an undone batch; submit a new batch to reapply.
    UndoneKey {
        /// The key of the undone batch.
        key: u64,
    },
    /// The staged batch is fuller than the op bound.
    TooManyOps {
        /// Staged op count.
        count: usize,
        /// Bound exceeded.
        max: usize,
    },
}

impl fmt::Display for AutoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownField { component, field } => {
                write!(f, "unknown field {component}.{field}")
            }
            Self::ReadOnly {
                component,
                field,
                reason,
            } => write!(f, "{component}.{field} is read-only: {reason}"),
            Self::BadValue {
                component,
                field,
                value,
                min,
                max,
            } => write!(
                f,
                "{component}.{field} value `{value}` outside {min}..={max}: refused, nothing staged"
            ),
            Self::BadOption {
                component,
                field,
                value,
                options,
            } => {
                let mut legal = String::new();
                for (i, option) in options.iter().enumerate() {
                    if i > 0 {
                        legal.push('|');
                    }
                    legal.push_str(option);
                }
                write!(
                    f,
                    "{component}.{field} value `{value}` is not one of {legal}: refused, nothing staged"
                )
            }
            Self::NonFinite { component, field } => {
                write!(
                    f,
                    "{component}.{field} must be finite: refused, nothing staged"
                )
            }
            Self::StaleRevision { expected, got } => write!(
                f,
                "stale batch: store is at {expected}, batch expects {got}"
            ),
            Self::DuplicateKey { key } => {
                write!(
                    f,
                    "idempotency key {key} was already used with another payload"
                )
            }
            Self::UndoneKey { key } => write!(
                f,
                "idempotency key {key} belongs to an undone batch; submit a new batch to reapply"
            ),
            Self::TooManyOps { count, max } => {
                write!(f, "batch holds {count} ops over the {max} bound")
            }
        }
    }
}

impl std::error::Error for AutoError {}

/// Format a hint bound without trailing noise (`9`, `0.1`, `-10000`).
fn fmt_bound(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Bound a cell text to [`MAX_TEXT_LEN`] at a character boundary.
fn bound_text(value: &str) -> String {
    bound_to(value, MAX_TEXT_LEN)
}

/// Build one declared row from its field schema over caller values.
///
/// One row builder per declared field carrying its min/max/step/dropdown/
/// read-only hints; the caller keeps declaration order. Pattern-only port of
/// the facet declared-fields shape over the existing registry, no reflection
/// runtime and no new dependency.
fn auto_row_for(
    schema: &FieldSchema,
    registry: &SchemaRegistry,
    values: &BTreeMap<(String, String), String>,
) -> AutoRow {
    let key = (schema.component.to_string(), schema.field.to_string());
    let live = values
        .get(&key)
        .map_or_else(|| schema.default.to_string(), |v| bound_text(v));
    let (min, max) = (fmt_bound(schema.min), fmt_bound(schema.max));
    let step = match schema.kind {
        SchemaKind::IntegerSlider
        | SchemaKind::FloatSlider
        | SchemaKind::Percentage
        | SchemaKind::AngleDeg
        | SchemaKind::DistanceM
        | SchemaKind::ScaleFactor
        | SchemaKind::QuatLane
        | SchemaKind::DurationSecs
        | SchemaKind::Count => Some(fmt_bound(schema.step)),
        _ => None,
    };
    let options = match schema.kind {
        SchemaKind::Dropdown => Some(schema.options),
        _ => None,
    };
    let (read_only_reason, pinned_version) = match schema.kind {
        SchemaKind::Version => match registry.current_version(schema.component) {
            Some(current) => (Some(schema.read_only_reason), Some(current)),
            None => (Some("unknown component: no RC4 schema serves it"), None),
        },
        SchemaKind::ReadOnly => (Some(schema.read_only_reason), None),
        _ => (None, None),
    };
    AutoRow {
        component: schema.component.to_string(),
        field: schema.field.to_string(),
        kind: schema.kind,
        value: live,
        min,
        max,
        step,
        options,
        read_only_reason,
        pinned_version,
    }
}

/// Build a grid from the given declared fields in declaration order.
///
/// Rows follow the slice order, one per declared field; bad values refuse
/// naming field and reason via [`validate_edit`]. No reflection runtime and
/// no new dependency.
pub fn build_declared_grid(
    schemas: &[FieldSchema],
    registry: &SchemaRegistry,
    values: &BTreeMap<(String, String), String>,
) -> AutoGrid {
    let mut rows = Vec::with_capacity(schemas.len());
    for schema in schemas {
        rows.push(auto_row_for(schema, registry, values));
    }
    AutoGrid { rows }
}

/// Build the automatic grid from the 20 field schemas over caller values.
///
/// `values` maps `(component, field)` to live text; missing entries fall
/// back to the schema default. When `registry` knows a component, its
/// version row pins at the served current version; components the registry
/// never serves render their version row read-only with the cause named.
pub fn build_grid(
    registry: &SchemaRegistry,
    values: &BTreeMap<(String, String), String>,
) -> AutoGrid {
    let mut grid = build_declared_grid(&field_schemas(), registry, values);
    grid.rows
        .sort_by(|a, b| (&a.component, &a.field).cmp(&(&b.component, &b.field)));
    grid
}

/// Declared fields for the registry panel: the first twelve schemas in order.
///
/// Reuses [`field_schemas`] with no new schema kind; rows render through
/// [`build_declared_grid`] with [`validate_edit`] refusals.
pub fn registry_panel_grid(
    registry: &SchemaRegistry,
    values: &BTreeMap<(String, String), String>,
) -> AutoGrid {
    let schemas = field_schemas();
    build_declared_grid(&schemas[..12], registry, values)
}

/// Find one grid row by component and field.
fn find_row<'a>(
    grid: &'a AutoGrid,
    component: &str,
    field: &str,
) -> Result<&'a AutoRow, AutoError> {
    grid.rows
        .iter()
        .find(|row| row.component == component && row.field == field)
        .ok_or_else(|| AutoError::UnknownField {
            component: component.to_string(),
            field: field.to_string(),
        })
}

/// Parse one finite float cell, refusing blanks and non-finite text.
fn parse_float(row: &AutoRow, value: &str) -> Result<f64, AutoError> {
    let parsed: f64 = value.trim().parse().map_err(|_| AutoError::NonFinite {
        component: row.component.clone(),
        field: row.field.clone(),
    })?;
    if !parsed.is_finite() {
        return Err(AutoError::NonFinite {
            component: row.component.clone(),
            field: row.field.clone(),
        });
    }
    Ok(parsed)
}

/// Refuse when `parsed` leaves `min..=max`, naming the legal range.
fn check_range(row: &AutoRow, parsed: f64, min: f64, max: f64) -> Result<(), AutoError> {
    if parsed < min || parsed > max {
        return Err(AutoError::BadValue {
            component: row.component.clone(),
            field: row.field.clone(),
            value: bound_text(&parsed.to_string()),
            min: row.min.clone(),
            max: row.max.clone(),
        });
    }
    Ok(())
}

/// Refusal quoting the raw `value` plus the legal range text.
fn bad_value(row: &AutoRow, value: &str, min: &str, max: &str) -> AutoError {
    AutoError::BadValue {
        component: row.component.clone(),
        field: row.field.clone(),
        value: bound_text(value),
        min: min.to_string(),
        max: max.to_string(),
    }
}

/// Refusal for a cell that never edits, quoting the row's own reason.
fn read_only(row: &AutoRow) -> AutoError {
    AutoError::ReadOnly {
        component: row.component.clone(),
        field: row.field.clone(),
        reason: row
            .read_only_reason
            .unwrap_or("cell never edits")
            .to_string(),
    }
}

/// Validate one edited text against its row hints.
pub fn validate_edit(row: &AutoRow, value: &str) -> Result<(), AutoError> {
    if row.is_read_only() {
        return Err(read_only(row));
    }
    // The schema table carries every numeric bound; parse it back once so
    // the refusal quotes the exact row hints the builder rendered.
    let min: f64 = row.min.parse().unwrap_or(f64::MIN);
    let max: f64 = row.max.parse().unwrap_or(f64::MAX);
    match row.kind {
        SchemaKind::Text => {
            if value.chars().count() > MAX_TEXT_LEN {
                return Err(bad_value(
                    row,
                    value,
                    "0 chars",
                    &format!("{MAX_TEXT_LEN} chars"),
                ));
            }
            Ok(())
        }
        SchemaKind::Integer | SchemaKind::IntegerSlider | SchemaKind::Count => {
            let parsed: i64 = value
                .trim()
                .parse()
                .map_err(|_| bad_value(row, value, &row.min, &row.max))?;
            check_range(row, parsed as f64, min, max)
        }
        SchemaKind::Float
        | SchemaKind::FloatSlider
        | SchemaKind::Percentage
        | SchemaKind::AngleDeg
        | SchemaKind::DistanceM
        | SchemaKind::ScaleFactor
        | SchemaKind::QuatLane
        | SchemaKind::DurationSecs => {
            let parsed = parse_float(row, value)?;
            check_range(row, parsed, min, max)
        }
        SchemaKind::Bool => {
            if parse_bool(value).is_none() {
                return Err(bad_value(row, value, "false", "true"));
            }
            Ok(())
        }
        SchemaKind::Dropdown => {
            let options = row.options.unwrap_or(&[]);
            if options.iter().any(|option| *option == value.trim()) {
                Ok(())
            } else {
                Err(AutoError::BadOption {
                    component: row.component.clone(),
                    field: row.field.clone(),
                    value: bound_text(value),
                    options,
                })
            }
        }
        SchemaKind::ColorRgb => check_color(row, value, min, max),
        SchemaKind::Vec3 => {
            let lanes: Vec<&str> = value.split(',').map(str::trim).collect();
            if lanes.len() != 3 {
                return Err(bad_value(row, value, &row.min, &row.max));
            }
            for lane in lanes {
                let parsed = parse_float(row, lane)?;
                check_range(row, parsed, min, max)?;
            }
            Ok(())
        }
        SchemaKind::EntityRef => {
            let parsed: u64 = value
                .trim()
                .parse()
                .map_err(|_| bad_value(row, value, &row.min, &row.max))?;
            if parsed == 0 {
                return Err(bad_value(row, value, &row.min, &row.max));
            }
            check_range(row, parsed as f64, min, max)
        }
        SchemaKind::AssetPath => check_asset_path(row, value),
        SchemaKind::Version | SchemaKind::ReadOnly => Err(read_only(row)),
    }
}

/// Validate an `r,g,b` cell: three integer lanes inside the row range. Every
/// refusal quotes the full `0,0,0..255,255,255` color range.
fn check_color(row: &AutoRow, value: &str, min: f64, max: f64) -> Result<(), AutoError> {
    let color_refusal = || bad_value(row, value, "0,0,0", "255,255,255");
    let lanes: Vec<&str> = value.split(',').map(str::trim).collect();
    if lanes.len() != 3 {
        return Err(color_refusal());
    }
    for lane in lanes {
        let parsed: i64 = lane.parse().map_err(|_| color_refusal())?;
        check_range(row, parsed as f64, min, max).map_err(|_| color_refusal())?;
    }
    Ok(())
}

/// Validate an asset path cell: 1..=256 bytes ending in a known suffix.
fn check_asset_path(row: &AutoRow, value: &str) -> Result<(), AutoError> {
    let text = value.trim();
    if text.is_empty() || text.len() > 256 {
        return Err(bad_value(
            row,
            value,
            "1 char",
            "256 chars, ending in .glb|.png|.wav",
        ));
    }
    if !ASSET_SUFFIXES.iter().any(|suffix| text.ends_with(suffix)) {
        return Err(bad_value(row, value, "name ending in", ".glb|.png|.wav"));
    }
    Ok(())
}

/// One staged field edit inside an [`AutoBatch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoOp {
    /// Component holding the field.
    pub component: String,
    /// Field within the component.
    pub field: String,
    /// Value before the edit.
    pub old_value: String,
    /// Value after the edit.
    pub new_value: String,
}

/// Atomic edit batch: every op applies together or none does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoBatch {
    /// Edits in stage order.
    pub ops: Vec<AutoOp>,
    /// Store revision the edits were staged against.
    pub base_revision: u64,
    /// Retry identity for the batch.
    pub idempotency_key: u64,
}

/// Dry-run outcome: one line per op plus the revision an apply would publish.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BatchPreview {
    /// One line per op, in batch order.
    pub effects: Vec<String>,
    /// `base_revision + 1`; the revision an apply would publish.
    pub resulting_revision: u64,
}

/// Value store the batches edit: live text plus a monotonic revision.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutoStore {
    values: BTreeMap<(String, String), String>,
    revision: u64,
}

impl AutoStore {
    /// Empty store at revision zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current revision.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Live value for one cell, if ever written.
    pub fn get(&self, component: &str, field: &str) -> Option<&str> {
        self.values
            .get(&(component.to_string(), field.to_string()))
            .map(String::as_str)
    }
}

/// Undo/redo journal over applied batches plus idempotency keys.
#[derive(Clone, Debug, Default)]
pub struct AutoJournal {
    past: Vec<AutoBatch>,
    future: Vec<AutoBatch>,
    seen: BTreeSet<u64>,
    undone: BTreeSet<u64>,
}

impl AutoJournal {
    /// Empty journal.
    pub fn new() -> Self {
        Self::default()
    }

    /// Depth of the undo stack.
    pub fn past_len(&self) -> usize {
        self.past.len()
    }

    /// Depth of the redo stack.
    pub fn future_len(&self) -> usize {
        self.future.len()
    }
}

/// Preview staged edits without side effects: one line per op plus the
/// revision an apply would publish. Never touches any store.
pub fn preview_edits(
    grid: &AutoGrid,
    edits: &BTreeMap<(String, String), String>,
    base_revision: u64,
) -> Result<BatchPreview, AutoError> {
    if edits.len() > MAX_OPS {
        return Err(AutoError::TooManyOps {
            count: edits.len(),
            max: MAX_OPS,
        });
    }
    let mut effects = Vec::with_capacity(edits.len());
    for ((component, field), value) in edits {
        let row = find_row(grid, component, field)?;
        validate_edit(row, value)?;
        effects.push(format!(
            "set {component}.{field} from `{}` to `{}`",
            row.value,
            bound_text(value)
        ));
    }
    Ok(BatchPreview {
        effects,
        resulting_revision: base_revision + 1,
    })
}

/// Emit staged edits as an atomic batch shape.
///
/// Validates every edit first: any failure returns the error and shapes
/// nothing. Never mutates any store; the caller applies the returned batch
/// with [`apply_batch`] (validate, then preview, then apply).
pub fn emit_batch(
    grid: &AutoGrid,
    edits: &BTreeMap<(String, String), String>,
    base_revision: u64,
    idempotency_key: u64,
    live_revision: u64,
) -> Result<AutoBatch, AutoError> {
    if live_revision != base_revision {
        return Err(AutoError::StaleRevision {
            expected: live_revision,
            got: base_revision,
        });
    }
    preview_edits(grid, edits, base_revision)?;
    let mut ops = Vec::with_capacity(edits.len());
    for ((component, field), value) in edits {
        let row = find_row(grid, component, field)?;
        ops.push(AutoOp {
            component: component.clone(),
            field: field.clone(),
            old_value: row.value.clone(),
            new_value: bound_text(value),
        });
    }
    Ok(AutoBatch {
        ops,
        base_revision,
        idempotency_key,
    })
}

/// Apply a batch atomically through the journal: validate the revision,
/// check idempotency, then commit every op and bump the revision by one.
/// A retry with the same key plus the same payload replays the stored
/// revision without re-applying; the same key with another payload is a
/// conflict, and the key of an undone batch is refused.
pub fn apply_batch(
    store: &mut AutoStore,
    journal: &mut AutoJournal,
    batch: &AutoBatch,
) -> Result<u64, AutoError> {
    if journal.undone.contains(&batch.idempotency_key) {
        return Err(AutoError::UndoneKey {
            key: batch.idempotency_key,
        });
    }
    if journal.seen.contains(&batch.idempotency_key) {
        let same = journal
            .past
            .iter()
            .any(|entry| entry.idempotency_key == batch.idempotency_key && entry.ops == batch.ops);
        if same {
            return Ok(store.revision);
        }
        return Err(AutoError::DuplicateKey {
            key: batch.idempotency_key,
        });
    }
    if batch.base_revision != store.revision {
        return Err(AutoError::StaleRevision {
            expected: store.revision,
            got: batch.base_revision,
        });
    }
    if batch.ops.len() > MAX_OPS {
        return Err(AutoError::TooManyOps {
            count: batch.ops.len(),
            max: MAX_OPS,
        });
    }
    for op in &batch.ops {
        store.values.insert(
            (op.component.clone(), op.field.clone()),
            op.new_value.clone(),
        );
    }
    store.revision += 1;
    journal.seen.insert(batch.idempotency_key);
    journal.past.push(batch.clone());
    journal.future.clear();
    Ok(store.revision)
}

/// Undo the newest applied batch as a new revision. None when empty.
pub fn undo_batch(store: &mut AutoStore, journal: &mut AutoJournal) -> Option<u64> {
    let entry = journal.past.pop()?;
    for op in &entry.ops {
        if op.old_value.is_empty() {
            store
                .values
                .remove(&(op.component.clone(), op.field.clone()));
        } else {
            store.values.insert(
                (op.component.clone(), op.field.clone()),
                op.old_value.clone(),
            );
        }
    }
    store.revision += 1;
    journal.undone.insert(entry.idempotency_key);
    journal.future.push(entry);
    Some(store.revision)
}

/// Redo the newest undone batch as a new revision. None when empty.
pub fn redo_batch(store: &mut AutoStore, journal: &mut AutoJournal) -> Option<u64> {
    let entry = journal.future.pop()?;
    for op in &entry.ops {
        store.values.insert(
            (op.component.clone(), op.field.clone()),
            op.new_value.clone(),
        );
    }
    store.revision += 1;
    journal.undone.remove(&entry.idempotency_key);
    journal.past.push(entry);
    Some(store.revision)
}

/// Editor-shell bridge: stage one auto-grid edit as a transform batch.
///
/// The shell owns the scene, so pose-lane edits (`pose.px`, `pose.rx`,
/// `pose.sx` shapes) fold into the selection's live [`Transform`](engine_scene::transform::Transform)
/// and commit through [`Shell::apply_ops`](crate::shell::Shell::apply_ops):
/// validated first, journaled with undo/redo, never around the service.
/// Non-pose fields return [`AutoError::UnknownField`]: the shell has no
/// generic payload op, so only pose lanes cross this bridge.
pub fn apply_pose_edit_on_shell(
    shell: &mut crate::shell::Shell,
    target: engine_scene::ids::ObjectId,
    grid: &AutoGrid,
    component: &str,
    field: &str,
    value: &str,
) -> Result<crate::commands::Applied, AutoError> {
    let row = find_row(grid, component, field)?;
    validate_edit(row, value)?;
    let live = shell
        .document()
        .hierarchy
        .local(target)
        .ok_or_else(|| AutoError::UnknownField {
            component: component.to_string(),
            field: field.to_string(),
        })?;
    let parsed = parse_float(row, value)?;
    let mut draft = live;
    match (component, field) {
        ("pose", "pose.px") => draft.position[0] = parsed as f32,
        ("pose", "pose.rx") => draft.rotation[0] = parsed as f32,
        ("pose", "pose.sx") => draft.scale[0] = parsed as f32,
        _ => {
            return Err(AutoError::UnknownField {
                component: component.to_string(),
                field: field.to_string(),
            });
        }
    }
    shell
        .apply_ops(vec![crate::commands::Op::SetLocal {
            id: target,
            local: draft,
        }])
        .map_err(|_| AutoError::StaleRevision {
            expected: shell.revision(),
            got: shell.revision(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto_grid() -> AutoGrid {
        build_grid(&SchemaRegistry::builtin(), &BTreeMap::new())
    }

    fn row(grid: &AutoGrid, component: &str, field: &str) -> AutoRow {
        find_row(grid, component, field).unwrap().clone()
    }

    #[test]
    fn twenty_schema_types_render_with_hints() {
        let grid = auto_grid();
        assert_eq!(grid.len(), 20);
        for kind in SchemaKind::all() {
            assert!(
                grid.rows.iter().any(|r| r.kind == kind),
                "schema type {kind} renders no row"
            );
        }
        assert!(grid.covers_all_kinds());
        // Hints ride each row: sliders carry a step, dropdowns carry options,
        // read-only rows carry their reason.
        let slider = row(&grid, "gadget", "tier");
        assert_eq!(slider.min, "0");
        assert_eq!(slider.max, "9");
        assert_eq!(slider.step.as_deref(), Some("1"));
        let float_slider = row(&grid, "pose", "pose.px");
        assert_eq!(float_slider.min, "-10000");
        assert_eq!(float_slider.max, "10000");
        assert_eq!(float_slider.step.as_deref(), Some("0.1"));
        let dropdown = row(&grid, "arena", "mode");
        assert_eq!(dropdown.options, Some(&MODES as &[&str]));
        let read_only = row(&grid, "museum", "note");
        assert!(read_only.is_read_only());
        assert_eq!(
            read_only.read_only_reason,
            Some("museum fixture loads read-only")
        );
        // Rows sort by component then field, so the same inputs rebuild the
        // same grid.
        let again = auto_grid();
        assert_eq!(grid, again);
        let mut keys: Vec<(String, String)> = grid
            .rows
            .iter()
            .map(|r| (r.component.clone(), r.field.clone()))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        let _ = &mut keys;
    }

    #[test]
    fn rc4_registry_pins_versions_and_names_unknown() {
        let grid = auto_grid();
        let version = row(&grid, "label", "version");
        assert_eq!(version.kind, SchemaKind::Version);
        assert!(version.is_read_only());
        assert_eq!(version.pinned_version, Some(1));
        // Components the registry never serves stay read-only and name
        // the cause instead of guessing a version.
        let registry = SchemaRegistry::new();
        let grid = build_grid(&registry, &BTreeMap::new());
        let version = row(&grid, "label", "version");
        assert!(version.is_read_only());
        assert_eq!(version.pinned_version, None);
        assert_eq!(
            version.read_only_reason,
            Some("unknown component: no RC4 schema serves it")
        );
        assert!(validate_edit(&version, "1").is_err());
    }

    #[test]
    fn five_out_of_range_edits_refused_with_range_named() {
        let grid = auto_grid();
        // 1. Integer slider over 0..=9.
        let tier = row(&grid, "gadget", "tier");
        let error = validate_edit(&tier, "99").unwrap_err();
        assert!(matches!(error, AutoError::BadValue { .. }));
        assert!(error.to_string().contains("0..=9"));
        // 2. Float slider over -10000..=10000.
        let lane = row(&grid, "pose", "pose.px");
        let error = validate_edit(&lane, "20000").unwrap_err();
        assert!(matches!(error, AutoError::BadValue { .. }));
        assert!(error.to_string().contains("-10000..=10000"));
        // 3. Percentage over 0..=100.
        let mix = row(&grid, "tune", "mix");
        let error = validate_edit(&mix, "150").unwrap_err();
        assert!(error.to_string().contains("0..=100"));
        // 4. Angle over -180..=180.
        let yaw = row(&grid, "pose", "yaw_deg");
        let error = validate_edit(&yaw, "270").unwrap_err();
        assert!(error.to_string().contains("-180..=180"));
        // 5. Scale under 0.001..=1000.
        let scale = row(&grid, "pose", "pose.sx");
        let error = validate_edit(&scale, "0").unwrap_err();
        assert!(error.to_string().contains("0.001..=1000"));
        // The dropdown refusal names the legal set beside the five ranges.
        let mode = row(&grid, "arena", "mode");
        let error = validate_edit(&mode, "arena").unwrap_err();
        assert!(matches!(error, AutoError::BadOption { .. }));
        assert!(error.to_string().contains("ring|pit|hall"));
        // In-range edits pass on the same rows.
        validate_edit(&tier, "7").unwrap();
        validate_edit(&lane, "3.5").unwrap();
        validate_edit(&mix, "25").unwrap();
        validate_edit(&yaw, "-90").unwrap();
        validate_edit(&scale, "2").unwrap();
        validate_edit(&mode, "pit").unwrap();
    }

    #[test]
    fn read_only_and_non_finite_never_stage() {
        let grid = auto_grid();
        let note = row(&grid, "museum", "note");
        let error = validate_edit(&note, "other").unwrap_err();
        assert!(matches!(error, AutoError::ReadOnly { .. }));
        assert!(error.to_string().contains("museum fixture loads read-only"));
        let lane = row(&grid, "pose", "pose.px");
        assert!(matches!(
            validate_edit(&lane, "nan").unwrap_err(),
            AutoError::NonFinite { .. }
        ));
        let enabled = row(&grid, "gadget", "enabled");
        validate_edit(&enabled, "yes").unwrap();
        assert!(matches!(
            validate_edit(&enabled, "maybe").unwrap_err(),
            AutoError::BadValue { .. }
        ));
    }

    #[test]
    fn preview_and_emit_validate_then_shape_without_mutation() {
        let grid = auto_grid();
        let store = AutoStore::new();
        let mut edits = BTreeMap::new();
        edits.insert(
            ("label".to_string(), "text".to_string()),
            "Hall".to_string(),
        );
        edits.insert(("gadget".to_string(), "tier".to_string()), "7".to_string());
        let preview = preview_edits(&grid, &edits, 0).unwrap();
        assert_eq!(preview.effects.len(), 2);
        assert_eq!(preview.resulting_revision, 1);
        let batch = emit_batch(&grid, &edits, 0, 41, 0).unwrap();
        assert_eq!(batch.base_revision, 0);
        assert_eq!(batch.idempotency_key, 41);
        assert_eq!(batch.ops.len(), 2);
        assert_eq!(store.revision(), 0);
        // One bad edit blocks the whole batch and stages nothing.
        edits.insert(
            ("pose".to_string(), "pose.px".to_string()),
            "nan".to_string(),
        );
        assert!(matches!(
            preview_edits(&grid, &edits, 0).unwrap_err(),
            AutoError::NonFinite { .. }
        ));
        assert!(matches!(
            emit_batch(&grid, &edits, 0, 42, 0).unwrap_err(),
            AutoError::NonFinite { .. }
        ));
        // Unknown fields refuse with their address.
        let mut missing = BTreeMap::new();
        missing.insert(
            ("label".to_string(), "missing".to_string()),
            "x".to_string(),
        );
        assert!(matches!(
            preview_edits(&grid, &missing, 0).unwrap_err(),
            AutoError::UnknownField { .. }
        ));
    }

    #[test]
    fn apply_undo_redo_restore_values_with_new_revisions() {
        let grid = auto_grid();
        let mut store = AutoStore::new();
        let mut journal = AutoJournal::new();
        let mut edits = BTreeMap::new();
        edits.insert(
            ("label".to_string(), "text".to_string()),
            "Hall".to_string(),
        );
        let batch = emit_batch(&grid, &edits, 0, 7, 0).unwrap();
        apply_batch(&mut store, &mut journal, &batch).unwrap();
        assert_eq!(store.get("label", "text"), Some("Hall"));
        assert_eq!(store.revision(), 1);
        // Same key plus same payload replays without re-applying.
        assert_eq!(apply_batch(&mut store, &mut journal, &batch).unwrap(), 1);
        // Same key with another payload is a conflict.
        let mut other = BTreeMap::new();
        other.insert(
            ("label".to_string(), "text".to_string()),
            "Other".to_string(),
        );
        let clash = emit_batch(&grid, &other, 1, 7, 1).unwrap();
        assert_eq!(
            apply_batch(&mut store, &mut journal, &clash).unwrap_err(),
            AutoError::DuplicateKey { key: 7 }
        );
        // Undo restores the default and moves the revision on (never reused).
        undo_batch(&mut store, &mut journal).expect("journal holds the batch");
        assert_eq!(store.get("label", "text"), None);
        assert_eq!(store.revision(), 2);
        // The undone key stays refused; redo reapplies under a new revision.
        assert_eq!(
            apply_batch(&mut store, &mut journal, &batch).unwrap_err(),
            AutoError::UndoneKey { key: 7 }
        );
        redo_batch(&mut store, &mut journal).expect("redo holds the batch");
        assert_eq!(store.get("label", "text"), Some("Hall"));
        assert_eq!(store.revision(), 3);
        assert_eq!(journal.past_len(), 1);
        assert_eq!(journal.future_len(), 0);
    }

    #[test]
    fn stale_batches_refuse_before_anything_applies() {
        let grid = auto_grid();
        let mut store = AutoStore::new();
        let mut journal = AutoJournal::new();
        let mut edits = BTreeMap::new();
        edits.insert(("gadget".to_string(), "tier".to_string()), "3".to_string());
        assert_eq!(
            emit_batch(&grid, &edits, 0, 1, 2).unwrap_err(),
            AutoError::StaleRevision {
                expected: 2,
                got: 0
            }
        );
        let batch = emit_batch(&grid, &edits, 0, 1, 0).unwrap();
        apply_batch(&mut store, &mut journal, &batch).unwrap();
        let late = emit_batch(&grid, &edits, 0, 2, 0).unwrap();
        assert_eq!(
            apply_batch(&mut store, &mut journal, &late).unwrap_err(),
            AutoError::StaleRevision {
                expected: 1,
                got: 0
            }
        );
        assert_eq!(store.revision(), 1);
    }

    #[test]
    fn shell_consumer_applies_pose_lane_with_undo() {
        use crate::shell::Shell;
        use engine_scene::ids::ObjectId;
        use engine_scene::transform::Transform;

        let mut shell = Shell::new();
        let root = ObjectId::new();
        shell
            .insert_node(root, ObjectId::NIL, Transform::identity())
            .unwrap();
        assert!(shell.select(root));
        // The auto grid renders the pose lanes the shell bridge edits.
        let grid = auto_grid();
        assert!(grid.rows.iter().any(|r| r.field == "pose.px"));
        apply_pose_edit_on_shell(&mut shell, root, &grid, "pose", "pose.px", "4.5").unwrap();
        let local = shell.document().hierarchy.local(root).unwrap();
        assert!((local.position[0] - 4.5).abs() <= 1e-5);
        // Out-of-range lane edits refuse naming the range and write nothing.
        let before = shell.document().clone();
        let error = apply_pose_edit_on_shell(&mut shell, root, &grid, "pose", "pose.px", "20000")
            .unwrap_err();
        assert!(error.to_string().contains("-10000..=10000"));
        assert_eq!(*shell.document(), before);
        // The shell journal undoes the auto edit like any command batch.
        shell.undo().expect("journal holds the auto pose batch");
        let local = shell.document().hierarchy.local(root).unwrap();
        assert!((local.position[0] - 0.0).abs() <= 1e-5);
        shell.redo().expect("redo holds the auto pose batch");
        let local = shell.document().hierarchy.local(root).unwrap();
        assert!((local.position[0] - 4.5).abs() <= 1e-5);
    }

    #[test]
    fn auto_grid_declared_twelve_in_order_with_three_refusals() {
        let registry = SchemaRegistry::builtin();
        let schemas = field_schemas();
        let twelve = &schemas[..12];
        assert_eq!(twelve.len(), 12);
        let grid = build_declared_grid(twelve, &registry, &BTreeMap::new());
        assert_eq!(grid.len(), 12);
        let order: Vec<(&str, &str)> = grid
            .rows()
            .iter()
            .map(|row| (row.component.as_str(), row.field.as_str()))
            .collect();
        let declared: Vec<(&str, &str)> = twelve
            .iter()
            .map(|schema| (schema.component, schema.field))
            .collect();
        assert_eq!(order, declared);
        let bad = [
            ("gadget", "tier", "99"),
            ("pose", "pose.px", "20000"),
            ("tune", "mix", "150"),
        ];
        for (component, field, value) in bad {
            let row = grid
                .rows()
                .iter()
                .find(|row| row.component == component && row.field == field)
                .expect("declared row renders");
            let error = validate_edit(row, value).unwrap_err();
            let text = error.to_string();
            assert!(
                text.contains(component) && text.contains(field),
                "refusal names field, got: {text}"
            );
            assert!(
                text.contains("refused"),
                "refusal names reason, got: {text}"
            );
        }
        let tier = grid
            .rows()
            .iter()
            .find(|row| row.field == "tier")
            .expect("tier renders");
        validate_edit(tier, "7").unwrap();
    }
}
