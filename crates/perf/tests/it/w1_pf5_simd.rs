//! Spatial radius A/B: scalar fallback versus the vector-shaped path.
//! One loop only; identical hit sets required, milliseconds are evidence.

use engine_perf::workloads::{
    spatial_query_ab, spatial_query_scalar, spatial_query_simd, SpatialQuery,
};

#[test]
fn w1_pf5_simd_matches_scalar_and_reports_ab() {
    let workload = SpatialQuery::canonical();
    assert_eq!(SpatialQuery::NAME, "spatial_query");
    let (xs, zs) = workload.build_points();
    assert_eq!(xs.len(), workload.points, "setup builds every lane");
    assert_eq!(zs.len(), workload.points, "setup builds every lane");
    for v in xs.iter().chain(zs.iter()) {
        assert!(v.is_finite(), "lanes stay finite");
    }
    let radius_sq = workload.radius_sq();
    let mut out_scalar = vec![0u32; xs.len()];
    let mut out_simd = vec![0u32; xs.len()];
    let n_scalar = spatial_query_scalar(
        &xs,
        &zs,
        workload.qx,
        workload.qz,
        radius_sq,
        &mut out_scalar,
    );
    let n_simd = spatial_query_simd(&xs, &zs, workload.qx, workload.qz, radius_sq, &mut out_simd);
    assert_eq!(n_scalar, n_simd, "one loop, one count");
    assert_eq!(
        &out_scalar[..n_scalar],
        &out_simd[..n_simd],
        "hit sets bit-identical"
    );

    // Remainder tail: 7 points force the scalar tail (fails if it drops any).
    let tail = SpatialQuery {
        points: 7,
        ..workload
    };
    let (txs, tzs) = tail.build_points();
    let mut tail_a = vec![0u32; 7];
    let mut tail_b = vec![0u32; 7];
    let ta = spatial_query_scalar(&txs, &tzs, tail.qx, tail.qz, tail.radius_sq(), &mut tail_a);
    let tb = spatial_query_simd(&txs, &tzs, tail.qx, tail.qz, tail.radius_sq(), &mut tail_b);
    assert_eq!(ta, tb, "tail counts agree");
    assert_eq!(&tail_a[..ta], &tail_b[..tb], "tail hits agree");

    // Empty input finds nothing on both paths.
    let mut empty_out = Vec::new();
    assert_eq!(
        spatial_query_scalar(&[], &[], 0.0, 0.0, 1.0, &mut empty_out),
        0,
        "empty scalar finds nothing"
    );
    assert_eq!(
        spatial_query_simd(&[], &[], 0.0, 0.0, 1.0, &mut empty_out),
        0,
        "empty vector finds nothing"
    );

    // Moved seed moves the build (fails if the loop ignores its input).
    let moved = workload.with_seed(workload.seed + 1);
    let (mxs, mzs) = moved.build_points();
    assert_ne!((mxs, mzs), (xs.clone(), zs.clone()), "seed moves the lanes");

    let (scalar_ms, simd_ms) =
        spatial_query_ab(&workload, &xs, &zs, &mut out_scalar, &mut out_simd, 120);
    assert_eq!(
        &out_scalar[..n_scalar],
        &out_simd[..n_simd],
        "timing passes keep the hit sets"
    );
    println!(
        "spatial_query A/B: scalar {scalar_ms:.3} ms -> simd {simd_ms:.3} ms, identical 1/1 (hits={n_scalar})"
    );
}
