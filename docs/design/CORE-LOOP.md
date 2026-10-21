# CORE-LOOP — the bout loop, the first minute, the ladder beats

One currency exists: glory (`crates/game/src/ladder.rs`).
Gold, crowd favor, economy-full: not in this slice (`docs/plan/gameplay.md:29`).

## Core loop

One line. Start state > player action > reward > changed state:

two fighters 8 m apart with full hp, stamina and poise > close distance, attack, defend, dodge > damage, stagger, stamina spent, parry punish > hp, stamina, poise, round clock and round wins changed

Session loop beside it: round (600 ticks, best of 5, first to 3) > ladder rung > glory rank > next opponent tier.
Round rule (`docs/plan/rome-spec.json`): 600 ticks at 60 Hz; higher total hp wins on timeout; an exact tie replays the round.
Arena numbers: half extent 10 m; spawns -4 m and +4 m; march 1.7 m/s; sword 120 permille damage, 15-tick startup, 8-tick recovery, blade reach 1500 mm; hp 100 display = 1000 permille inside.
Sword numbers come from `content/design/combat.rows:9` (the v2 row) and `content/design/weapons.rows:24` (gladius); the reach is `crates/arena/src/match_wire.rs:33`. `content/arena/rome-spec.json` keeps the forge parity numbers (25 damage, 0.5 s cooldown, 200 cm range) and stays as it is: its drift test pins them.

## First minute in order

What the player learns first, in order. Times at 60 Hz.

1. Launch to first bout within 45 s, capped at 60 s. No menu maze. The bar is first bout, not first win.
2. Tick 0: round 1 of best of 5. Two fighters 8 m apart. Full hp, stamina, poise. Round clock starts.
3. Ticks 0-229: close distance. 6.5 m to cover at 1.7 m/s is about 3.8 s. The player learns to move first.
4. By tick 240: first exchange. Light attack meets block. The player learns one attack and its one answer.
5. Ticks 240-600: dodge and parry appear. A dodge avoids; a fresh block press parries. Spam tires through stamina.
6. Round end: higher hp wins on timeout; all foes at 0 hp wins early; a tie replays. Best of 5, first to 3.
7. Bout end: rung climbs on a win and retreats on a loss. Glory pays on a win and drains 50 on a loss. The next tier waits.

## Beat chart per ladder rung

Rungs run `0..=7` (`MAX_RUNG` 7 in `crates/game/src/ladder.rs`).
Tier never falls; the boss is last. The rows file wins:
`content/design/ladder.rows:25-32` seats tier 1,1,2,2,2,3,3,3 and the boss
flag on rung 7. The code still seats tier 4 on rung 7
(`crates/game/src/ladder.rs:38`, `BOSS_TIER` 4 at `:45`); that seat is the open gap.
New things per rung are the beat map, at most 3 per rung
(`MAX_NEW_ELEMENTS_PER_RUNG`, Rogers L4). Minutes are the per-rung bout budget.
The cells below are the rows file, never a second copy.

| rung | tier | new elements | new count | minutes | reward, glory counts |
|---|---|---|---|---|---|
| 0 | T1 grunt | light, parry | 2 | 2 | 100, rank Tiro |
| 1 | T1 grunt | heavy | 1 | 2 | 125, rank Tiro |
| 2 | T2 veteran | dodge answer, stamina | 2 | 3 | 150, rank Gladiator |
| 3 | T2 veteran | riposte | 1 | 3 | 175, rank Gladiator |
| 4 | T2 veteran | stamina pressure, wall band | 2 | 4 | 200, rank Gladiator |
| 5 | T3 elite | elite pressure | 1 | 4 | 225, rank Primus |
| 6 | T3 elite | gate mouth, ring | 2 | 5 | 250, rank Primus |
| 7 | T3 elite, boss flag | final bout | 1 | 6 | 275 plus 500 per loop, rank Champion |

Beat map and rows: `content/design/ladder.rows:19-23` and `:25-32`. Tier
names: `content/design/enemies.rows:10-13`. Rank floors and names:
`content/design/loop.rows:9` and `crates/game/src/ladder.rs:29-31`.

Glory math (`crates/game/src/ladder.rs:17-21`, `content/design/loop.rows:9`): win pays 100 plus 25 per rung held; a top-rung win loops to rung 0 and adds 500.
Rank floors: 0, 300, 900, 1800, 3600 to Tiro, Gladiator, Primus, Champion, Legend (`content/design/loop.rows:9`, `crates/game/src/ladder.rs:29-31`).
Minutes: `RUNG_MINUTES` is the per-rung bout budget, the `minutes` cell above,
2, 2, 3, 3, 4, 4, 5, 6 and rising with the rung. One bout is at most five 10 s
rounds plus resets, so a rung budget can never be read as one round. Rung 0
starts within 60 s of launch (`FIRST_BOUT_START_LIMIT_SECS`), and
`FIRST_BOUT_START_SECS` 45 is the target inside that cap.
Second door: `content/design/ladder.rows` is the designer beat chart and
`tools/beat-lint.mjs` gates it (at most 3 new per rung, tier never falls, boss
last, reward and rank equal to `crates/game/src/ladder.rs`). The lint also
holds the rows file to the compiled chart: the new-element total
(`tools/beat-lint.mjs:164-166`) and the minutes
(`tools/beat-lint.mjs:183-185`) must equal `RUNG_NEW_ELEMENTS` and
`RUNG_MINUTES`, so the `new count` and `minutes` cells above are the same
numbers in both doors. The first-bout bar is not a lint rule
(`tools/beat-lint.mjs:11-15`): `crates/game/src/ladder.rs:49-51` owns it
(45 s target, 60 s cap) and its Rust test proves it.

## Reward table

A skipped fight pays nothing. No show, no glory, no rung move.

| outcome | glory | rung | note |
|---|---|---|---|
| win | 100 plus 25 per rung, plus 500 on a top-rung loop | climbs 1, loops to 0 at the top | the only source of glory |
| lose | -50 (`GLORY_LOSS_PENALTY`), floored at 0 | retreats 1, floor pins at 0 | the glory drain: a loss pays no reward and takes 50 back |
| skip | 0 | stays | a skipped fight pays nothing |
