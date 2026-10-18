# CT52 provenance enforce rehearsal

Row: CT52 content. Tier A. Size S. Owns `content/PROVENANCE-ENFORCE.md`.
Gate code stays CT28 (`crates/assets/src/provenance.rs`, landed). Matrix stays CT46
(`content/PROVENANCE-AUDIT.md`, `tools/provenance-audit.mjs`).
Needs: CT28 (lock, landed), CT46 (matrix). Consumer: CT28 lock, PR1 chains.

This is a rehearsal only. Nothing here turns the gate on for a release.

## 1. A seeded missing-provenance bundle is refused

The rehearsal is a test, so it runs on every checkout and repeats. Test
`CT52: a seeded 50-record bundle names each seeded gap and nothing else, the same twice`
in `tools/provenance-audit.test.mjs` takes the first 50 records of the real matrix
(all clean), then seeds three gaps at fixed slots in the bundle copy only (no repo
file is touched):

| slot | seeded gap | named refusal |
| --- | --- | --- |
| 7 | `url` set to empty | `missing_url` on `content/arena/src/debris.gltf` |
| 19 | `author` set to blanks | `missing_author` on `content/arena/textures/sand_normal_2k.jpg` |
| 33 | `sha` set to empty | `bad_sha` on `content/arena/textures/cloth_orm.png` |

The dry-run names every gap, not only the first, and names nothing else. The same
three gap kinds are covered on the Rust side by `provenance_each_hand_gap_is_named`
and `provenance_release_matrix_of_fifty_names_the_gap` in
`crates/assets/src/provenance.rs`.

## 2. The matrix is re-checked

`node tools/provenance-audit.mjs` on the current tree: 75 records (58 at CT46; the
15 cooked arena maps from CT74 and the 2 studio HDRI copies from CT80 are new),
3 refusals, all `bad_name`: the gate allows 64 characters and these names have more.

- `content/fighters/incoming/modular_rpg_characters_ik_rig_for_animating.7z`
- `content/fighters/glb/clips/human_male_idle_combat_main_hand_melee.glb`
- `content/fighters/glb/clips/human_male_cast_two_handed_melee_2.glb`

The four `sha_drift` findings CT52 saw on 2026-10-17 (`sand_floor.gltf`, `stands.gltf`,
`banner.gltf`, `placement.json`) are gone: CT82 re-pinned `content/arena/manifest.json`.
The 3 name findings are real and open for CT28 to see before the lock; the owner rows
are the fighter cooks (a rename moves a pinned file name, so it is not done here). A test
pins that no other refusal code appears in the real matrix.

## 3. Bytes x2

Test `CT52: ...` runs the seeded dry-run twice and `auditBody()` twice: both pairs
are equal. The generated block in `content/PROVENANCE-AUDIT.md` is checked by the
test `unlisted trees name an owner, and the generated block is current`.

## Proof

- Rehearsal and matrix: `node --test tools/provenance-audit.test.mjs` (9 tests).
- Rust: `cargo test --locked -p engine-assets provenance` on GitHub: PASS (run 39101397483).
