# CT30 receipt — moved-pin proof: `fighter_bind` 4/4 on the pin bundle, staged 600-tick identical

Packet: CT30 (content, Tier A, Size S). Lane `.lanes/ct30` (base `53a304f`),
no code touched, nothing committed. Owns only this file.

## Pin under proof

- The CT21 graft fix + pin move already landed on master (`ad451d3`):
  hero `bundle_sha256` → `522e78a8…ea2b`, `body_glb_sha256` → `4bdc600a…bbbd5`
  (`content/fighters/manifest.json` hero row: `tris_lod0` 5850, `skin_rows`
  3933, `bundle_bytes` 90364971; opponents unchanged).
- Pin triple agrees, byte for byte:
  `manifest.json::variants[]` == `crates/game/src/fighter_bind.rs`
  `EXPECTED_BUNDLE_SHA256` / `EXPECTED_BODY_SHA256` (all 4 rows, verified by
  the passing bind test below), and all 4 on-disk body GLBs hash to the
  pinned body SHAs (hero `4BDC600A…BBBD5`, opp_male_light `9F4D71E6…A7CE1`,
  opp_female_medium `08427DD6…CB6886`, opp_female_light `EF908B0F…6595BD`,
  hashed in-lane 2026-10-02).
- Bundles (`bundle/*.e2040`) are not on disk: reproducible cook outputs,
  excluded from git per CT7+CT8; the "pin bundle" here is the pinned SHA set
  above, which is what `fighter_bind` enforces.

## `fighter_bind` 4/4 on the pin bundle

- `cargo test --manifest-path .lanes/ct30/Cargo.toml --target-dir
  target/pool-2 -j 4 --locked -p engine-game fighter_bind` → 3 passed,
  0 failed (`binds_all_four_ct7_bodies_from_manifest`, tampered-SHA refused,
  missing-body refused; 266 filtered out).
- "4/4" = all 4 variants (hero + 3 opponents) bind against the moved pin:
  manifest order preserved, joints 45/45/45/45, clips 14x4, tris_lod0
  5850/5014/5916/5184, body file names `<variant>_body.glb`, all present and
  non-empty. Both refusal tests stay green (zeroed hero bundle SHA fails with
  the culprit named; missing GLB dir fails with `hero_body.glb` named).

## Staged 600-tick identical x2

- `staged_fight --content-dir .lanes/ct30/content/fighters --ticks 600`,
  run twice to separate out-dirs (temp `ct30-a`, `ct30-b`), same binary:
  - A: ticks=600 cues=30 hits=27 knockdowns=3 final_hash=`2935468d340b210d`
    captures=4 receipt `45727395D73FDDFE…76626C6`.
  - B: ticks=600 cues=30 hits=27 knockdowns=3 final_hash=`2935468d340b210d`
    captures=4 receipt `45727395D73FDDFE…76626C6`.
- `receipt.json` A == B byte-identical (`True`; SHA-256 `45727395…76626c6`
  both); `captures/manifest.json` identical both runs (`95C16D4C…92EC289D`);
  `frame_000000.png` identical both runs (`66454768…9129529`); receipt cites
  the moved-pin SHAs, `seed:2040`, `final_hp:[1000,0,0,0]`, `sequence_fnv`
  `3a19eb61c5db7504`, `golden:d57b5c8ae3096614`.
- Receipts/captures are run artifacts in temp dirs, not committed; the
  committed proof is this file plus the green test gate.

## Provenance (unchanged, no new bytes)

- Sources still CC0-1.0 System G6 (upstream 7z pins `ef53003c…fd99`,
  `5e97fcbb…75aaf` untouched); `refused[]` still refusal-only
  (Quaternius/Mixamo/Fab/LAFAN1/Geno nowhere in the tree). No downloads, no
  Blender re-cook, no GLB/manifest/code edits in this packet.

## Accept

- `cargo test --locked -p engine-game fighter_bind` green (3 passed, 4/4
  variants bound on the moved pin); staged 600-tick receipt identical x2
  (SHA `45727395…76626c6`, final_hash `2935468d340b210d`).
