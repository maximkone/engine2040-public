# CT51 — cold-load + stream report for arena + fighters

Packet: CT51 (content, Tier A, Size S). Lane `builder-content-r15`.
Owns only this file. Bundles frozen: no bytes cooked, no code touched,
no manifest changed. Needs: CT7 (ticked), CT8 (ticked).
Sources: committed `content/arena/report.json` (+ `report_run2.json`) and
`content/fighters/manifest.json`; headless re-checks run in-lane 2026-10-09.
Box: Windows headless, no reference GPU. Reference-GPU wall clock and VRAM
with adapter+driver lines stay OPEN for the CT27/PF13 pilot (named in §7).

## 1. Per-bundle bytes (cold-load set)

| bundle | file bytes | MiB | entries | pin |
|---|---|---|---|---|
| arena (CT8 kit) | 93,026,981 | 88.72 | 172 | `report.json` bundle + `report_run2.json` (same bytes/entries) |
| hero.e2040 | 90,364,971 | 86.18 | 42 | `manifest.json` bundle_sha256 `522e78a8…` |
| opp_male_light.e2040 | 90,271,167 | 86.09 | 42 | `manifest.json` bundle_sha256 `0c1b528c…` |
| opp_female_medium.e2040 | 90,353,121 | 86.17 | 42 | `manifest.json` bundle_sha256 `9a341a41…` |
| opp_female_light.e2040 | 90,286,695 | 86.10 | 42 | `manifest.json` bundle_sha256 `a9d1f3f9…` |
| TOTAL cold set | 454,302,935 | 433.26 | — | — |

On-disk sources behind the bundles (lane-measured, `Get-ChildItem` sums):
arena `src/*.gltf` 46,329,590 + `textures/` 25,292,066 +
`hdri/` 73,750,640 bytes; fighters `glb/` 7,869,648 bytes.

## 2. Cold load vs the 5 s bar (headless)

| bundle | headless load_ms | source |
|---|---|---|
| arena | 240 (`report.json`) / 47 (`report_run2.json`) | CT8 proof runs; wall time varies run to run, bytes do not |
| hero | 44 | `manifest.json` |
| opp_male_light | 46 | `manifest.json` |
| opp_female_medium | 87 | `manifest.json` |
| opp_female_light | 81 | `manifest.json` |
| SUM headless | 498 (with the 240 ms arena run) | — |

498 ms headless ≪ 5 s. Cold wall clock on the reference GPU stays OPEN
(pilot §7); nothing here suggests pressure: the whole cold set is 433 MiB
and every per-bundle headless load is under 250 ms.

## 3. Bundle loads via AS1 (in-lane 2026-10-09, pool-2, --locked)

- `cargo test -p engine-assets --lib`: 128 passed, 0 failed, 2 ignored
  (bundle round-trip, digest-mismatch, archive, retarget, provenance suites).
- `cargo test -p engine-game --lib fighter_bind`: 3 passed, 0 failed
  (`binds_all_four_ct7_bodies_from_manifest`, tampered-SHA refused,
  missing-body refused).
- Arena `report.json`: `corrupt_refused: true`, spatial 0 findings clean.
- `fighter_bind` green means the 4 pinned fighter bundles resolve;
  tampered-SHA and missing-body refusals stay green.

## 4. Cook x2 identical

- Arena: `report.json` vs `report_run2.json` agree on bundle bytes
  (93,026,981), entries (172), per-asset digests and LOD chains. Only
  `load_ms` (240 vs 47) and JSON trailing whitespace differ: timing varies,
  bytes do not.
- Fighters: the 4 bundle SHAs in §1 equal the `manifest.json` pins
  (re-stated from the CT32 `DEDUPE-MANIFEST.md` audit on the same pins;
  pins themselves untouched here).

## 5. VRAM (estimate with method, pilot OPEN)

Method: uploaded bytes = texel bytes × 4/3 (mip chain to 1x1), sRGB/linear
as bound. Disk bytes are lane-measured; GPU bytes are computed, not read
from a driver.

- Arena bound set (`report.json` textures: 4× 1024² + 2× 256², RGBA8):
  (4 × 4,194,304 + 2 × 262,144) × 4/3 ≈ 23.1 MB.
- Fighters: one 1024² RGBA8 composite atlas per variant ≈ 5.6 MB each,
  ≈ 22.3 MB if all four resident; the staged fight pages fewer.
- The 12 `*_2k.jpg` maps + 3 HDRIs are intake/staging bytes (CT45/CT57),
  not bound by the day-soak path; excluded from the estimate.
- Estimated resident texture VRAM for the day cold set: ≈ 30–46 MB
  (arena 23.1 + 1–4 fighter atlases). Geometry/LOD buffers on top.
- No adapter line on this box (headless, no dGPU read). The reference-GPU
  VRAM + cold wall-clock capture with adapter+driver lines is the §7 pilot.

## 6. Tick allocs

Zero new tick allocs: this row adds one markdown file. Loading runs
offline at startup through the AS1 bundle path (`cook_and_write` /
`verify_bundle_bytes`, the `BUNDLE_RESOLVE` shape); the fixed tick never
cooks and no tick `.rs` is touched. Golden hash untouched (no tick bytes).

## 7. Consumers and remainder

- Consumers: CT27 soak (empty-stands baseline CT61 first, then populated
  CT72) and the PF13 baseline read §§1–2/5.
- Remainder (one pilot, reference GPU RTX 3050 6 GB): cold wall clock for
  arena + 4 fighter bundles with adapter+driver lines, resident VRAM from
  the driver, 0-findings staging note. Pass bar: cold ≤ 5 s, VRAM posted.
- Kill rule honored: no number above is presented as a GPU measurement;
  §5 is labeled estimate until the pilot posts the adapter line.
