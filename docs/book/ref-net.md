# Reference: net (`crates/net`) and the dedicated server

Details only. Signatures are copied from source. Units: fixed ticks at 60 Hz, millimetres,
decidegrees (`yaw_ddeg`), permille (`hp`), bytes, whole milliseconds for pacing.

Read first: `docs/HASH-LAW.md`, section "Rules for the authoritative simulation" and the netcode
line: the server is authoritative, clients predict and resimulate, and a mismatch is corrected
from the server snapshot and counted. Net owns no simulation state. Transport timing, latency
reports, beacon transit and wall clocks never enter a hash ("OUT" list in HASH-LAW).

## 0. Module map

| Module | Role |
|---|---|
| `delta` | Quantized `FighterSnap` / `FullSnap`, `encode_delta` / `decode_delta`, `Interpolator` (constant-delay remote view). |
| `snapshot` | `save_snapshot` / `load_snapshot` file codec, `ConfirmedHistory`, `TimelineSampler`. |
| `interp` | `SnapshotInterpolator` (Hermite C1 positions, quaternion yaw dispatch). Render-time only. |
| `session` | LAN wire over UDP: `Server` (authority), `Client` (prediction), packet codecs, IOQ3-style framing. |
| `dedicated` | `HeadlessServer`: the dedicated match server, slots, AI fill, snapshots, heal hooks. `Beacon` LAN discovery. |
| `netcode` | `Session<S: Sim>`: rollback with repeat-last prediction, cap 9 ticks. |
| `rollback` | `P2pSession<S: SimTick>`: two-player rollback tape, 128-frame queues, WAN cap. |
| `delay` | Input-delay helpers driven by frame advantage. |
| `quic` | `Transport` trait, `UdpTransport`, `RelayTransport`, seeded `SimQuicNet`; with feature `quic`: `QuinnEndpoint`. |
| `webtransport` | Browser transport (feature `webtransport`): `WtServer`, pinned certificate, stream framing. |
| `online` | Online played bout over QUIC through the relay (feature `quic`). |
| `courier`, `split`, `relay` | Real-UDP two-machine lockstep, STUN echo, hole punch, relay core, invite codes. |
| `lobby` | LAN lobby: build id, join, ready-up, AI-fill slots, start parameters. |
| `played` | One played bout: lobby, dedicated server, predicting clients, scripted drop and rejoin over `link::Wire`. |
| `link` | Seeded fault-injection wire: loss, jitter, reorder, outage, delay histograms. |
| `heal` | `NetHeal`: disconnect, reconnect and desync resync through `engine_observe::supervisor`. |
| `interest` | Relevancy, per-connection priority, bandwidth ledger for siege replication. |
| `scale`, `wan_soak`, `tick_budget` | 8 to 32 fighter hardening, WAN soak, siege tick budget (presentation numbers only). |
| `cells` | Tick-budgeted pre-cooked map-cell streaming (`CellQueue`, `CellPacket`, `CellReceipt`). |
| `replay`, `receipt` | Input-tape replays with keyframes and divergence attribution; JSON receipts for posted proofs. |

Binaries (`crates/net/src/bin/`): `net_bout` (plays the posted proofs: `mp6`, `on30` with feature `quic`),
`net_relay` (self-hostable relay core).

## 1. Snapshot quantization and deltas (`delta.rs`)

