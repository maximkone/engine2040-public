//! Public-surface contracts for engine-spatial: meter rounding, sight
//! occlusion and the 20 m horizon, support-surface edges, and why_hidden.
//! Every fixture is meters in, exact integer cm out.

use engine_spatial::index3::Index3;
use engine_spatial::sight::{visible, SightWorld};
use engine_spatial::why_hidden::{why_hidden, Draw, Hidden};
use engine_spatial::{enumerate_support_surfaces, to_cm, Actor, SupportKind, SupportSurface};

fn actor(id: u32, pos: [f32; 3], size: [f32; 3], physical: bool) -> Actor {
    Actor {
        id,
        name: format!("a{id}"),
        x: pos[0],
        y: pos[1],
        z: pos[2],
        sx: size[0],
        sy: size[1],
        sz: size[2],
        parent: None,
        physical,
        vx: 0.0,
        vz: 0.0,
        yaw: 0.0,
    }
}

/// Support surfaces for object 9 in an otherwise fixed scene.
fn surfaces_of(object: &Actor, rest: &[Actor]) -> Vec<SupportSurface> {
    let mut all = vec![object.clone()];
    all.extend_from_slice(rest);
    let index = Index3::build(rest);
    enumerate_support_surfaces(&all, object.id, &index)
}

#[test]
fn to_cm_rounds_half_away_from_zero() {
    assert_eq!(to_cm(0.125), 13, "+12.5 cm rounds up");
    assert_eq!(to_cm(-0.125), -13, "-12.5 cm rounds away from zero");
    assert_eq!(to_cm(0.124), 12);
    assert_eq!(to_cm(-0.124), -12);
    assert_eq!(to_cm(1.0), 100);
}

#[test]
fn nonphysical_target_is_queryable_but_never_occludes() {
    let eye = [0.0, 1.6, 0.0];
    let target = actor(1, [5.0, 1.6, 0.0], [1.0, 1.8, 1.0], false);
    let shell = actor(2, [2.5, 1.6, 0.0], [4.0, 3.2, 4.0], false);
    let world = SightWorld::build(&[target.clone(), shell.clone()]);
    assert!(world.box_of(1).is_some(), "non-physical target keeps a box");
    let see = visible(&world, eye, 0.0, 1).expect("target is indexed");
    assert!(see.los, "shells never block the ray");
    assert!(see.in_view, "target sits straight ahead");
    assert!(see.visible());

    let wall = actor(3, [2.5, 1.5, 0.0], [1.0, 3.0, 6.0], true);
    let world = SightWorld::build(&[target, shell, wall]);
    let see = visible(&world, eye, 0.0, 1).expect("target is indexed");
    assert!(!see.los, "physical wall blocks the ray");
    assert!(
        see.in_view,
        "the wall does not move the target out of the cone"
    );
    assert!(!see.visible());
}

#[test]
fn exact_twenty_meter_horizon() {
    let eye = [0.0, 1.6, 0.0];
    let edge = SightWorld::build(&[actor(1, [20.0, 1.6, 0.0], [1.0, 1.8, 1.0], true)]);
    let see = visible(&edge, eye, 0.0, 1).expect("edge target is indexed");
    assert!(see.los, "20.0 m is unoccluded");
    assert!(see.in_view, "20.0 m is inside the horizon");
    assert!(see.visible());

    let past = SightWorld::build(&[actor(2, [20.5, 1.6, 0.0], [1.0, 1.8, 1.0], true)]);
    let see = visible(&past, eye, 0.0, 2).expect("past target is indexed");
    assert!(see.los, "20.5 m stays unoccluded");
    assert!(!see.in_view, "20.5 m is past the horizon");
    assert!(!see.visible());
}

#[test]
fn support_surfaces_keep_the_120_cm_and_5_cm_edges() {
    let object = actor(9, [0.0, 0.31, 0.0], [0.4, 0.4, 0.4], true);

    // Object bottom at 11 cm; a 5 cm top leaves a 6 cm gap, past the rest edge.
    let five = actor(8, [0.0, 0.0, 0.0], [1.0, 0.10, 1.0], true);
    let under = surfaces_of(&object, std::slice::from_ref(&five));
    let under = under
        .iter()
        .find(|s| s.supporter == Some(8))
        .expect("5 cm top");
    assert_eq!(under.top_cm, 5);
    assert_eq!(under.gap_cm, 6);
    assert!(!under.rests, "6 cm gap is past the 5 cm rest edge");

    // A 6 cm top leaves exactly 5 cm: the last resting contact.
    let six = actor(8, [0.0, 0.0, 0.0], [1.0, 0.12, 1.0], true);
    let under = surfaces_of(&object, std::slice::from_ref(&six));
    let under = under
        .iter()
        .find(|s| s.supporter == Some(8))
        .expect("6 cm top");
    assert_eq!(under.top_cm, 6);
    assert_eq!(under.gap_cm, 5);
    assert!(under.rests, "5 cm gap is still resting contact");

    // 120 cm on every side is a tread; one cm more is a shelf top.
    let tread_prop = actor(9, [0.0, 1.40, 0.0], [0.4, 0.4, 0.4], true);
    let tread = actor(8, [0.0, 0.6, 0.0], [1.2, 1.2, 1.2], true);
    let top = surfaces_of(&tread_prop, std::slice::from_ref(&tread));
    let top = top
        .iter()
        .find(|s| s.supporter == Some(8))
        .expect("tread top");
    assert_eq!(top.kind, SupportKind::Tread);

    let shelf_prop = actor(9, [0.0, 1.50, 0.0], [0.4, 0.4, 0.4], true);
    let shelf = actor(8, [0.0, 0.65, 0.0], [1.3, 1.3, 1.3], true);
    let top = surfaces_of(&shelf_prop, std::slice::from_ref(&shelf));
    let top = top
        .iter()
        .find(|s| s.supporter == Some(8))
        .expect("shelf top");
    assert_eq!(top.kind, SupportKind::ShelfTop);
}

#[test]
fn why_hidden_reports_range_and_behind_eye_before_occlusion() {
    let eye = [0.0, 1.0, 0.0];

    // 30 m ahead with a wall at 3 m: range fails before occlusion is asked.
    let far = actor(1, [30.0, 1.0, 0.0], [1.0, 2.0, 1.0], true);
    let far_wall = actor(2, [3.0, 1.0, 0.0], [0.5, 2.0, 4.0], true);
    let why = why_hidden(&[far, far_wall], Draw::default(), 1, eye, 0.0).expect("finite eye");
    assert_eq!(why.verdict, Hidden::TooFar);
    assert_eq!(why.evidence, 3000, "distance to the target in cm");
    assert!(!matches!(why.verdict, Hidden::OccludedBy(_)));

    // 30 m behind the eye with a wall at -3 m: the eye test comes first.
    let behind = actor(3, [-30.0, 1.0, 0.0], [1.0, 2.0, 1.0], true);
    let behind_wall = actor(4, [-3.0, 1.0, 0.0], [0.5, 2.0, 4.0], true);
    let why = why_hidden(&[behind, behind_wall], Draw::default(), 3, eye, 0.0).expect("finite eye");
    assert_eq!(why.verdict, Hidden::BehindEye);
    assert_eq!(why.evidence, -3000, "cm along the facing, behind the eye");
    assert!(!matches!(why.verdict, Hidden::OccludedBy(_)));
}
