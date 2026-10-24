# design rows format

Designers change numbers in a text file. No Rust edit. The game reads
the file, checks every row, and applies it as a recorded swap at the
next tick boundary. A stale or invalid file is refused and the last
good rows stay. The golden hash never moves.

## Layout

Line 1 names the table and its version:

```text
table <name> version <n>
```

`<name>` picks the table (`combat`, `ai`, `loop`). `<n>` is a whole
number and never 0. A shipped version stays frozen; a change ships as
a new version, never as an edit to an old one.

Then one row per line:

```text
<id> <key>=<int> <key>=<int> ...
```

- `<id>` names the row, e.g. `v2`.
- `<key>` is the exact field name in code. An unknown key is refused
  with its line number. A missing key is refused too.
- `<int>` is a whole number: ASCII digits only. No floats, no
  fractions, no signs (HASH-LAW: floats would break the hash).
- `#` starts a comment that runs to the end of the line. Blank lines
  are ignored.

## Units

The unit lives in the key:

- `_ticks`: fixed ticks at 60 Hz (one tick is 16.7 ms).
- `_permille`: parts per thousand on a `0..=1000` scale.
- `_mm`: millimetres. `_cm`: centimetres.

Keys without a suffix carry the unit from this page: damage is
permille, stamina costs are points, chip numerator and denominator
are plain counts. Range checks are the existing `validate_tables`
rules; the loader adds no new ranges.

Legal ranges for the `combat` table:

- `attack_damage`: `1..=1000` permille (zero damage is refused).
- Tick fields: startup `1..=60`, recovery `0..=120`, dodge
  `1..=120` with i-frames inside it, parry `0..=60` and strictly
  below startup, stagger and react `0..=120`, hit-stop `0..=60`,
  knockdown `0..=600`.
- Stamina fields: costs and regen `0..=100` points (full stamina).
- Chip: denominator `1..=16`, numerator `0..=denominator`.
- The shipped v1 row (`parry == startup == 4`) predates the strict
  rule and stays valid for replays only; a new file must keep the
  parry window below startup, so a file that copies v1 exactly is
  refused. Start from v2's numbers.

A shipped row is frozen. `v1` and `v2` in `content/design/combat.rows`
mirror `TABLES_V1` and `TABLES_V2`; change one of their numbers and the
loader refuses the whole file with that row's line number, because an
old tape must keep reading the numbers it was recorded under. A change
is a new version: bump the header, then ship the changed numbers under
a new row id (`v3`, `v4`, ...). The old rows may stay as reference.

## Frame data per move

One move reads as three numbers in ticks: `startup active recovery`.

- `startup` counts up to and including the first active tick.
  Age below startup whiffs. Age at startup is live.
- `active` counts live ticks. Steel can hit only here.
- `recovery` counts committed ticks after active. Steel is never
  live here. No cancel leaves recovery.

Each move also answers two player questions as fields:

- `avoid=<parry|dodge|block|range>`: how the player avoids it.
  One answer only. It must sit in the player's own kit.
- `punish=<ticks>`: the window to hit back. Whole ticks at
  60 Hz. It sits inside recovery, never past it.

Current tables carry the same ideas under older names:
`speed_ticks` is startup, `counter` (1 parry, 2 dodge, 3 block)
is avoid, recovery is `attack_recovery_ticks` or `endlag_ticks`.
A future per-move table uses `startup`, `active`, `recovery`,
`avoid` and `punish` directly.

`content/design/weapons.rows` version 2 carries these fields directly:
`speed_ticks` stays the startup key, and `active_ticks`, `recovery_ticks`,
`avoid` and `punish_ticks` are the frame-data fields. Its `counter` code
must read as the same answer as `avoid`, and `punish_ticks` must sit inside
`recovery_ticks`; the game test
`crates/game/tests/it/design_combat.rs::every_move_carries_frame_data`
holds both.

The same rows carry `edge_wear_permille` (`0..=1000`, BG15): the condition
the kit's steel arrives with. `Heft::edge_wear_q16`
(`crates/combat/src/heft.rs`) turns it into the dulled fraction the cut
resolver already scales a landed blow by, so a dulled gladius carries less
energy into every sever than a fresh one. The key is optional: a row that
names no wear is a fresh edge (`0`), and a blunt weapon names `0` because it
has no edge to dull.

The row's `avoid` answer has to be a window, not a wish: the parry may not
answer the whole tell of the fastest weapon in `weapons.rows`, and may answer
at most half of its own tell. Both gates are measured off tapes for the
shipped row and for the swept candidates 8 to 13 by
`crates/game/tests/it/design_combat.rs::parry_sweep_8_to_13_leaves_a_verdict_the_shipped_row_sits_on`,
with the numbers and the reason in `docs/adr/0010-parry-window.md`. A faster
weapon in this file or a shorter startup moves that verdict, so the sweep is
rerun before the number changes.

## Worked example

Readable-timing combat row (version 2). Every value passes
`validate_tables`: startup 15 sits in `12..=18`, parry 7 sits in
`6..=8` and below startup:

```text
# Readable-timing pass: slower startup, wider parry.
table combat version 2
v2 attack_damage=120 attack_startup_ticks=15 attack_recovery_ticks=8 attack_stamina_cost=25 dodge_stamina_cost=20 block_hold_cost_per_tick=1 stamina_regen_per_tick=2 dodge_duration_ticks=10 dodge_iframes_ticks=6 block_chip_num=1 block_chip_den=5 parry_window_ticks=7 parry_stagger_ticks=12 hit_react_ticks=6 hitstop_ticks=4 knockdown_ticks=30
```

The v2 row above is reference: an edit inside it is refused.

## Change one number

Copy the row you start from, give the copy the next version id, and set
the header to that same number. Here v3 is v2 with a slower light
startup (16 instead of 15); v1 and v2 stay as reference rows:

```text
# Slower light startup: v3 of the readable-timing row.
table combat version 3
v1 attack_damage=120 attack_startup_ticks=4 attack_recovery_ticks=8 attack_stamina_cost=25 dodge_stamina_cost=20 block_hold_cost_per_tick=1 stamina_regen_per_tick=2 dodge_duration_ticks=10 dodge_iframes_ticks=6 block_chip_num=1 block_chip_den=5 parry_window_ticks=4 parry_stagger_ticks=12 hit_react_ticks=6 hitstop_ticks=4 knockdown_ticks=30
v2 attack_damage=120 attack_startup_ticks=15 attack_recovery_ticks=8 attack_stamina_cost=25 dodge_stamina_cost=20 block_hold_cost_per_tick=1 stamina_regen_per_tick=2 dodge_duration_ticks=10 dodge_iframes_ticks=6 block_chip_num=1 block_chip_den=5 parry_window_ticks=7 parry_stagger_ticks=12 hit_react_ticks=6 hitstop_ticks=4 knockdown_ticks=30
v3 attack_damage=120 attack_startup_ticks=16 attack_recovery_ticks=8 attack_stamina_cost=25 dodge_stamina_cost=20 block_hold_cost_per_tick=1 stamina_regen_per_tick=2 dodge_duration_ticks=10 dodge_iframes_ticks=6 block_chip_num=1 block_chip_den=5 parry_window_ticks=7 parry_stagger_ticks=12 hit_react_ticks=6 hitstop_ticks=4 knockdown_ticks=30
```

`v3` passes `validate_tables` (startup 16 is inside `1..=60`, parry 7
stays below 16), and the header picks it. Practice mode reads the file
at the next tick boundary and swaps that row in; the swap is written to
the tape. A refused file changes nothing and the last good rows stay
live, so a typo costs a log line, not a broken bout.
