# KAM2 ART0 - free art factory

Turns a 3D model into isometric sprite frames in the RA2 look of
`docs/plan/kam2-style-lock.md`, then packs them into one atlas PNG plus `manifest.json`.
Two scripts, no add-ons, no downloads, no paid tools.

| file | what it does |
| --- | --- |
| `render_frames.py` | Blender 5.2 headless: orthographic ring, transparent film, `frame_NN.png` set + `frames.json`. |
| `pack_atlas.mjs` | Node 24, builtins only: PNG decode/encode through `node:zlib`, atlas PNG + `manifest.json`. |
| `build_house.py` | Blender 5.2 headless: assembles one 2x2 house from Kenney Fantasy Town Kit pieces, one `.glb` per state. |
| `build_buildings.py` | Blender 5.2 headless: assembles the five KAM2 house kinds on their real footprints (3x3 warehouse, 2x2 farm/mill/bakery/woodcutter), one `.glb` per state. |
| `buildings_receipt.mjs` | Node 24, builtins only: verifies the ART3 building atlases against their `source.json` and prints the receipt. |

Needs Blender 5.2 at `C:\Program Files\Blender Foundation\Blender 5.2\blender.exe` and Node 24.
Run everything from the repo root. The `--` after the script path separates our flags from Blender's.

## Self-test (no model needed)

```powershell
& "C:\Program Files\Blender Foundation\Blender 5.2\blender.exe" --background --factory-startup --python tools\kam2\render_frames.py -- --self-test --out C:\Users\me\AppData\Local\Temp\opencode\kam2-art\proof
node tools\kam2\pack_atlas.mjs --in C:\Users\me\AppData\Local\Temp\opencode\kam2-art\proof --out C:\Users\me\AppData\Local\Temp\opencode\kam2-art\proof\atlas --id unit.selftest --kind unit --anim idle:1:2:true
```

The first line builds a steel cube and a red cone in an empty scene and renders 8 facings.
It ends with:

```
KAM2 ART OK frames=8 out=C:\Users\me\AppData\Local\Temp\opencode\kam2-art\proof
```

The second line writes `unit.selftest.atlas.png` (8 columns x 1 row, 1 px gutter) and
`manifest.json`, and ends with:

```
KAM2 PACK OK frames=8 atlas=528x66 bytes=<n> manifest=<out>\manifest.json
```

## Render a model

```powershell
& "C:\Program Files\Blender Foundation\Blender 5.2\blender.exe" --background --factory-startup --python tools\kam2\render_frames.py -- --scene content\props\ai\ai_amphora.glb --out C:\Users\me\AppData\Local\Temp\opencode\kam2-art\amphora --frames 8 --size 64 --mask
```

`.blend`, `.glb` and `.gltf` load. A loaded scene's own cameras are always hidden; its lights
too unless `--keep-lights`. Output is `<out>\frame_NN.png`, `<out>\mask_NN.png` (with
`--mask`) and `<out>\frames.json` (frame names, size, pitch, pivot, bounds).

| flag | default | meaning |
| --- | --- | --- |
| `--scene <path>` | - | model to render (or use `--self-test`) |
| `--out <dir>` | `%TEMP%\kam2-art\<name>` | output folder |
| `--frames <n>` | 8 | frames around the ring, 45 deg per step |
| `--size <px\|WxH>` | 64 | cell in px: one number for a square cell, `96x72` for a cameo |
| `--pitch <deg>` | 26.565 (`atan 0.5`) | camera pitch |
| `--margin <f>` | 0.06 | framing margin as a fraction of the span |
| `--samples <n>` | 48 | Cycles samples, CPU |
| `--prefix <name>` | frame | frame file prefix |
| `--engine cycles\|eevee\|workbench` | cycles | render engine |
| `--mask` | off | extra pass: flat white on the model, alpha only |
| `--no-shadow` | off | skip the contact-shadow ellipse |
| `--keep-lights` | off | keep the loaded scene's lights |
| `--frame <n>` | scene frame | pose to render |
| `--action <name>` | - | play that clip (glTF action) instead of the imported pose |
| `--height-px <px>` | - | fit the upright body to `px` instead of filling the cell |
| `--span <units>` | - | fixed ortho span, to reuse one scale across runs |
| `--ground-px <px>` | centred | cell row where the ground pivot lands |
| `--floor-z <z>` | pose low point | ground plane of the model |
| `--yaw <deg>` | - | render one fixed yaw instead of the ring (sets frames 1) |
| `--pivot-x <x>` | bounds centre | world x of the ground pivot; the camera aims at it |
| `--pivot-y <y>` | bounds centre | world y of the ground pivot |

Frame `NN` is the compass heading `NN` of the style-lock order `S, SW, W, NW, N, NE, E, SE`.
The model stands still and the camera walks the ring, which is the same picture as a fixed
camera and a turning unit. Frame 0 faces the camera.

An animated glTF arrives with one action per clip and an NLA track per clip. `--action walk`
mutes every track, binds the clip's slot to each object it drives and jumps to `--frame`.
`--height-px` ignores the tilted bounding box (a stride would shrink the body), fits the floor
to crown height, and `--ground-px 56` puts the feet on row 56; `--span` from the first run
keeps every later pose at the same scale. Split an animation into one folder per frame and
pack the rows with `--anim`.

## Build a building

Buildings do not rotate: render the ring once and keep the 45 deg frame. `build_house.py`
assembles the worked example from the modular Kenney Fantasy Town Kit (CC0), which ships
1 unit wall cells, hip roof quarters, doors, posts and floors, not a house:

