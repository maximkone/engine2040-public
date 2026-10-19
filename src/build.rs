//! The `build` verb of the `engine2040` bin: build one target of a project's game and print
//! where the result is. `windows` runs the cargo release build of the `arena` game bin; `web`
//! runs `node tools/web-build.mjs` and, when that fails, adds what `node tools/web-doctor.mjs`
//! says is missing, so the fix is one copy away. Both builds run in the engine checkout this
//! exe was built from. Every command goes through a runner, so tests drive the steps with a
//! script instead of a toolchain.

use crate::cli::Error;
use crate::manifest::{joined, Choice, Target};
use crate::project::{parse_target, Flags, Project};
use std::ffi::OsString;
use std::io::{self, ErrorKind, Write};
use std::path::PathBuf;
use std::process::Command;

/// The web build tool, relative to the engine checkout.
const WEB_BUILD: &str = "tools/web-build.mjs";
/// The tool that lists what the web target still needs, with one fix command per piece.
const WEB_DOCTOR: &str = "tools/web-doctor.mjs";
/// Fix lines for the two programs the builds start themselves.
const NODE_FIX: &str = "winget install --id OpenJS.NodeJS.LTS -e (Node 24 or newer)";
const CARGO_FIX: &str = "winget install --id Rustlang.Rustup -e";

/// One command a build runs.
struct Step {
    program: &'static str,
    args: Vec<&'static str>,
    dir: PathBuf,
    /// Hold stdout for the caller instead of showing it live.
    capture: bool,
    /// What to run when the program is not installed.
    fix: &'static str,
}

/// How a step ended: its exit code (none when a signal killed it) and, when captured, stdout.
struct Outcome {
    code: Option<i32>,
    output: String,
}

type Runner<'a> = dyn FnMut(&Step) -> io::Result<Outcome> + 'a;

/// Where the builds run and where their results land.
struct BuildEnv {
    /// The engine checkout: holds `tools/` and the workspace `Cargo.toml`.
    root: PathBuf,
    /// Cargo's target directory.
    target_dir: PathBuf,
    /// `std::env::consts::OS` of the machine.
    os: &'static str,
    /// `std::env::consts::EXE_SUFFIX` of the machine.
    exe_suffix: &'static str,
}

impl BuildEnv {
    fn host() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let target_dir = std::env::var_os("CARGO_TARGET_DIR")
            .map_or_else(|| root.join("target"), |dir| root.join(dir));
        Self {
            root,
            target_dir,
            os: std::env::consts::OS,
            exe_suffix: std::env::consts::EXE_SUFFIX,
        }
    }
}

/// Start the process; a build step's output is shown live, a captured one is returned.
fn run_process(step: &Step) -> io::Result<Outcome> {
    let mut command = Command::new(step.program);
    command.args(&step.args).current_dir(&step.dir);
    if step.capture {
        let output = command.output()?;
        Ok(Outcome {
            code: output.status.code(),
            output: String::from_utf8_lossy(&output.stdout).into_owned(),
        })
    } else {
        Ok(Outcome {
            code: command.status()?.code(),
            output: String::new(),
        })
    }
}

/// `build --target <windows|web> [--manifest <path>]`.
pub fn build_verb(args: impl Iterator<Item = OsString>, out: &mut dyn Write) -> Result<(), Error> {
    let env = BuildEnv::host();
    let mut run = run_process;
    build_verb_with(args, &env, &mut run, out)
}

/// The `build` verb against a chosen checkout and runner: the manifest check and the target
/// dispatch that [`build_verb`] runs on the host. Tests drive it with a script instead of a
/// toolchain, so a generated project's manifest and both target command lines are checked.
fn build_verb_with(
    args: impl Iterator<Item = OsString>,
    env: &BuildEnv,
    run: &mut Runner<'_>,
    out: &mut dyn Write,
) -> Result<(), Error> {
    let flags = Flags::parse(args, &[], &["--target", "--manifest"])?;
    flags.no_positional("build")?;
    let target = parse_target(flags.text("--target")?)?;
    let manifest_path = flags.manifest();
    let project = Project::load(&manifest_path)?;
    if !project.manifest.targets.contains(&target) {
        return Err(Error::Project(format!(
            "{}: key `targets` does not list `{}` (it lists: {}); add it there to build for it",
            manifest_path.display(),
            target.word(),
            joined(&project.manifest.targets, ", ")
        )));
    }
    build(env, target, run, out)
}

