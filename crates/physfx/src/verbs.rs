//! verbs: the five fx calls. Each one turns an event into a typed receipt.
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
use crate::soft::{ClothDesc, ClothError, SoftReceipt};
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

/// A cloth was asked for: the sheet to hang.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClothEvent {
    /// The cape or banner to hang.
    pub desc: ClothDesc,
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

/// What a cloth call hung: its stable slot and its receipt at the spawn tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClothFx {
    /// The cloth's slot in [`PhysFx::cloths`].
    pub slot: usize,
    /// The spawn receipt: size and a zero sag until the world steps.
    pub receipt: SoftReceipt,
}

impl ClothFx {
    /// The one line the page and the window print.
    #[must_use]
    pub fn receipt_line(&self) -> String {
        self.receipt.receipt_line()
    }
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
pub fn fx_limb(world: &mut PhysFx, body: &BodyDesc, event: &LimbEvent) -> Result<LimbFx, Error> {
    let bone = body.bone(&event.bone).ok_or_else(|| Error::UnknownBone {
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
    world.spawn_limb_of(
        crate::Material::Flesh,
        event.part,
        shape,
        event.at,
        event.velocity,
    )?;
    Ok(LimbFx {
        part: event.part,
        bone: event.bone.clone(),
        energy,
        gore: body.gore_permille,
    })
}

/// Paints a wall that broke: spawns the pieces and answers with how many went
/// in. The body's gore rides along in the receipt for the picture.
pub fn fx_break(world: &mut PhysFx, body: &BodyDesc, event: &BreakEvent) -> Result<BreakFx, Error> {
    let batch = world.spawn_debris_of(crate::Material::Stone, &event.shards, event.impulse)?;
    Ok(BreakFx {
        batch: batch.id,
        admitted: batch.admitted,
        gore: body.gore_permille,
    })
}

/// Paints a blast: shoves the bodies inside it and answers with how many
/// moved. The body's gore rides along in the receipt for the picture.
pub fn fx_blast(world: &mut PhysFx, body: &BodyDesc, event: &BlastEvent) -> Result<BlastFx, Error> {
    let pushed = world.apply_blast(event.blast)?;
    Ok(BlastFx {
        pushed,
        gore: body.gore_permille,
    })
}

/// Hangs a cape or a banner and answers with its spawn receipt. The cloth moves
/// with [`PhysFx::step`]; read [`PhysFx::cloth_receipt`] after the steps for its
/// sag. A refusal leaves the world as it was.
pub fn fx_cloth(world: &mut PhysFx, event: &ClothEvent) -> Result<ClothFx, ClothError> {
    let slot = world.spawn_cloth(event.desc)?;
    let receipt = world.cloth_receipt(slot).ok_or(ClothError::Missing)?;
    Ok(ClothFx { slot, receipt })
}

#[cfg(test)]
mod fx_verbs {
    //! fx_verbs: the four fx calls over the committed fighter body.

    use super::*;
    use crate::body::{load, BodyDesc, BodyFileError};
    use crate::caps::Tier;
    use crate::types::Blast;
    use engine_anim::contract::LocalPose;
    use std::path::Path;

    /// The fifteen physical bones of the fighter, in the file's parent-first
    /// order: the CT7 rig the look draws.
    const CT7: [&str; 15] = [
        "pelvis",
        "torso",
        "head",
        "leg_upper_l",
        "leg_lower_l",
        "foot_l",
        "leg_upper_r",
        "leg_lower_r",
        "foot_r",
        "arm_upper_l",
        "arm_lower_l",
        "palm_l",
        "arm_upper_r",
        "arm_lower_r",
        "palm_r",
    ];

    /// The pose the body stands in: each bone's offset from its parent, meters.
    const OFFSETS: [(&str, [f32; 3]); 15] = [
        ("pelvis", [0.0, 1.1, 0.0]),
        ("torso", [0.0, 0.17, 0.0]),
        ("head", [0.0, 0.5, 0.0]),
        ("leg_upper_l", [-0.1, -0.25, 0.0]),
        ("leg_lower_l", [0.0, -0.4, 0.0]),
        ("foot_l", [0.0, -0.3, 0.0]),
        ("leg_upper_r", [0.1, -0.25, 0.0]),
        ("leg_lower_r", [0.0, -0.4, 0.0]),
        ("foot_r", [0.0, -0.3, 0.0]),
        ("arm_upper_l", [-0.25, -0.1, 0.0]),
        ("arm_lower_l", [0.0, -0.25, 0.0]),
        ("palm_l", [0.0, -0.2, 0.0]),
        ("arm_upper_r", [0.25, -0.1, 0.0]),
        ("arm_lower_r", [0.0, -0.25, 0.0]),
        ("palm_r", [0.0, -0.2, 0.0]),
    ];

    /// The committed fighter body, read once.
    fn fighter() -> BodyDesc {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        load(&root.join("../../content/bodies/fighter.toml")).expect("the fighter body loads")
    }

    /// The pose of `body`, in the body file's own bone order.
    fn pose_of(body: &BodyDesc) -> LocalPose {
        let positions = body
            .bones
            .iter()
            .map(|bone| {
                OFFSETS
                    .iter()
                    .find(|(name, _)| *name == bone.name)
                    .expect("bone offset")
                    .1
            })
            .collect();
        LocalPose {
            positions,
            rotations: vec![Transform::IDENTITY.rot; body.bones.len()],
        }
    }

    /// A world on a floor whose top is at y = 0.
    fn arena() -> PhysFx {
        let mut world = PhysFx::new(Tier::Comfort);
        world
            .add_static_box([50.0, 0.5, 50.0], [0.0, -0.5, 0.0])
            .expect("floor");
        world
    }

    fn at(pos: [f32; 3]) -> Transform {
        Transform {
            pos,
            ..Transform::IDENTITY
        }
    }

    fn fell(pose: LocalPose) -> RagdollEvent {
        RagdollEvent {
            slot: 0,
            pose,
            velocity: [0.0; 3],
            impulse: [0.0; 3],
        }
    }

    fn limb() -> LimbEvent {
        LimbEvent {
            slot: 0,
            bone: "arm_lower_l".to_string(),
            part: 7,
            at: at([0.0, 1.0, 0.0]),
            velocity: [1.0, 2.0, 0.0],
        }
    }

    fn shards() -> Vec<Shard> {
        [[-1.0, 1.0, 4.0], [-0.5, 1.0, 4.0], [0.0, 1.0, 4.0]]
            .iter()
            .map(|pos| Shard {
                at: at(*pos),
                half: [0.1; 3],
                velocity: [0.0; 3],
            })
            .collect()
    }

    fn wall() -> BreakEvent {
        BreakEvent {
            shards: shards(),
            impulse: [0.0, 200.0, 0.0],
        }
    }

    fn blast() -> BlastEvent {
        BlastEvent {
            blast: Blast {
                center_mm: [0, 1270, 0],
                radius_mm: 5000,
                impulse_ns: 1000,
            },
        }
    }

    #[test]
    fn each_fx_call_answers_a_receipt_for_the_fighter_body() {
        let body = fighter();
        let names: Vec<&str> = body.bones.iter().map(|bone| bone.name.as_str()).collect();
        assert_eq!(names, CT7, "the body file carries the CT7 rig's bones");
        assert_eq!(
            body.cuts.len(),
            CT7.len() - 1,
            "every joint but the root cuts"
        );

        let mut ragg = arena();
        let receipt = fx_ragdoll(&mut ragg, &body, &fell(pose_of(&body))).expect("ragdoll");
        assert_eq!(receipt.slot, 0);
        assert_eq!(receipt.bones, CT7.len());
        assert_eq!(receipt.muscle, body.muscle_permille);
        assert_eq!(receipt.gore, body.gore_permille);

        let mut limbs = arena();
        let receipt = fx_limb(&mut limbs, &body, &limb()).expect("limb");
        assert_eq!(receipt.bone, "arm_lower_l");
        assert_eq!(
            Some(receipt.energy),
            body.cut_energy("arm_lower_l"),
            "the receipt carries the file's price"
        );
        assert_eq!(receipt.gore, body.gore_permille);

        let mut walls = arena();
        let receipt = fx_break(&mut walls, &body, &wall()).expect("break");
        assert_eq!(receipt.admitted, 3);
        assert_eq!(receipt.gore, body.gore_permille);

        let mut blasts = arena();
        fx_ragdoll(&mut blasts, &body, &fell(pose_of(&body))).expect("ragdoll");
        let receipt = fx_blast(&mut blasts, &body, &blast()).expect("blast");
        assert_eq!(receipt.pushed, CT7.len());
        assert_eq!(receipt.gore, body.gore_permille);
    }

    #[test]
    fn the_same_event_twice_gives_the_same_receipt() {
        let body = fighter();
        let pose = pose_of(&body);

        let ragdoll = || {
            let mut world = arena();
            fx_ragdoll(&mut world, &body, &fell(pose.clone())).expect("ragdoll")
        };
        assert_eq!(ragdoll(), ragdoll());

        let limb_once = || {
            let mut world = arena();
            fx_limb(&mut world, &body, &limb()).expect("limb")
        };
        assert_eq!(limb_once(), limb_once());

        let wall_once = || {
            let mut world = arena();
            fx_break(&mut world, &body, &wall()).expect("break")
        };
        assert_eq!(wall_once(), wall_once());

        let blast_once = || {
            let mut world = arena();
            fx_ragdoll(&mut world, &body, &fell(pose.clone())).expect("ragdoll");
            fx_blast(&mut world, &body, &blast()).expect("blast")
        };
        assert_eq!(blast_once(), blast_once());
    }

    #[test]
    fn a_body_file_that_is_not_there_names_its_path() {
        let path = std::env::temp_dir().join(format!(
            "engine2040-physfx-missing-{}.toml",
            std::process::id()
        ));
        let error = load(&path).expect_err("there is no body file");
        assert!(matches!(error, BodyFileError::Read { .. }), "{error}");
        let text = error.to_string();
        assert!(text.contains("could not be read"), "{text}");
        assert!(text.contains("engine2040-physfx-missing"), "{text}");
    }

    #[test]
    fn a_bad_body_file_names_the_key_and_its_line() {
        let path =
            std::env::temp_dir().join(format!("engine2040-physfx-bad-{}.toml", std::process::id()));
        std::fs::write(
            &path,
            "name = \"fighter\"\ngore_permille = 1000\nsheen = 3\n",
        )
        .expect("write the bad body");
        let error = load(&path).expect_err("an unknown key is refused");
        let text = error.to_string();
        assert!(text.contains("`sheen`"), "{text}");
        assert!(text.contains("line 3"), "{text}");

        std::fs::write(
            &path,
            "name = \"fighter\"\ngore_permille = 1000\n\n[bone.pelvis]\nparent = \"\"\nradius = 0.09\nhalf_height = 0.0\n",
        )
        .expect("write the incomplete body");
        let error = load(&path).expect_err("a missing key is refused");
        assert!(error.to_string().contains("`muscle_permille`"), "{error}");

        std::fs::remove_file(&path).expect("remove the test body");
    }
}
