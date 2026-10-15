//! FB2 version receipt: pinned asset, scene, save and tape versions, the stored
//! migration fixture that still replays, and the typed refusals it must give.
//!
//! Consumer: the FB2 board row; the report lands in `target/evidence/b/versions/`.
//! Data owner: this module owns only the contract table and the report text.
//! Each [`VersionRow`] names one owner constant. The default tier reads the owner
//! source through `include_str!`, so a bumped owner version fails until its row
//! changes (zero unlisted versions). The `m1` tier drives the owners' public APIs
//! (bundle cook and loader, scene schema, prefab and codec, editor save) and
//! records what they answer; nothing is reimplemented here.
//! Determinism scope: the report is a pure function of the owner sources, the
//! fixture and the owner outputs, so two runs write identical bytes. Errors: a
//! failed check fails its test with a [`ReceiptError`] naming the step.

use std::fmt;

use engine_core::hash_bytes;

use crate::museum::{load_tape_v1, migrate_and_replay};
use crate::{record, Config, Error, GOLDEN_600};

/// Stored tape v1 fixture: the museum loads, migrates and replays it.
static TAPE_V1: &[u8] = include_bytes!("../../../fixtures/tape-v1.bin");

/// Why a receipt check stopped. Owner failures keep the owner's own text under
/// the step that asked; an invariant names the rule that broke.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReceiptError {
    /// An owner call or file step failed; `detail` is the owner's own text.
    Failed {
        /// The receipt step that made the call.
        step: &'static str,
        /// The error text as the owner's `Display` prints it.
        detail: String,
    },
    /// A fixed rule of the receipt did not hold.
    Invariant(&'static str),
}

impl fmt::Display for ReceiptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed { step, detail } => write!(f, "{step} failed: {detail}"),
            Self::Invariant(rule) => write!(f, "invariant broken: {rule}"),
        }
    }
}

impl std::error::Error for ReceiptError {}

/// Map any owner or file error into a [`ReceiptError::Failed`] at `step`.
pub fn at<E: fmt::Display>(step: &'static str) -> impl FnOnce(E) -> ReceiptError {
    move |error| ReceiptError::Failed {
        step,
        detail: error.to_string(),
    }
}

/// Whether a row pins the version an owner writes today, or the oldest one it
/// still reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The version the owner writes today.
    Current,
    /// The oldest version the owner still migrates; never above its current.
    Floor,
}

/// One pinned owner constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VersionRow {
    /// Stable row id printed in the report.
    pub id: &'static str,
    /// Owner source file, relative to `crates/`.
    pub owner: &'static str,
    /// Constant name as the owner declares it.
    pub constant: &'static str,
    /// Right-hand side exactly as the owner writes it, e.g. `1` or `SCHEMA_VERSION`.
    pub expr: &'static str,
    /// Resolved value of the constant.
    pub value: u32,
    /// Current version or migration floor.
    pub role: Role,
}

