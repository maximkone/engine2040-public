//! body: a body file is read into plain fields, and a wrong file says which line was wrong.

use crate::body::{parse, BodyError};

/// A body file with three bones and one cuttable joint.
const FILE: &str = r#"name = "tester"
gore_permille = 800
muscle_permille = 900

[bone.pelvis]
parent = ""
radius = 0.14
half_height = 0.05

[bone.spine]
parent = "pelvis"
radius = 0.13
half_height = 0.05

[bone.head]
parent = "spine"
radius = 0.11
half_height = 0.04

[cut.spine]
energy = 40
"#;

#[test]
fn a_body_file_is_read_into_its_fields() {
    let body = parse(FILE).expect("body file");
    assert_eq!(body.name, "tester");
    assert_eq!(body.gore_permille, 800);
    assert_eq!(body.muscle_permille, 900);
    assert_eq!(body.bones.len(), 3);
    assert_eq!(body.bones[0].name, "pelvis");
    assert_eq!(body.bones[0].parent, "");
    assert_eq!(body.bones[1].parent, "pelvis");
    assert!((body.bones[2].radius - 0.11).abs() < 1e-6);
    assert!((body.bones[2].half_height - 0.04).abs() < 1e-6);
    assert_eq!(body.cuts.len(), 1);
    assert_eq!(body.cut_energy("spine"), Some(40));
    assert_eq!(body.cut_energy("head"), None);
}

#[test]
fn the_same_file_reads_the_same_way_twice() {
    let first = parse(FILE).expect("body file");
    let second = parse(FILE).expect("body file");
    assert_eq!(first, second);
}

#[test]
fn a_body_gives_a_rig_of_its_bones_and_its_muscle() {
    let body = parse(FILE).expect("body file");
    let rig = body.rig().expect("rig");
    assert_eq!(rig.bones(), 3);
    assert_eq!(rig.muscle_permille(), 900);
}

#[test]
fn an_unknown_key_is_refused_with_its_line() {
    let file = "name = \"tester\"\ncolour = 1\n";
    let err = parse(file).expect_err("unknown key");
    assert_eq!(
        err,
        BodyError::UnknownKey {
            key: "colour".to_string(),
            line: 2,
        }
    );
}

#[test]
fn an_unknown_table_is_refused_with_the_line_of_the_key_in_it() {
    let file = "[wing.left]\nspan = 1\n";
    let err = parse(file).expect_err("unknown table");
    assert_eq!(
        err,
        BodyError::UnknownTable {
            table: "wing.left".to_string(),
            line: 2,
        }
    );
}

#[test]
fn the_same_key_twice_is_refused_with_its_line() {
    let file = "name = \"tester\"\nname = \"other\"\n";
    let err = parse(file).expect_err("duplicate");
    assert_eq!(
        err,
        BodyError::Duplicate {
            key: "name".to_string(),
            line: 2,
        }
    );
}

#[test]
fn a_key_the_body_needs_and_the_file_lacks_is_refused() {
    let file = "name = \"tester\"\n";
    let err = parse(file).expect_err("missing");
    assert_eq!(
        err,
        BodyError::Missing {
            key: "gore_permille".to_string(),
        }
    );
}

#[test]
fn a_muscle_past_the_whole_is_refused_with_its_line() {
    let file = "name = \"tester\"\ngore_permille = 800\nmuscle_permille = 1001\n";
    let err = parse(file).expect_err("too much muscle");
    assert_eq!(
        err,
        BodyError::BadValue {
            key: "muscle_permille".to_string(),
            value: "1001".to_string(),
            line: 3,
        }
    );
}

#[test]
fn a_muscle_that_is_not_a_whole_number_is_refused() {
    let file = "name = \"tester\"\ngore_permille = 800\nmuscle_permille = 0.9\n";
    let err = parse(file).expect_err("scale muscle");
    assert!(matches!(err, BodyError::BadValue { .. }), "{err:?}");
}

#[test]
fn a_bone_whose_parent_comes_later_is_refused() {
    let file = "name = \"tester\"\ngore_permille = 800\nmuscle_permille = 900\n\n\
                [bone.head]\nparent = \"spine\"\nradius = 0.11\nhalf_height = 0.04\n\n\
                [bone.spine]\nparent = \"pelvis\"\nradius = 0.13\nhalf_height = 0.05\n\n\
                [bone.pelvis]\nparent = \"\"\nradius = 0.14\nhalf_height = 0.05\n";
    let err = parse(file).expect_err("out of order");
    assert!(matches!(err, BodyError::Order { .. }), "{err:?}");
}

#[test]
fn a_cut_at_a_bone_the_body_does_not_have_is_refused() {
    let file = "name = \"tester\"\ngore_permille = 800\nmuscle_permille = 900\n\n\
                [bone.pelvis]\nparent = \"\"\nradius = 0.14\nhalf_height = 0.05\n\n\
                [cut.tail]\nenergy = 40\n";
    let err = parse(file).expect_err("cut at nothing");
    assert!(matches!(err, BodyError::UnknownBone { .. }), "{err:?}");
}

#[test]
fn a_line_that_is_not_a_key_is_refused_with_its_line() {
    let file = "name = \"tester\"\njust words\n";
    let err = parse(file).expect_err("syntax");
    assert_eq!(err, BodyError::Syntax { line: 2 });
}

#[test]
fn a_comment_and_a_blank_line_are_not_a_key() {
    let file = "# a comment\n\nname = \"tester\" # who\n";
    let err = parse(file).expect_err("missing gore");
    assert_eq!(
        err,
        BodyError::Missing {
            key: "gore_permille".to_string(),
        }
    );
}
