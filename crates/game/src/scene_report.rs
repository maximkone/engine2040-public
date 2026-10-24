//! scene_report: the live spatial scene report an AI reads (rows SP62, EG35).
//!
//! The `arena view:` laws judge a run as counters, and a PNG judges one frame
//! by its pixels. Neither says WHERE things are, what touches what, who hides
//! whom from the camera, or what the sun does to each object. This module
//! turns one frame's scene into that: every body, prop and spear with its
//! world place and bounds, its screen rect and cover, its floor contact, the
//! hand that holds what, the bone a spear is stuck in, what hides it from the
//! camera, whether the sun reaches it and where its shadow lands, plus the
//! camera, the light, the renderer's pass switches and the view laws. One
//! compact JSON line and a readable text form come out of the same numbers.
//!
//! The report is built from data the look and the renderer already own: the
//! posed triangles and joints of each body (`fighter_look`), the prop rows,
//! the spear scene, the draw list's light and the presenter's pass facts
//! (`engine_render::present::RenderFacts`). The pixel laws (shadow darkness,
//! material and scale checks on a PNG) belong to `tools/frame-audit.mjs`; this
//! report states the geometry those checks judge against.
//!
//! Consumer: the `arena` binary (`--scene-report`, `--scene-at`, the agent
//! seat's `scene` line) and the door's `view.scene` verb. Units: meters,
//! degrees for angles, centimeters for contact gaps, picture share `0..=1`.
//! Determinism: the same input gives the same bytes (fixed order, no clock, no
//! hash map). Budget: a few thousand ray-triangle tests per sample, built on
//! request only, never per tick. View only: nothing here reaches the sim hash.
//! Errors: none; a non-finite number prints as JSON `null`.

use std::collections::BTreeMap;

use engine_render::v1::Camera;

use crate::view_truth::{cover, screen_rect, GROUND_Y, STEP_M};

pub mod emit;
pub mod geom;
#[cfg(test)]
mod tests;
pub mod things;

use geom::{
    add, corners, dist, dot, finite, in_picture, inside, point_box, point_seg, project, scale,
    seg_box, seg_seg, seg_tri, sub, P3,
};

/// Version of the JSON shape; a reader checks it before it trusts a field.
pub const SCHEMA: u32 = 1;
/// Gap under which two things count as touching, centimeters.
pub const TOUCH_CM: f32 = 4.0;
/// A body within this many centimeters of its support counts as standing on it.
pub const GROUNDED_CM: f32 = 3.0;
/// A spear tip farther than this from every bone of the body it is stuck in is
/// a defect, centimeters.
pub const STUCK_BONE_CM: f32 = 25.0;
/// How far a sun ray reaches, meters (the sun map's far plane).
const SUN_REACH_M: f32 = 60.0;
/// Pull a sun ray's start off its own surface, meters.
const SUN_BIAS_M: f32 = 0.03;
/// A spear tail this close to a palm is in that hand, meters.
const PALM_HOLD_M: f32 = 0.18;
/// A thing hidden more than this share is a finding.
const HIDDEN_FINDING: f32 = 0.5;
/// Forward light and sky sun further apart than this is a finding, degrees.
const SKY_SUN_APART_DEG: f32 = 8.0;

/// What a thing is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A match fighter.
    Fighter,
    /// A staged prop: a kit actor, the ring floor or its wall.
    Prop,
    /// A spear in a hand, in the air, in a body or on the sand.
    Spear,
}

impl Kind {
    /// The kind's name on the wire.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Fighter => "fighter",
            Self::Prop => "prop",
            Self::Spear => "spear",
        }
    }
}

/// The solid a thing presents to a ray.
#[derive(Clone, Debug)]
pub enum Shape {
    /// Its bounds are the solid.
    Box,
    /// Seen and measured but blocks nothing: the ring floor, the stands.
    Hollow,
    /// Posed triangles in world meters.
    Mesh {
        /// Vertex positions.
        positions: Vec<P3>,
        /// Triangle list into `positions`.
        indices: Vec<u32>,
    },
    /// A rod: a spear's shaft.
    Rod {
        /// One end.
        a: P3,
        /// The other end.
        b: P3,
        /// Thickness as a radius, meters.
        radius: f32,
    },
}

/// A point on a thing with the way its surface faces there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// World place.
    pub at: P3,
    /// Unit surface normal; `None` when the thing has no surface data (a box).
    pub normal: Option<P3>,
}

