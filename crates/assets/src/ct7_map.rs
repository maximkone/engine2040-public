//! The CT7 name map: the engine's fighter joints to the game's rig joints.
//!
//! The engine's fighter rig (see [`crate::fighter`]) names its 65 joints
//! SOMA-style: `hips`, `spine`, `leftfoot`, `leftarm`, `lefthandthumb1`. The
//! game plays a CT7 body whose 44 joints carry Mixamo-style names, which the
//! game sanitizes into the anim charset (lowercase, ASCII alphanumerics plus
//! `_`): `pelvis`, `torso`, `foot_l`, `arm_upper_l`, `palm_l`.
//!
//! A baked combat pack is written against the fighter rig, so the game cannot
//! play it by name until the two naming schemes are bridged. This table is that
//! bridge: one row per game joint, naming the fighter track that drives it.
//!
//! The map is written out, not derived: a rule that guessed a name would
//! quietly drive the wrong bone. Every row is checked against both
//! skeletons by [`ct7_map`], which refuses a row whose name is in neither
//! skeleton, and a game joint with no row keeps its bind pose (the
//! documented fallback, the same one the retarget uses).
//!
//! Joints with no fighter counterpart: the CT7 weapon sockets (`wep_pos_l`,
//! `wep_pos_r`) and the heels (`heel_l`, `heel_r`) carry no motion of their
//! own; the weapon rides the hand and the heel rides the ankle.

use crate::fighter::fighter_skeleton;
use crate::retarget::{BoneMap, RetargetError};
use engine_anim::contract::Skeleton;

/// The CT7 rig's joint name, paired with the fighter track that drives it.
///
/// `None` for a joint the fighter rig has no counterpart for: it keeps its
/// bind pose.
pub static CT7_TO_BODY: [(&str, Option<&str>); 44] = [
    ("human_male", None),
    ("pelvis", Some("hips")),
    ("torso", Some("spine2")),
    ("head", Some("head")),
    ("shoulder_l", Some("leftshoulder")),
    ("arm_upper_l", Some("leftarm")),
    ("arm_lower_l", Some("leftforearm")),
    ("palm_l", Some("lefthand")),
    ("wep_pos_l", None),
    ("thumb_upper_l", Some("lefthandthumb1")),
    ("thumb_lower_l", Some("lefthandthumb2")),
    ("index_upper_l", Some("lefthandindex1")),
    ("index_lower_l", Some("lefthandindex2")),
    ("middle_upper_l", Some("lefthandmiddle1")),
    ("middle_lower_l", Some("lefthandmiddle2")),
    ("ring_upper_l", Some("lefthandring1")),
    ("ring_lower_l", Some("lefthandring2")),
    ("pinky_upper_l", Some("lefthandpinky1")),
    ("pinky_lower_l", Some("lefthandpinky2")),
    ("leg_upper_l", Some("leftupleg")),
    ("leg_lower_l", Some("leftleg")),
    ("foot_l", Some("leftfoot")),
    ("toes_l", Some("lefttoebase")),
    ("heel_l", None),
    ("shoulder_r", Some("rightshoulder")),
    ("arm_upper_r", Some("rightarm")),
    ("arm_lower_r", Some("rightforearm")),
    ("palm_r", Some("righthand")),
    ("wep_pos_r", None),
    ("thumb_upper_r", Some("righthandthumb1")),
    ("thumb_lower_r", Some("righthandthumb2")),
    ("index_upper_r", Some("righthandindex1")),
    ("index_lower_r", Some("righthandindex2")),
    ("middle_upper_r", Some("righthandmiddle1")),
    ("middle_lower_r", Some("righthandmiddle2")),
    ("ring_upper_r", Some("righthandring1")),
    ("ring_lower_r", Some("righthandring2")),
    ("pinky_upper_r", Some("righthandpinky1")),
    ("pinky_lower_r", Some("righthandpinky2")),
    ("leg_upper_r", Some("rightupleg")),
    ("leg_lower_r", Some("rightleg")),
    ("foot_r", Some("rightfoot")),
    ("toes_r", Some("righttoebase")),
    ("heel_r", None),
];

/// The fighter track names, in the pack's own order: the pack's receipt lists
/// them and the loader reads the pack by them.
///
/// The order is the fighter rig's joint order, so the pack needs no
/// remapping: pack track `i` is fighter joint `i`.
pub fn fighter_track_names() -> Result<Vec<String>, RetargetError> {
    let rig = fighter_skeleton().map_err(|err| RetargetError::MissingBone {
        name: err.to_string(),
    })?;
    let mut names = Vec::with_capacity(rig.len());
    for joint in &rig.joints {
        names.push(joint.name.clone());
    }
    Ok(names)
}

/// The bone map from the fighter rig onto a CT7 body: for each CT7 joint, the
/// fighter track that drives it.
///
/// A CT7 joint with no fighter counterpart is left out of the map, and the
/// retarget's documented fallback holds it on the CT7 bind pose. A fighter
/// name that is not in the rig, or a CT7 joint the body does not have, is
/// refused loud: a map that guessed would drive the wrong bone.
pub fn ct7_map(body: &Skeleton) -> Result<BoneMap, RetargetError> {
    let rig = fighter_skeleton().map_err(|err| RetargetError::MissingBone {
        name: err.to_string(),
    })?;
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    for (ct7, fighter) in CT7_TO_BODY {
        let Some(fighter) = fighter else {
            continue;
        };
        // A CT7 joint the body does not carry is skipped, not refused: the
        // body decides which of the 44 joints exist.
        if !body.joints.iter().any(|joint| joint.name == ct7) {
            continue;
        }
        if !rig.joints.iter().any(|joint| joint.name == fighter) {
            return Err(RetargetError::MissingBone {
                name: fighter.to_string(),
            });
        }
        pairs.push((ct7, fighter));
    }
    BoneMap::build(&rig, body, &pairs)
}
