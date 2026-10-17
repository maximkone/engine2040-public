# Render lighting: per-feature checks

Shared law is in `SKILL.md`. Each check says what to ask and what to flag. A pass needs a test, a capture or an ms number per item, never a promise. Paths are under `crates/render/src` unless a crate is named.

## 1. Shadow maps and light types

Source: Lengyel Vol 2 ch. 8. Depth bias math is in skill `render-pipeline-math` check 8. Files: `shadow_cascade.rs`, `twin_shadow.rs`.

| # | Check | Flag |
|---|---|---|
| 1 | Pick the light type by shape: bulb is a point, cone is a spot, sun is an infinite light. | one type stretched to every job, like a point light posed as the sun |
| 2 | Every local light has a finite reach. Attenuation hits zero at a max radius, stays capped near zero distance, and eases out at the edge. | bare inverse square with no radius, dim over most of its range |
| 3 | Spot cone: inner and outer angles ease full to zero. A projected map clamps to a black border outside. | one hard cutoff that rings, or edge texels smeared inward |
| 4 | Sun is a constant direction and constant illuminance, no position falloff. | point-light distance math on the sun, so brightness shifts across the arena |
| 5 | Tighten each light with a tangent scissor. Solve the rect from tangent planes. Handle behind-camera and camera-inside-sphere. | center plus-minus radius projected: clips lit pixels or shades the whole view |
| 6 | Tighten depth with sphere z bounds. Mirror them for reverse-Z. Clamp to near and far. | full-range lighting with no depth bounds, or one formula for both depth directions |
| 7 | Spot shadows are a depth-only pass from the light. Compare projected z and average over several taps. | one binary tap per pixel: stair-stepped, shimmering edges |
| 8 | Point shadows use six 90-degree faces. Flip winding on mirrored faces. Clamp offsets to the chosen face. | offsets spill across cube edges and draw wrong-face stripes |
| 9 | Sun cascades: overlapping splits with fixed texel size, snapped centers and blended overlap samples. | one giant sun map, or unsnapped splits whose edges crawl as the camera moves |

## 2. Torch point lights

Live torches light the forward frame, never the tick. Source: Lengyel Vol 2 ch. 8.1.1. Files: `shadow_cluster/torch_cluster.rs`, `shadow_cluster.rs`, `shaders/forward.wgsl`.

| # | Check | Flag |
|---|---|---|
| 1 | Finite inputs: `torch_cluster.rs::TorchLight::validate` refuses non-finite pos, non-positive `range_m`, negative color or intensity with `LightError::Invalid`. | a bad light reaching assignment |
| 2 | Known sphere: each light carries `range_m` as its cutoff. `ClusterConfig::validate` (same file) bounds `width_px`, `height_px` and `depth_slices`. | a world-wide light with no cutoff |
| 3 | Falloff to zero: the forward shade in `forward.wgsl` scales by one falloff with zero at cutoff, clamped past the edge and capped near the center by a size. The light uniform is sized by `LIGHT_UNIFORM_BYTES` in `v1_gpu.rs`. | endless tail, or blow-up near the center |
| 4 | `intensity` changes output without touching `range_m`. | reach grows with brightness |
| 5 | Assignment walks cell-major, sorted, with `shadow_cluster.rs::CLUSTER_TILE_PX` = 16. It clamps to `max_per_cluster` and counts overflow. | hash-order walk, or silent drop |
| 6 | Over `shadow_cluster.rs::MAX_TORCH_LIGHTS` (256) it clamps with `LightError::BudgetExceeded` and a receipt. Bad input still fails. | failing on many torches, or a quiet clamp |
| 7 | Ambient base is separate. `ibl.rs::sh_uniform_bytes_into` packs 9 SH coefficients into `FORWARD_IBL_SH_UNIFORM_BYTES` (144). AO scales ambient only in `forward.wgsl`. | torches carrying the base, or AO darkening direct light |
| 8 | Tangents are handed through: `crates/assets/src/import_repair.rs::TangentMode` keeps or recomputes via `orthogonalize_tangent`. `crates/assets/src/mesh_import.rs::fallback_tangents` and `crates/assets/src/cooked.rs::validate_mesh` demand a finite `w` of 1 or -1. | a torch sliding on bad frames |
| 9 | Torch bytes stay out of `state_hash` (`docs/HASH-LAW.md`). Golden `d57b5c8ae3096614` unchanged x2. | light bytes in tick or replay |

