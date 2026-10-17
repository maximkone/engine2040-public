//! blast: explosions that shove bodies and arrows that fly an arc to the authority's hit.

use super::{GroupId, PhysFx};
use crate::check::{self, BLAST_REFERENCE_KG, MAX_COORD, MAX_SPEED};
use crate::error::{Error, Field};
use crate::types::{Blast, Flight, Key, Transform};
use engine_physics::material::Material;
use engine_physics::ragdoll::BoneShape;
use engine_physics::rapier::{FIXED_DT, GRAVITY_Y};
use rapier3d::math::Vector;

/// Closer to the center than this (meters) a body has no outward direction and goes straight up.
const AT_CENTER: f32 = 1.0e-4;

impl PhysFx {
    /// Shoves every body inside the blast's radius straight away from its center and wakes it.
    /// The gain is `impulse / 70 kg` at the center and falls off linearly to nothing at the
    /// radius, whatever the body weighs; a body at the radius or beyond is not touched. Returns
    /// how many bodies were pushed. A bad blast is refused and changes nothing.
    pub fn apply_blast(&mut self, blast: Blast) -> Result<usize, Error> {
        check::blast(blast)?;
        let center = blast.center_mm.map(|c| c as f32 / 1000.0);
        let reach = blast.radius_mm as f32 / 1000.0;
        let peak = blast.impulse_ns as f32 / BLAST_REFERENCE_KG;
        let mut pushed = 0;
        for rec in self.records.iter().flatten() {
            let Some(body) = self.world.bodies.get_mut(rec.handle) else {
                continue;
            };
            let at = body.translation();
            let offset = [at.x - center[0], at.y - center[1], at.z - center[2]];
            let distance = offset.iter().map(|c| c * c).sum::<f32>().sqrt();
            if distance >= reach {
                continue;
            }
            let away = if distance > AT_CENTER {
                offset.map(|c| c / distance)
            } else {
                [0.0, 1.0, 0.0]
            };
            let gain = peak * (1.0 - distance / reach);
            let v = body.linvel();
            body.set_linvel(
                Vector::new(
                    v.x + away[0] * gain,
                    v.y + away[1] * gain,
                    v.z + away[2] * gain,
                ),
                true,
            );
            pushed += 1;
        }
        Ok(pushed)
    }

    /// Launches an arrow or a thrown object (named `id`, replaced by a respawn of the id) that
    /// reaches `flight.to` after exactly `flight.ticks` steps, on a path with no drag; the
    /// capsule points along its launch direction. The hit itself stays authority: this only
    /// draws the arc to it, and the body goes on as a normal body after the hit tick.
    pub fn spawn_thrown(
        &mut self,
        id: u32,
        shape: BoneShape,
        material: Material,
        flight: Flight,
    ) -> Result<(), Error> {
        check::shape(shape)?;
        check::vec3(flight.from, MAX_COORD, Field::Position)?;
        check::vec3(flight.to, MAX_COORD, Field::Position)?;
        check::flight_ticks(flight.ticks)?;
        let velocity = ballistic_velocity(flight, self.caps.solver_iterations);
        check::vec3(velocity, MAX_SPEED, Field::Velocity)?;
        let at = Transform {
            pos: flight.from,
            rot: aim_axis(velocity),
        };
        let handle = self.spawn_single(
            GroupId::Thrown(id),
            Key::Thrown { id },
            material.props(),
            shape,
            at,
            velocity,
        );
        if let Some(body) = self.world.bodies.get_mut(handle) {
            body.set_linear_damping(0.0);
        }
        Ok(())
    }
}

/// The launch velocity, m/s, whose arc lands on `flight.to` after exactly `flight.ticks` steps
/// of a world that cuts each step into `substeps` gravity-then-move substeps: the world moves a
/// body `n * v * dt + g * dt^2 * n * (n * substeps + 1) / (2 * substeps)` in `n` steps.
fn ballistic_velocity(flight: Flight, substeps: usize) -> [f32; 3] {
    let ticks = flight.ticks as f32;
    let substeps = substeps as f32;
    let mut v = [0, 1, 2].map(|k| (flight.to[k] - flight.from[k]) / (ticks * FIXED_DT));
    v[1] -= GRAVITY_Y * FIXED_DT * (ticks * substeps + 1.0) / (2.0 * substeps);
    v
}

/// The rotation that turns a capsule's long axis (local +Y) along `direction`; no turn for a
/// direction too short to have one.
fn aim_axis(direction: [f32; 3]) -> [f32; 4] {
    let length = direction.iter().map(|c| c * c).sum::<f32>().sqrt();
    if length < 1.0e-6 {
        return Transform::IDENTITY.rot;
    }
    let d = direction.map(|c| c / length);
    // Shortest arc from +Y to d: axis Y x d, scalar 1 + Y . d. Straight down is a half turn
    // about X.
    let w = 1.0 + d[1];
    if w < 1.0e-6 {
        return [1.0, 0.0, 0.0, 0.0];
    }
    let q = [d[2], 0.0, -d[0], w];
    let norm = q.iter().map(|c| c * c).sum::<f32>().sqrt();
    q.map(|c| c / norm)
}
