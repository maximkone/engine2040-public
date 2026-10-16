---
name: sky-bake-checks
description: "Use when a builder wires the analytic sky bake, its fog feed or its exposure anchor in crates render before calling the sky live."
version: 0.1.0
author: skillworks
---

# Sky bake checks

Nine checks for a physical-sky diff. Name the crate in each finding (render). PASS needs a bake or pixel proof per item, never a promise.

1. **Sun direction valid.** Ask: does `SkyParams::validate` in `crates/render/src/sky.rs` require finite nonzero `sun_dir` via `validate_frame_args`? Flag: zero or non-finite sun, or north/up swapped so the sun rises west. (render)
2. **Air pinned.** Ask: is `turbidity` inside `SKY_TURBIDITY_MIN..=SKY_TURBIDITY_MAX` with `SkyParams::midday` near 2.0? Flag: clean 1.0 or hazy 20.0 air that breaks the scatter math. (render)
3. **Small cached bake.** Ask: does `bake_sky` build `SKY_CAPTURE_W x SKY_CAPTURE_H` (64x32) under `SKY_CAPTURE_MAX_DIM`, cached until time moves, inside `SKY_BUDGET_MS`? Flag: full-res or per-frame rebake that eats the frame. (render)
4. **Bytes identical.** Ask: does `SkyCapture::to_bytes` match x2 with row-major order and f64 accumulation in `mean_radiance`? Flag: HashMap order or f32 sum that drifts run to run. (render)
5. **Fog from horizon.** Ask: does `fog_color` average the equator rows and `look_fog_color` feed `FogParams::from_sky_capture`? Flag: zenith or ground texels tinting fog, or fog on a constant. (render)
6. **Live sky sampled.** Ask: does `fs_sky` in `crates/render/src/shaders/forward.wgsl` sample the sky-view texture with `fog_factor_infinite`, bound via `FogFwd::day` in `presenter.rs`? Flag: bake on disk while `fs_sky` draws the analytic ramp only. (render)
7. **Sky-fog agree.** Ask: is the fully fogged composite within `FOG_SKY_RAMP_TOL` of the fog color, with sky on-vs-off above the EG8 epsilon? Flag: distant sand cut out against the sky, or a probe-passing sky with identical pixels. (render)
8. **Exposure from EV100.** Ask: does `ev100_to_exposure` carry `exposure_ev100` (`ARENA_DAY_EXPOSURE_EV100` 12.0) to fog and IBL, tone mapped once in post? Flag: exposure baked into sky texels, or a second tone map in `fs_sky`. (render)
9. **Ground sane, joined.** Ask: is `ground_albedo` inside 0..=1 (`arena_day.rs` sand 0.42/0.32/0.20) and `spawn_look_bake` joined before bind? Flag: 100% white sand that blows out, or a frame that reads the fallback bake. (render)

## Do / Don't
- Do: bake once at 64x32, bind the texture in `fs_sky`, feed fog and exposure from it, prove on-vs-off pixels differ. Don't: rebake per frame or land a bake no live pass samples.

source: Game Engine Gems 1, ch. 15
