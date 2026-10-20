# AIN6 juggle budget receipt (2026-10-20)

Owner order, in chat: **"YES ALL"** to closing the juggle as a 15-hit
stun-combo cap, with a new hash receipt. This receipt names the number, the
first tick the behavior differs, and the checks. The root golden does not move.

## The number

| channel | number | unit | where |
|---|---|---|---|
| juggle budget | 15 | consecutive clean hits | `JUGGLE_BUDGET` in `crates/combat/src/fight.rs` |

The first clean hit on an un-stunned victim sets the counter to 1. Each further
clean hit while the victim is still in hitstun (`react > 0`) adds one. Hit 16
(one past the budget) is a knockdown for `tables.knockdown_ticks` instead of
more stun, and the counter resets to 0. A block, a dodge, a parry, an expired
hitstun clock, and any knockdown also reset it.

The counter is part of `FighterState` (`juggle: u8`) and therefore of
`MatchSnapshot` and `FightState`; the combat test tape hash folds it.

## Reachability

With the default 120 damage and 1000 hp a clean combo kills at hit 9, so the
budget is a cap for low-damage rows. The Retiarius class does 60 damage
(`classes::tables(Retiarius, TABLES_V2)`), survives hit 15 at 100 hp, and is
knocked down by hit 16 at 40 hp; `juggle_budget_fires_before_retiarius_kills`
proves that exact bout.

## First differing tick

Hits 1..15 produce the same state and events as before (the counter is not read
by any other rule). The first difference is the **16th consecutive clean hit**:
the old rule set `react` and emitted only `Hit`; the new rule emits `Hit` plus
`Knockdown`, sets a 30-tick knockdown, and zeros the counter. The arena match
hash folds hp/stamina and the tick events, so a fixed Retiarius bout's hash
first differs on the tick of that 16th hit. The root fixture never takes 16
consecutive hits, so `d57b5c8ae3096614` stays.

## Checks

- `cargo test -p engine-combat --lib juggle_budget` -> 3 passed
  (`..._sixteenth_stun_hit`, `..._resets_when_hitstun_expires`,
  `..._fires_before_retiarius_kills`).
- `cargo test -p engine-combat --lib` -> 310 passed, 2 ignored, 0 failed
  (GitHub runner, `wip-mvgg3hd959k`).
- `cargo test -p engine-arena --lib` -> 230 passed, 0 failed, 5 ignored
  (GitHub runner, `wip-mvgg3hcq7uv`; the three `live_jump_*` tests included).
- Root `target/release/engine2040.exe` x2 -> `final_hash=d57b5c8ae3096614`
  both times; the root does not link combat/arena.
- `node tools/rust-quality.mjs --crate combat` -> PASS 99 (A), 0 ratchet
  regressions.
