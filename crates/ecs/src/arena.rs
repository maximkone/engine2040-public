//! arena: pre-sized per-phase scratch for tick temporaries.
//!
//! Owner: engine-ecs. Units: entity slots, scratch bytes, phases
//! (`schedule::Phase`). Determinism: index order in, canonical `(id, gen)`
//! order out; no hash maps, no wall clock. Errors: [`ArenaError`]; a tick
//! that outgrows its pre-size refuses instead of reallocating.

use crate::archetype::Entity;
use crate::schedule::Phase;

/// Tick scratch outgrew its pre-size. Only [`PhaseArena::reserve`] grows;
/// every tick path refuses with this instead of reallocating.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArenaError {
    /// Scratch needs `needed` items but holds `capacity`.
    Overflow {
        /// Required item count (entities or bytes, per call docs).
        needed: usize,
        /// Reserved capacity at the call.
        capacity: usize,
    },
}

impl core::fmt::Display for ArenaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Overflow { needed, capacity } => write!(
                f,
                "arena scratch overflow: need {needed} with {capacity} reserved"
            ),
        }
    }
}

impl std::error::Error for ArenaError {}

/// Pre-sized scratch for per-phase ECS temporaries: a kill/query worklist
/// plus a checkpoint byte buffer, both reset at each phase boundary.
///
/// A system calls [`PhaseArena::begin_phase`] once per phase, then fills the
/// scratch through the fallible `try_*` pushes. The pushes check capacity
/// and return [`ArenaError::Overflow`] instead of growing, so the only
/// allocator on the tick path is the setup-time [`PhaseArena::reserve`].
/// The [`World`](crate::archetype::World) batch entry points
/// (`flush_with`, `collect_live_into`, `checkpoint_with`) clear the scratch
/// first and own it for the call: a refused call changes nothing.
///
/// Budget: O(scratch) time per tick, O(1) extra memory beyond the pre-size,
/// zero heap allocation after [`PhaseArena::reserve`].
///
/// Kill rule (pre-size-only): when a tick refuses, reserve more up front.
/// If plain pre-sized reuse ever covers the workload, this arena stays a
/// thin pair of `Vec`s and gains no bump-pointer fast path.
///
/// ```ignore
/// let mut arena = PhaseArena::with_capacity(128, 4096);
/// arena.reserve(128, 4096); // setup only; ticks below never grow
/// arena.begin_phase(Phase::Update);
/// ```
#[derive(Clone, Debug, Default)]
pub struct PhaseArena {
    entities: Vec<Entity>,
    bytes: Vec<u8>,
    phase: Option<Phase>,
}

impl PhaseArena {
    /// Empty arena; [`PhaseArena::reserve`] before tick use.
    pub fn new() -> Self {
        Self::default()
    }

    /// Empty arena with `entity_slots` entity slots and `byte_len` scratch
    /// bytes reserved up front (setup path; may allocate once here).
    pub fn with_capacity(entity_slots: usize, byte_len: usize) -> Self {
        Self {
            entities: Vec::with_capacity(entity_slots),
            bytes: Vec::with_capacity(byte_len),
            phase: None,
        }
    }

    /// Ensure at least `entity_slots` entity slots and `byte_len` scratch
    /// bytes of capacity, keeping existing contents. Totals, not deltas, so
    /// repeat calls are idempotent. The only grower: tick paths never call
    /// this. Budget: O(new capacity) time when it grows, nothing otherwise.
    pub fn reserve(&mut self, entity_slots: usize, byte_len: usize) {
        if self.entities.capacity() < entity_slots {
            self.entities.reserve(entity_slots - self.entities.len());
        }
        if self.bytes.capacity() < byte_len {
            self.bytes.reserve(byte_len - self.bytes.len());
        }
    }

    /// Open `phase`: drop every temporary but keep every capacity, and stamp
    /// the phase for receipts. Clearing never allocates.
    pub fn begin_phase(&mut self, phase: Phase) {
        self.entities.clear();
        self.bytes.clear();
        self.phase = Some(phase);
    }

    /// Drop every temporary and the phase stamp, keeping every capacity.
    /// Clearing never allocates.
    pub fn clear(&mut self) {
        self.entities.clear();
        self.bytes.clear();
        self.phase = None;
    }