/// One named joint of a fighter in world meters.
#[derive(Clone, Debug, PartialEq)]
pub struct Joint {
    /// The rig's joint name (`palm_R`, `head`).
    pub name: String,
    /// World place.
    pub at: P3,
    /// Parent joint index in the same list.
    pub parent: Option<usize>,
}

/// A body part skinned to a hand: what the fighter holds.
#[derive(Clone, Debug, PartialEq)]
pub struct HeldPart {
    /// The part's name (`gladius_blade`), or `part_<n>` when the rig names none.
    pub name: String,
    /// `'r'` or `'l'`.
    pub hand: char,
    /// World center of the part's bounds.
    pub at: P3,
    /// World bounds of the part.
    pub bounds: (P3, P3),
    /// Longest span of the part, meters.
    pub len_m: f32,
}

/// What only a fighter has.
#[derive(Clone, Debug, PartialEq)]
pub struct FighterFacts {
    /// Match slot.
    pub slot: u8,
    /// `idle`, `run`, `attack` or `down`.
    pub act: String,
    /// Clip index sampled.
    pub clip: u8,
    /// Wears the red hit tint.
    pub tinted: bool,
    /// Drawn from its ragdoll.
    pub ragdolled: bool,
    /// Hit points from the sim, when the caller has them.
    pub hp: Option<u16>,
    /// Drawn root (feet) in meters.
    pub root: P3,
    /// Drawn yaw in radians.
    pub yaw: f32,
    /// The weapon row the sim armed this slot with.
    pub armed: Option<String>,
    /// Slide of each planted foot from its spot, meters (left, right).
    pub foot_slide_m: [Option<f32>; 2],
    /// Body parts drawn with the skin error material.
    pub error_parts: u32,
    /// Posed vertices or joints with a non-finite component.
    pub nonfinite: u32,
    /// Parts skinned to a hand joint.
    pub held: Vec<HeldPart>,
    /// The caller knows which parts are skinned where, so an empty `held` is
    /// a fact and not an unknown.
    pub held_known: bool,
}

/// Where a spear is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpearState {
    /// In a hand.
    Held,
    /// In the air.
    Flying,
    /// Stuck in a fighter or in the world.
    Stuck,
    /// Lying on the sand.
    Rest,
}

impl SpearState {
    /// The state's name on the wire.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Held => "held",
            Self::Flying => "flying",
            Self::Stuck => "stuck",
            Self::Rest => "rest",
        }
    }
}

/// What only a spear has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpearFacts {
    /// The slot that threw (or holds) it.
    pub owner: u8,
    /// Held, flying, stuck or at rest.
    pub state: SpearState,
    /// Tip in world meters.
    pub tip: P3,
    /// Unit direction from tail to tip.
    pub dir: P3,
    /// Length tail to tip, meters.
    pub len_m: f32,
    /// The slot whose palm holds it.
    pub held_by: Option<u8>,
    /// The fighter it is stuck in.
    pub stuck_in: Option<u8>,
}

/// One thing in the scene.
#[derive(Clone, Debug)]
pub struct Thing {
    /// Stable id: `fighter_0`, `prop_3_pillar`, `spear_1`.
    pub id: String,
    /// What it is.
    pub kind: Kind,
    /// Short human name.
    pub label: String,
    /// World bounds, ordered and finite, or `None` when it has none.
    pub bounds: Option<(P3, P3)>,
    /// What a ray sees of it.
    pub shape: Shape,
    /// The frame draws it. A prop the match collides with but the window never
    /// draws is an invisible obstacle.
    pub drawn: bool,
    /// Points the visibility and sun tests send rays to.
    pub samples: Vec<Sample>,
    /// Named joints (fighters).
    pub joints: Vec<Joint>,
    /// Fighter facts.
    pub fighter: Option<FighterFacts>,
    /// Spear facts.
    pub spear: Option<SpearFacts>,
}

/// The frame's directional light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    /// Unit direction toward the sun.
    pub sun_dir: P3,
    /// Linear color.
    pub color: P3,
    /// Intensity scale.
    pub intensity: f32,
    /// Direction toward the sun the sky and ambient bake uses, when the scene
    /// has one apart from the forward light.
    pub sky_sun_dir: Option<P3>,
}

/// The renderer's sun shadow map, as `RenderFacts` reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowMap {
    /// The map is recorded this frame.
    pub on: bool,
    /// Edge in texels.
    pub edge_px: u32,
    /// Half the ortho box edge, meters.
    pub half_extent_m: f32,
    /// One texel, meters.
    pub texel_m: f32,
    /// Row-vector sun view-projection; `None` when the sun has no direction.
    pub view_proj: Option<[[f32; 4]; 4]>,
}

