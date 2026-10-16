//! meshlet_cluster_vectors: cone cluster-cull score over a meshlet fixture.
//!
//! Purpose: pin the cluster-cull cut (rasterized tris down >= 30%) with a
//! fresh cone test. Owner is the render lane; consumer is the future
//! virtual-geometry cluster cut (`crates/render/src/meshlet.rs`).
//! Units: axes unitless, cutoff is a cosine, tris are counts, ratios f64.
//! Determinism: pure `f32` math (`+ - * /`, `sqrt` on fixed inputs) with
//! stable order; presentation-side helper, never hashed.
//! Errors: degenerate or non-finite cones never cull (return visible).
//!
//! Attribution (idea-only, zero donor bytes): cone/cluster cull-score idea
//! follows `getMeshletScore` in `clusterizer.cpp` from zeux/meshoptimizer
//! (MIT, https://github.com/zeux/meshoptimizer). Every line below is fresh
//! Rust; no donor code, constants, or text was copied.
//!
//! KAV: 10-meshlet cube fixture (640 tris); view +Z culls 4 back meshlets
//! (256 tris, cut 0.40); scores order front > side > back; rerun identical.

/// One meshlet cone: outward axis, cull cutoff, triangle count.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Meshlet {
    /// Cone axis (need not be unit; zero means degenerate).
    axis: [f32; 3],
    /// Cull cutoff as a cosine: culled when `dot(axis, view) < cutoff`.
    cutoff: f32,
    /// Triangles in this meshlet.
    tris: u32,
}

/// Cull summary over a fixture for one view direction.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CullReport {
    total_tris: u32,
    culled_tris: u32,
    drawn_tris: u32,
    /// `culled / total` in `0..=1`.
    cut_ratio: f64,
}

/// Dot product of two 3-vectors.
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Length of a 3-vector.
fn len(v: [f32; 3]) -> f32 {
    dot(v, v).sqrt()
}

/// Normalize, or zero when too short or non-finite.
fn normalize_or_zero(v: [f32; 3]) -> [f32; 3] {
    if !v.iter().all(|x| x.is_finite()) {
        return [0.0, 0.0, 0.0];
    }
    let l = len(v);
    if !l.is_finite() || l < 1.0e-12 {
        return [0.0, 0.0, 0.0];
    }
    [v[0] / l, v[1] / l, v[2] / l]
}

/// True when the cone carries no culling information.
fn is_degenerate(m: &Meshlet) -> bool {
    if !m.cutoff.is_finite() || !m.axis.iter().all(|x| x.is_finite()) {
        return true;
    }
    len(m.axis) < 1.0e-6
}

/// Cluster-cull score for one meshlet and one view.
///
/// Fresh Rust inspired by the `getMeshletScore` idea: visibility margin
/// `dot(axis, view) - cutoff`. Positive means visible, negative means
/// culled. Degenerate cones and bad views return `1.0` (visible).
fn cluster_cull_score(m: &Meshlet, to_camera: [f32; 3]) -> f32 {
    if is_degenerate(m) {
        return 1.0;
    }
    let n = normalize_or_zero(m.axis);
    let v = normalize_or_zero(to_camera);
    if len(v) < 1.0e-12 {
        return 1.0;
    }
    dot(n, v) - m.cutoff
}

/// True when the whole meshlet is back-facing and may be skipped.
fn cone_culled(m: &Meshlet, to_camera: [f32; 3]) -> bool {
    if is_degenerate(m) {
        return false;
    }
    let v = normalize_or_zero(to_camera);
    if len(v) < 1.0e-12 || !v.iter().all(|x| x.is_finite()) {
        return false;
    }
    cluster_cull_score(m, to_camera) < 0.0
}

/// Cull every meshlet for one view; stable order, no allocation beyond out.
fn cull_report(meshlets: &[Meshlet], to_camera: [f32; 3]) -> CullReport {
    let mut total: u32 = 0;
    let mut culled: u32 = 0;
    for m in meshlets {
        total = total.saturating_add(m.tris);
        if cone_culled(m, to_camera) {
            culled = culled.saturating_add(m.tris);
        }
    }
    let drawn = total.saturating_sub(culled);
    let cut_ratio = if total == 0 {
        0.0
    } else {
        f64::from(culled) / f64::from(total)
    };
    CullReport {
        total_tris: total,
        culled_tris: culled,
        drawn_tris: drawn,
        cut_ratio,
    }
}

