//! Canonical budget workloads: arena tick, physics, AI, destruction, spine.
//! Each workload owns all its state locally (no statics, no shared handles)
//! so the same canonical run hashes identically on any thread. Folding is a
//! local FNV-1a over canonical bytes only; wall-clock time never enters.

use crate::alloc::{self, AllocSnapshot};
use crate::arena::{FrameArena, FrameError, CULL_KEYS, FRAME_BYTES};
use engine_ai::{Brain, Percept};
use engine_arena::{ArenaConfig, FighterId, FighterTape, AI_PERIOD_TICKS, MAX_PARTICIPANTS};
use engine_core::fixed::Fixed;
use engine_core::hash::FNV64_OFFSET;
use engine_core::Rng;
use engine_destruction::{event_bytes, BondedCollection};
use engine_ecs::spine::{AiState, Body as SpineBody, Spine, Transform, Vitals, MAX_SPINE_SLOTS};
use engine_physics::{Body, Space};
use engine_render::core::ForwardPlus;

/// Fixed step shared by every workload, matching the 60 Hz tick.
pub const DT_S: f32 = 1.0 / 60.0;

/// Multiplier of the workload fold. The recorded baselines pin hashes made
/// with it, so it stays distinct from the standard FNV-1a prime
/// ([`engine_core::hash::FNV64_PRIME`]); changing it re-records every row.
const MIX_PRIME: u64 = 0x1000_0000_01b3;

/// Fold one word into the running hash, little-endian per HASH-LAW.
fn mix(hash: &mut u64, word: u64) {
    for byte in word.to_le_bytes() {
        *hash ^= u64::from(byte);
        *hash = hash.wrapping_mul(MIX_PRIME);
    }
}

/// Fold a checkpoint run into the canonical hash: the run length folds in
/// first, so a rejected config (empty run) can never match a recorded
/// baseline. Single owner of the arena fold; every arena workload calls it.
fn fold_checkpoints(checkpoints: &[engine_arena::Checkpoint]) -> u64 {
    let mut hash = FNV64_OFFSET;
    mix(&mut hash, checkpoints.len() as u64);
    for cp in checkpoints {
        mix(&mut hash, u64::from(cp.tick));
        mix(&mut hash, cp.hash);
    }
    hash
}

/// One measured canonical run: hash plus the thread-local allocations made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkloadReport {
    /// Workload name, matching `baselines`.
    pub workload: &'static str,
    /// Fixed ticks simulated.
    pub ticks: u32,
    /// Canonical hash of the run.
    pub hash: u64,
    /// Bytes allocated on the run thread.
    pub alloc_bytes: u64,
    /// Allocation count on the run thread.
    pub alloc_count: u64,
}

/// Run `hash` under the allocation counter and pack the report: the one
/// owner of the report shape every workload's `run_report` fills.
fn measured_report(
    workload: &'static str,
    ticks: u32,
    hash: impl FnOnce() -> u64,
) -> WorkloadReport {
    let (hash, observed) = alloc::measure(hash);
    WorkloadReport {
        workload,
        ticks,
        hash,
        alloc_bytes: observed.bytes,
        alloc_count: observed.count,
    }
}

/// Canonical arena hash over `config`: empty tape, so every slot is
/// AI-filled and the run covers arena, brain, combat-step and destruction.
fn arena_run_hash(config: &ArenaConfig) -> u64 {
    fold_checkpoints(&engine_arena::run(config, &FighterTape(Vec::new())))
}

/// Single owner for every arena match config built in this file.
/// Consumer: `ArenaTick::config` plus `ScaledArenaTick::config`.
/// Data owner: caller-owned values (pure value, `Copy` out).
/// Units: ticks are fixed steps, extent is meters. Determinism: field copy
/// only, no clock. Errors: none (callers pass baseline-listed shapes).
fn arena_config(seed: u64, ticks: u32, fighters: u8, half_extent_m: f32) -> ArenaConfig {
    // TABLES_VERSION_V1 keeps the golden frozen on v1 rules.
    ArenaConfig {
        seed,
        ticks,
        fighters,
        half_extent_m,
        table_version: 1,
    }
}

/// Full-match arena tick at the engine maximum fighter count.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArenaTick {
    /// Match seed, expanded per the arena fixture rules.
    pub seed: u64,
    /// Fixed ticks per run.
    pub ticks: u32,
    /// Arena half-extent in meters.
    pub half_extent_m: f32,
}

impl ArenaTick {
    /// Workload name used in baselines and budget errors.
    pub const NAME: &'static str = "arena_tick";

    /// Canonical workload: `MAX_PARTICIPANTS` (16, the engine maximum) with an
    /// empty tape, so every slot is AI-filled and the run covers arena plus
    /// brains plus combat plus the destruction prop. Seed 1 is chosen
    /// because its spawn geometry produces organic combat (40 combat events
    /// and 17 crate breaks over 600 ticks), covering the hit and break
    /// paths, not just movement.
    pub const fn canonical() -> Self {
        Self {
            seed: 1,
            ticks: 600,
            half_extent_m: 20.0,
        }
    }

    /// Same shape with a different seed (regression proofs).
    pub const fn with_seed(self, seed: u64) -> Self {
        Self { seed, ..self }
    }

    /// Match config for this workload: `MAX_PARTICIPANTS` slots.
    pub fn config(&self) -> ArenaConfig {
        arena_config(self.seed, self.ticks, MAX_PARTICIPANTS, self.half_extent_m)
    }

    /// Canonical hash: FNV over the run length plus every checkpoint.
    /// The length folds in first, so a rejected config (empty run) can never
    /// match a recorded baseline.
    pub fn run_hash(&self) -> u64 {
        arena_run_hash(&self.config())
    }

    /// Canonical run plus its allocation totals.
    pub fn run_report(&self) -> WorkloadReport {
        measured_report(Self::NAME, self.ticks, || self.run_hash())
    }
}

/// Fixed-step integration of a seeded body cloud (std-only `Space`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicsStep {
    /// Seed for body placement and velocities.
    pub seed: u64,
    /// Body count.
    pub bodies: usize,
    /// Fixed ticks per run.
    pub ticks: u32,
}

impl PhysicsStep {
    /// Workload name used in baselines and budget errors.
    pub const NAME: &'static str = "physics_step";

    /// Canonical workload: 256 bodies, 600 ticks.
    pub const fn canonical() -> Self {
        Self {
            seed: 4_242,
            bodies: 256,
            ticks: 600,
        }
    }

    /// Same shape with a different seed (regression proofs).
    pub const fn with_seed(self, seed: u64) -> Self {
        Self { seed, ..self }
    }

