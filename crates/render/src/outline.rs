//! outline: the locked fighter's screen-space x-ray outline (SW150), the CPU
//! twin of `shaders/outline_mask.wgsl` plus `shaders/outline_edge.wgsl`.
//!
//! Shape, ported from three.js `examples/jsm/postprocessing/OutlinePass.js`
//! (`render`, `_getPrepareMaskMaterial`, `_updateTextureMatrix`,
//! `_getEdgeDetectionMaterial`, `_getOverlayMaterial`): the selected ids are
//! drawn white into a half-res mask over a depth prefill of everything else,
//! and the mask's second channel says whether the fragment passed the depth
//! test, so a target behind the sand berm carries 0 and one in front carries 1.
//! A Sobel edge pass over the mask coverage colours each edge texel from that
//! visibility (visible white, hidden the dim warm 0.1/0.04/0.02), a thickness
//! blur and a wider glow blur spread the band, and one additive composite lays
//! it on the frame. The edge colour never reads the frame pixels, so the
//! outline shows through sand and bodies instead of vanishing behind them.
//! Every tap clamps to the field's edge the way a `ClampToEdge` sampler does,
//! so the mask's own border never draws a false edge.
//!
//! Numbers are the donor's: mask at 1/2 of the frame on both axes
//! ([`OUTLINE_DOWN_SAMPLE`]), [`EDGE_STRENGTH`] 3.0, [`EDGE_THICKNESS`] 1.0,
//! [`MAX_EDGE_THICKNESS`] and [`MAX_EDGE_GLOW`] 4, [`VISIBLE_EDGE_COLOR`]
//! white, [`HIDDEN_EDGE_COLOR`] 0.1/0.04/0.02, and the cosine [`pulse_scale`].
//!
//! Units: metres and radians, texels for sizes, linear colour `0..=1` per
//! channel before the additive composite. Determinism: `+ - * /` and `sqrt`
//! only, fixed texel order, no clock and no map, so the same camera, prefill
//! and selection give the same bytes twice. Presentation only: nothing here is
//! read or written by the simulation and nothing is hashed.

use crate::draw::{dot, len, scale, sub};
use crate::quality::Quality;
use engine_core::vec3::cross3;

/// `a > b`, with `false` for a NaN on either side: the form every rejection
/// below wants, stated once so the NaN rule reads the same at each site.
fn is_greater(a: f32, b: f32) -> bool {
    a.partial_cmp(&b) == Some(core::cmp::Ordering::Greater)
}

/// The mask renders at 1/this of the frame on both axes: 1280x720 runs the
/// mask at 640x360. The donor's `downSampleRatio`.
pub const OUTLINE_DOWN_SAMPLE: u32 = 2;
/// Multiplier on the Sobel gradient before the composite: the donor's
/// `edgeStrength`.
pub const EDGE_STRENGTH: f32 = 3.0;
/// Thickness kernel radius in mask texels the quality default uses: the
/// donor's `edgeThickness`.
pub const EDGE_THICKNESS: f32 = 1.0;
/// Largest thickness kernel radius: the donor's `MAX_EDGE_THICKNESS`.
pub const MAX_EDGE_THICKNESS: u32 = 4;
/// Largest glow kernel radius: the donor's `MAX_EDGE_GLOW`.
pub const MAX_EDGE_GLOW: u32 = 4;
/// Edge colour of a target in front of the depth prefill (the donor's
/// `visibleEdgeColor`).
pub const VISIBLE_EDGE_COLOR: [f32; 3] = [1.0, 1.0, 1.0];
/// Edge colour of a target the prefill covers: the donor's `hiddenEdgeColor`,
/// a dim warm ember so a fighter behind the berm still reads.
pub const HIDDEN_EDGE_COLOR: [f32; 3] = [0.1, 0.04, 0.02];
/// How far behind the prefill a fragment may sit and still count as visible,
/// metres: the donor's depth-compare slack.
pub const DEPTH_BIAS_M: f32 = 0.02;

/// How much of the outline a quality tier draws: the row's knob.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutlineTier {
    /// No mask, no edge, no composite: 0 draws.
    Off,
    /// The mask at [`OUTLINE_DOWN_SAMPLE`] of the frame and the full edge
    /// chain.
    Half,
}

