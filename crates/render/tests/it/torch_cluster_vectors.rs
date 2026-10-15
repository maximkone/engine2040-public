//! torch_cluster_vectors: the clustered torch assignment as its callers see it.
//!
//! Consumer: the window `submit_shadow_cluster` and the post chain reach the
//! CPU assignment through the crate root
//! (`engine_render::shadow_cluster::{ClusterPass, ClusterConfig, ...}`).
//! Units: pixels for centers and radii, meters for depth and range, ms only
//! inside receipts (recorded, never asserted here). Determinism: sorted
//! cell-major order, so one input script builds bit-identical passes.
//! Errors are typed [`LightError`](engine_render::shadow_cluster::LightError):
//! over-budget inputs degrade (clamp plus a recorded error), bad inputs fail.
//!
//! Why this file exists: `crates/render/src/shadow_cluster/torch_cluster.rs`
//! changed 4 times in 30 days yet held zero `#[test]` and no `tests/it`
//! module named it, so a dropped re-export or a loosened bound would keep
//! the crate green while breaking every caller at their `use` line. Each
//! vector below enters through the public root path with hand-computed
//! expectations, so a behavior break fails here first.

use engine_render::shadow_cluster::{
    ClusterConfig, ClusterPass, LightError, ScreenLight, TorchLight, CASCADE_LAMBDA,
    CLUSTER_TILE_PX, MAX_CLUSTER_CELLS, MAX_DEPTH_SLICES, MAX_LIGHTS_PER_CLUSTER, MAX_TORCH_LIGHTS,
    RT19_REJECT_DEPTH_M, RT19_TIGHT_FAR_M, RT19_TIGHT_NEAR_M,
};

/// The pinned re-export contract: every name a caller reaches through the
/// crate root resolves with the reference bounds.
#[test]
fn root_re_exports_resolve_with_reference_bounds() {
    assert_eq!(CLUSTER_TILE_PX, 16, "tile edge via root");
    assert_eq!(MAX_LIGHTS_PER_CLUSTER, 24, "lights per cell via root");
    assert_eq!(MAX_DEPTH_SLICES, 32, "depth slices via root");
    assert_eq!(MAX_TORCH_LIGHTS, 256, "lights per frame via root");
    assert_eq!(MAX_CLUSTER_CELLS, 1 << 20, "cell ceiling via root");
    assert_eq!(CASCADE_LAMBDA, 0.5, "slice blend via root");
    assert_eq!(RT19_TIGHT_NEAR_M, 0.5, "tightened near via root");
    assert_eq!(RT19_TIGHT_FAR_M, 20.0, "tightened far via root");
    assert_eq!(RT19_REJECT_DEPTH_M, 20.0, "early-z reject via root");
    TorchLight {
        pos: [1.0, 2.0, 3.0],
        range_m: 5.0,
        color: [1.0, 0.6, 0.2],
        intensity: 2.0,
    }
    .validate()
    .expect("valid torch validates via root");
}

/// One tile, one slice, one light dead center with a huge radius: the whole
/// pass is exactly one pair in cell zero.
fn tiny_config() -> ClusterConfig {
    ClusterConfig {
        width_px: 16,
        height_px: 16,
        tile_px: 16,
        depth_slices: 1,
        near_m: 0.5,
        far_m: 60.0,
        max_lights: MAX_TORCH_LIGHTS,
        max_per_cluster: MAX_LIGHTS_PER_CLUSTER,
    }
}

fn wide_center_light() -> ScreenLight {
    ScreenLight {
        x_px: 8.0,
        y_px: 8.0,
        depth_m: 5.0,
        radius_px: 100.0,
    }
}

#[test]
fn single_cell_assign_is_exact_and_deterministic() {
    let config = tiny_config();
    assert_eq!((config.tiles_x(), config.tiles_y()), (1, 1));
    assert_eq!(config.cells(), 1);
    let pass = ClusterPass::build(config, &[wide_center_light()]).expect("builds via root");
    assert_eq!(pass.counts.as_slice(), &[1], "one pair in the one cell");
    assert_eq!(pass.offsets.as_slice(), &[0], "prefix starts at zero");
    assert_eq!(pass.indices.as_slice(), &[0], "light zero stored");
    assert_eq!(pass.lights_in(0), &[0], "cell slice reads back");
    assert_eq!(pass.dropped_lights, 0, "nothing dropped");
    assert_eq!(pass.overflow_cells, 0, "nothing clamped");
    assert!(pass.degraded.is_none(), "clean build degrades nothing");
    let again = ClusterPass::build(config, &[wide_center_light()]).expect("rebuilds");
    assert_eq!(pass.counts, again.counts, "counts x2 identical");
    assert_eq!(pass.offsets, again.offsets, "offsets x2 identical");
    assert_eq!(pass.indices, again.indices, "indices x2 identical");
    assert_eq!(
        pass.rt19_digest(),
        again.rt19_digest(),
        "digest x2 identical"
    );
    let receipt = pass.receipt();
    assert_eq!(receipt.pass, "torch_cluster", "receipt names the pass");
    assert_eq!(receipt.texels, 0, "cluster pass owns no texels");
    assert_eq!(receipt.cells, 1, "receipt carries the cell count");
    assert!(!receipt.degraded, "clean receipt is not degraded");
}

