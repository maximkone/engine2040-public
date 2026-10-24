//! The game-project verbs of the `engine2040` bin: `new` makes a project directory from a
//! template, `run --headless` plays its seeded proof or runs the game it names, and `build`
//! (in `build.rs`) makes that game. A project is a directory with a `game.toml` (see
//! `manifest.rs`); the game it builds and runs is one bin of the engine workspace
//! (`workspace.rs`), `engine-game:arena` unless the file says otherwise.

use crate::cli::{usage, write_text_receipt, Error};
use crate::manifest::{check_name, lookup, unwired_note, valid_words, Manifest, Target};
use crate::process::{self, Checkout, Runner, Step};
use crate::workspace::{self, GameBin};
use engine_core::hash::{hex16, Fnv64};
use engine_proof::{record, replay, Config, MAX_TICKS};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

/// The manifest name a verb reads when `--manifest` is not given.
const DEFAULT_MANIFEST: &str = "game.toml";
/// Where `new` writes the project name inside template files.
const NAME_PLACEHOLDER: &str = "{{name}}";

/// A starter project: its files, relative to the project directory.
struct Template {
    name: &'static str,
    files: &'static [(&'static str, &'static str)],
}

/// Every template, embedded so the bin works from any directory. `docs/templates/<name>/` holds
/// the same files on disk (a test keeps the two equal).
static TEMPLATES: [Template; 2] = [
    Template {
        name: "arena-duel",
        files: &[
            (
                "game.toml",
                include_str!("../docs/templates/arena-duel/game.toml"),
            ),
            (
                "README.md",
                include_str!("../docs/templates/arena-duel/README.md"),
            ),
            (
                "content/seed.txt",
                include_str!("../docs/templates/arena-duel/content/seed.txt"),
            ),
        ],
    },
    Template {
        name: "siege",
        files: &[
            (
                "game.toml",
                include_str!("../docs/templates/siege/game.toml"),
            ),
            (
                "README.md",
                include_str!("../docs/templates/siege/README.md"),
            ),
            (
                "content/seed.txt",
                include_str!("../docs/templates/siege/content/seed.txt"),
            ),
        ],
    },
];

fn template_names() -> String {
    let names: Vec<&str> = TEMPLATES.iter().map(|template| template.name).collect();
    names.join(", ")
}

/// A project verb of the command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    /// `new <template> <dir>`
    New,
    /// `run --headless`
    Run,
    /// `build --target <windows|web>`
    Build,
}

impl Verb {
    /// The verb a first command-line word names; any other word is left to the golden CLI.
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "new" => Some(Self::New),
            "run" => Some(Self::Run),
            "build" => Some(Self::Build),
            _ => None,
        }
    }
}

/// Run one project verb; `out` receives everything the verb prints.
pub fn execute(
    verb: Verb,
    args: impl Iterator<Item = OsString>,
    out: &mut dyn Write,
) -> Result<(), Error> {
    match verb {
        Verb::New => new_verb(args, out),
        Verb::Run => run_verb(args, out),
        Verb::Build => crate::build::build_verb(args, out),
    }
}

