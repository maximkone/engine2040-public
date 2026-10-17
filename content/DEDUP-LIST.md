# Dedup list — content/ only, no deletes

Date: 2026-10-05. Same bytes found by SHA256. Nothing removed. Pick one live copy per group.

Exact hash dupes:

- 2,072,320 bytes x3: fighters/glb/hero_body.glb = fighters/ct21-recook/hero_export_a.glb = fighters/ct21-recook/hero_export_b.glb. Live: glb/hero_body.glb.
- 815,261 bytes x2: fighters/previews/hero_wire.png = fighters/ct21-recook/hero_wire.png. Live: previews copy.
- 806,620 bytes x2: fighters/previews/hero_rest.png = fighters/ct21-recook/hero_rest.png. Live: previews copy.

Same-size groups (check before use):

- 648,413 bytes x23: all arena/previews + maps/previews PNG files. Likely clones. Eyes only.
- 18,236 bytes x2: arena/textures/metal_roughness.png = maps/textures/metal_roughness.png.
- 663 bytes x2: arena/textures/flat_normal.png = maps/textures/flat_normal.png.
- 797 bytes x2: materials/reference/cloth.json = materials/siege/cloth.json.
- 912 bytes x3: materials/reference/marble.json = materials/siege/marble.json, near materials/siege/stone.json.
- 3,879 bytes x2: maps/manifest.json = maps/manifest_run1.json. Same for ludus report/manifest pairs.

Unused (read-never, eyes or record only): previews, ref-sheets, report_run1, ct21-recook exports.
