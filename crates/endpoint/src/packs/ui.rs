//! `ui` pack: the integer layout solver and one pinned drag.
//!
//! Not loaded by default: a session pays for these verbs only when it asks.
//! The data owner is `engine-ui` (`layout`): this pack reads the authored
//! document as JSON, calls the owner and answers numbers; it copies no solver.
//!
//! - `ui.layout.solve` (observe): document plus viewport in, one pixel rect
//!   per panel, the inner box, the leftover and the digest. An optional
//!   palette id adds the contrast rows for body and accent text.
//! - `ui.layout.resize` (commit): one drag (`pin`, `resize` or `unpin`) on a
//!   panel, then the moved panel ids versus the document as sent. No session
//!   state: the client keeps the document and sends its `pins` back.
//!
//! Units: whole pixels and whole ticks, never a float. Limits: 32 panels and
//! 32 rows per document (the crate allows more; the door keeps answers small).
//! Determinism: the same document and viewport give the same rects and digest.
//! Errors: a refusal names the field, the legal band and one recovery; an
//! overflow is the typed `overflow` code carrying the axis, the shortfall in
//! pixels and the first panel of the caller's shrink order.

use engine_ui::layout::{
    Axis, Constraint, LayoutDoc, LayoutError, Margins, PanelId, PanelPin, PanelSpec, RowSpec,
    SolvedLayout, Viewport, Whitespace, MAX_ROWS,
};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::{json, Map, Value};

use super::{bad_args, PackDef, PackError, VerbSpec};
use crate::tool_capabilities::CapabilityCard;

/// Pack name.
pub const PACK: &str = "ui";

/// Verb: solve one layout document against a viewport.
pub const TOOL_LAYOUT_SOLVE: &str = "ui.layout.solve";

/// Verb: apply one pinned drag and report the moved panels.
pub const TOOL_LAYOUT_RESIZE: &str = "ui.layout.resize";

/// Most panels one document may carry through this pack.
pub const MAX_PACK_PANELS: usize = 32;

/// Largest fixed or min/max track size the reader accepts, in pixels.
pub const MAX_TRACK_PX: u32 = 1_000_000;

/// Largest aspect part the reader accepts.
pub const MAX_ASPECT_PART: u32 = 1_000;

/// Largest gap or margin the reader accepts, in pixels.
pub const MAX_PACK_GAP_PX: u32 = 512;

/// The ui pack: two verbs, not loaded by default.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "ui: solve an integer layout document into pixel rects (with an optional \
         palette contrast report) and apply one pinned drag, in memory",
        false,
    );
    pack.add(VerbSpec::of::<LayoutSolveTool>(
        TOOL_LAYOUT_SOLVE,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<LayoutResizeTool>(
        TOOL_LAYOUT_RESIZE,
        PACK,
        CapabilityCard::Commit,
    )?);
    Ok(pack)
}

/// Args for `ui.layout.solve`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LayoutSolveArgs {
    /// Layout document: margins, row gap, whitespace, rows and optional pins.
    pub document: Value,
    /// Viewport width in pixels.
    pub width_px: u32,
    /// Viewport height in pixels.
    pub height_px: u32,
    /// Panel ids in the caller's shrink order, first to shrink first.
    #[serde(default)]
    pub shrink_order: Option<Vec<u16>>,
    /// Palette id `0..=3` for the contrast rows; omitted means no report.
    #[serde(default)]
    pub palette: Option<u8>,
}

/// One solved rectangle.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RectOut {
    /// Left edge, pixels.
    pub x: u32,
    /// Top edge, pixels.
    pub y: u32,
    /// Width, pixels.
    pub w: u32,
    /// Height, pixels.
    pub h: u32,
}

/// One panel and its solved rectangle.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PanelRectOut {
    /// Panel id, echoed from the document.
    pub id: u16,
    /// Solved rectangle, pixels.
    pub rect: RectOut,
}

/// One active pin, in the shape the document reader takes back.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PinOut {
    /// Panel the override names.
    pub id: u16,
    /// Width rule (`fixed`, `minmax`, `fraction` or `aspect`), same shape as
    /// the document reader's `pins`.
    pub width: Value,
}

/// One text role's contrast row.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RoleOut {
    /// Measured contrast in permille (`2100` is 2.1:1).
    pub ratio_permille: u32,
    /// Floor the role must reach, in permille.
    pub required_permille: u32,
    /// True when the measured contrast reaches the floor.
    pub passes: bool,
}

/// Palette contrast rows measured against the document background.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct PaletteOut {
    /// Palette name, from the HUD palette set.
    pub palette: String,
    /// Body text row.
    pub body: RoleOut,
    /// Accent text row.
    pub accent: RoleOut,
    /// True when both roles pass.
    pub passes: bool,
}

