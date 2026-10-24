//! project: the smallest real project, `project.new` through `project.run`.
//!
//! Owner: `engine-endpoint`; consumer: an operator who wants a project folder
//! and one headless run of it in a single session, with no network and no
//! install step.
//!
//! A project is a folder holding `game.toml`:
//!
//! ```toml
//! # fracas: the smallest real project
//! name = "fracas"
//! game = "engine-game:arena"
//! fixture = 0
//! ```
//!
//! `game` is the `<package>:<bin>` key the game manifest owns; this door builds
//! only [`HEADLESS_GAME`], the engine's own arena, because that is the one game
//! a headless run boots. `fixture` names one of the packaged briefs
//! ([`fixture_briefs`]), so a new project is real the moment it is written.
//!
//! Units: whole ticks and bytes. Determinism: the build is the packaged slice's
//! own five-minute play and exact replay, so the same manifest always writes the
//! same package bytes; `project.run` rebuilds and refuses a package whose bytes
//! moved. Errors: typed [`McpToolError`], every refusal names the key or the
//! artifact and writes nothing.
//!
//! `modules` lists the packs a project wants. Every name must be one the door
//! publishes: the door resolves the list when it reads the project and reports
//! it in the build and run receipts. A pack the registry marks non-default
//! loads only when the file names it; without the key the door's default set
//! stands, exactly as before.

use std::path::{Component, Path, PathBuf};

use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::{KnownModule, PackDef, PackError, VerbSpec};
use crate::build_game::{build_fixture, fixture_briefs, PACKAGE_MAGIC};
use crate::mcp::{McpServer, McpToolError};
use crate::tool_capabilities::CapabilityCard;

/// Pack name.
pub const PACK: &str = "project";
/// Verb: write the smallest real project.
pub const TOOL_PROJECT_NEW: &str = "project.new";
/// Verb: build a project into its package.
pub const TOOL_PROJECT_BUILD: &str = "project.build";
/// Verb: run a built project headless.
pub const TOOL_PROJECT_RUN: &str = "project.run";

/// Manifest file a project folder carries.
pub const MANIFEST_FILE: &str = "game.toml";
/// Folder the package is written into, under the project.
pub const BUILD_DIR: &str = "build";
/// Package file suffix.
pub const PACKAGE_SUFFIX: &str = "e2package";
/// The one game this door builds and runs headless: the engine's arena.
pub const HEADLESS_GAME: &str = "engine-game:arena";
/// Folder `project.new` writes into when `dir` names none.
pub const DEFAULT_DIR: &str = "projects";
/// Longest project name accepted.
pub const MAX_NAME: usize = 64;

/// Typed project manifest: the keys this door owns.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ProjectManifest {
    /// Project name; plain ASCII letters, digits, `-` and `_`.
    pub name: String,
    /// The `<package>:<bin>` the project builds.
    pub game: String,
    /// Packaged brief the project builds.
    pub fixture: usize,
    /// Packs the file lists under `modules`, in declaration order, no repeats;
    /// empty when the file names none, which is the door's default set.
    pub modules: Vec<String>,
}

/// One module as the door resolved it: the pack name a project listed and
/// whether a fresh session already advertises that pack.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ProjectModule {
    /// Pack name, e.g. `geo`.
    pub name: String,
    /// True when the pack loads without `api.load`.
    pub default_loaded: bool,
}

/// `project.new` args: the name, and where to put it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ProjectNewArgs {
    /// Project name: 1..=64 of `0-9 A-Z a-z - _`.
    pub name: String,
    /// Folder under the project root; default `projects/<name>`.
    pub dir: Option<String>,
    /// Packaged brief to build, 0..=9; default 0.
    pub fixture: Option<usize>,
}

/// What `project.new` answers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ProjectNewOutput {
    /// Folder the project was written into, relative to the project root.
    pub dir: String,
    /// Manifest path, relative to the project root.
    pub manifest: String,
    /// The game the manifest names.
    pub game: String,
    /// The packaged brief the manifest names.
    pub fixture: usize,
}

