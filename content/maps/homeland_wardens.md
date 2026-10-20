# Wardens homeland — Frostmarch Valley

Map file: `content/maps/homeland_wardens.json`
Faction: wardens · Size: 180 × 180 m · Levels: 1 to 20 · One portal out, to `contested_middle`
Mirror partner: `homeland_ashkin` (same shape, other palette and terrain)

## The shape

One valley. Closed at the south end, open at the north end. The road runs up
the middle of it. The keep sits in the closed end, so every run back to town is
a walk back down the same road you came up.

Six landmarks, all of them oversized masses with a job:

| id | kind | x | z | purpose |
|---|---|---|---|---|
| `warden_keep` | keep | 4 | -70 | home claim, respawn hearth, quartermaster, war-window board |
| `stone_smithy` | smithy | -46 | -34 | repair, re-plate and temper starter gear |
| `bone_shrine` | shrine | 46 | -26 | class buffs, healing water, tier-1 respec |
| `high_market` | market | -40 | 26 | buy potions and plate, sell loot, bounty board |
| `choke_barricade` | barricade | 0 | 34 | the last line, narrows the road to the gate |
| `warden_gate` | gate | 0 | 64 | the only exit to the contested middle, shut outside war windows |

Terrain is cold highland: grey granite scree, packed snow in the lee of the
walls, a frozen creek along the west wall, a low ridgeline with the gate cut
through it closing the north end. No trees above the treeline. Colours are
silver ground, bone sky, steel blue haze, gold trim.

**The mirror rule.** The Ashkin map is this map with every `(x, z)` replaced by
`(-x, -z)`. Same keep, same road, same barricade, same gate, same nine camps,
same distances — the other half turned around. Here the home is `-Z` and the
gate is `+Z`; there the home is `+Z` and the gate is `-Z`. Both sides learn one
shape and meet in the middle on equal footing.

The level band runs as a ladder up the valley, and the ladder is geography, not
a label:

| camp | level | where it sits |
|---|---|---|
| `camp_01_frozen_shallows` | 1 | 36 m from the keep, on the creek side |
| `camp_02_scree_slopes` | 3 | 35 m from the keep, opposite shoulder |
| `camp_03_ruined_watchpost` | 5 | outside the smithy spur, far west |
| `camp_04_rib_hollow` | 8 | past the shrine, east wall |
| `camp_05_market_road` | 11 | below the market spur |
| `camp_06_barricade_rocks` | 14 | past the barricade, east side |
| `camp_07_ashen_slope` | 17 | outside the market, far west |
| `camp_08_gate_ravine` | 19 | 44 m east of the gate |
| `camp_09_crown_rock` | 20 | 35 m west of the gate |

The camps are open world spawn clusters. No instance, no loading screen, no
door. Each one respawns on its own timer, 30 s at the low end and 90 s at the
boss end. A level 1 player can stand on the low camps and watch them refill.

## The route

The road is one spine with three spurs. It is short on purpose: 162 m from the
keep forecourt to the portal.

| from | to | m |
|---|---|---|
| keep forecourt (4, -62) | scree bend (-14, -44) | 25.5 |
| scree bend | smithy shoulder (-30, -18) | 30.5 |
| smithy shoulder | market junction (-18, 6) | 26.8 |
| market junction | barricade (0, 34) | 33.3 |
| barricade | gate (0, 64) | 30.0 |
| gate | portal (0, 80) | 16.0 |
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
`contested_middle` and the Wardens enter that map from its south side, next to
`spawn_warden_s`. This homeland lies south of the middle and the Ashkin
homeland north of it, so the two valleys point at each other. Levels 1 to 20
here, 21 to 60 there: the gate is the whole level band boundary.

## What a level 1 player sees

The player spawns in the keep courtyard. Three spawn clusters sit inside the
keep wall: `spawn_keep_west` and `spawn_keep_east` (6 each) and
`spawn_keep_road` (4). Sixteen slots, the size of one war party.

In frame at spawn:

- The keep itself: squared stone, silver plate roof, a bone-white brazier at
  the hearth, the war-window board by the door.
- The road going north, and the first bend at 25 m.
- The first camp 36 m away on the creek side: hounds and goats on the ice.
- The shrine's bone spires 61 m east, already softened by the haze.

Not in frame: the gate. It is 134 m away with two bends in the way, and the
haze closes at 90 m. The first sign of the war is gold trim, and it only shows
in the last 30 m of the road. A new player learns where the gate is by
walking there, not by looking for it.

## Sight lines

- The road is the only long view on the map. Every landmark is seen from the
  road, and the road is seen from a landmark.
- From the keep, the gate is 134 m away and the bend at 25 m takes it off the
  frame before it can ever be seen from home.
- From the barricade, the gate is 30 m away and 26 m wide: it fills the frame.
  The portal behind it stays hidden until you are under the arch.
- From the gate looking back down the valley, the barricade is 30 m and the
  road behind it is still readable at 120 m, but the valley past that is gone.
- Fog is steel blue and heavy. Past 90 m a landmark is a flat silhouette, not a
  building. You navigate by silhouette and by the road.
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
