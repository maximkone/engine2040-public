//! fx: the Rapier world, its spawn order, the caps and the transform read-back.

mod blast;
mod hit;
mod joints;
mod receipt;
mod sever;
mod tether;

pub use hit::HIT_FADE_S;
pub use receipt::RagdollReceipt;
pub use sever::Severed;
pub use tether::TetherInfo;

use crate::caps::{Caps, Tier};
use crate::check::{self, MAX_IMPULSE, MAX_SPEED};
use crate::error::{Error, Field};
use crate::groups;
use crate::rig::Rig;
use crate::tether::Tether;
use crate::types::{Batch, Key, Placed, Shard, Stats, Transform};
use engine_anim::contract::{validate_pose, LocalPose};
use engine_physics::layers as table;
use engine_physics::material::{Material, Props};
use engine_physics::ragdoll::{insert_joints, world_transforms, BoneShape};
use engine_physics::rapier::{FIXED_DT, GRAVITY_Y};
use rapier3d::dynamics::{RigidBody, RigidBodyBuilder, RigidBodyHandle};
#[cfg(test)]
use rapier3d::geometry::InteractionGroups;
use rapier3d::geometry::{ColliderBuilder, ColliderHandle, ContactPair};
use rapier3d::math::{Rotation, Vector};
use rapier3d::pipeline::PhysicsWorld;
use std::f32::consts::PI;

/// Limbs and debris shed per step at most, so an overload thins out instead of blinking.
const SHED_PER_STEP: usize = 8;
/// `user_data` of a body that has no record (the static boxes).
const NO_RECORD: u128 = u128::MAX;

/// Static boxes: floor, walls.
const STATIC_FRICTION: f32 = 0.8;

/// One live body and the last two ticks of its pose.
struct Record {
    handle: RigidBodyHandle,
    collider: ColliderHandle,
    key: Key,
    /// Pose one tick ago.
    prev: Transform,
    /// Pose after the last step.
    curr: Transform,
    /// The step that last read this body while it was awake.
    stamp: u64,
}

/// What a group is, which also says what replaces it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupId {
    /// The ragdoll of a fighter slot: a respawn in the slot replaces it.
    Ragdoll(u8),
    /// A limb part: a respawn of the part replaces it.
    Limb(u32),
    /// An arrow or thrown object: a respawn of the id replaces it.
    Thrown(u32),
    /// One debris shard.
    Shard,
}

/// The table layer of a group: the bones of a fighter and the limbs that came off are the
/// body, a shard of a break is debris, a thrown object is a prop.
fn layer_of(id: &GroupId) -> u32 {
    match id {
        GroupId::Ragdoll(_) | GroupId::Limb(_) => table::BODY,
        GroupId::Thrown(_) => table::PROP,
        GroupId::Shard => table::DEBRIS,
    }
}

/// Bodies spawned together; recycled together, because half a ragdoll is no ragdoll.
struct Group {
    id: GroupId,
    /// Indices into `records`.
    bodies: Vec<u32>,
    /// What the spawn of a ragdoll measured; `None` for every other group.
    receipt: Option<RagdollReceipt>,
    /// The joint motors of a ragdoll; `None` for every other group.
    drive: Option<hit::Drive>,
    /// Parent bone of every bone, in skeleton order; empty for every other group. `sever` reads
    /// the joint this names, so the ragdoll keeps no second copy of its rig.
    parents: Vec<Option<usize>>,
    /// How hard the joints of a ragdoll brace, permille of the whole; zero for
    /// every other group.
    muscle_permille: u16,
}

/// A clock that always reads zero: the default, so the crate never touches `Instant`.
fn no_clock() -> u64 {
    0
}

/// The physical layer. See the crate docs for the rules.
pub struct PhysFx {
    caps: Caps,
    world: PhysicsWorld,
    /// Slab of bodies; the index is stored in the Rapier body's `user_data`.
    records: Vec<Option<Record>>,
    /// Freed slab indices, reused last-in first-out.
    free: Vec<u32>,
    /// Spawn order, oldest first: the recycling order.
    groups: Vec<Group>,
    /// Records the last step read as awake.
    awake: Vec<u32>,
    scratch: Vec<u32>,
    /// Ropes, oldest first.
    tethers: Vec<Tether>,
    live: usize,
    statics: usize,
    next_batch: u32,
    steps: u64,
    recycled: u64,
    contacts: usize,
    micros: u64,
    clock: fn() -> u64,
}