/// The view laws so far, as the `arena view:` line folds them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Laws {
    /// The `arena view:` line; empty before the first folded frame.
    pub line: String,
    /// `ok` or `FAIL(metric,..)`.
    pub verdict: String,
    /// One `arena WARN:` line per broken floor.
    pub warnings: Vec<String>,
}

/// Pixels the renderer's draw-id buffer gave each thing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pixels {
    /// Pixels in the buffer.
    pub total: u64,
    /// Pixels no draw claimed (sky or clear).
    pub clear: u64,
    /// Pixels per thing id.
    pub by_id: BTreeMap<String, u64>,
}

/// Everything one report is built from.
#[derive(Clone, Debug)]
pub struct SceneInput {
    /// `headless` or `window`.
    pub source: &'static str,
    /// Ticks simulated.
    pub tick: u32,
    /// Match seed.
    pub seed: u64,
    /// Picture size in pixels.
    pub viewport: (u32, u32),
    /// The camera that drew the frame.
    pub camera: Camera,
    /// Name of the shot the camera is in.
    pub shot: String,
    /// `director`, `free` or `pinned`.
    pub mode: String,
    /// The sun.
    pub light: Light,
    /// The sun shadow map.
    pub shadow: ShadowMap,
    /// Contact-shadow strength `0..=1`.
    pub contact: f32,
    /// Every switchable pass and whether it runs.
    pub passes: Vec<(String, bool)>,
    /// Everything in the scene, in a fixed order.
    pub things: Vec<Thing>,
    /// The view laws.
    pub laws: Laws,
    /// Id-buffer pixels, when the caller took them.
    pub pixels: Option<Pixels>,
    /// A live renderer supplied the pass switches and the sun map; false means
    /// the shipped defaults stand in (a headless run has no renderer).
    pub renderer_live: bool,
}

/// One built report: the JSON line, the text form and the findings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// One compact JSON object on one line (it embeds `text`).
    pub json: String,
    /// The readable form, lines joined by `\n`.
    pub text: String,
    /// Short sentences naming what looks wrong, empty when nothing does.
    pub findings: Vec<String>,
}

/// How much of a thing the camera sees.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Vis {
    pub visible: f32,
    pub occluded: f32,
    pub offscreen: f32,
    /// Blocking thing index and the share of samples it hides, largest first.
    pub by: Vec<(usize, f32)>,
}

/// What the sun does to a thing.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SunRow {
    pub lit: f32,
    pub blocked: f32,
    pub by: Vec<(usize, f32)>,
    pub casts: &'static str,
    pub footprint: Option<Footprint>,
}

/// Where a thing's shadow falls on the sand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Footprint {
    pub xz: [f32; 4],
    pub len_m: f32,
    pub screen: Option<[f32; 4]>,
}

/// A fighter's contact with the ground or a prop it stands on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FloorRow {
    pub min_y_cm: f32,
    pub grounded: bool,
    pub standing_on: Option<usize>,
    pub feet_cm: [Option<f32>; 2],
}

/// Two things within touching distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Touch {
    pub other: usize,
    pub gap_cm: f32,
    pub overlap_cm: f32,
}

/// One hand of a fighter and what is in it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HandRow {
    pub hand: char,
    pub at: Option<P3>,
    pub holds: Vec<(String, &'static str)>,
}

/// Where a stuck spear's tip sits.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StuckRow {
    pub fighter: Option<usize>,
    pub bone: Option<String>,
    pub dist_cm: f32,
    pub tip_inside: bool,
    pub near: Option<usize>,
}

/// Everything measured about one thing.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Row {
    pub screen: Option<[f32; 4]>,
    pub cover: f32,
    pub depth_m: Option<f32>,
    pub vis: Vis,
    pub sun: SunRow,
    pub floor: Option<FloorRow>,
    pub touch: Vec<Touch>,
    pub hands: Vec<HandRow>,
    pub stuck: Option<StuckRow>,
    pub px: Option<u64>,
}

/// Build the report for one frame.
#[must_use]
pub fn build(input: &SceneInput) -> Report {
    let rows: Vec<Row> = (0..input.things.len()).map(|i| measure(input, i)).collect();
    let findings = findings(input, &rows);
    let text = emit::text(input, &rows, &findings);
    let json = emit::json(input, &rows, &findings, &text);
    Report {
        json,
        text,
        findings,
    }
}

