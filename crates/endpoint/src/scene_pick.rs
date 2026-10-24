//! `scene.pick`: point at a capture, get the object or the floor point.
//!
//! Consumer: any MCP client that looked at a capture and wants the exact
//! object under a pixel, the pixel of an object, or the frame as regions of
//! objects; `scene.place` takes the answer back as a label. Data owner: the
//! staged document, read only; the camera maths and the manifest text are
//! [`super::pick`]'s. Units: integer centimeters and centidegrees for the
//! camera, pixels in the frame. The caller sends the camera of its capture
//! (the door cannot see which camera made a PNG). Determinism: the same
//! scene and arguments give the same bytes and the same digest. Errors:
//! typed refusals naming the legal range and one recovery.

use std::borrow::Cow;

use engine_core::hash_hex;
use engine_scene::ids::ObjectId;
use engine_spatial::Actor;
use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::model::ToolAnnotations;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::mcp::{McpServer, McpToolError, ServerState};
use super::pick::{resolve_name, Camera, NameError, PickManifest, Picked};
use super::play::{CAPTURE_DEFAULT_HEIGHT, CAPTURE_DEFAULT_WIDTH, CAPTURE_MAX_DIM};
use super::scene_query::label_of;
use super::scene_tools::{parse_object_ref, snapshot};

/// Door tool name for pointing at a capture.
pub const TOOL_SCENE_PICK: &str = "scene.pick";
/// Vertical field of view when the caller sends none, in centidegrees.
pub const DEFAULT_FOV_CDEG: i32 = 6_000;
/// Most manifest samples one call may take.
pub const MAX_PICK_CELLS: u32 = 4_096;

/// Arguments of `scene.pick`: the capture's camera plus exactly one question.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ScenePickArgs {
    /// Eye [x, y, z] in cm, y up.
    pub eye_cm: [i32; 3],
    /// Turn about y in centidegrees; 0 looks along +z, positive turns toward +x.
    pub yaw_cdeg: i32,
    /// Tilt in centidegrees, negative looks down; default 0.
    pub pitch_cdeg: Option<i32>,
    /// Vertical field of view in centidegrees, 100..=17900; default 6000.
    pub fov_cdeg: Option<i32>,
    /// Frame [width, height] in pixels, each 1..=2048; default [96, 64].
    pub size: Option<[u32; 2]>,
    /// Question 1: pixel [x, y] to read; answers the object or the floor point there.
    pub pixel: Option<[u32; 2]>,
    /// Question 2: object label (`lamp_2 [uuid]`) to find in the frame.
    pub object: Option<String>,
    /// Question 3: cell edge in pixels; answers the frame as rectangles of objects.
    pub cell: Option<u32>,
    /// Question 4: a mark id from `view.observe` (a body slot) to find in the frame.
    pub mark: Option<u64>,
}

/// Answer of `scene.pick`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ScenePickOutput {
    /// Which question this answers: pixel, object or cell.
    pub kind: String,
    /// Digest of the answer text; equal for equal input.
    pub digest: String,
    /// Pixel question: the object label under the pixel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity: Option<String>,
    /// Pixel question with no object: the floor point [x, y, z] in cm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point_cm: Option<[i32; 3]>,
    /// Object question: the pixel [x, y] its center lands in; absent behind the eye.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[i32; 2]>,
    /// Object question: distance along the view axis in cm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_cm: Option<i32>,
    /// Object question: true when `at` is inside the frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_frame: Option<bool>,
    /// Cell question: `x0 y0 x1 y1 label|-` per rectangle; they tile the frame.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<String>,
}

