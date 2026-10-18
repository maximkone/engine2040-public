# CT78 urban-war pilot scout (CT9 kit)

Row: CT78 content. Tier B. Size S. Owns `content/maps/PILOT-SCOUT.md`.
The CT9 kit is frozen: nothing here adds, moves or edits a file.
Needs: CT9. Consumers: CT56 (pilot cook), UW39.

## The pick

One variant and one corner, all from the frozen CT9 district kit (no new geometry):

| variant | corner | actors (asset) |
| --- | --- | --- |
| `district-corner` | `district-corner-01` | `insula_a` (insula), `stall_fish` (market_stall), `city_wall` (street_wall) |

Files: `content/maps/urban-war-pilot/variant.json` and `corner.json`. The three
actors keep the coordinates and sizes they have in `placement_district.json`.

## Probes: 0 findings

- Spatial: the SPF1 known-good gate runs the full engine-spatial check catalog
  with a flat nav view over `corner.json` and pins `corner actors=3
  verdict=clean` (test `known_good_shipped_scenes_pin_zero_red_and_the_yellow_list`
  in `crates/playtest/src/spatial_eval/known_good.rs`). Run on the runner
  (`cargo test --locked -p engine-playtest --lib known_good`): PASS, run 39104843792.
- Cook: `node content/maps/urban-war-pilot/cook-corner.mjs` resolves the three
  actors against the frozen CT9 cook: 3 of 3 hulls contain their mesh, LOD0
  triangles and cook digests equal the rows in `content/maps/report.json`,
  every asset is under the prop bar of 5,000 triangles (worst: insula, 2,112).

## Bytes and VRAM guess

| item | number |
| --- | --- |
| glTF source bytes, 3 actors | 384147 |
| LOD0 triangles, 3 actors | 5376 |
| albedo maps bound (plaster for insula, canvas for market_stall, stone for street_wall), 512x512 each | 3 x 1398101 |
| shared data maps (flat_normal, metal_roughness), 256x256 each | 2 x 349525 |
| texture VRAM guess, RGBA8 plus full mips (upper bound, no block compression) | 4893353 bytes (4.67 MiB) |

The guess is `floor(w * h * 4 * 4 / 3)` per map, the same bound `tools/arena-report.mjs`
uses. The potato cap for a whole scene is 256 MiB, so the pilot corner takes under 2 percent
of it. Measured VRAM with an adapter and driver line needs the reference GPU and is not claimed.

## Bytes x2

`node content/maps/urban-war-pilot/cook-corner.mjs` run twice leaves `corner-cook.json`
at SHA-256 `018c4cf77164c5125c6edde555b0a1bc9bcac6e212ebf8b7354660bdd62c5ffa` both times.
`node --test tools/pilot-scout.test.mjs` checks every number in this page against the
pilot files and the CT9 textures.

## Not here

The Rust cook of the corner meshes through CT1 and CT2, the AS1 bundle and its load are CT56.
