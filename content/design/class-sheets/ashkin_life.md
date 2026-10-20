# Ashkin Life — class sheet

A healer who moves injury instead of mending it. Injury is tracked in points (1 point = 1 damage) and the sheet reads the fight as a ledger: what she takes, what she pushes onto somebody else, what she mends.

**Role:** healer and injury broker · ashkin · tier 1 (branch `ashkin_officiator`)
**Growth:** +2 WIS, +1 DEX per level · **Primary:** WIS, DEX
**Loadout:** ledger stave · oath tablet
**2026 fix (roster):** an injury transfer HUD and a hard cap.

Data file: `content/classes/ashkin_life.json`.

## House numbers

These are the numbers this sheet is written in. They are design numbers, not measured ones.

- 60 Hz tick. Range and radius in metres, cast and cooldown in seconds, mana in points.
- **Damage and healing:** `D = base x (1 + 0.04 x (S - 10))`, `S = 10 + growth x (level - 1)`.
  `damage.base` in the class file is the level-1 raw number. WIS: L1 = 10 (x1.00) ·
  L10 = 28 (x1.72) · L20 = 48 (x2.52) · L40 = 88 (x4.12).
- **Health:** `200 + 25 x CON`. **Mitigation:** 1% per CON point above 10, capped 35%.
  The Life has 0 CON growth, so it is 500 health and 0% mitigation at every level.
- **A standard weapon hit** (gladius) is 120 damage, 12% of a 1000 health fighter.
  4.2 hits kill the Life.
- **Telegraph floor:** 340 ms is the readable minimum
  (`crates/combat/tests/it/telegraph_floor.rs`). Nothing on this sheet telegraphs.
- Caps and effects are flat. Only the hit and the heal scale on WIS.

The reference build below is level 10 with no extra points in WIS: WIS 28, x1.72.

## Role

Healer and injury broker. The Life has no damage of her own. She wins the long fight
and loses the short one. Every cap on this sheet exists so she cannot cheat the
ledger.

## Stat priority

1. **WIS** — +2 per level. Every heal and every debuff number.
2. **DEX** — +1 per level. Move speed and cast speed, so she can reach the wound and
   leave it before the next hit lands.
3. **CON** — nothing. 500 health, 0% mitigation, at every level. That is the weakness.
4. **STR, INT** — never taken.

What WIS buys at the reference build (WIS 28, x1.72):

| number | base | at L10 |
|---|---|---|
| Mending Song | 40 per second | 69 per second, 344 per cast |
| Cleanse Rite | 60 | 103 |
| Vital Surge | 250 | 430 |
| Mending Choir | 20 per second | 34 per second |
| Wound Lantern | 40 per target | 69 per target |

Mana: 110 at level 1, +7 per level. Regen 5 per second out of combat, 1.5 per second
in combat.

## The injury model

- Injury is damage taken, tracked in points. 1 point = 1 damage point.
- Injury shows as a percent of max health. 40% slows you 20%, 70% cuts movement 40%,
  100% downs you.
- **Nothing is deleted.** Injury moved onto the Life lands on the Life as real
  damage. The redirect moves damage to the ally; it never erases it.

Three tools, three different rules:

| tool | cap | cost |
|---|---|---|
| Burden Shift | 30% of the target injury, hard cap 25% of the Life max health: 125 damage points at 500 health | 14 s cooldown, 16 mana |
| Burden Relay | 200 damage moved in total, over 8 s. Damage past the cap still lands on the Life | 1 mana per 10 damage moved, 20 mana at most |
| Return From Ash | one revive per 300 s | 12% of the XP needed for the next level |

## Abilities

