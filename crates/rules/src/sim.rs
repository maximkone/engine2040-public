//! A small integer reference arena that applies rule actions. It is the replay proof's
//! simulation and the shape the real arena adapter follows: build a `World`, call
//! `Rules::tick`, apply the actions in order. It owns no game balance.

use engine_core::{mix64, Fnv64};

use crate::error::RulesError;
use crate::fixtures::ring_world;
use crate::host::{Action, ActorView, World, BTN_ATTACK, BTN_BLOCK, BTN_DODGE};
use crate::limits::MAX_FIGHTERS;
use crate::rules::{Rules, TickReport};

/// Attack lasts this many ticks; it lands on `HIT_AGE`.
const ATTACK_TICKS: u32 = 20;
const HIT_AGE: u32 = 8;
const REACH_MM: i64 = 1_500;
const ATTACK_COST: u16 = 100;
const BASE_DAMAGE: u16 = 120;
const ARENA_MM: i32 = 20_000;

/// Ticks in one FPS round (10 s at 60 Hz, the same cadence as the duel proof).
pub const FPS_ROUND_TICKS: u32 = 600;
/// Round wins that take an FPS match (best of 3, the short CQB slice).
pub const FPS_WINS_FOR_MATCH: u32 = 2;

/// Winner of one FPS round. Caller: `Sim::step` in this file folds the winner of
/// every `FPS_ROUND_TICKS` boundary into `FpsMatch`; the arena and the door read
/// scripted CQB shots through the unchanged `Rules::tick` plus tape path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FpsRoundWinner {
    /// Every living suspect is down and at least one officer stands.
    Officers,
    /// Every living officer is down and at least one suspect stands.
    Suspects,
    /// Both sides still stand, nobody stands, or only neutrals stand: replay.
    Tie,
}

/// Best-of-3 fold of FPS rounds: first to `FPS_WINS_FOR_MATCH`; ties replay and
/// score nothing. Pure integer counts; the observer lives beside the sim digest so
/// old tapes replay byte-identical. Caller: `Sim::step` in this file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FpsMatch {
    rounds: u32,
    wins_officers: u32,
    wins_suspects: u32,
    ties: u32,
}

impl FpsMatch {
    /// Empty match. Pure.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            rounds: 0,
            wins_officers: 0,
            wins_suspects: 0,
            ties: 0,
        }
    }

    /// Fold one verdict (tie: round counted, no score).
    pub fn apply(&mut self, winner: FpsRoundWinner) {
        self.rounds = self.rounds.saturating_add(1);
        match winner {
            FpsRoundWinner::Officers => {
                self.wins_officers = self.wins_officers.saturating_add(1);
            }
            FpsRoundWinner::Suspects => {
                self.wins_suspects = self.wins_suspects.saturating_add(1);
            }
            FpsRoundWinner::Tie => {
                self.ties = self.ties.saturating_add(1);
            }
        }
    }

    /// Winner once a side reaches `FPS_WINS_FOR_MATCH`. Pure.
    #[must_use]
    pub fn winner(self) -> Option<FpsRoundWinner> {
        if self.wins_officers >= FPS_WINS_FOR_MATCH {
            Some(FpsRoundWinner::Officers)
        } else if self.wins_suspects >= FPS_WINS_FOR_MATCH {
            Some(FpsRoundWinner::Suspects)
        } else {
            None
        }
    }

    /// Rounds played including tie replays. Pure.
    #[must_use]
    pub fn rounds_played(self) -> u32 {
        self.rounds
    }

    /// Round wins `(officers, suspects)`. Pure.
    #[must_use]
    pub fn wins(self) -> (u32, u32) {
        (self.wins_officers, self.wins_suspects)
    }

    /// Tie replays so far. Pure.
    #[must_use]
    pub fn ties(self) -> u32 {
        self.ties
    }

    /// FNV-1a 64 over the counts, in field order. Pure.
    #[must_use]
    pub fn digest(self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_u32(self.rounds);
        hasher.write_u32(self.wins_officers);
        hasher.write_u32(self.wins_suspects);
        hasher.write_u32(self.ties);
        mix64(hasher.finish())
    }
}