/// `ui.layout.solve` result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LayoutSolveOutput {
    /// Panel rects in authored order (rows top to bottom, panels left to
    /// right).
    pub panels: Vec<PanelRectOut>,
    /// Inner box inside the margins, pixels.
    pub inner: RectOut,
    /// Unused width at the right edge, pixels.
    pub leftover_w: u32,
    /// Unused height at the bottom edge, pixels.
    pub leftover_h: u32,
    /// FNV-1a64 over the rects and the leftover, 16 lowercase hex chars.
    pub digest: String,
    /// Active pins, in pin order; send them back to keep a drag.
    pub pins: Vec<PinOut>,
    /// Contrast rows when a palette id was given.
    pub palette: Option<PaletteOut>,
}

/// Args for `ui.layout.resize`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LayoutResizeArgs {
    /// Layout document, as `ui.layout.solve` takes it.
    pub document: Value,
    /// Viewport width in pixels.
    pub width_px: u32,
    /// Viewport height in pixels.
    pub height_px: u32,
    /// Panel the drag names.
    pub panel: u16,
    /// Edit: `pin`, `resize` or `unpin`.
    pub edit: String,
    /// Drag target width in pixels; required for `resize`.
    #[serde(default)]
    pub px: Option<u32>,
    /// Width rule for `pin`, same shape as a panel's `width`.
    #[serde(default)]
    pub width: Option<Value>,
    /// Drag axis: `horizontal` (default) or `vertical` (refused).
    #[serde(default)]
    pub axis: Option<String>,
    /// Panel ids in the caller's shrink order.
    #[serde(default)]
    pub shrink_order: Option<Vec<u16>>,
}

/// `ui.layout.resize` result: the receipt plus the new solve.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LayoutResizeOutput {
    /// Panel ids whose rect moved versus the document as sent, authored order.
    pub moved: Vec<u16>,
    /// The solve after the edit.
    pub layout: LayoutSolveOutput,
}

/// `ui.layout.solve` handler: parse the document, solve it, answer numbers.
fn layout_solve(
    service: &crate::mcp::McpServer,
    args: &LayoutSolveArgs,
) -> Result<LayoutSolveOutput, crate::mcp::McpToolError> {
    service.dispatch(TOOL_LAYOUT_SOLVE, |_state| run_solve(args))
}

/// `ui.layout.resize` handler: apply one drag, answer the receipt.
fn layout_resize(
    service: &crate::mcp::McpServer,
    args: &LayoutResizeArgs,
) -> Result<LayoutResizeOutput, crate::mcp::McpToolError> {
    service.dispatch(TOOL_LAYOUT_RESIZE, |_state| run_resize(args))
}

crate::e2_tool!(
    /// `ui.layout.solve` tool: integer layout solve.
    LayoutSolveTool,
    name = TOOL_LAYOUT_SOLVE,
    description = "Solve a layout document (rows, panels with fixed/minmax/fraction/aspect, margins, gaps, whitespace, pins) at width_px by height_px: one pixel rect per panel, the inner box, leftover and digest, plus body/accent contrast when palette is given. Overflow refuses with the axis, shortfall and first shrink panel.",
    readonly = true,
    args = LayoutSolveArgs,
    output = LayoutSolveOutput,
    invoke = layout_solve
);

crate::e2_tool!(
    /// `ui.layout.resize` tool: one pinned drag.
    LayoutResizeTool,
    name = TOOL_LAYOUT_RESIZE,
    description = "Apply one drag to a layout document: edit pin (width rule), resize (px on the horizontal axis) or unpin one panel, then report the moved panel ids versus the document as sent plus the new rects, pins and digest. No session state: send the returned pins back for the next drag.",
    readonly = false,
    args = LayoutResizeArgs,
    output = LayoutResizeOutput,
    invoke = layout_resize
);

/// A refusal that names the field, the legal band and one recovery.
pub(crate) fn field_refusal(field: &str, band: &str, recovery: &str) -> crate::mcp::McpToolError {
    bad_args(format!("{field}: legal {band}; recovery: {recovery}"))
}

/// Refuse an object that carries a key outside `legal`; a typo never passes
/// silently.
fn only_keys(
    map: &Map<String, Value>,
    path: &str,
    legal: &[&str],
) -> Result<(), crate::mcp::McpToolError> {
    for key in map.keys() {
        if !legal.contains(&key.as_str()) {
            return Err(field_refusal(
                &format!("{path}.{key}"),
                &format!("keys {}", legal.join(", ")),
                &format!("rename `{key}` to one of {}", legal.join(", ")),
            ));
        }
    }
    Ok(())
}

