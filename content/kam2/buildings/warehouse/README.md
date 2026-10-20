# warehouse - 3x3 at native scale

Receipt: `KAM2 BUILDINGS OK kinds=5 cells=10 bytes=319951 nativescale=50.6`

One rendered building kind. Model assembled from the CC0 Kenney Fantasy Town Kit 2.0 by
`tools/kam2/build_buildings.py`; the kit itself is never committed.

| file | what it is |
| --- | --- |
| `house.warehouse.atlas.png` | 322x644, 2 x 320 px cells, 1 px gutter, 100,043 bytes |
| `manifest.json` | id, kind, footprint 3x3, pivot, cell, rows `finished` and `half_built` |
| `source.json` | kit URL and sha256, assembly table, render settings, atlas hash, built .glb hashes |
| `README.md` | this receipt |

## Model

Two storeys on the sim's 3x3 footprint. The lower floor is ten plaster wall panels,
one small window and a wide open loading bay on the south face; the upper floor is twelve
timber panels, six of them small windows. Four corner posts run both floors. The roof is the
kit's 3x3 hip: four corner quarters, four edge slopes and one flat cap. A loaded cart, a
lantern and a banner at the door give the goods read.

Half-built state: Nine floor planks, the south and west ground-floor walls, four corner posts and four
scaffold panels on the open east and north sides. No upper floor and no roof.

## Numbers

- Footprint 3x3 tiles, cell 320x320, atlas 322x644.
- Ortho span 6.3241 world units, 50.60 px per kit cell,
  factor 1.00 against the 50.6 px native scale.
- Ground row 240, pivot `160,240`, the footprint centre on the ground plane.
  The half-built row uses the same pivot, so the two rows stack.
- Alpha box: finished [47,81]-[272,290], half-built [47,144]-[271,290].
- Yaw 135 deg (45 deg off S), pitch 26.5651 deg, transparent film, baked contact shadow,
  Cycles CPU 48 samples, fixed seed.
- Atlas sha256 `cdf94a006615e0fd119dd4104d90c5d27877349f66e37dd03376cae65c05e3d9`, 100,043 bytes (cap 400 KB).

## Rebuild

From the repo root, with the kit unzipped:

```powershell
blender --background --factory-startup --python tools\kam2\build_buildings.py -- --kit "<pack>\Models\GLB format" --out $env:TEMP\kam2 --kind warehouse --state finished
blender --background --factory-startup --python tools\kam2\render_frames.py -- --scene "$env:TEMP\kam2\warehouse\finished.glb" --out "$env:TEMP\kam2\warehouse\finished" --yaw 135 --size 320 --span 6.3241 --ground-px 240 --pivot-x 1.5 --pivot-y 1.5 --samples 48
```

Repeat both lines with `half_built` for row 1, copy `frame_00.png` of each state to
`state_00.png` and `state_01.png`, then pack with the command in `source.json`. The full
five-kind loop and the byte-identical rebuild proof are in [`../README.md`](../README.md).

## Open items

The kit has no crate or bale, so `cart-high` is the goods stand-in. The 3x3 hip has
a flat cap because the kit has no 3-wide gable span; that is the kit's intended 3x3 roof.

One shared item: every kind in this set and the cottage draw at the same 50.6 px per tile
(ART3b), and `tools/kam2/buildings_receipt.mjs` re-checks that number per kind.
