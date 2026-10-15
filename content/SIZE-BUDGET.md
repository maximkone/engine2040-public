# SIZE-BUDGET — content (2026-10-13)

Proof: `rg --files content` summed via node stdlib: 145.3MB/566 files → 96.9MB/564 files.
Kept on disk: moved (not deleted) `arena_night.hdr` (26.7MB) + `arena_sunset.hdr`
(21.8MB) → `content/local/hdri/` (gitignored cache). Tracked `arena_day.hdr`
(21.3MB) stays canonical; previews + `arena/hdri/PROVENANCE.md` untouched.
Restore: re-download via PROVENANCE.md URLs/SHAs, or copy back from `content/local/hdri/`.

Budget (tracked `rg --files content` MB): total ≤110. arena ≤95 (hdri ≤25:
one 4k HDR + previews), fighters ≤27, maps ≤12, sprites ≤4, audio ≤3,
props ≤1, crowd ≤1, rest ≤1. Next wins if needed: downsample `arena/src/*.gltf`
(37MB) + `arena/textures/*_2k.jpg` (19MB); fighters `ct21-recook/*.blend` (1.3MB).

Gitignore untouched: `target/`, `/build/`, `content/fighters/bundle/`,
`content/arena/bundle/`, `content/local/`, `content/incoming/` already ignored.
Note: `git ls-files content` still reports ~189MB (44MB ignored-but-tracked
history); needs owner `git rm --cached`, not done here.
