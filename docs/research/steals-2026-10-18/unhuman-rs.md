STEAL CARD: UNHUMAN-RS (wave 2026-10-18, block 7, position 47). Read-only pass; donor files read live
2026-10-18, paths taken from the pinned tree. No bytes vendored.
Repo: https://github.com/unhuman-engine/unhuman-rs @ 4dcdbd3dccf2 (main, updated 2026-10-17T06:32Z, 2 stars).
Licence: Apache-2.0 (LICENSE:1-3 "Apache License / Version 2.0", + GitHub licenseInfo apache-2.0, read 2026-10-18).
What it is: a Rust engine workspace ("Unhuman"/UHE): an RHI with a render-graph compiler over wgpu and Vulkan backends,
a renderer (passes: geometry, shadow, ssao, composite, readback, lines, editor; animation with clip/sampler/compression/
state machine; IK; text), physics behind a backend trait with a Rapier 3D implementation, a jobs crate, a shader crate
(Slang to SPIR-V with a content-addressed cache), scene serialization, spatial audio, and an egui editor. docs/PARITY.md
tracks parity with the original C++ engine at one pinned commit.

=== 1. WE HAVE ===
- A pipeline cache with a versioned on-disk format and an FNV-1a key: crates/render/src/cache.rs:5-18 (`CACHE_VERSION`,
  refuses an unknown version), :96-125 (`PipelineKey { shader_hash, defines, layout }`, hash of the shader source bytes).
- A render graph: crates/render/src/graph.rs:36 (deterministic topological order, Kahn, smallest id first), :91
  (non-overlapping lifetimes, explicit barriers), :298 `barrier_rank`; passes wired by `compile_*_for_forward` helpers.
- Rapier behind a spawn list, with snapshot/restore and a state hash: crates/physics/src/rapier.rs:117-147, :283, :303-324,
  :388-412 (rebuild from the spawn list, then put the dynamic states back).
- Animation clips and a cook path: crates/anim (pack.rs) and crates/assets/src/retarget.rs:1178-1184.

=== 2. STEALS (ranked) ===
1. Put the toolchain version in the shader cache key (we do not) | UHE/src/shaders/src/cache.rs:1-7, :16-43, :45-50, :53-80,
   :120-146 (read 2026-10-18) | Apache-2.0 | RENDER/ASSETS (build artifact, not the tick) | lands crates/render/src/cache.rs | S
   Idea: the key is keyed on source bytes PLUS every compiler flag PLUS the compiler version, and the header says why the
   third is the one people forget: the compiler can legitimately change its output between versions, so a key without it
   serves stale artifacts after a toolchain bump - a bug that looks like a driver bug (":1-7"). The hash is written out
   (FNV-1a with its constants) because `DefaultHasher` is explicitly not stable across Rust versions (":16-43"), strings are
   length-prefixed so `ab`+`c` cannot collide with `a`+`bc` (":45-50"), and the key carries a domain tag so a future layout
   change cannot read an old file (":53-80").
   Maps: our key is `(shader_hash, defines, layout)` over the shader source bytes (crates/render/src/cache.rs:96-125) with a
   format `CACHE_VERSION` but no toolchain identity. A compiler/flags change on the same source reuses an old artifact.
   Moves: add the compiler version (and the flag set we actually pass) to `PipelineKey`, length-prefixed into the same
   FNV-1a, and keep the format version for the file layout.
   Proof: build twice on one toolchain -> cache hit, identical bytes; bump the version field -> 0 hits and a fresh build,
   and the golden hash is untouched (this is a build artifact, never sim state).
   Kill: kill it if the key can be satisfied by a default/empty version string (then every artifact looks valid again), or if
   the cache lives inside the hashed path. A cache must be droppable without moving the hash.

2. A render-graph compiler that culls, tracks lifetimes and reports every error at once | UHE/src/rhi/src/graph/compiler.rs:11-25,
   :27-41, :54-70, :174-201, :197-201, :255-289, :292-302, :339-535, :520-535 (read 2026-10-18) | Apache-2.0 | RENDER |
   lands crates/render/src/graph.rs (deepen) | M
   Idea: passes declare resources; `compile` runs topological_order -> cull -> synthesize_barriers and returns a
   `CompiledGraph` of pure data (":174-201") - the backend looks the recording closure up by pass INDEX (":11-25"), so the
   compiled graph is testable without a device. Two details are worth taking: (a) the sort VERIFIES that declaration order
   is already topological and keeps declaration order, so a single-queue frame compiles to exactly what was written
   (":197-201"); (b) `cull` drops a pass whose output nothing later reads (":255-289"), so a debug/post pass costs nothing
   when it is off. Barriers are pre-only and as narrow as the access actually taken (":292-302"),
   a post-barrier is emitted only from the last use (":515"), and validation COLLECTS every error before failing (":53-60").
   `ResourceLifetime { first_use, last_use }` is what transient reuse will need (":27-41", ":520-535").
   Maps: crates/render/src/graph.rs already has Kahn order and barriers; it has no cull, no lifetime table, and the
   `compile_*_for_forward` helpers each build their own pass list, so a pass with no consumer still runs and a validation
   failure reports one thing at a time.
   Moves: add cull + a lifetime table to graph.rs, assert "declared order == compiled order" for the single-queue case, and
   return all validation errors in one call. Keep it pure data so it is unit-testable without a GPU.
   Proof: crates/render lib test: a pass nothing reads disappears from the compiled graph; a deliberately out-of-order
   declaration fails with a named error; two errors in one graph are both reported; `cargo test -p engine-render --lib graph`
   passes and the captures are unchanged (adding the graph step must not move pixels).
   Kill: kill the cull if the compiled order can differ from declaration order in the single-queue frame - then the graph can
   only disagree with the golden and with the capture - or if validation stops at the first error. One run must name all of them.

