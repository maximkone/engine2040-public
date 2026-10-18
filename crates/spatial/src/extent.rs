//! SPX8: a stray-robust scene extent.
//!
//! Consumer: the scene brief and the `stray` family of checks (lane A wires
//! them; see `docs/plan/spatial-entry-points.md`). One actor at -1,000,000 cm
//! made a Flax scene read 44x too big; this finds the strays and states the
//! extent without them. Units: meters in, integer cm out. Determinism: integer
//! medians (lower median), id-ordered output.
//!
//! Rules: with fewer than 4 boxes nothing is excluded. A box is a *stray* when
//! its center is more than 20x the median center distance from the median
//! center (floor 100 cm), and a *backdrop* when its diagonal is more than 50x
//! the median diagonal. If half the scene or more would be excluded, nothing
//! is: that is the layout, not a stray.

use super::index::{pos_known, size_known, to_cm};
use super::intmath::isqrt;
use super::Actor;

/// Stray factor: center distance over the median distance.
const STRAY_X: i64 = 20;
/// Backdrop factor: diagonal over the median diagonal.
const BACKDROP_X: i64 = 50;
/// Floor for the median distance in cm, so a tight cluster does not flag
/// ordinary spread.
const MIN_MEDIAN_CM: i64 = 100;

/// One excluded box and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Excluded {
    pub id: u32,
    /// `stray` or `backdrop`.
    pub why: &'static str,
    /// Distance (stray) or diagonal (backdrop) over its median, whole times.
    pub ratio: i64,
}

/// Extent of the scene without its strays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extent {
    pub min_cm: [i32; 3],
    pub max_cm: [i32; 3],
    /// Excluded boxes, id ascending.
    pub excluded: Vec<Excluded>,
    /// Diagonal of the full extent over the robust one, in tenths (10 = same).
    pub inflation_x10: i64,
}

struct Box3c {
    id: u32,
    lo: [i32; 3],
    hi: [i32; 3],
}

fn median(sorted: &[i64]) -> i64 {
    sorted[(sorted.len() - 1) / 2]
}

fn diag(lo: [i32; 3], hi: [i32; 3]) -> i64 {
    let sq: i64 = (0..3).map(|k| i64::from(hi[k] - lo[k]).pow(2)).sum();
    isqrt(sq)
}

/// Robust extent over valid boxes, or `None` for an empty scene.
pub fn robust_extent(actors: &[Actor]) -> Option<Extent> {
    let mut boxes: Vec<Box3c> = actors
        .iter()
        .filter(|a| pos_known(a) && size_known(a) && a.sx > 0.0 && a.sy > 0.0 && a.sz > 0.0)
        .map(|a| {
            let half = [
                to_cm(a.sx).abs() / 2,
                to_cm(a.sy).abs() / 2,
                to_cm(a.sz).abs() / 2,
            ];
            let c = [to_cm(a.x), to_cm(a.y), to_cm(a.z)];
            Box3c {
                id: a.id,
                lo: [c[0] - half[0], c[1] - half[1], c[2] - half[2]],
                hi: [c[0] + half[0], c[1] + half[1], c[2] + half[2]],
            }
        })
        .collect();
    if boxes.is_empty() {
        return None;
    }
    boxes.sort_by_key(|b| b.id);
    let center = |b: &Box3c| [0, 1, 2].map(|k| i64::from(b.lo[k] + b.hi[k]) / 2);
    let mut excluded = Vec::new();
    if boxes.len() >= 4 {
        let axis = |k: usize| {
            let mut v: Vec<i64> = boxes.iter().map(|b| center(b)[k]).collect();
            v.sort_unstable();
            median(&v)
        };
        let mid = [axis(0), axis(1), axis(2)];
        let dist = |b: &Box3c| {
            let c = center(b);
            isqrt((0..3).map(|k| (c[k] - mid[k]).pow(2)).sum())
        };
        let mut dists: Vec<i64> = boxes.iter().map(dist).collect();
        dists.sort_unstable();
        let med_d = median(&dists).max(MIN_MEDIAN_CM);
        let mut diags: Vec<i64> = boxes.iter().map(|b| diag(b.lo, b.hi)).collect();
        diags.sort_unstable();
        let med_diag = median(&diags).max(1);
        for b in &boxes {
            if dist(b) > STRAY_X * med_d {
                excluded.push(Excluded {
                    id: b.id,
                    why: "stray",
                    ratio: dist(b) / med_d,
                });
            } else if diag(b.lo, b.hi) > BACKDROP_X * med_diag {
                excluded.push(Excluded {
                    id: b.id,
                    why: "backdrop",
                    ratio: diag(b.lo, b.hi) / med_diag,
                });
            }
        }
        if excluded.len() * 2 >= boxes.len() {
            excluded.clear();
        }
    }
    let hull = |set: &[&Box3c]| -> ([i32; 3], [i32; 3]) {
        (
            [0, 1, 2].map(|k| set.iter().map(|b| b.lo[k]).min().unwrap_or(0)),
            [0, 1, 2].map(|k| set.iter().map(|b| b.hi[k]).max().unwrap_or(0)),
        )
    };
    let kept: Vec<&Box3c> = boxes
        .iter()
        .filter(|b| !excluded.iter().any(|e| e.id == b.id))
        .collect();
    let (min_cm, max_cm) = hull(&kept);
    let all: Vec<&Box3c> = boxes.iter().collect();
    let (full_lo, full_hi) = hull(&all);
    let inflation_x10 = diag(full_lo, full_hi) * 10 / diag(min_cm, max_cm).max(1);
    Some(Extent {
        min_cm,
        max_cm,
        excluded,
        inflation_x10,
    })
}

