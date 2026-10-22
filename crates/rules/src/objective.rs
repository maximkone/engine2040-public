//! A game objective over the reference arena: a goal zone the player must reach, a player
//! that loses when it dies, and enemies that walk at the player.
//!
//! Consumer: the door's `level.build` grammar (kinds `player`, `enemy`, `goal`) and any
//! `play.run` that names a win or a loss. Data owner: this module. Units: millimetres,
//! fixed ticks, permille health. Determinism: integer arithmetic plus FNV-1a only, no
//! wall clock, no allocation per tick, so the same seed and the same tape give the same
//! outcome and the same digest.
//!
//! The rules never write the arena. Enemy chase is a wish input, the same shape a
//! controller writes (`engine_rules::host::Action::Wish`), so the reference arena and the
//! real arena both take it through their own input path.

use engine_core::Fnv64;

use crate::error::RulesError;
use crate::host::{ActorView, BTN_ATTACK};
use crate::limits::MAX_BODIES;
use crate::rules::{Rules, TickReport};
use crate::sim::{ScriptInput, Sim, Wall};

/// Wish magnitude an enemy chase uses each tick, percent of top speed.
pub const CHASE_WISH: i8 = 100;
/// Player slot in the two-actor default match (the tape's human seat).
pub const PLAYER_SLOT: u8 = 0;
/// Enemy slot in the two-actor default match.
pub const ENEMY_SLOT: u8 = 1;

/// An axis-aligned goal zone in millimetres, inclusive on all four sides. Reaching it
/// wins. The zone is a floor patch, never a body: the player stands inside it. The rect
/// body is the arena's own [`Wall`] (one owner for the edge math), so "inside the goal"
/// and "inside the wall" cannot drift apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GoalZone(Wall);

impl GoalZone {
    /// A zone from two corners in any order; edges are normalized so `x0 <= x1`.
    pub fn new(x0_mm: i32, z0_mm: i32, x1_mm: i32, z1_mm: i32) -> Self {
        Self(Wall::new(x0_mm, z0_mm, x1_mm, z1_mm))
    }

    /// True when the point stands on or inside the zone.
    pub fn contains(&self, x_mm: i32, z_mm: i32) -> bool {
        self.0.contains(x_mm, z_mm)
    }

    /// Side lengths `(east-west, north-south)` in millimetres.
    pub fn span_mm(&self) -> (i32, i32) {
        let rect = self.0;
        (
            rect.x1_mm.saturating_sub(rect.x0_mm),
            rect.z1_mm.saturating_sub(rect.z0_mm),
        )
    }

    /// The rectangle itself: west, north, east, south edges in millimetres.
    pub fn rect(&self) -> (i32, i32, i32, i32) {
        let rect = self.0;
        (rect.x0_mm, rect.z0_mm, rect.x1_mm, rect.z1_mm)
    }
}

/// How the objective stands after a tick. `Win` and `Lose` are final.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectiveOutcome {
    /// No side has met its condition yet.
    Running,
    /// The player stands in the goal zone while alive.
    Win,
    /// The player is dead, or the clock ran out with the goal unreached.
    Lose,
}

impl ObjectiveOutcome {
    /// Stable tag for receipts and digests.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Win => "win",
            Self::Lose => "lose",
        }
    }

    /// Tag byte folded into the digest; frozen.
    pub const fn tag(self) -> u8 {
        match self {
            Self::Running => 0,
            Self::Win => 1,
            Self::Lose => 2,
        }
    }

    /// True once the match has a verdict.
    pub const fn final_outcome(self) -> bool {
        matches!(self, Self::Win | Self::Lose)
    }
}

/// A goal match: the reference arena plus the objective read over it. The player is slot
/// 0 (tape inputs), enemies chase it every tick unless the caller's own input for that
/// slot says otherwise.
#[derive(Clone, Debug)]
pub struct ObjectiveMatch {
    sim: Sim,
    goal: GoalZone,
    player: u8,
    enemies: Vec<u8>,
    outcome: ObjectiveOutcome,
    ticks: u32,
    /// True when the player seat walks at the goal on its own (a caller with no tape).
    autopilot: bool,
}

impl ObjectiveMatch {
    /// The two-actor default: player slot 0, one enemy on slot 1, `seed` on the ring
    /// layout.
    pub fn duel(seed: u64, goal: GoalZone) -> Self {
        Self::new(seed, goal, &[ENEMY_SLOT], 2)
    }

