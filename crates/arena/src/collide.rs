//! collide: integer fighter collision for the default leg (EG20).
//!
//! The default leg used to clamp each fighter into a square on its own, so
//! eight fighters could stand on one point and props never blocked (bodies
//! G6; geaen 12.5.3.6, gems3 ch 18). This pass runs once per tick after
//! every slot moved: fighter capsules push apart through the `controller2`
//! fighter push ([`separate_pair`], reused, not forked), fighter centres are
//! pushed out of static pillar boxes, and a round wall keeps every capsule
//! inside the arena circle.
//!
//! TC3: a body push is a shove. The push-apart displacement one fighter
//! takes in a tick feeds that fighter's balance an impulse scaled by the
//! push ([`PUSH_SHOVE_TABLE`]): a light blow holds the stance, a packed
//! crowd staggers, and a hard shove drops the fighter. The shove lands only
//! under rules v2 ([`ArenaRules::balance`]); rules v1 never runs the pass.
//!
//! Consumer: `Sim::step_with` under [`ArenaRules::collision`]
//! (rules v2 turns it on; v1 never runs it). Data owner: the caller owns the
//! walkers; pillars travel in the rules. Units: integer millimetres on the
//! XZ plane. Determinism: `+ - * /` and `isqrt` on integers only, pairs in
//! slot order, so the same poses give the same pushes on every PC. Never
//! panics; no allocation (the caller's slice is pushed in place).
//!
//! [`ArenaRules::collision`]: crate::ArenaRules::collision
//! [`separate_pair`]: engine_physics::controller2::separate_pair

use engine_physics::controller2::{separate_pair, Walker, RADIUS_MM};

/// Most relaxation passes per tick: each pass pushes every overlapping pair
/// half its overlap, then resolves pillars and the wall. The pass loop stops
/// at the first pass that moved no planar pose, because that pass left every
/// walker exactly as it found it and every later pass would run the same
/// integer pushes on the same inputs.
const COLLIDE_PASSES: usize = 4;

/// Most static pillar boxes one match carries.
pub const MAX_PILLARS: usize = 8;

/// Fighter capsule radius the collision pass keeps between bodies and from
/// pillars and the wall, in mm (the `controller2` walker radius).
pub const COLLIDE_RADIUS_MM: i32 = RADIUS_MM;

/// One static full-height pillar: an axis-aligned XZ footprint in mm.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pillar {
    /// Minimum corner `[x, z]` in mm.
    pub min_mm: [i32; 2],
    /// Maximum corner `[x, z]` in mm.
    pub max_mm: [i32; 2],
}

impl Pillar {
    /// Footprint with each lane ordered min to max.
    #[must_use]
    pub fn new(a: [i32; 2], b: [i32; 2]) -> Self {
        Self {
            min_mm: [a[0].min(b[0]), a[1].min(b[1])],
            max_mm: [a[0].max(b[0]), a[1].max(b[1])],
        }
    }
}

/// One row of the body-push shove table: the push edge in millimetres of one
/// tick's push-apart displacement and the impulse a push over that edge feeds
/// the balance.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PushShoveRow {
    /// The push edge in millimetres: a push over this edge lands the band.
    pub over_push_mm: i32,
    /// Impulse the band feeds the balance, Q8 impulse units.
    pub impulse_q8: u16,
}

/// The body-push shove table: the push bands, smallest edge first. A push
/// that does not pass the first edge lands no shove at all; a push past the
/// last edge lands the last row's impulse. Every edge is the push in
/// millimetres that matches the impulse in Q8, so one table holds the light
/// blow, the stagger, the stumble and the fall. The impulses are the
/// arena's own EG34 balance steps, so a body push reads like a landed blow.
pub const PUSH_SHOVE_TABLE: [PushShoveRow; 4] = [
    PushShoveRow {
        over_push_mm: 40,
        impulse_q8: crate::BALANCE_BLOCK_Q8,
    },
    PushShoveRow {
        over_push_mm: 256,
        impulse_q8: crate::BALANCE_PARRY_Q8,
    },
    PushShoveRow {
        over_push_mm: 384,
        impulse_q8: crate::BALANCE_HIT_Q8,
    },
    PushShoveRow {
        over_push_mm: 640,
        impulse_q8: crate::BALANCE_FALL_Q8,
    },
];

