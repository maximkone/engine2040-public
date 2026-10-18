//! particles_gpu: fixed seeded pool for dust, torch flame and sparks (RP2).
//!
//! Owner: render lane; consumers WIN1/ED1 frames and Q1 captures through the
//! RA2 graph (transient buffers owned there, never here). Units: meters for
//! positions, sizes and speeds; seconds for age and life; ticks at 60 Hz.
//! Determinism: fixed emitter order, SplitMix64 stream, stable compaction,
//! math uses only `+ - * /` and one `sqrt`; same seed gives identical bytes.
//! Errors are typed [`ParticleError`]; the headless twin names the missing
//! window device while the window build binds the pool for real with 0
//! validation errors. Posted budget: [`PARTICLE_BUDGET_MS`] ms.

use std::fmt;
use std::time::Instant;

#[cfg(feature = "window")]
use crate::decals::{compute_storage_entry, compute_uniform_entry};

#[path = "fluids_gpu.rs"]
/// Seeded presentation fluid pool beside the three emitters; see [`fluids_gpu`].
pub mod fluids_gpu;
#[path = "vfx_hanabi.rs"]
/// Hanabi batch layer over the seeded pool; see [`vfx_hanabi`].
pub mod vfx_hanabi;

#[cfg(feature = "window")]
#[path = "particle_draw_gpu.rs"]
mod particle_draw_gpu;
#[cfg(feature = "window")]
use particle_draw_gpu::{
    particle_draw_atlas, particle_draw_buffer, particle_draw_layout, particle_draw_pipeline,
    particle_pack_instance, particle_pack_uniform, PARTICLE_DRAW_MAX_INSTANCES,
    PARTICLE_DRAW_SCENE_DEPTH_BYTES,
};
#[cfg(feature = "window")]
pub use particle_draw_gpu::{PARTICLE_DRAW_INSTANCE_BYTES, PARTICLE_DRAW_UNIFORM_BYTES};

// ---------------------------------------------------------------------------
// Bounds: every external size is validated before allocation.
// ---------------------------------------------------------------------------

/// Fixed simulation step in seconds (60 Hz presentation ticks).
/// Single owner of the value is `engine_core::DT`.
pub const TICK_DT: f32 = engine_core::DT;
/// Largest integrated step in seconds; bigger asks clamp, never explode.
pub const MAX_DT_CLAMP: f32 = 0.1;
/// Default pool cap: the worst-case live set stays bounded per frame.
pub const POOL_CAP: usize = 4096;
/// Hard cap for custom pools; bigger asks are refused loudly.
pub const POOL_CAP_HARD: usize = 8192;
/// Fixed emitter count: dust, torch flame, sparks, always in that order.
pub const EMITTER_COUNT: usize = 3;
/// Posted GPU budget at 1080p on the reference GPU (PF2 enforces).
pub const PARTICLE_BUDGET_MS: f64 = 1.0;
/// Largest accepted viewport edge in pixels (mirrors the RA1 frame bound).
pub const MAX_VIEWPORT_PX: u32 = 2048;
/// Largest accepted spawn rate per emitter; above this the setter refuses.
pub const MAX_RATE_PER_S: f32 = 2000.0;
/// Longest accepted particle life in seconds.
pub const MAX_LIFE_S: f32 = 5.0;
/// Most spawns integrated per emitter per tick; excess accrues, never bursts.
pub const MAX_SPAWN_PER_TICK: u32 = 256;

// ---------------------------------------------------------------------------
// Errors: typed; bad input fails loudly, the headless twin names the
// window device bind it cannot run without a device.
// ---------------------------------------------------------------------------

/// Particle pool failures. `BudgetExceeded` refuses the build or the write;
/// `NoDevice` covers headless-twin calls with no wgpu device in hand;
/// `Device`/`Validation` cover device-side failures on the window build,
/// which never takes the twin path.
#[derive(Clone, PartialEq, Debug)]
pub enum ParticleError {
    /// Caller input is unusable; reason carried.
    Invalid(String),
    /// The request exceeded a bound; nothing was built or changed.
    BudgetExceeded {
        /// Requested amount.
        requested: u32,
        /// Amount the pool would have accepted.
        allowed: u32,
    },
    /// No device behind this call; what to build first, carried.
    NoDevice(String),
    /// A device, queue or map call failed; message carried.
    Device(String),
    /// A wgpu validation error fired inside the submit scope; kept loud.
    Validation(String),
}

impl fmt::Display for ParticleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "particle input: {e}"),
            Self::BudgetExceeded { requested, allowed } => write!(
                f,
                "particle budget exceeded: {requested} requested, {allowed} allowed"
            ),
            Self::NoDevice(e) => write!(f, "particle bind needs a device: {e}"),
            Self::Device(e) => write!(f, "particle device: {e}"),
            Self::Validation(e) => write!(f, "particle validation: {e}"),
        }
    }
}

impl std::error::Error for ParticleError {}

// ---------------------------------------------------------------------------
// Emitters: three fixed kinds in one fixed order; plain data, seeded at run.
// ---------------------------------------------------------------------------

/// Which fixed emitter owns a particle. Order is the integration order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EmitterKind {
    /// Slow settling motes kicked up by footwork.
    Dust,
    /// Buoyant torch flame puffs rising from braziers.
    Flame,
    /// Fast hot strike sparks with gravity drop.
    Sparks,
}

impl EmitterKind {
    /// Stable slot index; also the `kind` tag stored per particle.
    pub fn index(self) -> usize {
        match self {
            Self::Dust => 0,
            Self::Flame => 1,
            Self::Sparks => 2,
        }
    }

    /// Recover the kind from a stored tag; unknown tags read as dust.
    pub fn from_tag(tag: u8) -> Self {
        match tag {
            1 => Self::Flame,
            2 => Self::Sparks,
            _ => Self::Dust,
        }
    }

    /// Flipbook grid (cols, rows) per kind; frame count is their product.
    pub fn grid(self) -> (u8, u8) {
        match self {
            Self::Dust => (4, 4),
            Self::Flame => (4, 2),
            Self::Sparks => (8, 2),
        }
    }

    /// Only sparks loop their flipbook; dust and flame clamp on the end.
    pub fn looping(self) -> bool {
        matches!(self, Self::Sparks)
    }

    /// Size multiplier over life: dust grows, flame shrinks, sparks hold.
    fn size_mul(self, age01: f32) -> f32 {
        match self {
            Self::Dust => 0.4 + 1.2 * age01,
            Self::Flame => 1.2 - 0.6 * age01,
            Self::Sparks => 1.0,
        }
    }

    /// Alpha multiplier over life: dust fades late, flame and sparks linear.
    fn alpha_mul(self, age01: f32) -> f32 {
        match self {
            Self::Dust => 1.0 - age01 * age01,
            Self::Flame | Self::Sparks => 1.0 - age01,
        }
    }
}

/// One fixed emitter: spawn box plus init ranges plus force terms, all plain
/// data in meters, seconds and per-second rates. No RNG lives here; the pool
/// owns the single stream so call order alone decides the bytes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PoolEmitter {
    /// Which fixed kind this slot runs.
    pub kind: EmitterKind,
    /// Spawn box center in meters.
    pub origin_m: [f32; 3],
    /// Spawn box half-extent in meters.
    pub extent_m: [f32; 3],
    /// Mean launch direction (need not be unit; normalized per spawn).
    pub dir: [f32; 3],
    /// Cone jitter around `dir` in m/s per axis.
    pub spread: f32,
    /// Steady spawn rate per second after the first-tick burst.
    pub rate_per_s: f32,
    /// One-shot burst on tick zero.
    pub burst: u32,
    /// Particle life range in seconds, ascending, within `MAX_LIFE_S`.
    pub life_s: [f32; 2],
    /// Launch speed range in m/s, ascending.
    pub speed_m_s: [f32; 2],
    /// Vertical acceleration in m/s^2 (negative rises, like flame buoyancy).
    pub gravity_m_s2: f32,
    /// Linear drag per second.
    pub drag_per_s: f32,
    /// Base quad size range in meters, ascending.
    pub size_m: [f32; 2],
    /// Tint at birth (RGBA).
    pub tint_a: [f32; 4],
    /// Tint at death (RGBA, before the age fade).
    pub tint_b: [f32; 4],
}

impl PoolEmitter {
    /// Slow settling footwork dust.
    pub fn dust() -> Self {
        Self {
            kind: EmitterKind::Dust,
            origin_m: [0.0, 0.2, 0.0],
            extent_m: [1.5, 0.1, 1.5],
            dir: [0.0, 1.0, 0.0],
            spread: 0.6,
            rate_per_s: 180.0,
            burst: 24,
            life_s: [0.8, 1.8],
            speed_m_s: [0.5, 2.0],
            gravity_m_s2: 0.6,
            drag_per_s: 1.2,
            size_m: [0.08, 0.20],
            tint_a: [0.85, 0.78, 0.66, 1.0],
            tint_b: [0.85, 0.78, 0.66, 0.0],
        }
    }

    /// Buoyant torch flame puffs.
    pub fn flame() -> Self {
        Self {
            kind: EmitterKind::Flame,
            origin_m: [0.0, 1.0, 0.0],
            extent_m: [0.15, 0.1, 0.15],
            dir: [0.0, 1.0, 0.0],
            spread: 0.25,
            rate_per_s: 90.0,
            burst: 12,
            life_s: [0.4, 0.9],
            speed_m_s: [1.0, 2.5],
            gravity_m_s2: -2.5,
            drag_per_s: 0.4,
            size_m: [0.05, 0.12],
            tint_a: [1.0, 0.8, 0.4, 1.0],
            tint_b: [0.9, 0.2, 0.1, 0.0],
        }
    }

    /// Fast hot strike sparks.
    pub fn sparks() -> Self {
        Self {
            kind: EmitterKind::Sparks,
            origin_m: [0.0, 0.8, 0.0],
            extent_m: [0.1, 0.1, 0.1],
            dir: [0.0, 1.0, 0.0],
            spread: 1.0,
            rate_per_s: 60.0,
            burst: 8,
            life_s: [0.4, 1.0],
            speed_m_s: [3.0, 8.0],
            gravity_m_s2: 9.0,
            drag_per_s: 0.2,
            size_m: [0.02, 0.05],
            tint_a: [1.0, 0.95, 0.7, 1.0],
            tint_b: [1.0, 0.4, 0.1, 0.0],
        }
    }

