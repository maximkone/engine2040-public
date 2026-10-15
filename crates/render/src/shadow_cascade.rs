//! shadow_cascade: cascade-split selection plus per-cascade shadow-map
//! sizing for the arena-day sun (RL27).
//!
//! Owner: render lane. The sun cascade count and the practical-split
//! blend live in `shadow_cluster` (`SUN_CASCADES`, `CASCADE_LAMBDA`);
//! this file only turns a camera range into pinned split distances and
//! map sizes. Std only. Consumer: the day-sweep fixture, then RL30.
//! Units: meters for ranges and splits, texels for map edges.
//! Determinism: pure functions of the inputs; same range gives the same
//! splits and sizes, byte-identical x2. Errors: [`CascadeError`] names
//! the bad range loudly. Overlap build ports Bevy's log-split builder
//! plus overlap proportion (see `CASCADE_OVERLAP`); the legacy practical
//! splits stay pinned beside it.

use super::shadow_cluster::{practical_split, CASCADE_LAMBDA, SUN_CASCADES};
use std::cmp::Ordering;
use std::error::Error;
use std::fmt;

/// Map edge in texels per cascade, near to far. Pinned as data.
pub const CASCADE_MAP_SIZES: [u32; 3] = [512, 256, 128];

/// Float divisor matching `SUN_CASCADES`; a named constant so the
/// practical-split blend never casts the count through `as`.
const CASCADE_DIVISOR: f32 = 3.0;
const _: [(); SUN_CASCADES] = [(); CASCADE_MAP_SIZES.len()];

// PORT(bevyengine/bevy@59fed6a181902d40ccd739dd8f7c97d6d5714f42:crates/bevy_light/src/cascade.rs)
// Apache-2.0 (LICENSE-APACHE read live 2026-10-09; the repo is dual
// MIT/Apache-2.0). What: `calculate_cascade_bounds` log-spaced far bounds
// plus the `overlap_proportion` near bounds (`overlap_factor * bound`).
// Adapted, no bytes pasted: Bevy's `Vec<f32>` plus ECS/camera deps become
// fixed `[f32; 3]` pure math over meters; the 0.2 overlap pins below.

/// Proportion of each cascade overlapping the previous one in far-bound
/// units (Bevy `overlap_proportion` default 0.2). Cascade `i > 0` opens at
/// `far[i - 1] * (1 - CASCADE_OVERLAP)`, so neighbours blend instead of
/// cutting hard. Unitless, pinned in `[0, 1)`.
pub const CASCADE_OVERLAP: f32 = 0.2;

/// Far edge of the first cascade in meters (Bevy `first_cascade_far_bound`
/// default 10.0). The log build spaces cascades geometrically from this
/// edge to the camera far; `day_row` pins it so the day sweep stays data.
pub const FIRST_CASCADE_FAR_M: f32 = 10.0;

/// Split distances in meters over a camera range (RL27).
/// Practical split: half logarithmic plus half uniform, with the blend
/// from `shadow_cluster::CASCADE_LAMBDA`. Index `i` holds the far edge
/// of cascade `i`; the last split always equals `far_m`.
#[must_use]
pub fn split_distances(near_m: f32, far_m: f32) -> [f32; 3] {
    let mut splits = [0.0_f32; 3];
    for (i, split) in splits.iter_mut().enumerate() {
        *split = practical_split(
            near_m,
            far_m,
            (i + 1) as f32 / CASCADE_DIVISOR,
            CASCADE_LAMBDA,
        );
    }
    splits[SUN_CASCADES - 1] = far_m;
    splits
}

/// Checked split selection: bad ranges fail loudly, never clamp
/// silently (a clamped shadow range would hide a broken day rig).
pub fn checked_splits(near_m: f32, far_m: f32) -> Result<[f32; 3], CascadeError> {
    if near_m.partial_cmp(&0.0) != Some(Ordering::Greater) {
        return Err(CascadeError::BadRange {
            near_m,
            far_m,
            reason: "near must be over 0",
        });
    }
    if far_m.partial_cmp(&near_m) != Some(Ordering::Greater) {
        return Err(CascadeError::BadRange {
            near_m,
            far_m,
            reason: "far must be past near",
        });
    }
    Ok(split_distances(near_m, far_m))
}

