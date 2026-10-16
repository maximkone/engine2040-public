// Donor: o3de/o3de `screenshot_utils.py::compare_golden_image`, Apache-2.0.
// Link: https://github.com/o3de/o3de
// Pattern only: mean-similarity compare of capture bytes against a golden
// with threshold assert; fresh pure-Rust write, no donor code copied.
// Donor: mitsuhiko/insta@5df39ceafc8a47f82ec9953aaba12072d316adc8
// (`insta/src/env.rs`, `insta/src/runtime.rs`), Apache-2.0, read live 2026-10-09.
// PORT: file-snapshot pending-update workflow (`SnapshotUpdate`,
// `snapshot_update_behavior`, pending `.pending` file beside the golden)
// reimplemented std-only for capture bytes with the mean-similarity
// threshold gate; no donor bytes copied.
// License: https://raw.githubusercontent.com/mitsuhiko/insta/5df39ceafc8a47f82ec9953aaba12072d316adc8/LICENSE

//! Golden-capture mean-similarity compare for hero shots.
//!
//! Compares raw capture bytes against golden bytes as a mean per-byte
//! similarity in `0.0..=1.0` and asserts against a threshold. The score is
//! always included in the failure message so a failing hero shot prints its
//! number.
//!
//! File goldens follow the donor pending-update workflow: a mismatch never
//! overwrites the golden unless the update mode says so; otherwise it writes
//! a `.pending` file beside the golden and fails, so review stays explicit.

/// Mean per-byte similarity between two buffers.
///
/// Each byte pair scores `1.0 - |a - b| / 255.0`; the result is the mean.
/// Returns `1.0` when both buffers are empty, `0.0` when lengths differ or
/// exactly one side is empty.
pub fn mean_similarity(current: &[u8], golden: &[u8]) -> f64 {
    if current.len() != golden.len() {
        return 0.0;
    }
    if current.is_empty() {
        return 1.0;
    }
    let mut sum = 0.0_f64;
    for (c, g) in current.iter().zip(golden.iter()) {
        let diff = (*c as i16 - *g as i16).unsigned_abs() as f64;
        sum += 1.0 - diff / 255.0;
    }
    sum / current.len() as f64
}

/// Assert `mean_similarity` meets `threshold`, printing the score on failure.
pub fn assert_similarity(current: &[u8], golden: &[u8], threshold: f64) {
    let score = mean_similarity(current, golden);
    assert!(
        score >= threshold,
        "golden capture similarity {score:.6} below threshold {threshold:.6}"
    );
}

/// Sequential golden file stem helper: `format!("{stem}_{index:04}.png")`.
pub fn golden_name(stem: &str, index: u32) -> String {
    format!("{stem}_{index:04}.png")
}

use std::path::{Path, PathBuf};

/// How file goldens update on mismatch, ported from the donor `SnapshotUpdate`.
///
/// `Auto` writes a pending file off CI and fails on CI; `Always` and `Force`
/// overwrite the golden in place; `New` only records new goldens as pending;
/// `No` never writes; `Unseen` records only missing goldens as pending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileUpdate {
    /// Pending file off CI, fail on CI.
    Auto,
    /// Overwrite the golden in place.
    Always,
    /// Record only missing goldens as pending.
    New,
    /// Never write, only fail.
    No,
    /// Record missing goldens as pending, update present ones in place.
    Unseen,
    /// Overwrite the golden in place, like `Always`.
    Force,
}

/// Where an update lands, ported from the donor `SnapshotUpdateBehavior`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileUpdateBehavior {
    /// Overwrite the golden file.
    InPlace,
    /// Write a `.pending` file beside the golden.
    NewFile,
    /// Write nothing, only fail.
    NoUpdate,
}

/// Parse one update word, matching the donor `INSTA_UPDATE` values.
///
/// Accepts `auto`, `always` (plus legacy `1`), `new`, `no`, `unseen` and
/// `force`. Panics on anything else so a typo fails loud.
pub fn parse_file_update(word: &str) -> FileUpdate {
    match word {
        "auto" => FileUpdate::Auto,
        "always" | "1" => FileUpdate::Always,
        "new" => FileUpdate::New,
        "no" => FileUpdate::No,
        "unseen" => FileUpdate::Unseen,
        "force" => FileUpdate::Force,
        _ => panic!("unknown capture update {word:?}: want auto|always|new|no|unseen|force"),
    }
}

