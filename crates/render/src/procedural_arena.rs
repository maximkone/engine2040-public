//! procedural_arena: a lit arena ring with eight skinned fighters, built from code alone (WEB5).
//!
//! The browser has no files to read yet, so its first arena frame draws this stand-in through the
//! same presenter the Windows window uses: a sand floor with a red fighting ring, a stone wall
//! with a gate gap, torch pillars, and eight box-built fighters bound to two joints (legs, and
//! the torso with head, arms and sword) that sway on the GPU skin path. The sky, sun shadow,
//! height fog, image-based light and bloom come from the presenter, not from here. The native
//! window and the headless twin can draw the same list, so a browser frame has a native twin of
//! the same camera and time.
//!
//! Units: meters, radians, seconds. Determinism: the same build gives the same bytes and the
//! same time gives the same camera and pose; pixels are presentation and never enter a
//! simulation hash. Errors are typed [`ArenaError`].
//! Caller: `crates/web` (the browser arena frame) and the native twin tests below.

use std::f32::consts::TAU;
use std::fmt;

use glam::{Mat3, Vec3};

use crate::present::{DrawItem, DrawList, MeshKey};
use crate::v1::{self, Camera, DirectionalLight, MeshData, PbrMaterial};
use crate::v1_skin::SkinWeights;

/// Fighters in the ring: the arena count.
pub const FIGHTERS: usize = 8;
/// Mesh id base of the stand-in parts. Distinct from the cube (1), the dressing, staged-fight,
/// fighter-look, arena-kit and twin-fighter bases.
pub const MESH_ID_BASE: u64 = 0x7EB0_0000;
/// Radius of the circle the fighters stand on, meters.
pub const FIGHTER_RING_M: f32 = 3.0;
/// Radius of the stone wall, meters.
pub const WALL_RADIUS_M: f32 = 11.0;
/// Joints every fighter is bound to: 0 legs, 1 torso, head, arms and sword.
pub const JOINTS: usize = 2;

/// Wall boxes around the circle; the first two are left out as the gate.
const WALL_SEGMENTS: usize = 40;
/// Every fifth wall segment carries a torch pillar.
const PILLAR_EVERY: usize = 5;
/// Height of the hips, where the torso joint pivots.
const HIP_Y_M: f32 = 0.88;

/// A row-major 4x4 joint matrix in the AN1 layout `Presenter::write_skin_block` takes: rows are
/// the images of the axes, the last row is the translation.
pub type JointMatrix = [[f32; 4]; 4];

/// The stand-in could not be built or drawn.
#[derive(Clone, PartialEq, Debug)]
pub struct ArenaError(pub String);

impl fmt::Display for ArenaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "procedural arena: {}", self.0)
    }
}

impl std::error::Error for ArenaError {}

fn arena_err(what: &str, err: impl fmt::Display) -> ArenaError {
    ArenaError(format!("{what}: {err}"))
}

/// One resident mesh of the stand-in and how it draws.
#[derive(Clone, Debug)]
pub struct Part {
    /// Cache key the presenter holds the mesh under.
    pub key: MeshKey,
    /// Bind-pose geometry in world meters.
    pub mesh: MeshData,
    /// Joint rows, `Some` for a fighter (drawn through the skinned pipeline).
    pub skin: Option<SkinWeights>,
    /// Skin block of a fighter (its index), 0 for a static part.
    pub block: usize,
    /// Material factors of the draw.
    pub material: PbrMaterial,
}

/// Where one fighter stands: feet position and yaw (local +Z faces the ring centre).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spot {
    feet: [f32; 3],
    yaw: f32,
}

/// The built stand-in: meshes in draw order, then nothing else to hold.
#[derive(Clone, Debug)]
pub struct ProceduralArena {
    parts: Vec<Part>,
    spots: Vec<Spot>,
}

/// Mesh under construction: boxes and flat shapes appended into one vertex list.
#[derive(Default)]
struct MeshBuilder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    joints: Vec<u16>,
}

/// The faces of a unit box as `(outward normal, along, up)`, with `along x up = normal` so the
/// corners `(-,-) (+,-) (+,+) (-,+)` run counter-clockwise seen from outside.
const BOX_FACES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
    ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
    ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
    ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
    ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
];
const BOX_UVS: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];

