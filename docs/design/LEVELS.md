# LEVELS — Rome arena use per actor (function before form)

## Scale first

Fighter height 1800mm (1.8 m, eye 170cm), blade reach 1500mm
(`crates/arena/src/match_wire.rs:33`), dodge 10 ticks (`content/design/combat.rows:9`).
No geometry below means anything until these three hold: a fighter must fit under
the gate (500cm), be hit inside 2000mm of a zone centre, and strike inside 1500mm.

Function before form: every actor below names its use first, then one past
event (cause and result, two sentences), then its goal.

## Zone circle

Start at centre (0,0) r 250cm, the holdable dominance point one fighter can
hold alone. Entrance at gate mouth (1000,0) r 200cm, the curtain risk point on
the floor edge. One way back through the ring: from the gate the fighter walks
the open sand to the centre and back, 8 m spawn to spawn.

## Advantage points and their heights

The ring's reachable advantage points are `centre` (r 250cm), `crate_a`
(100cm) and `debris_field` (100cm): the crate and the mound stand at the same
height since the 2026-10-17 cover-form move (the mound was 150 cm, see the delta
below), so neither spawn starts the bout behind the taller cover. All three
stand on the 10 m duel floor. `stands` at 300cm is the tallest silhouette on the
layout, but it is the crowd seating 20 m out, off the floor, so the fight never
reaches it and it carries no advantage flag. Walls at 300cm bound the bowl north
and south. Gate tunnel at 500cm is the tallest mouth. Nothing else is raised:
torches 200cm poles.

## Actors (one line each)

sand_floor: open sand use, the footing every bout is fought on. Past event: the first bout spilled blood here and the crowd roared, so the sand was raked fresh. Goal: learn.
stands: crowd magnet use, raised seats that watch and pressure the bowl. Past event: the crowd once threw bread at a fleeing recruit and he slipped, so stewards now net the front row. Goal: explore.
wall_north: north bound use, stops fighters and blades leaving the bowl. Past event: a charge once dented the north boards and the bout paused, so the crew bolted iron strips behind them. Goal: learn.
wall_south: south bound use, mirrors the north wall for the bowl edge. Past event: a shield slid under the south gap and cut a horn-blower's foot, so the gap was sealed with sandbags. Goal: learn.
gate_tunnel: sole entrance use, the curtain every fighter walks through. Past event: the gate jammed shut once and the bout waited, so the crew oils it nightly. Goal: explore.
torch_west: west light use, marks the gate flank after dark. Past event: the west flame blew out mid-bout and the flank went dark, so the crew doubled the oil cups. Goal: unknown place.
torch_east: east light use, marks the gate flank after dark. Past event: the east flame spat oil and burned a banner corner, so the wick was trimmed short. Goal: unknown place.
banner_west: west mark use, cloth that calls the crowd side. Past event: the west banner tore loose and covered a fighter's head, so the tie-ropes were doubled. Goal: explore.
banner_east: east mark use, cloth that calls the crowd side. Past event: the east banner faded in the sun and looked grey on capture, so the dyer re-dipped it red. Goal: explore.
crate_a: ring-band cover use, the bumper that starts in spawn A's own quarter, inside the ring band 7.07 m out that blocks swords and sight. Past event: a recruit hid behind it and lived, so veterans kick it aside first. Goal: unknown place.
debris_field: ring-band cover use, a mound inside the ring band 7.07 m out that slows feet and breaks lines, standing at the crate's 100 cm so both spawns start beside cover of the same height. Past event: a champion tripped on rubble and lost the bout, so sweepers leave half the stones. Goal: moral choice.

## Cover and the entrance (2026-10-17)

The duel floor is the fight ring (10 m half-extent around the origin). Both
"cover" objects now stand inside it: `crate_a` at (-5,-5) and `debris_field` at
(5,5), mirrored across the duel axis x, both on the ring band 7.07 m out, clear
of the 3 m centre circle and of the walls, and each one inside one spawn's own
quarter (spawn A (-4,0) west, spawn B (4,0) east). Both spawns reach a bumper in
510 cm (3.0 s at the 170 cm/s march) **and at the same height** (100 cm each
since the 2026-10-17 cover-form move), and the gate fails a layout where one side
starts the bout uncovered or starts it behind the taller cover. Before the
crate_a move both bumpers stood at x +5 m, so spawn A walked 1030 cm (6.1 s) to
cover while spawn B stood 510 cm from it, and this file's "neither spawn owns the
only cover" line was false.