/// Every pinned version, in report order. A new `VERSION` constant in a scanned
/// owner file must get a row here, or [`unlisted_versions`] fails.
pub const VERSION_ROWS: &[VersionRow] = &[
    VersionRow {
        id: "bundle",
        owner: "assets/src/bundle.rs",
        constant: "BUNDLE_VERSION",
        expr: "1",
        value: 1,
        role: Role::Current,
    },
    VersionRow {
        id: "archive",
        owner: "assets/src/archive.rs",
        constant: "ARCHIVE_VERSION",
        expr: "1",
        value: 1,
        role: Role::Current,
    },
    VersionRow {
        id: "schema",
        owner: "scene/src/ids.rs",
        constant: "SCHEMA_VERSION",
        expr: "1",
        value: 1,
        role: Role::Current,
    },
    VersionRow {
        id: "component",
        owner: "scene/src/component.rs",
        constant: "SCHEMA_VERSION",
        expr: "1",
        value: 1,
        role: Role::Current,
    },
    VersionRow {
        id: "scene",
        owner: "scene/src/codec.rs",
        constant: "CURRENT_VERSION",
        expr: "SCHEMA_VERSION",
        value: 1,
        role: Role::Current,
    },
    VersionRow {
        id: "scene-floor",
        owner: "scene/src/codec.rs",
        constant: "MIN_SUPPORTED_VERSION",
        expr: "1",
        value: 1,
        role: Role::Floor,
    },
    VersionRow {
        id: "prefab",
        owner: "scene/src/prefab.rs",
        constant: "PREFAB_VERSION",
        expr: "1",
        value: 1,
        role: Role::Current,
    },
    VersionRow {
        id: "provenance",
        owner: "scene/src/provenance.rs",
        constant: "PROVENANCE_VERSION",
        expr: "1",
        value: 1,
        role: Role::Current,
    },
    VersionRow {
        id: "provenance-floor",
        owner: "scene/src/provenance.rs",
        constant: "MIN_PROVENANCE_VERSION",
        expr: "1",
        value: 1,
        role: Role::Floor,
    },
    VersionRow {
        id: "save",
        owner: "editor/src/save.rs",
        constant: "SAVE_VERSION",
        expr: "3",
        value: 3,
        role: Role::Current,
    },
    VersionRow {
        id: "save-floor",
        owner: "editor/src/save.rs",
        constant: "MIN_SAVE_VERSION",
        expr: "1",
        value: 1,
        role: Role::Floor,
    },
    VersionRow {
        id: "tape",
        owner: "proof/src/tape.rs",
        constant: "FORMAT_VERSION",
        expr: "1",
        value: 1,
        role: Role::Current,
    },
];

/// Owner sources the drift scan reads, keyed by the owner path the rows use.
const OWNER_SOURCES: &[(&str, &str)] = &[
    (
        "assets/src/archive.rs",
        include_str!("../../assets/src/archive.rs"),
    ),
    (
        "assets/src/bundle.rs",
        include_str!("../../assets/src/bundle.rs"),
    ),
    ("editor/src/save.rs", include_str!("../../editor/src/save.rs")),
    ("proof/src/tape.rs", include_str!("tape.rs")),
    ("scene/src/codec.rs", include_str!("../../scene/src/codec.rs")),
    (
        "scene/src/component.rs",
        include_str!("../../scene/src/component.rs"),
    ),
    ("scene/src/ids.rs", include_str!("../../scene/src/ids.rs")),
    (
        "scene/src/prefab.rs",
        include_str!("../../scene/src/prefab.rs"),
    ),
    (
        "scene/src/provenance.rs",
        include_str!("../../scene/src/provenance.rs"),
    ),
];

/// Source text of one scanned owner file.
fn owner_source(owner: &str) -> Option<&'static str> {
    OWNER_SOURCES
        .iter()
        .find(|(path, _)| *path == owner)
        .map(|(_, text)| *text)
}

/// Right-hand side of the first `pub const <constant>:` line in `source`.
fn declared_rhs(source: &str, constant: &str) -> Option<String> {
    let head = format!("pub const {constant}:");
    let line = source
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(head.as_str()))?;
    let (_, rhs) = line.split_once('=')?;
    Some(rhs.trim().trim_end_matches(';').trim().to_string())
}

/// True when the row's expression resolves to its pinned value: a literal, or
/// the name of another row with the same value.
fn resolves(row: &VersionRow) -> bool {
    match row.expr.parse::<u32>() {
        Ok(literal) => literal == row.value,
        Err(_) => VERSION_ROWS
            .iter()
            .any(|other| other.constant == row.expr && other.value == row.value),
    }
}

/// One line per row whose owner source does not declare the pinned expression,
/// or whose expression does not resolve to the pinned value. Empty when every
/// row matches its owner.
pub fn row_mismatches() -> Vec<String> {
    VERSION_ROWS
        .iter()
        .filter_map(|row| {
            let declared =
                owner_source(row.owner).and_then(|text| declared_rhs(text, row.constant));
            match declared {
                Some(rhs) if rhs == row.expr && resolves(row) => None,
                Some(rhs) => Some(format!(
                    "{}: {} = {rhs}, row pins {} = {}",
                    row.id, row.constant, row.expr, row.value
                )),
                None => Some(format!(
                    "{}: {} not declared in {}",
                    row.id, row.constant, row.owner
                )),
            }
        })
        .collect()
}