/// `project.build` and `project.run` args: the folder `project.new` wrote.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ProjectDirArgs {
    /// Project folder under the project root.
    pub dir: String,
}

/// What `project.build` answers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ProjectBuildOutput {
    /// Project folder, relative to the project root.
    pub dir: String,
    /// Package path, relative to the project root.
    pub package: String,
    /// Package bytes written.
    pub bytes: u64,
    /// FNV-1a over the package bytes, 16 hex digits.
    pub digest: String,
    /// The five-minute play's integer hash, 16 hex digits.
    pub play_digest: String,
    /// Checkpoints the play folded.
    pub checkpoints: u32,
    /// Packs the manifest listed, resolved against the registry; empty when it
    /// lists none, which is the door's default set.
    pub modules: Vec<ProjectModule>,
}

/// What `project.run` answers: the run, and the package it ran.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ProjectRunOutput {
    /// Project folder, relative to the project root.
    pub dir: String,
    /// Package path, relative to the project root.
    pub package: String,
    /// Package bytes read.
    pub bytes: u64,
    /// FNV-1a over the package bytes, 16 hex digits.
    pub digest: String,
    /// The play's integer hash, 16 hex digits.
    pub play_digest: String,
    /// Checkpoints the play folded.
    pub checkpoints: u32,
    /// True when the rebuild replayed the play exactly; a mismatch refuses.
    pub replay_matched: bool,
    /// Packs the manifest listed, resolved against the registry; empty when it
    /// lists none, which is the door's default set.
    pub modules: Vec<ProjectModule>,
}

/// Parse the three keys this door owns; anything else refuses by key name.
fn parse_manifest(text: &str) -> Result<ProjectManifest, McpToolError> {
    let mut manifest = ProjectManifest {
        game: HEADLESS_GAME.to_owned(),
        ..ProjectManifest::default()
    };
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| {
            McpToolError::new(
                "bad_manifest",
                format!(
                    "{MANIFEST_FILE} line {} is not `key = value`; recovery: write name, game and \
                     fixture one per line",
                    index + 1
                ),
            )
        })?;
        let key = key.trim();
        let raw = value.trim();
        // A module list is the one list key here, so it keeps its brackets.
        if key == "modules" {
            manifest.modules = parse_modules(raw)?;
            continue;
        }
        let value = raw.trim_matches('"');
        match key {
            "name" => manifest.name = value.to_owned(),
            "game" => manifest.game = value.to_owned(),
            "fixture" => {
                manifest.fixture = value.parse().map_err(|_| {
                    McpToolError::new(
                        "bad_manifest",
                        format!(
                            "fixture `{value}` is not a whole number; recovery: name one of 0..={}",
                            fixture_briefs().len() - 1
                        ),
                    )
                })?;
            }
            other => {
                return Err(McpToolError::new(
                    "bad_manifest",
                    format!(
                        "{MANIFEST_FILE} line {} sets `{other}`, which this door does not read; \
                         recovery: keep name, game, fixture and modules",
                        index + 1
                    ),
                ));
            }
        }
    }
    if manifest.name.is_empty()
        || !manifest
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        || manifest.name.len() > MAX_NAME
    {
        return Err(McpToolError::new(
            "bad_name",
            format!(
                "`{}` is not a project name; recovery: 1..={MAX_NAME} of ASCII letters, digits, \
                 `-` or `_`",
                manifest.name
            ),
        ));
    }
    if manifest.fixture >= fixture_briefs().len() {
        return Err(McpToolError::new(
            "bad_manifest",
            format!(
                "fixture {} is unknown; recovery: name one of 0..={}",
                manifest.fixture,
                fixture_briefs().len() - 1
            ),
        ));
    }
    Ok(manifest)
}

