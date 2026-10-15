# ggrs 0.13 details

Condensed from gschup/ggrs at commit e97e3d2 (MIT OR Apache-2.0), reworded, short code kept. Each section names its source file in the ggrs repo (`ggrs:`).

## Setup (ggrs:docs/setup.md, ggrs:Cargo.toml)

- `ggrs = "0.13"` in `Cargo.toml`. The crate version is the manifest's (`version = "0.13.0"`, `rust-version = "1.87"`); the setup guide's own version line is older than the crate and is not the version to pin. Feature `sync-send` adds `Send + Sync` bounds to `Config`, `NonBlockingSocket` and related types; feature `wasm-bindgen` enables wasm32 support (for WebRTC networking in a browser use the Matchbox sockets, which also do the signalling).
- ggrs assumes you already know the socket address of every client. It does no NAT traversal and has no signalling server.
- `Input` must implement `Default`: the default value stands for "no input" for a disconnected player. A typical input is a `Copy` struct of bools with `Serialize` and `Deserialize`.
- `InputPredictor` guesses a remote input that has not arrived. `PredictRepeatLast` repeats the last known input (good for held buttons). `PredictDefault` always predicts `Input::default()` (good when an input is a one-off "pressed this frame" event). You can implement the `InputPredictor` trait yourself.

```rust
impl ggrs::Config for GgrsConfig {
    type Input = Input;                              // sent over the network each frame
    type InputPredictor = ggrs::PredictRepeatLast;   // how a missing remote input is predicted
    type State = GameState;                          // saved and loaded during rollbacks
    type Address = std::net::SocketAddr;
}
```

## Builder options and lockstep (ggrs:docs/sessions.md, ggrs:docs/main-loop.md, ggrs:CHANGELOG.md)

| Method | Default | Meaning |
|---|---|---|
| `with_num_players(n)?` | 2 | players, spectators not counted, at least 1; re-validates players already added (0.13) |
| `with_fps(fps)?` | 60 | expected update rate, used for frame sync |
| `with_input_delay(n)` | 0 | frames of delay on local input; fewer rollbacks; in lockstep it schedules local input ahead |
| `with_max_prediction_window(n)` | 8 | frames ggrs predicts ahead; 0 = lockstep (no prediction, no rollback) |
| `with_sparse_saving_mode(bool)` | false | save only at the last confirmed frame |
| `with_desync_detection_mode(mode)` | Off | `DesyncDetection::On { interval }`, interval above 0 (`start_p2p_session()` rejects 0 since 0.13) |
| `with_disconnect_timeout(d)` | 2 s | silence before a peer is disconnected |
| `with_disconnect_notify_delay(d)` | 500 ms | silence before `NetworkInterrupted` |
| `with_max_frames_behind(n)` | 10 | spectator catch-up threshold |
| `with_catchup_speed(n)` | 1 | spectator frames per `advance_frame()` while catching up, at least 1; may exceed `with_max_frames_behind` since 0.13 |
| `with_check_distance(n)` | | sync test only: frames rolled back each frame |

- Sessions: `P2PSession` (every client builds one, peer to peer), `SpectatorSession` (follows a host, sends no input), `SyncTestSession` (local determinism check). The builder is consumed by the `start_*` call.
- `set_input_delay(local_handle, delay)` changes the delay of a running P2P session (0.13; also with several local players on one endpoint). Lowering it can drop inputs that now land before the newest queued frame; raising it repeats the last input to fill the gap.
- Lockstep (`with_max_prediction_window(0)`): `advance_frame()` can return an empty request list while inputs are missing; check `in_lockstep_mode()`, keep the accumulated time and retry after more polling. It stays conservative, so give it enough input delay to cover polling and scheduling jitter. Since 0.13 `P2PSession::advance_frame_with_wait()` and `advance_frame_with_wait_timeout()` poll for a bounded time before they return the empty list, which saves a hand-written busy loop, and `current_frame()` stays aligned with the game frames emitted. Save and load requests are never sent in lockstep mode.

## The loop (ggrs:docs/main-loop.md, ggrs:CHANGELOG.md)

```rust
session.poll_remote_clients();
for event in session.events() { /* WaitRecommendation { skip_frames } => frames_to_skip += skip_frames, ... */ }
time_since_last_frame += last_tick.elapsed();
last_tick = Instant::now();
while time_since_last_frame >= fps_delta {
    session.add_local_input(local_handle, current_input)?;       // once per local handle
    match session.advance_frame() {
        Ok(requests) => {
            if requests.is_empty() && session.in_lockstep_mode() { break; } // lockstep stall
            time_since_last_frame -= fps_delta;
            for request in requests { handle_ggrs_request(request); }
        }
        Err(GgrsError::PredictionThreshold) => { time_since_last_frame -= fps_delta; }
        Err(e) => return Err(e),
    }
}
```

- `poll_remote_clients()` runs every iteration, whether or not a frame is advanced.
- While `Synchronizing`, `advance_frame()` returns `Err(GgrsError::NotSynchronized)`; check `current_state()` or treat it as a skip.
- `network_stats(handle)` uses a one-second window and returns `Err(GgrsError::NotEnoughData)` until it is full, even in `Running`; that is different from `NotSynchronized` (not connected yet). `NetworkStats::kbps_sent` was removed in 0.13: the stats report queue length, round-trip time and frame advantage only. The docs sample still has a comment about bandwidth; there is no such field.

