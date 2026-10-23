//! SP66: light reach — a point is lit when any light's reach covers it.
//!
//! Consumer: `scene.spatial` and `scene.brief` in `engine-endpoint` join the
//! document's `Light` components with the node poses, hand the lights here, and
//! list the dark objects and the unknown ones apart. Units: integer centimetres
//! in and out; meters arrive as `f32` and a value that is not finite has no
//! centimetre answer. Determinism: ids sorted, integer comparisons only, one
//! FNV-64 digest over the ordered verdicts — one scene hashes one way. Errors:
//! none; unreadable input is an answer ([`Lit::Unknown`]), never a panic.
//!
//! Rules: a directional light covers every point. A point or spot light covers
//! the points within its range, inclusive (a spot's cone is not read here — this
//! is reach, not visibility). A point or spot light whose range cannot be read
//! (not finite, or not positive) covers nothing and makes a point that no other
//! light covers `Unknown` instead of `Dark`. `Dark` therefore means every light
//! was read and none of them reaches the point.

use super::index::to_cm;
use super::intmath::isqrt;
use super::Actor;
use engine_core::Fnv64;

/// Family of a light, as the scene's `Light` component names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightFamily {
    /// Sun-like: covers every point at any distance.
    Directional,
    /// Omnidirectional, reaches its range.
    Point,
    /// Cone light: reach only here (the cone belongs to the render side).
    Spot,
}

/// One light the reach query reads: family, place and reach, all whole cm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReachLight {
    /// Light family.
    pub family: LightFamily,
    /// Light position in whole centimetres.
    pub at_cm: [i64; 3],
    /// Reach in whole centimetres. `None` when the authored range cannot be
    /// read (not finite, or not positive on a point or spot light); a
    /// directional light ignores it.
    pub range_cm: Option<i64>,
}

/// What one light or the whole set says about a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lit {
    /// At least one light reaches the point.
    Lit,
    /// Every light was read and none reaches the point.
    Dark,
    /// No light reaches the point, and at least one range could not be read, so
    /// the point is not `Dark`: it is `Unknown`.
    Unknown,
}

/// One light's contribution to a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// This light covers the point.
    Covers,
    /// This light was read and does not reach the point.
    Misses,
    /// This light's range cannot be read.
    Unreadable,
}

/// Whole centimetres of a meter value; `None` when it is not finite.
///
/// The conversion is the crate's one meter-to-cm rounding (`index::to_cm`), so
/// a light's place reads exactly like an actor's.
pub fn cm_of(meters: f32) -> Option<i64> {
    if !meters.is_finite() {
        return None;
    }
    Some(i64::from(to_cm(meters)))
}

impl ReachLight {
    /// A light of `family` at `at_cm` reading `range` meters, as the scene's
    /// `Light` component authors it. A directional light keeps no range (it
    /// covers every point); a point or spot light with an unreadable or
    /// non-positive range keeps `None` so the answer is `Unknown`, not `Dark`.
    pub fn new(family: LightFamily, at_cm: [i64; 3], range_m: f32) -> Self {
        let range_cm = match family {
            LightFamily::Directional => None,
            LightFamily::Point | LightFamily::Spot => cm_of(range_m).filter(|cm| *cm > 0),
        };
        Self {
            family,
            at_cm,
            range_cm,
        }
    }

    /// This light's contribution to a point in whole centimetres.
    pub fn reaches(&self, point_cm: [i64; 3]) -> Reach {
        match self.family {
            LightFamily::Directional => Reach::Covers,
            LightFamily::Point | LightFamily::Spot => match self.range_cm {
                None => Reach::Unreadable,
                // A range that is not positive cannot come through `new`, but
                // a hand-built light may carry one: it reads as unreadable, so
                // the answer stays `Unknown`, never a false `Dark`.
                Some(range) if range <= 0 => Reach::Unreadable,
                Some(range) => {
                    let [dx, dy, dz] = [
                        point_cm[0] - self.at_cm[0],
                        point_cm[1] - self.at_cm[1],
                        point_cm[2] - self.at_cm[2],
                    ];
                    if dx * dx + dy * dy + dz * dz <= range * range {
                        Reach::Covers
                    } else {
                        Reach::Misses
                    }
                }
            },
        }
    }
}

/// The set's answer for one point: `Lit` if any light covers it, else
/// `Unknown` if any light was unreadable, else `Dark`.
pub fn point_lit(point_cm: [i64; 3], lights: &[ReachLight]) -> Lit {
    let mut unreadable = false;
    for light in lights {
        match light.reaches(point_cm) {
            Reach::Covers => return Lit::Lit,
            Reach::Misses => {}
            Reach::Unreadable => unreadable = true,
        }
    }
    if unreadable {
        Lit::Unknown
    } else {
        Lit::Dark
    }
}

