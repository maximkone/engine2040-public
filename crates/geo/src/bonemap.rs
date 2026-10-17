//! bonemap: name the 22 humanoid bones from an imported rig's joint names (std only, no
//! feature).
//!
//! Owner: `engine-geo`; consumer: a retarget or a view that needs a body part by name
//! (GEO6b), which passes its own joint list in. Units: none — a joint index is a `usize`
//! into that caller's order, never a pointer. Determinism: a pure, case-insensitive
//! function of the names, no allocation beyond the returned vector, output sorted by
//! [`HumanoidBone`] with at most one joint per bone, so a cook run twice gives the same
//! vector. Errors: none; a name that fits no bone is simply absent.

use HumanoidBone::{
    Chest, Head, Hips, LeftFoot, LeftHand, LeftLowerArm, LeftLowerLeg, LeftShoulder, LeftToes,
    LeftUpperArm, LeftUpperLeg, Neck, RightFoot, RightHand, RightLowerArm, RightLowerLeg,
    RightShoulder, RightToes, RightUpperArm, RightUpperLeg, Spine, UpperChest,
};

/// The 22 humanoid bones in report order: Hips, Spine, Chest, UpperChest, Neck, Head,
/// then Left and Right of Shoulder, UpperArm, LowerArm, Hand, UpperLeg, LowerLeg, Foot
/// and Toes. The first six are central and take a joint with no side; the rest need one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HumanoidBone {
    Hips,
    Spine,
    Chest,
    UpperChest,
    Neck,
    Head,
    LeftShoulder,
    RightShoulder,
    LeftUpperArm,
    RightUpperArm,
    LeftLowerArm,
    RightLowerArm,
    LeftHand,
    RightHand,
    LeftUpperLeg,
    RightUpperLeg,
    LeftLowerLeg,
    RightLowerLeg,
    LeftFoot,
    RightFoot,
    LeftToes,
    RightToes,
}

impl HumanoidBone {
    /// The report name of this bone (`"Hips"`, `"LeftUpperArm"`), one word per bone.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Hips => "Hips",
            Spine => "Spine",
            Chest => "Chest",
            UpperChest => "UpperChest",
            Neck => "Neck",
            Head => "Head",
            LeftShoulder => "LeftShoulder",
            RightShoulder => "RightShoulder",
            LeftUpperArm => "LeftUpperArm",
            RightUpperArm => "RightUpperArm",
            LeftLowerArm => "LeftLowerArm",
            RightLowerArm => "RightLowerArm",
            LeftHand => "LeftHand",
            RightHand => "RightHand",
            LeftUpperLeg => "LeftUpperLeg",
            RightUpperLeg => "RightUpperLeg",
            LeftLowerLeg => "LeftLowerLeg",
            RightLowerLeg => "RightLowerLeg",
            LeftFoot => "LeftFoot",
            RightFoot => "RightFoot",
            LeftToes => "LeftToes",
            RightToes => "RightToes",
        }
    }

    /// The side this bone needs; `None` for a central bone.
    fn side(self) -> Option<Side> {
        match self {
            Hips | Spine | Chest | UpperChest | Neck | Head => None,
            LeftShoulder | LeftUpperArm | LeftLowerArm | LeftHand | LeftUpperLeg | LeftLowerLeg
            | LeftFoot | LeftToes => Some(Side::Left),
            RightShoulder | RightUpperArm | RightLowerArm | RightHand | RightUpperLeg
            | RightLowerLeg | RightFoot | RightToes => Some(Side::Right),
        }
    }
}

/// Bone and the joint names a rig may give it, in [`HumanoidBone`] order: the matcher
/// walks this table, so a result comes out sorted by bone for free. A joint name matches
/// a stem exactly once separators (`. _ -` and space) are ignored, so `spine_01` is
/// `spine01`, and a stem is never a substring: `forearm` is not `arm` and
/// `leftarmtwist1` is nobody. Mixamo, Rigify, VRM, the UE mannequin and Blender names
/// all appear here.
const BONES: [(HumanoidBone, &[&str]); 22] = [
    (Hips, &["hips", "pelvis"]),
    (Spine, &["spine", "spine01"]),
    (Chest, &["chest", "spine1", "spine02", "spine001"]),
    (UpperChest, &["upperchest", "spine2", "spine03", "spine002"]),
    (Neck, &["neck", "neck01"]),
    (Head, &["head"]),
    (LeftShoulder, &["shoulder", "clavicle", "collar"]),
    (RightShoulder, &["shoulder", "clavicle", "collar"]),
    (LeftUpperArm, &["upperarm", "arm"]),
    (RightUpperArm, &["upperarm", "arm"]),
    (LeftLowerArm, &["lowerarm", "forearm", "elbow"]),
    (RightLowerArm, &["lowerarm", "forearm", "elbow"]),
    (LeftHand, &["hand", "wrist"]),
    (RightHand, &["hand", "wrist"]),
    (LeftUpperLeg, &["upperleg", "upleg", "thigh", "femur"]),
    (RightUpperLeg, &["upperleg", "upleg", "thigh", "femur"]),
    (LeftLowerLeg, &["lowerleg", "calf", "shin", "leg", "knee"]),
    (RightLowerLeg, &["lowerleg", "calf", "shin", "leg", "knee"]),
    (LeftFoot, &["foot", "ankle"]),
    (RightFoot, &["foot", "ankle"]),
    (LeftToes, &["toes", "toe", "ball", "toebase"]),
    (RightToes, &["toes", "toe", "ball", "toebase"]),
];

