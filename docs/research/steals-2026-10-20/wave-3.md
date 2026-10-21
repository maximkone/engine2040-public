# Steals wave 3: the audit first (2026-10-20)

Ask (owner, 2026-10-20): "can we do a big wave of steals from all GitHub engines now,
big and fast, so the engine is still above them in 2040?"

Answer: yes, but the wave is six rows, not twenty. Two earlier waves (74 scouts)
plus this repo's own gap list are now partly stale: the obvious steals have
landed, and several "missing" items in `docs/plan/finish-line.md` already exist
in the tree. Boarding them again would spend sessions on duplicate work. So the
first half of this dive is an audit; the second half is the six rows in
`docs/SPRINT-26-steals-wave3.md`.

## State, end of 2026-10-20

| Row | State | Evidence |
|---|---|---|
| GS43 contact shadows | **landed** (`baf9c07`, board `838665d`) | `--lib gtao` 49 passed; live-frame feet test 8.4% / 8.8% mean luma drop; arena capture pair 519 darker / 0 lighter; a fresh HEAD build reproduces both capture shas |
| GS44 auto exposure | **landed** (`c59c4ed3`, board `45d550aec`) | meter + 64-bin GPU histogram + blit stops; 13 render tests green (10 remote, 3 GPU on this PC) plus the blit test; golden x2 green (run 39608142881); the captured night arena waits on VS7 (torches as lights) |
| GS45 pass report | **landed** (`a8be8c9`, board `1e372c6`) | `--lib pass_report` 6 passed; a missing writer names pass and texture; JSON byte-equal x2 |
| GS46 world.query verb | landed earlier (`008c197`) | 146 packs/door tests |
| GS48 AI decision log | landed earlier (`4f00b39`) | 5 ai + 4 door + 21 play-pack tests |

Branches: `git branch -a` shows `master` only (plus the `origin-tags/lucid-hopper-mcam05`
tag ref); there is no unmerged feature branch to pick up. The engine is one tree.

## Method

Five checks, all against the tree on 2026-10-20, no engine built or benchmarked:

1. `git grep` for the donor symbol or the feature name in `crates/`.
2. Read the file the finish-line gap names, when it exists.
3. Read the donor file live (raw source or GitHub contents API), pin the path.
4. Keep the row only if a consumer on a finish-line gate exists.
5. Write down why a rejected candidate stays rejected, so it is not mined twice.

## Already landed (do not board again)

| Candidate the old docs call missing | What is in the tree | Evidence |
|---|---|---|
| Bevy schedule ambiguity detection | `AmbiguityLevel`, `ambiguities()`, `validate()` | `crates/ecs/src/schedule.rs:85` has the PORT note and the three-level knob |
| FSR1 EASU + RCAS upscale | Spatial upscaler beside the frozen bilinear path | `docs/plan/steals.md` row landed `0c672df7` |
| ggrs rollback history bound | 128-slot circular input history | `docs/plan/steals.md` row landed `e449553e` |
| Machine look score (SSIM / FLIP / deltaE) | Integer SSIM and FLIP in the gate and in the engine | `tools/iqa-gate.mjs`, `crates/playtest/src/look/metric.rs` |
| meshoptimizer / gltfpack cooking | Meshlet build and mesh cook | `crates/assets/src/mesh_cook.rs`, `crates/assets/src/meshlet.rs` |
| `tune.sweep` verb | Sweep pack | `crates/endpoint/src/packs/sweep.rs` |
| Text rendering for the HUD | Text lane in render and UI | `crates/render/src/text.rs`, `crates/ui/src/text.rs` |
| `linkme` distributed registration | Rejected by design | `crates/anim/src/layers.rs:268`: registration stays explicit name-keyed; std-only |
| Godot `SpringArm3D` camera boom | Donor path is gone on Godot 4.x (contents API 404 on 2026-10-20) | no row; re-open only with a live path |

Reading the finish-line gap list without this check would have produced at least
four duplicate rows. That is the real speed-up: an hour of grepping beats a week
of porting.

## The six open rows

| ID | Area | What it takes | Donor (read) | Size |
|---|---|---|---|---|
| GS43 | render | contact shadows, 16-step depth march over 0.3 m | `bevyengine/bevy` `contact_shadows.rs` (2026-10-20) | S |
| GS44 | render | 64-bin auto exposure, 0.18 aim, plus or minus 4 stops | `bevyengine/bevy` `auto_exposure/` (2026-10-20) | S |
| GS45 | render | pass report: ran, culled, failed, with a reason | `uinosoft/zenfg`, `Themaister/Granite` (2026-10-11) | M |
| GS46 | door | `world.query` with BRP `with`/`without`/`option`/`has`/`strict` | `bevyengine/bevy` `bevy_remote/src/lib.rs` (2026-10-20) | S |
| GS47 | proof | one digest on Windows, Linux and wasm; a seed bump fails | `lazyxeon/AstraWeave` replay workflow (2026-10-20) | M |
| GS48 | ai | decision ring plus `ai.decisions show|diff` | `lazyxeon/AstraWeave` `aw_decision` (2026-10-20) | S |

Three S rows are look or the door, which is where the finish line is thinnest:
Q1 is 0 of 10 and the browser still draws more than it plays. GS47 closes the
"one hash on every PC" claim, which is the sentence the whole engine rests on.

## Why not twenty

The two earlier waves produced 33 GS rows and a long tail of half-wired modules.
The lesson is in the same research file that produced them: "our worst habit is
modules that land and never reach the screen." A wave is not big because it has
many rows; it is big when each row changes a number a stranger can run. Six rows,
each with an Accept, beat twenty with a paragraph.

## What no GitHub engine has (keep building)

Checked 2026-10-20 against the engines in `docs/research/steals-2026-10-20/`:

- a typed door that refuses by default, with undo and a receipt per edit;
- a counterfactual fork of the running match before an edit applies;
- replay from the tape with zero model calls;
- one tick hash on every target;
- provider conformance across mock, local and remote with the same task result.

Bevy ships `bevy_remote` (JSON-RPC world read and write), Solers ships an agent
inside Godot, AstraWeave ships a deterministic ECS with a 19-case cross-OS replay
matrix. None ships the five together. That is the 2040 bet: the chassis is
commodity, the contract is not.

## Re-open signals

- A donor path 404 today can return: check the default branch again before
  rejecting a second time.
- `contact_shadow` and `auto_exposure` remain absent only until GS43 and GS44
  land; after that this file's table is history, not a plan.
- If a future donor ships typed undo plus fork plus replay-without-model in one
  engine, re-run this audit before writing another row.
