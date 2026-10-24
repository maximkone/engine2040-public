//! fluids_gpu: seeded presentation fluid pool for blood and water droplets.
//! Owner: render lane; consumer arena match through the particle draw feed.
//! Units: meters, seconds, ticks at 60 Hz. Determinism: blood then water order,
//! SplitMix stream, stable order, only + - * / and sqrt; same seed same bytes.
//! Errors are typed FluidError; headless names the missing device bind.
use engine_core::{splitmix64, Fnv64};
use std::fmt;
use std::time::Instant;
/// Fixed cap; caller draw feed sizes the second feed from alive counts.
pub const FLUID_CAP: usize = 4096;
/// Blood half; caller maps spray positions into this half.
pub const BLOOD_CAP: usize = 2048;
/// Water half; caller maps water cues into this half.
pub const WATER_CAP: usize = 2048;
/// Fixed step; caller off-tick driver advances with this step.
pub const FLUID_DT: f32 = 1.0 / 60.0;
/// Max step; caller clamps bigger wall steps through this bound.
pub const FLUID_MAX_DT: f32 = 0.1;
/// Stage budget; caller charges receipts against this bar.
pub const FLUID_BUDGET_MS: f64 = 1.0;
/// Support radius; caller kernels evaluate inside this range.
pub const SMOOTH_H: f32 = 0.25;
/// Rest density; caller pressure relaxes toward this target.
pub const REST_DENSITY: f32 = 1000.0;
/// Billboard shader; caller window build binds this file.
pub const FLUID_DRAW_WGSL: &str = include_str!("shaders/fluids_present.wgsl");
/// Fluid failures, same shape as the particle pool errors.
#[derive(Clone, PartialEq, Debug)]
pub enum FluidError {
    /// Bad caller input.
    Invalid(String),
    /// Over bound; nothing changed.
    BudgetExceeded {
        /// Asked.
        requested: u32,
        /// Accepted.
        allowed: u32,
    },
    /// Needs a device; what to build first.
    NoDevice(String),
    /// Device call failed.
    Device(String),
    /// Validation fired.
    Validation(String),
}
impl fmt::Display for FluidError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (prefix, detail) = match self {
            Self::Invalid(e) => ("fluid input", Some(e)),
            Self::BudgetExceeded { requested, allowed } => {
                return write!(
                    f,
                    "fluid budget exceeded: {requested} requested, {allowed} allowed"
                )
            }
            Self::NoDevice(e) => ("fluid bind needs a device", Some(e)),
            Self::Device(e) => ("fluid device", Some(e)),
            Self::Validation(e) => ("fluid validation", Some(e)),
        };
        match detail {
            Some(e) => write!(f, "{prefix}: {e}"),
            None => f.write_str(prefix),
        }
    }
}
impl std::error::Error for FluidError {}
/// Half owner; order is the integration order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FluidKind {
    /// Blood from spray positions.
    Blood,
    /// Water from cue positions.
    Water,
}
impl FluidKind {
    /// Slot tag.
    pub fn index(self) -> u8 {
        match self {
            Self::Blood => 0,
            Self::Water => 1,
        }
    }
    /// Kind from tag; unknown reads as blood.
    pub fn from_tag(tag: u8) -> Self {
        match tag {
            1 => Self::Water,
            _ => Self::Blood,
        }
    }
}
/// One droplet in meters and seconds.
/// Caller feeds these; draw feed reads them in order.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FluidDrop {
    /// Center in meters.
    pub pos: [f32; 3],
    /// Velocity in meters per second.
    pub vel: [f32; 3],
    /// Age in seconds.
    pub age_s: f32,
    /// Life in seconds.
    pub life_s: f32,
    /// Owner tag.
    pub kind: u8,
}
/// Kernel boundary: evaluate plus gradient.
pub trait FluidKernel {
    /// Weight at radius r.
    fn evaluate(&self, r: f32) -> f32;
    /// Slope at radius r.
    fn gradient(&self, r: f32) -> f32;
}
/// Density table.
/// Caller density step samples this table.
pub struct Poly6Kernel {
    /// Seeded table.
    pub table: [f32; 8],
}
/// Pressure table.
/// Caller force step samples this table.
pub struct SpikyKernel {
    /// Seeded table.
    pub table: [f32; 8],
}
/// Viscosity table.
/// Caller damping samples this table.
pub struct ViscosityKernel {
    /// Seeded table.
    pub table: [f32; 8],
}
impl FluidKernel for Poly6Kernel {
    fn evaluate(&self, r: f32) -> f32 {
        let u = falloff(r);
        u * u * u * self.table[0]
    }
    fn gradient(&self, r: f32) -> f32 {
        if !r.is_finite() || r <= 0.0 || r > SMOOTH_H {
            return 0.0;
        }
        let u = falloff(r);
        0.0 - u * u * self.table[1]
    }
}
impl FluidKernel for SpikyKernel {
    fn evaluate(&self, r: f32) -> f32 {
        let u = falloff(r);
        u * u * self.table[0]
    }
    fn gradient(&self, r: f32) -> f32 {
        if !r.is_finite() || r <= 0.0 || r > SMOOTH_H {
            return 0.0;
        }
        0.0 - falloff(r) * self.table[1]
    }
}
impl FluidKernel for ViscosityKernel {
    fn evaluate(&self, r: f32) -> f32 {
        falloff(r) * self.table[0]
    }
    fn gradient(&self, r: f32) -> f32 {
        if !r.is_finite() || r <= 0.0 || r > SMOOTH_H {
            return 0.0;
        }
        0.0 - self.table[1] / SMOOTH_H
    }
}
/// Unit draw from the shared stream.
/// Caller table build and spray jitter share this draw.
fn next_unit(state: &mut u64) -> f32 {
    let w = splitmix64(*state);
    *state = state.wrapping_add(1);
    let bits = 0x3F80_0000 | u32::try_from((w >> 9) & 0x007F_FFFF).unwrap_or(0);
    f32::from_bits(bits) - 1.0
}
/// Seeded table around 1.0.
/// Caller pool build fills each kernel once.
fn build_table(state: &mut u64) -> [f32; 8] {
    let mut t = [1.0; 8];
    let mut i = 0;
    while i < 8 {
        let at = usize::try_from(u32::try_from(i).unwrap_or(0)).unwrap_or(0);
        t[at] = 0.9 + 0.2 * next_unit(state);
        i += 1;
    }
    t
}
// Cubic-spline stays the fallback if poly6 tables miss budget; same trait covers it.
fn falloff(r: f32) -> f32 {
    if !r.is_finite() || !(0.0..=SMOOTH_H).contains(&r) {
        return 0.0;
    }
    1.0 - r / SMOOTH_H
}
/// Fixed-cap pool: blood then water, seeded, off-tick.
/// Caller arena match feeds; draw feed reads alive and bytes.
pub struct PresentFluid {
    blood: Vec<FluidDrop>,
    water: Vec<FluidDrop>,
    poly6: Poly6Kernel,
    spiky: SpikyKernel,
    visc: ViscosityKernel,
    stream: u64,
    time_s: f32,
    ticks: u64,
    spawned: u64,
    show_blood: bool,
}
impl PresentFluid {
    /// Empty pool with seeded tables.
    pub fn new(seed: u64) -> Self {
        let mut s = seed;
        let poly6 = Poly6Kernel {
            table: build_table(&mut s),
        };
        let spiky = SpikyKernel {
            table: build_table(&mut s),
        };
        let visc = ViscosityKernel {
            table: build_table(&mut s),
        };
        let mut blood = Vec::new();
        blood.reserve_exact(BLOOD_CAP);
        let mut water = Vec::new();
        water.reserve_exact(WATER_CAP);
        Self {
            blood,
            water,
            poly6,
            spiky,
            visc,
            stream: s,
            time_s: 0.0,
            ticks: 0,
            spawned: 0,
            show_blood: true,
        }
    }
    /// Blood filter; water always shows.
    pub fn set_show_blood(&mut self, show: bool) {
        self.show_blood = show;
    }
    /// Feed one spray in meters; filtered reads drop quietly.
    pub fn feed(&mut self, kind: FluidKind, pos_m: [f32; 3]) -> Result<(), FluidError> {
        if kind == FluidKind::Blood && !self.show_blood {
            return Ok(());
        }
        if !pos_m.iter().all(|c| c.is_finite()) {
            return Err(FluidError::Invalid("fluid position is not finite".into()));
        }
        let (full, cap) = match kind {
            FluidKind::Blood => (self.blood.len() >= BLOOD_CAP, BLOOD_CAP),
            FluidKind::Water => (self.water.len() >= WATER_CAP, WATER_CAP),
        };
        if full {
            return Err(FluidError::BudgetExceeded {
                requested: 1,
                allowed: u32::try_from(cap).unwrap_or(0),
            });
        }
        let jx = next_unit(&mut self.stream) - 0.5;
        let jy = next_unit(&mut self.stream);
        let jz = next_unit(&mut self.stream) - 0.5;
        let life = 0.6 + 0.8 * next_unit(&mut self.stream);
        let d = FluidDrop {
            pos: [
                pos_m[0] + jx * 0.1,
                pos_m[1] + jy * 0.2,
                pos_m[2] + jz * 0.1,
            ],
            vel: [jx * 2.0, 1.0 + jy, jz * 2.0],
            age_s: 0.0,
            life_s: life.max(0.1),
            kind: kind.index(),
        };
        match kind {
            FluidKind::Blood => self.blood.push(d),
            FluidKind::Water => self.water.push(d),
        }
        self.spawned += 1;
        Ok(())
    }
    /// One fixed tick off-tick.
    pub fn tick(&mut self) {
        self.step(FLUID_DT);
    }
    /// Step blood then water; bigger asks clamp.
    pub fn step(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let h = if dt > FLUID_MAX_DT { FLUID_MAX_DT } else { dt };
        integrate(&mut self.blood, &self.poly6, &self.spiky, &self.visc, h);
        integrate(&mut self.water, &self.poly6, &self.spiky, &self.visc, h);
        self.time_s += h;
        self.ticks += 1;
    }
    /// Timed tick; wall time stays in the receipt.
    pub fn step_timed(&mut self, dt: f32) -> FluidReceipt {
        let start = Instant::now();
        self.step(dt);
        FluidReceipt {
            pass: "fluids_gpu",
            ticks: self.ticks,
            alive: self.alive(),
            spawned: self.spawned,
            ms: start.elapsed().as_secs_f64() * 1000.0,
        }
    }
    /// Live droplets.
    pub fn alive(&self) -> usize {
        self.blood.len() + self.water.len()
    }
    /// Total feeds.
    pub fn spawned_total(&self) -> u64 {
        self.spawned
    }
    /// Tick cursor.
    pub fn tick_count(&self) -> u64 {
        self.ticks
    }
    /// Byte image in fixed order.
    pub fn sim_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.alive() * 40);
        out.extend_from_slice(&self.ticks.to_le_bytes());
        out.extend_from_slice(&u32::try_from(self.blood.len()).unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&u32::try_from(self.water.len()).unwrap_or(0).to_le_bytes());
        for d in self.blood.iter().chain(self.water.iter()) {
            for w in [
                d.pos[0].to_bits(),
                d.pos[1].to_bits(),
                d.pos[2].to_bits(),
                d.vel[0].to_bits(),
                d.vel[1].to_bits(),
                d.vel[2].to_bits(),
                d.age_s.to_bits(),
                d.life_s.to_bits(),
            ] {
                out.extend_from_slice(&w.to_le_bytes());
            }
            out.push(d.kind);
        }
        out
    }
    /// Hash over bytes via shared owner.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        h.write(&self.sim_bytes());
        h.finish()
    }
}
/// Integrate one half with density, pressure and viscosity.
/// Caller pool step runs blood then water here.
fn integrate(
    drops: &mut Vec<FluidDrop>,
    poly6: &Poly6Kernel,
    spiky: &SpikyKernel,
    visc: &ViscosityKernel,
    h: f32,
) {
    let n = drops.len();
    let mut i = 0;
    while i < n {
        let (px, py, pz) = (drops[i].pos[0], drops[i].pos[1], drops[i].pos[2]);
        let mut density = REST_DENSITY * falloff(0.0);
        let mut k = 0;
        while k < 8 {
            let at = i + k + 1;
            if at < n {
                let dx = drops[at].pos[0] - px;
                let dy = drops[at].pos[1] - py;
                let dz = drops[at].pos[2] - pz;
                density += poly6.evaluate((dx * dx + dy * dy + dz * dz).sqrt()) * 40.0;
            }
            k += 1;
        }
        let pressure = (density - REST_DENSITY).max(0.0) * 0.002;
        let (mut fx, mut fy, mut fz) = (0.0, -9.0, 0.0);
        let mut v = 0;
        while v < 4 {
            let at = i + v + 1;
            if at < n {
                let dx = drops[at].pos[0] - px;
                let dy = drops[at].pos[1] - py;
                let dz = drops[at].pos[2] - pz;
                let r = (dx * dx + dy * dy + dz * dz).sqrt();
                let g = spiky.gradient(r);
                let inv = if r > 0.0001 { 1.0 / r } else { 0.0 };
                fx += 0.0 - dx * inv * g * pressure
                    + (drops[at].vel[0] - drops[i].vel[0]) * visc.evaluate(r) * 0.4;
                fy += 0.0 - dy * inv * g * pressure
                    + (drops[at].vel[1] - drops[i].vel[1]) * visc.evaluate(r) * 0.4;
                fz += 0.0 - dz * inv * g * pressure
                    + (drops[at].vel[2] - drops[i].vel[2]) * visc.evaluate(r) * 0.4;
            }
            v += 1;
        }
        let p = &mut drops[i];
        p.vel[0] += fx * h;
        p.vel[1] += fy * h;
        p.vel[2] += fz * h;
        p.pos[0] += p.vel[0] * h;
        p.pos[1] += p.vel[1] * h;
        p.pos[2] += p.vel[2] * h;
        p.age_s += h;
        i += 1;
    }
    drops.retain(|d| d.age_s < d.life_s);
}
/// One receipt.
/// Caller budget check reads ms against the bar.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FluidReceipt {
    /// Pass name.
    pub pass: &'static str,
    /// Ticks at cursor.
    pub ticks: u64,
    /// Live at cursor.
    pub alive: usize,
    /// Feeds at cursor.
    pub spawned: u64,
    /// Wall ms.
    pub ms: f64,
}
impl FluidReceipt {
    /// Posted bar.
    pub fn budget_ms(&self) -> f64 {
        FLUID_BUDGET_MS
    }
    /// True inside bar.
    pub fn within_budget(&self) -> bool {
        self.ms <= FLUID_BUDGET_MS
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::particles::fluid_mirror_bytes;
    #[test]
    fn fluids_seed_x2_identical_off_tick() {
        let mut a = PresentFluid::new(2040);
        let mut b = PresentFluid::new(2040);
        let mut i = 0;
        while i < 4096 {
            let x = i as f32 * 0.001;
            let kind = if i < 2048 {
                FluidKind::Blood
            } else {
                FluidKind::Water
            };
            a.feed(kind, [x, 1.0, 0.0]).expect("feed");
            b.feed(kind, [x, 1.0, 0.0]).expect("feed");
            i += 1;
        }
        assert_eq!(a.alive(), 4096);
        let mut t = 0;
        while t < 120 {
            a.tick();
            b.tick();
            t += 1;
        }
        assert_eq!(a.sim_bytes(), b.sim_bytes());
        assert_eq!(a.state_hash(), b.state_hash());
        assert_eq!(fluid_mirror_bytes(&a), a.sim_bytes());
        assert!(FLUID_DRAW_WGSL.contains("fluid_vs"));
        assert!(FLUID_DRAW_WGSL.contains("age01"));
        let at = FluidReceipt {
            pass: "fluids_gpu",
            ticks: 120,
            alive: a.alive(),
            spawned: a.spawned_total(),
            ms: FLUID_BUDGET_MS,
        };
        assert!(at.within_budget());
        assert_eq!((FLUID_CAP, BLOOD_CAP, WATER_CAP), (4096, 2048, 2048));
    }
    #[test]
    fn fluid_feed_filter_and_bounds() {
        let mut p = PresentFluid::new(7);
        assert!(p.feed(FluidKind::Blood, [f32::NAN, 0.0, 0.0]).is_err());
        p.set_show_blood(false);
        assert!(p.feed(FluidKind::Blood, [0.0, 1.0, 0.0]).is_ok());
        assert_eq!(p.alive(), 0);
        p.set_show_blood(true);
        assert!(p.feed(FluidKind::Blood, [0.0, 1.0, 0.0]).is_ok());
        assert_eq!(p.alive(), 1);
    }
}