/// Decide one FPS round from who stands: team 0 are officers, team 1 are suspects,
/// higher teams are neutrals and never decide. No allocation, stable order free
/// (counts only), no clock. Caller: `Sim::step` in this file.
#[must_use]
pub fn decide_fps_round(fighters: &[ActorView]) -> FpsRoundWinner {
    let mut officers = 0_u32;
    let mut suspects = 0_u32;
    for view in fighters {
        if !view.alive {
            continue;
        }
        if view.team == 0 {
            officers = officers.saturating_add(1);
        } else if view.team == 1 {
            suspects = suspects.saturating_add(1);
        }
    }
    if officers > 0 && suspects == 0 {
        FpsRoundWinner::Officers
    } else if suspects > 0 && officers == 0 {
        FpsRoundWinner::Suspects
    } else {
        FpsRoundWinner::Tie
    }
}

/// One fighter's command for one tick: the 10-byte combat-tape input layout, slot up to 63.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptInput {
    /// Zero-based fixed tick.
    pub tick: u32,
    /// Actor slot.
    pub fighter: u8,
    /// Forward wish, `-100..=100`.
    pub mx: i8,
    /// Right wish, `-100..=100`.
    pub mz: i8,
    /// Facing in centidegrees.
    pub facing: i16,
    /// Button bits.
    pub buttons: u8,
}

impl ScriptInput {
    /// Bytes per input on a tape.
    pub const BYTES: usize = 10;

    /// Little-endian bytes (same layout as the combat tape's input).
    pub fn append_bytes(self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.tick.to_le_bytes());
        bytes.push(self.fighter);
        bytes.extend_from_slice(&self.mx.to_le_bytes());
        bytes.extend_from_slice(&self.mz.to_le_bytes());
        bytes.extend_from_slice(&self.facing.to_le_bytes());
        bytes.push(self.buttons);
    }
}

/// One static wall: an axis-aligned box in millimetres, inclusive on all sides. The
/// reference arena owns no game balance; this is the walled scene the rules drive with zero
/// providers (only `Rules` and this static world, no arena or door).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wall {
    /// West edge, millimetres.
    pub x0_mm: i32,
    /// North edge, millimetres.
    pub z0_mm: i32,
    /// East edge, millimetres.
    pub x1_mm: i32,
    /// South edge, millimetres.
    pub z1_mm: i32,
}

impl Wall {
    /// A wall from two corners in any order; the edges are normalized so `x0 <= x1`.
    pub fn new(x0_mm: i32, z0_mm: i32, x1_mm: i32, z1_mm: i32) -> Self {
        Self {
            x0_mm: x0_mm.min(x1_mm),
            z0_mm: z0_mm.min(z1_mm),
            x1_mm: x0_mm.max(x1_mm),
            z1_mm: z0_mm.max(z1_mm),
        }
    }

    /// True when the point is on or inside the wall.
    pub fn contains(&self, x_mm: i32, z_mm: i32) -> bool {
        x_mm >= self.x0_mm && x_mm <= self.x1_mm && z_mm >= self.z0_mm && z_mm <= self.z1_mm
    }
}

/// The reference arena.
#[derive(Clone, Debug)]
pub struct Sim {
    tick: u32,
    seed: u64,
    fighters: Vec<ActorView>,
    wall: Option<Wall>,
    /// FPS observer: round wins folded at every `FPS_ROUND_TICKS` boundary. Kept out
    /// of `digest` so old tapes replay byte-identical; read it via `fps_match`.
    fps: FpsMatch,
}

impl Sim {
    /// `count` fighters (1..=64) on the ring layout, two teams by id parity.
    pub fn new(count: usize, seed: u64) -> Self {
        let world = ring_world(count, 0, seed);
        let fighters = world
            .fighters
            .into_iter()
            .map(|mut view| {
                view.buttons = 0;
                view.blocking = false;
                view.attacking = false;
                view.dodging = false;
                view.hp_permille = 1_000;
                view.stamina = 1_000;
                view
            })
            .collect();
        Self {
            tick: 0,
            seed,
            fighters,
            wall: None,
            fps: FpsMatch::new(),
        }
    }