/// `owner:CONSTANT` for every `VERSION` constant in the scanned owner sources
/// that no row pins, in scan order. Empty means zero unlisted versions.
pub fn unlisted_versions() -> Vec<String> {
    let mut unlisted = Vec::new();
    for (owner, text) in OWNER_SOURCES {
        for line in text.lines().map(str::trim) {
            let Some(name) = line
                .strip_prefix("pub const ")
                .and_then(|rest| rest.split(':').next())
            else {
                continue;
            };
            if !name.contains("VERSION") {
                continue;
            }
            if !VERSION_ROWS
                .iter()
                .any(|row| row.owner == *owner && row.constant == name)
            {
                unlisted.push(format!("{owner}:{name}"));
            }
        }
    }
    unlisted
}

/// One line per floor that is zero or above its owner's current version.
/// Compat policy: the oldest readable version never exceeds the one written.
pub fn floor_violations() -> Vec<String> {
    VERSION_ROWS
        .iter()
        .filter(|row| row.role == Role::Floor)
        .filter_map(|floor| {
            let current = VERSION_ROWS
                .iter()
                .find(|row| row.owner == floor.owner && row.role == Role::Current);
            match current {
                Some(current) if floor.value >= 1 && floor.value <= current.value => None,
                _ => Some(format!(
                    "{} floor {} is zero or above its current",
                    floor.id, floor.value
                )),
            }
        })
        .collect()
}

/// Legacy checkpoint after the default run's last tick.
pub fn golden_checkpoint() -> Result<u64, ReceiptError> {
    let tape = record(Config::default(), &[]).map_err(at("default run"))?;
    tape.checkpoints
        .last()
        .map(|checkpoint| checkpoint.legacy)
        .ok_or(ReceiptError::Invariant(
            "default run recorded no checkpoints",
        ))
}

/// Load, migrate and replay the stored tape v1 fixture; returns its checkpoint
/// count, which must equal the fixture's own tick count.
pub fn museum_replay() -> Result<usize, ReceiptError> {
    let tape = load_tape_v1(TAPE_V1).map_err(at("load tape-v1"))?;
    let checkpoints = migrate_and_replay(TAPE_V1).map_err(at("replay tape-v1"))?;
    if usize::try_from(tape.config.ticks).ok() != Some(checkpoints.len()) {
        return Err(ReceiptError::Invariant(
            "replayed checkpoints differ from the fixture ticks",
        ));
    }
    Ok(checkpoints.len())
}

/// `name typed` when the check answered with its own variant, else an
/// `UNEXPECTED` line that fails the report.
fn tag(name: &str, ok: bool) -> String {
    if ok {
        format!("{name} typed")
    } else {
        format!("{name} UNEXPECTED")
    }
}

/// Typed refusals of the museum loader, one per corruption.
pub fn tape_refusals() -> Vec<String> {
    let mut bad_magic = TAPE_V1.to_vec();
    bad_magic[0] ^= 0xFF;
    vec![
        tag(
            "empty-tape",
            matches!(load_tape_v1(&[]), Err(Error::Invalid(_))),
        ),
        tag(
            "short-tape",
            matches!(load_tape_v1(&TAPE_V1[..8]), Err(Error::Invalid(_))),
        ),
        tag(
            "bad-magic",
            matches!(load_tape_v1(&bad_magic), Err(Error::Invalid(_))),
        ),
    ]
}

/// Scratch folder for the `m1` tier's file moves, under the evidence folder.
#[cfg(feature = "m1")]
fn scratch_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/evidence/b/versions/scratch")
}

