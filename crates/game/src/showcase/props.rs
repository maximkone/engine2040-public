//! props: the tour's machine-props station route. The kit stages three props-kit machines in the
//! ring (`content/arena/placement.json` ids 12..14: ballista, crane, cart, all dressing), and the
//! station walks the human past each one so the tour shows what the props cook made.
//!
//! Owner: this module owns the walk-past waypoints only; the placement and the meshes belong to
//! `crate::arena_kit` and the props kit. Presentation and script only: nothing here collides, and
//! the machines are dressing (no obstacle box), so the walk cannot be blocked by them.
//!
//! Determinism: fixed waypoints and a nearest-first order with an index tie-break, so one tour
//! replay walks the same line. Budget: three waypoints, no work per tick.

use super::ruins::Waypoint;

/// One machine prop as placed: its planar center, meters (placement.json ids 12, 13, 14).
pub const MACHINES: [Waypoint; 3] = [[-8.0, 4.0], [-8.0, -4.0], [6.0, -6.0]];
/// How far beside a machine the walk passes, meters: clear of its own box on the `+z` side.
const CLEARANCE_M: f32 = 1.4;

/// The three waypoints, one beside each machine in `+z`, ordered nearest first from `from` so the
/// detour stays inside one station; a tie keeps the placement order.
#[must_use]
pub fn machine_walk(from: Waypoint) -> Vec<Waypoint> {
    let mut goals: Vec<(f32, usize, Waypoint)> = MACHINES
        .iter()
        .enumerate()
        .map(|(index, machine)| {
            let goal = [machine[0], machine[1] + CLEARANCE_M];
            let d = (goal[0] - from[0]).hypot(goal[1] - from[1]);
            (d, index, goal)
        })
        .collect();
    goals.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    goals.into_iter().map(|(_, _, goal)| goal).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F2P: the walk visits every one of the three machine props, nearest first, and stays inside
    /// the ring the placement puts them in.
    #[test]
    fn the_walk_passes_all_three_machines_nearest_first() {
        let from = [0.0, 0.0];
        let route = machine_walk(from);
        assert_eq!(route.len(), MACHINES.len(), "one waypoint per machine");
        for machine in MACHINES {
            assert!(
                route.iter().any(
                    |at| (at[0] - machine[0]).abs() < 0.01 && (at[1] - machine[1]).abs() <= 2.0
                ),
                "machine {machine:?} is on the walk: {route:?}"
            );
        }
        let mut last = 0.0;
        for at in &route {
            let d = (at[0] - from[0]).hypot(at[1] - from[1]);
            assert!(d >= last, "nearest first: {route:?}");
            last = d;
            assert!(
                at[0].abs() <= 9.0 && at[1].abs() <= 9.0,
                "waypoint {at:?} inside the ring"
            );
        }
    }

    /// The waypoints track the placement: each sits beside one machine actor the kit actually
    /// stages, so a moved machine fails here before the tour walks to a stale spot.
    #[cfg(feature = "app")]
    #[test]
    fn the_waypoints_sit_beside_the_placed_machines() {
        let actors = crate::arena_kit::kit_actors().expect("embedded placement parses");
        let machines: Vec<_> = actors
            .iter()
            .filter(|actor| (12..=14).contains(&actor.id))
            .collect();
        assert_eq!(machines.len(), MACHINES.len(), "three machine actors");
        for at in machine_walk([0.0, 0.0]) {
            let near = machines
                .iter()
                .map(|actor| (actor.pos_m[0] - at[0]).hypot(actor.pos_m[2] - at[1]))
                .fold(f32::INFINITY, f32::min);
            assert!(near <= 2.0, "waypoint {at:?} is {near} m from a machine");
        }
    }
}
