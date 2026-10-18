//! alloc_probe: the one heap-counting allocator for zero-allocation proofs.
//! Owner: engine-core (feature `test-util`). A test binary installs it once:
//!
//! ```ignore
//! #[cfg(test)]
//! #[global_allocator]
//! static ALLOC: engine_core::alloc_probe::CountingAlloc = engine_core::alloc_probe::CountingAlloc;
//! ```
//!
//! then proves a hot path with `let (_, n) = measure(|| world.step()); assert_eq!(n.calls, 0);`.
//! Counting is per thread, so parallel tests never see each other's heap use.
//! Reallocation counts as an allocation (the default `realloc` allocates).
//! Frees on the measured thread lower the live level, so [`AllocCount::peak_live_bytes`]
//! is the high-water mark of the heap above the level at the start of the window.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static CALLS: Cell<u64> = const { Cell::new(0) };
    static BYTES: Cell<u64> = const { Cell::new(0) };
    /// Live heap level against the window start; frees of older blocks go below zero.
    static LIVE: Cell<i64> = const { Cell::new(0) };
    static PEAK: Cell<i64> = const { Cell::new(0) };
}

/// `System` allocator that counts this thread's allocations while armed.
#[derive(Clone, Copy, Debug, Default)]
pub struct CountingAlloc;

#[expect(
    unsafe_code,
    reason = "the workspace's one unsafe site: a counting allocator is only expressible as an unsafe GlobalAlloc impl"
)]
// SAFETY: every call forwards the caller's layout and pointer unchanged to
// `System`, so `System`'s guarantees hold. The counters are const-initialized
// thread-locals that never allocate, so counting cannot recurse or race.
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s layout contract.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() && ARMED.get() {
            CALLS.set(CALLS.get().saturating_add(1));
            let size = u64::try_from(layout.size()).unwrap_or(u64::MAX);
            BYTES.set(BYTES.get().saturating_add(size));
            let live = LIVE
                .get()
                .saturating_add(i64::try_from(size).unwrap_or(i64::MAX));
            LIVE.set(live);
            PEAK.set(PEAK.get().max(live));
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `alloc` above with this same `layout`.
        unsafe { System.dealloc(ptr, layout) };
        if ARMED.get() {
            let size = i64::try_from(layout.size()).unwrap_or(i64::MAX);
            LIVE.set(LIVE.get().saturating_sub(size));
        }
    }
}

/// Heap use observed by [`measure`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AllocCount {
    /// Allocation calls (including growth by reallocation).
    pub calls: u64,
    /// Bytes requested by those calls.
    pub bytes: u64,
    /// Highest live heap reached in the window, in bytes above the heap level
    /// at its start: requests minus frees, so a buffer made and dropped over
    /// and over reads once, not once per round. Frees made on other threads
    /// are not seen.
    pub peak_live_bytes: u64,
}

/// Run `f` with this thread's allocations counted. Nested calls are allowed:
/// the inner count restarts and the outer arm state is restored afterwards.
/// Counts are zero unless [`CountingAlloc`] is the global allocator.
pub fn measure<R>(f: impl FnOnce() -> R) -> (R, AllocCount) {
    let was = ARMED.get();
    CALLS.set(0);
    BYTES.set(0);
    LIVE.set(0);
    PEAK.set(0);
    ARMED.set(true);
    let value = f();
    ARMED.set(was);
    let count = AllocCount {
        calls: CALLS.get(),
        bytes: BYTES.get(),
        peak_live_bytes: u64::try_from(PEAK.get()).unwrap_or(0),
    };
    (value, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_heap_use_and_ignores_stack_work() {
        let (sum, none) = measure(|| (0..64u64).sum::<u64>());
        assert_eq!(sum, 2016);
        assert_eq!(none, AllocCount::default());
        let (v, some) = measure(|| vec![0u8; 100]);
        assert_eq!(v.len(), 100);
        assert_eq!(some.calls, 1);
        assert!(some.bytes >= 100);
    }

    #[test]
    fn growth_counts_and_reserved_capacity_does_not() {
        let mut buf: Vec<u32> = Vec::with_capacity(16);
        let ((), reserved) = measure(|| buf.extend(0..16));
        assert_eq!(
            reserved.calls, 0,
            "pushing within capacity is allocation-free"
        );
        let ((), grown) = measure(|| buf.push(16));
        assert_eq!(grown.calls, 1, "growing past capacity reallocates");
        assert_eq!(buf.len(), 17);
    }

    #[test]
    fn peak_live_counts_what_is_held_not_what_is_requested() {
        let ((), churn) = measure(|| {
            for _ in 0..8 {
                std::hint::black_box(vec![0u8; 1 << 20]);
            }
        });
        assert_eq!(churn.calls, 8);
        assert_eq!(churn.bytes, 8 << 20, "eight 1 MiB requests in total");
        assert_eq!(churn.peak_live_bytes, 1 << 20, "never more than one held");
        let (held, kept) = measure(|| (0..4).map(|_| vec![0u8; 1 << 20]).collect::<Vec<_>>());
        assert!(
            (4 << 20..(4 << 20) + 1024).contains(&kept.peak_live_bytes),
            "four buffers held at once, plus the outer vec: {}",
            kept.peak_live_bytes
        );
        drop(held);
    }

    #[test]
    fn disarmed_after_measure() {
        let _ = measure(|| ());
        let before = CALLS.get();
        let v = vec![1u8; 8];
        assert_eq!(CALLS.get(), before, "no counting outside measure");
        drop(v);
    }
}
