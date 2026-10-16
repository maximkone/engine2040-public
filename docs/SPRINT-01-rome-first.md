### Rome first (2026-10-02 restructure: dispatch these first; rows byte-identical, see docs/plan/RESTRUCTURE-2026-10-02.md)

- [ ] BV3 viewer: per-pass GPU timers in the capture manifest (decision D1, Eyes job) — `crates/viewer` reads wgpu
  timestamp queries through Bevy's render diagnostics (`RenderDiagnosticsPlugin`, feature `bevy_diagnostic` + the
  adapter's `TIMESTAMP_QUERY`) and writes `target/bv1/manifest.json` beside the PNGs: adapter name, wgpu version, frame
  p50/p95/max in ms over the captured frames and one row per render pass with its GPU ms; an adapter without timestamp
  queries writes `"gpu_timers": "unsupported"` and the frame wall times, never a fake number. Tier A. Size S. Owns
  `crates/viewer/src/manifest.rs` and the capture wiring in `crates/viewer/src/main.rs`. Accept: F2P
  `cargo test --locked -p engine-viewer --no-default-features --lib manifest` (percentiles, unsupported shape, JSON
  round-trip x2); pilot capture writes a manifest whose pass rows sum within 10% of the frame GPU time; golden
  unchanged. needs: BV1.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
  Done so far 2026-10-05 (Eyes, not ticked): `manifest.json` carries adapter `NVIDIA GeForce RTX 3050 6GB Laptop GPU`,
  `wgpu` 30.0.1, frame p50 5.45 ms, p95 7.378 ms, max 86.09 ms over 601 frames at 1920x1080 (dev profile, kit plus 2
  fighters), 8 pass rows (`main_opaque_pass_3d` 1.549 ms, `per_view_shadow_pass` 0.073 ms, `clustering` 0.086 ms ...) and
  `gpu_ms_sum` 1.866. Open: the clause 'pass rows sum within 10% of the frame GPU time' has no measure, Bevy has no
  whole-frame GPU span (BV8).
- [ ] BV5 viewer: release pilot capture and the golden — run the BV1 pilot in the release profile and prove the sim
  untouched. Tier A. Size S. Owns no source file; records numbers in this row. Command A: `cargo run --locked -p
  engine-viewer --release -- --scene arena --fighters 2 --frames 120 --capture target/bv1/` (class `exclusive`
  in the heavy queue, the first release build of Bevy takes over an hour; run it with `run_in_background`). Accept:
  `target/bv1/frame_000060.png` exists, log `bv1 tris=<n> joints=43` with n over 5000, `manifest.json` `frame_ms.p95` at most
  33; command B `node tools/proof.mjs` prints golden `d57b5c8ae3096614` x2; `cargo deny check` (with advisories) green.
  needs: BV1.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
- [ ] BV6 viewer: clips cut to each body — today `crates/viewer/src/main.rs` builds one clip set from the hero's rest
  translations (`retarget_clip(&clip, &rest, &hero_rig)`); the 3 opponent bodies share the joint names but may have other
  bone lengths. Tier A. Size S. Owns `crates/viewer/src/rig.rs` and `crates/viewer/src/main.rs`. Build: one clip set and one
  animation graph per distinct body in `BODIES` (`Plan.clips` becomes a list per body, `Clips` a resource per body). Accept:
  new test `rig::tests::every_body_gets_clips_at_its_own_size` in `crates/viewer/src/rig.rs`: for each of the 4 bodies in
  `BODIES` and each of the 3 clips the first pelvis translation key y is within 10% of that body's rest pelvis y (12 of
  12); command `cargo test --locked -p engine-viewer --no-default-features --lib rig`; golden unchanged.
  needs: BV2.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
- [ ] BV7 viewer: camera that keeps both fighters in frame — the pilot picture shows the hero small and the arena kit as big
  slabs (a wood slab fills most of the view from camera yaw 0, 90 and 180; yaw 270 shows the arena). Tier A. Size S.
  Owns a new `crates/viewer/src/camera.rs` (std only, declared in `crates/viewer/src/lib.rs`) and the call in
  `aim_camera` in `crates/viewer/src/main.rs`. Build: move the camera math out of `aim_camera` into
  `camera::aim(hero, other, dist, yaw_deg) -> (eye, target)` and a check that no kit box from `kit::arena_layout`
  lies between the eye and the target. Accept: test `camera::tests::both_fighters_inside_the_frame` over the 600 frozen
  frames (hero and opponent inside a 45 degree vertical field of view at 16:9 on 600 of 600 ticks) and
  `camera::tests::no_kit_between_eye_and_target` (0 of 600 blocked); command `cargo test --locked -p engine-viewer
  --no-default-features --lib camera`; golden unchanged. needs: BV2.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
- [ ] BV8 viewer: whole-frame GPU time for the BV3 check — Bevy gives one timestamp span per pass and no frame total.
  Tier A. Size S. Owns `crates/viewer/src/main.rs` (`collect_gpu_timers`) and `crates/viewer/src/manifest.rs`. Build:
  record the earliest span start and the latest span end of a frame as `gpu_frame_ms` (wgpu timestamp queries through
  `RenderDiagnosticsPlugin`; if Bevy 0.20 exposes only elapsed times, add one timestamp pair around the whole camera render
  with `RenderContext`/a diagnostic span in a custom node) and write it as `"gpu_frame_ms"` in `manifest.json` beside
  `gpu_ms_sum`. Accept: test `manifest::tests::the_frame_gpu_time_is_in_the_json` (value round-trips x2); pilot
  `engine-viewer --scene arena --fighters 2 --frames 120 --capture target/bv1/` writes a manifest where `gpu_ms_sum` is
  within 10% of `gpu_frame_ms` (then tick BV3); an adapter without timestamp queries still writes `"unsupported"`.
  needs: BV3.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
