# CORE-LOOP — the bout loop, the first minute, the ladder beats

One currency exists: glory (`crates/game/src/ladder.rs`).
Gold, crowd favor, economy-full: not in this slice (`docs/plan/gameplay.md:59`).

## Core loop

One line. Start state > player action > reward > changed state:

two fighters 8 m apart with full hp, stamina and poise > close distance, attack, defend, dodge > damage, stagger, stamina spent, parry punish > hp, stamina, poise, round clock and round wins changed

Session loop beside it: round (600 ticks, best of 5, first to 3) > ladder rung > glory rank > next opponent tier.
Round rule (`docs/plan/rome-spec.json`): 600 ticks at 60 Hz; higher total hp wins on timeout; an exact tie replays the round.
Arena numbers: half extent 10 m; spawns -4 m and +4 m; march 1.7 m/s; sword 25 damage, 0.5 s cooldown, 2.0 m range; hp 100 display = 1000 permille inside.

## First minute in order

What the player learns first, in order. Times at 60 Hz.

1. Launch to first bout within 60 s. No menu maze. The bar is first bout, not first win.
2. Tick 0: round 1 of best of 5. Two fighters 8 m apart. Full hp, stamina, poise. Round clock starts.
3. Ticks 0-212: close distance. 6 m to cover at 1.7 m/s is about 3.5 s. The player learns to move first.
4. By tick 240: first exchange. Light attack meets block. The player learns one attack and its one answer.
5. Ticks 240-600: dodge and parry appear. A dodge avoids; a fresh block press parries. Spam tires through stamina.
6. Round end: higher hp wins on timeout; all foes at 0 hp wins early; a tie replays. Best of 5, first to 3.
7. Bout end: rung climbs on a win and retreats on a loss. Glory pays on a win and drains 50 on a loss. The next tier waits.

## Beat chart per ladder rung

Rungs run `0..=7` (`MAX_RUNG` 7 in `crates/game/src/ladder.rs`).
Tier never falls (`RUNG_ENEMY_TIER`); the boss is last (`BOSS_TIER` 4).
New things per rung are `RUNG_NEW_ELEMENTS`, each rung at most 3
(`MAX_NEW_ELEMENTS_PER_RUNG`, Rogers L4). Minutes are `RUNG_MINUTES`.
Both columns below are the ladder's own numbers, never a second copy.

| rung | tier | new, at most 3 | new count | minutes | reward, glory counts |
|---|---|---|---|---|---|
| 0 | T1 recruit | move, light attack | 2 | 2 | 100, rank Tiro |
| 1 | T1 recruit | block answer | 1 | 2 | 125, rank Tiro |
| 2 | T2 veteran | dodge, heavy tell | 2 | 3 | 150, rank Tiro |
| 3 | T2 veteran | parry window | 1 | 3 | 175, rank Gladiator |
| 4 | T2 veteran | riposte, stamina pressure | 2 | 4 | 200, rank Gladiator |
| 5 | T3 champion | clash | 1 | 4 | 225, rank Gladiator |
| 6 | T3 champion | poise stagger, ring control | 2 | 5 | 250, rank Primus |
| 7 | T4 primus, boss last | phase 2 answer | 1 | 6 | 275 plus 500 per loop, rank Champion to Legend |

Glory math (`crates/game/src/ladder.rs`): win pays 100 plus 25 per rung held; a top-rung win loops to rung 0 and adds 500.
Rank floors: 0, 300, 900, 1800, 3600 to Tiro, Gladiator, Primus, Champion, Legend.
Minutes: `RUNG_MINUTES` is the per-rung bout budget, the `minutes` cell above,
2, 2, 3, 3, 4, 4, 5, 6 and rising with the rung. One bout is at most five 10 s
rounds plus resets, so a rung budget can never be read as one round. Rung 0
starts within 60 s of launch (`FIRST_BOUT_START_LIMIT_SECS`), and
`FIRST_BOUT_START_SECS` 45 is the target inside that cap.
Second door: `content/design/ladder.rows` is the designer beat chart and
`tools/beat-lint.mjs` gates it (at most 3 new per rung, tier never falls, boss
last, reward and rank equal to `crates/game/src/ladder.rs`). The lint mirrors
reward, rank, the 3-new cap and the first-bout cap, but not the per-rung minute
and new-count arrays, so the `new count` and `minutes` cells above are the
compiled `RUNG_NEW_ELEMENTS` and `RUNG_MINUTES` that `Ladder::beat()` returns,
while the rows file stated its own (minutes 1, 1, 2, 2, 2, 3, 3, 4; new totals
2, 2, 2, 3, 2, 2, 2, 3) and no gate could trip on that. Packet
`des-ladder-chart-drift-20261017` brings the rows file and the lint onto these
same arrays, so neither ever carries a third copy.

## Reward table

A skipped fight pays nothing. No show, no glory, no rung move.

| outcome | glory | rung | note |
|---|---|---|---|
| win | 100 plus 25 per rung, plus 500 on a top-rung loop | climbs 1, loops to 0 at the top | the only source of glory |
| lose | -50 (`GLORY_LOSS_PENALTY`), floored at 0 | retreats 1, floor pins at 0 | the glory drain: a loss pays no reward and takes 50 back |
| skip | 0 | stays | a skipped fight pays nothing |
