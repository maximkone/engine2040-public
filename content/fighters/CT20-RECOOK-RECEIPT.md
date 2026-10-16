# CT20-RECOOK receipt — hero re-cook vs pin 688f + declined 12f7 (lane `.lanes/ct20`, no commit)

Packet: CT20 (asset-specialist, free-first). Board CT19 follow-up: the DECLINED
`hero_body.glb` delta (`12f7…`) — re-cook the hero assembly from staged sources
with a full Blender re-cook receipt (CT19-COOK disclosed non-re-execution),
then compare against pin `688f` + declined `12f7`.
Owned paths: `content/fighters/` ONLY — nothing else touched, nothing committed.

## License / allow-list (no new sources)

- No downloads, no new bytes introduced. Upstream 7z re-hashed in lane, match
  manifest pins: `ef53003c…fd99` (16,600,670 B), `5e97fcbb…75aaf` (1,239,677 B).
- Staged inputs: `incoming/staged/hero.blend` `a2ba5d3f…cb02` (1,407,601 B),
  `hero.parts.json` `3d29b138…2ce2`, `hero.weapon.json` `760765b7…6541`
  (verts 2934 → 3049). All derived from the CC0-1.0 System G6 pack.
- `refused[]` untouched and still clean; Quaternius/Mixamo/Fab/LAFAN1/Geno
  nowhere in the lane. CC0-1.0 System G6 allow-list holds.

## Blender re-cook receipt (re-executed, not disclosed)

- Blender 5.2.0 LTS (`fbe6228777e7`, 2026-07-14), glTF exporter `v5.2.39`
  (same generator string as the pin).
- Pass A: `blender --background <lane>/content/fighters/incoming/staged/hero.blend
  --python <lane>/content/fighters/tools/export.py -- --variant hero
  --atlas <lane>/content/fighters/atlas/hero_atlas.png
  --bodyglb <lane>/content/fighters/ct20-recook/hero_export_a.glb`
  → `CT7-EXPORTED hero verts=3049 tris=5850 uvs=11848`.
- Pass B: identical command to `hero_export_b.glb` → same log line.
- SHAs (lane-hashed):
  - A: `12F7FCCC813426D7F8635BAE57AF1F35F88A23700EEF7C9F439E6C9C6BAE77BA`
    (2,072,316 B)
  - B: `12F7FCCC813426D7F8635BAE57AF1F35F88A23700EEF7C9F439E6C9C6BAE77BA`
    (2,072,316 B) — cook identical twice: PASS.
  - Pin `content/fighters/glb/hero_body.glb`:
    `688F607D30D3C50BE284CE4EB226604AD46979B1727300DAA7C0D3ECF68F8D8E`
    (2,048,196 B) — untouched on disk.

## Comparison: fresh cook reproduces the DECLINED delta, not the pin

| | pin `688f…` | fresh re-cook `12f7…` |
|---|---|---|
| bytes | 2,048,196 | 2,072,316 (+24,120) |
| JSON chunk | 18,028 | 18,032 (+4) |
| BIN chunk | 2,030,140 | 2,054,256 (+24,116) |
| generator | Khronos glTF Blender I/O v5.2.39 | identical |
| materials/images/textures/nodes/accessors | 4/4/4/45/33 | identical |
| embedded atlas PNGs (bufferViews) | 565144/532614/440311/226618 | identical |
| mesh prim0 (mat0) | 920 vtx / 4344 idx | identical |
| mesh prim1 (mat1) | 1374 vtx / 6696 idx | identical |
| mesh prim2 (mat2, gloves/helmet/weapon quadrant) | 821 vtx / 3636 idx, bbox x ±0.68, y 1.04–1.81 | 1177 vtx / 4302 idx (+356/+666), bbox x ±1.4, y 1.04–3.87, z ±1.6 |
| mesh prim3 (mat3) | 462 vtx / 2208 idx | identical |
| total verts | 3577 (= manifest `skin_rows`) | 3933 (+356) |

