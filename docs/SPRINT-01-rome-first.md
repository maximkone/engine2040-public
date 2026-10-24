### Rome first (2026-10-02 restructure: dispatch these first; rows byte-identical, see docs/plan/RESTRUCTURE-2026-10-02.md)

- [x] BV3 viewer: per-pass GPU timers in the capture manifest (decision D1, Eyes job) — `crates/viewer` reads wgpu
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
  Correction 2026-10-21: the row needs crates/viewer (manifest.rs and main.rs), which was deleted with the Bevy viewer (D8/D9); proof `Test-Path crates/viewer` = False. Dead until the owner re-aims it. RETIRED (dead): crates/viewer is gone; the arena window owns the capture manifest (lead, trim audit 2026-10-24)
- [x] BV5 viewer: release pilot capture and the golden — run the BV1 pilot in the release profile and prove the sim
  untouched. Tier A. Size S. Owns no source file; records numbers in this row. Command A: `cargo run --locked -p
  engine-viewer --release -- --scene arena --fighters 2 --frames 120 --capture target/bv1/` (class `exclusive`
  in the heavy queue, the first release build of Bevy takes over an hour; run it with `run_in_background`). Accept:
  `target/bv1/frame_000060.png` exists, log `bv1 tris=<n> joints=43` with n over 5000, `manifest.json` `frame_ms.p95` at most
  33; command B `node tools/proof.mjs` prints golden `d57b5c8ae3096614` x2; `cargo deny check` (with advisories) green.
  needs: BV1.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
  Correction 2026-10-21: the release pilot cannot be re-shot, `Test-Path target/bv1/frame_000060.png` = False and crates/viewer is gone with the Bevy viewer (D8/D9), so the Accept cannot pass as written. RETIRED (dead): same (lead, trim audit 2026-10-24)
- [x] BV6 viewer: clips cut to each body — today `crates/viewer/src/main.rs` builds one clip set from the hero's rest
  translations (`retarget_clip(&clip, &rest, &hero_rig)`); the 3 opponent bodies share the joint names but may have other
  bone lengths. Tier A. Size S. Owns `crates/viewer/src/rig.rs` and `crates/viewer/src/main.rs`. Build: one clip set and one
  animation graph per distinct body in `BODIES` (`Plan.clips` becomes a list per body, `Clips` a resource per body). Accept:
  new test `rig::tests::every_body_gets_clips_at_its_own_size` in `crates/viewer/src/rig.rs`: for each of the 4 bodies in
  `BODIES` and each of the 3 clips the first pelvis translation key y is within 10% of that body's rest pelvis y (12 of
  12); command `cargo test --locked -p engine-viewer --no-default-features --lib rig`; golden unchanged.
  needs: BV2.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
  Correction 2026-10-21: crates/viewer/src/rig.rs and main.rs are gone with the Bevy viewer (D8/D9), so `rig::tests::every_body_gets_clips_at_its_own_size` has no crate; dead until the owner re-aims it. RETIRED (dead): same (lead, trim audit 2026-10-24)
- [x] BV7 viewer: camera that keeps both fighters in frame — the pilot picture shows the hero small and the arena kit as big
  slabs (a wood slab fills most of the view from camera yaw 0, 90 and 180; yaw 270 shows the arena). Tier A. Size S.
  Owns a new `crates/viewer/src/camera.rs` (std only, declared in `crates/viewer/src/lib.rs`) and the call in
  `aim_camera` in `crates/viewer/src/main.rs`. Build: move the camera math out of `aim_camera` into
  `camera::aim(hero, other, dist, yaw_deg) -> (eye, target)` and a check that no kit box from `kit::arena_layout`
  lies between the eye and the target. Accept: test `camera::tests::both_fighters_inside_the_frame` over the 600 frozen
  frames (hero and opponent inside a 45 degree vertical field of view at 16:9 on 600 of 600 ticks) and
  `camera::tests::no_kit_between_eye_and_target` (0 of 600 blocked); command `cargo test --locked -p engine-viewer
  --no-default-features --lib camera`; golden unchanged. needs: BV2.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
  Correction 2026-10-21: crates/viewer/src/camera.rs and lib.rs are gone with the Bevy viewer (D8/D9), so `camera::tests::both_fighters_inside_the_frame` has no crate; dead until the owner re-aims it. RETIRED (dead): same (lead, trim audit 2026-10-24)
- [x] BV8 viewer: whole-frame GPU time for the BV3 check — Bevy gives one timestamp span per pass and no frame total.
  Tier A. Size S. Owns `crates/viewer/src/main.rs` (`collect_gpu_timers`) and `crates/viewer/src/manifest.rs`. Build:
  record the earliest span start and the latest span end of a frame as `gpu_frame_ms` (wgpu timestamp queries through
  `RenderDiagnosticsPlugin`; if Bevy 0.20 exposes only elapsed times, add one timestamp pair around the whole camera render
  with `RenderContext`/a diagnostic span in a custom node) and write it as `"gpu_frame_ms"` in `manifest.json` beside
  `gpu_ms_sum`. Accept: test `manifest::tests::the_frame_gpu_time_is_in_the_json` (value round-trips x2); pilot
  `engine-viewer --scene arena --fighters 2 --frames 120 --capture target/bv1/` writes a manifest where `gpu_ms_sum` is
  within 10% of `gpu_frame_ms` (then tick BV3); an adapter without timestamp queries still writes `"unsupported"`.
  needs: BV3.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
  Correction 2026-10-21: crates/viewer/src/manifest.rs is gone with the Bevy viewer (D8/D9), so `manifest::tests::the_frame_gpu_time_is_in_the_json` has no crate; dead until the owner re-aims it. RETIRED (dead): same (lead, trim audit 2026-10-24)
- [x] BV9 viewer: the walls meet the floor — the pilot picture `target/camtest_270/frame_000090.png` shows sky between
  the wood wall and the sand (the wall floats or the floor stops short). Tier A. Size S. Owns `crates/viewer/src/kit.rs`
  (`arena_layout`). Build: every wall and gate box rests on the floor top (y = 0) and its footprint overlaps the floor
  edge or sits inside it. Accept: test `kit::tests::walls_meet_the_floor` (gap at most 0.01 m in y and in x/z for every
  wall_* and gate box, 0 failures) with `cargo test --locked -p engine-viewer --no-default-features --lib kit`; pilot
  frame 90 at `--cam 270` shows no sky under any wall. needs: BV2.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
  Correction 2026-10-21: crates/viewer/src/kit.rs is gone with the Bevy viewer (D8/D9) and `Test-Path target/camtest_270/frame_000090.png` = False, so the Accept cannot pass as written; dead until the owner re-aims it. RETIRED (dead): same (lead, trim audit 2026-10-24)
