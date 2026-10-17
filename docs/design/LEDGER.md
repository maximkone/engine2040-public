# LEDGER — bout pools, sources and drains

One currency exists: glory (`crates/game/src/ladder.rs`).
Gold, crowd favor: not in this slice (economy-full CUT, `docs/plan/gameplay.md:108`).

Every value that changes in a bout has a named source and drain below.
A balance bar that can fail runs after every change:
`pq2-balance-tournament` caps kill share at 350 permille
(`crates/playtest/src/tournament.rs:46`); `bench` mirrors it at 350
(`crates/game/src/bench.rs:66`); one gate stays, no second number.
Trial count 200 per difficulty is stated before any rate; one change per
run; before and after go in `docs/design/balance-log.md`.

## Pools

| pool | source | drain | converter | file |
|---|---|---|---|---|
| hp | full at round start (`FightState::default`, 1000 permille) | hits (`Tables` attack_damage 120 permille) | damage to round win (higher hp wins on timeout; best of 5, first to 3) | `crates/combat/src/fight.rs` |
| stamina | regen 2 per tick after quiet delay (`STAMINA_REGEN_PER_TICK`) | attack 25 (`ATTACK_STAMINA_COST`), dodge 20 (`DODGE_STAMINA_COST`), block hold 1 per tick (`BLOCK_HOLD_COST_PER_TICK`) | spend to no act (`StaminaOut`; spam tires) | `crates/combat/src/fight.rs` |
| morale | offline only: fork `SetAggression` (`crates/arena/src/fork.rs`) or the editor tuning panel; a live bout needs the rung on the match config first | offline only (no live-bout drain) | morale to lanes (high attacks and advances; low blocks, dodges and retreats) | `crates/ai/src/gladiator.rs` |
| poise | regen 1 per quiet tick after 90 quiet ticks (100 cap) | poise damage per hit (parry 250, clash 200) | zero to stagger (`recover_from_stagger`; hits become openings) | `crates/combat/src/depth.rs` |
| glory | won bout (100 plus 25 per rung; 500 per loop) | lost bout (50, `GLORY_LOSS_PENALTY`, floored at 0) | glory to rank (floors 0, 300, 900, 1800, 3600 to Tiro, Gladiator, Primus, Champion, Legend) | `crates/game/src/ladder.rs` |

A row with no drain is a bug. Every row above names one: a lost bout
drains 50 glory (`GLORY_LOSS_PENALTY`, floored at 0), so the counter can
fall as well as climb.

## Loops

Main are rows 1-3 (3 mains). Minor are rows 4-6 (3 minors).

| system | loop type | resource | source | sink |
|---|---|---|---|---|
| exchange | engine | hp | round-start refill | hits to zero (round win) |
| pressure | engine | stamina | quiet regen | attack, dodge and block spend (exhaustion) |
| ladder | economy | glory | bout wins | lost bout drains 50, then rank |
| stagger | engine | poise | quiet regen | poise damage (stagger punish) |
| nerve | ecology | morale (design only: no live bout drives it) | designed: landed hits and the crowd; live: fork `SetAggression` and the editor tuning panel only | designed: taken hits (retreat); live: none, same as the pools row above |
| repair | economy | pending_repairs | top-rung loop (arms 1) | `take_repair` (fires once; public API, no live consumer yet) |

Rows 5 and 6 are labelled design-only on purpose: `morale_step` says no live
bout calls it (`crates/ai/src/depth.rs:1566`), `apply_morale_drift` has no
non-test caller, and `take_repair` has no non-test caller either
(`crates/game/src/ladder.rs:307`). The nerve ring's source is data with no
caller today (`crowd_volume_permille` and `morale_drift` are tension-table
rows, `crates/ai/src/depth.rs:1447`), not a bout event, so it is named as
designed until the arena drives it. Rows 1-4 are live: hp, stamina, glory and
poise all move inside a bout, each with the drain named above.
