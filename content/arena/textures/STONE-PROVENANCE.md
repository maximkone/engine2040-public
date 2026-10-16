# CT26-stone provenance — real sandstone for the stands and walls (2026-09-28)

Second CT26 material drop: the flat grey `stone_albedo.png` look is replaced
by a real CC0 PBR sandstone-block set on `stands` + `walls` (the masonry
ring the owner sees around the sand in every establishing shot). The
generator (`../tools/gen.mjs`) and every other asset are untouched; the old
flat PNGs stay as the fallback and are never deleted.

Licenses read live 2026-09-28 (CC0-only rule holds):
- https://polyhaven.com/license — all assets CC0-1.0 ("no rights reserved":
  commercial use, no attribution needed, redistribution allowed).
- https://ambientcg.com/license — all assets CC0 1.0 Universal (same terms).
  ambientCG read as the fallback source for the next materials; this drop uses
  Poly Haven bytes only.
- Asset page https://polyhaven.com/a/sandstone_blocks_04 badges CC0, author
  Rob Tuytel, category Brick & Block / Sandstone Brick & Block, 3 m wide.

| Slot (`textures/`) | Source file (`Textures/jpg/2k/sandstone_blocks_04/`) | SHA-256 | Bytes | Dims |
|---|---|---|---|---|
| `stone_albedo_2k.jpg` | `sandstone_blocks_04_diff_2k.jpg` | `4571ff357e041bacb75ab06ad1958e74a0b396e0f3d8b9dc9d4717f704996304` | 2066659 | 2048x2048 |
| `stone_normal_2k.jpg` | `sandstone_blocks_04_nor_gl_2k.jpg` | `b367bdae7a75df039f3425a6eb9d395cd390a7a67b581bec89a44d2a31a8dbdc` | 2182355 | 2048x2048 |
| `stone_roughness_2k.jpg` | `sandstone_blocks_04_rough_2k.jpg` | `98495e70e919ea892f2119100e64d8d5a5accb9982703fdd923477e7f2a346fb` | 1607004 | 2048x2048 |

Direct downloads: `https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k/sandstone_blocks_04/<file>`
(all HTTP 200, 2026-09-28; byte sizes match the
`https://api.polyhaven.com/files/sandstone_blocks_04` rows). Verbatim
originals archived 2026-10-02 to `C:\engine2040-archive\2026-10-02\incoming\`
as `polyhaven_sandstone_blocks_04_{diff,nor_gl,rough}_2k.jpg` (removed from
git: byte-identical to the `stone_*_2k.jpg` slots above, re-downloadable
below; skill staging step re-downloads there when needed).

## What changed

- `textures/stone_{albedo,normal,roughness}_2k.jpg` — new, byte-identical to
  the Poly Haven originals (SHAs above).
- `src/stands.gltf` (96468 B -> 5762437 B, sha
  `bb2657ec595ea4759550d94f01bc3ccb2a6f9f778c8610d99b346c0ee90cf844`)
  and `src/walls.gltf` (96508 B -> 5762477 B, sha
  `0fcea4f01f496812b6bbe93a15e48d7f10f77812f72a6cf3a566cad52aa2d54c`):
  albedo + normal embedded as bufferView images (`image/jpeg`, 2048x2048,
  ≤ 2k POT per budget) because the engine import lane reads bufferView slices
  and `data:` URIs only — external file URIs fail the cook
  (`crates/assets/src/mesh_import.rs`, `import_repair.rs::raw_image_bytes`).
  Embedded slices re-extract to exactly the staged SHAs. Materials
  `MAT_stands` / `MAT_walls`: `baseColorFactor` white (were
  `[0.62,0.60,0.57]` / `[0.58,0.55,0.50]`) so the real albedo shows,
  `baseColorTexture` + `normalTexture` bound, metallic 0, roughness 0.9
  scalar (unchanged). Sampler `LINEAR_MIPMAP_LINEAR` + REPEAT, same as sand.
  Wiring script (deterministic, stdlib-only) ran in temp, not in git.
- `manifest.json` — refreshed `src/stands.gltf` + `src/walls.gltf` entries +
  3 new texture entries (sha + bytes each).

## What did NOT change

- Geometry: mesh/buffer bytes 0..66528 byte-equal to HEAD in both files
  (verified against `HEAD:content/arena/src/{stands,walls}.gltf`); LOD0 tris
  unchanged (stands 1344, walls 1344, both ≤ 5k prop bar).
- `stone_roughness_2k.jpg` is staged + provenanced but NOT bound: wiring a
  grayscale rough map as `metallicRoughnessTexture` would read metalness from
  the B channel (~0.9 = mirror stone). The authored ORM (R=occlusion,
  G=rough, B=0) rides the CT26 remainder — same note as sand.
- `gate_tunnel` + `debris` still map to the flat `stone_albedo` conceptually;
  they re-cook onto this set (or their own stone pick) in the CT26 remainder.

## Verification (2026-09-28, node stdlib + engine code reads)

- Source-identical twice: independent re-download byte-matches all 3 SHAs.
- JPEG SOI + SOF dims parsed per file: all 2048x2048.
- Eyes: `stone_albedo_2k.jpg` opened with Read — weathered sandstone block
  courses, warm travertine tone, block joints, no banding, no seams. PASS.
- glTF: parses, 37 bufferViews in bounds, texture indices resolve 0/1,
  sampler LINEAR_MIPMAP_LINEAR + REPEAT, TEXCOORD_0 present on all 7+7 prims.
- Cook-identical twice: deterministic re-wire of the pristine HEAD copies in
  temp reproduces the in-tree SHAs exactly
  (`bb2657ec…` / `0fcea4f0…`).
- Budgets: textures ≤ 2k POT with mips (sampler), LOD0 tris unchanged,
  hulls/bbox/placement untouched.
- Engine cook of the rewired files runs the landed `probe_textures`/texture
  path unchanged (code frozen); `content/arena/proof/` re-run receipt lands
  with the CT26 remainder (it also re-baselines the sand + stone digests and
  bundle bytes, deferred to the orchestrator per the CT26-sand note).

## UNVERIFIED here (no display server; rides the captures pilot)

- Window build load of the retextured stands/walls + before/after window
  captures. Pair target: `target/showcase/2026-09-27-q1/fight/frame_000015.png`
  (before, flat grey) vs the same frame after, on the RTX 3050 reference GPU,
  scored by the art seat. Q1/RA14 pilot owns it.

Next asset: wood (torch pole, crate) — Poly Haven `wood_*`/`bark_*`
candidate or ambientCG equivalent, same shape: 2k albedo+normal staged,
wired, provenanced; then cloth (banner), metal (torch cup, gate fittings);
then gate_tunnel/debris onto the stone set; then torches/banners/gate hero
props + crowd backdrop.
