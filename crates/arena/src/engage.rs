//! engage: who fights whom under rules v2, one plan per tick.
//!
//! Consumer: [`Sim::step_with`](crate::Sim) builds one [`Plan`] before the
//! brains decide and aims each brain's [`Percept`] through it. Data owner: the
//! plan is rebuilt from the fight and pose state every tick and keeps nothing,
//! so snapshots, forks and replays reproduce it from state alone. Units:
//! millimetres from the pose, permille for hp and stamina, ticks for swing
//! clocks. Determinism: integer-only ordering by `(separation, slot)`; the
//! attack tokens ([`Squad::claim_token`]) go to the nearest attackers first,
//! so slot order never favours a fighter. Errors: none, an unseatable roster
//! leaves every brain free to engage. Budget: `O(n^2)` integer compares on
//! the stack, zero allocation.
//!
//! Each living fighter sees its nearest three living foes (teammates and the
//! fallen are not foes) and fights the nearest one whose attack token it can
//! get; a fighter that cannot get one waits outside the ring on its nearest
//! foe, so at most two fighters press one target.

use crate::{match_wire, FighterState, MAX_SLOTS};
use engine_ai::{FoeView, MeleeView, Percept, Squad, StrikePhase};
use engine_combat::fight::FighterState as FightFighter;
use engine_combat::{FightState, TABLES_V1};
use engine_input::FighterId;

/// Combat stamina points to permille (the maximum is 100 points).
const STAMINA_TO_PERMILLE: u16 = 10;

/// Per-slot melee views and targets for one tick.
pub(crate) struct Plan {
    views: [MeleeView; MAX_SLOTS],
    targets: [usize; MAX_SLOTS],
}

/// Swing phase and ticks to impact from a combat fighter's clocks.
fn strike_phase(fighter: &FightFighter) -> (StrikePhase, u32) {
    let startup = TABLES_V1.attack_startup_ticks;
    if !fighter.is_attacking {
        (StrikePhase::Idle, 0)
    } else if fighter.attack_age < startup {
        (StrikePhase::Windup, startup - fighter.attack_age)
    } else if fighter.attack_age == startup {
        (StrikePhase::Active, 0)
    } else {
        (StrikePhase::Recovery, 0)
    }
}

/// Planar separation in whole millimetres between two feet positions.
fn separation_mm(a: [i32; 3], b: [i32; 3]) -> i32 {
    let dx = i64::from(a[0]) - i64::from(b[0]);
    let dz = i64::from(a[2]) - i64::from(b[2]);
    let len = u64::try_from(dx * dx + dz * dz).unwrap_or(0).isqrt();
    i32::try_from(len).unwrap_or(i32::MAX)
}

/// Per-slot feet positions in whole millimetres, converted once per tick so
/// the per-foe scans read integers instead of re-converting every pair.
fn pose_mm(states: &[FighterState], count: usize) -> [[i32; 3]; MAX_SLOTS] {
    let mut mm = [[0i32; 3]; MAX_SLOTS];
    for (cell, state) in mm.iter_mut().zip(states.iter()).take(count) {
        *cell = match_wire::pos_mm(state.pos);
    }
    mm
}

/// Planar separations in mm for every unordered pair of living foes: one
/// integer sqrt per pair, stored at `[lower][higher]` so the reverse read
/// reuses it. Teammates, the fallen and empty slots stay zero and are never
/// read.
fn separation_table(
    mm: &[[i32; 3]; MAX_SLOTS],
    fight: &FightState,
    teams: &[u8; MAX_SLOTS],
    count: usize,
) -> [[i32; MAX_SLOTS]; MAX_SLOTS] {
    let mut table = [[0i32; MAX_SLOTS]; MAX_SLOTS];
    for a in 0..count {
        if fight.fighters[a].hp_permille == 0 {
            continue;
        }
        for b in (a + 1)..count {
            if fight.fighters[b].hp_permille != 0 && teams[a] != teams[b] {
                table[a][b] = separation_mm(mm[a], mm[b]);
            }
        }
    }
    table
}