/// Rig prefixes dropped before matching, with the side a VRM arm prefix declares.
const PREFIXES: [(&str, Option<Side>); 7] = [
    ("mixamorig", None),
    ("j_bip_c_", None),
    ("j_bip_l_", Some(Side::Left)),
    ("j_bip_r_", Some(Side::Right)),
    ("cc_base_", None),
    ("bip001", None),
    ("def-", None),
];

/// The side a joint name declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

/// The humanoid bone each name in `joint_names` names, with its joint index, sorted by
/// bone and holding at most one joint per bone.
///
/// Names are read case-insensitively: a namespace up to the last `:` and one known rig
/// prefix (`mixamorig`, `def-`, `j_bip_c_`, `j_bip_l_`, `j_bip_r_`, `cc_base_`,
/// `bip001`) are dropped, a side comes from a `left`/`right` word or a leading or
/// trailing `l`/`r`/`left`/`right` token, separators (`. _ -` and space) never matter,
/// and a namespace token a rig glued on (`char_Hips`) is skipped. The first joint that
/// names a bone keeps it; a joint that fits no bone (a finger, a twist, a tail) is left
/// out rather than guessed, and a legless skeleton returns only the bones it has.
#[must_use]
pub fn auto_map(joint_names: &[&str]) -> Vec<(HumanoidBone, usize)> {
    // One slot per bone, in table order, so the result is sorted by bone.
    let mut found = BONES.map(|(bone, _)| (bone, None::<usize>));
    for (index, raw) in joint_names.iter().enumerate() {
        if let Some(bone) = classify(raw) {
            for slot in &mut found {
                if slot.0 == bone {
                    slot.1.get_or_insert(index);
                    break;
                }
            }
        }
    }
    found
        .iter()
        .filter_map(|&(bone, joint)| joint.map(|joint| (bone, joint)))
        .collect()
}

/// The bone one joint name names, if any.
fn classify(raw: &str) -> Option<HumanoidBone> {
    // A namespace before the last ':' never carries the bone name.
    let mut name = raw.rsplit_once(':').map_or(raw, |(_, tail)| tail);
    let mut known = None;
    for (prefix, prefix_side) in PREFIXES {
        if let Some(rest) = strip_ci(name, prefix) {
            (name, known) = (rest, prefix_side);
            break;
        }
    }
    // A trailing `l`/`r`/`left`/`right` token is a side, not part of the stem.
    let (head, tail_side) = cut_last(name);
    if tail_side.is_some() {
        (name, known) = (head, tail_side);
    }
    // Longest candidate first, then from each later token on, so a namespace a rig glued
    // in front of the stem still reads.
    let mut start = 0;
    loop {
        let (candidate, here) = cut_side(&name[start..]);
        if let Some(bone) = best_match(candidate, known.or(here)) {
            return Some(bone);
        }
        let ahead = &name[start..];
        let sep = ahead.find(is_sep)?;
        start += sep + ahead[sep..].find(|c| !is_sep(c))?;
    }
}

/// The bone `candidate` names on `side`, if any: a stem has to name the joint exactly,
/// so a longer or shorter word is never a guess.
fn best_match(candidate: &str, side: Option<Side>) -> Option<HumanoidBone> {
    BONES
        .iter()
        .find(|(bone, stems)| {
            bone.side() == side && stems.iter().any(|stem| eat(candidate, stem) == Some(""))
        })
        .map(|(bone, _)| *bone)
}

/// Consume `stem` at the front of `text`, ignoring the separators in `text`, and return
/// the text after it (case-insensitively). `None` when it does not match.
fn eat<'a>(text: &'a str, stem: &str) -> Option<&'a str> {
    let mut rest = text;
    for want in stem.chars() {
        rest = rest.trim_start_matches(is_sep);
        let mut chars = rest.chars();
        match chars.next() {
            Some(c) if c.eq_ignore_ascii_case(&want) => rest = chars.as_str(),
            _ => return None,
        }
    }
    Some(rest.trim_start_matches(is_sep))
}

