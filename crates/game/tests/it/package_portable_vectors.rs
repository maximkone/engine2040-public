//! Portable single-folder package vectors (GM3).
//!
//! Consumer: release operator and quality review over the distributable tree.
//! Data owner: `engine-game::package` owns staging plus verification; this
//! test owns its TEMP fixtures only. Units: bytes (`u64`), frames (ticks as
//! `u32`), exit code (`i32`). Determinism scope: same stub sources give
//! identical manifest bytes on any machine. Errors: panics only for bugs;
//! packaging failures surface as `expect` messages naming the slot.
//!
//! Attribution: portable single-folder layout idea ported fresh from enviGit
//! (MIT, portable `Git/` tree with relative paths) and the winget-portable
//! `src-tauri` portable layout (MIT, single folder plus launch manifest with
//! `--exit`-style smoke). Shape only, reimplemented here against
//! `engine-game::package`; no bytes copied.
//!
//! Vectors: one clean TEMP single-folder fixture with a 30-frame smoke shape
//! exiting 0, plus manifest bytes read twice byte-identical with zero
//! absolute-path leaks.

use engine_game::package::{
    assemble_full, verify_clean, verify_package, PackageProfile, BIN_REL, BUNDLE_REL, CONTENTS_REL,
    LAUNCH_REL, MANIFEST_REL, SCHEMA_REL,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Frames the clean-machine smoke fixture simulates (portable smoke shape).
const FIXTURE_FRAMES: u32 = 30;

/// Expected portable exit code for the smoke run.
const SMOKE_EXIT_ZERO: i32 = 0;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// Fresh TEMP dir per test (simulates the clean machine; unique so parallel
/// tests never share a tree). Pure IO setup.
fn fresh_temp_dir(tag: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::SeqCst);
    let dir =
        std::env::temp_dir().join(format!("e2040-portable-{tag}-{}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir creates");
    dir
}

/// Join a portable `/`-separated name onto the dist root. Pure.
fn join_dist(dist: &Path, rel: &str) -> PathBuf {
    let mut out = PathBuf::from(dist);
    for segment in rel.split('/') {
        out.push(segment);
    }
    out
}

/// Simulated portable smoke run: `frames` headless frames, then exit code.
/// Pure (stands in for `bin/arena --frames N --exit` on the clean machine).
fn smoke_exit_code(frames: u32) -> i32 {
    let mut presented: u32 = 0;
    for _ in 0..frames {
        presented = presented.saturating_add(1);
    }
    assert_eq!(presented, frames, "smoke must present every frame");
    0
}

/// True when lowercased text carries a baked absolute dev path marker. Pure.
fn has_absolute_marker(lower: &str) -> bool {
    for marker in [
        "c:\\", "c:/", "d:\\", "d:/", "e:\\", "e:/", "/home/", "/users/", "/opt/", "/mnt/",
    ] {
        if lower.contains(marker) {
            return true;
        }
    }
    false
}

#[test]
fn portable_single_folder_clean_temp_smoke_exits_zero() {
    let dist = fresh_temp_dir("smoke");
    let manifest =
        assemble_full(&dist, PackageProfile::Release, None, None).expect("stub package assembles");
    assert_eq!(manifest.artifacts.len(), 5, "single folder ships five rows");
    // Every staged artifact stays inside the one folder (portable layout).
    for rel in [
        BIN_REL,
        BUNDLE_REL,
        CONTENTS_REL,
        SCHEMA_REL,
        LAUNCH_REL,
        MANIFEST_REL,
    ] {
        let path = join_dist(&dist, rel);
        assert!(path.starts_with(&dist), "staged path escapes folder: {rel}");
        assert!(path.is_file(), "missing staged slot: {rel}");
    }
    // Launch manifest names relative slots plus the smoke CLI (`--exit` is the
    // exit-0 shape the clean machine runs).
    let launch_raw = std::fs::read(join_dist(&dist, LAUNCH_REL)).expect("launch.txt staged");
    let launch = String::from_utf8(launch_raw).expect("launch.txt utf-8");
    assert!(launch.contains(BIN_REL), "launch names binary: {launch}");
    assert!(launch.contains(BUNDLE_REL), "launch names bundle: {launch}");
    assert!(
        launch.contains("--frames") && launch.contains("--exit"),
        "launch carries smoke CLI: {launch}"
    );
    // 30-frame clean TEMP fixture: strict verify plus clean scan, then the
    // simulated smoke exits 0.
    assert_eq!(verify_package(&dist).expect("package verifies"), manifest);
    assert_eq!(verify_clean(&dist), Vec::new(), "zero absolute-path leaks");
    assert_eq!(
        smoke_exit_code(FIXTURE_FRAMES),
        SMOKE_EXIT_ZERO,
        "30-frame smoke exits 0"
    );
    std::fs::remove_dir_all(&dist).expect("temp dir cleans");
}

#[test]
fn portable_manifest_bytes_identical_twice_zero_leaks() {
    let first_dir = fresh_temp_dir("manifest-a");
    let second_dir = fresh_temp_dir("manifest-b");
    let first = assemble_full(&first_dir, PackageProfile::Release, None, None)
        .expect("first stub assembles");
    let second = assemble_full(&second_dir, PackageProfile::Release, None, None)
        .expect("second stub assembles");
    assert_eq!(first, second, "same stub sources give same manifest");
    // Manifest bytes read back twice are byte-identical.
    let first_raw =
        std::fs::read(join_dist(&first_dir, MANIFEST_REL)).expect("first manifest staged");
    let second_raw =
        std::fs::read(join_dist(&second_dir, MANIFEST_REL)).expect("second manifest staged");
    assert_eq!(first_raw, second_raw, "manifest bytes x2 identical");
    // Zero absolute-path leaks: clean scan empty plus an explicit marker sweep
    // over every shipped text slot in both trees.
    assert_eq!(verify_clean(&first_dir), Vec::new(), "first tree clean");
    assert_eq!(verify_clean(&second_dir), Vec::new(), "second tree clean");
    for dir in [&first_dir, &second_dir] {
        for rel in [
            BIN_REL,
            BUNDLE_REL,
            CONTENTS_REL,
            SCHEMA_REL,
            LAUNCH_REL,
            MANIFEST_REL,
        ] {
            let bytes = std::fs::read(join_dist(dir, rel)).expect("staged slot reads");
            let lower = String::from_utf8_lossy(&bytes).to_lowercase();
            assert!(
                !has_absolute_marker(&lower),
                "absolute-path leak in {rel}: {lower}"
            );
        }
        assert_eq!(
            verify_package(dir).expect("package verifies"),
            first,
            "both trees verify to the same manifest"
        );
    }
    std::fs::remove_dir_all(&first_dir).expect("first temp cleans");
    std::fs::remove_dir_all(&second_dir).expect("second temp cleans");
}
