# Warden Sorcerer — class sheet

**Role:** elemental nuker · wardens · tier 1 (branch `warden_mage`)
**Growth:** +2 INT, +1 DEX per level · **Primary:** INT
**Loadout:** long silver focus staff, gold ferrules, bone white grip · no offhand
**2026 fix (roster):** ground-targeted casts with a decal matched to the charge bar.

## Stat priority

1. **INT** — +2 per level, the biggest growth in the wardens. Every damage number
   on this sheet scales on INT.
2. **DEX** — +1 per level. It shortens the small casts only (0.2 s to 0.8 s spells).
3. **CON** — nothing. 80 health and no mitigation, at every level. The class buys
   its safety with range and with the blink.

## The three lines

| line | job | shape | tier 0 | tier 2 | tier 4 |
|---|---|---|---|---|---|
| fire | single target damage and burn | line | Ember Lance | Ember Pike | Ember Verdict |
| electric | group damage and chain | circle | Arc Snap | Storm Ring | Thunderclap |
| ice | group control, slow then freeze | circle | Rime Field | Rime Crush | Winter Lock |

Plus two non-element spells: **Spark Step** (the escape) and **Rime Ward** (the
self shield).

## Numbers model

- **Damage:** `D = base x (1 + 0.04 x (S - 10))`, `S = 10 + growth x (level - 1)`.
  INT: L1 = 10 (x1.00) · L10 = 28 (x1.72) · L20 = 48 (x2.52) · L40 = 88 (x4.12).
- **Health:** `200 + 25 x CON` = **200 at every level**. **Mitigation:** 1% per CON
  point, capped at 35%. The Sorcerer has 0% of both.
- **Cast:** a spell with a listed cast of 1.0 s or less gets 2% shorter per DEX point
  above 10, floor 70%. Any cast over 1.0 s is fixed, so a ground charge always takes
  the time the decal promised.
- **Fields:** `range` = reach in metres, `radius` = area radius in metres,
  `cast` = seconds, `cooldown` = seconds, `mana` = points, `telegraph` = the decal.

## The charge (this is the class)

1. Press the spell. A decal appears on the ground at the aim point and **the radius
   of the decal is the radius of the spell**: 4.0 m Arc Snap, 4.5 m Rime Field, 5.5 m
   Storm Ring, 6.0 m Rime Crush, 7.0 m Thunderclap, 7.5 m Winter Lock. The circle on
   the ground is the truth, never the cast range.
2. The charge runs 1.2 s to 2.4 s. The caster stands still. Moving away cancels it.
3. **Damage during the charge cancels the release.** No damage, no mana spent, the
   cooldown starts at half length, and the caster is pushed back 2 m.
4. The last 0.3 s before the release is the **release window**: the spell fires even
   if the caster is hit inside it.
5. The Enchanter's **Bind Mark** is the reliable interrupt: it cancels any charge,
   even inside the release window, and roots the caster for 1 s. The only answer the
   Sorcerer has is **Spark Step** (7 m blink, 9 s cooldown, 12 mana).

## The eleven that matter

| Ability | Tier | Numbers |
|---|---|---|
| **Ember Lance** | 0 | Single, reach 24 m, 0.5 s cast, cd 2.5 s, 8 mana. Base **22** (L10: 38). Burn 20% of the hit per second for 4 s, stacks to 3. No charge: it fires while moving and while hit. |
| **Arc Snap** | 0 | Circle r 4.0 m, cast 1.2 s, cd 8 s, 14 mana. Base **16** (L10: 28). Slow 25% / 2 s, chain to 1 extra enemy within 5 m for half damage. |
| **Rime Field** | 0 | Circle r 4.5 m, cast 1.4 s, cd 10 s, 16 mana. Base **12** (L10: 21). Slow 30% / 3 s. Control, not damage. |
| **Spark Step** | 1 | Self, blink 7 m, 0.2 s cast, cd 9 s, 12 mana. Base **8** (L10: 14) in a 2.5 m burst at the start point. A hit inside the cast cancels the blink. |
| **Ember Pike** | 2 | Single, reach 26 m, line telegraph, cast 1.6 s, cd 9 s, 22 mana. Base **46** (L10: 79). Burn 30% of the hit per second for 5 s, knockback 1.5 m. |
| **Storm Ring** | 2 | Circle r 5.5 m, cast 1.6 s, cd 14 s, 24 mana. Base **30** (L10: 52). Slow 35% / 3 s, chain to 3 extra enemies within 7 m for half damage. |
| **Rime Crush** | 2 | Circle r 6.0 m, cast 1.9 s, cd 18 s, 28 mana. Base **26** (L10: 45). Slow 45% / 4 s, root 1 s. |
| **Rime Ward** | 3 | Self, cast 0.4 s, cd 24 s, 30 mana. Shield **130** on the INT scale (L10: 200) for 8 s, plus 30% weaker ice slows. |
| **Ember Verdict** | 4 | Single, reach 26 m, line telegraph, cast 2.2 s, cd 20 s, 40 mana. Base **78** (L10: 134). Burn 40% of the hit per second for 6 s, target takes 20% more fire damage for 4 s. |
| **Thunderclap** | 4 | Circle r 7.0 m, cast 2.2 s, cd 30 s, 48 mana. Base **55** (L10: 95). Stun 0.8 s, slow 50% / 4 s. |
| **Winter Lock** | 4 | Circle r 7.5 m, cast 2.4 s, cd 36 s, 55 mana. Base **44** (L10: 76). Freeze 2.5 s, slow 60% / 5 s after, frozen targets take 25% more damage for 6 s. |

Mana per tier: tier 0 casts cost 8 to 16, tier 1 costs 12, tier 2 costs 22 to 28,
tier 3 costs 30, tier 4 costs 40 to 55. Cooldown per tier: tier 0 sits between 2.5 s
and 10 s, tier 1 at 9 s, tier 2 between 9 s and 18 s, tier 3 at 24 s, tier 4 between
20 s and 36 s.

## How it plays

1. **Second cast, not first.** Nothing lands on the opening charge unless the enemy
   walks into the decal on their own. Set up the circle and wait.
2. **Fire the leader.** Ember Lance burns and stacks, so one marked target dies
   while the group is held in place.
3. **Let the ice work.** Rime Field and Rime Crush do not kill; they make the front
   line stand still inside Winter Lock while the party hits.
4. **Spend the mana on the exit.** Keep 12 mana for Spark Step. A caster with no
   blink is a caster with 80 health.
5. **Do not stand in the circle.** 2 m of knockback is not much, and the release
   window is only 0.3 s wide.

## What it loses to

- **warden_assassin, ashkin_shadow** — one hit during the charge cancels the release
  and throws the caster back. The second spell never comes out.
- **ashkin_rune** — the rival caster casts on the move, so it never has to stand
  still in a circle while the Sorcerer does.
- **warden_archer** — steady ranged poke. The circles only hit what stands in them
  and the caster has to stand still to make the circle.
- **Its own health** — 80 at every level. Two good weapon hits end the class.

## What it counters

- **warden_fighter, warden_warrior, warden_assassin, ashkin_shadow** — every one of
  them has to walk into a decal to hurt the caster. The ice line roots them there.
- **A stacked group** — electric and ice hit the whole pile, and the fire line works
  the leader down one burn at a time.
- **warden_cleric, ashkin_life** — the burn and the freeze outlast the healing tick
  once the group is rooted.

## Open questions

- CON 0 gives 80 health by the shared health formula. If the wardens get a class
  health multiplier later, the Sorcerer loses more of it than anyone else.
- The slow numbers assume a base walk speed of 100%. Move speed changes move them.
