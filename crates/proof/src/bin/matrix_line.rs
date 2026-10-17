//! Print this machine's HS2 matrix rows: the runner label plus the frozen
//! golden and the BX28 corpus digests on the first line, the corpus block
//! digests on the second, for CI.
//!
//! Both lines come from [`engine_proof::matrix::collect_local_entry`], rendered
//! through [`engine_proof::matrix::matrix_digest_line`] and
//! [`engine_proof::matrix::matrix_blocks_line`]. The golden-matrix workflow
//! stores them as one row file per runner (ubuntu, windows, arm) and
//! `tools/golden-matrix.mjs --rows` compares them. Nothing here touches the
//! simulation hash.

/// The two lines one runner stores: the digest row, then the corpus block row.
fn matrix_lines() -> Result<(String, String), String> {
    let entry = engine_proof::matrix::collect_local_entry().map_err(|e| e.to_string())?;
    Ok((
        engine_proof::matrix::matrix_digest_line(&entry),
        engine_proof::matrix::matrix_blocks_line(&entry),
    ))
}

fn main() {
    match matrix_lines() {
        Ok((row, blocks)) => {
            println!("{row}");
            println!("{blocks}");
        }
        Err(reason) => {
            eprintln!("matrix_line: {reason}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::matrix_lines;

    #[test]
    fn matrix_line_prints_one_parseable_row_twice() {
        let first = matrix_lines().expect("matrix row collects on this host");
        let second = matrix_lines().expect("matrix row collects twice on this host");
        assert_eq!(first, second, "two rows on one machine must match");
        let parsed = engine_proof::matrix::parse_matrix_digest_line(&first.0)
            .expect("printed row must parse back to its fields");
        assert!(
            !parsed.host.is_empty(),
            "row {} must name its host",
            first.0
        );
    }

    #[test]
    fn matrix_line_row_names_this_target() {
        let (line, _) = matrix_lines().expect("matrix row collects on this host");
        let parsed = engine_proof::matrix::parse_matrix_digest_line(&line)
            .expect("printed row must parse back to its fields");
        assert_eq!(
            parsed.host,
            engine_proof::matrix::host_label(),
            "row {line} must name this target"
        );
    }

    #[test]
    fn matrix_line_block_row_agrees_with_the_digest_row() {
        let (row, blocks) = matrix_lines().expect("matrix row collects on this host");
        let parsed = engine_proof::matrix::parse_matrix_digest_line(&row)
            .expect("printed row must parse back to its fields");
        let every = engine_proof::break_corpus::CHECKPOINT_EVERY;
        let head = format!("{} every={every} cut_checkpoints=", parsed.host);
        let list = blocks
            .strip_prefix(&head)
            .unwrap_or_else(|| panic!("block row {blocks} must start with {head}"));
        let digests: Vec<&str> = list.split(',').collect();
        assert_eq!(
            digests.len(),
            parsed.blocks,
            "one digest per block the row counts"
        );
        for digest in &digests {
            assert!(
                digest.len() == 16
                    && digest
                        .bytes()
                        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
                "block digest {digest} must be 16 lower-hex digits"
            );
        }
    }
}
