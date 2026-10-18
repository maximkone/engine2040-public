STEAL CARD, wave 2026-10-18/5, position 45/50. Read only, no bytes vendored.
Donor: trentpolack/joy-engine @ d91c1cdf2c7b (develop, 2026-10-17T22:05Z, 1 star) https://github.com/trentpolack/joy-engine
Source list: github.com/topics/game-engine?l=rust&o=desc&s=updated (position 45).
Every reader below was fetched live on 2026-10-18. Golden d57b5c8ae3096614 is P2P on every row.
Default branch is `develop` (not main); last push 2026-10-17, one day before this read.

== 1. WHAT IT IS ==
A cross-platform engine for prototyping, procedural generation and "things that seem fun", by Trent
Polack. It has a form language (procedural geometry) with its own lab tool, a destruction system built on
chunk support, an SDF collision path plugging into Rapier3D, a particle lab, an editor with a level
document and a property editor, and schemas for every authored document. It is small and sharp: each
subsystem keeps its rule in a doc comment and its data in the type.

== 2. LICENCE ==
MIT. LICENSE line 1 reads "MIT License / Copyright (c) 2026 Trent Polack" (read 2026-10-18), and every
source file repeats it (e.g. src/physics/destruction/chunk_support.rs:2 "Licensed under the MIT License").

== 3. STEALS (ranked by value to the duel per hour) ==
1. Data-driven component authoring contracts: schema, presets, conditional fields, advisory validation |
   src/gameplay/component_schema.rs:4-9,26-28,52,64,113,139,150,174,185,198-230,247-262,306,323,348 (read
   2026-10-18) | MIT | editor/content only (never the tick) | crates/editor/src/inspector_auto.rs,
   crates/editor/src/commands/component.rs, crates/scene/src/codec.rs, crates/editor/src/inspector.rs | M
   Idea: a schema describes AUTHORED JSON; runtime behaviour stays with the systems that interpret a
   component, and the two key rules are in the doc comment (:7-9): "Fields outside a schema remain valid
   custom data and round trip unchanged, and component types without a schema are never reported."
   A field is a ComponentFieldKind (:28: Number, Boolean, Text, Choice, Vector3, Color, ...), may carry a
   ComponentFieldCondition (:52) so it is only active when a sibling field says so
   (is_component_field_active :348), and lives in a ComponentFieldSchema (:64) with a checked constructor
   (:115). A ComponentSchema (:150) plus ComponentPreset (:139) sit in a ComponentSchemaCatalog (:198)
   that can register (:230), look up by kind (:247), create a component from a preset (:262) and list
   schemas (:257). Validation is ADVISORY and returns a Vec<ComponentIssue> (:174, :306), each naming the
   authored path that needs attention; validate_entity (:323) runs a whole entity.
   Maps onto us: arch.md notes our editor has "schema rows without reflection" (inspector_auto.rs:1-10)
   and a big editor plan with parked rows. This donor is the non-reflection path done well, and it adds
   the two rules we do not have: an UNKNOWN field is legal and round-trips untouched (so a hand-edited
   scene never loses data to the tool), and a component with no schema is silently ignored rather than
   flagged (so the tool never blocks). Plus conditional fields, which is how a fight config shows the
   right rows for the right move.
   Moves: an unknown authored field is dropped or flagged by the tool -> round-trips unchanged; validation
   is pass/fail -> a list of issue paths (advisory); a component needs inspect code -> a schema row.
   Proof: `cargo test -p engine-editor --lib component_schema` x2: an unknown field survives a
   save/load round-trip byte-identical, a missing required field yields one issue with its path, and a
   schemaless component is not reported. Golden unchanged.
   Kill: if a schema change can alter cooked bundle bytes or any hashed value, it stays editor-side
   (a schema is a view of content, never content).

2. A geometric masonry support graph: face contacts carry support up and sideways, horizontal travel
   spends a cantilever allowance | src/physics/destruction/chunk_support.rs:4-8,19-20,29-39,41-56,58-63,
   65-83 (read 2026-10-18) | MIT | SIM (but a new, self-contained rule; needs a hash receipt) |
   crates/destruction/src/bonded.rs, crates/destruction/src/bonded/blast.rs, crates/destruction/src/
   structure.rs, crates/destruction/src/cluster.rs, crates/destruction/src/bond_grade.rs | M
   Idea: the doc comment states the rule and its limits in one breath (:4-8): face contacts carry support
   upward and sideways, horizontal offsets spend the cantilever allowance, and it is "a geometric support
   rule, not a stress, torque, or material-load solver". The topology is "built once for the fixed chunk
   collection; damage only changes which chunks are active" (:8). ChunkSupportOptions (:31) is
   ground_height, maximum_cantilever_distance and an optional contact_tolerance (default 0.0001, :20);
   ChunkSupportState (:47) reports per chunk: chunk_id, anchored, supported, and lateral_distance as
   Option<f64> where None means no permitted route to a ground anchor (:43-44, :54-55). SupportEdge (:60)
   carries the distance a hop spends, and ChunkSupportGraph::new (:80) builds every contact pair, failing
   with an explicit error on a non-finite ground height or a negative/non-finite span or tolerance
   (:86-93).
   Maps onto us: we already have a destruction stack (bonded bond grading, blast, cluster, structure) but
   nothing that answers "does this wall still stand after the piece I just removed", which is the arena
   question behind sunk_cm and the break receipts. Two transferable properties: the rule is DETERMINISTIC
   and geometric (integer-ish distance comparisons, no solver, no iteration count), and the expensive
   topology is built once so damage is a cheap re-walk of a fixed graph. The typed Option<distance>
   answer also gives a receipt ("unsupported, nearest anchor 0.4 m too far") instead of a bool.
   Moves: "did the wall fall" by physics -> by a baked support graph; a collapse reason unknown -> the
   lateral distance to the nearest anchor; per-step cost of support unknown -> one graph walk over
   active chunks. Proof: `cargo test -p engine-destruction --lib support_cantilever` x2 on a fixture wall
   (remove the anchor, the wall loses support; remove a cantilever tip, it does not) plus a golden x2
   identical run once it enters the tick.
   Kill: if the graph changes a hash without a receipt, or if a single-frame re-walk of the active set
   costs more than the physics step it replaces, it lands as a tool/receipt only.

