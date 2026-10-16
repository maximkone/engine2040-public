//! schedule: fixed-order phases. std-only, deterministic.
use crate::archetype::World as ArchetypeWorld;
use crate::arena::{ArenaError, PhaseArena};
use engine_core::Rng;
/// Column mask for read/write sets: bit 1 is Position, bit 2 is Velocity.
/// Zero touches no hashed column and keeps existing callers building.
pub type ColumnMask = u8;
pub type Tick = u32;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Startup,
    PreUpdate,
    Update,
    PostUpdate,
}
impl Phase {
    pub const ORDER: [Phase; 4] = [
        Phase::Startup,
        Phase::PreUpdate,
        Phase::Update,
        Phase::PostUpdate,
    ];
}
/// Legacy two-way change check, frozen: true when `stamp` was written after
/// `since` (wrapping compare). Kept for the pre-existing tick test; new
/// systems compare through
/// [`is_newer_than`](crate::archetype::is_newer_than), which also retires
/// stamps older than the archetype
/// [`MAX_CHANGE_AGE`](crate::archetype::MAX_CHANGE_AGE).
#[inline]
pub fn changed_since(stamp: Tick, since: Tick) -> bool {
    let d = stamp.wrapping_sub(since);
    d != 0 && d < Tick::MAX / 2
}
pub struct Ctx {
    pub tick: Tick,
    pub dt: f32,
    pub phase: Phase,
}
type SysFn<W> = Box<dyn FnMut(&mut W, &mut Ctx)>;
struct Sys<W> {
    phase: Phase,
    name: &'static str,
    reads: ColumnMask,
    writes: ColumnMask,
    last_run: Tick,
    f: SysFn<W>,
}
/// Build refusal naming both systems of an unordered conflicting pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleError {
    /// Two systems in one phase share a column with no order edge.
    Conflict(&'static str, &'static str, Phase),
}
impl core::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Conflict(a, b, ph) => write!(
                f,
                "schedule conflict in {ph:?}: '{a}' and '{b}' share a column with no order edge"
            ),
        }
    }
}
impl std::error::Error for ScheduleError {}
/// Severity knob for the conflict check, std-only.
/// PORT(bevyengine/bevy:bevy_ecs 0.20.0 ScheduleBuildSettings.ambiguity_detection).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AmbiguityLevel {
    /// Refuse the build on the first unordered conflicting pair.
    #[default]
    Error,
    /// Pass the build; read the pairs with [`Schedule::ambiguities`].
    Warn,
    /// Skip the check entirely.
    Ignore,
}
pub struct Schedule<W> {
    tick: Tick,
    started: bool,
    systems: Vec<Sys<W>>,
    edges: Vec<(&'static str, &'static str)>,
    seed: Option<u64>,
    order: Vec<usize>,
    ambiguity: AmbiguityLevel,
    // Flat per-phase index lists in live run order; rebuilt with `order`.
    phase_order: [Vec<usize>; 4],
}
impl<W> Default for Schedule<W> {
    fn default() -> Self {
        Self::new()
    }
}
impl<W> Schedule<W> {
    pub fn new() -> Self {
        let mut out = Self {
            tick: 0,
            started: false,
            systems: vec![],
            edges: vec![],
            seed: None,
            order: vec![],
            ambiguity: AmbiguityLevel::Error,
            phase_order: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
        };
        out.set_ambiguity_level(AmbiguityLevel::Error);
        out
    }
    pub fn add_system(
        &mut self,
        phase: Phase,
        name: &'static str,
        f: impl FnMut(&mut W, &mut Ctx) + 'static,
    ) -> &mut Self {
        self.add_system_with_access(phase, name, 0, 0, f)
    }
    /// Declare columns (bit 1 Position, bit 2 Velocity).
    /// Caller: `crates/perf/src/main.rs:258` via `run_arena_tick`.
    pub fn add_system_with_access(
        &mut self,
        phase: Phase,
        name: &'static str,
        reads: ColumnMask,
        writes: ColumnMask,
        f: impl FnMut(&mut W, &mut Ctx) + 'static,
    ) -> &mut Self {
        self.systems.push(Sys {
            phase,
            name,
            reads,
            writes,
            last_run: 0,
            f: Box::new(f),
        });
        self.rebuild_order();
        self
    }
    /// Order edge `before` runs before `after` in the same phase.
    pub fn add_order(&mut self, before: &'static str, after: &'static str) -> &mut Self {
        self.edges.push((before, after));
        self.rebuild_order();
        self
    }
    /// Seeded shuffle of free orderings; `None` (default) keeps registration.
    pub fn set_shuffle_seed(&mut self, seed: u64) -> &mut Self {
        self.seed = Some(seed);
        self.rebuild_order();
        self
    }
    pub fn len(&self) -> usize {
        self.systems.len()
    }
    pub fn is_empty(&self) -> bool {
        self.systems.is_empty()
    }
    pub fn tick(&self) -> Tick {
        self.tick
    }
    pub fn last_run(&self, name: &str) -> Option<Tick> {
        self.systems
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.last_run)
    }
    fn conflicts(a: &Sys<W>, b: &Sys<W>) -> bool {
        (a.writes & (b.reads | b.writes)) != 0 || (b.writes & (a.reads | a.writes)) != 0
    }
    /// Set the knob; registration stays name-keyed.
    pub fn set_ambiguity_level(&mut self, level: AmbiguityLevel) -> &mut Self {
        self.ambiguity = level;
        self
    }
    /// Every unordered conflicting pair in registration order.
    pub fn ambiguities(&self) -> Vec<ScheduleError> {
        let mut out = Vec::new();
        for i in 0..self.systems.len() {
            for j in (i + 1)..self.systems.len() {
                let (a, b) = (&self.systems[i], &self.systems[j]);
                let edged = self
                    .edges
                    .iter()
                    .any(|(x, y)| (*x == a.name && *y == b.name) || (*x == b.name && *y == a.name));
                if a.phase == b.phase && Self::conflicts(a, b) && !edged {
                    out.push(ScheduleError::Conflict(a.name, b.name, a.phase));
                }
            }
        }
        out
    }
    /// Check under a level: Error refuses, Warn reports, Ignore skips.
    pub fn validate_with(
        &self,
        level: AmbiguityLevel,
    ) -> Result<Vec<ScheduleError>, ScheduleError> {
        let list = if level == AmbiguityLevel::Ignore {
            Vec::new()
        } else {
            self.ambiguities()
        };
        if level == AmbiguityLevel::Error {
            match list.first() {
                Some(first) => Err(*first),
                None => Ok(list),
            }
        } else {
            Ok(list)
        }
    }
    /// Build check under the knob: Error refuses, Warn and Ignore pass.
    pub fn validate(&self) -> Result<(), ScheduleError> {
        self.validate_with(self.ambiguity).map(|_| ())
    }
    fn rebuild_order(&mut self) {
        self.order.clear();
        for (pi, ph) in Phase::ORDER.iter().enumerate() {
            let mut rest: Vec<usize> = vec![];
            for (idx, s) in self.systems.iter().enumerate() {
                if s.phase == *ph {
                    rest.push(idx);
                }
            }
            let mut rng_opt = self
                .seed
                .map(|s| Rng::new(s).fork(u64::try_from(pi).unwrap_or(0)));
            while !rest.is_empty() {
                let mut ready: Vec<usize> = vec![];
                for (ri, c) in rest.iter().enumerate() {
                    let mut blocked = false;
                    for r in rest.iter() {
                        if *r == *c {
                            continue;
                        }
                        let (rn, cn) = (self.systems[*r].name, self.systems[*c].name);
                        if self.edges.iter().any(|(x, y)| *x == rn && *y == cn) {
                            blocked = true;
                            break;
                        }
                    }
                    if !blocked {
                        ready.push(ri);
                    }
                }
                let pick = match rng_opt.as_mut() {
                    Some(r) if ready.len() > 1 => {
                        let len = u64::try_from(ready.len()).unwrap_or(2);
                        let at = r.next_range(0, len);
                        ready[usize::try_from(at).unwrap_or(0)]
                    }
                    _ => ready.first().copied().unwrap_or(0),
                };
                self.order.push(rest.remove(pick));
            }
        }
        for (pi, ph) in Phase::ORDER.iter().enumerate() {
            self.phase_order[pi].clear();
            for k in 0..self.order.len() {
                let idx = self.order[k];
                if self.systems[idx].phase == *ph {
                    self.phase_order[pi].push(idx);
                }
            }
        }
    }
    pub fn run(&mut self, world: &mut W, dt: f32) {
        self.tick = self.tick.wrapping_add(1);
        let t = self.tick;
        for ph in Phase::ORDER {
            if ph == Phase::Startup && self.started {
                continue;
            }
            for k in 0..self.order.len() {
                let idx = self.order[k];
                if self.systems[idx].phase != ph {
                    continue;
                }
                let mut cx = Ctx {
                    tick: t,
                    dt,
                    phase: ph,
                };
                let sys = &mut self.systems[idx];
                (sys.f)(world, &mut cx);
                sys.last_run = t;
            }
        }
        self.started = true;
    }
}

