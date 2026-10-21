//! spot: shadowed spot lights (flashlights and lamps) for the clustered path.
//!
//! Owner: render lane; consumers are the live window frame and the frame
//! report. Units: meters for positions and ranges, texels for the shadow
//! atlas, pixels for screen footprints, milliseconds in receipts.
//! Determinism: fixed slot order, one raster rule, no clock in the pixels;
//! the timer appears in receipts only. Presentation only: nothing here feeds
//! the tick or the hash.
//!
//! Shape: up to [`MAX_SPOTS`] spot lights share one square depth atlas of
//! [`ATLAS_GRID`] by [`ATLAS_GRID`] tiles, one tile per slot in slot order.
//! A spot joins the clustered path through [`SpotLight::screen_light`],
//! which returns the same [`ScreenLight`] the clustered point lights already
//! use, so a flashlight rides the grid the torches fill. The CPU reference
//! [`SpotAtlas::bake`] is the parity oracle for the device bake
//! `submit_spot_atlas` (behind `window`): both rasterize the same accepted
//! triangle list through the same [`spot_clip_matrix`], and the gate's
//! readback arm `atlas_parity` holds a software adapter to
//! [`PARITY_AGREEMENT`] at [`PARITY_TOLERANCE`].
//!
//! Two device entries share that bake: `submit_spot_atlas` encodes, submits
//! and waits on the device (the budget arm: one submit against
//! [`SPOT_PASS_BUDGET_MS`]), and `encode_spot_atlas` records the same pass
//! into a caller-supplied command encoder without submitting it (the live
//! window frame's entry: the frame's own one command buffer carries the
//! atlas, so the frame never waits on the GPU).
//!
//! Bounds: a triangle with any vertex at or behind the spot near plane is
//! dropped by both paths (the room fixture keeps every occluder in front);
//! more spots than atlas slots and more triangles than [`MAX_OCCLUDER_TRIS`]
//! degrade with a recorded clamp. Bad input fails loudly with the lane's
//! [`LightError`].

#![forbid(unsafe_code)]

use crate::shadow_cluster::{LightError, ScreenLight};

/// The encode-only arm of the bake: the live window frame records the spot
/// pass in its own command buffer, so the frame never waits on the GPU.
#[cfg(feature = "window")]
mod encode;

#[cfg(feature = "window")]
pub use encode::{encode_spot_atlas, SpotEncode};

/// Most shadowed spots in one frame; the atlas has one tile per spot.
pub const MAX_SPOTS: usize = 16;
/// Atlas tiles per side; [`MAX_SPOTS`] tiles fill it exactly.
pub const ATLAS_GRID: usize = 4;
/// Smallest accepted atlas tile edge in texels.
pub const ATLAS_TILE_MIN: usize = 32;
/// Largest accepted atlas tile edge in texels.
pub const ATLAS_TILE_MAX: usize = 512;
/// Most occluder triangles accepted per frame; the tail degrades.
pub const MAX_OCCLUDER_TRIS: usize = 512;
/// Spot near plane in meters; geometry closer than this is dropped.
pub const SPOT_NEAR_M: f32 = 0.05;
/// Depth disagreement that counts a texel as mismatched (normalized depth).
pub const PARITY_TOLERANCE: f32 = 1.0e-4;
/// Share of atlas texels the device bake must match the reference on.
pub const PARITY_AGREEMENT: f32 = 0.99;
/// Posted budget for one atlas submit in ms: the spot share of the 14 ms
/// frame at Base. Evidence only, never hashed.
pub const SPOT_PASS_BUDGET_MS: f64 = 2.0;
/// Registry row this pass proves, for the live-frame gate.
pub const GATE_ROW: &str = "spot";
/// Submit entry the live-frame gate watches.
pub const GATE_PASS: &str = "submit_spot_atlas";

// ---------------------------------------------------------------------------
// Spot lights: one cone in world space with a bounded influence range.
// ---------------------------------------------------------------------------

/// One spot light: a flashlight or a lamp cone, in world space.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpotLight {
    /// Aperture position in meters.
    pub pos: [f32; 3],
    /// Emission direction; need not be unit length, must be nonzero.
    pub dir: [f32; 3],
    /// Cosine of the inner (full brightness) half angle, `>= outer_cos`.
    pub inner_cos: f32,
    /// Cosine of the outer (zero) half angle, `(0, 1)`; the shadow cone ends
    /// here.
    pub outer_cos: f32,
    /// Influence range in meters, `> 0`.
    pub range_m: f32,
    /// Linear RGB color, components `>= 0`.
    pub color: [f32; 3],
    /// Brightness scale, `>= 0`.
    pub intensity: f32,
    /// True when this spot bakes a shadow tile.
    pub cast_shadow: bool,
}

impl SpotLight {
    /// Validate finiteness, the cone order, the range and the color.
    pub fn validate(&self) -> Result<(), LightError> {
        if !self.pos.iter().all(|c| c.is_finite()) {
            return Err(LightError::Invalid("spot position is not finite".into()));
        }
        let dir = self.unit_dir()?;
        if !dir.iter().all(|c| c.is_finite()) {
            return Err(LightError::Invalid("spot direction is not finite".into()));
        }
        if !self.inner_cos.is_finite() || !(0.0..=1.0).contains(&self.inner_cos) {
            return Err(LightError::Invalid(format!(
                "spot inner cosine {} outside 0..=1",
                self.inner_cos
            )));
        }
        if !self.outer_cos.is_finite() || self.outer_cos <= 0.0 || self.outer_cos > 1.0 {
            return Err(LightError::Invalid(format!(
                "spot outer cosine {} outside (0, 1]",
                self.outer_cos
            )));
        }
        if self.inner_cos < self.outer_cos {
            return Err(LightError::Invalid(format!(
                "spot inner cosine {} under outer cosine {}",
                self.inner_cos, self.outer_cos
            )));
        }
        if !self.range_m.is_finite() || self.range_m <= 0.0 {
            return Err(LightError::Invalid(format!(
                "spot range {} is not positive finite",
                self.range_m
            )));
        }
        if !self.color.iter().all(|c| c.is_finite() && *c >= 0.0) {
            return Err(LightError::Invalid(
                "spot color is not finite non-negative".into(),
            ));
        }
        if !self.intensity.is_finite() || self.intensity < 0.0 {
            return Err(LightError::Invalid(format!(
                "spot intensity {} is not finite non-negative",
                self.intensity
            )));
        }
        Ok(())
    }

    /// Unit emission direction; a zero vector is a typed refusal.
    pub fn unit_dir(&self) -> Result<[f32; 3], LightError> {
        let len2 = dot(self.dir, self.dir);
        if !len2.is_finite() || len2 <= 0.0 {
            return Err(LightError::Invalid(
                "spot direction is zero or not finite".into(),
            ));
        }
        let len = len2.sqrt();
        Ok([self.dir[0] / len, self.dir[1] / len, self.dir[2] / len])
    }

    /// Tangent of the outer half angle: the shadow cone covers exactly the
    /// lit cone, so the atlas tile spends no texel on dark space.
    pub fn shadow_tan_half(&self) -> Result<f32, LightError> {
        let c = self.outer_cos;
        if !c.is_finite() || c <= 0.0 || c > 1.0 {
            return Err(LightError::Invalid(format!(
                "spot outer cosine {c} cannot give a shadow cone"
            )));
        }
        let s2 = (1.0 - c * c).max(0.0);
        Ok(s2.sqrt() / c)
    }

    /// Cone falloff at `cos_angle`: 1 inside the inner cone, 0 outside the
    /// outer one, a smooth step between. The one owner of this curve.
    pub fn cone_factor(&self, cos_angle: f32) -> f32 {
        let span = self.inner_cos - self.outer_cos;
        if span <= 0.0 {
            return if cos_angle >= self.inner_cos {
                1.0
            } else {
                0.0
            };
        }
        let t = ((cos_angle - self.outer_cos) / span).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// Distance falloff at `dist_m`: zero at and past the range, one at the
    /// aperture, a squared window between.
    pub fn distance_factor(&self, dist_m: f32) -> f32 {
        if !dist_m.is_finite() || dist_m < 0.0 || dist_m >= self.range_m {
            return 0.0;
        }
        let t = (dist_m / self.range_m).clamp(0.0, 1.0);
        let t2 = t * t;
        let window = 1.0 - t2 * t2;
        window * window / (dist_m * dist_m + 1.0)
    }

    /// Screen footprint for the clustered path: the same [`ScreenLight`] the
    /// clustered point lights use. `None` when the aperture is behind the
    /// view near plane or past its far plane.
    pub fn screen_light(&self, view: &SpotView) -> Result<Option<ScreenLight>, LightError> {
        view.validate()?;
        let Some(center) = view.project(self.pos) else {
            return Ok(None);
        };
        let world_radius = self.range_m * SPOT_FOOTPRINT_FRAC;
        let focal_px = view.height_px * 0.5 / view.tan_half;
        let radius_px = world_radius / center[2] * focal_px;
        let screen = ScreenLight {
            x_px: center[0],
            y_px: center[1],
            depth_m: center[2],
            radius_px,
        };
        screen
            .validate()
            .map_err(|e| LightError::Invalid(format!("spot footprint: {e}")))?;
        Ok(Some(screen))
    }
}

/// Share of a spot's range used as its screen-space footprint radius: the
/// full cone at the range is wide, the lit pool is not.
pub const SPOT_FOOTPRINT_FRAC: f32 = 0.35;

/// View basis for projecting spots into the clustered screen grid: camera
/// eye, orthonormal basis, focal tangent, frame size in pixels, near and far.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpotView {
    /// Camera eye in meters.
    pub eye: [f32; 3],
    /// Camera right axis, unit length.
    pub right: [f32; 3],
    /// Camera up axis, unit length.
    pub up: [f32; 3],
    /// Camera forward axis, unit length.
    pub forward: [f32; 3],
    /// Tangent of half the vertical field of view, `> 0`.
    pub tan_half: f32,
    /// Frame width in pixels, a whole positive number.
    pub width_px: f32,
    /// Frame height in pixels, a whole positive number.
    pub height_px: f32,
    /// View near plane in meters, `> 0`.
    pub near_m: f32,
    /// View far plane in meters, `> near_m`.
    pub far_m: f32,
}

