//! SPR8 tests: compact `scene.query` answers keep the digest and the matched
//! ids while the wire shrinks, and the ledger posts both sizes.

use super::*;
use crate::watch_ledger::WatchLedger;
use engine_editor::capability::Capability;
use engine_editor::commands::{self, Batch, Document, Journal, Op};
use engine_scene::transform::Transform;

/// `count` unit nodes on a 3 m grid, the first one at the origin.
fn grid_server(count: usize) -> McpServer {
    let mut doc = Document::new();
    let mut journal = Journal::new();
    let ops: Vec<Op> = (0..count)
        .map(|i| Op::Insert {
            id: engine_scene::ids::ObjectId::new(),
            parent: engine_scene::ids::ObjectId::NIL,
            local: Transform::new(
                [(i % 10) as f32 * 3.0, 0.5, (i / 10) as f32 * 3.0 - 9.0],
                [0.0, 0.0, 0.0, 1.0],
                [1.0; 3],
            ),
        })
        .collect();
    let batch = Batch {
        ops,
        base_revision: doc.revision,
        idempotency_key: 1,
        provenance: None,
    };
    commands::apply(&mut doc, &mut journal, &batch).expect("grid seeds");
    McpServer::with_document(doc, journal, 7, Capability::unrestricted(7))
}

fn visible(format: Option<&str>, matched_only: Option<bool>) -> SceneQueryArgs {
    SceneQueryArgs {
        kind: "visible".to_owned(),
        eye_m: Some([-2.0, 0.5, 0.0]),
        yaw: Some(0.0),
        format: format.map(str::to_owned),
        matched_only,
        ..Default::default()
    }
}

fn wire(output: &SceneQueryOutput) -> usize {
    rmcp::serde_json::to_string(output)
        .expect("output serializes")
        .len()
}

/// The same 64-hit `visible` answer, asked three ways: identical digest,
/// count and matched ids, and the compact forms cost at most 40 percent.
#[test]
fn query_compact_visible_shrinks_with_the_same_digest_and_ids() {
    let server = grid_server(70);
    let full = SceneQueryTool::invoke(&server, visible(None, Some(false))).expect("full rows");
    assert_eq!(full.hits.len(), 64, "the full answer holds the 64-hit cap");
    let matched: Vec<String> = full
        .hits
        .iter()
        .filter(|hit| hit.matched)
        .map(|hit| hit.object.clone())
        .collect();
    assert!(!matched.is_empty(), "something is in sight");
    assert!(matched.len() < 64, "something is blocked");

    let default = SceneQueryTool::invoke(&server, visible(None, None)).expect("default rows");
    assert!(
        default.hits.iter().all(|hit| hit.matched),
        "visible drops blocked rows"
    );
    let ids = SceneQueryTool::invoke(&server, visible(Some("ids"), None)).expect("ids");
    let count = SceneQueryTool::invoke(&server, visible(Some("count"), None)).expect("count");

    for other in [&default, &ids, &count] {
        assert_eq!(other.digest, full.digest, "digest does not move");
        assert_eq!(other.count as usize, matched.len());
    }
    assert_eq!(ids.ids, matched, "ids are the matched labels");
    assert!(ids.hits.is_empty() && count.hits.is_empty() && count.ids.is_empty());
    let (before, after) = (wire(&full), wire(&ids));
    eprintln!(
        "query_compact: visible 64 hits {before} chars full, {after} ids, {} default",
        wire(&default)
    );
    assert!(after * 10 <= before * 4, "ids {after} vs full {before}");
    assert!(
        wire(&default) * 10 <= before * 4,
        "default {} vs full {before}",
        wire(&default)
    );
}

/// An unknown format refuses with the legal words.
#[test]
fn query_compact_bad_format_names_the_legal_words() {
    let server = grid_server(3);
    let refused = SceneQueryTool::invoke(&server, visible(Some("table"), None))
        .expect_err("unknown format refuses");
    assert_eq!(refused.code, "bad_format");
    assert!(
        refused.message.contains("rows, ids or count"),
        "{}",
        refused.message
    );
}

/// The ledger keeps the full answer chars and the wire chars apart and
/// prints both columns.
#[test]
fn query_compact_ledger_prints_chars_before_and_after() {
    let actors: Vec<Actor> = (0..4)
        .map(|i| Actor {
            id: i,
            name: format!("crate_{i}"),
            x: i as f32 * 2.0,
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
        })
        .collect();
    let args = QueryArgs::near(0, 800);
    let answer = run_query(&actors, &ReachView::default(), &args).expect("near answers");
    let mut ledger = WatchLedger::new();
    ledger.record_query(QueryKind::Near, 40, &answer, answer.bytes / 4);
    let row = ledger.verb_row(QueryKind::Near);
    assert_eq!(row.answer_chars, answer.bytes as u64);
    assert_eq!(row.wire_chars, (answer.bytes / 4) as u64);
    let table = ledger.token_table_text();
    assert!(
        table.starts_with("verb calls req_chars ans_chars wire_chars "),
        "{table}"
    );
    assert!(
        table.contains(&format!("near 1 40 {} {} ", answer.bytes, answer.bytes / 4)),
        "{table}"
    );
}
