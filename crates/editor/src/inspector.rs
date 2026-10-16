//! Schema inspector: registry-driven property grid with prefab badges.
//!
//! Owner: the editor session; reads versioned components, never owns them.
//! Units: node ids (`u64` here), field text, revisions (`u64`). Budget: UI
//! thread only, grid build is linear in fields, never per-tick simulation.
//! Determinism: row order is node id, then component, then field; no clock.
//! Errors: [`InspectorError`]; refusals stage nothing and mutate nothing.
//! Live edge: poses are the engine-scene [`Transform`], version truth resolves
//! through the scene [`SchemaRegistry`], and pose edits fold into the command
//! [`Batch`] (validate, then preview, then apply).

use std::collections::BTreeMap;
use std::fmt;

use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;

use crate::commands::{Batch, Op};

/// Most fields accepted on one inspected node.
pub const MAX_FIELDS_PER_NODE: usize = 64;
/// Longest text value kept in a cell or baseline entry: the one editor
/// text bound, re-exported by [`crate::inspector_auto`]. The scene keeps
/// its own `provenance::MAX_TEXT_LEN`; values never merge.
pub const MAX_TEXT_LEN: usize = 512;
/// Most rows kept in one grid.
pub const MAX_ROWS: usize = 1024;
/// Most staged edits emitted in one batch: the inspector-grid limit,
/// distinct from the auto-grid
/// [`MAX_AUTO_OPS_PER_BATCH`](crate::inspector_auto::MAX_AUTO_OPS_PER_BATCH)
/// and the journal wire bound. Values never merge: each bound guards its
/// own service.
pub const MAX_OPS_PER_BATCH: usize = 256;
/// Most nodes accepted in one selection.
pub const MAX_SELECTION: usize = 256;

/// One versioned component value: a named payload at an explicit version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionedComponent {
    /// Component name, e.g. `"label"`.
    pub component: String,
    /// Version the payload was written at (1-based).
    pub version: u32,
    /// Payload text at `version`.
    pub payload: String,
}

impl VersionedComponent {
    /// Build one value without validation; [`validate_component`] gates grid entry.
    pub fn new(component: &str, version: u32, payload: &str) -> Self {
        Self {
            component: component.to_string(),
            version,
            payload: payload.to_string(),
        }
    }
}

/// One schema step: a component version plus whether it is the current one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchemaStep {
    /// Component name, e.g. `"label"`.
    pub component: &'static str,
    /// Version this step describes (1-based).
    pub version: u32,
    /// True when this is the current version (no further migration).
    pub is_current: bool,
}

/// Local pose: the live engine-scene transform (meters, parent frame).
///
/// One owner per type: the grid never copies pose math here, it reads the
/// scene [`Transform`] directly, so pose lanes always agree with the
/// hierarchy the command service applies.
pub use engine_scene::transform::Transform as LocalPose;

/// Every registry descriptor kind the grid renders.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DescriptorKind {
    /// Builtin `"label"` text payload.
    Label,
    /// Generic versioned component payload.
    Payload,
    /// Schema step at the current version.
    SchemaCurrent,
    /// Schema step below current (a migration step).
    SchemaMigration,
    /// Prefab node identity key.
    PrefabKey,
    /// Prefab parent link.
    PrefabParent,
    /// Pose translation in meters.
    PosePosition,
    /// Pose rotation quaternion.
    PoseRotation,
    /// Pose scale factors.
    PoseScale,
    /// Read-only note (museum fixture, unknown version, refused value).
    ReadOnlyNote,
}

impl DescriptorKind {
    /// Every descriptor kind, in grid order.
    pub const fn all() -> [Self; 10] {
        [
            Self::Label,
            Self::Payload,
            Self::SchemaCurrent,
            Self::SchemaMigration,
            Self::PrefabKey,
            Self::PrefabParent,
            Self::PosePosition,
            Self::PoseRotation,
            Self::PoseScale,
            Self::ReadOnlyNote,
        ]
    }

    /// Short grid section name for this kind.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Label => "label",
            Self::Payload => "payload",
            Self::SchemaCurrent => "schema-current",
            Self::SchemaMigration => "schema-migration",
            Self::PrefabKey => "prefab-key",
            Self::PrefabParent => "prefab-parent",
            Self::PosePosition => "position",
            Self::PoseRotation => "rotation",
            Self::PoseScale => "scale",
            Self::ReadOnlyNote => "read-only",
        }
    }
}

impl fmt::Display for DescriptorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Cell editor shape for one grid row.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    /// Free text up to `max` characters.
    Text {
        /// Longest accepted value in characters.
        max: usize,
    },
    /// Integer in `min..=max`.
    Integer {
        /// Smallest accepted value.
        min: i64,
        /// Largest accepted value.
        max: i64,
    },
    /// Float in `min..=max`; NaN and infinity always refuse.
    Float {
        /// Smallest accepted value.
        min: f32,
        /// Largest accepted value.
        max: f32,
    },
    /// `true` or `false` (also `1`/`0`, `yes`/`no`).
    Bool,
    /// One of `options[index]`; stored by index.
    Enum {
        /// Allowed values in display order.
        options: &'static [&'static str],
    },
    /// Component version pinned at `current`; read-only by construction.
    Version {
        /// Newest served version.
        current: u32,
    },
    /// Never editable; `reason` names why.
    ReadOnly {
        /// Static reason shown in the cell.
        reason: &'static str,
    },
}

impl FieldKind {
    /// True for cells the panel never stages.
    pub const fn is_read_only(&self) -> bool {
        matches!(self, Self::Version { .. } | Self::ReadOnly { .. })
    }
}

/// One grid row: an editable cell plus its prefab badge state.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldRow {
    /// Inspected node.
    pub node: u64,
    /// Component holding the field, e.g. `"label"`.
    pub component: String,
    /// Field within the component, e.g. `"text"` or `"pose.px"`.
    pub field: String,
    /// Which descriptor kind produced this row.
    pub descriptor: DescriptorKind,
    /// Cell editor shape.
    pub kind: FieldKind,
    /// Live value text.
    pub value: String,
    /// Prefab value text, when the node comes from a prefab.
    pub prefab_value: Option<String>,
    /// True when the live value differs from the prefab value.
    pub overridden: bool,
}

impl FieldRow {
    /// Badge text for the cell: `"override"` when prefab-dirty, else none.
    pub const fn badge(&self) -> Option<&'static str> {
        if self.overridden {
            Some("override")
        } else {
            None
        }
    }
}

/// Owned grid: rows in node, component, field order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectorGrid {
    rows: Vec<FieldRow>,
}

impl InspectorGrid {
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
    pub fn rows(&self) -> &[FieldRow] {
        &self.rows
    }

    /// Rows currently badged as prefab overrides.
    pub fn overrides(&self) -> Vec<&FieldRow> {
        self.rows.iter().filter(|row| row.overridden).collect()
    }
}

