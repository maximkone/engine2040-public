# Skill-on vs skill-off pairs: vram-budget-checks

Three VRAM review situations. "Off" is habit; "On" names the check, the failure it stops, and the proof it asks for.

## P1: float budget with a query inside (checks 1, 8)

Input: a diff adds `crates/perf/src/vram.rs::budget()` with `f64` math, `budget - reservation` that can wrap, and the Windows `QueryVideoMemoryInfo` call inside the same fn. Test: one fixed numbers case passes.

Answer with the skill: Reject. Check 1: keep the math saturating integer (`reservation = min(budget/2, requested)`, `pressure_pct`), no float, no panic path; move the OS query to the caller so the pure fn stays `x2 equal`. Check 8: prove `crates/perf/src/budget.rs::check_report` still passes and the golden hash is unchanged, since presentation must not steer the tick.

What a skill-less answer misses: the wrap on `usage above budget`, the non-determinism of float and of the OS call inside the pure fn, and the hash proof.

## P2: CPU copy into the live pool (checks 2, 4)

Input: a diff in `crates/render/src/uploads.rs` copies new bytes straight into `StagingBelt` live storage with `memcpy`, then clears the ring entry at once to "keep it fast". Test: single upload looks right.

Answer with the skill: Reject. Check 2: `StagingBelt::upload` must refuse `OverCapacity` unchanged, stage through the ring, and `advance_frame` must retire only fenced-done entries; a full belt must throttle the loader, not overwrite. Check 4: queue the move in command order so use ends before move and move before reuse; a CPU wait that tracks each texture use is the bug the order removes. Proof: `live_bytes`, `high_water_bytes`, and a fill-then-throttle test.

What a skill-less answer misses: the corrupt-when-moving race, the early-clear that reuses bytes still in flight, and the missing back-pressure on the loader thread.

## P3: uncapped sweep plus evict-on-miss (checks 5, 6)

Input: a diff in `crates/render/src/stream.rs` defrags the whole pool every frame with no byte cap and evicts on every miss, including pinned L0 in `TileResidency`. PR says "fewer misses". Test: small pool passes.

Answer with the skill: Not yet. Check 5: cap bytes per pass and split the pool into regions, gated by `crates/perf/src/budget.rs::check_time_ms`; one hole at the start must not copy the pool. Check 6: score residents vs waiters once per frame, never evict pinned L0, watch `TileResidency::evicted`, `misses_uncovered`, and `crates/assets/src/stream.rs::StreamEvent::Evicted` for thrash. Proof: frame-time numbers plus a pathological-layout case.

What a skill-less answer misses: the spike on the pathological layout, the L0 evict that turns one miss into every-frame misses, and the thrash loop with no score.