| Type | Fields and units |
|---|---|
| `FighterSnap` | `slot: u8`, `x_mm: i32`, `y_mm: i32`, `z_mm: i32`, `yaw_ddeg: i16`, `hp: u16` (permille), stamina. Built by `FighterSnap::quantize(slot, pos_m: [f32;3], yaw_rad: f32, hp: u16, stamina: u16) -> Result<Self, NetError>`. `slot >= MAX_PARTICIPANTS` is `NetError::BadInput`. |
| `FullSnap` | `tick: u32`, `fighters: Vec<FighterSnap>`. `new(tick, fighters) -> Result<Self, NetError>`, `validate()`, `full_wire_len()`. |
| `FighterDelta` / `Delta` | `Delta { base_tick: u32, tick: u32, changed_slots: Vec<(u8, FighterDelta)> }`. Changed fields are flagged by mask bits `MASK_X` 0x01, `MASK_Y` 0x02, `MASK_Z` 0x04, `MASK_YAW` 0x08, `MASK_HP` 0x10, `MASK_STAMINA` 0x20; `MASK_ALL` = 0x3F. |
| `encode_delta(full: &FullSnap, baseline: &FullSnap) -> Result<Delta, NetError>` | Omits unchanged slots. Identical pair gives an empty, valid delta. |
| `decode_delta(delta: &Delta, baseline: &FullSnap) -> Result<FullSnap, NetError>` | Roundtrip-exact with the full snapshot (test `delta_roundtrip_equals_full_snapshot`). |
| `Interpolator::new(delay_ms)`, `with_cap(delay_ms, cap)`; `push(time_ms, FullSnap) -> bool`; `delay_ms()`; `len()` | Constant delay. `DEFAULT_INTERP_DELAY_MS` = 100, `MAX_INTERP_SAMPLES` = 32. Clamps, never extrapolates. |
| `InterpResult` | Returned by the interpolator query; integer math only. |

Snapshot files (`snapshot.rs`): `save_snapshot(&FullSnap) -> Result<Vec<u8>, NetError>` and
`load_snapshot(&[u8]) -> Result<FullSnap, NetError>` use the session codec. Corrupt bytes refuse
(test `corrupt_bytes_refuse`). `ConfirmedHistory` holds delayed authoritative snapshots with
unchanged-marker compression. `TimelineSampler` trails the newest confirmed tick by a delay.

Example, adapted from `delta_roundtrip_equals_full_snapshot` (`crates/net/src/delta.rs`):

```rust
let delta = encode_delta(&full, &baseline).expect("encode");
assert_eq!((delta.base_tick, delta.tick), (10, 12));
assert_eq!(decode_delta(&delta, &baseline).expect("decode"), full);
```

## 2. Session wire (`session.rs`)

| Constant | Value | Meaning |
|---|---|---|
| `WIRE_MAGIC` | `0x314E` (u16) | First bytes of every session packet. |
| `WIRE_VERSION` | 1 | Mismatch is `NetError::VersionMismatch`. |
| `KIND_INPUT` 0, `KIND_STATE` 1, `KIND_SNAPSHOT` 2 | Packet kinds. |
| `STATE_DELTA_BIT` | 0x80 | Set on a state packet that carries a delta. |
| `SNAPSHOT_HEADER_LEN` 9, `SNAPSHOT_SLOT_LEN` 19, `STATE_DELTA_HEADER_LEN` 21 | Fixed header sizes. |
| `REDUNDANCY` | 3 | Inputs resent per packet (`redundancy_window(tape, end, n)`). |
| `INPUT_DELAY_TICKS` | 2 | Local input delay. |
| `MAX_INPUTS_PER_PACKET` 16; `MAX_PACKET_BYTES` 1200; `INPUT_BYTES` 10 | Budget. The transport never fragments. |
| `INPUT_COMPRESSED_BIT` | 0x80 | `encode_input_compressed(tick, &[FighterInput])` sets it. |
| `IOQ3_HEADER_LEN` 10; `IOQ3_FRAGMENT_SIZE` 1300; `IOQ3_FRAGMENT_BIT` 1 << 31; `IOQ3_OOB_SEQUENCE` `u32::MAX` | Sequence and fragment framing: `ioq3_encode_header`, `ioq3_decode_header`, `ioq3_fragment_message`, `ioq3_reassemble`. |

Codec functions: `encode_input(tick, &[FighterInput]) -> Result<Vec<u8>, NetError>`,
`decode_input(&[u8]) -> Result<InputPacket, NetError>`, `resync_packets(...)`.