    /// Check every range and NaN before a build or a submit trusts this.
    pub fn validate(&self) -> Result<(), ParticleError> {
        for (name, v) in [
            ("origin", self.origin_m.as_slice()),
            ("extent", self.extent_m.as_slice()),
            ("dir", self.dir.as_slice()),
        ] {
            if !v.iter().all(|c| c.is_finite()) {
                return Err(ParticleError::Invalid(format!(
                    "emitter {name} is not finite"
                )));
            }
        }
        if !self.extent_m.iter().all(|c| *c >= 0.0) {
            return Err(ParticleError::Invalid(
                "emitter extent must be non-negative".into(),
            ));
        }
        let dir_sq =
            self.dir[0] * self.dir[0] + self.dir[1] * self.dir[1] + self.dir[2] * self.dir[2];
        if !dir_sq.is_finite() || dir_sq <= 1e-9 {
            return Err(ParticleError::Invalid("emitter dir must be nonzero".into()));
        }
        if !self.spread.is_finite() || self.spread < 0.0 {
            return Err(ParticleError::Invalid(
                "emitter spread must be non-negative finite".into(),
            ));
        }
        if !self.rate_per_s.is_finite() || self.rate_per_s < 0.0 {
            return Err(ParticleError::Invalid(
                "emitter rate must be non-negative finite".into(),
            ));
        }
        if self.rate_per_s > MAX_RATE_PER_S {
            return Err(ParticleError::BudgetExceeded {
                requested: self.rate_per_s as u32,
                allowed: MAX_RATE_PER_S as u32,
            });
        }
        let [lo, hi] = self.life_s;
        if !lo.is_finite() || !hi.is_finite() || lo < 1e-3 || hi < lo || hi > MAX_LIFE_S {
            return Err(ParticleError::Invalid(format!(
                "emitter life [{lo}, {hi}] must ascend within (0, {MAX_LIFE_S}]"
            )));
        }
        let [slo, shi] = self.speed_m_s;
        if !slo.is_finite() || !shi.is_finite() || slo < 0.0 || shi < slo {
            return Err(ParticleError::Invalid(format!(
                "emitter speed [{slo}, {shi}] must ascend from zero"
            )));
        }
        if !self.gravity_m_s2.is_finite() || !self.drag_per_s.is_finite() {
            return Err(ParticleError::Invalid(
                "emitter gravity and drag must be finite".into(),
            ));
        }
        if self.drag_per_s < 0.0 {
            return Err(ParticleError::Invalid(
                "emitter drag must be non-negative".into(),
            ));
        }
        let [zlo, zhi] = self.size_m;
        if !zlo.is_finite() || !zhi.is_finite() || zlo < 1e-6 || zhi < zlo {
            return Err(ParticleError::Invalid(format!(
                "emitter size [{zlo}, {zhi}] must ascend from positive"
            )));
        }
        for (name, tint) in [("tint_a", self.tint_a), ("tint_b", self.tint_b)] {
            if !tint.iter().all(|c| c.is_finite()) {
                return Err(ParticleError::Invalid(format!(
                    "emitter {name} is not finite"
                )));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Particles: one fixed pool, structure-of-arrays shape per particle row.
// ---------------------------------------------------------------------------

/// One live particle: meters, seconds and a flipbook cell. `kind` repeats
/// the emitter tag so the byte image needs no side table.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GpuParticle {
    /// Center in meters.
    pub pos: [f32; 3],
    /// Velocity in m/s.
    pub vel: [f32; 3],
    /// Age in seconds.
    pub age_s: f32,
    /// Life in seconds; dead once `age_s >= life_s`.
    pub life_s: f32,
    /// Base quad size in meters at birth.
    pub base_m: f32,
    /// Current quad size in meters after the age curve.
    pub size_m: f32,
    /// Current alpha after color and age fades.
    pub alpha: f32,
    /// Current tint (RGBA).
    pub tint: [f32; 4],
    /// Flipbook cell index into the kind grid.
    pub frame: u16,
    /// Owner tag (`EmitterKind::index`).
    pub kind: u8,
}

/// SplitMix64 stream: the only RNG in the pool. State advances by the odd
/// gamma each draw, so consecutive seeds diverge from the first particle.
#[derive(Clone, Debug)]
struct SeedStream(u64);

impl SeedStream {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform unit draw in [0, 1).
    fn next_unit(&mut self) -> f32 {
        (self.next_u64() >> 32) as u32 as f32 / 4_294_967_300.0
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Accrue fractional spawns for one emitter and return this tick's spawn
/// count. Adds `rate_per_s` (clamped at zero) times `dt` seconds to `accrue`,
/// takes the whole part, and caps at [`MAX_SPAWN_PER_TICK`]; excess stays
/// accrued, never bursts. Units are counts and seconds. Deterministic: only
/// `+ - * /` and one `floor`; identical accumulator, rate and step give
/// identical counts. Shared owner for the CPU mirror in `particles.rs`.
pub(crate) fn throttle_spawn(accrue: &mut f32, rate_per_s: f32, dt: f32) -> u32 {
    *accrue += rate_per_s.max(0.0) * dt;
    // Accrue stays finite and non-negative here: rates clamp at zero, steps
    // are positive, and each tick removes the whole part, so the floor lands
    // far under `u32::MAX` and the truncations below are exact. The trio
    // lives here once so both pools share one owner instead of two copies.
    let mut n = accrue.floor() as u32;
    *accrue -= n as f32;
    if n > MAX_SPAWN_PER_TICK {
        *accrue += (n - MAX_SPAWN_PER_TICK) as f32;
        n = MAX_SPAWN_PER_TICK;
    }
    n
}

/// Fixed seeded pool for the three emitters: spawn in fixed order, integrate
/// in slot order, cull dead with a stable compact. The live buffer is sized
/// once at build; stepping never grows it.
pub struct ParticlePool {
    emitters: [PoolEmitter; EMITTER_COUNT],
    live: Vec<GpuParticle>,
    stream: SeedStream,
    accrue: [f32; EMITTER_COUNT],
    time_s: f32,
    ticks: u64,
    spawned: u64,
    dropped_total: u64,
    capped: bool,
    cap: usize,
}

impl ParticlePool {
    /// Fixed pool (dust, flame, sparks) with [`POOL_CAP`] slots.
    pub fn new(seed: u64) -> Self {
        let emitters = [
            PoolEmitter::dust(),
            PoolEmitter::flame(),
            PoolEmitter::sparks(),
        ];
        let mut live = Vec::new();
        live.reserve_exact(POOL_CAP);
        Self {
            emitters,
            live,
            stream: SeedStream::new(seed),
            accrue: [0.0; EMITTER_COUNT],
            time_s: 0.0,
            ticks: 0,
            spawned: 0,
            dropped_total: 0,
            capped: false,
            cap: POOL_CAP,
        }
    }

    /// Fixed emitters with a custom cap in `1..=POOL_CAP_HARD`.
    pub fn with_capacity(seed: u64, cap: usize) -> Result<Self, ParticleError> {
        if cap == 0 {
            return Err(ParticleError::Invalid(
                "particle cap must be at least 1".into(),
            ));
        }
        if cap > POOL_CAP_HARD {
            return Err(ParticleError::BudgetExceeded {
                requested: cap as u32,
                allowed: POOL_CAP_HARD as u32,
            });
        }
        let mut pool = Self::new(seed);
        pool.cap = cap;
        let mut live = Vec::new();
        live.reserve_exact(cap);
        pool.live = live;
        Ok(pool)
    }

    /// Slots the pool holds at most.
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Check the pool before a submit trusts it.
    pub fn validate(&self) -> Result<(), ParticleError> {
        if self.cap == 0 || self.cap > POOL_CAP_HARD {
            return Err(ParticleError::Invalid(format!(
                "particle cap {} out of 1..={POOL_CAP_HARD}",
                self.cap
            )));
        }
        for e in self.emitters.iter() {
            e.validate()?;
        }
        Ok(())
    }

    /// Advance one fixed tick.
    pub fn tick(&mut self) {
        self.step(TICK_DT);
    }

    /// Advance by `dt` seconds. Non-finite and non-positive steps are
    /// ignored; bigger steps clamp to [`MAX_DT_CLAMP`]. Spawn order is dust,
    /// flame, sparks; integration is slot order; the cull is stable.
    pub fn step(&mut self, dt: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let dt = if dt > MAX_DT_CLAMP { MAX_DT_CLAMP } else { dt };
        for ei in 0..EMITTER_COUNT {
            let burst = if self.ticks == 0 {
                self.emitters[ei].burst
            } else {
                0
            };
            for _ in 0..burst {
                self.spawn(ei);
            }
            let rate = self.emitters[ei].rate_per_s;
            let n = throttle_spawn(&mut self.accrue[ei], rate, dt);
            for _ in 0..n {
                self.spawn(ei);
            }
        }
        for i in 0..self.live.len() {
            let tag = self.live[i].kind;
            let kind = EmitterKind::from_tag(tag);
            let slot = kind.index().min(EMITTER_COUNT - 1);
            let gravity = self.emitters[slot].gravity_m_s2;
            let drag = self.emitters[slot].drag_per_s;
            let p = &mut self.live[i];
            p.age_s += dt;
            p.vel[1] -= gravity * dt;
            let k = 1.0 / (1.0 + drag * dt);
            p.vel[0] *= k;
            p.vel[1] *= k;
            p.vel[2] *= k;
            p.pos[0] += p.vel[0] * dt;
            p.pos[1] += p.vel[1] * dt;
            p.pos[2] += p.vel[2] * dt;
            let age01 = (p.age_s / p.life_s).clamp(0.0, 1.0);
            let emitter = self.emitters[slot];
            let base = p.base_m;
            let p = &mut self.live[i];
            p.size_m = base * kind.size_mul(age01);
            let fade = kind.alpha_mul(age01);
            p.alpha = lerp(emitter.tint_a[3], emitter.tint_b[3], age01) * fade;
            p.tint = [
                lerp(emitter.tint_a[0], emitter.tint_b[0], age01),
                lerp(emitter.tint_a[1], emitter.tint_b[1], age01),
                lerp(emitter.tint_a[2], emitter.tint_b[2], age01),
                p.alpha,
            ];
            let (cols, rows) = kind.grid();
            let cells = cols.max(1) as usize * rows.max(1) as usize;
            let mut cell = (age01 * cells as f32) as usize;
            if kind.looping() {
                cell %= cells;
            } else if cell >= cells {
                cell = cells - 1;
            }
            p.frame = cell as u16;
        }
        self.live.retain(|p| p.age_s < p.life_s);
        self.time_s += dt;
        self.ticks += 1;
    }

    /// Advance one fixed tick and time it. Wall clock lands in the receipt
    /// only, never in bytes or hashes.
    pub fn step_timed(&mut self, dt: f32) -> ParticleReceipt {
        let start = Instant::now();
        self.step(dt);
        ParticleReceipt {
            pass: "particles_gpu",
            ticks: self.ticks,
            alive: self.live.len(),
            spawned: self.spawned,
            ms: start.elapsed().as_secs_f64() * 1000.0,
            degraded: self.capped,
        }
    }

    /// Live particle count, always within the cap.
    pub fn alive(&self) -> usize {
        self.live.len()
    }

    /// Total spawns since build, including culled particles.
    pub fn spawned_total(&self) -> u64 {
        self.spawned
    }

    /// Total spawns refused by the cap since build.
    pub fn dropped_total(&self) -> u64 {
        self.dropped_total
    }

    /// True once a spawn hit the cap; the pool kept simulating, degraded.
    pub fn capped(&self) -> bool {
        self.capped
    }

    /// Simulated seconds.
    pub fn time_s(&self) -> f32 {
        self.time_s
    }

    /// Simulated ticks.
    pub fn tick_count(&self) -> u64 {
        self.ticks
    }

    /// Live particles in slot order.
    pub fn particles(&self) -> &[GpuParticle] {
        &self.live
    }

    /// The three fixed emitters in integration order.
    pub fn emitters(&self) -> &[PoolEmitter; EMITTER_COUNT] {
        &self.emitters
    }

    /// One batched draw over dust, flame, sparks in fixed order.
    /// Caller: the next present bind reads [`vfx_hanabi::HanabiBatch`] here.
    pub fn hanabi_dispatch(&self, batch: &vfx_hanabi::HanabiBatch) -> u32 {
        debug_assert!(self.emitters[0].kind == EmitterKind::Dust);
        debug_assert!(self.emitters[1].kind == EmitterKind::Flame);
        debug_assert!(self.emitters[2].kind == EmitterKind::Sparks);
        batch.dispatch_count()
    }
    /// Backing capacity probe for the no-steady-state-allocation test.
    pub fn slot_capacity(&self) -> usize {
        self.live.capacity()
    }

    /// Retune one steady rate. Finite rates within the max apply at the next
    /// tick; anything else is refused and the old rate stands.
    pub fn set_rate(&mut self, kind: EmitterKind, rate: f32) -> Result<(), ParticleError> {
        if !rate.is_finite() {
            return Err(ParticleError::Invalid(format!("rate {rate} is not finite")));
        }
        if rate < 0.0 {
            return Err(ParticleError::Invalid(format!("rate {rate} is negative")));
        }
        if rate > MAX_RATE_PER_S {
            return Err(ParticleError::BudgetExceeded {
                requested: rate as u32,
                allowed: MAX_RATE_PER_S as u32,
            });
        }
        self.emitters[kind.index()].rate_per_s = rate;
        Ok(())
    }

    /// Deterministic byte image: tick plus count plus every live particle in
    /// slot order. Identical seeds give identical bytes, tick for tick.
    pub fn particle_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + self.live.len() * 43);
        out.extend_from_slice(&self.ticks.to_le_bytes());
        out.extend_from_slice(&(self.live.len() as u32).to_le_bytes());
        for p in self.live.iter() {
            for w in [
                p.pos[0].to_bits(),
                p.pos[1].to_bits(),
                p.pos[2].to_bits(),
                p.vel[0].to_bits(),
                p.vel[1].to_bits(),
                p.vel[2].to_bits(),
                p.age_s.to_bits(),
                p.life_s.to_bits(),
                p.size_m.to_bits(),
                p.alpha.to_bits(),
            ] {
                out.extend_from_slice(&w.to_le_bytes());
            }
            out.extend_from_slice(&p.frame.to_le_bytes());
            out.push(p.kind);
        }
        out
    }

    /// FNV-1a hash over [`Self::particle_bytes`]; same seed, same hash.
    pub fn state_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in self.particle_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01B3);
        }
        h
    }

