//! spine_schedule: the five spine systems registered on the fixed phases.
//!
//! Owner: engine-ecs. Consumer: the arena spine step registers one closure
//! per [`SpineSystem`] and runs the built [`Schedule`] once per tick. The
//! order is code, never data: inputs then ai on `PreUpdate`, rules then
//! motion on `Update`, extract on `PostUpdate`. A second registration of
//! the same system refuses, a missing system refuses at build, so a
//! schedule either holds exactly the five systems in [`SpineSystem::ORDER`]
//! or does not exist. Postcondition of the old systems-module deletion:
//! this module builds only on `schedule.rs`; a `systems.rs` is never
//! imported (pinned by a test).
//!
//! Float quarantine (ARC14): the spine step takes integer-nanosecond dt
//! (`engine_core::time` shape). [`SpineSchedule::register_ns`] systems read
//! [`SpineCtx::dt_ns`] and never `schedule::Ctx::dt`; [`SpineTick::run`]
//! takes `dt_ns: u64`. No float appears in this file outside its tests (the
//! quarantine test in `spine.rs` scans it); until the schedule itself takes
//! integer dt (ARC18) the underlying `Schedule::run` is fed its default
//! value, which no registered spine system reads.

use core::fmt;
use std::cell::Cell;
use std::rc::Rc;

use crate::schedule::{Ctx, Phase, Schedule, Tick};

/// One of the five spine systems, in run order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpineSystem {
    /// Tape inputs land on their slots (`PreUpdate`).
    Inputs,
    /// Brains fill the slots without a tape input (`PreUpdate`).
    Ai,
    /// Combat rules resolve the merged intents (`Update`).
    Rules,
    /// Locomotion plus collision move every live slot (`Update`).
    Motion,
    /// Checkpoint and extraction read the columns (`PostUpdate`).
    Extract,
}

impl SpineSystem {
    /// Every system in run order.
    pub const ORDER: [SpineSystem; 5] = [
        SpineSystem::Inputs,
        SpineSystem::Ai,
        SpineSystem::Rules,
        SpineSystem::Motion,
        SpineSystem::Extract,
    ];

    /// Stable system name (also the [`Schedule`] system name).
    pub fn name(self) -> &'static str {
        match self {
            SpineSystem::Inputs => "spine.inputs",
            SpineSystem::Ai => "spine.ai",
            SpineSystem::Rules => "spine.rules",
            SpineSystem::Motion => "spine.motion",
            SpineSystem::Extract => "spine.extract",
        }
    }

    /// The one phase this system may run in.
    pub fn phase(self) -> Phase {
        match self {
            SpineSystem::Inputs | SpineSystem::Ai => Phase::PreUpdate,
            SpineSystem::Rules | SpineSystem::Motion => Phase::Update,
            SpineSystem::Extract => Phase::PostUpdate,
        }
    }

    /// Index into [`SpineSystem::ORDER`].
    fn index(self) -> usize {
        match self {
            SpineSystem::Inputs => 0,
            SpineSystem::Ai => 1,
            SpineSystem::Rules => 2,
            SpineSystem::Motion => 3,
            SpineSystem::Extract => 4,
        }
    }
}

/// Registration failure. Loud `Err`, never panics; the schedule is not built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpineScheduleError {
    /// The system was already registered.
    Duplicate(SpineSystem),
    /// `build` found this system unregistered.
    Missing(SpineSystem),
}

impl fmt::Display for SpineScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate(sys) => write!(f, "spine system {} registered twice", sys.name()),
            Self::Missing(sys) => write!(f, "spine system {} is not registered", sys.name()),
        }
    }
}

impl std::error::Error for SpineScheduleError {}

type SysFn<W> = Box<dyn FnMut(&mut W, &mut Ctx)>;

/// What a spine system sees of one tick: the schedule tick, the phase and
/// the step length in whole nanoseconds. No float field, by design.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpineCtx {
    /// Schedule tick (1 on the first run).
    pub tick: Tick,
    /// Phase the system runs in.
    pub phase: Phase,
    /// Step length of this tick in nanoseconds.
    pub dt_ns: u64,
}

/// Builder holding at most one closure per [`SpineSystem`].
pub struct SpineSchedule<W> {
    slots: [Option<SysFn<W>>; 5],
    dt_ns: Rc<Cell<u64>>,
}

impl<W: 'static> Default for SpineSchedule<W> {
    fn default() -> Self {
        Self::new()
    }
}

