# CT26-cloth provenance — real crimson velvet for the banner cloth (2026-09-30)

Fourth CT26 material drop: the flat red `cloth_albedo.png` look is replaced
by a real CC0 PBR velvet set on `banner_cloth` (the twin red banners the
owner sees flanking the gate in every establishing shot). The generator
(`../tools/gen.mjs`) and every other asset are untouched; the old flat PNGs
stay as the fallback and are never deleted.

Licenses read live 2026-09-30 (CC0-only rule holds):
- https://polyhaven.com/license — all assets CC0-1.0 ("no rights reserved":
  commercial use, no attribution needed, redistribution allowed).
- https://ambientcg.com/license — all assets CC0 1.0 Universal (same terms).
  ambientCG read as the fallback source for the next materials; this drop uses
  Poly Haven bytes only.
- Asset page https://polyhaven.com/a/velour_velvet badges CC0, authors Rico
  Cilliers (plus colormass processing), category Textiles & Leather / Pile &
  Velvet / Velvet & Velour, tags red/luxury/royal.

| Slot (`textures/`) | Source file (`Textures/jpg/2k/velour_velvet/`) | SHA-256 | Bytes | Dims |
|---|---|---|---|---|
| `cloth_albedo_2k.jpg` | `velour_velvet_diff_2k.jpg` | `7c81a1577f50a023825d8d5e26ec7f8596e47534d8ff601aa0b074980833e875` | 3195339 | 2048x1971 |
| `cloth_normal_2k.jpg` | `velour_velvet_nor_gl_2k.jpg` | `20207c7b1e4328f8b8b635435d33d5adf23c44f045d1f711b4105999d1883745` | 2656496 | 2048x1971 |
| `cloth_roughness_2k.jpg` | `velour_velvet_rough_2k.jpg` | `23f0b2bfdf7f88bff072878bb763c6e825ab154da50b51040b607911b74154c3` | 3891940 | 2048x1972 |

Direct downloads: `https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k/velour_velvet/<file>`
(all HTTP 200, 2026-09-30; byte sizes match the
`https://api.polyhaven.com/files/velour_velvet` rows). Verbatim
originals archived 2026-10-02 to `C:\engine2040-archive\2026-10-02\incoming\`
as `polyhaven_velour_velvet_{diff,nor_gl,rough}_2k.jpg` (removed from git:
byte-identical to the `cloth_*_2k.jpg` slots above, re-downloadable below;
skill staging step re-downloads there when needed).

## Rejected candidate (eyes veto, nothing ships)

`rough_linen` (`https://polyhaven.com/a/rough_linen`, CC0, Rico Cilliers /
colormass) was staged first, then REJECTED on eyes-check: its albedo is a
blue-grey chambray, and the art-directed banner red
(`baseColorFactor [0.62,0.1,0.12]`) multiplied over blue-grey reads as muddy
purple-brown. The velvet diffuse above is natively crimson — within noise of
the art-directed red — so the cloth keeps its look-direction red with a
white factor while gaining real pile response and a normal map. All linen
bytes were deleted from `textures/` and `content/incoming/`; no linen row
enters `manifest.json`.

## What changed

- `textures/cloth_{albedo,normal,roughness}_2k.jpg` — new, byte-identical to
  the Poly Haven originals (SHAs above).
- `src/banner.gltf` (31900 B ->
  7835041 B, sha `8ec8f95135e83f238abab74fa95c55ebd94432f4204baccbe104d458c09e8702`):
  albedo + normal embedded as bufferView images (`image/jpeg`, ≤ 2k per
  budget) because the engine import lane reads bufferView slices and `data:`
  URIs only — external file URIs fail the cook
  (`crates/assets/src/mesh_import.rs`, `import_repair.rs::raw_image_bytes`).
  Embedded slices re-extract to exactly the staged SHAs. Split material
  (torch precedent): `MAT_banner_cloth` carries the velvet PBR (white factor
  — the albedo already is the banner red — albedo + normal bound, metallic
  0, roughness 0.85 scalar unchanged) while pole + bar move to a factor-only
  dark wood `MAT_banner_pole` (`[0.30,0.20,0.12]`, metallic 0, roughness
  0.8) so the rigging no longer reads as red cloth. Cloth prim stays
  material 1, pole/bar prims move 0 -> 0 on the new slot 0 (the only mesh
  JSON change besides the embed). Sampler `LINEAR_MIPMAP_LINEAR` + REPEAT,
  same as sand/stone/wood. Wiring script (deterministic, stdlib-only) ran in
  temp, not in git.