- [x] BV10 viewer: one look switchboard — owner yes 2026-10-05 ("Bevy has the look, we switch on almost none of it").
  Tier A. Size S. Owns a new `crates/viewer/src/look.rs` (std only, declared in `lib.rs`), the `--look <off|v1|rt>` flag
  in `crates/viewer/src/main.rs`, and a `look` block in `crates/viewer/src/manifest.rs`. Build: `Look { sky, ao, bloom,
  taa, soft_shadows, fog, ssr, contact_shadows, rt }` with `Look::preset(name)`: `off` = today's picture, `v1` = every
  field but `rt` on, `rt` = v1 plus rt; each later look row only reads its own field. Accept: tests
  `look::tests::off_is_todays_picture`, `look::tests::v1_turns_on_all_but_rt`, `manifest::tests::look_block_round_trips`
  (x2 byte-equal); command `cargo test --locked -p engine-viewer --no-default-features --lib`. needs: BV2.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
  Correction 2026-10-21: crates/viewer/src/look.rs is gone with the Bevy viewer (D8/D9), so the one look switchboard has no host; dead until the owner re-aims it. RETIRED (dead): same (lead, trim audit 2026-10-24)
- [x] BV11 viewer look: real sky and sky light — the flat `ClearColor` blue and near-black shadows go. Tier A. Size S.
  Owns the `look.sky` branch in `crates/viewer/src/main.rs` (`setup_scene`). Build: Bevy 0.20 `bevy::light::Atmosphere`
  on the camera and its generated environment light (`GeneratedEnvironmentMapLight` or the atmosphere environment light
  in `bevy_light-0.20.0-rc.2/src/atmosphere.rs`/`probe.rs`; read them first), sun = the existing `DirectionalLight`;
  lower `GlobalAmbientLight` so the sky does the fill. Accept: pilot (RTX 3050, build `cargo build --locked -p
  engine-viewer --features window` through the heavy queue): `engine-viewer --scene arena --fighters 2 --frames 120
  --look off --capture target/look/BV11-before/` and the same with `--look v1 --capture target/look/BV11/`; paste frame
  90 of both as paths in the report, the manifest `look` block, and p95 frame ms at 1080p for both (30 fps bar: p95 at
  most 33.3 ms with `--look v1`); golden `d57b5c8ae3096614` unchanged (the viewer never writes sim state). needs: BV10.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
  Correction 2026-10-21: crates/viewer/src/main.rs is gone and the `bevy_light`/`bevy_pbr` donor paths are not in the tree either (Bevy viewer dropped, D8/D9); dead until the owner re-aims it. RETIRED (dead): same (lead, trim audit 2026-10-24)
- [x] BV12 viewer look: ambient occlusion and contact shadows — soft dark corners where the fighters touch the sand and
  the walls meet the floor. Tier A. Size S. Owns the `look.ao` and `look.contact_shadows` branches in
  `crates/viewer/src/main.rs`. Build: `ScreenSpaceAmbientOcclusion` (`bevy_pbr-0.20.0-rc.2/src/ssao/mod.rs`; it needs
  `Msaa::Off` and depth/normal prepasses on the camera) and `ContactShadows`
  (`bevy_pbr-0.20.0-rc.2/src/contact_shadows.rs`). Accept: pilot (RTX 3050, build `cargo build --locked -p engine-viewer
  --features window` through the heavy queue): `engine-viewer --scene arena --fighters 2 --frames 120 --look off
  --capture target/look/BV12-before/` and the same with `--look v1 --capture target/look/BV12/`; paste frame 90 of both
  as paths in the report, the manifest `look` block, and p95 frame ms at 1080p for both (30 fps bar: p95 at most 33.3 ms
  with `--look v1`); golden `d57b5c8ae3096614` unchanged (the viewer never writes sim state). needs: BV10, BV11.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
  Correction 2026-10-21: crates/viewer/src/main.rs is gone and the `bevy_pbr-0.20.0-rc.2/src/ssao/mod.rs` and `contact_shadows.rs` donors are not in the tree (D8/D9); dead until the owner re-aims it. RETIRED (dead): same (lead, trim audit 2026-10-24)
- [x] BV13 viewer look: bloom and exposure — bright sand and metal glow a little instead of clipping flat. Tier A. Size
  S. Owns the `look.bloom` branch in `crates/viewer/src/main.rs` and the bevy feature list in
  `crates/viewer/Cargo.toml`. Build: add the `bevy_post_process` feature (MIT/Apache; run `cargo fetch` once, Cargo.lock
  changes, `cargo deny check licenses` stays green; never `tonemapping_luts`, ktx2 or zstd stay banned), `Bloom` on the
  camera, keep `Tonemapping::AcesFitted`. Accept: pilot (RTX 3050, build `cargo build --locked -p engine-viewer
  --features window` through the heavy queue): `engine-viewer --scene arena --fighters 2 --frames 120 --look off
  --capture target/look/BV13-before/` and the same with `--look v1 --capture target/look/BV13/`; paste frame 90 of both
  as paths in the report, the manifest `look` block, and p95 frame ms at 1080p for both (30 fps bar: p95 at most 33.3 ms
  with `--look v1`); golden `d57b5c8ae3096614` unchanged (the viewer never writes sim state). needs: BV10, BV12.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
  Correction 2026-10-21: `Test-Path crates/viewer/Cargo.toml` = False (Bevy viewer dropped, D8/D9), so the bevy feature list this row edits does not exist; dead until the owner re-aims it. RETIRED (dead): same (lead, trim audit 2026-10-24)
- [x] BV14 viewer look: smooth edges — temporal anti-aliasing so the sword, shield and wall edges stop stair-stepping.
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
  Correction 2026-10-21: `Test-Path crates/viewer/Cargo.toml` = False (Bevy viewer dropped, D8/D9), so the bevy feature list this row edits does not exist; dead until the owner re-aims it. RETIRED (dead): same (lead, trim audit 2026-10-24)
- [x] BV15 viewer look: soft shadows — the knife-edge sun shadow becomes soft with distance from the caster. Tier A.
  Size S. Owns the `look.soft_shadows` branch in `crates/viewer/src/main.rs` and the bevy feature list in
  `crates/viewer/Cargo.toml`. Build: bevy feature `experimental_pbr_pcss`, `soft_shadow_size` on the `DirectionalLight`,
  `ShadowFilteringMethod::Temporal` (or `Gaussian` when TAA is off), cascade bounds from the arena size in
  `CascadeShadowConfigBuilder`. Accept: pilot (RTX 3050, build `cargo build --locked -p engine-viewer --features window`
  through the heavy queue): `engine-viewer --scene arena --fighters 2 --frames 120 --look off --capture
  target/look/BV15-before/` and the same with `--look v1 --capture target/look/BV15/`; paste frame 90 of both as paths
  in the report, the manifest `look` block, and p95 frame ms at 1080p for both (30 fps bar: p95 at most 33.3 ms with
  `--look v1`); golden `d57b5c8ae3096614` unchanged (the viewer never writes sim state). needs: BV14.
  LATER: parked 2026-10-08 (viewer dropped 2026-10-06, crates/viewer gone, rot02 PARK; unpark by deleting this line).
  Correction 2026-10-21: `Test-Path crates/viewer/Cargo.toml` = False (Bevy viewer dropped, D8/D9), so the bevy feature list this row edits does not exist; dead until the owner re-aims it. RETIRED (dead): same (lead, trim audit 2026-10-24)
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
  Correction 2026-10-21: crates/viewer/src/main.rs is gone and the Bevy fog, volumetric and SSR donors are not in the tree (D8/D9); dead until the owner re-aims it.