/// Read the update mode from `CAPTURE_UPDATE`, falling back to `INSTA_UPDATE`.
///
/// Empty or unset means `Auto`. Units: env text; determinism: same env gives
/// the same mode.
pub fn file_update_from_env() -> FileUpdate {
    let word = std::env::var("CAPTURE_UPDATE")
        .or_else(|_| std::env::var("INSTA_UPDATE"))
        .unwrap_or_default();
    if word.is_empty() {
        return FileUpdate::Auto;
    }
    parse_file_update(word.trim())
}

/// True on CI, where `Auto` must fail instead of writing pending files.
pub fn is_ci() -> bool {
    for key in ["CI", "GITHUB_ACTIONS", "TF_BUILD"] {
        if let Ok(value) = std::env::var(key) {
            let lower = value.to_ascii_lowercase();
            if lower == "1" || lower == "true" {
                return true;
            }
        }
    }
    false
}

/// Map an update mode plus newness to a landing place.
///
/// Mirrors the donor `snapshot_update_behavior`: `Always`/`Force` land in
/// place, `Auto` lands as a pending file off CI and nowhere on CI, `Unseen`
/// lands new goldens as pending and present ones in place, `New` always lands
/// as pending, `No` never lands.
pub fn file_update_behavior(update: FileUpdate, is_new: bool) -> FileUpdateBehavior {
    file_update_behavior_with_ci(update, is_new, is_ci())
}

/// Pure form of [`file_update_behavior`] for tests: caller passes CI-ness.
pub fn file_update_behavior_with_ci(
    update: FileUpdate,
    is_new: bool,
    on_ci: bool,
) -> FileUpdateBehavior {
    match update {
        FileUpdate::Always | FileUpdate::Force => FileUpdateBehavior::InPlace,
        FileUpdate::Auto => {
            if on_ci {
                FileUpdateBehavior::NoUpdate
            } else {
                FileUpdateBehavior::NewFile
            }
        }
        FileUpdate::Unseen => {
            if is_new {
                FileUpdateBehavior::NewFile
            } else {
                FileUpdateBehavior::InPlace
            }
        }
        FileUpdate::New => FileUpdateBehavior::NewFile,
        FileUpdate::No => FileUpdateBehavior::NoUpdate,
    }
}

/// Pending path beside a golden: `<golden>.pending`.
///
/// Keeps the golden untouched until review; the reviewer renames the pending
/// file over the golden to accept. Units: file paths; determinism: pure.
pub fn pending_path(golden: &Path) -> PathBuf {
    let mut text = golden.as_os_str().to_owned();
    text.push(".pending");
    PathBuf::from(text)
}

/// Check capture bytes against a file golden with the threshold gate.
///
/// Reads the golden file, scores with [`mean_similarity`], and enforces
/// `threshold`. On a below-threshold mismatch the landing follows
/// [`file_update_behavior`]: `InPlace` refreshes the golden but still fails
/// so the gate stays loud, `NewFile` writes the pending file and fails,
/// `NoUpdate` only fails. A passing check removes any stale pending file.
/// Returns the score. Panics with the score in the message on failure.
pub fn check_file_golden(current: &[u8], golden_path: &Path, threshold: f64) -> f64 {
    check_file_golden_with_update(current, golden_path, threshold, file_update_from_env())
}

