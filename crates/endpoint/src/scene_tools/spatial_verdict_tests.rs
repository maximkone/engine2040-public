//! Door tests for SPF8 role triage, SPF9 verdict rows, SPX7 z-fight notes and
//! the SPF10 folded `scene.brief`, over staged scenes with fixed ids.

use super::super::mcp::{SceneReadArgs, SceneReadTool};
use super::*;
use engine_editor::capability::Capability;
use engine_editor::commands::Journal;
use engine_scene::component::{AssetName, Collider, ColliderShape, Component, Mesh};

/// One staged node: where, and which components.
#[derive(Clone, Copy, Default)]
pub(crate) struct Node {
    pub(crate) at: [f32; 3],
    pub(crate) mesh: Option<&'static str>,
    /// Collider half extents in meters.
    pub(crate) collider: Option<[f32; 3]>,
}

pub(crate) fn at(x: f32, z: f32) -> Node {
    Node {
        at: [x, 0.0, z],
        ..Node::default()
    }
}

/// Fixed id of node `n`: id-text order is staging order.
pub(crate) fn uuid(n: usize) -> ObjectId {
    ObjectId::parse(&format!("00000000-0000-0000-0000-{n:012}")).expect("fixed id parses")
}

/// One batch per node, so the revision counts the nodes.
pub(crate) fn door(nodes: &[Node]) -> McpServer {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    for (n, node) in nodes.iter().enumerate() {
        let id = uuid(n + 1);
        let mut ops = Vec::new();
        ops.push(Op::Insert {
            id,
            parent: ObjectId::NIL,
            local: Transform::new(node.at, [0.0, 0.0, 0.0, 1.0], [1.0; 3]),
        });
        if let Some(asset) = node.mesh {
            ops.push(Op::SetComponent {
                id,
                component: Component::Mesh(Mesh {
                    asset: AssetName::try_from(asset).expect("asset name"),
                    cast_shadows: true,
                }),
            });
        }
        if let Some(half) = node.collider {
            ops.push(Op::SetComponent {
                id,
                component: Component::Collider(Collider {
                    shape: ColliderShape::Box,
                    half,
                    sensor: false,
                }),
            });
        }
        let batch = Batch {
            ops,
            base_revision: doc.revision,
            idempotency_key: 100 + n as u64,
            provenance: None,
        };
        commands::apply(&mut doc, &mut journal, &batch).expect("node stages");
    }
    McpServer::with_document(doc, journal, 7, Capability::unrestricted(7))
}

fn spatial(server: &McpServer, args: SceneSpatialArgs) -> Result<SceneSpatialOutput, McpToolError> {
    SceneSpatialTool::invoke(server, args)
}

fn overlaps(args: SceneSpatialArgs) -> SceneSpatialArgs {
    SceneSpatialArgs {
        checks: Some(vec!["overlap".to_owned()]),
        ..args
    }
}

/// `n` pairs of cubes, each pair on one footprint, pairs 10 m apart.
fn pairs(n: usize) -> McpServer {
    let nodes: Vec<Node> = (0..n * 2).map(|i| at((i / 2) as f32 * 10.0, 0.0)).collect();
    door(&nodes)
}

/// The allow row for pair `k` of [`pairs`]: `check a b verdict rev expiry reason`.
fn allow_pair(k: usize, verdict: &str, rev: u64, expiry: u64) -> String {
    let (a, b) = (uuid(2 * k + 1), uuid(2 * k + 2));
    format!("overlap {a} {b} {verdict} {rev} {expiry} stacked on purpose")
}

fn with_allow(rows: Vec<String>) -> SceneSpatialArgs {
    overlaps(SceneSpatialArgs {
        allow: Some(rows),
        ..Default::default()
    })
}

/// The learner lines of an answer.
fn learned(out: &SceneSpatialOutput) -> Vec<&String> {
    out.suppressed
        .iter()
        .filter(|line| line.starts_with("learned "))
        .collect()
}

