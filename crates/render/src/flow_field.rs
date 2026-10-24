//! flow_field: the quarter-res screen-space flow field (SW41): one stable-fluids
//! step on the CPU, the oracle the WGSL pass must match.
//!
//! Why a field and not particles: dust, smoke, blood mist and heat shimmer are
//! soft layers that drift. One velocity field plus one dye field carries all
//! four at a quarter of the frame's resolution, and a fraction of the cost of a
//! particle per mote.
//!
//! Passes, in the donor's order (PavelDoGreat/WebGL-Fluid-Simulation
//! `script.js` `step(dt)`): curl, vorticity, divergence, [`PRESSURE_ITERATIONS`]
//! Jacobi pressure iterations, gradient subtract, advect velocity, advect dye.
//! The donor's config is kept: [`SIM_RESOLUTION`], [`DENSITY_DISSIPATION`],
//! [`VELOCITY_DISSIPATION`], [`CURL`].
//!
//! Owner: render. Units: cells, seconds, f32 field values (grid cells per
//! second for velocity). View only, never hashed. Determinism: plain slices read
//! and written in one fixed order, one seed, no clock, no thread, no map, so the
//! same seed and the same calls give the same bytes.

use engine_core::CounterRng;

/// The sim grid is a quarter of the frame on each axis: 1920x1080 -> 480x270.
pub const QUARTER: u32 = 4;

/// Donor `SIM_RESOLUTION`: the grid's long edge on the reference page. This
/// oracle takes the frame's quarter instead, so the constant names the donor
/// shape the constants below were tuned against.
pub const SIM_RESOLUTION: u32 = 128;

/// Donor `DENSITY_DISSIPATION`: dye lost per second (`1 / (1 + d * dt)`).
pub const DENSITY_DISSIPATION: f32 = 1.0;

/// Donor `VELOCITY_DISSIPATION`: velocity lost per second.
pub const VELOCITY_DISSIPATION: f32 = 0.2;

/// Donor `CURL`: vorticity confinement strength, so eddies stay crisp.
pub const CURL: f32 = 30.0;

/// Donor `PRESSURE_ITERATIONS`: Jacobi sweeps per step.
pub const PRESSURE_ITERATIONS: u32 = 20;

/// Donor `PRESSURE`: the previous pressure field carries over at this fraction.
pub const PRESSURE_CARRY: f32 = 0.8;

/// A rectangle of the sim grid a splat may write, in cells, `x1`/`y1` exclusive.
///
/// The soft layers belong to a part of the screen (the sand floor, a brazier's
/// cone), never to the whole frame: a splat outside its mask writes nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mask {
    /// First writable column.
    pub x0: u32,
    /// First writable row.
    pub y0: u32,
    /// One past the last writable column.
    pub x1: u32,
    /// One past the last writable row.
    pub y1: u32,
}

impl Mask {
    /// The whole grid.
    #[must_use]
    pub fn whole(width: u32, height: u32) -> Self {
        Self {
            x0: 0,
            y0: 0,
            x1: width,
            y1: height,
        }
    }

    /// True when `(x, y)` is inside this mask.
    #[must_use]
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }

    /// The mask cut to a `width` x `height` grid.
    #[must_use]
    fn clipped(self, width: u32, height: u32) -> Self {
        Self {
            x0: self.x0.min(width),
            y0: self.y0.min(height),
            x1: self.x1.min(width),
            y1: self.y1.min(height),
        }
    }
}

/// One quarter-res flow field: velocity, dye and the pressure scratch a step
/// reads and writes.
///
/// Every buffer is allocated at construction, so [`FlowField::step`] allocates
/// nothing and a frame's cost is the grid's own size.
#[derive(Clone, Debug)]
pub struct FlowField {
    width: u32,
    height: u32,
    seed: u64,
    /// Velocity, cells per second, x and y lanes.
    vx: Vec<f32>,
    vy: Vec<f32>,
    /// The soft layers' dye, one value per cell.
    dye: Vec<f32>,
    /// Per-step scratch: curl, divergence, pressure (front and back).
    curl: Vec<f32>,
    divergence: Vec<f32>,
    pressure: Vec<f32>,
    pressure_back: Vec<f32>,
    /// Scratch for the two advections, so a step never allocates.
    advected: Vec<f32>,
    advected_y: Vec<f32>,
}

impl FlowField {
    /// A field over a `frame_width` x `frame_height` frame, at a quarter of it.
    #[must_use]
    pub fn new(frame_width: u32, frame_height: u32) -> Self {
        Self::seeded(0, frame_width, frame_height)
    }