impl SpotView {
    /// Validate the basis, the frame size and the depth range.
    pub fn validate(&self) -> Result<(), LightError> {
        for (name, axis) in [
            ("eye", self.eye),
            ("right", self.right),
            ("up", self.up),
            ("forward", self.forward),
        ] {
            if !axis.iter().all(|c| c.is_finite()) {
                return Err(LightError::Invalid(format!(
                    "spot view {name} is not finite"
                )));
            }
        }
        for (name, axis) in [
            ("right", self.right),
            ("up", self.up),
            ("forward", self.forward),
        ] {
            if dot(axis, axis) <= 0.0 {
                return Err(LightError::Invalid(format!("spot view {name} is zero")));
            }
        }
        if !self.tan_half.is_finite() || self.tan_half <= 0.0 {
            return Err(LightError::Invalid(format!(
                "spot view tangent {} is not positive finite",
                self.tan_half
            )));
        }
        for (name, px) in [("width", self.width_px), ("height", self.height_px)] {
            if !px.is_finite() || px < 1.0 || px.fract() != 0.0 {
                return Err(LightError::Invalid(format!(
                    "spot view {name} {px} is not a whole positive pixel count"
                )));
            }
        }
        if !self.near_m.is_finite() || !self.far_m.is_finite() {
            return Err(LightError::Invalid("spot view range is not finite".into()));
        }
        if self.near_m <= 0.0 || self.far_m <= self.near_m {
            return Err(LightError::Invalid(format!(
                "spot view range {}..{} is not 0 < near < far",
                self.near_m, self.far_m
            )));
        }
        Ok(())
    }

    /// Project a world point to `[x_px, y_px, view depth]`. `None` behind the
    /// near plane or past the far plane.
    pub fn project(&self, point: [f32; 3]) -> Option<[f32; 3]> {
        let v = [
            point[0] - self.eye[0],
            point[1] - self.eye[1],
            point[2] - self.eye[2],
        ];
        let depth = dot(self.forward, v);
        if !depth.is_finite() || depth <= self.near_m || depth >= self.far_m {
            return None;
        }
        let ndc_x = dot(self.right, v) / (depth * self.tan_half);
        let ndc_y = dot(self.up, v) / (depth * self.tan_half);
        let x_px = (ndc_x * 0.5 + 0.5) * self.width_px;
        let y_px = (0.5 - ndc_y * 0.5) * self.height_px;
        Some([x_px, y_px, depth])
    }
}

// ---------------------------------------------------------------------------
// Atlas shape: tiles in one square depth texture, slot order fixed.
// ---------------------------------------------------------------------------

/// Square shadow atlas shape: tiles per side and tile edge in texels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SpotAtlasConfig {
    /// Tiles per side, `1..=ATLAS_GRID`.
    pub grid: usize,
    /// Tile edge in texels, `ATLAS_TILE_MIN..=ATLAS_TILE_MAX`.
    pub tile_px: usize,
}

impl SpotAtlasConfig {
    /// Validate the grid and the tile edge before any allocation.
    pub fn validate(&self) -> Result<(), LightError> {
        if self.grid == 0 || self.grid > ATLAS_GRID {
            return Err(LightError::Invalid(format!(
                "spot atlas grid {} outside 1..={ATLAS_GRID}",
                self.grid
            )));
        }
        if !(ATLAS_TILE_MIN..=ATLAS_TILE_MAX).contains(&self.tile_px) {
            return Err(LightError::Invalid(format!(
                "spot atlas tile {} outside {ATLAS_TILE_MIN}..={ATLAS_TILE_MAX}",
                self.tile_px
            )));
        }
        Ok(())
    }

    /// Clamp an over-budget shape to the bounds; returns the degrade signal
    /// when either number moved, so the receipt records the clamp. The
    /// recorded amounts are atlas texels, so a smaller grid and a smaller
    /// tile are told apart by one comparable number.
    pub fn degrade(grid: usize, tile_px: usize) -> (Self, Option<LightError>) {
        let clamped_grid = grid.clamp(1, ATLAS_GRID);
        let clamped_tile = tile_px.clamp(ATLAS_TILE_MIN, ATLAS_TILE_MAX);
        let config = Self {
            grid: clamped_grid,
            tile_px: clamped_tile,
        };
        if clamped_grid == grid && clamped_tile == tile_px {
            return (config, None);
        }
        let requested = grid
            .saturating_mul(grid)
            .saturating_mul(tile_px)
            .saturating_mul(tile_px);
        let allowed = clamped_grid
            .saturating_mul(clamped_grid)
            .saturating_mul(clamped_tile)
            .saturating_mul(clamped_tile);
        (
            config,
            Some(LightError::BudgetExceeded {
                requested: u32::try_from(requested).unwrap_or(u32::MAX),
                allowed: u32::try_from(allowed).unwrap_or(u32::MAX),
            }),
        )
    }

    /// Shadow slots in this atlas: tiles per side squared.
    pub fn slots(&self) -> usize {
        self.grid * self.grid
    }

    /// Atlas edge in texels.
    pub fn atlas_px(&self) -> usize {
        self.grid * self.tile_px
    }

    /// Atlas texels in the depth tile.
    pub fn texels(&self) -> usize {
        let edge = self.atlas_px();
        edge * edge
    }

    /// Top-left texel of `slot` in slot order, or `None` past the slots.
    pub fn slot_origin(&self, slot: usize) -> Option<[usize; 2]> {
        if slot >= self.slots() {
            return None;
        }
        Some([
            (slot % self.grid) * self.tile_px,
            (slot / self.grid) * self.tile_px,
        ])
    }
}

// ---------------------------------------------------------------------------
// Occluders: plain world triangles. The room fixture is built from boxes.
// ---------------------------------------------------------------------------

/// One occluder triangle in world space, meters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct OccluderTri {
    /// First corner.
    pub a: [f32; 3],
    /// Second corner.
    pub b: [f32; 3],
    /// Third corner.
    pub c: [f32; 3],
}

/// Accept `tri` for `clip` only when every corner is in front of the near
/// plane; both bake paths call this so they rasterize the same list.
fn tri_in_front(tri: &OccluderTri, clip: &[f32; 16], near_m: f32) -> bool {
    [tri.a, tri.b, tri.c]
        .iter()
        .all(|p| mul_clip(clip, *p)[3] > near_m)
}

/// Accepted vertices for every slot, concatenated in slot order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct SlotRun {
    /// First vertex of this slot in the shared scratch.
    first_vertex: usize,
    /// Vertices in this slot: three per accepted triangle.
    vertex_count: usize,
}

/// Fill `verts` with the triangles both paths accept, slot by slot, and
/// record each slot's run. Returns the number of dropped triangles.
fn build_slot_geometry(
    verts: &mut Vec<[f32; 3]>,
    runs: &mut Vec<SlotRun>,
    clips: &[[f32; 16]],
    tris: &[OccluderTri],
    near_m: f32,
) -> u32 {
    verts.clear();
    runs.clear();
    let mut dropped = 0u32;
    for clip in clips {
        let first = verts.len();
        for tri in tris {
            if tri_in_front(tri, clip, near_m) {
                verts.push(tri.a);
                verts.push(tri.b);
                verts.push(tri.c);
            } else {
                dropped += 1;
            }
        }
        runs.push(SlotRun {
            first_vertex: first,
            vertex_count: verts.len() - first,
        });
    }
    dropped
}

