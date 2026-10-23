//! The engine checkout's Cargo workspace, read the way the project verbs need it.
//!
//! `game.toml` names the game a project builds and runs as `<package>:<bin>`. Only the
//! workspace can say whether that name exists: which packages it has, which bins each package
//! declares, and which features a bin needs. This module reads the workspace `Cargo.toml` and
//! each member's `Cargo.toml`; it runs no cargo and writes nothing. A refused name is answered
//! with the names that do exist, so the fix is one edit away.

use crate::manifest::Game;
use std::fs;
use std::path::{Path, PathBuf};

/// One bin a package declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bin {
    /// The `--bin` name.
    pub name: String,
    /// The `[[bin]]` block's `required-features`; a build passes them with `--features`.
    pub features: Vec<String>,
}

/// One workspace package and its bins, both sorted by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    /// The `[package] name`.
    pub name: String,
    /// Every bin target Cargo would build for the package.
    pub bins: Vec<Bin>,
}

/// A resolved `game = "<package>:<bin>"`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameBin {
    /// The workspace package to build.
    pub package: String,
    /// The bin of that package to build and run.
    pub bin: String,
    /// Features the bin needs, from its `[[bin]]` block.
    pub features: Vec<String>,
}

impl GameBin {
    /// `<package>:<bin>`, the spelling `game.toml` uses.
    pub fn spec(&self) -> String {
        format!("{}:{}", self.package, self.bin)
    }
}

/// The game of a project without a `game` key: the engine's own windowed arena. Its features
/// are pinned to `crates/game/Cargo.toml` by a test, so the constant cannot drift from the
/// `[[bin]]` block that declares them.
pub fn default_game() -> GameBin {
    let game = Game::default();
    GameBin {
        package: game.package,
        bin: game.bin,
        features: vec!["app".to_owned()],
    }
}

/// The bin `package:bin` names, or why the workspace has no such game, listing what it has.
pub fn resolve(root: &Path, package: &str, bin: &str) -> Result<GameBin, String> {
    let packages = packages(root)?;
    let found = packages
        .iter()
        .find(|candidate| candidate.name == package)
        .ok_or_else(|| {
            let names: Vec<&str> = packages
                .iter()
                .map(|candidate| candidate.name.as_str())
                .collect();
            format!(
                "the workspace in {} has no package `{package}`; packages: {}",
                root.display(),
                listed(&names)
            )
        })?;
    let target = found.bins.iter().find(|b| b.name == bin).ok_or_else(|| {
        let names: Vec<&str> = found.bins.iter().map(|b| b.name.as_str()).collect();
        format!(
            "package `{package}` has no bin `{bin}`; its bins: {}",
            listed(&names)
        )
    })?;
    Ok(GameBin {
        package: package.to_owned(),
        bin: bin.to_owned(),
        features: target.features.clone(),
    })
}

fn listed(names: &[&str]) -> String {
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}

/// Every package of the workspace at `root` (which holds its `Cargo.toml`), sorted by name.
pub fn packages(root: &Path) -> Result<Vec<Package>, String> {
    let root_cargo = root.join("Cargo.toml");
    let text = fs::read_to_string(&root_cargo)
        .map_err(|error| format!("cannot read {}: {error}", root_cargo.display()))?;
    let mut dirs = vec![root.to_path_buf()];
    for entry in members(&text) {
        dirs.extend(member_dirs(root, &entry)?);
    }
    let mut out: Vec<Package> = Vec::new();
    for dir in dirs {
        let cargo_path = dir.join("Cargo.toml");
        if !cargo_path.is_file() {
            continue;
        }
        let cargo = read_cargo(&cargo_path)?;
        let Some(name) = cargo.name else {
            return Err(format!(
                "{}: `[package] name` is missing",
                cargo_path.display()
            ));
        };
        if out.iter().any(|package| package.name == name) {
            continue;
        }
        let mut bins = cargo.bins;
        if cargo.autobins {
            bins.extend(auto_bins(&dir, &name));
        }
        bins.sort_by(|a, b| a.name.cmp(&b.name));
        bins.dedup_by(|a, b| a.name == b.name);
        out.push(Package { name, bins });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// The bins Cargo infers when `autobins` is on: `src/main.rs` and every `src/bin/*.rs`.
fn auto_bins(dir: &Path, package: &str) -> Vec<Bin> {
    let mut bins = Vec::new();
    if dir.join("src/main.rs").is_file() {
        bins.push(Bin {
            name: package.to_owned(),
            features: Vec::new(),
        });
    }
    if let Ok(entries) = fs::read_dir(dir.join("src/bin")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|extension| extension == "rs") {
                if let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) {
                    bins.push(Bin {
                        name: name.to_owned(),
                        features: Vec::new(),
                    });
                }
            }
        }
    }
    bins
}