    /// GPU submit seam for the RA2 billboard bind. Validates the pool, then
    /// names the missing window device bind on the headless twin (never
    /// silent success). The CPU sim above is the authority here; the
    /// `window` build below records the live pass instead.
    #[cfg(not(feature = "window"))]
    pub fn submit_gpu(&self, viewport_px: [u32; 2]) -> Result<ParticleReceipt, ParticleError> {
        self.validate()?;
        if viewport_px[0] == 0 || viewport_px[1] == 0 {
            return Err(ParticleError::Invalid(format!(
                "particle viewport {}x{} is zero",
                viewport_px[0], viewport_px[1]
            )));
        }
        if viewport_px[0] > MAX_VIEWPORT_PX || viewport_px[1] > MAX_VIEWPORT_PX {
            return Err(ParticleError::Invalid(format!(
                "particle viewport {}x{} exceeds {MAX_VIEWPORT_PX}",
                viewport_px[0], viewport_px[1]
            )));
        }
        Err(ParticleError::NoDevice(
            "particles_gpu: window build with a live device binds the pool; CPU sim is the authority".into(),
        ))
    }

    fn spawn(&mut self, ei: usize) {
        if self.live.len() >= self.cap {
            self.capped = true;
            self.dropped_total += 1;
            return;
        }
        let e = self.emitters[ei];
        let jx = self.stream.next_unit() * 2.0 - 1.0;
        let jy = self.stream.next_unit() * 2.0 - 1.0;
        let jz = self.stream.next_unit() * 2.0 - 1.0;
        let dx = self.stream.next_unit() * 2.0 - 1.0;
        let dy = self.stream.next_unit() * 2.0 - 1.0;
        let dz = self.stream.next_unit() * 2.0 - 1.0;
        let u_speed = self.stream.next_unit();
        let u_life = self.stream.next_unit();
        let u_size = self.stream.next_unit();
        let mut vx = e.dir[0] + dx * e.spread;
        let mut vy = e.dir[1] + dy * e.spread;
        let mut vz = e.dir[2] + dz * e.spread;
        let mut len = (vx * vx + vy * vy + vz * vz).sqrt();
        if !len.is_finite() || len <= 1e-6 {
            vx = 0.0;
            vy = 1.0;
            vz = 0.0;
            len = 1.0;
        }
        let speed = e.speed_m_s[0] + (e.speed_m_s[1] - e.speed_m_s[0]) * u_speed;
        let life = (e.life_s[0] + (e.life_s[1] - e.life_s[0]) * u_life).max(1e-3);
        let base = e.size_m[0] + (e.size_m[1] - e.size_m[0]) * u_size;
        let (cols, rows) = e.kind.grid();
        let cells = cols.max(1) as usize * rows.max(1) as usize;
        debug_assert!(cells > 0);
        self.live.push(GpuParticle {
            pos: [
                e.origin_m[0] + jx * e.extent_m[0],
                e.origin_m[1] + jy * e.extent_m[1],
                e.origin_m[2] + jz * e.extent_m[2],
            ],
            vel: [vx / len * speed, vy / len * speed, vz / len * speed],
            age_s: 0.0,
            life_s: life,
            base_m: base,
            size_m: base * e.kind.size_mul(0.0),
            alpha: lerp(e.tint_a[3], e.tint_b[3], 0.0) * e.kind.alpha_mul(0.0),
            tint: [e.tint_a[0], e.tint_a[1], e.tint_a[2], e.tint_a[3]],
            frame: 0,
            kind: e.kind.index() as u8,
        });
        self.spawned += 1;
    }
}

// ---------------------------------------------------------------------------
// Staged draw helpers (CPU) plus staged draw WGSL (not bound yet).
// The bind probe below still owns the submit; this slice stages math only.
// ---------------------------------------------------------------------------

/// Soft depth fade range in meters; gaps at or past this read fully faded.
pub const SOFT_FADE_RANGE_M: f32 = 0.3;
/// Velocity stretch gain per m/s; stretch factor is `1 + speed * gain`.
pub const STRETCH_GAIN: f32 = 0.25;
/// Largest velocity stretch factor; longer asks clamp, never explode.
pub const STRETCH_MAX: f32 = 3.0;

/// Soft depth fade over the scene gap in meters. Returns 1 at zero gap and
/// 0 at or past [`SOFT_FADE_RANGE_M`], linear and monotonic between.
/// Units are meters in, unitless factor out. Deterministic: fixed math only,
/// no wall clock. Non-finite gaps read 0; gaps at or below zero read 1.
pub fn soft_fade(depth_gap: f32) -> f32 {
    if !depth_gap.is_finite() {
        return 0.0;
    }
    if depth_gap <= 0.0 {
        return 1.0;
    }
    if depth_gap >= SOFT_FADE_RANGE_M {
        return 0.0;
    }
    1.0 - depth_gap / SOFT_FADE_RANGE_M
}

/// Two flipbook cells plus blend weights for one fractional frame.
/// Units are cells and unitless weights. Deterministic: `floor` plus
/// fixed math only. Weights always sum to 1; empty atlases read cell 0.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FlipbookPair {
    /// First cell id, always inside the atlas.
    pub cell_a: u32,
    /// Second cell id, always inside the atlas.
    pub cell_b: u32,
    /// Weight for `cell_a`; equals `1 - frac`.
    pub weight_a: f32,
    /// Weight for `cell_b`; equals `frac`.
    pub weight_b: f32,
}

/// u32 to f32 without a numeric `as` cast: split into 16-bit halves, each
/// exactly representable, then scale and add. Exact below 2^24, which covers
/// every flipbook atlas here (at most 16 cells); huge counts round the way
/// the hardware conversion would. Deterministic: fixed math only.
fn u32_to_f32(v: u32) -> f32 {
    let hi = v >> 16;
    let lo = v & 0xFFFF;
    // `>> 16` always fits a u16; the fallback is unreachable-by-construction.
    let hi_f = f32::from(u16::try_from(hi).unwrap_or(u16::MAX));
    let lo_f = f32::from(u16::try_from(lo).unwrap_or(u16::MAX));
    hi_f * 65536.0 + lo_f
}

/// Floored non-negative f32 to u32, saturating, without a numeric `as` cast.
/// Reads the float bits with integer ops only; truncation of a non-negative
/// value is its floor. Exact below 2^31; larger magnitudes saturate to
/// u32::MAX and the caller clamps into the atlas, so saturation only picks
/// the last cell. Deterministic: integer ops only, no wall clock.
fn floor_to_u32_sat(v: f32) -> u32 {
    if !v.is_finite() || v <= 0.0 {
        return 0;
    }
    let bits = v.to_bits();
    let exp = (bits >> 23) & 0xFF;
    if exp < 127 {
        return 0;
    }
    if exp >= 158 {
        return u32::MAX;
    }
    let shift = exp - 127;
    let mant = (bits & 0x7F_FFFF) | 0x80_0000;
    if shift >= 23 {
        mant << (shift - 23)
    } else {
        mant >> (23 - shift)
    }
}

/// Pick the two flipbook cells around `frame_f` for a `cells`-cell atlas.
/// `frame_f` is the fractional frame cursor; `cells` is the cell count.
/// Clamps outside cursors into range; the last cell blends into itself.
/// Deterministic: `floor` plus fixed math only, no wall clock.
pub fn flipbook_pair(frame_f: f32, cells: u32) -> FlipbookPair {
    if cells == 0 {
        return FlipbookPair {
            cell_a: 0,
            cell_b: 0,
            weight_a: 1.0,
            weight_b: 0.0,
        };
    }
    if cells == 1 || !frame_f.is_finite() {
        return FlipbookPair {
            cell_a: 0,
            cell_b: 0,
            weight_a: 1.0,
            weight_b: 0.0,
        };
    }
    let last = cells - 1;
    let max_f = u32_to_f32(last);
    let clamped = frame_f.clamp(0.0, max_f);
    let base_f = clamped.floor();
    let frac = clamped - base_f;
    let cell_a = floor_to_u32_sat(base_f).min(last);
    let cell_b = cell_a.saturating_add(1).min(last);
    if cell_b == cell_a {
        return FlipbookPair {
            cell_a,
            cell_b,
            weight_a: 1.0,
            weight_b: 0.0,
        };
    }
    FlipbookPair {
        cell_a,
        cell_b,
        weight_a: 1.0 - frac,
        weight_b: frac,
    }
}

/// Velocity stretch length plus direction. Units are factor out and m/s in.
/// Deterministic: one `sqrt` plus fixed math only, no wall clock.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StretchLenDir {
    /// Stretch factor: 1 is round, larger is longer, at most [`STRETCH_MAX`].
    pub len: f32,
    /// Finite unit direction of motion, or up when still.
    pub dir: [f32; 3],
}