    /// The walled scene: fighters can never end a tick inside `wall` (they are pushed out
    /// along the least-penetrated edge). Positions the layout already placed inside are
    /// resolved at once, so the scene starts clean.
    pub fn with_wall(mut self, wall: Wall) -> Self {
        self.wall = Some(wall);
        self.resolve_wall();
        self
    }

    /// The static wall, if any.
    pub fn wall(&self) -> Option<Wall> {
        self.wall
    }

    /// The FPS observer so far (rounds folded at every `FPS_ROUND_TICKS` boundary).
    pub fn fps_match(&self) -> FpsMatch {
        self.fps
    }

    /// Round winner on the current fighters (observer read, changes nothing).
    pub fn fps_round_winner(&self) -> FpsRoundWinner {
        decide_fps_round(&self.fighters)
    }

    /// Next tick to run.
    pub fn tick(&self) -> u32 {
        self.tick
    }

    /// Actors, in slot order.
    pub fn fighters(&self) -> &[ActorView] {
        &self.fighters
    }

    /// The world rules see now.
    pub fn world(&self) -> World {
        World {
            tick: self.tick,
            seed: self.seed,
            fighters: self.fighters.clone(),
        }
    }

    /// FNV-1a 64 over every fighter field and the tick.
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_u32(self.tick);
        hasher.write_u64(self.seed);
        for view in &self.fighters {
            hasher.write_u8(view.id);
            hasher.write_u8(view.team);
            hasher.write_bool(view.alive);
            hasher.write_i32(view.x_mm);
            hasher.write_i32(view.z_mm);
            hasher.write_u16(view.hp_permille);
            hasher.write_u16(view.stamina);
            hasher.write_u32(view.attack_age);
            hasher.write_bool(view.attacking);
            hasher.write_bool(view.blocking);
            hasher.write_bool(view.dodging);
            hasher.write_u32(view.react);
            hasher.write_u32(view.knockdown);
            hasher.write_u32(view.shield);
            hasher.write_u8(view.buttons);
        }
        mix64(hasher.finish())
    }

    /// Run one tick: inputs, native attacks, rules, actions, timers.
    ///
    /// `inputs` holds only this tick's commands; a fighter with none idles this tick.
    pub fn step(
        &mut self,
        inputs: &[ScriptInput],
        rules: &mut Rules,
    ) -> Result<TickReport, RulesError> {
        self.apply_inputs(inputs);
        self.native_attacks();
        let report = rules.tick(&self.world())?;
        for emitted in &report.actions {
            self.apply(emitted.action);
        }
        self.resolve_wall();
        self.advance_timers();
        self.tick += 1;
        if self.tick.is_multiple_of(FPS_ROUND_TICKS) {
            let winner = decide_fps_round(&self.fighters);
            self.fps.apply(winner);
        }
        Ok(report)
    }

    /// Push every fighter inside the wall back out along the least-penetrated edge (west,
    /// east, north, south in that tie order). Integer-only and order-free: each fighter is
    /// resolved on its own, so the result is the same on every machine.
    fn resolve_wall(&mut self) {
        let Some(wall) = self.wall else {
            return;
        };
        for view in &mut self.fighters {
            if !wall.contains(view.x_mm, view.z_mm) {
                continue;
            }
            let to_west = view.x_mm.saturating_sub(wall.x0_mm);
            let to_east = wall.x1_mm.saturating_sub(view.x_mm);
            let to_north = view.z_mm.saturating_sub(wall.z0_mm);
            let to_south = wall.z1_mm.saturating_sub(view.z_mm);
            let mut edge = 0;
            let mut least = to_west;
            if to_east < least {
                edge = 1;
                least = to_east;
            }
            if to_north < least {
                edge = 2;
                least = to_north;
            }
            if to_south < least {
                edge = 3;
            }
            match edge {
                0 => view.x_mm = wall.x0_mm.saturating_sub(1),
                1 => view.x_mm = wall.x1_mm.saturating_add(1),
                2 => view.z_mm = wall.z0_mm.saturating_sub(1),
                _ => view.z_mm = wall.z1_mm.saturating_add(1),
            }
        }
    }

    fn apply_inputs(&mut self, inputs: &[ScriptInput]) {
        for view in &mut self.fighters {
            view.buttons = 0;
            view.mx = 0;
            view.mz = 0;
        }
        for input in inputs.iter().filter(|input| input.tick == self.tick) {
            let Some(view) = self.fighters.get_mut(usize::from(input.fighter)) else {
                continue;
            };
            if !view.alive {
                continue;
            }
            view.buttons = input.buttons & (BTN_ATTACK | BTN_BLOCK | BTN_DODGE);
            view.mx = input.mx.clamp(-100, 100);
            view.mz = input.mz.clamp(-100, 100);
            view.facing_cdeg = input.facing.clamp(-18_000, 18_000);
            view.blocking = view.buttons & BTN_BLOCK != 0 && view.knockdown == 0;
            let free = view.react == 0 && view.knockdown == 0 && !view.attacking;
            if view.buttons & BTN_ATTACK != 0 && free && view.stamina >= ATTACK_COST {
                view.attacking = true;
                view.attack_age = 0;
                view.stamina -= ATTACK_COST;
            }
            view.dodging = view.buttons & BTN_DODGE != 0 && view.react == 0;
            if view.react == 0 && view.knockdown == 0 {
                view.x_mm = (view.x_mm + i32::from(view.mx) * 3).clamp(-ARENA_MM, ARENA_MM);
                view.z_mm = (view.z_mm + i32::from(view.mz) * 3).clamp(-ARENA_MM, ARENA_MM);
            }
        }
    }

    /// The sim's own attack: on `HIT_AGE` the attacker hits the nearest foe in reach.
    fn native_attacks(&mut self) {
        let mut hits: Vec<(u8, u8)> = Vec::new();
        for me in self
            .fighters
            .iter()
            .filter(|v| v.alive && v.attacking && v.attack_age == HIT_AGE)
        {
            let mut best: Option<(i64, u8)> = None;
            for foe in self
                .fighters
                .iter()
                .filter(|o| o.alive && o.team != me.team)
            {
                let d = crate::host::distance_mm(me, foe);
                if d <= REACH_MM && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, foe.id));
                }
            }
            if let Some((_, foe)) = best {
                hits.push((me.id, foe));
            }
        }
        for (_, foe) in hits {
            let blocking = self.fighters[usize::from(foe)].blocking;
            let damage = if blocking {
                BASE_DAMAGE / 6
            } else {
                BASE_DAMAGE
            };
            self.apply(Action::Damage {
                target: foe,
                permille: damage,
            });
        }
    }

    fn apply(&mut self, action: Action) {
        let slot = |target: u8| usize::from(target).min(MAX_FIGHTERS - 1);
        match action {
            Action::Damage { target, permille } => {
                if let Some(view) = self.fighters.get_mut(slot(target)) {
                    let amount = if view.shield > 0 {
                        permille / 2
                    } else {
                        permille
                    };
                    view.hp_permille = view.hp_permille.saturating_sub(amount);
                    if view.hp_permille == 0 {
                        view.alive = false;
                    }
                }
            }
            Action::Heal { target, permille } => {
                if let Some(view) = self.fighters.get_mut(slot(target)).filter(|v| v.alive) {
                    view.hp_permille = view.hp_permille.saturating_add(permille).min(1_000);
                }
            }
            Action::Stun { target, ticks } => {
                if let Some(view) = self.fighters.get_mut(slot(target)) {
                    view.react = view.react.max(u32::from(ticks));
                }
            }
            Action::Push {
                target,
                dx_mm,
                dz_mm,
            } => {
                if let Some(view) = self.fighters.get_mut(slot(target)) {
                    view.x_mm = view.x_mm.saturating_add(dx_mm).clamp(-ARENA_MM, ARENA_MM);
                    view.z_mm = view.z_mm.saturating_add(dz_mm).clamp(-ARENA_MM, ARENA_MM);
                }
            }
            Action::DrainStamina { target, amount } => {
                if let Some(view) = self.fighters.get_mut(slot(target)) {
                    view.stamina = view.stamina.saturating_sub(amount);
                }
            }
            Action::Shield { target, ticks } => {
                if let Some(view) = self.fighters.get_mut(slot(target)) {
                    view.shield = view.shield.max(u32::from(ticks));
                }
            }
            // Input actions drive a real arena slot (`arena::RulesController`); this reference
            // arena takes its inputs from the tape only.
            Action::Wish { .. } | Action::Face { .. } | Action::Press { .. } => {}
        }
    }

    fn advance_timers(&mut self) {
        for view in &mut self.fighters {
            view.react = view.react.saturating_sub(1);
            view.knockdown = view.knockdown.saturating_sub(1);
            view.shield = view.shield.saturating_sub(1);
            if view.attacking {
                view.attack_age += 1;
                if view.attack_age >= ATTACK_TICKS {
                    view.attacking = false;
                    view.attack_age = 0;
                }
            }
            view.stamina = view.stamina.saturating_add(2).min(1_000);
        }
    }
}

