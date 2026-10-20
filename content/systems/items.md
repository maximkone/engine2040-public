# Items — the arithmetic

The machine-readable file is `items.json`. This page is the rule book for it.
Every number an engine needs is in the file. This page explains how the
numbers combine.

Source: `docs/research/ryl1-2026-10-20/dossier.md` sections 4 and 10. The
mechanics are re-derived. The names, the numbers and the item lists are ours.

## 1. The one shape

An item is: a template, a level band, a metal, an upgrade level, a roll
percent per stat, a set id, a condition pool, and a list of socketed gems.

The template says the slot (`weapon`, `offhand`, `head`, `chest`, `gloves`,
`boots`, `ring`, `neck`) and the primary stat. The primary stat is the stat
that decides the item's grade.

## 2. The F base of an item

The F base is what a plain F item of that template and level carries.

```
base0 = round(slot_base[slot][stat] * template.base_mult * level_band.base_mult)
base  = round(base0 * metal_multiplier) + template.base_bonus[stat]
```

- `slot_base` is in `f_base.slot_base`. It is the level 1 number.
- `level_band.base_mult` scales it with level: L1 x1.0, L2 x2.0, L3 x3.5, L4 x5.0.
- `metal_multiplier = 1 + 0.08 * (metal_quality - 1)`. Iron is x1.00, cobalt is x1.40.
- `template.base_bonus` is a flat extra, 0 for every template today.

Level bands: L1 levels 1-15, L2 levels 16-35, L3 levels 36-60, L4 levels 61-100.

The metal scales the base, so it moves the numbers and not the grade.

## 3. The grade

Every stat gets its own grade. The item's grade is the grade of its primary
stat. The band table is in `grade_bands` and its unit is **percent above the F
base of that same stat**.

```
pct  = ceil(100 * (value / base_adj - 1))
base_adj = base * (1 + 0.12 * upgrade_level)
grade = the band with min <= pct <= max
```

`ceil` means any value above the base counts as at least +1 percent. The
bands tile 0..999 with no gap:

| grade | percent above the F base |
|-------|--------------------------|
| F     | 0                        |
| D     | 1 - 24                   |
| C     | 25 - 49                  |
| B     | 50 - 79                  |
| A     | 80 - 119                 |
| AA    | 120 - 169                |
| AAA   | 170 - 999                |

Upgrades multiply the stats and the base by the same factor, so an upgrade
never changes the grade. Only the roll and the metal decide that.

Values are whole numbers. A value recomputed from a rounded number can land
one band off when it sits exactly on an edge, so an item keeps its stored roll
percent as the authority.

## 4. Worked example: a 34 STR item

A short sword dropped in the south valley at level band L2. Iron, +0.

```
base0 = round(10 * 1.0 * 2.0) = 20       # slot_base.weapon.str, template.base_mult, L2 mult
base  = round(20 * 1.00) = 20            # iron is quality 1
pct   = ceil(100 * (34 / 20 - 1)) = ceil(70) = 70
grade = 70 is inside 50..79  ->  B
```

So a 34 STR L2 short sword is a **B weapon**. The STR of this sword can run
from F (0 percent over the base) to AAA (170 percent), so the roll alone
decides the grade.

Two checks:

- The same blade at +5: `base_adj = 20 * 1.60 = 32`, `value = round(32 * 1.70) = 54`,
  `pct = ceil(100 * (54 / 32 - 1)) = 69` -> still **B**.
- The same blade in steel: `base = round(20 * 1.16) = 23`, the same 70 percent roll
  gives `value = round(23 * 1.70) = 39` STR, and the grade is still **B**.

## 5. Upgrades

The cap is +5. After +5 the sockets open. There is no +6.

```
cost = round((100 + 40 * (target_level - 1)) * level_band.upgrade_cost_mult)
```

The metal of the item is spent too: one unit per attempt, so refining iron
burns iron.

The rates are a table, `upgrades.level_table`. One row per target level.

| target | coin (L1) | success | downgrade on failure | max endurance burned |
|--------|-----------|---------|----------------------|----------------------|
| +1     | 100       | 0.95    | 0.35                 | 1 - 3                |
| +2     | 140       | 0.90    | 0.35                 | 1 - 4                |
| +3     | 180       | 0.82    | 0.32                 | 2 - 5                |
| +4     | 220       | 0.70    | 0.28                 | 3 - 7                |
| +5     | 260       | 0.55    | 0.20                 | 5 - 10               |