/// The outline tier for a quality tier: off on `Potato` (the iGPU floor has no
/// spare 640x360 pass plus four blurs), half on every other tier. Pure table.
#[must_use]
pub const fn outline_tier(quality: Quality) -> OutlineTier {
    match quality {
        Quality::Potato => OutlineTier::Off,
        Quality::Web | Quality::Mobile | Quality::Comfort | Quality::High => OutlineTier::Half,
    }
}

/// What one frame's outline costs, before any device work.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OutlinePlan {
    /// Whether the chain runs at all this frame.
    pub enabled: bool,
    /// Mask target size in texels, `[0, 0]` when disabled.
    pub mask_size: [u32; 2],
    /// Mask draws: one per selected id, since the donor draws the selected
    /// meshes themselves. Zero ids cost zero draws.
    pub draws: u32,
}

/// The frame's outline plan: enabled when the tier draws it, the frame has
/// area and at least one id is selected. The mask is `ceil(frame / 2)` in both
/// axes, the same half-target rule `gtao_half` uses.
#[must_use]
pub fn outline_plan(
    quality: Quality,
    frame_width_px: u32,
    frame_height_px: u32,
    selected_ids: usize,
) -> OutlinePlan {
    let off = OutlinePlan {
        enabled: false,
        mask_size: [0, 0],
        draws: 0,
    };
    if outline_tier(quality) == OutlineTier::Off || frame_width_px == 0 || frame_height_px == 0 {
        return off;
    }
    let Some(draws) = u32::try_from(selected_ids).ok().filter(|n| *n > 0) else {
        return off;
    };
    OutlinePlan {
        enabled: true,
        mask_size: [
            frame_width_px.div_ceil(OUTLINE_DOWN_SAMPLE),
            frame_height_px.div_ceil(OUTLINE_DOWN_SAMPLE),
        ],
        draws,
    }
}

/// The camera the mask is projected with: eye, orthonormal basis and the
/// half-frame tangents.
#[derive(Clone, Copy, Debug)]
pub struct OutlineView {
    /// Eye position, metres.
    pub eye: [f32; 3],
    /// Camera right axis, unit.
    pub right: [f32; 3],
    /// Camera up axis, unit.
    pub up: [f32; 3],
    /// Camera forward axis, unit; the camera looks along `+forward`.
    pub forward: [f32; 3],
    /// Half-width tangent at unit distance, `tan(fov_y / 2) * aspect`.
    pub tan_x: f32,
    /// Half-height tangent at unit distance, `tan(fov_y / 2)`.
    pub tan_y: f32,
    /// Near plane, metres.
    pub near_m: f32,
    /// Far plane, metres; the mask pass and the prefill agree on it.
    pub far_m: f32,
}

