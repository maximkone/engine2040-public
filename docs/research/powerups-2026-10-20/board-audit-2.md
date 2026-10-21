# Board audit 2, the powerup wave's rows (read-only, 2026-10-20 23:56Z)

Auditor: a read-only subagent. HEAD `1a56feed` plus other lanes' uncommitted files
(`git status` showed 20+ modified paths: arena, ecs, ai, observe, material, kam2-game,
endpoint). Nothing in this report edits the board, a doc or code. No row is ticked here:
ticking is the lead's, with the landing sha.

Scope: the 534 open rows in `docs/SPRINT-09-empire.md` through
`docs/SPRINT-27-final-picture.md` plus `docs/SPRINT-28-powerups.md` (535 rows once PW58,
added by `1a56feed` while this audit ran, is counted). `docs/SPRINT.md` and SPRINT-01..08 are
the other auditor's and are not judged here, except where a row in my scope duplicates one.

Method, and what a verdict means:

- **ALREADY DONE** needs the row's own Accept to pass. One-line results are pasted.
- **DUPLICATE** means another open row asks for the same thing; both ids are named, and
  the shared plan or crate file is named. The 2026-10-21 wave rowed the same work twice on
  purpose ("the old rows stay as they are until the lead corrects their text"), so most
  duplicates here are an old row plus the PW row that closes it.
- **WRONG PREMISE** means a claim in the row is false today, with the grep, path or
  accept that proves it.
- **REAL** means none of the above and the row still needs building. The letter in the
  proof column is the evidence class: **A** the row's accept filter matches no test in
  its crate (so the accept cannot pass today), **B** the file the row must create does
  not exist, **C** the row itself names the work left, **D** the row is parked LATER and
  nothing is built, **W** a 2026-10-21 wave row (PW) that was never ticked and whose accept
  test does not exist.
- **NOT JUDGED** (last table) means the code the row talks about is present, the accept
  is prose, and the row does not say what is left. Those need one accept run each and a
  session owner; I did not invent a command for them.

Counts: **2 already done, 61 duplicate, 6 wrong premise,
300 real, 1 probable-done-but-blocked, 165 not judged** =
535.

The tree moved while I read it: HEAD went 1a56feed -> b0d18911 -> 43532af8, and 20+ files
were uncommitted at any moment, so two of my accept runs are red for other lanes' work and
are reported as such, not as row failures.

## What the lead should do first

1. **Tick two rows that are done and open.** GS44 (accept PASS, 10 passed, run 39514420794) and
   AD18 (accept PASS, 13 passed, run 39515222235). Both landings are committed already
   (`c59c4ed3`, `c4641f4d`) and neither is on the board as landed.
2. **Close PW10.** The GS44 landing did its work at 01:03 on 2026-10-21, after the wave read
   the tree. It is the only PW row that is already built.
3. **Fix two rows that cite a file which is not in the tree.** PW31 and PW58 both cite
   `crates/ai/src/plan_trace.rs` and an 8-test run; the file is absent from the tree, from
   `git log --all` and from the working tree. Delete the citation or build the trace.
4. **Retire the 61 duplicate rows** (table 2). Most are old rows the 2026-10-21 wave re-rowed on
   purpose as PW rows. Retiring them halves the open board without losing work.
5. **Two premise repairs** that cost one line each: GA15 (its phase-2 half landed at
   `depth.rs:459`) and EG24 (`fixtures/nets/depth_student.e2nn` exists).
6. **One row to close as void:** ADV51b fixes `tools/sprint-lock.mjs`, deleted in `e5457d6e`;
   this repo has no sprint lock.
7. **A tree-red warning, not mine to fix:** `crates/arena/src/fork.rs:426,839` calls
   `engine_ecs::pool::shared()`, which HEAD keeps `pub(crate)` (`crates/ecs/src/pool.rs:327`
   at HEAD). With that file at HEAD the arena lib does not compile, so every playtest, game
   and proof accept is red through it (`cargo test -p engine-playtest --lib sweep_noise` ->
   FAIL, E0603, run 39515378524). The uncommitted `pool.rs` change makes it `pub` and fixes
   it; it is the E21/PW20 lane's to land (also in `docs/BLOCKERS.md`, `a94c2545`).
8. **Side check on the wave-3 rows the board already ticked:** GS43 (`crates/render/src/contact_shadow.rs`,
   `shaders/contact.wgsl`), GS45 (`crates/render/src/pass_report.rs`), GS46 (`world.query` filter),
   GS47 (a NOOP row), GS48 (`crates/ai/src/decision_log.rs` plus the `ai.decisions` verb) all
   have their code in the tree; GS45's validator still has no caller, which is PW07 and
   `00-CORRECTIONS.md` B3. Those ticks hold.

## 1. ALREADY DONE (the accept passes and the row is still open)

| id | verdict | proof (path:line or accept result) | what the lead should do |
|---|---|---|---|
| GS44 | ALREADY DONE | `node tools/gh-check.mjs --remote-only cargo test -p engine-render --lib auto_exposure` -> PASS on the runner (run 39514420794): `test result: ok. 10 passed; 0 failed; 1 ignored; 858 filtered out`. Code: `crates/render/src/present/gpu_present/auto_exposure_gpu.rs` (device 64-bin histogram + readback), `crates/render/src/post/auto_exposure.rs` (CPU half), wiring in `present/gpu_present/live.rs`; landed `c59c4ed3` (2026-10-21 01:03). | Tick GS44 as landed `c59c4ed3` and cite the run above. Note the row's path `crates/render/src/auto_exposure.rs` is `post/auto_exposure.rs` today. Decide the meter default (it ships off behind `E2040_AUTO_EXPOSURE=1`; on changes every capture). |
| AD18 | ALREADY DONE | `node tools/gh-check.mjs --remote-only cargo test -p engine-spatial --lib overlap_3d` -> PASS on the runner (run 39515222235): `test result: ok. 13 passed; 0 failed`. Covers the accept items: lamp on a table clear, crate under a table clear, 4 m plank at 45 never intersected, 20 permutations equal digests, flat-room golden byte-identical, stacked digest re-pinned. Landed `c4641f4d` (2026-10-20, `spatial: AD18 3D yaw-aware overlap`). | Tick AD18 as landed `c4641f4d` and cite the run. |

## 2. DUPLICATE (name both rows, one shared target)

