//! ossim model: the integer types one screen of "You're the OS" needs.
//! Units: ticks (u64), pages and slots (u16), score (i32).
//! Determinism: every field is an integer or a bool; no floats, no clock.

/// Fixed simulation rate. One tick is one 60 Hz step.
pub const TICK_HZ: u64 = 60;
/// CPU slots on the single screen.
pub const CPU_SLOTS: usize = 4;
/// Process slots; ids are never reused, slots are.
pub const MAX_PROCESSES: usize = 48;
/// Grid columns for the process layout.
pub const GRID_COLS: usize = 7;
/// Grid rows for the process layout; 7 x 6 = 42 cells for 48 slots.
pub const GRID_ROWS: usize = 6;
/// Installed RAM in pages, the most a stage can map.
pub const RAM_PAGES: u16 = 64;
/// Pages per installed RAM row in the stage table.
pub const PAGES_PER_ROW: u16 = 8;
/// Swap slots, one page each, the most a stage can own.
pub const SWAP_SLOTS: u16 = 32;
/// Starvation is capped here; one more step kills the process.
pub const STARVATION_MAX: u8 = 6;
/// Idle ticks per starvation level.
pub const STARVATION_TICKS: u32 = 600;
/// Idle ticks per starvation level for a priority process.
pub const STARVATION_TICKS_PRIORITY: u32 = 360;
/// Running ticks that reset starvation to zero.
pub const HAPPY_RUNNING_TICKS: u32 = 300;
/// Fastest swap, in ticks.
pub const SWAP_TICKS_MIN: u16 = 6;
/// Slowest swap, in ticks.
pub const SWAP_TICKS_MAX: u16 = 30;
/// Fastest I/O completion, in ticks.
pub const IO_TICKS_MIN: u32 = 60;
/// Slowest I/O completion, in ticks.
pub const IO_TICKS_MAX: u32 = 300;
/// Kills that reboot the machine.
pub const KILLS_TO_REBOOT: u8 = 10;
/// One minute of ticks.
pub const TICKS_PER_MINUTE: u32 = 3_600;
/// Six minutes of ticks, the longest stage.
pub const SIX_MINUTES: u32 = 21_600;

/// Score for a process that finishes its work.
pub const SCORE_GRACEFUL: i32 = 100;
/// Score for one I/O request that becomes ready.
pub const SCORE_IO_READY: i32 = 25;
/// Score lost when a process is killed by starvation.
pub const SCORE_KILL: i32 = -50;
/// Score lost when a running process takes a page fault.
pub const SCORE_PAGE_FAULT: i32 = -10;
/// Score for surviving a stage.
pub const SCORE_WIN: i32 = 500;

/// Process lifecycle. `Done` keeps the slot's record until it is reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcState {
    /// Resident, has no CPU, waits for the player.
    Idle,
    /// Holds a CPU slot.
    Running,
    /// Waits for the single I/O queue.
    BlockedIo,
    /// Pages are in swap, or the process is moving between RAM and swap.
    BlockedPage,
    /// Finished or killed; no longer part of the live count.
    Done,
}

impl ProcState {
    /// Stable one-word label for a UI or a log line.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            ProcState::Idle => "idle",
            ProcState::Running => "running",
            ProcState::BlockedIo => "io",
            ProcState::BlockedPage => "page",
            ProcState::Done => "done",
        }
    }

    /// One-byte tag used by the state hash.
    #[must_use]
    pub(crate) const fn tag(self) -> u8 {
        match self {
            ProcState::Idle => 0,
            ProcState::Running => 1,
            ProcState::BlockedIo => 2,
            ProcState::BlockedPage => 3,
            ProcState::Done => 4,
        }
    }
}

/// The one long action a process may be in the middle of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    /// Nothing in flight.
    None,
    /// Pages are moving from RAM to swap.
    SwapOut,
    /// Pages are moving from swap to RAM.
    SwapIn,
}