/// 10-meshlet cube fixture: 4 back, 4 front, 2 sides; 64 tris each.
fn fixture_cube_10() -> [Meshlet; 10] {
    let back = Meshlet {
        axis: [0.0, 0.0, -1.0],
        cutoff: 0.0,
        tris: 64,
    };
    let front = Meshlet {
        axis: [0.0, 0.0, 1.0],
        cutoff: 0.0,
        tris: 64,
    };
    [
        back,
        back,
        back,
        back,
        front,
        front,
        front,
        front,
        Meshlet {
            axis: [1.0, 0.0, 0.0],
            cutoff: 0.0,
            tris: 64,
        },
        Meshlet {
            axis: [-1.0, 0.0, 0.0],
            cutoff: 0.0,
            tris: 64,
        },
    ]
}

/// Camera direction for the KAV: camera sits at +Z looking at the cube.
fn kav_view() -> [f32; 3] {
    [0.0, 0.0, 1.0]
}

#[test]
fn cone_cull_cuts_rasterized_tris_at_least_30_percent() {
    let meshlets = fixture_cube_10();
    let report = cull_report(&meshlets, kav_view());
    assert_eq!(report.total_tris, 640);
    assert_eq!(report.culled_tris, 256);
    assert_eq!(report.drawn_tris, 384);
    assert!(
        (report.cut_ratio - 0.40).abs() < 1.0e-12,
        "cut {} want 0.40",
        report.cut_ratio
    );
    assert!(
        report.cut_ratio >= 0.30,
        "cut {} must move rasterized tris down >= 30%",
        report.cut_ratio
    );
    assert!(report.drawn_tris < report.total_tris);
    println!(
        "meshlet cut: total {} culled {} drawn {} ratio {:.4}",
        report.total_tris, report.culled_tris, report.drawn_tris, report.cut_ratio
    );
}

#[test]
fn cull_score_deterministic_x2() {
    let meshlets = fixture_cube_10();
    let first = cull_report(&meshlets, kav_view());
    let second = cull_report(&meshlets, kav_view());
    assert_eq!(first, second, "rerun must be identical");
    let scores_first: Vec<f32> = meshlets
        .iter()
        .map(|m| cluster_cull_score(m, kav_view()))
        .collect();
    let scores_second: Vec<f32> = meshlets
        .iter()
        .map(|m| cluster_cull_score(m, kav_view()))
        .collect();
    assert_eq!(scores_first, scores_second, "scores rerun identical");
    assert_eq!(first.total_tris, 640);
    println!(
        "deterministic x2: cut {:.4} scores {:?}",
        first.cut_ratio, scores_first
    );
}

#[test]
fn degenerate_cone_never_culls() {
    let view = kav_view();
    let zero = Meshlet {
        axis: [0.0, 0.0, 0.0],
        cutoff: 0.0,
        tris: 64,
    };
    assert!(!cone_culled(&zero, view), "zero axis stays visible");
    assert!(cluster_cull_score(&zero, view) > 0.0);
    let nan = Meshlet {
        axis: [f32::NAN, 0.0, 0.0],
        cutoff: 0.0,
        tris: 64,
    };
    assert!(!cone_culled(&nan, view), "NaN axis stays visible");
    let bad_cutoff = Meshlet {
        axis: [0.0, 0.0, -1.0],
        cutoff: f32::INFINITY,
        tris: 64,
    };
    assert!(
        !cone_culled(&bad_cutoff, view),
        "non-finite cutoff stays visible"
    );
    let back = Meshlet {
        axis: [0.0, 0.0, -1.0],
        cutoff: 0.0,
        tris: 64,
    };
    assert!(
        !cone_culled(&back, [0.0, 0.0, 0.0]),
        "zero view stays visible"
    );
    assert!(
        !cone_culled(&back, [f32::NAN, 0.0, 0.0]),
        "NaN view stays visible"
    );
}

#[test]
fn cone_scores_order_front_above_side_above_back() {
    let view = kav_view();
    let front = Meshlet {
        axis: [0.0, 0.0, 1.0],
        cutoff: 0.0,
        tris: 64,
    };
    let side = Meshlet {
        axis: [1.0, 0.0, 0.0],
        cutoff: 0.0,
        tris: 64,
    };
    let back = Meshlet {
        axis: [0.0, 0.0, -1.0],
        cutoff: 0.0,
        tris: 64,
    };
    let sf = cluster_cull_score(&front, view);
    let ss = cluster_cull_score(&side, view);
    let sb = cluster_cull_score(&back, view);
    assert_eq!(sf, 1.0);
    assert_eq!(ss, 0.0);
    assert_eq!(sb, -1.0);
    assert!(
        sf.total_cmp(&ss) == core::cmp::Ordering::Greater,
        "front {sf} above side {ss}"
    );
    assert!(
        ss.total_cmp(&sb) == core::cmp::Ordering::Greater,
        "side {ss} above back {sb}"
    );
    assert!(!cone_culled(&front, view));
    assert!(!cone_culled(&side, view), "grazing side stays drawn");
    assert!(cone_culled(&back, view), "back cone culls");
}
