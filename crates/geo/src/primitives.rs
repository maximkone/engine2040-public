//! Closed test solids, built the same way every call. Used by the seeded tests of every tool and
//! by callers that need a quick blank (a cutter, a stand-in collision shape).

use crate::mesh::{frac, narrow, GeoMesh};

fn idx(i: usize) -> u32 {
    u32::try_from(i).unwrap_or(u32::MAX)
}

/// Axis-aligned box centered on the origin: 8 shared vertices, 12 triangles.
#[must_use]
pub fn cuboid(size: [f32; 3]) -> GeoMesh {
    let h = size.map(|v| v * 0.5);
    let positions = vec![
        [-h[0], -h[1], -h[2]],
        [h[0], -h[1], -h[2]],
        [h[0], h[1], -h[2]],
        [-h[0], h[1], -h[2]],
        [-h[0], -h[1], h[2]],
        [h[0], -h[1], h[2]],
        [h[0], h[1], h[2]],
        [-h[0], h[1], h[2]],
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, // -z
        4, 5, 6, 4, 6, 7, // +z
        0, 4, 7, 0, 7, 3, // -x
        1, 2, 6, 1, 6, 5, // +x
        0, 1, 5, 0, 5, 4, // -y
        3, 7, 6, 3, 6, 2, // +y
    ];
    GeoMesh::from_positions(positions, indices)
}