Do: bound each torch by range, ease falloff to zero, assign per tile in fixed order, prove on-vs-off pixels plus hash unchanged. Don't: walk lights in hash order or mix reach with brightness.

## 3. IBL, normal maps, gloss maps

Source: Gregory GEA ch. 10.3.1. Files: `ibl.rs`, `baked_gi.rs`, `gi.rs`, `shaders/forward.wgsl`, `present/gpu_present/presenter.rs`.

| # | Check | Flag |
|---|---|---|
| 1 | Normal map decode: `forward.wgsl` decodes with bias and unit length, and rebuilds z when only xy is stored. | raw RGB used as a vector, or `fallback_tangents` skipped so mirrored UVs invert |
| 2 | Tangent handedness: `tangent.w` stays +-1 from `recompute_tangents` (`crates/assets/src/import_repair.rs`) through `crates/assets/src/cooked.rs` to the shader. | w dropped or averaged: mirrored islands wash out, seams show |
| 3 | Gloss per texel: k_s and alpha ride per texel through the material map sample, not one material scalar. | a flat ambient hides gloss, so wet and dry sand read the same |
| 4 | Env cube: `ibl.rs::prefilter_cube_desc` builds a cube at infinity, sampled by the ray reflected about the normal. | a spherical map near the poles smears one texel |
| 5 | Diffuse from SH: `ibl.rs::shade_ibl` takes diffuse from `gi.rs::sh_irradiance` SH, divided by pi for non-metals. | ambient added after tone map, or a metal keeps diffuse |
| 6 | Split-sum specular: pick the prefiltered level by roughness. Add the `brdf_integrate` scale and bias over `BRDF_LUT_SIZE` (64). | one mip for all roughness: rough and mirror match |
| 7 | Bake inputs pinned: cook from `content/arena/hdri/*.hdr` with fixed roughness steps and `IBL_CUBE_SIZE` (64). Bytes x2 equal. | a live HDRI swap or drifting roughness steps |
| 8 | Probe and IBL agree: `baked_gi.rs::fighter_irradiance` agrees with `ibl.rs::eval_sh` on the same normal. | the probe path and the `ibl.rs::FORWARD_IBL_INPUTS` path diverge: fighters pop |
| 9 | Live bind proves pixels: `ibl_prefilter`, `ibl_brdf` and `ibl_sh` (`FORWARD_IBL_INPUTS`) are bound in the forward group built by `Presenter::forward_bind`. On vs off is above the epsilon. | the bake exists but the forward shade does not read it, so frames are identical |

The old flat `AMBIENT_FLOOR` constant is gone (see the comment in `twin_shadow.rs`). Ambient now comes from the baked IBL.

## 4. Sky bake, fog feed, exposure

Source: Game Engine Gems 1 ch. 15. Files: `sky.rs`, `fog.rs`, `present/fog_fwd.rs`, `arena_day.rs`, `shaders/forward.wgsl`.

