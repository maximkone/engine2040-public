---
name: ggrs-rollback-rules
description: "Use when writing or fixing a Rust game loop that uses ggrs 0.13 rollback networking (P2P, spectator, sync test): Config, SessionBuilder, save, load and advance requests, events, sparse saving, time sync, desync detection, and the GGPO rules for determinism, input delay and rollback"
version: 0.2.0
author: skillworks
license: "Apache-2.0 (derived from gschup/ggrs @ e97e3d2 and pond3r/ggpo @ 7ddadef)"
---

# ggrs 0.13 rollback rules

Pinned to ggrs 0.13 (gschup/ggrs at commit e97e3d2, MIT OR Apache-2.0; Cargo.toml version 0.13.0) plus the GGPO developer guide (pond3r/ggpo at 7ddadef, MIT). ggrs returns a list of requests for your loop to fulfil instead of calling callbacks. Names changed between releases: use the ones below and check docs.rs for others. Each rule ends with its source file: `ggrs:` for the ggrs repo, `ggpo:` for the GGPO repo. Details: `references/ggrs-api.md` and `references/ggpo-rules.md`. Credit and licence: `references/sources.md`.

## Setup and sessions

- `ggrs = "0.13"`: the crate version comes from Cargo.toml (0.13.0, rust-version 1.87); the older version line in the setup guide is stale. Implement `ggrs::Config` with four types: `type Input` (sent every frame; it must implement `Default`, which means "no input"), `type InputPredictor = ggrs::PredictRepeatLast` (held buttons; `ggrs::PredictDefault` suits one-off events), `type State` (saved and loaded on rollback) and `type Address = std::net::SocketAddr`. [ggrs:Cargo.toml] [ggrs:docs/setup.md]
- The game must be deterministic on every peer and CPU: no float drift between x86 and ARM or debug and release (use integers or fixed point), no `HashMap` iteration order (use `BTreeMap`), and an RNG seeded from game state. ggrs does no NAT traversal; you exchange addresses first. [ggrs:docs/setup.md]
- Three sessions come from `SessionBuilder::<GgrsConfig>::new()`: `start_p2p_session(socket)`, `start_spectator_session(host_addr, socket)` and `start_synctest_session()` (with `with_check_distance(n)`). [ggrs:docs/sessions.md]
- P2P chain: `let socket = UdpNonBlockingSocket::bind_to_port(7000)?;` then `.with_num_players(2)?.with_fps(60)?.with_input_delay(2).add_player(PlayerType::Local, 0)?.add_player(PlayerType::Remote(addr), 1)?.start_p2p_session(socket)?`. `with_num_players` and `with_fps` return a `Result`, `with_input_delay` does not. Since 0.13 `with_num_players(n)?` also re-validates the players already added. Defaults: 2 players, 60 fps, delay 0, prediction window 8. [ggrs:docs/sessions.md] [ggrs:CHANGELOG.md]
- `with_max_prediction_window(0)` is conservative lockstep: no prediction and no rollback, the game stalls until remote inputs arrive, and `advance_frame()` then returns an empty request list (`session.in_lockstep_mode()` is true; keep the accumulated time and poll again). Give it enough input delay to cover polling jitter. New in 0.13: `P2PSession::advance_frame_with_wait()` and `advance_frame_with_wait_timeout()` poll for a bounded time first. [ggrs:docs/sessions.md] [ggrs:docs/main-loop.md] [ggrs:CHANGELOG.md]
- Several local players on one client are separate `PlayerType::Local` handles; call `add_local_input` once per handle before each `advance_frame`. A new P2P session is `Synchronizing` until `current_state()` is `Running`. [ggrs:docs/sessions.md]
- `P2PSession::set_input_delay(local_handle, delay)` changes a local player's delay mid-session (new in 0.13). Lowering it can drop inputs that now land before the newest queued frame; raising it repeats the last input to fill the gap. [ggrs:docs/sessions.md] [ggrs:CHANGELOG.md]

## Main loop and requests

