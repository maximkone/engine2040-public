//! ossim screen: the VGA text-mode cell buffer the headless core lacked.
//!
//! Consumer: a front end, or the `screen` example
//! (`cargo run -p engine-ossim --example screen`). Data owner: [`Screen`].
//! Units: 80 x 25 cells, one ASCII glyph plus one VGA color index per cell.
//! Determinism: [`Screen::render`] is a pure function of [`State`]; no float,
//! no clock, no allocation beyond the fixed buffer and [`Screen::to_text`].
//! The layout is fixed so two renders of the same state are byte-equal.

use crate::model::{grid_cell, ProcState, GRID_ROWS, MAX_PROCESSES, STARVATION_MAX};
use crate::sim::State;

/// Text rows on the simulated screen.
pub const SCREEN_ROWS: usize = 25;
/// Text columns on the simulated screen.
pub const SCREEN_COLS: usize = 80;
/// Cells in one screen.
pub const SCREEN_CELLS: usize = SCREEN_COLS * SCREEN_ROWS;

/// Dim gray foreground (VGA color 8).
pub const FG_DIM: u8 = 8;
/// Green ok foreground (VGA color 10).
pub const FG_OK: u8 = 10;
/// Cyan info foreground (VGA color 11).
pub const FG_INFO: u8 = 11;
/// Amber warning foreground (VGA color 12).
pub const FG_WARN: u8 = 12;
/// Bright white foreground (VGA color 15).
pub const FG_BRIGHT: u8 = 15;

/// One text cell: an ASCII glyph plus a VGA foreground color index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// ASCII glyph.
    pub glyph: u8,
    /// Foreground palette index `0..=15`.
    pub fg: u8,
}

impl Cell {
    /// A blank dim cell (space, gray).
    pub const BLANK: Cell = Cell {
        glyph: b' ',
        fg: FG_DIM,
    };
}

/// One rendered screen: [`SCREEN_CELLS`] cells in row-major order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screen {
    cells: [Cell; SCREEN_CELLS],
}

impl Screen {
    /// Render `state` into a fresh screen. The only input is the state, so the
    /// same state always gives the same cells.
    #[must_use]
    pub fn render(state: &State) -> Self {
        let mut screen = Self {
            cells: [Cell::BLANK; SCREEN_CELLS],
        };
        screen.header(state);
        screen.grid(state);
        screen.io_queue(state);
        screen.starvation(state);
        screen.score_parts(state);
        screen
    }

    /// All cells, row-major.
    #[must_use]
    pub fn cells(&self) -> &[Cell; SCREEN_CELLS] {
        &self.cells
    }

    /// One cell, or `None` off screen.
    #[must_use]
    pub fn cell(&self, col: usize, row: usize) -> Option<Cell> {
        if col >= SCREEN_COLS || row >= SCREEN_ROWS {
            return None;
        }
        self.cells.get(row * SCREEN_COLS + col).copied()
    }