/// Deterministic form of [`check_file_golden`] for tests: caller passes mode.
pub fn check_file_golden_with_update(
    current: &[u8],
    golden_path: &Path,
    threshold: f64,
    update: FileUpdate,
) -> f64 {
    assert!(
        threshold.is_finite() && (0.0..=1.0).contains(&threshold),
        "threshold {threshold:?} must sit in 0.0..=1.0"
    );
    let is_new = !golden_path.exists();
    if is_new {
        match file_update_behavior(update, true) {
            FileUpdateBehavior::InPlace => {
                if let Some(parent) = golden_path.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent).expect("golden parent dir creates");
                    }
                }
                std::fs::write(golden_path, current).expect("new golden writes");
                let _ = std::fs::remove_file(pending_path(golden_path));
                return 1.0;
            }
            FileUpdateBehavior::NewFile => {
                let pending = pending_path(golden_path);
                if let Some(parent) = pending.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent).expect("pending parent dir creates");
                    }
                }
                std::fs::write(&pending, current).expect("pending file writes");
                panic!(
                    "new file golden missing {}: wrote pending {} for review",
                    golden_path.display(),
                    pending.display()
                );
            }
            FileUpdateBehavior::NoUpdate => {
                panic!(
                    "new file golden missing {}: set CAPTURE_UPDATE=new to record it",
                    golden_path.display()
                );
            }
        }
    }
    let golden = std::fs::read(golden_path).expect("golden file reads");
    let score = mean_similarity(current, &golden);
    if score >= threshold {
        let _ = std::fs::remove_file(pending_path(golden_path));
        return score;
    }
    match file_update_behavior(update, false) {
        FileUpdateBehavior::InPlace => {
            std::fs::write(golden_path, current).expect("golden refresh writes");
            let _ = std::fs::remove_file(pending_path(golden_path));
            panic!(
                "golden capture similarity {score:.6} below threshold {threshold:.6} for {} (refreshed in place, still failing)",
                golden_path.display()
            );
        }
        FileUpdateBehavior::NewFile => {
            let pending = pending_path(golden_path);
            std::fs::write(&pending, current).expect("pending file writes");
            panic!(
                "golden capture similarity {score:.6} below threshold {threshold:.6} for {}: wrote pending {}",
                golden_path.display(),
                pending.display()
            );
        }
        FileUpdateBehavior::NoUpdate => {
            panic!(
                "golden capture similarity {score:.6} below threshold {threshold:.6} for {}",
                golden_path.display(),
            );
        }
    }
}

#[test]
fn identical_buffers_score_one() {
    let golden = vec![10_u8, 20, 30, 200, 255, 0];
    let current = golden.clone();
    let score = mean_similarity(&current, &golden);
    println!("passed identical similarity {score:.6}");
    assert!((score - 1.0).abs() < f64::EPSILON, "score was {score}");
    assert_similarity(&current, &golden, 1.0);
}

#[test]
fn one_pixel_change_scores_between_and_fails_full_threshold() {
    let golden = vec![0_u8; 64];
    let mut current = golden.clone();
    current[7] = 255;
    let score = mean_similarity(&current, &golden);
    println!("passed one-pixel similarity {score:.6}");
    assert!(
        score > 0.0 && score < 1.0,
        "expected score in (0, 1), got {score}"
    );
    let threshold = 1.0;
    let result = std::panic::catch_unwind(|| assert_similarity(&current, &golden, threshold));
    assert!(result.is_err(), "expected threshold {threshold} to fail");
    let message = match result {
        Err(payload) => {
            if let Some(text) = payload.downcast_ref::<String>() {
                text.clone()
            } else if let Some(text) = payload.downcast_ref::<&str>() {
                (*text).to_owned()
            } else {
                String::new()
            }
        }
        Ok(()) => String::new(),
    };
    let expected = format!("{score:.6}");
    assert!(
        message.contains(&expected),
        "fail message {message:?} missing score {expected}"
    );
    println!("passed threshold failure printed score {score:.6}");
}

#[test]
fn empty_vs_full_scores_zero() {
    let golden: Vec<u8> = Vec::new();
    let current = vec![255_u8; 16];
    let score = mean_similarity(&current, &golden);
    println!("passed empty-vs-full similarity {score:.6}");
    assert_eq!(score, 0.0, "score was {score}");
    assert_eq!(golden_name("hero", 1), "hero_0001.png");
}

/// Unique temp golden path for one file test: no two tests share a file.
fn temp_golden(tag: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "engine2040-capture-{}-{tag}.bin",
        std::process::id()
    ));
    path
}

