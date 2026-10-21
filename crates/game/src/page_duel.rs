//! page_duel: the browser page's duel, sized to the kit corridor.
//!
//! Owner: engine-game. Consumer: the web page (`crates/web/src/fight.rs`)
//! builds its match from these numbers, and the test below proves the pair
//! starts inside the kit's corridor with the rival in front of the camera.
//! Units: meters for places, radians for headings. Determinism scope: the seed
//! and the extent give the same two start feet on every machine (the arena's
//! splitmix stream, no RNG of its own). Errors: typed [`PageDuelError`], never
//! panics. Budget: one spawn read at init; zero per tick.

use engine_arena::initial_state;

/// Seed of the page's duel. Picked so both seats stand on the corridor's centre
/// line, the human in front of the rival, about five meters apart: the page's
/// first frame is the fight, not two fighters walking to each other.
pub const PAGE_DUEL_SEED: u64 = 326_245;
/// Half-extent of the arena the page plays in, meters. The engine refuses an
/// arena under five meters, and the stone the fighter can see must be the wall
/// that stops it: the kit hall runs from the stands at x -14 to the gate at
/// x 13 with its own walls at z ±10, so a thirteen meter box puts the sim's
/// walls past the stone and lets the corridor's walls do the stopping.
pub const PAGE_DUEL_HALF_EXTENT_M: f32 = 13.0;
/// Seats in the page's match: the human plus one brain.
pub const PAGE_DUEL_SEATS: u8 = 2;

/// Where the page's duel starts: the two seats' feet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageDuelStart {
    /// The human seat's feet, meters.
    pub human_m: [f32; 3],
    /// The rival seat's feet, meters.
    pub rival_m: [f32; 3],
}

/// Why the page's duel could not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageDuelError {
    /// The arena refused the seed, the seat count or the extent.
    Refused,
}

/// The page's duel start: where the two seats stand when the match opens.
pub fn page_duel_start() -> Result<PageDuelStart, PageDuelError> {
    let states = initial_state(PAGE_DUEL_SEED, PAGE_DUEL_SEATS, PAGE_DUEL_HALF_EXTENT_M)
        .ok_or(PageDuelError::Refused)?;
    let human = states.first().ok_or(PageDuelError::Refused)?;
    let rival = states.get(1).ok_or(PageDuelError::Refused)?;
    Ok(PageDuelStart {
        human_m: human.pos,
        rival_m: rival.pos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena_kit::{kit_actors, KitActor, KitAsset};
    use engine_render::shoulder::{SHOULDER_BACK_M, SHOULDER_SIDE_M};

    /// The corridor's walls as the placement table holds them: each wall's own
    /// box, so a moved wall moves the corridor with it.
    fn wall_boxes(actors: &[KitActor]) -> Vec<((f32, f32), (f32, f32))> {
        actors
            .iter()
            .filter(|actor| actor.asset == KitAsset::Walls)
            .map(|actor| {
                let (x, z) = (actor.pos_m[0], actor.pos_m[2]);
                let (sx, sz) = (actor.size_m[0], actor.size_m[2]);
                ((x - sx * 0.5, x + sx * 0.5), (z - sz * 0.5, z + sz * 0.5))
            })
            .collect()
    }

    /// The page's duel starts where the camera can see it: both seats on the
    /// corridor's floor, the rival in front of the human, and the eye behind
    /// the human still inside the stone.
    #[test]
    fn the_page_duel_starts_in_the_kit_corridor_with_the_rival_in_front() {
        let actors = kit_actors().expect("embedded placement parses");
        let walls = wall_boxes(&actors);
        assert_eq!(walls.len(), 2, "the kit has a north and a south wall");
        // The wall with the smaller z stands north. The corridor runs between
        // their inner faces, and only where both walls stand.
        let (north, south) = if walls[0].1 .0 < walls[1].1 .0 {
            (walls[0], walls[1])
        } else {
            (walls[1], walls[0])
        };
        let x_min = north.0 .0.max(south.0 .0);
        let x_max = north.0 .1.min(south.0 .1);
        let z_min = north.1 .1;
        let z_max = south.1 .0;
        let start = page_duel_start().expect("the arena spawns the page's duel");
        let human = start.human_m;
        let rival = start.rival_m;
        for (who, feet) in [("human", human), ("rival", rival)] {
            assert!(
                feet[0] > x_min && feet[0] < x_max,
                "{who} stands on the corridor floor at x {}",
                feet[0]
            );
            assert!(
                feet[2] > z_min && feet[2] < z_max,
                "{who} stands on the corridor floor at z {}",
                feet[2]
            );
        }
        let gap = rival[2] - human[2];
        assert!(
            (3.0..7.0).contains(&gap),
            "the rival stands {gap} meters in front of the human"
        );
        assert!(
            (rival[0] - human[0]).abs() < 2.0,
            "the rival stands across the corridor, not off to the side"
        );
        // The eye sits behind the human and to its right, so it needs floor too.
        let eye = [human[0] + SHOULDER_SIDE_M, human[2] - SHOULDER_BACK_M];
        assert!(
            eye[0] > x_min && eye[0] < x_max && eye[1] > z_min && eye[1] < z_max,
            "the eye at {eye:?} stays inside the corridor"
        );
        // The stone stops the fighter, not the sim's own box: the walls the
        // fighter sees sit inside the arena the sim bounds.
        assert!(
            PAGE_DUEL_HALF_EXTENT_M > z_max.abs(),
            "the sim's walls sit past the corridor's stone"
        );
    }
}
