# 0010 Parry window stays 7, sweep 8 to 13 is warranted

- Status: Proposed
- Context: v2 carries parry 7 with startup 15. House numbers are parry 6-8 and light startup 12-18. A common action-game reference parries at about 13 frames, wider than our 8-tick cap. The design test pins parry below startup and at 8 or fewer.
- Decision: Keep v2 parry at 7. Run one sweep of parry 8 to 13 with startup fixed at 15, then pick the value with its reason here. No code change until the sweep lands.
- Consequences: One number moves at a time. The sweep writes its before and after in `docs/design/balance-log.md`. Golden stays v1.
- Compliance: `crates/game/tests/it/design_combat.rs` `v2_parry_window_shorter_than_startup` fails if parry leaves `1..=8` or reaches startup.
- Advice: Systems designer, via GA14 sweep. Books search found no engine-book parry heading; house numbers come from the packet brief.
