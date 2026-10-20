//! soma: the SOMA-77 joint table and its name map onto the fighter-65 rig.
//!
//! Consumer: the offline motion bake builds a `BoneMap` from [`soma_pairs`]
//! to land a teacher clip on the fighter rig; the tick never reads this
//! module. Data owner: `engine-assets`.
//! Source: nv-tlabs/kimodo `kimodo/skeleton/definitions.py`,
//! `SOMASkeleton77.bone_order_names_with_parents` (Apache-2.0), rows 86-164;
//! targets: the fighter profile `joints65` (65 bones).
//! Names are lowercased because the anim contract accepts `[a-z0-9_]` only.
//! Determinism: fixed tables in parent-first order, string compares only.

/// SOMA-77: `(joint, parent)` in parent-first order, lowercased. The root's
/// parent is `None`.
pub static SOMA77: [(&str, Option<&str>); 77] = [
    ("hips", None),
    ("spine1", Some("hips")),
    ("spine2", Some("spine1")),
    ("chest", Some("spine2")),
    ("neck1", Some("chest")),
    ("neck2", Some("neck1")),
    ("head", Some("neck2")),
    ("headend", Some("head")),
    ("jaw", Some("head")),
    ("lefteye", Some("head")),
    ("righteye", Some("head")),
    ("leftshoulder", Some("chest")),
    ("leftarm", Some("leftshoulder")),
    ("leftforearm", Some("leftarm")),
    ("lefthand", Some("leftforearm")),
    ("lefthandthumb1", Some("lefthand")),
    ("lefthandthumb2", Some("lefthandthumb1")),
    ("lefthandthumb3", Some("lefthandthumb2")),
    ("lefthandthumbend", Some("lefthandthumb3")),
    ("lefthandindex1", Some("lefthand")),
    ("lefthandindex2", Some("lefthandindex1")),
    ("lefthandindex3", Some("lefthandindex2")),
    ("lefthandindex4", Some("lefthandindex3")),
    ("lefthandindexend", Some("lefthandindex4")),
    ("lefthandmiddle1", Some("lefthand")),
    ("lefthandmiddle2", Some("lefthandmiddle1")),
    ("lefthandmiddle3", Some("lefthandmiddle2")),
    ("lefthandmiddle4", Some("lefthandmiddle3")),
    ("lefthandmiddleend", Some("lefthandmiddle4")),
    ("lefthandring1", Some("lefthand")),
    ("lefthandring2", Some("lefthandring1")),
    ("lefthandring3", Some("lefthandring2")),
    ("lefthandring4", Some("lefthandring3")),
    ("lefthandringend", Some("lefthandring4")),
    ("lefthandpinky1", Some("lefthand")),
    ("lefthandpinky2", Some("lefthandpinky1")),
    ("lefthandpinky3", Some("lefthandpinky2")),
    ("lefthandpinky4", Some("lefthandpinky3")),
    ("lefthandpinkyend", Some("lefthandpinky4")),
    ("rightshoulder", Some("chest")),
    ("rightarm", Some("rightshoulder")),
    ("rightforearm", Some("rightarm")),
    ("righthand", Some("rightforearm")),
    ("righthandthumb1", Some("righthand")),
    ("righthandthumb2", Some("righthandthumb1")),
    ("righthandthumb3", Some("righthandthumb2")),
    ("righthandthumbend", Some("righthandthumb3")),
    ("righthandindex1", Some("righthand")),
    ("righthandindex2", Some("righthandindex1")),
    ("righthandindex3", Some("righthandindex2")),
    ("righthandindex4", Some("righthandindex3")),
    ("righthandindexend", Some("righthandindex4")),
    ("righthandmiddle1", Some("righthand")),
    ("righthandmiddle2", Some("righthandmiddle1")),
    ("righthandmiddle3", Some("righthandmiddle2")),
    ("righthandmiddle4", Some("righthandmiddle3")),
    ("righthandmiddleend", Some("righthandmiddle4")),
    ("righthandring1", Some("righthand")),
    ("righthandring2", Some("righthandring1")),
    ("righthandring3", Some("righthandring2")),
    ("righthandring4", Some("righthandring3")),
    ("righthandringend", Some("righthandring4")),
    ("righthandpinky1", Some("righthand")),
    ("righthandpinky2", Some("righthandpinky1")),
    ("righthandpinky3", Some("righthandpinky2")),
    ("righthandpinky4", Some("righthandpinky3")),
    ("righthandpinkyend", Some("righthandpinky4")),
    ("leftleg", Some("hips")),
    ("leftshin", Some("leftleg")),
    ("leftfoot", Some("leftshin")),
    ("lefttoebase", Some("leftfoot")),
    ("lefttoeend", Some("lefttoebase")),
    ("rightleg", Some("hips")),
    ("rightshin", Some("rightleg")),
    ("rightfoot", Some("rightshin")),
    ("righttoebase", Some("rightfoot")),
    ("righttoeend", Some("righttoebase")),
];

