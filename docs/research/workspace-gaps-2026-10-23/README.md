# Workspace and project-system gaps vs Flax and peers (2026-10-23)

**Verdict.** Our "project" is a 7-key flat `game.toml` read by a CLI and a served schema; four of
the seven keys are validated and printed but change no build and no run, and `build` always compiles
the `arena` bin. Totally absent vs Flax and peers: plugin/extension system, package store, 3D terrain
and foliage, visual scripting, a material-graph compiler, an animation/cutscene timeline, a UI layout
editor, a network remote inspector, an installer with version metadata, and a real minidump.
What is strong is everything behind the door: undo, prefabs with overrides, inspector, gizmo,
play-in-editor, content-keyed cook cache with a real dependency graph, asset and shader hot reload,
patch delivery by blob hash, crash receipt, profiler, console, rebinding, audio buses, en/de locale, CI.

Method: tree greps on 2026-10-23 (every path below opened or hit), `node tools/workspace-check.mjs`
PASS with `SUMMARY unwired modules: 469 of 850` (`node tools/wiring-scan.mjs`: 7,206 islands), and the
peer facts cited from local research files, never memory. Class: **missing** = nothing in `crates`;
**unwired** = code exists, no caller/door path; **works** = proven by a module and its tests.

| # | Gap | Peer does (local source) | Ours (path or NOTHING) | Class | Smallest real build | Risk |
|---|---|---|---|---|---|---|
| 1 | Project file selects no game code | Fyrox export CLI for PC/WASM/Android (`01-engines.md:28`); balaur `project.toml` `[check] strict` (`01-rust-indie.md:237`) | `game.toml` = 7 keys (`src/manifest.rs:128-142`); `build` hard-codes `arena` (`src/build.rs:176`); `run` records the proof fixture, not project content (`src/project.rs:417`) | unwired (4 of 7 keys decorative: `quality`, `physics_layers`, `gore`, `content`) | add `game = "crate:bin"` to the manifest and pass it to cargo; `run` loads the declared content roots | med: build/run receipts move |
| 2 | Plugin / extension system | O3DE Gems = modular features (`01-engines.md:29`); Fyrox game-code dylib reload (`:28`) | NOTHING: `git grep -i plugin -- crates` = 10 lines, all comments and donor paths (e.g. `crates/endpoint/src/remote_shim.rs:2`) | missing | static module registry: `modules = [...]` resolved against a `linkme` slice; unknown name refused with the list (no ABI, no unsafe) | low-med |
| 3 | Package / asset store, dependency fetch | nearest local idea: O3DE Gems (`01-engines.md:29`); no store cited locally | NOTHING: a "package" is a Cargo crate in this one workspace | missing | `e2040 add <crate>` writes a project lock; `build` refuses an unlocked dependency | med: licences, supply chain |
| 4 | Prefabs exist but the door cannot reach them | Godot/Fyrox/Flax edit prefabs in the scene editor (Godot full editor, `01-engines.md:27`) | `crates/scene/src/prefab.rs` (`OverrideMap:413`), `crates/editor/src/prefab.rs`; no `Op::Prefab` (`commands.rs:137-237`); `git grep -i prefab -- crates/endpoint` = 0 | unwired | `prefab.save` + `prefab.place` in a non-default pack over the existing capture/instantiate | low |
| 5 | 3D terrain and foliage | Flax terrain actor (`03-flax.md:51`); scatter placement (`03-flax.md:53`) | NOTHING: `crates/assets/src/collider_cook.rs:17` "Heightfields are not cooked"; `git grep -i terrain -- crates/render crates/nav crates/assets crates/spatial` = 0; only the 2D `kam2-sim` tile enum | missing | height grid -> cooked mesh + collider, one `terrain.set` verb; scatter reuses `crates/world/src/scatter.rs` | high: new render + nav path |
| 6 | Visual scripting | Flax "C# and visual scripting (read)" (`01-engines.md:31`); Godot full editor (`:27`) | NOTHING visual: `crates/rules` runs Rhai text (`rules.set`); the node graph has no script node (`crates/editor/src/graph.rs:26-31`) | missing | `NodeKind::Rule` in the graph, compiled to the existing Rhai source so the run path is unchanged | med |
| 7 | Material node graph never compiles | Godot shader editor (`01-engines.md:27`) | graph is stored in the document (`crates/editor/src/commands.rs:75`) but `material/src/compile.rs` starts from a `MaterialRecord`, and a MaterialX nodegraph is refused by name (`crates/material/src/mtlx_import.rs:617`) | unwired | `material.from_graph` lowers Const/Op nodes into record params; no new WGSL path | med |
| 8 | Animation / cutscene timeline | Godot animation editor (`01-engines.md:27`) | NOTHING: no `cutscene`/`dopesheet`; the only `sequencer` is music steps (`crates/audio/src/music.rs:444`) | missing | a keyframe document + `anim.timeline.set` played through the existing `anim.preview` | med |
| 9 | UI layout editor / widget tree | Godot Control, Castle anchors, Babylon GUI (`01-engines.md:36-37`) | screens are Rust data (`crates/ui/src/menus.rs`, `settings.rs`, `lobby_ui.rs`); no widget tree and no layout file | missing | `.ui.toml` rows (anchor, offsets, style ref) loaded into the existing HUD snapshot | med-high: render side |
| 10 | Remote inspector / debugger over a network | bevy_remote HTTP transport (our own PORT line, `crates/endpoint/src/remote_shim.rs:2`) | loopback in-process only: "in-process, no socket" (`remote_shim.rs:1-2`) | missing | serve the same allowlisted `BrpRequest` table on a loopback TCP port | med: auth, sandbox |
| 11 | Installer, version metadata, one-click export | Fyrox export CLI (`01-engines.md:28`); Godot many export targets (`:27`); bar "one-command staged validated installers" (`VISION-TABLES.md:95`) | `build` builds the arena bin; `tools/ship.mjs:1073` replicates `package.rs assemble_full` over `dist`; no installer, no version or icon in `game.toml` | unwired | `build` writes `dist/<name>/` + `build.json` (version, digest) and one zip/MSI step | high: GM3, signing |
| 12 | Profiler has no trace file; GPU capture is external | Godot Tracy/Perfetto hooks (`01-engines.md:27`); PlayCanvas `gpu-profiler.js` (`:38`) | `crates/perf/src/profiler.rs` (puffin scopes); a capture needs `tools/rdc-diff.mjs` outside the engine | unwired | `perf.trace` writes Chrome-trace JSON of the eight tick phases (ENG7) | low |
| 13 | Crash dump is a 12-byte stub | our own PORT names the donor `minidumper` (`crates/game/src/crash.rs:733`) | `write_minidump_stub` writes magic + tick + len (`crash.rs:743-753`); the TOML report itself works | unwired | take `minidumper` behind a feature and attach it to the existing hook | low-med |
| 14 | Localization is two static tables | Godot CSV translations with context and plurals (`01-engines.md:27`) | `crates/ui/src/locale.rs`, static en/de | works (thin) | CSV -> table builder; a test fails on a key with no `en` (ENG10) | low |
| 15 | Per-asset import settings are code, not data | O3DE asset processor (`01-engines.md:29`) | `ImportSettings` is a Rust argument (`crates/assets/src/import_repair.rs:145`); the cook key does include it (`crates/assets/src/cooker.rs:4-7`) | works (thin) | `.import.toml` sidecar per asset read into the cook key | low |
| 16 | Asset browser shows no previews | not cited locally | `crates/editor/src/browser.rs` lists rows; `git grep -i thumbnail -- crates` = 0 | missing | cook one 128 px PNG per asset at import and show it in the browser row | low |
| 17 | Project lifecycle has no door verbs | quench3d ships 9 MCP tools incl. `doctor` (`01-engines.md:39`) | no `project.*` among the 104 `TOOL_*` constants; only `game.build` (`crates/endpoint/src/build_game.rs:24`) | unwired | `project.new`, `project.build`, `project.run` over `src/project.rs` | low |