impl OutlineView {
    /// A view from an eye, a target, a vertical field of view and the frame
    /// aspect. `None` when the eye and target coincide, the fov is not in
    /// `(0, pi)`, or the aspect, near or far plane is unusable: a degenerate
    /// view has no projection.
    #[must_use]
    pub fn new(
        eye: [f32; 3],
        target: [f32; 3],
        fov_y_rad: f32,
        aspect: f32,
        near_m: f32,
        far_m: f32,
    ) -> Option<Self> {
        let f = sub(target, eye);
        let dist = len(f);
        if !(fov_y_rad > 0.0 && fov_y_rad < core::f32::consts::PI) || !is_greater(dist, 1e-6) {
            return None;
        }
        if !(aspect > 0.0 && aspect.is_finite())
            || !is_greater(near_m, 0.0)
            || !is_greater(far_m, near_m)
        {
            return None;
        }
        let forward = scale(f, 1.0 / dist);
        // The arena's world up; a camera looking straight up falls back to +Z
        // so the basis stays defined.
        let hint = if forward[1].abs() > 0.999 {
            [0.0, 0.0, 1.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let right = normalize(cross3(forward, hint))?;
        let up = cross3(right, forward);
        let tan_y = (fov_y_rad * 0.5).tan();
        Some(Self {
            eye,
            right,
            up,
            forward,
            tan_x: tan_y * aspect,
            tan_y,
            near_m,
            far_m,
        })
    }

    /// The world-space ray direction through frame uv `(x, y)` in `0..=1`,
    /// row 0 at the top, unit length. `None` on a non-finite uv.
    #[must_use]
    pub fn ray(&self, u: f32, v: f32) -> Option<[f32; 3]> {
        if !(u.is_finite() && v.is_finite()) {
            return None;
        }
        let ndc_x = 2.0 * u - 1.0;
        let ndc_y = 1.0 - 2.0 * v;
        let (sx, sy) = (ndc_x * self.tan_x, ndc_y * self.tan_y);
        let dir = [
            self.forward[0] + self.right[0] * sx + self.up[0] * sy,
            self.forward[1] + self.right[1] * sx + self.up[1] * sy,
            self.forward[2] + self.right[2] * sx + self.up[2] * sy,
        ];
        normalize(dir)
    }

    /// Row-major `0.5 bias * projection * view`, the donor's
    /// `_updateTextureMatrix`: multiply a world position by it and divide by
    /// `w` for the mask or prefill texel uv in `0..=1`. Written `m[row][col]`.
    #[must_use]
    pub fn texture_matrix(&self) -> [[f32; 4]; 4] {
        mat_mul(&BIAS, &mat_mul(&self.projection(), &self.view_matrix()))
    }

    /// The uv a world position projects to, `None` at or behind the eye: the
    /// [`Self::texture_matrix`] applied, then divided by `w`. This is the
    /// prefill texel the mask fragment compares its own depth against. The
    /// donor matrix is bottom-up (`v = 0` at the bottom, the GL convention) and
    /// a wgpu texture's row 0 is the top, so `v` flips here, the twin of the
    /// shader's `1.0 - y / w`. A negative `w` is a point behind the eye, which
    /// mirrors through the centre of the frame and has no texel of its own.
    #[must_use]
    pub fn project_uv(&self, world: [f32; 3]) -> Option<[f32; 2]> {
        let clip = mat_vec(&self.texture_matrix(), [world[0], world[1], world[2], 1.0]);
        if !(clip[3] > 1e-6) {
            return None;
        }
        Some([clip[0] / clip[3], 1.0 - clip[1] / clip[3]])
    }

    /// Row-major perspective projection, `0..=1` clip depth (wgpu convention).
    #[must_use]
    fn projection(&self) -> [[f32; 4]; 4] {
        let fx = 1.0 / self.tan_x;
        let fy = 1.0 / self.tan_y;
        let n = self.near_m;
        let f = self.far_m;
        [
            [fx, 0.0, 0.0, 0.0],
            [0.0, fy, 0.0, 0.0],
            [0.0, 0.0, f / (n - f), n * f / (n - f)],
            [0.0, 0.0, -1.0, 0.0],
        ]
    }

    /// Row-major world-to-camera: rows are the basis, the camera looks `-Z`.
    #[must_use]
    fn view_matrix(&self) -> [[f32; 4]; 4] {
        let [r, u, f] = [self.right, self.up, self.forward];
        [
            [r[0], r[1], r[2], -dot(r, self.eye)],
            [u[0], u[1], u[2], -dot(u, self.eye)],
            [-f[0], -f[1], -f[2], dot(f, self.eye)],
            [0.0, 0.0, 0.0, 1.0],
        ]
    }
}

/// The donor's `0.5` bias: clip `-1..=1` to uv `0..=1`.
const BIAS: [[f32; 4]; 4] = [
    [0.5, 0.0, 0.0, 0.5],
    [0.0, 0.5, 0.0, 0.5],
    [0.0, 0.0, 0.5, 0.5],
    [0.0, 0.0, 0.0, 1.0],
];

/// Row-major `a * b`.
#[must_use]
fn mat_mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for (row, a_row) in a.iter().enumerate() {
        for (col, out_cell) in out[row].iter_mut().enumerate() {
            let mut sum = 0.0f32;
            for (k, a_val) in a_row.iter().enumerate() {
                sum += a_val * b[k][col];
            }
            *out_cell = sum;
        }
    }
    out
}

/// Row-major matrix times a column vector.
#[must_use]
fn mat_vec(m: &[[f32; 4]; 4], v: [f32; 4]) -> [f32; 4] {
    let mut out = [0.0f32; 4];
    for (row, row_vals) in m.iter().enumerate() {
        out[row] =
            row_vals[0] * v[0] + row_vals[1] * v[1] + row_vals[2] * v[2] + row_vals[3] * v[3];
    }
    out
}

/// The depth prefill the mask is compared against: one ray distance in metres
/// per texel, [`f32::INFINITY`] where nothing covers.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneDepth {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// Row-major distance along the texel's ray, metres.
    pub depth: Vec<f32>,
}