    /// `fighters` actors (2..=64) with `enemies` chasing `player`.
    pub fn new(seed: u64, goal: GoalZone, enemies: &[u8], fighters: usize) -> Self {
        Self {
            sim: Sim::new(fighters.clamp(2, MAX_BODIES), seed),
            goal,
            player: PLAYER_SLOT,
            enemies: enemies
                .iter()
                .copied()
                .filter(|slot| usize::from(*slot) < fighters.clamp(2, MAX_BODIES))
                .collect(),
            outcome: ObjectiveOutcome::Running,
            ticks: 0,
            autopilot: false,
        }
    }

    /// The player seat walks at the goal on its own when the caller supplies no input
    /// for it. Returns the match so a builder chain reads in one expression.
    pub fn with_autopilot(mut self, on: bool) -> Self {
        self.autopilot = on;
        self
    }

    /// True when the player seat walks at the goal on its own.
    pub fn autopilot(&self) -> bool {
        self.autopilot
    }

    /// The goal zone.
    pub fn goal(&self) -> GoalZone {
        self.goal
    }

    /// The player slot.
    pub fn player(&self) -> u8 {
        self.player
    }

    /// The chasing slots, in the order given.
    pub fn enemies(&self) -> &[u8] {
        &self.enemies
    }

    /// The verdict so far.
    pub fn outcome(&self) -> ObjectiveOutcome {
        self.outcome
    }

    /// Ticks run.
    pub fn ticks(&self) -> u32 {
        self.ticks
    }

    /// The walled scene: a wall spans the room, so a level can put one between the player
    /// and the goal and the walk cannot cross it. A fighter never ends a tick inside it;
    /// one already inside is pushed out at once.
    pub fn with_wall(mut self, wall: Wall) -> Self {
        self.sim = self.sim.with_wall(wall);
        self
    }

    /// The static wall, if any.
    pub fn wall(&self) -> Option<Wall> {
        self.sim.wall()
    }

    /// The arena under the objective.
    pub fn sim(&self) -> &Sim {
        &self.sim
    }

    /// Start the player and the enemies from authored poses in millimetres. Slots the
    /// caller does not name stay on the ring layout. This is how a built level's floor,
    /// player and goal seats become the match's starting spots, so the match plays the
    /// level a caller authored, not a ring. Returns how many seats moved.
    pub fn place_actors(&mut self, poses: &[(u8, i32, i32)]) -> u32 {
        let mut placed = 0;
        for (slot, x_mm, z_mm) in poses {
            if self.sim.place(*slot, *x_mm, *z_mm) {
                placed += 1;
            }
        }
        // A seat placed inside the goal at tick 0 has already won; read it now so the
        // caller's first step reports the verdict instead of stepping past it.
        self.read_outcome();
        placed
    }

    /// The player's view; `None` when the layout holds no such slot.
    pub fn player_view(&self) -> Option<&ActorView> {
        self.sim.fighters().get(usize::from(self.player))
    }

    /// True when no verdict has landed yet.
    pub fn running(&self) -> bool {
        !self.outcome.final_outcome()
    }

