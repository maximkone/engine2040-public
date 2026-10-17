//! draw: scene draw list with frustum culling and LOD select (RA3).
//!
//! Owner: render lane; consumer RA2 graph executor (wires next).
//! Units: meters for world space; depth buckets are integer millimeters.
//! Determinism: `+ - * /` and `sqrt` only, integer `(depth_bucket, id)`
//! sort keys, so identical input bytes give identical output bytes.
//! Errors: typed `DrawError`; over-cap and unknown tables fail loudly.
//! Budget: build is O(n log n) in visible draws, capped at 256/frame;
//! a reused `DrawList` allocates nothing after warmup.
//! Scale proof: a seeded 1,000-agent storm fixture plus an instancing
//! budget beside the cap — one instanced draw per mesh/LOD squad — with
//! the same integer keys and loud over-budget refusal.

use crate::lod_fade;
use engine_core::vec3::cross3;
use engine_core::{Fnv64, Rng};
use std::error::Error;
use std::fmt;

/// Most draws in one frame. Mirrors `present::MAX_DRAWS_PER_FRAME` as a
/// local constant (this module is std-only plus the engine hash/rng
/// owner); the two must stay equal, asserted by the consumer when it
/// wires up.
pub const MAX_DRAWS: usize = 256;

/// Most instanced draws in one frame: one per mesh/LOD squad, budgeted
/// beside the per-draw cap and refused loudly past it.
pub const MAX_INSTANCED_GROUPS: usize = 256;

/// Most instances one instanced draw carries; the 1,000-agent storm fits
/// inside a single squad with room to spare.
pub const MAX_INSTANCES_PER_GROUP: usize = 1024;

/// Agents in the seeded draw-storm scale fixture.
pub const STORM_AGENTS: usize = 1000;

/// Millimeters per meter for the integer depth bucket.
const MM_PER_M: f32 = 1000.0;

/// Smallest usable near-plane distance in meters.
const MIN_NEAR_M: f32 = 1e-3;

/// Plain-data camera. Right-handed, +Y up, meters; looks along `forward`.
#[derive(Clone, Debug)]
pub struct Camera {
    /// Eye position in meters.
    pub pos_m: [f32; 3],
    /// View direction (need not be unit; must be finite and nonzero).
    pub forward: [f32; 3],
    /// World up hint (must not be parallel to `forward`).
    pub up: [f32; 3],
    /// Vertical field of view in radians, exclusive of 0 and pi.
    pub fov_y_rad: f32,
    /// Width over height, finite and positive.
    pub aspect: f32,
    /// Near plane distance in meters, must satisfy `0 < near < far`.
    pub near_m: f32,
    /// Far plane distance in meters.
    pub far_m: f32,
}

/// One scene node as plain data: bounding sphere plus LOD bookkeeping.
#[derive(Clone, Debug)]
pub struct SceneNode {
    /// Stable identity; ties the integer sort key, never reused mid-frame.
    pub id: u64,
    /// Stable mesh identity the executor resolves through the GPU cache.
    pub mesh: u64,
    /// Sphere center in meters.
    pub center_m: [f32; 3],
    /// Sphere radius in meters, finite and non-negative.
    pub radius_m: f32,
    /// LOD levels available for this node, at least 1.
    pub lod_levels: u8,
    /// Which [`LodTable`] governs this node; must equal the passed table.
    pub lod_table: u64,
    /// LOD picked last frame; hysteresis bands hold it until the distance
    /// clearly leaves the band. Must be below `lod_levels`.
    pub prev_lod: u8,
}

/// Minimal owned snapshot feeding one [`DrawList::build`] call.
#[derive(Clone, Debug, Default)]
pub struct SceneSnapshot {
    /// Nodes in any order; output order never follows input order.
    pub nodes: Vec<SceneNode>,
}

/// Distance bands mapping camera distance to LOD level.
#[derive(Clone, Debug)]
pub struct LodTable {
    /// Identity matched against [`SceneNode::lod_table`].
    pub id: u64,
    /// Ascending boundary distances in meters; level count is
    /// `bands_m.len() + 1`, capped by each node's `lod_levels`.
    pub bands_m: Vec<f32>,
    /// Half-width of the hysteresis band around each boundary in meters.
    pub hysteresis_m: f32,
    /// Ascending projected-size boundaries in pixels, or `None` for the
    /// distance bands; the last entry separates level 0 from level 1 and the
    /// first separates the two coarsest. Read by [`lod_by_pixel_error`].
    pub pixel_thresholds_px: Option<Vec<f32>>,
}

/// One culled, LOD-picked draw in integer-stable order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Draw {
    /// Stable node identity (secondary sort key).
    pub id: u64,
    /// Stable mesh identity.
    pub mesh: u64,
    /// LOD level picked with hysteresis.
    pub lod: u8,
    /// Dither keep share 0..=255 for the outgoing LOD edge: 255 fully
    /// kept, dissolving toward 0 across the hysteresis band. The forward
    /// pass thresholds pixels against it, so the switch dissolves instead
    /// of popping; the coarsest level stays 255.
    pub fade: u8,
    /// Euclidean camera distance in meters.
    pub distance_m: f32,
    /// Primary sort key: millimeters, saturated at `u32::MAX`.
    pub depth_bucket: u32,
}

/// Typed draw-list failures; every variant names what to fix.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawError {
    /// Camera axes or optics are degenerate; message names the field.
    BadCamera(String),
    /// One node carries NaN, a negative radius, zero levels, or a
    /// `prev_lod` outside its levels.
    BadNode { id: u64, reason: String },
    /// The LOD table itself is degenerate (NaN, unordered, over-wide).
    BadLodTable(String),
    /// Pixel-error LOD inputs are degenerate; the message names the field.
    BadPixelLod(String),
    /// A node names a table that was not supplied; nothing is guessed.
    UnknownLodTable {
        node: u64,
        expected: u64,
        found: u64,
    },
    /// More visible draws than the cap; caller must split or shrink.
    OverCap { visible: usize, cap: usize },
    /// More mesh/LOD squads than the instancing budget; caller must raise
    /// the budget or split the scene, never silently drops a squad.
    OverInstancedCap { groups: usize, cap: usize },
    /// One squad holds more instances than its per-group budget; split the
    /// squad or raise the per-group budget.
    OverInstances {
        mesh: u64,
        lod: u8,
        count: usize,
        cap: usize,
    },
}

impl fmt::Display for DrawError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DrawError::BadCamera(why) => write!(f, "bad camera: {why}"),
            DrawError::BadNode { id, reason } => write!(f, "bad node {id}: {reason}"),
            DrawError::BadLodTable(why) => write!(f, "bad lod table: {why}"),
            DrawError::BadPixelLod(why) => write!(f, "bad pixel lod: {why}"),
            DrawError::UnknownLodTable {
                node,
                expected,
                found,
            } => write!(
                f,
                "node {node} names lod table {found}, expected {expected}"
            ),
            DrawError::OverCap { visible, cap } => {
                write!(f, "draw cap exceeded: {visible} visible, cap {cap}")
            }
            DrawError::OverInstancedCap { groups, cap } => {
                write!(f, "instanced cap exceeded: {groups} squads, cap {cap}")
            }
            DrawError::OverInstances {
                mesh,
                lod,
                count,
                cap,
            } => write!(
                f,
                "squad (mesh {mesh}, lod {lod}) holds {count} instances, cap {cap}"
            ),
        }
    }
}

impl Error for DrawError {}

/// GPU cull shader source (GS18 slice 1): the same `-radius` rule the CPU
/// oracle runs. `DrawList::build` pins it so the two never drift apart.
const CULL_WGSL: &str = include_str!("shaders/cull.wgsl");

/// One frustum plane: inside where `normal . x + dist >= -radius`.
#[derive(Clone, Copy, Debug)]
struct Plane {
    normal: [f32; 3],
    dist: f32,
}

/// GPU cull CPU oracle (GS18 slice 1): the same `-radius` rule the
/// `shaders/cull.wgsl` compute shader runs, over packed `[nx, ny, nz, dist]`
/// planes. Std-only; consumer is `cull_lod` below and the next slice's GPU
/// runner. Units: meters for centers and radii. Determinism: `+ - *` only
/// in fixed plane order, so same bytes give the same flags x2.
fn planes_to_packed(planes: &[Plane; 6]) -> [[f32; 4]; 6] {
    let mut packed = [[0.0_f32; 4]; 6];
    let mut i = 0;
    while i < 6 {
        packed[i] = [
            planes[i].normal[0],
            planes[i].normal[1],
            planes[i].normal[2],
            planes[i].dist,
        ];
        i += 1;
    }
    packed
}

/// True when the sphere survives all six planes with the `-radius` rule
/// (Lengyel vol2 9.4.2 sphere visibility). Touching counts as visible.
/// Non-finite inputs simply cull away; validation in `cull_lod` stays loud.
#[must_use]
pub fn sphere_visible_cpu(center_m: [f32; 3], radius_m: f32, planes: &[[f32; 4]; 6]) -> bool {
    if !is_finite3(center_m) || !radius_m.is_finite() || radius_m < 0.0 {
        return false;
    }
    for plane in planes {
        let dist =
            plane[0] * center_m[0] + plane[1] * center_m[1] + plane[2] * center_m[2] + plane[3];
        if dist < -radius_m {
            return false;
        }
    }
    true
}

