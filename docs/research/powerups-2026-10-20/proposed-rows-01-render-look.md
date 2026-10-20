# Proposed rows 01 - render look: rows, order and patch-ready specs (2026-10-20)

Companion to `01-render-look-steals.md` (same folder). This file proposes rows for the lead to merge
into the board (I did not touch `docs/SPRINT*.md`, `docs/FOCUS.md` or `docs/plan/finish-line.md`).
`crates/render` is HELD by the GS44 auto-exposure landing agent, so **no code lands now**: every row
below carries a patch-ready spec (exact file path inside `crates/render`, exact function or WGSL entry
point, exact test names, exact proof command, golden value that must not move).

Licences and donor files were read live on 2026-10-20 (GitHub contents API). PORT TYPE is named in
every row: **algorithm-shaping only** means we read the method and write our own WGSL or Rust, we
vendor no donor bytes; **Rust shape** means the donor is Rust and its structure can be reimplemented
closely (still our own code, no bytes copied).

---

## (b) The order of gap B7 (the lead schedules this)

Gap B7 of `docs/plan/finish-line.md` item 7: "a depth and normal prepass, GTAO on ambient only, the
post chain (bloom, grade, TAA) as a real pass, particles and decals drawn in the scene, and cull, sort
and instancing in the live draw loop."

**Cut 1 of B7: LF5 (GPU motion vectors).** Every other temporal thing in the frame reads the same
velocity target (RL38 TAA, later motion blur and the RT denoiser), LF5 is Tier A, and its need
(LF3) is already ticked, so one target unlocks the most rows.
**Cut 2 of B7: RL60 + RL32 (transform on DrawItem, one draw per bin).** RL41 and RL35 both list
"needs: RL60" on the board, and the live loop still draws one item at a time (verified: `submit.rs`
encodes per item), so this is the gate for the next two cuts.
**Cut 3 of B7: RL41 (depth plus normal prepass).** The depth half already landed in LF3 landing 2
(`present/gpu_present/gtao_live.rs:523` calls `encode_depth_prepass`); this cut adds the normal target
the frame lacks and takes the AO off the sun.
**Cut 4 of B7: RL38 (TAA v2 as a real pass).** It needs cut 1 (the velocity target) to be live, and
B7 names the post chain with TAA in it; bloom and grade are already ticked (LF4).
**Cut 5 of B7: GS18b (draws go indirect).** It rides on cut 2 and carries the DX12 `first_instance`
risk, so it lands last and only if the CPU bin path misses its number.

Side row that the look wants next, not named in B7: RL35 + QA4 (the sun shadow the frame samples).
RL35 needs RL60 (cut 2), so it follows cut 2 and can run in parallel with cuts 3 and 4.

---

## (c) RL59 and the dead `debug_draw.rs`

`crates/render/src/debug_draw.rs` (284 lines, read today) is a std-only contract: `DebugDraw` trait
(line 45), `DebugRecorder` (line 52), `line_verts` (line 143). Its own header says: "the Rapier
backend and the arena binary call `DebugDraw::collect` (later slices)". `git grep DebugDraw crates/`
today hits only `debug_draw.rs` and `lib.rs:136` - there is no caller, so the file is dead weight
exactly as `docs/plan/finish-line.md` item 13 says.

**RL59 call site (exact):** a new file `crates/render/src/present/gpu_present/debug_view.rs` with
`fn encode_debug_view(enc: &mut wgpu::CommandEncoder, targets: &FrameTargets, view: DebugView)`;
called from `present_frame_live` in `crates/render/src/present/gpu_present/live.rs` right after
`self.encode_blit(...)` (live.rs:1314), before the present copy. The same pass consumes
`debug_draw::line_verts(&recorder.frames(), &mut verts)` and draws them as a line list, so the dead
`DebugDraw` trait gets its first caller inside `crates/render` (no game-crate change needed).
RL59 also needs PF25 (per-pass GPU ms), so PF25 comes first in the pair.

**Rest of finish-line item 13: still unowned.** Verified today by grep: no cvars anywhere in
`crates/` (0 hits), no Perfetto or `tracing` dependency in any `Cargo.toml` (0 hits), no in-game
console crate. Pause and one-tick step exist but live in `crates/game/src/bin/arena.rs`
(`--pause-at-tick`, `paused tick=`) and the time scale exists in `crates/game/src/virt.rs`
(`SPEED_STEPS`, permille ramps), both in the game lane, not in `crates/render`. So item 13 splits:
cvars, the console and the Perfetto trace have **no row and no owner**; pause, step and time scale
exist in the game lane and only need the render-side trace scopes.

---

## Rows

### RXX-01 - LF5: GPU motion vectors on the forward pass

**What it is.** A velocity target written by the forward pass next to the depth target: current clip
position minus previous clip position per pixel, camera-only for the static kit and palette-based for
the skinned fighters. The tree has no such target today: `crates/render/src/post/gpu.rs` takes a CPU
`MotionField` and the frame has no velocity attachment (OBSERVED 2026-10-20).

**Donor.** NVIDIAGameWorks/Falcor, https://github.com/NVIDIAGameWorks/Falcor, file
`Source/RenderPasses/GBuffer/VBuffer/VBufferRT.slang` and `Source/RenderPasses/GBuffer/VBuffer/VBufferRaster.slang`,
licence BSD-3-Clause (LICENSE.md read live 2026-10-20), date read 2026-10-20.

**PORT TYPE: algorithm-shaping only.** Falcor is Slang and DX12/Vulkan; our renderer is WGSL. We port
the method (one velocity target, current minus previous clip, camera-only for static geometry) and
write our own WGSL. No donor bytes.

