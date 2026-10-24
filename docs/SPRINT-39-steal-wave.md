### Steal wave (2026-10-24, lead-merged from nine hunts)

Hunters read the donor code and ours; every file:line below was checked on 2026-10-24. Render,
feel and anim rows are presentation only: the golden `d57b5c8ae3096614` does not move. Rows that change the fight land on rules v3 (SW27, lead 2026-10-24): an older tape and
the golden never move, so no row re-pins anything; a row's own text that still says the golden moves
means its v3 hash. Rows that own
`crates/game/src/fighter_look.rs` wait for the group C camera framing to land.
Not taken: the nightly-only build flags (`no-embed-metadata`, `-Zcache-proc-macros`: a nightly
toolchain on every agent), `--jobs-frontend` (rustc 1.97.1 has no such flag), cranelift (slower
600-tick tests), Feel / A* / PuppetMaster / Easy Save / Master Audio (already ours: hit stop, ORCA
crowd, partial ragdoll, versioned save slots, voice limits and ducking). Ideas with no code read yet:
octahedral impostors for the far crowd, a probe grid for baked GI, an aim chain for head and weapon.

#### Render speed and the potato tier

- [ ] SW1 render: the sun map grows three cascade layers, and the far two re-render less often -
  `CascadeCadence` (`crates/render/src/shadow_cascade.rs:620`, landed 4c8bc309d) drives them.
  Today the live sun pass has one map: `present/gpu_present/sun_shadow.rs:627 encode` (one `self.view`,
  `LoadOp::Clear(1.0)` at :649, one `set_viewport` at :660) held as `presenter.rs:342 sun_sub`; the
  `for i in 0..3` at `sun_shadow.rs:711` is the `#[cfg(test)] fn sun_uv_depth` matrix loop, not cascades.
  Do: `depth_or_array_layers = layers` from `present/gpu_present/sun_shadow.rs:186 cascade_depth_desc`,
  `encode` takes `frame: u64` and loops `0..layers` with `depth_slice: Some(cascade)` so an undrawn
  layer is never cleared; `record` (`sun_shadow.rs:587`) gains a not-due arm that returns 0 us and
  writes neither uniform (its `false` arm at :594 calls `disable_forward` and would unshadow a still
  scene); `force_all()` on the resize/tier hook `present/gpu_present/quality_apply.rs:24` and on a
  sun move. Threads land at `present/gpu_present/submit.rs:1143` and `present.rs:342` (lead's files).
  Donor: https://github.com/google/filament/blob/main/filament/src/ShadowMapManager.cpp
  (`updateCascadeShadowMaps`, `ShadowMap::getScissor`) for the per-layer scissor; cadence is ours.
  Tier B. Size M. Owns crates/render/src/present/gpu_present/sun_shadow.rs,
  crates/render/src/shadow_cascade.rs, crates/render/src/quality.rs.
  Accept: `cargo test -p engine-render --features window --lib sun_shadow` (5 tests today, stays >0)
  plus a new cadence proof: 3 layers, cascade 2 drawn once over 4 frames, skipped frames' uniform
  bytes byte-equal, layer depth kept.
  F2P: `sun_layers_cadence_keeps_the_uniform_of_a_skipped_cascade` and `cascade_two_draws_once_in_four`.
  P2P: `cargo test -p engine-render --lib cascade_cadence` (3 tests) stays green; golden untouched.
  needs: none (not QA4's split count, not RL35's static/dynamic caster split).

- [ ] SW2 render: the main pass and the sun pass stop paying one CPU draw per mesh - they call
  `draw_indexed_indirect` from the args `gpu_cull` already writes. Today the hook is prose only:
  `crates/render/src/gpu_cull.rs:10` "until it lands no", `lib.rs:201`, while every pass still calls
  `draw_indexed` (`present/gpu_present/sun_shadow.rs:685`, `present/gpu_present/submit.rs:1118`); the
  5-u32 block is built at `crates/render/src/draw.rs:316 indirect_args_5`. Add a shadow-args block
  (one cull, two consumers) and `detail_cull_permille` on `crates/render/src/quality.rs` so sub-pixel
  geometry never reaches the vertex stage.
  Donor: https://github.com/ellioman/Indirect-Rendering-With-Compute-Shaders/blob/master/Assets/
  Shaders/02_CullingCS.compute (`IsVisibleAfterDetailCulling`, `_ArgsBuffer`, `_ShadowArgsBuffer`).
  Tier B. Size M. Owns crates/render/src/gpu_cull.rs, crates/render/src/draw.rs,
  crates/render/src/present/gpu_present/submit.rs.
  Accept: `cargo test -p engine-render --features window --lib indirect` >0 tests, and a GPU frame
  whose args block reads count>0 for 7 of 8 meshes with 1 culled, detail cull dropping >=1 mesh.
  F2P: `indirect_draw_reads_the_gpu_cull_args` (draws only when count>0).
  P2P: `cargo test -p engine-render --lib gpu_cull` stays green; no CPU draw list change.
  needs: none (the shadow-args half wants SW1's layers). Progress 2026-10-24: Landed 13ad5e7ae: gpu_cull args blocks (main + shadow), draw_instances, detail cull per tier; run 40119138069 green, 3 tests. Left: the --features window --lib indirect GPU frame (count>0 for 7 of 8).

- [ ] SW3 render: fighters behind the sand berm and the wall stop being drawn at all - a real Hi-Z
  pyramid replaces the single-wall hack. `crates/render/src/shaders/cull.wgsl:61` binds
  `array<vec4<f32>, 1>` and `:64 fn hiz_hidden_wall` tests one occluder plane; `HizPyramid` and
  `run_hiz_on_device` exist only in the comment at `cull.wgsl:15-16` (grep: 2 hits, 0 definitions).
  Do: `hiz_reduce.wgsl` 2x2 min-reduce per mip, binding 5 becomes `texture_depth_2d` + mip count,
  two-pass (cull against last frame's pyramid, then re-cull after depth).
  Donor: https://github.com/Frizi/dovetail/blob/main/shaders/depth_reduce.wgsl (`depth_reduce`,
  `workgroup_size(16,16)`; the file notes wgpu has no single-sample min reduce).
  Tier B. Size M. Owns crates/render/src/gpu_cull.rs, crates/render/src/shaders/cull.wgsl,
  new crates/render/src/hiz_pyramid.rs + shaders/hiz_reduce.wgsl.
  Accept: `cargo test -p engine-render --features window --lib hiz` >0 tests; numbers: log2(720)=10
  mip levels, the cull picks a mip from screen extent, a wall hides 4 of 8 fighters on the CPU oracle.
  F2P: `hiz_pyramid_matches_the_cpu_oracle_across_a_depth_step`.
  P2P: `cargo test -p engine-render --lib gpu_cull` stays green; the flag rule is unchanged.
  needs: SW2.

- [x] SW4 render: the web/GL2 tier hides blocked fighters with no depth prepass and no GPU readback -
  a CPU software Hi-Z buffer beside the existing frustum cull. Today the only occlusion path is GPU
  compute (`cull.wgsl`), and `draw.rs:764 cull_lod` is frustum-only; grep "occluder|cpu occl" over
  crates/render = 0 hits.
  Do: occluder meshes rasterized into a small depth buffer, max-depth mip chain to 1x1, AABBs tested
  against 4 parent texels; `draw::build` takes an optional `&OcclusionBuffer`.
  Donor: https://github.com/godotengine/godot/blob/master/servers/rendering/
  renderer_scene_occlusion_cull.cpp (`HZBuffer::resize`, `update_mips`, `CHECK_OFFSET`).
  Tier B. Size M. Owns new crates/render/src/occl_cpu.rs, crates/render/src/draw.rs.
  Accept: `cargo test -p engine-render --lib occl_cpu` >0 tests; numbers: 256x144 buffer -> 9 mips,
  verdicts identical to `cull_cpu_flags_into` on a wall scene, 0 GPU calls.
  F2P: `occl_cpu_verdicts_match_the_gpu_flag_rule_on_a_wall`.
  P2P: `cargo test -p engine-render --lib draw` stays green; no GPU path change.
  needs: none. Landed 81f85fac1: CPU software Hi-Z in occl_cpu.rs (256x144 view depths, 9 max-depth mips, AABB vs 4 parent texels), cull_lod takes Option<&OcclusionBuffer>. Left: the potato-tier caller (nothing calls the buffer yet) and its 6 clippy lints (the clippy agent has them).

- [ ] SW5 render: on the potato tier the eight torches are summed once per vertex, not per pixel -
  `crates/render/src/torch_light.rs:24 TORCH_MAX = 8` and `shaders/forward.wgsl:33` still run the
  8-torch loop per fragment (`cluster.wgsl:8` only masks empty cells). Add a vertex-lighting variant
  behind a `quality.rs` knob: `torch_lighting: Pixel | Vertex`, kept on `Quality::Potato` only.
  Donor: https://github.com/godotengine/godot/blob/master/servers/rendering/renderer_rd/shaders/
  scene_forward_vertex_lights_inc.glsl (`light_compute_vertex`, `roughness_to_shininess`: 4 FMAs
  instead of a special-function exp2 at 0.25x rate, exponent range shrunk so mediump holds).
  Tier B. Size S. Owns crates/render/src/shaders/forward_torch_vertex.wgsl (new),
  crates/render/src/quality.rs.
  Accept: `cargo test -p engine-render --features window --lib torch` >0 tests; numbers: 8 lights
  evaluated at 4 vertices per card instead of per fragment, highlight error under 1% on a flat wall.
  F2P: `torch_vertex_sum_matches_the_pixel_loop_within_one_percent`.
  P2P: `cargo test -p engine-render --lib torch_light` stays green; pixel path untouched.
  needs: none.

- [ ] SW6 render: the potato tier stops paying 8 bytes per HDR pixel - one format per tier instead of
  one for all. Today `crates/render/src/present/gpu_present.rs:14 HDR_FORMAT = Rgba16Float` for every
  tier and `quality.rs:98 Knobs` has no format row.
  Do: `hdr_format`/`normal_format` per tier; potato gets `Rgb10a2Unorm`, mid `Rg11B10Uf`.
  Donor: https://support.arm.com/documentation/101897/0304/Buffers-and-textures/
  Texture-sampling-performance (Arm GPU Best Practices: highp samplers can be half the speed of
  mediump; RGB10_A2/RGB9_E5 instead of FP16/FP32 targets for bandwidth).
  Tier B. Size S. Owns crates/render/src/present/gpu_present.rs, crates/render/src/quality.rs.
  Accept: `cargo test -p engine-render --features window --lib quality` >0 tests; numbers: at 1280x720
  the HDR target drops from 7.4 MB (Rgba16Float) to 3.7 MB (Rgb10a2), bytes per tier pinned in a test.
  F2P: `hdr_format_follows_the_tier` and `potato_hdr_target_is_half_the_bytes`.
  P2P: `cargo test -p engine-render --lib quality` stays green; golden untouched.
  needs: none.

- [x] SW7 render: smoke and dust stop cutting a hard line across the sand - soft depth-faded particle
  cards. The math and the seam already exist but the live path feeds them zeros:
  `crates/render/src/particles_gpu.rs:1578` writes `[0u8; 64]`, and the comment at :1570 says it
  "reads a constant zero gap until the closing slice feeds scene depth". Shader side is ready:
  `shaders/particle.wgsl:57 draw_soft_fade`, `:19 fade_range`, `:50 scene_depth` at group(1)
  binding(0); `particles_gpu.rs:968 SOFT_FADE_RANGE_M = 0.3`. Do: bind group 1 gets the frame depth
  view, the fragment computes the gap from `textureLoad` and `fade_range = 0.3`, the zeroed block
  goes away.
  Donor: none needed - the seam is in-repo; the shader shape follows the Fab/Unity soft-card rule
  (fade = clamp(gap / range)).
  Tier B. Size S. Owns crates/render/src/particles_gpu.rs, crates/render/src/shaders/particle.wgsl.
  Accept: `cargo test -p engine-render --features window --lib particles_gpu` (35 tests today, >0);
  numbers: a card 0.15 m into the sand reads fade 0.5, at 0.3 m reads 0, at 0 reads 1.
  F2P: `particle_draw_binds_the_scene_depth_and_fades_at_the_range`.
  P2P: `cargo test -p engine-render --lib soft_fade` (5 tests) stays green; golden untouched.
  needs: none. Landed 6fad56434: particle cards fade by draw_soft_fade over the packed 0.3 m range (run 40118612090, 36 passed).

#### Feel and juice (lurk hunt, hit-stop finding)

- [ ] SW20 combat: a press inside hit-stop is buffered - `core_step_into` returns before
  `start_actions` (`crates/combat/src/fight.rs:864` before `:870`), so a frozen tick's press never
  reaches `attack_buffer.push` (`fight.rs:1170`) and the held clock (`fight.rs:885`) ages nothing;
  queue the tick's attack bits for a mid-swing fighter before the hitstop decrement.
  Donor: mattsumi/blue-archive-fighting-game `scripts/fight/fighter.gd` `_record_presses`
  (BUFFER=7, called before its hitstop return). Tier B. Size S. Owns `crates/combat/src/fight.rs`,
  `crates/arena/src/heft.rs` tests.
  Accept: `cargo test -p engine-arena --lib a_press_inside_hitstop` = 1 test un-ignored: `buffer=1`
  on frozen tick 31 and a second swing starting tick 36 (today `buffer=0`, no swing, run 40115608178).
  F2P: `heft::tests::a_press_inside_hitstop_is_kept_and_fires_after_the_freeze`
  (`crates/arena/src/heft.rs:598`, today `#[ignore = "BUG: ..."]`): the fix drops the ignore and
  asserts the second swing instead of the tick-for-tick baseline equality.
  P2P: `cargo test -p engine-combat --lib feel`; the golden `d57b5c8ae3096614` moves (sim).
  needs: SW27 (lead 2026-10-24: lands on rules v3 only; v1 and v2 tapes replay byte-equal, the golden d57b5c8ae3096614 never moves; the v3 tape hash is pinned in this row's test).
- [x] SW21 game: a blow throws the view, not the world - extend the landed QL8 rotation shake
  (`crates/game/src/camera_feed.rs:589` `RotationShake`, `:633` `shake_rotation_offset`): one shared
  cap (`SHAKE_ROTATION_CAP_DEG`, `:580`) becomes a separate tilt budget for yaw+pitch and a roll
  budget (Juicee 1.6 deg / 0.9 deg), and a directional recoil `0.6*strength*exp(-5*p)` along the hit
  direction so the aim kicks away from the blow. What is new: the split budgets, the recoil term.
  Donor: Kelpekk/Juicee `addons/juicee/effects/shake_3d_effect.gd` `_apply`. Tier B. Size S.
  Owns `crates/game/src/camera_feed.rs` (shake hunks only).
  Accept: `cargo test -p engine-game --features app --lib shake` >0 passed, new assert: a +X hit
  tilts yaw away from +X, every channel stays within 1.6 deg tilt / 0.9 deg roll, and the pose is
  exactly rest at the end of the window.
  F2P: no directional input today (`shake_rotation_offset` takes `trauma_permille, tick` only);
  new `impact_shake(dir, strength, tick)`.
  P2P: `--lib camera_feed`; `ql8_proof_capture_runs_with_shake_off` (`:2384`) stays byte-equal when
  the recoil term is zero.
  needs: QL8 (done). Landed 9b9f93ea8: split tilt/roll shake budgets and a directional aim recoil (run 40119490394, 3 tests).

- [x] SW22 game: the finisher lands and the screen freezes white - a 4-tick beat plus a full-screen
  white overlay at 0.9 alpha that fades over 5 ticks after the freeze starts, on a killing blow;
  today the only white is the body tint (`crates/game/src/fighter_look_flash.rs:41`) and
  `HIT_FLASH_TICKS` (`crates/endpoint/src/play/play_session.rs:1161`) is a receipt disc.
  Donor: Kelpekk/Juicee `addons/juicee/effects/freeze_frame_effect.gd` `_apply` (0.06 s, flash 0.9,
  fade 0.08). Tier B. Size S. Owns new `crates/game/src/finisher_beat.rs` + one `mod` line.
  Accept: `cargo test -p engine-game --features app --lib overlay` >0 passed (today
  `duel_overlay_tracks_live_hp_and_pips`, run 40115608178 style), new
  `finisher_overlay_flashes_white_and_fades`: overlay alpha >0 on the beat tick, exactly 0 by beat+9.
  F2P: no screen-wide beat exists; the new module's `finisher_overlay_flashes_white_and_fades`.
  P2P: `--lib camera_feed`, golden unchanged (presentation only).
  needs: none. Landed fdedfdb8f: finisher beat, 4-tick freeze, 0.9 white overlay fading in 5, clear on beat+9 (run 40118412192, 3 tests).

- [x] SW23 game: a hit goes overbright and blinks - `flash_material`
  (`crates/game/src/fighter_look_flash.rs:41`) only cuts G/B by 0.10/0.20 and lifts exposure 1.45x
  over `flash_pulse = (left/15)^2` (`:31`, `FLASH_TICKS 15` at `crates/game/src/fighter_look.rs:685`),
  so the body can never read as white. Add a white-bias overbright term (Juicee boost 2.0 on the lit
  channels, albedo detail kept) and a blink count with `pow(decay, i)` for a multi-hit burst.
  Donor: Kelpekk/Juicee `addons/juicee/effects/flash_effect.gd` `_apply`. Tier B. Size S.
  Owns `crates/game/src/fighter_look_flash.rs`.
  Accept: `cargo test -p engine-game --features app --lib flash` = 3 tests (2 today):
  `flash_material_warms_lifts_and_stays_valid`, `flash_pulse_is_short_and_bright_first`, plus one
  asserting exposure > 2.0 at the peak and a monotone fall to 1.0.
  F2P: `flash_pulse` peaks at exposure 1.45 and `base_color` never rises above the base (`:44-48`).
  P2P: `--lib fighter_look`; RL57's particle light hunk is untouched.
  needs: none. Landed fd02d3570: overbright decaying hit flash + blink echo; run 40123786335 green, 5 tests.

- [x] SW24 combat: every feel number in one validated table - the presentation literals move beside
  `FEEL_V1` (`crates/combat/src/feel.rs:425`) and `JuicingParams` (`:167`): `SHAKE_ROTATION_CAP_DEG`
  (`crates/game/src/camera_feed.rs:580`), `FLASH_TICKS`/`FLASH_LIFT`
  (`crates/game/src/fighter_look.rs:685`, `fighter_look_flash.rs:24`), the knockdown slow-mo factor
  (`crates/combat/src/feel_flow.rs:22` id 1) and a play-time pitch variation (today detune is
  bake-time only, `crates/audio/src/bank.rs:236`).
  Donor: Miisan-png/godot-liquid-ui `scripts/core/juice_config.gd` (whole file). Tier B. Size S.
  Owns `crates/combat/src/feel.rs`, one read each in camera_feed, fighter_look_flash, feel_flow.
  Accept: `cargo test -p engine-combat --lib feel` >0 passed plus a fence test: every field inside
  its range and `latency_ms(SHIPPED.buffer_ticks) == 33` unchanged.
  F2P: no single table holds these four; `pitch_variation` has 0 hits in `crates/audio`.
  P2P: `--lib feel`; the golden is unchanged (const values mirror today's literals).
  needs: SW21, SW23 (both read the table). Landed af06ff26b: one validated presentation feel row (PresentationFeel/PRESENTATION_V1 in combat), master compile fix for fighter_look_flash; run 40127078684 green.

- [x] SW25 anim: planted feet without a live contact solve every tick - bake a per-clip
  foot-grounded weight track (30 Hz sample, permille) into `crates/anim/src/motion_bake.rs` beside
  `bake_len`, so `fbik`'s `FootLocker` starts from a per-clip planted weight instead of only the
  live test `planted_from_contacts` (`crates/anim/src/feet.rs:137`, `contact_feet.rs`).
  Donor: nicholas-maltbie/OpenKCC `Packages/com.nickmaltbie.openkcc/Editor/HumanoidFootIKEditor.cs`
  `BakeAnimations` (samplingRate 30). Tier B. Size M. Owns `crates/anim/src/motion_bake.rs`,
  `crates/anim/src/loco_bake.rs` caller.
  Accept: `cargo test -p engine-anim --lib foot` >0 passed (`planted_foot_slides_under_the_straight_budget`
  and friends today), new: a clip with the left foot down ticks 0..10 bakes weight >= 900 and
  <= 100 at tick 20.
  F2P: `ik_weight` has 0 hits in `crates/anim` (no baked track).
  P2P: `--lib motion_bake`; the foot-slide budget is not worse.
  needs: none. Landed 6a7fef7fc: per-clip foot-plant weight lane at 30 Hz from the loco contact masks (runs 40119750576 and 40120015231, 14 and 7 tests).

#### Tools and build (build hunt)

- [x] SW10 tools: a doc that copies a source file's contract carries that file's hash, and a code
  change without the doc change fails - new `tools/doc-pin.mjs` (first 8 bytes of sha256 over
  CRLF-normalised text) with a small `{docPath, sourcePath}` registry, called from
  `tools/workspace-check.mjs`. First pin: the doc that lists the door's verbs by hand (find it with
  `git grep -l "scene.read" -- docs`) against `crates/endpoint/src/resources.rs`. Today
  `tools/docs-links.mjs` checks that links resolve, with no content hash.
  Donor: rust-analyzer `xtask/src/tidy.rs` (`check_lsp_extensions_docs`, `stable_hash`).
  Tier C. Size S. Owns new `tools/doc-pin.mjs` + `tools/doc-pin.test.mjs`, one call in
  `tools/workspace-check.mjs`.
  Accept: `node --test tools/doc-pin.test.mjs` >0 passed: a fixture source edit without its doc edit
  exits 1 naming both files; a CRLF and an LF checkout hash equal.
  F2P: no doc pin exists. P2P: `node tools/workspace-check.mjs` PASS. needs: none. Landed 59b1394: tools/doc-pin.mjs (+7 tests), called from workspace-check.

- [x] SW11 tools: `#[should_panic]` becomes a ratchet family - a new one fails unless its file is on
  an allowlist with a reason. Today there are 2 hits, both legitimate receipt-failure asserts
  (`crates/game/src/bin/staged_fight.rs:901`, `crates/render/src/test_gpu.rs:210`); they go on the list.
  Donor: rust-analyzer `xtask/src/tidy.rs` `check_test_attrs` (the `need_panic` allowlist).
  Tier C. Size XS. Owns `tools/rust-quality.mjs` (one family) and its test.
  Accept: the rust-quality test suite passes with a fixture that adds a third `should_panic` and
  sees FAIL; `node tools/rust-quality.mjs` on HEAD stays PASS. needs: none. Landed 87490a8: should_panic is a rust-quality family with a 2-row allow list (27/27 tests).

- [x] SW12 tools: `crates/workspace-hack` stays honest - a verify step (cargo-hakari's `verify`, about
  ten lines over `cargo metadata`) fails when a workspace crate resolves a dependency feature set the
  hack crate does not unify. Today the hack crate exists and nothing checks it.
  Tier C. Size XS. Owns one check in `tools/workspace-check.mjs` and a test.
  Accept: `node tools/workspace-check.mjs` prints a `workspace-hack verify` PASS line on HEAD and
  FAIL on a fixture with an un-unified feature. needs: none. Landed 8c7a6b8: feature-unify resolveViolations over cargo metadata, wired as workspace-hack verify (7 tests, live resolve 0 violations).

- [ ] SW13 build: Windows builds take `--cfg windows_slim_errors` (the `windows` crate's own switch,
  `windows::core::Error` 16 -> 4 bytes; render uses `windows` 0.62 at `crates/render/Cargo.toml:91`)
  and `-C symbol-mangling-version=v0` (shorter symbols, clearer closure backtraces) in
  `.cargo/config.toml` `[target.x86_64-pc-windows-msvc] rustflags`.
  Donor: zed-industries/zed `.cargo/config.toml`. Tier C. Size XS. Owns `.cargo/config.toml`.
  Accept: a render unit test asserts `size_of::<windows::core::Error>() == 4`; link time and target
  size before/after on a fresh pool are written in the commit.
  Trap: any rustflags change rebuilds the tree for every agent and the mirror cache, so it lands at
  the end of a day. needs: lead: end-of-day cache window.

- [ ] SW14 build: the ship exe gets a 10 MB main-thread stack and a static CRT (`/STACK:10000000`,
  `+crt-static`) on the ship build only, so a deep destruction or nav recursion cannot overflow the
  1 MB default and the exe needs no redistributable. Dev builds unchanged.
  Donor: nushell `.cargo/config.toml`; astral-sh/ruff and uv set the same `+crt-static` (ruff#11503).
  Tier C. Size S. Owns the ship build's link flags (wherever the ship build sets them today).
  Accept: the ship arena build with the flags, `dumpbin /headers` shows the 10 MB stack reserve, a
  window run prints `view=ok`. Check every DLL-only dependency first and stop if `+crt-static`
  breaks one. needs: none.

#### Unity store ideas

- [ ] SW30 ai: reactive sequence and reactive fallback for the behaviour tree - a guard that flips
  halts the running child (Behavior Designer's "conditional aborts"). `crates/ai/src/bt.rs:234
  Sequence` and `:236 Selector` remember their place and cannot abort
  (`git grep -in "reactive\|halt_child" -- crates/ai` = 0 hits). Add `BtNode::ReactiveSequence` and
  `ReactiveFallback`, a running child per node and `halt_child` that resets a subtree.
  Donor: BehaviorTree.CPP `src/controls/reactive_sequence.cpp` (`ReactiveSequence::tick`) and
  `include/behaviortree_cpp/controls/reactive_fallback.h`. Tier B. Size S. Owns `crates/ai/src/bt.rs`.
  Accept: `cargo test -p engine-ai --lib bt` >0 passed with new
  `reactive_sequence_halts_the_running_child_when_its_guard_fails` (3 children, child 2 Running,
  the guard flips: child 2 halted, restart at 0) and `reactive_fallback_success_halts_the_rest`.
  F2P: no node can abort a running branch. P2P: `--lib bt` and `--lib gladiator` green; the golden
  holds because no shipped tree uses the new nodes. Switching the gladiator's tree to them moves the
  fight and is an owner call. needs: none. Progress 2026-10-24: Landed 9631dfe44: reactive sequence/fallback halt the running child; run 40120357384 green, 24 tests. Left: P2P --lib gladiator and the golden.

- [x] SW31 editor: a tuning struct draws its own editor panel - `#[derive(Inspector)]` with
  `#[inspector(min, max, step, group, readonly)]` field attributes emits
  `fn ui(&mut self, ui: &mut egui::Ui) -> bool`, so a new tuning field needs no editor code (Odin
  Inspector's pitch). Today `git grep -n "derive(.*Inspect" -- crates` = 0 hits and `crates/editor`
  has no proc-macro.
  Donor: jakobhellermann/bevy-inspector-egui `crates/bevy-inspector-egui-derive/src/lib.rs`
  (`expand_struct`, `extract_inspector_attributes`); the macro shape only, no bevy_reflect.
  Tier C. Size M. Owns new `crates/editor-derive` (proc-macro), `crates/editor/src/inspector.rs`, the
  manifest/ARCHITECTURE/workspace-check edge lines.
  Accept: the editor crate's `--lib inspector` tests >0 passed: a 3-field struct with one attribute
  and one skipped field emits 2 widgets with the attribute's range; first user: one feel table.
  needs: none. Landed 4abf7734b: derive(Inspector) draws a tuning struct's egui panel (run 40120427544, 33 tests).

- [ ] SW32 game: look knobs reload while the arena runs - a `live` feature loads a small dylib of
  non-generic tuning fns (`LiveTune { cam_boom, shake_scale, flash_lift }` passed by `&mut`), watches
  it with a 300 ms debounce and keeps the last good lib when a load fails; the hook sits beside the
  30-frame content poll in `crates/game/src/bin/arena.rs` (`HOT_RELOAD_POLL_EVERY_FRAMES`).
  Donor: rksm/hot-lib-reloader-rs `src/lib.rs` (its limits: no signature change, no generics, no
  globals in the reloaded code). Tier C. Size M. Owns new `crates/live`, one hook in `bin/arena.rs`.
  Accept: `cargo test -p engine-game --features live --lib live_reload` >0: a fixture lib returning 1
  then 2 is observed as 2 without a restart. Presentation knobs only, never a sim value (golden).
  needs: none.

#### Fab and Rust engines

- [ ] SW40 game: the fighter turns in place - the presentation root turns to catch up with the view.
  `TurnGate::step` in `crates/anim/src/turn_in_place.rs` (landed 301842cc6) fires
  `TurnClip::{Left90,Right90,Left180,Right180}` after 15 ticks at 90 deg and 45 at 180 (130 deg
  switches clip); the root yaw offset eases in over `TURN_BLEND_TICKS` 12 and returns by clip end.
  The look's root placement at `crates/game/src/fighter_look.rs:1958` feeds it the aim yaw the head
  already tracks (`crates/anim/src/ik.rs:253` `look_at`, 70 deg rival limit); `slot.yaw` untouched.
  Donor: ALS-Refactored `AlsAnimationInstance.cpp` `RefreshTurnInPlace`, fab.com/s/0eb8afffb774.
  Tier B. Size S. Owns `crates/game/src/fighter_look.rs` (the root-placement hunk only).
  Accept: `cargo test -p engine-game --features app --lib turn_in_place` prints 3 passed (0 today):
  a slow 90 deg view plays Right90 with `slot.yaw` byte-equal, a moving body never turns, x2 equal.
  F2P: the root snaps to `slot.yaw` every tick; no turn clip ever plays.
  P2P: `cargo test -p engine-game --features app --lib view_truth` green; golden unchanged.
  needs: group C camera framing landed (it owns `crates/game/src/fighter_look.rs` today).

- [ ] SW41 render: dust, smoke, blood mist and heat shimmer that drift - one quarter-res
  screen-space flow field instead of particles for the soft layers. Passes: curl, vorticity,
  divergence, 20 Jacobi pressure iterations, gradient subtract, advect velocity then dye
  (donor config: SIM_RESOLUTION 128, DENSITY_DISSIPATION 1, VELOCITY_DISSIPATION 0.2, CURL 30).
  Splats come from `crates/render/src/particle_events.rs` and the brazier; `fluids_gpu.rs` keeps
  close blood droplets. Unblocks QA9 (open today: `node tools/board-slice.mjs --find
  "heat shimmer"` returns QA9), which is waiting on a cheap carrier for the mask.
  Donor: PavelDoGreat/WebGL-Fluid-Simulation `script.js` `step(dt)` + `config`; Fab Fluid Ninja
  (listing URL not captured, fab.com/search?q=fluid+ninja). Tier B. Size M.
  Owns new `crates/render/src/flow_field.rs` + one `pub mod` line in `crates/render/src/lib.rs`.
  Accept: `cargo test -p engine-render --lib flow_field` prints 4 passed (0 today): a CPU oracle
  keeps a splat inside its mask, strength 0 is byte-equal, the same seed x2 matches, 1920x1080
  runs at quarter res 480x270. Then QA9 rides this carrier.
  F2P: no flow field; `git grep -in "vorticity\|curl" -- crates/render` is 0 hits.
  P2P: `cargo test -p engine-render --lib fluids` green; golden unchanged (view only).
  needs: none. Progress 2026-10-24: CPU oracle landed 46b3a5f5d (curl through advect dye, run 40118995253, 4 tests). Left: the quarter-res GPU pass and the draw.

- [ ] SW42 game: the fight camera cuts and eases like a director - a stack of shots, highest
  priority wins, a tween between them and a dead zone that holds still while the fighters stay
  inside it. Sits over the new framing work in `crates/game/src/fighter_look_frame.rs` and the
  pure solve `crates/combat/src/camera_framing.rs:75 frame_melee`; it must not replace either.
  Shots today: 1 (`crates/game/src/bin/arena.rs:1113 framing: Framing`).
  Donor: ramokz/phantom-camera `phantom_camera_3d.gd` (`set_priority`, `FollowMode::FRAMED` dead
  zone, `follow_damping` 0.1 per axis, `tween_resource`, `inactive_update_mode::NEVER`).
  Tier B. Size M. Owns new `crates/game/src/camera_stack.rs`, `crates/game/src/bin/arena.rs`.
  Accept: `cargo test -p engine-game --features app --lib camera_stack` prints 3 passed (0 today):
  a higher-priority shot takes over after its blend ticks, the eye holds inside the dead zone,
  a shot gone stale drops back to the next priority, x2 equal.
  F2P: one framing result per tick; `git grep -in "camera_stack\|blend_curve" -- crates` 0 hits.
  P2P: `cargo test -p engine-game --features app --lib camera_frame` green; golden unchanged.
  needs: group C camera framing landed (it owns `crates/game/src/fighter_look.rs` today). Progress 2026-10-24: its needs is met: the group C framing landed da9e6d8a4 (fight camera frames hero + threat; shake after the refit; look-ahead applied after frame_hold; 316 passed in run 40119267777, 5 pre-existing reds).

- [ ] SW43 anim: a strafing body leans the way it moves - four damped axes over the loco blend.
  `VelocityBlend { forward, backward, left, right }` from the body-frame velocity, each eased with
  a half-life damper, snapping on spawn/teleport (`bInitializationRequired` in the donor), fed to
  `crates/anim/src/space.rs:244 weights_into`; `GaitPhase` stays the clock.
  Donor: ALS-Refactored `AlsAnimationInstance.cpp` `RefreshVelocityBlend`, fab.com/s/0eb8afffb774.
  Tier B. Size S. Owns new `crates/anim/src/velocity_blend.rs`, `crates/anim/src/space.rs`.
  Accept: `cargo test -p engine-anim --lib velocity_blend` prints 3 passed (0 today): a pure
  strafe sets only `right` and reaches 0.9 of it inside one half-life, a diagonal splits both,
  x2 equal. `git grep -in "velocity_blend" -- crates/anim` is 0 hits today.
  F2P: the blend weights come from speed and heading with no damped body-frame vector.
  P2P: `cargo test -p engine-anim --lib space` green; golden unchanged (view only).
  needs: none. Progress 2026-10-24: landed 3ae1d0dc6 (four damped body-frame velocity axes); no run id in the commit, the lead asks for it.

- [ ] SW44 game: a hard landing rolls instead of stacking - a fall above the speed threshold plays
  a landing roll, and a roll that leaves the ground hands the body to the ragdoll.
  Gate beside the ground get-up (`crates/game/src/getup_pick.rs:147 pick`, `crates/game/src/
  ragdoll_look.rs`), driven from the landing branch in `crates/game/src/fighter_look.rs`.
  Donor: ALS-Refactored `AlsCharacter.cpp` rolling gate (`bStartRollingOnLand`,
  `RollingOnLandSpeedThreshold` on downward velocity, `bInterruptRollingWhenInAir` -> ragdoll,
  `Rolling.bCrouchOnStart`). Tier B. Size S.
  Owns `crates/game/src/fighter_look.rs` (landing hunk), `crates/game/src/getup_pick.rs`.
  Accept: `cargo test -p engine-game --features app --lib land_roll` prints 3 passed (0 today): a
  fast fall rolls, a slow one absorbs, a roll that leaves the ground goes ragdoll; today's
  `landing` tests stay green.
  `git grep -in "start_roll\|landing_roll\|land_roll" -- crates` is 0 hits today.
  F2P: a knocked fighter lands flat and always stands up from the ground pose.
  P2P: `cargo test -p engine-game --features app --lib getup` green; golden unchanged.
  needs: group C camera framing landed (it owns `crates/game/src/fighter_look.rs` today).

- [x] SW45 anim: a hit react picks its variant by an index - one blend node with per-input weights
  instead of one graph branch per variant. `NodeKind` at `crates/anim/src/graph.rs:22` gains an
  index-selected blend: a `u8` parameter chooses the live input, each input carries its own
  weight, the switch eases over the node's blend ticks. Consumer: the shipped table
  `content/design/hit-reactions.rows` parsed into `crates/game/src/hit_reaction.rs:240
  ReactionTable` (sector x weight -> `Shape`), which today names a shape per cell with no
  index-selected node behind it.
  Donor: Fyrox `fyrox-animation/src/machine/mod.rs` (`PoseNode::BlendAnimationsByIndex`,
  `IndexedBlendInput`, `Parameter::{Rule,Weight,Index}`).
  Tier B. Size M. Owns `crates/anim/src/graph.rs`, `content/design/hit-reactions.rows`,
  `crates/game/src/hit_reaction.rs`.
  Accept: `cargo test -p engine-anim --lib graph` prints the 3 new `index_blend_*` tests (0
  today) and today's graph tests stay green: an index picks one variant and weights it, the
  others stay at zero, a switch eases over its ticks, x2 equal.
  `git grep -in "BlendByIndex\|by_index" -- crates/anim` is 0 hits today.
  F2P: a variant needs its own graph branch; no integer selector and no per-input weight.
  P2P: `cargo test -p engine-game --lib hit_reaction` green; golden unchanged.
  needs: none. Landed 183a8c22b: index-selected blend node; game hit_reaction 8 passed run 40126053110, anim lib 604 passed run 40126097197 (the 1 red is hit_track at HEAD, not this row).

- [ ] SW46 combat: a hit-confirm chains into a different follow-up - the combo step names a base
  index plus a conditional one gated on the last blow's outcome. `ComboTable` at
  `crates/combat/src/combo.rs:76` and `ComboCursor` at `:117` become `ComboStep { base,
  conditional: Option<(u8, ComboReq)> }`, and `crates/combat/src/combo.rs:141 attack` takes the
  outcome (`AnyDamage`, `Blocked`, `Whiffed`). Rows stay in `crates/combat/src/tables.rs:357
  tables_for_version`.
  Donor: veloren/veloren `common/src/comp/melee.rs` `CustomCombo { base, conditional }`.
  Tier B. Size S. SIM. Owns `crates/combat/src/combo.rs`, `crates/combat/src/fight.rs`.
  Accept: `cargo test -p engine-combat --lib combo` prints the 3 new `combo_conditional_*` tests
  (0 today) and today's combo tests stay green: a blocked swing chains to the bash slot, a clean
  hit to the base slot, a whiff to the reset, x2 equal.
  F2P: `ComboCursor::next(stance)` walks one fixed sequence per stance; no outcome is read.
  P2P: `cargo test -p engine-combat --lib tables_v2` green; golden re-pinned with a receipt.
  needs: SW27 (lead 2026-10-24: lands on rules v3 only; v1 and v2 tapes replay byte-equal, the golden d57b5c8ae3096614 never moves; the v3 tape hash is pinned in this row's test).
- [ ] SW47 combat: a wide swing through two bodies hits the second weaker - power scales with the
  order and a per-swing cap stops the chain. `resolve_tick` at `crates/combat/src/sweep.rs:463`
  already loops every victim in ascending id; carry `n` per sweep and apply
  `power = base * (1 + (n-1)*scaling)` in integer permille, stop at `max_targets`; `SwingGate`
  at `:547` keeps one contact per victim per swing.
  Donor: veloren/veloren `common/src/comp/melee.rs` `MultiTarget::Scaling` +
  `simultaneous_hits` (default 1).
  Tier B. Size S. SIM. Owns `crates/combat/src/sweep.rs`, `crates/combat/src/tables.rs`.
  Accept: `cargo test -p engine-combat --lib sweep` prints the 3 new `sweep_multi_*` tests (0
  today) and today's sweep tests stay green: two victims take 1000 and 1250 permille, a third is
  refused past the cap, x2 equal.
  F2P: every victim of one sweep takes the same power; `git grep -in "multi_target\|scaling"
  -- crates/combat` is 0 hits.
  P2P: `cargo test -p engine-combat --lib sweep` green x2; golden re-pinned with a receipt.
  needs: SW27 (lead 2026-10-24: lands on rules v3 only; v1 and v2 tapes replay byte-equal, the golden d57b5c8ae3096614 never moves; the v3 tape hash is pinned in this row's test).
- [ ] SW48 combat: an unblockable blow lands - each move declares whether it can be blocked,
  parried or dodged, and the resolve reads the flag. `SweepTables` at `crates/combat/src/
  sweep.rs:58` gains `SweepAnswer { blockable, parryable, dodgeable }`; `sweep_one` at `:359`
  returns `Hit` instead of `Blocked` when `!blockable`; the AI reads the flag for its guard
  choice (`crates/ai/src/gladiator.rs`).
  Donor: veloren/veloren `common/src/comp/melee.rs` (`Melee::dodgeable`, `MeleeConstructor::
  blockable`).
  Tier B. Size S. SIM. Owns `crates/combat/src/sweep.rs`, `crates/combat/src/tables.rs`.
  Accept: `cargo test -p engine-combat --lib sweep` prints the 2 new `sweep_answer_*` tests (0
  today) and today's sweep tests stay green: an unblockable move reports Hit, a blockable one
  reports Blocked, a dodgeable one reports Dodged.
  `git grep -in "dodgeable\|blockable\|parryable" -- crates/combat` is 0 hits today.
  F2P: every move can be blocked; the outcome is a geometry fact, not a move fact.
  P2P: `cargo test -p engine-combat --lib tables_v2` green; golden re-pinned with a receipt.
  needs: SW27 (lead 2026-10-24: lands on rules v3 only; v1 and v2 tapes replay byte-equal, the golden d57b5c8ae3096614 never moves; the v3 tape hash is pinned in this row's test).

#### Decided by the lead (2026-10-24): fight changes go on rules v3

- [ ] SW27 combat: rules v3, so fight fixes ship without moving any recorded tape - versions are
  append-only: v1 is the frozen hit rule (the golden scene), v2 is today's live rule with recorded tapes
  (arena_matrix, heft, armour, bolt tests, replays), so a behaviour change goes into a new version. Add
  `FIGHT_RULES_V3 = 3` beside `FIGHT_RULES_V2` (`crates/combat/src/fight.rs:530`), one helper for "v2
  or later" and use it at every site that today means "the v2 feature set" (`version == FIGHT_RULES_V2`
  in crates/arena: armour.rs:94, authority.rs:99, bolt.rs:323, heft.rs:80, lib.rs:1170/1238/1319/1347/1424/
  1543/1587, live.rs:344/542, spine_step.rs:166/613 and the rest the grep finds), `ArenaRules::is_known`
  (`crates/arena/src/lib.rs:298`) and the unknown-version refusal (`collide.rs:338`) accept 3, and
  `tables_for_version` (`crates/combat/src/tables.rs:357`) maps 3 to v2's tables. At this step v3 plays
  exactly like v2; the live default stays v2.
  Tier A. Size M. SIM (behaviour-neutral). Owns the version sites above; no rule change.
  Accept: `cargo test -p engine-arena --lib rules_v3` >0 passed: the same tape under v2 and v3 gives the
  same hash on every tick (600 ticks, 2 and 8 fighters); `cargo test -p engine-arena --lib` and
  `cargo test -p engine-combat --lib` green with every pinned v2 hash unchanged; the golden x2 at
  `d57b5c8ae3096614`. Then the v3-only rows (SW20, SW26, SW46, SW47, SW48, SW82) land behind
  `version >= FIGHT_RULES_V3`, and one later commit moves the live default to v3 with the new v3 tape hash
  pinned. F2P: version 3 is refused as unknown. P2P: all of the above. needs: none.

- [ ] SW26 combat: a held ATTACK is one press inside the feint window - FEEL1b's armed duel (55a090dda,
  run 40113669615) found that holding ATTACK restarts the swing inside the v2 feint window, so a held
  press landed the pugio at press+27 (3 ticks late) while one tap lands exactly on the row's startup.
  Decide the rule (a press is edge-triggered and a hold is the same press, or a hold inside the window
  is a deliberate feint cancel), then pin it.
  Tier B. Size S. SIM. Owns the press-edge and feint arm in `crates/combat/src/fight.rs`.
  Accept: `cargo test -p engine-combat --lib feint` >0 passed with a new
  `a_held_attack_is_one_press_inside_the_feint_window` (or the rule chosen): held and tapped presses land
  on the same tick; `feel1b_duel_lands_the_clava_after_the_pugio_by_the_rows` stays green.
  F2P: a held press lands 3 ticks late. P2P: `--lib feel`; the golden moves only if a recorded tape holds
  ATTACK across the window. needs: SW27 (lead 2026-10-24: lands on rules v3 only; v1 and v2 tapes replay byte-equal, the golden d57b5c8ae3096614 never moves; the v3 tape hash is pinned in this row's test). Rule chosen: a press is edge-triggered, so a held ATTACK is the same press; a feint is its own input, never a hold.

#### Godot engine and addons (wave 2)

- [ ] SW50 render: the sunlit sand stops clipping to a flat white - the arena gets Godot 4.4's AgX
  curve beside the ACES one. `crates/render/src/shaders/tonemap.wgsl:101 aces_tonemap` and `:130
  pbr_neutral` are the only two curves; the pipeline flag at `tonemap.wgsl:88` already selects
  between them, so AgX is a third arm, not a new pass.
  Do: `agx_tonemap(color)` = rec709->rec2020+inset matrix, `allenwp_curve` (crossover 0.18 on the
  display white, Reinhard-like shoulder `s = slope*s*(1+s/w)/(1+slope*s/shoulder_max)`, power toe
  `t/(t+toe_a)` at contrast, mixed by `x < 0.18`), clamp to display white, outset+rec2020->709
  matrix; add `TonemapCurve::Agx` to the post preset and keep ACES as default.
  Donor: https://github.com/godotengine/godot/blob/master/servers/rendering/renderer_rd/shaders/
  effects/tonemap.glsl (`tonemap_agx`, `allenwp_curve`, `agx_outset_rec2020_to_rec709_matrix`,
  `TONEMAPPER_AGX 4`). Tier B. Size S. Owns `crates/render/src/shaders/tonemap.wgsl`,
  `crates/render/src/post/chain.rs`, `crates/render/src/post/preset.rs`.
  Accept: `cargo test -p engine-render --lib tonemap` >0 passed (today ACES + PBR Neutral only); new:
  a saturated 4.0 sand value keeps more chroma than ACES under AgX (hue angle within 5 deg of input,
  ACES shifts it further), 0.18 grey maps within +-0.5% of 0.18 display, every channel monotone.
  F2P: `agx` has 0 hits in `crates/render/src/shaders/tonemap.wgsl`.
  P2P: `cargo test -p engine-render --lib tonemap` stays green; golden untouched (view only, ACES default).
  needs: none.

- [ ] SW51 render: a fighter reads against the bright sand at any sun angle - a rim term and a
  backlight (light through the body) in the forward shade. Today the skin block is
  `crates/render/src/shaders/forward.wgsl:751-840` (LF8 `SKIN_SUBSURFACE_POWER 12.0`, one lobe) and
  `git grep -in "backlight\|\brim_light" -- crates/render` = 0 hits.
  Do: `rim_light = pow(max(1e-4, 1 - n_dot_v), max(0, (1 - roughness) * 16))` times a per-material
  `rim` (0..1, default 0.35 on skin/cloth, 0 on sand) times `mix(1, albedo, rim_tint)`;
  `diffuse += light_color * (1/PI - diffuse_brdf_nl) * backlight` so the sunset behind a fighter
  moves light through the silhouette. Both ride the existing material words.
  Donor: https://github.com/godotengine/godot/blob/master/servers/rendering/renderer_rd/shaders/
  scene_forward_lights_inc.glsl (`light_compute` LIGHT_RIM_USED and LIGHT_BACKLIGHT_USED arms).
  Tier B. Size S. Owns `crates/render/src/shaders/forward.wgsl`, `crates/render/src/material_kind.rs`.
  Accept: `cargo test -p engine-render --lib rim` >0 passed (0 today); numbers: rim on a grazing
  normal (n_dot_v 0.1, roughness 0.4) adds 0.27 of the light, front-on (0.9) adds under 1e-4,
  backlight adds only where `n_dot_l < 0`, rim 0 is byte-equal to today.
  F2P: no rim or backlight term exists in the shade.
  P2P: `cargo test -p engine-render --lib material_kind` green; golden untouched.
  needs: none.

- [ ] SW52 render: embers, dust and blood mist get pushed around by the arena instead of falling in
  a straight line - a particle force list (sphere, box, vector field) with attenuation and
  directionality. Today `crates/render/src/particles_gpu.rs:643 step` integrates gravity (`:671`) and
  drag only; `git grep -in "attractor\|vector_field" -- crates/render` = 0 hits.
  Do: `ParticleForce { kind, centre, extents, strength, attenuation, directionality }`, max 32;
  sphere/box `amount = (1 - d)` (0 outside), vector field samples a 3D noise lattice baked once;
  `amount = amount.powf(attenuation)`, `dir = mix(dir, forward, directionality)`,
  `vel -= mass * amount * dir * strength` with `mass` from lifetime; use it for brazier updraft and
  a slow vortex over the blood pool.
  Donor: https://github.com/godotengine/godot/blob/master/servers/rendering/renderer_rd/shaders/
  particles.glsl (`main` attractor loop, `ATTRACTOR_TYPE_SPHERE/BOX/VECTOR_FIELD`, `MAX_ATTRACTORS 32`).
  Tier B. Size S. Owns `crates/render/src/particles_gpu.rs`, `crates/render/src/shaders/particle.wgsl`.
  Accept: `cargo test -p engine-render --lib particles_gpu` (35 tests today, >0) plus a new
  `forced_particle_bends_toward_the_attractor`: a particle 1.0 m outside a sphere force is untouched,
  at 0.5 m of a radius-1 sphere it moves 0.5*strength toward the centre on one 60 Hz tick, x2 equal.
  F2P: `step` has no force stage.
  P2P: `cargo test -p engine-render --lib soft_fade` green; golden untouched (presentation only).
  needs: none.

- [ ] SW53 render: the sky and the sand gradients stop banding on an 8-bit potato display - one
  ordered dither before quantization. Today the present converts to an 8-bit target with no dither:
  `crates/render/src/present/gpu_present.rs:14 HDR_FORMAT` feeds the surface, and `git grep -in
  "dither\|deband" -- crates/render` hits only the LOD cross-fade (`crates/render/src/lod_fade.rs`),
  never the output.
  Do: `screen_space_dither(frag_coord)` = `fract(dot(vec2(171,231), frag_coord) / vec3(103,71,97))`
  minus 0.5, divided by `2^bits - 1`, added after the linear->sRGB encode and before the store; put
  it in the tonemap/present shader behind one `quality.rs` knob (`output_dither_px_bits`, 8 on every
  tier, 10 on the HDR path).
  Donor: https://github.com/godotengine/godot/blob/master/servers/rendering/renderer_rd/shaders/
  effects/tonemap.glsl (`screen_space_dither`, `FLAG_USE_8_BIT_DEBANDING`; Iestyn's Portal 2 RGB
  dither, 7 instructions).
  Tier B. Size XS. Owns `crates/render/src/shaders/tonemap.wgsl`, `crates/render/src/quality.rs`.
  Accept: `cargo test -p engine-render --lib dither` >0 passed (0 today); numbers: a flat 0.5 sand
  ramp that quantizes to 3 identical 8-bit steps without dither shows >= 8 distinct levels with it,
  and the mean of 16x16 px moves under 0.2% (no overall brightening).
  F2P: no output dither exists.
  P2P: `cargo test -p engine-render --lib lod_fade` green; golden untouched.
  needs: none.

- [ ] SW54 anim: hair, straps and the loincloth chain bend over the shoulder instead of through it -
  per-joint damping curves plus collision shapes on the spring chain. Today
  `crates/anim/src/springs.rs` has one global `SpringParams { stiff_q, damp_q, max_offset_q }`
  (`:78 new`, `:246 step`) and `git grep -in "collision\|radius" -- crates/anim/src/springs.rs` = 0
  hits: no shape pushes a tip out.
  Do: per-joint `radius_mm`, `stiff_milli`, `drag_milli`, `gravity_milli` (VRM spring bone) sampled
  through one curve along the chain, plus `Vec<SpringCollider>` (sphere and capsule with `offset`
  and `inside` flag) resolved after `step`, before `apply_to_pose` (`:309`); an excluded collider
  pair list per chain so a braid does not fight its own root.
  Donor: https://github.com/godotengine/godot/blob/master/scene/3d/spring_bone_simulator_3d.cpp
  (`set_radius_damping_curve`, `set_stiffness_damping_curve`, `set_drag_damping_curve`,
  `set_gravity_damping_curve`, `set_enable_all_child_collisions`, `set_exclude_collision_path`,
  `SpringBoneCollision3D` spheres/capsules; MIT from VRM Consortium).
  Tier B. Size M. Owns `crates/anim/src/springs.rs`, one read in `crates/game/src/fighter_look.rs`.
  Accept: `cargo test -p engine-anim --lib springs` >0 passed (today: spring bound + carried-velocity
  tests) plus `a_capsule_pushes_the_tip_out`: a chain whose tip rest sits 20 mm inside a 40 mm
  shoulder capsule ends outside it within 2 ticks and settles without oscillation; per-joint drag
  gives a tail that lags its root by >= 3 ticks; x2 equal.
  F2P: one stiffness/damping pair per chain, no collider.
  P2P: `cargo test -p engine-anim --lib springs` and `--lib crowd_scale` stay green; golden untouched.
  needs: SW102 (the spring collision half lands there; this row keeps the per-joint curves).

#### Bevy plugins (wave 2)

- [ ] SW60 render: a fighter reads as a silhouette - a 2 px black rim around the body that does not
  break on flat-shaded armour. Today the outline shader is an orphan: `crates/render/src/shaders/
  outline.wgsl:13 fn outline_fragment` exists and **nothing draws it** (`git grep -n "outline.wgsl|
  outline_fragment" -- crates` = the file itself + one comment at `bloom_dual.rs:594`), and nothing
  averages normals per position, so an extruded hull would tear at every hard edge
  (`git grep -n "outline_normal" -- crates` = 0 hits).
  Do: new `crates/render/src/outline.rs` with
  `fn outline_normal_key(p: [f32;3]) -> [u32;3]` (quantize to 1e-5 m, so a shared corner of two
  faces is one key) and `pub fn generate_outline_normals(pos: &[[f32;3]], idx: &[u32], normals:
  Option<&[[f32;3]]>) -> Result<Vec<[f32;3]>, OutlineError>`: per triangle, for each of
  `(i0,i1,i2),(i1,i2,i0),(i2,i0,i1)` accumulate `angle * n` into the corner's key, then normalize
  (0 on an all-zero key). Clamp `angle` at pi so a degenerate sliver adds nothing. Then a second
  pass in `present/outline_draw_gpu.rs`: draw the hull with the outline normals, stencil-write the
  body in the same pass, and draw the lifted hull only where the stencil is 0, so the body is not
  filled and another fighter in front occludes the rim; `crates/render/src/quality.rs` gains
  `outline_px: u8` (potato 2, mid 2, high 3, 0 = off). Distance-constant width comes from the
  existing `outline.wgsl:23` screen-space normal test, kept as the fragment gate.
  Donor: https://github.com/komadori/bevy_mod_outline/blob/master/src/generate.rs
  (`OutlineMeshExt::generate_outline_normals`, `GenerateOutlineNormalsFrom::{VertexNormal,
  FaceNormal, ExternalBisector}`) and https://github.com/komadori/bevy_mod_outline/blob/master/
  src/lib.rs (`ATTRIBUTE_OUTLINE_NORMAL`, `OutlineVolume { visible, width, colour }`,
  `OutlineStencil { enabled, offset }`, `OutlineStencilEnabled::{Always, IfVolume, Never}`).
  Tier B. Size M. Owns new crates/render/src/outline.rs, new
  crates/render/src/present/outline_draw_gpu.rs, crates/render/src/shaders/outline.wgsl,
  crates/render/src/quality.rs.
  Accept: `cargo test -p engine-render --features window --lib outline` >0 tests; numbers: a
  24-vertex flat cube collapses to 8 position keys and its corner outline normal sits at 45 deg
  between the two face normals (`dot > 0.7`); the same cube under `FaceNormal` keeps 24 keys; a
  1.6 m fighter at 6 m with `outline_px = 2` costs one extra draw of its own index buffer and
  0 extra textures; `outline_px = 0` draws nothing (pass list length unchanged).
  F2P: `outline_normals_average_across_a_hard_edge` and `outline_pass_is_skipped_at_zero_px`.
  P2P: `cargo test -p engine-render --lib bloom_dual` stays green; golden untouched.
  needs: none (a capture with `view=ok` for the rim; does not touch `fighter_look.rs`).

- [x] SW61 ui: the pause menu is playable on a pad - one stick/D-pad press moves the highlight to
  the button that is actually in that direction, wrapping at the edge only where the menu says so.
  Today there is no focus or directional navigation at all: `git grep -rn "Focusable\|NavRequest\|
  nav_focus" -- crates/ui crates/game` = 0 hits; the only "focus" in the UI crate is the IME text
  gate (`crates/ui/src/text.rs:118`), and `crates/ui/src/menus.rs:558 Menu::step` moves the
  highlight by list index only.
  Do: new `crates/ui/src/nav_focus.rs`, pure and std-only: `pub enum NavRequest { Move(Dir), Action,
  Cancel, FocusOn(u16), Unlock }`, `pub struct FocusNode { pub id: u16, pub pos: [f32; 2], pub
  state: FocusState }` with `FocusState::{Inert, Prioritized, Active, Focused, Blocked}`,
  `pub fn resolve_2d(from: [f32;2], dir: Dir, nodes: &[FocusNode], wrap: bool, edge: [f32;2]) ->
  Option<u16>` = nearest unblocked node strictly inside the direction's half-plane, ties to the
  smaller squared distance; `wrap` projects the cursor one menu width past the edge and re-runs
  the same test. `pub fn pick_first(nodes: &[FocusNode]) -> Option<u16>` in the donor's order:
  Focused, then the active sub-menu's active child, then Prioritized, then root, then any. `Menu`
  (`menus.rs`) keeps `menu_id + focus: u16` and calls `resolve_2d` from `step`, so a screen
  transition still stays a pure function of (screen, selection, one request) - no new state in the
  save. Sub-menu entry: `Action` on a node with a child menu id focuses that menu's prioritized
  child, `Cancel` focuses the parent node.
  Donor: https://github.com/nicopap/ui-navigation/blob/master/src/resolve.rs (`resolve_2d`,
  `resolve_index`, `pick_first_focused`, `resolve_scope`, `active_menu`; the cycle assert in
  `root_path`), https://github.com/nicopap/ui-navigation/blob/master/src/menu.rs (`MenuSetting
  { wrapping, scope }`, `MenuBuilder::{NamedParent, EntityParent, Root}`).
  Tier B. Size M. Owns new crates/ui/src/nav_focus.rs, crates/ui/src/menus.rs.
  Accept: `cargo test -p engine-ui --lib nav_focus` >0 tests; numbers: on a 3x3 grid from the
  centre each of the 4 directions lands on its orthogonal neighbour in 1 request and 2 opposite
  requests return to the centre; a non-wrapping row at the right edge returns `None` (highlight
  unchanged, 0 events); the same row with `wrap` lands on the leftmost node; a `Blocked` node is
  never chosen.
  F2P: `nav_moves_to_the_neighbour_in_the_pressed_direction` and `wrap_only_on_a_wrapping_menu`.
  P2P: `cargo test -p engine-ui --lib menus` (step transitions) stays green; golden untouched.
  needs: none (`crates/ui/src/menus.rs` is not owned by group C). Landed c6e821a3c: directional focus via resolve_2d, rows and menu_id scope; run 40125004609 green, 49 tests.

- [ ] SW62 render: dust and blood stop popping in front of each other - translucent particle cards
  draw far to near instead of in spawn order. Today the draw list is pool order: `git grep -n
  "sort" -- crates/render/src/particles_gpu.rs` = 0 hits, while the pipeline blends premultiplied
  alpha (`crates/render/src/particle_draw_gpu.rs:201 PREMULTIPLIED_ALPHA_BLENDING`), which is
  order-sensitive for the dust/blood cards (`shaders/particle.wgsl:6`).
  Do: one sort key per live instance, `key = (depth_bits << 16) | (spawn_age_bits & 0xffff)`, and a
  CPU sort of the instance index list before upload in `particles_gpu.rs` (the pool already owns
  one staging vec - reuse it, no per-frame allocation). Quantize depth to 16 bits from the view
  camera so two cards in the same texel order stably by age; `quality.rs` gains
  `sort_particles: SortMode { Off, NearFirst }` (potato: `NearFirst` only above 64 live particles,
  `Off` below, so the small fights pay nothing). Skip the pass when every effect is additive
  (flame/spark), since order cannot change an additive sum. This is the cheap sibling of the
  donor's GPU mergesort: no extra buffer, no compute dispatch.
  Donor: https://github.com/djeedai/bevy_hanabi/blob/main/src/render/sort.rs
  (`GpuSortBufferSingleEntry { count, key, key2, value }`, `SortBindGroups::BLOCK_SIZE = 1024`,
  `SORT_BUFFER_BYTE_SIZE = 3 * 1024 * 1024`, `COPY_SOURCE_PARAM_COUNT`) plus
  https://github.com/djeedai/bevy_hanabi/blob/main/src/render/mod.rs (`GpuSimParams::
  camera_position`, the dual-key ordering idea).
  Tier B. Size S. Owns crates/render/src/particles_gpu.rs (draw list order only),
  crates/render/src/quality.rs.
  Accept: `cargo test -p engine-render --features window --lib particles_gpu` (35 tests today,
  stays >0) plus a new proof: 512 live particles at depths 5/2/9 m draw 9, 5, 2; the sorted list is
  a permutation of the input (same multiset of ids); the staging vec does not grow after the second
  frame; an additive-only frame skips the sort (0 comparisons).
  F2P: `particle_cards_draw_far_to_near` and `additive_only_frame_skips_the_sort`.
  P2P: `cargo test -p engine-render --lib soft_fade` (5 tests) stays green; golden untouched.
  needs: none (SW7's depth-faded cards land independently).

## not written as rows (read, real, but not worth a row this wave)

- bevy_hanabi fractional spawn accrual: already ours - `crates/render/src/particles_gpu.rs:512
  Accrue fractional spawns...` with `vfx_hanabi.rs:211 throttle_spawn`, so the "low rate stutters
  at 30 fps" donor idea is closed.
- bevy_hanabi effect graph / property buffer table (`src/render/buffer_table.rs`): we already
  batch 1..=8 effects in one dispatch (`vfx_hanabi.rs:153`), so only the buffer table's free-list
  would be new, and nothing in a 2-4 GB budget grows a pool per effect today.
- bevy_history / bevy_whoosh / bevy_kira crossfade: `crates/audio/src/declick.rs:119`
  two-gain crossfade law and `dsp.rs:487 track_tween_bar_crossfade` already cover the donor's
  channel tween.

#### Rust gamedev on GitHub (wave 2)

- [ ] SW70 assets+render: the far crowd and the debris field pick triangles from one cluster LOD
  hierarchy instead of a 4-mesh chain — 100+ seated bodies and broke chunks stop carrying LOD0 triangles
  at 40 m. Today the cook builds a per-mesh chain only: `crates/assets/src/mesh_cook.rs:45 LOD_RATIOS`
  (LOD0 + three simplifications), `crates/assets/src/lod_error.rs:131` measures that chain, and the live
  pick is per body: `crates/render/src/arena_lod.rs:185 pick` (`lod_by_pixel_error`). Meshlets are cooked
  (`crates/assets/src/meshlet.rs:510 build_meshlets`, 64 v / 124 t, `crates/assets/src/meshlet_cull.rs:144
  cone_culled`) but no draw path exists and clusters carry no LOD level: `crates/render/src/meshlet.rs:47
  cluster_cut` is a flat cut. Grep "cluster_lod|group_lod|clod|hierarch" over `crates/` = 0 hits.
  Do: a `ClodTree` in assets (level 0 = original clusters; each level groups adjacent clusters, simplifies
  the group to `simplify_ratio` 0.5 with `simplify_threshold` 0.85, keeps `error = max(prev*1.0, cur) +
  cur*0`) and a render select that walks the tree while the group's error over the screen threshold.
  Tier B. Size L. Owns new `crates/assets/src/clod.rs`, `crates/assets/src/mesh_cook.rs` (chain hunk),
  `crates/render/src/meshlet.rs`, `crates/render/src/arena_lod.rs`.
  Donor: https://github.com/zeux/meshoptimizer/blob/master/demo/clusterlod.h (`clodConfig`,
  `clodBounds` projected-error comment, `clodCluster`, `clodGroup`, `clodBuildHierarchy`,
  `clodLocalIndices`; defaults at :709-724).
  Accept: `cargo test -p engine-assets --lib clod` >0 tests; numbers: a 20k-tri fixture yields 5 levels,
  the select at 30 m draws <= 45% of the LOD0 triangle count with the group error under 1.5 px at 720p,
  and the level 0 index list is byte-equal to `build_meshlets` output.
  F2P: `clod_select_at_thirty_metres_draws_under_half_the_lod0_triangles`.
  P2P: `cargo test -p engine-assets --lib meshlet` stays green (143+ tests today); golden untouched.
  needs: none (presentation only; DX6 debris LOD may consume it later). Progress 2026-10-24: Step 1 landed 550fcf3cd: crates/assets/src/cluster_lod.rs (+_tests) builds the cluster-LOD tree over a cooked mesh (meshlet leaves, groups of 4 simplified to 0.5, monotone error, cut/cut_triangles/projected_error_px), 7 tests --features import, run 40126037723. The module is cluster_lod.rs, not clod.rs. Left: the render select in meshlet.rs and its 30 m receipt test, the arena_lod pick, the mesh_cook chain hunk, P2P --lib meshlet.

- [ ] SW71 assets: a simplified far body stops smearing the cloth and keeps the seam — attribute-aware
  simplification with locked seams replaces the collapse that averages UVs. Today `collapse.rs:254
  collapse_midpoint` averages normals and uv0 (`collapse.rs:285-288`) per collapsed vertex, so a seam
  halfway down a tunic smears both sides and no vertex is pinned: grep "seam|locks|attribute_weight|
  protect_mask" over `crates/assets` = 0 hits, and `crates/assets/src/cooked.rs:77 CookedMesh` carries
  positions/normals/uv0/tangents/indices only.
  Do: `simplify_with_attributes(mesh, attr_weights: [f32; 3], locks: &[u8], target_tris, max_error)` over
  uv0 + normal with weights (0.5 uv, 0.5 normal), a `seam_lock` pass that pins the first vertex of every
  uv discontinuity, and an error term that adds the attribute delta to the quadric so a locked seam cannot
  collapse. Tier B. Size M. Owns `crates/assets/src/collapse.rs`, `crates/assets/src/mesh_cook.rs`
  (chain hunk only), `crates/assets/src/lod_error.rs` (attribute measurement).
  Donor: https://github.com/zeux/meshoptimizer/blob/master/demo/clusterlod.h (`clodMesh.attribute_weights`,
  `.attribute_protect_mask`, `.vertex_lock`, `simplify_error_clamped`, `simplify_dilate_borders`) plus
  `https://github.com/zeux/meshoptimizer/releases/tag/v0.25` (`meshopt_simplifyWithAttributes`,
  `meshopt_simplifyWithUpdate`).
  Accept: `cargo test -p engine-assets --lib simplify` >0 (today `crate::collapse` tests); numbers: on a
  cylinder-with-seam fixture the seam angle stays within 2 deg across all 3 levels and the UV island count
  is unchanged, where today it drops by 1.
  F2P: `simplified_far_body_keeps_its_uv_seam_and_normal_cone`.
  P2P: `cargo test -p engine-assets --lib lod_error` stays green; the cook's tri budgets unchanged.
  needs: none.

- [ ] SW72 render+assets: a vertex costs 20 bytes on the potato tier instead of 48 — quantized position,
  octahedral normal and 16-bit UV in the cooked vertex buffer. Today `crates/render/src/present/
  gpu_present/submit.rs:2354 FORWARD_VERTEX_STRIDE: u64 = 48` (3+3+2+4 floats) and the cook writes raw
  f32 (`cooked.rs:77` positions/normals/uv0/tangents); grep "octahedral|quantize.*vertex|Snorm16|
  unpack_snorm" over `crates/assets crates/render` = 0 hits (quantization exists only in `anim_compress.rs`
  and `bc.rs`). At 720p with ~1.2 M vertices a frame fetches 57 MB instead of 24 MB.
  Do: cook emits `VertexPacked { pos: [i16;4] (mesh-local snorm16, w = 1), n: u32 (oct 2x16), uv: [u16;2] }`
  plus one `mesh_scale: [f32;4]` per draw; `forward.wgsl` / `sun_shadow.wgsl` unpack in the vertex stage
  (`oct_to_normal` with the `z = s - |x| - |y|` reconstruct). Tier B. Size M. Owns
  `crates/assets/src/mesh_cook.rs` (pack step), `crates/assets/src/cooked.rs`, new
  `crates/render/src/vertex_pack.rs`, `crates/render/src/shaders/forward.wgsl`,
  `crates/render/src/present/gpu_present/submit.rs` (stride + layout).
  Donor: https://github.com/zeux/meshoptimizer/blob/master/src/vertexfilter.cpp (`decodeFilterOct`: z from
  s - |x| - |y|, the z<0 fold, `decodeFilterQuat`, `decodeFilterExp`).
  Accept: `cargo test -p engine-render --features window --lib vertex_pack` >0 tests; numbers: stride 48 ->
  20 bytes (-58%), decoded normal within 0.2 deg and position within 0.3 mm of the f32 source on the
  fighter fixture, and a captured frame is byte-different but SSIM >= 0.999 against the f32 frame.
  F2P: `packed_vertex_round_trips_within_a_third_of_a_millimetre`.
  P2P: `cargo test -p engine-render --lib draw` stays green; golden untouched (presentation).
  needs: none (SW2/SW3 rewrite the draw path, not the layout).

- [x] SW73 assets: triangles that are under a pixel at the arena's far seats never reach the vertex stage —
  the cook also caps LOD error by edge length, not only by triangle budget. Today the cook budget is a
  triangle count (`crates/assets/src/mesh_cook.rs:45 LOD_RATIOS`, `PROP_MAX_TRIS`,
  `gen_intake.rs:1229 current = simplify(&current, target)`); grep "edge_len|subpixel|sub_pixel|
  error_edge" over `crates/assets` = 0 hits, so a thin railing keeps 500 sub-pixel triangles at 60 m.
  Do: after each simplify, measure the longest edge of every kept triangle in mesh units; drop a triangle
  whose projected longest edge at the LOD's switch distance is under `MIN_EDGE_PX` (0.6 px) until the
  simplify result is stable; record `edge_limited: bool` on the level so `lod_error` proves the stray
  stays under the budget.
  Tier B. Size S. Owns `crates/assets/src/mesh_cook.rs`, `crates/assets/src/lod_error.rs` (measurement).
  Donor: https://github.com/zeux/meshoptimizer/blob/master/demo/clusterlod.h (`simplify_error_edge_limit`
  at :46, applied at :497-523 as `min(error, sqrt(max_edge_sq) * limit)`).
  Accept: `cargo test -p engine-assets --lib edge_limit` >0 tests; numbers: on a railing fixture the LOD2
  level loses >= 120 sub-pixel triangles (500 -> 380) while the measured surface stray stays <= 0.9 m,
  and a 5 m prop is untouched.
  F2P: `edge_limit_drops_sub_pixel_triangles_and_keeps_the_surface_error`.
  P2P: `cargo test -p engine-assets --lib mesh_cook` stays green; the arena's LOD0 cook output is
  byte-equal (level 0 is never edge-limited).
  needs: none. Landed 4ef769d65: edge-length LOD cap after the simplify; edge_limit 3 tests run 40119543439, mesh_cook 20 tests run 40120056437.

## Not written as rows (already ours or no code read)

- LOD picked by projected pixel error: already live (`crates/render/src/arena_lod.rs:185`, `LodTable`
  pixel thresholds). Distance-band switch kept as the fixture path. No row.
- Vertex-cache and overdraw index order: already ours (`mesh_cook.rs:698 optimize_vertex_cache`,
  `:721 optimize_overdraw`, `:796 optimize_index_order`). meshopt vfetchoptimizer would only add a
  different heuristic; drop.
- CPU occlusion (SW4) and indirect draws (SW2) already on tomorrow's board; wgpu's occlusion-query sample
  loses to our Hi-Z plan for a potato tier (a query must be read back a frame late anyway).
- Profiling: puffin's gate (`puffin/src/lib.rs` `set_scopes_on`, `are_scopes_on` AtomicBool, 54 ns on /
  <1 ns off, `ProfilerScope` begin/end on Drop, `FrameView`/`merge_scopes_for_thread`) is what
  `crates/perf/src/profiler.rs:57 is_enabled` + `:345 span` already do, and SP63b owns GPU pass numbers.
  The only gap (0 hits for "puffin" in `crates/render`) is span placement, which needs no donor. No row.
- Fyrox: `fyrox-core/src/profiler.rs` is 404 on master (moved/removed); its blend-by-index tree was already
  taken today. No new portable render file read. No row.
- wgpu bunnymark (`examples/features/src/bunnymark/mod.rs`) now draws per bunny with a dynamic uniform
  offset and no `RenderBundle`s, so it is not a bundle-reuse donor. Our arena draws ~200 meshes per pass:
  a bundle cache is worth ~0.2 ms CPU at best. Drop.

#### Open-source fighting games (wave 2)

- [ ] SW80 game: a practice dummy that fights back on your terms - guard mode, spacing and recovery as data.
  Today there is no practice scenario: `git grep -in "practice" -- crates/game` shows only the editor
  tuning child (`crates/editor/src/tuning.rs:530`), and a scripted bout is hand-written presses
  (`crates/game/src/bin/staged_fight.rs:96`). No mode table, no dummy guard.
  Do: `PracticePlan` rows (guard: none/auto/all/random, dummy: stand/crouch/jump, distance:
  close/medium/far, fall_recovery, button_jam) -> one `Intent` per tick, fed by the existing input
  seam so the tick sees intents, never a new rule. Donor numbers: random guard fires under
  `random < 500` (50%), auto-guard timer 15 ticks, distance bands 10/80/130/260 px at 240p,
  fall-recovery command buffered 2 frames.
  Donor: https://github.com/ikemen-engine/Ikemen-GO/blob/bd2ead96/data/training.zss
  (`_iksys_trainingGuardMode`, `_iksys_trainingDistance`, `_iksys_trainingDummyMode`,
  `_iksys_trainingFallRecovery`, `assertInput`, `assertCommand{buffer.time: 2}`).
  Tier B. Size M. Owns new `crates/game/src/practice_plan.rs` + one `pub mod` line.
  Accept: `cargo test -p engine-game --features app --lib practice_plan` prints 4 passed (0 today):
  seed 2040 random guard blocks 18-22 of 40 attacks, close mode holds body distance at 10 px after
  20 ticks, a jump-mode dummy leaves the ground, the same plan twice is byte-equal.
  F2P: a dummy is only the recorded tape; no plan type exists.
  P2P: `cargo test -p engine-game --features app --lib staged_fight` green; golden unchanged (the
  default record fixture passes no plan).
  needs: none.

- [ ] SW81 game: the guard bar that breaks - a posture meter read off the exchange, HUD only.
  Today posture means body capsules, not guard: `crates/physics/src/stance.rs:117` ("One fighter's
  posture" = Stand/Crouch/Prone), `git grep -in "posture" -- crates` = 3 hits and none is a meter;
  stamina is a sim field (`crates/arena/src/controller.rs:131`) with no bar of its own.
  Do: `GuardPressure` per slot: a block adds cost, a deflect adds none and removes 40, 3 quiet
  seconds heal 25/tick, cost scaled 0.6 while the guard is held (sustained guard), and a break at
  1000 opens the riposte window the exchange already tracks. HUD: one bar per fighter through
  `stage_hp_bars` (`crates/game/src/console/overlay.rs:23`) so it rides `push_hp_bar(x0,y0,x1,y1,frac)`.
  Donor: https://github.com/Shadow-46/adaptive-boss-arena/blob/main/Assets/_Project/Scripts/Combat/DefenceResolver.cs
  (`ResolveDefence`, `BlockPostureMultiplier` 0.6, `ResistsStagger`) and its posture-bar colour
  growth; Sekiro rule that low vitality slows recovery (Activision mechanics page, "Posture and
  Deflection").
  Tier B. Size S. Owns new `crates/game/src/guard_pressure.rs`, `crates/game/src/console/overlay.rs`.
  Accept: `cargo test -p engine-game --features app --lib guard_pressure` prints 4 passed (0 today):
  five blocks inside 40 ticks break the guard, a deflect leaves the meter byte-equal, 180 quiet
  ticks reach 0, the same event tape twice is equal. Reads `SweepEvent::Blocked`
  (`crates/combat/src/arena.rs:219`) and `last_parry_tick`; writes no authority byte.
  F2P: no meter; a broken guard is only a denied riposte (`ExchangeError::Denied("no-parry")`).
  P2P: `cargo test -p engine-game --features app --lib overlay` green; golden unchanged.
  needs: none.

- [ ] SW82 combat: one resolver answers "did the defence hold?" for both men - perilous, unparryable,
  hyper-armour and a held guard that bleeds slower.
  Today there is no such rule set: `git grep -in "unblockable|hyper_armour|hyperarmor" -- crates/combat/src
  crates/arena/src` = 0 hits; a block is one event (`crates/combat/src/arena.rs:219 SweepEvent::Blocked`)
  and the parry window is a row field (`crates/combat/src/design.rs:59 parry_window_ticks`), so
  "defended" is answered in more than one place.
  Do: `DefenceQuery { is_defending, can_deflect, can_block, time_in_defence, deflect_window,
  unblockable, unparryable } -> DefenceOutcome { None, Deflected, Blocked }` in integer ticks;
  unblockable returns None, unparryable degrades to Blocked when `can_block`, hyper-armour refuses
  stagger in Startup/Active only and never in Recovery.
  Donor: https://github.com/Shadow-46/adaptive-boss-arena/blob/main/Assets/_Project/Scripts/Combat/DefenceResolver.cs
  (`ResolveDefence`, `ResistsStagger`, `DefenceOutcome`).
  Tier B. Size M. Owns new `crates/combat/src/defence.rs`, `crates/combat/src/arena.rs` (the block arm).
  Accept: `cargo test -p engine-combat --lib defence` prints 5 passed (0 today): unblockable through
  a live parry is None, unparryable against a blocker is Blocked, hyper-armour holds in Active and
  fails in Recovery, a 4-tick-old guard is Blocked at window 3, x2 equal.
  F2P: today a guard either blocks or it does not; nothing marks a blow unanswerable.
  P2P: `cargo test -p engine-combat --lib arena` green after the re-pin.
  needs: SW48 (its unblockable flag) and SW27 (lead 2026-10-24: lands on rules v3 only; v1 and v2 tapes replay byte-equal, the golden d57b5c8ae3096614 never moves; the v3 tape hash is pinned in this row's test).
- [ ] SW83 game: a training overlay that says why the exchange went that way - last 12 presses, the
  advantage number, and the answer the tick took.
  Today the overlay is text cells plus HP bars only: `crates/game/src/console/overlay.rs:16
  stage_hud`, `:37 push_hp_bar`, and `git grep -in "input_history|last_press" -- crates/game/src
  crates/render/src` = 0 hits; the 128-quad ring comment at `overlay.rs:7` already reserves room.
  Do: `TrainingOverlay` rows built from the same `FeelPreset` buffer the tick reads
  (`crates/combat/src/feel.rs:24 MAX_BUFFER_TICKS`, capacity at `:46`): one row per press with age,
  one row per exchange (`blocked -3`, `parry +9`, `whiff 0`), toggled by a key, never hashed.
  Donor: https://castagneengine.com/docs/modules/attacks/ref/ (`AttackFrameAdvantage`,
  `AttackFA`, `AttackFAHit`, `AttackFABlock`: advantage derived from total duration and the last
  hit frame, not authored twice) + Ikemen-GO `data/training.zss` state maps for the readout rows.
  Tier B. Size S. Owns new `crates/game/src/console/training_overlay.rs`, `console/overlay.rs`.
  Accept: `cargo test -p engine-game --features app --lib training_overlay` prints 4 passed (0
  today): 12 presses keep press order and saturate at 12, a blocked exchange row reads "-3", an
  idle tick adds no row, the same history twice is equal.
  F2P: no history, no advantage number; the HUD shows HP and wins only.
  P2P: `cargo test -p engine-game --features app --lib overlay` green; golden unchanged.
  needs: none.

- [x] SW84 combat: every attack row states its frame advantage, derived once and pinned by a test.
  Today the row carries the parts but never the answer: `crates/combat/src/design.rs:47
  buffer_ticks`, `:51 startup_ticks`, `:53 recovery_ticks`, `:55 victim_hold_ticks`, `:59
  parry_window_ticks`; `git grep -in "advantage" -- crates/combat/src` = 0 hits.
  Do: `BattlePatterns::advantage_on_hit()` = `victim_hold_ticks - recovery_ticks` and
  `advantage_on_block()` = `parry_window_ticks`-scaled block hold minus recovery, both saturating
  i32, plus `table()` printing one line per shipped row; a test pins each shipped row's number so a
  tuning edit that flips an advantage from plus to minus fails loudly.
  Donor: https://castagneengine.com/docs/modules/attacks/ref/ (`AttackFrameAdvantage`,
  `AttackHitstunBlockstun`, config `AttackDefault-Hitstun/Blockstun`, and the tutorial row
  "Frame Advantage Block: 2").
  Tier B. Size S. Owns `crates/combat/src/design.rs`.
  Accept: `cargo test -p engine-combat --lib design` prints 6 passed (4 today): SHIPPED reads
  +6 on hit and -3 on block, a slow row reads negative on both, a zero-hold row cannot overflow,
  and no derived number changes a stored field. Pure read, no golden move.
  F2P: advantage is only implicit in three separate fields.
  P2P: `cargo test -p engine-combat --lib design` green; golden untouched.
  needs: none. Landed 89b961115: design rows state frame advantage, table prints one line (run 40119568034, 11 tests); SHIPPED reads 0 on hit / -3 on block, so +6 needs a tuning move (a v3 row).

- [x] SW85 ai: the AI stops reading your inputs - it sees what a man could see, one beat late, and
  every adaptation it makes announces itself.
  Today the AI is handed the foe's next intent: `crates/ai/src/opponent.rs:166 observe_foe(foe_attacking,
  dist_mm)`, `:132 observe(band, last: Intent, next: Intent)`; `git grep -in "perception_delay|
  stale_ms|observe_delay" -- crates/ai crates/endpoint` = 0 hits. An opponent that answers a press
  the tick it is made cannot be feinted.
  Do: `Sight { facing, action, action_age, dist_mm, hp_permille }` filled only from the last staged
  frame through a fixed 16-slot ring (`PERCEPTION_TICKS = 8`, 133 ms at 60 Hz), no intent, no
  stamina, no cooldown; `ReactionBand { lo_ms: 220, hi_ms: 420 }` with a seeded miss chance; and a
  `tell` string required on every adaptation row so a counter cannot ship silently.
  Donor: https://github.com/Shadow-46/adaptive-boss-arena/blob/main/Docs/ARCHITECTURE.md
  (`DelayedPerceptionSource`, `PlayerObservation` omission table, `ReactionProfile`,
  `CounterStrategy.TellMessage`) with `Combat/DefenceResolver.cs` as the decision shape.
  Tier B. Size M. Owns new `crates/ai/src/perception.rs`, `crates/ai/src/opponent.rs` (one read site).
  Accept: `cargo test -p engine-ai --lib perception` prints 4 passed (0 today): a sight read at tick
  T equals the frame staged at T-8, a feint inside the window is never seen, the ring never grows
  past 16, the same seed twice is equal. Land the ring and its tests first: the default AI keeps its
  current read, so the golden does not move until a later row swaps it.
  F2P: intent is read the same tick it is produced.
  P2P: `cargo test -p engine-ai --lib opponent` green; golden unchanged while opt-in.
  needs: none for the module; the default swap is a separate row on rules v3 (SW27). Landed 8924c45da: 8-tick 16-slot sight ring, reaction band, a tell on every adaptation; run 40126589420 green. The default-read swap rides rules v3 (SW27), golden untouched.

#### Potato GPU tricks (wave 2)

- [ ] SW90 render: wet and polished stone reflects the arena, at half resolution and a fifth of a full-res pass -
  today the cut order names a pass that does not exist: `crates/render/src/quality.rs:355 "ssr"` sits fifth of eight
  in `CUT_ORDER`, `quality.rs:1056` gives it a unit string, and `git grep "fn ssr\|SsrConfig\|ssr_pass"` over crates
  = 0 hits (no `shaders/ssr.wgsl`, no `ssr.rs`). The depth the march needs is already there (`gtao.rs` half-res
  "ao_half" target, `gtao::upsample_bilateral`), and the Hi-Z for the mip march is SW3's pyramid.
  Do: new `crates/render/src/ssr.rs` (CPU twin: `SsrConfig { width, height, steps, depth_tolerance, distance_fade,
  curve_fade_in, roughness_cut }`, `hit_t(t)`, `resolve_blend`) + `shaders/ssr.wgsl` (compute 8x8, entry
  `ssr_trace`) that (a) skips a pixel whose roughness >= 0.7 or whose metalness is 0, (b) marches the Hi-Z from level 0
  advancing one mip up when the ray is inside geometry and one down when it is in front (`mip_offset = hit ? -1 : +1`),
  (c) normalises the ray so Z is +-1 and steps by parametric `t` with `t_max` clipped at the screen edge, (d) writes
  colour plus validity in alpha rather than rejecting a hit, (e) runs `ceil(w/2) x ceil(h/2)` on `Potato`/`Web` and
  full on the other tiers; the half-res result is composited with the existing depth-aware bilateral upsample. A
  `quality.rs` knob `ssr: Off | Half | Full` joins `Knobs` so `CUT_ORDER` finally has a target to cut.
  Donor: https://github.com/godotengine/godot/blob/master/servers/rendering/renderer_rd/shaders/effects/
  screen_space_reflection.glsl (`compute_geometric_normal`, the parametric-t Hi-Z while loop with
  `mip_offset`, `depth_tolerance`, validity in alpha); the non-Hi-Z fallback march (fixed `MAX_STEP`, `totalStep =
  max(|xLen|,|yLen|)`, `minThickness = vPNeighbor.x - vP.x` times 3, `thickness 0.018`, distance attenuation
  `ratio*ratio`, fresnel) is https://github.com/mrdoob/three.js/blob/dev/examples/jsm/shaders/SSRShader.js
  (`getViewPosition`, `viewPositionToXY`, `pointToLineDistance`).
  Tier B. Size M. Owns crates/render/src/ssr.rs, crates/render/src/shaders/ssr.wgsl, crates/render/src/quality.rs.
  Accept: `cargo test -p engine-render --features window --lib ssr` >0 tests; numbers: at 1280x720 the potato target is
  640x360 = 230 400 texels, a quarter of the 921 600 a full-res pass would touch; the CPU twin's march stops in at most
  `steps` (16 half / 8 potato), the same degenerate depth step hits at the same `t` twice, and a mirror of the wall
  under the camera returns the wall texel, not the sky.
  F2P: `ssr_half_target_is_a_quarter_of_the_texels` and `ssr_march_hits_the_wall_at_the_binary_twins_t`.
  P2P: `cargo test -p engine-render --lib quality` and `--lib gtao` stay green; golden untouched.
  needs: none (better once SW3's Hi-Z pyramid exists; the fixed-step march works before it).

- [ ] SW91 render: the sun cuts shafts through the gate arch and the wall gaps, on the potato tier, for one quarter-res
  pass - today there is no shaft pass: `git grep -in "godray\|god_ray\|light_shaft" -- crates/render` = 0 hits, fog is
  closed-form (`forward.wgsl:356-382`), and the only shaft row (RL56, `docs/SPRINT-21-look-feel.md:330`) is a Tier B
  Size L froxel volume whose own text keeps closed-form fog on potato. `Shaders/fog_vol.wgsl` already marches an
  analytic volume plus one cloud slice, so in-scatter exists but nothing turns an occluder into a ray.
  Do: new `crates/render/src/godray.rs` (`GodrayConfig { quarter_w, quarter_h, samples, decay, weight, exposure,
  density, mask_threshold }`, `mask_share(luma)`, `sum_along_ray(mask, sun_px, px)`) + `shaders/godray.wgsl`: bright-pass
  the HDR frame into a `ceil(w/4) x ceil(h/4)` mask (the emissive region the chapter sums), then additive `NUM_SAMPLES`
  taps from the pixel toward the sun's screen position with `deltaTexCoord *= 1.0/NUM_SAMPLES * Density`,
  `illuminationDecay *= Decay`, `sample *= illuminationDecay * Weight`, `return color * Exposure`; skip the whole pass
  when the sun is behind the camera or off screen, and on `Web` fall back to the 8-tap path. Composite after fog in the
  look chain (`post/chain.rs`), before tonemap.
  Donor: https://developer.nvidia.com/gpugems/gpugems3/part-ii-light-and-shadows/chapter-13-volumetric-light-scattering-post-process
  (Listing 13-1: `main()`, `deltaTexCoord *= 1.0f/NUM_SAMPLES * Density`, `illuminationDecay *= Decay`,
  `sample *= illuminationDecay * Weight`, `return float4(color * Exposure, 1)`; section 13.3 the occlusion estimate as
  "the proportion of samples that hit the emissive region", 13.5 the stochastic-sample note against banding).
  Tier B. Size S. Owns new crates/render/src/godray.rs, new crates/render/src/shaders/godray.wgsl,
  crates/render/src/post/chain.rs.
  Accept: `cargo test -p engine-render --features window --lib godray` >0 tests; numbers: at 1280x720 the mask is
  320x180 = 57 600 texels, 1/16 of the frame; 8 taps on potato, 16 on the rest; a fully occluded pixel (mask share 0)
  adds exactly 0.0 and an unoccluded one adds `exposure` times the weight sum, both pinned.
  F2P: `godray_quarter_mask_is_a_sixteenth_of_the_texels` and `godray_occluded_pixel_adds_nothing`.
  P2P: `cargo test -p engine-render --lib fog` stays green; the closed-form fog path is untouched; golden untouched.
  needs: none (RL56's froxel volume is the other, larger answer; this one ships first). Progress 2026-10-24: oracle + WGSL landed 26e241199 (godray 3 passed, run 40120102070). Left: the composite hook in post/chain.rs (a godray field on PostChain).

- [ ] SW92 render: the sun shadow stops being nine taps on every lit pixel - four rotated taps, or a dither-free
  two-tap gradient - chosen by tier - `crates/render/src/shaders/forward.wgsl:233 sun_shadow_pcf` is a fixed 3x3
  `textureSampleCompare` (9 taps, +9 shadow-map samples per lit fragment on every pass that shades a lit pixel), the
  file says so at `forward.wgsl:123`. A 4-tap corner kernel exists but only in the v1 bake:
  `crates/render/src/v1_ibl_shadow.rs:363 SHADOW_PCF_TAPS: [[f32; 2]; 4]`, four corners of a 1x1 texel box read at
  `:577-584` as `lit / 4`; it is unrotated and has no tier row. `pub struct Knobs` (`quality.rs:98`) has no
  shadow-filter field, and `git grep -in "soft_shadow\|pcf_taps" -- crates/render` = those 5 bake hits only.
  Do: `Knobs.shadow_filter: Pcf3x3 | Rotated4 | Tap2` (potato `Rotated4`, web `Tap2`), a `@override` in
  `forward.wgsl` so one pipeline still compiles, the tap rotation from the existing 4x4 Bayer table in
  `shaders/lod_dither.wgsl` (`bayer4_at`) turned into an angle, and the depth bias kept exactly as `sun_bias` returns
  it: fewer taps must not move the contact line of the fighter's feet on the sand.
  Donor: https://github.com/godotengine/godot/blob/master/servers/rendering/renderer_rd/shaders/scene_forward_lights_inc.glsl
  (`quick_hash` = the CoD:AW interleaved gradient noise, `magic = vec3(0.06711056, 0.00583715, 52.9829189)`;
  `sample_directional_pcf_shadow` rotates a disk kernel by `quick_hash(gl_FragCoord.xy + taa_frame_count * 5.588238)
  * 2*PI`, takes the centre texel alone when the sample count is 0, and averages `sc_directional_soft_shadow_samples()`
  rotated taps).
  Tier B. Size S. Owns crates/render/src/shaders/forward.wgsl, crates/render/src/quality.rs.
  Accept: `cargo test -p engine-render --features window --lib shadow` >0 tests; numbers: 9 compare taps -> 4 (5 of 9
  shadow fetches gone, 56%) and -> 2 on web (78% gone), the CPU oracle's `pcf3x3` and `rotated4` differ by under 2/255 on a flat
  wall lit at 40 degrees and by under 4/255 across a straight foot-contact edge, and the two-tap arm stays under the
  same 4/255 bar.
  F2P: `rotated_four_taps_land_within_two_levels_of_the_nine`.
  P2P: `cargo test -p engine-render --lib torch_light` and `--lib contact_shadow` stay green; golden untouched.
  needs: none (SW1's cascades and this row do not touch the same lines).

#### Procedural and physical animation (wave 2)

- [ ] SW100 anim: a fighter's arm stops popping straight when the spear tip is just out of reach -
  the two-bone solve gains a soften band. Today there is none: `git grep -n soften -- crates/anim/src` = 0 hits,
  while `crates/anim/src/ik.rs:192 two_bone` and `:143 two_bone_pole` snap to `clamp(len, |l1-l2|, l1+l2)`
  and `crates/anim/src/fbik.rs:22 FABRIK_ITERS` = 8 then takes the analytic result anyway (`fbik.rs:219`).
  Do: `two_bone_soften(root, mid, end, target, pole, soften)` in `ik.rs`; `da = (l1+l2)*soften`,
  `ds = l1+l2-da`, and past `da` the target length becomes `da + ds - ds*ratio`,
  `ratio = 81/(alpha+3)^4`, `alpha = (len-da)/ds` (derivative 1 at the band, asymptote 1);
  `fbik::solve_reach` (`fbik.rs:207`) passes `0.3` for arms, `two_bone` keeps `soften = 0`.
  Donor: https://github.com/guillaumeblanc/ozz-animation/blob/master/src/animation/runtime/ik_two_bone_job.cc
  (`SoftenTarget`, `ComputeMidJoint` law-of-cosine, `ComputeStartJoint` pole plane).
  Tier B. Size S. Owns crates/anim/src/ik.rs, crates/anim/src/fbik.rs.
  Accept: `cargo test -p engine-anim --lib ik` >0 tests; numbers: soften 0.3 of a 0.62 m chain,
  two targets 0.5 mm apart across the band move the end < 0.1 mm, a 2x-length target leaves the end at
  0.7*chain +/- 1 mm, and `soften = 0` reproduces today's pose byte for byte.
  F2P: `soften_makes_the_reach_continuous_at_the_band` and `soften_holds_the_end_short_of_a_far_target`.
  P2P: `cargo test -p engine-anim --lib fabrik` and `--lib feet` stay green; golden untouched.
  needs: none.

- [ ] SW101 anim: the head and the spear aim a point ahead of the joint, not the joint's own axis,
  and the twist follows the rival instead of rolling - the aim job gains an offset, a pole plane and a
  weight lerp. Today `crates/anim/src/ik.rs:253 look_at` reads the target as a direction ("`to` the target
  offset from the joint (its length is ignored)", :244) with yaw/pitch clamps (`AimLimits`, :222), and
  `fbik.rs:393 solve_aim` aims the joint axis itself; `git grep -n "offset.*aim\|pole.*aim\|twist" --
  crates/anim/src/ik.rs` = 0 aim hits.
  Do: `aim_offset(forward, up, offset, pole, to, limits, weight)` in `ik.rs`: the offsetted forward is
  `offset + forward*(AIl - AOl)` with `AOl = dot(forward, offset)`, `ACl2 = |offset|^2 - AOl^2`,
  `AIl = sqrt(|to|^2 - ACl2)`; rotate that onto `to`, then align the aim plane to the pole
  (`Cross(pole, to)` against `Cross(rotated_up, to)`), then `twist` about the aim axis, then shortest-arc
  NLerp from identity by `weight`; `reached = false` and identity when `ACl2 > |to|^2`.
  `solve_look`/`solve_aim` (`fbik.rs:385`, `:393`) call it with the head offset 0.09 m forward and the
  spear grip offset 0.4 m along the shaft.
  Donor: https://github.com/guillaumeblanc/ozz-animation/blob/master/src/animation/runtime/ik_aim_job.cc
  (`ComputeOffsettedForward`, `IKAimJob::Run` pole plane + `twist_angle` + `weight` NLerp; geogebra
  `ik_aim_offset.ggb` diagram in `media/doc/src`). Tier A. Size S. Owns crates/anim/src/ik.rs,
  crates/anim/src/fbik.rs.
  Accept: `cargo test -p engine-anim --lib aim` >0 tests; numbers: an offset of 1.2 m on a target 6 m away
  moves the joint's aim by < 0.2 degrees versus no offset, the joint positions of `solve_look` are
  unchanged (bit-equal before/after), an out-of-sphere target returns `reached = false` and identity.
  F2P: `aim_offset_keeps_the_same_aim_as_a_bare_target` and `aim_beyond_the_offset_sphere_refuses`.
  P2P: `cargo test -p engine-anim --lib ik` stays green; `--lib fbik` cost receipts unchanged.
  needs: SW100 (same function's soften path).

- [ ] SW102 anim: plumes, straps and the crest stop passing through the shoulder and the helmet -
  spring bones gain sphere/capsule collider push-out. Today `crates/anim/src/springs.rs:162 SpringSet`
  integrates a spring-damper per bone (`:230 step`, `:199 add_bone`, `:213 kick`, `MAX_BONES` = 64) with
  no collider at all: `git grep -n -i "collider\|sphere\|capsule" -- crates/anim/src/springs.rs` = 0 hits.
  Do: `SpringCollider::{Sphere{center_q, radius_q}, Capsule{head_q, tail_q, radius_q}}` plus
  `SpringSet::collide(&[[i32; 3]])`, run after `step` and before `apply_to_pose` (`:309`): distance =
  `|tail - nearest| - bone_r - collider_r`, and when < 0 the tail moves along the delta by `-distance`
  and its length is re-normalized to the bone length (integer Q12, stable bone/collider order, capped at
  8 colliders).
  Donor: https://github.com/pixiv/three-vrm/blob/dev/packages/three-vrm-springbone/src/
  VRMSpringBoneColliderShapeSphere.ts + `VRMSpringBoneColliderShapeCapsule.ts` (`calculateCollision`:
  nearest point by `dot(tail-head, obj-head)/|tail-head|^2`, then `distance = length - objectRadius -
  radius`, push `target/|target|`), with the re-normalize step from
  https://github.com/vrm-c/UniVRM/blob/master/Packages/VRM/Runtime/SpringBone/Logic/SpringBoneSystem.cs
  (`UpdateProcess`, "長さをboneLengthに強制" after each collider). Tier B. Size M. Owns
  crates/anim/src/springs.rs.
  Accept: `cargo test -p engine-anim --lib springs` >0 tests; numbers: 8 bones + 3 spheres + 2 capsules
  stay inside `OPS_PER_LANE_TICK` (8/bone) + 24/collider lane, a tail 1 mm inside a sphere ends 0 mm
  inside, `max_offset_q` never past `VMAX_Q`, same impulse script digests identically twice.
  F2P: `a_crest_tail_never_ends_inside_the_helmet_sphere` and `capsule_push_out_keeps_the_bone_length`.
  P2P: `cargo test -p engine-anim --lib springs` green; `--lib lod` untouched; golden untouched.
  needs: none.

- [ ] SW103 game: the crest and the tunic skirt swing when the fighter turns, and kick when he is hit -
  the dress rig is wired to the springs that already exist. Today nothing constructs a spring set:
  `git grep -n "SpringSet" -- crates` finds matches only inside `crates/anim/src/springs.rs` (its own
  tests), and the kit's dress is rigid skin: `fighter_kit.rs:883` "The galea and its crest",
  `:781` "The tunic: a belt and a red skirt", `:946 CREST_HALF_THICKNESS_M` = 0.0075.
  Do: new `crates/game/src/fighter_look_dress.rs`: a `Dress` per slot (crest 3 bones, tunic skirt 6
  bones, 2 straps, `MAX_BONES` 16), anchors read from the drawn pose each frame, `apply_to_pose` writes
  the tips back, colliders from the same pose (spine capsule, two thigh capsules, helmet sphere);
  `kick` on the hit tick from `hit_react`'s push (`crates/anim/src/hit_react.rs:169`), amplitude by
  `HitDir`, plus a turn-rate kick from `turn_in_place`'s yaw rate. Frozen slots (LOD far tier) skip the
  step and keep their anchor offset.
  Donor: the same VRM pair as SW102 for the constraint order, plus
  https://github.com/vrm-c/UniVRM/blob/master/Packages/VRM10/Runtime/Components/SpringBone/
  VRM10SpringBoneJoint.cs for the per-joint limits (pitch/yaw cone, `m_stiffnessForce` 1.0, `m_dragForce`,
  `m_hitRadius`). Tier B. Size M. Owns new crates/game/src/fighter_look_dress.rs,
  crates/game/src/fighter_look.rs (one call in `plant_feet`'s frame path).
  Accept: `cargo test -p engine-game --features app --lib dress` >0 tests; numbers: crest tip travel
  4-8 cm over a 180 degree turn, 0 skirt vertices inside a thigh capsule across a 600-tick run,
  the whole dress <= 0.1 ms for 8 fighters, a headless capture line `view=ok` with the crest tilted
  on a turning frame.
  F2P: `a_turning_fighter_lags_his_crest_then_settles` and `a_hit_kicks_the_skirt_off_the_thigh`.
  P2P: `cargo test -p engine-game --lib fighter_look` stays green; presentation only, golden untouched.
  needs: SW102 (colliders first, or the crest swings through the helmet).

- [x] SW104 anim: a second hit during a flinch rides the first instead of snapping the body back -
  overlapping flinches blend. Today a flinch is one windowed additive pass:
  `crates/anim/src/hit_react.rs:57 VIEW_FLINCH_TICKS` (27 ticks), `:351` the weight curve,
  `:263 hit_mask`, `:464 apply_hit_react` adds one strike at one `weight` (`:424`), and nothing carries
  the decaying lean of strike 1 into strike 2 - `git grep -n "overlap\|pending\|subsequent" --
  crates/anim/src/hit_react.rs` = 0 hits.
  Do: `FlinchStack` (2 lanes, oldest first) keeping each strike's `(tick, HitDir, weight)`; the drawn
  delta is the sum of lane deltas scaled by `weight_at(age)` with the later lane's perpendicular axis
  orthonormalized against the earlier one; a third strike replaces the oldest lane at its decayed weight
  (no snap), and per-bone caps reuse `PEAK_M` (`:46`, 0.10 m) and `ROT_PEAK_RAD` (`:54`, 0.15 rad) so the
  sum never passes 1.6x one strike.
  Donor: https://github.com/Vaei/ProcHitReact/blob/main/Source/ProcHitReact/Private/HitReact.cpp
  (`UHitReact::HitReact` pending-impulse path; profile `SubsequentImpulseScalars`, ascending-time
  validation in `Private/HitReactProfile.cpp`) - we keep our integer pose path, only the overlap policy
  is ported.
  Tier B. Size S. Owns crates/anim/src/hit_react.rs.
  Accept: `cargo test -p engine-anim --lib hit_react` >0 tests; numbers: strikes 6 ticks apart, the drawn
  head position moves < 2 mm between tick 11 and 12 (no snap), two opposite strikes peak at <= 1.6x one,
  the same strike script digests identically twice.
  F2P: `a_second_strike_six_ticks_later_does_not_snap_the_lean` and
  `two_opposite_strikes_stay_under_the_peak_cap`.
  P2P: `cargo test -p engine-anim --lib hit_react` green; `--lib additive` green; golden untouched.
  needs: none. Landed 49edb34bf: FlinchStack (2 lanes, Gram-Schmidt), seam < 2 mm and third-strike jump < 2 mm asserted, OVERLAP_CAP 1.6; runs 40126035224 (18) and 40126391669 (7) green. No live caller of apply_flinches yet (injured.rs/ragdoll_look own the call).

#### Adaptive audio (wave 2)

- [ ] SW110 audio: the mix stops cracking when eight fighters trade blows - a real limiter replaces the hard clip.
  Today `crates/audio/src/dsp.rs:100 BusMixer::render_frame` sums the three buses and returns
  `(m * master).clamp(-1.0, 1.0)` at `dsp.rs:102`, i.e. a hard clip with no ceiling, no lookahead and
  no makeup; `git grep -in "limiter" -- crates/audio` = 0 hits, and `device.rs:577` only saturates the
  i16 cast. Do: new `crates/audio/src/limiter.rs` with `Limiter { ceiling_q, pre_gain_q, attack_ms,
  release_ms, delay_ring, gain_buckets }`, `process(&mut [f32])` in place before the clamp, lookahead
  delay of `ceil(sample_rate * attack) + 1` frames, bucket gain = min over the buckets, soft knee
  below the ceiling; hook `dsp.rs:100 render_frame`, `dsp.rs:111 mix_into` and the engine block before
  the device write.
  Donor: https://github.com/godotengine/godot/blob/master/servers/audio/effects/audio_effect_hard_limiter.cpp
  (`AudioEffectHardLimiterInstance::process`, `instantiate`) and
  https://github.com/godotengine/godot/blob/master/servers/audio/effects/audio_effect_limiter.cpp
  (`AudioEffectLimiterInstance::process`; defaults ceiling -0.1 dB, soft_clip 2 dB, ratio 10).
  Tier A. Size S. Owns crates/audio/src/limiter.rs, crates/audio/src/dsp.rs, crates/audio/src/lib.rs.
  Accept: `cargo test -p engine-audio --lib limiter` >0 tests; numbers: 64 voices at full gain peak at
  0.9886 (-0.1 dBFS) instead of clipping, 0 samples with |x| > 1.0, gain back to 1.0 within `release`,
  latency == `ceil(48000*0.005)+1` = 241 frames at a 5 ms attack.
  F2P: `limiter_holds_the_ceiling_under_sixty_four_voices` and `limiter_gain_recovers_inside_release`.
  P2P: `cargo test -p engine-audio --lib dsp` stays green; golden untouched.
  needs: none.

- [ ] SW111 audio: the crowd is on four sides of the sand, so a knockdown at the north wall is heard in the south stand a beat later.
  Today the crowd is one global state: `crates/audio/src/crowd.rs:134 pub struct Crowd` holds
  `duck_remaining`/`duck_total` and one `bed_gain()` (`crowd.rs:178`); one `Reaction` per event
  (`crowd.rs:193`); `git grep -n "pos_cm\|position" -- crates/audio/src/crowd.rs` = 0 hits, so no stand,
  no distance, no pan. The gunfire layer already delays by `SPEED_OF_SOUND_CM_PER_S = 34_300`
  (`gunfire.rs:33`); the crowd ignores travel time. Do: new `crates/audio/src/crowd_stands.rs` with
  `Stands { seats_q16: [u16; 4], excite_q16: [i32; 4], pending: [u64; 4] }`, one seeded seat weight per
  stand, excitement +level on a cue at that stand and 10% leak to the neighbours per tick, a reaction
  scheduled at `dist_cm * 1000 / 34_300` ms (fixed tick count) and played through the existing pan with
  a 6 dB near-stand lift; the chant claps on `music::STEPS_PER_BEAT` so it sits on the score.
  Donor: https://github.com/jarikomppa/soloud/blob/master/src/core/soloud_core_3d.cpp
  (`update3dVoices_internal` per-voice `vol` from the attenuator + speaker-dot pan, `attenuateInvDistance`);
  the travel delay reuses our own `gunfire.rs:33`.
  Tier B. Size M. Owns crates/audio/src/crowd_stands.rs, crates/audio/src/crowd.rs.
  Accept: `cargo test -p engine-audio --lib crowd_stands` >0 tests; numbers: a knockdown at the north
  stand reaches the south stand 80 m away 233 ms later (8000 cm / 34_300 cm/s), the near stand reads
  +6 dB, 4 stands cost 0 heap allocations per tick, and the same seed + cue order replays the same
  reaction times.
  F2P: `crowd_wave_arrives_at_the_far_stand_after_233_ms` and `stand_excitement_leaks_to_neighbours`.
  P2P: `cargo test -p engine-audio --lib crowd` (cooldown tests) stays green; golden untouched.
  needs: none.

- [x] SW112 audio: a blow echoes off the wall behind you before the tail starts - early reflections sized from the arena, not a fixed preset.
  Today the room answer is one late tail: `crates/audio/src/reverb.rs:97 ReverbParams::tunnel` (pre-delay
  18 ms, RT60 1100 ms, damping 0.65) built from `apply_comb` (`reverb.rs:346`) plus `apply_allpass`
  (`reverb.rs:384`); `gunfire.rs:175 Zone::contains` only picks a zone and `gunfire.rs:146` is the
  street "longer slap" comment. `git grep -in "early\|reflection" -- crates/audio/src/reverb.rs` shows
  only decay-window maths (`reverb.rs:219 decay_db`, `reverb.rs:664` reference window), 0 early taps.
  Do: new `crates/audio/src/early_reflections.rs` with `taps(dist_cm) -> [Tap; 4]`, `r_a = d / 343.3`,
  `r0 = 2/3 r_a`, `r1 = 4/3 r_a`, `T_i = (2^(i/7) - 1) * (r1 - r0)`, each tap gained by the nearest
  wall's `occlude::absorption_q16`; four integer delay lines summed into the existing wet bus, no
  allocation on the audio thread.
  Donor: https://github.com/kcat/openal-soft/blob/master/alc/effects/reverb.cpp
  (`EARLY_TAP_LENGTHS`, `EARLY_ALLPASS_LENGTHS`, `DensityScale`, the `r_a`/`r_0`/`r_1` derivation).
  Tier B. Size S. Owns crates/audio/src/early_reflections.rs, crates/audio/src/reverb.rs.
  Accept: `cargo test -p engine-audio --lib early_reflections` >0 tests; numbers: a wall 20 m away
  gives the first tap at 39 ms (2/3 * 20 m / 343.3 m/s), then 43/47/52 ms, a stone tap 3 dB above a
  sand tap, 0 allocations in `process`.
  F2P: `twenty_metre_wall_returns_four_taps_at_39_43_47_52_ms` and `sand_tap_is_lower_than_stone_tap`.
  P2P: `cargo test -p engine-audio --lib reverb` (impulse + decay tests) stays green; golden untouched.
  needs: none. Landed 0a95fd207: taps at 39/43/47/52 ms for a 20 m wall, wall gain from the absorption row, no audio-thread alloc; runs 40126187494 (7) and 40126585019 (20) green.

- [ ] SW113 audio: a thrown spear and a charging fighter change pitch as they pass, instead of sliding by at one pitch.
  Today nothing in the mix has a playback rate: `git grep -n "doppler" -- crates/audio` = 0 hits;
  `spatial.rs` carries `Pos2` and `pan_for` only; the only pitch shift in the crate happens at load,
  `cue.rs:366` (`Clip::from_pcm_at_rate`, linear resample) and the bank's `variant` offsets
  (`bank.rs:88`). So a source closing at 10 m/s sounds identical to a static one.
  Do: new `crates/audio/src/doppler.rs` with `ratio(speaker_cm_s: [i32; 3], source_cm_s: [i32; 3],
  listener_vel: [i32; 3]) -> i32` in Q12 (`(c - f*v_l) / (c - f*v_s)`, `c = 34_300` cm/s, clamp
  0.5..=2.0, snap to 1.0 under 50 cm/s); feed it to the voice's step/read cursor beside the existing
  resample path, listener velocity from the camera delta per tick, source velocity from the cue owner.
  Donor: https://github.com/jarikomppa/soloud/blob/master/src/core/soloud_core_3d.cpp
  (`doppler`, incl. the `maxspeed = aSoundSpeed / aFactor` clamp and `vls`/`vss` MIN).
  Tier B. Size S. Owns crates/audio/src/doppler.rs, crates/audio/src/mix.rs.
  Accept: `cargo test -p engine-audio --lib doppler` >0 tests; numbers: closing 1000 cm/s gives Q12
  1.030, receding gives 0.971, both snap to 1.000 under 50 cm/s, clamp holds at 2.0 for a 30 000 cm/s
  source, 0 heap allocations per voice tick.
  F2P: `closing_source_speeds_up_and_receding_slows_down` and `doppler_snaps_to_unity_at_rest`.
  P2P: `cargo test -p engine-audio --lib spatial` stays green; golden untouched.
  needs: none.

- [ ] SW114 audio: walking out of the tunnel into the street eases the tail over instead of snapping it.
  Today one zone owns the acoustics: `reverb.rs:141 pub struct ReverbZone` (one center + params),
  `gunfire.rs:175 Zone::contains(pos_cm)` returns a single zone, and `gunfire.rs:456 report` builds one
  tail from it; `git grep -n "blend\|crossfade" -- crates/audio/src/reverb.rs
  crates/audio/src/gunfire.rs` = 0 hits. Crossing the mouth swaps RT60 1100 ms -> 600 ms and pre-delay
  18 ms -> 12 ms in one block, so the old tail is dropped and the new one starts mid-decay.
  Do: new `crates/audio/src/zone_blend.rs` with `Blend { zones: [u8; 2], weight_q16: [i32; 2] }`: at
  most two zones active, weight from distance to the zone edge with a 200 cm feather and 60 cm
  hysteresis, two `Reverb` instances fed the same send, equal-power weights (`sqrt`) over one
  `RT60/4` crossfade, and the total wet held constant.
  Donor: https://github.com/kcat/openal-soft/blob/master/alc/effects/reverb.cpp
  (`MinDecayTime`/`MaxDecayTime` 0.1..20 s, `DefaultModulationTime` 0.25 s, the density -> line-length
  cube-root mapping used to keep total energy while the room changes).
  Tier B. Size M. Owns crates/audio/src/zone_blend.rs, crates/audio/src/gunfire.rs.
  Accept: `cargo test -p engine-audio --lib zone_blend` >0 tests; numbers: a listener 50 cm inside the
  tunnel edge reads 25/75 tunnel/street, weights square to 1.0, hysteresis keeps 1 switch on a 3 cm
  jitter walk that flips 8 times without it, and the wet RMS never drops more than 3 dB across the walk.
  F2P: `zone_weights_crossfade_without_a_wet_dip` and `hysteresis_stops_flip_on_a_jitter_walk`.
  P2P: `cargo test -p engine-audio --lib gunfire` (zone report tests) stays green; golden untouched.
  needs: none.

#### Free realistic assets (wave 2)

- [ ] SW120 content: the arena sky is the real Colosseum morning sun, not a synthetic preset -
  Colosseum 2k HDRI (Rome, 41.8904/12.4936) as the outdoor day probe plus its backplates.
  Today the only third-party HDRI wired into code is the studio probe:
  `crates/assets/src/kit_import.rs:51 pub const STUDIO_HDRI` and
  `crates/render/src/ibl_arena_env.rs:249 include_bytes!(...polyhaven-studio-small-09-1k.hdr)`;
  the Colosseum row exists in `content/fetch.list:16` (md5 `660eb208…`) but no
  `*colosseum*` byte is anywhere under `content/` (`Get-ChildItem -Recurse -Filter *colosseum*`
  → 0 files). Sky presets stay analytic: `content/sky/preset_day.json` (variance only).
  Do: fetch `https://dl.polyhaven.org/file/ph-assets/HDRIs/hdr/2k/colosseum_2k.hdr`
  (6,090,156 B, API md5 `660eb20813279a49ca70605e89f3a108`, CC0 per
  `https://polyhaven.com/license`, authors Greg Zaal (HDRI) / Rico Cilliers (backplates),
  EVS cap 22, white balance 5400 K, 2048x1024) into `content/arena/hdri/arena_rome_2k.hdr`
  beside the three potato skies; one PROVENANCE.md row (URL + CC0 + author + SHA-256 + bytes);
  `IblArenaEnv` gains a `Rome` slot so the establishing view picks it when `arena_rome` is set.
  Tier A. Size S. Owns content/arena/hdri/PROVENANCE.md, crates/render/src/ibl_arena_env.rs.
  Accept: `cargo test -p engine-render --lib ibl_arena_env` >0 tests, and the new
  `rome_hdri_parses_to_2048x1024_and_its_sha_matches_the_row` (byte length 6,090,156,
  2048x1024, sha256 in the row); `node --test tools/provenance-audit.test.mjs` green.
  F2P: `rome_hdri_parses_to_2048x1024_and_its_sha_matches_the_row`.
  P2P: `cargo test -p engine-assets provenance` stays green; the studio probe is untouched.
  needs: none (potato tier keeps the 1k arena_day_1k as the fallback when VRAM < 3 GB).

- [ ] SW121 content: the arena statues are real scanned marble, not sandstone stand-ins -
  Poly Haven `Marble Bust 01` (Rico Cilliers, 17,456 tris, "marble", "bust", "statue").
  Today the marble binds sandstone and says so:
  `content/arena/ref-sheets/from-design-studio/O-032/SOURCES.md:3` "Marble stand-in crops:
  Poly Haven `sandstone_blocks_04` 2k" and `O-032/DELIVERY.md:6` "Skin has no engine asset and
  marble binds sandstone … NEED-11 tracks real CC0 photo sets"; the only statue-ish bytes are
  `content/arena/textures/ambientcg-white-marble.png` (332 B, a 1x1 tint probe).
  Do: `https://dl.polyhaven.org/file/ph-assets/Models/gltf/1k/marble_bust_01/marble_bust_01_1k.gltf`
  (2,793 B, md5 `599d6dd7…`) + its 4 `include` maps (diff/nor_gl/arm/rough at 1k, same API entry)
  → `content/props/polyhaven/marble_bust_01/`, CC0, one PROVENANCE row; run it through
  `crates/assets/src/gen_intake.rs` with `max_tris: PROP_MAX_TRIS` (5,000) so 17,456 → <=5,000
  LOD0; place one 2.2 m bust per wall niche in `content/arena/dressing.json`
  (`crates/game/src/windowed_dressing.rs:24` embeds that JSON; its frozen budget line says
  "0 new tris", so this row re-baselines that line to +5,000 and says so).
  Tier B. Size M. Owns content/props/polyhaven/, content/arena/dressing.json,
  crates/game/src/windowed_dressing.rs.
  Accept: `cargo test -p engine-assets --features import,registry --test it polyhaven_bust`
  (new) proves <=5,000 tris, cooks byte-identical twice, bundle loads;
  `cargo test -p engine-game --lib windowed_dressing` (dressing parses 2 new actors).
  F2P: `marble_bust_cooks_under_5000_tris_and_twice_equal`.
  P2P: `node --test tools/ai-props.test.mjs` and the CT53 dressing test stay green.
  needs: none.

- [ ] SW122 content: the podium wall and the marble trim get a real veined marble set, the one
  material the arena still fakes. Today `content/arena/textures/stone_*_2k.jpg` is
  Poly Haven `sandstone_blocks_04` for both stone and marble (`content/PROVENANCE-AUDIT.md:41-43`),
  the travertine row `url travertine004_1k` in `content/fetch.list` (`ambientCG Travertine004
  1K-JPG.zip`, 3,487,181 B, md5 `14597a5c3b231d53b01b6cd711fc28d3` per the list) never landed
  (`git ls-files content` → 0 travertine paths), and `content/materials/siege/marble.json` is
  parameter-only (MIT/Apache, "no texture bytes").
  Do: fetch that zip (CC0 per `https://ambientcg.com/view?id=Travertine004`, "All assets are
  released under the Creative Commons CC0 license"), keep `Color/NormalGL/Roughness/AO` 1K JPG,
  plus `marble_01` 1k diff/nor_gl/rough/ao from the URLs already pinned in `content/fetch.list`
  (Poly Haven, Rob Tuytel, CC0); cook both through the arena_cook bin beside the 15 KTX2 maps.
  Tier A. Size M. Owns content/arena/sources/, content/arena/cooked/, content/arena/PBR-INTAKE.md.
  Accept: `cargo test -p engine-assets --features import --test it arena_cook` stays green and
  grows 6 maps (15 → 21 KTX2, `cook.json` PSNR unchanged for the old 15);
  `node --test tools/provenance-audit.test.mjs` green with one row per new map.
  F2P: `travertine_and_marble_cook_twice_equal`.
  P2P: the existing 15-map byte comparison must still pass unmodified.
  needs: none (potato: 1k only, 6 maps ≈ 2.6 MB source).

- [ ] SW124 content: the arena sand reads as sand up close on a potato — a real 1k sand set
  layered over the flat Ground093C colour. Today the floor is `Ground093C` 1K JPG
  (`content/arena/sources/PROVENANCE.md`, "sand (arena ground)") cooked to
  `cooked/sand_base/normal/orm.ktx2`; there is no second sand set and no macro/micro pair
  (`git grep -il "dense_sand\|microdetail" -- content crates` → 0 hits).
  Do: fetch `dense_sand` 1k diff/nor_gl/arm/rough from
  `https://dl.polyhaven.org/file/ph-assets/Textures/jpg/1k/dense_sand/dense_sand_diff_1k.jpg`
  (681,515 B, md5 `1c3d2c1f…`), `…_nor_gl_1k.jpg` (993,025 B, `ade5aef2…`),
  `…_arm_1k.jpg` (685,180 B, `1310b206…`), `…_rough_1k.jpg` (476,589 B, `5e14009b…`),
  CC0, Dimitrios Savva; add a second UV-tiled sand material in `content/arena/placement.json`
  (`texture_scale` 8x on the arena floor) so the sand has 2 m and 0.25 m detail at once.
  Tier A. Size S. Owns content/arena/sources/, content/arena/cooked/, content/arena/placement.json.
  Accept: `cargo test -p engine-assets --features import --test it arena_cook` green with
  4 new maps (15 → 19) and `node tools/arena-report.mjs` clean on the new manifest rows.
  F2P: `dense_sand_cooks_twice_equal`.
  P2P: the 15 committed KTX2 maps stay byte-equal.
  needs: none (1k total 2.8 MB; comfortable tier can take the 2k twins later).

CW: two Poly Haven models that were on my shortlist but do not earn a row yet —
`sand_rocks_small_01` is 1,386,500 tris (decimate to 5k loses the scan look; revisit as a
far-only scatter mesh), `stone_01` 71,916 tris with no 1k-only gltf worth the bandwidth.

---

## Rejected (with the line I read)

- Scan the World (MyMiniFactory) — Roman busts/Colosseum scans are CC-BY-NC-SA; NC is out.
- Sonniss GDC bundles — free-to-use but the licence forbids shipping the raw files as assets
  (`content/` is git-tracked); 20-30 GB per bundle is also past the 5 MB commit bar.
- Megascans / Fab / Quixel Bridge — Unreal-only licence, excluded by the brief.
- `kite_shield` (Poly Haven, CC0) — a medieval kite shield, not a scutum; look, not Rome.
- Smithsonian Open Access 3D — `https://3d-api.si.edu/search?q=roman` and
  `https://3d-api.si.edu/content?q=roman` both answer 404; the documented download path wants
  a real `3d_package:` uuid plus an api.si.edu key this seat does not hold. No bytes read, no row.
- Poly Haven `stone_wall`/`large_sandstone_blocks_01` — we already ship
  `sandstone_blocks_04` 2k as `stone_*_2k.jpg`, so a second ashlar set buys nothing.
- Quaternius/Poly Pizza props — already landed (`content/props/roman/`), and Poly Pizza is
  CC-BY 3.0, not CC0; no new realistic Roman prop found there today.

## Dead ends

- ambientCG model/texture search has no Roman category and its API serves no md5 for new
  asset ids (same wall GS22 v2 hit); only the zip-url rows with a scouted md5 are usable.
- Poly Haven has no travertine texture (`trav*` → 0 ids) and no Roman helmet/scutum/gladius
  model (521 models scanned: only `kite_shield`, `marble_bust_01`, `bronze_ray_statue` come close).
- OpenGameArt "medieval sound effects - weapon impacts" page renders with no file links
  (0 `sites/default/files` hrefs), so it cannot be pinned; the starninjas pack can.

#### Melee AI and the match director (wave 2)

- [x] SW130 ai: the match director paces a live tension gauge instead of a static ladder - it fills
  while a viable target is latched, decays only after a grace delay, freezes during the peak and the
  cooldown, and a quota caps how many peaks a bout may have.
  `crates/ai/src/director.rs:31 tension()` is the only pacing scalar (`intensity / cap`, no fill curve,
  no decay, no grace, no peak freeze, no quota; `git grep -in "grace\|hysteresis\|quota" -- crates/ai`
  hits only the LLM quota in `mind.rs:222`) and `crates/ai/src/depth.rs:1503 TENSION_LADDER` is seven
  fixed rows keyed by rung. Add `PressureGauge` in `crates/ai/src/director.rs`: integer permille
  progression, `fill = p + (1000 - p) * dt_ms / fill_ms`, decrease `dt_ms / decrease_ms` after
  `decrease_delay_ms`, `AggressiveThreshold`, `MaxPeaks`, `CooldownMs`, `FreezeDuringPeak`, plus
  `Mode::{Relax,Build,Peak,Fade}` transitions and a bounded reason ring.
  Donor: https://github.com/zeljkovranjes/sbox-two-brain-director/blob/main/Code/SboxTwoBrains/Core/PressureDirector.cs
  (`PressureDirector.Tick`, step 4 gauge fill/decrease, step 5 Normal->Aggressive, step 6 sweep end)
  and `Core/PressureState.cs`, `Core/PressureSection.cs` (FillSeconds 0.5..600, DecreaseDelaySeconds
  0..600, AggressiveThresholdProgression (0,1], MaxOpportunities 1..100),
  `tests/SboxTwoBrains.Tests/Macro/GaugeFormulaTests.cs` (`Fill_MatchesResearchFormulaExactly_Over120Ticks`,
  `Decrease_WaitsForDelay_ThenDecreasesLinearly`, `Gauge_Frozen_WhileAggressive`, `Gauge_DoesNotFill_DuringCooldown`).
  Tier B. Size S. Owns `crates/ai/src/director.rs`.
  Accept: `cargo test -p engine-ai --lib director` >0 passed with new
  `pressure_gauge_fills_asymptotically_clamps_and_decays_after_grace` (fill_ms 2000, dt 16 ms, 120 ticks:
  monotone, < 1000, then a lost latch holds during grace and decays to 0, never negative) and
  `peak_freezes_gauge_and_quota_caps_cycles` (peak holds progression, quota stops the third peak,
  the reason ring names `quota_blocked`). F2P: pacing is a static per-rung table, so a bout cannot
  build to a peak and back down. P2P: `--lib director` and `--lib depth` green; integer permille, no
  float in the hash path, no shipped call site, so the golden holds.
  needs: none. Landed 4ace21db4: PressureGauge in director.rs (7 passed, run 40119722705); no shipped caller yet.

- [x] SW131 ai: the duelist baits instead of only rushing - bait is the resting stance, and moving in
  is a timed window it opens once every ~2 s, closes after 1 s, and aborts the moment the foe turns
  invulnerable, so a punish lands on a whiffed answer rather than on a coin flip.
  `git grep -in "bait" -- crates/ai` = 0 hits. `crates/ai/src/depth.rs:586 FeintState` only *reacts* to
  the foe's feint (`FEINT_WINDOW_TICKS` 12, `FEINT_ATTACK_BONUS` 120) and
  `crates/ai/src/melee.rs:217 score_bias` punishes a recovering foe; nothing offers a feint.
  Add `BaitLadder` in `crates/ai/src/depth.rs`: `bait_rng` counter, `approach_window` bool,
  `approach_tick`, consts `BAIT_OPEN_CHANCE_PERMILLE_PER_TICK` (1000/121 ~ 8), `APPROACH_WINDOW_TICKS`
  60, `BAIT_RESET_ON_INVULNERABLE`, and a fixed priority ladder
  `mitigate > wait > punish > defend > challenge > approach > keep_distance`.
  Donor: https://github.com/altf4/SmashBot/blob/master/Strategies/bait.py (`Bait.step`: the
  `random.randint(0, 120) == 0` approach roll, the 60-frame `approach_frame` reset, the
  `opponent_state.invulnerability_left > 0` abort, and the ordered tactic gates
  `Punish.canpunish` before `Defend.needsdefense`).
  Tier B. Size S. Owns `crates/ai/src/depth.rs`.
  Accept: `cargo test -p engine-ai --lib depth` >0 passed with new
  `bait_is_the_resting_stance_and_approach_is_a_closed_window` (seeded counter over 600 ticks opens
  the window 4..6 times, each window <= 60 ticks) and
  `foe_invulnerability_aborts_the_approach_window`. F2P: the duelist never baits, so the player is
  never punished for a read. P2P: `--lib depth` and `--lib melee` green; the ladder is opt-in and no
  shipped policy reads it, so the golden holds.
  needs: none. Landed 3dc832ac2: bait as resting stance + timed approach window; run 40126620710 green, 62 depth tests, 3 new.

- [x] SW132 ai: the crowd gets a lull - the director sends the opponent offstage for a fixed sweep
  window, it re-enters through the nearest gate, and the same decision that took it offstage cannot
  send it back again.
  `git grep -in "offstage\|ingress" -- crates` = 0 hits for both; `crates/ai/src/eqs.rs:878
  build_ambush_query` picks a nest but never leaves the pit, and `crates/ai/src/director.rs:31
  tension()` has no window concept.
  Add `crates/ai/src/director.rs` (or `depth.rs`) `OffstageWindow { sweep_ticks_remaining,
  entry_gate, block_opportunity }`: enter only in `Peak`, pick the least-recently-used gate
  (`visit_stamp` per gate), hold `sweep_ticks`, exit through the nearest gate, refuse re-entry while
  `block_opportunity` is the current one.
  Donor: https://github.com/zeljkovranjes/sbox-two-brain-director/blob/main/Code/SboxTwoBrains/Core/OffstageModule.cs
  (`OffstageModule.Evaluate`: `sweep_ticks`, the `OffstageBlockOpportunityId` refusal, the
  least-recently-visited-node pick, `PickIngress` nearest-first ordinal tie-break) plus
  `Core/PressureState.cs` (`SweepSecondsRemaining`, `IngressBanRemaining`).
  Tier B. Size M. Owns `crates/ai/src/director.rs`.
  Accept: `cargo test -p engine-ai --lib director` >0 passed with new
  `sweep_window_expires_and_the_entry_gate_is_banned` (one entry, sweep held N ticks, exit through the
  nearest gate, a second request on the same opportunity refused) and
  `gate_pick_is_least_recently_used_with_ordinal_tie_break` (two gates, equal distance: id order wins).
  F2P: the opponent never leaves the pit, so there is no breather and no re-entry beat. P2P:
  `--lib director` green; pure decision data, no spawn call site, golden holds.
  needs: none. Landed 509bb33d8: LRU exit gate, nearest re-entry, one sweep per opportunity; run 40126589420 green, 19 tests.

- [x] SW133 ai: an ambusher that hides on a memory, not on a live sighting - it holds the nest until a
  timeout, then gives up and rejoins the fight, so a stalking opponent reads as patient instead of
  stuck.
  `crates/ai/src/eqs.rs:875 build_ambush_query` chooses the nest from line of sight to the target;
  there is no ambush state, timeout or memory-source gate (`git grep -in "ambush_active\|role_timeout"
  -- crates` = 0 hits) and `crates/ai/src/opponent.rs` tracks intents, not holds.
  Add `AmbushHold { ticks_left, target_id, node_id, from_memory }` to `crates/ai/src/depth.rs`:
  enter only when the last evidence is remembered (no contact this window) and the actor holds the
  `ambusher` role from `depth.rs` `Squad`; `AMBUSH_TIMEOUT_TICKS` clears the flag and emits
  `ambush_timeout`.
  Donor: https://github.com/zeljkovranjes/sbox-two-brain-director/blob/main/Code/SboxTwoBrains/Core/AmbushModule.cs
  (`AmbushModule.Evaluate`: `best.Source != EvidenceSource.Memory` gate, role gate, timer expiry branch).
  Tier C. Size S. Owns `crates/ai/src/depth.rs`.
  Accept: `cargo test -p engine-ai --lib depth` >0 passed with new
  `ambush_holds_on_memory_and_clears_on_timeout` (memory evidence enters, live contact refuses,
  timeout clears the flag with the reason code). F2P: no hold state exists, so an ambush cannot be
  timed out or read. P2P: `--lib depth` and `--lib eqs` green; opt-in flag, golden holds.
  needs: none. Landed edf003575: AmbushHold with a 120-tick timeout and ambush_timeout reason (depth 59 passed run 40120022832, eqs 26 passed run 40120720823); left: the eqs nest pick reading the hold.

- [ ] SW134 game: the 96 spectators answer the fight - a crowd heat gauge fed by blows, near misses and
  the director's peak raises a hush or a roar, and the stands share it with a one-beat lag.
  `crates/game/src/crowd_stands.rs:81 crowd_instances` and `:136 stage_crowd` place static impostors
  (seeded facing and size only, `SPECTATOR_HEIGHT_M` 1.62) and `git grep -in "cheer\|roar\|gasp" --
  crates/game crates/ai` hits only dialogue strings and the audio bank (`crates/audio/src/bank.rs:472
  Synth::Roar`), so no spectator state exists.
  Add `crates/game/src/crowd_mood.rs`: `CrowdMood { heat_permille, lagged_permille }`, `heat_step`
  (blow = +180, near miss = +60, peak = +300, decay 8/tick to a floor set by the director's
  `crowd_volume(rung)`), and `instance_cheer(seat, heat)` feeding `crowd_instances`.
  Donor: (no donor read: see DEAD ENDS; this row is the engine's own gap, taken from the brief's
  spectator ask and priced against `crowd_stands.rs`).
  Tier B. Size M. Owns `crates/game/src/crowd_mood.rs`.
  Accept: `cargo test -p engine-game --lib crowd_mood` >0 passed with
  `heat_rises_on_a_blow_and_decays_to_the_director_floor` and `stands_lag_one_beat_behind_the_heat`
  (seat pose digest differs between heat 100 and 900). F2P: the stands are furniture. P2P:
  `--lib crowd_mood` and `--lib crowd_stands` green; presentation only, no hash path.
  needs: a donor read before this is picked up (or drop it).

#### Wildcard (wave 2)

- [ ] SW140 render: a hit prints a manga frame for 3 ticks - ink/paper posterise, radial speed lines
  from the impact point, a white burst and a zoom-punch into the screen. Today the only hit-wide
  overlay is white: `crates/game/src/finisher_beat.rs:4` ("full-screen white overlay at 0.9 alpha
  fades over 5 ticks"), and the post chain is six passes with grade only
  (`crates/render/src/post.rs:29` taa -> taau -> bloom -> exposure -> aces -> grade;
  `shaders/post/post_chain.wgsl:12 post_chain_apply` = bloom + exposure + ACES + contrast/sat/lift).
  grep `speed_line|speed lines|posterize` over crates = 0 hits.
  Do: a seventh pass `rp1-impact` after grade; one fullscreen WGSL `impact.wgsl` sampling the graded
  LDR: `zoom_uv = center + dir*(1 - 0.045*hit)`, `hash21` shake at 0.012 UV, `gray` -> 3-step
  posterise -> ink/paper mix, speed lines `abs(sin(angle*72 + hash*24))` gated `smoothstep(0.82,1.0)`
  masked `smoothstep(0.08,0.30,dist)*(1-smoothstep(0.78,1.05,dist))`, block cracks at
  `step(0.91, hash21(floor(angle*18), floor(dist*32)))`, center flash 0.35, then
  `mix(base, ink_layer, hit)`. `hit` is permille from the hit event and decays over 3 ticks;
  intensity knob on `crates/render/src/quality.rs` off at Potato.
  Donor: https://godotshaders.com/shader/screen-sampled-black-and-white-manga-hit-impact-post-process-shader/
  (`hash21`, `fragment`; constants zoom_strength 0.045, shake 0.012, line_density 72, line_cut 0.82,
  posterize_steps 3, contrast 2.6, flash 0.35, vignette 0.55).
  Tier B (3 texture taps + one atan per pixel, potato runs it at 0.5 res). Size M.
  Owns new `crates/render/src/shaders/post/impact.wgsl`, `crates/render/src/post.rs` (pass 7),
  new `crates/game/src/hit_print.rs` + one `mod` line.
  Accept: `cargo test -p engine-render --features window --lib impact` >0 tests; numbers: at hit=1 a
  paper pixel (0.9 gray) reads paper, an ink pixel (0.1) reads ink, 3 posterise bands, and at hit=0 the
  pass is byte-equal to today's graded frame (CPU oracle, 64x36).
  F2P: `impact_pass_is_identity_at_zero_and_three_bands_at_one`.
  P2P: `cargo test -p engine-render --lib post` and `-p engine-game --features app --lib finisher` stay green.
  needs: none (lead 2026-10-24, realistic lane: build it as a realistic impact punch - zoom-punch, a radial blur from the impact point and a short exposure lift, 3 ticks; no posterize, ink or speed lines; off at Potato).
- [ ] SW141 game: the duel has a photo mode - freeze the fight, fly a bounded camera, roll the lens,
  and save a 2x supersampled still with a JSON sidecar of where it was taken. Today there is no
  player-facing camera at all: grep `photo` over crates = 6 hits, all prose about the photo-studio
  HDRI or a photographic texture (`render/src/ibl_arena_env.rs:5`, `render/src/tier.rs:27`), and
  `freecam` = 0 hits; `crates/game/src/camera_rig.rs`
  and `camera_arm.rs` are the fight cameras, and `crates/game/src/capture.rs` only writes frames the
  harness asks for.
  Do: `PhotoMode` state machine (Off -> Entered -> Shooting -> Restored) over the existing `Camera`
  (`crates/game/src/camera.rs:154 fov_y_rad`): paused sim (read the last tick, never step), six-axis
  damped flight inside the arena radius, focal length 24-200 mm mapped to `fov_y_rad`, roll, exposure
  compensation into the existing `post_grade` (`post_chain.wgsl:11`), supersample 2.0 with a
  `max_capture_dim` cap (1920x1080 -> 3840x2160 unless capped), and a JSON sidecar
  (eye, target, fov, grade, tier, tick). On exit restore every knob it touched, byte-equal to the
  pre-enter snapshot (the restore list is the donor's best idea).
  Donor: https://github.com/Calinou/godot-photo-mode-demo/blob/master/addons/photo_mode/photo_mode.gd
  (`take_screenshot`, `apply_high_quality_settings`, `restore_old_quality_settings`,
  `SUPERSAMPLE_FACTOR = 2.0`), plus the bounds/cap rules from
  https://store.godotengine.org/asset/gibberi/lente-photo-mode-for-godot/ (union of boxes/spheres,
  `maximum_capture_dimension`, JSON sidecar per PNG).
  Tier B (potato 1x capture). Size M. Owns new `crates/game/src/photo_mode.rs` + one `mod` line,
  a `Mode::Photo` arm in `crates/game/src/modes.rs`, and a `--photo <path>` line in `crates/game/src/bin/arena_args.rs`.
  Accept: `cargo test -p engine-game --features app --lib photo` >0 tests; numbers: entering at tick
  640 leaves the sim hash at tick 640 after 30 photo frames, a 1280x720 still caps to 2560x1440,
  and every restored field compares equal.
  F2P: `photo_mode_freezes_the_tick_and_restores_every_knob` and `photo_capture_supersamples_and_caps`.
  P2P: `cargo test -p engine-game --features app --lib capture` stays green; golden untouched.
  needs: none. Progress 2026-10-24: step 1 landed 3704cc09f (freeze, bounded fly, 24-200 mm, 2x capped still plan with JSON sidecar; run 40119792795, 5 tests). Left: the capture write and SW145's door verb.

- [ ] SW142 render: the arena gets weather - rain sheets, expanding ripple rings pressed into the
  sand, and a wet sheen on the ground. Today there is no weather path at all: grep `rain|weather` over
  `crates/game/src` returns only audio/bank and arg words (0 weather modules), and the sand shader has
  no wetness term. Existing decals (`crates/render/src/decals.rs` `SplatField`) are stamps, not
  ripples.
  Do: `Weather` enum (Clear | Gust | Rain) with a permille intensity, stepped on ticks not seconds so a
  capture is reproducible; a half-res screen pass `weather.wgsl` that lays a stable world-space grid
  (`world_xz / 0.5 m`), hashes one cell centre per 5x5 neighbourhood, and offsets the screen UV by the
  ripple derivative `((p2-p1)/(2h)) * pow(1-t,2)` (25 taps at half-res, 7 at potato 1/3 res);
  rain streaks as a stretched particle layer reusing `STRETCH_GAIN` (`crates/render/src/particles_gpu.rs:969`);
  wet sand = ground albedo * 0.75 and roughness -0.3 under `intensity`.
  Donor: https://godotshaders.com/shader/rain-drop-floor-effect/ (`hash12`, `hash22`, `fragment`:
  MAX_RADIUS 2, `sin(31*d)` ring, `offset_amount` 0.05, `effect_mix` 0.55) for the ring field;
  https://godotshaders.com/shader/rain-puddles-with-ripples-and-reflection/ for the wetness/roughness
  coupling.
  Tier B (25 half-res taps). Size M. Owns new `crates/render/src/weather.rs`,
  new `crates/render/src/shaders/weather.wgsl`, `crates/render/src/quality.rs`.
  Accept: `cargo test -p engine-render --features window --lib weather` >0 tests; numbers: rain
  intensity 1000 gives 25 ring cells per 5x5 window at 0.5 m spacing, a ripple peak displaces the UV
  by 0.05 at half-res, gauge 0 leaves the frame byte-equal, and the same tick seed gives the same
  displacement twice.
  F2P: `ripple_field_is_stable_in_world_space_and_zero_at_clear`.
  P2P: `cargo test -p engine-render --lib particles_gpu` (35 tests) stays green; golden untouched.
  needs: none (visual weather only; a fight-affecting storm would be a SIM row).

- [ ] SW143 render: the hot sand shimmers and the far wall breathes - depth-masked heat haze, only
  where the air is over hot ground. Today the post chain has no refraction term and grep
  `heat_haze|haze|shimmer` over `crates/render` = 0 hits (`shimmer` elsewhere is only the playtest
  look judge's flicker clip, `crates/playtest/src/look/judge.rs`).
  Do: 5-octave value noise at 8/16/32/64/128 cells, two counter-scrolling sample UVs, `shift` scaled by
  `max(depth - 0.99, 0)` so nothing near the lens wobbles; `max_shift` 2.5 px at 1080p, strength knob
  on `quality.rs` (Potato 0, Mid 0.5, High 1.0). One extra screen tap; skip the pass when strength is 0.
  Donor: https://godotshaders.com/shader/heat-haze-shader/ (arlez80, MIT: `value_noise`, `noise`,
  `random`, `fragment`; `max_shift` 2.5, `speed` 0.03, `min_heathaze_dist` 0.99).
  Tier B (two taps + 10 noise lookups, or a 256x256 noise texture on potato). Size S.
  Owns new `crates/render/src/shaders/haze.wgsl`, `crates/render/src/post.rs` (insert before grade),
  `crates/render/src/quality.rs`.
  Accept: `cargo test -p engine-render --features window --lib haze` >0 tests; numbers: a pixel at
  depth 1.0 shifts under 2.5 px, a pixel at depth 0.5 shifts 0, strength 0 is byte-identical.
  F2P: `haze_never_touches_the_near_field`.
  P2P: `cargo test -p engine-render --lib post` stays green.
  needs: none.

- [ ] SW144 game: a slow-mo beat pushes the camera in while the lens widens - the background smears
  and the two fighters stay the same size. Today the framing rig only dollies to fit bodies
  (`crates/game/src/camera_frame.rs:655 fn dolly`, `DOLLY_DEADBAND 0.02` at :42, `open the lens just
  enough` at :706) and the felt time factor lives in `engine-combat`'s `SlowMo` slots
  (`crates/combat/src/gamefeel.rs:243 add_slow_mo`, `SLOWMO_DEFAULT_PERMILLE 300`) with no camera
  coupling: grep `dolly_zoom|dollyzoom` over crates = 0 hits.
  Do: `DollyZoom` that captures `frustum_h = 2*d*tan(fov/2)` at the beat's first tick and, each tick of
  the beat, sets `fov_y = 2*atan(h/(2*d))` from the current eye-to-target distance, driven by the sim
  time factor (`factor_permille / 1000`) so a 300 permille beat reads 3x stronger than 900; clamp
  0.35 rad <= fov <= 1.40 rad, and hold the fighters' on-screen height within 2%.
  Donor: https://docs.unity3d.com/Manual/DollyZoom.html (`FrustumHeightAtDistance`,
  `FOVForHeightAndDistance`) and https://www.noveltech.dev/unity-dolly-zoom-effect (`DollyZoom.Update`,
  `width/(2*tan(0.5*fov))`).
  Tier B. Size S. Owns `crates/game/src/camera_frame.rs` (one new fn + call in the beat path),
  `crates/game/src/camera_feed.rs` (slow-mo read).
  Accept: `cargo test -p engine-game --features app --lib dolly_zoom` >0 tests; numbers: moving the eye
  from 6 m to 3 m doubles the frustum at the subject and halves fov exactly, on-screen height of a
  1.8 m body stays within 2%, fov stays inside 0.35..1.40.
  F2P: `dolly_zoom_holds_the_subject_height_at_every_step`.
  P2P: `cargo test -p engine-game --features app --lib camera_frame` stays green; golden untouched.
  needs: none (reads the feel clock, writes only the lens).

- [ ] SW145 endpoint: an agent can ask for a still - one `photo.shoot` verb returns a PNG path and the
  judged frame, so a run can prove "it looked like this" without a person. Today the door has
  `scene.read`, `scene.brief`, `scene.place`, `tx.preview`, `tx.apply` (`crates/endpoint/src/assistant.rs:1329`,
  `crates/endpoint/src/audit/mod.rs:661-662`) and grep `"capture` over `crates/endpoint/src` = 0 hits;
  captures only happen from `--capture-dir` on the arena binary.
  Do: a verb that parks the sim at the current tick, takes the SW141 camera pose (or a named preset:
  `hero`, `overhead`, `blade`), renders one supersampled frame through the same post chain, writes png
  + sidecar, and returns `{path, tick, eye, fov, hash}`. Idempotent per (tick, pose): the second call
  with the same inputs returns the same path and hash, so a retry never doubles the file.
  Donor: the idempotent record-and-replay verb shape is ours (`crates/endpoint` scene verbs); the
  capture rules (supersample 2.0, `maximum_capture_dimension`, JSON sidecar per image, refuse in a
  no-render context instead of hanging) come from
  https://store.godotengine.org/asset/gibberi/lente-photo-mode-for-godot/ and
  https://github.com/Calinou/godot-photo-mode-demo/blob/master/addons/photo_mode/photo_mode.gd
  (`take_screenshot` returns early when no frame is rendered).
  Tier B. Size S. Owns `crates/endpoint/src/photo_verb.rs` (new) + one registration line,
  `crates/endpoint/src/audit/mod.rs` verb list.
  Accept: `cargo test -p engine-endpoint --features mcp --lib photo` >0 tests; numbers: two calls at
  tick 300 return one file, one hash pair, and the returned path exists; a headless session returns a
  typed error, not a panic.
  F2P: `photo_shoot_is_idempotent_per_tick_and_pose`.
  P2P: `cargo test -p engine-endpoint --features mcp --lib scene` stays green.
  needs: SW141.

#### Store plugins mapped to open code (wave 2)

- [ ] SW150 render: the locked target wears an outline that shows through sand and bodies - a 2-pass
  mask + edge pass over the same depth, the target hidden behind the berm reading a dim warm edge
  instead of vanishing. Today there is no outline at all: `git grep -Ein "outline|jump_flood|sobel"
  -- crates/render` is 0 hits (the only `outline` in the tree is `crates/game/src/fighter_kit.rs:950`,
  helmet geometry). Draws land in `crates/render/src/present/gpu_present/submit.rs:1118` with the
  post chain beside `crates/render/src/display_grade.rs:45 pbr_neutral`.
  Do: `outline_mask.wgsl` renders the selected fighter ids with `override` white against a depth
  prefill of everything else; `outline_edge.wgsl` Sobel-s the mask at half res and picks
  `visibleEdgeColor` vs `hiddenEdgeColor` from a depth compare (texture matrix = 0.5 bias * proj *
  view); two separable blurs (thickness kernel 4, glow kernel 4) then one additive composite.
  Donor: https://github.com/mrdoob/three.js/blob/dev/examples/jsm/postprocessing/OutlinePass.js
  (`render`, `_getPrepareMaskMaterial`, `_updateTextureMatrix`, `_getEdgeDetectionMaterial`;
  numbers: `downSampleRatio` 2, `edgeStrength` 3.0, `edgeThickness` 1.0, MAX_EDGE_THICKNESS 4,
  MAX_EDGE_GLOW 4, visible edge white, hidden edge 0.1/0.04/0.02, `pulsePeriod` cosine).
  Tier B. Size M. Owns new `crates/render/src/outline.rs`, new
  `crates/render/src/shaders/outline_mask.wgsl`, `outline_edge.wgsl`, one hook in
  `crates/render/src/present/gpu_present/submit.rs`.
  Accept: `cargo test -p engine-render --lib outline` >0 passed; numbers: at 1280x720 the mask runs
  at 640x360, a target 1 m behind `ARENA_WALL_BOX` reads the hidden color, an unoccluded one reads
  the visible color, the edge band is 4 px on the CPU oracle, 0 selected ids costs 0 draws.
  F2P: `outline_edge_picks_the_hidden_color_through_the_wall` and `no_selection_draws_nothing`.
  P2P: `cargo test -p engine-render --lib display_grade` stays green; golden unchanged (view only).
  needs: none. Progress 2026-10-24: Oracle + tests + WGSL landed (612097074, 7ed7cdc25, b9e6a72fb, add33353a, a19e3ac5e); --features v1 outline tests green. Left: the two wgpu pipelines, the submit.rs hook and the --features window on-vs-off pixel test, naga validation.

- [ ] SW151 game: a blow prints its number where it lands - a 27 damage hit floats up from the
  shoulder it hit and fades, so the fight reads without the HUD. Today the value exists and is never
  drawn: `crates/combat/src/reaction.rs:36 damage_number` is a receipt field, and
  `git grep -Ein "world_text|label3d|project_world" -- crates` is 0 hits. The glyph path to reuse
  is in the HUD: `crates/render/src/hud_pass.rs:220 draw`, `:389 text::layout`,
  `:208 text_quads` (one quad per laid-out cell).
  Do: `hit_number.rs` holds a 16-entry ring of `{tick, at_m, amount, side}` filled from the fight_fx
  hit event (`crates/game/src/bin/arena_frame/fight_fx.rs`, FEEL5); each live entry projects its
  anchor with the frame view-proj, lays its digits out with `text::layout`, and emits atlas quads in
  the camera plane at `pixel_size` m per atlas pixel, Y-billboard, rising 0.6 m over 36 ticks with
  `(1 - p)^2` alpha; a `no_depth_test` flag draws it through sand when the body is buried.
  Donor: https://github.com/godotengine/godot/blob/master/scene/3d/label_3d.cpp (`_bind_methods`
  billboard enum Disabled/Enabled/Y-Billboard, `FLAG_FIXED_SIZE`, `FLAG_DISABLE_DEPTH_TEST`,
  `pixel_size` default 0.005 m with range 0.0001..128, `outline_modulate`, `_im_update`/`_shape`
  quad build; the same camera-plane quad rule).
  Tier B. Size S. Owns new `crates/game/src/hit_number.rs` + one `mod` line, one call in
  `crates/game/src/bin/arena_frame/fight_fx.rs`.
  Accept: `cargo test -p engine-game --features app --lib hit_number` >0 passed; numbers: a 27 at
  3 m projects to a 12 px glyph height at 720p, the quad stays vertical when the camera pitches 30
  deg (Y-billboard), alpha is 1 on the hit tick and exactly 0 at tick+36, x2 equal.
  F2P: `hit_number_rises_and_fades_to_zero` and `y_billboard_keeps_the_quad_upright`.
  P2P: `cargo test -p engine-game --features app --lib fight_fx` green; golden unchanged.
  needs: none.

- [ ] SW152 render: the fight gets one film grade instead of three hand-written constants - a 16-cube
  colour LUT plus an 8-bit dither, so dust, torch and blood land in one palette and the potato tier
  stops banding in the sky. Today the display stage is one tone map and a radial vignette:
  `crates/render/src/display_grade.rs:45 pbr_neutral`, `:3` "the radial grade vignette";
  `git grep -Ein "lut|color_correct|dither" -- crates/render` is 0 hits.
  Do: `grade_lut.rs` holds a 16^3 `[[u8;3];4096]` (12,288 bytes, mip-free) with `apply(rgb)` doing
  trilinear lookup on the CPU twin and a `grade_lut.wgsl` sampler3D on device, `dither_8bit(px)`
  adding a 4x4 ordered offset before the sRGB store; neutral LUT must be byte-identical to today.
  Donor: https://github.com/godotengine/godot/blob/master/servers/rendering/renderer_rd/shaders/
  effects/tonemap.glsl (`Params.flags` FLAG_USE_COLOR_CORRECTION / FLAG_USE_8_BIT_DEBANDING,
  `source_color_correction` as sampler3D with a `USE_1D_LUT` sampler2D fallback, `linear_to_srgb`,
  `tonemap_aces`, `tonemap_agx`; the full-screen triangle `uv_interp` vertex).
  Tier B. Size S. Owns new `crates/render/src/grade_lut.rs`, new
  `crates/render/src/shaders/grade_lut.wgsl`, `crates/render/src/display_grade.rs`.
  Accept: `cargo test -p engine-render --lib grade_lut` >0 passed; numbers: the neutral LUT is
  byte-equal to `pbr_neutral` output on 256 samples, a +5% red LUT moves mid grey by 12 permille,
  a 32^3 LUT costs 98,304 bytes and is refused on the potato tier, dither keeps a 1-unit ramp
  monotone.
  F2P: `neutral_lut_keeps_todays_tonemap_byte_equal` and `dither_breaks_a_flat_gradient_band`.
  P2P: `cargo test -p engine-render --lib display_grade` green; golden unchanged.
  needs: SW53 (the 8-bit dither half lands there).

- [ ] SW153 render: the arena's dressing costs one draw instead of thirty - banners, torch brackets,
  urns and benches bake into one vertex/index pair at load, and the potato tier stops paying 30
  `draw_indexed` calls for geometry that never moves. Today every prop is its own item through
  `crates/render/src/draw.rs:872 build`, and `git grep -Ein "static_batch|merge_mesh|batch_mesh"
  -- crates/render` is 0 hits; the only instanced path is the far impostor
  (`crates/render/src/present/impostor_gpu.rs:1107`).
  Do: `static_batch::merge(&[MeshRef], &[[f32;16]])` pre-transforms positions/normals, rebases every
  index by the running vertex count, refuses mixed attribute sets with a named error, and returns one
  `DrawItem`; `merge_vertices` dedupes by a quantised `(pos, normal, uv)` key (tolerance 1e-4) so the
  batch is smaller than the sum.
  Donor: https://github.com/mrdoob/three.js/blob/dev/examples/jsm/utils/BufferGeometryUtils.js
  (`mergeGeometries` attribute-compatibility checks + index offset, `mergeVertices` hash key with a
  tolerance, `mergeAttributes`).
  Tier B. Size S. Owns new `crates/render/src/static_batch.rs`, caller in
  `crates/game/src/arena_kit.rs`.
  Accept: `cargo test -p engine-render --lib static_batch` >0 passed; numbers: 30 props with 24
  vertices each become 1 item with 720 vertices, indices rebased into `0..720`, `merge_vertices`
  drops a duplicated 4-vertex quad to 4 of 8, a normal-less mesh is refused by name, x2 equal.
  F2P: `thirty_props_become_one_draw_item` and `mixed_attributes_are_refused`.
  P2P: `cargo test -p engine-render --lib draw` green; golden unchanged (load-time only).
  needs: none (SW2's indirect args read the same `DrawItem` list afterwards). Progress 2026-10-24: Landed 0b39d1912: the arena dressing bakes into one vertex/index pair at load. Left: the --lib draw P2P and the arena_kit caller per its agent.

- [ ] SW155 camera: the boom stops popping when a pillar clips it - pull in fast (0.2 s), release
  slow (0.4 s), hold the nearest point for 0.25 s, and never act on a 2-tick graze. Today the sweep
  is one shot per frame against two fixed boxes: `crates/game/src/camera.rs:236 BOOM_RADIUS_M 0.3`,
  `:241 ARENA_PILLAR_BOX`, `:246 arena_boom_obstacles() -> [Aabb; 2]`, consumed by
  `crates/game/src/camera_rig.rs:766 eye_m(params, obstacles: &[Aabb])`; `git grep -Ein
  "deocclud|MinimumOcclusion|distance_smooth" -- crates/game` is 0 hits.
  Do: a `BoomFilter { last_len, since_tick, prev_damp }` between the sweep and the eye: an occlusion
  must last `OCCLUSION_HOLD_TICKS` 15 before the eye moves, the pulled distance eases with a 0.2 s
  half-life and the release with 0.4 s, and `min(distance, held_distance)` while inside the hold
  window so a walking fighter cannot make the eye breathe.
  Donor: https://github.com/Unity-Technologies/com.unity.cinemachine/blob/main/com.unity.cinemachine/
  Runtime/Behaviours/CinemachineDeoccluder.cs (`PostPipelineStageCallback` Body stage,
  `VcamExtraState::ApplyDistanceSmoothing`/`UpdateDistanceSmoothing`,
  `ObstacleAvoidance.Default`: CameraRadius 0.4, Damping 0.4, DampingWhenOccluded 0.2,
  MaximumEffort 4, MinimumDistanceFromTarget 0.3).
  Tier B. Size S. Owns `crates/game/src/camera_rig.rs` (boom hunk only), `crates/game/src/camera.rs`.
  Accept: `cargo test -p engine-game --features app --lib boom_filter` >0 passed; numbers: a steady
  occlusion reaches 90% of the pull-in inside 12 ticks and releases 90% inside 24, a 2-tick graze
  with the hold at 15 ticks leaves the eye byte-equal, the held point never moves back through a
  wall, x2 equal.
  F2P: `boom_pulls_in_faster_than_it_releases` and `short_graze_moves_no_eye`.
  P2P: `cargo test -p engine-game --features app --lib camera_rig` green; golden unchanged (camera is
  presentation; `slot.yaw` untouched).
  needs: SW42 (camera stack first; both touch the rig).

#### Physics steals from the physfx and Jolt agents (wave 2)

- [ ] SW161 game: a partly keyframed limb meets a wall without exploding - every physfx body is
  dynamic (`git grep -n -i "kinematic" -- crates/physfx/src` = 0 hits); the tether hand and the
  spear grip are solved as free bodies and pay solver jitter when they meet geometry.
  Do: `crates/physfx/src/fx/drive.rs`, a `Drive::Keyframed { key, speed_m_s }` mode: move the body
  with `set_next_kinematic_translation/rotation` for the tick, add `-gravity * dt` to its linear
  velocity after the step so the solver does not fight gravity, and clamp its speed to `10 m/s`.
  Donor: https://github.com/jrouwe/JoltPhysics/blob/master/Samples/Tests/Rig/SoftKeyframedRigTest.cpp
  (`DriveToPoseUsingKinematics(mPose, dt)` then `AddLinearVelocity(GetGravity() * dt)` to "cancel
  gravity that will be applied in the next step", `bcs.mMaxLinearVelocity = 10.0f`).
  Tier B. Size S. Owns `crates/physfx/src/fx/drive.rs` (new) and the `mod` line of
  `crates/physfx/src/fx.rs`.
  Accept: `cargo test -p engine-physfx --features rapier --lib keyframed` prints 2 passed: a
  keyframed hand holds 2 mm per tick through a wall pass, and a dynamic neighbour is not launched
  (its speed stays under 3 m/s).
  F2P: `a_keyframed_hand_holds_its_pose_and_meets_a_wall_without_a_spike`. P2P: full physfx lib
  green; golden unchanged.
  needs: none.

- [x] SW162 bodies: the ragdoll's inertia is floored like its mass - `stabilize_masses`
  (`crates/physics/src/ragdoll.rs:1302-1360`, `MASS_RATIO_LO = 0.8`, `MASS_RATIO_HI = 1.2`) clamps
  child/parent mass and rescales the total, but nothing raises a parent's inertia:
  `git grep -i "inertia|principal" -- crates/physfx/src/rig.rs crates/physfx/src/fx.rs` = 0 hits.
  That is the shape of the red PX42 set (`every_named_joint_holds_its_human_range_under_a_shove`,
  the ankle reaching -50 degrees against its 40).
  Do: a second pass `stabilize_inertia` beside `stabilize_masses`: decompose each bone's inertia into
  principal moments, sum the children's per joint walking the leaves first, set the parent's moment
  to `max(own, min(2 * own, children_sum))`, rebuild the tensor; physfx applies the result through
  `ColliderBuilder` mass properties.
  Donor: https://github.com/jrouwe/JoltPhysics/blob/master/Jolt/Physics/Ragdoll/Ragdoll.cpp
  (`RagdollSettings::Stabilize`: `cMinMassRatio = 0.8f`, `cMaxMassRatio = 1.2f`,
  `cMaxInertiaIncrease = 2.0f`, `minimum = min(cMaxInertiaIncrease * principal.mDiagonal[0],
  principal.mChildSum)`; after Havok's "Stop my Constraints from Blowing Up!", Oliver Strunk).
  Tier B. Size S. Owns `crates/physics/src/ragdoll.rs` (the `stabilize_masses` neighbourhood only)
  plus `crates/physfx/src/rig.rs` if physfx takes it.
  Accept: `cargo test -p engine-physics --features rapier --lib stabilize` prints 1 new test: a hand
  whose inertia is 1/40 of the forearm comes back inside the bound, total mass unchanged, bytes x2.
  F2P: `stabilize_raises_a_heavy_hand_over_a_light_arm`. P2P: `cargo test -p engine-physics
  --features rapier --lib balance_hash` green; golden unchanged (mass ratios already landed and are
  unchanged by this half).
  needs: none. Progress 2026-10-24: Landed 9f452e7e5: ragdoll inertia floored like its mass; 2 stabilize tests run 40126101501, balance_hash run 40126385693 green. Left: golden re-run (lead batch). Landed 9f452e7e5: inertia floored like mass; 2 stabilize tests run 40126101501, balance_hash run 40126385693; golden d57b5c8ae3096614 unchanged at a19e3ac5e (run 40127182965, 7 golden tests).

- [ ] SW163 render: a stiff ground contact and a soft body-to-body contact in one build - physfx
  sets only the solver budget (`crates/physfx/src/fx.rs:163`
  `num_solver_iterations`/`max_ccd_substeps`) and builds every collider with `.restitution(0.0)`
  (`:217`, `:599`); there is no per-contact softness anywhere (`git grep -i
  "contact_skin|normalized_allowed" -- crates` = 0 hits).
  Do: `crates/physfx/src/caps.rs` gains `ground_softness_hz` and `body_softness_hz` per tier;
  `fx.rs` builds the floor and walls with the stiff value and bones/debris with the soft one, and
  skips the restitution stage when every material reads zero bounce (true today).
  Donor: https://github.com/erincatto/box2d/blob/main/src/contact_solver.c
  (`b2PrepareContacts_Overflow`: `constraint->softness = staticSoftness` when a side has no solver
  body, `contactSoftness` otherwise, "stiffer for static contacts to avoid bodies getting pushed
  through the ground"; `anyRestitution` skips the stage) and
  https://github.com/erincatto/box2d/blob/main/src/solver.c (the stage list).
  Tier B. Size S. Owns `crates/physfx/src/caps.rs`, `crates/physfx/src/fx.rs` (the collider build).
  Accept: `cargo test -p engine-physfx --features rapier --lib softness` prints 2 passed: a 30-body
  pile at tick 60 keeps every body inside the arena bound with penetration under 5 mm, and the same
  pile gives the same bytes twice.
  F2P: `the_ground_holds_a_pile_without_sinking_and_a_limb_pair_stays_soft`. P2P: full physfx lib
  green; golden unchanged.
  needs: none. Progress 2026-10-24: Rescope: per zeHZplC8 the row assumes ground_softness_hz/body_softness_hz per tier and per-contact softness the physics crate does not expose; needs a rewrite against powered.rs before work.

- [ ] SW164 bodies: solve the joint chain from the root out - rapier gets every joint in the order
  the builder inserted it (`crates/physics/src/ragdoll.rs::insert_joints`), so a root bone and a hand
  bone are solved as equals and a long chain stretches under a shove; there is no priority concept
  in physfx either (`git grep -in "priority" -- crates/physics/src/ragdoll.rs
  crates/physics/src/powered.rs crates/physfx/src` finds only prose at
  `crates/physfx/src/types.rs:105`).
  Do: give each joint a build-time priority that rises toward the root (child 0, parent +1) and
  insert/increase stiffness in that order so the root wins the first iterations; prove it with a
  chain-stretch metric rather than a pose pin.
  Donor: https://github.com/jrouwe/JoltPhysics/blob/master/Jolt/Physics/Ragdoll/Ragdoll.cpp
  (`RagdollSettings::CalculateConstraintPriorities(uint32 inBasePriority)`: "Start with the base
  priority and increment towards the root", written onto `mConstraintPriority`, reaching rapier as
  insertion order plus a stiffness weight).
  Tier B. Size S. Owns the joint-insertion hunk of `crates/physics/src/ragdoll.rs`.
  Accept: `cargo test -p engine-physics --features rapier --lib joint_priority` prints 2 passed: a
  4-bone chain hanging by its root stretches less than half as far at the tip, and the same chain
  twice is byte-equal.
  F2P: `a_root_first_chain_stretches_less_at_the_tip`. P2P: `cargo test -p engine-physics --features
  rapier --lib snapshot` green; golden unchanged.
  needs: none (client-local ragdoll only; `Ragdoll::state_hash` stays out of the fold per the
  physics-bodies skill). Progress 2026-10-24: Rescope: per zeHZplC8 the row assumes insertion order plus a global num_solver_iterations harness that crates/physics does not have (pipelines live in powered.rs); rewrite before work.

#### Pose drive wiring (wave 2)

- [ ] SW170 physics: the pose drive reaches the live look loop - `PhysFx::drive_pose(slot, prev,
  now, dt, weight)` (`crates/physfx/src/fx/follow.rs:54`, landed `b0c3ad9a9`, 4 tests, run
  40117279321) drives the joints at the clip pose with the clip's own angular velocity, capped and
  muscle-scaled, and writes nothing at weight 0. It has no caller today (`git grep -n drive_pose --
  crates/game` = 0 hits), so a shoved standing reaction still shows position-only bracing.
  Do: in `crates/game/src/ragdoll_look/hit.rs`, where `ragdoll_reactions` steps the reaction world
  (`hit.rs:203 fx.step()`), keep each reacting slot's last shown per-bone pose as one
  `[Transform; rig.bones()]` slice (the look already reads the same bones through `placed(fx,
  Key::Bone { slot, bone }, 1.0)`), build the current slice the same way, and call
  `fx.drive_pose(slot_u8, &prev_bones, &now_bones, FIXED_DT, weight)` right after that `step()`,
  with weight 1 while the reaction is in its driven window and 0.0 once it hands the body back
  (weight 0 is a no-op, so the fade stays exactly today's bytes). Same call site list for
  `crates/game/src/ragdoll_look.rs:922 fx.step()`, which is another AI's dirty file today: take
  that hunk only when it is free, and the `hit.rs` hunk lands the row on its own.
  Donor: JoltPhysics `Jolt/Physics/Ragdoll/Ragdoll.cpp::DriveToPoseUsingMotors(prev_pose, pose,
  dt)` - the motors take the pose and the pose delta's angular velocity (already ported into
  `follow.rs`).
  Tier B. Size S. Owns `crates/game/src/ragdoll_look/hit.rs`; needs the `ragdoll_look.rs` hunk.
  Accept: `cargo test -p engine-game --features app --lib follow` >0 tests; numbers: one tick after
  a chest shove on a driven arm, the lower arm's world error against the clip pose is smaller with
  the drive than with the same reaction and no drive, and a weight-0 run leaves the shown
  transforms byte-identical to today's.
  F2P: `look_follow_tracks_the_clip_after_a_shove` and `look_follow_weight_zero_keeps_the_shown_bytes`.
  P2P: `cargo test -p engine-game --features app --lib ragdoll_react` stays green; golden untouched.
  needs: none.
