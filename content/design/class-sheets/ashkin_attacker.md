# Ashkin Attacker — drain bruiser

Tier 1 Ashkin branch of `ashkin_combatant`. Trades with the strongest thing
on the field and eats it. Nothing heals without a landed hit.

## Role

Drain bruiser. The Attacker is a melee health economy: hit the thing that
is trying to kill you, and that thing pays for your healing. The rule that
makes the class work: **healing comes from a hit that landed, never from a
timer, and the enemy sees the link the whole time.**

Three jobs.

1. Find the biggest target and attach the link to it.
2. Move, because standing still in melee means losing the link and the drain.
3. Break the bind when the Templar grabs you: knockback is the escape.

## Stat priority

| stat | growth per level | why |
|---|---|---|
| STR | 2 | STR 2 per level is the highest growth in the branch: the drain scales hard |
| CON | 1 | the Attacker eats damage while it trades, so CON is the secondary |
| DEX | 0 | the gap closer is a dash, not a speed stat |
| INT | 0 | no casting |
| WIS | 0 | the healing is stolen, not cast |

Primary order: **STR, CON**.

## Numbers (house rules)

- 60 Hz tick. 1 tick = 16.7 ms.
- `damage.base` is the level-1 raw number in the contested middle, no gear.
- Every point of the scaling stat above 10 adds **3% of base**.
- Full readable telegraph floor is **340 ms** (`crates/combat/tests/it/telegraph_floor.rs`).
- Range, radius in meters. Cast, cooldown in seconds.

## Loadout

Weapon: ashkin paired blades. Offhand: none.
Silhouette: hunched, wide, blades low. Two hands for damage, no shield, so
every point of CON is a point of staying alive.

## Key abilities

| ability | tier | kind | what it does | numbers |
|---|---|---|---|---|
| Rend | 0 | melee | stacking burn opener | 55 dmg (STR), 8 dmg/s for 5.0 s, 3 stacks, cd 5.0 s |
| Reaping Arc | 0 | melee cone | small self-heal on a group | 50 dmg (STR), 110° arc, 2.8 m, heals 25% of damage dealt, 0.35 s wind-up, cd 6.0 s |
| Siphon | 1 | melee | **the drain** | 70 dmg (STR), heals 35% of damage dealt, link line shown to both teams, 0 on a blocked hit, breaks at 12.0 m or 6.0 s, heal cap 90/s, cd 3.0 s |
| Cinder Burst | 1 | aoe circle | **the escape from a bind** | 70 dmg (STR), 3.5 m circle at 5.0 m, cast 0.35 s, knockback 4.0 m, breaks every bind and tether, cancels a charging enemy, cd 14.0 s, 16 mana |
| Battle Cry | 1 | buff circle | **the positional aura** | 8.0 m radius, 12.0 s, +25% attack speed, +10% movement speed, +15% damage taken, edge always visible, cd 25.0 s, 20 mana |
| Scorch Line | 2 | aoe line | cut the fight in half | 95 dmg (STR), 10.0 m line 1.2 m wide, 18 dmg/s for 4.0 s, -30% speed while burning, cast 0.4 s, cd 16.0 s, 22 mana |
| Feast | 2 | heal | cash in the link | consumes the Siphon link, heals 12% max health, clears Rend stacks, cd 20.0 s |
| Red March | 3 | aoe circle | self-heal finisher | 110 dmg (STR), 6.0 m circle, cast 0.5 s, heals 40% of damage dealt capped at 220, pulls targets 2.0 m, a Siphon within 3.0 s heals double, cd 28.0 s, 26 mana |
| Blood Pact | 3 | debuff | mark the target | +20% damage taken from the Attacker, -20% damage dealt to the Attacker, 10.0 s, cast 0.3 s, cd 24.0 s, 18 mana, no stacking |
| Wolf Rush | 3 | melee line | gap closer that feeds the drain | 80 dmg (STR), dash 10.0 m, stagger 0.8 s, ends with a free Siphon, 0.4 s charge tell, cd 16.0 s, 12 mana |
| Crimson Thirst | 4 | buff | the win condition | 15.0 s, every landed hit heals 45% of its damage, each heal adds 1.0 s up to 6.0 s more, cannot be healed above 80% max health, the Siphon link does not break on range, cd 90.0 s, 40 mana |

### Siphon, in words

70 damage on a 2.4 m reach strike, cd 3.0 s. The healing is **35% of the
damage actually dealt** — 24.5 health on a clean hit. A hit that is blocked,
parried or fully absorbed by a shield heals **0**, and the link flashes for
0.5 s so the enemy knows it wasted the swing.

The link is a visible red tether between the two of you, shown to both
teams. It breaks at 12.0 m or after 6.0 s, and healing from it is capped
at 90 health per second. Crimson Thirst lifts that cap and stops the range
break.

That visible link is the whole fix. The old drain healed on a timer and
read as passive regeneration. This one reads as a leash, and the enemy can
see exactly when it snaps.

### Cinder Burst, in words

The answer to being bound. A 3.5 m ground burst at 5.0 m, 70 damage, and
**4.0 m of knockback**. Any knockback breaks any bind, so the burst cuts
Iron Tether, cuts the Gunner's anchor pull, and cuts a charging enemy at the
same time. The knockback is also the answer to the Templar closing 8.0 m
with Crushing Advance.

### Battle Cry, in words

Not a song. An **8.0 m radius aura on the ground that follows the
Attacker** for 12.0 s: +25% attack speed and +10% movement speed inside,
+15% damage taken inside. The edge is always drawn, so the enemy knows
where the safe ground is not.

Position matters: the aura is worth most when the enemy has to stand in it
to reach the Attacker, and worth nothing if the Attacker walks it out of
the fight.

## What it loses to

- **Control.** The Attacker has one knockback and no real crowd control.
  **Purge of Bonds** (Templar, 6.0 s cooldown) clears the drain links in one
  cast and the class has no answer left. That is the direct counter.
- **Range.** The link breaks at 12.0 m. A kiter that also slows keeps the
  Attacker at 0 healing and 0 damage.
- **Grouped burst.** Reaping Arc heals 25% of the damage dealt, so the
  Attacker scales with what it hits. A pack that deletes it in one burst
  ends the trade before the drain pays.
- **The 80% heal cap.** Crimson Thirst cannot take the Attacker over 80%
  health, so a burst that lands at the cap wins outright.

## What it counters

- **warden_acolyte** — the drain out-heals a slow first aid kit. 24.5 per
  3.0 s cooldown beats a tick heal, and Feast converts a full link into
  12% max health on the spot.
- **warden_cleric** — a chanting tick heal runs below the drain rate. The
  Attacker wins the trade while the cleric is still behind.
- **warden_sorcerer** — 70 damage per 3.0 s with a heal attached removes a
  caster that cannot take a trade. Scorch Line also denies the cast ground.
