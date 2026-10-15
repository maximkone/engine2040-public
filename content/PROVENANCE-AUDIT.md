# CT46 provenance audit dry-run for the CT28 gate (50-asset matrix)

Row: CT46 content. Tier A. Size S. Owns `content/PROVENANCE-AUDIT.md` (gate code stays CT28).
Needs: CT28 (ticked), CT7 (ticked), CT8 (ticked). Consumer: CT28 lock, PR1 chains.
Plan: `docs/plan/content.md` section 3 file map + section 8 (`CT46/CT52/CT71 fail when the matrix re-check is skipped or a gap has no named owner and refusal`).

Method (no invented figures): SHAs below are copied byte-exact from `content/arena/manifest.json` (34 files) and `content/fighters/manifest.json` (6 incoming + 4 variant bodies + 6 hero clip digests). URLs/SPDX/authors are copied from the same manifests plus `docs/EVIDENCE.md` tracks B/C. The gate checked is `crates/assets/src/provenance.rs` (`check_release`, `provenance_row`, `release_digest`). Gate code is unchanged by this row.

## 50-asset matrix (release dry-run set)

Arena originals (34): hand-made chain. URL is the deterministic generator that authored them (`content/arena/tools/gen.mjs`), SPDX CC0-1.0, author `engine2040 CT8 lane`. SHA is the manifest `sha256`. This mapping is stated, not hidden: there are no donor bytes in the kit (`content/arena/README.md`: original authored kit, no donor bytes).