/// Resolve one tick of contacts in place: up to [`COLLIDE_PASSES`] passes of
/// pairwise push-apart in slot order, then pillar push-out, then the round
/// wall of `wall_radius_mm` (centre at the origin). A pass that moved no
/// walker is a fixed point, so the loop stops there and leaves the array the
/// full pass count would hold.
pub(crate) fn resolve(walkers: &mut [Walker], pillars: &[Pillar], wall_radius_mm: i32) {
    let mut before = [[0i32; 3]; crate::MAX_SLOTS];
    for _ in 0..COLLIDE_PASSES {
        copy_poses(walkers, &mut before);
        relax_pass(walkers, pillars, wall_radius_mm);
        if !poses_moved(walkers, &before) {
            break;
        }
    }
}

/// The pre-shortcut resolution, kept as the tick-for-tick equality oracle:
/// the full [`COLLIDE_PASSES`] count with no fixed-point stop. Test-only, so
/// the shipped tick carries no branch for it.
#[cfg(test)]
pub(crate) fn resolve_fixed_passes(
    walkers: &mut [Walker],
    pillars: &[Pillar],
    wall_radius_mm: i32,
) {
    for _ in 0..COLLIDE_PASSES {
        relax_pass(walkers, pillars, wall_radius_mm);
    }
}

/// One relaxation pass: every unordered pair pushes apart in slot order, then
/// every walker leaves the pillar footprints and the round wall.
fn relax_pass(walkers: &mut [Walker], pillars: &[Pillar], wall_radius_mm: i32) {
    for i in 0..walkers.len() {
        let (head, tail) = walkers.split_at_mut(i + 1);
        let Some(a) = head.last_mut() else {
            continue;
        };
        for b in tail.iter_mut() {
            separate_pair(a, b);
        }
    }
    for walker in walkers.iter_mut() {
        for pillar in pillars {
            push_out(walker, pillar);
        }
        hold_inside(walker, wall_radius_mm);
    }
}

/// Snapshot every walker's planar pose into `before`, for the fixed-point
/// test of [`resolve`]. Callers slice a `[Walker; MAX_SLOTS]` array, so the
/// snapshot always covers every walker [`poses_moved`] then compares.
fn copy_poses(walkers: &[Walker], before: &mut [[i32; 3]; crate::MAX_SLOTS]) {
    debug_assert!(
        walkers.len() <= crate::MAX_SLOTS,
        "walkers off the arena cap"
    );
    for (slot, walker) in walkers.iter().enumerate().take(crate::MAX_SLOTS) {
        before[slot] = walker.pos_mm;
    }
}

/// True when one [`relax_pass`] moved at least one walker's planar pose. The
/// pass touches planar positions only, so this decides the fixed point.
#[must_use]
fn poses_moved(walkers: &[Walker], before: &[[i32; 3]]) -> bool {
    walkers
        .iter()
        .zip(before.iter())
        .any(|(walker, was)| walker.pos_mm != *was)
}

/// Push a fighter centre out of a pillar footprint grown by the capsule
/// radius, along the axis of least penetration (x first on ties, then the
/// lower side), so the centre ends on the grown edge.
fn push_out(walker: &mut Walker, pillar: &Pillar) {
    let r = COLLIDE_RADIUS_MM;
    let (x, z) = (walker.pos_mm[0], walker.pos_mm[2]);
    let (x0, x1) = (pillar.min_mm[0] - r, pillar.max_mm[0] + r);
    let (z0, z1) = (pillar.min_mm[1] - r, pillar.max_mm[1] + r);
    if x <= x0 || x >= x1 || z <= z0 || z >= z1 {
        return;
    }
    let pushes = [
        (x - x0, 0, x0),
        (x1 - x, 0, x1),
        (z - z0, 2, z0),
        (z1 - z, 2, z1),
    ];
    let mut best = pushes[0];
    for push in pushes {
        if push.0 < best.0 {
            best = push;
        }
    }
    walker.pos_mm[best.1] = best.2;
}