    /// FNV-1a 64 over the tick count, the verdict tag and the arena digest.
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_u32(self.ticks);
        hasher.write_u8(self.outcome.tag());
        hasher.write_u64(self.sim.digest());
        hasher.finish()
    }

    /// Read the verdict from the current arena state. Win beats Lose: a player that
    /// reached the goal on the tick it died still won, because the goal came first.
    fn read_outcome(&mut self) {
        if self.outcome.final_outcome() {
            return;
        }
        let Some(player) = self.sim.fighters().get(usize::from(self.player)).copied() else {
            return;
        };
        if player.alive && self.goal.contains(player.x_mm, player.z_mm) {
            self.outcome = ObjectiveOutcome::Win;
        } else if !player.alive {
            self.outcome = ObjectiveOutcome::Lose;
        }
    }

    /// Chase inputs for every enemy, in slot order: a full wish toward the player on each
    /// axis, 0 on an axis already level. An enemy that is not alive, staggered or knocked
    /// down is given no wish (the arena drops it anyway, this keeps the tape quiet).
    fn chase_inputs(&self, tick: u32) -> Vec<ScriptInput> {
        let Some(player) = self.sim.fighters().get(usize::from(self.player)).copied() else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(self.enemies.len());
        for slot in &self.enemies {
            let Some(view) = self.sim.fighters().get(usize::from(*slot)).copied() else {
                continue;
            };
            if !view.alive || view.knockdown > 0 || view.react > 0 {
                continue;
            }
            let step = |from: i32, to: i32| -> i8 {
                let gap = i64::from(to) - i64::from(from);
                let wish = if gap == 0 { 0 } else { CHASE_WISH };
                if gap < 0 {
                    -wish
                } else {
                    wish
                }
            };
            out.push(ScriptInput {
                tick,
                fighter: *slot,
                mx: step(view.x_mm, player.x_mm),
                mz: step(view.z_mm, player.z_mm),
                facing: view.facing_cdeg,
                // In reach, the enemy swings: that is how a chase turns into a loss. Out
                // of reach it only walks, so a level with a far goal stays a walk.
                buttons: if crate::host::distance_mm(&view, &player) <= crate::sim::REACH_MM {
                    crate::host::BTN_ATTACK
                } else {
                    0
                },
            });
        }
        out
    }

    /// Run one tick: the caller's inputs for the player seat plus the chase wishes, then
    /// the arena step, then the verdict. A finished match ignores further steps and
    /// returns an empty report, so a caller cannot change a landed verdict.
    pub fn step(
        &mut self,
        inputs: &[ScriptInput],
        rules: &mut Rules,
    ) -> Result<TickReport, RulesError> {
        if self.outcome.final_outcome() {
            return Ok(TickReport::default());
        }
        let tick = self.sim.tick();
        let mut row = self.chase_inputs(tick);
        let caller_drives_player = inputs
            .iter()
            .any(|input| input.tick == tick && input.fighter == self.player);
        if self.autopilot && !caller_drives_player {
            // A caller with no tape of its own still needs a player that moves, or the
            // goal can never be reached. The autopilot walks at the goal at the same
            // full wish the chase uses.
            if let Some(seat) = self.goal_seat(tick) {
                row.push(seat);
            }
        }
        // The caller's own input for a slot wins: a tape seat drives itself, and a caller
        // may hand the enemy a scripted button press.
        for input in inputs.iter().filter(|input| input.tick == tick) {
            row.retain(|chase| chase.fighter != input.fighter);
            row.push(ScriptInput {
                mx: input.mx,
                mz: input.mz,
                ..*input
            });
        }
        let report = self.sim.step(&row, rules)?;
        self.ticks = self.sim.tick();
        self.read_outcome();
        Ok(report)
    }

    /// The player seat's wish toward the goal centre, or `None` when the player is not
    /// alive, staggered or already inside the zone.
    fn goal_seat(&self, tick: u32) -> Option<ScriptInput> {
        let player = self.sim.fighters().get(usize::from(self.player)).copied()?;
        if !player.alive || player.knockdown > 0 || player.react > 0 {
            return None;
        }
        let (west, north, east, south) = self.goal.rect();
        let (cx, cz) = (
            west.saturating_add(east) / 2,
            north.saturating_add(south) / 2,
        );
        let wish = |from: i32, to: i32| -> i8 {
            let gap = i64::from(to) - i64::from(from);
            match gap.cmp(&0) {
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Less => -CHASE_WISH,
                std::cmp::Ordering::Greater => CHASE_WISH,
            }
        };
        Some(ScriptInput {
            tick,
            fighter: self.player,
            mx: wish(player.x_mm, cx),
            mz: wish(player.z_mm, cz),
            facing: player.facing_cdeg,
            buttons: 0,
        })
    }

    /// Step until the verdict lands or `limit` further ticks have run. A match that runs
    /// out of ticks without reaching the goal has failed its objective: the clock is the
    /// second lose condition, beside death, so a level with a wall in the way reads a loss
    /// instead of running forever. Returns the ticks run, `0` when it was already decided.
    pub fn run_until_decided(
        &mut self,
        limit: u32,
        inputs: &[ScriptInput],
        rules: &mut Rules,
    ) -> Result<u32, RulesError> {
        let mut ran = 0;
        while self.running() && ran < limit {
            self.step(inputs, rules)?;
            ran += 1;
        }
        if self.running() && ran >= limit {
            // Out of time with the goal unreached: the objective failed.
            self.outcome = ObjectiveOutcome::Lose;
        }
        Ok(ran)
    }
}

/// A tape that walks the player east at full wish for `ticks` ticks. Used by tests and by
/// the door's own play proof.
pub fn walk_east(ticks: u32) -> Vec<ScriptInput> {
    held(PLAYER_SLOT, CHASE_WISH, 0, ticks)
}

/// A tape that walks the player west at full wish for `ticks` ticks.
pub fn walk_west(ticks: u32) -> Vec<ScriptInput> {
    held(PLAYER_SLOT, -CHASE_WISH, 0, ticks)
}