impl SceneDepth {
    /// An empty field, every texel [`f32::INFINITY`], at `width` by `height`.
    #[must_use]
    pub fn sky(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            depth: vec![f32::INFINITY; texels(width, height)],
        }
    }

    /// Depth at texel `(x, y)`, [`f32::INFINITY`] outside the field.
    #[must_use]
    pub fn at(&self, x: u32, y: u32) -> f32 {
        field_at(&self.depth, self.width, self.height, x, y, f32::INFINITY)
    }

    /// Prefill one axis-aligned box (the sand berm `ARENA_WALL_BOX`, a body):
    /// every texel whose ray enters the box keeps the box's entry distance when
    /// that is nearer than what is already there. The slab test here is what
    /// the depth prefill's rasterizer answers for that box.
    pub fn prefill_box(&mut self, view: &OutlineView, min: [f32; 3], max: [f32; 3]) {
        for y in 0..self.height {
            for x in 0..self.width {
                let Some(uv) = texel_center(self.width, self.height, x, y) else {
                    continue;
                };
                let Some(dir) = view.ray(uv[0], uv[1]) else {
                    continue;
                };
                let Some(t) = ray_box(view.eye, dir, min, max) else {
                    continue;
                };
                if let Some(slot) = self.depth.get_mut(texel_index(self.width, x, y)) {
                    if t < *slot {
                        *slot = t;
                    }
                }
            }
        }
    }
}

/// One selected target: its id (the mask pass draws by id) and the bounding
/// sphere the CPU oracle rasterizes in place of the mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutlineTarget {
    /// Fighter id, the selection key.
    pub id: u32,
    /// Bounding-sphere centre, world space, metres.
    pub center: [f32; 3],
    /// Bounding-sphere radius, metres.
    pub radius_m: f32,
}

/// The half-res mask: coverage (the white the mask pass writes) and the
/// visibility channel its depth test produces.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineMask {
    /// Width in texels, the plan's `mask_size[0]`.
    pub width: u32,
    /// Height in texels, the plan's `mask_size[1]`.
    pub height: u32,
    /// Distance of the selected surface per texel, [`f32::INFINITY`] where no
    /// target covers.
    pub depth: Vec<f32>,
    /// `1` where the target passed the depth test against the prefill (in
    /// front or level with it), `0` where the prefill covers it. Texels no
    /// target covers carry `1`: nothing is in front of nothing, so the outside
    /// of a visible outline stays visible.
    pub visible: Vec<f32>,
}

impl OutlineMask {
    /// Coverage at texel `(x, y)`: `1` where a selected target was drawn.
    #[must_use]
    pub fn coverage(&self, x: u32, y: u32) -> f32 {
        if self.depth_at(x, y).is_finite() {
            1.0
        } else {
            0.0
        }
    }

    /// Mask depth at texel `(x, y)`, [`f32::INFINITY`] outside.
    #[must_use]
    pub fn depth_at(&self, x: u32, y: u32) -> f32 {
        field_at(&self.depth, self.width, self.height, x, y, f32::INFINITY)
    }

    /// Visibility at texel `(x, y)`, `1` outside.
    #[must_use]
    pub fn visible_at(&self, x: u32, y: u32) -> f32 {
        field_at(&self.visible, self.width, self.height, x, y, 1.0)
    }
}

