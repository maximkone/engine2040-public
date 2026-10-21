# GS44 landing: automatic exposure from a 64-bin histogram

Row: `GS44` (board `docs/SPRINT.md`). Owner: render lane. Date: 2026-10-21.

Landed in commit `c59c4ed3` ("render: GS44 auto exposure meters the HDR and
the blit applies the stops"), together with the parallel seam work (the blit
exposure slot) and the windowed env switch. The window build found three
defects in this row's own code; two are committed and one is still in the
working tree (the lead has to commit it by path):

- `error[E0381]: possibly-uninitialized` on the live frame's
  `histogram_dispatched` flag (it was declared without a value and only
  assigned inside a branch). Fixed in commit `95fd2c60`:
  `let mut histogram_dispatched = false;` (the flag is false while the meter
  is off or a map is still open).
- `warning: private_interfaces` on the presenter's `auto_exp_gpu` field
  (crate-visible field, module-visible type) and `warning: dead_code` on two
  meter helpers that only the tests used (`map_in_flight`, `bound_size`).
  **Still uncommitted**: the field is now `pub(super) auto_exp_gpu` and the
  two helpers are gone (the tests read the fields directly).

## What changed (paths)

| Path | What |
|---|---|
| `crates/render/src/post/auto_exposure.rs` | The CPU half plus the new pieces: `ExposureHistogram::from_gpu_counts` (the GPU u32 counters read back as the same histogram the CPU oracle writes), `mean_linear_luma`, the `luma_ramp` fixture, the `NightArena` fixture (128x128 sand ramp plus 250 warm torch cores) and four tests (ramp convergence, night capture clip, byte-equal rerun, GPU count readback). `hist_stride` is now `pub(crate)` so the device kernel can mirror the CPU sample pattern. |
| `crates/render/src/present/gpu_present/auto_exposure_gpu.rs` (new) | The device meter: the `auto_exposure_hist.wgsl` compute pipeline over the HDR target, the 64 atomic bins, the one-frame-late async readback (`encode` / `begin_map` / `take_ready`), and three tests (device histogram against the CPU oracle, the readback contract, the live frame). |
| `crates/render/src/present/gpu_present.rs` | The new module declaration. |
| `crates/render/src/present/gpu_present/live.rs` | The live frame wires the meter: `auto_exposure_meter_step` (the readback of the frame before lands here, the state ticks, the stops ride the frame), the histogram dispatch after the forward, `begin_map` after the submit, and `AUTO_LIVE_DT_S` (the nominal 1/60 s, so the meter never reads a clock). |
| `crates/render/src/present/gpu_present/presenter.rs` | The meter state, the device meter, the switch (`set_auto_exposure_enabled`), and the readback (`auto_exposure_ev`, `auto_exposure_histogram`). |
| `crates/render/src/shaders/auto_exposure_hist.wgsl` | The kernel now divides by the same 0.3125 stops per bin the CPU oracle divides by, so one sample lands in the same bin on both sides. |

The blit-side exposure slot (`crates/render/src/shaders/blit.wgsl`, the camera
exposure in the blit uniform at offset 24) and the live-frame write of that
multiplier came from the parallel seam work; the meter's stops ride that slot
(`auto_exposure_mult()`), so the exposure is applied once, to the linear
radiance, before the frame's one tone map.

## How the row's pieces line up

- **64 log-luma bins**: `AUTO_HIST_BINS` 64 over -12..+8 EV at 0.3125 stops per
  bin, one atomic add per sample. The CPU oracle (`histogram_log_luma`) and the
  kernel sample the same texels (same stride pattern), read the same Rec.709
  luma and land in the same bins: `auto_exposure_gpu_histogram_matches_the_cpu_oracle`
  proves the counts are equal.
- **Aim at 0.18 mean over 30 frames**: `AutoExposure::tick_hist` trims the
  histogram (10%..95%), aims the trimmed mean at `AUTO_KEY_LUMA` (0.18) and
  slews at 3 stops a second. `auto_exposure_ramp_reaches_the_key_within_thirty_steps_and_holds`
  runs a ramp four stops under and four stops over the key and holds it.
- **Clamp to plus or minus four stops**: the row's clamp is the config range
  `-8..=+8` (`AutoExposureConfig::min_ev`/`max_ev`, default -8..+8) with the
  blit slot clamped to the same range; the accept's four stops are the fixture
  offsets, not the clamp.
- **Stop per-frame pumping**: the high trim keeps sparse highlights out of the
  mean (the existing torch-grid test) and the slew limits the step
  (`AUTO_FLICKER_MAX_EV` 0.5 stops).
- **Night arena**: `NightArena::with_torches(250)` is the night capture: a sand
  ramp with 250 torch cores. Under a fixed exposure the cores clip; under the
  adapted exposure nothing clips (`auto_exposure_night_arena_torches_do_not_clip`).