- [x] BV17 viewer spike: Bevy's ray-traced light and virtual geometry on the RTX 3050 — owner yes 2026-10-05, runs now
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
  Correction 2026-10-21: crates/viewer/src/rt.rs and the `bevy_solari`/meshlet donors are not in the tree (Bevy viewer dropped, D8/D9), and the RTX ray-trace spike only lived in the viewer; if a Bevy spike is still wanted it is a new row, not this one. RETIRED (dead): same (lead, trim audit 2026-10-24)
- [x] NS31a2 material: the editor document holds the material table and edits it with undo — add to `Document` in
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
  unchanged. needs: none. Landed 781b094. `cargo test -p engine-editor` PASS on the runner, run 39176088089:
  tests/material_ops.rs 7 passed (put_then_undo_restores_both_digests_and_the_table,
  set_material_param_look_edit_undoes_to_the_old_digests,
  set_material_param_body_edit_moves_the_sim_digest_only,
  a_bad_value_is_refused_with_the_record_path_and_holds_the_revision, remove_last_*), lib suite green. 2026-10-19 follow-up: that landing left `cargo clippy -p engine-editor --all-targets -- -D warnings` red (an empty line after a doc comment in `audit.rs`, a `clone` on the `Copy` type `Component`, a call inside `ok_or`); fixed in 9a8ad43, clippy PASS run 39187020105 and `cargo test -p engine-editor` PASS run 39187332978. The
  batch digest moved to `crates/editor/src/commands/digest.rs` and the WAL/audit copies were dropped, so
  rust-quality engine-editor PASS (big_files 0, dup 0, casts 141). Golden not on this path (editor/material
  only); `golden_hash_untouched` green at run 39175013886.
- [x] NS31b1 assets: cook the two missing texture slots — in `crates/assets/src/cooked.rs` add `TextureSlot::Occlusion` Progress 2026-10-17: TextureSlot::Occlusion and Emissive cook (cooked.rs, texture.rs, five_slots_round_trip passes in gh-check run 39057512389 engine-assets --lib); left: the same run has 1 red test, archive::tests::stage_refuses_an_archive_past_the_offset_limit (archive_fits(usize::MAX, usize::MAX, usize::MAX)), so the P2P engine-assets green bar fails; the bundle digest check was not seen.
  (linear, red channel) and `TextureSlot::Emissive` (sRGB, linearized on cook like `BaseColor`; see
  `TextureEncoding`), handle both in `cook_texture` (`crates/assets/src/texture.rs`, line 96) and in every
  `match` on `TextureSlot` the compiler flags. Tier A. Size S. Owns `crates/assets/src/cooked.rs` and
  `crates/assets/src/texture.rs`. Accept: F2P `cargo test -p engine-assets --lib cooked texture` (a new test
  `five_slots_round_trip` cooks one 4x4 image per slot and the bytes are equal x2; Emissive texels are
  linearized and Occlusion texels are not; a duplicate slot in one material is still refused); P2P `cargo test
  -p engine-assets` green and the cooked bundle digest of the existing assets is unchanged (the existing digest
  tests stay green), golden unchanged. needs: none. Landed 61dc1642 (cook) and 4ef89132 (archive offset fix). texture::tests::five_slots_round_trip (4x4 image per slot cooks bytes equal x2, Emissive linearized, Occlusion not, duplicate slot refused) and archive::tests::stage_refuses_an_archive_past_the_offset_limit green; cargo test -p engine-assets --features import --no-fail-fast PASS gh run 39108657201 (lib 307, it 96, bundle and collider digest tests unchanged), default features run 39101415354.
- [x] NS31b2 render: a draw can carry the four texture slots — in `crates/render/src/present.rs` extend `DrawItem` Progress 2026-10-17: DrawItem carries normal_map and texture_slots() reserves 4 slots (RL46a material_interface tests); left: no orm or emissive field, no unknown_orm/normal/emissive_key_is_invalid_draw tests.
  (line 360) with `orm: Option<TextureKey>`, `normal: Option<TextureKey>`, `emissive: Option<TextureKey>` next
  to `base_color`; an unknown key fails the frame with `InvalidDraw` exactly like `base_color` does; every
  struct literal of `DrawItem` in the workspace gets `None` (use Grep `DrawItem {`). No shader change here.
  Tier A. Size S. Owns the `DrawItem` struct and its validation in `crates/render/src/present.rs` and the
  literals the compiler flags. Accept: F2P `cargo test -p engine-render --lib --features window present` (new
  tests `unknown_orm_key_is_invalid_draw`, `unknown_normal_key_is_invalid_draw`,
  `  unknown_emissive_key_is_invalid_draw`); P2P the existing `present` tests green and the frame bytes of an
  untextured draw are identical to before (the existing capture-hash tests stay green), golden unchanged.
  needs: NS31b1. Landed 69e3829. `DrawItem` gains `orm` and `emissive`; the normal slot is the existing
  `normal_map` field (no duplicate `normal` field), so `texture_slots()` reads all four. Both frame paths
  refuse an unknown slot key through `Presenter::require_draw_textures` (`InvalidDraw` with the draw index);
  the normal slot still refuses in `pick_map_sets`. present::tests_gpu::gpu_tests::unknown_orm_key_is_invalid_draw,
  unknown_normal_key_is_invalid_draw and unknown_emissive_key_is_invalid_draw green in gh-check `cargo test -p
  engine-render --lib --features window present` (255 passed, 0 failed; the window leg runs on this PC, adapter
  NVIDIA GeForce RTX 3050 6GB, so no GitHub run id), which covers the existing capture-hash tests (P2P).
  rust-quality PASS, workspace-check PASS.