/// Read an unsigned integer inside `min..=max`, naming the field and band.
pub(crate) fn read_u32(
    value: &Value,
    field: &str,
    min: u32,
    max: u32,
) -> Result<u32, crate::mcp::McpToolError> {
    let raw = value
        .as_u64()
        .ok_or_else(|| field_refusal(field, "a non-negative whole number", "write digits only"))?;
    let number = u32::try_from(raw)
        .map_err(|_| field_refusal(field, &format!("{min}..={max}"), "lower the number"))?;
    if number < min || number > max {
        return Err(field_refusal(
            field,
            &format!("{min}..={max}"),
            &format!("use a value in {min}..={max}"),
        ));
    }
    Ok(number)
}

/// Read a signed integer inside `min..=max`, naming the field and band.
pub(crate) fn read_i32(
    value: &Value,
    field: &str,
    min: i64,
    max: i64,
) -> Result<i32, crate::mcp::McpToolError> {
    let raw = value
        .as_i64()
        .ok_or_else(|| field_refusal(field, "a whole number", "write digits only"))?;
    if raw < min || raw > max {
        return Err(field_refusal(
            field,
            &format!("{min}..={max}"),
            &format!("use a value in {min}..={max}"),
        ));
    }
    i32::try_from(raw)
        .map_err(|_| field_refusal(field, &format!("{min}..={max}"), "lower the number"))
}

/// Read a two-number array, each inside `min..=max`.
fn read_pair(
    value: &Value,
    field: &str,
    min: u32,
    max: u32,
) -> Result<[u32; 2], crate::mcp::McpToolError> {
    let parts = value
        .as_array()
        .filter(|parts| parts.len() == 2)
        .ok_or_else(|| {
            field_refusal(
                field,
                &format!("two numbers in {min}..={max}"),
                "write [first, second]",
            )
        })?;
    Ok([
        read_u32(&parts[0], &format!("{field}[0]"), min, max)?,
        read_u32(&parts[1], &format!("{field}[1]"), min, max)?,
    ])
}

/// Read one constraint object: exactly one of `fixed`, `minmax`, `fraction`
/// or `aspect`.
fn read_constraint(value: &Value, field: &str) -> Result<Constraint, crate::mcp::McpToolError> {
    let map = value.as_object().ok_or_else(|| {
        field_refusal(
            field,
            "an object with exactly one of fixed, minmax, fraction, aspect",
            "write {\"fixed\":300} or {\"fraction\":[1,1]}",
        )
    })?;
    if map.len() != 1 {
        return Err(field_refusal(
            field,
            "exactly one key of fixed, minmax, fraction, aspect",
            "drop the extra keys",
        ));
    }
    let (kind, body) = map.iter().next().ok_or_else(|| {
        field_refusal(
            field,
            "exactly one key of fixed, minmax, fraction, aspect",
            "write {\"fixed\":300}",
        )
    })?;
    let name = format!("{field}.{kind}");
    match kind.as_str() {
        "fixed" => Ok(Constraint::Fixed {
            px: read_u32(body, &name, 0, MAX_TRACK_PX)?,
        }),
        "minmax" => {
            let pair = read_pair(body, &name, 0, MAX_TRACK_PX)?;
            Ok(Constraint::MinMax {
                min_px: pair[0],
                max_px: pair[1],
            })
        }
        "fraction" => {
            let pair = read_pair(body, &name, 1, MAX_TRACK_PX)?;
            Ok(Constraint::Fraction {
                num: pair[0],
                den: pair[1],
            })
        }
        "aspect" => {
            let pair = read_pair(body, &name, 1, MAX_ASPECT_PART)?;
            Ok(Constraint::Aspect {
                w: pair[0],
                h: pair[1],
            })
        }
        other => Err(field_refusal(
            &format!("{field}.{other}"),
            "one of fixed, minmax, fraction, aspect",
            "rename the key to a legal rule",
        )),
    }
}

/// Constraint back to the reader's own JSON shape, so pins round-trip.
fn write_constraint(width: Constraint) -> Value {
    match width {
        Constraint::Fixed { px } => json!({ "fixed": px }),
        Constraint::MinMax { min_px, max_px } => json!({ "minmax": [min_px, max_px] }),
        Constraint::Fraction { num, den } => json!({ "fraction": [num, den] }),
        Constraint::Aspect { w, h } => json!({ "aspect": [w, h] }),
    }
}