**Closes.** LF5 (Tier A, needs LF3 - ticked) and, downstream, RL37 -> RL38. Part of gap B7 (cut 1).

**Owned paths.** `crates/render/src/shaders/forward.wgsl`, `crates/render/src/present/gpu_present/submit.rs`,
`crates/render/src/present/gpu_present/live.rs`, new `crates/render/src/shaders/velocity.wgsl`,
`crates/render/src/present.rs` (previous transform on DrawItem).

**Accept.** `cargo test -p engine-render --lib motion_vector` prints one line
`motion_vector_pan_shift_within_0_1px_and_static_is_zero` PASS: the static camera fixture gives an
all-zero target, the known 10 degree pan gives the analytic shift within 0.1 px, a moved body carries
its move, validation errors 0.

**Kill line.** Forward pass grows by more than 0.3 ms at 1080p on the RTX 3050, or the pan shift misses
0.1 px after two tuning rounds -> the velocity target dies and TAA stays off.

**Cost.** M.

**Spec (patch-ready).**
- New file `crates/render/src/shaders/velocity.wgsl`: `@vertex fn vs_velocity(in: VIn) -> @builtin(position) vec4<f32>`
  (clip position of the current frame) and `@vertex fn vs_velocity_skinned(in: SIn) -> ...`, plus
  `@fragment fn fs_velocity(...) -> @location(0) vec2<f32>` writing `(clip - prev_clip) * vec2(0.5, -0.5)`.
  Previous clip comes from a uniform `PrevClipU { view_proj_prev: mat4x4<f32> }` in group 0 binding 20
  (bindings 0..19 are taken in `forward.wgsl` today).
- Changed file `crates/render/src/shaders/forward.wgsl`: extend `FsOut` (line 355) with
  `@location(2) velocity: vec2<f32>` and write it in `fs` (line 703); `vs` (line 376) and `vs_skinned`
  (line 389) carry the previous clip for the skinned path.
- Changed file `crates/render/src/present/gpu_present/submit.rs`: in `ensure_targets` (line 456) add
  the velocity texture `Rg16Float` with `RENDER_ATTACHMENT | TEXTURE_BINDING`; in `encode_forward`
  (line 973) add it as colour attachment 2 of the forward pipeline, and add the `PrevClipU` bind
  group entry; `forward_color_target_count()` (used in `ensure_targets`) goes from 2 to 3.
- Changed file `crates/render/src/present.rs`: `DrawItem` (line 410) gets `prev_model: [[f32; 4]; 4]`
  (identity when there is no previous frame) and `BinKey`/`group_into_bins` (lines 550, 601) keep the
  existing order rules.
- Changed file `crates/render/src/present/gpu_present/live.rs`: `present_frame_live` (line 1150) keeps
  the last frame's view-projection and the last frame's instance records, and passes them into the
  forward bind.
- Tests (new module in `crates/render/src/present/gpu_present/submit.rs` or a new
  `crates/render/src/present/gpu_present/velocity.rs`): `motion_vector_static_camera_is_all_zero`,
  `motion_vector_camera_pan_matches_analytic_shift`, `motion_vector_moved_body_carries_its_move`,
  `motion_vector_target_is_rg16_float_and_bindable`.
- Proof command: `node tools/gh-check.mjs cargo test -p engine-render --lib motion_vector`.
- Golden: `node tools/proof.mjs` must still print `golden d57b5c8ae3096614 OK` (twice).

### RXX-02 - RL60 + RL32 + GS18b: one draw per bin, GPU cull, indirect draws

**What it is.** Give `DrawItem` a model matrix, sort opaque draws into bins by one u64 key, draw each
bin once with an instance-step buffer, and later let a compute cull write the indirect args. Today the
live loop draws one item at a time and `DrawItem` (present.rs:410) has no transform (OBSERVED).

**Donor.** schell/renderling, https://github.com/schell/renderling, files
`crates/renderling/src/draw.rs` (`DrawIndirectArgs {vertex_count, instance_count, first_vertex, first_instance}`)
and `crates/renderling/src/cull/shader.rs` (cull writes `instance_count = 0` on the indirect arg),
licence MIT OR Apache-2.0 (LICENSE read live 2026-10-20), date read 2026-10-20. Shape only from
`crates/render/src/draw.rs` in our tree: `cull_cpu_flags_into` (246), `visible_count` (264),
`indirect_args_5` (278), `shaders/cull.wgsl`.