/// The mask pass over the CPU oracle: for every texel take the nearest selected
/// target its ray hits, then compare that distance against the prefill over the
/// frame texels the half-res texel covers (the nearest occluder wins), with
/// [`DEPTH_BIAS_M`] of slack. An empty selection writes no target, so the mask
/// has no coverage and the frame costs no draw.
#[must_use]
pub fn mask_pass(
    view: &OutlineView,
    scene: &SceneDepth,
    targets: &[OutlineTarget],
    mask_width: u32,
    mask_height: u32,
) -> OutlineMask {
    let mut mask = OutlineMask {
        width: mask_width,
        height: mask_height,
        depth: vec![f32::INFINITY; texels(mask_width, mask_height)],
        visible: vec![1.0; texels(mask_width, mask_height)],
    };
    if targets.is_empty() || mask_width == 0 || mask_height == 0 {
        return mask;
    }
    let step_x = (scene.width.div_ceil(mask_width)).max(1);
    let step_y = (scene.height.div_ceil(mask_height)).max(1);
    for y in 0..mask_height {
        for x in 0..mask_width {
            let Some(uv) = texel_center(mask_width, mask_height, x, y) else {
                continue;
            };
            let Some(dir) = view.ray(uv[0], uv[1]) else {
                continue;
            };
            let mut nearest = f32::INFINITY;
            for target in targets {
                if let Some(t) = ray_sphere(view.eye, dir, target.center, target.radius_m) {
                    nearest = nearest.min(t);
                }
            }
            if !nearest.is_finite() {
                continue;
            }
            let mut occluder = f32::INFINITY;
            for oy in 0..step_y {
                for ox in 0..step_x {
                    occluder = occluder.min(scene.at(x * step_x + ox, y * step_y + oy));
                }
            }
            let index = texel_index(mask_width, x, y);
            mask.depth[index] = nearest;
            mask.visible[index] = if nearest <= occluder + DEPTH_BIAS_M {
                1.0
            } else {
                0.0
            };
        }
    }
    mask
}

/// The edge field: one RGB colour per mask texel, the Sobel magnitude times the
/// visible or hidden edge colour. Never clamped; the composite saturates.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineEdge {
    /// Width in texels, the mask's.
    pub width: u32,
    /// Height in texels, the mask's.
    pub height: u32,
    /// Row-major colour, three channels.
    pub color: Vec<[f32; 3]>,
}

impl OutlineEdge {
    /// Colour at texel `(x, y)`, black outside.
    #[must_use]
    pub fn at(&self, x: u32, y: u32) -> [f32; 3] {
        if x >= self.width || y >= self.height {
            return [0.0; 3];
        }
        self.color
            .get(texel_index(self.width, x, y))
            .copied()
            .unwrap_or([0.0; 3])
    }

    /// The brightest texel of the field: its colour and position, `None` for a
    /// field with no edge at all (no selection, no coverage).
    #[must_use]
    pub fn peak(&self) -> Option<([f32; 3], u32, u32)> {
        let mut best: Option<([f32; 3], u32, u32, f32)> = None;
        for y in 0..self.height {
            for x in 0..self.width {
                let c = self.at(x, y);
                let lum = c[0].max(c[1]).max(c[2]);
                if lum > 0.0 && best.is_none_or(|(_, _, _, b)| lum > b) {
                    best = Some((c, x, y, lum));
                }
            }
        }
        best.map(|(c, x, y, _)| (c, x, y))
    }
}

