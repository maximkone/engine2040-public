# INDEX O-042: what the engine loads

Order matters. Load top to bottom.

## Load (7)
1. tokens.json: palette, reference 640x360, scales x2/x3/x4.
2. atlas.json: 27 pieces, image atlas.png.
3. atlas.png: 256x64, nearest, 9-slice.
4. font-5x7.json: cell 6x8, first 32, last 126.
5. font-5x7.png: 96x48 white glyphs, tint by text color.
6. ds-pixel-5x7.ttf: same glyphs as outlines for headings.
7. layout.json: 5 screens (hud, menu, pause, settings, inventory).

## Proof only (do not load)
preview-menu.png, preview-pause.png, preview-settings.png, preview-hud.png, preview-inventory.png, preview-atlas.png, out.png, thumb-256.png, page.html, icons.png, icons@2x.png.

## Docs (read, do not load)
ADOPT.md: 3-step swap plan.
DELIVERY.md: kit contents and loader files.
PULL.md: fresh file list.
DEDUP.md: skip list.
VERDICT.md, DESIGN-REVIEW.md, EYE.md: pass proof.
brief.json, brief-gate.json, design-audit.json: order proof.
font-glyphs.json: glyph source.
tokens.css: web mirror of tokens.json.
