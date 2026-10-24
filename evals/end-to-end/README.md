# end-to-end ladder — six fixed prompts through the real door

Board row: DOOR7 (`docs/SPRINT.md`). Donor read: `SummerEngine/summer@88d0a5b6`
`evals/end-to-end/README.md` (MIT). The point stays the donor's: a ladder that
measures the model lane, not our mood. Six prompts never change, every rung
drives a real door verb, and a rung passes only when the door answers.

Code: `crates/endpoint/src/e2e_ladder.rs` (rungs, loop driver, projector, tape)
and `crates/endpoint/src/bin/e2040-ladder.rs` (prints one report and stops).
Comparison tool: `tools/agent-ladder.mjs`.

## The six rungs

| Rung | Prompt the loop reads | Door verbs | Gate (all must hold) |
|---|---|---|---|
| E0 `door_alive` | read the door status and report the advertised verb count | `door.status` | `session == 7`, `tools > 0` |
| E1 `player_walks` | reset the bout, push the player forward, read where they ended up | `sim.reset`, `sim.step` | `fighters.0.pos.0` moves |
| E2 `two_fighters_trade` | press both fighters to attack and read the damage they take | `sim.step` | `events.*.kind == hit`, both `hp < 1000` |
| E3 `parry_lands` | hold the guard into the incoming attack and read the parry | `sim.step` | `events.*.kind == parry`, `fighters.1.hp == 1000` |
| E4 `camp_work_builds` | build the camp brief and read the landed placement count | `level.build` | `verdict == pass`, `revision > 0` |
| E5 `slice_runs` | run the six hundred tick slice and read its digest | `sim.step` | `tick == 600`, `digest > 0` |

Gates are all-or-nothing. One missed check fails the rung, and the row prints
which check missed. A rung whose verb refuses prints `verb:code` from the door's
own typed refusal (for example `sim.step:bad_ticks`), so a red rung names what to
fix instead of asking the reader to guess.

The model half is `engine_provider::agent_loop`'s scripted planner — this repo's
mock provider. It is offline, deterministic, no sampling, no network, no key.
`MockAdapter` in `crates/provider/src/mock.rs` scripts reply outcomes — succeed,
timeout, quota, privacy downgrade — and carries no tool-call channel, so the
scripted planner is what "the mock provider" means here. The door half is real:
every call goes through `PackRegistry::invoke`, the locked, screened, audited
path a `tools/call` takes. So a green ladder proves the door serves these six
jobs; swapping a live planner in is the next step, not this one.

## What one run prints

Per rung: gate, door calls, cost in estimated tokens (the loop's own estimate,
one per four bytes), a transcript digest, the refusing verb, the missed check.
Wall clock in milliseconds prints per rung in the operator table only.

```
rung name                 gate calls tokens transcript       refused verb:code  missed check
E0  door_alive           pass    1     76 d754e12fc924374a   -                  -
E1  player_walks         pass    2    664 abb4a11c34fa86db   -                  -
E2  two_fighters_trade   pass    1    358 54a59f59addebef0   -                  -
E3  parry_lands          pass    1    518 1febe92f8d6d6f3a   -                  -
E4  camp_work_builds     pass    1    410 8b32a202ec3fbe0b   -                  -
E5  slice_runs           pass    1    270 d823548459a4076d   -                  -

6 of 6 rungs pass, 0 failed; 7 door calls, 2296 est tokens
wall ms per rung: E0=2 E1=0 E2=0 E3=0 E4=4 E5=2
tape (the compared artifact, no wall clock):
e2040-e2e-ladder-ev1	1
E0	door_alive	pass	1	76	d754e12fc924374a	-	-
E1	player_walks	pass	2	664	abb4a11c34fa86db	-	-
E2	two_fighters_trade	pass	1	358	54a59f59addebef0	-	-
E3	parry_lands	pass	1	518	1febe92f8d6d6f3a	-	-
E4	camp_work_builds	pass	1	410	8b32a202ec3fbe0b	-	-
E5	slice_runs	pass	1	270	d823548459a4076d	-	-
```