/// Deterministic scripted inputs: every fighter acts on a staggered cadence from `seed`, so
/// blocking, attacking and moving all occur. Sorted by `(tick, fighter)`.
pub fn scripted_inputs(count: usize, ticks: u32, seed: u64) -> Vec<ScriptInput> {
    let count = count.clamp(1, MAX_FIGHTERS);
    let mut inputs = Vec::new();
    for tick in 0..ticks {
        for index in 0..count {
            let id = u8::try_from(index).unwrap_or(u8::MAX);
            let roll = mix64(seed ^ (u64::from(tick) << 8) ^ u64::from(id));
            // Roughly three of four fighters act each tick; the rest idle.
            if roll & 3 == 0 {
                continue;
            }
            let buttons = match (roll >> 4) & 7 {
                0..=2 => BTN_BLOCK,
                3 | 4 => BTN_ATTACK | BTN_BLOCK,
                5 => BTN_ATTACK,
                _ => 0,
            };
            let wish = |shift: u32| i8::try_from(((roll >> shift) & 0x3f) as i64 - 32).unwrap_or(0);
            inputs.push(ScriptInput {
                tick,
                fighter: id,
                mx: wish(8),
                mz: wish(14),
                facing: i16::try_from(((roll >> 20) & 0x7fff) as i64 - 16_384).unwrap_or(0),
                buttons,
            });
        }
    }
    inputs
}

