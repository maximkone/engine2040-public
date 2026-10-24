//! fighter_kit: the hero's Roman kit, fitted to the body it sits on.
//!
//! The cooked hero is a bare man in shorts under a pale plate suit that the cook welded into the
//! same mesh and atlas (armour, gauntlets, helm and boots on one atlas). Drawn as it ships it reads
//! as white clay, shows the body's skin through the suit's gaps from behind, and ends in a smooth
//! egg of a helm. This module takes the suit off and dresses the same body as a legionary: a lorica
//! segmentata of overlapping steel plates with shoulder lames, a leather belt and a red tunic, a
//! bronze galea with a neck guard, cheek plates and a horsehair crest, bronze greaves and leather
//! sandals. Every piece is built from the body's own vertices (a shrinkwrap: each ring stands a
//! margin off the widest body point of its slice), so no skin can poke through the kit by
//! construction, and the tests hold the margin against the real hero bytes.
//!
//! Owner: this module; consumer: `FighterLook::load_rig` (cold path, once per rig). Units: meters,
//! bind pose, Y up, the hero faces +Z. Determinism: pure, the same body gives the same parts. A
//! body that is not the hero (the three opponents) is left untouched and gives none.

use std::f32::consts::{PI, TAU};

use engine_anim::contract::Skeleton;
use engine_assets::body_import::SkinnedBody;
// The one f32 cross in the workspace (a private copy here doubled rust-quality's dup_fns).
use engine_core::vec3::cross3 as cross;

use super::fighter_kit_tex::{self as tex, Texels};

/// The variant this kit dresses.
pub const HERO_VARIANT: &str = "hero";
/// Sectors around every ring.
const SECTORS: usize = 28;
/// Parts the kit adds to the rig (the rig holds 16 in all, the body and the gear take 8).
pub const KIT_PARTS: usize = 6;

type V3 = [f32; 3];

fn fl(i: usize) -> f32 {
    f32::from(u16::try_from(i).unwrap_or(u16::MAX))
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: V3, k: f32) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l > 1.0e-9 {
        scale(a, 1.0 / l)
    } else {
        [0.0, 1.0, 0.0]
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ------------------------------------------------------------------------------------ the body

/// The atlas tile a triangle's texels sit in (`content/fighters/tools/assemble.py` TILES).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tile {
    /// The heavy plate suit (torso, arms and legs in one piece).
    Armor,
    /// The bare body: torso, arms and legs.
    Body,
    /// The face.
    Head,
    /// The hair.
    Hair,
    /// The bare hands.
    Hands,
    /// The bare feet and lower legs.
    Feet,
    /// The boots.
    Boots,
    /// The gauntlets.
    Gloves,
    /// The heavy helm.
    Helmet,
    /// Anything else: the grafted weapons point at an empty corner.
    Other,
}

/// Where each atlas quadrant (one cooked part per quadrant) starts in the 2048 px atlas.
const QUADRANT_ORIGIN_PX: [[f32; 2]; 4] =
    [[0.0, 0.0], [1024.0, 0.0], [0.0, 1024.0], [1024.0, 1024.0]];
/// Edge of one quadrant, px.
const QUADRANT_PX: f32 = 1024.0;
/// `(tile, x, y, size)` in px of the 2048 atlas.
const TILES: [(Tile, f32, f32, f32); 9] = [
    (Tile::Armor, 0.0, 0.0, 1024.0),
    (Tile::Body, 1024.0, 0.0, 512.0),
    (Tile::Head, 1536.0, 0.0, 512.0),
    (Tile::Hair, 1024.0, 512.0, 512.0),
    (Tile::Hands, 1536.0, 512.0, 512.0),
    (Tile::Feet, 1024.0, 1024.0, 512.0),
    (Tile::Boots, 1536.0, 1024.0, 512.0),
    (Tile::Gloves, 0.0, 1536.0, 512.0),
    (Tile::Helmet, 512.0, 1536.0, 512.0),
];

/// The tile of a point of cooked part `part` at `uv`.
#[must_use]
pub fn tile_of(part: usize, uv: [f32; 2]) -> Tile {
    let Some(origin) = QUADRANT_ORIGIN_PX.get(part) else {
        return Tile::Other;
    };
    let x = origin[0] + uv[0] * QUADRANT_PX;
    let y = origin[1] + uv[1] * QUADRANT_PX;
    TILES
        .iter()
        .find(|(_, tx, ty, size)| x >= *tx && x < tx + size && y >= *ty && y < ty + size)
        .map_or(Tile::Other, |(tile, ..)| *tile)
}

/// The tile of one triangle: where its UV centroid falls.
fn triangle_tile(part: usize, uv: &[[f32; 2]], tri: &[u32]) -> Tile {
    let at = |i: u32| {
        uv.get(usize::try_from(i).unwrap_or(usize::MAX))
            .copied()
            .unwrap_or([0.0; 2])
    };
    let (a, b, c) = (at(tri[0]), at(tri[1]), at(tri[2]));
    tile_of(
        part,
        [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0],
    )
}

/// Triangles of `body` whose texels sit in `tile`.
fn tile_triangles(body: &SkinnedBody, tile: Tile) -> usize {
    body.parts
        .iter()
        .enumerate()
        .map(|(pi, part)| {
            part.mesh
                .indices
                .chunks_exact(3)
                .filter(|tri| triangle_tile(pi, &part.mesh.uv0, tri) == tile)
                .count()
        })
        .sum()
}

