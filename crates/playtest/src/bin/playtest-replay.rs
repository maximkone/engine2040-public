//! Replay a named playtest scenario and print one receipt line.
//!
//! `playtest-replay --list` names the corpus, `--all` replays every scenario
//! and a single name replays just that one. Exit codes: `0` every replayed
//! scenario matched its recorded digest, `1` a recorded mismatch, `2` a usage
//! mistake or a typed corpus error.

use engine_playtest::replay_corpus::{names, replay};
use std::process::ExitCode;

/// Usage line printed on a bad call.
const USAGE: &str = "usage: playtest-replay <name> | --all | --list";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--list") => {
            for name in names() {
                println!("{name}");
            }
            ExitCode::SUCCESS
        }
        Some("--all") => run_all(),
        Some(name) if !name.starts_with('-') => run_one(name),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Replay the whole corpus; one failing scenario is enough to fail the call.
fn run_all() -> ExitCode {
    let mut code = 0u8;
    for name in names() {
        match replay(name) {
            Ok(report) => {
                println!("{}", report.receipt());
                if !report.pass {
                    code = 1;
                }
            }
            Err(error) => {
                eprintln!("ERROR {name}: {error}");
                code = 2;
            }
        }
    }
    ExitCode::from(code)
}

/// Replay one scenario by name.
fn run_one(name: &str) -> ExitCode {
    match replay(name) {
        Ok(report) => {
            println!("{}", report.receipt());
            if report.pass {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(error) => {
            eprintln!("ERROR {error}");
            ExitCode::from(2)
        }
    }
}