**PORT TYPE: Rust shape.** renderling is Rust on wgpu, so the struct layout, the indirect-args buffer
and the cull-then-draw structure can be reimplemented closely, still as our own Rust and our own WGSL
(the donor's compute is rust-gpu, so the shader is written again in WGSL).

**Closes.** RL60 (transform, per-bin draw), RL32 (sort and group), GS18b (draws go indirect). Part of
gap B7 (cuts 2 and 5).

**Owned paths.** `crates/render/src/present.rs`, `crates/render/src/draw.rs`, `crates/render/src/gpu_cull.rs`,
`crates/render/src/shaders/cull.wgsl`, `crates/render/src/present/gpu_present/submit.rs`,
`crates/render/src/present/gpu_present/forward_bind.rs`.

**Accept.** `cargo test -p engine-render --lib bins` prints `bins_256_copies_one_draw_and_order_stable` PASS:
256 copies of one kit mesh make 1 `draw_indexed` with 256 instances (today 256 draws), the order is
stable under input permutation, and the capture bytes are equal before and after. For GS18b:
`cargo test -p engine-render --lib gpu_cull` prints `gpu_cull_matches_cpu_oracle_on_10k_spheres` PASS
and the same frame with `E2040_CPU_CULL=1` is byte-equal.

**Kill line.** The GPU cull answer misses the CPU oracle on 10,000 spheres, or DX12 drops
`first_instance` on the count draw (wgpu-hal 30.0.1 dx12/command.rs, known in our docs) and the
per-instance data cannot ride the instance-step buffer -> GS18b dies and the CPU bin path stays.

**Cost.** M (RL60 + RL32), L (GS18b).

**Spec (patch-ready).**
- Changed file `crates/render/src/present.rs`: `DrawItem` (410) gets `model: [[f32; 4]; 4]` and
  `prev_model` (RXX-01); `BinKey::build` (552) keeps its four parts; `group_into_bins` (601) keeps
  first-seen key order; a new `pub fn instance_records(items: &[(BinKey, InstanceRecord)]) -> Vec<InstanceRecord>`.
- Changed file `crates/render/src/draw.rs`: `MAX_DRAWS` (24) and `MAX_INSTANCES_PER_GROUP` (32) stay;
  new `pub fn bin_draw_calls(bins: &[(BinKey, Vec<usize>)], items: &[(BinKey, InstanceRecord)]) -> Vec<BinDraw>`;
  `indirect_args_5` (278) keeps its 5-word form.
- Changed file `crates/render/src/shaders/cull.wgsl`: keep the pinned CPU rule in `draw.rs`; add the
  WGSL cull that writes `instance_count = 0` for a culled primitive and `1` for a visible one, matching
  `gpu_cull.rs` `CULL_WORKGROUP` (64) and `INDIRECT_WORDS` (5).
- Changed file `crates/render/src/gpu_cull.rs`: `CULL_WGSL` (the `include_str!` of `shaders/cull.wgsl`)
  stays the one shader both halves use; add `pub fn submit_indirect_draws(...)` that calls
  `draw_indexed_indirect` and is gated by the adapter feature check (no count draws on DX12).
- Changed file `crates/render/src/present/gpu_present/submit.rs`: `encode_forward` (973) loops over
  bins instead of items; `forward_bind.rs` gains the instance-step vertex buffer layout
  (`VertexStepMode::Instance`).
- Tests: `bins_256_copies_one_draw_and_order_stable`, `bins_order_survives_input_permutation`,
  `bins_capture_bytes_equal_before_and_after`, `gpu_cull_matches_cpu_oracle_on_10k_spheres`,
  `gpu_cull_indirect_args_instance_count_zero_when_culled`.
- Proof command: `node tools/gh-check.mjs cargo test -p engine-render --lib bins` and
  `node tools/gh-check.mjs cargo test -p engine-render --lib gpu_cull`.
- Golden: `node tools/proof.mjs` must still print `golden d57b5c8ae3096614 OK` (twice).

### RXX-03 - RL41: depth and normal prepass, AO on ambient only

**What it is.** One prepass that writes depth and a world normal target, then GTAO reads the real
normal instead of the depth-difference normal and multiplies the ambient term only. The depth half
already landed (LF3 landing 2); the normal target does not exist
(`crates/render/src/gtao.rs:680 live_facing_normal` derives it from depth).

**Donor.** NVIDIAGameWorks/Falcor, https://github.com/NVIDIAGameWorks/Falcor, files
`Source/RenderPasses/GBuffer/GBuffer/DepthPass.3d.slang` and
`Source/RenderPasses/GBuffer/GBuffer/GBufferRT.slang` (target layout), licence BSD-3-Clause (read live
2026-10-20), date read 2026-10-20.

**PORT TYPE: algorithm-shaping only.** Falcor is Slang; we write our own WGSL prepass and our own
normal target. No donor bytes.

**Closes.** RL41 (needs LF3 - ticked, and RL60 - cut 2) and the "depth and normal prepass" half of gap
B7 (cut 3).

**Owned paths.** `crates/render/src/present/gpu_present/submit.rs`, `crates/render/src/present/gpu_present/gtao_live.rs`,
`crates/render/src/gtao.rs`, `crates/render/src/shaders/forward.wgsl`, new
`crates/render/src/shaders/prepass.wgsl`.

**Accept.** `cargo test -p engine-render --lib gtao` prints `gtao_normal_prepass_ao_scales_ambient_only` PASS:
the forward pass reads the prepass normal, the sun term is byte-equal with AO on and off on a sunlit
sand fixture, and the AO field still moves on the covered pixels.

**Kill line.** Forward plus prepass is not at least 10 percent below the old forward p50 at 1080p, or
the AO field changes under the sun -> the normal target dies and GTAO keeps the depth-difference normal.

**Cost.** M.

**Spec (patch-ready).**
- New file `crates/render/src/shaders/prepass.wgsl`: `@vertex fn vs_prepass(in: VIn) -> PrepassOut`
  (clip position plus world normal), `@vertex fn vs_prepass_skinned(in: SIn) -> PrepassOut`,
  `@fragment fn fs_prepass(in: PrepassOut) -> @location(0) vec4<f32>` writing the packed world normal.
- Changed file `crates/render/src/present/gpu_present/submit.rs`: `encode_depth_prepass` (924) becomes
  `encode_depth_normal_prepass` and writes the normal target as well; `ensure_targets` (456) adds the
  normal texture (`Rgba16Float` or `Rg16Float` with a sign convention, chosen in the target layout);
  the forward pass then clears the depth and writes it again, so the frame's final depth is unchanged.
- Changed file `crates/render/src/present/gpu_present/gtao_live.rs`: the GTAO submitter (the call at
  line 523) binds the normal target instead of computing `live_facing_normal`.
- Changed file `crates/render/src/gtao.rs`: `gtao_half_res` (half-res oracle) and
  `upsample_bilateral` keep their signatures; a new `pub fn gtao_half_res_with_normals(depth_full, normals_full, config)`
  and the CPU mirror `live_facing_normal` (680) become the fallback only.
- Changed file `crates/render/src/shaders/forward.wgsl`: `fs` (703) keeps the AO slot (group 1
  binding 0) and multiplies the ambient term only; the blit never multiplies AO over the sun.
- Tests: `gtao_normal_prepass_ao_scales_ambient_only`, `gtao_prepass_normal_matches_cpu_oracle`,
  `gtao_sun_term_is_byte_equal_with_ao_on_and_off`.
- Proof command: `node tools/gh-check.mjs cargo test -p engine-render --lib gtao`.
- Golden: `node tools/proof.mjs` must still print `golden d57b5c8ae3096614 OK` (twice).

### RXX-04 - RL38: TAA v2 as a real pass (velocity reprojection, clamp, Catmull-Rom)

**What it is.** One TAA resolve that reprojects the history with the velocity target from RXX-01,
clamps the history to the neighbourhood, samples the history with Catmull-Rom and applies an
anti-flicker term. Three TAA copies exist today (post.rs, `shaders/post/taa_compute.wgsl`,
`shaders/post/taa_resolve.wgsl`) and the live frame never jitters.

**Donor.** NVIDIAGameWorks/Falcor, https://github.com/NVIDIAGameWorks/Falcor, file
`Source/RenderPasses/TAA/TAA.ps.slang` (5.8 KB, read live: it reads `gTexMotionVec`, does a
Catmull-Rom history sample, and carries `gColorBoxSigma` and `gAntiFlicker`), licence BSD-3-Clause
(read live 2026-10-20), date read 2026-10-20. Second hand already named in the tree: pmndrs/upscaler
and FSR3 for the upsample half, so this row ports only the resolve half.

**PORT TYPE: algorithm-shaping only.** Slang in the donor, WGSL for us. We write our own WGSL.

**Closes.** RL38 (needs RL37, which needs the RXX-01 velocity target) and the "post chain (bloom,
grade, TAA) as a real pass" half of gap B7 (cut 4).