/// Keep the capsule inside the round wall: a centre past the radius scales
/// back onto it (toward the origin), integer-only.
fn hold_inside(walker: &mut Walker, wall_radius_mm: i32) {
    let radius = i64::from(wall_radius_mm.max(0));
    let (x, z) = (i64::from(walker.pos_mm[0]), i64::from(walker.pos_mm[2]));
    let dist2 = u64::try_from(x * x + z * z).unwrap_or(u64::MAX);
    let radius2 = u64::try_from(radius * radius).unwrap_or(0);
    if dist2 <= radius2 {
        return;
    }
    let dist = i64::try_from(dist2.isqrt()).unwrap_or(i64::MAX).max(1);
    walker.pos_mm[0] = i32::try_from(x * radius / dist).unwrap_or(walker.pos_mm[0]);
    walker.pos_mm[2] = i32::try_from(z * radius / dist).unwrap_or(walker.pos_mm[2]);
}

/// EG20 collision pass: every fighter as an integer-mm walker, resolved by
/// [`resolve`] (push-apart, pillars, round wall at the square clamp's inset),
/// then only the integer displacement goes back onto the float pose, so an
/// untouched fighter keeps its exact bits. The spear step runs it again after
/// dragging bodies; a drag is not a body push, so it lands no shove.
pub(crate) fn collide_states(
    states: &mut [crate::FighterState],
    rules: &crate::ArenaRules,
    half_extent_m: f32,
    fighters: usize,
) {
    collide_states_with(states, None, rules, half_extent_m, fighters, resolve);
}

/// The tick's collision pass: [`collide_states`] plus the body-push shove.
/// The push-apart displacement each fighter took this tick feeds its balance
/// an impulse scaled by the push ([`PUSH_SHOVE_TABLE`]); the balance states
/// live in the tick, so the caller hands them in.
pub(crate) fn collide_states_shove(
    states: &mut [crate::FighterState],
    balance: &mut [crate::BalanceState],
    rules: &crate::ArenaRules,
    half_extent_m: f32,
    fighters: usize,
) {
    collide_states_with(
        states,
        Some(balance),
        rules,
        half_extent_m,
        fighters,
        resolve,
    );
}

/// [`collide_states`] with the pre-shortcut resolution: the full pass count
/// and no fixed-point stop. Test-only equality oracle for the tick-for-tick
/// proof; the shipped tick never calls it.
#[cfg(test)]
pub(crate) fn collide_states_fixed_passes(
    states: &mut [crate::FighterState],
    rules: &crate::ArenaRules,
    half_extent_m: f32,
    fighters: usize,
) {
    collide_states_with(
        states,
        None,
        rules,
        half_extent_m,
        fighters,
        resolve_fixed_passes,
    );
}

/// [`collide_states_shove`] with the pre-shortcut resolution: the tick-for-
/// tick equality oracle shoves exactly like the shipped tick, so the two
/// runs stay equal in balance as well as in poses.
#[cfg(test)]
pub(crate) fn collide_states_fixed_passes_shove(
    states: &mut [crate::FighterState],
    balance: &mut [crate::BalanceState],
    rules: &crate::ArenaRules,
    half_extent_m: f32,
    fighters: usize,
) {
    collide_states_with(
        states,
        Some(balance),
        rules,
        half_extent_m,
        fighters,
        resolve_fixed_passes,
    );
}

