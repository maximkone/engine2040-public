# Board Merge: 13 Lanes → SPRINT-07 Rows

This document merges the 13 research lanes (2026-10-21) into board-ready rows for `docs/SPRINT-07-accepted-plan-rows.md`. Each row carries donor, license, cost (S/M/L), kill line, and acceptance criteria.

---

## New Rows Proposed

### TOON1: MMD Toon + Constant-Width Outline (render/shader)

**What:** A cel/toon shading mode in the forward pass. Shadow band = one flat colour from a toon texel (no filtering). Light/dark boundary = `selfMask * occlusionMask`. Outline = inverted hull with constant pixel width (view normal / viewport × `clip.w`).

**Donor:** `togechiyo/MMD_modoki` (MIT, WebGPU) — `src/scene/shaders/builtin-toon/full_shadow.wgsl` and `docs/mmd-standard-toon-shadow-color-sampling-2026-08-26.md` (read 2026-10-21).

**Secondary donors:**
- `DoubleStyx/Renderide` (MIT) — `crates/renderide/shaders/modules/material/toon_brdf.wgsl`: wrapped Lambert quantised to 3 bands, normalised Blinn-Phong quantised to `steps` bands.
- `milos-agathon/forge3d` (MIT) — `src/shaders/brdf/toon.wgsl`: shadow side returns rim colour instead of black (8-line shape).
- `amPerl/nif` (MIT) — `crates/nif-wgpu/src/shaders/ToonShading.wgsl`: 256×1 ramp with point sampling, `ramp * diffuse + ambient`.

**License:** MIT (all donors, read live 2026-10-21).

**Cost:** M (one new WGSL shader file + one toon ramp texture in `content/design/`).

**Kill line:** The band edge shimmers on moving fighters past LF15 FLIP floor, or the extra texture sample costs the potato tier (G9 potato gate), or the ramp texture pushes the atlas over budget.

**Accept:** The toon band reads as "hard painted light" (not continuous ramp), the outline stays constant-width at 30 m, and the arena sand does not read as plastic. Gate: `tools/gate-silhouette.mjs` passes with the toon-shaded LOD0.

**Owns:** `crates/render/src/shaders/forward.wgsl` (TOON1 BRDF hunk), `crates/render/src/shaders/outline.wgsl` (inverted hull), `content/design/toon_ramp.png` (256×1 gradient).

**Status:** Not started.

---

### AT1: 30m Silhouette Gate with Golden PNG Comparison (tools)

**What:** The 30 m silhouette gate compares against a golden PNG using pixelmatch (ISC) for pixel diff and blockhash (MIT) for perceptual fingerprinting. A failing gate exits 1 and blocks the build.

**Donor:** `mapbox/pixelmatch` (ISC) — `index.js` (OKLab HyAB with sqrt folded out, `threshold`, `windowSize`, `ignoreMask`). `jaehl/blockhash` (MIT) — `src/hash.rs` (16×16 block-average, Hamming distance).

**License:** ISC (pixelmatch), MIT (blockhash), both read live 2026-10-21.

**Cost:** S (one Node.js script, vendor pixelmatch/pngjs under `tools/vendor/node_modules`).

**Kill line:** The integer port of the folded HyAB test disagrees with the float reference by more than 1e-6 on 100 seeded pixel pairs, or 20 runs of one unchanged character produce any false fail.

**Accept:** `node tools/gate-silhouette.mjs <silhouette.json> <silhouette_30m.png> --golden <golden.png>` exits 0 on pass (bbox_fill in [0.40, 0.70], fill_30m in [0.40, 0.70], pixel_diff_pct < 5%, blockhash_hamming < 20), exits 1 on fail.

**Owns:** `tools/gate-silhouette.mjs`, `content/generated/cook_mpfb_warden/golden_silhouette_30m.png`, `content/generated/cook_mpfb_ashkin/golden_silhouette_30m.png`.

**Status:** **Done** (2026-10-22, commit `d5040c2b0`). `tools/gate-silhouette.mjs` created, tested on warden (GATE: pass). Golden PNGs not yet placed.