That is one real run, captured on 2026-10-24: six rungs green, seven door calls,
2296 estimated tokens. The tape rows are tab-separated; the table above them is
spaced for reading. A second run in a second process printed the same six rows,
which is the check `tools/agent-ladder.mjs` automates.

## Run it

The tool never builds, the same way `tools/bins.mjs` does. A seat that needs it
runs the printed line first.

```
cargo build -p engine-endpoint --features mcp --bin e2040-ladder
target/debug/e2040-ladder.exe            # the table above, plus wall clock
target/debug/e2040-ladder.exe --tape     # only the compared artifact
node tools/agent-ladder.mjs              # run twice, check the two tapes agree
node tools/agent-ladder.mjs --once       # one run, no repeat check
node --test tools/agent-ladder.test.mjs  # the comparison logic, no build needed
```

`node tools/agent-ladder.mjs` is the accept check: the same tape twice gives the
same table. Exit 0 means every rung passed and both tapes matched field for
field; exit 1 names the rung and the column that moved; exit 2 means the binary
is missing (it prints the build line) or the ladder could not run.

## Why the tape repeats

- Eight tab-separated columns per rung: `id name gate calls tokens transcript
  refused check`. No wall clock, so timing noise cannot reach the comparison.
- The `transcript` column is a digest of the projected observations, so a rung
  that reads the same numbers twice digests the same.
- A leaf a fresh document mints on every build — a scene node id — reaches the
  planner as `<minted>`. `level.build` gives `placements[].id` a new UUID per
  build; masking it keeps E4's tape stable while the gate still reads the real
  value.
- Every sim rung uses the golden seed `2040` at `half_extent_m: 20.0`, so the
  rungs walk the same fixed tick path the golden hash guards.

## Measurements the tapes encode

These numbers came from running the door, not from reading the design docs.

- **E2 needs a 14 tick gap.** An attack resolves four ticks after the press, and
  a fighter hit on tick 4 is in hit react and recovery until tick 14. Pressing
  both at tick 0 lands one blow and the tape reads one damaged fighter. So E2
  presses slot 0 at tick 0 and slot 1 at tick 14, over 40 ticks: hits on ticks 4
  and 18, both fighters at 880 hp.
- **E3 holds the guard across the parry window.** The window is four ticks, so
  slot 1 holds `buttons: 2` on ticks 1 through 5 while slot 0 presses at tick 0.
- **Input rows are sparse.** A slot with no row on a tick stays idle, so a rung
  carries only its press rows. That also keeps args under the loop's 2048
  character tool-input cap: E1's twelve-row walk fits, a 600-tick full tape
  would not.

## Where the code lives, and why not the row's path

The row named `crates/playtest/src/eval.rs`. The harness is in `crates/endpoint`
instead: a rung must call a door verb, and `engine_playtest` cannot depend on
`engine_endpoint` (endpoint already depends on playtest, so that edge would
cycle). The eval crate keeps its own scorers; this ladder is the door's.

## What is proven, and what is not

- The six rungs run on the live door verbs: 13 unit tests in
  `crates/endpoint/src/e2e_ladder.rs`, including the two-run tape equality and
  the "a refusal names its verb" case. Green on the runner, 2026-10-24.
- `tools/agent-ladder.mjs` is unit-proven (`node --test
  tools/agent-ladder.test.mjs`) and has parsed and compared two real captured
  ladder outputs, which agreed field for field.
- It has not driven a built `e2040-ladder.exe` yet: no seat has built that
  binary in this tree, so `node tools/agent-ladder.mjs` currently exits 2 with
  the build line. Building it is one cargo line, not a code gap.
- Nothing here looks at a pixel. The ladder reads typed door answers; visual
  proof stays with the shot and frame gates.
