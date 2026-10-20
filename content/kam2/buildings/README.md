# ART3 - the building kinds as rendered atlases

Receipt: `KAM2 BUILDINGS OK kinds=5 cells=10 bytes=319951 nativescale=50.6`

Five kinds, each assembled from the CC0 Kenney Fantasy Town Kit 2.0 and rendered once at the
style-lock angle. Footprints are the sim's own, from `HouseKind::footprint` in
`crates/kam2-sim/src/economy.rs`, not a blanket 2x2.

| id | footprint | cell | atlas | bytes | sha256 |
| --- | --- | --- | --- | --- | --- |
| `house.warehouse` | 3x3 | 320 | 322x644 | 100,043 | `cdf94a00...e3d9` |
| `house.farm` | 2x2 | 224 | 226x452 | 53,449 | `82b842fb...a207` |
| `house.mill` | 2x2 | 288 | 290x580 | 67,995 | `1b041d0b...4d9b` |
| `house.bakery` | 2x2 | 224 | 226x452 | 51,609 | `6e44de23...a06c` |
| `house.woodcutter` | 2x2 | 224 | 226x452 | 46,855 | `fc0bdf47...f346` |

Full hashes are in each `source.json`. Every atlas is under the 400 KB cap.

## What each kind is

- **warehouse** (3x3): two storeys, plaster base and timber upper, wide loading bay, 3x3 hip
  roof, loaded cart, lantern and banner.
- **farm** (2x2): low 2x1 barn with a gable roof on the north half, dirt plot with three hedge
  crop rows and a fence and gate on the south half.
- **mill** (2x2): two-storey tower, hip cap, the kit's four-sail windmill on a short axle.
- **bakery** (2x2): shopfront with a wide doorway and shutters, hip roof, tall chimney, front
  bench and banner.
- **woodcutter** (2x2): open-front shed on four posts, hip roof, log stack and chopping block.

## Style lock numbers

- Orthographic, pitch `atan(0.5)` = 26.5651 deg, one yaw 135 deg = 45 deg off the S axis
  (camera at the south-east corner, south and east faces visible). Buildings do not rotate.
- Transparent film, baked contact shadow (35% `#1B1F24`, 2 px along the sun), Cycles CPU,
  48 samples, fixed seed.
- One row per state: row 0 `finished`, row 1 `half_built`, one facing per row. Atlas gutter
  1 px, alpha test 0.5.
- Pivot is the footprint centre on the ground plane. Every state of a kind lands on the same
  pivot pixel, so the rows stack without a jump.

| kind | cell | span (world) | ground row | pivot px | px per kit cell |
| --- | --- | --- | --- | --- | --- |
| warehouse | 320 | 6.3241 | 240 | 160,240 | 50.60 |
| farm | 224 | 4.4269 | 168 | 112,168 | 50.60 |
| mill | 288 | 5.6917 | 216 | 144,216 | 50.60 |
| bakery | 224 | 4.4269 | 168 | 112,168 | 50.60 |
| woodcutter | 224 | 4.4269 | 168 | 112,168 | 50.60 |

Scale is native: 50.6 px per kit cell, the same number `content/kam2/house/source.json`
records as native. Factor 1.00, no shrink. A 2x2 kind sits in 224 px and the 3x3 warehouse in
320 px because the footprint diamond, the roof height and the contact shadow all have to fit;
128 px cannot hold a 2x2 at this scale, let alone a 3x3.

## Rebuild

The kit is not committed. Download the CC0 zip (3,854,691 bytes, sha256
`1a7530c0...67ef`, no account), unzip it, then:

