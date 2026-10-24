//! The committed arena kit cooks the same twice. The real `arena_cook` bin cooks the CC0 texture
//! sources into two fresh roots; both runs and the committed `content/arena/cooked/` must agree
//! byte for byte (24 KTX2 maps and the receipt). The 8 kit meshes cook through the CT2 prop lane
//! twice to equal output under the prop budget.
#![cfg(feature = "import")]

use engine_assets::{cook_gltf_bytes, MeshClass, PROP_MAX_TRIS};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn arena() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/arena")
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create dir");
    for entry in fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("dir entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

/// Cook the committed sources into a fresh root and return every cooked file by name.
fn cook(tag: &str) -> BTreeMap<String, Vec<u8>> {
    let root = std::env::temp_dir().join(format!("arena_cook_x2_{}_{tag}", std::process::id()));
    copy_tree(&arena().join("sources"), &root.join("sources"));
    let out = Command::new(env!("CARGO_BIN_EXE_arena_cook"))
        .arg("--root")
        .arg(&root)
        .output()
        .expect("run arena_cook");
    assert!(
        out.status.success(),
        "arena_cook failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let cooked = read_set(&root.join("cooked"));
    fs::remove_dir_all(&root).expect("remove temp root");
    cooked
}

fn read_set(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(dir)
        .expect("read cooked dir")
        .map(|entry| {
            let path = entry.expect("dir entry").path();
            let name = path.file_name().expect("file name").to_string_lossy();
            (
                name.into_owned(),
                fs::read(&path).expect("read cooked file"),
            )
        })
        .collect()
}

#[test]
fn arena_cook_committed_set_is_identical_twice_and_matches_the_tree() {
    let first = cook("a");
    let second = cook("b");
    assert_eq!(first.len(), 25, "24 KTX2 maps and cook.json");
    assert!(first == second, "two cooks of the same sources differ");
    let committed = read_set(&arena().join("cooked"));
    assert_eq!(
        first.keys().collect::<Vec<_>>(),
        committed.keys().collect::<Vec<_>>()
    );
    // The receipt is text: compare it without line-ending noise from a CRLF checkout.
    for (name, bytes) in &first {
        let want = &committed[name];
        if name.ends_with(".json") {
            let text = |b: &[u8]| String::from_utf8_lossy(b).replace("\r\n", "\n");
            assert_eq!(
                text(bytes),
                text(want),
                "{name} differs from the committed file"
            );
        } else {
            assert!(bytes == want, "{name} differs from the committed file");
        }
    }
}

#[test]
fn travertine_and_marble_cook_twice_equal() {
    // SW122: the marble sets landed as sources; the cook must be reproducible
    // before their KTX2 maps are committed beside the older five.
    let first = cook("sw122a");
    let second = cook("sw122b");
    for set in ["marble", "marble_tiles"] {
        for suffix in ["base", "normal", "orm"] {
            let name = format!("{set}_{suffix}.ktx2");
            assert_eq!(
                first[&name], second[&name],
                "{name} cooks equal twice (SW122)"
            );
            assert!(!first[&name].is_empty(), "{name} has bytes");
        }
    }
}

#[test]
fn dense_sand_cooks_twice_equal() {
    // SW124: Poly Haven naming plus the packed arm image. Two cooks must agree,
    // and the arm's metal channel must have reached the ORM's blue channel.
    let first = cook("sw124a");
    let second = cook("sw124b");
    for suffix in ["base", "normal", "orm"] {
        let name = format!("sand_dense_{suffix}.ktx2");
        assert_eq!(
            first[&name], second[&name],
            "{name} cooks equal twice (SW124)"
        );
    }
    let orm = &first["sand_dense_orm.ktx2"];
    assert!(!orm.is_empty(), "the packed arm cooked an ORM map");
}

#[test]
fn arena_kit_meshes_cook_identical_twice_within_the_prop_budget() {
    let mut files: Vec<PathBuf> = fs::read_dir(arena().join("src"))
        .expect("kit src dir")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "gltf"))
        .collect();
    files.sort();
    assert_eq!(files.len(), 8, "the 8 kit meshes");
    for path in files {
        let name = path.file_name().expect("file name").to_string_lossy();
        let bytes = fs::read(&path).expect("kit mesh");
        let first =
            cook_gltf_bytes(&bytes, MeshClass::Prop).unwrap_or_else(|e| panic!("{name}: {e}"));
        let second = cook_gltf_bytes(&bytes, MeshClass::Prop).expect("recook");
        assert!(first == second, "{name}: cook x2 differs");
        assert!(
            first.meshes.iter().all(|m| m.base_tris <= PROP_MAX_TRIS),
            "{name}: over the prop budget"
        );
    }
}