/// Log-spaced far edge per cascade in meters over `[first_far_m, far_m]`
/// (Bevy `calculate_cascade_bounds` for 3 cascades). Index `i` holds the
/// far edge of cascade `i`: geometrically spaced from the first far edge,
/// so each step multiplies by `sqrt(far_m / first_far_m)`; the last edge
/// always equals `far_m`. Pure; same inputs give the same edges.
/// Callers must validate `0 < first_far_m < far_m` via [`checked_ranges`].
#[must_use]
pub fn log_cascade_bounds(first_far_m: f32, far_m: f32) -> [f32; 3] {
    let base = (far_m / first_far_m).powf(1.0 / 2.0);
    let mut bounds = [first_far_m, first_far_m * base, far_m];
    bounds[SUN_CASCADES - 1] = far_m;
    bounds
}

/// One cascade interval in meters: overlap-aware near plus far edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CascadeRange {
    /// Near edge in meters; cascade 0 uses the camera near, later cascades
    /// reuse the previous far scaled by `1 - CASCADE_OVERLAP`.
    pub near_m: f32,
    /// Far edge in meters from [`log_cascade_bounds`].
    pub far_m: f32,
}

/// Overlap-aware cascade intervals over a camera range (Bevy near bounds:
/// the camera near, then each previous far scaled by the overlap factor).
/// Pure; same range gives the same intervals. Callers must validate the
/// range via [`checked_ranges`]: `0 < near_m < first_far_m < far_m`.
#[must_use]
pub fn cascade_ranges(near_m: f32, first_far_m: f32, far_m: f32) -> [CascadeRange; 3] {
    let far = log_cascade_bounds(first_far_m, far_m);
    let overlap_factor = 1.0 - CASCADE_OVERLAP;
    let near = [near_m, far[0] * overlap_factor, far[1] * overlap_factor];
    [
        CascadeRange {
            near_m: near[0],
            far_m: far[0],
        },
        CascadeRange {
            near_m: near[1],
            far_m: far[1],
        },
        CascadeRange {
            near_m: near[2],
            far_m: far[2],
        },
    ]
}

/// Checked overlap-aware intervals: bad ranges fail loudly, never clamp
/// silently (a clamped shadow range would hide a broken day rig).
pub fn checked_ranges(
    near_m: f32,
    first_far_m: f32,
    far_m: f32,
) -> Result<[CascadeRange; 3], CascadeError> {
    if near_m.partial_cmp(&0.0) != Some(Ordering::Greater) {
        return Err(CascadeError::BadRange {
            near_m,
            far_m,
            reason: "near must be over 0",
        });
    }
    if far_m.partial_cmp(&near_m) != Some(Ordering::Greater) {
        return Err(CascadeError::BadRange {
            near_m,
            far_m,
            reason: "far must be past near",
        });
    }
    if !(first_far_m > near_m && first_far_m < far_m) {
        return Err(CascadeError::BadRange {
            near_m,
            far_m,
            reason: "first far must sit past near and before far",
        });
    }
    Ok(cascade_ranges(near_m, first_far_m, far_m))
}

/// Map edge in texels for one cascade; out-of-range cascades fail loudly.
pub fn cascade_map_size(cascade: usize) -> Result<u32, CascadeError> {
    CASCADE_MAP_SIZES
        .get(cascade)
        .copied()
        .ok_or(CascadeError::BadCascade {
            cascade,
            cascades: SUN_CASCADES,
        })
}

/// Day-sweep probe input: one sun elevation over the arena range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DayProbe {
    /// Sun elevation in degrees; kept as data, never used in tick.
    pub elevation_deg: f32,
    /// Camera near in meters.
    pub near_m: f32,
    /// Camera far in meters.
    pub far_m: f32,
}

/// Day-sweep probe output: overlap-aware intervals plus map sizes,
/// formatted byte-stable.
#[derive(Clone, Debug, PartialEq)]
pub struct DayRow {
    /// Sun elevation in degrees.
    pub elevation_deg: f32,
    /// Far edge per cascade in meters (log build).
    pub splits_m: [f32; 3],
    /// Near edge per cascade in meters; `near_m[i + 1]` sits inside
    /// cascade `i` by `CASCADE_OVERLAP`.
    pub near_m: [f32; 3],
    /// Map edge per cascade in texels.
    pub map_px: [u32; 3],
}

