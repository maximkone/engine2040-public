# Pairs for fight-timing-tables
## 1. Startup too fast
Input: diff sets `ATTACK_STARTUP_TICKS` 4 in `crates/combat/src/fight.rs` for v2.
With skill: fails check 3. Ask for 12-18 ticks. Flag v1 value carried over.
Without: approves fast jab. Playtest reads as poke. Misses window rule.
## 2. Parry with no window
Input: `tables_for_version(2)` adds parry 4 ticks with no source window in `crates/combat/src/tables.rs`.
With skill: fails checks 5 and 7. Ask for window in `FighterState`. Flag dense row.
Without: lands parry everywhere. Misses sparse map and sink rule.
## 3. V1 rewritten
Input: v2 edit touches `TABLES_V1` bytes in `crates/combat/src/fight.rs`.
With skill: fails checks 7 and 8. Ask for frozen v1 plus x2 digest. Flag tape break.
Without: passes tests once. Misses replay and hash proof.
