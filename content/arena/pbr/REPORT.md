# CT45 — PBR 2k + HDRI provenance report with VRAM caps

Row: CT45 content, Tier B, Size M. Owns this file only
(`content/arena/pbr/REPORT.md`). CT26 pass frozen (no re-wire here);
`content/arena/hdri/` untouched (read only, no writes).
Needs CT26 + CT8 are ticked. Consumer: Q1 captures, RA14 presenter.

Status of this drop: COMPLETE 2026-10-19. The report lands with real
bytes, provenance per file, cook-x2 checks, calculated per-map VRAM and
a measured whole-process VRAM peak on the reference GPU, plus window
captures for the Q1/RA14 read. No invented figures: the measurement
method is stated and the run that produced it is named.

## Accept checklist

- 0 findings: PASS headless. Spatial over the placed kit: 11 actors,
  0 findings, verdict clean, hash `cbf29ce484222325`, identical twice.
  (`content/arena/report.json:5`, `content/arena/report_run2.json:5`.)
- Q1/RA14 read PBR (no noise): PASS (delegated art read, 2026-10-19).
  Window captures from the RTX 3050:
  `target/reports/ct45/vram/frame_000000..04.png` (1200-frame
  `--scene arena --fighters 8` run, `--capture-every 60`, exit 0). On
  `frame_000002.png` the sand reads its authored aerial-beach grain,
  the terraces read their plank albedo, and the painted walls and stone
  band read cleanly: no sparkle, banding or white noise. Headless read
  path green (see Consumer below).
- Cook x2 identical: PASS on what this row touches. All 19 staged
  files hash identical twice and match `manifest.json`
  (`SHA-MATCH-X2`, table below). Engine lane
  `cargo test --locked -p engine-assets --lib`: 128 passed, 0 failed,
  2 ignored (run 2026-10-09). Bundle `identical_twice: true`
  (bytes 93026981, entries 172) and spatial `identical_twice: true`
  (`report.json:4-5`, `report_run2.json:4-5`).
- Provenance per file: PASS. Tables below, one row per file, with
  source URL, SPDX, author, SHA-256 and bytes. No new downloads, no
  new donor bytes in this drop.
- Per-map bytes + VRAM on the reference GPU: PASS. Per-map disk bytes
  are real (re-statted 2026-10-09, all `BYTES-MATCH`). Per-map VRAM is
  the calculated RGBA8+mips estimate with the formula stated (no
  per-texture API exists on this stack). Measured whole-process peak on
  the reference GPU: **443.1 MiB dedicated** (shared 424.7 MiB),
  identical on two 300-frame runs, over
  `target/release/arena.exe --scene arena --fighters 8 --frames 300
  --exit --require-framing` (exit 0). Method: Windows
  `\GPU Process Memory(*)\Dedicated Usage` sampled per PID every
  100 ms, summed over the process's LUID instances (only the RTX 3050
  LUID is non-zero). Adapter line, both ways: `NVIDIA GeForce RTX 3050
  6GB Laptop GPU, driver 610.62` (nvidia-smi) and the arena's own
  `arena vram budget_mb=5233 usage_mb=0 headroom_mb=5233
  dedicated_mb=6001`. The measured figure is the whole process (bound
  set + meshes + swapchain + HUD), so it bounds the 213.1 MiB bound set
  from above and stays inside the 1024 MiB comfortable cap.

No invented figures: every SHA/bytes/dims below was re-statted or
re-hashed from disk in this lane except where a cell cites its prior
live license read with date. Estimates are labeled estimates.

## Frozen scope

CT26 close-out is frozen and not re-proven here. Current in-tree SHAs
(per `content/arena/textures/ORM-PROVENANCE.md:47-75` and
`content/arena/manifest.json:3-34`):

- `src/sand_floor.gltf` `6814d73a…` bytes 1791278
- `src/stands.gltf` `66167f35…` bytes 5763553
- `src/walls.gltf` `4f5ae00a…` bytes 5763593
- `src/gate_tunnel.gltf` `5a630383…` bytes 5736073
- `src/torch.gltf` `b04cde0a…` bytes 6821759
- `src/banner.gltf` `40046652…` bytes 7837265
- `src/crate.gltf` `322d4cf3…` bytes 6879480
- `src/debris.gltf` `0f2da95d…` bytes 5736589

