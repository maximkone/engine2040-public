STEAL SCOUT: raaiden666/kodanu (BLOCK 3, position 23, 2026-10-18). Read-only pass. Donor files read live from raw.githubusercontent.com on 2026-10-18; no clone, no build, no run, no bytes vendored.
Repo: raaiden666/kodanu -- a 3D game engine on wgpu + winit with its own ECS core: forward renderer, instancing, MSAA, retained-mode UI, plugins driving systems over a shared world, rapier3d physics, fixed/variable timestep (README:1-25). 2 stars, pushed 2026-10-18, default branch master. Tree is 969 KB: kodanu_ecs (67 src), kodanu_graphics (71), kodanu_window (27), plus small math/time/transform/physics/input/camera/app crates.
Licence: NO LICENSE file in the tree (read live 2026-10-18: the full file list has no LICENSE, COPYING or NOTICE). The workspace manifest crates/../Cargo.toml:25 declares `license = "MIT OR Apache-2.0"` and Cargo.toml:26-27 points at a different owner (sneakypin/kodanu). So the declared pair is a claim with no text on disk. Our rule 4 allows porting or adapting donor code from any source; recorded honestly. Treat as IDEA-ONLY where a whole file would otherwise be copied.
Plans read: runtime.md (tick, schedule), quality.md, HASH-LAW.md, skills tick-and-runtime + wiring-a-feature.

== 1. WE ALREADY HAVE ==
- A fixed schedule with named phases: crates/ecs/src/schedule.rs:12-24 -- `Phase::ORDER` = [Startup, PreUpdate, Update, PostUpdate], four phases, fixed order, std-only, deterministic. Systems are named boxed closures run in registration order.
- One integer accumulator: crates/core/src/tick.rs (IntAcc is the only accumulator; HASH-LAW fix-step rule). Do not re-steal the accumulator.
- An ECS with world fork: crates/ecs/src/archetype.rs (5 columns x 16 spine slots; change ticks already a Bevy port). Physics bodies: crates/physics/src/rapier.rs (limbs, ragdoll).
- Typed diagnostic events: crates/observe.

== 2. STEALS (ranked by value for the Rome duel per hour) ==
1. The full stage ladder for one frame, fixed stages run N times | raaiden666/kodanu@master:crates/kodanu_ecs/src/scheduler/api/scheduler.rs:5-8,36-58 (read 2026-10-18) | declared MIT OR Apache-2.0 (no LICENSE file) | SIM (order) | lands crates/ecs/src/schedule.rs (`Phase::ORDER`, :19-24) | S
   Idea: the frame is a fixed ladder, not four phases. `run_full(cell, delta)` consumes the accumulator into `fixed_steps`, then runs PreFixedUpdate/FixedUpdate/PostFixedUpdate that many times, then the variable part once: TimeUpdate, PreInputUpdate, PreUpdate, Update, LateUpdate, PreRender, Render, PostRender, PostInputUpdate. Stages live in `[Schedule; Stage::COUNT]`, so the set is data and the count is the enum.
   Maps: our spine has four phases and no input or render stage. The reusable part is the ORDER, stated once as data, with the fixed trio explicitly inside the loop and everything else outside it. That is exactly the boundary HASH-LAW wants: only the three fixed stages may touch the hash.
   Moves: named stages 4 -> a ladder with an input stage before Update and render stages after; the fixed/variable split becomes one readable list. Order change touches golden -> land as a no-behaviour-change re-order first (golden must NOT move).
   Proof: cargo test -p engine-ecs --lib sched_ladder x2 (the ladder is one list; the fixed trio runs `fixed_steps` times and nothing else does; golden d57b5c8ae3096614 unchanged after the re-order).
   Risk: an input stage before Update can change who walks first. Kill: if the golden moves, keep four phases and add only the names as a doc, not as stages.