/// `text` without the ASCII-case-insensitive `prefix`, when it starts with it.
fn strip_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// The side a single token names, if it is one.
fn side_of(token: &str) -> Option<Side> {
    if token.eq_ignore_ascii_case("l") || token.eq_ignore_ascii_case("left") {
        return Some(Side::Left);
    }
    (token.eq_ignore_ascii_case("r") || token.eq_ignore_ascii_case("right")).then_some(Side::Right)
}

/// `text` without a trailing side token, and that token's side.
fn cut_last(text: &str) -> (&str, Option<Side>) {
    match text.trim_end_matches(is_sep).rsplit_once(is_sep) {
        Some((head, token)) if side_of(token).is_some() => (head, side_of(token)),
        _ => (text, None),
    }
}

/// `text` without a leading side: a lone `l`/`r`/`left`/`right` token, or that word
/// glued to the front (`LeftShoulder`).
fn cut_side(text: &str) -> (&str, Option<Side>) {
    let head = text.trim_start_matches(is_sep);
    let (token, tail) = head.split_once(is_sep).unwrap_or((head, ""));
    if let Some(side) = side_of(token) {
        return (tail.trim_start_matches(is_sep), Some(side));
    }
    for (word, side) in [("left", Side::Left), ("right", Side::Right)] {
        if let Some(rest) = strip_ci(text, word) {
            return (rest, Some(side));
        }
    }
    (text, None)
}

/// A separator a rig name may put between tokens: never part of a stem.
fn is_sep(c: char) -> bool {
    matches!(c, '_' | '.' | '-' | ' ')
}

#[cfg(test)]
mod tests {
    //! Five rig conventions in four forms each: 20 fixtures, 22 bones apiece.
    use super::*;

    /// `prefix|left|right|` then the 14 stems in [`BONES`] order, `*` the side slot:
    /// Mixamo, Rigify, VRM, the UE mannequin and Blender.
    const CONVENTIONS: [&str; 5] = [
        concat!(
            "mixamorig:|Left|Right|Hips|Spine|Spine1|Spine2|Neck|Head|",
            "*Shoulder|*Arm|*ForeArm|*Hand|*UpLeg|*Leg|*Foot|*ToeBase"
        ),
        concat!(
            "DEF-|L|R|hips|spine|spine.001|spine.002|neck|head|",
            "shoulder.*|upper_arm.*|forearm.*|hand.*|thigh.*|shin.*|foot.*|toe.*"
        ),
        concat!(
            "J_Bip_|L_|R_|C_Hips|C_Spine|C_Chest|C_UpperChest|C_Neck|C_Head|",
            "*Shoulder|*UpperArm|*LowerArm|*Hand|*UpperLeg|*LowerLeg|*Foot|*Toes"
        ),
        concat!(
            "|l|r|pelvis|spine_01|spine_02|spine_03|neck_01|head|",
            "clavicle_*|upperarm_*|lowerarm_*|hand_*|thigh_*|calf_*|foot_*|ball_*"
        ),
        concat!(
            "|L|R|Hips|Spine|Chest|UpperChest|Neck|Head|",
            "Shoulder.*|UpperArm.*|LowerArm.*|Hand.*|UpperLeg.*|LowerLeg.*|Foot.*|Toes.*"
        ),
    ];

    /// The fixture line split into prefix, both sides and the 14 stems.
    fn parts(conv: &str) -> (&str, &str, &str, Vec<&str>) {
        let fields: Vec<&str> = conv.split('|').collect();
        assert_eq!(fields.len(), 17, "{conv}");
        (fields[0], fields[1], fields[2], fields[3..].to_vec())
    }

    /// The 22 joint names the convention gives the bones, in [`BONES`] order.
    fn real(conv: &str) -> Vec<String> {
        let (prefix, left, right, stems) = parts(conv);
        (0..22)
            .map(|i| {
                let (stem, side) = match i {
                    0..=5 => (stems[i], ""),
                    _ if i % 2 == 0 => (stems[6 + (i - 6) / 2], left),
                    _ => (stems[6 + (i - 6) / 2], right),
                };
                format!("{prefix}{}", stem.replace('*', side))
            })
            .collect()
    }

    /// The 30 extra finger and twist joints of the shuffled fixture, named the
    /// convention's way: none of them names a humanoid bone.
    fn extras(conv: &str) -> Vec<String> {
        const FINGERS: &str = concat!(
            "Thumb1|Thumb2|Thumb3|Index1|Index2|Index3|Middle1|Middle2|Middle3|",
            "Ring1|Ring2|Pinky1|Pinky2|ArmTwist1|ForeArmTwist1"
        );
        let (prefix, left, right, stems) = parts(conv);
        let at_front = stems[6].starts_with('*');
        let mut out = Vec::with_capacity(30);
        for finger in FINGERS.split('|') {
            for side in [left, right] {
                out.push(if at_front {
                    format!("{prefix}{side}{finger}")
                } else {
                    format!("{prefix}{finger}_{side}")
                });
            }
        }
        out
    }