/// One slot's quantized sightings of every living foe, in slot order, reading
/// the tick's pair separations.
fn sightings(
    fight: &FightState,
    teams: &[u8; MAX_SLOTS],
    count: usize,
    slot: usize,
    sep: &[[i32; MAX_SLOTS]; MAX_SLOTS],
    out: &mut [FoeView; MAX_SLOTS],
) -> usize {
    let mut seen = 0;
    for other in 0..count {
        let fighter = &fight.fighters[other];
        if other == slot || teams[other] == teams[slot] || fighter.hp_permille == 0 {
            continue;
        }
        let (phase, frames) = strike_phase(fighter);
        let (low, high) = if slot < other {
            (slot, other)
        } else {
            (other, slot)
        };
        out[seen] = FoeView::quantize(
            u8::try_from(other).unwrap_or(u8::MAX),
            sep[low][high],
            fighter.hp_permille,
            phase,
            frames,
        );
        seen += 1;
    }
    seen
}

impl Plan {
    /// Plan the tick over `count` slots: convert every pose once, measure
    /// each living foe pair once, build every living fighter's view, then
    /// hand out attack tokens nearest attacker first.
    pub(crate) fn build(
        states: &[FighterState],
        fight: &FightState,
        teams: &[u8; MAX_SLOTS],
        count: usize,
    ) -> Self {
        let blank = FoeView::default();
        let mut views = [MeleeView::EMPTY; MAX_SLOTS];
        let mut targets = [usize::MAX; MAX_SLOTS];
        let mut order = [(i32::MAX, 0usize); MAX_SLOTS];
        let mut roster = [FighterId(0); MAX_SLOTS];
        let mm = pose_mm(states, count);
        let sep = separation_table(&mm, fight, teams, count);
        let (mut ranked, mut living) = (0usize, 0usize);
        for slot in 0..count {
            let me = &fight.fighters[slot];
            if me.hp_permille == 0 {
                continue;
            }
            roster[living] = FighterId(u8::try_from(slot).unwrap_or(u8::MAX));
            living += 1;
            let mut found = [blank; MAX_SLOTS];
            let seen = sightings(fight, teams, count, slot, &sep, &mut found);
            let stamina = me.stamina.saturating_mul(STAMINA_TO_PERMILLE);
            views[slot] = MeleeView::nearest(me.hp_permille, stamina, &found[..seen]);
            if let Some(nearest) = views[slot].foes().first() {
                targets[slot] = usize::from(nearest.slot);
                order[ranked] = (nearest.dist_mm(), slot);
                ranked += 1;
            }
        }
        // Insertion sort by `(dist_mm, slot)`: same lexicographic order as
        // `sort_unstable` on the tuple, but without the std smallsort
        // symbol the stale arena rlib references. Slots are unique, so
        // the total order matches exactly; stack-only, zero allocation.
        let mut i = 1usize;
        while i < ranked {
            let mut j = i;
            while j > 0 && order[j] < order[j - 1] {
                order.swap(j, j - 1);
                j -= 1;
            }
            i += 1;
        }
        let mut squad = Squad::new(&roster[..living]).ok();
        for &(_, slot) in &order[..ranked] {
            let Some(book) = squad.as_mut() else { break };
            let me = FighterId(u8::try_from(slot).unwrap_or(u8::MAX));
            let granted = views[slot]
                .foes()
                .iter()
                .find(|foe| book.claim_token(me, FighterId(foe.slot)).unwrap_or(true));
            let (aim, engage) = match granted {
                Some(foe) => (foe.slot, true),
                None => (views[slot].target().slot, false),
            };
            targets[slot] = usize::from(aim);
            views[slot] = views[slot].targeting(aim, engage);
        }
        Self { views, targets }
    }

    /// Aim `percept` at this slot's planned target and attach its view. A
    /// slot with no living foe keeps the legacy percept.
    pub(crate) fn aim(
        &self,
        percept: Percept,
        slot: usize,
        states: &[FighterState],
        fight: &FightState,
    ) -> Percept {
        let target = self.targets[slot];
        if target >= MAX_SLOTS {
            return percept;
        }
        let me = match_wire::pos_mm(states[slot].pos);
        let dist_mm = separation_mm(me, match_wire::pos_mm(states[target].pos));
        let aimed = Percept::new(
            percept.self_pos,
            percept.self_yaw,
            states[target].pos,
            fight.fighters[target].is_attacking,
            f32::from(u16::try_from(dist_mm / 10).unwrap_or(u16::MAX)) / 100.0,
            percept.tick,
        );
        aimed.with_melee(self.views[slot])
    }
}