/// Stretch factor grows with speed and clamps at [`STRETCH_MAX`].
/// Direction is velocity normalized; still or non-finite input reads up
/// with factor 1. Finite output is guaranteed. Units are m/s in.
pub fn stretch_len_dir(vel: [f32; 3]) -> StretchLenDir {
    let speed_sq = vel[0] * vel[0] + vel[1] * vel[1] + vel[2] * vel[2];
    if !speed_sq.is_finite() {
        return StretchLenDir {
            len: 1.0,
            dir: [0.0, 1.0, 0.0],
        };
    }
    let speed = speed_sq.sqrt();
    if !speed.is_finite() || speed <= 1e-6 {
        return StretchLenDir {
            len: 1.0,
            dir: [0.0, 1.0, 0.0],
        };
    }
    let dir = [vel[0] / speed, vel[1] / speed, vel[2] / speed];
    if !dir.iter().all(|c| c.is_finite()) {
        return StretchLenDir {
            len: 1.0,
            dir: [0.0, 1.0, 0.0],
        };
    }
    let grown = 1.0 + speed * STRETCH_GAIN;
    let len = grown.min(STRETCH_MAX);
    if !len.is_finite() {
        return StretchLenDir {
            len: 1.0,
            dir: [0.0, 1.0, 0.0],
        };
    }
    StretchLenDir { len, dir }
}

/// One staged instance for the Kenney flipbook draw: the CPU mirror of one
/// `particle.wgsl` instance over the seeded pool. Units are meters for
/// position and size, cells for the flipbook pair, unitless for blend
/// weights, fade and stretch. Deterministic: [`flipbook_pair`],
/// [`soft_fade`] and [`stretch_len_dir`] only, no wall clock.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DrawRecord {
    /// Center in meters.
    pub pos: [f32; 3],
    /// Quad size in meters after the age curve.
    pub size_m: f32,
    /// First flipbook cell, always inside the kind grid.
    pub frame_a: u32,
    /// Second flipbook cell, always inside the kind grid.
    pub frame_b: u32,
    /// Blend weight for `frame_a`; `1 - frac` of the age cursor.
    pub blend_a: f32,
    /// Blend weight for `frame_b`; `frac` of the age cursor.
    pub blend_b: f32,
    /// Soft depth fade factor; 1 with the fade off.
    pub fade: f32,
    /// Velocity stretch factor; 1 is round, at most [`STRETCH_MAX`].
    pub stretch: f32,
    /// Current tint (RGBA).
    pub tint: [f32; 4],
    /// Owner tag (`EmitterKind::index`); output runs 0, 1, 2 in order.
    pub kind: u8,
}

impl ParticlePool {
    /// Stage one [`DrawRecord`] per live particle for the fixed emitters in
    /// fixed order (dust, flame, sparks; live slot order inside each kind).
    /// Feed staged for the window ParticleSubmitter submit: the live bind
    /// plus scene-depth gaps land with the submit.rs depth resolve, which
    /// sits outside this packet's paths. The CPU sim stays the authority.
    /// `gap_of` names each particle's scene-depth gap in meters; with `soft_on` the fade is
    /// [`soft_fade`] of that gap, otherwise 1. Records append to `out`
    /// (caller-owned, reused across frames); the return is the appended
    /// count. Pool bounds and the seeded stream are untouched.
    /// Deterministic: fixed order plus fixed math only, no wall clock.
    pub fn draw_records(
        &self,
        soft_on: bool,
        gap_of: impl Fn(&GpuParticle) -> f32,
        out: &mut Vec<DrawRecord>,
    ) -> usize {
        let base = out.len();
        for kind in [EmitterKind::Dust, EmitterKind::Flame, EmitterKind::Sparks] {
            let (cols, rows) = kind.grid();
            let cells = u32::from(cols.max(1)) * u32::from(rows.max(1));
            let cells_f = u32_to_f32(cells);
            let slot = kind.index();
            for p in self.live.iter().filter(|p| usize::from(p.kind) == slot) {
                let age01 = (p.age_s / p.life_s).clamp(0.0, 1.0);
                let pair = flipbook_pair(age01 * cells_f, cells);
                let stretch = stretch_len_dir(p.vel);
                let fade = if soft_on { soft_fade(gap_of(p)) } else { 1.0 };
                out.push(DrawRecord {
                    pos: p.pos,
                    size_m: p.size_m,
                    frame_a: pair.cell_a,
                    frame_b: pair.cell_b,
                    blend_a: pair.weight_a,
                    blend_b: pair.weight_b,
                    fade,
                    stretch: stretch.len,
                    tint: p.tint,
                    kind: p.kind,
                });
            }
        }
        out.len() - base
    }
}

/// Kenney flipbook draw shader over the fixed dust, flame, sparks emitters
/// in fixed order; pool bounds and the seeded stream are untouched.
/// Draw path is the `shaders/particle.wgsl` file; `ParticleSubmitter`
/// still binds `PARTICLE_BIND_WGSL` on the window build.
/// Engine-owned WGSL, fixed math only, no wall clock.
pub const PARTICLE_DRAW_WGSL: &str = include_str!("shaders/particle.wgsl");

// ---------------------------------------------------------------------------
// Window device submit (behind `window`): the live pool byte image bound
// under the shared LookFrame bind shape. Engine-owned WGSL, no donor code.
// ---------------------------------------------------------------------------

/// Bind probe over the pool byte image: copies the bound words to prove
/// the billboard bind-group path validates. Look uniform bound and live.
#[cfg(feature = "window")]
const PARTICLE_BIND_WGSL: &str = r#"
struct LookU {
    frame_idx: u32,
    width_px: u32,
    height_px: u32,
    exposure: f32,
    sun_dir: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0) var<uniform> look: LookU;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> dst: array<u32>;
@compute @workgroup_size(64)
fn bind_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&src)) { return; }
    dst[i] = src[i] + u32(look.exposure * 0.0);
}
"#;

/// One window submit over the live pool: tick cursor, live count, the
/// validation outcome and the wall-clock encode + submit + readback ms.
#[cfg(feature = "window")]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ParticleGpuReceipt {
    /// Ticks simulated when the receipt was taken.
    pub ticks: u64,
    /// Live particles bound.
    pub alive: usize,
    /// Total spawns at the cursor.
    pub spawned: u64,
    /// wgpu validation errors observed; green means 0.
    pub validation_errors: u32,
    /// Wall time for encode + submit + readback in ms (evidence only).
    pub submit_ms: f64,
}

#[cfg(feature = "window")]
impl ParticleGpuReceipt {
    /// Posted GPU budget this submit is charged against.
    pub fn budget_ms(&self) -> f64 {
        PARTICLE_BUDGET_MS
    }

    /// True while the submit stayed inside the posted budget.
    pub fn within_budget(&self) -> bool {
        self.submit_ms <= PARTICLE_BUDGET_MS
    }

    /// True when the submit observed zero validation errors.
    pub fn is_clean(&self) -> bool {
        self.validation_errors == 0
    }
}

/// Reusable window submit state: the bind-probe pipeline plus the look bind
/// layout over one uniform buffer, with shape-keyed word buffers and one
/// bind group. Built once per frame loop (WIN1 and ED1 each own one);
/// steady-state submits on a matching shape create no buffers or bind
/// groups (the reuse test proves the count stays flat).
///
/// Beside the probe sits the staged draw side ([`Self::draw_staged`] plus
/// [`Self::encode_draw`]): the flipbook pipeline over [`PARTICLE_DRAW_WGSL`]
/// with the view uniform, the max-size instance buffer and one bind group,
/// built on the first non-empty batch and reused after.
#[cfg(feature = "window")]
pub struct ParticleSubmitter {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    look_buf: wgpu::Buffer,
    cached_words: Option<u64>,
    src_buf: Option<wgpu::Buffer>,
    dst_buf: Option<wgpu::Buffer>,
    readback: Option<wgpu::Buffer>,
    bind: Option<wgpu::BindGroup>,
    created_buffers: u64,
    draw_view: [u8; PARTICLE_DRAW_UNIFORM_BYTES],
    draw: Option<ParticleDrawState>,
}

/// Device state for the staged draw: the flipbook pipeline built from
/// [`PARTICLE_DRAW_WGSL`], the 112-byte view uniform, the max-size instance
/// buffer, the one-texel atlas stand-in and the bind group that ties them
/// together under the shader's group 0. Built once, on the first non-empty
/// batch; every later batch rewrites the same buffers.
#[cfg(feature = "window")]
struct ParticleDrawState {
    pipeline: wgpu::RenderPipeline,
    bind: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    instances: wgpu::Buffer,
    scene_depth: wgpu::Buffer,
    /// The one-texel white atlas stand-in the bind group samples.
    atlas: wgpu::Texture,
    /// Staged instance bytes, reused across frames (grown once).
    packed: Vec<u8>,
    /// Instances in the batch currently on the device; 0 draws nothing.
    staged: u32,
}