    /// The screen as 25 lines of 80 glyphs, `\n`-joined (no trailing newline).
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::with_capacity(SCREEN_CELLS + SCREEN_ROWS);
        for row in 0..SCREEN_ROWS {
            for col in 0..SCREEN_COLS {
                let glyph = self
                    .cell(col, row)
                    .map_or(' ', |cell| char::from(cell.glyph));
                out.push(glyph);
            }
            if row + 1 < SCREEN_ROWS {
                out.push('\n');
            }
        }
        out
    }

    /// Title row, CPU slots, RAM/swap and the live count.
    fn header(&mut self, state: &State) {
        self.put_str(0, 0, "YOU'RE THE OS", FG_BRIGHT);
        self.put_str(15, 0, "T", FG_DIM);
        self.put_u64(17, 0, state.tick, 6, FG_BRIGHT);
        self.put_str(25, 0, "SCORE", FG_DIM);
        self.put_i32(31, 0, state.score, 6, FG_WARN);
        self.put_str(40, 0, "LIVE", FG_DIM);
        self.put_usize(45, 0, state.live(), 2, FG_OK);
        self.put_str(48, 0, "/", FG_DIM);
        self.put_usize(49, 0, MAX_PROCESSES, 2, FG_DIM);
        self.put_str(54, 0, "STAGE", FG_DIM);
        self.put_usize(60, 0, usize::from(state.stage), 2, FG_INFO);
        if let Some(stage) = crate::stages::STAGES.get(usize::from(state.stage)) {
            self.put_str(0, 1, stage.name, FG_INFO);
        }

        self.put_str(0, 2, "CPU", FG_DIM);
        for (slot, cpu) in state.cpus.iter().enumerate() {
            let col = 4 + slot * 9;
            self.put_str(col, 2, "p", FG_DIM);
            match cpu {
                Some(id) => self.put_u32(col + 1, 2, *id, 5, FG_OK),
                None => self.put_str(col + 1, 2, "--", FG_DIM),
            }
        }

        self.put_str(0, 3, "RAM", FG_DIM);
        self.put_usize(4, 3, usize::from(state.ram_pages_used), 2, FG_INFO);
        self.put_str(6, 3, "/", FG_DIM);
        self.put_usize(7, 3, usize::from(state.ram_pages_cap), 2, FG_INFO);
        self.put_str(12, 3, "SWAP", FG_DIM);
        self.put_usize(17, 3, usize::from(state.swap_slots_used), 2, FG_INFO);
        self.put_str(19, 3, "/", FG_DIM);
        self.put_usize(20, 3, usize::from(state.swap_slots_cap), 2, FG_INFO);
        self.put_str(25, 3, "IO", FG_DIM);
        self.put_usize(28, 3, usize::from(state.io_len), 2, FG_INFO);
        self.put_str(33, 3, "KILLS", FG_DIM);
        self.put_usize(39, 3, usize::from(state.kills), 2, FG_WARN);
    }

    /// The 7 x 6 process grid: one 11-column cell per slot.
    fn grid(&mut self, state: &State) {
        self.put_str(0, 4, "PROCESSES", FG_DIM);
        for (slot, process) in state.processes.iter().enumerate() {
            let (gx, gy) = grid_cell(slot);
            let col = 1 + gx * 11;
            let row = 5 + gy % GRID_ROWS;
            let Some(process) = process else {
                continue;
            };
            let fg = match process.state {
                ProcState::Running => FG_OK,
                ProcState::Idle => FG_INFO,
                ProcState::BlockedIo => FG_INFO,
                ProcState::BlockedPage => FG_WARN,
                ProcState::Done => FG_DIM,
            };
            self.put_u32(col, row, process.id, 4, fg);
            self.put_glyph(col + 5, row, mark(process.state), fg);
            self.put_usize(col + 6, row, usize::from(process.starvation), 1, FG_WARN);
            self.put_glyph(
                col + 8,
                row,
                if process.resident { b'R' } else { b'S' },
                FG_DIM,
            );
            self.put_usize(col + 9, row, usize::from(process.pages), 1, FG_DIM);
        }
    }

    /// The single I/O queue, up to eight entries.
    fn io_queue(&mut self, state: &State) {
        self.put_str(0, 12, "IO QUEUE", FG_DIM);
        let shown = usize::from(state.io_len).min(8);
        for index in 0..shown {
            let Some(entry) = state.io_queue.get(index).copied().flatten() else {
                continue;
            };
            let col = 10 + index * 9;
            self.put_str(col, 12, "p", FG_DIM);
            self.put_u32(col + 1, 12, entry.process, 4, FG_INFO);
            self.put_str(col + 5, 12, "@", FG_DIM);
            self.put_u64(col + 6, 12, entry.ready_tick % 100_000, 5, FG_DIM);
        }
    }

    /// Processes at starvation above zero: id, bar, residency and pages.
    fn starvation(&mut self, state: &State) {
        self.put_str(0, 14, "STARVATION", FG_DIM);
        let mut line = 0usize;
        for process in state.processes.iter().flatten() {
            if process.starvation == 0 || line >= 6 {
                continue;
            }
            let row = 15 + line;
            self.put_str(0, row, "p", FG_DIM);
            self.put_u32(1, row, process.id, 4, FG_WARN);
            self.put_glyph(6, row, b'[', FG_DIM);
            let max = usize::from(STARVATION_MAX);
            let level = usize::from(process.starvation).min(max);
            for step in 0..max {
                let glyph = if step < level { b'#' } else { b'-' };
                self.put_glyph(7 + step, row, glyph, FG_WARN);
            }
            self.put_glyph(7 + max, row, b']', FG_DIM);
            self.put_str(9 + max, row, "pages", FG_DIM);
            self.put_usize(15 + max, row, usize::from(process.pages), 1, FG_DIM);
            line += 1;
        }
    }

    /// The score parts behind the integer score.
    fn score_parts(&mut self, state: &State) {
        self.put_str(0, 22, "PARTS", FG_DIM);
        self.put_str(6, 22, "done", FG_DIM);
        self.put_u32(11, 22, state.parts.graceful, 4, FG_OK);
        self.put_str(17, 22, "io", FG_DIM);
        self.put_u32(20, 22, state.parts.io_ready, 4, FG_INFO);
        self.put_str(26, 22, "fault", FG_DIM);
        self.put_u32(32, 22, state.parts.faults, 4, FG_WARN);
        self.put_str(38, 22, "kill", FG_DIM);
        self.put_u32(43, 22, state.parts.kills, 4, FG_WARN);
        self.put_str(49, 22, "won", FG_DIM);
        self.put_str(53, 22, if state.parts.won { "yes" } else { "no" }, FG_OK);
    }

    fn set(&mut self, col: usize, row: usize, glyph: u8, fg: u8) {
        if col >= SCREEN_COLS || row >= SCREEN_ROWS {
            return;
        }
        self.cells[row * SCREEN_COLS + col] = Cell { glyph, fg };
    }

    fn put_glyph(&mut self, col: usize, row: usize, glyph: u8, fg: u8) {
        self.set(col, row, glyph, fg);
    }

    fn put_str(&mut self, col: usize, row: usize, text: &str, fg: u8) {
        for (offset, byte) in text.bytes().enumerate() {
            self.set(col + offset, row, byte, fg);
        }
    }

    /// Right-aligned unsigned value in `width` columns (space padded).
    fn put_u64(&mut self, col: usize, row: usize, value: u64, width: usize, fg: u8) {
        let mut buf = [b' '; 20];
        let mut n = value;
        let mut at = buf.len();
        loop {
            at -= 1;
            buf[at] = b'0' + u8::try_from(n % 10).unwrap_or(0);
            n /= 10;
            if n == 0 || at == 0 {
                break;
            }
        }
        let digits = &buf[at..];
        let pad = width.saturating_sub(digits.len());
        for step in 0..pad {
            self.set(col + step, row, b' ', fg);
        }
        for (step, byte) in digits.iter().enumerate() {
            self.set(col + pad + step, row, *byte, fg);
        }
    }

    fn put_usize(&mut self, col: usize, row: usize, value: usize, width: usize, fg: u8) {
        self.put_u64(col, row, u64::try_from(value).unwrap_or(0), width, fg);
    }

    fn put_u32(&mut self, col: usize, row: usize, value: u32, width: usize, fg: u8) {
        self.put_u64(col, row, u64::from(value), width, fg);
    }

    /// Right-aligned signed value in `width` columns.
    fn put_i32(&mut self, col: usize, row: usize, value: i32, width: usize, fg: u8) {
        let magnitude = value.unsigned_abs();
        let sign = if value < 0 { b'-' } else { b' ' };
        self.put_u64(
            col + 1,
            row,
            u64::from(magnitude),
            width.saturating_sub(1),
            fg,
        );
        self.set(col, row, sign, fg);
    }
}