- [ ] BV9 viewer: the walls meet the floor — the pilot picture `target/camtest_270/frame_000090.png` shows sky between
  the wood wall and the sand (the wall floats or the floor stops short). Tier A. Size S. Owns `crates/viewer/src/kit.rs`
  (`arena_layout`). Build: every wall and gate box rests on the floor top (y = 0) and its footprint overlaps the floor
  edge or sits inside it. Accept: test `kit::tests::walls_meet_the_floor` (gap at most 0.01 m in y and in x/z for every
  wall_* and gate box, 0 failures) with `cargo test --locked -p engine-viewer --no-default-features --lib kit`; pilot
  frame 90 at `--cam 270` shows no sky under any wall. needs: BV2.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
- [ ] BV10 viewer: one look switchboard — owner yes 2026-10-05 ("Bevy has the look, we switch on almost none of it").
  Tier A. Size S. Owns a new `crates/viewer/src/look.rs` (std only, declared in `lib.rs`), the `--look <off|v1|rt>` flag
  in `crates/viewer/src/main.rs`, and a `look` block in `crates/viewer/src/manifest.rs`. Build: `Look { sky, ao, bloom,
  taa, soft_shadows, fog, ssr, contact_shadows, rt }` with `Look::preset(name)`: `off` = today's picture, `v1` = every
  field but `rt` on, `rt` = v1 plus rt; each later look row only reads its own field. Accept: tests
  `look::tests::off_is_todays_picture`, `look::tests::v1_turns_on_all_but_rt`, `manifest::tests::look_block_round_trips`
  (x2 byte-equal); command `cargo test --locked -p engine-viewer --no-default-features --lib`. needs: BV2.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
- [ ] BV11 viewer look: real sky and sky light — the flat `ClearColor` blue and near-black shadows go. Tier A. Size S.
  Owns the `look.sky` branch in `crates/viewer/src/main.rs` (`setup_scene`). Build: Bevy 0.20 `bevy::light::Atmosphere`
  on the camera and its generated environment light (`GeneratedEnvironmentMapLight` or the atmosphere environment light
  in `bevy_light-0.20.0-rc.2/src/atmosphere.rs`/`probe.rs`; read them first), sun = the existing `DirectionalLight`;
  lower `GlobalAmbientLight` so the sky does the fill. Accept: pilot (RTX 3050, build `cargo build --locked -p
  engine-viewer --features window` through the heavy queue): `engine-viewer --scene arena --fighters 2 --frames 120
  --look off --capture target/look/BV11-before/` and the same with `--look v1 --capture target/look/BV11/`; paste frame
  90 of both as paths in the report, the manifest `look` block, and p95 frame ms at 1080p for both (30 fps bar: p95 at
  most 33.3 ms with `--look v1`); golden `d57b5c8ae3096614` unchanged (the viewer never writes sim state). needs: BV10.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
- [ ] BV12 viewer look: ambient occlusion and contact shadows — soft dark corners where the fighters touch the sand and
  the walls meet the floor. Tier A. Size S. Owns the `look.ao` and `look.contact_shadows` branches in
  `crates/viewer/src/main.rs`. Build: `ScreenSpaceAmbientOcclusion` (`bevy_pbr-0.20.0-rc.2/src/ssao/mod.rs`; it needs
  `Msaa::Off` and depth/normal prepasses on the camera) and `ContactShadows`
  (`bevy_pbr-0.20.0-rc.2/src/contact_shadows.rs`). Accept: pilot (RTX 3050, build `cargo build --locked -p engine-viewer
  --features window` through the heavy queue): `engine-viewer --scene arena --fighters 2 --frames 120 --look off
  --capture target/look/BV12-before/` and the same with `--look v1 --capture target/look/BV12/`; paste frame 90 of both
  as paths in the report, the manifest `look` block, and p95 frame ms at 1080p for both (30 fps bar: p95 at most 33.3 ms
  with `--look v1`); golden `d57b5c8ae3096614` unchanged (the viewer never writes sim state). needs: BV10, BV11.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
- [ ] BV13 viewer look: bloom and exposure — bright sand and metal glow a little instead of clipping flat. Tier A. Size
  S. Owns the `look.bloom` branch in `crates/viewer/src/main.rs` and the bevy feature list in
  `crates/viewer/Cargo.toml`. Build: add the `bevy_post_process` feature (MIT/Apache; run `cargo fetch` once, Cargo.lock
  changes, `cargo deny check licenses` stays green; never `tonemapping_luts`, ktx2 or zstd stay banned), `Bloom` on the
  camera, keep `Tonemapping::AcesFitted`. Accept: pilot (RTX 3050, build `cargo build --locked -p engine-viewer
  --features window` through the heavy queue): `engine-viewer --scene arena --fighters 2 --frames 120 --look off
  --capture target/look/BV13-before/` and the same with `--look v1 --capture target/look/BV13/`; paste frame 90 of both
  as paths in the report, the manifest `look` block, and p95 frame ms at 1080p for both (30 fps bar: p95 at most 33.3 ms
  with `--look v1`); golden `d57b5c8ae3096614` unchanged (the viewer never writes sim state). needs: BV10, BV12.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
- [ ] BV14 viewer look: smooth edges — temporal anti-aliasing so the sword, shield and wall edges stop stair-stepping.
  Tier A. Size S. Owns the `look.taa` branch in `crates/viewer/src/main.rs` and the bevy feature list in
  `crates/viewer/Cargo.toml`. Build: add `bevy_anti_alias` (MIT/Apache, `cargo fetch` once, deny check green),
  `TemporalAntiAliasing` on the camera with `Msaa::Off`; frames are drawn per sim tick, so warm TAA with 8 repeated
  draws of tick 0 before the first capture and say so in the manifest. Accept: pilot (RTX 3050, build `cargo build
  --locked -p engine-viewer --features window` through the heavy queue): `engine-viewer --scene arena --fighters 2
  --frames 120 --look off --capture target/look/BV14-before/` and the same with `--look v1 --capture target/look/BV14/`;
  paste frame 90 of both as paths in the report, the manifest `look` block, and p95 frame ms at 1080p for both (30 fps
  bar: p95 at most 33.3 ms with `--look v1`); golden `d57b5c8ae3096614` unchanged (the viewer never writes sim state).
  needs: BV13.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
