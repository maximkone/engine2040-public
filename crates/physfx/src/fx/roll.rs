//! roll: rolling resistance at the contact, so the sand stops a limb friction cannot.
//!
//! A capsule lying on the sand and spinning about its own long axis has no slip at the
//! contact: Coulomb friction never sees it, so it rolls in place until the hand drain in
//! [`super::rest`] (or nothing at all) takes it away. Box2D v3 stores a per-contact
//! `rollingResistance` on the constraint and solves a rolling impulse with
//! `rollingMass = 1/(iA + iB)`, warm-started and bounded by the normal impulse
//! (github.com/erincatto/box2d `src/contact_solver.c`, `b2PrepareContacts_Overflow` and
//! `b2WarmStartContacts_Overflow`: `constraint->rollingResistance`, `cp->rollingImpulse`,
//! `wA -= iA * rollingImpulse`). This is that term carried to 3D: the rolling axis is the
//! contact plane (a 2D body rolls about the one axis out of its plane; a 3D body rolls
//! about the two axes tangent to the contact normal), and the impulse is a pure couple, so
//! it never moves the body's centre of mass.
//!
//! Per contact: `rolling_mass = 1/(invI_a + invI_b)` about each tangent axis, the relative
//! angular velocity's component on that axis clamped by `rolling * normal_impulse * mass`,
//! applied equal and opposite. `rolling` is a length (a moment arm: torque per unit normal
//! force), [`ROLLING`] is sand. [`ROLLING_OFF`] is the floor: the pass does nothing. The
//! pass reads the pairs the narrow phase already solved and allocates nothing.
//! Presentation only, never hashed: a sleeping body is never woken by it.
//!
//! Default: [`ROLLING_OFF`]. Two existing pins move with the pass on
//! (`tests::hit::a_toned_body_keeps_its_shape_where_a_limp_one_folds` and
//! `tests::settle::a_knocked_down_ragdoll_falls_asleep_within_three_seconds`, which sleeps
//! after 188 ticks instead of its pinned window), so the look turns it on with
//! `set_rolling(ROLLING)` when a reviewed frame takes the new settle timing.

use super::PhysFx;
use rapier3d::dynamics::RigidBody;
use rapier3d::math::Vector;

/// Rolling resistance coefficient, meters: the moment arm that turns a contact's normal
/// impulse into the bound on the torque impulse that resists rolling. Box2D v3's
/// `b2ContactConstraint::rollingResistance` (`src/contact_solver.c`). Sand grips: a
/// limb stops within a couple of ticks, a dead body is not dragged.
pub const ROLLING: f32 = 0.05;

/// The off switch: no rolling resistance at all, the world before this pass existed.
pub const ROLLING_OFF: f32 = 0.0;

/// Below this a component reads as no roll: radians per second, and it keeps a resting
/// contact from writing impulse noise every tick.
const ROLL_FLOOR_RAD_S: f32 = 1.0e-4;

/// Two unit axes spanning the contact plane of `normal` (unit).
fn tangent_axes(normal: Vector) -> (Vector, Vector) {
    // Any helper that is not parallel to the normal gives a stable pair.
    let helper = if normal.x.abs() < 0.9 {
        Vector::X
    } else {
        Vector::Y
    };
    let u = normal.cross(helper).normalize();
    (u, normal.cross(u))
}

/// The body's inverse angular inertia about `axis` (unit): `axis . (I^-1 axis)`.
///
/// Rapier keeps the square root of the world inverse inertia tensor, so two products give
/// the answer. A static or rotation-locked body reads zero and cannot take a couple.
fn inv_inertia_about(body: &RigidBody, axis: Vector) -> f32 {
    let m = body.mass_properties().effective_world_inv_inertia;
    axis.dot(m * (m * axis)).max(0.0)
}

/// The contact normal as a unit vector, or `None` when the narrow phase holds a degenerate
/// one (never a panic on solver data).
fn unit(normal: Vector) -> Option<Vector> {
    let len = normal.length();
    (len > 1.0e-6 && len.is_finite()).then(|| normal / len)
}

impl PhysFx {
    /// Sets the rolling resistance of every contact this world solves, in meters; a
    /// non-finite value or one below zero reads as [`ROLLING_OFF`]. `ROLLING` is the sand.
    pub fn set_rolling(&mut self, rolling: f32) {
        self.rolling = if rolling.is_finite() {
            rolling.max(0.0)
        } else {
            ROLLING_OFF
        };
    }