/// The `modules` value: `["geo", "scene"]` on one line, names in declaration
/// order, repeats dropped, a trailing comma fine. Resolution against the pack
/// registry happens in [`read_project`].
fn parse_modules(value: &str) -> Result<Vec<String>, McpToolError> {
    let inner = value
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .ok_or_else(|| {
            McpToolError::new(
                "bad_manifest",
                format!(
                    "modules `{value}` is not a list; recovery: write \
                     modules = [\"scene\", \"geo\"]"
                ),
            )
        })?;
    let mut modules: Vec<String> = Vec::new();
    for item in inner.split(',') {
        let name = item.trim();
        if name.is_empty() {
            continue; // a trailing comma is fine
        }
        let name = name.trim_matches('"');
        if name.is_empty() {
            return Err(McpToolError::new(
                "bad_manifest",
                "a module name is not empty; recovery: name packs the door lists",
            ));
        }
        let name = name.to_owned();
        if !modules.contains(&name) {
            modules.push(name);
        }
    }
    Ok(modules)
}

/// The manifest text `project.new` writes.
fn manifest_text(name: &str, fixture: usize) -> String {
    format!(
        "# {name}: the smallest real project. project.build reads it, project.run boots it.\n\
         name = \"{name}\"\ngame = \"{HEADLESS_GAME}\"\nfixture = {fixture}\n"
    )
}

/// A relative path that stays inside the project root, or a refusal.
fn checked_rel(rel: &str) -> Result<PathBuf, McpToolError> {
    let path = Path::new(rel);
    if rel.is_empty() || path.is_absolute() {
        return Err(McpToolError::new(
            "bad_path",
            format!("`{rel}` is not a relative folder under the project root"),
        ));
    }
    for part in path.components() {
        if !matches!(part, Component::Normal(_)) {
            return Err(McpToolError::new(
                "bad_path",
                format!("`{rel}` leaves the project root; recovery: name a folder inside it"),
            ));
        }
    }
    Ok(path.to_path_buf())
}

/// A project read from disk: its keys, and the registry rows its `modules` list names.
struct LoadedProject {
    manifest: ProjectManifest,
    modules: Vec<KnownModule>,
}

/// Read and parse `<root>/<dir>/game.toml`, then resolve its modules.
fn read_project(root: &Path, dir: &str) -> Result<LoadedProject, McpToolError> {
    let base = checked_rel(dir)?;
    let path = root.join(&base).join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&path).map_err(|error| {
        McpToolError::new(
            "no_project",
            format!("`{dir}` holds no {MANIFEST_FILE} ({error}); recovery: call project.new first"),
        )
    })?;
    let manifest = parse_manifest(&text)?;
    let modules = super::resolve_modules(&manifest.modules)?;
    Ok(LoadedProject { manifest, modules })
}

/// The resolved rows as a receipt reports them.
fn project_modules(modules: &[KnownModule]) -> Vec<ProjectModule> {
    modules
        .iter()
        .map(|module| ProjectModule {
            name: module.name.to_owned(),
            default_loaded: module.default_loaded,
        })
        .collect()
}

/// Package path of one project, relative to the project root.
fn package_rel(dir: &str, name: &str) -> String {
    format!("{dir}/{BUILD_DIR}/{name}.{PACKAGE_SUFFIX}")
}

/// Build the manifest's packaged brief: the five-minute play and its exact replay.
fn build_project(
    manifest: &ProjectManifest,
) -> Result<crate::build_game::PlayableSlice, McpToolError> {
    if manifest.game != HEADLESS_GAME {
        return Err(McpToolError::new(
            "unsupported_game",
            format!(
                "game `{}` is not {HEADLESS_GAME}; recovery: this door builds and runs only the \
                 engine arena headless",
                manifest.game
            ),
        ));
    }
    build_fixture(manifest.fixture).map_err(|error| {
        McpToolError::new(error.code, format!("{}: {}", error.code, error.message))
    })
}