`NetError` variants (partial): `BadConfig`, `BadInput`, `StaleTick(u32)`, `LateInput(u32)`,
`VersionMismatch { .. }`, `BadMagic`, `BadKind(u8)`, `Truncated`, `TooLarge`, `TooManyInputs(usize)`,
`SimRejected`.

Re-exports for genre-neutral callers: `BodyId`, `BodyInput`, `BodyTape` (aliases of the input types).
`session` re-exports `FullSnap`, `FighterSnap`, `Delta`, `Interpolator`, `InterpResult`, `DEFAULT_INTERP_DELAY_MS`,
`MAX_INTERP_SAMPLES`, `MASK_ALL` from `delta`.

## 3. Authoritative path: `session::Server` and `Client` (LAN, UDP)

Server-authoritative arena match with client prediction. Consumers: the packaged game LAN play and
`crates/net/tests/it`.

| Item | Signature and behaviour |
|---|---|
| `Server::new(ArenaConfig) -> Result<Self, NetError>` | Holds one cold `LiveMatch`. |
| `Server::tick(...)` | Steps exactly once per authored tick with received human inputs. Slots without an input are AI-filled by the arena brains. `tick_now()`, `steps()` (one step per tick), `checkpoints()`. |
| `Client::new(ArenaConfig, local: FighterId) -> Result<Self, NetError>` | Holds the predicted tape. `local()`, `tick_now()`, `checkpoints()`, `server_tick()`. |
| `Client::predict(&mut self, FighterInput) -> Result<Checkpoint, NetError>` | Local prediction for one tick. |
| `Client::on_server_state(...)` | Applies an authoritative state. Unpredicted ticks step AI-filled, as the prefix run does. |
| `Client::resimulate(&mut self) -> Vec<Checkpoint>` | Replays from the authority snapshot to the tip; one step per unacked tick, never from zero. |
| `Client::resync_from_authority(...)` | Full resync from a snapshot. |
| `last_mispredicts() -> &[u32]`, `total_mispredicts() -> u64` | Mismatched ticks are counted, never ignored. |
| `ingest_cell_bytes(&[u8]) -> Result<u64, NetError>`, `cell_have()`, `cell_count()`, `cell_digest()` | Map cell streaming on the client. |

## 4. The dedicated server: `dedicated::HeadlessServer`

One authored arena match. Connected clients submit inputs; empty or dropped slots AI-fill through the
arena brain fill; a desync resyncs from the latest `SnapshotDelta`.

| Method | Signature and behaviour |
|---|---|
| `HeadlessServer::new(ArenaConfig) -> Result<Self, ServerError>` | Rejects bad config with `ServerError::BadConfig`. |
| `connect(&mut self, token: u64) -> Result<u8, ServerError>` | Assigns a free slot (`SlotFull` when none). |
| `disconnect(&mut self, token: u64) -> bool` | Frees the slot; AI fills it next tick. |
| `submit(&mut self, token: u64, FighterInput) -> Result<(), ServerError>` | Bad input rejects loudly and changes nothing. |
| `tick(&mut self) -> Result<(Checkpoint, SnapshotDelta), ServerError>` | One authoritative step. `MatchOver` after `config.ticks`. |
| `snapshot(&self) -> Option<SnapshotDelta>`; `resync(token) -> Result<SnapshotDelta, ServerError>` | Latest delta for a client. |
| `verify(token, tick, hash) -> Result<(), ServerError>` | `Desync { tick, want, got }` when a client hash differs. |
| `report_latency(token, ms) -> Result<(), ServerError>`; `latency_ms(token) -> Option<u64>` | Telemetry only. `MAX_LATENCY_MS` 10 000. |
| `theater_tape(&self) -> Result<ReplayTape, ServerError>`; `theater(&self) -> Result<Theater, ServerError>`; `divergence_report(...)` | Replay and attribution. |
| `cell_queue()`, `cell_queue_mut()`, `cell_packets_last_tick()`, `cell_receipts_last_tick()`, `cell_bytes_last_tick()` | Cell streaming, budgeted per tick. |
| `heal()`, `heal_notices()`, `is_slot_quarantined(slot)`, `disconnect_via_supervisor(token) -> Result<ReconnectPlan, NetHealError>`, `reclaim_slot(token) -> Result<u8, NetHealError>`, `resync_via_supervisor(token) -> Result<SnapshotDelta, NetHealError>`, `heal_reset_slot(slot) -> bool`, `apply_heal_boundary(boundary_tick: u64) -> Vec<AppliedHeal>` | Heal path (section 7). |
| `connected_count()`, `ai_filled_last_tick()`, `slot_of(token)`, `is_connected(token)`, `is_finished()` | Status. |