#[test]
fn spatial_allow_keeps_the_finding_listed_and_out_of_the_red_count() {
    let server = pairs(2);
    let before = spatial(&server, overlaps(SceneSpatialArgs::default())).expect("reads");
    assert_eq!((before.verdict.as_str(), before.total), ("red", 2));
    let after = spatial(
        &server,
        with_allow(vec![allow_pair(0, "intentional", 0, 0)]),
    )
    .expect("reads with allow");
    assert_eq!(
        (after.verdict.as_str(), after.total),
        ("red", 1),
        "pair 1 stays red"
    );
    assert_eq!(
        after.hash, before.hash,
        "the hash covers the full finding set"
    );
    assert_eq!(after.suppressed.len(), 1);
    assert!(
        after.suppressed[0].starts_with("finding overlap 0 1 red")
            && after.suppressed[0].ends_with("suppressed intentional (allow): stacked on purpose"),
        "{:?}",
        after.suppressed
    );
    // With both pairs allowed nothing is left to worry about.
    let both = spatial(
        &server,
        with_allow(vec![
            allow_pair(0, "intentional", 0, 0),
            allow_pair(1, "deferred", 0, 0),
        ]),
    )
    .expect("reads");
    assert_eq!(
        (both.verdict.as_str(), both.total, both.suppressed.len()),
        ("clean", 0, 2)
    );
}

#[test]
fn spatial_allow_refuses_a_missing_reason_naming_the_field() {
    let server = pairs(1);
    let (a, b) = (uuid(1), uuid(2));
    let refuse =
        |row: String| spatial(&server, with_allow(vec![row])).expect_err("a bad row refuses");
    let missing = refuse(format!("overlap {a} {b} intentional 0 0"));
    assert_eq!(missing.code, "bad_allow");
    assert!(
        missing.message.contains("allow[0].reason"),
        "{}",
        missing.message
    );
    assert!(missing
        .message
        .contains("row grammar: check a b|- verdict rev expiry reason"));
    for (row, field) in [
        (format!("overlap {a} {b} fine 0 0 why"), "allow[0].verdict"),
        (
            format!("nope {a} {b} intentional 0 0 why"),
            "allow[0].check",
        ),
        (
            format!("overlap {a} {b} intentional x 0 why"),
            "allow[0].rev",
        ),
        (format!("overlap {a}"), "allow[0].b"),
    ] {
        let refused = refuse(row);
        assert!(
            refused.message.contains(field),
            "{field}: {}",
            refused.message
        );
    }
    assert_eq!(
        refuse(format!("overlap {} {b} intentional 0 0 why", uuid(99))).code,
        "unknown_actor"
    );
    let too_many = spatial(&server, with_allow(vec![allow_pair(0, "fixed", 0, 0); 257]))
        .expect_err("257 rows refuse");
    assert_eq!(too_many.code, "too_many_allow");
}

#[test]
fn spatial_allow_fixed_never_suppresses_and_an_expired_row_stops() {
    let server = pairs(1);
    let revision = SceneReadTool::invoke(&server, SceneReadArgs::default())
        .expect("scene reads")
        .revision;
    let total = |row: String| {
        spatial(&server, with_allow(vec![row]))
            .expect("reads")
            .total
    };
    assert_eq!(total(allow_pair(0, "fixed", 0, 0)), 1, "fixed is a note");
    let lapsed = allow_pair(0, "intentional", 1, revision - 1);
    assert_eq!(total(lapsed), 1, "expired at {}", revision - 1);
    let holding = allow_pair(0, "intentional", 1, revision);
    assert_eq!(total(holding), 0, "holds through its expiry revision");
}

#[test]
fn spatial_allow_three_verdicts_relax_level_1_and_twelve_cap_at_3() {
    let server = pairs(13);
    let ask = |n: usize, reversed: bool| {
        let mut rows: Vec<String> = (0..n)
            .map(|k| allow_pair(k, "false_positive", 0, 0))
            .collect();
        if reversed {
            rows.reverse();
        }
        let mut out = spatial(&server, with_allow(rows)).expect("reads");
        out.repeat_count = 0;
        out
    };
    assert!(
        learned(&ask(2, false)).is_empty(),
        "two verdicts teach nothing yet"
    );
    let three = ask(3, false);
    assert_eq!(learned(&three).len(), 1);
    assert!(
        learned(&three)[0].starts_with("learned overlap x prop: level 1 (3 verdicts"),
        "{:?}",
        three.suppressed
    );
    let twelve = ask(12, false);
    assert!(learned(&twelve)[0].starts_with("learned overlap x prop: level 3 (12 verdicts"));
    // Red errors never relax: the 13th pair is still there, red.
    assert_eq!((twelve.verdict.as_str(), twelve.total), ("red", 1));
    assert_eq!(
        twelve.suppressed.len(),
        13,
        "twelve findings plus the learner line"
    );
    // The same rows in any order give one table.
    assert_eq!(ask(12, true), twelve);
}