## Requests (ggrs:docs/requests-and-events.md)

```rust
GgrsRequest::SaveGameState { cell, frame } => {
    assert_eq!(game.current_frame(), frame);   // sanity check
    cell.save(frame, Some(state), Some(checksum));
}
GgrsRequest::LoadGameState { cell, frame } => {
    let state = cell.load().expect("no game state found");   // needs State: Clone; otherwise cell.data()
    game.restore_state(state);
}
GgrsRequest::AdvanceFrame { inputs } => {
    for (handle, (input, status)) in inputs.iter().enumerate() { /* Confirmed, Predicted, Disconnected */ }
    game.step();
}
```

- By default ggrs asks for a save before every advance (a prediction window of 8 keeps 9 save slots). `Predicted` inputs come from your `InputPredictor` and may be rolled back; `Disconnected` inputs are always `Default`.
- Pass `None` as the checksum and the state still saves and loads, but sync test and desync detection have nothing to compare.

## Events and time sync (ggrs:docs/requests-and-events.md, ggrs:docs/time-synchronization.md)

| Event | Meaning |
|---|---|
| `Synchronizing { addr, total, count }` | `count` of `total` sync round trips done |
| `Synchronized { addr }` | peer ready |
| `Disconnected { addr }` | peer timed out or left |
| `NetworkInterrupted { addr, disconnect_timeout }` | silence; disconnect pending in that many ms |
| `NetworkResumed { addr }` | traffic is back |
| `WaitRecommendation { skip_frames }` | you are ahead; skip that many frames |
| `DesyncDetected { frame, local_checksum, remote_checksum, addr }` | checksums differ at `frame`: a determinism bug |

- `WaitRecommendation` needs you ahead by 3 or more frames for 60 consecutive frames. `frames_ahead()` is positive when ahead and negative when behind; one smooth way to use it is to scale the frame time by `1.0 + 0.1 * frames_ahead.min(3)`. The event suits slow or turn-based games; `frames_ahead()` suits fast ones.

## Sparse saving (ggrs:docs/sparse-saving.md, ggrs:CHANGELOG.md)

Off by default (a save every frame). On, ggrs saves only at the last confirmed frame, the newest frame whose inputs are all real: at most one save per tick, but a misprediction re-simulates from further back. Use it when a state save is expensive; measure both ways. Request handling does not change. It cannot be combined with a sync test session, which needs a checksum for every frame of its check window; `start_synctest_session()` then returns `GgrsError::InvalidRequest`. Since 0.13 desync detection no longer panics with sparse saving when the checksum interval frame was not saved exactly.

## What changed, newest first (ggrs:CHANGELOG.md)

| Release | Change | What to do |
|---|---|---|
| after 0.13.0 | listed as Unreleased at the pin: late spectators can follow a host they are far behind (within the buffer limits); a rollback on a caught-up player's disconnect once the spectator has caught up | not in 0.13.0: do not rely on them with `ggrs = "0.13"` |
| 0.13.0, breaking | `NetworkStats::kbps_sent` removed | drop any bandwidth display built on it |
| 0.13.0 | lockstep keeps `current_frame()` aligned with emitted game frames; `advance_frame_with_wait()` and `advance_frame_with_wait_timeout()` added | in lockstep use them for the bounded wait |
| 0.13.0 | `set_input_delay()` works mid-session; the freeze after raising the delay (gap-fill inputs) and the wrong rejection of a delayed peer's first packet are fixed; delay changes work with several local players on one endpoint | no workaround needed |
| 0.13.0 | `with_num_players()` re-validates added handles; `start_p2p_session()` rejects `DesyncDetection::On { interval: 0 }` | set the player count before adding players; use an interval of at least 1 |
| 0.13.0 | desync detection no longer panics with sparse saving; malformed input packets are discarded with warnings; spectator catch-up is capped by the host's confirmed frames and by the buffer size; an all-local-player session no longer leaks memory | none |
| 0.12.0, breaking | the input wire format changed: clients on different ggrs versions cannot talk | one version on every peer |
| 0.12.0, breaking | `network_stats()` returns the new `GgrsError::NotEnoughData` in the first second of a connection (`NotSynchronized` now means only not yet connected) | handle both as non-fatal |
| 0.12.0, breaking | `start_synctest_session()` returns `GgrsError::InvalidRequest` with sparse saving on; `with_num_players()` returns a `Result` and rejects 0 | add the `?`; do not mix sparse saving with a sync test |
| 0.12.0, breaking | `Config` needs `type InputPredictor` | add `type InputPredictor = ggrs::PredictRepeatLast;` |
| 0.11.0, breaking | `Config::Input` needs `Default` plus serde `Serialize` and `DeserializeOwned` (it was bytemuck `Pod` and `Zeroable`); lockstep by `max_prediction` 0 and non-`Clone` states arrived | derive `Default`, `Serialize`, `Deserialize` on the input |