    fn seeded_space(&self) -> Space {
        let mut space = Space::new();
        let mut rng = Rng::new(self.seed);
        for i in 0..self.bodies {
            let pick = |rng: &mut Rng| (rng.next_u64() % 2_000) as f32 / 10.0 - 100.0;
            let (x, y) = (pick(&mut rng), pick(&mut rng));
            let (vx, vy) = if i % 2 == 0 { (1.0, 0.5) } else { (-0.5, 0.25) };
            space.add(Body {
                x,
                y,
                vx,
                vy,
                hx: 0.5,
                hy: 0.5,
            });
        }
        space
    }

    /// Canonical hash: FNV over the body count plus every tick's state hash.
    pub fn run_hash(&self) -> u64 {
        let mut space = self.seeded_space();
        let mut hash = FNV64_OFFSET;
        mix(&mut hash, self.bodies as u64);
        for _ in 0..self.ticks {
            space.step(DT_S);
            mix(&mut hash, space.state_hash());
        }
        hash
    }

    /// Canonical run plus its allocation totals.
    pub fn run_report(&self) -> WorkloadReport {
        measured_report(Self::NAME, self.ticks, || self.run_hash())
    }

    /// Warmed space (built plus settled) for steady-state measurement.
    fn warmed_space(&self) -> Space {
        let mut space = self.seeded_space();
        for _ in 0..30 {
            space.step(DT_S);
        }
        space
    }

    /// Steady-state slice: hash plus allocations over `measure_ticks` steps
    /// on a warmed space. Allocation-free by construction; the test asserts it.
    pub fn steady(&self, measure_ticks: u32) -> (u64, AllocSnapshot) {
        let mut space = self.warmed_space();
        alloc::measure(|| {
            let mut hash = FNV64_OFFSET;
            for _ in 0..measure_ticks {
                space.step(DT_S);
                mix(&mut hash, space.state_hash());
            }
            hash
        })
    }
}

/// Scripted-percept decisions for one brain per fighter slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AiDecide {
    /// Seed for the scripted percept lattice.
    pub seed: u64,
    /// Fixed ticks per run.
    pub ticks: u32,
}

impl AiDecide {
    /// Workload name used in baselines and budget errors.
    pub const NAME: &'static str = "ai_decide";

    /// Canonical workload: `MAX_PARTICIPANTS` brains, 600 ticks.
    pub const fn canonical() -> Self {
        Self {
            seed: 9_001,
            ticks: 600,
        }
    }

    /// Same shape with a different seed (regression proofs).
    pub const fn with_seed(self, seed: u64) -> Self {
        Self { seed, ..self }
    }

    fn brains() -> Vec<Brain> {
        (0..MAX_PARTICIPANTS)
            .map(|slot| {
                Brain::new(
                    FighterId::new(slot).expect("canonical slots are below MAX_PARTICIPANTS"),
                    AI_PERIOD_TICKS,
                )
            })
            .collect()
    }

    /// Canonical hash: FNV over every decided input's fields, no heap use.
    pub fn run_hash(&self) -> u64 {
        let mut brains = Self::brains();
        let mut hash = FNV64_OFFSET;
        for tick in 0..self.ticks {
            for (slot, brain) in brains.iter_mut().enumerate() {
                let percept = scripted_percept(self.seed, slot_u8(slot), tick);
                mix_input(&mut hash, brain.decide(&percept));
            }
        }
        hash
    }

    /// Canonical run plus its allocation totals.
    pub fn run_report(&self) -> WorkloadReport {
        measured_report(Self::NAME, self.ticks, || self.run_hash())
    }

    /// Steady-state slice: hash plus allocations over `measure_ticks` ticks
    /// on warmed brains. Allocation-free by construction; the test asserts it.
    pub fn steady(&self, measure_ticks: u32) -> (u64, AllocSnapshot) {
        let mut brains = Self::brains();
        for tick in 0..12 {
            for (slot, brain) in brains.iter_mut().enumerate() {
                let _ = brain.decide(&scripted_percept(self.seed, slot_u8(slot), tick));
            }
        }
        alloc::measure(|| {
            let mut hash = FNV64_OFFSET;
            for tick in 0..measure_ticks {
                for (slot, brain) in brains.iter_mut().enumerate() {
                    let percept = scripted_percept(self.seed, slot_u8(slot), tick);
                    mix_input(&mut hash, brain.decide(&percept));
                }
            }
            hash
        })
    }
}

/// Fold one decided input without allocating: tick plus packed fields.
fn mix_input(hash: &mut u64, input: engine_arena::FighterInput) {
    mix(hash, u64::from(input.tick));
    let packed = (u64::from(input.fighter.0) << 40)
        | (u64::from(input.mx as u8) << 32)
        | (u64::from(input.mz as u8) << 24)
        | (u64::from(input.facing as u16) << 8)
        | u64::from(input.buttons.0);
    mix(hash, packed);
}

/// Slot index for the scripted percept lattice as a saturating `u8`.
///
/// Consumer: [`AiDecide`] run/steady hashes feed [`scripted_percept`].
/// Data owner: this conversion owns the `usize` slot to `u8` lattice map.
/// Units: slot index, `0..=255` fits, larger slots saturate to `255`.
/// Determinism: pure saturating map, same slot gives same byte, no clock.
/// Errors: none (saturates, never panics).
fn slot_u8(slot: usize) -> u8 {
    u8::try_from(slot).unwrap_or(u8::MAX)
}

/// Deterministic percept lattice from integers plus one `sqrt`, the same
/// shape the arena feeds its brains. All coordinates stay finite.
fn scripted_percept(seed: u64, slot: u8, tick: u32) -> Percept {
    let a = (tick as u64)
        .wrapping_mul(7)
        .wrapping_add(slot as u64 * 13)
        .wrapping_add(seed);
    let lane = |div: u64| (a / div % 41) as f32 - 20.0;
    let (x, z, fx, fz) = (lane(1), lane(41), lane(1_681), lane(68_921));
    let (dx, dz) = (fx - x, fz - z);
    Percept::new(
        [x, 0.0, z],
        0.0,
        [fx, 0.0, fz],
        (tick + u32::from(slot)).is_multiple_of(9),
        (dx * dx + dz * dz).sqrt(),
        tick,
    )
}

#[cfg(test)]
mod slot_tests {
    use super::*;

    #[test]
    fn slot_u8_saturates_large_slot() {
        assert_eq!(slot_u8(0), 0);
        assert_eq!(slot_u8(15), 15);
        assert_eq!(slot_u8(255), 255);
        assert_eq!(slot_u8(256), 255);
        assert_eq!(slot_u8(300), 255);
        assert_eq!(slot_u8(usize::MAX), u8::MAX);
    }
}

