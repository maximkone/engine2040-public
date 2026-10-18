//! EV21 tests at the door: the 100-task gold pointing eval answered through
//! `scene.pick`, objects found back in the frame, ids deciding over names,
//! the frame as regions, and the refusals.

use super::*;
use crate::insight::mcp_tools::{ApiDescribeArgs, ApiDescribeTool};
use crate::scene_tools::{SceneBriefArgs, SceneBriefTool};
use engine_editor::capability::Capability;
use engine_editor::commands::{self, Batch, Document, Journal, Op};
use engine_scene::component::{AssetName, Collider, ColliderShape, Component, Mesh};
use engine_scene::transform::Transform;
use engine_spatial::Actor;

const GOLD: &str = include_str!("../../../../fixtures/pick/gold.txt");

/// What a gold task expects: an object (gold id) or a floor point in cm.
enum Want {
    Id(u32),
    Point([i32; 3]),
}

struct Gold {
    camera: ScenePickArgs,
    /// `(gold id, mesh name, position, full size)` in meters.
    actors: Vec<(u32, String, [f32; 3], [f32; 3])>,
    tasks: Vec<([u32; 2], Want)>,
}

fn gold() -> Gold {
    let mut camera = ScenePickArgs::default();
    let (mut actors, mut tasks) = (Vec::new(), Vec::new());
    for line in GOLD.lines() {
        let w: Vec<&str> = line.split_whitespace().collect();
        let f = |text: &str| text.parse::<f32>().expect("gold number");
        match w.as_slice() {
            ["camera", eye, yaw, pitch, fov, size] => {
                let e: Vec<i32> = eye.split(',').map(|v| v.parse().expect("eye")).collect();
                let (sw, sh) = size.split_once('x').expect("size");
                camera = ScenePickArgs {
                    eye_cm: [e[0], e[1], e[2]],
                    yaw_cdeg: yaw.parse().expect("yaw"),
                    pitch_cdeg: Some(pitch.parse().expect("pitch")),
                    fov_cdeg: Some(fov.parse().expect("fov")),
                    size: Some([sw.parse().expect("w"), sh.parse().expect("h")]),
                    ..Default::default()
                };
            }
            ["actor", id, name, x, y, z, sx, sy, sz] => actors.push((
                id.parse().expect("id"),
                (*name).to_owned(),
                [f(x), f(y), f(z)],
                [f(sx), f(sy), f(sz)],
            )),
            ["task", x, y, "id", id] => tasks.push((
                [x.parse().expect("x"), y.parse().expect("y")],
                Want::Id(id.parse().expect("id")),
            )),
            ["task", x, y, "pt", px, py, pz] => tasks.push((
                [x.parse().expect("x"), y.parse().expect("y")],
                Want::Point([
                    px.parse().expect("px"),
                    py.parse().expect("py"),
                    pz.parse().expect("pz"),
                ]),
            )),
            _ => {}
        }
    }
    Gold {
        camera,
        actors,
        tasks,
    }
}

/// Fixed id of gold actor `n`, so the staging order is the gold order.
fn uuid_of(n: u32) -> String {
    format!("00000000-0000-0000-0000-{n:012}")
}

/// The gold scene staged as nodes: a mesh asset named like the actor and a
/// box collider of the actor's size.
fn door(gold: &Gold) -> McpServer {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let mut ops = Vec::new();
    for (n, name, pos, size) in &gold.actors {
        let id = ObjectId::parse(&uuid_of(*n)).expect("fixed id parses");
        ops.push(Op::Insert {
            id,
            parent: ObjectId::NIL,
            local: Transform::new(*pos, [0.0, 0.0, 0.0, 1.0], [1.0; 3]),
        });
        ops.push(Op::SetComponent {
            id,
            component: Component::Mesh(Mesh {
                asset: AssetName::try_from(name.as_str()).expect("asset name"),
                cast_shadows: true,
            }),
        });
        ops.push(Op::SetComponent {
            id,
            component: Component::Collider(Collider {
                shape: ColliderShape::Box,
                half: size.map(|side| side * 0.5),
                sensor: false,
            }),
        });
    }
    let batch = Batch {
        ops,
        base_revision: doc.revision,
        idempotency_key: 1,
        provenance: None,
    };
    commands::apply(&mut doc, &mut journal, &batch).expect("gold scene stages");
    McpServer::with_document(doc, journal, 7, Capability::unrestricted(7))
}

fn ask(server: &McpServer, camera: &ScenePickArgs, question: ScenePickArgs) -> ScenePickOutput {
    let args = ScenePickArgs {
        pixel: question.pixel,
        object: question.object,
        cell: question.cell,
        ..camera.clone()
    };
    ScenePickTool::invoke(server, args).expect("pick serves")
}