The entrance now sits on the floor edge: `gate_tunnel` centre is x 10 m, so the
tunnel mouth touches the 10 m duel floor. `entrance_share` 35 permille of the
1600 floor cells read the gate's 3 m radius (8 while the centre still sat 2 m
outside), so a fighter who crosses the mouth reads the one curtain risk point
(`gate_tunnel class=2 ... risk=1`) -- the entrance problem from Worlds II.6. The
mouth grazes the ring band at x 7..8, z +-1.5, so a fighter who walks the ring
passes through it. A further move is a later run's one change.

Measured: `possible_death` 104 -> 156 (`crate_a` move) -> 208 (`debris_field`
move) -> 234 (entrance move); `entrance_share` 8 -> 35 with the entrance move
and `walk_s` 4.7, `cover 2 bumpers`, `no_use 0`, `physical_unread 0` unchanged.

## Rules that make a flag fail (2026-10-17, level check second rule pass)

Two flags used to be claims the gate never tested, and both false passes were
real poses in this file's own history; a third flag (`risk`) was read by nothing
at all (measured 2026-10-17):

- Cover off the duel floor. `debris_field` sat at (34,0) for three days while the
  check printed `cover 2 bumpers` and exit 0; the fight never walks out there, so
  that bumper was dressing. Now every class=4 row must stand on the 10 m floor
  (`|cx|,|cz| <= 1000`): `cover crate_a stands outside the duel floor at
  (3400,0): the fight never reaches it`, exit 1.
- A dominance flag no fighter can hold. A `dominance=1` row on the 15 m
  `sand_floor` passed, though two fighters stand outside each other's 1500mm
  reach across it. Now a dominance row must sit on the floor and its radius must
  stay inside the hold circle (the tool's own cap `DOMINANCE_HOLD_RADIUS_CM`
  300 cm; the engine blade reach is 1500mm, `crates/arena/src/match_wire.rs:33`):
  `dominance sand_floor is not holdable alone (radius_cm 1500 above cap 300 or
  centre off the duel floor)`, exit 1.
- A risk flag no rule read. The fourth column (`risk`) was read by nothing:
  seeding `risk=1` on the live 60 m `sand_floor` row printed `failures: []` and
  exit 0 while the fight never walks 60 m out (Worlds II.6: a risk point the
  fight cannot reach is dead space). Now every `risk=1` row must stand on the
  10 m floor (`|cx|,|cz| <= 1000`): `risk sand_floor stands outside the duel
  floor at (-6000,0): the fight never reaches it`, exit 1. No minimum
  risk-point count: the director's bar list names none, and the entrance risk
  point is already named above ("Zone circle"), so a minimum would fail a legal
  flat layout.

Live rows already pass all three: `centre` holds r 250 cm on the floor, both bumpers
stand at (-5,-5) and (5,5). Check now: walk_s 4.7, cover 2 bumpers,
entrance_share 35, no_use 0, physical_unread 0, deaths_per_zone none (no tapes),
possible_death 234, exit 0. `tools/level-check.test.mjs` seeds the off-floor
pose and the unholdable flag (that fixture now also reads both spawns uncovered,
because an off-floor bumper is no cover), a boundary case at the 300 cm cap and
the cover-reach pair; 24 tests pass. One older test fixture moved its
`dominance=1` from the 15 m `sand_floor` row to `centre` (its assertion,
`failures == []`, is unchanged) because the new rule makes the old fixture
illegal.

## Deltas

2026-10-12: zones file created for all 11 placement actors plus centre; no
placement change (git log -3 -- content/arena: cff1c4d backup 2026-10-10,
2f02202 backup 2026-10-09, 419c0fef content land). Check before: walk_s 4.7,
cover 0, entrance_share 10, no_use 11, physical_unread 11, FAIL zones/cover/
dominance/use/scale. Check after: see `node tools/level-check.mjs` output.