/// Flags for packed spheres `[x, y, z, radius]`: 1 visible, 0 culled, in
/// input order. Writes at most `flags.len()`, returns the count written.
/// No allocation; the caller reuses its buffer across frames.
pub fn cull_cpu_flags_into(
    spheres: &[[f32; 4]],
    planes: &[[f32; 4]; 6],
    flags: &mut [u8],
) -> usize {
    let n = spheres.len().min(flags.len());
    let mut i = 0;
    while i < n {
        let s = spheres[i];
        let visible = sphere_visible_cpu([s[0], s[1], s[2]], s[3], planes);
        flags[i] = u8::from(visible);
        i += 1;
    }
    n
}

/// Visible count over `cull_cpu_flags_into` output.
#[must_use]
pub fn visible_count(flags: &[u8]) -> u32 {
    let mut total: u32 = 0;
    for flag in flags {
        if *flag == 1 {
            total = total.saturating_add(1);
        }
    }
    total
}

/// Five-u32 indirect args for one instanced cube draw: 36 indices, one
/// instance per visible sphere, then zeros. No `MULTI_DRAW_INDIRECT_COUNT`,
/// so an iGPU without it still runs.
#[must_use]
pub fn indirect_args_5(visible: u32) -> [u32; 5] {
    [36, visible, 0, 0, 0]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(v: [f32; 3], s: f32) -> [f32; 3] {
    [v[0] * s, v[1] * s, v[2] * s]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn len(v: [f32; 3]) -> f32 {
    dot(v, v).sqrt()
}

fn is_finite3(v: [f32; 3]) -> bool {
    v[0].is_finite() && v[1].is_finite() && v[2].is_finite()
}

/// Orthonormal camera basis plus half-tangent optics, or the refusal.
struct Basis {
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
    tan_half_h: f32,
    tan_half_w: f32,
}

fn validate_camera(camera: &Camera) -> Result<Basis, DrawError> {
    if !is_finite3(camera.pos_m) {
        return Err(DrawError::BadCamera("pos_m is not finite".to_string()));
    }
    if !is_finite3(camera.forward) {
        return Err(DrawError::BadCamera("forward is not finite".to_string()));
    }
    if !is_finite3(camera.up) {
        return Err(DrawError::BadCamera("up is not finite".to_string()));
    }
    let f_len = len(camera.forward);
    let u_len = len(camera.up);
    if f_len <= 1e-6 {
        return Err(DrawError::BadCamera("forward is degenerate".to_string()));
    }
    if u_len <= 1e-6 {
        return Err(DrawError::BadCamera("up is degenerate".to_string()));
    }
    let cos = (dot(camera.forward, camera.up) / (f_len * u_len)).abs();
    if cos >= 1.0 - 1e-6 {
        return Err(DrawError::BadCamera(
            "up is parallel to forward".to_string(),
        ));
    }
    if !camera.fov_y_rad.is_finite()
        || camera.fov_y_rad <= 1e-3
        || camera.fov_y_rad >= std::f32::consts::PI - 1e-3
    {
        return Err(DrawError::BadCamera(
            "fov_y_rad is outside (0, pi)".to_string(),
        ));
    }
    if !camera.aspect.is_finite() || camera.aspect < 1e-3 || camera.aspect > 1e4 {
        return Err(DrawError::BadCamera(
            "aspect is outside [1e-3, 1e4]".to_string(),
        ));
    }
    if !camera.near_m.is_finite() || camera.near_m < MIN_NEAR_M {
        return Err(DrawError::BadCamera(
            "near_m must be finite and >= 1mm".to_string(),
        ));
    }
    if !camera.far_m.is_finite() || camera.far_m <= camera.near_m {
        return Err(DrawError::BadCamera(
            "far_m must be finite and above near_m".to_string(),
        ));
    }
    let forward = scale(camera.forward, 1.0 / f_len);
    let right = scale(
        cross3(forward, camera.up),
        1.0 / len(cross3(forward, camera.up)),
    );
    let up = cross3(right, forward);
    let tan_half_h = (camera.fov_y_rad * 0.5).tan();
    let tan_half_w = tan_half_h * camera.aspect;
    Ok(Basis {
        right,
        up,
        forward,
        tan_half_h,
        tan_half_w,
    })
}

fn validate_lod_table(table: &LodTable) -> Result<(), DrawError> {
    if !table.hysteresis_m.is_finite() || table.hysteresis_m < 0.0 {
        return Err(DrawError::BadLodTable(
            "hysteresis_m must be finite and non-negative".to_string(),
        ));
    }
    if table.bands_m.len() > usize::from(u8::MAX) {
        return Err(DrawError::BadLodTable(
            "more bands than LOD levels fit in u8".to_string(),
        ));
    }
    let mut prev = 0.0f32;
    for (i, band) in table.bands_m.iter().enumerate() {
        if !band.is_finite() || *band <= 0.0 {
            return Err(DrawError::BadLodTable(format!(
                "band {i} must be finite and positive"
            )));
        }
        if i > 0 && *band <= prev {
            return Err(DrawError::BadLodTable(format!(
                "band {i} must ascend above {prev}"
            )));
        }
        prev = *band;
    }
    if let Some(thresholds) = &table.pixel_thresholds_px {
        let mut prev = 0.0f32;
        for (i, threshold) in thresholds.iter().enumerate() {
            if !threshold.is_finite() || *threshold <= 0.0 {
                return Err(DrawError::BadLodTable(format!(
                    "pixel threshold {i} must be finite and positive"
                )));
            }
            if i > 0 && *threshold <= prev {
                return Err(DrawError::BadLodTable(format!(
                    "pixel threshold {i} must ascend above {prev}"
                )));
            }
            prev = *threshold;
        }
    }
    Ok(())
}

/// Six inward-facing frustum planes from a validated basis.
fn frustum_planes(camera: &Camera, basis: &Basis) -> [Plane; 6] {
    let through_origin = |normal: [f32; 3]| Plane {
        dist: -dot(normal, camera.pos_m),
        normal,
    };
    let edge_l = add(basis.forward, scale(basis.right, -basis.tan_half_w));
    let edge_r = add(basis.forward, scale(basis.right, basis.tan_half_w));
    let edge_b = add(basis.forward, scale(basis.up, -basis.tan_half_h));
    let edge_t = add(basis.forward, scale(basis.up, basis.tan_half_h));
    let unit = |v: [f32; 3]| scale(v, 1.0 / len(v));
    let near_point = add(camera.pos_m, scale(basis.forward, camera.near_m));
    let far_point = add(camera.pos_m, scale(basis.forward, camera.far_m));
    let near = Plane {
        normal: basis.forward,
        dist: -dot(basis.forward, near_point),
    };
    let far = Plane {
        normal: scale(basis.forward, -1.0),
        dist: dot(basis.forward, far_point),
    };
    [
        near,
        far,
        through_origin(unit(cross3(edge_l, basis.up))),
        through_origin(unit(cross3(basis.up, edge_r))),
        through_origin(unit(cross3(basis.right, edge_b))),
        through_origin(unit(cross3(edge_t, basis.right))),
    ]
}

/// Hysteresis LOD: from `prev`, coarsen only past `band + hyst` and
/// refine only below `band - hyst`, so jitter inside the band holds.
/// With zero hysteresis this equals the raw band level from any start.
fn pick_lod(dist: f32, prev: u8, levels: u8, bands: &[f32], hyst: f32) -> u8 {
    let max_lod = levels - 1;
    let usable = bands.len().min(max_lod as usize);
    let mut lod = prev.min(max_lod);
    while (lod as usize) < usable && dist > bands[lod as usize] + hyst {
        lod += 1;
    }
    while lod > 0 && dist < bands[lod as usize - 1] - hyst {
        lod -= 1;
    }
    lod
}

/// LOD level from a node's projected size on screen: meters in (`radius_m`,
/// `distance_m`), pixels out. `screen_h` is the frame height in pixels and
/// `tan_half_fov_y` is `tan(fov_y_rad / 2)` for that vertical field of view;
/// the projected diameter is
/// `2 * radius_m / (2 * distance_m * tan_half_fov_y) * screen_h`, then the
/// ascending `thresholds_px` are walked from the finest boundary down, the
/// mirrored form of [`pick_lod`]'s hysteresis: level `i` holds while the
/// size stays at or above `thresholds_px[len - 1 - i]`, so a node that
/// shrinks on screen lands on a coarser level. `prev` is honoured and the
/// pick caps at `levels`.
///
/// Errors: [`DrawError::BadPixelLod`] on a non-finite or negative
/// `radius_m`, a `distance_m` that is not above zero, a `screen_h` outside
/// 1..=65535 pixels, a non-finite `tan_half_fov_y <= 0`, `levels` of zero, a
/// negative or non-finite `hyst_px`, or an empty `thresholds_px`. No caller
/// yet: the pick in [`cull_lod`] carries no frame height in its call chain.
#[expect(clippy::too_many_arguments, reason = "8 inputs mirror the pick")]
pub fn lod_by_pixel_error(
    radius_m: f32,
    distance_m: f32,
    screen_h: u32,
    tan_half_fov_y: f32,
    levels: u8,
    prev: u8,
    thresholds_px: &[f32],
    hyst_px: f32,
) -> Result<u8, DrawError> {
    let bad = |why: &str| DrawError::BadPixelLod(why.to_string());
    if !radius_m.is_finite() || radius_m < 0.0 {
        return Err(bad("radius_m must be finite and non-negative"));
    }
    if !distance_m.is_finite() || distance_m <= 0.0 {
        return Err(bad("distance_m must be finite and positive"));
    }
    if screen_h == 0 || screen_h > u32::from(u16::MAX) {
        return Err(bad("screen_h must be 1..=65535 pixels"));
    }
    if !tan_half_fov_y.is_finite() || tan_half_fov_y <= 0.0 {
        return Err(bad("tan_half_fov_y must be finite and positive"));
    }
    if levels == 0 {
        return Err(bad("levels must be at least 1"));
    }
    if !hyst_px.is_finite() || hyst_px < 0.0 {
        return Err(bad("hyst_px must be finite and non-negative"));
    }
    if thresholds_px.is_empty() {
        return Err(bad("thresholds_px must not be empty"));
    }
    let screen_h_px = f32::from(u16::try_from(screen_h).unwrap_or(u16::MAX));
    let size_px = 2.0 * radius_m / (2.0 * distance_m * tan_half_fov_y) * screen_h_px;
    // `pick_lod`'s rule mirrored: the level index walks down the ascending
    // boundaries as the size shrinks.
    let len = thresholds_px.len();
    let max_lod = levels - 1;
    let usable = len.min(usize::from(max_lod));
    let mut lod = prev.min(max_lod);
    let edge = |step: usize| thresholds_px[len - 1 - step];
    while usize::from(lod) < usable && size_px < edge(usize::from(lod)) - hyst_px {
        lod += 1;
    }
    while lod > 0 && usize::from(lod) <= len && size_px > edge(usize::from(lod) - 1) + hyst_px {
        lod -= 1;
    }
    Ok(lod)
}

/// Dither keep share for one draw's outgoing LOD edge: 255 fully kept.
///
/// The switch is the next coarsening boundary past `lod` and the band is
/// twice the table hysteresis, so the share dissolves across the same
/// meters the hysteresis holds the level. The coarsest level has no
/// outgoing edge and stays 255, as does a table with no band to read.
/// The forward pass thresholds pixels against this share, so a fading LOD
/// dissolves instead of popping. Calls [`crate::lod_fade::fade`].
#[must_use]
pub fn fade_for_lod(distance_m: f32, lod: u8, table: &LodTable) -> u8 {
    match table.bands_m.get(usize::from(lod)) {
        Some(switch_m) => lod_fade::fade(distance_m, *switch_m, table.hysteresis_m * 2.0),
        None => 255,
    }
}

/// One weapon-trail strip vertex: position plus age fade.
///
/// Consumer: the next slice's `present/**` strip draw reads these verts.
/// Units: meters for `pos_m`, unit share 0..=1 for `alpha`. Determinism:
/// pure data, so identical slices give identical verts x2. Errors: none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrailStripVert {
    /// Strip position in meters.
    pub pos_m: [f32; 3],
    /// Opacity share 0..=1, `1 - age/11` clamped.
    pub alpha: f32,
}

/// Build weapon-trail strip verts from oldest-first base/tip pairs.
///
/// One base vert plus one tip vert per pair in input order, so the output
/// feeds a triangle strip directly. Alpha is `1 - age/11` clamped 0..=1,
/// the same fade the game ring uses. Empty input gives empty out;
/// non-finite coordinates are skipped, never stored; input past 12 pairs
/// is capped, so the output holds at most 24 verts. Pure and deterministic
/// x2; the one small bounded allocation is the returned strip.
#[must_use]
pub fn trail_strip_verts(
    base: &[[f32; 3]],
    tip: &[[f32; 3]],
    age_ticks: &[u8],
) -> Vec<TrailStripVert> {
    let pairs = base.len().min(tip.len()).min(age_ticks.len()).min(12);
    let mut out = Vec::with_capacity(pairs.saturating_mul(2));
    let mut i = 0;
    while i < pairs {
        let b = base[i];
        let t = tip[i];
        if is_finite3(b) && is_finite3(t) {
            let alpha = (1.0 - f32::from(age_ticks[i]) / 11.0).clamp(0.0, 1.0);
            out.push(TrailStripVert { pos_m: b, alpha });
            out.push(TrailStripVert { pos_m: t, alpha });
        }
        i += 1;
    }
    out
}

/// One node through validation, frustum cull and LOD pick; `None` means
/// culled. Shared by the per-draw list and the instanced grouping so the
/// two never drift apart.
fn cull_lod(
    node: &SceneNode,
    planes: &[Plane; 6],
    camera_pos_m: [f32; 3],
    lod_table: &LodTable,
) -> Result<Option<Draw>, DrawError> {
    if !is_finite3(node.center_m) {
        return Err(DrawError::BadNode {
            id: node.id,
            reason: "center_m is not finite".to_string(),
        });
    }
    if !node.radius_m.is_finite() || node.radius_m < 0.0 {
        return Err(DrawError::BadNode {
            id: node.id,
            reason: "radius_m must be finite and non-negative".to_string(),
        });
    }
    if node.lod_levels < 1 {
        return Err(DrawError::BadNode {
            id: node.id,
            reason: "lod_levels must be at least 1".to_string(),
        });
    }
    if node.prev_lod >= node.lod_levels {
        return Err(DrawError::BadNode {
            id: node.id,
            reason: "prev_lod is outside lod_levels".to_string(),
        });
    }
    if node.lod_table != lod_table.id {
        return Err(DrawError::UnknownLodTable {
            node: node.id,
            expected: lod_table.id,
            found: node.lod_table,
        });
    }
    let offset = sub(node.center_m, camera_pos_m);
    let distance_m = len(offset);
    let packed = planes_to_packed(planes);
    // One rule: the batched oracle decides, so per-node and GPU batches agree.
    let sphere = [[
        node.center_m[0],
        node.center_m[1],
        node.center_m[2],
        node.radius_m,
    ]];
    let mut flag = [0u8; 1];
    cull_cpu_flags_into(&sphere, &packed, &mut flag);
    if visible_count(&flag) == 0 {
        return Ok(None);
    }
    let lod = pick_lod(
        distance_m,
        node.prev_lod,
        node.lod_levels,
        &lod_table.bands_m,
        lod_table.hysteresis_m,
    );
    let fade = fade_for_lod(distance_m, lod, lod_table);
    let depth_bucket = (distance_m * MM_PER_M).max(0.0) as u32;
    Ok(Some(Draw {
        id: node.id,
        mesh: node.mesh,
        lod,
        distance_m,
        depth_bucket,
        fade,
    }))
}

/// Reusable draw list; reuse across frames to hold the buffer warm.
#[derive(Clone, Debug, Default)]
pub struct DrawList {
    draws: Vec<Draw>,
}

impl DrawList {
    /// Empty list with no allocation.
    pub fn new() -> Self {
        Self { draws: Vec::new() }
    }

    /// Visible draws in integer-stable `(depth_bucket, id)` order.
    pub fn draws(&self) -> &[Draw] {
        &self.draws
    }

    /// Reserved buffer capacity; steady-state rebuilds must not move it.
    pub fn capacity(&self) -> usize {
        self.draws.capacity()
    }

    /// FNV-1a digest over `(id, lod, depth_bucket)` in output order;
    /// equal bytes twice prove the order is bit-stable.
    pub fn order_digest(&self) -> u64 {
        let mut hash: u64 = engine_core::hash::FNV64_OFFSET;
        for draw in &self.draws {
            for word in [draw.id, draw.depth_bucket as u64, draw.lod as u64] {
                for byte in word.to_le_bytes() {
                    hash ^= byte as u64;
                    hash = hash.wrapping_mul(engine_core::hash::FNV64_PRIME);
                }
            }
        }
        hash
    }

    /// Cull, LOD-pick and integer-sort one snapshot. The buffer is
    /// cleared and refilled in place, so steady-state frames allocate
    /// nothing once warm. Over-cap and unknown tables are loud errors
    /// that leave the list empty, never a silent truncation.
    pub fn build(
        &mut self,
        snapshot: &SceneSnapshot,
        camera: &Camera,
        lod_table: &LodTable,
    ) -> Result<&[Draw], DrawError> {
        let basis = validate_camera(camera)?;
        validate_lod_table(lod_table)?;
        let planes = frustum_planes(camera, &basis);
        self.draws.clear();
        // Warm the buffer once toward the cap; the over-cap push path
        // below only ever allocates on the loud error path.
        if self.draws.capacity() < snapshot.nodes.len().min(MAX_DRAWS + 1) {
            self.draws
                .reserve(snapshot.nodes.len().min(MAX_DRAWS + 1) - self.draws.capacity());
        }
        for node in &snapshot.nodes {
            if let Some(draw) = cull_lod(node, &planes, camera.pos_m, lod_table)? {
                self.draws.push(draw);
            }
        }
        if self.draws.len() > MAX_DRAWS {
            let visible = self.draws.len();
            self.draws.clear();
            return Err(DrawError::OverCap {
                visible,
                cap: MAX_DRAWS,
            });
        }
        // Stable integer sort: input order never decides output order.
        self.draws.sort_by_key(|a| (a.depth_bucket, a.id));
        // Live GPU wiring: the executor builds its compute pipeline from
        // `CULL_WGSL` and its 5-u32 args from this list's visible count.
        // `len` fits `u32`: the over-cap return above holds 256 max.
        let visible = u32::try_from(self.draws.len()).unwrap_or(u32::MAX);
        let indirect = indirect_args_5(visible);
        assert!(
            CULL_WGSL.contains("atomicAdd")
                && CULL_WGSL.contains("-radius")
                && indirect[0] == 36
                && indirect[1] == visible,
            "shaders/cull.wgsl must keep the -radius rule and visible count"
        );
        Ok(&self.draws)
    }
}

/// How many squads and instances one instanced frame may carry; the submit
/// path sizes its per-squad storage from this budget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstancedBudget {
    /// Most `(mesh, lod)` squads per frame.
    pub max_groups: usize,
    /// Most instances inside one squad.
    pub max_instances_per_group: usize,
}

