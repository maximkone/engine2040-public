//! The game-project verbs through the real `engine2040` binary: make a project, run it twice,
//! break its manifest, and see the exit codes and messages a caller (or an AI) gets.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        loop {
            let path = std::env::temp_dir().join(format!(
                "engine2040-project-cli-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("temporary directory: {error}"),
            }
        }
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn engine2040(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_engine2040"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn new_arena_duel_then_run_headless_prints_the_golden_receipt_twice() {
    let temp = Temp::new();
    let created = engine2040(&temp.0, &["new", "arena-duel", "duel-1"]);
    assert!(stdout(&created).contains("created project `duel-1`"));
    let project = temp.0.join("duel-1");
    for file in ["game.toml", "README.md", "content/seed.txt"] {
        assert!(project.join(file).is_file(), "{file}");
    }
    let args = ["run", "--headless", "--ticks", "600"];
    let first = stdout(&engine2040(&project, &args));
    let second = stdout(&engine2040(&project, &args));
    assert_eq!(first, second);
    assert!(first.contains("final_hash=d57b5c8ae3096614"), "{first}");
    assert!(first.contains("engine2040 headless proof:"), "{first}");
    // The manifest can also come from outside the project directory.
    let from_outside = stdout(&engine2040(
        &temp.0,
        &["run", "--headless", "--manifest", "duel-1/game.toml"],
    ));
    assert_eq!(
        first, from_outside,
        "ticks default to the 600-tick golden length"
    );
}

#[test]
fn bad_manifest_exits_1_naming_the_key_and_the_line() {
    let temp = Temp::new();
    stdout(&engine2040(&temp.0, &["new", "siege", "siege-1"]));
    let project = temp.0.join("siege-1");
    let path = project.join("game.toml");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        text.replace("targets = [\"windows\", \"web\"]", "targets = [\"mac\"]"),
    )
    .unwrap();
    let line = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .position(|row| row.starts_with("targets"))
        .unwrap()
        + 1;
    let run = engine2040(&project, &["run", "--headless"]);
    assert_eq!(run.status.code(), Some(1));
    let shown = stderr(&run);
    assert!(
        shown.contains(&format!("line {line}, key `targets`")),
        "{shown}"
    );
    assert!(shown.contains("valid values: windows, web"), "{shown}");
}

#[test]
fn new_refuses_a_non_empty_directory_and_unknown_template() {
    let temp = Temp::new();
    fs::create_dir(temp.0.join("taken")).unwrap();
    fs::write(temp.0.join("taken").join("mine.txt"), "mine").unwrap();
    let taken = engine2040(&temp.0, &["new", "arena-duel", "taken"]);
    assert_eq!(taken.status.code(), Some(1));
    assert!(stderr(&taken).contains("is not empty"));
    assert_eq!(fs::read_dir(temp.0.join("taken")).unwrap().count(), 1);
    let unknown = engine2040(&temp.0, &["new", "moba", "fresh"]);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(stderr(&unknown).contains("valid templates: arena-duel, siege"));
    assert!(!temp.0.join("fresh").exists());
}

#[test]
fn build_refuses_an_unknown_target_with_the_valid_ones() {
    let temp = Temp::new();
    let output = engine2040(&temp.0, &["build", "--target", "ps5"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("valid targets: windows, web"));
    let no_manifest = engine2040(&temp.0, &["build", "--target", "web"]);
    assert_eq!(no_manifest.status.code(), Some(1));
    assert!(stderr(&no_manifest).contains("cannot read game.toml"));
}

#[test]
fn the_golden_recorder_still_runs_beside_the_project_verbs() {
    let temp = Temp::new();
    let text = stdout(&engine2040(&temp.0, &["--verify"]));
    assert!(
        text.contains("ticks=600 final_hash=d57b5c8ae3096614"),
        "{text}"
    );
    assert!(text.contains("mode=run verified=true"), "{text}");
    assert!(stdout(&engine2040(&temp.0, &["--help"])).contains("run --headless"));
}