/// Remove a golden plus its pending sidecar; best-effort for test isolation.
fn clean_golden(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(pending_path(path));
}

/// Extract the panic text from `catch_unwind`, for score-in-message checks.
fn panic_text(result: Result<f64, Box<dyn std::any::Any + Send>>) -> String {
    match result {
        Err(payload) => {
            if let Some(text) = payload.downcast_ref::<String>() {
                text.clone()
            } else if let Some(text) = payload.downcast_ref::<&str>() {
                (*text).to_owned()
            } else {
                String::new()
            }
        }
        Ok(_) => String::new(),
    }
}

/// Deterministic capture bytes: rotating gradient from a seed byte.
fn gradient_bytes(len: usize, seed: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let at = u32::from(seed).wrapping_add(i as u32) % 256;
        out.push(at as u8);
    }
    out
}

#[test]
fn file_identical_bytes_pass_and_clean_pending() {
    let path = temp_golden("identical");
    clean_golden(&path);
    let golden = gradient_bytes(64, 7);
    std::fs::write(&path, &golden).expect("golden writes");
    std::fs::write(pending_path(&path), b"stale").expect("stale pending writes");
    let score = check_file_golden_with_update(&golden, &path, 1.0, FileUpdate::No);
    println!("passed file identical score {score:.6}");
    assert!((score - 1.0).abs() < f64::EPSILON, "score was {score}");
    assert!(!pending_path(&path).exists(), "stale pending must go");
    clean_golden(&path);
}

#[test]
fn file_one_pixel_mismatch_writes_pending_and_reports_score() {
    let path = temp_golden("one-pixel");
    clean_golden(&path);
    let golden = vec![0_u8; 64];
    let mut current = golden.clone();
    current[7] = 255;
    std::fs::write(&path, &golden).expect("golden writes");
    let expected = mean_similarity(&current, &golden);
    let result = std::panic::catch_unwind(|| {
        check_file_golden_with_update(&current, &path, 1.0, FileUpdate::New)
    });
    assert!(result.is_err(), "threshold 1.0 must fail");
    let message = panic_text(result);
    assert!(
        message.contains(&format!("{expected:.6}")),
        "message {message:?} missing score"
    );
    assert!(message.contains(".pending"), "message must name pending");
    let pending = std::fs::read(pending_path(&path)).expect("pending reads");
    assert_eq!(pending, current, "pending must hold current bytes");
    let golden_now = std::fs::read(&path).expect("golden reads");
    assert_eq!(golden_now, golden, "golden stays untouched on NewFile");
    println!("passed file one-pixel pending score {expected:.6}");
    clean_golden(&path);
}

#[test]
fn file_missing_golden_writes_pending_and_fails() {
    let path = temp_golden("missing-new");
    clean_golden(&path);
    let current = gradient_bytes(32, 11);
    let result = std::panic::catch_unwind(|| {
        check_file_golden_with_update(&current, &path, 1.0, FileUpdate::New)
    });
    assert!(result.is_err(), "missing golden must fail");
    let message = panic_text(result);
    assert!(
        message.contains("missing"),
        "message {message:?} must say missing"
    );
    assert!(!path.exists(), "golden must not appear on NewFile");
    let pending = std::fs::read(pending_path(&path)).expect("pending reads");
    assert_eq!(pending, current, "pending must hold current bytes");
    println!("passed file missing writes pending");
    clean_golden(&path);
}

#[test]
fn file_missing_golden_in_place_creates_and_passes() {
    let path = temp_golden("missing-always");
    clean_golden(&path);
    let current = gradient_bytes(32, 13);
    let score = check_file_golden_with_update(&current, &path, 1.0, FileUpdate::Always);
    assert!(
        (score - 1.0).abs() < f64::EPSILON,
        "new in-place must score 1.0"
    );
    let stored = std::fs::read(&path).expect("golden reads");
    assert_eq!(stored, current, "in-place must create the golden");
    assert!(
        !pending_path(&path).exists(),
        "no pending on in-place create"
    );
    let again = check_file_golden_with_update(&current, &path, 1.0, FileUpdate::No);
    assert!((again - 1.0).abs() < f64::EPSILON, "second check must pass");
    println!("passed file missing in-place creates golden");
    clean_golden(&path);
}