#[cfg(feature = "window")]
impl ParticleSubmitter {
    /// Compile the bind-probe pipeline and the look bind layout; allocate
    /// the 32-byte look uniform buffer. No pool-size state yet: word
    /// buffers size on the first submit and resize only on shape change.
    pub fn new(device: &wgpu::Device) -> Self {
        use crate::pipeline_warmup::CountedPipelines as _;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rp2-particle-bind"),
            source: wgpu::ShaderSource::Wgsl(PARTICLE_BIND_WGSL.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rp2-particle-bind-layout"),
            entries: &[
                compute_uniform_entry(0),
                compute_storage_entry(1, true),
                compute_storage_entry(2, false),
            ],
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rp2-particle-pipe-layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline_counted(&wgpu::ComputePipelineDescriptor {
            label: Some("rp2-particle-pipeline"),
            layout: Some(&pipe_layout),
            module: &shader,
            entry_point: Some("bind_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let look_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rp2-particle-look-uniform"),
            size: crate::graph::LOOK_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            look_buf,
            cached_words: None,
            src_buf: None,
            dst_buf: None,
            readback: None,
            bind: None,
            created_buffers: 0,
            // Identity draw camera (right +X, up +Y) until a frame binds one.
            draw_view: particle_pack_uniform(
                [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0],
                ],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
            ),
            draw: None,
        }
    }

    /// Sized word plus readback buffers created so far (probe for the
    /// reuse test: flat across matching-shape submits).
    pub fn created_buffers(&self) -> u64 {
        self.created_buffers
    }

    /// Bind the frame camera the staged draw uses: the view-projection the
    /// HDR frame is drawn with, in column-major order, plus the camera right
    /// and up axes the billboard expands along (world space meters, unit
    /// length). Non-finite input is refused before anything reaches the
    /// device, so a bad camera never draws a wild quad. Called before the
    /// first batch it only stages the bytes, which that batch uploads.
    /// Steady frames rewrite the same 112 bytes: no frame creates anything.
    pub fn set_draw_view(
        &mut self,
        queue: &wgpu::Queue,
        view_proj: [[f32; 4]; 4],
        cam_right: [f32; 3],
        cam_up: [f32; 3],
    ) -> Result<(), ParticleError> {
        if !view_proj
            .iter()
            .all(|col| col.iter().all(|c| c.is_finite()))
            || !cam_right.iter().all(|c| c.is_finite())
            || !cam_up.iter().all(|c| c.is_finite())
        {
            return Err(ParticleError::Invalid(
                "particle draw view is not finite".into(),
            ));
        }
        let packed = particle_pack_uniform(view_proj, cam_right, cam_up);
        self.draw_view = packed;
        if let Some(draw) = self.draw.as_ref() {
            queue.write_buffer(&draw.uniform, 0, &packed);
        }
        Ok(())
    }

    /// Upload one staged batch (one instance per [`DrawRecord`], in the
    /// feed's fixed emitter order) and report its instance count. An empty
    /// batch is a calm frame: it reports 0 and sizes nothing. The first
    /// non-empty batch builds the pipeline, the view uniform, the atlas
    /// stand-in and the max-size instance buffer; every later batch rewrites
    /// the same buffers and bind group, so a same-shape batch creates
    /// nothing. Over-cap batches are refused before any device work. This
    /// path never maps and never waits on the device.
    pub fn draw_staged(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        records: &[DrawRecord],
    ) -> Result<u32, ParticleError> {
        if records.len() > PARTICLE_DRAW_MAX_INSTANCES {
            return Err(ParticleError::BudgetExceeded {
                requested: u32::try_from(records.len()).unwrap_or(u32::MAX),
                allowed: u32::try_from(PARTICLE_DRAW_MAX_INSTANCES).unwrap_or(u32::MAX),
            });
        }
        if records.is_empty() {
            if let Some(draw) = self.draw.as_mut() {
                draw.staged = 0;
            }
            return Ok(0);
        }
        if self.draw.is_none() {
            self.draw = Some(ParticleDrawState::build(device, queue, &self.draw_view)?);
            self.created_buffers = self.created_buffers.saturating_add(3);
        }
        let Some(draw) = self.draw.as_mut() else {
            return Err(ParticleError::Device(
                "particle draw state lost its built buffers".into(),
            ));
        };
        let bytes = records.len() * PARTICLE_DRAW_INSTANCE_BYTES;
        draw.packed.clear();
        draw.packed.resize(bytes, 0);
        for (slot, record) in records.iter().enumerate() {
            let at = slot * PARTICLE_DRAW_INSTANCE_BYTES;
            particle_pack_instance(
                record,
                &mut draw.packed[at..at + PARTICLE_DRAW_INSTANCE_BYTES],
            );
        }
        queue.write_buffer(&draw.instances, 0, &draw.packed);
        draw.staged = u32::try_from(records.len()).map_err(|_| {
            ParticleError::Device("particle draw batch exceeds the u32 instance count".into())
        })?;
        Ok(draw.staged)
    }

    /// Draw the staged batch into `pass`: one instanced draw over the
    /// pipeline built from [`PARTICLE_DRAW_WGSL`], or nothing when no batch
    /// is staged, so a calm frame issues no draw call at all. The pass must
    /// carry one HDR colour target and no depth attachment, the shape the
    /// pipeline declares; the caller owns it and its target, so the quads
    /// land in the frame the blit reads. This reads device state only: no
    /// map call, no poll wait, no frame stall.
    pub fn encode_draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(draw) = self.draw.as_ref() else {
            return;
        };
        if draw.staged == 0 {
            return;
        }
        // The two seams the batch binds: the one-texel stand-in atlas and the
        // zeroed depth block the shader declares until the closing slice.
        debug_assert_eq!(draw.atlas.size().width, 1);
        debug_assert_eq!(draw.scene_depth.size(), PARTICLE_DRAW_SCENE_DEPTH_BYTES);
        pass.set_pipeline(&draw.pipeline);
        pass.set_bind_group(0, &draw.bind, &[]);
        pass.draw(0..4, 0..draw.staged);
    }
}

#[cfg(feature = "window")]
impl ParticleDrawState {
    /// Build the draw state once: the pipeline plus its layout, the view
    /// uniform, the max-size instance buffer (sized for the pool cap, so no
    /// frame resizes it), the one-texel white atlas stand-in and the unread
    /// scene-depth seam at binding 4. Those three buffers are what the create
    /// counter charges. Runtime sizes that cannot fit a `u64` are refused
    /// before any device work.
    fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: &[u8; PARTICLE_DRAW_UNIFORM_BYTES],
    ) -> Result<Self, ParticleError> {
        let layout = particle_draw_layout(device);
        let pipeline = particle_draw_pipeline(device, &layout);
        let uniform_bytes = u64::try_from(view.len())
            .map_err(|_| ParticleError::Invalid("particle draw uniform exceeds u64".into()))?;
        let uniform = particle_draw_buffer(
            device,
            "rp2-particle-draw-uniform",
            uniform_bytes,
            wgpu::BufferUsages::UNIFORM,
        );
        queue.write_buffer(&uniform, 0, view);
        let batch_bytes = PARTICLE_DRAW_MAX_INSTANCES.saturating_mul(PARTICLE_DRAW_INSTANCE_BYTES);
        let instances = particle_draw_buffer(
            device,
            "rp2-particle-draw-instances",
            u64::try_from(batch_bytes).map_err(|_| {
                ParticleError::Invalid("particle draw batch exceeds u64 bytes".into())
            })?,
            wgpu::BufferUsages::STORAGE,
        );
        // The shader declares this seam but reads a constant zero gap until
        // the closing slice feeds scene depth, so one zeroed block serves it.
        let scene_depth = particle_draw_buffer(
            device,
            "rp2-particle-draw-scene-depth",
            PARTICLE_DRAW_SCENE_DEPTH_BYTES,
            wgpu::BufferUsages::STORAGE,
        );
        queue.write_buffer(&scene_depth, 0, &[0u8; 64]);
        let (atlas, atlas_view) = particle_draw_atlas(device, queue);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("rp2-particle-draw-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rp2-particle-draw-bind"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: instances.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: scene_depth.as_entire_binding(),
                },
            ],
        });
        Ok(Self {
            pipeline,
            bind,
            uniform,
            instances,
            scene_depth,
            atlas,
            packed: Vec::with_capacity(PARTICLE_DRAW_MAX_INSTANCES * PARTICLE_DRAW_INSTANCE_BYTES),
            staged: 0,
        })
    }
}

/// Map a readback buffer and wait for it. One poll covers every pending map.
/// Shared [`crate::accel`] core with the `particle` error prefix kept.
#[cfg(feature = "window")]
fn particle_map_and_wait(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Result<(), String> {
    crate::accel::map_readback(device, buffer, "particle")
}

/// Read `count` little-endian u32 words from a mapped readback buffer.
/// Shared [`crate::accel`] core with the `particle` text kept.
#[cfg(feature = "window")]
fn particle_readback_u32(buffer: &wgpu::Buffer, count: usize) -> Result<Vec<u32>, String> {
    crate::accel::readback_u32(buffer, count, "particle", "words")
}

#[cfg(feature = "window")]
impl ParticlePool {
    /// Window submit over the live pool (RP2 live): bind the deterministic
    /// byte image (zero-padded to whole words) under the shared
    /// [`crate::gtao::LookFrame`] bind, run one bind-group probe pass over
    /// it and read it back exactly, proving the billboard bind validates
    /// with 0 errors and the device image matches the CPU sim word for
    /// word. Fails loudly (never silent, never the twin refusal) on bad
    /// frames or any device fault.
    pub fn submit_gpu(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &crate::gtao::LookFrame,
        submitter: &mut ParticleSubmitter,
    ) -> Result<ParticleGpuReceipt, ParticleError> {
        use std::time::Instant;
        self.validate()?;
        frame
            .validate()
            .map_err(|e| ParticleError::Invalid(format!("particle look frame: {e}")))?;
        let bytes = self.particle_bytes();
        let words = bytes.len().div_ceil(4) as u64;
        let count = u32::try_from(words)
            .map_err(|_| ParticleError::Invalid(format!("particle words {words} exceed u32")))?;
        if words == 0 {
            return Err(ParticleError::Invalid(
                "particle byte image is empty".into(),
            ));
        }
        let byte_len = words * 4;
        let started = Instant::now();

        if submitter.cached_words != Some(words) {
            let src_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rp2-particle-src"),
                size: byte_len,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let dst_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rp2-particle-dst"),
                size: byte_len,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rp2-particle-readback"),
                size: byte_len,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rp2-particle-bind"),
                layout: &submitter.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: submitter.look_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: src_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: dst_buf.as_entire_binding(),
                    },
                ],
            });
            submitter.src_buf = Some(src_buf);
            submitter.dst_buf = Some(dst_buf);
            submitter.readback = Some(readback);
            submitter.bind = Some(bind);
            submitter.cached_words = Some(words);
            submitter.created_buffers = submitter.created_buffers.saturating_add(3);
        }
        let (src_buf, dst_buf, readback, bind) = match (
            &submitter.src_buf,
            &submitter.dst_buf,
            &submitter.readback,
            &submitter.bind,
        ) {
            (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
            _ => {
                return Err(ParticleError::Device(
                    "particle submitter lost its sized buffers".into(),
                ));
            }
        };

        queue.write_buffer(
            &submitter.look_buf,
            0,
            &crate::graph::look_frame_uniform_bytes(frame),
        );
        let mut padded = vec![0u8; byte_len as usize];
        padded[..bytes.len()].copy_from_slice(&bytes);
        queue.write_buffer(src_buf, 0, &padded);

        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rp2-particle-frame"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("rp2-particle-bind-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&submitter.pipeline);
            pass.set_bind_group(0, bind, &[]);
            pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(dst_buf, 0, readback, 0, byte_len);
        queue.submit(Some(encoder.finish()));
        particle_map_and_wait(device, readback).map_err(ParticleError::Device)?;
        let back =
            particle_readback_u32(readback, words as usize).map_err(ParticleError::Device)?;
        readback.unmap();
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(ParticleError::Validation(format!(
                "particle bind validation: {err:?}"
            )));
        }
        for (i, chunk) in padded.chunks_exact(4).enumerate() {
            let want = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if back[i] != want {
                return Err(ParticleError::Device(format!(
                    "particle bind roundtrip word {i} differs: {} vs {want}",
                    back[i]
                )));
            }
        }
        Ok(ParticleGpuReceipt {
            ticks: self.ticks,
            alive: self.live.len(),
            spawned: self.spawned,
            validation_errors: 0,
            submit_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}

// ---------------------------------------------------------------------------
// Receipts: what ran, how many, how long, charged against the 1.0 ms budget.
// ---------------------------------------------------------------------------

/// One CPU-sim receipt: tick cursor plus live/spawn counts plus wall time.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ParticleReceipt {
    /// Pass name (`"particles_gpu"`).
    pub pass: &'static str,
    /// Ticks simulated when the receipt was taken.
    pub ticks: u64,
    /// Live particles at the cursor.
    pub alive: usize,
    /// Total spawns at the cursor.
    pub spawned: u64,
    /// Wall-clock sim time in ms; evidence only, never hashed.
    pub ms: f64,
    /// True once a spawn hit the pool cap.
    pub degraded: bool,
}

impl ParticleReceipt {
    /// Posted GPU budget this pass is charged against.
    pub fn budget_ms(&self) -> f64 {
        PARTICLE_BUDGET_MS
    }

