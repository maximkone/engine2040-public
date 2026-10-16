# CT32 — 14-canonical-clip dedupe manifest + before/after MB through AS1 bundles

Packet: CT32 (content, Tier A, Size S). Lane `builder-content-r13`.
Owns only this file. No code touched, no bytes cooked, nothing committed.
Needs: CT7 (ticked). Sources: CC0-1.0 System G6 via CT7; no new downloads,
no Blender re-cook, no Mixamo/non-commercial bytes.

## 1. The 14 canonical clips

Source-action mapping pinned by `content/fighters/tools/ct7proof/src/main.rs:12-27`
(`CLIPS` table). Cook digest = `manifest.json` retarget digest. Bundle digest =
AS1 entry blake3, identical in all 4 variant bundles (see §2). Sidecar SHA =
`content/fighters/sidecars/<short>.json` (14 files, schema owned by CT33,
values read-only here). GLB SHA = `content/fighters/glb/clips/<action>.glb`.

| # | short | source action | glb bytes | glb sha16 | cook digest | dur s | keys | loop | root | sidecar bytes | sidecar sha16 | bundle clip bytes | bundle digest16 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | idle | human_male_idle | 69532 | a2b0cfa670722e2e | 185e70c7034759e2 | 0.792 | 819 | true | false | 183 | aed842d70bf25f08 | 29873 | 018cd00438046630 |
| 2 | run | human_male_run | 69532 | d70d936ef077de8d | 3ea8557d0d3aec49 | 0.792 | 819 | true | true | 180 | 10ce219f7ba03170 | 29872 | cb83b749dcd6297d |
| 3 | jump | human_male_jump | 69532 | 616bae7ea3976f22 | a2e04e8655f597bb | 0.792 | 819 | false | true | 183 | a0ef99d906c40aeb | 29873 | 4ee373312fc13ccb |
| 4 | death | human_male_death | 71260 | 1893e5cb2e2494901 | 9d1cea58b0889137 | 0.833 | 862 | false | false | 186 | d57189ff7312bc6e | 31422 | a57df8d2db86505b |
| 5 | idle_combat_1h | human_male_idle_combat_main_hand_melee | 69556 | a936e4a6d12961ea | 143b54df62b7441a | 0.792 | 819 | true | false | 208 | 41e10f30e0705d07 | 29896 | d88b6a7b06434c46 |
| 6 | idle_combat_unarmed | human_male_idle_combat_unarmed | 69548 | 36f6c97144f8a47e3 | f76a98cfcf0a9866 | 0.792 | 819 | true | false | 213 | 22e7a01c8cc8ec4b | 29888 | a8be09781273d3f2 |
| 7 | attack_1h_1 | human_male_cast_main_hand_melee | 53996 | 29afa8abbb1a8f13e | 1696e44a049e5f7f | 0.417 | 432 | false | false | 324 | 9867452fcbfdc5c3 | 15957 | 34cfea0f6a6830ef |
| 8 | attack_1h_2 | human_male_cast_main_hand_melee_2 | 53996 | 54283fa8fd025e2b | 8f8ffa50539f3657 | 0.417 | 432 | false | false | 324 | 597c3f4598186a7b | 15959 | 18c82aa9262ab1fb |
| 9 | attack_1h_3 | human_male_cast_main_hand_melee_3 | 53996 | ccc80face7b028b29 | 4274703f355d99c3 | 0.417 | 432 | false | false | 324 | bfcf126173c42cfa | 15959 | 9089f7d3181a9cf4 |
| 10 | attack_2h_1 | human_male_cast_two_handed_melee | 53996 | e157db55608e07974 | e4497f7576e75611 | 0.417 | 432 | false | false | 325 | f689700e5f358867 | 15958 | b54759dde9a19a26 |
| 11 | attack_2h_2 | human_male_cast_two_handed_melee_2 | 53996 | d3a5d5c287b4ce6ec | 0e6beb775c1f2e22 | 0.417 | 432 | false | false | 325 | 51a4b8edd1b5b86ae | 15960 | 914770241d19c731 |
| 12 | block_shield | human_male_cast_off_hand_shield | 53996 | 9c356f894fc8047a | a52bcd35da54bd52 | 0.417 | 432 | false | false | 208 | cb2bc392359d286b7 | 15957 | bfab0abb4a594a3b |
| 13 | attack_dual_1 | human_male_cast_dual_wield_melee | 53996 | 0f5eea98b4ea71ca | f24f6072ccb699d2 | 0.417 | 432 | false | false | 420 | a557ac629848d952 | 15958 | 2e199256a857da80 |
| 14 | taunt | human_male_power_up | 53984 | e58392df62208740 | 4f322525c7970901 | 0.417 | 432 | false | false | 186 | 9d6b6121dc9e8bc01 | 15945 | d0c5539c6b54c142 |

Full SHA-256 for every GLB, sidecar and bundle file was hashed 2026-10-08
(`ct32-hash` run); sha16 above is the first 16 hex chars of each full hash.
Sidecar `loop`/`root_motion` read from the lane sidecar files; they agree
with the `manifest.json` cook flags for all 14 clips.

