# AIN7 live jump receipt (2026-10-20)

Owner order, in chat: **"YES ALL"** to wiring a jump into the live arena so
the air table is visible outside the sim leg. This receipt names the input,
the numbers, the tape, the live capture and the checks. The root golden does
not move.

## Input

| site | change |
|---|---|
| `crates/input/src/fighter.rs` | `JUMP = 0x20`, part of `ALL_BUTTONS`; combat and feel masks still use the three `KNOWN_BUTTONS` |
| `crates/game/src/windowed.rs` | keyboard `E` sets `JUMP` beside the rebindable action rows |
| `crates/arena/src/lib.rs` | a fresh press on the floor sets the height-channel rise; the first press arms the channel mid-match |
| `crates/game/src/view_truth.rs` | a takeoff/landing crossing is not a pose pop; a live jump is not idle float for 60 ticks, after which a hover flags again |

The wire range is now `0..=55` (`ALL_BUTTONS`), and the agent seat accepts the
jump bit (`docs/DEMO.md`). The pad has no jump row yet; the keyboard is the
live jump.

## The numbers

| channel | number | unit | live path |
|---|---|---|---|
| take-off rise | 5.0 | m/s (83 mm/tick) | `JUMP_RISE_MM_TICK` |
| apex speed | 2.0 | m/s (33 mm/tick) | `AIR_APEX_MM_TICK` |
| apex gravity | 0.5 | scale while the jump is held | `FIGHTER_GRAVITY / 2` |
| hold cut | 0.5 | rise kept on release | `rise /= 2` |
| camera lead | 0.5 | s | already live (AIN6) |

The horizontal air ramp (0.30 gain, 0.20 turn brake) and the 120 ms coyote stay
on the sim controller: the live presentation leg is a kinematic locomotion
machine with no air momentum owner, and the flat arena has no ledge to walk
off. The live jump uses the vertical half of the table; the sim leg keeps the
whole table.

## First differing tick

`live_jump_first_hash_diff_is_the_press_tick` runs the same 60-tick
two-fighter tape with and without a `JUMP` press at tick 10 (held through tick
17) and compares the per-tick checkpoints: the first differing checkpoint is
tick 10, the press tick. Before tick 10 the match is the old floor-plane match
byte for byte; the channel arms on the press, so the hash folds the rise only
from then on. The same tape x2 repeats its peak and its hashes.

## Live proof

Tape: `target/clips/ain7-live/jump.tape` (human slot 0, `JUMP` held ticks
10..17). Run:

```
target\release\arena.exe --scene arena --fighters 2 --frames 120 --exit --play ^
  --tape target\clips\ain7-live\jump.tape --capture-dir target\clips\ain7-live\live2 ^
  --capture-every 5 --rows AIN7 --sha abcdef1 --fps 30 --mute
```

`--capture-every 5` keeps the wall-clock tick rule at 120 sim ticks; a capture
every 2 frames starves the sim (7 ticks) on this PC.

- captures: `target/clips/ain7-live/live2/`; `frame_000003.png` (tick 16) is
  the apex, head above the horizon, clear of the ground.
- `arena sim: ticks=120 hash=4de244ba51859f8b`; the same tape ran to the same
  hash in four runs (`pace`, `run3`, `live`, `live2`) and in the `--fixed-dt`
  run.
- `arena tape: inputs=120 applied=120 unreached=0 missed=0`.
- `arena view: frames=120 sim_ticks=0.8..119.2 ... sunk_cm_max=0.0
  idle_float_cm_max=0.0 pops=26 pop_acts=run:26 pop_cm_max=38.5
  eye_in_body=0 eye_below_ground=0 ...`; no `arena WARN:` line. The jump adds
  no idle float and no pop act; the `run:*` pops are the open AM24 live
  residual, not the jump.

## Checks

- `cargo test -p engine-arena --lib live_jump` -> 3 passed.
- `cargo test -p engine-input --lib` -> 52 passed.
- `cargo test -p engine-game --lib --features app view_truth` -> 22 passed.
- `cargo test -p engine-arena --lib` -> 230 passed, 0 failed, 5 ignored
  (GitHub runner, `wip-mvgg3hcq7uv`).
- The spear rope swing receipt is unchanged (`rope_swing_holds_the_arc...`).
- Root `target/release/engine2040.exe` x2 -> `final_hash=d57b5c8ae3096614`
  both times; the root does not link arena/game.
