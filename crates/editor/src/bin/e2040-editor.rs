//! `e2040-editor`: dock layout, viewport texture, gizmo batches,
//! save/reopen and child-process play.
//!
//! `--help` prints flags. Every failure is a typed [`EditorAppError`](engine_editor::app::EditorAppError)
//! with a recovery hint; the process exits 1 naming both.
//!
//! Recorded runs (`--record-a2 OUT`, `--record-a3 OUT`) execute the scripted
//! evidence sessions inside this executable: same shell, browser, gizmo and
//! save paths as the window, with captures from the executable's own window
//! paths (GPU offscreen first, then the live viewport panel). They open no
//! interactive window, so headless machines still produce honest captures
//! named by the method that made them.

use std::cell::RefCell;
use std::path::PathBuf;

use engine_editor::a2_session::{self, encode_png_rgba8};
use engine_editor::a3_session;
use engine_editor::app::{parse_args, run, ViewportSnapshot, HELP};
use engine_editor::shell::Shell;
use engine_editor::viewport_image::ViewportImage;

/// Value after `--record-a2` / `--record-a3`, if the flag is present.
fn record_dir(argv: &[String], flag: &str) -> Option<PathBuf> {
    let mut iter = argv.iter();
    for arg in iter.by_ref() {
        if arg == flag {
            return iter.next().map(PathBuf::from);
        }
    }
    None
}

/// Capture supplier for the recorded runs: the executable's window paths.
///
/// Tries the GPU offscreen presenter first (a real device render with its
/// adapter named), then the live viewport panel raster (the exact pixels the
/// window shows). Returns `None` only when both fail, in which case the
/// session keeps its headless fallback and says so in the report.
fn exe_capture(snapshot: &RefCell<ViewportSnapshot>, shell: &Shell) -> Option<(Vec<u8>, String)> {
    const WIDTH: u32 = 640;
    const HEIGHT: u32 = 400;
    match snapshot.borrow_mut().capture(shell, WIDTH, HEIGHT) {
        Ok(out) => Some((
            out.png,
            format!(
                "exe-gpu-offscreen {WIDTH}x{HEIGHT} {} draws adapter {} driver {} validation_errors {} | {}",
                out.draws,
                out.adapter,
                out.driver,
                out.validation_errors,
                out.backend.line()
            ),
        )),
        Err(gpu_error) => match ViewportImage::render(shell, WIDTH, HEIGHT) {
            Ok(image) => match encode_png_rgba8(&image.rgba, image.width, image.height) {
                Ok(png) => Some((
                    png,
                    format!(
                        "exe-live-panel {WIDTH}x{HEIGHT} {} draws rev {} (window viewport raster; \
                         GPU offscreen unavailable: {gpu_error})",
                        image.draws, image.revision
                    ),
                )),
                Err(_) => None,
            },
            Err(_) => None,
        },
    }
}

/// Drive the recorded editable-scene session in this process and note how.
fn record_a2(out: PathBuf) -> i32 {
    let content = a2_session::lane_content_root();
    let snapshot = RefCell::new(ViewportSnapshot::new());
    let capture = |shell: &Shell| exe_capture(&snapshot, shell);
    match a2_session::run_session(&content, &out, Some(&capture)) {
        Ok(report) => {
            println!(
                "record-a2 ok: {} nodes r{} bytes_match={} method={}",
                report.nodes, report.revision, report.bytes_match, report.capture_method
            );
            0
        }
        Err(error) => {
            eprintln!("record-a2 failed: {error}");
            1
        }
    }
}