impl Schedule<ArchetypeWorld> {
    /// Run one deterministic arena tick through per-phase scratch.
    ///
    /// Advances the world clock once, then visits each phase in
    /// [`Phase::ORDER`] (startup once): opens the phase with
    /// [`PhaseArena::begin_phase`], runs that phase's systems, and applies
    /// the phase scratch entry point for the archetype world (census after
    /// pre-update unless no kills are staged, staged-kill flush after update
    /// unless no kills are staged, byte checkpoint after post-update). Returns the checkpoint digest, which equals
    /// [`ArchetypeWorld::state_hash`] bit for bit.
    ///
    /// Units: ticks advance by one, `dt` is seconds. Determinism: phase
    /// order plus declared edges plus seed; no hash maps, no wall clock.
    /// Free systems shuffle by seed behind registration order by default.
    /// The tick also runs the threshold-gated stamp scan
    /// ([`ArchetypeWorld::check_change_ticks_if_due`]), which clamps stamps
    /// older than the maximum change age without moving any checkpoint.
    /// Errors: [`ArenaError::Overflow`] when the pre-size is short; phases
    /// already run keep their system effects, so reserve more up front and
    /// retry from the last checkpoint. Budget: O(systems plus slots) time,
    /// zero heap allocation after setup reserves.
    pub fn run_arena_tick(
        &mut self,
        world: &mut ArchetypeWorld,
        arena: &mut PhaseArena,
        dt: f32,
    ) -> Result<u64, ArenaError> {
        world.advance();
        self.tick = self.tick.wrapping_add(1);
        let t = self.tick;
        let mut checkpoint: u64 = 0;
        for (pi, ph) in Phase::ORDER.iter().enumerate() {
            if *ph == Phase::Startup && self.started {
                continue;
            }
            arena.begin_phase(*ph);
            for k in 0..self.phase_order[pi].len() {
                let idx = self.phase_order[pi][k];
                let mut cx = Ctx {
                    tick: t,
                    dt,
                    phase: *ph,
                };
                let sys = &mut self.systems[idx];
                (sys.f)(world, &mut cx);
                sys.last_run = t;
            }
            if *ph == Phase::PreUpdate {
                // Steady-state skip: the census worklist is cleared at the
                // next begin_phase before any Update system runs, so with no
                // staged kills it is a dead scan over the settled tail.
                // Skipping touches only scratch, never hashed state.
                if world.pending() != 0 {
                    world.collect_live_into(arena)?;
                }
            } else if *ph == Phase::Update {
                if world.pending() != 0 {
                    world.flush_with(arena)?;
                }
            } else if *ph == Phase::PostUpdate {
                checkpoint = world.checkpoint_with(arena)?;
            }
        }
        self.started = true;
        world.check_change_ticks_if_due();
        Ok(checkpoint)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    type Log = Rc<RefCell<Vec<&'static str>>>;
    fn logged(log: &Log, s: &'static str) -> impl FnMut(&mut (), &mut Ctx) + 'static {
        let log = Rc::clone(log);
        move |(), _| log.borrow_mut().push(s)
    }
    #[test]
    fn fixed_order_within_phase() {
        let log: Log = Rc::new(RefCell::new(vec![]));
        let mut s: Schedule<()> = Schedule::new();
        s.add_system(Phase::Update, "a", logged(&log, "a"));
        s.add_system(Phase::Update, "b", logged(&log, "b"));
        s.add_system(Phase::Update, "c", logged(&log, "c"));
        s.run(&mut (), 0.016);
        assert_eq!(*log.borrow(), vec!["a", "b", "c"]);
    }
    #[test]
    fn phase_index_cache_matches_live_run_order() {
        let log: Log = Rc::new(RefCell::new(vec![]));
        let mut s: Schedule<()> = Schedule::new();
        s.add_system(Phase::PreUpdate, "pre", logged(&log, "pre"));
        s.add_system(Phase::Update, "a", logged(&log, "a"));
        s.add_system(Phase::Update, "b", logged(&log, "b"));
        s.add_system(Phase::PostUpdate, "post", logged(&log, "post"));
        assert_eq!(s.phase_order[1], vec![0]);
        assert_eq!(s.phase_order[2], vec![1, 2]);
        assert_eq!(s.phase_order[3], vec![3]);
        assert!(s.phase_order[0].is_empty());
        s.run(&mut (), 0.016);
        assert_eq!(*log.borrow(), vec!["pre", "a", "b", "post"]);
        s.add_system(Phase::Update, "c", logged(&log, "c"));
        assert_eq!(s.phase_order[2], vec![1, 2, 4]);
        let mut flat: Vec<usize> = Vec::new();
        for pi in 0..4 {
            flat.extend(s.phase_order[pi].iter().copied());
        }
        assert_eq!(flat, s.order);
        log.borrow_mut().clear();
        s.run(&mut (), 0.016);
        assert_eq!(*log.borrow(), vec!["pre", "a", "b", "c", "post"]);
    }
    #[test]
    fn phase_order() {
        let log: Log = Rc::new(RefCell::new(vec![]));
        let mut s: Schedule<()> = Schedule::new();
        s.add_system(Phase::PostUpdate, "post", logged(&log, "post"));
        s.add_system(Phase::Update, "upd", logged(&log, "upd"));
        s.add_system(Phase::PreUpdate, "pre", logged(&log, "pre"));
        s.run(&mut (), 0.016);
        assert_eq!(*log.borrow(), vec!["pre", "upd", "post"]);
    }
    #[test]
    fn startup_runs_once() {
        let log: Log = Rc::new(RefCell::new(vec![]));
        let mut s: Schedule<()> = Schedule::new();
        s.add_system(Phase::Startup, "init", logged(&log, "init"));
        s.add_system(Phase::Update, "u", logged(&log, "u"));
        s.run(&mut (), 0.016);
        s.run(&mut (), 0.016);
        assert_eq!(*log.borrow(), vec!["init", "u", "u"]);
    }
    #[test]
    fn tick_and_changed_since() {
        let mut s: Schedule<()> = Schedule::new();
        s.add_system(Phase::Update, "u", |(), _| {});
        assert_eq!(s.tick(), 0);
        s.run(&mut (), 0.016);
        s.run(&mut (), 0.016);
        assert_eq!(s.tick(), 2);
        assert_eq!(s.last_run("u"), Some(2));
        assert!(changed_since(2, 1) && !changed_since(1, 1) && !changed_since(1, 2));
    }
    #[test]
    fn deterministic_double_run() {
        let mk = || {
            let log: Log = Rc::new(RefCell::new(vec![]));
            let mut s: Schedule<()> = Schedule::new();
            s.add_system(Phase::PreUpdate, "a", logged(&log, "a"));
            s.add_system(Phase::Update, "b", logged(&log, "b"));
            s.add_system(Phase::PostUpdate, "c", logged(&log, "c"));
            (s, log)
        };
        let (mut s1, l1) = mk();
        let (mut s2, l2) = mk();
        for _ in 0..3 {
            s1.run(&mut (), 0.016);
            s2.run(&mut (), 0.016);
        }
        assert_eq!(*l1.borrow(), *l2.borrow());
    }
    #[test]
    fn three_way_changed_since_run() {
        use crate::archetype::{is_newer_than, MAX_CHANGE_AGE};
        let now: Tick = 3_400_000_000;
        let last: Tick = now - 200_000;
        assert!(is_newer_than(now - 10, last, now));
        assert!(is_newer_than(now, last, now));
        assert!(!is_newer_than(0, last, now));
        assert!(!is_newer_than(last, last, now));
        // Older than MAX retires even when the system just ran.
        let old = now.wrapping_sub(MAX_CHANGE_AGE).wrapping_sub(1);
        assert!(!is_newer_than(old, now.wrapping_sub(1), now));
        // Wraps with the clock.
        assert!(is_newer_than(0, Tick::MAX - 1, 1));
        assert!(!is_newer_than(Tick::MAX - 1, Tick::MAX - 1, 1));
    }
    #[test]
    fn arena_tick_deterministic_and_scan_drift_free() {
        use crate::archetype::{Position, Velocity};
        use crate::arena::PhaseArena;
        fn seeded(slots: usize) -> ArchetypeWorld {
            let mut world = ArchetypeWorld::new();
            world.reserve(slots * 2);
            for i in 0..slots {
                let f = i as f32;
                world.spawn(
                    Some(Position { x: f * 0.25, y: -f }),
                    Some(Velocity { x: 0.5, y: -0.25 }),
                );
            }
            world
        }
        fn run_ticks(world: &mut ArchetypeWorld) -> Vec<u64> {
            let mut arena = PhaseArena::with_capacity(2048, 65536);
            arena.reserve(2048, 65536);
            let mut schedule: Schedule<ArchetypeWorld> = Schedule::new();
            schedule.add_system(Phase::Update, "integrate", |w: &mut ArchetypeWorld, ctx| {
                w.integrate(ctx.dt);
            });
            let mut out = Vec::with_capacity(8);
            for _ in 0..8 {
                out.push(
                    schedule
                        .run_arena_tick(world, &mut arena, 0.016)
                        .expect("tick fits the pre-size"),
                );
            }
            out
        }
        let mut a = seeded(64);
        let mut b = seeded(64);
        let (ha, hb) = (run_ticks(&mut a), run_ticks(&mut b));
        assert_eq!(ha, hb, "full-tick digests drifted between identical runs");
        assert!(
            ha.windows(2).any(|w| w[0] != w[1]),
            "script never moved the checkpoint"
        );
        // A scan that clamps nothing moves no checkpoint.
        let before = a.state_hash();
        assert_eq!(a.check_change_ticks(), 0);
        assert_eq!(a.state_hash(), before, "scan moved hashed state");
    }
    #[test]
    fn sched_missing_edge_refuses_naming_both() {
        let mut s: Schedule<()> = Schedule::new();
        s.add_system_with_access(Phase::Update, "pos_a", 0, 1, |(), _| {});
        s.add_system_with_access(Phase::Update, "pos_b", 0, 1, |(), _| {});
        let err = s.validate().expect_err("conflict without edge must refuse");
        assert_eq!(
            err,
            ScheduleError::Conflict("pos_a", "pos_b", Phase::Update)
        );
        let text = err.to_string();
        assert!(text.contains("pos_a") && text.contains("pos_b"), "{text}");
        s.add_order("pos_a", "pos_b");
        assert!(s.validate().is_ok());
    }
    #[test]
    fn sched_severity_warn_reports_and_ignore_skips() {
        let mut s: Schedule<()> = Schedule::new();
        s.add_system_with_access(Phase::Update, "pos_a", 0, 1, |(), _| {});
        s.add_system_with_access(Phase::Update, "pos_b", 0, 1, |(), _| {});
        let want = ScheduleError::Conflict("pos_a", "pos_b", Phase::Update);
        assert_eq!(s.validate_with(AmbiguityLevel::Error).unwrap_err(), want);
        let warn = s.validate_with(AmbiguityLevel::Warn).expect("Warn passes");
        assert_eq!(warn, vec![want]);
        assert!(s
            .validate_with(AmbiguityLevel::Ignore)
            .expect("Ignore")
            .is_empty());
        s.set_ambiguity_level(AmbiguityLevel::Warn);
        assert!(s.validate().is_ok());
        s.set_ambiguity_level(AmbiguityLevel::Ignore);
        assert!(s.validate().is_ok());
    }
    #[test]
    fn sched_shuffle_keeps_hash_across_50_seeds() {
        use crate::archetype::{Position, Velocity};
        use crate::arena::PhaseArena;
        let mut first = 0u64;
        let mut saw_noop_first = false;
        for seed in 0..50u64 {
            let mut world = ArchetypeWorld::new();
            world.reserve(16);
            for i in 0..8 {
                let f = i as f32;
                world.spawn(
                    Some(Position { x: f, y: -f }),
                    Some(Velocity { x: 0.5, y: 0.25 }),
                );
            }
            let mut arena = PhaseArena::with_capacity(32, 2048);
            arena.reserve(32, 2048);
            let log: Log = Rc::new(RefCell::new(vec![]));
            let (l1, l2) = (Rc::clone(&log), Rc::clone(&log));
            let mut s: Schedule<ArchetypeWorld> = Schedule::new();
            s.add_system_with_access(Phase::Update, "pos", 2, 1, move |w, cx| {
                l1.borrow_mut().push("pos");
                w.integrate(cx.dt);
            });
            s.add_system(Phase::Update, "noop", move |_, _| {
                l2.borrow_mut().push("noop");
            });
            s.set_shuffle_seed(seed);
            assert!(s.validate().is_ok());
            let mut hash = 0;
            for _ in 0..4 {
                hash = s
                    .run_arena_tick(&mut world, &mut arena, 0.016)
                    .expect("fits");
            }
            if seed == 0 {
                first = hash;
            } else {
                assert_eq!(hash, first, "seed {seed} moved the hash");
            }
            if log.borrow().first() == Some(&"noop") {
                saw_noop_first = true;
            }
        }
        assert!(saw_noop_first, "shuffle never reordered the free pair");
        assert_ne!(first, 0, "script never moved the checkpoint");
    }
    #[test]
    fn arena_tick_skips_settled_tail_census_when_no_kills() {
        use crate::archetype::{Position, Velocity};
        use crate::arena::PhaseArena;
        fn mixed_world() -> (ArchetypeWorld, Vec<crate::archetype::Entity>) {
            let mut world = ArchetypeWorld::new();
            world.reserve(64);
            for i in 0..8 {
                let f = i as f32;
                world.spawn(
                    Some(Position { x: f, y: -f }),
                    Some(Velocity { x: 0.5, y: -0.25 }),
                );
            }
            let mut tail = Vec::with_capacity(32);
            for k in 0..32 {
                let kf = k as f32;
                let x = (kf * 0.37) % 20.0 - 10.0;
                tail.push(world.spawn(
                    Some(Position { x, y: 0.5 }),
                    Some(Velocity { x: 0.0, y: 0.0 }),
                ));
            }
            (world, tail)
        }
        fn tick_state() -> (Schedule<ArchetypeWorld>, PhaseArena) {
            let mut schedule: Schedule<ArchetypeWorld> = Schedule::new();
            schedule.add_system(Phase::Update, "integrate", |w: &mut ArchetypeWorld, ctx| {
                w.integrate(ctx.dt);
            });
            let mut arena = PhaseArena::with_capacity(0, 4096);
            arena.reserve(0, 4096);
            (schedule, arena)
        }
        let (mut first, tail_first) = mixed_world();
        let before: Vec<u32> = tail_first
            .iter()
            .map(|e| first.get_pos(*e).expect("tail row is live").x.to_bits())
            .collect();
        let (mut schedule, mut arena) = tick_state();
        let mut first_hashes = Vec::with_capacity(4);
        for _ in 0..4 {
            let checkpoint = schedule
                .run_arena_tick(&mut first, &mut arena, 0.016)
                .expect("settled tail census must skip when no kills are staged");
            assert_eq!(checkpoint, first.state_hash());
            first_hashes.push(checkpoint);
        }
        for (e, bits) in tail_first.iter().zip(before.iter()) {
            let pos = first.get_pos(*e).expect("tail row stays live");
            assert_eq!(pos.x.to_bits(), *bits, "settled tail moved");
            assert_eq!(pos.y.to_bits(), 0.5f32.to_bits(), "settled tail lifted");
        }
        let (mut second, _) = mixed_world();
        let (mut schedule2, mut arena2) = tick_state();
        let mut second_hashes = Vec::with_capacity(4);
        for _ in 0..4 {
            second_hashes.push(
                schedule2
                    .run_arena_tick(&mut second, &mut arena2, 0.016)
                    .expect("repeat run fits the byte presize"),
            );
        }
        assert_eq!(first_hashes, second_hashes, "identical runs drifted");
        assert!(
            first_hashes.windows(2).any(|w| w[0] != w[1]),
            "dynamic prefix never moved the checkpoint"
        );
    }
}