/// The directories a `members` entry names. `dir/*` is the only pattern this workspace uses;
/// any other pattern is refused rather than guessed.
fn member_dirs(root: &Path, entry: &str) -> Result<Vec<PathBuf>, String> {
    let entry = entry.trim();
    if entry == "." {
        return Ok(Vec::new());
    }
    let Some((prefix, rest)) = entry.split_once('*') else {
        return Ok(vec![root.join(entry)]);
    };
    if !rest.is_empty() || !(prefix.is_empty() || prefix.ends_with('/')) {
        return Err(format!(
            "member pattern `{entry}` is not `dir/*`; the workspace reader does not guess"
        ));
    }
    let dir = root.join(prefix.trim_end_matches('/'));
    let entries =
        fs::read_dir(&dir).map_err(|error| format!("cannot list {}: {error}", dir.display()))?;
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    found.sort();
    Ok(found)
}

/// The workspace `Cargo.toml`'s `members` patterns; empty when it lists none.
fn members(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut at = 0;
    while at < lines.len() {
        if let Some((key, value)) = assignment(lines[at]) {
            if key == "members" {
                let mut value = value.to_owned();
                while !closed(&value) && at + 1 < lines.len() {
                    at += 1;
                    value.push(' ');
                    value.push_str(lines[at].trim());
                }
                return items(&value);
            }
        }
        at += 1;
    }
    Vec::new()
}

/// What one member's `Cargo.toml` says about its bins.
struct CargoFile {
    name: Option<String>,
    autobins: bool,
    bins: Vec<Bin>,
}

#[derive(PartialEq)]
enum Section {
    Package,
    Bin,
    Other,
}

fn read_cargo(path: &Path) -> Result<CargoFile, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let lines: Vec<&str> = text.lines().collect();
    let mut cargo = CargoFile {
        name: None,
        autobins: true,
        bins: Vec::new(),
    };
    let mut section = Section::Other;
    let mut bin: Option<Bin> = None;
    let mut at = 0;
    while at < lines.len() {
        let body = lines[at].trim();
        if let Some(header) = body
            .strip_prefix("[[")
            .and_then(|rest| rest.strip_suffix("]]"))
        {
            finish_bin(&mut cargo.bins, bin.take());
            section = if header.trim() == "bin" {
                bin = Some(Bin {
                    name: String::new(),
                    features: Vec::new(),
                });
                Section::Bin
            } else {
                Section::Other
            };
            at += 1;
            continue;
        }
        if body.starts_with('[') {
            finish_bin(&mut cargo.bins, bin.take());
            section = if body.starts_with("[package]") {
                Section::Package
            } else {
                Section::Other
            };
            at += 1;
            continue;
        }
        if section != Section::Other {
            if let Some((key, raw)) = assignment(lines[at]) {
                let mut value = raw.to_owned();
                while value.starts_with('[') && !closed(&value) && at + 1 < lines.len() {
                    at += 1;
                    value.push(' ');
                    value.push_str(lines[at].trim());
                }
                match section {
                    Section::Package => match key {
                        "name" => cargo.name = string(&value),
                        "autobins" => cargo.autobins = value.trim() != "false",
                        _ => {}
                    },
                    Section::Bin => {
                        if let Some(bin) = bin.as_mut() {
                            match key {
                                "name" => bin.name = string(&value).unwrap_or_default(),
                                "required-features" => bin.features = items(&value),
                                _ => {}
                            }
                        }
                    }
                    Section::Other => {}
                }
            }
        }
        at += 1;
    }
    finish_bin(&mut cargo.bins, bin.take());
    Ok(cargo)
}

fn finish_bin(bins: &mut Vec<Bin>, bin: Option<Bin>) {
    if let Some(bin) = bin {
        if !bin.name.is_empty() {
            bins.push(bin);
        }
    }
}

/// One `key = value` line, comment stripped; `None` for blank lines and comments.
fn assignment(line: &str) -> Option<(&str, &str)> {
    let body = line.trim();
    if body.is_empty() || body.starts_with('#') {
        return None;
    }
    let (key, value) = body.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    Some((key, strip_comment(value.trim())))
}

/// The value without a trailing `# comment` (a `#` inside a string stays).
fn strip_comment(value: &str) -> &str {
    let mut quoted = false;
    let mut escaped = false;
    for (at, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            '#' if !quoted => return value[..at].trim_end(),
            _ => {}
        }
    }
    value.trim_end()
}

/// Whether an array value has both brackets.
fn closed(value: &str) -> bool {
    let opens = value.matches('[').count();
    opens > 0 && opens == value.matches(']').count()
}

