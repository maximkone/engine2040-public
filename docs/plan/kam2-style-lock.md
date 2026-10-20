# KAM2 style lock - RA2-look 2.5D, measured

Every number here is fixed before art volume starts. Changing one later means re-rendering
everything that used it, so changes are owner decisions.

## Projection and grid

- Orthographic camera, pitch `atan(0.5)` = 26.565°, yaw fixed at 45° multiples.
- Tile 64 x 32 px (2:1 diamond). The world grid is square; the projection makes it diamond.
- Height levels: 4. One step = 16 px of screen lift. Cliff face = one tile wide, two tiles tall.
- Zoom: 100%, 125%, 150% only; sprites render at 1:1 and scale by integer steps with
  nearest-neighbour above 100% to keep the RA2 crispness.

## Facings

- Eight per mobile unit, clockwise, index 0 = facing the camera (south).
- Order: S, SW, W, NW, N, NE, E, SE.
- Buildings do not rotate.
- Vehicles carry a second sprite set for the turret, pivoting around `turret_anchor` in the
  manifest, with the same eight facings but drawn above the hull layer.

## Sprite sizes at 100%

- Infantry 34 px tall, 24 px wide box.
- Cavalry 44 px tall.
- Vehicle hull 40-56 px long, 32-40 px tall; turret 20-32 px.
- Siege 56-72 px.
- Buildings: 2x2, 3x3 or 4x4 tiles; height up to 3 levels; footprint mask in the manifest.
- Trees 40-64 px; rocks 16-32 px; wares 10-14 px.

## Light

- One sun, elevation 50°, azimuth -40° from camera right.
- Key `#FFE8C4` at 3.2, fill `#6E86A8` at 0.35, ambient `#3A4250` at 0.15.
- Baked ambient occlusion; contact shadow baked into the sprite as a soft ellipse at 35%
  black, offset 2 px along the sun.
- Hard specular on metal, none on cloth. Rim darkening on the shadow side; no outlines.

## Palette

Base ramps (sRGB). Team colours replace the mask channel at runtime.

| Use | Hex |
| --- | --- |
| grass light / dark | `#8FA84E` / `#5E7433` |
| dirt light / dark | `#C9A26B` / `#8A6A43` |
| sand | `#D9BE86` |
| rock | `#8A8172` |
| water shallow / deep | `#4E8FB5` / `#1F4E6E` |
| wood | `#8B5A2B` |
| plaster | `#D8CDB4` |
| roof red | `#A33B2E` |
| roof slate | `#3E4A57` |
| steel | `#9AA3AB` |
| gold | `#D9A441` |
| shadow | `#1B1F24` |
| team blue | `#2B6FD6` |
| team red | `#C6372E` |
| team green | `#3E8E4E` |
| team yellow | `#D8B02A` |
| team purple | `#7A4FA3` |
| team black | `#23262B` |

Saturation sits higher than KaM and lower than toy art: RA2 reads at a glance, so value
contrast carries the silhouette and hue separates factions.

## Animation

- 10 fps for infantry and workers, 12 fps for vehicles, 8 fps for buildings' idle smoke.
- Walk 8 frames, attack 6, die 8, idle 2, work 6.
- Loop points recorded per clip in the manifest; no per-frame metadata in the atlas.

## Team colour

- Second render pass: flat emission white on the team areas, black elsewhere, stored as the
  alpha of a mask atlas.
- Runtime tint: multiply base by team colour, keep luminance; the mask never ships as art.

## Cameos

- Built from the same model: yaw 45°, pitch 35°, ortho, 96 x 72 px, transparent background.
- One cameo per building and per unit; the sidebar uses them at 64 x 48.

## Atlas and manifest

- 2048 x 2048 max per atlas, 1 px transparent padding, no mipmaps, alpha test 0.5.
- Manifest per asset: `id`, `kind`, `tile_w`, `tile_h`, `pivot`, `footprint`, `turret_anchor`,
  `facings`, `anims` (name, frames, fps, loop), `atlas`, `mask_atlas`, `cameo`.
- Ids are stable: `unit.swordsman`, `house.farm`, `house.farm.stage2`, `tile.grass.01`.

## Camera and UI

- The view never rotates. Edge scroll and minimap click only.
- Sidebar right side, 176 px at 100%, chrome from the CC0 UI set, parchment panels.
- Fonts: OFL only (Cinzel for titles, IM Fell for body), bitmap-safe at 12 and 14 px.
