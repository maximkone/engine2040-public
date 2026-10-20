# Landing receipt: OPT27 provider inference arbiter (local models and the game)

Landing agent, 2026-10-21. Row: `docs/SPRINT-13-engine-edge.md` OPT27 ("provider:
inference arbiter — local models and the game"). Owned paths: `crates/provider/**`
and `crates/endpoint/src/packs/**`. No verb was added to the endpoint packs (the row
does not need one), so `crates/endpoint/**` is untouched.

## What the row still wanted

The board progress line for OPT27 reads: "provider side done (arbiter, serving
adapter); left: feed live frame p95 and read_vram, window-build budget check on a
GPU."

The arbiter existed but nothing could hand it a live reading: `PressureSample` had
to be built by hand by a caller that did not exist. This landing adds that wire and
proves the provider half of the budget check.

## What I changed

- `crates/provider/src/pressure.rs` (new): `PressureFeed` — the live feed. It
  keeps the last `FRAME_WINDOW` (120) frame times, computes the frame-time p95
  (nearest rank, `ceil(0.95 n)`, integers only), and turns each frame plus a
  VRAM reading into the one `PressureSample` the arbiter decides on.
  `push_frame(frame_us, vram)` returns the typed `ArbiterNotice` when the
  placement moved. `route(cpu, bound)` builds the `LocalRoute` on the same
  reader the feed publishes, so the request path always calls the sidecar the
  placement names. No clock reads, no allocation after construction, one atomic
  store per frame. `FrameWindow` keeps the frames sorted by time with their
  arrival number, so the oldest frame drops and the p95 is one index read.
- `crates/provider/src/arbiter.rs`: `VramHeadroom::from_bytes(budget, used)` and
  `VramHeadroom::MB` — the `read_vram` seam. The OS reading (the shape
  `render::backend::read_vram` returns, DXGI bytes) becomes the MB the arbiter's
  thresholds use; a missing half reads as `Unknown`, never as a guess.
- `crates/provider/src/lib.rs`: `pub mod pressure;`, the re-exports
  (`FrameWindow`, `PressureError`, `PressureFeed`, `FRAME_WINDOW`,
  `MAX_FRAME_WINDOW`) and one doc line.

Tests (new, in `crates/provider/src/pressure.rs`):

1. `frame_window_p95_is_the_nearest_rank_over_the_kept_frames` — the p95 rule and
   the window bound; bad windows refused by name.
2. `live_frames_move_the_placement_with_typed_notices` — quiet frames earn the
   GPU, hot frames take it away, every move carries its reason, and the same
   frame series twice gives the same notices (no clock, no guess).
3. `a_vram_reading_feeds_the_arbiter_and_never_guesses` — the `read_vram` seam:
   bytes become MB, a missing half is unknown, a short reading refuses the GPU
   and a squeezed reading leaves it at once with `MoveReason::VramShort`.
4. `a_loaded_local_model_keeps_the_frame_budget_and_degrades_with_typed_notices`
   — the budget check, provider half: with a local model loaded (a loopback
   sidecar stands in for `llama-server`), quiet frames keep it on the GPU and it
   serves there; hot frames take it off the GPU, the tick's request parks and
   comes back as a typed `AiError::Unavailable` refusal with a typed
   `Hold::Deferred` / `Hold::QueueFull` and a counted refusal, 60 frames of
   provider work cost less than one frame budget, and the parked work runs later
   (off the tick) with the same validated proposal.

## Proof (exact commands, one line each)

- `node tools/gh-check.mjs --remote-only --budget 540 cargo test -p engine-provider --lib pressure`
  → PASS on the runner: 7 passed, 0 failed (run 39494694249).
- `node tools/gh-check.mjs --remote-only --budget 540 cargo test -p engine-provider --lib`
  → PASS on the runner: 273 passed, 0 failed, 6 ignored (run 39497097770).
- `node tools/gh-check.mjs --remote-only --budget 540 cargo clippy -p engine-provider --all-targets -- -D warnings`
  → PASS on the runner, no warnings (run 39496319804).
- `node tools/gh-check.mjs --remote-only --budget 540 cargo test -p engine-provider --test matrix`
  → PASS on the runner: 2 passed (run 39496601823).
- `node tools/rust-quality.mjs` → FAIL: 10 ratchet regressions, all in other
  crates (`engine-ai`, `engine-endpoint`, `engine-observe`, `engine-physics`,
  `engine-render`, `engine-web`). `engine-provider` is not among them.
- `cargo check -p engine-provider --all-targets` (local) → first run: exit 1, two
  compile errors in my new file (a shadowed helper name, a trait not in scope).
  Both fixed; the final tree then proved on the runner (the lines above, clippy
  `-D warnings` clean) and the local re-run finished green: "Finished `dev`
  profile [optimized + debuginfo] target(s) in 2m 03s" (it waited on the shared
  `target/` lock first, which is normal in this tree).

The tick hash golden `d57b5c8ae3096614` did not move: no sim crate and no tick
path was touched (`crates/provider` is outside the fixed tick; live inference
still runs off the tick and submits validated recorded actions).

## How the row's Accept maps to the proof

- "the arbiter picks the provider under a budget" — `PressureFeed` turns the
  live frame p95 and the live VRAM reading into the placement under the frame
  budget (`live_frames_move_the_placement_with_typed_notices`), and the request
  path follows it (`a_loaded_local_model_keeps_the_frame_budget...`).
- "requests degrade with typed notices" — `ArbiterNotice` reasons plus
  `Hold::Deferred` / `Hold::QueueFull` and the counted refusal, proved in the
  same test.
- "the same task gives the same result on mock, local and remote" — the provider
  conformance matrix: same transcript, preview and fork on every backend
  (`--test matrix`, PASS).
- "replay runs with no model call at all" and "the cassettes are content-addressed
  and re-run byte-equal" — the cassette suite inside `--lib` (273 passed): the
  replay path records once, replays twice byte-identical with zero socket
  attempts, and a changed request misses by content with a typed reason.

## Still open

1. The live window-build budget check on the reference GPU (the row's own
   wording: "with a local model loaded the window build holds the P6 frame
   budget"). It needs the render/game lane: the window must call
   `PressureFeed::push_frame` with its frame time and
   `VramHeadroom::from_bytes(read_vram())` every frame, and the P6 numbers
   (CPU frame p95 ≤ 8 ms, GPU p95 ≤ 14 ms at Base) must be measured on the RTX
   3050 with a local model loaded. Those crates are not mine; this landing
   delivers the feed and the seam and proves the provider half only.
2. No caller of the feed yet. The window (or the game loop) has to own the
   `PressureFeed` and push a frame per frame; until then the arbiter is fed by
   hand in tests only.
3. The board row is not ticked by me: the tick needs the landing sha, so the
   lead must add `- [x] OPT27 ... Landed <sha7>` to
   `docs/SPRINT-13-engine-edge.md` when committing the three paths.
4. Note for the lead: another agent's git operation reverted my edits to
   `crates/provider/src/lib.rs` and `crates/provider/src/arbiter.rs` at 20:36Z
   during this session (the untracked `pressure.rs` survived). I re-applied them
   and re-ran the proofs above on the final tree. Please commit
   `crates/provider/src/pressure.rs`, `crates/provider/src/arbiter.rs` and
   `crates/provider/src/lib.rs` by path before another sweep runs.

## Files

- `crates/provider/src/pressure.rs` (new)
- `crates/provider/src/arbiter.rs` (edited)
- `crates/provider/src/lib.rs` (edited)
- `docs/research/powerups-2026-10-20/landing-OPT27.md` (this receipt)