/// Read the authored layout document.
fn read_document(value: &Value) -> Result<LayoutDoc, crate::mcp::McpToolError> {
    let map = value.as_object().ok_or_else(|| {
        field_refusal(
            "document",
            "an object with rows (required) and optional background, margins, row_gap, whitespace, pins",
            "send {\"rows\":[{\"height\":{\"fraction\":[1,1]},\"panels\":[{\"id\":0,\"width\":{\"fraction\":[1,1]}}]}]}",
        )
    })?;
    only_keys(
        map,
        "document",
        &[
            "background",
            "margins",
            "row_gap",
            "whitespace",
            "rows",
            "pins",
        ],
    )?;
    let mut doc = LayoutDoc::new(read_rows(map.get("rows"))?);
    if let Some(background) = map.get("background") {
        let parts = background
            .as_array()
            .filter(|parts| parts.len() == 3)
            .ok_or_else(|| {
                field_refusal(
                    "document.background",
                    "three numbers 0..=255 (sRGB)",
                    "write [0,0,0]",
                )
            })?;
        doc.background = [
            u8::try_from(read_u32(&parts[0], "document.background[0]", 0, 255)?)
                .map_err(|_| field_refusal("document.background[0]", "0..=255", "lower it"))?,
            u8::try_from(read_u32(&parts[1], "document.background[1]", 0, 255)?)
                .map_err(|_| field_refusal("document.background[1]", "0..=255", "lower it"))?,
            u8::try_from(read_u32(&parts[2], "document.background[2]", 0, 255)?)
                .map_err(|_| field_refusal("document.background[2]", "0..=255", "lower it"))?,
        ];
    }
    if let Some(margins) = map.get("margins") {
        doc.margins = read_margins(margins)?;
    }
    if let Some(gap) = map.get("row_gap") {
        doc.row_gap = read_u32(gap, "document.row_gap", 0, MAX_PACK_GAP_PX)?;
    }
    if let Some(whitespace) = map.get("whitespace") {
        doc.whitespace = read_whitespace(whitespace)?;
    }
    if let Some(pins) = map.get("pins") {
        read_pins(&mut doc, pins)?;
    }
    let panels: usize = doc.rows.iter().map(|row| row.panels.len()).sum();
    if panels > MAX_PACK_PANELS {
        return Err(field_refusal(
            "document.rows[].panels",
            &format!("1..={MAX_PACK_PANELS} panels per document"),
            "split the layout into two documents",
        ));
    }
    Ok(doc)
}

/// Read the row bands.
fn read_rows(rows: Option<&Value>) -> Result<Vec<RowSpec>, crate::mcp::McpToolError> {
    let list = rows.and_then(Value::as_array).ok_or_else(|| {
        field_refusal(
            "document.rows",
            &format!("an array of 1..={MAX_ROWS} rows"),
            "write at least one row band",
        )
    })?;
    if list.is_empty() || list.len() > MAX_ROWS {
        return Err(field_refusal(
            "document.rows",
            &format!("1..={MAX_ROWS} rows"),
            "use 1..=32 rows",
        ));
    }
    let mut out = Vec::with_capacity(list.len());
    for (index, row) in list.iter().enumerate() {
        out.push(read_row(row, index)?);
    }
    Ok(out)
}

/// Read one row band.
fn read_row(row: &Value, index: usize) -> Result<RowSpec, crate::mcp::McpToolError> {
    let path = format!("document.rows[{index}]");
    let map = row.as_object().ok_or_else(|| {
        field_refusal(
            &path,
            "an object with height and panels (gap optional)",
            "write {\"height\":{\"fraction\":[1,1]},\"panels\":[...]}",
        )
    })?;
    only_keys(map, &path, &["height", "gap", "panels"])?;
    let height = read_constraint(
        map.get("height")
            .ok_or_else(|| field_refusal(&format!("{path}.height"), "a size rule", "add height"))?,
        &format!("{path}.height"),
    )?;
    let gap = match map.get("gap") {
        Some(gap) => read_u32(gap, &format!("{path}.gap"), 0, MAX_PACK_GAP_PX)?,
        None => 0,
    };
    let panels = map.get("panels").and_then(Value::as_array).ok_or_else(|| {
        field_refusal(
            &format!("{path}.panels"),
            &format!("an array of 1..={MAX_PACK_PANELS} panels"),
            "add at least one panel",
        )
    })?;
    if panels.is_empty() || panels.len() > MAX_PACK_PANELS {
        return Err(field_refusal(
            &format!("{path}.panels"),
            &format!("1..={MAX_PACK_PANELS} panels"),
            "use 1..=32 panels in the document",
        ));
    }
    let mut specs = Vec::with_capacity(panels.len());
    for (panel_index, panel) in panels.iter().enumerate() {
        specs.push(read_panel(panel, &path, panel_index)?);
    }
    Ok(RowSpec {
        height,
        gap,
        panels: specs,
    })
}

