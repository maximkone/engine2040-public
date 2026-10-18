//! tether: ropes between two points and the soft pull that carries a ragdoll along the sim's
//! own track.

use super::{GroupId, PhysFx};
use crate::check::{self, MAX_COORD, MAX_SPEED};
use crate::error::{Error, Field};
use crate::tether::{self, Tether, TetherState};
use engine_physics::rapier::{FIXED_DT, GRAVITY_Y};
use rapier3d::math::Vector;

/// Spring of the carry, per second squared: a ragdoll 10 cm behind its track closes at 14 m/s^2.
const CARRY_STIFFNESS: f32 = 144.0;
/// Damper of the carry, per second: twice the square root of the stiffness, so a body that is
/// pulled arrives without bouncing.
const CARRY_DAMPING: f32 = 24.0;

/// What a rope says about itself, for the caller's sound and shake.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TetherInfo {
    /// Slack, taut or loose.
    pub state: TetherState,
    /// The rope's length, meters.
    pub rest_m: f32,
    /// Newtons pulling the first end toward the second (the second is pulled the opposite way);
    /// zero while slack or loose.
    pub pull: [f32; 3],
    /// True once the rope stood still long enough to sleep.
    pub asleep: bool,
}

impl PhysFx {
    /// Ropes alive.
    pub fn tether_count(&self) -> usize {
        self.tethers.len()
    }

    /// Ties a rope named `id` (replaced by a second call with the same id) from `a` to `b`,
    /// `rest_m` meters long, with the node count of the tier. The oldest rope is recycled when
    /// the tier's rope cap is full. A bad end or length is refused and changes nothing.
    pub fn attach_tether(
        &mut self,
        id: u32,
        a: [f32; 3],
        b: [f32; 3],
        rest_m: f32,
    ) -> Result<(), Error> {
        check::vec3(a, MAX_COORD, Field::Position)?;
        check::vec3(b, MAX_COORD, Field::Position)?;
        tether::check_rest(rest_m)?;
        self.tethers.retain(|t| t.id != id);
        while self.tethers.len() >= self.caps.tethers {
            self.tethers.remove(0);
            self.recycled += 1;
        }
        let sweeps = self.caps.solver_iterations * 4;
        self.tethers.push(Tether::new(
            id,
            a,
            b,
            rest_m,
            self.caps.tether_nodes,
            sweeps,
        ));
        Ok(())
    }

    /// Asks for the two ends of rope `id` at the next step (call it once per tick, before
    /// `step`). Returns false when there is no such rope.
    pub fn move_tether(&mut self, id: u32, a: [f32; 3], b: [f32; 3]) -> Result<bool, Error> {
        check::vec3(a, MAX_COORD, Field::Position)?;
        check::vec3(b, MAX_COORD, Field::Position)?;
        Ok(self.tether_mut(id).map(|t| t.set_ends(a, b)).is_some())
    }

    /// Changes the length of rope `id`: a reel that takes rope in or pays it out. False when
    /// there is no such rope. A bad length is refused.
    pub fn retune_tether(&mut self, id: u32, rest_m: f32) -> Result<bool, Error> {
        tether::check_rest(rest_m)?;
        Ok(self.tether_mut(id).map(|t| t.set_rest(rest_m)).is_some())
    }

    /// Lets go of rope `id`: it falls, settles and is removed once it sleeps. False when there
    /// is no such rope.
    pub fn release_tether(&mut self, id: u32) -> bool {
        self.tether_mut(id).map(Tether::release).is_some()
    }

    /// The nodes of rope `id`, end to end, blended between the last two ticks like
    /// [`PhysFx::transforms`]. `None` when there is no such rope.
    pub fn tether_nodes(&self, id: u32, alpha: f32) -> Result<Option<Vec<[f32; 3]>>, Error> {
        if !alpha.is_finite() {
            return Err(Error::NonFinite(Field::Alpha));
        }
        let t = alpha.clamp(0.0, 1.0);
        Ok(self.tethers.iter().find(|r| r.id == id).map(|r| r.nodes(t)))
    }

