//! Test GPU gate: a GPU test that finds no adapter says so, and
//! `E2040_REQUIRE_GPU=1` turns that skip into a failure — but only for tests
//! that call `require_or_skip`. The render crate routes its GPU probes here
//! (`graph::gx1_probe` for the shared GX1 device, `init_or_skip` in
//! `present/tests_gpu.rs`); a site that still prints its own `SKIPPED` line
//! bypasses both the gate and `skip_count`, and `tools/proof.mjs` counts only
//! the routed lines.
//!
//! Owner: render crate. Consumers: GPU-backed tests that call
//! `require_or_skip` (`present/tests_gpu.rs`, `graph.rs`, `ibl.rs` first) and
//! `tools/proof.mjs`, which counts the `SKIPPED` lines of the render leg into
//! its RESULT line.
//! Units: none. Determinism: none; a skip writes one line to stderr, which
//! libtest output capture never swallows, and bumps the skip counter below.
//! Errors: only a missing adapter is a skip; any other probe failure stays
//! the caller's panic.

use std::cell::RefCell;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};

/// Most tests that hold a GPU device at once. On the dev PC (Intel plus
/// NVIDIA, DX12 and Vulkan in one process) twelve test threads opening and
/// closing devices together stall inside the vendor drivers: every thread
/// waits on a driver lock and the run never ends, CPU-only tests included.
/// Two at a time finished every full run; twelve stalled most of them.
pub const GPU_TEST_SLOTS: usize = 2;

/// A counting gate: `take` waits for a free slot, `give` frees one.
struct Gate {
    free: Mutex<usize>,
    freed: Condvar,
}

impl Gate {
    const fn new(slots: usize) -> Self {
        Self {
            free: Mutex::new(slots),
            freed: Condvar::new(),
        }
    }

    fn take(&self) {
        let mut free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
        while *free == 0 {
            free = self
                .freed
                .wait(free)
                .unwrap_or_else(PoisonError::into_inner);
        }
        *free -= 1;
    }

    fn give(&self) {
        let mut free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
        *free += 1;
        self.freed.notify_one();
    }
}

/// The process-wide device gate behind [`hold_gpu_slot`].
static GATE: Gate = Gate::new(GPU_TEST_SLOTS);

/// One held device slot; dropping it frees the slot for the next test.
struct HeldSlot;

impl Drop for HeldSlot {
    fn drop(&mut self) {
        GATE.give();
    }
}

thread_local! {
    /// The slot this test thread holds. libtest gives each test its own
    /// thread, so the slot frees when the test thread exits, after the
    /// test's device has dropped; a panicking test frees it too.
    static HELD: RefCell<Option<HeldSlot>> = const { RefCell::new(None) };
}

/// Take a device slot for the rest of this test, waiting while
/// [`GPU_TEST_SLOTS`] other tests hold one. Call it before the first
/// `wgpu::Instance::new` of a test. Idempotent per thread, so a test that
/// opens two devices still holds one slot. Units: none. Determinism: none,
/// it only orders tests in time.
pub fn hold_gpu_slot() {
    HELD.with(|held| {
        if held.borrow().is_some() {
            return;
        }
        GATE.take();
        *held.borrow_mut() = Some(HeldSlot);
    });
}

/// Environment switch: `1` makes a missing adapter a failure, but only
/// where the test routes through `require_or_skip`. The presenter bring-up
/// reads it too (`adapter::SelectionPolicy::from_env`): under `1` a fallback
/// adapter (software, or one the surface forces) fails the bring-up.
pub const REQUIRE_GPU_ENV: &str = "E2040_REQUIRE_GPU";

/// Line prefix for a skipped GPU test; `tools/proof.mjs` counts these lines.
pub const SKIP_PREFIX: &str = "SKIPPED";

/// How many skips went through `require_or_skip`/`note_skip` this process.
/// Ad-hoc `eprintln!("SKIPPED ...")` sites bypass both the gate and this
/// counter; the counter makes the routed skips visible, not the total.
static SKIP_COUNT: AtomicU64 = AtomicU64::new(0);

/// Routed skips so far in this process (see `SKIP_COUNT`).
pub fn skip_count() -> u64 {
    SKIP_COUNT.load(Ordering::Relaxed)
}

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
    SKIP_COUNT.fetch_add(1, Ordering::Relaxed);
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
    fn the_gate_makes_a_full_house_wait_until_a_slot_frees() {
        use std::sync::mpsc;
        use std::time::Duration;
        let gate = Gate::new(1);
        gate.take();
        let (tx, rx) = mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                gate.take();
                tx.send(()).expect("receiver is alive");
                gate.give();
            });
            assert!(
                rx.recv_timeout(Duration::from_millis(150)).is_err(),
                "the second taker waits while the only slot is held"
            );
            gate.give();
            assert!(
                rx.recv_timeout(Duration::from_secs(30)).is_ok(),
                "the second taker runs once the slot frees"
            );
        });
        // The slot came back: the gate is whole again.
        gate.take();
        gate.give();
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

    #[cfg(feature = "gpu")]
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
