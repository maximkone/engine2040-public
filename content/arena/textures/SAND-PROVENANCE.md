# CT26-sand provenance — real sand for the arena floor (2026-09-27/28)

First CT26 material drop: the generated flat `sand_albedo.png` look is replaced
by a real CC0 PBR sand set on `sand_floor` (the largest screen-area surface).
The generator (`../tools/gen.mjs`) and every other asset are untouched; the old
flat PNGs stay as the fallback and are never deleted.

Licenses read live 2026-09-27/28 (CC0-only rule holds):
- https://polyhaven.com/license — all assets CC0-1.0 ("no rights reserved":
  commercial use, no attribution needed, redistribution allowed).
- https://ambientcg.com/license — all assets CC0 1.0 Universal (same terms).
  ambientCG read as the fallback source for the next materials; this drop uses
  Poly Haven bytes only.
- Asset page https://polyhaven.com/a/aerial_beach_01 badges CC0, author
  Rob Tuytel, category Ground & Terrain / Sand.

| Slot (`textures/`) | Source file (`HDRIs/Textures/jpg/2k/`) | SHA-256 | Bytes | Dims |
|---|---|---|---|---|
| `sand_albedo_2k.jpg` | `aerial_beach_01/aerial_beach_01_diff_2k.jpg` | `1803d7e2861b304bea6d2e124dcd4c657fcb93843b84c17bb2db98ce48a87e4e` | 457152 | 2048x2048 |
| `sand_normal_2k.jpg` | `aerial_beach_01/aerial_beach_01_nor_gl_2k.jpg` | `0f35185b7225fe2027897bb7e4586c7deb048817524f1ed104fdae7ce80bdc06` | 863847 | 2048x2048 |
| `sand_roughness_2k.jpg` | `aerial_beach_01/aerial_beach_01_rough_2k.jpg` | `318b440c109de7411f9e7443f38ae3cf873f34e90afd762c931bae3b23426da2` | 505596 | 2048x2048 |

Direct downloads: `https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k/<path>`
(all HTTP 200, 2026-09-28). Verbatim originals archived 2026-10-02 to
`C:\engine2040-archive\2026-10-02\incoming\` as
`polyhaven_aerial_beach_01_{diff,nor_gl,rough}_2k.jpg` (removed from git:
byte-identical to the `sand_*_2k.jpg` slots above, re-downloadable above;
skill staging step re-downloads there when needed).

## What changed

- `textures/sand_{albedo,normal,roughness}_2k.jpg` — new, byte-identical to
  the Poly Haven originals (SHAs above).
- `src/sand_floor.gltf` (28231 B ->
  1790167 B, sha `8a1604b16a6053b126ac9f9d79e72f1dd07632e15e6413cd73b99f80563f700c`):
  albedo + normal embedded as bufferView images (`image/jpeg`, 2048x2048,
  ≤ 2k POT per budget) because the engine import lane reads bufferView slices
  and `data:` URIs only — external file URIs fail the cook
  (`crates/assets/src/mesh_import.rs`, `import_repair.rs::raw_image_bytes`).
  Embedded slices re-extract to exactly the staged SHAs. Material
  `MAT_sand_floor`: `baseColorFactor` white (was `[0.76,0.62,0.42]`) so the
  real albedo shows, `baseColorTexture` + `normalTexture` bound, metallic 0,
  roughness 0.95 scalar. JPEG decode confirmed available in the cook
  (`zune-jpeg` in `Cargo.lock`; `with_guessed_format` in `probe_textures`).
- `manifest.json` — refreshed `src/sand_floor.gltf` entry + 3 new texture
  entries (sha + bytes each).

## What did NOT change

- Geometry: meshes/accessors/nodes/scenes byte-equal to HEAD; buffer bytes
  0..20016 identical (verified `HEAD:content/arena/src/sand_floor.gltf`).
- `sand_roughness_2k.jpg` is staged + provenanced but NOT bound: wiring a
  grayscale rough map as `metallicRoughnessTexture` would read metalness from
  the B channel (~0.9 = mirror sand). The authored ORM (R=occlusion, G=rough,
  B=0) rides the CT26 remainder.
- UVs stay 0..1 over the 30 m floor (~68 px/m): correct tiling (repeat 4-8x)
  rides the CT26 remainder with the A1 clamp note in `content/arena/tools/gen.mjs`.

## Verification (2026-09-28, node stdlib + engine code reads)

- Source-identical twice: independent re-download byte-matches all 3 SHAs.
- JPEG SOI + SOF dims parsed per file: all 2048x2048.
- Eyes: `sand_albedo_2k.jpg` opened with Read — clean wind-rippled sand, no
  banding, no seams, tileable appearance. PASS.
- glTF: parses, 7 bufferViews in bounds, texture indices resolve 0/1,
  sampler LINEAR_MIPMAP_LINEAR + REPEAT, TEXCOORD_0 present on the primitive.
- Budgets: textures ≤ 2k POT with mips (sampler), sand LOD0 512 tris
  unchanged (≤ 5k prop bar), hulls/bbox/placement untouched.

## UNVERIFIED here (no display server; rides the captures pilot)

- Window build load of the retextured floor + before/after window captures.
  Pair target: `target/showcase/2026-09-27-q1/fight/frame_000015.png`
  (before, flat) vs the same frame after, on the RTX 3050 reference GPU,
  scored by the art seat. Q1/RA14 pilot owns it.
- `content/arena/report.json` re-baseline (sand digest + bundle bytes move by
  design) and a `gen.mjs` clobber gate (re-running the generator would wipe
  this drop) — orchestrator follow-ups, same as the CT8-HDRI lane note.
- Engine cook of the new images runs the landed `probe_textures`/texture path
  unchanged (code frozen); a cook-x2 receipt lands with the CT26 remainder.

Next asset: stone (stands/walls) — Poly Haven `rocky_terrain`/`sandstone`
candidate or ambientCG equivalent, same shape: 2k albedo+normal staged,
wired, provenanced; then wood, cloth, metal; then torches/banners/gate props.
