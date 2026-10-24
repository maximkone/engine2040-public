//! The `build` verb of the `engine2040` bin: build one target of a project's game and print
//! where the result is. `windows` runs the cargo release build of the bin `game.toml` names
//! (the engine's `arena` when it names none) and exports it to `<project>/dist/<name>/` with a
//! `build.json` receipt and one archive step; `web` runs `node tools/web-build.mjs` and, when
//! that fails, adds what `node tools/web-doctor.mjs` says is missing, so the fix is one copy
//! away. Both builds run in the engine checkout this exe was built from. Every command goes
//! through a runner, so tests drive the steps with a script instead of a toolchain.

use crate::cli::Error;
use crate::manifest::{joined, Choice, Target};
use crate::process::{self, Checkout, Runner, Step};
use crate::project::{parse_target, Flags, LockRow, Project};
use crate::workspace::GameBin;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The web build tool, relative to the engine checkout.
const WEB_BUILD: &str = "tools/web-build.mjs";
/// The tool that lists what the web target still needs, with one fix command per piece.
const WEB_DOCTOR: &str = "tools/web-doctor.mjs";
/// The export receipt, inside `<project>/dist/<name>/`.
const BUILD_JSON: &str = "build.json";
/// Fix lines for the two programs the builds start themselves.
const NODE_FIX: &str = "winget install --id OpenJS.NodeJS.LTS -e (Node 24 or newer)";
const CARGO_FIX: &str = "winget install --id Rustlang.Rustup -e";
/// Windows 10 and newer ship bsdtar, which the export uses for its one archive step.
const TAR_FIX: &str = "tar ships with Windows 10+ (C:\\Windows\\System32\\tar.exe); install it or zip dist/<name> by hand";
/// The export hashes through the platform tool; this machine's certutil is the fix line.
const CERTUTIL_FIX: &str = "certutil ships with Windows (C:\\Windows\\System32\\certutil.exe)";

/// `build --target <windows|web> [--manifest <path>]`.
pub fn build_verb(args: impl Iterator<Item = OsString>, out: &mut dyn Write) -> Result<(), Error> {
    let checkout = Checkout::host();
    let mut run = process::run_process;
    build_verb_with(args, &checkout, &mut run, out)
}

/// The `build` verb against a chosen checkout and runner: the manifest check and the target
/// dispatch that [`build_verb`] runs on the host. Tests drive it with a script instead of a
/// toolchain, so a generated project's manifest and both target command lines are checked.
fn build_verb_with(
    args: impl Iterator<Item = OsString>,
    checkout: &Checkout,
    run: &mut Runner<'_>,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let flags = Flags::parse(args, &[], &["--target", "--manifest"])?;
    flags.no_positional("build")?;
    let target = parse_target(flags.text("--target")?)?;
    let manifest_path = flags.manifest();
    let project = Project::load_in(&manifest_path, &checkout.root)?;
    if !project.manifest.targets.contains(&target) {
        return Err(Error::Project(format!(
            "{}: key `targets` does not list `{}` (it lists: {}); add it there to build for it",
            manifest_path.display(),
            target.word(),
            joined(&project.manifest.targets, ", ")
        )));
    }
    check_lock(&project, &checkout.root)?;
    let result = build(checkout, target, &project.game(), run, out)?;
    if let Some(exe) = result {
        export_project(&project, &exe, run, out)?;
    }
    Ok(())
}

/// Enforce the project lock before anything builds: the first build writes it, a later build
/// refuses a dependency row that is missing or whose crate content moved, naming the row.
fn check_lock(project: &Project, root: &Path) -> Result<(), Error> {
    let rows = project.dependency_rows(root)?;
    let path = project.lock_path();
    if !path.is_file() {
        write_lock(&path, &rows)?;
        return Ok(());
    }
    let text = fs::read_to_string(&path)
        .map_err(|error| Error::Project(format!("cannot read {}: {error}", path.display())))?;
    let locked = parse_lock(&text);
    for row in &rows {
        let Some(hash) = locked
            .iter()
            .find(|(name, _)| name == &row.name)
            .map(|(_, hash)| hash)
        else {
            return Err(Error::Project(format!(
                "{}: no row for `{}`; the lock predates this dependency: delete {} and build again",
                path.display(),
                row.name,
                path.display()
            )));
        };
        if hash != &row.hash {
            return Err(Error::Project(format!(
                "{}: row `{}` moved (locked {hash}, now {}); delete {} and build again to accept the new crate content",
                path.display(),
                row.name,
                row.hash,
                path.display()
            )));
        }
    }
    Ok(())
}

/// Write the lock: one comment line, then one `name version source hash` row per crate.
fn write_lock(path: &Path, rows: &[LockRow]) -> Result<(), Error> {
    let mut text = String::from(
        "# engine2040 project lock: one row per resolved crate: name version source hash\n",
    );
    for row in rows {
        text.push_str(&row.line());
        text.push('\n');
    }
    fs::write(path, text)
        .map_err(|error| Error::Project(format!("cannot write {}: {error}", path.display())))
}

