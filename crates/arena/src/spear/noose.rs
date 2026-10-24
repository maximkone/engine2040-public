//! noose: the laquearius' cord (row SG11), a rope throw that snares a man.
//!
//! Plan: `docs/plan/spear-grapple.md` ("Laquearius noose"). Isidore (Etymologies
//! 18.56) has the laquearii throw a noose over a man who is running, tangle him,
//! then chase him down and knock him over with the shield boss. A fighter
//! presses `THROW_NOOSE`; a loop flies 6 m along the facing. It is wider than
//! a spear's tip, and wider again on a man moving away from the thrower. A
//! catch snares him for up to 90 ticks: he cannot dodge, he walks at two
//! fifths of his speed and the cord drags him back when he pulls away. A boss
//! push landing on a snared man knocks him down and takes the cord off; any
//! blade that crosses the cord cuts it (the shared rope cut of the spear).
//!
//! This is a child of [`super`] so the noose rides the spear's phases, events,
//! cooldown, rope cut and view: a snare is a [`Tether`] with `noose` set and a
//! loop in the air is a [`Flight`] with `noose` set. Consumer: `Sim::step_with`
//! through [`SpearState`], only for a match built with `MatchSpec::noose`; the
//! window reads the same [`SpearSlot`]s the spear fills. Units: integer
//! millimetres, mm per tick (60 Hz), Q16 unit vectors. Determinism: integers
//! and the Q16 `swept_cast` only; the extra fold bytes exist only for a noose
//! flight, a noose snare, or a match with the noose armed. Errors: none.

use engine_combat::FightEvent;
use engine_input::fighter::DODGE;
use engine_input::Buttons;

use super::*;

/// Fold byte of a noose flight or snare, after the spear's own bytes.
pub(super) const TAG: u8 = 0x4E;
/// Fold byte opening the noose block of a checkpoint.
const FOLD_TAG: u8 = 0x4F;
/// Muzzle speed of the loop, mm per tick (30 m/s).
const NOOSE_SPEED: i32 = 500;
/// Flight ticks the loop lives: 6 m at [`NOOSE_SPEED`].
const NOOSE_MAX_TICKS: u16 = 12;
/// What the open loop adds to the body radius when it closes, mm.
const LOOP_MM: i32 = 150;
/// What it adds on top for a man running away from the thrower, mm: the loop
/// lands over a fleeing man, which is what the laquearius was for.
const CHASE_MM: i32 = 250;
/// Ticks a snare holds before the cord slips off.
const SNARE_TICKS: u16 = 90;
/// Share of the walking speed a snared man keeps, percent.
const WALK_PERCENT: i32 = 40;
/// Cord paid out beyond the gap at the catch, mm.
const SLACK_MM: i64 = 100;
/// Shortest the cord tightens to, mm: outside the boss push's reach, so the
/// thrower has to walk in to shove.
const MIN_ROPE_MM: i64 = 900;
/// Pull on a man straining at the end of the cord, mm per tick.
const TUG_MM: i64 = 12;
/// Most the cord drags one man in a tick, mm.
const DRAG_MAX_MM: i64 = 160;
/// Freeze on the catch, ticks: the loop closing is felt.
const CATCH_HITSTOP: u32 = 3;

/// Flight ticks of a throw: a noose's range is shorter than a spear's.
pub(super) const fn max_ticks(noose: bool) -> u16 {
    if noose {
        NOOSE_MAX_TICKS
    } else {
        MAX_FLIGHT_TICKS
    }
}

/// Ticks a tether lives before it times out.
pub(super) const fn tether_max(noose: bool) -> u16 {
    if noose {
        SNARE_TICKS
    } else {
        TETHER_MAX
    }
}

/// A fighter capsule widened by `extra` mm: the open loop around a body.
fn loop_capsule(id: u32, body_mm: [i32; 3], extra: i32) -> CastCapsule {
    let mut capsule = hurt_capsule(id, body_mm);
    capsule.radius = fx(CAPSULE_RADIUS + extra);
    capsule
}

