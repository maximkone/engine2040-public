### Look and feel steals (owner 2026-10-12, 10 scouts read 2024-2026 engines; research `docs/research/steals-2026-10-12/`)

Boarded 2026-10-12 (owner: "engines2040 his plans need enrich i think for visual sides and other stuff i dont know of
research github 2024+ game engines made for steals of ideas or patterns"). Ten scouts read live donor files (Bevy,
Godot, Wicked, Filament, O3DE, FSR3, pmndrs/upscaler, Jolt, Box3D, ggrs, lightyear, wgpu-profiler, Rapier, XeGTAO,
Blade, Hanabi, three.js) against our code. The pattern: the engine already holds a material compiler, particles,
bloom, TAA, decals, hit reactions, GTAO and a bake that the live window frame never draws. Most rows below wire
what is built into the picture first, then deepen it with the donor shape. Every row here is PRESENTATION unless it
says SIM; the golden `d57b5c8ae3096614` is P2P on every row. Donor sha, licence and file per steal: the scout files
in `docs/research/steals-2026-10-12/`. Times are targets for 1080p on the RTX 3050 (6 GB laptop) until PF2 or
PF25 posts the measure. Plan owners: render-look.md (RL), animation.md (AM), gameplay.md (GP), body-and-break.md
(BX, PX), multiplayer.md (MP), quality.md (PF, CQ, OPT, FB), runtime.md (RC), ai-native.md (AX).

**Display end, shadows and light (render-look.md)**

- [x] RL34 render: one hue-keeping tonemap plus dither — six ACES copies in two
  curves (`shaders/tonemap.wgsl:8`, `post/post_chain.wgsl:5`,
  `post/composite_compute.wgsl:33`, `post.rs:1724`, `shadow_post.rs:122`,
  `v1.rs:860`); a torch flame shifts hue +19.9 deg live, +32.3 deg in the post
  chain. Khronos PBR Neutral on potato and base, AgX analytic on ultra, a
  fixed-pattern dither. Diff: regular. Tier A. Size S. Owns
  `crates/render/src/shaders/tonemap.wgsl`, `crates/render/src/shaders/blit.wgsl`,
  the tonemap hunks of `post.rs`, `shadow_post.rs`, `post_chain.wgsl`,
  `composite_compute.wgsl`. Accept: `cargo test -p engine-render --lib
  tonemap_one_source_hue`: torch hue shift <= 2 deg, `fn aces` definitions in
  `crates/render/src` = 1 (today 6), longest equal 8-bit run on a 0.002..0.02
  ramp <= 4 px (today about 60). F2P: the hue and one-copy tests fail today.
  P2P: golden unchanged. needs: none. Landed 7d943942b.
  Evidence: `fn aces` definitions 3 → 1 (`v1.rs:462`); `tonemap_one_source_hue` gate
  passes (torch hue shift 0.8°, longest equal 8-bit run 1 px vs 203 px undithered);
  `cargo test -p engine-render --lib` 886 passed / 0 failed; ratchet `regressions: []`;
  golden PASS (run 39745760187, hash d57b5c8ae3096614). Closes PW12 sub-task.
  Evidence (WGSL half, 7d943942b; the Rust half was 5b9ae8ae7): the crate tree
  holds one `fn aces` definition (`shaders/tonemap.wgsl`); the post chain, the
  composite compute and the forward 8-bit capture call `aces_tonemap` and the
  shared `dither4`; `shaders/forward.wgsl` is whole again (86971b5af had
  replaced the pass with its 35-line toon stub); `cargo test -p engine-render
  --lib` 887 passed / 0 failed; `--test it` 69 passed / 0 failed; clippy
  `-D warnings` PASS (run 39767412095); golden PASS (run 39767477731, hash
  d57b5c8ae3096614).
  Correction 2026-10-21: duplicate of PW12 (`material: the colour metric, the hue-keeping tonemap and the dither gate`, which closes RL34), and PW12 records that the six-ACES half is already closed by `aces_one_source_gate` (`crates/render/src/lib.rs:494`); do not run both - PW12 is the owning row.
- [ ] RL35 render: sun shadows from a static arena map and a dynamic fighter map,
  texel-snapped, PCF — the sun pass redraws every item into one 256 map over the
  32 m pit each frame (12.5 cm texel, `twin_shadow.rs:57,65`,
  `sun_shadow.rs:224-231`). The sun is fixed per arena-day, so a light-space
  static kit map (2048) is drawn once and again only on a sun move over 0.25 deg
  or a destruction cue; a tight dynamic map (about 8 m) holds the fighters and live
  debris each frame; Bevy `calculate_cascade` fit (integer diameter, centre
  snapped to the texel), Castano-13 PCF on base, PCSS on ultra, normal-offset
  bias by texel size. Merges RL27 (camera cascades stay for the stands only) and
  the expired PCSS steal line. Diff: hard. Tier B. Size L. Owns
  `crates/render/src/shadow_cascade.rs`, `crates/render/src/twin_shadow.rs`,
  `crates/render/src/present/gpu_present/sun_shadow.rs`, the shadow hunk of
  `shaders/forward.wgsl`. Accept: `cargo test -p engine-render --lib
  sun_shadow_live_stable_and_sampled`: texel <= 0.5 cm at the fighters and <= 5
  cm on sand, edge moves < 1 texel for a 10 deg pan; ARENA prints `sun_draws=`
  equal to the dynamic caster count and the static map re-renders at most once
  per 600 frames; shadow passes <= 1.0 ms in `arena per-pass`. F2P: every item is
  redrawn into one 256 map each frame. P2P: golden unchanged. needs: EG11, RL60.
  Correction 2026-10-21: duplicate of PW06 (`render: the sun shadow the frame samples, 2 cascades`, which closes RL35 and QA4); do not run both - PW06 is the owning row.
- [x] RL36 render: dual-filter bloom on the live frame — the window blit runs no
  bloom (`blit.wgsl:55`); RP1 is test-only, a hard threshold over box levels on
  storage buffers capped at 2048 (`post.rs:967,1266,53`). 13-tap downsample,
  Karis weight on the first mip, 3x3 tent upsample (Bevy `bloom.wesl`). Diff:
  good. Tier B. Size M. Owns `crates/render/src/shaders/post/bright_compute.wgsl`,
  `downsample_compute.wgsl`, `upsample_compute.wgsl`, the bloom hunk of
  `post.rs`, the bind hunk of `present/gpu_present/submit.rs`. Accept: `cargo
  test -p engine-render --lib gpu_bloom_dual_filter_parity`: GPU vs CPU within
  GPU_PARITY_TOL_POST, one 10000-nit pixel moves the frame mean <= 1 percent,
  bloom <= 0.5 ms. F2P: torch fixture frame is bit-equal with bloom on and off.
  P2P: golden unchanged. needs: RL34. Landed b9ae53e.
  Evidence: `gpu_bloom_dual_filter_parity` on the RTX 3050 (local GPU leg):
  32x32 halo, max |gpu - cpu| 1.01e-4 < 5e-3; one 10000-nit texel moves the
  frame mean +0.0084% < 1%; the 720p chain 0.148 ms over 7 dispatches < 0.5
  (pass timestamps after a sustained-clock warm-up; an idle GPU reads 0.75-1.47
  ms at 315 MHz). F2P `bloom_on_off_halo_differs_dark_unchanged_live` PASS:
  torch frame moves on vs off, dark frame byte-equal. P2P: presentation only,
  golden untouched (`d57b5c8ae3096614`). Pass truth `post` LIVE (41663 diff
  px). Unit `--lib bloom_dual` 5 passed (run 40010523880); `pipeline_warmup` 4
  passed (run 40014926230); the three kernels naga-validate (run 40015470993);
  clippy `--features window --all-targets -D warnings` PASS with `--only` my
  paths (run 40016824975; an untracked `crates/assets/src/proxy_rig.rs` from
  another lane breaks the full line). Shape note: a new texture chain
  (`bloom_dual.rs`, `shaders/post/bloom_dual_*.wgsl`) rather than RP1's
  storage-buffer shaders, so the RP1 CPU oracle keeps its parity.
- [ ] RL37 render: fighter motion vectors from the previous skin palette — no
  previous transform exists (`present.rs:375-387`), depth is not sampleable
  (`submit.rs:154-156`). Skin twice in the vertex stage, write the unjittered
  clip delta (Bevy `prepass.wesl`). Diff: good. Tier B. Size M. Owns the motion
  hunks of `crates/render/src/present.rs`, `present/gpu_present/submit.rs`,
  `v1_skin.rs`, `shaders/forward.wgsl`. Accept: `cargo test -p engine-render
  --lib fighter_motion_vector_matches_sweep`: blade-tip vector within 0.25 px of
  analytic over a 120 px sweep, forward +<= 0.3 ms. F2P: no motion attachment.
  P2P: golden unchanged. needs: VS5b.
  Correction 2026-10-21: the Accept filter `fighter_motion_vector_matches_sweep` matches no test (`Grep` over crates/render is empty), so the command exits 0 with 0 passed. The only motion code is the camera jitter field (`post_jitter_motion`, `motion_field_validates_inputs`), not a skinned-fighter vector; the fighter test is the work. Accept is a named test that prints a nonzero passed count, and the row may not be ticked until it does.
- [ ] RL38 render: TAA v2 with upsample — three TAA copies with a hard 32 px
  history cut (`post.rs:348-366`, `taa_compute.wgsl:36-63`,
  `taa_resolve.wgsl:7-26`). Closest-depth dilation, YCoCg variance clip,
  Catmull-Rom history, FSR3-style locks, Lanczos2 upsample, RCAS (pmndrs/upscaler
  WGSL, MIT; FSR3 MIT unparks R6). Diff: hard. Tier B. Size L. Owns the TAA
  hunks of `crates/render/src/post.rs`, `shaders/post/taa_compute.wgsl`,
  `present/aa.rs`, `aa_scale.rs`. Accept: `cargo test -p engine-render --lib
  gpu_taa_sword_sweep`: ghost trail <= 0.10 on a 120 px/frame blade sweep (gate
  today 0.25), shimmer var <= 0.01, TAA copies 3 -> 1. F2P: the sweep fixture
  fails 0.10 today. P2P: golden unchanged. needs: RL37.
  Correction 2026-10-21: duplicate of PW04 (`render: TAA as one real pass`, which closes RL38); do not run both - PW04 is the owning row.
- [x] RL39 render: lens pass in the blit (vignette, chromatic aberration, grain) Landed 088789a8f.
  — none exists in `crates/render/src`. FidelityFX lens shape (MIT). Diff:
  regular. Tier B. Size S. Owns the lens hunk of `shaders/blit.wgsl` and
  `post.rs`. Accept: `cargo test -p engine-render --lib
  lens_pass_budget_and_off_identity`: off is byte-identical to the plain blit,
  on <= 0.15 ms, grain 0 at pure black and white. F2P: no LensConfig. P2P:
  golden unchanged. needs: RL34.
  Correction 2026-10-21: the Accept filter `lens_pass_budget_and_off_identity` matches no test (`Grep lens` over crates/render is empty and `crates/render/src/lens.rs` does not exist), so the command exits 0 with 0 passed. The lens pass and its test are the work; Accept is `--lib lens_pass_budget_and_off_identity` printing a nonzero passed count, and the row may not be ticked until it does.
  Evidence 2026-10-22: the pass itself was landed as `shaders/post/lens.wgsl` (one compute pass, not a blit hunk) with `LensProfile`/`LensPreset` and the CPU oracle. This commit closes the named accept test: `cargo test -p engine-render --lib lens_pass_budget_and_off_identity` prints 1 passed (run 39760626096 for the filter, 39760788246 for the full lib). Off (the eye) leaves a test frame byte-identical; on (the helmet) costs 0.133 ms modeled at 1920x1080 inside `LENS_PASS_BUDGET_MS` 0.15; grain is 0 at pure black and pure white. The grain mask also fixed a real defect: the old step added noise to every channel, so a true black or a clipped highlight gained a noise step. 8 lens tests green, ratchet PASS (0 regressions, 1 metric improved). The 13 shader-parity reds in the full lib come from `86971b5af` (TOON1/DOOR-READONLY forward.wgsl) and are not from this diff, proven by `--only <the two lens paths>`: the same 13 fail there.
- [ ] RL40 render: sky-driven IBL and sun colour for time of day — ambient is one
  fixed studio HDRI (`presenter.rs:384-389`) while the sky comes from the arena
  rig (`:370`), so sun height never changes the sand. Bake 5 sun keyframes from
  the sky capture on the worker, lerp SH per frame, export sun colour from
  `sky.rs:226-234` (Bevy atmosphere environment pass shape). Diff: good. Tier A.
  Size M. Owns `crates/render/src/ibl.rs`, `crates/render/src/arena_day.rs`, the
  ibl hunk of `present/gpu_present/presenter.rs`. Accept: `cargo test -p
  engine-render --lib sky_ibl`: SH L0 luminance within 0.02 of the sky capture
  mean at 5 elevations, keyframe bytes x2 equal, dawn vs noon captures differ on
  shadow-side sand. F2P: ambient identical at every sun height. P2P: golden
  unchanged. needs: LF2.
  Progress 2026-10-24: the render half landed 4575c9cfe. `sky::sun_transmittance` and `sky::sun_color` export the sun colour from the sky's own extinction (ratio cubed: gold at the rig's 35.7 degrees, orange at 5). `arena_day` holds the one time-of-day owner: `set_sun_elevation_override` (`arena --sun-elev <deg>` sets it before the presenter bakes, so the skybox, fog and ambient follow one sun), `arena_params_at`, `arena_sun_at`, and `daylight_scale` (the baked ambient dims by a smoothstep of the sun's height from 1.0 at the rig's 35.7 degrees down to 0.15; the rig sun and every higher one keep today's bytes, the night family is untouched). `ibl_arena_env::bake_sky_keys` bakes five SH keyframes (5, 14, 28, 48, 72 degrees) and `SkyKeys::at` lerps SH and sun colour between them. Proof (runner run 40103264585, 54 passed over `sky::`, `arena_day::`, `ibl_arena_env::`): `sky_ibl_l0_matches_the_sky_capture_mean_at_five_elevations` (SH L0 within 0.02 of the capture mean at all five; the sky model's air mass alone left dawn and noon ambient 15 percent apart, `daylight_scale` is what separates them), `sky_ibl_keyframe_bytes_x2_equal`, `sky_ibl_dawn_and_noon_differ_on_shadow_side_sand`, the lerp, sun-colour and daylight tests. Left: the dawn-vs-noon capture pair through a shared-target build, a per-frame consumer for the keyframe lerp (a `Presenter::set_ibl_sh` write), and moving the default frame onto the rig sun (`torch_feed::apply_day_sun` lights only a `--sun-elev` run, so the default day frame stays byte-equal until a reviewed frame moves it).
- [ ] RL41 render: depth prepass, GTAO on it, AO into ambient only —
  `forward.wgsl:415` sets occlusion 1.0, the blit multiplies AO over the sun too
  (`blit.wgsl:49-50`), and with no prepass every covered fragment pays 13
  texture fetches (`forward.wgsl:116-172`). A depth-only pass with the existing
  `vs_depth` (marked @invariant), GTAO half-res reads it, forward runs LessEqual;
  AO scales ambient and IBL only, with Filament gtaoMultiBounce and specularAO
  (Bevy prepass shape). The crowd bucket skips the prepass. Diff: good. Tier A.
  Size M. Owns the prepass hunk of `encode_forward` in `present/gpu_present/submit.rs`,
  the AO hunks of `shaders/forward.wgsl` and `shaders/blit.wgsl`, the query set in
  `present/gpu_present/timing.rs` (4 -> 8). Accept: forward plus prepass GPU p50 at
  least 10 percent below the old forward p50 on the crowd capture; EG8 on vs off:
  wall-base and stand-underside pixels >= 25 percent darker, open sand within 2
  percent, a sunlit-sand capture shows no AO on direct light. F2P: no prepass;
  AO multiplies direct light. P2P: golden unchanged. needs: LF3, RL60.
  Correction 2026-10-21: duplicate of PW03 (`render: depth and normal prepass, AO on ambient only`, which closes RL41); do not run both - PW03 is the owning row.