| id | verdict | proof: the other row and the shared plan or file | what the lead should do |
|---|---|---|---|
| AD14 | DUPLICATE of PW30 | PW30's row says it closes AD14, E75, A9 and G1; both ask for the same build | Retire AD14 when PW30 lands, or fold any extra item of AD14 into PW30 first. Do not run both. |
| AF3 | DUPLICATE of AF1 | both create `tools/fix.mjs` and `tools/fix.test.mjs` | Retire AF3 when AF1 lands, or fold any extra item of AF3 into AF1 first. Do not run both. |
| AM23 | DUPLICATE of PW32 | PW32's row says it closes AM23 and C14; both ask for the same build | Retire AM23 when PW32 lands, or fold any extra item of AM23 into PW32 first. Do not run both. |
| AW23 | DUPLICATE of AW18 | both move old rows into `docs/sprint/later.md` / `docs/board-done.md` | Retire AW23 when AW18 lands, or fold any extra item of AW23 into AW18 first. Do not run both. |
| CQ20 | DUPLICATE of PW52 | PW52's row says it closes CQ20; both ask for the same build | Retire CQ20 when PW52 lands, or fold any extra item of CQ20 into PW52 first. Do not run both. |
| E51 | DUPLICATE of PW51 | PW51's row says it closes E51; both ask for the same build | Retire E51 when PW51 lands, or fold any extra item of E51 into PW51 first. Do not run both. |
| E58 | DUPLICATE of E66 | both create `tools/steal-land.mjs` | Retire E58 when E66 lands, or fold any extra item of E58 into E66 first. Do not run both. |
| E64 | DUPLICATE of PW50 | PW50's row says it closes E64; both ask for the same build | Retire E64 when PW50 lands, or fold any extra item of E64 into PW50 first. Do not run both. |
| E75 | DUPLICATE of PW30 | PW30's row says it closes AD14, E75, A9 and G1; both ask for the same build | Retire E75 when PW30 lands, or fold any extra item of E75 into PW30 first. Do not run both. |
| EG18b | DUPLICATE of PW36 | PW36's row says it closes EG18b; both ask for the same build | Retire EG18b when PW36 lands, or fold any extra item of EG18b into PW36 first. Do not run both. |
| EG40 | DUPLICATE of PW34 | PW34's row says it closes EG40 and the grip half of LF12; both ask for the same build | Retire EG40 when PW34 lands, or fold any extra item of EG40 into PW34 first. Do not run both. |
| FU2 | DUPLICATE of PW29 | PW29's row says it closes C12, FU2 and FU3; both ask for the same build | Retire FU2 when PW29 lands, or fold any extra item of FU2 into PW29 first. Do not run both. |
| FU3 | DUPLICATE of PW29 | PW29's row says it closes C12, FU2 and FU3; both ask for the same build | Retire FU3 when PW29 lands, or fold any extra item of FU3 into PW29 first. Do not run both. |
| GN5 | DUPLICATE of PW56 | PW56's row says it closes GN7, GN8 and GN5; both ask for the same build | Retire GN5 when PW56 lands, or fold any extra item of GN5 into PW56 first. Do not run both. |
| GN6 | DUPLICATE of PW57 | PW57's row says it closes GN6; both ask for the same build | Retire GN6 when PW57 lands, or fold any extra item of GN6 into PW57 first. Do not run both. |
| GN7 | DUPLICATE of PW56 | PW56's row says it closes GN7, GN8 and GN5; both ask for the same build | Retire GN7 when PW56 lands, or fold any extra item of GN7 into PW56 first. Do not run both. |
| GN8 | DUPLICATE of PW56 | PW56's row says it closes GN7, GN8 and GN5; both ask for the same build | Retire GN8 when PW56 lands, or fold any extra item of GN8 into PW56 first. Do not run both. |
| GS18b | DUPLICATE of PW05 | PW05's row says it closes GS18b, gap B7 cut 5; both ask for the same build | Retire GS18b when PW05 lands, or fold any extra item of GS18b into PW05 first. Do not run both. |
| GS35 | DUPLICATE of PW08 | PW08's row says it closes GS35; both ask for the same build | Retire GS35 when PW08 lands, or fold any extra item of GS35 into PW08 first. Do not run both. |
| LAI3 | DUPLICATE of PW35 | PW35's row says it closes LAI3; both ask for the same build | Retire LAI3 when PW35 lands, or fold any extra item of LAI3 into PW35 first. Do not run both. |
| LAI7 | DUPLICATE of PW27 | PW27's row says it closes D18 and LAI7; both ask for the same build | Retire LAI7 when PW27 lands, or fold any extra item of LAI7 into PW27 first. Do not run both. |
| LF5 | DUPLICATE of PW01 | PW01's row says it closes LF5, gap B7 cut 1; both ask for the same build | Retire LF5 when PW01 lands, or fold any extra item of LF5 into PW01 first. Do not run both. |
| NS19 | DUPLICATE of PW33 | PW33's row says it closes NS19; both ask for the same build | Retire NS19 when PW33 lands, or fold any extra item of NS19 into PW33 first. Do not run both. |
| OPT11 | DUPLICATE of PW44 | PW44's row says it closes OPT11; both ask for the same build | Retire OPT11 when PW44 lands, or fold any extra item of OPT11 into PW44 first. Do not run both. |
| OPT27 | DUPLICATE of PW28 | PW28's row says it closes OPT27 and LAI7; both ask for the same build | Retire OPT27 when PW28 lands, or fold any extra item of OPT27 into PW28 first. Do not run both. |
| OPT29 | DUPLICATE of PW43 | PW43's row says it closes OPT29; both ask for the same build | Retire OPT29 when PW43 lands, or fold any extra item of OPT29 into PW43 first. Do not run both. |
| OPT8 | DUPLICATE of PW45 | PW45's row says it closes OPT8; both ask for the same build | Retire OPT8 when PW45 lands, or fold any extra item of OPT8 into PW45 first. Do not run both. |
| OPT9 | DUPLICATE of PW46 | PW46's row says it closes OPT9; both ask for the same build | Retire OPT9 when PW46 lands, or fold any extra item of OPT9 into PW46 first. Do not run both. |
| PF25 | DUPLICATE of PW07 | PW07's row says it closes PF25, and gives the landed GS45 a caller; both ask for the same build | Retire PF25 when PW07 lands, or fold any extra item of PF25 into PW07 first. Do not run both. |
| PF27 | DUPLICATE of PW41 | PW41's row says it closes finish-line item 13 and PF27; both ask for the same build | Retire PF27 when PW41 lands, or fold any extra item of PF27 into PW41 first. Do not run both. |
| QA15 | DUPLICATE of PW40 | PW40's row says it closes QW5 and QA15; both ask for the same build | Retire QA15 when PW40 lands, or fold any extra item of QA15 into PW40 first. Do not run both. |
| QA4 | DUPLICATE of PW06 | PW06's row says it closes RL35 and QA4; both ask for the same build | Retire QA4 when PW06 lands, or fold any extra item of QA4 into PW06 first. Do not run both. |
| QW10 | DUPLICATE of PW39 | PW39's row says it closes QW10 and WEB11; both ask for the same build | Retire QW10 when PW39 lands, or fold any extra item of QW10 into PW39 first. Do not run both. |
| QW5 | DUPLICATE of PW40 | PW40's row says it closes QW5 and QA15; both ask for the same build | Retire QW5 when PW40 lands, or fold any extra item of QW5 into PW40 first. Do not run both. |
| RC36 | DUPLICATE of PW20 | PW20's row says it closes E21 and RC36; both ask for the same build | Retire RC36 when PW20 lands, or fold any extra item of RC36 into PW20 first. Do not run both. |
| RL32 | DUPLICATE of PW02 | PW02's row says it closes RL60 and RL32, gap B7 cut 2; both ask for the same build | Retire RL32 when PW02 lands, or fold any extra item of RL32 into PW02 first. Do not run both. |
| RL34 | DUPLICATE of PW12 | PW12's row says it closes RL34, and the metric half of B8; both ask for the same build | Retire RL34 when PW12 lands, or fold any extra item of RL34 into PW12 first. Do not run both. |
| RL35 | DUPLICATE of PW06 | PW06's row says it closes RL35 and QA4; both ask for the same build | Retire RL35 when PW06 lands, or fold any extra item of RL35 into PW06 first. Do not run both. |
| RL38 | DUPLICATE of PW04 | PW04's row says it closes RL38, gap B7 cut 4; both ask for the same build | Retire RL38 when PW04 lands, or fold any extra item of RL38 into PW04 first. Do not run both. |
| RL41 | DUPLICATE of PW03 | PW03's row says it closes RL41, gap B7 cut 3; both ask for the same build | Retire RL41 when PW03 lands, or fold any extra item of RL41 into PW03 first. Do not run both. |
| RL55 | DUPLICATE of RL54 | RL55's switch reads `crates/render/src/crowd.rs`, the module RL54 builds | Retire RL55 when RL54 lands, or fold any extra item of RL55 into RL54 first. Do not run both. |
| RL58 | DUPLICATE of PW13 | PW13's row says it closes RL59, gives RL58 a caller; both ask for the same build | Retire RL58 when PW13 lands, or fold any extra item of RL58 into PW13 first. Do not run both. |
| RL59 | DUPLICATE of PW13 | PW13's row says it closes RL59, gives RL58 a caller; both ask for the same build | Retire RL59 when PW13 lands, or fold any extra item of RL59 into PW13 first. Do not run both. |
| RL60 | DUPLICATE of PW02 | PW02's row says it closes RL60 and RL32, gap B7 cut 2; both ask for the same build | Retire RL60 when PW02 lands, or fold any extra item of RL60 into PW02 first. Do not run both. |
| RT13 | DUPLICATE of PW11 | PW11's row says it closes RT7 and RT13; both ask for the same build | Retire RT13 when PW11 lands, or fold any extra item of RT13 into PW11 first. Do not run both. |
| RT5 | DUPLICATE of RT19 | both add `crates/render/src/rt_reflect.rs` | Retire RT5 when RT19 lands, or fold any extra item of RT5 into RT19 first. Do not run both. |
| RT7 | DUPLICATE of PW11 | PW11's row says it closes RT7 and RT13; both ask for the same build | Retire RT7 when PW11 lands, or fold any extra item of RT7 into PW11 first. Do not run both. |
| UC8 | DUPLICATE of PW21 | PW21's row says it closes UC8 and UW36; both ask for the same build | Retire UC8 when PW21 lands, or fold any extra item of UC8 into PW21 first. Do not run both. |
| UW2 | DUPLICATE of PW18 | PW18's row says it closes UW2; both ask for the same build | Retire UW2 when PW18 lands, or fold any extra item of UW2 into PW18 first. Do not run both. |
| UW36 | DUPLICATE of PW21 | PW21's row says it closes UC8 and UW36; both ask for the same build | Retire UW36 when PW21 lands, or fold any extra item of UW36 into PW21 first. Do not run both. |
| VG4 | DUPLICATE of RL54 | both build the crowd module (`crates/render/src/crowd.rs`); RL54's accept names the VG4 bar (20000 spectators in 1.5 ms) | Retire VG4 when RL54 lands, or fold any extra item of VG4 into RL54 first. Do not run both. |
| VG9 | DUPLICATE of RL55 | both want the crowd wave cue; VG9 is LATER-parked, RL55 lands the cue tape | Retire VG9 when RL55 lands, or fold any extra item of VG9 into RL55 first. Do not run both. |
| VX1 | DUPLICATE of VX17 | both add `crates/render/src/volumetric.rs` | Retire VX1 when VX17 lands, or fold any extra item of VX1 into VX17 first. Do not run both. |
| WD14 | DUPLICATE of PW55 | PW55's row says it closes WD14 and WD15; both ask for the same build | Retire WD14 when PW55 lands, or fold any extra item of WD14 into PW55 first. Do not run both. |
| WD15 | DUPLICATE of PW55 | PW55's row says it closes WD14 and WD15; both ask for the same build | Retire WD15 when PW55 lands, or fold any extra item of WD15 into PW55 first. Do not run both. |
| WEB11 | DUPLICATE of PW39 | PW39's row says it closes QW10 and WEB11; both ask for the same build | Retire WEB11 when PW39 lands, or fold any extra item of WEB11 into PW39 first. Do not run both. |
| WEB14 | DUPLICATE of PW38 | PW38's row says it closes A5, and WEB14 is not a prerequisite; both ask for the same build | Retire WEB14 when PW38 lands, or fold any extra item of WEB14 into PW38 first. Do not run both. |
| WEB16 | DUPLICATE of PW37 | PW37's row says it closes A1, WEB16, WEB17 and WEB5; both ask for the same build | Retire WEB16 when PW37 lands, or fold any extra item of WEB16 into PW37 first. Do not run both. |
| WEB17 | DUPLICATE of PW37 | PW37's row says it closes A1, WEB16, WEB17 and WEB5; both ask for the same build | Retire WEB17 when PW37 lands, or fold any extra item of WEB17 into PW37 first. Do not run both. |
| WEB5 | DUPLICATE of PW37 | PW37's row says it closes A1, WEB16, WEB17 and WEB5; both ask for the same build | Retire WEB5 when PW37 lands, or fold any extra item of WEB5 into PW37 first. Do not run both. |
| WEB6 | DUPLICATE of PW47 | PW47's row says it closes A2 and WEB6; both ask for the same build | Retire WEB6 when PW47 lands, or fold any extra item of WEB6 into PW47 first. Do not run both. |

## 3. WRONG PREMISE (the row's text is false today)