2026-10-14: no placement change since (placement.json last touched
a8c911ab 2026-10-12 backup; content/arena log 4d88a021/fbb43577/576cdc97
backups only). Check now: walk_s 4.7, cover 2 bumpers, entrance_share 8,
no_use 0, physical_unread 0, deaths_per_zone none (no tapes),
possible_death 104, exit 0.

2026-10-17: still no placement change (placement.json last touched a8c911ab
2026-10-12; newest content/arena commits cbd4c007/ea1aa5a4/8a059133 are
backups). Check now: walk_s 4.7, cover 2 bumpers, entrance_share 8, no_use 0,
physical_unread 0, deaths_per_zone none (no tapes), possible_death 104,
exit 0. Two findings this run:
- Generator drift: `content/arena/tools/gen.mjs` still lists wall_north /
  wall_south at z -8 / +8 while the shipped placement.json has -10 / +10, so
  a regen would pull both walls 2 m inward with no layout check in the way
  (torch rows differ only in float formatting). The regen also refuses while
  CC0 `*_2k.jpg` drops sit in content/arena/textures, so placement.json is
  edited in lockstep, never regenerated.
- New fail rule: a zone row named after an actor must carry that actor's
  placement centre within 50 cm. Before it, a moved actor with a stale row
  passed every rule silent; now it prints `zone row crate_a centre (2900,0)
  disagrees with placement (500,-500)` and exits 1.

2026-10-17 crate move: `crate_a` moved from (29,0) to (5,-5) — ring band
(7.07 m out), clear of the 3 m centre circle, both spawns and the north wall.
`content/arena/placement.json`, `content/arena/tools/gen.mjs` and the
`crate_a` zone row moved together (both `include_str!` sites read the one
placement.json; the generator was not run, see the drift note above). Check
now: walk_s 4.7, cover 2 bumpers, entrance_share 8, no_use 0, physical_unread
0, deaths_per_zone none (no tapes), possible_death 156, exit 0. `debris_field`
and the entrance are unchanged; the staged_fight receipt and PNG digests are
byte-identical (the live bout steps presentation plus the kit collision world
only, so a crate move is not in the receipt).

2026-10-17 debris move: `debris_field` moved from (34,0) to (5,5) — the mirror
of `crate_a` across the duel axis x, ring band (7.07 m out), clear of the 3 m
centre circle, both spawns and the walls. `content/arena/placement.json`,
`content/arena/tools/gen.mjs` and the `debris_field` zone row moved together
(both `include_str!` sites read the one placement.json; the generator was not
run, see the drift note above). The zone row keeps class 4, radius 125, height
50 and risk 1, and gains advantage 1 to match `crate_a`. Check now: walk_s 4.7,
cover 2 bumpers, entrance_share 8, no_use 0, physical_unread 0, deaths_per_zone
none (no tapes), possible_death 208, exit 0. The entrance is unchanged; the
staged_fight receipt and PNG digests are byte-identical (the live bout steps
presentation plus the kit collision world only).

2026-10-17 entrance move: `gate_tunnel` moved from (12,0) to (10,0) — the tunnel
mouth now stands on the floor edge, so the way in crosses the duel floor instead
of ending 2 m outside it on a strip no fighter walks.
`content/arena/placement.json`, `content/arena/tools/gen.mjs` and the
`gate_tunnel` zone row moved together (both `include_str!` sites read the one
placement.json; the generator was not run, see the drift note above). The row
keeps class 2, cz 0, radius 200, height 500, advantage 0, dominance 0 and risk 1,
so the gate stays the one curtain risk point. Check now: walk_s 4.7, cover 2
bumpers, entrance_share 35, no_use 0, physical_unread 0, deaths_per_zone none
(no tapes), possible_death 234, exit 0.
`tools/level-check.test.mjs` pins entrance_share to the 25..60 band and
possible_death 234, so a later move cannot return the entrance to single digits
silent. The staged_fight receipt and PNG digests are byte-identical before and
after the move (the live bout steps presentation plus the kit collision world
only; no fighter path reaches the gate mouth in 600 ticks).

