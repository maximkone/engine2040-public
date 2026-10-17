//! Print this machine's HS2 matrix digest row: the runner label plus the
//! frozen golden and the BX28 corpus digests, one line per machine for CI.
//!
//! The line is [`engine_proof::matrix::collect_local_entry`] rendered through
//! [`engine_proof::matrix::matrix_digest_line`]; a later CI step collects one
//! row per runner (ubuntu, windows, arm) and compares them. Nothing here
//! touches the simulation hash.

fn matrix_line() -> Result<String, String> {
    let entry = engine_proof::matrix::collect_local_entry().map_err(|e| e.to_string())?;
    Ok(engine_proof::matrix::matrix_digest_line(&entry))
}

fn main() {
    match matrix_line() {
        Ok(line) => println!("{line}"),
        Err(reason) => {
            eprintln!("matrix_line: {reason}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::matrix_line;

    #[test]
    fn matrix_line_prints_one_parseable_row_twice() {
        let first = matrix_line().expect("matrix row collects on this host");
        let second = matrix_line().expect("matrix row collects twice on this host");
        assert_eq!(first, second, "two rows on one machine must match");
        let parsed = engine_proof::matrix::parse_matrix_digest_line(&first)
            .expect("printed row must parse back to its fields");
        assert!(!parsed.host.is_empty(), "row {first} must name its host");
    }

    #[test]
    fn matrix_line_row_names_this_target() {
        let line = matrix_line().expect("matrix row collects on this host");
        let parsed = engine_proof::matrix::parse_matrix_digest_line(&line)
            .expect("printed row must parse back to its fields");
        assert_eq!(
            parsed.host,
            engine_proof::matrix::host_label(),
            "row {line} must name this target"
        );
    }
}
