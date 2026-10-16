# Content systems: assets, scenes, space, editor

Content flows one way: source plus settings plus importer version
gives a cooked artifact with a blake3 digest. The editor, scripts,
the CLI, MCP clients, and in-engine providers all use the same
command loop: validate, preview without side effects, atomic apply,
journal, undo. Full signatures live in rustdoc:
`cargo doc -p <crate> --no-deps`.

## Assets and saves

- `engine-assets` owns glTF mesh and material import (reimport keeps
  IDs), texture cook with mips and sRGB rules, skinned mesh/skeleton/
  clip import, polling watch with debounced recook, setting-driven
  repairs, the content bundle v1 (cooked meshes, textures, skins,
  clips packed with blake3 digests), and the native animation and
  mesh cooks (retarget by bone map, root motion and foot contacts,
  LOD chain, collision hulls, `FRAC_` chunks, budget checks at
  import). Cook is identical twice.
- Shipped content lives in `content/fighters/manifest.json`: the hero
  plus three opponents on one 45-joint skeleton, 1024 sRGB textures,
  14 clips each, with per-variant bundle and body SHAs. The staged
  fight binds all four (`VARIANTS` in
  `crates/game/src/fighter_bind.rs`).
- `engine-save` owns the versioned envelope and bounded generations
  with failed-write recovery. A corrupt save falls back to the
  previous generation.

## Scene, spatial, editor

- `engine-scene` owns persistent IDs, transform hierarchy, schema
  metadata, and the versioned codec. Identity is three things:
  generational handles (live), UUIDs (authored), digests (versions).
- `engine-spatial` owns scene understanding as deterministic,
  model-free core service: staged before/after gate, relation graph,
  bounded brief, intent placement solver (`next to wall:north`
  becomes a transform in deterministic code — models give intent,
  code owns coordinates), semantic diff, and revision-bound repairs.
- `engine-editor` owns the dock shell over command batches, viewport
  picking and gizmo to batch, save/reopen, child-process play,
  durable journal, provenance receipts, capability tokens, snapshots
  with instant restart, selective undo, the findings panel, live
  tuning (combat/AI numbers at tick boundaries, replay-identical),
  and TPL1 templates: arena, prop, weapon, and fighter starters.
  Every template previews, applies through the command service,
  imports, validates, plays, and undoes.

Check the templates headlessly (verified this wave):

```powershell
cargo test --locked -p engine-editor --lib templates
```

Expected: `8 passed` (each template previews, applies, imports, and
undoes; taken names refuse with a suggestion; combined templates
stay spatially clean).

To add a weapon through tables plus templates, see
[Add a weapon](recipes-add-weapon.md). To place a new arena, see
[New map](recipes-new-map.md).

## Reference

- `crates/assets/`, `content/fighters/manifest.json`
- `crates/scene/`, `crates/spatial/`
- `crates/editor/src/templates.rs`, `crates/editor/src/shell.rs`, `crates/editor/src/commands.rs`
- `crates/game/src/fighter_bind.rs`, `crates/save/`
- `docs/EDITOR_AI_CONTROL.md`, `docs/ARCHITECTURE.md`
