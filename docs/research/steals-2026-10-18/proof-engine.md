STEAL SCOUT: SURFACE, TRAIL, COMBO WINDOWS AND THE ROLLBACK SKELETON (Mattbusel/proof-engine)   scout date 2026-10-18, read-only run
Repo: https://github.com/Mattbusel/proof-engine  sha 17d4b5d2191c91527a8fa7e560fb5b6da877fb53 (2026-10-17T18:11:13-04:00, "0.3.1: GitHub as repository, graphics keyword, CI and MSRV badges")
Files read live 2026-10-18 by shallow clone; no bytes vendored. All paths below are relative to the repo root (src/...).
LICENCE: MIT (LICENSE, "Copyright (c) 2026 Matthew Busel"; API spdx MIT).
What it is: a large single-crate Rust game engine (~600 .rs files) whose stated theme is "every visual is math": attractors, fractals, metaballs, curves, glyphs, particles, plus
  deferred rendering, a scripting VM, an egui editor, audio synthesis and an ecs. It is wide, not deep; almost every module is a reference sketch rather than a finished subsystem.
  The three things worth taking are self-contained algorithms with a clear interface: a closed-mesh isosurface extractor, a curve-to-polyline tessellator, and a combo/hit-confirm
  window model. The rollback module is worth reading as a checklist of what NOT to copy (f64 seconds, HashMap order) as much as what to copy.

== 1. WE HAVE ==
- Render has trails and particles already: crates/render/src/{trail.rs,particles.rs,particle_draw_gpu.rs,vfx_hanabi.rs}, plus wound_map.rs and decals.rs for gore surfaces.
- Combat has windows and hit timing: crates/combat/{hitsim.rs,fight.rs,nerve.rs,feel.rs}; inputs reach it as actions from the tape.
- Net has the full rollback shape: crates/net/src/{netcode.rs,rollback.rs,snapshot.rs,interp.rs,replay.rs} with integer ticks; crates/proof/read.rs owns divergence reading.
- Everything above runs on the fixed integer tick; the donor's versions mostly run on f64 seconds.

== 2. STEALS (ranked by value per hour) ==
1. Marching tetrahedra isosurface: a closed mesh with no case tables and gradient-exact normals | src/metaball/marching_cubes.rs:49-77 (+ extract_with for per-vertex colour/emission) | MIT | PRESENTATION (vfx) | lands crates/render/src/{particles.rs,vfx_hanabi.rs} or a new vfx/blob.rs; test in crates/playtest | M
   Pattern: each cube is split into the six tetrahedra that share the main diagonal 0->7 (:49-61), and each tetrahedron is cut by the isosurface in one of three ways (no crossing,
   one triangle, two triangles). The module doc gives the two payoffs: no 256-entry case table, no ambiguous cases, and "a closed mesh for any field" (:1-10). Normals come from the
   field's gradient, "so they are exact for a smooth field rather than averaged from faces" (:65-68).
   Maps onto: a metaball field is the natural shape for a blood pool, a wet wound blob, a spell effect or the crowd's dust cap, and "closed mesh, no table, exact normals" is exactly
   what a smoke test can hold. Keeping EDGE_TABLE/TRI_TABLE as all-zero constants for caller compatibility (:16-19) is a small trick worth copying if we ever replace an extractor.
   Moves: a field renders as a closed surface; a test walking the extracted mesh finds no boundary edge (every edge has exactly two triangles) for 100 seeded fields.
   Proof: `cargo test -p engine-render --lib marching_tetra` (closed-mesh invariant) x2 + one capture A/B; validation_errors 0.
   Risk: a few more triangles than marching cubes for the same grid; the donor accepts that where triangle count is not the budget. Kill: if the added triangles push a vfx pass over
   its milliseconds bar, or if any extracted mesh shows a boundary edge.
2. Curve -> polyline tessellation by De Casteljau, with a segment budget per curve | src/curves/tessellate.rs:12-58 (+ entity_curves.rs CurveType) | MIT | PRESENTATION (trail) | lands crates/render/src/trail.rs | S
   Pattern: `tessellate_curve` dispatches on a curve kind and samples `n = segment_count.max(4)` points, returning one polyline (:12-30); Bezier goes through De Casteljau, which is
   numerically the calm choice (repeated lerp, no factorial basis) (:36-58). A fully dissolved curve with no point velocity returns an empty polyline, so a dead trail draws nothing
   (:13-15).
   Maps onto: our trail.rs already draws a sword arc; a curve kind plus a per-curve sample count gives a trail whose length, curvature and point count are data rather than fixed
   geometry, and De Casteljau is the right evaluator to port. The "nothing to draw" early-out is a free win on the last frames of a fade.
   Moves: a trail's point count is data; a dissolved trail costs zero draws; the sampled polyline is identical on every machine (fixed sample count, no adaptive epsilon).
   Proof: `cargo test -p engine-render --lib trail_curve` x2 (sample count honoured; dissolved curve empty) + golden frame unchanged with trails off.
   Risk: a fixed sample count makes a long trail look faceted. Kill: if any facet is visible in the arc capture at the shipped sample count, raise the count or drop the curve path.
