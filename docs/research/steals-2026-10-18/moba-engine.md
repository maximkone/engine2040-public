# STEAL CARD: moba-engine (wave 2026-10-18/2, position 2)

Donor: `JakeGT/moba-engine` @ `main`, pushed 2026-10-18T16:30:57Z, created 2026-10-18, 1 star, 8,302 KB, 14 crates, README claims 216 tests.
Read live 2026-10-18 with `gh api` and `raw.githubusercontent.com`, read-only: no clone, no build, no bytes vendored.
Licence: **MIT OR Apache-2.0** (dual). Root `LICENSE` is a pointer file ("dual-licensed under either of"); `LICENSE-MIT` ("MIT License, Copyright (c) 2026 MOBA Engine contributors") and `LICENSE-APACHE` read live today; `Cargo.toml` `workspace.package.license = "MIT OR Apache-2.0"`. `gh api license.spdx_id` says `NOASSERTION` because the root file is a pointer, not a canonical text.

## What it is

A server-authoritative Rust + Vulkan MOBA at 0.1 preview: a deterministic 30 Hz 5v5 simulation on Q32.32 fixed point (`Fix`, i64), custom UDP with intent input, per-viewer visibility-filtered delta snapshots, local-hero prediction and remote interpolation, a headless authoritative server with bots, a Vulkan client, plus developer binaries (`state-hash`, `soak`, `fuzz-wire`, `match-soak`) and `justfile` quality gates. Its shape is close to ours: layered crates, a frozen golden state hash, per-crate determinism guards, replay by input tape.

## Steals

**1. Replay header pins the content digest and a sim-logic version, and every failure is a named variant** | `crates/sim/src/replay.rs:8-34` (format v5: magic `MOBARPL1`, `VERSION: u8 = 5`, `SIM_LOGIC_VERSION: u8 = 5`, `content_hash u64`, `MIN_CHECKPOINT_BYTES = 12`), `:37-82` (`ReplayError::{Truncated, BadHeader, UnsupportedVersion, UnsupportedLogicVersion, ContentMismatch{expected,actual}, CheckpointMismatch{tick,expected,actual}, HashMismatch{expected,actual}}`), `:106-113` (`"replay diverged at tick {tick}: expected {expected:#018x}, got {actual:#018x}"`) | MIT OR Apache-2.0 | SIM (tooling) | lands on `crates/net/src/replay.rs:26-32` (`REPLAY_VERSION = 1`, `KEYFRAME_EVERY = 600`, `Keyframe{tick,hash}`) and `crates/proof` tapes
Idea: a tape that records *what world it was recorded against*, not only its inputs: a content digest and a tick-logic version. A tape whose content or tick behaviour moved refuses by name (`ContentMismatch`, `UnsupportedLogicVersion`) instead of surfacing as a bare hash mismatch on the last tick. The error already carries the tick, which is what `engine_core::diverge::first_divergence` (`crates/core/src/diverge.rs:11`) gives us at the field level — pair them.
Moves: today a tape carries `ArenaConfig` and a keyframe grid; add two scalars and three refusal variants. A tape recorded before a content change stops instead of replaying into a wrong bout.
Kill: kill if `node tools/tape-checksum.mjs` already refuses a tape whose config digest differs, or if the two extra header fields move the frozen tape bytes (MP9 shapes). Then keep the error variants only.

**2. Allocation budget as a named-scenario matrix that prints its numbers** | `crates/sim/tests/allocations.rs:24-48` (`CountingAllocator` as `#[global_allocator]`), `:50-70` (steady 300 ticks: `allocations <= 2`, `bytes <= 512`, after printing `"steady ticks: {n} allocations, {b} bytes over 300 ticks"`), `:72-134` (six status modifiers), `:136-161` (wave spawn: 8 / 1024 B), `:185-223` (respawn), `:225-256` (shop) | MIT OR Apache-2.0 | SIM (proof) | lands on `crates/core/src/alloc_probe.rs:1-22` (`CountingAlloc` armed per thread, `measure(|| ...) -> AllocCount`) and the arena tick budget
Idea: the guard already exists here, but the *shape* is the steal: five named gameplay scenarios, each with its own budget and an `eprintln` of the measured numbers before the assert, so a regression names the scenario ("modifier ticks allocated 5 times") instead of a bare count.
Moves: one budget for the quiet tick becomes one per scenario (status effects landing, a wave spawning, a respawn, a purchase). Numbers in the log are evidence for the tick-budget row.
Kill: kill if `crates/observe`'s tick-histogram tests already cover these scenarios with the same budgets; then only add the printed line.