/// True when `victim` wishes to move away from `thrower`: the wish points down
/// the line from the thrower through the victim. Wishes are world axes.
fn flees(thrower_mm: [i32; 3], victim_mm: [i32; 3], victim: FighterInput) -> bool {
    let away = [
        i64::from(victim_mm[0]) - i64::from(thrower_mm[0]),
        i64::from(victim_mm[2]) - i64::from(thrower_mm[2]),
    ];
    i64::from(victim.mx) * away[0] + i64::from(victim.mz) * away[1] > 0
}

/// A walk wish at [`WALK_PERCENT`] of itself.
fn slowed(wish: i8) -> i8 {
    i8::try_from(i32::from(wish) * WALK_PERCENT / 100).unwrap_or(0)
}

impl SpearState {
    /// A state that throws nooses and no spears: `THROW_SPEAR` does nothing.
    pub(crate) fn noose_only(fighters: usize) -> Self {
        let mut state = Self::new(fighters);
        state.spear_on = false;
        state.noose_on = true;
        state
    }

    /// The state after the spear is armed: a noose-only `prev` keeps its noose
    /// and throws spears too.
    pub(crate) fn with_spear(prev: Option<Self>, fighters: usize) -> Self {
        match prev {
            Some(mut state) => {
                state.spear_on = true;
                state
            }
            None => Self::new(fighters),
        }
    }

    /// True when the noose is armed.
    pub(crate) fn noose_armed(&self) -> bool {
        self.noose_on
    }

    /// Append a [`FightEvent::Knockdown`] to `log` for every man a boss push
    /// toppled in the last step, so the tick log, the hash and the balance fall
    /// treat him like any other knockdown. The attacker is the thrower.
    pub(crate) fn topple_events(&self, log: &mut Vec<FightEvent>) {
        log.extend(
            self.toppled
                .iter()
                .map(|&(attacker, victim)| FightEvent::Knockdown { attacker, victim }),
        );
    }

    /// The noose block of the checkpoint: the press edges and the pushes that
    /// wait for the freeze to end. The flights and snares fold with the phases.
    pub(super) fn fold_noose(&self, digest: &mut Fnv64) {
        digest.write_u8(FOLD_TAG);
        for slot in 0..self.fighters {
            digest.write_bool(self.noose_held[slot]);
            digest.write_bool(self.pushed[slot]);
        }
    }

    /// Launch a loop from `slot` along its facing.
    pub(super) fn throw_noose(&mut self, slot: usize, input: FighterInput, env: &Env<'_>) {
        let (sin, cos) = facing_q16(input.facing);
        let dir = [sin, cos];
        let body = pos_mm(env.states[slot].pos);
        let (pos_mm, vel_mm) = throw_start(body, dir, SPEAR_AHEAD, SPEAR_HEIGHT, NOOSE_SPEED);
        self.slots[slot].cooldown = THROW_COOLDOWN;
        self.slots[slot].wrap_mm = [0; 3];
        self.slots[slot].phase = Phase::Flight(Flight {
            pos_mm,
            vel_mm,
            age: 0,
            origin_mm: [body[0], body[2]],
            dir,
            world: false,
            noose: true,
        });
        self.events.push(SpearEvent::Thrown {
            thrower: slot_id(slot),
            at_mm: pos_mm,
            vel_mm,
        });
    }

    /// True when a live tether already holds `slot` as its victim.
    fn corded(&self, slot: usize) -> bool {
        self.slots()
            .iter()
            .any(|s| matches!(s.phase, Phase::Tether(t) if t.victim.map(usize::from) == Some(slot)))
    }

    /// The nearest man the loop meets on `from..to`, with the contact point.
    /// Legal like a spear's target, and not already corded; the loop is wider
    /// than the body, and wider again on a man running from the thrower.
    pub(super) fn cast_loop(
        &mut self,
        slot: usize,
        env: &Env<'_>,
        from: [i32; 3],
        to: [i32; 3],
    ) -> Option<(usize, [i32; 3])> {
        let thrower = pos_mm(env.states[slot].pos);
        let mut caps = [CastCapsule {
            id: 0,
            a: [Fixed::ZERO; 3],
            b: [Fixed::ZERO; 3],
            radius: Fixed::ZERO,
        }; MAX_SLOTS];
        let mut count = 0;
        for (victim, state) in env.states.iter().enumerate().take(self.fighters) {
            let combat = &env.fight.fighters[victim];
            if victim == slot
                || combat.hp_permille == 0
                || env.rules.teams[victim] == env.rules.teams[slot]
                || combat.invulnerable_with(env.tables)
                || self.corded(victim)
            {
                continue;
            }
            let body = pos_mm(state.pos);
            let chase = if flees(thrower, body, env.inputs[victim]) {
                CHASE_MM
            } else {
                0
            };
            caps[count] = loop_capsule(u32::from(slot_id(victim)), body, LOOP_MM + chase);
            count += 1;
        }
        let (id, at) = cast_contact(&mut self.hits, from, to, &caps[..count])?;
        Some((usize::try_from(id).ok()?, at))
    }