impl PhysFx {
    /// An empty world with the caps of `tier`.
    pub fn new(tier: Tier) -> Self {
        // The tier table is checked by a test, so it needs no validation here.
        Self::build(tier.caps())
    }

    /// An empty world with a cap table of the caller's own. Refused when a limit that must be
    /// above zero is zero.
    pub fn with_caps(caps: Caps) -> Result<Self, Error> {
        caps.validate()?;
        Ok(Self::build(caps))
    }

    /// Sets the clock that fills [`Stats::micros`]: a function returning microseconds. The crate
    /// has none of its own, so a browser build stays free of `Instant`.
    #[must_use]
    pub fn with_clock(mut self, clock: fn() -> u64) -> Self {
        self.clock = clock;
        self
    }

    fn build(caps: Caps) -> Self {
        let mut world = PhysicsWorld::new();
        world.gravity = Vector::new(0.0, GRAVITY_Y, 0.0);
        world.integration_parameters.dt = FIXED_DT;
        world.integration_parameters.num_solver_iterations = caps.solver_iterations;
        world.integration_parameters.max_ccd_substeps = caps.ccd_substeps;
        Self {
            caps,
            world,
            records: Vec::with_capacity(caps.bodies),
            free: Vec::new(),
            groups: Vec::new(),
            awake: Vec::with_capacity(caps.bodies),
            scratch: Vec::with_capacity(caps.bodies),
            tethers: Vec::new(),
            live: 0,
            statics: 0,
            next_batch: 0,
            steps: 0,
            recycled: 0,
            contacts: 0,
            micros: 0,
            clock: no_clock,
        }
    }

    /// Adds an immovable box: the floor, a wall, a prop that never moves. Refused past the
    /// static cap or with a bad size or position. The box is on the world layer.
    pub fn add_static_box(&mut self, half: [f32; 3], pos: [f32; 3]) -> Result<(), Error> {
        self.add_static_box_on(table::WORLD, half, pos)
    }

    /// Adds an immovable box on the table layer `layer`: the floor and the walls are the
    /// world, a crate is a prop. A layer the table does not know is refused, so a box can
    /// never land on a layer that meets nothing by accident.
    pub fn add_static_box_on(
        &mut self,
        layer: u32,
        half: [f32; 3],
        pos: [f32; 3],
    ) -> Result<(), Error> {
        check::static_half(half)?;
        check::vec3(pos, check::MAX_COORD, Field::Position)?;
        if !table::is_single(layer) {
            return Err(Error::OutOfRange(Field::Layer));
        }
        if self.statics >= self.caps.statics {
            return Err(Error::OverCap {
                need: self.statics + 1,
                cap: self.caps.statics,
            });
        }
        let body = RigidBodyBuilder::fixed()
            .translation(Vector::new(pos[0], pos[1], pos[2]))
            .user_data(NO_RECORD)
            .build();
        let collider = ColliderBuilder::cuboid(half[0], half[1], half[2])
            .friction(STATIC_FRICTION)
            .restitution(0.0)
            .collision_groups(groups::of(layer))
            .build();
        let handle = self.world.bodies.insert(body);
        self.world
            .colliders
            .insert_with_parent(collider, handle, &mut self.world.bodies);
        self.statics += 1;
        Ok(())
    }