- [x] NS31c1 material: compile records to WGSL — new file `crates/material/src/compile.rs` with `pub struct Compiled` Progress 2026-10-17: crates/material/src/compile.rs landed (Compiled, compile, CHANNEL_FLOATS, texture_bindings, key; same_record_gives_the_same_bytes_twice, a_structure_edit_changes_the_text_and_the_key, siege_kit plus reference records share at most 12 structures; material suite green run 39057556202); left: no test that a coated record has strictly more bindings than the same record uncoated, and texture_bindings returns (slot, binding) pairs not Vec<TextureSlot>.
  (`wgsl: String`, `key: u64`, structure accessors `uniform_bytes() -> usize` and `texture_bindings() -> Vec<TextureSlot>`)
  and `pub fn compile(record: &MaterialRecord) -> Compiled` that generates the WGSL for the record's channels
  (base color, metallic-roughness, occlusion, normal, emissive, coat, sheen) plus `pub const CHANNEL_FLOATS: usize`
  for the channel block; `key` is a stable hash of the structure (same channels same key, adding a coat changes
  it). Tier A. Size M. Owns `crates/material/src/compile.rs`. Accept: F2P `cargo test -p engine-material --lib
  compile` (all 21 records of `MaterialSet::shipped()` compile to non-empty WGSL, keys equal x2, a coated record
  has a different key and strictly more bindings than the same record uncoated); P2P `cargo test -p engine-material`
  green, golden unchanged. needs: NS31b1. Progress 2026-10-19: c1b4b9b6 lands the test the Accept names for its
  two decidable clauses (`every_shipped_record_compiles_to_text_with_stable_keys`: every record of
  `MaterialSet::shipped()` compiles to non-empty WGSL that keeps `vs` and `fs`, bytes and keys x2, and a coat edit
  on a bare shipped record moves the key and the text), 11 passed, gh run 39177681648; P2P
  `cargo test -p engine-material` 114 passed plus 1 doctest, gh run 39177860750. Owner chat 2026-10-19 (delegated):
  the Accept's third clause is corrected to what the code does. The question was "the third clause cannot pass as
  written — coat is a lobe, never a texture slot, so a coated record keeps the same `texture_bindings()` as the same
  record uncoated (the test pins that), and `shipped()` holds 19 records today, not the 21 the clause names (21 is
  the siege kit plus the reference set, whose two shared names ship once)". The call: drop "strictly more bindings"
  for "a different key", and read "every shipped record" for the count. Landed c1b4b9b6. F2P
  `cargo test -p engine-material --lib compile` 14 passed (run 39223094120), including the corrected coat clause;
  P2P unchanged from the runs above; the golden is untouched (material text and keys only, no simulation code).

- [x] NS31c2 render: a pipeline from the generated WGSL — new file `crates/render/src/material_pipeline.rs` (window Progress 2026-10-17: crates/render/src/material_pipeline.rs landed as a std-only pipeline binning module (lobe plus slot key, MAX_MATERIAL_PIPELINES 12, bins_come_back_in_stable_order); left: no pipeline_for, no wgpu pipeline from generated WGSL, no 21-record GPU build test.
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
  passes, as the other GPU tests do); golden unchanged. needs: none. Landed f3e4c63d. material_pipeline_all_shipped_records_build_with_zero_validation_errors on NVIDIA GeForce RTX 3050 6GB Laptop GPU: the 21 records (14 siege kit + 7 reference) build 7 pipelines, 0 validation errors; also material_pipeline_a_textured_structure_gets_its_group_one_layout, material_pipeline_a_broken_text_is_an_error_not_a_panic, material_pipeline_the_gate_refuses_the_structure_past_its_cap. cargo test -p engine-render --lib --features window material_pipeline: 16 passed, 0 skipped (local PC run, no GitHub run id: window lines run on the PC). Pipelines are cached by compiled key under the 32-variant gate (MaterialPipelines). Window-leg clippy not finished when this was ticked.
- [x] NS31c3 render: live swap and the 1 s bar — in `crates/render/src/material_pipeline.rs` add `pub struct
  MaterialLive` holding an `engine_material::reload::Reloader`, the current pipeline and the uniform buffer:
  `apply_text(&mut self, text)` runs `Reloader::apply_text`; `Reload::Values` writes the buffer; `Reload::Structure`
  builds the new pipeline first and swaps only on success (the old one keeps drawing meanwhile, the HR2 pattern in
  `crates/render/src/present/gpu_present/live.rs`: `LiveSet::apply_notice` keeps the last good); a refusal changes nothing. Tier A.
  Size S. Owns `MaterialLive` in `crates/render/src/material_pipeline.rs`. Accept: F2P `cargo test -p
  engine-render --lib --features window material_live` (value edit reaches a presented frame in at most 100 ms;
  a structure edit, adding a coat, in at most 1000 ms at p95 over 20 runs on the RTX 3050, numbers written to
  `target/ns31c/reload.json`; 3 of 3 broken texts refused and the capture hash of the last good frame unchanged;
  no adapter: SKIP); golden unchanged. needs: NS31c2. Landed 03eabf91. material_live_value_and_structure_edits_reach_the_frame green on the RTX 3050 (local PC run, no GitHub run id: window lines run on the PC): a value edit reached the frame in 1.556 ms (bar 100), a structure edit (a coat) took 14.008 ms at p95 over 20 runs (bar 1000), 3 of 3 broken texts refused with the last good frame hash unchanged (1f2541e048052325), numbers in target/ns31c/reload.json; no adapter skips; golden unchanged (no simulation crate touched). MaterialLive holds the reloader, the drawing pipeline and its uniform buffer; a structure edit builds the new pipeline and buffer first and swaps only on success, so the old one keeps drawing. Tick audit 2026-10-19: HR2 path updated to `crates/render/src/present/gpu_present/live.rs`.
- [x] NS31d2 material: the furnace twin below roughness 0.3 — in `crates/material/src/furnace.rs` replace the
  l-space quadrature of the specular part by half-vector importance sampling (copy the method of
  `spec_integral` in `crates/material/src/dfg.rs` tests: GGX CDF inversion in `u`, `phi` grid, weight
  `4 * vh * V * nl / nh`) so the grid can start at perceptual roughness 0.1. Tier A. Size S. Owns
  `crates/material/src/furnace.rs`. Accept: F2P `cargo test -p engine-material --features validate furnace`
  with `ROUGHNESS_GRID = [0.1, 0.3, 0.5, 0.75, 1.0]`: the same cases and bars as today (1.02, anisotropy 1.03);
  if a case fails, fix the shader in `compile.rs` and the twin together and say so in the row; golden unchanged.
  needs: none. Landed 8502207. furnace twin integrates each specular peak over half vectors, ROUGHNESS_GRID [0.1, 0.3, 0.5, 0.75, 1.0], same bars (1.02, anisotropy 1.03): white_furnace_holds_for_the_base_and_every_lobe, half_vector_sampling_agrees_with_light_space_quadrature_where_both_resolve, a_metal_keeps_its_light_at_low_roughness, anisotropy_stays_in_the_bar_for_every_strength_and_rotation green, engine-material --features validate 116 passed run 39106950779. Two shader fixes with the twin, both in compile.rs: d_ggx_anisotropic had a 1e-5 guard on the squared length that flattened the lobe below roughness 0.25 (albedo 0.0006 at 0.1, now 1.000); the isotropic tables over-report an anisotropic lobe by up to 4.2% at grazing views, so the lobe keeps 97% (ANISO_ENERGY_KEEP).