/// Build the FB2 receipt text. Same owner sources, fixture and owner outputs
/// give the same text.
pub fn build_report() -> Result<String, ReceiptError> {
    let golden = golden_checkpoint()?;
    let replayed = museum_replay()?;
    let tier = if cfg!(feature = "m1") {
        "contract+owner"
    } else {
        "contract"
    };
    let mut lines = vec![
        "fb2 version receipt".to_string(),
        format!("tier {tier}"),
        format!(
            "golden {golden:016x} {}",
            if golden == GOLDEN_600 { "ok" } else { "CHANGED" }
        ),
        format!(
            "fixture tape-v1 bytes {} digest {:016x}",
            TAPE_V1.len(),
            hash_bytes(TAPE_V1)
        ),
        format!("museum tape-v1 checkpoints {replayed} replay ok"),
    ];
    for refusal in tape_refusals() {
        lines.push(format!("refusal {refusal}"));
    }
    for row in VERSION_ROWS {
        lines.push(format!(
            "row {} {} {} = {} role {:?} value {}",
            row.id, row.owner, row.constant, row.expr, row.role, row.value
        ));
    }
    lines.push(format!(
        "rows {} mismatches {} unlisted {} floor-violations {}",
        VERSION_ROWS.len(),
        row_mismatches().len(),
        unlisted_versions().len(),
        floor_violations().len()
    ));
    #[cfg(feature = "m1")]
    {
        let record = owner::owner_record(&scratch_dir())?;
        lines.extend(owner::owner_lines(&record));
    }
    #[cfg(not(feature = "m1"))]
    lines.push("owner tier not run (build with --features m1)".to_string());
    Ok(format!("{}\n", lines.join("\n")))
}

/// The `m1` tier: the owners' own APIs answer the FB2 questions. Calls are
/// serialized because they share one scratch folder.
#[cfg(feature = "m1")]
mod owner {
    use std::fs;
    use std::path::Path;
    use std::sync::Mutex;

    use engine_assets::{
        cook_and_write, cook_bundle_bytes, demo_staged, load_bundle_file, verify_bundle_bytes,
        write_bundle_file, BundleCookError, BundleError, EntryKind, LoadedBundle, StagedCooked,
        BUNDLE_VERSION,
    };
    use engine_core::hash_bytes;
    use engine_editor::commands::{Document, Journal};
    use engine_editor::save::{load_from_bytes, save_to_bytes, SaveError, SAVE_VERSION};
    use engine_editor::templates::{TemplateKind, TemplateService};
    use engine_scene::codec::{CodecError, Scene, CURRENT_VERSION, MIN_SUPPORTED_VERSION};
    use engine_scene::prefab::{Prefab, PrefabNode, PREFAB_VERSION};
    use engine_scene::schema::{SchemaError, SchemaRegistry, VersionedComponent};
    use engine_scene::transform::{Hierarchy, Transform};

    use super::{at, ReceiptError};

    static SCRATCH_LOCK: Mutex<()> = Mutex::new(());

