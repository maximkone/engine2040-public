# Ashkin Shadow — class sheet

A stealth assassin that fights on a detection meter, not on a health bar. The whole kit pushes watchers down the ladder, spends one opener, then survives the second it gets for it.

**Role:** stealth assassin · ashkin · tier 1 (branch `ashkin_officiator`)
**Growth:** +2 DEX, +1 STR per level · **Primary:** DEX, STR
**Loadout:** twin ash knives · ash cord
**2026 fix (roster):** one second of reveal after the opener.

Data file: `content/classes/ashkin_shadow.json`.

## House numbers

These are the numbers this sheet is written in. They are design numbers, not measured ones.

- 60 Hz tick. Range and radius in metres, cast and cooldown in seconds, mana in points.
- **Damage:** `D = base x (1 + 0.04 x (S - 10))`, `S = 10 + growth x (level - 1)`.
  `damage.base` in the class file is the level-1 raw number. DEX: L1 = 10 (x1.00) ·
  L10 = 28 (x1.72) · L20 = 48 (x2.52) · L40 = 88 (x4.12).
- **Health:** `200 + 25 x CON`. **Mitigation:** 1% per CON point above 10, capped 35%.
  The Shadow has 0 CON growth, so it is 500 health and 0% mitigation at every level.
- **A standard weapon hit** (gladius) is 120 damage, 12% of a 1000 health fighter.
  4.2 hits kill the Shadow.
- **Telegraph floor:** 340 ms is the readable minimum
  (`crates/combat/tests/it/telegraph_floor.rs`). The cones are 0.4 s and 0.5 s.

The reference build below is level 10 with no extra points in DEX: DEX 28, x1.72.

## Role

Stealth assassin. The Shadow has no ranged tool. If it cannot reach a target, it
waits. Its damage is one hit and one opener; the rest of the kit is about staying
unseen.

## Stat priority

1. **DEX** — +2 per level. Every damage number, the reach and the attack speed.
2. **STR** — +1 per level. The backup, small amounts on the blade only.
3. **CON** — nothing. 500 health, 0% mitigation, at every level. That is the weakness.
4. **INT, WIS** — never taken.

What DEX buys at the reference build (DEX 28, x1.72):

| number | base | at L10 |
|---|---|---|
| Quickblade | 50 | 86 |
| Cull from unseen | 80 x 3.5 | 138 x 3.5 = 481 |
| Fan of Blades per enemy | 45 | 77 (116 from unseen) |
| Nightfall Arc per enemy | 90 | 155 |
| Waning Dark per enemy | 100 | 172 |

Mana: 60 at level 1, +4 per level. No Shadow skill costs mana.

## The detection ladder

Every watcher who can see the Shadow runs a 0-100 detection meter, shown in four steps:

| detection | step | what the watcher does |
|---|---|---|
| 0-24 | unaware | nothing |
| 25-49 | curious | turns toward the Shadow |
| 50-74 | suspicious | walks to the last known spot |
| 75-99 | searching | sweeps the area |
| 100 | spotted | the Shadow is revealed |

Detection gain per second, per watcher:

| state | gain |
|---|---|
| Shadow unseen | 0 |
| visible, standing still | 4 |
| visible, walking | 10 |
| visible, sprinting | 22 |
| visible within 6 m | +30 on top |
| target carries Night Brand | x2 |

The Shadow pushes this meter down: Veil Step makes it unseen, Shroud Pulse drops one
step (25 points) and halves the gain for 6 s, Black Escape drops 25 points and hides,
Waning Dark drops one step for everyone it hits. Night Brand pushes it up: a marked
target gains twice as fast.

One step = 25 detection points. That is the whole conversion between the ladder and
the numbers.

## Abilities

| tier | name | kind | shape | range | cast | cd | damage | key numbers |
|---|---|---|---|---|---|---|---|---|
| 0 | Veil Step | stealth | self | 0 | 0.0 | 12 | 0 | 3 s unseen, watchers gain no detection. Breaks on attack or sprint |
| 0 | Quickblade | melee | single | 2.2 | 0.2 | 1.0 | base 50 (86 at L10) | the basic blade, 2.2 m reach |
| 0 | Windstep | buff | self | 0 | 0.0 | 25 | 0 | +35% move speed, +20% attack speed, -25% detection gain, for 6 s. Sprinting cancels it |
| 1 | Cull | melee | single | 2.4 | 0.0 | 14 | base 80 x 3.5 from unseen (481 at L10) | ignores 50% armour, capped at 60% of a player max health (600), then a 1.0 s reveal |
| 1 | Fan of Blades | aoe | cone | 3.2 | 0.2 | 16 | base 45 per enemy (77) | 90 degree cone, 1.5x unseen, 0.4 s telegraph, 60% cap per player hit |
| 1 | Shroud Pulse | aoe | circle | 12 | 0.0 | 18 | 0 | 5 m cloud, drops one detection step, halves detection gain for 6 s. Does no damage |
| 2 | Night Brand | debuff | target | 14 | 0.3 | 18 | 0 | mark for 10 s: +20% damage to it, its detection gain on the Shadow x2 |
| 2 | Black Escape | stealth | self | 0 | 0.0 | 20 | 0 | 2.5 s unseen, -25 detection points, next melee hit within 4 s is an opener again (the 1.0 s reveal still applies) |
| 3 | Nightfall Arc | aoe | cone | 4.0 | 0.3 | 24 | base 90 per enemy (155) | 120 degree cone, 1.5 s silence, 60% cap, 0.5 s telegraph |
| 4 | Waning Dark | aoe | circle | 10 | 0.0 | 45 | base 100 per enemy (172) | 8 m circle, drops one detection step for everyone hit, next melee hit within 6 s is an opener, 60% cap |