impl<W: 'static> SpineSchedule<W> {
    /// Empty builder.
    pub fn new() -> Self {
        Self {
            slots: [None, None, None, None, None],
            dt_ns: Rc::new(Cell::new(0)),
        }
    }

    /// Register `system`; a second registration refuses and keeps the first.
    pub fn register(
        &mut self,
        system: SpineSystem,
        f: impl FnMut(&mut W, &mut Ctx) + 'static,
    ) -> Result<&mut Self, SpineScheduleError> {
        let slot = &mut self.slots[system.index()];
        if slot.is_some() {
            return Err(SpineScheduleError::Duplicate(system));
        }
        *slot = Some(Box::new(f));
        Ok(self)
    }

    /// Register `system` with the integer-ns view: the closure gets a
    /// [`SpineCtx`] whose `dt_ns` is the value passed to [`SpineTick::run`].
    /// Same refusal rules as [`SpineSchedule::register`].
    pub fn register_ns(
        &mut self,
        system: SpineSystem,
        mut f: impl FnMut(&mut W, SpineCtx) + 'static,
    ) -> Result<&mut Self, SpineScheduleError> {
        let dt_ns = Rc::clone(&self.dt_ns);
        self.register(system, move |world, cx| {
            f(
                world,
                SpineCtx {
                    tick: cx.tick,
                    phase: cx.phase,
                    dt_ns: dt_ns.get(),
                },
            );
        })
    }

    /// Registered system count.
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }

    /// True when nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Build the [`Schedule`]: every system in [`SpineSystem::ORDER`] on its
    /// own phase, or the first missing system named.
    pub fn build(self) -> Result<Schedule<W>, SpineScheduleError> {
        let mut schedule = Schedule::new();
        let mut slots = self.slots;
        for system in SpineSystem::ORDER {
            let f = slots[system.index()]
                .take()
                .ok_or(SpineScheduleError::Missing(system))?;
            schedule.add_system(system.phase(), system.name(), f);
        }
        Ok(schedule)
    }

    /// Build the integer-ns runner: every system registered, one
    /// [`SpineTick::run`] per tick.
    pub fn build_ns(self) -> Result<SpineTick<W>, SpineScheduleError> {
        let dt_ns = Rc::clone(&self.dt_ns);
        Ok(SpineTick {
            schedule: self.build()?,
            dt_ns,
        })
    }
}

/// A built spine schedule stepped by integer nanoseconds.
pub struct SpineTick<W> {
    schedule: Schedule<W>,
    dt_ns: Rc<Cell<u64>>,
}

impl<W> SpineTick<W> {
    /// Run one tick of `dt_ns` nanoseconds through the five systems in
    /// order. The step length reaches systems only as [`SpineCtx::dt_ns`].
    pub fn run(&mut self, world: &mut W, dt_ns: u64) {
        self.dt_ns.set(dt_ns);
        self.schedule.run(world, Default::default());
    }

    /// Ticks run so far.
    pub fn tick(&self) -> Tick {
        self.schedule.tick()
    }

