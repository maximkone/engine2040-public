//! Door tests for the lane-B questions: a plain-language floor plan for
//! `relation` (not only door versus oracle), pair numbers on its hits, and the
//! sees, why_hidden, clearance, slide and ground kinds.

use super::*;
use engine_editor::capability::Capability;
use engine_editor::commands::{self, Batch, Document, Journal, Op};
use engine_scene::component::{AssetName, Component, Mesh};
use engine_scene::ids::ObjectId;
use engine_scene::transform::Transform;

/// Floor plan, meters, +X east, -Z north, every piece a 1 m cube:
/// table at the origin, lamp 3 m west, chair 3 m east, sofa 3 m north and,
/// when `wall`, a wall piece 1.5 m west of the table (between lamp and table).
fn room(wall: bool) -> McpServer {
    let mut spots = vec![
        ("table", [0.0, 0.0, 0.0]),
        ("lamp", [-3.0, 0.0, 0.0]),
        ("chair", [3.0, 0.0, 0.0]),
        ("sofa", [0.0, 0.0, -3.0]),
    ];
    if wall {
        spots.push(("wall", [-1.5, 0.0, 0.0]));
    }
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let mut ops = Vec::new();
    for (name, pos) in spots {
        let id = ObjectId::new();
        ops.push(Op::Insert {
            id,
            parent: ObjectId::NIL,
            local: Transform::new(pos, [0.0, 0.0, 0.0, 1.0], [1.0; 3]),
        });
        ops.push(Op::SetComponent {
            id,
            component: Component::Mesh(Mesh {
                asset: AssetName::try_from(name).expect("legal name"),
                cast_shadows: true,
            }),
        });
    }
    let batch = Batch {
        ops,
        base_revision: doc.revision,
        idempotency_key: 1,
        provenance: None,
    };
    commands::apply(&mut doc, &mut journal, &batch).expect("room seeds");
    McpServer::with_document(doc, journal, 7, Capability::unrestricted(7))
}

/// The door label of the piece named `name` (`table_1 [uuid]`).
fn label(server: &McpServer, name: &str) -> String {
    let brief = crate::scene_tools::SceneBriefTool::invoke(
        server,
        crate::scene_tools::SceneBriefArgs::default(),
    )
    .expect("brief reads");
    brief
        .objects
        .into_iter()
        .find(|l| l.starts_with(&format!("{name}_1 [")))
        .expect("piece is staged")
}

fn ask(server: &McpServer, args: SceneQueryArgs) -> SceneQueryOutput {
    SceneQueryTool::invoke(server, args).expect("query serves")
}

fn base(kind: &str, server: &McpServer, anchor: &str) -> SceneQueryArgs {
    SceneQueryArgs {
        kind: kind.to_owned(),
        anchor: Some(label(server, anchor)),
        matched_only: Some(false),
        ..Default::default()
    }
}

/// Plain-language gold: the lamp stands west of the table, so it is left of
/// the table, 300 cm away, bearing 270, 200 cm of floor between them; the
/// chair is east, so it is right of the table. Nothing else qualifies.
#[test]
fn relation_plain_left_and_right_of_the_table() {
    let server = room(false);
    let ask_rel = |word: &str| {
        let mut args = base("relation", &server, "table");
        args.relation = Some(word.to_owned());
        ask(&server, args)
    };
    let left = ask_rel("left_of");
    assert_eq!(left.hits.len(), 1, "{left:?}");
    assert!(left.hits[0].object.starts_with("lamp_1 ["));
    let reason = &left.hits[0].reason;
    assert!(
        reason.contains("300 cm, bearing 270 deg, gap 200 cm"),
        "{reason}"
    );
    let right = ask_rel("right_of");
    assert_eq!(right.hits.len(), 1, "{right:?}");
    assert!(right.hits[0].object.starts_with("chair_1 ["));
    assert!(
        right.hits[0].reason.contains("bearing 90 deg"),
        "{}",
        right.hits[0].reason
    );
}

/// A wall between lamp and table hides the table from the lamp and is named
/// as the blocker; with the sofa the way is clear.
#[test]
fn query_space_sees_names_the_blocker() {
    let server = room(true);
    let mut args = base("sees", &server, "lamp");
    args.other = Some(label(&server, "table"));
    let blocked = ask(&server, args);
    // Two unmatched rows tie on the match bit and sort by actor index, which
    // follows the random object ids: find each row by its label, not by place.
    assert_eq!(blocked.hits.len(), 2, "{blocked:?}");
    let row_of = |name: &str| {
        blocked
            .hits
            .iter()
            .find(|hit| hit.object.starts_with(&format!("{name}_1 [")))
            .unwrap_or_else(|| panic!("no `{name}` row in {blocked:?}"))
    };
    let target = row_of("table");
    assert!(!target.matched, "{blocked:?}");
    assert!(
        target.reason.starts_with("sees: hidden"),
        "{}",
        target.reason
    );
    let blocker = row_of("wall");
    assert!(!blocker.matched, "{blocked:?}");
    assert!(
        blocker.reason.starts_with("blocks the view at "),
        "{}",
        blocker.reason
    );
    let mut clear = base("sees", &server, "sofa");
    clear.other = Some(label(&server, "table"));
    let seen = ask(&server, clear);
    assert!(seen.hits[0].matched, "{seen:?}");
    assert_eq!(seen.hits.len(), 1, "no blocker row when the way is clear");
}

/// Clearance reports the 50 cm gap to the wall as too narrow for 60 cm, with
/// the number to act on; slide stops 200 cm short of the chair; ground finds
/// the chair top under a point above it; why_hidden gives one verdict word.
#[test]
fn query_space_clearance_slide_ground_and_why_hidden() {
    let server = room(true);
    let mut narrow = base("clearance", &server, "table");
    narrow.radius_cm = Some(60);
    let hit = &ask(&server, narrow).hits[0];
    assert!(hit.object.starts_with("wall_1 ["), "{}", hit.object);
    assert!(
        !hit.matched && hit.reason.contains("narrow gap 50 cm"),
        "{}",
        hit.reason
    );

    let mut slid = base("slide", &server, "table");
    slid.dir = Some("+x".to_owned());
    let out = ask(&server, slid);
    assert!(
        out.hits[0].reason.contains("free 200 cm"),
        "{}",
        out.hits[0].reason
    );
    assert!(out.hits[1].object.starts_with("chair_1 ["), "{out:?}");

    let ground = ask(
        &server,
        SceneQueryArgs {
            kind: "ground".to_owned(),
            eye_m: Some([3.0, 5.0, 0.0]),
            ..Default::default()
        },
    );
    assert!(ground.hits[0].object.starts_with("chair_1 ["), "{ground:?}");
    assert!(
        ground.hits[0].reason.contains("top 50 cm"),
        "{}",
        ground.hits[0].reason
    );

    let mut why = base("why_hidden", &server, "sofa");
    why.eye_m = Some([0.0, 0.5, 5.0]);
    why.yaw = Some(0.0);
    let verdict = ask(&server, why);
    assert!(
        verdict.hits[0].reason.starts_with("why_hidden: "),
        "{}",
        verdict.hits[0].reason
    );
    assert!(!verdict.hits[0].matched, "{verdict:?}");

    let mut bad = base("slide", &server, "table");
    bad.dir = Some("sideways".to_owned());
    let refused = SceneQueryTool::invoke(&server, bad).expect_err("bad dir refuses");
    assert_eq!(refused.code, "bad_dir");
    assert!(refused.message.contains("+x"), "{}", refused.message);
}
