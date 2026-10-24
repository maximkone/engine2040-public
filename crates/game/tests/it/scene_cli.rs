//! The scene report through the real binary (rows SP62, EG35): `arena.exe
//! --scene-report <seed> <ticks> <fighters>` plays a bout with no window and no
//! GPU, prints one `arena scene-report: ` JSON line and the text lines, and exits.
//! This is the route the door's `view.scene` takes with no seat, so the line
//! prefix, the exit codes and the bytes are pinned here against the real
//! executable, not against the door's fake host.
//!
//! Expected outcomes, written down before the runs:
//! - `--scene-report 2040 120 2` exits 0 and prints the prefixed line once, a
//!   one-line JSON object that names `fighter_0` and `fighter_1`, tick 120,
//!   schema 1 and the `arena view:` laws line; the text lines follow as
//!   `arena scene-text| ` rows.
//! - the same request twice prints the same bytes.
//! - `--showcase` plays the tour (source `headless_showcase`).
//! - fewer than two fighters, a bad number or a missing value exit 2 and print
//!   no report line.

#![cfg(feature = "app")]

use std::process::{Command, Output};

const PREFIX: &str = "arena scene-report: ";

fn arena(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_arena"))
        .args(args)
        .output()
        .expect("arena.exe starts")
}

fn report_line(out: &Output) -> Option<String> {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| {
            line.strip_prefix(PREFIX)
                .map(|json| json.trim_end().to_owned())
        })
}

#[test]
fn a_windowless_scene_report_names_both_fighters_and_repeats_its_bytes() {
    let first = arena(&["--scene-report", "2040", "120", "2"]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let json = report_line(&first).expect("one prefixed report line");
    assert!(
        json.starts_with("{\"schema\":1,\"source\":\"headless\",\"tick\":120"),
        "{json}"
    );
    for id in ["fighter_0", "fighter_1"] {
        assert!(json.contains(&format!("\"id\":\"{id}\"")), "{id}");
    }
    assert!(
        json.contains("\"line\":\"arena view: "),
        "the laws line rides along"
    );
    assert!(!json.contains('\n') && !json.contains("NaN"));
    let stdout = String::from_utf8_lossy(&first.stdout);
    assert_eq!(stdout.matches(PREFIX).count(), 1, "exactly one report line");
    assert!(
        stdout.contains("arena scene-text| scene tick=120 seed=2040"),
        "{stdout}"
    );
    let second = arena(&["--scene-report", "2040", "120", "2"]);
    assert_eq!(
        report_line(&second).as_deref(),
        Some(json.as_str()),
        "same request, same bytes"
    );
}

#[test]
fn scene_showcase_and_named_shot_shape_the_report() {
    let tour = arena(&["--scene-report", "2040", "600", "2", "--showcase"]);
    assert_eq!(
        tour.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&tour.stderr)
    );
    let json = report_line(&tour).expect("report line");
    // The sample a reader wants to see (`--nocapture` shows it): the text form of the tour's last frame.
    let stdout = String::from_utf8_lossy(&tour.stdout);
    println!("{stdout}");
    println!("SAMPLE (first lines of the text form):");
    for line in stdout
        .lines()
        .filter(|l| l.starts_with("arena scene-text| "))
        .take(20)
    {
        println!("{line}");
    }
    assert!(json.contains("\"source\":\"headless_showcase\""), "{json}");
    let hero = arena(&["--scene-report", "2040", "120", "2", "--shot", "hero"]);
    let wide = arena(&["--scene-report", "2040", "120", "2", "--shot", "wide"]);
    let (hero, wide) = (
        report_line(&hero).expect("hero"),
        report_line(&wide).expect("wide"),
    );
    assert!(hero.contains("\"shot\":\"hero\""), "{hero}");
    assert!(wide.contains("\"shot\":\"wide\""), "{wide}");
    assert_ne!(hero, wide, "a different shot is a different camera");
}

#[test]
fn scene_bad_request_exits_two_and_prints_no_report() {
    for bad in [
        vec!["--scene-report", "2040", "120", "1"],
        vec!["--scene-report", "2040", "x", "2"],
        vec!["--scene-report", "2040"],
        vec!["--scene-report", "2040", "120", "2", "--shot", "top"],
    ] {
        let out = arena(&bad);
        assert_eq!(out.status.code(), Some(2), "{bad:?}");
        assert!(report_line(&out).is_none(), "{bad:?}");
        assert!(!out.stderr.is_empty(), "{bad:?} says why on stderr");
    }
}
