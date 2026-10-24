//! Tests of the scene report on synthetic scenes: a camera, a pillar, a wall, two
//! fighters and spears, each case changing one thing so a regression in the
//! maths turns exactly its own test red.

use engine_render::present::sun_shadow_view_proj;
use engine_render::v1::Camera;

use super::geom::{unit, P3};
use super::{
    build, measure, FighterFacts, HeldPart, Joint, Kind, Laws, Light, Pixels, Report, Sample,
    SceneInput, ShadowMap, Shape, SpearFacts, SpearState, Thing,
};

/// The arena day sun, toward the sun.
fn sun() -> P3 {
    unit([-0.62, 0.55, 0.45]).expect("sun has a direction")
}

/// Eight triangles-pairs of a box as a mesh.
fn box_mesh(lo: P3, hi: P3) -> Shape {
    let positions: Vec<P3> = super::geom::corners(lo, hi).to_vec();
    let quads: [[u32; 4]; 6] = [
        [0, 1, 3, 2],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 3, 7, 6],
        [0, 2, 6, 4],
        [1, 3, 7, 5],
    ];
    let indices = quads
        .iter()
        .flat_map(|q| [q[0], q[1], q[2], q[0], q[2], q[3]])
        .collect();
    Shape::Mesh { positions, indices }
}

/// Corner and center samples of a box; with normals when `facing` asks.
fn samples(lo: P3, hi: P3, facing: bool) -> Vec<Sample> {
    let mid = [
        (lo[0] + hi[0]) * 0.5,
        (lo[1] + hi[1]) * 0.5,
        (lo[2] + hi[2]) * 0.5,
    ];
    let mut out: Vec<Sample> = super::geom::corners(lo, hi)
        .iter()
        .map(|c| Sample {
            at: *c,
            normal: facing.then(|| unit(super::geom::sub(*c, mid))).flatten(),
        })
        .collect();
    out.push(Sample {
        at: mid,
        normal: None,
    });
    out
}

fn joint(name: &str, at: P3, parent: Option<usize>) -> Joint {
    Joint {
        name: name.to_owned(),
        at,
        parent,
    }
}

/// A box-bodied fighter standing at `x`, `z` with `lift` meters under its feet.
fn fighter(slot: u8, x: f32, z: f32, lift: f32) -> Thing {
    let lo = [x - 0.3, lift, z - 0.2];
    let hi = [x + 0.3, lift + 1.8, z + 0.2];
    Thing {
        id: format!("fighter_{slot}"),
        kind: Kind::Fighter,
        label: format!("slot {slot}"),
        bounds: Some((lo, hi)),
        shape: box_mesh(lo, hi),
        drawn: true,
        samples: samples(lo, hi, true),
        joints: vec![
            joint("pelvis", [x, lift + 0.9, z], None),
            joint("torso", [x, lift + 1.3, z], Some(0)),
            joint("head", [x, lift + 1.7, z], Some(1)),
            joint("palm_R", [x + 0.4, lift + 1.1, z + 0.2], Some(1)),
            joint("palm_L", [x - 0.4, lift + 1.1, z + 0.2], Some(1)),
            joint("foot_L", [x - 0.15, lift + 0.08, z], Some(0)),
            joint("foot_R", [x + 0.15, lift + 0.09, z], Some(0)),
        ],
        fighter: Some(FighterFacts {
            slot,
            act: "idle".to_owned(),
            clip: 3,
            tinted: false,
            ragdolled: false,
            hp: Some(1000),
            root: [x, lift, z],
            yaw: 0.0,
            armed: Some("gladius".to_owned()),
            foot_slide_m: [Some(0.003), None],
            error_parts: 0,
            nonfinite: 0,
            held: vec![HeldPart {
                name: "gladius_blade".to_owned(),
                hand: 'r',
                at: [x + 0.4, lift + 1.1, z + 0.2],
                bounds: (
                    [x + 0.35, lift + 1.0, z + 0.2],
                    [x + 0.45, lift + 1.2, z + 0.9],
                ),
                len_m: 0.7,
            }],
            held_known: true,
        }),
        spear: None,
    }
}