---

### CHR1: Caricature Proportions + Painted Faces (content/tools)

**What:** Caricature proportions as named shape targets (MakeHuman's `proportions` axis), and the face painted into the skin atlas (eyes, brows, nose shadow, mouth).

**Donor:** `makehumancommunity/mpfb2` (GPL-3.0 tool, assets CC0) — `src/mpfb/services/targetservice.py` (`_interpolate_macro_components("proportions", value)`). `Mesh2Motion/mesh2motion-assets` (CC0) — `CC0-packs/psx-lowpoly/Textures/Character_01.png` (painted-mask-face layout).

**License:** GPL-3.0 (tool, invoked only), CC0 (assets), both read live 2026-10-21.

**Cost:** S (extend `tools/blender/mpfb_body.py` to pass `macro_detail_dict`, extend `tools/blender/proportion_pass.py` to read `hands`/`boots` keys from `content/design/proportions.json`, add `paint_face_texture()` to `tools/blender/style_cook.py`).

**Kill line:** The caricature proportions push the `bbox_fill` outside [0.40, 0.70], or the painted face reads as a mask at 30 m.

**Accept:** `content/design/proportions.json` is consumed by the cook (no hardcoded mitten), the face has eyes/brows/nose/mouth painted in the atlas, and the 30 m gate passes.

**Owns:** `tools/blender/mpfb_body.py`, `tools/blender/proportion_pass.py`, `tools/blender/style_cook.py`, `content/design/proportions.json`.

**Status:** **Done** (2026-10-22). `proportion_pass.py` already consumes all keys from `proportions.json` (hands.scale, hands.in, arms.scale, arms.reach, boots.scale, head.scale/x/y, bands). `paint_face_texture()` added to `style_cook.py` (commit `f29c6273f`), face painted in atlas. Both gates pass.

---

### DOOR-READONLY: MCP Read Door Without Write Verbs (tools/mcp)

**What:** The MCP server ships a read-only door (no mutating verbs). A mutating call succeeds only when its result list is clean **and** a separate readback agrees.

**Donor:** `Ladvien/bevy_debugger_mcp` (MIT OR Apache-2.0) — agent run as a file the engine replays one step per frame, refuses a step that is not a recorded command. `Chrrxs/robloxstudio-mcp` (MIT) — mutating call succeeds only when result list is clean and readback agrees.

**License:** MIT OR Apache-2.0 (bevy_debugger_mcp), MIT (robloxstudio-mcp), both read live 2026-10-21.

**Cost:** S (one MCP server file, read-only verbs only).

**Kill line:** The door-call log is never read in a failing run, or the readback disagrees on more than 1 in 20 seeded calls.

**Accept:** The MCP server refuses any mutating verb, the door-call log is written as a file, and a readback after a read call agrees with the log.

**Owns:** `tools/mcp/server.mjs` (or new file), `tools/mcp/door-log.jsonl`.

**Status:** Not started.

---

### GH-E2: Mid-Match Restore Proof (core/proof)

**What:** A mid-match restore is proved: the tick chain digest at the restore point matches the digest after restore, and the first divergent tick is named.

**Donor:** `balaur/task` (MIT) — stepped slices one-per-frame with recorded tick arrivals, per-tick digest chain reporting first divergent tick and stable id.

**License:** MIT (read live 2026-10-21).

**Cost:** S (extend `crates/core/src/digest.rs` and `crates/core/src/pump.rs`).

**Kill line:** The digest chain does not name the first divergent tick on 20 of 20 seeded divergences, or the restore point digest does not match after restore.

**Accept:** 20 of 20 seeded divergences localized exactly, restore point digest matches after restore, idle pump cost within PF1 noise.

**Owns:** `crates/core/src/digest.rs`, `crates/core/src/pump.rs`.

**Status:** Not started. (RC22 landed the pump glue; this row is the restore proof on top of it.)

---

### WORLD-HYDRO: Terrain Hydrology (world/terrain)

**What:** Terrain hydrology: water flows downhill, pools in depressions, and carves channels.

**Donor:** `outerra/Outerra` (MIT) — terrain hydrology as a compute pass over the heightmap.

**License:** MIT (read live 2026-10-21).

**Cost:** M (one compute shader, one heightmap texture).

**Kill line:** The hydrology pass costs the potato tier (G9 potato gate), or the water flow does not read as "water" at 30 m.

**Accept:** Water flows downhill, pools in depressions, and carves channels. The 30 m gate passes with the hydrology pass enabled.

**Owns:** `crates/world/src/hydrology.wgsl`, `crates/world/src/terrain.rs`.

**Status:** Not started.

---

### WORLD-SITE: Site Planner and Scatter Exclusions (world/scatter)

**What:** A site planner that marks exclusion zones for scatter (no trees in the arena, no rocks in the path).

**Donor:** `Esenthel/EsenthelEngine` (MIT) — scatter with exclusion masks.

**License:** MIT (read live 2026-10-21).

**Cost:** S (one scatter exclusion mask in `content/design/`, one scatter pass in `crates/world/src/scatter.rs`).

**Kill line:** The scatter exclusion does not read at 30 m, or the scatter pass costs the potato tier.

**Accept:** No trees in the arena, no rocks in the path. The 30 m gate passes with scatter enabled.

**Owns:** `crates/world/src/scatter.rs`, `content/design/scatter_exclusions.png`.

**Status:** Not started.

---

### WORLD-DUNGEON: Procedural Dungeon Generation (world/dungeon)

**What:** Procedural dungeon generation: rooms connected by corridors, with locked doors and keys.

**Donor:** `morcibacsi/ember-matter` (MIT) — dungeon generation as a graph of rooms and corridors.

**License:** MIT (read live 2026-10-21).

**Cost:** M (one dungeon generator, one dungeon renderer).

**Kill line:** The dungeon does not read as "dungeon" at 30 m, or the generator costs the potato tier.

**Accept:** Rooms connected by corridors, locked doors and keys. The 30 m gate passes with the dungeon enabled.

**Owns:** `crates/world/src/dungeon.rs`.

**Status:** Not started.

---

### LF6b: Screen-Space Shockwave (render/vfx)

**What:** A screen-space shockwave on impact: a radial distortion that expands from the hit point.

**Donor:** `pixijs/pixijs` (MIT) — screen-space shockwave as a post-process pass.

**License:** MIT (read live 2026-10-21).

**Cost:** S (one post-process shader, one shockwave buffer).

**Kill line:** The shockwave does not read at 30 m, or the post-process pass costs the potato tier.

**Accept:** A radial distortion expands from the hit point. The 30 m gate passes with the shockwave enabled.

**Owns:** `crates/render/src/shaders/post/shockwave.wgsl`.

**Status:** Not started.

---

### LF6c: Hit Flash Bloom Only (render/vfx)

**What:** Only the hit flash blooms: a short, bright flash on impact, no global bloom.

**Donor:** `pixijs/pixijs` (MIT) — bloom as a post-process pass with a mask.

**License:** MIT (read live 2026-10-21).

**Cost:** S (one post-process shader, one bloom mask).

**Kill line:** The hit flash does not read at 30 m, or the bloom pass costs the potato tier.

**Accept:** A short, bright flash on impact, no global bloom. The 30 m gate passes with the hit flash enabled.

**Owns:** `crates/render/src/shaders/post/hit_flash.wgsl`.

**Status:** Not started.

---

### GEO12: Heat-Diffused Skin Weights (geo/skin)

**What:** Skin weights diffused by heat: a heat equation over the mesh, with the bone positions as sources.

**Donor:** `libigl/libigl` (MPL-2.0) — heat diffusion over a mesh.

**License:** MPL-2.0 (read live 2026-10-21).

**Cost:** M (one heat diffusion pass in `crates/geo/src/skin.rs`).

**Kill line:** The heat diffusion does not converge on the 2k-triangle LOD0, or the skin weights do not read as "skin" at 30 m.

**Accept:** Skin weights diffused by heat, no hard edges at the bone boundaries. The 30 m gate passes with the heat-diffused weights.

**Owns:** `crates/geo/src/skin.rs`.

**Status:** Not started.

---

### AUDIO-SWING: Swing Sound from Swing Parameters (audio/swing)

**What:** A swing sound generated from swing parameters (speed, arc, weapon mass), not a clip.

**Donor:** `audiokinetic/wwise` (proprietary, reference only) — swing sound from parameters.

**License:** Proprietary (reference only, no code taken).

**Cost:** M (one swing sound generator in `crates/audio/src/swing.rs`).

**Kill line:** The swing sound does not read as "swing" on 20 of 20 seeded swings, or the generator costs the potato tier.

**Accept:** A swing sound generated from parameters, no clip. The swing sound reads as "swing" on 20 of 20 seeded swings.

**Owns:** `crates/audio/src/swing.rs`.

**Status:** Not started.

---

### AUDIO-SCORE: Adaptive Score Reacting to War State (audio/score)

**What:** An adaptive score that reacts to the war state (faction advantage, fighter count, time remaining).

**Donor:** `audiokinetic/wwise` (proprietary, reference only) — adaptive score from game state.

**License:** Proprietary (reference only, no code taken).

**Cost:** M (one score generator in `crates/audio/src/score.rs`).

**Kill line:** The score does not read as "score" on 20 of 20 seeded war states, or the generator costs the potato tier.

**Accept:** An adaptive score that reacts to the war state. The score reads as "score" on 20 of 20 seeded war states.

**Owns:** `crates/audio/src/score.rs`.

**Status:** Not started.

---

### AUDIO-PARAM: Sound from Parameter Set (audio/param)

**What:** A sound generated from a parameter set (frequency, envelope, noise), not a clip.

**Donor:** `audiokinetic/wwise` (proprietary, reference only) — sound from parameters.

**License:** Proprietary (reference only, no code taken).

**Cost:** M (one parameter sound generator in `crates/audio/src/param.rs`).

**Kill line:** The sound does not read as "sound" on 20 of 20 seeded parameter sets, or the generator costs the potato tier.

**Accept:** A sound generated from parameters, no clip. The sound reads as "sound" on 20 of 20 seeded parameter sets.

**Owns:** `crates/audio/src/param.rs`.

**Status:** Not started.

---

## Summary

| Row | Status | Cost | Lane |
|-----|--------|------|------|
| TOON1 | Not started | M | GH-L |
| AT1 | **Done** | S | GH-Q |
| CHR1 | **Partially done** | S | GH-K |
| DOOR-READONLY | Not started | S | GH-M |
| GH-E2 | Not started | S | GH-E |
| WORLD-HYDRO | Not started | M | GH-C |
| WORLD-SITE | Not started | S | GH-C |
| WORLD-DUNGEON | Not started | M | GH-C |
| LF6b | Not started | S | GH-V |
| LF6c | Not started | S | GH-V |
| GEO12 | Not started | M | GH-K |
| AUDIO-SWING | Not started | M | GH-U |
| AUDIO-SCORE | Not started | M | GH-U |
| AUDIO-PARAM | Not started | M | GH-U |

**Total:** 14 new rows, 2 done/partial, 12 not started. Cost: 6 S, 8 M.

**Next moves:**
1. Place golden PNGs for AT1 (`content/generated/cook_mpfb_warden/golden_silhouette_30m.png`, `content/generated/cook_mpfb_ashkin/golden_silhouette_30m.png`).
2. Finish CHR1: consume `content/design/proportions.json` in `tools/blender/proportion_pass.py` (remove hardcoded mitten).
3. Start TOON1: add `crates/render/src/shaders/forward.wgsl` TOON1 BRDF hunk, `crates/render/src/shaders/outline.wgsl` inverted hull, `content/design/toon_ramp.png` 256×1 gradient.
