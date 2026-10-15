//! Viewport texture: CPU-rasterized panel image validated by the renderer's texture API.
//!
//! The consumer is the editor viewport panel (the live egui texture) and the
//! operator's Xvfb smoke. The data owner is [`ViewportImage`]: an RGBA8 pixel
//! buffer in a fixed panel size. Units are pixels for sizes and marker radii.
//!
//! Two rendering paths share the panel:
//!
//! * Live texture (here): rasterized on the CPU from the [`Shell`] rows
//!   through the [`Viewport`] camera (floor grid, node markers, selection
//!   ring). Every upload is validated by the renderer's
//!   [`TextureUpload`](engine_render::present::TextureUpload) gate, so the
//!   same byte-length and dimension rules guard the panel and the GPU cache.
//!   Same document plus same camera always yields the same bytes.
//! * GPU snapshot ([`app`](crate::app)): the [`Presenter`](engine_render::present::Presenter)
//!   renders a draw list derived from the same document offscreen and writes
//!   PNG receipts for the reference-GPU session. Compositing Presenter output
//!   live inside egui needs a shared-device constructor in `engine-render`
//!   (proposed in the editor report); until then the live texture stays CPU-side.
//!
//! Sizes follow the renderer's [`SizeClass`](engine_render::present::SizeClass):
//! zero area suspends (minimize keeps last-good), over-bound rejects. The
//! live texture additionally caps edges at [`MAX_VIEWPORT_TEX`] pixels so the
//! per-frame egui upload stays cheap; full-size GPU snapshots use the renderer's own
//! 2048 bound.

use engine_render::present::{SizeClass, TextureUpload};
use engine_scene::transform::{Hierarchy, HierarchyError};

use crate::shell::{SceneRow, Shell};

/// Largest live-texture edge in pixels. Larger panels downscale through the
/// shell viewport size before rasterizing; GPU snapshots keep the renderer's own bound.
pub const MAX_VIEWPORT_TEX: u32 = 1024;
/// Grid spacing in pixels for the floor backdrop.
const GRID_STEP_PX: u32 = 32;
/// Half-size in pixels of a node marker square.
const MARKER_HALF_PX: i32 = 3;
/// Half-size in pixels of the selected-node marker square.
const SELECTED_HALF_PX: i32 = 5;

/// What rejected a viewport raster. Sizes only: a degenerate camera still
/// draws the grid (markers are skipped), so the panel never goes blank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewportImageError {
    /// Zero area (minimized): keep the last-good texture, draw nothing.
    Suspended,
    /// An edge past [`MAX_VIEWPORT_TEX`] (or past the renderer's frame bound).
    BadSize {
        /// Requested width in pixels.
        width: u32,
        /// Requested height in pixels.
        height: u32,
    },
    /// A snapshot row rebuild failed (duplicate id, unknown parent or
    /// non-finite pose); the panel refuses instead of silently dropping
    /// markers.
    Hierarchy(HierarchyError),
}

impl std::fmt::Display for ViewportImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Suspended => f.write_str("viewport suspended (zero area): keep last-good"),
            Self::BadSize { width, height } => write!(
                f,
                "viewport size {width}x{height} rejected (live cap {MAX_VIEWPORT_TEX}px)"
            ),
            Self::Hierarchy(error) => write!(f, "viewport markers: {error}"),
        }
    }
}

impl std::error::Error for ViewportImageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Hierarchy(error) => Some(error),
            _ => None,
        }
    }
}

/// One rasterized viewport frame: RGBA8 pixels plus the source revision.
///
/// `revision` is the [`Shell`] revision rasterized (the panel re-renders when
/// it moves); `draws` counts node markers, mirroring the renderer's receipt naming.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewportImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Exactly `width * height * 4` bytes, row-major from the top-left.
    pub rgba: Vec<u8>,
    /// Shell revision rasterized.
    pub revision: u64,
    /// Node markers drawn.
    pub draws: u32,
}

