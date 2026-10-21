//! ossim sim: one deterministic tick over four CPUs, RAM, swap and one I/O
//! queue. Seeded with `engine-core`'s counter RNG and hashed with its FNV fold.

use engine_core::{CounterRng, Fnv64};

use crate::{
    Action, Config, Event, EventKind, IoEntry, LossReason, Outcome, Pending, ProcState, Process,
    Refusal, ScoreParts, Stage, CPU_SLOTS, KILLS_TO_REBOOT, MAX_PROCESSES, STARVATION_MAX,
};

/// Read-only snapshot for a UI or a test. The sim keeps running without it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    /// Ticks processed so far.
    pub tick: u64,
    /// Stage index.
    pub stage: u8,
    /// Process on each CPU slot, `None` when free.
    pub cpus: [Option<u32>; CPU_SLOTS],
    /// Every process slot, including `Done` records.
    pub processes: [Option<Process>; MAX_PROCESSES],
    /// Pages currently mapped in RAM.
    pub ram_pages_used: u16,
    /// RAM pages installed by the stage.
    pub ram_pages_cap: u16,
    /// Swap slots currently holding pages.
    pub swap_slots_used: u16,
    /// Swap slots owned by the stage.
    pub swap_slots_cap: u16,
    /// The single I/O queue; only the first `io_len` entries are valid.
    pub io_queue: [Option<IoEntry>; MAX_PROCESSES],
    /// Number of valid I/O entries.
    pub io_len: u8,
    /// Integer score from [`ScoreParts::total`].
    pub score: i32,
    /// Counts behind the score.
    pub parts: ScoreParts,
    /// Kills so far, capped at a `u8`.
    pub kills: u8,
    /// Stage result, `None` while running.
    pub outcome: Option<Outcome>,
}

impl State {
    /// Live processes: every state except `Done`.
    #[must_use]
    pub fn live(&self) -> usize {
        self.processes
            .iter()
            .flatten()
            .filter(|p| p.state != ProcState::Done)
            .count()
    }
}

/// One headless game. No globals, no wall clock, no floats.
#[derive(Clone, Debug)]
pub struct Sim {
    seed: u64,
    stage: Stage,
    cfg: Config,
    rng: CounterRng,
    draws: u64,
    tick: u64,
    cpus: [Option<u32>; CPU_SLOTS],
    processes: [Option<Process>; MAX_PROCESSES],
    ram_pages: u16,
    swap_slots: u16,
    ram_used: u16,
    swap_used: u16,
    io: [Option<IoEntry>; MAX_PROCESSES],
    io_len: u8,
    next_id: u32,
    parts: ScoreParts,
    outcome: Option<Outcome>,
    events: Vec<Event>,
}

impl Sim {
    /// New sim with default rules.
    #[must_use]
    pub fn new(seed: u64, stage: Stage) -> Self {
        Self::with_config(seed, stage, Config::default())
    }

    /// New sim with explicit rules knobs.
    #[must_use]
    pub fn with_config(seed: u64, stage: Stage, cfg: Config) -> Self {
        let mut sim = Self {
            seed,
            stage,
            cfg,
            rng: CounterRng::new(seed),
            draws: 0,
            tick: 0,
            cpus: [None; CPU_SLOTS],
            processes: [None; MAX_PROCESSES],
            ram_pages: stage.ram_pages(),
            swap_slots: stage.swap_slots.min(crate::SWAP_SLOTS),
            ram_used: 0,
            swap_used: 0,
            io: [None; MAX_PROCESSES],
            io_len: 0,
            next_id: 0,
            parts: ScoreParts::default(),
            outcome: None,
            events: Vec::with_capacity(8),
        };
        for _ in 0..stage.start_processes {
            sim.spawn(0);
        }
        sim
    }

    /// Apply at most one action, then advance one tick.
    ///
    /// A refused action leaves the state unchanged but the tick still runs:
    /// the caller sees the [`Refusal`] and time moves on. After the stage ends
    /// every action is [`Refusal::Finished`] and time stops.
    pub fn tick(&mut self, action: Option<Action>) -> Result<(), Refusal> {
        self.events.clear();
        if self.outcome.is_some() {
            return if action.is_some() {
                Err(Refusal::Finished)
            } else {
                Ok(())
            };
        }
        let now = self.tick;
        let refusal = match action {
            Some(action) => self.apply(action, now).err(),
            None => None,
        };
        self.step_timers(now);
        self.step_processes();
        self.step_spawns(now);
        self.step_ambient(now);
        self.step_stage(now);
        self.tick = now + 1;
        match refusal {
            Some(refusal) => Err(refusal),
            None => Ok(()),
        }
    }

