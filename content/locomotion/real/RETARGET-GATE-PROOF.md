# CT37 piece C — retarget-gate proof for the first-16 intake

Row: CT37 (real-bytes locomotion intake, Tier A, Size M).
This file is ONE splitter piece, not the whole row: the CT1 retarget
tolerance gate plus the bind-pose checklist the first 16 real clips
must pass, with a fresh bake-glue re-proof from this run. No bytes
fetched here. No real clips retargeted. `bake.json` / `clips.json`
frozen. Pieces A (`INTAKE-MANIFEST.md`) and B (`VERIFY-BAKE-GLUE.md`)
untouched.

No lane was opened for this piece: 100+ live lanes against the
10-lane cap, so a new worktree for a markdown-only receipt was
refused as waste. Work is content-only, zero crate hunks.

## 1. Fresh-run proof (this run, pasted)

Command (heavy slot, 12m52s wait, 7.14s build plus run):

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
`db_digest`. `bake_identical_twice` asserts field equality, so the
Accept "bake x2 identical" half holds for the glue.

Same run, all ok:

- `unlicensed_source_refused` (Geno mesh `CC-BY-NC-4.0`, license mismatch).
- `unknown_source_refused` (`mixamo`, `lafan1`).
- `bad_specs_refused` (empty set, bad name, dup tags, bad ranges).
- `hero_set_bakes_with_counts_and_tags` (8 clips, 360 frames, tags).
- `search_repeats_on_baked_db` (LM1 reads the bake: same mid-walk
  query twice returns `(clip 1, tick 30)` at cost 0).

Geno and LAFAN1 shaped refusals green. LM1 read green.

## 2. CT1 retarget gate (source-verified, not re-run here)

Source: `crates/assets/src/retarget.rs` (read this run).

- Tolerances: rotations within 1e-4 per component, scaled offsets
  within 1e-5 per component, on every mapped joint and key
  (`:1652-1653`, `:1686`, `:1705`). Test
  `three_clips_retarget_within_tolerance` (`:1716`) checks idle,
  walk, run through `assert_within_tolerance`.
- Bind check: `validate_bind` (`:234-261`) refuses row-count
  mismatch (`BindMismatch`), non-finite positions or rotations
  (`NonFinite`), and non-unit quats (`BadBind`). Bad skeletons,
  maps, quats, and keys are typed `RetargetError`, never panic.
- Map: `BoneMap::build` (`:275-302`) resolves target and source
  names in pair order. Missing names give `MissingBone`.
  Repeated targets give `DuplicateBone`.
- Fallback: unmapped target joints hold the target bind pose on
  one key (`:263-269`, test `unmapped_joints_hold_bind` at
  `:1807`). Degenerate bone lengths retarget unscaled.
- Real-clip rule: each of the first 16 clips must pass
  `validate_bind` on source and target, then
  `assert_within_tolerance`, then cook with sidecar tags. That
  check rides the fetch piece, not this file.

A fresh `cargo test --locked -p engine-assets --features import
--lib three_clips_retarget_within_tolerance -- --nocapture`
attempt this run timed out in the shared queue (exit 124 after a
25s slot wait plus compile start, `Blocking waiting for file
lock on build directory`). No figure is claimed for it. Piece B
saw the same 12m timeout. The retarget re-run rides the fetch
piece.

## 3. Remainder (explicitly NOT this piece)

- Fetch piece: download `100STYLE.zip`, verify md5, list, pin 16 names.
- CT76 scout: 2 clips end to end (separate row, not claimed here).
- Retarget plus bind-pose verify plus sidecars for the 16.
- EVIDENCE row per file (URL plus SPDX plus author plus SHA).
- `cmu` registry hunk plus license decision (piece A s.2).
- Re-run of the `engine-assets` retarget gate with the first real cook.
- LM1 reads-the-bake proof over real bytes (CT69).