- `manifest.json` — refreshed `src/banner.gltf` entry + 3 new texture
  entries (sha + bytes each).

## What did NOT change

- Geometry: buffer bytes 0..21264 byte-equal to HEAD (verified against
  `HEAD:content/arena/src/banner.gltf`); LOD0 tris unchanged (≤ 5k prop
  bar), hulls/bbox/placement untouched.
- `cloth_roughness_2k.jpg` is staged + provenanced but NOT bound: wiring a
  grayscale rough map as `metallicRoughnessTexture` would read metalness from
  the B channel (~0.9 = mirror cloth). The authored ORM (R=occlusion,
  G=rough, B=0) rides the CT26 remainder — same note as sand/stone/wood.
- Dims note: velvet 2k ships 2048x1971/1972 (source is 0.3 m wide, not
  square), not 2048x2048 like the sand/stone/wood picks. This is cook-legal:
  the lane caps dims at 4096 (`texture.rs::MAX_TEXTURE_DIM`), pixels at
  16.7M (ours 4.04M), bytes at 16MB (ours ≤ 4.7MB), and its own NPOT mip
  test (5x3 -> 2x1 -> 1x1) covers the odd-edge chain (ours runs 12 levels to
  1x1, ≤ `MAX_MIPS` 13). Sampler wrap stays REPEAT.
- `gate_tunnel` + `debris` still map to the flat generated PNGs; metal
  (torch cup, gate fittings) rides the next drop.

## Verification (2026-09-30, node stdlib + engine code reads + cargo)

- Source-identical twice: independent re-download byte-matches all 3 SHAs.
- JPEG SOI + SOF dims parsed per file (table above).
- Eyes: `cloth_albedo_2k.jpg` opened with Read — deep crimson velvet pile,
  even dye, no banding, no seams. PASS. `rough_linen` albedo opened and
  REJECTED (blue-grey, see above).
- glTF: parses, image bufferViews 15/16 in bounds of the 5873099-byte
  buffer, texture indices resolve 0/1, sampler LINEAR_MIPMAP_LINEAR +
  REPEAT, TEXCOORD_0 present on all 3 prims.
- Cook-identical twice: deterministic re-wire of the pristine HEAD copy in
  temp reproduces the in-tree SHA exactly (`8ec8f951…`).
- Budgets: textures ≤ 2k per edge with mips (sampler), LOD0 tris unchanged,
  hulls/bbox/placement untouched; A2 session manifest check is additive-safe
  (only requires names, never exact SHAs).
- Engine lane: `cargo test -p engine-assets --lib` green on today's tree
  (99 passed, 0 failed); the rewired file parses through the landed
  `probe_textures`/texture path unchanged (code frozen).

## UNVERIFIED here (no display server; rides the captures pilot)

- Window build load of the retextured banner + before/after window captures.
  Pair target: `target/showcase/2026-09-27-q1/fight/frame_000015.png`
  (before, flat red — confirmed present) vs the same frame after, on the
  RTX 3050 reference GPU, scored by the art seat. Q1/RA14 pilot owns it.
- `content/arena/report.json` re-baseline (banner digest + bundle bytes move
  by design) and a `gen.mjs` clobber gate (re-running the generator would
  wipe this drop) — orchestrator follow-ups, same as the
  CT26-sand/stone/wood notes.

Next asset: metal (torch cup, gate fittings) — Poly Haven metal candidate
or ambientCG equivalent, same shape: 2k albedo+normal staged, wired,
provenanced; then gate_tunnel/debris onto the stone set; then hero props
(torches, banners, gate) + crowd backdrop per the CT8 remainder.
