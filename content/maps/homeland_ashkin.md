# Ashkin homeland — Emberstrand Coast

Map file: `content/maps/homeland_ashkin.json`
Faction: ashkin · Size: 180 × 180 m · Levels: 1 to 20 · One portal out, to `contested_middle`
Mirror partner: `homeland_wardens` (same shape, other palette and terrain)

## The shape

One coast road. Closed at the north end, open at the south end. The road runs
down the middle of it. The keep sits in the closed end, on the black sand, so
every run back to the forge is a walk back up the same road you came down.

Six landmarks, all of them oversized masses with a job:

| id | kind | x | z | purpose |
|---|---|---|---|---|
| `ember_keep` | keep | -4 | 70 | home claim, respawn hearth, quartermaster, war-window board |
| `black_smithy` | smithy | 46 | 34 | reforge, re-edge and re-plate starter gear |
| `ash_shrine` | shrine | -46 | 26 | class blessings, hot-spring heal, tier-1 respec |
| `ember_market` | market | 40 | -26 | buy tonics and plate, sell loot, bounty board |
| `choke_ward` | barricade | 0 | -34 | the last line, narrows the road to the gate |
| `ember_gate` | gate | 0 | -64 | the only exit to the contested middle, shut outside war windows |

Terrain is volcanic coast: black sand along the east shore, basalt shelves
stepping up to a low caldera ridge inland to the north-west, ember seams glowing
in the cracks. No green, no shade, no relief from the sun. Colours are black
iron ground, ember sky, blood red haze, bronze trim.

**The mirror rule.** This map is `homeland_wardens` with every `(x, z)`
replaced by `(-x, -z)`. Same keep, same road, same barricade, same gate, same
nine camps, same distances — the other half turned around. Here the home is
`+Z` and the gate is `-Z`; there the home is `-Z` and the gate is `+Z`. Both
sides learn one shape and meet in the middle on equal footing.

The level band runs as a ladder down the coast, and the ladder is geography,
not a label:

| camp | level | where it sits |
|---|---|---|
| `camp_01_black_shallows` | 1 | 36 m from the keep, on the tide pools |
| `camp_02_ember_slopes` | 3 | 35 m from the keep, opposite shoulder |
| `camp_03_broken_harbour` | 5 | outside the smithy spur, far east |
| `camp_04_tide_hollow` | 8 | past the shrine, west wall |
| `camp_05_market_road` | 11 | below the market spur |
| `camp_06_ward_rocks` | 14 | past the barricade, west side |
| `camp_07_scoria_slope` | 17 | outside the market, far east |
| `camp_08_gate_ravine` | 19 | 44 m west of the gate |
| `camp_09_crown_rock` | 20 | 35 m east of the gate |

The camps are open world spawn clusters. No instance, no loading screen, no
door. Each one respawns on its own timer, 30 s at the low end and 90 s at the
boss end. A level 1 player can stand on the low camps and watch them refill.

## The route

The road is one spine with three spurs. It is short on purpose: 162 m from the
keep forecourt to the portal.

| from | to | m |
|---|---|---|
| keep forecourt (-4, 62) | basalt bend (14, 44) | 25.5 |
| basalt bend | smithy shoulder (30, 18) | 30.5 |
| smithy shoulder | market junction (18, -6) | 26.8 |
| market junction | barricade (0, -34) | 33.3 |
| barricade | gate (0, -64) | 30.0 |
| gate | portal (0, -80) | 16.0 |
| **keep forecourt** | **portal** | **162.1** |

Spurs, measured along the road:

- Smithy: 78.6 m from the keep.
- Shrine: 56.2 m from the keep, off the first bend.
- Market: 112.5 m from the keep, off the third bend.

The order matters. The smithy and the shrine are close enough to hit on the way
to the first camp. The market needs a real walk, so it becomes a destination
rather than a stop. The barricade is the last thing between the player and the
gate, and the two hardest camps sit past it. A level 1 player can reach the
gate in under half a minute, but the gate is not where the good mobs are.

The portal is the last 16 m of the road, under the gate arch. It opens onto
`contested_middle` and the Ashkin enter that map from its north side, next to
`spawn_ashkin_n`. This homeland lies north of the middle and the Wardens
homeland south of it, so the two coasts point at each other. Levels 1 to 20
here, 21 to 60 there: the gate is the whole level band boundary.

## What a level 1 player sees

The player spawns in the ember keep courtyard on the black sand. Three spawn
clusters sit inside the keep wall: `spawn_keep_west` and `spawn_keep_east` (6
each) and `spawn_keep_road` (4). Sixteen slots, the size of one war party.

In frame at spawn:

- The keep itself: black iron over basalt, the forge-fire pit, the war-window
  board by the door.
- The road running south along the basalt shelf, and the first bend at 25 m.
- The first camp 36 m away on the tide pools: ember hounds and the cinder
  crabs that give the market its shells.
- The ash shrine 61 m west, its ember seams already glowing through the red
  haze.

Not in frame: the gate. It is 134 m away with two bends in the way, and the
haze closes at 70 m, twenty metres closer than on the cold side. The last
sign of the war is the bronze trim on the gate, and it only shows in the last
30 m of the road.

The read is different here and that is the point. On the Wardens side the
ground is pale and the sky is pale, so you see the shape of the mountain
first. Here the ground is black and the haze is blood red, so you see the glow
first: a camp fire, a seam, the shrine. Same map, opposite first impression.

## Sight lines

- The road is the only long view on the map. Every landmark is seen from the
  road, and the road is seen from a landmark.
- From the keep, the gate is 134 m away and the bend at 25 m takes it off the
  frame before it can ever be seen from home.
- From the barricade, the gate is 30 m away and 26 m wide: it fills the frame.
  The portal behind it stays hidden until you are under the arch.
- From the gate looking back up the coast, the barricade is 30 m and the road
  behind it is still readable at 120 m, but the coast past that is gone.
- Fog is blood red and dark, and it eats the read 20 m earlier than the cold
  map does: past 70 m a landmark is a black triangle, not a building. This
  side feels tighter and closer. The mirror gives both sides the same
  distances and the same times; only the mood changes.
- The camps are on the same rule. The level 1 camp is visible from the keep
  wall at 36 m. The level 19 and 20 camps sit 44 m and 35 m to either side of
  the gate, so the last third of the road is the dangerous one. The ladder is
  geography: a level 1 player cannot even see the good ground until they have
  earned the road to it.

## Travel budget

| leg | m | run 6 m/s | walk 4 m/s |
|---|---|---|---|
| keep forecourt → portal | 162.1 | 27 s | 41 s |
| round trip | 324.2 | 54 s | 81 s |
| gate → portal | 16.0 | 3 s | 4 s |
| keep → level 1 camp | 36.1 | 6 s | 9 s |
| keep → smithy | 78.6 | 13 s | 20 s |
| keep → shrine | 56.2 | 9 s | 14 s |
| keep → market | 112.5 | 19 s | 28 s |
| keep → level 17 camp | 134.2 | 22 s | 34 s |

Straight line from the keep centre to the portal is 150.1 m, so the road costs
12 m of detour. That is the whole travel economy: 27 seconds of running
between home and the war gate, 54 seconds round trip.

In a 60-minute war window the walk is not the cost, the farming is. A player
spends the window on camps and shows up at the gate with the run already paid
for. The mirror map is the same, so a Warden and an Ashkin running the same
plan arrive within a few seconds of each other.
