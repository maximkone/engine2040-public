//! draw: the speed demo's mesh builders. Shells become flat-shaded render data whose UVs
//! are laid out in world meters (one facade bay [`BAY_M`] wide, one floor [`FLOOR_M`] tall on a
//! wall, [`BAY_M`] squares on a roof), so a repeating facade tile lines up on every face,
//! a fresh cut face included. Presentation only. Units: meters.

use engine_destruction::csg::PropMesh;
use engine_render::present::{DrawItem, MeshKey};
use engine_render::v1::{upload_mesh_tiled, MeshData, PbrMaterial};

use super::num::{add, cross, dot, idx, rotate, scale, sub, unit_or};

/// One facade bay along a wall, meters.
pub(super) const BAY_M: f32 = 3.0;
/// One floor up a wall, meters.
pub(super) const FLOOR_M: f32 = 3.6;
/// The speed demo's mesh digest (its meshes are built, not cooked).
const DIGEST: [u8; 32] = [0x5D; 32];

/// The UV of a point on a face with normal `n`: walls run along their own horizontal
/// direction in bays and up in floors; flat faces tile in bays on x and z.
pub(super) fn facade_uv(p: [f32; 3], n: [f32; 3]) -> [f32; 2] {
    let uv = if n[1].abs() > 0.7 {
        [p[0] / BAY_M, p[2] / BAY_M]
    } else {
        let along = unit_or([-n[2], 0.0, n[0]], [1.0, 0.0, 0.0]);
        [dot(p, along) / BAY_M, -p[1] / FLOOR_M]
    };
    // The tiles repeat every whole unit, so wrapping by a whole span keeps the pattern and
    // keeps a body thrown kilometres away inside the upload's UV range.
    uv.map(|c| (c + UV_WRAP).rem_euclid(2.0 * UV_WRAP) - UV_WRAP)
}

/// UVs wrap into `-UV_WRAP..UV_WRAP` (whole tiles; under the tiled upload limit of 1024).
const UV_WRAP: f32 = 256.0;

fn unit(a: [f32; 3]) -> [f32; 3] {
    unit_or(a, [0.0, 1.0, 0.0])
}

pub(super) fn key(base: u64, i: usize) -> MeshKey {
    MeshKey::new(base + u64::try_from(i).unwrap_or(0), DIGEST)
}

pub(super) fn item(mesh: MeshKey, material: PbrMaterial) -> DrawItem {
    DrawItem {
        mesh,
        material,
        base_color: None,
        normal_map: None,
        orm: None,
        emissive: None,
    }
}

/// A shell as flat-shaded render data, turned by `angle` about `axis` through `pivot`
/// and moved by `offset`.
pub(super) fn flat_mesh(
    shell: &PropMesh,
    offset: [f32; 3],
    axis: [f32; 3],
    angle: f32,
    pivot: [f32; 3],
) -> Result<MeshData, String> {
    let turn = |p: [f32; 3]| add(add(rodrigues(sub(p, pivot), axis, angle), pivot), offset);
    flat_from(shell, turn)
}

/// A shell placed by a pose (the car).
pub(super) fn flat_mesh_rot(
    shell: &PropMesh,
    pos: [f32; 3],
    rot: [f32; 4],
) -> Result<MeshData, String> {
    flat_from(shell, |p| add(rotate(rot, p), pos))
}

/// Flat-shaded triangles waiting to become meshes: every triangle its own three vertices.
#[derive(Clone, Debug, Default)]
pub(super) struct Soup {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
}

/// Most triangles one mesh carries (three vertices each, under the 65,535-vertex limit).
const MESH_TRIS: usize = 21_000;