impl Default for InstancedBudget {
    /// The frame budgets beside the per-draw cap.
    fn default() -> Self {
        Self {
            max_groups: MAX_INSTANCED_GROUPS,
            max_instances_per_group: MAX_INSTANCES_PER_GROUP,
        }
    }
}

/// One mesh/LOD squad: every visible instance sharing mesh and LOD level
/// submits as a single instanced draw beside the per-draw cap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstancedDraw {
    /// Stable mesh identity the executor resolves through the GPU cache.
    pub mesh: u64,
    /// LOD level shared by the whole squad.
    pub lod: u8,
    /// Instances in the squad.
    pub count: usize,
    /// Nearest member in millimeters, saturated at `u32::MAX`; the primary
    /// sort key beside mesh and LOD.
    pub depth_bucket: u32,
    /// Digest over member ids in `(depth_bucket, id)` order; equal bytes
    /// twice prove the grouping is bit-stable.
    pub ids_digest: u64,
}

/// Reusable instanced draw list; reuse across frames to hold both buffers
/// warm. The scratch buffer holds one entry per visible instance before
/// grouping, so it settles at the storm working set, not the draw cap.
#[derive(Clone, Debug, Default)]
pub struct InstancedList {
    groups: Vec<InstancedDraw>,
    scratch: Vec<Draw>,
}