3. Physics sync with source/written markers: an unchanged body costs nothing, an outside edit survives | UHE/src/physics/src/physics_system3d.rs:1-10,
   :86-136, :137-158, :159-213, :226-233, :311-345 (read 2026-10-18) | Apache-2.0 | SIM/PHYSICS (the rebuild path; not the
   hashed authority it guards) | lands crates/physics/src/rapier.rs | M
   Idea: sync runs both ways, once per step (":1-10"). Every entry keeps `source` (everything the body was BUILT from) and
   `written` (the pose this backend last put in the scene). Equal source -> leave the body alone; `written != pose` after a
   step means someone edited the scene, so the pose is PUSHED into Rapier instead of being overwritten next frame; a
   different source rebuilds the body (":86-136"). Statics are skipped on the way out (":137-158"). `Source` is exactly the
   description that decides the body (":311-345"), and a refused shape is tracked with the same markers so it does not
   re-refuse every frame (":214-225").
   Maps: crates/physics/src/rapier.rs rebuilds the world from the spawn list and restores the dynamic states
   (:388-412) - a full rebuild - and it has no notion of "the pose was changed outside" (editor, door, debug tool). Our
   rebuild cost is paid whenever anything changes, and a door/editor write can be silently overwritten by the next step.
   Moves: keep the spawn description beside the handle, rebuild only on a description change, and treat a pose the backend
   did not write as an input, not as drift. The markers stay on the backend side, outside the state hash.
   Proof: crates/physics lib test with the rapier feature: 600 steps with no change rebuild 0 bodies; one changed radius
   rebuilds exactly 1; a pose written between steps survives the next step; `state_hash` equal in all three runs.
   Kill: kill it if the description is a partial copy that can drift from the spawn list (then rebuild never fires and the
   body stays stale forever), or if the markers enter the hashed authority state (they are build bookkeeping, not sim).

4. Cook-time keyframe reduction by tolerance, with a measure path that touches nothing | UHE/src/renderer/src/animation/compression.rs:1-7,
   :13-32, :34-68, :133-214, :228-282, :310-337 (read 2026-10-18) | Apache-2.0 | ANIM/ASSETS (presentation clips only) |
   lands crates/anim (clip cook) + crates/assets | S-M
   Idea: a greedy linear reduction: an interior key is dropped when the straight blend between the last kept key and the next
   source key stays within tolerance at EVERY source sample time, so the track never deviates by more than that tolerance
   (":1-7"); step tracks keep every key because dropping one changes their semantics (":6-7", ":228-234"). Tolerance is per
   value type - position 1e-3, scale 1e-4, rotation 0.02 rad (":13-32") - and the stats report keys and bytes before/after
   plus a reduction ratio (":34-68"). `compute_keep_mask` keeps first and last and walks the rest (":154-214"); the API is
   split: `compress_clip` applies, `measure_clip` reports without touching the clip (":328-337"), and there are tests for a
   straight track reducing to its ends, a curved track keeping the key that carries the curve, and step tracks untouched (":351-401").
   Maps: our clips ship as authored; a packed clip carries every key. The 10-12 anim card rejected ACL ("pack.rs exists; no
   memory problem to fix") - this is not ACL: ~130 lines, no codec, a tolerance and a measurement, and it can be run at cook
   time on the clips we already cook.
   Moves: reduce at cook time with the tolerance chosen per value type from what the eye and the hash need; ship `measure`
   first so the cost (bytes saved, error bound) is a number before anything is applied.
   Proof: crates/anim lib test: a straight track reduces to its ends; a curved track keeps the curvature key; a step clip is
   byte-identical; measure reports the same numbers twice and leaves the clip unchanged; the golden hash is unchanged.
   Kill: kill it for any clip the sim or the hash reads. A tolerance is not exact, and the anim contract must not gain a
   lossy step. Also kill it if a STEP/idle clip loses an interior key (the header says the semantics change).

=== 3. NOT WORTH IT ===
- The Vulkan backend (UHE/Stormbreaker/vulkan, ~50 files) and the RHI trait split: engine2040 is wgpu 30 only (skill
  wgpu-render, and the 2026-10-07 decision to own one renderer). A second backend with hand-written barriers is a second
  truth to keep in sync with the captures.
- The task DAG (UHE/src/jobs/src/taskgraph.rs:45-55: `deps_remaining` re-armed each execute, a SIGNED `total_remaining` so a
  dispatch bug shows as a negative instead of hanging, `GraphPtr` with a SAFETY note): a good design with no consumer here -
  our tick is one thread plus off-thread results at a tick boundary, so this is a note, not a steal.
- docs/PARITY.md and the "port the C++ upstream at commit 780d010" workflow: excellent discipline for their project (they
  catch three stale upstream doc claims); engine2040 has no upstream to chase, and our parity target is our own golden.
- The editor (UHE-EDITOR/mirror + editor, ~40 panels) and the `File.Name.rs` naming style (UHE/src/... not crates/): no
  consumer, and the layout does not match our crate map.

DONE 4 steals, 8 donor files read.
