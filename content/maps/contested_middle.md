# Contested Middle — the map everyone fights over

`content/maps/contested_middle.json` — 300 m x 300 m, level band 21-60, 64 players, two factions.
Wardens (light, cool, gold) hold the south and the south-west. Ashkin (dark, warm, bronze) hold the
north and the north-east. This is the middle valley between the two starter lands: no quest, no
boss worth a night, only the three statues and the ground they stand on.

## The shape

A wide dusty bowl with 12 m of relief. The ruined keep sits on the only rise, dead centre. Four
roads run into it, four gates open in its walls, and the middle statue stands in the courtyard on
the crossing of the two main roads. The east-west road is cut in two by the keep: it stops at the
west gate and starts again at the east gate, so the middle is the only crossing on the plain.

```
                                  N
                      spawn_ashkin_n (0,+130)
                                  |
                                  |  main road 8 m
   spawn_ashkin_e (132,+44)         |         keep_tower_nw (-42,+34)
          |  6 m spur              |         keep_gate_n (0,+33)
   road_post_e -- statue_east -- keep_gate_e --+---- KEEP ----+-- keep_gate_w -- statue_west -- road_post_w
     (140,0)      (92,0)     (33,0)      |  statue_mid  |     (-33,0)       (-92,0)       (-140,0)
                                          |    (0,0)     |
   spawn_warden_w (-132,-44)              |              |         keep_tower_se (42,-34)
          |  6 m spur                    |              |         keep_gate_s (0,-33)
   road_post_w -------------------------+              |
                                  |  main road 8 m
                      spawn_warden_s (0,-130)
                                  S
```

West is left, north is up, the origin is the middle statue. Off the drawing: `fort_nw`
(-58,+78) and `fort_se` (+58,-78) in the far quarters, `pool_w` (-110,-28) and `pool_e` (+110,+28)
in the dips, `pool_nw` (-18,+76) and `pool_se` (+18,-76) flanking the main road, the two watch
towers on the outer rises, and the six pve camps.

- **The keep.** A ring of 5.5 m walls, 60 m a side, a 12 m gate on each of the four sides, and
  corner towers 14 m high (level 60). The courtyard floor is bare: no cover inside the middle
  ring. The statue stands 6 m and the wall is 5.5 m, so from the plain you cannot see the middle
  statue at all except through the four gates; only the corner towers look over the wall, and
  they see a head, not a base. The middle is alive when the gates are busy.
- **The three statues.** `statue_west` (-92, 0) and `statue_east` (92, 0) stand in the open on
  the east-west road, 12 m rings, 45 s to capture. `statue_mid` (0, 0) stands inside the keep
  courtyard, a 9 m ring, 90 s to capture. A ring of 9 m takes five standing bodies; a ring of
  12 m takes a raid. That is why the middle is slow and the sides are fast.
- **The two forts.** `fort_nw` (-58, +78) and `fort_se` (+58, -78), 16 m build plots, off every
  road. A guild buys a plot, builds a fort, and the fort is a respawn point, a shop and a
  mining claim that survives between wars. The statues reset every war; the forts do not. That
  asymmetry is the whole reason the map has forts.
- **The water.** Four teal pools in the low quarters, 18 m and 12 m across. Water is a hazard
  band, not a swim area: 40% speed, no cover, and you are visible from the rim. The west pool
  sits on the line from the Wardens' west hall to the west statue, so the cheap flank route is
  the slow one.
- **The ground.** 16 pieces of cover: the keep walls and rubble line, two low broken walls, four
  rock clusters, four wrecked carts. Cover kinds and their standing height: `keep_wall` 5.5 m
  (blocks sight), `rock` 2.8 m, `wreck` 2.4 m, `wall_low` 2.0 m, `rubble` 1.6 m (step over).
  `cover` w is the size along x, h is the size along z.
- **The pve.** Six fixed camps (beast 33, bandit 40, beast 47) in the open, by the roads and the
  buttes. They are the traffic: a camp pulls players into a lane and gives the other lane a
  free run.

## The three reads

**A lone player.** You are a scout, not a capturer. The nearest statue is 12 s away, but 45 s of
capture on an open road is an invitation, and the watchtower on the west rise looks down that
whole road. The only clean play is the swap: take the west statue, wait for the road to empty,
then walk the 92 m to the middle. The keep is the only place with walls on this map, and it has
four gates, so the moment you step into it you are seen from whichever side you came. A lone
player should feel the size of the war, not win it.

**A party of five.** Five bodies fit the 9 m middle ring. That is the whole party fight: the
keep is a party place, the side statues (12 m rings, on the road) are raid places. A party
holds the middle against a small raid and loses it to a full hall, so a party has to split —
two on the road statue, three in the courtyard — and the split is visible from the towers. The
fort changes the maths: a party with a guild fort behind it respawns 16 s from the fight
instead of walking 43 s from the hall, and comes back to a fight it can win.

**A 64-player window (32v32).** Four halls of 16, four roads, one keep. The window is four lane
fights that all meet in the middle: the north road faces the south road through the keep, the
east road faces the west road along the same axis, and every player at the middle statue sees
all four lanes at once. The side statues are lane points. The middle is the only point where the
whole map can see you. With a 26 s run and a 90 s capture, the first middle capture lands about
three minutes into the window, and that is the match. The forts decide what happens after: the
guild that built the NW fort starts every war 16 s from the north hall, the guild without it
walks 43 s from its own hall, and the statues forget everything while the forts remember.

## The sight lines

`from`/`to` are ids in the JSON (landmarks, nodes, spawns). `width_m` is the width of the clear
lane that is mutually visible, not the width of a target.