/// Bonded crate prop under a fixed sword-hit schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DestructionStep {
    /// Bake seed for the crate prop.
    pub seed: u64,
    /// Fixed ticks per run.
    pub ticks: u32,
}

impl DestructionStep {
    /// Workload name used in baselines and budget errors.
    pub const NAME: &'static str = "destruction_step";

    /// Impact every N ticks, mirroring the arena sword-hit formula.
    pub const IMPACT_EVERY: u32 = 40;

    /// Canonical workload: 8-piece crate, 600 ticks, 15 hits.
    pub const fn canonical() -> Self {
        Self {
            seed: 5_150,
            ticks: 600,
        }
    }

    /// Same shape with a different seed (regression proofs).
    pub const fn with_seed(self, seed: u64) -> Self {
        Self { seed, ..self }
    }

    /// Canonical hash: FNV over the chunk count, every tick's state hash and
    /// every drained break event's canonical bytes.
    pub fn run_hash(&self) -> u64 {
        // Fixed descriptor: valid by construction (8 pieces, positive half).
        let mut prop = BondedCollection::crate_prop(0.0, 0.5, 0.0, 0.5, 8, self.seed)
            .expect("canonical crate descriptor is valid");
        let mut hash = FNV64_OFFSET;
        mix(&mut hash, prop.len() as u64);
        for tick in 0..self.ticks {
            if tick % Self::IMPACT_EVERY == 0 {
                let chunk = (tick / Self::IMPACT_EVERY) as usize % prop.len().max(1);
                if let Some(mass) = prop.chunk_mass(chunk) {
                    // Same over-threshold formula the arena uses for sword hits.
                    let impulse = mass * prop.break_threshold() + 1.0;
                    let _ = prop.apply_impact(chunk, impulse, tick);
                }
            }
            prop.step(DT_S);
            mix(&mut hash, prop.state_hash());
            for event in prop.drain_events() {
                for byte in event_bytes(&event) {
                    mix(&mut hash, u64::from(byte));
                }
            }
        }
        hash
    }

    /// Canonical run plus its allocation totals.
    pub fn run_report(&self) -> WorkloadReport {
        measured_report(Self::NAME, self.ticks, || self.run_hash())
    }
}

/// Seeded 16-slot spine both spine bars step: slot-derived lanes in
/// [-20, 20) meters (the [`SpatialQuery`] lane shape), the frozen capsule
/// size, full-health vitals. Single owner of the spine setup in this file.
fn seeded_spine(seed: u64) -> Spine {
    let mut spine = Spine::with_capacity(MAX_SPINE_SLOTS);
    let live = fill_spine(&mut spine, seed);
    debug_assert_eq!(live, MAX_SPINE_SLOTS, "spine reserves every slot at build");
    spine
}

/// Spawn every slot of `spine` with the seeded lanes; stops at the first
/// refusal and returns the live slots. A refusal is data (`Full`), never a
/// panic: the fold below stays deterministic on whatever is live, and the
/// canonical-hash tests refuse a short spine loudly via hash mismatch.
fn fill_spine(spine: &mut Spine, seed: u64) -> usize {
    let mut live = 0;
    for slot in 0..MAX_SPINE_SLOTS {
        let lane = |k: u64| {
            let v = seed
                .wrapping_mul(7)
                .wrapping_add(slot as u64 * 13)
                .wrapping_add(k * 101)
                % 4000;
            v as f32 / 100.0 - 20.0
        };
        let spawned = spine.spawn(
            Transform {
                pos: [lane(1), 0.0, lane(2)],
                yaw: lane(3) * 0.05,
            },
            SpineBody {
                vel: [Fixed::ZERO; 2],
                radius: Fixed(26_214),
                height: Fixed(117_965),
            },
            Vitals {
                hp_permille: 1000 - slot as u16 * 10,
                stamina: 100,
                poise: 0,
                state: 0,
            },
        );
        if spawned.is_err() {
            break;
        }
        live += 1;
    }
    live
}

/// One transform-plus-body pass over every live slot (the Motion-column
/// writes): pose advances, the planar delta lands in the body column. One
/// pass, no allocation; the seed rides the body lanes so a reseeded run
/// writes different columns, not just a different start.
fn step_transform_body(spine: &mut Spine, pass: u32, seed: u64) {
    let drift = (seed % 1000) as i64;
    spine.for_each_mut(|slot, cell| {
        let n = i64::from(pass.wrapping_add(slot as u32));
        cell.transform.pos[0] += 0.01;
        cell.transform.pos[2] -= 0.005;
        cell.transform.yaw += 0.001;
        cell.body.vel = [
            Fixed::from_bits(n.wrapping_mul(655).wrapping_add(drift)),
            Fixed::from_bits(n.wrapping_mul(-331).wrapping_add(drift)),
        ];
    });
}

/// One full-tick pass over every live slot (all five columns, the shape the
/// ARC step writes per tick): pose, body, vitals, intent and gait. One pass,
/// no allocation; the pose accumulates from the seeded base so a reseeded
/// run folds a different history.
fn step_full_tick(spine: &mut Spine, tick: u32) {
    spine.for_each_mut(|slot, cell| {
        cell.transform.pos[0] += 0.01;
        cell.transform.yaw += 0.001;
        cell.body.vel = [Fixed::from_bits(655), Fixed::from_bits(-655)];
        cell.vitals.stamina = cell.vitals.stamina.saturating_sub(1);
        cell.vitals.state = Vitals::ATTACKING;
        cell.ai.intent.mx = slot as i8;
        cell.ai.clock = tick;
        cell.ai.source = AiState::FROM_TAPE;
        cell.anim.phase += 0.05;
        cell.anim.loco = 2;
    });
}

/// Transform-plus-body bar (OPT14): 10k entity-steps of the Motion-column
/// writes with a streaming digest fold per pass. Ticks counts spine passes;
/// entities covered are slots times passes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpineTransformBody {
    /// Seed for the spine lanes and body drift.
    pub seed: u64,
    /// Passes over the full 16-slot spine per canonical run.
    pub passes: u32,
}

impl SpineTransformBody {
    /// Workload name used in baselines and budget errors.
    pub const NAME: &'static str = "spine_transform_body";

    /// Entity-steps per canonical run: every pass steps all 16 slots.
    pub const ENTITIES: usize = 10_000;

    /// Canonical workload: seed 2040, 625 passes of 16 slots.
    pub const fn canonical() -> Self {
        Self {
            seed: 2_040,
            passes: 625,
        }
    }