Geometry, LOD0 tris (total 7472), hulls, bbox and placement are
untouched. `hdri/` bytes are read-only inputs to this report.

## Provenance per file (12 PBR maps + 4 ORM + 3 HDRI)

License reads below are the prior live reads cited per file, not new
reads from this lane. All PBR/HDRI downloads are CC0-1.0.

| Slot | Source | SPDX | Author | SHA-256 | Bytes |
|---|---|---|---|---|---|
| `textures/sand_albedo_2k.jpg` | `https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k/aerial_beach_01/aerial_beach_01_diff_2k.jpg` (read live 2026-09-27/28) | CC0-1.0 (`https://polyhaven.com/license`, page `https://polyhaven.com/a/aerial_beach_01`) | Rob Tuytel | `1803d7e2861b304bea6d2e124dcd4c657fcb93843b84c17bb2db98ce48a87e4e` | 457152 |
| `textures/sand_normal_2k.jpg` | `.../aerial_beach_01/aerial_beach_01_nor_gl_2k.jpg` (same read) | CC0-1.0 | Rob Tuytel | `0f35185b7225fe2027897bb7e4586c7deb048817524f1ed104fdae7ce80bdc06` | 863847 |
| `textures/sand_roughness_2k.jpg` | `.../aerial_beach_01/aerial_beach_01_rough_2k.jpg` (same read) | CC0-1.0 | Rob Tuytel | `318b440c109de7411f9e7443f38ae3cf873f34e90afd762c931bae3b23426da2` | 505596 |
| `textures/stone_albedo_2k.jpg` | `https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k/sandstone_blocks_04/sandstone_blocks_04_diff_2k.jpg` (read live 2026-09-28) | CC0-1.0 (`https://polyhaven.com/license`, page `https://polyhaven.com/a/sandstone_blocks_04`) | Rob Tuytel | `4571ff357e041bacb75ab06ad1958e74a0b396e0f3d8b9dc9d4717f704996304` | 2066659 |
| `textures/stone_normal_2k.jpg` | `.../sandstone_blocks_04/sandstone_blocks_04_nor_gl_2k.jpg` (same read) | CC0-1.0 | Rob Tuytel | `b367bdae7a75df039f3425a6eb9d395cd390a7a67b581bec89a44d2a31a8dbdc` | 2182355 |
| `textures/stone_roughness_2k.jpg` | `.../sandstone_blocks_04/sandstone_blocks_04_rough_2k.jpg` (same read) | CC0-1.0 | Rob Tuytel | `98495e70e919ea892f2119100e64d8d5a5accb9982703fdd923477e7f2a346fb` | 1607004 |
| `textures/wood_albedo_2k.jpg` | `https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k/wooden_planks/wooden_planks_diff_2k.jpg` (read live 2026-09-29) | CC0-1.0 (`https://polyhaven.com/license`, page `https://polyhaven.com/a/wooden_planks`) | Charlotte Baglioni + Dario Barresi | `02a124390e368fe7ae4bb98235fd02113cd4159fd5a7ea580ccba3e319b3577b` | 1940468 |
| `textures/wood_normal_2k.jpg` | `.../wooden_planks/wooden_planks_nor_gl_2k.jpg` (same read) | CC0-1.0 | Charlotte Baglioni + Dario Barresi | `a7ff9954729e7d4dd678b1e35a2edacef9246e45d25a714c000ce95e3cc4e436` | 3135183 |
| `textures/wood_roughness_2k.jpg` | `.../wooden_planks/wooden_planks_rough_2k.jpg` (same read) | CC0-1.0 | Charlotte Baglioni + Dario Barresi | `6b9761d9955a6405e73d0040348cf856bfa3d94127baab3b177cdd8d27390f95` | 1010429 |
| `textures/cloth_albedo_2k.jpg` | `https://dl.polyhaven.org/file/ph-assets/Textures/jpg/2k/velour_velvet/velour_velvet_diff_2k.jpg` (read live 2026-09-30) | CC0-1.0 (`https://polyhaven.com/license`, page `https://polyhaven.com/a/velour_velvet`) | Rico Cilliers (+ colormass) | `7c81a1577f50a023825d8d5e26ec7f8596e47534d8ff601aa0b074980833e875` | 3195339 |
| `textures/cloth_normal_2k.jpg` | `.../velour_velvet/velour_velvet_nor_gl_2k.jpg` (same read) | CC0-1.0 | Rico Cilliers (+ colormass) | `20207c7b1e4328f8b8b635435d33d5adf23c44f045d1f711b4105999d1883745` | 2656496 |
| `textures/cloth_roughness_2k.jpg` | `.../velour_velvet/velour_velvet_rough_2k.jpg` (same read) | CC0-1.0 | Rico Cilliers (+ colormass) | `23f0b2bfdf7f88bff072878bb763c6e825ab154da50b51040b607911b74154c3` | 3891940 |
| `textures/sand_orm.png` | Authored original, deterministic stdlib script (no URL by construction; `ORM-PROVENANCE.md:10-31`) | CC0-1.0 (kit generator output) | engine2040 CT26 lane | `dab40fa6e1cf645b0418f6fe3dbaf95bc6c6839677eea15026a2cfb2ab57baaa` | 664 |
| `textures/stone_orm.png` | Authored original (same note) | CC0-1.0 | engine2040 CT26 lane | `55c28331a119efccdd555460024a179d7427894f11c7d06d4a9c9cc1a56fb39f` | 664 |
| `textures/wood_orm.png` | Authored original (same note) | CC0-1.0 | engine2040 CT26 lane | `3ff77c78f00914c17c68484d728acc6e4c0b6d0a269f0606fac74126173a34c0` | 664 |
| `textures/cloth_orm.png` | Authored original (same note) | CC0-1.0 | engine2040 CT26 lane | `ee00980f79c991be07c3015869efd6478388e3fa3e08240b7625f59986b246f1` | 664 |
| `hdri/arena_day.hdr` | `https://dl.polyhaven.org/file/ph-assets/HDRIs/hdr/4k/goegap_4k.hdr` (read live 2026-09-27; page `https://polyhaven.com/a/goegap`) | CC0-1.0 (`https://polyhaven.com/license`) | Greg Zaal | `c3ddb9860a308449718f1187bf988638f09eb155ad1eed45ac7ab6607eda47c0` | 22384990 |
| `hdri/arena_sunset.hdr` | `.../venice_sunset_4k.hdr` (same read; page `https://polyhaven.com/a/venice_sunset`) | CC0-1.0 | Greg Zaal | `f243a060f84df1437705d2e50bbdbcce66aa55ddb2d913c18eb3f87873213b10` | 22841596 |
| `hdri/arena_night.hdr` | `.../moonlit_golf_4k.hdr` (same read; page `https://polyhaven.com/a/moonlit_golf`) | CC0-1.0 | Greg Zaal | `66452c13b77cd8dbdd44b8ff9837d304c64d5d095ef3c93b31465024a2fb0d07` | 27968841 |

