# Warden Enchanter — class sheet

**Role:** control and buffs · wardens · tier 1 (branch `warden_mage`)
**Growth:** +1.5 INT, +1.5 DEX per level · **Primary:** INT, DEX
**Loadout:** short silver focus rod with a gold ring · bone white grimoire on a chain
**2026 fix (roster):** the interrupt becomes the dedicated interrupt.

The roster row still calls this role "seals and buffs". That word is an RYL1 skill
name, so the class file and this sheet say "control and buffs" instead. The mechanic
is unchanged: one interrupt that cancels any charge.

## Stat priority

1. **INT** — +1.5 per level. Every buff amount scales on INT, so the class gets
   stronger as the war goes on.
2. **DEX** — +1.5 per level. It shortens the small casts, and the small cast is the
   interrupt. DEX is the stat that makes the class good at its one real job.
3. **CON** — nothing. 80 health and no mitigation, at every level.

## The four jobs

| job | spell | tier | what it does |
|---|---|---|---|
| interrupt | Bind Mark | 0 | cancels any cast or charge, roots 1 s, blocks the next cast 1.5 s |
| interrupt, capstone | Deep Bind | 4 | cancels a charge, root 3 s, removes one buff |
| weapon buff | Keen Edge (1 ally), War Edge (5 allies) | 0 and 4 | +18% and +34% weapon damage, INT scaled |
| party buff | Haste Ward, Ward Link, Mana Well | 1 to 3 | move speed, damage taken, mana regen |
| ground control | Snare Field, Hush Field | 1 and 3 | slow fields with decal telegraphs |
| personal shield | Hoarfrost Skin | 2 | shield 200 at L10 for 8 s |
| filler damage | Spark Jab | 0 | 18 at L10, keeps the class from going silent |

## Numbers model

- **Damage:** `D = base x (1 + 0.04 x (S - 10))`, `S = 10 + growth x (level - 1)`.
  INT: L1 = 10 (x1.00) · L10 = 23.5 (x1.54) · L20 = 38.5 (x2.14) · L40 = 68.5 (x3.34).
- **Health:** `200 + 25 x CON` = **200 at every level**. **Mitigation:** 1% per CON
  point, capped at 35%. The Enchanter has 0% of both.
- **Cast:** a spell with a listed cast of 1.0 s or less gets 2% shorter per DEX point
  above 10, floor 70%. At DEX 40 the interrupt is a 0.21 s cast. Any cast over 1.0 s
  is fixed, so a ground field always takes the time the decal promised.
- **Fields:** `range` = reach in metres, `radius` = area radius in metres,
  `cast` = seconds, `cooldown` = seconds, `mana` = points, `telegraph` = the decal.

## The interrupt (this is the class)

1. **Bind Mark** has a 0.3 s cast and a 14 s cooldown. It cancels any cast or charge
   in progress, blocks the next cast for 1.5 s and roots the target for 1 s. An enemy
   charge that dies leaves its decal on the ground with no spell on it.
2. The interrupt **cannot catch a caster in the last 0.3 s of a charge**. That window
   is the enemy caster's only safe moment, so the Enchanter fires before the charge
   starts, not while it runs.
3. **Deep Bind** (tier 4) does the same thing but pins the caster for 3 s, so the
   party can hit a caster that would otherwise walk away.
4. The two ground fields, **Snare Field** (5.0 m) and **Hush Field** (8.0 m), use the
   same decal rule as the caster lines: the decal radius is the spell radius, and the
   field lives 8 s after the release.
5. Buffs land on the party, not on the enemy. Keen Edge on one ally inside 12 m; the
   party buffs on 4 or 5 allies inside a 12 to 16 m radius around the caster.

## The eleven that matter

