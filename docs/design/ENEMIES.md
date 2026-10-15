# ENEMIES — arena duel enemies (design)

Seat: enemy designer. Date: 2026-10-12.
Goal: every enemy has one reason to exist and one counter from the player's own kit.
Rule: no enemy attack without a tell and a counter.

## Code kinds read (invent none)

Read 2026-10-12, byte-exact:

- `crates/ai/src/gladiator.rs`: `Intent` (Attack, Block, Dodge, Advance, Retreat, Reposition). One `Percept` in, one `FighterInput` out.
- `crates/ai/src/depth.rs`: `Role` (Holder pins the center and trades blows; Flanker circles for side arcs). `Phase` (Open, Pressure, Finish) is pacing only.
- `crates/ai/src/melee.rs`: `StrikePhase` (Idle, Windup, Active, Recovery). `is_threat` is Windup or Active. Not a kind. A clock.
- `crates/ai/src/personality.rs`: `Personality` (reaction 0-5 ticks, range 1000-1500 mm, aggression 250-750). Not a kind. A temperament.
- `crates/ai/src/legion.rs`: `Formation` (Line, Column, Wedge, ShieldWall) and `Order` (Hold, Advance, Charge, Retreat, FlankLeft, FlankRight). Squad shapes for hundreds of soldiers. Not single-duel kinds.

Tier check (re-checked 2026-10-14): `rg -n "grunt|veteran|elite|boss" crates/ai/src` hits `crates/ai/src/depth.rs` (`SHIPPED_TIERS` tiers 1-3 matching `content/design/enemies.rows`, `PRIMUS_TIER` tier 4 code side, one `elite` doc comment).
Tiers 1-3 below are shipped in code; the tier_4 file row lands with GA13.

## Global numbers both cards use

Combat is v2 (`content/design/combat.rows`, `crates/combat/src/tables.rs`):

- Tell: `attack_startup_ticks=15` (250 ms at 60 Hz). Recovery 8. V1 startup 4 is replay-only.
- Parry window 7 (below startup, so a held block ages out). Parry stagger 12.
- Dodge duration 10, i-frames 6. Dodge cost 20, attack cost 25, regen 2 per tick.
- Damage 120 permille. Chip 1/5. Hit react 6, hitstop 4, knockdown 30.
- Every attack reads Windup (15) then Active (1) then Recovery (8) from `StrikePhase`.

Bodies and space:

- Size: capsule radius 0.4 m, height 1.8 m (`crates/arena/src/lib.rs`). Both cards use the same body as the player. Fair duel.
- Reach: blade 200-1500 mm (`crates/arena/src/match_wire.rs`). Preferred swing 1000-1500 mm (`personality.rs`).
- Speed: top 5.0 m/s (`TOP_SPEED_MPS`). Dodge covers about 0.83 m (5 m/s over 10 ticks).
- Wait ring 3000 mm, edge 500 mm (`melee.rs`). Player spawns 8 m apart (`docs/design/CORE-LOOP.md`).

## House numbers

Two numbers below are house numbers, not measured facts:

- The 60% tier bar (a tier beats the tier below in 60% or more of seeded duels). Stated on each tier row. Keep until the tournament proves another number.
- The 2-4 tick human noise (33-66 ms) plus one action per 8 ticks in `crates/playtest/src/winrates.rs`. The code marks it as a house number itself. Calibrate it on recorded human tapes when any exist. Today no tapes exist, so the number stays.

## Card 1 — Holder duelist (`Role::Holder`)

- Size: capsule 0.4 m radius, 1.8 m tall. Same as the player.
- Archetype: center holder. Pins the middle and trades blows. Block bias +75, attack +25 (`depth.rs`).
- Speed: top 5.0 m/s, same as the player. Advance bias -25. It does not chase. It holds.
- Moves: Attack, Block, Dodge, Advance, Retreat (plus Reposition when far). Same `Intent` set as the player.
- Tell in ticks: 15. Windup 15, Active 1, Recovery 8. Read from `StrikePhase`.
- Counter: parry. A fresh block press inside the 7-tick window negates the hit and staggers the holder 12 ticks. Dodge (6 i-frames) also works, but parry is the intended answer.
- Reason for HP: 1000 permille (100 display). Same as the player. HP tests answers, not grind. Stats stay in the narrow 900-1100 band per `docs/design/matchups.csv`. Edge comes from hit rate, not from HP.
- Silhouette note: square stance, shield forward, feet planted. Reads as a wall. Spectacle serves information, never hides the tell.
- Question it asks the player:
  - "Can you avoid my damage?" Answer: parry. Fresh block inside 7 ticks.
  - "Can you hit me?" Answer: attack in the punish window. Its Recovery is 8 ticks. Strike then, as `melee.rs` rewards with +250 attack bias on a recovering target.
