//! cluster_vectors: Forward+ z-slice known-answer vectors.
//!
//! Purpose: pin the 1920x1080 tile-16 cluster grid (120x68x32) and the
//! exponential view-depth slices over near 0.5 m .. far 50 m. Owner is the
//! render lane; consumer is the clustered torch-light assignment in
//! `shadow_cluster` (same tile/slice shape on the CPU path).
//! Units: pixels for tiles, meters for view depth, indices for cells.
//! Determinism: pure `f64` math (`ln`/`exp` on fixed inputs only) with
//! stable iteration; presentation-side helper, never hashed.
//! Errors: out-of-range slice/tile queries return `None`, never panic.
//!
//! Attribution (idea-only, zero donor bytes): exponential depth slicing
//! follows the *idea* of Olsson clustered shading and the tile-shape idea
//! of the Godot cluster renderer. Licence unverified, so no donor code,
//! constants, or text was copied; every line below is fresh math.
//!
//! KAV: known-answer vectors for grid dims, slice boundaries, view-depth
//! roundtrip (rel-err <= 1e-4) and exact corner-tile pixel bounds.

const WIDTH_PX: u32 = 1920;
const HEIGHT_PX: u32 = 1080;
const TILE_PX: u32 = 16;
const TILES_X: u32 = 120;
const TILES_Y: u32 = 68;
const SLICES: u32 = 32;
const NEAR_M: f64 = 0.5;
const FAR_M: f64 = 50.0;
/// Roundtrip budget: view-depth encode/decode must agree this well.
const ROUNDTRIP_REL_MAX: f64 = 1e-4;

/// Total cluster cells for the 1080p grid.
const CELLS: u32 = TILES_X * TILES_Y * SLICES;

/// KAV view depths in meters: endpoints, mid-range samples and points near
/// slice seams. Every entry lies inside `[NEAR_M, FAR_M]`.
const KAV_DEPTHS_M: [f64; 14] = [
    0.5, 0.6, 0.75, 1.0, 1.5, 2.0, 3.0, 5.0, 8.0, 12.0, 20.0, 35.0, 49.0, 50.0,
];

/// Ratio `FAR_M / NEAR_M` (100x range).
fn range_ratio() -> f64 {
    FAR_M / NEAR_M
}

/// Log of the range ratio; the single owner of the slice spacing.
fn log_ratio() -> f64 {
    range_ratio().ln()
}

/// Depth boundary `i` for `i` in `0..=SLICES`: `NEAR * ratio^(i/N)`.
/// Returns `None` when `i > SLICES`.
fn boundary_m(i: u32) -> Option<f64> {
    if i > SLICES {
        return None;
    }
    let t = f64::from(i) / f64::from(SLICES);
    Some(NEAR_M * (t * log_ratio()).exp())
}

/// View-depth interval of slice `i` as `(near, far)` in meters.
/// Returns `None` when `i >= SLICES`.
fn slice_bounds_m(slice: u32) -> Option<(f64, f64)> {
    if slice >= SLICES {
        return None;
    }
    let (lo, hi) = (boundary_m(slice)?, boundary_m(slice + 1)?);
    Some((lo, hi))
}

/// Continuous slice coordinate of view depth `d` (units: slices).
/// `NEAR_M -> 0.0`, `FAR_M -> SLICES`. Caller clamps to the grid.
fn depth_to_coord(d_m: f64) -> f64 {
    f64::from(SLICES) * (d_m / NEAR_M).ln() / log_ratio()
}

/// View depth in meters for a continuous slice coordinate.
fn coord_to_depth(coord: f64) -> f64 {
    NEAR_M * ((coord / f64::from(SLICES)) * log_ratio()).exp()
}

/// Slice index for view depth `d_m`: `floor(coord)` clamped to the grid.
/// Depths at/below near map to 0, at/above far map to `SLICES - 1`.
fn slice_of_depth(d_m: f64) -> u32 {
    if !d_m.is_finite() || d_m <= NEAR_M {
        return 0;
    }
    if d_m >= FAR_M {
        return SLICES - 1;
    }
    // Half-open slices: nudge by 1e-9 coords so a boundary value hit by
    // ln/exp roundtrip lands in its own slice instead of one below.
    // 1e-9 coords is far under one slice yet far over f64 noise (~1e-14).
    let c = (depth_to_coord(d_m) + 1e-9).floor() as u32;
    c.min(SLICES - 1)
}

/// Relative error `|a - b| / b` (0 when both are 0).
fn rel_err(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        return (a - b).abs();
    }
    ((a - b) / b).abs()
}

/// Tile holding pixel `(x, y)`; `None` outside the frame.
fn tile_of_pixel(x: u32, y: u32) -> Option<(u32, u32)> {
    if x >= WIDTH_PX || y >= HEIGHT_PX {
        return None;
    }
    Some((x / TILE_PX, y / TILE_PX))
}

/// Pixel bounds `(x0, x1, y0, y1)` of tile `(tx, ty)` with half-open
/// convention; edge tiles clip to the frame. `None` for bad tile ids.
fn tile_bounds_px(tx: u32, ty: u32) -> Option<(u32, u32, u32, u32)> {
    if tx >= TILES_X || ty >= TILES_Y {
        return None;
    }
    let x0 = tx * TILE_PX;
    let y0 = ty * TILE_PX;
    let x1 = (x0 + TILE_PX).min(WIDTH_PX);
    let y1 = (y0 + TILE_PX).min(HEIGHT_PX);
    Some((x0, x1, y0, y1))
}

