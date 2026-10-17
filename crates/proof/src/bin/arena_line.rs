//! Print this machine's arena tape rows for the golden matrix (DSN2): the
//! runner label and the match digest on the first line, then one line per
//! tick with that tick's checkpoint hash.
//!
//! The text comes from [`engine_proof::arena_matrix::ArenaTape::rows`]. The
//! golden-matrix workflow stores it per runner (ubuntu, windows, arm, wasm)
//! and `tools/golden-matrix.mjs --arena` compares the files, naming the first
//! tick that differs. Nothing here touches the simulation hash.

use engine_proof::arena_matrix::{run, Fixture};

/// The rows one runner stores.
fn arena_rows() -> Result<String, String> {
    let tape = run(&Fixture::STANDARD).map_err(|e| e.to_string())?;
    Ok(tape.rows(&engine_proof::matrix::host_label()))
}

fn main() {
    match arena_rows() {
        Ok(text) => print!("{text}"),
        Err(reason) => {
            eprintln!("arena_line: {reason}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::arena_rows;

    #[test]
    fn arena_line_names_this_target_on_every_line() {
        let text = arena_rows().expect("the match runs on this host");
        let host = engine_proof::matrix::host_label();
        let mut lines = 0;
        for line in text.lines() {
            assert!(
                line.starts_with(&format!("{host} ")),
                "line {line} must start with the host {host}"
            );
            lines += 1;
        }
        assert_eq!(
            lines,
            1 + engine_proof::arena_matrix::TICKS as usize,
            "a summary line plus one line per tick"
        );
    }

    #[test]
    fn arena_line_prints_the_same_rows_twice() {
        assert_eq!(
            arena_rows().expect("first match"),
            arena_rows().expect("second match"),
            "two matches on one machine must match"
        );
    }
}
