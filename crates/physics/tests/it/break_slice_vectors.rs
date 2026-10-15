//! break_slice_vectors: consecutive plane-slice tool shape with exact tri rule.
//! Owner: engine-physics (std-only vectors). Units: meters, plane dist in meters.
//! Determinism: fixed order, plus-minus-mul-div only, Fnv64 digest; no wall clock.
//! Errors: none (pure geometry); degenerate inputs copy, never panic.
// PORT(the-dunk/Godot-Destruction@main:plane-slice plus threshold idea, fresh Rust)
// Donor: https://github.com/the-dunk/Godot-Destruction, SPDX MIT.
// Idea taken: slice a mesh with a plane, ignore slivers under a threshold.
// Fresh implementation below: no donor code copied.

use engine_core::Fnv64;

/// Distance under which a vertex counts as on the plane (donor threshold idea).
const EPS: f32 = 0.000_001;

/// Triangle mesh as plain vectors in meters.
struct Mesh {
    verts: Vec<[f32; 3]>,
    tris: Vec<[u32; 3]>,
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn plane_dist(p: [f32; 3], normal: [f32; 3], offset: f32) -> f32 {
    dot(p, normal) - offset
}

fn lerp_point(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// Deterministic content digest over verts then tris.
fn digest(mesh: &Mesh) -> u64 {
    let mut h = Fnv64::new();
    let n_verts = u32::try_from(mesh.verts.len()).expect("mesh fits u32");
    let n_tris = u32::try_from(mesh.tris.len()).expect("mesh fits u32");
    h.write_u32(n_verts);
    h.write_u32(n_tris);
    for v in &mesh.verts {
        h.write_f32(v[0]);
        h.write_f32(v[1]);
        h.write_f32(v[2]);
    }
    for t in &mesh.tris {
        h.write_u32(t[0]);
        h.write_u32(t[1]);
        h.write_u32(t[2]);
    }
    h.finish()
}

/// Split one mesh by a plane into below, above and cut count.
/// A crossing tri yields exactly 3 output tris (1 plus 2), so
/// below plus above equals input plus 2 per cut when no vertex
/// lies within eps of the plane. Touching tris copy to one side.
fn slice_one(mesh: &Mesh, normal: [f32; 3], offset: f32, eps: f32) -> (Mesh, Mesh, u32) {
    let mut below = Mesh {
        verts: mesh.verts.clone(),
        tris: Vec::new(),
    };
    let mut above = Mesh {
        verts: mesh.verts.clone(),
        tris: Vec::new(),
    };
    let mut cut: u32 = 0;
    for tri in &mesh.tris {
        let ia = tri[0];
        let ib = tri[1];
        let ic = tri[2];
        let pa = mesh.verts[usize::try_from(ia).expect("index fits usize")];
        let pb = mesh.verts[usize::try_from(ib).expect("index fits usize")];
        let pc = mesh.verts[usize::try_from(ic).expect("index fits usize")];
        let da = plane_dist(pa, normal, offset);
        let db = plane_dist(pb, normal, offset);
        let dc = plane_dist(pc, normal, offset);
        let pos = [da, db, dc].iter().filter(|d| **d > eps).count();
        let neg = [da, db, dc].iter().filter(|d| **d < -eps).count();
        if pos > 0 && neg > 0 && pos + neg == 3 {
            cut += 1;
            let (single_idx, single_p, single_d, double_a, double_b) = if pos == 1 {
                let s = if da > eps {
                    (ia, pa, da)
                } else if db > eps {
                    (ib, pb, db)
                } else {
                    (ic, pc, dc)
                };
                let others: Vec<(u32, [f32; 3], f32)> = [(ia, pa, da), (ib, pb, db), (ic, pc, dc)]
                    .into_iter()
                    .filter(|(_, _, d)| *d < -eps)
                    .collect();
                (s.0, s.1, s.2, others[0].clone(), others[1].clone())
            } else {
                let s = if da < -eps {
                    (ia, pa, da)
                } else if db < -eps {
                    (ib, pb, db)
                } else {
                    (ic, pc, dc)
                };
                let others: Vec<(u32, [f32; 3], f32)> = [(ia, pa, da), (ib, pb, db), (ic, pc, dc)]
                    .into_iter()
                    .filter(|(_, _, d)| *d > eps)
                    .collect();
                (s.0, s.1, s.2, others[0].clone(), others[1].clone())
            };
            let t0 = single_d / (single_d - double_a.2);
            let t1 = single_d / (single_d - double_b.2);
            let q0 = lerp_point(single_p, double_a.1, t0);
            let q1 = lerp_point(single_p, double_b.1, t1);
            let q0_idx = u32::try_from(below.verts.len()).expect("mesh fits u32");
            below.verts.push(q0);
            above.verts.push(q0);
            let q1_idx = u32::try_from(below.verts.len()).expect("mesh fits u32");
            below.verts.push(q1);
            above.verts.push(q1);
            if pos == 1 {
                above.tris.push([single_idx, q0_idx, q1_idx]);
                below.tris.push([double_a.0, double_b.0, q1_idx]);
                below.tris.push([double_a.0, q1_idx, q0_idx]);
            } else {
                below.tris.push([single_idx, q0_idx, q1_idx]);
                above.tris.push([double_a.0, double_b.0, q1_idx]);
                above.tris.push([double_a.0, q1_idx, q0_idx]);
            }
        } else if neg == 0 {
            above.tris.push(*tri);
        } else if pos == 0 {
            below.tris.push(*tri);
        } else {
            above.tris.push(*tri);
        }
    }
    (below, above, cut)
}

/// Consecutive slice with a width offset: first at d0, then the upper
/// fragment again at d0 plus width. Returns below, slab, above, cut0, cut1.
fn slice_consecutive(
    mesh: &Mesh,
    normal: [f32; 3],
    d0: f32,
    width: f32,
    eps: f32,
) -> (Mesh, Mesh, Mesh, u32, u32) {
    let (below, rest, cut0) = slice_one(mesh, normal, d0, eps);
    let (slab, above, cut1) = slice_one(&rest, normal, d0 + width, eps);
    (below, slab, above, cut0, cut1)
}

/// Unit cube centered at the origin, 8 verts and 12 tris.
fn cube_mesh() -> Mesh {
    let h: f32 = 0.5;
    let verts = vec![
        [-h, -h, -h],
        [h, -h, -h],
        [h, h, -h],
        [-h, h, -h],
        [-h, -h, h],
        [h, -h, h],
        [h, h, h],
        [-h, h, h],
    ];
    let tris = vec![
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 4, 7],
        [0, 7, 3],
        [1, 2, 6],
        [1, 6, 5],
        [0, 1, 5],
        [0, 5, 4],
        [3, 7, 6],
        [3, 6, 2],
    ];
    Mesh { verts, tris }
}

/// Min and max x over verts actually used by tris.
/// Unused cloned verts stay in the buffer, so a whole-buffer scan lies.
fn min_max_x_used(mesh: &Mesh) -> (f32, f32) {
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for tri in &mesh.tris {
        for idx in tri {
            let v = mesh.verts[usize::try_from(*idx).expect("index fits usize")];
            if v[0] < lo {
                lo = v[0];
            }
            if v[0] > hi {
                hi = v[0];
            }
        }
    }
    (lo, hi)
}

#[test]
fn single_plane_tri_rule_exact() {
    let mesh = cube_mesh();
    let before = mesh.tris.len();
    let (below, above, cut) = slice_one(&mesh, [1.0, 0.0, 0.0], 0.0, EPS);
    assert!(cut > 0, "plane x=0 must cut the cube");
    assert!(!below.tris.is_empty() && !above.tris.is_empty());
    assert_eq!(
        below.tris.len() + above.tris.len(),
        before + 2 * usize::try_from(cut).expect("cut fits usize"),
        "Tm1+Tm2 must equal TM+2*Ncf"
    );
    assert_ne!(
        below.tris.len() + above.tris.len(),
        before + usize::try_from(cut).expect("cut fits usize"),
        "quad side must emit 2 tris, not 1"
    );
    let (_, below_hi) = min_max_x_used(&below);
    let (above_lo, _) = min_max_x_used(&above);
    assert!(below_hi <= EPS, "below cap sits on the plane");
    assert!(above_lo >= -EPS, "above cap sits on the plane");
}

#[test]
fn consecutive_width_offset_rule_exact() {
    let mesh = cube_mesh();
    let before = mesh.tris.len();
    let width: f32 = 0.5;
    let (below, slab, above, cut0, cut1) =
        slice_consecutive(&mesh, [1.0, 0.0, 0.0], -0.25, width, EPS);
    assert!(cut0 > 0 && cut1 > 0, "both offset planes must cut");
    assert!(!below.tris.is_empty() && !slab.tris.is_empty() && !above.tris.is_empty());
    let (below0, rest0, cut0a) = slice_one(&mesh, [1.0, 0.0, 0.0], -0.25, EPS);
    assert_eq!(cut0a, cut0, "consecutive first cut matches solo cut");
    assert_eq!(
        below.tris.len(),
        below0.tris.len(),
        "below matches solo first cut"
    );
    assert_eq!(
        below0.tris.len() + rest0.tris.len(),
        before + 2 * usize::try_from(cut0).expect("cut fits usize"),
        "first plane keeps the exact rule"
    );
    assert_eq!(
        slab.tris.len() + above.tris.len(),
        rest0.tris.len() + 2 * usize::try_from(cut1).expect("cut fits usize"),
        "second offset plane keeps the exact rule"
    );
    assert_eq!(
        below.tris.len() + slab.tris.len() + above.tris.len(),
        before
            + 2 * usize::try_from(cut0).expect("cut fits usize")
            + 2 * usize::try_from(cut1).expect("cut fits usize"),
        "chained total must equal TM+2*N0+2*N1"
    );
    let (_, below_hi) = min_max_x_used(&below);
    let (slab_lo, slab_hi) = min_max_x_used(&slab);
    let (above_lo, _) = min_max_x_used(&above);
    assert!(below_hi <= -0.25 + EPS, "below stays under the first plane");
    assert!(
        slab_lo >= -0.25 - EPS && slab_hi <= 0.25 + EPS,
        "slab fills the width"
    );
    assert!(above_lo >= 0.25 - EPS, "above starts at the offset plane");
}

#[test]
fn digest_twice_stable_and_distinct() {
    let mesh = cube_mesh();
    let (below, above, _) = slice_one(&mesh, [1.0, 0.0, 0.0], 0.0, EPS);
    let d_below_a = digest(&below);
    let d_below_b = digest(&below);
    let d_above_a = digest(&above);
    let d_above_b = digest(&above);
    assert_eq!(d_below_a, d_below_b, "digest x2 must match for below");
    assert_eq!(d_above_a, d_above_b, "digest x2 must match for above");
    assert_ne!(d_below_a, d_above_a, "opposite fragments must digest apart");
    let (below2, above2, _) = slice_one(&mesh, [1.0, 0.0, 0.0], 0.0, EPS);
    assert_eq!(d_below_a, digest(&below2));
    assert_eq!(d_above_a, digest(&above2));
}

#[test]
fn threshold_suppresses_sliver_cut() {
    let mesh = Mesh {
        verts: vec![
            [0.000_000_5, 0.0, 0.0],
            [-0.000_000_5, 1.0, 0.0],
            [1.0, 0.0, 1.0],
        ],
        tris: vec![[0, 1, 2]],
    };
    let (below, above, cut) = slice_one(&mesh, [1.0, 0.0, 0.0], 0.0, EPS);
    assert_eq!(cut, 0, "sliver inside EPS must not count as a cut");
    assert_eq!(
        below.tris.len() + above.tris.len(),
        1,
        "suppressed cut keeps the tri count"
    );
    let (_, _, cut0) = slice_one(&mesh, [1.0, 0.0, 0.0], 0.0, 0.0);
    assert_eq!(cut0, 1, "zero threshold sees the straddle");
    let (below0, above0, _) = slice_one(&mesh, [1.0, 0.0, 0.0], 0.0, 0.0);
    assert_eq!(below0.tris.len() + above0.tris.len(), 3);
}
