STEAL CARD, wave 2026-10-18/5, position 15/50. Read only, no bytes vendored.
Donor: aethelisdev/aeon-engine @ 521a06270857 (main, 2026-10-18T14:43Z, 24 stars) https://github.com/aethelisdev/aeon-engine
Source list: github.com/topics/game-engine?l=rust&o=desc&s=updated (position 15).
Every reader below was fetched live on 2026-10-18. Golden d57b5c8ae3096614 is P2P on every row.

== 1. WHAT IT IS ==
"The 3D Safe Modular game engine", a wgpu engine in 28 crates: ae_core (ecs, camera, time, spatial,
telemetry, registry), ae_renderer, ae_physics, ae_audio, ae_animation, ae_editor + ae_editor_ui, an
immediate-mode UI framework of its own (irisui: core, layout, text, widgets, dock, wgpu, winit), and a
plugin pair (ae_plugin_api / ae_plugin_host) that loads cdylib plugins and hot-reloads them. Its selling
point is modularity with a single isolated unsafe boundary.

== 2. LICENCE ==
MPL-2.0. LICENSE line 1 reads "Mozilla Public License Version 2.0" (read 2026-10-18); every source file
carries "SPDX-License-Identifier: MPL-2.0" (e.g. crates/ae_core/src/telemetry.rs:1). MPL is file-level
copyleft: a port of a file keeps that file's terms, so a port should be a rewrite of the idea.

