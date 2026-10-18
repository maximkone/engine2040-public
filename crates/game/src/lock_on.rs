//! lock_on: the human fighter's soft lock-on, the `facing` the window fills in.
//!
//! Owner: game lane, input mapping. Consumers: the `arena` window
//! (`bin/arena_frame`), which puts the bearing on the live keyboard and pad
//! input every tick, and `spear_window::demo_input`, which aims the
//! `--spear-demo` script with the same [`bearing_cdeg`]. The bindings and the
//! pad emit `facing = 0`, and the sim reads `facing` as the swing direction of
//! a standing attack and the flight direction of a spear throw
//! (`engine_arena`'s `facing_q16`), so before this a standing `F` swung toward
//! world +z and `Q` threw toward world +z, not at the rival.
//!
//! Units: positions are meters (the drawn places of `App::fighter_views`),
//! `facing` is centidegrees, `-17999..=17999`, 0 is world +z and +9000 is world
//! +x (`atan2(dx, dz)`). Determinism: pure float math over the drawn places,
//! then one integer quantize; the sim sees only the integer, exactly as it sees
//! a keyboard facing, and the match identity, tape format and every hashed byte
//! are untouched. Errors: none; with no living rival, or a place that is not
//! finite, the answer is `None` and the caller keeps the input as it came.
//! Budget: one pass over the fighters, no heap.

use crate::FighterView;

/// Largest `facing` the lock-on writes, centidegrees. The wire allows 18000;
/// the demo script has always stopped one short, and so does this.
const FACING_CAP_CDEG: i32 = 17_999;

/// Floor distance (meters) below which two fighters share a spot and name no
/// bearing; the caller keeps the facing it had.
const MIN_RANGE_M: f32 = 1.0e-3;

/// A float rounded to a whole number, saturating at the `i32` ends (a NaN is
/// 0). The window scripts' wishes and facings are far inside `i32`.
#[must_use]
pub fn whole(v: f32) -> i32 {
    v.round() as i32
}

/// Bearing of a floor offset `(dx_m, dz_m)` as the sim's `facing`: centidegrees
/// from world +z toward world +x, rounded and held inside `-17999..=17999`.
/// `None` when an offset is not finite or the two spots coincide. Pure.
#[must_use]
pub fn bearing_cdeg(dx_m: f32, dz_m: f32) -> Option<i16> {
    if !(dx_m.is_finite() && dz_m.is_finite()) || dx_m.hypot(dz_m) < MIN_RANGE_M {
        return None;
    }
    let centideg = dx_m.atan2(dz_m).to_degrees() * 100.0;
    i16::try_from(whole(centideg).clamp(-FACING_CAP_CDEG, FACING_CAP_CDEG)).ok()
}