/// The share `n` of `total`, `0.0` for an empty total.
fn share(n: u32, total: usize) -> f32 {
    if total == 0 {
        return 0.0;
    }
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
        / f32::from(u16::try_from(total).unwrap_or(u16::MAX))
}

/// Measure one thing against the rest of the scene.
fn measure(input: &SceneInput, i: usize) -> Row {
    let thing = &input.things[i];
    let screen = thing
        .bounds
        .and_then(|(lo, hi)| screen_rect(&input.camera, lo, hi));
    let depth_m = thing.bounds.map(|(lo, hi)| {
        let mid = scale(add(lo, hi), 0.5);
        dist(input.camera.eye.to_array(), mid)
    });
    let vis = visibility(input, i);
    let sun = sun_facts(input, i);
    let floor = (thing.kind == Kind::Fighter)
        .then(|| floor_row(input, i))
        .flatten();
    let px = input
        .pixels
        .as_ref()
        .map(|px| px.by_id.get(&thing.id).copied().unwrap_or(0));
    Row {
        cover: screen.map_or(0.0, cover),
        screen,
        depth_m,
        vis,
        sun,
        floor,
        touch: touches(input, i),
        hands: hands(input, i),
        stuck: stuck_row(input, i),
        px,
    }
}

/// The nearest thing other than `skip` that blocks the segment `from..to`.
/// A hollow thing blocks nothing, and a box the segment starts inside is a
/// room around the start, not a wall in front of it.
fn first_block(things: &[Thing], skip: usize, from: P3, to: P3) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    for (j, other) in things.iter().enumerate() {
        if j == skip {
            continue;
        }
        let Some(t) = block_t(other, from, to) else {
            continue;
        };
        if best.is_none_or(|(tb, _)| t < tb) {
            best = Some((t, j));
        }
    }
    best.map(|(_, j)| j)
}

/// Where along `from..to` (`0..1`) `other` first blocks it.
fn block_t(other: &Thing, from: P3, to: P3) -> Option<f32> {
    let (lo, hi) = other.bounds?;
    match &other.shape {
        Shape::Hollow => None,
        Shape::Box => {
            if inside(from, lo, hi) {
                return None;
            }
            seg_box(from, to, lo, hi).filter(|t| *t < 0.999)
        }
        Shape::Mesh { positions, indices } => {
            let pad = [0.02; 3];
            seg_box(from, to, sub(lo, pad), add(hi, pad))?;
            let mut nearest: Option<f32> = None;
            for tri in indices.chunks_exact(3) {
                let corner = |k: usize| {
                    usize::try_from(tri[k])
                        .ok()
                        .and_then(|at| positions.get(at))
                        .copied()
                };
                let (Some(a), Some(b), Some(c)) = (corner(0), corner(1), corner(2)) else {
                    continue;
                };
                if let Some(t) = seg_tri(from, to, [a, b, c]).filter(|t| *t > 0.001 && *t < 0.999) {
                    nearest = Some(nearest.map_or(t, |n| n.min(t)));
                }
            }
            nearest
        }
        Shape::Rod { a, b, radius } => {
            let (gap, t) = seg_seg(from, to, *a, *b);
            (gap <= *radius && t > 0.001 && t < 0.999).then_some(t)
        }
    }
}

