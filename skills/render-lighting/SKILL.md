---
name: render-lighting
description: "Use when you add or review lights and sky in crates/render: shadow maps, point/spot/sun lights, torches, IBL, normal or gloss maps, GTAO, sky bake, fog feed, exposure, wound or decal maps. A pass counts only if on-vs-off pixels differ."
---

# Lighting, sky and paint maps in crates/render

Seven features, one shared law. The per-feature check lists (with symbols and constants) are in [reference.md](reference.md). Open the section for your feature. wgpu names are in skill `wgpu-render`. Matrix, tangent and shadow-bias math is in skill `render-pipeline-math`. Culling and VRAM are in skill `render-cull-vram`.

## Shared law

1. **On vs off pixels.** A feature counts only if frames with it on differ from frames with it off by more than the pass-truth epsilon (`crates/render/src/pass_truth.rs::PASS_EPSILON` = 5 levels, gate EG8, spec `docs/book/ref-render.md` section 4). A bake that no live pass samples is not live. Zero validation errors proves nothing.
2. **State the ms.** Each live pass reports its cost. Compare to the budgets in `docs/VISUAL-TARGET.md` and to the named constants (`GTAO_BUDGET_MS` 0.8, `SKY_BUDGET_MS` 0.4).
3. **Nothing enters the hash.** Light, sky, AO, torch and wound bytes live in the presentation layer. They never enter `state_hash` (`docs/HASH-LAW.md`). Wall clock stays in receipts only. The golden hash is unchanged x2.
4. **Same bytes twice.** A bake or queue given the same input gives the same bytes. Row-major order, integer or f64 accumulation, no `HashMap` iteration, no clock.
5. **Bake once and bind it.** Bake from pinned inputs (fixed HDRI, fixed roughness steps, fixed capture size). Cache until the input moves. Then bind the result in the live pass. No per-frame rebake.
6. **AO scales ambient only.** Never direct sun, never the final color.
7. **Tone map once**, in post. Exposure comes from EV100 and is never baked into sky texels.
8. **A local light has finite reach.** Attenuation reaches zero at a max radius and eases out at the edge. Brightness is separate from reach.
9. **Fixed order, capped work.** Assignment, drain and eviction walk in a stable order (cell-major, or `(tick, char_id)`). Overflow is counted and reported, never silently dropped.
10. **Tangent handedness** (`tangent.w` = +1 or -1) is checked once, in skill `render-pipeline-math` check 6. Assets cook it: `crates/assets/src/import_repair.rs::recompute_tangents`.

## Feature table

| Feature | Open this | The one thing that breaks |
|---|---|---|
| Shadow maps, point, spot, sun | `crates/render/src/shadow_cascade.rs`, `crates/render/src/twin_shadow.rs` | unsnapped cascades crawl, a light with no radius, cube taps spill across faces |
| Torch point lights | `crates/render/src/shadow_cluster.rs`, `crates/render/src/shadow_cluster/torch_cluster.rs` | hash-order assignment, reach tied to brightness, over 256 lights fails instead of clamping |
| IBL, normal and gloss maps | `crates/render/src/ibl.rs`, `crates/render/src/shaders/forward.wgsl` | the bake exists but the forward pass does not sample it, so frames are identical |
| Sky bake, fog feed, exposure | `crates/render/src/sky.rs`, `crates/render/src/fog.rs`, `crates/render/src/arena_day.rs` | per-frame rebake, `fs_sky` draws the analytic ramp only, a second tone map |
| GTAO | `crates/render/src/gtao.rs`, `crates/render/src/shaders/blit.wgsl` | `ao_tex` stays 1.0 live (probes-only run), AO darkens direct sun |
| Deferred sun tiles | `crates/render` (not verified live) | full sun and soft shadow on every pixel, one draw per tile |
| Wound paint maps | `crates/render/src/wound_map.rs`, `crates/render/src/decals.rs` | one combined map smears cuts across fighters, uncapped drain |

## Do and don't

- Do bound each light, bake once, bind it, feed fog and exposure from it, and prove on-vs-off pixels plus ms.
- Do keep per-fighter paint maps capped and drained in tick order.
- Don't land a bake no live pass samples, a constant `ao_tex`, a probes-only run, or a light with no cutoff.
- Don't let light, sky or wound bytes reach the tick.

Sources: Lengyel, Foundations of Game Engine Development Vol 2 (ch. 8.1-8.3.3 lights and shadows, 10.1 decals); Gregory, Game Engine Architecture 2nd ed. (ch. 10.3.1 IBL); Game Engine Gems 1 (ch. 15 sky, ch. 18 GTAO); Game Engine Gems 2 (ch. 4 deferred).
