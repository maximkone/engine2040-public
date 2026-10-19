# SPRINT-24: editor for indie developers 2026-10-19

Rows from the 2026-10-19 editor audits. Spec:
`docs/plan/EDITOR-INDIE-2026-10-19.md`. The `ui` build is green again
(2c2accdf) and the shell has a theme, an Activity panel and an empty-scene
start band. EI1 landed d0ff9f0c (camera orbit/pan/zoom/frame), EI4 landed
aff52663 (scale, snapping, W/E/R/T) and EI5 landed 09ba69e0 (prefabs); EI2 and
EI3 are next.

- [x] EI1 editor: the viewport camera orbits, pans, zooms and frames — Landed d0ff9f0c.
  `viewport.rs:110-117` fixes the eye at `[0,1.5,5]` and fov 60; the viewport
  panel only click-picks and drags a gizmo. Add orbit (right-drag or
  Alt+left-drag), pan (middle-drag), wheel zoom and `F` frame-selection to the
  viewport camera, with clamped pitch and distance. Diff: regular. Tier B.
  Size S. Owns `crates/editor/src/viewport.rs` and the viewport-panel hunks of
  `shell_ui.rs`. Accept: a test drives two orbit steps and asserts yaw/pitch
  move; wheel changes distance and clamps; `F` frames a selection's bounds;
  two captures with different eyes differ. F2P: the eye cannot move. P2P:
  click-pick and gizmo drag unchanged. needs: none.
- [ ] EI2 editor: the viewport shows the scene, not a marker grid —
  `viewport_image.rs:184-230` paints a grid and 3-5 px markers; the GPU
  `Presenter` path writes offscreen PNG receipts only (`app.rs:16-20`).
  Upload the presenter frame at panel size as the egui texture, keeping the
  CPU grid as the no-GPU fallback. Diff: regular. Tier B. Size M. Owns
  `crates/editor/src/app.rs` (texture refresh) and `viewport_image.rs`
  (fallback split). Accept: a 2-node scene capture differs from the grid
  baseline; the texture matches the panel size; the same revision gives the
  same hash twice. F2P: the viewport never shows geometry. P2P: headless
  captures unchanged. needs: none.
- [ ] EI3 editor: a dragged `.glb` becomes nodes — `browser.rs` reads bundle
  listings and never owns intake; `tools/gltf-intake.mjs` is the only path.
  Add a drop target on the viewport that runs the intake, reports progress,
  places the result at the cursor and refuses a bad file without touching the
  scene. Diff: regular. Tier B. Size M. Owns `crates/editor/src/browser.rs`,
  a new `import.rs` (or a `shell.rs` intake seam) and one fixture `.glb`.
  Accept: a dropped fixture yields N nodes and one undo step; a corrupt file
  shows an error and leaves the document byte-equal. F2P: no drop path.
  P2P: the browser search numbers unchanged. needs: none.
- [x] EI4 editor: gizmo scale, snapping and W/E/R/T — Landed aff52663.
  `gizmo.rs:37-46` had Translate and Rotate only; the crate had zero `Key::` shortcuts. Add a
  Scale tool, 0.25 m / 15 deg snapping (Shift toggles) and W/E/R/T tool keys;
  keep one undo step per drag. Diff: regular. Tier B. Size S. Owns
  `crates/editor/src/gizmo.rs`, `shell_ui.rs` input hunks. Accept: a scale
  drag applies snapped steps; each key switches the tool; a drag still folds
  into one journal entry. F2P: no scale, no keys. P2P: translate/rotate
  results unchanged. needs: none.
- [x] EI5 editor: prefabs save a selection and instantiate it — Landed 09ba69e0.
  4 hardcoded `TemplateKind`s; 0 of 10 `Op` variants are prefab-related. Add
  `prefab.save`
  and `prefab.instantiate` over the command service, with names and an undo
  round-trip. Diff: regular. Tier B. Size M. Owns `crates/editor/src/templates.rs`,
  `commands.rs` and one test. Accept: save a 3-node selection, instantiate it
  twice, edit one instance; the other and the source stay byte-equal. F2P:
  no reuse. P2P: template reservations unchanged. needs: none.
- [ ] EI6 editor: mission authoring has a home — PLF8 is parked; KaM Remake
  ships 209 `.script` missions over 248 maps with per-map text sidecars and a
  goal HUD. Add a mission tab that edits goals and a text sidecar, previews
  the HUD and replays one mission. Diff: regular. Tier C. Size L. Owns a new
  `mission.rs` and the dock panel entry. Accept: one authored goal appears in
  the HUD capture and replays to the same end tick twice. F2P: no mission
  path. P2P: the arena play flow unchanged. needs: PLF8.

## Plan says X (corrected)

- ED20 was ticked while `Panel::Activity` had no `TabViewer` arm and the `ui`
  build did not compile. The arm and the ring panel now exist (2c2accdf).
- ED17's "every panel <= 1 ms" bench iterated all 8 panels and could not
  compile before 2c2accdf; rerun it before quoting the table.
- ED15/ED16 "play parity" is the scripted headless session, not the window's
  Play button (`a3_session.rs:921` vs `app.rs:818`). EI2/EI4 close the gap.
- PLF8 (`sequencer.rs`), DX7 (`fracture_tool.rs`) and AD7 (`spatial_view.rs`)
  name files that do not exist yet; they are plans, not landed code.
