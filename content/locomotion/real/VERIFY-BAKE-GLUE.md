# CT37 piece B — bake-glue verify on the frozen synthetic bake

Row: CT37 (real-bytes locomotion intake, Tier A, Size M).
This file is ONE splitter piece, not the whole row: fresh-run proof that the
bake glue the first 16 real clips will flow through (bake x2 identical,
typed refusals, LM1 repeat read) is green on the frozen 8-clip synthetic
bake. No bytes fetched here; no real clips retargeted; `bake.json` /
`clips.json` frozen; piece A (`INTAKE-MANIFEST.md`) untouched.

No lane was opened for this piece: 78 live lanes against the 10-lane cap,
so a 79th worktree for a markdown-only receipt was refused as waste.
Work is content-only, zero crate hunks.

## 1. Fresh-run proof (this run, pasted)

Command (isolated `CARGO_TARGET_DIR`, `--locked`, contention with 3
queued builds; the shared-queue wait was 20 m, the build + run 34.5 s):

    cargo test --locked -p engine-anim --lib loco_bake -- --nocapture

Result: `test result: ok. 6 passed; 0 failed; 0 ignored`
(`264 filtered out`, `finished in 0.00s`).

Bake-x2 line pasted:

    CT3_BAKE clips=8 frames=360 entries=360 digest=0x8f7dc23e91f52395

Per-clip lines pasted (all match `content/locomotion/bake.json`):

    CT3_CLIP idle_stand frames=60 digest=0xf3497b3b844f48ed
    CT3_CLIP walk_forward frames=60 digest=0x0b4038167dc9c8a6
    CT3_CLIP run_forward frames=60 digest=0x5f4ea413a160279e
    CT3_CLIP start_walk frames=30 digest=0xcddbf202c3030b81
    CT3_CLIP stop_walk frames=30 digest=0xc783bba11cff2b51
    CT3_CLIP pivot_turn frames=30 digest=0x6926f3e84f8a4f03
    CT3_CLIP strafe_left frames=60 digest=0xdfa91ab2a76f9ddf
    CT3_CLIP turn_in_place frames=30 digest=0xead9ddc03cb419cd

`db_digest 0x8f7dc23e91f52395` equals the frozen `bake.json`
`db_digest`; `bake_identical_twice` asserts field equality, so the
Accept's "bake x2 identical" half holds for the glue.

## 2. Refusal matrix (same run, all ok)

| Test (`crates/anim/src/loco_bake.rs`) | Covers (per piece A s.4) |
|---|---|
| `unlicensed_source_refused` | Geno mesh `CC-BY-NC-4.0`, license mismatch |
| `unknown_source_refused` | `mixamo`, `lafan1` sources |
| `bad_specs_refused` | empty set, bad name, dup tags, bad ranges |
| `hero_set_bakes_with_counts_and_tags` | 8 clips / 360 frames / tags |
| `search_repeats_on_baked_db` | LM1 reads the bake: same mid-walk query twice returns `(clip 1, tick 30)` at cost 0 |

Geno/LAFAN1-shaped refusals green; LM1-reads-the-bake green.

## 3. CT1 retarget gate (source-verified, NOT re-run here)

`crates/assets/src/retarget.rs` (2217 lines): `retarget_clip`,
`retarget_cook`, `retarget_cook_with_sidecar`, `retarget_digest`, and the
gate test `three_clips_retarget_within_tolerance` with
`assert_within_tolerance`: rotation deltas within 1e-4 per component,
scaled offsets within 1e-5 per component, on every mapped joint and key.
A `cargo test --locked -p engine-assets --lib retarget` attempt this run
timed out in the shared queue (exit 124 after a 12 m slot wait, 13
waiting); no figure is claimed for it. The real-clip retarget + bind-pose
verify rides the fetch piece.

## 4. Remainder (explicitly NOT this piece)

- Fetch piece: download `100STYLE.zip`, verify md5, pin 16 names (piece A s.3).
- CT76 scout: 2 clips end to end (separate row, not claimed here).
- Retarget + bind-pose verify + sidecars for the 16; EVIDENCE row per file.
- `cmu` registry hunk + license decision (piece A s.2), then the CMU half.
- Re-run of the `engine-assets retarget` gate alongside the first real cook.
