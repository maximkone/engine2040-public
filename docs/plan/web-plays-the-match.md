# The page plays the match it draws

Plan for row WEB15. The browser page runs the same match the native window runs and
draws it, instead of the looped stand-in scene it draws today.

## What exists (measured 2026-10-19)

- `crates/web/src/arena.rs` draws `engine_render::procedural_arena::ProceduralArena`: a
  lit ring with eight cosmetically animated bodies. Nothing in it comes from a match, so
  a key press cannot move a fighter. `?server=…&play` joins a native duel
  (`crates/web/src/netplay.rs`) but that leg is headless: the file has no render call, so
  WASD drives a body nobody sees.
- `engine-game` builds for `wasm32-unknown-unknown` with `features = ["app"]`
  (`cargo check --target wasm32-unknown-unknown -p engine-game --features app`, exit 0).
  `engine-web` compiles with it as a wasm-only dependency (exit 0, 42 s).
- The native window's live path is small and already in the lib:
  `live_fight_dots` (in `crates/game/src/bin/arena.rs`, reads
  `App::window_frame_poses`, `App::hp`, the tick's `FightCue`s), `stage_fight_bodies`
  (uploads one procedural body mesh per dot), `push_fight_draws` (one draw per resident
  body), `hud_draw_list`, `dressing_camera` (`crates/game/src/windowed.rs`,
  `staged_fight_frames.rs`).
- `App::new(config)` / `App::with_world(config, kit_match_world()?)` /
  `App::into_duel()` build a session with no file IO; `App::tick(FighterInput)` steps one
  fixed tick and fills every non-human slot with a brain. `engine_game::camera` is plain
  math (no winit).

## Steps

- [x] 1. Lift `live_fight_dots` from `bin/arena.rs` into `engine-game` (staged fight
      frames), re-export it, and call it from the bin. No behaviour change.
      Landed: `windowed::staged_fight_frames::tests`, 10 passed
      (`cargo test -p engine-game --features app --lib staged_fight_frames::tests`).
      Found on the way: the helper read the pose cursor (a local root-motion
      pose, zero at tick 0) instead of the match's fighter views, so every
      body drew at the origin; it now reads `App::fighter_views()`.
- [x] 2. `crates/web/src/fight.rs`: hold an `App` over the kit world with duel rules, step
      it from the page's frame loop at the fixed 60 Hz rate, and print the tick and hash
      on the page (`#fight`) so a run is readable without a GPU.
      Landed: the page prints `fight: tick=… hp=…/… at=(x,z) (x,z)` and the tick
      advances (`tick=1020` after two minutes at roughly 10 steps a second in the
      built-in browser pane, which draws about 1 fps).
- [x] 3. Draw the fight: keep the procedural ring for scenery, stage the body meshes from
      the dots, push the body draws over `hud_draw_list` + a camera, present.
      Landed: `ProceduralArena::list_scenery` (+ test `scenery_is_every_static_part…`),
      `stage_fight_bodies` + `push_fight_draws` over it, browser capture shows the two
      match bodies over the ring.
- [x] 4. Input: the page's held keys become the human slot's `FighterInput` each tick
      (`crates/web/src/browser.rs` already owns the key table and `LivePilot`'s mapping).
      Landed: `browser::seat_input(tick, facing)` + `browser::seat_look()` (mouse look and
      Q/E through `Facing`), proven by `a_walking_seat_moves_its_dot` (a held forward wish
      walks the seat's feet ~1 m over 90 ticks).
- [x] 5. Camera: follow the human slot with `engine_game::camera` (the native wish/boom
      math), falling back to the establishing camera when the slot is floored.
      Landed as `ProceduralArena::framing_camera`: the pair's own feet drive the frame
      (perpendicular bisector, validated, falls back to the ring camera on a degenerate or
      non-finite pair), with a test that projects both feet into NDC.
- [ ] 6. Prove it: `cargo check --target wasm32-unknown-unknown -p engine-web`,
      `node tools/web-build.mjs`, then a real browser capture showing two bodies at two
      different places, the tick advancing, and a key press moving the slot.

## Out of scope (named, not hidden)

The full cooked-kit look in the browser needs the asset rows: fetch the cooked kit
(WEB6), KTX2 textures (QW5), compressed meshes with LODs (QW6), HDRI out of the wasm
(QW4). Until those land, the page draws the procedural ring and the procedural bodies
with the match driving them. That is a playable picture, not the final art.

## Shape

`engine-web` gains one module and one query flag (`?fight`). The sim is the authority:
the page steps `App`, never re-implements a rule. Nothing in this plan enters the golden
hash; the browser leg is presentation plus the same `Sim::step` the native leg uses.
