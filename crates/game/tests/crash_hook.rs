//! EG4 crash hook: a forced panic leaves a crash report that validates, the
//! run log mirrors the status lines, and the process exits with the crash code.
//! Owner: game crate.
//!
//! Expected outcomes, written down before the runs:
//! - helper child panics after 5 ticks: exit 102 (`CrashKind::Panic`); report
//!   tick 5, mode fight, adapter as set, tape tail 5 x 10 bytes, checkpoint
//!   12 bytes (tick 4, its hash); the log holds the panic line and the status
//!   lines the helper said.
//! - same child with the report path taken by a directory: exit 103
//!   (`CrashKind::ReportFailed`), the log still holds the panic line.
//! - a worker thread panics and its owner joins it: the child exits 0 (alive),
//!   the log holds one `arena worker panic on` line, and no report is written.
//! - a worker panic re-raised on main with `resume_unwind`: exit 102 with a
//!   valid report, the log names the unwound panic.
//! - `arena --frames 600 --exit` with `E2040_TEST_PANIC_AT_TICK=5` (needs the
//!   `app` feature, so it builds only with it): exit 102, report tick 5, tape
//!   tail 50 bytes, and every stdout `arena ` line appears in the run log.
//!
//! The hook exits the process, so the panicking runs happen in child processes.
//! The helper is `#[ignore]`d and only the parent tests start it.

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use engine_game::crash::{self, CrashKind, CrashMode, CrashReport};

/// Parent-to-helper env: the directory the helper installs its hook into.
const CHILD_DIR_ENV: &str = "E2040_CRASH_CHILD_DIR";
/// Parent-to-helper env: `writable` or `unwritable` (report path blocked).
const CHILD_MODE_ENV: &str = "E2040_CRASH_CHILD_MODE";
/// Adapter string the helper installs.
const CHILD_ADAPTER: &str = "Test Adapter 1 (driver 0.1)";
/// Ticks the helper runs before it panics.
const CHILD_TICKS: u32 = 5;
/// Checkpoint hash base; tick `t` in the helper carries `BASE + t`.
const CHILD_HASH_BASE: u64 = 0xD57B_0000_0000_0000;
/// Canonical bytes of one fighter input (`FighterInput::append_bytes`).
const INPUT_RECORD_BYTES: usize = 10;

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("e2040-crash-hook-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Paths in `dir` whose file name starts with `prefix` and ends with `suffix`, sorted.
fn files_named(dir: &Path, prefix: &str, suffix: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("run dir readable")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix) && name.ends_with(suffix))
        })
        .collect();
    found.sort();
    found
}

/// The one crash report a run left in `dir`, parsed and validated.
fn only_report(dir: &Path) -> CrashReport {
    let mut reports = files_named(dir, "crash-", ".toml");
    assert_eq!(
        reports.len(),
        1,
        "want one crash report in {}: {reports:?}",
        dir.display()
    );
    let text = std::fs::read_to_string(reports.remove(0)).expect("crash report readable");
    let report = crash::parse_report(&text).expect("crash report parses");
    report.validate().expect("crash report validates");
    report
}

/// The one run log in `dir`, as text.
fn only_log(dir: &Path) -> String {
    let mut logs = files_named(dir, "arena-", ".log");
    assert_eq!(
        logs.len(),
        1,
        "want one run log in {}: {logs:?}",
        dir.display()
    );
    std::fs::read_to_string(logs.remove(0)).expect("run log readable")
}

