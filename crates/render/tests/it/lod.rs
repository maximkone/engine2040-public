//! lod: LOD triangle-count steps (count drops, look kept).
//!
//! Purpose: pin that coarser LOD levels carry strictly fewer triangles
//! while the node stays visible with the same identity (the look is kept,
//! never culled to zero). Owner is the render lane; consumer is the RA2
//! executor, which feeds `lod` back into `prev_lod`.
//! Units: meters for distance, triangles for cost.
//! Determinism: fixed camera/table/distances, no RNG; rebuilds must agree.
use engine_render::draw::{Camera, DrawList, LodTable, SceneNode, SceneSnapshot};
const TRIS_PER_LOD: [u64; 4] = [6_000, 3_000, 1_200, 400];
const DIST_M_PER_LOD: [f32; 4] = [5.0, 15.0, 30.0, 50.0];
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
    }
}
fn node_at(id: u64, dist_m: f32, prev_lod: u8) -> SceneNode {
    SceneNode {
        id,
        mesh: 42,
        center_m: [0.0, 0.0, -dist_m],
        radius_m: 0.5,
        lod_levels: 4,
        lod_table: 7,
        prev_lod,
    }
}
fn tris_for_lod(lod: u8) -> u64 {
    TRIS_PER_LOD[lod as usize]
}
#[test]
fn lod_steps_drop_triangle_count() {
    let camera = test_camera();
    let table = test_table();
    let mut list = DrawList::new();
    let mut prev_tris = u64::MAX;
    for (step, dist) in DIST_M_PER_LOD.iter().enumerate() {
        let expected_lod = step as u8;
        for prev in [0u8, 3u8] {
            let snapshot = SceneSnapshot {
                nodes: vec![node_at(1, *dist, prev)],
            };
            let draws = list.build(&snapshot, &camera, &table).expect("builds");
            assert_eq!(draws.len(), 1, "step {step} culled at {dist} m");
            assert_eq!(
                draws[0].lod, expected_lod,
                "step {step} picked wrong LOD at {dist} m from prev {prev}"
            );
        }
        let tris = tris_for_lod(expected_lod);
        assert!(
            tris < prev_tris,
            "step {step}: {tris} tris did not drop below {prev_tris}"
        );
        prev_tris = tris;
    }
    assert!(TRIS_PER_LOD.windows(2).all(|w| w[0] > w[1]));
    assert!(TRIS_PER_LOD[0] > TRIS_PER_LOD[3]);
}
#[test]
fn coarsest_lod_keeps_the_look() {
    let camera = test_camera();
    let table = test_table();
    let mut list = DrawList::new();
    let far = DIST_M_PER_LOD[3];
    let snapshot = SceneSnapshot {
        nodes: vec![node_at(1, far, 0)],
    };
    let draws = list.build(&snapshot, &camera, &table).expect("builds");
    assert_eq!(draws.len(), 1, "far node culled");
    assert_eq!((draws[0].id, draws[0].mesh), (1, 42));
    assert_eq!(draws[0].lod, 3);
    assert!(
        tris_for_lod(draws[0].lod) > 0,
        "coarsest LOD must keep triangles"
    );
    let digest = list.order_digest();
    let draws2 = list.build(&snapshot, &camera, &table).expect("rebuilds");
    assert_eq!(draws2[0].lod, 3);
    assert_eq!(list.order_digest(), digest);
}
#[test]
fn frame_total_drops_as_dolly_recedes() {
    let camera = test_camera();
    let table = test_table();
    let mut list = DrawList::new();
    let near = SceneSnapshot {
        nodes: DIST_M_PER_LOD
            .iter()
            .enumerate()
            .map(|(i, d)| node_at(100 + i as u64, *d, 0))
            .collect(),
    };
    let draws = list.build(&near, &camera, &table).expect("builds");
    assert_eq!(draws.len(), 4);
    let near_total: u64 = draws.iter().map(|d| tris_for_lod(d.lod)).sum();
    assert_eq!(near_total, TRIS_PER_LOD.iter().sum::<u64>());
    let mut prev_lods = [0u8; 4];
    for (i, d) in draws.iter().enumerate() {
        prev_lods[i] = d.lod;
    }
    let far = SceneSnapshot {
        nodes: DIST_M_PER_LOD
            .iter()
            .enumerate()
            .map(|(i, d)| node_at(100 + i as u64, *d + 30.0, prev_lods[i]))
            .collect(),
    };
    let draws_far = list.build(&far, &camera, &table).expect("builds");
    assert_eq!(draws_far.len(), 4, "dolly culled a node");
    let far_total: u64 = draws_far.iter().map(|d| tris_for_lod(d.lod)).sum();
    assert!(
        far_total < near_total,
        "far total {far_total} did not drop below near {near_total}"
    );
    assert!(far_total > 0, "far frame must keep geometry");
}
