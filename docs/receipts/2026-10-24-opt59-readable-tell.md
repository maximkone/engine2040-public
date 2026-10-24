# OPT59 readable tell: startup 15 -> 30 ticks (2026-10-24)

The player could not read a tell. Both shipped timing sets gave a light attack
15 ticks of wind-up (250 ms at 60 Hz), and the one measured readability study
(theseus 900621, n=30 players: median read 800 ms, IQR 525-900, heuristic floor
500 ms) says that window is too short. OPT59 raises the shipped tell to the
30-tick floor and changes nothing else about the attack shape.

## What changed

| file | before | after |
|---|---|---|
| `crates/combat/src/design.rs` (`MIN_STARTUP_TICKS`, `SHIPPED`) | startup 15, recovery 8, floor 12 | startup 30 (named floor const), recovery 6, `validate` refuses anything under 30 |
| `crates/combat/src/tables.rs` (`v2_rows`) | startup 15, recovery 8 | startup `MIN_STARTUP_TICKS` (30), recovery 6 |
| `crates/ai/src/tell.rs` (new owner; `depth.rs` re-exports) | tiers 1-4 tell 15, designer fence `15..=60` | tell 30, fence `30..=60` |
| `content/design/enemies.rows` | `tell_ticks=15` on tier_1..tier_4 | `tell_ticks=30` |
| `content/design/combat.rows` | v2 line startup 15, recovery 8 | v2 line startup 30, recovery 6 |
| `crates/game/src/practice.rs` (`V2_LINE`) | the same v2 row, mirrored for the reload test | startup 30, recovery 6 |
| `crates/proof/src/arena_matrix.rs` | digest pin `605f6503a39082f4`, slot 1 re-presses block every 8 ticks | pin `2b30084d1530cc78`, slot 1 holds block 15 of 16 ticks, slot 0 guards from phase 60, and the digest test prints the tape tally |
| `crates/playtest/src/ttk/rome-ttk.csv` | derived from startup 15 / recovery 8 | derived from startup 30 / recovery 6 |
| `crates/arena/src/lib.rs` (test) | pins `12..=18` and hit tick 15 | pins `>= 30` and hit tick 30 |
| `tests/proof_cli.rs` | the golden assert had no message | the assert names the new hash when it moves |

Recovery goes shorter (8 -> 6), not longer: the swing cycle is 36 ticks instead
of 38 with an untouched recovery, so the longer tell does not only make every
exchange slower. The chain window still lands inside the swing
(`chain_allowed` opens at startup + grace and closes at startup + recovery).

Not changed: `TABLES_V1` (startup 4) stays the replay-only reference row, and a
designer swap row keeps its `1..=60` sanity fence. The readability floor is a
law on shipped rows, not on every custom row.

## Golden: GOLDEN_600 did not move

`cargo test -p engine2040 --release --test proof_cli default_and_max_seed`
(run [40062028191](https://github.com/maximkone/engine2040-public/actions/runs/40062028191)):
PASS, `final_hash` still `d57b5c8ae3096614`. The fixture is the `engine-proof`
2D scene and never calls a combat table, so there is nothing to re-pin there.

## The arena digest moved

The rules-v2 kit match (`crates/proof/src/arena_matrix.rs`,
`Fixture::STANDARD`, 8 fighters, 1200 ticks) folds the combat timing, so it
moved. Both the live path and the spine path fold the same new value.

| | digest |
|---|---|
| before (HEAD `97458fb80`) | `605f6503a39082f4` |
| after | `2b30084d1530cc78` |

Runs: before `cargo test -p engine-proof --features arena --lib
arena_digest_is_pinned -- --nocapture` (run
[40062208906](https://github.com/maximkone/engine2040-public/actions/runs/40062208906));
after the same line with `arena,spine` (run
[40064476529](https://github.com/maximkone/engine2040-public/actions/runs/40064476529)),
9 passed, live and spine paths on the same value.

## Bout outcome, before and after

The same 1200-tick match, tallied by the tape (hits, blocks, parries, dodges,
knockdowns, breaks). 1200 ticks is 20 s of fight.

| | digest | hits | blocks | parries | dodges | knockdowns | breaks |
|---|---|---|---|---|---|---|---|
| before, startup 15 / recovery 8 | `605f6503a39082f4` | 60 | 2 | 4 | 0 | 6 | 11 |
| startup 30 / recovery 8 (rejected) | `0f15bd2ed9cb94c2` | 44 | 3 | 6 | 1 | 2 | 11 |
| landing: startup 30 / recovery 6 | `2b30084d1530cc78` | 52 | 3 | 1 | 0 | 4 | 11 |

The 30/8 experiment lost a third of the blows (60 -> 44) and two thirds of the
knockdowns (6 -> 2): the fight still resolved, but it was the "only gets slower"
outcome the row warned about. Shortening recovery to 6 puts the pace back
(52 hits, 4 knockdowns) while the tell stays readable.

The tape's guard script moved with the timing. With the impact 15 ticks later,
the old script (slot 1 re-pressing block every 8 ticks) stopped meeting any blow
on a held block: the same 1200 ticks showed `blocks=0, parries=2`, and the
coverage law in `arena_tape_exercises_block_parry_knockdown_and_the_crate` went
red. The script now holds block for 15 of every 16 ticks on slot 1 and guards
from phase 60 (was 84) on slot 0, so the tape still shows hits, a held-block
soak, a parry, knockdowns and crate breaks. That is a script change inside the
fixture, not in the rules; the pin above is the number it produces.

Every run prints the tally beside the digest, so a future retune that stalls the
fight fails on `hits > 0` / `knockdowns > 0` as well as on the hash.

## Proof lines

- `cargo test -p engine-combat --lib design` (run 40061970617): 7 passed,
  including `every_attack_opens_a_readable_window`, which holds the floor on
  `design::SHIPPED` and `TABLES_V2`, checks that grace, parry and chain land
  inside the wind-up, and proves `validate` refuses 29 ticks.
- `cargo test -p engine-combat --test it design` (run 40062045894): 3 passed.
- `cargo test -p engine-ai --lib depth` (run 40062133240): 57 passed.
- `cargo test -p engine-game --lib` (run 40062397361): 625 passed, 6 ignored.
- `cargo test -p engine-net --lib` (run 40062169271): 482 passed, 5 ignored.
- `cargo test -p engine-proof --features arena,spine --lib arena` (run
  40064476529): 9 passed, the live and spine digests on `2b30084d1530cc78`.
- `cargo test -p engine-playtest --lib`: the derived time-to-kill table
  `crates/playtest/src/ttk/rome-ttk.csv` was regenerated from the new rows
  (startup 30, recovery 6) in the same landing.
