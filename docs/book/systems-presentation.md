# Presentation systems: render, audio, HUD, world look

Presentation interpolates snapshots. It never mutates authoritative
state: wall clock, threads, and GPU work are allowed here and only
here. A render feature counts when its pass submits on a real device
in the window build — CPU reference math alone is a merge gate, not
done (`docs/SPRINT.md` truth rule). Full signatures live in rustdoc:
`cargo doc -p <crate> --no-deps`.

## Render: what runs headless, what needs a GPU

- `engine-render` owns the offscreen V1 forward PBR path with PNG
  capture (headless), plus IBL, shadows, skinning, cascades, probe GI,
  bloom, TAA jitter, grading, particles, and decals as CPU references.
  The window presenter (`present.rs`), frame graph executor
  (`executor.rs`, `graph.rs`), sky, layered materials, GPU particles,
  decals, fog, TAAU/post, and AA/scale passes submit on a real device
  in the window build (GX1–GX10, RL/RP rows).
- Headless captures are real PNGs through the software rasterizer
  (the staged fight writes 4 PNGs plus a manifest). The bare `capture`
  MCP tool answers structured `unsupported_capture` when unwired;
  `play.step` never does — it always returns a real PNG.
- GPU-only gates (reference-GPU captures, per-pass ms, 60 fps bars)
  are marked `(reference GPU)` on the board and are still open where
  the board says so (RL5 bake, RL11 capture bundle, Q1 V1–V3). This
  page claims nothing for them beyond what `docs/SPRINT.md` ticks.

Headless render proof (no display, no GPU) via the staged fight from
the [quick start](quick-start.md):

```powershell
cargo run --locked -p engine-game --features app --bin staged_fight -- --ticks 600 --out-dir target/quickstart-fight
```

Expected: `staged_fight done: ticks=600 ... captures=4` plus a
`manifest.json` and `receipt.json` under the out dir. Same manifest
plus seed plus script gives the same PNGs.

## Audio, HUD, input devices, destruction look

- `engine-audio` owns the device mixer, decode, spatial mix, crowd
  reactions, and hit-event-synced coherence (10 combat scenarios,
  no per-tick allocation). A device loss reopens with backoff while
  the mixer keeps running.
- `engine-ui` owns the HUD model, colorblind palettes, text layout,
  and quads.
- `engine-input` owns bindings, rebinding, and dead zones (keyboard
  and mouse feed `FighterInput` in the window build).
- `engine-destruction` simulates authoritatively (see the
  [simulation page](systems-simulation.md)); its dust, debris, and
  blood-decal look submits through the render graph.
- `engine-world` owns the deterministic bake: seed N plus size M
  gives versioned bytes (`bake`), recipe-keyed maps (`blockout`:
  same seed plus recipe plus size gives the same bytes), value-noise
  heights, cells, terrain, and sky. Pathfinding lives in
  `engine-nav`; world only emits cells. To ship a new arena from a
  recipe, see [New map](recipes-new-map.md).

Check the world bake headlessly (verified this wave):

```powershell
cargo test --locked -p engine-world --lib blockout
```

Expected: `4 passed` (`same_inputs_identical`,
`recipe_and_seed_split`, `effective_seed_deterministic`,
`oversize_blockout_refuses`).

Windowed play needs a display and a GPU adapter and is outside the
headless gate: the `arena` window binary takes `--frames N --exit`
and the `window` example draws procedural sprites. Window binaries
fail loudly on init errors instead of printing a fake frame.

## Reference

- `crates/render/src/present.rs`, `crates/render/src/executor.rs`, `crates/render/src/graph.rs`
- `crates/game/src/bin/staged_fight.rs`, `crates/game/src/capture.rs`
- `crates/audio/`, `crates/ui/`, `crates/input/`, `crates/world/src/blockout.rs`, `crates/world/src/lib.rs`
- `docs/SPRINT.md` (truth rule, GX/RL/RP rows), `docs/VISUAL-TARGET.md`