#[test]
fn empty_build_is_all_zero_with_a_different_digest() {
    let full = ClusterPass::build(tiny_config(), &[wide_center_light()]).expect("builds");
    let empty = ClusterPass::build(tiny_config(), &[]).expect("empty builds");
    assert_eq!(empty.counts.as_slice(), &[0], "no pairs assigned");
    assert_eq!(empty.offsets.as_slice(), &[0], "prefix stays zero");
    assert!(empty.indices.is_empty(), "no indices stored");
    assert!(empty.lights_in(0).is_empty(), "cell slice is empty");
    assert!(empty.degraded.is_none(), "empty build degrades nothing");
    assert!(!empty.receipt().degraded, "empty receipt is not degraded");
    assert_ne!(
        empty.rt19_digest(),
        full.rt19_digest(),
        "digest pins content: empty differs from lit"
    );
}

#[test]
fn far_offscreen_light_builds_but_assigns_nothing() {
    let offscreen = ScreenLight {
        x_px: -500.0,
        y_px: -500.0,
        depth_m: 5.0,
        radius_px: 1.0,
    };
    let pass = ClusterPass::build(tiny_config(), &[offscreen]).expect("builds");
    assert_eq!(
        pass.counts.as_slice(),
        &[0],
        "circle never touches the tile"
    );
    assert!(pass.indices.is_empty(), "no pairs stored");
    assert!(pass.degraded.is_none(), "a miss is not a degrade");
}

#[test]
fn tail_past_max_lights_degrades_with_exact_counts() {
    let config = ClusterConfig {
        max_lights: 1,
        ..tiny_config()
    };
    let lights = [wide_center_light(), wide_center_light()];
    let pass = ClusterPass::build(config, &lights).expect("tail drops, never fails");
    assert_eq!(pass.dropped_lights, 1, "exactly the tail drops");
    assert!(
        matches!(
            pass.degraded,
            Some(LightError::BudgetExceeded {
                requested: 2,
                allowed: 1
            })
        ),
        "degrade records 2 requested 1 used, got {:?}",
        pass.degraded
    );
    assert_eq!(pass.counts.as_slice(), &[1], "kept prefix assigns");
    assert_eq!(pass.indices.as_slice(), &[0], "kept prefix is light zero");
    assert!(pass.receipt().degraded, "receipt flags the degrade");
}

#[test]
fn torch_validate_rejects_every_bad_field() {
    let good = TorchLight {
        pos: [1.0, 2.0, 3.0],
        range_m: 5.0,
        color: [1.0, 0.6, 0.2],
        intensity: 2.0,
    };
    assert!(good.validate().is_ok(), "baseline validates");
    let mut bad = good;
    bad.pos[0] = f32::NAN;
    assert!(
        matches!(bad.validate(), Err(LightError::Invalid(_))),
        "NaN position fails"
    );
    let mut bad = good;
    bad.range_m = 0.0;
    assert!(
        matches!(bad.validate(), Err(LightError::Invalid(_))),
        "zero range fails"
    );
    let mut bad = good;
    bad.range_m = -2.0;
    assert!(
        matches!(bad.validate(), Err(LightError::Invalid(_))),
        "negative range fails"
    );
    let mut bad = good;
    bad.range_m = f32::INFINITY;
    assert!(
        matches!(bad.validate(), Err(LightError::Invalid(_))),
        "infinite range fails"
    );
    let mut bad = good;
    bad.color[1] = -0.5;
    assert!(
        matches!(bad.validate(), Err(LightError::Invalid(_))),
        "negative color fails"
    );
    let mut bad = good;
    bad.color[2] = f32::NAN;
    assert!(
        matches!(bad.validate(), Err(LightError::Invalid(_))),
        "NaN color fails"
    );
    let mut bad = good;
    bad.intensity = -1.0;
    assert!(
        matches!(bad.validate(), Err(LightError::Invalid(_))),
        "negative intensity fails"
    );
    let mut bad = good;
    bad.intensity = f32::NAN;
    assert!(
        matches!(bad.validate(), Err(LightError::Invalid(_))),
        "NaN intensity fails"
    );
}

#[test]
fn screen_light_validate_rejects_non_finite_and_negative_radius() {
    let good = wide_center_light();
    assert!(good.validate().is_ok(), "baseline validates");
    let point = ScreenLight {
        radius_px: 0.0,
        ..good
    };
    assert!(
        point.validate().is_ok(),
        "zero radius is a valid point light"
    );
    let mut bad = good;
    bad.x_px = f32::NAN;
    assert!(
        matches!(bad.validate(), Err(LightError::Invalid(_))),
        "NaN center fails"
    );
    let mut bad = good;
    bad.depth_m = f32::INFINITY;
    assert!(
        matches!(bad.validate(), Err(LightError::Invalid(_))),
        "infinite depth fails"
    );
    let mut bad = good;
    bad.radius_px = -1.0;
    assert!(
        matches!(bad.validate(), Err(LightError::Invalid(_))),
        "negative radius fails"
    );
    let mut bad = good;
    bad.radius_px = f32::NAN;
    assert!(
        matches!(bad.validate(), Err(LightError::Invalid(_))),
        "NaN radius fails"
    );
}