2026-10-17 advantage flag: `stands` `advantage=1` -> `advantage=0`. The flag was
set on a row 20 m out, 10 m outside the 10 m duel floor, and nothing tested it:
the gate printed exit 0 while no fighter in the ring could ever reach the
stands. Now every `advantage=1` row must stand on the floor
(`|cx|,|cz| <= 1000`) and at least one must, else `advantage stands stands
outside the duel floor at (-2000,0): the fight never reaches it` or `no
advantage point inside the duel floor`, exit 1. No cell but `advantage` moved:
class 1, centre (-2000,0), radius 600, height 300, dominance 0 and risk 0 all
stay (the stands is the crowd magnet, not an advantage point, and it is 12x6 m
seating whose pose is generated, so moving it onto the floor is a placement
packet). The three reachable advantage points (`centre`, `crate_a`,
`debris_field`) stand on the floor before and after. No Rust code reads the
flag, so no sim byte moves. Measured with the rule in and the row unchanged:
exit 1 with that line; after the row fix: walk_s 4.7, cover 2 bumpers,
entrance_share 35, no_use 0, physical_unread 0, deaths_per_zone none (no tapes),
possible_death 234, exit 0. `tools/level-check.test.mjs` seeds an off-floor
`advantage=1` row, a floor-edge row at exactly cx 1000 and a floor with no
advantage row at all.

2026-10-17 debris raise: `debris_field` raised from a 50 cm rubble scatter to a
150 cm mound (`y` 0.25 -> 0.75, `sy` 0.5 -> 1.5), so the ring's raised
advantage point is reachable: `crate_a` (100cm) is flat single-crate cover and
`stands` at 300cm is the crowd seating 20 m out, 10 m outside the 10 m duel
floor, where the fight never reaches it (Worlds II.6: an advantage point the
fight cannot reach is dead space). `content/arena/placement.json`,
`content/arena/tools/gen.mjs` and the `debris_field` zone row (`height_cm` 50 ->
150) moved together (both `include_str!` sites read the one placement.json; the
generator was not run, see the drift note above). Centre (5,5) and the 2.5 m
footprint stay, so the collision world is byte-identical (the kit keeps x/z
footprints only, no Y) and the bout receipt and PNG digests do not move. New
gate rule: a zone row named after a placement actor must carry that actor's box
height (`sy` in cm) within 5 cm, and an actor with no numeric `sy` fails loud.
Measured before (flat rubble, row 50): walk_s 4.7, cover 2 bumpers,
entrance_share 35, no_use 0, physical_unread 0, deaths_per_zone none (no tapes),
possible_death 234, exit 0; with the mound in and the row stale: `zone row
debris_field height_cm 50 disagrees with placement box height 150`, exit 1;
after the row fix: the same seven lines, exit 0. `tools/level-check.test.mjs`
seeds the displaced-height claim (400 cm over the shipped 100 cm box) and the
5 cm tolerance edge; 20 tests pass.

2026-10-17 spawn cover fairness: `crate_a` moved from (5,-5) to (-5,-5), the
mirror of `debris_field` across the duel axis x, so each spawn starts 510 cm
(3.0 s) from a bumper. Before the move both bumpers stood at x +5 m, so spawn A
(-4,0) walked 1030 cm (6.1 s) to cover while spawn B (4,0) stood 510 cm from it:
the fighter who started beside cover dominated the first exchange (Worlds II.6),
and this file's "neither spawn owns the only cover" line was false.
`content/arena/placement.json`, `content/arena/tools/gen.mjs` and the `crate_a`
zone row moved together (both `include_str!` sites read the one placement.json;
the generator was not run, see the drift note above). The row keeps class 4,
cz -500, radius 100, height 100, advantage 1, dominance 0 and risk 0. New gate
rule: every spawn must have a floor bumper inside `COVER_REACH_CM` 600 cm (3.5 s
at the march), else `cover spawn A at (-400,0) has no bumper within 600 cm
(nearest 1030 cm): one side starts the bout uncovered`, exit 1 (spawn B read
510 cm, so only A failed). Measured with the rule in and `crate_a` still at
(5,-5): exactly that one line, exit 1; after the move: walk_s 4.7, cover 2
bumpers, entrance_share 35, no_use 0, physical_unread 0, deaths_per_zone none
(no tapes), possible_death 234, exit 0, both spawns 510 cm. `possible_death`
holds at 234 because the floor grid is symmetric about both axes, so a mirrored
bumper covers the same cells. `tools/level-check.test.mjs` seeds the one-sided
pair, the 600/601 cm reach edge and the moved 510/510 pair; 24 tests pass.

