# KAM2 art: first rendered unit and first rendered building kinds

`buildings/` holds the five economy kinds as rendered atlases, each on its sim footprint:
`house.warehouse` (3x3) and `house.farm`, `house.mill`, `house.bakery`, `house.woodcutter`
(2x2). One yaw 45 deg off S, two rows (`finished`, `half_built`), native scale 50.6 px per
kit cell. Receipt and rebuild proof: `buildings/README.md`.

`house/` is the first building: `house.cottage`, a Kenney Fantasy Town Kit cottage on a 2x2
footprint, three states (finished, half-built, damaged) in one 130x390 atlas. Receipt and
numbers: `house/README.md`; source and licence: `house/source.json`.

`unit.soldier.atlas.png` is the first real sprite sheet of the game: a Kenney Blocky
Characters model (`character-j.glb`, CC0 1.0) rendered here in Blender 5.2 / Cycles CPU
at 64x64 per cell, orthographic, pitch 26.5651 degrees, eight yaws of 45 degrees.

- 528x594 px, 9 rows x 8 columns, 72 cells, 1 px transparent gutter, no empty cell.
- Row 0 is the single walking pose, rows 1-8 are walk frames 0,2,...,14 of the 16-frame
  clip at 10 fps.
- Pivot 32,56. Body 34 px from ground to crown, 37 px silhouette, 42 px alpha box with
  contact shadow.
- Source and licence: `source.json` (Kenney, CC0 1.0, no account needed).
- Reproduce: `tools/kam2/README.md`.

Known at this stage: the clip has no foot planting (frames 4 and 12 lift both feet), and
there is no team-colour mask pass yet. Both are look-lock items, not defects of the
pipeline.
