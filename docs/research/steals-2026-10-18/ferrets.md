STEAL CARD: valeriyr/ferrets (BLOCK 10, position 30, GitHub topic game-engine?l=rust&o=desc&s=updated). Read live 2026-10-18, read-only. No bytes vendored.
Donor head read: 7814b152494bf990f74d3d2bfa228416f497c596 (branch main).

What it is: a deterministic RTS engine in Rust on Bevy. Fixed-point/integer sim on a fixed tick, lockstep networking (commands only), checksums per interval for desync, Lua-scripted content and AI, grid pathfinding, streaming replays, Bevy only at the render edge. 13 crates (ferrets-simulation, -network, -replay, -math, -random, -pathfinder, -physics, -content, -script, -steam, -geometry, -bevy-plugin, ferrets-demo). 5 stars, 0 issues, active.

Licence: MIT OR Apache-2.0 (read live 2026-10-18: LICENSE-MIT = "MIT License", LICENSE-APACHE = "Apache License, Version 2.0"; GitHub-detected SPDX: Apache-2.0). No asset licence needed to read code.

Engine2040 anchors it lands on:
- crates/net/src/replay.rs:27-32 (REPLAY_VERSION 1, KEYFRAME_EVERY 600, in-memory keyframes) and :15-17 (12 bytes/tick checkpoints).
- crates/net/src/ack.rs:1-45 (32-bit ack bitfield + SentLedger, WINDOW 64) and crates/net/src/courier.rs:1-14 (redundant lockstep blob resend).
- crates/arena/src/sync.rs (SystemHashes, verify_rollback) and crates/net/src/netcode.rs (snapshot/restore).
- crates/core (engine-core): Fnv64, mix64 (integer hash + mix owners).

=== STEALS (ranked by value per hour) ===

1. Crash-tolerant streaming replay file: length-prefixed records, truncated tail = clean EOF, flush per tick | valeriyr/ferrets@7814b15:crates/ferrets-replay/src/format.rs:74-118 (write_chunk/read_chunk + read) and crates/ferrets-replay/src/recorder.rs:26-33 | MIT OR Apache-2.0 | SIM TOOLING (replay) | lands crates/net/src/replay.rs:27 and a new on-disk tape file | S
   Idea: MAGIC prelude ["F","R","E","P"] (format.rs:18), then per record a u32 little-endian length followed by `bcs` bytes. A recording flushes after every record (recorder.rs:30), so a cut-short stream ends in a truncated chunk; the reader treats partial/EOF as end-of-stream and keeps every complete record before it (format.rs:97-118). The header is mandatory (a missing header is an error, a missing trailing record is not, format.rs:49-52).
   Maps: engine2040's replay is a 12-byte keyframe every 600 ticks held in memory with REPLAY_VERSION 1 and no durable file. Steal the framing rules for the first file-backed tape: append-only, flush per record, versioned envelope, tolerance of a truncated tail. Same shape fits the arena-capture tooling and bug bundles.
   Kill rule: kill it if the replay must seek back to patch a header or a length (only an append-only stream wins here); keep the in-memory keyframe grid otherwise.

2. Deterministic state-checksum rules that stop false desyncs | valeriyr/ferrets@7814b15:crates/ferrets-simulation/src/checksum.rs:31-37 (CHECKSUM_INTERVAL = 8, fixed seed 0), :75-84 (write_str widens the length to a fixed u64), :93-151 (entities visited in ascending SimulationId order) | MIT OR Apache-2.0 | SIM (proof) | lands crates/arena/src/sync.rs and the HS2 per-tick hash rows | M
   Idea: fold only deterministic integer/fixed-point state; never fold the ECS `Entity` handle (assigned non-deterministically, checksum.rs:4-8); visit entities in ascending integer id so two peers hash the same sequence; every field is fixed width and explicitly little-endian; a string's length prefix is widened to eight bytes, never written as `usize` (four bytes on a 32-bit peer), else the digest depends on pointer width (checksum.rs:75-84).
   Maps: engine2040 hashes per tick and posts golden rows. Steal the anti-false-desync rules as a checklist for every new hashed field: fixed width, explicit endianness, sorted integer keys, no ECS/handle/pointer/`usize`, and an explicit interval (their 8) documented against the golden cadence.
   Kill rule: kill any field that is not in the snapshot (it cannot be part of the state hash) and revert if the golden hash d57b5c8ae3096614 moves.

3. Two-plane demux: control and gameplay views over one socket | valeriyr/ferrets@7814b15:crates/ferrets-network/src/demux.rs:34-51 (split) and :60-77 (pump routes by Message variant, connection events to both) | MIT OR Apache-2.0 | SIM-adjacent (wire only) | lands crates/net (courier.rs owns its UDP socket today) | S
   Idea: wrap one transport, tag on send, and on receive route each message to the control inbox or the gameplay inbox by its variant; connect/disconnect goes to both; each plane sees only its own traffic and presents the same transport trait.
   Maps: engine2040 runs lobby/relay control and courier/delta gameplay on separate sockets. If the siege wants one UDP socket for both planes (NAT, port budget), steal the split; it is a pure routing view, no sim state.
   Kill rule: kill if the planes need different reliability, MTU or pacing that one socket cannot serve, or if the split adds a lock/latency hop on the gameplay path.

4. Replay record carries inputs, drops and a periodic checksum; header pins version + engine + model | valeriyr/ferrets@7814b15:crates/ferrets-replay/src/record.rs:8-20 (tick, inputs, dropped, optional checksum) and crates/ferrets-replay/src/header.rs:41-59 | MIT OR Apache-2.0 | SIM TOOLING (replay) | lands crates/net/src/replay.rs FighterTape + REPLAY_VERSION | S
   Idea: one record per tick holds exactly the realized input; players whose drop took effect that tick are recorded so playback excludes them exactly as live, not as idle-but-present; a checksum is sampled at the interval and re-read on playback to prove determinism; the header records the movement model and projection because they change the sim without changing the game's name.
   Maps: engine2040's tape records inputs; steal recording drop events and the "anything that changes the sim but is not in the name gets recorded" rule (MOVEMENT_MODEL/PROJECTION ~ our config/rule set and content version).
   Kill rule: kill the drop records if a dropped seat already stops contributing deterministically without a marker and the tape replay matches byte-for-byte.

=== NOT WORTH STEALING ===
- Integer mix fn crates/ferrets-random/src/hash.rs:5-12: a MurmurHash3-style finalizer; engine-core already owns Fnv64/mix64.
- Bevy integration, Steam transport and Lua content: not the engine2040 shape (no Bevy, no Lua).

DONE 4 steals, 8 donor files read.
