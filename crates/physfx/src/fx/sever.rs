//! sever: one bone of a ragdoll leaves the body as a limb group of its own (QR12).
//!
//! An authority cut names a bone. Severing removes the joint between that bone and its parent
//! and moves the bone's body out of the ragdoll group into a [`GroupId::Limb`] group, keeping
//! the pose and the velocity it had. The bones below it keep their joints to it, so the whole
//! chain falls with it, and the ragdoll keeps every other bone: a fifteen-bone rig leaves
//! fourteen. Nothing here is authority: the cut is the truth and this only paints what it says.
//!
//! Consumer: [`PhysFx::sever`], called by `crates/game/src/ragdoll_look.rs`
//! (`FighterLook::sever_ragdoll`) when the arena raises a limb-loss event. The look draws the
//! stump cap from the [`Severed`] record; the arena does not raise that event yet.

use super::{GroupId, PhysFx};
use crate::error::{Error, Field};
use crate::types::Key;
use rapier3d::dynamics::RigidBodyHandle;

/// What one sever took: the body that left and the joint that no longer holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Severed {
    /// Bone index in the rig's skeleton, the bone the cut named.
    pub bone: u16,
    /// Its parent bone in the rig; the joint between the two was removed.
    pub parent: Option<u16>,
    /// Id of the limb group the body moved to; a later sever with the same id replaces it.
    pub part: u32,
}

impl PhysFx {
    /// Severs the bone `bone` of the ragdoll in `slot` into the limb group `part`: the joint to
    /// the bone's parent is removed and the bone's body leaves the ragdoll group with the pose
    /// and velocity it had. The bones below it stay jointed to it, so the chain falls with it;
    /// the ragdoll keeps every other bone. One bone severs once: a second call is refused.
    /// `Ok(None)` when the slot has no ragdoll. A bone the rig does not have, or the root (it
    /// has no parent joint), is refused with [`Error::OutOfRange`] and changes nothing.
    pub fn sever(&mut self, slot: u8, bone: u16, part: u32) -> Result<Option<Severed>, Error> {
        let Some(group) = self
            .groups
            .iter()
            .position(|g| g.id == GroupId::Ragdoll(slot))
        else {
            return Ok(None);
        };
        let at = usize::from(bone);
        let parent = match self.groups[group].parents.get(at) {
            Some(Some(parent)) => *parent,
            Some(None) => return Err(Error::OutOfRange(Field::Bone)),
            None => return Err(Error::OutOfRange(Field::Bone)),
        };
        let key = Key::Bone { slot, bone };
        let Some(index) = self.body_at(group, key) else {
            return Err(Error::OutOfRange(Field::Bone));
        };
        // Everything is checked: nothing below can fail, so a refusal leaves the body joined.
        let Some(bone_handle) = self.handle_of(key) else {
            return Err(Error::OutOfRange(Field::Bone));
        };
        let parent_bone = u16::try_from(parent).map_err(|_| Error::OutOfRange(Field::Bone))?;
        let Some(parent_handle) = self.handle_of(Key::Bone {
            slot,
            bone: parent_bone,
        }) else {
            return Err(Error::OutOfRange(Field::Bone));
        };
        let joints: Vec<_> = self
            .world
            .impulse_joints
            .joints_between(parent_handle, bone_handle)
            .map(|(handle, _)| handle)
            .collect();
        for handle in &joints {
            self.world.impulse_joints.remove(*handle, true);
        }
        // The brace of a hit holds the joint that just left; drop its entry so the fade never
        // reads a stale handle.
        if let Some(drive) = self.groups[group].drive.as_mut() {
            drive.joints.retain(|(handle, _)| !joints.contains(handle));
        }
        let body = self.groups[group].bodies.remove(index);
        // A re-sever of the same id replaces the old limb, like every other spawn.
        self.drop_group(GroupId::Limb(part));
        self.groups.push(super::Group {
            id: GroupId::Limb(part),
            bodies: vec![body],
            receipt: None,
            drive: None,
            parents: Vec::new(),
            muscle_permille: 0,
        });
        Ok(Some(Severed {
            bone,
            parent: Some(parent_bone),
            part,
        }))
    }

    /// Bones the ragdoll in `slot` still holds, or `None` without one. A sever lowers it by one;
    /// the severed bone's chain below it stays counted, because its bodies never left the world.
    pub fn ragdoll_bones(&self, slot: u8) -> Option<usize> {
        self.groups
            .iter()
            .find(|g| g.id == GroupId::Ragdoll(slot))
            .map(|g| g.bodies.len())
    }

    /// Bodies the limb group `part` holds, or `None` when no such group exists.
    pub fn limb_bodies(&self, part: u32) -> Option<usize> {
        self.groups
            .iter()
            .find(|g| g.id == GroupId::Limb(part))
            .map(|g| g.bodies.len())
    }

    /// Where the body of `key` sits in `group`'s body list, if it is there.
    fn body_at(&self, group: usize, key: Key) -> Option<usize> {
        let bodies = &self.groups[group].bodies;
        let mut found = None;
        for (at, body) in bodies.iter().enumerate() {
            let record = usize::try_from(*body)
                .ok()
                .and_then(|index| self.records.get(index))
                .and_then(Option::as_ref);
            if record.is_some_and(|record| record.key == key) {
                found = Some(at);
                break;
            }
        }
        found
    }

    /// The Rapier body of `key`, wherever the group bookkeeping put it.
    fn handle_of(&self, key: Key) -> Option<RigidBodyHandle> {
        self.records
            .iter()
            .flatten()
            .find(|record| record.key == key)
            .map(|record| record.handle)
    }
}
