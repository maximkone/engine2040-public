# E48 receipt: three glTF fighters, no magenta; sky bake under the bar

Row: `E48` in `docs/SPRINT-10-empire-inbox-drain.md`. Date 2026-10-23.

## What magenta means now

`crates/render/src/v1_skin.rs:47` defines `ERROR_MAGENTA` and the skinned
fragment path shades it when a primitive has no material or skin. Magenta is a
deliberate error signal, not a look. So "0 magenta frames" means: every
primitive of every fighter names a material, and the fighter carries a skin.

Test that proves the error path is live (so the green means something):
`v1_skin::tests::missing_skin_renders_the_error_path`.
GPU capture validation: `v1_skin::tests::gpu_skinned_capture_is_validation_clean`.

## The three fighters (and the two extras on disk)

`check.mjs` reads the JSON chunk of each GLB and counts primitives, materials,
base-colour textures, skins and joints. Run from the repo root:

    node docs/receipts/2026-10-23-e48-fighter-frames/check.mjs \
      content/fighters/glb/hero_body.glb \
      content/fighters/glb/opp_female_light_body.glb \
      content/fighters/glb/opp_male_light_body.glb

Output:

    PASS hero_body.glb: meshes 1 prims 4 materials 4 primsWithoutMaterial 0 primsWithoutBaseColorTex 0 skins 1 joints 43 skinnedNodes 1
    PASS opp_female_light_body.glb: meshes 1 prims 4 materials 4 primsWithoutMaterial 0 primsWithoutBaseColorTex 0 skins 1 joints 43 skinnedNodes 1
    PASS opp_male_light_body.glb: meshes 1 prims 4 materials 4 primsWithoutMaterial 0 primsWithoutBaseColorTex 0 skins 1 joints 43 skinnedNodes 1
    PASS glb: every fighter has a material and a skin

The same check also passes for `opp_female_medium_body.glb` and
`ct21-recook/hero_export_a.glb` (identical counts).

## Sky bake cost

New test `sky::tests::the_live_day_bake_fits_the_frame_bar` bakes the midday
sky nine times at the live capture size `SKY_CAPTURE_W x SKY_CAPTURE_H` (64x32,
the same size `arena_day.rs` uses) and asserts the median is under 0.4 ms.

Command and result (GitHub runner, `tools/gh-check.mjs`):

    cargo test -p engine-render --lib the_live_day_bake_fits_the_frame_bar -- --nocapture
    sky bake 64x32: median 0.233 ms, best 0.232 ms
    test result: ok. 1 passed; 0 failed; 930 filtered out
    https://github.com/maximkone/engine2040-public/actions/runs/40005372619

The test prints the number every run, so a regression shows its own cost.

## Open

The GLB check is a receipt script, not a gate: `crates/assets` was dirty with
another session's work, so the check was not wired into a crate test. Wire it
into `crates/assets` (body_import or a `tests/it` module) when that crate is
free, and it becomes a gate.