/// Sort blockers by share, largest first, then by index.
fn ranked(counts: &BTreeMap<usize, u32>, total: usize) -> Vec<(usize, f32)> {
    let mut out: Vec<(usize, f32)> = counts.iter().map(|(j, n)| (*j, share(*n, total))).collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

/// What the camera sees of thing `i`: for each sample, in the picture or not,
/// and if it is, whether a nearer thing is in the way.
fn visibility(input: &SceneInput, i: usize) -> Vis {
    let thing = &input.things[i];
    let total = thing.samples.len();
    let eye = input.camera.eye.to_array();
    let (mut seen, mut hidden, mut off) = (0u32, 0u32, 0u32);
    let mut by: BTreeMap<usize, u32> = BTreeMap::new();
    for sample in &thing.samples {
        let on_screen = project(&input.camera, sample.at)
            .is_some_and(|(ndc, depth)| in_picture(ndc) && depth <= input.camera.far);
        if !on_screen {
            off += 1;
            continue;
        }
        match first_block(&input.things, i, eye, sample.at) {
            Some(j) => {
                hidden += 1;
                *by.entry(j).or_insert(0) += 1;
            }
            None => seen += 1,
        }
    }
    Vis {
        visible: share(seen, total),
        occluded: share(hidden, total),
        offscreen: share(off, total),
        by: ranked(&by, total),
    }
}

/// Sun elevation above the horizon, degrees.
pub(crate) fn sun_elev_deg(sun: P3) -> f32 {
    sun[1].clamp(-1.0, 1.0).asin().to_degrees()
}

/// What the sun does to thing `i`: the share of its surface lit, the share a
/// nearer thing shades, whether the sun map holds it, and its ground shadow.
fn sun_facts(input: &SceneInput, i: usize) -> SunRow {
    let thing = &input.things[i];
    let hollow = matches!(thing.shape, Shape::Hollow);
    let sun = input.light.sun_dir;
    let up = sun_elev_deg(sun) > 0.0 && input.light.intensity > 0.0;
    let total = thing.samples.len();
    let (mut lit, mut shaded) = (0u32, 0u32);
    let mut by: BTreeMap<usize, u32> = BTreeMap::new();
    if up {
        for sample in &thing.samples {
            if sample.normal.is_some_and(|n| dot(n, sun) <= 0.0) {
                continue;
            }
            let from = add(sample.at, scale(sun, SUN_BIAS_M));
            let to = add(from, scale(sun, SUN_REACH_M));
            match first_block(&input.things, i, from, to) {
                Some(j) => {
                    shaded += 1;
                    *by.entry(j).or_insert(0) += 1;
                }
                None => lit += 1,
            }
        }
    }
    SunRow {
        lit: share(lit, total),
        blocked: share(shaded, total),
        by: ranked(&by, total),
        casts: if hollow { "n/a" } else { casts(input, thing) },
        footprint: (!hollow && up).then(|| footprint(input, thing)).flatten(),
    }
}

/// Whether the sun map holds this thing: `off`, `outside`, `partial` or `yes`.
fn casts(input: &SceneInput, thing: &Thing) -> &'static str {
    if !input.shadow.on || sun_elev_deg(input.light.sun_dir) <= 0.0 {
        return "off";
    }
    let (Some(m), Some((lo, hi))) = (input.shadow.view_proj, thing.bounds) else {
        return "off";
    };
    let held = corners(lo, hi)
        .iter()
        .filter(|p| {
            let c: P3 =
                std::array::from_fn(|k| p[0] * m[0][k] + p[1] * m[1][k] + p[2] * m[2][k] + m[3][k]);
            c[0].abs() <= 1.0 && c[1].abs() <= 1.0 && (0.0..=1.0).contains(&c[2])
        })
        .count();
    match held {
        0 => "outside",
        8 => "yes",
        _ => "partial",
    }
}

/// The patch of sand a thing's bounds shade: each corner slid along the sun
/// ray to the ground, boxed in x and z.
fn footprint(input: &SceneInput, thing: &Thing) -> Option<Footprint> {
    let (lo, hi) = thing.bounds?;
    let sun = input.light.sun_dir;
    let (mut x0, mut z0, mut x1, mut z1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in corners(lo, hi) {
        let run = (p[1] - GROUND_Y).max(0.0) / sun[1];
        let (x, z) = (p[0] - sun[0] * run, p[2] - sun[2] * run);
        (x0, z0, x1, z1) = (x0.min(x), z0.min(z), x1.max(x), z1.max(z));
    }
    if !(x0.is_finite() && x1.is_finite() && z0.is_finite() && z1.is_finite()) {
        return None;
    }
    let tan = sun_elev_deg(sun).to_radians().tan();
    Some(Footprint {
        xz: [x0, z0, x1, z1],
        len_m: if tan > 1e-4 {
            (hi[1] - lo[1]) / tan
        } else {
            f32::MAX
        },
        screen: screen_rect(&input.camera, [x0, GROUND_Y, z0], [x1, GROUND_Y, z1]),
    })
}

/// Index of a joint by name, ignoring case.
pub(crate) fn joint_at<'a>(thing: &'a Thing, name: &str) -> Option<&'a Joint> {
    thing
        .joints
        .iter()
        .find(|j| j.name.eq_ignore_ascii_case(name))
}