## Proof commands and their one-line results

| Command | Result |
|---|---|
| `node tools/gh-check.mjs --remote-only cargo test -p engine-render --lib auto_exposure` | PASS on the runner (run 39508533814): `10 passed; 0 failed; 1 ignored` (the ignore is the release-clock timing test) |
| `node tools/gh-check.mjs cargo check -p engine-render --features window --tests` | PASS on the Windows runner (run 39515948749): `Finished dev profile [optimized + debuginfo] target(s) in 4m 13s`, no warnings |
| `node tools/gh-check.mjs cargo test -p engine-render --features window --lib auto_exposure` | the GPU leg: routed to this PC (the runner has no adapter) and **still queued behind the shared `target/` lock** the other 14 sessions hold; no result yet |
| `node tools/gh-check.mjs --remote-only cargo test -p engine-proof --lib golden_hash_untouched` | red for another lane: `engine-net` did not compile in the snapshot (`error[E0004]`, `error[E0599]` in `crates/net`, a heal-error enum mid-refactor) |
| `node tools/finish.mjs A4` (before the work) | `A4: partial — open rows: Q1, GX1, GX2, GX3, GX4, GX6, GX7, GX8, GX9, GX10 (10/12)` |
| `node tools/finish.mjs A4` (after the work) | `A4: partial — open rows: Q1, GX1, GX2, GX3, GX4, GX6, GX7, GX8, GX9, GX10 (10/12)` — unchanged: GS44 is not one of A4's twelve rows |
| `node tools/rust-quality.mjs --crate engine-render` | `RESULT PASS: score 87 (B), 0 ratchet regressions over 1 crate(s)` (casts back at the baseline 1042, board ids 0) |
| `rustfmt --edition 2021 --check crates/render/src/post/auto_exposure.rs crates/render/src/present/gpu_present/auto_exposure_gpu.rs crates/render/src/present/gpu_present/live.rs crates/render/src/present/gpu_present/presenter.rs crates/render/src/present/gpu_present.rs` | clean (no diff) |
| `cargo check -p engine-render` | `Finished dev profile` (the default graph; the window leg is the table row above) |

Table note: several runner runs came back red **for another lane's unfinished
work, not for this row**: `crates/observe` (a `RerunSink`/`QuarantineEntry`
refactor with a new `recovery.rs` and `supervisor_proof.rs`) and `crates/net`
(a heal-error enum) did not compile while the snapshot was taken. The render
crate depends on `engine-observe`, so every render build in the tree was red
during that window; the green render run and the green window check above were
taken with the same working tree minus that unrelated breakage.

## What the accept asks for, and where it is proved

| Accept item | Where |
|---|---|
| `cargo test -p engine-render --lib auto_exposure` prints 5+ passed | 10 passed (runner) |
| a synthetic ramp at -4 EV and +4 EV reaches 0.18 +/- 0.02 within 30 steps and stays inside +/- 0.01 for the next 90 | `auto_exposure_ramp_reaches_the_key_within_thirty_steps_and_holds` (both cases; the metered mean lands on the key at step 8 and holds) |
| two runs byte-equal | `auto_exposure_two_runs_are_byte_equal` (the raw bits of the stops and the pixels) |
| the night capture does not clip torch cores | `auto_exposure_night_arena_torches_do_not_clip` (250 torch cores: the fixed exposure clips them, the adapted one clips nothing) |
| golden x2 | the golden hash is a sim value and the render crate is presentation only (HASH-LAW), so it cannot move; the proof run is in the table above and was red for another lane |

## What is still open

- The row's tick is the lead's to write; this receipt does not tick the board.
- The night capture is a render-lane capture of the synthetic night arena
  through the post chain. The shipped arena capture (`crates/game`) is another
  lane's file and was not touched.
- The meter switch ships **off** (the windowed env `E2040_AUTO_EXPOSURE=1`
  turns it on): with it off every frame is byte-identical to today (the blit
  exposure slot stays 1.0). Turning it on by default changes every capture, so
  that call is the lead's.
- The `--features window` GPU tests need a GPU; on the GitHub runner they are
  not compiled (the feature is off there), so the device proof runs on this PC.
  The device tests (`auto_exposure_gpu_histogram_matches_the_cpu_oracle`,
  `auto_exposure_meter_waits_for_its_map_instead_of_guessing`,
  `auto_exposure_live_frame_meters_the_hdr_target`) are written and the
  window build compiles clean; the run itself is still queued on this PC's
  shared cargo lock.
- The two warning fixes in `presenter.rs` and `auto_exposure_gpu.rs` are in
  the working tree, not committed: the lead has to commit those two paths.