    /// True while the sim stayed inside the posted budget.
    pub fn within_budget(&self) -> bool {
        self.ms <= PARTICLE_BUDGET_MS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settled(seed: u64, ticks: u32) -> ParticlePool {
        let mut pool = ParticlePool::new(seed);
        for _ in 0..ticks {
            pool.tick();
        }
        pool
    }

    #[test]
    fn soft_fade_one_at_zero_monotonic() {
        assert_eq!(soft_fade(0.0), 1.0, "zero gap reads fully visible");
        assert_eq!(
            soft_fade(SOFT_FADE_RANGE_M),
            0.0,
            "gap at the range reads fully faded"
        );
        assert_eq!(
            soft_fade(SOFT_FADE_RANGE_M + 1.0),
            0.0,
            "gap past the range stays faded"
        );
        assert_eq!(soft_fade(-0.5), 1.0, "negative gap clamps to visible");
        assert_eq!(soft_fade(f32::NAN), 0.0, "NaN gap fails closed");
        assert_eq!(soft_fade(f32::INFINITY), 0.0, "infinite gap fails closed");
        let gaps = [0.0, 0.05, 0.1, 0.15, 0.2, 0.25, SOFT_FADE_RANGE_M, 1.0];
        let mut prev = 2.0;
        for gap in gaps {
            let v = soft_fade(gap);
            assert!(v.is_finite(), "fade {v} at gap {gap} must stay finite");
            assert!(
                (0.0..=1.0).contains(&v),
                "fade {v} at gap {gap} escapes 0..=1"
            );
            assert!(v <= prev, "fade must not rise: {v} after {prev}");
            prev = v;
        }
        let mid = soft_fade(0.1);
        assert!(
            mid > 0.0 && mid < 1.0,
            "mid gap must partly fade, got {mid}"
        );
    }

    #[test]
    fn flipbook_weights_sum_one() {
        let exact = flipbook_pair(5.3, 16);
        assert_eq!((exact.cell_a, exact.cell_b), (5, 6));
        assert!((exact.weight_a - 0.7).abs() < 1e-6);
        assert!((exact.weight_b - 0.3).abs() < 1e-6);
        for cells in [1u32, 2, 8, 16] {
            for frame_f in [-1.0, 0.0, 0.3, 5.3, 15.0, 15.7, 100.0, f32::NAN] {
                let p = flipbook_pair(frame_f, cells);
                assert!(
                    p.cell_a < cells && p.cell_b < cells,
                    "cells {}:{} escape 0..{cells}",
                    p.cell_a,
                    p.cell_b
                );
                assert!(
                    p.weight_a.is_finite() && p.weight_b.is_finite(),
                    "weights must stay finite"
                );
                assert!(
                    (0.0..=1.0).contains(&p.weight_a) && (0.0..=1.0).contains(&p.weight_b),
                    "weights {}:{} escape 0..=1",
                    p.weight_a,
                    p.weight_b
                );
                let sum = p.weight_a + p.weight_b;
                assert!((sum - 1.0).abs() < 1e-6, "weights must sum to 1, got {sum}");
            }
        }
        let empty = flipbook_pair(3.0, 0);
        assert_eq!(
            (empty.cell_a, empty.cell_b, empty.weight_a, empty.weight_b),
            (0, 0, 1.0, 0.0)
        );
        let last = flipbook_pair(15.7, 16);
        assert_eq!((last.cell_a, last.cell_b), (15, 15));
        assert_eq!((last.weight_a, last.weight_b), (1.0, 0.0));
    }

    #[test]
    fn kenney_flipbook_blend_weights() {
        // Spike fixture: demo seed 2040 stepped 120 ticks at 60 Hz plus a
        // flipbook_pair sweep over a 16-frame atlas (cols 4, rows 4).
        let mut a = ParticlePool::new(2040);
        let mut b = ParticlePool::new(2040);
        for _ in 0..120 {
            a.tick();
            b.tick();
        }
        assert_eq!(a.state_hash(), b.state_hash(), "hash x2 identical, diff 0");
        assert_eq!(a.particle_bytes(), b.particle_bytes(), "bytes x2 identical");
        assert_eq!(a.alive(), b.alive(), "alive x2 identical");
        assert!(a.alive() > 0, "the sim must actually run");
        assert_eq!(POOL_CAP, 4096, "pool bound stays exact");
        assert_eq!(EMITTER_COUNT, 3, "emitter count stays exact");
        assert_eq!(PARTICLE_BUDGET_MS, 1.0, "particle bar stays 1.0 ms");
        for age01 in [
            0.0, 0.0625, 0.125, 0.1875, 0.25, 0.3125, 0.375, 0.4375, 0.5, 0.5625, 0.625, 0.6875,
            0.75, 0.8125, 0.875, 0.9375, 1.0,
        ] {
            let pair = flipbook_pair(age01 * 16.0, 16);
            assert!(
                pair.cell_a < 16 && pair.cell_b < 16,
                "cells {}:{} escape the 16-cell atlas",
                pair.cell_a,
                pair.cell_b
            );
            assert!(
                pair.weight_a.is_finite() && pair.weight_b.is_finite(),
                "weights must stay finite"
            );
            let sum = pair.weight_a + pair.weight_b;
            assert!(
                (sum - 1.0).abs() < 1e-6,
                "weight pair must sum to 1.0, got {sum}"
            );
        }
        for needle in ["draw_vs", "draw_fs", "draw_flipbook_uv", "stretch", "blend"] {
            assert!(
                PARTICLE_DRAW_WGSL.contains(needle),
                "wired particle.wgsl must stage {needle}"
            );
        }
        let at = ParticleReceipt {
            pass: "particles_gpu",
            ticks: 120,
            alive: a.alive(),
            spawned: a.spawned_total(),
            ms: PARTICLE_BUDGET_MS,
            degraded: false,
        };
        assert!(at.within_budget(), "exactly at budget is within");
    }

    #[test]
    fn gpu_draw_changes_pixels() {
        // Row accept: the torch-region mask differs on vs off with 4096
        // live. Two pools prove x2 identity; the fade-off feed reads fully
        // visible while the fade-on feed over a past-range gap reads fully
        // faded, so the staged picture changes with the pass.
        let mut a = ParticlePool::new(2040);
        let mut b = ParticlePool::new(2040);
        for kind in [EmitterKind::Dust, EmitterKind::Flame, EmitterKind::Sparks] {
            a.set_rate(kind, MAX_RATE_PER_S)
                .expect("MAX_RATE_PER_S passes set_rate validation");
            b.set_rate(kind, MAX_RATE_PER_S)
                .expect("MAX_RATE_PER_S passes set_rate validation");
        }
        for _ in 0..200 {
            a.tick();
            b.tick();
        }
        assert_eq!(POOL_CAP, 4096, "pool bound stays exact");
        assert!(a.capped(), "the saturated frame must hit the cap");
        assert!(b.capped(), "the saturated frame must hit the cap");
        assert!(a.dropped_total() > 0, "saturation must refuse spawns");
        assert_eq!(a.alive(), b.alive(), "alive x2 identical");
        let live = a.alive();
        assert!(live >= 4000, "the frame must stage ~4096 live, got {live}");
        assert_eq!(a.state_hash(), b.state_hash(), "hash x2 identical, diff 0");
        let mut off = Vec::new();
        let mut on = Vec::new();
        let n_off = a.draw_records(false, |_| SOFT_FADE_RANGE_M, &mut off);
        let n_on = a.draw_records(true, |_| SOFT_FADE_RANGE_M, &mut on);
        assert_eq!((n_off, n_on), (live, live));
        assert!(
            off.iter().all(|r| r.fade == 1.0),
            "fade off reads fully visible"
        );
        assert!(
            on.iter().all(|r| r.fade == 0.0),
            "past-range gap reads fully faded"
        );
        let mut per_kind = [0usize; EMITTER_COUNT];
        let mut prev_kind = 0u8;
        for r in on.iter() {
            assert!(
                r.fade.is_finite() && r.blend_a.is_finite() && r.blend_b.is_finite(),
                "feed must stay finite"
            );
            let sum = r.blend_a + r.blend_b;
            assert!((sum - 1.0).abs() < 1e-6, "weights must sum to 1, got {sum}");
            assert!(r.kind >= prev_kind, "feed stays in fixed emitter order");
            prev_kind = r.kind;
            per_kind[usize::from(r.kind)] += 1;
        }
        assert!(
            per_kind.iter().all(|c| *c > 0),
            "every emitter stages, got {per_kind:?}"
        );
        let mut again = Vec::new();
        let n_again = b.draw_records(true, |_| SOFT_FADE_RANGE_M, &mut again);
        assert_eq!(n_again, live);
        assert_eq!(on, again, "feed x2 identical");
        let mut zero_gap = Vec::new();
        a.draw_records(true, |_| 0.0, &mut zero_gap);
        assert!(
            zero_gap.iter().all(|r| r.fade == 1.0),
            "zero gap reads fully visible"
        );
        let mut nan_gap = Vec::new();
        a.draw_records(true, |_| f32::NAN, &mut nan_gap);
        assert!(
            nan_gap.iter().all(|r| r.fade == 0.0),
            "NaN gap fails closed"
        );
    }

    #[test]
    #[ignore = "timing: wall clock; run on the --perf leg, never by default"]
    fn gpu_draw_stage_within_budget() {
        // Row accept: the 4096-live stage stays inside PARTICLE_BUDGET_MS.
        let mut pool = ParticlePool::new(2040);
        for kind in [EmitterKind::Dust, EmitterKind::Flame, EmitterKind::Sparks] {
            pool.set_rate(kind, MAX_RATE_PER_S)
                .expect("MAX_RATE_PER_S passes set_rate validation");
        }
        for _ in 0..200 {
            pool.tick();
        }
        assert!(pool.capped(), "the saturated frame must hit the cap");
        assert!(pool.dropped_total() > 0, "saturation must refuse spawns");
        let live = pool.alive();
        assert!(live >= 4000, "the frame must stage ~4096 live, got {live}");
        let mut scratch = Vec::with_capacity(live);
        // Warmup plus best-of-7: wall clock on the shared box jitters under
        // parallel load, so the budget verdict reads the fastest clean
        // sample, the stage's true cost, never a preempted one.
        pool.draw_records(true, |p| p.pos[1], &mut scratch);
        let mut best_ms = f64::INFINITY;
        for _ in 0..7 {
            scratch.clear();
            let start = Instant::now();
            let n = pool.draw_records(true, |p| p.pos[1], &mut scratch);
            assert_eq!(n, live);
            best_ms = best_ms.min(start.elapsed().as_secs_f64() * 1000.0);
        }
        assert!(
            best_ms <= PARTICLE_BUDGET_MS,
            "stage {best_ms} ms exceeds 1.0 ms"
        );
    }

    #[test]
    fn stretch_grows_with_speed_finite() {
        let still = stretch_len_dir([0.0, 0.0, 0.0]);
        assert_eq!(still.len, 1.0);
        assert_eq!(still.dir, [0.0, 1.0, 0.0]);
        let bad = stretch_len_dir([f32::NAN, 0.0, 0.0]);
        assert_eq!(bad.len, 1.0);
        assert_eq!(bad.dir, [0.0, 1.0, 0.0]);
        let mut prev = 0.0;
        for speed in [0.5, 2.0, 8.0] {
            let s = stretch_len_dir([speed, 0.0, 0.0]);
            assert!(s.len.is_finite(), "stretch must stay finite");
            assert!(
                (1.0..=STRETCH_MAX).contains(&s.len),
                "stretch {} escapes 1..={STRETCH_MAX}",
                s.len
            );
            assert!(s.dir.iter().all(|c| c.is_finite()));
            let norm = s.dir[0] * s.dir[0] + s.dir[1] * s.dir[1] + s.dir[2] * s.dir[2];
            assert!(
                (norm - 1.0).abs() < 1e-5,
                "direction must stay unit, got norm {norm}"
            );
            assert!(s.len > prev, "stretch must grow with speed");
            prev = s.len;
        }
        let fast = stretch_len_dir([100.0, 0.0, 0.0]);
        assert_eq!(fast.len, STRETCH_MAX, "huge speeds clamp at the max");
    }

    #[test]
    fn draw_wgsl_stages_expand_fade_blend_stretch() {
        for needle in [
            "draw_vs",
            "draw_fs",
            "draw_soft_fade",
            "draw_flipbook_uv",
            "stretch",
            "blend",
            "fade",
        ] {
            assert!(
                PARTICLE_DRAW_WGSL.contains(needle),
                "draw WGSL must stage {needle}"
            );
        }
    }

    #[test]
    fn throttle_carries_fractions_across_ticks() {
        let mut accrue = 0.0;
        let first = throttle_spawn(&mut accrue, 90.0, TICK_DT);
        let second = throttle_spawn(&mut accrue, 90.0, TICK_DT);
        assert_eq!(
            (first, second),
            (1, 2),
            "90/s at 60 Hz accrues 1.5 per tick: 1 then 2"
        );
        assert!(
            (0.0..1.0).contains(&accrue),
            "whole parts leave only a fraction, got {accrue}"
        );
    }

    #[test]
    fn throttle_caps_and_keeps_excess() {
        let mut accrue = 0.0;
        let n = throttle_spawn(&mut accrue, 1.0e6, MAX_DT_CLAMP);
        assert_eq!(n, MAX_SPAWN_PER_TICK, "one tick never bursts past the cap");
        assert!(
            accrue > MAX_SPAWN_PER_TICK as f32,
            "capped excess stays accrued, got {accrue}"
        );
    }

    #[test]
    fn throttle_clamps_negative_rate() {
        let mut accrue = 1.5;
        let n = throttle_spawn(&mut accrue, -10.0, TICK_DT);
        assert_eq!(
            n, 1,
            "negative rates add nothing, the stored whole part fires"
        );
        assert!(
            (accrue - 0.5).abs() < 1e-6,
            "stored fraction survives a refused rate, got {accrue}"
        );
    }

    #[test]
    fn seed_x2_identical_bytes_every_tick() {
        let mut a = ParticlePool::new(2040);
        let mut b = ParticlePool::new(2040);
        for _ in 0..120 {
            a.tick();
            b.tick();
            assert_eq!(a.particle_bytes(), b.particle_bytes());
        }
        assert_eq!(a.state_hash(), b.state_hash());
        assert!(a.alive() > 0, "the sim must actually run");
        assert!(a.time_s() > 0.0, "sim time must advance");
    }

    #[test]
    fn hash_moves_with_time() {
        let early = settled(2040, 1);
        let late = settled(2040, 120);
        assert_ne!(
            early.state_hash(),
            late.state_hash(),
            "a frozen hash would hide a dead sim"
        );
    }

    #[test]
    fn consecutive_seeds_diverge() {
        let a = settled(11, 5);
        let b = settled(12, 5);
        assert_ne!(
            a.particle_bytes(),
            b.particle_bytes(),
            "collapsed streams would replay one seed twice"
        );
    }

    #[test]
    fn first_tick_burst_has_no_deaths() {
        let mut pool = ParticlePool::new(5);
        pool.tick();
        assert!(pool.alive() > 0, "burst plus rate must spawn");
        assert_eq!(
            pool.spawned_total(),
            pool.alive() as u64,
            "min life 0.4 s outlives one tick"
        );
        assert_eq!(pool.tick_count(), 1);
        assert_eq!(pool.time_s(), TICK_DT);
    }

    #[test]
    fn pool_cap_never_exceeded() {
        let mut pool = ParticlePool::with_capacity(9, 64).expect("bounded pool builds");
        for _ in 0..300 {
            pool.tick();
        }
        assert!(pool.alive() <= 64, "live set must respect the cap");
        assert!(pool.capped(), "a 64-slot pool under full rate must cap");
        assert!(pool.dropped_total() > 0, "refused spawns must be counted");
    }

    #[test]
    fn steady_state_capacity_stable() {
        let mut pool = ParticlePool::with_capacity(3, 256).expect("bounded pool builds");
        for _ in 0..200 {
            pool.tick();
        }
        let before = pool.slot_capacity();
        for _ in 0..120 {
            pool.tick();
        }
        assert_eq!(
            pool.slot_capacity(),
            before,
            "stepping must not grow the live buffer"
        );
    }

    #[test]
    fn particles_finite_frames_in_range() {
        let pool = settled(7, 300);
        assert!(!pool.particles().is_empty(), "pool must hold particles");
        for p in pool.particles() {
            assert!(
                p.pos.iter().all(|c| c.is_finite())
                    && p.vel.iter().all(|c| c.is_finite())
                    && p.size_m.is_finite()
                    && p.alpha.is_finite(),
                "NaN would poison the instance upload"
            );
            assert!(p.size_m > 0.0, "degenerate quads would vanish");
            assert!(
                (0.0..=1.0).contains(&p.alpha),
                "alpha {} escapes the blend range",
                p.alpha
            );
            let kind = EmitterKind::from_tag(p.kind);
            let (cols, rows) = kind.grid();
            let cells = cols as usize * rows as usize;
            assert!(
                (p.frame as usize) < cells,
                "frame {} escapes the {cells}-cell atlas",
                p.frame
            );
        }
    }

    #[test]
    #[cfg(not(feature = "window"))]
    fn gpu_submit_without_device_names_the_window_build() {
        let pool = settled(21, 10);
        match pool.submit_gpu([1280, 720]) {
            Err(ParticleError::NoDevice(_)) => {}
            other => panic!("good viewport must hit the twin refusal, got {other:?}"),
        }
    }

    #[test]
    #[cfg(not(feature = "window"))]
    fn gpu_submit_rejects_bad_viewport() {
        let pool = settled(21, 2);
        match pool.submit_gpu([0, 720]) {
            Err(ParticleError::Invalid(_)) => {}
            other => panic!("zero viewport must be Invalid, got {other:?}"),
        }
        match pool.submit_gpu([4096, 720]) {
            Err(ParticleError::Invalid(_)) => {}
            other => panic!("oversize viewport must be Invalid, got {other:?}"),
        }
    }
    #[test]
    fn with_capacity_bounds() {
        match ParticlePool::with_capacity(1, 0) {
            Err(ParticleError::Invalid(_)) => {}
            other => panic!("zero cap must be Invalid, got {}", other.is_ok()),
        }
        match ParticlePool::with_capacity(1, POOL_CAP_HARD + 1) {
            Err(ParticleError::BudgetExceeded { requested, allowed }) => {
                assert_eq!(
                    (requested, allowed),
                    (POOL_CAP_HARD as u32 + 1, POOL_CAP_HARD as u32)
                );
            }
            other => panic!("over-hard cap must refuse, got {}", other.is_ok()),
        }
        assert!(
            ParticlePool::with_capacity(1, POOL_CAP_HARD).is_ok(),
            "hard cap itself must build"
        );
    }

    #[test]
    fn set_rate_bounds() {
        let mut pool = ParticlePool::new(4);
        match pool.set_rate(EmitterKind::Dust, f32::NAN) {
            Err(ParticleError::Invalid(_)) => {}
            other => panic!("NaN rate must be Invalid, got {other:?}"),
        }
        match pool.set_rate(EmitterKind::Dust, -1.0) {
            Err(ParticleError::Invalid(_)) => {}
            other => panic!("negative rate must be Invalid, got {other:?}"),
        }
        match pool.set_rate(EmitterKind::Dust, MAX_RATE_PER_S + 1.0) {
            Err(ParticleError::BudgetExceeded { .. }) => {}
            other => panic!("over-max rate must refuse, got {other:?}"),
        }
        pool.set_rate(EmitterKind::Dust, 10.0)
            .expect("in-budget rate applies");
        assert_eq!(pool.emitters()[0].rate_per_s, 10.0);
    }

    #[test]
    fn nonpositive_or_nan_dt_ignored() {
        let mut pool = settled(8, 5);
        let (ticks, time) = (pool.tick_count(), pool.time_s());
        pool.step(f32::NAN);
        pool.step(0.0);
        pool.step(-1.0);
        assert_eq!((pool.tick_count(), pool.time_s()), (ticks, time));
    }

    #[test]
    fn emitter_order_dust_flame_sparks() {
        let pool = settled(15, 60);
        let kinds: Vec<EmitterKind> = pool.emitters().iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![EmitterKind::Dust, EmitterKind::Flame, EmitterKind::Sparks]
        );
        for want in kinds {
            assert!(
                pool.particles()
                    .iter()
                    .any(|p| EmitterKind::from_tag(p.kind) == want),
                "fixed order must keep every kind alive at 60 ticks"
            );
        }
    }