Byte-level reason: only primitive 2 differs. The staged `hero.blend` on disk is
POST-weapons-graft (BODY 3049 verts; `hero.weapon.json` 2934 → 3049) while the
pin predates the graft (pin total 3577 = manifest `skin_rows`; Q2 bbox is
head/hand-height only). Git order confirms the staleness: pin GLB committed at
`11e3ffa` (CT7+CT8), staged `hero.blend` added later at `85dd39c` with no
re-export of the pin. The declined `12f7…` prefix matches the fresh cook hash
exactly (`12F7FCCC…`), so the delta is explained as stale tooling: export from
post-graft staged sources without a pin update.

## Eyes (Read tool on fresh Blender renders — REFUSE the fresh cook)

- `ct20-recook/hero_rest.png` + `hero_wire.png` (fresh `12f7` GLB, 1.8 m pole):
  silhouette coherent and symmetric, ~1.8 m vs pole, feet grounded, wire clean,
  no UV stretch/seams/rig breakage on the body — BUT no sword/shield in the
  hands while +356 Q2 verts span ±1.4 m / up to 3.87 m: the grafted weapons are
  baked meters off the body. The stale committed `previews/hero_rest.png` shows
  the same failure as a red slab flying beside the fighter.
- Verdict on the bytes: body PASS, weapon placement FAIL → the fresh `12f7`
  cook is REFUSED for any pin move, even though the export step itself is
  deterministic (A == B). The 12f7 DECLINE stands as correct.

## Verdict: PIN HOLDS (pin-moves-with-reason conditions NOT met)

- The fresh cook reproduces `12f7` with the byte-level reason above, but the
  pin may move only with license/provenance intact (true) PLUS consumer tests
  green on the result (false: `fighter_bind.rs` pins `688f…`, and the fresh
  bytes fail eyes). Moving the pin would also need a `crates/game` edit outside
  this packet's owned paths. So: pin `688f…` stays; manifest + code pins
  unchanged; declined `12f7…` stays declined — now with a receipt instead of a
  non-re-execution disclosure.
- OPEN follow-up, CT21-class (CT15-style named row): fix `tools/weapons.py`
  graft transform so sword/shield land in the hands (Q2 bbox back to body
  scale), re-cook hero, eyes with weapons visible + in-hand, cook x2 identical,
  budgets hold, then move the pin (`manifest.json` body/bundle SHAs +
  `crates/game/src/fighter_bind.rs` `EXPECTED_*`) with `fighter_bind` green.

## Budgets

- Hero ≤ 40k tris, ≤ 4 mats: fresh 5850 tris / 4 prims+4 mats; pin 5628 per
  manifest. Hold either way. Textures unchanged (1024 sRGB + 11 mips).

## Consumer proof (on the pin result)

- `cargo test -p engine-game fighter_bind` (lane worktree) → 3 passed, 0 failed
  (`binds_all_four_ct7_bodies_from_manifest`, tampered-SHA refused,
  missing-body refused).

## EVIDENCE.md row proposals

- None: no new assets, no new sources. Provenance intact per `manifest.json`
  `sources[]` + `incoming_files[8]` (CC0-1.0, System G6, SHAs above).

## Files (all inside owned `content/fighters/`)

- New: `ct20-recook/hero_export_a.glb`, `ct20-recook/hero_export_b.glb`
  (re-cook outputs, kept out of `glb/` so the pin is untouched),
  `ct20-recook/hero_rest.png`, `ct20-recook/hero_wire.png` (eyes renders),
  `CT20-RECOOK-RECEIPT.md` (this file).
- NOTE (orchestrator wave-52 collect): only this receipt landed in the tree;
  the 6.7 MB re-cook binaries + renders stay in lane `.lanes/ct20` (refused
  cook — no value in committing declined bytes).