#[test]
fn file_mismatch_in_place_refreshes_but_gate_still_fails() {
    let path = temp_golden("mismatch-always");
    clean_golden(&path);
    let golden = vec![10_u8; 48];
    let current = vec![200_u8; 48];
    std::fs::write(&path, &golden).expect("golden writes");
    let expected = mean_similarity(&current, &golden);
    assert!(expected < 1.0, "fixture must mismatch");
    let result = std::panic::catch_unwind(|| {
        check_file_golden_with_update(&current, &path, 1.0, FileUpdate::Always)
    });
    assert!(result.is_err(), "gate must fail even when refreshing");
    let message = panic_text(result);
    assert!(
        message.contains(&format!("{expected:.6}")),
        "message {message:?} missing score"
    );
    let stored = std::fs::read(&path).expect("golden reads");
    assert_eq!(stored, current, "in-place must refresh the golden");
    assert!(!pending_path(&path).exists(), "no pending on in-place");
    println!("passed file in-place refresh keeps gate {expected:.6}");
    clean_golden(&path);
}

#[test]
fn file_mismatch_no_update_writes_nothing() {
    let path = temp_golden("mismatch-no");
    clean_golden(&path);
    let golden = gradient_bytes(48, 3);
    let mut current = golden.clone();
    current[5] = current[5].wrapping_add(77);
    std::fs::write(&path, &golden).expect("golden writes");
    let expected = mean_similarity(&current, &golden);
    let result = std::panic::catch_unwind(|| {
        check_file_golden_with_update(&current, &path, 1.0, FileUpdate::No)
    });
    assert!(result.is_err(), "gate must fail");
    let message = panic_text(result);
    assert!(
        message.contains(&format!("{expected:.6}")),
        "message {message:?} missing score"
    );
    assert!(!pending_path(&path).exists(), "NoUpdate must write nothing");
    let stored = std::fs::read(&path).expect("golden reads");
    assert_eq!(stored, golden, "golden stays untouched on NoUpdate");
    println!("passed file no-update writes nothing {expected:.6}");
    clean_golden(&path);
}

#[test]
fn file_unseen_new_pends_old_refreshes_in_place() {
    let fresh = temp_golden("unseen-new");
    let old = temp_golden("unseen-old");
    clean_golden(&fresh);
    clean_golden(&old);
    let current = gradient_bytes(24, 29);
    let missing = std::panic::catch_unwind(|| {
        check_file_golden_with_update(&current, &fresh, 1.0, FileUpdate::Unseen)
    });
    assert!(missing.is_err(), "unseen missing must pend");
    assert!(
        std::fs::read(pending_path(&fresh)).is_ok(),
        "pending must exist"
    );
    assert!(!fresh.exists(), "golden must not appear for unseen new");
    let golden = vec![1_u8; 24];
    let changed = vec![250_u8; 24];
    std::fs::write(&old, &golden).expect("golden writes");
    let mismatch = std::panic::catch_unwind(|| {
        check_file_golden_with_update(&changed, &old, 1.0, FileUpdate::Unseen)
    });
    assert!(mismatch.is_err(), "unseen mismatch still gates");
    let stored = std::fs::read(&old).expect("golden reads");
    assert_eq!(stored, changed, "unseen old refreshes in place");
    println!("passed file unseen branches");
    clean_golden(&fresh);
    clean_golden(&old);
}

#[test]
fn file_threshold_gate_holds_on_disk() {
    let path = temp_golden("threshold-disk");
    clean_golden(&path);
    let golden = gradient_bytes(64, 5);
    let mut near = golden.clone();
    near[0] = near[0].wrapping_add(2);
    near[1] = near[1].wrapping_add(1);
    std::fs::write(&path, &golden).expect("golden writes");
    let score = mean_similarity(&near, &golden);
    assert!(
        score < 1.0 && score > 0.99,
        "fixture must sit near 1.0, got {score}"
    );
    let loose = check_file_golden_with_update(&near, &path, 0.99, FileUpdate::No);
    assert!((loose - score).abs() < f64::EPSILON, "loose gate must pass");
    let tight = std::panic::catch_unwind(|| {
        check_file_golden_with_update(&near, &path, 1.0, FileUpdate::No)
    });
    assert!(tight.is_err(), "tight gate must fail");
    assert!(
        !pending_path(&path).exists(),
        "NoUpdate writes nothing either way"
    );
    println!("passed file threshold gate {score:.6}");
    clean_golden(&path);
}

