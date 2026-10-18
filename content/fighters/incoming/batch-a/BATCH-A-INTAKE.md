# CT42 batch-a INTAKE note — first half (8 of 16 clips)

Run: builder-content-r1, 2026-10-09. Splitter piece of Size-M CT42
(Tier A): source re-acquisition + first-half bake with provenance.
Second half + CT1 cook proof land in the follow-up.

r3 addendum: builder-content-r3, 2026-10-09. Splitter piece of Size-M
CT42 (Tier A): re-export (identity-scale fix) + first-half CT1 cook
proof with receipt. Owns only batch-a/ files touched below; second
half (8 clips) + eyes stay for the follow-up.

## Source (verified live this run)

- Pack: https://opengameart.org/sites/default/files/modular_rpg_characters.7z
- Page badge re-read live 2026-10-09: `License(s): CC0`
  (creativecommons.org/publicdomain/zero/1.0/). SPDX: CC0-1.0.
- Author: System G6 (aka Qoma), same as CT7.
- Downloaded bytes: 16600670, matches manifest.json pin exactly.
- SHA256: ef53003c6b29798a634fb27a3f9fc3181a2c02b217689744f2944fc5fcc0fd99,
  matches manifest.json pin exactly.
- Local copies (gitignored, never tracked): content/local/modular_rpg_characters.7z
  + content/local/mrc/modular_rpg_characters/{human_male.blend,human_female.blend}.
- Action census of human_male.blend (Blender 5.2 headless): 50 actions.
  14 already curated by CT7. Batch-a takes 16 new combat actions.

## First-half bake (8 clips, armature-only GLB, CT7 export shape)

r3 re-export (this run, Blender 5.2.0 LTS, one process per clip):
identity scale fcurves dropped in-Blender (worst |v-1| 2.1e-06, gate
1e-4), then identity scale channels stripped from the GLB (worst
sampled |v-1| 4.7e-06, gate 1e-4). Result is the canonical shape:
86 channels (43 translation + 43 rotation, 0 scale) with 129 samplers,
exactly like the CT7 canonical clips.

| short | source action | glb bytes | glb sha16 |
|---|---|---|---|
| attack_dual_2 | human_male_cast_dual_wield_melee_2 | 54052 | 2009192052bc9219 |
| attack_dual_3 | human_male_cast_dual_wield_melee_3 | 55764 | 3951a985d1da422b |
| attack_dual_4 | human_male_cast_dual_wield_melee_4 | 54052 | 93a39d3de5314791 |
| attack_2h_3 | human_male_cast_two_handed_melee_3 | 54052 | 1b7e5755fd8423ed |
| attack_2h_4 | human_male_cast_two_handed_melee_4 | 54052 | 0b5f5a9af2b42997 |
| attack_2h_5 | human_male_cast_two_handed_melee_5 | 54052 | 94ce6013c15968e7 |
| attack_bow_1 | human_male_cast_two_handed_bow | 54048 | 0513f6c78860de1d |
| attack_bow_2 | human_male_cast_two_handed_bow_2 | 54052 | 8d136204eb63203b |

All 8 sha16 distinct. Sizes sit with the canonical family (54-71 KB).
Sidecars in batch-a/sidecars/ (207-265 bytes each, canonical schema:
loop false, root false, ground_y 0.02, feet foot_l/foot_r; melee clips carry
one SOCKET_wep window, bow clips tags attack/ranged with no window).

## Blender finding (affects bake method, not CT7 bytes)

Baking all 8 in one Blender session exported STALE animation for 5 clips
(5 byte-identical GLBs). Baking each action in its own Blender invocation
gives 8 distinct, correct clips. Batch-b and any re-bake MUST use
one-process-per-clip. CT7's 14 canonical GLBs are unaffected (all 14 cook
digests differ).

