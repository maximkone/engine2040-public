//! physics: fixed-dt semi-implicit Euler bodies + AABB overlap query.

/// Center-of-mass balance core plus stagger response.
pub mod balance;
pub mod broadphase;
/// Named float-to-int conversions for tick paths: checked `TryFrom`
/// entries plus saturating wrappers, shared by broadphase cell math.
pub mod cast;
/// Tick-path integer narrowing on the core checked owner: clamp-then-narrow
/// and Q16 downscale helpers the servo paths adopt next.
pub mod checked_fix;
/// CPU verlet cloth on fixed-point math: bone-anchored, presentation
/// only, never checkpoints. Std-only.
pub mod cloth;
#[cfg(feature = "rapier")]
pub mod controller;
/// Character controller v2: steps, slopes, platforms, ledge sense, pushes.
/// Integer-millimeter, std-only; arena locomotion walks these walkers.
pub mod controller2;
/// Integer Quake-style FPS walk beside controller2: accel, friction, traction.
pub mod fps_move;
/// Grapple servo on opponent ragdoll state: torque caps, breakaway,
/// ID-sorted solve order. Std-only.
pub mod grapple;
pub mod islands;
pub mod islands_build;
pub mod kcc;
/// Severed limbs as bodies: mass from volume and density, inherited velocity
/// plus blade impulse, stump recoil, canonical order, capped, snapshotted.
/// Integer, std-only.
pub mod limb;
/// Physics LOD for armies: far soldiers as capsules, near ones full detail,
/// CCD only for fast weapons and projectiles, island sleep for rest.
pub mod lod;
pub mod overlap;
/// PD velocity-servo drive law for the powered ragdoll: D-damped servo
/// axes, magnitude clamp, dt fallback to P-only. Std-only.
pub mod pd;
#[cfg(feature = "rapier")]
pub mod powered;
pub mod query;
#[cfg(feature = "rapier")]
pub mod ragdoll;
/// Frozen-baseline fall/clinch measurement tape: seeded scripts beside
/// `ragdoll`, read-only on the tick; the fix rows consume its report.
#[cfg(feature = "rapier")]
pub mod ragdoll_tape;
#[cfg(feature = "rapier")]
pub mod rapier;
pub mod rigid;
pub mod solver;
/// Cumulative pain, servo fatigue and per-bone injury beside the balance
/// core: the arena hit pipeline feeds each blow here plus `balance`.
/// Integer, std-only.
pub mod stagger_memory;
pub mod substep;
pub mod vehicle;
pub mod verify;

pub use rigid::Body;

#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: engine_core::alloc_probe::CountingAlloc =
    engine_core::alloc_probe::CountingAlloc;

pub struct Space {
    pub bodies: Vec<Body>,
}

impl Default for Space {
    fn default() -> Self {
        Self::new()
    }
}

impl Space {
    pub fn new() -> Self {
        Self { bodies: vec![] }
    }
    pub fn add(&mut self, b: Body) -> usize {
        self.bodies.push(b);
        self.bodies.len() - 1
    }
    pub fn step(&mut self, dt: f32) {
        rigid::integrate(&mut self.bodies, dt);
    }
    pub fn overlap(&self, x: f32, y: f32, hx: f32, hy: f32) -> Vec<usize> {
        overlap::overlap_query(&self.bodies, x, y, hx, hy)
    }
    pub fn state_hash(&self) -> u64 {
        rigid::state_hash(&self.bodies)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_hits_touching_misses_clear() {
        let mut s = Space::new();
        s.add(Body {
            x: 0.0,
            y: 1.0,
            vx: 0.0,
            vy: 0.0,
            hx: 0.5,
            hy: 0.5,
        });
        assert_eq!(s.overlap(0.4, 1.0, 0.5, 0.5), vec![0]);
        assert!(s.overlap(50.0, 50.0, 1.0, 1.0).is_empty());
    }

    #[test]
    fn ground_clamp_holds_and_hash_stable() {
        let (mut a, mut b) = (Space::new(), Space::new());
        for s in [&mut a, &mut b] {
            s.add(Body {
                x: 0.0,
                y: 5.0,
                vx: 1.0,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            });
            s.step(1.0 / 60.0);
        }
        assert_eq!(a.state_hash(), b.state_hash());
        // fall long enough to hit the ground plane stub
        for _ in 0..600 {
            a.step(1.0 / 60.0);
        }
        assert_eq!(a.bodies[0].y, a.bodies[0].hy);
        assert_eq!(a.bodies[0].vy, 0.0);
    }
}
