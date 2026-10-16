# Compress note — content/ only, no deletes

Date: 2026-10-05. Scope: content/ only. No files removed. No binaries changed.

Biggest compressible sets (measured today):

- arena/textures 12x *_2k.jpg = 22.42 MB. 1k or quality 80 cuts ~75%. Keep 2k as source.
- arena/previews + maps/previews 23x PNG = 14.22 MB. All 648,413 bytes each. Regenerable. Do not ship.
- fighters/atlas 20x PNG = 11.66 MB. The q0-q3 chain is the compression ladder already.
- arena/hdri 3x HDR = 70.33 MB. Already LFS. Do not recompress. Leave as is.

Win: ship 1k textures + one atlas level + zero previews. Saves ~30 MB shipped. Source files stay.