## The opener, the reveal and the counterplay window

Cull is the only hit that ends a fight, and it is deliberately not enough to end one.

1. From unseen, the opener hits for 3.5x and ignores half the armour: 481 at L10.
2. The whole hit is **capped at 60% of the target max health** against a player, 600
   damage at 1000 health. The cap bites from L20 on, where the raw opener is 705.
   Against an NPC the cap is 120%, so an opener can still delete an NPC.
3. After it lands, the Shadow is **revealed for 1.0 s** to every watcher. Their
   detection jumps to spotted. The reveal does not end early by itself.
4. **Counterplay window:** during that 1.0 s the target takes 50% less damage, and its
   next melee hit stuns the Shadow for 0.6 s. A target that reacts beats the opener.
5. The Shadow's stealth is broken by the attack. Veil Step (12 s) or Black Escape
   (20 s, and one re-opener) is the way back in.

The 60% cap is the fix that keeps the Shadow honest: the opener starts a fight, it
does not end one.

## Effect vocabulary

Effects are objects: `type`, `value`, `duration` (seconds, 0 = instant).

| type | value means | duration means |
|---|---|---|
| stealth | seconds unseen | how long it lasts |
| stealth_multiplier | damage multiplier while unseen | 0 |
| armour_ignore | percent of armour ignored | 0 |
| damage_cap_vs_player | percent of the target max health, one hit | 0 |
| reveal | seconds of forced reveal | how long the reveal lasts |
| counter_window | percent less damage taken by the target | how long the window is |
| detection_step | steps of the ladder, 1 step = 25 points | how long it lasts |
| detection_point | detection points removed | how long it lasts |
| detection_gain | percent change to the gain rate, negative is quieter | how long it lasts |
| move_speed | percent faster | how long |
| attack_speed | percent faster | how long |
| mark | seconds of the mark | how long the mark lasts |
| damage_boost | percent more damage dealt | how long |
| silence | seconds of silence | how long |
| reopener | seconds in which the next melee hit counts as an opener | how long |

## HUD contract

- Every enemy near the Shadow shows their detection meter as four pips: curious,
  suspicious, searching, spotted. The Shadow sees the same four pips on their own
  frame, mirrored. That is the whole read of the class.
- After an opener, the target shows **REVEALED 1.0 s** and their frame flashes the
  counter window.
- The Shadow own frame shows the damage cap reminder next to Cull: **MAX 600** against
  a player.
- Night Brand shows as a mark icon on the target with the remaining seconds.

## What it loses to

| enemy | why it hurts |
|---|---|
| warden_rogue | a 20 m reveal pulse on a 20 s cooldown finds every hidden enemy inside it. A Shadow picked early is a slow swordsman |
| ashkin_gunner | the dig pulse reveals hidden enemies to the whole Ashkin faction for 12 s and cancels stealth effects on the ground inside it, so Veil Step inside the pulse does nothing. Nobody gets to be safe across the war |
| warden_defender | the shield eats the opener from the front, which is the side the Shadow has to attack |
| warden_fighter | 500 health, 0% mitigation and 1.7 standard hits (120 damage each), with no escape tool beyond Black Escape |
| anyone who marks | Night Brand is a two-way street. A marked Shadow is seen twice as fast |

Once revealed for 1 s there is no second opening, only the counter that lands inside
that second.

## What it counters

| enemy | why it works |
|---|---|
| warden_cleric | a healer stands still and stays behind the line. One opener puts the party on the back foot |
| warden_priest | the buffer never carries a weapon and never sees the Shadow coming |
| warden_mage | a caster has no escape and no armour; the 60% cap means the first hit starts the trade |

Counter window for them: the target gets 50% damage reduction and a stun counter for
1.0 s after the opener. A good target always takes it. The Shadow has to expect the
counter, not plan the kill.

## Numbers to watch

- The 3.5x opener multiplier and the 60% cap are one dial. At L10 the opener is 481
  damage, under the 600 cap. At L20 it is 705 and the cap bites, so from L20 the
  opener is a flat 60% chunk. If the Shadow one-shots players, cut the cap to 50%,
  not the multiplier.
- 1.0 s reveal and the 50% counter window are the fairness of the class. Keep both.
- Detection gain of 4 per second standing still means a Shadow that stops in the open
  is seen in 25 s. Walking is 10 per second, sprinting 22 per second. Those three
  numbers are the whole stealth economy.
- 0 CON growth, 500 health and 0% mitigation are the weakness on purpose. Keep them.
