# Spike card: the programmable world model and the harness split (2026-10-22)

Two board rows, one card each. Source notes: `01-arxiv.md` in this folder. The other 2026-10-22
dive (`docs/research/finish-line-2026-10-22/03-arxiv-engine.md`) did not cover these papers, so
nothing here repeats it.

## PROG1: a typed entity view off the sim (spike)

- Paper: arXiv 2609.10540, Programmable World Model (AlayaLab/pwm), submitted 2026-09-09.
- Hypothesis: the sim can hand a renderer everything it needs to draw the fight, without the
  renderer reading sim internals and without the sim knowing a renderer exists.
- Baseline: today the only state that leaves the sim is a position and a yaw
  (`crates/ecs/src/bridge.rs:306` `encode_arena` writes slot, three `f32` metres and one `f32`
  yaw; `crates/game/src/lib.rs:1959` `FighterView` carries the same three numbers). A renderer
  can draw a fighter's location and nothing else: not its kind, not its bounds, not its weapon.
- Candidate: one typed view of the sim state, naming each entity by kind, with bounds and a
  per-tick change list.
- Fixture and seed: the seed-2040 600-tick arena tape, 8 fighters, the golden
  `d57b5c8ae3096614`. The same tape already runs headless: `arena --view-line 2040 300 8`.
- Metric: three numbers, all printed by the test.
  1. view bytes identical twice for the same tape (determinism, the same bar the bridge wire
     already has);
  2. the count of entities the view reports against the count the sim holds (a wrong count is
     the failure, like CombatStateBench's 94% count accuracy);
  3. the state accuracy of the view: for a seeded frame, does the view's answer match the sim's
     own answer about every entity (CombatStateBench reports 98% state accuracy on its bench).
- Resource budget: CPU only, no GPU, no model, no new dependency. The view is integers and
  `f32` presentation numbers, in the existing crates.
- Stop rule: one session. If the typed view cannot be built without touching the sim's tick
  order or the golden, the spike ends and the row closes as REJECTED (a view that changes the
  sim is a different engine).
- Discriminating ablation: the view with the change list removed. The count and determinism
  numbers must not move; only the change-list number does. If the count moves, the view is
  reading something mutable.
- Real engine consumer: `crates/web` (the page draws the fight from the view) and
  `crates/endpoint` (the door answers "what is in the arena" from the same view).
- Owned files: `crates/ecs/src/bridge.rs` (the view beside `encode_arena`),
  `crates/scene/src/schema.rs` (the typed view), `crates/proof` (the tape).
- Kill criterion: the view's count disagrees with the sim's count on the seed-2040 tape, or the
  golden `d57b5c8ae3096614` moves, or the view needs a new crate dependency. Any of the three
  ends the row.
- F2P: no typed entity view; the only state that leaves the sim is a position and a yaw.
- P2P: golden `d57b5c8ae3096614` x2; the bridge wire layout is unchanged.

## PROG2: the engine draws its own proxies (spike)

- Paper: arXiv 2610.27999, Matrix-Game 4.0, submitted 2026-10-20.
- Hypothesis: the web page can draw the engine's own geometry instead of stand-ins, and the
  native and web frames can then be compared as one picture.
- Baseline: the page draws a procedural ring and stand-in bodies; the sim state reaches the
  draw thread over `crates/web/src/frame_bus.rs`, but nothing carries geometry, so the web
  picture is not the engine's picture. The paper's own split: a lightweight engine keeps the
  world state and executes the interaction rules, and a separate renderer turns engine-rendered
  low-poly proxies into detailed visuals.
- Candidate: the engine emits its own low-poly proxy geometry for the arena, and the page draws
  that.
- Fixture and seed: the arena scene, 8 fighters, the seed-2040 tape; the native capture and
  the headless-Chrome capture of the same tick.
- Metric:
  1. the fighter count in the native frame and in the web frame, equal;
  2. the SSIM number from `crates/web/src/arena_view.rs:303` `ssim` on the two captures;
  3. the ms per frame on the RTX 3050, against the paper's 480p 60 FPS on one RTX 5090 (their
     number is on a much larger card, so this is a budget read, not a target to match).
- Resource budget: the RTX 3050 (6 GB), the wasm build, headless Chrome. No model weights, no
  diffusion model, no new service: the card is a renderer split, not a generative model.
- Stop rule: one session for the capture pair and the numbers. The paper's distilled causal
  video generator is a separate, later step and is not part of this spike.
- Discriminating ablation: the page with the proxies replaced by the stand-ins it draws today.
  The fighter count and the SSIM number must move; the golden hash must not.
- Real engine consumer: the browser page (`crates/web/src/arena.rs`) and the frame gate in
  `crates/web/src/arena_view.rs:387`.
- Owned files: `crates/game/src/fighter_look_acts.rs` (the proxy draw),
  `crates/web/src/arena.rs` (the page reads the proxies).
- Kill criterion: the native and web captures cannot show the same fighter count on one tick, or
  the golden `d57b5c8ae3096614` moves, or the page's bundle budget (first bundle under 2,040,000
  brotli bytes) cannot carry the proxy bytes. Any of the three ends the row.
- F2P: the page draws stand-ins, not the engine's own geometry.
- P2P: golden `d57b5c8ae3096614` x2; the native frame bytes unchanged.

## What is NOT in this card

- The diffusion video renderer of Matrix-Game 4.0, and SolarRenderer's 4-step structure-
  conditioned path. Both need weights and a GPU budget the potato tier does not have. They are
  the offline golden path, a later card, with a size audit first.
- A change to the sim. Neither row touches the tick, the rules or the golden.
- A new crate. The view lives in `crates/scene` and `crates/ecs`, which already exist.