    #[test]
    fn receipt_carries_budget() {
        // Exactness default (no wall clock): synthetic receipts pin the
        // 1.0 ms flag both ways; the measured sim ms is evidence only here.
        // Boosted-clock enforcement lives in particle_sim_wall_budget
        // (--perf only, load-dependent).
        let mut pool = ParticlePool::new(30);
        let receipt = pool.step_timed(TICK_DT);
        assert_eq!(receipt.budget_ms(), PARTICLE_BUDGET_MS);
        assert_eq!(receipt.pass, "particles_gpu");
        assert!(receipt.ms >= 0.0, "timing must be a real duration");
        assert_eq!(receipt.budget_ms(), 1.0, "particle bar stays 1.0 ms");
        let at = ParticleReceipt {
            pass: "particles_gpu",
            ticks: 1,
            alive: 1,
            spawned: 1,
            ms: PARTICLE_BUDGET_MS,
            degraded: false,
        };
        assert!(at.within_budget(), "exactly at budget is within");
        let over = ParticleReceipt {
            ms: PARTICLE_BUDGET_MS + 0.5,
            ..at
        };
        assert!(!over.within_budget(), "over budget fails loudly");
    }

    #[test]
    #[ignore = "timing: particle sim wall-clock needs release --perf on boosted clocks (fails under parallel load)"]
    fn particle_sim_wall_budget() {
        // Seeded budget regression: the same settled sim that the default
        // test proves exact must clear the 1.0 ms bar on boosted release
        // clocks. Perf harness line keeps --perf medians readable.
        let mut pool = ParticlePool::new(30);
        let receipt = pool.step_timed(TICK_DT);
        eprintln!(
            "particle_sim_wall_budget: measured {:.3} ms vs {:.1} ms budget",
            receipt.ms,
            receipt.budget_ms()
        );
        assert!(
            receipt.within_budget(),
            "seeded sim over budget: {:.3} ms vs {:.1} ms",
            receipt.ms,
            receipt.budget_ms()
        );
    }

    // ----- Window device tests (behind `window`; skip loudly without one) -----

    /// Device shared with the GX1 suite: one device for the run, so parallel
    /// tests never exhaust adapters. `None` is skip-not-green, never a pass.
    #[cfg(feature = "window")]
    fn particle_device() -> Option<(
        &'static wgpu::Device,
        &'static wgpu::Queue,
        &'static wgpu::AdapterInfo,
    )> {
        crate::graph::gx1_test_device()
    }

