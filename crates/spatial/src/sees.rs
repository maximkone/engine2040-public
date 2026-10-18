//! SPR7 + SPX6: pair sight with the occluder, and the reason a thing is hidden.
//!
//! Consumer: the `scene.query` kinds `sees` and `why_hidden` (lane A wires
//! them; see `docs/plan/spatial-entry-points.md`). Units: meters in, integer
//! cm inside. Determinism: id-sorted solids, integer slab math from
//! `sight.rs`, no float anywhere, so the same actors give the same bytes.
//! Budget: five rays over O(solids), no allocation per ray.
//!
//! One definition of blocking: [`first_blocker`] uses the same strict-interior
//! segment test as `sight::los_actors`, so "blocked" and "has a blocker" are
//! the same statement.

use super::index::{to_cm, Box3};
use super::intmath::{angle_deg, isqrt};
use super::navcheck::octant;
use super::relation::DIR_RANGE_CM;
use super::sight::{
    contains, mul_div_round, ray_entry, seg_hits, SightError, SightWorld, VIEW_DY_CM,
};
use super::Actor;

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

fn mid(b: &Box3) -> [i32; 3] {
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

/// What the caller knows about the target that geometry cannot say: whether
/// it has something to draw. `Default` assumes both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Draw {
    pub has_mesh: bool,
    pub has_material: bool,
}

impl Default for Draw {
    fn default() -> Self {
        Self {
            has_mesh: true,
            has_material: true,
        }
    }
}

/// First failing reason a target is not seen, in the fixed order of the enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hidden {
    Visible,
    NotFound,
    NoMesh,
    NoMaterial,
    ZeroScale,
    DegenerateBounds,
    BehindEye,
    TooFar,
    OutOfView,
    OccludedBy(u32),
}

impl Hidden {
    /// Door word: `occluded_by:<id>` carries the occluder, the rest are fixed.
    pub fn word(self) -> String {
        match self {
            Self::Visible => "visible".to_owned(),
            Self::NotFound => "not_found".to_owned(),
            Self::NoMesh => "no_mesh".to_owned(),
            Self::NoMaterial => "no_material".to_owned(),
            Self::ZeroScale => "zero_scale".to_owned(),
            Self::DegenerateBounds => "degenerate_bounds".to_owned(),
            Self::BehindEye => "behind_eye".to_owned(),
            Self::TooFar => "too_far".to_owned(),
            Self::OutOfView => "out_of_view".to_owned(),
            Self::OccludedBy(id) => format!("occluded_by:{id}"),
        }
    }
}

/// A verdict plus the one number that explains it: smallest side for
/// `zero_scale`, cm along the facing for `behind_eye`, distance for
/// `too_far`, degrees off the facing (or cm of height) for `out_of_view`, cm to
/// the occluder for `occluded_by`, 0 otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Why {
    pub verdict: Hidden,
    pub evidence: i64,
}