- [ ] BV15 viewer look: soft shadows — the knife-edge sun shadow becomes soft with distance from the caster. Tier A.
  Size S. Owns the `look.soft_shadows` branch in `crates/viewer/src/main.rs` and the bevy feature list in
  `crates/viewer/Cargo.toml`. Build: bevy feature `experimental_pbr_pcss`, `soft_shadow_size` on the `DirectionalLight`,
  `ShadowFilteringMethod::Temporal` (or `Gaussian` when TAA is off), cascade bounds from the arena size in
  `CascadeShadowConfigBuilder`. Accept: pilot (RTX 3050, build `cargo build --locked -p engine-viewer --features window`
  through the heavy queue): `engine-viewer --scene arena --fighters 2 --frames 120 --look off --capture
  target/look/BV15-before/` and the same with `--look v1 --capture target/look/BV15/`; paste frame 90 of both as paths
  in the report, the manifest `look` block, and p95 frame ms at 1080p for both (30 fps bar: p95 at most 33.3 ms with
  `--look v1`); golden `d57b5c8ae3096614` unchanged (the viewer never writes sim state). needs: BV14.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
- [ ] BV16 viewer look: fog, sun shafts and reflections — air between the camera and the far walls, light shafts through
  the gate, wet-looking sand where the material record says so. Tier A. Size S. Owns the `look.fog` and `look.ssr`
  branches in `crates/viewer/src/main.rs`. Build: `DistanceFog` (`bevy_pbr-0.20.0-rc.2/src/fog.rs`), `VolumetricFog` on
  the camera and `VolumetricLight` on the sun (`bevy_light-0.20.0-rc.2/src/volumetric.rs`), `ScreenSpaceReflections`
  (`bevy_pbr-0.20.0-rc.2/src/ssr/mod.rs`, needs the deferred or depth prepass setup it documents). Accept: pilot (RTX
  3050, build `cargo build --locked -p engine-viewer --features window` through the heavy queue): `engine-viewer --scene
  arena --fighters 2 --frames 120 --look off --capture target/look/BV16-before/` and the same with `--look v1 --capture
  target/look/BV16/`; paste frame 90 of both as paths in the report, the manifest `look` block, and p95 frame ms at
  1080p for both (30 fps bar: p95 at most 33.3 ms with `--look v1`); golden `d57b5c8ae3096614` unchanged (the viewer
  never writes sim state). Then the look builder files one line in `docs/FINISH-RICH.md` beside A4 with the six
  before/after frame paths. needs: BV15.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
- [ ] BV17 viewer spike: Bevy's ray-traced light and virtual geometry on the RTX 3050 — owner yes 2026-10-05, runs now
  despite D7 (one measured spike, no new plan files). Bevy 0.20 ships both as experiments: `bevy_solari` (ray-traced
  direct and bounce light, the Lumen-like part) and `meshlet` (Nanite-like virtual geometry,
  `bevy_pbr-0.20.0-rc.2/src/meshlet/`). Our RT2-RT13 and VG1-VG11 rows (about 130 sessions) would rebuild them in
  `crates/render`. Tier A. Size M. Owns a new optional feature `rt` in `crates/viewer/Cargo.toml` (bevy features
  `bevy_solari`, `meshlet`, `meshlet_processor`; never `dlss`: NVIDIA SDK, D5) and a new `crates/viewer/src/rt.rs`
  (window-only, `look.rt`). Build: `cargo fetch` once and read the fetched `bevy_solari-0.20.0-rc.2/src` and Bevy's
  solari and meshlet examples for the exact names; `SolariLighting` on the camera with `RaytracingMesh3d` on the kit and
  fighter meshes; separately one kit mesh as `MeshletMesh3d` through `MeshletPlugin`. The first build runs alone
  (`heavy.mjs run --class exclusive`); record its minutes and peak RAM. Accept: a report table with: adapter supports
  ray queries (yes/no from wgpu), Solari p50/p95 frame ms at 1080p and 720p, meshlet p95 frame ms, VRAM if reported,
  first-build minutes, frame 90 pictures `target/look/BV17-solari/` and `target/look/BV17-meshlet/`; `cargo deny check
  licenses` green; golden unchanged. Decision rule, written into `docs/archive/2026-10-08/plan/render-2040.md` as one dated paragraph:
  Solari p95 at most 33.3 ms at 1080p, or at 720p with a clean picture, means PROPOSED: re-aim RT2-RT13 at porting and
  fixing Bevy Solari in the viewer (Maxim decides); slower means the measured gap is written down and the RT rows stay
  as they are. needs: BV2.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
- [ ] NS31a2 material: the editor document holds the material table and edits it with undo — add to `Document` in
  `crates/editor/src/commands.rs` a `materials: engine_material::MaterialSet` (default `MaterialSet::siege_kit()`;
  `crates/editor/Cargo.toml` takes `engine-material`, add the edge `engine-editor -> engine-material` to
  `allowedEdges` in `tools/workspace-check.mjs`), and two ops `Op::PutMaterial { text }` and `Op::SetMaterialParam
  { material, path, value }` that call `MaterialSet::parse_push` / `put` and `MaterialSet::set_param`; the undo of
  `SetMaterialParam` is `set_param` with the returned `ParamEdit.before`, the undo of `PutMaterial` is `put` of the
  returned previous record (or removal of a new one: add `MaterialSet::remove_last` with a test). Tier A. Size S.
  Owns the two ops in `crates/editor/src/commands.rs` and one new test file `crates/editor/tests/material_ops.rs`.
  Accept: F2P `cargo test -p engine-editor --test material_ops` (put then undo restores `MaterialSet::look_digest`
  and `sim_digest` exactly; `SetMaterialParam look.specular_roughness 0.08` then undo gives the old digests;
  `SetMaterialParam body.toughness 9` moves `sim_digest` only; a bad value is refused with the record path and the
  document revision does not change); P2P `cargo test -p engine-editor --lib` green, golden `d57b5c8ae3096614`
  unchanged. needs: NS31a.