| # | Check | Flag |
|---|---|---|
| 1 | Sun direction valid: `sky.rs::SkyParams::validate` requires a finite nonzero `sun_dir` via `validate_frame_args`. | zero or non-finite sun, or north and up swapped so the sun rises west |
| 2 | Air pinned: `turbidity` inside `SKY_TURBIDITY_MIN..=SKY_TURBIDITY_MAX` (1.5 to 10.0). `SkyParams::midday` uses 2.0. | clean or hazy air outside the range (a test uses 20.0 as the bad case) |
| 3 | Small cached bake: `bake_sky` builds `SKY_CAPTURE_W x SKY_CAPTURE_H` (64x32), under `SKY_CAPTURE_MAX_DIM` (256). Cached until time moves. Inside `SKY_BUDGET_MS` (0.4). | full-res or per-frame rebake |
| 4 | Bytes identical: `SkyCapture::to_bytes` matches x2. Row-major order, f64 accumulation in `mean_radiance`. | `HashMap` order or an f32 sum that drifts |
| 5 | Fog from horizon: `sky.rs::fog_color` averages the equator rows. `look_fog_color` feeds `fog.rs::FogParams::from_sky_capture`. | zenith or ground texels tinting fog, or fog on a constant |
| 6 | Live sky sampled: `fs_sky` in `forward.wgsl` samples the sky-view texture with `fog_factor_infinite`. It is bound through `FogFwd::day` (`present/fog_fwd.rs`, called in `presenter.rs`). | a bake on disk while `fs_sky` draws the analytic ramp only |
| 7 | Sky and fog agree: the fully fogged composite is within `fog.rs::FOG_SKY_RAMP_TOL` (0.02) of the fog color. Sky on vs off is above the epsilon. | distant sand cut out against the sky, or a probe-passing sky with identical pixels |
| 8 | Exposure from EV100: `ev100_to_exposure` carries `exposure_ev100` (`arena_day.rs::ARENA_DAY_EXPOSURE_EV100` = 12.0) to fog and IBL. Tone map once in post. | exposure baked into sky texels, or a second tone map in `fs_sky` |
| 9 | Ground sane and joined: `ground_albedo` in 0..=1 (`arena_day.rs::ARENA_DAY_GROUND_ALBEDO` = 0.42, 0.32, 0.20). `spawn_look_bake` is joined before the bind. | 100% white sand that blows out, or a frame that reads the fallback bake |

## 5. GTAO ambient occlusion

Live GTAO darkens creases from depth, not a constant. Source: Game Engine Gems 1 ch. 18. Files: `gtao.rs`, `shaders/blit.wgsl`, `crates/game/src/bin/arena.rs`.

| # | Check | Flag |
|---|---|---|
| 1 | Depth source: `gtao.rs::GtaoAo` reads the stored forward depth. | depth stored once and never read, or a new prepass with no caller |
| 2 | Half-res work: AO is built at half res (`GTAO_HALF_DIV` = 2, `GTAO_HALF_PASS`) and lifted by `GTAO_UPSAMPLE_PASS`. | full-res AO with no ms, or a transient above `GTAO_HALF_TRANSIENT_BUDGET_BYTES` (4.2 MB) |
| 3 | Tap pattern: `GTAO_SLICES` (4) and `GTAO_TAPS` (3) set slice directions and steps. | one slice or one tap with no on-vs-off gain, or a step that ignores `GtaoConfig` |
| 4 | Range: `radius_px` in `(0, GTAO_RADIUS_MAX_PX]` (64) and `falloff_m` in `(0, GTAO_FALLOFF_MAX_M]` (32). | a huge radius that flattens the frame, or a falloff that never reaches the wall base |
| 5 | Blit multiply: `blit.wgsl` clamps `ao_tex` to 0..1 and darkens ambient only. | `ao_tex` stuck at 1.0, or AO darkening direct sun |
| 6 | Live, not probes: AO runs on live frames, not only under `--look-probes` (`crates/game/src/bin/arena.rs::LOOK_PROBES_FLAG`). | live `ao_tex` stays constant while probes look fine |
| 7 | Budget: live cost is at or under `GTAO_BUDGET_MS` (0.8), by `ms <= GTAO_BUDGET_MS`. | no live ms number, or cost above budget with no cut |
| 8 | Stable output: the same depth gives the same AO twice within `GTAO_GPU_PARITY_TOL` (0.002). | the golden hash moves with AO on, or open sand shifts over 2% on vs off |