/// The Sobel edge pass at the mask's own resolution: a 3x3 Sobel over the mask
/// coverage, magnitude `/ 4` so a hard coverage step reads `1`, in the visible
/// or the hidden colour picked by the visibility channel over the same 3x3
/// window. One neighbour behind the prefill makes the whole edge hidden, so the
/// outline shows through the berm. The strength multiplier is the composite's
/// (the donor's `edgeStrength`), not this pass's. A mask with no coverage gives
/// a black field, and the composite then changes no pixel.
#[must_use]
pub fn edge_pass(mask: &OutlineMask) -> OutlineEdge {
    let mut color = vec![[0.0f32; 3]; texels(mask.width, mask.height)];
    if mask.width < 3 || mask.height < 3 {
        return OutlineEdge {
            width: mask.width,
            height: mask.height,
            color,
        };
    }
    // Sobel x and y kernels, written out so the sign convention is explicit.
    const KX: [[f32; 3]; 3] = [[-1.0, 0.0, 1.0], [-2.0, 0.0, 2.0], [-1.0, 0.0, 1.0]];
    const KY: [[f32; 3]; 3] = [[-1.0, -2.0, -1.0], [0.0, 0.0, 0.0], [1.0, 2.0, 1.0]];
    for y in 0..mask.height {
        for x in 0..mask.width {
            let mut gx = 0.0f32;
            let mut gy = 0.0f32;
            for (row, kx_row) in KX.iter().enumerate() {
                for (col, kx) in kx_row.iter().enumerate() {
                    let offset = i64::try_from(row).unwrap_or(0);
                    let ix = i64::from(x) + i64::try_from(col).unwrap_or(0) - 1;
                    let iy = i64::from(y) + offset - 1;
                    let c = coverage_at(mask, ix, iy);
                    gx += kx * c;
                    gy += KY[row][col] * c;
                }
            }
            let magnitude = (gx * gx + gy * gy).sqrt() / 4.0;
            if magnitude <= 0.0 {
                continue;
            }
            // The donor tests the four central neighbours; this pass takes the
            // whole 3x3 window the gradient itself reads, so an edge that any
            // covered fragment contributes to reads hidden. On a round
            // silhouette the donor's four taps miss the diagonal neighbour the
            // gradient came from and paint a white speck over the berm.
            let mut visibility = 1.0f32;
            for row in -1i64..=1 {
                for col in -1i64..=1 {
                    let ix = clamp_index(mask.width, i64::from(x) + col);
                    let iy = clamp_index(mask.height, i64::from(y) + row);
                    visibility = visibility.min(mask.visible_at(ix, iy));
                }
            }
            let base = if visibility > 0.999 {
                VISIBLE_EDGE_COLOR
            } else {
                HIDDEN_EDGE_COLOR
            };
            color[texel_index(mask.width, x, y)] = base.map(|c| c * magnitude);
        }
    }
    OutlineEdge {
        width: mask.width,
        height: mask.height,
        color,
    }
}

/// A signed texel index clamped into `0..limit`, `0` on a zero limit: the
/// `ClampToEdge` addressing every tap of the pass uses, so the field's border
/// never reads as a coverage step.
#[must_use]
fn clamp_index(limit: u32, at: i64) -> u32 {
    let last = limit.saturating_sub(1);
    if at < 0 {
        return 0;
    }
    u32::try_from(at).unwrap_or(last).min(last)
}

/// Coverage at a signed texel index, clamped to the mask's edge.
#[must_use]
fn coverage_at(mask: &OutlineMask, x: i64, y: i64) -> f32 {
    mask.coverage(clamp_index(mask.width, x), clamp_index(mask.height, y))
}

/// Which axis a separable blur runs along.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OutlineAxis {
    /// Across the row, `x`.
    X,
    /// Down the column, `y`.
    Y,
}

/// The thickness kernel radius actually used: `wanted` rounded to whole texels
/// and clamped to `1..=max` (the donor's `MAX_EDGE_THICKNESS` and
/// `MAX_EDGE_GLOW` caps), so a `0` request still draws a hairline and a huge
/// one stays inside the cap.
#[must_use]
pub fn kernel_radius_px(wanted: f32, max: u32) -> u32 {
    if !wanted.is_finite() || wanted <= 0.0 || max == 0 {
        return 1;
    }
    let rounded = wanted.round();
    // The caps are 4 texels, so walking them keeps the count exact without a
    // float-to-int cast.
    let mut radius = 1u32;
    while radius < max {
        let next = f32::from(u16::try_from(radius + 1).unwrap_or(1));
        if next > rounded {
            break;
        }
        radius += 1;
    }
    radius
}