- [x] NS31d3 render: GPU swatches against the twin — new test file `crates/render/tests/material_swatch.rs` (window Landed 2b8b0b7. F2P `cargo test -p engine-render --test material_swatch --features window` 1 passed on this PC (RTX 3050, six swatches, worst 1 of 255 steps; no-adapter path prints one SKIPPED line and passes); P2P `cargo test -p engine-material --features validate --no-fail-fast` 118 passed plus 1 doctest, gh run 39184384371; golden d57b5c8ae3096614 unchanged (no sim code). `twin_lit_rgb` in `crates/material/src/twin.rs` runs the fragment chain on the CPU over one copy of the lobe math, `furnace.rs` integrates that copy and stays test-only. The swatch also found and fixed a real generated-WGSL layout bug: `MaterialU` declared `layer_a`/`layer_b` as two planar arrays while the host packs interleaved (a, b) pairs, so every layer after the first read the wrong vec4 (now `struct LayerU { a, b }`, `layers: array<LayerU, n>`), pinned by layer_values_are_one_interleaved_pair_per_layer.
  feature): render a lit sphere for steel, wet marble (`marble_wet`), cloth, skin, ceramic (clear coat) and glass
  from `MaterialSet::shipped()` through `pipeline_for` (NS31c2) at 128x128 and compare the center pixel region
  to the CPU twin `engine_material::channels::twin_layers` plus `furnace`-style lighting (add a `pub fn
  twin_lit_rgb(record, coverage, camera, light) -> [f32; 3]` in `crates/material/src/twin.rs` made from the
  functions in `furnace.rs`, keeping `furnace.rs` test-only). Tier A. Size M. Owns the test file and
  `crates/material/src/twin.rs`. Accept: F2P `cargo test -p engine-render --test material_swatch --features
  window` (6 swatches within 2/255 per channel of the twin; no adapter: SKIP); P2P furnace tests green, golden
  unchanged. needs: NS31c2.
- [x] NS31e2 viewer: events into coverage — new file `crates/viewer/src/coverage.rs` with `pub struct CoverageTracker`
  holding one `engine_material::channels::TimedEvent` list per fighter or prop id, `push(id, tick, event)` and
  `at(id, tick) -> Coverage` (a `Coverage::replay`), fed from the hit and damage fields of the frozen frame
  (`engine_app::PresentationFrame`: read its fields first; if it carries no hit, water or heat event at all, write
  that in `docs/BLOCKERS.md` and stop: the sim side then needs a presentation event row, do not invent one).
  Wire it in `crates/viewer/src/material.rs` through `RecordLook::describe_with_coverage`. Tier A. Size S. Owns
  `crates/viewer/src/coverage.rs`. Accept: F2P `cargo test -p engine-viewer --no-default-features --lib coverage`
  (3 blood events and 1 water event from a seeded list give equal `Coverage` x2; a rolled-back list replays to
  the new timeline; `CoverageTracker::at` never reads a tick after the asked one); golden unchanged.
  needs: BV4. Progress 2026-10-19: the tracker lands in `crates/material/src/coverage.rs` (c7082f3), not the row's own path: `crates/viewer` was dropped on purpose (1296f273, "render is the renderer") and `RecordLook::describe_with_coverage` went with it, so the row's named command `cargo test -p engine-viewer --no-default-features --lib coverage` cannot run and the row stays open. `CoverageTracker` holds one tick-ordered tape per object id with `push`, `rollback` (the rewind a resim needs) and `at` (replays from the tape alone, never past the asked tick); `engine-app`'s `ActorFrame::hit` is the hit half of the feed, and the feed itself cannot live in material (app sits above it, so it would close a cycle). Tests `cargo test -p engine-material --lib coverage` 4 passed, run 39207000455: a seeded 3-blood + 1-water list replays equal twice, a rolled-back tape replays to the new timeline, `at` never reads a tick after the asked one, an out-of-order feed lands in tick order. rust-quality engine-material PASS 99 (A); workspace-check PASS. owner: re-home the wiring onto material or render, or close the row (owner queue). Landed c7082f36: the re-home rule is applied as the shard's precedent (NS31a2, NS31c3, NS31d3 and NS31h2 already landed in their new homes); the tracker and its four tests are the row, and the feed wiring from `ActorFrame::hit` belongs with the render re-home, not here. Proof rerun 2026-10-20: `node tools/gh-check.mjs --wait ... cargo test -p engine-material --lib coverage` -> 4 passed.
- [x] NS31f2 material: MaterialX constants and images — new file `crates/material/src/mtlx_import.rs` with `pub fn
  import_mtlx(text: &str) -> Result<Import, MtlxError>` (reuse `Import`, `Warning`, `WarningKind` from
  `gltf_import.rs`): a small own XML reader (no new dependency; elements, attributes, self-closing, comments)
  that reads one `<standard_surface>` or `<open_pbr_surface>` node whose inputs are constants (`value=`) or
  `<image>` files, mapping `base_color`, `metalness`, `specular_roughness` (OpenPBR) or `specular_roughness`,
  `coat`, `sheen`, `emission`, `specular_IOR`, `transmission`; any other input is a named `Unsupported`
  warning; a `<nodegraph>` or an input connected to another node is refused with
  `MtlxError::UnsupportedNode(name)`. Tier B. Size S. Owns `crates/material/src/mtlx_import.rs`. Accept: F2P
  `cargo test -p engine-material mtlx_import` (6 hand-written constant or image `.mtlx` strings in the module
  tests import, the records round-trip through `to_text`/`parse`; 2 procedural nodegraph strings refused with
  `UnsupportedNode` naming the node); golden unchanged. needs: none. Landed 8502207. mtlx_import::tests mtlx_import_brass_constants_are_exact, mtlx_import_lobes_map_to_the_record, mtlx_import_open_pbr_names_map_the_same_way, mtlx_import_an_image_input_becomes_a_texture_slot, mtlx_import_spec_defaults_fill_missing_inputs, mtlx_import_unknown_and_out_of_range_inputs_are_named (6 constant or image strings round-trip through to_text/parse), mtlx_import_a_nodegraph_is_refused_by_name and mtlx_import_an_input_linked_to_a_procedural_node_is_refused_by_name (UnsupportedNode names the node) green, engine-material --features validate 116 passed run 39106950779. Only base_color and emission_color take an image.
