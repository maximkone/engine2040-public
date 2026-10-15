//! Integer-cm XZ index. Sweep-and-prune broadphase, frozen pair order.
//! All math on i32 cm so builds are bit-identical across targets.

use super::Actor;

pub const CM_PER_M: f32 = 100.0;

/// XZ footprint in integer centimeters.
#[derive(Debug, Clone, Copy)]
pub struct CmRect {
    pub id: u32,
    pub min_x: i32,
    pub max_x: i32,
    pub min_z: i32,
    pub max_z: i32,
    pub area: i64,
}

pub fn to_cm(v: f32) -> i32 {
    (v * CM_PER_M).round() as i32
}

pub fn pos_known(a: &Actor) -> bool {
    a.x.is_finite() && a.y.is_finite() && a.z.is_finite()
}

pub fn size_known(a: &Actor) -> bool {
    a.sx.is_finite() && a.sy.is_finite() && a.sz.is_finite()
}

/// Footprints for actors with known position. Sorted for determinism.
pub fn build_rects(actors: &[Actor]) -> Vec<CmRect> {
    let mut rects: Vec<CmRect> = actors
        .iter()
        .filter(|a| pos_known(a) && size_known(a))
        .map(|a| {
            let hx = i64::from(to_cm(a.sx).abs()) / 2;
            let hz = i64::from(to_cm(a.sz).abs()) / 2;
            let cx = i64::from(to_cm(a.x));
            let cz = i64::from(to_cm(a.z));
            let min_x = cx - hx;
            let max_x = cx + hx;
            let min_z = cz - hz;
            let max_z = cz + hz;
            CmRect {
                id: a.id,
                min_x: clamp_to_i32(min_x),
                max_x: clamp_to_i32(max_x),
                min_z: clamp_to_i32(min_z),
                max_z: clamp_to_i32(max_z),
                area: (max_x - min_x).max(0) * (max_z - min_z).max(0),
            }
        })
        .collect();
    rects.sort_by_key(|r| (r.min_x, r.min_z, r.id));
    rects
}

/// Clamp an i64 cm value into the i32 range (saturating, never wraps).
/// Giant coordinates saturate at the edge so rects keep min <= max.
fn clamp_to_i32(v: i64) -> i32 {
    i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX })
}

/// Sweep broadphase: pairs (lo_id, hi_id) with lo_id < hi_id, sorted.
/// Zero-area rects never pair (touching edges are not overlap).
pub fn overlap_pairs(rects: &[CmRect]) -> Vec<(u32, u32, i64)> {
    let mut active: Vec<usize> = Vec::new();
    let mut pairs: Vec<(u32, u32, i64)> = Vec::new();
    for (i, r) in rects.iter().enumerate() {
        active.retain(|&j| rects[j].max_x > r.min_x);
        for &j in &active {
            let o = &rects[j];
            let ix = i64::from(r.max_x.min(o.max_x)) - i64::from(r.min_x.max(o.min_x));
            if ix <= 0 {
                continue;
            }
            let iz = i64::from(r.max_z.min(o.max_z)) - i64::from(r.min_z.max(o.min_z));
            if iz <= 0 {
                continue;
            }
            let inter = ix * iz;
            if inter <= 0 {
                continue;
            }
            let (lo, hi) = if o.id < r.id {
                (o.id, r.id)
            } else {
                (r.id, o.id)
            };
            pairs.push((lo, hi, inter));
        }
        active.push(i);
    }
    pairs.sort_unstable();
    pairs
}

/// Overlap percent of the smaller rect, rounded to 0.1.
pub fn overlap_pct(inter: i64, area_a: i64, area_b: i64) -> f32 {
    let m = area_a.min(area_b).max(1) as f64;
    ((inter as f64 / m * 1000.0).round() / 10.0) as f32
}

/// 3D integer-cm volume. rstar-shape (bbox volumes), std-only reimplementation.
#[derive(Debug, Clone, Copy)]
pub struct Box3 {
    pub id: u32,
    pub min_x: i32,
    pub max_x: i32,
    pub min_y: i32,
    pub max_y: i32,
    pub min_z: i32,
    pub max_z: i32,
}

pub const XYZ_CELL_CM: i32 = 100;