| Ability | Tier | Numbers |
|---|---|---|
| **Bind Mark** | 0 | Target, reach 18 m, 0.3 s cast, cd 14 s, 10 mana. No damage. Interrupt, blocks the next cast 1.5 s, root 1 s. |
| **Keen Edge** | 0 | Target, reach 12 m, 0.4 s cast, cd 20 s, 12 mana. Weapon damage **+18%** at L10 (**+40%** at L40) for 20 s, one ally, stacks three times. |
| **Spark Jab** | 0 | Single, reach 16 m, 0.2 s cast, cd 3 s, 6 mana. Base **12** (L10: 18). Slow 15% / 1.5 s. |
| **Snare Field** | 1 | Circle r 5.0 m, cast 0.8 s, cd 22 s, 18 mana. Base **6** (L10: 9). Slow 50% / 5 s on entering, the field keeps slowing for 8 s. |
| **Haste Ward** | 1 | Aura r 12 m, cast 0.6 s, cd 30 s, 20 mana. Move speed **+30%** for 12 s on up to 4 allies, caster included. |
| **Ward Link** | 2 | Aura r 14 m, cast 1.0 s, cd 45 s, 35 mana. Damage taken **-18%** at L10 (**-40%** at L40) for 15 s on up to 5 allies. |
| **Hoarfrost Skin** | 2 | Self, 0.3 s cast, cd 30 s, 25 mana. Shield **200** at L10 on the INT scale, for 8 s. |
| **Mana Well** | 3 | Aura r 14 m, cast 0.8 s, cd 50 s, 40 mana. Mana regen **3 per second** at L10 for 20 s on up to 4 allies. |
| **Hush Field** | 3 | Circle r 8.0 m, cast 1.2 s, cd 40 s, 30 mana. No damage. Slow 60% / 8 s, the caster is never slowed by its own field. |
| **Deep Bind** | 4 | Target, reach 20 m, 0.5 s cast, cd 45 s, 40 mana. No damage. Root 3 s, cancels a charge in progress, removes one buff. |
| **War Edge** | 4 | Aura r 16 m, cast 1.0 s, cd 60 s, 45 mana. Weapon damage **+34%** at L10 (**+73%** at L40) and attack speed +10% for 25 s on up to 5 allies. |

Mana per tier: tier 0 spells cost 6 to 12, tier 1 costs 18 to 20, tier 2 costs 25 to
35, tier 3 costs 30 to 40, tier 4 costs 40 to 45. Cooldown per tier: tier 0 sits at
3 s to 20 s, tier 1 at 22 s to 30 s, tier 2 at 30 s to 45 s, tier 3 at 40 s to 50 s,
tier 4 at 45 s to 60 s.

## How it plays

1. **Keep the interrupt for the first charge you see**, not the second. The first
   charge of a push is the one that carries the enemy plan.
2. **Stay 14 to 18 m back.** Bind Mark reaches 18 m, the buffs reach 12 to 16 m, and
   the class has 80 health.
3. **Buff the front line, not yourself.** Keen Edge and War Edge are worth more than
   any damage this class can put out.
4. **Save the party buffs for the clash.** Ward Link and War Edge are 45 s and 60 s
   cooldowns. One good trade each is the whole war.
5. **Never trade.** Spark Jab is for weaving between controls, not for winning a
   duel. 80 health, no self healing, one assassin opener ends the class.

## What it loses to

- **warden_assassin, ashkin_shadow** — the lowest health in the game. One opener
  ends the Enchanter, and the interrupt is on a 14 s cooldown when it happens.
- **warden_defender, ashkin_templar** — a shielded front line takes no damage from
  binds, and the Enchanter brings none of its own, so the party loses the timer.
- **The objective** — every control needs a target inside 18 m, so the Enchanter
  cannot hold a capture point on its own.
- **warden_cleric, ashkin_life** — heals outlast binds. Root 3 s is not a kill
  window when the healer is next to the target.

## What it counters

- **warden_sorcerer, ashkin_rune** — the interrupt cancels the charge every time, so
  the enemy group spell never lands. This is the strongest matchup in the wardens
  line.
- **warden_assassin, ashkin_shadow** — Deep Bind pins the opener for 3 s and the
  weapon buff turns the rest of the party into the answer.
- **A slow push** — the two slow fields and the roots hold a push back long enough
  for the party to win the trade.

## Open questions

- CON 0 gives 80 health by the shared health formula, the lowest in the game. If
  wardens get a class health multiplier later, this class needs one too.
- The buff values are percentages of weapon damage. If gear adds flat damage
  instead, the weapon buff needs a different formula.
