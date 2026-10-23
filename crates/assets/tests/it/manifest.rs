//! Seeded bake manifest: fixture rebuild equality and closure-exact
//! invalidation over the real cook cache.
//!
//! The rebuild leg bakes the demo staging twenty times through
//! [`cook_bundle_with_manifest`] and compares bundle bytes and manifest bytes
//! across runs (seed 2040, thread count 1, relative paths, versions posted).
//!
//! The invalidation leg drives the frozen [`Cooker`] and [`AssetRegistry`]
//! closure over a six-asset fixture in ten different ways and checks, per
//! edit, the reported `invalidated` set, the full-sweep `fresh` set and the
//! manifest's stale artifact paths against a hard-coded oracle.

#![cfg(all(feature = "import", feature = "registry"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use engine_assets::bundle_cook::demo_staged;
use engine_assets::cooker::{CookError, CookFailure, CookOutcome, Cooker};
use engine_assets::manifest::{
    cook_bundle_with_manifest, Manifest, DEFAULT_SEED, DEFAULT_THREADS, PROFILE_POTATO,
};
use engine_assets::registry::{content_hash, sha_hex, AssetId};

const MESH_V2: &[u8] = b"mesh-source-v2";
const MESH_V3: &[u8] = b"mesh-source-v3";
const MAT_V2: &[u8] = b"mat-source-v2";
const SKIN_V2: &[u8] = b"skin-source-v2";
const CLIP_V2: &[u8] = b"clip-source-v2";
const PACK_V2: &[u8] = b"pack-source-v2";
const LEAF_V2: &[u8] = b"leaf-source-v2";

/// Topological fixture order: every asset follows its dependencies.
const NAMES: [&str; 6] = ["mesh", "mat", "skin", "clip", "pack", "leaf"];

/// Dependency edges declared at fixture build time.
const DEPS: [(&str, &[&str]); 6] = [
    ("mesh", &[]),
    ("mat", &["mesh"]),
    ("skin", &["mat"]),
    ("clip", &["skin"]),
    ("pack", &["clip"]),
    ("leaf", &[]),
];

fn fixture_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("manifest_it_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    dir
}

fn artifact_path(name: &str) -> String {
    format!("art/{name}.bin")
}

fn paths(names: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = names.iter().map(|name| artifact_path(name)).collect();
    out.sort();
    out
}

fn set(names: &[&'static str]) -> BTreeSet<&'static str> {
    names.iter().copied().collect()
}

fn good(content: &[u8]) -> Result<Vec<u8>, CookFailure> {
    Ok(format!("{}:{}", sha_hex(content_hash(content)), content.len()).into_bytes())
}

fn fail(_content: &[u8]) -> Result<Vec<u8>, CookFailure> {
    Err(CookFailure::new("fixture refuses this edit"))
}

/// Six cooked assets plus the contents and dependency lists the sweep replays.
struct Fixture {
    cooker: Cooker,
    ids: BTreeMap<&'static str, AssetId>,
    contents: BTreeMap<&'static str, Vec<u8>>,
    deps: BTreeMap<&'static str, Vec<&'static str>>,
}

impl Fixture {
    fn build() -> Self {
        let mut fixture = Self {
            cooker: Cooker::new(),
            ids: BTreeMap::new(),
            contents: BTreeMap::new(),
            deps: BTreeMap::new(),
        };
        for (name, deps) in DEPS {
            let content = format!("{name}-source-v1").into_bytes();
            let dep_ids: Vec<AssetId> = deps
                .iter()
                .map(|dep| *fixture.ids.get(dep).expect("dependency cooked first"))
                .collect();
            let outcome = fixture
                .cooker
                .cook(&content, &dep_ids, good)
                .expect("fixture asset cooks");
            fixture.ids.insert(name, outcome.id);
            fixture.contents.insert(name, content);
            fixture.deps.insert(name, deps.to_vec());
        }
        fixture
    }

    fn id(&self, name: &str) -> AssetId {
        *self.ids.get(name).expect("known asset")
    }

    fn deps_of(&self, name: &str) -> Vec<AssetId> {
        self.deps
            .get(name)
            .expect("known asset")
            .iter()
            .map(|dep| self.id(dep))
            .collect()
    }

    fn recook(&mut self, name: &str) -> CookOutcome {
        let id = self.id(name);
        let content = self.contents.get(name).expect("known content").clone();
        let deps = self.deps_of(name);
        self.cooker
            .recook(id, &content, &deps, good)
            .expect("recook succeeds")
    }

    fn sweep(&mut self) -> BTreeSet<&'static str> {
        let mut fresh = BTreeSet::new();
        for name in NAMES {
            if self.recook(name).fresh {
                fresh.insert(name);
            }
        }
        fresh
    }

    fn name_of(&self, id: AssetId) -> &'static str {
        self.ids
            .iter()
            .find(|(_, known)| **known == id)
            .map(|(name, _)| *name)
            .expect("known id")
    }

    fn names(&self, ids: &[AssetId]) -> BTreeSet<&'static str> {
        ids.iter().map(|id| self.name_of(*id)).collect()
    }
}