    /// One fixture: its joint names, and the joint each bone in [`BONES`] names.
    fn fixture(conv: &str, variant: u8) -> (Vec<String>, Vec<(HumanoidBone, usize)>) {
        let names = real(conv);
        let wanted: Vec<String> = match variant {
            1 => names.iter().map(|name| name.to_uppercase()).collect(),
            2 => names.iter().map(|name| format!("char_{name}")).collect(),
            _ => names,
        };
        let mut joints = Vec::with_capacity(52);
        let mut order = vec![0; 22];
        if variant == 3 {
            // Shuffled: the 30 extra joints around a scramble of the real names.
            let mut extra = extras(conv).into_iter();
            for i in 0..22 {
                joints.extend(extra.next());
                let bone = (i * 7) % 22;
                order[bone] = joints.len();
                joints.push(wanted[bone].clone());
            }
            joints.extend(extra);
        } else {
            for (bone, joint) in wanted.iter().enumerate() {
                order[bone] = joints.len();
                joints.push(joint.clone());
            }
        }
        let expected = BONES.iter().map(|(bone, _)| *bone).zip(order).collect();
        (joints, expected)
    }

    #[test]
    fn twenty_fixtures_map_all_22_bones() {
        let mut fixtures = 0;
        for conv in CONVENTIONS {
            for variant in 0..4u8 {
                fixtures += 1;
                let (joints, expected) = fixture(conv, variant);
                let refs: Vec<&str> = joints.iter().map(String::as_str).collect();
                let map = auto_map(&refs);
                assert_eq!(map, expected, "{conv} variant {variant}");
                for (seen, (bone, joint)) in map.iter().enumerate() {
                    let reused = map[..seen].iter().any(|(_, other)| other == joint);
                    assert!(!reused, "{bone:?} repeats a joint in variant {variant}");
                }
            }
        }
        assert_eq!(fixtures, 20);
    }

    #[test]
    fn legless_rig_gets_only_the_bones_it_has() {
        // The Mixamo fixture without its leg joints: no leg bone is invented.
        let names = real(CONVENTIONS[0]);
        let joints: Vec<&str> = names[..14].iter().map(String::as_str).collect();
        let map = auto_map(&joints);
        assert_eq!(map.len(), 14);
        for (bone, _) in &map {
            assert!(*bone < HumanoidBone::LeftUpperLeg, "{bone:?} is a leg bone");
        }
        // A joint that fits no bone is absent; empty input is not a panic.
        let junk = ["mixamorig:Tail1", "mixamorig:Jaw"];
        assert!(auto_map(&junk).is_empty());
        assert!(auto_map(&[]).is_empty());
    }

    #[test]
    fn first_joint_keeps_a_bone_and_no_index_repeats() {
        let map = auto_map(&[
            "mixamorig:Hips",
            "mixamorig:Pelvis",
            "mixamorig:Spine",
            "mixamorig:LeftUpLeg",
        ]);
        assert_eq!(
            map,
            vec![
                (HumanoidBone::Hips, 0),
                (HumanoidBone::Spine, 2),
                (HumanoidBone::LeftUpperLeg, 3)
            ]
        );
    }

    #[test]
    fn names_sides_and_table_order_line_up() {
        let names: Vec<&str> = BONES.iter().map(|(bone, _)| bone.name()).collect();
        assert_eq!(
            names.join(","),
            concat!(
                "Hips,Spine,Chest,UpperChest,Neck,Head,LeftShoulder,RightShoulder,",
                "LeftUpperArm,RightUpperArm,LeftLowerArm,RightLowerArm,LeftHand,RightHand,",
                "LeftUpperLeg,RightUpperLeg,LeftLowerLeg,RightLowerLeg,LeftFoot,RightFoot,",
                "LeftToes,RightToes"
            )
        );
        for (i, (bone, stems)) in BONES.iter().enumerate() {
            // The table order is the bone order, so a result comes out sorted by bone.
            for (other, _) in &BONES[..i] {
                assert!(other < bone, "{bone:?} is out of table order");
            }
            assert!(!stems.is_empty(), "{bone:?} has a stem");
            let named = match bone.name() {
                name if name.starts_with("Left") => Some(Side::Left),
                name if name.starts_with("Right") => Some(Side::Right),
                _ => None,
            };
            assert_eq!(bone.side(), named, "{bone:?} and its name disagree");
        }
        // No stem names two bones on one side: every stem stays reachable.
        for (a, stems) in BONES {
            for (b, others) in BONES {
                let shared = a != b
                    && a.side() == b.side()
                    && stems.iter().any(|stem| others.contains(stem));
                assert!(!shared, "{a:?} and {b:?} share a stem on one side");
            }
        }
    }
}