2026-10-17 cover form: `debris_field` lowered from a 150 cm mound to the crate's
100 cm (`y` 0.75 -> 0.5, `sy` 1.5 -> 1, and the zone row `height_cm` 150 -> 100
with it), so the two spawns start the bout beside cover of the same height.
Before the move both spawns reached a bumper in 510 cm (3.0 s), but spawn B
reached a 150 cm mound while spawn A reached a 100 cm cube: 50 cm of extra
silhouette on B's side is a first-exchange advantage (Worlds II.6), and no rule
read a bumper's height, so the gate printed exit 0 while this file claimed both
spawns started fair. `content/arena/placement.json`, `content/arena/tools/gen.mjs`
and the `debris_field` zone row moved together (both `include_str!` sites read
the one placement.json; the generator was not run, see the drift note above). The
row keeps class 4, centre (500,500), radius 125, advantage 1, dominance 0 and
risk 1. The change moves no centre, no radius and no collision footprint (the kit
keeps x/z footprints only, no Y), so every counted number holds and the seven
lines are byte-identical before and after: `walk_s 4.7`, `cover 2 bumpers`,
`entrance_share 35`, `no_use 0`, `physical_unread 0`, `deaths_per_zone none (no
tapes)`, `possible_death 234`, exit 0. New gate rule: the two spawns' nearest
floor bumper must stand within `HEIGHT_TOLERANCE_CM` 5 cm of each other, else
`cover spawn A starts beside crate_a (100 cm) while spawn B starts beside
debris_field (150 cm): one side starts the bout with better cover`, exit 1.
Measured with the rule in and the mound still at 150 cm: exactly that one line,
exit 1 (both spawns were 510 cm from their own bumper, so the reach rule was
quiet and the form rule was the only failure; the tool's seven printed lines were
already identical then). After the move: the same seven lines, exit 0, with
`nearestCoverRow` reading `crate_a` 510/100 for spawn A and `debris_field`
510/100 for spawn B. A spawn with no floor bumper adds no form line: the reach
rule already fails that spawn. `tools/level-check.test.mjs` freezes the unequal
pair, seeds the equal pair, the 5 cm cover-form edge and the off-floor one-sided
pair; 27 tests pass.

2026-10-17 risk flag: the fourth column (`risk`) is now read, the last of the
four zone flags to become a claim the gate can fail. It was read by nothing:
seeding `risk=1` on the live 60 m `sand_floor` row printed `failures: []` and
exit 0 -- the exact false claim the advantage rule closed for `stands`
(Worlds II.6: a risk point the fight cannot reach is dead space). New gate rule:
every `risk=1` row must stand on the 10 m duel floor (`|cx|,|cz| <= 1000`), else
`risk <id> stands outside the duel floor at (<cx>,<cz>): the fight never reaches
it`, exit 1. No cell moved: the only two live risk rows already stand on the
floor, `gate_tunnel` on the inclusive edge (1000,0) and `debris_field` at
(500,500), so `node tools/level-check.mjs` prints the same seven lines, exit 0:
`walk_s 4.7`, `cover 2 bumpers`, `entrance_share 35`, `no_use 0`,
`physical_unread 0`, `deaths_per_zone none (no tapes)`, `possible_death 234`.
Measured with the rule in: the seeded layout prints exactly one line,
`rule: risk sand_floor stands outside the duel floor at (-6000,0): the fight
never reaches it`, exit 1; before the rule the same layout printed `failures: []`,
exit 0. No minimum risk-point count: the director's bar list names none and the
entrance risk point is already named ("Zone circle"), so a minimum would fail a
legal flat layout. No Rust file, no centre, radius, class, advantage or dominance
cell moves, so the bout receipt cannot move (the kit reads x/z footprints only)
and the seven lines stay byte-identical. `tools/level-check.test.mjs` seeds the
off-floor row and the floor edge at exactly cx 1000, and the live-file test names
the two rows (`gate_tunnel`, `debris_field`) and asserts no other row carries the
flag; 29 tests pass.

## Tapes

No tournament tapes yet, so no deaths_per_zone heatmap. No-play
possible_death counts floor cells within 2000mm of a zone centre; a zone
that can never see one is dead space.