    /// Turns a fighter into a ragdoll. `pose` is the animated pose at the moment authority says
    /// the fighter is floored (its root carries the world placement), `velocity` is how fast the
    /// fighter was moving, and `impulse` (newton-seconds) hits the root bone, so the joints carry
    /// the blow up the chain. A ragdoll already in `slot` is replaced.
    pub fn spawn_ragdoll(
        &mut self,
        slot: u8,
        rig: &Rig,
        pose: &LocalPose,
        velocity: [f32; 3],
        impulse: [f32; 3],
    ) -> Result<(), Error> {
        validate_pose(rig.skeleton(), pose).map_err(Error::Anim)?;
        check::vec3(velocity, MAX_SPEED, Field::Velocity)?;
        check::vec3(impulse, MAX_IMPULSE, Field::Impulse)?;
        let need = rig.bones();
        if need > self.caps.bodies {
            return Err(Error::OverCap {
                need,
                cap: self.caps.bodies,
            });
        }
        let (pos, rot) = world_transforms(rig.skeleton(), pose);
        let mut placed = Vec::with_capacity(need);
        for (p, q) in pos.iter().zip(&rot) {
            placed.push(check::transform(Transform { pos: *p, rot: *q })?);
        }
        let flesh = Material::Flesh.props();
        let kick = check::velocity_gain(impulse, capsule_mass(rig.shapes()[0], flesh.density()))?;

        // Every check is done: nothing below can fail, so a refusal never leaves half a ragdoll.
        self.drop_group(GroupId::Ragdoll(slot));
        while self.count_ragdolls() >= self.caps.ragdolls {
            let Some(oldest) = self
                .groups
                .iter()
                .position(|g| matches!(g.id, GroupId::Ragdoll(_)))
            else {
                break;
            };
            self.recycle_at(oldest);
        }
        self.make_room(need);

        let overlaps = receipt::unjoined_overlaps(rig, &placed);
        let layer = layer_of(&GroupId::Ragdoll(slot));
        let mut bodies = Vec::with_capacity(need);
        let mut handles = Vec::with_capacity(need);
        for (bone, (at, shape)) in placed.iter().zip(rig.shapes()).enumerate() {
            let mut v = velocity;
            if bone == 0 {
                v = [v[0] + kick[0], v[1] + kick[1], v[2] + kick[2]];
            }
            let key = Key::Bone {
                slot,
                bone: bone as u16,
            };
            let collider = ColliderBuilder::capsule_y(shape.half_height, shape.radius);
            let (index, handle) = self.insert_body(key, *at, collider, flesh, v, layer);
            bodies.push(index);
            handles.push(handle);
        }
        insert_joints(
            rig.skeleton(),
            &pos,
            &rot,
            rig.depths(),
            &handles,
            &mut self.world.impulse_joints,
        );
        self.make_hinges(rig, &placed, &handles);
        let filtered = self.switch_off_overlaps(rig, &overlaps, &handles);
        let receipt = self.measure_ragdoll(rig, &handles, overlaps.len(), filtered);
        let drive = self.spawn_drive(rig, &placed, &handles);
        let parents = (0..need).map(|bone| rig.parent(bone)).collect();
        self.groups.push(Group {
            id: GroupId::Ragdoll(slot),
            bodies,
            receipt: Some(receipt),
            drive: Some(drive),
            parents,
            muscle_permille: rig.muscle_permille(),
        });
        Ok(())
    }

    /// Detaches one limb of flesh as a free body. `part` names it for the read-back and for a
    /// respawn.
    pub fn spawn_limb(
        &mut self,
        part: u32,
        shape: BoneShape,
        at: Transform,
        velocity: [f32; 3],
    ) -> Result<(), Error> {
        self.spawn_limb_of(Material::Flesh, part, shape, at, velocity)
    }

    /// Detaches one limb as a free body made of `material` (a wooden leg, an iron arm).
    pub fn spawn_limb_of(
        &mut self,
        material: Material,
        part: u32,
        shape: BoneShape,
        at: Transform,
        velocity: [f32; 3],
    ) -> Result<(), Error> {
        check::shape(shape)?;
        let at = check::transform(at)?;
        check::vec3(velocity, MAX_SPEED, Field::Velocity)?;
        self.spawn_single(
            GroupId::Limb(part),
            Key::Limb { part },
            material.props(),
            shape,
            at,
            velocity,
        );
        Ok(())
    }

    /// Scatters stone pieces: `spawn_debris_of` with [`Material::Stone`].
    pub fn spawn_debris(&mut self, shards: &[Shard], impulse: [f32; 3]) -> Result<Batch, Error> {
        self.spawn_debris_of(Material::Stone, shards, impulse)
    }

