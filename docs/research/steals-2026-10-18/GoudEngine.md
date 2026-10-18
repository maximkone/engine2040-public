STEAL CARD: GOUDENGINE (wave 2026-10-18, block 7, position 37). Read-only pass; donor files read live
2026-10-18, paths taken from the pinned tree. No bytes vendored.
Repo: https://github.com/aram-devdocs/GoudEngine @ 14516d6e6077 (main, updated 2026-10-18T03:03Z, 6 stars, ALPHA).
Licence: none found. `gh repo view` reports `licenseInfo: null`, and the tree has no LICENSE, COPYING or NOTICE blob
(checked live 2026-10-18); the README claims no licence either. Treat as all rights reserved: ideas and numbers only,
no code, no text.
What it is: a Rust game engine (OpenGL + wgpu backends behind one render trait) with generated SDKs for ten languages over
a single C FFI header, a GGPO-style rollback netcode module, a layer-lint tool, per-frame phase timing, and an MCP bridge
that attaches to a running game.

=== 1. WE HAVE ===
- An acyclic layer map in prose (docs/ARCHITECTURE.md) and a boundary check (tools/workspace-check.mjs: top-level
  `crates/<c>/tests/it/` only, no new `tests/*.rs`, board and doc edits) - plus tools/wiring-cov.mjs and tools/sim-source-scan.mjs.
- Rollback with a cap of 8 and whole-Sim clones: crates/net/src/netcode.rs; per-system desync proof crates/arena/src/sync.rs.
- A runner lane for tests/clippy (node tools/gh-check.mjs) and perf receipts (crates/perf, tools/potato-check.mjs).

=== 2. STEALS (ranked) ===
1. A layer lint: scan the imports, fail on an upward edge, one printed violation per finding | tools/lint_layers.rs:1-16,
   :27-34, :72-90, :99-129, :130-143, :144-149, :166-249, :260-405 (read 2026-10-18) | no licence found | PROOF/TOOLING |
   lands tools/ (a new `layers.mjs` beside workspace-check.mjs) | M
   Idea: five layers are an ordered enum (":27-34"); a file is classified by its path (":72-90") and an import by its first
   path segment (":99-129"); `is_violation` is a plain comparison of the ordinals (":130-143"); exemptions are a named list
   (":144-149"). `main` prints each violation as `file:line: from -> to import` plus the line content (":57-68"), then exits
   0 clean, 1 with violations, 2 on usage error (":166-249"), and the classifier itself is unit-tested (":260-405").
   Maps: docs/ARCHITECTURE.md is a picture, and tools/workspace-check.mjs checks file placement and doc/board shape - nothing
   fails when `engine-core` starts importing a GPU crate. The picture cannot enforce itself.
   Moves: one table crate -> layer, kept in the tool's source but printed in the violation line, an exempt list where every
   entry carries a reason, and exit codes CI can read. Run it in the same place workspace-check runs.
   Proof: `node tools/layers.mjs` exits 0 on master; adding one upward import to a Foundation crate makes it exit 1 and name
   the file and line; the golden hash is untouched.
   Kill: kill it if the layer map lives in the tool while docs/ARCHITECTURE.md says something else (two sources of truth),
   or if an exemption is silent. Both make the green meaningless.

2. Rollback with a snapshot ring, a named mismatch frame, and "evicted" as its own answer | goud_engine/src/libs/networking/rollback.rs:3-5,
   :21-27, :35-54, :104-105, :175-220, :235-250, :265-342, :348-366 (read 2026-10-18) | no licence found | SIM (net) |
   lands crates/net/src/netcode.rs | M
   Idea: `GameState` is `advance(inputs)` + `state_hash()` (":21-27"); `RollbackConfig` defaults to `max_rollback_frames: 7`,
   `input_delay_frames: 2`, `desync_detection: true` (":35-54"); the ring holds `max+1` states and trims the oldest (":104-105",
   ":218-220"); the snapshot is pushed BEFORE the frame is applied, which is what makes "restore frame F" mean frame F
   (":181-186"). `receive_remote_input` keeps `earliest_mismatch_frame` so a rollback reaches far enough (":235-250");
   `rollback_and_resimulate` restores at or before it, truncates the ring, clears the hash cache for the resimulated frames
   and returns how many were resimulated (":265-342"). `check_desync` answers `InSync`, `Desync { frame }` or
   `FrameNotAvailable { frame }` (":348-366").
   Maps: our netcode caps at 8 and clones the whole Sim per advance and per resim step, and `check_remote` compares predicted
   frames. Two ideas are missing: a DESYNC answer that names the frame, and a third answer for "that frame is gone from the
   ring" (today an evicted frame can look like a match).
   Moves: name the frame in every desync result; restore-from-before-the-frame semantics documented at the snapshot site;
   the hash cache eviction tied to the resim window.
   Proof: crates/net lib test: a deliberate remote input change reports `Desync { frame }` at the exact frame; a query for a
   frame older than the ring reports `FrameNotAvailable`, not InSync; tip hash equal to baseline.
   Kill: kill it if the snapshot is taken after the frame is applied (restore is off by one), or if `FrameNotAvailable` gets
   folded into `InSync`. Both turn a real desync into a green run.