- [ ] NS31b1 assets: cook the two missing texture slots — in `crates/assets/src/cooked.rs` add `TextureSlot::Occlusion`
  (linear, red channel) and `TextureSlot::Emissive` (sRGB, linearized on cook like `BaseColor`; see
  `TextureEncoding`), handle both in `cook_texture` (`crates/assets/src/texture.rs`, line 96) and in every
  `match` on `TextureSlot` the compiler flags. Tier A. Size S. Owns `crates/assets/src/cooked.rs` and
  `crates/assets/src/texture.rs`. Accept: F2P `cargo test -p engine-assets --lib cooked texture` (a new test
  `five_slots_round_trip` cooks one 4x4 image per slot and the bytes are equal x2; Emissive texels are
  linearized and Occlusion texels are not; a duplicate slot in one material is still refused); P2P `cargo test
  -p engine-assets` green and the cooked bundle digest of the existing assets is unchanged (the existing digest
  tests stay green), golden unchanged. needs: none.
- [ ] NS31b2 render: a draw can carry the four texture slots — in `crates/render/src/present.rs` extend `DrawItem`
  (line 360) with `orm: Option<TextureKey>`, `normal: Option<TextureKey>`, `emissive: Option<TextureKey>` next
  to `base_color`; an unknown key fails the frame with `InvalidDraw` exactly like `base_color` does; every
  struct literal of `DrawItem` in the workspace gets `None` (use Grep `DrawItem {`). No shader change here.
  Tier A. Size S. Owns the `DrawItem` struct and its validation in `crates/render/src/present.rs` and the
  literals the compiler flags. Accept: F2P `cargo test -p engine-render --lib --features window present` (new
  tests `unknown_orm_key_is_invalid_draw`, `unknown_normal_key_is_invalid_draw`,
  `  unknown_emissive_key_is_invalid_draw`); P2P the existing `present` tests green and the frame bytes of an
  untextured draw are identical to before (the existing capture-hash tests stay green), golden unchanged.
  needs: NS31b1.
- [ ] NS31c1 material: compile records to WGSL — new file `crates/material/src/compile.rs` with `pub struct Compiled`
  (`wgsl: String`, `key: u64`, structure accessors `uniform_bytes() -> usize` and `texture_bindings() -> Vec<TextureSlot>`)
  and `pub fn compile(record: &MaterialRecord) -> Compiled` that generates the WGSL for the record's channels
  (base color, metallic-roughness, occlusion, normal, emissive, coat, sheen) plus `pub const CHANNEL_FLOATS: usize`
  for the channel block; `key` is a stable hash of the structure (same channels same key, adding a coat changes
  it). Tier A. Size M. Owns `crates/material/src/compile.rs`. Accept: F2P `cargo test -p engine-material --lib
  compile` (all 21 records of `MaterialSet::shipped()` compile to non-empty WGSL, keys equal x2, a coated record
  has a different key and strictly more bindings than the same record uncoated); P2P `cargo test -p engine-material`
  green, golden unchanged. needs: NS31b1.
- [ ] NS31c2 render: a pipeline from the generated WGSL — new file `crates/render/src/material_pipeline.rs` (window
  feature) with `pub fn pipeline_for(device, compiled: &engine_material::compile::Compiled) ->
  Result<wgpu::RenderPipeline, MaterialPipelineError>` that creates the shader module from `compiled.wgsl`,
  binds group 0 (camera, light, `MaterialU` of `compiled.structure.uniform_bytes()` bytes, channel block of
  `CHANNEL_FLOATS` floats) and group 1 only for the slots in `compiled.structure.texture_bindings()`, inside a
  validation error scope (an error becomes `MaterialPipelineError::Validation(text)`, never a panic); pipelines
  are cached by `compiled.key` in the existing manifest `crates/render/src/variants.rs` (`MAX_VARIANTS` 32).
  `crates/render/Cargo.toml` takes `engine-material` (edge `engine-render -> engine-material` in
  `tools/workspace-check.mjs`; mind that engine-material depends on engine-destruction, check there is no
  cycle). Tier A. Size M. Owns `crates/render/src/material_pipeline.rs`. Accept: F2P `cargo test -p engine-render
  --lib --features window material_pipeline` (all 21 records of `MaterialSet::shipped()` build a pipeline on the
  RTX 3050 with 0 validation errors; they share at most 12 pipelines; no adapter means the test prints SKIP and
  passes, as the other GPU tests do); golden unchanged. needs: NS31c, NS31b2.
- [ ] NS31c3 render: live swap and the 1 s bar — in `crates/render/src/material_pipeline.rs` add `pub struct
  MaterialLive` holding an `engine_material::reload::Reloader`, the current pipeline and the uniform buffer:
  `apply_text(&mut self, text)` runs `Reloader::apply_text`; `Reload::Values` writes the buffer; `Reload::Structure`
  builds the new pipeline first and swaps only on success (the old one keeps drawing meanwhile, the HR2 pattern in
  `crates/render/src/live.rs`: `LiveSet::apply_notice` keeps the last good); a refusal changes nothing. Tier A.
  Size S. Owns `MaterialLive` in `crates/render/src/material_pipeline.rs`. Accept: F2P `cargo test -p
  engine-render --lib --features window material_live` (value edit reaches a presented frame in at most 100 ms;
  a structure edit, adding a coat, in at most 1000 ms at p95 over 20 runs on the RTX 3050, numbers written to
  `target/ns31c/reload.json`; 3 of 3 broken texts refused and the capture hash of the last good frame unchanged;
  no adapter: SKIP); golden unchanged. needs: NS31c2.
