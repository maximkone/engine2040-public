# Cleric (warden_cleric) — class sheet

Tier 1, branch of the Acolyte. Human faction. Clean-room names.

## Role

Healer. The party lives or dies on the chant, not on a button press. The
cleric is a sustain class with one cleanse attached: a ticking aura that
heals everyone standing in it and strips one debuff every few seconds.
There is no instant full heal in this kit. Numbers come back over time,
in a circle, and the cleric has to be standing in it.

## Stat priority

1. **WIS** (+2 per level) — every tick, every shield, every revive threshold.
2. **CON** (+1 per level) — the cleric must survive being the closest thing
   to the front rank.
3. Everything else. STR, DEX and INT do not grow. The cleric never wins a
   damage race and is built not to.

**Scaling rule** (same for every ability in this sheet): `damage.base` is
the value at 0 points of the scale stat. Each WIS point adds **+0.5 HP**
for healing, shield and regen, **+0.4 damage** for damage skills. Mana pool
is 300 at level 1 and +20 per level; mana regenerates 6 per second out of
combat and 2 per second in combat.

## Key abilities

| Ability | Tier | Numbers |
|---|---|---|
| Field Dressing | 0 | 1.2 s channel, 8 m, one ally. 90 HP (+0.5/WIS) over 3.0 s. CD 6 s, 8 mana. Stops at missing HP. |
| Steady Arms | 0 | 6 m aura, +12% attack speed, +25% stamina regen, 12 s. CD 25 s, 6 mana. |
| **Chant of Steady Hands** | 1 | **8 m circle, 12 s, tick every 1.0 s, 8 HP (+0.5/WIS) per ally. Strips one debuff every 5 s from one ally (worst threat, then lowest HP). CD 14 s, 30 mana to open + 2 per tick.** |
| Oaken Ward | 1 | One ally within 12 m. 220 (+0.5/WIS) absorbed damage, 12 s. CD 25 s, 20 mana. |
| Binding Word | 1 | One enemy within 25 m. 40 (+0.4/WIS) damage, 35% slow for 6 s. CD 12 s, 15 mana. |
| Pilgrim's March | 2 | 10 m circle, +25% move speed, +30% stamina regen, 8 s; ends on damage taken. CD 45 s, 25 mana. |
| Hymn of the Long Watch | 2 | 9 m circle, 10 s, tick 1.0 s, 12 HP (+0.5/WIS), -12% damage taken, one debuff stripped at 3 s and 8 s. CD 40 s, 45 mana. |
| Unbroken Vigil | 3 | Revival. 6 s channel at the ally's tomb marker (within 30 m). Returns 40% HP, no mana, costs them 25% of the XP for their next level. CD 300 s, 60 mana. |
| Sundered Vow | 3 | One downed enemy within 30 m cannot be revived by enemy healers for 40 s. CD 90 s, 25 mana. |
| Hymn of the Last Dawn | 4 | 12 m circle, 15 s, tick every 0.67 s, 10 HP (+0.5/WIS), -10% damage taken, one debuff stripped every 2 s. CD 240 s, 120 mana. |

Shape vocabulary: `single` = one chosen ally, `target` = one chosen enemy,
`circle` = ground area, `self` = caster only.

## The buff and flask economy

- **The flask is dead in a cleric group.** The chant ticks 8 HP per second
  to everyone inside an 8 m circle for 12 s, and the party never needs to
  drink. That is the promise of the class, and it costs the cleric its
  own uptime: the chant has a 1.5 s open, breaks on a single hit, and
  breaks on silence.
- **The cleanse is the real heal.** 8 HP a tick does not save a player who
  is slowed, rooted or silenced. Stripping one debuff every 5 s does. The
  cleric's damage numbers are deliberately tiny so the choice is never
  "heal more or cleanse".
- **No free full heals.** Field Dressing is a 1.2 s channel that pays out
  over 3 s and stops at missing HP. Oaken Ward absorbs damage, it does not
  give health back. Nothing in this kit restores a whole bar.
- **The revival is priced three ways**: 300 s cooldown, 60 mana, and the
  rescued player loses 25% of the XP they need for their next level. A
  fallen warden leaves a visible tomb marker where it died, showing name
  and time of death; the marker stands for the whole war window, and the
  revive is the only thing that can use it. One revive per war window.
  Sundered Vow denies the same button to the enemy.

## What it loses to

- **Burst faster than a tick.** Assassin opener, cannon volley, reaper's
  dash: anything that lands a big hit before 1.5 s of channel wins, because
  the chant breaks and the party has no numbers left.
- **Dispel and silence.** One cast removes the whole kit.
- **Attrition it is not standing in.** 8 m is a short radius in a war.

## What it counters

- **warden_assassin, warden_rogue** — burst damage that the chant simply
  out-lasts, provided the assassin cannot reach the cleric.
- **ashkin_attacker** — drain bruisers win by attrition; the chant plus the
  barrier wins the attrition race.

## Loadout

Hymn mace (one hand, fast startup), oak ward-shield (light, +10 armor).
