STEAL-BYRO scout, 2026-10-18. Wave 2026-10-18/1, block 1: github.com/topics/game-engine (l=rust, o=desc, s=updated), position 1. Area: durable file writes, save validation, ragdoll seeding, backend-type isolation.
Read only, ideas only; no code copied, nothing vendored. Every donor file below was fetched live on 2026-10-18.

== 1. WHAT IT IS ==
ByroRedux (matiaszanolli, 26 stars, updated 2026-10-18) is a ground-up Rust + Vulkan rebuild of the Gamebryo/Creation engine lineage, Linux-first, aimed at loading and playing seven shipped Bethesda games (Oblivion, Fallout 3/NV/4/76, Skyrim SE, Starfield) from their own `.esm`/`.bsa`/`.ba2` data. It is an alpha: NIF/BA2/BSA/ESM parsers, a Vulkan 1.3 renderer with ray-query shadows and SVGF, Rapier3D physics with Havok ragdoll import, a Papyrus parser/interpreter, quest and save systems, and a native egui HUD. Our interest is not the content pipelines (we have no Gamebryo data) but the plain-`std` durability, validation and seeding contracts in `crates/core`, `crates/save` and `crates/physics`.

== 2. LICENCE ==
MIT (SPDX `MIT` via GitHub API 2026-10-18; LICENSE read live 2026-10-18: https://raw.githubusercontent.com/matiaszanolli/ByroRedux/main/LICENSE — "MIT License, Copyright (c) 2026 ByroRedux contributors"). Ideas only here; nothing ported.

== 3. STEALS (ranked by value to the fixed tick / cook / ragdoll work) ==
1. Crash-safe write: unique temp, fsync, readback verify, rename, then fsync the parent dir | matiaszanolli/ByroRedux@main:crates/core/src/atomic_file.rs:19-25,27-76 | TOOLING (never the tick) | engine2040 `assets/src/gen_intake.rs:756-765`, `assets/src/cooker.rs`, `editor/src/wal.rs` | S
   Idea: the staging path is `.{name}.{pid}.{counter}.tmp` beside the target, so two processes and two threads in one process never collide on one staging inode (:19-25). The durable sequence, in order: create temp → `write_all` → `flush` → `sync_all` → read back and compare bytes (catches a short write or a lying filesystem *before* it replaces a good file) → `rename` → fsync the **parent directory**, the step most often missed, because a rename is not durable until the directory metadata is on the platter (:27-76). A failure up to and including the rename removes the temp (:63-66). Windows journals the rename, so the dir step is skipped there rather than failed (:39-41). One helper replaced three drifting copies (:3-7).
   Maps: engine2040 cook entries, tapes and reports are written as several plain writes (`gen_intake.rs:756-765`); one shared contract deletes the half-written-entry class XX6 lists, and the readback-verify idea gives us the "kill between file 1 and file 2" accept a cheap check.
   Kill: any write that fsyncs per line in a hot loop, or the readback compare costs > 1% of cook time -> keep the plain write for that path and list it.

2. Pre-save referential-integrity pass that refuses a structurally broken world | matiaszanolli/ByroRedux@main:crates/save/src/validate.rs:24-70 | SIM (a check, not a tick step) | engine2040 `ecs`/`snapshot` world walk before a hash or tape write | M
   Idea: before persisting, walk the World and collect typed violations instead of serialising whatever is there: one-directional or dangling `Parent`/`Children` edges, an equipment slot indexing outside its inventory, an animation clip handle not in the registry, an item-instance id that does not resolve, an entity reference past `next_entity`, a non-finite material scalar. Each error carries the entity, a `ValidationKind` and a detail string, and the save driver refuses the write when the list is non-empty (:24-70). Cross-owner checks (`FormId` needs the data store) are layered on by the binary that owns the resource (:49-53).
   Maps: engine2040's hash law wants "same complete input fixture -> same hash"; a dangling ref or a NaN in the fixture is exactly the corruption tail that makes a hash meaningless, so a pre-hash validate gate turns a silent divergence into one named error before the golden is consulted.
   Kill: if the walk costs more than the tick budget on the fight fixture, run it only in the save/replay path and never in the arena tick.

3. Bounded sanity gate on a ragdoll seed pose, rejecting before the world is touched | matiaszanolli/ByroRedux@main:crates/physics/src/ragdoll.rs:35-115,355-363 | SIM (physics) | engine2040 `physics` ragdoll activation (physics-bodies skill: spawn order, rollback rebuild) | S
   Idea: `seed_pose_is_sane` (:100-115) rejects a seed that is non-finite, beyond a world-space absolute bound (`SEED_SANE_ABS_BOUND_BU = 1.0e8`, :53), or too far from the actor root (`SEED_SANE_MAX_ROOT_OFFSET_BU = 1.0e5`, :54), returning a typed `SeedInsanity` (NonFinite / BeyondWorldBound / BeyondActorReach, :58). `build_ragdoll` (:355-363) returns a `SeedRejection` *without touching the world*, and a test asserts that (:1067). The bound is justified the same way as a keyframe-target bound elsewhere (:35-40).
   Maps: engine2040 ragdolls are rebuilt on rollback; a corrupt or stale seed today fails late and mid-frame. A cheap, typed, pre-spawn gate makes a bad seed one named rejection at the rebuild boundary instead of a physics artefact in the capture.
   Kill: any rejection that changes the golden at tick 600, or a bound that fires on the staged fight's authored poses -> widen the bound and re-record nothing.

4. Backend types kept out of the core layer as a plain-bits newtype | matiaszanolli/ByroRedux@main:crates/physics/src/config.rs:27-50 | SIM (layer hygiene) | engine2040 `docs/ARCHITECTURE.md` acyclic layers; `engine-core` stays std-only | S
   Idea: `TriMeshFlagBits(pub u16)` mirrors `rapier3d::parry::TriMeshFlags` bit for bit, with named consts and one `DEFAULT`, precisely so the `core` crate does not have to alias a Rapier type (:27-50). The physics crate converts once with `from_bits_truncate` (:29-30). All three previously-inline tunable sites become one `ContactConfig` resource (:1-11).
   Maps: engine2040 hashes the authority layer and keeps the float/backend layer separate; a plain-bits newtype is how a rapier-facing constant reaches hashed sim code without dragging the backend type across the layer edge.
   Kill: if the bits drift from the backend's values and no test pins them -> add the one mirror test or drop the idea.

== 4. NOT WORTH STEALING ==
- `crates/core/src/ecs/access.rs:1-31` + `lock_tracker.rs:1-46`: declared access + a runtime ABBA lock-order graph aimed at rayon-parallel ECS. Our spine is 5 systems in a fixed order in one thread (`spine_schedule.rs`); the Bevy `shuffle_seed` steal already covers the verification need, so the runtime lock graph is dead weight.
- `crates/core/src/ecs/debug_load.rs:1-11`: the staged typed request queue drained once per frame is the shape engine2040 already uses (off-thread results at tick boundaries); nothing new.
- The Gamebryo content pipelines (NIF/BA2/ESM, Papyrus, Scaleform-via-Ruffle): no donor data and no target here.

== 5. FILES READ ==
README.md, LICENSE, crates/core/src/atomic_file.rs, crates/core/src/ecs/access.rs, crates/core/src/ecs/lock_tracker.rs, crates/core/src/ecs/debug_load.rs, crates/save/src/disk.rs, crates/save/src/validate.rs, crates/physics/src/lib.rs, crates/physics/src/config.rs, crates/physics/src/ragdoll.rs, crates/renderer/src/deferred_destroy.rs (all @main, 2026-10-18).