| From | To | Width | Why it hurts |
|---|---|---|---|
| `spawn_ashkin_n` | `keep_gate_n` | 10 m | the north road is a straight clear lane into the keep |
| `spawn_warden_s` | `keep_gate_s` | 10 m | the south road is the same lane, mirrored |
| `road_post_w` | `keep_gate_w` | 12 m | the west road: the west statue stands in it, seen from both ends |
| `road_post_e` | `keep_gate_e` | 12 m | the east road, mirrored |
| `keep_gate_n` | `statue_mid` | 12 m | the northern window into the middle ring |
| `keep_gate_s` | `statue_mid` | 12 m | the southern window |
| `keep_gate_w` | `statue_mid` | 12 m | the western window |
| `keep_gate_e` | `statue_mid` | 12 m | the eastern window |
| `keep_tower_nw` | `statue_west` | 8 m | the corner tower covers the west statue down the road |
| `keep_tower_se` | `statue_east` | 8 m | the mirror of that |
| `keep_tower_nw` | `statue_mid` | 6 m | the only angle into the middle that is not a gate: over the west wall, a grazing shot at the statue's head |
| `keep_tower_se` | `statue_mid` | 6 m | the same grazing shot from the other corner |
| `watchtower_w` | `statue_west` | 6 m | a broken tower on the west rise, 47 m from the statue and clear of the whole west road |
| `watchtower_e` | `statue_east` | 6 m | the mirror |

The rule behind the list: the keep's 5.5 m walls seal the courtyard at ground level, so the
middle statue has exactly four windows (the gates) and two grazing angles over the wall (the
corner towers, which clear the wall by 5 m at the statue's head). The side statues have no
walls at all, only lanes. Nothing in the JSON has cover inside a statue ring, so all three
nodes are exposed by construction.

## Travel times

Assumption: run 5.0 m/s on dry ground, 2.0 m/s in a pool. Distances are along the roads where a
road exists, otherwise straight line. If the war movement constant lands on a different number,
multiply every time by 5.0 / speed.

| From | To | Distance | Run | Note |
|---|---|---|---|---|
| `spawn_warden_s` | `keep_gate_s` | 97 m | 19 s | straight down the south road |
| `spawn_warden_s` | `statue_mid` | 130 m | 26 s | road plus the south gate |
| `spawn_warden_s` | `statue_west` | 222 m | 44 s | through the keep, out the west gate |
| `spawn_warden_s` | `fort_se` | 78 m | 16 s | the Wardens' own doorstep |
| `spawn_warden_s` | `fort_nw` | 216 m | 43 s | across the whole map, open ground |
| `spawn_warden_w` | `statue_west` | 60 m | 12 s | the pool is on the line: 22 s wading, 16 s skirting it |
| `spawn_warden_w` | `statue_mid` | 185 m | 37 s | by the west road and the west gate |
| `spawn_warden_w` | `fort_se` | 193 m | 39 s | open ground, no road touches a fort |
| `spawn_warden_w` | `fort_nw` | 143 m | 29 s | the far fort, across the north approach |
| `spawn_ashkin_n` | `statue_mid` | 130 m | 26 s | mirror of the Wardens' south road |
| `spawn_ashkin_n` | `statue_east` | 222 m | 44 s | mirror of the Wardens' west statue |
| `spawn_ashkin_n` | `fort_nw` | 78 m | 16 s | the Ashkin's own doorstep |
| `spawn_ashkin_n` | `fort_se` | 216 m | 43 s | across the whole map |
| `spawn_ashkin_e` | `statue_east` | 60 m | 12 s | the pool is on the line: 22 s wading, 16 s skirting it |
| `spawn_ashkin_e` | `statue_mid` | 185 m | 37 s | by the east road and the east gate |
| `statue_west` | `statue_mid` | 92 m | 18 s | down the west road, through the west gate |
| `statue_east` | `statue_mid` | 92 m | 18 s | down the east road, through the east gate |
| `statue_west` | `statue_east` | 184 m | 37 s | only through the keep courtyard |

The arithmetic that matters: the middle statue is 26 s away and takes 90 s to take, so a hall
reaches it at about minute three of a ten-minute window; a side statue is 12 s away and takes
45 s, so it flips fast and is worth holding only in numbers. A fort turns a 43 s walk into a
16 s respawn, and that is the only reason a guild builds one.

## For the headless test

- Capture points are the three `kind: "statue"` nodes. `capture_progress_s` is seconds,
  `radius` is metres, `contested_by` is 2 everywhere.
- The two `kind: "fort"` nodes are build plots, `capture_progress_s` 0, and they persist between
  wars. The four `kind: "gate"` nodes are lane markers, `capture_progress_s` 0. So
  "nodes with capture_progress_s > 0" and "nodes with kind statue" are the same three.
- `from`/`to` in `sight_lines` and `terrain.roads` are ids of landmarks, nodes, spawns or
  `pve_spots` in the same file. Every id in the file is unique.
- `cover` w is along x, h is along z (a footprint box). Standing height per kind is in the table
  above. No cover box touches a statue ring, so all three nodes are open.
- `terrain.water` is a hazard band: 40% speed, no cover, not a swim area.
- Palette is the world palette from `content/design/faction-war-style.md`: ochre sand `#C9A46A`,
  sepia sky `#D8C9A8`, sea teal `#3E7F7A`, fog `#B9B29C`, gold accent `#C9A227`. Capture rings
  take the faction colour: Wardens bone white `#E8E2D4` with gold, Ashkin black iron `#2B2724`
  with blood red `#7A1F1C`.
- The map is symmetric under a 180° turn about the origin, which swaps the two factions.