3. A combo/hit-confirm window model: buffered inputs with a window and per-link min delay | src/combat/combo.rs:51-127, 129-215 (+ ComboDatabase at :291) | MIT | SIM (input windows) | lands crates/combat/src/{exchange.rs,design.rs} and crates/input/src/{action_state.rs,gestures.rs} | M
   Pattern: an `InputBuffer` is a ring of `BufferedInput{input,timestamp,consumed}`, capped by `max_size`, expiring entries older than `buffer_window`, with
   `consume_next_unconsumed` and `peek_sequence` for matching (:51-127). A `ComboLink` carries `time_window` ("maximum seconds after the previous hit's confirm window opens") and
   `min_delay` ("prevents mashing being too easy") (:129-145); a `Combo` is links + hits + an ender, with `matches_sequence` and a priority (:219-290).
   Maps onto: our exchange already resolves combos and parries on the tick, so take the *data shape*, not the code: a consumed flag per buffered input, a window per link, and a
   `min_delay` that stops mashing. Convert every f64 seconds field to u32 ticks - the donor's `current_time: f64` (:64) is exactly the float-tick mistake we must not port.
   Moves: a combo window is an integer tick span; a buffered input is consumed once and never twice; mashing inside `min_delay` does not advance the chain.
   Proof: `cargo test -p engine-combat --lib combo_window` x2 on the integer tick; golden d57b5c8ae3096614 unchanged.
   Risk: an integer window quantises the feel the design wanted. Kill: if a designer cannot hit the window with tick granularity at 60 Hz, widen the window in data rather than
   reintroducing a float clock.
4. The rollback skeleton as a checklist: frame/latest input ring, per-interval desync detector | src/replay/rollback.rs:1-9, 110-165, 184-292 | MIT | SIM (netcode reference) | lands crates/net/src/{netcode.rs,rollback.rs,replay.rs}; reader in crates/proof/read.rs | S
   Pattern: the module doc lists the exact five pieces of a rollback loop (GameState trait, RollbackBuffer ring indexed by frame, InputPredictor repeating the last confirmed input,
   RollbackSession, NetworkStats) (:1-9). `RollbackBuffer` keeps snapshots AND inputs by frame with `oldest_snapshot_frame` for the eviction bound (:200-248); `DesyncDetector`
   records local and remote checksums at a `check_interval` and collects `DesyncEvent`s (:254-292); `InputPredictor` tracks a `prediction_streak` and reports
   `prediction_error(player, actual)` (:113-165).
   Maps onto: we already have all of this, so the value is the *comparison*: what is our eviction bound, and do we report prediction error the way the donor does? Take the two
   named quantities (`prediction_streak`, earliest retained frame) if our netcode lacks them; a long streak with no error is a remote player who stopped sending.
   Moves: a named "how many frames predicted in a row" counter and a named "oldest frame still retained" bound on the session, both displayed in the WAN soak receipt.
   Proof: `cargo test -p engine-net --lib session` x2 with a 200-frame stall: streak rises, no desync, and the retained window is the documented bound.
   Risk: the donor's `HashMap`/`VecDeque` by frame and f64 timestamps are not deterministic; copy the counters, not the containers. Kill: if any part of this reaches the sim state
   rather than the session bookkeeping, or if it changes the golden replay.

== 3. NOT WORTH IT ==
- The rest of the ~70 top-level modules (fractal, quantum, relativistic, electromagnetic, number_theory, symbolic, topology, svogi, restir, nishita_sky, worldgen, economy,
  narrative, ecology): breadth without tests. Same trap as our own BACKLOG; read one file if a row needs it, not the subtree.
- The scripting VM (src/scripting) and the shader graph (src/shader_graph): two language surfaces. Our rules crate already loads data; a second VM is not the duel's problem.
- src/deferred, src/wgpu_backend, src/rendergraph: a second renderer. Our crates/render is the one path; porting anything here is more risk than the algorithm-level steals above.
- The glyph/glyph SDF atlas (src/glyph/*, sdf_generator.rs 33 KB): beautiful, but our HUD text path is already fixed and this is UI polish, not a duel-feel lever.

DONE 4 steals, 5 donor files read (metaball/marching_cubes.rs, curves/tessellate.rs, combat/combo.rs, replay/rollback.rs, LICENSE).
