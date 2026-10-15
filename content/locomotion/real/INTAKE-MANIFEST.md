# CT37 piece A — real-intake license gate + fetch manifest

Row: CT37 (real-bytes locomotion intake, Tier A, Size M).
This file is ONE splitter piece, not the whole row: the license gate proof
plus the pinned fetch manifest for the first 16 real clips. No bytes fetched
here; nothing baked; `bake.json` / `clips.json` frozen.

## 1. Live license reads (2026-10-09, this run)

100STYLE (Mason, Starke, Komura 2022):

- Mirror: `https://zenodo.org/api/records/8127870` (record 8127870,
  DOI 10.1145/3522618, published 2022-05-04).
- Creators: Mason, Ian (MIT); Starke, Sebastian (Meta); Komura, Taku (HKU).
- License id on the record: `cc-by-4.0`.
- Files: `100STYLE.zip`, 1468515354 bytes,
  md5:3cf627852fd8192024c04a8d0ef49583 (original BVH);
  `100Style-Labelled-Data.zip`, 14751560934 bytes,
  md5:453378058265c217915e34f792ab6f58 (processed, not wanted).
- Dataset page `https://ianxmason.github.io/100style/` confirms over 4M
  frames across 100 locomotion styles, with Zenodo plus BVH-only Drive links.
- Ship rule: motions only, with credit. The `Geno.fbx` mesh is
  non-commercial and never ships (the bake refuses it, §4).

CMU Graphics Lab mocap (`http://mocap.cs.cmu.edu/faqs.php`, read live):

- Quote: "The motion capture data may be copied, modified, or redistributed
  without permission."
- No commercial-use clause and no no-resale clause on the FAQ page. The
  plan donor table carries CMU as commercial-use-allowed with no direct
  resale (`docs/plan/content.md:218`); that reading stands until a stronger
  source is read.
- Capture context on the page: 41 markers, 12 MX-40 cameras at 120 Hz,
  3 m x 8 m volume. The full DB ships as per-subject zips (c3d set over
  4 GB); the page asks users not to crawl.

## 2. Cook-gate finding (source-read this run)

`crates/anim/src/loco_bake.rs:63-69`: the bake registry holds exactly two
sources, `quaternius` and `100style`. `validate_spec` (`:338-350`) refuses
unknown sources and any license that does not equal the registry license.
There is no `cmu` source yet.

Consequence:

- The first-16 intake runs on 100STYLE motions only through the current
  gate. No crate hunk is needed for that half.
- CMU clips need a `cmu` registry hunk in `engine-anim` plus a license
  decision (the CMU grant is custom permissive, not CC0/CC-BY; the skill
  allows CC0/CC-BY in git, so CMU originals stay in git-ignored
  `content/local/` and ship only as cooked output). That hunk is a later
  CT37 piece with crate touch, not this file.

## 3. Fetch manifest (pinned, not executed)

- Fetch `100STYLE.zip` only (BVH originals). Pinned: 1468515354 bytes,
  md5:3cf627852fd8192024c04a8d0ef49583. Do not fetch the processed zip.
- Land originals in git-ignored `content/local/` (never committed).
  Convert BVH to glTF once in any tool, then cook through the CT1 retarget
  path: rotations within 1e-4 per component, scaled offsets within 1e-5
  (`crates/assets/src/retarget.rs:1652-1653`, test
  `three_clips_retarget_within_tolerance` at `:1716`).
- First-16 clip names get pinned at fetch from the zip listing. No names
  are listed here because none were read. Per-file EVIDENCE rows (URL plus
  SPDX plus author plus SHA) ride the fetch piece, one row per file.
- Record-level EVIDENCE proposal: 100STYLE dataset, Zenodo record 8127870,
  `https://zenodo.org/records/8127870`, Mason / Starke / Komura,
  CC-BY-4.0, record md5s in §1, per-file SHAs at fetch. Scope: motions-only
  locomotion; the Geno mesh is excluded.
- Recommended order: CT76 scout first (2 motions-only clips, owns
  `content/locomotion/SCOUT-REPORT.md`), then the remaining 14.

## 4. Refusal matrix (source-verified this run, last green per §5)

| Case | Test (`loco_bake.rs`) | Refusal |
|---|---|---|
| Geno mesh, `CC-BY-NC-4.0` on `100style` | `unlicensed_source_refused` (:651) | `Unlicensed { geno_mesh }` |
| License mismatch, `CC-BY-4.0` on Quaternius `idle_stand` | `unlicensed_source_refused` (:674) | `Unlicensed { idle_stand }` |
| `mixamo` source | `unknown_source_refused` (:687) | `UnknownSource { mixamo }` |
| `lafan1` source | `unknown_source_refused` (:709) | `UnknownSource { lafan1 }` |
| Empty set, bad name, dup tags, bad ranges | `bad_specs_refused` (:717) | `BadSpec { clip, reason }` |

QAL-v1.0 refusal coverage rides CT76, not this piece.

## 5. Baselines pasted (attributed, not re-run)

Synthetic bake (frozen, `content/locomotion/bake.json`):
`db_digest 0x8f7dc23e91f52395`, 8 clips, 360 frames, 360 entries,
`identical_twice: true`.
Landed receipt (`content/locomotion/STYLE-BAKE.md` §3, lane
`ct31-style-bake`):
`cargo test --locked -p engine-anim --lib loco_bake` gives 6 passed,
0 failed, with
`CT3_BAKE clips=8 frames=360 entries=360 digest=0x8f7dc23e91f52395`.
LM1 path (`loco_bake.rs:599-637`): `search_repeats_on_baked_db` asserts
the same mid-walk query twice returns `(clip 1, tick 30)` at cost 0.

No fresh `cargo test` ran in this piece: the shared runner held 4 busy
slots with 20 or more queued across two 20-minute waits, then the spawn
breaker opened. The bake-x2 re-run rides the fetch piece. No figure above
comes from a command that did not run.

## 6. Remainder (explicitly NOT this piece)

- Fetch piece: download `100STYLE.zip`, verify md5, list, pin 16 names.
- CT76 scout: 2 clips end to end with sidecars plus EVIDENCE plus scout bake.
- Retarget plus bind-pose verify plus sidecars for the 16; bake x2 identical.
- `cmu` registry hunk plus license decision, then the CMU half (CT48 side).
- LM1 reads-the-bake proof (CT69); style-diverse bake (CT31 remainder).
