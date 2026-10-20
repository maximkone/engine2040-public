# AI-native steals: the seat, the warn, and the next five

Owner 2026-10-20: "AI is not AI native... no auto fix or warn". Eight
research agents went out: five over open-source engine plugins and dev-loop
tools, three over arXiv. This file keeps only the mechanisms that fit this
engine, the row each gets, and what not to copy.

Two pieces landed the same day:

- **AIN1, the live seat.** `arena.exe --agent-seat <port>` binds loopback,
  takes one client, and lets it author slot 0's `FighterInput` per fixed
  tick. Every tick returns `tick <n> <hash>`; the view line comes every 60.
  Inputs pass `FighterInput::validate`, so a tape, a pad and an agent fold
  the same hash. Receipt: `arena seat: ... held=0 late=0 invalid=0`.
- **Self-warn.** `ViewLaws::warnings()` prints one `arena WARN: <metric>
  value=<v> floor=<f>` per broken product floor, and a test pins those
  floors to `tools/product-bar.json`. A clean run is silent.

## Ranked next moves

### 1. The seat becomes an MCP surface (AIN2)

Sources: Orak, arXiv:2506.03610 (one plug-and-play MCP toolset over many
games, recorded runs for ablations); ConduitMCP
(github.com/Advik-B/ConduitMCP: in-process bridge, local IPC, `gd_play` /
`gd_input` / frame waits); Unity MCP marshals every call to the main thread.
Mechanism: the agent never touches engine internals, only `observe / act /
step / end`, and the step is stamped to a tick. Landing: `seat.open`,
`seat.step`, `seat.close` in the door, driving the AIN1 socket. Proof: a
30-tick MCP tape equals the AIN1 client tape hash at the same tick; the
golden does not move. Grade: observed, landing inferred.

### 2. Wire the view host (AIN3)

Sources: Unreal MCP exposes read-only resources serialized on the game
thread; GameWorld, arXiv:2604.07429, counts invalid semantic actions instead
of dropping them. Here `view.laws` exists but no binary calls
`register_view_source`, so the door answers unsupported: an agent has no
eyes. Landing: register a view source in the MCP host that runs
`FighterLook::view_run` for the line (and the CPU pose for camera + bodies
when it can). Proof: a real door call returns two equal lines; a bad request
stays a typed refusal. Grade: observed (the gap is in this tree).

### 3. Warn -> bundle -> bisect (AIN4)

Sources: rr (github.com/rr-debugger/rr) keeps the failing run's trace;
C-Reduce (github.com/csmith-project/creduce) shrinks the input; delta
debugging for CPS, arXiv:2607.25695, keeps the smallest failing tape;
Revizor, arXiv:2105.06872, diffs two builds only on contract fields.
Landing: a WARN writes `target/repro/<metric>-<stamp>/` with seed, ticks, the
view line and the crash tape tail; `replay.bisect` returns the first tick
whose receipt crosses a monotone floor. Proof: a seeded fault yields one
bundle and a tape that reproduces it; a clean run is silent. Grade: observed
(rr, creduce), landing inferred.

### 4. The brain commits and predicts (AIN5)

Sources: FightingICE's opponent action table; BehaviorTree.CPP's RUNNING
leaf; Fluid HTN replans only on a watched fact; rolling-horizon plus an
opponent model, arXiv:2003.13949; FightLadder, arXiv:2406.02081, rates
exploitability rather than head-to-head wins. Landing: an `ActionLease` in
`crates/ai/src/depth.rs` (hold an intent N ticks, abort on a threat bit) and
a four-bucket foe table. Proof: the lease test; a fixed-seed bench prints
win rate and damage twice identical. Grade: observed, transfer inferred.

### 5. Movement and camera numbers (AIN6)

Sources: Celeste64 (air accel scaled by `dot(wish, facing)`, half gravity
near apex, 0.12 s coyote); SM64 (wall kick, ledge snap); Phantom Camera
(0.5 s look-ahead with accel/decel damping); Ikemen GO (juggle budget 15).
Landing: the air ramp and apex gravity in the controller, velocity lead in
the camera, one numbers table. This moves the golden hash, so it lands only
with a written receipt. Grade: observed numbers, fit inferred.

## What not to copy

- A model inside the 60 Hz tick. The seat records validated inputs; the
  model stays outside, so replay still works.
- A Python training loop in the engine (ML-Agents is the donor for cadence
  and action masks, not a runtime dependency).
- An auto-commit repair loop. Patches are proposed in a scratch tree and
  accepted only by tests plus replay (Agentless, arXiv:2407.01489).
