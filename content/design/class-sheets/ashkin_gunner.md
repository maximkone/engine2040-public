# Ashkin Gunner — arm-cannon artillery

Tier 1 Ashkin branch of `ashkin_combatant`. Owns the far half of the field.
Every shot roots him and shoves him back, so the ground is chosen before
the trigger is pulled.

## Role

Arm-cannon artillery. The Gunner does not enter the brawl: it shapes it.
Set a zone at 20 m or 25 m, punish anyone who walks into it, and reveal
what the enemy thought was hidden.

Three jobs.

1. Put damage on the ground before the enemy reaches it.
2. Break the enemy's stealth for the whole faction, not just for yourself.
3. Stay out of reach: the recoil always pushes you back 1.5 m.

## Stat priority

| stat | growth per level | why |
|---|---|---|
| DEX | 1.5 | every cannon number scales on DEX |
| CON | 1.5 | the Gunner eats one dive and must live through it |
| STR | 0 | the artillery arm does not carry the fighter |
| INT | 0 | no casting |
| WIS | 0 | no healing done here |

Primary order: **DEX, CON**. Matches the silhouette: mobile and tough.

## Numbers (house rules)

- 60 Hz tick. 1 tick = 16.7 ms.
- `damage.base` is the level-1 raw number in the contested middle, no gear.
- Every point of the scaling stat above 10 adds **3% of base**.
- Full readable telegraph floor is **340 ms** (`crates/combat/tests/it/telegraph_floor.rs`).
- Range, radius in meters. Cast, cooldown in seconds.

## Loadout

Weapon: arm cannon. Offhand: mechanical claw.
Silhouette: wide stance, cannon diagonal across the shoulder, barrel longer
than the torso, muzzle glowing, one arm replaced by machinery, the other
hand a claw. Enclosed helm, no face.

## Key abilities

| ability | tier | kind | what it does | numbers |
|---|---|---|---|---|
| Claw Strike | 0 | melee | the free hand, no root | 30 dmg (DEX), stagger 0.4 s, 2.0 m, cd 4.0 s |
| Arc Cannon | 0 | projectile | **the main gun** | 70 dmg (DEX), 30° cone, 20.0 m, roots the Gunner 0.6 s, 0.4 s muzzle trace, 1.5 m recoil, cd 5.0 s |
| Recoil Step | 0 | utility | the answer to the root | cancels the charge root, 3.0 m back along the aim, cd 6.0 s |
| Cluster Shell | 1 | aoe circle | lob a shell over cover | 85 dmg (DEX), 12.0 m throw, 3.5 m circle, cast 0.5 s, stagger 1.0 s, knockback 2.0 m, breaks binds, burning ground 3.0 s at 10 dmg/s, cd 14.0 s, 15 mana |
| Ash Cloak | 1 | stealth | hidden, but it dies when you shoot | 10.0 s, hidden beyond 8.0 m, broken by any cannon ability, broken by taking 60+ damage in one hit, movement -10%, cd 22.0 s, 12 mana |
| Smoke Bloom | 1 | utility | deny sight to a whole team | 5.0 m cloud at 8.0 m, 8.0 s, enemies inside see only 3.0 m, blocks cannon sight lines both ways, cast 0.4 s, cd 18.0 s, 14 mana |
| Barrage | 2 | aoe circle | the zone | 140 dmg (DEX), 25.0 m, 6.0 m circle, cast 1.0 s, 6 shells over 3.0 s at 0.5 s each, 45 damage per shell in a 3.0 m circle, cd 30.0 s, 26 mana |
| Aim True | 2 | buff | the artillery button | 8.0 s, next shot +50% damage, pierces one extra target, the trace turns white, cd 25.0 s, 10 mana |
| Anchor Shot | 3 | projectile | hook a target into cannon range | 60 dmg (DEX), 22.0 m, roots 2.0 s, pulls the Gunner to the target over 1.0 s, breaks on knockback, cd 12.0 s, 12 mana |
| Ground Pulse | 3 | utility | **faction-wide reveal** | 15.0 m radius around the Gunner, cast 0.6 s, reveals every hidden enemy to the whole Ashkin faction for 12.0 s, cancels stealth effects on the ground, cd 30.0 s, 20 mana |
| Siege Stance | 4 | buff | artillery mode | 15.0 s, cannon fires every 1.2 s with no cooldown, each shot roots 0.5 s and adds 1.0 m of recoil, +30% cannon damage, cannot move, cd 120.0 s, 45 mana |

### Arc Cannon, in words

**Root-and-charge, 0.4 s muzzle trace, 30 degree cone at 20.0 m.**

- While the cannon cycles the Gunner is **rooted for 0.6 s**. The gun is not
  a mobile option; it is a commitment.
- The shot draws a **glowing line from the muzzle to the aim point for
  0.4 s** and the shell lands at the end of that line. 400 ms is above the
  340 ms readable floor, so the warning is legal but tight.
- **Recoil pushes the Gunner 1.5 m back** along the aim, every shot. Six
  shots put the Gunner 9.0 m behind where the fight started.
- The cone is 30 degrees. At 20.0 m that is a 10.7 m wide bite, so the
  shot hits a group, not a person.

Recoil Step is the counter to the root: 6.0 s cooldown, no mana, 3.0 m back
and the trace stays where it was.

### Ash Cloak, in words

The cloak and the cannon cannot live together.

- The Gunner is hidden from enemies **more than 8.0 m away** for 10.0 s.
- **Firing any cannon ability breaks it**: Arc Cannon, Cluster Shell, Anchor
  Shot, Barrage, and the Siege Stance shots.
- Taking **more than 60 damage in one hit** breaks it too.
- Movement speed is -10% while cloaked.

So the order is shot first, cloak second. Cloaking with a live target in
cannon range is a mistake, not a strategy.

### Ground Pulse, in words

The reveal belongs to the **whole Ashkin faction**, not to the Gunner.

- A 15.0 m pulse around the Gunner, 0.6 s cast, 30.0 s cooldown.
- Every hidden enemy inside is **revealed to the entire Ashkin team for
  12.0 s**. Not the Gunner alone, not a personal vision: every Ashkin in
  the warband gets the mark, and the mark is visible to the enemy too.
- The pulse also cancels stealth effects on the ground inside it: cloaks,
  shadow steps and smoke walls.

That is the answer to a rogue or an assassin. One pulse, 15.0 m, and the
whole faction stops guessing where the knife is.

## What it loses to

- **Close range.** Every shot roots the Gunner 0.6 s and pushes him 1.5 m
  back, so a fast melee arrives between shots and the recoil walks him
  into the enemy line.
- **Losing the ground.** Siege Stance roots him 0.5 s per shot for 15.0 s
  and the recoil stacks: after 6 shots the Gunner has been pushed 6.0 m
  back and has to be pulled off the line by hand.
- **Its own smoke.** Smoke Bloom blocks cannon sight lines **for both
  sides**. The Gunner can break a sight line he was counting on.
- **Focus fire.** 1.5 DEX and 1.5 CON is mobile and tough, not durable.
  One clean dive into the 0.6 s root window is a dead Gunner.

## What it counters

- **warden_rogue** — Ground Pulse reveals the whole Ashkin faction, so one
  dig finds every hidden rogue and the whole team acts on it.
- **warden_assassin** — same answer, and stronger: the class that opens
  from stealth cannot open when every Ashkin sees the mark.
- **warden_archer** — 20.0 m and 25.0 m of reach beats a bow, and Smoke
  Bloom takes the arrow line away entirely.