2. Double-buffered events: read last tick's, write this tick's | raaiden666/kodanu@master:crates/kodanu_ecs/src/ecs/api/event_buffer.rs:5-8,28-45 (read 2026-10-18) | declared MIT OR Apache-2.0 (no LICENSE file) | SIM (order) | lands crates/observe and crates/combat (cues, hit receipts) | S
   Idea: events go into `current`; readers iterate `previous`; `update()` swaps the two and clears the new `current`. So a reader in tick N sees exactly the events sent in tick N-1, and a writer can never be read by its own tick. No ordering rule is needed per consumer, and a missed drain does not lose the events (they survive the swap).
   Maps: this removes a whole class of order questions in a fixed tick: a cue emitted by the combat step is consumed by exactly the next tick's presentation step, never the same one. It also gives diagnostics a stable "one tick behind" contract.
   Moves: cross-step reads go from "same tick, drains in ambiguous order" to "one tick behind, always". Unmeasured.
   Proof: cargo test -p engine-observe --lib events x2 (an event sent in tick N is visible once in tick N+1 and never in tick N; a second `update` makes it invisible; the count is stable across 600 ticks) plus golden unchanged.
   Risk: one tick of latency is wrong for a hit receipt that must land on the press tick. Kill: if any consumer needs same-tick delivery, keep the double buffer for diagnostics only and leave the sim path alone.

3. Per-component added/changed tick sidecars beside the dense array | raaiden666/kodanu@master:crates/kodanu_ecs/src/ecs/internal/sparse_set.rs:5-11,41-70 (read 2026-10-18) | declared MIT OR Apache-2.0 (no LICENSE file) | SIM (query filter) | lands crates/ecs/src/archetype.rs change ticks | S
   Idea: a component store that keeps identity separate from data: `sparse` maps an entity id to a dense index, `indices` maps the dense index back, `dense` holds the values, and `added`/`changed` hold one `Tick` per dense slot so a query can filter by "added since" or "changed since" without a second structure. `insert` defaults the ticks to `Tick::ZERO`, and insert returns the replaced value.
   Maps: our change ticks are a Bevy port already (archetype.rs), so this is a design comparison. The one idea worth weighing is keeping the `added` tick separate from the `changed` tick: today a newly inserted component and a written component can look alike to a filter.
   Moves: a "new this tick" filter becomes possible without scanning. Unmeasured, and likely not needed at 16 spine slots.
   Proof: none proposed at this size; if a filter ever needs it, a unit test that insert then write then read distinguishes added from changed.
   Risk: two extra `Vec<Tick>` per component for a filter nobody uses. Kill: if no query in crates/ecs asks for added-vs-changed within the arena tick, do not port; this is IDEA-ONLY.

4. Finite-guarded physics queries in the resource wrapper | raaiden666/kodanu@master:crates/kodanu_physics/src/resources/physics_world.rs:44-50 (read 2026-10-18) | declared MIT OR Apache-2.0 (no LICENSE file) | SIM (authority guard) | lands crates/physics/src/rapier.rs query helpers | S
   Idea: the raycast entry refuses before it reaches rapier: a non-finite or non-positive `max_distance` returns None, and a non-finite origin or direction returns None. The wrapper also owns handle lifetime: `remove_rigid_body`/`remove_collider` remove from the world and null the handle, so a stale handle cannot be used twice.
   Maps: our rapier wrapper already rebuilds the world from the spawn list (per the physics card: rapier.rs restore rebuilds from the spawn list), so the lifetime half is covered; the guard half is the small win -- one `is_finite` gate stops a NaN sweep from entering parry and returning a garbage fraction.
   Moves: NaN or infinite query inputs are refused at the boundary instead of inside parry. Unmeasured.
   Proof: cargo test -p engine-physics --features rapier --lib query_guard x2 (NaN origin, inf direction, distance 0 and negative each return None; a valid ray is unchanged) plus golden unchanged.
   Risk: a caller that relies on rapier clamping now gets None. Kill: if any existing caller passes distance 0 meaning "unbounded", fix the caller, else drop the guard.

== 3. NOT WORTH STEALING ==
- The whole ECS (sparse set, registries, query macros, plugins, stages, system params): our spine is 5 columns x 16 slots with a fixed order and the workspace denies unsafe; the big ECS shape does not pay for the Rome duel (same verdict as docs/research/steals-2026-10-12/arch.md).
- The accelerator at crates/kodanu_ecs/src/scheduler/internal/fixed_runner.rs:14-35: `max_steps` 4 clamp on an f32 accumulator. We already have one integer accumulator (crates/core/src/tick.rs) and the clamp is a policy we state elsewhere.
- The renderer (kodanu_graphics: 71 files, forward pass, instancing, MSAA, UI nodes, asset server): a from-scratch wgpu forward path that duplicates crates/render without our hash, tiers or proof gates.
- The licence position: with no LICENSE text on disk, porting anything more than a shape or an idea is not worth the record-keeping. Ideas above stand on their own.

DONE 4 steals, 5 donor files read.