**3. Reconnect grace: the slot is reserved, a bot covers it, and the client resumes on its session token** | `docs/architecture.md` "Match flow" and "Server authority and netcode" (silence marks a client disconnected, slot reserved, bot covers for `reconnect_grace` 60 s; the client resumes with the session token from the welcome, the server rebinds the address, restores the slot, resets the input sequence and sends a full snapshot) and `crates/net/tests/reconnect.rs` (behaviour test) | MIT OR Apache-2.0 | SIM (session policy) | lands on `crates/net/src/session.rs`, `crates/net/src/lobby.rs`, the bot-cover path in `crates/ai` and the arena's spawn ledger
Idea: four separate rules, each cheap: (a) a dropped peer keeps its slot for a grace window instead of freeing it, (b) a bot fills the slot so the fight keeps its fighter count, (c) resume is authenticated by the session token handed out at connect, (d) on resume the input sequence resets and the peer gets a full snapshot, not a delta against a baseline it lost.
Moves: no grace window today; a dropped client either stalls the bout or is gone. Target: a reconnect inside the window resumes with 0 lost inputs and no extra fighter.
Kill: kill the token step if the transport already binds a peer to a connection id it can rebind (then the token is only a demux id) — or if the duel is 2-player and a drop ends the bout by design.

**4. Reconciliation policy: snap on a big error, ease only when idle, tolerate while moving** | `crates/net/src/prediction.rs:16-21` (`TICK`, `SNAP_DISTANCE = Fix::from_i32(8)`, `IDLE_CORRECTION = Fix::from_bits(2_147_483_648)` = 0.5 units, one half per snapshot), `:80-98` (`reconcile`: not-initialised or dead snaps; error > 8 units snaps (dash/respawn/clamp); only when there is no order does it move 0.5 units toward the target), `:157-171` (a 4-unit lag while moving is tolerated, not corrected) | MIT OR Apache-2.0 | PRESENTATION (drawn state) | lands on `crates/net/src/interp.rs` and the drawn-fighter path in `crates/game`, after a rollback resim
Idea: three regimes instead of one decay constant. A teleport-scale error (respawn, dash, clamp) snaps; a small error with a live order is *latency, not desync*, and is left alone so the fighter does not stutter; a small error while idle eases at a fixed 0.5 units per snapshot.
Moves: if a rollback resim draws the local fighter off its pose, a snap/ease/tolerate policy fixes the visible case at once and leaves the moving case alone. No sim reader sees the offset.
Kill: kill the whole path if a scripted mispredict in the net tests draws 0 error (rollback resim is exact), or if any sim reader sees the correction (grep gate) or the golden moves.

## Already have (checked, not steals)

- Layering gate: `justfile` `layering:` greps `cargo tree -e normal` for `moba-render|moba-platform|ash|winit|gpu-allocator`. We do the same from real manifests: `tools/workspace-check.mjs:19` (`allowedEdges`) over cached `cargo metadata` (`:184`), with `tools/arch-fitness.mjs:1-3` checking acyclicity.
- Per-crate clippy determinism guard (`crates/sim/clippy.toml`, `crates/game/clippy.toml`: `disallowed-methods` Instant/SystemTime/sleep, `disallowed-types` HashMap/HashSet): ours is strictly larger (`crates/core/clippy.toml:1-35` also bans `f32::sin/cos/atan2/exp/ln/powf/mul_add`).
- Q32.32 fixed point with CORDIC and integer Newton sqrt (`crates/core/src/fix.rs:1-16`, `:360-401`): we ship `engine_core::fixed` with a LUT + CORDIC `atan2`.
- FNV-1a stable state hash with typed little-endian writes (`crates/core/src/hash.rs:12-69`): we ship `Fnv64`.
- Golden 600-tick scripted-input state hash diffed in CI (`justfile state-hash-check`, `crates/tools/src/bin/state-hash.rs:7-41`): we ship the golden `d57b5c8ae3096614` plus `tools/golden-matrix.mjs`.
- Per-viewer interest filtering with a starve ledger (`crates/net/src/interest.rs:403`, `:535`): we ship `Interest::select` (distance ring + view hemisphere), so moba's fog-of-war event filter adds nothing.
- Lossy-link soak with per-client KiB/s and tick µs (`docs/perf/0009-presentation-events.md:28-41`): we ship the WAN profiles in `tools/mp6-check.mjs` and `crates/net/src/wan_soak.rs`.

DONE 4 steals, 9 donor files read (replay.rs, allocations.rs, state-hash.rs, interpolation.rs, prediction.rs, hash.rs, architecture.md, adr/0001, justfile, Cargo.toml, clippy.toml x2, docs/perf/0009).