#[cfg(test)]
mod sim_wall_tests {
    use super::{scripted_inputs, ScriptInput, Wall};

    #[test]
    fn wall_new_normalizes_reversed_corners() {
        assert_eq!(
            Wall::new(1_000, 100, -1_000, -100),
            Wall::new(-1_000, -100, 1_000, 100)
        );
        let wall = Wall::new(1_000, 100, -1_000, -100);
        assert_eq!(
            (wall.x0_mm, wall.z0_mm, wall.x1_mm, wall.z1_mm),
            (-1_000, -100, 1_000, 100)
        );
    }

    #[test]
    fn wall_contains_is_inclusive_on_all_sides() {
        let wall = Wall::new(-1_000, -100, 1_000, 100);
        assert!(wall.contains(-1_000, -100));
        assert!(wall.contains(1_000, 100));
        assert!(wall.contains(0, 0));
        assert!(!wall.contains(-1_001, 0));
        assert!(!wall.contains(1_001, 0));
        assert!(!wall.contains(0, -101));
        assert!(!wall.contains(0, 101));
    }

    #[test]
    fn script_input_appends_ten_little_endian_bytes() {
        let input = ScriptInput {
            tick: 1,
            fighter: 2,
            mx: 3,
            mz: -4,
            facing: 258,
            buttons: 7,
        };
        assert_eq!(ScriptInput::BYTES, 10);
        let mut bytes = Vec::new();
        input.append_bytes(&mut bytes);
        assert_eq!(bytes, vec![1, 0, 0, 0, 2, 3, 252, 2, 1, 7]);
    }

