//! CT42 batch-a: the 16 combat clips retarget through CT1 onto the hero bind, dedupe against the
//! 14 canonical cook digests and against each other, cook identical twice, and match the committed
//! receipt row for row (glb bytes, digest, duration, keys, loop, root motion). The receipt was
//! cooked on Windows; two of its digests (attack_2h_3, power_up_2) differ on a Linux runner (a
//! platform float difference), so the digest column is compared on Windows only.
#![cfg(feature = "import")]

use engine_assets::body_import::import_clip_source;
use engine_assets::clip_intake::CANONICAL_DIGESTS;
use engine_assets::clip_receipt::{MAX_CLIP_KEYS, MAX_CLIP_SECS};
use engine_assets::{
    import_clip, retarget_cook_with_sidecar, retarget_digest, validate_bind, BoneMap, CookConfig,
    RetargetPlan,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn fighters() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters")
}

/// The receipt's clip rows (`| short | action | ...`), header and rule dropped.
fn receipt_rows(batch: &Path) -> Vec<String> {
    let text = fs::read_to_string(batch.join("BATCH-A-RECEIPT.md")).expect("receipt");
    text.lines()
        .filter(|line| line.starts_with("| ") && !line.starts_with("| short"))
        .map(str::to_owned)
        .collect()
}

/// A receipt row with its digest cell (the fourth) dropped.
fn without_digest(row: &str) -> String {
    let mut cells: Vec<&str> = row.split('|').collect();
    cells.remove(4);
    cells.join("|")
}

#[test]
fn retarget_batch_a_sixteen_clips_cook_twice_deduped_and_match_the_receipt() {
    let batch = fighters().join("incoming/batch-a");
    let hero = import_clip_source(&fs::read(fighters().join("glb/hero_body.glb")).expect("hero"))
        .expect("hero imports");
    validate_bind(&hero.skeleton, &hero.bind).expect("hero bind is valid");
    let joint = |name: &str| {
        hero.skeleton
            .joints
            .iter()
            .position(|j| j.name == name)
            .unwrap_or_else(|| panic!("hero has no {name}"))
    };
    let config = CookConfig {
        root_joint: joint("pelvis"),
        left_foot: joint("foot_l"),
        right_foot: joint("foot_r"),
        ground_y: 0.02,
        start_tick: 0,
    };
    let pairs: Vec<(&str, &str)> = hero
        .skeleton
        .joints
        .iter()
        .map(|j| (j.name.as_str(), j.name.as_str()))
        .collect();
    let map = BoneMap::build(&hero.skeleton, &hero.skeleton, &pairs).expect("bone map");
    let canonical: BTreeSet<u64> = CANONICAL_DIGESTS.into_iter().collect();
    let mut seen = BTreeSet::new();
    let mut rows = Vec::new();
    for want in receipt_rows(&batch) {
        let cells: Vec<&str> = want.split('|').map(str::trim).collect();
        let (short, action) = (cells[1], cells[2]);
        let glb = fs::read(batch.join(format!("glb/{action}.glb"))).expect("clip glb");
        let sidecar =
            fs::read_to_string(batch.join(format!("sidecars/{short}.json"))).expect("sidecar");
        // The clip rides the hero rig (same bones, same bind): the way the 14 canonical digests were cooked.
        let (doc, buffers, _) = gltf::import_slice(&glb).expect("clip parses");
        let clips = import_clip(&doc, &buffers, &hero.skeleton).expect("clip imports");
        assert_eq!(clips.len(), 1, "{short}: one animation");
        let plan = RetargetPlan {
            source_clip: &clips[0],
            source: &hero.skeleton,
            source_bind: &hero.bind,
            target: &hero.skeleton,
            target_bind: &hero.bind,
            map: &map,
            config: &config,
        };
        let first = retarget_cook_with_sidecar(&plan, &sidecar).expect("CT1 cook");
        let second = retarget_cook_with_sidecar(&plan, &sidecar).expect("CT1 recook");
        assert!(first == second, "{short}: cook x2 differs");
        let digest = retarget_digest(&first);
        assert!(
            !canonical.contains(&digest),
            "{short} repeats a canonical digest"
        );
        assert!(seen.insert(digest), "{short} repeats a batch-a digest");
        let keys: usize = first
            .clip
            .positions
            .tracks
            .iter()
            .map(|t| t.times.len())
            .sum();
        assert!(
            keys <= MAX_CLIP_KEYS && first.clip.duration <= MAX_CLIP_SECS,
            "{short}: over the clip budget"
        );
        rows.push(format!(
            "| {short} | {action} | {} | {digest:016x} | {:.3} | {keys} | {} | {} |",
            glb.len(),
            first.clip.duration,
            first.tags.looped,
            first.tags.root_motion,
        ));
    }
    assert_eq!(rows.len(), 16, "batch-a holds 16 clips");
    let committed = receipt_rows(&batch);
    if !cfg!(windows) {
        let strip = |rows: &[String]| rows.iter().map(|r| without_digest(r)).collect::<Vec<_>>();
        assert_eq!(
            strip(&rows),
            strip(&committed),
            "the cook no longer matches BATCH-A-RECEIPT.md"
        );
        return;
    }
    assert_eq!(
        rows, committed,
        "the cook no longer matches BATCH-A-RECEIPT.md"
    );
}
