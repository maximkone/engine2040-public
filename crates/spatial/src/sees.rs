//! SPR7: pair sight with the occluder.
//!
//! Consumer: the `scene.query` kind `sees` and the `visible_from` relation (see
//! `docs/plan/spatial-entry-points.md`); `why_hidden.rs` builds on it. Units: meters in, integer
//! cm inside. Determinism: id-sorted solids, integer slab math from
//! `sight.rs`, no float anywhere, so the same actors give the same bytes.
//! Budget: five rays over O(solids), no allocation per ray.
//!
//! One definition of blocking: [`first_blocker`] uses the same strict-interior
//! segment test as `sight::los_actors`, so "blocked" and "has a blocker" are
//! the same statement.

use super::index::Box3;
use super::intmath::isqrt;
use super::sight::{contains, mul_div_round, ray_entry, seg_hits, SightError, SightWorld};

/// The solid a sight ray runs into first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blocker {
    /// Actor id of the occluder.
    pub id: u32,
    /// Where the ray enters it, integer cm.
    pub entry_cm: [i32; 3],
    /// Distance from the eye to that entry, floored, in cm.
    pub dist_cm: i32,
}

/// First solid strictly between `e` and `t` (cm), or `None` when the way is
/// clear. Boxes holding either endpoint never block (own body, room shell);
/// non-physical actors never block. Nearest entry wins, ties go to the lower id.
pub fn first_blocker(world: &SightWorld, e: [i32; 3], t: [i32; 3]) -> Option<Blocker> {
    let d = [
        i64::from(t[0]) - i64::from(e[0]),
        i64::from(t[1]) - i64::from(e[1]),
        i64::from(t[2]) - i64::from(e[2]),
    ];
    let len_sq = d.iter().map(|v| v * v).sum::<i64>();
    let mut best: Option<(i64, i64, u32)> = None;
    for b in world.solids() {
        if contains(b, e) || contains(b, t) || !seg_hits(*b, e, d) {
            continue;
        }
        let (n, den) = ray_entry(b, e, d).unwrap_or((0, 1));
        if best.is_none_or(|(bn, bd, _)| n * bd < bn * den) {
            best = Some((n, den, b.id));
        }
    }
    let (n, den, id) = best?;
    let dist2 =
        i128::from(len_sq) * i128::from(n) * i128::from(n) / (i128::from(den) * i128::from(den));
    let at = |k: usize| e[k] + i32::try_from(mul_div_round(d[k], n, den)).unwrap_or(0);
    Some(Blocker {
        id,
        entry_cm: [at(0), at(1), at(2)],
        dist_cm: i32::try_from(isqrt(i64::try_from(dist2).unwrap_or(i64::MAX))).unwrap_or(i32::MAX),
    })
}

/// Rays in the fan: the center plus four mid-height corners.
pub const FAN_RAYS: u32 = 5;

/// Pair sight result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sees {
    /// The center-to-center ray is clear (the `los_actors` answer).
    pub visible: bool,
    /// Rays of the fan that reach the target ([`FAN_RAYS`] at most).
    pub clear_rays: u32,
    /// Blocker of the first blocked ray, center ray first; `None` when all clear.
    pub blocker: Option<Blocker>,
}

impl Sees {
    /// Share of the fan that reaches the target, 0..=100 in steps of 20.
    pub fn fraction_pct(&self) -> u32 {
        self.clear_rays * 100 / FAN_RAYS
    }
}

pub(crate) fn mid(b: &Box3) -> [i32; 3] {
    [
        (b.min_x + b.max_x) / 2,
        (b.min_y + b.max_y) / 2,
        (b.min_z + b.max_z) / 2,
    ]
}

