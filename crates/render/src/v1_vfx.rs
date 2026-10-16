//! v1 VFX (R5): dust and torch particles plus blood decals for the arena.
//!
//! Builds on the R1 v1 mesh path (`upload_mesh`) and runs behind the `v1`
//! feature so the headless default stays std-only. Consumers: GM1 (arena
//! feel: dust kicks, torch flames) and CB2 (gore events: blood on sand).
//! Nothing here enters any simulation hash; pixels never do either.
//!
//! * [`Particles`]: CPU-simulated fixed-dt pool. `spawn` takes explicit
//!   position/velocity/life so the sim core holds no RNG and no wall clock;
//!   the pool is capped and a full pool reuses the oldest slot (insertion
//!   order), counting each reuse. Torch fire reuses this core with its own
//!   constants via [`Particles::spawn_torch_flame`].
//! * [`DecalSet`]: bounded blood splats on the sand, hashed into XZ cells
//!   for local query, faded by age in ticks.
//! * GPU upload: both kinds upload as instanced quads. [`quad_mesh`] builds
//!   the unit quad through the shared v1 `upload_mesh` validation; the
//!   per-instance byte images pack world pos + size + sRGB-ish color + alpha
//!   for the `v1_gpu` billboard pass (billboarding itself lands with GM1).
//!
//! Units: meters, seconds, ticks at 60 Hz. Determinism scope: fixed
//! [`FIXED_DT`], slot insertion order, integer sequence numbers; math uses
//! only `+ - * /` and one `sqrt` for normal normalization. No `HashMap`
//! iteration, no wall clock, no `Instant` anywhere on these paths.

use std::fmt;

use crate::decals::BLOOD_COLOR;
pub use crate::decals::MAX_DECAL_RADIUS_M as MAX_DECAL_RADIUS;
pub use crate::decals::{BLOOD_FADE_TICKS, MAX_DECALS};
use crate::particles::FIXED_DT;
use crate::v1::{upload_mesh, MeshData, MeshError};

// ---------------------------------------------------------------------------
// Particles: dust and torch sparks in one capped oldest-reuse pool.
// ---------------------------------------------------------------------------

/// Default pool cap: worst-case instance bytes stay small
/// (`1024 * 32 B = 32 KiB` per frame upload).
pub const MAX_PARTICLES: usize = 1024;
/// Hard bound for [`Particles::with_capacity`]; keeps custom pools bounded.
pub const MAX_PARTICLE_CAP: usize = 4096;
/// Longest accepted particle life in seconds; longer asks are rejected.
pub const MAX_PARTICLE_LIFE: f32 = 30.0;
/// Dust settles slowly: downward acceleration in m/s^2.
pub const DUST_GRAVITY: f32 = 0.6;
/// Dust drag (wide, slow motes lose speed fast).
pub const DUST_DRAG: f32 = 1.2;
/// Dust base quad size in meters.
pub const DUST_SIZE: f32 = 0.12;
/// Torch sparks rise: upward acceleration in m/s^2.
pub const TORCH_BUOYANCY: f32 = 2.5;
/// Torch spark drag (hot, fast, keeps speed).
pub const TORCH_DRAG: f32 = 0.4;
/// Torch spark base quad size in meters.
pub const TORCH_SIZE: f32 = 0.06;
/// Sparks per [`Particles::spawn_torch_flame`] call (one flame puff).
pub const TORCH_FLAME_COUNT: u32 = 4;
/// Upward launch speed of a flame puff in m/s.
pub const TORCH_RISE: f32 = 1.6;
/// Horizontal spread of a flame puff in m/s.
pub const TORCH_SPREAD: f32 = 0.5;
/// Flame spark life in seconds.
pub const TORCH_LIFE: f32 = 0.7;
/// Display color of dust (warm sand kicked up).
pub const DUST_COLOR: [f32; 3] = [0.75, 0.68, 0.55];
/// Display color of torch sparks (hot orange).
pub const SPARK_COLOR: [f32; 3] = [1.0, 0.55, 0.15];

/// Which visual a particle carries; also selects its integration constants.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParticleKind {
    /// Kicked-up sand: settles slowly, high drag, grows as it fades.
    Dust,
    /// Torch spark: rises fast, low drag, shrinks as it fades.
    TorchSpark,
}