/// One verb's command line: `--name value` pairs, bare `--switch`es and plain words.
pub struct Flags {
    /// Words that are not options, in order (paths may be native, not UTF-8).
    pub positional: Vec<OsString>,
    values: Vec<(&'static str, OsString)>,
    switches: Vec<&'static str>,
}

impl Flags {
    /// Split `args` by the flags this verb knows; anything else starting with `--` is refused.
    pub fn parse(
        mut args: impl Iterator<Item = OsString>,
        switches: &[&'static str],
        valued: &[&'static str],
    ) -> Result<Self, Error> {
        let mut flags = Self {
            positional: Vec::new(),
            values: Vec::new(),
            switches: Vec::new(),
        };
        while let Some(arg) = args.next() {
            // Options are ASCII, so a word that is not UTF-8 is a path.
            let word = arg.to_str().unwrap_or("");
            if let Some(&name) = switches.iter().find(|name| **name == word) {
                if flags.switches.contains(&name) {
                    return Err(usage(format!("duplicate option {name}")));
                }
                flags.switches.push(name);
            } else if let Some(&name) = valued.iter().find(|name| **name == word) {
                if flags.values.iter().any(|(seen, _)| *seen == name) {
                    return Err(usage(format!("duplicate option {name}")));
                }
                let value = args
                    .next()
                    .ok_or_else(|| usage(format!("missing value for {name}")))?;
                flags.values.push((name, value));
            } else if word.starts_with("--") {
                return Err(usage(format!("unknown option {word}")));
            } else {
                flags.positional.push(arg);
            }
        }
        Ok(flags)
    }

    /// Whether the bare switch was given.
    pub fn switch(&self, name: &str) -> bool {
        self.switches.contains(&name)
    }

    /// The raw value given to a valued option.
    pub fn value(&self, name: &str) -> Option<&OsString> {
        self.values
            .iter()
            .find(|(seen, _)| *seen == name)
            .map(|(_, value)| value)
    }

    /// The UTF-8 text of an option's value.
    pub fn text(&self, name: &str) -> Result<Option<&str>, Error> {
        self.value(name)
            .map(|value| {
                value
                    .to_str()
                    .ok_or_else(|| usage(format!("{name} must be UTF-8")))
            })
            .transpose()
    }

    /// `--manifest`, or `./game.toml`.
    pub fn manifest(&self) -> PathBuf {
        self.value("--manifest")
            .map_or_else(|| PathBuf::from(DEFAULT_MANIFEST), PathBuf::from)
    }

    /// Refuse words that no option of this verb takes.
    pub fn no_positional(&self, verb: &str) -> Result<(), Error> {
        match self.positional.first() {
            Some(word) => Err(usage(format!(
                "{verb} takes no argument `{}`",
                word.to_string_lossy()
            ))),
            None => Ok(()),
        }
    }
}

fn new_verb(args: impl Iterator<Item = OsString>, out: &mut dyn Write) -> Result<(), Error> {
    let flags = Flags::parse(args, &[], &[])?;
    let [template, dir] = flags.positional.as_slice() else {
        return Err(usage(
            "new takes a template and a directory: new <template> <dir>",
        ));
    };
    let template = template
        .to_str()
        .ok_or_else(|| usage("the template name must be UTF-8"))?;
    create_project(template, Path::new(dir), out)
}

/// Write the template's files into `dir`. Nothing is written unless `dir` is missing or empty
/// and the rendered `game.toml` parses; an existing file is never replaced.
fn create_project(template_name: &str, dir: &Path, out: &mut dyn Write) -> Result<(), Error> {
    let template = TEMPLATES
        .iter()
        .find(|template| template.name == template_name)
        .ok_or_else(|| {
            usage(format!(
                "unknown template `{template_name}`; valid templates: {}",
                template_names()
            ))
        })?;
    let name = project_name(dir)?;
    let files: Vec<(&str, String)> = template
        .files
        .iter()
        .map(|(path, text)| (*path, text.replace(NAME_PLACEHOLDER, &name)))
        .collect();
    let manifest = files
        .iter()
        .find(|(path, _)| *path == DEFAULT_MANIFEST)
        .ok_or_else(|| {
            Error::Project(format!(
                "template {template_name} has no {DEFAULT_MANIFEST}"
            ))
        })?;
    Manifest::parse(&manifest.1).map_err(|error| {
        Error::Project(format!(
            "template {template_name} is broken: {DEFAULT_MANIFEST}: {error}"
        ))
    })?;
    require_empty(dir)?;
    for (path, text) in &files {
        let target = dir.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| io_error("create", parent, &error))?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| io_error("create", &target, &error))?;
        file.write_all(text.as_bytes())
            .map_err(|error| io_error("write", &target, &error))?;
    }
    writeln!(
        out,
        "created project `{name}` from template {template_name} in {} ({} files)",
        dir.display(),
        files.len()
    )?;
    writeln!(
        out,
        "next: cd {} and run: engine2040 run --headless --ticks 600",
        dir.display()
    )?;
    Ok(())
}

fn io_error(verb: &str, path: &Path, error: &std::io::Error) -> Error {
    Error::Project(format!("cannot {verb} {}: {error}", path.display()))
}

/// The project name is the directory's own name, so it has to be a legal name.
fn project_name(dir: &Path) -> Result<String, Error> {
    let absolute = std::path::absolute(dir).map_err(|error| io_error("resolve", dir, &error))?;
    let name = absolute
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| usage(format!("cannot name a project after `{}`", dir.display())))?;
    check_name(name).map_err(|why| {
        usage(format!(
            "the directory name `{name}` cannot be a project name ({why}); choose another directory"
        ))
    })?;
    Ok(name.to_owned())
}

/// A missing directory is fine; an existing one must hold nothing.
fn require_empty(dir: &Path) -> Result<(), Error> {
    match fs::read_dir(dir) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(Error::Project(format!(
                    "{} is not empty: new never overwrites; pick a new directory",
                    dir.display()
                )));
            }
            Ok(())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("read", dir, &error)),
    }
}

/// A project on disk: its parsed manifest and the directory it sits in.
#[derive(Debug)]
pub struct Project {
    /// The parsed `game.toml`.
    pub manifest: Manifest,
    dir: PathBuf,
    manifest_path: PathBuf,
    /// The declared game bin; `None` means the file names no game, so the engine's arena (for
    /// `build`) and the proof fixture (for `run`) apply.
    game: Option<GameBin>,
}

impl Project {
    /// Read `game.toml`, resolve the game it names against a chosen engine checkout (`root`
    /// holds the workspace `Cargo.toml`), and refuse a key that no build or run reads yet. A
    /// project that names no game never reads the workspace, so an old project loads as it
    /// always did.
    pub fn load_in(manifest_path: &Path, root: &Path) -> Result<Self, Error> {
        let text = fs::read_to_string(manifest_path).map_err(|error| {
            Error::Project(format!(
                "cannot read {}: {error}; make a project with `engine2040 new arena-duel <dir>` or pass --manifest <path>",
                manifest_path.display()
            ))
        })?;
        let manifest = Manifest::parse(&text)
            .map_err(|error| Error::Project(format!("{}: {error}", manifest_path.display())))?;
        let dir = manifest_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let game = if manifest.sets("game") {
            Some(
                workspace::resolve(root, &manifest.game.package, &manifest.game.bin).map_err(
                    |why| {
                        Error::Project(format!(
                            "{}: key `game`: {why}; write game = \"<package>:<bin>\"",
                            manifest_path.display()
                        ))
                    },
                )?,
            )
        } else {
            None
        };
        for content_root in &manifest.content {
            if !dir.join(content_root).is_dir() {
                return Err(Error::Project(format!(
                    "{}: key `content`: root `{content_root}` is not a directory (looked for {})",
                    manifest_path.display(),
                    dir.join(content_root).display()
                )));
            }
        }
        refuse_unwired(manifest_path, &manifest)?;
        Ok(Self {
            manifest,
            dir,
            manifest_path: manifest_path.to_path_buf(),
            game,
        })
    }

