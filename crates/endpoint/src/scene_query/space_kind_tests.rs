//! Door tests for the `measure`, `room_of`, `in_room`, `exits_of` and `glance`
//! kinds, over staged scenes with fixed ids.

use super::*;
use crate::scene_tools::spatial_verdict_tests::{at, door, uuid, Node};

fn ask(server: &McpServer, args: SceneQueryArgs) -> SceneQueryOutput {
    SceneQueryTool::invoke(server, args).expect("query serves")
}

fn refuse(server: &McpServer, args: SceneQueryArgs) -> McpToolError {
    SceneQueryTool::invoke(server, args).expect_err("the query refuses")
}

fn call(kind: &str) -> SceneQueryArgs {
    SceneQueryArgs {
        kind: kind.to_owned(),
        matched_only: Some(false),
        ..Default::default()
    }
}

#[test]
fn query_measure_reads_the_gap_and_the_shortfall() {
    // Two 1 m crates, 30 cm apart on x.
    let server = door(&[at(0.0, 0.0), at(1.3, 0.0)]);
    let pair = |args: SceneQueryArgs| SceneQueryArgs {
        anchor: Some(uuid(1).to_string()),
        other: Some(uuid(2).to_string()),
        ..args
    };
    let gap = ask(
        &server,
        pair(SceneQueryArgs {
            what: Some("gap".to_owned()),
            radius_cm: Some(90),
            ..call("measure")
        }),
    );
    let hit = &gap.hits[0];
    assert!(
        hit.reason
            .starts_with("measure gap: gap 30 cm, axis_gap [30, -100, -100], apart")
            && hit.reason.ends_with("short by 60 cm"),
        "{}",
        hit.reason
    );
    assert!(!hit.matched);
    // A looser bar passes.
    let ok = ask(
        &server,
        pair(SceneQueryArgs {
            radius_cm: Some(20),
            ..call("measure")
        }),
    );
    assert!(ok.hits[0].matched && ok.hits[0].reason.ends_with("passed"));
    // Aligned by centers on every axis: 130 cm off on x, less the 1 cm tolerance.
    let align = ask(
        &server,
        pair(SceneQueryArgs {
            what: Some("align".to_owned()),
            ..call("measure")
        }),
    );
    assert!(
        align.hits[0].reason.ends_with("short by 129 cm"),
        "{}",
        align.hits[0].reason
    );
    // Bad words refuse naming the legal ones.
    let bad = refuse(
        &server,
        pair(SceneQueryArgs {
            what: Some("near".to_owned()),
            ..call("measure")
        }),
    );
    assert_eq!(bad.code, "bad_what");
    assert!(
        bad.message.contains("gap, overlap, align, contain"),
        "{}",
        bad.message
    );
}

/// A two-room house, walls 20 cm thick and 3 m tall: rooms 400 cm wide split
/// by a wall with a 120 cm doorway. A crate stands in each room.
fn house() -> McpServer {
    let wall = |lo: [i32; 2], hi: [i32; 2]| {
        let m = |v: i32| v as f32 / 100.0;
        Node {
            at: [m(lo[0] + hi[0]) / 2.0, 1.5, m(lo[1] + hi[1]) / 2.0],
            collider: Some([m(hi[0] - lo[0]) / 2.0, 1.5, m(hi[1] - lo[1]) / 2.0]),
            ..Node::default()
        }
    };
    door(&[
        wall([0, 0], [860, 20]),
        wall([0, 420], [860, 440]),
        wall([0, 20], [20, 420]),
        wall([840, 20], [860, 420]),
        wall([420, 20], [440, 160]),
        wall([420, 280], [440, 420]),
        at(2.2, 2.2),
        at(6.4, 2.2),
    ])
}