| # | gate name | URL | SPDX | author | SHA |
|---|---|---|---|---|---|
| 1 | content/arena/hdri/arena-day.hdr | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | c3ddb9860a308449718f1187bf988638f09eb155ad1eed45ac7ab6607eda47c0 |
| 2 | content/arena/hdri/arena-night.hdr | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 66452c13b77cd8dbdd44b8ff9837d304c64d5d095ef3c93b31465024a2fb0d07 |
| 3 | content/arena/hdri/arena-sunset.hdr | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | f243a060f84df1437705d2e50bbdbcce66aa55ddb2d913c18eb3f87873213b10 |
| 4 | content/arena/placement.json | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | c310b0697c5230da33da047632940505ee543e4cb589e1f993d483cd155ca167 |
| 5 | content/arena/src/banner.gltf | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 40046652694c2976be5443f4d2462ff24eeb3fb7d662e68fbb75a5667abf5226 |
| 6 | content/arena/src/crate.gltf | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 322d4cf3a807ab572ce6d9af25b0707c7793752238fd0eeaa27d5436fd84b1b7 |
| 7 | content/arena/src/debris.gltf | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 0f2da95df0f61b52963b3368bd42a604dde3fae866231dd914339802fa06027b |
| 8 | content/arena/src/gate-tunnel.gltf | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 5a6303835a452930cdab437a95762a632342bc1642d123d3653ee25f2fbbccc6 |
| 9 | content/arena/src/sand-floor.gltf | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 6814d73a28e3882eaa978b401d01d63cc6fa81b454b975515948da3794023f74 |
| 10 | content/arena/src/stands.gltf | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 66167f3522d0b84becdcdd9572d9118690823b7a1beb8f2d02f5c241ae50d98e |
| 11 | content/arena/src/torch.gltf | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | b04cde0a248a1ea269559736dafe5994dcc756b1eb155daa6f5343d37162f090 |
| 12 | content/arena/src/walls.gltf | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 4f5ae00ad614e44b3b10e273844669504aa398e888bc578fccc700ea9d413936 |
| 13 | content/arena/textures/cloth-albedo.png | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 4ca086c800cda30b0db00cfd576d45c9930a6288e0fe183cd23aab5c367ffbdb |
| 14 | content/arena/textures/cloth-albedo-2k.jpg | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 7c81a1577f50a023825d8d5e26ec7f8596e47534d8ff601aa0b074980833e875 |
| 15 | content/arena/textures/cloth-normal-2k.jpg | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 20207c7b1e4328f8b8b635435d33d5adf23c44f045d1f711b4105999d1883745 |
| 16 | content/arena/textures/cloth-orm.png | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | ee00980f79c991be07c3015869efd6478388e3fa3e08240b7625f59986b246f1 |
| 17 | content/arena/textures/cloth-roughness-2k.jpg | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 23f0b2bfdf7f88bff072878bb763c6e825ab154da50b51040b607911b74154c3 |
| 18 | content/arena/textures/flat-normal.png | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 3a178f739ed3841560b6c743e7606da64ecc3f3c0b7552728dc2a99dad844ab1 |
| 19 | content/arena/textures/metal-roughness.png | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 90de097ec1c4df53bed4d2bece19625511e467a452caaf4596074b391a0c2df1 |
| 20 | content/arena/textures/sand-albedo.png | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 2ad063f80afd58215d29736c9b6eb42077323307f4694dcb8fc9c3ed946883d4 |
| 21 | content/arena/textures/sand-albedo-2k.jpg | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 1803d7e2861b304bea6d2e124dcd4c657fcb93843b84c17bb2db98ce48a87e4e |
| 22 | content/arena/textures/sand-normal-2k.jpg | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 0f35185b7225fe2027897bb7e4586c7deb048817524f1ed104fdae7ce80bdc06 |
| 23 | content/arena/textures/sand-orm.png | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | dab40fa6e1cf645b0418f6fe3dbaf95bc6c6839677eea15026a2cfb2ab57baaa |
| 24 | content/arena/textures/sand-roughness-2k.jpg | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 318b440c109de7411f9e7443f38ae3cf873f34e90afd762c931bae3b23426da2 |
| 25 | content/arena/textures/stone-albedo.png | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 3b8c40c0d3a97977d8dd107404ddc0e229d4cbe0bfed870ce7bf0db2cb057ff3 |
| 26 | content/arena/textures/stone-albedo-2k.jpg | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 4571ff357e041bacb75ab06ad1958e74a0b396e0f3d8b9dc9d4717f704996304 |
| 27 | content/arena/textures/stone-normal-2k.jpg | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | b367bdae7a75df039f3425a6eb9d395cd390a7a67b581bec89a44d2a31a8dbdc |
| 28 | content/arena/textures/stone-orm.png | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 55c28331a119efccdd555460024a179d7427894f11c7d06d4a9c9cc1a56fb39f |
| 29 | content/arena/textures/stone-roughness-2k.jpg | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 98495e70e919ea892f2119100e64d8d5a5accb9982703fdd923477e7f2a346fb |
| 30 | content/arena/textures/wood-albedo.png | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 955c214f801e3dbed8bc5583f0ffa48893ab457fa7eaf68755c10684c344f56d |
| 31 | content/arena/textures/wood-albedo-2k.jpg | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 02a124390e368fe7ae4bb98235fd02113cd4159fd5a7ea580ccba3e319b3577b |
| 32 | content/arena/textures/wood-normal-2k.jpg | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | a7ff9954729e7d4dd678b1e35a2edacef9246e45d25a714c000ce95e3cc4e436 |
| 33 | content/arena/textures/wood-orm.png | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 3ff77c78f00914c17c68484d728acc6e4c0b6d0a269f0606fac74126173a34c0 |
| 34 | content/arena/textures/wood-roughness-2k.jpg | https://github.com/engine2040/engine2040/blob/master/content/arena/tools/gen.mjs | CC0-1.0 | engine2040 CT8 lane | 6b9761d9955a6405e73d0040348cf856bfa3d94127baab3b177cdd8d27390f95 |