`ServerError` variants: `BadConfig`, `BadInput`, `SlotFull`, `StaleClient { token }`, `Desync { tick, want, got }`,
`NoSnapshot`, `MatchOver`, `NotFinished`. Display text is fixed per variant.

LAN discovery: `Beacon` (`BEACON_MAGIC` 0x504D, `BEACON_VERSION` 1, `BEACON_LEN` 18). `Beacon::encode(self) -> Result<[u8; 18], ServerError>`,
`decode(&[u8])`, `validate()`, `announce(&UdpSocket, to: SocketAddr) -> io::Result<usize>`,
`listen_once(&UdpSocket) -> io::Result<Option<(Beacon, SocketAddr)>>`. `ticks_to_ms(u32) -> u64`, `ms_to_ticks(u64) -> u64`.

Example, adapted from `loopback_drop_rejoin_converges` (`crates/net/src/dedicated.rs`), which drops and rejoins a client
and checks convergence:

```rust
let mut server = HeadlessServer::new(config(2))?;
let home = server.connect(0xA11CE)?;          // slot 0
assert_eq!(home, 0);
let (checkpoint, delta) = server.tick()?;      // slot 1 is AI-filled
```

Consumers of `HeadlessServer` in this repo: `crates/game/src/siege.rs`, `crates/net/src/played.rs`, `heal.rs`, `cells.rs`.
No standalone dedicated binary exists in `crates/net`; the server is a library type.

## 5. Rollback and lockstep as implemented

Three mechanisms exist. They do not share one path. Pick the one the caller already uses.

| Mechanism | Where | What it does | Bounds and tests |
|---|---|---|---|
| Server authority with client resimulation | `session::Client`, `dedicated::HeadlessServer` | Client predicts its own slot; server truth corrects it; the client resimulates from a snapshot. | Counted mispredicts. `checkpoint` comparison via `verify`. Test: `loopback_session_converges_under_latency_and_loss`, `mispredict_tick_named_on_divergence` (session.rs). |
| `netcode::Session<S: Sim>` | `netcode.rs` | Save or load snapshots, repeat-last prediction for the remote, confirm when real input arrives, resimulate from the earliest wrong frame. | `MAX_ROLLBACK` 9; `HISTORY_CAP` 10. `Sim` trait: `step(&mut self, [u32;2])`, `hash(&self) -> u64`. `advance(local, remote: Option<u32>) -> u64`; `confirm(frame, input) -> Result<usize, NetcodeError>`; `check_remote(frame, peer) -> Result<(), NetcodeError>`. Tests: `loss_10pct_converges`, `rollback_bounded_9`, `wan_61_tick_rollback_refused_against_9_tick_cap`. |
| `rollback::P2pSession<S: SimTick>` | `rollback.rs` | Two-player rollback tape. `SimTick::advance([u32;2])`, `state_hash()`. Save and load game state, frame log, `check_desync(frame, peer_hash)`. | `MAX_P2P_ROLLBACK` 128; `INPUT_QUEUE_LEN` 128; `PLAYERS` 2; `WAN_ROLLBACK_CAP` 9 for `confirm_frame_wan`. `advance_frame(local, remote) -> u64`; `confirm_frame(frame, remote) -> Result<usize, RollbackError>`. Tests: `tape_identical_x2`, `cap_respected_at_128`, `fold_deterministic_and_sensitive`. |
| Lockstep slot exchange | `split.rs` (private `lockstep`), `courier.rs` | Two real machines play one bout in lockstep: each step, the frames due on crossing links are traded through `Courier`, with resends. No rollback. | Real UDP over LAN or via the relay. Probes report delay. Test: the `net_bout mp6` receipts. |
| Input delay | `delay.rs` | `clamp_delay`, `delay_for_advantage` step the delay by at most one unit per call from `frame advantage = local - remote`. `DEFAULT_DELAY`, `MIN_DELAY..=MAX_DELAY`. | Integer only. |