/// A sealed room with two pillars: floor, ceiling, four walls, two boxes.
/// Every triangle sits well inside the spot cones the fixture lights use.
pub fn dark_room() -> Vec<OccluderTri> {
    let mut tris = Vec::with_capacity(36);
    // Floor at y=0 and ceiling at y=3, 10 x 10 meters in x and z.
    push_quad(
        &mut tris,
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        [10.0, 0.0, 10.0],
        [0.0, 0.0, 10.0],
    );
    push_quad(
        &mut tris,
        [0.0, 3.0, 0.0],
        [0.0, 3.0, 10.0],
        [10.0, 3.0, 10.0],
        [10.0, 3.0, 0.0],
    );
    push_quad(
        &mut tris,
        [0.0, 0.0, 0.0],
        [0.0, 3.0, 0.0],
        [10.0, 3.0, 0.0],
        [10.0, 0.0, 0.0],
    );
    push_quad(
        &mut tris,
        [10.0, 0.0, 0.0],
        [10.0, 3.0, 0.0],
        [10.0, 3.0, 10.0],
        [10.0, 0.0, 10.0],
    );
    push_quad(
        &mut tris,
        [10.0, 0.0, 10.0],
        [10.0, 3.0, 10.0],
        [0.0, 3.0, 10.0],
        [0.0, 0.0, 10.0],
    );
    push_quad(
        &mut tris,
        [0.0, 0.0, 10.0],
        [0.0, 3.0, 10.0],
        [0.0, 3.0, 0.0],
        [0.0, 0.0, 0.0],
    );
    push_box(&mut tris, [4.5, 1.5, 4.5], [0.45, 1.5, 0.45]);
    push_box(&mut tris, [6.5, 1.5, 6.5], [0.45, 1.5, 0.45]);
    tris
}

/// The fixture's sixteen shadowed spots: four lamps on each wall, aimed
/// down and inward at the floor. Deterministic order, no clock.
pub fn dark_room_spots() -> Vec<SpotLight> {
    let mut spots = Vec::with_capacity(MAX_SPOTS);
    for i in 0..4 {
        let t = 1.5 + 2.0 * f32_from_small(i);
        let mut south = lamp([t, 2.4, 0.35], [0.0, -0.45, 1.0]);
        south.color = [1.0, 0.86, 0.62];
        spots.push(south);
        let mut north = lamp([t, 2.4, 9.65], [0.0, -0.45, -1.0]);
        north.color = [1.0, 0.86, 0.62];
        spots.push(north);
        let mut west = lamp([0.35, 2.4, t], [1.0, -0.45, 0.0]);
        west.color = [1.0, 0.86, 0.62];
        spots.push(west);
        let mut east = lamp([9.65, 2.4, t], [-1.0, -0.45, 0.0]);
        east.color = [1.0, 0.86, 0.62];
        spots.push(east);
    }
    spots
}

/// One fixture lamp: warm, shadowed, a 30 degree outer cone.
fn lamp(pos: [f32; 3], dir: [f32; 3]) -> SpotLight {
    SpotLight {
        pos,
        dir,
        inner_cos: 0.94,
        outer_cos: 0.866,
        range_m: 7.0,
        color: [1.0, 0.86, 0.62],
        intensity: 6.0,
        cast_shadow: true,
    }
}

/// Small whole numbers as `f32` without a numeric cast.
fn f32_from_small(v: usize) -> f32 {
    f32::from(u8::try_from(v).unwrap_or(u8::MAX))
}

/// Two triangles for a planar quad, corners in order.
fn push_quad(tris: &mut Vec<OccluderTri>, a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) {
    tris.push(OccluderTri { a, b, c });
    tris.push(OccluderTri { a, b: c, c: d });
}

/// Twelve triangles for an axis-aligned box.
fn push_box(tris: &mut Vec<OccluderTri>, center: [f32; 3], half: [f32; 3]) {
    let lo = [
        center[0] - half[0],
        center[1] - half[1],
        center[2] - half[2],
    ];
    let hi = [
        center[0] + half[0],
        center[1] + half[1],
        center[2] + half[2],
    ];
    let (x0, x1) = (lo[0], hi[0]);
    let (y0, y1) = (lo[1], hi[1]);
    let (z0, z1) = (lo[2], hi[2]);
    push_quad(tris, [x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]);
    push_quad(tris, [x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]);
    push_quad(tris, [x0, y0, z0], [x0, y1, z0], [x1, y1, z0], [x1, y0, z0]);
    push_quad(tris, [x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]);
    push_quad(tris, [x1, y0, z1], [x1, y1, z1], [x0, y1, z1], [x0, y0, z1]);
    push_quad(tris, [x0, y0, z1], [x0, y1, z1], [x0, y1, z0], [x0, y0, z0]);
}

// ---------------------------------------------------------------------------
// Projection: one clip matrix serves the GPU uniform and the CPU reference.
// ---------------------------------------------------------------------------

/// Light-space clip matrix for one spot: column-major 4x4, wgpu depth 0..1.
/// The up hint falls back to +x when the direction is near vertical.
pub fn spot_clip_matrix(
    pos: [f32; 3],
    dir: [f32; 3],
    tan_half: f32,
    near_m: f32,
    far_m: f32,
) -> Result<[f32; 16], LightError> {
    if !pos.iter().all(|c| c.is_finite()) || !dir.iter().all(|c| c.is_finite()) {
        return Err(LightError::Invalid(
            "spot matrix input is not finite".into(),
        ));
    }
    if !tan_half.is_finite() || tan_half <= 0.0 {
        return Err(LightError::Invalid(format!(
            "spot matrix tangent {tan_half} is not positive finite"
        )));
    }
    if !near_m.is_finite() || !far_m.is_finite() || near_m <= 0.0 || far_m <= near_m {
        return Err(LightError::Invalid(format!(
            "spot matrix range {near_m}..{far_m} is not 0 < near < far"
        )));
    }
    let forward = unit(dir)?;
    let hint = if dot(forward, [0.0, 1.0, 0.0]).abs() > 0.99 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let right = unit(engine_core::vec3::cross3(hint, forward))?;
    let up = engine_core::vec3::cross3(forward, right);
    let sx = 1.0 / tan_half;
    let k = far_m / (far_m - near_m);
    let d = -far_m * near_m / (far_m - near_m);
    let mut m = [0.0f32; 16];
    // Row 0: right axis scaled by the focal tangent.
    m[0] = right[0] * sx;
    m[4] = right[1] * sx;
    m[8] = right[2] * sx;
    m[12] = -dot(right, pos) * sx;
    // Row 1: up axis.
    m[1] = up[0] * sx;
    m[5] = up[1] * sx;
    m[9] = up[2] * sx;
    m[13] = -dot(up, pos) * sx;
    // Row 2: depth mapped to 0..1 over the range.
    m[2] = forward[0] * k;
    m[6] = forward[1] * k;
    m[10] = forward[2] * k;
    m[14] = -dot(forward, pos) * k + d;
    // Row 3: clip w is view depth.
    m[3] = forward[0];
    m[7] = forward[1];
    m[11] = forward[2];
    m[15] = -dot(forward, pos);
    Ok(m)
}

/// Column-major matrix times a point.
fn mul_clip(m: &[f32; 16], p: [f32; 3]) -> [f32; 4] {
    [
        m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
        m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
        m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
        m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15],
    ]
}

/// Dot product of two 3-vectors.
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Unit vector; zero and non-finite input is a typed refusal.
fn unit(v: [f32; 3]) -> Result<[f32; 3], LightError> {
    let len2 = dot(v, v);
    if !len2.is_finite() || len2 <= 0.0 {
        return Err(LightError::Invalid("zero or non-finite axis".into()));
    }
    let len = len2.sqrt();
    Ok([v[0] / len, v[1] / len, v[2] / len])
}

// ---------------------------------------------------------------------------
// CPU reference: the parity oracle. Deterministic depth raster in atlas space.
// ---------------------------------------------------------------------------

/// One bake's receipt: slots used, texels touched, triangles dropped.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AtlasReceipt {
    /// Spots submitted.
    pub spots: usize,
    /// Spots that baked a shadow tile.
    pub shadowed: usize,
    /// Atlas texels in the tile.
    pub texels: usize,
    /// Texels a triangle wrote.
    pub written: u32,
    /// Triangles dropped for crossing a spot near plane.
    pub dropped_tris: u32,
    /// True when a budget clamp fired before the bake.
    pub degraded: bool,
}

/// CPU shadow atlas: normalized depth per texel (0 near, 1 far/unwritten).
#[derive(Clone, PartialEq, Debug)]
pub struct SpotAtlas {
    config: SpotAtlasConfig,
    depth: Vec<f32>,
    verts: Vec<[f32; 3]>,
    runs: Vec<SlotRun>,
    clips: Vec<[f32; 16]>,
    degraded: bool,
}