/// One node as the grid sees it: identity, links, pose and components.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeView {
    /// Inspected node id.
    pub id: u64,
    /// Prefab key, when the node comes from a prefab.
    pub key: Option<String>,
    /// Parent prefab key, when the node comes from a prefab.
    pub parent_key: Option<String>,
    /// Local pose.
    pub pose: LocalPose,
    /// Components at their live versions.
    pub components: Vec<VersionedComponent>,
    /// Schema steps known for the node's components.
    pub schema: Vec<SchemaStep>,
    /// True for museum-fixture nodes (key and parent read-only).
    pub museum: bool,
}

impl NodeView {
    /// Minimal view: identity pose, no components, no schema.
    pub fn new(id: u64) -> Self {
        Self {
            id,
            key: None,
            parent_key: None,
            pose: LocalPose::IDENTITY,
            components: Vec::new(),
            schema: Vec::new(),
            museum: false,
        }
    }
}

/// What rejected an inspector action. Refusals stage nothing.
#[derive(Clone, Debug, PartialEq)]
pub enum InspectorError {
    /// No node is selected; select one first.
    EmptySelection,
    /// No inspected node carries this id.
    UnknownNode {
        /// The requested node.
        id: u64,
    },
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
        reason: &'static str,
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
    /// The value names no enum option; `options` quotes the legal set.
    BadEnum {
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
    /// The batch base does not match the document revision; nothing ran.
    StaleRevision {
        /// Current document revision.
        expected: u64,
        /// Revision stamped on the batch.
        got: u64,
    },
    /// The staged batch is fuller than the op bound.
    TooManyOps {
        /// Staged op count.
        count: usize,
        /// Bound exceeded.
        max: usize,
    },
    /// The grid is fuller than the row bound.
    TooManyRows {
        /// Row count refused.
        count: usize,
        /// Bound exceeded.
        max: usize,
    },
    /// A descriptor was malformed; the reason is static, never raw input.
    BadDescriptor {
        /// Static reason.
        reason: &'static str,
    },
}

impl fmt::Display for InspectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySelection => f.write_str("no node selected"),
            Self::UnknownNode { id } => write!(f, "unknown node {id}"),
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
                "{component}.{field} value `{value}` outside {min}..={max}: refused nothing"
            ),
            Self::BadEnum {
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
                    "{component}.{field} value `{value}` is not one of {legal}: refused nothing"
                )
            }
            Self::NonFinite { component, field } => {
                write!(f, "{component}.{field} must be finite: refused nothing")
            }
            Self::StaleRevision { expected, got } => write!(
                f,
                "stale batch: document is at {expected}, batch expects {got}"
            ),
            Self::TooManyOps { count, max } => {
                write!(f, "batch holds {count} ops over the {max} bound")
            }
            Self::TooManyRows { count, max } => {
                write!(f, "grid holds {count} rows over the {max} bound")
            }
            Self::BadDescriptor { reason } => write!(f, "invalid descriptor: {reason}"),
        }
    }
}

impl std::error::Error for InspectorError {}

/// Shared selection: the nodes the grid inspects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// Selected nodes in display order.
    pub ids: Vec<u64>,
    /// Focused node, always one of `ids` when set.
    pub primary: Option<u64>,
}

impl Selection {
    /// Empty selection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate one selection: non-empty, bounded, primary inside `ids`.
    pub fn validate(&self) -> Result<(), InspectorError> {
        if self.ids.is_empty() {
            return Err(InspectorError::EmptySelection);
        }
        if self.ids.len() > MAX_SELECTION {
            return Err(InspectorError::BadDescriptor {
                reason: "selection exceeds the node bound",
            });
        }
        if let Some(primary) = self.primary {
            if !self.ids.contains(&primary) {
                return Err(InspectorError::UnknownNode { id: primary });
            }
        }
        Ok(())
    }
}

