---
name: gtao-ao-checks
description: Use when a builder wires GTAO ambient occlusion into the live arena frame and a reviewer checks the darkening before it ships.
---

# GTAO AO checks

Live GTAO darkens creases from depth, not a constant. Eight checks. Each names the file to open. PASS needs on-vs-off pixels plus ms, never a manifest claim.

1. **Depth source.** Ask: does `GtaoAo` in `crates/render/src/gtao.rs` read the stored forward depth? Flag: depth stored once then never read; a new prepass with no caller. (render)
2. **Half-res work.** Ask: is AO built at half res by `GTAO_HALF_DIV` and `GTAO_HALF_PASS` then lifted by `GTAO_UPSAMPLE_PASS`? Flag: full-res AO with no ms; transient above `GTAO_HALF_TRANSIENT_BUDGET_BYTES`. (render)
3. **Tap pattern.** Ask: do `GTAO_SLICES` and `GTAO_TAPS` set the slice dirs and steps in `crates/render/src/gtao.rs`? Flag: one slice or one tap with no on-vs-off gain; step ignores `GtaoConfig`. (render)
4. **Range.** Ask: are `radius_px` and `falloff_m` inside `(0, GTAO_RADIUS_MAX_PX]` and `(0, GTAO_FALLOFF_MAX_M]`? Flag: huge radius that flattens the frame; falloff that never reaches the wall base. (render)
5. **Blit multiply.** Ask: does `crates/render/src/shaders/blit.wgsl:49` clamp `ao_tex` to 0..1 and darken ambient only? Flag: `ao_tex` stuck at 1.0; AO darkening direct sun too. (render)
6. **Live, not probes.** Ask: does AO run on live frames, not only under `--look-probes` in `crates/game/src/bin/arena.rs:98` (`LOOK_PROBES_FLAG`)? Flag: live `ao_tex` stays constant while probes look fine. (game)
7. **Budget.** Ask: is live cost at or under `GTAO_BUDGET_MS` 0.8 ms by `ms <= GTAO_BUDGET_MS`? Flag: no live ms number; cost above budget with no cut. (render)
8. **Stable output.** Ask: does the same depth give the same AO twice within `GTAO_GPU_PARITY_TOL`? Flag: golden hash moves with AO on; open sand shifts over 2% on vs off. (render)

## Do / Don't
- Do: ask for feet and wall bases darker on vs off, open sand within 2%, plus live ms at or under budget.
- Don't: take a constant `ao_tex`, a probes-only run, or AO that darkens direct sun as done.

source: Game Engine Gems 1, ch. 18