impl SpotAtlas {
    /// Allocate the depth tile plus the reused geometry scratch. The scratch
    /// grows once and never in a steady frame.
    pub fn new(config: SpotAtlasConfig) -> Result<Self, LightError> {
        config.validate()?;
        Ok(Self {
            config,
            depth: vec![1.0; config.texels()],
            verts: Vec::new(),
            runs: Vec::new(),
            clips: Vec::new(),
            degraded: false,
        })
    }

    /// Atlas shape this reference holds.
    pub fn config(&self) -> SpotAtlasConfig {
        self.config
    }

    /// Normalized depth at one atlas texel; `None` outside the atlas.
    pub fn depth_at(&self, x_px: usize, y_px: usize) -> Option<f32> {
        let edge = self.config.atlas_px();
        if x_px >= edge || y_px >= edge {
            return None;
        }
        self.depth.get(y_px * edge + x_px).copied()
    }

    /// Bake every shadowing spot into its slot. Over-budget input degrades
    /// (tail dropped, clamp recorded) instead of failing the frame.
    pub fn bake(
        &mut self,
        spots: &[SpotLight],
        tris: &[OccluderTri],
    ) -> Result<AtlasReceipt, LightError> {
        self.depth.fill(1.0);
        self.clips.clear();
        self.degraded = false;
        let slots = self.config.slots();
        let mut shadowed = 0usize;
        for spot in spots.iter().take(slots) {
            spot.validate()?;
            if !spot.cast_shadow {
                self.clips.push([0.0f32; 16]);
                continue;
            }
            let clip = spot_clip_matrix(
                spot.pos,
                spot.dir,
                spot.shadow_tan_half()?,
                SPOT_NEAR_M,
                spot.range_m,
            )?;
            self.clips.push(clip);
            shadowed += 1;
        }
        if spots.len() > slots || tris.len() > MAX_OCCLUDER_TRIS {
            self.degraded = true;
        }
        let tris = &tris[..tris.len().min(MAX_OCCLUDER_TRIS)];
        let dropped = build_slot_geometry(
            &mut self.verts,
            &mut self.runs,
            &self.clips,
            tris,
            SPOT_NEAR_M,
        );
        let written = self.raster_slots()?;
        Ok(AtlasReceipt {
            spots: spots.len(),
            shadowed,
            texels: self.config.texels(),
            written,
            dropped_tris: dropped,
            degraded: self.degraded,
        })
    }

    /// Rasterize each slot's run into its tile; returns texels written.
    fn raster_slots(&mut self) -> Result<u32, LightError> {
        let tile = self.config.tile_px;
        let edge = self.config.atlas_px();
        let mut written = 0u32;
        for (slot, run) in self.runs.iter().enumerate() {
            let Some(origin) = self.config.slot_origin(slot) else {
                break;
            };
            let Some(clip) = self.clips.get(slot) else {
                break;
            };
            if *clip == [0.0f32; 16] {
                continue;
            }
            let verts = &self.verts[run.first_vertex..run.first_vertex + run.vertex_count];
            written += raster_run(&mut self.depth, edge, tile, origin, verts, clip);
        }
        Ok(written)
    }

    /// Shadow factor at a world point for `slot`: 1 lit, 0 shadowed. One tap
    /// with `bias` in normalized depth; points off the tile read lit.
    pub fn shadow_factor(&self, slot: usize, clip: &[f32; 16], point: [f32; 3], bias: f32) -> f32 {
        let clip_p = mul_clip(clip, point);
        let w = clip_p[3];
        if !w.is_finite() || w <= SPOT_NEAR_M {
            return 1.0;
        }
        let ndc_x = clip_p[0] / w;
        let ndc_y = clip_p[1] / w;
        let depth = clip_p[2] / w;
        if ndc_x.abs() > 1.0 || ndc_y.abs() > 1.0 || !(0.0..=1.0).contains(&depth) {
            return 1.0;
        }
        let Some(origin) = self.config.slot_origin(slot) else {
            return 1.0;
        };
        let tile_px = f32_from_small_or(self.config.tile_px);
        let tx = texel_index((ndc_x * 0.5 + 0.5) * tile_px, self.config.tile_px);
        let ty = texel_index((0.5 - ndc_y * 0.5) * tile_px, self.config.tile_px);
        let stored = self.depth_at(origin[0] + tx, origin[1] + ty).unwrap_or(1.0);
        if stored + bias < depth {
            0.0
        } else {
            1.0
        }
    }
}

/// Pixel count as `f32` for the CPU reference; tiles stay within `u16`.
fn f32_from_small_or(v: usize) -> f32 {
    f32::from(u16::try_from(v).unwrap_or(u16::MAX))
}

/// Texel index for a position in texels, clamped to the tile.
fn texel_index(px: f32, tile: usize) -> usize {
    if !px.is_finite() || px <= 0.0 {
        return 0;
    }
    let mut idx = 0usize;
    let mut edge = 1.0f32;
    while idx + 1 < tile && px >= edge {
        idx += 1;
        edge += 1.0;
    }
    idx
}

/// Rasterize one slot's triangles into its tile; returns texels written.
fn raster_run(
    depth: &mut [f32],
    edge: usize,
    tile: usize,
    origin: [usize; 2],
    verts: &[[f32; 3]],
    clip: &[f32; 16],
) -> u32 {
    let mut written = 0u32;
    for tri in verts.chunks_exact(3) {
        let mut screen = [[0.0f32; 4]; 3];
        for (slot, p) in screen.iter_mut().zip(tri) {
            let clip_p = mul_clip(clip, *p);
            let w = clip_p[3];
            let inv_w = 1.0 / w;
            let sx = (clip_p[0] * inv_w * 0.5 + 0.5) * f32_from_small_or(tile);
            let sy = (0.5 - clip_p[1] * inv_w * 0.5) * f32_from_small_or(tile);
            let z_ndc = clip_p[2] * inv_w;
            *slot = [sx, sy, inv_w, z_ndc];
        }
        written += raster_tri(depth, edge, tile, origin, &screen);
    }
    written
}

/// Scanline raster of one projected triangle; nearest depth wins.
fn raster_tri(
    depth: &mut [f32],
    edge: usize,
    tile: usize,
    origin: [usize; 2],
    v: &[[f32; 4]; 3],
) -> u32 {
    let area =
        (v[1][0] - v[0][0]) * (v[2][1] - v[0][1]) - (v[2][0] - v[0][0]) * (v[1][1] - v[0][1]);
    if area == 0.0 || !area.is_finite() {
        return 0;
    }
    let inv_area = 1.0 / area;
    let y_min = v[0][1].min(v[1][1]).min(v[2][1]);
    let y_max = v[0][1].max(v[1][1]).max(v[2][1]);
    let x_min = v[0][0].min(v[1][0]).min(v[2][0]);
    let x_max = v[0][0].max(v[1][0]).max(v[2][0]);
    let mut written = 0u32;
    let mut py = 0usize;
    let mut fy = 0.5f32;
    while py < tile && fy <= y_max {
        if fy >= y_min {
            let mut px = 0usize;
            let mut fx = 0.5f32;
            while px < tile && fx < x_min {
                px += 1;
                fx += 1.0;
            }
            while px < tile && fx <= x_max {
                if let Some(z) = tri_depth(v, fx, fy, inv_area) {
                    let at = (origin[1] + py) * edge + origin[0] + px;
                    if z < depth[at] {
                        depth[at] = z;
                        written += 1;
                    }
                }
                px += 1;
                fx += 1.0;
            }
        }
        py += 1;
        fy += 1.0;
    }
    written
}