fn pixel(x: u32, y: u32) -> ScenePickArgs {
    ScenePickArgs {
        pixel: Some([x, y]),
        ..Default::default()
    }
}

/// The 100 gold tasks answered through the door: every pixel resolves to its
/// object (by uuid) or its floor point, x2 equal bytes.
#[test]
fn pick_door_gold_pointing_eval_resolves_100_of_100() {
    let gold = gold();
    let server = door(&gold);
    let mut solved = 0;
    let mut objects = 0;
    let mut first_run = Vec::new();
    for (at, want) in &gold.tasks {
        let out = ask(&server, &gold.camera, pixel(at[0], at[1]));
        match want {
            Want::Id(n) => {
                let label = out.entity.clone().expect("an object is under the pixel");
                assert!(
                    label.ends_with(&format!("[{}]", uuid_of(*n))),
                    "{at:?}: {label}"
                );
                assert_eq!(out.point_cm, None);
                objects += 1;
            }
            Want::Point(point) => {
                assert_eq!(out.point_cm, Some(*point), "{at:?}");
                assert_eq!(out.entity, None);
            }
        }
        solved += 1;
        first_run.push(rmcp::serde_json::to_string(&out).expect("serializes"));
    }
    assert_eq!(solved, 100);
    assert!(objects >= 30, "{objects} object answers");
    // Bytes x2: a second walk over the same scene gives the same answers.
    for ((at, _), before) in gold.tasks.iter().zip(&first_run) {
        let again = ask(&server, &gold.camera, pixel(at[0], at[1]));
        assert_eq!(
            &rmcp::serde_json::to_string(&again).expect("serializes"),
            before
        );
    }
}

/// An object found in the frame points back at itself: its pixel picks it.
#[test]
fn pick_door_finds_an_object_and_the_pixel_picks_it_back() {
    let gold = gold();
    let server = door(&gold);
    let labels = SceneBriefTool::invoke(&server, SceneBriefArgs::default())
        .expect("brief reads")
        .objects;
    for (index, name) in [(1, "barrel"), (5, "altar")] {
        let label = &labels[index];
        assert!(label.starts_with(&format!("{name}_1 [")), "{label}");
        let found = ask(
            &server,
            &gold.camera,
            ScenePickArgs {
                object: Some(label.clone()),
                ..Default::default()
            },
        );
        assert_eq!(found.kind, "object");
        assert_eq!(found.in_frame, Some(true), "{name} is in the frame");
        let [x, y] = found.at.expect("in front of the eye");
        assert!(found.depth_cm.is_some_and(|depth| depth > 0));
        let back = ask(
            &server,
            &gold.camera,
            pixel(u32::try_from(x).expect("x"), u32::try_from(y).expect("y")),
        );
        assert_eq!(back.entity.as_ref(), Some(label), "{name} at {x},{y}");
    }
    // The pillar behind the eye of a camera turned away is not in front.
    let behind = ScenePickArgs {
        yaw_cdeg: 18_000,
        object: Some(labels[3].clone()),
        ..gold.camera.clone()
    };
    let out = ScenePickTool::invoke(&server, behind).expect("pick serves");
    assert_eq!((out.at, out.depth_cm, out.in_frame), (None, None, None));
}

/// The bracketed id decides: a label with another object's name still finds
/// the object its uuid names, and a name shared by two objects is refused
/// with both labels.
#[test]
fn pick_door_duplicate_names_resolve_by_id_only() {
    let gold = gold();
    let server = door(&gold);
    let labels = SceneBriefTool::invoke(&server, SceneBriefArgs::default())
        .expect("brief reads")
        .objects;
    let at = |object: &str| {
        ask(
            &server,
            &gold.camera,
            ScenePickArgs {
                object: Some(object.to_owned()),
                ..Default::default()
            },
        )
    };
    let second_crate = labels
        .iter()
        .find(|label| label.starts_with("crate_2 ["))
        .expect("two crates are staged");
    let uuid = second_crate
        .split_once('[')
        .expect("label")
        .1
        .trim_end_matches(']');
    let by_id = at(second_crate);
    let wrong_name = at(&format!("crate_1 [{uuid}]"));
    assert_eq!(by_id.at, wrong_name.at, "the name part never decides");
    assert_eq!(by_id.digest, wrong_name.digest);
    // A bare unique name resolves; a stem two objects share names nobody.
    assert_eq!(at("crate_2").at, by_id.at);
    let error = ScenePickTool::invoke(
        &server,
        ScenePickArgs {
            object: Some("crate".to_owned()),
            ..gold.camera.clone()
        },
    )
    .expect_err("no object is named crate");
    assert_eq!(error.code, "unknown_actor");

    // Two actors that share a display name: only the id tells them apart.
    let twin = |id: u32| Actor {
        id,
        name: "crate".to_owned(),
        x: 0.0,
        y: 0.5,
        z: 0.0,
        sx: 1.0,
        sy: 1.0,
        sz: 1.0,
        parent: None,
        physical: true,
        vx: 0.0,
        vz: 0.0,
        yaw: 0.0,
    };
    let actors = [twin(0), twin(1)];
    let ids = [
        ObjectId::parse(&uuid_of(1)).expect("id"),
        ObjectId::parse(&uuid_of(2)).expect("id"),
    ];
    let shared = object_id("crate", &actors, &ids).expect_err("two share the name");
    assert_eq!(shared.code, "ambiguous_name");
    assert!(shared.message.contains(&uuid_of(1)) && shared.message.contains(&uuid_of(2)));
    let one = object_id(&format!("crate [{}]", uuid_of(2)), &actors, &ids).expect("id decides");
    assert_eq!(one, 1);
}

