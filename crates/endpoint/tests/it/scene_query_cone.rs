#![cfg(feature = "mcp")]

//! scene.query cone vectors: the door answers exact ids over a staged
//! five-entity scene, left and right partition the half-plane, a turned
//! observer drops the target it now faces away from, repeats are identical,
//! and a bad range refuses with the retry call. Lib-level vectors pin the
//! integer bearings, the filter, the typed refusals, the eye observer, the
//! sight cross-check, and the advertised tool-list headroom.

use engine_editor::capability::Capability;
use engine_editor::commands::{self, Batch, Document, Journal, Op};
use engine_endpoint::mcp::{
    McpServer, TOOL_LIST_BUDGET_CHARS, TOOL_LIST_MIN_HEADROOM_CHARS,
};
use engine_endpoint::scene_query::{
    cone_rows, QueryArgs, SceneQueryArgs, SceneQueryOutput, SceneQueryTool, Side,
};
use engine_endpoint::scene_tools::{SceneBriefArgs, SceneBriefTool};
use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;
use engine_spatial::sight::{visible, SightWorld};
use engine_spatial::Actor;
use rmcp::handler::server::router::tool::SyncTool;

const IDENTITY: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
/// Yaw pi about Y: the quaternion that faces the observer toward -X.
const FACING_BACK: [f32; 4] = [0.0, 1.0, 0.0, 0.0];

/// Staged ids, in the order the door labels them (observer first).
const IDS: [&str; 5] = [
    "11111111-1111-1111-1111-111111111111",
    "22222222-2222-2222-2222-222222222222",
    "33333333-3333-3333-3333-333333333333",
    "44444444-4444-4444-4444-444444444444",
    "55555555-5555-5555-5555-555555555555",
];
/// Positions in metres: observer at the origin; ahead 304 cm at 9 degrees,
/// left and right 223 cm at 63 degrees, behind 206 cm at 166 degrees.
const SPOTS: [[f32; 3]; 5] = [
    [0.0, 0.5, 0.0],
    [3.0, 0.5, 0.5],
    [1.0, 0.5, -2.0],
    [1.0, 0.5, 2.0],
    [-2.0, 0.5, 0.5],
];

/// Stage the five entities; `observer_rotation` turns the observer.
fn staged(observer_rotation: [f32; 4]) -> McpServer {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    for (index, (text, spot)) in IDS.iter().zip(SPOTS).enumerate() {
        let rotation = if index == 0 {
            observer_rotation
        } else {
            IDENTITY
        };
        let batch = Batch {
            ops: vec![Op::Insert {
                id: ObjectId::parse(text).expect("fixed id parses"),
                parent: ObjectId::NIL,
                local: Transform::new(spot, rotation, [1.0, 1.0, 1.0]),
            }],
            base_revision: doc.revision,
            idempotency_key: 11 + u64::try_from(index).expect("five ids"),
            provenance: None,
        };
        commands::apply(&mut doc, &mut journal, &batch).expect("seed applies");
    }
    McpServer::with_document(doc, journal, 7, Capability::unrestricted(7))
}

/// The door label of staged entity `index` (`prop_NNNN [uuid]`).
fn label_of(server: &McpServer, index: usize) -> String {
    SceneBriefTool::invoke(server, SceneBriefArgs { budget_bytes: None })
        .expect("brief reads")
        .objects
        .into_iter()
        .find(|label| label.contains(IDS[index]))
        .expect("label names the staged id")
}

fn cone_args(anchor: &str, half: i32) -> SceneQueryArgs {
    SceneQueryArgs {
        kind: "cone".to_owned(),
        anchor: Some(anchor.to_owned()),
        radius_cm: Some(500),
        half_angle_deg: Some(half),
        ..Default::default()
    }
}

fn objects(out: &SceneQueryOutput) -> Vec<String> {
    out.hits.iter().map(|hit| hit.object.clone()).collect()
}

fn sided(out: &SceneQueryOutput, side: &str) -> Vec<String> {
    let prefix = format!("cone: {side} ");
    out.hits
        .iter()
        .filter(|hit| hit.reason.starts_with(&prefix))
        .map(|hit| hit.object.clone())
        .collect()
}