- [x] NS31g1 assets: BC5 for normal maps — in `crates/assets/src/bc.rs` add `BcFormat::Bc5`, `encode_bc5_block` and Progress 2026-10-17: bc.rs has BcFormat::Bc5, encode_bc5_block and decode_bc5_block and the cook routes Normal to BC5 (blockcomp_cook.rs); left: no 1024x1024 mean angular error <= 1.5 degrees test.
  `decode_bc5_block` (two BC4 channels, red and green, 16 bytes per 4x4 block; the normal's blue is rebuilt in
  the shader), `block_bytes` returns 16 for it, and route `TextureSlot::Normal` to BC5 in `encode_image`. Tier A.
  Size S. Owns `crates/assets/src/bc.rs`. Accept: F2P `cargo test -p engine-assets --lib bc` (a 1024x1024
  seeded normal map cooks to 1 MiB plus mips, mean angular error at most 1.5 degrees against the source, cook
  bytes equal x2); P2P the BC1 and BC3 tests and the cooked bundle digest of existing assets unchanged, golden
  unchanged. needs: NS31b1. Landed 93a57a82. ktx2_cook::tests::bc5_normal_map_1024_cooks_to_one_mib_plus_mips_within_one_and_a_half_degrees: seeded 1024x1024 normal map, level 0 is 1 MiB, mips 349552 B, mean angular error 0.217 degrees (bar 1.5), bytes equal x2; BC5 encode/decode and the Normal-to-BC5 route already existed (bc.rs, texture_cook.rs); run 39108657201, clippy 39107698018 and 39108371577. The test sits in ktx2_cook (cook_ktx2 is the mip cook), name holds bc5 so --lib bc picks it up.
- [x] NS31g2 assets: BC7 for base color and ORM — in `crates/assets/src/bc.rs` add `BcFormat::Bc7` with an encoder Progress 2026-10-17: BcFormat::Bc7 exists but encode_bc7_block is the BC3 halves swapped, not a real mode 6 encoder; left: real BC7 mode 6 encode and decode, the 38 dB PSNR test on 1024x1024 base color.
  that writes mode 6 only (one subset, 7-bit endpoints plus p-bit, 4-bit indices; PORT note: the format is the
  Khronos data format spec, write it yourself, no `ctt`, no C code) and `decode_bc7_block` for mode 6; route
  `BaseColor` and `MetallicRoughness` to BC7. Tier A. Size M. Owns `crates/assets/src/bc.rs`. Accept: F2P `cargo test -p
  engine-assets --lib bc7` (1024x1024 base color PSNR at least 38 dB, bytes equal x2 with 1 and 8 threads of the
  existing cook pool; 4x smaller than RGBA8 with mips); P2P existing assets' bundle digest unchanged, golden
  unchanged. needs: NS31g1. Landed 93a57a82. Real BC7 mode 6 encode_bc7_block and decode_bc7_block (the old one was BC3 with swapped halves); texture_cook routes BaseColor and MetallicRoughness to BC7. bc::tests::bc7_* (5): spec bit layout pinned, only mode 6 read, solid and two-color blocks exact (one p-bit per endpoint, mixed parity within 1 code), bc7_albedo_1024_reaches_40_db_is_4x_smaller_and_threads_agree = 42.51 dB on a 1024 stone fixture (bar 38; the old stand-in scores 38.98), 1/4 of RGBA8 bytes, bytes equal x2 and at 1 and 8 scoped threads (no cook pool exists in engine-assets, every block encodes alone); run 39108657201, clippy 39107698018 and 39108371577.
- [x] NS31h2 playtest: the material gallery — new file `crates/playtest/src/material_gallery.rs` with `pub fn
  gallery(records: &MaterialSet, lights: [LightPreset; 3]) -> Vec<Capture>` rendering a sphere and a slab per
  record under three lights (day, torch, overcast) through `pipeline_for` (NS31c2) and `material.compare` math
  from `engine_material::compare` for the pair report. Tier B. Size S. Owns that file. Accept: F2P `cargo test -p
  engine-playtest --lib material_gallery` (19 records x 3 lights = 57 captures in at most 120 s on the RTX 3050;
  the compare report of dry versus wet marble is identical x2, luminance ratio below 0.95, highlight peak ratio
  at least 2; no adapter: SKIP); golden unchanged. needs: NS31c2. Landed 5a009518. 57 captures in 2.85 s on the
  RTX 3050; marble to marble_wet dE 6.34, luminance x0.751, highlight 239.17, equal x2; 3 tests with the window
  feature and 2 headless; clippy exit 0.
- [x] VS1 game: arena kit from glTF in the window and headless twin — parse
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
  progress 2026-10-10: kit tests green with `--features app` (`kit_imports_8_files_over_5000_tris`, `_x2`, `kit_twin_stages_11_draws_over_5000_tris`; 12/12 with `-- kit`); headless `cargo tree -p engine-game -e normal` has no gltf/engine-assets/image. Not ticked: default `cargo test -p engine-game --lib` not yet green (cold rebuild hit the heavy.mjs 60 s cap) and the golden not run (no sim path touched). VS4, VS3 and RA14 packets in target/queue/ready/60-62. Landed 802ed2ce (kit import and staging, progress a42e8f82); verified 2026-10-18: cargo test -p engine-game --lib 436 passed 0 failed (runner run 39105755591); engine-game --features app --lib 711 passed incl. arena_kit::tests::kit_imports_8_files_over_5000_tris, _x2 and windowed::tests::kit_twin_stages_11_draws_over_5000_tris (8 kit meshes over 5,000 tris, frame-0 PNG differs from the cube frame, run on this RTX 3050); cargo tree -p engine-game -e normal has no gltf; no sim crate touched, golden d57b5c8ae3096614 unchanged (engine-proof does not depend on engine-game).
- [x] VS3 game: kit textures — VS1 meshes bind their glTF base-color images
  from `content/arena/textures` through VS2. Tier A. Size S. Owns the texture
  half of `crates/game/src/arena_kit.rs`. Accept: F2P the sand region of the
  frame-60 capture has a luminance variance at least 4 times the untextured
  run; P2P golden unchanged. needs: VS1, VS2. Landed 0050d23f6. `kit_texture` resolves
  and decodes the base-color image a kit file names; a named image the import does not
  carry refuses with `KitError::Texture` naming file and asset id, and bad bytes refuse
  the same way (no white fallback). `E2040_KIT_TEXTURES=off` is read once at import so
  the staged report and the window twin list go flat together. Proof: `cargo test -p
  engine-game --features app --lib arena_kit` 52 passed (incl. the missing-image and
  switch tests); F2P capture pair at the frame-60 establishing view, sand rect
  x64..126 y72..93 (n=1386) luminance variance 14.810 vs 0.881 = 16.8x (bar 4x) on
  the kept frames `target/vs3/kit-tex60c/frame_000001.png` (14869 B) vs
  `target/vs3/kit-plain60c/frame_000001.png` (12451 B), re-measured by the lead
  (the first run read 12.912 vs 0.881 = 14.7x; the tree had moved); the probe that
  shot them was temporary and is not in the tree, the frames are the evidence;
  `cargo test -p engine2040 --release --test proof_cli default_and_max_seed` PASS with
  `final_hash == d57b5c8ae3096614`; clippy `-p engine-game --features app --all-targets
  --no-deps -- -D warnings` PASS (Windows runner); rustfmt clean; `rust-quality --owned`
  PASS (production lines 1497, under the 1500 cap, so the big-file ratchet no longer
  regresses). Named caveat: the app lib tests in the shared tree also need another
  lane's uncommitted allocator cfg fix in `crates/game/src/lib.rs`, which this row does
  not own or commit.