    /// Scatters the pieces of a break, all made of `material`. `impulse` (newton-seconds) is the
    /// blast's total momentum: every admitted shard gains `impulse / total mass` on top of its
    /// own velocity, so a heavier material flies slower. A list longer than the body cap is cut
    /// to the cap, in the caller's order; [`Batch::admitted`] says how many went in. Every
    /// shard is checked, admitted or not.
    pub fn spawn_debris_of(
        &mut self,
        material: Material,
        shards: &[Shard],
        impulse: [f32; 3],
    ) -> Result<Batch, Error> {
        let props = material.props();
        if shards.is_empty() {
            return Err(Error::NoShards);
        }
        check::vec3(impulse, MAX_IMPULSE, Field::Impulse)?;
        let mut checked = Vec::with_capacity(shards.len());
        for shard in shards {
            let at = check::transform(shard.at)?;
            check::vec3(shard.velocity, MAX_SPEED, Field::Velocity)?;
            for h in shard.half {
                check::extent(h)?;
            }
            checked.push((at, shard.half, shard.velocity));
        }
        let admitted = checked.len().min(self.caps.bodies);
        let mass: f32 = checked[..admitted]
            .iter()
            .map(|(_, half, _)| box_mass(*half, props.density()))
            .sum();
        let kick = check::velocity_gain(impulse, mass)?;

        self.make_room(admitted);
        let id = self.next_batch;
        self.next_batch = self.next_batch.wrapping_add(1);
        let layer = layer_of(&GroupId::Shard);
        for (i, (at, half, velocity)) in checked[..admitted].iter().enumerate() {
            let v = [
                velocity[0] + kick[0],
                velocity[1] + kick[1],
                velocity[2] + kick[2],
            ];
            let key = Key::Shard {
                batch: id,
                index: i as u32,
            };
            let collider = ColliderBuilder::cuboid(half[0], half[1], half[2]);
            let (index, _) = self.insert_body(key, *at, collider, props, v, layer);
            self.groups.push(Group {
                id: GroupId::Shard,
                bodies: vec![index],
                receipt: None,
                drive: None,
                parents: Vec::new(),
                muscle_permille: 0,
            });
        }
        Ok(Batch { id, admitted })
    }

    /// Advances the world one tick (60 Hz). Call it once per sim tick.
    pub fn step(&mut self) {
        let started = (self.clock)();
        self.fade_drives();
        self.world.step();
        self.step_tethers();
        self.steps += 1;
        self.capture();
        self.shed_contacts();
        self.micros = (self.clock)().saturating_sub(started);
    }

    /// Every body's transform, oldest spawn first, blended between the last two ticks:
    /// `alpha = 0` is the previous tick, `1` the latest. An alpha outside `[0, 1]` is clamped;
    /// a non-finite one is refused. A sleeping body reads the same at every alpha.
    pub fn transforms(&self, alpha: f32) -> Result<Vec<Placed>, Error> {
        if !alpha.is_finite() {
            return Err(Error::NonFinite(Field::Alpha));
        }
        let t = alpha.clamp(0.0, 1.0);
        let mut out = Vec::with_capacity(self.live);
        for group in &self.groups {
            for index in &group.bodies {
                if let Some(Some(rec)) = self.records.get(*index as usize) {
                    out.push(Placed {
                        key: rec.key,
                        at: Transform::blend(rec.prev, rec.curr, t),
                    });
                }
            }
        }
        Ok(out)
    }

    /// The (friction, restitution, density) the collider of every body was built with, spawn
    /// order not guaranteed: lets a test check what the world really holds against the table.
    #[cfg(test)]
    pub(crate) fn collider_numbers(&self) -> Vec<(f32, f32, f32)> {
        self.records
            .iter()
            .flatten()
            .filter_map(|rec| self.world.colliders.get(rec.collider))
            .map(|c| (c.friction(), c.restitution(), c.density()))
            .collect()
    }

    /// The collision groups of every collider in the world, the static boxes included: lets a
    /// test check that each body carries the group and the mask of its table layer.
    #[cfg(test)]
    pub(crate) fn collider_groups(&self) -> Vec<InteractionGroups> {
        self.world
            .colliders
            .iter()
            .map(|(_, collider)| collider.collision_groups())
            .collect()
    }

