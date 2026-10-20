//! Meshlet cull vectors: frustum plus cone, zero false negatives.
//!
//! Owner: `engine-assets`; consumer: the render meshlet path submits only
//! the clusters this filter keeps. Units: meters. Determinism: fixed grids
//! and `f32` arithmetic, so reruns agree. Errors: cook errors surface through
//! `expect` with the invariant named.
//!
//! The cull law under test: a visible meshlet is never culled. Ground truth
//! is a triangle fully inside every frustum plane and turned toward the eye,
//! which is a sufficient proof of visibility on its own.
//!
//! Own-code bounds derived from in-tree `meshlet.rs`; the cone cutoff idea is
//! meshoptimizer (Zeux, MIT). No donor bytes.

use engine_assets::meshlet::{build_meshlets, meshlet_bounds, Meshlet, MeshletBounds};
use engine_assets::meshlet_cull::{
    cone_culled, cull_meshlets, CullFilter, CullPlane, CullVerdict, Frustum,
};

/// Flat grid of `cells` cells per side, centered on `center` at depth `z`.
///
/// Quads split row-major: the face normal is `+Z`, or `-Z` when `flip`.
fn grid(cells: u32, center: [f32; 2], z: f32, flip: bool) -> (Vec<[f32; 3]>, Vec<u32>) {
    let side = cells.saturating_add(1);
    let half = f32::from(u16::try_from(cells).expect("cells fit u16")) * 0.5;
    let mut positions = Vec::with_capacity(usize::try_from(side * side).expect("grid fits"));
    for y in 0..side {
        for x in 0..side {
            let wide_x = f32::from(u16::try_from(x).expect("x fits u16"));
            let wide_y = f32::from(u16::try_from(y).expect("y fits u16"));
            positions.push([wide_x + center[0] - half, wide_y + center[1] - half, z]);
        }
    }
    let at = |x: u32, y: u32| -> u32 { y.saturating_mul(side).saturating_add(x) };
    let mut indices = Vec::with_capacity(usize::try_from(cells * cells * 6).expect("indices fit"));
    for y in 0..cells {
        for x in 0..cells {
            let (a, b, c, d) = (at(x, y), at(x + 1, y), at(x, y + 1), at(x + 1, y + 1));
            indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    if flip {
        for tri in indices.chunks_exact_mut(3) {
            tri.swap(1, 2);
        }
    }
    (positions, indices)
}

/// Orthographic view-projection, row-major, GL clip depth `[-w, w]`.
fn ortho_box(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> [[f32; 4]; 4] {
    [
        [
            2.0 / (right - left),
            0.0,
            0.0,
            -(right + left) / (right - left),
        ],
        [
            0.0,
            2.0 / (top - bottom),
            0.0,
            -(top + bottom) / (top - bottom),
        ],
        [0.0, 0.0, -2.0 / (far - near), -(far + near) / (far - near)],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// True when one triangle is fully inside the view and faces the eye.
///
/// That is a sufficient proof of visibility, so a culled meshlet carrying
/// such a triangle is a false negative.
fn definitely_visible(positions: &[[f32; 3]], meshlet: &Meshlet, filter: &CullFilter) -> bool {
    meshlet.triangles.iter().any(|tri| {
        let mut corners = [[0.0_f32; 3]; 3];
        for (slot, local) in corners.iter_mut().zip(tri.iter()) {
            let Some(vertex) = meshlet
                .vertices
                .get(usize::try_from(*local).ok().unwrap_or(usize::MAX))
                .copied()
            else {
                return false;
            };
            let Some(point) = positions
                .get(usize::try_from(vertex).ok().unwrap_or(usize::MAX))
                .copied()
            else {
                return false;
            };
            *slot = point;
        }
        if !corners
            .iter()
            .all(|point| filter.frustum.contains_sphere(*point, 0.0))
        {
            return false;
        }
        let normal = cross(sub(corners[1], corners[0]), sub(corners[2], corners[0]));
        let centroid = [
            (corners[0][0] + corners[1][0] + corners[2][0]) / 3.0,
            (corners[0][1] + corners[1][1] + corners[2][1]) / 3.0,
            (corners[0][2] + corners[1][2] + corners[2][2]) / 3.0,
        ];
        dot(normal, sub(filter.eye, centroid)) > 0.0
    })
}

/// One bank per cull reason: visible, backfacing, outside left, outside far.
fn fixtures() -> Vec<(Vec<[f32; 3]>, Meshlet)> {
    let mut out = Vec::new();
    for (center, z, flip) in [
        ([0.0, 0.0], -4.0, false),
        ([0.0, 0.0], -4.0, true),
        ([-30.0, 0.0], -4.0, false),
        ([0.0, 0.0], -30.0, false),
    ] {
        let (positions, indices) = grid(16, center, z, flip);
        let meshlets = build_meshlets(&positions, &indices).expect("cook accepts the grid");
        for meshlet in meshlets {
            out.push((positions.clone(), meshlet));
        }
    }
    out
}

/// The test view: box `x,y in [-4, 4]`, depth `z in [-8, -2]`, eye at origin.
fn test_filter() -> CullFilter {
    CullFilter::new(
        Frustum::from_view_projection(&ortho_box(-4.0, 4.0, -4.0, 4.0, 2.0, 8.0)),
        [0.0, 0.0, 0.0],
    )
}

#[test]
fn plane_signs_are_inside_positive_and_normals_unit() {
    let plane = CullPlane::new([0.0, 0.0, 2.0], -6.0);
    let len = dot(plane.normal, plane.normal).sqrt();
    assert!((len - 1.0).abs() <= 1e-6, "plane normal is unit length");
    assert!(
        plane.signed_distance([1.0, 2.0, 4.0]) > 0.0,
        "inside is positive"
    );
    assert!(
        plane.signed_distance([0.0, 0.0, 2.0]) < 0.0,
        "outside is negative"
    );
    assert!(
        plane.signed_distance([0.0, 0.0, 3.0]).abs() <= 1e-6,
        "the plane itself reads zero"
    );
    let flat = CullPlane::new([0.0, 0.0, 0.0], 0.0);
    assert!(
        flat.signed_distance([5.0, 5.0, 5.0]) > 0.0,
        "degenerate plane accepts"
    );
    let nan = CullPlane::new([f32::NAN, 0.0, 0.0], 1.0);
    assert!(
        nan.signed_distance([1.0, 2.0, 3.0]) > 0.0,
        "non-finite plane accepts"
    );
}

#[test]
fn ortho_frustum_keeps_inside_and_rejects_each_side() {
    let frustum = Frustum::from_view_projection(&ortho_box(-2.0, 2.0, -1.5, 1.5, 1.0, 6.0));
    assert!(
        frustum.contains_sphere([0.0, 0.0, -3.0], 0.25),
        "center is inside"
    );
    for center in [
        [3.0, 0.0, -3.0],
        [-3.0, 0.0, -3.0],
        [0.0, 2.5, -3.0],
        [0.0, -2.5, -3.0],
        [0.0, 0.0, -0.5],
        [0.0, 0.0, -8.0],
    ] {
        assert!(
            !frustum.contains_sphere(center, 0.1),
            "center {center:?} lies outside one plane"
        );
    }
    assert!(
        frustum.contains_sphere([-2.1, 0.0, -3.0], 0.25),
        "a sphere straddling a plane is kept"
    );
    let half_space = Frustum::from_planes([CullPlane::new([1.0, 0.0, 0.0], 0.0); 6]);
    assert!(
        half_space.contains_sphere([0.5, 0.0, 0.0], 0.1),
        "positive side kept"
    );
    assert!(
        !half_space.contains_sphere([-0.5, 0.0, 0.0], 0.1),
        "negative side culled"
    );
}

#[test]
fn cone_culls_only_when_the_whole_face_turns_away() {
    let front = MeshletBounds {
        center: [0.0, 0.0, -4.0],
        radius_m: 0.5,
        cone_axis: [0.0, 0.0, 1.0],
        cone_cos: 1.0,
    };
    let back = MeshletBounds {
        cone_axis: [0.0, 0.0, -1.0],
        ..front
    };
    assert!(
        !cone_culled(&front, [0.0, 0.0, 0.0]),
        "a face toward the eye stays"
    );
    assert!(cone_culled(&back, [0.0, 0.0, 0.0]), "a face away is culled");
    let wide = MeshletBounds {
        cone_cos: 0.0,
        radius_m: 1.0,
        ..back
    };
    assert!(
        !cone_culled(&wide, [4.0, 0.0, -4.0]),
        "a 90 degree cone seen from the side stays"
    );
    let near = MeshletBounds {
        center: [0.0, 0.0, -1.0],
        radius_m: 3.0,
        cone_axis: [0.0, 0.0, -1.0],
        cone_cos: 1.0,
    };
    assert!(
        !cone_culled(&near, [0.0, 0.0, 0.0]),
        "a sphere that reaches past the tangent plane stays"
    );
    let broken = MeshletBounds {
        center: [f32::NAN, 0.0, 0.0],
        ..front
    };
    assert!(
        !cone_culled(&broken, [0.0, 0.0, 0.0]),
        "non-finite bounds stay"
    );
}

#[test]
fn mixed_grid_cull_has_zero_false_negatives() {
    let filter = test_filter();
    let fixtures = fixtures();
    let mut culled = 0_usize;
    let mut false_negatives = 0_usize;
    let mut reasons = [0_usize; 3];
    for (positions, meshlet) in &fixtures {
        let bounds = meshlet_bounds(positions, meshlet);
        let verdict = filter.verdict(&bounds);
        if verdict.is_visible() {
            reasons[0] = reasons[0].saturating_add(1);
            continue;
        }
        culled = culled.saturating_add(1);
        match verdict {
            CullVerdict::OutsideFrustum => reasons[1] = reasons[1].saturating_add(1),
            CullVerdict::FacingAway => reasons[2] = reasons[2].saturating_add(1),
            CullVerdict::Visible => {}
        }
        if definitely_visible(positions, meshlet, &filter) {
            false_negatives = false_negatives.saturating_add(1);
        }
    }
    println!(
        "meshlets={} visible={} culled={} outside_frustum={} facing_away={} false_negatives={false_negatives}",
        fixtures.len(),
        reasons[0],
        culled,
        reasons[1],
        reasons[2]
    );
    assert_eq!(false_negatives, 0, "a visible meshlet is never culled");
    assert!(reasons[0] > 0, "the front bank stays visible");
    assert!(reasons[1] > 0, "the far bank is culled by the frustum");
    assert!(reasons[2] > 0, "the flipped bank is culled by the cone");
}

#[test]
fn batch_cull_keeps_input_order_and_repeats() {
    let filter = test_filter();
    let fixtures = fixtures();
    let bounds: Vec<MeshletBounds> = fixtures
        .iter()
        .map(|(positions, meshlet)| meshlet_bounds(positions, meshlet))
        .collect();
    let first = cull_meshlets(&filter, &bounds);
    let second = cull_meshlets(&filter, &bounds);
    assert_eq!(first, second, "the same bounds give the same verdicts");
    assert_eq!(first.len(), bounds.len(), "one verdict per meshlet");
    assert!(first.iter().any(|verdict| verdict.is_visible()));
    assert!(first.iter().any(|verdict| !verdict.is_visible()));
    for (bounds, verdict) in bounds.iter().zip(first.iter()) {
        assert_eq!(
            filter.visible(bounds),
            verdict.is_visible(),
            "single and batch verdicts agree"
        );
    }
}
