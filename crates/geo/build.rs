// Compiles the vendored xatlas and its C shim for the `uv` feature. Without the feature this
// script does nothing, so the default build needs no C++ compiler.

use std::io::Write;

/// One cargo directive on the build-script protocol channel (stdout).
fn directive(line: &str) {
    // A failed write only loses a rerun hint; the build itself is unaffected.
    let _ = writeln!(std::io::stdout(), "cargo:{line}");
}

fn main() {
    directive("rerun-if-changed=build.rs");
    #[cfg(feature = "uv")]
    {
        directive("rerun-if-changed=vendor");
        cc::Build::new()
            .cpp(true)
            .file("vendor/xatlas/xatlas.cpp")
            .file("vendor/xatlas_shim.cpp")
            .include("vendor")
            // No worker threads: the unwrap must not depend on the machine.
            .define("XA_MULTITHREADED", "0")
            .warnings(false)
            .compile("engine_geo_xatlas");
    }
}
