# Editor plugins read as door verbs

Read 2026-10-23. engine2040 has no editor panels by design (D4/D6): the door is
the only way in, one typed verb per action, each undoable or read-only, refused
by default, priced before it runs (`docs/plan/ai-door.md`). So every editor
feature has to become a verb. This asks the other editors what their plugin
actually does, and what verb that gesture implies for us.

Ground rules used for every candidate:

- Permissive licence (MIT/zlib) = a real port. GPL or paid = ideas only, no
  code read for shape beyond the interface, never linked.
- The verb must be reproducible headless (no window, no GPU) and must land
  through the existing gate (`expected_revision` + `idempotency_key` +
  `tx.preview` approval + `tx.undo` token), not beside it.
- Donor facts are static reads of the pinned files below. Nothing but our own
  crates was executed.

## What the door already has (so a steal is only what is missing)

| Fact | Evidence |
|---|---|
| 100 verb-name constants | `TOOL_*: &str` count over `crates/endpoint/src` |
| one-object placement with want lines, solver repair hints, gated apply, undo key | `crates/endpoint/src/scene_tools.rs` `ScenePlaceArgs`, `RepairHintDto` |
| preview -> single-use approval -> apply -> journal undo by token | `crates/endpoint/src/mcp.rs` `tx_preview`/`tx_apply`/`tx_undo` (`undo_mismatch`) |
| a batch is at most 256 ops | `MAX_TX_OPS = 256`, `crates/endpoint/src/mcp.rs:120` |
| the editor op set has no batch/paint op | `Op::{Insert,Remove,SetLocal,Reparent,SetMaterial,SetComponent,...}`, `crates/editor/src/commands.rs:135` |
| level briefs already place 24 props and bake a 13x13 nav grid | `crates/endpoint/src/build_level.rs` (`nav_tiles`, 169-tile test) |
| a non-default pack costs 0 chars of the 75,000 tool-list budget | `crates/endpoint/src/mcp.rs` budget ledger |

Two deterministic libraries exist with **no door verb at all**:

- `crates/world/src/wfc.rs`: seeded wave function collapse over a tile set and
  adjacency table; its own header names "the `world.bake`-style door row" as
  the missing consumer.
- `crates/world/src/scatter.rs`: seeded Bridson Poisson-disk points
  (PORT Kromey `@5c8273cb`, recipe mixed into the seed), bake-time only.

That is the thesis of this note: the cheapest editor-feeling verbs are the ones
where the substrate already landed. Only the gesture is missing.

## Candidates