/// The one-glyph mark for a process state.
fn mark(state: ProcState) -> u8 {
    match state {
        ProcState::Running => b'>',
        ProcState::Idle => b'.',
        ProcState::BlockedIo => b'i',
        ProcState::BlockedPage => b'p',
        ProcState::Done => b'x',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, Sim, STAGES};

    fn boot() -> Sim {
        Sim::new(7, STAGES[0])
    }

    #[test]
    fn screen_is_80x25_and_repeats() {
        let sim = boot();
        let state = sim.state();
        let first = Screen::render(&state);
        let second = Screen::render(&state);
        assert_eq!(first, second, "the same state renders byte-equal");
        assert_eq!(first.cells().len(), SCREEN_CELLS);
        assert_eq!(first.to_text().lines().count(), SCREEN_ROWS);
        for line in first.to_text().lines() {
            assert_eq!(line.chars().count(), SCREEN_COLS, "every row is 80 columns");
        }
    }

    #[test]
    fn screen_header_names_the_stage_tick_and_score() {
        let sim = boot();
        let text = Screen::render(&sim.state()).to_text();
        for needle in ["YOU'RE THE OS", "FIRST BOOT", "SCORE", "LIVE", "RAM", "CPU"] {
            assert!(text.contains(needle), "header holds {needle}: {text}");
        }
    }

    #[test]
    fn screen_marks_a_running_process_and_its_cpu() {
        let mut sim = boot();
        let id = sim
            .state()
            .processes
            .iter()
            .flatten()
            .next()
            .expect("the first boot stage starts one process")
            .id;
        sim.tick(Some(Action::Assign {
            cpu: 0,
            process: id,
        }))
        .expect("assigning a resident idle process is legal");
        let state = sim.state();
        assert_eq!(state.cpus[0], Some(id), "cpu 0 holds the process");
        let screen = Screen::render(&state);
        let marks = screen
            .cells()
            .iter()
            .filter(|cell| cell.glyph == b'>')
            .count();
        assert!(marks >= 1, "the running process shows its mark");
        let text = screen.to_text();
        assert!(text.contains("p"), "cpu slot line names p<id>");
    }

    #[test]
    fn screen_off_grid_access_is_none() {
        let sim = boot();
        let screen = Screen::render(&sim.state());
        assert!(screen.cell(SCREEN_COLS, 0).is_none());
        assert!(screen.cell(0, SCREEN_ROWS).is_none());
        assert!(screen.cell(0, 0).is_some());
    }
}