| tier | name | kind | shape | range | cast | cd | mana | key numbers |
|---|---|---|---|---|---|---|---|---|
| 0 | Mending Song | heal | target | 18 (4 m aura) | 1.2 | 3.0 | 12 | 69 per second for 5 s (344 per cast) to the target and up to 3 allies within 4 m |
| 0 | Mending Ward | buff | target | 16 | 0.3 | 20 | 8 | 25% less damage for 6 s, does not stack. On a standard 120 damage hit that is 30 points off every time |
| 0 | Burden Shift | utility | target | 20 | 0.8 | 14 | 16 | moves up to 30% of target injury into her, hard cap 125 points (25% of 500 health) |
| 1 | Wound Lantern | debuff | circle | 20 | 0.0 | 16 | 20 | 5 m circle, 69 damage, +25% injury from every hit for 6 s |
| 1 | Burden Relay | buff | target | 20 | 0.0 | 25 | 20 | 8 s, up to 200 damage moved to one ally. Past the cap it lands on her. 1 mana per 10 damage moved |
| 1 | Cleanse Rite | heal | target | 16 | 0.6 | 18 | 14 | 103 healing, removes 2 debuffs. Interruptible |
| 2 | Life Exchange | utility | target | 18 | 1.0 | 30 | 24 | Life and target swap injury, each capped at 30% of own max health (150 and 300 points). Overflow stays. Target loses 1 debuff. She is left at 40% move speed for 2 s |
| 2 | Mending Choir | heal | self | 0 (8 m aura) | 0.0 | 30 | 26 | 12 s aura, 34 per second to every ally within 8 m. One aura per ally |
| 3 | Vital Surge | heal | target | 20 | 0.8 | 60 | 30 | 430 healing, clears 2 debuffs, target deals +25% damage for 6 s. Does not work on a downed ally |
| 4 | Return From Ash | heal | target | 24 | 4.0 | 300 | 40 | 4 s channel, ally returns at 40% health with half their injury cleared. Costs 12% of a level of XP |

## Effect vocabulary

Effects are objects: `type`, `value`, `duration` (seconds, 0 = instant).

| type | value means | duration means |
|---|---|---|
| heal_per_second | healing per second | how long it ticks |
| heal_burst | flat healing | 0 |
| damage_reduction | percent less damage taken | how long |
| damage_boost | percent more damage dealt | how long |
| debuff_cleanse | how many debuffs | 0 |
| injury_transfer | percent of the target injury moved | 0 |
| injury_transfer_cap | percent of the Life max health | 0 |
| injury_swap | percent of max health, per side | 0 |
| injury_amplify | percent more injury taken | how long |
| damage_redirect | damage points that can be moved | how long the relay lasts |
| damage_redirect_mana | mana per 10 damage moved | how long |
| move_speed | percent change, negative is slower | how long |
| resurrect | percent of max health on return | 0 |
| xp_cost | percent of a level of XP | 0 |
| aura | radius in metres | how long |

## HUD contract

The split has to be readable at a glance, on every frame, for every ally.

- Every ally frame shows three bars: **HURT** (their injury as a percent), **PUSHED**
  (injury the Life has moved onto them) and **MENDED** (health the Life has restored
  on them).
- The Life own frame shows **HURT** plus a two-number split line: **MOVED IN 240** and
  **MOVED OUT 60**. Burden Shift raises MOVED IN, Burden Relay raises MOVED OUT on the
  marked ally.
- After every Burden Shift the two bars must move in lockstep: the target HURT drops by
  exactly what the Life HURT rises. The numbers are the proof that nothing was deleted.
- Return From Ash shows a 4 s channel bar and, when it fires, a red **-12% XP** line on
  the Life frame. A failed channel shows **CHANNEL FAILED** and no XP line.

## What it loses to

| enemy | why it hurts |
|---|---|
| warden_rogue, warden_assassin | they kill her in 4-5 standard hits (120 damage each). WIS does not block a knife and she has 500 health and 0% mitigation |
| a boss that hits hard | Burden Relay is capped at 200 damage, under two standard hits. The rest lands on her |
| her own tools | one Burden Shift of 125 points onto a low ally can down that ally. The caps limit the damage, not the mistake |
| warden_defender and other armour | 25% damage reduction with 0 injury growth cuts her heals by a quarter and gives her nothing to move |

The Life cannot finish a duel. If the party dies, she dies with them.

## What it counters

| enemy | why it works |
|---|---|
| warden_fighter, warden_warrior | they trade hits and rely on regen and outlast. She wins the long fight, and every Burden Shift makes their damage worse |
| ashkin_attacker | a drain bruiser who heals 35% of every hit that lands and nothing else. A mended party outlasts the drain: he wins the first minute, not the third |

Counter window for them: the numbers are on the HUD. A warden who reads **MOVED IN
240** knows the Life is loaded and that the next shift lands on him.

## Numbers to watch

- The 125 point cap on Burden Shift (25% of 500 health) is the load-bearing number.
  Lower it and the class becomes a damage sponge for the team, raise it and she
  becomes a second health bar for the boss.
- The 200 damage cap on Burden Relay is under two standard hits (120 each). That is
  deliberate: the redirect must never be a free delete for the boss.
- 12% of a level of XP for the revive is the price. It is paid in progress, not in
  numbers, so it never breaks the economy.
- 0 CON growth, 500 health and 0% mitigation are the weakness on purpose. Keep them.