    /// Same shape with a different seed (regression proofs).
    pub const fn with_seed(self, seed: u64) -> Self {
        Self { seed, ..self }
    }

    /// Canonical hash: FNV over the entity count plus every pass's
    /// transform and body digests. The count folds in first, so a rejected
    /// shape (empty run) can never match a recorded baseline.
    pub fn run_hash(&self) -> u64 {
        let mut spine = seeded_spine(self.seed);
        let mut hash = FNV64_OFFSET;
        mix(&mut hash, Self::ENTITIES as u64);
        for pass in 0..self.passes {
            step_transform_body(&mut spine, pass, self.seed);
            let digests = spine.column_digests();
            mix(&mut hash, digests[0]);
            mix(&mut hash, digests[1]);
        }
        hash
    }

    /// Canonical run plus its allocation totals.
    pub fn run_report(&self) -> WorkloadReport {
        measured_report(Self::NAME, self.passes, || self.run_hash())
    }

    /// Steady-state slice: hash plus allocations over `measure_passes`
    /// passes on a warmed spine. Allocation-free by construction (the OPT14
    /// zero-alloc bar); the test asserts it.
    pub fn steady(&self, measure_passes: u32) -> (u64, AllocSnapshot) {
        let mut spine = seeded_spine(self.seed);
        for pass in 0..12 {
            step_transform_body(&mut spine, pass, self.seed);
        }
        alloc::measure(|| {
            let mut hash = FNV64_OFFSET;
            for pass in 0..measure_passes {
                step_transform_body(&mut spine, pass, self.seed);
                let digests = spine.column_digests();
                mix(&mut hash, digests[0]);
                mix(&mut hash, digests[1]);
            }
            hash
        })
    }
}

/// Full-tick bar (OPT14): 1k agent-ticks of all-five-column writes with a
/// streaming digest fold per pass. Ticks counts spine passes; agents covered
/// are slots times passes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpineFullTick {
    /// Seed for the spine lanes.
    pub seed: u64,
    /// Passes over the full 16-slot spine per canonical run.
    pub passes: u32,
}

impl SpineFullTick {
    /// Workload name used in baselines and budget errors.
    pub const NAME: &'static str = "spine_full_tick";

    /// Agent-ticks per canonical run: every pass ticks all 16 slots.
    pub const AGENTS: usize = 1_024;

    /// Canonical workload: seed 2040, 64 passes of 16 slots.
    pub const fn canonical() -> Self {
        Self {
            seed: 2_040,
            passes: 64,
        }
    }

    /// Same shape with a different seed (regression proofs).
    pub const fn with_seed(self, seed: u64) -> Self {
        Self { seed, ..self }
    }

    /// Canonical hash: FNV over the agent count plus every pass's five
    /// column digests. The count folds in first, so a rejected shape (empty
    /// run) can never match a recorded baseline.
    pub fn run_hash(&self) -> u64 {
        let mut spine = seeded_spine(self.seed);
        let mut hash = FNV64_OFFSET;
        mix(&mut hash, Self::AGENTS as u64);
        for pass in 0..self.passes {
            step_full_tick(&mut spine, pass);
            let digests = spine.column_digests();
            for digest in digests {
                mix(&mut hash, digest);
            }
        }
        hash
    }

    /// Canonical run plus its allocation totals.
    pub fn run_report(&self) -> WorkloadReport {
        measured_report(Self::NAME, self.passes, || self.run_hash())
    }

    /// Steady-state slice: hash plus allocations over `measure_passes`
    /// passes on a warmed spine. Allocation-free by construction (the OPT14
    /// zero-alloc bar); the test asserts it.
    pub fn steady(&self, measure_passes: u32) -> (u64, AllocSnapshot) {
        let mut spine = seeded_spine(self.seed);
        for pass in 0..12 {
            step_full_tick(&mut spine, pass);
        }
        alloc::measure(|| {
            let mut hash = FNV64_OFFSET;
            for pass in 0..measure_passes {
                step_full_tick(&mut spine, pass);
                let digests = spine.column_digests();
                for digest in digests {
                    mix(&mut hash, digest);
                }
            }
            hash
        })
    }
}

/// Render cull keys over arena scratch: the draw-order consumer of the
/// frame arena. Each frame streams [`CULL_KEYS`] seeded depth keys into stack
/// scratch, sorts them (the render-style transient compute), stages the
/// order as arena bytes and decodes the staged bytes into a reused
/// [`ForwardPlus`] draw list, whose hash folds the frame. The arena plus the
/// draw list are presized once at setup and warmed before the window, so
/// steady-state frames allocate nothing.
///
/// Consumer: this workload (the perf harness staging the render cull); data
/// owner: the frame's `ForwardPlus` draw list. Units: frames for runs, keys
/// for the draw list, bytes for the presize. Determinism: the seeded key
/// stream plus the integer sort plus the FNV fold repeat exactly; wall clock
/// never enters. Errors: [`FrameError::Overflow`] when the arena was not
/// presized for [`FRAME_BYTES`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderCull {
    /// Stream seed for the key contents.
    pub seed: u64,
    /// Frames per [`RenderCull::steady`] run.
    pub frames: u32,
}

impl RenderCull {
    /// Workload name for reports and budget errors.
    pub const NAME: &'static str = "render_cull";

    /// Canonical workload: seed 7, 120 frames of arena-backed cull keys.
    pub const fn canonical() -> Self {
        Self {
            seed: 7,
            frames: 120,
        }
    }

    /// Same shape with a different seed (regression proofs).
    pub const fn with_seed(self, seed: u64) -> Self {
        Self { seed, ..self }
    }

    /// Cull view for one run: 1080p at the origin, matching the headless
    /// tile grid. Allocates its tile grid and draw-list capacity once, at
    /// setup; steady-state frames on top of it never allocate.
    fn forward() -> ForwardPlus {
        let mut forward = ForwardPlus::new(1920, 1080, 0.0, 0.0);
        forward.draw.reserve(CULL_KEYS);
        forward
    }

