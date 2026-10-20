//! ossim: the headless core of "You're the OS", an OS puzzle game.
//!
//! One screen holds four CPU slots, 64 RAM pages plus swap slots, one I/O
//! queue and up to 48 processes. The player assigns CPUs, swaps pages and
//! sends processes to I/O while starvation climbs. Twelve const stages run
//! from a one-minute boot to a six-minute last patch.
//!
//! Units and determinism: 60 Hz integer ticks, pages and slots as `u16`,
//! seeds through `engine_core::CounterRng`, one FNV state hash. There is no
//! float, no wall clock, no global state and no platform math in this crate.
//! Replaying a seed and an action tape produces the same hash.
//!
//! The game idea comes from `plbrault/youre-the-os` (GPL-3.0). This crate is
//! original code: no source, text or asset from that project is copied.

mod model;
mod sim;
mod stages;
#[cfg(test)]
mod tests;

/// VGA text-mode cell buffer over a [`State`] snapshot (OS6).
pub mod screen;

pub use model::{
    grid_cell, Action, Config, Event, EventKind, IoEntry, LossReason, Outcome, Pending, ProcState,
    Process, Refusal, ScoreParts, Stage, CPU_SLOTS, GRID_COLS, GRID_ROWS, HAPPY_RUNNING_TICKS,
    IO_TICKS_MAX, IO_TICKS_MIN, KILLS_TO_REBOOT, MAX_PROCESSES, PAGES_PER_ROW, RAM_PAGES,
    SCORE_GRACEFUL, SCORE_IO_READY, SCORE_KILL, SCORE_PAGE_FAULT, SCORE_WIN, SIX_MINUTES,
    STARVATION_MAX, STARVATION_TICKS, STARVATION_TICKS_PRIORITY, SWAP_SLOTS, SWAP_TICKS_MAX,
    SWAP_TICKS_MIN, TICKS_PER_MINUTE, TICK_HZ,
};
pub use screen::{Cell, Screen, SCREEN_CELLS, SCREEN_COLS, SCREEN_ROWS};
pub use sim::{Sim, State};
pub use stages::STAGES;