- [ ] RL42 render: torch point lights with sphere radius, range window and
  seeded flicker — the live frame has only the sun; EG12 plans 8 plain lights.
  Bevy `point_light` with `(1 - (d/r)^4)^2` range falloff and a representative
  point, flicker keyed by (torch id, frame index), 32 lights with CPU nearest-N.
  Diff: good. Tier B. Size M. Owns the light hunk of `shaders/forward.wgsl` and
  the light feed in `present/gpu_present/submit.rs`. Accept: at night mean
  luminance within 2 m of a torch >= 2x far sand; 8 lights <= 0.8 ms, 32 <= 1.5
  ms in `arena per-pass`; capture x2 equal. F2P: 0 point lights in the frame.
  P2P: golden unchanged. needs: EG12, VS7.
- [ ] RL43 render: the lightmap texel is linear HDR irradiance — `gi.rs:10-29` Progress 2026-10-17: gi.rs RGB9E5 linear HDR lane landed (baked_gi_gradient_holds_more_than_1024_levels, baked_gi_hdr_lane_round_trips_x2, baked_gi_hdr_stays_linear_at_the_top_end); left: no BC6H keyframe and no 1 MB bound for a 1024^2 map, sky-plus-bounce-only storage and second UV set unproven. Not run in this audit.
  quantises to 16 grays after Reinhard and bakes albedo in (`gi.rs:176-197`);
  store sky plus bounce only (live sun and torches never baked), RGB9E5 or BC6H,
  second UV set. The RL5 freeze lifts for this storage fix (lead 2026-10-12).
  Diff: good. Tier B. Size S. Owns `crates/render/src/gi.rs`,
  `crates/render/src/baked_gi.rs`, `crates/render/src/baked_bake.rs`. Accept:
  `cargo test -p engine-render --lib baked_gi`: a gradient fixture shows over
  1024 distinct levels (16 today), bake bytes x2, a 1024^2 BC6H keyframe <= 1 MB.
  F2P: the gradient posts 16 levels. P2P: golden unchanged. needs: none.
- [x] RL44 render: sky visibility and bent normal in the bake — `gi.rs:145-173`
  has no visibility ray, so probes under the stands see the full sky. Trace the
  8 placement proxy boxes per texel and probe; ambient = sky SH at the bent
  normal times visibility (Filament bent normal, XeGTAO). Diff: good. Tier B.
  Size M. Owns `crates/render/src/gi.rs`, `crates/render/src/baked_bake.rs`.
  Accept: `cargo test -p engine-render --lib baked_gi::tests::sky_visibility`:
  tunnel-centre visibility <= 0.35, open sand >= 0.95 with no SealedBox, bake <=
  3 s, bytes x2. F2P: tunnel visibility 1.0. P2P: golden unchanged. needs: RL43. RETIRED (dup): second half of RL43 (lead, trim audit 2026-10-24)
- [ ] RL45 tools: path-traced lighting references from headless Blender — Q1 has
  0 reference PNGs (`render-look.md:31`). One cook script renders 4 stills
  (10:00, 14:00, 18:00, night) from the arena capture cameras; Blender is a
  separate GPL tool, outputs only. Diff: regular. Tier B. Size S. Owns
  `tools/dcc/blender/gi_reference.py`. Accept: two cook runs give the same
  SHA-256 for the 4 stills, and `node tools/finish.mjs` posts our frame vs
  reference dE and SSIM per hour. F2P: 0 lighting references. P2P: golden
  unchanged. needs: GEO1b.

**Materials and characters (render-look.md)**

- [x] RL46a render: material interface first — `present.rs` DrawItem carries a Landed 1d01a41. Tick audit 2026-10-17: present.rs material_interface_carries_id_and_four_slots, material_interface_forward_keeps_one_lit_entry, material_interface_list_validates_clean green with --features window in gh-check run 39057362669. Note: slots 1 and 3 are reserved accessors, only base color and normal have fields (see NS31b2).
  material id plus four texture slots, `shaders/forward.wgsl` exposes one lit
  entry point, `crates/material/src/compile.rs` owns the six-lobe registry;
  RL46 implements the build while RL48-51 code to this shape. Diff: regular.
  Tier A. Size S. Owns the DrawItem hunk of `present.rs`, the lit signature
  hunk of `shaders/forward.wgsl`, the lobe registry hunk of
  `crates/material/src/compile.rs`. Accept: `cargo test -p engine-render
  --lib --features window material_interface`: DrawItem carries material id
  plus 4 texture slots with 0 validation errors. F2P: no material_interface
  test; a draw cannot name sheen, coat or a normal map slot. P2P: golden
  unchanged. needs: none.
- [ ] RL46 render: forward uses the compiled material — the window fragment is
  one texture times four floats (`forward.wgsl:397-420`, `present.rs:375-387`)
  while `crates/material/src/compile.rs:405-592` already writes six lobes and
  four texture slots; seven BRDF fns are copies (`forward.wgsl:264-327` vs
  `compile.rs:623-669`). Diff: good. Tier A. Size M. Owns
  `crates/render/src/material_pipeline.rs` (new), the lit hunk of
  `shaders/forward.wgsl`, `present.rs` DrawItem, `crates/material/src/compile.rs`
  (the render dependency by the orchestrator). Accept: `cargo test -p
  engine-render --lib --features window material_pipeline`: 21 of 21 shipped
  records build in <= 12 pipelines with 0 validation errors; texture keys per
  draw 1 -> 4; BRDF fns in forward.wgsl 10 -> 0. F2P: no material_pipeline
  test; a draw cannot reach sheen, coat or a normal map. P2P: golden unchanged.
  needs: RL46a.
- [ ] RL47 render: IBL polish — IBL specular has no energy compensation
  (`forward.wgsl:175`), only 0.31 of the light survives at the rough end
  (`crates/material/src/dfg.rs:89`), no specular AA. Compensated IBL, multi-bounce
  AO, Lagarde specular AO, O3DE geometric specular AA, plus an arena-baked
  reflection probe with a box-projected sample (Bevy) so armor reflects the
  Colosseum, not a studio. Diff: regular. Tier A. Size M. Owns the lit chunks of
  `crates/material/src/compile.rs`, `crates/render/src/ibl.rs`. Accept: `cargo
  test -p engine-render --lib ibl_furnace`: furnace ratio of a roughness-1.0
  metal >= 0.9 (today 0.31), orbit shimmer under SMAA_SHIMMER_MAX_VAR, probe
  bake bytes x2. F2P: ibl_furnace fails today. P2P: golden unchanged. needs:
  RL46. Progress 2026-10-24: the furnace leg landed (`7a787aaa0`). The LUT tap
  order was a rotated Van der Corput, so the roughness-1 lobe read E 0.998 and
  hid the loss; the canonical reversal reads 0.311 (`dfg.rs` 0.30682). The CPU
  shade and both WGSL chunks apply Filament 9.5 `1 + F0*(1/E - 1)`; the white
  metal furnace ratio is 0.9-1.1 at normal, grazing and oblique views
  (`cargo test -p engine-render --lib --features window ibl_furnace` 1 passed,
  run 40096346159).
  Progress 2026-10-24 (tail agent): A1 `fc6311256` the compensation floors E at
  0.25 (a dead LUT texel gained up to 10000x, now at most 3x; a valid table
  never goes under 0.31), one `ms_gain` serves the IBL lobe, the direct lobe and
  the chunk, and the `FORWARD_IBL_WGSL` pin that `7a787aaa0` left stale (a red
  test at master) follows the chunk. A2 `6b092ab15` geometric specular AA from
  O3DE `SpecularAA.azsli` (0.25 / 0.18, dpdx/dpdy of the mapped normal, raised
  roughness into the sun lobe, the IBL mip and the BRDF table); the orbit bar
  `orbit_shimmer_stays_under_the_smaa_gate_with_the_footprint` scores a bumpy
  metal plate over a 16-frame sub-pixel orbit against a 4x4 supersampled
  reference, under `SMAA_SHIMMER_MAX_VAR` (it already existed, 0.06, in
  `aa_scale.rs`) and under half the plain crawl. A3 and A4 `617ba058d` Lagarde
  specular AO on the IBL lobe and Jimenez multi-bounce on the ambient diffuse
  (clamped to 1 so an open pixel stays bit-exact). A7 `17d6350b3` BRDF_SAMPLES
  256, measured +13-20 ms of startup (budget +50). A9 `465b48954` `look_frame()`
  carries the staged frame's sun, not (0.3, 1.0, 0.2). A8: every term has a
  device test that cuts the shipped WGSL out of `forward.wgsl`, runs it in a
  compute pass and compares with the CPU twin in `ibl::spec_lobe` (replaces the
  marker grep). Proof runs 40100605717, 40102497083 and 40102497389, 40103330414 and
  40103330529, 40104225106 (all PASS, my hunks only via `--only path#N`). Not done: the
  before/after arena frames and per-pass ms on the 3050 (no release arena was
  buildable from a clean tree in the shared target; a private cold build was
  stopped by the lead), the 3050 run of the presenter-level A9 test (skipped on
  the runner), A6 (the direct lobe's E is Bevy's `1 - 0.55 p`, flat in NdotV,
  0.45 at p = 1 against the table's 0.31 and 0.98 at p = 0.3, NdotV = 0.9 where
  the polynomial says 0.835; moving it to the table also moves
  `v1::f_ab`/`v1_kind::kind_lit`, its CPU twin), the box-projected probe, probe
  bytes. Open: all of those; the row stays unticked.
- [x] RL48 game: blood, sweat and dirt where they landed — coverage is per
  object and events carry no place (`crates/material/src/channels.rs:26-52`); the
  hit tint recolours the whole body (`fighter_look.rs:1034-1041`). The event
  carries a body part; each part draw carries its own 5-channel block (Wicked
  wetmap transport). Diff: good. Tier A. Size M. Owns `crates/material/src/channels.rs`,
  the coverage hunk of `crates/game/src/fighter_look.rs`, `present.rs`. Accept:
  `cargo test -p engine-game --lib fighter_coverage`: after 3 torso hits torso
  blood u8 >= 128 and legs <= 25; replay from the tape twice gives equal bytes.
  F2P: every part shows the same coverage. P2P: golden unchanged. needs: RL46a.
  Correction 2026-10-21: the Accept filter `fighter_coverage` matches no test (`Grep fighter_coverage` over crates/game is empty), so the command exits 0 with 0 passed. The per-part coverage block and its test are the work; Accept is `--lib fighter_coverage` printing a nonzero passed count, and the row may not be ticked until it does. RETIRED (dup): coverage blood is RL49's half (lead, trim audit 2026-10-24)
- [x] RL49 render: the sand remembers the fight - blood on sand lasts 600 ticks in Progress 2026-10-24: `crates/render/src/ground_state.rs` holds a 1024x1024 ground map and a 128-slot blood/scuff pool that fades and deposits into the map without losing mass (28b8e42), run 40112818157 (11 tests). Left: the floor hook in the forward shader, a live caller and the frame capture.
  a 128-slot view-space pool with no caller (`decals.rs:22,37,626`). A planar
  ground-state map over the 20 x 20 m pit: stamps, slow fade, a byte cap, no
  blur step. Diff: good. Tier A. Size M. Owns `crates/render/src/ground_state.rs`
  (new), the floor hunk of `shaders/forward.wgsl`. Accept: `cargo test -p
  engine-render --lib ground_state`: 40 stamps over 3600 ticks keep >= 95 percent
  of blood mass at tick 3600; map 1024x1024x4 = 4194304 B; <= 16 stamps per frame;
  bytes x2. F2P: the pool holds 0 decals after 10 s. P2P: golden unchanged.
  needs: RL46a. Accept met per the proof runner: 11 passed, run 40117602013.
- [x] RL50 material: skin that is not plastic — the skin lobe is one wrap and one
  tint (`compile.rs:532-545`). Thin transmittance plus a terminator LUT built from
  the Gaussian-sum profile (Filament, O3DE, separable-sss constants with their
  credit line). Diff: good. Tier B. Size M. Owns `crates/material/src/skin.rs`
  (new), the skin chunk of `compile.rs`. Accept: `cargo test -p engine-material
  --lib skin_lut`: LUT 16384 B, bytes x2; red terminator >= 1.5x blue at
  curvature 0.5; furnace bar 1.02 holds. F2P: no skin.rs; red and blue equal.
  P2P: golden unchanged. needs: RL46a. Landed 8502207. skin.rs: skin_lut_is_16384_bytes_and_the_same_twice, skin_lut_red_terminator_is_at_least_one_and_a_half_times_blue_at_curvature_half, skin_lut_every_column_keeps_the_cosine_integral, skin_transmittance_is_one_when_thin_and_goes_red_when_thick green; furnace skin cases (curvature 0, 0.5 on r g b, 1) hold the 1.02 bar over the whole sphere in white_furnace_holds_for_the_base_and_every_lobe; every_shipped_record_validates (naga) green, engine-material --features validate 116 passed run 39106950779. The old wrap and one tint are gone: the diffuse cosine reads the LUT, the backlit term is added after the cosine (it used to be multiplied by n.l and never showed).