    /// Tick on which `system` last ran, `None` before its first run.
    pub fn last_run(&self, system: SpineSystem) -> Option<Tick> {
        self.schedule.last_run(system.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    type Log = Rc<RefCell<Vec<(&'static str, Phase)>>>;
    type NsLog = Rc<RefCell<Vec<(&'static str, Tick, u64)>>>;

    fn logged(log: &Log, system: SpineSystem) -> impl FnMut(&mut (), &mut Ctx) + 'static {
        let log = Rc::clone(log);
        move |(), cx| log.borrow_mut().push((system.name(), cx.phase))
    }

    fn full(log: &Log) -> SpineSchedule<()> {
        let mut builder = SpineSchedule::new();
        // Register out of order on purpose: the build order is the law.
        for system in [
            SpineSystem::Extract,
            SpineSystem::Motion,
            SpineSystem::Inputs,
            SpineSystem::Rules,
            SpineSystem::Ai,
        ] {
            builder
                .register(system, logged(log, system))
                .expect("first registration lands");
        }
        builder
    }

    #[test]
    fn fixed_order_across_phases() {
        let log: Log = Rc::new(RefCell::new(vec![]));
        let mut schedule = full(&log).build().expect("five systems build");
        assert_eq!(schedule.len(), 5);
        schedule.run(&mut (), 0.016);
        let want: Vec<(&str, Phase)> = SpineSystem::ORDER
            .iter()
            .map(|s| (s.name(), s.phase()))
            .collect();
        assert_eq!(*log.borrow(), want);
        assert_eq!(
            want.iter().map(|(_, p)| *p).collect::<Vec<_>>(),
            [
                Phase::PreUpdate,
                Phase::PreUpdate,
                Phase::Update,
                Phase::Update,
                Phase::PostUpdate
            ]
        );
        // Two ticks keep the order and every system reports its last run.
        schedule.run(&mut (), 0.016);
        assert_eq!(log.borrow().len(), 10);
        for system in SpineSystem::ORDER {
            assert_eq!(schedule.last_run(system.name()), Some(2));
        }
    }

    #[test]
    fn duplicate_registration_refuses_and_keeps_the_first() {
        let log: Log = Rc::new(RefCell::new(vec![]));
        let mut builder = full(&log);
        let refused = builder
            .register(SpineSystem::Rules, |(), _| panic!("second rules ran"))
            .err()
            .expect("duplicate refuses");
        assert_eq!(refused, SpineScheduleError::Duplicate(SpineSystem::Rules));
        assert_eq!(
            refused.to_string(),
            "spine system spine.rules registered twice"
        );
        assert_eq!(builder.len(), 5);
        let mut schedule = builder.build().expect("still builds");
        schedule.run(&mut (), 0.016);
        assert_eq!(log.borrow().len(), 5, "the kept closure ran once");
    }

    #[test]
    fn missing_system_refuses_at_build() {
        let mut builder: SpineSchedule<()> = SpineSchedule::new();
        assert!(builder.is_empty());
        builder
            .register(SpineSystem::Inputs, |(), _| {})
            .expect("inputs lands");
        builder
            .register(SpineSystem::Rules, |(), _| {})
            .expect("rules lands");
        let refused = builder.build().err().expect("ai is missing");
        assert_eq!(refused, SpineScheduleError::Missing(SpineSystem::Ai));
        assert_eq!(
            refused.to_string(),
            "spine system spine.ai is not registered"
        );
    }

    #[test]
    fn spine_step_takes_integer_ns_dt() {
        let seen: NsLog = Rc::new(RefCell::new(vec![]));
        let mut builder: SpineSchedule<()> = SpineSchedule::new();
        for system in SpineSystem::ORDER {
            let seen = Rc::clone(&seen);
            builder
                .register_ns(system, move |(), cx| {
                    assert_eq!(cx.phase, system.phase());
                    seen.borrow_mut().push((system.name(), cx.tick, cx.dt_ns));
                })
                .expect("first registration lands");
        }
        let mut tick = builder.build_ns().expect("five systems build");
        // The exact 60 Hz step and a long frame: raw ns, no rounding.
        tick.run(&mut (), 16_666_666);
        tick.run(&mut (), 100_000_000);
        let seen = seen.borrow();
        assert_eq!(seen.len(), 10);
        assert!(seen[..5].iter().all(|s| (s.1, s.2) == (1, 16_666_666)));
        assert!(seen[5..].iter().all(|s| (s.1, s.2) == (2, 100_000_000)));
        let names: Vec<&str> = seen[..5].iter().map(|s| s.0).collect();
        let want: Vec<&str> = SpineSystem::ORDER.iter().map(|s| s.name()).collect();
        assert_eq!(names, want);
        assert_eq!(tick.tick(), 2);
        assert_eq!(tick.last_run(SpineSystem::Extract), Some(2));
    }

    #[test]
    fn register_ns_refuses_duplicates_and_missing_like_register() {
        let mut builder: SpineSchedule<()> = SpineSchedule::new();
        builder
            .register_ns(SpineSystem::Ai, |(), _| {})
            .expect("lands");
        let dup = builder
            .register_ns(SpineSystem::Ai, |(), _| {})
            .err()
            .expect("duplicate refuses");
        assert_eq!(dup, SpineScheduleError::Duplicate(SpineSystem::Ai));
        let missing = builder.build_ns().err().expect("four systems missing");
        assert_eq!(missing, SpineScheduleError::Missing(SpineSystem::Inputs));
    }

    #[test]
    fn sl4_postcondition_no_systems_module() {
        // The old `systems.rs` stays deleted; the spine builds on `schedule.rs` only.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/systems.rs");
        assert!(
            !path.exists(),
            "systems.rs must stay deleted: {}",
            path.display()
        );
        let lib = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs"),
        )
        .expect("lib.rs reads");
        assert!(
            !lib.contains("mod systems"),
            "lib.rs must not declare systems"
        );
    }
}
