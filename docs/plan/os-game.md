# "You're the OS" — plan to a playable one-screen puzzle game

A small deterministic game for engine2040: four CPU slots, 64 RAM pages plus
swap, one I/O queue, up to 48 processes and twelve stages. The player keeps
processes running, swaps pages and survives starvation; ten kills reboot the
machine. The headless core lives in `crates/ossim` (package `engine-ossim`).

Owner lanes: the sim is engine work; art and copy are the owner's. Prefix:
`OS`. Base commit: the commit that lands `crates/ossim` (fill in on landing).

Idea source: <https://github.com/plbrault/youre-the-os> (GPL-3.0). **Ideas
only.** No source file, comment, string, table, art or stage text from that
project is copied. Our code is original and stays Apache-2.0.

## 1. Where it stands

| Capability | Status | Evidence |
|---|---|---|
| Integer 60 Hz tick, no floats, no clock | implemented, unit-tested | `crates/ossim/src/sim.rs`, `production_source_has_no_float_or_wall_clock` |
| 4 CPUs, 48 process slots, 7x6 grid layout | implemented, unit-tested | `model.rs`, `grid_layout_wraps_after_42_cells` |
| 64 RAM pages plus stage swap slots | implemented, unit-tested | `ram_and_swap_accounting_balances`, `swap_in_needs_ram_room` |
| One I/O queue with 60-300 tick ready times | implemented, unit-tested | `io_ready_delivers_after_the_timer` |
| Starvation (600 ticks, 360 priority), happiness reset at 300 | implemented, unit-tested | `starvation_climbs_every_600_ticks`, `happiness_resets_after_300_running_ticks` |
| Ten kills reboot; min-process defeat; win tick | implemented, unit-tested | `ten_kills_reboot_the_machine`, `every_stage_has_a_loss_path` |
| 12 const stages, one six-minute stage | implemented, unit-tested | `stages.rs`, `stage_table_has_twelve_rows_and_a_six_minute_stage` |
| Deterministic state hash | implemented, unit-tested | `same_tape_twice_gives_the_same_hash` |
| Window, drawing, input mapping, sound | not built | none |
| Web page and replay viewer | not built | none |

The sim is engine-neutral: it depends on `engine-core` only and names no Rome
type. A window shell can read `Sim::state()`, draw it, and send one `Action`
per tick.

## 2. Finish line

Tier A (4 weeks): one window opens, a player finishes all 12 stages with mouse
or keyboard, score and stage select save locally. Tier B (5 weeks): sound
cues, a tutorial page, a replay tape that reproduces a run byte for byte on
another PC. Tier C (6 weeks): the same sim plays in the browser through the
existing wasm build, and a recorded run replays there with the same hash.

Measurable:
- `cargo test -p engine-ossim --lib` green with 20+ tests.
- Same seed and action tape produce the same hash on native and wasm.
- A full stage-12 run (21,600 ticks) fits one frame budget check: 0.2 ms per
  tick for the sim on the reference PC.
- No float, clock or global state in `crates/ossim/src/` (source test).

## 3. Target design

Data flow: `Sim::tick(Option<Action>)` applies one player action, then runs
timers (I/O and swap completions), process starvation and work, the stage
spawn timeline, one ambient roll and the stage rules. A UI reads
`Sim::state()` and `Sim::events()` after each tick and sends nothing back
except actions. Presentation never feeds state.

Ownership: `engine-ossim` owns the model, stages and hash. The window shell
owns input repeat, layout, fonts and sound. `engine-core` owns the tick rate,
the counter RNG and the FNV fold; the crate has no other dependency.

Determinism: integer fields only. Seeds expand through
`engine_core::CounterRng`, so seeds 2k and 2k+1 differ. Every random choice
draws from a stored counter, and the counter is in the hash. Same seed plus
the same action tape gives the same hash at every checkpoint.

API sketch:

```
Sim::new(seed, STAGES[i]) -> Sim
sim.tick(Some(Action::Assign { cpu, process })) -> Result<(), Refusal>
sim.state() -> State      sim.hash() -> u64
sim.score() -> i32        sim.outcome() -> Option<Outcome>
sim.events() -> &[Event]  sim.fault(process) -> Result<(), Refusal>
```

Budgets: one `State` clone is about 5 KB and is not per-frame in the sim; the
window takes one snapshot per drawn frame. The tick itself allocates only when
it pushes an event, and events are bounded by live processes.

## 4. Hard problems