r3 scale finding (root cause of the cook refusal): these 8 source actions
carry scale fcurves on every bone (90-126 curves per action), all identity
noise (worst |v-1| 2.1e-06). Blender 5.2 exports them as 43 scale channels,
which the CT1 cook refuses by design (typed UnsupportedTarget "scale";
CT7's 14 actions export without scale channels). Fix keeps motion intact:
drop identity scale fcurves in-Blender (gate 1e-4, measured worst 2.1e-06),
export, then strip identity scale channels from the GLB (gate 1e-4,
measured worst 4.7e-06). Trans/rot tracks untouched; result matches the
canonical 86-channel shape. The r1 sha16 column above is superseded by the
r3 table.

## Row Accept status (honest, no invented figures)

- `cargo test --locked -p engine-assets retarget`: exit 0, but 0 tests matched
  (0 passed, 0 failed; integration targets also 0 matched). engine-assets has
  NO retarget unit tests, so this gate is vacuous. CT24/CT59 should own a real
  retarget gate; the 14 canonical digests in DEDUPE-MANIFEST.md are the dedupe
  set the cook checks against.
- Bind pose: hero bind reused unchanged from CT7 (validate_bind green in CT7
  proof; same rig, same armature). Not re-run here.
- Cook x2 + digest dedupe (r3, this run): driver at batch-a/cook/src/main.rs
  (content-owned standalone crate; bind verify, retarget_cook_with_sidecar x2
  identical, dedupe vs the 14 canonical cook digests + pairwise, finite-value
  and non-empty checks, writes BATCH-A-RECEIPT.md). `cargo build --locked`
  exit 0; `cargo run --locked` exit 0 twice with identical output:
  CT42-BIND joints=45 bind=valid, then 8 CT42-CLIP lines (dur 0.417 s,
  keys 432 except attack_dual_3 keys 475, all cook_x2=identical), then
  CT42-OK clips=8 dedupe=clean cook_x2=identical. Cook digests: dual_2
  abddab3a78895c56, dual_3 27f8b7f5968ad6c7, dual_4 e4f414cfcb5a316c,
  2h_3 12d35aeb55546991, 2h_4 db7d8fc406632b39, 2h_5 11f05faaf7103797,
  bow_1 27757541ee501c2a, bow_2 c4ab161dc864997e — all 8 distinct and
  none matching the 14 canonical digests (driver refuses dupes, exit 0).
  BATCH-A-RECEIPT.md SHA256
  F5A6C03BA6A4FE4252A505521B5FFD537E5DDB8BBB335C491B0F7B1505954E47,
  byte-identical across both runs.
- Budgets: clip GLB bytes hold (54048-55764, canonical family 53984-71260).
  Keys/durations from the cook: 0.417 s, 432 keys (475 for dual_3, whose
  action spans frames 0-10).
- Bind pose: hero bind validate_bind green re-run here (CT42-BIND joints=45
  bind=valid on the CT7 hero_body.glb, same rig).
- Provenance: per-file rows below; no new downloads beyond the pinned CC0 7z.

## EVIDENCE.md row proposals (one per baked file)

Source for all: url https://opengameart.org/sites/default/files/modular_rpg_characters.7z,
SPDX CC0-1.0, author System G6, upstream sha
ef53003c6b29798a634fb27a3f9fc3181a2c02b217689744f2944fc5fcc0fd99.

- batch-a/glb/human_male_cast_dual_wield_melee_2.glb (sha16 2009192052bc9219):
  baked FK action frame 1-10, armature-only export, per-process bake,
  r3 identity-scale clean (86 channels).
- batch-a/glb/human_male_cast_dual_wield_melee_3.glb (sha16 3951a985d1da422b):
  baked FK action frame 0-10, armature-only export, per-process bake,
  r3 identity-scale clean (86 channels).
- batch-a/glb/human_male_cast_dual_wield_melee_4.glb (sha16 93a39d3de5314791):
  baked FK action frame 1-10, armature-only export, per-process bake,
  r3 identity-scale clean (86 channels).
- batch-a/glb/human_male_cast_two_handed_melee_3.glb (sha16 1b7e5755fd8423ed):
  baked FK action frame 1-10, armature-only export, per-process bake,
  r3 identity-scale clean (86 channels).
- batch-a/glb/human_male_cast_two_handed_melee_4.glb (sha16 0b5f5a9af2b42997):
  baked FK action frame 1-10, armature-only export, per-process bake,
  r3 identity-scale clean (86 channels).
- batch-a/glb/human_male_cast_two_handed_melee_5.glb (sha16 94ce6013c15968e7):
  baked FK action frame 1-10, armature-only export, per-process bake,
  r3 identity-scale clean (86 channels).
- batch-a/glb/human_male_cast_two_handed_bow.glb (sha16 0513f6c78860de1d):
  baked FK action frame 1-10, armature-only export, per-process bake,
  r3 identity-scale clean (86 channels).
- batch-a/glb/human_male_cast_two_handed_bow_2.glb (sha16 8d136204eb63203b):
  baked FK action frame 1-10, armature-only export, per-process bake,
  r3 identity-scale clean (86 channels).
- batch-a/sidecars/*.json (8 files): hand-made by this lane, CC0-1.0,
  engine2040 CT42 lane author, change: new hit-window/loop/root tags per clip.
- batch-a/cook/src/main.rs + Cargo.toml: new tool by this lane (not shipped
  bytes), mirrors the clip leg of content/fighters/tools/ct7proof/src/main.rs.

r5 addendum: builder-content-r5, 2026-10-09. Splitter piece of Size-M
CT42 (Tier A): second-half bake (8 clips) + sidecars + full 16-clip CT1
cook proof with receipt. Owns only batch-a/ files touched below; visual
eyes stay for the follow-up (CT24 audit).

## Second-half bake (8 clips, armature-only GLB, CT7 export shape)

Baked with content/fighters/tools/clips.py, one Blender 5.2.0 LTS process
per clip (the r1 stale-animation finding: per-process bake is mandatory).
Blender 5.2 uses layered actions (no Action.fcurves), so the r3
"drop in-Blender" step has no 5.2 equivalent; the effective clean is the
post-export strip below, with the sampled gate proving the scale tracks
were identity noise. Trans/rot tracks untouched.

Pre-strip shape per GLB: 43 trans + 43 rot + 43 scale channels.
Scale sample gate (1e-4): worst |v-1| 4.29e-06 (1290 samples each for the
five 10-frame attacks; 2451 samples each for the two 19-frame idles;
power_up_2 worst 4.05e-06 over 1419 samples). All 8 PASS.
Stripped 43 scale channels per file (node rewrite, JSON + BIN preserved,
BIN untouched). Post-strip shape, verified over all 16 batch-a GLBs:
86 channels (43 translation + 43 rotation, 0 scale), exactly the CT7
canonical shape.

| short | source action | frames | glb bytes | glb sha16 |
|---|---|---|---|---|
| attack_xbow_1 | human_male_cast_two_handed_crossbow | 1-10 | 54000 | 566bd890d1a82eda |
| attack_staff_1 | human_male_cast_two_handed_staff | 1-10 | 53996 | 312dc145957fa364 |
| attack_wand_1 | human_male_cast_main_hand_wand | 1-10 | 53992 | 1352db0f65a92758 |
| attack_unarmed_1 | human_male_cast_unarmed_magic | 1-10 | 53992 | 2a723ce776758d0a |
| attack_unarmed_2 | human_male_cast_unarmed_magic_2 | 0-9 | 53964 | db6ba03b817954dc |
| idle_combat_dual | human_male_idle_combat_dual_wield_melee | 1-19 | 69556 | 25ca9f18e74c48fa |
| idle_combat_2h | human_male_idle_combat_two_handed_melee | 1-19 | 69556 | 43d046d559bc4b9c |
| power_up_2 | human_male_power_up_2 | 0-10 | 55696 | 5ad92945611324f2 |

All 8 sha16 distinct and distinct from the first-half 8. Byte sizes sit
with the canonical family (first-half 54048-55764; canonical 53984-71260;
idles 69556 match the canonical idle pair 69548-69556).
Sidecars in batch-a/sidecars/ (canonical schema: loop, root, ground_y
0.02, feet foot_l/foot_r): xbow/wand tags attack/ranged, no window;
staff one SOCKET_wep_R window; unarmed x2 tags attack/melee, no window
(no socket on fists); idle_combat x2 loop true tags idle/combat;
power_up_2 tags taunt.

## Full cook proof (16 clips, this run)

Driver at batch-a/cook/src/main.rs (extended CLIPS 8 -> 16; dedupe set
unchanged: the 14 canonical cook digests + pairwise inside the batch).
`cargo build --locked` exit 0; `cargo run --locked` exit 0 twice:
CT42-BIND joints=45 bind=valid, then 16 CT42-CLIP lines, all
cook_x2=identical; CT42-OK clips=16 dedupe=clean cook_x2=identical.
First-half 8 digests reproduce the r3 receipt exactly (no regression).
Second-half cook digests: xbow_1 eb67ceeb8d16cc02, staff_1
c9ef7e932135d20b, wand_1 16d3bb6544e3022c, unarmed_1 088df2afa4627669,
unarmed_2 0c7c230e161374c3, idle_dual 0a1b66f185a22832, idle_2h
ce829c5930905657, power_up_2 9917b956e8407d4f (dur 0.417 s keys 432
except unarmed_2 0.375 s keys 432, idle pair 0.792 s keys 819 loop=true,
power_up_2 0.417 s keys 475). All 16 distinct, none matching the 14
canonical digests (driver refuses dupes, exit 0).
BATCH-A-RECEIPT.md SHA256
FFA3BDA90361288265072C44A4C8B13EA79CEF00F2ACAD54BADA64DF4D4AF236,
byte-identical across both runs.

## Row Accept status (honest, no invented figures)

- `cargo test --locked -p engine-assets retarget`: exit 0, 0 tests
  matched (vacuous gate, same as r3; engine-assets still has NO retarget
  unit tests; CT24/CT59 should own a real retarget gate).
- Bind pose: hero bind validate_bind green (CT42-BIND joints=45
  bind=valid, CT7 hero_body.glb, same rig).
- Cook x2 + digest dedupe: 16 clips, dedupe clean vs the 14 canonical
  cook digests + pairwise, finite-value and non-empty checks, receipt
  byte-identical x2 (SHA above).
- Budgets: GLB bytes 53964-69556 (canonical family 53984-71260; the two
  idle 69556 match the canonical idle shape 0.792 s / 819 keys).
- Provenance: per-file rows in this note + the r3 rows above; no new
  downloads beyond the pinned CC0 7z (same URL/author/SHA as r1).

## EVIDENCE.md row proposals (second-half files)

Source for all: url https://opengameart.org/sites/default/files/modular_rpg_characters.7z,
SPDX CC0-1.0, author System G6, upstream sha
ef53003c6b29798a634fb27a3f9fc3181a2c02b217689744f2944fc5fcc0fd99.

- batch-a/glb/human_male_cast_two_handed_crossbow.glb (sha16 566bd890d1a82eda):
  baked FK action frame 1-10, armature-only export, per-process bake,
  r5 scale-strip clean (86 channels).
- batch-a/glb/human_male_cast_two_handed_staff.glb (sha16 312dc145957fa364):
  baked FK action frame 1-10, armature-only export, per-process bake,
  r5 scale-strip clean (86 channels).
- batch-a/glb/human_male_cast_main_hand_wand.glb (sha16 1352db0f65a92758):
  baked FK action frame 1-10, armature-only export, per-process bake,
  r5 scale-strip clean (86 channels).
- batch-a/glb/human_male_cast_unarmed_magic.glb (sha16 2a723ce776758d0a):
  baked FK action frame 1-10, armature-only export, per-process bake,
  r5 scale-strip clean (86 channels).
- batch-a/glb/human_male_cast_unarmed_magic_2.glb (sha16 db6ba03b817954dc):
  baked FK action frame 0-9, armature-only export, per-process bake,
  r5 scale-strip clean (86 channels).
- batch-a/glb/human_male_idle_combat_dual_wield_melee.glb (sha16 25ca9f18e74c48fa):
  baked FK action frame 1-19, armature-only export, per-process bake,
  r5 scale-strip clean (86 channels).
- batch-a/glb/human_male_idle_combat_two_handed_melee.glb (sha16 43d046d559bc4b9c):
  baked FK action frame 1-19, armature-only export, per-process bake,
  r5 scale-strip clean (86 channels).
- batch-a/glb/human_male_power_up_2.glb (sha16 5ad92945611324f2):
  baked FK action frame 0-10, armature-only export, per-process bake,
  r5 scale-strip clean (86 channels).
- batch-a/sidecars/{attack_xbow_1,attack_staff_1,attack_wand_1,attack_unarmed_1,attack_unarmed_2,idle_combat_dual,idle_combat_2h,power_up_2}.json
  (8 files): hand-made by this lane, CC0-1.0, engine2040 CT42 lane author,
  change: new hit-window/loop/root tags per clip.
- batch-a/cook/src/main.rs: extended by this lane (CLIPS 8 -> 16, receipt
  16 of 16); not shipped bytes.

## Remainder (follow-up run)

1. DONE r5: second-half bake + sidecars, full 16-clip cook/dedupe
   finished, CT42-CLIP digest lines pasted above, BATCH-A-RECEIPT.md
   written and byte-identical x2.
2. Visual eyes check per cooked clip (silhouette vs 1.8 m reference) —
   still not done (headless bake only; clips are armature-only GLBs, a
   still render would show bones, not the skinned silhouette).
3. CT42 closes at 16 of 16 pending lead tick: needs CT1 + CT7 (both
   ticked), consumer CT23 receipt / CT24 audit.

## Open gates

- No retarget unit tests in engine-assets (Accept command vacuous).
- Visual preview of the 16 clips not rendered.
- Lead tick to close CT42 at 16 of 16.

## CT42 closure (2026-10-18)

The batch is vendored and held by tests instead of the standalone driver.

- Vendoring: `.gitignore` now keeps `content/fighters/incoming/batch-a/` (16 GLB,
  16 sidecars, this note, the receipt); the driver crate `cook/` and `recheck/`
  stay local scratch.
- Retarget test: `retarget_batch_a_sixteen_clips_cook_twice_deduped_and_match_the_receipt`
  (`crates/assets/tests/it/retarget_batch_a.rs`) takes the hero bind from
  `glb/hero_body.glb` (`validate_bind`), cooks each clip through
  `retarget_cook_with_sidecar` twice (identical), refuses a digest that repeats one of the 14
  canonical cook digests or an earlier batch-a digest, and compares glb bytes, duration,
  keys, loop and root-motion flags with the receipt rows. The Accept line
  `cargo test --locked -p engine-assets retarget` matches 0 tests unless `--features import`
  is on (the whole retarget module is behind it): the proof line is
  `cargo test --locked -p engine-assets --features import,registry --test it -- retarget_batch_a`.
  Result on the runner: PASS, run 39105798540. The Accept line with the feature on,
  `cargo test --locked -p engine-assets --features import retarget` (20 retarget lib tests
  plus this one), PASS, run 39106737518. The whole `engine-assets` suite
  (`--features import,registry --no-fail-fast`) PASS, run 39107761835.
- Platform note: two cook digests (`attack_2h_3`, `power_up_2`) differ between the Windows
  cook that wrote the receipt and a Linux runner (a float difference between platforms: the
  same files cook to the receipt digests on this PC); every other column and 14 of 16 digests
  agree. The test compares the digest column on Windows only
  (run on this PC 2026-10-18 with `cargo test --features import --test it retarget_batch_a`:
  PASS, all 16 digests equal the receipt) and everything else on every platform.
- Provenance per file: `node --test tools/batch-a.test.mjs` pins each GLB to the sha16 in
  this note and checks one sidecar per clip, named after its clip, with the foot bones.
- Still open: the visual eyes check above (armature-only GLBs show bones, not the silhouette).