#[test]
fn grid_dims_are_120x68x32() {
    assert_eq!(WIDTH_PX.div_ceil(TILE_PX), TILES_X);
    assert_eq!(HEIGHT_PX.div_ceil(TILE_PX), TILES_Y);
    assert_eq!((TILES_X, TILES_Y, SLICES), (120, 68, 32));
    assert_eq!(TILES_X * TILES_Y, 8_160);
    assert_eq!(CELLS, 261_120);
    assert_eq!((NEAR_M, FAR_M), (0.5, 50.0));
}

#[test]
fn slice_boundaries_monotonic_contiguous() {
    let mut prev_hi = NEAR_M;
    for i in 0..SLICES {
        let (lo, hi) = slice_bounds_m(i).expect("slice in range");
        assert!(lo.is_finite() && hi.is_finite(), "slice {i} finite");
        assert!(lo < hi, "slice {i} grows");
        assert_eq!(lo, prev_hi, "slice {i} starts where prior ended");
        prev_hi = hi;
    }
    assert_eq!(boundary_m(0), Some(NEAR_M));
    assert!(
        rel_err(prev_hi, FAR_M) <= 1e-12,
        "last far {prev_hi} near {FAR_M}"
    );
    assert_eq!(boundary_m(SLICES + 1), None);
    assert_eq!(slice_bounds_m(SLICES), None);
}

#[test]
fn view_z_roundtrip_rel_err_within_1e_minus_4() {
    let mut worst = 0.0f64;
    for d in KAV_DEPTHS_M {
        let back = coord_to_depth(depth_to_coord(d));
        let e = rel_err(back, d);
        worst = worst.max(e);
        assert!(
            e <= ROUNDTRIP_REL_MAX,
            "depth {d} roundtrip {back} rel-err {e} over budget"
        );
    }
    // Seam neighbourhoods: each boundary approached from both sides must
    // also roundtrip; this catches log/exp asymmetry near slice edges.
    for i in 1..SLICES {
        let b = boundary_m(i).expect("boundary in range");
        for d in [b * (1.0 - 1e-9), b, b * (1.0 + 1e-9)] {
            if d < NEAR_M || d > FAR_M {
                continue;
            }
            let back = coord_to_depth(depth_to_coord(d));
            worst = worst.max(rel_err(back, d));
            assert!(
                rel_err(back, d) <= ROUNDTRIP_REL_MAX,
                "seam {i} depth {d} rel-err over budget"
            );
        }
    }
    assert!(worst <= ROUNDTRIP_REL_MAX, "worst {worst}");
}

#[test]
fn slice_of_depth_kav_vectors() {
    assert_eq!(slice_of_depth(NEAR_M), 0);
    assert_eq!(slice_of_depth(FAR_M), SLICES - 1);
    assert_eq!(slice_of_depth(0.0), 0);
    assert_eq!(slice_of_depth(f64::NAN), 0);
    assert_eq!(slice_of_depth(1e9), SLICES - 1);
    // Mid-slice probes land inside their own slice.
    for i in 0..SLICES {
        let (lo, hi) = slice_bounds_m(i).expect("slice in range");
        let mid = (lo + hi) * 0.5;
        assert_eq!(slice_of_depth(mid), i, "mid of slice {i}");
        assert_eq!(slice_of_depth(lo), i.min(SLICES - 1), "lo of slice {i}");
    }
    // Monotonic: deeper samples never map to a shallower slice.
    let mut prev = 0u32;
    for d in KAV_DEPTHS_M {
        let s = slice_of_depth(d);
        assert!(s >= prev, "depth {d} slice {s} after {prev}");
        prev = s;
    }
}

#[test]
fn corner_tile_bounds_exact() {
    // Top-left, top-right, bottom-left, bottom-right tiles in pixels.
    // Bottom row is partial: 68 * 16 = 1088 > 1080, so height 8.
    assert_eq!(tile_bounds_px(0, 0), Some((0, 16, 0, 16)));
    assert_eq!(tile_bounds_px(119, 0), Some((1904, 1920, 0, 16)));
    assert_eq!(tile_bounds_px(0, 67), Some((0, 16, 1072, 1080)));
    assert_eq!(tile_bounds_px(119, 67), Some((1904, 1920, 1072, 1080)));
    // Pixel roundtrips through the corner tiles.
    assert_eq!(tile_of_pixel(0, 0), Some((0, 0)));
    assert_eq!(tile_of_pixel(15, 15), Some((0, 0)));
    assert_eq!(tile_of_pixel(1919, 0), Some((119, 0)));
    assert_eq!(tile_of_pixel(0, 1079), Some((0, 67)));
    assert_eq!(tile_of_pixel(1919, 1079), Some((119, 67)));
    // Out-of-frame pixels and tiles fail loudly with `None`.
    assert_eq!(tile_of_pixel(1920, 0), None);
    assert_eq!(tile_of_pixel(0, 1080), None);
    assert_eq!(tile_bounds_px(120, 0), None);
    assert_eq!(tile_bounds_px(0, 68), None);
}