    /// Read-only snapshot of everything a UI draws.
    #[must_use]
    pub fn state(&self) -> State {
        State {
            tick: self.tick,
            stage: self.stage.id,
            cpus: self.cpus,
            processes: self.processes,
            ram_pages_used: self.ram_used,
            ram_pages_cap: self.ram_pages,
            swap_slots_used: self.swap_used,
            swap_slots_cap: self.swap_slots,
            io_queue: self.io,
            io_len: self.io_len,
            score: self.parts.total(),
            parts: self.parts,
            kills: u8::try_from(self.parts.kills).unwrap_or(u8::MAX),
            outcome: self.outcome,
        }
    }

    /// Canonical state hash: seed, stage, tick, RNG, every process, CPU, page
    /// and queue field in fixed order. Same seed and tape give the same hash.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write_u8(1);
        h.write_u64(self.seed);
        h.write_u64(u64::from(self.stage.id));
        h.write_u64(self.tick);
        h.write_u64(self.draws);
        for word in self.rng.key() {
            h.write_u64(word);
        }
        h.write_u16(self.ram_used);
        h.write_u16(self.swap_used);
        h.write_u32(self.next_id);
        h.write_i32(self.parts.total());
        h.write_u32(self.parts.graceful);
        h.write_u32(self.parts.io_ready);
        h.write_u32(self.parts.kills);
        h.write_u32(self.parts.faults);
        h.write_bool(self.parts.won);
        for cpu in &self.cpus {
            h.write_bool(cpu.is_some());
            if let Some(id) = cpu {
                h.write_u32(*id);
            }
        }
        for slot in &self.processes {
            match slot {
                None => h.write_u8(0),
                Some(p) => {
                    h.write_u8(1);
                    h.write_u32(p.id);
                    h.write_u8(p.state.tag());
                    h.write_u8(p.starvation);
                    h.write_u8(p.pages);
                    h.write_bool(p.resident);
                    h.write_u8(p.pending.tag());
                    h.write_u64(p.swap_done);
                    h.write_u64(p.io_ready);
                    h.write_bool(p.priority);
                    h.write_u64(p.spawn_tick);
                    h.write_u32(p.wait_ticks);
                    h.write_u32(p.run_streak);
                    h.write_u32(p.worked);
                    h.write_u32(p.work);
                }
            }
        }
        for i in 0..usize::from(self.io_len) {
            if let Some(entry) = self.io[i] {
                h.write_u32(entry.process);
                h.write_u64(entry.ready_tick);
            }
        }
        h.write_u8(match self.outcome {
            None => 0,
            Some(Outcome::Won) => 1,
            Some(Outcome::Lost(LossReason::Reboot)) => 2,
            Some(Outcome::Lost(LossReason::TooFewProcesses)) => 3,
        });
        h.finish()
    }

    /// Integer score; equals `state().parts.total()`.
    #[must_use]
    pub fn score(&self) -> i32 {
        self.parts.total()
    }

    /// Counts behind the score.
    #[must_use]
    pub fn parts(&self) -> ScoreParts {
        self.parts
    }

    /// Stage result, `None` while running.
    #[must_use]
    pub fn outcome(&self) -> Option<Outcome> {
        self.outcome
    }

    /// Events produced since the last tick started. An injected
    /// [`Sim::fault`] adds to this list immediately.
    #[must_use]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Stage copy this sim runs.
    #[must_use]
    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// Live process count.
    #[must_use]
    pub fn live(&self) -> usize {
        self.processes
            .iter()
            .flatten()
            .filter(|p| p.state != ProcState::Done)
            .count()
    }

    /// Force a page fault on a running process, the same path ambient faults
    /// take. Tools, tests and a tutorial use it; a running process is required.
    /// The swap file must have room for the pages: a full swap refuses the
    /// fault with [`Refusal::NoSwap`] instead of over-committing the file.
    pub fn fault(&mut self, process: u32) -> Result<(), Refusal> {
        let Some((slot, p)) = self.locate(process) else {
            return Err(Refusal::NoSuchProcess);
        };
        if p.state != ProcState::Running || !p.resident {
            return Err(Refusal::WrongState);
        }
        self.fault_slot(slot)
    }

    fn apply(&mut self, action: Action, now: u64) -> Result<(), Refusal> {
        match action {
            Action::Assign { cpu, process } => self.assign(cpu, process),
            Action::Unassign { cpu } => self.unassign(cpu),
            Action::SwapIn { process } => self.swap_in(process, now),
            Action::SwapOut { process } => self.swap_out(process, now),
            Action::DoIo { process } => self.do_io(process, now),
        }
    }

    fn assign(&mut self, cpu: u8, process: u32) -> Result<(), Refusal> {
        let cpu = usize::from(cpu);
        if cpu >= CPU_SLOTS {
            return Err(Refusal::BadCpu);
        }
        if self.cpus[cpu].is_some() {
            return Err(Refusal::CpuBusy);
        }
        let Some((slot, mut p)) = self.locate(process) else {
            return Err(Refusal::NoSuchProcess);
        };
        if p.state != ProcState::Idle || p.pending != Pending::None || !p.resident {
            return Err(Refusal::WrongState);
        }
        p.state = ProcState::Running;
        p.run_streak = 0;
        p.wait_ticks = 0;
        self.processes[slot] = Some(p);
        self.cpus[cpu] = Some(process);
        Ok(())
    }

    fn unassign(&mut self, cpu: u8) -> Result<(), Refusal> {
        let cpu = usize::from(cpu);
        if cpu >= CPU_SLOTS {
            return Err(Refusal::BadCpu);
        }
        let Some(id) = self.cpus[cpu].take() else {
            return Err(Refusal::CpuEmpty);
        };
        if let Some((slot, mut p)) = self.locate(id) {
            p.state = ProcState::Idle;
            p.run_streak = 0;
            self.processes[slot] = Some(p);
        }
        Ok(())
    }

    fn swap_in(&mut self, process: u32, now: u64) -> Result<(), Refusal> {
        let Some((slot, mut p)) = self.locate(process) else {
            return Err(Refusal::NoSuchProcess);
        };
        if p.pending != Pending::None {
            return Err(Refusal::Busy);
        }
        if p.resident || p.state != ProcState::BlockedPage {
            return Err(Refusal::WrongState);
        }
        if self.ram_used + u16::from(p.pages) > self.ram_pages {
            return Err(Refusal::NoRam);
        }
        self.swap_used = self.swap_used.saturating_sub(u16::from(p.pages));
        self.ram_used += u16::from(p.pages);
        p.pending = Pending::SwapIn;
        p.swap_done = now + self.swap_ticks();
        p.io_ready = 0;
        self.processes[slot] = Some(p);
        Ok(())
    }

    fn swap_out(&mut self, process: u32, now: u64) -> Result<(), Refusal> {
        let Some((slot, mut p)) = self.locate(process) else {
            return Err(Refusal::NoSuchProcess);
        };
        if p.pending != Pending::None {
            return Err(Refusal::Busy);
        }
        if !p.resident || !matches!(p.state, ProcState::Idle | ProcState::Running) {
            return Err(Refusal::WrongState);
        }
        // The swap file must hold the pages when they land, so a swap already
        // on its way counts against the room: two swaps in flight may not
        // both pass a check that only looks at the pages already in swap.
        if self.swap_committed() + u16::from(p.pages) > self.swap_slots {
            return Err(Refusal::NoSwap);
        }
        if let Some(cpu) = self.cpu_of(p.id) {
            self.cpus[cpu] = None;
        }
        p.state = ProcState::BlockedPage;
        p.pending = Pending::SwapOut;
        p.swap_done = now + self.swap_ticks();
        p.run_streak = 0;
        self.processes[slot] = Some(p);
        Ok(())
    }

    fn do_io(&mut self, process: u32, now: u64) -> Result<(), Refusal> {
        let Some((slot, p)) = self.locate(process) else {
            return Err(Refusal::NoSuchProcess);
        };
        if p.state != ProcState::Running {
            return Err(Refusal::WrongState);
        }
        if usize::from(self.io_len) >= MAX_PROCESSES {
            return Err(Refusal::QueueFull);
        }
        if let Some(cpu) = self.cpu_of(p.id) {
            self.cpus[cpu] = None;
        }
        let ready = now + self.io_ticks();
        self.push_io(p.id, ready);
        let mut blocked = p;
        blocked.state = ProcState::BlockedIo;
        blocked.io_ready = ready;
        blocked.run_streak = 0;
        self.processes[slot] = Some(blocked);
        self.events.push(Event {
            tick: now,
            kind: EventKind::IoRequest,
            process,
        });
        Ok(())
    }

    fn step_timers(&mut self, now: u64) {
        let mut slot = 0;
        while slot < MAX_PROCESSES {
            if let Some(p) = self.processes[slot] {
                if p.pending != Pending::None
                    && p.state == ProcState::BlockedPage
                    && now >= p.swap_done
                {
                    self.complete_swap(slot, now);
                }
            }
            slot += 1;
        }
        self.drain_io(now);
    }

    fn complete_swap(&mut self, slot: usize, now: u64) {
        let Some(mut p) = self.processes[slot] else {
            return;
        };
        match p.pending {
            Pending::SwapOut => {
                self.ram_used = self.ram_used.saturating_sub(u16::from(p.pages));
                self.swap_used += u16::from(p.pages);
                p.resident = false;
            }
            Pending::SwapIn => {
                p.resident = true;
                p.state = ProcState::Idle;
            }
            Pending::None => return,
        }
        p.pending = Pending::None;
        p.swap_done = 0;
        let id = p.id;
        self.processes[slot] = Some(p);
        self.events.push(Event {
            tick: now,
            kind: EventKind::SwapDone,
            process: id,
        });
    }

    fn drain_io(&mut self, now: u64) {
        let mut i = 0;
        while i < usize::from(self.io_len) {
            let Some(entry) = self.io[i] else {
                i += 1;
                continue;
            };
            if entry.ready_tick > now {
                i += 1;
                continue;
            }
            self.parts.io_ready += 1;
            self.events.push(Event {
                tick: now,
                kind: EventKind::IoReady,
                process: entry.process,
            });
            if let Some((slot, mut p)) = self.locate(entry.process) {
                if p.state == ProcState::BlockedIo {
                    p.state = ProcState::Idle;
                    p.io_ready = now;
                    self.processes[slot] = Some(p);
                }
            }
            let len = usize::from(self.io_len);
            let mut j = i;
            while j + 1 < len {
                self.io[j] = self.io[j + 1];
                j += 1;
            }
            self.io_len -= 1;
            self.io[usize::from(self.io_len)] = None;
        }
    }

    fn step_processes(&mut self) {
        let happy = self.cfg.happy_running_ticks;
        let normal = self.cfg.starvation_ticks;
        let priority = self.cfg.starvation_ticks_priority;
        let mut slot = 0;
        while slot < MAX_PROCESSES {
            let Some(mut p) = self.processes[slot] else {
                slot += 1;
                continue;
            };
            match p.state {
                ProcState::Running => {
                    p.run_streak += 1;
                    p.worked += 1;
                    if p.run_streak >= happy {
                        p.run_streak = 0;
                        p.starvation = 0;
                    }
                    if p.worked >= p.work {
                        self.processes[slot] = Some(p);
                        self.finish(slot, EventKind::Graceful);
                        slot += 1;
                        continue;
                    }
                }
                ProcState::Idle => {
                    p.wait_ticks += 1;
                    let threshold = if p.priority { priority } else { normal };
                    if threshold > 0 && p.wait_ticks >= threshold {
                        p.wait_ticks = 0;
                        p.starvation = p.starvation.saturating_add(1);
                        if p.starvation >= STARVATION_MAX {
                            self.processes[slot] = Some(p);
                            self.finish(slot, EventKind::Kill);
                            slot += 1;
                            continue;
                        }
                    }
                }
                _ => {}
            }
            self.processes[slot] = Some(p);
            slot += 1;
        }
    }

    fn step_spawns(&mut self, now: u64) {
        let stage = self.stage;
        if stage.spawn_count == 0 || stage.spawn_every == 0 {
            return;
        }
        let first = u64::from(stage.spawn_first);
        let every = u64::from(stage.spawn_every);
        let last = first + every * u64::from(stage.spawn_count - 1);
        if now >= first && now <= last && (now - first).is_multiple_of(every) {
            self.spawn(now);
        }
    }

    fn step_ambient(&mut self, now: u64) {
        let period = self.cfg.ambient_period;
        if period == 0 || now == 0 || !now.is_multiple_of(period) {
            return;
        }
        let running = self
            .processes
            .iter()
            .flatten()
            .filter(|p| p.state == ProcState::Running)
            .count();
        if running == 0 {
            return;
        }
        let word = self.draw();
        let pick =
            usize::try_from((word >> 8) % u64::try_from(running).unwrap_or(u64::MAX)).unwrap_or(0);
        let mut seen = 0;
        let mut chosen = None;
        let mut slot = 0;
        while slot < MAX_PROCESSES {
            if let Some(p) = self.processes[slot] {
                if p.state == ProcState::Running {
                    if seen == pick {
                        chosen = Some(slot);
                        break;
                    }
                    seen += 1;
                }
            }
            slot += 1;
        }
        let Some(slot) = chosen else {
            return;
        };
        match word % 3 {
            0 => {
                let _ = self.fault_slot(slot);
            }
            1 => self.request_io_slot(slot, now),
            _ => {}
        }
    }

    fn step_stage(&mut self, now: u64) {
        if self.outcome.is_some() {
            return;
        }
        let now32 = u32::try_from(now).unwrap_or(u32::MAX);
        for &(at, min) in self.stage.min_curve {
            if at == now32 && self.live() < usize::from(min) {
                self.outcome = Some(Outcome::Lost(LossReason::TooFewProcesses));
                return;
            }
        }
        let limit = if self.stage.max_kills == 0 {
            u32::from(KILLS_TO_REBOOT)
        } else {
            u32::from(self.stage.max_kills)
        };
        if self.parts.kills >= limit {
            self.outcome = Some(Outcome::Lost(LossReason::Reboot));
            return;
        }
        if now + 1 >= u64::from(self.stage.win_tick) {
            self.parts.won = true;
            self.outcome = Some(Outcome::Won);
        }
    }

    fn spawn(&mut self, now: u64) {
        let Some(slot) = self.free_slot() else {
            return;
        };
        let word = self.draw();
        let pages = 1 + (word % 6) as u8;
        let pages16 = u16::from(pages);
        let resident = if self.ram_used + pages16 <= self.ram_pages {
            self.ram_used += pages16;
            true
        } else if self.swap_committed() + pages16 <= self.swap_slots {
            self.swap_used += pages16;
            false
        } else {
            return;
        };
        let id = self.next_id;
        self.next_id += 1;
        let priority = self.stage.priority_every != 0
            && id.is_multiple_of(u32::from(self.stage.priority_every));
        let process = Process {
            id,
            slot: u8::try_from(slot).unwrap_or(u8::MAX),
            state: if resident {
                ProcState::Idle
            } else {
                ProcState::BlockedPage
            },
            starvation: 0,
            pages,
            resident,
            pending: Pending::None,
            swap_done: 0,
            io_ready: 0,
            priority,
            spawn_tick: now,
            wait_ticks: 0,
            run_streak: 0,
            worked: 0,
            work: 600 + ((word >> 8) % 900) as u32,
        };
        self.processes[slot] = Some(process);
        self.events.push(Event {
            tick: now,
            kind: EventKind::Spawn,
            process: id,
        });
    }

    fn request_io_slot(&mut self, slot: usize, now: u64) {
        let Some(p) = self.processes[slot] else {
            return;
        };
        if p.state != ProcState::Running {
            return;
        }
        if usize::from(self.io_len) >= MAX_PROCESSES {
            return;
        }
        if let Some(cpu) = self.cpu_of(p.id) {
            self.cpus[cpu] = None;
        }
        let ready = now + self.io_ticks();
        self.push_io(p.id, ready);
        let mut blocked = p;
        blocked.state = ProcState::BlockedIo;
        blocked.io_ready = ready;
        blocked.run_streak = 0;
        self.processes[slot] = Some(blocked);
        self.events.push(Event {
            tick: now,
            kind: EventKind::IoRequest,
            process: p.id,
        });
    }

    fn fault_slot(&mut self, slot: usize) -> Result<(), Refusal> {
        let Some(p) = self.processes[slot] else {
            return Err(Refusal::NoSuchProcess);
        };
        if p.state != ProcState::Running || !p.resident {
            return Err(Refusal::WrongState);
        }
        // A page fault has to land in the swap file. A full swap refuses the
        // fault: the process keeps running instead of pushing the swap file
        // past the slots the stage owns.
        if self.swap_committed() + u16::from(p.pages) > self.swap_slots {
            return Err(Refusal::NoSwap);
        }
        if let Some(cpu) = self.cpu_of(p.id) {
            self.cpus[cpu] = None;
        }
        let mut faulted = p;
        faulted.state = ProcState::BlockedPage;
        faulted.pending = Pending::SwapOut;
        faulted.swap_done = self.tick + self.swap_ticks();
        faulted.run_streak = 0;
        self.processes[slot] = Some(faulted);
        self.parts.faults += 1;
        self.events.push(Event {
            tick: self.tick,
            kind: EventKind::PageFault,
            process: p.id,
        });
        Ok(())
    }

    fn finish(&mut self, slot: usize, kind: EventKind) {
        let Some(mut p) = self.processes[slot] else {
            return;
        };
        if p.state == ProcState::Done {
            return;
        }
        if p.resident {
            self.ram_used = self.ram_used.saturating_sub(u16::from(p.pages));
        } else {
            self.swap_used = self.swap_used.saturating_sub(u16::from(p.pages));
        }
        if let Some(cpu) = self.cpu_of(p.id) {
            self.cpus[cpu] = None;
        }
        p.state = ProcState::Done;
        p.pending = Pending::None;
        p.swap_done = 0;
        p.run_streak = 0;
        self.processes[slot] = Some(p);
        match kind {
            EventKind::Graceful => self.parts.graceful += 1,
            EventKind::Kill => self.parts.kills += 1,
            _ => {}
        }
        self.events.push(Event {
            tick: self.tick,
            kind,
            process: p.id,
        });
    }

    fn push_io(&mut self, process: u32, ready: u64) {
        let index = usize::from(self.io_len);
        self.io[index] = Some(IoEntry {
            process,
            ready_tick: ready,
        });
        self.io_len += 1;
    }

    fn locate(&self, process: u32) -> Option<(usize, Process)> {
        for (slot, candidate) in self.processes.iter().enumerate() {
            if let Some(p) = candidate {
                if p.state != ProcState::Done && p.id == process {
                    return Some((slot, *p));
                }
            }
        }
        None
    }

    fn cpu_of(&self, process: u32) -> Option<usize> {
        self.cpus.iter().position(|cpu| *cpu == Some(process))
    }

    /// Pages the swap file must hold right now: the ones already in swap
    /// plus the ones on their way there (a swap-out in flight claims its
    /// pages, they are counted when the swap lands).
    fn swap_committed(&self) -> u16 {
        let in_flight = self
            .processes
            .iter()
            .flatten()
            .filter(|p| p.state != ProcState::Done && p.pending == Pending::SwapOut)
            .map(|p| u16::from(p.pages))
            .fold(0u16, |sum, pages| sum.saturating_add(pages));
        self.swap_used.saturating_add(in_flight)
    }

    fn free_slot(&self) -> Option<usize> {
        self.processes.iter().position(|slot| match slot {
            None => true,
            Some(p) => p.state == ProcState::Done,
        })
    }

    fn draw(&mut self) -> u64 {
        let word = self.rng.sample(self.draws);
        self.draws += 1;
        word
    }

    fn swap_ticks(&mut self) -> u64 {
        let (lo, hi) = self.stage.swap_ticks;
        let lo = lo.max(crate::SWAP_TICKS_MIN);
        let hi = hi.clamp(crate::SWAP_TICKS_MIN, crate::SWAP_TICKS_MAX);
        self.range(u64::from(lo), u64::from(hi.max(lo)))
    }

    fn io_ticks(&mut self) -> u64 {
        let (lo, hi) = self.cfg.io_ticks;
        let lo = lo.max(crate::IO_TICKS_MIN);
        let hi = hi.clamp(crate::IO_TICKS_MIN, crate::IO_TICKS_MAX);
        self.range(u64::from(lo), u64::from(hi.max(lo)))
    }

    /// Inclusive range draw; equal bounds consume a word so tapes stay stable.
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        let width = hi - lo + 1;
        lo + self.draw() % width
    }
}
