# Scaling and data conventions — one authority

Owner: integration, 2026-10-21. This file is the single source for how the
numbers in `content/classes/*.json` and `content/design/class-sheets/*.md`
are read. When two sheets disagree, this file wins.

## Rule S1 — damage scales with the scale stat, percent of base

`damage.base` is the level-1 raw value, no gear, in the contested middle.
For a scale stat `S` and base `B`:

    bonus  = ((S - 10) * B * pct) / 100      integer math, rounded down
    result = B + bonus

`pct` is **4** unless an ability states another integer percent. `S` below 10
contributes nothing. `scale: "none"` means no stat contribution at all.

`S` itself comes from the class growth and the level:

    S = 10 + growth(stat) * (level - 1)

Integer only: the tick uses fixed and integer math (`docs/HASH-LAW.md`), so a
percent rule is arithmetic, never a float multiply.

The house baseline, used by every class:

    health   = 80 + 10 * CON
    mitigation = 1% per CON, capped at 35%
    mana pool = 300 at level 1 for casters, +20 per level
    mana regen = 6 per second out of combat, 2 per second in combat

Files authored before this ruling used 3 percent, or a flat per point rule.
The conversion table is in `stats.md`.

## Rule S2 — heals, shields, buffs and resources are flat per point

Healing, shield values, buff magnitudes, mana and endurance are `base + flat
* stat`, with `flat` written in the sheet's Numbers table. They do not use
Rule S1. A heal that is a percent of base is a debuff or a damage effect, and
belongs under Rule S1.

## Rule S3 — cast time shrinks with the stat, and never below the floor

Cast time is reduced by 2 percent per point of the relevant stat above 10,
floor 70 percent of the listed cast time, and never below the **340 ms
readable telegraph floor** (`crates/combat/tests/it/telegraph_floor.rs`).
A 400 ms muzzle trace is legal. A 200 ms one is not.

## Rule S4 — telegraph and shape

Every ability with `kind: "aoe"` has a telegraph, and the telegraph shape
matches the effect shape: cone for cone, circle for circle, line for line.
`charge` means the telegraph grows over the cast and the effect resolves at
the end of it. `radius` and `range` are metres. `cast` and `cooldown` are
seconds.

Shape semantics, fixed: `single` is one chosen ally, `self` is the caster,
`target` is one chosen enemy, `cone`, `circle` and `line` are ground shapes.

## Rule S5 — growth, counters, pulses

- `growth` carries all five stats. Stats that do not grow are `0`, not
  missing. The per-level numbers are exactly those in `roster.json`; the
  point budget behind them is two points per level for the six starters and
  three for the advanced classes, spent as the roster lists. The sum of the
  listed growth is therefore not always the point budget, and that is fine.
- `counters` is a list of class ids (`ashkin_shadow`), not prose. A sheet may
  explain them in words; the JSON stays ids.
- A detection pulse (reveal, dig) breaks **enemy** stealth only. Allies are
  exempt. A friendly reveal is a bug, not a feature.
- Death costs gear condition, never experience. There is no death XP loss.
- A tomb marker is visible to both factions. Griefing a revive must cost
  something for the griefer too.
- Revival costs the **revived** player 25 percent of the XP they need for
  their next level; the caster pays only mana and channel time. The cleric
  revives one ally fast, the priest revives a small group slowly.

## Rule S6 — auras anchor at the caster's feet

A circular aura anchors where the caster stood when it was cast and does not
follow. It breaks when the caster is hit. A caster must stand inside their
own aura to keep it, which is what makes the break meaningful.

## Rule S7 — the 2026 adjustments are in the design, not in the numbers

The class fixes (opener caps, cloak breaks on fire, net as peel, injury
transfer caps, shared flask) are part of the ability definition and its
notes. A class file that states a fix must also state the number that
enforces it.
