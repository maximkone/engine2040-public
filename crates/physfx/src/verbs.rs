//! verbs: the four fx calls. Each one turns an event into a typed receipt.
//!
//! Owner: this crate. A caller names a body and hands over what the sim said;
//! the verb paints it in the float world and answers with a receipt that says
//! what happened, in numbers the page and the window print.
//!
//! Units: meters and seconds for the world, permille for the gore, whole
//! newton-seconds for the impulses and the cut energies.
//!
//! Determinism: the same event on the same world gives the same receipt. A
//! receipt holds no clock, no randomness and no digest: it is printed, never
//! hashed.
//!
//! Errors: typed, and a refusal leaves the world exactly as it was.
//!
//! Budget: one call per event, not per tick.

use crate::body::BodyDesc;
use crate::error::Error;
use crate::fx::PhysFx;
use crate::types::{Blast, Shard, Transform};
use engine_anim::contract::LocalPose;

/// A fighter fell: the slot it lies in, the pose it fell in and the blow that
/// put it down.
#[derive(Clone, Debug, PartialEq)]
pub struct RagdollEvent {
    /// The fighter's slot.
    pub slot: u8,
    /// Where the bones were, one entry per bone of the body, parents first.
    pub pose: LocalPose,
    /// How fast the fighter was moving, meters per second.
    pub velocity: [f32; 3],
    /// The blow that put it down, newton-seconds.
    pub impulse: [f32; 3],
}

/// A limb came off: the bone it was, the part id it is known by and where it
/// left.
#[derive(Clone, Debug, PartialEq)]
pub struct LimbEvent {
    /// The fighter's slot.
    pub slot: u8,
    /// The bone that came off, by the name the body file gives it.
    pub bone: String,
    /// The part id the severed piece is known by.
    pub part: u32,
    /// Where the piece is.
    pub at: Transform,
    /// How fast the piece leaves, meters per second.
    pub velocity: [f32; 3],
}

/// A wall broke: the pieces it broke into and the break's momentum.
#[derive(Clone, Debug, PartialEq)]
pub struct BreakEvent {
    /// The pieces, in the order the sim raised them.
    pub shards: Vec<Shard>,
    /// The break's total momentum, newton-seconds.
    pub impulse: [f32; 3],
}

/// A blast went off: where it went off and how far it reaches, in the sim's own
/// integers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlastEvent {
    /// The blast the sim raised.
    pub blast: Blast,
}

/// What a ragdoll call painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RagdollFx {
    /// The fighter's slot.
    pub slot: u8,
    /// The bones the ragdoll has.
    pub bones: usize,
    /// Bone pairs that overlapped at the spawn.
    pub overlaps: usize,
    /// Of those, the grandparent pairs whose contacts were switched off.
    pub filtered: usize,
    /// How hard the muscles brace, permille of the whole.
    pub muscle: u16,
    /// How much blood the body sheds, permille of the whole.
    pub gore: u16,
}

/// What a limb call painted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LimbFx {
    /// The part id the severed piece is known by.
    pub part: u32,
    /// The bone that came off.
    pub bone: String,
    /// The energy the body says that cut costs, newton-seconds.
    pub energy: u32,
    /// How much blood the body sheds, permille of the whole.
    pub gore: u16,
}

/// What a break call painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BreakFx {
    /// The batch id the pieces are known by.
    pub batch: u32,
    /// The pieces that went in.
    pub admitted: usize,
    /// How much blood the body sheds, permille of the whole.
    pub gore: u16,
}

/// What a blast call painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlastFx {
    /// The bodies the blast shoved.
    pub pushed: usize,
    /// How much blood the body sheds, permille of the whole.
    pub gore: u16,
}

/// Paints a fighter that fell: spawns the ragdoll of that body and answers
/// with what it measured.
///
/// The body's bones give the capsules, the body's muscle gives the brace
/// strength, and the body's gore rides along in the receipt for the picture. A
/// refusal leaves the world as it was.
pub fn fx_ragdoll(
    world: &mut PhysFx,
    body: &BodyDesc,
    event: &RagdollEvent,
) -> Result<RagdollFx, Error> {
    let rig = body.rig()?;
    world.spawn_ragdoll(event.slot, &rig, &event.pose, event.velocity, event.impulse)?;
    let receipt = world.ragdoll_receipt(event.slot);
    Ok(RagdollFx {
        slot: event.slot,
        bones: rig.bones(),
        overlaps: receipt.map_or(0, |r| r.overlaps),
        filtered: receipt.map_or(0, |r| r.filtered),
        muscle: body.muscle_permille,
        gore: body.gore_permille,
    })
}

/// Paints a limb that came off: spawns the piece and answers with what it is.
///
/// The body says which joints a blade may take and what that cut costs; the
/// sim says the piece came off. A joint the body does not carry, or one a
/// blade cannot take, is refused and the world is left as it was.
pub fn fx_limb(
    world: &mut PhysFx,
    body: &BodyDesc,
    event: &LimbEvent,
) -> Result<LimbFx, Error> {
    let bone = body
        .bone(&event.bone)
        .ok_or_else(|| Error::UnknownBone {
            name: event.bone.clone(),
        })?;
    let energy = body
        .cut_energy(&event.bone)
        .ok_or_else(|| Error::UncuttableJoint {
            joint: event.bone.clone(),
        })?;
    let shape = crate::BoneShape {
        radius: bone.radius,
        half_height: bone.half_height,
    };
    world.spawn_limb_of(crate::Material::Flesh, event.part, shape, event.at, event.velocity)?;
    Ok(LimbFx {
        part: event.part,
        bone: event.bone.clone(),
        energy,
        gore: body.gore_permille,
    })
}

/// Paints a wall that broke: spawns the pieces and answers with how many went
/// in. The body's gore rides along in the receipt for the picture.
pub fn fx_break(
    world: &mut PhysFx,
    body: &BodyDesc,
    event: &BreakEvent,
) -> Result<BreakFx, Error> {
    let batch = world.spawn_debris_of(crate::Material::Stone, &event.shards, event.impulse)?;
    Ok(BreakFx {
        batch: batch.id,
        admitted: batch.admitted,
        gore: body.gore_permille,
    })
}

/// Paints a blast: shoves the bodies inside it and answers with how many
/// moved. The body's gore rides along in the receipt for the picture.
pub fn fx_blast(
    world: &mut PhysFx,
    body: &BodyDesc,
    event: &BlastEvent,
) -> Result<BlastFx, Error> {
    let pushed = world.apply_blast(event.blast)?;
    Ok(BlastFx {
        pushed,
        gore: body.gore_permille,
    })
}
