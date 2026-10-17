---
name: net-rollback
description: "Use when you change crates/net session, snapshot, interpolation or online code, or write a rollback loop (ggrs 0.13 or GGPO style): server authority, prediction and correction, input delay, save, load and advance requests, desync detection, no extrapolation."
---

# Net and rollback

Crate: `crates/net` (`engine-net`). Game side: `crates/game/src/netgame.rs`. Net never owns transport: IO stays outside the fixed tick (`AGENTS.md` rule 3).

Is ggrs a dependency? No. No `Cargo.toml` in the repo names `ggrs` (grep 2026-10-17). `crates/net/src/netcode.rs` and `rollback.rs` reimplement the request pattern and the input queue std-only, as shape only (ported from gschup/ggrs @ e97e3d2, no donor bytes). The ggrs API digest is in `references/ggrs-api.md`. Read it when you port a donor idea (sync-layer checksum, save and load cells, input queue) or if the owner adds the crate.

## Policy: no extrapolation

The engine never extrapolates remote motion. Remotes render in the past through interpolation. A query past the newest sample clamps to the newest anchor and is flagged. The flag is named `QueryKind::Extrapolate` (`crates/net/src/interp.rs`) but the answer is a clamp, not a guess. Same for `crates/net/src/delta.rs::Interpolator` ("clamping, never extrapolating, past the newest sample").

Dead reckoning (Game Engine Gems 2 ch. 18) is not this engine's policy. Skill `ai-door` carries no dead-reckoning rule for the same reason.

## Six rules (Gaffer On Games)

Source: https://gafferongames.com/post/what_every_programmer_needs_to_know_about_game_networking/ (verified 2026-10-10). Own words below.

1. **Server owns the world.** Clients send inputs, never positions. Sim authority sits in `crates/net/src/session.rs::Server`. The client `InputPacket` carries input only.
2. **Predict self, accept correction.** Hide your own latency, then obey. Predict local moves at once. Reconcile from `session.rs::StatePacket` by rewind and replay of inputs.
3. **Keep past inputs to replay.** Correction needs the input log. Keep a ring of past inputs for resends: `session.rs::redundancy_window`.
4. **Snapshot the shared truth.** Deltas ride on full state saves. Persist full state with `crates/net/src/snapshot.rs::save_snapshot` and `load_snapshot`. Send deltas only against a confirmed baseline.
5. **Show others in the past.** Smooth remotes by interpolation, not the latest packet. Render remotes through `crates/net/src/interp.rs::SnapshotInterpolator`. Never snap to the newest packet.
6. **Step fixed turns. Lockstep needs the same start.** Advance the sim in fixed ticks from one input set. Join state comes from `snapshot.rs::ConfirmedHistory`.

Don't: let the client set its own position, feed variable dt to the sim, show remote snapshots unsmoothed, or extrapolate.

## What is in the tree

| Piece | Where |
|---|---|
| Rollback tape, 2 players, input queues (128 slots each) | `crates/net/src/rollback.rs` (`InputQueue`, `SimTick`) |
| Small-window session: save, load, repeat-last predict, resim, hash stream | `crates/net/src/netcode.rs` (`Session<S: Sim>`, `MAX_ROLLBACK` 9 = 150 ms at 60 Hz, `HISTORY_CAP`) |
| Input delay from frame advantage | `crates/net/src/delay.rs` (`clamp_delay`, `DEFAULT_DELAY`) |
| Server, packets, loopback test under latency and loss | `crates/net/src/session.rs` |
| Snapshots, confirmed history | `crates/net/src/snapshot.rs` |
| Delta wire and tick-clock interpolator | `crates/net/src/delta.rs` |
| Presentation interpolation (Hermite, quaternion yaw, snapshot ring) | `crates/net/src/interp.rs` |
| NAT path choice | `crates/net/src/online.rs::resolve_path` |
| Remote view in the game | `crates/game/src/netgame.rs` |

Rules that hold here:
- A resim beyond `MAX_ROLLBACK` refuses `TooFar`. After eviction, reads refuse loudly: `hash_at` returns `None`, `check_remote` returns `UnknownFrame`.
- Interpolated floats are presentation. They never enter the sim hash and never run on the fixed tick. Ring keys stay integer ticks.
- Invalid snapshots reject loudly and change nothing. Never panic on hostile bytes.

## GGPO rules for the game (any rollback loop)

1. **Only state and inputs matter.** Each peer runs a full copy and only inputs travel. Renderers and audio that do not change the outcome are not state.
2. **Deterministic, serializable, steppable.** The step is deterministic. State is fully encapsulated and serializable. The game can load, save and run one frame without drawing. Advance by a fixed time step; keep update apart from render.
3. **Seed and clock.** Same RNG seed at frame 0 on every peer, with the generator state inside the saved state. Wall-clock time never feeds state. Hidden state (statics, raw pointers) must be saved and restored too.
4. **Delay first, rollback for the rest.** Make frame delay as high as the game tolerates. A fighting game about 1 frame, a puzzle game 4-5. It also shortens rollbacks. Effects made inside a rollback wait until the rollback ends (a 2-frame flash can vanish in a 3-frame rollback).

Full text: `references/ggpo-rules.md`.

## Proof

- `node tools/gh-check.mjs cargo test -p engine-net --lib session` (skill `github-check`).
- Rollback and replay hashes: same seed, same inputs, same bytes twice. A hash that moves: skill `determinism-debug`.
- Fixed-step and accumulator rules for the loop: skill `tick-and-runtime`.
- Run a sync test (roll back and replay every frame, compare checksums) while you write game code. A `DesyncDetected` style mismatch is a determinism bug, never a net bug first.

## Pitfalls

- A client position in a packet is a bug. Inputs only.
- Full state every tick is too expensive. Send inputs, snapshot on a schedule, delta against confirmed.
- A skipped or reordered request (save, load, advance) gives wrong behavior or a panic. Fulfil every request in the order given.
- `ggrs` names change between releases. Check the digest and docs.rs before you copy a call.

Sources: Gaffer On Games, "What every programmer needs to know about game networking"; Game Engine Gems 2 ch. 18 (dead reckoning, rejected); ggrs docs (MIT OR Apache-2.0) and the GGPO developer guide (MIT), credit in `references/sources.md`.