    /// The collision groups of the collider of the body named by `key`: `None` when the world
    /// holds no such body.
    #[cfg(test)]
    pub(crate) fn groups_of(&self, key: Key) -> Option<InteractionGroups> {
        let rec = self.records.iter().flatten().find(|rec| rec.key == key)?;
        self.world
            .colliders
            .get(rec.collider)
            .map(|collider| collider.collision_groups())
    }

    /// The number of contact pairs the body named by `key` holds with anything.
    #[cfg(test)]
    pub(crate) fn contact_count(&self, key: Key) -> usize {
        let Some(rec) = self.records.iter().flatten().find(|rec| rec.key == key) else {
            return 0;
        };
        self.world
            .narrow_phase
            .contact_pairs_with(rec.collider)
            .filter(|pair| pair.has_any_active_contact())
            .count()
    }

    /// Counters as of the last step.
    pub fn stats(&self) -> Stats {
        Stats {
            bodies: self.live,
            awake: self.world.islands.num_active_bodies(),
            contacts: self.contacts,
            recycled: self.recycled,
            steps: self.steps,
            micros: self.micros,
        }
    }

    /// Makes room for and inserts one free capsule as a group of its own, replacing a group
    /// with the same id. The caller has checked every input. Returns the Rapier handle.
    fn spawn_single(
        &mut self,
        id: GroupId,
        key: Key,
        props: Props,
        shape: BoneShape,
        at: Transform,
        velocity: [f32; 3],
    ) -> RigidBodyHandle {
        self.drop_group(id);
        self.make_room(1);
        let layer = layer_of(&id);
        let collider = ColliderBuilder::capsule_y(shape.half_height, shape.radius);
        let (index, handle) = self.insert_body(key, at, collider, props, velocity, layer);
        self.groups.push(Group {
            id,
            bodies: vec![index],
            receipt: None,
            drive: None,
            parents: Vec::new(),
            muscle_permille: 0,
        });
        handle
    }

    /// Inserts one dynamic body with a record; returns its slab index and Rapier handle. The
    /// collider takes the group and mask of the table layer it is on.
    fn insert_body(
        &mut self,
        key: Key,
        at: Transform,
        collider: ColliderBuilder,
        material: Props,
        velocity: [f32; 3],
        layer: u32,
    ) -> (u32, RigidBodyHandle) {
        let index = match self.free.pop() {
            Some(index) => index,
            None => {
                self.records.push(None);
                (self.records.len() - 1) as u32
            }
        };
        let mut body = RigidBodyBuilder::dynamic()
            .translation(Vector::new(at.pos[0], at.pos[1], at.pos[2]))
            .linvel(Vector::new(velocity[0], velocity[1], velocity[2]))
            .linear_damping(material.linear_damping())
            .angular_damping(material.angular_damping())
            .user_data(u128::from(index))
            .build();
        body.set_rotation(
            Rotation::from_xyzw(at.rot[0], at.rot[1], at.rot[2], at.rot[3]),
            false,
        );
        let handle = self.world.bodies.insert(body);
        let collider = collider
            .density(material.density())
            .friction(material.friction())
            .restitution(material.restitution())
            .collision_groups(groups::of(layer))
            .build();
        let collider =
            self.world
                .colliders
                .insert_with_parent(collider, handle, &mut self.world.bodies);
        self.records[index as usize] = Some(Record {
            handle,
            collider,
            key,
            prev: at,
            curr: at,
            stamp: 0,
        });
        self.live += 1;
        (index, handle)
    }

    /// Reads every awake body once; a body that fell asleep since the last step is settled so
    /// its two stored poses agree and the blend stops moving it.
    fn capture(&mut self) {
        self.scratch.clear();
        for handle in self.world.islands.active_bodies() {
            let Some(body) = self.world.bodies.get(handle) else {
                continue;
            };
            let Ok(index) = usize::try_from(body.user_data) else {
                continue;
            };
            let Some(Some(rec)) = self.records.get_mut(index) else {
                continue;
            };
            if rec.handle != handle {
                continue;
            }
            rec.prev = rec.curr;
            rec.curr = pose_of(body);
            rec.stamp = self.steps;
            self.scratch.push(index as u32);
        }
        for index in &self.awake {
            let Some(Some(rec)) = self.records.get_mut(*index as usize) else {
                continue;
            };
            if rec.stamp == self.steps {
                continue;
            }
            if let Some(body) = self.world.bodies.get(rec.handle) {
                let pose = pose_of(body);
                rec.prev = pose;
                rec.curr = pose;
            }
        }
        std::mem::swap(&mut self.awake, &mut self.scratch);
    }