/// `project.new`: write the folder and its manifest.
fn project_new(root: &Path, args: &ProjectNewArgs) -> Result<ProjectNewOutput, McpToolError> {
    let fixture = args.fixture.unwrap_or(0);
    if fixture >= fixture_briefs().len() {
        return Err(McpToolError::new(
            "bad_manifest",
            format!(
                "fixture {fixture} is unknown; recovery: name one of 0..={}",
                fixture_briefs().len() - 1
            ),
        ));
    }
    let dir = match args.dir.as_deref() {
        Some(dir) => dir.to_owned(),
        None => format!("{DEFAULT_DIR}/{}", args.name),
    };
    let base = checked_rel(&dir)?;
    let name = args.name.as_str();
    if name.is_empty()
        || name.len() > MAX_NAME
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(McpToolError::new(
            "bad_name",
            format!(
                "`{name}` is not a project name; recovery: 1..={MAX_NAME} of ASCII letters, \
                 digits, `-` or `_`"
            ),
        ));
    }
    let folder = root.join(&base);
    let manifest_path = folder.join(MANIFEST_FILE);
    if manifest_path.exists() {
        return Err(McpToolError::new(
            "project_exists",
            format!("`{dir}` already holds a {MANIFEST_FILE}; recovery: name another folder"),
        ));
    }
    std::fs::create_dir_all(&folder).map_err(|error| {
        McpToolError::new(
            "write_failed",
            format!("could not create `{dir}` ({error}); recovery: check the project root"),
        )
    })?;
    let text = manifest_text(name, fixture);
    std::fs::write(&manifest_path, text.as_bytes()).map_err(|error| {
        McpToolError::new(
            "write_failed",
            format!("could not write `{dir}/{MANIFEST_FILE}` ({error})"),
        )
    })?;
    Ok(ProjectNewOutput {
        manifest: format!("{dir}/{MANIFEST_FILE}"),
        dir,
        game: HEADLESS_GAME.to_owned(),
        fixture,
    })
}

/// `project.build`: build the manifest and write its package.
fn project_build(root: &Path, args: &ProjectDirArgs) -> Result<ProjectBuildOutput, McpToolError> {
    let loaded = read_project(root, &args.dir)?;
    let manifest = &loaded.manifest;
    let slice = build_project(manifest)?;
    let bytes = slice.package_bytes();
    if bytes.len() > crate::build_game::PACKAGE_MAX_BYTES {
        return Err(McpToolError::new(
            "package_too_large",
            format!(
                "the package is {} bytes; at most {}",
                bytes.len(),
                crate::build_game::PACKAGE_MAX_BYTES
            ),
        ));
    }
    let package = package_rel(&args.dir, &manifest.name);
    let path = root.join(&package);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            McpToolError::new(
                "write_failed",
                format!("could not create `{BUILD_DIR}` ({error})"),
            )
        })?;
    }
    std::fs::write(&path, &bytes).map_err(|error| {
        McpToolError::new(
            "write_failed",
            format!("could not write `{package}` ({error})"),
        )
    })?;
    Ok(ProjectBuildOutput {
        dir: args.dir.clone(),
        package,
        bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        digest: format!("{:016x}", engine_core::hash_bytes(&bytes)),
        play_digest: format!("{:016x}", slice.play_digest),
        checkpoints: slice.eval.checkpoints,
        modules: project_modules(&loaded.modules),
    })
}

/// `project.run`: read the package, rebuild the project and play it headless.
fn project_run(root: &Path, args: &ProjectDirArgs) -> Result<ProjectRunOutput, McpToolError> {
    let loaded = read_project(root, &args.dir)?;
    let manifest = &loaded.manifest;
    let package = package_rel(&args.dir, &manifest.name);
    let bytes = std::fs::read(root.join(&package)).map_err(|error| {
        McpToolError::new(
            "not_built",
            format!("`{package}` is missing ({error}); recovery: call project.build first"),
        )
    })?;
    if bytes.len() < PACKAGE_MAGIC.len() || bytes[..PACKAGE_MAGIC.len()] != PACKAGE_MAGIC {
        return Err(McpToolError::new(
            "bad_package",
            format!("`{package}` does not start with the package magic; recovery: rebuild it"),
        ));
    }
    let slice = build_project(&manifest)?;
    let rebuilt = slice.package_bytes();
    if rebuilt != bytes {
        return Err(McpToolError::new(
            "stale_package",
            format!(
                "`{package}` is not what this project builds now; recovery: call project.build \
                 again"
            ),
        ));
    }
    Ok(ProjectRunOutput {
        dir: args.dir.clone(),
        package,
        bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        digest: format!("{:016x}", engine_core::hash_bytes(&bytes)),
        play_digest: format!("{:016x}", slice.play_digest),
        checkpoints: slice.eval.checkpoints,
        // `build_project` refuses a play whose replay diverges, so a slice here
        // replayed exactly.
        replay_matched: true,
        modules: project_modules(&loaded.modules),
    })
}