impl ViewportImage {
    /// Rasterize the shell through its viewport camera at `width` x `height`.
    ///
    /// Floor grid plus one marker per pickable row (amber for the selection).
    /// Err on zero area ([`ViewportImageError::Suspended`]), over-bound
    /// sizes or a corrupt snapshot row ([`ViewportImageError::Hierarchy`]);
    /// a degenerate camera draws the grid without markers.
    pub fn render(shell: &Shell, width: u32, height: u32) -> Result<Self, ViewportImageError> {
        match SizeClass::classify(width, height) {
            SizeClass::Suspend => return Err(ViewportImageError::Suspended),
            SizeClass::Reject => {
                return Err(ViewportImageError::BadSize { width, height });
            }
            SizeClass::Sizes => {}
        }
        if width > MAX_VIEWPORT_TEX || height > MAX_VIEWPORT_TEX {
            return Err(ViewportImageError::BadSize { width, height });
        }
        let mut rgba = vec![0u8; width as usize * height as usize * 4];
        paint_grid(&mut rgba, width, height);
        let draws = paint_markers(&mut rgba, width, height, shell)
            .map_err(ViewportImageError::Hierarchy)?;
        Ok(Self {
            width,
            height,
            rgba,
            revision: shell.revision(),
            draws,
        })
    }

    /// View this frame as a renderer texture upload. The returned value always
    /// passes [`TextureUpload::validate`]; callers re-validate before any GPU
    /// handoff so the gate stays on the upload path, not just construction.
    pub fn to_upload(&self) -> TextureUpload {
        TextureUpload {
            width: self.width,
            height: self.height,
            rgba: self.rgba.clone(),
        }
    }

    /// View this frame as an egui image for the live texture handle.
    #[cfg(feature = "ui")]
    pub fn to_color_image(&self) -> egui::ColorImage {
        egui::ColorImage::from_rgba_unmultiplied(
            [self.width as usize, self.height as usize],
            &self.rgba,
        )
    }
}

/// Dark renderer-clear backdrop with a lighter grid and a center crosshair.
fn paint_grid(rgba: &mut [u8], width: u32, height: u32) {
    const BG: [u8; 3] = [7, 8, 15];
    const LINE: [u8; 3] = [22, 26, 44];
    const CROSS: [u8; 3] = [60, 66, 96];
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        let x = i as u32 % width;
        let y = i as u32 / width;
        let mut c = BG;
        if x.is_multiple_of(GRID_STEP_PX) || y.is_multiple_of(GRID_STEP_PX) {
            c = LINE;
        }
        if x == width / 2 || y == height / 2 {
            c = CROSS;
        }
        px[0..3].copy_from_slice(&c);
        px[3] = 255;
    }
}

/// Rebuild the marker hierarchy from snapshot rows in row order (roots
/// first, so parents insert before children). Any duplicate, unknown
/// parent or non-finite pose fails here instead of dropping a marker.
fn marker_tree(rows: &[SceneRow]) -> Result<Hierarchy, HierarchyError> {
    let mut tree = Hierarchy::new();
    for row in rows {
        tree.insert(row.id, row.parent, row.local)?;
    }
    Ok(tree)
}

