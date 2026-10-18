//! SPF6 + SPX5: how far can this box slide, and what is the ground under a point.
//!
//! Consumer: the `scene.query` kinds `slide` (also `travel`) and `ground` (lane A
//! wires them; see `docs/plan/spatial-entry-points.md`). Units: meters in,
//! integer cm inside. Determinism: obstacles ordered by id, ties to the lower
//! id, integer math only. Budget: O(actors) per query.
//!
//! Only the six axis words and the eight horizontal octants are legal, so every
//! swept slab time is an integer and the answer is exact. A diagonal moves `t`
//! cm on each of its two axes (its Euclidean length is `t * sqrt(2)`). Touching
//! counts as clear; the mover, its children and non-physical actors never block.

use super::index::{actor_box, Box3};
use super::Actor;

/// One of the legal slide directions, as an integer step per axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dir(pub [i32; 3]);

impl Dir {
    /// Parse a door word: `+x -x +z -z up down` or an octant such as `+x+z`.
    pub fn parse(word: &str) -> Option<Self> {
        let v = match word {
            "+x" => [1, 0, 0],
            "-x" => [-1, 0, 0],
            "+z" => [0, 0, 1],
            "-z" => [0, 0, -1],
            "up" => [0, 1, 0],
            "down" => [0, -1, 0],
            "+x+z" => [1, 0, 1],
            "+x-z" => [1, 0, -1],
            "-x+z" => [-1, 0, 1],
            "-x-z" => [-1, 0, -1],
            _ => return None,
        };
        Some(Self(v))
    }

    /// Legal words, comma separated, for refusals that name them.
    pub fn words() -> &'static str {
        "+x, -x, +z, -z, up, down, +x+z, +x-z, -x+z, -x-z"
    }
}

/// Result of one slide query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slide {
    /// Centimeters the box can move along the direction before it would overlap.
    pub free_cm: i32,
    /// First obstacle in the way, when the slide stops before `max_range_cm`.
    pub blocker: Option<u32>,
    /// No obstacle inside the range: `free_cm == max_range_cm`.
    pub capped: bool,
    /// The box already overlaps `blocker` before moving: `free_cm` is 0.
    pub already_blocked: bool,
}

/// True when `child` hangs under `root` through its parent chain.
fn under(actors: &[Actor], child: &Actor, root: u32) -> bool {
    let mut cur = child.parent;
    for _ in 0..16 {
        match cur {
            Some(p) if p == root => return true,
            Some(p) => cur = actors.iter().find(|a| a.id == p).and_then(|a| a.parent),
            None => return false,
        }
    }
    false
}

/// Open window `(lo, hi)` of travel times in which the mover's interior
/// overlaps the obstacle's, or `None` when it never does (integer ends).
fn window(m: &Box3, o: &Box3, dir: [i32; 3]) -> Option<(i32, i32)> {
    let mlo = [m.min_x, m.min_y, m.min_z];
    let mhi = [m.max_x, m.max_y, m.max_z];
    let olo = [o.min_x, o.min_y, o.min_z];
    let ohi = [o.max_x, o.max_y, o.max_z];
    let (mut lo, mut hi) = (i32::MIN, i32::MAX);
    for k in 0..3 {
        match dir[k] {
            0 => {
                if !(mlo[k] < ohi[k] && olo[k] < mhi[k]) {
                    return None;
                }
            }
            1 => {
                lo = lo.max(olo[k] - mhi[k]);
                hi = hi.min(ohi[k] - mlo[k]);
            }
            _ => {
                lo = lo.max(mlo[k] - ohi[k]);
                hi = hi.min(mhi[k] - olo[k]);
            }
        }
    }
    (lo < hi).then_some((lo, hi))
}

/// Slide `mover` along `dir` up to `max_range_cm` (1..) against every other
/// physical actor. `None` for an unknown or degenerate mover. Read-only.
pub fn slide(actors: &[Actor], mover: u32, dir: Dir, max_range_cm: i32) -> Option<Slide> {
    let me = actor_box(actors.iter().find(|a| a.id == mover)?)?;
    let range = max_range_cm.max(1);
    let mut best: Option<(i32, u32)> = None;
    let mut already = None;
    let mut ordered: Vec<&Actor> = actors
        .iter()
        .filter(|a| a.physical && a.id != mover)
        .collect();
    ordered.sort_by_key(|a| a.id);
    for other in ordered {
        if under(actors, other, mover) {
            continue;
        }
        let Some(ob) = actor_box(other) else { continue };
        let Some((lo, hi)) = window(&me, &ob, dir.0) else {
            continue;
        };
        if lo < 0 && hi > 0 {
            already = already.or(Some(ob.id));
        } else if lo >= 0 && best.is_none_or(|(t, _)| lo < t) {
            best = Some((lo, ob.id));
        }
    }
    if let Some(id) = already {
        return Some(Slide {
            free_cm: 0,
            blocker: Some(id),
            capped: false,
            already_blocked: true,
        });
    }
    Some(match best {
        Some((t, id)) if t < range => Slide {
            free_cm: t,
            blocker: Some(id),
            capped: false,
            already_blocked: false,
        },
        _ => Slide {
            free_cm: range,
            blocker: None,
            capped: true,
            already_blocked: false,
        },
    })
}

