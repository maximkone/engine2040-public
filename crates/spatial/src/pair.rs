//! SPR2: the numbers behind a directional answer.
//!
//! Consumer: `scene.query kind=relation` hits (lane A wires them; see
//! `docs/plan/spatial-entry-points.md`). Units: meters in, integer cm out.
//! Determinism: integer math only, no float; the same actors give the same
//! numbers in any input order.

use super::intmath::{bearing_cw, isqrt};
use super::relation::{edge_gap2, frame_fwd, v2view, Frame};
use super::Actor;

/// Plan-view distance, compass bearing and clear gap between two actors, all
/// integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairNumbers {
    /// XZ distance between the two centers, floored, in cm.
    pub dist_cm: i64,
    /// Whole degrees clockwise from the frame's forward, seen from the
    /// anchor: 0 ahead, 90 right, 180 behind, 270 left (magnitude floored).
    pub bearing_deg: u32,
    /// XZ edge-to-edge gap in cm, 0 when the footprints touch or overlap.
    pub gap_cm: i32,
}

/// Distance, bearing and gap of `thing` as seen from `anchor` in `frame`
/// (SPR2): the figures an answer to "what is left_of the anchor" can carry,
/// so a hit needs no second call. World forward is north, Subject uses the
/// anchor's own facing, Viewer uses that viewer's. `None` for unknown or
/// degenerate actors, the same id, or a yawless facing.
pub fn pair_numbers(
    actors: &[Actor],
    anchor: u32,
    thing: u32,
    frame: Frame,
) -> Option<PairNumbers> {
    if anchor == thing {
        return None;
    }
    let view = v2view(actors);
    let (ba, bt) = (view.boxes.get(&anchor)?, view.boxes.get(&thing)?);
    let (ca, ct) = (view.centers.get(&anchor)?, view.centers.get(&thing)?);
    let f = frame_fwd(&view, frame, anchor)?;
    let dx = i64::from(ct.0) - i64::from(ca.0);
    let dz = i64::from(ct.2) - i64::from(ca.2);
    let (fx, fz) = (i64::from(f.0), i64::from(f.1));
    let (dot, lat) = (dx * fx + dz * fz, dx * -fz + dz * fx);
    Some(PairNumbers {
        dist_cm: isqrt(dx * dx + dz * dz),
        bearing_deg: bearing_cw(dot, lat),
        gap_cm: i32::try_from(isqrt(edge_gap2(ba, bt))).unwrap_or(i32::MAX),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f32::consts::FRAC_PI_2;

    fn cube(id: u32, x: f32, z: f32, yaw: f32) -> Actor {
        Actor {
            id,
            name: format!("c{id}"),
            x,
            y: 0.5,
            z,
            sx: 1.0,
            sy: 1.0,
            sz: 1.0,
            parent: None,
            physical: true,
            vx: 0.0,
            vz: 0.0,
            yaw,
        }
    }

    #[test]
    fn pair_numbers_read_the_plan() {
        // Floor plan, north up: lamp (0,-3), chair (-3,-3), sofa (-3,0), table
        // (0,0) facing east, door (3,0), rug (0,3).
        let actors = vec![
            cube(1, 0.0, 0.0, 0.0),
            cube(2, 0.0, -3.0, 0.0),
            cube(3, -3.0, 0.0, FRAC_PI_2),
            cube(4, 3.0, 0.0, 0.0),
            cube(5, 0.0, 3.0, 0.0),
            cube(6, -3.0, -3.0, 0.0),
        ];
        let n = |a: u32, t: u32, f: Frame| pair_numbers(&actors, a, t, f).expect("numbers");
        // Sofa as seen from the table, world frame: 300 cm west, 200 cm gap.
        let west = n(1, 3, Frame::World);
        assert_eq!(
            (west.dist_cm, west.bearing_deg, west.gap_cm),
            (300, 270, 200)
        );
        assert_eq!(n(1, 4, Frame::World).bearing_deg, 90);
        assert_eq!(n(1, 2, Frame::World).bearing_deg, 0);
        assert_eq!(n(1, 5, Frame::World).bearing_deg, 180);
        // Table faces east: the door is dead ahead, the lamp 90 degrees left.
        assert_eq!(n(1, 4, Frame::Subject).bearing_deg, 0);
        assert_eq!(n(1, 2, Frame::Subject).bearing_deg, 270);
        // Sofa faces south: the table is on its left.
        assert_eq!(n(3, 1, Frame::Subject).bearing_deg, 270);
        // Chair is the diagonal: 424 cm away, 45 degrees left of north.
        let diag = n(1, 6, Frame::World);
        assert_eq!((diag.dist_cm, diag.bearing_deg), (424, 315));
        assert_eq!(n(1, 6, Frame::World), n(1, 6, Frame::World), "replay x2");
        assert_eq!(pair_numbers(&actors, 1, 1, Frame::World), None);
        assert_eq!(pair_numbers(&actors, 1, 99, Frame::World), None);
    }

    /// Independent check with no angle math: the bearing lands in the right
    /// quadrant for the signs of the offset, and the distance is the floor
    /// root found by counting up.
    #[test]
    fn bearing_quadrant_and_distance_match_counting() {
        use engine_core::Rng;
        let mut rng = Rng::new(0xb34);
        for case in 0..100 {
            let mut p = || (rng.next_u32() % 4000) as f32 / 100.0 - 20.0;
            let (ax, az, tx, tz) = (p(), p(), p(), p());
            let actors = vec![cube(1, ax, az, 0.0), cube(2, tx, tz, 0.0)];
            let got = pair_numbers(&actors, 1, 2, Frame::World).expect("numbers");
            let cm = |v: f32| (v * 100.0).round() as i64;
            let (dx, dz) = (cm(tx) - cm(ax), cm(tz) - cm(az));
            if dx.abs() < 300 || dz.abs() < 300 {
                continue;
            }
            // North is -z, east is +x: east+north is 0..90, east+south 90..180.
            let want = match (dx > 0, dz < 0) {
                (true, true) => 0..90,
                (true, false) => 90..180,
                (false, false) => 180..270,
                (false, true) => 270..360,
            };
            assert!(
                want.contains(&got.bearing_deg),
                "case {case}: {}",
                got.bearing_deg
            );
            let mut r = 0i64;
            while (r + 1) * (r + 1) <= dx * dx + dz * dz {
                r += 1;
            }
            assert_eq!(got.dist_cm, r, "case {case} dist");
        }
    }
}
