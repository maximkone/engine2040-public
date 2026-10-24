//! AD20 step 2 tests: the level's own nav bake reaches the door, and
//! `scene.query reachable` answers from the level the AI just built.

use super::nav_level::level_mesh;
use super::*;
use crate::build_level::{BuildLevelArgs, BuildLevelTool};
use crate::mcp::{McpServer, ServerState};
use crate::scene_tools::{SceneBriefArgs, SceneBriefTool};
use engine_editor::capability::Capability;
use engine_spatial::sight::{reachable, Reach};
use engine_spatial::{NavView, Tile};

/// Two crates in one 10 m room, the level build's own brief shape.
const BRIEF_CRATES: &str = "prop crate crate_01\n\
     want crate_01 next_to point:200,-300 [0,50]\n\
     prop crate crate_02\n\
     want crate_02 next_to object:crate_01 [50,150]";

/// A level tile bake from explicit walk flags: rows `(tx, tz, walk)`.
fn tiles(rows: &[(i32, i32, bool)]) -> NavView {
    let mut view = NavView::default();
    for &(tx, tz, walk) in rows {
        view.tiles.insert((tx, tz), Tile { walk });
    }
    view
}

/// A 4 x 2 m south block and the same north, optionally joined by the middle
/// row: the cheapest shape where one non-walkable row is a wall.
fn two_blocks(join: bool) -> NavView {
    let mut rows = Vec::new();
    for tx in -2..=1 {
        for tz in [-2, -1, 1, 2] {
            rows.push((tx, tz, true));
        }
        if join {
            rows.push((tx, 0, true));
        }
    }
    tiles(&rows)
}

fn labels(server: &McpServer) -> Vec<String> {
    SceneBriefTool::invoke(server, SceneBriefArgs::default())
        .expect("brief reads")
        .objects
}

fn door_reach(server: &McpServer, anchor: &str) -> SceneQueryOutput {
    SceneQueryTool::invoke(
        server,
        SceneQueryArgs {
            kind: "reachable".to_owned(),
            anchor: Some(anchor.to_owned()),
            ..Default::default()
        },
    )
    .expect("the built level answers reachable")
}

#[test]
fn level_mesh_keeps_the_level_walk_and_a_walled_row_blocks() {
    let walled = level_mesh(&two_blocks(false)).expect("the blocks bake");
    let rows = [
        (1, [-1.0, 0.5, -1.5]),
        (2, [-1.0, 0.5, 1.5]),
        (3, [9.0, 0.5, 9.0]),
    ];
    let view = engine_nav::reach::reach_view(&walled, &rows);
    assert_eq!(
        reachable(&view, 1, 2),
        Reach::Blocked,
        "the wall row seals it"
    );
    assert_eq!(
        reachable(&view, 1, 3),
        Reach::OffMesh,
        "off the level floor"
    );

    let joined = level_mesh(&two_blocks(true)).expect("the joined blocks bake");
    let view = engine_nav::reach::reach_view(&joined, &rows);
    assert_eq!(
        reachable(&view, 1, 2),
        Reach::Reachable,
        "a walkable row joins the blocks"
    );
    assert_eq!(reachable(&view, 1, 3), Reach::OffMesh);
    // A level with no walkable tile bakes nothing: the refusal stays honest.
    assert!(level_mesh(&tiles(&[(0, 0, false)])).is_none());
}

#[test]
fn build_level_hands_the_door_a_view_from_the_level_it_built() {
    let server = McpServer::new(11, Capability::unrestricted(11));
    let out = BuildLevelTool::invoke(
        &server,
        BuildLevelArgs {
            brief: BRIEF_CRATES.to_owned(),
            seed: 3000,
            expected_revision: 0,
            idempotency_key: 700,
        },
    )
    .expect("the level builds");
    assert!(out.solved, "{}", out.reason);
    assert_eq!(out.verdict, "pass");

    // The door now answers from the level it just built: both crates are on
    // the room floor the build baked, so neither pair is off-mesh.
    let names = labels(&server);
    assert_eq!(names.len(), 2, "two built props: {names:?}");
    let reach = door_reach(&server, &names[0]);
    let other = reach
        .hits
        .iter()
        .find(|hit| hit.object == names[1])
        .unwrap_or_else(|| panic!("{} has a row: {:?}", names[1], reach.hits));
    assert_eq!(other.reason, "reachable", "same room, same bake");
    let again = door_reach(&server, &names[0]);
    assert_eq!(reach.digest, again.digest, "twice the same bytes");
}

#[test]
fn handing_the_session_a_level_bake_arms_the_door_and_an_empty_bake_disarms_it() {
    let mut state = ServerState::with_document(
        engine_editor::commands::Document::new(),
        engine_editor::commands::Journal::new(),
        7,
        Capability::unrestricted(7),
    );
    state.hand_level_nav(&two_blocks(true));
    assert!(state.nav.is_some(), "the session holds the level mesh");
    // A level that baked no walkable tile leaves the refusal honest instead
    // of answering every pair from an empty world.
    state.hand_level_nav(&tiles(&[(0, 0, false)]));
    assert!(state.nav.is_none(), "an empty bake hands no view");
}