/// One square per row world position, amber for the selection. Returns the
/// marker count. Rows that project behind the camera or off-panel are skipped;
/// a degenerate camera draws nothing (grid only). A corrupt snapshot row
/// (duplicate id, unknown parent, non-finite pose) is a typed error, never
/// a silently dropped marker.
fn paint_markers(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    shell: &Shell,
) -> Result<u32, HierarchyError> {
    const NODE: [u8; 3] = [235, 235, 240];
    const SELECTED: [u8; 3] = [255, 180, 40];
    let rows = shell.rows();
    if rows.is_empty() {
        return Ok(0);
    }
    // Rebuild world poses from the snapshot alone (same rule as picking), so
    // markers sit where the nodes are drawn.
    let tree = marker_tree(&rows)?;
    let camera = shell.viewport.camera;
    let aspect = f64::from(shell.viewport.size.aspect());
    let (Some(view), Some(proj)) = (
        camera.view_matrix_rows(),
        camera.projection_matrix_rows(aspect),
    ) else {
        return Ok(0);
    };
    let mut draws = 0u32;
    for row in &rows {
        let Some(world) = tree.world(row.id) else {
            continue;
        };
        let Some((px, py)) = project(world.position, &view, &proj, width, height) else {
            continue;
        };
        let selected = shell.selection == Some(row.id);
        let half = if selected {
            SELECTED_HALF_PX
        } else {
            MARKER_HALF_PX
        };
        let color = if selected { SELECTED } else { NODE };
        fill_square(rgba, width, height, px, py, half, color);
        draws += 1;
    }
    Ok(draws)
}

/// Project a world point to panel pixels. None behind the camera, outside the
/// clip volume, or on garbage math. Origin top-left, units pixels.
fn project(
    world: [f32; 3],
    view: &[[f64; 4]; 4],
    proj: &[[f64; 4]; 4],
    width: u32,
    height: u32,
) -> Option<(i32, i32)> {
    let p = [
        f64::from(world[0]),
        f64::from(world[1]),
        f64::from(world[2]),
    ];
    let eye = mat_vec(view, p);
    let clip = mat_vec(proj, [eye[0], eye[1], eye[2]]);
    if !clip[3].is_finite() || clip[3] <= 0.0 {
        return None;
    }
    let ndc = [clip[0] / clip[3], clip[1] / clip[3]];
    if !ndc[0].is_finite() || !ndc[1].is_finite() {
        return None;
    }
    if ndc[0].abs() > 1.0 || ndc[1].abs() > 1.0 {
        return None;
    }
    let x = ((ndc[0] * 0.5 + 0.5) * f64::from(width)).round();
    let y = ((1.0 - (ndc[1] * 0.5 + 0.5)) * f64::from(height)).round();
    if x < 0.0 || y < 0.0 || x >= f64::from(width) || y >= f64::from(height) {
        return None;
    }
    Some((x as i32, y as i32))
}

/// Row-major matrix times a point (homogeneous w = 1 in, full clip out).
fn mat_vec(m: &[[f64; 4]; 4], p: [f64; 3]) -> [f64; 4] {
    [
        m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2] + m[0][3],
        m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2] + m[1][3],
        m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2] + m[2][3],
        m[3][0] * p[0] + m[3][1] * p[1] + m[3][2] * p[2] + m[3][3],
    ]
}