- [x] VS4 game: fighters at their simulated positions — `App::fighter_views()` Progress 2026-10-17: App::fighter_views landed (dd7b4743) with lib.rs fighter_views_match_sim_positions_every_tick (2 fighters, 120 ticks, byte-equal to the dataset); left: no test that at least 2 fighter meshes over 5,000 triangles are drawn.
  returns slot, position and yaw from the authoritative dataset each tick; the
  4 CT7 bodies import (after `fighter_bind::bind_all`) and draw in bind pose at
  those positions in the window and capture legs. Tier A. Size M. Owns
  `crates/game/src/fighter_draw.rs`, `fighter_views` in
  `crates/game/src/lib.rs`, the window-leg feed in `crates/game/src/camera_feed.rs`.
  Accept: F2P for a 2-fighter 120-tick run every frame's fighter positions
  equal `fighter_views` (test) and at least 2 meshes of over 5,000 triangles
  are drawn; P2P golden unchanged. needs: VS1. Landed c8bed518b. fighter_draw:: 3 passed (39919063323) and the moved default test 1 passed (39918750341); the capture test checks every frame's drawn feet equal [view.x, 0, view.z] and each body clears 5,000 tris, the window test poses roots within 1e-5 and two slot bodies clear 5,000 tris; lib.rs 4011 -> 3985 so the over-3000 cap is gone; golden d57b5c8ae3096614 and both arena pins re-proven at HEAD (39920678515, 39920678454).
- [x] VS5a anim: fighter pose to joint matrices — LOC2 `LocalPose` to the 45
  AN1 skinning matrices through the glTF-to-AN1 joint remap (43-bone
  bijection, 2 identity nodes). Tier A. Size M. Owns the remap and matrix
  builder in `crates/anim` plus its tests. Accept: F2P matrices for tick t are
  byte-identical x2 and the bind pose gives identity-times-inverse-bind within
  1e-5; P2P `cargo test -p engine-anim --lib` green. needs: VS4. Landed 1266f629a.
  `crates/anim/src/an1.rs` — `JointRemap::by_name`, `remap_pose` and
  `an1_skinning_matrices`; 6 an1 tests green (40007008375, re-run on this PC)
  cover the 43-bone bijection with 2 identity nodes, bind pose within 1e-5 per
  cell, byte-identical matrices x2 for one tick and the typed refusals; `cargo
  test -p engine-anim --lib` 526 passed 0 failed (40012328612); clippy
  `--all-targets -- -D warnings` PASS (40010908096); rust-quality --crate anim
  score 99. No sim crate touched, golden d57b5c8ae3096614 unchanged.
- [ ] VS5b game: skinned, animated fighters — `PoseSource::ImportedFighter`
  returns the live pose instead of `ArtDeferred`; VS5a matrices feed
  `v1_skin::GpuSkinned`; clips from `content/locomotion`. Tier A. Size M. Owns
  the `ImportedFighter` arm in `crates/game/src/lib.rs` and the skin bind in
  `crates/game/src/fighter_draw.rs`. Accept: F2P over 30 frames each fighter's
  silhouette mask changes (limb motion) and no frame has magenta over 0.5%;
  P2P golden unchanged. needs: VS5a.
- [x] VS7 game: torches are lights — torch actors from the placement feed the
  clustered light list (GX2) with presentation-only flicker; a night preset.
  Tier B. Size M. Owns the light feed in `crates/game/src/arena_kit.rs`.
  Accept: F2P the frame light list holds every torch actor; at night the mean
  luminance within 2 m of a torch is at least 2 times the far sand; P2P
  golden unchanged, 0 validation errors. needs: VS1, GX2. Landed 283f9fc3.
  F2P `node tools/gh-check.mjs cargo test -p engine-game --lib torch` ->
  12 passed, 0 failed (run 39724798962): the frame light list holds every
  torch actor, and the sand within 2 m of a torch is at least twice the far
  sand across the flicker; `node tools/gh-check.mjs cargo test -p engine-render
  --lib torch` -> 15 passed, 0 failed. The night arena, captured on the
  reference GPU (RTX 3050, `E2040_NIGHT=1`, `target/vs7-night5/`, sha
  `d918ab5c9dc4a96e`, `validation_errors=0`): the sky is black, the sand is
  dark, the torch-lit wall reads warm, and the opened PNG measures the
  brightest sand cell at 10.1x the far sand (frame 0) and 6.0x (frame 1).
  P2P golden d57b5c8ae3096614 unchanged
  (`cargo test -p engine2040 --release --test proof_cli default_and_max_seed`,
  run 39724333202); the day capture is unchanged too (the day rig keeps every
  bit: `the_day_rig_keeps_the_shipped_bytes`, 10 passed, run 39719248827).
  Progress
  2026-10-22: the feed landed (`ffc7dce26`, the duplicate module declaration
  fixed in `b5ba1ddf5`). `crates/render/src/torch_light.rs` owns the light
  type (position, warm color, reach, intensity, a bounded flicker), the
  presenter takes the scene's list (`set_torch_lights`), and the frame packs
  it into the shader's torch block. `crates/game/src/torch_feed.rs` turns the
  placement's torch actors into lights (the flame sits at the head of the
  stand) and `torch_acceptance.rs` mirrors the shader's falloff as the
  oracle. Proof: `cargo test -p engine-game --lib torch` -> 12 passed, 0
  failed (run 39704085075), including the two Accept arms - the frame light
  list holds every torch actor, and the sand within 2 m of a torch is at
  least twice the far sand across the flicker. The night preset landed
  (`37c0c79f0`, the `app` gate in `31b601ba3`): `E2040_NIGHT=1` drops the
  sun to a moonlit fraction (`NIGHT_SUN_SCALE`, 0.12) and the placement's
  torches carry the picture; unset keeps the shipped daylight look, so
  every existing capture stays byte-equal. Golden leg PASS
  (`cargo test -p engine2040 --release --test proof_cli default_and_max_seed`,
  run 39704439941). The night rig landed too (`923f177c9`, the night gain in
  `f478e7c47`): the arena night preset bakes a moonlit sky (the sky params
  carry a `night_gain`, so the ambient sits under the torches), the presenter
  builds under that rig (`init_offscreen_with_preset`,
  `init_for_window_with_preset`), and the capture feeds the placement's torch
  actors per frame. The day rig keeps every bit
  (`the_day_rig_keeps_the_shipped_bytes`), so the day capture is unchanged.
