# Baselines, vocabulary and rulings

Owner: integration, 2026-10-21. Companion to `scaling.md`. When a sheet, a
class file and this page disagree, this page wins.

## Level-1 baselines

The house baseline, one for the whole game:

    health     = 200 + 25 * CON
    mitigation = 1% per CON, capped at 35%
    mana pool  = 300 at level 1 for casters, +20 per level
    mana regen = 6 per second out of combat, 2 per second in combat
    stat S     = 10 + growth(stat) * (level - 1)

A standard hit lands for 120. A level 1 heavy with 11.5 CON has 487 health,
so four hits kill it. A level 1 caster with 0 CON has 200 health, so two do.
That is the whole reason casters are fragile and bruisers are not.

| archetype | CON at level 1 | health at level 1 | move m/s |
|---|---|---|---|
| wardens heavy | 11.5 | 487 | 5.0 |
| wardens light | 10 | 450 | 5.6 |
| wardens caster | 10 | 450 | 5.0 |
| wardens medic | 11 | 475 | 5.0 |
| ashkin heavy | 11.5 | 487 | 4.8 |
| ashkin gunner | 11.5 | 487 | 4.6 |
| ashkin caster | 10 | 450 | 5.0 |

Weapon damage adds to class base. A weapon with `base` 20 at level 1 means
20 plus the class's own base. Armour subtracts a flat value from incoming
damage, and the fight is a race between a healer's output and a bruiser's
swing rate, not between two damage totals.

The flask: three charges, two second cooldown, one shared pool for healing
and mana, and the pool is per player. A priest psalm is stronger in a group
and slower solo. That is the whole reason flasks are not the answer to a
dead party member.

## Effect vocabulary

One word, one meaning. These are the only effect strings allowed in a class
file.

`damage` `heal` `shield` `slow` `root` `stun` `freeze` `blind` `weaken`
(the target deals less) `damage_taken` (the target takes more) `mark`
`stealth` `break_stealth` `reveal` `noise` `conceal_zone` `dash` `knockback`
`drag` `stagger` `execute` `poise_damage` `buff` `debuff` `cleanse`
`revive` `tomb_marker` `xp_cost` `channel` `dot` `mana_cost` `move_speed`
`attack_speed` `evade` `damage_reduction` `spell_shield` `charge` `resource`.

## Rulings on the open questions

1. **Revival cost.** The revived player pays 25 percent of the XP they need
   for their next level. The caster pays mana and channel time only. The
   cleric revives one ally fast; the priest revives a small group slowly.
   Both exist; they are not the same ability.
2. **Tomb markers** are visible to both factions. A griefer pays for it.
3. **Auras** anchor where the caster stood and break when the caster is hit.
4. **Fame is war-only.** Boss kills, world firsts and bounties pay coin and
   renown. `content/systems/economy.md` currently lists them as fame
   sources; that line is wrong and must change.
5. **Node buff** lasts two minutes while the node is held, with a two minute
   grace after it is lost. Not two hours. A permanent buff makes the war
   about who arrives first.
6. **Fort hit counts** are the invariant: 200, 500 and 1000 clean hits at
   level 1 for levels 1, 2 and 3. Hit points get rescaled when the class
   damage numbers land; the hit counts do not move.
7. **Wire tick 30 Hz, authoritative sim tick 60 Hz.** One wire tick carries
   two sim ticks. The 100 ms input buffer is the WAN target; the shipped LAN
   path uses 33 ms and is a detail of the duel, not of the war.
8. **Detection** is one shared model, not one per class. It lives in
   `content/systems/detection.json`.
9. **Scaling deviations** — these abilities were authored with a flat
   per-point rule or a 3 percent rule. The house rule is **4 percent of base
   per point above 10** (see `scaling.md`). Convert the base so the value at
   DEX 40 stays where the sheet put it.

| file | ability | authored | house base at 4% |
|---|---|---|---|
| `warden_archer.json` | Fan Shot | 18 + 0.55 per DEX per arrow | 18 (unchanged) |
| `warden_archer.json` | Longshot | 70 + 1.2 per DEX | 48 |
| `warden_assassin.json` | Fang Burst | 45 + 1.1 per DEX | 35 |
| `ashkin_templar.json` `ashkin_attacker.json` `ashkin_gunner.json` | every damage ability | 3 percent | base unchanged, rule becomes 4 percent |

Healing, shields and buffs stay flat per point under Rule S2 and are not
converted. A heal is not a damage number.

10. **Sockets open at +5**, one gem per socket, three gems at the top grade.
    Grade bands and failure rates are in `items.json` and are the authority
    for gear.
11. **Level cap is 60.** Ability tier maps to level: tier 0 at level 1,
    tier 1 at 10, tier 2 at 20, tier 3 at 30, tier 4 at 40. A tier 5 does
    not exist; if a class needs a sixth line it gets a new class.
12. **No name from the study game appears anywhere in this repo** — not in a
    class file, not in a style document, not in a comment. The factions are
    the Wardens and the Ashkin. The study game is referred to as "the 2005
    reference" in prose.
