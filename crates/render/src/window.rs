//! window: std-only WindowShell. Frame count + fixed-dt accumulator only; window events never feed state_hash.

use super::NullRenderer;

/// Fixed simulation dt: 60Hz. Accumulator clamps spiral-of-death at 5 steps.
pub const FIXED_DT: f32 = 1.0 / 60.0;
pub const MAX_STEPS_PER_PUMP: u32 = 5;

/// Window surface state. Informational only — never hashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceState {
    Headless,
    Open,
    Closed,
}

pub struct WindowShell {
    renderer: NullRenderer,
    surface: SurfaceState,
    width: u32,
    height: u32,
    acc: f32,
    ticks: u64,
}

impl Default for WindowShell {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowShell {
    pub fn new() -> Self {
        Self {
            renderer: NullRenderer::new(),
            surface: SurfaceState::Headless,
            width: 1280,
            height: 720,
            acc: 0.0,
            ticks: 0,
        }
    }

    /// OS event: open window. Does not touch sim.
    pub fn open(&mut self, width: u32, height: u32) {
        self.surface = SurfaceState::Open;
        self.width = width;
        self.height = height;
    }

    /// OS event: close window. Does not touch sim.
    pub fn close(&mut self) {
        self.surface = SurfaceState::Closed;
    }

    /// OS event: resize surface. Drops/recreates surface only. Does not touch sim.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    /// Pump real elapsed seconds through a fixed-dt accumulator.
    /// Calls `step_tick(tick)` once per fixed step, then `begin_frame` once per call.
    /// Returns number of fixed steps taken (0..=MAX_STEPS_PER_PUMP).
    pub fn pump(&mut self, elapsed_s: f32, tick: u64, mut step_tick: impl FnMut(u64)) -> u32 {
        let clamped = elapsed_s.min(FIXED_DT * MAX_STEPS_PER_PUMP as f32);
        self.acc += clamped;
        let mut steps = 0;
        while self.acc >= FIXED_DT && steps < MAX_STEPS_PER_PUMP {
            self.acc -= FIXED_DT;
            step_tick(tick + steps as u64);
            steps += 1;
            self.ticks += 1;
        }
        self.renderer.begin_frame(tick);
        steps
    }

    pub fn surface(&self) -> SurfaceState {
        self.surface
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn draw_count(&self) -> u64 {
        self.renderer.draw_count()
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_close_resize_never_blocks_frames() {
        let mut w = WindowShell::new();
        w.open(1920, 1080);
        assert_eq!(w.surface(), SurfaceState::Open);
        let n = w.pump(1.0 / 60.0, 0, |_| {});
        assert_eq!(n, 1);
        w.resize(800, 600);
        assert_eq!(w.size(), (800, 600));
        let n2 = w.pump(1.0 / 60.0, 1, |_| {});
        assert_eq!(n2, 1);
        w.close();
        assert_eq!(w.surface(), SurfaceState::Closed);
        let n3 = w.pump(1.0 / 60.0, 2, |_| {});
        assert_eq!(n3, 1);
        assert_eq!(w.draw_count(), 3);
    }

    #[test]
    fn accumulator_clamps_spiral() {
        let mut w = WindowShell::new();
        // 10s elapsed clamps to 5 fixed steps, not 600.
        let n = w.pump(10.0, 0, |_| {});
        assert_eq!(n, MAX_STEPS_PER_PUMP);
        assert_eq!(w.ticks(), MAX_STEPS_PER_PUMP as u64);
    }

    #[test]
    fn sub_frame_elapsed_accumulates() {
        let mut w = WindowShell::new();
        let n1 = w.pump(1.0 / 120.0, 0, |_| {});
        assert_eq!(n1, 0);
        let n2 = w.pump(1.0 / 120.0, 0, |_| {});
        assert_eq!(n2, 1);
    }
}