fn build(
    env: &BuildEnv,
    target: Target,
    run: &mut Runner<'_>,
    out: &mut dyn Write,
) -> Result<(), Error> {
    if !env.root.is_dir() {
        return Err(Error::Project(format!(
            "the engine checkout {} is missing: build runs in the checkout this exe was built from",
            env.root.display()
        )));
    }
    match target {
        Target::Windows => build_windows(env, run, out),
        Target::Web => build_web(env, run, out),
    }
}

/// Start `step`; a program that is not installed is reported with its install command.
fn start(run: &mut Runner<'_>, step: &Step) -> Result<Outcome, Error> {
    run(step).map_err(|error| {
        if error.kind() == ErrorKind::NotFound {
            Error::Project(format!(
                "`{}` was not found on PATH; install it: {}",
                step.program, step.fix
            ))
        } else {
            Error::Project(format!("cannot start `{}`: {error}", step.program))
        }
    })
}

fn exit_text(code: Option<i32>) -> String {
    code.map_or_else(
        || "ended by a signal".to_owned(),
        |code| format!("exit code {code}"),
    )
}

fn build_windows(env: &BuildEnv, run: &mut Runner<'_>, out: &mut dyn Write) -> Result<(), Error> {
    if env.os != "windows" {
        return Err(Error::Project(format!(
            "target windows builds on a Windows machine; this one is {}",
            env.os
        )));
    }
    let step = Step {
        program: "cargo",
        args: vec![
            "build",
            "--locked",
            "--release",
            "-p",
            "engine-game",
            "--features",
            "app",
            "--bin",
            "arena",
        ],
        dir: env.root.clone(),
        capture: false,
        fix: CARGO_FIX,
    };
    let outcome = start(run, &step)?;
    if outcome.code != Some(0) {
        return Err(Error::Project(format!(
            "cargo build failed ({}); the compiler output above says why",
            exit_text(outcome.code)
        )));
    }
    let exe = env
        .target_dir
        .join("release")
        .join(format!("arena{}", env.exe_suffix));
    if !exe.is_file() {
        return Err(Error::Project(format!(
            "cargo finished but {} is missing; if the target directory moved, set CARGO_TARGET_DIR",
            exe.display()
        )));
    }
    writeln!(out, "built windows: {}", exe.display())?;
    Ok(())
}

fn build_web(env: &BuildEnv, run: &mut Runner<'_>, out: &mut dyn Write) -> Result<(), Error> {
    if !env.root.join(WEB_BUILD).is_file() {
        return Err(Error::Project(format!(
            "{WEB_BUILD} is missing from the engine checkout {}",
            env.root.display()
        )));
    }
    let outcome = start(run, &node_step(env, WEB_BUILD, false))?;
    if outcome.code != Some(0) {
        let head = format!(
            "node {WEB_BUILD} failed ({}); its output above names the step",
            exit_text(outcome.code)
        );
        return Err(Error::Project(format!("{head}{}", doctor_report(env, run))));
    }
    let dist = env.root.join("dist").join("web");
    if !dist.join("index.html").is_file() {
        return Err(Error::Project(format!(
            "{WEB_BUILD} finished but {} has no index.html",
            dist.display()
        )));
    }
    writeln!(out, "built web: {}", dist.display())?;
    Ok(())
}

fn node_step(env: &BuildEnv, script: &'static str, capture: bool) -> Step {
    Step {
        program: "node",
        args: vec![script],
        dir: env.root.clone(),
        capture,
        fix: NODE_FIX,
    }
}

