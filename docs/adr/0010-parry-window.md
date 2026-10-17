# 0010 Parry window stays 7, the 8 to 13 sweep is measured

- Status: Accepted
- Context: v2 carries parry 7 with startup 15. House numbers are parry 6-8 and light startup 12-18. A common
  action-game reference parries at about 13 frames, wider than our 8-tick cap, so the sweep was warranted.
- Decision: Keep v2 parry at 7. The sweep below was run as tapes, not as prose
  (`crates/game/tests/it/design_combat.rs::parry_sweep_8_to_13_leaves_a_verdict_the_shipped_row_sits_on`),
  startup fixed at 15 and one number moved at a time: each candidate is a copy of `TABLES_V2` with only
  `parry_window_ticks` changed, and every press tick of the tell is scanned, not computed.

  Two gates decide a candidate:

  1. Tell gate: the parry may not answer the whole tell of the fastest weapon in `content/design/weapons.rows`
     (the pugio, 12 ticks), or the attacker's feint has no meaning (Bycer ch8: a tell and a commit).
  2. Half gate: the row may answer at most half of its own tell
     (`parry_window_ticks * 2 <= attack_startup_ticks`), or pressing late in a swing is a promise the
     defender can always keep (Sellers ch9-10: a gate needs a cost).

  Measured sweep, startup 15, tell 15 ticks, parrying press ticks read off the tapes:

  | parry | press ticks that parry | covered permille | tell gate | half gate |
  |---|---|---|---|---|
  | 7 (shipped) | 8 | 533 | pass | pass (14 <= 15) |
  | 8 | 9 | 600 | pass | fail (16 > 15) |
  | 9 | 10 | 667 | pass | fail |
  | 10 | 11 | 733 | pass | fail |
  | 11 | 12 | 800 | pass | fail |
  | 12 | 13 | 867 | fail (13 > 12) | fail |
  | 13 | 14 | 933 | fail (14 > 12) | fail |

  No candidate 8 to 13 survives both gates: 12 and 13 swallow the pugio's whole 12-tick tell, and 8 to 11
  all answer more than half of the tell while buying at most 4 extra press ticks. The widest window both
  gates allow with startup 15 and a 12-tick fastest tell is 7, which is the row that ships; it answers
  8 of 15 tell ticks (533 permille) and leaves 7 ticks where a parry is already late.
- Consequences: One number moves at a time. The sweep is now a check, so a wider window, a shorter startup
  or a faster weapon in `weapons.rows` moves the verdict with it. A chosen value other than 7 needs this
  table rerun and the reason rewritten here, plus a systems-designer line in `docs/design/balance-log.md`.
  Golden stays v1.
- Compliance: `crates/game/tests/it/design_combat.rs` `parry_sweep_8_to_13_leaves_a_verdict_the_shipped_row_sits_on`
  (sweep, both gates, the shipped numbers) and `v2_parry_window_shorter_than_startup` (the 8-tick house cap).
- Advice: Systems designer for any value other than 7, via a GA14 sweep. Books search found no engine-book
  parry heading; the house numbers come from the packet brief and the sweep above.