impl MeshBuilder {
    /// Append a box of `size` centred at `center`, turned by `yaw` about +Y, bound to `joint`.
    /// Six faces of four corners each, wound counter-clockwise seen from outside: the forward
    /// pipeline culls back faces.
    fn push_box(&mut self, center: [f32; 3], size: [f32; 3], yaw: f32, joint: u16) {
        let (s, c) = yaw.sin_cos();
        let turn = |v: [f32; 3]| [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c];
        for (normal, along, up) in BOX_FACES {
            let base = u32::try_from(self.positions.len()).unwrap_or(u32::MAX);
            for (i, (a, b)) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)]
                .into_iter()
                .enumerate()
            {
                let unit: [f32; 3] =
                    std::array::from_fn(|k| normal[k] * 0.5 + along[k] * a + up[k] * b);
                let local = turn([unit[0] * size[0], unit[1] * size[1], unit[2] * size[2]]);
                self.positions.push([
                    center[0] + local[0],
                    center[1] + local[1],
                    center[2] + local[2],
                ]);
                self.normals.push(turn(normal));
                self.uvs.push(BOX_UVS[i]);
                self.joints.push(joint);
            }
            self.indices
                .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    /// Append one upward-facing flat triangle at height `y`; the winding is fixed to face +Y.
    fn push_flat_tri(&mut self, a: [f32; 2], b: [f32; 2], c: [f32; 2], y: f32, uv_scale: f32) {
        let ab = [b[0] - a[0], b[1] - a[1]];
        let ac = [c[0] - a[0], c[1] - a[1]];
        // The Y component of (b - a) x (c - a) for points in the XZ plane is az*bx - ax*bz.
        let up = ab[1] * ac[0] - ab[0] * ac[1];
        let (second, third) = if up > 0.0 { (b, c) } else { (c, b) };
        let base = u32::try_from(self.positions.len()).unwrap_or(u32::MAX);
        for p in [a, second, third] {
            self.positions.push([p[0], y, p[1]]);
            self.normals.push([0.0, 1.0, 0.0]);
            self.uvs.push([
                (p[0] * uv_scale + 0.5).clamp(0.0, 1.0),
                (p[1] * uv_scale + 0.5).clamp(0.0, 1.0),
            ]);
            self.joints.push(0);
        }
        self.indices.extend([base, base + 1, base + 2]);
    }

    /// A flat disc of `radius` at height `y`.
    fn push_disc(&mut self, radius: f32, segments: usize, y: f32, uv_scale: f32) {
        for i in 0..segments {
            let a0 = TAU * unit(i) / unit(segments);
            let a1 = TAU * unit(i + 1) / unit(segments);
            self.push_flat_tri(
                [0.0, 0.0],
                [radius * a0.cos(), radius * a0.sin()],
                [radius * a1.cos(), radius * a1.sin()],
                y,
                uv_scale,
            );
        }
    }

    /// A flat ring between `inner` and `outer` at height `y`.
    fn push_annulus(&mut self, inner: f32, outer: f32, segments: usize, y: f32, uv_scale: f32) {
        for i in 0..segments {
            let a0 = TAU * unit(i) / unit(segments);
            let a1 = TAU * unit(i + 1) / unit(segments);
            let p = |r: f32, a: f32| [r * a.cos(), r * a.sin()];
            self.push_flat_tri(p(inner, a0), p(outer, a0), p(outer, a1), y, uv_scale);
            self.push_flat_tri(p(inner, a0), p(outer, a1), p(inner, a1), y, uv_scale);
        }
    }

    fn finish(self) -> Result<(MeshData, Vec<u16>), ArenaError> {
        let mesh = v1::upload_mesh(&self.positions, &self.normals, &self.uvs, &self.indices)
            .map_err(|e| arena_err("mesh", e))?;
        Ok((mesh, self.joints))
    }
}

/// One-influence skin rows: every vertex follows its single joint at full weight.
fn rigid_skin(joints: &[u16]) -> SkinWeights {
    SkinWeights {
        joints: joints.iter().map(|j| [*j, 0, 0, 0]).collect(),
        weights: joints.iter().map(|_| [1.0, 0.0, 0.0, 0.0]).collect(),
    }
}

fn material(base_color: [f32; 3], roughness: f32, exposure: f32) -> PbrMaterial {
    PbrMaterial {
        base_color,
        metallic: 0.0,
        roughness,
        exposure,
    }
}