## What we already match (do not rebuild)

- Undo/redo plus a durable journal, one batch is one undo: `crates/editor/src/commands.rs`, `tx.undo` in `crates/endpoint/src/mcp.rs`.
- Prefabs with per-instance overrides that survive edits underneath: `crates/scene/src/prefab.rs`, `crates/editor/src/prefab.rs`.
- Schema inspector, automatic inspector rows, outliner, dock panels, layout-as-data, theme: `crates/editor/src/{inspector,inspector_auto,outline,panels,theme}.rs`.
- Gizmo and viewport picking: `crates/editor/src/gizmo.rs`, `crates/editor/src/viewport.rs`.
- Play-in-editor with pause/step/restart over snapshots: `crates/editor/src/play_in.rs`, `play.rs`.
- Scene save/load with stable ids and a codec: `crates/scene/src/codec.rs`, `crates/endpoint/src/packs/scene/save.rs`.
- Asset cook, content-keyed cache (DDC), dependency graph with transitive dependents, reimport, debounced recook, reload notices: `crates/assets/src/{cooker,registry,recook,watcher}.rs`.
- Data hot reload at frame boundaries (clips, tuning) and shader hot reload: `crates/game/src/reload.rs`, `crates/render/src/shader_gate.rs`.
- Patch delivery by blob hash with base verification: `crates/assets/src/asset_source/patch.rs`, `crates/assets/src/bin/e2patch.rs`, `crates/game/src/arena_kit.rs:1188`.
- Crash receipt, next start prints it once: `crates/game/src/crash.rs`; profiler: `crates/perf/src/profiler.rs`; console and log verbs: `crates/game/src/console.rs`, `crates/endpoint/src/log_tools.rs`.
- Input rebinding with deadzone and a 1.7k-line settings screen: `crates/input/src/bindings.rs`, `crates/ui/src/settings.rs`.
- Audio mixer with buses, spatialization, occlusion, HRTF, reverb and crowd-bed ducking: `crates/audio/src/{mix,spatial,occlude,hrtf,reverb,crowd}.rs`.
- CI that runs the shipped binary, the golden matrix, the web build and the wasm checks: `.github/workflows/{ship-check,golden-matrix,web-build,wasm-check}.yml`.
- Project CLI and two embedded templates: `src/project.rs`, `src/build.rs`, `docs/templates/{arena-duel,siege}`, `crates/editor/src/templates.rs`.
- A human editor binary exists (winit + egui) but is not the authoring path by owner decision: `crates/editor/src/bin/e2040-editor.rs`, `crates/editor/src/app.rs`, `docs/plan/finish-line.md:74`.

## Three that matter most this month

1. **Make `game.toml` a contract (row 1).** Today two projects differ only by seed and name: `quality`,
   `gore`, `physics_layers` and `content` are parsed, echoed (`src/manifest.rs:488`) and dropped, and
   `build` compiles `arena` whatever the project says (`src/build.rs:176`). Every other gap on this list
   lands in a project setting, so without this the rest cannot be configured. It is also the cheapest:
   one key, one cargo argument, one content-root read.
2. **Give the door the prefabs it already has (row 4).** The substrate is finished and tested
   (`crates/scene/src/prefab.rs`), and zero door verbs name it. Every rival editor's core gesture is
   "save this subtree, place it again"; an AI that cannot repeat a prop cluster cannot build a level.
   Two verbs in a non-default pack close it with no new data model.
3. **An export that ships a project, not the arena (row 11).** The ship run proves the engine, not a
   user's game: `tools/ship.mjs` stages a fixed five-file set and no installer, version or icon exists.
   With row 1 in place this becomes "build the declared bin, write `dist/<name>/` plus `build.json`,
   then one archive step", and the Finish A "in the player's hands" bar stops depending on a manual run.