fn stderr_text(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Helper for the parent tests: installs the hook, runs a few ticks of
/// records, then panics. Only the parent starts it (`--ignored --exact`), so a
/// plain test run shows it as ignored rather than as a pass.
#[test]
#[ignore = "helper: started by the crash_hook parent tests in a child process"]
fn crash_hook_child() {
    let dir = PathBuf::from(std::env::var_os(CHILD_DIR_ENV).expect("parent sets the child dir"));
    let mode = std::env::var(CHILD_MODE_ENV).expect("parent sets the child mode");
    let paths = crash::install_run_hook(&dir).expect("hook and log install");
    crash::set_adapter(CHILD_ADAPTER);
    crash::mirror_line(&format!("arena test: running {CHILD_TICKS} ticks"));
    if mode == "worker" {
        worker_mode();
        return;
    }
    for tick in 0..CHILD_TICKS {
        let input = [u8::try_from(tick).unwrap_or(0); INPUT_RECORD_BYTES];
        crash::record_fight_tick(
            tick + 1,
            &input,
            Some((tick, CHILD_HASH_BASE + u64::from(tick))),
        );
    }
    match mode.as_str() {
        "resume" => resume_mode(),
        "writable" => main_mode(&paths, false),
        "unwritable" => main_mode(&paths, true),
        other => panic!("unknown child mode: {other}"),
    }
}

/// The forced main-thread panic. The hook treats a thread named `main` as the
/// process main thread (test threads never are), so the panic runs there.
/// `unwritable` puts a directory where the report goes, so its write fails.
fn main_mode(paths: &crash::RunPaths, unwritable: bool) {
    let report = paths.report.clone();
    let main = std::thread::Builder::new()
        .name("main".to_string())
        .spawn(move || -> () {
            if unwritable {
                std::fs::create_dir(&report).expect("blocker directory");
            }
            panic!("forced panic at tick {CHILD_TICKS}");
        })
        .expect("named main thread");
    let _ = main.join();
    panic!("main-named thread returned: the hook did not exit");
}

/// Mirrors arena's `main`: a worker panic that its owner re-raises with
/// `resume_unwind` skips the hook, so the catch around the body must write the
/// report and exit with the crash code.
fn resume_mode() {
    let main = std::thread::Builder::new()
        .name("main".to_string())
        .spawn(|| -> () {
            let body = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let worker = std::thread::spawn(|| -> () { panic!("forced worker panic") });
                if let Err(payload) = worker.join() {
                    std::panic::resume_unwind(payload);
                }
            }));
            if let Err(payload) = body {
                crash::fatal_unwind(payload);
            }
        })
        .expect("named main thread");
    let _ = main.join();
    panic!("resume path returned: fatal_unwind did not exit");
}

/// A worker panic that its owner joins: the process must survive it.
fn worker_mode() {
    let worker = std::thread::spawn(|| -> () { panic!("forced worker panic") });
    assert!(worker.join().is_err(), "owner did not see the worker panic");
    crash::mirror_line("arena test: owner recovered from worker panic");
}

/// Run the helper in a child process; the hook exits it, so it cannot run in-process.
fn run_child(dir: &Path, mode: &str) -> Output {
    Command::new(std::env::current_exe().expect("test binary path"))
        .args(["--ignored", "--exact", "crash_hook_child", "--nocapture"])
        .env(CHILD_DIR_ENV, dir)
        .env(CHILD_MODE_ENV, mode)
        .output()
        .expect("child test binary runs")
}

#[test]
fn forced_panic_writes_a_valid_report_and_exits_with_the_crash_code() {
    let dir = scratch_dir("child");
    let out = run_child(&dir, "writable");
    assert_eq!(
        out.status.code(),
        Some(crash::exit_code(CrashKind::Panic)),
        "stderr:\n{}",
        stderr_text(&out)
    );
    let report = only_report(&dir);
    assert_eq!(report.tick, CHILD_TICKS);
    assert_eq!(report.mode, CrashMode::Fight);
    assert_eq!(report.adapter, CHILD_ADAPTER);
    assert_eq!(
        report.tape_tail.len(),
        CHILD_TICKS as usize * INPUT_RECORD_BYTES
    );
    let mut last_checkpoint = 4u32.to_le_bytes().to_vec();
    last_checkpoint.extend_from_slice(&(CHILD_HASH_BASE + 4).to_le_bytes());
    assert_eq!(report.checkpoint, last_checkpoint);
    let log = only_log(&dir);
    assert!(log.contains("arena test: running 5 ticks"), "log:\n{log}");
    assert!(log.contains("arena panic at "), "log:\n{log}");
    assert!(log.contains("forced panic at tick 5"), "log:\n{log}");
    assert!(
        stderr_text(&out).contains("arena crash report:"),
        "stderr:\n{}",
        stderr_text(&out)
    );
}