/// Body colours, cycled by fighter so neighbours differ: red, blue, green, gold (linear).
const TEAM_COLORS: [[f32; 3]; 4] = [
    [0.60, 0.10, 0.08],
    [0.10, 0.30, 0.65],
    [0.10, 0.48, 0.22],
    [0.70, 0.52, 0.10],
];

/// Local boxes of one fighter: `(centre, size, joint)` with x right, y up, z forward.
const FIGHTER_BOXES: [([f32; 3], [f32; 3], u16); 7] = [
    ([-0.15, 0.43, 0.0], [0.24, 0.86, 0.28], 0),
    ([0.15, 0.43, 0.0], [0.24, 0.86, 0.28], 0),
    ([0.0, 1.17, 0.0], [0.56, 0.62, 0.32], 1),
    ([0.0, 1.62, 0.02], [0.26, 0.28, 0.26], 1),
    ([-0.38, 1.15, 0.0], [0.14, 0.62, 0.16], 1),
    ([0.38, 1.15, 0.0], [0.14, 0.62, 0.16], 1),
    ([0.44, 1.05, 0.42], [0.05, 0.05, 0.90], 1),
];

/// A small count or index as a float: the shapes here never pass `u16::MAX`.
fn unit(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

/// A frame side as a float (render sizes stop at 2048).
fn unit16(n: u32) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

/// A static mesh and its material, before it gets a key.
type Scenery = (MeshData, PbrMaterial);

/// The floor, the fighting ring, the wall, its pillars and the torch heads.
fn scenery() -> Result<Vec<Scenery>, ArenaError> {
    let mut floor = MeshBuilder::default();
    floor.push_disc(WALL_RADIUS_M + 3.0, 72, 0.0, 1.0 / 28.0);
    let mut ring = MeshBuilder::default();
    ring.push_annulus(
        FIGHTER_RING_M + 1.6,
        FIGHTER_RING_M + 1.9,
        72,
        0.012,
        1.0 / 28.0,
    );
    let mut wall = MeshBuilder::default();
    let mut pillars = MeshBuilder::default();
    let mut torches = MeshBuilder::default();
    let step = TAU / unit(WALL_SEGMENTS);
    for i in 2..WALL_SEGMENTS {
        let theta = step * unit(i);
        let (s, c) = theta.sin_cos();
        let width = WALL_RADIUS_M * step * 1.05;
        // Local x runs along the tangent (-sin, cos).
        let yaw = (-c).atan2(-s);
        wall.push_box(
            [WALL_RADIUS_M * c, 1.7, WALL_RADIUS_M * s],
            [width, 3.4, 0.8],
            yaw,
            0,
        );
        if i % PILLAR_EVERY == 0 {
            let r = WALL_RADIUS_M - 0.1;
            pillars.push_box([r * c, 2.5, r * s], [1.0, 5.0, 1.0], yaw, 0);
            torches.push_box([r * c, 5.2, r * s], [0.34, 0.34, 0.34], yaw, 0);
        }
    }
    // Warm and bright: an exposure above 1 pushes the torch heads over the bloom threshold.
    let pieces = [
        (floor, material([0.55, 0.45, 0.30], 0.95, 1.0)),
        (ring, material([0.32, 0.05, 0.04], 0.8, 1.0)),
        (wall, material([0.42, 0.40, 0.37], 0.9, 1.0)),
        (pillars, material([0.30, 0.28, 0.26], 0.9, 1.0)),
        (torches, material([1.0, 0.55, 0.15], 0.5, 3.0)),
    ];
    let mut out = Vec::with_capacity(pieces.len());
    for (builder, mat) in pieces {
        out.push((builder.finish()?.0, mat));
    }
    Ok(out)
}

/// Fighter `slot`: where it stands, its bind-pose mesh in world meters and its joint rows.
fn fighter(slot: usize) -> Result<(Spot, MeshData, SkinWeights), ArenaError> {
    let theta = TAU * unit(slot) / unit(FIGHTERS);
    let feet = [
        FIGHTER_RING_M * theta.cos(),
        0.0,
        FIGHTER_RING_M * theta.sin(),
    ];
    // Face the centre: local +Z maps to (sin yaw, 0, cos yaw).
    let yaw = (-feet[0]).atan2(-feet[2]);
    let (s, c) = yaw.sin_cos();
    let mut body = MeshBuilder::default();
    for (centre, size, joint) in FIGHTER_BOXES {
        let at = [
            feet[0] + centre[0] * c + centre[2] * s,
            feet[1] + centre[1],
            feet[2] - centre[0] * s + centre[2] * c,
        ];
        body.push_box(at, size, yaw, joint);
    }
    let (mesh, joints) = body.finish()?;
    Ok((Spot { feet, yaw }, mesh, rigid_skin(&joints)))
}

impl ProceduralArena {
    /// Build every mesh of the stand-in. Pure CPU work, no device.
    pub fn build() -> Result<Self, ArenaError> {
        let mut parts = Vec::new();
        let mut spots = Vec::with_capacity(FIGHTERS);
        let key = |n: usize| MeshKey::new(MESH_ID_BASE + u64::try_from(n).unwrap_or(0), [0xA7; 32]);
        for (mesh, material) in scenery()? {
            parts.push(Part {
                key: key(parts.len()),
                mesh,
                skin: None,
                block: 0,
                material,
            });
        }
        for slot in 0..FIGHTERS {
            let (spot, mesh, skin) = fighter(slot)?;
            spots.push(spot);
            parts.push(Part {
                key: key(parts.len()),
                mesh,
                skin: Some(skin),
                block: slot,
                material: material(TEAM_COLORS[slot % TEAM_COLORS.len()], 0.7, 1.0),
            });
        }
        Ok(Self { parts, spots })
    }

    /// The meshes in draw order: floor, ring, wall, pillars, torches, then the fighters.
    #[must_use]
    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    /// The camera at time `t_s`: a slow circle inside the wall, looking at the ring.
    pub fn camera(&self, width: u32, height: u32, t_s: f32) -> Result<Camera, ArenaError> {
        if width == 0 || height == 0 {
            return Err(ArenaError(format!("frame size {width}x{height}")));
        }
        let phi = 0.15 * t_s + 0.6;
        let mut camera = Camera::new(
            [8.2 * phi.cos(), 3.4, 8.2 * phi.sin()],
            [0.0, 1.1, 0.0],
            [0.0, 1.0, 0.0],
            58.0_f32.to_radians(),
        );
        camera.aspect = unit16(width) / unit16(height);
        camera.far = 120.0;
        camera.validate().map_err(|e| arena_err("camera", e))?;
        Ok(camera)
    }

    /// A camera that frames two fighter feet positions: on the perpendicular bisector of the
    /// pair, high and far enough that both fit the frame. Falls back to the ring camera when a
    /// position is not finite or the pair has no direction, so a frame always has a camera.
    pub fn framing_camera(
        &self,
        width: u32,
        height: u32,
        a: [f32; 3],
        b: [f32; 3],
    ) -> Result<Camera, ArenaError> {
        let ring = self.camera(width, height, 0.0)?;
        if !a.iter().chain(b.iter()).all(|v| v.is_finite()) {
            return Ok(ring);
        }
        let (a, b) = (Vec3::from_array(a), Vec3::from_array(b));
        let mid = (a + b) * 0.5;
        let Some(along) = (b - a).try_normalize() else {
            return Ok(ring);
        };
        let Some(side) = along.cross(Vec3::Y).try_normalize() else {
            return Ok(ring);
        };
        let dist = ((b - a).length() * 1.8).clamp(6.0, 20.0);
        // The eye looks outward from near the ring centre at the pair, so the fighters are
        // framed against the wall instead of silhouetted on it; when the pair is at the centre
        // there is no inward direction, so the perpendicular side is used.
        let inward = (-mid).try_normalize().unwrap_or(side);
        let mut eye = mid + inward * dist + Vec3::Y * (dist * 0.45);
        // Keep the eye inside the wall, whatever the pair does: clamp the horizontal radius.
        let flat = Vec3::new(eye.x, 0.0, eye.z);
        let radius = flat.length();
        let limit = WALL_RADIUS_M - 1.5;
        if radius > limit {
            let k = limit / radius;
            eye = Vec3::new(flat.x * k, eye.y, flat.z * k);
        }
        let mut camera = Camera::new(
            eye.to_array(),
            (mid + Vec3::Y * 0.9).to_array(),
            [0.0, 1.0, 0.0],
            58.0_f32.to_radians(),
        );
        camera.aspect = unit16(width) / unit16(height);
        camera.far = 200.0;
        if camera.validate().is_err() {
            return Ok(ring);
        }
        Ok(camera)
    }

    /// The warm key light the twin scenes use.
    #[must_use]
    pub fn light() -> DirectionalLight {
        DirectionalLight {
            direction: Vec3::new(-0.45, 0.75, 0.55),
            color: Vec3::new(1.0, 0.96, 0.90),
            intensity: 3.0,
        }
    }

    /// The draw list for a `width` x `height` frame at time `t_s`, one draw per part.
    pub fn list_at(&self, width: u32, height: u32, t_s: f32) -> Result<DrawList, ArenaError> {
        self.list_of(width, height, t_s, |_| true)
    }

    /// The same list without the fighter parts: the ring, floor and props a played match keeps
    /// while the match's own bodies are staged over it (WEB15). The camera stays the stand-in's.
    pub fn list_scenery(&self, width: u32, height: u32, t_s: f32) -> Result<DrawList, ArenaError> {
        self.list_of(width, height, t_s, |part| part.skin.is_none())
    }

    /// One draw per part `keep` accepts, under the stand-in camera.
    fn list_of(
        &self,
        width: u32,
        height: u32,
        t_s: f32,
        keep: impl Fn(&Part) -> bool,
    ) -> Result<DrawList, ArenaError> {
        Ok(DrawList {
            camera: self.camera(width, height, t_s)?,
            light: Self::light(),
            items: self
                .parts
                .iter()
                .filter(|part| keep(part))
                .map(|part| DrawItem {
                    mesh: part.key,
                    material: part.material.clone(),
                    base_color: None,
                    normal_map: None,
                    orm: None,
                    emissive: None,
                })
                .collect(),
        })
    }

    /// Joint matrices of fighter `slot` at time `t_s`: the legs stay planted, the torso leans
    /// and twists about the hips, each fighter on its own phase. `None` past the last fighter.
    #[must_use]
    pub fn joint_matrices(&self, slot: usize, t_s: f32) -> Option<[JointMatrix; JOINTS]> {
        let spot = self.spots.get(slot)?;
        let phase = unit(slot) * 0.8;
        let lean = 0.10 * (1.7 * t_s + phase).sin();
        let twist = 0.14 * (1.1 * t_s + 2.0 * phase).sin();
        let (s, c) = spot.yaw.sin_cos();
        let right = Vec3::new(c, 0.0, -s);
        let rotation = Mat3::from_axis_angle(right, lean) * Mat3::from_axis_angle(Vec3::Y, twist);
        let pivot = Vec3::new(spot.feet[0], HIP_Y_M, spot.feet[2]);
        Some([IDENTITY, about_pivot(rotation, pivot)])
    }
}

/// The identity joint matrix: the bind pose.
pub const IDENTITY: JointMatrix = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// The AN1 row-vector matrix of `rotation` (a column-convention rotation) about `pivot`: a point
/// `v` goes to `rotation * (v - pivot) + pivot`, written so that `v_row * M` gives it.
fn about_pivot(rotation: Mat3, pivot: Vec3) -> JointMatrix {
    let moved = pivot - rotation * pivot;
    let row = |axis: Vec3, w: f32| [axis.x, axis.y, axis.z, w];
    [
        row(rotation.x_axis, 0.0),
        row(rotation.y_axis, 0.0),
        row(rotation.z_axis, 0.0),
        row(moved, 1.0),
    ]
}

#[cfg(feature = "window")]
impl ProceduralArena {
    /// Upload every mesh into `presenter`: statics as plain meshes, fighters as skinned meshes
    /// in their own skin block. Cold path, once per presenter.
    pub fn stage(
        &self,
        presenter: &mut crate::present::Presenter,
    ) -> Result<(), crate::present::PresentError> {
        for part in &self.parts {
            match &part.skin {
                Some(skin) => {
                    presenter.upload_skinned_mesh(part.key, &part.mesh, skin, part.block)?;
                }
                None => presenter.upload_mesh(part.key, &part.mesh)?,
            }
        }
        Ok(())
    }

    /// Write every fighter's joint matrices for time `t_s`: eight small buffer writes.
    pub fn pose(
        &self,
        presenter: &mut crate::present::Presenter,
        t_s: f32,
    ) -> Result<(), crate::present::PresentError> {
        for slot in 0..self.spots.len() {
            if let Some(matrices) = self.joint_matrices(slot, t_s) {
                presenter.write_skin_block(slot, &matrices)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arena() -> ProceduralArena {
        ProceduralArena::build().expect("the stand-in builds")
    }

    /// `v_row * M` for a point, the way the skin shader applies a joint matrix.
    fn apply(m: &JointMatrix, v: [f32; 3]) -> [f32; 3] {
        let mut out = [0.0; 3];
        for (j, o) in out.iter_mut().enumerate() {
            *o = v[0] * m[0][j] + v[1] * m[1][j] + v[2] * m[2][j] + m[3][j];
        }
        out
    }

    #[test]
    fn five_static_parts_then_one_skinned_part_per_fighter() {
        let arena = arena();
        let parts = arena.parts();
        assert_eq!(parts.len(), 5 + FIGHTERS);
        assert!(
            parts[..5].iter().all(|p| p.skin.is_none()),
            "scenery is static"
        );
        for (slot, part) in parts[5..].iter().enumerate() {
            let skin = part.skin.as_ref().expect("a fighter is skinned");
            assert_eq!(part.block, slot, "fighter {slot} owns skin block {slot}");
            assert_eq!(skin.joints.len(), part.mesh.positions.len());
            assert_eq!(skin.weights.len(), part.mesh.positions.len());
            assert!(
                skin.joints.iter().all(|row| usize::from(row[0]) < JOINTS),
                "every vertex names a real joint"
            );
            assert!(
                skin.weights
                    .iter()
                    .all(|row| (row.iter().sum::<f32>() - 1.0).abs() < 1e-6),
                "weights sum to one"
            );
        }
        let mut keys: Vec<MeshKey> = parts.iter().map(|p| p.key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), parts.len(), "every part has its own key");
    }

    #[test]
    fn every_triangle_faces_the_way_its_vertex_normals_point() {
        for (index, part) in arena().parts().iter().enumerate() {
            let m = &part.mesh;
            for tri in m.indices.chunks_exact(3) {
                let [a, b, c] =
                    [tri[0], tri[1], tri[2]].map(|i| Vec3::from_array(m.positions[i as usize]));
                let n = Vec3::from_array(m.normals[tri[0] as usize]);
                let face = (b - a).cross(c - a);
                assert!(
                    face.dot(n) > 0.0,
                    "part {index}: a triangle winds against its normal (face {face:?}, normal {n:?})"
                );
            }
        }
    }

    #[test]
    fn fighters_stand_on_the_floor_inside_the_wall_and_face_the_centre() {
        let arena = arena();
        for (slot, part) in arena.parts()[5..].iter().enumerate() {
            let lowest = part
                .mesh
                .positions
                .iter()
                .map(|p| p[1])
                .fold(f32::INFINITY, f32::min);
            let highest = part
                .mesh
                .positions
                .iter()
                .map(|p| p[1])
                .fold(f32::NEG_INFINITY, f32::max);
            assert!(
                lowest.abs() < 1e-5,
                "fighter {slot} feet touch y=0, got {lowest}"
            );
            assert!(
                (1.7..1.8).contains(&highest),
                "fighter {slot} stands {highest} m tall"
            );
            let farthest = part
                .mesh
                .positions
                .iter()
                .map(|p| (p[0] * p[0] + p[2] * p[2]).sqrt())
                .fold(0.0, f32::max);
            assert!(
                farthest < WALL_RADIUS_M - 2.0,
                "fighter {slot} is inside the wall"
            );
            // The sword reaches toward the centre side: the mean point sits farther out than the
            // feet circle's inner side, and the facing yaw puts local +Z at the centre.
            let spot = arena.spots[slot];
            let to_centre = Vec3::new(-spot.feet[0], 0.0, -spot.feet[2]).normalize();
            let forward = Vec3::new(spot.yaw.sin(), 0.0, spot.yaw.cos());
            assert!(
                forward.dot(to_centre) > 0.999,
                "fighter {slot} faces the centre"
            );
        }
    }

    #[test]
    fn the_wall_leaves_a_gate_and_the_floor_is_covered() {
        let arena = arena();
        let wall = &arena.parts()[2].mesh;
        // The two skipped segments sit at angles 0 and one step: no wall vertex is near +X.
        let near_gate = wall
            .positions
            .iter()
            .filter(|p| p[0] > WALL_RADIUS_M - 1.5 && (-0.5..2.2).contains(&p[2]))
            .count();
        assert_eq!(near_gate, 0, "the gate is open");
        let floor = &arena.parts()[0].mesh;
        let widest = floor
            .positions
            .iter()
            .map(|p| p[0].abs())
            .fold(0.0, f32::max);
        assert!(
            widest >= WALL_RADIUS_M + 2.9,
            "the floor runs under the wall: {widest}"
        );
    }

    #[test]
    fn scenery_is_every_static_part_and_never_a_fighter() {
        let arena = arena();
        let all = arena.list_at(1280, 720, 2.5).expect("list builds");
        let scenery = arena.list_scenery(1280, 720, 2.5).expect("scenery builds");
        let fighters = arena
            .parts()
            .iter()
            .filter(|part| part.skin.is_some())
            .count();
        assert_eq!(
            scenery.items.len() + fighters,
            all.items.len(),
            "the scenery list drops exactly the skinned fighter parts"
        );
        assert!(
            scenery.items.len() >= 5,
            "the ring, floor and props stay in the scenery list"
        );
        scenery.validate().expect("the scenery list is valid");
    }

    #[test]
    fn the_framing_camera_keeps_both_fighters_in_view_and_falls_back() {
        let arena = arena();
        let camera = arena
            .framing_camera(1280, 720, [-3.0, 0.0, 0.0], [3.0, 0.0, 1.0])
            .expect("camera builds");
        camera.validate().expect("the framing camera is valid");
        let vp = camera.view_proj().expect("its matrix builds");
        for point in [[-3.0f32, 0.0, 0.0], [3.0, 0.0, 1.0]] {
            let clip = vp * glam::Vec4::new(point[0], 1.0, point[2], 1.0);
            let ndc = clip.truncate() / clip.w;
            assert!(
                ndc.x.abs() < 1.0 && ndc.y.abs() < 1.0,
                "a fighter left the frame: {ndc:?}"
            );
        }
        let ring = arena
            .camera(1280, 720, 0.0)
            .expect("the ring camera builds");
        let same = arena
            .framing_camera(1280, 720, [1.0, 0.0, 1.0], [1.0, 0.0, 1.0])
            .expect("a degenerate pair still gives a camera");
        assert_eq!(same.eye, ring.eye, "it falls back to the ring camera");
        let bad = arena
            .framing_camera(1280, 720, [f32::NAN, 0.0, 0.0], [0.0, 0.0, 0.0])
            .expect("a non-finite position still gives a camera");
        assert_eq!(bad.eye, ring.eye);
        // A pair fighting at the wall keeps the eye inside it, not behind the stone.
        let near = arena
            .framing_camera(1280, 720, [10.6, 0.0, 2.0], [10.0, 0.0, 2.4])
            .expect("a pair at the wall still gives a camera");
        let radius = (near.eye.x * near.eye.x + near.eye.z * near.eye.z).sqrt();
        assert!(
            radius <= WALL_RADIUS_M - 1.5 + 1e-3,
            "the eye stays inside the wall: radius {radius:.3} m"
        );
    }

    #[test]
    fn the_draw_list_has_one_valid_draw_per_part_and_a_valid_camera() {
        let arena = arena();
        for (w, h) in [(1280, 720), (640, 360), (333, 777)] {
            let list = arena.list_at(w, h, 2.5).expect("list builds");
            assert_eq!(list.items.len(), arena.parts().len());
            list.validate()
                .expect("the list passes the presenter's own check");
            assert!((list.camera.aspect - w as f32 / h as f32).abs() < 1e-6);
        }
        assert!(
            arena.list_at(0, 720, 0.0).is_err(),
            "a zero width is refused"
        );
        assert!(
            arena.list_at(1280, 0, 0.0).is_err(),
            "a zero height is refused"
        );
        let a = arena.camera(1280, 720, 0.0).expect("camera at 0");
        let b = arena.camera(1280, 720, 5.0).expect("camera at 5");
        assert_ne!(a.eye, b.eye, "the camera orbits with time");
        assert!((a.eye.y - b.eye.y).abs() < 1e-6, "at a fixed height");
    }

    #[test]
    fn the_torso_joint_moves_the_head_and_never_the_hips_or_feet() {
        let arena = arena();
        let spot = arena.spots[3];
        let hip = [spot.feet[0], HIP_Y_M, spot.feet[2]];
        let head = [spot.feet[0], 1.62, spot.feet[2]];
        let mut moved_head = 0.0f32;
        for step in 0..40 {
            let t = step as f32 * 0.25;
            let [legs, torso] = arena.joint_matrices(3, t).expect("fighter 3 exists");
            assert_eq!(legs, IDENTITY, "the legs stay planted");
            let pivot = apply(&torso, hip);
            for axis in 0..3 {
                assert!(
                    (pivot[axis] - hip[axis]).abs() < 1e-5,
                    "the hips are the pivot"
                );
            }
            let h = apply(&torso, head);
            moved_head = moved_head.max(
                ((h[0] - head[0]).powi(2) + (h[1] - head[1]).powi(2) + (h[2] - head[2]).powi(2))
                    .sqrt(),
            );
            // A rotation keeps lengths: the head stays the same distance from the hips.
            let before = ((head[0] - hip[0]).powi(2)
                + (head[1] - hip[1]).powi(2)
                + (head[2] - hip[2]).powi(2))
            .sqrt();
            let after =
                ((h[0] - hip[0]).powi(2) + (h[1] - hip[1]).powi(2) + (h[2] - hip[2]).powi(2))
                    .sqrt();
            assert!(
                (before - after).abs() < 1e-4,
                "the torso turns, it does not stretch"
            );
        }
        assert!(
            moved_head > 0.03,
            "the head sways by centimetres: {moved_head}"
        );
        assert!(
            arena.joint_matrices(FIGHTERS, 0.0).is_none(),
            "no ninth fighter"
        );
        let a = arena.joint_matrices(0, 1.0).expect("fighter 0");
        let b = arena.joint_matrices(1, 1.0).expect("fighter 1");
        assert_ne!(a[1], b[1], "each fighter sways on its own phase");
    }
}

/// The native twin: the same list through the headless presenter, on the live frame path.
#[cfg(all(test, feature = "window"))]
mod gpu_tests {
    use super::*;
    use crate::pass_truth::LookToggles;
    use crate::present::{PresentError, Presenter};

    fn presenter(width: u32, height: u32) -> Option<Presenter> {
        match Presenter::init_offscreen() {
            Ok(mut presenter) => {
                presenter.resize(width, height).expect("valid size applies");
                Some(presenter)
            }
            Err(PresentError::NoAdapter) => {
                eprintln!("SKIPPED procedural arena: no wgpu adapter on this machine");
                None
            }
            Err(err) => panic!("presenter init failed (must fail loudly): {err}"),
        }
    }

    #[test]
    fn the_live_frame_of_the_stand_in_draws_a_lit_arena_with_no_validation_error() {
        let Some(mut presenter) = presenter(640, 360) else {
            return;
        };
        let arena = ProceduralArena::build().expect("builds");
        arena.stage(&mut presenter).expect("stages");
        for part in arena.parts().iter().skip(5) {
            assert_eq!(presenter.skin_block_of(&part.key), Some(part.block));
        }
        arena.pose(&mut presenter, 0.0).expect("poses");
        let list = arena.list_at(640, 360, 0.0).expect("list");
        let frame = presenter
            .capture_live_pixels(&list, &LookToggles::all_on())
            .expect("the live frame renders and reads back");
        assert!(frame.receipt.is_clean(), "{:?}", frame.receipt);
        assert_eq!(
            usize::try_from(frame.receipt.draws).expect("draw count fits"),
            arena.parts().len()
        );
        let capture = crate::present::FrameCapture {
            width: frame.width,
            height: frame.height,
            rgba: frame.rgba.clone(),
        };
        let stats = capture.stats();
        eprintln!(
            "procedural arena native twin: hash {:016x} {stats:?}",
            capture.hash()
        );
        assert!(!stats.is_blank(), "{stats:?}");
        assert!(
            stats.distinct > 400,
            "sky, floor, wall and fighters give many colours: {stats:?}"
        );
        assert!(stats.dominant_share < 0.4, "{stats:?}");
        // Every part draws pixels: the id target holds the draw index of each one.
        for index in 0..arena.parts().len() {
            let id = crate::draw_id::id_of_draw(index);
            assert!(
                frame.ids.contains(&id),
                "draw {index} leaves no pixel in the id target"
            );
        }
        // The same time draws the same bytes, a later time moves the fighters and the camera.
        let again = presenter
            .capture_live_pixels(&list, &LookToggles::all_on())
            .expect("second frame");
        assert_eq!(again.rgba, frame.rgba, "the same list draws the same frame");
        arena.pose(&mut presenter, 2.0).expect("poses later");
        let later = presenter
            .capture_live_pixels(
                &arena.list_at(640, 360, 2.0).expect("list"),
                &LookToggles::all_on(),
            )
            .expect("later frame");
        assert_ne!(later.rgba, frame.rgba, "time moves the picture");
    }
}
