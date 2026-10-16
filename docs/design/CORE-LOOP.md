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
7. Bout end: rung climbs on a win and retreats on a loss. Glory pays on wins only. The next tier waits.

## Beat chart per ladder rung

Rungs run `0..=7` (`MAX_RUNG` 7 in `crates/game/src/ladder.rs`).
Tier never falls. The boss is last. At most 3 new things per rung (Rogers L4).

| rung | tier | new, at most 3 | reward, glory counts |
|---|---|---|---|
| 0 | T1 recruit | move, light attack | 100, rank Tiro |
| 1 | T1 recruit | block answer | 125, rank Tiro |
| 2 | T2 veteran | dodge, heavy tell | 150, rank Tiro |
| 3 | T2 veteran | parry window | 175, rank Gladiator |
| 4 | T2 veteran | riposte, stamina pressure | 200, rank Gladiator |
| 5 | T3 champion | clash, poise stagger | 225, rank Gladiator |
| 6 | T3 champion | ring control | 250, rank Primus |
| 7 | T4 primus, boss last | phase 2 answer | 275 plus 500 per loop, rank Champion to Legend |

Glory math (`crates/game/src/ladder.rs`): win pays 100 plus 25 per rung held; a top-rung win loops to rung 0 and adds 500.
Rank floors: 0, 300, 900, 1800, 3600 to Tiro, Gladiator, Primus, Champion, Legend.
Minutes: rung 0 starts within 60 s of launch; each rung is about 1-2 min (one bout is at most five 10 s rounds plus resets).

## Reward table

A skipped fight pays nothing. No show, no glory, no rung move.

| outcome | glory | rung | note |
|---|---|---|---|
| win | 100 plus 25 per rung, plus 500 on a top-rung loop | climbs 1, loops to 0 at the top | the only source of glory |
| lose | 0 | retreats 1, floor pins at 0 | no penalty column in code today; loss moves the rung, never the glory |
| skip | 0 | stays | a skipped fight pays nothing |
