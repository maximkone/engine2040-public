# DELIVERY O-032: reference sheets for engine2040 materials + arena look
1. Landing in engine2040: `content/arena/ref-sheets/from-design-studio/O-032/` (copy this folder; never edit engine files outside from-design-studio/).
2. Eight sheets `out-1200x*.png` (sand/steel/skin/marble/wood/cloth/arena-day/torch-night), each 4 tiles + look note (color; roughness; wetness; wear); overview `out.png` 1280x720 + `thumb-256.png`.
3. Tiles `assets/` (28 PNG, CC0): Poly Haven crops via staged engine sets + authored look targets; `assets.json` carries bytes + sha256; sources + licenses in SOURCES.md.
4. Engine loader (read-only): `crates/assets/src/mesh_import.rs` takes glTF bufferView slices + `data:` URIs only (external URIs fail cook); JPEG/PNG max 2k POT; ORM per ORM-PROVENANCE.md.
5. Skin has no engine asset and marble binds sandstone: those sheets are look targets (NEED-11 tracks real CC0 photo sets); HDRIs referenced by manifest SHA, not rasterized.
6. Gates: `brief.json` (system fantasy), `design-audit.json` (AUDIT PASS), `DESIGN-REVIEW.md` (judge 10/10 SHIP); source SHAs re-verified against `content/arena/manifest.json`.
7. Adopt: commit these bytes in engine2040, then point the art seat at the sheets for the next material drop.
8. Proof here: `node tools/orders-check.mjs --built O-032` (BUILT PASS) and `node tools/game-ui.mjs --check` (GAME-UI PASS).