Lockstep and rollback sit beside the arena authority. Arena checkpoints (`Checkpoint { tick, hash }`) remain
the truth; the mechanisms above only decide when and how a client converges to it.

Played bout (`played.rs`): a lobby forms two humans plus AI fill; the host runs `HeadlessServer`; clients
predict; a scripted drop and rejoin runs over `link::Wire`. `HostLedger` records the host's accounting.

## 6. Transports

### 6.1 `quic::Transport` (always built)

```rust
pub trait Transport {
    fn send_unreliable(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError>;
    fn send_reliable(&mut self, to: PeerId, kind: ControlKind, bytes: &[u8]) -> Result<(), NetError>;
    fn recv(&mut self) -> Vec<Inbound>;
}
```

Over the budget (1200 bytes) fails before touching a socket (`check_budget`). Over raw UDP a reliable
message rides a datagram (documented downgrade); over QUIC it rides a stream. `PeerId(pub SocketAddr)`,
`ControlKind`, `ChannelDelivery`, `channel_config(Channel) -> ChannelConfig`, `channel_is_reliable`, `channel_priority`.
Helpers: `send_input`, `send_state`, `send_state_delta`, `send_snapshot`, `send_lobby` (generic over `Transport`).

| Type | Behaviour |
|---|---|
| `UdpTransport::new(UdpSocket) -> io::Result<Self>`; `bind_loopback()`; `local_addr()` | Plain UDP, the headless default. |
| `RelayTransport::new(UdpSocket, RelayLeg) -> io::Result<Self>`; `bind_loopback(leg)`; `send_register()`; `registered()`; `denied()`; `srflx()` | Relay-wrapped UDP with STUN binding. `wrap` and `unwrap` frame relay messages (`MAX_RELAY_BYTES`). |
| `SimQuicNet`, `SimQuicFabric::new(latency: u32, seed: u64)`, `SimTransport` | Seeded, tick-ordered simulation of QUIC delivery for tests. Sockets never enter sim hashes. `dropped()`, `delivered()`, `bytes_sent(peer)`. |

### 6.2 QUIC (feature `quic`, dependency `quinn` 0.11)

`QuinnEndpoint::bind(addr: SocketAddr) -> io::Result<Self>` (client-only, ephemeral port), `quinn_transport()`,
`client_crypto() -> Result<quinn::ClientConfig, rustls::Error>`, `server_crypto(...)`, `StdRuntime`.
`online::Path`, `online::resolve_path(nats) -> PunchOutcome`, `online::OnlineWire::open(path, hop_latency, seed) -> Result<Self, BoutError>`
drive the online bout through the relay (`net_bout on30`). Tests run with `--features quic`.

### 6.3 WebTransport (feature `webtransport`, implies `quic`; `wtransport` 0.7, `tokio`)