    #[test]
    fn scripted_inputs_are_sorted_deterministic_and_bounded() {
        let first = scripted_inputs(2, 10, 7);
        let second = scripted_inputs(2, 10, 7);
        assert_eq!(first, second);
        let mut sorted = first.clone();
        sorted.sort_by_key(|input| (input.tick, input.fighter));
        assert_eq!(first, sorted);
        for input in &first {
            assert!(input.tick < 10);
            assert!(input.fighter < 2);
            assert!((-32..=31).contains(&input.mx));
            assert!((-32..=31).contains(&input.mz));
        }
    }
}

#[cfg(test)]
mod fps_tests {
    //! FPS round/match plus the CQB text slice: elimination decides the round, best of
    //! three takes the match, the reference sim tracks it without moving its digest.
    //!
    //! Consumer: `Sim::step` in this file folds every `FPS_ROUND_TICKS` boundary into
    //! `FpsMatch`; the arena and the door read scripted CQB shots through the unchanged
    //! `Rules::tick` plus tape path. Units: ticks, millimetres, permille. Determinism:
    //! integer counts and saturating folds only, no clock, no allocation in the decide
    //! path. Errors are panics via asserts.

    use super::{decide_fps_round, FpsMatch, FpsRoundWinner, Sim, FPS_ROUND_TICKS};
    use crate::fixtures::fighter;
    use crate::host::World;

    const CQB_SLICE: &str = include_str!("../scripts/cqb_slice.rhai");

    fn fps_world(officers_alive: u32, suspects_alive: u32) -> World {
        let mut fighters = Vec::new();
        for i in 0..officers_alive {
            let id = u8::try_from(i).unwrap_or(u8::MAX);
            fighters.push(fighter(id, 0, 0, 0));
        }
        for s in 0..suspects_alive {
            let id = u8::try_from(2 + s).unwrap_or(u8::MAX);
            fighters.push(fighter(id, 1, 900, 0));
        }
        let mut ordered = fighters;
        ordered.sort_by_key(|view| view.id);
        let officers = usize::try_from(officers_alive).unwrap_or(usize::MAX);
        for (index, view) in ordered.iter_mut().enumerate() {
            view.id = u8::try_from(index).unwrap_or(u8::MAX);
            if index < officers {
                view.team = 0;
            } else {
                view.team = 1;
            }
        }
        World {
            tick: 599,
            seed: 7,
            fighters: ordered,
        }
    }

    #[test]
    fn fps_round_tie_while_both_sides_live_and_when_empty() {
        assert_eq!(
            decide_fps_round(&fps_world(2, 2).fighters),
            FpsRoundWinner::Tie,
            "both sides up is no decision via decide"
        );
        assert_eq!(
            decide_fps_round(
                &World {
                    tick: 0,
                    seed: 0,
                    fighters: Vec::new(),
                }
                .fighters
            ),
            FpsRoundWinner::Tie,
            "empty world is a tie via decide"
        );
        assert_eq!(
            decide_fps_round(&fps_world(0, 0).fighters),
            FpsRoundWinner::Tie,
            "all down is a tie via decide"
        );
    }

    #[test]
    fn fps_round_names_the_elimination_winner() {
        assert_eq!(
            decide_fps_round(&fps_world(2, 0).fighters),
            FpsRoundWinner::Officers,
            "suspects down names officers via decide"
        );
        assert_eq!(
            decide_fps_round(&fps_world(0, 3).fighters),
            FpsRoundWinner::Suspects,
            "officers down names suspects via decide"
        );
    }