#[cfg(test)]
mod stray_extent {
    use super::*;
    use engine_core::Rng;

    fn prop(id: u32, x: i32, y: i32, z: i32, size: i32) -> Actor {
        let m = |v: i32| v as f32 / 100.0;
        Actor {
            id,
            name: format!("p{id}"),
            x: m(x),
            y: m(y),
            z: m(z),
            sx: m(size),
            sy: m(size),
            sz: m(size),
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    /// Oracle: bounding box of the given props, straight min/max.
    fn bbox(props: &[Actor]) -> ([i32; 3], [i32; 3]) {
        let mut lo = [i32::MAX; 3];
        let mut hi = [i32::MIN; 3];
        for a in props {
            let c = [to_cm(a.x), to_cm(a.y), to_cm(a.z)];
            let h = to_cm(a.sx) / 2;
            for k in 0..3 {
                lo[k] = lo[k].min(c[k] - h);
                hi[k] = hi[k].max(c[k] + h);
            }
        }
        (lo, hi)
    }

    fn scatter(rng: &mut Rng, n: u32, span: i32) -> Vec<Actor> {
        (1..=n)
            .map(|i| {
                let mut r = |m: i32| (rng.next_u32() % m as u32) as i32;
                prop(i, r(span) - span / 2, 50, r(span) - span / 2, 100)
            })
            .collect()
    }

    #[test]
    fn sixty_props_and_one_far_away_keep_the_extent_and_list_one_stray() {
        let mut rng = Rng::new(0x5712a);
        let mut props = scatter(&mut rng, 60, 1000);
        let (lo, hi) = bbox(&props);
        props.push(prop(99, 1_000_000, 50, 0, 100));
        let e = robust_extent(&props).unwrap();
        assert_eq!((e.min_cm, e.max_cm), (lo, hi));
        assert_eq!(e.excluded.len(), 1);
        assert_eq!((e.excluded[0].id, e.excluded[0].why), (99, "stray"));
        assert!(e.inflation_x10 > 1000, "inflation {}", e.inflation_x10);
        // Three boxes: silent, even with one far away.
        let three = vec![
            prop(1, 0, 50, 0, 100),
            prop(2, 200, 50, 0, 100),
            prop(3, 1_000_000, 50, 0, 100),
        ];
        assert!(robust_extent(&three).unwrap().excluded.is_empty());
        // Half the scene far away flags none.
        let mut halves = scatter(&mut rng, 6, 500);
        halves.extend((7..=12).map(|i| prop(i, 500_000 + i as i32 * 100, 50, 0, 100)));
        assert!(robust_extent(&halves).unwrap().excluded.is_empty());
        assert_eq!(robust_extent(&[]), None);
    }

    #[test]
    fn a_big_backdrop_is_excluded_too() {
        let mut rng = Rng::new(0xbd);
        let mut scene = scatter(&mut rng, 30, 800);
        scene.push(prop(77, 0, 0, 0, 60_000));
        let e = robust_extent(&scene).unwrap();
        assert_eq!((e.excluded[0].id, e.excluded[0].why), (77, "backdrop"));
    }

    #[test]
    fn seeded_scenes_match_the_props_only_box_and_ignore_order() {
        let mut rng = Rng::new(0x57a7);
        for case in 0..100 {
            let n = 5 + rng.next_u32() % 40;
            let span = 400 + (rng.next_u32() % 1500) as i32;
            let mut scene = scatter(&mut rng, n, span);
            let (lo, hi) = bbox(&scene);
            let strays = rng.next_u32() % 3;
            for s in 0..strays {
                let far = 200_000 + (rng.next_u32() % 100_000) as i32;
                scene.push(prop(
                    500 + s,
                    if s % 2 == 0 { far } else { -far },
                    50,
                    far / 2,
                    100,
                ));
            }
            let want = robust_extent(&scene).unwrap();
            assert_eq!((want.min_cm, want.max_cm), (lo, hi), "case {case}");
            assert_eq!(want.excluded.len(), strays as usize, "case {case}");
            scene.reverse();
            assert_eq!(robust_extent(&scene).unwrap(), want, "case {case} order");
        }
    }
}