impl Pending {
    /// One-byte tag used by the state hash.
    #[must_use]
    pub(crate) const fn tag(self) -> u8 {
        match self {
            Pending::None => 0,
            Pending::SwapOut => 1,
            Pending::SwapIn => 2,
        }
    }
}

/// One process row. Copyable so a tick can hold it while mutating the sim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Process {
    /// Stable process id, unique for the whole stage.
    pub id: u32,
    /// Slot the process occupies, `0..MAX_PROCESSES`.
    pub slot: u8,
    /// Current lifecycle state.
    pub state: ProcState,
    /// Starvation level `0..=STARVATION_MAX`; the next step kills.
    pub starvation: u8,
    /// Pages the process needs, `1..=6`.
    pub pages: u8,
    /// Pages are in RAM when true, in swap when false.
    pub resident: bool,
    /// Long action in flight.
    pub pending: Pending,
    /// Tick at which `pending` completes.
    pub swap_done: u64,
    /// Tick at which a blocked I/O request becomes ready.
    pub io_ready: u64,
    /// Priority processes starve faster between CPU turns.
    pub priority: bool,
    /// Tick the process first appeared.
    pub spawn_tick: u64,
    /// Consecutive idle ticks toward the next starvation level.
    pub wait_ticks: u32,
    /// Consecutive running ticks toward a happiness reset.
    pub run_streak: u32,
    /// CPU ticks worked so far.
    pub worked: u32,
    /// CPU ticks the process needs before it exits gracefully.
    pub work: u32,
}

impl Process {
    /// Grid cell for this slot; slots above 42 share cells, the UI badges them.
    #[must_use]
    pub fn cell(&self) -> (usize, usize) {
        grid_cell(usize::from(self.slot))
    }
}

/// Grid cell `(column, row)` for a process slot. The grid wraps; 48 slots
/// over 42 cells means the last six draw on the first six cells.
#[must_use]
pub const fn grid_cell(slot: usize) -> (usize, usize) {
    ((slot % GRID_COLS), ((slot / GRID_COLS) % GRID_ROWS))
}

/// One entry of the single I/O queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoEntry {
    /// Process waiting on this request.
    pub process: u32,
    /// Tick the request becomes ready.
    pub ready_tick: u64,
}

/// The five player actions. An action applies before the tick advances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Put a resident idle process on a CPU slot.
    Assign {
        /// CPU slot, `0..CPU_SLOTS`.
        cpu: u8,
        /// Process id.
        process: u32,
    },
    /// Take a process off a CPU slot.
    Unassign {
        /// CPU slot, `0..CPU_SLOTS`.
        cpu: u8,
    },
    /// Move a process's pages from swap back into RAM.
    SwapIn {
        /// Process id.
        process: u32,
    },
    /// Move a process's pages from RAM into swap.
    SwapOut {
        /// Process id.
        process: u32,
    },
    /// Send a running process to the I/O queue.
    DoIo {
        /// Process id.
        process: u32,
    },
}

/// Why the sim refused an action. A refusal never panics and never blocks
/// the tick: time still advances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The stage already ended.
    Finished,
    /// CPU slot is not in `0..CPU_SLOTS`.
    BadCpu,
    /// CPU slot already holds a process.
    CpuBusy,
    /// CPU slot holds nothing.
    CpuEmpty,
    /// No live process has that id.
    NoSuchProcess,
    /// The process is not in a state this action accepts.
    WrongState,
    /// A swap is already in flight for this process.
    Busy,
    /// RAM has no room for these pages.
    NoRam,
    /// Swap has no room for these pages.
    NoSwap,
    /// The single I/O queue is full.
    QueueFull,
}

/// Why a stage was lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LossReason {
    /// Kills reached the stage limit; the machine reboots.
    Reboot,
    /// A min-process checkpoint found too few live processes.
    TooFewProcesses,
}

/// Stage result. `None` means the stage is still running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The win tick was reached without a defeat.
    Won,
    /// A defeat rule fired first.
    Lost(LossReason),
}

