# CT31 splitter piece A — style-tag schema + synthetic bake proof

Row: CT31 (motion-DB style bake into LM1, Tier A, Size M).
This file is ONE splitter piece, not the whole row: style-tag schema plus
the bake-x2 + typed-refusal proof on the 8 synthetic stand-ins. Real
100STYLE + CMU bytes enter under CT37/CT48; the style-diverse bake and the
LM1 proof ride CT69. No bytes fetched here; nothing placed.

## 1. Style-tag schema

- Style tags ride beside locomotion tags in clip sidecars and bake rows.
- Charset follows the clip sidecar schema (`crates/anim/src/loco_bake.rs`
  `valid_token`): ASCII alphanumerics plus `_`, no leading digit, max 64
  chars. Unknown style ids are refused like unknown sources, never guessed.
- Closed Tier A vocabulary: `neutral` plus real 100STYLE style names as
  real clips land (CT37/CT48 file one EVIDENCE row per clip and extend the
  list here). Synthetic stand-ins carry `neutral` only — they prove the
  bake path, never a style.

## 2. Synthetic style assignments (this piece)

| Clip | Locomotion tag | Style | Source | License | Frames | Digest |
|---|---|---|---|---|---|---|
| idle_stand | idle | neutral | quaternius (synthetic) | CC0-1.0 | 60 | 0xf3497b3b844f48ed |
| walk_forward | walk | neutral | quaternius (synthetic) | CC0-1.0 | 60 | 0x0b4038167dc9c8a6 |
| run_forward | run | neutral | quaternius (synthetic) | CC0-1.0 | 60 | 0x5f4ea413a160279e |
| start_walk | start | neutral | 100style (synthetic, motions only) | CC-BY-4.0 | 30 | 0xcddbf202c3030b81 |
| stop_walk | stop | neutral | 100style (synthetic, motions only) | CC-BY-4.0 | 30 | 0xc783bba11cff2b51 |
| pivot_turn | pivot | neutral | 100style (synthetic, motions only) | CC-BY-4.0 | 30 | 0x6926f3e84f8a4f03 |
| strafe_left | strafe | neutral | quaternius (synthetic) | CC0-1.0 | 60 | 0xdfa91ab2a76f9ddf |
| turn_in_place | turn | neutral | quaternius (synthetic) | CC0-1.0 | 30 | 0xead9ddc03cb419cd |

Machine map: every row above is `{clip, style: neutral}`; real styles
replace `neutral` per clip only with a real-bytes EVIDENCE row.

## 3. Bake x2 proof (Accept half 1, pasted)

Command (lane `ct31-style-bake`, pool-2 flags):

    cargo test --manifest-path .lanes/ct31-style-bake/Cargo.toml --target-dir target/pool-2 -j 4 --locked -p engine-anim --lib loco_bake -- --nocapture

Result: 6 passed, 0 failed (`loco_bake::tests::*`). Bake line pasted:

    CT3_BAKE clips=8 frames=360 entries=360 digest=0x8f7dc23e91f52395

Per-clip digests match `content/locomotion/bake.json` (`db_digest`
`0x8f7dc23e91f52395`, 360 frames, `identical_twice: true`); the second
bake is field-equal to the first (`bake_identical_twice` asserts
`first == second`).

## 4. Typed refusals (Accept half 2, pasted)

Green in the same run, all typed `BakeError`, never a guessed output:

| Case | Test | Refusal |
|---|---|---|
| Geno mesh (`CC-BY-NC-4.0` on `100style`) | `unlicensed_source_refused` | `Unlicensed { geno_mesh }` |
| License mismatch (`CC-BY-4.0` on Quaternius `idle_stand`) | `unlicensed_source_refused` | `Unlicensed { idle_stand }` |
| `mixamo` source | `unknown_source_refused` | `UnknownSource { mixamo }` |
| `lafan1` source | `unknown_source_refused` | `UnknownSource { lafan1 }` |
| Empty set / 0 frames / duplicate / bad name / dup tags / NaN speed | `bad_specs_refused` | `BadSpec { clip, reason }` |

## 5. Consumer (LM1 reads the bake)

`motion_match::search` over the baked DB is the LM1 path:
`hero_set_bakes_with_counts_and_tags` builds a mid-walk query and asserts
the pick is `(clip 1, tick 30)`; `search_repeats_on_baked_db` asserts the
same query twice returns the same pick at cost 0. Both green in §3 run.

## 6. License and exclusions

CC0-1.0 Quaternius synthetic + CC-BY-4.0 100STYLE motions-only synthetic
with credit (`content/locomotion/ATTRIBUTION.md`, Zenodo 8127870 mirror).
Never ship: `Geno.fbx` mesh (non-commercial), LAFAN1 (CC BY-NC 4.0),
Unreal-only content — all refused per §4. `content/local/` uncommitted.

## 7. Remainder (explicitly NOT this piece)

- CT37/CT48: first/second 16 real clips with bind-pose verification.
- CT31 remainder: style-diverse bake over real clips into LM1.
- CT69: LM1 proof (style tags read, contacts fire on staged walk).
- Future: style-aware LM1 query weight (crate hunk, needs its own row).