#[test]
fn query_rooms_name_the_room_its_members_and_its_exit() {
    let server = house();
    let west = uuid(7).to_string();
    let east = uuid(8).to_string();
    let of = |anchor: &str| {
        ask(
            &server,
            SceneQueryArgs {
                anchor: Some(anchor.to_owned()),
                ..call("room_of")
            },
        )
    };
    let (a, b) = (of(&west), of(&east));
    assert!(
        a.hits[0].reason.starts_with("room_of: room_1 area "),
        "{}",
        a.hits[0].reason
    );
    assert!(
        b.hits[0].reason.starts_with("room_of: room_2 area "),
        "{}",
        b.hits[0].reason
    );
    assert!(
        a.hits[0].reason.contains("1 exit(s), 1 member(s)"),
        "{}",
        a.hits[0].reason
    );
    // Members of room 2: the east crate only.
    let members = ask(
        &server,
        SceneQueryArgs {
            room: Some(2),
            ..call("in_room")
        },
    );
    assert_eq!(members.hits.len(), 1);
    assert!(members.hits[0].object.contains(&east) && members.hits[0].reason == "in_room: room_2");
    // Exits by room number or by an object in it: the one 120 cm doorway.
    for args in [
        SceneQueryArgs {
            room: Some(1),
            ..call("exits_of")
        },
        SceneQueryArgs {
            anchor: Some(east.clone()),
            ..call("exits_of")
        },
    ] {
        let exits = ask(&server, args);
        assert_eq!(exits.hits.len(), 1, "{:?}", exits.hits);
        assert!(
            exits.hits[0].reason.ends_with("to room_2, 120 cm wide")
                || exits.hits[0].reason.ends_with("to room_1, 120 cm wide"),
            "{}",
            exits.hits[0].reason
        );
    }
    // An unknown room refuses; a missing field says which.
    let ghost = refuse(
        &server,
        SceneQueryArgs {
            room: Some(9),
            ..call("in_room")
        },
    );
    assert_eq!(ghost.code, "bad_room");
    assert_eq!(refuse(&server, call("in_room")).code, "missing_arg");
}

#[test]
fn query_glance_lists_coverage_and_names_the_hidden_crate() {
    // Eye at the origin facing +x: a pillar 3 m ahead, a crate behind it,
    // another crate off to the side that stays in view.
    let pillar = Node {
        at: [3.0, 1.0, 0.0],
        collider: Some([0.5, 1.0, 0.5]),
        ..Node::default()
    };
    let hidden = Node {
        at: [6.0, 1.0, 0.0],
        collider: Some([0.5, 0.5, 0.5]),
        ..Node::default()
    };
    let side = Node {
        at: [6.0, 1.0, -4.0],
        collider: Some([0.5, 0.5, 0.5]),
        ..Node::default()
    };
    let server = door(&[pillar, hidden, side]);
    let out = ask(
        &server,
        SceneQueryArgs {
            eye_m: Some([0.0, 1.0, 0.0]),
            yaw: Some(0.0),
            ..call("glance")
        },
    );
    let row = |n: usize| {
        out.hits
            .iter()
            .find(|h| h.object.contains(&uuid(n).to_string()))
    };
    assert!(row(1).is_some_and(|h| h.matched && h.reason.starts_with("glance: ")));
    assert!(
        row(3).is_some_and(|h| h.matched),
        "the side crate is visible"
    );
    assert!(
        row(2).is_some_and(|h| !h.matched && h.reason == "glance: occluded"),
        "{:?}",
        out.hits
    );
    let summary = out
        .hits
        .iter()
        .find(|h| h.reason.starts_with("glance 24x24 fov 90"))
        .expect("summary row");
    assert!(summary.reason.contains("background ") && summary.reason.contains("tokens"));
    // The same call twice gives the same bytes; bad shapes refuse.
    let again = ask(
        &server,
        SceneQueryArgs {
            eye_m: Some([0.0, 1.0, 0.0]),
            yaw: Some(0.0),
            ..call("glance")
        },
    );
    assert_eq!(again.digest, out.digest);
    let bad = refuse(
        &server,
        SceneQueryArgs {
            eye_m: Some([0.0, 1.0, 0.0]),
            half_angle_deg: Some(90),
            ..call("glance")
        },
    );
    assert_eq!(bad.code, "bad_view");
}