    /// The bin this project builds and runs: the game it names, else the engine's arena.
    pub fn game(&self) -> GameBin {
        self.game.clone().unwrap_or_else(workspace::default_game)
    }

    /// The project directory: where `dist/` and the content roots live.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where `build` reads and writes this project's lock: `project.lock` beside the manifest.
    pub fn lock_path(&self) -> PathBuf {
        self.dir.join(LOCK_FILE)
    }

    /// The crates this project builds against, read from the engine checkout. A package here
    /// is only a Cargo crate in the one workspace: no store and no fetch, so the manifest
    /// names the only crate a project resolves — the game it builds.
    pub fn dependency_rows(&self, root: &Path) -> Result<Vec<LockRow>, Error> {
        let game = self.game();
        let crate_dir = find_crate(root, &game.package)?;
        Ok(vec![lock_row(root, &game.package, &crate_dir)?])
    }

    /// Whether `game.toml` names a game, so `run` runs that bin instead of the proof fixture.
    pub fn names_game(&self) -> bool {
        self.game.is_some()
    }

    /// The run seed: the number in `seed_file`, or the engine default without one.
    fn seed(&self) -> Result<u64, Error> {
        let Some(relative) = &self.manifest.seed_file else {
            return Ok(Config::default().seed);
        };
        let path = self.dir.join(relative);
        let text = fs::read_to_string(&path).map_err(|error| {
            Error::Project(format!(
                "{}: key `seed_file`: cannot read {}: {error}",
                self.manifest_path.display(),
                path.display()
            ))
        })?;
        parse_seed(&path, &text)
    }
}

/// The lock file `build` writes beside the manifest.
pub const LOCK_FILE: &str = "project.lock";

/// One crate a project builds against, as `project.lock` records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockRow {
    /// Cargo package name.
    pub name: String,
    /// The version the crate's manifest declares.
    pub version: String,
    /// Where the crate came from: `crate:<path from the engine checkout, forward slashes>`.
    pub source: String,
    /// 16 hex digits over the crate directory's files, their relative paths included.
    pub hash: String,
}

impl LockRow {
    /// The lock line: `name version source hash`.
    pub fn line(&self) -> String {
        format!(
            "{} {} {} {}",
            self.name, self.version, self.source, self.hash
        )
    }
}

/// The crate directory of `package` in a checkout: the checkout root or one `crates/` member.
fn find_crate(root: &Path, package: &str) -> Result<PathBuf, Error> {
    let mut dirs = vec![root.to_path_buf()];
    if let Ok(entries) = fs::read_dir(root.join("crates")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            }
        }
    }
    for dir in dirs {
        let cargo = dir.join("Cargo.toml");
        let Ok(text) = fs::read_to_string(&cargo) else {
            continue;
        };
        if package_name(&text).as_deref() == Some(package) {
            return Ok(dir);
        }
    }
    Err(Error::Project(format!(
        "the engine checkout {} has no crate `{package}`; build runs in the checkout this exe was built from",
        root.display()
    )))
}

/// The `[package] name` of a crate manifest.
fn package_name(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some(rest) = line.strip_prefix("name") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        return Some(rest.trim().trim_matches('"').to_owned());
    }
    None
}

/// The version a crate manifest declares, or the workspace's when the crate inherits it.
fn crate_version(root: &Path, text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = line.strip_prefix("version") {
            if let Some(rest) = rest.trim_start().strip_prefix('=') {
                return Some(rest.trim().trim_matches('"').to_owned());
            }
        }
        if line.starts_with("version.workspace") {
            return workspace_version(root);
        }
    }
    None
}

/// The `[workspace.package] version` of the checkout root manifest.
fn workspace_version(root: &Path) -> Option<String> {
    let text = fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[workspace.package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = line.strip_prefix("version") {
            if let Some(rest) = rest.trim_start().strip_prefix('=') {
                return Some(rest.trim().trim_matches('"').to_owned());
            }
        }
    }
    None
}

/// One lock row for a crate directory: name, version, source and content hash.
fn lock_row(root: &Path, package: &str, dir: &Path) -> Result<LockRow, Error> {
    let cargo_path = dir.join("Cargo.toml");
    let text = fs::read_to_string(&cargo_path).map_err(|error| {
        Error::Project(format!("cannot read {}: {error}", cargo_path.display()))
    })?;
    let version = crate_version(root, &text).ok_or_else(|| {
        Error::Project(format!(
            "{}: `[package] version` is missing; the lock records one version per crate",
            cargo_path.display()
        ))
    })?;
    Ok(LockRow {
        name: package.to_owned(),
        version,
        source: format!("crate:{}", relative_source(root, dir)),
        hash: crate_hash(dir)?,
    })
}

/// `dir` from `root`, forward slashes; the root itself is `.`.
fn relative_source(root: &Path, dir: &Path) -> String {
    let relative = dir.strip_prefix(root).unwrap_or(dir);
    let text = relative.to_string_lossy().replace('\\', "/");
    if text.is_empty() {
        ".".to_owned()
    } else {
        text
    }
}

