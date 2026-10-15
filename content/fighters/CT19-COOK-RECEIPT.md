# CT19-COOK receipt — fighter incoming bind (lane `.lanes/ct19c`, no commit)

Packet: CT19-COOK (asset-specialist, free-first). Board `docs/SPRINT.md` CT19
(wave-49 line): LAND stands; this packet covers cook + place + per-file
provenance. Orchestrator collects; nothing here is committed.

## License (allow-list: CC0-1.0 System G6 only)

- Live re-read 2026-09-27: `https://opengameart.org/content/modular-rpg-characters`
  serves Author System G6, `License(s): CC0`, both 7z still listed
  (16.6 MB + 1.2 MB). No other source introduced; no Quaternius / Mixamo /
  Fab / LAFAN1 / Geno bytes anywhere in the lane.
- 7z SHAs re-hashed 2026-09-27, match manifest pins:
  `ef53003c…fd99` (16,600,670 B), `5e97fcbb…75aaf` (1,239,677 B).

## Files changed in this lane (only)

- `content/fighters/manifest.json`: `sources[].license_read_live` refreshed to
  the 2026-09-27 live read; new top-level `incoming_files[8]` per-file
  provenance rows (2 upstream 7z + 4 staged hero files with SHA-256/bytes +
  2 extraction intermediates). `variants[]` untouched, `refused[4]` untouched
  (still refusal-only, no forbidden bytes).
- crates/spatial/examples/ct19_ten_place.rs (planned, lane-local only, never landed): 10-placement proof
  harness (not part of the spatial crate; orchestrator may drop or keep).

Kept OUT per packet: the DECLINED `hero_body.glb` delta (`12f7…`) is absent;
`content/fighters/glb/hero_body.glb` still pins
`688f607d…f68d8e` (hashed in lane and root, identical).

## Cook-bytes-identical x2

- 4 body GLBs hashed twice 2026-09-27: pass1 == pass2 (`True`), all equal to
  the manifest pins (hero `688f…`, opp_male_light `9f4d…`,
  opp_female_medium `0842…`, opp_female_light `ef90…`).
- Engine cook gates (same CT1/CT2/texture path): `cargo test -p engine-assets
  --features import identical` → 4 passed
  (`retarget::cook_identical_twice`, `mesh_cook::cook_identical_twice`,
  `archive::round_trip_entries_identical`, `bc::encode_identical_bytes_twice`);
  `cook` filter → 19 passed (incl. `over_budget_meshes_refused_with_counts`).

## Retarget tolerance (CT1)

- Contract (`crates/assets/src/retarget.rs::assert_within_tolerance`):
  bind-relative rotation deltas within 1e-4 per component, scaled offsets
  within 1e-5 per component, every mapped joint and key.
- `cargo test -p engine-assets --features import three_clips_retarget` →
  `three_clips_retarget_within_tolerance` ok; full `retarget` filter → 13 passed.

## 10 placements, 0 findings (SP5)

- `cargo run -p engine-spatial --example ct19_ten_place` (lane worktree):
  10 hero-scale (0.6 x 1.8 x 0.6) bodies solved one at a time
  (NextTo N/S/E/W walls, 4 NextTo points, center+Facing gate, NextTo fighter),
  all grounded (y=0.90); final `run()` over the full catalog:
  `placements=10 findings=0 verdict=clean hash=cbf29ce484222325`.
- Gate suite: `cargo test -p engine-spatial place` → 13 passed;
  `cargo test -p engine-spatial --lib` → 119 passed, 0 failed, 7 ignored.

## Budgets (hero ≤ 40k tris, ≤ 4 mats)

- hero 5628 tris / 4 prims; opp_male_light 5014; opp_female_medium 5916;
  opp_female_light 5184 (all LOD0, from manifest; max 5916 << 40k/25k).
  Textures 1024 sRGB + 11 mips; 4 quadrant materials per body.

## Eyes (12/12 PASS, Read tool)

- Rest x4: full silhouettes, symmetric limbs, grounded feet, ~1.8 m scale bar
  matches figure height. Attacks x4: coherent mid-swing deformation, no
  detached limbs, feet planted. Wires x4 (+hero): clean topology, no holes,
  no stretch artifacts. Nothing refused. Note: hero previews render untextured
  grey (preview style, not a defect); opponents show atlas textures.

## Consumer

- `cargo test -p engine-game fighter_bind` (root) → 3 passed
  (`binds_all_four_ct7_bodies_from_manifest`, tampered-SHA refused,
  missing-body refused). Lane `variants[]` region verified pin-identical
  (hero bundle `14a0926d…`, body `688f607d…` present; refused[] still
  refusal-only), so the lane manifest binds the same consumer.

## EVIDENCE.md row proposals (one per asset)

| File | Source URL | SPDX | Author | SHA-256 | Changed |
|---|---|---|---|---|---|
| `content/fighters/incoming/modular_rpg_characters.7z` | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (Qoma) | `ef53003c…fd99` | none (upstream bytes) |
| `content/fighters/incoming/modular_rpg_characters_ik_rig_for_animating.7z` | same page | CC0-1.0 | System G6 (Qoma) | `5e97fcbb…75aaf` | none (IK reference only) |
| `content/fighters/incoming/staged/hero.blend` | derived from 7z above | CC0-1.0 | System G6, staged CT19 lane | `a2ba5d3fbcb01f70e14485bd2c2ef1f117e2601e601b65487abf3f6b9498cb02` | assembled via `tools/assemble.py` (9 parts, scale 0.4294) |
| `content/fighters/incoming/staged/hero.blend1` | derived | CC0-1.0 | System G6, staged CT19 lane | `24de66d6…a69d4` | Blender backup, not cooked |
| `content/fighters/incoming/staged/hero.parts.json` | derived | CC0-1.0 | engine2040 CT19 lane | `3d29b138…2ce2` | emitted by `assemble.py` (atlas tiles) |
| `content/fighters/incoming/staged/hero.weapon.json` | derived | CC0-1.0 | engine2040 CT19 lane | `760765b7c9c4f89c7ca3afd3d1ace3e674f0a45729960e0fe4a2d89526542341` | emitted by `tools/weapons.py` (sword+shield graft) |

(Full 64-hex SHAs live in the lane `manifest.json::incoming_files`.)

## Standing conditions (not failures)

- The Blender assemble/weapons/export stages were not re-executed in this
  packet (20-min budget; Blender 5.2 path is `content/fighters/tools/run_ct19.js`).
  Standing evidence: staged SHAs above, 12/12 eyes PASS, engine-cook x2 gates
  green, body GLB pins unchanged. Re-running the Blender cook is a CT20-class
  follow-up if the orchestrator wants a fresh Blender receipt.
- Root tree untouched except pre-existing untracked `content/fighters/bundle/`
  + `.opencode/bin/` (not mine, not staged, not committed).