**Owned paths.** `crates/render/src/post/taa.rs`, `crates/render/src/post/gpu.rs`,
`crates/render/src/shaders/post/taa_compute.wgsl`, `crates/render/src/shaders/post/taa_resolve.wgsl`,
`crates/render/src/post/chain.rs`, `crates/render/src/present/aa.rs`.

**Accept.** `cargo test -p engine-render --lib gpu_taa_sword_sweep` prints
`gpu_taa_sword_sweep_ghost_0_08_and_three_copies_become_one` PASS: ghost trail at most 0.10 on a
120 px per frame blade sweep (today 0.25), shimmer variance at most 0.01, and the TAA copies count
drops from 3 to 1.

**Kill line.** Ghost trail stays above 0.10 after two tuning rounds -> keep the 3x3 box clamp, drop the
Catmull-Rom and the anti-flicker, and the row closes as the clamp-only TAA.

**Cost.** M.

**Spec (patch-ready).**
- Changed file `crates/render/src/post/taa.rs`: `TaaParams` (9) gains `catmull_rom: bool`,
  `anti_flicker: bool` and `history_sigma: f32`; `taa_resolve` (145) and `taa_resolve_jittered` (201)
  keep their signatures and gain the new terms; `trail_luma` (326) stays the ghost metric.
- Changed file `crates/render/src/shaders/post/taa_compute.wgsl`: the compute entry becomes the one
  resolve (velocity texture in, history texture in, clamped and Catmull-Rom sampled history out);
  `crates/render/src/shaders/post/taa_resolve.wgsl` is deleted and its callers moved
  (`post/gpu.rs` and `post/chain.rs`).
- Changed file `crates/render/src/post/gpu.rs`: `pack_motion` (235) is replaced by the GPU velocity
  texture bind; `MotionField` stays for the CPU oracle.
- Changed file `crates/render/src/present/aa.rs`: `select_aa` picks TAA when the velocity target is
  live and the scale is 1.0, SMAA otherwise.
- Tests: `gpu_taa_sword_sweep_ghost_0_08_and_three_copies_become_one`,
  `taa_history_rejected_on_camera_cut_uses_history_valid_flag`,
  `taa_catmull_rom_history_is_stable_under_input_permutation`.
- Proof command: `node tools/gh-check.mjs cargo test -p engine-render --lib taa`.
- Golden: `node tools/proof.mjs` must still print `golden d57b5c8ae3096614 OK` (twice).

### RXX-05 - RL35 + QA4: the sun shadow the frame samples (static arena map, dynamic fighter map)

**What it is.** One world-fixed arena shadow map (2048 over the pit) that is redrawn only when the sun
moves, plus a tight fighter map, sampled with a PCF kernel, and the arena sun split into 2 cascades
with the practical split at lambda 0.5. The sun depth map exists and is sampled by a 4-tap compare
(`forward.wgsl:185 sun_shadow_pcf`), but the arena map is redrawn every frame over a 256 map.

