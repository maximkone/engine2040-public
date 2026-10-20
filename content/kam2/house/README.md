# ART2 + ART3b - the cottage: native scale, three states, one cameo

Receipts (from `node tools/kam2/buildings_receipt.mjs`):

```
KAM2 HOUSE OK cells=3 atlas=226x678 bytes=75907 cameo=6400
KAM2 SCALE OK kinds=6 scale=50.600..50.600 px-per-tile=50.6
KAM2 CAMEOS OK kinds=6 cells=6 bytes=39193
```

One Kenney Fantasy Town Kit cottage, assembled on a 2x2 footprint, rendered orthographically
at the style-lock angle and packed into one atlas with three states. ART3b moved it off the
first pass's 0.79 shrink: the cell grew 128 -> 224 px and the render now uses the same 50.6 px
per kit cell (50.6 px per tile) as `content/kam2/buildings/`.

- Model: Kenney Fantasy Town Kit 2.0, CC0 1.0, no account.
  Page `https://kenney.nl/assets/fantasy-town-kit`,
  zip `https://kenney.nl/media/pages/assets/fantasy-town-kit/efe948d309-1754222374/kenney_fantasy-town-kit_2.0.zip`
  (3,854,691 bytes, sha256 `1a7530c0...67ef`). The castle kit was checked first and holds no
  house pieces: walls, towers, gates and siege engines only.
- Assembled by `tools/kam2/build_house.py` from kit pieces (wall panels, hip roof quarters,
  chimney, corner posts, floor planks), one `.glb` per state, transforms baked into the mesh.
  The built `.glb` files are not committed: the script plus the hashes in `source.json`
  rebuild them.
