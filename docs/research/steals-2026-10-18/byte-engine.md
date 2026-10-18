STEAL CARD: Game-Tek/Byte-Engine (BLOCK 10, position 40, GitHub topic game-engine?l=rust&o=desc&s=updated). Read live 2026-10-18, read-only. No bytes vendored.
Donor head read: b1119f992c8e9fca472275415851b291339a43eb (branch dev; the repo's default branch is `dev`, not `main`).

What it is: a Rust application runtime for graphics, input, audio, physics, resources, networking and retained UI, split into small crates behind a `byte-engine` facade. Named crates: ahi (audio hardware interface, per-OS backends), behl-ish lower layers plus beld (bake/list/query/inspect/delete CLI), besl (Byte Engine Shader Language: lexer/parser/vm/optimization), betp (Byte Engine Transport Protocol: reliable UDP-ish client/server), byte-engine (facade: application, input, ui, network, physics, rendering, space, time, metrics), math, resource-management (asset pipeline, gpu_worker, ibl, materialx, pbr, processors, solver, stream, inspect), utils. Nightly Rust, pinned toolchain nightly-2026-08-31. 20 stars. Early, breaking API changes expected.

Licence: MIT (read live 2026-10-18: LICENSE = "MIT License ... Copyright (c) 2023 Facundo Villa"; GitHub-detected SPDX: MIT). No asset licence needed to read code.

Engine2040 anchors it lands on:
- crates/render/src/uploads.rs (GPU uploads) and crates/render (staging belt) — the GPU worker frees the CPU pool.
- crates/assets (cook/manifest) and crates/assets/src/retarget.rs (RootTable/ContactTable precedent for a stored-model -> typed-resource conversion).
- crates/game input path (pad.rs, window input) — action bindings.
- crates/net/src/ack.rs:1-45 already has the 32-bit ack bitfield + WINDOW 64, so betp's reliability layer is NOT proposed here.

=== STEALS (ranked by value per hour) ===

1. One thread-confined GPU compute worker for offline bakes, with bounded in-flight and oneshot replies | Game-Tek/Byte-Engine@b1119f9:crates/resource-management/src/gpu_worker.rs:1-6 (IBL generator and material-mip generator share it), :26-30 (POLL_INTERVAL 1 ms), :32-47 (GpuProcessor: submit without waiting, poll when signalled, MAX_IN_FLIGHT bound) | MIT | SIM-adjacent (asset/render off-thread) | lands crates/render/src/uploads.rs + crates/assets cook | M
   Idea: a single worker thread owns the GPU compute context (it never crosses threads), submits up to MAX_IN_FLIGHT requests without blocking, and completes them on poll; callers submit from a pool and await a oneshot reply, so no pool thread blocks while the GPU works and several requests overlap (POLL_INTERVAL only paces completion checks).
   Maps: engine2040 does GPU work on the render/tick path and asset work on pool threads. Steal a dedicated worker with a hard in-flight cap for cookies (IBL, mip bake, mesh cook) so the sim tick and the CPU pool never wait on the GPU; the oneshot reply keeps results arriving at a tick boundary.
   Kill rule: kill if any bake result reaches the sim before a tick boundary, or if MAX_IN_FLIGHT is unbounded (scratch grows without bound).

2. Dependency-aware stored-model -> runtime-resource conversion over named byte ranges | Game-Tek/Byte-Engine@b1119f9:crates/resource-management/src/solver.rs:7-32 (Solver/StoredModel: read the record once, solve dependencies, keep a reader for the binary data) and crates/resource-management/src/stream.rs:1-57 (Stream/StreamMut: one named range of a resource's bytes, e.g. "Vertex"/"Index") | MIT | SIM-adjacent (asset cook/load) | lands crates/assets cook and the runtime asset readers | M
   Idea: a requested resource is read from storage once (`request` passes the already-read record to the solver, solver.rs:19-21); each stored model deserializes its record, recursively solves its dependencies, and keeps a borrowed byte reader for the payload; the payload is addressed by name into typed POD ranges (stream.rs:41-44 casts via bytemuck).
   Maps: engine2040 cooks content into tables read at load. Steal the "record once + named subrange" shape for the cooked manifest: one file entry, a solve step that resolves dependencies, and named vertex/index/anim ranges instead of ad-hoc offsets. Matches the retarget RootTable/ContactTable precedent.
   Kill rule: kill if the added indirection needs a dynamic dispatch or allocation on the load path where a fixed table exists; keep the current cook if it already reads each record once.

3. Action-based input: declarative bindings with a per-action tick policy | Game-Tek/Byte-Engine@b1119f9:crates/byte-engine/src/input/action.rs:1-16 (Action decouples gameplay intent from physical controls; bindings are Keyboard.W / Gamepad.LeftStick) and :42-53 (Action::new over ActionBindingDescription; TickPolicy controls how often events emit) | MIT | PRESENTATION/GAME (input) | lands crates/game input (pad.rs, window handlers) + contract | S
   Idea: gameplay declares an intent (Move, Fire, Confirm) with one or more physical bindings and a type; a tick policy decides how often the action emits (edge vs held vs repeat). Device details are never read by gameplay.
   Maps: engine2040 input feeds the fixed tick through recorded inputs. Steal the Action + TickPolicy split to keep window/device code out of the sim contract and to make an action's emit cadence explicit and testable.
   Kill rule: kill if a binding cannot be replayed deterministically (a tick policy that depends on wall time rather than tick), or if it puts device reads inside the fixed tick.

=== NOT WORTH STEALING ===
- besl (shader language VM) crates/besl/src/{lexer,parser,vm}: engine2040 writes WGSL; no second shader front end.
- betp reliability (crates/betp/src/stream.rs:16-38) and the ack/retransmit logic: engine2040 already has crates/net/src/ack.rs (32-bit ack bitfield, WINDOW 64) and courier.rs.
- Platform graphics/audio backends (ahi, ghi) and retained UI: engine2040 owns wgpu/render and its own shell.

DONE 3 steals, 6 donor files read.