/// Fill a `(2*half+1)` square clipped to the panel.
fn fill_square(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    cx: i32,
    cy: i32,
    half: i32,
    color: [u8; 3],
) {
    for dy in -half..=half {
        for dx in -half..=half {
            let (x, y) = (cx + dx, cy + dy);
            if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                continue;
            }
            let i = (y as u32 * width + x as u32) as usize * 4;
            rgba[i..i + 3].copy_from_slice(&color);
            rgba[i + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::ids::ObjectId;
    use engine_scene::transform::Transform;

    fn pose_at(x: f32) -> Transform {
        Transform::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0])
    }

    fn seeded_shell() -> (Shell, ObjectId) {
        let mut shell = Shell::new();
        let root = ObjectId::new();
        shell
            .insert_node(root, ObjectId::NIL, pose_at(0.0))
            .unwrap();
        (shell, root)
    }

    #[test]
    fn renders_deterministically_with_revision_and_draws() {
        let (mut shell, root) = seeded_shell();
        assert!(shell.select(root));
        let first = ViewportImage::render(&shell, 320, 200).unwrap();
        assert_eq!((first.width, first.height), (320, 200));
        assert_eq!(first.rgba.len(), 320 * 200 * 4);
        assert_eq!(first.revision, shell.revision());
        assert_eq!(first.draws, 1);
        // Same document plus same camera is byte-identical.
        assert_eq!(ViewportImage::render(&shell, 320, 200).unwrap(), first);
    }

    #[test]
    fn suspend_and_reject_sizes() {
        let (shell, _) = seeded_shell();
        assert_eq!(
            ViewportImage::render(&shell, 0, 600),
            Err(ViewportImageError::Suspended)
        );
        assert_eq!(
            ViewportImage::render(&shell, 0, 0),
            Err(ViewportImageError::Suspended)
        );
        assert_eq!(
            ViewportImage::render(&shell, MAX_VIEWPORT_TEX + 1, 8),
            Err(ViewportImageError::BadSize {
                width: MAX_VIEWPORT_TEX + 1,
                height: 8
            })
        );
        assert!(ViewportImage::render(&shell, MAX_VIEWPORT_TEX, MAX_VIEWPORT_TEX).is_ok());
        let report = ViewportImage::render(&shell, 9999, 8)
            .unwrap_err()
            .to_string();
        assert!(report.contains("9999"));
    }

    #[test]
    fn selection_changes_pixels() {
        let (mut shell, root) = seeded_shell();
        let plain = ViewportImage::render(&shell, 320, 200).unwrap();
        assert_eq!(plain.draws, 1);
        assert!(shell.select(root));
        let picked = ViewportImage::render(&shell, 320, 200).unwrap();
        assert_eq!(picked.draws, 1);
        assert_ne!(plain.rgba, picked.rgba, "amber ring differs from plain dot");
    }

    #[test]
    fn empty_scene_is_grid_baseline() {
        let shell = Shell::new();
        let img = ViewportImage::render(&shell, 160, 120).unwrap();
        assert_eq!((img.draws, img.revision), (0, 0));
        // Top-left corner sits on both grid lines: the LINE color, opaque.
        assert_eq!(&img.rgba[0..4], &[22, 26, 44, 255]);
    }

    #[test]
    fn upload_passes_the_ra1_gate() {
        let (shell, _) = seeded_shell();
        let img = ViewportImage::render(&shell, 64, 48).unwrap();
        img.to_upload().validate().expect("live frame uploads");
        // The renderer gate itself still refuses garbage lengths.
        let bad = TextureUpload {
            width: 4,
            height: 4,
            rgba: vec![0u8; 63],
        };
        assert!(bad.validate().is_err());
    }

    #[test]
    fn degenerate_camera_still_draws_grid() {
        let (mut shell, _) = seeded_shell();
        shell.viewport.camera.position = shell.viewport.camera.target;
        let img = ViewportImage::render(&shell, 160, 120).unwrap();
        assert_eq!(img.draws, 0, "no markers without a valid camera");
        assert_eq!(img.rgba.len(), 160 * 120 * 4);
    }

    #[test]
    fn corrupt_marker_rows_are_typed_errors() {
        // Duplicate ids refuse instead of silently dropping a marker.
        let dup = ObjectId::new();
        let rows = vec![
            SceneRow {
                id: dup,
                parent: ObjectId::NIL,
                depth: 0,
                local: pose_at(0.0),
            },
            SceneRow {
                id: dup,
                parent: ObjectId::NIL,
                depth: 0,
                local: pose_at(1.0),
            },
        ];
        assert_eq!(
            marker_tree(&rows),
            Err(HierarchyError::Duplicate),
            "repeated row id refuses"
        );
        // A child naming a missing parent refuses as well.
        let orphan_rows = vec![SceneRow {
            id: ObjectId::new(),
            parent: ObjectId::new(),
            depth: 1,
            local: pose_at(0.0),
        }];
        assert_eq!(
            marker_tree(&orphan_rows),
            Err(HierarchyError::UnknownParent),
            "missing parent refuses"
        );
        // The render error wraps the typed cause with a source chain.
        let wrapped = ViewportImageError::Hierarchy(HierarchyError::Duplicate);
        assert!(wrapped.to_string().contains("already exists"));
        assert!(std::error::Error::source(&wrapped).is_some());
    }
}
