# ART2 - first rendered building: house.cottage

Receipt: `KAM2 HOUSE OK cells=3 atlas=130x390 bytes=53718`

One Kenney Fantasy Town Kit cottage, assembled on a 2x2 footprint, rendered orthographically
at the style-lock angle and packed into one atlas with three states.

- Model: Kenney Fantasy Town Kit 2.0, CC0 1.0, no account.
  Page `https://kenney.nl/assets/fantasy-town-kit`,
  zip `https://kenney.nl/media/pages/assets/fantasy-town-kit/efe948d309-1754222374/kenney_fantasy-town-kit_2.0.zip`
  (3,854,691 bytes, sha256 `1a7530c0...67ef`). The castle kit was checked first and holds no
  house pieces: walls, towers, gates and siege engines only.
- Assembled by `tools/kam2/build_house.py` from kit pieces (wall panels, hip roof quarters,
  chimney, corner posts, floor planks), one `.glb` per state, transforms baked into the mesh.
  The built `.glb` files are not committed: the script plus the hashes in `source.json` rebuild them.
- Rendered by `tools/kam2/render_frames.py`, Blender 5.2.0 LTS, Cycles CPU, 48 samples:
  128x128 cell, ortho pitch 26.5651 deg, one yaw 45 deg off the S axis (ring frame 01, camera at
  the building's south-east corner, south and east faces visible), transparent film, baked
  contact shadow, span 3.2 world units, ground row 95.
- Roof pieces carry one flat `#A33B2E` material to hit the lock's roof red; every other kit
  material is as authored.
- Packed by `tools/kam2/pack_atlas.mjs`: 1 px gutter, one facing, three rows
  (0 finished, 1 half_built, 2 damaged), pivot `64,95` = footprint centre on the ground plane,
  footprint 2x2 tiles.

| file | what it is |
| --- | --- |
| `house.cottage.atlas.png` | 130x390, 3 x 128 px cells, 1 px gutter, 53,718 bytes |
| `manifest.json` | id, kind, tile, pivot, footprint, one facing, three state rows |
| `source.json` | model, URL, licence, assembly table, render and pack settings, pixel scale |
| `README.md` | this receipt |

How to rebuild, from the repo root:

```powershell
# 1. download and unzip the kit (CC0, no account)
Invoke-WebRequest -Uri "https://kenney.nl/media/pages/assets/fantasy-town-kit/efe948d309-1754222374/kenney_fantasy-town-kit_2.0.zip" -OutFile kenney_fantasy-town-kit_20.zip
Expand-Archive kenney_fantasy-town-kit_20.zip -DestinationPath fantasy-town-kit
# 2. one .glb per state
blender --background --factory-startup --python tools\kam2\build_house.py -- --kit "fantasy-town-kit\Models\GLB format" --out $env:TEMP\kam2\house --state finished
blender --background --factory-startup --python tools\kam2\build_house.py -- --kit "fantasy-town-kit\Models\GLB format" --out $env:TEMP\kam2\house --state half_built
blender --background --factory-startup --python tools\kam2\build_house.py -- --kit "fantasy-town-kit\Models\GLB format" --out $env:TEMP\kam2\house --state damaged
# 3. eight-yaw ring per state; keep frame 01, the 45 deg view
blender --background --factory-startup --python tools\kam2\render_frames.py -- --scene $env:TEMP\kam2\house\finished.glb --out $env:TEMP\kam2\house\finished --frames 8 --size 128 --span 3.2 --ground-px 95 --samples 48
blender --background --factory-startup --python tools\kam2\render_frames.py -- --scene $env:TEMP\kam2\house\half_built.glb --out $env:TEMP\kam2\house\half_built --frames 8 --size 128 --span 3.2 --ground-px 95 --samples 48
blender --background --factory-startup --python tools\kam2\render_frames.py -- --scene $env:TEMP\kam2\house\damaged.glb --out $env:TEMP\kam2\house\damaged --frames 8 --size 128 --span 3.2 --ground-px 95 --samples 48
# 4. one PNG per state row, in order
New-Item -ItemType Directory -Force $env:TEMP\kam2\house\pack | Out-Null
Copy-Item $env:TEMP\kam2\house\finished\frame_01.png $env:TEMP\kam2\house\pack\state_00.png
Copy-Item $env:TEMP\kam2\house\half_built\frame_01.png $env:TEMP\kam2\house\pack\state_01.png
Copy-Item $env:TEMP\kam2\house\damaged\frame_01.png $env:TEMP\kam2\house\pack\state_02.png
# 5. pack
node tools\kam2\pack_atlas.mjs --in $env:TEMP\kam2\house\pack --out content\kam2\house --id house.cottage --kind building --facings 1 --anim finished:1:8:true --anim half_built:1:8:true --anim damaged:1:8:true --footprint 2x2 --pivot 64,95
```

## Numbers

- Cell 128x128. Footprint 2x2 tiles. Pivot `64,95`, the footprint centre on the ground
  plane; the front footprint edge lands on row 120.
- Scale 40 px per kit cell = 0.79 of the lock's native 64x32 px tile. A 2x2 footprint diamond
  is 128 px wide at native scale and the kit's eaves add 0.14 unit per side (136 px), which no
  128 px cell holds: the factor is 0.79 for all three states so they stack.
- Alpha extent: columns 3..124 for all three states, rows 25..122 (half-built starts at row 32),
  so each cell holds a 122 x 98 px alpha box with the baked shadow reaching row 122.
- Atlas 130 x 390, 53,718 bytes (cap 400 KB, lock cap 2048 px per side).

## Open items

- Walls, wood and stone keep the kit's pastel palette; only the roof was moved to the lock
  red. Matching the plaster `#D8CDB4`, wood `#8B5A2B` and stone `#8A8172` ramps is a look-lock
  pass over all future buildings, not one asset.
- A 2x2 building at native tile scale needs a 192 px cell (or a manifest ground-scale field).
  At 128 px everything is drawn at 0.79.
- Three build states live as rows of one id; the lock names stages as separate ids
  (`house.farm.stage2`). Decide which shape the runtime wants before art volume.
- No team-colour mask and no cameo pass yet: both manifest fields are null.
- The baked contact shadow is a soft fringe under a footprint-filling building. A
  building-specific ellipse (bigger offset) is a renderer question, not an asset question.
