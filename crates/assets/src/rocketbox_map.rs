//! The Rocketbox name map: the engine's fighter joints to a Rocketbox body.
//!
//! Microsoft Rocketbox avatars (MIT) carry a 3ds Max Biped skeleton: 80 bones
//! named `Bip01 Pelvis`, `Bip01 L UpperArm`, `Bip01 R Finger12` and a face
//! rig. The cook renames them into the anim contract's charset the obvious
//! way (lowercase, a space becomes `_`): `bip01_pelvis`, `bip01_l_upperarm`.
//! A baked combat pack is written against the fighter rig (SOMA-style names,
//! see [`crate::fighter`]), so this table bridges the two, the way
//! [`crate::ct7_map`] does for the CT7 body.
//!
//! The table is written out, not derived, for the same reason as the CT7 one:
//! a rule that guessed would quietly drive the wrong bone.
//!
//! The fingers: the fighter rig has four bones per finger, the first one in
//! the palm (`lefthandindex1` sits 4 cm past the wrist, `lefthandindex2` at
//! the knuckle). The Biped has the three phalanges only, starting at the
//! knuckle, so `Finger1`, `Finger11`, `Finger12` take index 2, 3 and 4. The
//! Biped thumb starts in the palm like the fighter's, so `Finger0`, `Finger01`,
//! `Finger02` take thumb 1, 2 and 3.
//!
//! Joints with no row (the face bones, the eyes, the `bip01` root) carry no
//! fighter motion and hold their bind pose: the retarget's documented
//! fallback.

use crate::ct7_map::fighter_track_names;
use crate::retarget::{BoneMap, RetargetError};
use engine_anim::contract::Skeleton;

/// A Rocketbox joint (contract charset), paired with the fighter track that
/// drives it.
pub static ROCKETBOX_TO_BODY: [(&str, &str); 52] = [
    ("bip01_pelvis", "hips"),
    ("bip01_spine", "spine"),
    ("bip01_spine1", "spine1"),
    ("bip01_spine2", "spine2"),
    ("bip01_neck", "neck"),
    ("bip01_head", "head"),
    ("bip01_l_clavicle", "leftshoulder"),
    ("bip01_l_upperarm", "leftarm"),
    ("bip01_l_forearm", "leftforearm"),
    ("bip01_l_hand", "lefthand"),
    ("bip01_l_finger0", "lefthandthumb1"),
    ("bip01_l_finger01", "lefthandthumb2"),
    ("bip01_l_finger02", "lefthandthumb3"),
    ("bip01_l_finger1", "lefthandindex2"),
    ("bip01_l_finger11", "lefthandindex3"),
    ("bip01_l_finger12", "lefthandindex4"),
    ("bip01_l_finger2", "lefthandmiddle2"),
    ("bip01_l_finger21", "lefthandmiddle3"),
    ("bip01_l_finger22", "lefthandmiddle4"),
    ("bip01_l_finger3", "lefthandring2"),
    ("bip01_l_finger31", "lefthandring3"),
    ("bip01_l_finger32", "lefthandring4"),
    ("bip01_l_finger4", "lefthandpinky2"),
    ("bip01_l_finger41", "lefthandpinky3"),
    ("bip01_l_finger42", "lefthandpinky4"),
    ("bip01_r_clavicle", "rightshoulder"),
    ("bip01_r_upperarm", "rightarm"),
    ("bip01_r_forearm", "rightforearm"),
    ("bip01_r_hand", "righthand"),
    ("bip01_r_finger0", "righthandthumb1"),
    ("bip01_r_finger01", "righthandthumb2"),
    ("bip01_r_finger02", "righthandthumb3"),
    ("bip01_r_finger1", "righthandindex2"),
    ("bip01_r_finger11", "righthandindex3"),
    ("bip01_r_finger12", "righthandindex4"),
    ("bip01_r_finger2", "righthandmiddle2"),
    ("bip01_r_finger21", "righthandmiddle3"),
    ("bip01_r_finger22", "righthandmiddle4"),
    ("bip01_r_finger3", "righthandring2"),
    ("bip01_r_finger31", "righthandring3"),
    ("bip01_r_finger32", "righthandring4"),
    ("bip01_r_finger4", "righthandpinky2"),
    ("bip01_r_finger41", "righthandpinky3"),
    ("bip01_r_finger42", "righthandpinky4"),
    ("bip01_l_thigh", "leftupleg"),
    ("bip01_l_calf", "leftleg"),
    ("bip01_l_foot", "leftfoot"),
    ("bip01_l_toe0", "lefttoebase"),
    ("bip01_r_thigh", "rightupleg"),
    ("bip01_r_calf", "rightleg"),
    ("bip01_r_foot", "rightfoot"),
    ("bip01_r_toe0", "righttoebase"),
];

