STEAL CARD, wave 2026-10-18/5, position 5/50. Read only, no bytes vendored.
Donor: PommeMC/Client @ 1e9ecc4ea1d3 (master, 2026-10-18T16:14Z, 209 stars) https://github.com/PommeMC/Client
Source list: github.com/topics/game-engine?l=rust&o=desc&s=updated (position 5).
Every reader below was fetched live on 2026-10-18. Golden d57b5c8ae3096614 is P2P on every row.

== 1. WHAT IT IS ==
A Minecraft: Java Edition client written from scratch in Rust + Vulkan (winit, ash/pyronyx, azalea for
protocol pieces), joining vanilla servers 1.20-26.3. A full game client: its own renderer (20+ pipelines,
GLSL not WGSL), a chunk mesher with block AO and a visibility graph, a build of gpu-allocator, its own
protocol crate with version tables, and a scripted in-game test harness that drives the real client.

== 2. LICENCE ==
GPL-3.0. LICENSE line 1 reads "GNU GENERAL PUBLIC LICENSE / Version 3, 29 June 2007" (read 2026-10-18).
No SPDX tag line in the file; the GitHub API reports spdx_id GPL-3.0. Strong copyleft: our engineering
rules (AGENTS.md 4) allow porting donor code from any source, so this is not a blocker, but a port should
be a rewrite of the idea, not a copy of the file, and the card says so plainly.

== 3. STEALS (ranked by value to the duel per hour) ==
1. A Vulkan sub-allocator with best-fit free lists, allocation reports and a visualizer | pomme-gpu-allocator/src/allocator/free_list_allocator/mod.rs:15,26-37,159 ; pomme-gpu-allocator/src/lib.rs:73,90-112,173-227 ; pomme-gpu-allocator/src/visualizer/allocation_reports.rs:24 (read 2026-10-18) | GPL-3.0 | PRESENTATION | crates/render/src/frame_alloc.rs, crates/render/src/gpu.rs, crates/render/src/webgpu_limits.rs, a new VRAM report beside crates/perf | S-M
   Idea: one allocator owns the device; allocations are best-fit into sub-allocated MemoryChunks
   (free_list_allocator/mod.rs:15 `USE_BEST_FIT`, :26-37 MemoryChunk = chunk_id: NonZeroU64, size, offset,
   allocation_type, name, backtrace), sized by AllocationSizes{device_memblock_size, host_memblock_size}
   (lib.rs:173-227) with a MemoryLocation tier (lib.rs:73). Every allocation carries a name; with
   AllocatorDebugSettings.store_stack_traces (lib.rs:90-112) a report can show who allocated what. The
   visualizer (allocation_reports.rs:24) filters/sorts and prints bytes under the filter — a VRAM budget
   ledger you can read.
   Maps onto us: wgpu owns allocation, so we cannot take the Vulkan side. We take the BOOKKEEPING: one
   place that names every buffer/texture, the site, the size and the lifetime, and reports VRAM per
   subsystem (textures / meshes / uniforms+targets). That is the missing half of render-cull-vram's
   "integer pool math" and of the VRAM row the render budget still lacks. The same "named allocation"
   list is what a staging-belt row needs to prove "GPU objects made per timed frame 4 -> 0" (arch.md
   steal 1).
   Moves: named live allocations 0 -> all device objects; VRAM per subsystem unknown -> one report row
   per subsystem; a leak (drops to 0) today invisible -> one report line (unmeasured).
   Kill: if the port needs a second allocator under wgpu, or if the report costs > 0.1 ms per frame, or if
   any byte of it enters the tick.

2. Declarative in-game scenario tests driven against the real client over a JSON-line socket | tools/ingame-test/src/main.rs:1-30 ; pomme-client/src/harness/protocol.rs:14-56,163,232,303,373 ; tests/ingame/blocks.toml:3-31 (read 2026-10-18) | GPL-3.0 | PRESENTATION (test harness, never the tick) | crates/playtest (door.rs, bot_harness.rs, keypoint), crates/endpoint play/capture, the arena receipts | M
   Idea: a scenario is a TOML list of steps — `wait_ms`, a `chat` command (a leading `/` sends a server
   command), or an `expect` of observable state. The harness client speaks one JSON object per line both
   ways: Command (protocol.rs:16: Status, Chat, Click, Inventory, Entities, Player, Hud, Block, LookAt)
   and the snapshots the app answers with (inventory :163, entities :232, player :303, hud :337,
   block :373). The runner starts and verifies the server once (main.rs:20 SERVER_SHA1, :29 OUT_DIR)
   and starts one harness client per script.
   Maps onto us: our playtest crate already has door.rs, bot_harness.rs, keypoint and eval, and
   test-and-proof wants "command - exit - hash - excerpt - verdict". The steal is the SHAPE: a scenario
   file that is only steps and expected state, run against the real binary through the door, with the
   expected values written as data (blocks.toml:17 `expect = { block = "stone", at = [3,-56,4] }`);
   and the rule that an `expect` names a field the app can observe, never a screenshot.
   Moves: a scenario is code today -> data; a new scenario needs a Rust edit -> a TOML row; "did the
   client and the sim agree on this block/fighter state" as one receipt line. Proof: `cargo test -p
   engine-playtest --lib scenario_step` x2 plus the blocks.toml-shaped fixture replayed through the
   door twice with the same hash.
   Kill: if a step cannot be driven through the existing door without a new sim API, or if the harness
   needs a GPU in CI, the row stops at the headless steps.

3. A baked section-visibility graph with a per-frame occlusion walk | pomme-client/src/renderer/chunk/occlusion_graph.rs:1-3,74,86,134,222 (read 2026-10-18) | GPL-3.0 | PRESENTATION (culling; the sim keeps its own visibility) | crates/render/src/gpu_cull.rs, crates/spatial/src/fps_visibility.rs, crates/spatial/src/isovist.rs, crates/spatial/src/why_hidden.rs | M
   Idea: at mesh time each section computes a 6x6 face-to-face connectivity set (a flood fill, the
   vanilla VisGraph); it is stored as a 64-bit mask (VisibilitySet(u64), :74) with visible_between(a,b)
   (:86) and compute_visibility (:134). Per frame a walk (compute_visible_mask, :222) starts at the
   camera section and follows only edges the mask allows, so the render list is conservative and cheap.
   Maps onto us: spatial/src/fps_visibility.rs and isovist.rs already answer "can see"; the steal is the
   BAKED integer mask per cell plus the flood-fill walk that consumes it — no per-frame ray casts, one
   u64 per cell, deterministic, and it names WHY a cell is hidden (why_hidden.rs already wants that
   receipt). It is the culling half of render-cull-vram ("bake bounds once, never in the hash").
   Moves: per-frame frustum-only culling -> + baked visibility mask; a hidden-cell verdict that names an
   edge today -> the edge name; GPU cull dispatch input smaller (unmeasured).
   Kill: one false hidden (a fighter the analytic FPS says is visible) ends the row; also kill if the
   mask costs more than the frustum test it replaces.

== 4. NOT WORTH IT ==
- The Vulkan/ash renderer, the GLSL pipeline set and the chunk mesher: we are wgpu + WGSL, and our meshes
  are cooked offline; the mesher's ideas (block AO, greedy faces) are already covered by assets/src.
- The protocol crate with per-version JSON tables: Minecraft-version data, no duel use.
- Their fork target `pyronyx`: a binding layer, not a technique.
DONE 3 steals, 8 donor files read.