| Item | Behaviour |
|---|---|
| `webtransport::WtServer::bind(addr: SocketAddr, cert: &PinnedCert) -> io::Result<Self>`; `local_addr()`; `peers() -> Vec<PeerId>`; `take_departures() -> Vec<Departure>`; `dropped_datagrams() -> u64` | One dedicated thread runs a single-threaded tokio runtime; the tick loop never awaits. Unreliable frames ride datagrams into a bounded inbox (full inbox drops). Reliable frames ride one bidirectional stream per session. |
| `SESSION_PATH` = `"/e2040"`; `MAX_SESSIONS` = 16; `RELIABLE_FRAMES` = `CHANNEL_MEMORY_BYTES / MAX_PACKET_BYTES` | |
| `DepartReason`, `Departure` | Why a session left. |
| `cert::PinnedCert::mint(subject_alt_names, valid_days: u32) -> Result<Self, CertError>`; `hash() -> [u8;32]`; `hash_hex()`; `leaf_der()`; `validity()`; `cert_pem()`; `key_pem()`; `leaf_hash(der)`; `check_pinnable(der, now_unix)` | `MAX_VALIDITY_DAYS` = 14. Browser pin is SHA-256 of the leaf DER. Every run mints a fresh certificate. |
| `frame::encode_message(ControlKind, payload) -> Result<Vec<u8>, NetError>`; `decode_header([u8;3]) -> Result<(ControlKind, usize), NetError>` | `FRAME_HEADER_LEN` 3: kind u8, length u16 little-endian. Payload is the codec's exact bytes. |

Tests: `webtransport_plays_a_600_tick_snapshot_duel_with_a_pinned_client`,
`webtransport_refuses_a_client_pinned_to_the_wrong_hash`, `webtransport_answers_the_wrong_path_with_a_rejection`.

## 7. Heal, interest, scale, cells, replays

| Module | Key items |
|---|---|
| `heal.rs` | `NetHeal` owns the `engine_observe::supervisor::Supervisor`, a ring buffer, per-slot quarantine and operator notices. `QuarantineNotice`, `ReconnectPlan`, `AppliedHeal`, `NetHealError`. Over-budget faults quarantine the triggering slot loudly. Backoff is tick-derived, not clock-derived. |
| `interest.rs` | Per-connection relevancy by distance and view, rate LOD for far entities, bandwidth ledger (u128 headroom). Snapshot slots are named through the `BodySnap` alias. |
| `scale.rs` | 8 to 32 fighter bandwidth caps, priority deltas and soak. Loopback only. |
| `cells.rs` | `CellQueue`, `CellPacket`, `CellReceipt`: pre-cooked map cells streamed under a per-tick byte budget; hashed with `engine-core` FNV. |
| `tick_budget.rs` | Siege server tick budget; `author_ai(seed, tick, index)` and `author_ai_bytes`. Wall-clock ms are presentation only. |
| `replay.rs` | Records input tapes with keyframes, frame-steps them (`Theater`), reports the first divergence (`divergence_report`). |
| `receipt.rs` | JSON form of the played proofs' numbers, read by `tools/mp6-check.mjs`. |
| `link.rs` | `FaultLink` and `Wire`: seeded loss, jitter, reorder, scripted outage. Delay histograms. Used by `played` and `split`. |
| `lobby.rs` | `BuildId::{validate, check_against}`; `Lobby::new(BuildId, fighters: u8, seed: u64)`, `join(token, BuildId) -> Result<u8, LobbyError>`, `leave(token) -> bool`, `set_ready(token, bool)`, `slot_of(token)`; `StartParams`; `encode`/`decode`. |
| `courier.rs`, `relay.rs`, `online.rs` | UDP lockstep courier with resends; STUN echo; hole punch; invite-code relay; online path choice. |

## 8. Genre ledger (GE2)

`tools/workspace-check.mjs` holds the `genreLedger` set (line 564 onward). Engine crates below
`arena`/`game` may name the fighter and gladiator types only in the files listed there. A new file
with those names fails the gate. New engine code names snapshot types through the neutral alias:
`net::delta::BodySnap` (`pub type BodySnap = FighterSnap;`), or the `BodyId`, `BodyInput`, `BodyTape`
re-exports. The ledger never grows; moves shrink it.

