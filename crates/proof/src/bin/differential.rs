//! differential: the ship differential tool.
//!
//! Two jobs: run a curated scenario list against a build and write the
//! receipt log, or compare two receipt logs and name the divergences.
//!
//!   differential --scenarios <list> --tier <tier> --build <dir> --out <log>
//!   differential --reference <log> --candidate <log>
//!
//! The exit code is 0 when every in-scope step matched, 1 when one
//! divergence or more was found and 2 when the tool could not run.

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let mut err = std::io::BufWriter::new(stderr.lock());
    let code = engine_proof::differential::cli(&args, &mut out, &mut err);
    let _ = out.flush();
    let _ = err.flush();
    std::process::exit(code);
}