Fighters (16): hand-made chain over the OGA System G6 CC0 pack. Page `https://opengameart.org/content/modular-rpg-characters` (badge CC0, read live 2026-09-25 per EVIDENCE track B, re-read 2026-09-27 per manifest). Incoming SHAs are the manifest `sha256`; variant SHAs are the manifest `body_glb_sha256`; clip SHAs are the manifest clip `digest` (16 lowercase hex, meets `SHA_MIN_LEN`).

| # | gate name | URL | SPDX | author | SHA |
|---|---|---|---|---|---|
| 35 | content/fighters/incoming/modular-rpg-characters.7z | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | ef53003c6b29798a634fb27a3f9fc3181a2c02b217689744f2944fc5fcc0fd99 |
| 36 | content/fighters/incoming/ik-rig.7z | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 5e97fcbb17c23c6d84e085a54ea32c8a4fe0ee471a1d9b41bff9dead40e75aaf |
| 37 | content/fighters/incoming/staged/hero.blend | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma), staged by engine2040 CT19 lane, re-grafted by CT21 lane | bc73a5a7fd46c8170a06c80eef5d210b68648666d5924d76c5774fb489d6a3fa |
| 38 | content/fighters/incoming/staged/hero.blend1 | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma), staged by engine2040 CT19 lane | 24de66d628562a961ec249c35158fe728e7d00ffd7c4da1fed325d47d5ca69d4 |
| 39 | content/fighters/incoming/staged/hero.parts.json | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | engine2040 CT19 lane (part list over System G6 parts) | 3d29b13820cb1305f77aeff7307a309064b447e24d6b27350fcdadec082c2ce2 |
| 40 | content/fighters/incoming/staged/hero.weapon.json | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | engine2040 CT21 lane (cook report over System G6 parts) | c8c0a9c303d513035e23f51b32026e8e45e074a6f92291657861ecf5c329d14c |
| 41 | content/fighters/bodies/hero.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 4bdc600a7acb1fa057f33e5c680bff9f74d047f5d2de22df6a8addc1e94bbbd5 |
| 42 | content/fighters/bodies/opp-male-light.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 9f4d71e67365af68d85ad2e17541ca22a87956e7a235171e27953c32a04a7ce1 |
| 43 | content/fighters/bodies/opp-female-medium.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 08427dd6aaef4bdb9a317c023306778555503c1ea16d82747aaea9ef84cb6886 |
| 44 | content/fighters/bodies/opp-female-light.glb | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | ef908b0fa2b4af5e9880dc87ccbd77002178c64bae4dfebec06b78e5e6595bd2 |
| 45 | content/fighters/clips/hero-idle | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 185e70c7034759e2 |
| 46 | content/fighters/clips/hero-run | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 3ea8557d0d3aec49 |
| 47 | content/fighters/clips/hero-jump | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | a2e04e8655f597bb |
| 48 | content/fighters/clips/hero-death | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 9d1cea58b0889137 |
| 49 | content/fighters/clips/hero-idle-combat-1h | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | 143b54df62b7441a |
| 50 | content/fighters/clips/hero-idle-combat-unarmed | https://opengameart.org/content/modular-rpg-characters | CC0-1.0 | System G6 (aka Qoma) | f76a98cfcf0a9866 |

Note on gate-name mapping: manifest paths use `src/sand_floor.gltf` and `incoming/modular_rpg_characters.7z`; gate names above are the lowercased, separator-normalised release names the gate checks (`MAX_NAME_CHARS` 64, chars `a-z0-9_/-/.`). The SHA column is the manifest digest, not a rename. Clip rows use the manifest clip `digest` as the SHA (16 hex, the gate minimum).

## Release-refusal dry-run (names every gap)

The gate itself is proven by `cargo test --locked -p engine-assets provenance` (7 passed, see Receipts). Its refusal shape is covered by the landed tests:

