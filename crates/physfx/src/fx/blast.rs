//! blast: explosions that shove bodies and arrows that fly an arc to the authority's hit.

use super::{GroupId, PhysFx};
use crate::check::{self, BLAST_REFERENCE_KG, MAX_COORD, MAX_SPEED};
use crate::error::{Error, Field};
use crate::types::{Blast, Flight, Impact, Key, Transform};
use engine_physics::material::Material;
use engine_physics::ragdoll::BoneShape;
use engine_physics::rapier::{FIXED_DT, GRAVITY_Y};
use rapier3d::dynamics::RigidBodyHandle;
use rapier3d::math::Vector;

/// Closer to the center than this (meters) a body has no outward direction and goes straight up.
const AT_CENTER: f32 = 1.0e-4;

/// A body inside the reach of a blast or an impact.
struct Near {
    handle: RigidBodyHandle,
    /// From the center to the body, meters.
    offset: [f32; 3],
    distance: f32,
}

impl PhysFx {
    /// The bodies closer to `center` than `reach` (meters), nearest first and at most `limit` of
    /// them; ties go to the older body, so the pick is the same every run.
    fn nearest(&self, center: [f32; 3], reach: f32, limit: usize) -> Vec<Near> {
        let mut near: Vec<(u32, Near)> = Vec::new();
        for (index, rec) in self.records.iter().enumerate() {
            let Some(rec) = rec else { continue };
            let Some(body) = self.world.bodies.get(rec.handle) else {
                continue;
            };
            let at = body.translation();
            let offset = [at.x - center[0], at.y - center[1], at.z - center[2]];
            let distance = offset.iter().map(|c| c * c).sum::<f32>().sqrt();
            if distance < reach {
                near.push((
                    index as u32,
                    Near {
                        handle: rec.handle,
                        offset,
                        distance,
                    },
                ));
            }
        }
        near.sort_by(|a, b| a.1.distance.total_cmp(&b.1.distance).then(a.0.cmp(&b.0)));
        near.truncate(limit);
        near.into_iter().map(|(_, n)| n).collect()
    }

    /// Shoves the bodies inside the blast's radius straight away from its center and wakes them.
    /// The gain is `impulse / 70 kg` at the center and falls off linearly to nothing at the
    /// radius, whatever the body weighs; a body at the radius or beyond is not touched, and only
    /// the nearest `Caps::blast_bodies` are. Returns how many bodies were pushed. A bad blast is
    /// refused and changes nothing.
    pub fn apply_blast(&mut self, blast: Blast) -> Result<usize, Error> {
        check::blast(blast)?;
        let center = blast.center_mm.map(|c| c as f32 / 1000.0);
        let reach = blast.radius_mm as f32 / 1000.0;
        let peak = blast.impulse_ns as f32 / BLAST_REFERENCE_KG;
        let mut pushed = 0;
        for near in self.nearest(center, reach, self.caps.blast_bodies) {
            let Some(body) = self.world.bodies.get_mut(near.handle) else {
                continue;
            };
            let away = if near.distance > AT_CENTER {
                near.offset.map(|c| c / near.distance)
            } else {
                [0.0, 1.0, 0.0]
            };
            let gain = peak * (1.0 - near.distance / reach);
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

    /// Shoves the bodies around a projectile's hit point along its heading with a real impulse:
    /// `impulse_ns` at the hit point, falling off linearly to nothing at the radius. A heavy body
    /// gains less speed than a light one, and a hit off its center spins it. No body gains more
    /// than the speed cap, only the nearest `Caps::impact_bodies` are touched, and they wake.
    /// Returns how many were hit. A bad impact is refused and changes nothing.
    pub fn apply_impact(&mut self, hit: Impact) -> Result<usize, Error> {
        check::impact(hit)?;
        let point = hit.at_mm.map(|c| c as f32 / 1000.0);
        let reach = hit.radius_mm as f32 / 1000.0;
        let heading = hit.direction.map(f32::from);
        let length = heading.iter().map(|c| c * c).sum::<f32>().sqrt();
        let heading = heading.map(|c| c / length);
        let mut struck = 0;
        for near in self.nearest(point, reach, self.caps.impact_bodies) {
            let Some(body) = self.world.bodies.get_mut(near.handle) else {
                continue;
            };
            let share = hit.impulse_ns as f32 * (1.0 - near.distance / reach);
            let impulse = share.min(MAX_SPEED * body.mass());
            body.apply_impulse_at_point(
                Vector::new(
                    heading[0] * impulse,
                    heading[1] * impulse,
                    heading[2] * impulse,
                ),
                Vector::new(point[0], point[1], point[2]),
                true,
            );
            struck += 1;
        }
        Ok(struck)
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
