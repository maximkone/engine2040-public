# CT52 provenance enforce rehearsal

Row: CT52 content. Tier A. Size S. Owns `content/PROVENANCE-ENFORCE.md`.
Gate code stays CT28 (`crates/assets/src/provenance.rs`). Matrix stays CT46
(`content/PROVENANCE-AUDIT.md`, `tools/provenance-audit.mjs`).
Needs: CT28 (lock), CT46 (matrix). Consumer: CT28 lock, PR1 chains.

This is a rehearsal only. Nothing here turns the gate on for a release.

## 1. Seeded missing-provenance bundle is refused

Bundle: the first 50 shipped records from the CT46 matrix. Three gaps were
seeded at fixed slots (not in the repo files, only in the rehearsal bundle):

| slot | seeded gap | named refusal |
| --- | --- | --- |
| 7 | `url` set to empty | `missing_url` on `content/arena/src/debris.gltf` |
| 19 | `author` set to blanks | `missing_author` on `content/arena/textures/sand_normal_2k.jpg` |
| 33 | `sha` set to empty | `bad_sha` on `content/arena/textures/cloth_orm.png` |

The dry-run names every gap, not only the first one. Result for the 50-record
bundle: 9 refusals. The 3 seeded gaps are named above. The other 6 come from
the real matrix, not from the seed:

- `sha_drift` on `content/arena/src/sand_floor.gltf`, `content/arena/src/stands.gltf`,
  `content/arena/src/banner.gltf`, `content/arena/placement.json` (file on disk no longer
  matches the manifest SHA).
- `bad_name` on `content/fighters/incoming/modular_rpg_characters_ik_rig_for_animating.7z`
  and `content/fighters/glb/clips/human_male_idle_combat_main_hand_melee.glb`
  (names longer than the 64-char gate limit).

Rust side: the gate tests `provenance_each_hand_gap_is_named` and
`provenance_release_matrix_of_fifty_names_the_gap` in `crates/assets/src/provenance.rs`
cover the same three gap kinds (empty URL, blank author, short SHA) and the
named asset on a 50-record release.

## 2. 50-asset matrix re-checked

`node tools/provenance-audit.mjs` on the current tree:

- 58 records in the matrix (CT46 said 58 rows, 7 refusals named). Still 58 and 7.
- Full-matrix refusals: `sha_drift` (4), `bad_name` (3). No missing URL, author, or SPDX.
- The generated block in `content/PROVENANCE-AUDIT.md` is current (no stale block).
- `node --test tools/provenance-audit.test.mjs`: 5 of 5 pass.

Those 7 are real findings for CT28 to see before the lock. They are not fixed here
(the owner rows for the drifted files are CT46 and the fighter/arena cooks).

## 3. Bytes x2

The rehearsal script ran twice on the same tree. Both outputs have the same
SHA-256 (`7220c976e1feb18eca7e92148fcb588270727beab30e933c63e1658089cdddcd`).

## 4. Rust proof

`cargo test --locked -p engine-assets provenance` runs on GitHub via
`node tools/gh-check.mjs`. Result: PASS, run 39064454169.

## Proof

- Rehearsal: 2 runs, identical bytes (section 3).
- Dry-run: section 1 and 2 above.
- Rust: `cargo test --locked -p engine-assets provenance` on GitHub: PASS (run 39064454169).
