//! ecs: two worlds with one checkout rule, plus phases and scratch.
//!
//! Owner: engine-ecs. The legacy [`World`] stays byte-stable for the frozen
//! proof path; the archetype [`archetype::World`] plus [`PhaseArena`] is the
//! tick path driven by [`schedule::Schedule::run_arena_tick`]. New simulation
//! code uses the archetype path; see [`WORLD_FORK_DECISION`] for the board
//! record and why both types stay.

pub mod archetype;
pub mod arena;
pub mod bridge;
pub mod jobs;
pub mod parallel;
pub mod pool;
pub mod schedule;
pub mod snapshot;
pub mod spine;
pub mod spine_schedule;

pub use archetype::{ComponentId, Entity, Registry};
pub use arena::{ArenaError, PhaseArena};
pub use pool::{JobPool, PoolError, Scope};
use engine_core::Fnv64;

// The one counting allocator of this test binary: zero-allocation tests read
// it through `engine_core::alloc_probe::measure`.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: engine_core::alloc_probe::CountingAlloc =
    engine_core::alloc_probe::CountingAlloc;

/// Board record for the two-World fork; the value names the row and why both stay.
///
/// New simulation code uses the archetype path with per-phase scratch.
pub const WORLD_FORK_DECISION: &str = "RC10 R2 keeps both Worlds: legacy crate::World stays byte-stable for the frozen golden proof, ai and perf path; archetype::World plus PhaseArena via Schedule::run_arena_tick is the arena tick path; moving proof, ai and perf waits for a packet owning those crates.";

#[derive(Clone, Copy, Debug, Default)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Velocity {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug)]
pub struct World {
    pub pos: Vec<Position>,
    pub vel: Vec<Velocity>,
    pub alive: Vec<bool>,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    pub fn new() -> Self {
        Self {
            pos: vec![],
            vel: vec![],
            alive: vec![],
        }
    }
    pub fn spawn(&mut self, p: Position, v: Velocity) -> usize {
        let id = self.alive.len();
        self.pos.push(p);
        self.vel.push(v);
        self.alive.push(true);
        id
    }
    pub fn count(&self) -> usize {
        self.alive.iter().filter(|&&a| a).count()
    }
    /// kill: staged despawn (deferred structural op). Never compacts mid-iteration.
    pub fn kill(&mut self, id: usize) -> bool {
        if id < self.alive.len() && self.alive[id] {
            self.alive[id] = false;
            true
        } else {
            false
        }
    }
    /// integrate system: pos += vel * dt
    pub fn integrate(&mut self, dt: f32) {
        for i in 0..self.alive.len() {
            if self.alive[i] {
                self.pos[i].x += self.vel[i].x * dt;
                self.pos[i].y += self.vel[i].y * dt;
            }
        }
    }
    /// deterministic state hash (quantized via raw bits)
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        for i in 0..self.alive.len() {
            if self.alive[i] {
                h.write_f32(self.pos[i].x);
                h.write_f32(self.pos[i].y);
            }
        }
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrate_moves_by_vel_dt() {
        let mut w = World::new();
        w.spawn(Position { x: 0.0, y: 0.0 }, Velocity { x: 2.0, y: 1.0 });
        w.integrate(0.5);
        assert_eq!(w.pos[0].x, 1.0);
        assert_eq!(w.pos[0].y, 0.5);
        assert_eq!(w.count(), 1);
    }

    #[test]
    fn state_hash_deterministic_and_sensitive() {
        let (mut a, mut b) = (World::new(), World::new());
        for w in [&mut a, &mut b] {
            w.spawn(Position { x: 3.0, y: 4.0 }, Velocity { x: 0.0, y: 0.0 });
        }
        assert_eq!(a.state_hash(), b.state_hash());
        b.pos[0].x = 5.0;
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_is_fnv1a_over_live_position_bits() {
        let mut w = World::new();
        w.spawn(Position { x: 3.0, y: 4.0 }, Velocity::default());
        w.spawn(Position { x: -1.5, y: 0.25 }, Velocity::default());
        w.spawn(Position { x: 9.0, y: 9.0 }, Velocity::default());
        assert!(w.kill(1));
        // Oracle: FNV-1a 64 over the little-endian f32 bits of entities 0 and
        // 2 (the dead slot is skipped), computed outside this crate.
        assert_eq!(w.state_hash(), 0x4535_71fa_9f8c_6015);
    }
}