- [ ] NS31d2 material: the furnace twin below roughness 0.3 — in `crates/material/src/furnace.rs` replace the
  l-space quadrature of the specular part by half-vector importance sampling (copy the method of
  `spec_integral` in `crates/material/src/dfg.rs` tests: GGX CDF inversion in `u`, `phi` grid, weight
  `4 * vh * V * nl / nh`) so the grid can start at perceptual roughness 0.1. Tier A. Size S. Owns
  `crates/material/src/furnace.rs`. Accept: F2P `cargo test -p engine-material --features validate furnace`
  with `ROUGHNESS_GRID = [0.1, 0.3, 0.5, 0.75, 1.0]`: the same cases and bars as today (1.02, anisotropy 1.03);
  if a case fails, fix the shader in `compile.rs` and the twin together and say so in the row; golden unchanged.
  needs: NS31d.
- [ ] NS31d3 render: GPU swatches against the twin — new test file `crates/render/tests/material_swatch.rs` (window
  feature): render a lit sphere for steel, wet marble (`marble_wet`), cloth, skin, ceramic (clear coat) and glass
  from `MaterialSet::shipped()` through `pipeline_for` (NS31c2) at 128x128 and compare the center pixel region
  to the CPU twin `engine_material::channels::twin_layers` plus `furnace`-style lighting (add a `pub fn
  twin_lit_rgb(record, coverage, camera, light) -> [f32; 3]` in `crates/material/src/twin.rs` made from the
  functions in `furnace.rs`, keeping `furnace.rs` test-only). Tier A. Size M. Owns the test file and
  `crates/material/src/twin.rs`. Accept: F2P `cargo test -p engine-render --test material_swatch --features
  window` (6 swatches within 2/255 per channel of the twin; no adapter: SKIP); P2P furnace tests green, golden
  unchanged. needs: NS31c2, NS31d.
- [ ] NS31e2 viewer: events into coverage — new file `crates/viewer/src/coverage.rs` with `pub struct CoverageTracker`
  holding one `engine_material::channels::TimedEvent` list per fighter or prop id, `push(id, tick, event)` and
  `at(id, tick) -> Coverage` (a `Coverage::replay`), fed from the hit and damage fields of the frozen frame
  (`engine_app::PresentationFrame`: read its fields first; if it carries no hit, water or heat event at all, write
  that in `docs/BLOCKERS.md` and stop: the sim side then needs a presentation event row, do not invent one).
  Wire it in `crates/viewer/src/material.rs` through `RecordLook::describe_with_coverage`. Tier A. Size S. Owns
  `crates/viewer/src/coverage.rs`. Accept: F2P `cargo test -p engine-viewer --no-default-features --lib coverage`
  (3 blood events and 1 water event from a seeded list give equal `Coverage` x2; a rolled-back list replays to
  the new timeline; `CoverageTracker::at` never reads a tick after the asked one); golden unchanged.
  needs: BV4.
- [ ] NS31f2 material: MaterialX constants and images — new file `crates/material/src/mtlx_import.rs` with `pub fn
  import_mtlx(text: &str) -> Result<Import, MtlxError>` (reuse `Import`, `Warning`, `WarningKind` from
  `gltf_import.rs`): a small own XML reader (no new dependency; elements, attributes, self-closing, comments)
  that reads one `<standard_surface>` or `<open_pbr_surface>` node whose inputs are constants (`value=`) or
  `<image>` files, mapping `base_color`, `metalness`, `specular_roughness` (OpenPBR) or `specular_roughness`,
  `coat`, `sheen`, `emission`, `specular_IOR`, `transmission`; any other input is a named `Unsupported`
  warning; a `<nodegraph>` or an input connected to another node is refused with
  `MtlxError::UnsupportedNode(name)`. Tier B. Size S. Owns `crates/material/src/mtlx_import.rs`. Accept: F2P
  `cargo test -p engine-material mtlx_import` (6 hand-written constant or image `.mtlx` strings in the module
  tests import, the records round-trip through `to_text`/`parse`; 2 procedural nodegraph strings refused with
  `UnsupportedNode` naming the node); golden unchanged. needs: NS31f.
- [ ] NS31g1 assets: BC5 for normal maps — in `crates/assets/src/bc.rs` add `BcFormat::Bc5`, `encode_bc5_block` and
  `decode_bc5_block` (two BC4 channels, red and green, 16 bytes per 4x4 block; the normal's blue is rebuilt in
  the shader), `block_bytes` returns 16 for it, and route `TextureSlot::Normal` to BC5 in `encode_image`. Tier A.
  Size S. Owns `crates/assets/src/bc.rs`. Accept: F2P `cargo test -p engine-assets --lib bc` (a 1024x1024
  seeded normal map cooks to 1 MiB plus mips, mean angular error at most 1.5 degrees against the source, cook
  bytes equal x2); P2P the BC1 and BC3 tests and the cooked bundle digest of existing assets unchanged, golden
  unchanged. needs: NS31b1.
- [ ] NS31g2 assets: BC7 for base color and ORM — in `crates/assets/src/bc.rs` add `BcFormat::Bc7` with an encoder
  that writes mode 6 only (one subset, 7-bit endpoints plus p-bit, 4-bit indices; PORT note: the format is the
  Khronos data format spec, write it yourself, no `ctt`, no C code) and `decode_bc7_block` for mode 6; route
  `BaseColor` and `MetallicRoughness` to BC7. Tier A. Size M. Owns `crates/assets/src/bc.rs`. Accept: F2P `cargo test -p
  engine-assets --lib bc7` (1024x1024 base color PSNR at least 38 dB, bytes equal x2 with 1 and 8 threads of the
  existing cook pool; 4x smaller than RGBA8 with mips); P2P existing assets' bundle digest unchanged, golden
  unchanged. needs: NS31g1.