/// Depth the device writes at the pixel centre: the clip-space z/w of the
/// fragment, interpolated in screen space (the rasterizer's own rule), so
/// the reference and the device atlas hold the same number.
fn tri_depth(v: &[[f32; 4]; 3], x: f32, y: f32, inv_area: f32) -> Option<f32> {
    let w0 = ((v[1][0] - x) * (v[2][1] - y) - (v[2][0] - x) * (v[1][1] - y)) * inv_area;
    let w1 = ((v[2][0] - x) * (v[0][1] - y) - (v[0][0] - x) * (v[2][1] - y)) * inv_area;
    let w2 = 1.0 - w0 - w1;
    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
        return None;
    }
    let z = w0 * v[0][3] + w1 * v[1][3] + w2 * v[2][3];
    if z.is_finite() {
        Some(z.clamp(0.0, 1.0))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Device bake (window): one depth atlas, one pass, one draw per slot.
// ---------------------------------------------------------------------------

/// Byte size of one uniform slot; a multiple of the 256-byte offset
/// alignment every adapter guarantees.
pub const UNIFORM_SLOT_BYTES: u64 = 256;
/// Byte size of the matrix a slot uniform carries.
pub const SLOT_MATRIX_BYTES: u64 = 64;

/// Depth-only vertex shader over one spot's clip matrix. Engine-owned code.
#[cfg(feature = "window")]
pub const SPOT_DEPTH_WGSL: &str = r#"
struct SlotU {
    clip: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> slot: SlotU;
@vertex
fn vs(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return slot.clip * vec4<f32>(pos, 1.0);
}
"#;

/// Device-bake receipt: one pass, its texels and the posted budget. Parity
/// against the CPU reference is measured separately by [`atlas_parity`], so a
/// steady frame never pays for a readback stall.
#[cfg(feature = "window")]
#[derive(Clone, PartialEq, Debug)]
pub struct SpotGpuReceipt {
    /// Spots submitted.
    pub spots: usize,
    /// Spots that baked a shadow tile.
    pub shadowed: usize,
    /// Atlas texels resident.
    pub texels: usize,
    /// Real wgpu passes recorded (always one).
    pub passes_submitted: usize,
    /// wgpu validation errors observed; green means 0.
    pub validation_errors: u32,
    /// Triangles dropped for crossing a spot near plane, as the CPU reference
    /// drops them too.
    pub dropped_tris: u32,
    /// Wall time for encode plus submit in ms (evidence only).
    pub submit_ms: f64,
    /// True when a budget clamp fired before the submit.
    pub degraded: bool,
}

#[cfg(feature = "window")]
impl SpotGpuReceipt {
    /// True while the submit stayed inside the posted budget.
    pub fn within_budget(&self) -> bool {
        self.submit_ms <= SPOT_PASS_BUDGET_MS
    }

    /// True when the submit observed zero validation errors.
    pub fn is_clean(&self) -> bool {
        self.validation_errors == 0
    }

    /// Pass name for the frame report and the live-frame gate.
    pub fn pass(&self) -> &'static str {
        GATE_PASS
    }
}

/// Device depth atlas against the CPU reference: agreement share.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ParityReceipt {
    /// Atlas texels compared.
    pub compared: u32,
    /// Texels past [`PARITY_TOLERANCE`].
    pub mismatched: u32,
    /// Largest absolute depth disagreement seen.
    pub max_diff: f32,
}

impl ParityReceipt {
    /// Share of texels inside tolerance, 0..=1.
    pub fn agreement(&self) -> f32 {
        if self.compared == 0 {
            return 0.0;
        }
        let bad = self.mismatched.min(self.compared);
        let good = self.compared - bad;
        f32_from_u32(good) / f32_from_u32(self.compared)
    }

    /// True when agreement reaches [`PARITY_AGREEMENT`].
    pub fn passes(&self) -> bool {
        self.agreement() >= PARITY_AGREEMENT
    }
}

/// Whole `u32` as `f32` for a ratio; counts here stay within 2^24.
fn f32_from_u32(v: u32) -> f32 {
    let low = v & 0xffff;
    let high = v >> 16;
    f32::from(u16::try_from(low).unwrap_or(0))
        + 65536.0 * f32::from(u16::try_from(high).unwrap_or(0))
}

/// Reusable device bake: pipeline, bind shape, uniform buffer, vertex buffer
/// and the atlas texture. Built once per frame loop; a submit on a matching
/// shape creates no pipeline, bind group or texture.
#[cfg(feature = "window")]
pub struct SpotAtlasGpu {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind: wgpu::BindGroup,
    verts: Option<wgpu::Buffer>,
    verts_len: u64,
    atlas: Option<wgpu::Texture>,
    atlas_px: usize,
    scratch: Vec<u8>,
    vert_scratch: Vec<[f32; 3]>,
    runs: Vec<SlotRun>,
    clips: Vec<[f32; 16]>,
}

#[cfg(feature = "window")]
impl SpotAtlasGpu {
    /// Compile the depth pipeline and the dynamic-offset uniform bind, and
    /// allocate the fixed-size uniform and vertex scratch.
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spot-depth-shader"),
            source: wgpu::ShaderSource::Wgsl(SPOT_DEPTH_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spot-slot-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(SLOT_MATRIX_BYTES),
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spot-depth-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spot-depth-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 12,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    }],
                })],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spot-slot-uniforms"),
            size: UNIFORM_SLOT_BYTES * uniform_slots(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spot-slot-bind"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                // One 256-byte slot per binding: the dynamic offset walks
                // the slots. Binding the whole buffer would leave no room
                // for any offset above zero.
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniforms.clone(),
                    offset: 0,
                    size: wgpu::BufferSize::new(UNIFORM_SLOT_BYTES),
                }),
            }],
        });
        Self {
            pipeline,
            uniforms,
            bind,
            verts: None,
            verts_len: 0,
            atlas: None,
            atlas_px: 0,
            scratch: Vec::new(),
            vert_scratch: Vec::new(),
            runs: Vec::new(),
            clips: Vec::new(),
        }
    }

    /// Resident atlas edge in texels (probe for the reuse test).
    pub fn atlas_px(&self) -> usize {
        self.atlas_px
    }

    /// Ensure the atlas for `config`, rebuilding only on shape change.
    fn ensure_atlas(
        &mut self,
        device: &wgpu::Device,
        config: SpotAtlasConfig,
    ) -> Result<(), LightError> {
        if self.atlas_px == config.atlas_px() {
            return Ok(());
        }
        let edge = u32::try_from(config.atlas_px())
            .map_err(|_| LightError::Invalid("spot atlas edge does not fit u32".into()))?;
        self.atlas = Some(device.create_texture(&wgpu::TextureDescriptor {
            label: Some("spot-atlas-depth"),
            size: wgpu::Extent3d {
                width: edge,
                height: edge,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        }));
        self.atlas_px = config.atlas_px();
        Ok(())
    }

    /// Ensure the vertex buffer covers `bytes`, rebuilding only on growth.
    fn ensure_verts(&mut self, device: &wgpu::Device, bytes: u64) -> Result<(), LightError> {
        if self.verts_len >= bytes && self.verts.is_some() {
            return Ok(());
        }
        let size = bytes.max(12);
        self.verts = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spot-occluders"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        self.verts_len = size;
        Ok(())
    }
}

/// Uniform slots in the dynamic-offset buffer: one per atlas slot.
#[cfg(feature = "window")]
fn uniform_slots() -> u64 {
    u64::try_from(MAX_SPOTS).unwrap_or(1)
}

/// Encode the depth atlas on `device` and submit once. Consumes no submitter
/// capacity on a matching shape and never reads the atlas back, so a steady
/// frame pays no readback stall; [`atlas_parity`] is the gate's readback arm.
/// Fails loudly with typed errors: bad frames or shapes are `Invalid`,
/// adapter or map faults are `Device`, a validation scope hit is `Validation`.
/// Native only: it waits on the device, and a browser tab may not block.
#[cfg(all(feature = "window", not(target_arch = "wasm32")))]
pub fn submit_spot_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    spots: &[SpotLight],
    tris: &[OccluderTri],
    config: SpotAtlasConfig,
    submitter: &mut SpotAtlasGpu,
) -> Result<SpotGpuReceipt, LightError> {
    let prep = encode::prepare_atlas(device, queue, spots, tris, config, submitter)?;
    let started = engine_clock::now();
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("spot-atlas-bake"),
    });
    encode_atlas_pass(submitter, config, &mut encoder)?;
    queue.submit(Some(encoder.finish()));
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| device_err("spot submit poll", &e))?;
    let submit_ms = engine_clock::since(started).as_secs_f64() * 1000.0;
    if let Some(err) = pollster::block_on(scope.pop()) {
        return Err(scope_err(&err));
    }
    Ok(SpotGpuReceipt {
        spots: prep.spots,
        shadowed: prep.shadowed,
        texels: config.texels(),
        passes_submitted: 1,
        validation_errors: 0,
        dropped_tris: prep.dropped_tris,
        submit_ms,
        degraded: prep.degraded,
    })
}

/// Gate arm: copy the resident atlas back and compare every texel with the
/// CPU reference. Software-adapter parity is this row; it is not per frame.
/// Native only: the readback waits on the device.
#[cfg(all(feature = "window", not(target_arch = "wasm32")))]
pub fn atlas_parity(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    submitter: &SpotAtlasGpu,
    reference: &SpotAtlas,
) -> Result<ParityReceipt, LightError> {
    let encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("spot-atlas-parity"),
    });
    read_back_atlas(
        device,
        queue,
        submitter,
        reference.config(),
        encoder,
        reference,
    )
}

/// Validate the spots and build one clip matrix per atlas slot. Slots past
/// the spot list hold the zero matrix and are never drawn.
#[cfg(feature = "window")]
fn build_slot_clips(
    spots: &[SpotLight],
    slots: usize,
    clips: &mut Vec<[f32; 16]>,
) -> Result<usize, LightError> {
    clips.clear();
    let mut shadowed = 0usize;
    for slot in 0..slots {
        let Some(spot) = spots.get(slot) else {
            clips.push([0.0f32; 16]);
            continue;
        };
        spot.validate()?;
        if !spot.cast_shadow {
            clips.push([0.0f32; 16]);
            continue;
        }
        let clip = spot_clip_matrix(
            spot.pos,
            spot.dir,
            spot.shadow_tan_half()?,
            SPOT_NEAR_M,
            spot.range_m,
        )?;
        clips.push(clip);
        shadowed += 1;
    }
    Ok(shadowed)
}

