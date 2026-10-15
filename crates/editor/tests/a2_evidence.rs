//! Evidence run: one recorded editable-scene session over licensed
//! asset bytes, writing the lane evidence dir with a byte-compare receipt.

use engine_editor::a2_session::{lane_content_root, lane_evidence_dir, run_session};

/// Import licensed bytes, place three objects, edit one component plus one
/// material, save, close, reopen; the reopen is byte-identical to the save.
#[test]
fn a2_session_saves_and_reopens_byte_identical() {
    let content = lane_content_root();
    assert!(
        content.join("fighters/glb/hero_body.glb").exists(),
        "lane CT7 bytes missing at {}",
        content.display()
    );
    let out = lane_evidence_dir();
    let report = run_session(&content, &out, None).expect("A2 session completes");
    assert!(report.bytes_match, "reopen truncated the scene");
    assert_eq!(report.nodes, 3, "three objects placed");
    assert_eq!(
        report.revision, 5,
        "three inserts plus component plus material edits"
    );
    assert_eq!(report.refusals.len(), 3, "three refusals exercised");

    let saved = std::fs::read(out.join("scene.edoc")).expect("scene file written");
    let reopened = std::fs::read(out.join("reopened.edoc")).expect("reopened file written");
    assert_eq!(saved, reopened, "reopened bytes match saved bytes");
    assert!(!saved.is_empty(), "scene file holds nodes");
    // The edited material persists in the scene bytes themselves.
    let needle = b"hero-mat";
    assert!(
        saved.windows(needle.len()).any(|w| w == needle),
        "scene bytes carry the bound material"
    );

    for file in ["report.txt", "rows.txt", "journal.txt"] {
        assert!(out.join(file).exists(), "evidence {file} written");
    }
    let text = std::fs::read_to_string(out.join("report.txt")).expect("report written");
    assert!(
        text.contains("byte-identical: true"),
        "receipt claims equality"
    );
    assert!(text.contains("CC0-1.0"), "licenses named");
}