| Donor (pinned, licence) | Gesture it replaces | Data it writes | Headless | Verb it implies |
|---|---|---|---|---|
| Godot `modules/gridmap/editor/grid_map_editor_plugin.cpp` `@b2220b8da944daad6d87fd365a30de56575bca90` 2026-08-31 (MIT) | pick a mesh from a palette, drag over a grid to paint/erase; bucket-fill a selection; rotate a selection | one `set_cell_item(cell, item, orientation)` per cell; paint is one undo action ("GridMap Paint") | data model yes (GridMap is core); the editor plugin itself is UI | `level.paint`: cells + kit palette -> one batch of `Op::Insert` |
| LDtk `src/electron.renderer/data/def/AutoLayerRuleDef.hx` `@fd27ad4a21…` 2024-01-11 and `app/extraFiles/samples/AutoLayers_2_stamps.ldtk` `@1bb00471269f5ff98a17d9dad8c82b5980ded57b` (MIT) | author a tile rule once, then every layer cell is computed from its neighbours | rule: `pattern`, `chance`, `breakOnMatch`, `tileMode` (`Single`/`Stamp`), `tileRectsIds`, `flipX/Y`, `xModulo`, perlin offsets | yes (Haxe CLI, JSON in/out) | rules are arguments of `level.paint`, not a second verb: the same cell grid, computed instead of clicked |
| Cyclops Level Builder `godot/addons/cyclops_level_builder/LICENSE.md` `@76d7712aa98aca2ffc34edfb7480a80e842acd41` 2023-12-14 (MIT, Mark McKay) | drag a box, extrude a face, cut a hole; blockout with CSG | brush polygons (mesh), exported as glTF or a Godot scene | partly: export actions are scripts; geometry needs the editor | `level.brush` (not ranked: real CSG code, see below) |
| Trenchbroom `lib/TbMdlLib/src/BrushBuilder.cpp` `@0c27147164a1aa73c0819f57d877c7e2d7492db0` 2026-08-26 (**GPL-3.0**) | create cube/cylinder/cone/sphere, then clip and bevel the solid | brush planes + material name | no GUI-free path | ideas only: a brush is a closed plane set, so a blockout verb can be `plane[]` in |
| Godot `modules/navigation_3d/3d/nav_mesh_generator_3d.cpp` `@2dd1f4ef604608c46fa42c2dd2df1055fee5c48b` 2026-03-05 (MIT), Recast `License.txt` (zlib) | press "Bake NavigationMesh": parse source geometry, then agent radius/height/climb and cell size decide the walkable mesh | navmesh polygons + bake state; source geometry carries the root and group filter | yes (`bake_from_source_geometry_data`, worker task) | `nav.mark` + a bake read: mark boxes (block, cost, link) over the existing bake |
| Godot `scene/resources/bone_map.cpp` `@eabb9a63d4…` 2026-03-04 (MIT) | retarget dialog: map profile bone names to this skeleton's bone names | profile -> skeleton name table, saved with the rig | yes (resource, no UI needed) | `anim.retarget`: source bone names -> our 17 roles, coverage in permille |
| ink `compiler/Compiler.cs` `@7fd74c5f43…` 2021-09-01 and `ink-engine-runtime/Story.cs` `@4864a8667c5fa30873134448fa45abb7ab96f5fb` 2024-05-22 (MIT) | write prose with branches, compile, then play and pick a choice | compiled story JSON; `Story.ToJson()` saves the run, `ChooseChoiceIndex(i)` steps it | yes (`inklecate` is a CLI compiler and player) | `dialogue.compile` + `dialogue.step` (bigger steal, see below) |
| Yarn Spinner `YarnSpinner.Compiler/Compiler.cs` (MIT) | same gesture as ink, nodes instead of prose | node graph + compiled program | yes | alternate donor; ink wins on being text-first (a weak model writes it) |
| Blender Cell Fracture (GPL-3.0-or-later, already spiked 2026-10-23) | break a wall into cells | chunks + seed | yes (`-b`) | nothing new: `break.fracture` exists and the spike row already covers it |
| Godot `core/object/undo_redo.cpp` `@1c45c14d7f…` 2026-07-16 (MIT) | a drag becomes one undo step via `MERGE_ENDS`/`MERGE_ALL` | action list with a merge window | yes (core object) | **no verb**: our 256-op batch already is the stroke; merge would be a second way to spell it |

Not read live, therefore not ranked: Houdini HDAs (proprietary; the idea is
"parameters + recipe", already how `level.build` works), Unity ProBuilder,
Polybrush and NavMeshModifierVolume (paid/Companion licence, ideas only).

## Proposed verbs

Shapes follow the house gate: mutation takes `expected_revision` and
`idempotency_key`, returns the new `revision`, the rows and an undo `key`; a
refusal is a stable `code` plus one line naming the fix.