/// The rows a lock file holds: name to content hash. Comments and short rows are skipped.
fn parse_lock(text: &str) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.split_whitespace();
        let (Some(name), Some(_version), Some(_source), Some(hash)) =
            (words.next(), words.next(), words.next(), words.next())
        else {
            continue;
        };
        rows.push((name.to_owned(), hash.to_owned()));
    }
    rows
}

/// What a target built: the exe of a windows build; `None` for web, whose output already is a
/// folder.
fn build(
    checkout: &Checkout,
    target: Target,
    game: &GameBin,
    run: &mut Runner<'_>,
    out: &mut dyn Write,
) -> Result<Option<PathBuf>, Error> {
    if !checkout.root.is_dir() {
        return Err(Error::Project(format!(
            "the engine checkout {} is missing: build runs in the checkout this exe was built from",
            checkout.root.display()
        )));
    }
    match target {
        Target::Windows => build_windows(checkout, game, run, out).map(Some),
        Target::Web => {
            if game.spec() != crate::workspace::default_game().spec() {
                return Err(Error::Project(format!(
                    "{WEB_BUILD} builds the engine's own arena page, not `{}`: web export of another game is not wired yet: {}. Build --target windows instead, or remove the `game` line",
                    game.spec(),
                    game.spec()
                )));
            }
            build_web(checkout, run, out).map(|()| None)
        }
    }
}

fn build_windows(
    checkout: &Checkout,
    game: &GameBin,
    run: &mut Runner<'_>,
    out: &mut dyn Write,
) -> Result<PathBuf, Error> {
    if checkout.os != "windows" {
        return Err(Error::Project(format!(
            "target windows builds on a Windows machine; this one is {}",
            checkout.os
        )));
    }
    // The command follows the bin the workspace declares: its package, the features its
    // `[[bin]]` block requires, and its name.
    let mut args: Vec<String> = ["build", "--locked", "--release", "-p"]
        .iter()
        .map(|word| (*word).to_owned())
        .collect();
    args.push(game.package.clone());
    if !game.features.is_empty() {
        args.push("--features".to_owned());
        args.push(game.features.join(","));
    }
    args.push("--bin".to_owned());
    args.push(game.bin.clone());
    let step = Step {
        program: "cargo".into(),
        args,
        dir: checkout.root.clone(),
        capture: false,
        fix: CARGO_FIX,
    };
    let outcome = process::start(run, &step)?;
    if outcome.code != Some(0) {
        return Err(Error::Project(format!(
            "cargo build failed ({}); the compiler output above says why",
            process::exit_text(outcome.code)
        )));
    }
    let exe = checkout.exe("release", &game.bin);
    if !exe.is_file() {
        return Err(Error::Project(format!(
            "cargo finished but {} is missing; if the target directory moved, set CARGO_TARGET_DIR",
            exe.display()
        )));
    }
    writeln!(out, "built windows: {}", exe.display())?;
    Ok(exe)
}

/// One project export: the built exe into `<project>/dist/<name>/`, a `build.json` beside it
/// naming the project, its version, the target and one sha256 per staged file, then one
/// archive step that zips the folder to `<project>/dist/<name>.zip`.
///
/// The payload today is the game exe alone; packing the project's content is the rest of
/// WG11. The folder is replaced on every export, because a stale exe beside a fresh
/// `build.json` would be a false receipt.
fn export_project(
    project: &Project,
    exe: &Path,
    run: &mut Runner<'_>,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let name = project.manifest.name.clone();
    let game = project.game();
    let file_name = exe
        .file_name()
        .and_then(|file| file.to_str())
        .ok_or_else(|| {
            Error::Project(format!(
                "cannot name the export file of {}: the exe name is not UTF-8",
                exe.display()
            ))
        })?
        .to_owned();
    let dist_root = project.dir().join("dist");
    let dist = dist_root.join(&name);
    if dist.exists() {
        fs::remove_dir_all(&dist).map_err(|error| io_error("clear", &dist, &error))?;
    }
    fs::create_dir_all(&dist).map_err(|error| io_error("create", &dist, &error))?;
    let staged = dist.join(&file_name);
    fs::copy(exe, &staged).map_err(|error| {
        Error::Project(format!(
            "cannot copy {} to {}: {error}",
            exe.display(),
            staged.display()
        ))
    })?;
    let bytes = fs::read(&staged).map_err(|error| io_error("read", &staged, &error))?;
    let digest = file_sha256(&staged, project, run)?;
    let archive = dist_root.join(format!("{name}.zip"));
    let json = format!(
        "{{\n  \"schema\": 1,\n  \"name\": \"{name}\",\n  \"version\": \"{}\",\n  \"target\": \"windows\",\n  \"game\": \"{}\",\n  \"files\": [\n    {{\"path\": \"{file_name}\", \"bytes\": {}, \"sha256\": \"{digest}\"}}\n  ],\n  \"archive\": \"dist/{name}.zip\"\n}}\n",
        env!("CARGO_PKG_VERSION"),
        game.spec(),
        bytes.len()
    );
    let receipt = dist.join(BUILD_JSON);
    fs::write(&receipt, &json).map_err(|error| io_error("write", &receipt, &error))?;
    let step = Step {
        program: "tar".into(),
        args: vec![
            "-a".to_owned(),
            "-c".to_owned(),
            "-f".to_owned(),
            archive.display().to_string(),
            "-C".to_owned(),
            dist_root.display().to_string(),
            name.clone(),
        ],
        dir: project.dir().to_path_buf(),
        capture: false,
        fix: TAR_FIX,
    };
    let outcome = process::start(run, &step)?;
    if outcome.code != Some(0) {
        return Err(Error::Project(format!(
            "the archive step failed ({}); {} is exported but {} is missing",
            process::exit_text(outcome.code),
            dist.display(),
            archive.display()
        )));
    }
    if !archive.is_file() {
        return Err(Error::Project(format!(
            "tar finished but {} is missing; the folder {} is exported",
            archive.display(),
            dist.display()
        )));
    }
    writeln!(
        out,
        "exported {}: {} ({} bytes, sha256 {digest}, archive {})",
        name,
        dist.display(),
        bytes.len(),
        archive.display()
    )?;
    Ok(())
}