/// The frame as regions: they tile it, show objects and floor, and the
/// bytes and digest repeat.
#[test]
fn pick_door_cell_manifest_tiles_the_frame_and_repeats_bytes() {
    let gold = gold();
    let server = door(&gold);
    let ask_cells = || {
        ask(
            &server,
            &gold.camera,
            ScenePickArgs {
                cell: Some(64),
                ..Default::default()
            },
        )
    };
    let first = ask_cells();
    let second = ask_cells();
    assert_eq!(
        rmcp::serde_json::to_string(&first).expect("serializes"),
        rmcp::serde_json::to_string(&second).expect("serializes")
    );
    assert_eq!(first.kind, "cell");
    let mut area = 0u64;
    for row in &first.regions {
        let w: Vec<&str> = row.split_whitespace().collect();
        let n = |at: usize| w[at].parse::<u64>().expect("region number");
        area += (n(2) - n(0)) * (n(3) - n(1));
    }
    assert_eq!(area, 1280 * 720, "no gap and no overlap");
    assert!(first.regions.iter().any(|row| row.ends_with(" -")), "floor");
    assert!(
        first.regions.iter().any(|row| row.contains(" wall_1 [")),
        "the wide wall is sampled"
    );
}

#[test]
fn pick_door_refuses_with_the_legal_range() {
    let gold = gold();
    let server = door(&gold);
    let refuse = |question: ScenePickArgs, camera: ScenePickArgs| {
        let args = ScenePickArgs {
            pixel: question.pixel,
            object: question.object,
            cell: question.cell,
            ..camera
        };
        ScenePickTool::invoke(&server, args).expect_err("refused")
    };
    let camera = gold.camera.clone();
    let small = ScenePickArgs {
        size: Some([0, 64]),
        ..camera.clone()
    };
    assert_eq!(refuse(pixel(0, 0), small).code, "bad_camera");
    let wide_fov = ScenePickArgs {
        fov_cdeg: Some(18_000),
        ..camera.clone()
    };
    let error = refuse(pixel(0, 0), wide_fov);
    assert!(error.message.contains("100..=17900"), "{}", error.message);
    let outside = refuse(pixel(1280, 0), camera.clone());
    assert_eq!(outside.code, "bad_pixel");
    let none = refuse(ScenePickArgs::default(), camera.clone());
    assert_eq!(none.code, "bad_pick");
    let two = refuse(
        ScenePickArgs {
            pixel: Some([1, 1]),
            cell: Some(64),
            ..Default::default()
        },
        camera.clone(),
    );
    assert_eq!(two.code, "bad_pick");
    let fine = refuse(
        ScenePickArgs {
            cell: Some(1),
            ..Default::default()
        },
        camera.clone(),
    );
    assert_eq!(fine.code, "bad_cell");
    assert!(fine.message.contains("at least"), "{}", fine.message);
    let ghost = refuse(
        ScenePickArgs {
            object: Some("ghost [00000000-0000-0000-0000-0000000000ff]".to_owned()),
            ..Default::default()
        },
        camera,
    );
    assert_eq!(ghost.code, "unknown_actor");
}

/// The verb is in the scene pack, so `api.describe` answers its arguments.
#[test]
fn pick_door_is_listed_and_described() {
    assert!(crate::mcp::tool_names().contains(&TOOL_SCENE_PICK));
    let server = McpServer::new(7, Capability::unrestricted(7));
    let entry = ApiDescribeTool::invoke(
        &server,
        ApiDescribeArgs {
            path: TOOL_SCENE_PICK.to_owned(),
        },
    )
    .expect("scene.pick describes");
    assert!(entry.docs.contains("Args: "), "{}", entry.docs);
    assert!(entry.docs.contains("eye_cm:"), "{}", entry.docs);
}
