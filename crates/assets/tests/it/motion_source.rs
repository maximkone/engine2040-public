//! CT14: the provider-style motion source lands a prompt+seed clip on the
//! fighter rig, proves the bind pose, meets the planted-foot budget, and marks
//! or refuses a non-commercial model before the packaged path writes anything.

#![cfg(all(feature = "import", feature = "registry"))]

use std::path::PathBuf;

use engine_assets::manifest::Manifest;
use engine_assets::motion_source::{
    bake_motion, BakeConfig, BakedMotion, LicenseMark, MockMotionSource, MotionError,
    MotionRequest, MAX_FOOT_SLIDE_MM,
};
use engine_assets::retarget::retarget_digest;

const PROMPT: &str = "sword slash";

fn bake(seed: u64) -> BakedMotion {
    let source = MockMotionSource::commercial();
    let request = MotionRequest::new(PROMPT, seed).expect("prompt");
    bake_motion(&source, &request, &BakeConfig::default()).expect("bake")
}

fn package_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("motion_source_it_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn sword_slash_seed_42_imports_retargets_and_plays_within_twenty_mm() {
    let first = bake(42);
    let second = bake(42);

    assert_eq!(first.motion.source.len(), 22, "HumanML3D-22 source");
    assert_eq!(first.motion.clip.positions.tracks.len(), 22);
    assert_eq!(first.motion.clip.name, "sword_slash");
    assert_eq!(first.bind.source_joints, 22);
    assert_eq!(first.bind.target_joints, 65, "fighter-65 target");
    assert_eq!(first.bind.pairs, 20, "every source joint with a target");
    assert_eq!(first.retargeted.clip.positions.tracks.len(), 65);
    assert_eq!(first.retargeted.contacts.contacts.len(), 120);
    assert_eq!(first.play.ticks, 60);
    assert_eq!(first.play.left_plants, 60, "left foot stays planted");
    assert_eq!(first.play.right_plants, 60, "right foot stays planted");
    assert_eq!(first.play.left_lifts, 0);
    assert_eq!(first.play.right_lifts, 0);
    assert_eq!(
        first.play.foot_slide_mm, 0,
        "the authored feet do not slide"
    );
    assert!(first.play.foot_slide_mm <= MAX_FOOT_SLIDE_MM);
    assert!(
        first.play.max_joint_travel_mm >= 100,
        "the slash moves a hand: {} mm",
        first.play.max_joint_travel_mm
    );
    assert!(
        first.play.moving_joints >= 4,
        "only {} joints moved",
        first.play.moving_joints
    );
    assert_eq!(first.retargeted.tags.hit_windows.len(), 1);
    assert_eq!(first.retargeted.tags.hit_windows[0].socket, "SOCKET_sword");

    assert_eq!(first.retargeted, second.retargeted);
    assert_eq!(
        retarget_digest(&first.retargeted),
        retarget_digest(&second.retargeted)
    );
    assert_eq!(first.compressed.as_bytes(), second.compressed.as_bytes());
    assert_eq!(first.manifest_bytes, second.manifest_bytes);
    assert_eq!(first.manifest_digest, second.manifest_digest);
    assert_eq!(first.license, LicenseMark::Commercial);
    assert_eq!(first.model_hash.len(), 64);

    let other = bake(43);
    assert_ne!(
        first.motion.clip, other.motion.clip,
        "the seed changes the authored clip"
    );
    assert_ne!(first.manifest_digest, other.manifest_digest);

    let header = first.compressed.header().expect("header");
    assert_eq!(header.joints, 65);
    assert!((header.duration - 1.0).abs() < 1e-4);
    assert!(
        first.compression.bytes < first.compression.raw_bytes,
        "compression {} -> {}",
        first.compression.raw_bytes,
        first.compression.bytes
    );

    let manifest = Manifest::parse(&first.manifest_bytes).expect("parse manifest");
    assert_eq!(manifest, first.manifest);
    assert_eq!(manifest.seed(), 42);
    assert_eq!(manifest.threads(), 1);
    assert_eq!(manifest.profile(), "potato");
    assert_eq!(manifest.sources().count(), 2);
    assert_eq!(manifest.artifacts().count(), 1);
    let settings = std::str::from_utf8(manifest.settings()).expect("settings are text");
    assert!(settings.contains(&format!("model-hash {}", first.model_hash)));
    assert!(settings.contains("prompt sword slash"));
    assert!(settings.contains("seed 42"));
    assert!(settings.contains("license commercial"));
    assert!(settings.contains(&format!("target-bind {}", first.bind.target_digest)));
    assert!(settings.contains("training\tengine2040 authored mock clips\tCC0-1.0\tcommercial"));
    assert!(settings.contains("foot-slide-mm 0"));

    println!(
        "CT14 seed 42: source={} target={} pairs={} ticks={} slide={}mm travel={}mm moving={} manifest={} compressed={}B/{}B",
        first.bind.source_joints,
        first.bind.target_joints,
        first.bind.pairs,
        first.play.ticks,
        first.play.foot_slide_mm,
        first.play.max_joint_travel_mm,
        first.play.moving_joints,
        first.manifest_digest,
        first.compression.bytes,
        first.compression.raw_bytes,
    );
}

