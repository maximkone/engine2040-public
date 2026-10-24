//! rest: the sand catches a ragdoll that has stopped falling.
//!
//! A body lying on the sand keeps a slow joint/contact crawl: every bone reads under walking
//! speed, yet a capsule that rolls on its own axis has no slip at the contact, so friction
//! cannot stop it and the engine's damping alone takes seconds. Rapier only sleeps a body
//! whose pose drifts under a fraction of a millimeter per step, so one crawling bone keeps
//! the whole jointed island awake and the look's fall never settles.
//!
//! While every bone of a ragdoll (or of a limb that came off one) is slow and no bone has
//! drifted past a couple of centimeters from where its run began, this drains a little linear
//! and angular velocity per step, the way sand resists a rolling limb. After half a second of
//! that the island is put to sleep outright: the crawl never meets Rapier's pose gate, and
//! sand holds a body that has stopped falling. A body still falling has a bone past the
//! bounds, so the assist stays off and the flop stays live; a hit, a blast or a tether wakes
//! a sleeping body as it always did. The drift check is what makes the run survive the one-tick
//! velocity spikes a resting joint chain still shows: the eye sees no motion, so the sand takes
//! it.
//!
//! A group is taken only when every awake body in its island is at rest too: a still group
//! turned into a wall under a limb that is still spinning pumps the limb instead of holding
//! it. The integer ragdoll has the same damping rule in `engine_physics::ragdoll_contacts`
//! (`damp_resting_crawl`); this is the float layer's copy. Presentation only, never hashed.
//! Budget: one pass over the ragdoll groups per tick, no allocation.

use super::{pose_of, GroupId, PhysFx};
use rapier3d::dynamics::RigidBody;
use rapier3d::math::Vector;

/// One group's rest reading, as [`PhysFx::rest_state`] reports it: bone count,
/// quiet ticks, sleeping bones, fastest linear and angular speed, its
/// persistent island and whether that island reads at rest.
#[cfg(test)]
pub(crate) type RestState = (usize, u32, usize, f32, f32, Option<u32>, bool);

/// Every bone of a lying ragdoll is under this: meters per second.
const REST_MAX_BONE_MPS: f32 = 0.5;
/// Every bone of a lying ragdoll is under this: radians per second, for the sleep decision.
/// Rapier's own bound for a body that unambiguously moves (`pi` / 2), tightened: a lying bone
/// above it is still moving to the eye.
const REST_MAX_BONE_RAD_S: f32 = 1.5;
/// The angular bound for damping, looser than the sleep bound: a limb lying on the sand can
/// roll in place while its linear speed already reads still, and the sand should catch it
/// before it stops on its own.
const REST_DAMP_MAX_BONE_RAD_S: f32 = 12.0;
/// Ticks every bone of a group must read at rest in a row before the sand takes it: half a
/// second at 60 Hz.
const REST_SETTLE_TICKS: u32 = 30;
/// How far a bone may drift from where its still run began, meters. A resting joint chain
/// still shows one-tick velocity spikes; the eye reads the pose, so the run measures that.
const REST_STILL_M: f32 = 0.02;
/// Linear velocity kept per rest-damped step.
const REST_KEEP_LINEAR: f32 = 0.9;
/// Angular velocity kept per rest-damped step. A rolling capsule has no contact slip, so this
/// is the only thing that stops it; eight steps take a crawl to a third.
const REST_KEEP_ANGULAR: f32 = 0.8;

/// True while `body` is asleep or under both sleep bounds.
fn at_rest(body: &RigidBody) -> bool {
    body.is_sleeping()
        || (body.linvel().length_squared() < REST_MAX_BONE_MPS * REST_MAX_BONE_MPS
            && body.angvel().length_squared() < REST_MAX_BONE_RAD_S * REST_MAX_BONE_RAD_S)
}

/// True while `body` is asleep or slow enough for the sand to resist it.
fn slow_enough_to_damp(body: &RigidBody) -> bool {
    body.is_sleeping()
        || (body.linvel().length_squared() < REST_MAX_BONE_MPS * REST_MAX_BONE_MPS
            && body.angvel().length_squared() < REST_DAMP_MAX_BONE_RAD_S * REST_DAMP_MAX_BONE_RAD_S)
}

/// True when `a` and `b` are within `limit` meters of each other.
fn within(a: [f32; 3], b: [f32; 3], limit: f32) -> bool {
    let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    dx * dx + dy * dy + dz * dz <= limit * limit
}

/// `index` as a record slot; a body index is a `u32`, a slot is a `usize`.
fn slot(index: u32) -> Option<usize> {
    usize::try_from(index).ok()
}