/// The capture camera, or the range that was missed.
fn camera_of(args: &ScenePickArgs) -> Result<Camera, McpToolError> {
    let size = args
        .size
        .unwrap_or([CAPTURE_DEFAULT_WIDTH, CAPTURE_DEFAULT_HEIGHT]);
    if size
        .iter()
        .any(|side| !(1..=CAPTURE_MAX_DIM).contains(side))
    {
        return Err(McpToolError::new(
            "bad_camera",
            format!(
                "size {}x{} is outside 1..={CAPTURE_MAX_DIM} per side; send the size of the capture",
                size[0], size[1]
            ),
        ));
    }
    let fov_cdeg = args.fov_cdeg.unwrap_or(DEFAULT_FOV_CDEG);
    if !(100..=17_900).contains(&fov_cdeg) {
        return Err(McpToolError::new(
            "bad_camera",
            format!("fov_cdeg {fov_cdeg} is outside 100..=17900 (1 to 179 degrees)"),
        ));
    }
    let pitch_cdeg = args.pitch_cdeg.unwrap_or(0);
    if !(-8_999..=8_999).contains(&pitch_cdeg) {
        return Err(McpToolError::new(
            "bad_camera",
            format!(
                "pitch_cdeg {pitch_cdeg} is outside -8999..=8999 (straight down to straight up)"
            ),
        ));
    }
    Ok(Camera {
        eye_cm: args.eye_cm,
        yaw_cdeg: args.yaw_cdeg,
        pitch_cdeg,
        fov_cdeg,
        size,
    })
}

/// Samples a `cell`-pixel grid takes over a frame.
fn cells_in(size: [u32; 2], cell: u32) -> u32 {
    size[0]
        .div_ceil(cell)
        .saturating_mul(size[1].div_ceil(cell))
}

fn check_cell(size: [u32; 2], cell: u32) -> Result<(), McpToolError> {
    if cell == 0 || cells_in(size, cell) > MAX_PICK_CELLS {
        let least = (1..=size[0].max(size[1]))
            .find(|edge| cells_in(size, *edge) <= MAX_PICK_CELLS)
            .unwrap_or(1);
        return Err(McpToolError::new(
            "bad_cell",
            format!(
                "cell {cell} must make 1..={MAX_PICK_CELLS} samples of a {}x{} frame; send a cell of at least {least}",
                size[0], size[1]
            ),
        ));
    }
    Ok(())
}

/// The object question's answer: where one staged object projects, with the
/// refusal a non-finite pose gets.
fn project_output(
    camera: &Camera,
    actor: &Actor,
    object: &str,
) -> Result<ScenePickOutput, McpToolError> {
    let center = [actor.x, actor.y, actor.z].map(|m| f64::from(m) * 100.0);
    if !center.iter().all(|lane| lane.is_finite()) {
        return Err(McpToolError::new(
            "no_pose",
            format!("{object} has no finite position; place it first with scene.place"),
        ));
    }
    let found = camera.project_px(center);
    Ok(ScenePickOutput {
        kind: "object".to_owned(),
        at: found.map(|(pixel, _)| pixel),
        depth_cm: found.map(|(_, depth)| depth),
        in_frame: found.map(|([x, y], _)| {
            let inside = |at: i32, side: u32| at >= 0 && i64::from(at) < i64::from(side);
            inside(x, camera.size[0]) && inside(y, camera.size[1])
        }),
        ..Default::default()
    })
}

/// The staged object a mark names: a body slot from `view.observe`.
fn marked_id(mark: u64, actors: &[Actor]) -> Result<u32, McpToolError> {
    let count = actors.len();
    usize::try_from(mark)
        .ok()
        .filter(|at| *at < count)
        .and_then(|at| u32::try_from(at).ok())
        .ok_or_else(|| {
            McpToolError::new(
                "unknown_actor",
                format!(
                    "mark {mark} names no drawn body; legal marks are 0..={} for the {count} \
                     staged objects: re-run view.observe and pick one of its marks",
                    count.saturating_sub(1)
                ),
            )
        })
}