== 3. STEALS (ranked by value to the duel per hour) ==
1. Frame telemetry as plain structs: a zero-allocation ring, pacing stats, CPU/GPU stage timings, a draw
   breakdown and a VRAM breakdown | crates/ae_core/src/telemetry.rs:10-20,41,96,177-198,202-223,227-238,240-268,270-281 (read 2026-10-18) | MPL-2.0 | PRESENTATION | crates/perf/src/profiler.rs (FrameReport), the render pass-timing row, a VRAM report row | S
   Idea: FrameRingBuffer<const N=240> (:16) stores frame times in a fixed array with push-and-wrap and no
   heap (:41); calculate_stats (:96) fills FramePacingStats (:177-198) with average fps and frametime,
   variance, the 1% and 0.1% LOW fps (the 99th/99.9th percentile slowest frames), min/max, spikes over
   16 ms and 33 ms, and a stutter rate. CpuSyncTimings (:202) splits main_logic / physics / render_prep /
   wait_for_gpu / ui_editor and answers is_cpu_bound(total_gpu_ms) (:219: active CPU > GPU and waiting
   < 2 ms). GpuPassTimings (:227) is four named passes (shadow, main opaque, post, ui) plus a total.
   DrawCallBreakdown (:242) counts total/batched/instanced/compute/culled, triangles and vertices, and
   gives instancing_ratio_percent (:261). VramStats (:272) splits textures / mesh-index / uniform+targets.
   Maps onto us: perf/src/profiler.rs already has a FrameReport with p50/p95/p99; this adds the shape of
   the rest of the panel as DATA rather than UI — 1%/0.1% lows and a stutter rate instead of only
   percentiles, a four-pass split for the GPU (which is exactly the XX1/arch.md steal-1 goal: "the top 3
   passes named"), a draw breakdown with culled_meshes (the number render-cull-vram wants to move), and a
   VRAM split. It is a `Copy` struct with no allocation, so it can ride the frame report without a hot
   alloc (tick-and-runtime).
   Moves: percentiles only -> + 1%/0.1% lows, stutter rate, spike counts; unnamed GPU time -> 4 named
   stages; VRAM per subsystem unknown -> one struct; culled/instanced counts unknown -> per frame.
   Proof: `cargo test -p engine-perf --lib frame_pacing_stats` x2 on a fixture series whose 1% low is
   known by hand; the CSV gains the columns; golden unchanged.
   Kill: if a field needs a GPU readback per frame, or if any stat enters the tick or a hash. Timing
   numbers stay presentation (HASH-LAW).

2. A native plugin host whose unsafe is two functions, an ABI hash symbol, and versioned staging copies
   of the library | crates/ae_plugin_host/src/lib.rs:6-15,23-35,55-63,69-83,99-120 ; crates/ae_plugin_api/src/abi.rs:8-19,22-29,33 (read 2026-10-18) | MPL-2.0 | PRESENTATION / dev only; a restore must hash-equal | crates/editor (dev-only dylib), the reload-drill row (arch.md steal 10), rust-safety rules | M
   Idea: the crate doc says this is "the ONLY unsafe in the entire Aeon engine", in load_library() and
   call_plugin_update() (:13-15). Before loading, the host copies the freshly built library to a
   versioned staging name (copy_to_staging :69 copies source -> staging/<stem>_v<N>.<ext>, :55
   versioned_filename) so the compiler can overwrite the original while the old copy stays loaded —
   the same reason Fyrox copies a private DLL name. It then resolves a `plugin_abi_hash` symbol
   (:110-113) and refuses a null/mismatched hash (:118), and the hash itself is a const string per
   profile (abi.rs:8-19, ENGINE_ABI_HASH with a CStr twin for FFI).
   Maps onto us: arch.md steal 10 fixed the SHAPE (copy the file, load the new one, then drop the old)
   but had no working example of the ABI guard. This is that example, and it also answers the rust-safety
   bar ("SAFETY docs, lock spans, join"): one crate, one trait, two unsafe functions, every unsafe block
   carrying a SAFETY comment that names the path guarantee (:100-102, :108-109, :115-116). Our tree denies
   unsafe (Cargo.toml:47); a dev-only presentation dylib is the one place the owner may lift it.
   Moves: reload drill has no ABI guard -> a hash symbol per plugin refused on mismatch; unsafe sites 0
   (today, no dylib) -> a named pair with SAFETY docs; a failed load leaves no plugin -> load new, swap,
   then drop old. Proof: `cargo test -p engine-app --lib reload_drill` x2 (snapshot, relaunch, restore,
   same hash at tick N+K) plus a test that a deliberately wrong hash is refused.
   Kill: any unsafe outside those two functions, a loaded plugin reachable in a ship build, or a
   restored hash that differs from the straight run.

3. Registry-driven entity snapshots for undo/redo, with entity-id remapping | crates/ae_editor/src/undo_redo.rs:4-8,13-30,41,46,64,86,289,327 (read 2026-10-18) | MPL-2.0 | editor only | crates/editor/src/commands_coalesce.rs, crates/editor/src/commands/component.rs, crates/ecs/src/snapshot.rs | M
   Idea: an EntitySnapshot (:27) is just a list of SerializedComponent{type_name, data: Vec<u8>} (:15),
   captured and applied through the central ComponentRegistry rather than hardcoded fields (capture_with_registry :46, apply_with_registry :69, spawn_with_registry :91) so a new component is undoable
   the day it is registered. Command::undo/redo (:157/:222) re-spawn entities, and remap_entity (:289)
   rewrites the stored ids so a redo after a re-spawn still points at the right entity.
   Maps onto us: our editor already coalesces commands (commands_coalesce.rs) and our ecs has snapshot.rs
   / world_snapshot.rs; the steal is the REGISTRY indirection (undo works for a component nobody wrote
   undo code for) and, above all, remap_entity — a snapshot that stores entity ids is wrong the moment a
   spawn reuses an index, and our spine is a fixed hand of 16 slots where reuse is the normal case.
   Moves: a new component needs undo code -> registry handles it; a redo after respawn points at a stale
   slot -> remapped; undo coverage per component 0 -> all registered. Proof: `cargo test -p engine-editor
   --lib undo_registry` x2: a component registered in the test can be captured, undone and redone, and a
   redo across a respawn lands on the same entity.
   Kill: if the serializer writes unordered bytes (our codec is sorted/diff-stable, scene/src/codec.rs),
   or if a snapshot is taken mid-tick.

== 4. NOT WORTH IT ==
- irisui (immediate-mode UI + dock + layout + text): a whole UI framework to replace egui; we are on
  egui 0.36 and the panel budget is the only ask (parked.md ED14-ED19). Structure, not a port.
- hecs as the ECS backend: ours is a fixed 5-column spine; hecs is a general archetype store we do not
  want in the hash (arch.md "not worth it" already says this).
- ae_physics / ae_2d / ae_animation crates: small didactic modules beside Rapier, less than what we have.
DONE 3 steals, 9 donor files read.
