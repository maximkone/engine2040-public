# LEVELS — Rome arena use per actor (function before form)

## Scale first

Fighter height 1800mm (1.8 m, eye 170cm), sword reach 2000mm (200 cm, rome-spec),
dodge distance 2m. No geometry below means anything until these three hold:
a fighter must fit under the gate (500cm), be hit inside 2000mm of a zone
centre, and clear a bumper with one 2m dodge.

Function before form: every actor below names its use first, then one past
event (cause and result, two sentences), then its goal.

## Zone circle

Start at centre (0,0) r 250cm, the holdable dominance point one fighter can
hold alone. Entrance at gate mouth (1200,0) r 200cm, the curtain risk point.
One way back through the ring: from the gate the fighter walks the open sand
to the centre and back, 8 m spawn to spawn.

## Raised point

Stands at height 300cm above the sand, the only raised advantage point. Walls
at 300cm bound the bowl north and south. Gate tunnel at 500cm is the tallest
mouth. Nothing else is raised: crate 100cm, debris 50cm, torches 200cm poles.

## Actors (one line each)

sand_floor: open sand use, the footing every bout is fought on. Past event: the first bout spilled blood here and the crowd roared, so the sand was raked fresh. Goal: learn.
stands: crowd advantage use, raised seats that watch and pressure the bowl. Past event: the crowd once threw bread at a fleeing recruit and he slipped, so stewards now net the front row. Goal: explore.
wall_north: north bound use, stops fighters and blades leaving the bowl. Past event: a charge once dented the north boards and the bout paused, so the crew bolted iron strips behind them. Goal: learn.
wall_south: south bound use, mirrors the north wall for the bowl edge. Past event: a shield slid under the south gap and cut a horn-blower's foot, so the gap was sealed with sandbags. Goal: learn.
gate_tunnel: sole entrance use, the curtain every fighter walks through. Past event: the gate jammed shut once and the bout waited, so the crew oils it nightly. Goal: explore.
torch_west: west light use, marks the gate flank after dark. Past event: the west flame blew out mid-bout and the flank went dark, so the crew doubled the oil cups. Goal: unknown place.
torch_east: east light use, marks the gate flank after dark. Past event: the east flame spat oil and burned a banner corner, so the wick was trimmed short. Goal: unknown place.
banner_west: west mark use, cloth that calls the crowd side. Past event: the west banner tore loose and covered a fighter's head, so the tie-ropes were doubled. Goal: explore.
banner_east: east mark use, cloth that calls the crowd side. Past event: the east banner faded in the sun and looked grey on capture, so the dyer re-dipped it red. Goal: explore.
crate_a: only cover use, the bumper that blocks swords and sight. Past event: a recruit hid behind it and lived, so veterans kick it aside first. Goal: unknown place.
debris_field: rough risk use, the second bumper that slows feet and breaks lines. Past event: a champion tripped on rubble and lost the bout, so sweepers leave half the stones. Goal: moral choice.

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

## Tapes

No tournament tapes yet, so no deaths_per_zone heatmap. No-play
possible_death counts floor cells within 2000mm of a zone centre; a zone
that can never see one is dead space.