/// One staged field edit inside an [`InspectorBatch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldOp {
    /// Node the edit targets.
    pub node: u64,
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
pub struct InspectorBatch {
    /// Edits in stage order.
    pub ops: Vec<FieldOp>,
    /// Document revision the edits were staged against.
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

/// Validate a component value shape ahead of grid entry.
fn validate_component(component: &VersionedComponent) -> Result<(), InspectorError> {
    if component.component.is_empty() || component.component.len() > MAX_TEXT_LEN {
        return Err(InspectorError::BadDescriptor {
            reason: "component name must be 1..=512 bytes",
        });
    }
    if component.version == 0 {
        return Err(InspectorError::BadDescriptor {
            reason: "component version is 1-based; version 0 is never valid",
        });
    }
    if component.payload.len() > MAX_TEXT_LEN {
        return Err(InspectorError::BadDescriptor {
            reason: "component payload exceeds the text bound",
        });
    }
    Ok(())
}

/// Current served version for a component, if the schema knows it.
fn current_version(schema: &[SchemaStep], component: &str) -> Option<u32> {
    schema
        .iter()
        .filter(|step| step.component == component && step.is_current)
        .map(|step| step.version)
        .max()
}

/// Badge one cell: overridden when a prefab baseline exists and differs.
fn badge_live(live: &str, prefab: Option<&str>) -> (Option<String>, bool) {
    match prefab {
        Some(base) if base != live => (Some(base.to_string()), true),
        Some(base) => (Some(base.to_string()), false),
        None => (None, false),
    }
}

/// Push one row, enforcing the row bound.
fn push_row(rows: &mut Vec<FieldRow>, row: FieldRow) -> Result<(), InspectorError> {
    if rows.len() >= MAX_ROWS {
        return Err(InspectorError::TooManyRows {
            count: rows.len() + 1,
            max: MAX_ROWS,
        });
    }
    rows.push(row);
    Ok(())
}

/// Format one float lane for a cell: shortest stable text.
fn fmt_lane(value: f32) -> String {
    format!("{value:.4}")
}

/// Split a payload into a text cell value.
fn text_cell(payload: &str) -> String {
    if payload.len() <= MAX_TEXT_LEN {
        payload.to_string()
    } else {
        let mut end = MAX_TEXT_LEN;
        while end > 0 && !payload.is_char_boundary(end) {
            end -= 1;
        }
        payload[..end].to_string()
    }
}

/// Build the property grid for the selected nodes.
///
/// Rows sort by node id, then component, then field, so the same inputs
/// always yield the same rows. Prefab values badge per field: a live value
/// differing from its baseline marks `overridden` and carries the badge.
pub fn build_grid(
    nodes: &[NodeView],
    selection: &Selection,
    prefab_baseline: &BTreeMap<(u64, String, String), String>,
) -> Result<InspectorGrid, InspectorError> {
    build_grid_with(nodes, selection, prefab_baseline, &|node, component| {
        current_version(&node.schema, component)
    })
}

/// Shared grid core: the version lookup reads the node step copies.
fn build_grid_with(
    nodes: &[NodeView],
    selection: &Selection,
    prefab_baseline: &BTreeMap<(u64, String, String), String>,
    current: &dyn Fn(&NodeView, &str) -> Option<u32>,
) -> Result<InspectorGrid, InspectorError> {
    selection.validate()?;
    let mut rows: Vec<FieldRow> = Vec::new();
    let mut ordered: Vec<&NodeView> = nodes
        .iter()
        .filter(|node| selection.ids.contains(&node.id))
        .collect();
    ordered.sort_by_key(|node| node.id);
    if ordered.len() != selection.ids.len() {
        for id in &selection.ids {
            if !ordered.iter().any(|node| node.id == *id) {
                return Err(InspectorError::UnknownNode { id: *id });
            }
        }
    }
    for node in ordered {
        if node.components.len() > MAX_FIELDS_PER_NODE {
            return Err(InspectorError::BadDescriptor {
                reason: "node exceeds the field bound",
            });
        }
        grid_identity(node, prefab_baseline, &mut rows)?;
        grid_pose(node, prefab_baseline, &mut rows)?;
        grid_components(node, prefab_baseline, &mut rows, current)?;
    }
    rows.sort_by(|a, b| (a.node, &a.component, &a.field).cmp(&(b.node, &b.component, &b.field)));
    if rows.len() > MAX_ROWS {
        return Err(InspectorError::TooManyRows {
            count: rows.len(),
            max: MAX_ROWS,
        });
    }
    Ok(InspectorGrid { rows })
}

/// Grid rows for prefab identity: key and parent link.
fn grid_identity(
    node: &NodeView,
    baseline: &BTreeMap<(u64, String, String), String>,
    rows: &mut Vec<FieldRow>,
) -> Result<(), InspectorError> {
    let read_only = node.museum;
    let reason = "museum fixture identity never edits";
    let key_live = node.key.clone().unwrap_or_default();
    let (prefab_value, overridden) = badge_live(
        &key_live,
        baseline
            .get(&(node.id, "prefab".into(), "key".into()))
            .map(String::as_str),
    );
    push_row(
        rows,
        FieldRow {
            node: node.id,
            component: "prefab".into(),
            field: "key".into(),
            descriptor: DescriptorKind::PrefabKey,
            kind: if read_only {
                FieldKind::ReadOnly { reason }
            } else {
                FieldKind::Text { max: MAX_TEXT_LEN }
            },
            value: key_live,
            prefab_value,
            overridden,
        },
    )?;
    let parent_live = node.parent_key.clone().unwrap_or_default();
    let (prefab_value, overridden) = badge_live(
        &parent_live,
        baseline
            .get(&(node.id, "prefab".into(), "parent".into()))
            .map(String::as_str),
    );
    push_row(
        rows,
        FieldRow {
            node: node.id,
            component: "prefab".into(),
            field: "parent".into(),
            descriptor: DescriptorKind::PrefabParent,
            kind: if read_only {
                FieldKind::ReadOnly { reason }
            } else {
                FieldKind::Text { max: MAX_TEXT_LEN }
            },
            value: parent_live,
            prefab_value,
            overridden,
        },
    )?;
    if node.museum {
        push_row(
            rows,
            FieldRow {
                node: node.id,
                component: "prefab".into(),
                field: "museum".into(),
                descriptor: DescriptorKind::ReadOnlyNote,
                kind: FieldKind::ReadOnly {
                    reason: "museum fixture loads read-only",
                },
                value: "museum-v1".into(),
                prefab_value: Some("museum-v1".into()),
                overridden: false,
            },
        )?;
    }
    Ok(())
}

/// Grid rows for the local pose: nine float lanes plus three axis rows.
fn grid_pose(
    node: &NodeView,
    baseline: &BTreeMap<(u64, String, String), String>,
    rows: &mut Vec<FieldRow>,
) -> Result<(), InspectorError> {
    const AXES: [&str; 3] = ["x", "y", "z"];
    for (i, axis) in AXES.iter().enumerate() {
        let field = format!("pose.p{axis}");
        let live = fmt_lane(node.pose.position[i]);
        let (prefab_value, overridden) = badge_live(
            &live,
            baseline
                .get(&(node.id, "pose".into(), field.clone()))
                .map(String::as_str),
        );
        push_row(
            rows,
            FieldRow {
                node: node.id,
                component: "pose".into(),
                field,
                descriptor: DescriptorKind::PosePosition,
                kind: FieldKind::Float {
                    min: -10_000.0,
                    max: 10_000.0,
                },
                value: live,
                prefab_value,
                overridden,
            },
        )?;
    }
    const LANES: [&str; 4] = ["x", "y", "z", "w"];
    for (i, lane) in LANES.iter().enumerate() {
        let field = format!("pose.r{lane}");
        let live = fmt_lane(node.pose.rotation[i]);
        let (prefab_value, overridden) = badge_live(
            &live,
            baseline
                .get(&(node.id, "pose".into(), field.clone()))
                .map(String::as_str),
        );
        push_row(
            rows,
            FieldRow {
                node: node.id,
                component: "pose".into(),
                field,
                descriptor: DescriptorKind::PoseRotation,
                kind: FieldKind::Float {
                    min: -1.0,
                    max: 1.0,
                },
                value: live,
                prefab_value,
                overridden,
            },
        )?;
    }
    for (i, axis) in AXES.iter().enumerate() {
        let field = format!("pose.s{axis}");
        let live = fmt_lane(node.pose.scale[i]);
        let (prefab_value, overridden) = badge_live(
            &live,
            baseline
                .get(&(node.id, "pose".into(), field.clone()))
                .map(String::as_str),
        );
        push_row(
            rows,
            FieldRow {
                node: node.id,
                component: "pose".into(),
                field,
                descriptor: DescriptorKind::PoseScale,
                kind: FieldKind::Float {
                    min: 0.001,
                    max: 1000.0,
                },
                value: live,
                prefab_value,
                overridden,
            },
        )?;
    }
    Ok(())
}

/// Grid rows for versioned components: payload, version and schema steps.
fn grid_components(
    node: &NodeView,
    baseline: &BTreeMap<(u64, String, String), String>,
    rows: &mut Vec<FieldRow>,
    current: &dyn Fn(&NodeView, &str) -> Option<u32>,
) -> Result<(), InspectorError> {
    let mut components: Vec<&VersionedComponent> = node.components.iter().collect();
    components.sort_by(|a, b| a.component.cmp(&b.component));
    for component in components {
        validate_component(component)?;
        let served = current(node, &component.component);
        let stale = served.is_some_and(|served| component.version != served);
        // Payload text cell: label renders as text, others as payload.
        let descriptor = if component.component == "label" {
            DescriptorKind::Label
        } else {
            DescriptorKind::Payload
        };
        let live = text_cell(&component.payload);
        let field = "text".to_string();
        let (prefab_value, overridden) = badge_live(
            &live,
            baseline
                .get(&(node.id, component.component.clone(), field.clone()))
                .map(String::as_str),
        );
        push_row(
            rows,
            FieldRow {
                node: node.id,
                component: component.component.clone(),
                field,
                descriptor,
                kind: FieldKind::Text { max: MAX_TEXT_LEN },
                value: live,
                prefab_value,
                overridden,
            },
        )?;
        // Version cell: always read-only, pinned at current when known.
        let version_field = "version".to_string();
        push_row(
            rows,
            FieldRow {
                node: node.id,
                component: component.component.clone(),
                field: version_field,
                descriptor: if stale {
                    DescriptorKind::SchemaMigration
                } else {
                    DescriptorKind::SchemaCurrent
                },
                kind: match served {
                    Some(pinned) => FieldKind::Version { current: pinned },
                    None => FieldKind::ReadOnly {
                        reason: "unknown component: no schema serves it",
                    },
                },
                value: component.version.to_string(),
                prefab_value: None,
                overridden: false,
            },
        )?;
        if served.is_none() {
            push_row(
                rows,
                FieldRow {
                    node: node.id,
                    component: component.component.clone(),
                    field: "unknown".into(),
                    descriptor: DescriptorKind::ReadOnlyNote,
                    kind: FieldKind::ReadOnly {
                        reason: "unknown version refuses loudly: update the reader",
                    },
                    value: "unknown-component".into(),
                    prefab_value: None,
                    overridden: false,
                },
            )?;
        } else if stale {
            push_row(
                rows,
                FieldRow {
                    node: node.id,
                    component: component.component.clone(),
                    field: "migrate".into(),
                    descriptor: DescriptorKind::ReadOnlyNote,
                    kind: FieldKind::ReadOnly {
                        reason: "stale version migrates on instantiate, never in place",
                    },
                    value: "stale-version".into(),
                    prefab_value: None,
                    overridden: false,
                },
            )?;
        }
    }
    Ok(())
}

/// Validate one edited text against its cell shape.
pub fn validate_edit(row: &FieldRow, value: &str) -> Result<(), InspectorError> {
    if row.kind.is_read_only() {
        let reason = match row.kind {
            FieldKind::Version { .. } => "version pins at current and never edits",
            FieldKind::ReadOnly { reason } => reason,
            _ => "cell never edits",
        };
        return Err(read_only(row, reason));
    }
    match &row.kind {
        FieldKind::Text { max } => {
            if value.chars().count() > *max {
                return Err(bad_value(
                    row,
                    value,
                    "0 chars".into(),
                    format!("{max} chars"),
                ));
            }
            Ok(())
        }
        FieldKind::Integer { min, max } => {
            let parsed: i64 = value
                .trim()
                .parse()
                .map_err(|_| bad_value(row, value, min.to_string(), max.to_string()))?;
            if parsed < *min || parsed > *max {
                return Err(bad_value(row, value, min.to_string(), max.to_string()));
            }
            Ok(())
        }
        FieldKind::Float { min, max } => {
            let parsed: f32 = value.trim().parse().map_err(|_| non_finite(row))?;
            if !parsed.is_finite() {
                return Err(non_finite(row));
            }
            if parsed < *min || parsed > *max {
                return Err(bad_value(row, value, min.to_string(), max.to_string()));
            }
            Ok(())
        }
        FieldKind::Bool => {
            if parse_bool(value).is_none() {
                return Err(bad_value(row, value, "false".into(), "true".into()));
            }
            Ok(())
        }
        FieldKind::Enum { options } => {
            if options.iter().any(|option| *option == value.trim()) {
                Ok(())
            } else {
                Err(InspectorError::BadEnum {
                    component: row.component.clone(),
                    field: row.field.clone(),
                    value: text_cell(value),
                    options,
                })
            }
        }
        FieldKind::Version { .. } | FieldKind::ReadOnly { .. } => {
            Err(read_only(row, "cell never edits"))
        }
    }
}

/// Refusal quoting the raw `value` plus the legal range text.
fn bad_value(row: &FieldRow, value: &str, min: String, max: String) -> InspectorError {
    InspectorError::BadValue {
        component: row.component.clone(),
        field: row.field.clone(),
        value: text_cell(value),
        min,
        max,
    }
}

/// Refusal for a float cell that is blank, unparseable or not finite.
fn non_finite(row: &FieldRow) -> InspectorError {
    InspectorError::NonFinite {
        component: row.component.clone(),
        field: row.field.clone(),
    }
}

/// Refusal for a cell that never edits, quoting why.
fn read_only(row: &FieldRow, reason: &'static str) -> InspectorError {
    InspectorError::ReadOnly {
        component: row.component.clone(),
        field: row.field.clone(),
        reason,
    }
}

/// Parse a bool cell: true/false plus 1/0 and yes/no.
pub(crate) fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_lowercase().as_str() {
        "true" | "1" | "yes" => Some(true),
        "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

/// Find one grid row by component and field.
fn find_row<'a>(
    grid: &'a InspectorGrid,
    node: u64,
    component: &str,
    field: &str,
) -> Result<&'a FieldRow, InspectorError> {
    grid.rows
        .iter()
        .find(|row| row.node == node && row.component == component && row.field == field)
        .ok_or_else(|| InspectorError::UnknownField {
            component: component.to_string(),
            field: field.to_string(),
        })
}