/// Read one panel spec.
fn read_panel(
    panel: &Value,
    row_path: &str,
    index: usize,
) -> Result<PanelSpec, crate::mcp::McpToolError> {
    let path = format!("{row_path}.panels[{index}]");
    let map = panel.as_object().ok_or_else(|| {
        field_refusal(
            &path,
            "an object with id and width",
            "write {\"id\":0,\"width\":{\"fixed\":300}}",
        )
    })?;
    only_keys(map, &path, &["id", "width"])?;
    let id = read_u32(
        map.get("id")
            .ok_or_else(|| field_refusal(&format!("{path}.id"), "0..=65535", "add id"))?,
        &format!("{path}.id"),
        0,
        65_535,
    )?;
    let id = u16::try_from(id)
        .map_err(|_| field_refusal(&format!("{path}.id"), "0..=65535", "lower the id"))?;
    let width = read_constraint(
        map.get("width")
            .ok_or_else(|| field_refusal(&format!("{path}.width"), "a size rule", "add width"))?,
        &format!("{path}.width"),
    )?;
    Ok(PanelSpec {
        id: PanelId(id),
        width,
    })
}

/// Read the page margins.
fn read_margins(value: &Value) -> Result<Margins, crate::mcp::McpToolError> {
    let map = value.as_object().ok_or_else(|| {
        field_refusal(
            "document.margins",
            "an object with left, top, right, bottom",
            "write {\"left\":8,\"top\":8,\"right\":8,\"bottom\":8}",
        )
    })?;
    only_keys(map, "document.margins", &["left", "top", "right", "bottom"])?;
    let read = |key: &str| {
        map.get(key).map_or(Ok(0), |px| {
            read_u32(px, &format!("document.margins.{key}"), 0, MAX_PACK_GAP_PX)
        })
    };
    Ok(Margins {
        left: read("left")?,
        top: read("top")?,
        right: read("right")?,
        bottom: read("bottom")?,
    })
}

/// Read the whitespace taste name.
fn read_whitespace(value: &Value) -> Result<Whitespace, crate::mcp::McpToolError> {
    match value.as_str() {
        Some("compact") => Ok(Whitespace::Compact),
        Some("balanced") => Ok(Whitespace::Balanced),
        Some("spacious") => Ok(Whitespace::Spacious),
        _ => Err(field_refusal(
            "document.whitespace",
            "one of compact, balanced, spacious",
            "pick one of the three names",
        )),
    }
}

/// Read the caller's pins back onto the document.
fn read_pins(doc: &mut LayoutDoc, pins: &Value) -> Result<(), crate::mcp::McpToolError> {
    let list = pins.as_array().ok_or_else(|| {
        field_refusal(
            "document.pins",
            "an array of {id, width}",
            "send the pins `ui.layout.solve` returned",
        )
    })?;
    if list.len() > MAX_PACK_PANELS {
        return Err(field_refusal(
            "document.pins",
            &format!("0..={MAX_PACK_PANELS} pins"),
            "drop pins for panels you no longer drag",
        ));
    }
    for (index, pin) in list.iter().enumerate() {
        let path = format!("document.pins[{index}]");
        let map = pin.as_object().ok_or_else(|| {
            field_refusal(
                &path,
                "an object with id and width",
                "send the pins `ui.layout.solve` returned",
            )
        })?;
        only_keys(map, &path, &["id", "width"])?;
        let id = read_u32(
            map.get("id")
                .ok_or_else(|| field_refusal(&format!("{path}.id"), "0..=65535", "add id"))?,
            &format!("{path}.id"),
            0,
            65_535,
        )?;
        let id = u16::try_from(id)
            .map_err(|_| field_refusal(&format!("{path}.id"), "0..=65535", "lower the id"))?;
        let width = read_constraint(
            map.get("width").ok_or_else(|| {
                field_refusal(&format!("{path}.width"), "a size rule", "add width")
            })?,
            &format!("{path}.width"),
        )?;
        doc.pin(PanelId(id), width).map_err(layout_refusal)?;
    }
    Ok(())
}

/// Map a crate error onto a door refusal; overflow keeps its own code.
fn layout_refusal(error: LayoutError) -> crate::mcp::McpToolError {
    match error {
        LayoutError::Overflow {
            axis,
            shortfall_px,
            shrink_first,
        } => crate::mcp::McpToolError::new(
            "overflow",
            format!(
                "layout {} overflow by {shortfall_px} px (first shrink panel: {}); recovery: shrink that panel, name a shrink order or widen the viewport",
                axis,
                shrink_first.map_or_else(|| "none".to_owned(), |id| id.0.to_string())
            ),
        ),
        other => bad_args(format!("{other}; recovery: fix the named field")),
    }
}

/// Read the caller's shrink order.
fn read_shrink_order(order: Option<&Vec<u16>>) -> Result<Vec<PanelId>, crate::mcp::McpToolError> {
    let mut out = Vec::new();
    for id in order.into_iter().flatten() {
        if out.len() >= MAX_PACK_PANELS {
            return Err(field_refusal(
                "shrink_order",
                &format!("0..={MAX_PACK_PANELS} panel ids"),
                "name only the panels you would shrink",
            ));
        }
        out.push(PanelId(*id));
    }
    Ok(out)
}

