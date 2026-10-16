//! A-01 DelayBuffer bench: measure-only timing of a 500-tick input script.
//!
//! Pattern: push one input per tick into `engine_net::DelayBuffer`, read the
//! delayed input back each tick, checksum it. No behavior change anywhere:
//! this file only drives the public `new` / `push_local` / `readable` API
//! and prints wall-clock time. Units: ticks, compact inputs (`u32`).

use engine_net::DelayBuffer;
use std::time::Instant;

/// Ticks in the input script.
const TICKS: usize = 500;
/// Ticks between sending an input and rendering it.
const DELAY: usize = 2;

/// Deterministic nonzero input script (tick -> input). Pure function.
fn script(tick: usize) -> u32 {
    (tick as u32).wrapping_mul(0x9E37_79B1).wrapping_add(1)
}

#[test]
fn a01_delay_buffer_500_tick_timing() {
    let mut buf = DelayBuffer::new(DELAY);
    let mut checksum: u64 = 0;
    let start = Instant::now();
    for tick in 0..TICKS {
        buf.push_local(script(tick));
        checksum = checksum.wrapping_add(u64::from(buf.readable()));
    }
    let elapsed = start.elapsed();

    // Expected checksum: neutral 0 during warmup, then script[t - DELAY].
    let mut expected: u64 = 0;
    for tick in 0..TICKS {
        let v = if tick + 1 > DELAY {
            script(tick - DELAY)
        } else {
            0
        };
        expected = expected.wrapping_add(u64::from(v));
    }
    assert_eq!(checksum, expected, "delayed readout must match script");

    let total_us = elapsed.as_micros();
    let ns_per_tick = elapsed.as_nanos() / TICKS as u128;
    println!(
        "a01-delay-bench: {TICKS} ticks delay={DELAY} total={total_us}us ns/tick={ns_per_tick}"
    );
}