/// Shared body of [`collide_states`] and its test oracle. `resolve_passes` is
/// the resolution the caller pins: the shipped fixed-point stop, or the
/// oracle's fixed pass count. Both callers pass a function item, so the
/// indirect call folds into the one resolution at its call site. `balance` is
/// the caller's balance slice when the tick shoves bodies, `None` when the
/// caller owns no balance (the spear's drag).
fn collide_states_with(
    states: &mut [crate::FighterState],
    balance: Option<&mut [crate::BalanceState]>,
    rules: &crate::ArenaRules,
    half_extent_m: f32,
    fighters: usize,
    resolve_passes: fn(&mut [Walker], &[Pillar], i32),
) {
    let mut walkers = [Walker::spawn(0, [0; 3]); crate::MAX_SLOTS];
    for ((slot, walker), state) in walkers.iter_mut().enumerate().zip(states.iter()) {
        *walker = Walker::spawn(
            u32::try_from(slot).unwrap_or(u32::MAX),
            crate::match_wire::pos_mm(state.pos),
        );
    }
    let before = walkers;
    let mut pillars = [Pillar::new([0, 0], [0, 0]); MAX_PILLARS];
    let mut count = 0;
    for pillar in rules.pillars.iter().flatten() {
        pillars[count] = *pillar;
        count += 1;
    }
    let limit = crate::wall_limit(half_extent_m);
    let radius_mm = crate::match_wire::pos_mm([limit, 0.0, 0.0])[0];
    let fighters = fighters.min(crate::MAX_SLOTS);
    resolve_passes(&mut walkers[..fighters], &pillars[..count], radius_mm);
    if let Some(balance) = balance {
        if rules.balance {
            shove_bodies(balance, &before, &walkers, fighters);
        }
    }
    for ((state, after), was) in states.iter_mut().zip(&walkers).zip(&before) {
        for axis in [0, 2] {
            let moved = after.pos_mm[axis] - was.pos_mm[axis];
            if moved != 0 {
                let clamped = moved.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
                let metres = f32::from(i16::try_from(clamped).unwrap_or(0)) / 1000.0;
                state.pos[axis] = crate::canon_zero(state.pos[axis] + metres);
            }
        }
    }
}

/// The body-push shove: the push-apart displacement the collision pass gave
/// each fighter this tick, scaled by [`PUSH_SHOVE_TABLE`], feeds that
/// fighter's balance. A push under the table's threshold lands nothing; a
/// packed crowd staggers and a hard shove drops the fighter. The impulses
/// come from the table, so they stay under the balance ceiling and the
/// impact always lands. Rules v2 only: the caller gates the call on
/// [`crate::ArenaRules::balance`], so rules v1 never reaches a balance.
fn shove_bodies(
    balance: &mut [crate::BalanceState],
    before: &[Walker],
    after: &[Walker],
    fighters: usize,
) {
    let count = fighters
        .min(before.len())
        .min(after.len())
        .min(balance.len());
    for slot in 0..count {
        let push = push_mm(before[slot].pos_mm, after[slot].pos_mm);
        let impulse = shove_impulse_q8(push);
        if impulse == 0 {
            continue;
        }
        let _ = balance[slot].apply_impact(impulse);
    }
}

/// The push-apart displacement of one fighter in one tick, millimetres: the
/// length of the planar delta the collision pass applied. Integer only.
fn push_mm(before: [i32; 3], after: [i32; 3]) -> i32 {
    let dx = i64::from(after[0]) - i64::from(before[0]);
    let dz = i64::from(after[2]) - i64::from(before[2]);
    let dist2 = u64::try_from(dx * dx + dz * dz).unwrap_or(u64::MAX);
    i32::try_from(dist2.isqrt()).unwrap_or(i32::MAX)
}