fn manifest_of(fixture: &Fixture) -> Manifest {
    let mut manifest =
        Manifest::new(DEFAULT_SEED, DEFAULT_THREADS, PROFILE_POTATO).expect("manifest builds");
    manifest.set_settings(b"fixture").expect("settings");
    for name in NAMES {
        let id = fixture.id(name);
        let payload = &fixture
            .cooker
            .cache
            .get(&id)
            .expect("cached payload")
            .payload;
        manifest
            .add_artifact(&artifact_path(name), payload)
            .expect("artifact recorded");
    }
    manifest
}

/// One way to edit the fixture and the exact sets the cook cache must report.
struct Case {
    label: &'static str,
    edit: Edit,
    /// True when the edit itself runs the cook closure (a no-op is a cache hit).
    refreshed: bool,
    /// Transitive dependents the registry reports as invalidated.
    invalidated: &'static [&'static str],
    /// Assets the full sweep after the edit rebuilds (the edited asset already
    /// cooked, so it appears here only when it has dependents).
    sweep: &'static [&'static str],
    /// Artifact bytes the manifest reports stale (own payload changes only).
    stale: &'static [&'static str],
}

enum Edit {
    Content(&'static str, &'static [u8]),
    Deps(&'static str, &'static [&'static str]),
    NoOp(&'static str),
    Failed(&'static str, &'static [u8]),
}

const CASES: [Case; 10] = [
    Case {
        label: "mesh content",
        edit: Edit::Content("mesh", MESH_V2),
        refreshed: true,
        invalidated: &["mat", "skin", "clip", "pack"],
        sweep: &["mat", "skin", "clip", "pack"],
        stale: &["mesh"],
    },
    Case {
        label: "material content",
        edit: Edit::Content("mat", MAT_V2),
        refreshed: true,
        invalidated: &["skin", "clip", "pack"],
        sweep: &["skin", "clip", "pack"],
        stale: &["mat"],
    },
    Case {
        label: "skin content",
        edit: Edit::Content("skin", SKIN_V2),
        refreshed: true,
        invalidated: &["clip", "pack"],
        sweep: &["clip", "pack"],
        stale: &["skin"],
    },
    Case {
        label: "clip content",
        edit: Edit::Content("clip", CLIP_V2),
        refreshed: true,
        invalidated: &["pack"],
        sweep: &["pack"],
        stale: &["clip"],
    },
    Case {
        label: "top content",
        edit: Edit::Content("pack", PACK_V2),
        refreshed: true,
        invalidated: &[],
        sweep: &[],
        stale: &["pack"],
    },
    Case {
        label: "leaf content",
        edit: Edit::Content("leaf", LEAF_V2),
        refreshed: true,
        invalidated: &[],
        sweep: &[],
        stale: &["leaf"],
    },
    Case {
        label: "no-op rewrite",
        edit: Edit::NoOp("mesh"),
        refreshed: false,
        invalidated: &[],
        sweep: &[],
        stale: &[],
    },
    Case {
        label: "dependency added",
        edit: Edit::Deps("skin", &["mat", "leaf"]),
        refreshed: true,
        invalidated: &["clip", "pack"],
        sweep: &["clip", "pack"],
        stale: &[],
    },
    Case {
        label: "dependency removed",
        edit: Edit::Deps("skin", &["mat"]),
        refreshed: true,
        invalidated: &["clip", "pack"],
        sweep: &["clip", "pack"],
        stale: &[],
    },
    Case {
        label: "failed edit then good edit",
        edit: Edit::Failed("mesh", MESH_V3),
        refreshed: true,
        invalidated: &["mat", "skin", "clip", "pack"],
        sweep: &["mat", "skin", "clip", "pack"],
        stale: &["mesh"],
    },
];

fn failed_then_good(fixture: &mut Fixture, name: &'static str, bytes: &[u8]) -> CookOutcome {
    let id = fixture.id(name);
    let deps = fixture.deps_of(name);
    let before = fixture
        .cooker
        .cache
        .get(&id)
        .expect("cached payload")
        .clone();
    let refused = fixture.cooker.recook(id, bytes, &deps, fail);
    assert!(
        matches!(refused, Err(CookError::Failed(_))),
        "a failed job must report Failed"
    );
    let kept = fixture
        .cooker
        .cache
        .get(&id)
        .expect("cached payload")
        .clone();
    assert_eq!(kept, before, "a failed job keeps the last-good payload");
    fixture.contents.insert(name, bytes.to_vec());
    fixture
        .cooker
        .recook(id, bytes, &deps, good)
        .expect("the retried edit cooks")
}

fn apply(fixture: &mut Fixture, edit: &Edit) -> CookOutcome {
    match edit {
        Edit::Content(name, bytes) => {
            fixture.contents.insert(name, bytes.to_vec());
            fixture.recook(name)
        }
        Edit::Deps(name, deps) => {
            fixture.deps.insert(name, deps.to_vec());
            fixture.recook(name)
        }
        Edit::NoOp(name) => fixture.recook(name),
        Edit::Failed(name, bytes) => failed_then_good(fixture, name, bytes),
    }
}

fn run_case(fixture: &mut Fixture, case: &Case) {
    let before = manifest_of(fixture);
    let outcome = apply(fixture, &case.edit);
    assert_eq!(
        outcome.fresh, case.refreshed,
        "{}: the edit's own cook",
        case.label
    );
    assert_eq!(
        fixture.names(&outcome.invalidated),
        set(case.invalidated),
        "{}: invalidated set",
        case.label
    );
    assert_eq!(
        fixture.sweep(),
        set(case.sweep),
        "{}: full-sweep fresh set",
        case.label
    );
    let after = manifest_of(fixture);
    assert_eq!(
        after.stale_artifacts(&before),
        paths(case.stale),
        "{}: stale artifact paths",
        case.label
    );
}

/// Bake the demo staging twenty times and require identical bytes every run.
#[test]
fn twenty_fixture_rebuilds_are_byte_equal() {
    let mut first: Option<(Vec<u8>, Vec<u8>, String)> = None;
    for run in 0..20 {
        let dir = fixture_dir(&format!("x20_{run:02}"));
        let out = dir.join("demo.e2bundle");
        let baked = cook_bundle_with_manifest(
            &demo_staged(),
            &out,
            false,
            DEFAULT_SEED,
            DEFAULT_THREADS,
            PROFILE_POTATO,
        )
        .expect("bake runs");
        let manifest_bytes = std::fs::read(&baked.manifest).expect("manifest bytes read");
        let bundle_bytes = std::fs::read(&out).expect("bundle bytes read");
        let text = String::from_utf8(manifest_bytes.clone()).expect("manifest is utf-8");
        assert!(
            text.contains(&format!("\nseed {DEFAULT_SEED}\n")),
            "run {run}: seed line missing"
        );
        assert!(
            text.contains(&format!("\nthreads {DEFAULT_THREADS}\n")),
            "run {run}: thread line missing"
        );
        assert!(
            text.contains("\nimporter-version 1\n"),
            "run {run}: importer version line missing"
        );
        assert!(
            text.contains(&format!("\nprofile {PROFILE_POTATO}\n")),
            "run {run}: profile line missing"
        );
        let manifest = Manifest::read(&baked.manifest).expect("manifest reads back");
        assert_eq!(manifest.sources().count(), 5, "run {run}");
        assert_eq!(manifest.artifacts().count(), 1, "run {run}");
        assert_eq!(
            manifest.digest().expect("manifest digest"),
            baked.digest,
            "run {run}: reported digest"
        );
        manifest.verify_artifacts(&dir).expect("artifact verifies");
        match &first {
            Some((first_manifest, first_bundle, first_digest)) => {
                assert_eq!(&manifest_bytes, first_manifest, "run {run}: manifest bytes");
                assert_eq!(&bundle_bytes, first_bundle, "run {run}: bundle bytes");
                assert_eq!(&baked.digest, first_digest, "run {run}: digest");
            }
            None => {
                println!(
                    "run 0: manifest digest {} (bundle {} bytes)",
                    baked.digest,
                    bundle_bytes.len()
                );
                first = Some((manifest_bytes, bundle_bytes, baked.digest.clone()));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert!(first.is_some(), "twenty runs completed");
}

/// Ten different edits; each must invalidate exactly the dependent closure.
#[test]
fn ten_edits_invalidate_exactly_the_transitive_closure() {
    assert_eq!(CASES.len(), 10);
    let mut fixture = Fixture::build();
    for case in &CASES {
        run_case(&mut fixture, case);
    }
}