/// A solid box prop.
fn prop(index: usize, kind: &str, lo: P3, hi: P3) -> Thing {
    Thing {
        id: format!("prop_{index}_{kind}"),
        kind: Kind::Prop,
        label: kind.to_owned(),
        bounds: Some((lo, hi)),
        shape: Shape::Box,
        drawn: true,
        samples: samples(lo, hi, false),
        joints: Vec::new(),
        fighter: None,
        spear: None,
    }
}

/// A spear with its tip at `tip` pointing along +x.
fn spear(slot: u8, state: SpearState, tip: P3, stuck_in: Option<u8>) -> Thing {
    let dir = [1.0, 0.0, 0.0];
    let tail = [tip[0] - 1.9, tip[1], tip[2]];
    let lo = [tail[0], tip[1] - 0.02, tip[2] - 0.02];
    let hi = [tip[0], tip[1] + 0.02, tip[2] + 0.02];
    Thing {
        id: format!("spear_{slot}"),
        kind: Kind::Spear,
        label: format!("spear of slot {slot}"),
        bounds: Some((lo, hi)),
        shape: Shape::Rod {
            a: tail,
            b: tip,
            radius: 0.03,
        },
        drawn: true,
        samples: vec![
            Sample {
                at: tip,
                normal: None,
            },
            Sample {
                at: tail,
                normal: None,
            },
        ],
        joints: Vec::new(),
        fighter: None,
        spear: Some(SpearFacts {
            owner: slot,
            state,
            tip,
            dir,
            len_m: 1.9,
            held_by: None,
            stuck_in,
        }),
    }
}

fn scene(things: Vec<Thing>) -> SceneInput {
    let mut camera = Camera::new(
        [0.0, 1.7, 9.0],
        [0.0, 1.0, 0.0],
        [0.0, 1.0, 0.0],
        50.0f32.to_radians(),
    );
    camera.aspect = 16.0 / 9.0;
    camera.far = 200.0;
    SceneInput {
        source: "headless",
        tick: 120,
        seed: 2040,
        viewport: (1280, 720),
        camera,
        shot: "action".to_owned(),
        mode: "director".to_owned(),
        light: Light {
            sun_dir: sun(),
            color: [1.0, 0.92, 0.8],
            intensity: 3.3,
            sky_sun_dir: None,
        },
        shadow: ShadowMap {
            on: true,
            edge_px: 2048,
            half_extent_m: 15.0,
            texel_m: 30.0 / 2048.0,
            view_proj: sun_shadow_view_proj(sun()),
        },
        contact: 1.0,
        passes: vec![("sun_shadow".to_owned(), true), ("fog".to_owned(), true)],
        things,
        laws: Laws {
            line: "arena view: frames=120 bodies=240".to_owned(),
            verdict: "ok".to_owned(),
            warnings: Vec::new(),
        },
        pixels: None,
        renderer_live: true,
    }
}

fn index(input: &SceneInput, id: &str) -> usize {
    input
        .things
        .iter()
        .position(|t| t.id == id)
        .unwrap_or_else(|| panic!("no thing {id}"))
}