Detail per drop lives in `textures/SAND-PROVENANCE.md`,
`STONE-PROVENANCE.md`, `WOOD-PROVENANCE.md`, `CLOTH-PROVENANCE.md`,
`ORM-PROVENANCE.md` and `hdri/PROVENANCE.md`. Rejected candidates
shipped nothing (linen albedo rejected on eyes-check, see
`CLOTH-PROVENANCE.md:33-43`).

## Per-map bytes + VRAM estimate (generated)

Everything in the block below is written by `node tools/arena-report.mjs --write`
from the files on disk (stat, SHA-256, JPEG SOF / PNG IHDR / HDR header) and
`manifest.json`; `node tools/arena-report.mjs` fails when it goes stale. Roughness
JPEGs are staged and provenanced but NOT bound: a grayscale rough map would read
metalness ~0.9 from the B channel, and the authored ORM carries rough in G with B=0.
The two night/sunset HDRIs sit in `content/local/hdri` (SIZE-BUDGET.md) and are read from there.

<!-- gen:pbr-vram -->
| File | Disk bytes | SHA-256 | Dims | VRAM est. RGBA8+mips | Bound |
| --- | --- | --- | --- | --- | --- |
| `hdri/arena_day.hdr` | 22384990 | `c3ddb9860a308449718f1187bf988638f09eb155ad1eed45ac7ab6607eda47c0` | 4096x2048 | 44739242 | one active HDRI |
| `hdri/arena_sunset.hdr` (content/local/hdri) | 22841596 | `f243a060f84df1437705d2e50bbdbcce66aa55ddb2d913c18eb3f87873213b10` | 4096x2048 | 44739242 | no |
| `hdri/arena_night.hdr` (content/local/hdri) | 27968841 | `66452c13b77cd8dbdd44b8ff9837d304c64d5d095ef3c93b31465024a2fb0d07` | 4096x2048 | 44739242 | no |
| `textures/sand_albedo_2k.jpg` | 457152 | `1803d7e2861b304bea6d2e124dcd4c657fcb93843b84c17bb2db98ce48a87e4e` | 2048x2048 | 22369621 | yes |
| `textures/sand_normal_2k.jpg` | 863847 | `0f35185b7225fe2027897bb7e4586c7deb048817524f1ed104fdae7ce80bdc06` | 2048x2048 | 22369621 | yes |
| `textures/sand_roughness_2k.jpg` | 505596 | `318b440c109de7411f9e7443f38ae3cf873f34e90afd762c931bae3b23426da2` | 2048x2048 | 22369621 | no |
| `textures/stone_albedo_2k.jpg` | 2066659 | `4571ff357e041bacb75ab06ad1958e74a0b396e0f3d8b9dc9d4717f704996304` | 2048x2048 | 22369621 | yes |
| `textures/stone_normal_2k.jpg` | 2182355 | `b367bdae7a75df039f3425a6eb9d395cd390a7a67b581bec89a44d2a31a8dbdc` | 2048x2048 | 22369621 | yes |
| `textures/stone_roughness_2k.jpg` | 1607004 | `98495e70e919ea892f2119100e64d8d5a5accb9982703fdd923477e7f2a346fb` | 2048x2048 | 22369621 | no |
| `textures/wood_albedo_2k.jpg` | 1940468 | `02a124390e368fe7ae4bb98235fd02113cd4159fd5a7ea580ccba3e319b3577b` | 2048x2048 | 22369621 | yes |
| `textures/wood_normal_2k.jpg` | 3135183 | `a7ff9954729e7d4dd678b1e35a2edacef9246e45d25a714c000ce95e3cc4e436` | 2048x2048 | 22369621 | yes |
| `textures/wood_roughness_2k.jpg` | 1010429 | `6b9761d9955a6405e73d0040348cf856bfa3d94127baab3b177cdd8d27390f95` | 2048x2048 | 22369621 | no |
| `textures/cloth_albedo_2k.jpg` | 3195339 | `7c81a1577f50a023825d8d5e26ec7f8596e47534d8ff601aa0b074980833e875` | 2048x1971 | 21528576 | yes |
| `textures/cloth_normal_2k.jpg` | 2656496 | `20207c7b1e4328f8b8b635435d33d5adf23c44f045d1f711b4105999d1883745` | 2048x1971 | 21528576 | yes |
| `textures/cloth_roughness_2k.jpg` | 3891940 | `23f0b2bfdf7f88bff072878bb763c6e825ab154da50b51040b607911b74154c3` | 2048x1972 | 21539498 | no |
| `textures/sand_orm.png` | 664 | `dab40fa6e1cf645b0418f6fe3dbaf95bc6c6839677eea15026a2cfb2ab57baaa` | 256x256 | 349525 | yes |
| `textures/stone_orm.png` | 664 | `55c28331a119efccdd555460024a179d7427894f11c7d06d4a9c9cc1a56fb39f` | 256x256 | 349525 | yes |
| `textures/wood_orm.png` | 664 | `3ff77c78f00914c17c68484d728acc6e4c0b6d0a269f0606fac74126173a34c0` | 256x256 | 349525 | yes |
| `textures/cloth_orm.png` | 664 | `ee00980f79c991be07c3015869efd6478388e3fa3e08240b7625f59986b246f1` | 256x256 | 349525 | yes |