/// The lowercase hex sha256 of one staged file, from the hashing tool Windows ships.
///
/// The headless default graph stays std-only (`tools/workspace-check.mjs`), so no hash
/// crate rides this binary: `certutil` is the platform's own answer and its one output line
/// is the digest. A machine without it is named with the file it had to hash.
fn file_sha256(file: &Path, project: &Project, run: &mut Runner<'_>) -> Result<String, Error> {
    let step = Step {
        program: "certutil".into(),
        args: vec![
            "-hashfile".to_owned(),
            file.display().to_string(),
            "SHA256".to_owned(),
        ],
        dir: project.dir().to_path_buf(),
        capture: true,
        fix: CERTUTIL_FIX,
    };
    let outcome = process::start(run, &step)?;
    if outcome.code != Some(0) {
        return Err(Error::Project(format!(
            "the sha256 step failed ({}); `certutil -hashfile <file> SHA256` has to print the digest of {}",
            process::exit_text(outcome.code),
            file.display()
        )));
    }
    sha256_line(&outcome.output).ok_or_else(|| {
        Error::Project(format!(
            "certutil printed no 64-hex sha256 of {}: {}",
            file.display(),
            outcome.output.trim()
        ))
    })
}

/// The first line of `text` that is one 64-character hex sha256.
fn sha256_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| line.len() == 64 && line.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|line| line.to_ascii_lowercase())
}

fn io_error(verb: &str, path: &Path, error: &std::io::Error) -> Error {
    Error::Project(format!("cannot {verb} {}: {error}", path.display()))
}

fn build_web(checkout: &Checkout, run: &mut Runner<'_>, out: &mut dyn Write) -> Result<(), Error> {
    if !checkout.root.join(WEB_BUILD).is_file() {
        return Err(Error::Project(format!(
            "{WEB_BUILD} is missing from the engine checkout {}",
            checkout.root.display()
        )));
    }
    let outcome = process::start(run, &node_step(checkout, WEB_BUILD, false))?;
    if outcome.code != Some(0) {
        let head = format!(
            "node {WEB_BUILD} failed ({}); its output above names the step",
            process::exit_text(outcome.code)
        );
        return Err(Error::Project(format!(
            "{head}{}",
            doctor_report(checkout, run)
        )));
    }
    let dist = checkout.root.join("dist").join("web");
    if !dist.join("index.html").is_file() {
        return Err(Error::Project(format!(
            "{WEB_BUILD} finished but {} has no index.html",
            dist.display()
        )));
    }
    writeln!(out, "built web: {}", dist.display())?;
    Ok(())
}

fn node_step(checkout: &Checkout, script: &'static str, capture: bool) -> Step {
    Step {
        program: "node".into(),
        args: vec![script.to_owned()],
        dir: checkout.root.clone(),
        capture,
        fix: NODE_FIX,
    }
}