- [ ] NS31h2 playtest: the material gallery — new file `crates/playtest/src/material_gallery.rs` with `pub fn
  gallery(records: &MaterialSet, lights: [LightPreset; 3]) -> Vec<Capture>` rendering a sphere and a slab per
  record under three lights (day, torch, overcast) through `pipeline_for` (NS31c2) and `material.compare` math
  from `engine_material::compare` for the pair report. Tier B. Size S. Owns that file. Accept: F2P `cargo test -p
  engine-playtest --lib material_gallery` (19 records x 3 lights = 57 captures in at most 120 s on the RTX 3050;
  the compare report of dry versus wet marble is identical x2, luminance ratio below 0.95, highlight peak ratio
  at least 2; no adapter: SKIP); golden unchanged. needs: NS31c2.
- [ ] VS1 game: arena kit from glTF in the window and headless twin — parse
  `content/arena/placement.json`, import each `content/arena/src/<asset>.gltf`
  through `engine_assets::mesh_import::import_gltf_path`, fit to the placement
  box, upload with `Presenter::upload_mesh`, push the draws in place of the six
  dressing cubes. Tier A. Size M. Owns `crates/game/src/arena_kit.rs`, the
  `app` feature line in `crates/game/Cargo.toml`, the staging call in
  `crates/game/src/windowed.rs` and `crates/game/src/bin/arena.rs`. Accept:
  F2P a test stages 8 kit meshes with at least 5,000 triangles in total (today
  7 cubes, 84 triangles) and the frame-0 capture SHA differs from the cube
  capture SHA; P2P golden `d57b5c8ae3096614` unchanged, `cargo test -p
  engine-game --lib` green, headless default builds with no `gltf` in
  `cargo tree -p engine-game -e normal`. needs: CT8.
  progress 2026-10-10: kit tests green with `--features app` (`kit_imports_8_files_over_5000_tris`, `_x2`, `kit_twin_stages_11_draws_over_5000_tris`; 12/12 with `-- kit`); headless `cargo tree -p engine-game -e normal` has no gltf/engine-assets/image. Not ticked: default `cargo test -p engine-game --lib` not yet green (cold rebuild hit the heavy.mjs 60 s cap) and the golden not run (no sim path touched). VS4, VS3 and RA14 packets in target/queue/ready/60-62.
- [ ] VS3 game: kit textures — VS1 meshes bind their glTF base-color images
  from `content/arena/textures` through VS2. Tier A. Size S. Owns the texture
  half of `crates/game/src/arena_kit.rs`. Accept: F2P the sand region of the
  frame-60 capture has a luminance variance at least 4 times the untextured
  run; P2P golden unchanged. needs: VS1, VS2.
- [ ] VS4 game: fighters at their simulated positions — `App::fighter_views()`
  returns slot, position and yaw from the authoritative dataset each tick; the
  4 CT7 bodies import (after `fighter_bind::bind_all`) and draw in bind pose at
  those positions in the window and capture legs. Tier A. Size M. Owns
  `crates/game/src/fighter_draw.rs`, `fighter_views` in
  `crates/game/src/lib.rs`, the window-leg feed in `crates/game/src/camera_feed.rs`.
  Accept: F2P for a 2-fighter 120-tick run every frame's fighter positions
  equal `fighter_views` (test) and at least 2 meshes of over 5,000 triangles
  are drawn; P2P golden unchanged. needs: VS1.
- [ ] VS5a anim: fighter pose to joint matrices — LOC2 `LocalPose` to the 45
  AN1 skinning matrices through the glTF-to-AN1 joint remap (43-bone
  bijection, 2 identity nodes). Tier A. Size M. Owns the remap and matrix
  builder in `crates/anim` plus its tests. Accept: F2P matrices for tick t are
  byte-identical x2 and the bind pose gives identity-times-inverse-bind within
  1e-5; P2P `cargo test -p engine-anim --lib` green. needs: VS4.
- [ ] VS5b game: skinned, animated fighters — `PoseSource::ImportedFighter`
  returns the live pose instead of `ArtDeferred`; VS5a matrices feed
  `v1_skin::GpuSkinned`; clips from `content/locomotion`. Tier A. Size M. Owns
  the `ImportedFighter` arm in `crates/game/src/lib.rs` and the skin bind in
  `crates/game/src/fighter_draw.rs`. Accept: F2P over 30 frames each fighter's
  silhouette mask changes (limb motion) and no frame has magenta over 0.5%;
  P2P golden unchanged. needs: VS5a.
- [ ] VS7 game: torches are lights — torch actors from the placement feed the
  clustered light list (GX2) with presentation-only flicker; a night preset.
  Tier B. Size M. Owns the light feed in `crates/game/src/arena_kit.rs`.
  Accept: F2P the frame light list holds every torch actor; at night the mean
  luminance within 2 m of a torch is at least 2 times the far sand; P2P
  golden unchanged, 0 validation errors. needs: VS1, GX2.
- [ ] VS9 proof: daily arena shot — one command runs the arena capture
  (`--scene arena --fighters 2 --camera-path arena600 --frames 120 --exit`),
  writes 3 PNGs, their manifests and silhouette masks under `target/vs9/`, and
  refuses the cube SHA, the 64x48 noise SHA, empty fighter boxes and magenta
  over 0.5%. Tier A. Size S. Owns `tools/arena-shot.mjs`. Accept: F2P the
  command FAILs on today's build (cube SHA) and PASSes after VS1+VS4; P2P
  `node tools/proof.mjs` golden unchanged. needs: VS1.
- [ ] RA14 render: arena content through the presenter — CT7 fighters +
  CT8 kit (sand, stands, walls, gate, torches) in the window build and
  the llvmpipe twin instead of the cube placeholder. Tier A. Size M.
  Owns content-feed hunks beside `crates/render/src/present.rs` + the
  sky/IBL/fog binds (RA1/RL2/RL4/RL7 frozen). Accept: window + twin show
  sand, stands and fighters on the reference GPU with 0 validation
  errors; q1-arena120 sequence_fnv moves with motion, never 120
   identical bytes; llvmpipe parity within tolerance. needs: GX1, CT7,
   CT8, RA1, VS1, VS3, VS4. (PLN49 R1.; render-look.md Row changes, applied 2026-10-03: the arena draw list gets one owner, so RA14 closes on the VS rows as the integration gate.)