/// The floor a fighter stands on and how close its feet are.
fn floor_row(input: &SceneInput, i: usize) -> Option<FloorRow> {
    let thing = &input.things[i];
    let (lo, _) = thing.bounds?;
    let root = thing.fighter.as_ref()?.root;
    let mut support = (GROUND_Y, None);
    for (j, other) in input.things.iter().enumerate() {
        let (olo, ohi) = match (&other.shape, other.bounds) {
            (Shape::Box, Some(b)) if other.kind == Kind::Prop => b,
            _ => continue,
        };
        let under =
            olo[0] <= root[0] && root[0] <= ohi[0] && olo[2] <= root[2] && root[2] <= ohi[2];
        if under && ohi[1] - olo[1] > STEP_M && ohi[1] <= root[1] + 0.2 && ohi[1] > support.0 {
            support = (ohi[1], Some(j));
        }
    }
    let min_y_cm = (lo[1] - support.0) * 100.0;
    let feet = |name: &str| joint_at(thing, name).map(|j| (j.at[1] - support.0) * 100.0);
    Some(FloorRow {
        min_y_cm,
        grounded: min_y_cm.abs() <= GROUNDED_CM,
        standing_on: support.1,
        feet_cm: [feet("foot_l"), feet("foot_r")],
    })
}

/// Distance in centimeters from thing `a`'s samples to thing `b`'s solid.
fn gap_cm(a: &Thing, b: &Thing) -> f32 {
    let mut best = f32::MAX;
    for s in &a.samples {
        let d = match (&b.shape, b.bounds) {
            (Shape::Box, Some((lo, hi))) => point_box(s.at, lo, hi),
            (Shape::Rod { a: p, b: q, radius }, _) => (point_seg(s.at, *p, *q) - radius).max(0.0),
            _ => b
                .samples
                .iter()
                .map(|t| dist(s.at, t.at))
                .fold(f32::MAX, f32::min),
        };
        best = best.min(d);
    }
    best * 100.0
}

/// Deepest a sample of `a` sits inside the box of `b`, centimeters.
fn overlap_cm(a: &Thing, b: &Thing) -> f32 {
    let (Shape::Box, Some((lo, hi))) = (&b.shape, b.bounds) else {
        return 0.0;
    };
    a.samples
        .iter()
        .filter(|s| inside(s.at, lo, hi))
        .map(|s| {
            (0..3)
                .map(|k| (s.at[k] - lo[k]).min(hi[k] - s.at[k]))
                .fold(f32::MAX, f32::min)
        })
        .fold(0.0, f32::max)
        * 100.0
}

/// Things within [`TOUCH_CM`] of thing `i`, nearest first. Two props never
/// touch for this report's purpose, and a floor-height prop is ground.
fn touches(input: &SceneInput, i: usize) -> Vec<Touch> {
    let me = &input.things[i];
    if matches!(me.shape, Shape::Hollow) || me.bounds.is_none() {
        return Vec::new();
    }
    let ground_like =
        |t: &Thing| t.kind == Kind::Prop && t.bounds.is_none_or(|(lo, hi)| hi[1] - lo[1] <= STEP_M);
    let mut out = Vec::new();
    for (j, other) in input.things.iter().enumerate() {
        let skip = j == i
            || matches!(other.shape, Shape::Hollow)
            || other.bounds.is_none()
            || (me.kind == Kind::Prop && other.kind == Kind::Prop)
            || ground_like(me)
            || ground_like(other);
        if skip {
            continue;
        }
        let gap = gap_cm(me, other).min(gap_cm(other, me));
        let overlap = overlap_cm(me, other).max(overlap_cm(other, me));
        if gap <= TOUCH_CM || overlap > 0.0 {
            out.push(Touch {
                other: j,
                gap_cm: gap,
                overlap_cm: overlap,
            });
        }
    }
    out.sort_by(|a, b| a.gap_cm.total_cmp(&b.gap_cm).then(a.other.cmp(&b.other)));
    out
}

/// The palm joint of a hand, falling back to the weapon socket then the hand.
fn palm(thing: &Thing, hand: char) -> Option<&Joint> {
    [
        format!("palm_{hand}"),
        format!("wep_pos_{hand}"),
        format!("hand_{hand}"),
    ]
    .iter()
    .find_map(|name| joint_at(thing, name))
}