impl InstancedList {
    /// Empty list with no allocation.
    pub fn new() -> Self {
        Self {
            groups: Vec::new(),
            scratch: Vec::new(),
        }
    }

    /// Squads in integer-stable `(depth_bucket, mesh, lod)` order.
    pub fn groups(&self) -> &[InstancedDraw] {
        &self.groups
    }

    /// Reserved group capacity; steady-state rebuilds must not move it.
    pub fn capacity(&self) -> usize {
        self.groups.capacity()
    }

    /// Digest over the squads in output order; equal bytes twice prove the
    /// order is bit-stable. The member count stays out: it is derived from
    /// the same members the id digest already covers.
    pub fn order_digest(&self) -> u64 {
        let mut hash = Fnv64::new();
        for group in &self.groups {
            hash.write_u64(group.mesh);
            hash.write_u32(group.depth_bucket);
            hash.write_u8(group.lod);
            hash.write_u64(group.ids_digest);
        }
        hash.finish()
    }

    /// Cull, LOD-pick and group one snapshot into squads. Both buffers are
    /// cleared and refilled in place, so steady-state frames allocate
    /// nothing once warm. Over-budget squads, oversized squads and unknown
    /// tables are loud errors that leave the list empty, never a silent
    /// truncation.
    pub fn build(
        &mut self,
        snapshot: &SceneSnapshot,
        camera: &Camera,
        lod_table: &LodTable,
        budget: InstancedBudget,
    ) -> Result<&[InstancedDraw], DrawError> {
        let basis = validate_camera(camera)?;
        validate_lod_table(lod_table)?;
        let planes = frustum_planes(camera, &basis);
        self.scratch.clear();
        self.groups.clear();
        if self.scratch.capacity() < snapshot.nodes.len() {
            self.scratch
                .reserve(snapshot.nodes.len() - self.scratch.capacity());
        }
        for node in &snapshot.nodes {
            if let Some(draw) = cull_lod(node, &planes, camera.pos_m, lod_table)? {
                self.scratch.push(draw);
            }
        }
        // Integer-stable grouping: squad order never follows input order.
        self.scratch
            .sort_by_key(|a| (a.mesh, a.lod, a.depth_bucket, a.id));
        let mut index = 0;
        while index < self.scratch.len() {
            let head = self.scratch[index];
            let mut digest = Fnv64::new();
            let mut cursor = index;
            // Members arrive nearest-first, so ids enter the digest in
            // `(depth_bucket, id)` order.
            while cursor < self.scratch.len()
                && self.scratch[cursor].mesh == head.mesh
                && self.scratch[cursor].lod == head.lod
            {
                digest.write_u64(self.scratch[cursor].id);
                cursor += 1;
            }
            let count = cursor - index;
            if count > budget.max_instances_per_group {
                self.scratch.clear();
                self.groups.clear();
                return Err(DrawError::OverInstances {
                    mesh: head.mesh,
                    lod: head.lod,
                    count,
                    cap: budget.max_instances_per_group,
                });
            }
            self.groups.push(InstancedDraw {
                mesh: head.mesh,
                lod: head.lod,
                count,
                depth_bucket: head.depth_bucket,
                ids_digest: digest.finish(),
            });
            index = cursor;
        }
        if self.groups.len() > budget.max_groups {
            let groups = self.groups.len();
            self.scratch.clear();
            self.groups.clear();
            return Err(DrawError::OverInstancedCap {
                groups,
                cap: budget.max_groups,
            });
        }
        // Nearest squad first; mesh and LOD break depth ties deterministically.
        self.groups.sort_by_key(|a| (a.depth_bucket, a.mesh, a.lod));
        Ok(&self.groups)
    }
}

/// Seeded draw-storm fixture: the scale proof input. One thousand agents in
/// eight mesh squads around a fixed camera; nine of ten face the lens and
/// every tenth sits behind it, so the cull path always has work to do.
/// Same seed gives identical positions on every platform: integer streams
/// from one forked generator, `+ - * /` float math only.
pub struct StormFixture {
    /// One thousand agents in build order (never the output order).
    pub snapshot: SceneSnapshot,
    /// Fixed 90-degree camera at the origin looking down `-z`.
    pub camera: Camera,
    /// Three bands at 10/20/40 m with a half-meter hysteresis.
    pub lod_table: LodTable,
}

/// Uniform meters in `[0, span)` from an integer stream; the single float
/// scaling keeps fixture bytes identical on every platform.
fn span_m(stream: &mut Rng, span: f32) -> f32 {
    // Uniform [0, 1): fix the exponent to 2^0 and draw the 23 mantissa
    // bits from the stream, so no float cast ever appears.
    let unit = f32::from_bits(0x3F80_0000 | (stream.next_u32() >> 9)) - 1.0;
    unit * span
}

/// Build the storm fixture for `seed`. Nine of ten agents stand in a hall
/// ahead of the lens; every tenth stands behind the camera and must cull
/// away, leaving 900 visible instances over the loud per-draw cap.
pub fn storm_fixture(seed: u64) -> StormFixture {
    let base = Rng::new(seed);
    let mut nodes = Vec::with_capacity(STORM_AGENTS);
    for agent in 0..STORM_AGENTS {
        // One forked stream per agent, so seeds never alias each other.
        // The index stays below 1,000: `usize` always fits `u64`, so the
        // fallback below is dead on every platform and panics never appear.
        let i = u64::try_from(agent).unwrap_or(u64::MAX);
        let mut stream = base.fork(i);
        let mesh = stream.next_u64() % 8;
        let hall = agent % 10 != 9;
        let x_m = span_m(&mut stream, 60.0) - 30.0;
        let y_m = span_m(&mut stream, 10.0) - 5.0;
        let z_m = if hall {
            -(span_m(&mut stream, 40.0) + 20.0)
        } else {
            span_m(&mut stream, 20.0) + 10.0
        };
        let center_m = [x_m, y_m, z_m];
        let distance_m = len(sub(center_m, [0.0, 0.0, 0.0]));
        // Raw band level holds steady under the table hysteresis by
        // construction, so the fixture never flickers between runs.
        let prev_lod = pick_lod(distance_m, 0, 4, &[10.0, 20.0, 40.0], 0.0);
        nodes.push(SceneNode {
            id: i,
            mesh,
            center_m,
            radius_m: 0.5,
            lod_levels: 4,
            lod_table: 7,
            prev_lod,
        });
    }
    StormFixture {
        snapshot: SceneSnapshot { nodes },
        camera: Camera {
            pos_m: [0.0, 0.0, 0.0],
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            fov_y_rad: std::f32::consts::FRAC_PI_2,
            aspect: 16.0 / 9.0,
            near_m: 0.1,
            far_m: 100.0,
        },
        lod_table: LodTable {
            id: 7,
            bands_m: vec![10.0, 20.0, 40.0],
            hysteresis_m: 0.5,
            pixel_thresholds_px: None,
        },
    }
}

