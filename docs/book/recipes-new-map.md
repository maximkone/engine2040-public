# Recipe: new map

Ship a new arena as a deterministic recipe plus a TPL1 arena item:
same seed plus recipe plus size gives the same bytes, and the placed
arena stays spatially clean. Full signatures live in rustdoc:
`cargo doc -p engine-world --no-deps` and
`cargo doc -p engine-editor --no-deps`.

## 1. Bake the ground (blockout retired)

> Retired 2026-10: `crates/world/src/blockout.rs` (`bake_blockout`,
> `effective_seed`, `blockout_hash`, recipe `"arena-blockout"`) was
> deleted with no live caller. Do not build on it; what follows is the
> live cell path.

Map ground comes from `crates/world/src/cells.rs`:

- `bake_cell(seed, coord, size)` bakes one cell: header, eroded
  heights, isolated-hole bits. Same seed plus coord plus size gives
  the same bytes (`cell_streaming_bakes_byte_identical_x2`).
- `Cells::stream`/`publish` streams cells through a byte budget at
  frame boundaries; `seam_joins` scans shared edges and the publish
  gate refuses broken seams with the check named.
- Underneath, `engine-world::bake(seed, size)` still emits the legacy
  versioned whole-map bytes: magic `W204`, version 1, seed, size,
  then `size × size` heights (integer lattice plus smootherstep,
  0–255).

A new map is a new seed (recipes now live in the scatter seed mix,
`crates/world/src/scatter.rs`): different seed or size gives
different bytes by construction.

## 2. Create the arena item through the TPL1 template

The arena starter goes through the editor command service:
`Shell::template_apply(TemplateKind::Arena, name)` in
`crates/editor/src/shell.rs` over `TemplateService` in
`crates/editor/src/templates.rs`; over MCP it is `tx.preview`
then `tx.apply` (see [Drive it from MCP](recipes-drive-from-mcp.md)).
The staged delta must add no spatial findings — combined templates
stay spatially clean.

## 3. Verify headlessly

Bake determinism plus the templates (both verified this wave):

```powershell
cargo test --locked -p engine-world --test cell_streaming
cargo test --locked -p engine-editor --lib templates
```

Expected: world streaming green (9-cell walk, seam nest, rebake
byte-identical), editor `8 passed`. Then run the
scripted match on the new content dir from the
[quick start](quick-start.md) to prove the map plays:

```powershell
cargo run --locked -p engine-game --features app --bin staged_fight -- --ticks 600 --out-dir target/newmap-check
```

Expected: `staged_fight done: ticks=600 ... captures=4` with a
manifest and receipt. Twice with different out dirs gives the same
`final_hash`.

Gate notes: this recipe is fully headless. Baked lightmaps, sky
LUTs, fog tint, and 60 fps bars are reference-GPU gates owned by
the RL rows (see the [presentation page](systems-presentation.md))
and are not claimed here. Board rows WD (world bake) and TPL1
(arena template) own this recipe.

## Reference

- `crates/world/src/lib.rs` — `bake`, `height`
- `crates/world/src/cells.rs` — `bake_cell`, `Cells`, `seam_joins`
- `crates/editor/src/templates.rs` — `TemplateKind::Arena`
- `crates/game/src/bin/staged_fight.rs` — `--content-dir`, `--out-dir`, `--ticks`