    #[test]
    fn fps_match_best_of_three_with_tie_replays() {
        let mut played = FpsMatch::new();
        assert_eq!(played.winner(), None, "empty match has no winner");
        played.apply(FpsRoundWinner::Tie);
        assert_eq!(played.winner(), None, "a tie replay scores nothing");
        assert_eq!(played.ties(), 1, "tie counted via match");
        played.apply(FpsRoundWinner::Officers);
        assert_eq!(played.winner(), None, "one win is not the match");
        played.apply(FpsRoundWinner::Suspects);
        played.apply(FpsRoundWinner::Officers);
        assert_eq!(
            played.winner(),
            Some(FpsRoundWinner::Officers),
            "two wins take the match via apply"
        );
        assert_eq!(played.wins(), (2, 1), "wins split via match");
        assert_eq!(played.rounds_played(), 4, "ties replay via match");
        assert_eq!(played.digest(), played.digest(), "digest repeats via match");
        assert_ne!(
            FpsMatch::new().digest(),
            played.digest(),
            "wins move the digest via match"
        );
    }

    #[test]
    fn sim_tracks_fps_observer_and_replays_exactly() {
        let mut first = Sim::new(4, 21);
        let mut rules = crate::Rules::default();
        for _ in 0..FPS_ROUND_TICKS {
            first.step(&[], &mut rules).expect("step");
        }
        assert_eq!(
            first.fps_match().rounds_played(),
            1,
            "one round per 600 ticks via step"
        );
        let mut second = Sim::new(4, 21);
        let mut other = crate::Rules::default();
        for _ in 0..FPS_ROUND_TICKS {
            second.step(&[], &mut other).expect("step");
        }
        assert_eq!(first.digest(), second.digest(), "digest repeats via step");
        assert_eq!(
            first.fps_match(),
            second.fps_match(),
            "observer replays via step"
        );
        assert_eq!(
            first.fps_round_winner(),
            decide_fps_round(first.fighters()),
            "live winner reads current fighters via step"
        );
    }

    #[test]
    fn cqb_slice_installs_acts_and_hashes_in_tape() {
        let mut rules = crate::Rules::default();
        let receipt = rules.set(CQB_SLICE, 0).expect("cqb slice installs");
        assert_eq!(receipt.hash, crate::script_hash(CQB_SLICE), "hash via set");
        assert!(receipt.max_ops > 0, "ops counted via set");
        let world = World {
            tick: 100,
            seed: 7,
            fighters: vec![fighter(0, 0, 0, 0), fighter(1, 1, 900, 0)],
        };
        let report = rules.tick(&world).expect("tick");
        assert!(report.faults.is_empty(), "{:?}", report.faults);
        assert!(
            report.actions.iter().any(|e| e.action
                == crate::Action::Damage {
                    target: 1,
                    permille: 120,
                }),
            "point-blank hits full via tick: {:?}",
            report.actions
        );
        for (gap_mm, permille) in [(2_000, 90), (4_000, 60)] {
            let mut band_rules = crate::Rules::default();
            band_rules.set(CQB_SLICE, 0).expect("cqb slice installs");
            let band_world = World {
                tick: 200,
                seed: 7,
                fighters: vec![fighter(0, 0, 0, 0), fighter(1, 1, gap_mm, 0)],
            };
            let band_report = band_rules.tick(&band_world).expect("tick");
            assert!(band_report.faults.is_empty(), "{:?}", band_report.faults);
            assert!(
                band_report.actions.iter().any(|e| e.action
                    == crate::Action::Damage {
                        target: 1,
                        permille,
                    }),
                "gap {gap_mm} hits {permille} via tick: {:?}",
                band_report.actions
            );
        }
        let mut library = crate::tape::ScriptLibrary::new();
        let tape =
            crate::tape::record_scripted(2, 7, 600, CQB_SLICE, &mut library).expect("records");
        assert_eq!(
            tape.scripts[0].hash,
            crate::script_hash(CQB_SLICE),
            "script hash in tape via record"
        );
        let first = crate::tape::replay(&tape, &library, crate::Limits::DEFAULT).expect("replay 1");
        let second =
            crate::tape::replay(&tape, &library, crate::Limits::DEFAULT).expect("replay 2");
        assert_eq!(first, second, "slice replays via tape");
        assert_eq!(first, *tape.checkpoints.last().unwrap(), "final via tape");
    }
}
