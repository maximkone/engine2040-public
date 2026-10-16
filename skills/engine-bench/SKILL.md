---
name: engine-bench
description: Use when measuring engine2040 performance - BENCH packet form, paired comparable runs, distribution and allocation reporting. Loaded by the pilot for BENCH packets.
---

# Engine bench (from pilot)

BENCH states mode (read-only measurement or writer on owned paths),
consumer, crate, baseline, fixture, run budget. Extend existing
harnesses. One machine/device exclusively; parallel builds/GPU tests
invalidate timings unless controlled.

Report hardware/driver, build/profile, seed/input, warmup, sample
count, distribution (p50/p95/p99), allocations/peak memory, raw output
paths. Paired comparable runs; report noise, regressions, gains. No
threshold weakening, timing-test deletion, golden repinning. GPU claims
need actual GPU path; missing baselines/hardware stays unverified.
Allocation claims use `engine_core::alloc_probe::measure` (calls/bytes
per tick after warmup). CPU regressions prefer Gungraun (iai-callgrind)
beside wall-clock p50/p95/p99; GPU claim carries per-pass timestamps.

Return bottleneck, evidence, smallest next improvement; no source writes on measurement-only packets, no commits/dispatch.