- Bound PBR set (albedo + normal + ORM per material): 178672978 bytes (170.4 MiB).
- Plus one active 4k HDRI: 44739242 bytes. Active bound estimate: 223412220 bytes (213.1 MiB).
- Cap, potato tier (2 GB VRAM floor): 268435456 bytes (256.0 MiB): PASS.
- Cap, comfortable tier (3050 class): 1073741824 bytes (1024.0 MiB): PASS.
- Estimate is an upper bound (RGBA8 plus full mips, no compressed upload). Measured VRAM with the adapter and driver line needs the reference GPU and is not claimed here.
<!-- /gen:pbr-vram -->

Caps: every bound map is <= 2048 per edge (POT except velvet
2048x1971/1972, cook-legal per `CLOTH-PROVENANCE.md:77-82`), every
ORM is 256x256, every HDRI is 4096x2048. Cook caps enforced in code:
`crates/assets/src/texture.rs:19` (`MAX_TEXTURE_DIM` 4096),
`:23` (`MAX_MIPS` 13), `crates/assets/src/bundle.rs:693-698`
(over-dim/over-mip refusal). The ~213 MiB active estimate leaves
headroom inside the 6 GB reference budget. Measured 2026-10-19 with the
adapter+driver line on the RTX 3050 laptop: peak process dedicated VRAM
443.1 MiB (shared 424.7 MiB), identical on two runs; method in the
Accept checklist above.

