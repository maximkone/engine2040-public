# PULL O-042: fresh pull list

HEAD 7706d9e at 2026-10-05T20:14Z.
Fetch shows 0 behind origin/master.
Turf was clean before this pass.

## Base drop: 28 files

Engine load (7 files, 39283 B):
atlas.png 1169, atlas.json 5034, font-5x7.png 811, font-5x7.json 630, ds-pixel-5x7.ttf 12892, tokens.json 1232, layout.json 17515.

Proof only (10 files, ~146 KB, skip on import):
preview-atlas.png 6564, preview-hud.png 6720, preview-menu.png 7962, preview-inventory.png 8216, preview-pause.png 8283, preview-settings.png 8291, out.png 79328, thumb-256.png 16858, page.html 3550, icons.png 419, icons@2x.png 626.

Docs (11 files):
ADOPT.md 389, DELIVERY.md 1048, VERDICT.md 592, EYE.md 473, DESIGN-REVIEW.md 1123, brief.json 881, brief-gate.json 952, design-audit.json 2599, font-glyphs.json 8365, tokens.css 434, plus this file and DEDUP.md and INDEX.md.

## Stale imports fixed in this pass
brief-gate.json, design-audit.json, DESIGN-REVIEW.md: absolute PC paths now relative.
tokens.css, page.html: designs/O-042 prefix now from-design-studio/O-042.
tokens.json: kits/game-ui path now tokens.css mirror.
DELIVERY.md: FORMATS.md marked as design-studio side, not in this drop.
