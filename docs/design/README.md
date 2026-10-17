# design

Notes about what the player sees and feels.

- [PLAYTEST.md](PLAYTEST.md): newcomer sessions, script, survey, funnel, bug grades.
- [FORMAT.md](FORMAT.md): rows text format designers edit (des-rows-doc).
- [CONCEPT.md](CONCEPT.md): game concept and pillars.
- [CORE-LOOP.md](CORE-LOOP.md): core loop and progression.
- [ENEMIES.md](ENEMIES.md): enemy archetypes and encounters.
- [HUD.md](HUD.md): HUD layout and overlays.
- [LEVELS.md](LEVELS.md): level flow and arenas.
- [KAM-RTS-IDEAS.md](KAM-RTS-IDEAS.md): ideas from the kam-web RTS port (command layer, lockstep, logistics, AI knobs, fighter plus commander).
- [SLICE.md](SLICE.md): vertical slice scope.
- [STAGES.md](STAGES.md): stage gates and delivery stages.
- [LEDGER.md](LEDGER.md): design decision ledger.
- [balance-log.md](balance-log.md): balance change log.
- [burndown.tsv](burndown.tsv): burndown data (planner-producer-r355 owns it).
- [matchups.csv](matchups.csv): matchup table data.

## not data yet (crates/combat/src/fight.rs)

Tunable consts with no rows key; bounds stay consts, tables check inside them:

- `MAX_HP_PERMILLE` = 1000 (structural, stays const).
- `MAX_STAMINA` = 100 (structural, stays const).
- `MAX_INPUTS_PER_TICK` = 32 (hard bound, stays const).
- `MAX_EVENTS_PER_TICK` = 512 (hard bound, stays const).
- `MAX_ATTACK_STARTED_PER_TICK` = SLOT_COUNT (16 cues, hard bound, stays const).
- `FIGHT_RULES_V1` = 1, `FIGHT_RULES_V2` = 2 (version tags, not tunable).

All 16 `Tables` fields are data (see `content/design/combat.rows`); v1 and v2 stay frozen.

## finished rows

- [des-rows-combat]: `content/design/combat.rows` ships v1 plus v2 copies; `crates/combat/src/tables_text.rs` parses them into `SwapInput` (unknown/missing keys refused, ranges are `validate_tables`). Proof: `cargo test -p engine-combat --lib tables_text`, 7 pass (re-measured 2026-10-17T00:36Z).
- [des-rows-doc]: `FORMAT.md` is the finished rows page (layout, units, legal ranges, worked example); `content/design/` files follow it. A shipped row is frozen, so shipping a changed number means a new header version plus a new row id, and the page now shows that path.
- [des-reload] (board EG29): practice reload watches `content/design/combat.rows` through `engine_assets::watcher` and applies it as a recorded swap at a tick boundary; a stale or invalid file is refused and last-good stays. Proof: `cargo test -p engine-game --lib practice` 3 pass, `--lib reload_keeps_last_good` 1 pass.
- [des-rows-ai]: `content/design/ai.rows` parses into `AiConfig` through `crates/ai/src/curves_text.rs`, ranges are `AiConfig::try_new`. Proof: `cargo test -p engine-ai --lib curves_text` 3 pass.
- [des-rows-ladder]: `content/design/loop.rows` folds the `crates/game/src/ladder.rs` constants at a progression boundary, the named loss drain (`glory_loss_penalty`) included; bad rows keep last-good. Proof: `cargo test -p engine-game --lib rows` 13 pass.

## not data yet (crates/combat/src/tables.rs)

Per-kind V2 numbers still live in Rust with no rows key. A `combat` rows row
carries the 16 `Tables` fields only, so a designer cannot edit these yet:
`INPUT_BUFFER_TICKS_V1` = 0 and `INPUT_BUFFER_TICKS_V2` = 8 ticks,
`LIGHT_POISE_DAMAGE_V2` = 60 and `HEAVY_POISE_DAMAGE_V2` = 100 poise points,
`LIGHT_STAMINA_COST_V2` = 15 and `HEAVY_STAMINA_COST_V2` = 30 stamina points,
`POISE_REGEN_DELAY_TICKS_V2` = 30 ticks, and the per-kind freeze
`LIGHT_HITSTOP_TICKS_V2` = 3, `HEAVY_HITSTOP_TICKS_V2` = 6,
`PARRY_HITSTOP_TICKS_V2` = 8 ticks. The `MAX_*` bounds beside them are
structural. These want their own keys (see rows GP31 and GP30 on the board).
The v2 row's own 15/7 literals are already data in `combat.rows`.
