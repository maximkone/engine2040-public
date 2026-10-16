---
name: combat-rows-checks
description: Use when a builder adds or reviews data-driven combat numbers in crates/combat; it checks that rows live in text files with validation before the diff lands.
version: 0.1.0
author: skillworks
---

# Combat rows checks

Nine checks for `crates/combat/src/tables_text.rs`. Numbers live in rows, Rust holds the truth, text must match it.

1. **Header first.** Ask: does line 1 of `content/design/combat.rows` read `table combat version <n>` via `parse_header` in `tables_text.rs`? Flag: missing or zero version that still loads. (combat)
2. **Known keys only.** Ask: is every key a member of `KNOWN` in `tables_text.rs` in `Tables` order? Flag: typo key that silently becomes a default. (combat)
3. **Whole numbers only.** Ask: does `decimal` in `tables_text.rs` take ASCII digits with overflow check and refuse dot, sign and empty? Flag: float or negative that parses as a nearby int. (combat)
4. **Units in keys.** Ask: does each key carry its unit (`_ticks` at 60 Hz, `_permille`, stamina points) per `docs/design/FORMAT.md` and `Tables` in `fight.rs`? Flag: bare `damage` or `startup` with no unit. (combat)
5. **Validate before use.** Ask: does `parse_tables` in `tables_text.rs` run `validate_tables` in `tables.rs` before building a `SwapInput`? Flag: parsed rows that step without `MoveTableSet::validate`. (combat)
6. **Digests equal truth.** Ask: do parsed v1 and v2 equal `TABLES_V1` and `TABLES_V2` in `fight.rs` with `v2_rows()` in `tables.rs`? Flag: text fork that passes parsing but moves the golden. (combat)
7. **Refuse with line.** Ask: does each refusal from `parse_tables` carry its line number in `CombatError` value? Flag: bad row with no line, or range failure naming no culprit value. (combat)
8. **Wired once, cold only.** Ask: is the loader `pub mod tables_text` in `lib.rs`, std-only, called at a tick boundary and never in the step? Flag: loader in the hot path or heap in `step_with_tables`. (combat)
9. **Doc plus x2 proof.** Ask: does `docs/design/FORMAT.md` match the file and does `cargo test -p engine-combat --lib tables_text` pass x2? Flag: doc drift or one green run that differs on retry. (combat)

## Do / Don't
- Do: keep one text door to the same numbers with line-numbered refusals. Don't: hard-code a tuned number in Rust or accept a float key.
source: Wolverson, Hands-on Rust, ch. 15