/// Box with one separate quad per face (24 vertices, normals and a 3x2 UV layout): every face
/// edge is a UV seam. The shape a game kit crate has.
#[must_use]
pub fn cuboid_faces(size: [f32; 3]) -> GeoMesh {
    let h = size.map(|v| v * 0.5);
    // Per face: normal, u axis, v axis, in outward counter-clockwise order.
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ];
    let mut mesh = GeoMesh::default();
    for (f, (n, u, v)) in faces.iter().enumerate() {
        let base = idx(mesh.positions.len());
        // Cell of a 3x2 layout, inset by 1/64 so charts never touch.
        let (cx, cy) = (f % 3, f / 3);
        let (u0, v0) = (
            narrow(frac(cx, 3)) + 1.0 / 64.0,
            narrow(frac(cy, 2)) + 1.0 / 64.0,
        );
        let (du, dv) = (1.0 / 3.0 - 1.0 / 32.0, 0.5 - 1.0 / 32.0);
        for (su, sv) in [(-1.0_f32, -1.0_f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = [
                n[0] * h[0] + u[0] * su * h[0] + v[0] * sv * h[0],
                n[1] * h[1] + u[1] * su * h[1] + v[1] * sv * h[1],
                n[2] * h[2] + u[2] * su * h[2] + v[2] * sv * h[2],
            ];
            mesh.positions.push(p);
            mesh.normals.push(*n);
            mesh.uvs
                .push([u0 + du * (su * 0.5 + 0.5), v0 + dv * (sv * 0.5 + 0.5)]);
        }
        mesh.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    mesh
}

/// UV sphere: `rings` bands pole to pole, `segments` around, one shared vertex per pole and a
/// seam column (first and last column share positions but not UVs).
#[must_use]
pub fn uv_sphere(radius: f32, rings: usize, segments: usize) -> GeoMesh {
    let rings = rings.max(2);
    let segments = segments.max(3);
    let mut mesh = GeoMesh::default();
    // Pole vertices, then (rings - 1) full rings of segments + 1 columns.
    mesh.positions.push([0.0, radius, 0.0]);
    mesh.normals.push([0.0, 1.0, 0.0]);
    mesh.uvs.push([0.5, 0.0]);
    for r in 1..rings {
        let phi = std::f64::consts::PI * frac(r, rings);
        for s in 0..=segments {
            // The seam column repeats column 0 exactly (same bits), only its UV differs.
            let theta = std::f64::consts::TAU * frac(s % segments, segments);
            let n = [
                narrow(phi.sin() * theta.cos()),
                narrow(phi.cos()),
                narrow(phi.sin() * theta.sin()),
            ];
            mesh.positions.push(n.map(|v| v * radius));
            mesh.normals.push(n);
            mesh.uvs
                .push([narrow(frac(s, segments)), narrow(frac(r, rings))]);
        }
    }
    mesh.positions.push([0.0, -radius, 0.0]);
    mesh.normals.push([0.0, -1.0, 0.0]);
    mesh.uvs.push([0.5, 1.0]);
    let ring_start = |r: usize| 1 + (r - 1) * (segments + 1);
    let south = idx(mesh.positions.len() - 1);
    for s in 0..segments {
        mesh.indices
            .extend_from_slice(&[0, idx(ring_start(1) + s + 1), idx(ring_start(1) + s)]);
    }
    for r in 1..rings - 1 {
        for s in 0..segments {
            let (a, b) = (idx(ring_start(r) + s), idx(ring_start(r) + s + 1));
            let (c, d) = (idx(ring_start(r + 1) + s), idx(ring_start(r + 1) + s + 1));
            mesh.indices.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }
    for s in 0..segments {
        let last = rings - 1;
        mesh.indices.extend_from_slice(&[
            south,
            idx(ring_start(last) + s),
            idx(ring_start(last) + s + 1),
        ]);
    }
    mesh
}

/// Capped cylinder along Y: `segments` sides, shared rim vertices, one center vertex per cap.
#[must_use]
pub fn cylinder(radius: f32, height: f32, segments: usize) -> GeoMesh {
    let segments = segments.max(3);
    let h = height * 0.5;
    let mut positions = Vec::new();
    for s in 0..segments {
        let theta = std::f64::consts::TAU * frac(s, segments);
        positions.push([
            narrow(theta.cos()) * radius,
            -h,
            narrow(theta.sin()) * radius,
        ]);
    }
    for s in 0..segments {
        let p = positions[s];
        positions.push([p[0], h, p[2]]);
    }
    positions.push([0.0, -h, 0.0]);
    positions.push([0.0, h, 0.0]);
    let (bottom, top) = (idx(2 * segments), idx(2 * segments + 1));
    let mut indices = Vec::new();
    for s in 0..segments {
        let n = (s + 1) % segments;
        let (a, b) = (idx(s), idx(n));
        let (c, d) = (idx(segments + s), idx(segments + n));
        indices.extend_from_slice(&[a, d, b, a, c, d]);
        indices.extend_from_slice(&[bottom, a, b]);
        indices.extend_from_slice(&[top, d, c]);
    }
    GeoMesh::from_positions(positions, indices)
}

/// Torus in the XZ plane: `major` ring radius, `minor` tube radius.
#[must_use]
pub fn torus(major: f32, minor: f32, major_segments: usize, minor_segments: usize) -> GeoMesh {
    let (ms, ns) = (major_segments.max(3), minor_segments.max(3));
    let mut positions = Vec::new();
    for i in 0..ms {
        let a = std::f64::consts::TAU * frac(i, ms);
        for j in 0..ns {
            let b = std::f64::consts::TAU * frac(j, ns);
            let ring = f64::from(major) + f64::from(minor) * b.cos();
            positions.push([
                narrow(ring * a.cos()),
                narrow(f64::from(minor) * b.sin()),
                narrow(ring * a.sin()),
            ]);
        }
    }
    let mut indices = Vec::new();
    for i in 0..ms {
        for j in 0..ns {
            let (i1, j1) = ((i + 1) % ms, (j + 1) % ns);
            let (a, b) = (idx(i * ns + j), idx(i1 * ns + j));
            let (c, d) = (idx(i * ns + j1), idx(i1 * ns + j1));
            indices.extend_from_slice(&[a, c, d, a, d, b]);
        }
    }
    GeoMesh::from_positions(positions, indices)
}

/// L-shaped extrusion (a concave prop): an `arm` by `arm` square missing from a `size` square,
/// `height` tall. Closed, volume `(size * size - arm * arm) * height`.
#[must_use]
pub fn l_prism(size: f32, arm: f32, height: f32) -> GeoMesh {
    let outline = [
        [0.0, 0.0],
        [size, 0.0],
        [size, arm],
        [arm, arm],
        [arm, size],
        [0.0, size],
    ];
    let n = outline.len();
    let mut positions = Vec::new();
    for p in outline {
        positions.push([p[0], 0.0, p[1]]);
    }
    for p in outline {
        positions.push([p[0], height, p[1]]);
    }
    let mut indices = Vec::new();
    // The L is the union of two convex quads: split the floor and ceiling by hand.
    // The outline runs clockwise seen from +Y (x right, z toward the viewer), so the fan order
    // points down (-Y): use it for the floor and reverse it for the ceiling.
    let fan = [[0, 1, 2], [0, 2, 3], [0, 3, 4], [0, 4, 5]];
    let (lo, hi) = (0usize, n);
    for f in fan {
        indices.extend_from_slice(&[idx(lo + f[0]), idx(lo + f[1]), idx(lo + f[2])]);
        indices.extend_from_slice(&[idx(hi + f[0]), idx(hi + f[2]), idx(hi + f[1])]);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        let (a, b, c, d) = (idx(lo + i), idx(lo + j), idx(hi + i), idx(hi + j));
        indices.extend_from_slice(&[a, c, d, a, d, b]);
    }
    GeoMesh::from_positions(positions, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closed(mesh: &GeoMesh, what: &str) {
        mesh.validate().unwrap();
        let report = mesh.edge_report();
        assert_eq!(report.open_edges, 0, "{what}: open edges");
        assert!(report.edges > 0);
    }

    #[test]
    fn every_primitive_is_closed_and_outward() {
        let cube = cuboid([2.0, 2.0, 2.0]);
        closed(&cube, "cube");
        assert!((cube.volume() - 8.0).abs() < 1e-9);
        let faces = cuboid_faces([2.0, 2.0, 2.0]);
        closed(&faces, "faces");
        assert!((faces.volume() - 8.0).abs() < 1e-9);
        assert_eq!(faces.positions.len(), 24);
        let sphere = uv_sphere(1.0, 12, 16);
        closed(&sphere, "sphere");
        assert!(sphere.volume() > 3.9 && sphere.volume() < 4.19);
        let cyl = cylinder(1.0, 2.0, 24);
        closed(&cyl, "cyl");
        assert!(cyl.volume() > 6.0 && cyl.volume() < std::f64::consts::PI * 2.0 + 1e-3);
        let torus = torus(2.0, 0.5, 24, 12);
        closed(&torus, "torus");
        assert!(torus.volume() > 0.0);
        let l = l_prism(2.0, 1.0, 1.0);
        closed(&l, "l");
        assert!((l.volume() - 3.0).abs() < 1e-9);
    }

    #[test]
    fn open_mesh_is_reported_open() {
        let mut cube = cuboid([1.0, 1.0, 1.0]);
        cube.indices.truncate(33);
        assert_eq!(cube.edge_report().open_edges, 3);
        assert!(!cube.edge_report().is_closed());
    }

    #[test]
    fn digest_is_stable_and_sensitive() {
        let a = cuboid([1.0, 2.0, 3.0]);
        assert_eq!(a.digest(), cuboid([1.0, 2.0, 3.0]).digest());
        assert_ne!(a.digest(), cuboid([1.0, 2.0, 3.5]).digest());
    }

    #[test]
    fn welded_merges_split_vertices() {
        let faces = cuboid_faces([1.0, 1.0, 1.0]);
        let welded = faces.welded();
        assert_eq!(welded.positions.len(), 8);
        assert_eq!(welded.triangle_count(), 12);
        closed(&welded, "welded");
    }

    #[test]
    fn placed_quarter_turns_keep_volume_and_orientation() {
        let l = l_prism(2.0, 1.0, 1.0);
        for turns in 0..4 {
            let moved = l.placed([1.0, 1.0, 1.0], turns, [5.0, 0.0, -3.0]);
            assert!((moved.volume() - l.volume()).abs() < 1e-6);
            closed(&moved, "moved");
        }
    }
}