/// The joints the kit hangs on, as skeleton indices; side 0 is `_L`, side 1 is `_R`.
#[derive(Clone, Copy, Debug)]
struct Joints {
    pelvis: usize,
    torso: usize,
    head: usize,
    shoulder: [usize; 2],
    arm_upper: [usize; 2],
    arm_lower: [usize; 2],
    leg_upper: [usize; 2],
}

impl Joints {
    fn find(skeleton: &Skeleton) -> Option<Self> {
        let at = |name: &str| {
            skeleton
                .joints
                .iter()
                .position(|j| j.name.eq_ignore_ascii_case(name))
        };
        let pair = |stem: &str| Some([at(&format!("{stem}_L"))?, at(&format!("{stem}_R"))?]);
        Some(Self {
            pelvis: at("pelvis")?,
            torso: at("torso")?,
            head: at("head")?,
            shoulder: pair("shoulder")?,
            arm_upper: pair("arm_upper")?,
            arm_lower: pair("arm_lower")?,
            leg_upper: pair("leg_upper")?,
        })
    }
}

/// One body vertex with its tile and the joint that moves it most.
#[derive(Clone, Copy, Debug)]
struct Vert {
    p: V3,
    tile: Tile,
    dom: usize,
}

/// Every vertex of the body with the tile its triangles sit in.
fn scan(body: &SkinnedBody) -> Vec<Vert> {
    let mut out = Vec::new();
    for (part_index, part) in body.parts.iter().enumerate() {
        let mut tiles = vec![Tile::Other; part.mesh.positions.len()];
        for tri in part.mesh.indices.chunks_exact(3) {
            let tile = triangle_tile(part_index, &part.mesh.uv0, tri);
            for i in tri {
                if let Some(slot) = tiles.get_mut(usize::try_from(*i).unwrap_or(usize::MAX)) {
                    *slot = tile;
                }
            }
        }
        for (i, p) in part.mesh.positions.iter().enumerate() {
            let (joints, weights) = (part.skin.joints.get(i), part.skin.weights.get(i));
            let dom = match (joints, weights) {
                (Some(j), Some(w)) => {
                    let best = (0..4).max_by(|a, b| w[*a].total_cmp(&w[*b])).unwrap_or(0);
                    usize::from(j[best])
                }
                _ => 0,
            };
            out.push(Vert {
                p: *p,
                tile: tiles.get(i).copied().unwrap_or(Tile::Other),
                dom,
            });
        }
    }
    out
}

/// A joint's bind position in model space, from its inverse bind (row-vector layout: the joint
/// origin is `-t * R^T`).
pub(super) fn joint_origin(inverse_bind: &[[f32; 4]; 4]) -> V3 {
    let t = inverse_bind[3];
    std::array::from_fn(|j| -(0..3).map(|k| t[k] * inverse_bind[j][k]).sum::<f32>())
}

/// What the kit measures its pieces from: the joints and the body's own vertices.
struct Fit {
    joints: Joints,
    origin: Vec<V3>,
    verts: Vec<Vert>,
}

impl Fit {
    /// Measure `body`; `None` when it lacks a joint the kit hangs on.
    fn of(body: &SkinnedBody) -> Option<Self> {
        Some(Self {
            joints: Joints::find(&body.skeleton)?,
            origin: body.inverse_binds.iter().map(joint_origin).collect(),
            verts: scan(body),
        })
    }

    fn at(&self, joint: usize) -> V3 {
        self.origin.get(joint).copied().unwrap_or([0.0; 3])
    }

    /// Body-tile points whose dominant joint passes `keep`.
    fn body_points(&self, keep: impl Fn(&Vert) -> bool) -> Vec<V3> {
        self.verts
            .iter()
            .filter(|v| v.tile == Tile::Body && keep(v))
            .map(|v| v.p)
            .collect()
    }

    /// The trunk: torso and pelvis, plus the inner shoulder (the part of the shoulder that is
    /// chest and not yet arm).
    fn trunk_points(&self) -> Vec<V3> {
        let j = self.joints;
        self.body_points(|v| {
            v.dom == j.torso
                || v.dom == j.pelvis
                || (j.shoulder.contains(&v.dom) && v.p[0].abs() < TRUNK_HALF_WIDTH_M)
        })
    }

    /// Hips and thighs: what the tunic hangs over.
    fn hip_points(&self) -> Vec<V3> {
        let j = self.joints;
        self.body_points(|v| v.dom == j.pelvis || j.leg_upper.contains(&v.dom))
    }

    /// The scalp and the neck: what the galea sits on.
    fn head_points(&self) -> Vec<V3> {
        self.verts
            .iter()
            .filter(|v| {
                matches!(v.tile, Tile::Head | Tile::Hair)
                    || (v.tile == Tile::Body
                        && v.p[1] > NECK_FROM_Y_M
                        && v.p[0].abs() < NECK_HALF_WIDTH_M)
            })
            .map(|v| v.p)
            .collect()
    }
}

/// Past this side distance a shoulder vertex is deltoid, which the shoulder lames cover.
const TRUNK_HALF_WIDTH_M: f32 = 0.13;
/// Body points above this height and inside the neck width are neck.
const NECK_FROM_Y_M: f32 = 1.50;
/// Half width of the neck, meters.
const NECK_HALF_WIDTH_M: f32 = 0.09;

// -------------------------------------------------------------------------------- shrinkwrap