- [x] WIN6 game: staged fight to the presenter — the staged 600-tick
  receipt (27 hits, 3 knockdowns) drives arena fight frames with bodies
  visible and hits readable instead of noise tiles. Tier A. Size M.
  Owns capture-path hunks beside `crates/game/src/windowed.rs` (WIN1
  loop frozen). Accept: one staged 600-tick run emits 4+ frames with
   fighters visible; frame manifests differ across frames; CT7 SHAs
   pinned. needs: CT7, CT15, WIN1, RA1. (PLN49 R2.)
  (A7 split 2026-10-02: all four needs ticked, still stuck 3
  helpers/72h — receipt unowned per `docs/plan/render-look.md`.
  (Lead 2026-10-02: judge-PASS lane `win6-top` HELD, not landed — see
  `docs/BLOCKERS.md` WIN6 2026-10-02; lane kept for the 335 fix.)
  (Lead 2026-10-03: 335 ratchet-fix judged PASS 338; landing BLOCKED on
  patch context — in-flight staged `lib.rs` comment edit shifts the
  lane's `lib.rs:72-77` hunk (`does not match index`, transaction
  aborted, tree verified unchanged). Retry after the sync lands.)
  progress 2026-10-10: staged_fight writes the staged fight raster frames (fighters drawn, hits flashing) plus manifest.json; 5/5 staged_fight tests with --features app; release run 27 hits, 3 knockdowns, 4 frames 128x96; commit 3697e065. Default-feature cargo check still owed (heavy breaker open).
- [ ] CT29 content: window fighter-bind staging — CT7 hero plus two CT15
  staged fighters bound into the WIN1 window twin draw list via a new
  `content/fighters/window_bind.json` placement (no pass code), so the
  owner/player frame shows fighters instead of the lone twin cube. Tier A.
  Size M. Owns `content/fighters/window_bind.json` + window-bind glue
  (stone-slice files untouched). Accept: capture pair before
  `target/pilot-gx13/owner-arena/frame_000015.png` vs after on the
  reference GPU scores higher by this seat on silhouette + composition
  with 0 validation errors; golden unchanged. needs: CT7, CT15, WIN1, VS4.
  (CT29 deep-dive 2026-10-03: root cause — the row bundles a GPU-gated
  capture accept with an unowned data file, so 5 helpers in 72 h re-shot
  captures while `window_bind.json` stayed missing and no planner
  reframed it; different approach — land the data half first as CT29a
  (JSON + std-only pin loader, no GPU), keep the capture pair on CT29
  for the reference GPU; proof: `cargo test --locked -p engine-game
  --lib window_bind` 4/4 green plus `fighter_bind` green, golden
  unchanged.)
- [ ] UI3 ui: HUD overlay model — names, HP pips, tick and hit flash as
  data with x2-identical bytes (landed `e76c0f2`, never boarded). Tier A.
  Size XS. Owns `crates/ui/src/lib.rs` overlay hunks (encode/decode/tests
  only, no endpoint; UI2 frozen). Accept: bytes round-trip; seeded bytes
  identical twice; in-crate FrozenFrame raster shows names/HP/tick/flash.
  needs: UI2. (PLN48 orphan verdict; checker wave-48: stub + zero-caller
  FAIL held — UI4 lands the WIN1 consumer; wave-14 replan.)