#[test]
fn door_cone_ahead_returns_the_exact_staged_id() {
    let server = staged(IDENTITY);
    let observer = label_of(&server, 0);
    let out = SceneQueryTool::invoke(&server, cone_args(&observer, 45)).expect("cone serves");
    assert_eq!(out.kind, "cone");
    assert_eq!(objects(&out), vec![label_of(&server, 1)]);
    assert!(out.hits[0].reason.starts_with("cone: ahead 9 deg 304 cm"));
}

#[test]
fn door_cone_left_and_right_partition_the_half_plane() {
    let server = staged(IDENTITY);
    let observer = label_of(&server, 0);
    let out = SceneQueryTool::invoke(&server, cone_args(&observer, 90)).expect("half-plane serves");
    assert_eq!(sided(&out, "ahead"), vec![label_of(&server, 1)]);
    assert_eq!(sided(&out, "left"), vec![label_of(&server, 2)]);
    assert_eq!(sided(&out, "right"), vec![label_of(&server, 3)]);
    assert_eq!(out.hits.len(), 3, "the behind target stays out of a half-plane");
}

#[test]
fn door_cone_turned_observer_drops_the_target_it_faces_away_from() {
    let server = staged(FACING_BACK);
    let observer = label_of(&server, 0);
    let out = SceneQueryTool::invoke(&server, cone_args(&observer, 45)).expect("cone serves");
    assert_eq!(
        objects(&out),
        vec![label_of(&server, 4)],
        "only the target at -X reads ahead"
    );
}

#[test]
fn door_cone_repeats_are_identical() {
    let server = staged(IDENTITY);
    let observer = label_of(&server, 0);
    let first = SceneQueryTool::invoke(&server, cone_args(&observer, 180)).expect("first serves");
    let again = SceneQueryTool::invoke(&server, cone_args(&observer, 180)).expect("again serves");
    assert_eq!(first.digest, again.digest, "digest is stable");
    assert_eq!(first, again, "bytes and rows are identical");
    assert_eq!(first.hits.len(), 4, "the full circle holds all four targets");
}

#[test]
fn door_cone_negative_range_refuses_with_the_retry_call() {
    let server = staged(IDENTITY);
    let mut args = cone_args(&label_of(&server, 0), 45);
    args.radius_cm = Some(-5);
    let err = SceneQueryTool::invoke(&server, args).expect_err("negative range refuses");
    assert_eq!(err.code, "bad_radius");
    assert!(
        err.message.contains("retry scene.query"),
        "recovery line: {}",
        err.message
    );
}

/// Actor `id` at `(x, z)` metres, on the floor plane at a fixed height.
fn at(id: u32, x: f32, z: f32) -> Actor {
    Actor {
        id,
        name: format!("ring_{id}"),
        x,
        y: 0.4,
        z,
        sx: 0.8,
        sy: 0.8,
        sz: 0.8,
        parent: None,
        physical: true,
        vx: 0.0,
        vz: 0.0,
        yaw: 0.0,
    }
}

#[test]
fn cone_bearings_are_exact_integer_degrees() {
    // Observer 0 at the origin facing +X; each target sits on a known offset.
    let mut actors = vec![at(0, 0.0, 0.0)];
    let offsets: [(u32, f32, f32); 8] = [
        (1, 1.0, 0.0),
        (2, 1.0, 1.0),
        (3, 0.0, 1.0),
        (4, -1.0, 0.0),
        (5, -1.0, 1.0),
        (6, 1.0, 2.0),
        (7, 2.0, 1.0),
        (8, 0.0, -1.0),
    ];
    for (id, x, z) in offsets {
        actors.push(at(id, x, z));
    }
    actors.push(at(9, 0.0, 0.0));
    let rows = cone_rows(&actors, &QueryArgs::cone_from(0, 180, 500)).expect("ring answers");
    let got: Vec<(u32, Side, u32, i64)> = rows
        .iter()
        .map(|r| (r.id, r.side, r.bearing_deg, r.distance_cm))
        .collect();
    assert_eq!(
        got,
        vec![
            (1, Side::Ahead, 0, 100),
            (2, Side::Ahead, 45, 141),
            (3, Side::Right, 90, 100),
            (4, Side::Behind, 180, 100),
            (5, Side::Behind, 135, 141),
            (6, Side::Right, 63, 223),
            (7, Side::Ahead, 26, 223),
            (8, Side::Left, 90, 100),
            (9, Side::Ahead, 0, 0),
        ]
    );
}