3. A hot-path budget asserted in the test, with separate debug and release limits | goud_engine/src/libs/networking/rollback_tests.rs:314-380
   (read 2026-10-18) | no licence found | PERF (proof) | lands crates/net tests + crates/perf | S
   Idea: a dedicated benchmark test rolls back once and times it; the limit is `5_000` microseconds under
   `cfg!(debug_assertions)` and `1_000` in release (":372-374"), the failure message prints the measured micros (":366-378"),
   and `std::time::Instant` is used only here (":5"). The dev loop stays usable, CI still catches a regression.
   Maps: our resim cost is measured outside the test (perf CSV, tools/mp6-check.mjs); a regression lands as a board row after
   the fact. A two-tier assert inside the resim test makes it a red test.
   Proof: `cargo test -p engine-net --lib netcode` on the runner below both limits; a deliberate extra clone pushes release
   over 1 ms and fails.
   Kill: kill the absolute microsecond limit once the machine class changes (a runner is not the dev box) unless the class is
   recorded beside the number, the way a frame-budget line carries its baseline. Without it the test flakes and gets deleted.

4. A fixed table of named frame phases, recorded by name into a thread-local, readable with the profiler off |
   goud_engine/src/libs/graphics/frame_timing.rs:1-40, :42-60 (read 2026-10-18) | no licence found | PERF/TOOLING |
   lands crates/perf + the render frame receipt | S
   Idea: 15 phases each in one field (begin/end frame, surface_acquire, uniform_upload, render_pass, gpu_submit,
   readback_stall, surface_present, shadow_pass, shadow_build, render3d_scene, anim_eval, bone_pack, bone_upload; ":9-40")
   with one recorder that matches the phase NAME to the field (":46-60") and a thread-local so the values exist even when
   the debugger route is inactive (":1-5", ":42-44"). The name set is the contract; a new phase is a field plus one match arm.
   Maps: our per-frame receipt exists, but a phase can be added by writing to a free-form string, so two builds can print
   different phase sets and a diff has no stable keys.
   Moves: one closed enum of phase names, recorded by name; the receipt prints the same keys every run in the same order.
   Proof: run the window twice and diff the phase lines: identical key set and order; a render phase that is off reports 0
   instead of vanishing.
   Kill: kill it if a phase can be recorded without appearing in the printed set (a silently dropped phase), or if the
   thread-local is read across threads (a worker's phases would be lost). Keep the worker numbers in a returned struct.

=== 3. NOT WORTH IT ===
- The MCP debugger bridge (tools/goudengine-mcp, whole crate + ws_relay): crates/endpoint already has the door, the verb
  pack, discovery and a capability table (crates/endpoint/src/discover.rs, door_dispatch.rs, tool_capabilities.rs,
  mcp.rs), and the ai-door rule is that verbs are recorded and idempotent.
- Generated SDKs for ten languages over one C header (sdks/**, one .g.* file per language per type): our consumers are Rust
  and the browser; there is no second-language surface to keep in sync.
- The OpenGL backend and the render-trait matrix (libs/graphics/backend/{opengl,wgpu,null}): engine2040 is wgpu-only
  (skill wgpu-render, wgpu 30); a second backend doubles the surface for no picture change.
- The JNI/Android and Kotlin/Swift wrappers: no mobile target on the board.

DONE 4 steals, 6 donor files read.