    /// [`FlowField::new`] with the seed the brazier's drift draws from.
    #[must_use]
    pub fn seeded(seed: u64, frame_width: u32, frame_height: u32) -> Self {
        Self::with_grid(
            seed,
            (frame_width / QUARTER).max(1),
            (frame_height / QUARTER).max(1),
        )
    }

    /// A field over an explicit grid (tests and odd frame sizes).
    #[must_use]
    pub fn with_grid(seed: u64, width: u32, height: u32) -> Self {
        let (width, height) = (width.max(1), height.max(1));
        let cells = usize::try_from(u64::from(width) * u64::from(height)).unwrap_or(1);
        Self {
            width,
            height,
            seed,
            vx: vec![0.0; cells],
            vy: vec![0.0; cells],
            dye: vec![0.0; cells],
            curl: vec![0.0; cells],
            divergence: vec![0.0; cells],
            pressure: vec![0.0; cells],
            pressure_back: vec![0.0; cells],
            advected: vec![0.0; cells],
            advected_y: vec![0.0; cells],
        }
    }

    /// Grid width in cells.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Grid height in cells.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Cells in the grid.
    #[must_use]
    pub fn cells(&self) -> usize {
        self.dye.len()
    }

    /// Velocity at `(x, y)`, cells per second; a cell off the grid reads zero.
    #[must_use]
    pub fn velocity(&self, x: u32, y: u32) -> [f32; 2] {
        match self.index(x, y) {
            Some(i) => [self.vx[i], self.vy[i]],
            None => [0.0, 0.0],
        }
    }

    /// Dye at `(x, y)`; a cell off the grid reads zero.
    #[must_use]
    pub fn dye_at(&self, x: u32, y: u32) -> f32 {
        self.index(x, y).map_or(0.0, |i| self.dye[i])
    }

    /// Every dye value, row by row (the pass's readback shape).
    #[must_use]
    pub fn dye(&self) -> &[f32] {
        &self.dye
    }

    /// Total dye: the number a receipt prints.
    #[must_use]
    pub fn dye_total(&self) -> f32 {
        self.dye.iter().sum()
    }