/// A JSON line is one line, balanced outside strings, with no NaN or inf.
fn well_formed(json: &str) {
    assert!(!json.contains('\n'), "one line");
    assert!(
        !json.contains("NaN") && !json.contains("inf"),
        "finite numbers only"
    );
    let (mut depth, mut in_str, mut esc) = (0i32, false, false);
    for c in json.chars() {
        if in_str {
            match (esc, c) {
                (true, _) => esc = false,
                (false, '\\') => esc = true,
                (false, '"') => in_str = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            _ => {}
        }
        assert!(depth >= 0, "closed before opened");
    }
    assert!(!in_str && depth == 0, "balanced");
}

/// The standard scene: two fighters, a pillar in front of the first, a wall
/// west of the first.
fn standard() -> SceneInput {
    scene(vec![
        fighter(0, 0.0, 0.0, 0.0),
        fighter(1, 4.0, 0.0, 0.0),
        prop(2, "pillar", [-0.6, 0.0, 3.0], [0.6, 3.0, 4.2]),
        prop(3, "wall", [-2.5, 0.0, -3.0], [-1.5, 6.0, 3.0]),
    ])
}

#[test]
fn the_report_names_every_thing_with_a_finite_rect_and_repeats_its_bytes() {
    let input = standard();
    let a = build(&input);
    let b = build(&input);
    assert_eq!(a, b, "the same scene reports the same bytes");
    well_formed(&a.json);
    for thing in &input.things {
        assert!(
            a.json.contains(&format!("\"id\":\"{}\"", thing.id)),
            "{}",
            thing.id
        );
        assert!(a.text.contains(&thing.id), "text names {}", thing.id);
    }
    assert!(a
        .json
        .starts_with("{\"schema\":1,\"source\":\"headless\",\"tick\":120"));
    assert!(a.json.contains("\"text\":["), "the text rides the json");
    let row = measure(&input, index(&input, "fighter_1"));
    let rect = row.screen.expect("fighter_1 is in the picture");
    assert!(
        rect.iter().all(|v| v.is_finite() && v.abs() <= 1.0),
        "{rect:?}"
    );
    assert!(row.cover > 0.0 && row.cover < 0.5, "{}", row.cover);
}

#[test]
fn a_pillar_between_the_camera_and_a_fighter_hides_it_and_is_named() {
    let input = standard();
    let behind = measure(&input, index(&input, "fighter_0"));
    let pillar = index(&input, "prop_2_pillar");
    assert!(
        behind.vis.occluded > 0.8,
        "fighter_0 is behind the pillar: {:?}",
        behind.vis
    );
    assert_eq!(behind.vis.by.first().map(|b| b.0), Some(pillar));
    let clear = measure(&input, index(&input, "fighter_1"));
    assert!(
        clear.vis.visible > 0.99 && clear.vis.by.is_empty(),
        "{:?}",
        clear.vis
    );
    // Move the pillar away: the same fighter is seen again.
    let mut moved = standard();
    let at = index(&moved, "prop_2_pillar");
    moved.things[at] = prop(2, "pillar", [20.0, 0.0, 3.0], [21.2, 3.0, 4.2]);
    let seen = measure(&moved, index(&moved, "fighter_0"));
    assert!(seen.vis.occluded < 0.01, "{:?}", seen.vis);
    let report = build(&input);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.contains("fighter_0") && f.contains("hidden behind prop_2_pillar")),
        "{:?}",
        report.findings
    );
}

#[test]
fn a_fighter_outside_the_frame_is_named_and_one_cut_by_the_edge_is_too() {
    let mut input = standard();
    input.things.push(fighter(2, 60.0, 0.0, 0.0));
    let report = build(&input);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f == "fighter_2 is outside the picture"),
        "{:?}",
        report.findings
    );
    let mut edge = standard();
    edge.things.push(fighter(2, 7.5, 0.0, 0.0));
    let row = measure(&edge, index(&edge, "fighter_2"));
    assert!(
        row.vis.offscreen > 0.1 && row.vis.visible > 0.0,
        "{:?}",
        row.vis
    );
    assert!(build(&edge)
        .findings
        .iter()
        .any(|f| f.contains("fighter_2 is cut by the frame edge")));
}

