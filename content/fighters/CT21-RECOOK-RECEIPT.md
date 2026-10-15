# CT21 receipt — fixed weapon graft, hero re-cook, pin moved (no commit)

Packet: CT21 (asset-specialist, free-first), CT20 follow-up. Lane `.lanes/ct21`.
Owned paths touched: `content/fighters/tools/weapons.py`, `content/fighters/`
(staged hero, body GLB, bundle, manifest, previews, this receipt) plus the
packet-ordered pin move in `crates/game/src/fighter_bind.rs` (`EXPECTED_*`).
Nothing committed.

## Bug (CT20 finding)

`tools/weapons.py graft()` reused the appended weapon's stored object location
verbatim as the bone-parent offset. That location equals the ORIGINAL-scale
(4.1917 m rig) bone head, so in the meter-scale (1.8 m) staged file the baked
weapons landed ~2.33x too far out and ~2.33x too large (Q2 bbox x +/-1.4 m,
z up to 3.87 m; red slab beside the fighter). A second defect hid behind the
first: reusing BONE parenting also reuses the source file's
`matrix_parent_inverse` (built for the original-scale head), so merely scaling
the location still bakes the wrong world transform.

## Fix (`tools/weapons.py` only code change)

- `STAGE_SCALE = 1.8 / 4.1917` (matches `assemble.py SCALE`): appended weapon
  verts are shrunk to staged meter space about the grip (object origin).
- The grip is placed at the STAGED `wep_pos_R/L` bone head with the author's
  identity rotation, and the bake runs UNPARENTED
  (`ob.parent = None`, world = T(head)), so no stale parent-inverse leaks in.
  The 100% `wep_pos` vertex weight still binds the baked verts to the animated
  hand bone in every shipped clip.
- Docstring corrected (the old "rest matrices verified identical" claim was
  false post-`assemble.py` scaling).

## License / allow-list (no new sources)

- No downloads, no new bytes. Upstream 7z re-used from the manifest pins
  (`ef53003c…fd99`, `5e97fcbb…75aaf`); graft inputs are the same CC0-1.0
  System G6 pack parts. `refused[]` untouched and still clean
  (Quaternius/Mixamo/Fab/LAFAN1/Geno nowhere in the tree).

## Blender re-cook receipt (Blender 5.2.0 LTS, exporter v5.2.39)

- Base: `incoming/staged/hero.blend1` (pre-graft backup, 2934 verts,
  bbox x +/-0.68 / z 0–1.82) + fixed `weapons.py`
  (`--sword main --shield 1`) → `incoming/staged/hero.blend`
  `bc73a5a7…6a3fa` (1,407,456 B), verts 2934 → 3049.
- Staged BODY bbox after fix: x [-0.73, 0.85], y [-0.70, 0.54],
  z [0.00, 1.82] — height back to exactly body scale (was z 3.87).
- Export x2 via `tools/export.py`:
  A = `4bdc600a…bbbd5` (2,072,320 B), B = `4bdc600a…bbbd5` — identical: PASS.
- `ct7proof` (CT1/CT2/texture/bundle gates) on `content/fighters`: `CT7-OK`.
  Hero: joints=45, tris_lod0=5850, lods=[[1448,662,288,135],
  [2232,1098,555,273],[1434,603,243,93],[736,358,162,70]], prims=4,
  skin_rows=3933, tex=1024x1024mips=11, clips=14 (digests match manifest),
  entries=42, bundle=90,364,971 B. All three opponents reproduce their manifest
  pins byte-for-byte (bundles 90271167/90353121/90286695) — only hero moved.

## Eyes (Read tool on `ct21-recook/hero_rest.png` + `hero_wire.png`) — PASS

- Silhouette coherent and symmetric, feet grounded, ~1.8 m against the striped
  1.8 m pole. Sword visibly IN the right hand (grip at palm, blade horizontal
  outward, body-proportioned ~0.9 m); shield strapped on the left forearm
  (~0.55 m heater). No flying slab, no detached limbs.
- Wire: clean freestyle topology over body + weapons, no holes.
- Scale next to reference: figure height matches the pole; weapons extend
  modestly beyond the silhouette only at the hands.
- UV/seams/rig: no stretch or seam artifacts at preview resolution; bind-pose
  limbs attached. (Hero body renders flat grey — the standing CT19 preview
  style, not a defect; opponents show atlas color.)
- Fixed renders also promoted to `previews/hero_rest.png` + `hero_wire.png`,
  replacing the CT20 red-slab failure shots.

## Budgets

- Hero 5850 tris / 4 prims+4 mats (<= 40k / <= 4). Opponents unchanged
  (5014/5916/5184). Textures unchanged (1024 sRGB + 11 mips). Hold.

## Pin move

- `manifest.json` hero row: `tris_lod0` 5628 → 5850, Q2 LOD chain
  [1212,566,240,93] → [1434,603,243,93], `skin_rows` 3577 → 3933,
  `bundle_bytes` 90332115 → 90364971,
  `bundle_sha256` → `522e78a8…ea2b`, `body_glb_sha256` → `4bdc600a…bbbd5`;
  `incoming_files` hero.blend / hero.weapon.json rows re-hashed;
  `cook_gates.eyes_pass` notes the CT21 re-eyes.
- `crates/game/src/fighter_bind.rs`: `EXPECTED_BUNDLE_SHA256[0]` →
  `522e78a8…`, `EXPECTED_BODY_SHA256[0]` → `4bdc600a…` (comments note CT21).
- Consumer: `cargo test -p engine-game fighter_bind` → 3 passed, 0 failed
  (`binds_all_four_ct7_bodies_from_manifest`, tampered-SHA refused,
  missing-body refused).

## EVIDENCE.md row proposals (one per changed asset; CC0-1.0 System G6, no new sources)

| File | Source URL | SPDX | Author | SHA-256 | Changed |
|---|---|---|---|---|---|
| `content/fighters/incoming/staged/hero.blend` | https://opengameart.org/content/modular-rpg-characters (derived) | CC0-1.0 | System G6, re-grafted CT21 lane | `bc73a5a7…6a3fa` | CT21 graft fix (STAGE_SCALE + unparented head bake) |
| `content/fighters/incoming/staged/hero.weapon.json` | derived | CC0-1.0 | engine2040 CT21 lane | `c8c0a9c3…29d14c` | re-emitted by fixed `weapons.py` |
| `content/fighters/glb/hero_body.glb` | derived (cook of the above) | CC0-1.0 | engine2040 CT21 lane | `4bdc600a…bbbd5` | re-exported, cook x2 identical |
| `content/fighters/bundle/hero.e2040` | derived (ct7proof cook) | CC0-1.0 | engine2040 CT21 lane | `522e78a8…ea2b` | re-bundled, CT7-OK |

## Files (all inside owned `content/fighters/`, plus the ordered pin move)

- Modified: `tools/weapons.py`, `incoming/staged/hero.blend`,
  `incoming/staged/hero.weapon.json`, `glb/hero_body.glb`,
  `bundle/hero.e2040`, `manifest.json`, `previews/hero_rest.png`,
  `previews/hero_wire.png`.
- New: `ct21-recook/hero_fixed.blend` (fixed staged copy),
  `ct21-recook/hero_export_a.glb` + `hero_export_b.glb` (x2 evidence),
  `ct21-recook/hero_rest.png` + `hero_wire.png` (eyes renders),
  `ct21-recook/hero.weapon.json` (graft report),
  `CT21-RECOOK-RECEIPT.md` (this file).
- Modified (packet-ordered pin): `crates/game/src/fighter_bind.rs`.
