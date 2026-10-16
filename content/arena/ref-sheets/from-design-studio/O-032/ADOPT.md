# ADOPT O-032: reference sheets for engine2040 materials + arena look
Adopt: commit these bytes, then point the art seat at `content/arena/ref-sheets/from-design-studio/O-032/` sheets for the next material drop.
Loader (read-only): `crates/assets/src/mesh_import.rs` takes glTF bufferView slices + `data:` URIs only; JPEG/PNG max 2k POT; ORM per ORM-PROVENANCE.md.
Check: source SHAs re-verified against `content/arena/manifest.json`.
Proof: `node tools/game-ui.mjs --check` (GAME-UI PASS) per DELIVERY.md.
