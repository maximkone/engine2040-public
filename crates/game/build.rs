// GS7: a hybrid laptop picks the discrete GPU for an exe that exports `NvOptimusEnablement` and
// `AmdPowerXpressRequestHighPerformance` (the statics live in `src/bin/arena.rs`). MSVC drops
// exports nobody asked for, so the linker is told to keep both, on the `arena` bin only.

use std::io::Write;

/// One cargo directive on the build-script protocol channel (stdout).
fn directive(line: &str) {
    // A failed write only loses a directive the next build repeats.
    let _ = writeln!(std::io::stdout(), "cargo:{line}");
}

fn main() {
    directive("rerun-if-changed=build.rs");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os == "windows" && target_env == "msvc" {
        for name in [
            "NvOptimusEnablement",
            "AmdPowerXpressRequestHighPerformance",
        ] {
            directive(&format!("rustc-link-arg-bin=arena=/EXPORT:{name},DATA"));
        }
    }
}