#[test]
fn the_sun_is_blocked_by_the_wall_and_free_without_it() {
    let input = standard();
    let shaded = measure(&input, index(&input, "fighter_0"));
    let wall = index(&input, "prop_3_wall");
    assert!(
        shaded.sun.blocked > 0.4,
        "the wall shades fighter_0: {:?}",
        shaded.sun
    );
    assert_eq!(shaded.sun.by.first().map(|b| b.0), Some(wall));
    let open = measure(&input, index(&input, "fighter_1"));
    assert!(
        open.sun.lit > 0.4 && open.sun.blocked < 0.01,
        "{:?}",
        open.sun
    );
    let mut clear = standard();
    clear.things.remove(index(&clear, "prop_3_wall"));
    let free = measure(&clear, index(&clear, "fighter_0"));
    assert!(
        free.sun.blocked < 0.01 && free.sun.lit > 0.4,
        "{:?}",
        free.sun
    );
    // A sun below the horizon lights nothing.
    let mut night = standard();
    night.light.sun_dir = unit([0.3, -0.5, 0.2]).expect("direction");
    let dark = measure(&night, index(&night, "fighter_1"));
    assert_eq!((dark.sun.lit, dark.sun.blocked), (0.0, 0.0));
    assert_eq!(dark.sun.casts, "off");
}

#[test]
fn the_sun_map_holds_the_pit_and_loses_what_stands_outside_it_and_goes_off_with_the_switch() {
    let mut input = standard();
    input.things.push(fighter(2, 40.0, 0.0, 0.0));
    let inside = measure(&input, index(&input, "fighter_1"));
    assert_eq!(inside.sun.casts, "yes");
    let far = measure(&input, index(&input, "fighter_2"));
    assert_eq!(far.sun.casts, "outside");
    assert!(build(&input)
        .findings
        .iter()
        .any(|f| f.contains("fighter_2 is outside the sun map")));
    let foot = inside.sun.footprint.expect("a shadow on the sand");
    assert!(foot.len_m > 1.0 && foot.xz[0] < foot.xz[2], "{foot:?}");
    input.shadow.on = false;
    let off = measure(&input, index(&input, "fighter_1"));
    assert_eq!(off.sun.casts, "off");
    assert!(build(&input)
        .findings
        .iter()
        .any(|f| f.starts_with("the sun shadow map is off")));
}

#[test]
fn floor_contact_names_the_support_and_flags_a_sunk_or_floating_body() {
    let mut input = standard();
    let ground = measure(&input, index(&input, "fighter_1"))
        .floor
        .expect("floor row");
    assert!(
        ground.grounded && ground.standing_on.is_none(),
        "{ground:?}"
    );
    assert!((ground.feet_cm[0].expect("left ankle") - 8.0).abs() < 0.1);
    input.things.push(fighter(2, -6.0, 6.0, 0.18));
    input.things.push(fighter(3, -8.0, 6.0, -0.06));
    let report = build(&input);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.starts_with("fighter_2 idles 18.0 cm above its floor")),
        "{:?}",
        report.findings
    );
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.starts_with("fighter_3 is sunk 6.0 cm into its floor")),
        "{:?}",
        report.findings
    );
    // A fighter on a platform stands on it, not on the ground.
    let mut stage = standard();
    stage
        .things
        .push(prop(4, "platform", [5.0, 0.0, -2.0], [8.0, 0.6, 2.0]));
    stage.things.push(fighter(2, 6.5, 0.0, 0.6));
    let on = measure(&stage, index(&stage, "fighter_2"))
        .floor
        .expect("floor row");
    assert_eq!(on.standing_on, Some(index(&stage, "prop_4_platform")));
    assert!(on.grounded, "{on:?}");
}