    /// Run one frame on `arena` and `forward`: reset, stream the cull keys,
    /// sort them in stack scratch (the render-style transient compute), stage
    /// the order as arena bytes, decode the staged bytes into the reused
    /// draw list and fold the frame. Both scratch owners must already be
    /// presized; setup owns the presize, frames never grow.
    ///
    /// # Errors
    ///
    /// [`FrameError::Overflow`] when the arena was not presized.
    pub fn run_frame(
        &self,
        arena: &mut FrameArena,
        forward: &mut ForwardPlus,
        frame: u32,
    ) -> Result<u64, FrameError> {
        arena.reset();
        let mut stream = Rng::new(self.seed).fork(u64::from(frame));
        let mut keys = [0u32; CULL_KEYS];
        for key in keys.iter_mut() {
            *key = stream.next_u32();
        }
        keys.sort_unstable();
        // Stage the sorted order as little-endian arena bytes, then decode
        // the staged bytes into the draw list: the render order provably
        // round-trips through the arena. The draw list keeps its setup
        // capacity across `clear`, so the steady state allocates nothing.
        let staged = arena.push_words(&keys)?;
        forward.draw.clear();
        forward.draw.extend(
            staged
                .chunks_exact(4)
                .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]])),
        );
        let mut hash = FNV64_OFFSET;
        mix(&mut hash, u64::from(frame));
        mix(&mut hash, forward.hash());
        Ok(hash)
    }

    /// Steady-state proof: a warmed, presized arena plus draw list runs every
    /// frame inside the counting window and must allocate nothing. Returns
    /// the folded frame hashes plus the window's allocation totals (both
    /// zero on pass).
    ///
    /// # Errors
    ///
    /// [`FrameError::Overflow`] when [`FRAME_BYTES`] stops fitting the keys
    /// (a shape change without a presize bump: fix the constant).
    pub fn steady(&self) -> Result<(u64, AllocSnapshot), FrameError> {
        let mut arena = FrameArena::with_capacity_bytes(FRAME_BYTES);
        let mut forward = Self::forward();
        for frame in 0..self.frames {
            self.run_frame(&mut arena, &mut forward, frame)?;
        }
        let (outcome, observed) = alloc::measure(|| {
            let mut hash = FNV64_OFFSET;
            let mut refusal: Option<FrameError> = None;
            for frame in 0..self.frames {
                match self.run_frame(&mut arena, &mut forward, frame) {
                    Ok(frame_hash) => mix(&mut hash, frame_hash),
                    Err(error) => {
                        refusal = Some(error);
                        break;
                    }
                }
            }
            match refusal {
                Some(error) => Err(error),
                None => Ok(hash),
            }
        });
        Ok((outcome?, observed))
    }
}

/// Fighter counts in the scaled arena matrix: 4/8/16-fighter canonical
/// runs beside the 16-slot [`ArenaTick`] default. Counts are fixed so every
/// row has a recorded baseline; unlisted counts have no row and no report.
pub const ARENA_MATRIX_COUNTS: [u8; 3] = [4, 8, 16];
/// Scaled arena tick: the [`ArenaTick`] canonical shape with an explicit
/// fighter count, so per-count hashes and allocation budgets are recorded
/// separately. Consumer: the per-machine baseline table plus the perf
/// report. Units: fighters, ticks, meters. Determinism: same thread-local
/// fold as [`ArenaTick`]; wall clock never enters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScaledArenaTick {
    /// Match seed, expanded per the arena fixture rules.
    pub seed: u64,
    /// Fixed ticks per run.
    pub ticks: u32,
    /// Arena half-extent in meters.
    pub half_extent_m: f32,
    /// Fighters, one of [`ARENA_MATRIX_COUNTS`].
    pub fighters: u8,
}

impl ScaledArenaTick {
    /// Workload name for the 4-fighter canonical run.
    pub const NAME_4: &'static str = "arena_tick_4";
    /// Workload name for the 8-fighter canonical run.
    pub const NAME_8: &'static str = "arena_tick_8";
    /// Workload name for the 16-fighter canonical run.
    pub const NAME_16: &'static str = "arena_tick_16";
    /// Workload name for the 4-fighter run on the `sim` track.
    pub const NAME_4_SIM: &'static str = "arena_tick_4_sim";
    /// Workload name for the 8-fighter run on the `sim` track.
    pub const NAME_8_SIM: &'static str = "arena_tick_8_sim";
    /// Workload name for the 16-fighter run on the `sim` track.
    pub const NAME_16_SIM: &'static str = "arena_tick_16_sim";

    /// Canonical scaled run: seed 1, 600 ticks, 20 m half-extent, the same
    /// shape as [`ArenaTick::canonical`]. `None` unless `fighters` is one of
    /// [`ARENA_MATRIX_COUNTS`], so every instance has a baseline row.
    pub const fn canonical_for(fighters: u8) -> Option<Self> {
        match fighters {
            4 | 8 | 16 => Some(Self {
                seed: 1,
                ticks: 600,
                half_extent_m: 20.0,
                fighters,
            }),
            _ => None,
        }
    }

    /// Same shape with a different seed (regression proofs).
    pub const fn with_seed(self, seed: u64) -> Self {
        Self { seed, ..self }
    }

    /// Workload name for this count, matching `baselines`.
    pub const fn workload_name(&self) -> &'static str {
        match self.fighters {
            4 => Self::NAME_4,
            8 => Self::NAME_8,
            _ => Self::NAME_16,
        }
    }

    /// Workload name for this count on the compiled track: checkpoint hashes
    /// carry a track tag byte, so `sim` builds need their own baseline row.
    pub const fn tracked_name(&self) -> &'static str {
        match (self.fighters, engine_arena::SIM_TRACK) {
            (4, false) => Self::NAME_4,
            (8, false) => Self::NAME_8,
            (_, false) => Self::NAME_16,
            (4, true) => Self::NAME_4_SIM,
            (8, true) => Self::NAME_8_SIM,
            (_, true) => Self::NAME_16_SIM,
        }
    }

    /// Match config for this workload: `fighters` explicit slots.
    pub fn config(&self) -> ArenaConfig {
        arena_config(self.seed, self.ticks, self.fighters, self.half_extent_m)
    }

    /// Canonical hash: the shared arena fold over this count's checkpoints.
    /// The 16-fighter run matches [`ArenaTick::canonical`] exactly (same
    /// config, same fold); smaller counts diverge by design.
    pub fn run_hash(&self) -> u64 {
        arena_run_hash(&self.config())
    }

    /// Canonical run plus its allocation totals under this count's
    /// track-aware name.
    pub fn run_report(&self) -> WorkloadReport {
        measured_report(self.tracked_name(), self.ticks, || self.run_hash())
    }
}

