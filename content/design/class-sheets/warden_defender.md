# Warden Defender — class sheet

**Role:** shield tank · wardens · tier 1 (branch `warden_fighter`)
**Growth:** +1.5 STR, +1.5 CON per level · **Primary:** STR, CON
**Loadout:** arming sword + heater shield
**2026 fix (roster):** a visible share meter, so the damage redirect reads as value.

## Stat priority

1. **STR** — every strike in the strike line scales on STR. Nothing else is close.
2. **CON** — health, mitigation, and the Second Stand heal.
3. **DEX** — shield turn speed only. Never above STR or CON.

## Numbers model

- **Damage:** `D = base x (1 + 0.04 x (S - 10))`, `S = 10 + growth x (level - 1)`.
  STR: L1 = 10 (x1.00) · L10 = 23.5 (x1.54) · L20 = 38.5 (x2.14) · L40 = 68.5 (x3.34).
- **Health:** `80 + 10 x CON`. **Mitigation:** 1% per CON point, capped at 35%.
  At L10: 315 health, 23% mitigation.
- **Share meter:** 0 to 100 points, shown under the health bar. It fills from
  redirected ally damage, decays 3 points/s while no share aura is up, and every
  25 points gives the caster -2% damage taken (max -8%).
- **Fields:** `range` = reach from the Defender, `radius` = size of the area,
  `cast` = wind-up seconds, `cooldown` = seconds, `mana` = points,
  `telegraph` = the decal that warns the effect.

## The five that matter

| Ability | Tier | Numbers |
|---|---|---|
| **Weight of the Line** | 0 | Aura r 8 m, 8 s, cd 30 s, 12 mana. Allies in the aura take **40% less damage**; **60% of what they avoid is redirected to you**; share meter **+1 per 3 damage redirected**. |
| **Bulwark Stance** | 0 | Self, 6 s, cd 12 s. Damage taken **-25%**, share fill **x1.5**, move speed 60%. |
| **Cradle Sweep** | 1 | Cone 120 deg, r 3.0 m, 0.5 s telegraph, cd 10 s. Base **34** (L10: 52). **Slow 40% for 3 s**. |
| **Shield Slam** | 2 | Circle r 3.5 m, 0.8 s telegraph, cd 18 s. Base **58** (L10: 89). Slow 50% / 4 s, stagger 1.0 s, knockback 1.5 m. |
| **Bulwark Charge** | 2 | Circle r 4.0 m, 0.6 s telegraph, cd 20 s. Base **84** (L10: 129). **Costs 100 share points**; allies within 6 m gain **+20% damage for 6 s**. |

Top of the lines: **Iron Tide** (T3, circle r 5.0, base 96, slow 60% / 4 s,
knockback 3 m, cd 26 s), **Hold Fast** (T4, circle r 6.0, base 132, slow 70% /
5 s, knockback 4 m, cd 60 s) and **Aegis Rite** (T4, aura r 12, **65% less
damage**, one meter point per damage redirected, meter cap 150, cd 90 s).

Also live: **Shield Ram** (T0, single, reach 2.2 m, base 24, slow 30% / 2 s,
cd 8 s), **Oath of the Line** (T1, aura r 10 m, **50% less damage**, one meter
point per 2 damage redirected, cd 45 s) and **Second Stand** (T3, heal 130 on the
CON scale, +60 meter points, damage taken -60% for 3 s, cd 45 s).

## How it plays

1. **Open the line.** Bulwark Stance, then plant inside the party's back line.
2. **Plant the shield.** Weight of the Line is the whole class: allies drop to
   40% less damage and the avoided damage comes to you, where it fills a bar you
   can both see and spend.
3. **Read the bar.** 25 points = -2% damage taken. The bar is a damage race you
   win by standing closer, not by standing tougher.
4. **Spend it.** 100 points unlocks Bulwark Charge. One hit, 84 base, and the
   party gets +20% damage for 6 s. That is the payoff the whole loop exists for.
5. **Hold the ground.** Cradle Sweep and Shield Slam do the interrupting: slow
   first, then the stagger, then the slam. Slowed enemies cannot answer.

## What it loses to

- **ashkin_gunner, ashkin_rune, warden_sorcerer** — the share aura is 8 m at
  tier 0 and 12 m at tier 4, and only damage taken by allies inside it fills
  the meter. Anything shot from outside never touches the meter.
- **warden_cleric, ashkin_life** — sustain beats a tank that trades HP for
  meter points. You have 315 health at L10; a chant aura out-heals that.
- **Two enemies at once** — one aura, one meter, one redirect. The tank cannot
  be everywhere, and Bulwark Charge is a single circle.

## What it counters

- **warden_assassin, warden_rogue, ashkin_shadow** — the opener lands into a
  40% reduction and a 40% slow for 3 s. The stealth burst becomes a health
  bar question.
- **warden_warrior, ashkin_combatant, ashkin_attacker** — melee pressure dies
  in the cone: slowed, staggered, then thrown back 3 m by Iron Tide.
- **ashkin_templar** — a second wall facing a wall. Whoever plants first wins
  the meter, and the meter is the weapon.