/// Why is `target` not seen from `eye_m` facing `yaw`? Checks in a fixed order
/// and stops at the first failure; the view rules are those of
/// `sight::visible` (90-degree cone, 20 m horizon, 5 m height window) and the
/// occluder comes from the same ray as [`sees_pair`]'s center ray.
pub fn why_hidden(
    actors: &[Actor],
    draw: Draw,
    target: u32,
    eye_m: [f32; 3],
    yaw: f32,
) -> Result<Why, SightError> {
    if !eye_m.iter().all(|v| v.is_finite()) {
        return Err(SightError::NonFinitePoint);
    }
    let why = |verdict, evidence| Ok(Why { verdict, evidence });
    let Some(a) = actors.iter().find(|a| a.id == target) else {
        return why(Hidden::NotFound, 0);
    };
    if !draw.has_mesh {
        return why(Hidden::NoMesh, 0);
    }
    if !draw.has_material {
        return why(Hidden::NoMaterial, 0);
    }
    let finite = [a.x, a.y, a.z, a.sx, a.sy, a.sz]
        .iter()
        .all(|v| v.is_finite());
    if finite && [a.sx, a.sy, a.sz].contains(&0.0) {
        let side = [a.sx, a.sy, a.sz].map(|v| to_cm(v).abs()).into_iter().min();
        return why(Hidden::ZeroScale, i64::from(side.unwrap_or(0)));
    }
    let world = SightWorld::build(actors);
    let Some(b) = world.box_of(target).filter(|_| finite) else {
        return why(Hidden::DegenerateBounds, 0);
    };
    let e = [to_cm(eye_m[0]), to_cm(eye_m[1]), to_cm(eye_m[2])];
    let c = mid(b);
    let (dx, dy, dz) = (
        i64::from(c[0]) - i64::from(e[0]),
        i64::from(c[1]) - i64::from(e[1]),
        i64::from(c[2]) - i64::from(e[2]),
    );
    let flat = dx != 0 || dz != 0;
    let (fx, fz) = if yaw.is_finite() {
        let (x, z) = octant(yaw);
        (i64::from(x), i64::from(z))
    } else {
        (0, 0)
    };
    let dot = fx * dx + fz * dz;
    let lat = fx * dz - fz * dx;
    if flat && yaw.is_finite() && dot <= 0 {
        return why(Hidden::BehindEye, dot);
    }
    let range = i64::from(DIR_RANGE_CM);
    if dx * dx + dz * dz > range * range {
        return why(Hidden::TooFar, isqrt(dx * dx + dz * dz));
    }
    if dy.abs() > i64::from(VIEW_DY_CM) {
        return why(Hidden::OutOfView, dy.abs());
    }
    if !yaw.is_finite() || (flat && lat.abs() > dot) {
        return why(Hidden::OutOfView, i64::from(angle_deg(dot, lat)));
    }
    match first_blocker(&world, e, c) {
        Some(hit) => why(Hidden::OccludedBy(hit.id), i64::from(hit.dist_cm)),
        None => why(Hidden::Visible, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn a_pillar_is_named_and_half_hidden_reads_strictly_between() {
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
    fn why_hidden_has_one_fixture_per_verdict() {
        let eye = [0.0, 1.0, 0.0];
        let base = |id, x: f32, z: f32| solid(id, x, 1.0, z, 1.0, 2.0, 1.0);
        let check = |actors: &[Actor], draw: Draw, target: u32, yaw: f32| {
            let first = why_hidden(actors, draw, target, eye, yaw).unwrap();
            assert_eq!(first, why_hidden(actors, draw, target, eye, yaw).unwrap());
            first.verdict.word()
        };
        let yaw0 = 0.0; // facing +X
        let ok = Draw::default();
        // visible: straight ahead, nothing between.
        assert_eq!(check(&[base(1, 6.0, 0.0)], ok, 1, yaw0), "visible");
        assert_eq!(check(&[base(1, 6.0, 0.0)], ok, 9, yaw0), "not_found");
        let no_mesh = Draw {
            has_mesh: false,
            ..ok
        };
        assert_eq!(check(&[base(1, 6.0, 0.0)], no_mesh, 1, yaw0), "no_mesh");
        let no_mat = Draw {
            has_material: false,
            ..ok
        };
        assert_eq!(check(&[base(1, 6.0, 0.0)], no_mat, 1, yaw0), "no_material");
        let flat = solid(1, 6.0, 1.0, 0.0, 1.0, 0.0, 1.0);
        assert_eq!(check(&[flat], ok, 1, yaw0), "zero_scale");
        let mut nan = base(1, 6.0, 0.0);
        nan.x = f32::NAN;
        assert_eq!(check(&[nan], ok, 1, yaw0), "degenerate_bounds");
        assert_eq!(check(&[base(1, -6.0, 0.0)], ok, 1, yaw0), "behind_eye");
        assert_eq!(check(&[base(1, 30.0, 0.0)], ok, 1, yaw0), "too_far");
        assert_eq!(check(&[base(1, 2.0, 6.0)], ok, 1, yaw0), "out_of_view");
        let wall = solid(2, 3.0, 1.0, 0.0, 0.5, 2.0, 4.0);
        let hidden = check(&[base(1, 6.0, 0.0), wall], ok, 1, yaw0);
        assert_eq!(hidden, "occluded_by:2");
        let why = why_hidden(
            &[base(1, 6.0, 0.0), solid(2, 3.0, 1.0, 0.0, 0.5, 2.0, 4.0)],
            ok,
            1,
            eye,
            yaw0,
        )
        .unwrap();
        assert_eq!(why.evidence, 275, "wall face at x = 275 cm from the eye");
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
}