/// Seeded XZ radius query over structure-of-arrays points: the measured
/// spatial hot loop. Consumer: the perf harness timing below plus the
/// integration test beside it. Data owner: caller-owned slices (setup builds
/// the lanes once, queries reuse them). Units: meters for positions and the
/// radius, indices for hits. Determinism: the seeded build plus the
/// index-order scan repeat exactly; wall clock only enters the millisecond
/// evidence pair, never the hit sets. Errors: none (short inputs truncate to
/// the shared prefix; no panic, no allocation).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpatialQuery {
    /// Stream seed for point placement.
    pub seed: u64,
    /// Point count (both lanes of the structure-of-arrays).
    pub points: usize,
    /// Query radius in meters.
    pub radius_m: f32,
    /// Query X in meters.
    pub qx: f32,
    /// Query Z in meters.
    pub qz: f32,
}

impl SpatialQuery {
    /// Workload name for reports and budget errors.
    pub const NAME: &'static str = "spatial_query";

    /// Canonical workload: seed 11, 4096 points over a 40 m square, 5 m
    /// radius around a fixed query point.
    pub const fn canonical() -> Self {
        Self {
            seed: 11,
            points: 4096,
            radius_m: 5.0,
            qx: 2.0,
            qz: -1.0,
        }
    }

    /// Same shape with a different seed (regression proofs).
    pub const fn with_seed(self, seed: u64) -> Self {
        Self { seed, ..self }
    }

    /// Squared radius for the distance compare (one multiply, no root).
    pub fn radius_sq(&self) -> f32 {
        self.radius_m * self.radius_m
    }

    /// Seeded point lanes in [-20, 20) meters, finite by construction.
    /// Allocates once at setup; steady-state queries reuse these lanes.
    pub fn build_points(&self) -> (Vec<f32>, Vec<f32>) {
        let mut rng = Rng::new(self.seed);
        let lane = |rng: &mut Rng| {
            let v = u16::try_from(rng.next_u32() % 4000).unwrap_or(3_999);
            f32::from(v) / 100.0 - 20.0
        };
        let mut xs = Vec::with_capacity(self.points);
        let mut zs = Vec::with_capacity(self.points);
        for _ in 0..self.points {
            xs.push(lane(&mut rng));
            zs.push(lane(&mut rng));
        }
        (xs, zs)
    }
}

/// Scalar fallback: one distance-squared compare per point in index order.
/// Writes hit indices into `out` and returns the hit count. Truncates to the
/// shared prefix of the three slices, so short scratch never panics.
pub fn spatial_query_scalar(
    xs: &[f32],
    zs: &[f32],
    qx: f32,
    qz: f32,
    radius_sq: f32,
    out: &mut [u32],
) -> usize {
    let n = xs.len().min(zs.len()).min(out.len());
    let mut count = 0;
    let mut i = 0;
    while i < n {
        let dx = xs[i] - qx;
        let dz = zs[i] - qz;
        if dx * dx + dz * dz <= radius_sq {
            out[count] = u32::try_from(i).unwrap_or(u32::MAX);
            count += 1;
        }
        i += 1;
    }
    count
}

/// Vector-shaped path: four-wide unrolled with the same per-point op order
/// as the scalar fallback (the layout a glam vector batch uses), plus a
/// scalar tail for the remainder. The compiler fuses each group of four into
/// SIMD lanes; the tail keeps every length bit-identical. Writes hit indices
/// into `out` in index order and returns the hit count, truncating to the
/// shared prefix like the fallback. This file stays std-only with no manifest
/// change; typing the same shape with the glam crate is a follow-up.
pub fn spatial_query_simd(
    xs: &[f32],
    zs: &[f32],
    qx: f32,
    qz: f32,
    radius_sq: f32,
    out: &mut [u32],
) -> usize {
    let n = xs.len().min(zs.len()).min(out.len());
    let mut count = 0;
    let mut i = 0;
    while i + 4 <= n {
        let dx0 = xs[i] - qx;
        let dz0 = zs[i] - qz;
        if dx0 * dx0 + dz0 * dz0 <= radius_sq {
            out[count] = u32::try_from(i).unwrap_or(u32::MAX);
            count += 1;
        }
        let dx1 = xs[i + 1] - qx;
        let dz1 = zs[i + 1] - qz;
        if dx1 * dx1 + dz1 * dz1 <= radius_sq {
            out[count] = u32::try_from(i + 1).unwrap_or(u32::MAX);
            count += 1;
        }
        let dx2 = xs[i + 2] - qx;
        let dz2 = zs[i + 2] - qz;
        if dx2 * dx2 + dz2 * dz2 <= radius_sq {
            out[count] = u32::try_from(i + 2).unwrap_or(u32::MAX);
            count += 1;
        }
        let dx3 = xs[i + 3] - qx;
        let dz3 = zs[i + 3] - qz;
        if dx3 * dx3 + dz3 * dz3 <= radius_sq {
            out[count] = u32::try_from(i + 3).unwrap_or(u32::MAX);
            count += 1;
        }
        i += 4;
    }
    while i < n {
        let dx = xs[i] - qx;
        let dz = zs[i] - qz;
        if dx * dx + dz * dz <= radius_sq {
            out[count] = u32::try_from(i).unwrap_or(u32::MAX);
            count += 1;
        }
        i += 1;
    }
    count
}

/// A/B timing of the two query paths in milliseconds (wall-clock evidence
/// only; pass or fail never depends on it). Both paths run `iters` passes
/// over caller-owned scratch with the optimizer sink engaged, scalar first.
pub fn spatial_query_ab(
    query: &SpatialQuery,
    xs: &[f32],
    zs: &[f32],
    scratch_a: &mut [u32],
    scratch_b: &mut [u32],
    iters: u32,
) -> (f64, f64) {
    let radius_sq = query.radius_sq();
    let start = std::time::Instant::now();
    for _ in 0..iters {
        std::hint::black_box(spatial_query_scalar(
            xs, zs, query.qx, query.qz, radius_sq, scratch_a,
        ));
    }
    let scalar_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = std::time::Instant::now();
    for _ in 0..iters {
        std::hint::black_box(spatial_query_simd(
            xs, zs, query.qx, query.qz, radius_sq, scratch_b,
        ));
    }
    let simd_ms = start.elapsed().as_secs_f64() * 1000.0;
    (scalar_ms, simd_ms)
}

#[cfg(test)]
mod matrix_tests {
    use super::*;

    /// Seed tweak that provably differs (high bits flip, never the legacy
    /// low-bit forcing).
    const SEED_TWEAK: u64 = 0x9E37_79B9;

