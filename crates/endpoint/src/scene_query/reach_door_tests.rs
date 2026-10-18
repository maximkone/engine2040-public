//! AD20 tests: `scene.query reachable` answers from a baked nav mesh and
//! refuses `needs_nav_view` when the session holds none, so it never says yes
//! to every pair.

use super::*;
use crate::mcp::ServerState;
use crate::scene_tools::{SceneBriefArgs, SceneBriefTool};
use engine_editor::capability::Capability;
use engine_editor::commands::{self, Batch, Document, Journal, Op};
use engine_nav::mesh::{BakeConfig, NavMesh};
use engine_scene::transform::Transform;

type Tri = [[f32; 3]; 3];

/// Upward-facing floor quad `[x0, x1] x [z0, z1]` as two triangles.
fn quad(x0: f32, z0: f32, x1: f32, z1: f32) -> [Tri; 2] {
    [
        [[x0, 0.0, z0], [x0, 0.0, z1], [x1, 0.0, z0]],
        [[x1, 0.0, z0], [x0, 0.0, z1], [x1, 0.0, z1]],
    ]
}

/// A west room and an east room 2 m apart, joined by a doorway when asked.
fn rooms(with_door: bool) -> NavMesh {
    let mut floor: Vec<Tri> = Vec::new();
    floor.extend(quad(0.0, 0.0, 4.0, 4.0));
    floor.extend(quad(6.0, 0.0, 10.0, 4.0));
    if with_door {
        floor.extend(quad(4.0, 1.0, 6.0, 3.0));
    }
    NavMesh::bake(&floor, &BakeConfig::default()).expect("floor bakes")
}

/// Four crates under fixed ids, so the staging order is the order listed:
/// two in the west room, one in the east room, one off the floor.
const CRATES: [(&str, [f32; 3]); 4] = [
    ("11111111-1111-1111-1111-111111111111", [2.0, 0.5, 2.0]),
    ("22222222-2222-2222-2222-222222222222", [3.0, 0.5, 3.0]),
    ("33333333-3333-3333-3333-333333333333", [8.0, 0.5, 2.0]),
    ("44444444-4444-4444-4444-444444444444", [-9.0, 0.5, -9.0]),
];

fn state() -> ServerState {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let ops = CRATES
        .iter()
        .map(|(id, pos)| Op::Insert {
            id: ObjectId::parse(id).expect("fixed id parses"),
            parent: ObjectId::NIL,
            local: Transform::new(*pos, [0.0, 0.0, 0.0, 1.0], [1.0; 3]),
        })
        .collect();
    let batch = Batch {
        ops,
        base_revision: doc.revision,
        idempotency_key: 1,
        provenance: None,
    };
    commands::apply(&mut doc, &mut journal, &batch).expect("seed applies");
    ServerState::with_document(doc, journal, 7, Capability::unrestricted(7))
}

fn labels(server: &McpServer) -> Vec<String> {
    SceneBriefTool::invoke(server, SceneBriefArgs::default())
        .expect("brief reads")
        .objects
}

fn reachable_from(server: &McpServer, anchor: &str) -> Result<SceneQueryOutput, McpToolError> {
    SceneQueryTool::invoke(
        server,
        SceneQueryArgs {
            kind: "reachable".to_owned(),
            anchor: Some(anchor.to_owned()),
            ..Default::default()
        },
    )
}

fn reason_of(out: &SceneQueryOutput, label: &str) -> String {
    out.hits
        .iter()
        .find(|hit| hit.object == label)
        .unwrap_or_else(|| panic!("{label} has a row: {:?}", out.hits))
        .reason
        .clone()
}

#[test]
fn reach_door_refuses_needs_nav_view_when_no_mesh_is_held() {
    let server = McpServer::with_state(state());
    let labels = labels(&server);
    let error = reachable_from(&server, &labels[0]).expect_err("no mesh, no answer");
    assert_eq!(error.code, "needs_nav_view");
    assert!(
        error.message.contains("baked nav mesh"),
        "{}",
        error.message
    );
    // The refusal is for reachable only: kinds that need no floor still answer.
    let near = SceneQueryTool::invoke(
        &server,
        SceneQueryArgs {
            kind: "near".to_owned(),
            anchor: Some(labels[0].clone()),
            radius_cm: Some(500),
            ..Default::default()
        },
    )
    .expect("near needs no mesh");
    assert!(near.count > 0);
}

#[test]
fn reach_door_two_rooms_are_blocked_across_the_wall_until_a_door_joins_them() {
    let walled = McpServer::with_state(state().with_nav(rooms(false)));
    let labels = labels(&walled);
    let out = reachable_from(&walled, &labels[0]).expect("a mesh answers");
    assert_eq!(reason_of(&out, &labels[1]), "reachable", "same room");
    assert_eq!(
        reason_of(&out, &labels[2]),
        "blocked:unreachable",
        "across the wall"
    );
    assert_eq!(
        reason_of(&out, &labels[3]),
        "blocked:off_mesh",
        "off the floor"
    );

    let joined = McpServer::with_state(state().with_nav(rooms(true)));
    let out = reachable_from(&joined, &labels[0]).expect("a mesh answers");
    assert_eq!(reason_of(&out, &labels[2]), "reachable", "through the door");
    assert_eq!(reason_of(&out, &labels[3]), "blocked:off_mesh");
    // Twice the same bytes.
    let again = reachable_from(&joined, &labels[0]).expect("a mesh answers");
    assert_eq!(out.digest, again.digest);
}