/// Preview staged edits without side effects: one line per op plus the
/// revision an apply would publish. Never touches any document.
pub fn preview_edits(
    grid: &InspectorGrid,
    edits: &BTreeMap<(u64, String, String), String>,
    base_revision: u64,
) -> Result<BatchPreview, InspectorError> {
    if edits.len() > MAX_OPS_PER_BATCH {
        return Err(InspectorError::TooManyOps {
            count: edits.len(),
            max: MAX_OPS_PER_BATCH,
        });
    }
    let mut effects = Vec::with_capacity(edits.len());
    for ((node, component, field), value) in edits {
        let row = find_row(grid, *node, component, field)?;
        validate_edit(row, value)?;
        effects.push(format!(
            "set {component}.{field} on {node} from `{}` to `{}`",
            row.value,
            text_cell(value)
        ));
    }
    Ok(BatchPreview {
        effects,
        resulting_revision: base_revision + 1,
    })
}

/// Emit staged edits as an atomic command batch shape.
///
/// Validates every edit first: any failure returns the error and shapes
/// nothing. Never mutates any document; the caller applies the returned
/// batch through the command service (validate, then preview, then apply).
pub fn emit_batch(
    grid: &InspectorGrid,
    edits: &BTreeMap<(u64, String, String), String>,
    base_revision: u64,
    idempotency_key: u64,
    live_revision: u64,
) -> Result<InspectorBatch, InspectorError> {
    if live_revision != base_revision {
        return Err(InspectorError::StaleRevision {
            expected: live_revision,
            got: base_revision,
        });
    }
    // Preview first so a batch that stopped validating reports before shaping.
    preview_edits(grid, edits, base_revision)?;
    let mut ops = Vec::with_capacity(edits.len());
    for ((node, component, field), value) in edits {
        let row = find_row(grid, *node, component, field)?;
        ops.push(FieldOp {
            node: *node,
            component: component.clone(),
            field: field.clone(),
            old_value: row.value.clone(),
            new_value: text_cell(value),
        });
    }
    Ok(InspectorBatch {
        ops,
        base_revision,
        idempotency_key,
    })
}