/// Event kinds a UI draws or narrates. Every event names one process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// A process entered a slot.
    Spawn,
    /// Starvation killed a process.
    Kill,
    /// A process finished its work.
    Graceful,
    /// A process entered the I/O queue.
    IoRequest,
    /// An I/O request became ready.
    IoReady,
    /// A running process lost its pages.
    PageFault,
    /// A swap-in or swap-out completed.
    SwapDone,
}

/// One event stamped with the tick that produced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    /// Tick that produced the event.
    pub tick: u64,
    /// What happened.
    pub kind: EventKind,
    /// Process the event is about.
    pub process: u32,
}

/// Counts behind the integer score. The score is always `total()`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScoreParts {
    /// Processes that finished their work.
    pub graceful: u32,
    /// I/O requests that became ready.
    pub io_ready: u32,
    /// Processes killed by starvation.
    pub kills: u32,
    /// Page faults taken by running processes.
    pub faults: u32,
    /// The stage was won.
    pub won: bool,
}

impl ScoreParts {
    /// Integer score from the fixed weights in this module.
    #[must_use]
    pub const fn total(&self) -> i32 {
        self.graceful as i32 * SCORE_GRACEFUL
            + self.io_ready as i32 * SCORE_IO_READY
            + self.kills as i32 * SCORE_KILL
            + self.faults as i32 * SCORE_PAGE_FAULT
            + if self.won { SCORE_WIN } else { 0 }
    }
}

/// One stage row: start load, spawn timeline, min curve, RAM rows, swap and
/// the win tick. All values are const so the table stays data, not code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stage {
    /// Stage index, `0..12`.
    pub id: u8,
    /// Short name for the stage select screen.
    pub name: &'static str,
    /// Processes present at tick 0.
    pub start_processes: u8,
    /// Tick of the first spawn on the timeline.
    pub spawn_first: u32,
    /// Ticks between spawns; zero disables the timeline.
    pub spawn_every: u32,
    /// Number of spawns on the timeline.
    pub spawn_count: u8,
    /// Min-process checkpoints as `(tick, min_live)`.
    pub min_curve: &'static [(u32, u8)],
    /// Installed RAM rows, eight pages each.
    pub ram_rows: u8,
    /// Swap slots owned by the stage.
    pub swap_slots: u16,
    /// Swap duration range in ticks, `(min, max)`.
    pub swap_ticks: (u16, u16),
    /// Tick that wins the stage.
    pub win_tick: u32,
    /// Kills that reboot the machine.
    pub max_kills: u8,
    /// Every Nth process is a priority process; zero means none.
    pub priority_every: u8,
}

impl Stage {
    /// Installed RAM in pages, capped at [`RAM_PAGES`].
    #[must_use]
    pub const fn ram_pages(&self) -> u16 {
        let pages = self.ram_rows as u16 * PAGES_PER_ROW;
        if pages > RAM_PAGES {
            RAM_PAGES
        } else {
            pages
        }
    }

    /// Tick of spawn `n`, or `None` when `n` is off the timeline.
    #[must_use]
    pub const fn spawn_tick(&self, n: u8) -> Option<u32> {
        if n >= self.spawn_count || self.spawn_every == 0 {
            return None;
        }
        Some(self.spawn_first + n as u32 * self.spawn_every)
    }
}

/// Rules knobs. `Sim::new` uses [`Config::default`]; tests shorten timings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Idle ticks per starvation level.
    pub starvation_ticks: u32,
    /// Idle ticks per starvation level for a priority process.
    pub starvation_ticks_priority: u32,
    /// Running ticks that reset starvation.
    pub happy_running_ticks: u32,
    /// I/O ready range in ticks, inclusive.
    pub io_ticks: (u32, u32),
    /// Ticks between ambient I/O and page-fault rolls; zero disables them.
    pub ambient_period: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            starvation_ticks: STARVATION_TICKS,
            starvation_ticks_priority: STARVATION_TICKS_PRIORITY,
            happy_running_ticks: HAPPY_RUNNING_TICKS,
            io_ticks: (IO_TICKS_MIN, IO_TICKS_MAX),
            ambient_period: 600,
        }
    }
}