/// The surface under a point: the highest top at or below `from_y_cm` among
/// physical boxes whose plan footprint holds `(x_cm, z_cm)` (edges count).
/// Returns `(top_y_cm, actor id)`, the lower id on equal tops.
pub fn ground_below(actors: &[Actor], x_cm: i32, z_cm: i32, from_y_cm: i32) -> Option<(i32, u32)> {
    let mut best: Option<(i32, u32)> = None;
    let mut ordered: Vec<&Actor> = actors.iter().filter(|a| a.physical).collect();
    ordered.sort_by_key(|a| a.id);
    for a in ordered {
        let Some(b) = actor_box(a) else { continue };
        let holds = b.min_x <= x_cm && x_cm <= b.max_x && b.min_z <= z_cm && z_cm <= b.max_z;
        if holds && b.max_y <= from_y_cm && best.is_none_or(|(y, _)| b.max_y > y) {
            best = Some((b.max_y, b.id));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Rng;

    fn actor(id: u32, lo: [i32; 3], size: [i32; 3]) -> Actor {
        // cm box with even sizes so the half-extent round trip is exact.
        let c = |k: usize| (lo[k] * 2 + size[k]) as f32 / 200.0;
        Actor {
            id,
            name: format!("a{id}"),
            x: c(0),
            y: c(1),
            z: c(2),
            sx: size[0] as f32 / 100.0,
            sy: size[1] as f32 / 100.0,
            sz: size[2] as f32 / 100.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    const DIRS: [&str; 10] = [
        "+x", "-x", "+z", "-z", "up", "down", "+x+z", "+x-z", "-x+z", "-x-z",
    ];

    /// Independent oracle: march in half-cm steps (everything doubled) with a
    /// strict-interior test. Half steps matter: the open window where a diagonal
    /// mover overlaps can be exactly 1 cm wide and hold no whole-cm instant.
    fn march(actors: &[Actor], mover: u32, dir: Dir, range: i32) -> (i32, Option<u32>, bool) {
        let me = actor_box(actors.iter().find(|a| a.id == mover).unwrap()).unwrap();
        let hit_at = |t2: i32| -> Option<u32> {
            let lo = [me.min_x, me.min_y, me.min_z].map(|v| v * 2);
            let hi = [me.max_x, me.max_y, me.max_z].map(|v| v * 2);
            let mut ids: Vec<&Actor> = actors
                .iter()
                .filter(|a| a.physical && a.id != mover)
                .collect();
            ids.sort_by_key(|a| a.id);
            ids.into_iter().find_map(|a| {
                let o = actor_box(a)?;
                let (olo, ohi) = ([o.min_x, o.min_y, o.min_z], [o.max_x, o.max_y, o.max_z]);
                let hit = (0..3).all(|k| {
                    lo[k] + t2 * dir.0[k] < ohi[k] * 2 && olo[k] * 2 < hi[k] + t2 * dir.0[k]
                });
                hit.then_some(o.id)
            })
        };
        if let Some(id) = hit_at(0) {
            return (0, Some(id), true);
        }
        for t2 in 1..=2 * range {
            if let Some(id) = hit_at(t2) {
                return ((t2 - 1) / 2, Some(id), false);
            }
        }
        (range, None, false)
    }

    #[test]
    fn slide_equals_a_one_cm_march_on_200_seeded_box_sets() {
        let mut rng = Rng::new(0x511d);
        let mut stopped = 0;
        let mut blocked_start = 0;
        for case in 0..200 {
            let n = 2 + rng.next_u32() % 6;
            let mut actors: Vec<Actor> = (0..n)
                .map(|i| {
                    let mut r = |m: u32| (rng.next_u32() % m) as i32;
                    let size = [(2 + r(8)) * 2, (2 + r(8)) * 2, (2 + r(8)) * 2];
                    let lo = [r(60) - 10, r(20), r(60) - 10];
                    let mut a = actor(i + 1, lo, size);
                    a.physical = r(7) != 0;
                    a
                })
                .collect();
            let range = 1 + (rng.next_u32() % 90) as i32;
            for word in DIRS {
                let dir = Dir::parse(word).unwrap();
                let got = slide(&actors, 1, dir, range).unwrap();
                let (free, blocker, already) = march(&actors, 1, dir, range);
                assert_eq!(got.free_cm, free, "case {case} {word}");
                assert_eq!(got.already_blocked, already, "case {case} {word}");
                assert_eq!(got.capped, blocker.is_none(), "case {case} {word}");
                if !already {
                    assert_eq!(got.blocker, blocker, "case {case} {word} blocker");
                }
                stopped += usize::from(blocker.is_some());
                blocked_start += usize::from(already);
            }
            // Shuffling the obstacles changes nothing.
            let want = slide(&actors, 1, Dir::parse("+x").unwrap(), range);
            actors.reverse();
            assert_eq!(
                slide(&actors, 1, Dir::parse("+x").unwrap(), range),
                want,
                "case {case} order"
            );
        }
        assert!(
            stopped > 100 && blocked_start > 5,
            "{stopped} {blocked_start}"
        );
    }

    #[test]
    fn touch_is_clear_overlap_is_blocked_and_empty_is_capped() {
        let mover = actor(1, [0, 0, 0], [100, 100, 100]);
        let flush = actor(2, [100, 0, 0], [100, 100, 100]);
        let ahead = actor(3, [250, 0, 0], [100, 100, 100]);
        let east = Dir::parse("+x").unwrap();
        // A face touch at the start is clear: the flush crate stops it at 0 cm
        // only once it tries to move in, so sliding -x is free.
        let back = slide(
            &[mover.clone(), flush.clone()],
            1,
            Dir::parse("-x").unwrap(),
            500,
        )
        .unwrap();
        assert_eq!(
            (back.free_cm, back.capped, back.already_blocked),
            (500, true, false)
        );
        let stop = slide(&[mover.clone(), flush.clone(), ahead.clone()], 1, east, 500).unwrap();
        assert_eq!(
            (stop.free_cm, stop.blocker, stop.already_blocked),
            (0, Some(2), false)
        );
        let far = slide(&[mover.clone(), ahead.clone()], 1, east, 500).unwrap();
        assert_eq!((far.free_cm, far.blocker), (150, Some(3)));
        // Overlapping start: zero and already blocked.
        let inside = actor(4, [50, 0, 0], [100, 100, 100]);
        let s = slide(&[mover.clone(), inside], 1, east, 500).unwrap();
        assert_eq!(
            (s.free_cm, s.already_blocked, s.blocker),
            (0, true, Some(4))
        );
        // Children of the mover are ignored.
        let mut child = actor(5, [100, 0, 0], [100, 100, 100]);
        child.parent = Some(1);
        let c = slide(&[mover.clone(), child], 1, east, 500).unwrap();
        assert!(c.capped);
        assert_eq!(slide(&[mover], 9, east, 500), None);
        assert_eq!(Dir::parse("north"), None);
    }

    #[test]
    fn ground_below_matches_a_one_cm_descent() {
        let mut rng = Rng::new(0x9a0d);
        for case in 0..50 {
            let actors: Vec<Actor> = (0..6)
                .map(|i| {
                    let mut r = |m: u32| (rng.next_u32() % m) as i32;
                    let mut a = actor(
                        i + 1,
                        [r(40), r(30), r(40)],
                        [(1 + r(10)) * 2, (1 + r(10)) * 2, (1 + r(10)) * 2],
                    );
                    a.physical = r(5) != 0;
                    a
                })
                .collect();
            let (x, z, from) = (
                (rng.next_u32() % 50) as i32,
                (rng.next_u32() % 50) as i32,
                40,
            );
            let boxes: Vec<Box3> = actors
                .iter()
                .filter(|a| a.physical)
                .filter_map(actor_box)
                .collect();
            let want = (-1..=from).rev().find_map(|y| {
                boxes
                    .iter()
                    .find(|b| {
                        b.max_y == y && b.min_x <= x && x <= b.max_x && b.min_z <= z && z <= b.max_z
                    })
                    .map(|b| (y, b.id))
            });
            assert_eq!(ground_below(&actors, x, z, from), want, "case {case}");
        }
    }
}
