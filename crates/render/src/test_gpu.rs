//! Test GPU gate: a GPU test that finds no adapter says so, and
//! `E2040_REQUIRE_GPU=1` turns that skip into a failure.
//!
//! Owner: render crate. Consumers: GPU-backed tests (`present/tests_gpu.rs`
//! first) and `tools/proof.mjs`, which counts the `SKIPPED` lines of the
//! render leg into its RESULT line. Units: none. Determinism: none; a skip
//! writes one line to stderr, which libtest output capture never swallows.
//! Errors: only a missing adapter is a skip; any other probe failure stays
//! the caller's panic.

use std::io::Write;

/// Environment switch: `1` makes a missing adapter a failure.
pub const REQUIRE_GPU_ENV: &str = "E2040_REQUIRE_GPU";

/// Line prefix for a skipped GPU test; `tools/proof.mjs` counts these lines.
pub const SKIP_PREFIX: &str = "SKIPPED";

/// A GPU probe that found no adapter, with the reason it gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoAdapter(pub String);

/// True when `E2040_REQUIRE_GPU` is `1`.
pub fn required_by_env() -> bool {
    std::env::var(REQUIRE_GPU_ENV).is_ok_and(|value| value == "1")
}

/// Use a GPU probe in a test. A device gives `Some` and the test runs. A
/// missing adapter writes `SKIPPED <test>: <why>` to stderr and gives `None`,
/// unless `E2040_REQUIRE_GPU=1`, which panics so the test fails.
pub fn require_or_skip<T>(test: &str, probe: Result<T, NoAdapter>) -> Option<T> {
    require_or_skip_with(test, probe, required_by_env())
}

/// [`require_or_skip`] with the requirement passed in, so the rule is tested
/// without touching the process environment.
pub fn require_or_skip_with<T>(
    test: &str,
    probe: Result<T, NoAdapter>,
    required: bool,
) -> Option<T> {
    match probe {
        Ok(device) => Some(device),
        Err(NoAdapter(why)) => {
            assert!(
                !required,
                "{test}: no GPU adapter and {REQUIRE_GPU_ENV}=1 requires one: {why}"
            );
            note_skip(test, &why);
            None
        }
    }
}

/// Print one `SKIPPED <test>: <why>` line for a GPU test that does not run.
/// A direct stderr write: libtest captures the print macros for passing
/// tests, and the proof gate must still see the skip.
pub fn note_skip(test: &str, why: &str) {
    let _ = writeln!(std::io::stderr(), "{SKIP_PREFIX} {test}: {why}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_or_skip_passes_a_device_through() {
        assert_eq!(
            require_or_skip_with("device_test", Ok::<u8, NoAdapter>(7), true),
            Some(7)
        );
    }

    #[test]
    fn require_or_skip_skips_without_an_adapter_when_not_required() {
        let got = require_or_skip_with(
            "skip_test",
            Err::<u8, NoAdapter>(NoAdapter("no wgpu adapter".into())),
            false,
        );
        assert_eq!(got, None);
    }

    #[test]
    #[should_panic(expected = "requires one")]
    fn require_or_skip_fails_without_an_adapter_when_required() {
        let _ = require_or_skip_with(
            "required_test",
            Err::<u8, NoAdapter>(NoAdapter("no wgpu adapter".into())),
            true,
        );
    }

    #[cfg(feature = "window")]
    #[test]
    fn require_or_skip_probes_the_real_adapter() {
        // The real probe. A device runs the body; with no adapter this skips
        // (or, under E2040_REQUIRE_GPU=1, fails right here).
        let probe = match crate::present::Presenter::init_offscreen() {
            Ok(_) => Ok(()),
            Err(crate::present::PresentError::NoAdapter) => {
                Err(NoAdapter("no wgpu adapter on this machine".into()))
            }
            Err(e) => panic!("presenter init failed (must fail loudly): {e}"),
        };
        let _ = require_or_skip("require_or_skip_probes_the_real_adapter", probe);
    }
}