#[test]
fn file_ten_sequential_stems_all_gated() {
    for index in 0..10_u32 {
        let stem = golden_name("hero", index);
        let mut path = std::env::temp_dir();
        path.push(format!(
            "engine2040-capture-{}-stem-{stem}",
            std::process::id()
        ));
        clean_golden(&path);
        let golden = gradient_bytes(40, index as u8);
        std::fs::write(&path, &golden).expect("golden writes");
        let score = check_file_golden_with_update(&golden, &path, 1.0, FileUpdate::No);
        assert!((score - 1.0).abs() < f64::EPSILON, "{stem} must gate 1.0");
        println!("passed file stem {stem} score {score:.6}");
        clean_golden(&path);
    }
}

#[test]
fn file_pending_path_is_stable_and_distinct() {
    let path = temp_golden("pending-path");
    clean_golden(&path);
    let pending = pending_path(&path);
    assert_ne!(pending, path, "pending must differ from golden");
    assert!(
        pending.to_string_lossy().ends_with(".pending"),
        "pending keeps suffix"
    );
    assert_eq!(pending_path(&path), pending, "pending path must repeat");
    let golden = gradient_bytes(16, 9);
    std::fs::write(&path, &golden).expect("golden writes");
    let score = check_file_golden_with_update(&golden, &path, 1.0, FileUpdate::Auto);
    assert!(
        (score - 1.0).abs() < f64::EPSILON,
        "auto pass must score 1.0"
    );
    println!("passed file pending path {}", pending.display());
    clean_golden(&path);
}

#[test]
fn update_behavior_matrix_matches_donor() {
    assert_eq!(
        file_update_behavior_with_ci(FileUpdate::Always, false, false),
        FileUpdateBehavior::InPlace
    );
    assert_eq!(
        file_update_behavior_with_ci(FileUpdate::Force, true, false),
        FileUpdateBehavior::InPlace
    );
    assert_eq!(
        file_update_behavior_with_ci(FileUpdate::Auto, false, false),
        FileUpdateBehavior::NewFile
    );
    assert_eq!(
        file_update_behavior_with_ci(FileUpdate::Auto, false, true),
        FileUpdateBehavior::NoUpdate
    );
    assert_eq!(
        file_update_behavior_with_ci(FileUpdate::Unseen, true, false),
        FileUpdateBehavior::NewFile
    );
    assert_eq!(
        file_update_behavior_with_ci(FileUpdate::Unseen, false, false),
        FileUpdateBehavior::InPlace
    );
    assert_eq!(
        file_update_behavior_with_ci(FileUpdate::New, false, false),
        FileUpdateBehavior::NewFile
    );
    assert_eq!(
        file_update_behavior_with_ci(FileUpdate::No, false, false),
        FileUpdateBehavior::NoUpdate
    );
    println!("passed file update behavior matrix 8/8");
}

#[test]
fn update_words_match_donor_env() {
    assert_eq!(parse_file_update("auto"), FileUpdate::Auto);
    assert_eq!(parse_file_update("always"), FileUpdate::Always);
    assert_eq!(parse_file_update("1"), FileUpdate::Always);
    assert_eq!(parse_file_update("new"), FileUpdate::New);
    assert_eq!(parse_file_update("no"), FileUpdate::No);
    assert_eq!(parse_file_update("unseen"), FileUpdate::Unseen);
    assert_eq!(parse_file_update("force"), FileUpdate::Force);
    let bad = std::panic::catch_unwind(|| parse_file_update("sometimes"));
    assert!(bad.is_err(), "unknown word must fail loud");
    println!("passed file update words 7/7");
}