| Verb | Args | Returns | Refusal |
|---|---|---|---|
| `level.paint` | `expected_revision`, `idempotency_key`, `kit` (palette name), `cells` (`[{x,z,yaw}]` or `{cx,cz,w,h}` regions), `rules?` (LDtk-style: `pattern`, `chance`, `tile`, `flip`), `scatter?` (`recipe`, `seed`, `min_dist`, `count`), `max_cells?` | `revision`, `cells`, `ops`, `ids` (first/last, deterministic), `digest`, `effects`, `key` | `too_many_cells` (>256, names the count and asks for a smaller region), `no_kit` (lists kits), `off_grid` (cell outside the level bounds), `seed_required` (scatter without a seed), `stale_revision`, `duplicate_key`, `apply_in_flight` |
| `anim.retarget` | `source` (bone-name list or an `asset.intake` clip), `profile` (`humanoid`), `overrides?` (`[{from,to}]`, method `manual`) | `mappings` (`role`, `source`, `confidence_permille`, `method`), `coverage_permille`, `missing` (roles), `digest` | `unknown_bone` (names it and the nearest alias), `duplicate_source`, `missing_roles` (lists the 17), `bad_override`, `not_a_clip` |
| `nav.mark` | `expected_revision`, `idempotency_key`, `marks` (`[{box:{min,max}, walk:"block"\|"cost", cost_milli} \| {from,to,kind:"ladder"\|"jump"}]`), `bake?` | `revision`, `marks`, `tiles`, `islands`, `digest`, `key` | `bad_box` (min >= max on the named axis), `cost_range`, `off_level`, `link_unreachable` (both ends named), `stale_revision` |
| `dialogue.compile` | `source` (text, <=64k chars), `name` | `digest`, `knots`, `start`, `choices` (text + index) | `parse_error` (line, column, expected token), `too_long`, `bad_encoding` |
| `dialogue.step` | `digest`, `choice` (index, `-1` = start), `state?` (opaque save blob) | `lines`, `choices`, `state` (blob), `state_digest` | `no_story` (run `dialogue.compile`), `bad_choice` (lists valid indexes), `state_mismatch` (blob belongs to another digest) |
| `level.brush` (later) | `planes` (`[{normal, offset}]`), `material` | `revision`, `faces`, `digest`, `key` | `open_solid` (plane set does not close), `non_convex`, `stale_revision` |

Refusals never partially write: each verb builds one batch, validates it, and
only then asks for the approval, the same order `scene.place` already uses.

## Top 3 steals

### 1. `level.paint` - paint the kit, rules and scatter in one gated call

- Donor: Godot GridMap paint (`@b2220b8d…`) for the gesture, LDtk auto-layer
  rules (`@1b` sample + `AutoLayerRuleDef.hx`) for the computed cell, our own
  `wfc`/`scatter` for the points.
- Integration home: new `crates/endpoint/src/packs/level_paint.rs`, loaded as
  a pack (0 chars of the default tool list). It expands cells into
  `Op::Insert` with ids derived from `(kit, seed, cell)` and reuses
  `validate` + `approve` + `apply`. No new `Op`, no `crates/world` change,
  no render change.
- Bounded reproduction: one call paints a 6x4 wall run (24 cells) from an
  existing kit; the same call twice gives one digest; `tx.undo` with that
  `key` returns the exact pre-paint revision bytes. Test in the same pack
  file, 24 cells against the existing `scene.place` path as the oracle.
- Kill criterion: stop if 24 cells need a new editor `Op` variant, if the
  default tool list grows (the pack must be non-default), or if the same
  `(kit, seed, cells)` ever gives two digests.
- Not the same as the P5 ladder card: that one bakes new geometry (fort from a
  layout JSON). This one paints placed kit cells into the live level document,
  so the two do not overlap.
- Number it moves: verbs 100 -> 101. Authoring: 48 gated calls (24 cells as
  24 preview+apply pairs) -> 1, so a 256-cell floor is 1 call instead of 512.
  Tasks a weak model can finish: +3 eval rows (paint a wall run, scatter a
  forest edge, repaint after undo) on the 75-row harness. Seconds are not
  measured here; the call count is.

### 2. `anim.retarget` - every external clip becomes playable, no new rig code

- Donor: Godot `BoneMap` (`@eabb9a63d4…`): profile bone -> skeleton bone table.
- Integration home: `crates/endpoint/src/packs/anim.rs`, which already states
  the add-a-verb recipe (`SyncTool<McpServer>` + one `pack.add(VerbSpec...)`).
  The mapping type, the 17 roles and the Mixamo aliases already exist
  (`crates/anim/src/contract.rs`: `bone_role_for_name`, `BoneMapping`,
  `bone_coverage`, `REQUIRED_BONE_COUNT`). The verb is a thin wrapper.