The array holds 57 quoted entries at the time of writing. The comment above it says "verified 2026-09-25,
50 files"; the two numbers disagree and should be reconciled by the owner.

## 9. Tests and commands

| Scope | Command |
|---|---|
| Unit tests (std-only default) | `node tools/gh-check.mjs cargo test -p engine-net --lib` |
| Integration tests | `node tools/gh-check.mjs cargo test -p engine-net --test it` |
| QUIC leg | `node tools/gh-check.mjs cargo test -p engine-net --features quic --lib` |
| WebTransport leg | `node tools/gh-check.mjs cargo test -p engine-net --features webtransport --lib` |
| Posted proofs | `net-bout mp6` and `net-bout on30` (binary `net_bout`); receipts read by `tools/mp6-check.mjs`. |

Named tests: `loopback_drop_rejoin_converges`, `sixteen_slot_drop_rejoin_converges`
(`dedicated.rs`), `ioq3_delta_beats_full_by_thirty_percent` (`delta.rs`), `file_roundtrip_equals`
(`snapshot.rs`), `webtransport_plays_a_600_tick_snapshot_duel_with_a_pinned_client` (`webtransport/tests.rs`).

## Where the code is

- `crates/net/src/lib.rs`: module list, re-exports of `dedicated` and `session` types.
- `crates/net/src/delta.rs`: `FighterSnap`, `FullSnap`, `Delta`, `encode_delta`, `decode_delta`, `Interpolator`, `BodySnap`.
- `crates/net/src/snapshot.rs`: snapshot file codec, `ConfirmedHistory`, `TimelineSampler`.
- `crates/net/src/interp.rs`: `SnapshotInterpolator`, render-time interpolation.
- `crates/net/src/session.rs`: wire constants, packet codecs, IOQ3 framing, `Server`, `Client`, `NetError`.
- `crates/net/src/dedicated.rs`: `HeadlessServer`, `Beacon`, `ServerError`.
- `crates/net/src/netcode.rs`: `Session<S: Sim>`, `MAX_ROLLBACK`, `Packet`, `Channel`.
- `crates/net/src/rollback.rs`: `P2pSession<S: SimTick>`, `InputQueue`, `FrameLog`.
- `crates/net/src/delay.rs`: input-delay helpers.
- `crates/net/src/quic.rs`: `Transport`, `UdpTransport`, `RelayTransport`, `SimQuicNet`, `QuinnEndpoint` (feature `quic`).
- `crates/net/src/webtransport.rs`, `webtransport/server.rs`, `cert.rs`, `frame.rs`, `tests.rs`: browser transport (feature `webtransport`).
- `crates/net/src/online.rs`: online bout over QUIC (feature `quic`).
- `crates/net/src/courier.rs`, `split.rs`, `relay.rs`: lockstep courier, two-machine split bout, relay core.
- `crates/net/src/lobby.rs`, `played.rs`, `link.rs`: lobby, played bout, fault-injection wire.
- `crates/net/src/heal.rs`: `NetHeal`, reconnect and desync resync.
- `crates/net/src/interest.rs`, `scale.rs`, `wan_soak.rs`, `tick_budget.rs`, `cells.rs`: relevancy, hardening, soak, budgets, map cells.
- `crates/net/src/replay.rs`, `receipt.rs`: replays and posted receipts.
- `crates/net/src/bin/net_bout.rs`, `bin/net_relay.rs`: proof runner, relay binary.
- `crates/net/tests/it/`: integration tests (`rollback.rs`, `relay.rs`, `quic_parity.rs`, `wan_fault_matrix.rs`, and others).
- `crates/net/testdata/`: input queue vectors, WebTransport fixtures.
- `tools/workspace-check.mjs` (line 564 onward): the GE2 genre ledger.
- `docs/HASH-LAW.md`: determinism scope and netcode rules.