    /// The loop closed over `victim`: a facing guard turns it away like a
    /// spear, anything else snares him. The cord is as long as the gap and
    /// a little slack; it never grows.
    pub(super) fn catch(
        &mut self,
        slot: usize,
        victim: usize,
        at: [i32; 3],
        f: Flight,
        env: &mut Env<'_>,
    ) {
        if self.guard_turns(slot, victim, env) {
            return;
        }
        let (me, you) = (xz(env.states[slot].pos), xz(env.states[victim].pos));
        let gap = isqrt(sq(me[0] - you[0]) + sq(me[1] - you[1]));
        let rope = (gap + SLACK_MM).max(MIN_ROPE_MM);
        env.fight.hitstop = env.fight.hitstop.max(CATCH_HITSTOP);
        self.slots[victim].drift_mm = [0, 0];
        self.slots[slot].phase = Phase::Tether(Tether {
            victim: Some(slot_id(victim)),
            goal_mm: you,
            anchor_mm: you,
            anchor_y_mm: at[1],
            rope_mm: i32::try_from(rope).unwrap_or(i32::MAX),
            dir: f.dir,
            age: 0,
            latched: false,
            yank: false,
            noose: true,
            speed: 0,
        });
        self.events.push(SpearEvent::Hit {
            thrower: slot_id(slot),
            victim: Some(slot_id(victim)),
            at_mm: at,
        });
    }

    /// One tick of a snare, after the cut, timeout and down checks: the cord
    /// tightens to the shortest gap it has seen (plus its slack) and a man at
    /// the end of it is hauled back toward the thrower, by what he pulled out
    /// and a tug besides. A man inside the cord is left alone.
    pub(super) fn snare(&mut self, slot: usize, mut t: Tether, env: &mut Env<'_>) {
        let Some(victim) = t.victim.map(usize::from) else {
            return;
        };
        let (me, you) = (xz(env.states[slot].pos), xz(env.states[victim].pos));
        let gap = isqrt(sq(me[0] - you[0]) + sq(me[1] - you[1]));
        let rope = i64::from(t.rope_mm).min(gap + SLACK_MM).max(MIN_ROPE_MM);
        t.rope_mm = i32::try_from(rope).unwrap_or(i32::MAX);
        let mut haul = [0, 0];
        let mut speed = 0;
        if gap >= rope {
            speed = (gap - rope + TUG_MM).min(DRAG_MAX_MM);
            push(&mut haul, unit(you, me), speed);
        }
        self.slots[victim].drift_mm = haul;
        t.age += 1;
        t.speed = u16::try_from(speed).unwrap_or(u16::MAX);
        if speed > 0 && t.age % PULL_CUE_PERIOD == 1 {
            self.events.push(SpearEvent::TetherPull {
                thrower: slot_id(slot),
                victim: t.victim,
                speed: t.speed,
            });
        }
        self.slots[slot].phase = Phase::Tether(t);
    }

    /// Before the fight step reads this tick's inputs: a snared man's dodge is
    /// stripped and his walk slowed in `row`, and a boss push that landed on him
    /// last tick is noted. The push sets the freeze and the balance channel that
    /// reports it clears when the next fight step begins, so the report is read
    /// here, ahead of that step, and acted on when the freeze ends.
    pub(crate) fn prepare(&mut self, row: &mut [FighterInput], fight: &FightState) {
        if !self.noose_on {
            return;
        }
        for held in &self.slots[..self.fighters] {
            let Phase::Tether(t) = held.phase else {
                continue;
            };
            let Some(v) = t.victim.map(usize::from).filter(|_| t.noose) else {
                continue;
            };
            if fight.fighters[v].balance_impulse_q8 != 0 {
                self.pushed[v] = true;
            }
            if let Some(input) = row.get_mut(v) {
                input.buttons = Buttons(input.buttons.0 & !DODGE);
                input.mx = slowed(input.mx);
                input.mz = slowed(input.mz);
            }
        }
    }