/// One scene's light answer: judged nodes, their counts, the dark and unknown
/// ids apart, and a digest over the ordered verdicts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReachReport {
    /// Nodes judged (pose readable).
    pub judged: u32,
    /// Nodes at least one light covers.
    pub lit: u32,
    /// Ids no light reaches, ascending.
    pub dark: Vec<u32>,
    /// Ids no light reaches while a range could not be read, ascending.
    pub unknown: Vec<u32>,
    /// Nodes skipped because their pose is not finite.
    pub unplaced: u32,
    /// Digest of the ordered verdicts; equal for equal answers.
    pub digest: u64,
}

impl ReachReport {
    /// `lit of judged`, with the dark and unknown ids apart.
    pub fn line(&self) -> String {
        let ids = |v: &[u32]| {
            if v.is_empty() {
                "-".to_string()
            } else {
                v.iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            }
        };
        format!(
            "lights: lit={} judged={} unplaced={} dark={} unknown={}",
            self.lit,
            self.judged,
            self.unplaced,
            ids(&self.dark),
            ids(&self.unknown)
        )
    }
}

/// Judge every actor's point against `lights`, by ascending id.
///
/// An actor whose pose is not finite is counted in `unplaced` and not judged;
/// the report never guesses for it.
pub fn reach_report(actors: &[Actor], lights: &[ReachLight]) -> ReachReport {
    let mut rows: Vec<(u32, [i64; 3])> = Vec::with_capacity(actors.len());
    let mut unplaced = 0u32;
    for actor in actors {
        match (cm_of(actor.x), cm_of(actor.y), cm_of(actor.z)) {
            (Some(x), Some(y), Some(z)) => rows.push((actor.id, [x, y, z])),
            _ => unplaced = unplaced.saturating_add(1),
        }
    }
    rows.sort_by_key(|(id, _)| *id);
    let mut report = ReachReport {
        unplaced,
        ..ReachReport::default()
    };
    let mut h = Fnv64::new();
    h.write(b"e2lightreach1");
    h.write_len(rows.len());
    for (id, point) in rows {
        let verdict = point_lit(point, lights);
        report.judged = report.judged.saturating_add(1);
        match verdict {
            Lit::Lit => report.lit = report.lit.saturating_add(1),
            Lit::Dark => report.dark.push(id),
            Lit::Unknown => report.unknown.push(id),
        }
        h.write_u64(u64::from(id));
        h.write_u64(match verdict {
            Lit::Lit => 1,
            Lit::Dark => 2,
            Lit::Unknown => 3,
        });
    }
    report.digest = h.finish();
    report
}

/// Distance in whole cm from a light to a point, floored; `None` when the
/// light's place is beyond the centimetre range the query reads.
pub fn distance_cm(light: &ReachLight, point_cm: [i64; 3]) -> Option<i64> {
    let [dx, dy, dz] = [
        point_cm[0].checked_sub(light.at_cm[0])?,
        point_cm[1].checked_sub(light.at_cm[1])?,
        point_cm[2].checked_sub(light.at_cm[2])?,
    ];
    let sq = dx
        .checked_mul(dx)?
        .checked_add(dy.checked_mul(dy)?)?
        .checked_add(dz.checked_mul(dz)?)?;
    Some(isqrt(sq))
}

#[cfg(test)]
mod tests {
    use super::*;

    const POINT_AT_ORIGIN: [i64; 3] = [0, 0, 0];