/// The impulse one push feeds the balance: the band the push falls into, or
/// zero when the push does not pass the table's first edge. Integer only.
fn shove_impulse_q8(push_mm: i32) -> u16 {
    let mut impulse = 0u16;
    for row in PUSH_SHOVE_TABLE {
        if push_mm <= row.over_push_mm {
            break;
        }
        impulse = row.impulse_q8;
    }
    impulse
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walkers_at(count: u32, pos: [i32; 3]) -> Vec<Walker> {
        (0..count).map(|id| Walker::spawn(id, pos)).collect()
    }

    /// Rules-v2 8-fighter bout fixture: 600 ticks, v2 tables, so
    /// [`crate::ArenaConfig::rules`] pins rules v2 at build and the EG20
    /// collision pass runs every tick.
    fn v2_bout_config() -> crate::ArenaConfig {
        crate::ArenaConfig {
            seed: 2040,
            ticks: 600,
            fighters: 8,
            half_extent_m: 20.0,
            table_version: engine_combat::tables::TABLES_VERSION_V2,
        }
    }

    /// Round wall radius in mm exactly as [`collide_states_with`] reads it.
    fn wall_radius_mm(config: &crate::ArenaConfig) -> i32 {
        crate::match_wire::pos_mm([crate::wall_limit(config.half_extent_m), 0.0, 0.0])[0]
    }

    /// The integer walkers [`collide_states_with`] builds for `states`: slot
    /// order, planar pose in mm.
    fn walkers_of(states: &[crate::FighterState]) -> [Walker; crate::MAX_SLOTS] {
        let mut walkers = [Walker::spawn(0, [0; 3]); crate::MAX_SLOTS];
        for ((slot, walker), state) in walkers.iter_mut().enumerate().zip(states.iter()) {
            *walker = Walker::spawn(
                u32::try_from(slot).unwrap_or(u32::MAX),
                crate::match_wire::pos_mm(state.pos),
            );
        }
        walkers
    }

    /// True when [`resolve`]'s fixed-point stop fires after its first pass on
    /// `walkers` (pillar-free, like the fixture's rules).
    fn shortcut_taken(walkers: &[Walker], wall_radius_mm: i32) -> bool {
        let mut before = [[0i32; 3]; crate::MAX_SLOTS];
        copy_poses(walkers, &mut before);
        let mut probe = [Walker::spawn(0, [0; 3]); crate::MAX_SLOTS];
        probe[..walkers.len()].copy_from_slice(walkers);
        relax_pass(&mut probe[..walkers.len()], &[], wall_radius_mm);
        !poses_moved(&probe[..walkers.len()], &before)
    }

    #[test]
    fn coincident_walkers_spread_and_hold_the_wall() {
        let mut walkers = walkers_at(8, [0, 0, 0]);
        for _ in 0..30 {
            resolve(&mut walkers, &[], 19_600);
        }
        for (i, a) in walkers.iter().enumerate() {
            for b in &walkers[i + 1..] {
                let dx = i64::from(b.pos_mm[0] - a.pos_mm[0]);
                let dz = i64::from(b.pos_mm[2] - a.pos_mm[2]);
                assert!(dx * dx + dz * dz >= 4 * i64::from(RADIUS_MM).pow(2) - 4);
            }
        }
    }

    #[test]
    fn pillar_pushes_the_centre_out_and_the_wall_holds() {
        let pillar = Pillar::new([1_000, -500], [-1_000, 500]);
        assert_eq!(pillar.min_mm, [-1_000, -500]);
        let mut walker = Walker::spawn(0, [100, 0, 400]);
        push_out(&mut walker, &pillar);
        assert_eq!(walker.pos_mm, [100, 0, 500 + RADIUS_MM]);
        let mut far = Walker::spawn(1, [30_000, 0, 40_000]);
        hold_inside(&mut far, 10_000);
        assert_eq!([far.pos_mm[0], far.pos_mm[2]], [6_000, 8_000]);
    }

    /// The fixed-point stop against the pre-shortcut fixed pass count, over
    /// the walker rows of a real rules-v2 8-fighter bout: both resolutions
    /// leave the same walkers on every tick, and the stop really fires.
    #[test]
    fn fixed_point_stop_matches_the_fixed_pass_oracle_over_a_v2_bout() {
        let config = v2_bout_config();
        let tape = crate::FighterTape::default();
        let radius = wall_radius_mm(&config);
        let mut sim = crate::Sim::build(&config, &tape, false, false).expect("sim");
        let mut stopped = 0usize;
        let mut ticks = 0usize;
        while sim.tick < config.ticks {
            let row = walkers_of(&sim.states);
            let count = sim.fighters;
            let mut shipped = row;
            let mut oracle = row;
            resolve(&mut shipped[..count], &[], radius);
            resolve_fixed_passes(&mut oracle[..count], &[], radius);
            assert_eq!(shipped, oracle, "walkers diverged at tick {}", sim.tick);
            if shortcut_taken(&row[..count], radius) {
                stopped += 1;
            }
            ticks += 1;
            sim.step(&tape).expect("step");
        }
        assert_eq!(ticks, usize::try_from(config.ticks).unwrap_or(0));
        assert!(stopped > 0, "the fixture must take the fixed-point stop");
    }

    /// The whole tick, shipped against the fixed-pass oracle, over a real
    /// rules-v2 8-fighter bout: every tick both runs emit the same events in
    /// the same order, advance to the same fight state and poses and report
    /// the same checkpoint hash.
    #[test]
    fn v2_match_matches_the_fixed_pass_oracle_tick_for_tick() {
        let config = v2_bout_config();
        let tape = crate::FighterTape::default();
        let mut shipped = crate::Sim::build(&config, &tape, false, false).expect("shipped sim");
        let mut oracle = crate::Sim::build(&config, &tape, false, false).expect("oracle sim");
        oracle.full_collide_passes = true;
        let mut events = 0usize;
        while shipped.tick < config.ticks {
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

    /// The shove fixture: 30 ticks of eight fighters, so the EG20 collision
    /// pass runs every tick and a shove has time to land. `v2` picks the
    /// rules-v2 tables; rules v1 runs the same fight on the v1 tables.
    fn shove_config(v2: bool) -> crate::ArenaConfig {
        crate::ArenaConfig {
            seed: 2040,
            ticks: 30,
            fighters: 8,
            half_extent_m: 20.0,
            table_version: if v2 {
                engine_combat::tables::TABLES_VERSION_V2
            } else {
                engine_combat::tables::TABLES_VERSION_V1
            },
        }
    }

    /// A still tape: every slot asks for no motion on every tick, so the
    /// eight fighters stand where the test puts them and every push they
    /// take comes from the collision pass.
    fn still_tape(config: &crate::ArenaConfig) -> crate::FighterTape {
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            for slot in 0..config.fighters {
                inputs.push(crate::FighterInput {
                    tick,
                    fighter: crate::FighterId(slot),
                    mx: 0,
                    mz: 0,
                    facing: 0,
                    buttons: engine_input::Buttons::NONE,
                });
            }
        }
        crate::FighterTape(inputs)
    }

    /// One packed-crowd run under `rules`: every fighter starts on one point,
    /// the fight runs to the end, and `each` sees the sim after every tick.
    fn packed_run(
        config: &crate::ArenaConfig,
        tape: &crate::FighterTape,
        rules: crate::ArenaRules,
        mut each: impl FnMut(&crate::Sim),
    ) -> crate::Sim {
        let mut sim = crate::Sim::build(config, tape, false, false).expect("valid sim");
        assert!(sim.set_rules(rules), "the rules must pin before tick 0");
        for state in sim.states.iter_mut() {
            state.pos = [1.0, 0.0, 2.0];
        }
        while sim.tick < config.ticks {
            sim.step(tape).expect("valid step");
            each(&sim);
        }
        sim
    }

    /// The balance hashes of a run: one hash per slot, the same order as the
    /// slots.
    fn balance_hashes(sim: &crate::Sim) -> Vec<u64> {
        sim.balance
            .iter()
            .map(|balance| balance.state_hash())
            .collect()
    }

    /// The shoves a run landed: the balance's applied-shove counters summed.
    fn balance_shoves(sim: &crate::Sim) -> u64 {
        sim.balance
            .iter()
            .map(|balance| balance.shove_count())
            .sum()
    }

    /// TC3: a body push is a shove. Eight fighters spawned on one point under
    /// rules v2 push each other apart, and the push-apart displacement feeds
    /// the balance a shove scaled by the push: at least one fighter stumbles
    /// inside 30 ticks. Two runs agree on the balance hashes, and the same
    /// spawn under rules v1 lands no shove at all.
    #[test]
    fn packed_crowd_push_feeds_the_balance_a_shove() {
        let config = shove_config(true);
        let tape = still_tape(&config);
        let v2 = crate::ArenaRules::v2_free_for_all();
        let mut stumbled = false;
        let sim = packed_run(&config, &tape, v2, |sim| {
            stumbled |= sim
                .balance
                .iter()
                .any(|balance| balance.gait() == crate::GaitPhase::Stumbling);
        });
        let hashes = balance_hashes(&sim);
        let shoves = balance_shoves(&sim);
        println!(
            "packed crowd gaits: {:?}",
            sim.balance.iter().map(|b| b.gait()).collect::<Vec<_>>()
        );
        assert!(shoves > 0, "the push must reach the balance");
        assert!(stumbled, "a packed crowd must stumble inside 30 ticks");
        let mut again_stumbled = false;
        let again = packed_run(&config, &tape, v2, |sim| {
            again_stumbled |= sim
                .balance
                .iter()
                .any(|balance| balance.gait() == crate::GaitPhase::Stumbling);
        });
        assert_eq!(
            hashes,
            balance_hashes(&again),
            "two runs agree on the hashes"
        );
        assert_eq!(stumbled, again_stumbled);
        assert_eq!(shoves, balance_shoves(&again));
        let v1_config = shove_config(false);
        let v1_tape = still_tape(&v1_config);
        let mut v1_stumbled = false;
        let v1 = packed_run(&v1_config, &v1_tape, crate::ArenaRules::V1, |sim| {
            v1_stumbled |= sim
                .balance
                .iter()
                .any(|balance| balance.gait() == crate::GaitPhase::Stumbling);
        });
        assert_eq!(balance_shoves(&v1), 0, "rules v1 never shoves");
        assert!(!v1_stumbled, "rules v1 never stumbles from a push");
        assert_ne!(
            hashes,
            balance_hashes(&v1),
            "the shove must reach the v2 balance"
        );
    }

    /// Paired receipt: the p50 microseconds of one bout tick's collision
    /// resolution, shipped against the pre-shortcut oracle, on the walker rows
    /// of a real rules-v2 8-fighter bout; then the same pair end to end over
    /// 600 ticks of that bout. Both sides run interleaved so one load sits
    /// under each. Run: cargo test -p engine-arena --release --lib
    /// collide_paired_receipt -- --ignored --nocapture
    #[test]
    #[ignore = "timing: run with --ignored (release, one thread) for the receipt"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn collide_paired_receipt() {
        const SAMPLES: usize = 101;
        const TICK_REPS: usize = 31;
        let config = v2_bout_config();
        let tape = crate::FighterTape::default();
        let rules = config.rules();
        let radius = wall_radius_mm(&config);
        let mut warm = crate::Sim::build(&config, &tape, false, false).expect("warm sim");
        let count = warm.fighters;
        // One row per tick of the real bout: the tick's walkers and the float
        // poses they came from. `stopped` counts the ticks whose first pass is
        // already a fixed point, the ticks the shipped loop cuts short.
        let rows = usize::try_from(config.ticks).unwrap_or(0);
        let mut walker_rows = Vec::with_capacity(rows);
        let mut state_rows = Vec::with_capacity(rows);
        let mut stopped = 0usize;
        while warm.tick < config.ticks {
            let row = walkers_of(&warm.states);
            if shortcut_taken(&row[..count], radius) {
                stopped += 1;
            }
            walker_rows.push(row);
            state_rows.push(warm.states.clone());
            warm.step(&tape).expect("warm step");
        }
        let per_tick = f64::from(u32::try_from(walker_rows.len()).unwrap_or(1));
        let mut scratch = [Walker::spawn(0, [0; 3]); crate::MAX_SLOTS];
        let mut states = warm.states.clone();
        let mut new_us = Vec::with_capacity(SAMPLES);
        let mut old_us = Vec::with_capacity(SAMPLES);
        let mut new_states_us = Vec::with_capacity(SAMPLES);
        let mut old_states_us = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let begin = std::time::Instant::now();
            for row in &walker_rows {
                scratch.copy_from_slice(row);
                resolve(&mut scratch[..count], &[], radius);
                std::hint::black_box(&scratch);
            }
            new_us.push(begin.elapsed().as_secs_f64() * 1.0e6 / per_tick);
            let begin = std::time::Instant::now();
            for row in &walker_rows {
                scratch.copy_from_slice(row);
                resolve_fixed_passes(&mut scratch[..count], &[], radius);
                std::hint::black_box(&scratch);
            }
            old_us.push(begin.elapsed().as_secs_f64() * 1.0e6 / per_tick);
            let begin = std::time::Instant::now();
            for row in &state_rows {
                states.copy_from_slice(row);
                collide_states(&mut states, &rules, config.half_extent_m, count);
                std::hint::black_box(&states);
            }
            new_states_us.push(begin.elapsed().as_secs_f64() * 1.0e6 / per_tick);
            let begin = std::time::Instant::now();
            for row in &state_rows {
                states.copy_from_slice(row);
                collide_states_fixed_passes(&mut states, &rules, config.half_extent_m, count);
                std::hint::black_box(&states);
            }
            old_states_us.push(begin.elapsed().as_secs_f64() * 1.0e6 / per_tick);
        }
        let (new_p50, old_p50) = p50_pair(&mut new_us, &mut old_us, SAMPLES);
        eprintln!(
            "collide_resolve fixture=v2_8f rows={} shortcut_ticks={stopped} old_p50_us={old_p50:.4} \
             new_p50_us={new_p50:.4} ratio_x={:.3}",
            walker_rows.len(),
            old_p50 / new_p50
        );
        let (new_states, old_states) = p50_pair(&mut new_states_us, &mut old_states_us, SAMPLES);
        eprintln!(
            "collide_states fixture=v2_8f old_p50_us={old_states:.4} new_p50_us={new_states:.4} \
             ratio_x={:.3}",
            old_states / new_states
        );
        assert!(
            new_p50 <= old_p50,
            "the fixed-point stop must not be the slower resolution: {new_p50:.4} vs {old_p50:.4}"
        );
        assert!(
            new_states <= old_states,
            "the fixed-point stop must not be the slower collision pass: {new_states:.4} vs \
             {old_states:.4}"
        );
        let (new_match, old_match) = pair_matches(&config, &tape, TICK_REPS);
        eprintln!(
            "v2_match_600ticks fixture=8f old_p50_us={old_match:.4} shipped_p50_us={new_match:.4} \
             ratio_x={:.3}",
            old_match / new_match
        );
    }

    /// p50 of each side of one interleaved sweep, and the ratio-free pair.
    fn p50_pair(new_us: &mut [f64], old_us: &mut [f64], samples: usize) -> (f64, f64) {
        new_us.sort_by(|a, b| a.partial_cmp(b).expect("finite samples"));
        old_us.sort_by(|a, b| a.partial_cmp(b).expect("finite samples"));
        (new_us[samples / 2], old_us[samples / 2])
    }

    /// One interleaved pair of whole-bout runs, p50 microseconds per tick of
    /// the shipped resolution against the fixed-pass oracle. Printed only:
    /// a bout-level p50 on a shared machine carries more load noise than the
    /// per-tick sweeps above, while the equality oracle proves the two runs
    /// hold the same state.
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn pair_matches(
        config: &crate::ArenaConfig,
        tape: &crate::FighterTape,
        reps: usize,
    ) -> (f64, f64) {
        let mut match_new = Vec::with_capacity(reps);
        let mut match_old = Vec::with_capacity(reps);
        for _ in 0..reps {
            let begin = std::time::Instant::now();
            let mut shipped = crate::Sim::build(config, tape, false, false).expect("shipped sim");
            while shipped.tick < config.ticks {
                shipped.step(tape).expect("shipped step");
            }
            match_new.push(begin.elapsed().as_secs_f64() * 1.0e6 / f64::from(config.ticks));
            let begin = std::time::Instant::now();
            let mut oracle = crate::Sim::build(config, tape, false, false).expect("oracle sim");
            oracle.full_collide_passes = true;
            while oracle.tick < config.ticks {
                oracle.step(tape).expect("oracle step");
            }
            match_old.push(begin.elapsed().as_secs_f64() * 1.0e6 / f64::from(config.ticks));
            assert_eq!(
                shipped.checkpoints.last(),
                oracle.checkpoints.last(),
                "the timed pair must agree on the final hash"
            );
        }
        p50_pair(&mut match_new, &mut match_old, reps)
    }
}
