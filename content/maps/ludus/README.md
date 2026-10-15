# WD2 ludus hub map (Tier C) — `content/maps/ludus/`

Playable Rome loop: the ludus hub boots from the lobby over the GP4 ladder.
Practice yard (drill floor, two pells, weapon rack), blacksmith corner (hut,
forge, stump, anvil with a forge interact point), party board corner (posts,
panel, three slips with a board interact point), plus the barracks hall and
the boundary wall. Nothing outside `content/maps/ludus/` is touched.

## Contents

| Path | What |
|---|---|
| `map.json` | The hub: seed, sky, 8 actors (the CT9 `placement_ludus.json` probe verbatim), 3 zones, 4 spawns, nav rect + routes, stream block |
| `cells.json` | WD1 stream manifest: hub 2x2, size 16, per-cell priority and baked cost |
| `content/maps/ludus/tools/assemble.mjs` | Deterministic assembler + validator (node stdlib only): re-emits `map.json`, `cells.json`, `content/maps/ludus/tools/map_data.rs`, `manifest.json` byte-identically |
| `content/maps/ludus/tools/map_data.rs` | Generated Rust bindings (actors, zones, spawns, routes, nav rect, cells) |
| `manifest.json` | sha256 + bytes per generated file (the provenance admits) |
| `manifest_run1.json` | Pre-regen copy, byte-identical to `manifest.json` (assembler determinism) |
| `proof/` | Standalone proof crate: only landed `engine-assets` / `engine-spatial` / `engine-nav` / `engine-world` APIs |
| `report.json` | Proof output: cook, bundle, spatial, nav, cells, replay |
| `report_run1.json` | Previous proof run, byte-identical to `report.json` (replay determinism) |
| `PROVENANCE.md` | One row per asset: CC0-only sources, Quaternius REFUSED, bundles/incoming excluded |

## Conventions

No new geometry: every actor renders a CT9 kit mesh (`../src/*.gltf`,
`../textures/*.png`, `../hdri/maps_day.hdr`) through the CT2/A2/AS1 cook.
Zones nest by design — the workshop and board corners sit inside the training
grounds, disjoint from each other. Spawns stand on free ground (the drill
floor is ground, not an obstacle). The stream block is the WD1 hub 2x2 at
size 16; the nav rect is baked flat at 1 m cells and every route endpoint was
validated free before the proof ever ran.

## Budgets (proven in `report.json`)

- Kit triangles: 7 ludus assets, worst rack 1920 LOD0 tris (CT2 `PROP_MAX_TRIS` = 5000).
- Textures: albedos 512, data 256, every mip chain to 1x1 (A2 rules).
- Spatial `run()` over the placed hub: 0 findings, verdict clean, hash x2.
- Nav: yard rect walkable, every drill route walks, bake identical x2 and under shuffled input.
- Cells: hub 2x2 bakes identical x2, seams join, 4/4 live with zero hitches, boundary rebake byte-identical, sim hash untouched by publish.
- Replay: the full pipeline twice gives one report byte.

## Reproduce

- `node tools/assemble.mjs` (from this dir) — regenerates byte-identical files.
- `cargo run --offline` (from `proof/`) — re-proves; exit 0 + GREEN.

Consumer: the WD1 map assembler streams `cells.json` through the AS2 queue,
gates publish via WD7, and walks `map.json` spawns over the baked nav. LW3
voices staff the forge and the board; AB8 builds its practice corner on this
map. No window GUI runs in this environment (no display server), stated, not
hidden.