#[test]
fn config_validate_rejects_bad_geometry_and_budgets() {
    let good = tiny_config();
    assert!(good.validate().is_ok(), "baseline validates");
    for broken in [
        ClusterConfig {
            width_px: 0,
            ..good
        },
        ClusterConfig {
            width_px: 2049,
            ..good
        },
        ClusterConfig { tile_px: 0, ..good },
        ClusterConfig {
            tile_px: 17,
            ..good
        },
        ClusterConfig {
            depth_slices: 0,
            ..good
        },
        ClusterConfig {
            depth_slices: MAX_DEPTH_SLICES + 1,
            ..good
        },
        ClusterConfig {
            near_m: 0.0,
            ..good
        },
        ClusterConfig {
            far_m: good.near_m,
            ..good
        },
        ClusterConfig {
            near_m: f32::NAN,
            ..good
        },
        ClusterConfig {
            max_lights: 0,
            ..good
        },
        ClusterConfig {
            max_per_cluster: 0,
            ..good
        },
    ] {
        assert!(broken.validate().is_err(), "bad config fails: {broken:?}");
    }
}

#[test]
fn grid_arithmetic_tiles_slices_and_cells_exactly() {
    let config = ClusterConfig {
        width_px: 64,
        height_px: 64,
        tile_px: CLUSTER_TILE_PX,
        depth_slices: 4,
        ..tiny_config()
    };
    assert_eq!((config.tiles_x(), config.tiles_y()), (4, 4));
    assert_eq!(config.cells(), 64, "4 x 4 tiles x 4 slices");
    let ragged = ClusterConfig {
        width_px: 65,
        height_px: 65,
        ..config
    };
    assert_eq!((ragged.tiles_x(), ragged.tiles_y()), (5, 5));
    let bounds = config.slice_bounds();
    assert_eq!(bounds.len(), 5, "slices + 1 boundaries");
    assert_eq!(
        (bounds[0], bounds[4]),
        (0.5, 60.0),
        "endpoints pin near/far"
    );
    for pair in bounds.windows(2) {
        assert!(pair[0] < pair[1], "bounds increase: {bounds:?}");
    }
    let pass = ClusterPass::build(config, &[]).expect("empty builds for layout");
    assert_eq!(pass.cell_index(0, 0, 0), 0, "origin cell");
    assert_eq!(pass.cell_index(1, 0, 0), 1, "x strides by one");
    assert_eq!(pass.cell_index(0, 1, 0), 4, "y strides by tiles_x");
    assert_eq!(pass.cell_index(0, 0, 1), 16, "slice strides by tile plane");
    assert_eq!(pass.cell_index(3, 3, 3), 63, "last cell is cells - 1");
}

#[test]
fn rt19_tightened_clamps_far_and_rejects_deep_centers() {
    let base = ClusterConfig {
        width_px: 32,
        height_px: 32,
        tile_px: CLUSTER_TILE_PX,
        depth_slices: 2,
        near_m: 0.5,
        far_m: 50.0,
        max_lights: MAX_TORCH_LIGHTS,
        max_per_cluster: MAX_LIGHTS_PER_CLUSTER,
    };
    assert_eq!(base.cells(), 8, "2 x 2 tiles x 2 slices");
    let near_light = ScreenLight {
        x_px: 8.0,
        y_px: 8.0,
        depth_m: 10.0,
        radius_px: 4.0,
    };
    let deep_light = ScreenLight {
        x_px: 24.0,
        y_px: 24.0,
        depth_m: 35.0,
        radius_px: 4.0,
    };
    let tight = ClusterPass::build_rt19_tightened(base, &[near_light, deep_light])
        .expect("tightened builds");
    assert_eq!(
        tight.config.near_m, RT19_TIGHT_NEAR_M,
        "near keeps its shape"
    );
    assert_eq!(tight.config.far_m, RT19_TIGHT_FAR_M, "far clamps to 20 m");
    assert!(
        !tight.indices.is_empty(),
        "the kept light still assigns pairs"
    );
    assert!(
        tight.indices.iter().all(|&i| i == 0),
        "only kept position zero survives the reject: {:?}",
        tight.indices
    );
}

#[test]
fn headless_submit_gpu_names_the_window_entry() {
    let pass = ClusterPass::build(tiny_config(), &[wide_center_light()]).expect("builds");
    match pass.submit_gpu() {
        Err(LightError::Invalid(detail)) => assert!(
            detail.contains("submit_shadow_cluster"),
            "seam names the window entry: {detail}"
        ),
        other => panic!("headless seam must fail Invalid, got {other:?}"),
    }
}