- **Difficulty without floating balance.** Stages are const rows; the
  min-process curve and spawn count are the only knobs. Technique: validate
  every stage with two scripted runs (fair autopilot wins, idle loses) inside
  the test suite. This keeps every stage provable, not tuned by feel.
- **Visible fairness.** A player must see why a process starves. Technique:
  `starvation` 0-6 and `wait_ticks` are in the snapshot; the UI draws six pips
  per process, not a percentage.
- **Swap and I/O both block.** Two timers could hide each other. Technique:
  one `Pending` field for swap, one queue entry for I/O; a process has at most
  one in flight and `Refusal::Busy` says so.
- **Replay at different frame rates.** Technique: the tape stores tick and
  action only; the renderer may drop frames, never ticks.
- **Stage text and art licensing.** Technique: write our own names and copy;
  the GPL project supplies the idea and nothing else.

## 5. Donors

The game idea is `plbrault/youre-the-os` (GPL-3.0), used as an idea only: a
screen of CPUs, processes, RAM, swap and I/O. No code, text, stage table or
asset is ported or adapted, so the GPL does not reach this repository. All
other owners (`CounterRng`, `Fnv64`, tick constants) are local `engine-core`
owners. Porting policy for local code is AGENTS rule 4.

## 6. Slim

Delete or never build:
- a process scheduler AI; the player is the scheduler;
- network play, accounts, leaderboards, telemetry;
- a second simulation model for the browser: one sim, two shells;
- per-process animation beyond state color and a progress bar;
- procedural audio; use a small set of pre-made cues or none;
- a level editor; stages stay const rows until the game ships;
- save games beyond stage unlock and best score;
- any Rome arena type in the sim crate.

Merge or reuse:
- the window shell reuses `engine-app` tick pacing and `engine-render` only if
  a 2D quad path already exists; otherwise one small `engine-ui` screen.
- replay uses the existing recorded-action and hash idea, not a new format.

## 7. Board tasks

Not on the SPRINT board yet; these rows are the proposal.

- [ ] OS1 headless sim core: 4 CPUs, 48 processes, RAM/swap, one I/O queue,
  starvation, score, 12 const stages, integer hash. Size M. Owns
  `crates/ossim/`. Accept: `cargo test -p engine-ossim --lib` green, 20+
  tests, same tape twice gives one hash. needs: none.
- [ ] OS2 stage balance pass: every stage has a scripted win and a scripted
  loss, stage 12 is six minutes, no stage needs more than four CPUs. Size S.
  Owns `crates/ossim/src/stages.rs`. Accept: the two stage-path tests cover
  all 12 rows and print the win tick. needs: OS1.
- [ ] OS3 one window: draw the four CPU slots, the 7x6 process grid, RAM and
  swap bars, I/O queue and score; map click and keys to the five actions.
  Size M. Owns `crates/ossim-ui/` (new game-side crate). Accept: a recorded
  capture of stage 0 and stage 3 at 1280x720 with legible starvation pips.
  needs: OS1.
- [ ] OS4 tutorial and score screen: one page explains assign, swap, I/O and
  starvation; stage select stores unlocks and best score. Size M. Owns
  `crates/ossim-ui/src/tutorial.rs`. Accept: a new player finishes stage 0
  without reading the plan, and the best score survives a restart.
  needs: OS3.
- [ ] OS5 browser and replay: the same sim in the wasm build, a tape of tick
  and action, and a hash check on load. Size M. Owns `web/ossim/`. Accept:
  one native tape replays in the page with the same 16-char hash.
  needs: OS1, OS3.

Row changes: none; OS ids are new and go on the board only when the owner
accepts this plan.

## 8. Quality gates

- `cargo test -p engine-ossim --lib`: 27 tests today; keep every stage path
  and the source scan.
- `rustfmt --edition 2021 --check crates/ossim/src/*.rs`.
- `node tools/rust-quality.mjs` stays PASS: no long function, no duplicate
  block, no file over the line cap.
- Replay gate: seed and tape twice, compare hash at 500-tick checkpoints.
- Window gate: one capture per stage group on the reference PC; no GPU claim
  before a capture exists.
- Web gate: wasm tape hash equals the native hash printed in the same run.

## 9. Session estimate

- Tier A (OS1, OS2, OS3): 6-8 agent sessions, about 4 weeks with owner review.
- Tier B (OS4 plus sound and replay): 2-3 sessions, week 5.
- Tier C (OS5, browser shell and capture): 2-3 sessions, week 6.
- The sim core is already built; the remaining cost is shell, copy, captures
  and balance, not simulation.