impl PhysFx {
    /// Drains a little velocity from every ragdoll and limb whose bones are all slow and still,
    /// then sleeps the island of a group that held still for [`REST_SETTLE_TICKS`]; call it
    /// after the world stepped, before the read-back.
    pub(super) fn settle_resting_bodies(&mut self) {
        for gi in 0..self.groups.len() {
            if !matches!(self.groups[gi].id, GroupId::Ragdoll(_) | GroupId::Limb(_)) {
                continue;
            }
            let mut damping = true;
            let mut still = true;
            for index in &self.groups[gi].bodies {
                let Some(record) = slot(*index)
                    .and_then(|i| self.records.get(i))
                    .and_then(Option::as_ref)
                else {
                    damping = false;
                    still = false;
                    break;
                };
                let Some(body) = self.world.bodies.get(record.handle) else {
                    damping = false;
                    still = false;
                    break;
                };
                damping &= slow_enough_to_damp(body);
                still &= within(record.settle.pos, pose_of(body).pos, REST_STILL_M);
            }
            if damping && still {
                self.groups[gi].quiet = self.groups[gi].quiet.saturating_add(1);
            } else {
                self.groups[gi].quiet = 0;
                self.anchor_group(gi);
            }
            if self.groups[gi].quiet >= REST_SETTLE_TICKS && self.sleep_island(gi) {
                continue;
            }
            if !damping {
                continue;
            }
            for bi in 0..self.groups[gi].bodies.len() {
                let index = self.groups[gi].bodies[bi];
                let Some(record) = slot(index)
                    .and_then(|i| self.records.get(i))
                    .and_then(Option::as_ref)
                else {
                    continue;
                };
                let Some(body) = self.world.bodies.get_mut(record.handle) else {
                    continue;
                };
                if body.is_sleeping() {
                    continue;
                }
                let (v, w) = (body.linvel(), body.angvel());
                body.set_linvel(
                    Vector::new(
                        v.x * REST_KEEP_LINEAR,
                        v.y * REST_KEEP_LINEAR,
                        v.z * REST_KEEP_LINEAR,
                    ),
                    false,
                );
                body.set_angvel(
                    Vector::new(
                        w.x * REST_KEEP_ANGULAR,
                        w.y * REST_KEEP_ANGULAR,
                        w.z * REST_KEEP_ANGULAR,
                    ),
                    false,
                );
            }
        }
    }

    /// Starts a new still run for group `gi` at the bones' current poses.
    fn anchor_group(&mut self, gi: usize) {
        for bi in 0..self.groups[gi].bodies.len() {
            let index = self.groups[gi].bodies[bi];
            let Some(record) = slot(index)
                .and_then(|i| self.records.get_mut(i))
                .and_then(Option::as_mut)
            else {
                continue;
            };
            if let Some(body) = self.world.bodies.get(record.handle) {
                record.settle = pose_of(body);
            }
        }
    }

    /// Puts every awake body of the persistent island that holds group `gi` to sleep, when the
    /// whole island reads at rest. False, and nothing changed, while a body of the island still
    /// moves. Sleeping only part of an island makes the slept bodies walls under a neighbour
    /// that still moves, which pumps that neighbour; the sand takes the island as one.
    fn sleep_island(&mut self, gi: usize) -> bool {
        let Some(island) = self.group_island(gi) else {
            return false;
        };
        if !self.island_at_rest(island) {
            return false;
        }
        for handle in self.world.islands.active_bodies() {
            if self
                .world
                .islands
                .persistent_island_of(&self.world.bodies, handle)
                != Some(island)
            {
                continue;
            }
            if let Some(body) = self.world.bodies.get_mut(handle) {
                // `sleep` zeroes the velocities too.
                body.sleep();
            }
        }
        true
    }

    /// True when every awake body of the persistent island `island` reads at rest.
    fn island_at_rest(&self, island: u32) -> bool {
        self.world.islands.active_bodies().all(|handle| {
            if self
                .world
                .islands
                .persistent_island_of(&self.world.bodies, handle)
                != Some(island)
            {
                return true;
            }
            self.world.bodies.get(handle).is_none_or(at_rest)
        })
    }

    /// The persistent island that holds group `gi`, when it still has one.
    fn group_island(&self, gi: usize) -> Option<u32> {
        let record = self
            .groups
            .get(gi)
            .and_then(|group| group.bodies.first())
            .and_then(|index| slot(*index))
            .and_then(|i| self.records.get(i))
            .and_then(Option::as_ref)?;
        self.world
            .islands
            .persistent_island_of(&self.world.bodies, record.handle)
    }

    /// Every ragdoll and limb group's bone count, quiet ticks, sleeping bones, fastest linear
    /// and angular speed, its persistent island and whether that island reads at rest; the
    /// settle test names the group that holds up the world.
    #[cfg(test)]
    pub(crate) fn rest_state(&self) -> Vec<RestState> {
        let mut out = Vec::new();
        for (gi, group) in self.groups.iter().enumerate() {
            if !matches!(group.id, GroupId::Ragdoll(_) | GroupId::Limb(_)) {
                continue;
            }
            let (mut v, mut w, mut sleeping) = (0.0f32, 0.0f32, 0usize);
            for index in &group.bodies {
                let Some(record) = slot(*index)
                    .and_then(|i| self.records.get(i))
                    .and_then(Option::as_ref)
                else {
                    continue;
                };
                let Some(body) = self.world.bodies.get(record.handle) else {
                    continue;
                };
                let speed = |x: f32, y: f32, z: f32| (x * x + y * y + z * z).sqrt();
                let (lv, av) = (body.linvel(), body.angvel());
                v = v.max(speed(lv.x, lv.y, lv.z));
                w = w.max(speed(av.x, av.y, av.z));
                sleeping += usize::from(body.is_sleeping());
            }
            let island = self.group_island(gi);
            let rest = island.is_some_and(|island| self.island_at_rest(island));
            out.push((
                group.bodies.len(),
                group.quiet,
                sleeping,
                v,
                w,
                island,
                rest,
            ));
        }
        out
    }
}
