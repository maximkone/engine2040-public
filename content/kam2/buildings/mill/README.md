# mill - 2x2 at native scale

Receipt: `KAM2 BUILDINGS OK kinds=5 cells=10 bytes=319951 nativescale=50.6`

One rendered building kind. Model assembled from the CC0 Kenney Fantasy Town Kit 2.0 by
`tools/kam2/build_buildings.py`; the kit itself is never committed.

| file | what it is |
| --- | --- |
| `house.mill.atlas.png` | 290x580, 2 x 288 px cells, 1 px gutter, 67,995 bytes |
| `manifest.json` | id, kind, footprint 2x2, pivot, cell, rows `finished` and `half_built` |
| `source.json` | kit URL and sha256, assembly table, render settings, atlas hash, built .glb hashes |
| `README.md` | this receipt |

## Model

A two-storey 2x2 tower: plaster lower floor with a door and two windows, timber upper
floor with four windows, two rows of corner posts and a 2x2 hip cap. The kit's `windmill`
piece is only the four sails and hub, so it is scaled to 0.72 (2.24 units across) and mounted
on a short timber axle on the south face, where the camera sees it as an X.

Half-built state: Four floor planks, the south and west lower walls, four corner posts and three scaffold
panels; no upper floor, no cap and no sails.

## Numbers

- Footprint 2x2 tiles, cell 288x288, atlas 290x580.
- Ortho span 5.6917 world units, 50.60 px per kit cell,
  factor 1.00 against the 50.6 px native scale.
- Ground row 216, pivot `144,216`, the footprint centre on the ground plane.
  The half-built row uses the same pivot, so the two rows stack.
- Alpha box: finished [52,64]-[223,250], half-built [67,137]-[220,250].
- Yaw 135 deg (45 deg off S), pitch 26.5651 deg, transparent film, baked contact shadow,
  Cycles CPU 48 samples, fixed seed.
- Atlas sha256 `1b041d0befee16ff8a1874755e3b267e2abbc1c0fc5eceae06c6abe58cef4d9b`, 67,995 bytes (cap 400 KB).

## Rebuild

From the repo root, with the kit unzipped:

```powershell
blender --background --factory-startup --python tools\kam2\build_buildings.py -- --kit "<pack>\Models\GLB format" --out $env:TEMP\kam2 --kind mill --state finished
blender --background --factory-startup --python tools\kam2\render_frames.py -- --scene "$env:TEMP\kam2\mill\finished.glb" --out "$env:TEMP\kam2\mill\finished" --yaw 135 --size 288 --span 5.6917 --ground-px 216 --pivot-x 1.0 --pivot-y 1.0 --samples 48
```

Repeat both lines with `half_built` for row 1, copy `frame_00.png` of each state to
`state_00.png` and `state_01.png`, then pack with the command in `source.json`. The full
five-kind loop and the byte-identical rebuild proof are in [`../README.md`](../README.md).

## Open items

The sails are a static kit piece, not an animation. The tower body is assembled from
wall panels because the kit ships no mill tower. A later pass can split the sails into a
separate layer and spin them at the lock's 8 fps idle rate.

One shared item: the cottage is still drawn at 0.79 of native, so this 2x2 kind is
about 27% larger in the same scene until the cottage is re-rendered or a ground-scale field
lands in the manifest.