- A warm `view` line per tick. One line per 60 ticks is enough to see, and
  keeps the seat's socket cheap.

Sources read: Orak 2506.03610, GameWorld 2604.07429, BugCraft 2503.20036,
SWE-Game 2609.33678, Revizor 2105.06872, ddmin-for-CPS 2607.25695, Agentless
2407.01489, AutoCodeRover 2404.05427, FightLadder 2406.02081, rolling-horizon
fighter AI 2003.13949, plus the repos named above.

## Landing order (owner 2026-10-20: "non stop, finishes, plan it all")

One row per proof, disjoint files, no agent commits: the main session reviews,
runs the gates and lands each row by path.

1. **AIN3, the view host** (smallest, unblocks every observation): register a
   source in the MCP host so `view.laws` serves the live `arena view:` line;
   `view.observe` serves camera and bodies where the headless CPU pose can.
   Files: `crates/endpoint/src/bin/e2040-mcp.rs`, `crates/endpoint/src/packs/view.rs`
   (plus a read-only `FighterLook::view_run`). Gate: a real door call returns
   two equal lines; the golden cannot move.
2. **AIN2, seat verbs** (same binary as 1, so one hand): `seat.open/step/close`
   launch or attach to `arena.exe --agent-seat`, push validated inputs and
   return the tick hash. Gate: 30-tick MCP tape equals the socket tape at
   tick 20; invalid actions are counted.
3. **AIN4, bundle and bisect** (independent of the door's view path): a WARN
   writes `target/repro/<metric>-<stamp>/`; `replay.bisect` finds the first
   tick a monotone floor crosses. Files: `crates/game/src/crash.rs`,
   `crates/game/src/view_truth.rs`, the bug pack. Gate: seeded fault once,
   clean run silent, golden x2.
4. **AIN5, brain lease and foe table** (crates/ai only): hold an intent N
   ticks, abort on a threat bit; four-bucket foe prediction; fixed-seed bench.
   Gate: two named tests plus a bench that repeats; golden x2.
5. **AIN6, numbers behind an opt-in channel** (physics/game/combat): air ramp,
   apex gravity, coyote, camera lead; the fight controller stays on today's
   numbers unless the channel is armed, exactly like `with_height`. Gate: an
   opt-in tape shows the numbers and the golden still reads
   `d57b5c8ae3096614`; the armed hash and its receipt are the owner's call.
   Armed 2026-10-20 (owner "ARM GOO ALL"): the sim controller uses
   `AirTuning::shaped`, the window camera uses `with_look_ahead`, and the
   receipt is `docs/receipts/2026-10-20-ain6-armed/README.md` (air tick 6 is
   the first differing tape tick; root golden unchanged x2). The juggle budget
   is built in combat: 15 consecutive clean hits, the 16th knocks down; the
   number, the first differing hit and the Retiarius reachability proof are in
   `docs/receipts/2026-10-20-ain6-juggle/README.md`.

### AIN6 numbers table (one owner: `AirTuning::shaped` and `LookAhead`)

| channel | number | unit | off path |
|---|---|---|---|
| air control gain | 0.30 | scale on the wish | same constant |
| turn brake | 0.20 | gain on a full reversal, 1.0 on the line | 1.0 |
| apex speed | 2.0 | m/s vertical | 0.0 (never) |
| apex gravity | 0.5 | scale near the apex | 1.0 |
| coyote | 120 | ms | 150 ms |
| jump hold | 6 | ticks | 0 |
| hold cut | 0.5 | rise kept on release | 1.0 |
| camera horizon | 0.5 | s | 0.0 |
| camera lead cap | 1.2 | m | 0.0 |
| accel halflife | 0.2 | s | 0.0 |
| decel halflife | 0.1 | s | 0.0 |
| juggle budget | 15 | consecutive clean hits | none (always on; hit 16 knocks down) |

Armed, `Controller::state_hash` folds `air_facing` and the hold deadline; the
unarmed controller reads none of the table, so `d57b5c8ae3096614` stays. The
arena arms the table in the sim leg (`build_controllers`) and the camera lead in
the window (`new_cam_follow`) under the owner's 2026-10-20 "ARM GOO ALL"; the
receipt is `docs/receipts/2026-10-20-ain6-armed/README.md`. On the fixed course
tape the armed step first differs on air tick 6 (the first reversed wish). The
juggle budget is built in combat (15 consecutive clean hits; hit 16 is a
knockdown) and its receipt is
`docs/receipts/2026-10-20-ain6-juggle/README.md`.