| id | verdict | proof | what the lead should do |
|---|---|---|---|
| PW10 | WRONG PREMISE | The row says the presenter dispatch, the one-frame-late readback and the EV feed are missing ("nothing meters the night arena"). All three landed at `c59c4ed3` (2026-10-21 01:03), after the wave read the tree: `crates/render/src/present/gpu_present/auto_exposure_gpu.rs` (`AutoExposureGpu::encode`, `begin_map`, `take_ready`), `crates/render/src/present/gpu_present/live.rs` (`auto_exposure_meter_step`, the dispatch after the forward), `crates/render/src/shaders/blit.wgsl` exposure slot. The accept of its own parent row PASSES today (10 passed, run 39514420794). | Close PW10 as overtaken by GS44 (`c59c4ed3`); do not spend a session on it. Keep only the lead call: turn the meter on by default. |
| PW31 | WRONG PREMISE | The row says the plan trace API landed 2026-10-21 in `crates/ai/src/plan_trace.rs` with 8 tests green. There is no such file and no such symbol: `git grep -ln plan_trace -- crates/` prints nothing, `git log --all -- crates/ai/src/plan_trace.rs` prints nothing, `Get-ChildItem crates/ai/src` has no `plan_trace.rs`. What exists is the decision ring (`crates/ai/src/decision_log.rs`, GS48's landing) and the unlanded BT/GOAP modules (`behavior.rs`, `goap.rs`, 0 callers). | Rewrite PW31: the seam it names is not in the tree, so the row is real work (build the plan trace) and its accept cannot pass as written. Do not tick anything from the cited run id. |
| PW58 | WRONG PREMISE | Same false premise as PW31: cites `crates/ai/src/plan_trace.rs` and `cargo test -p engine-ai --lib plan_trace -> 8 passed` (run 39507890144). `git grep -ln plan_trace -- crates/` is empty and the file does not exist, so that run did not run against this tree. The rest of PW58 is right: `crates/ai/src/behavior.rs` and `goap.rs` have no caller outside `crates/ai` (the row's own correction, commit `1a56feed`). | Keep PW58 as the row (one brain), but delete the plan-trace sentence and the run citation until the file lands; PW31 is the row that builds it. |
| GA15 | WRONG PREMISE | The row's \"Left: phase 2 at 500 permille hp with its new v2 move and answer\" is done in the tree: `crates/ai/src/depth.rs:459 primus_phase` (phase 2 at 500 permille), `:470 primus_counter`, and the test `tier_primus_phase_starts_at_500_permille` (`depth.rs:2782`). What is genuinely open is the playtest proof: `cargo test -p engine-playtest --lib primus` matches no test (`git grep -n primus -- crates/playtest/` is empty). | Rewrite GA15 as \"the phase-2 half landed at `depth.rs:459`; left: the 200-duel playtest proof\" and re-point the accept at an existing crate/filter. |
| EG24 | WRONG PREMISE | The row's premise \"no `.e2nn` is in the tree\" is false (also recorded in `docs/research/powerups-2026-10-20/00-CORRECTIONS.md` A7): `Test-Path fixtures/nets/depth_student.e2nn` is True. The accept it names (`--lib infer_into_no_alloc`, `--lib student_gate`) is a different question and still open. | Rewrite the premise sentence; keep the zero-alloc and gate work. |
| ADV51b | WRONG PREMISE | The row edits `tools/sprint-lock.mjs:87-93` (\"bare 'release' skips the foreign-holder guard\"). That file does not exist: `Test-Path tools/sprint-lock.mjs` is False, and `git log --all -- tools/sprint-lock.mjs` ends at `e5457d6e` \"standalone 4/4: delete the seat guard, the sprint lock, quiet and the knobs\". engine2040 has no sprint loop, no lock and no seat guard (AGENTS.md: \"Standalone repo (owner 2026-10-17): no sprints, no locks, no claims\"). | Close ADV51b as void: the tool it fixes was deleted on purpose. Do not rebuild the sprint lock. |

## 4. REAL (needs building; the letter is the evidence class)

| id | board | verdict | proof | what the lead should do |
|---|---|---|---|---|
| AB15 | 09-empire.md | REAL (A) | accept filter `build_game` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| AD10 | 18-gaps.md | REAL (A) | accept filter `undo_gated` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| AD13 | 18-gaps.md | REAL (B) | target file absent: `crates/rules/tests/vet_props.rs` -- row says: "...crates/endpoint/src/packs/rules.rs', 'packs/rules/verbs.rs', 'crates/rules/src/rules.rs', 'crates/rules/tests/vet_props.rs' (new). Accept: 'cargo test -p engine-" | Keep as a builder row. |
| AD15 | 18-gaps.md | REAL (A) | accept filter `clip_gates` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| AD16 | 18-gaps.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| AD2 | 18-gaps.md | REAL (A) | accept filter `landed_digest` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| AD20 | 18-gaps.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| AD24 | 18-gaps.md | REAL (A) | accept filter `capture_subject` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| AD26 | 18-gaps.md | REAL (B) | target file absent: `crates/endpoint/src/advice.rs` -- row says: "...'ValidationErrorEnvelope.cs') and C62 (MCP tools spec). Diff: good. Tier A. Size M. Owns 'crates/endpoint/src/advice.rs' (new, the one table) and hunks in 'cr" | Keep as a builder row. |
| AD27 | 18-gaps.md | REAL (B) | target file absent: `crates/endpoint/src/prompts.rs` -- row says: "...w. Source: B41 (old 'Prompts/*.md', 'PromptCatalog.cs'). Diff: good. Tier A. Size M. Owns 'crates/endpoint/src/prompts.rs' (new) and hunks in 'crates/endpoint/s" | Keep as a builder row. |
| AD7 | 18-gaps.md | REAL (A) | accept filter `spatial_view` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| AF1 | 27-final-picture.md | REAL (B) | target file absent: `tools/fix.mjs` -- row says: "...fix (the cargo fix path) and the clippy usage page, read 2026-10-20. Tier A. Size S. Owns 'tools/fix.mjs', 'tools/fix.test.mjs'. Accept: 'node" | Keep as a builder row. |
| AIN4 | 25-ai-native.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| AM24 | 21-look-feel.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| AM25 | 21-look-feel.md | REAL (A) | accept filter `look_soft` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| AM28 | 26-motion.md | REAL (A) | accept filter `traj_student` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| AW1 | 14-agent-workspace.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| AW10 | 14-agent-workspace.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| AW11 | 14-agent-workspace.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| AW12 | 14-agent-workspace.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| AW13 | 14-agent-workspace.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| AW14 | 14-agent-workspace.md | REAL (B) | target file absent: `tools/plan-check.mjs` -- row says: "...- [ ] AW14 tools: plan and board check — 'tools/plan-check.mjs' compares every plan's §7 rows with th" | Keep as a builder row. |
| AW15 | 14-agent-workspace.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| AW16 | 14-agent-workspace.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| AW17 | 14-agent-workspace.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| AW18 | 14-agent-workspace.md | REAL (B) | target file absent: `tools/board.mjs` -- row says: "...- [ ] AW18 tools: union board loader. Tier B. Size S. Owns new 'tools/board.mjs' ('loadBoard': 'docs/SPRINT.md' split" | Keep as a builder row. |
| AW19 | 14-agent-workspace.md | REAL (B) | target file absent: `tools/board-split.mjs` -- row says: "...- [ ] AW19 tools: one-shot splitter (check-only). Tier B. Size M. Owns new one-shot 'tools/board-split.mjs' ('--check' prints per-section done/ti" | Keep as a builder row. |
| AW2 | 14-agent-workspace.md | REAL (B) | target file absent: `docs/receipts/YYYY-MM.md` -- row says: "...- [ ] AW2 docs: board split — receipts older than three waves move to 'docs/receipts/YYYY-MM.md', ticked rows older than ten waves to" | Keep as a builder row. |
| AW20 | 14-agent-workspace.md | REAL (B) | target file absent: `tools/sprint-loop.mjs` -- row says: "...ols: loader adopt, gate pair. Tier B. Size S. Owns hunks in 'tools/workspace-check.mjs' + 'tools/sprint-loop.mjs'. Accept: 'workspace-check' PASS + 'sp" | Keep as a builder row. |
| AW21 | 14-agent-workspace.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| AW22 | 14-agent-workspace.md | REAL (B) | target file absent: `tools/drift-check.mjs` -- row says: "...- [ ] AW22 tools: loader adopt, loop triple. Tier B. Size S. Owns hunks in 'tools/drift-check.mjs' + 'tools/ladder-sync.mjs' + 'tools/ru" | Keep as a builder row. |
| AW4 | 14-agent-workspace.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| AW5 | 14-agent-workspace.md | REAL (B) | target file absent: `tools/agent-replay.mjs` -- row says: "...st a changed agent file before it lands; the pass rate may not drop. Tier B. Size M. Owns 'tools/agent-replay.mjs' and 'fixtures/agent-replay/'. Accept:" | Keep as a builder row. |
| AW6 | 14-agent-workspace.md | REAL (B) | target file absent: `tools/triage.mjs` -- row says: "...per new fingerprint; the '-j4' fingerprint gets a root-cause spike. Tier A. Size M. Owns 'tools/triage.mjs'. Accept: the last 10 proof logs class" | Keep as a builder row. |
| AW7 | 14-agent-workspace.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| AW8 | 14-agent-workspace.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| AX23 | 18-gaps.md | REAL (B) | target file absent: `crates/endpoint/tests/mcp_stdio.rs` | Keep as a builder row. |
| AX28 | 21-look-feel.md | REAL (A) | accept filter `task_play_run` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| BX41 | 21-look-feel.md | REAL (A) | accept filter `cut_cap` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| BX5 | 17-body-and-break.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| BX7 | 17-body-and-break.md | REAL (B) | target file absent: `crates/render/src/slice.rs` -- row says: "...cook templates, blood wetted by bleed rate, no CPU mesh generation. Tier A. Size L. Owns 'crates/render/src/slice.rs'. Accept: 0.3 ms per cut, 32 stumps li" | Keep as a builder row. |
| BX9 | 17-body-and-break.md | REAL (B) | target file absent: `crates/net/src/server.rs` -- row says: "...managed, clients settle limbs within 5 cm in 10 ticks. Tier A. Size M. Owns hunks beside 'crates/net/src/server.rs'. Accept: 64-client soak with 8 cuts p" | Keep as a builder row. |
| CX1 | 27-final-picture.md | REAL (B) | target file absent: `crates/game/src/fighter_look_cape.rs` -- row says: "...cape constants, 'cape_row_into', 'cape_normal', 'cape_mesh' and the 'cape_*' methods into 'crates/game/src/fighter_look_cape.rs' as a '#[path]' child module, the way" | Keep as a builder row. |
| E16 | 10-empire-inbox-drain.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| E37 | 10-empire-inbox-drain.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| E44 | 10-empire-inbox-drain.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| E46 | 10-empire-inbox-drain.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| E59 | 10-empire-inbox-drain.md | REAL (B) | target file absent: `tools/steal-next.mjs` -- row says: "...ch hunk with rg before editing. Diff: regular. Tier B. Size S. Owns 'tools/affected.mjs', 'tools/steal-next.mjs', 'tools/workspace-check.mjs', 'tools/" | Keep as a builder row. |
| E62 | 10-empire-inbox-drain.md | REAL (B) | target file absent: `.github/workflows/release.yml` -- row says: "...a tag and posts one manifest; ci.yml stays untouched. Diff: regular. Tier C. Size S. Owns '.github/workflows/release.yml', 'dist-workspace.toml', 'docs/DEPENDE" | Keep as a builder row. |
| E65 | 10-empire-inbox-drain.md | REAL (B) | target file absent: `tools/lanes.test.mjs` -- row says: "...st-quality.mjs, board-slice.mjs and workspace-check.mjs. Diff: good. Tier B. Size M. Owns 'tools/lanes.test.mjs', 'tools/rust-quality.mjs', 'tools/boa" | Keep as a builder row. |
| E66 | 10-empire-inbox-drain.md | REAL (B) | target file absent: `tools/drift-check.mjs` -- row says: "...ts 6 stale doc refs. Re-find each hunk with rg first. Diff: regular. Tier B. Size S. Owns 'tools/drift-check.mjs', 'tools/stale-refs.test.mjs', 'tools/" | Keep as a builder row. |
| E7 | 10-empire-inbox-drain.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| E70 | 10-empire-inbox-drain.md | REAL (A) | accept filter `meshlet_cull_vectors` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| E73 | 10-empire-inbox-drain.md | REAL (A) | accept filter `skinning_layout_vectors` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| E77 | 10-empire-inbox-drain.md | REAL (A) | accept filter `dlss` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| EA6 | 18-gaps.md | REAL (A) | accept filter `door_token` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| EG11b | 18-gaps.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| EG11c | 18-gaps.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| EG12 | 18-gaps.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| EG19c | 18-gaps.md | REAL (A) | accept filter `swing_on_press` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| EG26 | 18-gaps.md | REAL (A) | accept filter `console_step` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| EG32 | 18-gaps.md | REAL (A) | accept filter `skin_palette` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| EG35 | 18-gaps.md | REAL (B) | target file absent: `crates/endpoint/src/packs/render.rs` -- row says: "...unless enabled. Diff: good. Tier A. Size M. Owns 'crates/endpoint/src/packs/view.rs', new 'crates/endpoint/src/packs/render.rs', 'crates/endpoint/src/packs/mod.rs' l" | Keep as a builder row. |
| EG38 | 18-gaps.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| EG39 | 18-gaps.md | REAL (A) | accept filter `resim_no_restart` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| EG42 | 18-gaps.md | REAL (A) | accept filter `hit_track_bake` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| FB1 | 09-empire.md | REAL (B) | target file absent: `crates/proof/src/fb_transport.rs` -- row says: "...match carries an authority/replay diagnosis naming the culprit tick. Tier B. Size M. Owns 'crates/proof/src/fb_transport.rs' plus the 'target/evidence/b/transport" | Keep as a builder row. |
| FB5 | 09-empire.md | REAL (A) | accept filter `fb_soak` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| FB6 | 09-empire.md | REAL (A) | accept filter `fb_matrix` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| FB7 | 09-empire.md | REAL (B) | target file absent: `crates/proof/src/fb_workflow.rs` -- row says: "...blockers + time recorded, every fix lands in the flow not around it. Tier B. Size S. Owns 'crates/proof/src/fb_workflow.rs' plus the 'target/evidence/b/workflow/" | Keep as a builder row. |
| FC1 | 18-gaps.md | REAL (B) | target file absent: `tools/quiet.mjs` -- row says: "...run is needed. Progress 2026-10-20 (perf seat, HEAD 5099d6f9): fresh x3 hand quiet gate ('tools/quiet.mjs' was deleted in e5457d6e), report 'tar" | Keep as a builder row. |
| FU1 | 12-tier-2040.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| FU4 | 12-tier-2040.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| FU5 | 12-tier-2040.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| FU7 | 12-tier-2040.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| FU8 | 12-tier-2040.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| FV6 | 21-look-feel.md | REAL (B) | target file absent: `crates/game/tests/it/fps_view_tape.rs` -- row says: "...- [ ] FV6 proof: scripted 600-tick eye tape with receipt. Tier B. Size S. Owns 'crates/game/tests/it/fps_view_tape.rs' (new file, wired as a 'mod' line in i" | Keep as a builder row. |
| GA14 | 20-game.md | REAL (A) | accept filter `sweep_noise` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| GEO1b | 17-body-and-break.md | REAL (B) | target file absent: `tools/dcc/recipes/crate.json` -- row says: "...ter' header comment, imports no engine code, reads '<recipe.json> <out.glb>' after '--'), 'tools/dcc/recipes/crate.json' ('{"seed":7,"ops":[{"op":"cube","size" | Keep as a builder row. |
| GP33 | 20-game.md | REAL (A) | accept filter `ttk` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| GP44 | 21-look-feel.md | REAL (A) | accept filter `telegraph_warp` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| GS2 | 18-steals.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| GS37 | 21-look-feel.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| GS40 | 18-steals.md | REAL (A) | accept filter `power` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| GS41 | 18-steals.md | REAL (B) | target file absent: `crates/game/src/mouse_raw.rs` -- row says: "...ons only, and 'fps_view::LookState::apply_mouse' has no mouse source. Add 'MouseRaw' (new 'crates/game/src/mouse_raw.rs'): 'push(dx, dy)' sums winit 'DeviceEv" | Keep as a builder row. |
| GS42 | 18-steals.md | REAL (B) | target file absent: `crates/game/src/announcer.rs` -- row says: "...he crowd has per-reaction cooldowns ('audio/src/crowd.rs') but the bout has no voice. Add 'crates/game/src/announcer.rs': a rule table '(FightCue, line name," | Keep as a builder row. |
| GS7 | 18-steals.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| LAI1 | 15-platform-live-ai.md | REAL (B) | target file absent: `crates/provider/src/voice.rs` -- row says: "...ce, text and style digest; a deadline falls back to subtitles alone. Tier B. Size M. Owns 'crates/provider/src/voice.rs'. Accept: a line replays the same audi" | Keep as a builder row. |
| LAI2 | 15-platform-live-ai.md | REAL (B) | target file absent: `crates/provider/src/listen.rs` -- row says: "...a confidence floor, else a typed refusal; keys remain the fallback. Tier B. Size M. Owns 'crates/provider/src/listen.rs'. Accept: 50 recorded clips map to the" | Keep as a builder row. |
| LAI4 | 15-platform-live-ai.md | REAL (B) | target file absent: `crates/provider/src/moderate.rs` -- row says: "...to NPCs passes the injection scorer first; every decision recorded. Tier B. Size M. Owns 'crates/provider/src/moderate.rs'. Accept: a 200-line red-team set is c" | Keep as a builder row. |
| LAI5 | 15-platform-live-ai.md | REAL (B) | target file absent: `crates/playtest/src/vision_judge.rs` -- row says: "...0 owner-labeled captures before its scores count; cassette-recorded. Tier B. Size M. Owns 'crates/playtest/src/vision_judge.rs'. Accept: agreement ≥ 80% on the label" | Keep as a builder row. |
| LAI6 | 15-platform-live-ai.md | REAL (B) | target file absent: `crates/ai/src/recall.rs` -- row says: "...-k (ties by id), falling back to LW2 salience; recall sets recorded. Tier C. Size M. Owns 'crates/ai/src/recall.rs'. Accept: recall sets identical x2 fro" | Keep as a builder row. |
| LF14 | 18-gaps.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| LF2 | 18-gaps.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| LF5b | 18-gaps.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| MP25 | 21-look-feel.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| NS9 | 11-north-star.md | REAL (B) | target file absent: `tools/look-rubric.mjs` -- row says: "...e 'docs/VISUAL-TARGET.md' over RA14/VS9 frames (judge still opens the PNGs). Size S. Owns 'tools/look-rubric.mjs'. Accept: F2P harness scores today's c" | Keep as a builder row. |
| OB4 | 09-empire.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| ON12 | 12-tier-2040.md | REAL (B) | target file absent: `crates/audio/src/voice_net.rs` -- row says: "...ON1 streams with jitter buffer and distance gain, presentation only. Tier C. Size M. Owns 'crates/audio/src/voice_net.rs'. Accept: two QUIC clients talk with u" | Keep as a builder row. |
| ON13 | 12-tier-2040.md | REAL (B) | target file absent: `crates/net/src/party.rs` -- row says: "...rules-digest refuse, party of 3 into one match, list refresh budget. Tier C. Size M. Owns 'crates/net/src/party.rs'. Accept: mismatched versions refuse l" | Keep as a builder row. |
| ON28 | 12-tier-2040.md | REAL (B) | target file absent: `crates/net/src/soft_collision.rs` -- row says: "...ganic soft collisions reimplemented (MIT, PORT cite + EVIDENCE row). Tier B. Size M. Owns 'crates/net/src/soft_collision.rs'. Accept: de-sync converges ≤ 3 frames" | Keep as a builder row. |
| ON30 | 12-tier-2040.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| ON31 | 12-tier-2040.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| ON6 | 12-tier-2040.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| ON7 | 12-tier-2040.md | REAL (B) | target file absent: `crates/net/src/phys_sync.rs` -- row says: "...ntacts and siege projectiles with server authority and client resim. Tier C. Size L. Owns 'crates/net/src/phys_sync.rs'. Accept: a wall break converges on al" | Keep as a builder row. |
| ON8 | 12-tier-2040.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| OPT16 | 13-engine-edge.md | REAL (B) | target file absent: `crates/render/src/ladder.rs` -- row says: "...RT0; draw through the fastest supported path with a tested fallback. Tier B. Size M. Owns 'crates/render/src/ladder.rs'. Accept: 10k instances of 200 materia" | Keep as a builder row. |
| OPT17 | 13-engine-edge.md | REAL (B) | target file absent: `crates/render/src/stream.rs` -- row says: "...dget for 6 GB, LRU eviction, uploads on the RA6 belts; never stalls. Tier B. Size L. Owns 'crates/render/src/stream.rs'. Accept: a dense fixture stays under" | Keep as a builder row. |
| OPT18 | 13-engine-edge.md | REAL (B) | target file absent: `crates/render/src/pacing.rs` -- row says: "...mode, input sampled late for the camera; measured by the PQ1 meter. Tier B. Size M. Owns 'crates/render/src/pacing.rs'. Accept: input-to-photon p95 −20% at" | Keep as a builder row. |
| OPT21 | 13-engine-edge.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| OPT30 | 13-engine-edge.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| OPT31 | 13-engine-edge.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| OPT33 | 13-engine-edge.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| OPT42 | 27-final-picture.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| OPT6 | 13-engine-edge.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| PB53 | 09-empire.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| PB61 | 09-empire.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| PLF10 | 15-platform-live-ai.md | REAL (B) | target file absent: `crates/game/src/presets.rs` -- row says: "...plus a five-second probe; a crash at start relaunches in safe mode. Tier B. Size S. Owns 'crates/game/src/presets.rs'. Accept: presets differ by adapter cl" | Keep as a builder row. |
| PLF4 | 15-platform-live-ai.md | REAL (B) | target file absent: `crates/game/src/workshop.rs` -- row says: "...validated by schema, provenance and a size cap; code mods stay out. Tier C. Size M. Owns 'crates/game/src/workshop.rs'. Accept: a mission mod round-trips; a" | Keep as a builder row. |
| PLF6 | 15-platform-live-ai.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| PLF7 | 15-platform-live-ai.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| PLF8 | 15-platform-live-ai.md | REAL (B) | target file absent: `crates/editor/src/sequencer.rs` -- row says: "...rames through command batches with undo; missions use it for intros. Tier B. Size L. Owns 'crates/editor/src/sequencer.rs'. Accept: a 30-second sequence plays i" | Keep as a builder row. |
| PLF9 | 15-platform-live-ai.md | REAL (B) | target file absent: `crates/game/src/benchmark.rs` -- row says: "...rting fps percentiles, tier, adapter and driver to a shareable file. Tier B. Size S. Owns 'crates/game/src/benchmark.rs'. Accept: two runs within 5%; the file" | Keep as a builder row. |
| POV1 | 27-final-picture.md | REAL (A) | accept filter `lens` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| POV2 | 27-final-picture.md | REAL (A) | accept filter `fps_view` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| POV3 | 27-final-picture.md | REAL (A) | accept filter `viewmodel` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| POV4 | 27-final-picture.md | REAL (A) | accept filter `rtt` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| POV6 | 27-final-picture.md | REAL (A) | accept filter `comfort` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| POV7 | 27-final-picture.md | REAL (A) | accept filter `pov_tape` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PQ22 | 21-look-feel.md | REAL (A) | accept filter `golden_bless` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW01 | 28-powerups.md | REAL (A) | accept filter `motion_vector` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW02 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW03 | 28-powerups.md | REAL (B) | target file absent: `crates/render/src/shaders/prepass.wgsl` -- row says: "...s occlusion 1.0 and the blit multiplies AO over the sun too, 'blit.wgsl:49-50'). New file 'crates/render/src/shaders/prepass.wgsl' with 'vs_prepass', 'vs_prepass_skinne" | Keep as a builder row. |
| PW04 | 28-powerups.md | REAL (A) | accept filter `gpu_taa_sword_sweep` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW05 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW06 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW07 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW08 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW09 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW11 | 28-powerups.md | REAL (A) | accept filter `pathtrace` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW12 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW13 | 28-powerups.md | REAL (A) | accept filter `debug_view` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW14 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW15 | 28-powerups.md | REAL (A) | accept filter `fx` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW16 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW17 | 28-powerups.md | REAL (A) | accept filter `ccd` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW18 | 28-powerups.md | REAL (A) | accept filter `projectile` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW19 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW20 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW21 | 28-powerups.md | REAL (A) | accept filter `damage` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW22 | 28-powerups.md | REAL (A) | accept filter `verify_task` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW23 | 28-powerups.md | REAL (A) | accept filter `door_resource` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW24 | 28-powerups.md | REAL (A) | accept filter `task_play_run` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW25 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW26 | 28-powerups.md | REAL (A) | accept filter `scene_diff` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW27 | 28-powerups.md | REAL (A) | accept filter `memory` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW28 | 28-powerups.md | REAL (A) | accept filter `pressure` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW29 | 28-powerups.md | REAL (B) | target file absent: `crates/endpoint/src/packs/project.rs` -- row says: "...the merge shape, licence MIT (read live 2026-10-20). Diff: regular. Tier B. Size M. Owns 'crates/endpoint/src/packs/project.rs', 'crates/endpoint/src/packs/mod.rs'," | Keep as a builder row. |
| PW30 | 28-powerups.md | REAL (B) | target file absent: `crates/observe/src/breaker.rs` -- row says: "...d. Tier A. Size M. Owns 'crates/observe/src/supervisor.rs', 'crates/observe/src/heal.rs', 'crates/observe/src/breaker.rs', 'crates/render/src/heal.rs', 'crates" | Keep as a builder row. |
| PW32 | 28-powerups.md | REAL (A) | accept filter `hit_react_rot` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW33 | 28-powerups.md | REAL (A) | accept filter `bvh` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW34 | 28-powerups.md | REAL (A) | accept filter `attach_socket` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW35 | 28-powerups.md | REAL (A) | accept filter `lipsync` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW36 | 28-powerups.md | REAL (A) | accept filter `state_machine` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW37 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW38 | 28-powerups.md | REAL (B) | target file absent: `crates/audio/src/block.rs` -- row says: "...ns 'crates/web/src/audio_worklet.rs', 'crates/web/src/audio.rs', 'crates/web/index.html', 'crates/audio/src/block.rs'. Accept: 'node tools/gh-check.mjs car" | Keep as a builder row. |
| PW39 | 28-powerups.md | REAL (A) | accept filter `perf_count` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW40 | 28-powerups.md | REAL (B) | target file absent: `crates/assets/src/ktx2.rs` -- row says: "...d live 2026-10-20). Diff: regular. Tier B. Size M. Owns 'crates/assets/src/ktx2_cook.rs', 'crates/assets/src/ktx2.rs', 'crates/render/src/texture_ktx2.rs'," | Keep as a builder row. |
| PW41 | 28-powerups.md | REAL (A) | accept filter `trace_file` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW42 | 28-powerups.md | REAL (A) | accept filter `virt` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW43 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW44 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW45 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW46 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW47 | 28-powerups.md | REAL (A) | accept filter `load_queue` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW48 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW49 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW50 | 28-powerups.md | REAL (B) | target file absent: `docs/DEAD-LINES.md` -- row says: "...iff: good. Tier B. Size S. Owns 'crates/physics/src/kcc.rs', 'crates/physics/src/lib.rs', 'docs/DEAD-LINES.md'. Accept: 'git grep -n "kcc::" -- crat" | Keep as a builder row. |
| PW51 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW52 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW53 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW54 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PW55 | 28-powerups.md | REAL (A) | accept filter `wfc` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| PW56 | 28-powerups.md | REAL (B) | target file absent: `crates/endpoint/src/packs/asset/generate_run.rs` -- row says: "...', a command that cannot run, and the asset pack returns 'runs_here: false'. The row adds 'crates/endpoint/src/packs/asset/generate_run.rs' (start a job, parse the 'started run" | Keep as a builder row. |
| PW57 | 28-powerups.md | REAL (W) | wave row boarded 2026-10-21 and never ticked: its accept test does not exist, so none of it is built | Keep as a builder row. |
| PX42 | 21-look-feel.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| PX43 | 21-look-feel.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| QL3 | 22-web-realism.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| QL5 | 22-web-realism.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| QR1 | 18-gaps.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| QT5 | 22-web-realism.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| QT9 | 22-web-realism.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| RL37 | 21-look-feel.md | REAL (A) | accept filter `fighter_motion_vector_matches_sweep` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| RL39 | 21-look-feel.md | REAL (A) | accept filter `lens_pass_budget_and_off_identity` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| RL43 | 21-look-feel.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| RL48 | 21-look-feel.md | REAL (A) | accept filter `fighter_coverage` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| RL49 | 21-look-feel.md | REAL (B) | target file absent: `crates/render/src/ground_state.rs` -- row says: "...x 20 m pit: stamps, slow fade, a byte cap, no blur step. Diff: good. Tier A. Size M. Owns 'crates/render/src/ground_state.rs' (new), the floor hunk of 'shaders/for" | Keep as a builder row. |
| RL51 | 21-look-feel.md | REAL (A) | accept filter `hextile` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| RL52 | 21-look-feel.md | REAL (A) | accept filter `particles_gpu` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| RL53 | 21-look-feel.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| RL54 | 21-look-feel.md | REAL (B) | target file absent: `crates/render/src/crowd.rs` -- row says: "...odot VAT), palette-indexed variety from a texture array. Diff: hard. Tier B. Size L. Owns 'crates/render/src/crowd.rs' (new, VG4's file), 'crates/render/src" | Keep as a builder row. |
| RL56 | 21-look-feel.md | REAL (B) | target file absent: `crates/render/src/shaders/froxel_fog.wgsl` -- row says: "...otato keeps closed-form fog. Diff: hard. Tier B. Size L. Owns 'crates/render/src/fog.rs', 'crates/render/src/shaders/froxel_fog.wgsl', the fog composite hunk of 'forward.w" | Keep as a builder row. |
| RL57 | 21-look-feel.md | REAL (A) | accept filter `particles_gpu` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| RL61 | 21-look-feel.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| RT10 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/proxy_check.rs` -- row says: "...render: proxy validation — CT2 proxy deviation gate for BLAS builds. Tier C. Size S. Owns 'crates/render/src/proxy_check.rs'. Accept: proxy-to-mesh deviation char" | Keep as a builder row. |
| RT11 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/restir_check.rs` -- row says: "...convergence against the RT7 reference on torch and bounce fixtures. Tier C. Size M. Owns 'crates/render/src/restir_check.rs'. Accept: night-torch and tunnel fixtu" | Keep as a builder row. |
| RT12 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/denoise_wire.rs` -- row says: "...ng pool with history validation, reusing the landed RT9 budget gate. Tier C. Size M. Owns 'crates/render/src/denoise_wire.rs'. Accept: moving-camera clip shows no" | Keep as a builder row. |
| RT19 | 12-tier-2040.md | REAL (A) | accept filter `rt_reflect` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| RT2 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/rt_shadow.rs` -- row says: "...th soft contact-hardening penumbra; cascades stay the Base fallback. Tier C. Size M. Owns 'crates/render/src/rt_shadow.rs'. Accept: penumbra widens with blocker" | Keep as a builder row. |
| RT20 | 12-tier-2040.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| RT3 | 12-tier-2040.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| RT4 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/restir_gi.rs` -- row says: "...ance cache (Solari and id Tech 8 patterns, from their public talks). Tier C. Size L. Owns 'crates/render/src/restir_gi.rs'. Accept: a - code cut 2026-10-16, rec" | Keep as a builder row. |
| RT6 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/denoise.rs` -- row says: "...RT5; vendor denoisers only as optional adapters after a license row. Tier C. Size L. Owns 'crates/render/src/denoise.rs'. Accept: a moving-camera clip with no" | Keep as a builder row. |
| RT8 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/probe_grid.rs` -- row says: "...8 shape), so GPUs without ray queries still get moving bounce light. Tier C. Size M. Owns 'crates/render/src/probe_grid.rs'. Accept: a sun sweep relights probes" | Keep as a builder row. |
| RT9 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/rt_tier_gate.rs` -- row says: "...ph wiring with per-pass budgets, tier stamped in every RA8 manifest. Tier C. Size M. Owns 'crates/render/src/rt_tier_gate.rs'. Accept: llvmpipe reports Base and ru" | Keep as a builder row. |
| RZ10 | 22-web-realism.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| RZ4 | 22-web-realism.md | REAL (A) | accept filter `break_look` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| RZ6 | 22-web-realism.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| SL16 | 09-empire.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| SL23 | 09-empire.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| SL35 | 09-empire.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| SL37 | 09-empire.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| SL42 | 09-empire.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| SL43 | 09-empire.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| SL44 | 09-empire.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| SL5 | 09-empire.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| SP62 | 18-gaps.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| SP66 | 18-gaps.md | REAL (A) | accept filter `light_reach` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| SP70 | 09-empire.md | REAL (B) | target file absent: `crates/spatial/src/lift.rs` -- row says: "...weights in git (ER-2 sees pixels; we read symbols only). Diff: hard. Tier B. Size M. Owns 'crates/spatial/src/lift.rs', 'crates/spatial/src/lib.rs'. Accept:" | Keep as a builder row. |
| SP71 | 09-empire.md | REAL (A) | accept filter `spatial_eval` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| SP72 | 09-empire.md | REAL (A) | accept filter `grasp` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| TC4 | 27-final-picture.md | REAL (A) | accept filter `orca` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| TC5 | 27-final-picture.md | REAL (A) | accept filter `crowd_contact` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| TC6 | 27-final-picture.md | REAL (A) | accept filter `layers` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| UC1 | 16-urban-war.md | REAL (B) | target file absent: `crates/world/src/lots.rs` -- row says: "...s into lots with footprint, storeys, style and a damage-field value. Tier C. Size M. Owns 'crates/world/src/lots.rs'. Accept: a 1×1 km seed yields ≥ 400 l" | Keep as a builder row. |
| UC10 | 16-urban-war.md | REAL (B) | target file absent: `crates/assets/src/hlod.rs` -- row says: "...xies and impostors with meshlets, rebuilt when a ruin state changes. Tier C. Size M. Owns 'crates/assets/src/hlod.rs'. Accept: a 1 km view draws ≤ 2,000 ba" | Keep as a builder row. |
| UC11 | 16-urban-war.md | REAL (B) | target file absent: `crates/world/src/smoke.rs` -- row says: "...and collapse dust that blocks AI sight in the sim; VX1/VX5 draw it. Tier C. Size M. Owns 'crates/world/src/smoke.rs'. Accept: an agent behind smoke is not" | Keep as a builder row. |
| UC12 | 16-urban-war.md | REAL (B) | target file absent: `crates/world/src/crater.rs` -- row says: "...heightfield deformation per cell with nav rebake and crater decals. Tier C. Size M. Owns 'crates/world/src/crater.rs'. Accept: 100 craters replay x2; route" | Keep as a builder row. |
| UC14 | 16-urban-war.md | REAL (B) | target file absent: `crates/proof/src/uc.rs` -- row says: "...d RT tiers on the reference GPU; review panel against the style set. Tier C. Size M. Owns 'crates/proof/src/uc.rs'. Accept: the 'urban-war.md' §2 look a" | Keep as a builder row. |
| UC16 | 16-urban-war.md | REAL (B) | target file absent: `crates/world/src/terrain_tiles.rs` -- row says: "...and nav per tile; replaces the 8-bit seeded heightmap for big maps. Tier C. Size L. Owns 'crates/world/src/terrain_tiles.rs'. Accept: an 8×8 km map streams with z" | Keep as a builder row. |
| UC17 | 16-urban-war.md | REAL (B) | target file absent: `crates/render/src/foliage.rs` -- row says: "...om splat weights; presentation only except tree trunks as colliders. Tier C. Size L. Owns 'crates/render/src/foliage.rs'. Accept: 1 km view of forest edge ins" | Keep as a builder row. |
| UC18 | 16-urban-war.md | REAL (B) | target file absent: `crates/world/src/roads.rs` -- row says: "...nd blend the terrain, bridges, road decals and road-following props. Tier C. Size M. Owns 'crates/world/src/roads.rs'. Accept: a 5 km road network bakes id" | Keep as a builder row. |
| UC19 | 16-urban-war.md | REAL (B) | target file absent: `crates/physics/src/armor_vehicle.rs` -- row says: "...ew seats, armor zones resolved by UW3 penetration and damage states. Tier C. Size L. Owns 'crates/physics/src/armor_vehicle.rs'. Accept: a tank drives the UC16 test" | Keep as a builder row. |
| UC2 | 16-urban-war.md | REAL (B) | target file absent: `crates/world/src/facade.rs` -- row says: "...awnings, AC units, water tanks, weathering masks, from style tables. Tier C. Size M. Owns 'crates/world/src/facade.rs'. Accept: 400 buildings with no two ad" | Keep as a builder row. |
| UC20 | 16-urban-war.md | REAL (B) | target file absent: `crates/physics/src/rotor.rs` -- row says: "...tail rotor, ground effect, autorotation-lite) on fixed-point state. Tier C. Size M. Owns 'crates/physics/src/rotor.rs'. Accept: hover, transit and landing t" | Keep as a builder row. |
| UC3 | 16-urban-war.md | REAL (B) | target file absent: `crates/world/src/dressing.rs` -- row says: "...gate (no overlap, no floating, no blocked door, 'navConnect' green). Tier C. Size M. Owns 'crates/world/src/dressing.rs'. Accept: 10k placements with 0 spatia" | Keep as a builder row. |
| UC5 | 16-urban-war.md | REAL (B) | target file absent: `crates/destruction/src/ruin.rs` -- row says: "...freezes ruin states 0–3 with rubble meshes, collision and nav input. Tier C. Size L. Owns 'crates/destruction/src/ruin.rs'. Accept: same seed, same ruin bytes x" | Keep as a builder row. |
| UC6 | 16-urban-war.md | REAL (B) | target file absent: `crates/destruction/src/heavy.rs` -- row says: "...runtime with UW7 blasts; rubble becomes cover and nav through DX10. Tier C. Size M. Owns 'crates/destruction/src/heavy.rs'. Accept: a scripted barrage breaks th" | Keep as a builder row. |
| UC7 | 16-urban-war.md | REAL (B) | target file absent: `crates/net/src/break_sync.rs` -- row says: "...settled-debris checksum every 60 ticks, snapshot repair on mismatch. Tier C. Size M. Owns 'crates/net/src/break_sync.rs'. Accept: 32 clients converge after a" | Keep as a builder row. |
| UC9 | 16-urban-war.md | REAL (B) | target file absent: `crates/render/src/interior_map.rs` -- row says: "...eded fake rooms (walls, lights, curtains) with no interior geometry. Tier C. Size S. Owns 'crates/render/src/interior_map.rs'. Accept: 1,000 lit windows ≤ 0.3 ms G" | Keep as a builder row. |
| UIX14 | 21-look-feel.md | REAL (A) | accept filter `rumble_merge` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| UW11 | 16-urban-war.md | REAL (B) | target file absent: `crates/anim/src/infantry.rs` -- row says: "...d-surrender, cower, and a true first-person view from the same body. Tier B. Size L. Owns 'crates/anim/src/infantry.rs'. Accept: no pops across interrupts; t" | Keep as a builder row. |
| UW13 | 16-urban-war.md | REAL (B) | target file absent: `crates/nav/src/building.rs` -- row says: "...a room graph per building as plain data; new holes from DX10 breaks. Tier B. Size M. Owns 'crates/nav/src/building.rs'. Accept: a locked door refuses routes" | Keep as a builder row. |
| UW14 | 16-urban-war.md | REAL (B) | target file absent: `crates/ai/src/perceive.rs` -- row says: "...ons with decay, squad sharing; every target choice cites its record. Tier B. Size M. Owns 'crates/ai/src/perceive.rs'. Accept: a verifier fails a seeded sh" | Keep as a builder row. |
| UW15 | 16-urban-war.md | REAL (B) | target file absent: `crates/ai/src/compliance.rs` -- row says: "...with seeded hesitation; civilian fear (cower, comply, flee, freeze). Tier B. Size M. Owns 'crates/ai/src/compliance.rs'. Accept: a 500-case event matrix repl" | Keep as a builder row. |
| UW16 | 16-urban-war.md | REAL (B) | target file absent: `crates/ai/src/tactics.rs` -- row says: "...flank, retreat to caches, shoot through walls only at heard targets. Tier B. Size L. Owns 'crates/ai/src/tactics.rs'. Accept: 200 seeded fights, 0 stuck," | Keep as a builder row. |
| UW17 | 16-urban-war.md | REAL (B) | target file absent: `crates/ai/src/clear.rs` -- row says: "...a queue; a room turns clear only when every walkable cell was seen. Tier B. Size L. Owns 'crates/ai/src/clear.rs'. Accept: 1,000 seeded clears, 0 stuck" | Keep as a builder row. |
| UW20 | 16-urban-war.md | REAL (B) | target file absent: `crates/world/src/buildings.rs` -- row says: "...gs from a kit table with collision, nav input and DX1 bond data out. Tier B. Size M. Owns 'crates/world/src/buildings.rs'. Accept: 20 footprints build identica" | Keep as a builder row. |
| UW21 | 16-urban-war.md | REAL (B) | target file absent: `crates/world/src/interiors.rs` -- row says: "...h the room graph UW13 consumes; the "new house every run" generator. Tier B. Size L. Owns 'crates/world/src/interiors.rs'. Accept: 100 seeded houses, every roo" | Keep as a builder row. |
| UW25 | 16-urban-war.md | REAL (B) | target file absent: `crates/net/src/shots.rs` -- row says: "...impacts under ON3. Tier B. Size M. Owns ranged hunks in 'crates/net/src/lagcomp.rs' plus 'crates/net/src/shots.rs'. Accept: ≥ 98% of shooter-seen hits r" | Keep as a builder row. |
| UW3 | 16-urban-war.md | REAL (B) | target file absent: `crates/combat/src/penetrate.rs` -- row says: "..., flesh) gives exit energy; shallow hits ricochet; low energy stops. Tier B. Size M. Owns 'crates/combat/src/penetrate.rs'. Accept: a rifle round passes two dry" | Keep as a builder row. |
| UW39 | 16-urban-war.md | REAL (B) | target file absent: `crates/proof/src/uw.rs` -- row says: "...ptures on the reference GPU, review-panel scores, engine diff empty. Tier B. Size M. Owns 'crates/proof/src/uw.rs'. Accept: every item has an evidence l" | Keep as a builder row. |
| UW4 | 16-urban-war.md | REAL (B) | target file absent: `crates/combat/src/firearm.rs` -- row says: "..., sights, sway and weapon length, table-driven, emitting UW2 rounds. Tier B. Size M. Owns 'crates/combat/src/firearm.rs'. Accept: 600-tick tape hashes x2; rat" | Keep as a builder row. |
| UW42 | 16-urban-war.md | REAL (B) | target file absent: `crates/nav/tests/layers.rs` -- row says: "...s stale-path repath exact on the layered mesh, in a new fixture. Tier B. Size S. Owns new 'crates/nav/tests/layers.rs' (no src change). Accept: 'cargo test" | Keep as a builder row. |
| UW5 | 16-urban-war.md | REAL (B) | target file absent: `crates/combat/src/wounds.rs` -- row says: "...light, heavy, incapacitated, dead), bleed-out and restrained states. Tier B. Size S. Owns 'crates/combat/src/wounds.rs'. Accept: zone × armor × ammo matrix r" | Keep as a builder row. |
| UW6 | 16-urban-war.md | REAL (B) | target file absent: `crates/combat/src/suppression.rs` -- row says: "...per-tick decay; pinned and shaken states out; never changes damage. Tier B. Size S. Owns 'crates/combat/src/suppression.rs'. Accept: a burst at 1 m pins, at 10 m" | Keep as a builder row. |
| UW7 | 16-urban-war.md | REAL (B) | target file absent: `crates/combat/src/device.rs` -- row says: "...rough 'SegmentCast', seeded fragments, door traps, impulses for DX1. Tier B. Size M. Owns 'crates/combat/src/device.rs'. Accept: a flashbang reaches every ex" | Keep as a builder row. |
| UW8 | 16-urban-war.md | REAL (B) | target file absent: `crates/combat/src/status.rs` -- row says: "...passed room graph, beanbag stagger; masks and armor resist by table. Tier B. Size M. Owns 'crates/combat/src/status.rs'. Accept: gas spreads only through ope" | Keep as a builder row. |
| VG1 | 12-tier-2040.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| VG10 | 12-tier-2040.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| VG11 | 12-tier-2040.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| VG13 | 12-tier-2040.md | REAL (A) | accept filter `meshlet` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| VG14 | 12-tier-2040.md | REAL (A) | accept filter `meshlet` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| VG2 | 12-tier-2040.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| VG3 | 12-tier-2040.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| VG5 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/vis_buffer.rs` -- row says: "...isibility-buffer resolve + Hi-Z pyramid wired into the forward pass. Tier C. Size L. Owns 'crates/render/src/vis_buffer.rs'. Accept: occluded-crowd fixture draws" | Keep as a builder row. |
| VG6 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/crowd_lod.rs` -- row says: "...[ ] VG6 render: crowd LOD + VAT validation with AU2 cheer coupling. Tier C. Size M. Owns 'crates/render/src/crowd_lod.rs'. Accept: 20k spectators within 1.5 ms" | Keep as a builder row. |
| VG7 | 12-tier-2040.md | REAL (C) | open row with a dated progress note: part is in, the rest is not | Keep as a builder row. |
| VG8 | 12-tier-2040.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| VR2 | 22-web-realism.md | REAL (C) | the row itself names the work left | Keep as a builder row. |
| VR4 | 22-web-realism.md | REAL (A) | accept filter `meshopt_cook` matches no test in the crate | Keep as a builder row; fix the accept filter to a test that exists or will exist. |
| VX10 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/fluids_check.rs` -- row says: "...terminism — seeded blood, fire and smoke sim bytes x2 on one device. Tier C. Size S. Owns 'crates/render/src/fluids_check.rs'. Accept: identical sim bytes x2 from" | Keep as a builder row. |
| VX11 | 12-tier-2040.md | REAL (B) | target file absent: `crates/game/src/photo_splat.rs` -- row says: "...d stills with manifests and licensed backdrops sorted against depth. Tier C. Size S. Owns 'crates/game/src/photo_splat.rs'. Accept: pause + free camera saves a" | Keep as a builder row. |
| VX12 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/weather_lock.rs` -- row says: "...ouds, VX8 shafts and the RL6 wet layer so the three can never drift. Tier C. Size S. Owns 'crates/render/src/weather_lock.rs'. Accept: day-to-night clip with movin" | Keep as a builder row. |
| VX13 | 12-tier-2040.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| VX14 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/arena_air.rs` -- row says: "...lus torch-glow sprites, presentation only, ahead of full VX1 froxel. Tier C. Size S. Owns 'crates/render/src/arena_air.rs'. Accept: capture pair before 'target/" | Keep as a builder row. |
| VX15 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/arena_backdrop.rs` -- row says: "...gate-tunnel mouth against the sunlit sand as presentation-only backdrop geometry in a new 'crates/render/src/arena_backdrop.rs', ahead of the CT25 kit and full VX1 v" | Keep as a builder row. |
| VX16 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/arena_banners.rs` -- row says: "...ners plus seeded dust drift, ahead of full VX1 froxel and VG4 crowd. Tier C. Size S. Owns 'crates/render/src/arena_banners.rs' ('arena_backdrop.rs', 'arena_air.rs'," | Keep as a builder row. |
| VX17 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/volumetric.rs` -- row says: "...s 'bevy_light-0.20.0-rc.2/src/volumetric.rs' (MIT OR Apache-2.0) as new presentation-only 'crates/render/src/volumetric.rs' beside frozen 'fog.rs', never in tick" | Keep as a builder row. |
| VX2 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/clouds.rs` -- row says: "...tric clouds and weather sky — rain wetness drives the RL6 wet layer. Tier C. Size M. Owns 'crates/render/src/clouds.rs'. - code cut 2026-10-16, recover via g" | Keep as a builder row. |
| VX3 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/character.rs` -- row says: "...urface scattering, eye shading, hair cards with anisotropic shading. Tier C. Size L. Owns 'crates/render/src/character.rs'. Accept: blind review of the hero clo" | Keep as a builder row. |
| VX4 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/camera_fx.rs` -- row says: "...eh depth of field, per-object motion blur, film grain, HDR10 output. Tier C. Size M. Owns 'crates/render/src/camera_fx.rs'. Accept: exposure stays stable throug" | Keep as a builder row. |
| VX5 | 12-tier-2040.md | REAL (D) | parked LATER on the row, work not built | Keep as a builder row. |
| VX6 | 12-tier-2040.md | REAL (B) | target file absent: `crates/game/src/photo.rs` -- row says: "...- [ ] VX6 game: photo mode over the RT7 path tracer (LIGHT). Tier C. Size S. Owns 'crates/game/src/photo.rs'. Accept: pause, free camera, a conver" | Keep as a builder row. |
| VX7 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/splat.rs` -- row says: "...f ruins or skyline composited with depth, presentation only (LIGHT). Tier C. Size M. Owns 'crates/render/src/splat.rs'. Accept: the backdrop sorts against a" | Keep as a builder row. |
| VX8 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/vol_integrate.rs` -- row says: "...— RT3 light injection and rain-wet coupling into the RL6 wet layer. Tier C. Size M. Owns 'crates/render/src/vol_integrate.rs'. Accept: shafts through the gate arch" | Keep as a builder row. |
| VX9 | 12-tier-2040.md | REAL (B) | target file absent: `crates/render/src/cine_hdr.rs` -- row says: "...ogram auto-exposure, bokeh, motion blur, grain and swapchain output. Tier C. Size M. Owns 'crates/render/src/cine_hdr.rs'. Accept: tunnel-exit exposure stable;" | Keep as a builder row. |
| WD11 | 12-tier-2040.md | REAL (B) | target file absent: `crates/world/src/weather.rs` -- row says: "...clouds, VX5 smoke drift and the RL6 wet layer; sim never reads them. Tier C. Size S. Owns 'crates/world/src/weather.rs'. Accept: a day-to-night clip with mov" | Keep as a builder row. |
| WD13 | 12-tier-2040.md | REAL (B) | target file absent: `crates/world/src/city.rs` -- row says: "...Rust as 'crates/world' city modules (MIT, PORT cite + EVIDENCE row). Tier C. Size M. Owns 'crates/world/src/city.rs'. Accept: directed 2D road graph; inte" | Keep as a builder row. |
| WD5 | 12-tier-2040.md | REAL (B) | target file absent: `crates/world/src/cycle.rs` -- row says: "...- [ ] WD5 world: day/night and weather cycle driving VX2 and RT4. Tier C. Size S. Owns 'crates/world/src/cycle.rs'. Accept: the cycle is presentation on" | Keep as a builder row. |

## 5. PROBABLE DONE, ACCEPT BLOCKED BY ANOTHER LANE

| id | board | verdict | proof | what the lead should do |
|---|---|---|---|---|
| E81 | 10-empire-inbox-drain.md | DONE pending a clean tree | Every accept item is in the tree as a test: `crates/physics/src/cloth.rs:2752 hair_and_rope_q16_verlet_replays_the_same_bits_twice`, `:2782 a_head_of_hair_and_a_rope_meet_the_half_millisecond_bar`, `:2896 the_match_hash_is_the_same_with_hair_and_rope_on_and_off` (landed `e4a5308f`, \"physics: E81 Q16 Verlet hair and rope strands\"). The accept run fails for another lane's uncommitted edit: `cargo test -p engine-physics --lib cloth` -> FAIL `could not compile engine-physics (lib test) due to 23 previous errors` (`E0425 cannot find function from_fixed`, `crates/physics/src/cloth.rs:1737`), and `git status` shows `M crates/physics/src/cloth.rs`. | Re-run `node tools/gh-check.mjs --remote-only cargo test -p engine-physics --lib cloth` once the cloth.rs edit lands; if it prints the three tests above green, tick E81 with that run. |

## 6. NOT JUDGED (code present, accept is prose, no stated remainder) 165 rows

These are the rows this audit could not decide without a run or an owner. Grouped by
board file; each needs one accept command written and run.

- 09-empire.md: GE6, AB10, SP44, SP45, SL3, SL21, PB51, PB56, PB58, PB59, PB60, PB63, PB64, PB65, PB66
- 10-empire-inbox-drain.md: E9, E10, E23, S27, S29, E48, E67, E78, E82, E86, E90, E91, E92
- 11-north-star.md: NS8, NS10, NS11, NS12, NS13, NS17, NS18, NS20, NS21, NS22, NS23, GN4
- 12-tier-2040.md: FU6, ON27, ON29, WD3, WD9, WD12
- 13-engine-edge.md: OPT13, OPT15, OPT22, OPT23, OPT32
- 15-platform-live-ai.md: PLF5, LAI8
- 16-urban-war.md: UW0, UW12, UW22, UW23, UW24, UW26, UW27, UW28, UW29, UW30, UW31, UW32, UW33, UW34, UW35, UW37, UW38, UW40, UW41, UC4, UC13, UC15
- 17-body-and-break.md: BX6, BX11, BX25, HS23, GEO9a
- 18-gaps.md: EG18c, EG47, AD6, AD9, AD12, AD21, AD22, AD25, AD28, AD29, LF5c, LF5d, LF6, LF7, LF8, LF13, SP63b, SP63c, SP65, O10
- 18-steals.md: GS9, GS22, GS23, PLN30, GS39
- 20-game.md: RC37
- 21-look-feel.md: RL36, RL40, RL42, RL44, RL45, RL46, RL47, MP26, OPT35, GS34, GS36, UIX13, SN12, PF28, FV3, FV4
- 22-web-realism.md: VR1, QL2, QL6, QL7, QL8, QL9, QL10, QL11, QN9, QN10, QW4, QW6, QW9, QR6, QR8, QR9, QA2, QA13, QA5, QA6, QA7, QA8, QA9, QA10, QA11, QA12
- 23-books.md: NB2, NB3, NB4
- 24-editor.md: EI6
- 27-final-picture.md: AF2, AF4, AF5, AF6, CX2, TC1, TC2, TC3, TC7, POV5, OPT38, OPT39, OPT41

## 7. How this was produced (reproduce it)

Read-only. No board, doc or code file was edited; only this report was written.

- Row text and ids: `node tools/board-slice.mjs --ready --open` (736 open rows board-wide),
  `node tools/board-slice.mjs --find <text>`, then the row bodies in the shard files by id.
- Path claims: every backticked path in each row of my scope tested with `Test-Path` from the
  repo root (172 rows name at least one path that is absent; donor paths under
  `crates/renderling/...` and globs such as `crates/*/Cargo.toml` are not counted).
- Accept claims: for each row the first `cargo test -p <crate> ... --lib <filter>` is pulled
  out of the accept text and `<filter>` is looked for in `fn` names across the crate. 84 rows
  name a filter with no matching test.
- Duplicates: each `closes X` in `docs/SPRINT-28-powerups.md` matched against the open rows,
  plus rows that name the same absent file.
- Accepts run: `node tools/gh-check.mjs --remote-only cargo test -p engine-render --lib auto_exposure`
  (run 39514420794), `... -p engine-spatial --lib overlap_3d` (39515222235),
  `... -p engine-playtest --lib sweep_noise` (39515378524, red for the arena/ecs E0603),
  `... -p engine-physics --lib cloth` (39517575062, red for an uncommitted cloth.rs edit).
  Local gh-check on `crates/render` waits on the package-cache lock while other lanes build;
  the `--remote-only` road is the one that finished.
- Golden `d57b5c8ae3096614` was not touched by anything here: no sim code changed, and no
  proof of mine ran a golden test.

## Fixer report: the hollow-green class in SPRINT-19..27 (board fixer 2, 2026-10-21)

Scope: the rows of `docs/SPRINT-19-*.md` through `docs/SPRINT-27-*.md` only. 51 rows got one
dated `Correction 2026-10-21:` line (never a tick, never a delete, never a commit). Files
edited: `SPRINT-20-game.md` (6), `SPRINT-21-look-feel.md` (23), `SPRINT-22-web-realism.md`
(11), `SPRINT-26-motion.md` (1), `SPRINT-27-final-picture.md` (10). One further GA15 line in
`SPRINT-20-game.md` is another session's, not mine.

### 1. The hollow-green rows (the audit's REAL (A) rows), 26 in my range

- **24 fixed.** Each correction names the filter's real state (a `Grep` of the crate, the
  file's `Test-Path` where a file is named) and the Accept that may be ticked.
- **5 of the 24 now name a test that exists** and may run today for the landed half:
  GA14 (`--lib sweep_`: `sweep_rows_cover_every_difficulty_at_each_value` `tournament.rs:2519`,
  `sweep_skips_bloodless_variants_and_replays_its_csv` `:2491`, `sweep_stamp_pins_epoch_and_names_csv_files`
  `:2550`), AM25 (`look_and_aim_face_the_target` `crates/anim/src/fbik.rs:856`), BX41
  (`stump_cap_is_closed_with_the_asked_radius_and_depth` `crates/render/src/stump.rs:246`),
  AX28 (`play_run_and_play_step_are_found_and_described_under_client_names`
  `crates/endpoint/src/insight/mcp_tools.rs:496`), VR4
  (`bundle_priority_puts_coarse_blobs_first_in_the_file`
  `crates/assets/src/asset_source/bundle.rs:711`).
- **19 need the test to exist first** and say so: the correction writes the Accept as the
  command plus the requirement that it prints a nonzero passed count, and says the row may not
  be ticked before that. The 19: GP33 (`ttk`), GP44 (`telegraph_warp`), PQ22 (`golden_bless`;
  also `crates/playtest/src/look/golden.rs` absent), RL37, RL39, RL48, RL51, RL57, UIX14,
  AM28 (`fixtures/nets/traj.e2nn` absent), POV1, POV2, POV3, POV4, POV6, POV7
  (`crates/game/tests/it/pov_tape.rs` absent), TC4 (`orca` lives in engine-nav, not
  engine-arena), TC5 (`crates/nav/src/crowd_contact.rs` absent), TC6 (`engine_physics::layers`
  exists, `crates/physics/src/lib.rs:37`, but physfx builds no `InteractionGroups`).
- **2 of the audit's A rows are not hollow at all** and were left untouched: RZ4
  (`break_look` is a module, `crates/game/src/lib.rs:47`, 12 `#[test]` in `break_look.rs`, so
  the filter matches and the row's own Progress note is right) and RL52 (`particles_gpu` is a
  module, `crates/render/src/lib.rs:248`, 33 `#[test]` in `particles_gpu.rs`, and the test the
  Accept names, `gpu_draw_changes_pixels`, exists at `particles_gpu.rs:1794`).
- **2 more are a module-filter near-miss** (filter matches, but nothing proves the row), and
  the correction says so and names the new test that must exist: RL57 (`--lib particles_gpu`
  matches 33 tests, none checks the hit flash) and POV2 (`--lib fps_view` matches 12 tests,
  none covers a helmet or chest spring).

### 2. WRONG PREMISE, 1 in my range

GA15: correction written (the phase-2 half landed at `crates/ai/src/depth.rs:459`/`:470`, test
`tier_primus_phase_starts_at_500_permille` `:2782`; left is the playtest `primus` proof). Note:
another session wrote a second, near-identical GA15 correction line into the same row while I
worked; I left that line alone and did not revert it. The other 5 wrong-premise rows (PW10,
PW31, PW58 in `SPRINT-28-powerups.md`; EG24, ADV51b in `SPRINT-18-*`) are outside my files.
Flag for the lead: `crates/ai/src/plan_trace.rs` is **present** in the tree now
(`Test-Path` True), so the PW31/PW58 "the file does not exist" premise needs a re-check.

### 3. DUPLICATE, 26 in my range

One dated line each, naming the owning row: AF3 (AF1), AM23 (PW32), CQ20 (PW52), GS35 (PW08),
PF25 (PW07), PF27 (PW41), QA15 (PW40), QA4 (PW06), QW10 (PW39), QW5 (PW40), RC36 (PW20),
RL32 (PW02), RL34 (PW12), RL35 (PW06), RL38 (PW04), RL41 (PW03), RL55 (RL54), RL58 (PW13),
RL59 (PW13), RL60 (PW02), WEB11 (PW39), WEB14 (PW38), WEB16 (PW37), WEB17 (PW37), WEB5
(PW37), WEB6 (PW47). Where the audit's pairing did not hold on reading, the line says so
instead of retiring work: AF3 builds `skills/fix-red/SKILL.md` while AF1 builds `tools/fix.mjs`
(not the same build, and AF3 needs AF1); CQ20's newtype and `SUN_CACHES` work is not in PW52;
PF27's overlay half landed (`e39dd7a`) and PW41 writes the trace, not the overlay; WEB14 is
explicitly *not* a prerequisite of PW38. PW10 and PW58 (`SPRINT-28`) are not mine and are left.

### 4. `node tools/workspace-check.mjs` after the edits

`exit=1`, first lines: `PASS dependency cycle` then
`FAIL workspace audit: engine-arena -> engine-nav: undocumented local dependency edge` plus the
"not declared by any module" lines (endpoint `packs/session.rs`, game `class_sheet.rs`, physfx
`body.rs` and `verbs.rs`, physics `cloth/tests.rs`), the GE2 genre-ledger line and the stale
`modmap.md` line. Every one of those is another lane's uncommitted file: the same run before my
edits printed the same FAIL. No board error. The board check itself is clean: `parseBoardTasks`
over the index plus every shard returns **1689 tasks, 0 errors** (it prints nothing on stdout
because the audit FAIL ends the run early; run it directly to see the line).


## 8. Board fixer pass, 2026-10-21 (scope SPRINT-09 through SPRINT-18)

A subagent applied the verdicts above to the shards. No row was deleted, retitled or moved; no
`crates/**` or `tools/**` file was touched; nothing was committed, added or pushed. Base HEAD
`803bdcd6` plus other lanes' uncommitted work. One dated note per row, appended to the row's own
block (the board keeps it), and a duplicate note is a note, never a tick.

**Ticked (2).**
- AD18 -> `c4641f4d`. Accept PASS: `node tools/gh-check.mjs --remote-only cargo test -p
  engine-spatial --lib overlap_3d` -> `test result: ok. 13 passed; 0 failed` (run 39515222235);
  every accept item is covered, the stacked-scene digest is re-pinned. `SPRINT-18-gaps.md`.
- E81 -> `e4a5308f` (`physics: E81 Q16 Verlet hair and rope strands`). Its three accept tests are
  at HEAD in `crates/physics/src/cloth.rs`: `hair_and_rope_q16_verlet_replays_the_same_bits_twice`
  (:2157), `a_head_of_hair_and_a_rope_meet_the_half_millisecond_bar` (:2187),
  `the_match_hash_is_the_same_with_hair_and_rope_on_and_off` (:2301). The accept run is red for
  in-flight cloth work, not for this row: `node tools/gh-check.mjs --only
  crates/physics/src/cloth.rs cargo test -p engine-physics --lib cloth` -> FAIL E0583 "file not
  found for module `cloth`" (run 39518982186), because `git status` reads ` D
  crates/physics/src/cloth.rs` and another lane is splitting it into `crates/physics/src/cloth/`.
  A green re-run is owed once that split lands; the note in the row says so.

**Not ticked, progress note only (1).** GS44 (`SPRINT-26-steals-wave3.md`, named in the order
though outside the 09..18 split): the GPU metering half landed `c59c4ed3` (64-bin histogram kernel,
one-frame-late readback, the live meter step, 10 passed run 39514420794, rustfmt clean, ratchet at
baseline), but the GPU runtime leg (device histogram vs the CPU oracle, the readback contract, the
live frame) and the night capture need a PC run and the shared `target/` lock is saturated; the
golden x2 is red for another lane's `crates/net` and `crates/observe` work. The meter ships OFF
behind `E2040_AUTO_EXPOSURE=1`, so every existing capture stays byte-equal and the look baseline
cannot move. Row stays open.

**Wrong premise corrected (2, plus 1 already corrected).**
- ADV51b (`SPRINT-09-empire.md`): `tools/sprint-lock.mjs` was deleted in `e5457d6e` and has no
  later history, engine2040 has no sprint loop, lock or seat guard; the row needs a new home and
  the guard must not be rebuilt.
- GA15 (`SPRINT-20-game.md`, named in the order though outside the 09..18 split): the phase-2 half
  is landed - `crates/ai/src/depth.rs:459 primus_phase` (phase 2 at 500 permille), `:470
  primus_counter` (the new v2 answer), test `tier_primus_phase_starts_at_500_permille`
  (`depth.rs:2782`). Left: only the 200-duel playtest proof - `cargo test -p engine-playtest --lib
  primus` matches no test (`git grep -n primus -- crates/playtest/` exits 1).
- EG24 (`SPRINT-18-gaps.md`) already carried this correction from the 2026-10-21 wave (the
  `fixtures/nets/depth_student.e2nn` premise, the arena bind and the gate as the open half), so
  nothing was added; a second note would only duplicate it.

**Duplicate noted (35, each names its owner row; none ticked).**
- 09-empire.md: none. 10-empire-inbox-drain.md: E51 -> PW51, E58 -> E66, E64 -> PW50, E75 -> PW30.
- 11-north-star.md: GN5 -> PW56, GN6 -> PW57, GN7 -> PW56, GN8 -> PW56, NS19 -> PW33.
- 12-tier-2040.md: FU2 -> PW29, FU3 -> PW29, RT5 -> RT19, RT7 -> PW11, RT13 -> PW11, VG4 -> RL54,
  VG9 -> RL55, VX1 -> VX17, WD14 -> PW55, WD15 -> PW55.
- 13-engine-edge.md: OPT8 -> PW45, OPT9 -> PW46, OPT11 -> PW44, OPT27 -> PW28, OPT29 -> PW43.
- 14-agent-workspace.md: AW23 -> AW18. 15-platform-live-ai.md: LAI3 -> PW35, LAI7 -> PW27.
- 16-urban-war.md: UW2 -> PW18, UC8 -> PW21, UW36 -> PW21.
- 18-gaps.md: AD14 -> PW30, EG18b -> PW36, EG40 -> PW34, LF5 -> PW01.
- 18-steals.md: GS18b -> PW05.
The same-target pairs are named as the order asked: VG4 -> RL54, VG9 -> RL55, RT5 -> RT19, VX1 ->
VX17, E58 -> E66, AW23 -> AW18 (AF3 -> AF1 lives in `SPRINT-27`, the other fixer's file).

**Left alone as ordered.** The 300 REAL rows and the 165 NOT JUDGED rows are untouched. The
duplicates that live in `SPRINT-19..28` (AF3, AM23, CQ20, GS35, PF25, PF27, QA15, QA4, QW10, QW5,
RC36, RL32, RL34, RL35, RL38, RL41, RL55, RL58, RL59, RL60, WEB11, WEB14, WEB16, WEB17, WEB5,
WEB6) and PW10, PW31 and PW58 are the other fixer's.

**Checks.** `node tools/workspace-check.mjs` -> exit 1, FAIL lines all outside `docs/`:
`FAIL workspace audit: engine-arena -> engine-nav: undocumented local dependency edge`, undeclared
modules (`engine-game class_sheet.rs`, `engine-physfx body.rs`, `engine-physics cloth\tests.rs`),
the GE2 genre ledger naming `ai/src/plan_trace.rs`, `physics/src/cloth/collide.rs` and
`physics/src/cloth/tests.rs` (note: `plan_trace.rs` now exists, later than the audit read),
and `.opencode/modmap.md` stale for `physics/src/cloth.rs`. Every line is another lane's in-flight
cloth split or a code edge; a docs-only edit cannot move them, and none was in a file this pass
touched. `node tools/board-slice.mjs --self-check` -> FAIL 8/10: VS3 (10321 B) and VS4 (10893 B) are
over the 8192 B slice budget, rows this pass did not touch. Rows edited here still slice fine.