- Rendered by `tools/kam2/render_frames.py`, Blender 5.2.0 LTS, Cycles CPU, 48 samples:
  224x224 cell, ortho pitch 26.5651 deg, one yaw 135 deg = 45 deg off the S axis (camera at
  the building's south-east corner, south and east faces visible), transparent film, baked
  contact shadow, span 4.4269 world units, ground row 168, world pivot `1,1`.
- Cameo: the same finished model, one render at the cameo angles of the lock (yaw 45 deg off
  S, pitch 35, ortho, 96x72, transparent), packed beside the atlas as a 96x72 cell.
- Roof pieces carry one flat `#A33B2E` material to hit the lock's roof red; every other kit
  material is as authored.
- Packed by `tools/kam2/pack_atlas.mjs`: 1 px gutter, one facing, three rows
  (0 finished, 1 half_built, 2 damaged), pivot `112,168` = footprint centre on the ground
  plane, footprint 2x2 tiles; `--cameo ... --cameo-cell 96x72` writes the cameo PNG and the
  manifest's `cameo` object.

| file | what it is |
| --- | --- |
| `house.cottage.atlas.png` | 226x678, 3 x 224 px cells, 1 px gutter, 75,907 bytes |
| `house.cottage.cameo.png` | 98x74, one 96x72 cell, 1 px gutter, 6,400 bytes |
| `manifest.json` | id, kind, tile, pivot, footprint, one facing, three state rows, `cameo` |
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
# 3. one render per state, native scale, yaw 135 = 45 deg off S
blender --background --factory-startup --python tools\kam2\render_frames.py -- --scene $env:TEMP\kam2\house\finished.glb --out $env:TEMP\kam2\house\finished --yaw 135 --size 224 --span 4.4269 --ground-px 168 --pivot-x 1 --pivot-y 1 --samples 48
blender --background --factory-startup --python tools\kam2\render_frames.py -- --scene $env:TEMP\kam2\house\half_built.glb --out $env:TEMP\kam2\house\half_built --yaw 135 --size 224 --span 4.4269 --ground-px 168 --pivot-x 1 --pivot-y 1 --samples 48
blender --background --factory-startup --python tools\kam2\render_frames.py -- --scene $env:TEMP\kam2\house\damaged.glb --out $env:TEMP\kam2\house\damaged --yaw 135 --size 224 --span 4.4269 --ground-px 168 --pivot-x 1 --pivot-y 1 --samples 48
# 4. the cameo: same finished model, pitch 35, 96x72, transparent
blender --background --factory-startup --python tools\kam2\render_frames.py -- --scene $env:TEMP\kam2\house\finished.glb --out $env:TEMP\kam2\house\cameo --yaw 135 --pitch 35 --size 96x72 --samples 48
# 5. one PNG per state row, in order, then pack atlas and cameo
New-Item -ItemType Directory -Force $env:TEMP\kam2\house\pack | Out-Null
Copy-Item $env:TEMP\kam2\house\finished\frame_00.png $env:TEMP\kam2\house\pack\state_00.png
Copy-Item $env:TEMP\kam2\house\half_built\frame_00.png $env:TEMP\kam2\house\pack\state_01.png
Copy-Item $env:TEMP\kam2\house\damaged\frame_00.png $env:TEMP\kam2\house\pack\state_02.png
Copy-Item $env:TEMP\kam2\house\finished\frames.json $env:TEMP\kam2\house\pack\frames.json
node tools\kam2\pack_atlas.mjs --in $env:TEMP\kam2\house\pack --out content\kam2\house --id house.cottage --kind building --facings 1 --anim finished:1:8:true --anim half_built:1:8:true --anim damaged:1:8:true --footprint 2x2 --pivot 112,168 --cameo $env:TEMP\kam2\house\cameo\frame_00.png --cameo-cell 96x72
```

`node tools\kam2\buildings_receipt.mjs` re-checks the atlas hash, the cameo hash, the cell and
pivot, the footprint, the cameo angles and the 400 KB cap, then prints the receipts above.

## Numbers

- Cell 224x224, atlas 226x678 (3 rows of 224 + 1 px gutter), 75,907 bytes.
- Scale 50.6 px per kit cell = 50.6 px per tile, read back as cell px / world span
  (224 / 4.4269). That is the same number as the five kinds in `content/kam2/buildings/`
  and the same number `source.json` records as native, so the cottage no longer reads 27%
  too large. Footprint 2x2 tiles, pivot `112,168` = footprint centre on the ground plane.
- Alpha extent: columns 36..187 for all three states, rows 81..201 (half-built starts at
  row 90), so each cell holds a 152 x 121 px alpha box with the baked shadow reaching row 201.
- Cameo 98x74 (96x72 + 1 px gutter), 6,400 bytes, yaw 45 deg off S, pitch 35, transparent
  corners, alpha box columns 19..78 / rows 19..69. Manifest: `cameo` = `{id:
  "house.cottage.cameo", atlas: "house.cottage.cameo.png", cell: [96,72], size: [98,74],
  padding: 1, pivot: [48,52]}`. The sidebar draws it at 64x48.
- Atlas + cameo 82,307 bytes, under the 400 KB cap; no atlas side over the 2048 px lock.

## Rebuild proof

Two full rebuilds (three `.glb`, three renders, one cameo, one pack each) wrote the same bytes:

| file | sha256, both runs | bytes |
| --- | --- | --- |
| `house.cottage.atlas.png` | `b072c60322d579055f1fbe6425ddf86ecae04e2a638f191f1f25ff2667bd8f51` | 75,907 |
| `house.cottage.cameo.png` | `d172283a3bd066334a914cc2616c63002ad21e098a8f376c2d33f44354cce3f4` | 6,400 |

The renderer is fixed-seed Cycles; the packer decodes pixels and writes a minimal PNG, so
Blender's per-run metadata never reaches the atlas or the cameo.

## Open items

- Walls, wood and stone keep the kit's pastel palette; only the roof was moved to the lock
  red. Matching the plaster `#D8CDB4`, wood `#8B5A2B` and stone `#8A8172` ramps is a look-lock
  pass over all buildings, not one asset.
- Three build states live as rows of one id; the lock names stages as separate ids
  (`house.farm.stage2`). Decide which shape the runtime wants before art volume.
- No team-colour mask; `mask_atlas` stays null.
- The baked contact shadow is a soft fringe under a footprint-filling building. A
  building-specific ellipse (bigger offset) is a renderer question, not an asset question.
- The cottage atlas cell changed from 128 to 224 px in ART3b. The file paths did not change
  (`house/house.cottage.atlas.png`, `house/manifest.json`); the manifest's `tile_w`, `tile_h`,
  `pivot` and `atlas_size` did, and the tests in `crates/kam2-game/src/house.rs` and
  `crates/kam2-web/src/scene_flat.rs` follow the manifest.