#[test]
fn touching_is_a_gap_and_a_touch_list_and_far_things_do_not_touch() {
    let mut input = standard();
    input.things.push(fighter(2, 4.64, 0.0, 0.0));
    let near = measure(&input, index(&input, "fighter_2"));
    let other = index(&input, "fighter_1");
    let hit = near
        .touch
        .iter()
        .find(|t| t.other == other)
        .expect("fighter_1 touches");
    assert!(hit.gap_cm <= 4.0, "{hit:?}");
    let apart = measure(&input, index(&input, "fighter_0"));
    assert!(
        apart.touch.iter().all(|t| t.other != other),
        "fighter_0 is 4 m away"
    );
    let mut inside = standard();
    inside.things.push(fighter(2, 4.0, 0.0, 0.0));
    let deep = measure(&inside, index(&inside, "fighter_2"));
    assert!(deep
        .touch
        .iter()
        .any(|t| t.other == index(&inside, "fighter_1")));
}

#[test]
fn the_hand_that_holds_a_part_is_named_and_an_armed_empty_hand_is_a_finding() {
    let input = standard();
    let row = measure(&input, index(&input, "fighter_1"));
    let right = row
        .hands
        .iter()
        .find(|h| h.hand == 'r')
        .expect("right hand");
    assert_eq!(right.holds, vec![("gladius_blade".to_owned(), "skinned")]);
    assert!(right.at.is_some());
    let left = row.hands.iter().find(|h| h.hand == 'l').expect("left hand");
    assert!(left.holds.is_empty());
    let json = build(&input).json;
    assert!(
        json.contains("\"held_parts\":[{\"name\":\"gladius_blade\",\"hand\":\"r\""),
        "the held weapon is measured like a thing"
    );
    assert!(json.contains("\"screen\":{\"ndc\":["), "{json}");
    assert!(build(&input)
        .findings
        .iter()
        .all(|f| !f.contains("no hand holds")));
    let mut empty = standard();
    let at = index(&empty, "fighter_1");
    if let Some(f) = empty.things[at].fighter.as_mut() {
        f.held.clear();
    }
    assert!(
        build(&empty)
            .findings
            .iter()
            .any(|f| f == "fighter_1 is armed with gladius but no hand holds anything"),
        "an armed fighter with an empty hand"
    );
    // The same fighter with unknown skin data says nothing.
    if let Some(f) = empty.things[at].fighter.as_mut() {
        f.held_known = false;
    }
    assert!(build(&empty)
        .findings
        .iter()
        .all(|f| !f.contains("no hand holds")));
}

#[test]
fn a_spear_in_the_palm_is_held_and_a_spear_stuck_in_a_body_names_its_bone() {
    let mut input = standard();
    // Slot 1's right palm is at (4.4, 1.1, 0.2); a spear whose tail is there.
    let mut held = spear(1, SpearState::Held, [6.3, 1.1, 0.2], None);
    if let Some(s) = held.spear.as_mut() {
        s.held_by = Some(1);
    }
    input.things.push(held);
    let hands = measure(&input, index(&input, "fighter_1")).hands;
    let right = hands.iter().find(|h| h.hand == 'r').expect("right hand");
    assert!(
        right
            .holds
            .iter()
            .any(|(item, how)| item == "spear_1" && *how == "held"),
        "{right:?}"
    );

    let mut stuck = standard();
    stuck
        .things
        .push(spear(0, SpearState::Stuck, [4.0, 1.25, 0.0], Some(1)));
    let row = measure(&stuck, index(&stuck, "spear_0"))
        .stuck
        .expect("stuck row");
    assert_eq!(row.bone.as_deref(), Some("torso"));
    assert!(row.dist_cm < 10.0 && row.tip_inside, "{row:?}");
    assert!(
        build(&stuck)
            .findings
            .iter()
            .all(|f| !f.contains("spear_0 is stuck in")),
        "a good stick has no stuck finding"
    );

    let mut wrong = standard();
    wrong
        .things
        .push(spear(0, SpearState::Stuck, [4.0, 1.0, 1.5], Some(1)));
    let findings = build(&wrong).findings;
    assert!(
        findings
            .iter()
            .any(|f| f.contains("spear_0 is stuck in fighter_1 but its tip is")),
        "{findings:?}"
    );
    let mut world = standard();
    world
        .things
        .push(spear(0, SpearState::Stuck, [-1.4, 2.0, 0.0], None));
    let in_wall = measure(&world, index(&world, "spear_0"))
        .stuck
        .expect("world row");
    assert_eq!(in_wall.near, Some(index(&world, "prop_3_wall")));
    assert!(in_wall.dist_cm < 15.0, "{in_wall:?}");
}

