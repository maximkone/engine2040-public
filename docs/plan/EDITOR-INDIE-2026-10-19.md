# Editor for indie developers, 2026-10-19

Owner ask: the editor must be easy for an indie, almost on auto, visually nice
and advanced. Sources are the read-only audits `work/nb-gaps/nb-editor-audit.txt`
(engine2040 code) and `work/nb-gaps/nb-online-editor.txt` (Godot, Fyrox, Flax,
Stride, O3DE, Defold, bevy_editor_pls, KaM Remake's in-game editor). Book text
stays in `work/`.

## Landed (2c2accdf)

- The `ui` build was red: `TabViewer::ui` matched 7 of 8 `Panel` variants.
  The `Panel::Activity` arm now paints a real ring panel fed by
  `Shell::ActivityRing`; template applies and staged batch applies record
  human rows. The per-frame `ensure_builtin_panels` call is gone, so a closed
  tab stays closed.
- `theme.rs`: dark warm charcoal, bronze accent, 6 px corners, 13.5 pt body,
  pinned dark in both theme slots and applied once in `EditorApp::new`.
- Proof: `cargo test --locked -p engine-editor --lib --features ui` ->
  344 passed, 0 failed, 2 ignored; new tests for a closed tab, the Activity
  panel and the theme. `node tools/docs-coverage.mjs` -> PASS 0 regressions.

## In this pass

- Empty viewport band: the four tutorial-1 hints plus one button that applies
  Arena + Fighter + Weapon (a play-ready scene). Play now saves the live scene
  before it spawns the child and records the first-fight step.
- Tests: `one_click_starters_reach_a_play_ready_scene`,
  `empty_viewport_paints_the_start_band`.

## Rows (most valuable first)

| id | row | now | accept | engine source |
|---|---|---|---|---|
| EI1 (landed d0ff9f0c) | Camera navigation: orbit, pan, zoom, focus selection | eye fixed at `[0,1.5,5]`, fov 60 (`viewport.rs:110-117`); only click-pick and gizmo | orbit changes yaw/pitch, wheel changes distance, F frames the selection bounds; two captures with different eyes | Godot orbit/F; Fyrox editor camera; Unreal RMB+WASD |
| EI2 | The viewport shows the scene, not a marker grid | CPU raster of 3-5 px markers (`viewport_image.rs:184-230`); GPU path is offscreen PNG only (`app.rs:16-20`) | a 2-node capture differs from the grid baseline; the texture is the GPU frame at panel size | Flax play-in-editor; Fyrox scene preview |
| EI3 | Drag a `.glb` into the editor | `browser.rs` reads bundles, never owns them; `tools/gltf-intake.mjs` is the only intake | drag-drop a file -> N nodes placed at the cursor; a bad file keeps the scene and shows the error | Godot import dock (`.import` + reimport on change) |
| EI4 (landed aff52663) | Gizmo scale, snapping and W/E/R/T keys | `GizmoTool` had Translate and Rotate only (`gizmo.rs:37-46`); zero `Key::` shortcuts | a scale drag changes scale by snapped 0.25 m / 15 deg steps; W/E/R/T switches tool | Fyrox 1/2/3/4 + grid snap; bevy_editor_pls T/R/S |
| EI5 (landed 09ba69e0) | Prefabs: save a selection, instantiate it | 4 hardcoded `TemplateKind`s; 0 of 10 `Op` variants are prefab-related | save -> instantiate -> edit round-trip changes one instance only | Fyrox prefabs; Godot scenes |
| EI6 | Mission/script authoring home | PLF8 parked; KaM ships 209 `.script` missions over 248 maps | one goal HUD row authored in-editor and replayed | KaM mission scripts + goal HUD |

EI1, EI3, EI4 and EI5 have landed; EI2 is multi-day and EI6 waits on PLF8. Each row names a
file and a test, so a claim can be checked.

## What an indie needs, ranked

1. One click to a playable scene (landed in this pass).
2. A camera that can frame anything (EI1).
3. Drag in art (EI3).
4. See the game, not a grid (EI2).
5. Play what is on screen (landed in this pass).
6. Keys, snapping, scale (EI4).
7. Reuse a prefab (EI5).
8. Author a mission (EI6).

## Refused

- A web-based editor replacement: the shell is egui, tests already run headless,
  and a rewrite would delete the command-service guarantees.
- A marketplace/store: no consumer and no proof.
- Copying KaM binaries: its data and UX only, never the executables.
- A generic node graph for everything: the two graphs that exist (findings,
  commands) already cover the need; one editor-wide graph has no consumer.

## Board actions

EI1-EI6 are in `docs/SPRINT-24-editor.md`; EI1 (d0ff9f0c), EI4 (aff52663) and
EI5 (09ba69e0) landed. EI2 and EI3 are next. The audit's `PLAN SAYS X` list is
corrected there (ED20 was ticked without a working panel; ED17's bench could not
compile; PLF8/DX7 name files that do not exist).
