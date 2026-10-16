//! collide: integer fighter collision for the default leg (EG20).
//!
//! The default leg used to clamp each fighter into a square on its own, so
//! eight fighters could stand on one point and props never blocked (bodies
//! G6; geaen 12.5.3.6, gems3 ch 18). This pass runs once per tick after
//! every slot moved: fighter capsules push apart through the `controller2`
//! fighter push ([`separate_pair`], reused, not forked), fighter centres are
//! pushed out of static pillar boxes, and a round wall keeps every capsule
//! inside the arena circle.
//!
//! Consumer: `Sim::step_with` under [`ArenaRules::collision`]
//! (rules v2 turns it on; v1 never runs it). Data owner: the caller owns the
//! walkers; pillars travel in the rules. Units: integer millimetres on the
//! XZ plane. Determinism: `+ - * /` and `isqrt` on integers only, pairs in
//! slot order, so the same poses give the same pushes on every PC. Never
//! panics; no allocation (the caller's slice is pushed in place).
//!
//! [`ArenaRules::collision`]: crate::ArenaRules::collision
//! [`separate_pair`]: engine_physics::controller2::separate_pair

use engine_physics::controller2::{separate_pair, Walker, RADIUS_MM};

/// Relaxation passes per tick: each pass pushes every overlapping pair half
/// its overlap, then resolves pillars and the wall.
const COLLIDE_PASSES: usize = 4;

/// Most static pillar boxes one match carries.
pub const MAX_PILLARS: usize = 8;

/// Fighter capsule radius the collision pass keeps between bodies and from
/// pillars and the wall, in mm (the `controller2` walker radius).
pub const COLLIDE_RADIUS_MM: i32 = RADIUS_MM;

/// One static full-height pillar: an axis-aligned XZ footprint in mm.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pillar {
    /// Minimum corner `[x, z]` in mm.
    pub min_mm: [i32; 2],
    /// Maximum corner `[x, z]` in mm.
    pub max_mm: [i32; 2],
}

impl Pillar {
    /// Footprint with each lane ordered min to max.
    #[must_use]
    pub fn new(a: [i32; 2], b: [i32; 2]) -> Self {
        Self {
            min_mm: [a[0].min(b[0]), a[1].min(b[1])],
            max_mm: [a[0].max(b[0]), a[1].max(b[1])],
        }
    }
}

/// Resolve one tick of contacts in place: [`COLLIDE_PASSES`] passes of
/// pairwise push-apart in slot order, then pillar push-out, then the round
/// wall of `wall_radius_mm` (centre at the origin).
pub(crate) fn resolve(walkers: &mut [Walker], pillars: &[Pillar], wall_radius_mm: i32) {
    for _ in 0..COLLIDE_PASSES {
        for i in 0..walkers.len() {
            let (head, tail) = walkers.split_at_mut(i + 1);
            let Some(a) = head.last_mut() else {
                continue;
            };
            for b in tail.iter_mut() {
                separate_pair(a, b);
            }
        }
        for walker in walkers.iter_mut() {
            for pillar in pillars {
                push_out(walker, pillar);
            }
            hold_inside(walker, wall_radius_mm);
        }
    }
}

/// Push a fighter centre out of a pillar footprint grown by the capsule
/// radius, along the axis of least penetration (x first on ties, then the
/// lower side), so the centre ends on the grown edge.
fn push_out(walker: &mut Walker, pillar: &Pillar) {
    let r = COLLIDE_RADIUS_MM;
    let (x, z) = (walker.pos_mm[0], walker.pos_mm[2]);
    let (x0, x1) = (pillar.min_mm[0] - r, pillar.max_mm[0] + r);
    let (z0, z1) = (pillar.min_mm[1] - r, pillar.max_mm[1] + r);
    if x <= x0 || x >= x1 || z <= z0 || z >= z1 {
        return;
    }
    let pushes = [
        (x - x0, 0, x0),
        (x1 - x, 0, x1),
        (z - z0, 2, z0),
        (z1 - z, 2, z1),
    ];
    let mut best = pushes[0];
    for push in pushes {
        if push.0 < best.0 {
            best = push;
        }
    }
    walker.pos_mm[best.1] = best.2;
}

/// Keep the capsule inside the round wall: a centre past the radius scales
/// back onto it (toward the origin), integer-only.
fn hold_inside(walker: &mut Walker, wall_radius_mm: i32) {
    let radius = i64::from(wall_radius_mm.max(0));
    let (x, z) = (i64::from(walker.pos_mm[0]), i64::from(walker.pos_mm[2]));
    let dist2 = u64::try_from(x * x + z * z).unwrap_or(u64::MAX);
    let radius2 = u64::try_from(radius * radius).unwrap_or(0);
    if dist2 <= radius2 {
        return;
    }
    let dist = i64::try_from(dist2.isqrt()).unwrap_or(i64::MAX).max(1);
    walker.pos_mm[0] = i32::try_from(x * radius / dist).unwrap_or(walker.pos_mm[0]);
    walker.pos_mm[2] = i32::try_from(z * radius / dist).unwrap_or(walker.pos_mm[2]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walkers_at(count: u32, pos: [i32; 3]) -> Vec<Walker> {
        (0..count).map(|id| Walker::spawn(id, pos)).collect()
    }

    #[test]
    fn coincident_walkers_spread_and_hold_the_wall() {
        let mut walkers = walkers_at(8, [0, 0, 0]);
        for _ in 0..30 {
            resolve(&mut walkers, &[], 19_600);
        }
        for (i, a) in walkers.iter().enumerate() {
            for b in &walkers[i + 1..] {
                let dx = i64::from(b.pos_mm[0] - a.pos_mm[0]);
                let dz = i64::from(b.pos_mm[2] - a.pos_mm[2]);
                assert!(dx * dx + dz * dz >= 4 * i64::from(RADIUS_MM).pow(2) - 4);
            }
        }
    }

    #[test]
    fn pillar_pushes_the_centre_out_and_the_wall_holds() {
        let pillar = Pillar::new([1_000, -500], [-1_000, 500]);
        assert_eq!(pillar.min_mm, [-1_000, -500]);
        let mut walker = Walker::spawn(0, [100, 0, 400]);
        push_out(&mut walker, &pillar);
        assert_eq!(walker.pos_mm, [100, 0, 500 + RADIUS_MM]);
        let mut far = Walker::spawn(1, [30_000, 0, 40_000]);
        hold_inside(&mut far, 10_000);
        assert_eq!([far.pos_mm[0], far.pos_mm[2]], [6_000, 8_000]);
    }
}