/// Drive the recorded editor run in this process, plus the hosted loop.
fn record_a3(out: PathBuf) -> i32 {
    let content = a3_session::lane_content_root();
    let snapshot = RefCell::new(ViewportSnapshot::new());
    let capture = |shell: &Shell| exe_capture(&snapshot, shell);
    let report = match a3_session::run_session(&content, &out, Some(&capture)) {
        Ok(report) => report,
        Err(error) => {
            eprintln!("record-a3 failed: {error}");
            return 1;
        }
    };
    // In-viewport loop smoke on the recorded document (the hosted equivalent
    // of the window Play path): three neutral frames, then a checkpoint.
    let scene_path = out.join("scene.edoc");
    let play_in_line = match std::fs::read(&scene_path) {
        Ok(bytes) => match engine_editor::save::load_from_bytes(&bytes) {
            Ok(doc) => match a3_session::play_in_smoke(&doc) {
                Ok(line) => format!("in-viewport loop (exe): {line}"),
                Err(error) => {
                    eprintln!("record-a3 play-in failed: {error}");
                    return 1;
                }
            },
            Err(error) => {
                eprintln!("record-a3 reload failed: {error}");
                return 1;
            }
        },
        Err(error) => {
            eprintln!("record-a3 read failed: {error}");
            return 1;
        }
    };
    // The library run leaves a placeholder line; the executable replaces it
    // with its measured loop receipt.
    let report_path = out.join("report.txt");
    let patched = match std::fs::read_to_string(&report_path) {
        Ok(text) => text.replace(
            "in-viewport loop: (library run; the editor exe appends its loop)",
            &play_in_line,
        ),
        Err(error) => {
            eprintln!("record-a3 report patch failed: {error}");
            return 1;
        }
    };
    if !patched.contains(&play_in_line) {
        eprintln!("record-a3 report patch missed its placeholder");
        return 1;
    }
    if std::fs::write(&report_path, patched.as_bytes()).is_err() {
        eprintln!("record-a3 report write failed");
        return 1;
    }
    println!(
        "record-a3 ok: {} nodes r{} bytes_match={} method={} | {play_in_line}",
        report.nodes, report.revision, report.bytes_match, report.capture_method
    );
    0
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    if argv.iter().any(|flag| flag == "--help") {
        println!("{HELP}");
        println!("record: e2040-editor --record-a2 OUT | --record-a3 OUT");
        return;
    }
    if let Some(out) = record_dir(&argv, "--record-a2") {
        std::process::exit(record_a2(out));
    }
    if let Some(out) = record_dir(&argv, "--record-a3") {
        std::process::exit(record_a3(out));
    }
    let config = match parse_args(&argv) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("e2040-editor: {error} (recovery: {:?})", error.recovery());
            std::process::exit(2);
        }
    };
    if let Err(error) = run(config) {
        eprintln!("e2040-editor: {error} (recovery: {:?})", error.recovery());
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::record_dir;
    use std::path::PathBuf;

    fn args(flags: &[&str]) -> Vec<String> {
        flags.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn record_a2_flag_takes_the_next_arg_as_out_dir() {
        let argv = args(&["e2040-editor", "--record-a2", "out/a2"]);
        assert_eq!(
            record_dir(&argv, "--record-a2"),
            Some(PathBuf::from("out/a2"))
        );
    }

    #[test]
    fn record_a3_flag_takes_the_next_arg_as_out_dir() {
        let argv = args(&["e2040-editor", "--record-a3", "out/a3"]);
        assert_eq!(
            record_dir(&argv, "--record-a3"),
            Some(PathBuf::from("out/a3"))
        );
    }

    #[test]
    fn missing_flag_returns_none() {
        let argv = args(&["e2040-editor", "--viewport", "1280x720"]);
        assert_eq!(record_dir(&argv, "--record-a2"), None);
        assert_eq!(record_dir(&argv, "--record-a3"), None);
    }

    #[test]
    fn trailing_flag_without_a_value_returns_none() {
        let argv = args(&["e2040-editor", "--record-a2"]);
        assert_eq!(record_dir(&argv, "--record-a2"), None);
    }

    #[test]
    fn lookalike_flags_do_not_match() {
        let argv = args(&["e2040-editor", "--record-a2x", "out/a2"]);
        assert_eq!(record_dir(&argv, "--record-a2"), None);
        let argv = args(&["e2040-editor", "--record-a", "out/a2"]);
        assert_eq!(record_dir(&argv, "--record-a2"), None);
    }

    #[test]
    fn record_flags_are_independent() {
        let argv = args(&["e2040-editor", "--record-a3", "out/a3"]);
        assert_eq!(record_dir(&argv, "--record-a2"), None);
        assert_eq!(
            record_dir(&argv, "--record-a3"),
            Some(PathBuf::from("out/a3"))
        );
    }
}