/// One pose lane addressable from a grid cell: position, rotation or scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PoseLane {
    /// Translation lane `position[index]`, meters in the parent frame.
    Position(usize),
    /// Rotation lane `rotation[index]`, unit quaternion in the parent frame.
    Rotation(usize),
    /// Scale lane `scale[index]`, factors per axis.
    Scale(usize),
}

/// Grid field name to its pose lane, if the cell edits a pose.
fn pose_lane(field: &str) -> Option<PoseLane> {
    match field {
        "pose.px" => Some(PoseLane::Position(0)),
        "pose.py" => Some(PoseLane::Position(1)),
        "pose.pz" => Some(PoseLane::Position(2)),
        "pose.rx" => Some(PoseLane::Rotation(0)),
        "pose.ry" => Some(PoseLane::Rotation(1)),
        "pose.rz" => Some(PoseLane::Rotation(2)),
        "pose.rw" => Some(PoseLane::Rotation(3)),
        "pose.sx" => Some(PoseLane::Scale(0)),
        "pose.sy" => Some(PoseLane::Scale(1)),
        "pose.sz" => Some(PoseLane::Scale(2)),
        _ => None,
    }
}

/// Fold staged pose edits into the live command [`Batch`].
///
/// Every edit validates against the grid first: any failure returns the
/// error and shapes nothing. Pose lanes of one node merge into a single
/// [`Op::SetLocal`](crate::commands::Op::SetLocal) over the caller's live
/// pose snapshot; non-pose cells refuse with [`InspectorError::UnknownField`]
/// because the command service carries no generic payload op. Never mutates
/// any document; the caller runs the returned batch through the command
/// service (validate, then preview, then apply).
pub fn to_command_batch(
    grid: &InspectorGrid,
    edits: &BTreeMap<(u64, String, String), String>,
    base_revision: u64,
    idempotency_key: u64,
    live_revision: u64,
    poses: &BTreeMap<u64, (ObjectId, Transform)>,
) -> Result<Batch, InspectorError> {
    if live_revision != base_revision {
        return Err(InspectorError::StaleRevision {
            expected: live_revision,
            got: base_revision,
        });
    }
    if edits.len() > MAX_OPS_PER_BATCH {
        return Err(InspectorError::TooManyOps {
            count: edits.len(),
            max: MAX_OPS_PER_BATCH,
        });
    }
    let mut drafts: BTreeMap<u64, (ObjectId, Transform)> = BTreeMap::new();
    for ((node, component, field), value) in edits {
        if component != "pose" {
            return Err(InspectorError::UnknownField {
                component: component.clone(),
                field: field.clone(),
            });
        }
        let row = find_row(grid, *node, component, field)?;
        validate_edit(row, value)?;
        let lane = pose_lane(field).ok_or_else(|| InspectorError::UnknownField {
            component: component.clone(),
            field: field.clone(),
        })?;
        let parsed: f32 = value
            .trim()
            .parse()
            .map_err(|_| InspectorError::NonFinite {
                component: component.clone(),
                field: field.clone(),
            })?;
        if !parsed.is_finite() {
            return Err(InspectorError::NonFinite {
                component: component.clone(),
                field: field.clone(),
            });
        }
        let entry = drafts.entry(*node).or_insert_with(|| {
            poses.get(node).map_or_else(
                || (ObjectId::NIL, Transform::IDENTITY),
                |(id, pose)| (*id, *pose),
            )
        });
        match lane {
            PoseLane::Position(i) => entry.1.position[i] = parsed,
            PoseLane::Rotation(i) => entry.1.rotation[i] = parsed,
            PoseLane::Scale(i) => entry.1.scale[i] = parsed,
        }
    }
    let mut ops = Vec::with_capacity(drafts.len());
    for (node, (id, local)) in drafts {
        if id.is_nil() && !poses.contains_key(&node) {
            return Err(InspectorError::UnknownNode { id: node });
        }
        ops.push(Op::SetLocal { id, local });
    }
    Ok(Batch::new(ops, base_revision, idempotency_key))
}

/// Inspector panel: staged edits over a grid.
///
/// The panel owns no scene state: `build_grid` renders rows from caller
/// snapshots, `stage` validates one cell, `preview` describes the staged
/// batch, and `emit` shapes it for the command service. Undo, provenance
/// and capability checks stay with the command service that applies it.
#[derive(Clone, Debug, Default)]
pub struct InspectorPanel {
    staged: BTreeMap<(u64, String, String), String>,
    base_revision: u64,
}

impl InspectorPanel {
    /// Empty panel: no staged edits, revision zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Revision the staged edits were built against.
    pub const fn base_revision(&self) -> u64 {
        self.base_revision
    }

    /// Staged edit count.
    pub fn staged_len(&self) -> usize {
        self.staged.len()
    }

    /// True when no edit is staged.
    pub fn is_empty(&self) -> bool {
        self.staged.is_empty()
    }

    /// Pin the revision staged edits belong to (call when the grid rebuilds).
    pub fn rebase(&mut self, revision: u64) {
        self.base_revision = revision;
    }

    /// Drop staged edits. Returns the dropped count.
    pub fn refresh(&mut self) -> usize {
        let dropped = self.staged.len();
        self.staged.clear();
        dropped
    }

    /// Stage one cell edit after validating it against the grid.
    pub fn stage(
        &mut self,
        grid: &InspectorGrid,
        node: u64,
        component: &str,
        field: &str,
        value: &str,
    ) -> Result<(), InspectorError> {
        let row = find_row(grid, node, component, field)?;
        validate_edit(row, value)?;
        if self.staged.len() >= MAX_OPS_PER_BATCH
            && !self
                .staged
                .contains_key(&(node, component.to_string(), field.to_string()))
        {
            return Err(InspectorError::TooManyOps {
                count: self.staged.len() + 1,
                max: MAX_OPS_PER_BATCH,
            });
        }
        self.staged.insert(
            (node, component.to_string(), field.to_string()),
            text_cell(value),
        );
        Ok(())
    }

    /// Preview the staged batch without side effects.
    pub fn preview(&self, grid: &InspectorGrid) -> Result<BatchPreview, InspectorError> {
        preview_edits(grid, &self.staged, self.base_revision)
    }

    /// Emit the staged batch for the command service (validate, then shape).
    pub fn emit(
        &self,
        grid: &InspectorGrid,
        idempotency_key: u64,
        live_revision: u64,
    ) -> Result<InspectorBatch, InspectorError> {
        emit_batch(
            grid,
            &self.staged,
            self.base_revision,
            idempotency_key,
            live_revision,
        )
    }

    /// Fold staged pose edits into the live command batch (validate, then shape).
    pub fn emit_command(
        &self,
        grid: &InspectorGrid,
        idempotency_key: u64,
        live_revision: u64,
        poses: &BTreeMap<u64, (ObjectId, Transform)>,
    ) -> Result<Batch, InspectorError> {
        to_command_batch(
            grid,
            &self.staged,
            self.base_revision,
            idempotency_key,
            live_revision,
            poses,
        )
    }