```powershell
blender --background --factory-startup --python tools\kam2\build_house.py -- --kit "<pack>\Models\GLB format" --out $env:TEMP\kam2\house --state finished
blender --background --factory-startup --python tools\kam2\render_frames.py -- --scene $env:TEMP\kam2\house\finished.glb --out $env:TEMP\kam2\house\finished --yaw 135 --size 224 --span 4.4269 --ground-px 168 --pivot-x 1 --pivot-y 1 --samples 48
blender --background --factory-startup --python tools\kam2\render_frames.py -- --scene $env:TEMP\kam2\house\finished.glb --out $env:TEMP\kam2\house\cameo --yaw 135 --pitch 35 --size 96x72 --samples 48
```

`--yaw 135` is the camera at the building's south-east corner, the ring's 45 deg frame; keep
its `frame_00.png` and pack it with `--facings 1`. A 2x2 footprint plus the kit's eaves is 2.14
units across, so the cottage uses a 224 px cell at `--span 4.4269` = 50.6 px per kit cell, the
same scale as the five kinds below. The cameo is the same finished model at pitch 35 in a
96x72 cell. The contact shadow lands under the building and reads as a fringe at the base.

## Build the building kinds

`build_buildings.py` holds one assembly per kind, on the footprint the sim already uses
(`crates/kam2-sim/src/economy.rs`): warehouse 3x3, farm/mill/bakery/woodcutter 2x2. It
writes one `.glb` per state; the kit is never committed.

```powershell
blender --background --factory-startup --python tools\kam2\build_buildings.py -- --list
blender --background --factory-startup --python tools\kam2\build_buildings.py -- --kit "<pack>\Models\GLB format" --out $env:TEMP\kam2 --kind mill --state finished
blender --background --factory-startup --python tools\kam2\render_frames.py -- --scene $env:TEMP\kam2\mill\finished.glb --out $env:TEMP\kam2\mill\finished --yaw 135 --size 288 --span 5.6917 --ground-px 216 --pivot-x 1 --pivot-y 1 --samples 48
```

`--yaw 135` renders one frame at 45 deg off the S axis instead of walking the ring; it is the
same camera as ring frame 01 and is what the building atlases use. `--pivot-x/--pivot-y` fix a
world pivot: the camera aims at that point, so every state of one building lands on the same
pivot pixel even when the alpha box differs. Both flags are optional and leave the old ring
behaviour unchanged.

Receipts, exact spans, pivots and the byte-identical rebuild proof:
`content/kam2/buildings/README.md`.

## Pack an atlas

```powershell
node tools\kam2\pack_atlas.mjs --in <frames dir> --out <dir> --id unit.swordsman --kind unit --anim idle:1:2:true --footprint 1x1 --pivot auto
```

Input files are sorted by number and read facing-major: `facings` files per animation frame,
anims in `--anim` order. `mask_NN.png` files are skipped here and read by `--mask-dir`.

| flag | default | meaning |
| --- | --- | --- |
| `--in`, `--out`, `--id`, `--kind` | required | frames folder, output folder, stable id, `unit\|building\|tile\|prop` |
| `--facings <n>` | 8 | facings per frame (buildings use 1) |
| `--tile <px>`, `--tile-h <px>` | PNG size | cell size; must match every PNG |
| `--pad <px>` | 1 | transparent gutter around each cell |
| `--pattern <regex>` | every PNG | only names matching are frames (a cameo or sheet in the folder would otherwise be counted) |
| `--anim name:frames:fps:loop` | `idle:1:2:true` | repeatable; rows are allocated in order. Optional fifth field `:facings=N` for a row that holds fewer than the set's facings (an S-only animation) |
| `--pivot x,y\|auto` | `frames.json` pivot | ground anchor in cell px |
| `--footprint WxH` | 1x1 | tiles covered in the world |
| `--turret-anchor x,y` | none | turret pivot in cell px |
| `--mask-dir <dir>` | none | pack `mask_NN.png` into `<id>.mask.png` |
| `--cameo <file>` | none | copy beside the atlas and record the name |
| `--cameo-cell <WxH>` | none | pack the cameo as `<id>.cameo.png` with the gutter and record `{id, atlas, cell, size, padding, pivot}` |

Layout: column = facing, row = animation frame. The manifest carries the style-lock fields
`id, kind, tile_w, tile_h, pivot, footprint, turret_anchor, facings, anims (name, frames, fps,
loop, row), atlas, mask_atlas, cameo`, plus `atlas_size`, `padding`, `source_frames`,
`facing_count`. `tile_w`/`tile_h` are the sprite cell in the atlas (64x64 by default), not the
64x32 ground diamond. One atlas is capped at 2048 px per side; the packer refuses bigger sets.

## Rig and framing

- Camera: orthographic, pitch `atan(0.5)`, one span and one pivot pixel shared by all facings,
  so frames stack without drift.
- Shading is screen-locked: the sun rig rotates with the camera, so every facing reads with the
  same light. Key `#FFE8C4` 3.2 from front-right, fill `#6E86A8` 0.55 from front-left, ambient
  `#3A4250` at 0.5. The lock's `-40 deg from camera right` points behind the unit in a PBR
  renderer and leaves every front face black; the sign is flipped here, and the fill/ambient
  weights are lifted so the shade side reads instead of crushing. All numbers are constants at
  the top of `render_frames.py`, one place to tune after M1 review.
- Contact shadow: soft black ellipse at 35%, 2 px along the sun.

## Not in ART0 yet

- Cameo pass (96x72, pitch 35) and the separate turret pass: manifest fields exist and stay
  `null` until those passes land.
- The renderer emits one pose per run; an animation is rendered per frame (with `--frame`) into
  separate folders and packed with several `--anim` specs.
- Terrain tiles, cliffs and water need their own scene templates, not a new tool.
- No mipmaps in the atlas (style lock).