Ask for feet and wall bases darker on vs off, open sand within 2%, and live ms at or under budget. A constant `ao_tex` or a probes-only run is not done.

## 6. Deferred sun tiles (not verified live)

Source: Game Engine Gems 2 ch. 4. Mark any "live" statement here "unverified if live" until a capture proves it. Files: `crates/render/src`, grep for `deferred` and `tile`.

| # | Check | Flag |
|---|---|---|
| 1 | Tile size pinned: the frame splits into small fixed tiles. Classify cost is measured against shade saved. | fullscreen shade, or tiny tiles that cost more than they save |
| 2 | Sky fast path: sky tiles copy color with no light math, keyed by a stable sky id of zero. | sky paying for sun or shadow taps |
| 3 | Facing test: a tile skips sun diffuse and spec when normals face away from the sun. | backfaces lit as if facing the sun |
| 4 | Solid shadow skip: fully shadowed tiles skip sun and shadow taps. | dark tiles still running the sun shader |
| 5 | Soft edge only: multi-tap filtering only on shadow-edge tiles, one tap elsewhere. | a full-screen soft filter on every pixel |
| 6 | Fade at range end: shadow fades to unshadowed by depth near the draw-distance end. | a hard pop when geometry leaves range |
| 7 | Scatter by depth: scattering only past a start distance read from depth. | near tiles paying scatter they cannot show |
| 8 | Edge fragments: edge tiles shade all fragments and compare normals to catch intersections. | single-sample edges that shimmer or miss cuts |
| 9 | Few variants, merged tiles: one source per needed id, dead combos folded by table, same-id tiles merged into fewer quads. | dozens of hand shaders, or one draw per tile |

Prove ms saved plus pixels unchanged. Classify once per tile and run the cheapest shade that tile needs.

## 7. Wound paint maps

Live wounds paint per-fighter maps, never the tick. Source: Lengyel Vol 2 ch. 10.1. Files: `wound_map.rs` (the queue and store), `decals.rs` (the pool the design follows).

The model: one paint list per `char_id`. `WoundMaps::new(cap_bytes, paints_per_frame)`. `queue` and `queue_facing` take paints. `drain(frame)` returns at most `paints_per_frame` in `(tick, char_id)` order. Over `cap_bytes` the oldest character map is evicted and reported through `WoundMaps::last_evict`.

| # | Check | Flag |
|---|---|---|
| 1 | One map per fighter, keyed by `char_id`, like `decals.rs::DecalAtlas` keeps one tile set per decal. | one combined map that smears cuts across fighters |
| 2 | `queue` clamps `uv_rect` to 0..1 and drops empties. | rects outside 0..1 painting outside the map |
| 3 | `queue_facing` drops paints on backfaces, like `decals.rs::depth_reject` drops lifts over `DEPTH_REJECT_M` (0.15). | paints on hidden skin that never show |
| 4 | Seam fill: the bind step dilates each drained rect by `wound_map.rs::DILATE_PX` (1). The queue only records the constant and orders rects. | cracks at UV seams with no dilate step at the bind |
| 5 | Capped drain: at most `paints_per_frame`, in `(tick, char_id)` order. | uncapped drain, or a hash-order walk |
| 6 | Oldest evicted: over `cap_bytes`, evict the oldest character map and report it (`last_evict`). | newest dropped, or silent loss |
| 7 | Same bytes twice: the same queue gives the same drain bytes. | map iteration that drifts run to run |
| 8 | Wound bytes stay out of `state_hash` (`docs/HASH-LAW.md`). Wall clock only in receipts. | wound bytes in tick or replay |

`wound_map.rs` is std-only: no GPU, no wall clock. The GPU projection and atlas bind are not part of it yet.
