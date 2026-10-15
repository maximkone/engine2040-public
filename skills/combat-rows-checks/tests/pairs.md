# Pairs for combat-rows-checks

## 1. Float damage in rows
Input: "Review `content/design/combat.rows` where one row says `attack_damage=12.5` under `crates/combat/src/tables_text.rs`."
With skill: "Check 3, 7: `decimal` takes ASCII digits only and refuses dot, `parse_tables` refuses with the line in `CombatError`. Pass is a line-numbered refusal, `cargo test -p engine-combat --lib tables_text` green x2."
Without skill misses: accepts the float as 125 or 12, so text and `TABLES_V1` in `fight.rs` fork and the golden moves with no refusal line.

## 2. Typo key with missing unit
Input: "Review `content/design/combat.rows` with `startup=15` instead of `attack_startup_ticks=15` for `Tables` in `fight.rs`."
With skill: "Check 2, 4: key must be in `KNOWN` in `tables_text.rs`, each key carries `_ticks` per `docs/design/FORMAT.md`. Pass refuses the unknown bare key with its line."
Without skill misses: fills a default startup, so v2 keeps 4-tick tells and the unreadable timing the row was meant to fix.

## 3. Forked v2 that parses but differs
Input: "Review `crates/combat/src/tables_text.rs` where parsed v2 parses yet its digest differs from `TABLES_V2` and `v2_rows()` in `tables.rs`."
With skill: "Check 6, 5, 9: parsed v1/v2 must equal `TABLES_V1`/`TABLES_V2`, gated by `validate_tables` before `SwapInput`, proven by `cargo test -p engine-combat --lib tables_text` x2."
Without skill misses: ships the second door as a fork, so the proof scene and the duel run different numbers under one version tag.