impl DayRow {
    /// Fixed-format row: elevation with 1 decimal, splits and nears with 3.
    #[must_use]
    pub fn to_bytes(&self) -> String {
        format!(
            "elev={:.1} splits={:.3},{:.3},{:.3} nears={:.3},{:.3},{:.3} maps={},{},{}",
            self.elevation_deg,
            self.splits_m[0],
            self.splits_m[1],
            self.splits_m[2],
            self.near_m[0],
            self.near_m[1],
            self.near_m[2],
            self.map_px[0],
            self.map_px[1],
            self.map_px[2]
        )
    }
}

/// Run one day-sweep row: overlap-aware log intervals over
/// `FIRST_CASCADE_FAR_M` plus pinned map sizes.
pub fn day_row(probe: &DayProbe) -> Result<DayRow, CascadeError> {
    let ranges = checked_ranges(probe.near_m, FIRST_CASCADE_FAR_M, probe.far_m)?;
    Ok(DayRow {
        elevation_deg: probe.elevation_deg,
        splits_m: [ranges[0].far_m, ranges[1].far_m, ranges[2].far_m],
        near_m: [ranges[0].near_m, ranges[1].near_m, ranges[2].near_m],
        map_px: [
            cascade_map_size(0)?,
            cascade_map_size(1)?,
            cascade_map_size(2)?,
        ],
    })
}

/// Cascade selection failures; every variant names the bad input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CascadeError {
    /// Camera range is unusable; reason carried.
    BadRange {
        /// Near edge in meters.
        near_m: f32,
        /// Far edge in meters.
        far_m: f32,
        /// Why the range is bad.
        reason: &'static str,
    },
    /// Cascade index is past the pinned table.
    BadCascade {
        /// Asked cascade.
        cascade: usize,
        /// Pinned cascade count.
        cascades: usize,
    },
}

impl fmt::Display for CascadeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRange {
                near_m,
                far_m,
                reason,
            } => write!(
                f,
                "shadow cascade bad range: near {near_m} far {far_m}: {reason}"
            ),
            Self::BadCascade { cascade, cascades } => write!(
                f,
                "shadow cascade bad index: {cascade} of {cascades} cascades"
            ),
        }
    }
}