/// Radius around the vertical axis through `(0, cz)` for each of `n` sectors, from the points
/// within `band` of height `y0`: the farthest point of each sector, scaled so the ring's straight
/// edge between two sectors still clears it. An empty sector takes the blend of its nearest full
/// neighbours; the ring is then made at least the mean of its neighbours so a spike stays a spike
/// of one sector. `None` when no point is in the slice.
fn sector_radii(points: &[V3], cz: f32, y0: f32, band: f32, n: usize) -> Option<Vec<f32>> {
    let step = TAU / fl(n);
    let mut radius = vec![f32::NAN; n];
    let mut put = |sector: usize, r: f32| {
        let slot = &mut radius[sector];
        *slot = if slot.is_nan() { r } else { slot.max(r) };
    };
    for p in points.iter().filter(|p| (p[1] - y0).abs() <= band) {
        let (dx, dz) = (p[0], p[2] - cz);
        let r = dx.hypot(dz) / (0.5 * step).cos();
        let ang = dx.atan2(dz);
        let (near, off) = nearest_sector(ang, n);
        put(near, r);
        let next = if off > 0.0 {
            (near + 1) % n
        } else {
            (near + n - 1) % n
        };
        put(next, r * 0.98);
    }
    if radius.iter().all(|r| r.is_nan()) {
        return None;
    }
    let have: Vec<bool> = radius.iter().map(|r| !r.is_nan()).collect();
    for k in 0..n {
        if have[k] {
            continue;
        }
        let (mut a, mut b) = (1, 1);
        while !have[(k + n - a % n) % n] {
            a += 1;
        }
        while !have[(k + b) % n] {
            b += 1;
        }
        let (ra, rb) = (radius[(k + n - a % n) % n], radius[(k + b) % n]);
        radius[k] = (ra * fl(b) + rb * fl(a)) / fl(a + b);
    }
    let base = radius.clone();
    for (k, r) in radius.iter_mut().enumerate() {
        *r = r.max(0.5 * (base[(k + 1) % n] + base[(k + n - 1) % n]));
    }
    Some(radius)
}

/// The sector of `n` whose centre is nearest the angle `ang` (0 = +Z, growing toward +X), and
/// which side of that centre the angle lies on.
fn nearest_sector(ang: f32, n: usize) -> (usize, f32) {
    let step = TAU / fl(n);
    let wrap = |a: f32| {
        let mut a = a;
        while a > PI {
            a -= TAU;
        }
        while a < -PI {
            a += TAU;
        }
        a
    };
    (0..n)
        .map(|k| (k, wrap(ang - step * fl(k))))
        .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .unwrap_or((0, 0.0))
}

/// A ring of points at height `y`: sector `k` stands `off` outside its radius.
fn ring_at(radius: &[f32], y: f32, off: f32, cz: f32) -> Vec<V3> {
    let n = radius.len();
    radius
        .iter()
        .enumerate()
        .map(|(k, r)| {
            let phi = TAU * fl(k) / fl(n);
            [phi.sin() * (r + off), y, cz + phi.cos() * (r + off)]
        })
        .collect()
}

// ----------------------------------------------------------------------------------- meshes

/// Skin joints and weights for one vertex.
type Skin4 = ([u16; 4], [f32; 4]);

fn joint_id(joint: usize) -> u16 {
    u16::try_from(joint).unwrap_or(0)
}

/// Fully on one joint.
fn skin1(joint: usize) -> Skin4 {
    ([joint_id(joint), 0, 0, 0], [1.0, 0.0, 0.0, 0.0])
}

/// Shared between two joints: `wa` on `a`, the rest on `b`.
fn skin2(a: usize, wa: f32, b: usize) -> Skin4 {
    ([joint_id(a), joint_id(b), 0, 0], [wa, 1.0 - wa, 0.0, 0.0])
}

/// One kit part ready to skin: bind mesh, skin rows, texels and factors.
#[derive(Clone, PartialEq, Debug)]
pub struct KitPart {
    /// Part name for the view audit (`lorica`, `galea`, ...).
    pub name: &'static str,
    /// Bind positions, meters.
    pub positions: Vec<V3>,
    /// Unit normals.
    pub normals: Vec<V3>,
    /// UVs in `0..=1`.
    pub uv0: Vec<[f32; 2]>,
    /// Triangle indices.
    pub indices: Vec<u32>,
    /// Skin joints per vertex.
    pub joints: Vec<[u16; 4]>,
    /// Skin weights per vertex.
    pub weights: Vec<[f32; 4]>,
    /// Base-colour map.
    pub texels: Texels,
    /// Metallic factor.
    pub metallic: f32,
    /// Roughness factor.
    pub roughness: f32,
}

/// A mesh being built.
#[derive(Default)]
struct Mesh {
    pos: Vec<V3>,
    nrm: Vec<V3>,
    uv: Vec<[f32; 2]>,
    joints: Vec<[u16; 4]>,
    weights: Vec<[f32; 4]>,
    idx: Vec<u32>,
}

impl Mesh {
    fn vert(&mut self, p: V3, n: V3, uv: [f32; 2], skin: Skin4) -> u32 {
        self.pos.push(p);
        self.nrm.push(norm(n));
        self.uv.push([uv[0].clamp(0.0, 1.0), uv[1].clamp(0.0, 1.0)]);
        self.joints.push(skin.0);
        self.weights.push(skin.1);
        u32::try_from(self.pos.len() - 1).unwrap_or(u32::MAX)
    }

    fn at(&self, i: u32) -> V3 {
        self.pos[usize::try_from(i).unwrap_or(0)]
    }

