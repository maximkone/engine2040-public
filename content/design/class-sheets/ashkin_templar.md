# Ashkin Templar — peel tank

Tier 1 Ashkin branch of `ashkin_combatant`. A wall that drags enemies into
reach, then keeps the party alive while it does it.

## Role

Peel tank. The Templar does not win the damage race: it wins by making the
enemy fight on the Templar's ground. Three jobs, in order.

1. Pull one dangerous enemy into sword range and hold it there.
2. Keep the party alive with a shield that faces the archer.
3. Break the one control effect that would end the fight.

## Stat priority

| stat | growth per level | why |
|---|---|---|
| STR | 1.5 | every offensive number scales on STR |
| CON | 1.5 | the share, the shield pools and the damage taken all sit on CON |
| DEX | 0 | the Templar never needs to be first |
| INT | 0 | no casting |
| WIS | 0 | no healing done here |

Primary order: **STR, CON**.

## Numbers (house rules)

- 60 Hz tick. 1 tick = 16.7 ms.
- `damage.base` is the level-1 raw number in the contested middle, no gear.
- Every point of the scaling stat above 10 adds **3% of base**.
- Full readable telegraph floor is **340 ms** (`crates/combat/tests/it/telegraph_floor.rs`).
- Range, radius in meters. Cast, cooldown in seconds.

## Loadout

Weapon: ashkin longsword. Offhand: tower shield.
Silhouette: upright, wide, shield on one side. It reads as a wall at 30 m.

## Key abilities

| ability | tier | kind | what it does | numbers |
|---|---|---|---|---|
| Shield Bash | 0 | melee | stagger + push, opens a peel | 40 dmg (STR), stagger 0.8 s, push 1.5 m, cd 5.0 s |
| Tower Sweep | 0 | melee cone | hits a whole group in front | 55 dmg (STR), 120° arc, 2.6 m, 0.35 s wind-up, cd 7.0 s |
| Brace | 0 | buff | trade mobility for tank | 25% damage reduction, movement -30%, 4.0 s, cd 18.0 s, 10 mana |
| Iron Tether | 1 | control | **the real peel** | reach 9.0 m, cast 0.45 s, root 1.0 s, forced walk 3.0 m/s for 3.0 s, bound 2.5 s on arrival, cd 14.0 s, 18 mana |
| Bulwark Arc | 1 | buff | **party shield** | 120° frontal cone, 5.0 m, 6.0 s, absorbs 120 per hit, Templar eats 40% of it, cd 20.0 s, 25 mana |
| Purge of Bonds | 1 | utility | **anti-lock cleanse** | clears 1 debuff on self and 1 on each ally in 4.0 m, no DoT removal, cd 6.0 s, 12 mana |
| Phalanx Call | 2 | buff | group tanking | 6.0 m circle, 25% damage reduction for allies, 5.0 s, allies move -15%, cd 25.0 s, 22 mana |
| Crushing Advance | 3 | melee | close 8.0 m and hold | 85 dmg (STR), dash 8.0 m, root 1.2 s, stagger 1.0 s, knockback 2.0 m, 0.5 s charge tell, cd 12.0 s, 15 mana |
| Ash Bulwark | 3 | buff | the big shield | 300 damage absorbed over 8.0 s, blocks one control, reflects 20% of blocked melee, cd 45.0 s, 30 mana |
| Judgement of Iron | 3 | aoe circle | punish a standing aura | 120 dmg (STR), 4.0 m circle at 6.0 m, cast 0.6 s, stagger 1.5 s, knockback 3.0 m, breaks binds, cd 22.0 s, 28 mana |
| Hold the Line | 4 | utility | the anchor | 12.0 s immune to forced movement, allies in 3.0 m move +20%, cd 90.0 s, 40 mana |

### Iron Tether, in words

The old snare pinned a target where it stood. This one drags it to the
Templar: root 1.0 s, then the target walks 3.0 m/s toward the shield for
3.0 s, and if it arrives it is bound 2.5 s (rooted, slowed 40%). The line is
drawn on the ground the whole time, so the enemy sees it coming.

**Any knockback escapes it.** Cinder Burst (Attacker, 4.0 m push) and Cluster
Shell (Gunner, 2.0 m push) both break a bind on hit, and so does a dash.
That is the whole counterplay: pull it back out.

### Bulwark Arc, in words

A 120° cone on the ground in front of the Templar, 5.0 m deep, 6.0 s long.
It eats projectiles from the front (120 damage per hit) and the Templar
takes 40% of what it eats. Melee passes through it: this is the archer
answer, not the melee answer. It faces one way, so turn to face the bow.

### Purge of Bonds, in words

This replaces the old cleanse. It is not a buff and not a heal:
it removes one debuff from the Templar and one from every ally inside 4.0 m.
Roots, stuns, silences, chills. Damage over time stays. The 6.0 s cooldown
is the point: it beats one stunlock cycle, not a whole class.

## What it loses to

- **A clean stealth opener.** A 4x assassin strike lands before the arc is
  up. The Templar has no answer to a dive that starts on top of it.
- **Range.** Iron Tether reaches 9.0 m and Bulwark Arc needs the enemy in
  front of the shield. A ranged class that never walks in wins the trade.
- **Momentum fights.** The Templar has 2 damaging abilities and a charge.
  A class that can leave, relight and come back with a cooldown wins on
  uptime, not on a single exchange.
- **Its own cooldowns.** 14.0 s on Iron Tether, 20.0 s on Bulwark Arc and
  22.0 s on Judgement of Iron: two of those together leave 6 s of the fight
  with nothing but a shield bash.

## What it counters

- **warden_warrior** — the dual-wield combo needs free movement. One Iron
  Tether at 3.0 m/s and the combo never starts; the bound target is on the
  wrong side of the shield.
- **warden_archer** — Bulwark Arc is the answer to arrows. The archer has to
  step inside 5.0 m to shoot what it wants to hit.
- **warden_sorcerer** — ground casts are slow and telegraphed. Phalanx Call
  plus Judgement of Iron punish a caster that has to stand in one place.
