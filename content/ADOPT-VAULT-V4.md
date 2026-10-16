# ADOPT vault V4: asset-vault engine2040-v4 manifest in use

Order O-004 (asset-vault manifest engine2040-v4: 133 commercial-ok records, 131 verified at intake).
Bytes: V4 files in final dirs — `content/arena/textures/` (73 files), `content/arena/hdri/` (9 files), `content/audio/ambience/` + `footsteps/sfx-*` (24), `content/sprites/kenney-*`.

Adopting commit SHA: `47febab` (Wave E builders 2026-10-06: V4 files in final dirs; maker record asset-vault O-004 already cites 47febab).
Use, verified 2026-10-08: texture, hdri, ambience, footstep and sprite dirs hold the landed V4 set on disk.
Note: full-res V5 + V6 intake is open row E24 (inbox EB-2026-10-06-S31/S65); this file proves V4 only.

Proof: `git show --stat 47febab` + `Get-ChildItem content/arena/textures | Measure-Object`.
