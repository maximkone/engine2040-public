# design

Notes about what the player sees and feels.

- [PLAYTEST.md](PLAYTEST.md): newcomer sessions, script, survey, funnel, bug grades.
- [FORMAT.md](FORMAT.md): rows text format designers edit (des-rows-doc).

## not data yet (crates/combat/src/fight.rs)

Tunable consts with no rows key; bounds stay consts, tables check inside them:

- `MAX_HP_PERMILLE` = 1000 (structural, stays const).
- `MAX_STAMINA` = 100 (structural, stays const).
- `MAX_INPUTS_PER_TICK` = 32 (hard bound, stays const).
- `MAX_EVENTS_PER_TICK` = 512 (hard bound, stays const).
- `MAX_ATTACK_STARTED_PER_TICK` = SLOT_COUNT (16 cues, hard bound, stays const).
- `FIGHT_RULES_V1` = 1, `FIGHT_RULES_V2` = 2 (version tags, not tunable).

All 16 `Tables` fields are data (see `content/design/combat.rows` when it lands); v1 stays frozen.
- [des-rows-doc]: `FORMAT.md` is the finished rows page (layout, units, legal ranges, worked v2 example); `content/design/` files follow it.