/// Volumes for actors with known position and size. Sorted for determinism.
pub fn boxes_of(actors: &[Actor]) -> Vec<Box3> {
    let mut out: Vec<Box3> = actors
        .iter()
        .filter(|a| pos_known(a) && size_known(a))
        .map(|a| {
            let hx = to_cm(a.sx).abs() / 2;
            let hy = to_cm(a.sy).abs() / 2;
            let hz = to_cm(a.sz).abs() / 2;
            let (cx, cy, cz) = (to_cm(a.x), to_cm(a.y), to_cm(a.z));
            Box3 {
                id: a.id,
                min_x: cx - hx,
                max_x: cx + hx,
                min_y: cy - hy,
                max_y: cy + hy,
                min_z: cz - hz,
                max_z: cz + hz,
            }
        })
        .collect();
    out.sort_by_key(|b| (b.min_x, b.min_y, b.min_z, b.id));
    out
}

/// Strict volume overlap on integer centimeters. Touching faces never hit;
/// the one overlap test every XYZ pair path shares.
pub(crate) fn xyz_hit(a: &Box3, b: &Box3) -> bool {
    a.min_x < b.max_x
        && b.min_x < a.max_x
        && a.min_y < b.max_y
        && b.min_y < a.max_y
        && a.min_z < b.max_z
        && b.min_z < a.max_z
}

/// Uniform-grid XYZ broadphase: strict volume pairs, sorted + deduped.
/// Catches stacked volumes the XZ sweep misses; XZ sweep stays the 2D path.
pub fn xyz_pairs(actors: &[Actor]) -> Vec<(u32, u32)> {
    let boxes = boxes_of(actors);
    let mut cells: std::collections::BTreeMap<(i32, i32, i32), Vec<usize>> =
        std::collections::BTreeMap::new();
    for (i, b) in boxes.iter().enumerate() {
        let x0 = b.min_x.div_euclid(XYZ_CELL_CM);
        let x1 = (b.max_x - 1).div_euclid(XYZ_CELL_CM);
        let y0 = b.min_y.div_euclid(XYZ_CELL_CM);
        let y1 = (b.max_y - 1).div_euclid(XYZ_CELL_CM);
        let z0 = b.min_z.div_euclid(XYZ_CELL_CM);
        let z1 = (b.max_z - 1).div_euclid(XYZ_CELL_CM);
        // Clamp span so degenerate giants can't blow the grid.
        for cx in x0..=x1.min(x0 + 64) {
            for cy in y0..=y1.min(y0 + 64) {
                for cz in z0..=z1.min(z0 + 64) {
                    cells.entry((cx, cy, cz)).or_default().push(i);
                }
            }
        }
    }
    let mut set: std::collections::BTreeSet<(u32, u32)> = std::collections::BTreeSet::new();
    for ids in cells.values() {
        for (n, &i) in ids.iter().enumerate() {
            for &j in &ids[n + 1..] {
                let (a, b) = (&boxes[i], &boxes[j]);
                if xyz_hit(a, b) {
                    let (lo, hi) = if a.id < b.id {
                        (a.id, b.id)
                    } else {
                        (b.id, a.id)
                    };
                    set.insert((lo, hi));
                }
            }
        }
    }
    set.into_iter().collect()
}

/// Swept footprint: rect expanded along (vx_cm, vz_cm) over `horizon` ticks.
/// Static actors (zero velocity) return their own rect.
pub fn swept_rect(r: &CmRect, vx_cm: i32, vz_cm: i32) -> CmRect {
    let (min_x, max_x) = if vx_cm >= 0 {
        (r.min_x, r.max_x + vx_cm)
    } else {
        (r.min_x + vx_cm, r.max_x)
    };
    let (min_z, max_z) = if vz_cm >= 0 {
        (r.min_z, r.max_z + vz_cm)
    } else {
        (r.min_z + vz_cm, r.max_z)
    };
    CmRect {
        id: r.id,
        min_x,
        max_x,
        min_z,
        max_z,
        area: i64::from((max_x - min_x).max(0)) * i64::from((max_z - min_z).max(0)),
    }
}