**Donor.** TheRealMJP/Shadows, https://github.com/TheRealMJP/Shadows, files `Shadows/PCFKernels.hlsl`,
`Shadows/DrawCascades.hlsl`, `Shadows/DepthReduction.hlsl`, licence MIT (LICENSE read live
2026-10-20), date read 2026-10-20. Second hand, atlas layout: sirhco/verve,
https://github.com/sirhco/verve, file `src/client/islands/GlSceneWebgpu.zig`, licence Apache-2.0
(LICENSE read live 2026-10-20), date read 2026-10-20.

**PORT TYPE: algorithm-shaping only.** MJP is HLSL and DX12; verve is Zig with a WebGPU backend. We
port the methods (PCF kernel shape, cascade split table, atlas tile layout) and write our own WGSL and
our own Rust. No donor bytes.

**Closes.** RL35 (needs EG11 - ticked, and RL60 - cut 2) and QA4 (2 cascades with the practical split).
Side row for the look, not named in gap B7.

**Owned paths.** `crates/render/src/shadow_cascade.rs`, `crates/render/src/present/gpu_present/sun_shadow.rs`,
`crates/render/src/shaders/forward.wgsl`, `crates/render/src/twin_shadow.rs`.

**Accept.** `cargo test -p engine-render --lib sun_shadow_live_stable_and_sampled` prints
`sun_shadow_live_stable_and_sampled_texel_0_4cm_fighters` PASS: texel at most 0.5 cm at the fighters
and at most 5 cm on sand, the edge moves less than one texel for a 10 degree pan, and the static map
re-renders at most once per 600 frames. `cargo test -p engine-render --lib shadow_cascade` prints the
split table for the arena camera (QA4).

**Kill line.** Shadow passes above 1.0 ms in `arena per-pass`, or the static map re-renders more than
once per 600 frames, or the edge moves more than one texel for a 10 degree pan -> the row dies and the
single 256 map stays.

**Cost.** M.

**Spec (patch-ready).**
- Changed file `crates/render/src/shadow_cascade.rs`: `split_distances` (484) and `checked_splits` (500)
  gain a practical split at lambda 0.5 for 2 cascades; `cascade_ranges` (547) and `CASCADE_MAP_SIZES`
  (25) keep their shape; `snap_shadow_origin` (33) and `shadow_pcf` (106) stay the CPU oracle.
- Changed file `crates/render/src/present/gpu_present/sun_shadow.rs`: `SUN_SHADOW_SIZE` (22, 2048) and
  `SUN_SHADOW_HALF_EXTENT_M` (31, 15.0) stay; add `pub fn arena_map_dirty(sun_dir: [f32; 3]) -> bool`
  (the sun moved more than 0.25 degrees) and `pub fn fighter_map_view_proj(bounds_m: ([f32; 3], f32)) -> [[f32; 4]; 4]`;
  `encode_sun_depth` (submit.rs:447) draws the static map only when dirty and the fighter map every frame.
- Changed file `crates/render/src/shaders/forward.wgsl`: `sun_shadow_pcf` (185) reads the arena map and
  the fighter map and blends them; the distance dimmer goes.
- Tests: `sun_shadow_live_stable_and_sampled_texel_0_4cm_fighters`,
  `sun_shadow_static_map_renders_once_per_600_frames`, `shadow_cascade_prints_split_table_two_cascades`.
- Proof command: `node tools/gh-check.mjs cargo test -p engine-render --lib sun_shadow` and
  `node tools/gh-check.mjs cargo test -p engine-render --lib shadow_cascade`.
- Golden: `node tools/proof.mjs` must still print `golden d57b5c8ae3096614 OK` (twice).

### RXX-06 - RL59 + RL58: the debug views and the first caller of the dead `debug_draw.rs`

**What it is.** One pass that draws the selected debug view into the capture (8 views: overdraw,
wireframe, normals, shadow atlas, cascades, AO, motion, luminance) and, in the same pass, draws the
lines that `debug_draw::line_verts` produces. That line draw is the first caller of the dead
`DebugDraw` contract.

**Donor.** schell/renderling, https://github.com/schell/renderling, files
`crates/renderling/src/debug.rs` (the shader module) and `crates/renderling/src/debug/cpu.rs` (the
wgpu pipeline: bind group layout, render pipeline, a full-screen overlay that reads the slab and the
draw args), licence MIT OR Apache-2.0 (read live 2026-10-20), date read 2026-10-20.

**PORT TYPE: Rust shape.** The donor is Rust on wgpu, so the pipeline, bind group and overlay
structure are reimplemented closely, still our own code.

**Closes.** RL59 (needs PF25) and gives RL58 (`debug_draw.rs`) its first caller. Side row, not gap B7.

**Owned paths.** new `crates/render/src/present/gpu_present/debug_view.rs`, new
`crates/render/src/shaders/debug_view.wgsl`, `crates/render/src/debug_draw.rs`,
`crates/render/src/present/gpu_present/live.rs`.

**Accept.** `cargo test -p engine-render --lib debug_view` x2 prints
`debug_view_8_views_8_different_sha_and_off_is_byte_equal` PASS: the 8 views give 8 non-black PNGs
with 8 different SHA-256, no view selected costs under 0.1 ms, and an unknown view name is refused.
`cargo test -p engine-render --lib debug_draw` prints `debug_draw_has_a_live_caller` PASS.

**Kill line.** A debug view changes the capture bytes when it is off, or the overlay costs more than
0.1 ms at 1080p -> the row dies and `debug_draw.rs` stays dead.

**Cost.** S (RL59), M (the Rapier backend, which lives in the game lane and is not ours).