impl Soup {
    /// Append `shell`, every point moved by `place`.
    pub(super) fn add(&mut self, shell: &PropMesh, place: impl Fn([f32; 3]) -> [f32; 3]) {
        for t in shell.indices.chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| place(shell.positions[idx(i)]));
            let n = unit(cross(sub(b, a), sub(c, a)));
            self.positions.extend_from_slice(&[a, b, c]);
            self.normals.extend_from_slice(&[n, n, n]);
        }
    }

    /// True when nothing was added.
    pub(super) fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    /// The soup as render meshes, split so each fits the upload limit; facade UVs in meters.
    pub(super) fn meshes(&self) -> Result<Vec<MeshData>, String> {
        let mut out = Vec::new();
        for (ps, ns) in self
            .positions
            .chunks(MESH_TRIS * 3)
            .zip(self.normals.chunks(MESH_TRIS * 3))
        {
            let count = u32::try_from(ps.len()).map_err(|_| "mesh too large".to_string())?;
            let indices: Vec<u32> = (0..count).collect();
            let uvs: Vec<[f32; 2]> = ps.iter().zip(ns).map(|(p, n)| facade_uv(*p, *n)).collect();
            out.push(upload_mesh_tiled(ps, ns, &uvs, &indices).map_err(|e| format!("{e:?}"))?);
        }
        Ok(out)
    }
}

/// One shell as one flat-shaded mesh, every point moved by `place`.
pub(super) fn flat_from(
    shell: &PropMesh,
    place: impl Fn([f32; 3]) -> [f32; 3],
) -> Result<MeshData, String> {
    let mut soup = Soup::default();
    soup.add(shell, place);
    soup.meshes()?
        .into_iter()
        .next()
        .ok_or_else(|| "empty shell".to_string())
}

/// The blade as a thin flat bar from hilt to tip.
pub(super) fn blade_mesh(hilt: [f32; 3], tip: [f32; 3]) -> Result<MeshData, String> {
    let along = unit(sub(tip, hilt));
    let side = unit(cross(along, [0.0, 1.0, 0.0]));
    let up = cross(side, along);
    let w = scale(up, 0.45);
    let t = scale(side, 0.06);
    let corners = |base: [f32; 3], width: f32| {
        [
            add(add(base, scale(w, width)), t),
            add(sub(base, scale(w, width)), t),
            sub(sub(base, scale(w, width)), t),
            sub(add(base, scale(w, width)), t),
        ]
    };
    let [a0, a1, a2, a3] = corners(hilt, 1.0);
    let [b0, b1, b2, b3] = corners(tip, 0.15);
    let positions = vec![a0, a1, a2, a3, b0, b1, b2, b3];
    let indices = vec![
        0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7, 6, 3, 0, 4, 3, 4, 7, 0, 3, 2, 0, 2, 1,
        4, 5, 6, 4, 6, 7,
    ];
    flat_from(&PropMesh { positions, indices }, |p| p)
}

/// A thin square bar from `a` to `b`, `half_w` meters from its axis (the web line).
pub(super) fn bar_mesh(a: [f32; 3], b: [f32; 3], half_w: f32) -> Result<MeshData, String> {
    let along = unit(sub(b, a));
    let side = scale(unit(cross(along, [0.0, 1.0, 0.0])), half_w);
    let up = scale(unit(cross(side, along)), half_w);
    let lo = [a, b];
    let ring = |c: [f32; 3]| [add(c, side), add(c, up), sub(c, side), sub(c, up)];
    let (r0, r1) = (ring(lo[0]), ring(lo[1]));
    let positions = vec![r0[0], r0[1], r0[2], r0[3], r1[0], r1[1], r1[2], r1[3]];
    let indices = vec![
        0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7, 6, 3, 0, 4, 3, 4, 7,
    ];
    flat_from(&PropMesh { positions, indices }, |p| p)
}

pub(super) fn shell_bounds(shell: &PropMesh) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for p in &shell.positions {
        for a in 0..3 {
            min[a] = min[a].min(p[a]);
            max[a] = max[a].max(p[a]);
        }
    }
    (min, max)
}

pub(super) fn rodrigues(v: [f32; 3], axis: [f32; 3], angle: f32) -> [f32; 3] {
    let k = unit(axis);
    let (s, c) = angle.sin_cos();
    add(
        add(scale(v, c), scale(cross(k, v), s)),
        scale(k, dot(k, v) * (1.0 - c)),
    )
}