/// Pairs whose swept rects overlap but static rects do not: will-collide.
/// Returns (lo_id, hi_id, swept_inter). Sorted, deterministic.
pub fn sweep_pairs(actors: &[Actor], horizon_ticks: i32) -> Vec<(u32, u32, i64)> {
    let rects = build_rects(actors);
    let vel_of = |id: u32| -> (i32, i32) {
        actors
            .iter()
            .find(|a| a.id == id)
            .map(|a| {
                if a.vx.is_finite() && a.vz.is_finite() {
                    (
                        (i64::from(to_cm(a.vx)) * i64::from(horizon_ticks) / 60) as i32,
                        (i64::from(to_cm(a.vz)) * i64::from(horizon_ticks) / 60) as i32,
                    )
                } else {
                    (0, 0)
                }
            })
            .unwrap_or((0, 0))
    };
    let swept: Vec<CmRect> = rects
        .iter()
        .map(|r| {
            let (vx, vz) = vel_of(r.id);
            swept_rect(r, vx, vz)
        })
        .collect();
    let mut swept_sorted = swept.clone();
    swept_sorted.sort_by_key(|r| (r.min_x, r.min_z, r.id));
    let static_pairs: std::collections::BTreeSet<(u32, u32)> = overlap_pairs(&rects)
        .into_iter()
        .map(|(a, b, _)| (a, b))
        .collect();
    let area_of: std::collections::BTreeMap<u32, i64> =
        swept.iter().map(|r| (r.id, r.area)).collect();
    let mut out: Vec<(u32, u32, i64)> = Vec::new();
    for (lo, hi, inter) in overlap_pairs(&swept_sorted) {
        if static_pairs.contains(&(lo, hi)) {
            continue;
        }
        let _ = area_of.get(&lo);
        out.push((lo, hi, inter));
    }
    out.sort_unstable();
    out
}

/// k nearest actor ids to `id` by XZ center distance. Ties break by id.
/// Actors with unknown position are skipped. Order-independent.
pub fn nearest(actors: &[Actor], id: u32, k: usize) -> Vec<u32> {
    let center = |a: &Actor| -> Option<(i64, i64)> {
        if pos_known(a) {
            Some((i64::from(to_cm(a.x)), i64::from(to_cm(a.z))))
        } else {
            None
        }
    };
    let origin = match actors.iter().find(|a| a.id == id).and_then(center) {
        Some(c) => c,
        None => return Vec::new(),
    };
    let mut ds: Vec<(i64, u32)> = actors
        .iter()
        .filter(|a| a.id != id)
        .filter_map(|a| {
            center(a).map(|(x, z)| {
                let dx = x - origin.0;
                let dz = z - origin.1;
                (dx * dx + dz * dz, a.id)
            })
        })
        .collect();
    ds.sort_unstable();
    ds.into_iter().take(k).map(|(_, i)| i).collect()
}

/// Crowd-density grid: cell -> sorted actor ids whose center falls inside.
/// Cell size in cm. BTreeMap keeps cell order frozen.
pub fn density_grid(
    actors: &[Actor],
    cell_cm: i32,
) -> std::collections::BTreeMap<(i32, i32), Vec<u32>> {
    let mut grid: std::collections::BTreeMap<(i32, i32), Vec<u32>> =
        std::collections::BTreeMap::new();
    for a in actors {
        if !pos_known(a) {
            continue;
        }
        let cell = (
            to_cm(a.x).div_euclid(cell_cm.max(1)),
            to_cm(a.z).div_euclid(cell_cm.max(1)),
        );
        grid.entry(cell).or_default().push(a.id);
    }
    for ids in grid.values_mut() {
        ids.sort_unstable();
    }
    grid
}

#[cfg(test)]
mod index_tests {
    use super::{build_rects, overlap_pairs};
    use crate::Actor;
    fn actor(id: u32, x: f32) -> Actor {
        Actor {
            id,
            name: String::new(),
            x,
            y: 0.0,
            z: 0.0,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }
    #[test]
    fn overlap_pairs_pins_two_box_overlap() {
        let actors = vec![actor(1, 0.0), actor(2, 0.5), actor(3, 1.5)];
        let rects = build_rects(&actors);
        assert_eq!(
            rects.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(
            (rects[0].min_x, rects[0].max_x, rects[0].area),
            (-50, 50, 10000)
        );
        assert_eq!(
            (rects[1].min_x, rects[1].max_x, rects[1].area),
            (0, 100, 10000)
        );
        assert_eq!(overlap_pairs(&rects), vec![(1, 2, 5000)]);
    }
    #[test]
    fn build_rects_clamps_giant_coords_to_i32_range() {
        let mut wide = actor(1, 25_000_000.0);
        wide.sx = 10.0;
        wide.sz = 10.0;
        let rects = build_rects(&[wide]);
        assert_eq!(rects.len(), 1);
        assert!(
            rects[0].min_x <= rects[0].max_x,
            "giant rect wrapped: min {} max {}",
            rects[0].min_x,
            rects[0].max_x
        );
        assert_eq!(rects[0].max_x, i32::MAX);
    }
}