/// Can `from` see `to`? Eye is the center of `from`; the fan aims at the center
/// of `to` and at its four vertical edges at mid height, one cm inside so a
/// ray that ends on a face still counts the face as the target, not a blocker.
/// Unknown or degenerate ids refuse with [`SightError::UnknownActor`].
pub fn sees_pair(world: &SightWorld, from: u32, to: u32) -> Result<Sees, SightError> {
    let a = world.box_of(from).ok_or(SightError::UnknownActor(from))?;
    let b = world.box_of(to).ok_or(SightError::UnknownActor(to))?;
    let eye = mid(a);
    let c = mid(b);
    let inx = 1.min((b.max_x - b.min_x) / 2);
    let inz = 1.min((b.max_z - b.min_z) / 2);
    let targets = [
        c,
        [b.min_x + inx, c[1], b.min_z + inz],
        [b.max_x - inx, c[1], b.min_z + inz],
        [b.min_x + inx, c[1], b.max_z - inz],
        [b.max_x - inx, c[1], b.max_z - inz],
    ];
    let mut clear = 0;
    let mut blocker = None;
    let mut center_clear = false;
    for (i, t) in targets.iter().enumerate() {
        match first_blocker(world, eye, *t) {
            None => {
                clear += 1;
                center_clear |= i == 0;
            }
            Some(hit) => blocker = blocker.or(Some(hit)),
        }
    }
    Ok(Sees {
        visible: center_clear,
        clear_rays: clear,
        blocker,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Actor;
    use engine_core::Rng;

    fn solid(id: u32, x: f32, y: f32, z: f32, sx: f32, sy: f32, sz: f32) -> Actor {
        Actor {
            id,
            name: format!("a{id}"),
            x,
            y,
            z,
            sx,
            sy,
            sz,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw: 0.0,
        }
    }

    /// Independent oracle: Liang-Barsky in f64 over the same cm integers.
    /// Division of small integers is correctly rounded, so equal rationals tie
    /// exactly and the strict-interior rule matches without shared code.
    fn oracle_first(boxes: &[Box3], e: [i32; 3], t: [i32; 3]) -> Option<(u32, f64)> {
        let d: Vec<f64> = (0..3).map(|k| f64::from(t[k] - e[k])).collect();
        let mut best: Option<(u32, f64)> = None;
        for b in boxes {
            let inside = |p: [i32; 3]| {
                b.min_x <= p[0]
                    && p[0] <= b.max_x
                    && b.min_y <= p[1]
                    && p[1] <= b.max_y
                    && b.min_z <= p[2]
                    && p[2] <= b.max_z
            };
            if inside(e) || inside(t) {
                continue;
            }
            let (mut lo, mut hi) = (0.0f64, 1.0f64);
            let mut hit = true;
            for (k, (mn, mx)) in [(b.min_x, b.max_x), (b.min_y, b.max_y), (b.min_z, b.max_z)]
                .into_iter()
                .enumerate()
            {
                if d[k] == 0.0 {
                    if e[k] < mn || e[k] > mx {
                        hit = false;
                    }
                    continue;
                }
                let (a, c) = (f64::from(mn - e[k]) / d[k], f64::from(mx - e[k]) / d[k]);
                lo = lo.max(a.min(c));
                hi = hi.min(a.max(c));
            }
            if hit && lo < hi && best.is_none_or(|(_, s)| lo < s) {
                best = Some((b.id, lo));
            }
        }
        best
    }

    fn scene(rng: &mut Rng) -> Vec<Actor> {
        (1..=8u32)
            .map(|i| {
                let f = |r: &mut Rng, span: u32| (r.next_u32() % span) as f32 / 10.0;
                let mut a = solid(
                    i,
                    f(rng, 200) - 10.0,
                    f(rng, 30) + 0.5,
                    f(rng, 200) - 10.0,
                    f(rng, 40) + 0.5,
                    f(rng, 30) + 0.5,
                    f(rng, 40) + 0.5,
                );
                a.physical = !rng.next_u32().is_multiple_of(6);
                a
            })
            .collect()
    }

    #[test]
    fn sees_pair_matches_the_oracle_on_50_seeded_pairs() {
        let mut rng = Rng::new(0x5ee5);
        let mut blocked_seen = 0;
        for case in 0..50 {
            let actors = scene(&mut rng);
            let world = SightWorld::build(&actors);
            let (from, to) = (1 + case % 4, 5 + case % 4);
            let (a, b) = (world.box_of(from).unwrap(), world.box_of(to).unwrap());
            let boxes: Vec<Box3> = world.solids().to_vec();
            let got = sees_pair(&world, from, to).unwrap();
            let c = mid(b);
            let want_center = oracle_first(&boxes, mid(a), c);
            assert_eq!(got.visible, want_center.is_none(), "case {case} center");
            let (inx, inz) = (
                1.min((b.max_x - b.min_x) / 2),
                1.min((b.max_z - b.min_z) / 2),
            );
            let clear = [
                c,
                [b.min_x + inx, c[1], b.min_z + inz],
                [b.max_x - inx, c[1], b.min_z + inz],
                [b.min_x + inx, c[1], b.max_z - inz],
                [b.max_x - inx, c[1], b.max_z - inz],
            ]
            .iter()
            .filter(|t| oracle_first(&boxes, mid(a), **t).is_none())
            .count();
            assert_eq!(got.clear_rays as usize, clear, "case {case} fan");
            if let (Some(hit), Some((id, _))) = (got.blocker, oracle_first(&boxes, mid(a), c)) {
                assert_eq!(hit.id, id, "case {case} occluder");
                blocked_seen += 1;
            }
            assert_eq!(got, sees_pair(&world, from, to).unwrap(), "replay x2");
        }
        assert!(blocked_seen > 3, "seeds must hit blockers: {blocked_seen}");
    }

    #[test]
    fn sees_pair_names_the_pillar_and_half_hidden_reads_strictly_between() {
        let actors = vec![
            solid(1, 0.0, 1.0, 0.0, 1.0, 2.0, 1.0),
            solid(2, 10.0, 1.0, 0.0, 1.0, 2.0, 2.0),
            solid(3, 5.0, 1.0, 1.5, 1.0, 2.0, 3.4),
        ];
        let world = SightWorld::build(&actors);
        let seen = sees_pair(&world, 1, 2).unwrap();
        let pillar = seen.blocker.expect("pillar blocks");
        assert_eq!(
            (pillar.id, pillar.entry_cm[0], pillar.dist_cm),
            (3, 450, 450)
        );
        assert!(!seen.visible);
        assert!(
            seen.fraction_pct() > 0 && seen.fraction_pct() < 100,
            "{}",
            seen.fraction_pct()
        );
        // Without the pillar the whole fan arrives.
        let open = SightWorld::build(&actors[..2]);
        let clear = sees_pair(&open, 1, 2).unwrap();
        assert!(clear.visible && clear.fraction_pct() == 100 && clear.blocker.is_none());
        assert_eq!(sees_pair(&world, 1, 99), Err(SightError::UnknownActor(99)));
    }

    #[test]
    fn relation_visible_from_and_sees_never_disagree() {
        use crate::relation::{holds, Frame, Rel};
        let mut rng = Rng::new(0xd15a);
        let mut yes = 0;
        for case in 0..50 {
            let actors = scene(&mut rng);
            let world = SightWorld::build(&actors);
            for (from, to) in [(1, 5), (2, 6), (3, 7), (4, 8)] {
                let (a, b) = (world.box_of(from).unwrap(), world.box_of(to).unwrap());
                let same_story = a.min_y < b.max_y && b.min_y < a.max_y;
                let want = same_story && sees_pair(&world, from, to).unwrap().visible;
                let got = holds(&actors, from, Rel::Visible, to, Frame::World);
                assert_eq!(got, want, "case {case} {from}->{to}");
                yes += usize::from(got);
            }
        }
        assert!(yes > 10, "the seeds must see something: {yes}");
    }

    #[test]
    #[ignore = "timing: run under --perf in release"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn sees_pair_budget_under_a_fifth_of_a_ms() {
        let mut rng = Rng::new(0x5ee6);
        let world = SightWorld::build(&scene(&mut rng));
        let pairs = 2_000u32;
        let start = std::time::Instant::now();
        let mut seen = 0u32;
        for n in 0..pairs {
            let (from, to) = (1 + n % 4, 5 + n % 4);
            seen += u32::from(sees_pair(&world, from, to).unwrap().visible);
        }
        let per_pair_us = start.elapsed().as_secs_f64() / f64::from(pairs) * 1e6;
        println!("sees_pair: {per_pair_us:.3} us per pair ({seen} visible of {pairs})");
        assert!(
            per_pair_us <= 200.0,
            "pair budget 0.2 ms, got {per_pair_us:.3} us"
        );
    }
}
