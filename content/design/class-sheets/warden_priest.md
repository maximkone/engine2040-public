# Priest (warden_priest) — class sheet

Tier 1, branch of the Acolyte. Human faction. Clean-room names.

## Role

Buffer and mana donor. The priest does not heal a wound, it keeps the
group supplied: health, mana and speed trickle into a circle around it
while the party fights inside that circle. The priest is the reason a
party does not drink flasks, and it is the first thing a focused enemy
walks to and kills.

## Stat priority

1. **WIS** (+1.5 per level) — every tick of regen, every buff, every
   threshold.
2. **CON** (+1.5 per level) — the priest has to live through the moment
   the assassin arrives. It is squishier than the cleric and it knows it.
3. Everything else. STR, DEX and INT do not grow.

**Scaling rule** (same for every ability in this sheet): `damage.base` is
the value at 0 points of the scale stat. Each WIS point adds **+0.5 HP**
for regen, shield and healing, **+0.4 damage** for damage skills. Mana
pool is 300 at level 1 and +20 per level; mana regenerates 6 per second
out of combat and 2 per second in combat.

## Key abilities

| Ability | Tier | Numbers |
|---|---|---|
| Quiet Rest | 0 | Self, no mana. 60 HP (+0.5/WIS) over 6 s while standing still and taking no damage. CD 20 s. |
| Cheer Song | 0 | 6 m aura, +12% move speed, +30% stamina regen, 15 s. CD 30 s, 8 mana. |
| **Psalm of the Open Hand** | 1 | **10 m circle anchored on the priest, 20 s. Every 1.0 s: 12 HP (+0.5/WIS) and 6 MP to every ally inside. Re-cast refreshes it (CD 12 s from the start, 25 mana a cycle). The priest deals half damage while it runs. It ends when the priest dies or leaves the circle.** |
| Well of Quiet | 1 | The priest pays 40 mana; allies within 8 m gain 120 MP over 4 s. The priest gains none. CD 30 s. |
| Hymn of the Bulwark | 1 | 8 m circle, -20% damage taken for 8 s, priest and allies. CD 40 s, 30 mana. |
| Verse of the Steadfast Watch | 2 | 10 m circle, +20% attack speed, +10% cast speed for 10 s. CD 45 s, 30 mana. |
| Chastisement | 2 | One enemy within 24 m: 70 (+0.4/WIS) damage, 30% slow for 3 s. CD 8 s, 10 mana. |
| Hymn of Return | 3 | Revival. 6 s channel at the ally's tomb marker (within 30 m). Returns 25% HP, 50% MP, one debuff stripped; costs them 25% of the XP for their next level. CD 420 s, 80 mana. |
| Unbroken Choir | 3 | 8 m circle: strips two debuffs from every ally (one each, the second only if the first lands). CD 60 s, 35 mana. |
| Chorus of the Vanguard | 4 | 12 m circle for 12 s: +20% damage dealt and 8 HP (+0.5/WIS) per second for everyone inside. CD 180 s, 100 mana. |

Shape vocabulary: `self` = caster only, `single` = one chosen ally,
`target` = one chosen enemy, `circle` = ground area.

## The buff and flask economy

- **The psalm replaces the flask.** 12 HP a second, 20 s at a time, to
  everyone inside 10 m, for 25 mana a cycle. A priest group carries no
  flasks: the party regenerates while the fight runs, so nobody breaks a
  cast order to drink and the flask slot is free for something else. (The
  flask's own numbers belong to the item table; this sheet does not set
  them.)
- **The cost of the sustain is paid in damage.** While the psalm runs the
  priest deals half damage and cannot leave the circle without dropping
  it. The priest that stays back to hold the group stops being a threat
  at all. That is the trade the class is built on.
- **Mana is the second supply line.** Well of Quiet pays 40 mana and
  hands 120 MP to the party; the priest nets 80 MP across the cast. One
  priest feeds two casters, which is the whole reason a priest is in the
  group when a mage could heal.
- **No free full heals.** Nothing in this kit returns a whole bar at
  once. The regen is a trickle, the cleanse gives no health, the revival
  returns 25% HP and charges the rescued player 25% of their next-level
  XP on top of a 420 s cooldown and 80 mana. A fallen warden leaves a
  visible tomb marker where it died, showing name and time of death; the
  marker stands for the whole war window and the revival is the only way
  to use it.
- **The bulwark is a swing answer, not a state answer.** -20% damage
  taken for 8 s stops the one big hit. Damage over time still lands, so
  the psalm and the bulwark are needed together.

## What it loses to

- **One assassin who reaches the priest.** 70 damage a cast, half while
  the psalm runs, no gap closer, and every support cast has a 0.6 to 1.0 s
  open. The group does not have a sustain problem; it has a priest problem.
- **A spreading party.** The psalm, the bulwark, the verse and the chorus
  are all 8 to 12 m circles. If the fight splits the group, the priest
  has nothing to hold.
- **Sustained chip damage** with no big hit to time the bulwark against.

## What it counters

- **warden_sorcerer** — a nuker empties itself; the psalm keeps the
  party standing through the whole rotation.
- **ashkin_gunner** — artillery chip damage is exactly what a steady
  12 HP a second answers.
- **ashkin_attacker** — drain bruisers lose to sustained supply.

## Loadout

Censer staff (one hand, ranged), psalm book.