    /// One triangle; a sliver with no area is dropped (a collapsed ring makes them).
    fn tri(&mut self, a: u32, b: u32, c: u32) {
        let n = cross(sub(self.at(b), self.at(a)), sub(self.at(c), self.at(a)));
        if dot(n, n) > 1.0e-12 {
            self.idx.extend([a, b, c]);
        }
    }

    fn finish(self, name: &'static str, texels: Texels, metallic: f32, roughness: f32) -> KitPart {
        KitPart {
            name,
            positions: self.pos,
            normals: self.nrm,
            uv0: self.uv,
            indices: self.idx,
            joints: self.joints,
            weights: self.weights,
            texels,
            metallic,
            roughness,
        }
    }
}

/// How a loft over a grid of rings is skinned, mapped and faced.
struct Loft<'a> {
    /// The ring closes on itself (the seam column repeats, so `u` runs `0..=1`).
    wrap: bool,
    /// Draw the inside too (thin pieces seen from both sides).
    two_sided: bool,
    /// A point inside ring `i`, to tell outside from inside.
    centre: &'a dyn Fn(usize) -> V3,
    /// `(ring, column)` to UV.
    uv: &'a dyn Fn(usize, usize) -> [f32; 2],
    /// `(ring, column, point)` to skin rows.
    skin: &'a dyn Fn(usize, usize, V3) -> Skin4,
}

/// Loft a surface over `rings` (each ring the same number of points): smooth normals from the
/// grid, the faces turned to look away from the ring centres, an optional inside layer.
fn loft(mesh: &mut Mesh, rings: &[Vec<V3>], spec: &Loft<'_>) {
    let rows = rings.len();
    let Some(n) = rings.first().map(Vec::len).filter(|n| *n >= 2 && rows >= 2) else {
        return;
    };
    let cols = if spec.wrap { n + 1 } else { n };
    let point = |i: usize, c: usize| rings[i][c % n];
    // Normal of the winding (A, B, C) with A = (i, c), B = (i + 1, c), C = (i + 1, c + 1).
    let normal = |i: usize, c: usize| {
        let (c0, c1) = if spec.wrap {
            ((c + n - 1) % n, (c + 1) % n)
        } else {
            (c.saturating_sub(1), (c + 1).min(n - 1))
        };
        let dk = sub(rings[i][c1], rings[i][c0]);
        let di = sub(
            rings[(i + 1).min(rows - 1)][c % n],
            rings[i.saturating_sub(1)][c % n],
        );
        cross(di, dk)
    };
    let mut score = 0.0f32;
    for i in 0..rows {
        for c in 0..n {
            score += dot(normal(i, c), sub(point(i, c), (spec.centre)(i)));
        }
    }
    let flip = score < 0.0;
    let layers: &[bool] = if spec.two_sided {
        &[false, true]
    } else {
        &[false]
    };
    for inside in layers {
        let sign = if flip != *inside { -1.0 } else { 1.0 };
        let mut ids = vec![vec![0u32; cols]; rows];
        for (i, row) in ids.iter_mut().enumerate() {
            for (c, id) in row.iter_mut().enumerate() {
                let p = point(i, c);
                let nv = scale(norm(normal(i, c)), sign);
                *id = mesh.vert(p, nv, (spec.uv)(i, c), (spec.skin)(i, c % n, p));
            }
        }
        for i in 0..rows - 1 {
            for c in 0..cols - 1 {
                let (a, d, b, e) = (ids[i][c], ids[i][c + 1], ids[i + 1][c], ids[i + 1][c + 1]);
                if sign > 0.0 {
                    mesh.tri(a, b, e);
                    mesh.tri(a, e, d);
                } else {
                    mesh.tri(a, e, b);
                    mesh.tri(a, d, e);
                }
            }
        }
    }
}

/// A one-sided strip between two equally long polylines, its faces turned toward `facing`.
fn strip(mesh: &mut Mesh, a: &[V3], b: &[V3], facing: &dyn Fn(usize) -> V3, skin: Skin4) {
    let uv = |i: usize, row: f32| [fl(i) / fl(a.len().max(2) - 1), row];
    let ia: Vec<u32> = a
        .iter()
        .enumerate()
        .map(|(i, p)| mesh.vert(*p, facing(i), uv(i, 0.0), skin))
        .collect();
    let ib: Vec<u32> = b
        .iter()
        .enumerate()
        .map(|(i, p)| mesh.vert(*p, facing(i), uv(i, 1.0), skin))
        .collect();
    for i in 0..ia.len().saturating_sub(1) {
        let n = cross(
            sub(mesh.at(ib[i]), mesh.at(ia[i])),
            sub(mesh.at(ib[i + 1]), mesh.at(ia[i])),
        );
        if dot(n, facing(i)) >= 0.0 {
            mesh.tri(ia[i], ib[i], ib[i + 1]);
            mesh.tri(ia[i], ib[i + 1], ia[i + 1]);
        } else {
            mesh.tri(ia[i], ib[i + 1], ib[i]);
            mesh.tri(ia[i], ia[i + 1], ib[i + 1]);
        }
    }
}

// -------------------------------------------------------------------------------- the pieces