    /// Checked facts from one owner pass; each field answers one question.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct OwnerRecord {
        /// Digest of the demo bundle bytes (`engine_core::hash_bytes`).
        pub bundle_digest: u64,
        /// Demo bundle byte length.
        pub bundle_bytes: usize,
        /// Entries in the demo bundle; equals the staged count.
        pub bundle_entries: usize,
        /// Same staged set cooked twice and again in reversed order gives equal bytes.
        pub cook_stable: bool,
        /// One `name typed` line per refusal check; `UNEXPECTED` fails the test.
        pub refusals: Vec<String>,
        /// The bundle file moved to a new folder reads back to the same bytes.
        pub moved_bundle_equal: bool,
        /// Staged entries the moved bundle no longer names by kind and name.
        pub lost_bundle_refs: usize,
        /// The editor save stamps the current save version.
        pub save_stamped: bool,
        /// The saved document reopens equal.
        pub save_round_trip: bool,
        /// The saved document, moved to a new folder, reopens equal.
        pub save_moved_equal: bool,
        /// A save stamped one version above current is refused.
        pub save_future_refused: bool,
        /// A label at version 1 passes the registry unchanged.
        pub label_v1_loads: bool,
        /// A label at version 2 (above current) is refused as unknown.
        pub label_v2_refused: bool,
        /// Instances minted by the two-node prefab.
        pub prefab_instances: usize,
        /// Scene encode then decode gives an equal hierarchy.
        pub scene_round_trip: bool,
        /// A scene save one version above current is refused.
        pub scene_future_refused: bool,
    }

    /// Editor save checks: stamp, round trip, project move and a future version.
    struct SaveFacts {
        stamped: bool,
        round_trip: bool,
        moved_equal: bool,
        future_refused: bool,
    }

    /// Schema, prefab and scene checks.
    struct SchemaFacts {
        label_v1_loads: bool,
        label_v2_refused: bool,
        prefab_instances: usize,
        scene_round_trip: bool,
        scene_future_refused: bool,
    }

    fn tag(name: &str, ok: bool) -> String {
        if ok {
            format!("{name} typed")
        } else {
            format!("{name} UNEXPECTED")
        }
    }

    fn label(text: &str, version: u32) -> VersionedComponent {
        VersionedComponent {
            component: "label".to_string(),
            version,
            payload: text.to_string(),
        }
    }

    fn prefab_node(key: &str, parent: Option<&str>, x: f32, text: &str) -> PrefabNode {
        PrefabNode {
            key: key.to_string(),
            parent: parent.map(str::to_string),
            transform: Transform::new([x, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0]),
            components: vec![label(text, 1)],
        }
    }

    /// Create the folder that will hold `path`, if it has one.
    fn ensure_parent(path: &Path) -> Result<(), ReceiptError> {
        match path.parent() {
            Some(parent) => fs::create_dir_all(parent).map_err(at("create folder")),
            None => Ok(()),
        }
    }

    /// Move a file to a new folder, the way a project move does.
    fn carry(from: &Path, to: &Path) -> Result<(), ReceiptError> {
        ensure_parent(to)?;
        fs::rename(from, to).map_err(at("move project file"))
    }

    /// The staged set with every kind's insertion order reversed.
    fn reversed(staged: &StagedCooked) -> StagedCooked {
        let mut out = staged.clone();
        out.meshes.reverse();
        out.materials.reverse();
        out.textures.reverse();
        out.skins.reverse();
        out.clips.reverse();
        out
    }

    /// Staged names the loaded bundle no longer finds under their kind.
    fn lost_refs(staged: &StagedCooked, loaded: &LoadedBundle) -> usize {
        let missing = |kind: EntryKind, name: &String| loaded.find(kind, name).is_none();
        staged
            .meshes
            .iter()
            .filter(|(name, _)| missing(EntryKind::Mesh, name))
            .count()
            + staged
                .materials
                .iter()
                .filter(|(name, _)| missing(EntryKind::Material, name))
                .count()
            + staged
                .textures
                .iter()
                .filter(|(name, _)| missing(EntryKind::Texture, name))
                .count()
            + staged
                .skins
                .iter()
                .filter(|(name, _)| missing(EntryKind::Skin, name))
                .count()
            + staged
                .clips
                .iter()
                .filter(|(name, _)| missing(EntryKind::Clip, name))
                .count()
    }

    /// Cook the demo set three ways; return the first bytes and whether all
    /// three agree. The loader must also accept the bytes with every entry.
    fn cook_demo(staged: &StagedCooked) -> Result<(Vec<u8>, bool), ReceiptError> {
        let first = cook_bundle_bytes(staged).map_err(at("cook demo bundle"))?;
        let second = cook_bundle_bytes(staged).map_err(at("cook demo bundle again"))?;
        let third =
            cook_bundle_bytes(&reversed(staged)).map_err(at("cook reversed demo bundle"))?;
        let stable = first == second && first == third;
        let loaded = verify_bundle_bytes(&first, false).map_err(at("verify demo bundle"))?;
        if loaded.len() != staged.len() {
            return Err(ReceiptError::Invariant(
                "verify kept a different entry count",
            ));
        }
        Ok((first, stable))
    }

    /// The typed refusal checks, one line each.
    fn refusal_checks(scratch: &Path, first: &[u8]) -> Result<Vec<String>, ReceiptError> {
        let stub =
            cook_bundle_bytes(&StagedCooked::new()).map_err(at("cook empty stub"))?;
        let mut bad_magic = first.to_vec();
        bad_magic[0] ^= 0xFF;
        let mut future = first.to_vec();
        future[8..12].copy_from_slice(&(BUNDLE_VERSION + 1).to_le_bytes());
        let mut flipped = first.to_vec();
        let last = flipped.len() - 1;
        flipped[last] ^= 0x01;
        Ok(vec![
            tag(
                "empty-without-allow-stub",
                matches!(
                    cook_and_write(&StagedCooked::new(), &scratch.join("empty.e204"), false),
                    Err(BundleCookError::EmptyWithoutAllowStub)
                ),
            ),
            tag(
                "stub-without-allow-stub",
                matches!(
                    verify_bundle_bytes(&stub, false),
                    Err(BundleCookError::EmptyWithoutAllowStub)
                ),
            ),
            tag(
                "stub-with-allow-stub",
                verify_bundle_bytes(&stub, true).is_ok_and(|bundle| bundle.is_empty()),
            ),
            tag(
                "bad-magic",
                matches!(
                    verify_bundle_bytes(&bad_magic, false),
                    Err(BundleCookError::Bundle(BundleError::BadMagic))
                ),
            ),
            tag(
                "future-version",
                matches!(
                    verify_bundle_bytes(&future, false),
                    Err(BundleCookError::Bundle(BundleError::FutureVersion { found }))
                        if found == BUNDLE_VERSION + 1
                ),
            ),
            tag(
                "truncated",
                matches!(
                    verify_bundle_bytes(&first[..10], false),
                    Err(BundleCookError::Bundle(BundleError::Truncated { .. }))
                ),
            ),
            tag(
                "bundle-rejected",
                matches!(
                    verify_bundle_bytes(&flipped, false),
                    Err(BundleCookError::Bundle(_))
                ),
            ),
        ])
    }

    /// Bundle project move: write with the owner writer, carry to another
    /// folder, load there. Returns whether the bytes read back equal, and how
    /// many staged names were lost.
    fn bundle_move(
        scratch: &Path,
        staged: &StagedCooked,
        first: &[u8],
    ) -> Result<(bool, usize), ReceiptError> {
        let from = scratch.join("a").join("bundle.e204");
        let to = scratch.join("b").join("bundle.e204");
        ensure_parent(&from)?;
        write_bundle_file(first, &from, false).map_err(at("write demo bundle"))?;
        carry(&from, &to)?;
        let (moved, receipt) =
            load_bundle_file(&to, false).map_err(at("load moved bundle"))?;
        let on_disk = fs::read(&to).map_err(at("read moved bundle"))?;
        Ok((
            on_disk == first && receipt.bytes == first.len(),
            lost_refs(staged, &moved),
        ))
    }

    /// Editor save: a template-made document saves, reopens and moves equal, and
    /// a save one version above current is refused.
    fn save_facts(scratch: &Path) -> Result<SaveFacts, ReceiptError> {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let mut templates = TemplateService::new();
        templates
            .apply(&mut doc, &mut journal, TemplateKind::Arena, "arena", 1)
            .map_err(at("apply arena template"))?;
        let save = save_to_bytes(&doc);
        let from = scratch.join("a").join("project.edoc");
        let to = scratch.join("b").join("project.edoc");
        ensure_parent(&from)?;
        fs::write(&from, &save).map_err(at("write save"))?;
        carry(&from, &to)?;
        let moved = fs::read(&to).map_err(at("read moved save"))?;
        let mut future = save.clone();
        if let Some(slot) = future.get_mut(4..8) {
            slot.copy_from_slice(&(SAVE_VERSION + 1).to_le_bytes());
        }
        Ok(SaveFacts {
            stamped: save.get(4..8) == Some(&SAVE_VERSION.to_le_bytes()[..]),
            round_trip: load_from_bytes(&save).is_ok_and(|back| back == doc),
            moved_equal: load_from_bytes(&moved).is_ok_and(|back| back == doc),
            future_refused: matches!(
                load_from_bytes(&future),
                Err(SaveError::UnsupportedVersion { .. })
            ),
        })
    }

    /// Schema registry, prefab and scene: v1 loads unchanged, v2 is unknown, a
    /// two-node prefab instantiates and round-trips, a future scene is refused.
    fn schema_facts() -> Result<SchemaFacts, ReceiptError> {
        let registry = SchemaRegistry::builtin();
        let label_v1 = label("Grand Hall", 1);
        let label_v1_loads = registry
            .migrate_value(&label_v1)
            .is_ok_and(|value| value == label_v1);
        let label_v2_refused = matches!(
            registry.migrate_value(&label("Grand Hall", 2)),
            Err(SchemaError::UnknownVersion { got: 2, .. })
        );
        let prefab = Prefab {
            schema_version: PREFAB_VERSION,
            nodes: vec![
                prefab_node("hall", None, 0.0, "Grand Hall"),
                prefab_node("hall/plinth", Some("hall"), 1.0, "Plinth"),
            ],
        };
        prefab
            .validate(&registry)
            .map_err(at("validate prefab"))?;
        let mut tree = Hierarchy::new();
        let instance = prefab
            .instantiate(&mut tree, &registry)
            .map_err(at("instantiate prefab"))?;
        let back = Scene::encode(&tree)
            .decode()
            .map_err(at("decode scene"))?;
        let future_scene = Scene {
            version: CURRENT_VERSION + 1,
            nodes: Vec::new(),
        };
        Ok(SchemaFacts {
            label_v1_loads,
            label_v2_refused,
            prefab_instances: instance.map.len(),
            scene_round_trip: back == tree && tree.len() == instance.map.len(),
            scene_future_refused: matches!(
                future_scene.decode(),
                Err(CodecError::UnsupportedVersion { got, min, current })
                    if got == CURRENT_VERSION + 1
                        && min == MIN_SUPPORTED_VERSION
                        && current == CURRENT_VERSION
            ),
        })
    }

    /// One full owner pass in `scratch`. Any owner call that fails returns its
    /// error with the step name; no check is skipped.
    pub fn owner_record(scratch: &Path) -> Result<OwnerRecord, ReceiptError> {
        let _guard = SCRATCH_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        fs::create_dir_all(scratch).map_err(at("create scratch folder"))?;
        let staged = demo_staged();
        let (first, cook_stable) = cook_demo(&staged)?;
        let refusals = refusal_checks(scratch, &first)?;
        let (moved_bundle_equal, lost_bundle_refs) = bundle_move(scratch, &staged, &first)?;
        let save = save_facts(scratch)?;
        let schema = schema_facts()?;
        Ok(OwnerRecord {
            bundle_digest: hash_bytes(&first),
            bundle_bytes: first.len(),
            bundle_entries: staged.len(),
            cook_stable,
            refusals,
            moved_bundle_equal,
            lost_bundle_refs,
            save_stamped: save.stamped,
            save_round_trip: save.round_trip,
            save_moved_equal: save.moved_equal,
            save_future_refused: save.future_refused,
            label_v1_loads: schema.label_v1_loads,
            label_v2_refused: schema.label_v2_refused,
            prefab_instances: schema.prefab_instances,
            scene_round_trip: schema.scene_round_trip,
            scene_future_refused: schema.scene_future_refused,
        })
    }

    /// Report lines for one owner pass.
    pub fn owner_lines(record: &OwnerRecord) -> Vec<String> {
        vec![
            format!(
                "owner bundle entries {} bytes {} digest {:016x}",
                record.bundle_entries, record.bundle_bytes, record.bundle_digest
            ),
            format!("owner cook-stable {}", record.cook_stable),
            format!("owner refusals {}", record.refusals.join(", ")),
            format!(
                "owner project-move bundle-equal {} lost-refs {}",
                record.moved_bundle_equal, record.lost_bundle_refs
            ),
            format!(
                "owner save stamped {} round-trip {} moved-equal {} future-refused {}",
                record.save_stamped,
                record.save_round_trip,
                record.save_moved_equal,
                record.save_future_refused
            ),
            format!(
                "owner label v1-loads {} v2-refused {}",
                record.label_v1_loads, record.label_v2_refused
            ),
            format!(
                "owner prefab instances {} scene round-trip {} future-refused {}",
                record.prefab_instances, record.scene_round_trip, record.scene_future_refused
            ),
        ]
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn scratch() -> std::path::PathBuf {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/evidence/b/versions/scratch")
        }

        #[test]
        fn owner_cook_is_byte_stable_and_verifies() {
            let record = owner_record(&scratch()).expect("owner record");
            assert!(record.cook_stable, "cook bytes differ across runs");
            assert!(record.bundle_entries > 0);
            assert_eq!(record.bundle_entries, demo_staged().len());
        }

        #[test]
        fn owner_refusals_are_typed_and_named() {
            let record = owner_record(&scratch()).expect("owner record");
            assert_eq!(record.refusals.len(), 7, "{:?}", record.refusals);
            assert!(
                record
                    .refusals
                    .iter()
                    .all(|line| !line.contains("UNEXPECTED")),
                "{:?}",
                record.refusals
            );
        }

        #[test]
        fn owner_project_move_keeps_every_reference() {
            let record = owner_record(&scratch()).expect("owner record");
            assert!(record.moved_bundle_equal);
            assert_eq!(record.lost_bundle_refs, 0);
            assert!(record.save_moved_equal);
        }

        #[test]
        fn owner_schema_prefab_and_save_versions_hold() {
            let record = owner_record(&scratch()).expect("owner record");
            assert!(record.save_stamped && record.save_round_trip);
            assert!(record.save_future_refused);
            assert!(record.label_v1_loads && record.label_v2_refused);
            assert_eq!(record.prefab_instances, 2);
            assert!(record.scene_round_trip && record.scene_future_refused);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn evidence_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/evidence/b/versions")
    }

    #[test]
    fn every_pinned_row_matches_its_owner_source() {
        let mismatches = row_mismatches();
        assert!(mismatches.is_empty(), "{mismatches:?}");
    }

    #[test]
    fn no_owner_version_constant_is_unlisted() {
        let unlisted = unlisted_versions();
        assert!(unlisted.is_empty(), "unlisted owner versions: {unlisted:?}");
    }

    #[test]
    fn floors_never_exceed_their_current_version() {
        let violations = floor_violations();
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn golden_hash_is_unchanged() {
        assert_eq!(golden_checkpoint(), Ok(GOLDEN_600));
    }

    #[test]
    fn museum_tape_v1_replays_and_refuses_bad_bytes_by_name() {
        assert!(museum_replay().is_ok());
        for refusal in tape_refusals() {
            assert!(refusal.ends_with("typed"), "{refusal}");
        }
    }

    #[test]
    fn report_is_written_twice_and_reproduces() {
        let first = build_report().expect("first report");
        let second = build_report().expect("second report");
        assert_eq!(first, second);
        let prefix = if cfg!(feature = "m1") {
            "report-owner"
        } else {
            "report"
        };
        let dir = evidence_dir();
        fs::create_dir_all(&dir).expect("evidence folder");
        fs::write(dir.join(format!("{prefix}-1.txt")), &first).expect("write run 1");
        fs::write(dir.join(format!("{prefix}-2.txt")), &second).expect("write run 2");
        let one = fs::read(dir.join(format!("{prefix}-1.txt"))).expect("read run 1");
        let two = fs::read(dir.join(format!("{prefix}-2.txt"))).expect("read run 2");
        assert_eq!(one, two);
    }
}