- Every iteration: `poll_remote_clients()`, drain `events()`, read `frames_ahead()`, accumulate time, and per fixed step `add_local_input(handle, input)?` then `advance_frame()`; fulfil the requests; then render. [ggrs:docs/main-loop.md]
- `Err(GgrsError::PredictionThreshold)` (a peer is too far behind) and `Err(GgrsError::NotSynchronized)` mean skip the tick. [ggrs:docs/main-loop.md]
- `network_stats(handle)` gives `Err(GgrsError::NotEnoughData)` until its one-second window is full (`NotSynchronized`: peer not connected yet); retry later. `NetworkStats::kbps_sent` is gone in 0.13: the stats hold queue length, round-trip time and frame advantage only. [ggrs:docs/main-loop.md] [ggrs:CHANGELOG.md]
- Fulfil every request, in the order given. Skipping one or reordering them gives wrong behavior or a panic. [ggrs:docs/requests-and-events.md]
- `SaveGameState { cell, frame }`: `cell.save(frame, Some(state), Some(checksum))`. The frame is a sanity check; the checksum is optional for rollback but needed for sync test and desync detection. [ggrs:docs/requests-and-events.md]
- `LoadGameState { cell, frame }`: `cell.load()` clones the state, so it needs `State: Clone`; without `Clone` read it through `cell.data()`. [ggrs:docs/requests-and-events.md]
- `AdvanceFrame { inputs }`: `inputs` is a `Vec<(T::Input, InputStatus)>`, one entry per player in handle order. `InputStatus` is `Confirmed`, `Predicted` or `Disconnected`; a disconnected player's input is `T::Input::default()`. [ggrs:docs/requests-and-events.md]

## Events, sync and checks

- Events: `Synchronizing`, `Synchronized`, `Disconnected`, `NetworkInterrupted`, `NetworkResumed`, `WaitRecommendation { skip_frames }`, `DesyncDetected { frame, local_checksum, remote_checksum, addr }`. The queue is bounded: drain it every tick. [ggrs:docs/requests-and-events.md]
- Running ahead of a peer causes rollbacks on the peer. `WaitRecommendation` fires when you were ahead by 3 or more frames for 60 frames in a row; skipping frames stutters. `P2PSession::frames_ahead()` (signed) lets you stretch the frame time smoothly instead. [ggrs:docs/time-synchronization.md]
- `with_sparse_saving_mode(true)` saves only at the last confirmed frame: fewer saves, longer rollbacks, worth it when saving state is expensive. Request handling stays the same. `start_synctest_session()` with it on returns `GgrsError::InvalidRequest`. Since 0.13 desync detection no longer panics with it when a checksum frame was not saved exactly. [ggrs:docs/sparse-saving.md] [ggrs:CHANGELOG.md]
- `with_desync_detection_mode(DesyncDetection::On { interval: 10 })` compares checksums every `interval` frames; since 0.13 `start_p2p_session()` rejects `interval: 0`. It needs a checksum in `cell.save`. `DesyncDetected` means a determinism bug. [ggrs:docs/sessions.md] [ggrs:docs/requests-and-events.md] [ggrs:CHANGELOG.md]
- A sync test session rolls back and re-runs the last `check_distance` frames every frame and compares checksums, with no network. Run it while you write game code. [ggrs:docs/sessions.md]
- Also in 0.13: spectator catch-up is capped by the host's confirmed frames, so `with_catchup_speed(n)` may exceed `with_max_frames_behind(n)`; malformed input packets are discarded, not a panic; an all-local-player P2P session no longer leaks memory. Late spectators are Unreleased at the pin, after 0.13.0. Use one ggrs version on every peer (0.12 changed the wire format). [ggrs:CHANGELOG.md]

## GGPO rules for the game

- Only game state and game inputs matter. Each peer runs a full copy of the game and only inputs travel. Renderers, audio and effects that do not change the outcome are not state. [ggpo:doc/DeveloperGuide.md]
- The game needs a deterministic step, state that is fully encapsulated and serializable, and the ability to load, save and run one simulation frame without drawing it. Advance by a fixed time step per frame even if drawing is variable, and keep updating separate from rendering. [ggpo:doc/DeveloperGuide.md]
- Random numbers: the same seed at frame 0 for every peer, with the generator state inside the game state. Wall-clock time must not feed the state: remove it, or make one player's time part of the frame's input. Hidden state (statics, raw pointers) must be saved and restored too. [ggpo:doc/DeveloperGuide.md]
- Hide latency with frame (input) delay first and speculative execution (prediction and rollback) for the rest. Make the delay as high as the game tolerates (a fighting game 1 frame, a puzzle game 4 or 5): it also shortens rollbacks. A 2-frame flash can vanish inside a 3-frame rollback: raise the delay, or delay the effect until the rollback is over. [ggpo:doc/DeveloperGuide.md]
