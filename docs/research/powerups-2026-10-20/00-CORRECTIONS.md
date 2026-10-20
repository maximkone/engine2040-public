# Corrections: what our docs and our board get wrong today (2026-10-20)

Written by the lead while the powerup wave ran. Every line is a claim that is in a doc or on the board and is
false or stale as of HEAD `77789c2a`. Each line names the file and line so it can be checked in one grep.

Why this file exists: the board holds 292 open rows. A row that asks for work which is already done, or that
points at a file that does not exist, spends a session and moves no number. Nine lanes read the tree and found
the same class of defect repeatedly. Fix the rows once, here.

Evidence grade: `lead` = the lead checked it today with a grep or a glob. `lane` = a named lane read it live and
cited the path; the lead did not re-run it.

## A. Stale claims in our own docs

| # | The claim | Where | The truth | Evidence | Action |
|---|---|---|---|---|---|
| A1 | `nav::crowd` (ORCA) is dead code | `docs/plan/forge-adoption.md` A.3 item 5 | There is no `nav::crowd`. The ORCA code is `crates/nav/src/orca.rs` (971 lines, 0 callers). The live crowd cue is `crates/audio/src/crowd.rs` (512 lines, 6 intra-crate callers + `crates/game/src/audio_director.rs:17`) | lead (glob: 14 files in `crates/nav/src`, `crowd.rs` absent) + lane | correct the audit; decide orca: wire (NV4/NV10) or delete |
| A2 | `assets::import_repair` has no production caller | same A.3 item 5 | It is wired: `crates/render/src/v1.rs:256` `engine_assets::import_repair::recompute_tangents(&proxy)` and `crates/render/src/present/gpu_present/presenter.rs:2197` | lane | correct the audit; drop the row |
| A3 | `engine-playtest` is depended on by nobody | same A.3 item 5 | It has a consumer: `crates/endpoint/Cargo.toml:16,:32,:79` (optional dep, `verify` feature) and `crates/endpoint/src/packs/verify.rs:34-37` (`mirror`, `mirror_kit`, `replay::bisect_tape`, `verify_tape`, `seeded_bug`), plus `look_host.rs:7`, `packs/view.rs:49` | lane | correct the audit; drop the row |
| A4 | `ai::utility` is dead | A.3 item 5 and repeated by one wave lane | It is live from `crates/ai/src/gladiator.rs` and `personality.rs` - callers exist, but only inside `crates/ai`, so nothing outside the crate reaches it | lane | split the class: NO CALLER vs INTERNAL-ONLY |
| A5 | Cloth has no collision at all | `docs/research/COMPARE.md` D-CLOTH-1 (frozen 2026-09-30, pre-RZ7 file) | `crates/physics/src/cloth.rs` takes `ClothCollider`s today. The real gap is narrower: one collider (the wearer's torso), no self-collision (PX29's code was deleted in the RZ7 rewrite), no CCD | lane | add a dated correction beside the frozen denominator; do not rewrite the frozen items |
| A6 | 16 syn, 12 serde_json, 14 num_traits copies | `docs/SMOOTH-BUILD.md` | That was a 2026-10-17 build-artifact count, not `Cargo.lock`. `Cargo.lock` has 1,032 packages, 80 names with more than one version and 101 extra copies. serde_json is one version, num-traits is one version and is not a direct dependency | lane | fix the numbers in SMOOTH-BUILD.md |
| A7 | EG24: "no `.e2nn` is in the tree" | board row EG24 | `fixtures/nets/depth_student.e2nn` exists | lead (glob) | rewrite the row: EG24 is missing something else - name it |
| A8 | `content/reference/` holds the look reference set | implied by gap B8 | `content/reference/` does not exist. Gap B8 (machine look score) has no reference capture set at all | lane | new row: create the set, then the score can be calibrated |

## B. Board rows that point at nothing

| # | Row | The problem | Evidence | Action |
|---|---|---|---|---|
| B1 | GN4, GN5, GN7, GN8 | They all name `center/kaggle.mjs`. That file does not exist and has no git history | lane | give the generator chain a real home in the center repo, then re-point the rows |
| B2 | WD3, WD12, WD14, WD15 | Mostly already in the tree; the row text is stale | lane | rewrite the rows as "the X part is open", with the file and line |
| B3 | GS45 (pass report) | Landed, but `crates/render/src/pass_report.rs` `Plan::validate` (a read with no writer must fail the plan) has zero callers | lane | row: give the validator a caller |
| B4 | UW12, UW40 (nav layers) | `crates/nav/src/layers.rs` landed with `MESH_VERSION` 2 and zero consumers | lane | row: give the layer table a consumer |
| B5 | new (input layers) | `crates/input/src/bindings.rs:725` says "No live consumer yet" for `ActionSetStack`; the input layer is not in the tape | lane | new row: hash the input layer, or say why not |
| B6 | finish-line item 13 (debug kit) | cvars, in-game console, pause, step, time scale and a Perfetto trace have no row and no owner. `crates/game/src/bin/arena.rs` has pause and step, `crates/game/src/virt.rs` has time scale - game lane only | lane | new row for the rest; the render side has RL58 and RL59 |
| B7 | `crates/render/src/debug_draw.rs` | No caller | lane | RL59 gives it one: a debug view in the present path |

## C. Dead code, measured (do not delete while the wave runs)

Sizes from the wave's own pass; the baseline and the deletion manifest are in `docs/compaction-baseline.md`.

| File | Lines | Note | Decision needed |
|---|---|---|---|
| `crates/render/src/heal.rs` | 1,090 | RenderHeal | wire (N-11, AD14) or delete |
| `crates/nav/src/orca.rs` | 971 | 0 callers | wire (NV4, NV10) or delete - see A1 |
| `crates/physics/src/kcc.rs` | 928 | no production caller | delete or wire |
| `crates/audio/src/heal.rs` | 749 | DeviceHeal, called only from a `#[cfg(test)]` mod | the smart lane is wiring it now |
| `crates/observe/src/rerun_sink.rs` | 321 | test-only | wire (N-11, PF16) or delete |
| `crates/ai/src/behavior.rs` | 275 | legacy Brief/Op seam | the smart lane proposed wiring it to GA11; the compaction lane proposed deleting it - the lead picks one |

Total about 4,334 lines. Plus 7 feature flags with no user (`endpoint/tls`, `geo/default|kit|all`,
`kam2-game/window`, `playtest/tls`, `scene/math`), 91 `.profraw` files tracked in git (55.5 MB), and
`target/debug` at 28.9 GiB (row E86).

## D. The size picture

42 library crates, 771,684 lines in `crates/*/src`. Three crates carry a third: render 107,807, endpoint 74,503,
game 73,997. Top 20 files hold 9.2%; `crates/game/src/fighter_look.rs` alone is 6,038. `docs/archive` is 76.7% of
the docs folder (1,196 files, 54.3 MB). For agents: `.opencode/modmap.md` is 36,154 tokens, the entry set about
14,700. `crates/workspace-hack` is two lines with no dependent, so it unifies nothing (row OPT8).

## E. What to do with this file

1. The lead merges the row corrections into `docs/SPRINT*.md` and `docs/plan/finish-line.md` in one commit, by path.
2. The lead applies the three workspace-check rules the layers lane specified (sim-may-not-import-presentation,
   GPU-only-under-present, per-module IO) and the unwired-code detector the workspace lane built.
3. Deletions happen after the landing agents release their crates, one crate at a time, each with that crate's
   tests green and the golden `d57b5c8ae3096614` unchanged twice.
4. Nothing in this file is a claim about performance or the look. Those stay unknown until a capture says otherwise.