#[test]
fn non_commercial_model_is_marked_and_the_packaged_build_refuses() {
    let source = MockMotionSource::non_commercial();
    let request = MotionRequest::new(PROMPT, 42).expect("prompt");
    let baked = bake_motion(&source, &request, &BakeConfig::default()).expect("bake");
    assert_eq!(baked.license, LicenseMark::NonCommercial);
    assert_eq!(
        baked.motion.card.non_commercial().expect("dataset").license,
        "CC-BY-NC-4.0"
    );
    let settings = std::str::from_utf8(baked.manifest.settings()).expect("settings are text");
    assert!(settings.contains("license non-commercial"));
    assert!(settings.contains("AMASS"));
    assert!(settings.contains("CC-BY-NC-4.0"));
    assert!(settings.contains("non-commercial"));

    let dir = package_dir("nc");
    let error = baked.package(&dir).expect_err("the packaged path refuses");
    match error {
        MotionError::NonCommercial {
            model,
            dataset,
            license,
        } => {
            assert_eq!(model, "engine2040/mock-motion-amass");
            assert!(dataset.contains("AMASS"));
            assert_eq!(license, "CC-BY-NC-4.0");
        }
        other => panic!("want NonCommercial, got {other}"),
    }
    assert!(!dir.exists(), "a refused package writes nothing");
}

#[test]
fn commercial_package_writes_a_ct12_manifest_and_reads_it_back() {
    let baked = bake(42);
    let dir = package_dir("ok");
    let pack = baked.package(&dir).expect("package");
    assert_eq!(pack.clip_bytes, baked.compressed.byte_len());
    assert_eq!(pack.manifest_digest, baked.manifest_digest);
    assert_eq!(
        std::fs::read(&pack.clip_path).expect("clip bytes"),
        baked.compressed.as_bytes()
    );
    let manifest = Manifest::read(&pack.manifest_path).expect("read manifest");
    assert_eq!(manifest, baked.manifest);
    assert_eq!(
        manifest.digest().expect("manifest digest"),
        baked.manifest_digest
    );
    manifest.verify_artifacts(&dir).expect("artifact digest");

    let roles: Vec<&str> = manifest.sources().map(|record| record.role()).collect();
    assert!(roles.contains(&"model"));
    assert!(roles.contains(&"prompt"));
    let artifact = manifest.artifacts().next().expect("one artifact");
    assert_eq!(artifact.path(), "motion.e2anm");
    assert_eq!(
        manifest.settings(),
        baked.manifest.settings(),
        "settings round-trip byte for byte"
    );
}
