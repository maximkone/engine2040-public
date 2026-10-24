//! noose: the picture of the laquearius' cord (row SG11), a child of [`super`].
//!
//! The authority throws a loop and snares a man with it (`engine-arena`, `spear::noose`); the
//! look reads the same [`SpearSlot`]s the spear fills and draws no spear for it: a loop in the
//! air is only the cord out to it, and a snare is the cord from the thrower's hand to the
//! man's chest. The cord is the spear's rope, tied through the same verlet world, so it hangs
//! while the man is inside it and runs straight when he strains at its end (the authority
//! tightens the cord to the shortest gap it has seen, and `Tether::rope_mm` is that length).
//! Nobody is towed by a snare, so the ragdoll layer is told nothing: the man walks, slowed,
//! and a boss push topples him through the fight's own knockdown.
//!
//! Callers: `SpearLook::fold_slot`. Units: meters, Y up. Presentation only: nothing here is
//! hashed and the sim never reads it. Errors: none. Budget: one rope per slot, as before.

use super::*;
use engine_arena::spear::{Flight, Tether};

impl SpearLook {
    /// A loop in the air: no spear is drawn (the lane is cleared of the one the throw event
    /// made), only the cord from the hand out to the loop.
    pub(super) fn fold_loop(&mut self, thrower: u8, flight: &Flight, hand: [f32; 3]) {
        self.lanes[usize::from(thrower)].spear = None;
        let tip = mm3(flight.pos_mm);
        let gap = norm(sub(tip, hand));
        let blocks = self.fx.tether_blocks();
        self.tie(
            thrower,
            hand,
            tip,
            gap * FLIGHT_SLACK + FLIGHT_EXTRA_M,
            &blocks,
        );
    }

    /// A snare: the cord from the thrower's hand to the man's chest, as long as the
    /// authority's cord. No body is towed, so `bodies` is left alone.
    pub(super) fn fold_snare(
        &mut self,
        thrower: u8,
        tether: &Tether,
        hand: [f32; 3],
        feet: &dyn Fn(u8) -> Option<([f32; 3], f32)>,
        ties: Ties<'_>,
    ) {
        self.lanes[usize::from(thrower)].spear = None;
        let Some((victim, (pos, yaw))) = tether
            .victim
            .and_then(|victim| feet(victim).map(|at| (victim, at)))
        else {
            return;
        };
        let far = self.anchor(victim, pos, yaw, ties, false);
        let cord = f32::from(i16::try_from(tether.rope_mm).unwrap_or(i16::MAX)) / MM;
        let blocks = self.fx.tether_blocks();
        self.tie(thrower, hand, far, cord, &blocks);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FighterView;
    use engine_arena::spear::EndWhy;

    const TOWARD: [i32; 2] = [0, 65_536];

    fn fighters() -> [FighterView; 2] {
        [
            FighterView {
                slot: 0,
                pos_m: [0.0, 0.0, 0.0],
                yaw: 0.0,
            },
            FighterView {
                slot: 1,
                pos_m: [0.0, 0.0, 4.0],
                yaw: 0.0,
            },
        ]
    }

    fn slots(first: Phase) -> [SpearSlot; 2] {
        [
            SpearSlot {
                phase: first,
                ..SpearSlot::default()
            },
            SpearSlot::default(),
        ]
    }

    fn flight(noose: bool) -> Flight {
        Flight {
            pos_mm: [0, 1_300, 900],
            vel_mm: [0, 0, 500],
            age: 1,
            origin_mm: [0, 0],
            dir: TOWARD,
            world: false,
            noose,
        }
    }

    fn tether(noose: bool) -> Tether {
        Tether {
            victim: Some(1),
            goal_mm: [0, 4_000],
            anchor_mm: [0, 4_000],
            anchor_y_mm: 1_200,
            rope_mm: 4_100,
            dir: TOWARD,
            age: 4,
            latched: false,
            yank: false,
            noose,
            speed: 0,
        }
    }

    /// The thrown event, the flight, the catch and the end, fed to one picture. Returns the
    /// picture after each of the four ticks, and once more after the freed rope settled, as
    /// `(spears, ropes, towed bodies)`.
    fn tour(noose: bool) -> Vec<(usize, usize, usize)> {
        let mut look = SpearLook::new(DEFAULT_TIER);
        let who = fighters();
        let thrown = SpearEvent::Thrown {
            thrower: 0,
            at_mm: [0, 1_300, 400],
            vel_mm: [0, 0, 500],
        };
        let caught = SpearEvent::Hit {
            thrower: 0,
            victim: Some(1),
            at_mm: [0, 1_200, 3_600],
        };
        let freed = SpearEvent::TetherEnd {
            thrower: 0,
            victim: Some(1),
            why: EndWhy::Timeout,
            damage_permille: 0,
        };
        let ticks: [(&[SpearEvent], Phase); 4] = [
            (&[thrown], Phase::Flight(flight(noose))),
            (&[], Phase::Flight(flight(noose))),
            (&[caught], Phase::Tether(tether(noose))),
            (&[freed], Phase::Idle),
        ];
        let mut seen = Vec::new();
        for (events, phase) in ticks {
            look.observe(events, &slots(phase), &who, Ties::NONE);
            let towed = look.carries().iter().flatten().count();
            seen.push((look.spears(), look.ropes(), towed));
        }
        // A freed rope falls and settles before it is removed: let it.
        for _ in 0..600 {
            if look.ropes() == 0 {
                break;
            }
            look.observe(&[], &slots(Phase::Idle), &who, Ties::NONE);
        }
        seen.push((look.spears(), look.ropes(), 0));
        seen
    }

    #[test]
    fn a_loop_and_its_snare_draw_a_cord_and_no_spear() {
        let seen = tour(true);
        assert_eq!(
            seen[0],
            (0, 1, 0),
            "the loop in the air is a cord alone: {seen:?}"
        );
        assert_eq!(
            seen[2],
            (0, 1, 0),
            "the snare is a cord alone and tows nobody: {seen:?}"
        );
        assert_eq!(
            seen[4].1, 0,
            "the cord is gone once the freed rope has settled: {seen:?}"
        );
    }

    #[test]
    fn the_same_ticks_as_a_spear_draw_a_spear_and_tow_both_bodies() {
        // The control: without the noose flag the look is the spear's, so the difference
        // above is the noose branch and nothing else.
        let seen = tour(false);
        assert_eq!(seen[0].0, 1, "a spear flies: {seen:?}");
        assert_eq!(seen[2], (1, 1, 2), "a carry tows the pair: {seen:?}");
    }

    #[test]
    fn the_snare_cord_runs_from_the_hand_to_the_mans_chest() {
        let mut look = SpearLook::new(DEFAULT_TIER);
        let who = fighters();
        let caught = SpearEvent::Hit {
            thrower: 0,
            victim: Some(1),
            at_mm: [0, 1_200, 3_600],
        };
        for n in 0..30 {
            let events: &[SpearEvent] = if n == 0 { &[caught] } else { &[] };
            look.observe(
                events,
                &slots(Phase::Tether(tether(true))),
                &who,
                Ties::NONE,
            );
        }
        let nodes = look.rope(0, 1.0).expect("a rope is tied");
        let (first, last) = (nodes[0], nodes[nodes.len() - 1]);
        assert!(first[2].abs() < 0.5, "starts at the thrower: {first:?}");
        assert!(
            (last[2] - 4.0).abs() < 0.2 && (last[1] - CHEST_Y_M).abs() < 0.2,
            "ends on the man's chest: {last:?}"
        );
    }
}
