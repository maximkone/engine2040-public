# SPRINT-23: books 2026-10-19

Rows from the 2026-10-19 book pass. Spec: `docs/plan/BOOK-GAPS-3-2026-10-19.md`.
The two Dunsky animation gaps stay on AM14 and AM15; the GPG8 LOD gap stays on
AM13. Their plan text was stale and is corrected.

- [x] NB1 geo: the Rust/C++ FFI pair has layout asserts — `crates/geo/src/uv.rs:83-137` Landed 3269665a.
  mirrors `crates/geo/vendor/xatlas_shim.cpp:12-42` by hand; Miri skips the
  seam (`tools/miri-sites.json`). Add `size_of` and one `offset_of!` per field
  on the Rust side and matching `static_assert` in the shim. Diff: good.
  Tier A. Size S. Owns `crates/geo/src/uv.rs`,
  `crates/geo/vendor/xatlas_shim.cpp`. Accept: `rg -n "size_of|offset_of"
  crates/geo/src/uv.rs` hits; `cargo test -p engine-geo --lib --features uv`
  green; the real-FFI tests `uv.rs:529,707` stay green. F2P: a swapped
  same-width field passes today. P2P: golden unchanged. needs: none.
- [ ] NB2 endpoint: the stdio MCP door owns a deadline — `transport.rs:60
  timeout_for` has one hit, its definition; `mcp.rs` and `door_dispatch.rs`
  have no `timeout|deadline|Instant|elapsed`. Wire the budget into
  `door_dispatch.rs` and seal a `deadline_exceeded` refusal in the same
  `DoorAudit` record. Diff: regular. Tier A. Size S. Owns
  `crates/endpoint/src/door_dispatch.rs`, `crates/endpoint/src/transport.rs`,
  one `tests/it` case. Accept: `rg -n "timeout_for" crates/endpoint/src`
  shows a non-test caller; `cargo test -p engine-endpoint --lib` green; a
  wedged fixture returns `deadline_exceeded`. F2P: the door hangs with no
  budget. P2P: normal calls unchanged. needs: none. BLOCKED: `transport.rs:3-9`
  says the stdio door has no deadline to own; bounding a blocking call needs a
  runtime or a worker that cannot be killed, so this is an owner decision on
  the door contract.
- [ ] NB3 runtime: the job pool has a scaling curve — `ecs/src/pool.rs:212`
  scope and `physics/src/islands.rs:118 step_islands` keep bytes equal at
  1/2/4/8 threads, but no bench prints ms, speedup or efficiency;
  `crates/perf/src` has only the allocator ratio. Add the bench beside
  `step_islands`. Diff: good. Tier A. Size S. Owns the test hunk of
  `crates/physics/src/islands.rs`. Accept: `cargo test -p engine-physics
  --release islands_scale -- --nocapture` prints four rows with a speedup
  column; golden unchanged. F2P: equality is proven, throughput is not.
  P2P: golden unchanged. needs: RC36.
- [ ] NB4 ui: the HUD has aspect and safe-area rules — `render/src/overlay.rs:235`
  divides x by aspect over 0..1; `ui/src/text.rs:436` uses fixed 6x8 cells;
  no safe area, DPI scale or anchor exists. Add per-element anchors, a fixed
  safe margin and a whole-pixel DPI scale. Diff: regular. Tier B. Size S.
  Owns `crates/render/src/overlay.rs`, `crates/ui/src/text.rs`, one aspect
  test. Accept: captures at 1280x720, 1024x768 and 2560x1080 keep the HP
  strip inside a 5% margin and text >= 28 px at 1080p; a test asserts the
  strip keeps its pixel size +-1. F2P: the HUD is authored for 16:9 only.
  P2P: GP38's HP bars stay green. needs: UIX13.
- [x] NB5 anim: turn response is measured — `locomotion.rs:68-78` holds five Landed 2f09f84d.
  magic rates; `:764 turn_facing` is a proportional clamp on `-cross` with no
  damping term. A headless harness prints yaw error per tick for a 90 and a
  180 degree step at walk, run and pivot, no behaviour change. Diff: good.
  Tier B. Size S. Owns the test hunk of `crates/anim/src/locomotion.rs`.
  Accept: the harness prints settle ticks and peak overshoot; the bar (no
  overshoot, <= 12 ticks to 90%) is set from the first run and then asserted.
  F2P: no turn-response number exists. P2P: motion unchanged, golden
  unchanged. needs: none.