/// A Biped bone name in the contract charset: `Bip01 L UpperArm` becomes
/// `bip01_l_upperarm`. The cook names the bones this way; the map uses it
/// so a raw name and a cooked name land on the same row.
#[must_use]
pub fn contract_name(biped: &str) -> String {
    biped
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// True when the body carries a Rocketbox skeleton (its pelvis is the Biped
/// one), so a caller can pick this map over the CT7 one.
#[must_use]
pub fn is_rocketbox(body: &Skeleton) -> bool {
    body.joints
        .iter()
        .any(|joint| contract_name(&joint.name) == "bip01_pelvis")
}

/// The bone map from the fighter rig onto a Rocketbox body: for each body
/// joint with a row, the fighter track that drives it.
///
/// A row whose joint the body lacks is skipped (the body decides which bones
/// exist); a row naming a fighter track the rig does not have is refused loud.
pub fn rocketbox_map(body: &Skeleton) -> Result<BoneMap, RetargetError> {
    let rig = crate::fighter::fighter_skeleton().map_err(|err| RetargetError::MissingBone {
        name: err.to_string(),
    })?;
    let tracks = fighter_track_names()?;
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    for (row, fighter) in ROCKETBOX_TO_BODY {
        if !tracks.iter().any(|name| name == fighter) {
            return Err(RetargetError::MissingBone {
                name: fighter.to_string(),
            });
        }
        let Some(joint) = body
            .joints
            .iter()
            .find(|joint| contract_name(&joint.name) == row)
        else {
            continue;
        };
        pairs.push((joint.name.as_str(), fighter));
    }
    BoneMap::build(&rig, body, &pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_anim::contract::Joint;

    /// The Rocketbox skeleton as the cook names it (from the FBX names of Male_Adult_08: the skeletal bones and two face bones that stay unmapped).
    const BIPED: [(&str, Option<usize>); 20] = [
        ("Bip01 Pelvis", None),
        ("Bip01 Spine", Some(0)),
        ("Bip01 Spine1", Some(1)),
        ("Bip01 Spine2", Some(2)),
        ("Bip01 Neck", Some(3)),
        ("Bip01 Head", Some(4)),
        ("Bip01 MJaw", Some(5)),
        ("Bip01 LEye", Some(5)),
        ("Bip01 L Clavicle", Some(4)),
        ("Bip01 L UpperArm", Some(8)),
        ("Bip01 L Forearm", Some(9)),
        ("Bip01 L Hand", Some(10)),
        ("Bip01 L Finger0", Some(11)),
        ("Bip01 L Finger1", Some(11)),
        ("Bip01 L Finger11", Some(13)),
        ("Bip01 L Finger12", Some(14)),
        ("Bip01 L Thigh", Some(1)),
        ("Bip01 L Calf", Some(16)),
        ("Bip01 L Foot", Some(17)),
        ("Bip01 L Toe0", Some(18)),
    ];

    fn body() -> Skeleton {
        Skeleton {
            joints: BIPED
                .iter()
                .map(|(name, parent)| Joint {
                    name: contract_name(name),
                    parent: *parent,
                })
                .collect(),
        }
    }

    fn fighter_index(name: &str) -> usize {
        fighter_track_names()
            .expect("the fighter rig names itself")
            .iter()
            .position(|track| track == name)
            .expect("the fighter rig has the track")
    }

    /// How many body joints the map drives (`len` counts every body joint).
    fn mapped(map: &BoneMap, joints: usize) -> usize {
        (0..joints).filter(|j| map.source_of(*j).is_some()).count()
    }

    #[test]
    fn every_row_names_a_real_fighter_track_and_a_contract_name() {
        let tracks = fighter_track_names().expect("the fighter rig names itself");
        for (row, fighter) in ROCKETBOX_TO_BODY {
            assert!(
                tracks.iter().any(|t| t == fighter),
                "{fighter} is a fighter track"
            );
            assert_eq!(
                contract_name(row),
                row,
                "{row} is already in the contract charset"
            );
        }
        // No fighter track drives two body joints.
        let mut seen: Vec<&str> = ROCKETBOX_TO_BODY.iter().map(|(_, f)| *f).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            seen.len(),
            ROCKETBOX_TO_BODY.len(),
            "one body joint per track"
        );
    }

    #[test]
    fn the_biped_names_become_contract_names() {
        assert_eq!(contract_name("Bip01 L UpperArm"), "bip01_l_upperarm");
        assert_eq!(contract_name("Bip01 R Finger12"), "bip01_r_finger12");
    }

    #[test]
    fn a_cooked_rocketbox_body_maps_by_its_contract_names() {
        let skeleton = body();
        assert!(is_rocketbox(&skeleton));
        let map = rocketbox_map(&skeleton).expect("the map builds");
        // 20 bones, two face bones unmapped.
        assert_eq!(
            mapped(&map, skeleton.joints.len()),
            18,
            "every skeletal bone has a row"
        );
        let at = |name: &str| {
            skeleton
                .joints
                .iter()
                .position(|joint| joint.name == name)
                .expect("the body has the joint")
        };
        assert_eq!(
            map.source_of(at("bip01_pelvis")),
            Some(fighter_index("hips"))
        );
        assert_eq!(
            map.source_of(at("bip01_l_upperarm")),
            Some(fighter_index("leftarm"))
        );
        // The knuckle is the fighter's second index bone, not the palm one.
        assert_eq!(
            map.source_of(at("bip01_l_finger1")),
            Some(fighter_index("lefthandindex2"))
        );
        assert_eq!(
            map.source_of(at("bip01_l_finger0")),
            Some(fighter_index("lefthandthumb1"))
        );
        assert_eq!(
            map.source_of(at("bip01_mjaw")),
            None,
            "the jaw holds its bind"
        );
    }

    #[test]
    fn a_ct7_body_is_not_a_rocketbox_body() {
        let ct7 = Skeleton {
            joints: vec![Joint {
                name: "pelvis".to_string(),
                parent: None,
            }],
        };
        assert!(!is_rocketbox(&ct7));
        assert_eq!(
            mapped(&rocketbox_map(&ct7).expect("an empty map builds"), 1),
            0
        );
    }
}