#[test]
fn cone_filter_keeps_matching_names_only() {
    let actors = vec![at(0, 0.0, 0.0), at(1, 1.0, 0.0), at(2, 2.0, 0.0)];
    let mut args = QueryArgs::cone_from(0, 45, 500);
    args.filter = Some("ring_2".to_owned());
    let rows = cone_rows(&actors, &args).expect("filtered cone answers");
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![2]);
}

#[test]
fn cone_refusals_use_typed_codes_and_name_the_retry_call() {
    let actors = vec![at(0, 0.0, 0.0), at(1, 1.0, 0.0)];
    let mut both = QueryArgs::cone_from(0, 45, 500);
    both.eye_m = Some([0.0, 0.4, 0.0]);
    let mut neither = QueryArgs::cone_from(0, 45, 500);
    neither.anchor = None;
    let mut no_angle = QueryArgs::cone_from(0, 45, 500);
    no_angle.half_angle_deg = None;
    let cases = [
        (both, "bad_point"),
        (neither, "bad_point"),
        (QueryArgs::cone_from(99, 45, 500), "unknown_actor"),
        (QueryArgs::cone_from(0, 45, 0), "bad_radius"),
        (QueryArgs::cone_from(0, 45, -5), "bad_radius"),
        (QueryArgs::cone_from(0, 0, 500), "bad_limit"),
        (QueryArgs::cone_from(0, 181, 500), "bad_limit"),
        (no_angle, "bad_limit"),
    ];
    for (args, code) in cases {
        let err = cone_rows(&actors, &args).expect_err("refuses");
        assert_eq!(err.code, code, "{err}");
        assert!(err.hint.contains("retry scene.query"), "{}", err.hint);
    }
}

#[test]
fn cone_eye_observer_faces_yaw_and_reads_the_coincident_actor() {
    let actors = vec![at(0, 0.0, 0.0), at(1, -1.0, 0.0)];
    let args = QueryArgs::cone_at([0.0, 0.4, 0.0], std::f32::consts::PI, 45, 500);
    let rows = cone_rows(&actors, &args).expect("eye cone answers");
    let got: Vec<(u32, Side, u32, i64)> = rows
        .iter()
        .map(|r| (r.id, r.side, r.bearing_deg, r.distance_cm))
        .collect();
    assert_eq!(
        got,
        vec![(0, Side::Ahead, 0, 0), (1, Side::Ahead, 0, 100)],
        "facing -X puts actor 1 dead ahead; the coincident actor reads 0 cm"
    );
}

#[test]
fn cone_45_agrees_with_the_sight_cone_in_view() {
    let actors = vec![
        at(0, 0.0, 0.0),
        at(1, 3.0, 0.5),
        at(2, 1.0, -2.0),
        at(3, 1.0, 2.0),
        at(4, -2.0, 0.5),
        at(5, 5.0, 0.0),
        at(6, 2.0, 2.0),
    ];
    let world = SightWorld::build(&actors);
    let rows = cone_rows(&actors, &QueryArgs::cone_from(0, 45, 2000)).expect("cone answers");
    for actor in actors.iter().filter(|a| a.id != 0) {
        let seen = visible(&world, [0.0, 0.4, 0.0], 0.0, actor.id)
            .expect("sight answers")
            .in_view;
        assert_eq!(
            rows.iter().any(|r| r.id == actor.id),
            seen,
            "target {} cone vs sight cone",
            actor.id
        );
    }
}

#[test]
fn tool_list_headroom_holds_with_the_cone_op() {
    let server = McpServer::new(7, Capability::unrestricted(7));
    let ledger = server
        .tool_budget_gate_with_bloat(0)
        .expect("live list passes unbloated");
    eprintln!(
        "MEASURE tool-list total {} budget {} headroom {} floor {}",
        ledger.total_chars,
        TOOL_LIST_BUDGET_CHARS,
        ledger.headroom_chars,
        TOOL_LIST_MIN_HEADROOM_CHARS
    );
    assert!(ledger.headroom_chars >= TOOL_LIST_MIN_HEADROOM_CHARS);
    assert!(
        ledger.total_chars < 40_514,
        "default list {} stays under the door-v2 bound",
        ledger.total_chars
    );
}