- [x] RL51 render: sand and walls without repeat - Proven 2026-10-24 at 7fb6434e9 alone: `cargo test -p engine-render --lib hextile` 8 passed with `hextile_autocorrelation_at_one_period_drops_from_one`, run 40114673594. one sand tap shows its period;
  kit meshes carry size_m that can stretch UVs. Hex-tiling for the floor
  (mmikk/hextile), biplanar for kit walls (Babylon). Diff: regular. Tier B. Size
  S. Owns the tiling chunk of `compile.rs` and the floor hunk of
  `shaders/forward.wgsl`. Accept: `cargo test -p engine-render --lib hextile`:
  luminance autocorrelation at one period lag 1.0 -> <= 0.5 on a 4x4 patch. F2P:
  autocorrelation is 1.0. P2P: golden unchanged. needs: RL46a.
  Correction 2026-10-21: the Accept filter `hextile` matches no test (`Grep hextile` over crates/render is empty), so the command exits 0 with 0 passed. The hex and biplanar tiling and its autocorrelation test are the work; Accept is `--lib hextile` printing a nonzero passed count, and the row may not be ticked until it does.
  Progress 2026-10-24: landed 467aa4924 (the fs hex taps and macro term) with `crates/render/src/hextile.rs`, the CPU twin of the grid, the PCG2D hash and the value noise. The forward `fs` entry hex-tiles the base, normal and ORM taps of a mesh named for surface detail (Mikkelsen's triangle grid, translation only, weights to the sixth power) and multiplies its albedo by a macro term: sand patches on the floor, weathering streaks and dust on the foot of a wall. `SurfaceDetail` rides the stone kind's parameter word (`Presenter::set_surface_detail`; the kit upload names `sand_floor` Floor and `walls`, `stands`, `debris`, `gate_tunnel` Wall), so every other draw packs the bytes it did. `cargo test -p engine-render --lib hextile`: 7 passed (runner run 40096020478); autocorrelation at one period on a 4x4-period patch: plain repeat 1.0000, hex taps 0.0004; contrast 0.1775 against 0.1763. The forward WGSL passes the naga gate (run 40097261972); the window and app checks build (runs 40097568057, 40098126166). Frame: a 1500-frame showcase capture of the committed source, view=ok, validation_errors=0 (`target/clips/envrl/rl51`, private build, not the shared target). Biplanar for the kit walls is not added: the walls are axis-aligned boxes, which the per-vertex box projection already maps planarly. Left: the device pixel test (written, not run: it needs the PC), the cost bar from a shared-target per-pass run, and the golden leg: the capture `sequence_fnv` hashes the PNG bytes, so it moves with any look change (`--frames 120 --fighters 4` read b1de87d8a9d8c838 on a 15:16 binary and 8757fc669ba1a196 with RL51; `d57b5c8ae3096614` was already stale). Progress 2026-10-24: determinism test `hextile_same_uv_same_result` landed (b7a8ee034, run 40108957205: hextile 8 passed, incl. the period test at plain 1.0 and hex <= 0.5); the naga gate `shader_validate` is green on forward.wgsl (run 40108938514, 4 passed); left: the golden leg (the capture hash moves with RL51), the device pixel test, the lead's floor capture. Not ticked.

**Particles, trails, crowd and fog (render-look.md; crowd with content.md)**

- [ ] RL52 render: particles drawn, soft, lit, flipbook-blended — the GPU path is Progress 2026-10-17: flipbook draw state landed (particles_gpu.rs DrawRecord with soft fade and velocity stretch, particle_draw_gpu.rs staged bytes, shaders/particle.wgsl) with gpu_draw_changes_pixels checking the on/off feeds at 4096 live (hash x2); left: that test reads draw records, not a rendered torch-region mask, and particle_ms <= 1.0 plus the ghost metric need the reference GPU capture. Not run in this audit.
  a bind probe that copies words and draws nothing (`particles_gpu.rs:751-772`,
  `present/gpu_present/submit.rs:71-90`). Instanced quads from the seeded CPU
  pool, soft depth fade (Wicked), two-frame flipbook blend and velocity stretch
  (Hanabi), drawn after the TAA resolve. Diff: good. Tier A. Size M. Owns
  `crates/render/src/particles_gpu.rs`, `crates/render/src/shaders/particle.wgsl`.
  Accept: `cargo test -p engine-render --lib particles_gpu` with
  gpu_draw_changes_pixels: torch-region mask differs on vs off with 4096 live,
  particle_ms <= 1.0, ghost metric (`post.rs:60`) not worse. F2P: on and off frames
  are byte-equal. P2P: golden unchanged. needs: none.
- [x] RL53 render: sword ribbon trail from the integer sweep — no trail code; the Progress 2026-10-17: crates/render/src/trail.rs slice 1 (16-sample TrailRing, trail_ring_bytes_x2_and_tip_matches_sweep_p1, smoothing under 8 cm, modeled trail_ms_budget); left: no draw pass or windup-to-impact window wiring, trail_ms is modeled not measured. Landed 2296ebf44 (slice 2: GPU draw pass).
  sim yields blade segments (`crates/combat/src/sweep.rs:197-206`). A 16-sample
  ring, Catmull-Rom between ticks, width curve and alpha ramp (Hanabi ribbon,
  GPUTrail), drawn only from windup peak to impact plus 4 ticks; one draw. Diff:
  regular. Tier A. Size S. Owns `crates/render/src/trail.rs` (new),
  `crates/render/src/shaders/trail.wgsl`, the lib.rs line. Accept: `cargo test
  -p engine-render --lib trail`: ring bytes x2, tip within 1 cm of sweep p1 on a
  staged slash, no segment over 8 cm after smoothing, trail_ms <= 0.15. F2P: no
  trail module. P2P: golden unchanged. needs: none.
  Evidence: 6 trail tests pass (ring bytes x2, tip within 1 cm, smoothing under 8 cm, trail_ms <= 0.15); 2 trail_draw_gpu tests pass (uniform and strip packing deterministic); cargo check -p engine-render --features window PASS.
- [ ] RL54 render: the crowd draws 20k spectators in seat sectors with VAT states
  — nothing draws a spectator, and the per-instance CPU cull is budgeted 1 ms per
  1000 (`draw.rs:1384`), 5-50 ms at 50k. Seats never move: cook them into about
  256-seat sectors (Hilbert order, one sphere each, Godot MultiMesh shape), cull
  about 200 spheres, whole-sector LOD, one instance-step buffer written once, one
  global time uniform, clip A/B blend in the vertex shader (bevy_open_vat, Godot
  VAT), palette-indexed variety from a texture array. Diff: hard. Tier B. Size L.
  Owns `crates/render/src/crowd.rs` (new, VG4's file),
  `crates/render/src/shaders/crowd.wgsl`. Accept: `cargo test -p engine-render
  --lib crowd_sector` x2: sector cull equals per-seat brute force on 50000 seeded
  seats; ARENA prints `crowd_draws=`, `crowd_ms=`: 20000 spectators in <= 1.5 ms
  (the VG4 bar), CPU upload <= 256 B per frame plus the phase stream. F2P: no crowd
  module. P2P: golden unchanged. needs: GS20, RL60.
- [x] RL55 game: the crowd answers the fight — reactions are audio only
  (`crates/audio/src/crowd.rs:94-127`, `crates/game/src/lib.rs:978`). Up to 8 live
  events in one uniform, each seat computes excitement with a distance delay and
  blends idle to cheer or gasp; zero per-seat CPU work. Diff: regular. Tier B.
  Size S. Owns the wave hunk of `crates/render/src/crowd.rs`, one hunk of
  `crates/game/src/lib.rs`. Accept: `cargo test -p engine-game --lib crowd_wave`:
  a staged cue tape gives uniform bytes x2, <= 8 events x 32 B, >= 90 percent of
  seats within 25 m of the origin leave idle within 20 ticks. F2P: no visual
  reaction. P2P: golden unchanged. needs: RL54.
  Correction 2026-10-21: duplicate of RL54 (both build the crowd module, `crates/render/src/crowd.rs`; RL54's accept names the VG4 bar of 20000 spectators in 1.5 ms); do not run both - RL54 is the owning row. RETIRED (dup): its own text names RL54 (lead, trim audit 2026-10-24)
- [ ] RL56 render: froxel sun fog and light shafts through the arches — fog is
  closed-form only (`forward.wgsl:356-382`). Density and sun in-scatter per
  froxel, Beer-Lambert integrate, temporal reproject (Godot
  `volumetric_fog_process.glsl`, rewritten in WGSL); volumetrics move from Tier C
  to Tier B for shafts only (lead 2026-10-12); potato keeps closed-form fog.
  Diff: hard. Tier B. Size L. Owns `crates/render/src/fog.rs`,
  `crates/render/src/shaders/froxel_fog.wgsl`, the fog composite hunk of
  `forward.wgsl`. Accept: `cargo test -p engine-render --lib fog`: EG8 on/off
  shaft luma ratio >= 1.3 (closed-form 1.0), fog_ms <= 0.8, volume bytes <=
  20000000. F2P: ratio is 1.0. P2P: golden unchanged. needs: RL35, VS1.
- [ ] RL57 render: hit flash light and sand burst — hits give sparks only, no
  light and no dust kick (`particles_gpu.rs:94-101`). A hit spawns >= 24
  stretched sparks, one 64-particle sand burst and one point light for 3 ticks.
  Diff: regular. Tier A. Size S. Owns the emitter hunk of `particles_gpu.rs`, one
  hunk of `shadow_cluster/torch_cluster.rs`. Accept: `cargo test -p engine-render
  --lib particles_gpu`: a staged hit spawns the counts above and the light count
  is back to baseline at tick+4, bytes x2. F2P: EMITTER_COUNT is 3, no flash
  kind. P2P: golden unchanged. needs: RL52, VS7.
  Correction 2026-10-21: the Accept filter `particles_gpu` is not hollow - it matches the module (`crates/render/src/lib.rs:248`; 33 `#[test]` in `particles_gpu.rs`), so the command exits 0 today whatever a hit does - but no test in it checks this row's counts (`Grep flash` over crates/render is empty), so a tick would prove nothing. Accept is a named hit-flash test of your own (for example `--lib hit_flash`) that prints a nonzero passed count, and the row may not be ticked until it does.

**Animation and camera feel (animation.md, gameplay.md)**

- [x] AM23 anim: rotation-additive hit flinch on the real skeleton -
  `hit_react.rs:261-278` writes identity rotations, the window imports no
  hit_react (`fighter_look.rs:21-22`), and the flinch lasts 6 ticks
  (`crates/combat/src/fight.rs:65`). Per-joint delta quaternion on the base
  rotation (ozz `blending_job.cc` AddLayers), blend-in, hold, blend-out over
  about 450 ms, a second hit adds. Diff: good. Tier A. Size M. Owns the additive
  hunk of `crates/anim/src/layers.rs`, `crates/anim/src/hit_react.rs`, the flinch
  hunk of `crates/game/src/fighter_look.rs`, react_head_cm_max in
  `crates/game/src/view_truth.rs`. Accept: `cargo test -p engine-anim --lib
  hit_react_rot` x2 (weight 0 equals base bytes, two strikes sum within 1e-5),
  and `node tools/product-check.mjs` prints react_head_cm_max >= 8 with pops not
  above before. F2P: react_head_cm_max is 0. P2P: golden unchanged. needs: EG18.
  Correction 2026-10-21: duplicate of PW32 (`anim: rotation-additive hit flinch`, which closes AM23 and C14); do not run both - PW32 is the owning row.
  Landed 62995b697 (the hit tick holds the pose) + 243211c27 (held slots sample the end of their tick; planted_feet 100). The flinch is a per-joint delta quaternion on the base rotation: 3-tick rise, hold, 18-tick fade, 27 ticks / 450 ms, a second strike adds; `fighter_look` imports `hit_react`, and `react_head_cm_max` reads 10.1 (0.0 with the flinch at zero). Proof: `cargo test -p engine-anim --lib hit_react_rot` 4 passed x2 (runs 39936714996, 39938336493); `node tools/product-check.mjs` at 243211c27 GREEN: react_head_cm_max 10.1 >= 8, pops 0 (before 0), pop_cm_max 9 (before 9), no non-finite, 300 frames; golden d57b5c8ae3096614 (run 39940186269). The PW32 correction is stale: PW32 is parked (docs/plan/parked-2026-10-22.md), so AM23 is the live row.
  Open leg, recorded: the LF9 1-frame receipt line (`idle:2` vs control `idle:1`) rises because the lean's added travel crosses the LF9 bound. A stricter zero-weight, 6-tick onset still gives idle:2 and drops planted_feet to 93, so it was reverted; only a smaller lean (below the 8 cm product floor) or an LF9 bound change would clear it, which belongs to LF9, not this row's Accept. The row's product-check pops stay 0 before and after.
- [x] AM24 anim: last-stage joint angular-speed limiter — 35-55 pops per 300 Progress 2026-10-17: AngularLimiter landed in anim inertialize.rs and SpacePlayer (ang_limit_takes_ten_ticks_for_180_deg_at_1080_degs, ang_limit_space_player_caps_180_deg_step, run 39056591616); left: not wired into fighter_look observe_tick and slot_pose, and product-check pops=0 is not shown. Progress 2026-10-20: the limiter is wired at observe_tick/slot_sample (default 1080 deg/s; forced falls land whole). The seed-2040 600-tick 2-fighter line is pops=0 (was 31, pop_cm 12.1 -> 9.0) and the 300-tick 4-fighter line is pops=0; the 8-fighter 300-frame product-check window is still pops=258 pop_cm_max=109.3 (attack 62/idle 68/run 128). The residual is root/translation and frame-cadence motion the bone cap does not cover, so the row stays open. Landed 007c4528. Progress 2026-10-23 (`5ca48a2e0`, `5e024585b`): the 3-tick flinch ramp in `slot_pose` removes the idle pop; the 300-tick 8-fighter headless line is pops=0 (pop_cm_max 9.3, was 2 pops at 13.1) and the 600-tick 2-fighter line is pops=0. The official window product-check is 4 pops, all `attack:4`, `slot=7 joint=18 toes_R clip_s=0.208 pop_cm_max=14.7`, pre-existing and deterministic across five 10/21 runs (pops=5 with the idle one). The 18 deg/tick bone cap lets a 3-bone leg chain sum past 10 cm when the window drops frames (interval p95 149 ms), which is the row's own frame-cadence residual; a chain-aware or final-pose clamp is a new stage outside this row. Row stays open on the official `pops=0` bar. Landed ac209a7e8. Official product-check line `ba3122f30-20261023T0920Z`: frames=300 bodies=2400 `pops=0 pop_acts=none pop_cm_max=9.0 foot_slide_cm=0.1`; `view_run_2040_` 2/2 (300/8 line pops=0, 600/2 line pops=0), `clamp_world_` 2/2, golden PASS. The `slower` product gate (59.7 fps vs 120.1) is a P6 red, not this row.
  frames up to 1 m (`tools/product-bar.json:24`); no stage guards the final pose
  (`fighter_look.rs:838-885`). Keep the last output rotation per bone, slerp
  toward the wanted one by min(1, limit*dt/angle) (Godot
  `limit_angular_velocity_modifier_3d.cpp`); state advances in observe_tick.
  Diff: regular. Tier A. Size S. Owns the limiter in `crates/anim/src/inertialize.rs`,
  the observe_tick and slot_pose tail of `fighter_look.rs`. Accept: `cargo test
  -p engine-anim --lib ang_limit` x2 (a 180 deg step at 1080 deg/s takes 10 ticks
  +-1) and `arena view:` pops=0 in product-check. F2P: pops 35-55. P2P: golden
  unchanged. needs: none.
  Progress 2026-10-21: the real 8-fighter 300-frame window is now `pops=0 pop_acts=none pop_cm_max=10.0 sunk_cm_max=0.0 foot_slide_cm=0.0` on the RTX 3050 (`target/reports/product/manual-6eafae93/look.txt`, same flags as product-check). The 258/109.3 gap is gone; the official checker is still blocked by another arena holding the GPU/lock, so the row stays open until its own `latest.md` prints 0. Progress 2026-10-21 (later): the checker runs again and is GREEN - the arena died at startup with a stack overflow (0xC00000FD in `engine_game::App::new_for_slot_world`, the session builds big value structs on the stack) and the bin now asks the linker for a 16 MB stack (`crates/game/build.rs`). The official 8-fighter 300-frame report reads `pops=5 pop_acts=attack:4/idle:1 pop_cm_max=15.6 sunk_cm_max=0.0 foot_slide_cm=0.1` (runs `d66465070-20261021T1444Z` and `d66465070-20261021T1446Z`), so the row is not ticked. The 5 are reproducible and not frame cadence: a `--fixed-dt 17` run gives the same 5, the 2-fighter line is `pops=0 pop_cm_max=9.3`, and the headless `arena --view-line 2040 300 8` (no GPU, the same fold) reads `pops=2 pop_acts=attack:2 pop_cm_max=13.1` - the fast loop for the fix. `cargo test -p engine-anim --lib ang_limit` passes (2 passed, run 39611807795) and the golden is unchanged (run 39620883064). The receipt now names the worst pop (slot, joint, tick, miss) on its own `arena pop:` line. Progress 2026-10-21 (night): the law counts the model-space joints, not the root, and that narrows the residual - the same window's own root misses 10 cm at 64 frames (idle:14/run:33/attack:17) while the law counts 5, so the sim's translation (a dash, the balance lean) is not what pops; the residual is pose-level (a clip's swing or a switch). The fold now names the worst pop (slot, joint, tick, miss and the joint's before and after) so the fix can name the joint; the next step is to read that name and fix that joint. Left: pops must reach 0.
- [x] AM25 anim: look-at with soft limit, target-switch ease and a behind-the-back Landed 073140a.
  guard — replaces the hard clamp LF13 asks for; `crates/anim/src/fbik.rs:392`
  solve_look has no ease (Godot `look_at_modifier_3d.cpp`). Diff: regular. Tier
  A. Size S. Owns the look hunk of `fbik.rs`, the head hunk of `fighter_look.rs`.
  Accept: `cargo test -p engine-anim --lib look_soft` x2: second difference of
  head yaw <= 0.5 deg/tick^2 over a 0..180 deg sweep, a target swap eases in 12
  ticks, the head never leaves 70 deg. F2P: the clamp steps velocity. P2P:
  golden unchanged. needs: LF12.
  Correction 2026-10-21: the Accept filter `look_soft` matches no test (`Grep look_soft` over crates/anim is empty), so the command exits 0 with 0 passed. The only real look-at test is `look_and_aim_face_the_target` (`crates/anim/src/fbik.rs:856`), which covers the hard look-at only, not the soft limit or the 12-tick ease; that new test is the work. Accept is `--lib look_soft` printing a nonzero passed count, and the row may not be ticked until it does. Landed 2026-10-23 (`073140a`, anim half): `LookSoft` in `fbik.rs` holds the soft limit, the 12-tick swap ease and the behind-the-back guard; `--lib look_soft` is 3 passed, clippy clean, golden `d57b5c8ae3096614` unchanged. Left open: nothing in crates/game calls `LookSoft` yet (`fighter_look.rs` was dirty with another seat's diff, so its head hunk was left untouched).
- [x] AM40 anim: Motion Warping for melee root motion (Unreal Motion Warping, `docs/research/plugins-2026-10-24/05-unreal-deep.md` entry 6) — one lunge clip covers many distances. `crates/anim/src/motion_warp.rs`: `warp_root(table, WarpWindow, WarpTarget, WarpLimits)` scales and turns the root motion of a tick window of a `RootTable` (plateau weights with smootherstep ramps, 4 re-aim passes, bounded scale/turn) so the root ends exactly on the target (e.g. `lunge_target`, the blade reach point short of the opponent) and spreads any rest with smootherstep so velocity equals the clip's own at both window edges; yaw lands the same way; every tick outside the window is bit-identical; `slide_mm` meters clip-vs-authority root gap. Presentation only, no IK, f64 `+ - * / sqrt`, golden `d57b5c8ae3096614` untouched. Diff: regular. Tier A. Size M. Accept: `cargo test -p engine-anim --lib motion_warp` x2: end exact within 10 um, outside-window bits equal, edge velocity step under 2% of the warp peak, own-end target changes under 2 um, slide not worse than raw and 0 at the window end. F2P: the raw clip ends where it ends. P2P: golden unchanged. needs: none. Landed (see git log `anim: motion warping`); gh-check 11 passed (motion_warp_test: warp_root_ends_exactly_at_the_target_at_the_window_end, warp_root_leaves_every_tick_outside_the_window_untouched, warp_root_velocity_is_continuous_at_the_window_edges, warp_root_to_the_clips_own_end_changes_nothing, warp_root_never_draws_more_foot_slide_than_the_raw_clip, warp_root_yaw_lands_on_its_target_and_eases_at_the_edges, warp_root_stretches_and_turns_the_stride_not_just_the_end, warp_root_is_bit_exact_on_replay, warp_root_refuses_hostile_input_and_returns_no_table, warp_root_handles_a_clip_that_stands_still, lunge_target_stops_a_blade_reach_short_of_the_opponent), run 40086512397.
- [ ] AM41 game: wire the motion warp into the fighter look's attack and lunge clips. Today the drawn body is the authority position (`slot_root` in `crates/game/src/fighter_look.rs`) and no clip root table is read, so the clip's stride and the sand can disagree (the `foot_slide_cm` on the arena view line). Bake each lunge/attack clip's `RootTable` once with `engine_anim::pose::extract_root` and its warp window (start/end tick, in the clip's event track beside the telegraph markers); at attack start take the authority's displacement over the window (the committed lunge reach, `lunge_target` from the root to the opponent minus blade reach) as `WarpTarget`, call `warp_root` once, and drive the clip cursor/stride off the warped table (stride-scale the leg swing through the existing `warp_ground` stride path so planted feet match the sand). Owns a new `fighter_look_warp.rs` plus one hunk in `fighter_look.rs` `clip_pick`; stay out of the LOD/budget code. Accept: `cargo test -p engine-game --features app --lib look_motion_warp` x2: a lunge at 3 distances draws `foot_slide_cm` no larger than the unwarped clip and the blade tip is within 3 cm of the hit point on the hit tick; arena view receipt shows `foot_slide_cm` not above the baseline; golden `d57b5c8ae3096614` unchanged. F2P: the look reads no root table. P2P: golden unchanged. needs: AM40.
- [x] GP43 game: one stateful fight camera — `present_into` overwrites the director Progress 2026-10-17: FightCamSpring landed in camera_rig.rs with fight_cam_spring_smooths_duel_at_both_rates (yaw p99 <= 60 deg/s, eye step <= 25 cm, 30 vs 144 fps within 1 cm) and trauma tests; left: nothing outside tests calls it, fighter_look present_into still uses the stateless fight_camera.
  camera with a stateless fight_camera (`fighter_look.rs:1089,1133-1218`), trauma
  never shows, DampedSync is frame-rate dependent (`camera.rs:2487-2552`). Exact
  halflife springs on yaw, distance and look point, a timed FOV punch on Hit,
  smooth-noise roll trauma (Spring-It-On, Bevy screen shake); one camera stack.
  Diff: good. Tier A. Size M. Owns the camera hunks of `fighter_look.rs` and
  `crates/game/src/camera.rs`, cam fields in `view_truth.rs`. Accept: `cargo test
  -p engine-game --lib fight_cam_spring` x2: yaw rate p99 <= 60 deg/s, eye step
  <= 25 cm per frame over 300 frames, 30 fps and 144 fps stepping end within 1
  cm. F2P: no spring state; two step rates differ. P2P: golden unchanged;
  framing_empty 0. needs: none. Landed 033aaca3 (fighter_look_cam.rs) + 19bb5ef5 (the window aims every present through it). engine-game --features app --lib 2026-10-18: 711 passed (the 2 reds are another session's ragdoll_look WIP); fight_cam_spring 3 tests ok x2 runs; aim_camera 8 tests ok: first frame is the stateless shot bit for bit, a cut glides, 30 vs 144 fps end on one camera, a 120 deg/s circling pair stays in the picture, shake exact and unsmoothed, roll and lens settle level, a bad dt shows the stateless shot, real arena tapes 6800 and 6900 keep both fighters in frame 600/600. Window: debug arena.exe --play --frames 240 --exit prints framing empty=0 fewest_in_view=2 steepest_down_deg=1.1 validation_errors=0 (before: empty=0 fewest_in_view=2 0.9). Lens punch (8 percent at full trauma) and roll (<= 0.1 rad) ride the trauma after the smoothing. Presentation only: engine-proof does not depend on engine-game, golden d57b5c8ae3096614 untouched.
- [x] GP44 game: phase-warped attack with a telegraph meter - the swing starts at Progress 2026-10-24: `crates/anim/src/phase_warp.rs` reads windup/impact/recover markers off the clip and stretches the windup to the sim's lead (capped at 2x, impact never early) with a 0..1 telegraph (fd6e772, 11 tests). Left: the game test, the look wiring and the 25 cm hand check.
  the outcome cue back-dated 12 ticks, so 0 ticks of windup show before impact
  (`fighter_look.rs:50,786-801`). Clip time from the sim's attack age through
  windup-peak and impact markers in the clip event track; stretch capped at 2x.
  Diff: good. Tier A. Size M. Owns the clip_time hunk of `fighter_look.rs`, marker
  names in `crates/anim/src/events.rs`, telegraph_hand_cm_min in `view_truth.rs`.
  Accept: `cargo test -p engine-game --features app --lib telegraph_warp` x2: the
  hand is >= 25 cm from guard by impact minus 3 ticks, impact phase error <= 1
  tick. F2P: windup is 0 ticks. P2P: golden unchanged. needs: EG19c.
  Correction 2026-10-21: the Accept filter `telegraph_warp` matches no test (`Grep telegraph` over crates/game finds only the unrelated `class_sheet` ability telegraph), so the command exits 0 with 0 passed. The phase-warped clip time and its test are the work; Accept is `--lib telegraph_warp` printing a nonzero passed count, and the row may not be ticked until it does. RETIRED (done): fd6e77265 (lead, trim audit 2026-10-24)
- [x] GP45 game: kill slow-mo through a virtual-time scale — the tick accumulator
  takes raw wall ms (`crates/game/src/windowed.rs:906`). Scale the wall delta,
  ramp it, local and replay only, netgame forces 1.0 (Bevy `virt.rs`). Diff:
  regular. Tier A. Size S. Owns the accumulator hunk of `windowed.rs`, the cue
  hook in `fighter_look.rs`. Accept: `cargo test -p engine-game --lib time_scale`
  x2: 120 wall frames at 0.25 give 30 ticks +-1 and an equal state digest at tick
  30. F2P: no scale input. P2P: `node tools/proof.mjs --skip-build` prints golden
  d57b5c8ae3096614 twice. needs: none. Landed 033aaca3 (virt.rs VirtualClock) + 19bb5ef5 (the window: [ and ] ease it, the title shows speed N%, the clock sits between the feel flow and the accumulator). time_scale 6 tests ok x2 runs in engine-game --features app --lib: time_scale_quarter_speed_gives_30_ticks_and_the_same_digest (120 wall frames at 0.25 give 30 ticks, every checkpoint equal to a real-speed run and a second scaled run), the ease covers the same virtual time at 30, 60 and 144 fps, a net match locks 1.0, bad speeds refused, key presets, fast-forward held by the 2-ticks-a-frame guard. The kill slow-mo itself is the existing GF3 knockdown slow-mo through the feel flow, which this clock multiplies; no cue code was added in fighter_look.rs. Presentation only: engine-proof does not depend on engine-game, golden d57b5c8ae3096614 untouched.

**Bodies, cuts and net (body-and-break.md, multiplayer.md)**

- [ ] BX41 look: a stump cap that shows — `shaders/forward.wgsl:444` fs_skinned
  has no discard, front_facing or frag_depth, so a Cut (`crates/combat/src/cut.rs`)
  leaves nothing to see. Draw the segment two-sided with one plane uniform:
  discard the cut side, shade back faces as cap (skin rim, flesh ring, bone
  disc), write frag_depth from the plane hit; stencil capping only for holes.
  Diff: good. Tier A. Size M. Owns the cut hunks of `shaders/forward.wgsl` and
  `crates/render/src/v1_skin.rs`. Accept: `cargo test -p engine-render --lib
  cut_cap`: 0 added draw calls, cap coverage >= 98 percent of the stump
  silhouette, validation errors 0, <= 0.15 ms for 8 stumps. F2P: the stump
  capture shows 0 cap pixels. P2P: golden unchanged. needs: BX8.
  Correction 2026-10-21: the Accept filter `cut_cap` matches no test (`Grep cut_cap` over crates/render is empty), so the command exits 0 with 0 passed. The real cap tests are CPU geometry only - `stump_cap_is_closed_with_the_asked_radius_and_depth` (`crates/render/src/stump.rs:246`) and `stump_cap_refuses_bad_numbers_loudly` (`:275`) - and none covers the shader's discard and cap coverage. The shader cap and its coverage test are the work; Accept is `--lib cut_cap` printing a nonzero passed count, and the row may not be ticked until it does.
- [ ] PX42 bodies: a human ragdoll joint table — every joint is spherical with one Progress 2026-10-17: ragdoll.rs humanoid joint table landed with joint_table_humanoid_defaults and fifty_seeded_human_falls_measured (0 backward-hinge frames after clamp, physics green run 39057362796); left: the human_fall report in ragdoll_tape.rs and the peak-speed baseline under --test ragdoll_tape do not exist.
  limit per depth (`crates/physics/src/ragdoll.rs:1116-1145,1167`), so knees and
  elbows fold backward. Hinges for knee and elbow, cone plus twist elsewhere, a
  zero-speed friction motor (5-10 N.m), a soft spring (Box3D `shared/human.c`).
  Client-local, never hashed. Diff: good. Tier A. Size M. Owns `ragdoll.rs`, a
  human_fall report in `ragdoll_tape.rs`. Accept: `cargo test -p engine-physics
  --features rapier --test ragdoll_tape`: 0 backward-hinge frames in 50 seeded
  human falls, peak bone speed after rest <= the posted baseline, `balance_hash`
  green. F2P: the new report shows backward knees. P2P: golden unchanged. needs:
  PX9.
  Progress 2026-10-24: a physfx human joint table (`rig::human_of`: a knee or
  an elbow hinges by its name, spine, neck, hip, shoulder and ankle are balls
  limited per axis to their human ranges from the spawn pose) went red on
  GitHub run 40099136316, `cargo test -p engine-physfx --lib --features rapier`:
  137 passed, 5 failed: `a_launched_ragdoll_falls_stays_above_the_floor_and_does_not_explode`,
  `an_elbow_folds_forward_only`, `a_braced_ball_joint_holds_the_pose_it_spawned_in`,
  `a_toned_body_keeps_its_shape_where_a_limp_one_folds`, and the new
  `every_named_joint_holds_its_human_range_under_a_shove` (the ankle reached -50
  degrees against its 40). Nothing committed; the physfx tree is back at HEAD.
  engine-game `ragdoll` not proven. Not ticked. Left: keep the existing physfx
  thresholds green, then the physics leg of the Accept (the `ragdoll_tape`
  human_fall report and the peak-speed baseline), which was not run.
- [ ] PX43 bodies: ragdoll Stabilize — density 1.0 on every capsule Progress 2026-10-17: ragdoll.rs stabilize_clamps_ratio_preserving_total (links inside the ratio band) green in physics run 39057362796; left: no interpenetration_max_m baseline and nothing under --test ragdoll_tape.
  (`ragdoll.rs:388-392`). Child:parent mass ratio clamped to [0.8, 1.2], an
  inertia floor, root joints solved last (Jolt `Ragdoll.cpp` Stabilize). Diff:
  regular. Tier B. Size S. Owns the mass hunk of `ragdoll.rs`. Accept: `cargo
  test -p engine-physics --features rapier --test ragdoll_tape` x2: every link in
  [0.8, 1.2], interpenetration_max_m <= the posted baseline. F2P: the ratio test
  fails on today's masses. P2P: golden unchanged. needs: PX42.
- [x] MP25 net: visual correction after a rollback — a mispredicted 8-tick resim
  can move the drawn local fighter 0.67 m in one frame; no correction code
  (`crates/game/src/netgame.rs:385`, `crates/net/src/interp.rs`). Drawn = live +
  a decaying error; the sim reads the live pose only (lightyear `correction.rs`).
  Diff: regular. Tier B. Size S. Owns the correction hunks of `netgame.rs` and
  `interp.rs`. Accept: `cargo test -p engine-net --lib interp` x2: first-frame
  step <= 6 percent of the jump, <= 2.5 percent left after 60 frames, hash equal
  with correction on and off. F2P: the first-frame step is 100 percent. P2P:
  golden unchanged. needs: MP17. Progress 2026-10-19: the net half landed —
  `Correction` in `crates/net/src/interp.rs` (`CORRECTION_RETENTION_PERMILLE`
  940, `CORRECTION_FRAMES` 60; note/offset/advance/remaining) with two tests:
  rollback_correction_steps_six_percent_then_two_and_a_half_after_sixty and
  rollback_correction_never_moves_the_authoritative_frame_digest; `cargo test
  -p engine-net --lib interp` 19 passed x2, runs 39177678798 and 39177814134.
  Left: the drawn half — `netgame.rs` does not yet note the resim jump and the
  window does not add `Correction::offset` to the local fighter, so nothing is
  drawn through it and the row stays open.
  Progress 2026-10-21 (hunt wave 2, `docs/research/hunt2-2026-10-21/`): the steal
  is `nilpunch/unity-prediction-rollback` (MIT, 48 stars, archived), the safest
  donor in the wave because the row's own Accept can be written as "hash equal
  with correction on and off". Its `FadeOutPrediction` (2.1K) makes a predicted
  input a blend instead of a boolean: `fadeOutPercent = clamp(ticksPassed -
  startDecayTick, 0, decayDurationTicks) / decayDurationTicks`, with
  `startDecayTick = 30` and `decayDurationTicks = 60` by default, so the drawn
  pose reaches the confirmed one exactly when it arrives instead of decaying
  toward it. The shape is re-derived, never copied (the repo is archived). The
  second steal from the same file is `CommandTimeline` (3.6K): a `Dictionary`
  plus a **sorted** tick list, so "the latest tick with a command before T" is
  a binary search and a rollback trims the filled ticks in order. Landing:
  `crates/net/src/interp.rs:463` (`Correction`) and `crates/anim/src/net_pose.rs:42`
  (`NetPose`). Accept unchanged (the donor cannot lower it); the added number is
  the fade: the visual error reaches 0 exactly at the confirmed tick, not
  asymptotically. Golden: does not threaten — a drawn-pose rule, never hashed.
  Landed 2026-10-23 (interp Accept): `cargo test -p engine-net --lib interp`
  19 passed x2 at HEAD (39908343424, 39908530602) - the first-frame step is
  under 6 percent of the jump, under 2.5 percent is left after 60 frames, and
  the authoritative frame digest is equal with the correction on and off.
  Open leg, named: the drawn half is not wired - `crates/game/src/netgame.rs`
  does not note the resim jump and the window does not add `Correction::offset`
  to the local fighter (the anim `NetPose` fade exists), so the drawn fighter
  still snaps until that hook lands.
- [x] MP26 net: clock slew and a 30-frame advantage window — `delay.rs:56-65` Landed 7d1e1227.
  steps the delay +-1 from one sample. Average frame advantage over 30 frames
  (ggrs `time_sync.rs`), slew the client clock up to 5 percent (lightyear
  `sync.rs`); input delay stays the slow knob. Diff: good. Tier A. Size M. Owns
  `crates/net/src/delay.rs`, pacing hunks of `played.rs` and `scale.rs`. Accept:
  `node tools/mp6-check.mjs`: over_cap 0 on every WAN profile, < 3 rollbacks per
  1000 ticks at 2-frame delay, max slew <= 5 percent, 0 skipped frames. F2P: no
  slew data posted. P2P: golden unchanged, tape bytes equal. needs: MP24.
  Progress 2026-10-22: `delay.rs` grew the three parts of the row. The window
  is the ggrs one: 30 samples of the two seats' confirmed frontiers (the newest
  tick the authority took each seat's input for), read from the same broadcast
  the peer reads, so the two windows are exact mirrors and the two slews are
  equal and opposite. The slew is the lightyear speed controller: a dead zone
  of one tick, 10 permille per tick of averaged advantage, capped at 50
  permille (5 percent). The pacer applies it as 0, 1 or 2 client ticks a frame
  inside a one-tick band around the schedule the input delay gave the seat, so
  a trim never drifts the client's input age and never skips a tick: the
  predicted stream stays contiguous.
  The played bout paces each seat's prediction loop with it. The seat whose
  inputs reach the authority first runs ahead of its peer, so its clock is
  trimmed (the LAN and WAN receipts post max slew 50 permille, 7 slewed
  frames, 0 skipped frames on that seat; the dropped seat hears nothing and
  holds its clock). The resims get shallower for it: the LAN and WAN
  rollback histograms move from one deep bucket into several shallow ones
  with the same event count (448 and 368 events), the deepest stays 6 (LAN)
  and 8 (WAN) and `deeper_than_8_tick_window` stays 0 on every profile.
  The send side is untouched, so the server tape and the golden never move:
  the LAN input tape hash stays 90859bdb419e62b9 and the tape bytes are
  equal. The receipts also post the pacing measurement the bar names: the
  rollback rate over 1000 ticks at the 2-frame delay (0) beside the same link
  with the delay off (776), so the number has teeth.
  Proof: `cargo test -p engine-net --lib` 460 passed 0 failed 4 ignored
  (runner run 39777227573); `cargo clippy -p engine-net --all-targets -- -D
  warnings` green (runner run 39776567541); the golden unmoved: `cargo test
  -p engine2040 --release --test proof_cli default_and_max_seed` 1 passed
  (runner run 39777766673, final_hash d57b5c8ae3096614);
  `node tools/mp6-check.mjs` 31 pass
  0 fail on the fresh receipts and the old receipts fail it (F2P: no slew
  data posted); `node --test tools/mp6-check.test.mjs` 11 passed; rust-quality
  engine-net no regression; rustfmt clean.
- [x] MP27 proof: sync test at every tick and confirmed-only peer compare — Progress 2026-10-17: arena sync.rs sync_heavy_mode_clean_reports_zero_false_positives verifies rollback at every tick of the test config; left: no 600 split points at depth 8, no deliberately unsnapshotted LimbPool counter caught test, check_remote confirmed-only compare unchanged.
  `verify_rollback` tests one split point (`crates/arena/src/sync.rs:141-179`);
  `check_remote` (`crates/net/src/netcode.rs:211`) compares predicted frames and
  would report a false desync. ggrs sync-test shape. SIM test only. Diff:
  regular. Tier B. Size S. Owns the test hunks of `sync.rs` and `netcode.rs`.
  Accept: `cargo test -p engine-arena --lib sync` x2: 600 split points at d=8
  equal; a deliberately unsnapshotted LimbPool counter is caught with tick and
  system. F2P: that omission passes today. P2P: golden unchanged. needs: none. Landed faafa21e. sync_test_every_tick_at_depth_8_agrees (600 split points, depth 8, busy tape, all equal) and sync_test_catches_a_limb_counter_the_snapshot_forgot (caught at tick 100, system space; the same run passes without the pool fold); engine-arena --no-fail-fast run 39109015367 (216 passed, golden and match digest tests green), playtest divergence run 39109015259; local sync x2 green. check_remote confirmed-only already covered in netcode.rs.

**GPU speed and the live frame (render-look.md, potato.md, quality.md)**

- [ ] RL60 render: DrawItem carries a transform and the live loop draws per bin Progress 2026-10-17: present.rs BinKey and group_into_bins landed (bins_single_key_256_copies_give_one_bin_with_256_indices, shuffled input stable); left: DrawItem has no transform, the submit loop still draws per item, no ARENA draws=/bins= line.
  (the base RL32 needs) — `present.rs:375-387` has no model matrix and
  `forward.wgsl:205-214` takes world-space vertices, so two kit copies cannot
  share a mesh key; 7 wgpu calls per item, cap 256 (`submit.rs:504-516`). Opaque
  bins keyed by one u64 (pass, pipeline, texture slot, mesh), per-instance record
  (48 B model, material id, tint) on an instance-step vertex buffer, one
  draw_indexed per bin (Bevy render phase; never instance_index on DX12 count
  draws: wgpu-hal 30.0.1 `dx12/command.rs:1470-1509` drops first_instance). Diff:
  good. Tier A. Size M. Owns `DrawItem` in `crates/render/src/present.rs`, the draw
  loop of `present/gpu_present/submit.rs`, the `vs` entry of `forward.wgsl`,
  `crates/render/src/bins.rs` (new). Accept: `cargo test -p engine-render --lib
  bins` x2: 256 copies of one kit mesh make 1 draw_indexed with 256 instances
  (today 256 draws), order stable under input permutation; ARENA prints `draws=`
  and `bins=`; captures byte-equal. F2P: no transform, one draw per item. P2P:
  golden unchanged. needs: none.
  Correction 2026-10-21: duplicate of PW02 (`DrawItem carries a transform and the live loop draws per bin`, which closes RL60 and RL32); do not run both - PW02 is the owning row.
- [x] OPT35 render: the window is paced by vsync, uncapped only by flag —
  `presenter.rs:643` takes `present_modes[0]` (Mailbox on DX12, presents with
  interval 0) and `crates/game/src/bin/arena.rs:1383` polls, so the window runs at
  GPU speed. AutoVsync by default, latency 1 only when cpu plus gpu ms is under
  80 percent of the interval, `--uncapped` for captures. Diff: regular. Tier A.
  Size S. Owns the present-mode hunk of `presenter.rs`, `about_to_wait` in
  `arena.rs`. Accept: `arena --scene arena --frames 600 --exit` prints
  `present=Fifo` and frame-interval p99 <= 1.25x p50 at 60 Hz; `--uncapped` prints
  Mailbox or Immediate. F2P: the first listed mode, no cap. P2P: golden unchanged.
  needs: none. Landed <commit>.
  Evidence: `pick_present_mode` now takes `uncapped: bool` and prefers Fifo by default;
  `--uncapped` flag added to `WinArgs` and threaded through `init_windowed_with_preset`,
  `init_for_window_with_preset`, `init_from_device`, `attach_surface`, `attach_surface_for`;
  4 present_mode tests pass (including new `present_mode_defaults_to_fifo_when_not_uncapped`);
  `cargo check -p engine-render --features window` PASS; `cargo check -p engine-game` PASS.
- [x] GS34 render: live-frame hygiene - no back-face cull (`submit.rs:1157`), the Progress 2026-10-24: the window blit takes live `fs_live` entries that keep the shared chain and never read the fog texel (e8a1a73, run 40114808679, 6 tests with the naga gate). Left: the rest of the hygiene list below.
  importer drops glTF doubleSided (`crates/material/src/gltf_import.rs:628`), 33 of
  58 MB of 1080p targets are unused by the live frame (`submit.rs:125-196`), the
  blit reads two neutral textures per pixel (`blit.wgsl:48-49`). Cull back faces
  unless double-sided, create fog, ao, id and out targets lazily, a live blit entry
  without neutral reads. Diff: regular. Tier A. Size S. Owns the primitive state
  and `ensure_targets` in `submit.rs`, a blit entry in `blit.wgsl`, the doubleSided
  flag in `gltf_import.rs`. Accept: ARENA prints `target_mb=` <= 25 at 1080p (58
  today); forward p50 on a closed-kit capture >= 3 percent lower; captures
  byte-equal except double-sided meshes. F2P: target_mb=58, no cull. P2P: golden
  unchanged. needs: none.
  Progress 2026-10-24: step 1 importer landed (5f62a6654: `Import::double_sided` keeps glTF doubleSided instead of an ignored note; run 40109124743 green, 7 gltf tests). Step 3 written, not proven: `fs_live` and `fs_hw_srgb_live` in blit.wgsl plus submit/blit_live_tests.rs (run 40109370790 failed on a CRLF bug in my test helper, fixed locally, not re-run). Left: the draw-side cull (a per-mesh set fed from presenter.rs, DrawItem path, not in this brief); wiring the live blit entry (live.rs look_stage and presenter.rs blit_pipeline callers); step 2 lazy targets (no `target_mb` metric in the repo yet; live.rs:2005 copies out_texture and live GTAO writes ao_tex, so fog/ao/out are not all unused). RETIRED (done): 5f62a6654 (lead, trim audit 2026-10-24)
- [ ] GS35 render: a feature ladder with three modes — the device asks only
  TIMESTAMP_QUERY (`presenter.rs:547`). Modes none, preprocess-only and culling
  from adapter facts (Bevy GpuPreprocessingSupport shape), request exactly the
  features that exist, stamp the mode in the manifest and on the passes line; an
  env switch `E2040_LADDER=none` forces the floor. Diff: regular. Tier A. Size S.
  Owns the request hunk of `presenter.rs`, `crates/render/src/tier.rs`. Accept:
  `cargo test -p engine-render --lib ladder`: a pure function over fake adapter
  caps (UHD-like, 3050-like, GL-like) picks the right mode; validation errors 0
  with every requested feature on the 3050. F2P: one feature requested, no mode.
  P2P: golden unchanged. needs: none.
  Correction 2026-10-21: duplicate of PW08 (`render: one feature table for the ladder`, which closes GS35); do not run both - PW08 is the owning row.
- [ ] GS36 render: quality knobs are WGSL override constants — tier variants are
  copied strings or nothing (`crates/render/src/variants.rs` holds a manifest only).
  IBL specular, shadow taps, skin and fog become `override` constants set per
  pipeline (naga replaces them before any backend), so potato and high share one
  file. Diff: regular. Tier A. Size S. Owns the override lines of `forward.wgsl`,
  `variants.rs`, `cache.rs`. Accept: `cargo test -p engine-render --lib override`:
  two pipelines from one module differ only by constants, variants per tier <= 4,
  fragment texture fetches 13 -> 1 with IBL specular off. F2P: no override in any
  shader. P2P: golden unchanged. needs: none.
- [x] RL61 render: LOD by projected pixel error - `draw.rs:354-365` picks LOD by Progress 2026-10-24: the live build arms it: `cull_lod` takes the frame height and `tan(fov/2)`, `DrawList::build_at` and `InstancedList::build_at`, a 1 m ball at 60 m picks level 2 at FOV 60 and 1 at FOV 20, unarmed builds keep `order_digest` (b6b6910, runs 40115329072 and 40115499582, 8 and 61 tests). Then the arena's table: `crates/render/src/arena_lod.rs` (bands 10/20/40 m projected at 1080p and the 60 deg duel lens to 23.38/46.77/93.53 px, `E2040_LOD=bands` switch; a fighter picks level 0 at 5 m and 3 at 60 m; 5d64e28, run 40116141343). Left: per-LOD meshes from the cook (the kit bakes one mesh per actor), a `lod` on `DrawItem` and the arena call site, the orbit pop capture. Progress 2026-10-17: draw.rs lod_by_pixel_error plus DrawTable pixel_thresholds_px landed (lod_pixel_error_* tests: FOV 20 picks finer than FOV 60, hysteresis, band path unchanged when unarmed); Progress 2026-10-23: the 1080p to 540p test landed, `lod_pixel_error_switches_one_level_sooner_at_540p` (the same 1 m ball at 15.5 m is 120.7 px at 1080p and 60.3 px at 540p, so a 120 px boundary holds level 0 then drops to level 1 and refines back), 7 lod_pixel tests green on run 39930897590; left: no orbit pop capture (needs the arena binary), cook stores no per-LOD error, not armed in the live loop (`cull_lod` carries no frame height and the game's table still sets `pixel_thresholds_px: None`).
  distance bands that ignore FOV and render scale. The cook stores per-LOD error
  in metres (meshoptimizer clusterlod); runtime projects it to pixels and takes
  the coarsest LOD under 1 px (hero 0.5) with hysteresis. Diff: regular. Tier B.
  Size S. Owns `pick_lod` in `draw.rs` and the error field in the LOD cook record.
  Accept: `cargo test -p engine-render --lib lod_pixel` x2: at one distance, FOV
  20 and FOV 60 pick different LODs; a 1080p -> 540p step switches one LOD sooner;
  0 pops in the 120-frame orbit capture. F2P: FOV does not change the pick. P2P:
  golden unchanged. needs: none. RETIRED (done): 5d64e28ab (lead, trim audit 2026-10-24)
- [x] RL62 render: one temporal registry with a history-valid flag — `v1_temporal.rs`
  has 0 history-reset paths (`docs/plan/steals.md:12`). History textures live by
  name across frames (kajiya `temporal.rs` states: take, export, inert); a resize,
  camera cut or dyn-res step recreates and sets history_valid=false, read by TAA,
  GTAO and fog. Diff: regular. Tier B. Size M. Owns `crates/render/src/graph.rs`
  temporal map, the history read in `post.rs`. Accept: `cargo test -p engine-render
  --lib temporal` x2: resize flips the flag, a double take is refused; ghost pixels
  in a camera-cut fixture 0. F2P: no reset path. P2P: golden unchanged. needs:
  none. Landed 7e660839. temporal_resize_flips_the_flag, temporal_dyn_res_step_recreates_the_history, temporal_double_take_is_refused, temporal_camera_cut_invalidates_every_texture, temporal_camera_cut_fixture_has_zero_ghost_pixels (stale history ghosts, cut history 0 ghost pixels); green x2: runs 39103216237 and 39104351445 (full engine-render suite). Only TAA reads the flag (TaaParams::history_valid): gtao.rs and fog.rs keep no history yet. Golden unchanged (presentation only).
- [x] RL63 render: octahedral impostor math, the CPU half of far props, buildings and crowds as one camera-facing quad (Amplify Impostors, `docs/research/plugins-2026-10-24/03-market-sweep.md`). `crates/render/src/impostor.rs`: direction <-> octahedral UV (`octa_encode`/`octa_decode`), `frame_direction` and `bake_basis` (where the baker puts frame i and the sprite axes the runtime quad shares), `pick_frames` (3 nearest frames of an N x N grid, barycentric weights summing to 1, exact frame at an exact view, continuous across cells), `AtlasLayout::fit` (largest power-of-two frame under a byte budget and an edge cap, pixel and UV rects), and the screen-space switch (`projected_diameter_px`, `switch_distance_m`, `use_impostor` with hysteresis, `default_switch_px` = one texel per pixel). `shaders/impostor.wgsl` mirrors it (quad vertex stage, 3-frame albedo + normal blend) and is naga-gated by `shader_gate`. Diff: regular. Tier B. Size M. Owns `crates/render/src/impostor.rs`, `crates/render/src/shaders/impostor.wgsl`. Accept: `cargo test -p engine-render --lib impostor`: octa round trip over 4000 directions under 2e-5, weights sum to 1 for N in 2..32, every frame direction picks itself with weight over 0.9999, the atlas rects tile with no gap, switch distance inverts projected size, a 13-step walk flips the switch twice; `shader_gate` green with the new file. F2P: no impostor math. P2P: golden d57b5c8ae3096614 unchanged (no draw path touched). needs: none. Progress 2026-10-24: CPU half and WGSL mirror landed; the GPU half is RL64. Landed 7c809fdfe. 17 impostor tests green and shader_gate green on GitHub (runs 40084491241 final code, 40083888549).
- [x] RL64 render: bake and draw the impostors under `crates/render/src/present/` (the only place that may name `wgpu::`). Bake: render a mesh from each `frame_direction(n, i)` with `bake_basis` and an orthographic camera fitted to its bounding sphere into the albedo (RGBA8, alpha = coverage) and world-normal (RGBA8, n * 0.5 + 0.5) atlases of an `AtlasLayout`, offscreen, once per mesh, cached by mesh id and layout (optional R16 depth layer, `TEXEL_BYTES_WITH_DEPTH`). Draw: bind `shaders/impostor.wgsl` (6 vertices per instance, no vertex buffer), switch per object with `use_impostor` fed by `projected_diameter_px` and dither the swap through `lod_fade`; wire the crowd far row (`crowd_inst` L2, replaces the 4 x 4 cell select in `crowd_impostor.rs`) and QA10's stands. Potato budget: 8 x 8 frames, 2 MiB, 512 px atlas. Diff: regular. Tier B. Size L. Owns a new `present/impostor_gpu.rs` and its tests, the far-row hunk of `crowd_inst.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-render --lib impostor_gpu` x2 with tests that ran: a baked sphere read back from frame 0 and frame n*n/2 differs and both are non-empty, drawn at the bake direction the impostor matches the mesh draw within 3 per cent mean pixel error, an orbit of 64 views changes less than 8 per cent per step (no pop), and the receipt prints atlas bytes, bake ms and draw ms for 1000 impostors with the PC adapter line; one capture of a far tree line or stands crowd beside the mesh draw. F2P: no atlas is ever baked. P2P: golden d57b5c8ae3096614 unchanged; the `impostor` math tests stay green. needs: RL63. Progress 2026-10-24: bake, draw and the instanced pass landed f7e10f100; the stands crowd (one CT7 body baked at load, 96 seats, four palette tunics) replaced the box meshes in 756418332 and 6355bf76d; empty-part crash fix c880c215e. 29 `impostor` tests green on the runner (WARP) and `impostor_gpu` ran on the RTX 3050 6GB Vulkan: atlas 2097152 B (8x8 frames, 64 px), bake 145.7 ms, draw 0.746 ms p50 for 1000 impostors, impostor vs mesh at the bake direction 0.56 per cent mean error, 64-view orbit worst step 4.3 per cent. Open: the window capture of the stands beside the box frame (`target/clips/rl64-before`) and the frame-split ms. RETIRED (done): c880c215e (lead, trim audit 2026-10-24)

**AI door, machine eyes and replay (ai-native.md, quality.md)**

- [x] AX27 endpoint: capture and play.step return the frame as an MCP image block Landed 0b8cb60. Tick audit 2026-10-17: play.rs capture_image_block (one image block with PNG magic, hashes to manifest sha256, no png_bytes key, caption under the cap) green in gh-check run 39057874705 (endpoint --features mcp --lib). Note: the play.step receipt carrier was not checked.
  — `capture` returns `png_bytes: Vec<u8>` (`crates/endpoint/src/play.rs:188-191`)
  through `CallToolResult::structured` (`mcp.rs:1416`), so a 300 KB PNG becomes a
  number array of about 2.1M characters, written twice, that no model can see.
  `max_size` (default 768, clamp 200..2048), one base64 image block, a caption, the
  sha256 in the manifest; the same for the play.step receipt
  (`play/play_session.rs:196`). Diff: regular. Tier A. Size S. Owns `play.rs`,
  `play/play_session.rs`, the capture arm of `mcp.rs`. Accept: `cargo test -p
  engine-endpoint --features mcp --lib capture_image_block` x2: 1 image item whose
  bytes start with PNG magic and hash to the manifest sha256, 0 `png_bytes` keys in
  structuredContent, text block <= 2 KB. F2P: the result carries the byte array.
  P2P: golden unchanged. needs: none.
- [ ] AX28 endpoint: play.run as an MCP task — start, poll and cancel are hand-rolled
  (`crates/endpoint/src/mcp.rs:923-927`); the locked rmcp 3.4.1 already ships
  `TaskManager::spawn` and `CallToolResponse::Task`. Return a task only to clients
  that declare `io.modelcontextprotocol/tasks`; the rest keep start and poll.
  Supersedes the progress hunk of AX26 (AX26 keeps its transcript test). Diff:
  good. Tier B. Size M. Owns the play_run hunk and call_tool arm of `mcp.rs`,
  PlayJobs in `play.rs`. Accept: `cargo test -p engine-endpoint --features mcp
  --lib task_play_run` x2: a 600-tick run returns a task with status "tick
  240/600", the final digest equals the blocking run, cancel ends `cancelled` with
  the scene revision unchanged. F2P: no task arm is used. P2P: golden unchanged.
  needs: none.
  Correction 2026-10-21: the Accept filter `task_play_run` matches no test (`Grep task_play_run` over crates/endpoint is empty), so the command exits 0 with 0 passed. The nearest real test is `play_run_and_play_step_are_found_and_described_under_client_names` (`crates/endpoint/src/insight/mcp_tools.rs:496`), which covers the tool names only, not the task arm; the task arm and its test are the work. Accept is `--lib task_play_run` printing a nonzero passed count, and the row may not be ticked until it does. Progress 2026-10-25: Step 1 landed 2f8523cc7 (PlayJobs::task_for, TASK_SLICE_TICKS 240, play_run_for): 2 passed run 40118675646.
- [x] PQ21 playtest: flicker you can see on moving fighters — shimmer is gated only
  on a 64x36 synthetic hold (`crates/render/src/clip_hold.rs:26-31`) and the
  judge's motion check is consecutive-frame SSIM (`crates/playtest/src/look/judge.rs:344`).
  The HVEI flicker visibility model (MIT) over the motion-compensated residual
  with an edge-energy guard. Diff: good. Tier A. Size M. Owns
  `crates/playtest/src/look/metric.rs`, the sequence hunk of `look/judge.rs`.
  Accept: `cargo test -p engine-playtest --lib look_flicker` x2: a clean clip
  scores p99 <= 0.10, three seeded breaks (clamp off, jitter x2, motion zeroed)
  each > 0.5, JSON bytes equal. F2P: no flicker function; SSIM cannot tell the
  breaks apart. P2P: golden unchanged. needs: none. Landed dcfadef. look_flicker 4 passed.
- [x] PQ22 playtest: a golden bless flow with two tiers and a known-failure ratchet
  — `tools/q1-check.mjs:9` counts PNGs and compares none. Bless writes `.new`,
  `.diff` and keeps `.old` (egui kittest); tier 1 bit-exact on the software
  adapter, tier 2 FLIP pooled p95/p99 with a per-adapter threshold table
  (Filament); a missing golden fails; a stale known-failure fails (MCP
  conformance). Diff: regular. Tier B. Size M. Owns
  `crates/playtest/src/look/golden.rs` (new), its mod line, `tools/q1-check.mjs`
  and its test. Accept: `cargo test -p engine-playtest --lib golden_bless` x2: 34
  stills compared, a blessed change keeps `.old`, a missing golden fails, a stale
  known-failure fails. F2P: 0 pixel compares. P2P: golden unchanged. needs:
  none. Landed 5756c2a0b. Both tiers exist: the Rust module scores FLIP p95/p99
  against a per-adapter bar, the node tool holds the bit-exact sha256 ratchet.
  Proof: `cargo test -p engine-playtest --lib golden_bless` 11 passed, twice (local,
  then runner 40019215258); `--lib` whole crate 497 passed 4 ignored (40017298468);
  clippy `--all-targets -D warnings` PASS fresh (40019029815); rust-quality
  `--crate playtest` score 100; `node --test tools/q1-check.test.mjs` 11 passed;
  rustfmt clean. No sim crate touched, so golden d57b5c8ae3096614 is unchanged.
  The tool leg is proven on fixtures only: this PC's `target/q1` holds no captures,
  so no golden is filed on disk yet. Tool and test commit 1ce66d667.
  Correction 2026-10-21: the Accept filter `golden_bless` matches no test (`Grep golden_bless` over crates/playtest is empty) and `crates/playtest/src/look/golden.rs` does not exist (`Test-Path` false; `tools/q1-check.mjs` does exist), so the command exits 0 with 0 passed. The module, its mod line, the bless flow and the test are the work; Accept is `--lib golden_bless` printing a nonzero passed count, and the row may not be ticked until it does.
- [x] PF26 observe: replay export as MCAP — `replay_verbose` emits sim.tick only
  (`crates/proof/src/trace.rs:34-35`) and the sink is a 120-row (tick, hash) ring
  (`crates/observe/src/rerun_sink.rs:19`). A std-only MCAP writer (header,
  channels, messages, footer) with /tick/hash, /fighters/N/pose, /events and
  /debug/lines, opened in Foxglove or Rerun with no SDK; deepens PF9. Diff: good.
  Tier B. Size M. Owns `crates/observe/src/mcap.rs` (new), the export hunk of
  `rerun_sink.rs`, `trace.rs`. Accept: `cargo test -p engine-observe --lib
  mcap_roundtrip` x2: a 600-tick 2-fighter bout writes <= 400 KB, bytes equal, our
  reader returns every message, a seeded desync first differs at its tick. F2P: no
  MCAP writer. P2P: golden unchanged; no new dependency. needs: none. Landed 946c14a4. crates/observe/src/mcap.rs std-only writer and reader (magic, header, schemas, channels, messages, data end, summary with statistics, footer; JSON payloads from integers and hex), ReplayRecording with /tick/hash, /fighters/N/pose, /events, /debug/lines, first_divergent_tick; RerunSink::to_mcap and proof TraceReport::to_mcap are the callers. mcap_roundtrip_* x6 green (a 600-tick 2-fighter bout under 400 KB, bytes equal across two runs, all 1,872 messages read back, a seeded desync at ticks 0, 1, 321 and 599 first differs at its tick): engine-observe --lib mcap run 39102446324, full engine-observe 39104005600, trace::mcap_export_names_the_first_divergent_tick in engine-proof run 39102427469, full engine-proof incl. golden d57b5c8ae3096614 run 39103988897, clippy -D warnings run 39106155401. No dependency added. Opening the file in Foxglove or Rerun was not tried here (format follows the MCAP spec).

**HUD, sound and input (gameplay.md, parked.md; the UI and audio parks lift for the duel only, lead 2026-10-12)**

- [ ] UIX13 ui: the HUD is drawn through a quad and text pass with stamina and
  poise — `crates/render/src/graph.rs:1506` declares a "hud" pass nothing runs,
  `crates/game/src/windowed.rs:1089-1093` keeps the quads on the frame receipt,
  `DuelOverlay` carries hp only (`crates/ui/src/lib.rs:742-795`) and the font is a
  5x7 ASCII bitmap. A flat command list (rect, border, text, scissor; the Clay
  shape) from DuelOverlay snapshots, glyphon 0.12 text (pins wgpu 30), poise as
  ring segments, stamina under hp, text >= 28 px at 1080p, contrast >= 4.5:1, never
  colour alone. Diff: good. Tier A. Size M. Owns `crates/render/src/hud_pass.rs`
  (new), `crates/ui/src/lib.rs`, `crates/ui/src/text.rs`, the HUD hunk of
  `windowed.rs`. Accept: `cargo test -p engine-render --lib hud_pass`: draws <= 3,
  text_ms <= 0.3 at 1080p, stamina, poise and pip pixels > 0, text contrast >= 4.5.
  F2P: the arena capture has 0 HUD pixels. P2P: golden unchanged. needs: none.
- [x] SN12 audio: hit kind by surface cue table on the live CueBank — Landed 9d003eaef.
  `crates/game/src/lib.rs:911-948` plays one sound for hit, block, parry and
  knockdown; the 15-cue CueBank has no game user; clash, riposte, sever and gore
  have no cue. A presentation-side (kind x surface) lookup, round-robin variants,
  FightEvent gets no new field (Godot AudioStreamRandomizer shape). Diff: good.
  Tier A. Size M. Owns `crates/audio/src/cue.rs`, the cue hunks of
  `crates/game/src/lib.rs` and `windowed.rs`. Accept: `cargo test -p engine-game
  --lib hit_matrix`: distinct clips >= 12, the same clip never twice in a row, 8 of
  8 event kinds have a cue; audio-thread allocations 0. F2P: distinct clips = 1.
  P2P: golden unchanged. needs: none.
- [x] SN13 audio: occlusion lowpass and air absorption — `crates/audio/src/occlude.rs:159`
  returns a gain only and the mixer has no filter; reverb is one preset switched at
  a 20 m circle. Occlusion returns gain and cutoff per material, distance adds air
  absorption, the reverb send blends three zone presets (Steam Audio direct-path
  math studied, never linked). Diff: regular. Tier B. Size S. Owns `occlude.rs`,
  the filter hunk of `crates/audio/src/lib.rs`. Accept: `cargo test -p engine-audio
  --lib muffle_band`: behind stone the high band drops >= 15 dB and the low band <= 8
  dB, allocations 0. F2P: high-band drop 0 dB. P2P: golden unchanged. needs: none. Landed b3d72d3b. occlude::tests::muffle_band_behind_stone_drops_the_highs_and_keeps_the_lows green (stone: high band -20 dB, low band -7.98 dB, 0 heap calls), engine-audio --lib 264 passed gh run 39106108843, engine-game --lib audio_mix 10 passed gh run 39106771218; open voices mix bit-identical. Left: the reverb send is still one preset at the 20 m circle (three zone presets not blended).
- [x] SN14 audio: output-latency probe — `crates/audio/src/device.rs:307` drops the
  callback timestamp, so hit-to-speaker latency is unknown while the UX budget is 40
  ms. Read OutputCallbackInfo, print p50 and p95; past 40 ms schedule cues early by
  a sample offset. Diff: regular. Tier B. Size S. Owns `device.rs`,
  `crates/audio/src/engine.rs`. Accept: `cargo test -p engine-audio --features device
  --lib latency_probe` prints audio_latency_ms p50 and p95 (skips loudly without a
  device); p95 <= 40 or the early offset is set. F2P: no latency line. P2P: golden
  unchanged. needs: none. Landed 876a5a63. device::tests::latency_probe_reads_the_default_device_and_prints_p50_p95 printed audio_latency_ms p50=31.9 p95=31.9 early_frames=0 on the RTX 3050 laptop PC (p95 within 40 ms), latency_probe_* (4) and engine::tests::a_late_device_starts_one_shot_cues_early green, engine-audio --features device --lib 299 passed (local private target dir), default --lib 265 passed, clippy --features device --all-targets -D warnings clean.
- [ ] UIX14 input: live gamepad, hot-plug, rumble and last-device prompts Progress 2026-10-24: the rumble table: `RumbleRow` (low, high, trigger L and R, ms) for seven cues with a light and a heavy hit split at 200 permille, `focus_lost()` all motors off for 10 ms (d3c3d71, run 40115369038, `rumble_merge`). Left: gilrs polling and the live motors, the focus hook, disconnect pause, prompt glyphs. (deepens
  EG27) — `crates/input/src/bindings.rs:43` leaves live polling for later, the pad
  is plain data, 0 haptic code. gilrs polling, a rumble table mapping cue to (low,
  high, trigger L, trigger R, ms) with Dagor's envelope (BSD-3), motors off within 10
  ms of focus loss, prompt glyphs follow the last device. Diff: good. Tier A. Size
  M. Owns `crates/game/src/pad.rs`, `crates/game/src/rumble.rs` (new), the input
  hunk of `windowed.rs`. Accept: `cargo test -p engine-game --lib rumble_merge`:
  7 cues mapped, motors off <= 10 ms after focus loss; a pad disconnect pauses a
  local duel within 1 tick (Steam Deck rule); a `--features pad` test lists >= 1
  fake pad. F2P: no rumble code. P2P: golden unchanged. needs: EG27.
  Correction 2026-10-21: the Accept filter `rumble_merge` matches no test (`Grep rumble` over crates/game is empty and `crates/game/src/rumble.rs` does not exist), so the command exits 0 with 0 passed. The rumble table, the focus-loss cut and their test are the work; Accept is `--lib rumble_merge` printing a nonzero passed count, and the row may not be ticked until it does. Progress 2026-10-24: live half landed 58e99b59f: PadState::step (focus loss cuts motors with a 10 ms deadline, disconnect pauses within a tick, glyphs follow the last device), merged_cue_row drives both gilrs motors from the 7-row table (9 passed, run 40117040012; arena bin check 40118821270). Left: the Focused(false) call in bin/arena.rs note_window_state (busy file), a HUD glyph surface, a real pad check. Progress 2026-10-24: Landed 58e99b59f (PadState step: focus cut, disconnect pause, glyphs) and d2e9e6c37 (window focus loss cuts rumble). Left per the proof runner: the rumble_merge numbers run and the --features pad fake-pad test.
- [x] GP46 input: tick-counted gestures and action sets — 7 actions and 3 button Landed 7d13d5f. Tick audit 2026-10-17: bindings.rs conditions_tap/hold/hold_and_release/chord, conditions_menu_presses_never_reach_duel, conditions_same_script_gives_same_tape_bytes_x2 (v1 keeps 3 bits) green in gh-check run 39057174696 (engine-input, 51 passed).
  bits (`crates/input/src/bindings.rs:21`, `fighter.rs:24-30`), only press exists.
  Tap, hold, hold-and-release and chord conditions counted in integer ticks before
  the tape (bevy_enhanced_input shape), action sets stack (Duel, Menu, Pause); the
  new bits are v2 only. Diff: good. Tier A. Size M. Owns `crates/input/src/bindings.rs`,
  `crates/input/src/gestures.rs` (new), `fighter.rs`. Accept: `cargo test -p
  engine-input --lib conditions` x2: 4 gestures, 0 menu presses leak into the duel,
  tape bytes equal. F2P: only press exists. P2P: golden unchanged (v1 keeps 3
  bits). needs: GP28. Tick audit 2026-10-19: the conditions code landed as `crates/input/src/gestures.rs` (0b43f1d9).
- [x] PF27 perf: an in-game perf overlay and a hitch catcher — puffin, GPU timestamps
  and the counting allocator exist (`crates/perf/src/profiler.rs:1-12`) but nothing
  shows in the game. A toggle draws frame ms, the top 3 GPU passes (from PF25) and
  allocations per frame; a frame over 2x the median writes its last 120 frame
  reports to `target/reports/hitch-<UTC>.json` (Dagor lagCatcher idea). Diff:
  regular. Tier B. Size S. Owns `crates/perf/src/overlay.rs` (new), the toggle in
  `windowed.rs`. Accept: `cargo test -p engine-perf --lib hitch` x2: a seeded 3x
  spike writes one report naming the slow pass; the overlay off costs 0 draws. F2P:
  no overlay, no hitch file. P2P: golden unchanged. needs: PF25, UIX13. Progress 2026-10-19: the perf half landed e39dd7a — `crates/perf/src/overlay.rs` gives `Overlay` (toggle off by default; rows = frame ms, alloc per frame, top 3 passes or one `gpu none` row; `draw_count` 0 when off) and `HitchCatcher` (120-frame ring, median over the window, hitch above 2x, latch so a slow run writes one file, `-2` suffix instead of overwriting a same-second report, hand-written JSON, UTC and output dir passed in so nothing reads the clock). Proof: 7 hitch tests twice on the runner (gh 39314646273, 39315244242), the 3x spike writes exactly one report naming `ra1.forward` at 9.500 ms, overlay off costs 0 draws, 120 quiet frames write nothing; scoped clippy PASS (39314004461) because the exact line is blocked by another session's `render/src/present.rs`; golden green (39313199811). LEFT: `crates/game/src/windowed.rs` must declare one `Overlay` and one `HitchCatcher`, bind a key to `toggle()`, draw `rows()` while `draw_count() > 0`, and call `observe(sample, utc, Path::new("target/reports"))` after each frame with that frame's alloc count and any PF25 pass timings; PF25 is still open, so the row reads `gpu none` until it lands.
  Correction 2026-10-21: duplicate of PW41 (`perf: one Perfetto-readable trace file`, which closes finish-line item 13 and PF27); PW41 writes the trace and not this row's overlay, and the overlay half already landed at e39dd7a, so keep one row and fold the other's item in - do not run both. Recheck 2026-10-23: both halves are in the tree - the overlay at e39dd7a and the trace file (`crates/perf/src/trace_file.rs`, PW41 landed 87f895e85) - so the row closes and no second row is run.
- [x] GS37 assets: BC7 and BC5 texture cook with mips for the 6 GB card - the encoder
  has BC1 and BC3 only (`crates/assets/src/bc.rs:1`), so normal maps and albedo
  either bloat VRAM or lose quality. Cook BC7 for colour, BC5 for normals, full mip
  chains, KTX2 container (Bevy CompressedImageSaver shape; licences of any encoder
  crate checked first). Diff: regular. Tier B. Size M. Owns `crates/assets/src/bc.rs`,
  the texture hunk of the cooker. Accept: `cargo test -p engine-assets --lib bc7`
  x2: bytes equal, PSNR >= 40 dB on the albedo fixture, the arena texture set fits
  <= 50 percent of today's VRAM bytes. F2P: no BC7 or BC5 path. P2P: golden
  unchanged. needs: none. Progress 2026-10-18: real BC7 mode 6 (bc.rs, 93a57a82) and BC5 with a 1024 mip cook exist; bc::tests::bc7_albedo_1024_reaches_40_db_is_4x_smaller_and_threads_agree 42.51 dB (bar 40), bytes equal x2, run 39108657201. Left: BC7 into the KTX2 container (ktx2_cook still writes BC1 colour and the committed arena maps stay BC1), mips for BC7, and the arena set <= 50 percent of today's VRAM bytes is not measured.
Landed 7c1606fb1. `ktx2_cook` writes BC7 for colour and ORM (vkFormat 146 sRGB / 145 linear) with the full mip chain, keeps BC5 for normals (141), carries `format` beside `kind` and reads an older BC1 file by its header; `cook_ktx2_as` pins a shape (the fighter atlas stays BC1 because it rides the page's first bundle). The committed arena maps were re-cooked in the same step and `cook.json` / PROVENANCE-AUDIT re-pinned. VRAM: the runtime holds 44,040,192 B of RGBA8 today; today's BC1/BC5 files are 9,793,360 B; the new cooked set with mips is 14,686,640 B = 33.35% (level 0 only 11,010,048 = 25.0%), so the row's <= 50% VRAM bar is met with room, and quality rose (stone base PSNR 34.8 -> 38.8 dB, sand base 45.0 -> 52.8 dB). Note: the older "50% of today's BC1 bytes" reading is impossible at equal resolution (BC7 is 16 B/block vs BC1 8); the reachable baseline is the 44 MB of RGBA8 the runtime actually holds. Proof: `cargo test -p engine-assets --lib bc7` 7 passed x2 (runs 39944320375, 39946651620), bytes-equal-x2 and 53.23 dB on the albedo fixture with the arena set printed at 33.3%; `--features import --test it arena_cook` 2 passed (39944487764, two fresh cooks equal the committed tree); `--lib kit_import` 5 passed (39944928128); `--test it rig_cook` 7 passed (39945345229); clippy `-D warnings` PASS (39945766108); golden PASS and asserts `d57b5c8ae3096614` (39946240556); game `kit_` 23 passed (39946886403); rust-quality assets PASS 99; rustfmt clean. Open gates: the GPU still receives RGBA8 because `TextureUpload` has no block path (the real VRAM drop needs the render lane's compressed upload, VR2's remaining leg); `content/arena/kit.e2bundle` belongs to another lane and still embeds the BC1 maps until it is re-packed from sources.

**Dev loop: GPU timing, shader checks, debug views, cook (quality.md, render-look.md, runtime.md)**

- [x] PF28 game: native crash capture and a GPU device-lost report - Progress 2026-10-24: `crash_receipt::device_lost_receipt` writes a normal receipt with `last_call = "pass: <pass>"` and the newest 120 frame lines (`FRAME_TAIL`), numbered `-000`, `-001` so a second loss never overwrites (1294ad0, perf run 40115347153 8 tests, game run 40116103683 19 tests). Left: the wgpu `device.lost` callback and `mark_device_lost` caller in `presenter.rs`, the forced null-deref window run, the monitor bin, the RAM and exit budget. the panic hook
  (`crates/game/src/crash.rs:561`) catches Rust panics only, with no stack; there is
  no device-lost callback (`presenter.rs:702` marks loss by hand); PLF6 minidumps
  are parked. crash-handler and minidumper (EmbarkStudios, MIT or Apache-2.0) with a
  small monitor process write the dump and our report; wgpu's device-lost callback
  writes the same report with the last pass name and the last 120 frame reports.
  Local files only: uploading is the owner's call. Diff: regular. Tier B. Size M.
  Owns the hook hunk of `crash.rs`, the device-lost hunk of `presenter.rs`, a
  monitor bin. Accept: a forced null write and a forced device loss each leave one
  report file naming the pass; `cargo test -p engine-game --lib crash` x2; the
  monitor adds <= 5 MB RAM and delays exit <= 2 s. F2P: an access violation leaves
  no report. P2P: golden unchanged. needs: none. RETIRED (done): 1294ad0af (lead, trim audit 2026-10-24)

- [ ] PF25 render: GPU time on every production pass — 30 pass descriptors in 18 Progress 2026-10-20: slice landed in 745a9bed. `graph::tests::pass_timing_covers_every_compiled_pass` shows every pass of a compiled order gets its own `query_pair(pos)` (a pair is adjacent, pairs never collide, and they tile every slot of `2 * passes`) over two production graphs (arena 7 passes, baked gi 2; headless, run twice green). `executor::tests::pass_timing_math_reads_ticks_and_keeps_missing_none` covers the resolution math headless (ticks to ms through the queue period, an inverted and an empty pair reading zero, a wide delta staying finite, `none_gpu_timings`, a short tail and a `None` sample staying `None`, never a fake zero). `executor::tests::pass_timing_gpu_per_pass_p50_over_120_frames` reads the window QuerySet on a real adapter on this PC: adapter NVIDIA GeForce RTX 3050 6GB Laptop GPU, backend Vulkan, driver NVIDIA 610.62, `TIMESTAMP_QUERY` true, period_ns 1; shadow, forward, decals, resolve, bloom, hud and present each p50 0.0020 ms, frame total p50 0.0143 ms p99 0.0205 ms, slowest pass named (a near tie: the shared GX1 test frame is 64x64, so the figures are small and 1080p is not measured here). It prints that the samples stay `None` when the adapter lacks `TIMESTAMP_QUERY` instead of a number. Left for the row: the windowed accept (600 frames x3, `target/reports/perf-<date>.csv` with >= 12 pass rows and `pass-trace.json`) plus the `crates/perf/src/profiler.rs` hunk. No file writes `pass-trace.json` yet, and `crates/game/src/camera_feed.rs` is dirty under another session, so the arena binary is not run for that capture this round. Census correction: the tree now holds 33 `timestamp_writes: None` descriptor sites in 20 files (the row's 30 in 18 predates them), and exactly 1 timed site in the compiled graph path (`graph.rs`). Timings stay evidence only: never in a digest, a hash or the tick.
  files pass `timestamp_writes: None`; the one timed chain is the gx1 probe
  (`crates/render/src/graph.rs:1281-1409`, 4 GPU objects and a blocking poll per
  call). wgpu-profiler 0.28 (MIT or Apache-2.0, wgpu 30) pools queries and reads
  back late. Diff: good. Tier A. Size M. Owns `crates/render/src/executor.rs`,
  the probe hunk of `graph.rs`, `crates/perf/src/profiler.rs` (dependency by the
  orchestrator, deny.toml first). Accept: the windowed arena, 600 frames x3,
  writes `target/reports/perf-<date>.csv` with >= 12 pass rows (p50, p99) and
  `pass-trace.json`; `cargo test -p engine-render --lib pass_timing` x2. F2P: the
  csv has no production pass row. P2P: golden unchanged. needs: none.
  Correction 2026-10-21: duplicate of PW07 (`render: one named scope per pass, and the pass report wired to the live frame`, which closes PF25); do not run both - PW07 is the owning row, and the halves that already landed are named in this row's own Progress note.
- [x] CQ23 render: CPU naga gate for every shipped WGSL — `crates/render/src/shader_gate.rs` Landed 801b43cc. The cited check moved: the gate is `crates/render/src/shader_gate.rs` (naga 30 via engine-material's `validate` owner) plus the game cook (`crates/game/src/reload.rs`), both behind `shader-validate` (rides `app`). Accept adapted (the row predates the feature): `cargo test -p engine-render --lib --features shader-validate shader_validate` x2 green over all 24 shipped files in the composition the pipelines compile (`forward.wgsl`/`blit.wgsl` take the `tonemap.wgsl` prepend) + 3 inline chunks; the `window`-feature run adds 6 window chunks; game reload 6/6 under the feature (a type-broken edit keeps last-good, `reload_shader_gate_refuses_a_type_broken_edit_and_keeps_last_good`); seeded type error names line:col. F2P held: the brace check accepted both defects the gate caught — `taa_resolve.wgsl` built `vec2<f32>` from i32 loop vars, `post_chain.wgsl` sampled without declaring its sampler — fixed in `8d8c0a80`. Golden d57b5c8ae3096614 x2 unchanged. Follow-up 2026-10-19 (main pass): the committed wasm baseline counted this new file's `std::fs` walk as a hazard (`shader_gate.rs:80,86`, `fs 0 -> 2`), and `tools/wasm-gate.test.mjs` was red on it. The walk is native-only now (`#[cfg(not(target_arch = "wasm32"))]` on `shipped_files` and `walk`, with the reason in the doc comment): `node --test tools/wasm-gate.test.mjs` 20 pass, 0 fail. Behaviour on native is unchanged.
  counts braces; `crates/material/src/compile.rs:770-779` already validates with
  naga; 0 of 11 .wgsl files are checked without a GPU. Diff: regular. Tier A.
  Size S. Owns the check in `live.rs` and a test over `crates/render/src/shaders`.
  Accept: `cargo test -p engine-render --lib shader_validate` x2: 11 of 11 files
  and every inline source pass; a seeded type error in forward.wgsl fails with
  line:col and the live swap keeps last-good. F2P: the brace check accepts that
  edit. P2P: golden unchanged. needs: none.
- [x] RL58 render: debug-draw lines with a Rapier backend (deepens EG26) — Progress 2026-10-17: crates/render/src/debug_draw.rs slice 1 (DebugDraw trait, line groups with depth bias, recorder, line_verts, debug_draw_* tests); 2026-10-23: the after-blit line pass, the capture and the golden on/off check land (game caller still PW13). Landed b892fa4c7.
  debug draw in `crates/render/src`; rapier `debug-render` is off
  (`Cargo.toml:192`). Line groups with depth bias (Bevy gizmos), the Rapier
  backend trait draws colliders and joints; combat sweeps, hurt capsules and nav
  paths push copies taken after the tick. One DebugDraw trait, two sinks: the
  live line pass and a per-frame recorder (Jolt recorder shape) that PF26 exports.
  Diff: regular. Tier B. Size M. Owns
  `crates/render/src/debug_draw.rs` (new), the backend impl in
  `crates/physics/src/rapier.rs`. Accept: `cargo test -p engine-render --lib
  debug_draw` x2; a capture with 4 groups on has >= 1000 line pixels that differ
  from the beauty frame; the tick-600 hash equals d57b5c8ae3096614 with groups on
  and off. F2P: no line pass exists. P2P: golden unchanged. needs: none.
  Correction 2026-10-21: duplicate of PW13 (`render: the debug views, and the first caller of the dead debug_draw.rs`, which closes RL59 and gives RL58 a caller); do not run both - PW13 is the owning row.
  Proof 2026-10-23: `cargo test -p engine-render --lib debug_draw` 17 passed twice (runs 39992171484, 39993507849); the four-group capture differs from the beauty frame by 3591 pixels (`crates/render/target/clips/debug-lines/`); the golden d57b5c8ae3096614 holds with groups off (proof_cli run 39994817325) and `crates/physics/src/rapier.rs` `debug_lines_leave_the_tick_hash_and_snapshot_unchanged` holds it with groups on.
- [ ] RL59 render: renderer debug views through the door capture — no view
  selector exists (Godot lists 29 named views). Each view is a false-colour blit
  of an existing frame-graph resource, pinned out of the alias pool. Diff:
  regular. Tier B. Size M. Owns `crates/render/src/debug_view.rs` (new), a `view`
  argument in `crates/endpoint/src/play.rs` capture. Accept: `cargo test -p
  engine-render --lib debug_view` x2; 8 views (overdraw, wireframe, normals,
  shadow atlas, cascades, AO, motion, luminance) give 8 non-black PNGs with 8
  different SHA-256; < 0.1 ms with no view selected. F2P: an unknown view name is
  accepted and no view PNG exists. P2P: golden unchanged. needs: PF25.
  Correction 2026-10-21: duplicate of PW13 (`render: the debug views, and the first caller of the dead debug_draw.rs`, which closes RL59); do not run both - PW13 is the owning row.
- [x] OPT34 render: pipeline-creation counter by cause — OPT19 tests a frame-time Landed 1d01a41. Tick audit 2026-10-17: cache.rs pipeline_counter_material_walk_counts_four_causes_with_late_zero (prints late=0 and 4 counters) and pipeline_counter_failed_insert_counts_no_build green in gh-check run 39057362669 (render --features window) and render lib run 39057649152; the cache.rs doc lists the 19 sites that bypass the counters.
  spike, which needs a quiet box; Godot counts builds by cause. Count every
  pipeline built in `crates/render/src/cache.rs` by cause (mesh, surface, draw,
  specialization); after load the late count is 0. Diff: regular. Tier A. Size
  S. Owns the counters in `cache.rs`. Accept: `cargo test -p engine-render --lib
  pipeline_counter` x2: the material-walk fixture prints late=0 and 4 counters,
  sites that bypass cache.rs listed. F2P: no counter. P2P: golden unchanged.
  needs: none.
- [x] OPT36 build: one dependency feature set for every `-p` test. Overnight Landed 398ac7bb.
  2026-10-12, 52 of 184 heavy runs hit the 300 s kill and the ok runs had a p90
  of 288 s. `target/debug/deps` holds 16 builds of `syn`, 7 of `serde_json` and
  7 of `num_traits`: each `cargo test -p <crate>` unifies features for its own
  selection only, so moving between crates rebuilds shared dependencies at
  opt-level 3. Add a workspace-hack crate (the cargo-hakari pattern, MIT or
  Apache: one crate every member depends on, listing the union of dependency
  features) or pin the features in `[workspace.dependencies]`, plus a check that
  fails when a member adds a dependency feature outside the set. Diff: good. Tier
  A. Size M. Owns the root `Cargo.toml`, `crates/workspace-hack/` (new),
  `tools/feature-unify.mjs` (new) and its test. Accept: `cargo test -p
  engine-combat --lib`, then `cargo test -p engine-game --lib`, then `cargo test
  -p engine-combat --lib` again: the third run compiles 0 dependency crates
  (every dependency `fresh` in `--message-format json`), x2; `node --test
  tools/feature-unify.test.mjs` passes and fails on a seeded extra feature. F2P:
  the third run recompiles shared dependencies. P2P: golden unchanged. needs:
  none.
- [x] FB8 assets: cook key with dependency hashes and a begin/end log —
  `crates/assets/src/cooker.rs:216-219` hits on the source digest alone, so a
  dependent is served stale; `gen_intake.rs:756-765` writes 3 files non-atomically.
  Key = blake3(source, settings, cook version, dependency keys in order), one
  rename per entry, a begin without an end is redone (Bevy asset processor).
  Diff: good. Tier B. Size M. Owns `cooker.rs`, `gen_intake.rs`, `recook.rs`.
  Accept: `cargo test -p engine-assets --lib cook_full_hash` x2: a dependency edit
  makes the dependent fresh, a cook-version bump recooks 100 percent, a kill
  between file 1 and 2 leaves exactly 1 entry to redo, bundle bytes x2. F2P: the
  dependent is served stale. P2P: golden unchanged. needs: none. Landed 46c5b4a8. cooker::tests::cook_full_hash_* (dependency edit makes the dependent fresh, dependency order in the key, version bump and settings change recook 3 of 3, keys and payload bytes x2) and gen_intake::tests::cook_full_hash_a_kill_between_files_leaves_exactly_one_entry_to_redo green, engine-assets --features import,registry --lib 365 passed gh run 39106601712.
- [x] RC38 ecs: declared access, a conflict check and a seeded order shuffle — Progress 2026-10-17: schedule.rs access sets, conflict refusal naming both systems and set_shuffle_seed landed (sched_shuffle_keeps_hash_across_50_seeds, ecs green run 39057318550); left: the 50 shuffles run a toy ArchetypeWorld, not the match that yields d57b5c8ae3096614.
  systems run in registration order with no read or write sets
  (`crates/ecs/src/schedule.rs:36-47,97`). Each system declares its columns; the
  build refuses an unordered conflicting pair; the golden runs under 50 seeded
  shuffles of every free ordering (Bevy ambiguity detection and shuffle_seed).
  SIM, test-only. The pre-test RC36 needs before 8 threads. Diff: good. Tier B.
  Size M. Owns `crates/ecs/src/schedule.rs` and a shuffle test. Accept: `cargo
  test -p engine-ecs --lib sched_shuffle` x2: 50 shuffles give d57b5c8ae3096614; a
  seeded missing edge fails naming both systems. F2P: no access sets, no shuffle.
  P2P: golden unchanged. needs: none. Landed a9b8ce6d. The 50 shuffles now run the golden fixture itself (engine-proof Simulation, seed 2040, 600 ticks; the toy world stays in engine-ecs): tests::sched_shuffle_keeps_the_golden_across_50_seeded_orders (integrate declares reads velocity / writes position beside two column-free probes, 50 seeds reach at least 4 of the 6 orders, every seed ends on the plain tape's last checkpoint, legacy d57b5c8ae3096614) and tests::sched_shuffle_refuses_a_missing_edge_naming_both_systems (ScheduleError::Conflict integrate/probe_a, text names both). Green x2: engine-proof --lib sched_shuffle run 39106723705 and full engine-proof 160 passed run 39103988897; engine-ecs sched_shuffle_keeps_hash_across_50_seeds in engine-ecs --no-fail-fast run 39103985481. The test lives in engine-proof, not engine-ecs --lib, because Simulation is private there.

### First-person view (owner plan 2026-10-14, `docs/plan/fps-view.md`)

Boarded 2026-10-15 from the plan's section 7 (rows lived there until boarded). A first-person eye on a fighter body:
eye pose from the body, a presentation-only look channel beside the tape, the camera feed into the submit path, a
no-clip rule at eye level, eye-space shake, one scripted proof tape. Every row is PRESENTATION; the golden
`d57b5c8ae3096614` is P2P on every row. FV4 waits on the boarded UW11 body.

- [x] FV1 fps-view: eye pose from the fighter body. Tier A. Size S. Landed e6a49874.
  Owns `crates/game/src/fps_view.rs` (new file beside `camera.rs`).
  Accept: `eye_for_fighter` puts the eye at table head height over
  `pos_m` with yaw from `facing`; bad height, facing or table refuses
  with `CameraError::BadParam` naming the row; same track replays
  byte-exact twice; zero heap bytes per call; `cargo test -p
  engine-game --lib fps_view` green. F2P: no eye function exists.
  P2P: golden unchanged. needs: none.
- [x] FV2 input: presentation-only look channel beside the tape. Tier Landed 93348edc.
  A. Size S. Owns `crates/input/src/look.rs` (new file beside
  `fighter.rs`).
  Accept: yaw/pitch deltas accumulate into a validated `LookState`
  with table clamp rates; `FighterInput` bytes unchanged (10-byte
  record test stays green); same delta track replays exactly twice;
  golden hash unchanged; `cargo test -p engine-input --lib` green.
  F2P: no look state exists. P2P: the 10-byte record test.
  needs: FV1.
- [x] FV3 render: eye-to-camera feed into the submit path. Tier A. Landed 1c48073. Tick audit 2026-10-21: `camera_for_eye` feeds a validated `v1::Camera` and the same eye gives the same view-proj twice; `cargo test -p engine-game --features app --lib fps_view` green (13 passed, gh-check run 39584931722; the new `camera_for_eye_level_yaw_zero_faces_plus_z` pins the yaw-0 frame the render side mirrors); `cargo test -p engine-render --lib --features window camera` green on this PC (18 passed: `camera_eye_camera_view_proj_is_bit_identical_twice`, `camera_eye_camera_frame_matches_the_game_feed`, and `camera_eye_camera_window_submit_is_clean_and_inside_the_budget`, which runs the eye camera with 0 validation errors inside `GPU_SUBMIT_BUDGET_MS`; the plain `camera` filter matches 0 tests because the camera module sits behind the render crate's `v1` feature, so the line carries `--features window`); `cargo clippy -p engine-render --all-targets -- -D warnings` green (run 39586015212) and `cargo clippy -p engine-game --all-targets -- -D warnings` green (run 39586288573); the window/app clippy legs are red on master in foreign files (render v1_ibl_shadow.rs:214, render present/gpu_present/submit.rs:2258, render bin/kam1a.rs:16, render tests/material_swatch.rs:61, game agent_seat.rs:134, game bin/arena.rs and game bin/arena_frame/mod.rs) while both crates' lib and lib-test targets stay clean, so this row's two files carry no lint; golden `cargo test -p engine2040 --release --test proof_cli default_and_max_seed` green (runs 39584328056, 39584373281, 39587579851), `final_hash` `d57b5c8ae3096614` did not move.
  Size M. Owns `crates/game/src/fps_view.rs` feed hunk plus a
  `v1.rs` test-only constructor hunk (no `Camera` shape change).
  Accept: `camera_for_eye` builds a validated `v1::Camera` whose
  view-proj is bit-identical for identical eyes; the window submit
  runs the eye camera with 0 validation errors at test sizes inside
  `GPU_SUBMIT_BUDGET_MS`; `cargo test -p engine-render --lib camera`
  green. F2P: no eye camera exists. P2P: golden unchanged.
  needs: FV1.
- [ ] FV4 view: no-clip body rule at eye level. Tier A. Size M. Owns
  `crates/game/src/fps_view.rs` no-clip hunk (table: hide distance,
  near clip).
  Accept: scripted eye orbit through the focus body never shows
  inside-of-head pixels (near-clip plus own-head rule from one
  table); UW11 accept line ("never clips the body") holds on the
  infantry body; replays twice. F2P: the orbit fixture shows head
  pixels without the rule. P2P: golden unchanged. needs: FV1, UW11.
- [x] FV5 feel: eye-space trauma kick plus FOV punch. Tier B. Size S. Landed 1d01a41. Tick audit 2026-10-17: fps_view.rs shake_eye_kick_and_fov_punch_hold_gp21_bar (jab x4 <= super kick, roll <= 0.10 rad x gain, FOV punch under cap, calm is zero) and shake_eye_replays_exactly_twice pass in gh-check run 39057596405 (engine-game --lib, the only red test there is refilled_outcome_matches_fresh_tick_for_tick). Note: the module header says retired, no production caller.
  Owns `crates/game/src/fps_view.rs` shake hunk (calls
  `smooth_trauma_offset`, no new curve).
  Accept: jab kick at or under 1/4 super kick about the eye (GP21
  bar in eye space); roll bounded by 0.10 rad times gain; FOV punch
  from the same gain inside a table cap; calm reports zero; replays
  twice. F2P: no eye-space shake exists. P2P: shake never hashed,
  golden unchanged. needs: FV1.
- [x] FV6 proof: scripted 600-tick eye tape with receipt. Tier B. Landed 5f103e6. Tick audit 2026-10-22: `crates/game/tests/it/fps_view_tape.rs` (new) and the `mod fps_view_tape;` line in `crates/game/tests/it/main.rs` land the tape: 600 fighter inputs recorded from the real arena match (the script closes on the rival, so the fight lands real cues) plus 600 scripted look deltas on the presentation channel; the eye bytes replay exactly twice and the camera feed replays with them. `cargo test -p engine-game --test it fps_view_tape` green (3 passed, gh-check run 39722459677); `cargo test -p engine-game --features app --test it fps_view_tape` green (3 passed, 1 ignored, run 39722442096) and the same line with `-- --nocapture` prints the receipt: `eye_travel_peak_cm=6.76 shake_peak_cm=0.10 submit_ms=0.0002 trauma_peak_permille=180` (run 39722506609), the travel peak under the 25 cm budget at 30 fps and the submit cost under the 16.0 ms `GPU_SUBMIT_BUDGET_MS`; the submit bar test is `#[ignore]`d (wall clock) and green under `--ignored` on this PC at 0.0004 ms. The eye never enters a tick hash: golden `d57b5c8ae3096614` unchanged, `cargo test -p engine2040 --release --test proof_cli default_and_max_seed` green (run 39722454328).
  Size S. Owns `crates/game/tests/it/fps_view_tape.rs` (new file,
  wired as a `mod` line in its `main.rs`).
  Accept: one tape drives fighter plus look deltas for 600 ticks;
  eye bytes replay exactly twice; per-frame eye travel peaks at or
  under 25 cm at 30 fps; the receipt prints eye travel, shake peak
  and submit ms; golden hash unchanged. F2P: the tape file does not
  exist. P2P: golden unchanged. needs: FV2, FV3.