3. A Rapier narrow-phase dispatcher that ADDS a shape kind without changing any existing pair, with
   per-pair workspaces and never-feeding diagnostics | src/physics/sdf/sdf_dispatcher.rs:6-12,64-99,
   101-110,121,131,221 (read 2026-10-18) | MIT | SIM (physics; Rapier is our backend) |
   crates/physics/src/rapier.rs, crates/physics/src/query.rs, crates/physics/src/limb.rs,
   crates/physics/src/ragdoll.rs | M
   Idea: SdfQueryDispatcher runs FIRST in the solver's dispatcher chain, and pairs with no SDF collider
   return Unsupported (:6-8) so the default dispatcher handles them "exactly as before" — an extension
   that cannot perturb the pairs it does not own. One manifold per search point, merged when normals are
   nearly parallel (:8-9). Each pair keeps a workspace (:121 WorkspaceData for SdfPairWorkspace) holding
   its own DETERMINISTIC random stream and the cached points that warm-start the next step, so a carried
   point carries its solver impulses (:11-12). SdfCollisionCounters (:68) counts pairs, contacts,
   iterations and evaluations over "the current fixed step" (:64), resets per step (:77) and snapshots
   (:84); the doc is explicit that they are "diagnostics only: counters never feed back into the
   simulation" (:66).
   Maps onto us: crates/physics is Rapier3D already (rapier.rs), and our limbs/ragdolls/query path is
   where a new collider kind would land. The steal is the SHAPE of a safe extension: chain a custom
   PersistentQueryDispatcher (:131) in front, return Unsupported for everything you do not own, keep a
   per-pair deterministic workspace so the extra shape does not break replay (net-rollback / determinism),
   and expose per-step counters that provably cannot change the tick. The ParallelBitfield-ish
   "counters never feed the sim" rule is exactly the HASH-LAW line our perf numbers must respect.
   Moves: a new collider kind risks perturbing every pair -> Unsupported keeps the old path bit-for-bit;
   extra-shape determinism unknown -> per-pair seeded workspace; per-step collision cost unknown ->
   4 counters, presentation only. Proof: `cargo test -p engine-physics --lib sdf_dispatch_unsupported`
   x2: with the dispatcher chained and no new collider present, the golden hash and the manifold count
   are unchanged; with one present, replay x2 gives the same hash.
   Kill: any counter that reaches the tick, or any pair whose behaviour changes when the new collider is
   absent.

4. Offscreen capture target with CPU readback, built for headless golden images | src/rendering/
   capture.rs:4,11-16,18-26,28-54,62 (read 2026-10-18) | MIT | PRESENTATION | crates/render/src/present/
   gpu_present, crates/endpoint play/capture, crates/proof/src/gpu_live.rs | S
   Idea: OffscreenTarget (:19) is a renderable texture that reads back as tightly packed RGBA8 rows;
   CAPTURE_FORMAT is Rgba8Unorm, deliberately NON-sRGB to match the canvas output path (:11-12), and the
   copy uses wgpu's COPY_BYTES_PER_ROW_ALIGNMENT (:16) with read_rgba (:62) doing the pack. The wasm note
   (:21-22) is the trap: browsers cannot block on readback, so the texture is kept alive there but only
   native reads back.
   Maps onto us: our capture path exists (door capture, present/gpu_present); the transferable bits are
   the small ones that bite: pick the capture format to MATCH the presentation path so a golden image and
   a window frame are the same bytes, and treat wasm readback as a compile-time difference (cfg) rather
   than a runtime failure. Proof: `cargo test -p engine-render --lib capture_roundtrip` x2: a known
   clear colour reads back as the expected RGBA8 bytes with the expected row padding.
   Kill: if the capture format stops matching the window path (a golden that no longer equals the frame),
   the row is a regression, not a steal.

== 4. NOT WORTH IT ==
- The form language and form-lab (src/form/*, editor/tools/form-lab/*): a whole procedural-geometry DSL
  with its own lab; a research project of its own and no duel need.
- The particle lab and joyfx/post-processing documents: editor tooling for a different content pipeline.
- Its architecture/layers.json (a declared layer graph): the idea (documented edges) is already what
  docs/ARCHITECTURE.md does; nothing to port.
DONE 4 steals, 5 donor files read.