```powershell
$kit = "<unzip>\Models\GLB format"
$out = "$env:TEMP\kam2\buildings"
$cfg = @{
  warehouse  = @{ cell = 320; span = 6.3241; ground = 240; px = 1.5; py = 1.5; fp = "3x3" }
  farm       = @{ cell = 224; span = 4.4269; ground = 168; px = 1.0; py = 1.0; fp = "2x2" }
  mill       = @{ cell = 288; span = 5.6917; ground = 216; px = 1.0; py = 1.0; fp = "2x2" }
  bakery     = @{ cell = 224; span = 4.4269; ground = 168; px = 1.0; py = 1.0; fp = "2x2" }
  woodcutter = @{ cell = 224; span = 4.4269; ground = 168; px = 1.0; py = 1.0; fp = "2x2" }
}
foreach ($kind in $cfg.Keys) {
  $c = $cfg[$kind]
  foreach ($state in @("finished", "half_built")) {
    blender --background --factory-startup --python tools\kam2\build_buildings.py -- `
      --kit $kit --out $out --kind $kind --state $state
    blender --background --factory-startup --python tools\kam2\render_frames.py -- `
      --scene "$out\$kind\$state.glb" --out "$out\$kind\$state" --yaw 135 --size $c.cell `
      --span $c.span --ground-px $c.ground --pivot-x $c.px --pivot-y $c.py --samples 48
  }
  New-Item -ItemType Directory -Force "$out\pack\$kind" | Out-Null
  Copy-Item "$out\$kind\finished\frame_00.png" "$out\pack\$kind\state_00.png" -Force
  Copy-Item "$out\$kind\half_built\frame_00.png" "$out\pack\$kind\state_01.png" -Force
  Copy-Item "$out\$kind\finished\frames.json" "$out\pack\$kind\frames.json" -Force
  node tools\kam2\pack_atlas.mjs --in "$out\pack\$kind" --out "content\kam2\buildings\$kind" `
    --id "house.$kind" --kind building --facings 1 `
    --anim finished:1:8:true --anim half_built:1:8:true --footprint $c.fp
}
```

`build_buildings.py --list` prints the kind table. `--kind` and `--state` build one model.
`node tools\kam2\buildings_receipt.mjs` re-checks every atlas hash, footprint, pivot and the
400 KB cap, then prints the receipt line above.

## Rebuild proof

Two complete rebuilds (10 `.glb`, 10 renders, 5 atlases) wrote the same bytes:

| atlas | sha256, both runs | bytes |
| --- | --- | --- |
| `house.warehouse.atlas.png` | `cdf94a006615e0fd119dd4104d90c5d27877349f66e37dd03376cae65c05e3d9` | 100,043 |
| `house.farm.atlas.png` | `82b842fbc5b87fbe716d30015ffd86d8f91fdfaf58a7430e8f976131b560a207` | 53,449 |
| `house.mill.atlas.png` | `1b041d0befee16ff8a1874755e3b267e2abbc1c0fc5eceae06c6abe58cef4d9b` | 67,995 |
| `house.bakery.atlas.png` | `6e44de23bce8b29f919f7f33973f44a5de8a7698a644af6f551f1a2fee2aa06c` | 51,609 |
| `house.woodcutter.atlas.png` | `fc0bdf47fc44d89f81386f0285ed4978231ccd751093ef3b80f6b5ebe618f346` | 46,855 |

The renderer is fixed-seed Cycles; the packer decodes pixels and writes a minimal PNG, so
Blender's per-run metadata never reaches the atlas.

## Open items

- The cottage is drawn at 0.79 of native (128 px cell, span 3.2). These five are at 1.00, so
  the cottage will read about 21% smaller in the same scene. Either re-render the cottage at
  224/4.4269 or carry a per-asset ground-scale field in the manifest; one decision, then all
  six stack.
- The kit has no crates, bales, field or crop pieces. The warehouse cart and the farm hedge
  rows are stand-ins named in `source.json`; a better CC0 prop can replace them without a
  footprint change.
- No damaged row. `House` in the sim has no hit points, so damage would be a new game state
  and a separate assembly per kind, not a tint.
- No team-colour mask, no cameo. Both manifest fields stay `null` until those passes land.
- Kit palette is pastel; only the roofs (`#A33B2E`) and the farm plot (`#8A6A43`) use lock
  colours. A look-lock pass over wood, plaster and stone is one pass over all kinds.
- The mill sails are static. The kit piece is one mesh, so a turning sail needs either a
  separate blade mesh or a shader spin; idle animation is a later row.