impl ServerState {
    /// Answer one `scene.pick` call over the staged document. Read-only.
    pub fn scene_pick(&self, args: &ScenePickArgs) -> Result<ScenePickOutput, McpToolError> {
        let camera = camera_of(args)?;
        let snap = snapshot(&self.doc);
        let label = |id: u32| label_of(id, &snap.actors, &snap.ids);
        let mut out = match (args.pixel, &args.object, args.cell, args.mark) {
            (Some([x, y]), None, None, None) => {
                if x >= camera.size[0] || y >= camera.size[1] {
                    return Err(McpToolError::new(
                        "bad_pixel",
                        format!(
                            "pixel {x},{y} is outside the {}x{} frame; send x in 0..{} and y in 0..{}",
                            camera.size[0], camera.size[1], camera.size[0], camera.size[1]
                        ),
                    ));
                }
                match camera.pick(&snap.actors, x, y) {
                    Picked::Entity(id) => ScenePickOutput {
                        kind: "pixel".to_owned(),
                        entity: Some(label(id)),
                        ..Default::default()
                    },
                    Picked::Point(point) => ScenePickOutput {
                        kind: "pixel".to_owned(),
                        point_cm: Some(point),
                        ..Default::default()
                    },
                }
            }
            (None, Some(object), None, None) => {
                let id = object_id(object, &snap.actors, &snap.ids)?;
                let actor = &snap.actors[usize::try_from(id).unwrap_or(usize::MAX)];
                project_output(&camera, actor, object)?
            }
            (None, None, None, Some(mark)) => {
                let id = marked_id(mark, &snap.actors)?;
                let actor = &snap.actors[usize::try_from(id).unwrap_or(usize::MAX)];
                let mut out = project_output(&camera, actor, &label(id))?;
                out.kind = "mark".to_owned();
                out.entity = Some(label(id));
                out
            }
            (None, None, Some(cell), None) => {
                check_cell(camera.size, cell)?;
                let manifest =
                    PickManifest::build(&camera, &snap.actors, cell).ok_or_else(|| {
                        McpToolError::new("bad_cell", format!("cell {cell} makes no samples"))
                    })?;
                ScenePickOutput {
                    kind: "cell".to_owned(),
                    regions: manifest
                        .regions
                        .iter()
                        .map(|r| {
                            let who = r.id.map_or_else(|| "-".to_owned(), label);
                            format!("{} {} {} {} {who}", r.x0, r.y0, r.x1, r.y1)
                        })
                        .collect(),
                    ..Default::default()
                }
            }
            _ => {
                return Err(McpToolError::new(
                    "bad_pick",
                    "send exactly one of pixel, object, cell or mark; pixel reads one point, \
                     object finds a label in the frame, cell lists the frame as regions, mark \
                     finds one view.observe mark"
                        .to_owned(),
                ))
            }
        };
        let text = rmcp::serde_json::to_string(&out).unwrap_or_default();
        out.digest = hash_hex(text.as_bytes());
        Ok(out)
    }
}

/// The staged actor a label or bare name names. The bracketed uuid of a label
/// decides and its name part is ignored; a bare name resolves only when
/// exactly one object has it.
fn object_id(object: &str, actors: &[Actor], ids: &[ObjectId]) -> Result<u32, McpToolError> {
    let unknown = || {
        McpToolError::new(
            "unknown_actor",
            format!(
                "object `{object}` names no staged object ({} staged; re-read scene.brief \
                 for the staged labels)",
                ids.len()
            ),
        )
    };
    match parse_object_ref("object", object) {
        Ok(uuid) => ids
            .iter()
            .position(|id| *id == uuid)
            .map(|index| u32::try_from(index).unwrap_or(u32::MAX))
            .ok_or_else(unknown),
        Err(_) if object.contains('[') => Err(unknown()),
        Err(_) => match resolve_name(actors, object.trim()) {
            Ok(id) => Ok(id),
            Err(NameError::Missing) => Err(unknown()),
            Err(NameError::Ambiguous(twins)) => Err(McpToolError::new(
                "ambiguous_name",
                format!(
                    "name `{object}` is shared by {} objects; send one label with its \
                     bracketed id: {}",
                    twins.len(),
                    twins
                        .iter()
                        .map(|twin| label_of(*twin, actors, ids))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )),
        },
    }
}

/// MCP tool registration for `scene.pick` (read-only).
pub struct ScenePickTool;

impl ToolBase for ScenePickTool {
    type Parameter = ScenePickArgs;
    type Output = ScenePickOutput;
    type Error = McpToolError;

    fn name() -> Cow<'static, str> {
        TOOL_SCENE_PICK.into()
    }

    fn description() -> Option<Cow<'static, str>> {
        Some(
            "Point at a capture: send its camera plus a pixel (object or floor point), an object \
             label (its pixel) or a cell (the frame as object regions). Read-only."
                .into(),
        )
    }

    fn annotations() -> Option<ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for ScenePickTool {
    fn invoke(service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        service.dispatch(TOOL_SCENE_PICK, |state| state.scene_pick(&param))
    }
}

#[cfg(test)]
mod tests;