    /// Act on the pushes noted since the freeze began: the man goes down, the
    /// cord comes off.
    pub(super) fn topple_pushed(&mut self, env: &mut Env<'_>) {
        if !self.noose_on {
            return;
        }
        for slot in 0..self.fighters {
            let Phase::Tether(t) = self.slots[slot].phase else {
                continue;
            };
            let Some(v) = t.victim.filter(|_| t.noose) else {
                continue;
            };
            if !self.pushed[usize::from(v)] {
                continue;
            }
            let man = &mut env.fight.fighters[usize::from(v)];
            if man.hp_permille > 0 && man.knockdown == 0 {
                man.knockdown = env.tables.knockdown_ticks;
                man.is_attacking = false;
                man.is_dodging = false;
                man.is_blocking = false;
                self.toppled.push((slot_id(slot), v));
            }
            self.end(slot, EndWhy::Toppled, 0, env);
        }
        self.pushed = [false; MAX_SLOTS];
    }
}

impl crate::Sim {
    /// Arm the laquearius noose (row SG11) before the first tick: the
    /// `THROW_NOOSE` button throws a loop that snares a man. It rides the
    /// spear's state, so a noose-only match throws no spears. Refuses
    /// (`false`) past tick 0 and on the Rapier `sim` leg, like the spear.
    pub(crate) fn set_noose(&mut self) -> bool {
        if cfg!(feature = "sim") || self.tick != 0 {
            return false;
        }
        let mut state = self
            .spear
            .take()
            .unwrap_or_else(|| SpearState::noose_only(self.fighters));
        state.noose_on = true;
        self.spear = Some(state);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::MatchWorld;
    use crate::{ArenaConfig, Sim};
    use engine_input::fighter::{ATTACK, BLOCK, THROW_SPEAR};
    use engine_input::{FighterId, FighterTape};

    /// One scripted fighter-tick: `(buttons, facing, mx, mz)`.
    type Move = (u8, i16, i8, i8);

    const STILL: Move = (0, 0, 0, 0);
    /// Facing back down the line, toward a thrower standing at negative z.
    const BACK: i16 = 18_000;
    const FACES_THROWER: Move = (0, BACK, 0, 0);

    fn config() -> ArenaConfig {
        ArenaConfig {
            seed: 7,
            ticks: 600,
            fighters: 2,
            half_extent_m: 12.0,
            table_version: 2,
        }
    }

    fn build(spear: bool, noose: bool) -> Sim {
        let mut sim = Sim::build_world(
            &config(),
            &FighterTape::default(),
            false,
            false,
            MatchWorld::empty(),
        )
        .expect("sim builds");
        assert!(!spear || sim.set_spear());
        assert!(!noose || sim.set_noose());
        sim
    }

    /// Both throwing weapons armed.
    fn armed() -> Sim {
        build(true, true)
    }

    fn tick(sim: &mut Sim, script: &[Move]) {
        let at = sim.tick;
        let tape = FighterTape(
            script
                .iter()
                .enumerate()
                .map(|(slot, &(buttons, facing, mx, mz))| FighterInput {
                    tick: at,
                    fighter: FighterId(slot_id(slot)),
                    mx,
                    mz,
                    facing,
                    buttons: Buttons(buttons),
                })
                .collect(),
        );
        sim.cursor = 0;
        sim.step(&tape).expect("step");
    }

    fn events(sim: &Sim) -> Vec<SpearEvent> {
        sim.spear.as_ref().expect("spear state").events().to_vec()
    }

    fn phase(sim: &Sim, slot: usize) -> Phase {
        sim.spear.as_ref().expect("spear state").slots()[slot].phase
    }

    /// Run `n` ticks of the same script, collecting every spear event.
    fn run(sim: &mut Sim, n: u32, script: [Move; 2]) -> Vec<SpearEvent> {
        let mut seen = Vec::new();
        for _ in 0..n {
            tick(sim, &script);
            seen.extend(events(sim));
        }
        seen
    }

    fn place(sim: &mut Sim, thrower: [f32; 2], victim: [f32; 2]) {
        sim.states[0].pos = [thrower[0], 0.0, thrower[1]];
        sim.states[1].pos = [victim[0], 0.0, victim[1]];
    }

    /// Planar gap between the two fighters, mm.
    fn gap_mm(sim: &Sim) -> i64 {
        let (a, b) = (xz(sim.states[0].pos), xz(sim.states[1].pos));
        isqrt(sq(a[0] - b[0]) + sq(a[1] - b[1]))
    }

    /// Planar distance the victim has moved from `from`, mm.
    fn moved_mm(sim: &Sim, from: [i32; 2]) -> i64 {
        let to = xz(sim.states[1].pos);
        isqrt(sq(to[0] - from[0]) + sq(to[1] - from[1]))
    }

    fn caught(seen: &[SpearEvent]) -> bool {
        seen.iter().any(|e| {
            matches!(
                e,
                SpearEvent::Hit {
                    victim: Some(1),
                    ..
                }
            )
        })
    }

    fn first_end(seen: &[SpearEvent]) -> Option<EndWhy> {
        seen.iter().find_map(|e| match *e {
            SpearEvent::TetherEnd { why, .. } => Some(why),
            _ => None,
        })
    }

    /// A throw `button` from `ahead` metres behind a man `side` metres off the
    /// line; he runs straight away from the thrower when `runs`.
    fn throw_at(button: u8, ahead: f32, side: f32, runs: bool) -> bool {
        let mut sim = armed();
        place(&mut sim, [0.0, -ahead], [side, 0.0]);
        let away = if runs { 100 } else { 0 };
        tick(&mut sim, &[(button, 0, 0, 0), (0, 0, 0, away)]);
        let mut seen = events(&sim);
        seen.extend(run(&mut sim, 30, [STILL, (0, 0, 0, away)]));
        caught(&seen)
    }

    /// Thrower at z `-gap`, victim at the origin facing back down the line:
    /// the loop is thrown, and the sim stops on the tick of the catch.
    fn snared(gap: f32) -> (Sim, Vec<SpearEvent>) {
        let mut sim = armed();
        place(&mut sim, [0.0, -gap], [0.0, 0.0]);
        tick(&mut sim, &[(THROW_NOOSE, 0, 0, 0), FACES_THROWER]);
        let mut seen = events(&sim);
        while !caught(&seen) {
            tick(&mut sim, &[STILL, FACES_THROWER]);
            seen.extend(events(&sim));
            assert!(
                sim.tick < 40,
                "the loop closes on a man dead ahead: {seen:?}"
            );
        }
        assert!(
            matches!(phase(&sim, 0), Phase::Tether(t) if t.noose),
            "the catch is a snare"
        );
        (sim, seen)
    }

    fn rope_mm(sim: &Sim) -> i32 {
        match phase(sim, 0) {
            Phase::Tether(t) => t.rope_mm,
            other => panic!("no snare: {other:?}"),
        }
    }

    #[test]
    fn the_loop_is_wider_than_a_spear_and_wider_again_on_a_man_running_away() {
        // 0.75 m off the line: past a spear's reach (0.48 m), past the open
        // loop (0.63 m), inside the loop thrown over a runner (0.88 m).
        assert!(!throw_at(THROW_SPEAR, 5.0, 0.75, true), "a spear misses");
        assert!(
            !throw_at(THROW_NOOSE, 5.0, 0.75, false),
            "the open loop misses"
        );
        assert!(
            throw_at(THROW_NOOSE, 5.0, 0.75, true),
            "the loop takes a runner"
        );
        assert!(
            throw_at(THROW_NOOSE, 5.0, 0.0, false),
            "dead ahead is caught"
        );
    }

    #[test]
    fn the_loop_flies_six_metres() {
        assert!(
            throw_at(THROW_NOOSE, 6.2, 0.0, false),
            "inside the cord's range"
        );
        assert!(
            !throw_at(THROW_NOOSE, 7.4, 0.0, false),
            "past it the loop falls"
        );
    }

    #[test]
    fn a_catch_snares_and_the_cord_comes_off_after_ninety_ticks() {
        let (mut sim, _) = snared(3.0);
        let mut seen = Vec::new();
        let mut waited = 0;
        while first_end(&seen).is_none() {
            seen.extend(run(&mut sim, 1, [STILL, FACES_THROWER]));
            waited += 1;
            assert!(waited < 200, "the snare never ended");
        }
        assert_eq!(first_end(&seen), Some(EndWhy::Timeout));
        // Ninety snare ticks, plus the three-tick freeze on the catch.
        assert!((90..=96).contains(&waited), "ended after {waited} ticks");
        assert!(matches!(phase(&sim, 0), Phase::Idle));
    }

    #[test]
    fn a_snared_man_cannot_dodge() {
        let (mut sim, _) = snared(3.0);
        // Let the catch's freeze run out, so a dodge could start.
        run(&mut sim, 4, [STILL, FACES_THROWER]);
        tick(&mut sim, &[STILL, (DODGE, BACK, 0, 0)]);
        let latched = sim.last_inputs[1].expect("latched input");
        assert_eq!(latched.buttons.0 & DODGE, 0, "the dodge press is stripped");
        assert!(!sim.fight.fighters[1].is_dodging, "no dodge started");
        // The same press from a free man does dodge.
        let mut free = armed();
        place(&mut free, [0.0, -3.0], [0.0, 0.0]);
        tick(&mut free, &[STILL, (DODGE, BACK, 0, 0)]);
        assert!(free.fight.fighters[1].is_dodging, "a free man dodges");
    }

    #[test]
    fn a_snared_man_walks_at_two_fifths_of_a_free_mans_speed() {
        let (mut sim, _) = snared(3.0);
        run(&mut sim, 4, [STILL, FACES_THROWER]);
        let toward = (0, BACK, 0, -100);
        tick(&mut sim, &[STILL, toward]);
        let latched = sim.last_inputs[1].expect("latched input");
        assert_eq!(latched.mz, -40, "the walk wish is two fifths");
        assert_eq!(
            sim.last_inputs[0].expect("thrower").mz,
            0,
            "the thrower is free"
        );
        // Walking in, inside the cord, he covers about two fifths of the ground.
        let start = xz(sim.states[1].pos);
        let mut free = armed();
        place(&mut free, [0.0, -3.0], [0.0, 0.0]);
        tick(&mut free, &[STILL, toward]);
        let free_start = xz(free.states[1].pos);
        for _ in 0..30 {
            tick(&mut sim, &[STILL, toward]);
            tick(&mut free, &[STILL, toward]);
        }
        let (slow, fast) = (moved_mm(&sim, start), moved_mm(&free, free_start));
        assert!(
            slow * 100 >= fast * 25 && slow * 100 <= fast * 55,
            "snared walked {slow} mm against a free man's {fast} mm"
        );
    }

    #[test]
    fn the_cord_drags_a_man_back_when_he_pulls_away() {
        let (mut sim, _) = snared(3.0);
        let rope = i64::from(rope_mm(&sim));
        assert_eq!(rope, 3_100, "the gap at the catch and a hand of slack");
        let mut seen = Vec::new();
        for _ in 0..60 {
            seen.extend(run(&mut sim, 1, [STILL, (0, 0, 0, 100)]));
            assert!(
                gap_mm(&sim) <= rope + 50,
                "the cord holds him: gap {} against rope {rope}",
                gap_mm(&sim)
            );
        }
        assert!(
            seen.iter()
                .any(|e| matches!(e, SpearEvent::TetherPull { thrower: 0, .. })),
            "the cord pulled"
        );
        assert!(
            matches!(phase(&sim, 0), Phase::Tether(t) if t.noose),
            "still snared"
        );
        // The same run with no cord: he is long gone.
        let mut free = armed();
        place(&mut free, [0.0, -3.0], [0.0, 0.0]);
        run(&mut free, 70, [STILL, (0, 0, 0, 100)]);
        assert!(
            gap_mm(&free) > 6_000,
            "a free runner gets away: {}",
            gap_mm(&free)
        );
    }

    #[test]
    fn the_cord_tightens_as_the_thrower_closes_and_never_lets_out() {
        let (mut sim, _) = snared(3.0);
        let first = rope_mm(&sim);
        // The thrower walks up; the cord shortens to the gap and a hand of slack.
        run(&mut sim, 40, [(0, 0, 0, 100), FACES_THROWER]);
        let tighter = rope_mm(&sim);
        assert!(tighter < first - 500, "{tighter} against {first}");
        assert!(
            i64::from(tighter) >= MIN_ROPE_MM,
            "never shorter than the floor"
        );
        // He pulls away again: the rope does not grow back.
        run(&mut sim, 20, [STILL, (0, 0, 0, 100)]);
        assert!(rope_mm(&sim) <= tighter, "the loop only tightens");
    }

    #[test]
    fn a_blocking_man_cannot_shrug_the_cord_off() {
        let (mut sim, _) = snared(3.0);
        let seen = run(&mut sim, 30, [STILL, (BLOCK, BACK, 0, 0)]);
        assert_eq!(first_end(&seen), None, "a guard is no escape: {seen:?}");
        assert!(matches!(phase(&sim, 0), Phase::Tether(t) if t.noose));
    }

    /// The thrower (a gladius in hand) walks up on a man 3 m away and shoves
    /// him with the scutum, with the cord on him or not. Returns the sim, what
    /// the spear saw, the most stagger the man showed and whether he fell.
    fn push_run(cord: bool) -> (Sim, Vec<SpearEvent>, u32, bool) {
        let mut sim = armed();
        sim.fight.fighters[0].weapon = 2;
        place(&mut sim, [0.0, -3.0], [0.0, 0.0]);
        let mut seen = Vec::new();
        if cord {
            tick(&mut sim, &[(THROW_NOOSE, 0, 0, 0), FACES_THROWER]);
            seen.extend(events(&sim));
            while !caught(&seen) {
                tick(&mut sim, &[STILL, FACES_THROWER]);
                seen.extend(events(&sim));
                assert!(sim.tick < 40, "the loop closes");
            }
        }
        for _ in 0..200 {
            if gap_mm(&sim) <= 1_000 {
                break;
            }
            tick(&mut sim, &[(0, 0, 0, 100), FACES_THROWER]);
            seen.extend(events(&sim));
        }
        assert!(gap_mm(&sim) <= 1_000, "the thrower closed to shoving range");
        let (mut stagger, mut fell) = (0, false);
        for n in 0..40 {
            let press = if n == 0 { ATTACK | BLOCK } else { BLOCK };
            tick(&mut sim, &[(press, 0, 0, 0), FACES_THROWER]);
            seen.extend(events(&sim));
            stagger = stagger.max(sim.fight.fighters[1].react);
            fell |= sim.fight.fighters[1].knockdown > 0;
        }
        (sim, seen, stagger, fell)
    }

    #[test]
    fn a_shield_push_on_a_snared_man_is_a_knockdown() {
        let (sim, seen, _, fell) = push_run(true);
        assert!(fell, "the push puts the snared man on the ground: {seen:?}");
        assert!(
            seen.iter().any(|e| matches!(
                e,
                SpearEvent::TetherEnd {
                    why: EndWhy::Toppled,
                    victim: Some(1),
                    ..
                }
            )),
            "the cord comes off with the fall: {seen:?}"
        );
        assert!(matches!(phase(&sim, 0), Phase::Idle), "no cord left");
        // The same shove on a free man only staggers him.
        let (_, _, stagger, fell) = push_run(false);
        assert!(stagger > 0, "the shove landed");
        assert!(!fell, "a free man keeps his feet");
    }

    #[test]
    fn a_man_the_push_topples_is_a_knockdown_event_for_the_log_and_the_balance() {
        let (sim, ..) = push_run(true);
        let mut log = sim.events.iter().flat_map(|t| t.events.iter());
        assert!(
            log.any(|e| matches!(
                e,
                engine_combat::fight::FightEvent::Knockdown {
                    attacker: 0,
                    victim: 1
                }
            )),
            "the tick log carries the knockdown"
        );
    }

    /// A victim in mid-swing aims at `facing`; returns how the cord ended.
    fn cut_with(facing: i16) -> Option<EndWhy> {
        let (mut sim, _) = snared(3.0);
        let f = &mut sim.fight.fighters[1];
        f.is_attacking = true;
        f.attack_age = engine_combat::heft::startup(f.heft, &engine_combat::fight::TABLES_V2);
        first_end(&run(&mut sim, 20, [STILL, (0, facing, 0, 0)]))
    }

    #[test]
    fn a_slash_across_the_cord_cuts_it() {
        assert_eq!(
            cut_with(BACK),
            Some(EndWhy::Cut),
            "the swing crosses the cord"
        );
        assert_eq!(cut_with(0), None, "a swing away from the cord leaves it");
    }

    #[test]
    fn a_second_press_lets_the_man_go() {
        let (mut sim, _) = snared(3.0);
        // The catch freezes the bout for three ticks; a press then is lost.
        run(&mut sim, 4, [STILL, FACES_THROWER]);
        tick(&mut sim, &[(THROW_NOOSE, 0, 0, 0), FACES_THROWER]);
        assert_eq!(first_end(&events(&sim)), Some(EndWhy::Released));
        assert!(matches!(phase(&sim, 0), Phase::Idle));
    }

    #[test]
    fn a_guard_facing_the_thrower_turns_the_loop_away() {
        let mut sim = armed();
        place(&mut sim, [0.0, -4.0], [0.0, 0.0]);
        tick(&mut sim, &[(THROW_NOOSE, 0, 0, 0), (BLOCK, BACK, 0, 0)]);
        let mut seen = events(&sim);
        seen.extend(run(&mut sim, 20, [STILL, (BLOCK, BACK, 0, 0)]));
        assert!(
            seen.iter()
                .any(|e| matches!(e, SpearEvent::Deflected { victim: 1, .. })),
            "{seen:?}"
        );
        assert!(!caught(&seen));
        assert!(matches!(phase(&sim, 0), Phase::Idle));
    }

    #[test]
    fn each_button_throws_its_own_weapon() {
        let thrown = |button: u8| {
            let mut sim = armed();
            place(&mut sim, [0.0, -4.0], [0.0, 3.0]);
            tick(&mut sim, &[(button, 0, 0, 0), STILL]);
            match phase(&sim, 0) {
                Phase::Flight(f) => Some(f.noose),
                _ => None,
            }
        };
        assert_eq!(
            thrown(THROW_SPEAR),
            Some(false),
            "the spear button throws a spear"
        );
        assert_eq!(
            thrown(THROW_NOOSE),
            Some(true),
            "the noose button throws a loop"
        );
        // A noose-only match has no spear to throw.
        let mut only = build(false, true);
        place(&mut only, [0.0, -4.0], [0.0, 3.0]);
        tick(&mut only, &[(THROW_SPEAR, 0, 0, 0), STILL]);
        assert!(matches!(phase(&only, 0), Phase::Idle));
        assert!(events(&only).is_empty());
        tick(&mut only, &[(THROW_NOOSE, 0, 0, 0), STILL]);
        assert!(matches!(phase(&only, 0), Phase::Flight(f) if f.noose));
    }

    #[test]
    fn noose_off_ignores_the_button_and_latches_nothing() {
        // A spear-only match: the noose bit is not latched, nothing flies.
        let mut spear_only = build(true, false);
        place(&mut spear_only, [0.0, -4.0], [0.0, 3.0]);
        tick(&mut spear_only, &[(THROW_NOOSE, 0, 0, 0), STILL]);
        assert!(matches!(phase(&spear_only, 0), Phase::Idle));
        assert_eq!(
            spear_only.buttons[0] & THROW_NOOSE,
            0,
            "masked out of the hash"
        );
        // A match with no throwing weapon at all masks it too.
        let mut bare = build(false, false);
        tick(&mut bare, &[(THROW_NOOSE, 0, 0, 0), STILL]);
        assert_eq!(bare.buttons[0] & THROW_NOOSE, 0);
        // Armed, the bit is latched like the spear's.
        let mut both = armed();
        tick(&mut both, &[(THROW_NOOSE, 0, 0, 0), STILL]);
        assert_eq!(both.buttons[0] & THROW_NOOSE, THROW_NOOSE);
    }
}