/// 16 hex digits over a crate directory: every file's relative path and bytes, in path order.
fn crate_hash(dir: &Path) -> Result<String, Error> {
    let mut files = Vec::new();
    collect_files(dir, &mut files)?;
    files.sort();
    let mut hash = Fnv64::new();
    for file in &files {
        let bytes = fs::read(file)
            .map_err(|error| Error::Project(format!("cannot read {}: {error}", file.display())))?;
        hash.write(relative_source(dir, file).as_bytes());
        hash.write_len(bytes.len());
        hash.write(&bytes);
    }
    Ok(hex16(hash.finish()))
}

/// Every file under `dir`, recursively; directories hold no bytes to hash.
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Error> {
    let entries = fs::read_dir(dir)
        .map_err(|error| Error::Project(format!("cannot read {}: {error}", dir.display())))?;
    for entry in entries {
        let entry = entry
            .map_err(|error| Error::Project(format!("cannot read {}: {error}", dir.display())))?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// Refuse a key that parses but reaches no build or run, naming it, its line and what the
/// engine does instead, so a setting never looks applied when it is not.
fn refuse_unwired(manifest_path: &Path, manifest: &Manifest) -> Result<(), Error> {
    let unwired = manifest.unwired();
    let Some(&(key, line)) = unwired.first() else {
        return Ok(());
    };
    let note = unwired_note(key).unwrap_or("nothing reads it yet");
    let also: Vec<&str> = unwired.iter().skip(1).map(|(other, _)| *other).collect();
    let also = if also.is_empty() {
        String::new()
    } else {
        format!(" Also not wired yet: {}.", also.join(", "))
    };
    Err(Error::Project(format!(
        "{}: line {line}, key `{key}`: not wired yet: {key}; {note}.{also} Remove the line to use the engine's default.",
        manifest_path.display()
    )))
}

/// The one unsigned number of the seed file at `path`; blank lines and `#` comments are
/// skipped. A refusal names the file and the line.
fn parse_seed(path: &Path, text: &str) -> Result<u64, Error> {
    let refuse = |line: usize, why: String| {
        Error::Project(format!("{}: line {line}: {why}", path.display()))
    };
    let mut seed: Option<u64> = None;
    for (index, raw) in text.lines().enumerate() {
        let body = raw.trim_start_matches('\u{feff}').trim();
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        if seed.is_some() {
            return Err(refuse(
                index + 1,
                "a seed file holds exactly one number".to_owned(),
            ));
        }
        let number = body.parse::<u64>().map_err(|_| {
            refuse(
                index + 1,
                format!(
                    "`{body}` is not an unsigned whole number (0 to {})",
                    u64::MAX
                ),
            )
        })?;
        seed = Some(number);
    }
    seed.ok_or_else(|| {
        Error::Project(format!(
            "{}: no seed number: put one whole number on a line of its own",
            path.display()
        ))
    })
}

fn run_verb(args: impl Iterator<Item = OsString>, out: &mut dyn Write) -> Result<(), Error> {
    let checkout = Checkout::host();
    let mut run = process::run_process;
    run_verb_with(args, &checkout, &mut run, out)
}

/// The `run` verb against a chosen checkout and runner, so tests drive it without a game build.
fn run_verb_with(
    args: impl Iterator<Item = OsString>,
    checkout: &Checkout,
    run: &mut Runner<'_>,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let flags = Flags::parse(args, &["--headless"], &["--ticks", "--manifest"])?;
    flags.no_positional("run")?;
    if !flags.switch("--headless") {
        return Err(usage(
            "run needs --headless (the window game is built with: build --target windows)",
        ));
    }
    let ticks = match flags.text("--ticks")? {
        Some(text) => parse_ticks(text)?,
        None => Config::default().ticks,
    };
    let project = Project::load_in(&flags.manifest(), &checkout.root)?;
    run_project(&project, checkout, ticks, run, out)
}

/// The project's own game when `game.toml` names one; else the seeded proof fixture, which is
/// what every project ran before `game` existed, so its receipt does not move.
fn run_project(
    project: &Project,
    checkout: &Checkout,
    ticks: u32,
    run: &mut Runner<'_>,
    out: &mut dyn Write,
) -> Result<(), Error> {
    if project.names_game() {
        run_game(project, checkout, ticks, run, out)
    } else {
        run_headless(project, ticks, out)
    }
}

/// Run the built bin of the game the project names: `--headless --ticks <n>` in the project
/// directory, plus `--seed <n>` when a seed file is set. The bin is not built here; `build`
/// does that and this names the missing exe when it has not run.
fn run_game(
    project: &Project,
    checkout: &Checkout,
    ticks: u32,
    run: &mut Runner<'_>,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let game = project.game();
    let exe = checkout.exe("release", &game.bin);
    if !exe.is_file() {
        return Err(Error::Project(format!(
            "the game `{}` is not built: {} is missing; build it first: engine2040 build --target windows",
            game.spec(),
            exe.display()
        )));
    }
    let mut args = vec![
        "--headless".to_owned(),
        "--ticks".to_owned(),
        ticks.to_string(),
    ];
    if project.manifest.seed_file.is_some() {
        args.push("--seed".to_owned());
        args.push(project.seed()?.to_string());
    }
    writeln!(out, "engine2040 project: {}", project.manifest.summary())?;
    writeln!(
        out,
        "running {} {} in {}",
        game.spec(),
        args.join(" "),
        project.dir.display()
    )?;
    out.flush()?;
    let step = Step {
        program: exe,
        args,
        dir: project.dir.clone(),
        capture: false,
        fix: "build it: engine2040 build --target windows",
    };
    let outcome = process::start(run, &step)?;
    if outcome.code != Some(0) {
        return Err(Error::Project(format!(
            "the game `{}` failed ({}); its output above says why",
            game.spec(),
            process::exit_text(outcome.code)
        )));
    }
    writeln!(
        out,
        "ran {} ({})",
        game.spec(),
        process::exit_text(outcome.code)
    )?;
    Ok(())
}

fn parse_ticks(text: &str) -> Result<u32, Error> {
    match text.parse::<u32>() {
        Ok(ticks) if (1..=MAX_TICKS).contains(&ticks) => Ok(ticks),
        _ => Err(usage(format!(
            "ticks must be a whole number in 1..={MAX_TICKS}"
        ))),
    }
}

/// Play the project's seeded proof for `ticks` steps, replay it to check it, print the receipt.
fn run_headless(project: &Project, ticks: u32, out: &mut dyn Write) -> Result<(), Error> {
    let config = Config {
        seed: project.seed()?,
        ticks,
    };
    let tape = record(config, &[]).map_err(|error| Error::Runtime(Box::new(error)))?;
    replay(&tape).map_err(|error| Error::Runtime(Box::new(error)))?;
    writeln!(out, "engine2040 project: {}", project.manifest.summary())?;
    write_text_receipt(out, &tape, "run", true)?;
    Ok(())
}

/// The `--target` word as a [`Target`], refusing anything else with the valid list.
pub fn parse_target(word: Option<&str>) -> Result<Target, Error> {
    let valid = valid_words::<Target>();
    let word =
        word.ok_or_else(|| usage(format!("build needs --target; valid targets: {valid}")))?;
    lookup::<Target>(word)
        .ok_or_else(|| usage(format!("unknown target `{word}`; valid targets: {valid}")))
}

/// A fresh empty directory under the system temp dir, removed on drop.
#[cfg(test)]
pub struct TempDir(pub PathBuf);

#[cfg(test)]
impl TempDir {
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        loop {
            let path = std::env::temp_dir().join(format!(
                "engine2040-project-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => panic!("temporary directory: {error}"),
            }
        }
    }
}

#[cfg(test)]
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::HELP;
    use crate::manifest::Choice;

    fn words(args: &[&str]) -> impl Iterator<Item = OsString> {
        args.iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
            .into_iter()
    }

    /// Run a verb in-process and return what it printed.
    fn say(verb: Verb, args: &[&str]) -> Result<String, Error> {
        let mut out = Vec::new();
        execute(verb, words(args), &mut out)?;
        Ok(String::from_utf8(out).unwrap())
    }

    fn new_project(temp: &TempDir, template: &str, name: &str) -> PathBuf {
        let dir = temp.0.join(name);
        say(Verb::New, &[template, dir.to_str().unwrap()]).unwrap();
        dir
    }

    fn run_in(dir: &Path, extra: &[&str]) -> Result<String, Error> {
        let manifest = dir.join("game.toml");
        let mut args = vec!["--headless", "--manifest", manifest.to_str().unwrap()];
        args.extend_from_slice(extra);
        say(Verb::Run, &args)
    }

    fn message(error: &Error) -> String {
        error.to_string()
    }

    /// A project loaded against the checkout this test binary was built from.
    fn load_here(path: &Path) -> Result<Project, Error> {
        Project::load_in(path, Path::new(env!("CARGO_MANIFEST_DIR")))
    }

    #[test]
    fn new_template_arena_duel_parses_and_runs_600_ticks_to_one_hash_twice() {
        let temp = TempDir::new();
        let dir = new_project(&temp, "arena-duel", "duel-1");
        let manifest = load_here(&dir.join("game.toml")).unwrap().manifest;
        assert_eq!(manifest.name, "duel-1");
        let first = run_in(&dir, &["--ticks", "600"]).unwrap();
        let second = run_in(&dir, &["--ticks", "600"]).unwrap();
        assert_eq!(first, second, "same manifest, same receipt");
        let golden = format!("final_hash={:016x}", engine_proof::GOLDEN_600);
        assert!(
            first.contains(&golden),
            "seed 2040 over 600 ticks is the golden run: {first}"
        );
        assert!(
            first.starts_with("engine2040 project: duel-1 quality=medium"),
            "{first}"
        );
        assert!(first.contains("ticks=600"), "{first}");
        assert!(first.contains("mode=run verified=true"), "{first}");
    }

    #[test]
    fn new_template_seed_file_reaches_the_run() {
        let temp = TempDir::new();
        let dir = new_project(&temp, "arena-duel", "duel-2");
        let golden = run_in(&dir, &["--ticks", "30"]).unwrap();
        fs::write(dir.join("content/seed.txt"), "# other\n7\n").unwrap();
        let other = run_in(&dir, &["--ticks", "30"]).unwrap();
        assert!(other.contains("seed=7 "), "{other}");
        let hash = |text: &str| text.split("final_hash=").nth(1).unwrap()[..16].to_owned();
        assert_ne!(hash(&golden), hash(&other));
        let direct = record(Config { seed: 7, ticks: 30 }, &[]).unwrap();
        assert!(other.contains(&format!(
            "final_hash={:016x}",
            direct.checkpoints.last().unwrap().legacy
        )));
    }

    #[test]
    fn new_template_siege_generates_a_valid_project_that_runs() {
        let temp = TempDir::new();
        let dir = new_project(&temp, "siege", "siege-1");
        let project = load_here(&dir.join("game.toml")).unwrap();
        assert_eq!(project.manifest.targets, [Target::Windows, Target::Web]);
        let receipt = run_in(&dir, &["--ticks", "20"]).unwrap();
        assert!(receipt.contains("seed=64 "), "{receipt}");
        assert!(fs::read_to_string(dir.join("README.md"))
            .unwrap()
            .contains("What is missing"));
    }

    #[test]
    fn new_template_bad_manifest_names_key_and_line() {
        let temp = TempDir::new();
        let dir = new_project(&temp, "arena-duel", "duel-3");
        let path = dir.join("game.toml");
        let text = fs::read_to_string(&path).unwrap();
        let broken = format!("{text}quality = \"ultrahigh\"\n");
        let line = broken.lines().count();
        fs::write(&path, broken).unwrap();
        let shown = message(&run_in(&dir, &[]).unwrap_err());
        assert!(shown.contains(&format!("line {line}")), "{shown}");
        assert!(shown.contains("key `quality`"), "{shown}");
        assert!(
            shown.contains("minimal, low, medium, high, ultra"),
            "{shown}"
        );
        assert!(shown.contains("game.toml"), "{shown}");
    }

    #[test]
    fn new_template_missing_content_root_and_bad_seed_are_named() {
        let temp = TempDir::new();
        let dir = new_project(&temp, "arena-duel", "duel-4");
        fs::write(dir.join("content/seed.txt"), "2040\nabc\n").unwrap();
        let shown = message(&run_in(&dir, &[]).unwrap_err());
        assert!(shown.contains("seed.txt: line 2"), "{shown}");
        assert!(shown.contains("exactly one number"), "{shown}");
        fs::write(dir.join("content/seed.txt"), "abc\n").unwrap();
        assert!(message(&run_in(&dir, &[]).unwrap_err()).contains("line 1"));
        fs::write(dir.join("content/seed.txt"), "# nothing\n").unwrap();
        assert!(message(&run_in(&dir, &[]).unwrap_err()).contains("no seed number"));
        // The template names no content root; a project that names one gets its roots checked.
        let path = dir.join("game.toml");
        let text = fs::read_to_string(&path).unwrap();
        fs::write(&path, format!("{text}content = [\"content\"]\n")).unwrap();
        fs::remove_dir_all(dir.join("content")).unwrap();
        let shown = message(&run_in(&dir, &[]).unwrap_err());
        assert!(shown.contains("key `content`"), "{shown}");
        assert!(
            shown.contains("root `content` is not a directory"),
            "{shown}"
        );
    }

    /// The refusal a project gets when its manifest sets `extra` on line 3.
    fn refused_extra(temp: &TempDir, name: &str, extra: &str) -> String {
        let dir = temp.0.join(name);
        fs::create_dir_all(dir.join("content")).unwrap();
        let path = dir.join("game.toml");
        fs::write(
            &path,
            format!("name = \"a\"\ntargets = [\"web\"]\n{extra}\n"),
        )
        .unwrap();
        message(&Project::load_in(&path, Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap_err())
    }

    #[test]
    fn quality_is_refused_by_name() {
        let temp = TempDir::new();
        let shown = refused_extra(&temp, "quality-game", "quality = \"high\"");
        assert!(shown.contains("line 3, key `quality`"), "{shown}");
        assert!(shown.contains("not wired yet: quality"), "{shown}");
        assert!(shown.contains("engine's own tier"), "{shown}");
        assert!(shown.contains("Remove the line"), "{shown}");
    }

    #[test]
    fn gore_is_refused_by_name() {
        let temp = TempDir::new();
        let shown = refused_extra(&temp, "gore-game", "gore = \"off\"");
        assert!(shown.contains("line 3, key `gore`"), "{shown}");
        assert!(shown.contains("not wired yet: gore"), "{shown}");
        assert!(shown.contains("engine's own gore setting"), "{shown}");
    }

    #[test]
    fn physics_layers_is_refused_by_name() {
        let temp = TempDir::new();
        let shown = refused_extra(
            &temp,
            "layers-game",
            "physics_layers = [\"authority\", \"physical\"]",
        );
        assert!(shown.contains("line 3, key `physics_layers`"), "{shown}");
        assert!(shown.contains("not wired yet: physics_layers"), "{shown}");
        assert!(shown.contains("authority and physical"), "{shown}");
    }

    #[test]
    fn content_is_refused_by_name() {
        let temp = TempDir::new();
        let shown = refused_extra(&temp, "content-game", "content = [\"content\"]");
        assert!(shown.contains("line 3, key `content`"), "{shown}");
        assert!(shown.contains("not wired yet: content"), "{shown}");
        assert!(shown.contains("own content/ tree"), "{shown}");
    }

    #[test]
    fn two_unwired_keys_name_the_first_and_list_the_rest() {
        let temp = TempDir::new();
        let shown = refused_extra(&temp, "two-keys", "content = [\"content\"]\ngore = \"off\"");
        assert!(
            shown.contains("line 3, key `content`: not wired yet: content"),
            "{shown}"
        );
        assert!(shown.contains("Also not wired yet: gore."), "{shown}");
    }

    #[test]
    fn a_game_the_workspace_lacks_is_refused_with_the_names_it_has() {
        let temp = TempDir::new();
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let dir = temp.0.join("bad-game");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("game.toml");
        fs::write(
            &path,
            "name = \"a\"\ntargets = [\"windows\"]\ngame = \"engine-nope:arena\"\n",
        )
        .unwrap();
        let shown = message(&Project::load_in(&path, root).unwrap_err());
        assert!(shown.contains("key `game`"), "{shown}");
        assert!(shown.contains("has no package `engine-nope`"), "{shown}");
        assert!(shown.contains("engine-game"), "{shown}");
        fs::write(
            &path,
            "name = \"a\"\ntargets = [\"windows\"]\ngame = \"engine-game:nope\"\n",
        )
        .unwrap();
        let shown = message(&Project::load_in(&path, root).unwrap_err());
        assert!(shown.contains("has no bin `nope`"), "{shown}");
        assert!(shown.contains("staged_fight"), "{shown}");
    }

    #[test]
    fn two_projects_that_name_different_games_resolve_different_bins() {
        let temp = TempDir::new();
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let load = |name: &str, game: &str| {
            let dir = temp.0.join(name);
            fs::create_dir_all(&dir).unwrap();
            let path = dir.join("game.toml");
            fs::write(
                &path,
                format!("name = \"a\"\ntargets = [\"windows\"]\ngame = \"{game}\"\n"),
            )
            .unwrap();
            Project::load_in(&path, root).unwrap().game()
        };
        let arena = load("arena-project", "engine-game:arena");
        let mcp = load("mcp-project", "engine-endpoint:e2040-mcp");
        assert_eq!(arena.spec(), "engine-game:arena");
        assert_eq!(mcp.spec(), "engine-endpoint:e2040-mcp");
        assert_eq!(arena.features, ["app"]);
        assert_eq!(mcp.features, ["mcp"], "the bin's required-features");
    }

    /// A fake engine checkout with one package, one declared bin and no built output.
    fn fake_checkout(temp: &TempDir, package: &str, bin: &str) -> Checkout {
        let root = temp.0.join("engine");
        let member = root.join("crates").join(package);
        fs::create_dir_all(member.join("src/bin")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"engine2040\"\n\n[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();
        fs::write(
            member.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{package}\"\nautobins = false\n\n\
                 [[bin]]\nname = \"{bin}\"\npath = \"src/bin/{bin}.rs\"\n"
            ),
        )
        .unwrap();
        fs::write(
            member.join("src/bin").join(format!("{bin}.rs")),
            "fn main() {}\n",
        )
        .unwrap();
        Checkout {
            target_dir: root.join("target"),
            root,
            os: std::env::consts::OS,
            exe_suffix: std::env::consts::EXE_SUFFIX,
        }
    }

    #[test]
    fn run_of_a_declared_game_runs_the_built_bin_with_ticks_and_seed() {
        let temp = TempDir::new();
        let checkout = fake_checkout(&temp, "demo", "play");
        let dir = temp.0.join("project");
        fs::create_dir_all(dir.join("content")).unwrap();
        fs::write(dir.join("content/seed.txt"), "77\n").unwrap();
        let path = dir.join("game.toml");
        fs::write(
            &path,
            "name = \"demo\"\ntargets = [\"windows\"]\ngame = \"demo:play\"\nseed_file = \"content/seed.txt\"\n",
        )
        .unwrap();
        let exe = checkout.exe("release", "play");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, "").unwrap();
        let manifest = path.to_str().unwrap();
        let mut seen: Vec<String> = Vec::new();
        let mut out = Vec::new();
        let mut run = |step: &Step| {
            seen.push(format!(
                "{} {}",
                step.program.display(),
                step.args.join(" ")
            ));
            assert_eq!(step.dir, dir, "the game runs in the project directory");
            Ok(process::Outcome {
                code: Some(0),
                output: String::new(),
            })
        };
        run_verb_with(
            words(&["--headless", "--ticks", "30", "--manifest", manifest]),
            &checkout,
            &mut run,
            &mut out,
        )
        .unwrap();
        assert_eq!(
            seen,
            [format!("{} --headless --ticks 30 --seed 77", exe.display())]
        );
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("engine2040 project: demo "), "{text}");
        assert!(
            text.contains("running demo:play --headless --ticks 30 --seed 77"),
            "{text}"
        );
        assert!(text.contains("ran demo:play (exit code 0)"), "{text}");
    }

    #[test]
    fn run_of_a_declared_game_that_is_not_built_names_the_build() {
        let temp = TempDir::new();
        let checkout = fake_checkout(&temp, "demo", "play");
        let dir = temp.0.join("project");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("game.toml");
        fs::write(
            &path,
            "name = \"demo\"\ntargets = [\"windows\"]\ngame = \"demo:play\"\n",
        )
        .unwrap();
        let mut ran = false;
        let mut run = |_: &Step| {
            ran = true;
            Ok(process::Outcome {
                code: Some(0),
                output: String::new(),
            })
        };
        let error = run_verb_with(
            words(&["--headless", "--manifest", path.to_str().unwrap()]),
            &checkout,
            &mut run,
            &mut Vec::new(),
        )
        .unwrap_err();
        let shown = message(&error);
        assert!(
            shown.contains("the game `demo:play` is not built"),
            "{shown}"
        );
        assert!(shown.contains("build --target windows"), "{shown}");
        assert!(!ran, "nothing runs when the exe is missing");
    }

    #[test]
    fn new_template_refuses_a_non_empty_directory_and_writes_nothing() {
        let temp = TempDir::new();
        let dir = temp.0.join("taken");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("keep.txt"), "mine").unwrap();
        let error = say(Verb::New, &["arena-duel", dir.to_str().unwrap()]).unwrap_err();
        assert!(message(&error).contains("is not empty"), "{error}");
        assert_eq!(error.exit_code(), 1);
        assert_eq!(fs::read_to_string(dir.join("keep.txt")).unwrap(), "mine");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "nothing was added");
        // An existing empty directory is fine, and a second `new` over it is refused.
        let empty = temp.0.join("empty-ok");
        fs::create_dir(&empty).unwrap();
        say(Verb::New, &["siege", empty.to_str().unwrap()]).unwrap();
        let again = say(Verb::New, &["siege", empty.to_str().unwrap()]).unwrap_err();
        assert!(message(&again).contains("is not empty"));
    }

    #[test]
    fn new_template_unknown_template_and_bad_directory_name_are_usage_errors() {
        let temp = TempDir::new();
        let dir = temp.0.join("ok-name");
        let error = say(Verb::New, &["moba", dir.to_str().unwrap()]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(
            message(&error).contains("valid templates: arena-duel, siege"),
            "{error}"
        );
        assert!(!dir.exists());
        let bad = temp.0.join("Bad Name");
        let error = say(Verb::New, &["siege", bad.to_str().unwrap()]).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(
            message(&error).contains("cannot be a project name"),
            "{error}"
        );
        assert!(!bad.exists());
        assert_eq!(say(Verb::New, &["siege"]).unwrap_err().exit_code(), 2);
        assert_eq!(say(Verb::New, &["a", "b", "c"]).unwrap_err().exit_code(), 2);
    }

    #[test]
    fn new_template_embedded_files_equal_docs_templates() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/templates");
        let mut on_disk: Vec<String> = fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap())
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().into_string().unwrap())
            .collect();
        on_disk.sort();
        let mut embedded: Vec<&str> = TEMPLATES.iter().map(|t| t.name).collect();
        embedded.sort_unstable();
        assert_eq!(on_disk, embedded, "every template directory is embedded");
        for template in &TEMPLATES {
            let mut found = Vec::new();
            collect(&root.join(template.name), "", &mut found);
            found.sort();
            let mut listed: Vec<String> = template
                .files
                .iter()
                .map(|(path, _)| (*path).to_owned())
                .collect();
            listed.sort();
            assert_eq!(found, listed, "{} files", template.name);
            for (path, text) in template.files {
                let disk = fs::read_to_string(root.join(template.name).join(path)).unwrap();
                assert_eq!(&disk, text, "{}/{path}", template.name);
            }
        }
    }

    fn collect(dir: &Path, prefix: &str, found: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = format!("{prefix}{}", entry.file_name().into_string().unwrap());
            if entry.path().is_dir() {
                collect(&entry.path(), &format!("{name}/"), found);
            } else {
                found.push(name);
            }
        }
    }

    #[test]
    fn run_arguments_are_strict() {
        let cases: [&[&str]; 6] = [
            &[],
            &["--headless", "--ticks", "0"],
            &["--headless", "--ticks", "100001"],
            &["--headless", "--ticks", "x"],
            &["--headless", "--headless"],
            &["--headless", "extra"],
        ];
        for args in cases {
            let error = say(Verb::Run, args).unwrap_err();
            assert_eq!(error.exit_code(), 2, "{args:?}: {error}");
        }
        let missing = say(
            Verb::Run,
            &["--headless", "--manifest", "nowhere/game.toml"],
        );
        let shown = message(&missing.unwrap_err());
        assert!(shown.contains("cannot read nowhere"), "{shown}");
        assert!(shown.contains("engine2040 new arena-duel"), "{shown}");
    }

    #[test]
    fn parse_target_lists_the_valid_ones() {
        assert_eq!(parse_target(Some("web")).unwrap(), Target::Web);
        for bad in [Some("linux"), Some("Windows"), None] {
            let error = parse_target(bad).unwrap_err();
            assert_eq!(error.exit_code(), 2);
            assert!(
                message(&error).contains("valid targets: windows, web"),
                "{error}"
            );
        }
    }

    #[test]
    fn text_receipt_lines_are_the_golden_recorder_lines() {
        let tape = record(Config::default(), &[]).unwrap();
        let mut out = Vec::new();
        write_text_receipt(&mut out, &tape, "run", false).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "engine2040 headless proof:");
        assert_eq!(
            lines[1],
            format!("  ticks=600 final_hash={:016x}", engine_proof::GOLDEN_600)
        );
        assert!(lines[2].starts_with("  seed=2040 inputs=0 state_hash="));
        assert!(lines[2].ends_with(" backend=null mode=run verified=false"));
    }

    #[test]
    fn help_names_every_verb_template_and_target() {
        for word in ["new", "run", "build"] {
            assert!(Verb::from_word(word).is_some(), "{word}");
            assert!(HELP.contains(&format!("\n  {word} ")), "{word}");
        }
        for template in &TEMPLATES {
            assert!(HELP.contains(template.name), "{}", template.name);
        }
        for target in Target::WORDS {
            assert!(HELP.contains(target.word()), "{}", target.word());
        }
        // Anything else is the golden CLI's, so its flags keep working.
        for word in ["--help", "--verify", "--record", "--seed", ""] {
            assert_eq!(Verb::from_word(word), None, "{word}");
        }
    }
}