/// Which hand holds what: parts skinned to the hand, spears the sim puts in
/// the right palm, and any spear whose tail sits at a palm.
fn hands(input: &SceneInput, i: usize) -> Vec<HandRow> {
    let thing = &input.things[i];
    let Some(facts) = thing.fighter.as_ref() else {
        return Vec::new();
    };
    ['r', 'l']
        .into_iter()
        .map(|hand| {
            let at = palm(thing, hand).map(|j| j.at);
            let mut holds: Vec<(String, &'static str)> = facts
                .held
                .iter()
                .filter(|part| part.hand == hand)
                .map(|part| (part.name.clone(), "skinned"))
                .collect();
            for other in &input.things {
                let Some(spear) = other.spear.as_ref() else {
                    continue;
                };
                let in_palm = spear.held_by == Some(facts.slot) && hand == 'r';
                let tail = sub(spear.tip, scale(spear.dir, spear.len_m));
                let at_palm = at.is_some_and(|p| dist(p, tail) <= PALM_HOLD_M);
                if in_palm || at_palm {
                    holds.push((other.id.clone(), if in_palm { "held" } else { "at_palm" }));
                }
            }
            HandRow { hand, at, holds }
        })
        .collect()
}

/// Distance in centimeters from `tip` to the nearest bone of `body`, and the
/// bone's name. Fingers are not bones a spear sticks in.
fn nearest_bone(body: &Thing, tip: P3) -> Option<(String, f32)> {
    let mut best: Option<(String, f32)> = None;
    for joint in &body.joints {
        let lower = joint.name.to_ascii_lowercase();
        let finger = ["index", "middle", "pinky", "ring", "thumb", "finger"]
            .iter()
            .any(|f| lower.contains(f));
        if finger {
            continue;
        }
        let d = match joint.parent.and_then(|p| body.joints.get(p)) {
            Some(parent) => point_seg(tip, joint.at, parent.at),
            None => dist(tip, joint.at),
        } * 100.0;
        if best.as_ref().is_none_or(|(_, b)| d < *b) {
            best = Some((joint.name.clone(), d));
        }
    }
    best
}

/// Where a stuck spear's tip sits: the bone of the fighter it is in, or the
/// prop nearest the world point it hit.
fn stuck_row(input: &SceneInput, i: usize) -> Option<StuckRow> {
    let spear = input.things[i]
        .spear
        .as_ref()
        .filter(|s| s.state == SpearState::Stuck)?;
    let victim = spear.stuck_in.and_then(|slot| {
        input
            .things
            .iter()
            .position(|t| t.fighter.as_ref().is_some_and(|f| f.slot == slot))
    });
    if let Some(v) = victim {
        let body = &input.things[v];
        let bone = nearest_bone(body, spear.tip);
        let tip_inside = body
            .bounds
            .is_some_and(|(lo, hi)| inside(spear.tip, lo, hi));
        return Some(StuckRow {
            fighter: Some(v),
            dist_cm: bone.as_ref().map_or(f32::MAX, |b| b.1),
            bone: bone.map(|b| b.0),
            tip_inside,
            near: None,
        });
    }
    let near = input
        .things
        .iter()
        .enumerate()
        .filter(|(_, t)| t.kind == Kind::Prop && !matches!(t.shape, Shape::Hollow))
        .filter_map(|(j, t)| {
            t.bounds
                .map(|(lo, hi)| (j, point_box(spear.tip, lo, hi) * 100.0))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    Some(StuckRow {
        fighter: None,
        bone: None,
        dist_cm: near.map_or(f32::MAX, |n| n.1),
        tip_inside: false,
        near: near.map(|n| n.0),
    })
}

/// A percentage for a sentence.
pub(crate) fn pct(share: f32) -> String {
    format!("{:.0}%", share * 100.0)
}

/// Sentences naming what looks wrong. Order is by thing, then by check, so the
/// same scene always lists the same findings in the same order.
fn findings(input: &SceneInput, rows: &[Row]) -> Vec<String> {
    let mut out = Vec::new();
    let name = |j: usize| input.things[j].id.as_str();
    camera_findings(input, &mut out);
    if !input.shadow.on {
        out.push("the sun shadow map is off: nothing casts a shadow".to_owned());
    }
    if let Some(sky) = input.light.sky_sun_dir {
        let apart = dot(input.light.sun_dir, sky)
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees();
        if apart > SKY_SUN_APART_DEG {
            out.push(format!(
                "the forward light and the sky sun point {apart:.0} degrees apart, so cast shadows and the sky disagree"
            ));
        }
    }
    for (i, (thing, row)) in input.things.iter().zip(rows).enumerate() {
        let id = thing.id.as_str();
        let solid = !matches!(thing.shape, Shape::Hollow);
        if solid && !thing.drawn && thing.kind == Kind::Prop {
            out.push(format!(
                "{id} is solid in the match but the frame draws nothing for it"
            ));
        }
        if thing.kind != Kind::Prop && row.screen.is_none() {
            out.push(format!("{id} is outside the picture"));
        } else if thing.kind != Kind::Prop && row.vis.offscreen > 0.1 && row.vis.visible > 0.0 {
            out.push(format!(
                "{id} is cut by the frame edge ({} of it outside)",
                pct(row.vis.offscreen)
            ));
        }
        if row.vis.occluded >= HIDDEN_FINDING {
            let by = row.vis.by.first().map_or("?", |(j, _)| name(*j));
            out.push(format!(
                "{id} is {} hidden behind {by}",
                pct(row.vis.occluded)
            ));
        }
        if solid && row.sun.blocked >= HIDDEN_FINDING && row.sun.lit < 0.25 {
            let by = row.sun.by.first().map_or("?", |(j, _)| name(*j));
            out.push(format!(
                "{id} is in the shade of {by} ({} lit)",
                pct(row.sun.lit)
            ));
        }
        if solid && input.shadow.on && matches!(row.sun.casts, "outside" | "partial") {
            out.push(if row.sun.casts == "outside" {
                format!("{id} is outside the sun map, so it casts no shadow")
            } else {
                format!("{id} is only partly inside the sun map, so its shadow is cut")
            });
        }
        if let (Some(px), true) = (row.px, solid && row.cover > 0.01) {
            if px == 0 {
                out.push(format!("{id} covers {} of the picture on paper but the id buffer holds 0 of its pixels", pct(row.cover)));
            }
        }
        findings_fighter(thing, row, &mut out);
        findings_spear(input, i, row, &mut out);
    }
    if !input.laws.verdict.is_empty() && input.laws.verdict != "ok" {
        out.push(format!("view laws {}", input.laws.verdict));
    }
    out
}

/// The camera's own findings: inside a body, under the sand.
fn camera_findings(input: &SceneInput, out: &mut Vec<String>) {
    let eye = input.camera.eye.to_array();
    if eye[1] < GROUND_Y {
        out.push("the camera is under the sand".to_owned());
    }
    for thing in &input.things {
        let in_body =
            thing.kind == Kind::Fighter && thing.bounds.is_some_and(|(lo, hi)| inside(eye, lo, hi));
        if in_body {
            out.push(format!("the camera is inside {}", thing.id));
        }
    }
}

/// Findings that only a fighter can have.
fn findings_fighter(thing: &Thing, row: &Row, out: &mut Vec<String>) {
    let (Some(facts), id) = (thing.fighter.as_ref(), thing.id.as_str()) else {
        return;
    };
    if let Some(floor) = row.floor {
        if floor.min_y_cm < -GROUNDED_CM {
            out.push(format!(
                "{id} is sunk {:.1} cm into its floor",
                -floor.min_y_cm
            ));
        } else if floor.min_y_cm > 5.0 && facts.act == "idle" {
            out.push(format!(
                "{id} idles {:.1} cm above its floor",
                floor.min_y_cm
            ));
        }
    }
    if facts.error_parts > 0 {
        out.push(format!(
            "{id} draws {} part(s) with the skin error material",
            facts.error_parts
        ));
    }
    if facts.nonfinite > 0 {
        out.push(format!(
            "{id} has {} non-finite vertices or joints",
            facts.nonfinite
        ));
    }
    if let (Some(weapon), true) = (facts.armed.as_deref(), facts.held_known) {
        let holds = row.hands.iter().any(|h| !h.holds.is_empty());
        if !holds && facts.act != "down" {
            out.push(format!(
                "{id} is armed with {weapon} but no hand holds anything"
            ));
        }
    }
}

/// Findings that only a spear can have.
fn findings_spear(input: &SceneInput, i: usize, row: &Row, out: &mut Vec<String>) {
    let (Some(spear), Some(stuck)) = (input.things[i].spear.as_ref(), row.stuck.as_ref()) else {
        return;
    };
    let id = input.things[i].id.as_str();
    let Some(v) = stuck.fighter else {
        return;
    };
    let victim = input.things[v].id.as_str();
    let bone = stuck.bone.as_deref().unwrap_or("?");
    if stuck.dist_cm > STUCK_BONE_CM {
        out.push(format!(
            "{id} is stuck in {victim} but its tip is {:.0} cm from the nearest bone ({bone})",
            stuck.dist_cm
        ));
    } else if !stuck.tip_inside {
        out.push(format!(
            "{id} is stuck in {victim} but its tip is outside the body's bounds"
        ));
    }
    if !finite(spear.tip) {
        out.push(format!("{id} has a non-finite tip"));
    }
}