/// Plates in the lorica's girdle.
const BANDS: usize = 8;
/// Vertices the girdle takes at the start of the lorica part (three rings a plate, one seam
/// column more than sectors); the shoulder lames follow them. Test-only: the builders take the
/// count from the arrays they append.
#[cfg(test)]
pub const GIRDLE_VERTS: usize = BANDS * 3 * (SECTORS + 1);
/// Vertical overlap of two plates, meters.
const BAND_OVERLAP_M: f32 = 0.012;
/// How far the lorica stands off the body at its top edge, meters.
const LORICA_STAND_OFF_M: f32 = 0.020;
/// How far a plate's lower edge flares out over the plate below, meters.
const BAND_FLARE_M: f32 = 0.011;
/// Where the girdle starts above the pelvis joint, meters.
const WAIST_ABOVE_PELVIS_M: f32 = 0.105;
/// Where the collar ends above the shoulder joints, meters.
const COLLAR_ABOVE_SHOULDER_M: f32 = 0.085;

/// Skin rows of a girdle point: torso above the waist, blending to the pelvis below it.
fn trunk_skin(fit: &Fit, y: f32) -> Skin4 {
    let waist = fit.at(fit.joints.pelvis)[1] + WAIST_ABOVE_PELVIS_M;
    let t = smoothstep(waist - 0.03, waist + 0.07, y);
    skin2(fit.joints.torso, t, fit.joints.pelvis)
}

/// The girdle: eight plates, each a short shell around the trunk whose lower edge flares over
/// the plate below.
fn lorica_girdle(fit: &Fit, mesh: &mut Mesh) {
    let trunk = fit.trunk_points();
    let y_waist = fit.at(fit.joints.pelvis)[1] + WAIST_ABOVE_PELVIS_M;
    let y_collar = fit.at(fit.joints.shoulder[0])[1] + COLLAR_ABOVE_SHOULDER_M;
    let h = (y_collar - y_waist + fl(BANDS - 1) * BAND_OVERLAP_M) / fl(BANDS);
    for i in 0..BANDS {
        let top = y_collar - fl(i) * (h - BAND_OVERLAP_M);
        let ys = [top, top - 0.5 * h, top - h];
        let rings: Vec<Vec<V3>> = ys
            .iter()
            .enumerate()
            .filter_map(|(r, y)| {
                let radius = sector_radii(&trunk, 0.0, *y, 0.055, SECTORS)?;
                let off = LORICA_STAND_OFF_M + BAND_FLARE_M * fl(r) / 2.0 + 0.0015 * fl(i);
                Some(ring_at(&radius, *y, off, 0.0))
            })
            .collect();
        let centre = |r: usize| [0.0, ys[r.min(2)], 0.0];
        let uv = |r: usize, c: usize| [fl(c) / fl(SECTORS), fl(r) / 2.0];
        let skin = |_: usize, _: usize, p: V3| trunk_skin(fit, p[1]);
        let spec = Loft {
            wrap: true,
            two_sided: false,
            centre: &centre,
            uv: &uv,
            skin: &skin,
        };
        loft(mesh, &rings, &spec);
    }
}

/// Arc samples across one shoulder lame.
const LAME_ARC: usize = 13;
/// Half the arc a lame covers around the top of the arm, radians.
const LAME_HALF_ARC: f32 = 1.95;
/// Axial length of a lame, meters.
const LAME_LENGTH_M: f32 = 0.078;
/// Where each lame starts along the arm from the shoulder joint, meters (negative is toward the
/// neck, over the deltoid).
const LAME_START_M: [f32; 3] = [-0.045, 0.015, 0.075];

/// The frame of an arm: its axis and the two directions across it (`up` over the top of the arm,
/// `side` along the shoulder line).
struct ArmFrame {
    root: V3,
    axis: V3,
    up: V3,
    side: V3,
}

impl ArmFrame {
    fn new(fit: &Fit, side: usize) -> Self {
        let root = fit.at(fit.joints.arm_upper[side]);
        let axis = norm(sub(fit.at(fit.joints.arm_lower[side]), root));
        let up = norm(sub([0.0, 1.0, 0.0], scale(axis, axis[1])));
        Self {
            root,
            axis,
            up,
            side: cross(axis, up),
        }
    }

    /// `(distance along the arm, distance from its axis, angle around it from the top)` of `p`.
    fn coords(&self, p: V3) -> (f32, f32, f32) {
        let rel = sub(p, self.root);
        let t = dot(rel, self.axis);
        let perp = sub(rel, scale(self.axis, t));
        (
            t,
            dot(perp, perp).sqrt(),
            dot(perp, self.side).atan2(dot(perp, self.up)),
        )
    }

    fn point(&self, t: f32, radius: f32, theta: f32) -> V3 {
        let dir = add(scale(self.up, theta.cos()), scale(self.side, theta.sin()));
        add(add(self.root, scale(self.axis, t)), scale(dir, radius))
    }
}

/// Radius of the arm at distance `t` for each arc sample: the farthest body point near that
/// slice and angle; gaps take the blend of their neighbours.
fn arm_radii(frame: &ArmFrame, points: &[V3], t: f32) -> Vec<f32> {
    let mut radius = vec![f32::NAN; LAME_ARC];
    let span = 2.0 * LAME_HALF_ARC;
    for p in points {
        let (pt, r, theta) = frame.coords(*p);
        if (pt - t).abs() > 0.05 || theta.abs() > LAME_HALF_ARC + 0.2 {
            continue;
        }
        let f = (theta + LAME_HALF_ARC) / span * fl(LAME_ARC - 1);
        for sample in [f.floor(), f.ceil()] {
            let Some(slot) = (0..LAME_ARC).find(|m| (fl(*m) - sample).abs() < 0.25) else {
                continue;
            };
            radius[slot] = if radius[slot].is_nan() {
                r
            } else {
                radius[slot].max(r)
            };
        }
    }
    for m in 0..LAME_ARC {
        if !radius[m].is_nan() {
            continue;
        }
        let before = (0..m).rev().find(|k| !radius[*k].is_nan());
        let after = (m + 1..LAME_ARC).find(|k| !radius[*k].is_nan());
        radius[m] = match (before, after) {
            (Some(a), Some(b)) => (radius[a] * fl(b - m) + radius[b] * fl(m - a)) / fl(b - a),
            (Some(k), None) | (None, Some(k)) => radius[k],
            (None, None) => 0.07,
        };
    }
    radius
}