#[test]
fn spatial_triage_reads_a_collider_on_its_mesh_as_intentional() {
    let mesh = Node {
        mesh: Some("crate"),
        ..at(0.0, 0.0)
    };
    let wrap = Node {
        collider: Some([0.5, 0.5, 0.5]),
        ..at(0.0, 0.0)
    };
    let server = door(&[mesh, wrap, at(20.0, 0.0), at(20.0, 0.0)]);
    let plain = spatial(&server, overlaps(SceneSpatialArgs::default())).expect("reads");
    assert_eq!((plain.verdict.as_str(), plain.total), ("red", 2));
    let triaged = spatial(
        &server,
        overlaps(SceneSpatialArgs {
            triage: Some(true),
            ..Default::default()
        }),
    )
    .expect("reads triaged");
    assert_eq!(triaged.total, 2);
    assert!(
        triaged.findings[0].starts_with("finding overlap 2 3 red")
            && triaged.findings[0].ends_with(" error\n"),
        "errors that stop play lead: {:?}",
        triaged.findings
    );
    assert!(
        triaged.findings[1].starts_with("finding overlap 0 1 info")
            && triaged.findings[1].ends_with(" intentional\n"),
        "{:?}",
        triaged.findings
    );
    assert_eq!(triaged.hash, plain.hash);
    // Without the other pair the collider on its mesh is not a problem at all.
    let calm = door(&[mesh, wrap]);
    let verdict = |triage| {
        let args = SceneSpatialArgs {
            triage: Some(triage),
            ..Default::default()
        };
        spatial(&calm, overlaps(args)).expect("reads").verdict
    };
    assert_eq!(
        (verdict(false).as_str(), verdict(true).as_str()),
        ("red", "clean")
    );
}

#[test]
fn spatial_zfight_names_the_axis_and_the_fix_and_takes_an_epsilon() {
    // A 12 m x 9 m slab with its top at 10 cm and a wider plate whose top is
    // `lift` cm higher: the faces share 1,080,000 cm2.
    let scene = |lift: f32| {
        let slab = Node {
            at: [0.0, 0.05, 0.0],
            collider: Some([6.0, 0.05, 4.5]),
            ..Node::default()
        };
        let plate = Node {
            at: [0.0, 0.08 + lift, 0.0],
            collider: Some([7.0, 0.02, 5.5]),
            ..Node::default()
        };
        door(&[slab, plate])
    };
    let zfight = |server: &McpServer, epsilon_cm: Option<i32>| {
        spatial(
            server,
            SceneSpatialArgs {
                checks: Some(vec!["zfight".to_owned()]),
                epsilon_cm,
                ..Default::default()
            },
        )
    };
    let hit = zfight(&scene(0.0), None).expect("zfight is legal");
    assert_eq!(hit.total, 1);
    assert_eq!(
        hit.findings[0],
        "finding zfight 0 1 yellow 100 | faces 0 cm apart on y, 100% shared; move b +2 cm on y\n"
    );
    assert!(
        zfight(&scene(0.02), None)
            .expect("reads")
            .findings
            .is_empty(),
        "2 cm apart is clear"
    );
    assert_eq!(
        zfight(&scene(0.02), Some(2)).expect("reads").total,
        1,
        "a wider epsilon sees it"
    );
    let bad = zfight(&scene(0.0), Some(51)).expect_err("51 refuses");
    assert_eq!(bad.code, "bad_epsilon");
}

#[test]
fn spatial_brief_folds_forty_look_alikes_and_lists_the_stray() {
    let mut nodes: Vec<Node> = (0..40)
        .map(|i| at((i % 8) as f32 * 3.0, (i / 8) as f32 * 3.0))
        .collect();
    nodes.push(at(500.0, 0.0));
    let server = door(&nodes);
    let args = SceneBriefArgs {
        budget_bytes: Some(SCENE_BRIEF_MAX_BUDGET),
        ..Default::default()
    };
    let brief = SceneBriefTool::invoke(&server, args.clone()).expect("brief reads");
    let rows: Vec<&str> = brief
        .text
        .lines()
        .filter(|l| l.starts_with("object "))
        .collect();
    assert_eq!(
        rows,
        vec!["object prop x41 100x100x100 cm [0..40]"],
        "{}",
        brief.text
    );
    let extent = brief
        .text
        .lines()
        .find(|l| l.starts_with("extent "))
        .expect("extent row");
    assert!(extent.contains("excluded 40 why=stray"), "{extent}");
    // The handle table still carries one label per object, paged by the budget.
    assert_eq!(brief.objects.len(), brief.ids.len());
    assert!(brief.objects.len() > 10 && brief.next_cursor.is_some() == (brief.objects.len() < 41));
    let again = SceneBriefTool::invoke(
        &server,
        SceneBriefArgs {
            cursor: Some(0),
            ..args
        },
    )
    .expect("brief reads again");
    assert_eq!(again.text, brief.text);
}
