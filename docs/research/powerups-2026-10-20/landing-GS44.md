# GS44 landing: automatic exposure from a 64-bin histogram

Row: `GS44` (board `docs/SPRINT.md`). Owner: render lane. Date: 2026-10-21.

## What changed (paths)

| Path | What |
|---|---|
| `crates/render/src/post/auto_exposure.rs` | The CPU half plus the new pieces: `ExposureHistogram::from_gpu_counts` (the GPU u32 counters read back as the same histogram the CPU oracle writes), `mean_linear_luma`, the `luma_ramp` fixture, the `NightArena` fixture (128x128 sand ramp plus 250 warm torch cores) and four new tests (ramp convergence, night capture clip, byte-equal rerun, GPU count readback). `hist_stride` is now `pub(crate)` so the device kernel can mirror the CPU sample pattern. |
| `crates/render/src/present/gpu_present/auto_exposure_gpu.rs` (new) | The device meter: the `auto_exposure_hist.wgsl` compute pipeline over the HDR target, the 64 atomic bins, the one-frame-late async readback (`encode` / `begin_map` / `take_ready`), and three tests (device histogram against the CPU oracle, the readback contract, the live frame). |
| `crates/render/src/present/gpu_present.rs` | The new module declaration. |
| `crates/render/src/present/gpu_present/live.rs` | The live frame wires the meter: `auto_exposure_meter_step` (the readback of the frame before lands here, the state ticks, the stops ride the frame), the histogram dispatch after the forward, `begin_map` after the submit, and `AUTO_LIVE_DT_S` (the nominal 1/60 s, so the meter never reads a clock). |
| `crates/render/src/present/gpu_present/presenter.rs` | The meter state, the device meter, the switch (`set_auto_exposure_enabled`), and the readback (`auto_exposure_ev`, `auto_exposure_histogram`). |
| `crates/render/src/shaders/auto_exposure_hist.wgsl` | The kernel now divides by the same 0.3125 stops per bin the CPU oracle divides by, so one sample lands in the same bin on both sides. |

The blit-side exposure slot (`crates/render/src/shaders/blit.wgsl`, the camera
exposure in the blit uniform at offset 24) and the live-frame write of that
multiplier landed in the same tree from the parallel seam work; the meter's
stops ride that slot (`auto_exposure_mult()`), so the exposure is applied once,
to the linear radiance, before the frame's one tone map.

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

(see the table below; every line is the exact command and its result)

## What is still open

- The row's tick is the lead's to write; this receipt does not tick the board.
- The night capture is a render-lane capture of the synthetic night arena
  through the post chain. The shipped arena capture (`crates/game`) is another
  lane's file and was not touched.
- The meter switch ships **off**: with it off every frame is byte-identical to
  today (the blit exposure slot stays 1.0). The live-frame test turns it on.
  Turning it on by default changes every capture, so that call is the lead's.
- The `--features window` GPU tests need a GPU; on the GitHub runner they are
  not compiled (the feature is off there), so the device proof runs on this PC.