## 2. Dedupe map: 56 clip slots hold 14 unique payloads

Each of the 4 AS1 bundles carries 42 entries: 16 mesh + 4 material +
4 texture + 4 skin + 14 clip (entry names `<variant>_<short>`).
Framing walk over the 4 on-disk bundles (`ct32-parse` run 2026-10-08):

- Every short has the same blake3 digest and payload size in all 4 bundles
  (`ALL_CLIPS_MATCH_ACROSS_VARIANTS=True`): e.g. `idle` 018cd00438046630 /
  29873 bytes in hero, opp_male_light, opp_female_medium, opp_female_light.
- 56 clip slots → 14 unique payloads, one copy 308,477 payload bytes,
  four copies 1,233,908 payload bytes.
- Non-clip payload per set: 360,026,424 bytes. Textures are 89,478,872
  payload bytes per bundle (98.9% of each file, per-variant atlases, out of
  CT32 scope; digests not compared, no dedupe claimed).

## 3. Before/after MB through the AS1 bundles

Measured files (gitignored cook outputs, `content/fighters/bundle/`):

| bundle | file bytes | file sha256 | clip payload |
|---|---|---|---|
| hero.e2040 | 90,364,971 | 522e78a82eef3e8b63cd353de89e6889e5c73ea311f0b9c18107185e53b9ea2b | 308,477 |
| opp_male_light.e2040 | 90,271,167 | 0c1b528c56f0e9d12d6ebda793cef19b9e463919f9f89db6e0d25f67c742e89b | 308,477 |
| opp_female_medium.e2040 | 90,353,121 | 9a341a41d4aa42f27171800865dcd12a06c62c4e8e6773ae114355072c72a632 | 308,477 |
| opp_female_light.e2040 | 90,286,695 | a9d1f3f97dffd89e90bc54fcccad01b6ec349e5bde93b081a36fcf9b5da0c99e | 308,477 |

File SHAs equal the `manifest.json` pins byte for byte (all 4 rows).
Framing bytes per bundle: hero 3538, opp_male_light 3958,
opp_female_medium 4084, opp_female_light 4042 (16-byte header + per-entry
kind/name/len/digest framing).

- BEFORE (as shipped, clips quadrupled): 361,275,954 bytes = 344.54 MiB.
- AFTER (shared clip store: one 308,477-byte clip copy + 1,198 framing
  bytes beside the untouched per-variant bodies): 360,351,721 bytes =
  343.66 MiB.
- SAVED: 924,233 bytes = 0.88 MiB (3 clip copies + their entry framing).
- Formula: after = nonclip payload (360,026,424) + all bundle framing
  (15,622) + one clip copy (308,477) + one clip-copy framing (1,198).

Clips are small next to the 89.5 MB per-variant texture atlases, so the
shared-clip win is 0.88 MiB, stated plainly. The manifest's value is the
pinned 14-digest set every intake batch dedupes against (CT42/CT47/CT54/
CT68 pattern, CT79 tool, CT58 count).

## 4. Proof (row Accept: bundle loads, `fighter_bind` green, bytes x2)

- Bundle loads: `bundle_cook --check` (engine loader, verifies header +
  per-entry blake3 the way `BUNDLE_RESOLVE` accepts them) on all 4 bundles:
  hero entries:42 bytes:90364971, opp_male_light entries:42
  bytes:90271167, opp_female_medium entries:42 bytes:90353121,
  opp_female_light entries:42 bytes:90286695. Run twice; both passes
  byte-identical per bundle (out_sha16 hero 84372a0d7b87ca04,
  opp_male_light ee042bd200e74345, opp_female_medium 44bed9084dbd9384,
  opp_female_light 1412413125b12b60, pass1==pass2 IDENTICAL all four).
- `fighter_bind` green in-lane with lane FLAGS (pool-3, --locked):
  `cargo test -p engine-game --lib fighter_bind` 3 passed, 0 failed
  (`binds_all_four_ct7_bodies_from_manifest`, tampered-SHA refused,
  missing-body refused; 293 filtered).
- Bytes x2: the 4 file SHAs above match the `manifest.json` pins (both the
  lane copy and the measured files); `--check` pass1==pass2 identical.
- Loader unit gate in-lane: `cargo test -p engine-assets --lib`
  125 passed, 0 failed, 2 ignored (bundle round-trip, digest-mismatch,
  archive, retarget, provenance suites).
- No crate or content file changed or added except this manifest; golden
  hash untouched (no tick bytes, presentation-only data row).

## 5. Consumers and follow-ups

- CT42/CT47/CT54/CT68 dedupe each intake batch against the §1 digest set.
- CT79 (`tools/dedupe.py` + receipt) automates the walk in §2.
- CT39 audits the pin bundle (this manifest is its clip input).
- CT58 counts 14 + batches against this manifest with `fighter_bind` green.
- CT36/CB4/LM2 read the §1 sidecars through the CT33 schema (untouched here).
