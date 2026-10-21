# AM24 landing: the C1 clip sampler (anim side)

Row: AM24 anim: last-stage joint angular-speed limiter - 35-55 pops per 300
frames. Board: `docs/SPRINT-21-look-feel.md:302`. The row's own file is
`crates/anim/src/inertialize.rs`; its wiring is game side
(`crates/game/src/fighter_look.rs`), which this landing does not touch.

Scope: `crates/anim/**` and this receipt. No other crate, no `Cargo.toml`, no
content file changed.

## What the residual is

The board's 2026-10-20 progress line: "The residual is root/translation and
frame-cadence motion the bone cap does not cover". The game's own LF9 proof
names the same thing: "the 24 Hz keys of the run clip put a corner every 1.5
frames and the constant-velocity predictor misses it by 11 to 15 cm"
(`crates/game/src/fighter_look_proofs.rs:549`).

The fighter clips are keyed at 24 Hz and the sim ticks at 60 Hz. The pose
sampler read a key track with straight lines, so the sampled rate changed at
every key: the clip's own motion put a velocity corner in the pose, and the
arena's pop law (`crates/game/src/view_truth.rs`, `POP_M` 10 cm) reads a
corner as a clip snap. The bone cap cannot see it: the cap bounds the rate,
not the rate's change, and the clips' own rates sit under the 1080 deg/s cap
(the run clip's highest key pair asks for 849.0 deg/s), so the limiter passes
the corner through.

The first landing of this row read **both** lanes with a C1 curve
(commit `3b2d2b14`). The position lane is right; the rotation lane is not,
and it broke the row's own accept gate. This receipt lands the corrected
shape: the position lane keeps the C1 curve, the rotation lane goes back to
the straight shortest-arc read.

## What changed

- `crates/anim/src/pose.rs`: the **position** lane reads a key track with a
  C1 curve (unchanged from `3b2d2b14`): a cubic Hermite whose tangents are
  the neighbouring keys' central differences, zeroed where the tangent
  disagrees with the chord into or out of the key (a local extremum reads as
  a flat spot) and capped at three times the smaller of the two chord rates
  around it (`sample_track_c1`, `pose.rs:295`, `TANGENT_RATE_CAP` `pose.rs:163`).
  The key values are still hit exactly, the sampled value
  never leaves the key pair's span, and a track with fewer than three keys
  still reads as the straight line. The **rotation** lane reads the straight
  shortest-arc lerp again (`sample_quat_track`, `pose.rs:330`): the same sign
  flip, the same normalization, the same key search it has always had. The
  module doc records why the two lanes differ.
- `crates/anim/src/pose_c1_test.rs` (rewritten): the position lane's corner,
  key exactness and key span; the rotation lane's straight shortest arc
  (bit-exact against a test-local oracle); the rotation lane's rate against
  the key pair's own chord rate; the C1 rotation shape kept as a test-local
  oracle, so the rate guard demonstrably bites; the flipped key; the repeat
  and the short track.
- `docs/research/powerups-2026-10-20/landing-AM24.md` (this receipt).

The rotation C1 read (`sample_quat_track_c1`, `quat_tangent_at`, `quat_dot`,
`unit_quat`) is gone from `pose.rs`; the position C1 read (`sample_track_c1`,
`tangent_of`, `tangent_at`, `hermite`, `key_pair`, `TANGENT_RATE_CAP`) stays.

## The metric

The arena's own line, seed 2040, 600 ticks, 2 fighters
(`cargo test -p engine-game --features app --lib view_run_ -- --nocapture`),
before = the committed C1 tree (`3b2d2b14`), after = this landing:

| line | before (C1 on both lanes) | after (C1 position, straight rotation) |
|---|---|---|
| 2-fighter pops | **5** (`run:5`) | **0** (`none`) |
| 2-fighter pop_cm_max | 11.2 cm | 9.0 cm |
| 2-fighter idle_float_cm_max | 5.0 cm | 2.8 cm |
| accept gate `view_run_2040_600_has_no_joint_pops` | **FAILS** (`fighter_look.rs:5947`) | **PASSES** |

The 8-fighter headless line (the row's fighter count, the same seed and
command; the row's real product window is the GPU run, see "Still open"):

| line | before (C1 on both lanes) | after (C1 position, straight rotation) | pre-C1 baseline |
|---|---|---|---|
| pops | 2 (`attack:2`) | 2 (`attack:2`) | 2 (`attack:2`) |
| pop_cm_max | 12.5 cm | 13.1 cm | 13.1 cm |
| idle_float_cm_max | 2.7 cm | 2.4 cm | 2.3 cm |

The landing puts the 8-fighter line back on the pre-C1 numbers
(`pop_cm_max` 13.1 cm in both arms). The one number that moves is
`idle_float_cm_max`, by 0.1 cm in both directions: the 2-fighter line reads
2.9 -> 2.8 cm with the C1 position curve, the 8-fighter line reads
2.3 -> 2.4 cm with it. `tools/product-bar.json` has no floor on
`idle_float_cm_max` and no target on it either; the two lines move it by
opposite amounts, so the C1 position lane is neutral on it. The gated
numbers (`pops`, `pop_cm_max`) do not move at all.

Same-HEAD A/B (one HEAD, one tree each, only the anim files differ; the gate
test patched in the throwaway export tree to print its line when it passes):

| arm | 2-fighter `arena view:` | 8-fighter `arena view:` | run |
|---|---|---|---|
| baseline: HEAD with `3b2d2b14` reversed | `pops=0 pop_acts=none pop_cm_max=9.0 idle_float_cm_max=2.9` | `pops=2 pop_acts=attack:2 pop_cm_max=13.1 idle_float_cm_max=2.3` | 39553128177 |
| landing: HEAD plus this landing's anim files | `pops=0 pop_acts=none pop_cm_max=9.0 idle_float_cm_max=2.8` | `pops=2 pop_acts=attack:2 pop_cm_max=13.1 idle_float_cm_max=2.4` | 39553131996 |

Both arms: `8 passed; 0 failed; 805 filtered out`, and
`view_run_2040_600_has_no_joint_pops ... ok` in both. So at one HEAD the
landing moves **nothing** that is gated (`pops`, `pop_cm_max`) and moves
`idle_float_cm_max` by 0.1 cm down on the 2-fighter line and 0.1 cm up on the
8-fighter line. The gate test passes in the landing arm and fails in the
committed C1 tree (the "before" column above).

## The rule matrix: what decides the pops

Twenty-five cells, each one the same seed-2040 600-tick 2-fighter line with
one position rule and one rotation rule in the sampler (run 39546342238;
the matrix was measured on a throwaway public-export branch, never in the
private repo). `pops` / `pop_cm_max` / `idle_float_cm_max`:

| pos \ rot | straight | c1 | plain | bounded | dip |
|---|---|---|---|---|---|
| **straight** | **0** / 9.0 / 2.9 | 5 / 11.1 / 5.0 | 0 / 9.7 / 4.6 | 0 / 9.7 / 4.6 | 14 / 11.4 / 4.3 |
| **c1** | **0** / 9.0 / 2.8 | 5 / 11.2 / 5.0 | 0 / 9.7 / 4.5 | 0 / 9.7 / 4.5 | 14 / 11.4 / 4.3 |
| **plain** | **0** / 9.0 / 2.8 | 5 / 11.2 / 5.0 | 0 / 9.7 / 4.5 | 0 / 9.7 / 4.5 | 14 / 11.4 / 4.2 |
| **bounded** | **0** / 9.0 / 2.8 | 5 / 11.2 / 5.0 | 0 / 9.7 / 4.5 | 0 / 9.7 / 4.5 | 14 / 11.4 / 4.2 |
| **dip** | **0** / 9.0 / 2.8 | 5 / 11.1 / 5.0 | 0 / 9.7 / 4.5 | 0 / 9.7 / 4.5 | 14 / 11.4 / 4.3 |

Read down the `rot` columns: the **rotation** rule decides the pops, and
only the straight read is clean on every metric at once (pops 0, pop_cm_max
9.0, idle_float 2.8-2.9). Read across the `pos` rows: the position rule
does not change the pops or pop_cm_max at all; it moves idle_float by 0.1 cm
(2.9 -> 2.8 with the C1 position curve, the only positive it buys). The
matrix is decisive on its own: **no rule beats the straight rotation read on
pops, pop_cm_max and idle_float.**

## Why the rotation lane cannot be C1

The pop law is a constant-velocity predictor (`view_truth.rs:503-518`): it
holds the joint's last velocity and calls the gap between the joint and
`then + v*dt` a pop above 10 cm. So the read that wins is the one that keeps
the joint's world path on the line the predictor holds.

A per-segment Hermite cannot do that on the rotation lane. The mean value
theorem says the Hermite's rate somewhere inside the segment is at least the
segment's own chord rate, and the measured peak says by how much. The guard
in `pose_c1_test.rs` measures, inside one key pair, the ratio of the sampled
per-tick rate to the key pair's chord rate:

| read | worst ratio of the chord's rate (measured, the run clip's `leg_lower_R` keys) |
|---|---|
| straight shortest arc (the lane) | **1.0025** |
| per-segment Hermite (the oracle) | **1.6429** |

The fixture's own key pairs run at 316.7, 444.0, 107.1, 34.1, 132.6 and
61.3 deg/s, all under the 1080 deg/s cap, which is why the bone cap never
saw the corner.

The guard bound is `CHORD_RATE_BOUND = 1.1` (`pose_c1_test.rs:64`), between
the two. `the_rotation_rate_never_runs_past_the_chord` holds the lane under
it; `a_curved_rotation_read_runs_past_the_chord` holds the C1 shape over it,
so the C1 curve stays in the file as the oracle the lane must not read as.

Two earlier stories this landing corrects, both measured:

- The "a sign-flipped key pair reads as a 360 degree spin" story is a probe
  artifact. The repo's own `nlerp` (`pose.rs:136`) already flips to the
  shortest arc, and commit `3b2d2b14` did not change `nlerp`; the matrix's
  "straight" cell is the shortest-arc read. The flip test now compares the
  sampled rotation against the pair's own angle, not against a raw length.
- The straight rotation read hands a key back unchanged at the clip's edge
  (raw, not normalized), and the fixture's keys are about 4e-5 off unit, so
  the unit-sphere assertion compares against the key's own length.

## Proof

Every line below was run on the GitHub runner against this working tree
(`tools/gh-check.mjs`, exit 0 = pass, 1 = fail, 3 = still running). The
anim lines ran with `--only crates/anim/src/pose.rs,crates/anim/src/lib.rs,crates/anim/src/pose_c1_test.rs`.

| command | result | run |
|---|---|---|
| `cargo test -p engine-anim --lib` | `474 passed; 0 failed; 5 ignored` | 39552153191 |
| `cargo test -p engine-anim --lib ang_limit` | `2 passed; 0 failed; 477 filtered out` | 39552038031 |
| `cargo test -p engine-anim --lib ang_limit` | `2 passed; 0 failed; 477 filtered out` | 39552251971 |
| `cargo test -p engine-anim --lib pose_c1 -- --nocapture` | `10 passed; 0 failed; 469 filtered out` | 39552514754 |
| `cargo clippy -p engine-anim --all-targets -- -D warnings` | clean, exit 0 | 39552795465 |
| `cargo test -p engine-proof golden_is_unchanged` | `1 passed; 0 failed` (`fb_pillars::tests::golden_is_unchanged ... ok`) | 39552961963 |
| `cargo test -p engine-game --features app --lib view_run_ -- --nocapture` | `8 passed; 0 failed; 805 filtered out` (the 2-fighter gate passes, the 8-fighter line prints `pops=2 pop_acts=attack:2 pop_cm_max=13.1 idle_float_cm_max=2.4`) | 39551755972 |
| same command, same-HEAD A/B arm "baseline" (public mirror, `wip-am24-final-baseline`) | `8 passed; 0 failed; 805 filtered out`, 2-fighter `pops=0 pop_acts=none pop_cm_max=9.0 idle_float_cm_max=2.9`, 8-fighter `pops=2 pop_acts=attack:2 pop_cm_max=13.1 idle_float_cm_max=2.3` | 39553128177 |
| same command, same-HEAD A/B arm "landing" (public mirror, `wip-am24-final-landing`) | `8 passed; 0 failed; 805 filtered out`, 2-fighter `pops=0 pop_acts=none pop_cm_max=9.0 idle_float_cm_max=2.8`, 8-fighter `pops=2 pop_acts=attack:2 pop_cm_max=13.1 idle_float_cm_max=2.4` | 39553131996 |

The golden tick hash `d57b5c8ae3096614` (`GOLDEN_600`,
`crates/proof/src/fb_pillars.rs:400`) does not move.

The rule matrix and the diagnostic A/B above ran on throwaway branches of the
public mirror (`maximkone/engine2040-public`), never in the private repo:
the matrix on `wip-am24-rules` (run 39546342238), the before/after miss
sweep on `wip-am24-diag-before` / `wip-am24-diag-after` (700 misses over
4 cm and 4 over 10 cm before, 708 and 9 after, the 5 new ones all in the run
clip's `joint=16`), and the same-HEAD A/B of this receipt on
`wip-am24-final-baseline` / `wip-am24-final-landing` (runs 39553128177 and
39553131996).

## Still open

- The row's own product window is not mine to close. `node tools/product-check.mjs`
  builds `target/release/arena.exe` from the private working tree, and that tree
  currently carries other lanes' in-flight edits (`crates/app`, `arena`,
  `endpoint`, `game`, `perf`, `ui`, `web`, `input`, `playtest`, `spatial`), so a
  run now would measure a mixed tree, not this landing. The row stays open until
  its own `latest.md` prints 0.
- The `pops=0 pop_acts=none pop_cm_max=10.0` line the board quotes is a
  **pre-C1** measurement. It comes from
  `target/reports/product/manual-6eafae93/look.txt`, whose sha is `6eafae93`
  (2026-10-20T20:55Z), which is an ancestor of the C1 commit `3b2d2b14`
  (2026-10-21T03:14Z). The newest product-window run, sha `21131a5a`
  (2026-10-21T05:23Z, after the C1 commit), reads `pops=2
  pop_acts=attack:1/run:1 pop_cm_max=10.4 idle_float_cm_max=20.5`
  (`target/reports/product/runs/21131a5a-20261021T0523Z/look.txt`). That tree is
  101 first-parent commits past `6eafae93` (physfx, rules, assets, spatial,
  physics and more), so the pair is not a clean A/B and does not attribute
  those two pops to the anim lane; it does mean the row's window is not closed
  by either arm, and the landing is measured on the headless line instead.
- The run clip's **loop seam** is a 27.5 cm hand jump
  (`content/fighters/glb/clips/human_male_run.glb`: the keys run from 1/24 s
  to 19/24 s, so the wrap skips a full key interval; the worst joint moves
  27.5 cm, twelve joints move more than 0.5 cm). It is identical in both
  arms, so it does not show up as a pop, but it is a clip defect: it wants a
  per-clip loop flag in the assets and the wrap in the game crate.
- There is **no translation limiter** beside `AngularLimiter`
  (`crates/anim/src/inertialize.rs`), so the root/translation part of the
  row's residual is still uncovered. This landing does not add one.
- The `Fallen` knockdown still snaps: the sim's `Fallen` verdict lays a body
  flat (EG45), and the limiter bounds the rate of the local rotations, not
  that swap. The game's own LF9 note calls it "the one idle pop left".
- The death clip asks for more than the cap: `human_male_death.glb`'s
  highest key pair is 1198.9 deg/s (`arm_lower_R`, 49.95 deg over 41.7 ms)
  against the 1080 deg/s cap, so the limiter has to absorb it.
- `idle_float_cm_max` has no floor and no target in
  `tools/product-bar.json`; the C1 position lane moves it by 0.1 cm in
  opposite directions on the two lines, so it is left as it is.