/// One live particle: explicit state, insertion sequence for oldest-reuse.
#[derive(Clone, Copy, Debug)]
struct Slot {
    pos: [f32; 3],
    vel: [f32; 3],
    life: f32,
    age: f32,
    kind: ParticleKind,
    seq: u64,
}

/// CPU particle pool: fixed-dt, deterministic, capped with oldest-reuse.
///
/// The pool never allocates past its cap: a spawn into a full pool replaces
/// the oldest live slot (smallest insertion sequence) and bumps the reuse
/// counter, so GM1 can read load pressure without guessing.
#[derive(Clone, Debug)]
pub struct Particles {
    slots: Vec<Slot>,
    cap: usize,
    next_seq: u64,
    reused: u64,
    tick: u64,
}

impl Default for Particles {
    fn default() -> Self {
        Self::new()
    }
}

impl Particles {
    /// Empty pool with the default [`MAX_PARTICLES`] cap.
    pub fn new() -> Self {
        Self::with_capacity(MAX_PARTICLES)
    }

    /// Empty pool with a custom cap, clamped to `1..=MAX_PARTICLE_CAP`
    /// so tests can prove exact reuse on a tiny pool.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            slots: Vec::new(),
            cap: cap.clamp(1, MAX_PARTICLE_CAP),
            next_seq: 0,
            reused: 0,
            tick: 0,
        }
    }

    /// Pool cap (slots, not bytes).
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Live particle count.
    pub fn alive_count(&self) -> usize {
        self.slots.len()
    }

    /// How many spawns reused the oldest slot because the pool was full.
    pub fn reused_count(&self) -> u64 {
        self.reused
    }

    /// Fixed steps taken so far.
    pub fn tick_count(&self) -> u64 {
        self.tick
    }

    /// Insert one particle. Returns `false` (pool untouched) when any
    /// component is non-finite or `life` is outside `(0, MAX_PARTICLE_LIFE]`;
    /// otherwise inserts — reusing the oldest slot when full — and returns
    /// `true`.
    pub fn spawn(&mut self, pos: [f32; 3], vel: [f32; 3], life: f32, kind: ParticleKind) -> bool {
        if !pos.iter().all(|c| c.is_finite()) || !vel.iter().all(|c| c.is_finite()) {
            return false;
        }
        if !life.is_finite() || life <= 0.0 || life > MAX_PARTICLE_LIFE {
            return false;
        }
        let slot = Slot {
            pos,
            vel,
            life,
            age: 0.0,
            kind,
            seq: self.next_seq,
        };
        self.next_seq += 1;
        if self.slots.len() < self.cap {
            self.slots.push(slot);
        } else {
            // Full: reuse the oldest slot (smallest insertion sequence).
            // Slot order is insertion order except for replacements, so the
            // scan is a stable tie-break over equal sequences (impossible:
            // sequences are unique).
            let mut oldest = 0;
            for i in 1..self.slots.len() {
                if self.slots[i].seq < self.slots[oldest].seq {
                    oldest = i;
                }
            }
            self.slots[oldest] = slot;
            self.reused += 1;
        }
        true
    }

    /// Torch fire through the same particle core: spawns
    /// [`TORCH_FLAME_COUNT`] sparks above `base` with its own constants
    /// (rise, spread, life). Spread comes from an integer hash of the
    /// insertion sequence — deterministic for a repeated call script, with
    /// no RNG state and no wall clock. Returns accepted sparks.
    pub fn spawn_torch_flame(&mut self, base: [f32; 3]) -> usize {
        if !base.iter().all(|c| c.is_finite()) {
            return 0;
        }
        let mut accepted = 0;
        for _ in 0..TORCH_FLAME_COUNT {
            // Integer hash of the upcoming sequence: two byte lanes mapped
            // from [0, 255] to [-1, 1] for the horizontal kick.
            let h = self.next_seq.wrapping_mul(0x9E3779B97F4A7C15);
            let q = |shift: u32| ((h >> shift) & 0xFF) as f32 / 255.0 * 2.0 - 1.0;
            let vel = [q(0) * TORCH_SPREAD, TORCH_RISE, q(8) * TORCH_SPREAD];
            let pos = [base[0], base[1] + 0.05, base[2]];
            if self.spawn(pos, vel, TORCH_LIFE, ParticleKind::TorchSpark) {
                accepted += 1;
            }
        }
        accepted
    }

    /// Advance exactly one fixed step. `dt` must equal [`FIXED_DT`] bit for
    /// bit — any other value leaves the pool unchanged (still returns the
    /// current alive count) so a variable frame time can never leak into
    /// the sim. Returns the alive count after the step.
    pub fn step(&mut self, dt: f32) -> usize {
        if dt.to_bits() != FIXED_DT.to_bits() {
            return self.slots.len();
        }
        for slot in &mut self.slots {
            slot.age += FIXED_DT;
            match slot.kind {
                ParticleKind::Dust => {
                    slot.vel[1] -= DUST_GRAVITY * FIXED_DT;
                    let k = 1.0 / (1.0 + DUST_DRAG * FIXED_DT);
                    slot.vel[0] *= k;
                    slot.vel[1] *= k;
                    slot.vel[2] *= k;
                }
                ParticleKind::TorchSpark => {
                    slot.vel[1] += TORCH_BUOYANCY * FIXED_DT;
                    let k = 1.0 / (1.0 + TORCH_DRAG * FIXED_DT);
                    slot.vel[0] *= k;
                    slot.vel[1] *= k;
                    slot.vel[2] *= k;
                }
            }
            slot.pos[0] += slot.vel[0] * FIXED_DT;
            slot.pos[1] += slot.vel[1] * FIXED_DT;
            slot.pos[2] += slot.vel[2] * FIXED_DT;
        }
        // Stable retain: survivors keep slot order, so repeated scripts hash
        // identically.
        self.slots.retain(|s| s.age < s.life);
        self.tick += 1;
        self.slots.len()
    }

    /// Display size of a live slot: dust grows as it disperses, sparks
    /// shrink as they burn out. Pure function of age/life for exact tests.
    pub fn size_of(age: f32, life: f32, kind: ParticleKind) -> f32 {
        let t = (age / life).clamp(0.0, 1.0);
        match kind {
            ParticleKind::Dust => DUST_SIZE * (1.0 + t),
            ParticleKind::TorchSpark => TORCH_SIZE * (1.0 - 0.5 * t),
        }
    }

    /// Display alpha of a live slot: linear fade over life. Pure function
    /// of age/life for exact tests.
    pub fn alpha_of(age: f32, life: f32) -> f32 {
        (1.0 - (age / life).clamp(0.0, 1.0)).clamp(0.0, 1.0)
    }

    /// Base display color of a kind.
    pub fn color_of(kind: ParticleKind) -> [f32; 3] {
        match kind {
            ParticleKind::Dust => DUST_COLOR,
            ParticleKind::TorchSpark => SPARK_COLOR,
        }
    }

    /// FNV-1a hash over tick, alive count and every live slot in order
    /// (position/velocity/size/alpha bits plus kind). Same spawn script
    /// stepped the same way gives the same hash.
    pub fn state_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut mix = |b: u8| {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        };
        for b in self
            .slots
            .len()
            .to_le_bytes()
            .iter()
            .chain(self.tick.to_le_bytes().iter())
        {
            mix(*b);
        }
        for s in &self.slots {
            for w in [
                s.pos[0].to_bits(),
                s.pos[1].to_bits(),
                s.pos[2].to_bits(),
                s.vel[0].to_bits(),
                s.vel[1].to_bits(),
                s.vel[2].to_bits(),
                s.age.to_bits(),
                s.life.to_bits(),
            ] {
                for b in w.to_le_bytes() {
                    mix(b);
                }
            }
            mix(match s.kind {
                ParticleKind::Dust => 0,
                ParticleKind::TorchSpark => 1,
            });
            for b in s.seq.to_le_bytes() {
                mix(b);
            }
        }
        h
    }

    /// Per-instance bytes for the GPU billboard pass: one 32-byte record
    /// per live slot in slot order — world pos (3) + size (1) + rgba (4),
    /// f32 little-endian. Alpha already carries the age fade.
    pub fn upload_instances(&self) -> Vec<u8> {
        const STRIDE: usize = 8 * 4;
        let mut out = Vec::with_capacity(self.slots.len() * STRIDE);
        for s in &self.slots {
            for c in s.pos {
                out.extend_from_slice(&c.to_le_bytes());
            }
            out.extend_from_slice(&Self::size_of(s.age, s.life, s.kind).to_le_bytes());
            let col = Self::color_of(s.kind);
            for c in col {
                out.extend_from_slice(&c.to_le_bytes());
            }
            out.extend_from_slice(&Self::alpha_of(s.age, s.life).to_le_bytes());
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Decals: bounded blood splats on the sand, XZ cell hash, tick fade.
// ---------------------------------------------------------------------------

/// Hard bound for [`DecalSet::with_capacity`].
pub const MAX_DECAL_CAP: usize = 1024;
/// XZ cell edge in meters for [`DecalSet::query`].
pub const DECAL_CELL: f32 = 1.0;

/// Which gore visual a decal carries. Only blood ships in R5; CB2 adds
/// variants through this enum without changing the cell/fade core.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecalKind {
    /// Blood on sand.
    Blood,
}

/// One splat: ground-projected quad data plus age for the fade schedule.
#[derive(Clone, Copy, Debug)]
pub struct Decal {
    /// World position of the splat center in meters.
    pub pos: [f32; 3],
    /// Surface normal at the splat (unit; sand is +Y).
    pub normal: [f32; 3],
    /// Splat radius in meters.
    pub radius: f32,
    /// Gore variant.
    pub kind: DecalKind,
    /// Ticks since the splat; culled at [`BLOOD_FADE_TICKS`].
    pub age_ticks: u64,
    /// XZ cell from [`cell_of`], cached so query never recomputes.
    pub cell: (i32, i32),
}

impl Decal {
    /// Display alpha: linear fade `1 -> 0` over [`BLOOD_FADE_TICKS`].
    /// Pure function of age for exact tick-script tests.
    pub fn alpha(age_ticks: u64) -> f32 {
        if age_ticks >= BLOOD_FADE_TICKS {
            return 0.0;
        }
        (BLOOD_FADE_TICKS - age_ticks) as f32 / BLOOD_FADE_TICKS as f32
    }
}

/// XZ cell of a world position: `floor(x / DECAL_CELL), floor(z / DECAL_CELL)`.
/// Y (height) never participates — decals live on the ground plane.
pub fn cell_of(pos: [f32; 3]) -> (i32, i32) {
    (
        (pos[0] / DECAL_CELL).floor() as i32,
        (pos[2] / DECAL_CELL).floor() as i32,
    )
}

/// Bounded blood-splat set with cell query and tick fade.
///
/// Full sets evict the oldest splat (smallest insertion sequence) and count
/// each eviction, mirroring the particle pool so CB2 gore bursts degrade to
/// "oldest blood fades first" instead of dropping new hits.
#[derive(Clone, Debug)]
pub struct DecalSet {
    decals: Vec<Decal>,
    seqs: Vec<u64>,
    cap: usize,
    next_seq: u64,
    evicted: u64,
    tick: u64,
}

impl Default for DecalSet {
    fn default() -> Self {
        Self::new()
    }
}

impl DecalSet {
    /// Empty set with the default [`MAX_DECALS`] cap.
    pub fn new() -> Self {
        Self::with_capacity(MAX_DECALS)
    }

    /// Empty set with a custom cap, clamped to `1..=MAX_DECAL_CAP` so
    /// tests can prove exact eviction on a tiny set.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            decals: Vec::new(),
            seqs: Vec::new(),
            cap: cap.clamp(1, MAX_DECAL_CAP),
            next_seq: 0,
            evicted: 0,
            tick: 0,
        }
    }

    /// Splat cap.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Live splat count.
    pub fn len(&self) -> usize {
        self.decals.len()
    }

    /// Whether no splats are live.
    pub fn is_empty(&self) -> bool {
        self.decals.is_empty()
    }

    /// How many splats were evicted as oldest because the set was full.
    pub fn evicted_count(&self) -> u64 {
        self.evicted
    }

    /// Ticks advanced so far.
    pub fn tick_count(&self) -> u64 {
        self.tick
    }

    /// All live decals in insertion order (parallel to [`DecalSet::query`]
    /// results for tests).
    pub fn decals(&self) -> &[Decal] {
        &self.decals
    }

    /// Splat blood at `pos` with `normal` and `radius`. Returns `false`
    /// (set untouched) when any position/normal component is non-finite,
    /// the normal is degenerate, or `radius` is outside `(0,
    /// MAX_DECAL_RADIUS]`; otherwise inserts — evicting the oldest splat
    /// when full — and returns `true`.
    pub fn splat(&mut self, pos: [f32; 3], normal: [f32; 3], radius: f32, kind: DecalKind) -> bool {
        if !pos.iter().all(|c| c.is_finite()) || !normal.iter().all(|c| c.is_finite()) {
            return false;
        }
        if !radius.is_finite() || radius <= 0.0 || radius > MAX_DECAL_RADIUS {
            return false;
        }
        let l2 = normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2];
        if !l2.is_finite() || l2 < 1e-12 {
            return false;
        }
        let l = l2.sqrt();
        let decal = Decal {
            pos,
            normal: [normal[0] / l, normal[1] / l, normal[2] / l],
            radius,
            kind,
            age_ticks: 0,
            cell: cell_of(pos),
        };
        let seq = self.next_seq;
        self.next_seq += 1;
        if self.decals.len() < self.cap {
            self.decals.push(decal);
            self.seqs.push(seq);
        } else {
            let mut oldest = 0;
            for i in 1..self.seqs.len() {
                if self.seqs[i] < self.seqs[oldest] {
                    oldest = i;
                }
            }
            self.decals[oldest] = decal;
            self.seqs[oldest] = seq;
            self.evicted += 1;
        }
        true
    }

    /// Advance one tick: every splat ages by one and splats reaching
    /// [`BLOOD_FADE_TICKS`] are culled (stable order for survivors).
    pub fn tick(&mut self) {
        for d in &mut self.decals {
            d.age_ticks += 1;
        }
        // Retain must drop seqs in lockstep with decals; manual loop keeps
        // both vectors aligned without a second pass.
        let mut kept = 0;
        for i in 0..self.decals.len() {
            if self.decals[i].age_ticks < BLOOD_FADE_TICKS {
                if kept != i {
                    self.decals.swap(kept, i);
                    self.seqs.swap(kept, i);
                }
                kept += 1;
            }
        }
        self.decals.truncate(kept);
        self.seqs.truncate(kept);
        self.tick += 1;
    }

    /// All live decals in cell `cell` (exact XZ match), in set order.
    /// CB2 queries the victim's cell; GM1 queries torch cells for scorch.
    pub fn query(&self, cell: (i32, i32)) -> Vec<&Decal> {
        self.decals.iter().filter(|d| d.cell == cell).collect()
    }

    /// Per-instance bytes for the GPU ground-quad pass: one 32-byte record
    /// per live splat in set order — center pos (3) + radius (1) + rgba (4),
    /// f32 little-endian. Alpha already carries the tick fade.
    pub fn upload_instances(&self) -> Vec<u8> {
        const STRIDE: usize = 8 * 4;
        let mut out = Vec::with_capacity(self.decals.len() * STRIDE);
        for d in &self.decals {
            for c in d.pos {
                out.extend_from_slice(&c.to_le_bytes());
            }
            out.extend_from_slice(&d.radius.to_le_bytes());
            let col = match d.kind {
                DecalKind::Blood => BLOOD_COLOR,
            };
            for c in col {
                out.extend_from_slice(&c.to_le_bytes());
            }
            out.extend_from_slice(&Decal::alpha(d.age_ticks).to_le_bytes());
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Instanced quad: the unit geometry both uploads share, via the v1 mesh path.
// ---------------------------------------------------------------------------

/// Bytes per instance record (pos3 + size/radius + rgba = 8 f32 LE).
pub const INSTANCE_STRIDE: usize = 32;

/// Unit quad (−0.5..0.5 in XY, facing +Z) built through the shared v1
/// [`upload_mesh`] validation, so the billboard/ground-quad geometry takes
/// the same upload path as every v1 mesh. The GPU pass instances it with
/// [`Particles::upload_instances`] / [`DecalSet::upload_instances`]
/// (world center + size in the instance record; billboarding itself is the
/// `v1_gpu`/GM1 pass, not this CPU table).
pub fn quad_mesh() -> Result<MeshData, MeshError> {
    upload_mesh(
        &[
            [-0.5, -0.5, 0.0],
            [0.5, -0.5, 0.0],
            [0.5, 0.5, 0.0],
            [-0.5, 0.5, 0.0],
        ],
        &[
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
        ],
        &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        &[0, 1, 2, 0, 2, 3],
    )
}

/// VFX input failures, for consumers that want typed errors instead of the
/// boolean `spawn`/`splat` verdicts.
#[derive(Clone, PartialEq, Debug)]
pub enum VfxError {
    /// A position/velocity/normal component is non-finite; field carried.
    NonFinite(&'static str),
    /// Life outside `(0, MAX_PARTICLE_LIFE]`; value carried.
    BadLife(f32),
    /// Radius outside `(0, MAX_DECAL_RADIUS]`; value carried.
    BadRadius(f32),
    /// Normal has ~zero length.
    DegenerateNormal,
}

impl fmt::Display for VfxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(w) => write!(f, "vfx {w} is not finite"),
            Self::BadLife(v) => write!(f, "vfx life {v} outside (0, {MAX_PARTICLE_LIFE}]"),
            Self::BadRadius(v) => write!(f, "vfx radius {v} outside (0, {MAX_DECAL_RADIUS}]"),
            Self::DegenerateNormal => write!(f, "vfx normal has ~zero length"),
        }
    }
}