/// What the web doctor says is missing: its MISSING and `fix:` lines and the verdict line, so a
/// failed web build ends with the commands that repair the machine. Empty text when nothing is
/// missing.
fn doctor_report(env: &BuildEnv, run: &mut Runner<'_>) -> String {
    let report = match run(&node_step(env, WEB_DOCTOR, true)) {
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
    use std::fs;
    use std::io::Error as IoError;

    const DOCTOR_SAYS: &str = "OK node 24.1.0\nOK rust-target wasm32-unknown-unknown 1.97.1\nMISSING wasm-bindgen (Cargo.lock needs 0.2.100)\n  fix: cargo install wasm-bindgen-cli --version 0.2.100 --locked\nweb target: NOT READY 1 missing\n";

    /// A fake engine checkout with the web tools in place.
    fn checkout(temp: &TempDir, os: &'static str) -> BuildEnv {
        let root = temp.0.join("engine");
        fs::create_dir_all(root.join("tools")).unwrap();
        fs::write(root.join(WEB_BUILD), "").unwrap();
        BuildEnv {
            target_dir: root.join("target"),
            root,
            os,
            exe_suffix: if os == "windows" { ".exe" } else { "" },
        }
    }

    fn reply(code: i32, output: &str) -> io::Result<Outcome> {
        Ok(Outcome {
            code: Some(code),
            output: output.to_owned(),
        })
    }

    fn text(out: Vec<u8>) -> String {
        String::from_utf8(out).unwrap()
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
            seen.push(format!("{} {}", step.program, step.args.join(" ")));
            reply(0, "")
        };
        build(&env, Target::Windows, &mut run, &mut out).unwrap();
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
        let error = build(&env, Target::Windows, &mut run, &mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains("this one is linux"), "{error}");
        assert!(!ran);
    }

    #[test]
    fn windows_build_failure_and_missing_exe_are_loud() {
        let temp = TempDir::new();
        let env = checkout(&temp, "windows");
        let mut failing = |_: &Step| reply(101, "");
        let error = build(&env, Target::Windows, &mut failing, &mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains("exit code 101"), "{error}");
        let mut lying = |_: &Step| reply(0, "");
        let error = build(&env, Target::Windows, &mut lying, &mut Vec::new()).unwrap_err();
        let shown = error.to_string();
        assert!(shown.contains("arena.exe is missing"), "{shown}");
        assert!(shown.contains("CARGO_TARGET_DIR"), "{shown}");
        let mut gone = |_: &Step| Err(IoError::from(ErrorKind::NotFound));
        let error = build(&env, Target::Windows, &mut gone, &mut Vec::new()).unwrap_err();
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
            seen.push(format!("{} {}", step.program, step.args.join(" ")));
            fs::create_dir_all(&dist).unwrap();
            fs::write(dist.join("index.html"), "<!doctype html>").unwrap();
            reply(0, "")
        };
        build(&env, Target::Web, &mut run, &mut out).unwrap();
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
            seen.push(format!("{} {}", step.program, step.args.join(" ")));
            if step.capture {
                reply(1, DOCTOR_SAYS)
            } else {
                reply(1, "")
            }
        };
        let error = build(&env, Target::Web, &mut run, &mut Vec::new()).unwrap_err();
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
        let error = build(&env, Target::Web, &mut run, &mut Vec::new()).unwrap_err();
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
        let error = build(&env, Target::Web, &mut gone, &mut Vec::new()).unwrap_err();
        let shown = error.to_string();
        assert!(shown.contains("`node` was not found on PATH"), "{shown}");
        assert!(shown.contains("OpenJS.NodeJS.LTS"), "{shown}");
        let mut empty = |_: &Step| reply(0, "");
        let error = build(&env, Target::Web, &mut empty, &mut Vec::new()).unwrap_err();
        assert!(error.to_string().contains("has no index.html"), "{error}");
        fs::remove_file(env.root.join(WEB_BUILD)).unwrap();
        let error = build(&env, Target::Web, &mut empty, &mut Vec::new()).unwrap_err();
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
            seen.push(format!("{} {}", step.program, step.args.join(" ")));
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
}