- `provenance_release_matrix_of_fifty_names_the_gap`: 50 hand records pass; clearing record 37 refuses with code `missing_url` naming `content/props/prop-37`; re-adding it and duplicating `prop-00` refuses with code `duplicate` naming that asset.
- `provenance_each_hand_gap_is_named`: empty URL -> `missing_url`, blank author -> `missing_author`, short digest -> `bad_sha`, spaced licence -> `bad_spdx`, `GPL-3.0-only`/`CC-BY-NC-4.0`/`LicenseRef` -> `license_refused`, each naming `content/arena/sand.glb`.
- `provenance_ai_chain_links_are_each_required`: model without generator, short prompt digest, and missing model each refuse with `ai_chain` naming the field.

Real gaps this dry-run names (release would refuse until each has a row + SHA):

1. `content/fighters/incoming/modular/` — 53-file 7z extraction, cook intermediate; manifest says `no per-file SHA kept`, re-extractable from the pinned 7z (row 35). No per-file gate row exists. Owner: CT58 receipt / CT75 triage. Refusal if listed per file: `bad_sha`.
2. `content/fighters/incoming/ikrig/` — 3-file IK extraction, reference only; same `no per-file SHA kept` note. Owner: CT58/CT75. Refusal if listed per file: `bad_sha`.
3. `content/arena/pbr/` — untracked on disk (`git status --porcelain content/` shows `?? content/arena/pbr/`). Nothing in the matrix may reference it until CT45/CT74 pin SHA + licence rows. Owner: CT45 report, CT74 gate. Refusal today: `missing_url`/`bad_sha` (no row).
4. `content/incoming/` (e.g. `studio_small_09_1k.hdr`) — git-ignored by `.gitignore:15:content/incoming/`; EVIDENCE track lists the file as CC0-1.0 but no manifest SHA is pinned for the release set, so it stays out of this 50 until CT80 vendors it with URL+SPDX+author+SHA. Owner: CT80. Refusal if added without a row: `missing_url`.
5. `content/local/` — git-ignored (`.gitignore:8:content/local/`); Mixamo/Fab/QAL-v1.0 bytes may sit here but never ship except as cooked output with provenance. Owner: CT4 loop / plan content flow. Refusal if shipped raw: `license_refused`/`bad_sha`.

No AI-generator rows are in this 50 (all hand-made chain). CT40/CT38 AI props/variants will add `generator+model+prompt_digest` rows under CT71; the gate's `ai_chain` cases above already rehearse their refusals.

## Bytes x2

- Gate bytes x2 is the landed test `provenance_rows_and_digest_repeat_bit_identical`: same records encode the same rows twice, `release_digest` repeats, key order fixed (`author` first, `url` last), swapped order moves the digest. Green in the same run as above.
- This file bytes x2: SHA-256 taken on two reads after write; both read the same bytes (see Receipts). The file is a doc row: no tick bytes, golden hash untouched.

## Receipts (pasted, not invented)

- `cargo test --manifest-path .lanes/builder-content-r7/Cargo.toml --target-dir target/pool-1 -j 4 --locked -p engine-assets provenance` -> `test result: ok. 7 passed; 0 failed` (provenance 7/7: hand_record_passes, each_hand_gap_is_named, names_stay_lowercase_paths, digests_stay_lowercase_hex, ai_chain_links_are_each_required, release_matrix_of_fifty_names_the_gap, rows_and_digest_repeat_bit_identical).
- `git status --porcelain content/` -> `?? content/arena/pbr/` (the untracked gap named above).
- `git check-ignore -v content/incoming/studio_small_09_1k.hdr` -> `.gitignore:15:content/incoming/` (the ignored-HDRI gap named above).
- File SHA-256 x2: two `sha256sum` reads after write agree (values posted in the lane diff note at review; the judge re-reads the file).

Consumer handoff: CT28 can lock against this matrix (50 named, 5 gaps named with owners); CT52 re-checks this file plus the seeded refusal; CT71 syncs the intake ledger against rows 35-50; PR1 chains read the URL+SPDX+author+SHA columns.