impl std::error::Error for VfxError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn dust_script() -> Vec<([f32; 3], [f32; 3], f32, ParticleKind)> {
        vec![
            ([0.0, 0.1, 0.0], [1.0, 2.0, 0.0], 1.0, ParticleKind::Dust),
            ([1.0, 0.2, -1.0], [-1.0, 1.0, 0.5], 0.5, ParticleKind::Dust),
            (
                [0.0, 1.0, 0.0],
                [0.0, 0.5, 0.0],
                0.7,
                ParticleKind::TorchSpark,
            ),
            ([-2.0, 0.0, 2.0], [0.5, 3.0, -0.5], 2.0, ParticleKind::Dust),
        ]
    }

    fn run_script(steps: u64) -> (Particles, Vec<u64>) {
        let mut p = Particles::new();
        for (pos, vel, life, kind) in dust_script() {
            assert!(p.spawn(pos, vel, life, kind));
        }
        let mut hashes = Vec::new();
        for _ in 0..steps {
            p.step(FIXED_DT);
            hashes.push(p.state_hash());
        }
        (p, hashes)
    }

    #[test]
    fn pool_cap_reuses_oldest_with_exact_counter() {
        let mut p = Particles::with_capacity(3);
        assert!(p.spawn([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], 10.0, ParticleKind::Dust));
        assert!(p.spawn([1.0, 0.0, 0.0], [0.0, 0.0, 0.0], 10.0, ParticleKind::Dust));
        assert!(p.spawn([2.0, 0.0, 0.0], [0.0, 0.0, 0.0], 10.0, ParticleKind::Dust));
        assert_eq!((p.alive_count(), p.reused_count()), (3, 0));
        // Full: next spawn evicts x=0 (oldest), then x=1.
        assert!(p.spawn([9.0, 0.0, 0.0], [0.0, 0.0, 0.0], 10.0, ParticleKind::Dust));
        assert!(p.spawn([8.0, 0.0, 0.0], [0.0, 0.0, 0.0], 10.0, ParticleKind::Dust));
        assert_eq!((p.alive_count(), p.reused_count()), (3, 2));
        let mut xs: Vec<f32> = {
            let bytes = p.upload_instances();
            assert_eq!(bytes.len(), 3 * INSTANCE_STRIDE);
            (0..3)
                .map(|i| f32::from_le_bytes(bytes[i * 32..i * 32 + 4].try_into().unwrap()))
                .collect()
        };
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(xs, vec![2.0, 8.0, 9.0]);
    }

    #[test]
    fn same_spawn_script_twice_identical() {
        let (a, ha) = run_script(60);
        let (b, hb) = run_script(60);
        assert_eq!(ha, hb);
        assert_eq!(a.state_hash(), b.state_hash());
        assert_eq!(a.upload_instances(), b.upload_instances());
        // The sim must actually evolve: first and last hashes differ.
        assert_ne!(ha[0], *ha.last().unwrap());
    }

    #[test]
    fn torch_flame_reuses_core_with_own_constants() {
        let mut p = Particles::new();
        assert_eq!(
            p.spawn_torch_flame([0.0, 0.0, 0.0]),
            TORCH_FLAME_COUNT as usize
        );
        assert_eq!(p.alive_count(), TORCH_FLAME_COUNT as usize);
        // Same base twice: identical bytes (integer-hash spread, no RNG).
        let mut q = Particles::new();
        assert_eq!(
            q.spawn_torch_flame([0.0, 0.0, 0.0]),
            TORCH_FLAME_COUNT as usize
        );
        assert_eq!(p.state_hash(), q.state_hash());
        // Sparks rise on the first step while dust would settle.
        let y0: Vec<f32> = {
            let bytes = p.upload_instances();
            (0..TORCH_FLAME_COUNT as usize)
                .map(|i| f32::from_le_bytes(bytes[i * 32 + 4..i * 32 + 8].try_into().unwrap()))
                .collect()
        };
        p.step(FIXED_DT);
        let y1: Vec<f32> = {
            let bytes = p.upload_instances();
            (0..TORCH_FLAME_COUNT as usize)
                .map(|i| f32::from_le_bytes(bytes[i * 32 + 4..i * 32 + 8].try_into().unwrap()))
                .collect()
        };
        for (a, b) in y0.iter().zip(y1.iter()) {
            assert!(b > a, "torch sparks must rise");
        }
        // Flame rejects a non-finite base without touching the pool.
        assert_eq!(p.spawn_torch_flame([f32::NAN, 0.0, 0.0]), 0);
    }

    #[test]
    fn step_rejects_variable_dt_without_advancing() {
        let mut p = Particles::new();
        assert!(p.spawn([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], 5.0, ParticleKind::Dust));
        let before = p.state_hash();
        assert_eq!(p.step(1.0 / 30.0), 1);
        assert_eq!(p.tick_count(), 0);
        assert_eq!(p.state_hash(), before);
        assert_eq!(p.step(FIXED_DT), 1);
        assert_eq!(p.tick_count(), 1);
    }

    #[test]
    fn spawn_rejects_bad_input_pool_untouched() {
        let mut p = Particles::new();
        assert!(!p.spawn(
            [f32::NAN, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            1.0,
            ParticleKind::Dust
        ));
        assert!(!p.spawn(
            [0.0, 0.0, 0.0],
            [0.0, f32::INFINITY, 0.0],
            1.0,
            ParticleKind::Dust
        ));
        assert!(!p.spawn([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], 0.0, ParticleKind::Dust));
        assert!(!p.spawn(
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            MAX_PARTICLE_LIFE + 1.0,
            ParticleKind::Dust
        ));
        assert_eq!((p.alive_count(), p.reused_count()), (0, 0));
    }

    #[test]
    fn particles_die_on_schedule() {
        let mut p = Particles::new();
        assert!(p.spawn(
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            3.0 * FIXED_DT,
            ParticleKind::Dust
        ));
        assert_eq!(p.step(FIXED_DT), 1);
        assert_eq!(p.step(FIXED_DT), 1);
        // age (3 ticks) reaches life exactly: culled on the third step.
        assert_eq!(p.step(FIXED_DT), 0);
    }

    #[test]
    fn decals_bounded_with_exact_cell_query() {
        let mut d = DecalSet::with_capacity(4);
        assert!(d.splat([0.2, 0.0, 0.3], [0.0, 1.0, 0.0], 0.3, DecalKind::Blood));
        assert!(d.splat([0.7, 0.0, 0.1], [0.0, 1.0, 0.0], 0.4, DecalKind::Blood));
        assert!(d.splat([1.5, 0.0, 0.2], [0.0, 1.0, 0.0], 0.3, DecalKind::Blood));
        assert!(d.splat([-0.5, 0.0, -1.2], [0.0, 1.0, 0.0], 0.5, DecalKind::Blood));
        assert_eq!((d.len(), d.evicted_count()), (4, 0));
        // Cells: (0,0) x2, (1,0), (-1,-2).
        assert_eq!(d.query((0, 0)).len(), 2);
        assert_eq!(d.query((1, 0)).len(), 1);
        assert_eq!(d.query((-1, -2)).len(), 1);
        assert!(d.query((9, 9)).is_empty());
        // Full: oldest (0.2) evicted, counter exact, cell counts shift.
        assert!(d.splat([0.1, 0.0, 0.1], [0.0, 1.0, 0.0], 0.2, DecalKind::Blood));
        assert_eq!((d.len(), d.evicted_count()), (4, 1));
        assert_eq!(d.query((0, 0)).len(), 2);
        let radii: Vec<f32> = d.query((0, 0)).iter().map(|x| x.radius).collect();
        assert!(radii.contains(&0.4) && radii.contains(&0.2));
        assert!(!radii.contains(&0.3));
    }

    #[test]
    fn decal_fade_schedule_exact_on_tick_script() {
        let mut d = DecalSet::new();
        assert!(d.splat([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.5, DecalKind::Blood));
        // Fresh splat is fully opaque.
        assert_eq!(Decal::alpha(0), 1.0);
        for _ in 0..300 {
            d.tick();
        }
        assert_eq!(d.len(), 1);
        assert_eq!(d.decals()[0].age_ticks, 300);
        assert_eq!(
            Decal::alpha(300),
            (BLOOD_FADE_TICKS - 300) as f32 / BLOOD_FADE_TICKS as f32
        );
        // Upload alpha carries the same fade.
        let bytes = d.upload_instances();
        assert_eq!(bytes.len(), INSTANCE_STRIDE);
        let a = f32::from_le_bytes(bytes[28..32].try_into().unwrap());
        assert_eq!(a, Decal::alpha(300));
        for _ in 300..BLOOD_FADE_TICKS {
            d.tick();
        }
        // 600 ticks: culled exactly, alpha terminal is 0.
        assert!(d.is_empty());
        assert_eq!(Decal::alpha(BLOOD_FADE_TICKS), 0.0);
        assert_eq!(d.tick_count(), BLOOD_FADE_TICKS);
    }

    #[test]
    fn splat_rejects_bad_input_set_untouched() {
        let mut d = DecalSet::new();
        assert!(!d.splat([f32::NAN, 0.0, 0.0], [0.0, 1.0, 0.0], 0.3, DecalKind::Blood));
        assert!(!d.splat([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], 0.3, DecalKind::Blood));
        assert!(!d.splat([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.0, DecalKind::Blood));
        assert!(!d.splat(
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            MAX_DECAL_RADIUS + 0.5,
            DecalKind::Blood
        ));
        assert!(d.is_empty());
    }

    #[test]
    fn quad_mesh_rides_the_v1_upload_path() {
        let m = quad_mesh().expect("unit quad is valid");
        assert_eq!((m.positions.len(), m.indices.len()), (4, 6));
        assert!(!m.vertex_bytes().is_empty());
        assert_eq!(m.vertex_bytes().len(), 4 * 48);
        // Instance strides match the declared stride on both uploads.
        let mut p = Particles::new();
        assert!(p.spawn([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], 1.0, ParticleKind::Dust));
        assert_eq!(p.upload_instances().len(), INSTANCE_STRIDE);
        let mut d = DecalSet::new();
        assert!(d.splat([0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.3, DecalKind::Blood));
        assert_eq!(d.upload_instances().len(), INSTANCE_STRIDE);
    }

    #[test]
    fn uploads_stay_finite() {
        let mut p = Particles::new();
        for (pos, vel, life, kind) in dust_script() {
            assert!(p.spawn(pos, vel, life, kind));
        }
        p.spawn_torch_flame([1.0, 0.5, -1.0]);
        for _ in 0..120 {
            p.step(FIXED_DT);
        }
        for chunk in p.upload_instances().chunks_exact(4) {
            assert!(f32::from_le_bytes(chunk.try_into().unwrap()).is_finite());
        }
        let mut d = DecalSet::new();
        assert!(d.splat([3.0, 0.0, -2.0], [0.0, 1.0, 0.0], 1.0, DecalKind::Blood));
        for _ in 0..100 {
            d.tick();
        }
        for chunk in d.upload_instances().chunks_exact(4) {
            assert!(f32::from_le_bytes(chunk.try_into().unwrap()).is_finite());
        }
    }
}