/// The solve itself, shared by both verbs.
fn run_solve(args: &LayoutSolveArgs) -> Result<LayoutSolveOutput, crate::mcp::McpToolError> {
    let doc = read_document(&args.document)?;
    let viewport = Viewport::new(args.width_px, args.height_px);
    let shrink = read_shrink_order(args.shrink_order.as_ref())?;
    let solved = doc.solve(viewport, &shrink).map_err(layout_refusal)?;
    let palette = match args.palette {
        Some(id) => Some(palette_out(&doc, id)?),
        None => None,
    };
    Ok(solve_output(&doc, &solved, palette))
}

/// Build the answer from a solve and the document's pins.
fn solve_output(
    doc: &LayoutDoc,
    solved: &SolvedLayout,
    palette: Option<PaletteOut>,
) -> LayoutSolveOutput {
    LayoutSolveOutput {
        panels: solved
            .panels
            .iter()
            .map(|panel| PanelRectOut {
                id: panel.id.0,
                rect: rect_out(panel.rect),
            })
            .collect(),
        inner: rect_out(solved.inner),
        leftover_w: solved.leftover_w,
        leftover_h: solved.leftover_h,
        digest: format!("{:016x}", solved.digest),
        pins: doc
            .pins()
            .iter()
            .map(|pin: &PanelPin| PinOut {
                id: pin.id.0,
                width: write_constraint(pin.width),
            })
            .collect(),
        palette,
    }
}

/// One crate rect as a wire rect.
fn rect_out(rect: engine_ui::layout::Rect) -> RectOut {
    RectOut {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
    }
}

/// The palette contrast rows for `palette_id`.
fn palette_out(doc: &LayoutDoc, palette_id: u8) -> Result<PaletteOut, crate::mcp::McpToolError> {
    let report = doc.palette_report(palette_id).map_err(layout_refusal)?;
    let role = |index: usize| RoleOut {
        ratio_permille: report.roles[index].ratio_permille,
        required_permille: report.roles[index].required_permille,
        passes: report.roles[index].passes,
    };
    Ok(PaletteOut {
        palette: report.palette.to_owned(),
        body: role(0),
        accent: role(1),
        passes: report.passes,
    })
}

/// One drag on a parsed document, then the change receipt.
fn run_resize(args: &LayoutResizeArgs) -> Result<LayoutResizeOutput, crate::mcp::McpToolError> {
    let mut doc = read_document(&args.document)?;
    let viewport = Viewport::new(args.width_px, args.height_px);
    let shrink = read_shrink_order(args.shrink_order.as_ref())?;
    let previous = doc.solve(viewport, &shrink).map_err(layout_refusal)?;
    apply_edit(&mut doc, args)?;
    let edited = doc
        .fetch_changes(viewport, &shrink, Some(&previous))
        .map_err(layout_refusal)?;
    Ok(LayoutResizeOutput {
        moved: edited.moved.iter().map(|id| id.0).collect(),
        layout: solve_output(&doc, &edited.solved, None),
    })
}

