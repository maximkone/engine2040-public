# Warden Assassin — class sheet

One clean hit from nowhere, then the fight is over. Everything else is setup for that hit or the walk away from it.

- **Faction** wardens · **Tier** 1 · **Branch** warden_rogue · **Role** stealth burster
- **Growth per level** DEX +2, STR +1, CON 0, INT 0, WIS 0
- **Stat priority** DEX first (every damage number and both slows ride it), STR second, CON never. The class has 100 HP of paper and no heal, and that is the trade.
- **Units** range and radius in metres, cast and cooldown in seconds, mana in points against a 100-point bar (house number), damage in raw points before armour.

## Key abilities

| tier | name | kind | cast | cd | range | damage | what it does |
|---|---|---|---|---|---|---|---|
| 0 | Veil Breath | stealth | 0.6 s | 14 s | self | — | stealth 8 s: fill rate x0.45, detection range 16 m, facing halved, 3.0 of 5.0 m/s |
| 0 | Soft Step | utility | 0.3 s | 16 s | self | — | quiet 10 s: detection range 24 m, motion term 0.7 walking / 0.5 still |
| 0 | Short Fang | melee | 0.25 s | 1.2 s | 2.0 m | 26 + 1.0/DEX | the plain stab; any damage drops stealth |
| 1 | **Fang Burst** | melee | 0.3 s | 10 s | 2.2 m | 45 + 1.1/DEX, x2.4 players, x3.6 NPCs | the opener, needs stealth and drops it; slow 30% for 3 s |
| 1 | Hamstring | debuff | 0.4 s | 8 s | 2.5 m | 16 + 0.6/DEX | slow 45% for 5 s |
| 2 | Shadowstep | utility | 0.2 s | 9 s | self | — | dash 6 m, 2 iframes, drops stealth |
| 2 | Flaw Mark | debuff | 0.5 s | 14 s | 2.5 m | — | target takes +20% damage for 6 s |
| 3 | Rend | melee | 0.45 s | 12 s | 2.6 m cone 60° | 38 + 0.9/DEX | hits everything in the cone, slow 25% for 3 s |
| 3 | Blind Powder | control | 0.5 s | 20 s | 4.0 m cone 60° | 8 | 40% miss for 3 s inside, drops stealth |
| 4 | Last Breath | melee | 0.6 s | 45 s | 2.2 m | 70 + 1.4/DEX | +60% under 30% health, +30% only against players |

## Stealth rules

Detection is computed by the server, per observer, per tick. There is no stealth aura and no radius that reveals the assassin.

```
fill rate (points per second) = (D + F) x (1 - C) x M
```

- **D, distance term**: 100 points at 2 m or closer, 0 at the effective range R, linear in between.
- **F, facing term**: 100 inside 45° of the observer's look direction, 50 from 45° to 90°, 0 beyond 90°.
- **R, effective range** (the spec term): 30 m base, 24 m against a resting assassin, 16 m while in Veil Breath.
- **M, motion term**: 1.0 standing still, 1.4 walking, 2.0 sprinting.
- **C, concealment**: 0 normally, 0.55 inside Veil Breath. Soft Step cuts the motion term instead of the rate.

The observer's meter fills at that rate. At 100 points the target is **Spotted**: the observer and their three nearest allies within 20 m see it for 4 s, and the stealth ends. The meter drains 25 points per second once the rate is 0, with a 2 s grace. Every observer has its own meter: two people looking the same way notice the assassin at different times.

Inside Veil Breath the facing term is halved on top of the 0.55 concealment, so the assassin is really only safe behind a body or around a corner. The stealth ends on any damage, on Fang Burst, on Shadowstep, on Blind Powder, or at 8 s.

## The opener rule

The old number was x4.0 damage from stealth. Against players it is now **x2.4 — 60 percent of it** — and x3.6 against NPCs. The opener always forces the assassin out of stealth, so there is no second one from the same hiding place. At DEX 40, Fang Burst is 89 raw and 214 from stealth against a player.

## What it loses to

- **warden_defender** — damage redirection plus the net hold the assassin in place until the warband arrives. One pick becomes a loss.
- **warden_warrior** — the dual-wield stagger punishes the 0.6 s of Last Breath, the one moment the class stands still.
- **ashkin_templar** — the frontal guard arc puts a shield between the assassin and its target.
- **Any archer with a clear line** — the assassin is seen before the opener lands, and the opener lands on a target that is ready.
- **A second enemy** — no heal, no crowd tool. Rend is the answer and it is weak.

## What it counters

- **warden_sorcerer** — the charge cast is a standing charge, the assassin closes inside it and the ground decal is useless at 2 m.
- **warden_mage** — the magic cell has to be held, and the assassin is faster than the hold.
- **ashkin_gunner** — the cannon recoil is the tell, and 3 s of slow on a gunner is a dead gunner.
- **warden_cleric** — the chant does not move, so the opener lands on a stationary target.

## House numbers

Every number on this sheet is a design number, not a measured one. The 60% opener cap, the 3.0 m/s stealth speed, the 16 m stealth range, the 100-point meter and the 25 points per second drain are all placeholders to be tuned in playtest. The detection formula is the part that must not move: a binary aura is what made stealth feel like a coin flip.