impl Error for CascadeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shadow_cluster::{SHADOW_MAX, SHADOW_MIN};

    const PROBES: [DayProbe; 3] = [
        DayProbe {
            elevation_deg: 15.0,
            near_m: 0.5,
            far_m: 120.0,
        },
        DayProbe {
            elevation_deg: 45.0,
            near_m: 0.5,
            far_m: 120.0,
        },
        DayProbe {
            elevation_deg: 75.0,
            near_m: 0.5,
            far_m: 120.0,
        },
    ];

    fn sweep_bytes() -> Vec<String> {
        PROBES
            .iter()
            .map(|probe| day_row(probe).expect("probe in range").to_bytes())
            .collect()
    }

    #[test]
    fn splits_pinned_as_data() {
        let splits = checked_splits(0.5, 120.0).expect("range valid");
        assert_eq!(splits.len(), SUN_CASCADES);
        // Splits rise and the last split is the far edge exactly.
        assert!(splits[0] < splits[1] && splits[1] < splits[2]);
        assert_eq!(splits[2], 120.0);
        // Pinned blend values for near 0.5 far 120 lambda 0.5.
        assert!((splits[0] - 21.72).abs() < 0.05, "near split {}", splits[0]);
        assert!((splits[1] - 49.73).abs() < 0.05, "mid split {}", splits[1]);
    }

    #[test]
    fn map_sizes_within_bounds() {
        assert_eq!(CASCADE_MAP_SIZES.len(), SUN_CASCADES);
        for (i, size) in CASCADE_MAP_SIZES.iter().enumerate() {
            let got = cascade_map_size(i).expect("cascade in range");
            assert_eq!(&got, size);
            assert!(got >= SHADOW_MIN && got <= SHADOW_MAX);
        }
        let err = cascade_map_size(3).expect_err("cascade 3 out of range");
        assert_eq!(
            err,
            CascadeError::BadCascade {
                cascade: 3,
                cascades: SUN_CASCADES
            }
        );
        assert!(err.to_string().contains('3'));
    }

    #[test]
    fn day_sweep_byte_identical_x2() {
        let first = sweep_bytes();
        let second = sweep_bytes();
        assert_eq!(first, second);
        assert_eq!(first.len(), 3);
        assert!(first[0].starts_with("elev=15.0"));
        assert!(first[2].starts_with("elev=75.0"));
    }

    #[test]
    fn log_bounds_pinned_as_data() {
        let bounds = log_cascade_bounds(10.0, 120.0);
        assert_eq!(bounds.len(), SUN_CASCADES);
        // Bounds rise and the last bound is the far edge exactly.
        assert!(bounds[0] < bounds[1] && bounds[1] < bounds[2]);
        assert_eq!(bounds[2], 120.0);
        // Geometric spacing from the first far edge: sqrt(120 / 10).
        assert!((bounds[0] - 10.0).abs() < 0.05, "first {}", bounds[0]);
        assert!((bounds[1] - 34.64).abs() < 0.05, "mid {}", bounds[1]);
    }

    #[test]
    fn overlap_three_cascades() {
        let ranges = checked_ranges(0.5, 10.0, 120.0).expect("range valid");
        assert_eq!(ranges.len(), SUN_CASCADES);
        // Nears reuse the previous far scaled by 1 - overlap (0.8).
        assert_eq!(ranges[0].near_m, 0.5);
        assert!(
            (ranges[1].near_m - 8.0).abs() < 0.05,
            "near {}",
            ranges[1].near_m
        );
        assert!(
            (ranges[2].near_m - 27.71).abs() < 0.05,
            "near {}",
            ranges[2].near_m
        );
        // Every later cascade opens inside the previous one: 3 cascades,
        // 2 overlap seams, each interval still runs near to far.
        assert!(
            ranges[1].near_m < ranges[0].far_m,
            "seam 0 {}/{}",
            ranges[1].near_m,
            ranges[0].far_m
        );
        assert!(
            ranges[2].near_m < ranges[1].far_m,
            "seam 1 {}/{}",
            ranges[2].near_m,
            ranges[1].far_m
        );
        for range in &ranges {
            assert!(
                range.near_m < range.far_m,
                "interval {}/{}",
                range.near_m,
                range.far_m
            );
        }
        // Overlap meters stay positive on both seams.
        assert!(
            (ranges[0].far_m - ranges[1].near_m - 2.0).abs() < 0.05,
            "overlap 0 {}",
            ranges[0].far_m - ranges[1].near_m
        );
        assert!(
            (ranges[1].far_m - ranges[2].near_m - 6.93).abs() < 0.05,
            "overlap 1 {}",
            ranges[1].far_m - ranges[2].near_m
        );
    }

    #[test]
    fn bad_first_far_fails_loudly() {
        let err = checked_ranges(0.5, 0.5, 120.0).expect_err("first at near");
        assert!(err.to_string().contains("first far"));
        let err = checked_ranges(0.5, 120.0, 120.0).expect_err("first at far");
        assert!(err.to_string().contains("first far"));
        let err = checked_ranges(0.5, 10.0, 10.0).expect_err("far at first");
        assert!(err.to_string().contains("far"));
        // The day sweep pins the first far at 10 m: a short rig fails loudly.
        let probe = DayProbe {
            elevation_deg: 45.0,
            near_m: 0.5,
            far_m: 5.0,
        };
        let err = day_row(&probe).expect_err("short rig");
        assert!(err.to_string().contains("first far"));
    }

    #[test]
    fn day_row_carries_overlap() {
        let probe = DayProbe {
            elevation_deg: 45.0,
            near_m: 0.5,
            far_m: 120.0,
        };
        let row = day_row(&probe).expect("probe in range");
        assert!(
            (row.splits_m[0] - 10.0).abs() < 0.05,
            "far {}",
            row.splits_m[0]
        );
        assert!((row.near_m[1] - 8.0).abs() < 0.05, "near {}", row.near_m[1]);
        let bytes = row.to_bytes();
        assert!(bytes.contains("nears=0.500,8.000"), "row {bytes}");
    }

    #[test]
    fn bad_range_fails_loudly() {
        let err = checked_splits(0.0, 120.0).expect_err("zero near");
        assert!(err.to_string().contains("near"));
        let err = checked_splits(10.0, 5.0).expect_err("far before near");
        assert!(err.to_string().contains("far"));
    }
}