**Spec (patch-ready).**
- New file `crates/render/src/present/gpu_present/debug_view.rs`: `pub enum DebugView { Overdraw,
  Wireframe, Normals, ShadowAtlas, Cascades, Ao, Motion, Luminance }`, `pub fn parse_view(name: &str) ->
  Result<DebugView, ViewError>` (an unknown name is refused), and
  `pub(super) fn encode_debug_view(enc: &mut wgpu::CommandEncoder, targets: &FrameTargets, view: Option<DebugView>) -> Result<(), PresentError>`.
- New file `crates/render/src/shaders/debug_view.wgsl`: `@vertex fn vs_debug_view` and
  `@fragment fn fs_debug_view` for the full-screen views; `@vertex fn vs_debug_line` and
  `@fragment fn fs_debug_line` for the line list.
- Changed file `crates/render/src/present/gpu_present/live.rs`: `present_frame_live` (1150) calls
  `self.encode_debug_view(...)` right after `self.encode_blit(...)` (live.rs:1314) and before the
  present copy.
- Changed file `crates/render/src/debug_draw.rs`: `DebugDraw` (45) gains its first in-crate caller -
  a new `pub fn collect_lines(recorder: &mut DebugRecorder, draws: &[DrawItem])` that fills the
  `bounds` group; `line_verts` (143) keeps its contract and its byte-identical test.
- Tests: `debug_view_8_views_8_different_sha_and_off_is_byte_equal`,
  `debug_view_unknown_name_is_refused`, `debug_draw_has_a_live_caller`, `debug_draw_line_group_fills_from_draw_list`.
- Proof command: `node tools/gh-check.mjs cargo test -p engine-render --lib debug_view` and
  `node tools/gh-check.mjs cargo test -p engine-render --lib debug_draw`.
- Golden: `node tools/proof.mjs` must still print `golden d57b5c8ae3096614 OK` (twice).

### RXX-07 - PF25: one named scope per pass in the frame graph (the pass report)

**What it is.** Every pass in the compiled graph runs inside a named scope, and the frame prints one
line per pass in declaration order with its GPU ms. `pass_report.rs` already exists as the data model
(reads, writes, ran/culled/failed) and `executor.rs` already times passes; what is missing is the
per-pass name in the dump and the CSV with the production pass rows.

**Donor.** NVIDIAGameWorks/Falcor, https://github.com/NVIDIAGameWorks/Falcor, file
`Source/Falcor/RenderGraph/RenderGraphExe.cpp` (read live: every pass in the execution list runs inside
`FALCOR_PROFILE(ctx.pRenderContext, pass.name)`), licence BSD-3-Clause (read live 2026-10-20), date
read 2026-10-20.

**PORT TYPE: algorithm-shaping only.** We write our own Rust scope and our own CSV writer; the donor
gives the shape (one named scope per pass, in declaration order).

**Closes.** PF25 (GPU time on every production pass) and the "pass report" idea for the frame graph.
Side row, not gap B7, but RL59 depends on it.

**Owned paths.** `crates/render/src/executor.rs`, `crates/render/src/graph.rs`,
`crates/render/src/pass_report.rs`, `crates/render/src/perf/…` if it exists (no new crate file is
allowed here: the profiler hunk belongs to `crates/perf`, which is another lane).

**Accept.** `cargo test -p engine-render --lib pass_timing` prints
`pass_timing_every_compiled_pass_has_a_named_scope_and_a_row` PASS: every pass of the compiled arena
graph has its own query pair and one row in the report, in declaration order.

**Kill line.** The dump changes the sim hash or any digest, or the report still has fewer than 12
production pass rows after the change -> the row dies.

**Cost.** S.

**Spec (patch-ready).**
- Changed file `crates/render/src/executor.rs`: `query_pair` (480) keeps its pairing rule; add
  `pub(crate) fn pass_scope_name(pass_index: usize, pass_name: &str) -> String` and write one report
  row per pass after `resolve_query_set`, in execution order.
- Changed file `crates/render/src/pass_report.rs`: `Pass` (34) and `Plan` (48) keep their fields;
  `Plan::report_json` (184) and `Plan::report_line` (210) gain the per-pass GPU ms column.
- Changed file `crates/render/src/graph.rs`: the compiled order keeps its pass names; the existing test
  `graph::tests::pass_timing_covers_every_compiled_pass` stays green.
- Tests: `pass_timing_every_compiled_pass_has_a_named_scope_and_a_row`,
  `pass_report_rows_are_in_declaration_order`, `pass_report_does_not_change_the_digest`.
- Proof command: `node tools/gh-check.mjs cargo test -p engine-render --lib pass_timing`.
- Golden: `node tools/proof.mjs` must still print `golden d57b5c8ae3096614 OK` (twice).

### RXX-08 - VX2: volumetric clouds (side row, Tier C)

**What it is.** A Nubis-shaped cloud sky: a ray-marched cloud volume with a temporal reprojection
pass so the clouds do not crawl between frames. VX2 owns `crates/render/src/clouds.rs`, which was cut
on 2026-10-16 and can be recovered from git history.

**Donor.** AmanSachan1/Meteoros, https://github.com/AmanSachan1/Meteoros, files
`src/CloudScapes/shaders/cloudRayMarch.comp` and `src/CloudScapes/shaders/reprojection.comp`,
licence MIT (LICENSE read live 2026-10-20: "MIT License, Copyright (c) 2017 Aman Sachan"), date read
2026-10-20.

**PORT TYPE: algorithm-shaping only.** GLSL compute in the donor, WGSL for us; the method (ray march,
envelope shape, reprojection) is ported, the bytes are not.

**Closes.** VX2 (Tier C, needs GX7, GX8, RL4, RL6). Side row, not gap B7.