- [x] VS9 proof: daily arena shot — one command runs the arena capture
  (`--scene arena --fighters 2 --camera-path arena600 --frames 120 --exit`),
  writes 3 PNGs, their manifests and silhouette masks under `target/vs9/`, and
  refuses the cube SHA, the 64x48 noise SHA, empty fighter boxes and magenta
  over 0.5%. Tier A. Size S. Owns `tools/arena-shot.mjs`. Accept: F2P the
  command FAILs on today's build (cube SHA) and PASSes after VS1+VS4; P2P
  `node tools/proof.mjs` golden unchanged. needs: VS1. Landed fc2c97cf.
  `node tools/arena-shot.mjs` -> PASS build=beb39410 frames=3 masks=3
  magenta_max=0.000% dir=target/vs9; `--self-check` 17/17; golden
  `d57b5c8ae3096614` PASS via proof.mjs --skip-build.
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
  Owner chat 2026-10-19 (delegated): re-scoped per docs/plan/2026-10-18-deleted-code-triage.md: the `window_bind` bind glue is gone, so this row is the capture pair only.
  Correction 2026-10-21: the Accept's before frame `target/pilot-gx13/owner-arena/frame_000015.png` is missing (`Test-Path` = False), and the 2026-10-19 re-scope left the capture pair only, so the row cannot pass as written; it needs a pair this seat renders itself (twin frame vs kit frame) or an owner amendment.
- [x] UI3 ui: HUD overlay model — names, HP pips, tick and hit flash as Landed 780ad679. DuelOverlay called from windowed.rs; engine-ui 155 passed, run 39052304852.
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
  tick only with WIN1 consumer wired and stub removed.) Progress 2026-10-18: skipped, needs crates/render edits and real GPU captures.
- [ ] UI5 ui: play HUD — `play.step` PNGs show HP bars + names + button Progress 2026-10-17: ui side landed (play_hud.rs, raster test); left: draw it into play.step PNGs (endpoint paint_hud, new endpoint-to-ui edge).
  echo + tick + fighter-you arrow; 4F palette passes a colorblind check.
  Tier A. Size S. Owns HUD hunks beside `crates/ui/` (UI4 wiring
  frozen). Accept: seed95 PNGs carry bars/names/echo/tick/arrow;
  palette check green. needs: UI1, IN1. (PLAYTEST56 B6/C1 wave 56.)
- [x] SN10 audio: hit-sync numbers in the logs — per-hit loudness,
  clipping flag and hit-sync delta ride the smoke/staged logs over the
  PQ8 coherence bridge. Tier B. Size S. Owns log/bridge hunks beside
  `crates/audio/src/coherence.rs` + game call sites (PQ8 model frozen).
  Accept: 10 scenarios synced with loudness/clipping/delta per hit in
  the log; 0 audio-thread allocs; live 2ch 48 kHz numbers posted.
  needs: PQ8, AU1. (PLN49 R7.)
  LATER: parked 2026-10-03 (circus verdict 2: FOCUS.md parks audio, UI, modding, platform and shipping, living world, and no open finish bar lists this row); unpark by deleting this line.
  Correction 2026-10-21: duplicate of AU3 (SPRINT-02), which owns the hit-synced numbers; PB61 (SPRINT-09) names the same `crates/audio/src/coherence.rs` scenarios. Work AU3 only. RETIRED (dup): its own text names AU3 (lead, trim audit 2026-10-24)
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
- [x] Q2 proof: Q1 numeric receipt headless - refresh the Q1 evidence-only
  receipt with zero reference-GPU pixels. Tier A. Size S. Owns evidence
  only under `target/q1/` (no `crates/` hunks, no `content/` writes).
  Accept: `node tools/proof.mjs --skip-build` PASS golden
  `d57b5c8ae3096614` x2; `node tools/finish.mjs` recomputed with A4/P1
  lines quoted; placeholder refusal re-verified (V1 64x48 noise sha
  refused, V2/V3 single-cube sha refused, zero reference-GPU captures
  claimed); headless `perf-report.txt` refreshed, GPU lines PENDING.
  needs: WIN1, PF2. (Q1 deep-dive 2026-10-03.) (Ticked 2026-10-21:
  golden `d57b5c8ae3096614` re-proven x2 by two independent
  `node tools/proof.mjs --skip-build` runs; the command's overall RESULT
  FAIL names only other seats' in-flight gates, quoted in
  `docs/SPRINT-19-receipts.md` and `target/q1/receipt.md`. GPU lines
  PENDING, zero reference-GPU pixels claimed.)
- [x] CT12 assets: bake manifest — every cook and bake writes a seeded manifest
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
  Landed 51ec5cf29: `manifest.rs` (canonical LF: seed, threads, profile,
  importer + version, settings, sorted source + artifact rows with blake3 and a
  trailing checksum; corrupt/truncated/future text refused), and
  `bundle_cook --out` writes `<out>.manifest` beside the bundle. 20/20 fixture
  rebuilds byte-equal (manifest digest 82e66330f0ede22b..., seed 2040, threads
  1, profile potato), 10/10 edits invalidate exactly the transitive closure
  (failed edit keeps last-good), round-trip 13/13 and bin 3/3 green; runs
  39904265062 / 39904239734 / 39904264872, clippy 39904265069, golden 13/13
  d57b5c8ae3096614 (39903948299); baseline 929777 recorded (42f1ddc24). Park
  cleared: the code is landed and additive; DT5 remains its consumer.
- [ ] WIN10 game: arena CLI surface — `--help`, `--version`, `--mode`,
  `--list-modes` parsed as data before the winit loop; unknown flags
  exit non-zero with usage, never a silent no-op. Tier B. Size S.
  Owns arg hunks beside `crates/game/src/bin/arena.rs` (WIN9 exit path
  untouched). Accept: argv in, exit code out, loop never entered; mode
  list equals the WIN2 adapter list; replay x2. Proof: `cargo test -p
  engine-game --lib cli` green. needs: WIN1, WIN2.
  (PLN render-look.md EXTEND, proposed 2026-09-29.)
  LATER: parked 2026-10-03 (circus verdict 2: docs/plan/render-look.md parks shipping (rows stay, no writers unless a bar needs them) and only WIN9 is the bar's exception; this window polish changes no pixel, hash or timing); unpark by deleting this line.
- [x] WIN9 game: arena --exit watchdog — the exe self-exits within 60 s Landed 09ea66c3. arm_exit_watchdog; windowed tests 57 passed.
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
