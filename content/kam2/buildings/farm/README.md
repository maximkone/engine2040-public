# farm - 2x2 at native scale

Receipt: `KAM2 BUILDINGS OK kinds=5 cells=10 bytes=319951 nativescale=50.6`

One rendered building kind. Model assembled from the CC0 Kenney Fantasy Town Kit 2.0 by
`tools/kam2/build_buildings.py`; the kit itself is never committed.

| file | what it is |
| --- | --- |
| `house.farm.atlas.png` | 226x452, 2 x 224 px cells, 1 px gutter, 53,449 bytes |
| `manifest.json` | id, kind, footprint 2x2, pivot, cell, rows `finished` and `half_built` |
| `source.json` | kit URL and sha256, assembly table, render settings, atlas hash, built .glb hashes |
| `README.md` | this receipt |

## Model

The north half is a low 2x1 barn: six timber wall panels, a door facing the field and a
2-cell gable roof with the ridge along X. The south half is the field: two floor planks under
one flat dirt material, three crop rows of two low hedge pieces each, and a fence with a gate
on the other three sides.

Half-built state: The field fence and gate are up, the barn has its north wall and one side wall at full
height, the south face is half-height and the east side is open with one scaffold panel.

## Numbers

- Footprint 2x2 tiles, cell 224x224, atlas 226x452.
- Ortho span 4.4269 world units, 50.60 px per kit cell,
  factor 1.00 against the 50.6 px native scale.
- Ground row 168, pivot `112,168`, the footprint centre on the ground plane.
  The half-built row uses the same pivot, so the two rows stack.
- Alpha box: finished [30,47]-[188,201], half-built [39,89]-[188,200].
- Yaw 135 deg (45 deg off S), pitch 26.5651 deg, transparent film, baked contact shadow,
  Cycles CPU 48 samples, fixed seed.
- Atlas sha256 `82b842fbc5b87fbe716d30015ffd86d8f91fdfaf58a7430e8f976131b560a207`, 53,449 bytes (cap 400 KB).

## Rebuild

From the repo root, with the kit unzipped:

```powershell
blender --background --factory-startup --python tools\kam2\build_buildings.py -- --kit "<pack>\Models\GLB format" --out $env:TEMP\kam2 --kind farm --state finished
blender --background --factory-startup --python tools\kam2\render_frames.py -- --scene "$env:TEMP\kam2\farm\finished.glb" --out "$env:TEMP\kam2\farm\finished" --yaw 135 --size 224 --span 4.4269 --ground-px 168 --pivot-x 1.0 --pivot-y 1.0 --samples 48
```

Repeat both lines with `half_built` for row 1, copy `frame_00.png` of each state to
`state_00.png` and `state_01.png`, then pack with the command in `source.json`. The full
five-kind loop and the byte-identical rebuild proof are in [`../README.md`](../README.md).

## Open items

The kit has no field, crop, grain or bale piece. The hedge rows are the closest green
row the kit ships, and the dirt plot is a recoloured plank floor. Replace both when a better
CC0 farm prop is found; the footprint and pivot do not change.

One shared item: the cottage is still drawn at 0.79 of native, so this 2x2 kind is
about 27% larger in the same scene until the cottage is re-rendered or a ground-scale field
lands in the manifest.