The band multiplies the coin: L2 items pay x1.5, L3 x2.2, L4 x3.0.

`upgrades.failure` holds the defaults and the caps: downgrade chance 0.35,
endurance loss 1 to 10. A level row may lower them, never raise them. The
effective downgrade chance is `min(row, 0.35)` and the loss range is the row
range clamped to 1..10.

A failed attempt always burns max endurance. The burn is permanent: max
endurance drops by the rolled number and the endurance pool is clamped to the
new max. The coin and the metal are gone either way.

## 6. Worked example: an item at +4 fails

A +4 L2 blade tries for +5.

```
cost   = round(260 * 1.5) = 390 coin, plus one unit of the item's metal
success = 0.55   -> failure = 0.45
on failure: downgrade chance = min(0.20, 0.35) = 0.20
            max endurance burned = a whole number from 5 to 10
```

So of every attempt:

- 55 percent succeed and the blade is +5.
- 45 percent fail and the blade stays at +4, minus 5 to 10 max endurance.
- 9 percent of all attempts (0.45 * 0.20) also drop the blade to +3, minus the
  same 5 to 10 max endurance.

The grade does not move. The blade is still the B blade from section 4, just
with less endurance left to lose. At +5 it opens its sockets: a B blade gets
one socket.

## 7. Sockets and gems

- Sockets open at +5 only.
- Count by grade: F and D get none, C and B get one, A and AA get two, AAA gets three.
- Three is the hard cap, `sockets.per_item_max`.
- A socket holds one gem. One gem per socket, no sharing.
- A socket takes a gem of size 1 up to the level band capacity: L1 takes size 1,
  L2 size 2, L3 size 3, L4 size 4.
- Fourteen gem types in four sizes. `sockets.gem_types`, one value per size.
- Flat gems add a whole number to a stat. Percent gems add a percentage of the
  wearer's stat. Four gems are special: attack percent, hit points, crit
  percent, drop percent.
- A gem can be pulled out for free. The socket stays empty for good.

## 8. Metals

Six metals in one order: iron < bronze < steel < silver < gold < cobalt.
The `quality` field is the order, 1 to 6.

- The metal scales the base: x1.00, x1.08, x1.16, x1.24, x1.32, x1.40.
- The metal adds max endurance: +10 per step above iron.
- The metal is the fuel for upgrades. One unit per attempt, the item's own metal.
- The metal is chosen when the item is forged. It never changes after that.

## 9. Sets

`sets` names the members. `set_bonuses` gives the bonus per piece count. The
row with the highest piece count the wearer satisfies wins, and the bonus
sits on top of the item stats. It is not a grade and it does not change one.

## 10. Drops and the monster table

`drops` is a weighted table. Rows are grouped by source and level band, and the
weights of one group add up to 100. A weight is a percent.

To build a monster table: take the group for that source and that level band,
pick one row by weight, then roll that row's grade into a roll percent for
every stat of the item. The primary stat rolls in the top third of the band,
the other stats roll anywhere in the band.

```
value = round(base_adj * (1 + pct / 100))
```

Sources: `south_valley` and `north_valley` are the two starter valleys,
`central_basin` is the contested middle, `open_boss` is the open world.

## 11. Condition

Every item has a condition pool, `condition.max_endurance_by_slot`, scaled by
the level band and by the metal.

- On death every equipped item loses 10 percent of its max endurance, at least
  1. No experience is lost. Death costs gear, not the evening.
- When the pool hits zero the item breaks: max endurance drops by 1 for good
  and the pool is set to max endurance minus 1.
- A broken item is repaired at a smith for 30 percent of its max endurance.
  The permanent loss never comes back.

## 12. Open numbers

These are ours, not from the source. They can move.

- The band edges. 0, 24, 49, 79, 119, 169, 999.
- The success and downgrade rates per level.
- The coin costs and the metal prices.
- The F base of every slot.
- The gem values per size.
- The drop weights per group.
- The set bonuses.
- The condition numbers.