/// Write the per-slot matrices and the accepted vertices to their buffers,
/// growing the vertex buffer only on a larger frame.
#[cfg(feature = "window")]
fn upload_slots(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    submitter: &mut SpotAtlasGpu,
) -> Result<(), LightError> {
    let bytes = pack_vertices(&mut submitter.scratch, &submitter.vert_scratch)?;
    submitter.ensure_verts(device, bytes)?;
    for (slot, clip) in submitter.clips.iter().enumerate() {
        let offset = u64::try_from(slot)
            .map_err(|_| LightError::Invalid("spot slot does not fit u64".into()))?
            * UNIFORM_SLOT_BYTES;
        queue.write_buffer(&submitter.uniforms, offset, &matrix_bytes(clip));
    }
    let Some(verts) = submitter.verts.as_ref() else {
        return Err(LightError::Invalid(
            "spot vertex buffer missing after ensure".into(),
        ));
    };
    queue.write_buffer(verts, 0, &submitter.scratch);
    Ok(())
}

/// Record the depth pass: one clear, then one viewport and draw per slot.
#[cfg(feature = "window")]
fn encode_atlas_pass(
    submitter: &SpotAtlasGpu,
    config: SpotAtlasConfig,
    encoder: &mut wgpu::CommandEncoder,
) -> Result<(), LightError> {
    let Some(atlas) = submitter.atlas.as_ref() else {
        return Err(LightError::Invalid(
            "spot atlas missing after ensure".into(),
        ));
    };
    let view = atlas.create_view(&wgpu::TextureViewDescriptor::default());
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("spot-atlas-depth"),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(&submitter.pipeline);
    // The depth pipeline reads its occluder triangles from vertex buffer 0:
    // the accepted geometry `upload_slots` wrote. Without this binding the
    // device refuses the draw (the pipeline wants slot 0).
    if let Some(verts) = submitter.verts.as_ref() {
        pass.set_vertex_buffer(0, verts.slice(..));
    }
    let tile = tile_f32(config.tile_px)?;
    let slot_stride = usize::try_from(UNIFORM_SLOT_BYTES)
        .map_err(|_| LightError::Invalid("uniform slot does not fit usize".into()))?;
    for (slot, run) in submitter.runs.iter().enumerate() {
        let Some(origin) = config.slot_origin(slot) else {
            break;
        };
        if run.vertex_count == 0 {
            continue;
        }
        let first = u32::try_from(run.first_vertex)
            .map_err(|_| LightError::Invalid("spot run start does not fit u32".into()))?;
        let count = u32::try_from(run.vertex_count)
            .map_err(|_| LightError::Invalid("spot run length does not fit u32".into()))?;
        let offset = u32::try_from(slot * slot_stride)
            .map_err(|_| LightError::Invalid("uniform offset does not fit u32".into()))?;
        pass.set_viewport(
            tile_f32(origin[0])?,
            tile_f32(origin[1])?,
            tile,
            tile,
            0.0,
            1.0,
        );
        pass.set_bind_group(0, &submitter.bind, &[offset]);
        let last = first
            .checked_add(count)
            .ok_or_else(|| LightError::Invalid("spot run end overflows".into()))?;
        pass.draw(first..last, 0..1);
    }
    Ok(())
}

/// Copy the atlas into a readback buffer, wait for the map, and compare the
/// texels with the CPU reference. Native only: the map waits on the device.
#[cfg(all(feature = "window", not(target_arch = "wasm32")))]
fn read_back_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    submitter: &SpotAtlasGpu,
    config: SpotAtlasConfig,
    mut encoder: wgpu::CommandEncoder,
    reference: &SpotAtlas,
) -> Result<ParityReceipt, LightError> {
    let edge_px = config.atlas_px();
    let edge_u32 = u32::try_from(edge_px)
        .map_err(|_| LightError::Invalid("spot atlas edge does not fit u32".into()))?;
    let padded = (edge_u32 * 4).div_ceil(256) * 256;
    let padded_px = usize::try_from(padded)
        .map_err(|_| LightError::Invalid("spot readback row does not fit usize".into()))?;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("spot-readback"),
        size: u64::from(padded) * u64::from(edge_u32),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let Some(atlas) = submitter.atlas.as_ref() else {
        return Err(LightError::Invalid(
            "spot atlas missing for readback".into(),
        ));
    };
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: atlas,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::DepthOnly,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(edge_u32),
            },
        },
        wgpu::Extent3d {
            width: edge_u32,
            height: edge_u32,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| device_err("spot readback poll", &e))?;
    rx.recv()
        .map_err(|e| device_err("spot readback channel", &e))?
        .map_err(|e| device_err("spot readback map", &e))?;
    let mapped = slice
        .get_mapped_range()
        .map_err(|e| device_err("spot readback view", &e))?;
    let parity = compare_atlas(&mapped, padded_px, edge_px, reference);
    drop(mapped);
    readback.unmap();
    Ok(parity)
}

/// Cold error path: a device fault with its context. Built here, outside the
/// frame function, so the formatter is not charged to a steady frame.
#[cfg(feature = "window")]
fn device_err(context: &'static str, e: &dyn std::fmt::Display) -> LightError {
    LightError::Device(format!("{context}: {e}"))
}

/// Cold error path: a wgpu validation scope hit.
#[cfg(feature = "window")]
fn scope_err(e: &dyn std::fmt::Display) -> LightError {
    LightError::Validation(format!("spot atlas scope: {e}"))
}

