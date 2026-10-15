# CT26-wood provenance — real weathered planks for the crate and torch pole (2026-09-29)

Third CT26 material drop: the flat brown `wood_albedo.png` look is replaced
by a real CC0 PBR weathered-plank set on `crate` (all-wood plank box) and
the `torch_pole` mesh of `torch` (the props the owner sees beside the gate
in every establishing shot). The generator (`../tools/gen.mjs`) and every
other asset are untouched; the old flat PNGs stay as the fallback and are
never deleted.

Licenses read live 2026-09-29 (CC0-only rule holds):
- https://polyhaven.com/license — all assets CC0-1.0 ("no rights reserved":
  commercial use, no attribution needed, redistribution allowed).
- https://ambientcg.com/license — all assets CC0 1.0 Universal (same terms).
  ambientCG read as the fallback source for the next materials; this drop uses
  Poly Haven bytes only.
- Asset page https://polyhaven.com/a/wooden_planks badges CC0, authors
  Charlotte Baglioni (photography) + Dario Barresi (processing), category
  Wood / Boards & Planks / Weathered Planks, 2 m wide.

| Slot (`textures/`) | Source file (`Textures/jpg/2k/wooden_planks/`) | SHA-256 | Bytes | Dims |
|---|---|---|---|---|
| `wood_albedo_2k.jpg` | `wooden_planks_diff_2k.jpg` | `02a124390e368fe7ae4bb98235fd02113cd4159fd5a7ea580ccba3e319b3577b` | 1940468 | 2048x2048 |
| `wood_normal_2k.jpg` | `wooden_planks_nor_gl_2k.jpg` | `a7ff9954729e7d4dd678b1e35a2edacef9246e45d25a714c000ce95e3cc4e436` | 3135183 | 2048x2048 |
| `wood_roughness_2k.jpg` | `wooden_planks_rough_2k.jpg` | `6b9761d9955a6405e73d0040348cf856bfa3d94127baab3b177cdd8d27390f95` | 1010429 | 2048x2048 |

Direct downloads: `https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k/wooden_planks/<file>`
(all HTTP 200, 2026-09-29; byte sizes match the
`https://api.polyhaven.com/files/wooden_planks` rows). Verbatim
originals archived 2026-10-02 to `C:\engine2040-archive\2026-10-02\incoming\`
as `polyhaven_wooden_planks_{diff,nor_gl,rough}_2k.jpg` (removed from git:
byte-identical to the `wood_*_2k.jpg` slots above, re-downloadable below;
skill staging step re-downloads there when needed).

## What changed

- `textures/wood_{albedo,normal,roughness}_2k.jpg` — new, byte-identical to
  the Poly Haven originals (SHAs above).
- `src/crate.gltf` (110280 B ->
  6878371 B, sha `cd2f0204f1b0626154476428de9c463d7054fa3190ec7e08cea47584d4586bf1`):
  albedo + normal embedded as bufferView images (`image/jpeg`, 2048x2048,
  ≤ 2k POT per budget) because the engine import lane reads bufferView slices
  and `data:` URIs only — external file URIs fail the cook
  (`crates/assets/src/mesh_import.rs`, `import_repair.rs::raw_image_bytes`).
  Embedded slices re-extract to exactly the staged SHAs. Material
  `MAT_crate`: `baseColorFactor` white (was `[0.5,0.35,0.2]`) so the real
  albedo shows, `baseColorTexture` + `normalTexture` bound, metallic 0,
  roughness 0.8 scalar (unchanged). Sampler `LINEAR_MIPMAP_LINEAR` + REPEAT,
  same as sand/stone. Wiring script (deterministic, stdlib-only) ran in temp,
  not in git.
- `src/torch.gltf` (27864 B ->
  6796090 B, sha `31af47299a8ef79b4564db6988d15a9ec8da50cfb3d91369e54202667eb770f7`):
  same wood embed on a split material — `MAT_torch_pole` carries the wood
  PBR (white factor, albedo + normal, metallic 0, roughness 0.85) while the
  cup moves to a factor-only dark iron `MAT_torch_cup` (`[0.15,0.14,0.13]`,
  metallic 0.9, roughness 0.45) so the cup no longer reads as brown wood.
  The cup's real metal PBR set rides the CT26 metal remainder. Pole prim
  stays material 0, cup prim moves 0 -> 1 (the only mesh JSON change).
- `manifest.json` — refreshed `src/crate.gltf` + `src/torch.gltf` entries +
  3 new texture entries (sha + bytes each).

## What did NOT change

- Geometry: mesh/buffer bytes 0..76032 (crate) and 0..19008 (torch)
  byte-equal to HEAD (verified against
  `HEAD:content/arena/src/{crate,torch}.gltf`); LOD0 tris unchanged
  (crate 1536 worst-prop bar holder, torch pole+cup small, all ≤ 5k prop bar).
- `wood_roughness_2k.jpg` is staged + provenanced but NOT bound: wiring a
  grayscale rough map as `metallicRoughnessTexture` would read metalness from
  the B channel (~0.9 = mirror wood). The authored ORM (R=occlusion,
  G=rough, B=0) rides the CT26 remainder — same note as sand/stone.
- `banner` (cloth + poles) and `gate_tunnel`/`debris` still map to the flat
  generated PNGs; cloth rides the next drop, metal (torch cup, gate
  fittings) the one after.

## Verification (2026-09-29, node stdlib + engine code reads)

- Source-identical twice: staged byte sizes match the API rows; re-download
  byte-matches all 3 SHAs.
- JPEG SOI + SOF dims parsed per file: all 2048x2048.
- Eyes: `wood_albedo_2k.jpg` opened with Read — weathered planks, grain,
  knots, plank gaps, no banding, no seams. PASS.
- glTF: both parse, image bufferViews in bounds, texture indices resolve 0/1,
  sampler LINEAR_MIPMAP_LINEAR + REPEAT, TEXCOORD_0 present on all 8+2 prims.
- Cook-identical twice: deterministic re-wire of the pristine HEAD copies in
  temp reproduces the in-tree SHAs exactly (`cd2f0204…` / `31af4729…`).
- Budgets: textures ≤ 2k POT with mips (sampler), LOD0 tris unchanged,
  hulls/bbox/placement untouched; A2 session manifest check is additive-safe
  (only requires names, never exact SHAs).
- Engine import: `engine-assets` mesh-import tests green (see report);
  the rewired files parse through the landed `probe_textures`/texture path
  unchanged (code frozen).

## UNVERIFIED here (no display server; rides the captures pilot)

- Window build load of the retextured crate/torch + before/after window
  captures. Pair target: `target/showcase/2026-09-27-q1/fight/frame_000015.png`
  (before, flat brown) vs the same frame after, on the RTX 3050 reference GPU,
  scored by the art seat. Q1/RA14 pilot owns it.
- `content/arena/report.json` re-baseline (crate + torch digests + bundle
  bytes move by design) and a `gen.mjs` clobber gate (re-running the
  generator would wipe this drop) — orchestrator follow-ups, same as the
  CT26-sand/stone notes.

Next asset: cloth (banner) — Poly Haven fabric candidate or ambientCG
equivalent, same shape: 2k albedo+normal staged, wired, provenanced; then
metal (torch cup, gate fittings); then gate_tunnel/debris onto the stone set.