/// The strings of a one- or many-line `["a", "b"]` value.
fn items(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or("");
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            ',' if !quoted => {
                push_item(&mut out, &current);
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    push_item(&mut out, &current);
    out
}

fn push_item(out: &mut Vec<String>, text: &str) {
    let item = text.trim();
    if !item.is_empty() {
        out.push(item.to_owned());
    }
}

/// The text of a `"..."` value; `None` for anything else.
fn string(value: &str) -> Option<String> {
    let rest = value.trim().strip_prefix('"')?;
    let mut out = String::new();
    let mut escaped = false;
    for ch in rest.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some(out),
            _ => out.push(ch),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::TempDir;

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    /// A fake engine checkout: a root package plus `crates/*` members.
    fn checkout(temp: &TempDir, members_list: &str, crates: &[(&str, &str)]) -> PathBuf {
        let root = temp.0.join("engine");
        write(
            &root.join("Cargo.toml"),
            &format!("[package]\nname = \"engine2040\"\n\n[workspace]\nmembers = {members_list}\n"),
        );
        write(&root.join("src/main.rs"), "fn main() {}\n");
        for (dir, cargo) in crates {
            write(&root.join("crates").join(dir).join("Cargo.toml"), cargo);
        }
        root
    }

    #[test]
    fn the_real_workspace_resolves_the_arena_and_its_declared_features() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let arena = resolve(root, "engine-game", "arena").unwrap();
        assert_eq!(arena.spec(), "engine-game:arena");
        assert_eq!(
            arena.features,
            ["app"],
            "crates/game/Cargo.toml required-features"
        );
        assert_eq!(
            arena,
            default_game(),
            "the default game is the engine's arena"
        );
        assert!(resolve(root, "engine-endpoint", "e2040-mcp").is_ok());
    }

    #[test]
    fn unknown_package_and_bin_name_what_the_workspace_has() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let error = resolve(root, "engine-nope", "arena").unwrap_err();
        assert!(error.contains("has no package `engine-nope`"), "{error}");
        assert!(error.contains("engine-game"), "{error}");
        let error = resolve(root, "engine-game", "nope").unwrap_err();
        assert!(
            error.contains("package `engine-game` has no bin `nope`"),
            "{error}"
        );
        assert!(error.contains("staged_fight"), "{error}");
    }

    #[test]
    fn autobins_false_hides_helper_files_that_are_not_targets() {
        let temp = TempDir::new();
        let game = "[package]\nname = \"engine-game\"\nautobins = false\n\n\
                    [[bin]]\nname = \"arena\"\npath = \"src/bin/arena.rs\"\n\
                    required-features = [\"app\"]\n";
        let tool = "[package]\nname = \"engine-tool\"\n";
        let root = checkout(&temp, "[\"crates/*\"]", &[("game", game), ("tool", tool)]);
        write(&root.join("crates/game/src/bin/arena.rs"), "fn main() {}\n");
        write(
            &root.join("crates/game/src/bin/helper.rs"),
            "//! a module\n",
        );
        write(&root.join("crates/tool/src/main.rs"), "fn main() {}\n");
        write(&root.join("crates/tool/src/bin/extra.rs"), "fn main() {}\n");
        let packages = packages(&root).unwrap();
        let names: Vec<&str> = packages.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["engine-game", "engine-tool", "engine2040"]);
        let game = packages.iter().find(|p| p.name == "engine-game").unwrap();
        assert_eq!(
            game.bins
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>(),
            ["arena"],
            "autobins = false: the helper file is not a target"
        );
        assert_eq!(game.bins[0].features, ["app"]);
        let tool = packages.iter().find(|p| p.name == "engine-tool").unwrap();
        assert_eq!(
            tool.bins
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>(),
            ["engine-tool", "extra"]
        );
        assert_eq!(
            resolve(&root, "engine-game", "helper").unwrap_err(),
            "package `engine-game` has no bin `helper`; its bins: arena"
        );
    }

    #[test]
    fn member_patterns_are_read_and_a_missing_workspace_is_refused() {
        let temp = TempDir::new();
        let root = checkout(
            &temp,
            "[\n    \"crates/*\",\n]",
            &[("a", "[package]\nname = \"engine-a\"\n")],
        );
        assert_eq!(
            packages(&root)
                .unwrap()
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["engine-a", "engine2040"]
        );
        let error = packages(&temp.0.join("nowhere")).unwrap_err();
        assert!(error.contains("cannot read"), "{error}");
        let root = checkout(&temp, "[\"crates/**\"]", &[]);
        let error = packages(&root).unwrap_err();
        assert!(error.contains("is not `dir/*`"), "{error}");
    }
}
