//! SPR1 and SPX9 tests: every staged object gets a handle (`lamp_2 [id]`)
//! and a place (cm pose row), the wire tables obey the brief byte budget with
//! a cursor, an unrelated insert relabels nothing, and `scene.spatial` rolls
//! its findings up per actor.

use super::*;
use engine_editor::capability::Capability;
use engine_editor::commands::Journal;
use engine_scene::component::{AssetName, Collider, ColliderShape, Component, Mesh};

fn mesh(asset: &str) -> Component {
    Component::Mesh(Mesh {
        asset: AssetName::try_from(asset).expect("test asset name is legal"),
        cast_shadows: true,
    })
}

/// One batch of root nodes: `(mesh asset or "", position, extra component)`.
fn insert_nodes(
    doc: &mut Document,
    journal: &mut Journal,
    key: u64,
    nodes: &[(&str, [f32; 3], Option<Component>)],
) -> Vec<ObjectId> {
    let mut ops = Vec::new();
    let mut ids = Vec::new();
    for (asset, pos, extra) in nodes {
        let id = ObjectId::new();
        ids.push(id);
        ops.push(Op::Insert {
            id,
            parent: ObjectId::NIL,
            local: Transform::new(*pos, [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
        });
        if !asset.is_empty() {
            ops.push(Op::SetComponent {
                id,
                component: mesh(asset),
            });
        }
        if let Some(component) = extra {
            ops.push(Op::SetComponent {
                id,
                component: *component,
            });
        }
    }
    let batch = Batch {
        ops,
        base_revision: doc.revision,
        idempotency_key: key,
        provenance: None,
    };
    commands::apply(doc, journal, &batch).expect("seed applies");
    ids
}

/// `count` nodes on a 3 m grid, alternating lamp and crate meshes.
fn grid(count: usize) -> Vec<(&'static str, [f32; 3], Option<Component>)> {
    (0..count)
        .map(|i| {
            let asset = if i % 2 == 0 { "lamp" } else { "crate" };
            (
                asset,
                [(i % 20) as f32 * 3.0, 0.0, (i / 20) as f32 * 3.0],
                None,
            )
        })
        .collect()
}

fn brief_page(server: &McpServer, budget: u32, cursor: Option<u32>) -> SceneBriefOutput {
    SceneBriefTool::invoke(
        server,
        SceneBriefArgs {
            budget_bytes: Some(budget),
            cursor,
            ..Default::default()
        },
    )
    .expect("brief reads")
}

/// Every page of the tables, following `next_cursor` to the end.
fn all_pages(server: &McpServer, budget: u32) -> (Vec<String>, Vec<String>, usize) {
    let (mut objects, mut poses, mut pages) = (Vec::new(), Vec::new(), 0);
    let mut cursor = None;
    loop {
        let page = brief_page(server, budget, cursor);
        pages += 1;
        objects.extend(page.objects);
        poses.extend(page.poses);
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return (objects, poses, pages),
        }
    }
}

/// 200 objects read back as `lamp_2 [id] x y z ...` rows with unique handles;
/// the default budget keeps one call under 6 KB and a cursor walks the rest.
#[test]
fn brief_handles_200_objects_read_back_within_budget() {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let ids = insert_nodes(&mut doc, &mut journal, 1, &grid(200));
    let server = McpServer::with_document(doc, journal, 7, Capability::unrestricted(7));

    let first = brief_page(&server, SCENE_BRIEF_DEFAULT_BUDGET, None);
    let wire = rmcp::serde_json::to_string(&first).expect("output serializes");
    assert!(
        wire.len() < 6 * 1024,
        "default call is {} bytes",
        wire.len()
    );
    assert!(first.next_cursor.is_some(), "200 objects need a cursor");
    assert_eq!(first.ids.len(), first.poses.len());
    assert_eq!(first.ids.len(), first.objects.len());

    let (objects, poses, pages) = all_pages(&server, SCENE_BRIEF_MAX_BUDGET);
    assert!(pages > 1, "even the widest page needs a cursor");
    assert_eq!(objects.len(), 200);
    let handles: std::collections::BTreeSet<&str> = objects
        .iter()
        .map(|label| label.split(" [").next().expect("label has a name"))
        .collect();
    assert_eq!(handles.len(), 200, "every handle is unique");
    for want in ["lamp_1", "lamp_100", "crate_1", "crate_100"] {
        assert!(handles.contains(want), "missing handle {want}");
    }
    // Every label still resolves through the bracketed id.
    for (label, id) in objects.iter().zip(sorted(&ids)) {
        assert_eq!(
            &parse_object_ref("object", label).expect("label parses"),
            id
        );
    }
    // Index 0 is the id-sorted first node; its pose row names it and its cm.
    let name0 = objects[0].split(" [").next().expect("name part");
    assert!(
        poses[0].starts_with(&format!("{name0} [0] ")),
        "{}",
        poses[0]
    );
    assert_eq!(
        poses[0].split(' ').count(),
        10,
        "name [i] x y z sx sy sz dir #digest"
    );
    assert!(poses[0].contains(" 100 100 100 +x #"), "{}", poses[0]);
    // The brief TEXT names the same object with the same cm pose and digest,
    // so an agent reads coordinates without a second call.
    assert!(first.text.len() <= SCENE_BRIEF_DEFAULT_BUDGET as usize);
    let text_place = first
        .text
        .lines()
        .find(|line| line.starts_with(&format!("{name0} [0] ")))
        .unwrap_or_else(|| panic!("no place row for {name0} in:\n{}", first.text));
    assert!(text_place.contains(" 100 100 100 +x #"), "{text_place}");
    assert_eq!(
        text_place.rsplit(' ').next(),
        poses[0].rsplit(' ').next(),
        "text and table carry one digest"
    );
    eprintln!(
        "brief_handles: default call {} bytes, {pages} pages at max",
        wire.len()
    );
}

fn sorted(ids: &[ObjectId]) -> Vec<&ObjectId> {
    let mut v: Vec<&ObjectId> = ids.iter().collect();
    v.sort_by_key(|id| id.to_string());
    v
}

/// Inserting a node of another mesh relabels no existing node, and every
/// other node's pose row, cm numbers and own `#digest` are unchanged. The
/// scene digest does move (it counts nodes and edges), so the per-object
/// digest is the stable proof.
#[test]
fn brief_handles_survive_an_unrelated_insert() {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    insert_nodes(&mut doc, &mut journal, 1, &grid(6));
    let server = McpServer::with_document(doc, journal, 7, Capability::unrestricted(7));
    let by_id = |server: &McpServer| {
        let (objects, poses, _) = all_pages(server, SCENE_BRIEF_MAX_BUDGET);
        objects
            .into_iter()
            .zip(poses)
            .map(|(label, pose)| {
                let tail = pose.split_once("] ").expect("pose tail").1.to_owned();
                let digest = pose.rsplit(' ').next().expect("digest tail").to_owned();
                (label, (tail, digest))
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let scene_before = brief_page(&server, SCENE_BRIEF_DEFAULT_BUDGET, None).digest;
    let before = by_id(&server);
    {
        let mut state = server.lock().expect("test lock holds");
        let state = &mut *state;
        insert_nodes(
            &mut state.doc,
            &mut state.journal,
            2,
            &[("rock", [-9.0, 0.0, 9.0], None)],
        );
    }
    let scene_after = brief_page(&server, SCENE_BRIEF_DEFAULT_BUDGET, None).digest;
    let after = by_id(&server);
    assert_eq!(after.len(), before.len() + 1);
    assert_ne!(
        scene_after, scene_before,
        "the scene digest counts nodes, so it moves by construction"
    );
    for (label, (tail, digest)) in &before {
        let (new_tail, new_digest) = after.get(label).expect("label survives");
        assert_eq!(new_tail, tail, "{label} moved");
        assert_eq!(new_digest, digest, "{label} digest moved");
    }
}

/// A collider sizes the box (box, scale applied); a node without one stays
/// the 1 m default, and `size_source` counts both.
#[test]
fn brief_handles_size_from_collider_component() {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let collider = Component::Collider(Collider {
        shape: ColliderShape::Box,
        half: [1.0, 0.5, 2.0],
        sensor: false,
    });
    insert_nodes(
        &mut doc,
        &mut journal,
        1,
        &[
            ("wall", [0.0, 0.0, 0.0], Some(collider)),
            ("lamp", [5.0, 0.0, 0.0], None),
        ],
    );
    let server = McpServer::with_document(doc, journal, 7, Capability::unrestricted(7));
    let page = brief_page(&server, SCENE_BRIEF_DEFAULT_BUDGET, None);
    let wall = page
        .poses
        .iter()
        .find(|row| row.starts_with("wall_1 "))
        .expect("wall row");
    assert!(wall.contains(" 200 100 400 "), "{wall}");
    let lamp = page
        .poses
        .iter()
        .find(|row| row.starts_with("lamp_1 "))
        .expect("lamp row");
    assert!(lamp.contains(" 100 100 100 "), "{lamp}");
    let spatial = SceneSpatialTool::invoke(&server, SceneSpatialArgs::default()).expect("reads");
    assert_eq!(spatial.size_source.collider, 1);
    assert_eq!(spatial.size_source.default, 1);
}

/// A wide pad collider with four small nodes standing inside it, inserted in
/// the given order under fixed ids.
fn pad_server(order: &[usize]) -> McpServer {
    let pad = Component::Collider(Collider {
        shape: ColliderShape::Box,
        half: [2.0, 0.5, 2.0],
        sensor: false,
    });
    let spots: [(&str, [f32; 3], Option<Component>); 5] = [
        ("pad", [0.0, 0.0, 0.0], Some(pad)),
        ("crate", [-1.5, 0.0, -1.5], None),
        ("crate", [1.5, 0.0, -1.5], None),
        ("crate", [-1.5, 0.0, 1.5], None),
        ("crate", [1.5, 0.0, 1.5], None),
    ];
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let mut ops = Vec::new();
    for &i in order {
        let (asset, pos, extra) = &spots[i];
        let id = ObjectId::parse(&format!("00000000-0000-0000-0000-00000000000{}", i + 1))
            .expect("fixed id parses");
        ops.push(Op::Insert {
            id,
            parent: ObjectId::NIL,
            local: Transform::new(*pos, [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
        });
        ops.push(Op::SetComponent {
            id,
            component: mesh(asset),
        });
        if let Some(component) = extra {
            ops.push(Op::SetComponent {
                id,
                component: *component,
            });
        }
    }
    let batch = Batch {
        ops,
        base_revision: doc.revision,
        idempotency_key: 1,
        provenance: None,
    };
    commands::apply(&mut doc, &mut journal, &batch).expect("pad seeds");
    McpServer::with_document(doc, journal, 7, Capability::unrestricted(7))
}

fn spatial(server: &McpServer, limit: Option<u32>) -> SceneSpatialOutput {
    SceneSpatialTool::invoke(
        server,
        SceneSpatialArgs {
            limit,
            ..Default::default()
        },
    )
    .expect("spatial reads")
}

/// The actor standing under four crates leads the rollup with their count,
/// the counts add up, and the rollup does not depend on insertion order.
#[test]
fn spatial_rollup_leads_with_the_most_involved_actor() {
    let server = pad_server(&[0, 1, 2, 3, 4]);
    let report = spatial(&server, Some(2));
    let lead = &report.worst_actors[0];
    assert!(lead.id.starts_with("pad_1 ["), "{}", lead.id);
    assert!(lead.findings >= 4, "pad overlaps four crates: {lead:?}");
    assert!(lead.checks.contains(&"overlap".to_owned()), "{lead:?}");
    assert!(lead.red >= 4, "{lead:?}");
    for pair in report.worst_actors.windows(2) {
        assert!(pair[0].findings >= pair[1].findings, "sorted by findings");
    }
    let counts = report.counts;
    assert_eq!(counts.returned, 2, "the page holds two rows");
    assert_eq!(counts.total, counts.returned + counts.suppressed);
    assert!(counts.suppressed > 0, "rows were held back");
    assert_eq!(counts.total, report.total);

    // Same scene, other insertion order: same rollup bytes.
    let shuffled = spatial(&pad_server(&[4, 2, 0, 3, 1]), Some(2));
    assert_eq!(
        rmcp::serde_json::to_string(&(&report.worst_actors, &report.counts)).expect("json"),
        rmcp::serde_json::to_string(&(&shuffled.worst_actors, &shuffled.counts)).expect("json"),
    );
}

/// `format=count` drops the rows but keeps the counts and the rollup;
/// `format=ids` lists only the objects that carry a finding.
#[test]
fn spatial_rollup_compact_formats_keep_the_counts() {
    let server = pad_server(&[0, 1, 2, 3, 4]);
    let full = spatial(&server, None);
    let ask = |format: &str| {
        SceneSpatialTool::invoke(
            &server,
            SceneSpatialArgs {
                format: Some(format.to_owned()),
                ..Default::default()
            },
        )
        .expect("spatial reads")
    };
    let count = ask("count");
    assert!(count.findings.is_empty() && count.objects.is_empty());
    assert_eq!((count.hash, count.total), (full.hash, full.total));
    assert_eq!(count.worst_actors, full.worst_actors);
    let ids = ask("ids");
    assert!(ids.findings.is_empty());
    assert_eq!(ids.objects.len(), 5, "pad and four crates carry findings");
    let refused = SceneSpatialTool::invoke(
        &server,
        SceneSpatialArgs {
            format: Some("table".to_owned()),
            ..Default::default()
        },
    )
    .expect_err("unknown format refuses");
    assert_eq!(refused.code, "bad_format");
    let red = SceneSpatialTool::invoke(
        &server,
        SceneSpatialArgs {
            matched_only: Some(true),
            ..Default::default()
        },
    )
    .expect("spatial reads");
    assert!(red.findings.iter().all(|line| line.contains(" red ")));
}

/// SPF12 at the door: calls two to four carry `repeat_count`, the fifth
/// identical `scene.brief` is refused naming the next verb, and one changed
/// argument passes.
#[test]
fn read_repeat_door_refuses_the_fifth_identical_scene_read() {
    let server = pad_server(&[0, 1, 2, 3, 4]);
    for call in 1..=4u32 {
        let out = brief_page(&server, SCENE_BRIEF_DEFAULT_BUDGET, None);
        assert_eq!(out.repeat_count, if call == 1 { 0 } else { call });
    }
    let refused = SceneBriefTool::invoke(
        &server,
        SceneBriefArgs {
            budget_bytes: Some(SCENE_BRIEF_DEFAULT_BUDGET),
            ..Default::default()
        },
    )
    .expect_err("fifth identical read refuses");
    assert_eq!(refused.code, "read_repeat");
    assert!(
        refused.message.contains("scene.spatial"),
        "{}",
        refused.message
    );
    let changed = brief_page(&server, 2048, None);
    assert_eq!(changed.repeat_count, 0, "a changed argument starts over");
}

/// `include_spatial` carries the same verdict, total and hash a separate
/// `scene.spatial` call reports.
#[test]
fn read_repeat_include_spatial_counts_equal_a_spatial_call() {
    let server = pad_server(&[0, 1, 2, 3, 4]);
    let brief = SceneBriefTool::invoke(
        &server,
        SceneBriefArgs {
            include_spatial: Some(true),
            ..Default::default()
        },
    )
    .expect("brief reads");
    let spatial = spatial(&server, None);
    let summary = brief.spatial.expect("include_spatial adds the summary");
    assert_eq!(summary.verdict, spatial.verdict);
    assert_eq!(summary.total, spatial.total);
    assert_eq!(summary.hash, spatial.hash);
    let plain = brief_page(&server, SCENE_BRIEF_DEFAULT_BUDGET, None);
    assert!(plain.spatial.is_none(), "off unless asked");
}