- [ ] UI4 ui: HUD truth wiring — hud_cells/hud_quads in the smoke log
  match on-screen cells/quads, plus HP, facing wedges and 20 m bounds
  overlay on play-step captures. Tier A. Size S. Owns wiring hunks
  beside `crates/ui/src/` overlay + WIN1 HUD call sites (UI2 frozen;
  lands the WIN1 consumer UI3's tick requires). Accept: log counts equal
  painted counts on twin + play-step PNGs; HP/facing/bounds readable;
  bytes round-trip x2. needs: UI2, UI3, WIN1. (PLN49 R5; checker wave-48:
  tick only with WIN1 consumer wired and stub removed.)
- [ ] UI5 ui: play HUD — `play.step` PNGs show HP bars + names + button
  echo + tick + fighter-you arrow; 4F palette passes a colorblind check.
  Tier A. Size S. Owns HUD hunks beside `crates/ui/` (UI4 wiring
  frozen). Accept: seed95 PNGs carry bars/names/echo/tick/arrow;
  palette check green. needs: UI1, IN1. (PLAYTEST56 B6/C1 wave 56.)
- [ ] SN10 audio: hit-sync numbers in the logs — per-hit loudness,
  clipping flag and hit-sync delta ride the smoke/staged logs over the
  PQ8 coherence bridge. Tier B. Size S. Owns log/bridge hunks beside
  `crates/audio/src/coherence.rs` + game call sites (PQ8 model frozen).
  Accept: 10 scenarios synced with loudness/clipping/delta per hit in
  the log; 0 audio-thread allocs; live 2ch 48 kHz numbers posted.
  needs: PQ8, AU1. (PLN49 R7.)
  LATER: parked 2026-10-03 (circus verdict 2: FOCUS.md parks audio, UI, modding, platform and shipping, living world, and no open finish bar lists this row); unpark by deleting this line.
- [ ] Q1 proof: V1–V3 reference captures and a perf report on the reference
  GPU (reference GPU, HELD until HDRI + binds tick - pilot only, no lane).
  Size M. Owns evidence only under `target/q1/` (reuse
  `target/pilot-gx13/` + `target/showcase/`; no `crates/` hunks).
  Accept: HDRI vendored (`content/incoming/studio_small_09_1k.hdr` on disk
  or RL2 fallback landed) AND release pilot `--scene arena --fighters 8
  --camera-path arena600 --frames 120 --exit` posts V1 hero plus V2
  arena-day plus V3 fight action (3 distinct PNGs, RA8 manifests, PF2
  per-pass ms quiet-window, VRAM/draw counts); placeholder refusal (no
  64x48 noise, no single-cube sha); `node tools/proof.mjs` PASS golden
  `d57b5c8ae3096614` x2; `node tools/finish.mjs` P1/A4 moving. needs:
  RL1, RL2, RP1, WIN1, CT7, CT8, RL21, RL26, RA14, WIN6, GX13, PF2.
  (Owner 2026-10-10: RL11 dropped from needs; RL11 is driven by Q1.)
  (Q1 deep-dive 2026-10-03: board needs understated true prerequisites
  while the release build fails tree-wide on the absent HDRI; Q2 takes
  the headless numeric half now, Q1 keeps the visual half pilot-only
  HELD; proof: HDRI gate build green, then release pilot V1-V3.)
- [ ] Q2 proof: Q1 numeric receipt headless - refresh the Q1 evidence-only
  receipt with zero reference-GPU pixels. Tier A. Size S. Owns evidence
  only under `target/q1/` (no `crates/` hunks, no `content/` writes).
  Accept: `node tools/proof.mjs --skip-build` PASS golden
  `d57b5c8ae3096614` x2; `node tools/finish.mjs` recomputed with A4/P1
  lines quoted; placeholder refusal re-verified (V1 64x48 noise sha
  refused, V2/V3 single-cube sha refused, zero reference-GPU captures
  claimed); headless `perf-report.txt` refreshed, GPU lines PENDING.
  needs: WIN1, PF2. (Q1 deep-dive 2026-10-03.)
- [ ] CT12 assets: bake manifest — every cook and bake writes a seeded manifest
  of inputs, tool versions and outputs, so only what changed rebuilds and
  every output has a receipt for DT5 and PR1 (Snakemake discipline, MIT, no
  runtime). Tier B. Size S. Owns `crates/assets/src/manifest.rs` (new file
  beside frozen `cooker.rs` + `registry.rs` + `bundle_cook.rs` + `bundle.rs`).
  Accept: 20 of 20 fixture rebuilds byte-equal (tiny `demo_staged` bundle,
  thread count 1, relative paths, versions posted); 10 of 10 edits invalidate
  exactly the transitive closure (registry/cooker definition, last-good kept);
  seeded manifest round-trips source + settings + importer version + target
  profile to artifact + blake3; receipts feed DT5/PR1 (no DT5/PR1 hunks here).
  Proof: `cargo test --locked -p engine-assets manifest` green.
  needs: CT2. (CT12 deep-dive 2026-10-03: root cause — board accept
  truncated with zero deps while the cache + closure + byte-identical
  pieces already hold and the fixture scope stayed unbounded, so 3
  helpers in 72 h re-proved the same walls; different approach — one
  `manifest.rs` beside frozen owners, fixture-scoped 20/20 + closure-exact
  10/10, receipts feed DT5/PR1; proof: manifest suite green +
  workspace-check board-valid + proof golden x2.)
  LATER: parked 2026-10-03 (circus verdict 2: bake manifest for a cook pipeline no product binary calls (engine-assets is outside the arena window, e2040-mcp and the editor), consumer DT5 is open and off the duel; stuck 3 helpers and 3 deep-dives; unpark once BV1 or VS1 loads assets in a game binary); unpark by deleting this line.
- [ ] WIN10 game: arena CLI surface — `--help`, `--version`, `--mode`,
  `--list-modes` parsed as data before the winit loop; unknown flags
  exit non-zero with usage, never a silent no-op. Tier B. Size S.
  Owns arg hunks beside `crates/game/src/bin/arena.rs` (WIN9 exit path
  untouched). Accept: argv in, exit code out, loop never entered; mode
  list equals the WIN2 adapter list; replay x2. Proof: `cargo test -p
  engine-game --lib cli` green. needs: WIN1, WIN2.
  (PLN render-look.md EXTEND, proposed 2026-09-29.)
  LATER: parked 2026-10-03 (circus verdict 2: docs/plan/render-look.md parks shipping (rows stay, no writers unless a bar needs them) and only WIN9 is the bar's exception; this window polish changes no pixel, hash or timing); unpark by deleting this line.
- [ ] WIN9 game: arena --exit watchdog — the exe self-exits within 60 s
  of the last frame; no process outlives its shell timeout + grace.
  Tier A. Size S. Owns watchdog hunks beside `crates/game/src/bin/arena.rs`
  (exit path only). Accept: forced-stall drill with `--deadline-ms 60_000`
  exits ≤60 s with `Timeout` receipt naming elapsed+limit; default bound stays
  300 s so the normal 30-frame `--exit` run is unaffected (exit 0).
  Proof: `cargo test --locked -p engine-game --lib windowed` green (named
  deadline tests, replay x2; `--bin arena` proof allowed, the studio HDRI
  is on disk since 2026-10-03).
  needs: WIN1. (WIN9 deep-dive 2026-10-03: root cause — board accept truncated
  with zero deps while the tree lands a 300-s since-start bound and the bin
  proof is HDRI-blocked second-identical, so 3 helpers in 72 h re-proved the
  same walls; different approach — 60-s drill via the landed `--deadline-ms`
  override beside frozen exit path, lib proof only; proof: lib windowed green
  + workspace-check board-valid + proof golden x2.)
- [ ] WIN11 game: minimize/restore resilience — suspend mid-fight pauses
  present, restore resumes the same tick with last-good mode within one
  frame, never a black window or replay drift. Tier B. Size S. Owns
  suspend hunks beside `crates/game/src/windowed.rs` (WIN1 loop frozen;
  reuses the WIN8 drill shape one layer up). Accept: scripted
  suspend/resume keeps the tick and restores last-good within one
  frame; replay x2. Proof: `cargo test -p engine-game --lib windowed`
  green. needs: WIN1, WIN2.
  (PLN render-look.md EXTEND, proposed 2026-09-29.)
  LATER: parked 2026-10-03 (circus verdict 2: docs/plan/render-look.md parks shipping (rows stay, no writers unless a bar needs them) and only WIN9 is the bar's exception; this window polish changes no pixel, hash or timing); unpark by deleting this line.
