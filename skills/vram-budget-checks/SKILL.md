---
name: vram-budget-checks
description: "Use when a builder wires VRAM math, a staging belt, a streaming pool, or an eviction rule in crates perf, render or assets before calling the budget live."
version: 0.1.0
author: skillworks
---

# VRAM budget checks

Presentation-only pool math. VRAM answers never reach the tick or the hash.

1. **Pure integer math.** Ask: does VRAM pool math use saturating integer ops for reservation, budget, and usage, after `crates/assets/src/stream.rs::StreamQueue` (`used_bytes`, `remaining_bytes`)? Flag: float math, panics on underflow, OS query inside the pure fn. (assets)
2. **Staging ring throttles.** Ask: does `crates/render/src/uploads.rs::StagingBelt::upload` refuse `OverCapacity` unchanged and `advance_frame` retire only done frames? Flag: CPU copy into the live pool, ring entry cleared before its fence. (render)
3. **Chunks split and merge.** Ask: does the pool split a free chunk on place and join neighbours on free, aligned to the platform max? Flag: fixed blocks for variable sizes, neighbours left split so the pool shreds. (assets)
4. **Moves stay ordered.** Ask: are pool moves queued in command order so use finishes before move and move before reuse, beside `crates/render/src/stream.rs::TileResidency`? Flag: a CPU wait that tracks each texture use by hand. (render)
5. **Defrag has a cap.** Ask: is each pass capped in bytes per frame and regioned, checked by `crates/perf/src/budget.rs::check_time_ms`? Flag: a full-pool sweep with no cap that spikes on one hole at the start. (perf)
6. **Eviction scores once per frame.** Ask: does eviction compare resident vs waiter scores and keep pinned L0, seen in `TileResidency::evicted` and `crates/assets/src/stream.rs::StreamEvent::Evicted`? Flag: evict-on-miss with no score, L0 evicted, thrash with no guard. (render)
7. **Headers apart from data.** Ask: are fixed headers pooled apart from variable data with handles that fail stale, via `crates/assets/src/registry.rs::Registry::get`? Flag: a bare slot index that silently reads the next occupant. (assets)
8. **Never enters the hash.** Ask: do budget, pressure, and mip bias stay out of `state_hash`, proved by `crates/perf/src/budget.rs::check_report` and `docs/HASH-LAW.md`? Flag: VRAM numbers steering the tick or moving the golden hash. (perf)

## Do / Don't
- Do: keep pure math saturating, queue moves in order, cap defrag, score eviction, and prove the golden hash unchanged. Don't: float the budget, copy on the CPU, sweep the pool uncapped, evict without a score, or feed VRAM into the tick.

source: Game Engine Gems 1, ch. 11