/// One separable Gaussian blur, the donor's `_getSeparableBlurMaterial`:
/// `sigma = radius / 2` and one tap per texel out to `radius` on both sides.
/// All three channels blur together, so the visible and hidden edges keep their
/// own hue and only their band spreads.
#[must_use]
#[expect(clippy::cast_precision_loss, reason = "taps <= 4")]
pub fn blur_separable(edge: &OutlineEdge, axis: OutlineAxis, radius_px: u32) -> OutlineEdge {
    let radius = radius_px.max(1);
    let sigma = radius as f32 * 0.5;
    let taps = i64::from(radius);
    let mut weights: Vec<f32> = (-taps..=taps)
        .map(|off| gaussian_pdf(off as f32, sigma))
        .collect();
    let weight_sum: f32 = weights.iter().sum();
    if !is_greater(weight_sum, 0.0) {
        return edge.clone();
    }
    for w in &mut weights {
        *w /= weight_sum;
    }
    let mut color = vec![[0.0f32; 3]; texels(edge.width, edge.height)];
    for y in 0..edge.height {
        for x in 0..edge.width {
            let mut sum = [0.0f32; 3];
            for (tap, w) in weights.iter().enumerate() {
                let off = tap as i64 - taps;
                let (tx, ty) = match axis {
                    OutlineAxis::X => (i64::from(x) + off, i64::from(y)),
                    OutlineAxis::Y => (i64::from(x), i64::from(y) + off),
                };
                let c = sample_edge(edge, tx, ty);
                for channel in 0..3 {
                    sum[channel] += c[channel] * w;
                }
            }
            color[texel_index(edge.width, x, y)] = sum;
        }
    }
    OutlineEdge {
        width: edge.width,
        height: edge.height,
        color,
    }
}

/// A Gaussian tap weight `x` texels from the centre at `sigma` texels.
#[must_use]
fn gaussian_pdf(x: f32, sigma: f32) -> f32 {
    0.398_94 * (-0.5 * x * x / (sigma * sigma)).exp() / sigma
}

/// Colour at a signed texel index, clamped to the field's edge.
#[must_use]
fn sample_edge(edge: &OutlineEdge, x: i64, y: i64) -> [f32; 3] {
    edge.at(clamp_index(edge.width, x), clamp_index(edge.height, y))
}

/// The frame's outline band, ready for the composite: the thickness blur at the
/// requested radius capped by [`MAX_EDGE_THICKNESS`], plus the wider glow blur
/// at [`MAX_EDGE_GLOW`].
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineBand {
    /// Thickness-blurred edge field.
    pub thickness: OutlineEdge,
    /// Glow-blurred edge field.
    pub glow: OutlineEdge,
}

/// Run the four separable passes over an edge field in the donor's order:
/// thickness X then Y at mask resolution, then glow X and Y at the wide kernel
/// over the thickness result. `glow = false` skips the pair and leaves the glow
/// field equal to the thickness one, so a tier can pay two passes instead of
/// four.
#[must_use]
pub fn outline_band(edge: &OutlineEdge, edge_thickness: f32, glow: bool) -> OutlineBand {
    let radius = kernel_radius_px(edge_thickness, MAX_EDGE_THICKNESS);
    let along_x = blur_separable(edge, OutlineAxis::X, radius);
    let thickness = blur_separable(&along_x, OutlineAxis::Y, radius);
    if !glow {
        return OutlineBand {
            thickness: thickness.clone(),
            glow: thickness,
        };
    }
    let glow_x = blur_separable(&thickness, OutlineAxis::X, MAX_EDGE_GLOW);
    let glow = blur_separable(&glow_x, OutlineAxis::Y, MAX_EDGE_GLOW);
    OutlineBand { thickness, glow }
}

/// The additive composite, the donor's `_getOverlayMaterial` without the
/// pattern texture: `edge_strength * pulse * (thickness + glow * glow_scale)`
/// added to the frame colour and saturated at `1` per channel. `pulse` is
/// [`pulse_scale`], the donor's `edgeGlow` breathing.
#[must_use]
pub fn composite(
    frame: [f32; 3],
    band: &OutlineBand,
    x: u32,
    y: u32,
    edge_strength: f32,
    glow_scale: f32,
    pulse: f32,
) -> [f32; 3] {
    let t = band.thickness.at(x, y);
    let g = band.glow.at(x, y);
    let mut out = frame;
    for (channel, value) in out.iter_mut().enumerate() {
        let added = edge_strength * (t[channel] + g[channel] * glow_scale) * pulse;
        *value = (frame[channel] + added).clamp(0.0, 1.0);
    }
    out
}

