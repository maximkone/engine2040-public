# bakery - 2x2 at native scale

Receipt: `KAM2 BUILDINGS OK kinds=5 cells=10 bytes=319951 nativescale=50.6`

One rendered building kind. Model assembled from the CC0 Kenney Fantasy Town Kit 2.0 by
`tools/kam2/build_buildings.py`; the kit itself is never committed.

| file | what it is |
| --- | --- |
| `house.bakery.atlas.png` | 226x452, 2 x 224 px cells, 1 px gutter, 51,609 bytes |
| `manifest.json` | id, kind, footprint 2x2, pivot, cell, rows `finished` and `half_built` |
| `source.json` | kit URL and sha256, assembly table, render settings, atlas hash, built .glb hashes |
| `README.md` | this receipt |

## Model

A small shop: timber walls, a wide doorway and a small window on the south face,
shuttered windows on the east and west, four corner posts and a 2x2 hip roof. The chimney is
two kit pieces, base through the roof and cap above the apex. A bench at the front and a
banner by the door carry the shop read.

Half-built state: Four floor planks, the west wall at full height, the south face at half height, four
corner posts, three scaffold panels and the chimney base standing on site.

## Numbers

- Footprint 2x2 tiles, cell 224x224, atlas 226x452.
- Ortho span 4.4269 world units, 50.60 px per kit cell,
  factor 1.00 against the 50.6 px native scale.
- Ground row 168, pivot `112,168`, the footprint centre on the ground plane.
  The half-built row uses the same pivot, so the two rows stack.
- Alpha box: finished [35,70]-[188,203], half-built [35,89]-[188,202].
- Yaw 135 deg (45 deg off S), pitch 26.5651 deg, transparent film, baked contact shadow,
  Cycles CPU 48 samples, fixed seed.
- Atlas sha256 `6e44de23bce8b29f919f7f33973f44a5de8a7698a644af6f551f1a2fee2aa06c`, 51,609 bytes (cap 400 KB).

## Rebuild

From the repo root, with the kit unzipped:

```powershell
blender --background --factory-startup --python tools\kam2\build_buildings.py -- --kit "<pack>\Models\GLB format" --out $env:TEMP\kam2 --kind bakery --state finished
blender --background --factory-startup --python tools\kam2\render_frames.py -- --scene "$env:TEMP\kam2\bakery\finished.glb" --out "$env:TEMP\kam2\bakery\finished" --yaw 135 --size 224 --span 4.4269 --ground-px 168 --pivot-x 1.0 --pivot-y 1.0 --samples 48
```

Repeat both lines with `half_built` for row 1, copy `frame_00.png` of each state to
`state_00.png` and `state_01.png`, then pack with the command in `source.json`. The full
five-kind loop and the byte-identical rebuild proof are in [`../README.md`](../README.md).

## Open items

The kit has no oven, bread or sack piece, so the chimney and shopfront carry the kind.
A bakery-specific prop is a later row.

One shared item: the cottage is still drawn at 0.79 of native, so this 2x2 kind is
about 27% larger in the same scene until the cottage is re-rendered or a ground-scale field
lands in the manifest.