/// Renamed rows: `(fighter joint, soma joint)` where the names differ or a
/// plain name match binds the wrong joint. The spine and leg chains are
/// offset by one against SOMA, so `spine1` reads SOMA's `spine2`, and the
/// fighter's `leftupleg` is SOMA's `leftleg` (SOMA names the shin `leftshin`).
pub static RENAMES: [(&str, &str); 13] = [
    ("spine", "spine1"),
    ("spine1", "spine2"),
    ("spine2", "chest"),
    ("neck", "neck1"),
    ("headtop_end", "headend"),
    ("leftupleg", "leftleg"),
    ("leftleg", "leftshin"),
    ("lefttoe_end", "lefttoeend"),
    ("rightupleg", "rightleg"),
    ("rightleg", "rightshin"),
    ("righttoe_end", "righttoeend"),
    ("lefthandthumb4", "lefthandthumbend"),
    ("righthandthumb4", "righthandthumbend"),
];

/// SOMA joints the fighter rig has no counterpart for: the jaw, both eyes,
/// the second neck joint and the eight finger end points.
pub static UNUSED_SOMA: [&str; 12] = [
    "jaw",
    "lefteye",
    "righteye",
    "neck2",
    "lefthandindexend",
    "lefthandmiddleend",
    "lefthandringend",
    "lefthandpinkyend",
    "righthandindexend",
    "righthandmiddleend",
    "righthandringend",
    "righthandpinkyend",
];

/// The fighter joint a SOMA joint feeds, if a rename places it.
fn renamed_target(soma: &str) -> Option<&'static str> {
    RENAMES
        .iter()
        .find(|(_, source)| *source == soma)
        .map(|(target, _)| *target)
}

/// The 65 `(fighter joint, soma joint)` pairs in SOMA parent-first order, so a
/// skeleton built from them is parent-first too. Names match exactly unless a
/// rename says otherwise; SOMA joints in [`UNUSED_SOMA`] are skipped.
pub fn soma_pairs() -> Vec<(&'static str, &'static str)> {
    let mut pairs = Vec::with_capacity(SOMA77.len() - UNUSED_SOMA.len());
    for &(soma, _parent) in SOMA77.iter() {
        if UNUSED_SOMA.contains(&soma) {
            continue;
        }
        pairs.push((renamed_target(soma).unwrap_or(soma), soma));
    }
    pairs
}

/// Coverage as `(exact, renamed, unused)`: 52 pairs keep their name, 13 take a
/// rename, and 12 SOMA joints have no fighter counterpart.
pub fn coverage() -> (usize, usize, usize) {
    let mut exact = 0usize;
    let mut renamed = 0usize;
    for (target, _soma) in soma_pairs() {
        if RENAMES.iter().any(|(name, _)| *name == target) {
            renamed += 1;
        } else {
            exact += 1;
        }
    }
    (exact, renamed, UNUSED_SOMA.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The anim contract's joint-name rule: `[a-z0-9_]`, no leading digit,
    /// at most 64 chars. Spelled here because this module has no anim edge.
    fn engine_name(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 64
            && !name.as_bytes()[0].is_ascii_digit()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    }

    fn soma_index(name: &str) -> usize {
        SOMA77
            .iter()
            .position(|(soma, _)| *soma == name)
            .unwrap_or_else(|| panic!("soma has no {name}"))
    }

    #[test]
    fn rows_are_parent_first_and_engine_named() {
        assert_eq!(SOMA77.len(), 77);
        let mut roots = 0;
        for (index, &(name, parent)) in SOMA77.iter().enumerate() {
            assert!(engine_name(name), "{name} is not an engine joint name");
            match parent {
                None => roots += 1,
                Some(parent) => {
                    assert!(engine_name(parent), "{parent} is not an engine joint name");
                    assert!(
                        soma_index(parent) < index,
                        "{name} names parent {parent} after itself"
                    );
                }
            }
        }
        assert_eq!(roots, 1, "exactly one root");
    }

    #[test]
    fn pairs_cover_fifty_two_exact_thirteen_renamed_twelve_unused() {
        let pairs = soma_pairs();
        assert_eq!(pairs.len(), 65);
        assert_eq!(coverage(), (52, 13, 12));
        let mut targets = Vec::with_capacity(pairs.len());
        for &(target, soma) in &pairs {
            assert!(engine_name(target), "{target} is not an engine joint name");
            assert!(!targets.contains(&target), "{target} is mapped twice");
            targets.push(target);
            assert!(!UNUSED_SOMA.contains(&soma), "{soma} is unused but mapped");
            assert!(
                SOMA77.iter().any(|(name, _)| *name == soma),
                "{soma} is not a SOMA joint"
            );
        }
        for &(soma, _parent) in SOMA77.iter() {
            let mapped = pairs.iter().any(|(_, source)| *source == soma);
            assert_eq!(
                mapped,
                !UNUSED_SOMA.contains(&soma),
                "{soma} is mapped or unused, never both"
            );
        }
        assert_eq!(soma_pairs(), pairs, "the pair order repeats");
    }

    #[test]
    fn renames_route_the_offset_chains() {
        assert_eq!(RENAMES.len(), 13);
        for &(target, soma) in RENAMES.iter() {
            assert_ne!(target, soma, "{target} needs no rename");
            assert!(engine_name(target) && engine_name(soma));
            assert!(
                SOMA77.iter().any(|(name, _)| *name == soma),
                "{soma} is not a SOMA joint"
            );
        }
        for (target, soma) in [
            ("spine", "spine1"),
            ("spine1", "spine2"),
            ("spine2", "chest"),
            ("leftupleg", "leftleg"),
            ("leftleg", "leftshin"),
            ("rightupleg", "rightleg"),
            ("rightleg", "rightshin"),
        ] {
            assert!(
                RENAMES.contains(&(target, soma)),
                "{target} must read SOMA {soma}"
            );
        }
    }
}
