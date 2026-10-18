//! `input_tape <script>`: replay an input script on the PC and print its tape (WEB7).
//!
//! The browser check replays the same file as real key and mouse events and compares the
//! `input-tape: ticks=<n> bytes=<hex>` line it gets from the page with this one.
//! Exit: 0 printed, 1 the script or the record was refused, 2 no script given or unreadable.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    use engine_web::input_tape::{parse_script, replay};
    use std::process::ExitCode;

    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: input_tape <script>");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("input_tape: cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    let tape = parse_script(&text)
        .map_err(|e| e.to_string())
        .and_then(|steps| replay(&steps).map_err(|e| format!("{e:?}")));
    match tape {
        Ok(tape) => {
            println!("{}", tape.line());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("input_tape: {path}: {e}");
            ExitCode::from(1)
        }
    }
}

/// The browser build has nothing to run: the tape is sampled by the page itself.
#[cfg(target_arch = "wasm32")]
fn main() {}
