# Warden Archer — class sheet

The archer wins before it shoots. The fan shot is the payoff; the marks, the lines and the coat are the setup. Every arrow takes something away, so the fight it wins is a fight it never had.

- **Faction** wardens · **Tier** 1 · **Branch** warden_rogue · **Role** ranged debuffer
- **Growth per level** DEX +1.5, STR +1.5, CON 0, INT 0, WIS 0
- **Stat priority** DEX first — the multi-shot, the slows and the range all scale on it — then STR, then CON never. The class has no escape tool, so it must not be caught.
- **Units** range and radius in metres, cast and cooldown in seconds, mana in points against a 100-point bar (house number), damage in raw points before armour.

## Key abilities

| tier | name | kind | cast | cd | range | damage | what it does |
|---|---|---|---|---|---|---|---|
| 0 | Loose Arrow | projectile | 0.5 s | 1.2 s | 22 m | 24 + 0.8/DEX | the basic shot, no mana |
| 0 | Veil Coat | stealth | 1.0 s | 20 s | self | — | cloak 12 s: fill rate x0.55, detection range 21 m, 4.0 of 5.0 m/s, ends on any damage |
| 0 | Quarry Marks | utility | 0.8 s | 18 s | 20 m radius | — | reveals cloaked targets inside for 4 s, no meter filled |
| 1 | **Fan Shot** | projectile | 0.6 s | 9 s | 20 m cone 40° | 18 + 0.55/DEX **per arrow** | 3 arrows (4 from the coat), each slow 30% for 2.5 s and weaken 8% for 4 s |
| 1 | Binding Arrows | projectile | 0.8 s | 12 s | 18 m | 20 + 0.7/DEX | slow 55% for 4 s, weaken 12% for 6 s |
| 2 | Veil Line | utility | 1.0 s | 25 s | 18 m long, 4 m wide | — | dust line 12 s, 60% fill-rate cut for anyone on the far side |
| 2 | Volley | aoe | 1.2 s | 16 s | 24 m, 4 m circle | 20 + 0.45/DEX per arrow, 4 arrows | 1.2 s ground telegraph, slow 25% for 2 s |
| 3 | Hunter Mark | debuff | 0.7 s | 14 s | 20 m | — | 10 s: +25% damage from the archer, extra slow 20% for 3 s, shown to the warband |
| 3 | Longshot | projectile | 1.0 s | 10 s | 30 m | 70 + 1.2/DEX | one heavy arrow, a thin line is drawn for the whole 1.0 s |
| 4 | Storm of Arrows | aoe | 1.5 s | 45 s | 28 m, 6 m circle | 12 + 0.3/DEX per arrow, 12 arrows | over 3 s, 1.5 s ground telegraph, slow 40% for 3 s |

## Stealth rules

The archer hides with the same server-side model as the rest of the roster: graduated detection per observer, never a binary aura.

```
fill rate (points per second) = (D + F) x (1 - C) x M
```

- **D, distance term**: 100 points at 2 m or closer, 0 at the effective range R, linear in between.
- **F, facing term**: 100 inside 45° of the observer's look direction, 50 from 45° to 90°, 0 beyond 90°.
- **R, effective range** (the spec term): 30 m base, 26 m against a resting archer, 21 m inside Veil Coat.
- **M, motion term**: 1.0 standing still, 1.4 walking, 2.0 sprinting.
- **C, concealment**: 0 normally, 0.45 inside Veil Coat.

At 100 points the observer has **Spotted** the archer: that observer and their three nearest allies within 20 m see it for 4 s. The meter drains 25 points per second once the rate is 0, with a 2 s grace.

The archer's own stealth is a coat, not a skill set: 12 s, and it ends the instant the archer deals damage. Two things buy the archer the time it needs instead of the coat — Veil Line, which puts a 4 m wide dust screen between the archer and the melee ring, and Quarry Marks, which takes the cloak off a hidden enemy without filling anybody's meter.

## The shot rule

The multi-shot is scaled on **DEX**, per arrow: 18 + 0.55 per DEX. At DEX 40 that is 40 an arrow, 120 for the fan of three, 160 for the fan of four fired out of the coat.

Every arrow of the fan carries a real slow — **30% for 2.5 s** — plus an 8% weaken for 4 s. The slow and the weaken refresh, they do not stack. Binding Arrows is the heavy case: **55% slow for 4 s** and a 12% weaken for 6 s on one target. The damage is what the numbers say; the slow is the reason the archer is a class and not a bow.

## What it loses to

- **warden_fighter and ashkin_combatant** — 6 m of closing before the first arrow lands, and the archer has no answer under 8 m.
- **warden_fighter's net and ashkin_attacker's charge** — both end the fight before the bow speaks.
- **warden_defender and ashkin_templar** — a shield arc eats the arrows, so the archer needs another mark for them.
- **A sustained chase** — no escape tool, only 12 m of cloak, and the cloak breaks on the first shot.

## What it counters

- **warden_warrior** — the dual-wield runner crosses 10 m of slowed ground to reach the archer.
- **ashkin_templar** — the frontal guard answers melee, not a marked target at 20 m.
- **warden_sorcerer** — the ground-targeted cast is a stationary circle, and the volley lands on a circle.
- **warden_mage** — the magic cell has to be held, and the binding arrows stop the holder moving.
- **warden_assassin** — Quarry Marks strips a cloak and the detection meter is slow to fill on a target that is not looking at the archer.

## House numbers

Every number on this sheet is a design number, not a measured one: the 0.55 per-DEX arrow scale, the 30% and 55% slows, the 40 m² storm circle, the 21 m coat range and the 20 m reveal radius are placeholders to be tuned in playtest. The detection formula is the part that must not move — a binary aura is what made hiding feel like a coin flip.
