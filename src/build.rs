//! The `build` verb of the `engine2040` bin: build one target of a project's game and print
//! where the result is. `windows` runs the cargo release build of the bin `game.toml` names
//! (the engine's `arena` when it names none); `web` runs `node tools/web-build.mjs` and, when
//! that fails, adds what `node tools/web-doctor.mjs` says is missing, so the fix is one copy
//! away. Both builds run in the engine checkout this exe was built from. Every command goes
//! through a runner, so tests drive the steps with a script instead of a toolchain.

use crate::cli::Error;
use crate::manifest::{joined, Choice, Target};
use crate::process::{self, Checkout, Runner, Step};
use crate::project::{parse_target, Flags, Project};
use crate::workspace::GameBin;
use std::ffi::OsString;
use std::io::Write;

/// The web build tool, relative to the engine checkout.
const WEB_BUILD: &str = "tools/web-build.mjs";
/// The tool that lists what the web target still needs, with one fix command per piece.
const WEB_DOCTOR: &str = "tools/web-doctor.mjs";
/// Fix lines for the two programs the builds start themselves.
const NODE_FIX: &str = "winget install --id OpenJS.NodeJS.LTS -e (Node 24 or newer)";
const CARGO_FIX: &str = "winget install --id Rustlang.Rustup -e";

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
    build(checkout, target, &project.game(), run, out)
}

fn build(
    checkout: &Checkout,
    target: Target,
    game: &GameBin,
    run: &mut Runner<'_>,
    out: &mut dyn Write,
) -> Result<(), Error> {
    if !checkout.root.is_dir() {
        return Err(Error::Project(format!(
            "the engine checkout {} is missing: build runs in the checkout this exe was built from",
            checkout.root.display()
        )));
    }
    match target {
        Target::Windows => build_windows(checkout, game, run, out),
        Target::Web => {
            if game.spec() != crate::workspace::default_game().spec() {
                return Err(Error::Project(format!(
                    "{WEB_BUILD} builds the engine's own arena page, not `{}`: web export of another game is not wired yet: {}. Build --target windows instead, or remove the `game` line",
                    game.spec(),
                    game.spec()
                )));
            }
            build_web(checkout, run, out)
        }
    }
}

fn build_windows(
    checkout: &Checkout,
    game: &GameBin,
    run: &mut Runner<'_>,
    out: &mut dyn Write,
) -> Result<(), Error> {
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
    Ok(())
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
        build(checkout, target, &workspace::default_game(), run, out)
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
            seen,
            [
                "cargo build --locked --release -p engine-game --features app --bin arena",
                "node tools/web-build.mjs"
            ]
        );
        assert_eq!(
            text(out),
            format!(
                "built windows: {}\nbuilt web: {}\n",
                exe.display(),
                dist.display()
            )
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
}
