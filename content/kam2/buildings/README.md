# ART3 - the building kinds: one native scale, six kinds, six cameos

Receipts (from `node tools/kam2/buildings_receipt.mjs`):

```
KAM2 BUILDINGS OK kinds=5 cells=10 bytes=319951 nativescale=50.6
KAM2 HOUSE OK cells=3 atlas=226x678 bytes=75907 cameo=6400
KAM2 SCALE OK kinds=6 scale=50.600..50.600 px-per-tile=50.6
KAM2 CAMEOS OK kinds=6 cells=6 bytes=39193
```

Six kinds, each assembled from the CC0 Kenney Fantasy Town Kit 2.0 and rendered once at the
style-lock angle. Five live here; the sixth is the cottage in `content/kam2/house/`, re-rendered
in ART3b at this set's scale. Footprints are the sim's own, from `HouseKind::footprint` in
`crates/kam2-sim/src/economy.rs`, not a blanket 2x2.

| id | footprint | cell | atlas | bytes | sha256 | cameo (98x74) | cameo bytes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `house.warehouse` | 3x3 | 320 | 322x644 | 100,043 | `cdf94a00...e3d9` | 7,535 | `d6802a24...90b4` |
| `house.farm` | 2x2 | 224 | 226x452 | 53,449 | `82b842fb...a207` | 6,740 | `73a491d6...5e46` |
| `house.mill` | 2x2 | 288 | 290x580 | 67,995 | `1b041d0b...4d9b` | 5,522 | `b457e89f...69d8` |
| `house.bakery` | 2x2 | 224 | 226x452 | 51,609 | `6e44de23...a06c` | 6,286 | `9e7d4d0d...f46b` |
| `house.woodcutter` | 2x2 | 224 | 226x452 | 46,855 | `fc0bdf47...f346` | 6,710 | `85bae586...b73` |
| `house.cottage` | 2x2 | 224 | 226x678 | 75,907 | `b072c603...8f51` | 6,400 | `d172283a...3f4` |

Full hashes are in each `source.json` and in the proof tables below. Every atlas plus its cameo
is under the 400 KB cap (largest: warehouse 100,043 + 7,535 = 107,578 bytes).

## What each kind is

- **warehouse** (3x3): two storeys, plaster base and timber upper, wide loading bay, 3x3 hip
  roof, loaded cart, lantern and banner.
- **farm** (2x2): low 2x1 barn with a gable roof on the north half, dirt plot with three hedge
  crop rows and a fence and gate on the south half.
- **mill** (2x2): two-storey tower, hip cap, the kit's four-sail windmill on a short axle.
- **bakery** (2x2): shopfront with a wide doorway and shutters, hip roof, tall chimney, front
  bench and banner.
- **woodcutter** (2x2): open-front shed on four posts, hip roof, log stack and chopping block.
- **cottage** (2x2): the first worked building, plaster and timber walls, door on the south
  face, pyramid hip roof and chimney. Only kind with three rows: half-built shows floor, two
  walls and scaffold; damaged shows the caved roof and fallen chimney.

## Style lock numbers

- Orthographic, pitch `atan(0.5)` = 26.5651 deg, one yaw 135 deg = 45 deg off the S axis
  (camera at the south-east corner, south and east faces visible). Buildings do not rotate.
- Transparent film, baked contact shadow (35% `#1B1F24`, 2 px along the sun), Cycles CPU,
  48 samples, fixed seed.
- One row per state: finished, then half_built (cottage adds damaged), one facing per row.
  Atlas gutter 1 px, alpha test 0.5.
- Pivot is the footprint centre on the ground plane. Every state of a kind lands on the same
  pivot pixel, so the rows stack without a jump.
- Cameo: same model, yaw 45 deg off the S axis, pitch 35, ortho, 96x72 cell, transparent.
  The sidebar uses it at 64x48.

| kind | cell | span (world) | ground row | pivot px | px per tile |
| --- | --- | --- | --- | --- | --- |
| warehouse | 320 | 6.3241 | 240 | 160,240 | 50.600 |
| farm | 224 | 4.4269 | 168 | 112,168 | 50.600 |
| mill | 288 | 5.6917 | 216 | 144,216 | 50.600 |
| bakery | 224 | 4.4269 | 168 | 112,168 | 50.600 |
| woodcutter | 224 | 4.4269 | 168 | 112,168 | 50.600 |
| cottage | 224 | 4.4269 | 168 | 112,168 | 50.600 |