    /// How rope `id` is doing, or `None` when there is no such rope.
    pub fn tether_info(&self, id: u32) -> Option<TetherInfo> {
        self.tethers
            .iter()
            .find(|r| r.id == id)
            .map(|r| TetherInfo {
                state: r.state(),
                rest_m: r.rest(),
                pull: r.pull(),
                asleep: r.asleep(),
            })
    }

    /// Carries the ragdoll of `slot` along the sim's track: a spring-damper on its root bone
    /// toward `to` (meters) at `velocity` (m/s), the way a rope's end drags what it holds. The
    /// limbs follow through their joints, so they trail and swing. With `lift` false the
    /// carry leaves the height alone, so a body that was held up falls and lands. False when
    /// the slot has no ragdoll. Out-of-range input is refused.
    pub fn carry_ragdoll(
        &mut self,
        slot: u8,
        to: [f32; 3],
        velocity: [f32; 3],
        lift: bool,
    ) -> Result<bool, Error> {
        check::vec3(to, MAX_COORD, Field::Position)?;
        check::vec3(velocity, MAX_SPEED, Field::Velocity)?;
        let Some(group) = self.groups.iter().find(|g| g.id == GroupId::Ragdoll(slot)) else {
            return Ok(false);
        };
        let handle_of = |index: &u32| {
            let at = usize::try_from(*index).ok()?;
            self.records.get(at)?.as_ref().map(|rec| rec.handle)
        };
        let mut handles = group.bodies.iter().filter_map(handle_of);
        let Some(root) = handles.next() else {
            return Ok(false);
        };
        if lift {
            // Held up, the whole body floats: the soft joints cannot carry a heavy limb
            // chain on a light root, so gravity is taken off every other bone.
            let anti = -GRAVITY_Y * FIXED_DT;
            for handle in handles {
                if let Some(bone) = self.world.bodies.get_mut(handle) {
                    let v = bone.linvel();
                    bone.set_linvel(Vector::new(v.x, v.y + anti, v.z), true);
                }
            }
        }
        let Some(body) = self.world.bodies.get_mut(root) else {
            return Ok(false);
        };
        let (at, v) = (body.translation(), body.linvel());
        let up = if lift {
            CARRY_STIFFNESS * (to[1] - at.y) + CARRY_DAMPING * (velocity[1] - v.y) - GRAVITY_Y
        } else {
            0.0
        };
        let gain = [
            CARRY_STIFFNESS * (to[0] - at.x) + CARRY_DAMPING * (velocity[0] - v.x),
            up,
            CARRY_STIFFNESS * (to[2] - at.z) + CARRY_DAMPING * (velocity[2] - v.z),
        ]
        .map(|a| a * FIXED_DT);
        body.set_linvel(
            Vector::new(v.x + gain[0], v.y + gain[1], v.z + gain[2]),
            true,
        );
        Ok(true)
    }

    /// Where the root bone of the ragdoll in `slot` is now, meters, or `None` without one.
    pub fn ragdoll_root(&self, slot: u8) -> Option<[f32; 3]> {
        let index = self
            .groups
            .iter()
            .find(|g| g.id == GroupId::Ragdoll(slot))?
            .bodies
            .first()?;
        let rec = self.records.get(usize::try_from(*index).ok()?)?.as_ref()?;
        let at = self.world.bodies.get(rec.handle)?.translation();
        Some([at.x, at.y, at.z])
    }

    /// How fast the root bone of the ragdoll in `slot` moves, m/s, or `None` without one: what
    /// a caller asks to know that a body has landed and may get up.
    pub fn ragdoll_speed(&self, slot: u8) -> Option<f32> {
        let index = self
            .groups
            .iter()
            .find(|g| g.id == GroupId::Ragdoll(slot))?
            .bodies
            .first()?;
        let rec = self.records.get(usize::try_from(*index).ok()?)?.as_ref()?;
        let v = self.world.bodies.get(rec.handle)?.linvel();
        Some((v.x * v.x + v.y * v.y + v.z * v.z).sqrt())
    }

    fn tether_mut(&mut self, id: u32) -> Option<&mut Tether> {
        self.tethers.iter_mut().find(|t| t.id == id)
    }

    /// Steps every rope one tick and drops the released ones that came to rest.
    pub(super) fn step_tethers(&mut self) {
        for rope in &mut self.tethers {
            rope.step();
        }
        self.tethers.retain(|t| !t.spent());
    }
}