    /// The three fields as little-endian bytes: the byte-equality bar a CPU step
    /// and (later) the WGSL pass are compared on.
    #[must_use]
    pub fn snapshot(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.cells() * 12);
        for field in [&self.vx, &self.vy, &self.dye] {
            for value in field.iter() {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes
    }

    /// Throw `strength` of dye at `at` (cell coordinates, the cell centre is
    /// `+0.5`) with a Gaussian falloff over `radius` cells and push the velocity
    /// along `dir`. Only cells inside `mask` are written: a splat outside its
    /// mask leaves the field bit-identical. `strength == 0` writes nothing, so
    /// a zero-strength splat is byte-equal too.
    pub fn splat(&mut self, at: [f32; 2], radius: f32, strength: f32, dir: [f32; 2], mask: Mask) {
        if strength == 0.0 || !strength.is_finite() || !radius.is_finite() || radius <= 0.0 {
            return;
        }
        if !at[0].is_finite() || !at[1].is_finite() || !dir[0].is_finite() || !dir[1].is_finite() {
            return;
        }
        let mask = mask.clipped(self.width, self.height);
        let inverse = 1.0 / (radius * radius);
        for y in mask.y0..mask.y1 {
            for x in mask.x0..mask.x1 {
                let dx = f32::from(u16::try_from(x).unwrap_or(0)) + 0.5 - at[0];
                let dy = f32::from(u16::try_from(y).unwrap_or(0)) + 0.5 - at[1];
                let distance = (dx * dx + dy * dy) * inverse;
                if distance > 1.0 {
                    continue;
                }
                let falloff = (-distance).exp();
                let weight = strength * falloff;
                if let Some(i) = self.index(x, y) {
                    self.dye[i] += weight;
                    self.vx[i] += dir[0] * weight;
                    self.vy[i] += dir[1] * weight;
                }
            }
        }
    }

    /// A seeded ambient puff at `slot` (the brazier's drift, a footfall's dust):
    /// the position and direction come from this field's seed, so a rerun lands
    /// on the same cells.
    pub fn ambient(&mut self, slot: u64, strength: f32, mask: Mask) {
        let stream = CounterRng::new(self.seed ^ slot);
        let x = cell_of(stream.sample(0), self.width);
        let y = cell_of(stream.sample(1), self.height);
        let direction = match stream.sample(2) % 4 {
            0 => [1.0, 0.0],
            1 => [-1.0, 0.0],
            2 => [0.0, 1.0],
            _ => [0.0, -1.0],
        };
        let at = [x + 0.5, y + 0.5];
        self.splat(at, 2.5, strength, direction, mask);
    }

    /// One stable-fluids step over `dt` seconds, the donor's order: curl,
    /// vorticity, divergence, pressure Jacobi, gradient subtract, advect velocity
    /// then advect dye. Allocates nothing; the same field and `dt` give the same
    /// bytes.
    pub fn step(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.compute_curl();
        self.apply_vorticity(dt);
        self.compute_divergence();
        self.solve_pressure();
        self.subtract_gradient();
        self.advect_velocity(dt);
        self.advect_dye(dt);
    }

    /// The cell index of `(x, y)`, `None` off the grid.
    fn index(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        usize::try_from(u64::from(y) * u64::from(self.width) + u64::from(x)).ok()
    }

    /// `(x, y)` clamped to the grid, the boundary condition every pass reads.
    fn clamped(&self, x: i64, y: i64) -> usize {
        let x = u32::try_from(x.clamp(0, i64::from(self.width) - 1)).unwrap_or(0);
        let y = u32::try_from(y.clamp(0, i64::from(self.height) - 1)).unwrap_or(0);
        self.index(x, y).unwrap_or(0)
    }

    /// Curl at every cell: the y difference of x-velocity less the x difference
    /// of y-velocity, over two cells.
    fn compute_curl(&mut self) {
        let (width, height) = (i64::from(self.width), i64::from(self.height));
        for y in 0..height {
            for x in 0..width {
                let left = self.clamped(x - 1, y);
                let right = self.clamped(x + 1, y);
                let down = self.clamped(x, y - 1);
                let up = self.clamped(x, y + 1);
                let i = self.clamped(x, y);
                self.curl[i] =
                    0.5 * ((self.vy[right] - self.vy[left]) - (self.vx[up] - self.vx[down]));
            }
        }
    }

    /// Vorticity confinement: push the velocity along the curl gradient, so a
    /// small eddy does not smear away. The gradient is normalised, then scaled by
    /// [`CURL`] and the cell's own curl.
    fn apply_vorticity(&mut self, dt: f32) {
        let (width, height) = (i64::from(self.width), i64::from(self.height));
        let scale = CURL * dt;
        for y in 0..height {
            for x in 0..width {
                let i = self.clamped(x, y);
                let force_x = 0.5
                    * (self.curl[self.clamped(x + 1, y)].abs()
                        - self.curl[self.clamped(x - 1, y)].abs());
                let force_y = 0.5
                    * (self.curl[self.clamped(x, y + 1)].abs()
                        - self.curl[self.clamped(x, y - 1)].abs());
                let length = (force_x * force_x + force_y * force_y).sqrt();
                if length <= 0.0 {
                    continue;
                }
                let (force_x, force_y) = (force_x / length, force_y / length);
                let curl = self.curl[i];
                self.vx[i] += scale * force_y * curl;
                self.vy[i] -= scale * force_x * curl;
            }
        }
    }

    /// Divergence at every cell: the velocity field's own outward flow. The
    /// pressure solve turns it into a field that cancels it.
    fn compute_divergence(&mut self) {
        let (width, height) = (i64::from(self.width), i64::from(self.height));
        for y in 0..height {
            for x in 0..width {
                let i = self.clamped(x, y);
                self.divergence[i] = 0.5
                    * ((self.vx[self.clamped(x + 1, y)] - self.vx[self.clamped(x - 1, y)])
                        + (self.vy[self.clamped(x, y + 1)] - self.vy[self.clamped(x, y - 1)]));
            }
        }
    }

    /// Carry the last pressure at [`PRESSURE_CARRY`] and run
    /// [`PRESSURE_ITERATIONS`] Jacobi sweeps towards `divergence = 0`. Each sweep
    /// reads the previous one whole, in the same order, so the result is a
    /// function of the inputs only.
    fn solve_pressure(&mut self) {
        for value in &mut self.pressure {
            *value *= PRESSURE_CARRY;
        }
        let (width, height) = (i64::from(self.width), i64::from(self.height));
        for _ in 0..PRESSURE_ITERATIONS {
            for y in 0..height {
                for x in 0..width {
                    let i = self.clamped(x, y);
                    let around = self.pressure[self.clamped(x - 1, y)]
                        + self.pressure[self.clamped(x + 1, y)]
                        + self.pressure[self.clamped(x, y - 1)]
                        + self.pressure[self.clamped(x, y + 1)];
                    self.pressure_back[i] = (self.divergence[i] + around) * 0.25;
                }
            }
            core::mem::swap(&mut self.pressure, &mut self.pressure_back);
        }
    }

    /// Subtract the pressure gradient from the velocity: the projection step that
    /// makes the field divergence free.
    fn subtract_gradient(&mut self) {
        let (width, height) = (i64::from(self.width), i64::from(self.height));
        for y in 0..height {
            for x in 0..width {
                let i = self.clamped(x, y);
                self.vx[i] -= 0.5
                    * (self.pressure[self.clamped(x + 1, y)]
                        - self.pressure[self.clamped(x - 1, y)]);
                self.vy[i] -= 0.5
                    * (self.pressure[self.clamped(x, y + 1)]
                        - self.pressure[self.clamped(x, y - 1)]);
            }
        }
    }

    /// Semi-Lagrangian advection of the velocity field, with the donor's
    /// dissipation (`1 / (1 + d * dt)`). The backtrace clamps at the boundary.
    fn advect_velocity(&mut self, dt: f32) {
        let decay = 1.0 / (1.0 + VELOCITY_DISSIPATION * dt);
        let (width, height) = (i64::from(self.width), i64::from(self.height));
        for y in 0..height {
            for x in 0..width {
                let i = self.clamped(x, y);
                let back_x = f32::from(u16::try_from(x).unwrap_or(0)) - dt * self.vx[i];
                let back_y = f32::from(u16::try_from(y).unwrap_or(0)) - dt * self.vy[i];
                let (advected_x, advected_y) = self.sample_velocity(back_x, back_y);
                self.advected[i] = advected_x * decay;
                self.advected_y[i] = advected_y * decay;
            }
        }
        self.vx.copy_from_slice(&self.advected);
        self.vy.copy_from_slice(&self.advected_y);
    }

    /// Semi-Lagrangian advection of the dye by the velocity field, with
    /// [`DENSITY_DISSIPATION`]. Runs after the velocity advection, the donor's
    /// order: the dye rides the field the step produced.
    fn advect_dye(&mut self, dt: f32) {
        let decay = 1.0 / (1.0 + DENSITY_DISSIPATION * dt);
        let (width, height) = (i64::from(self.width), i64::from(self.height));
        for y in 0..height {
            for x in 0..width {
                let i = self.clamped(x, y);
                let back_x = f32::from(u16::try_from(x).unwrap_or(0)) - dt * self.vx[i];
                let back_y = f32::from(u16::try_from(y).unwrap_or(0)) - dt * self.vy[i];
                self.advected[i] = self.sample_dye(back_x, back_y) * decay;
            }
        }
        self.dye.copy_from_slice(&self.advected);
    }

    /// Bilinear velocity sample at a fractional cell position, clamped.
    fn sample_velocity(&self, x: f32, y: f32) -> (f32, f32) {
        let (x0, y0, tx, ty) = self.bilinear_terms(x, y);
        let blend = |field: &[f32]| {
            let top =
                field[self.clamped(x0, y0)] * (1.0 - tx) + field[self.clamped(x0 + 1, y0)] * tx;
            let bottom = field[self.clamped(x0, y0 + 1)] * (1.0 - tx)
                + field[self.clamped(x0 + 1, y0 + 1)] * tx;
            top * (1.0 - ty) + bottom * ty
        };
        (blend(&self.vx), blend(&self.vy))
    }

    /// Bilinear dye sample at a fractional cell position, clamped.
    fn sample_dye(&self, x: f32, y: f32) -> f32 {
        let (x0, y0, tx, ty) = self.bilinear_terms(x, y);
        let top =
            self.dye[self.clamped(x0, y0)] * (1.0 - tx) + self.dye[self.clamped(x0 + 1, y0)] * tx;
        let bottom = self.dye[self.clamped(x0, y0 + 1)] * (1.0 - tx)
            + self.dye[self.clamped(x0 + 1, y0 + 1)] * tx;
        top * (1.0 - ty) + bottom * ty
    }

    /// The four cells and the two weights a bilinear sample needs.
    fn bilinear_terms(&self, x: f32, y: f32) -> (i64, i64, f32, f32) {
        let last_x = f32::from(u16::try_from(self.width.saturating_sub(1)).unwrap_or(0));
        let last_y = f32::from(u16::try_from(self.height.saturating_sub(1)).unwrap_or(0));
        let x = x.clamp(0.0, last_x);
        let y = y.clamp(0.0, last_y);
        let x0 = x.floor();
        let y0 = y.floor();
        (floor_cell(x0), floor_cell(y0), x - x0, y - y0)
    }
}

/// A fractional cell coordinate floored to an integer cell, never negative.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the coordinate is clamped to the grid and floored, so the cast is exact"
)]
fn floor_cell(v: f32) -> i64 {
    i64::try_from(v.max(0.0).floor() as u64).unwrap_or(0)
}

/// A seeded word folded onto `0..width` as a cell column or row.
fn cell_of(word: u64, extent: u32) -> f32 {
    let cell = u32::try_from(word % u64::from(extent.max(1))).unwrap_or(0);
    f32::from(u16::try_from(cell.min(u32::from(u16::MAX))).unwrap_or(0))
}

#[cfg(test)]
#[path = "flow_field_tests.rs"]
mod tests;
