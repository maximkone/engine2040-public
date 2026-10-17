//! Print this machine's ragdoll tape rows for the golden matrix (WEB3): the
//! runner label and the fall digest on the first line, then one line of body
//! digests per tick.
//!
//! The text comes from [`engine_proof::ragdoll_matrix::RagdollTape::rows`]. The
//! golden-matrix workflow stores it per runner (ubuntu, windows, arm, wasm) and
//! `tools/golden-matrix.mjs --ragdoll` compares the files, naming the first
//! tick and body that differ. Nothing here touches the simulation hash.

use engine_proof::ragdoll_matrix::{run, Fixture};

/// The rows one runner stores.
fn ragdoll_rows() -> Result<String, String> {
    let tape = run(&Fixture::STANDARD).map_err(|e| e.to_string())?;
    Ok(tape.rows(&engine_proof::matrix::host_label()))
}

fn main() {
    match ragdoll_rows() {
        Ok(text) => print!("{text}"),
        Err(reason) => {
            eprintln!("ragdoll_line: {reason}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ragdoll_rows;

    #[test]
    fn ragdoll_line_names_this_target_on_every_line() {
        let text = ragdoll_rows().expect("the fall runs on this host");
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
            1 + engine_proof::ragdoll_matrix::TICKS,
            "a summary line plus one line per tick"
        );
    }

    #[test]
    fn ragdoll_line_prints_the_same_rows_twice() {
        assert_eq!(
            ragdoll_rows().expect("first fall"),
            ragdoll_rows().expect("second fall"),
            "two falls on one machine must match"
        );
    }
}
