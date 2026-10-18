//! Door tests for SPF3 facing wants and SPX10 `outside` on `scene.place`.

use super::super::mcp::{SceneReadArgs, SceneReadTool};
use super::spatial_verdict_tests::{at, door, uuid};
use super::*;

fn place(server: &McpServer, object: usize, wants: &[String]) -> ScenePlaceOutput {
    let revision = SceneReadTool::invoke(server, SceneReadArgs::default())
        .expect("scene reads")
        .revision;
    ScenePlaceTool::invoke(
        server,
        ScenePlaceArgs {
            expected_revision: revision,
            idempotency_key: 900 + revision,
            object: uuid(object).to_string(),
            wants: wants.to_vec(),
            delta: Some(false),
        },
    )
    .expect("place answers")
}

fn cm(meters: f32) -> i32 {
    (meters * 100.0).round() as i32
}

/// Player (node 1) at the origin facing +X, crate (node 2) in the far corner.
fn player_and_crate() -> McpServer {
    door(&[at(0.0, 0.0), at(8.0, 8.0)])
}

#[test]
fn place_facing_puts_a_crate_in_front_of_the_player_in_one_call() {
    let server = player_and_crate();
    let want = format!("in_front_of object:{} [20,60]", uuid(1));
    let out = place(&server, 2, &[want]);
    assert!(out.solved, "{}", out.reason);
    let at = out.placement.expect("a placement");
    // The player's box ends at +50 cm and the crate is 100 cm wide: the edge
    // gap is the crate centre minus 100.
    let gap = cm(at.x) - 100;
    assert!((20..=60).contains(&gap), "gap {gap} cm");
    assert!(
        cm(at.x) > 0,
        "in front of a player who faces +x is east of it"
    );
}

#[test]
fn place_facing_an_impossible_gap_names_the_want() {
    let server = player_and_crate();
    let want = format!("left_of object:{} [90000,90001]", uuid(1));
    let out = place(&server, 2, std::slice::from_ref(&want));
    assert!(!out.solved);
    assert_eq!(out.conflicts, vec![want]);
    assert!(out.reason.contains("no placement"), "{}", out.reason);
}

#[test]
fn place_facing_refuses_to_mix_with_classic_wants() {
    let server = player_and_crate();
    let revision = SceneReadTool::invoke(&server, SceneReadArgs::default())
        .expect("scene reads")
        .revision;
    let refused = ScenePlaceTool::invoke(
        &server,
        ScenePlaceArgs {
            expected_revision: revision,
            idempotency_key: 5,
            object: uuid(2).to_string(),
            wants: vec![
                format!("in_front_of object:{}", uuid(1)),
                "next_to wall:north [0,60]".to_owned(),
            ],
            delta: None,
        },
    )
    .expect_err("mixing refuses");
    assert_eq!(refused.code, "bad_want");
    let bad = ScenePlaceTool::invoke(
        &server,
        ScenePlaceArgs {
            expected_revision: revision,
            idempotency_key: 6,
            object: uuid(2).to_string(),
            wants: vec!["in_front_of wall:north".to_owned()],
            delta: None,
        },
    )
    .expect_err("a wall is no anchor");
    assert_eq!(bad.code, "bad_want");
}

#[test]
fn place_outside_keeps_a_free_pose_exactly_and_clears_a_region() {
    let server = player_and_crate();
    let free = place(&server, 2, &["outside region:100,100,300,300".to_owned()]);
    assert!(free.solved, "{}", free.reason);
    let kept = free.placement.expect("a placement");
    assert_eq!((cm(kept.x), cm(kept.z)), (800, 800), "a free pose stays");
    // A region over the crate moves it out with its footprint clear.
    let moved = place(&server, 2, &["outside region:700,700,900,900".to_owned()]);
    assert!(moved.solved, "{}", moved.reason);
    let p = moved.placement.expect("a placement");
    let (x, z) = (cm(p.x), cm(p.z));
    assert!(
        x + 50 <= 700 || 900 <= x - 50 || z + 50 <= 700 || 900 <= z - 50,
        "({x}, {z}) still touches the region"
    );
    // Another object is a legal target; an unknown id is a conflict.
    let by_object = place(&server, 2, &[format!("outside object:{}", uuid(1))]);
    assert!(by_object.solved, "{}", by_object.reason);
    let ghost = place(&server, 2, &[format!("outside object:{}", uuid(77))]);
    assert!(
        !ghost.solved && ghost.reason.starts_with("unknown-target"),
        "{}",
        ghost.reason
    );
}