/// What the web doctor says is missing: its MISSING and `fix:` lines and the verdict line, so a
/// failed web build ends with the commands that repair the machine. Empty text when nothing is
/// missing.
fn doctor_report(checkout: &Checkout, run: &mut Runner<'_>) -> String {
    let report = match run(&node_step(checkout, WEB_DOCTOR, true)) {
        Ok(outcome) => outcome.output,
        Err(error) => return format!("; node {WEB_DOCTOR} could not run: {error}"),
    };
    let lines: Vec<&str> = report
        .lines()
        .filter(|line| {
            line.starts_with("MISSING ")
                || line.trim_start().starts_with("fix:")
                || line.starts_with("web target:")
        })
        .collect();
    if lines.iter().any(|line| line.starts_with("MISSING ")) {
        format!(
            "; the machine is missing pieces (from node {WEB_DOCTOR}):\n{}",
            lines.join("\n")
        )
    } else {
        format!("; node {WEB_DOCTOR} reports nothing missing")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{execute, TempDir, Verb};
    use crate::workspace;
    use std::fs;
    use std::io;
    use std::io::Error as IoError;
    use std::io::ErrorKind;

    const DOCTOR_SAYS: &str = "OK node 24.1.0\nOK rust-target wasm32-unknown-unknown 1.97.1\nMISSING wasm-bindgen (Cargo.lock needs 0.2.100)\n  fix: cargo install wasm-bindgen-cli --version 0.2.100 --locked\nweb target: NOT READY 1 missing\n";

    /// A fake engine checkout with the web tools in place.
    fn checkout(temp: &TempDir, os: &'static str) -> Checkout {
        let root = temp.0.join("engine");
        fs::create_dir_all(root.join("tools")).unwrap();
        fs::write(root.join(WEB_BUILD), "").unwrap();
        // The lock resolves the game crate: the fake workspace carries the crate it names.
        let game_crate = root.join("crates").join("game");
        fs::create_dir_all(game_crate.join("src")).unwrap();
        fs::write(
            game_crate.join("Cargo.toml"),
            "[package]\nname = \"engine-game\"\nversion.workspace = true\n",
        )
        .unwrap();
        fs::write(game_crate.join("src").join("lib.rs"), "").unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.package]\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        Checkout {
            target_dir: root.join("target"),
            root,
            os,
            exe_suffix: if os == "windows" { ".exe" } else { "" },
        }
    }

    fn reply(code: i32, output: &str) -> io::Result<process::Outcome> {
        Ok(process::Outcome {
            code: Some(code),
            output: output.to_owned(),
        })
    }

    /// The sha256 of `bytes`: the oracle the export receipt has to match.
    fn sha256_dev(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut hex = String::with_capacity(64);
        for byte in Sha256::digest(bytes) {
            hex.push_str(&format!("{byte:02x}"));
        }
        hex
    }

    /// What `certutil -hashfile <file> SHA256` prints for the file the export staged.
    fn certutil_reply(step: &Step) -> io::Result<process::Outcome> {
        let file = &step.args[1];
        let digest = sha256_dev(&fs::read(file).unwrap());
        reply(
            0,
            &format!(
                "SHA256 hash of {file}:\n{digest}\nCertUtil: -hashfile command completed successfully.\n"
            ),
        )
    }

    fn text(out: Vec<u8>) -> String {
        String::from_utf8(out).unwrap()
    }

    /// `build` for a project that names no game: the engine's own arena.
    fn build_arena(
        checkout: &Checkout,
        target: Target,
        run: &mut Runner<'_>,
        out: &mut dyn Write,
    ) -> Result<(), Error> {
        build(checkout, target, &workspace::default_game(), run, out).map(|_| ())
    }

    #[test]
    fn windows_build_runs_the_release_arena_command_and_prints_the_exe() {
        let temp = TempDir::new();
        let env = checkout(&temp, "windows");
        fs::create_dir_all(env.target_dir.join("release")).unwrap();
        fs::write(env.target_dir.join("release/arena.exe"), "").unwrap();
        let mut seen: Vec<String> = Vec::new();
        let mut out = Vec::new();
        let mut run = |step: &Step| {
            assert_eq!(step.dir, env.root);
            assert!(!step.capture);
            seen.push(format!(
                "{} {}",
                step.program.display(),
                step.args.join(" ")
            ));
            reply(0, "")
        };
        build_arena(&env, Target::Windows, &mut run, &mut out).unwrap();
        assert_eq!(
            seen,
            ["cargo build --locked --release -p engine-game --features app --bin arena"]
        );
        let exe = env.target_dir.join("release").join("arena.exe");
        assert_eq!(text(out), format!("built windows: {}\n", exe.display()));
    }

    #[test]
    fn windows_build_refuses_other_machines_before_running_anything() {
        let temp = TempDir::new();
        let env = checkout(&temp, "linux");
        let mut ran = false;
        let mut run = |_: &Step| {
            ran = true;
            reply(0, "")
        };
        let error = build_arena(&env, Target::Windows, &mut run, &mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains("this one is linux"), "{error}");
        assert!(!ran);
    }

    #[test]
    fn windows_build_failure_and_missing_exe_are_loud() {
        let temp = TempDir::new();
        let env = checkout(&temp, "windows");
        let mut failing = |_: &Step| reply(101, "");
        let error = build_arena(&env, Target::Windows, &mut failing, &mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains("exit code 101"), "{error}");
        let mut lying = |_: &Step| reply(0, "");
        let error = build_arena(&env, Target::Windows, &mut lying, &mut Vec::new()).unwrap_err();
        let shown = error.to_string();
        assert!(shown.contains("arena.exe is missing"), "{shown}");
        assert!(shown.contains("CARGO_TARGET_DIR"), "{shown}");
        let mut gone = |_: &Step| Err(IoError::from(ErrorKind::NotFound));
        let error = build_arena(&env, Target::Windows, &mut gone, &mut Vec::new()).unwrap_err();
        assert!(
            error.to_string().contains("`cargo` was not found"),
            "{error}"
        );
    }

    #[test]
    fn web_build_runs_node_and_prints_dist_web() {
        let temp = TempDir::new();
        let env = checkout(&temp, "windows");
        let dist = env.root.join("dist").join("web");
        let mut seen: Vec<String> = Vec::new();
        let mut out = Vec::new();
        let mut run = |step: &Step| {
            seen.push(format!(
                "{} {}",
                step.program.display(),
                step.args.join(" ")
            ));
            fs::create_dir_all(&dist).unwrap();
            fs::write(dist.join("index.html"), "<!doctype html>").unwrap();
            reply(0, "")
        };
        build_arena(&env, Target::Web, &mut run, &mut out).unwrap();
        assert_eq!(
            seen,
            ["node tools/web-build.mjs"],
            "the doctor only runs on failure"
        );
        assert_eq!(text(out), format!("built web: {}\n", dist.display()));
    }

    #[test]
    fn web_build_failure_prints_the_doctor_fix_lines() {
        let temp = TempDir::new();
        let env = checkout(&temp, "windows");
        let mut seen: Vec<String> = Vec::new();
        let mut run = |step: &Step| {
            seen.push(format!(
                "{} {}",
                step.program.display(),
                step.args.join(" ")
            ));
            if step.capture {
                reply(1, DOCTOR_SAYS)
            } else {
                reply(1, "")
            }
        };
        let error = build_arena(&env, Target::Web, &mut run, &mut Vec::new()).unwrap_err();
        let shown = error.to_string();
        assert!(shown.contains("exit code 1"), "{shown}");
        assert!(
            shown.contains("MISSING wasm-bindgen (Cargo.lock needs 0.2.100)"),
            "{shown}"
        );
        assert!(
            shown.contains("fix: cargo install wasm-bindgen-cli --version 0.2.100 --locked"),
            "{shown}"
        );
        assert!(shown.contains("web target: NOT READY 1 missing"), "{shown}");
        assert!(
            !shown.contains("OK node"),
            "only the broken pieces: {shown}"
        );
        assert_eq!(
            seen,
            ["node tools/web-build.mjs", "node tools/web-doctor.mjs"]
        );
    }

    #[test]
    fn web_build_failure_with_a_ready_machine_says_so() {
        let temp = TempDir::new();
        let env = checkout(&temp, "windows");
        let mut run = |step: &Step| {
            if step.capture {
                reply(0, "OK node 24.1.0\nweb target: READY\n")
            } else {
                reply(1, "")
            }
        };
        let error = build_arena(&env, Target::Web, &mut run, &mut Vec::new()).unwrap_err();
        assert!(
            error.to_string().contains("reports nothing missing"),
            "{error}"
        );
    }

    #[test]
    fn web_build_without_node_or_dist_is_loud() {
        let temp = TempDir::new();
        let env = checkout(&temp, "windows");
        let mut gone = |_: &Step| Err(IoError::from(ErrorKind::NotFound));
        let error = build_arena(&env, Target::Web, &mut gone, &mut Vec::new()).unwrap_err();
        let shown = error.to_string();
        assert!(shown.contains("`node` was not found on PATH"), "{shown}");
        assert!(shown.contains("OpenJS.NodeJS.LTS"), "{shown}");
        let mut empty = |_: &Step| reply(0, "");
        let error = build_arena(&env, Target::Web, &mut empty, &mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains("has no index.html"), "{error}");
        fs::remove_file(env.root.join(WEB_BUILD)).unwrap();
        let error = build_arena(&env, Target::Web, &mut empty, &mut Vec::new()).unwrap_err();
        assert!(
            error.to_string().contains("tools/web-build.mjs is missing"),
            "{error}"
        );
    }

    fn words(args: &[&str]) -> impl Iterator<Item = OsString> {
        args.iter()
            .map(OsString::from)
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn new_template_generated_project_builds_both_targets_through_the_verb() {
        // What `new` wrote is what `build` reads: the generated manifest passes the verb's
        // checks and each target of it runs its one command and prints where the result is.
        let temp = TempDir::new();
        let dir = temp.0.join("duel-1");
        execute(
            Verb::New,
            words(&["arena-duel", dir.to_str().unwrap()]),
            &mut Vec::new(),
        )
        .unwrap();
        let env = checkout(&temp, "windows");
        let exe = env.target_dir.join("release").join("arena.exe");
        let dist = env.root.join("dist").join("web");
        let manifest = dir.join("game.toml");
        let manifest = manifest.to_str().unwrap();
        let mut seen: Vec<String> = Vec::new();
        let mut out = Vec::new();
        let mut run = |step: &Step| {
            seen.push(format!(
                "{} {}",
                step.program.display(),
                step.args.join(" ")
            ));
            if step.program.ends_with("tar") {
                fs::write(&step.args[3], "").unwrap();
                return reply(0, "");
            }
            if step.program.ends_with("certutil") {
                return certutil_reply(step);
            }
            fs::create_dir_all(env.target_dir.join("release")).unwrap();
            fs::write(&exe, "").unwrap();
            fs::create_dir_all(&dist).unwrap();
            fs::write(dist.join("index.html"), "<!doctype html>").unwrap();
            reply(0, "")
        };
        for target in ["windows", "web"] {
            build_verb_with(
                words(&["--target", target, "--manifest", manifest]),
                &env,
                &mut run,
                &mut out,
            )
            .unwrap();
        }
        assert_eq!(
            seen[0],
            "cargo build --locked --release -p engine-game --features app --bin arena"
        );
        assert!(
            seen[1].starts_with("certutil -hashfile ") && seen[1].ends_with("arena.exe SHA256"),
            "the export hashes the staged exe: {}",
            seen[1]
        );
        assert!(
            seen[2].starts_with("tar -a -c -f ") && seen[2].contains("duel-1.zip"),
            "one archive step after the build: {}",
            seen[2]
        );
        assert_eq!(seen[3], "node tools/web-build.mjs");
        let printed = text(out);
        assert!(
            printed.starts_with(&format!("built windows: {}\n", exe.display())),
            "{printed}"
        );
        assert!(
            printed.contains(&format!(
                "exported duel-1: {}",
                dir.join("dist").join("duel-1").display()
            )),
            "{printed}"
        );
        assert_eq!(
            printed.lines().last().unwrap(),
            format!("built web: {}", dist.display())
        );
    }

    #[test]
    fn build_verb_refuses_unknown_targets_and_targets_the_manifest_lacks() {
        for args in [&["--target", "linux"][..], &["--target", "Web"], &[]] {
            let error = build_verb(words(args), &mut Vec::new()).unwrap_err();
            assert_eq!(error.exit_code(), 2, "{args:?}");
            assert!(
                error.to_string().contains("valid targets: windows, web"),
                "{error}"
            );
        }
        let temp = TempDir::new();
        let dir = temp.0.join("web-only");
        execute(
            Verb::New,
            words(&["arena-duel", dir.to_str().unwrap()]),
            &mut Vec::new(),
        )
        .unwrap();
        let manifest = dir.join("game.toml");
        let edited = fs::read_to_string(&manifest)
            .unwrap()
            .replace("targets = [\"windows\", \"web\"]", "targets = [\"web\"]");
        fs::write(&manifest, edited).unwrap();
        let path = manifest.to_str().unwrap();
        let error = build_verb(
            words(&["--target", "windows", "--manifest", path]),
            &mut Vec::new(),
        )
        .unwrap_err();
        let shown = error.to_string();
        assert!(
            shown.contains("key `targets` does not list `windows`"),
            "{shown}"
        );
        assert!(shown.contains("it lists: web"), "{shown}");
        assert_eq!(error.exit_code(), 1);
    }

    /// A checkout whose root is the real workspace (so a declared `game` resolves) and whose
    /// target directory is fresh, so nothing is built.
    fn real_workspace_checkout(temp: &TempDir) -> Checkout {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
        Checkout {
            target_dir: temp.0.join("target"),
            root,
            os: "windows",
            exe_suffix: ".exe",
        }
    }

    fn project_with(temp: &TempDir, name: &str, manifest: &str) -> std::path::PathBuf {
        let dir = temp.0.join(name);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("game.toml");
        fs::write(&path, manifest).unwrap();
        path
    }

    #[test]
    fn windows_build_uses_the_package_and_bin_the_manifest_names() {
        let temp = TempDir::new();
        let checkout = real_workspace_checkout(&temp);
        let path = project_with(
            &temp,
            "mcp-game",
            "name = \"a\"\ntargets = [\"windows\"]\ngame = \"engine-endpoint:e2040-mcp\"\n",
        );
        let exe = checkout.exe("release", "e2040-mcp");
        let mut seen: Vec<String> = Vec::new();
        let mut run = |step: &Step| {
            if step.program.ends_with("certutil") {
                return certutil_reply(step);
            }
            if step.program.ends_with("tar") {
                fs::write(&step.args[3], "").unwrap();
                return reply(0, "");
            }
            seen.push(format!(
                "{} {}",
                step.program.display(),
                step.args.join(" ")
            ));
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            fs::write(&exe, "").unwrap();
            reply(0, "")
        };
        build_verb_with(
            words(&["--target", "windows", "--manifest", path.to_str().unwrap()]),
            &checkout,
            &mut run,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            seen,
            ["cargo build --locked --release -p engine-endpoint --features mcp --bin e2040-mcp"],
            "the bin's own package, required features and name"
        );
    }

    #[test]
    fn windows_build_exports_a_folder_with_a_hashed_build_json() {
        // WG11 first half: the build stages `<project>/dist/<name>/`, writes `build.json`
        // (name, version, target, one sha256 per file) and zips the folder in one step.
        let temp = TempDir::new();
        let env = checkout(&temp, "windows");
        let dir = temp.0.join("duel-1");
        fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("game.toml");
        fs::write(&manifest, "name = \"duel-1\"\ntargets = [\"windows\"]\n").unwrap();
        let exe = env.target_dir.join("release").join("arena.exe");
        let payload = b"MZ a fake release exe".to_vec();
        let mut seen: Vec<String> = Vec::new();
        let mut run = |step: &Step| {
            seen.push(format!(
                "{} {}",
                step.program.display(),
                step.args.join(" ")
            ));
            if step.program.ends_with("tar") {
                fs::write(&step.args[3], "").unwrap();
                return reply(0, "");
            }
            if step.program.ends_with("certutil") {
                return certutil_reply(step);
            }
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            fs::write(&exe, &payload).unwrap();
            reply(0, "")
        };
        let args = words(&[
            "--target",
            "windows",
            "--manifest",
            manifest.to_str().unwrap(),
        ]);
        let mut out = Vec::new();
        build_verb_with(args, &env, &mut run, &mut out).unwrap();

        let dist_root = dir.join("dist");
        let dist = dist_root.join("duel-1");
        assert_eq!(
            fs::read(dist.join("arena.exe")).unwrap(),
            payload,
            "the exe is copied into the export"
        );
        let digest = sha256_dev(&payload);
        let json = fs::read_to_string(dist.join("build.json")).unwrap();
        assert_eq!(
            json,
            format!(
                "{{\n  \"schema\": 1,\n  \"name\": \"duel-1\",\n  \"version\": \"{}\",\n  \"target\": \"windows\",\n  \"game\": \"engine-game:arena\",\n  \"files\": [\n    {{\"path\": \"arena.exe\", \"bytes\": {}, \"sha256\": \"{digest}\"}}\n  ],\n  \"archive\": \"dist/duel-1.zip\"\n}}\n",
                env!("CARGO_PKG_VERSION"),
                payload.len()
            ),
            "the receipt hashes the built exe"
        );
        let archive = dist_root.join("duel-1.zip");
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert_eq!(
            seen[1],
            format!(
                "certutil -hashfile {} SHA256",
                dist.join("arena.exe").display()
            ),
            "the platform tool hashes the staged file"
        );
        assert_eq!(
            seen[2],
            format!(
                "tar -a -c -f {} -C {} duel-1",
                archive.display(),
                dist_root.display()
            ),
            "one archive step, no shell"
        );
        assert!(archive.is_file(), "the scripted archive exists");
        let printed = text(out);
        assert!(
            printed.contains(&format!("exported duel-1: {}", dist.display())),
            "{printed}"
        );
        assert!(printed.contains(&digest), "{printed}");

        // A second export replaces its own folder, so no stale file sits beside the receipt.
        fs::write(dist.join("stale.txt"), "old").unwrap();
        let mut again = |step: &Step| {
            if step.program.ends_with("certutil") {
                return certutil_reply(step);
            }
            if step.program.ends_with("tar") {
                fs::write(&step.args[3], "").unwrap();
            }
            reply(0, "")
        };
        build_verb_with(
            words(&[
                "--target",
                "windows",
                "--manifest",
                manifest.to_str().unwrap(),
            ]),
            &env,
            &mut again,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(
            !dist.join("stale.txt").exists(),
            "the export folder is rebuilt"
        );
        assert_eq!(fs::read_to_string(dist.join("build.json")).unwrap(), json);
    }

    #[test]
    fn the_sha256_line_is_the_one_64_hex_line() {
        let printed = "SHA256 hash of C:\\x\\arena.exe:\n\
            ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789\n\
            CertUtil: -hashfile command completed successfully.\n";
        assert_eq!(
            sha256_line(printed).unwrap(),
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );
        assert_eq!(sha256_line("CertUtil: -hashfile FAILED\n"), None);
        assert_eq!(sha256_line(""), None);
    }

    #[test]
    fn windows_export_names_a_failed_archive_step() {
        let temp = TempDir::new();
        let env = checkout(&temp, "windows");
        let dir = temp.0.join("duel-1");
        fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("game.toml");
        fs::write(&manifest, "name = \"duel-1\"\ntargets = [\"windows\"]\n").unwrap();
        let exe = env.target_dir.join("release").join("arena.exe");
        let mut run = |step: &Step| {
            if step.program.ends_with("certutil") {
                return certutil_reply(step);
            }
            if step.program.ends_with("tar") {
                return reply(9, "");
            }
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            fs::write(&exe, b"MZ").unwrap();
            reply(0, "")
        };
        let error = build_verb_with(
            words(&[
                "--target",
                "windows",
                "--manifest",
                manifest.to_str().unwrap(),
            ]),
            &env,
            &mut run,
            &mut Vec::new(),
        )
        .unwrap_err();
        let shown = error.to_string();
        assert!(shown.contains("archive step failed"), "{shown}");
        assert!(shown.contains("duel-1.zip is missing"), "{shown}");
    }

    #[test]
    fn web_build_refuses_a_game_that_is_not_the_arena_page() {
        let temp = TempDir::new();
        let checkout = real_workspace_checkout(&temp);
        let path = project_with(
            &temp,
            "mcp-web",
            "name = \"a\"\ntargets = [\"web\"]\ngame = \"engine-endpoint:e2040-mcp\"\n",
        );
        let mut ran = false;
        let mut run = |_: &Step| {
            ran = true;
            reply(0, "")
        };
        let error = build_verb_with(
            words(&["--target", "web", "--manifest", path.to_str().unwrap()]),
            &checkout,
            &mut run,
            &mut Vec::new(),
        )
        .unwrap_err();
        let shown = error.to_string();
        assert!(
            shown.contains("not wired yet: engine-endpoint:e2040-mcp"),
            "{shown}"
        );
        assert!(shown.contains(WEB_BUILD), "{shown}");
        assert!(!ran, "nothing is built once the game cannot be exported");
    }

    /// A project that names no game, with a runner that fakes a successful windows build.
    fn lock_project(temp: &TempDir, env: &Checkout, name: &str) -> (std::path::PathBuf, PathBuf) {
        let dir = temp.0.join(name);
        fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("game.toml");
        fs::write(
            &manifest,
            format!("name = \"{name}\"\ntargets = [\"windows\"]\n"),
        )
        .unwrap();
        let exe = env.target_dir.join("release").join("arena.exe");
        (manifest, exe)
    }

    /// A runner that stages the arena exe, hashes and archives it: a windows build that works.
    fn passing_build(exe: &Path) -> impl FnMut(&Step) -> io::Result<process::Outcome> {
        let exe = exe.to_path_buf();
        move |step: &Step| {
            if step.program.ends_with("certutil") {
                return certutil_reply(step);
            }
            if step.program.ends_with("tar") {
                fs::write(&step.args[3], "").unwrap();
                return reply(0, "");
            }
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            fs::write(&exe, b"MZ").unwrap();
            reply(0, "")
        }
    }

    /// F2P: the first build writes `project.lock` and the next build reads it back; moving the
    /// crate's content refuses, naming the row.
    #[test]
    fn build_writes_the_project_lock_and_refuses_a_moved_crate() {
        let temp = TempDir::new();
        let env = checkout(&temp, "windows");
        let (manifest, exe) = lock_project(&temp, &env, "lock-duel");
        let words_of = |manifest: &Path| {
            words(&[
                "--target",
                "windows",
                "--manifest",
                manifest.to_str().unwrap(),
            ])
        };
        let mut first = passing_build(&exe);
        let mut out = Vec::new();
        build_verb_with(words_of(&manifest), &env, &mut first, &mut out).unwrap();
        let lock = manifest.parent().unwrap().join(crate::project::LOCK_FILE);
        let text = fs::read_to_string(&lock).expect("the first build writes the lock");
        assert!(
            text.lines()
                .any(|line| line.starts_with("engine-game 0.1.0 crate:crates/game ")),
            "{text}"
        );
        // The same rows on the next build: the lock is read, not rewritten.
        let mut second = passing_build(&exe);
        build_verb_with(words_of(&manifest), &env, &mut second, &mut Vec::new()).unwrap();
        // Moving the crate's content moves its hash, so the row refuses by name.
        fs::write(
            env.root.join("crates").join("game").join("src/lib.rs"),
            "// moved\n",
        )
        .unwrap();
        let mut third = passing_build(&exe);
        let error =
            build_verb_with(words_of(&manifest), &env, &mut third, &mut Vec::new()).unwrap_err();
        let shown = error.to_string();
        assert!(shown.contains("row `engine-game` moved"), "{shown}");
        assert!(shown.contains(&lock.display().to_string()), "{shown}");
    }

    /// F2P: a lock without a dependency row refuses the build, naming the missing row.
    #[test]
    fn build_refuses_a_dependency_the_lock_does_not_hold() {
        let temp = TempDir::new();
        let env = checkout(&temp, "windows");
        let (manifest, exe) = lock_project(&temp, &env, "stale-duel");
        fs::write(
            manifest.parent().unwrap().join(crate::project::LOCK_FILE),
            "# engine2040 project lock\n",
        )
        .unwrap();
        let mut run = passing_build(&exe);
        let error = build_verb_with(
            words(&[
                "--target",
                "windows",
                "--manifest",
                manifest.to_str().unwrap(),
            ]),
            &env,
            &mut run,
            &mut Vec::new(),
        )
        .unwrap_err();
        let shown = error.to_string();
        assert!(shown.contains("no row for `engine-game`"), "{shown}");
    }
}