/// SPF4 at the door: the gap and the push of the nearest neighbour, in cm and
/// permille, for the four Forge cases and the move the push asks for.
#[test]
fn query_clearance_reads_the_gap_the_push_and_holds_after_the_move() {
    let clearance = |anchor: Node, neighbour: Node, need_cm: i32| {
        let server = door(&[anchor, neighbour]);
        let out = ask(
            &server,
            SceneQueryArgs {
                anchor: Some(uuid(1).to_string()),
                radius_cm: Some(need_cm),
                ..call("clearance")
            },
        );
        (out.hits[0].matched, out.hits[0].reason.clone())
    };
    let at_xz = |x: f32, z: f32| at(x, z);
    // Apart 30 cm on x, the neighbour to the west: gap 30, push +x, short of 60.
    let (holds, reason) = clearance(at_xz(0.0, 0.0), at_xz(-1.3, 0.0), 60);
    assert!(!holds);
    assert_eq!(
        reason,
        "clearance: narrow gap 30 cm, need 60; push_pm [1000, 0, 0]"
    );
    // Touching reads 0 and exact.
    let (_, reason) = clearance(at_xz(0.0, 0.0), at_xz(-1.0, 0.0), 60);
    assert!(reason.starts_with("clearance: exact gap 0 cm"), "{reason}");
    // 12 cm of overlap on the shallowest axis: -12, pushed toward the anchor's side.
    let (_, reason) = clearance(at_xz(0.0, 0.0), at_xz(-0.88, 0.0), 60);
    assert_eq!(
        reason,
        "clearance: exact gap -12 cm, need 60; push_pm [1000, 0, 0]"
    );
    // A 30 cm by 40 cm corner gap is 50 cm.
    let (_, reason) = clearance(at_xz(0.0, 0.0), at_xz(-1.3, -1.4), 60);
    assert!(
        reason.starts_with("clearance: narrow gap 50 cm, need 60"),
        "{reason}"
    );
    // Moving the anchor by the push times (need - gap) makes the same query hold.
    let (holds, reason) = clearance(at_xz(0.3, 0.0), at_xz(-1.3, 0.0), 60);
    assert!(
        holds && reason.starts_with("clearance: upper_bound gap 60 cm"),
        "{reason}"
    );
}

#[test]
fn query_coverage_reads_the_share_the_verdict_and_the_next_call() {
    // A 2 m crate 6 m ahead of the eye; yaw 0 faces +x.
    let wide = Node {
        collider: Some([0.5, 1.0, 1.0]),
        ..at(6.0, 0.0)
    };
    let server = door(&[wide]);
    let cover = |yaw: f32, pitch: f32| {
        let out = ask(
            &server,
            SceneQueryArgs {
                anchor: Some(uuid(1).to_string()),
                eye_m: Some([0.0, 0.0, 0.0]),
                yaw: Some(yaw),
                pitch: Some(pitch),
                half_angle_deg: Some(15),
                ..call("coverage")
            },
        );
        (out.hits[0].matched, out.hits[0].reason.clone())
    };
    let (seen, reason) = cover(0.0, 0.0);
    assert!(
        seen && reason.starts_with("coverage: ") && reason.ends_with(" pm, ok"),
        "{reason}"
    );
    // Turned around: 0 pm, looks away, and the next call is named.
    let (seen, reason) = cover(std::f32::consts::PI, 0.0);
    assert!(
        !seen && reason.starts_with("coverage: 0 pm, looks_away; Next call: "),
        "{reason}"
    );
    // Pitched over the top the camera is flipped.
    let (_, reason) = cover(0.0, 2.6);
    assert!(reason.contains("flipped"), "{reason}");
    let bad = refuse(
        &server,
        SceneQueryArgs {
            anchor: Some(uuid(1).to_string()),
            eye_m: Some([0.0, 0.0, 0.0]),
            pitch: Some(4.0),
            ..call("coverage")
        },
    );
    assert_eq!(bad.code, "bad_view");
}