/// A tape that presses attack on `slot` every tick for `ticks` ticks.
pub fn attack_slot(slot: u8, ticks: u32) -> Vec<ScriptInput> {
    held(slot, 0, BTN_ATTACK, ticks)
}

/// One held wish and button set on one slot for `ticks` ticks: the shape both tapes share.
fn held(fighter: u8, mx: i8, buttons: u8, ticks: u32) -> Vec<ScriptInput> {
    (0..ticks)
        .map(|tick| ScriptInput {
            tick,
            fighter,
            mx,
            mz: 0,
            facing: 0,
            buttons,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Limits;

    fn rules() -> Rules {
        Rules::new(Limits::default())
    }

    #[test]
    fn a_run_out_of_time_with_the_goal_unreached_loses() {
        // The goal sits across the room from the ring layout, and the tape never moves the
        // player: the clock is what decides it.
        let far = GoalZone::new(9_000, 9_000, 9_500, 9_500);
        let mut match_ = ObjectiveMatch::duel(2040, far);
        let ran = match_
            .run_until_decided(40, &held(PLAYER_SLOT, 0, 0, 40), &mut rules())
            .expect("steps");
        assert_eq!(ran, 40);
        assert_eq!(
            match_.outcome(),
            ObjectiveOutcome::Lose,
            "out of time with the goal unreached"
        );
    }

    #[test]
    fn goal_zone_normalizes_and_includes_its_edges() {
        let zone = GoalZone::new(2_000, 1_000, -2_000, -1_000);
        assert_eq!(zone.rect(), (-2_000, -1_000, 2_000, 1_000));
        assert!(zone.contains(-2_000, -1_000));
        assert!(zone.contains(2_000, 1_000));
        assert!(zone.contains(0, 0));
        assert!(!zone.contains(2_001, 0));
        assert!(!zone.contains(0, -1_001));
        assert_eq!(zone.span_mm(), (4_000, 2_000));
    }

    #[test]
    fn reaching_the_goal_wins_and_the_verdict_freezes() {
        // A zone the player meets by walking east from the ring layout (~-1.6 m).
        let zone = GoalZone::new(-300, -3_000, 3_000, 3_000);
        let mut match_ = ObjectiveMatch::duel(2040, zone);
        assert!(match_.running());
        let inputs = walk_east(600);
        let ran = match_
            .run_until_decided(600, &inputs, &mut rules())
            .expect("the match steps");
        assert_eq!(match_.outcome(), ObjectiveOutcome::Win, "{ran} ticks ran");
        assert!(ran > 0 && ran < 600, "the win comes mid-run: {ran}");
        // Further steps change nothing and keep the digest.
        let settled = match_.digest();
        let ticks = match_.ticks();
        match_
            .step(&[], &mut rules())
            .expect("a finished match steps");
        assert_eq!(match_.digest(), settled);
        assert_eq!(match_.ticks(), ticks);
    }

    #[test]
    fn a_far_goal_is_out_of_time_and_that_loses() {
        // Walking east for 120 ticks does not reach a goal in the far corner, so the clock
        // decides it: the objective failed without the player dying.
        let far = GoalZone::new(9_000, 9_000, 9_500, 9_500);
        let mut walking = ObjectiveMatch::duel(7, far);
        let ran = walking
            .run_until_decided(120, &walk_east(600), &mut rules())
            .expect("steps");
        assert_eq!(ran, 120);
        assert_eq!(walking.outcome(), ObjectiveOutcome::Lose);
        assert!(
            walking.player_view().is_some_and(|view| view.alive),
            "the player was alive: it ran out of ticks, not health"
        );

        // The enemy swings at reach until the player dies: death is the other loss.
        let mut bout = ObjectiveMatch::duel(2040, far);
        let swings = attack_slot(ENEMY_SLOT, 400);
        let mut died = false;
        for _ in 0..400 {
            bout.step(&swings, &mut rules()).expect("steps");
            if bout.outcome() == ObjectiveOutcome::Lose {
                died = true;
                break;
            }
        }
        assert!(died, "the enemy's swings killed the player");
        assert!(!bout.player_view().expect("player slot").alive);
    }

    #[test]
    fn an_enemy_in_reach_swings_without_a_tape_and_wins_the_fight() {
        // A far goal and a player that only walks: the chasing enemy reaches it and hits.
        let far = GoalZone::new(9_000, 9_000, 9_500, 9_500);
        let mut bout = ObjectiveMatch::duel(2040, far);
        let ran = bout
            .run_until_decided(600, &[], &mut rules())
            .expect("steps");
        assert_eq!(bout.outcome(), ObjectiveOutcome::Lose, "{ran} ticks ran");
        assert!(bout.player_view().expect("player slot").hp_permille < 1_000);
    }

    #[test]
    fn the_enemy_closes_on_the_player() {
        let far = GoalZone::new(9_000, 9_000, 9_500, 9_500);
        let mut chase = ObjectiveMatch::duel(2040, far);
        let start = chase
            .sim()
            .fighters()
            .get(usize::from(ENEMY_SLOT))
            .copied()
            .expect("enemy slot");
        let inputs = walk_east(60);
        for _ in 0..60 {
            chase.step(&inputs, &mut rules()).expect("steps");
        }
        let end = chase
            .sim()
            .fighters()
            .get(usize::from(ENEMY_SLOT))
            .copied()
            .expect("enemy slot");
        assert!(
            end.x_mm > start.x_mm,
            "the enemy walked east after the player: {} -> {}",
            start.x_mm,
            end.x_mm
        );
        let gap = |view: ActorView| {
            let me = chase.player_view().copied().expect("player slot");
            crate::host::distance_mm(&view, &me)
        };
        assert!(
            gap(end) <= gap(start) + 300,
            "the enemy kept pace: {} -> {}",
            gap(start),
            gap(end)
        );
    }

    #[test]
    fn same_seed_same_tape_same_digest_and_outcome() {
        let zone = GoalZone::new(-300, -3_000, 3_000, 3_000);
        let run = || {
            let mut match_ = ObjectiveMatch::duel(2040, zone);
            match_
                .run_until_decided(600, &walk_east(600), &mut rules())
                .expect("steps");
            (match_.outcome(), match_.digest(), match_.ticks())
        };
        let first = run();
        assert_eq!(first, run());
        assert_eq!(first.0, ObjectiveOutcome::Win);
    }

    #[test]
    fn the_autopilot_walks_to_the_goal_and_wins() {
        // The zone is east of the ring layout; the autopilot alone reaches it.
        let zone = GoalZone::new(1_000, -500, 2_000, 500);
        let mut match_ = ObjectiveMatch::duel(2040, zone).with_autopilot(true);
        assert!(match_.autopilot());
        let ran = match_
            .run_until_decided(600, &[], &mut rules())
            .expect("steps");
        assert_eq!(match_.outcome(), ObjectiveOutcome::Win, "{ran} ticks ran");
        // A caller's own seat for the player wins over the autopilot: walking west
        // never reaches a zone in the east, so the chase catches the player instead.
        let mut driven = ObjectiveMatch::duel(2040, zone).with_autopilot(true);
        driven
            .run_until_decided(600, &walk_west(600), &mut rules())
            .expect("steps");
        assert_ne!(
            driven.outcome(),
            ObjectiveOutcome::Win,
            "the caller's wish kept the player out of the goal"
        );
    }

    #[test]
    fn an_out_of_range_enemy_slot_is_dropped() {
        let match_ = ObjectiveMatch::new(1, GoalZone::new(0, 0, 100, 100), &[1, 60], 2);
        assert_eq!(match_.enemies(), &[ENEMY_SLOT]);
    }

    #[test]
    fn seats_placed_from_a_level_start_the_match_there() {
        // The goal sits where a level would put it; the player starts on its edge and the
        // enemy far west. Placing the seats wins at once, so the first step reports it.
        let zone = GoalZone::new(1_000, -500, 2_000, 500);
        let mut match_ = ObjectiveMatch::duel(2040, zone);
        let placed = match_.place_actors(&[(PLAYER_SLOT, 1_500, 0), (ENEMY_SLOT, -2_000, 0)]);
        assert_eq!(placed, 2, "both seats moved");
        assert_eq!(
            match_.outcome(),
            ObjectiveOutcome::Win,
            "the seat is in the zone"
        );
        let player = match_.player_view().copied().expect("player slot");
        assert_eq!((player.x_mm, player.z_mm), (1_500, 0));

        // A seat outside the zone runs until it walks in.
        let mut walking = ObjectiveMatch::duel(2040, zone);
        walking.place_actors(&[(PLAYER_SLOT, -2_000, 0), (ENEMY_SLOT, 300, 0)]);
        assert_eq!(walking.outcome(), ObjectiveOutcome::Running);
        let ran = walking
            .run_until_decided(600, &walk_east(600), &mut rules())
            .expect("steps");
        assert_eq!(walking.outcome(), ObjectiveOutcome::Win, "{ran} ticks");
        // An out-of-range slot is not placed and not counted.
        assert_eq!(walking.place_actors(&[(9, 0, 0)]), 0);
    }
}