#[cfg(test)]
impl Plan {
    /// The pre-hoist plan: every foe pair converts both poses again and
    /// measures its own separation. Kept as the equality oracle for
    /// [`Plan::build`]; the test-only `Sim::pairwise_plan` switch drives
    /// whole matches through it, so a test can compare events, poses and
    /// hashes.
    pub(crate) fn build_pairwise(
        states: &[FighterState],
        fight: &FightState,
        teams: &[u8; MAX_SLOTS],
        count: usize,
    ) -> Self {
        let blank = FoeView::default();
        let mut views = [MeleeView::EMPTY; MAX_SLOTS];
        let mut targets = [usize::MAX; MAX_SLOTS];
        let mut order = [(i32::MAX, 0usize); MAX_SLOTS];
        let mut roster = [FighterId(0); MAX_SLOTS];
        let (mut ranked, mut living) = (0usize, 0usize);
        for slot in 0..count {
            let me = &fight.fighters[slot];
            if me.hp_permille == 0 {
                continue;
            }
            roster[living] = FighterId(u8::try_from(slot).unwrap_or(u8::MAX));
            living += 1;
            let mut found = [blank; MAX_SLOTS];
            let seen = sightings_pairwise(states, fight, teams, count, slot, &mut found);
            let stamina = me.stamina.saturating_mul(STAMINA_TO_PERMILLE);
            views[slot] = MeleeView::nearest(me.hp_permille, stamina, &found[..seen]);
            if let Some(nearest) = views[slot].foes().first() {
                targets[slot] = usize::from(nearest.slot);
                order[ranked] = (nearest.dist_mm(), slot);
                ranked += 1;
            }
        }
        let mut i = 1usize;
        while i < ranked {
            let mut j = i;
            while j > 0 && order[j] < order[j - 1] {
                order.swap(j, j - 1);
                j -= 1;
            }
            i += 1;
        }
        let mut squad = Squad::new(&roster[..living]).ok();
        for &(_, slot) in &order[..ranked] {
            let Some(book) = squad.as_mut() else { break };
            let me = FighterId(u8::try_from(slot).unwrap_or(u8::MAX));
            let granted = views[slot]
                .foes()
                .iter()
                .find(|foe| book.claim_token(me, FighterId(foe.slot)).unwrap_or(true));
            let (aim, engage) = match granted {
                Some(foe) => (foe.slot, true),
                None => (views[slot].target().slot, false),
            };
            targets[slot] = usize::from(aim);
            views[slot] = views[slot].targeting(aim, engage);
        }
        Self { views, targets }
    }
}

