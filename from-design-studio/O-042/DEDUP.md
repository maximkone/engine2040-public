# DEDUP O-042: what to skip

No byte-exact dups.
All SHAs differ (checked 28 files, 16-char prefixes all unique).

## Logical dups: 3 groups

1. Font in 4 forms, same 95 glyphs, 22698 B total.
font-5x7.png 811 + font-5x7.json 630 + font-glyphs.json 8365 + ds-pixel-5x7.ttf 12892.
Keep: ttf + font-5x7.json for the engine.
Skip on import: font-glyphs.json (source only).

2. Icons in 3 places, same 12 shapes.
icons.png 419 + icons@2x.png 626 + 12 icon rects inside atlas.png 1169.
Keep: atlas.png rects for the engine.
Skip on import: icons.png, icons@2x.png (proof only).

3. Pixels re-rendered as proof, ~142 KB.
atlas.png 1169 + layout.json 17515 build all previews.
preview-*.png 46036 + out.png 79328 + thumb-256.png 16858 are derived.
Skip on import: all preview-*.png, out.png, thumb-256.png, page.html.

## Win
Engine loads 7 files, 39283 B.
Skip list saves ~146 KB per import.
No bytes deleted; this file is the skip list.