    #[test]
    fn arena_config_is_the_single_owner() {
        // Single owner for every arena match config in this file: both
        // `ArenaTick::config` and `ScaledArenaTick::config` delegate here,
        // so the ratchet sees one body, not copies of the literal.
        let owned = arena_config(1, 600, 8, 20.0);
        assert_eq!(owned.seed, 1);
        assert_eq!(owned.ticks, 600);
        assert_eq!(owned.fighters, 8);
        assert_eq!(owned.half_extent_m, 20.0);
        assert_eq!(owned.table_version, 1);
        let scaled = ScaledArenaTick::canonical_for(8).expect("8 listed");
        assert_eq!(scaled.config(), owned);
        assert_eq!(
            ArenaTick::canonical().config(),
            arena_config(1, 600, MAX_PARTICIPANTS, 20.0)
        );
    }

    #[test]
    fn matrix_counts_are_listed_once() {
        assert_eq!(ARENA_MATRIX_COUNTS, [4, 8, 16]);
        for count in ARENA_MATRIX_COUNTS {
            let workload = ScaledArenaTick::canonical_for(count).expect("matrix count listed");
            assert_eq!(workload.fighters, count);
            assert_eq!(workload.ticks, 600);
        }
    }

    #[test]
    fn unlisted_counts_have_no_canonical_run() {
        for count in [0, 1, 2, 3, 5, 7, 9, 15, 17, u8::MAX] {
            assert!(
                ScaledArenaTick::canonical_for(count).is_none(),
                "{count} has no baseline row"
            );
        }
    }

    #[test]
    fn scaled_sixteen_matches_the_sixteen_slot_default() {
        let scaled = ScaledArenaTick::canonical_for(16).expect("16 listed");
        assert_eq!(scaled.run_hash(), ArenaTick::canonical().run_hash());
        assert_eq!(scaled.run_report(), {
            let default = ArenaTick::canonical().run_report();
            WorkloadReport {
                workload: scaled.tracked_name(),
                ..default
            }
        });
    }

    #[test]
    fn every_count_hashes_identically_twice() {
        for count in ARENA_MATRIX_COUNTS {
            let workload = ScaledArenaTick::canonical_for(count).expect("matrix count listed");
            assert_eq!(workload.run_hash(), workload.run_hash(), "{count} repeats");
            assert_eq!(
                workload.run_report(),
                workload.run_report(),
                "{count} allocates identically"
            );
        }
    }

    #[test]
    fn every_count_matches_its_baseline_row() {
        for count in ARENA_MATRIX_COUNTS {
            let workload = ScaledArenaTick::canonical_for(count).expect("matrix count listed");
            let report = workload.run_report();
            let recorded = crate::baselines::scaled_arena_baseline(count)
                .expect("one baseline row per count per track");
            assert_eq!(report.workload, recorded.workload, "{count} names its row");
            crate::budget::check_report(&report, &recorded)
                .expect("scaled run matches its baseline");
        }
    }

    #[test]
    fn seeded_hash_regression_trips_every_scaled_gate() {
        for count in ARENA_MATRIX_COUNTS {
            let workload = ScaledArenaTick::canonical_for(count)
                .expect("matrix count listed")
                .with_seed(1 ^ SEED_TWEAK);
            let report = workload.run_report();
            let recorded = crate::baselines::scaled_arena_baseline(count)
                .expect("one baseline row per count per track");
            assert_eq!(report.ticks, recorded.ticks, "{count} keeps its ticks");
            assert_ne!(
                report.hash, recorded.hash,
                "{count} hash moves with the seed"
            );
            let error =
                crate::budget::check_report(&report, &recorded).expect_err("scaled gate must trip");
            assert!(
                matches!(
                    error,
                    crate::budget::BudgetError::HashMismatch {
                        workload,
                        expected,
                        actual,
                    } if workload == report.workload
                        && expected == recorded.hash
                        && actual == report.hash
                ),
                "{count} trips HashMismatch, got {error}"
            );
        }
    }

    #[test]
    fn seeded_allocation_trips_the_scaled_gate() {
        let workload = ScaledArenaTick::canonical_for(8).expect("8 listed");
        let recorded = crate::baselines::scaled_arena_baseline(8).expect("8-fighter row recorded");
        let (hash, observed) = alloc::measure(|| {
            let hash = workload.run_hash();
            // black_box keeps the optimizer from eliding the sink in release.
            std::hint::black_box(vec![0u8; 64]);
            hash
        });
        assert_eq!(
            hash, recorded.hash,
            "simulation bytes untouched by the injection"
        );
        let report = WorkloadReport {
            workload: workload.tracked_name(),
            ticks: workload.ticks,
            hash,
            alloc_bytes: observed.bytes,
            alloc_count: observed.count,
        };
        assert!(
            report.alloc_bytes > recorded.alloc_bytes,
            "injection really allocates: {} vs budget {}",
            report.alloc_bytes,
            recorded.alloc_bytes
        );
        let error =
            crate::budget::check_report(&report, &recorded).expect_err("alloc gate must trip");
        assert!(
            matches!(
                error,
                crate::budget::BudgetError::AllocBytes { workload, .. }
                if workload == report.workload
            ),
            "injection trips AllocBytes, got {error}"
        );
    }

    #[test]
    fn steady_resim_slice_is_bounded_and_repeats() {
        // Optimize line 6 (arena_tick steady allocs): the resim path keeps
        // the match track live, so event ticks stage owned exchange/sweep
        // rows. Pin the steady-state cost of a 300-tick slice (prefix 600
        // minus prefix 300, same config so setup cancels out): it repeats
        // exactly and stays within its budgets.
        let tape = FighterTape(Vec::new());
        let config = ArenaConfig {
            seed: 1,
            ticks: 600,
            fighters: 8,
            half_extent_m: 20.0,
            table_version: 1,
        };
        let measured = || {
            let mut slices = Vec::with_capacity(2);
            for prefix in [300_u32, 600] {
                let ((_, checkpoints), snap) = alloc::measure(|| {
                    engine_arena::run_prefix(&config, &tape, prefix).expect("canonical prefix runs")
                });
                std::hint::black_box(fold_checkpoints(&checkpoints));
                slices.push((snap.bytes, snap.count));
            }
            slices
        };
        let first = measured();
        let second = measured();
        assert_eq!(first, second, "steady slice replays exactly");
        let (bytes, count) = (first[1].0 - first[0].0, first[1].1 - first[0].1);
        assert!(
            bytes <= 1_024,
            "steady 300-tick slice holds its byte budget: {bytes}"
        );
        assert!(
            count <= 96,
            "steady 300-tick slice holds its alloc budget: {count}"
        );
    }
}

#[cfg(test)]
mod spine_tests {
    use super::*;

    /// Seed tweak that provably differs (high bits flip, never the legacy
    /// low-bit forcing).
    const SEED_TWEAK: u64 = 0x9E37_79B9;