    /// Phase opened by [`PhaseArena::begin_phase`], if any.
    pub fn phase(&self) -> Option<Phase> {
        self.phase
    }

    /// True when both scratch buffers hold nothing (capacities untouched).
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty() && self.bytes.is_empty()
    }

    /// Staged entity temporaries in fill order.
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// Staged entity count.
    pub fn entity_len(&self) -> usize {
        self.entities.len()
    }

    /// Reserved entity slots (only [`PhaseArena::reserve`] raises this).
    pub fn entity_capacity(&self) -> usize {
        self.entities.capacity()
    }

    /// Staged scratch bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Staged scratch byte count.
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }

    /// Reserved scratch bytes (only [`PhaseArena::reserve`] raises this).
    pub fn byte_capacity(&self) -> usize {
        self.bytes.capacity()
    }

    /// Stage one entity temporary. Refuses with [`ArenaError::Overflow`]
    /// past the pre-size instead of growing; a refused push changes nothing.
    pub fn try_push_entity(&mut self, entity: Entity) -> Result<(), ArenaError> {
        if self.entities.len() >= self.entities.capacity() {
            return Err(ArenaError::Overflow {
                needed: self.entities.len() + 1,
                capacity: self.entities.capacity(),
            });
        }
        self.entities.push(entity);
        Ok(())
    }

    /// Stage a slice of entity temporaries. Checks the whole slice up front,
    /// so a refusal stages nothing instead of a prefix.
    pub fn try_extend_entities(&mut self, items: &[Entity]) -> Result<(), ArenaError> {
        let needed = self.entities.len() + items.len();
        if needed > self.entities.capacity() {
            return Err(ArenaError::Overflow {
                needed,
                capacity: self.entities.capacity(),
            });
        }
        self.entities.extend_from_slice(items);
        Ok(())
    }

    /// Stage scratch bytes. Checks the whole chunk up front, so a refusal
    /// stages nothing instead of a prefix.
    pub fn try_extend_bytes(&mut self, chunk: &[u8]) -> Result<(), ArenaError> {
        let needed = self.bytes.len() + chunk.len();
        if needed > self.bytes.capacity() {
            return Err(ArenaError::Overflow {
                needed,
                capacity: self.bytes.capacity(),
            });
        }
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }

    /// Drop staged entities, keeping capacity. Tick-batch entry point.
    pub(crate) fn clear_entities(&mut self) {
        self.entities.clear();
    }

    /// Sort staged entities by `(id, gen)` with an in-place unstable sort:
    /// no allocation, and deterministic because staged kills are unique.
    pub(crate) fn sort_entities_by_id(&mut self) {
        self.entities.sort_unstable_by_key(|e| (e.id, e.gen));
    }

    /// Drain staged entities, keeping capacity for the next phase.
    pub(crate) fn drain_entities(&mut self) -> std::vec::Drain<'_, Entity> {
        self.entities.drain(..)
    }

    /// Drop staged bytes, keeping capacity. Tick-batch entry point.
    pub(crate) fn clear_bytes(&mut self) {
        self.bytes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archetype::{Position, Velocity, World};

    /// Deterministic P+V world: `slots` entities on a fixed grid.
    fn seeded_world(slots: usize) -> World {
        let mut world = World::new();
        for i in 0..slots {
            let f = i as f32;
            world.spawn(
                Some(Position { x: f * 0.25, y: -f }),
                Some(Velocity { x: 0.5, y: -0.25 }),
            );
        }
        world
    }

    /// One representative tick: census, one staged kill plus a recycled
    /// spawn (stable population, free-list reuse), integrate, arena flush
    /// and arena checkpoint. Everything must fit the pre-size.
    fn tick_once(world: &mut World, arena: &mut PhaseArena, step: usize) {
        world.advance();
        world
            .collect_live_into(arena)
            .expect("census fits the pre-size");
        let live = arena.entity_len();
        assert!(live > 0, "census must see live entities");
        let victim = arena.entities()[step % live];
        assert!(world.kill(victim), "victim must stage");
        let f = step as f32;
        world.spawn(
            Some(Position {
                x: f * 0.125,
                y: f * -0.0625,
            }),
            Some(Velocity { x: 0.25, y: 0.125 }),
        );
        world.integrate(0.016);
        world.flush_with(arena).expect("kill set fits the pre-size");
        let checkpoint = world
            .checkpoint_with(arena)
            .expect("checkpoint fits the pre-size");
        std::hint::black_box(checkpoint);
    }

    #[test]
    fn tick_reuses_presized_scratch_without_allocating() {
        // Calibration first: the probe must observe a real allocation,
        // otherwise a zero reading would prove nothing.
        let (_, seen) = engine_core::alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(seen.calls >= 1, "alloc probe is blind");
        let mut world = seeded_world(64);
        world.reserve(128);
        let mut arena = PhaseArena::with_capacity(128, 4096);
        arena.reserve(128, 4096);
        // 10-tick warmup absorbs lazy setup; the gate below is steady state.
        for step in 0..10 {
            tick_once(&mut world, &mut arena, step);
        }
        for step in 10..20 {
            let ((), heap) = engine_core::alloc_probe::measure(|| {
                tick_once(&mut world, &mut arena, step);
            });
            assert_eq!(heap.calls, 0, "tick {step} allocated on the hot path");
        }
    }

    /// One scripted run under one path: per-tick checkpoint hashes.
    #[derive(Clone, Copy)]
    enum Path {
        Direct,
        Arena,
    }

    fn run_checkpoints(path: Path) -> Vec<u64> {
        let mut world = seeded_world(32);
        world.reserve(64);
        let mut arena = PhaseArena::with_capacity(64, 2048);
        let mut out = Vec::with_capacity(24);
        for step in 0..24 {
            world.advance();
            world
                .collect_live_into(&mut arena)
                .expect("census fits the pre-size");
            let victim = arena.entities()[step % arena.entity_len()];
            assert!(world.kill(victim), "victim must stage");
            let f = step as f32;
            world.spawn(
                Some(Position { x: f, y: -f }),
                Some(Velocity { x: 0.5, y: 0.25 }),
            );
            world.integrate(0.016);
            let hash = match path {
                Path::Direct => {
                    world.flush();
                    world.state_hash()
                }
                Path::Arena => {
                    world
                        .flush_with(&mut arena)
                        .expect("kill set fits the pre-size");
                    let hash = world
                        .checkpoint_with(&mut arena)
                        .expect("checkpoint fits the pre-size");
                    assert_eq!(
                        hash,
                        world.state_hash(),
                        "byte checkpoint differs on step {step}"
                    );
                    hash
                }
            };
            out.push(hash);
        }
        out
    }

    #[test]
    fn checkpoints_match_the_direct_path() {
        let direct = run_checkpoints(Path::Direct);
        let arena = run_checkpoints(Path::Arena);
        assert_eq!(direct, arena, "arena path moved a checkpoint");
        assert_eq!(
            run_checkpoints(Path::Arena),
            arena,
            "arena path is not deterministic"
        );
        assert!(
            arena.windows(2).any(|w| w[0] != w[1]),
            "script never moved the checkpoint"
        );
    }

    #[test]
    fn overflow_refuses_without_growing() {
        // Pre-size-only kill rule: manual pushes refuse past capacity and a
        // refused call stages nothing; only `reserve` grows.
        let probe = Entity { id: 7, gen: 0 };
        let mut arena = PhaseArena::new();
        assert_eq!(
            arena.try_push_entity(probe).err(),
            Some(ArenaError::Overflow {
                needed: 1,
                capacity: 0
            })
        );
        assert!(arena.is_empty());
        arena.reserve(3, 16);
        let entity_cap = arena.entity_capacity();
        assert!(entity_cap >= 3, "reserve must hold its total");
        for _ in 0..entity_cap {
            arena.try_push_entity(probe).expect("within capacity");
        }
        assert_eq!(
            arena.try_push_entity(probe).err(),
            Some(ArenaError::Overflow {
                needed: entity_cap + 1,
                capacity: entity_cap
            })
        );
        assert_eq!(arena.entity_len(), entity_cap);
        let byte_cap = arena.byte_capacity();
        assert!(byte_cap >= 16, "reserve must hold its total");
        arena
            .try_extend_bytes(&vec![0u8; byte_cap])
            .expect("within capacity");
        assert_eq!(
            arena.try_extend_bytes(&[9]).err(),
            Some(ArenaError::Overflow {
                needed: byte_cap + 1,
                capacity: byte_cap
            })
        );
        assert_eq!(arena.byte_len(), byte_cap);
        // A partial slice stages nothing, not a prefix.
        let mut small = PhaseArena::with_capacity(4, 4);
        small
            .try_extend_entities(&[probe; 3])
            .expect("3 of 4 slots fit");
        assert_eq!(
            small.try_extend_entities(&[probe; 2]).err(),
            Some(ArenaError::Overflow {
                needed: 5,
                capacity: small.entity_capacity()
            })
        );
        assert_eq!(small.entity_len(), 3);
        // Error text names the culprit for operator diagnostics.
        assert!(ArenaError::Overflow {
            needed: 9,
            capacity: 4
        }
        .to_string()
        .contains('9'));
    }

    #[test]
    fn refused_batch_calls_change_nothing() {
        // A flush that outgrows the worklist keeps every staged kill.
        let mut world = seeded_world(4);
        world.reserve(8);
        let mut census = PhaseArena::with_capacity(8, 256);
        world.collect_live_into(&mut census).expect("census fits");
        assert!(world.kill(census.entities()[0]));
        assert!(world.kill(census.entities()[1]));
        assert_eq!(world.pending(), 2);
        let mut tiny = PhaseArena::new();
        assert_eq!(
            world.flush_with(&mut tiny).err(),
            Some(ArenaError::Overflow {
                needed: 2,
                capacity: 0
            })
        );
        assert_eq!(world.pending(), 2, "refused flush must keep staged kills");
        tiny.reserve(2, 0);
        world.flush_with(&mut tiny).expect("reserved flush runs");
        assert_eq!((world.pending(), world.count()), (0, 2));
        // A flush that outgrows the free list refuses before despawning.
        let mut bare = seeded_world(2);
        let mut work = PhaseArena::with_capacity(4, 128);
        bare.collect_live_into(&mut work).expect("census fits");
        assert!(bare.kill(work.entities()[0]));
        assert_eq!(
            bare.flush_with(&mut work).err(),
            Some(ArenaError::Overflow {
                needed: 1,
                capacity: 0
            })
        );
        assert_eq!(bare.pending(), 1, "short free list must refuse first");
        bare.reserve(4);
        bare.flush_with(&mut work)
            .expect("reserved free absorbs the kill");
        assert_eq!(bare.count(), 1);
        // A checkpoint that outgrows the byte scratch leaves it empty.
        let mut bytes_tiny = PhaseArena::with_capacity(4, 0);
        assert!(matches!(
            bare.checkpoint_with(&mut bytes_tiny).err(),
            Some(ArenaError::Overflow { .. })
        ));
        assert_eq!(bytes_tiny.byte_len(), 0);
    }

    #[test]
    fn phase_reset_keeps_capacity_and_stamps_phase() {
        let fresh = PhaseArena::default();
        assert!(fresh.is_empty());
        assert_eq!(fresh.phase(), None);
        let mut arena = PhaseArena::with_capacity(8, 64);
        arena.begin_phase(Phase::PreUpdate);
        assert_eq!(arena.phase(), Some(Phase::PreUpdate));
        let (entity_cap, byte_cap) = (arena.entity_capacity(), arena.byte_capacity());
        let probe = Entity { id: 1, gen: 2 };
        for _ in 0..entity_cap {
            arena.try_push_entity(probe).expect("within capacity");
        }
        arena
            .try_extend_bytes(&vec![0u8; byte_cap])
            .expect("within capacity");
        assert!(!arena.is_empty());
        arena.begin_phase(Phase::Update);
        assert_eq!(arena.phase(), Some(Phase::Update));
        assert!(arena.is_empty());
        assert_eq!(
            (arena.entity_capacity(), arena.byte_capacity()),
            (entity_cap, byte_cap),
            "phase reset must keep every capacity"
        );
        arena.clear();
        assert_eq!(arena.phase(), None);
        assert!(arena.is_empty());
    }
}
