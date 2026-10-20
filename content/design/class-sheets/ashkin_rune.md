# Ashkin Rune — class sheet

An elemental painter. Every spell is a 0 s cast, so the Rune never stops to channel: it walks into a group and paints it. One rule holds it together, the per-cast budget.

**Role:** AoE nuker · ashkin · tier 1 (branch `ashkin_officiator`)
**Growth:** +2 INT, +1 DEX per level · **Primary:** INT, DEX
**Loadout:** runewrought stave · bound rune tablet
**2026 fix (roster):** elemental lines castable on the move, with an INT-scaled cap on group damage.

Data file: `content/classes/ashkin_rune.json`.

## House numbers

These are the numbers this sheet is written in. They are design numbers, not measured ones.

- 60 Hz tick. Range and radius in metres, cast and cooldown in seconds, mana in points.
- **Damage:** `D = base x (1 + 0.04 x (S - 10))`, `S = 10 + growth x (level - 1)`.
  `damage.base` in the class file is the level-1 raw number. INT: L1 = 10 (x1.00) ·
  L10 = 28 (x1.72) · L20 = 48 (x2.52) · L40 = 88 (x4.12).
- **Health:** `200 + 25 x CON`. **Mitigation:** 1% per CON point above 10, capped 35%.
  The Rune has 0 CON growth, so it is 500 health and 0% mitigation at every level.
- **A standard weapon hit** (gladius) is 120 damage, 12% of a 1000 health fighter.
  4.2 hits kill the Rune.
- **Telegraph floor:** 340 ms is the readable minimum
  (`crates/combat/tests/it/telegraph_floor.rs`). Every telegraph on this sheet is
  0.4 s or longer.
- Effects are flat. Only the hit scales on INT.

The reference build below is level 10 with no extra points in INT: INT 28, x1.72.

## Role

Group nuker. The Rune wins by choosing when to spend the budget and when to hold it.
It loses because it has 0 CON growth, one armor skin and no way to survive a lone
melee attacker.

## Stat priority

1. **INT** — +2 per level. Every hit on this sheet and the budget itself.
2. **DEX** — +1 per level. Move speed and cast speed, so the Rune can aim while it
   walks and can leave the circle it just painted.
3. **CON** — nothing. 500 health, 0% mitigation, at every level. That is the weakness.
4. **STR, WIS** — never taken.

What INT buys at the reference build (INT 28, x1.72):

| number | base | at L10 |
|---|---|---|
| per-cast budget | 440 | 757 per cast |
| Shatterstone per target | 110 | 189 |
| Stormlance per target | 100 | 172 |
| Frostbloom per target | 120 | 206 |
| Runebolt | 60 | 103 |
| Severance per target | 180 | 310 |
| Severance own hard cap | 600 | 1032 per cast |

Mana: 120 at level 1, +8 per level. Regen 6 per second out of combat, 2.5 per second
in combat.

## The one rule: the per-cast budget

Every cast has a damage budget: **440 x (1 + 0.04 x (INT - 10))** total damage across
every enemy that cast touches. Field damage counts. Damage over the budget is
**discarded** and never carries into the next cast.

Why it matters: a six-man group does not take six times the damage of one man. It
takes the budget, spread thin. At L10 the budget is 757 damage, about four
Shatterstones. A fifth enemy in the same circle takes nothing. A big group is not a
bigger kill.

| source | budget |
|---|---|
| base, always on | 440 x (1 + 0.04 x (INT - 10)) → 757 at L10 |
| Overload, 10 s | x1.5 → 1135 at L10 |
| Severance only | its own cap, 600 x (1 + 0.04 x (INT - 10)) → 1032 at L10 |

A cast that hits 4 or more enemies refunds 4 mana.

## Abilities