    #[test]
    fn canonical_shapes_cover_the_opt14_bars() {
        assert_eq!(MAX_SPINE_SLOTS, 16);
        let body = SpineTransformBody::canonical();
        assert_eq!(body.passes, 625);
        assert_eq!(
            SpineTransformBody::ENTITIES,
            MAX_SPINE_SLOTS * (body.passes as usize)
        );
        let tick = SpineFullTick::canonical();
        assert_eq!(tick.passes, 64);
        assert_eq!(
            SpineFullTick::AGENTS,
            MAX_SPINE_SLOTS * (tick.passes as usize)
        );
    }

    #[test]
    fn both_hash_and_report_repeat_x2() {
        let body = SpineTransformBody::canonical();
        assert_eq!(body.run_hash(), body.run_hash(), "transform-body repeats");
        assert_eq!(
            body.run_report(),
            body.run_report(),
            "transform-body allocates identically"
        );
        let tick = SpineFullTick::canonical();
        assert_eq!(tick.run_hash(), tick.run_hash(), "full tick repeats");
        assert_eq!(
            tick.run_report(),
            tick.run_report(),
            "full tick allocates identically"
        );
    }

    #[test]
    fn steady_windows_allocate_nothing_x2() {
        // Calibrate the probe first: a zero reading must mean zero.
        let (_, seen) = alloc::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(seen.count >= 1, "alloc probe is blind");
        for _ in 0..2 {
            let (_, observed) = SpineTransformBody::canonical().steady(625);
            assert_eq!(observed.bytes, 0, "transform-body passes allocate");
            assert_eq!(observed.count, 0, "transform-body passes allocate");
            let (_, observed) = SpineFullTick::canonical().steady(64);
            assert_eq!(observed.bytes, 0, "full-tick passes allocate");
            assert_eq!(observed.count, 0, "full-tick passes allocate");
        }
    }

    #[test]
    fn seeded_runs_move_both_hashes() {
        let body = SpineTransformBody::canonical().with_seed(2_040 ^ SEED_TWEAK);
        assert_ne!(
            body.run_hash(),
            SpineTransformBody::canonical().run_hash(),
            "seed moves transform-body"
        );
        let tick = SpineFullTick::canonical().with_seed(2_040 ^ SEED_TWEAK);
        assert_ne!(
            tick.run_hash(),
            SpineFullTick::canonical().run_hash(),
            "seed moves full tick"
        );
    }

    #[test]
    fn both_match_their_baseline_rows() {
        let body = SpineTransformBody::canonical();
        let report = body.run_report();
        let recorded = crate::baselines::baseline(SpineTransformBody::NAME)
            .expect("spine_transform_body baseline recorded");
        assert_eq!(report.workload, recorded.workload);
        crate::budget::check_report(&report, &recorded)
            .expect("transform-body matches its baseline");
        let tick = SpineFullTick::canonical();
        let report = tick.run_report();
        let recorded = crate::baselines::baseline(SpineFullTick::NAME)
            .expect("spine_full_tick baseline recorded");
        assert_eq!(report.workload, recorded.workload);
        crate::budget::check_report(&report, &recorded).expect("full tick matches its baseline");
    }

    #[test]
    fn full_spine_setup_refuses_without_panicking() {
        // A zero-room spine refuses every spawn with `Full`: the setup
        // helper must surface that refusal as a short fill, never panic.
        let mut spine = Spine::with_capacity(0);
        assert_eq!(fill_spine(&mut spine, 2_040), 0, "no room means no slots");
        assert_eq!(spine.live(), 0, "refused spawns stay dead");
    }

    #[test]
    fn seeded_setup_still_fills_every_slot() {
        // The canonical path keeps all sixteen slots live: same seed,
        // same lanes, same count as the pinned baseline rows.
        let mut spine = Spine::with_capacity(MAX_SPINE_SLOTS);
        assert_eq!(
            fill_spine(&mut spine, 2_040),
            MAX_SPINE_SLOTS,
            "full room means a full spine"
        );
        assert_eq!(spine.live(), MAX_SPINE_SLOTS, "every spawn lands");
    }
}

#[cfg(test)]
mod cull_tests {
    use super::*;

    fn harnessed() -> (FrameArena, ForwardPlus) {
        (
            FrameArena::with_capacity_bytes(FRAME_BYTES),
            RenderCull::forward(),
        )
    }

    #[test]
    fn cull_frames_repeat_and_stage_sorted_keys() {
        let workload = RenderCull::canonical();
        let (mut arena, mut forward) = harnessed();
        let first = workload
            .run_frame(&mut arena, &mut forward, 0)
            .expect("presized scratch fits");
        assert_eq!(forward.draw.len(), CULL_KEYS);
        let mut ordered = forward.draw.clone();
        ordered.sort_unstable();
        assert_eq!(
            forward.draw, ordered,
            "draw list holds the sorted cull keys"
        );
        let (mut arena, mut forward) = harnessed();
        let repeat = workload
            .run_frame(&mut arena, &mut forward, 0)
            .expect("repeat run fits");
        assert_eq!(first, repeat, "same seed and frame hash identically");
    }

    #[test]
    fn seeded_cull_moves_the_hash() {
        let workload = RenderCull::canonical();
        let moved = workload.with_seed(1_000_007);
        let (mut arena, mut forward) = harnessed();
        let base = workload
            .run_frame(&mut arena, &mut forward, 3)
            .expect("canonical fits");
        let (mut arena, mut forward) = harnessed();
        let other = moved
            .run_frame(&mut arena, &mut forward, 3)
            .expect("seeded fits");
        assert_ne!(base, other, "seed moves the cull hash");
    }

    #[test]
    fn empty_arena_refuses_the_cull_loudly() {
        let workload = RenderCull::canonical();
        let mut arena = FrameArena::new();
        let mut forward = RenderCull::forward();
        assert!(
            matches!(
                workload.run_frame(&mut arena, &mut forward, 0),
                Err(FrameError::Overflow { .. })
            ),
            "unpresized arena refuses the cull"
        );
        assert_eq!(arena.used_bytes(), 0, "refused cull moves no cursor");
    }

    #[test]
    fn steady_cull_allocates_nothing_and_repeats() {
        let workload = RenderCull::canonical();
        let (first, snap_first) = workload.steady().expect("presized scratch fits");
        let (second, snap_second) = workload.steady().expect("repeat run fits");
        assert_eq!(first, second, "steady culls hash identically");
        for (label, snap) in [("first", snap_first), ("second", snap_second)] {
            assert_eq!(snap.bytes, 0, "{label} run allocates no bytes");
            assert_eq!(snap.count, 0, "{label} run makes no allocations");
        }
    }
}