- Bounded reproduction: `asset.intake` a fixture glTF authored with Mixamo
  bone names, `anim.retarget` returns `coverage_permille = 1000`, 17/17 roles
  and `method: "alias"`; one renamed bone returns `unknown_bone` naming the
  fix; same input twice, same `digest`. `anim.preview` then plays the clip.
- Kill criterion: stop if any Mixamo name needs a code branch instead of a
  table entry (i.e. the fixture cannot reach 17/17 through
  `bone_role_for_name`), or if the mapping lands in the tick (it must stay
  import-time, so the golden cannot move).
- Number it moves: verbs 100 -> 101. Authoring: a third-party clip goes from
  "unknown role, refused" to 3 calls (intake -> retarget -> preview). Tasks:
  +2 eval rows (retarget a Mixamo fixture; refuse an unknown bone with the
  fix named).

### 3. `nav.mark` - marking volumes over a bake that already exists

- Donor: Godot nav bake (`@2dd1f4ef…`) for the settings shape, Recast (zlib)
  for area cost, and the marking-volume idea (Unity, paid, idea only).
- Integration home: `crates/nav` gains an `agent_radius` and a mark set on
  `BakeConfig` (today it holds only `cell_size` and `walkable_normal_y`,
  `crates/nav/src/mesh.rs:44`), plus `crates/endpoint` verb in a pack. The
  bake stays bake-time, so it never enters the tick or the hash.
- Bounded reproduction: level from `level.build`, mark a 4x4 ramp as
  `link:jump`, and `scene.query reach` reports the far side reachable;
  blocking the same box makes it unreachable. Empty mark set must give the
  same mesh bytes as today (the guard that nothing else changed).
- Kill criterion: stop if an empty mark set changes mesh bytes or the golden
  hash, or if reach answers change without a mark.
- Number it moves: verbs 100 -> 101. Authoring: nav was whatever
  `level.build` baked; now one call can say "this ramp is a jump, this floor
  is slow". Tasks: +1 eval row (mark, then prove the reach answer flips).

## Deliberately not ranked

- Dialogue/quest graph (ink, Yarn): the biggest missing subsystem (the door
  has no dialogue or quest verb at all) but it is a new crate of its own, not
  a wiring job. Shape is in the verb table above. Kill criterion for a later
  row: stop if the story state cannot be integer-only and saved as one blob,
  or if a step needs the tick.
- `level.brush` (Cyclops MIT / Trenchbroom GPL): real CSG and clipping is new
  geometry code with a new failure surface (`open_solid`). Ideas only from
  Trenchbroom; Cyclops would be the port source.
- Undo merge (Godot `UndoRedo`): already solved by the 256-op batch; adding
  a merge knob would be a second spelling of one stroke.
- Fracture: already landed and spiked; nothing to steal.

## What I could not verify

- No donor was executed: no Godot, Blender, ink or LDtk run on this PC. All
  donor facts are static reads of the pinned commits.
- Godot's paint gesture is editor-UI code; that the *data* model paints
  headless is inferred from `set_cell_item` being core, not run.
- LDtk's rule evaluator was not read (only the rule definition, the UI files
  and a serialized sample); the licence (MIT) was read.
- ink's CLI was confirmed from `README.md` only; `inklecate` was not built.
- Pace: the GitHub API rate limit (60/h, shared IP) was hit mid-read, so four
  paths are pinned by 10-char prefixes from the per-file atom feed
  (`fd27ad4a21`, `eabb9a63d4`, `1c45c14d7f`, `7fd74c5f43`); the rest are
  full 40-char SHAs.
- Authoring time in seconds is unmeasured; the numbers above are call counts
  and eval-row targets, not a stopwatch.