/// Spectator seat layout lives in the `crowd_seats` child so the reviewed
/// GS18 cull oracle stays alone in this file.
/// Caller: `crates/render/src/present/gpu_present/submit.rs` reads seats
/// through this re-export, so no caller edits are needed.
#[path = "crowd_seats.rs"]
pub mod crowd_seats;

pub use crowd_seats::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn test_camera() -> Camera {
        Camera {
            pos_m: [0.0, 0.0, 0.0],
            forward: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            fov_y_rad: std::f32::consts::FRAC_PI_2,
            aspect: 16.0 / 9.0,
            near_m: 0.1,
            far_m: 100.0,
        }
    }

    fn test_table() -> LodTable {
        LodTable {
            id: 7,
            bands_m: vec![10.0, 20.0, 40.0],
            hysteresis_m: 0.5,
            pixel_thresholds_px: None,
        }
    }

    fn node(id: u64, center_m: [f32; 3], radius_m: f32) -> SceneNode {
        SceneNode {
            id,
            mesh: id + 1000,
            center_m,
            radius_m,
            lod_levels: 4,
            lod_table: 7,
            prev_lod: 0,
        }
    }

    #[test]
    fn ahead_is_visible_behind_and_side_are_culled() {
        let camera = test_camera();
        let table = test_table();
        let snapshot = SceneSnapshot {
            nodes: vec![
                node(1, [0.0, 0.0, -10.0], 1.0),
                node(2, [0.0, 0.0, 10.0], 1.0),
                node(3, [40.0, 0.0, -10.0], 1.0),
                node(4, [0.0, 30.0, -10.0], 1.0),
            ],
        };
        let mut list = DrawList::new();
        let draws = list.build(&snapshot, &camera, &table).expect("builds");
        assert_eq!(draws.len(), 1);
        assert_eq!(draws[0].id, 1);
    }

    #[test]
    fn near_and_far_planes_cull() {
        let camera = test_camera();
        let table = test_table();
        let snapshot = SceneSnapshot {
            nodes: vec![
                node(1, [0.0, 0.0, -0.05], 0.01),
                node(2, [0.0, 0.0, -150.0], 1.0),
                node(3, [0.0, 0.0, -50.0], 1.0),
            ],
        };
        let mut list = DrawList::new();
        let draws = list.build(&snapshot, &camera, &table).expect("builds");
        assert_eq!(draws.len(), 1);
        assert_eq!(draws[0].id, 3);
    }

    #[test]
    fn touching_sphere_is_kept() {
        let camera = test_camera();
        let table = test_table();
        let snapshot = SceneSnapshot {
            nodes: vec![node(1, [0.0, 0.0, -101.0], 2.0)],
        };
        let mut list = DrawList::new();
        let draws = list.build(&snapshot, &camera, &table).expect("builds");
        assert_eq!(draws.len(), 1);
    }

    #[test]
    fn zero_hysteresis_equals_raw_band_level() {
        let table = LodTable {
            id: 7,
            bands_m: vec![10.0, 20.0, 40.0],
            hysteresis_m: 0.0,
            pixel_thresholds_px: None,
        };
        for (dist, raw) in [(5.0, 0), (10.01, 1), (19.99, 1), (20.01, 2), (50.0, 3)] {
            for prev in 0..4u8 {
                assert_eq!(pick_lod(dist, prev, 4, &table.bands_m, 0.0), raw);
            }
        }
    }

    #[test]
    fn lod_levels_cap_the_table() {
        let table = test_table();
        assert_eq!(pick_lod(100.0, 0, 2, &table.bands_m, table.hysteresis_m), 1);
        assert_eq!(pick_lod(100.0, 0, 1, &table.bands_m, table.hysteresis_m), 0);
    }

    #[test]
    fn jitter_inside_band_never_flickers_300_frames() {
        let camera = test_camera();
        let table = test_table();
        let mut list = DrawList::new();
        // From below the 10 m boundary and from above it: jitter of
        // +/-0.3 m stays inside the 1.0 m hysteresis band either way.
        for start_prev in [0u8, 1u8] {
            let mut prev = start_prev;
            for frame in 0..300u32 {
                let step = frame.wrapping_mul(2654435761).wrapping_shr(8) % 61;
                let jitter = step as f32 / 100.0 - 0.30;
                let mut n = node(1, [0.0, 0.0, -10.0 - jitter], 0.5);
                n.prev_lod = prev;
                let snapshot = SceneSnapshot { nodes: vec![n] };
                let draws = list.build(&snapshot, &camera, &table).expect("builds");
                assert_eq!(draws.len(), 1, "frame {frame} culled");
                assert_eq!(
                    draws[0].lod, start_prev,
                    "flicker at frame {frame} from {start_prev}"
                );
                prev = draws[0].lod;
            }
        }
    }

    #[test]
    fn clear_band_exit_switches_once_then_holds() {
        let camera = test_camera();
        let table = test_table();
        let mut list = DrawList::new();
        let mut prev = 0u8;
        let mut switches = 0u32;
        for frame in 0..60u32 {
            // Walk from 5 m to 16 m: exactly one 0 -> 1 crossing.
            let dist = 5.0 + frame as f32 * (11.0 / 59.0);
            let mut n = node(1, [0.0, 0.0, -dist], 0.5);
            n.prev_lod = prev;
            let snapshot = SceneSnapshot { nodes: vec![n] };
            let draws = list.build(&snapshot, &camera, &table).expect("builds");
            if draws[0].lod != prev {
                switches += 1;
                assert_eq!((prev, draws[0].lod), (0, 1));
                // The switch lands past band + hyst, never early.
                assert!(dist > 10.5, "switched early at {dist}");
                prev = draws[0].lod;
            }
        }
        assert_eq!((switches, prev), (1, 1));
    }

    #[test]
    fn fade_share_dissolves_across_the_band() {
        let camera = test_camera();
        let table = test_table();
        let mut list = DrawList::new();
        // Well inside LOD 0 (switch at 10 m, band twice the 0.5 m
        // hysteresis): fully kept.
        let near = SceneSnapshot {
            nodes: vec![node(1, [0.0, 0.0, -5.0], 0.5)],
        };
        let draws = list.build(&near, &camera, &table).expect("builds");
        assert_eq!(draws[0].lod, 0);
        assert_eq!(draws[0].fade, 255);
        // At the 10 m switch the outgoing LOD is half dissolved.
        let mid = SceneSnapshot {
            nodes: vec![node(2, [0.0, 0.0, -10.0], 0.5)],
        };
        let draws = list.build(&mid, &camera, &table).expect("builds");
        assert_eq!(draws[0].lod, 0);
        assert_eq!(draws[0].fade, crate::lod_fade::fade(10.0, 10.0, 1.0));
        assert_eq!(draws[0].fade, 128);
        // Held past the switch by hysteresis: dissolving, not popped.
        let past = SceneSnapshot {
            nodes: vec![node(3, [0.0, 0.0, -10.1], 0.5)],
        };
        let draws = list.build(&past, &camera, &table).expect("builds");
        assert_eq!(draws[0].lod, 0);
        assert!(
            (1..128).contains(&draws[0].fade),
            "held LOD past the switch must dissolve, got {}",
            draws[0].fade
        );
        // Same bytes twice prove the share is bit-stable.
        let first = draws[0].fade;
        let redrawn = list.build(&past, &camera, &table).expect("rebuilds");
        assert_eq!(redrawn[0].fade, first);
    }

    #[test]
    fn coarsest_lod_never_dissolves_and_zero_band_is_a_hard_switch() {
        let camera = test_camera();
        let table = test_table();
        let mut list = DrawList::new();
        // Far past the last band the coarsest LOD stays fully kept: there
        // is no outgoing edge to dissolve into.
        let far = SceneSnapshot {
            nodes: vec![node(1, [0.0, 0.0, -90.0], 0.5)],
        };
        let draws = list.build(&far, &camera, &table).expect("builds");
        assert_eq!(draws[0].lod, 3);
        assert_eq!(draws[0].fade, 255);
        // Zero hysteresis is a hard switch: kept below, next level above.
        let hard = LodTable {
            id: 7,
            bands_m: vec![10.0, 20.0, 40.0],
            hysteresis_m: 0.0,
            pixel_thresholds_px: None,
        };
        let below = SceneSnapshot {
            nodes: vec![node(2, [0.0, 0.0, -9.9], 0.5)],
        };
        let draws = list.build(&below, &camera, &hard).expect("builds");
        assert_eq!((draws[0].lod, draws[0].fade), (0, 255));
        let above = SceneSnapshot {
            nodes: vec![node(3, [0.0, 0.0, -10.1], 0.5)],
        };
        let draws = list.build(&above, &camera, &hard).expect("builds");
        assert_eq!((draws[0].lod, draws[0].fade), (1, 255));
        // A single LOD level has no band to read: fully kept, never panics.
        let single = LodTable {
            id: 9,
            bands_m: vec![],
            hysteresis_m: 0.0,
            pixel_thresholds_px: None,
        };
        let mut only = node(4, [0.0, 0.0, -5.0], 0.5);
        only.lod_levels = 1;
        only.lod_table = 9;
        let one = SceneSnapshot { nodes: vec![only] };
        let draws = list.build(&one, &camera, &single).expect("builds");
        assert_eq!((draws[0].lod, draws[0].fade), (0, 255));
    }

    #[test]
    fn order_is_input_permutation_stable() {
        let camera = test_camera();
        let table = test_table();
        let base: Vec<SceneNode> = (0..64u64)
            .map(|i| {
                let mut n = node(
                    1000 - i,
                    [((i * 37) % 17) as f32 - 8.0, 0.0, -5.0 - (i % 9) as f32],
                    0.5,
                );
                n.prev_lod = (i % 4) as u8;
                n
            })
            .collect();
        let mut first_bytes = Vec::new();
        let mut first_digest = 0u64;
        for round in 0..3 {
            // Deterministic rotation of the input order each round.
            let mut nodes = base.clone();
            let shift = (round * 17) % nodes.len().max(1);
            nodes.rotate_left(shift);
            let snapshot = SceneSnapshot { nodes };
            let mut list = DrawList::new();
            let draws = list.build(&snapshot, &camera, &table).expect("builds");
            let bytes: Vec<(u64, u64, u32, u8)> = draws
                .iter()
                .map(|d| (d.id, d.mesh, d.depth_bucket, d.lod))
                .collect();
            // Keys actually ascend.
            let keys: Vec<(u32, u64)> = draws.iter().map(|d| (d.depth_bucket, d.id)).collect();
            let mut sorted = keys.clone();
            sorted.sort_unstable();
            assert_eq!(keys, sorted);
            let digest = list.order_digest();
            if round == 0 {
                first_bytes = bytes;
                first_digest = digest;
            } else {
                assert_eq!(bytes, first_bytes, "order moved on round {round}");
                assert_eq!(digest, first_digest);
            }
        }
    }

    #[test]
    fn nearer_draw_sorts_first_and_id_breaks_ties() {
        let camera = test_camera();
        let table = test_table();
        let snapshot = SceneSnapshot {
            nodes: vec![
                node(9, [0.0, 0.0, -30.0], 0.5),
                node(3, [0.0, 0.0, -30.0], 0.5),
                node(5, [0.0, 0.0, -5.0], 0.5),
            ],
        };
        let mut list = DrawList::new();
        let draws = list.build(&snapshot, &camera, &table).expect("builds");
        assert_eq!([draws[0].id, draws[1].id, draws[2].id], [5, 3, 9]);
    }

    #[test]
    fn exactly_at_cap_builds_over_cap_refuses_loudly() {
        let camera = test_camera();
        let table = test_table();
        let grid = |count: usize| SceneSnapshot {
            nodes: (0..count as u64)
                .map(|i| {
                    node(
                        i,
                        [(i % 16) as f32 - 7.5, (i / 16 % 16) as f32 - 7.5, -30.0],
                        0.5,
                    )
                })
                .collect(),
        };
        let mut list = DrawList::new();
        let draws = list
            .build(&grid(MAX_DRAWS), &camera, &table)
            .expect("256 fits");
        assert_eq!(draws.len(), MAX_DRAWS);
        let err = list
            .build(&grid(MAX_DRAWS + 1), &camera, &table)
            .expect_err("257 must refuse");
        assert_eq!(
            err,
            DrawError::OverCap {
                visible: MAX_DRAWS + 1,
                cap: MAX_DRAWS
            }
        );
        assert!(list.draws().is_empty(), "error leaves no partial list");
    }

    #[test]
    fn bad_cameras_are_refused() {
        let table = test_table();
        let snapshot = SceneSnapshot {
            nodes: vec![node(1, [0.0, 0.0, -5.0], 0.5)],
        };
        let mut list = DrawList::new();
        let mut bad = test_camera();
        bad.fov_y_rad = 0.0;
        assert!(matches!(
            list.build(&snapshot, &bad, &table),
            Err(DrawError::BadCamera(_))
        ));
        let mut bad = test_camera();
        bad.near_m = 10.0;
        bad.far_m = 5.0;
        assert!(matches!(
            list.build(&snapshot, &bad, &table),
            Err(DrawError::BadCamera(_))
        ));
        let mut bad = test_camera();
        bad.forward = [0.0, 1.0, 0.0];
        assert!(matches!(
            list.build(&snapshot, &bad, &table),
            Err(DrawError::BadCamera(_))
        ));
        let mut bad = test_camera();
        bad.forward = [f32::NAN, 0.0, -1.0];
        assert!(matches!(
            list.build(&snapshot, &bad, &table),
            Err(DrawError::BadCamera(_))
        ));
        let mut bad = test_camera();
        bad.aspect = 0.0;
        assert!(matches!(
            list.build(&snapshot, &bad, &table),
            Err(DrawError::BadCamera(_))
        ));
    }

    #[test]
    fn unknown_lod_table_is_refused() {
        let camera = test_camera();
        let table = test_table();
        let mut n = node(1, [0.0, 0.0, -5.0], 0.5);
        n.lod_table = 999;
        let snapshot = SceneSnapshot { nodes: vec![n] };
        let mut list = DrawList::new();
        assert_eq!(
            list.build(&snapshot, &camera, &table),
            Err(DrawError::UnknownLodTable {
                node: 1,
                expected: 7,
                found: 999
            })
        );
    }

    #[test]
    fn bad_nodes_and_tables_are_refused() {
        let camera = test_camera();
        let table = test_table();
        let mut list = DrawList::new();
        let mut n = node(1, [f32::NAN, 0.0, -5.0], 0.5);
        let snapshot = SceneSnapshot {
            nodes: vec![n.clone()],
        };
        assert!(matches!(
            list.build(&snapshot, &camera, &table),
            Err(DrawError::BadNode { .. })
        ));
        n = node(2, [0.0, 0.0, -5.0], -1.0);
        let snapshot = SceneSnapshot { nodes: vec![n] };
        assert!(matches!(
            list.build(&snapshot, &camera, &table),
            Err(DrawError::BadNode { .. })
        ));
        let mut n = node(3, [0.0, 0.0, -5.0], 0.5);
        n.prev_lod = 4;
        let snapshot = SceneSnapshot { nodes: vec![n] };
        assert!(matches!(
            list.build(&snapshot, &camera, &table),
            Err(DrawError::BadNode { .. })
        ));
        let bad_table = LodTable {
            id: 7,
            bands_m: vec![20.0, 10.0],
            hysteresis_m: 0.5,
            pixel_thresholds_px: None,
        };
        let snapshot = SceneSnapshot {
            nodes: vec![node(1, [0.0, 0.0, -5.0], 0.5)],
        };
        assert!(matches!(
            list.build(&snapshot, &camera, &bad_table),
            Err(DrawError::BadLodTable(_))
        ));
    }

    #[test]
    fn steady_state_reuses_the_buffer() {
        let camera = test_camera();
        let table = test_table();
        let snapshot = SceneSnapshot {
            nodes: (0..32u64)
                .map(|i| node(i, [(i % 8) as f32 - 3.5, 0.0, -8.0 - (i / 8) as f32], 0.5))
                .collect(),
        };
        let mut list = DrawList::new();
        list.build(&snapshot, &camera, &table).expect("builds");
        // Borrow ends before the next build; capacity must not move.
        let (len, cap, digest) = (list.draws().len(), list.capacity(), list.order_digest());
        list.build(&snapshot, &camera, &table).expect("rebuilds");
        assert_eq!(list.draws().len(), len);
        assert_eq!(list.capacity(), cap);
        assert_eq!(list.order_digest(), digest);
    }

    #[test]
    fn error_display_names_the_fix() {
        assert!(format!("{}", DrawError::BadCamera("x".to_string())).contains("bad camera"));
        assert!(format!(
            "{}",
            DrawError::OverCap {
                visible: 300,
                cap: 256
            }
        )
        .contains("300"));
        assert!(format!(
            "{}",
            DrawError::OverInstancedCap {
                groups: 300,
                cap: 256
            }
        )
        .contains("300"));
        assert!(format!(
            "{}",
            DrawError::OverInstances {
                mesh: 42,
                lod: 1,
                count: 99,
                cap: 64
            }
        )
        .contains("99"));
    }

    fn storm() -> StormFixture {
        storm_fixture(2040)
    }

    fn visible_agents() -> usize {
        // Every tenth agent stands behind the lens and must cull away.
        STORM_AGENTS - STORM_AGENTS / 10
    }

    #[test]
    fn storm_has_a_thousand_agents() {
        let fixture = storm();
        assert_eq!(fixture.snapshot.nodes.len(), STORM_AGENTS);
        assert_eq!(fixture.lod_table.bands_m, vec![10.0, 20.0, 40.0]);
    }

    #[test]
    fn storm_keeps_the_loud_per_draw_gate() {
        let fixture = storm();
        let mut list = DrawList::new();
        assert_eq!(
            list.build(&fixture.snapshot, &fixture.camera, &fixture.lod_table),
            Err(DrawError::OverCap {
                visible: visible_agents(),
                cap: MAX_DRAWS
            })
        );
        assert!(list.draws().is_empty(), "error leaves no partial list");
    }

    #[test]
    fn storm_groups_share_mesh_and_lod() {
        let fixture = storm();
        let mut list = InstancedList::new();
        let groups = list
            .build(
                &fixture.snapshot,
                &fixture.camera,
                &fixture.lod_table,
                InstancedBudget::default(),
            )
            .expect("storm fits the instancing budget");
        // Every visible instance lands in exactly one squad.
        let total: usize = groups.iter().map(|g| g.count).sum();
        assert_eq!(total, visible_agents());
        assert!(!groups.is_empty(), "storm must form squads");
        // One instanced draw per squad: mesh/LOD pairs never repeat.
        let mut pairs: Vec<(u64, u8)> = groups.iter().map(|g| (g.mesh, g.lod)).collect();
        pairs.sort_unstable();
        pairs.dedup();
        assert_eq!(pairs.len(), groups.len());
        // Nearest squad first; mesh and LOD break depth ties.
        let keys: Vec<(u32, u64, u8)> = groups
            .iter()
            .map(|g| (g.depth_bucket, g.mesh, g.lod))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted);
        // Equal bytes twice prove the grouping is bit-stable.
        let digest = list.order_digest();
        list.build(
            &fixture.snapshot,
            &fixture.camera,
            &fixture.lod_table,
            InstancedBudget::default(),
        )
        .expect("rebuilds");
        assert_eq!(list.order_digest(), digest);
    }

    #[test]
    fn storm_order_survives_input_permutation() {
        let fixture = storm();
        let mut first: Vec<InstancedDraw> = Vec::new();
        let mut first_digest = 0u64;
        for round in 0..3 {
            let mut nodes = fixture.snapshot.nodes.clone();
            let shift = (round * 211) % nodes.len().max(1);
            nodes.rotate_left(shift);
            let snapshot = SceneSnapshot { nodes };
            let mut list = InstancedList::new();
            let groups = list
                .build(
                    &snapshot,
                    &fixture.camera,
                    &fixture.lod_table,
                    InstancedBudget::default(),
                )
                .expect("permuted storm still groups");
            if round == 0 {
                first = groups.to_vec();
                first_digest = list.order_digest();
            } else {
                assert_eq!(groups, first, "order moved on round {round}");
                assert_eq!(list.order_digest(), first_digest);
            }
        }
    }

    #[test]
    fn storm_seed_moves_the_fixture() {
        let digest_for = |seed: u64| {
            let fixture = storm_fixture(seed);
            let mut list = InstancedList::new();
            list.build(
                &fixture.snapshot,
                &fixture.camera,
                &fixture.lod_table,
                InstancedBudget::default(),
            )
            .expect("seeded storm fits");
            list.order_digest()
        };
        assert_eq!(digest_for(2040), digest_for(2040));
        assert_ne!(digest_for(2040), digest_for(99));
    }

    fn same_squad_snapshot() -> SceneSnapshot {
        SceneSnapshot {
            nodes: [1u64, 2, 3]
                .iter()
                .map(|id| {
                    let mut n = node(*id, [0.0, 0.0, -5.0], 0.5);
                    n.mesh = 42;
                    n
                })
                .collect(),
        }
    }

    #[test]
    fn same_squad_nodes_form_one_group() {
        let camera = test_camera();
        let table = test_table();
        let snapshot = same_squad_snapshot();
        let mut list = InstancedList::new();
        let groups = list
            .build(&snapshot, &camera, &table, InstancedBudget::default())
            .expect("one squad fits");
        assert_eq!(groups.len(), 1);
        assert_eq!((groups[0].mesh, groups[0].lod, groups[0].count), (42, 0, 3));
    }

    #[test]
    fn tiny_squad_budget_refuses_loudly() {
        let fixture = storm();
        let mut list = InstancedList::new();
        let full = list
            .build(
                &fixture.snapshot,
                &fixture.camera,
                &fixture.lod_table,
                InstancedBudget::default(),
            )
            .expect("storm fits the default budget");
        let squad_count = full.len();
        assert!(squad_count > 1, "storm must need several squads");
        let tight = InstancedBudget {
            max_groups: 1,
            ..InstancedBudget::default()
        };
        assert_eq!(
            list.build(
                &fixture.snapshot,
                &fixture.camera,
                &fixture.lod_table,
                tight
            ),
            Err(DrawError::OverInstancedCap {
                groups: squad_count,
                cap: 1
            })
        );
        assert!(list.groups().is_empty(), "error leaves no partial list");
    }

    #[test]
    fn tiny_instance_budget_refuses_loudly() {
        let camera = test_camera();
        let table = test_table();
        let snapshot = same_squad_snapshot();
        let mut list = InstancedList::new();
        let tight = InstancedBudget {
            max_instances_per_group: 2,
            ..InstancedBudget::default()
        };
        assert_eq!(
            list.build(&snapshot, &camera, &table, tight),
            Err(DrawError::OverInstances {
                mesh: 42,
                lod: 0,
                count: 3,
                cap: 2
            })
        );
        assert!(list.groups().is_empty(), "error leaves no partial list");
    }

    #[test]
    fn instanced_build_rejects_unknown_tables() {
        let camera = test_camera();
        let table = test_table();
        let mut n = node(1, [0.0, 0.0, -5.0], 0.5);
        n.lod_table = 999;
        let snapshot = SceneSnapshot { nodes: vec![n] };
        let mut list = InstancedList::new();
        assert_eq!(
            list.build(&snapshot, &camera, &table, InstancedBudget::default()),
            Err(DrawError::UnknownLodTable {
                node: 1,
                expected: 7,
                found: 999
            })
        );
    }

    #[test]
    fn instanced_steady_state_reuses_the_buffers() {
        let fixture = storm();
        let mut list = InstancedList::new();
        list.build(
            &fixture.snapshot,
            &fixture.camera,
            &fixture.lod_table,
            InstancedBudget::default(),
        )
        .expect("storm fits");
        // Borrows end before the next build; capacities must not move.
        let (len, cap, digest) = (list.groups().len(), list.capacity(), list.order_digest());
        list.build(
            &fixture.snapshot,
            &fixture.camera,
            &fixture.lod_table,
            InstancedBudget::default(),
        )
        .expect("rebuilds");
        assert_eq!(list.groups().len(), len);
        assert_eq!(list.capacity(), cap);
        assert_eq!(list.order_digest(), digest);
    }

    #[ignore = "timing: needs pinned perf hardware; run under --perf"]
    #[test]
    fn storm_build_stays_under_1ms_p95() {
        let fixture = storm();
        let budget = InstancedBudget::default();
        let mut list = InstancedList::new();
        // Warmup holds both buffers steady before the clock starts.
        for _ in 0..10 {
            list.build(
                &fixture.snapshot,
                &fixture.camera,
                &fixture.lod_table,
                budget,
            )
            .expect("storm fits the instancing budget");
        }
        let mut samples = Vec::with_capacity(200);
        for _ in 0..200 {
            let start = std::time::Instant::now();
            list.build(
                &fixture.snapshot,
                &fixture.camera,
                &fixture.lod_table,
                budget,
            )
            .expect("storm fits the instancing budget");
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(|a, b| a.total_cmp(b));
        let p95 = samples[190];
        assert!(p95 < 1.0, "storm p95 {p95:.3} ms exceeds the 1 ms budget");
    }

    fn cull_planes() -> [[f32; 4]; 6] {
        let camera = test_camera();
        let basis = validate_camera(&camera).expect("test camera valid");
        planes_to_packed(&frustum_planes(&camera, &basis))
    }

    #[test]
    fn gpu_cull_cpu_boundary_keeps_and_culls() {
        let planes = cull_planes();
        assert!(sphere_visible_cpu([0.0, 0.0, -10.0], 1.0, &planes));
        assert!(!sphere_visible_cpu([0.0, 0.0, 10.0], 1.0, &planes));
        assert!(sphere_visible_cpu([0.0, 0.0, -101.0], 2.0, &planes));
        assert!(!sphere_visible_cpu([0.0, 0.0, -150.0], 1.0, &planes));
        assert!(!sphere_visible_cpu([f32::NAN, 0.0, -10.0], 1.0, &planes));
        assert!(!sphere_visible_cpu([0.0, 0.0, -10.0], -1.0, &planes));
    }

    #[test]
    fn gpu_cull_cpu_10k_seeded_stable_x2() {
        let planes = cull_planes();
        let base = Rng::new(2040);
        let mut spheres = Vec::with_capacity(10_008);
        let mut i: u64 = 0;
        while spheres.len() < 10_000 {
            let mut s = base.fork(i);
            let x = span_m(&mut s, 60.0) - 30.0;
            let y = span_m(&mut s, 10.0) - 5.0;
            let z = if i % 10 == 9 {
                span_m(&mut s, 20.0) + 10.0
            } else {
                -(span_m(&mut s, 40.0) + 20.0)
            };
            spheres.push([x, y, z, 0.5]);
            i += 1;
        }
        spheres.push([0.0, 0.0, -10.0, 1.0]);
        spheres.push([0.0, 0.0, 10.0, 1.0]);
        spheres.push([40.0, 0.0, -10.0, 1.0]);
        spheres.push([0.0, 30.0, -10.0, 1.0]);
        spheres.push([0.0, 0.0, -0.05, 0.01]);
        spheres.push([0.0, 0.0, -150.0, 1.0]);
        spheres.push([0.0, 0.0, -50.0, 1.0]);
        spheres.push([0.0, 0.0, -101.0, 2.0]);
        let mut a = vec![0u8; spheres.len()];
        let mut b = vec![0u8; spheres.len()];
        assert_eq!(
            cull_cpu_flags_into(&spheres, &planes, &mut a),
            spheres.len()
        );
        assert_eq!(
            cull_cpu_flags_into(&spheres, &planes, &mut b),
            spheres.len()
        );
        assert_eq!(a, b, "same bytes give same flags x2");
        assert_eq!(visible_count(&a), visible_count(&b));
        assert!(visible_count(&a) > 0, "some spheres stay visible");
        assert!(visible_count(&a) < a.len() as u32, "some spheres cull away");
    }

    #[test]
    fn gpu_cull_indirect_args_are_5_u32() {
        assert_eq!(indirect_args_5(0), [36, 0, 0, 0, 0]);
        assert_eq!(indirect_args_5(7), [36, 7, 0, 0, 0]);
        assert!(CULL_WGSL.contains("atomicAdd"), "shader counts visible");
        assert!(
            CULL_WGSL.contains("-radius"),
            "shader keeps the -radius rule"
        );
    }

    #[test]
    fn trail_strip_fades_alpha_by_age_and_skips_idle() {
        // Twelve-pair swing, oldest first: oldest age 11 (clear) down to
        // newest age 0 (opaque).
        let mut base = [[0.0_f32; 3]; 12];
        let mut tip = [[0.0_f32; 3]; 12];
        let mut age_ticks = [0u8; 12];
        for step in 0u8..12 {
            let i = usize::from(step);
            let x = f32::from(step);
            base[i] = [x, 1.0, 0.0];
            tip[i] = [x, 1.0, 1.0];
            age_ticks[i] = 11 - step;
        }
        let first = trail_strip_verts(&base, &tip, &age_ticks);
        assert_eq!(first.len(), 24, "12 pairs give 24 strip verts");
        assert_eq!(first[0].pos_m, base[0], "base leads each pair");
        assert_eq!(first[1].pos_m, tip[0], "tip follows its base");
        assert_eq!(first[22].pos_m, base[11], "newest pair stays last");
        assert_eq!(first[23].pos_m, tip[11], "strip ends on newest tip");
        assert_eq!(first[0].alpha, 0.0, "oldest is clear");
        assert_eq!(first[23].alpha, 1.0, "newest is opaque");
        for pair in 0usize..11 {
            let older = first[pair.saturating_mul(2)].alpha;
            let newer = first[(pair + 1).saturating_mul(2)].alpha;
            assert!(older < newer, "alpha rises oldest to newest");
            assert_eq!(
                first[pair.saturating_mul(2)].alpha,
                first[pair.saturating_mul(2) + 1].alpha,
                "base and tip share one age fade"
            );
        }
        assert!(
            trail_strip_verts(&[], &[], &[]).is_empty(),
            "idle slots draw none"
        );
        let mut bad_base = base;
        bad_base[5] = [f32::NAN, 0.0, 0.0];
        let skipped = trail_strip_verts(&bad_base, &tip, &age_ticks);
        assert_eq!(skipped.len(), 22, "one NaN pair drops two verts");
        assert_eq!(skipped[8].pos_m, base[4], "order holds past the gap");
        assert_eq!(skipped[9].pos_m, tip[4], "tip order holds past the gap");
        assert_eq!(skipped[10].pos_m, base[6], "gap skips only the NaN pair");
        let big_base = [[0.0_f32; 3]; 13];
        let big_tip = [[1.0_f32; 3]; 13];
        let big_age = [0u8; 13];
        assert_eq!(
            trail_strip_verts(&big_base, &big_tip, &big_age).len(),
            24,
            "past 12 pairs caps at 24 verts"
        );
        let second = trail_strip_verts(&base, &tip, &age_ticks);
        assert_eq!(first, second, "same bytes give same verts x2");
    }

    /// `tan(fov_y_rad / 2)`: the optics the pixel pick divides by.
    fn tan_half_of(fov_y_rad: f32) -> f32 {
        (fov_y_rad * 0.5).tan()
    }

    const FOV_60: f32 = std::f32::consts::FRAC_PI_3;

    /// One-meter ball, 1080p, four levels, `prev` honoured.
    fn pick_px(dist: f32, fov: f32, prev: u8, t: &[f32], hyst: f32) -> u8 {
        lod_by_pixel_error(1.0, dist, 1080, tan_half_of(fov), 4, prev, t, hyst).expect("picks")
    }

    #[test]
    fn lod_pixel_error_picks_the_smaller_level_for_a_far_node() {
        // 40 and 120 px boundaries: 467 px at 4 m, 31 px at 60 m, 102 px at FOV 20.
        let t = [40.0, 120.0];
        let near = pick_px(4.0, FOV_60, 0, &t, 0.0);
        let far = pick_px(60.0, FOV_60, 0, &t, 0.0);
        let tele = pick_px(60.0, std::f32::consts::PI / 9.0, 0, &t, 0.0);
        assert_eq!((near, far, tele), (0, 2, 1), "467, 31 and 102 px");
        assert!(tele < far, "the narrower FOV picks the finer level");
    }

    #[test]
    fn lod_pixel_error_never_switches_inside_the_hysteresis_band() {
        // 120 px boundary, 10 px band: 110..130 px holds; 15.5 m is 120.7 px.
        let t = [120.0];
        assert_eq!(pick_px(15.5, FOV_60, 0, &t, 10.0), 0, "held");
        assert_eq!(pick_px(15.5, FOV_60, 1, &t, 10.0), 1, "held");
        assert_eq!(pick_px(18.0, FOV_60, 0, &t, 10.0), 1, "103.9 px leaves");
        assert_eq!(pick_px(13.0, FOV_60, 1, &t, 10.0), 0, "143.9 px leaves");
    }

    #[test]
    fn lod_pixel_error_refuses_bad_inputs() {
        let op = tan_half_of(FOV_60);
        let good = [60.0];
        let call = |r: f32, d: f32, h: u32, fov: f32, lv: u8, hyst: f32, t: &[f32]| {
            lod_by_pixel_error(r, d, h, fov, lv, 0, t, hyst)
        };
        let no = |why: &str, r: Result<u8, DrawError>| {
            assert!(matches!(r, Err(DrawError::BadPixelLod(_))), "{why}: {r:?}");
        };
        no("NaN radius", call(f32::NAN, 4.0, 1080, op, 4, 0.0, &good));
        no("negative r", call(-1.0, 4.0, 1080, op, 4, 0.0, &good));
        no("zero dist", call(1.0, 0.0, 1080, op, 4, 0.0, &good));
        no("NaN dist", call(1.0, f32::NAN, 1080, op, 4, 0.0, &good));
        no("zero h", call(1.0, 4.0, 0, op, 4, 0.0, &good));
        no("zero optics", call(1.0, 4.0, 1080, 0.0, 4, 0.0, &good));
        no("zero levels", call(1.0, 4.0, 1080, op, 0, 0.0, &good));
        no("negative hyst", call(1.0, 4.0, 1080, op, 4, -1.0, &good));
        no("no thresholds", call(1.0, 4.0, 1080, op, 4, 0.0, &[]));
        let err = call(f32::NAN, 4.0, 1080, op, 4, 0.0, &good).expect_err("refused");
        assert!(format!("{err}").contains("radius_m"), "got {err}");
    }

    #[test]
    fn lod_pixel_error_caps_at_levels() {
        // Four boundaries, a sub-pixel ball: the pick stops at the coarsest.
        let t = [1.0, 2.0, 3.0, 4.0];
        let op = tan_half_of(FOV_60);
        let two = lod_by_pixel_error(0.01, 200.0, 540, op, 2, 0, &t, 0.0).expect("two");
        let four = lod_by_pixel_error(0.01, 200.0, 540, op, 4, 0, &t, 0.0).expect("four");
        assert_eq!(two, 1, "two levels cap the pick");
        assert_eq!(four, 3, "four levels reach the last boundary");
    }

    #[test]
    fn lod_pixel_none_keeps_the_band_path_bytes_identical() {
        let camera = test_camera();
        let table = test_table();
        assert!(table.pixel_thresholds_px.is_none(), "no pixel path armed");
        let mut list = DrawList::new();
        for (dist, prev) in [(5.0f32, 0u8), (12.0, 1), (25.0, 1), (60.0, 2)] {
            let mut n = node(1, [0.0, 0.0, -dist], 0.5);
            n.prev_lod = prev;
            let snapshot = SceneSnapshot { nodes: vec![n] };
            let draws = list.build(&snapshot, &camera, &table).expect("builds");
            let band = pick_lod(dist, prev, 4, &table.bands_m, table.hysteresis_m);
            assert_eq!(draws[0].lod, band, "band level moved at {dist} m");
        }
    }

    #[test]
    fn lod_pixel_table_refuses_bad_thresholds() {
        let camera = test_camera();
        let snapshot = SceneSnapshot {
            nodes: vec![node(1, [0.0, 0.0, -5.0], 0.5)],
        };
        let build = |t: Vec<f32>| {
            let mut table = test_table();
            table.pixel_thresholds_px = Some(t);
            DrawList::new().build(&snapshot, &camera, &table).is_ok()
        };
        assert!(!build(vec![60.0, 0.0]), "zero boundary");
        assert!(!build(vec![60.0, 60.0]), "unordered boundary");
        assert!(!build(vec![f32::NAN]), "NaN boundary");
        assert!(build(vec![40.0, 120.0]), "an ascending pair builds");
    }
}