    /// Keeps the contact count under the cap by recycling the oldest limbs and debris. Ragdolls
    /// are never shed: they are the fighters.
    fn shed_contacts(&mut self) {
        let mut contacts = self.count_contacts();
        self.contacts = contacts;
        let mut shed = 0;
        while contacts > self.caps.contacts && shed < SHED_PER_STEP {
            let Some(oldest) = self
                .groups
                .iter()
                .position(|g| !matches!(g.id, GroupId::Ragdoll(_)))
            else {
                break;
            };
            contacts = contacts.saturating_sub(self.group_contacts(oldest));
            self.recycle_at(oldest);
            shed += 1;
        }
    }

    /// Active contact pairs that cost the solver something: touching, with an awake dynamic side.
    fn count_contacts(&self) -> usize {
        self.world
            .narrow_phase
            .contact_pairs()
            .filter(|pair| self.pair_is_live(pair))
            .count()
    }

    fn group_contacts(&self, group: usize) -> usize {
        let mut total = 0;
        for index in &self.groups[group].bodies {
            if let Some(Some(rec)) = self.records.get(*index as usize) {
                total += self
                    .world
                    .narrow_phase
                    .contact_pairs_with(rec.collider)
                    .filter(|pair| self.pair_is_live(pair))
                    .count();
            }
        }
        total
    }

    fn pair_is_live(&self, pair: &ContactPair) -> bool {
        pair.has_any_active_contact()
            && [pair.collider1, pair.collider2].iter().any(|c| {
                self.world
                    .colliders
                    .get(*c)
                    .and_then(|collider| collider.parent())
                    .and_then(|parent| self.world.bodies.get(parent))
                    .is_some_and(|body| body.is_dynamic() && !body.is_sleeping())
            })
    }

    fn count_ragdolls(&self) -> usize {
        self.groups
            .iter()
            .filter(|g| matches!(g.id, GroupId::Ragdoll(_)))
            .count()
    }

    /// Recycles the oldest groups until `need` more bodies fit. `need` is at most the cap.
    fn make_room(&mut self, need: usize) {
        while self.live + need > self.caps.bodies && !self.groups.is_empty() {
            self.recycle_at(0);
        }
    }

    /// Removes a group to honor a cap and counts its bodies as recycled.
    fn recycle_at(&mut self, group: usize) {
        self.recycled += self.remove_at(group) as u64;
    }

    /// Removes the group with this id if there is one (a replacement, not a recycling).
    fn drop_group(&mut self, id: GroupId) {
        if let Some(at) = self.groups.iter().position(|g| g.id == id) {
            self.remove_at(at);
        }
    }

    /// Takes a group out of the world and the slab; returns how many bodies it held.
    fn remove_at(&mut self, group: usize) -> usize {
        let group = self.groups.remove(group);
        for index in &group.bodies {
            if let Some(rec) = self.records.get_mut(*index as usize).and_then(Option::take) {
                self.world.remove_body(rec.handle);
                self.free.push(*index);
                self.live = self.live.saturating_sub(1);
            }
        }
        group.bodies.len()
    }
}

fn pose_of(body: &RigidBody) -> Transform {
    let t = body.translation();
    let r = body.rotation();
    Transform {
        pos: [t.x, t.y, t.z],
        rot: [r.x, r.y, r.z, r.w],
    }
}

/// Mass of a capsule: a cylinder of `2 * half_height` plus the two end caps (one sphere).
fn capsule_mass(shape: BoneShape, density: f32) -> f32 {
    let r = shape.radius;
    let cylinder = PI * r * r * 2.0 * shape.half_height;
    let sphere = 4.0 / 3.0 * PI * r * r * r;
    density * (cylinder + sphere)
}

fn box_mass(half: [f32; 3], density: f32) -> f32 {
    density * 8.0 * half[0] * half[1] * half[2]
}