- Mobility check: reach 1.5 m. From the 3 m wait ring it closes in about 36 ticks at 5 m/s. Player dodge covers 0.83 m per 10 ticks. The holder is not faster than the player, but it does not need to be. It holds the center and forces the player to come. It is a threat because it matches escape speed and cuts the ring, not because it outruns. An enemy slower than the player's escape is never a threat. This one is not slower.

## Card 2 — Flanker duelist (`Role::Flanker`)

- Size: capsule 0.4 m radius, 1.8 m tall. Same as the player.
- Archetype: circler. Goes for side arcs. Advance bias +75, dodge bias +50 (`depth.rs`).
- Speed: top 5.0 m/s, same as the player. It uses speed sideways, not head-on. It wraps and then closes.
- Moves: Attack, Block, Dodge, Advance, Retreat (plus Reposition when far). Same `Intent` set as the player. It prefers Dodge and Advance.
- Tell in ticks: 15. Windup 15, Active 1, Recovery 8. Same clock as Holder. Its arc reads as a sidestep before the swing.
- Counter: dodge. Sidestep the arc. The 6 i-frame ticks cover the 1 Active tick with margin. Block is the backup answer. Parry still works but the window is harder on a moving foe.
- Reason for HP: 1000 permille (100 display). Same as the player, same reason as Holder. Skill first. Veterans in `matchups.csv` sit at 110 HP only to set the odd cycle, still inside the narrow band.
- Silhouette note: bladed stance, body sideways, always moving. Reads as a knife circling. Distinct from the Holder wall at a glance.
- Question it asks the player:
  - "Can you avoid my damage?" Answer: dodge. Move with the arc, not into it.
  - "Can you hit me?" Answer: cut it off, then attack. Advance to close the 3 m ring, then strike when it commits. When it recovers (8 ticks), punish as with Holder.
- Mobility check: reach 1.5 m. It closes the 3 m ring in about 36 ticks at 5 m/s, same math as Holder. Player dodge is 0.83 m per 10 ticks. The flanker matches escape speed and adds lateral angle, so it stays a threat. A straight-line runner slower than 5 m/s would never catch a dodging player. This one does not run straight.

## Why no other cards

- `StrikePhase` is the shared clock, not an enemy.
- `Personality` (reaction, range, nerve) tunes both cards. Bold (750) presses, shy (250) guards. Both stay inside the same two cards.
- `Formation` and `Order` (Line, Wedge, ShieldWall, Charge, Flank) are squad shapes for the siege field, not single duelists. One melee enemy at a time per encounter. Squads wait for the legion seat.

## Tier plan (T1-T3 shipped in `depth.rs`, T4 code side landed, file row with GA13)

| tier | name | card | change from below | bar |
|---|---|---|---|---|
| T1 | grunt | Holder, calm (aggression 400, reaction 3-5) | first foe, slow to notice | beats nothing yet, teaches parry |
| T2 | veteran | Flanker, even (aggression 500, reaction 1-3) | adds the arc, must dodge | beats T1 in 60%+ of seeded duels (house number) |
| T3 | elite | Holder, bold (aggression 700) plus Flanker mix | stamina pressure, riposte | beats T2 in 60%+ of seeded duels (house number) |
| T4 | boss (code side landed, file row with GA13) | Holder body, Flanker feet, phase 2 | phase answer, see below | beats T3 in 60%+ of seeded duels (house number) |

Noise for all tiers: perfect player first (a variant nobody can finish is skipped), then the seeded 2-4 tick human error, default off. House number until human tapes calibrate it.

Boss: owner answered with `EB-2026-10-13-S2` [BOSS-primus-t4] (both asks CLOSED 2026-10-12/13): build the T4 primus boss as these cards propose it. Code side landed (`PRIMUS_TIER` in `crates/ai/src/depth.rs`: 1100 HP inside the narrow band, 15-tick tell, parry answer, phase 2 at or below 500 HP). The `tier_4` file row still lands with GA13, so `design_enemy.rs` still seats three tiers. No new boss stats beyond that answer.

## What the pilot checks

- Every swing shows its 15-tick Windup. No instant hit.
- Holder parry works in the 7-tick window and staggers 12.
- Flanker arc is dodged with the 6 i-frame ticks.
- Both cards at 1000 HP feel fair. Wins come from answers, not from HP.
- Open: boss kit, phase 2, and the calibrated noise number.