## Cook-x2 + findings + budgets (real numbers pasted)

- `cargo test --locked -p engine-assets --lib`: 128 passed, 0 failed,
  2 ignored (2026-10-09, this lane). Covers `cook_texture` first-cook
  == second-cook (`crates/assets/src/texture.rs:291-292,318-319`).
- File identity: all 19 files above `BYTES-MATCH` + `SHA-MATCH-X2`
  against `manifest.json` (re-hash with `node tools/arena-report.mjs`).
- Bundle: bytes 93026981, entries 172, `identical_twice: true`,
  `corrupt_refused: true` (`report.json:4`, `report_run2.json:4`).
- Spatial: actors 11, findings 0, verdict clean, hash
  `cbf29ce484222325`, `identical_twice: true` (`report.json:5`,
  `report_run2.json:5`).
- Embed slice-identity (CT26, frozen): every embedded image slice in
  all 8 glTF files re-extracts to exactly a staged `textures/` SHA
  (see `ORM-PROVENANCE.md:90-92`); no unknown slices.
- Budgets: LOD0 total 7472 tris (worst prop crate 1536, all <= 5k
  prop bar); textures <= 2k per edge with mips; ORM 664 B each;
  hulls contain; placement untouched.

Known staleness (not hidden): `report.json` / `report_run2.json`
`textures[]` still list the six flat 1024/256 PNGs, not the 2k/ORM
rows. The proof binary cooks `textures/*.png` only
(`content/arena/proof/src/main.rs:281-292,427`). The 2k/ORM truth
lives in the glTF embeds + `manifest.json` + the per-file
provenance docs above. Re-baselining the proof `textures[]` to 2k
dims/mips is orchestrator follow-up (CT26 remainder note), not part
of CT45.

## Consumer: Q1 captures, RA14 presenter

- HDRI read path: `crates/render/src/v1_ibl_shadow.rs:114`
  (`load_hdri` decodes CC0 Radiance HDR; `FORMAT=32-bit_rle_rgbe`,
  4096x2048, every texel finite per `hdri/PROVENANCE.md:28-36`).
- Texture cook path: `cook_texture` + `MAX_TEXTURE_DIM`/`MAX_MIPS` +
  bundle refusals (refs above). Bound sampler is
  `LINEAR_MIPMAP_LINEAR` + REPEAT with TEXCOORD_0 on every prim (per
  drop verification in each provenance file).
- Headless result: import/cook green (128/128 above); embeds resolve
  to staged SHAs; spatial 0 findings.
- Window result: VERIFIED 2026-10-19 on the RTX 3050 (window run, exit
  0): captures `target/reports/ct45/vram/frame_000000..04.png`, peak
  process dedicated VRAM 443.1 MiB with the adapter+driver line
  (`NVIDIA GeForce RTX 3050 6GB Laptop GPU, driver 610.62`); the
  delegated art read of the set is PASS (Accept checklist above).

## Follow-ups (named, not claimed)

- CT74 intake commit-gate reads the committed set only; untracked
  bytes never count.
- CT57 night-HDRI torch staging reads this report's HDRI rows.
- Q1/RA14 pilot: measured process VRAM and the capture set are posted
  (2026-10-19); per-map figures stay the calculated bound because the
  stack exposes no per-texture API.
- Proof `textures[]` 2k re-baseline is orchestrator follow-up (see
  staleness note).

Commands run (this lane): `cargo test --locked -p engine-assets
--lib` (128 passed, 0 failed, 2 ignored);
`node tools/arena-report.mjs` (19/19 BYTES-MATCH +
SHA-MATCH-X2, totals disk 96710551 / est 401539069).