**Owned paths.** new `crates/render/src/clouds.rs`, new `crates/render/src/shaders/clouds.wgsl`.

**Accept.** `cargo test -p engine-render --lib clouds` prints
`clouds_reprojection_keeps_the_frame_stable` PASS: a day-to-night clip shows moving clouds and the
reprojection pass keeps the cloud pixels stable between frames.

**Kill line.** The cloud pass costs more than 1.5 ms at 1080p on the RTX 3050, or the sky costs more
than the rest of the frame -> VX2 stays unbuilt and Tier C.

**Cost.** L.

**Spec (patch-ready).**
- New file `crates/render/src/clouds.rs`: `pub struct CloudConfig { pub coverage: f32, pub density: f32,
  pub steps: u32 }`, `pub fn cloud_ray_march_step(...) -> f32` (the CPU oracle for the WGSL march),
  `pub fn reproject(prev_uv: [f32; 2], flow: [f32; 2]) -> [f32; 2]`.
- New file `crates/render/src/shaders/clouds.wgsl`: `@compute fn cs_cloud_march` and
  `@compute fn cs_cloud_reproject`.
- Tests: `clouds_reprojection_keeps_the_frame_stable`, `clouds_cpu_oracle_matches_gpu_step_count`.
- Proof command: `node tools/gh-check.mjs cargo test -p engine-render --lib clouds`.
- Golden: `node tools/proof.mjs` must still print `golden d57b5c8ae3096614 OK` (twice).

### RXX-09 - RT4 + RT8: radiance cache for light with no bake (side row, parked)

**What it is.** One radiance cache that holds the indirect light and updates it as the sun moves, so a
day-to-night sweep relights the arena with no bake step. RT4 is parked (LATER, wave67) and RT8 is
frozen by owner decision D7, so this is a read now, port later row.

**Donor.** AMD FSR SDK 2.3.0 "Redstone", https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK,
files `Kits/FidelityFX/radiancecache/include/ffx_radiancecache.h` (7.3 KB) and `ffx_radiancecache.hpp`,
licence MIT (header read live 2026-10-20: "Copyright (C) 2026 Advanced Micro Devices, Inc." plus the MIT
grant), date read 2026-10-20.

**PORT TYPE: algorithm-shaping only.** The donor is HLSL plus a C++ API; we port the cache update rule
and write our own WGSL compute.

**Closes.** RT4 (dynamic GI with no bake) and RT8 (probe-grid GI for Base). Side row, parked, not gap B7.

**Owned paths.** new `crates/render/src/radiance_cache.rs` (the tree has `restir_di.rs` and
`gi_probe.rs`; `restir_gi.rs` was cut on 2026-10-16 and is recoverable from git history).

**Accept.** `cargo test -p engine-render --lib radiance_cache` prints
`radiance_cache_sun_sweep_relights_without_a_bake` PASS: a sun sweep relights the cache with no bake
step and the tunnel leak test stays green.

**Kill line.** The sweep misses its 2.5 ms budget on the reference GPU, or the tunnel leak test fails
with the cache on -> RT4 keeps its bake fallback.

**Cost.** M.

**Spec (patch-ready).**
- New file `crates/render/src/radiance_cache.rs`: `pub struct RadianceCache { pub grid: Vec<[f32; 4]>,
  pub valid: Vec<u8> }`, `pub fn update(cache: &mut RadianceCache, sun_dir: [f32; 3], rays: &[[f32; 4]]) -> u32`
  (the CPU oracle), `pub fn sample(cache: &RadianceCache, pos: [f32; 3]) -> [f32; 3]`.
- New file `crates/render/src/shaders/radiance_cache.wgsl`: `@compute fn cs_cache_update`,
  `@compute fn cs_cache_sample`.
- Tests: `radiance_cache_sun_sweep_relights_without_a_bake`,
  `radiance_cache_tunnel_leak_test_stays_green`, `radiance_cache_update_is_order_free`.
- Proof command: `node tools/gh-check.mjs cargo test -p engine-render --lib radiance_cache`.
- Golden: `node tools/proof.mjs` must still print `golden d57b5c8ae3096614 OK` (twice).

### RXX-10 - RT6: the engine denoiser, SVGF shape (side row, parked)

**What it is.** A spatiotemporal denoiser for the ray-traced passes: reproject, filter moments,
atrous filter, final modulate. RT6 needs RT7 fixtures that do not exist, so this row is read only.

**Donor.** NVIDIAGameWorks/Falcor, https://github.com/NVIDIAGameWorks/Falcor, files
`Source/RenderPasses/SVGFPass/SVGFReproject.ps.slang` (8.6 KB), `SVGFAtrous.ps.slang`,
`SVGFFilterMoments.ps.slang`, licence BSD-3-Clause (read live 2026-10-20), date read 2026-10-20.

**PORT TYPE: algorithm-shaping only.** Slang in the donor; we write our own WGSL when RT7 lands.

**Closes.** RT6 (needs RT2-RT5). Side row, parked, not gap B7.

**Owned paths.** new `crates/render/src/denoise.rs` (nothing in the tree owns it today).

**Accept.** `cargo test -p engine-render --lib denoise` prints
`denoise_reproject_matches_the_motion_vectors` PASS (the row stays a spike until RT7 fixtures exist).

**Kill line.** RT7 fixtures never land, or the denoiser costs more than 1.0 ms at 1080p -> the row
stays parked.

**Cost.** M.

