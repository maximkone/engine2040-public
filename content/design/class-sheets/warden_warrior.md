# Warden Warrior — class sheet

**Role:** dual-wield burst · wardens · tier 1 (branch `warden_fighter`)
**Growth:** +2 STR, +1 CON per level · **Primary:** STR, CON
**Loadout:** arming sword + arming sword
**2026 fix (roster):** the combo timing is an intended skill, not a cancel bug.

## Stat priority

1. **STR** — +2 per level, the biggest growth in the wardens. Every number on
   this sheet scales on STR.
2. **CON** — the thinnest health of the human line. Buy what you can, then stop.
3. **DEX** — nothing. The Warrior has no evasion tool; its defence is movement.

## The combo (this is the class)

1. **Marching Song** (T0) — aura r 6 m, +20% damage, chain window 0.8 s ->
   **1.2 s**, and the 4th chain link opens a **finale window of 2.0 s**.
2. **Chain** — Paired Strike landed inside the chain window adds a link:
   **+12% damage per link, 4 links max (+48%)**. A missed window resets the chain
   to 1 link. The button always fires; nothing is cancelled. The chain window is
   your slack: it is the time you have to fit Wide Sweep or Hammer Rhythm into
   the rotation without dropping the chain, which is exactly what the songs buy.
3. **Finale window** — 2.0 s (Marching Song), 3.0 s (Deep Song), 4.0 s (Last
   Verse, extendable to 6 s by landing links inside it).
4. **Crescendo inside that window** — **x2.2 damage** and the song is consumed.
   Outside it, Crescendo still fires, at base damage, and the song keeps running.

## Numbers model

- **Damage:** `D = base x (1 + 0.04 x (S - 10))`, `S = 10 + growth x (level - 1)`.
  STR: L1 = 10 (x1.00) · L10 = 28 (x1.72) · L20 = 48 (x2.62) · L40 = 88 (x4.22).
- **Health:** `200 + 25 x CON`. **Mitigation:** 1% per CON point, capped at 35%.
  At L10: 270 health, 19% mitigation — the softest of the human line.
- **Fields:** `range` = reach from the Warrior, `radius` = size of the area,
  `cast` = wind-up seconds, `cooldown` = seconds, `mana` = points,
  `telegraph` = the decal that warns the effect.

## The five that matter

| Ability | Tier | Numbers |
|---|---|---|
| **Paired Strike** | 0 | Single, reach 2.4 m, cd 0.5 s. Base **26** (L10: 45). **+12% per chain link, 4 max (+48%)**, chain window 0.8 s (1.2 s to 1.5 s in song). |
| **Marching Song** | 0 | Aura r 6 m, 12 s, cd 30 s, 18 mana. **+20% damage**, chain window **1.2 s**, finale window **2.0 s**. |
| **Wide Sweep** | 1 | Cone 120 deg, r 3.2 m, 0.6 s telegraph, cd 12 s. Base **44** (L10: 76). Slow 35% / 2 s, **each hit adds 1 chain link**. |
| **Deep Song** | 2 | Aura r 10 m, 14 s, cd 50 s, 32 mana. **+30% damage** (allies +20%), chain window **1.4 s**, finale window **3.0 s**, 4 mana back per link. |
| **Crescendo** | 2 | Circle r 3.5 m, 0.6 s telegraph, cd 16 s, 30 mana. Base **70** (L10: 120). **x2.2 inside the finale window** (L10: 265), knockback 2 m. |

Also live: **Hammer Rhythm** (T2, 3 m wide line, 10 m long, base 66, 1 chain link
per target hit), **Bloodbeat** (T3, next 3 links +20% and apply bleed),
**Song of Two** (T1, aura r 8 m, caster +20% and allies +12%, same chain and
finale windows), **Last Verse** (T4, aura r 12 m, caster +45% and allies +30%,
chain window 1.5 s, finale window 4.0 s -> 6 s) and **Final Crescendo** (T4,
circle r 5.0 m, base **150**, x2.5 in the window).

The song is one line that grows with the tier: aura r 6 -> 8 -> 10 -> 12 m,
your damage +20% -> +20% -> +30% -> +45%, chain window 1.2 -> 1.2 -> 1.4 -> 1.5 s,
finale window 2.0 -> 2.0 -> 3.0 -> 4.0 s.

## How it plays

1. **Open the song.** Marching Song is the enabler: without it the chain window
   is 0.8 s and there is no finale.
2. **Run the chain.** Paired Strike -> Stride Cut -> Wide Sweep, each one landed
   inside the window. Four links = +48% and the finale window opens.
3. **Spend it fast.** Two seconds is the whole window at tier 0 and six at tier
   4. Crescendo at x2.2 is 265 damage at L10; a wasted song is a wasted 265.
4. **Spend the mana.** Deep Song refunds 4 mana per link, Last Verse 6. A clean
   chain pays for the next song.
5. **Do not stop moving.** 270 health at L10 and no mitigation tool. The Warrior
   wins by being mid-combo, never by standing in a trade.

## What it loses to

- **warden_defender** — held outside 2.4 m the chain never lands, and Bulwark
  Charge punishes the standing still. The aura also eats the first hit of every
  exchange while the Warrior is still building the chain.
- **warden_cleric, ashkin_life** — a burst has to be bigger than the heal. It is
  not, once the song is already spent.
- **ashkin_gunner** — cannon range is outside the chain window. The combo needs
  2.4 m.
- **Two opponents** — one song, one finale window, one Crescendo. The chain is
  built on a single target.

## What it counters

- **warden_rogue, warden_assassin, ashkin_shadow** — the stealth opener gets
  finished before the second blade lands. Crescendo at x2.2 is more than a
  half-health rogue can pay.
- **warden_fighter, ashkin_combatant** — slow 35% and 4 chain links from one
  cone. The running combo is faster than their swing recovery.
- **ashkin_attacker** — a drain bruiser that wants to stay in melee pays the
  chain bonus on every hit it takes.