    /// The rolling resistance the next step will use, meters.
    #[must_use]
    pub fn rolling(&self) -> f32 {
        self.rolling
    }

    /// Turns on the sand's own rolling resistance, [`ROLLING`]: the one call the look
    /// makes when a reviewed frame takes the settle timing this pass changes.
    pub fn set_rolling_sand(&mut self) {
        self.rolling = ROLLING;
    }

    /// Brakes the rolling component of every touching pair's relative angular velocity by
    /// up to `rolling * normal impulse`. Call it right after the world stepped: it reads
    /// the impulses the solver just wrote and gives back the read-back unchanged shapes.
    pub(super) fn apply_rolling_resistance(&mut self) {
        let rolling = self.rolling;
        if rolling <= 0.0 {
            return;
        }
        // Field split: the pair iterator borrows the narrow phase, the couples need the
        // bodies; two different fields of the same world, so both are free at once.
        for pair in self.world.narrow_phase.contact_pairs() {
            if !pair.has_any_active_contact() {
                continue;
            }
            for manifold in pair.manifolds() {
                let Some(normal) = unit(manifold.data.normal) else {
                    continue;
                };
                let (Some(handle_a), Some(handle_b)) =
                    (manifold.data.rigid_body1, manifold.data.rigid_body2)
                else {
                    continue;
                };
                let mut normal_impulse = 0.0f32;
                for point in &manifold.points {
                    // The solver's impulse along the contact normal, push only.
                    normal_impulse += point.data.impulse.max(0.0);
                }
                if normal_impulse <= 0.0 {
                    continue;
                }
                let (Some(a), Some(b)) = (
                    self.world.bodies.get(handle_a),
                    self.world.bodies.get(handle_b),
                ) else {
                    continue;
                };
                // A sleeping pair stays asleep: this pass never wakes the world.
                if a.is_sleeping() && b.is_sleeping() {
                    continue;
                }
                let (u, v) = tangent_axes(normal);
                let rel = a.angvel() - b.angvel();
                // Two orthogonal tangent axes: each couple is solved on its own and the
                // pair is applied below, so neither reads the other's half.
                let mut couple = Vector::ZERO;
                for axis in [u, v] {
                    let spin = rel.dot(axis);
                    if spin.abs() <= ROLL_FLOOR_RAD_S {
                        continue;
                    }
                    let inv = inv_inertia_about(a, axis) + inv_inertia_about(b, axis);
                    if inv <= 0.0 {
                        continue;
                    }
                    let limit = rolling * normal_impulse;
                    let want = (-spin / inv).clamp(-limit, limit);
                    couple += axis * want;
                }
                if couple.dot(couple) <= 0.0 {
                    continue;
                }
                // Both sides read before either is written (fields borrowed).
                let apply_a = a.is_dynamic() && !a.is_sleeping();
                let apply_b = b.is_dynamic() && !b.is_sleeping();
                if apply_a {
                    if let Some(body) = self.world.bodies.get_mut(handle_a) {
                        body.apply_torque_impulse(couple, false);
                    }
                }
                if apply_b {
                    if let Some(body) = self.world.bodies.get_mut(handle_b) {
                        body.apply_torque_impulse(-couple, false);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::Tier;
    use crate::types::{Key, Transform};
    use engine_physics::ragdoll::BoneShape;
    use std::f32::consts::FRAC_1_SQRT_2;

    /// A capsule's shape: a 0.1 m radius, 0.4 m long limb.
    fn capsule() -> BoneShape {
        BoneShape {
            radius: 0.1,
            half_height: 0.2,
        }
    }

    /// A capsule lying on its long axis (local Y turned onto world X), its centre one
    /// radius above the floor top at y = 0.
    fn lying_at() -> Transform {
        Transform {
            pos: [0.0, 0.1, 0.0],
            rot: [0.0, 0.0, FRAC_1_SQRT_2, FRAC_1_SQRT_2],
        }
    }

    /// A world with a floor whose top is at y = 0, holding one rolling capsule.
    fn floor_world() -> PhysFx {
        let mut fx = PhysFx::new(Tier::Comfort);
        fx.add_static_box([50.0, 0.5, 50.0], [0.0, -0.5, 0.0])
            .expect("floor");
        fx
    }

    /// A capsule rolling along +Z at 0.8 m/s: spin 8 rad/s about its long axis, matched so
    /// the contact point does not slip. Fast enough that the rest assist in `rest.rs`
    /// leaves it alone (its linear speed is over the walking bound), so friction has
    /// nothing to hold and only rolling resistance can take the spin.
    fn rolling_capsule() -> PhysFx {
        let mut fx = floor_world();
        fx.set_rolling(ROLLING);
        fx.spawn_limb(7, capsule(), lying_at(), [0.0, 0.0, 0.8])
            .expect("limb");
        fx.set_spin(7, [8.0, 0.0, 0.0]);
        fx
    }

    fn length(v: [f32; 3]) -> f32 {
        (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
    }

    #[test]
    fn rolling_resistance_stops_a_capsule_that_friction_cannot() {
        let mut on = rolling_capsule();
        for _ in 0..60 {
            on.step();
        }
        let mut off = rolling_capsule();
        off.set_rolling(ROLLING_OFF);
        for _ in 0..60 {
            off.step();
        }
        let on_spin = on.spin_of(7).expect("limb");
        let off_spin = off.spin_of(7).expect("limb");
        println!(
            "roll: pass on {on_spin:?} ({:.3} rad/s), off {off_spin:?} ({:.3} rad/s)",
            length(on_spin),
            length(off_spin)
        );
        assert!(
            length(on_spin) < 0.5,
            "the sand stops the roll in 60 ticks: {on_spin:?}"
        );
        assert!(
            length(off_spin) > 2.0,
            "with the pass off the capsule still turns: {off_spin:?}"
        );
    }

    #[test]
    fn a_still_capsule_stays_still_with_rolling_on() {
        let mut fx = floor_world();
        fx.set_rolling(ROLLING);
        fx.spawn_limb(9, capsule(), lying_at(), [0.0; 3])
            .expect("limb");
        let before = fx.pose_of_part(9).expect("limb");
        for _ in 0..40 {
            fx.step();
        }
        let after = fx.pose_of_part(9).expect("limb");
        let spin = fx.spin_of(9).expect("limb");
        println!(
            "roll: still start {:?} end {:?} spin {spin:?}",
            before.pos, after.pos
        );
        for axis in 0..3 {
            assert!(
                (after.pos[axis] - before.pos[axis]).abs() < 5.0e-3,
                "a still capsule does not move: {:?} -> {:?}",
                before.pos,
                after.pos
            );
        }
        assert!(
            length(spin) < 0.05,
            "a still capsule does not spin: {spin:?}"
        );
    }

    #[test]
    fn rolling_pass_is_the_same_bytes_twice() {
        let run = || {
            let mut fx = rolling_capsule();
            for _ in 0..30 {
                fx.step();
            }
            let placed = fx.transforms(1.0).expect("transforms");
            let mut bytes = Vec::new();
            for one in &placed {
                for v in one.at.pos.iter().chain(one.at.rot.iter()) {
                    bytes.extend_from_slice(&v.to_bits().to_le_bytes());
                }
            }
            bytes.extend_from_slice(&length(fx.spin_of(7).expect("limb")).to_bits().to_le_bytes());
            bytes
        };
        assert_eq!(run(), run(), "the same rolling world gives the same bytes");
    }

    // The read-back helpers the tests above need on a body a test spawned by part number.

    impl PhysFx {
        /// The angular velocity of the limb `part`.
        pub(crate) fn spin_of(&self, part: u32) -> Option<[f32; 3]> {
            let record = self
                .records
                .iter()
                .flatten()
                .find(|rec| rec.key == Key::Limb { part })?;
            let body = self.world.bodies.get(record.handle)?;
            let w = body.angvel();
            Some([w.x, w.y, w.z])
        }

        /// Sets the angular velocity of the limb `part`, waking it.
        pub(crate) fn set_spin(&mut self, part: u32, spin: [f32; 3]) {
            let Some(handle) = self
                .records
                .iter()
                .flatten()
                .find(|rec| rec.key == Key::Limb { part })
                .map(|rec| rec.handle)
            else {
                return;
            };
            if let Some(body) = self.world.bodies.get_mut(handle) {
                body.set_angvel(Vector::new(spin[0], spin[1], spin[2]), true);
            }
        }

        /// The transform of the limb `part`.
        pub(crate) fn pose_of_part(&self, part: u32) -> Option<Transform> {
            let record = self
                .records
                .iter()
                .flatten()
                .find(|rec| rec.key == Key::Limb { part })?;
            let body = self.world.bodies.get(record.handle)?;
            Some(crate::fx::pose_of(body))
        }
    }
}