/// `project.new` handler: the project root the door was built in.
fn project_new_verb(
    _server: &McpServer,
    args: &ProjectNewArgs,
) -> Result<ProjectNewOutput, McpToolError> {
    project_new(&super::codex::project_root(None), args)
}

/// `project.build` handler: the project root the door was built in.
fn project_build_verb(
    _server: &McpServer,
    args: &ProjectDirArgs,
) -> Result<ProjectBuildOutput, McpToolError> {
    project_build(&super::codex::project_root(None), args)
}

/// `project.run` handler: the project root the door was built in.
fn project_run_verb(
    _server: &McpServer,
    args: &ProjectDirArgs,
) -> Result<ProjectRunOutput, McpToolError> {
    project_run(&super::codex::project_root(None), args)
}

crate::e2_tool!(
    /// `project.new`: write the smallest real project folder.
    ProjectNewTool,
    name = TOOL_PROJECT_NEW,
    description = "Write the smallest real project: a folder with `game.toml` naming the project, the game (`engine-game:arena`) and one packaged brief. Nothing else and no network: the folder builds and runs headless straight away. Refuses an existing folder, so a second new names another one.",
    readonly = false,
    args = ProjectNewArgs,
    output = ProjectNewOutput,
    invoke = project_new_verb
);

crate::e2_tool!(
    /// `project.build`: build the project into its package.
    ProjectBuildTool,
    name = TOOL_PROJECT_BUILD,
    description = "Build a project folder: read `game.toml`, build its packaged brief (the five-minute play plus its exact replay) and write `build/<name>.e2package`, answering the path, bytes, package digest and play digest. The same manifest always writes the same bytes. Refuses a game this door cannot build headless.",
    readonly = false,
    args = ProjectDirArgs,
    output = ProjectBuildOutput,
    invoke = project_build_verb
);

crate::e2_tool!(
    /// `project.run`: run the built package headless.
    ProjectRunTool,
    name = TOOL_PROJECT_RUN,
    description = "Run a built project headless: read `build/<name>.e2package`, rebuild the project, refuse if the package is stale, and answer the play digest, the checkpoints folded and whether the replay matched. No window, no network, no install step.",
    readonly = true,
    args = ProjectDirArgs,
    output = ProjectRunOutput,
    invoke = project_run_verb
);