/// Three lames over each shoulder, nested down the upper arm like the real guard.
fn shoulder_lames(fit: &Fit, mesh: &mut Mesh) {
    let j = fit.joints;
    for side in 0..2 {
        let frame = ArmFrame::new(fit, side);
        let arm = fit.body_points(|v| v.dom == j.shoulder[side] || v.dom == j.arm_upper[side]);
        for (li, start) in LAME_START_M.iter().enumerate() {
            let ts = [*start, start + 0.5 * LAME_LENGTH_M, start + LAME_LENGTH_M];
            let rings: Vec<Vec<V3>> = ts
                .iter()
                .enumerate()
                .map(|(r, t)| {
                    let radii = arm_radii(&frame, &arm, *t);
                    let off = 0.022 + 0.005 * fl(li) + 0.007 * fl(r) / 2.0;
                    radii
                        .iter()
                        .enumerate()
                        .map(|(m, rad)| {
                            let theta =
                                -LAME_HALF_ARC + 2.0 * LAME_HALF_ARC * fl(m) / fl(LAME_ARC - 1);
                            frame.point(*t, rad + off, theta)
                        })
                        .collect()
                })
                .collect();
            // The first lame rides the shoulder, the last the upper arm, the middle both.
            let on_arm = [0.0, 0.45, 0.75][li];
            let skin =
                move |_: usize, _: usize, _: V3| skin2(j.arm_upper[side], on_arm, j.shoulder[side]);
            let centre = |r: usize| add(frame.root, scale(frame.axis, ts[r.min(2)]));
            let uv = |r: usize, c: usize| [0.06 + 0.14 * fl(c) / fl(LAME_ARC - 1), fl(r) / 2.0];
            let spec = Loft {
                wrap: false,
                two_sided: false,
                centre: &centre,
                uv: &uv,
                skin: &skin,
            };
            loft(mesh, &rings, &spec);
        }
    }
}

/// Height of the tunic's hem, meters: above the knee.
const TUNIC_HEM_Y_M: f32 = 0.60;
/// Rings down the tunic.
const TUNIC_RINGS: usize = 8;

/// The tunic: a belt and a red skirt that clears the hips and both thighs.
fn tunic(fit: &Fit) -> Mesh {
    let mut mesh = Mesh::default();
    let mut hull = fit.hip_points();
    hull.extend(fit.trunk_points());
    let top = fit.at(fit.joints.pelvis)[1] + 0.10;
    let rings: Vec<Vec<V3>> = (0..TUNIC_RINGS)
        .filter_map(|r| {
            let t = fl(r) / fl(TUNIC_RINGS - 1);
            let y = top + (TUNIC_HEM_Y_M - top) * t;
            let radius = sector_radii(&hull, 0.0, y, 0.09, SECTORS)?;
            Some(ring_at(&radius, y, 0.022 + 0.040 * t, 0.0))
        })
        .collect();
    let j = fit.joints;
    let centre = |r: usize| [0.0, rings[r.min(rings.len() - 1)][0][1], 0.0];
    let uv = |r: usize, c: usize| [fl(c) / fl(SECTORS), fl(r) / fl(TUNIC_RINGS - 1)];
    // The sides follow their own thigh more and more toward the hem; the front and back stay on
    // the pelvis, so the tunic swings with the legs without tearing across them.
    let skin = |r: usize, c: usize, _: V3| {
        let t = fl(r) / fl(TUNIC_RINGS - 1);
        let side = (TAU * fl(c) / fl(SECTORS)).sin();
        let leg = side.abs() * 0.65 * t;
        let thigh = if side >= 0.0 {
            j.leg_upper[0]
        } else {
            j.leg_upper[1]
        };
        skin2(j.pelvis, 1.0 - leg, thigh)
    };
    let spec = Loft {
        wrap: true,
        two_sided: true,
        centre: &centre,
        uv: &uv,
        skin: &skin,
    };
    loft(&mut mesh, &rings, &spec);
    mesh
}

/// Rim of the galea around the head: the brow in front, above the ears at the sides, the nape
/// behind. Angles from +Z.
fn rim_y(phi: f32) -> f32 {
    let a = wrap_pi(phi).abs();
    if a < 0.75 {
        1.728
    } else if a < 1.6 {
        1.728 + (1.652 - 1.728) * smoothstep(0.75, 1.6, a)
    } else {
        1.652 + (1.605 - 1.652) * smoothstep(1.6, 2.4, a)
    }
}

fn wrap_pi(a: f32) -> f32 {
    let mut a = a;
    while a > PI {
        a -= TAU;
    }
    while a < -PI {
        a += TAU;
    }
    a
}

/// How far a flange hangs under the rim: the cheek plates at the sides, the neck guard behind.
fn hang(phi: f32) -> f32 {
    let a = wrap_pi(phi).abs();
    let cheek = 0.052 * smoothstep(0.95, 1.15, a) * (1.0 - smoothstep(1.35, 1.55, a));
    let nape = 0.088 * smoothstep(1.95, 2.45, a);
    cheek.max(nape)
}