/// The donor's `pulsePeriod` cosine: `1` at `t = 0`, `0.25` at half the period
/// and back to `1` at the period. `period_s <= 0` or a non-finite time is a
/// steady `1`, so the outline never blinks because of a bad clock.
#[must_use]
pub fn pulse_scale(period_s: f32, t_s: f32) -> f32 {
    if !is_greater(period_s, 0.0) || !t_s.is_finite() {
        return 1.0;
    }
    0.625 + 0.375 * (core::f32::consts::TAU * t_s / period_s).cos()
}

/// Texel count of a `width` by `height` field; `0` where a side cannot fit a
/// `usize`, which the callers never pass.
#[must_use]
fn texels(width: u32, height: u32) -> usize {
    usize::try_from(width).unwrap_or(0) * usize::try_from(height).unwrap_or(0)
}

/// One value of a row-major field at texel `(x, y)`, `fallback` outside. The
/// mask and the depth prefill are both this shape, so they share the read.
#[must_use]
fn field_at(values: &[f32], width: u32, height: u32, x: u32, y: u32, fallback: f32) -> f32 {
    if x >= width || y >= height {
        return fallback;
    }
    values
        .get(texel_index(width, x, y))
        .copied()
        .unwrap_or(fallback)
}

/// Row-major index of texel `(x, y)`.
#[must_use]
fn texel_index(width: u32, x: u32, y: u32) -> usize {
    usize::try_from(y)
        .unwrap_or(0)
        .saturating_mul(usize::try_from(width).unwrap_or(0))
        .saturating_add(usize::try_from(x).unwrap_or(0))
}

/// The uv of a texel's centre in `0..=1`, row 0 at the top; `None` on a zero
/// side.
#[must_use]
#[expect(clippy::cast_precision_loss, reason = "small texel coords")]
fn texel_center(width: u32, height: u32, x: u32, y: u32) -> Option<[f32; 2]> {
    if width == 0 || height == 0 {
        return None;
    }
    let u = (x as f32 + 0.5) / width as f32;
    let v = (y as f32 + 0.5) / height as f32;
    Some([u, v])
}

/// Nearest ray-sphere hit distance past the eye, metres; `None` on a miss.
/// `dir` must be unit length.
#[must_use]
fn ray_sphere(origin: [f32; 3], dir: [f32; 3], center: [f32; 3], radius: f32) -> Option<f32> {
    if !is_greater(radius, 0.0) {
        return None;
    }
    let oc = sub(origin, center);
    let b = dot(oc, dir);
    let c = dot(oc, oc) - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let near = -b - root;
    let far = -b + root;
    if near > 0.0 {
        Some(near)
    } else if far > 0.0 {
        Some(far)
    } else {
        None
    }
}

/// Ray against an axis-aligned box (slab test): the entry distance in metres,
/// or `None` when the ray misses or the whole box is behind the eye.
#[must_use]
fn ray_box(origin: [f32; 3], dir: [f32; 3], min: [f32; 3], max: [f32; 3]) -> Option<f32> {
    let mut t_near = 0.0f32;
    let mut t_far = f32::INFINITY;
    for axis in 0..3 {
        let d = dir[axis];
        let o = origin[axis];
        if d.abs() < 1e-9 {
            if o < min[axis] || o > max[axis] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / d;
        let mut t0 = (min[axis] - o) * inv;
        let mut t1 = (max[axis] - o) * inv;
        if t0 > t1 {
            core::mem::swap(&mut t0, &mut t1);
        }
        t_near = t_near.max(t0);
        t_far = t_far.min(t1);
        if t_near > t_far {
            return None;
        }
    }
    if t_far <= 0.0 {
        return None;
    }
    Some(t_near)
}

/// Unit vector, `None` on a zero-length input.
fn normalize(a: [f32; 3]) -> Option<[f32; 3]> {
    let l = len(a);
    if l > 1e-9 {
        Some(scale(a, 1.0 / l))
    } else {
        None
    }
}

#[cfg(test)]
#[path = "outline_tests.rs"]
mod tests;