/// The shipped project pack: three verbs, new and build commit, run observes.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "projects: project.new writes the smallest real project folder and its game.toml, \
         project.build builds it into its package, project.run plays the package headless",
        false,
    );
    pack.add(VerbSpec::of::<ProjectNewTool>(
        TOOL_PROJECT_NEW,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<ProjectBuildTool>(
        TOOL_PROJECT_BUILD,
        PACK,
        CapabilityCard::Commit,
    )?);
    pack.add(VerbSpec::of::<ProjectRunTool>(
        TOOL_PROJECT_RUN,
        PACK,
        CapabilityCard::Observe,
    )?);
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_editor::capability::Capability;

    fn server() -> McpServer {
        McpServer::new(7, Capability::unrestricted(7))
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("e2040-project-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// F2P: the three verbs are new; the round trip is the accept.
    #[test]
    fn new_build_run_round_trips_one_session() {
        let root = scratch("round-trip");
        let created = project_new(
            &root,
            &ProjectNewArgs {
                name: "fracas".to_owned(),
                dir: None,
                fixture: None,
            },
        )
        .expect("the project is written");
        assert_eq!(created.dir, "projects/fracas");
        assert_eq!(created.game, HEADLESS_GAME);
        assert_eq!(created.manifest, "projects/fracas/game.toml");
        let text = std::fs::read_to_string(root.join(&created.manifest)).expect("manifest lands");
        assert!(text.contains("name = \"fracas\""), "{text}");
        assert!(text.contains("game = \"engine-game:arena\""), "{text}");
        let built = project_build(
            &root,
            &ProjectDirArgs {
                dir: created.dir.clone(),
            },
        )
        .expect("the project builds");
        assert_eq!(built.package, "projects/fracas/build/fracas.e2package");
        assert!(built.bytes > 0);
        assert!(built.modules.is_empty(), "a plain manifest names no module");
        assert_eq!(built.digest.len(), 16);
        assert_eq!(built.play_digest.len(), 16);
        let bytes = std::fs::read(root.join(&built.package)).expect("package lands");
        assert_eq!(&bytes[..PACKAGE_MAGIC.len()], PACKAGE_MAGIC);
        assert_eq!(
            u64::try_from(bytes.len()).expect("fits"),
            built.bytes,
            "the receipt counts the bytes it wrote"
        );
        let ran = project_run(
            &root,
            &ProjectDirArgs {
                dir: created.dir.clone(),
            },
        )
        .expect("the project runs");
        assert_eq!(
            ran.digest, built.digest,
            "the run reads the package it built"
        );
        assert_eq!(ran.play_digest, built.play_digest);
        assert_eq!(ran.bytes, built.bytes);
        assert!(ran.replay_matched);
        // The build is deterministic: a second build writes the same bytes.
        let again = project_build(
            &root,
            &ProjectDirArgs {
                dir: created.dir.clone(),
            },
        )
        .expect("builds twice");
        assert_eq!(again.digest, built.digest);
    }

    #[test]
    fn refusals_name_the_key_or_the_artifact() {
        let root = scratch("refusals");
        let dir = ProjectDirArgs {
            dir: "projects/gone".to_owned(),
        };
        assert_eq!(
            project_build(&root, &dir).expect_err("no folder").code,
            "no_project"
        );
        assert_eq!(
            project_run(&root, &dir).expect_err("no folder").code,
            "no_project"
        );
        let created = project_new(
            &root,
            &ProjectNewArgs {
                name: "twice".to_owned(),
                dir: Some("projects/twice".to_owned()),
                fixture: Some(0),
            },
        )
        .expect("writes");
        let twice = project_new(
            &root,
            &ProjectNewArgs {
                name: "twice".to_owned(),
                dir: Some("projects/twice".to_owned()),
                fixture: None,
            },
        )
        .expect_err("the folder exists");
        assert_eq!(twice.code, "project_exists");
        let run_first = project_run(
            &root,
            &ProjectDirArgs {
                dir: created.dir.clone(),
            },
        )
        .expect_err("not built yet");
        assert_eq!(run_first.code, "not_built");
        // A game this door cannot build headless refuses by the key it read.
        std::fs::write(
            root.join(&created.manifest),
            "name = \"twice\"\ngame = \"kam2:game\"\nfixture = 0\n",
        )
        .expect("manifest rewritten");
        let other = project_build(
            &root,
            &ProjectDirArgs {
                dir: created.dir.clone(),
            },
        )
        .expect_err("another game");
        assert_eq!(other.code, "unsupported_game");
        assert!(other.message.contains("kam2:game"), "{}", other.message);
        // An unknown key names itself.
        std::fs::write(
            root.join(&created.manifest),
            "name = \"twice\"\ngame = \"engine-game:arena\"\ncolour = \"red\"\n",
        )
        .expect("manifest rewritten");
        let unknown = project_build(
            &root,
            &ProjectDirArgs {
                dir: created.dir.clone(),
            },
        )
        .expect_err("unknown key");
        assert_eq!(unknown.code, "bad_manifest");
        assert!(unknown.message.contains("colour"), "{}", unknown.message);
        // A folder that leaves the root refuses.
        let outside = project_build(
            &root,
            &ProjectDirArgs {
                dir: "../elsewhere".to_owned(),
            },
        )
        .expect_err("outside the root");
        assert_eq!(outside.code, "bad_path");
    }

    /// F2P on the door side: `modules` was an unknown key before WG2.
    #[test]
    fn modules_resolve_against_the_registry_and_name_the_unknown_one() {
        let text = "name = \"mods\"\ngame = \"engine-game:arena\"\nfixture = 0\n\
                    modules = [\"geo\", \"scene\", \"geo\"]\n";
        let manifest = parse_manifest(text).expect("modules parse");
        assert_eq!(
            manifest.modules,
            ["geo", "scene"],
            "declaration order, no repeats"
        );
        let resolved = crate::packs::resolve_modules(&manifest.modules).expect("both are packs");
        assert_eq!(
            project_modules(&resolved),
            vec![
                ProjectModule {
                    name: "geo".to_owned(),
                    default_loaded: false,
                },
                ProjectModule {
                    name: "scene".to_owned(),
                    default_loaded: true,
                },
            ]
        );
        // An unknown name refuses by code and lists every known pack, sorted,
        // with whether a fresh session already advertises it.
        let unknown =
            crate::packs::resolve_modules(&["shell".to_owned()]).expect_err("shell is no pack");
        assert_eq!(unknown.code, "unknown_module");
        assert!(unknown.message.contains("`shell`"), "{}", unknown.message);
        assert!(
            unknown.message.contains("scene (default)"),
            "{}",
            unknown.message
        );
        assert!(
            unknown.message.contains("geo (on demand)"),
            "{}",
            unknown.message
        );
        let names: Vec<&str> = crate::packs::known_modules()
            .iter()
            .map(|module| module.name)
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "the refusal lists the known names sorted");
        // The receipts report the resolved list.
        let root = scratch("modules");
        let created = project_new(
            &root,
            &ProjectNewArgs {
                name: "mods".to_owned(),
                dir: None,
                fixture: None,
            },
        )
        .expect("writes");
        let path = root.join(&created.manifest);
        std::fs::write(&path, text).expect("manifest with modules");
        let built = project_build(
            &root,
            &ProjectDirArgs {
                dir: created.dir.clone(),
            },
        )
        .expect("builds");
        assert_eq!(built.modules, project_modules(&resolved));
        // A value that is not a list refuses by its own key, before any build.
        std::fs::write(
            &path,
            "name = \"mods\"\ngame = \"engine-game:arena\"\nfixture = 0\nmodules = \"geo\"\n",
        )
        .expect("manifest rewritten");
        let bad = project_build(
            &root,
            &ProjectDirArgs {
                dir: created.dir.clone(),
            },
        )
        .expect_err("not a list");
        assert_eq!(bad.code, "bad_manifest");
        assert!(bad.message.contains("modules"), "{}", bad.message);
        // No key is today's answer: no module named, the default set stands.
        let plain = parse_manifest("name = \"mods\"\nfixture = 0\n").expect("parses");
        assert!(plain.modules.is_empty());
    }

    #[test]
    fn pack_registers_three_verbs_with_their_cards() {
        let pack = pack().expect("registers");
        assert_eq!(pack.name(), PACK);
        assert_eq!(
            pack.verb_names(),
            vec![TOOL_PROJECT_NEW, TOOL_PROJECT_BUILD, TOOL_PROJECT_RUN]
        );
        assert!(!pack.default_loaded());
        for name in [TOOL_PROJECT_NEW, TOOL_PROJECT_BUILD] {
            let verb = pack
                .verbs()
                .iter()
                .find(|verb| verb.name() == name)
                .expect("registered");
            assert_eq!(verb.card(), CapabilityCard::Commit, "{name}");
        }
        let server = server();
        let _ = &server;
    }
}

// WG2: this pack publishes its own row; `packs/mod.rs` keeps no hand list.
#[expect(
    unsafe_code,
    reason = "linkme publishes a pack row through a link section"
)]
#[linkme::distributed_slice(super::PACK_MODULES)]
static PACK_ROW: super::PackModule = super::PackModule {
    order: 14,
    name: PACK,
    default_loaded: false,
    tools: &["project.new", "project.build", "project.run"],
    build: pack,
};