/// How far the flange flares away from the head at its lower edge.
fn flare(phi: f32) -> f32 {
    if wrap_pi(phi).abs() > 1.8 {
        0.034
    } else {
        0.012
    }
}

/// Height of the dome's apex, meters.
const DOME_TOP_Y_M: f32 = 1.826;
/// Margin of the dome off the scalp, meters.
const DOME_STAND_OFF_M: f32 = 0.011;
/// Depth centre of the head, meters.
const HEAD_CZ_M: f32 = 0.01;
/// Heights of the dome's rings and, for the top rings, how much of the radius is left (the cap).
const DOME_RINGS: [(f32, f32); 10] = [
    (1.60, 1.0),
    (1.655, 1.0),
    (1.705, 1.0),
    (1.75, 1.0),
    (1.785, 1.0),
    (1.805, 1.0),
    (1.8125, 0.86),
    (1.818, 0.62),
    (1.822, 0.32),
    (DOME_TOP_Y_M, 0.0),
];

/// The galea and its crest. Returns the dome part and the crest part.
fn galea(fit: &Fit) -> (Mesh, Mesh) {
    let head = fit.head_points();
    let skin = skin1(fit.joints.head);
    let radii_at = |y: f32| sector_radii(&head, HEAD_CZ_M, y.clamp(1.60, 1.80), 0.05, SECTORS);
    let mut dome = Mesh::default();
    let centre = |_: usize| [0.0, 1.70, HEAD_CZ_M];
    let mut dome_rings: Vec<Vec<V3>> = Vec::new();
    for (y, cap) in DOME_RINGS {
        let ring: Vec<V3> = (0..SECTORS)
            .map(|k| {
                let phi = TAU * fl(k) / fl(SECTORS);
                let yy = y.max(rim_y(phi));
                let r = if cap == 0.0 {
                    0.004
                } else {
                    let row = radii_at(yy.min(1.805)).map_or(0.08, |r| r[k]);
                    (row + DOME_STAND_OFF_M) * cap
                };
                [phi.sin() * r, yy, HEAD_CZ_M + phi.cos() * r]
            })
            .collect();
        dome_rings.push(ring);
    }
    let rows = dome_rings.len();
    let uv = |r: usize, c: usize| [fl(c) / fl(SECTORS), 0.04 + 0.62 * fl(r) / fl(rows - 1)];
    let one = |_: usize, _: usize, _: V3| skin;
    let spec = Loft {
        wrap: true,
        two_sided: false,
        centre: &centre,
        uv: &uv,
        skin: &one,
    };
    loft(&mut dome, &dome_rings, &spec);
    // The flanges: three rings from the rim down, two-sided.
    let flange: Vec<Vec<V3>> = [0.0f32, 0.5, 1.0]
        .iter()
        .map(|t| {
            (0..SECTORS)
                .map(|k| {
                    let phi = TAU * fl(k) / fl(SECTORS);
                    let y0 = rim_y(phi);
                    let r = radii_at(y0).map_or(0.08, |r| r[k]) + DOME_STAND_OFF_M + flare(phi) * t;
                    [phi.sin() * r, y0 - hang(phi) * t, HEAD_CZ_M + phi.cos() * r]
                })
                .collect()
        })
        .collect();
    let uv = |r: usize, c: usize| [fl(c) / fl(SECTORS), 0.76 + 0.2 * fl(r) / 2.0];
    let spec = Loft {
        wrap: true,
        two_sided: true,
        centre: &centre,
        uv: &uv,
        skin: &one,
    };
    loft(&mut dome, &flange, &spec);
    let crest = crest(&dome_rings, skin);
    (dome, crest)
}

/// Half thickness of the crest, meters.
const CREST_HALF_THICKNESS_M: f32 = 0.0075;
/// Height of the crest over the dome at its middle, meters.
const CREST_HEIGHT_M: f32 = 0.058;

/// The crest: a fin over the dome's own front-to-back outline, tallest in the middle.
fn crest(dome: &[Vec<V3>], skin: Skin4) -> Mesh {
    let mut outline: Vec<[f32; 2]> = dome.iter().map(|r| [r[0][2], r[0][1]]).collect();
    outline.extend(
        dome.iter()
            .rev()
            .skip(1)
            .map(|r| [r[SECTORS / 2][2], r[SECTORS / 2][1]]),
    );
    let m = outline.len();
    let top: Vec<[f32; 2]> = (0..m)
        .map(|i| {
            let t = fl(i) / fl(m - 1);
            let height = CREST_HEIGHT_M * (PI * t).sin().max(0.0).powf(1.1) + 0.004;
            let (p0, p1) = (outline[i.saturating_sub(1)], outline[(i + 1).min(m - 1)]);
            let tangent = norm([0.0, p1[1] - p0[1], p1[0] - p0[0]]);
            let (mut nz, mut ny) = (tangent[1], -tangent[2]);
            // Away from the middle of the head.
            if nz * (outline[i][0] - HEAD_CZ_M) + ny * (outline[i][1] - 1.70) < 0.0 {
                nz = -nz;
                ny = -ny;
            }
            [outline[i][0] + nz * height, outline[i][1] + ny * height]
        })
        .collect();
    let line = |side: f32, from: &[[f32; 2]]| -> Vec<V3> {
        from.iter().map(|p| [side, p[1], p[0]]).collect()
    };
    let mut mesh = Mesh::default();
    let t = CREST_HALF_THICKNESS_M;
    strip(
        &mut mesh,
        &line(-t, &outline),
        &line(-t, &top),
        &|_| [-1.0, 0.0, 0.0],
        skin,
    );
    strip(
        &mut mesh,
        &line(t, &outline),
        &line(t, &top),
        &|_| [1.0, 0.0, 0.0],
        skin,
    );
    strip(
        &mut mesh,
        &line(-t, &top),
        &line(t, &top),
        &|_| [0.0, 1.0, 0.0],
        skin,
    );
    mesh
}

