# LEDGER — bout pools, sources and drains

One currency exists: glory (`crates/game/src/ladder.rs`).
Gold, crowd favor: not in this slice (economy-full CUT, `docs/plan/gameplay.md:59`).

Every value that changes in a bout has a named source and drain below.
A balance bar that can fail runs after every change:
`pq2-balance-tournament` caps kill share at 350 permille
(`crates/playtest/src/tournament.rs:40`); `bench` mirrors it at 350
(`crates/game/src/bench.rs:65`); one gate stays, no second number.
Trial count 200 per difficulty is stated before any rate; one change per
run; before and after go in `docs/design/balance-log.md`.

## Pools

| pool | source | drain | converter | file |
|---|---|---|---|---|
| hp | full at round start (`FightState::default`, 1000 permille) | hits (`Tables` attack_damage 120 permille) | damage to round win (higher hp wins on timeout; best of 5, first to 3) | `crates/combat/src/fight.rs` |
| stamina | regen 2 per tick after quiet delay (`Tables` stamina_regen_per_tick) | attack 25, dodge 20, block hold 1 per tick (per kind) | spend to no act (`StaminaOut`; spam tires) | `crates/combat/src/tables.rs` |
| morale | landed hits, crowd (`crowd_volume_permille`, `morale_step` drift) | taken hits, hp share (foe damage) | morale to lanes (high attacks and advances; low blocks, dodges and retreats) | `crates/ai/src/gladiator.rs` |
| poise | regen 1 per quiet tick after 90 quiet ticks (100 cap) | poise damage per hit (parry 250, clash 200) | zero to stagger (`recover_from_stagger`; hits become openings) | `crates/combat/src/depth.rs` |
| glory | won bout (100 plus 25 per rung; 500 per loop) | none in code (loss retreats rung; glory never falls) — BUG packet `des-glory-drain` | glory to rank (floors 0, 300, 900, 1800, 3600 to Tiro, Gladiator, Primus, Champion, Legend) | `crates/game/src/ladder.rs` |

A row with no drain is a bug. Glory above has no drain: filed as packet
`des-glory-drain` for the wire builder (add a loss penalty or a sink;
`crates/game/src/ladder.rs` values stay untouched here).

## Loops

Main are rows 1-3 (3 mains). Minor are rows 4-6 (3 minors).

| system | loop type | resource | source | sink |
|---|---|---|---|---|
| exchange | engine | hp | round-start refill | hits to zero (round win) |
| pressure | engine | stamina | quiet regen | attack, dodge and block spend (exhaustion) |
| ladder | economy | glory | bout wins | rank (no sink today; see glory BUG) |
| stagger | engine | poise | quiet regen | poise damage (stagger punish) |
| nerve | ecology | morale | landed hits and crowd | taken hits (retreat) |
| repair | economy | pending_repairs | top-rung loop (arms 1) | `take_repair` (fires once) |