    fn actor(id: u32, x: f32) -> Actor {
        Actor {
            id,
            name: format!("node_{id}"),
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
    fn light_reach_a_farther_light_covers_what_the_nearest_misses() {
        let near = ReachLight::new(LightFamily::Point, POINT_AT_ORIGIN, 0.5);
        let far = ReachLight::new(LightFamily::Point, [100, 0, 0], 2.0);
        let point = [90, 0, 0];
        assert_eq!(near.reaches(point), Reach::Misses);
        assert_eq!(far.reaches(point), Reach::Covers);
        assert_eq!(point_lit(point, &[near, far]), Lit::Lit);
        assert_eq!(point_lit(point, &[near]), Lit::Dark);
    }

    #[test]
    fn light_reach_a_point_outside_every_range_is_dark() {
        let lights = [
            ReachLight::new(LightFamily::Point, POINT_AT_ORIGIN, 1.0),
            ReachLight::new(LightFamily::Spot, [0, 0, 500], 2.0),
        ];
        assert_eq!(point_lit([0, 0, -400], &lights), Lit::Dark);
        // The boundary counts as covered: reach is inclusive.
        assert_eq!(point_lit([0, 0, 300], &lights), Lit::Lit);
    }

    #[test]
    fn light_reach_an_unreadable_range_is_unknown_never_dark() {
        let unreadable = [
            ReachLight::new(LightFamily::Point, POINT_AT_ORIGIN, f32::NAN),
            ReachLight::new(LightFamily::Spot, POINT_AT_ORIGIN, 0.0),
            ReachLight {
                family: LightFamily::Point,
                at_cm: POINT_AT_ORIGIN,
                range_cm: Some(-5),
            },
        ];
        for light in unreadable {
            assert_eq!(light.reaches([10_000, 10_000, 10_000]), Reach::Unreadable);
            assert_eq!(point_lit([10_000, 10_000, 10_000], &[light]), Lit::Unknown);
        }
        // A readable light that covers still wins over an unreadable one.
        let lit = ReachLight::new(LightFamily::Directional, POINT_AT_ORIGIN, 0.0);
        assert_eq!(point_lit([9, 9, 9], &[unreadable[0], lit]), Lit::Lit);
    }

    #[test]
    fn light_reach_a_directional_light_covers_every_point() {
        let sun = ReachLight::new(LightFamily::Directional, [1, 2, 3], f32::INFINITY);
        assert_eq!(sun.range_cm, None);
        assert_eq!(point_lit([-90_000, 5, 0], &[sun]), Lit::Lit);
    }

    #[test]
    fn light_reach_report_keeps_dark_and_unknown_apart_and_digests_x2() {
        let lights = [
            ReachLight::new(LightFamily::Point, POINT_AT_ORIGIN, 1.0),
            ReachLight::new(LightFamily::Point, [0, 0, 900], f32::NAN),
        ];
        let actors = [
            actor(7, 0.0),  // inside the near light
            actor(3, 5.0),  // outside the near light, the far one is unreadable
            actor(9, 10.0), // outside both
        ];
        let first = reach_report(&actors, &lights);
        let second = reach_report(&actors, &lights);
        assert_eq!(first, second);
        assert_eq!(first.digest, second.digest);
        assert_eq!(first.judged, 3);
        assert_eq!(first.lit, 1);
        assert!(first.dark.is_empty());
        assert_eq!(first.unknown, vec![3, 9]);
        assert_eq!(first.unplaced, 0);
        assert_eq!(
            first.line(),
            "lights: lit=1 judged=3 unplaced=0 dark=- unknown=3,9"
        );

        // Drop the unreadable light: the same two ids become plain dark, and
        // the digest moves because the answer moved.
        let readable = reach_report(&actors, &lights[..1]);
        assert_eq!(readable.dark, vec![3, 9]);
        assert!(readable.unknown.is_empty());
        assert_ne!(readable.digest, first.digest);
    }

    #[test]
    fn light_reach_report_unknown_ids_when_only_one_range_is_unreadable() {
        // Both lights are unreadable and neither covers: every id is unknown.
        let lights = [ReachLight::new(LightFamily::Point, POINT_AT_ORIGIN, 0.0)];
        let report = reach_report(&[actor(4, 3.0)], &lights);
        assert_eq!(report.unknown, vec![4]);
        assert!(report.dark.is_empty());
        assert_eq!(report.lit, 0);
    }

    #[test]
    fn light_reach_report_counts_an_unreadable_pose_as_unplaced() {
        let report = reach_report(&[actor(1, f32::NAN)], &[]);
        assert_eq!(report.judged, 0);
        assert_eq!(report.unplaced, 1);
        assert!(report.dark.is_empty());
        assert!(report.unknown.is_empty());
    }

    #[test]
    fn light_reach_distance_floors_and_refuses_overflow() {
        let light = ReachLight::new(LightFamily::Point, [0, 0, 0], 10.0);
        assert_eq!(distance_cm(&light, [300, 400, 0]), Some(500));
        assert_eq!(distance_cm(&light, [1, 1, 1]), Some(1));
        assert_eq!(distance_cm(&light, [i64::MIN, 0, 0]), None);
    }

    #[test]
    fn light_reach_the_check_catalog_is_still_eighteen_names() {
        // SP66 adds a query, never a check: the frozen catalog stays 18.
        assert_eq!(crate::SPATIAL_CHECKS.len(), 18);
    }
}
