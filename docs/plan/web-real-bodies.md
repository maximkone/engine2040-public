# The page draws the real fight, not stand-ins

Row WEB17. The page today draws a procedural ring (`ProceduralArena`) and two
boxes (`fight_body_mesh`). The native window draws the same match with the CT7
fighters through `engine_game::fighter_look::FighterLook` — skinned bodies, six
clips, hit flash, capes, trails — and the arena kit ring through `FightRing`.
That path exists, it is deterministic-presentation only, and the page cannot
reach it for one reason: everything it loads, it loads from disk.

## What exists (verified 2026-10-19)

- `crates/game/src/fighter_look.rs`: `FighterLook::load(content_dir: &Path)`,
  `stage_textures(&self, &mut Presenter)`, `stage_skinned_meshes(&mut self,
  &mut Presenter)`, `stage_frame(&mut self, &mut Presenter, fighters, alpha)`,
  `push_draws(&self, &mut DrawList, resident, fighters)`, `load_and_stage(...)`.
- `FighterLook::observe_tick` lives in `crates/game/src/fighter_look_acts.rs`.
- `FighterLook::load` reads `content/fighters/glb/*.glb` and `glb/clips/*` with
  `std::fs`, imports them with `engine_assets::body_import` (`gltf` + `image`),
  and re-hashes each body against the CT7 pins in `crate::fighter_bind`.
- `FightRing::load(kit_content_dir, half_extent_m)` + `ring.stage(presenter)`
  draw the arena kit the page stands in for.
- `crates/assets/src/bundle.rs` already carries entries of kind mesh, material,
  texture, skin and clip, and `BundleWriter`/`BundleSource` read them back. The
  `.e2bundle` format needs no change for any of this.

## Why the page cannot use it yet

1. `load` is `&Path`; wasm has `FsSource`, which is `Unsupported`.
2. A cooked bundle avoids `gltf` and `image` inside the wasm binary: those two
   crates are what `import` pulls, and they are big. The page should decode
   `CookedMesh`/`Skin`/`ImportedClip`/`CookedTexture` and never parse glTF.
3. `fighter_bind` re-hashes the source GLB bytes. Cooked entries need their own
   pin (the bundle carries per-entry blake3 digests already).

## Steps

- [ ] 1. Cook the bodies (assets, `import` feature): import the CT7 GLBs
      (`import_skinned_body`) and clips (`import_clip_source`) offline and stage
      meshes, materials, textures, skins and clips into a web bundle next to the
      arena kit, with a receipt (entries, bytes per kind, per-body digest). The
      fighters ship as their own file (`fighters.e2bundle`) fetched after the
      first frame, so the ring is up before the 8 MB of source GLBs matter.
- [ ] 2. `FighterLook::load_from(source: &dyn AssetSource)`: the same rigs from
      cooked entries instead of disk, with the bind done against the bundle's
      digests. `load(&Path)` keeps working and delegates.
- [ ] 3. The page stages it: `stage_textures`, `stage_skinned_meshes`,
      `FightRing::stage` once, then `observe_tick` + `stage_frame` +
      `push_draws` per frame, with the fight camera from `fighter_look_cam`.
      The boxes and the procedural ring leave the fight branch.
- [ ] 4. Cues and the HUD ride that path (WEB16): parry, block, hit, clash
      flashes come from the look layer the window already uses.
- [ ] 5. Proof: a capture pair from the real browser (boxes vs real bodies, one
      file each), the p50/p95 frame numbers from the CDP probe before and after,
      wasm bytes under the `tools/web-budget.json` limit, and the golden hash
      `d57b5c8ae3096614` unchanged twice. The look is presentation only: nothing
      here may enter the tick hash.

## Risk, named

CPU skinning in wasm is the unknown: about 3.9k vertices per fighter plus one
upload per changed part, per frame. If it does not fit the frame, the fallback
is fewer clip-driven parts in the page (torso and limbs at a lower update rate)
rather than a lower frame rate, and the measurement decides, not an estimate.

## Out of scope

Crowd, banners and stands beyond the kit ring the window already draws; new
clips; any change to the match rules.