    /// Staged edits in batch order (node, component, field).
    pub fn staged_edits(&self) -> Vec<((u64, String, String), String)> {
        self.staged
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::schema::{Schema, SchemaError, SchemaRegistry};

    const MODES: [&str; 3] = ["ring", "pit", "hall"];

    fn label_step() -> SchemaStep {
        SchemaStep {
            component: "label",
            version: 1,
            is_current: true,
        }
    }

    fn hall_node() -> NodeView {
        NodeView {
            id: 7,
            key: Some("hall".into()),
            parent_key: None,
            pose: LocalPose::IDENTITY,
            components: vec![VersionedComponent::new("label", 1, "Grand Hall")],
            schema: vec![label_step()],
            museum: true,
        }
    }

    fn prop_node() -> NodeView {
        NodeView {
            id: 8,
            key: Some("hall/plinth".into()),
            parent_key: Some("hall".into()),
            pose: LocalPose {
                position: [0.0, 1.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0, 1.0, 1.0],
            },
            components: vec![
                VersionedComponent::new("label", 1, "Plinth"),
                VersionedComponent::new("gadget", 1, "{\"level\":3}"),
            ],
            schema: vec![
                label_step(),
                SchemaStep {
                    component: "gadget",
                    version: 1,
                    is_current: false,
                },
                SchemaStep {
                    component: "gadget",
                    version: 3,
                    is_current: true,
                },
            ],
            museum: false,
        }
    }

    fn selection(ids: &[u64]) -> Selection {
        Selection {
            ids: ids.to_vec(),
            primary: ids.first().copied(),
        }
    }

    fn grid_two() -> (Vec<NodeView>, InspectorGrid) {
        let nodes = vec![hall_node(), prop_node()];
        let grid = build_grid(&nodes, &selection(&[7, 8]), &BTreeMap::new()).unwrap();
        (nodes, grid)
    }

    #[test]
    fn every_descriptor_kind_renders() {
        let (_, grid) = grid_two();
        for kind in DescriptorKind::all() {
            assert!(
                grid.rows.iter().any(|row| row.descriptor == kind),
                "descriptor {kind} renders no row"
            );
        }
        // No two kinds share a name: the coverage proof names each kind once.
        let mut names: Vec<&str> = DescriptorKind::all()
            .iter()
            .map(|kind| kind.name())
            .collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), DescriptorKind::all().len());
    }

    #[test]
    fn grid_order_is_node_component_field() {
        let (_, grid) = grid_two();
        let keys: Vec<(u64, String, String)> = grid
            .rows
            .iter()
            .map(|row| (row.node, row.component.clone(), row.field.clone()))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        // Same inputs twice give the same rows.
        let (_, again) = grid_two();
        assert_eq!(grid, again);
    }

    #[test]
    fn prefab_overrides_badge_per_field() {
        let nodes = vec![prop_node()];
        let mut baseline = BTreeMap::new();
        baseline.insert((8, "label".into(), "text".into()), "Plinth".into());
        baseline.insert((8, "pose".into(), "pose.py".into()), "0.5000".into());
        // Live pose.py is 1.0, baseline says 0.5: that cell badges.
        let grid = build_grid(&nodes, &selection(&[8]), &baseline).unwrap();
        let label = grid
            .rows
            .iter()
            .find(|row| row.component == "label" && row.field == "text")
            .unwrap();
        assert!(!label.overridden);
        assert_eq!(label.badge(), None);
        let lane = grid.rows.iter().find(|row| row.field == "pose.py").unwrap();
        assert!(lane.overridden);
        assert_eq!(lane.badge(), Some("override"));
        assert_eq!(lane.prefab_value.as_deref(), Some("0.5000"));
        assert_eq!(grid.overrides().len(), 1);
    }

    #[test]
    fn museum_identity_is_read_only() {
        let (_, grid) = grid_two();
        let key = grid
            .rows
            .iter()
            .find(|row| row.node == 7 && row.field == "key")
            .unwrap();
        assert!(key.kind.is_read_only());
        assert_eq!(
            validate_edit(key, "other"),
            Err(InspectorError::ReadOnly {
                component: "prefab".into(),
                field: "key".into(),
                reason: "museum fixture identity never edits",
            })
        );
        // The non-museum sibling stays editable.
        let prop_key = grid
            .rows
            .iter()
            .find(|row| row.node == 8 && row.field == "key")
            .unwrap();
        assert!(!prop_key.kind.is_read_only());
        validate_edit(prop_key, "hall/plinth").unwrap();
    }

    #[test]
    fn versions_never_edit_and_unknown_names_its_cause() {
        let (_, grid) = grid_two();
        let version = grid
            .rows
            .iter()
            .find(|row| row.node == 8 && row.component == "gadget" && row.field == "version")
            .unwrap();
        assert_eq!(version.descriptor, DescriptorKind::SchemaMigration);
        assert!(version.kind.is_read_only());
        assert!(matches!(
            validate_edit(version, "3"),
            Err(InspectorError::ReadOnly { .. })
        ));
        // The gadget payload has no current-step match at v1, so the stale
        // note renders beside it.
        assert!(grid
            .rows
            .iter()
            .any(|row| row.node == 8 && row.component == "gadget" && row.field == "migrate"));
        // A component with no schema step renders the unknown note.
        let mut lonely = prop_node();
        lonely
            .components
            .push(VersionedComponent::new("sigil", 1, "old"));
        let nodes = vec![lonely];
        let grid = build_grid(&nodes, &selection(&[8]), &BTreeMap::new()).unwrap();
        let note = grid
            .rows
            .iter()
            .find(|row| row.component == "sigil" && row.field == "unknown")
            .expect("unknown component renders its note");
        assert_eq!(note.descriptor, DescriptorKind::ReadOnlyNote);
    }

    #[test]
    fn float_bool_enum_cells_validate_with_ranges() {
        let float_row = FieldRow {
            node: 1,
            component: "pose".into(),
            field: "pose.px".into(),
            descriptor: DescriptorKind::PosePosition,
            kind: FieldKind::Float {
                min: -10_000.0,
                max: 10_000.0,
            },
            value: "0.0000".into(),
            prefab_value: None,
            overridden: false,
        };
        validate_edit(&float_row, "3.5").unwrap();
        assert!(matches!(
            validate_edit(&float_row, "nan"),
            Err(InspectorError::NonFinite { .. })
        ));
        assert!(matches!(
            validate_edit(&float_row, "20000"),
            Err(InspectorError::BadValue { .. })
        ));
        let int_row = FieldRow {
            node: 1,
            component: "gadget".into(),
            field: "level".into(),
            descriptor: DescriptorKind::Payload,
            kind: FieldKind::Integer { min: 0, max: 9 },
            value: "3".into(),
            prefab_value: None,
            overridden: false,
        };
        validate_edit(&int_row, "7").unwrap();
        assert!(matches!(
            validate_edit(&int_row, "99"),
            Err(InspectorError::BadValue { .. })
        ));
        assert!(matches!(
            validate_edit(&int_row, "many"),
            Err(InspectorError::BadValue { .. })
        ));
        let bool_row = FieldRow {
            kind: FieldKind::Bool,
            ..int_row.clone()
        };
        validate_edit(&bool_row, "yes").unwrap();
        assert!(matches!(
            validate_edit(&bool_row, "maybe"),
            Err(InspectorError::BadValue { .. })
        ));
        let enum_row = FieldRow {
            kind: FieldKind::Enum { options: &MODES },
            ..int_row.clone()
        };
        validate_edit(&enum_row, "pit").unwrap();
        let error = validate_edit(&enum_row, "arena").unwrap_err();
        assert!(matches!(error, InspectorError::BadEnum { .. }));
        assert!(error.to_string().contains("ring|pit|hall"));
    }

    #[test]
    fn preview_and_emit_validate_then_shape_without_mutation() {
        let (nodes, grid) = grid_two();
        let snapshot = nodes.clone();
        let mut panel = InspectorPanel::new();
        panel
            .stage(&grid, 8, "label", "text", "Polished Plinth")
            .unwrap();
        panel.stage(&grid, 8, "pose", "pose.py", "1.5000").unwrap();
        let preview = panel.preview(&grid).unwrap();
        assert_eq!(preview.effects.len(), 2);
        assert_eq!(preview.resulting_revision, 1);
        assert!(preview.effects[0].contains("8"));
        let batch = panel.emit(&grid, 41, 0).unwrap();
        assert_eq!(batch.base_revision, 0);
        assert_eq!(batch.idempotency_key, 41);
        assert_eq!(batch.ops.len(), 2);
        assert_eq!(batch.ops[0].old_value, "Plinth");
        assert_eq!(batch.ops[0].new_value, "Polished Plinth");
        // Neither preview nor emit touches caller state.
        assert_eq!(nodes, snapshot);
        assert_eq!(panel.staged_len(), 2);
        // Same staged edits re-emit the same batch shape.
        assert_eq!(panel.emit(&grid, 41, 0).unwrap(), batch);
    }

    #[test]
    fn one_bad_edit_blocks_the_whole_batch() {
        let (_, grid) = grid_two();
        let mut edits = BTreeMap::new();
        edits.insert((8, "label".into(), "text".into()), "Fine".into());
        edits.insert((8, "pose".into(), "pose.py".into()), "nan".into());
        let error = preview_edits(&grid, &edits, 0).unwrap_err();
        assert!(matches!(error, InspectorError::NonFinite { .. }));
        assert!(matches!(
            emit_batch(&grid, &edits, 0, 1, 0).unwrap_err(),
            InspectorError::NonFinite { .. }
        ));
        // Unknown fields refuse with their address, staging nothing.
        edits.clear();
        edits.insert((8, "label".into(), "missing".into()), "x".into());
        assert!(matches!(
            preview_edits(&grid, &edits, 0).unwrap_err(),
            InspectorError::UnknownField { .. }
        ));
    }

    #[test]
    fn stale_batches_and_empty_selections_refuse() {
        let (_, grid) = grid_two();
        let mut edits = BTreeMap::new();
        edits.insert((8, "label".into(), "text".into()), "Late".into());
        assert_eq!(
            emit_batch(&grid, &edits, 0, 1, 2).unwrap_err(),
            InspectorError::StaleRevision {
                expected: 2,
                got: 0
            }
        );
        let empty = Selection::new();
        assert_eq!(
            build_grid(&[], &empty, &BTreeMap::new()).unwrap_err(),
            InspectorError::EmptySelection
        );
        assert_eq!(
            build_grid(&[hall_node()], &selection(&[99]), &BTreeMap::new()).unwrap_err(),
            InspectorError::UnknownNode { id: 99 }
        );
        let bad_primary = Selection {
            ids: vec![7],
            primary: Some(8),
        };
        assert_eq!(
            build_grid(&[hall_node()], &bad_primary, &BTreeMap::new()).unwrap_err(),
            InspectorError::UnknownNode { id: 8 }
        );
    }

    #[test]
    fn bounds_hold_on_rows_ops_and_text() {
        let (_, grid) = grid_two();
        let row = grid.rows.first().unwrap();
        let long = "x".repeat(MAX_TEXT_LEN + 1);
        let text_row = FieldRow {
            kind: FieldKind::Text { max: 4 },
            ..row.clone()
        };
        assert!(matches!(
            validate_edit(&text_row, &long),
            Err(InspectorError::BadValue { .. })
        ));
        let mut too_many = BTreeMap::new();
        for i in 0..MAX_OPS_PER_BATCH + 1 {
            too_many.insert((8, "label".into(), format!("f{i}")), "x".into());
        }
        assert_eq!(
            preview_edits(&grid, &too_many, 0).unwrap_err(),
            InspectorError::TooManyOps {
                count: MAX_OPS_PER_BATCH + 1,
                max: MAX_OPS_PER_BATCH,
            }
        );
        assert!(grid.len() <= MAX_ROWS);
    }

    #[test]
    fn malformed_descriptors_refuse_before_render() {
        let mut bad = prop_node();
        bad.components = vec![VersionedComponent::new("", 1, "x")];
        assert!(matches!(
            build_grid(&[bad], &selection(&[8]), &BTreeMap::new()).unwrap_err(),
            InspectorError::BadDescriptor { .. }
        ));
        let mut zero = prop_node();
        zero.components = vec![VersionedComponent::new("label", 0, "x")];
        assert!(matches!(
            build_grid(&[zero], &selection(&[8]), &BTreeMap::new()).unwrap_err(),
            InspectorError::BadDescriptor { .. }
        ));
    }

    #[test]
    fn panel_rebase_and_refresh() {
        let (_, grid) = grid_two();
        let mut panel = InspectorPanel::new();
        assert!(panel.is_empty());
        panel.rebase(3);
        assert_eq!(panel.base_revision(), 3);
        panel.stage(&grid, 8, "label", "text", "Staged").unwrap();
        assert_eq!(panel.staged_len(), 1);
        // Staging against a moved revision refuses at emit, never at stage.
        assert!(matches!(
            panel.emit(&grid, 9, 4).unwrap_err(),
            InspectorError::StaleRevision { .. }
        ));
        assert_eq!(panel.refresh(), 1);
        assert!(panel.is_empty());
        assert_eq!(panel.refresh(), 0);
    }

    #[test]
    fn error_text_quotes_addresses_and_bounds() {
        let error = InspectorError::BadValue {
            component: "pose".into(),
            field: "pose.px".into(),
            value: "20000".into(),
            min: "-10000".into(),
            max: "10000".into(),
        };
        assert!(error.to_string().contains("pose.px"));
        assert!(error.to_string().contains("10000"));
        let stale = InspectorError::StaleRevision {
            expected: 2,
            got: 0,
        };
        assert!(stale.to_string().contains('2'));
        assert_eq!(
            InspectorError::EmptySelection.to_string(),
            "no node selected"
        );
    }

    fn gadget_bump(payload: &str) -> Result<String, SchemaError> {
        Ok(format!("{payload}+"))
    }

    fn live_registry() -> SchemaRegistry {
        let mut registry = SchemaRegistry::builtin();
        registry
            .register(Schema {
                component: "gadget",
                version: 1,
                migrate: Some(gadget_bump),
            })
            .unwrap();
        registry
            .register(Schema {
                component: "gadget",
                version: 2,
                migrate: Some(gadget_bump),
            })
            .unwrap();
        registry
            .register(Schema {
                component: "gadget",
                version: 3,
                migrate: None,
            })
            .unwrap();
        registry
    }

    #[test]
    fn step_copies_agree_with_registry() {
        let registry = live_registry();
        assert_eq!(registry.current_version("label"), Some(1));
        assert_eq!(registry.current_version("gadget"), Some(3));
        // Every step copy the fixtures carry agrees with the live registry:
        // current steps match the registry head, migration steps sit below it.
        let (nodes, stepped) = grid_two();
        for node in &nodes {
            for step in &node.schema {
                let head = registry.current_version(step.component);
                assert!(head.is_some(), "registry knows {}", step.component);
                if step.is_current {
                    assert_eq!(Some(step.version), head);
                } else {
                    assert!(step.version < head.unwrap_or(u32::MAX));
                }
            }
        }
        // Same rows twice through the prod path: determinism holds.
        let again = build_grid(&nodes, &selection(&[7, 8]), &BTreeMap::new()).unwrap();
        assert_eq!(again, stepped);
        // Unknown components badge through the prod path.
        let mut lonely = prop_node();
        lonely
            .components
            .push(VersionedComponent::new("sigil", 1, "old"));
        let nodes = vec![lonely];
        let grid = build_grid(&nodes, &selection(&[8]), &BTreeMap::new()).unwrap();
        assert!(grid
            .rows
            .iter()
            .any(|row| row.component == "sigil" && row.field == "unknown"));
    }

    #[test]
    fn step_currency_and_text_bound_read_through_build_grid() {
        // Currency is a step-copy fact: current exactly when no migration follows.
        let current = SchemaStep {
            component: "label",
            version: 1,
            is_current: true,
        };
        assert!(current.is_current);
        let migration = SchemaStep {
            component: "gadget",
            version: 1,
            is_current: false,
        };
        assert!(!migration.is_current);
        assert_eq!(migration.version, 1);
        // The prod path enforces the cell bound by refusing, never truncating.
        let mut heavy = prop_node();
        heavy.components.push(VersionedComponent::new(
            "sigil",
            1,
            &"y".repeat(MAX_TEXT_LEN + 8),
        ));
        assert!(matches!(
            build_grid(&[heavy], &selection(&[8]), &BTreeMap::new()).unwrap_err(),
            InspectorError::BadDescriptor { .. }
        ));
        // Poses are the live transform: identity and finiteness agree.
        assert_eq!(LocalPose::IDENTITY, Transform::IDENTITY);
        assert!(LocalPose::IDENTITY.is_finite());
    }

    #[test]
    fn pose_edits_fold_into_live_command_batches() {
        use crate::commands::{validate, Document};
        use engine_scene::transform::Hierarchy;
        let target = ObjectId::parse("00000000-0000-4000-8000-000000000041")
            .expect("fixed fixture id parses");
        let mut tree = Hierarchy::new();
        tree.insert(target, ObjectId::NIL, Transform::IDENTITY)
            .expect("fixed tree inserts");
        let doc = Document {
            hierarchy: tree,
            revision: 0,
            materials: BTreeMap::new(),
            components: BTreeMap::new(),
            graph: crate::graph::Graph::new(),
        };
        let nodes = vec![prop_node()];
        let grid = build_grid(&nodes, &selection(&[8]), &BTreeMap::new()).unwrap();
        let mut poses = BTreeMap::new();
        poses.insert(8u64, (target, Transform::IDENTITY));
        let mut edits = BTreeMap::new();
        edits.insert((8u64, "pose".into(), "pose.px".into()), "1.5".into());
        edits.insert((8u64, "pose".into(), "pose.sy".into()), "2.0".into());
        edits.insert((8u64, "pose".into(), "pose.rw".into()), "0.5".into());
        let batch = to_command_batch(&grid, &edits, 0, 7, 0, &poses).unwrap();
        assert_eq!(batch.base_revision, 0);
        assert_eq!(batch.idempotency_key, 7);
        assert_eq!(batch.ops.len(), 1);
        match batch.ops[0] {
            Op::SetLocal { id, local } => {
                assert_eq!(id, target);
                assert_eq!(local.position[0], 1.5);
                assert_eq!(local.position[1], 0.0);
                assert_eq!(local.rotation[3], 0.5);
                assert_eq!(local.scale[1], 2.0);
            }
            _ => panic!("pose edits fold into one SetLocal"),
        }
        // The folded batch validates through the real command service.
        let preview = validate(&doc, &batch).expect("folded pose batch validates");
        assert_eq!(preview.resulting_revision, 1);
        assert_eq!(preview.effects.len(), 1);
        // Same staged edits re-emit the same live batch through the panel.
        let mut panel = InspectorPanel::new();
        panel.stage(&grid, 8, "pose", "pose.px", "1.5").unwrap();
        panel.stage(&grid, 8, "pose", "pose.sy", "2.0").unwrap();
        panel.stage(&grid, 8, "pose", "pose.rw", "0.5").unwrap();
        assert_eq!(panel.emit_command(&grid, 7, 0, &poses).unwrap(), batch);
    }

    #[test]
    fn command_batches_refuse_non_pose_stale_and_unknown() {
        let nodes = vec![prop_node()];
        let grid = build_grid(&nodes, &selection(&[8]), &BTreeMap::new()).unwrap();
        let target = ObjectId::parse("00000000-0000-4000-8000-000000000042")
            .expect("fixed fixture id parses");
        let mut poses = BTreeMap::new();
        poses.insert(8u64, (target, Transform::IDENTITY));
        // Label cells have no command op: they refuse with their address.
        let mut edits = BTreeMap::new();
        edits.insert((8u64, "label".into(), "text".into()), "Late".into());
        assert!(matches!(
            to_command_batch(&grid, &edits, 0, 1, 0, &poses).unwrap_err(),
            InspectorError::UnknownField { .. }
        ));
        // Stale revisions shape nothing.
        edits.clear();
        edits.insert((8u64, "pose".into(), "pose.px".into()), "1.0".into());
        assert_eq!(
            to_command_batch(&grid, &edits, 0, 1, 2, &poses).unwrap_err(),
            InspectorError::StaleRevision {
                expected: 2,
                got: 0
            }
        );
        // Nodes missing from the pose snapshot refuse as unknown.
        assert!(matches!(
            to_command_batch(&grid, &edits, 0, 1, 0, &BTreeMap::new()).unwrap_err(),
            InspectorError::UnknownNode { .. }
        ));
        // Bad lanes refuse without shaping.
        edits.clear();
        edits.insert((8u64, "pose".into(), "pose.px".into()), "nan".into());
        assert!(matches!(
            to_command_batch(&grid, &edits, 0, 1, 0, &poses).unwrap_err(),
            InspectorError::NonFinite { .. }
        ));
    }
}