#[test]
fn resumed_worker_panic_writes_a_report_and_exits_with_the_crash_code() {
    let dir = scratch_dir("resume");
    let out = run_child(&dir, "resume");
    assert_eq!(
        out.status.code(),
        Some(crash::exit_code(CrashKind::Panic)),
        "stderr:\n{}",
        stderr_text(&out)
    );
    let report = only_report(&dir);
    assert_eq!(report.tick, CHILD_TICKS);
    assert_eq!(report.mode, CrashMode::Fight);
    let log = only_log(&dir);
    assert!(log.contains("arena worker panic on "), "log:\n{log}");
    assert!(
        log.contains("arena panic unwound to main: forced worker panic"),
        "log:\n{log}"
    );
}

#[test]
fn worker_panic_leaves_the_process_alive_and_logs_once() {
    let dir = scratch_dir("worker");
    let out = run_child(&dir, "worker");
    assert_eq!(out.status.code(), Some(0), "stderr:\n{}", stderr_text(&out));
    let log = only_log(&dir);
    let worker_lines: Vec<&str> = log
        .lines()
        .filter(|line| line.starts_with("arena worker panic on "))
        .collect();
    assert_eq!(worker_lines.len(), 1, "log:\n{log}");
    assert!(
        worker_lines[0].contains("crash_hook.rs:")
            && worker_lines[0].ends_with(": forced worker panic"),
        "worker line: {}",
        worker_lines[0]
    );
    assert!(
        log.contains("arena test: owner recovered from worker panic"),
        "log:\n{log}"
    );
    assert!(
        files_named(&dir, "crash-", ".toml").is_empty(),
        "a worker panic wrote a crash report"
    );
}

#[test]
fn unwritable_report_exits_with_the_report_failed_code() {
    let dir = scratch_dir("blocked");
    let out = run_child(&dir, "unwritable");
    assert_eq!(
        out.status.code(),
        Some(crash::exit_code(CrashKind::ReportFailed)),
        "stderr:\n{}",
        stderr_text(&out)
    );
    assert!(
        stderr_text(&out).contains("crash report not written"),
        "stderr:\n{}",
        stderr_text(&out)
    );
    let log = only_log(&dir);
    assert!(log.contains("forced panic at tick 5"), "log:\n{log}");
    assert!(log.contains("crash report not written"), "log:\n{log}");
}

/// The arena window binary with the forced panic at tick 5. Needs the window
/// and a GPU, so it builds only under `--features app`.
#[cfg(feature = "app")]
#[test]
fn arena_forced_panic_at_tick_five_leaves_a_report_and_the_crash_code() {
    /// Env var the arena binary reads: panic before this fixed tick runs.
    const ARENA_PANIC_ENV: &str = "E2040_TEST_PANIC_AT_TICK";
    let dir = scratch_dir("arena");
    let out = Command::new(env!("CARGO_BIN_EXE_arena"))
        .args(["--frames", "600", "--exit"])
        .env(ARENA_PANIC_ENV, "5")
        .env(crash::LOG_DIR_ENV, &dir)
        .output()
        .expect("arena runs");
    assert_eq!(
        out.status.code(),
        Some(crash::exit_code(CrashKind::Panic)),
        "stderr:\n{}",
        stderr_text(&out)
    );
    let report = only_report(&dir);
    assert_eq!(report.tick, 5);
    assert_eq!(report.mode, CrashMode::Fight);
    assert_eq!(report.tape_tail.len(), 5 * INPUT_RECORD_BYTES);
    assert!(
        report.adapter.contains('('),
        "adapter not named: {}",
        report.adapter
    );
    let log = only_log(&dir);
    assert!(
        log.contains("E2040_TEST_PANIC_AT_TICK: forced panic at tick 5"),
        "log:\n{log}"
    );
    // Status lines reach the run log unchanged: the backend pick on stdout
    // (the first `arena ` line) and every `arena` line on stderr (notices,
    // the log path, the panic and report notes).
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let pick = stdout
        .lines()
        .find(|line| line.starts_with("arena "))
        .expect("backend pick on stdout");
    assert!(
        log.lines().any(|logged| logged == pick),
        "backend pick missing from run log: {pick}"
    );
    let stderr = stderr_text(&out);
    let notes: Vec<&str> = stderr
        .lines()
        .filter(|line| line.starts_with("arena"))
        .collect();
    assert!(!notes.is_empty(), "no arena lines on stderr:\n{stderr}");
    for note in notes {
        assert!(
            log.lines().any(|logged| logged == note),
            "stderr line missing from run log: {note}"
        );
    }
}