/// Pack one slot's clip matrix into the 64 bytes the uniform holds (no pad).
#[cfg(feature = "window")]
fn matrix_bytes(clip: &[f32; 16]) -> [u8; 64] {
    let mut out = [0u8; 64];
    for (i, v) in clip.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

/// Tile or texel count as `f32`; atlas bounds keep every count within `u16`.
#[cfg(feature = "window")]
fn tile_f32(v: usize) -> Result<f32, LightError> {
    u16::try_from(v)
        .map(f32::from)
        .map_err(|_| LightError::Invalid(format!("{v} is past the 65535 texel float bound")))
}

/// Fill `scratch` with accepted vertices only, slot by slot, so the vertex
/// buffer and the CPU reference raster the same triangles.
#[cfg(feature = "window")]
fn pack_vertices(scratch: &mut Vec<u8>, verts: &[[f32; 3]]) -> Result<u64, LightError> {
    scratch.clear();
    scratch.reserve(verts.len() * 12);
    for v in verts {
        for c in v {
            scratch.extend_from_slice(&c.to_le_bytes());
        }
    }
    u64::try_from(scratch.len())
        .map_err(|_| LightError::Invalid("spot vertex bytes do not fit u64".into()))
}

/// Compare the mapped device atlas with the CPU reference texel by texel.
#[cfg(feature = "window")]
fn compare_atlas(
    mapped: &[u8],
    padded: usize,
    edge_px: usize,
    reference: &SpotAtlas,
) -> ParityReceipt {
    let mut compared = 0u32;
    let mut mismatched = 0u32;
    let mut max_diff = 0.0f32;
    for y in 0..edge_px {
        let start = padded * y;
        let Some(row) = mapped.get(start..start + edge_px * 4) else {
            break;
        };
        for (x, chunk) in row.chunks_exact(4).enumerate() {
            let Ok(bytes) = <[u8; 4]>::try_from(chunk) else {
                continue;
            };
            let device_depth = f32::from_le_bytes(bytes);
            let Some(cpu_depth) = reference.depth_at(x, y) else {
                continue;
            };
            compared += 1;
            let diff = (device_depth - cpu_depth).abs();
            if diff > max_diff {
                max_diff = diff;
            }
            if diff > PARITY_TOLERANCE {
                mismatched += 1;
            }
        }
    }
    ParityReceipt {
        compared,
        mismatched,
        max_diff,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_triangle_fixture() -> (SpotLight, Vec<OccluderTri>) {
        let spot = SpotLight {
            pos: [0.0, 0.0, 0.0],
            dir: [0.0, 0.0, 1.0],
            inner_cos: 0.99,
            outer_cos: 0.9,
            range_m: 10.0,
            color: [1.0, 1.0, 1.0],
            intensity: 1.0,
            cast_shadow: true,
        };
        let tris = vec![
            OccluderTri {
                a: [-1.0, -1.0, 2.0],
                b: [1.0, -1.0, 2.0],
                c: [1.0, 1.0, 2.0],
            },
            OccluderTri {
                a: [-1.0, -1.0, 2.0],
                b: [1.0, 1.0, 2.0],
                c: [-1.0, 1.0, 2.0],
            },
        ];
        (spot, tris)
    }

    fn unit_atlas() -> SpotAtlas {
        SpotAtlas::new(SpotAtlasConfig {
            grid: 1,
            tile_px: 32,
        })
        .expect("test atlas builds")
    }

    #[test]
    fn config_clamps_grid_and_tile_and_records_the_degrade() {
        let (config, degraded) = SpotAtlasConfig::degrade(9, 4096);
        assert_eq!(config.grid, ATLAS_GRID);
        assert_eq!(config.tile_px, ATLAS_TILE_MAX);
        match degraded {
            Some(LightError::BudgetExceeded { requested, allowed }) => {
                assert_eq!(requested, 1_358_954_496, "nine by nine at 4096 texels");
                assert_eq!(allowed, 4_194_304, "four by four at 512 texels");
            }
            other => panic!("an over-budget shape is recorded, got {other:?}"),
        }
        let (same, none) = SpotAtlasConfig::degrade(ATLAS_GRID, 128);
        assert_eq!(same.slots(), MAX_SPOTS);
        assert!(none.is_none(), "an in-budget shape is silent");
        assert!(SpotAtlasConfig {
            grid: 0,
            tile_px: 128
        }
        .validate()
        .is_err());
    }

    #[test]
    fn bad_cones_ranges_and_directions_fail_loudly() {
        let (good, _) = one_triangle_fixture();
        good.validate().expect("the fixture light is valid");
        let mut bad = good;
        bad.dir = [0.0, 0.0, 0.0];
        assert!(bad.validate().is_err(), "a zero direction refuses");
        let mut bad = good;
        bad.inner_cos = 0.5;
        bad.outer_cos = 0.9;
        assert!(bad.validate().is_err(), "an inverted cone refuses");
        let mut bad = good;
        bad.range_m = 0.0;
        assert!(bad.validate().is_err(), "a zero range refuses");
        let mut bad = good;
        bad.outer_cos = 0.0;
        assert!(bad.validate().is_err(), "a ninety degree cone refuses");
    }

    #[test]
    fn the_clip_matrix_puts_the_range_on_zero_and_one() {
        let (spot, _) = one_triangle_fixture();
        let clip = spot_clip_matrix(
            spot.pos,
            spot.dir,
            spot.shadow_tan_half().expect("tan"),
            SPOT_NEAR_M,
            spot.range_m,
        )
        .expect("matrix builds");
        let at_near = mul_clip(&clip, [0.0, 0.0, SPOT_NEAR_M]);
        let near_ndc = at_near[2] / at_near[3];
        assert!(near_ndc.abs() < 1.0e-3, "near lands on 0, got {near_ndc}");
        let at_far = mul_clip(&clip, [0.0, 0.0, spot.range_m]);
        let far_ndc = at_far[2] / at_far[3];
        assert!(
            (far_ndc - 1.0).abs() < 1.0e-3,
            "far lands on 1, got {far_ndc}"
        );
        let centre = mul_clip(&clip, [0.0, 0.0, 2.0]);
        assert!(centre[0].abs() < 1.0e-4 && centre[1].abs() < 1.0e-4);
    }

    #[test]
    fn a_pillar_triangle_hides_the_point_behind_it() {
        let (spot, tris) = one_triangle_fixture();
        let mut atlas = unit_atlas();
        let receipt = atlas.bake(&[spot], &tris).expect("bake");
        assert_eq!(receipt.shadowed, 1);
        assert!(receipt.written > 0, "the triangle wrote texels");
        let clip = spot_clip_matrix(
            spot.pos,
            spot.dir,
            spot.shadow_tan_half().expect("tan"),
            SPOT_NEAR_M,
            spot.range_m,
        )
        .expect("matrix");
        let behind = atlas.shadow_factor(0, &clip, [0.0, 0.0, 3.0], 1.0e-3);
        assert_eq!(behind, 0.0, "the triangle hides the point behind it");
        let open = atlas.shadow_factor(0, &clip, [3.0, 0.0, 3.0], 1.0e-3);
        assert_eq!(open, 1.0, "a point off the triangle stays lit");
        let in_front = atlas.shadow_factor(0, &clip, [0.0, 0.0, 1.0], 1.0e-3);
        assert_eq!(in_front, 1.0, "a point in front of the triangle stays lit");
    }

    #[test]
    fn the_dark_room_bakes_every_tile_and_is_bit_identical_twice() {
        let tris = dark_room();
        let spots = dark_room_spots();
        assert_eq!(spots.len(), MAX_SPOTS);
        let config = SpotAtlasConfig {
            grid: ATLAS_GRID,
            tile_px: 64,
        };
        let mut first = SpotAtlas::new(config).expect("atlas");
        let a = first.bake(&spots, &tris).expect("first bake");
        assert_eq!(a.shadowed, MAX_SPOTS);
        assert!(a.written > 0);
        for slot in 0..config.slots() {
            let origin = config.slot_origin(slot).expect("slot");
            let mut lit = 0usize;
            for y in 0..config.tile_px {
                for x in 0..config.tile_px {
                    let d = first.depth_at(origin[0] + x, origin[1] + y).expect("texel");
                    if d < 1.0 {
                        lit += 1;
                    }
                }
            }
            assert!(lit > 0, "slot {slot} baked no depth");
        }
        let mut second = SpotAtlas::new(config).expect("atlas");
        let b = second.bake(&spots, &tris).expect("second bake");
        assert_eq!(a, b, "the receipt repeats");
        assert_eq!(first.depth, second.depth, "the tile repeats bit for bit");
    }

    #[test]
    fn more_spots_than_slots_degrade_instead_of_failing() {
        let tris = dark_room();
        let mut spots = dark_room_spots();
        spots.push(lamp([5.0, 2.4, 5.0], [0.0, -1.0, 0.0]));
        let config = SpotAtlasConfig {
            grid: 2,
            tile_px: 32,
        };
        let mut atlas = SpotAtlas::new(config).expect("atlas");
        let receipt = atlas.bake(&spots, &tris).expect("bake degrades");
        assert!(receipt.degraded, "the tail is recorded");
        assert_eq!(receipt.shadowed, config.slots());
    }

    #[test]
    fn a_triangle_overflow_degrades_instead_of_failing() {
        let (spot, _) = one_triangle_fixture();
        let tris: Vec<OccluderTri> = (0..=MAX_OCCLUDER_TRIS)
            .map(|_| OccluderTri {
                a: [-1.0, -1.0, 2.0],
                b: [1.0, -1.0, 2.0],
                c: [1.0, 1.0, 2.0],
            })
            .collect();
        let mut atlas = unit_atlas();
        let receipt = atlas.bake(&[spot], &tris).expect("bake degrades");
        assert!(receipt.degraded, "the triangle tail is recorded");
    }

    #[test]
    fn the_screen_footprint_lands_in_front_and_refuses_behind() {
        let view = SpotView {
            eye: [5.0, 1.6, 12.0],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 1.0, 0.0],
            forward: [0.0, 0.0, -1.0],
            tan_half: 0.7,
            width_px: 640.0,
            height_px: 360.0,
            near_m: 0.1,
            far_m: 100.0,
        };
        let mut spot = lamp([5.0, 1.6, 5.0], [0.0, 0.0, -1.0]);
        spot.range_m = 4.0;
        let screen = spot
            .screen_light(&view)
            .expect("view validates")
            .expect("spot is in front");
        assert!((screen.x_px - 320.0).abs() < 0.5);
        assert!((screen.y_px - 180.0).abs() < 0.5);
        assert!(screen.radius_px > 0.0);
        assert!((screen.depth_m - 7.0).abs() < 1.0e-3);
        let behind = lamp([5.0, 1.6, 20.0], [0.0, 0.0, 1.0]);
        assert!(behind.screen_light(&view).expect("view").is_none());
    }

    #[test]
    fn cone_and_distance_factors_hold_their_bounds() {
        let (spot, _) = one_triangle_fixture();
        assert_eq!(spot.cone_factor(spot.inner_cos), 1.0);
        assert_eq!(spot.cone_factor(spot.outer_cos), 0.0);
        assert_eq!(spot.cone_factor(spot.outer_cos - 0.05), 0.0);
        let mid = spot.cone_factor((spot.inner_cos + spot.outer_cos) * 0.5);
        assert!((mid - 0.5).abs() < 1.0e-5, "the midpoint is half lit");
        assert_eq!(spot.distance_factor(0.0), 1.0);
        assert_eq!(spot.distance_factor(spot.range_m), 0.0);
        assert_eq!(spot.distance_factor(-1.0), 0.0);
    }

    #[test]
    fn the_parity_receipt_reads_agreement() {
        let clean = ParityReceipt {
            compared: 1000,
            mismatched: 0,
            max_diff: 0.0,
        };
        assert_eq!(clean.agreement(), 1.0);
        assert!(clean.passes());
        let dirty = ParityReceipt {
            compared: 1000,
            mismatched: 50,
            max_diff: 0.5,
        };
        assert!((dirty.agreement() - 0.95).abs() < 1.0e-6);
        assert!(!dirty.passes(), "five percent off is a failed parity");
        let empty = ParityReceipt {
            compared: 0,
            mismatched: 0,
            max_diff: 0.0,
        };
        assert_eq!(empty.agreement(), 0.0);
        assert!(!empty.passes());
    }

    // ------------------------------------------------------------------
    // Device bake (behind `window`): the row's own evidence on the
    // adapter under test. The device comes from the shared suite device
    // (skip, never fake, without an adapter).
    // ------------------------------------------------------------------

    #[cfg(feature = "window")]
    fn spot_device() -> Option<(
        &'static wgpu::Device,
        &'static wgpu::Queue,
        &'static wgpu::AdapterInfo,
    )> {
        crate::graph::gx1_test_device()
    }

    /// The dark room's atlas shape: four tiles per side at 64 texels.
    #[cfg(feature = "window")]
    fn dark_room_config() -> SpotAtlasConfig {
        SpotAtlasConfig {
            grid: ATLAS_GRID,
            tile_px: 64,
        }
    }

    #[cfg(feature = "window")]
    #[test]
    fn the_device_bakes_sixteen_shadowed_spots_in_one_submit() {
        // Caller: `submit_spot_atlas` (the row's submit entry).
        // Behavior: the dark room's sixteen lamps each bake one shadow tile
        // on the device, in one submit, with no validation error.
        let Some((device, queue, info)) = spot_device() else {
            eprintln!("SKIPPED spot device bake: no wgpu adapter on this machine");
            return;
        };
        let tris = dark_room();
        let spots = dark_room_spots();
        let config = dark_room_config();
        let mut submitter = SpotAtlasGpu::new(device);
        // Cold arm: the first submit pays the driver's shader compile, which
        // no steady frame pays. The steady arm is the budget the row answers
        // for, so it is the one the test holds.
        let cold = submit_spot_atlas(device, queue, &spots, &tris, config, &mut submitter)
            .expect("the dark room bakes on the device");
        // Three steady submits and the median reported: one slow sample from
        // a loaded box is not the pass's cost. Every sample is posted.
        let mut samples = Vec::with_capacity(3);
        for _ in 0..3 {
            samples.push(
                submit_spot_atlas(device, queue, &spots, &tris, config, &mut submitter)
                    .expect("the steady dark room bakes on the device"),
            );
        }
        samples.sort_by(|a, b| a.submit_ms.total_cmp(&b.submit_ms));
        let receipt = samples[1].clone();
        eprintln!(
            "spot device bake on {}: submit_ms {:.3} budget_ms {:.3} shadowed {} (cold {:.3} ms, samples {:.3}/{:.3}/{:.3})",
            info.name,
            receipt.submit_ms,
            SPOT_PASS_BUDGET_MS,
            receipt.shadowed,
            cold.submit_ms,
            samples[0].submit_ms,
            samples[1].submit_ms,
            samples[2].submit_ms
        );
        assert_eq!(receipt.spots, MAX_SPOTS, "every lamp reached the bake");
        assert_eq!(receipt.shadowed, MAX_SPOTS, "every lamp cast a shadow tile");
        assert_eq!(receipt.passes_submitted, 1, "the pass ran once");
        assert_eq!(receipt.validation_errors, 0, "the scope saw no error");
        assert!(receipt.is_clean(), "the submit was clean");
        assert_eq!(receipt.pass(), GATE_PASS);
        // The bar is the reference GPU's. A software adapter (the runner's
        // basic render driver, llvmpipe) posts its own number and never
        // holds the row to the reference budget: skip, not a fake pass.
        // Under E2040_REQUIRE_GPU=1 that adapter is a failure instead.
        let caps = crate::tier::AdapterCaps::new(&info.name, false, false, 0);
        if caps.is_software() {
            assert!(
                !crate::test_gpu::required_by_env(),
                "the spot budget needs the reference GPU; {} forbids the software adapter {}",
                crate::test_gpu::REQUIRE_GPU_ENV,
                info.name
            );
            crate::test_gpu::note_skip(
                "the spot budget",
                &format!(
                    "the budget is the reference GPU's; {} posted its own number",
                    info.name
                ),
            );
            return;
        }
        assert!(
            receipt.within_budget(),
            "the submit took {:.3} ms, over the {:.3} ms budget",
            receipt.submit_ms,
            SPOT_PASS_BUDGET_MS
        );
    }

    #[cfg(feature = "window")]
    #[test]
    fn the_device_atlas_agrees_with_the_cpu_reference() {
        // Caller: `atlas_parity` (the gate's readback arm).
        // Behavior: the device atlas and the CPU reference agree on the
        // dark room's depth, texel for texel, inside the row's tolerance.
        let Some((device, queue, info)) = spot_device() else {
            eprintln!("SKIPPED spot parity: no wgpu adapter on this machine");
            return;
        };
        let tris = dark_room();
        let spots = dark_room_spots();
        let config = dark_room_config();
        let mut reference = SpotAtlas::new(config).expect("the reference atlas");
        reference.bake(&spots, &tris).expect("the reference bakes");
        let mut submitter = SpotAtlasGpu::new(device);
        submit_spot_atlas(device, queue, &spots, &tris, config, &mut submitter)
            .expect("the device bakes the same room");
        let parity = atlas_parity(device, queue, &submitter, &reference).expect("the readback");
        eprintln!(
            "spot parity on {}: compared {} mismatched {} max_diff {} agreement {}",
            info.name,
            parity.compared,
            parity.mismatched,
            parity.max_diff,
            parity.agreement()
        );
        assert_eq!(
            parity.compared as usize,
            config.atlas_px() * config.atlas_px(),
            "every atlas texel was compared"
        );
        assert!(
            parity.passes(),
            "the device atlas agrees with the CPU reference: {} of {} texels inside {}",
            parity.compared - parity.mismatched.min(parity.compared),
            parity.compared,
            PARITY_TOLERANCE
        );
    }

    #[cfg(feature = "window")]
    #[test]
    fn the_encode_path_records_the_pass_without_submitting_it() {
        // Caller: `encode_spot_atlas` (the live frame's entry).
        // Behavior: the encode records the depth pass into the caller's
        // command buffer and submits nothing itself: before the caller's
        // submit the atlas still reads empty, after it the atlas matches the
        // CPU reference. The live frame keeps its no-wait shape.
        let Some((device, queue, _info)) = spot_device() else {
            eprintln!("SKIPPED spot encode: no wgpu adapter on this machine");
            return;
        };
        let tris = dark_room();
        let spots = dark_room_spots();
        let config = dark_room_config();
        let mut reference = SpotAtlas::new(config).expect("the reference atlas");
        reference.bake(&spots, &tris).expect("the reference bakes");
        let mut submitter = SpotAtlasGpu::new(device);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("spot-encode-test"),
        });
        let receipt = encode_spot_atlas(
            device,
            queue,
            &spots,
            &tris,
            config,
            &mut submitter,
            &mut enc,
        )
        .expect("the encode records the pass");
        assert!(receipt.ran(), "one pass was recorded");
        assert_eq!(receipt.passes_encoded, 1);
        assert_eq!(receipt.shadowed, MAX_SPOTS);
        assert_eq!(receipt.pass(), GATE_PASS);
        let before = atlas_parity(device, queue, &submitter, &reference).expect("the readback");
        assert!(
            !before.passes(),
            "the unsubmitted encode left the atlas empty (agreement {})",
            before.agreement()
        );
        queue.submit(Some(enc.finish()));
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the poll returns");
        let after = atlas_parity(device, queue, &submitter, &reference).expect("the readback");
        assert!(
            after.passes(),
            "the caller's submit landed the pass: agreement {}",
            after.agreement()
        );
    }

    #[test]
    fn both_paths_drop_a_triangle_behind_the_near_plane() {
        let clip = spot_clip_matrix([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], 1.0, SPOT_NEAR_M, 10.0)
            .expect("matrix");
        let front = OccluderTri {
            a: [-1.0, -1.0, 2.0],
            b: [1.0, -1.0, 2.0],
            c: [1.0, 1.0, 2.0],
        };
        let crossing = OccluderTri {
            a: [-1.0, -1.0, -1.0],
            b: [1.0, -1.0, 2.0],
            c: [1.0, 1.0, 2.0],
        };
        let mut verts = Vec::new();
        let mut runs = Vec::new();
        let dropped = build_slot_geometry(
            &mut verts,
            &mut runs,
            &[clip],
            &[front, crossing],
            SPOT_NEAR_M,
        );
        assert_eq!(dropped, 1, "the crossing triangle is dropped");
        assert_eq!(runs[0].vertex_count, 3, "one triangle stays");
        assert_eq!(verts.len(), 3);
    }
}