/// The `facing` that points `human` at the nearest living rival: every other
/// fighter whose `hp` (permille, indexed by slot) is above zero, the closest on
/// the floor, the lower slot when two stand equally far. A slot past the end of
/// `hp` counts as alive. `None` when `human` is not in `views`, nobody else is
/// left alive, or the nearest has no readable bearing. Pure over the views.
#[must_use]
pub fn nearest_rival_facing(human: u8, views: &[FighterView], hp: &[u16]) -> Option<i16> {
    let me = views.iter().find(|view| view.slot == human)?;
    let (_, _, dx, dz) = views
        .iter()
        .filter(|view| view.slot != human)
        .filter(|view| hp.get(usize::from(view.slot)).is_none_or(|&left| left > 0))
        .filter_map(|view| {
            let (dx, dz) = (view.pos_m[0] - me.pos_m[0], view.pos_m[2] - me.pos_m[2]);
            let dist = dx.hypot(dz);
            dist.is_finite().then_some((dist, view.slot, dx, dz))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))?;
    bearing_cdeg(dx, dz)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(slot: u8, x: f32, z: f32) -> FighterView {
        FighterView {
            slot,
            pos_m: [x, 0.0, z],
            yaw: 0.0,
        }
    }

    /// Two fighters: the human at (2, 5), one rival `(dx, dz)` meters away.
    fn facing_to(dx: f32, dz: f32) -> Option<i16> {
        let views = [view(0, 2.0, 5.0), view(1, 2.0 + dx, 5.0 + dz)];
        nearest_rival_facing(0, &views, &[1000, 1000])
    }

    #[test]
    fn eight_relative_positions_name_the_eight_compass_bearings() {
        // 0 is world +z, +9000 is world +x: the sim's `facing_q16` axes.
        let cases = [
            ("north +z", (0.0, 4.0), 0),
            ("north east", (3.0, 3.0), 4_500),
            ("east +x", (6.0, 0.0), 9_000),
            ("south east", (2.5, -2.5), 13_500),
            ("south -z", (0.0, -7.0), 17_999),
            ("south west", (-2.0, -2.0), -13_500),
            ("west -x", (-5.0, 0.0), -9_000),
            ("north west", (-4.0, 4.0), -4_500),
        ];
        for (name, (dx, dz), want) in cases {
            assert_eq!(facing_to(dx, dz), Some(want), "{name}");
        }
    }

    #[test]
    fn the_bearing_follows_the_angle_and_stays_inside_the_wire_range() {
        // A 3-4-5 triangle: atan2(3, 4) = 36.87 degrees.
        assert_eq!(bearing_cdeg(3.0, 4.0), Some(3_687));
        assert_eq!(bearing_cdeg(-3.0, 4.0), Some(-3_687));
        for step in 0..720u16 {
            let turn = f32::from(step) * core::f32::consts::TAU / 720.0;
            let facing = bearing_cdeg(turn.sin() * 9.0, turn.cos() * 9.0).expect("a bearing");
            assert!((-17_999..=17_999).contains(&facing), "{turn}: {facing}");
        }
    }

    #[test]
    fn the_nearest_rival_wins_and_a_tie_goes_to_the_lower_slot() {
        // Slot 2 is nearer than slot 1: it is the one aimed at (north east).
        let views = [view(0, 0.0, 0.0), view(1, -9.0, 0.0), view(2, 2.0, 2.0)];
        assert_eq!(nearest_rival_facing(0, &views, &[1000; 3]), Some(4_500));
        // Slots 2 and 1 stand 3 m away on opposite sides, listed 2 first: slot 1
        // (west) wins the tie whatever the order of the views.
        let tied = [view(0, 0.0, 0.0), view(2, 3.0, 0.0), view(1, -3.0, 0.0)];
        assert_eq!(nearest_rival_facing(0, &tied, &[1000; 3]), Some(-9_000));
        let flipped = [view(0, 0.0, 0.0), view(1, -3.0, 0.0), view(2, 3.0, 0.0)];
        assert_eq!(nearest_rival_facing(0, &flipped, &[1000; 3]), Some(-9_000));
        // The human need not be slot 0.
        let third = [view(0, 8.0, 0.0), view(1, 0.0, 0.0), view(2, 0.0, 4.0)];
        assert_eq!(nearest_rival_facing(2, &third, &[1000; 3]), Some(17_999));
    }

    #[test]
    fn dead_rivals_are_ignored() {
        let views = [view(0, 0.0, 0.0), view(1, 1.0, 1.0), view(2, -6.0, 0.0)];
        // Slot 1 is nearest but out of hp: the live one farther away is aimed at.
        assert_eq!(
            nearest_rival_facing(0, &views, &[700, 0, 400]),
            Some(-9_000)
        );
        // With the live rival also out, nobody is left to aim at.
        assert_eq!(nearest_rival_facing(0, &views, &[700, 0, 0]), None);
        // The human's own hp never matters: a downed human still faces the rival.
        assert_eq!(nearest_rival_facing(0, &views, &[0, 300, 300]), Some(4_500));
        // A slot past the end of the hp table counts as alive.
        assert_eq!(nearest_rival_facing(0, &views, &[700]), Some(4_500));
    }

    #[test]
    fn nobody_to_aim_at_names_no_bearing() {
        let alone = [view(0, 1.0, 1.0)];
        assert_eq!(nearest_rival_facing(0, &alone, &[1000]), None);
        let views = [view(0, 0.0, 0.0), view(1, 3.0, 0.0)];
        assert_eq!(
            nearest_rival_facing(2, &views, &[1000; 2]),
            None,
            "no human"
        );
        assert_eq!(nearest_rival_facing(0, &[], &[]), None);
        // Standing on the same spot gives no direction; a broken place is skipped.
        assert_eq!(bearing_cdeg(0.0, 0.0), None);
        let stacked = [view(0, 2.0, 2.0), view(1, 2.0, 2.0)];
        assert_eq!(nearest_rival_facing(0, &stacked, &[1000; 2]), None);
        let broken = [view(0, 0.0, 0.0), view(1, f32::NAN, 0.0), view(2, 0.0, 5.0)];
        assert_eq!(nearest_rival_facing(0, &broken, &[1000; 3]), Some(0));
        assert_eq!(bearing_cdeg(f32::INFINITY, 1.0), None);
    }
}