/// The pre-hoist sightings: both poses convert per pair, kept as the
/// equality oracle for [`sightings`].
#[cfg(test)]
fn sightings_pairwise(
    states: &[FighterState],
    fight: &FightState,
    teams: &[u8; MAX_SLOTS],
    count: usize,
    slot: usize,
    out: &mut [FoeView; MAX_SLOTS],
) -> usize {
    let mine = match_wire::pos_mm(states[slot].pos);
    let mut seen = 0;
    for other in 0..count {
        let fighter = &fight.fighters[other];
        if other == slot || teams[other] == teams[slot] || fighter.hp_permille == 0 {
            continue;
        }
        let (phase, frames) = strike_phase(fighter);
        out[seen] = FoeView::quantize(
            u8::try_from(other).unwrap_or(u8::MAX),
            separation_mm(mine, match_wire::pos_mm(states[other].pos)),
            fighter.hp_permille,
            phase,
            frames,
        );
        seen += 1;
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArenaConfig, ArenaRules, FightEvent, FighterTape, Sim};
    use engine_combat::tables::TABLES_VERSION_V2;

    /// One 8-fighter rules-v2 config for the equality and timing receipts.
    fn bout_config() -> ArenaConfig {
        ArenaConfig {
            seed: 2040,
            ticks: 600,
            fighters: 8,
            half_extent_m: 20.0,
            table_version: TABLES_VERSION_V2,
        }
    }

    /// The hoisted plan against the pre-hoist pairwise oracle over a real
    /// 8-fighter v2 bout: every tick both plans list the same views and
    /// targets, and the two whole runs emit the same events in the same
    /// order, advance to the same poses and report the same checkpoint hash.
    #[test]
    fn v2_match_matches_the_pairwise_plan_oracle_tick_for_tick() {
        let config = bout_config();
        let tape = FighterTape::default();
        let mut shipped = Sim::build(&config, &tape, false, false).expect("shipped sim");
        let mut oracle = Sim::build(&config, &tape, false, false).expect("oracle sim");
        oracle.pairwise_plan = true;
        let mut events = 0usize;
        while shipped.tick < config.ticks {
            let built = Plan::build(&shipped.states, &shipped.fight, &shipped.rules.teams, 8);
            let pairwise =
                Plan::build_pairwise(&shipped.states, &shipped.fight, &shipped.rules.teams, 8);
            assert_eq!(
                (built.views, built.targets),
                (pairwise.views, pairwise.targets),
                "plan differs at tick {}",
                shipped.tick
            );
            shipped.step(&tape).expect("shipped step");
            oracle.step(&tape).expect("oracle step");
            assert_eq!(
                shipped.fight, oracle.fight,
                "fight state diverged at tick {}",
                shipped.tick
            );
            assert_eq!(
                shipped.states, oracle.states,
                "poses diverged at tick {}",
                shipped.tick
            );
            assert_eq!(
                shipped.events.last(),
                oracle.events.last(),
                "events diverged at tick {}",
                shipped.tick
            );
            assert_eq!(
                shipped.checkpoints.last(),
                oracle.checkpoints.last(),
                "hash diverged at tick {}",
                shipped.tick
            );
            events += shipped.events.last().expect("row").events.len();
        }
        assert!(events > 0, "the fixture must land blows, not idle");
    }

    /// Timing receipt: p50 microseconds of one [`Plan::build`] plus its eight
    /// [`Plan::aim`] calls, the hoisted table against the pairwise oracle, on
    /// a real 8-fighter v2 bout's poses, and the same pair end to end over
    /// 600 ticks of that match. Both sides run interleaved so one load sits
    /// under each. Run: cargo test -p engine-arena --release --lib
    /// plan_paired_receipt -- --ignored --nocapture
    #[test]
    #[ignore = "timing: run with --ignored (release, one thread) for the receipt"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn plan_paired_receipt() {
        const SAMPLES: usize = 201;
        const BATCH: usize = 32;
        const TICK_REPS: usize = 21;
        let config = bout_config();
        let tape = FighterTape::default();
        let mut warm = Sim::build(&config, &tape, false, false).expect("warm sim");
        for _ in 0..120 {
            warm.step(&tape).expect("warm step");
        }
        let states = warm.states;
        let fight = warm.fight;
        let teams = warm.rules.teams;
        let count = warm.fighters;
        let percept = Percept::new([0.0, 0.0, 0.0], 0.0, [1.0, 0.0, 0.0], false, 1.0, 0);
        let mut new_us = Vec::with_capacity(SAMPLES);
        let mut old_us = Vec::with_capacity(SAMPLES);
        let batch = f64::from(u32::try_from(BATCH).unwrap_or(1));
        for _ in 0..SAMPLES {
            let begin = std::time::Instant::now();
            for _ in 0..BATCH {
                let plan = Plan::build(&states, &fight, &teams, count);
                for slot in 0..count {
                    std::hint::black_box(plan.aim(percept, slot, &states, &fight));
                }
            }
            new_us.push(begin.elapsed().as_secs_f64() * 1.0e6 / batch);
            let begin = std::time::Instant::now();
            for _ in 0..BATCH {
                let plan = Plan::build_pairwise(&states, &fight, &teams, count);
                for slot in 0..count {
                    std::hint::black_box(plan.aim(percept, slot, &states, &fight));
                }
            }
            old_us.push(begin.elapsed().as_secs_f64() * 1.0e6 / batch);
        }
        new_us.sort_by(|a, b| a.partial_cmp(b).expect("finite samples"));
        old_us.sort_by(|a, b| a.partial_cmp(b).expect("finite samples"));
        let (new_p50, old_p50) = (new_us[SAMPLES / 2], old_us[SAMPLES / 2]);
        eprintln!(
            "plan_build fixture=v2_8f old_p50_us={old_p50:.4} table_p50_us={new_p50:.4} ratio_x={:.3}",
            old_p50 / new_p50
        );
        assert!(
            new_p50 <= old_p50,
            "the hoisted plan must not be the slower one: {new_p50:.4} vs {old_p50:.4}"
        );
        let mut match_new = Vec::with_capacity(TICK_REPS);
        let mut match_old = Vec::with_capacity(TICK_REPS);
        for _ in 0..TICK_REPS {
            let begin = std::time::Instant::now();
            let mut shipped = Sim::build(&config, &tape, false, false).expect("shipped sim");
            while shipped.tick < config.ticks {
                shipped.step(&tape).expect("shipped step");
            }
            match_new.push(begin.elapsed().as_secs_f64() * 1.0e6);
            let begin = std::time::Instant::now();
            let mut oracle = Sim::build(&config, &tape, false, false).expect("oracle sim");
            oracle.pairwise_plan = true;
            while oracle.tick < config.ticks {
                oracle.step(&tape).expect("oracle step");
            }
            match_old.push(begin.elapsed().as_secs_f64() * 1.0e6);
            assert_eq!(
                shipped.checkpoints.last(),
                oracle.checkpoints.last(),
                "the timed pair must agree on the final hash"
            );
        }
        match_new.sort_by(|a, b| a.partial_cmp(b).expect("finite samples"));
        match_old.sort_by(|a, b| a.partial_cmp(b).expect("finite samples"));
        let (new_match, old_match) = (match_new[TICK_REPS / 2], match_old[TICK_REPS / 2]);
        eprintln!(
            "v2_match_600ticks fixture=8f old_p50_us={old_match:.1} table_p50_us={new_match:.1} ratio_x={:.3}",
            old_match / new_match
        );
        assert!(
            new_match <= old_match,
            "the hoisted plan must not slow the match: {new_match:.1} vs {old_match:.1}"
        );
    }

    fn states(at: &[[f32; 2]]) -> Vec<FighterState> {
        at.iter()
            .enumerate()
            .map(|(slot, xz)| FighterState {
                id: FighterId(u8::try_from(slot).expect("slot")),
                pos: [xz[0], 0.0, xz[1]],
                yaw: 0.0,
                worn: 0,
            })
            .collect()
    }

    fn teams(of: &[u8]) -> [u8; MAX_SLOTS] {
        let mut out = [0u8; MAX_SLOTS];
        out[..of.len()].copy_from_slice(of);
        out
    }

    fn engaged(plan: &Plan, count: usize) -> usize {
        plan.views[..count]
            .iter()
            .filter(|view| view.engage())
            .count()
    }

    #[test]
    fn melee_percept_plan_never_lets_more_than_two_press_one_target() {
        // Four attackers on team 0 ring one target (slot 4) on team 1.
        let at = [[2.0, 0.0], [0.0, 3.0], [-4.0, 0.0], [0.0, -5.0], [0.0, 0.0]];
        let fight = FightState::new(5).expect("fight");
        let plan = Plan::build(&states(&at), &fight, &teams(&[0, 0, 0, 0, 1]), 5);
        let pressing: Vec<bool> = plan.views[..4].iter().map(|v| v.engage()).collect();
        assert_eq!(
            pressing,
            vec![true, true, false, false],
            "the two nearest press"
        );
        assert_eq!(engaged(&plan, 4), 2);
        for slot in 0..4 {
            assert_eq!(
                plan.targets[slot], 4,
                "a waiter still faces its nearest foe"
            );
        }
    }

    #[test]
    fn melee_percept_plan_sends_a_third_attacker_to_a_free_foe() {
        // Attackers 0..=2 are all nearest to foe 3; foe 4 stands farther off.
        let at = [[1.0, 0.0], [0.0, 1.5], [-2.0, 0.0], [0.0, 0.0], [9.0, 9.0]];
        let fight = FightState::new(5).expect("fight");
        let plan = Plan::build(&states(&at), &fight, &teams(&[0, 0, 0, 1, 1]), 5);
        assert_eq!(plan.targets[0], 3);
        assert_eq!(plan.targets[1], 3);
        assert_eq!(plan.targets[2], 4, "the third attacker takes the free foe");
        assert!(plan.views[2].engage(), "and presses it");
        assert_eq!(engaged(&plan, 3), 3);
    }

    #[test]
    fn melee_percept_plan_lists_the_nearest_three_foes_in_order() {
        let at = [[0.0, 0.0], [7.0, 0.0], [3.0, 0.0], [5.0, 0.0], [1.0, 0.0]];
        let fight = FightState::new(5).expect("fight");
        let plan = Plan::build(&states(&at), &fight, &teams(&[0, 1, 2, 3, 4]), 5);
        let listed: Vec<u8> = plan.views[0].foes().iter().map(|foe| foe.slot).collect();
        assert_eq!(
            listed,
            vec![4, 2, 3],
            "four foes, nearest three in distance order"
        );
        assert_eq!(plan.targets[0], 4);
    }

    #[test]
    fn melee_percept_plan_skips_teammates_and_the_fallen() {
        let at = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]];
        let mut fight = FightState::new(4).expect("fight");
        fight.fighters[2].hp_permille = 0;
        let plan = Plan::build(&states(&at), &fight, &teams(&[0, 0, 1, 1]), 4);
        let listed: Vec<u8> = plan.views[0].foes().iter().map(|foe| foe.slot).collect();
        assert_eq!(listed, vec![3], "slot 1 is a teammate and slot 2 is down");
        assert!(!plan.views[2].is_live(), "a fallen fighter plans nothing");
        assert_eq!(plan.targets[2], usize::MAX);
    }

    #[test]
    fn melee_percept_aim_points_the_percept_at_the_planned_target() {
        let at = [[0.0, 0.0], [4.0, 0.0], [2.0, 0.0]];
        let mut fight = FightState::new(3).expect("fight");
        fight.fighters[2].is_attacking = true;
        fight.fighters[2].attack_age = 2;
        let pose = states(&at);
        let plan = Plan::build(&pose, &fight, &teams(&[0, 1, 2]), 3);
        let base = Percept::new(pose[0].pos, 0.0, pose[1].pos, false, 4.0, 9);
        let aimed = plan.aim(base, 0, &pose, &fight);
        assert_eq!(aimed.foe_pos, pose[2].pos, "faces the nearer foe");
        assert!((aimed.dist_m - 2.0).abs() < 0.011);
        assert!(aimed.foe_attacking);
        assert_eq!(aimed.tick, 9);
        let target = aimed.melee.target();
        assert_eq!(
            (target.slot, target.phase, target.frames_to_active),
            (2, StrikePhase::Windup, 2)
        );
        assert_eq!(aimed.melee.foes().len(), 2);
    }

    /// One v2 duel: the first knockdown decides. `Some(slot)` wins, `None`
    /// is a same-tick double knockout or no knockdown inside the budget.
    fn duel(seed: u64, ticks: u32) -> (Option<u8>, bool) {
        let config = ArenaConfig::try_new(seed, ticks, 2, 20.0).expect("valid config");
        let tape = FighterTape::default();
        let mut sim = Sim::build(&config, &tape, false, false).expect("build");
        assert!(sim.set_rules(ArenaRules::v2_free_for_all()));
        while sim.tick < config.ticks {
            sim.step(&tape).expect("step");
            let mut down = [false; 2];
            for event in &sim.events.last().expect("row").events {
                if let FightEvent::Knockdown { victim, .. } = *event {
                    down[usize::from(victim)] = true;
                }
            }
            match down {
                [true, false] => return (Some(1), false),
                [false, true] => return (Some(0), false),
                [true, true] => return (None, true),
                [false, false] => {}
            }
        }
        (None, false)
    }

    /// `(slot-0 wins, slot-1 wins, double knockouts)` over `0..seeds`.
    fn split(seeds: u64, ticks: u32) -> (u32, u32, u32) {
        let (mut zero, mut one, mut both) = (0, 0, 0);
        for seed in 0..seeds {
            match duel(seed, ticks) {
                (Some(0), _) => zero += 1,
                (Some(_), _) => one += 1,
                (None, true) => both += 1,
                (None, false) => {}
            }
        }
        (zero, one, both)
    }

    #[test]
    fn v2_duels_with_temperaments_replay_equal_and_often_decide() {
        let first = split(24, 3_600);
        assert_eq!(first, split(24, 3_600), "same seeds, same split");
        let decisive = first.0 + first.1;
        assert!(
            decisive >= 10,
            "temperaments must break the lockstep: {first:?}"
        );
    }

    #[test]
    #[ignore = "slow: 400 brain duels; run with --ignored for the win-split bar"]
    fn v2_duels_split_by_temperament_have_winners_and_no_slot_edge() {
        let (zero, one, both) = split(400, 3_600);
        let decisive = zero + one;
        println!("v2 split: slot0 {zero} slot1 {one} decisive {decisive} double_ko {both} of 400");
        assert!(
            decisive * 100 >= 400 * 60,
            "at least 60% decisive: {decisive} of 400"
        );
        assert!(
            both * 100 < 400 * 20,
            "double knockouts under 20%: {both} of 400"
        );
        let share = zero * 100 / decisive.max(1);
        assert!(
            (40..=60).contains(&share),
            "slot 0 won {share}% of {decisive}"
        );
    }
}