    #[cfg(feature = "window")]
    fn particle_frame() -> crate::gtao::LookFrame {
        crate::gtao::LookFrame {
            frame_index: 11,
            viewport_px: [64, 64],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        }
    }

    #[test]
    #[cfg(feature = "window")]
    fn gpu_pool_binds_exact_with_look_frame() {
        let Some((device, queue, info)) = particle_device() else {
            eprintln!("SKIPPED particle bind: no wgpu adapter on this machine");
            return;
        };
        let mut pool = ParticlePool::new(2040);
        for _ in 0..60 {
            pool.tick();
        }
        assert!(pool.alive() > 0, "the sim must actually run");
        let frame = particle_frame();
        let mut submitter = ParticleSubmitter::new(device);
        assert_eq!(submitter.created_buffers(), 0);
        let receipt = pool
            .submit_gpu(device, queue, &frame, &mut submitter)
            .expect("live pool binds clean");
        assert!(receipt.is_clean(), "0 validation errors");
        assert_eq!(
            (receipt.ticks, receipt.alive, receipt.spawned),
            (pool.tick_count(), pool.alive(), pool.spawned_total())
        );
        assert_eq!(receipt.budget_ms(), PARTICLE_BUDGET_MS);
        // The pool sims on between submits, so the word shape moves: a
        // same-tick resubmit reuses the sized buffers exactly.
        let before = submitter.created_buffers();
        let again = pool
            .submit_gpu(device, queue, &frame, &mut submitter)
            .expect("same-tick bind reuses");
        assert!(again.is_clean());
        assert_eq!(
            submitter.created_buffers(),
            before,
            "matching-shape submits size nothing"
        );
        eprintln!(
            "RP2 particle bind on {}: alive={} words={} submit_ms={:.3} resubmit_ms={:.3} (budget {PARTICLE_BUDGET_MS} ms)",
            info.name,
            receipt.alive,
            pool.particle_bytes().len().div_ceil(4),
            receipt.submit_ms,
            again.submit_ms
        );
    }

    #[test]
    #[cfg(feature = "window")]
    fn gpu_empty_pool_still_binds() {
        let Some((device, queue, _)) = particle_device() else {
            eprintln!("SKIPPED particle empty bind: no wgpu adapter on this machine");
            return;
        };
        // Zero ticks: the 12-byte header image still records a real pass.
        let pool = ParticlePool::new(7);
        assert_eq!(pool.alive(), 0);
        let mut submitter = ParticleSubmitter::new(device);
        let receipt = pool
            .submit_gpu(device, queue, &particle_frame(), &mut submitter)
            .expect("empty pool binds clean");
        assert!(receipt.is_clean());
        assert_eq!((receipt.ticks, receipt.alive), (0, 0));
    }

    #[test]
    #[cfg(feature = "window")]
    fn gpu_refuses_bad_frame_before_sizing() {
        let Some((device, queue, _)) = particle_device() else {
            eprintln!("SKIPPED particle refusals: no wgpu adapter on this machine");
            return;
        };
        let pool = settled(21, 10);
        let frame = particle_frame();
        let mut submitter = ParticleSubmitter::new(device);
        assert_eq!(submitter.created_buffers(), 0);
        let bad = crate::gtao::LookFrame {
            exposure_ev100: f32::NAN,
            ..frame
        };
        assert!(matches!(
            pool.submit_gpu(device, queue, &bad, &mut submitter),
            Err(ParticleError::Invalid(_))
        ));
        assert_eq!(submitter.created_buffers(), 0, "refusals size no buffers");
    }

    // ----- RL52 staged draw side (behind `window`) -----

    /// Saturated pool the draw tests share: every emitter at its rate cap, so
    /// a frame stages the pool cap exactly.
    #[cfg(feature = "window")]
    fn saturated_pool() -> ParticlePool {
        let mut pool = ParticlePool::new(2040);
        for kind in [EmitterKind::Dust, EmitterKind::Flame, EmitterKind::Sparks] {
            pool.set_rate(kind, MAX_RATE_PER_S)
                .expect("MAX_RATE_PER_S passes set_rate validation");
        }
        for _ in 0..200 {
            pool.tick();
        }
        pool
    }

    #[test]
    #[cfg(feature = "window")]
    fn particles_draw_stages_one_instance_per_live_particle() {
        // Behavior: the staged feed draws one instance per live particle. A
        // saturated frame stages alive() records and `draw_staged` reports
        // that count; an empty batch reports 0, sizes no buffer and is never
        // an error; a same-shape batch creates nothing.
        let Some((device, queue, _)) = particle_device() else {
            eprintln!("SKIPPED particle draw staging: no wgpu adapter on this machine");
            return;
        };
        let pool = saturated_pool();
        let live = pool.alive();
        assert!(live >= 4000, "the frame must stage ~4096 live, got {live}");
        let mut records = Vec::new();
        let staged = pool.draw_records(false, |_| 0.0, &mut records);
        assert_eq!(staged, live, "one record per live particle");
        let mut submitter = ParticleSubmitter::new(device);
        let before = submitter.created_buffers();
        let calm = submitter
            .draw_staged(device, queue, &[])
            .expect("an empty batch is a calm frame, never an error");
        assert_eq!(calm, 0, "an empty batch reports no instance");
        assert_eq!(
            submitter.created_buffers(),
            before,
            "an empty batch sizes no buffer"
        );
        let count = submitter
            .draw_staged(device, queue, &records)
            .expect("the saturated batch uploads");
        assert_eq!(
            count,
            u32::try_from(live).expect("the pool cap fits u32"),
            "one instance per live particle"
        );
        let built = submitter.created_buffers();
        assert_eq!(built, before + 3, "the first batch builds the draw buffers");
        let again = submitter
            .draw_staged(device, queue, &records)
            .expect("a same-shape batch reuses");
        assert_eq!(again, count, "the same batch stages the same count");
        assert_eq!(
            submitter.created_buffers(),
            built,
            "a same-shape batch creates no buffers or bind groups"
        );
        eprintln!(
            "RL52 particle draw stage: instances={count} records={} alive={live}",
            records.len()
        );
    }

    #[test]
    #[cfg(feature = "window")]
    fn particles_draw_pipeline_matches_the_staged_record() {
        // Behavior: the draw pipeline is the staged-record draw. It is built
        // from PARTICLE_DRAW_WGSL (the same needles the CPU test pins), the
        // draw side builds one render pipeline and never maps or waits on
        // the device, and one encoded pass over a staged batch validates
        // clean against the HDR target it declares.
        let Some((device, queue, _)) = particle_device() else {
            eprintln!("SKIPPED particle draw pipeline: no wgpu adapter on this machine");
            return;
        };
        for needle in [
            "draw_vs",
            "draw_fs",
            "draw_flipbook_uv",
            "stretch",
            "blend",
            "fade",
        ] {
            assert!(
                PARTICLE_DRAW_WGSL.contains(needle),
                "draw WGSL must stage {needle}"
            );
        }
        let builders = include_str!("particle_draw_gpu.rs");
        let builders_at = builders
            .find("fn particle_draw_pipeline")
            .expect("the draw builders exist");
        let src = include_str!("particles_gpu.rs");
        let submitter_at = src
            .find("pub struct ParticleSubmitter")
            .expect("the submitter exists");
        let end = src[submitter_at..]
            .find("fn particle_map_and_wait")
            .map(|at| submitter_at + at)
            .expect("the draw side ends before the bind map helper");
        let draw_side = format!("{}{}", &builders[builders_at..], &src[submitter_at..end]);
        assert!(
            draw_side.contains("PARTICLE_DRAW_WGSL"),
            "the pipeline is built from the staged draw shader"
        );
        assert!(
            draw_side.contains("create_render_pipeline"),
            "the draw side builds one render pipeline"
        );
        assert!(
            !draw_side.contains("map_async"),
            "the draw path never maps a buffer"
        );
        assert!(
            !draw_side.contains(".poll("),
            "the draw path never waits on the device"
        );

        let mut pool = ParticlePool::new(2040);
        for _ in 0..30 {
            pool.tick();
        }
        let mut records = Vec::new();
        let staged = pool.draw_records(false, |_| 0.0, &mut records);
        assert!(staged > 0, "the pool must stage live particles");
        let mut submitter = ParticleSubmitter::new(device);
        assert_eq!(
            submitter
                .draw_staged(device, queue, &records)
                .expect("the batch uploads"),
            u32::try_from(staged).expect("the staged count fits u32")
        );
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let hdr = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rl52-particle-draw-target"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::present::HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = hdr.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rl52-particle-draw-frame"),
        });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rl52-particle-draw-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            submitter.encode_draw(&mut pass);
        }
        queue.submit(Some(enc.finish()));
        assert!(
            pollster::block_on(scope.pop()).is_none(),
            "the staged draw pass must validate clean"
        );
    }

    #[test]
    #[cfg(feature = "window")]
    fn particles_draw_batch_is_deterministic_twice() {
        // Behavior: the batch is a pure function of the pool. Two pools on
        // one seed stage identical records, identical packed instance bytes
        // and identical instance counts, so the draw replays byte for byte.
        let Some((device, queue, _)) = particle_device() else {
            eprintln!("SKIPPED particle draw determinism: no wgpu adapter on this machine");
            return;
        };
        let mut a = ParticlePool::new(2040);
        let mut b = ParticlePool::new(2040);
        for _ in 0..120 {
            a.tick();
            b.tick();
        }
        let mut ra = Vec::new();
        let mut rb = Vec::new();
        let na = a.draw_records(false, |_| 0.0, &mut ra);
        let nb = b.draw_records(false, |_| 0.0, &mut rb);
        assert!(na > 0, "the sim must actually stage");
        assert_eq!(na, nb, "two pools on one seed stage the same count");
        assert_eq!(ra, rb, "two pools on one seed stage the same records");
        let mut pa = vec![0u8; na * PARTICLE_DRAW_INSTANCE_BYTES];
        let mut pb = vec![0u8; nb * PARTICLE_DRAW_INSTANCE_BYTES];
        for (slot, record) in ra.iter().enumerate() {
            let at = slot * PARTICLE_DRAW_INSTANCE_BYTES;
            let dst = &mut pa[at..at + PARTICLE_DRAW_INSTANCE_BYTES];
            particle_pack_instance(record, dst);
        }
        for (slot, record) in rb.iter().enumerate() {
            let at = slot * PARTICLE_DRAW_INSTANCE_BYTES;
            let dst = &mut pb[at..at + PARTICLE_DRAW_INSTANCE_BYTES];
            particle_pack_instance(record, dst);
        }
        assert_eq!(pa, pb, "the packed instance bytes are identical twice");
        let mut first = ParticleSubmitter::new(device);
        let mut second = ParticleSubmitter::new(device);
        let ca = first
            .draw_staged(device, queue, &ra)
            .expect("the first batch uploads");
        let cb = second
            .draw_staged(device, queue, &rb)
            .expect("the second batch uploads");
        assert_eq!(
            (ca, cb),
            (
                u32::try_from(na).expect("the staged count fits u32"),
                u32::try_from(nb).expect("the staged count fits u32")
            ),
            "the same pool twice reports the same instance count"
        );
    }
}