#[test]
fn a_spear_rod_hides_what_it_crosses_and_the_id_buffer_catches_a_thing_drawn_nowhere() {
    let mut input = standard();
    input
        .things
        .push(spear(1, SpearState::Flying, [3.0, 1.0, 6.0], None));
    let rod = index(&input, "spear_1");
    // A rod across the sight line of fighter_1's chest hides some of it.
    let mut cross = standard();
    cross.things.push(Thing {
        shape: Shape::Rod {
            a: [1.5, 0.5, 4.0],
            b: [3.0, 2.5, 4.0],
            radius: 0.4,
        },
        ..spear(1, SpearState::Flying, [5.0, 2.5, 4.0], None)
    });
    let seen = measure(&cross, index(&cross, "fighter_1"));
    assert!(
        seen.vis.occluded > 0.0 && seen.vis.by[0].0 == index(&cross, "spear_1"),
        "{:?}",
        seen.vis
    );
    let _ = rod;
    // The id buffer: fighter_1 projects a share of the picture but owns 0 pixels.
    let mut px = standard();
    px.pixels = Some(Pixels {
        total: 921_600,
        clear: 900_000,
        by_id: std::iter::once(("fighter_0".to_owned(), 5000_u64)).collect(),
    });
    let report = build(&px);
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.starts_with("fighter_1 covers") && f.contains("holds 0 of its pixels")),
        "{:?}",
        report.findings
    );
    assert!(report.json.contains("\"px\":5000"));
}

#[test]
fn laws_and_the_camera_ride_the_report_and_a_failing_verdict_is_a_finding() {
    let mut input = standard();
    input.laws.verdict = "FAIL(view_eye_in_body)".to_owned();
    input.laws.warnings = vec!["arena WARN: view_eye_in_body value=3 floor=0".to_owned()];
    let report = build(&input);
    assert!(report
        .findings
        .iter()
        .any(|f| f == "view laws FAIL(view_eye_in_body)"));
    assert!(report
        .json
        .contains("\"verdict\":\"FAIL(view_eye_in_body)\""));
    assert!(report.json.contains("\"heading_deg\":"));
    assert!(report.text.contains("shot action (director)"));
    assert!(report.text.contains("arena WARN: view_eye_in_body"));
    // The camera inside a body is named by the report itself.
    let mut inside = standard();
    (
        inside.camera.eye.x,
        inside.camera.eye.y,
        inside.camera.eye.z,
    ) = (0.0, 1.0, 0.0);
    (
        inside.camera.target.x,
        inside.camera.target.y,
        inside.camera.target.z,
    ) = (0.0, 1.0, -5.0);
    assert!(build(&inside)
        .findings
        .iter()
        .any(|f| f == "the camera is inside fighter_0"));
}

#[test]
fn names_with_quotes_are_escaped_and_the_text_form_has_one_line_per_fact() {
    let mut input = standard();
    input.things[0].label = "slot \"0\"\\north".to_owned();
    let Report { json, text, .. } = build(&input);
    well_formed(&json);
    assert!(json.contains("slot \\\"0\\\"\\\\north"));
    for needle in [
        "scene tick=120 seed=2040",
        "camera eye",
        "sun elevation",
        "passes: sun_shadow=on fog=on",
        "screen:",
        "seen:",
        "sun:",
        "floor:",
        "hand r:",
        "laws:",
    ] {
        assert!(text.contains(needle), "missing `{needle}` in:\n{text}");
    }
}
