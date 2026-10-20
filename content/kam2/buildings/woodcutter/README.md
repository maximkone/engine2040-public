# woodcutter - 2x2 at native scale

Receipt: `KAM2 BUILDINGS OK kinds=5 cells=10 bytes=319951 nativescale=50.6`

One rendered building kind. Model assembled from the CC0 Kenney Fantasy Town Kit 2.0 by
`tools/kam2/build_buildings.py`; the kit itself is never committed.

| file | what it is |
| --- | --- |
| `house.woodcutter.atlas.png` | 226x452, 2 x 224 px cells, 1 px gutter, 46,855 bytes |
| `manifest.json` | id, kind, footprint 2x2, pivot, cell, rows `finished` and `half_built` |
| `source.json` | kit URL and sha256, assembly table, render settings, atlas hash, built .glb hashes |
| `README.md` | this receipt |

## Model

An open-front shed: four floor planks, a timber back wall and two rear side panels on
four corner posts, under a 2x2 hip roof. The front is open so the log stack reads: three cut
posts stacked along X, two loose logs on the ground and a chopping block, all kit timber.

Half-built state: Floor planks, the back wall, four posts, two scaffold panels on the open front and two
logs already on site.

## Numbers

- Footprint 2x2 tiles, cell 224x224, atlas 226x452.
- Ortho span 4.4269 world units, 50.60 px per kit cell,
  factor 1.00 against the 50.6 px native scale.
- Ground row 168, pivot `112,168`, the footprint centre on the ground plane.
  The half-built row uses the same pivot, so the two rows stack.
- Alpha box: finished [35,87]-[188,202], half-built [35,89]-[188,202].
- Yaw 135 deg (45 deg off S), pitch 26.5651 deg, transparent film, baked contact shadow,
  Cycles CPU 48 samples, fixed seed.
- Atlas sha256 `fc0bdf47fc44d89f81386f0285ed4978231ccd751093ef3b80f6b5ebe618f346`, 46,855 bytes (cap 400 KB).

## Rebuild

From the repo root, with the kit unzipped:

```powershell
blender --background --factory-startup --python tools\kam2\build_buildings.py -- --kit "<pack>\Models\GLB format" --out $env:TEMP\kam2 --kind woodcutter --state finished
blender --background --factory-startup --python tools\kam2\render_frames.py -- --scene "$env:TEMP\kam2\woodcutter\finished.glb" --out "$env:TEMP\kam2\woodcutter\finished" --yaw 135 --size 224 --span 4.4269 --ground-px 168 --pivot-x 1.0 --pivot-y 1.0 --samples 48
```

Repeat both lines with `half_built` for row 1, copy `frame_00.png` of each state to
`state_00.png` and `state_01.png`, then pack with the command in `source.json`. The full
five-kind loop and the byte-identical rebuild proof are in [`../README.md`](../README.md).

## Open items

The logs are scaled `pillar-wood` pieces; the kit has no log, axe or saw. The open
front needs the roof shadow to stay light enough at 100%; check it in the renderer's first
screenshot.

One shared item: every kind in this set and the cottage draw at the same 50.6 px per tile
(ART3b), and `tools/kam2/buildings_receipt.mjs` re-checks that number per kind.