Scale is one number for the whole set: 50.6 px per kit cell = 50.6 px per tile, `cell px /
world span` (e.g. 320 / 6.3241), read back per kind by the receipt. Factor 1.00, no shrink.
A 2x2 kind sits in 224 px, the mill's sail circle needs 288 and the 3x3 warehouse 320, because
the footprint diamond, the roof or sails and the contact shadow all have to fit. The cottage
was the last 0.79 outlier (128 px cell, span 3.2); ART3b gave it the same 224 px cell as the
other 2x2 kinds.

## Cameos

One cameo per kind, rendered from the finished model at pitch 35 and yaw 45 deg off S:

- source frame 96x72, packed beside the atlas as a 96x72 cell with the same 1 px gutter
  (98x74 PNG, `house.<kind>.cameo.png`), transparent background checked at the corners.
- the manifest's `cameo` is an object:
  `{id: "house.<kind>.cameo", atlas: "house.<kind>.cameo.png", cell: [96,72], size: [98,74],
  padding: 1, pivot: [x,y]}`. `pivot` is the render's ground anchor inside the cameo cell.
- atlas plus cameo bytes stay under the 400 KB cap per kind.
- 6 cells in total, 39,193 bytes of cameo PNGs.

## Rebuild

The kit is not committed. Download the CC0 zip (3,854,691 bytes, sha256
`1a7530c0...67ef`, no account), unzip it, then from the repo root:

```powershell
$kit = "<unzip>\Models\GLB format"
$out = "$env:TEMP\kam2\buildings"
$cfg = @(
  @{ kind = "warehouse";  cell = 320; span = 6.3241; ground = 240; px = 1.5; py = 1.5; fp = "3x3"; states = @("finished", "half_built") },
  @{ kind = "farm";       cell = 224; span = 4.4269; ground = 168; px = 1.0; py = 1.0; fp = "2x2"; states = @("finished", "half_built") },
  @{ kind = "mill";       cell = 288; span = 5.6917; ground = 216; px = 1.0; py = 1.0; fp = "2x2"; states = @("finished", "half_built") },
  @{ kind = "bakery";     cell = 224; span = 4.4269; ground = 168; px = 1.0; py = 1.0; fp = "2x2"; states = @("finished", "half_built") },
  @{ kind = "woodcutter"; cell = 224; span = 4.4269; ground = 168; px = 1.0; py = 1.0; fp = "2x2"; states = @("finished", "half_built") },
  @{ kind = "cottage";    cell = 224; span = 4.4269; ground = 168; px = 1.0; py = 1.0; fp = "2x2"; states = @("finished", "half_built", "damaged") }
)
foreach ($c in $cfg) {
  $glb = "$out\glb\$($c.kind)"
  $frames = "$out\frames\$($c.kind)"
  New-Item -ItemType Directory -Force $glb, "$out\pack\$($c.kind)" | Out-Null
  foreach ($state in $c.states) {
    if ($c.kind -eq "cottage") {
      blender --background --factory-startup --python tools\kam2\build_house.py -- `
        --kit $kit --out $glb --state $state
    } else {
      blender --background --factory-startup --python tools\kam2\build_buildings.py -- `
        --kit $kit --out "$out\glb" --kind $c.kind --state $state
    }
    blender --background --factory-startup --python tools\kam2\render_frames.py -- `
      --scene "$glb\$state.glb" --out "$frames\$state" --yaw 135 --size $c.cell `
      --span $c.span --ground-px $c.ground --pivot-x $c.px --pivot-y $c.py --samples 48
  }
  $i = 0
  foreach ($state in $c.states) {
    Copy-Item "$frames\$state\frame_00.png" ("$out\pack\$($c.kind)\state_{0:d2}.png" -f $i) -Force
    $i += 1
  }
  Copy-Item "$frames\finished\frames.json" "$out\pack\$($c.kind)\frames.json" -Force
  blender --background --factory-startup --python tools\kam2\render_frames.py -- `
    --scene "$glb\finished.glb" --out "$frames\cameo" --yaw 135 --pitch 35 --size 96x72 --samples 48
  $pivot = "$([int]($c.cell / 2)),$($c.ground)"
  $animArgs = foreach ($state in $c.states) { "--anim"; "${state}:1:8:true" }
  $dest = if ($c.kind -eq "cottage") { "content\kam2\house" } else { "content\kam2\buildings\$($c.kind)" }
  node tools\kam2\pack_atlas.mjs --in "$out\pack\$($c.kind)" --out $dest --id "house.$($c.kind)" `
    --kind building --facings 1 $animArgs --footprint $c.fp --pivot $pivot `
    --cameo "$frames\cameo\frame_00.png" --cameo-cell 96x72
}
```

`build_buildings.py --list` prints the kind table. `--kind` and `--state` build one model;
`build_house.py --state` builds one cottage. `render_frames.py --size 96x72` renders the
non-square cameo cell (one number keeps the old square cell).

`node tools\kam2\buildings_receipt.mjs` re-checks every atlas and cameo hash, footprint, pivot,
cell, scale, cameo angle and the 400 KB cap, then prints the receipts above. Exit 0 pass.

## Rebuild proof

Two complete rebuilds (13 `.glb`, 13 atlas renders, 6 cameo renders, 6 packs) wrote the same
bytes. Column A is the committed content, column B a second independent run:

| file | sha256, run A = run B | bytes |
| --- | --- | --- |
| `house.warehouse.atlas.png` | `cdf94a006615e0fd119dd4104d90c5d27877349f66e37dd03376cae65c05e3d9` | 100,043 |
| `house.farm.atlas.png` | `82b842fbc5b87fbe716d30015ffd86d8f91fdfaf58a7430e8f976131b560a207` | 53,449 |
| `house.mill.atlas.png` | `1b041d0befee16ff8a1874755e3b267e2abbc1c0fc5eceae06c6abe58cef4d9b` | 67,995 |
| `house.bakery.atlas.png` | `6e44de23bce8b29f919f7f33973f44a5de8a7698a644af6f551f1a2fee2aa06c` | 51,609 |
| `house.woodcutter.atlas.png` | `fc0bdf47fc44d89f81386f0285ed4978231ccd751093ef3b80f6b5ebe618f346` | 46,855 |
| `house.cottage.atlas.png` | `b072c60322d579055f1fbe6425ddf86ecae04e2a638f191f1f25ff2667bd8f51` | 75,907 |
| `house.warehouse.cameo.png` | `d6802a24dae05f04c4b3c0dff8073cdf4af46a9e4821f1b2b7688336cc4f90b4` | 7,535 |
| `house.farm.cameo.png` | `73a491d6a743086858ef63c98efba175ca424255f4c1e9ff42f7f9b8a4f59e46` | 6,740 |
| `house.mill.cameo.png` | `b457e89f004bc83edefc1e791ae227fa5cc8fde176f96d63e13b3db4dd9169d8` | 5,522 |
| `house.bakery.cameo.png` | `9e7d4d0df6969cf9570c6d187fb8abae130e32a6211a84272ed9a64ff422f46b` | 6,286 |
| `house.woodcutter.cameo.png` | `85bae5865ded6e0ec70fef07bab616e8ee3206a0245d72df93b71aef56d81b73` | 6,710 |
| `house.cottage.cameo.png` | `d172283a3bd066334a914cc2616c63002ad21e098a8f376c2d33f44354cce3f4` | 6,400 |

The renderer is fixed-seed Cycles; the packer decodes pixels and writes a minimal PNG, so
Blender's per-run metadata never reaches an atlas or a cameo. The same two numbers come out of
`sha256sum` on the committed files and on the second run's files.

## Open items

- The kit has no crates, bales, field or crop pieces. The warehouse cart and the farm hedge
  rows are stand-ins named in `source.json`; a better CC0 prop can replace them without a
  footprint change.
- No damaged row for the five kinds. `House` in the sim has no hit points, so damage would be
  a new game state and a separate assembly per kind, not a tint. The cottage keeps its damaged
  row from ART2; nothing selects it at runtime.
- No team-colour mask: `mask_atlas` stays null for every kind.
- Kit palette is pastel; only the roofs (`#A33B2E`) and the farm plot (`#8A6A43`) use lock
  colours. A look-lock pass over wood, plaster and stone is one pass over all kinds.
- The mill sails are static. The kit piece is one mesh, so a turning sail needs either a
  separate blade mesh or a shader spin; idle animation is a later row.
- The cottage cell changed from 128 to 224 px in ART3b, so the manifest's `tile_w`/`tile_h`,
  `pivot` and `atlas_size` changed with it. File paths did not change, and the parsing tests
  in `crates/kam2-game/src/house.rs` and `crates/kam2-web/src/scene_flat.rs` follow the
  manifest values.