/// Below this height a boot triangle is sandal, above it greave, meters.
const SANDAL_TOP_Y_M: f32 = 0.15;

/// The boots re-made: the lower foot as leather sandals, everything above the ankle as bronze
/// greaves. Both keep the boot's own triangles, normals and skin rows; only the UVs are new.
fn boots(body: &SkinnedBody) -> (Mesh, Mesh) {
    let (mut greaves, mut sandals) = (Mesh::default(), Mesh::default());
    for (pi, part) in body.parts.iter().enumerate() {
        for tri in part.mesh.indices.chunks_exact(3) {
            if triangle_tile(pi, &part.mesh.uv0, tri) != Tile::Boots {
                continue;
            }
            let rows: Vec<usize> = tri
                .iter()
                .map(|i| usize::try_from(*i).unwrap_or(0))
                .collect();
            let p: Vec<V3> = rows.iter().map(|r| part.mesh.positions[*r]).collect();
            let height = (p[0][1] + p[1][1] + p[2][1]) / 3.0;
            let face = cross(sub(p[1], p[0]), sub(p[2], p[0]));
            let across_x = face[0].abs() > face[2].abs();
            let target = if height < SANDAL_TOP_Y_M {
                &mut sandals
            } else {
                &mut greaves
            };
            let ids: Vec<u32> = rows
                .iter()
                .zip(&p)
                .map(|(r, p)| {
                    let u = (0.5 + (if across_x { p[2] } else { p[0] }) * 1.6).clamp(0.02, 0.98);
                    let v = if height < SANDAL_TOP_Y_M {
                        (1.0 - p[1] / SANDAL_TOP_Y_M).clamp(0.02, 0.98)
                    } else {
                        (0.04 + (0.52 - p[1])).clamp(0.02, 0.62)
                    };
                    let skin = (part.skin.joints[*r], part.skin.weights[*r]);
                    target.vert(*p, part.mesh.normals[*r], [u, v], skin)
                })
                .collect();
            target.tri(ids[0], ids[1], ids[2]);
        }
    }
    (greaves, sandals)
}

// ------------------------------------------------------------------------------------- dress

/// Triangles of these tiles leave the body: the plate suit, the gauntlets, the helm and the boots
/// (the boots come back re-made as greaves and sandals).
const STRIPPED: [Tile; 4] = [Tile::Armor, Tile::Gloves, Tile::Helmet, Tile::Boots];

/// Take the stripped tiles' triangles out of the body parts (their vertices stay, unreferenced).
fn strip_suit(body: &mut SkinnedBody) {
    for (pi, part) in body.parts.iter_mut().enumerate() {
        let uv = &part.mesh.uv0;
        let mut kept = Vec::with_capacity(part.mesh.indices.len());
        for tri in part.mesh.indices.chunks_exact(3) {
            if !STRIPPED.contains(&triangle_tile(pi, uv, tri)) {
                kept.extend_from_slice(tri);
            }
        }
        part.mesh.indices = kept;
    }
}

/// Fewest triangles of the plate suit and of the helm a body must carry to be the hero (the
/// suit is 1448, the helm 408; the opponents' light armour has no such helm).
const SUIT_MIN_TRIS: usize = 1000;
const HELM_MIN_TRIS: usize = 300;

/// The hero's heavy suit and helm are in the atlas tiles this kit takes off.
fn wears_the_heavy_suit(body: &SkinnedBody) -> bool {
    tile_triangles(body, Tile::Armor) >= SUIT_MIN_TRIS
        && tile_triangles(body, Tile::Helmet) >= HELM_MIN_TRIS
}

/// Dress the hero: take the plate suit off `body` and return the Roman kit's parts. Anything that
/// is not the hero (or has not the hero's four atlas parts and joints) gives nothing and is left
/// as it is.
pub fn dress_hero(body: &mut SkinnedBody, variant: &str) -> Vec<KitPart> {
    if variant != HERO_VARIANT || body.parts.len() < 4 || !wears_the_heavy_suit(body) {
        return Vec::new();
    }
    let Some(fit) = Fit::of(body) else {
        return Vec::new();
    };
    let mut lorica = Mesh::default();
    lorica_girdle(&fit, &mut lorica);
    shoulder_lames(&fit, &mut lorica);
    let (dome, crest) = galea(&fit);
    let (greaves, sandals) = boots(body);
    strip_suit(body);
    vec![
        lorica.finish("lorica", tex::lorica(), 0.85, 0.42),
        tunic(&fit).finish("tunic", tex::tunic(), 0.0, 0.85),
        dome.finish("galea", tex::bronze(), 0.85, 0.38),
        crest.finish("plume", tex::plume(), 0.0, 0.9),
        greaves.finish("greaves", tex::bronze(), 0.8, 0.4),
        sandals.finish("caligae", tex::leather(), 0.0, 0.8),
    ]
}

#[cfg(test)]
#[path = "fighter_kit_tests.rs"]
mod tests;
