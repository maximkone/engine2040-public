---
name: fight-timing-tables
description: Use when a builder changes fight timing or parry windows in crates/combat before the bout ships.
---
# Fight timing tables
1. **From and to.** Ask: does each row in `crates/combat/src/tables.rs::MoveTableSet` name source and target? (combat)
   Flag: a `tables_for_version(2)` row with `*` that lets death go to attack.
2. **Kind.** Ask: is each change typed instant, timed blend, or staged in `crates/combat/src/fight.rs::Tables`? (combat)
   Flag: a cross-fade where down-to-stand needs a staged state.
3. **Length.** Ask: are startup 12-18 and parry 6-8 as integer ticks in `crates/combat/src/fight.rs::ATTACK_STARTUP_TICKS`? (combat)
   Flag: float seconds or `PARRY_WINDOW_TICKS` 4 carried into v2.
4. **Curve.** Ask: does each timed blend name its ease shape beside `crates/combat/src/fight.rs::HITSTOP_TICKS`? (combat)
   Flag: a blend factor with no curve that snaps at the ends.
5. **Window.** Ask: can a cancel fire only inside its source window in `crates/combat/src/fight.rs::FighterState`? (combat)
   Flag: a punch-to-react that fires in the first half of the swing.
6. **Sparse map.** Ask: is the move matrix sparse with death as sink in `crates/combat/src/tables.rs::validate_tables`? (combat)
   Flag: a dense matrix that lets death reach any move.
7. **Defaults first.** Ask: does `TABLES_V1` stay frozen while `tables_for_version(2)` overrides listed rows? (combat)
   Flag: a v2 edit that rewrites v1 bytes and breaks old tapes.
8. **Proof.** Ask: does `cargo test -p engine-combat --lib tables_v2` pass x2 with golden hash unchanged? (combat)
   Flag: a green claim with no digest or a hash move with no receipt.
## Do / Don't
- Do: list from, to, kind, length, curve, and window per row in `crates/combat/src/tables.rs`. Don't: ship a timed change with no window or curve.
source: Gregory, Game Engine Architecture (2nd ed.), ch. 11.11.2.2-11.11.2.3