| tier | name | kind | shape | range | cast | cd | mana | damage | key numbers |
|---|---|---|---|---|---|---|---|---|---|
| 0 | Runebolt | projectile | single | 24 | 0.15 | 1.2 | 6 | base 60 (103 at L10) | one target only, the budget never applies to it |
| 0 | Cinder Trail | projectile | line | 16 (2.2 wide) | 0.0 | 3.0 | 10 | base 40 per target (69) | burn 8 damage per second for 6 s, flat |
| 0 | Basalt Skin | buff | self | 0 | 0.0 | 18 | 12 | 0 | 30% less damage for 8 s, does not stack |
| 1 | Shatterstone | aoe | circle | 20 | 0.0 | 8.0 | 24 | base 110 per target (189) | 6 m circle, 0.5 s warning, counts against the budget |
| 1 | Stormlance | aoe | line | 24 (3 wide) | 0.0 | 9.0 | 26 | base 100 per target (172) | max 6 targets, 0.6 s telegraph, 0.5 s stagger |
| 1 | Slag Pool | debuff | circle | 18 | 0.0 | 14.0 | 18 | base 10 tick (17) | 5 m field for 8 s, 12 damage per second flat, slow 25%, 80 damage per target max |
| 2 | Frostbloom | aoe | cone | 16 | 0.0 | 12.0 | 30 | base 120 per target (206) | 60 degree cone, slow 40% for 3 s |
| 2 | Rune Conduit | utility | self | 0 | 0.0 | 0 | 0 | 0 | passive: the budget, 440 scaled on INT |
| 3 | Overload | buff | self | 0 | 0.0 | 45 | 0 | 0 | budget x1.5 and +15% move speed for 10 s |
| 4 | Severance | aoe | circle | 24 | 0.0 | 30 | 40 | base 180 per target (310) | 8 m circle, own hard cap 1032 at L10, 1.2 s warning |

Castable on the move: every nuke has cast 0.0. The Rune walks, aims and paints. It
never channels.

## Effect vocabulary

Effects are objects: `type`, `value`, `duration` (seconds, 0 = instant).

| type | value means | duration means |
|---|---|---|
| burn | damage per second, flat | how long it burns |
| ground_field | damage per second in the field, flat | how long the field lasts |
| slow | percent slower | how long |
| stagger | seconds of stagger | 0 |
| damage_reduction | percent less damage taken | how long |
| move_speed | percent faster | how long |
| group_damage_cap | flat part of the budget, 440 | 0 |
| cap_multiplier | multiplier on the budget, 1.5 | how long |

## HUD contract

- Your own frame shows the live budget: **757 damage left** at L10, **1135** while
  Overload runs. It empties as a cast lands and refills on the next cast.
- A warning circle appears 0.5 s before Shatterstone and Stormlance, 0.8 s before
  Slag Pool, 1.2 s before Severance. Enemies can walk out. That is the counterplay.
- Mana bar, and the cooldown ring on each skill.

## What it loses to

| enemy | why it hurts |
|---|---|
| warden_fighter, warden_warrior, warden_rogue | they close 20 m faster than the Rune walks away. 1.7 standard hits (120 damage each) kill a 500 health, 0% mitigation Rune |
| a lone melee attacker | every skill the Rune owns is aimed at a group. One rogue picks it apart |
| its own warning circles | standing 3 m off the circle wastes the whole cast. The budget does not care how many enemies are in it |

The Rune has no self-sustain beyond Basalt Skin and no escape tool. A mistake is a
corpse.

## What it counters

| enemy | why it works |
|---|---|
| warden_defender | the shield blocks a frontal hit, it does not block a 6 m circle in the ground |
| warden_priest | the buffer stands behind the line, in the circle |
| ashkin_templar | the same: a frontal party arc cannot cover a circle |

Counter window for them: the 0.5 s warning circle, then walk out. A group that reads
the circle and steps aside pays nothing.

## Numbers to watch

- Budget 440 scaled on INT is the load-bearing number of the class. If the Rune feels
  weak, raise the flat part before touching the per-target damage: the budget is the
  cap that decides everything.
- A Shatterstone at L10 is 189 damage, 19% of a 1000 health fighter. Four of them
  land 756 damage, three quarters of one fighter, but the budget only pays for four
  across the whole cast. A group walks into the circle and the Rune's damage is
  spent as fast as it lands.
- 0 CON growth, 500 health and 0% mitigation are the weakness on purpose. Keep them.
