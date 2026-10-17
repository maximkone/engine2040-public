//! physics: fixed-dt semi-implicit Euler bodies + AABB overlap query.

/// Center-of-mass balance core plus stagger response.
pub mod balance;
/// Presentation cloth for fighter capes and arena banners: CPU Verlet in floats at the fixed
/// 60 Hz step, per-tier cost table, sleeping when still. Never hashed, never read by the sim.
pub mod cloth;
/// Static collision shapes (convex hulls, triangle meshes) and the reader for the cooked
/// collider bytes the asset cook writes. Std-only.
pub mod collider;
#[cfg(feature = "rapier")]
pub mod controller;
/// Character controller v2: steps, slopes, platforms, ledge sense, pushes.
/// Integer-millimeter, std-only; arena locomotion walks these walkers.
pub mod controller2;
/// Canonical island topology plus snapshot bytes the Rapier world checkpoints
/// through. Std-only; no sim step.
pub mod islands;
pub mod kcc;
/// The collision layer table (fighter, prop, debris, sensor, world) as plain integers.
pub mod layers;
/// Severed limbs as bodies: mass from volume and density, inherited velocity
/// plus blade impulse, stump recoil, canonical order, capped, snapshotted.
/// Integer, std-only.
pub mod limb;
/// One table of density, friction, bounce and damping per material (wood, stone,
/// flesh, metal, sand, cloth). Integer rows, std-only.
pub mod material;
pub mod overlap;
/// PD velocity-servo drive law for the powered ragdoll: D-damped servo
/// axes, magnitude clamp, dt fallback to P-only. Std-only.
pub mod pd;
/// Ray casts and swept-sphere casts against cooked static colliders (the door's
/// `physics.raycast` and `physics.shapecast`). Std-only.
pub mod query;
#[cfg(feature = "rapier")]
pub mod powered;
#[cfg(feature = "rapier")]
pub mod ragdoll;
/// Frozen-baseline fall/clinch measurement tape: seeded scripts beside
/// `ragdoll`, read-only on the tick; the fix rows consume its report.
#[cfg(feature = "rapier")]
pub mod ragdoll_tape;
#[cfg(feature = "rapier")]
pub mod rapier;
pub mod rigid;
/// Cumulative pain, servo fatigue and per-bone injury beside the balance
/// core: the arena hit pipeline feeds each blow here plus `balance`.
/// Integer, std-only.
pub mod stagger_memory;

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
