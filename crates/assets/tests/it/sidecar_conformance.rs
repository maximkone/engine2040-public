//! sidecar conformance: the shipped fighter sidecars parse through the engine
//! parser, meet what CB4 (hit windows), LM2 (contacts) and CT24 (traversal)
//! read, and `content/fighters/sidecars/SCHEMA.md` names every field and limit
//! the parser enforces.
#![cfg(feature = "import")]

use engine_assets::{
    parse_sidecar, ClipTags, RetargetError, MAX_HIT_WINDOWS, MAX_JSON_DEPTH, MAX_SIDECAR_BYTES,
    MAX_TAGS, MAX_TAG_CHARS, MAX_TRAVERSAL, TRAVERSAL_EDGES,
};
use std::fs;
use std::path::PathBuf;

fn sidecar_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters/sidecars")
}

/// Every shipped sidecar as `(file stem, parsed tags)`, sorted by stem.
fn shipped() -> Vec<(String, ClipTags)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(sidecar_dir()).expect("sidecars dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_some_and(|ext| ext == "json") {
            let stem = path.file_stem().expect("stem").to_string_lossy().into_owned();
            let text = fs::read_to_string(&path).expect("read sidecar");
            let tags = parse_sidecar(&text).unwrap_or_else(|err| panic!("{stem}: {err:?}"));
            out.push((stem, tags));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn sidecar_shipped_files_parse_and_name_their_clip() {
    let all = shipped();
    assert_eq!(all.len(), 14, "one sidecar per canonical clip");
    for (stem, tags) in &all {
        assert_eq!(&tags.clip, stem, "clip field equals the file stem");
    }
}

#[test]
fn sidecar_attack_clips_carry_cb4_windows() {
    for (stem, tags) in shipped() {
        if tags.tags.iter().any(|tag| tag == "attack") {
            assert!(!tags.hit_windows.is_empty(), "{stem}: attack without a window");
        }
        for window in &tags.hit_windows {
            assert!(window.socket.starts_with("SOCKET_"), "{stem}: {}", window.socket);
        }
    }
}

#[test]
fn sidecar_every_clip_carries_lm2_contacts() {
    for (stem, tags) in shipped() {
        let feet = tags.feet.as_ref().unwrap_or_else(|| panic!("{stem}: no feet"));
        assert_eq!(feet, &("foot_l".to_string(), "foot_r".to_string()), "{stem}");
        assert!(tags.ground_y.is_some_and(|y| y > 0.0), "{stem}: ground_y");
        if tags.tags.iter().any(|tag| tag == "idle" || tag == "run") {
            assert!(tags.looped, "{stem}: idle and run tile");
        }
    }
}

#[test]
fn sidecar_traversal_schema_for_ct24_takes_the_closed_edge_list() {
    for edge in TRAVERSAL_EDGES {
        let doc = format!(r#"{{"clip":"x","traversal":[{{"edge":"{edge}","time_s":0.25}}]}}"#);
        let tags = parse_sidecar(&doc).expect("known edge parses");
        assert_eq!(tags.traversal.len(), 1);
        assert_eq!(tags.traversal[0].edge, edge);
    }
    for bad in [
        r#"{"traversal":[{"edge":"fly","time_s":0.0}]}"#,
        r#"{"traversal":[{"edge":"vault","time_s":-0.1}]}"#,
        r#"{"traversal":[{"edge":"vault"}]}"#,
    ] {
        assert!(matches!(parse_sidecar(bad), Err(RetargetError::BadSidecar { .. })), "{bad}");
    }
}

#[test]
fn sidecar_schema_page_names_every_field_and_limit() {
    let page = fs::read_to_string(sidecar_dir().join("SCHEMA.md")).expect("SCHEMA.md");
    for field in ["clip", "loop", "root_motion", "tags", "ground_y", "feet", "hit_windows", "traversal"] {
        assert!(page.contains(&format!("| `{field}` |")), "field {field}");
    }
    for edge in TRAVERSAL_EDGES {
        assert!(page.contains(&format!("`{edge}`")), "edge {edge}");
    }
    let limits: [(&str, usize); 6] = [
        ("MAX_SIDECAR_BYTES", MAX_SIDECAR_BYTES),
        ("MAX_TAGS", MAX_TAGS),
        ("MAX_TAG_CHARS", MAX_TAG_CHARS),
        ("MAX_HIT_WINDOWS", MAX_HIT_WINDOWS),
        ("MAX_TRAVERSAL", MAX_TRAVERSAL),
        ("MAX_JSON_DEPTH", MAX_JSON_DEPTH as usize),
    ];
    for (name, value) in limits {
        assert!(page.contains(&format!("| `{name}` | {value} |")), "limit {name} = {value}");
    }
}