**Spec (patch-ready).**
- New file `crates/render/src/denoise.rs`: `pub struct DenoiseParams { pub variance_epsilon: f32,
  pub phi_color: f32, pub phi_depth: f32 }`, `pub fn reproject(uv: [f32; 2], flow: [f32; 2]) -> [f32; 2]`,
  `pub fn atrous_weights(...) -> [f32; 5]`.
- New file `crates/render/src/shaders/denoise.wgsl`: `@compute fn cs_svgf_reproject`,
  `@compute fn cs_svgf_moments`, `@compute fn cs_svgf_atrous`, `@compute fn cs_svgf_modulate`.
- Tests: `denoise_reproject_matches_the_motion_vectors`, `denoise_atrous_weights_sum_to_one`.
- Proof command: `node tools/gh-check.mjs cargo test -p engine-render --lib denoise`.
- Golden: `node tools/proof.mjs` must still print `golden d57b5c8ae3096614 OK` (twice).

### RXX-11 - FG1 (new row, proposed): frame generation (side row, research only)

**What it is.** Frame interpolation that shows the frame between two rendered frames. It needs an
optical-flow provider, and there is no open one: AMD's flow is host supplied and the NVIDIA Optical
Flow SDK is proprietary. So the row proposes the interface and the timing only.

**Donor.** AMD FSR SDK 2.3.0 "Redstone", https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK,
files `Kits/FidelityFX/framegeneration/fsr3/include/ffx_frameinterpolation.h` (24.9 KB, version 3.1.6
read live) and `ffx_opticalflow.h`, licence MIT (header read live 2026-10-20), date read 2026-10-20.

**PORT TYPE: algorithm-shaping only.** HLSL plus a C API in the donor; we write our own WGSL and our own
Rust interface when a flow source exists.

**Closes.** No board row: propose a new row "FG1 frame generation" next to RL38. Side row, research only.

**Owned paths.** none yet (no file lands with this row).

**Accept.** None until a flow provider exists; the row stays a read.

**Kill line.** No optical-flow provider in the tree by the time the row is scheduled -> the row dies on
arrival.

**Cost.** L.

**Spec (patch-ready).**
- No file lands. The interface sketch for the future row: a trait
  `pub trait FlowProvider { fn flow(&self, prev: &Frame, next: &Frame) -> Vec<[f32; 2]>; }` in
  `crates/render/src/post/frame_gen.rs`, with a CPU oracle and a placeholder that refuses.
- Tests (only when the row is scheduled): `frame_gen_flow_provider_is_refused_without_a_source`.
- Proof command: `node tools/gh-check.mjs cargo test -p engine-render --lib frame_gen`.
- Golden: `node tools/proof.mjs` must still print `golden d57b5c8ae3096614 OK` (twice).

### RXX-12 - VX3: hair and strand shading (side row, research only)

**What it is.** Anisotropic shading for hair cards through linearly transformed cosines, so a torch
or the sun lights a strand correctly. VX3 also asks for skin subsurface scattering and eye shading:
no MIT or Apache donor was found for those two today, so this row covers the hair half only.

**Donor.** selfshadow/ltc_code, https://github.com/selfshadow/ltc_code, files `webgl/` and `fit/`
(the LTC tables and the fit code; the LICENSE names the paper as a condition), licence BSD-3-Clause
(LICENSE read live 2026-10-20), date read 2026-10-20.

**PORT TYPE: algorithm-shaping only.** GLSL and C in the donor; we port the lobe and the fit rule into
our material path as our own Rust and WGSL. The paper reference must travel with the port.

**Closes.** VX3 (hair half; skin SSS and eyes stay unowned). Side row, Tier C, research only.

**Owned paths.** `crates/render/src/material_layer.rs`, `crates/render/src/material/…` (the material
crate owns the compiled material; the render side keeps the tables).

**Accept.** `cargo test -p engine-render --lib ltc` prints
`ltc_lobe_matches_the_reference_under_a_area_light` PASS: the lobe matches the donor table within the
material tolerance.

**Kill line.** VX3 stays Tier C and the proof game has no hair cards -> the row waits; the tables are
not landed.

**Cost.** S.

**Spec (patch-ready).**
- Changed file `crates/render/src/material_layer.rs`: a new `pub fn ltc_magnitude(...) -> [f32; 2]` and
  `pub fn ltc_amplitude(...) -> [f32; 3]` beside the existing lobe helpers.
- New file `crates/render/src/material/ltc_tables.rs`: the 2 x 2 amplitude and magnitude tables with the
  paper reference in the header (the numbers are re-fit by us, never copied).
- Tests: `ltc_lobe_matches_the_reference_under_a_area_light`, `ltc_tables_are_monotone_in_roughness`.
- Proof command: `node tools/gh-check.mjs cargo test -p engine-render --lib ltc`.
- Golden: `node tools/proof.mjs` must still print `golden d57b5c8ae3096614 OK` (twice).

---

## What the lead merges

- New board rows to add: RXX-01 (LF5 slice 2), RXX-02 (RL60 + RL32 + GS18b), RXX-03 (RL41 slice 2),
  RXX-04 (RL38), RXX-05 (RL35 + QA4), RXX-06 (RL59 + RL58 caller), RXX-07 (PF25 slice 2),
  RXX-08 (VX2), RXX-09 (RT4 + RT8), RXX-10 (RT6), RXX-11 (new FG1), RXX-12 (VX3 hair half).
- B7 order: LF5 -> RL60 + RL32 -> RL41 -> RL38 -> GS18b. Side row next: RL35 + QA4.
- Finish-line item 13 rest: cvars, the in-game console and the Perfetto trace have no row and no
  owner; pause, step and time scale exist in the game lane only.