/// Apply the one edit the args name.
fn apply_edit(
    doc: &mut LayoutDoc,
    args: &LayoutResizeArgs,
) -> Result<(), crate::mcp::McpToolError> {
    let id = PanelId(args.panel);
    match args.edit.as_str() {
        "resize" => {
            let px = args.px.ok_or_else(|| {
                field_refusal(
                    "px",
                    &format!("0..={MAX_TRACK_PX}"),
                    "give the drag's target width",
                )
            })?;
            let axis = match args.axis.as_deref() {
                None | Some("horizontal") => Axis::Horizontal,
                Some("vertical") => Axis::Vertical,
                Some(_) => {
                    return Err(field_refusal(
                        "axis",
                        "one of horizontal, vertical",
                        "pick horizontal for a panel drag",
                    ))
                }
            };
            doc.resize(id, axis, px).map_err(layout_refusal)
        }
        "pin" => {
            let width = read_constraint(
                args.width.as_ref().ok_or_else(|| {
                    field_refusal(
                        "width",
                        "one of fixed, minmax, fraction, aspect",
                        "give the width rule to pin",
                    )
                })?,
                "width",
            )?;
            doc.pin(id, width).map_err(layout_refusal)
        }
        "unpin" => {
            if doc.unpin(id) {
                Ok(())
            } else {
                Err(field_refusal(
                    "panel",
                    "a panel id with an active pin",
                    "pin or resize the panel first",
                ))
            }
        }
        other => Err(field_refusal(
            "edit",
            "one of pin, resize, unpin",
            &format!("rename `{other}` to one of the three edits"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packs::{PackRegistry, PACK_NAMES};

    /// The 2x2 grid the ui crate pins: margins 8, gaps 8, four fraction cells.
    fn grid_document() -> Value {
        json!({
            "margins": {"left": 8, "top": 8, "right": 8, "bottom": 8},
            "row_gap": 8,
            "rows": [
                {"height": {"fraction": [1, 1]}, "gap": 8, "panels": [
                    {"id": 0, "width": {"fraction": [1, 1]}},
                    {"id": 1, "width": {"fraction": [1, 1]}}
                ]},
                {"height": {"fraction": [1, 1]}, "gap": 8, "panels": [
                    {"id": 2, "width": {"fraction": [1, 1]}},
                    {"id": 3, "width": {"fraction": [1, 1]}}
                ]}
            ]
        })
    }

    /// A row of `count` fixed panels with `gap` between them.
    fn fixed_row(count: u16, px: u32, gap: u32) -> Value {
        let panels: Vec<Value> = (0..count)
            .map(|id| json!({"id": id, "width": {"fixed": px}}))
            .collect();
        json!({"rows": [{"height": {"fixed": 100}, "gap": gap, "panels": panels}]})
    }

    fn solve_args(document: Value, width_px: u32, height_px: u32) -> LayoutSolveArgs {
        LayoutSolveArgs {
            document,
            width_px,
            height_px,
            shrink_order: None,
            palette: None,
        }
    }

    fn rects(output: &LayoutSolveOutput) -> Vec<(u16, u32, u32, u32, u32)> {
        output
            .panels
            .iter()
            .map(|panel| {
                (
                    panel.id,
                    panel.rect.x,
                    panel.rect.y,
                    panel.rect.w,
                    panel.rect.h,
                )
            })
            .collect()
    }

    #[test]
    fn solve_reports_exact_cells_on_both_fixtures() {
        let first = run_solve(&solve_args(grid_document(), 1280, 720)).expect("1280x720 solves");
        assert_eq!(
            rects(&first),
            vec![
                (0, 8, 8, 628, 348),
                (1, 644, 8, 628, 348),
                (2, 8, 364, 628, 348),
                (3, 644, 364, 628, 348),
            ]
        );
        assert_eq!(
            (first.inner.x, first.inner.y, first.inner.w, first.inner.h),
            (8, 8, 1264, 704)
        );
        assert_eq!((first.leftover_w, first.leftover_h), (0, 0));
        // No margin cell left uncovered: every rect sits inside the inner box.
        for panel in &first.panels {
            assert!(panel.rect.x >= first.inner.x && panel.rect.y >= first.inner.y);
            assert!(panel.rect.x + panel.rect.w <= first.inner.x + first.inner.w);
            assert!(panel.rect.y + panel.rect.h <= first.inner.y + first.inner.h);
        }

        let second = run_solve(&solve_args(grid_document(), 360, 640)).expect("360x640 solves");
        assert_eq!(
            rects(&second),
            vec![
                (0, 8, 8, 168, 308),
                (1, 184, 8, 168, 308),
                (2, 8, 324, 168, 308),
                (3, 184, 324, 168, 308),
            ]
        );
        // Same document, same viewport, same digest; a different viewport moves it.
        let again = run_solve(&solve_args(grid_document(), 1280, 720)).expect("solves again");
        assert_eq!(again.digest, first.digest);
        assert_ne!(second.digest, first.digest);
        assert_eq!(first.digest.len(), 16);
    }

    #[test]
    fn solve_names_the_palette_rows_it_measures() {
        let mut args = solve_args(grid_document(), 1280, 720);
        args.palette = Some(1);
        let output = run_solve(&args).expect("palette 1 exists");
        let palette = output.palette.expect("a palette id asks for rows");
        assert!(palette.body.required_permille > 0);
        assert_eq!(palette.passes, palette.body.passes && palette.accent.passes);
        let mut bad = args;
        bad.palette = Some(9);
        let refusal = run_solve(&bad).expect_err("palette 9 refuses");
        assert_eq!(refusal.code, "bad_args");
        assert!(refusal.message.contains("0..=3"), "{}", refusal.message);
    }

    #[test]
    fn overflow_refusal_names_the_shortfall_and_the_first_shrink_panel() {
        let mut args = solve_args(fixed_row(3, 60, 8), 100, 100);
        args.shrink_order = Some(vec![0]);
        let refusal = run_solve(&args).expect_err("196 px of panels in 100 px");
        assert_eq!(refusal.code, "overflow");
        assert!(refusal.message.contains("96"), "{}", refusal.message);
        assert!(
            refusal.message.contains("first shrink panel: 0"),
            "{}",
            refusal.message
        );
        assert!(refusal.message.contains("recovery"), "{}", refusal.message);
    }

    #[test]
    fn resize_moves_the_dragged_panels_and_unpin_restores_the_first_digest() {
        let document = json!({"rows": [{"height": {"fixed": 720}, "panels": [
            {"id": 0, "width": {"fixed": 300}},
            {"id": 1, "width": {"fraction": [1, 1]}},
            {"id": 2, "width": {"fraction": [1, 1]}}
        ]}]});
        let baseline = run_solve(&solve_args(document.clone(), 1280, 720)).expect("baseline");
        assert_eq!(
            baseline
                .panels
                .iter()
                .map(|panel| panel.rect.w)
                .collect::<Vec<_>>(),
            vec![300, 490, 490]
        );

        let mut args = LayoutResizeArgs {
            document,
            width_px: 1280,
            height_px: 720,
            panel: 1,
            edit: "resize".to_owned(),
            px: Some(530),
            width: None,
            axis: None,
            shrink_order: None,
        };
        let first = run_resize(&args).expect("the drag runs");
        assert_eq!(first.moved, vec![1, 2]);
        assert_eq!(
            first
                .layout
                .panels
                .iter()
                .map(|panel| panel.rect.w)
                .collect::<Vec<_>>(),
            vec![300, 530, 450]
        );
        assert_ne!(first.layout.digest, baseline.digest);
        assert_eq!(first.layout.pins.len(), 1);
        assert_eq!(first.layout.pins[0].id, 1);

        // The same drag twice is a no-op: send the pins back and the receipt
        // is empty.
        let pinned = json!({"rows": [{"height": {"fixed": 720}, "panels": [
            {"id": 0, "width": {"fixed": 300}},
            {"id": 1, "width": {"fraction": [1, 1]}},
            {"id": 2, "width": {"fraction": [1, 1]}}
        ]}], "pins": rmcp::serde_json::to_value(&first.layout.pins).expect("pins serialize")});
        args.document = pinned.clone();
        let repeat = run_resize(&args).expect("the repeat drag runs");
        assert!(repeat.moved.is_empty(), "repeat moved {:?}", repeat.moved);
        assert_eq!(repeat.layout.digest, first.layout.digest);

        // Unpin returns the authored rects byte for byte.
        args.edit = "unpin".to_owned();
        args.px = None;
        let released = run_resize(&args).expect("unpin runs");
        assert_eq!(released.moved, vec![1, 2]);
        assert_eq!(released.layout.digest, baseline.digest);

        // A vertical drag is refused with the crate's own sentence.
        args.edit = "resize".to_owned();
        args.px = Some(400);
        args.axis = Some("vertical".to_owned());
        args.document = pinned;
        let refusal = run_resize(&args).expect_err("vertical drag refuses");
        assert!(refusal.message.contains("vertical"), "{}", refusal.message);
    }

    #[test]
    fn bad_documents_name_the_field_the_band_and_one_recovery() {
        let refusal =
            run_solve(&solve_args(json!({"rows": []}), 800, 600)).expect_err("no rows refuses");
        assert!(
            refusal.message.contains("document.rows"),
            "{}",
            refusal.message
        );
        assert!(refusal.message.contains("1..=32"), "{}", refusal.message);
        assert!(refusal.message.contains("recovery"), "{}", refusal.message);

        let mut document = grid_document();
        document["rows"][0]["panels"][0]["width"] = json!({"fixed": -1});
        let refusal = run_solve(&solve_args(document, 800, 600)).expect_err("bad width refuses");
        assert!(
            refusal
                .message
                .contains("document.rows[0].panels[0].width.fixed"),
            "{}",
            refusal.message
        );

        let mut document = grid_document();
        if let Some(panels) = document["rows"][0]["panels"].as_array_mut() {
            for id in 0..40 {
                panels.push(json!({"id": 100 + id, "width": {"fixed": 10}}));
            }
        }
        let refusal = run_solve(&solve_args(document, 800, 600)).expect_err("41 panels refuse");
        assert!(refusal.message.contains("1..=32"), "{}", refusal.message);
    }

    #[test]
    fn the_registry_serves_both_ui_verbs_and_neither_is_default_loaded() {
        let registry = PackRegistry::shipped();
        assert!(registry.errors().is_empty(), "{:?}", registry.errors());
        assert!(PACK_NAMES.contains(&PACK));
        assert!(!registry.default_loaded().contains(PACK));
        let solve = registry.verb(TOOL_LAYOUT_SOLVE).expect("solve registers");
        assert_eq!(solve.pack(), PACK);
        assert_eq!(solve.card(), CapabilityCard::Observe);
        assert!(solve.readonly());
        let resize = registry.verb(TOOL_LAYOUT_RESIZE).expect("resize registers");
        assert_eq!(resize.pack(), PACK);
        assert_eq!(resize.card(), CapabilityCard::Commit);
        assert!(!resize.readonly());
    }
}
