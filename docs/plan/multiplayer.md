# Multiplayer

Pre-merge text: `git show a0e13976:docs/plan/multiplayer.md`. The internet half (`docs/archive/2026-10-08/plan/online.md`, prefix ON) is merged in below; its file is removed from `docs/plan/` (identical copy in the internet-play part below). Pre-merge text: `git show a0e13976:docs/plan/online.md`.

Goal: the arena duel played by 2 or more people on LAN and over the internet, server-authoritative with rollback, one hash.

Acceptance:
1. `cargo test -p engine-net --lib snapshot` green (MP17 wiring, MP10 interpolation); golden `d57b5c8ae3096614` unchanged, x2.
2. MP6: 2 humans + 6 AI on real hardware converge through a drop and rejoin; loss and latency distributions posted.
3. ON30: the same bout over QUIC + relay with a real NAT leg; the 64-slot soak at 150 ms / 30 ms / 2% converges in 300 ticks or less.

Order: MP6, MP10, MP17, ON30 (Now, in that order), then the table below in ID order.

Owner crates: `net` (`crates/net`), `arena`, `game`, `proof`. Prefixes: MP (LAN), ON (internet).

Open board IDs: MP6 MP8 MP10 MP12 MP14 MP15 MP16 MP16a MP16b MP17 MP18 MP19 MP21 MP22 ON6 ON7 ON8 ON12 ON13 ON14 ON15 ON16 ON17 ON18 ON19 ON20 ON21 ON22 ON23 ON24 ON25 ON26 ON27 ON28 ON29 ON30 ON31 ON36

## 1. Where it stands

| Capability | Status |
| --- | --- |
| UDP LAN + prediction + resim ≤8 (NET1) | unit-tested, integrated, ticked |
| Snapshot deltas + interpolation (NET2) | unit-tested, integrated, ticked |
| Dedicated server + discovery + reconnect (MP1) | unit-tested, integrated, ticked |
| 8–16 hardening + priority deltas (MP2) | unit-tested, integrated, ticked |
| Lobby browser + ready-up + AI-fill (MP3) | unit-tested, integrated, ticked |
| Replays + theater + keyframes (MP4) | unit-tested, integrated, ticked |
| Lag window on the dedicated server (MP5, QN12) | `lagcomp.rs` restored 2026-10-18 as the input-arrival window (per-slot, sized from measured lag or reported round trip, capped at 16 ticks); `HeadlessServer::hold_ticks` holds ticks open, a `HitReceipt` per attack press; caller `played.rs`. View rewind for snapshot clients is not built. Status 2026-10-18 below |
| Disconnect/desync heal (AH4) | unit-tested, integrated, ticked |
| Canonical caps (GE3, GE7) | unit-tested, integrated, ticked |
| Loopback LAN with AI fill (GM2) | unit-tested, integrated, ticked |
| Internet seam (ON1–ON4, ON9–ON11) | unit-tested, integrated, ticked, owned by the internet-play part below |
| Input sanitize (MP13) | `anticheat.rs` restored 2026-10-18 as the integer sanitize rule (spear bit masked in a match without the spear, `F_SPEAR`, a bounded log naming each tick); the float speed/teleport, Merkle and killcam code stays deleted |
| Server-authority admission (`guard.rs`) | restored 2026-10-18, called from `HeadlessServer::offer` (caller `played.rs`): malformed, bad-field, wrong-slot, future, stale, late, rewrite and rate refusals counted per slot in a `SlotReceipt`. `webduel.rs` (browser duel host) does not route through it yet |
| Quantized interest grid | absent as a file, owned elsewhere |
| Steam relay transport | `steam_relay.rs` was removed in `3151bd46` (it had no production caller); Status 2026-10-17 below |
| Snapshot interpolation file | missing (rowed) |
| Party / networked physics / voice net | missing (rowed, other plan) |
| Channel hints | hint only, not a table |
| Input compression | missing (rowed) |
| Lobby screen consumer (UIX3) | missing (rowed, other lane) |
| WAN-shaped code in MP files | absent by gate |

Module notes that constrain the rows below:

- The MP/ON split is final: MP owns the LAN loop (session, delta,
  scale, lobby, replay, lagcomp, heal, rollback, snapshot once MP10
  lands); ON owns everything past the router (quinn transport, relay,
  steam relay, interest, WAN soak, cheat authority, party, physics
  sync, voice). A row that touches both files is two rows, never one.
  MP21 reads `interest.rs` through `ConnectionInterest::select` and
  never writes it; `guard.rs` verdicts are read the same way.
- 1200 bytes wins everywhere: `MAX_PACKET_BYTES ==
  MAX_DATAGRAM_BYTES == 1200` on every encode path (session, scale,
  quic guard). Over-budget returns `OverBudget`, never fragments.
  The old 1400-byte line in the previous revision of this plan is
  dead; no row text carries it.
- `session.rs` keeps its NET1 meaning and `lobby.rs` its MP3 meaning;
  version bumps refuse loudly (`WIRE_VERSION`, `LOBBY_VERSION`,
  `RELAY_WIRE_VERSION` each refuse unknown with the digest pair).
- ms timing never enters match seed or state: browser refresh,
  beacon expiry and ping-to-tick conversion stay pacing-only; slot
  order plus ready transitions are pure input order.

### Internet play (docs/archive/2026-10-08/plan/online.md section 1)

Newcomer first: the engine plays LAN fights today with prediction, deltas and
a headless server; QUIC, relay, interest and the WAN soak now exist as code,
while cheat authority, party product, networked physics and voice transport
are still missing.

| Capability | Status |
| --- | --- |
| UDP LAN + prediction + resim ≤8 ticks (NET1) | unit-tested, integrated |
| Snapshot deltas + interpolation (NET2) | unit-tested, integrated |
| Dedicated server + beacon + slot reclaim + AI fill (MP1) | unit-tested, integrated |
| Lobby browser + ready-up + AI-fill (MP3) | unit-tested, integrated |
| Lag window, bounded (MP5, QN12) | `lagcomp.rs` restored 2026-10-18 (input-arrival window, cap 16 ticks); Status 2026-10-18 below |
| 8–16 hardening, priority deltas, kbps caps (MP2) | unit-tested, integrated |
| Disconnect/desync heal (AH4) | unit-tested, integrated |
| QUIC transport (ON1) | unit-tested, integrated |
| Relay + STUN/ICE-lite + invite codes (ON2) | unit-tested, integrated |
| Interest + priority replication 64+500AI (ON3) | unit-tested, library-side |
| WAN fault soak 150ms/30ms/2% at 64 (ON4) | unit-tested, integrated |
| Siege interest proof (ON9) | unit-tested, integrated |
| Siege tick budget (ON10) | unit-tested, integrated |
| Hour-long online soak (ON11) | unit-tested, integrated |
| Quantized interest grid | implemented in `interest.rs`, no standalone `aoi.rs` |
| Local cheat guards (sanitize + speed/teleport) | sanitize restored 2026-10-18 in `anticheat.rs` on the dedicated admission path; speed/teleport checks are not rebuilt (the server authors movement from inputs, so a client cannot claim a position) |
| Channel hints (reliable/unreliable/sequenced) | implemented as hint only |
| Server-authority cheat checks + tapes (ON5) | `guard.rs` restored 2026-10-18 on `HeadlessServer::offer`; refusals counted per slot and folded into the bout digest; Status 2026-10-18 below |
| Steam overlay relay fabric | modules cut 2026-10-16, never wired; the feature is not on any live path |
| Netcode P2P history | implemented, unbounded history |
| Party + server list + matchmaking-lite (ON6) | missing (board owns `session.rs` text) |
| Networked physics, debris/ragdoll/projectiles (ON7) | missing |
| Proximity voice over Opus (ON8/ON12) | missing |

Module notes that constrain the rows below:

- Authority never moves: the server runs the same `arena` code as clients; AI
  slots are ordinary fighters with broadcast inputs (carried from this plan).
- `engine-core` stays std-only; QUIC/Opus deps live in `net`/`audio` behind
  features, never in the fixed tick. Net IO stays off-tick; the sim consumes
  validated inputs at tick boundaries only.
- Snapshot cap reconciliation (from the pre-merge multiplayer §5): 1200 B wins everywhere;
  the old 1400 B line is dead. Every encode path asserts `<= 1200`, never fragments.
- `crates/net/src/session.rs` the file keeps its NET1 meaning; the ON6 product
  (party/list/lite) lands in new `party.rs`, never by renaming `session.rs`.
- `interest.rs` production wiring is deferred by its own header: the in-test
  consumer is real encode/decode, but `session.rs`/`scale.rs` do not call
  `ConnectionInterest::select` yet (grep 2026-10-01: zero hits in either
  file); ON29 closes exactly that seam.
- `guard.rs` and `anticheat.rs` restored 2026-10-18 with a production
  caller (`HeadlessServer::offer`, driven by `played.rs`); see Status
  2026-10-18.
- `steam_relay.rs` cut 2026-10-16, never wired; the feature is not on any
  live path.

Status 2026-10-17 (net code removed on purpose): rot packet `0-rot-net-dead-r1` removed eight modules from `crates/net`: `lagcomp.rs` (MP5), `guard.rs` and `anticheat.rs` (ON5), `replicate.rs` (RC14), `server.rs` (`SessionServer`), `steam_relay.rs` (PLF3), `nat.rs` and `resim.rs`. It also removed eight test files and the tests-only siege and human-bout runners in `interest.rs` and `scale.rs`. All of it sits in one `auto: backup` commit, `3151bd46` (27 files, 8,306 lines deleted). It was a dead-code cut, not a sweep accident and not a decision about the features: per the packet, their only users were each other and tests. The packet closed DONE with `cargo test -p engine-net` lib 237 passed. MP5, RC14, ON5 and PLF3 are still ticked on the board, so those ticks have no code behind them. Recover with `git show 3151bd46^:crates/net/src/<file>`. Update 2026-10-18: `guard.rs`, `anticheat.rs` (integer port) and `lagcomp.rs` are back with a production caller (Status 2026-10-18); `replicate.rs`, `server.rs`, `steam_relay.rs`, `nat.rs` and `resim.rs` stay deleted (nothing calls them), so RC14 and PLF3 still have no code.

Status 2026-10-17 (what is stale in this file), updated 2026-10-18: still stale are the Steam relay table cell, the "Ownership" entry for `server.rs`, the CUT line that says `steam_relay.rs` stays, and the Quality gates line "`steam_relay.rs`, the third WAN home" (that file is deleted). The lag-window and cheat-guard cells, the `LagWindow`/`HitReceipt` sketch, the Freeze rule, hard problem 5, the Steals note and the "Lag-window honesty" gate now say what the restored code does. Open rows that name removed code: MP12 (needs RC14), BX9 (`server.rs`); UW25 names `lagcomp.rs`, which is back, but its ranged view rewind is not built. ON9 keeps its cap constants in `interest.rs`, but the runner that charted 64 clients and 500 AI against them is gone (`siege_kbps_chart` has 0 grep hits), so that chart cannot be re-run from the tree.

Status 2026-10-17 (board vs the Now list): the board has MP6, MP10, MP17 and MP24 ticked; ON30 is open; MP23 is not on the board. MP6 is ticked with a loopback Accept (fault injector, 2 humans + 6 AI; `net_bout mp6` PASS and `node tools/mp6-check.mjs` 24 pass). The Acceptance 2 line and Tier A above say real hardware; that run has not happened. Checked on the GitHub runner today: `cargo test -p engine-net --lib` 237 passed, 0 failed, 2 ignored (run 39025608102); `--lib snapshot` 38 passed (run 39024796869); `scale::` 16, `played::` 11, `replay::` 11, `lobby::` 11 (loopback only); `heal` 5 (local run).

Status 2026-10-17 (MP5 lag compensation, owner call), resolved 2026-10-18: choice (a) was taken; `lagcomp.rs` is restored with a real caller (Status 2026-10-18).

Status 2026-10-18 (server authority restored): client input datagrams now enter the dedicated server through `HeadlessServer::offer(token, bytes, arrival_tick)`. `guard.rs` screens each one (rate per arrival tick, the wire codec, then slot, future, stale, late and rewrite checks per input) and counts every refusal per slot in a `SlotReceipt`; `anticheat.rs` sanitizes admitted inputs (a spear press in a match without the spear is masked, `F_SPEAR`) and logs each changed input with its tick; `lagcomp.rs` sizes each connected slot's window from its measured one-way lag (or a reported round trip), one tick more, capped at 16, and `HeadlessServer::hold_ticks` holds every tick open for the widest one; every attack or spear press gets a `HitReceipt` (window in ticks and ms, fair or refused) that re-adjudicates exactly. The production caller is the played bout's host in `played.rs` (`net_bout`); `BoutReport` carries the receipts and folds them into its digest. Measured on the runner: a cheating seat tried every kind 4 times (malformed, impossible field, wrong slot, far future, stale replay, rewrite, plus a flood: 28 rate refusals) and the server tip, tape and checkpoints equalled the clean bout x2; honest seats show 0 cheat refusals (the rejoining seat shows 90 late copies of its backlog). A seat 80 ms away (2 to 3 ticks each way) behind a one-tick floor measured lag 3, the hold grew to 4 and all 12 of its presses landed on their own ticks: its tape equals the tape it plays beside the server, x2. A seat past the cap (18 ticks one way) has every press refused with window 16 and is stalled out to the AI. On the canonical MP6 bouts the measured window equals the configured hold (LAN 3, WAN 10). Not built: the browser duel host (`webduel.rs`, `net_serve`) does not route through `offer` yet; view rewind for snapshot-interpolated clients; RC14 replication and interest on the live send path (ON29). Landed `cc91744a`; MP13, MP20 and QN12 ticked on it (runs 39111873633, 39111897900, golden 39112424935).

Status 2026-10-17 (owner: multiplayer on the web too): browser clients cannot use raw UDP or QUIC. WEB9 in `docs/plan/web-and-realism.md` adds WebTransport (WebSocket fallback) to the dedicated server beside QUIC and proves one native client and one headless Chrome client end a duel on one state digest on a single GitHub runner. The snapshot codec, rollback and heal stay as they are.

## 2. Finish line

Scope tier: Core proof 4 (FOCUS Now, 2026-10-10). Rows are never unticked. Gap (rowed, never duplicated):
MP6 two-machine proof — all converge claims are loopback-only until it posts.

Tier A alpha (friends fight for real, proven on hardware): MP6
two-machine played proof, MP7 theater that attributes. Bar: 2 humans
plus 6 AI over real LAN hardware converge mid-fight through a
scripted drop+rejoin; measured loss/latency distributions posted and
the resim window sized by that data; theater replays three tapes with
frame-step and names the same first divergent tick as CR1 on all
three; keyframe resume proven per index.

Tier B v1 (a wire with rules, not comments): MP8 channel table,
MP9 compression experiment. Bar: every wire `KIND_*` maps to a
`Channel` plus a priority class in a table the tests read, combat
state never starves under a 16-fighter flood, 16-fighter full
snapshots stay ≤1200 bytes with deltas strictly smaller on quiet
tapes; compression lands only with ≥20% kbps saving at 16 fighters
or dies with numbers.

Tier C LAN-scale (the deeper finish line, new in this revision):
MP21 takes the LAN loop to 64 fighters plus a 1000-agent legion
read, interest-filtered, replay-exact. Best existing
implementation, named: `interest.rs::ConnectionInterest::select`
(`MAX_INTEREST_ENTITIES=1024`, `MAX_CONNECTIONS=64`,
`STARVE_TICKS=30`) with `siege_kbps_chart`, plus
`tick_budget.rs::BudgetReport` (p99 ≤ 4 ms at 64 + 500 AI, the P6
bar) and `wan_soak.rs` (150 ms / 30 ms / 2%, converge ≤ 300) —
all ticked under ON3, ON9, ON10, ON4, ON11. Edges: 16 fighters
full-rate inside 1200 bytes (MP2 shed); 32 (MP16) with idle shed
first; 64 plus 1000 (MP21) interest-filtered at 10–15 Hz inside
the ON9 128 kbps cap, with RTX 3050 6 GB kbps + p99 numbers
posted. A 2040-level LAN is a 64-slot siege playing ten minutes
and replaying exactly with per-match theater (MP19), windows
sized by data (MP14, MP20), physics authority over the ON7 seam —
no second codec, no WAN code in MP files, golden unchanged.

CUT forever on this plan: server fleet, crossplay and moderation
stay CUT per FOCUS (WAN relay, cheat authority and voice are ON
rows).

### Internet play (docs/archive/2026-10-08/plan/online.md section 2)

Scope tier: Core proof 4 (FOCUS Now, 2026-10-10). Rows are never unticked. Highest-value gap (already rowed, never
duplicated): ON30 online two-machine played proof — every internet
claim above is seeded-loopback-only until it posts.

Tier A alpha (internet fights for real): ON1 QUIC parity, ON5 cheat guards,
ON3-interest core at 8–16. Bar: 2 humans + 6 AI over QUIC converge with
MP1 tapes; seeded speed/rate/hit-claim cheats refused with replay tapes;
per-client kbps charted; LAN UDP path untouched.

Tier B v1 (online): ON2 relay + hole punch + invite codes, ON6
party-lite, ON4 WAN soak at 16–32. Bar: two machines on different networks join
by invite code with relay fallback; party of 3 queues into one match; soak at
150 ms RTT / 30 ms jitter / 2% loss + reorder + drop/rejoin converges on every
authoritative checkpoint with a per-fault report.

Tier C (EMPIRE outcome 1, Finish C C4 + P4 slice): ON3 full 64 +
500 AI, ON7 networked physics, ON8 voice, ON9–ON11 siege proofs. Bar: 64-slot
bot-filled siege plays 10 minutes under the ON4 fault model and replays
exactly; wall break converges on all clients at 150 ms RTT replay-exact; two
clients talk under 150 ms added latency with mute; reconnect under 5 s with
slot reclaim; one-hour soak zero desyncs; per-client bandwidth under cap;
server tick p99 ≤ 4 ms at 64 + 500 AI.

CUT (never): server fleet orchestration / autoscaling, kernel or client-side
anticheat drivers, crossplay certification, WebTransport/browser play,
proprietary Steamworks SDK vendoring (the own-code `steam_relay.rs` overlay
fabric beside QUIC stays; the SDK pump behind the `steam` feature only drains
its queues), moderation tooling.

## 3. Target design

This plan replaces `docs/systems/netcode.md` after review. What it
carries over unchanged: the server-authoritative model (the server
runs the same `arena` code as clients, no second sim), prediction
with repeat-last-input plus resimulation capped at 8 ticks, deltas
against the last-acked baseline with checkpoint hashes in every
snapshot, AI slots as ordinary fighters with broadcast inputs, net IO
off the tick with validated inputs consumed only at tick boundaries,
and the determinism rule (same seed plus same tape gives same bytes,
same selection and same soak hash; ties break by stable slot order,
never by thread or hash order).

- Status 2026-10-17: the disk shows the resim cap is 9, not 8, so every line in this file that
  quotes an 8-tick resim cap is stale (the lines above, the table row above, and the item below):
  `crates/net/src/netcode.rs:22 pub const MAX_ROLLBACK: usize = 9;` (its own test at `:550` asserts 9,
  "150 ms at 60 Hz is 9 ticks"), `crates/net/src/rollback.rs:39 WAN_ROLLBACK_CAP: usize =
  crate::netcode::MAX_ROLLBACK` with `:1171` asserting 9, and `crates/anim/src/net_pose.rs:34
  pub const RTT_TICKS_150MS: u32 = 9;`. Row S18 in `docs/plan/BOOK-GAPS-2026-10-12.md`.

Data flow per tick: clients send inputs with 3x redundancy
(`INPUT_BYTES=10` each) on unreliable datagrams; the server echoes
acknowledged inputs and emits deltas against the last-acked baseline
plus a checkpoint hash every snapshot (`SNAPSHOT_EVERY_TICKS=6`,
owned by `interest.rs`, read — never redefined — here). Clients
predict with repeat-last-input and resimulate at most 8 ticks on
authority arrival; deeper divergence triggers AH4 resync from the
authoritative snapshot (counted, never ignored). On the dedicated
server a remote press counts when it arrives before its tick is
authored: the server holds each tick open for the widest connected
`LagWindow` (one-way lag plus one tick, at most 16 ticks) and every
attack press gets a `HitReceipt` naming that window, so replays stay
exact under HASH-LAW. Lobby: beacon browser at 4 Hz, ready-up
with version plus rules-digest match, AI-fill slots as ordinary
fighters, all messages typed structs with fixed wire lengths, never
JSON. Replays: input tapes plus a keyframe every 600 ticks (proven
resume points through `run_prefix`/`resume`), theater offers
play/pause/frame-step plus the CR1 divergence report.

Ownership: `session.rs` (codec + redundancy), `delta.rs` (snapshots
+ interp), `dedicated.rs` (server + beacons), `scale.rs` (caps +
priorities + soak), `lobby.rs` (browser + ready-up + start),
`replay.rs` (tapes + keyframes + theater), `lagcomp.rs` (window +
verdicts + receipts), `heal.rs` (drop/desync recovery),
`rollback.rs` + `netcode.rs` (resim ring + 2P session shape).
Threads: net IO off the tick on std sockets; the sim consumes
validated inputs at tick boundaries only; no async runtime in the
loop (a thread-count-dependent reduction would break tier-3
determinism). Determinism: integer math plus stable slot order only
in every encode/select/adjudicate path; shuffled inputs give
identical bytes (`interest.rs` proves the pattern, MP8 copies it).
Budgets: resim ≤8 ticks/frame, snapshot ≤1200 bytes, 2 fighters
≤16 kbps, 8 fighters ≤64 kbps, 16 fighters ≤96 kbps (targets posted
per row, enforced in tests, narrowed by measurement never by silent
compression). Scale-up rule (MP16, MP21): 16 fighters serialize
full-rate; 32 shed idle first under the same 1200-byte assert; 64
plus the legion never serialize full — the loopback reads
`ConnectionInterest::select` per connection and sends only the
selected set at 10–15 Hz, far fighters at reduced rate, nothing
unsent past `STARVE_TICKS=30`. Interest is read-only from MP.
Snapshot rule (MP10, MP17): `snapshot.rs` owns Hermite plus
quaternion plus the extrapolate guard; `delta.rs::Interpolator`
keeps its 100 ms clamp-never-extrapolate meaning; the session path
calls both and never forks either. API sketch (landed, frozen shapes):

```rust
pub struct LagWindow { pub ticks: u8 } // ≤ MAX_WINDOW_TICKS (16)
pub struct HitReceipt { pub claim: HitClaim, pub window_ticks: u8, pub window_ms: u64, pub accepted: bool }
// HeadlessServer::offer(token, datagram, arrival_tick) -> admitted inputs; hold_ticks() -> u32
pub fn record_tape(session: &Session) -> Tape;
pub fn play_tape(tape: &Tape, upto: Tick) -> Checkpoint;
```

### Internet play (docs/archive/2026-10-08/plan/online.md section 3)

Data flow: fighters tick at 60 Hz on the server; clients send inputs every
tick with redundancy over QUIC datagrams; server echoes acks and sends
interest-filtered deltas against the last-acked baseline; checkpoint hash rides
every snapshot. Lobby, party, invite and cheat-report traffic rides reliable
QUIC streams; voice rides its own streams with sequence + jitter buffer
(presentation only, never in the hash).

Ownership and threads: `Transport` trait in `quic.rs` (UDP impl stays,
quinn impl added); `relay.rs` (relay binary + STUN/ICE-lite + invite codes);
`interest.rs` (AOI grid + per-connection priority + delta budgets);
`wan_soak.rs` (fault injector + report); `guard.rs` (authority checks +
report tapes); `party.rs` (party/list/lite); `phys_sync.rs` (debris/ragdoll/
projectile authority + resim); `crates/audio/src/voice.rs` extended by
`voice_net.rs` (Opus encode/decode + proximity mix, presentation only).
Net and voice IO run off-tick; sim consumes validated inputs at tick
boundaries; heal applies at tick boundaries and is recorded in the tape.

Determinism: authoritative sim uses integer/fixed math and stable slot order
only (`+ - * /`, `sqrt` per HASH-LAW tier 3); quantization is mm positions,
12-bit angles, 8-bit velocity deltas (Fiedler shapes, reimplemented); AI inputs
broadcast byte-identical; voice, jitter buffers, RTT reports and beacon timing
never enter hashes. Same config + same tape gives identical checkpoints on
Windows + Linux CI.

Budgets: snapshot ≤ 1200 B (no fragmentation); resim ≤ 8 ticks/frame, deeper
divergence triggers AH4 resync (counted); per-client caps 2 fighters ≤ 16 kbps,
8 ≤ 64 kbps at 20 Hz, 16 ≤ 96 kbps at 15 Hz, 64 + AI ≤ 128 kbps at 10–15 Hz
with far fighters at reduced rate; server tick p99 ≤ 4 ms at 64 + 500 AI;
voice +40 kbps peak per audible neighbor, ≤ 4 neighbors mixed; reconnect < 5 s.

API sketch:

```rust
pub trait Transport { // quic.rs; UDP impl stays, quinn impl added
  fn send_unreliable(&mut self, to: PeerId, bytes: &[u8]) -> Result<(), NetError>;
  fn send_reliable(&mut self, to: PeerId, kind: ControlKind, bytes: &[u8]) -> Result<(), NetError>;
}
pub struct InterestCfg { pub near_hz: u8, pub far_hz: u8, pub cap_bps: u32 }
pub fn invite_code(rng: &mut SplitMix64) -> [u8; 6];
pub fn guard_report(tick: Tick, why: GuardHit) -> Tape; // ON5, replay-backed
```

**Steals 2026-10-12** (board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`). Corrections are a presentation offset: drawn = live + a decaying error; the sim reads the live pose (MP25, lightyear correction). Pacing slews the client clock up to 5 percent over a 30-frame advantage window, never skips frames (MP26, ggrs and lightyear). Snapshots can be sparse: one saved state, resim from it. Peer checkpoints are compared only for confirmed frames; the every-tick sync test is the F2P for any new snapshot field (MP27).

## 4. Hard problems

1. Played proof under real routers → scripted two-machine bouts
   with a drop+rejoin mid-fight plus posted loss/latency
   distributions that size the resim window by data. Why: soak
   reports prove the code, only hardware proves the game; home-router
   loss may exceed 8 ticks and the plan must say so with numbers.
   Reference: Glenn Fiedler's networked-physics series (articles,
   reimplemented shapes). Donor: netcode.io packet shape only.
2. Priorities that cannot starve combat → channel table plus
   priority classes read by the tests, with a 16-fighter flood
   fixture asserting combat slots always serialize first and idle
   slots shed first. Why: comments do not schedule packets; a table
   the encoder reads does. Reference: lightyear channels
   (reimplemented, no dependency). Donor: lightyear channel shape.

3. Theater that proves instead of plays → keyframes as proven
   resume points (every index resumes through the arena snapshot
   codec) plus divergence reports that match CR1 tick-for-tick on
   three tapes. Why: a replay that disagrees with the causal
   machinery is a second truth. Reference: CR1/BUG1 machinery
   (own code, reused). Donor: none, own code.
4. Interpolation that does not fork the delta path → MP10 builds
   `snapshot.rs` and MP17 wires it beside the frozen delta
   Interpolator, with a 16-fighter jitter fixture proving both
   agree and the delta bytes unchanged. Why: two interpolators
   that disagree are two truths. Reference: flax-mcp Hermite +
   quaternion shapes (MP10). Donor: none new, own code.
5. 64 fighters plus 1000 agents on a LAN loop without touching
   authority → MP21 reads `ConnectionInterest::select` per
   connection in loopback, sends the selected set only, and kills
   on digest drift, float in the path, or kbps over the ON9 cap.
   Why: full-rate 64 never fits 1200 bytes; interest already
   charts under ON9. Reference: ON3/ON9 charts plus
   `tick_budget.rs` p99 method (own code). Donor: none.

### Internet play (docs/archive/2026-10-08/plan/online.md section 4)

1. QUIC without losing melee feel → unreliable datagrams for inputs/snapshots,
   reliable streams only for control; why: handshake/TLS and stream head-of-line
   blocking must never sit in the input path; reference: quinn docs + RFC 9000
   (datagrams) + RFC 9221; donor: quinn-rs/quinn (dep).
2. Joining behind home routers with no fleet → self-hosted relay binary +
   STUN binding + ICE-lite host/srflx/relay candidates with relay fallback;
   why: hole punching fails on symmetric NATs, a relay must always work;
   reference: RFC 8445 (ICE) + RFC 5389 (STUN) + Valve GameNetworkingSockets
   relay design docs (reference only, no C++ vendoring); donor: none as dep.
3. 64 fighters + 500 AI in 128 kbps → AOI grid relevancy + per-connection
   priority (local inputs > remote combat > pose > idle) + reduced far rate +
   AI input broadcast; why: bandwidth is the ceiling at siege scale; reference:
   Epic Iris relevancy/priority docs (public, reimplement) + Fiedler snapshot
   compression (articles, reimplement); donors: lightyear channels,
   bevy_replicon visibility (patterns only).
4. Rollback under 150 ms RTT → ≤8-tick resim then AH4 resync, checkpoint hash
   per snapshot, input redundancy + XOR/RLE compression; why: WAN loss bursts
   exceed LAN windows; reference: GGPO rollback concepts + backroll-rs XOR+RLE
   shape (reimplement integer-only); donor: backroll-rs (pattern only).
   Status 2026-10-17: the "≤8-tick resim" here is stale: the disk holds `crates/net/src/netcode.rs:22
   MAX_ROLLBACK = 9` and `crates/anim/src/net_pose.rs:34 RTT_TICKS_150MS = 9;`, so 150 ms RTT is 9
   ticks. Row S18 in `docs/plan/BOOK-GAPS-2026-10-12.md`.
5. Cheating without spyware → server-authority rate/range/movement/lag-window
   checks, hits only through MP5 lag compensation, refusals logged with replay
   tapes; why: kernel drivers are out, authority plus evidence is the product;
   reference: Valve Source lag-compensation notes (reimplement) + GNS cert
   model (patterns); no donor dep. Status 2026-10-18: built for the
   dedicated input path (`guard.rs` refusals per slot, `anticheat.rs`
   sanitize flags, `lagcomp.rs` arrival window); hit claims against a
   rewound view are not built, because no client sends one.
6. Networked physics that converges → server-authoritative break/contact events,
   clients predict debris presentation but resim authoritative events; joint
   rollback snapshots bodies + XPBD lambdas + substep cursor (PORT-AV-JOINT);
   why: physics has hidden solver state beyond poses; reference: UE 5.8
   networked-physics docs (IN per RESEARCH-VERDICT, reimplement) + Avian XPBD
   joint notes (pattern only); donor: none as dep until tier-3 audit passes.
7. Voice that never desyncs the sim → Opus over QUIC streams, jitter buffer +
   proximity mix presentation-only, push-to-talk + mute authoritative as flags;
   why: audio timing must not leak into the hash; reference: RFC 6716 (Opus) +
   Xiph docs; donor: opus lib (binding dep) + patterns only.
8. Hints that actually route + histories that stay bounded → `Channel` maps to
   the `Transport` datagram/stream path at the call site, and every rollback
   history (P2P tape plus `netcode.rs` session log) evicts past its cap with a
   requested-frame guard; why: an unmapped hint silently rides the wrong path
   and an unbounded log grows the hour-long soak; reference: lightyear channel
   table (patterns) + ggrs confirmed-frame discard (idea, reimplemented
   std-only); donors: lightyear, ggrs (patterns only, no dep).

**Steals 2026-10-12, hard problem.** Lag compensation: the duel is rollback, so hits are sim events inside resimulated ticks and need no rewind. The siege uses snapshots plus lagcomp: window = render cursor (8 ticks) + one-way latency (about 4.5 ticks at 150 ms RTT), about 12.5 ticks, so the 5-tick cap (`crates/net/src/lagcomp.rs:19-22`) would refuse every claim from a default client; MP20 sizes it with data. Status 2026-10-18: the restored `lagcomp.rs` is the arrival window of the input-authoritative dedicated server (cap 16 ticks, sized per slot from measured lag); a snapshot-view rewind for interpolated siege clients is still not built (the siege has no network client yet).

## 6. Slim

- Delete: nothing this wave (GE7 already deleted the lobby/scale
  private caps; SL17 already unified the tick-tuning constants).
  Deletion candidates ride their own audit rows, never drive-bys.
- Merge: none proposed. (`server.rs` `SessionServer` vs
  `dedicated.rs` `HeadlessServer` stay separate: 2P rollback harness
  vs LAN match server with beacons; a merge proposal needs a reader
  row first, and this plan does not invent one.)
- Freeze: `session.rs` codec shape (`WIRE_VERSION=1`,
  `MAX_PACKET_BYTES=1200`, `REDUNDANCY=3`), `delta.rs` quanta
  (mm positions, ddeg yaw), `lagcomp.rs` window shape (≤16 ticks,
  `MAX_WINDOW_TICKS`, in every receipt), `replay.rs` keyframe grid
  (every 600 ticks).
  Changes bump versions and refuse loudly; silent widening fails
  the gate.
- What we will not build: relay, WAN transport, hole punching,
  server browser backend, fleet orchestration, crossplay,
  moderation, full anticheat (ON5–ON8 own the scoped slices);
  a second codec beside the session codec (compression is an
  encoding inside it, MP9); a parallel replay system beside
  QA1/CR1 (theater reuses their tapes and attribution); QUIC in MP
  files (ON1 owns `quic.rs`; the LAN UDP path stays).

### Internet play (docs/archive/2026-10-08/plan/online.md section 6)

- Delete: the dead 1400 B snapshot line (1200 B wins); any WAN-shaped grep hits
  outside owned ON files (relay/hostname/stun/turn strings stay inside ON only).
- Merge: ON6 product into one `party.rs` (party + list + lite share invite +
  version-digest code with MP3 lobby); voice net into one `voice_net.rs`
  beside the bake-only `voice.rs` (no parallel audio state); WAN faults into
  one `wan_soak.rs` reused by every ON row.
- Freeze: `session.rs`/`delta.rs` codecs (additive version bump only, old
  versions refuse loudly); MP1–MP5 semantics (ON rows deepen them, never fork
  AI or authority); `aoi.rs` grid math (ON3 adds budgets around it, not a new grid).
- Will not build: fleet orchestration/autoscaling, kernel anticheat, crossplay
  cert, WebTransport/browser, Steam relay, moderation, server-side voice mixing
  beyond 4-neighbor proximity, dedicated physics server (arena sim stays one).
- Each row names its consumer, replaces or deepens existing code, states its
  budget, and adds no mirrored types (CODELAW slim law).
- Reusable pieces indexed (pointers, not copies): contract-test patterns for
  the channel table + 1200 B + starve-guard tests (see
  `../library/contract-test-patterns.md`); cross-crate harness wiring for the
  ON4/ON11 soak gates (see `../library/cross-crate-harness-wiring.md`).

## 7. Board tasks

Rows from the steals (2026-10-12, board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`): MP25 visual correction, MP26 clock slew, MP27 every-tick sync test.

Ticked rows are dropped. Rows never boarded are not listed (git keeps them). Now rows are a checklist with the Accept line verbatim; the rest are a table.

- [ ] MP6 two-machine played proof. Accept: played bout converges with full checkpoint equality; distributions posted; 16-bot x600 soak still hashes x2. needs: MP1, MP2, MP3.
- [ ] MP10 snapshot interpolation. Accept: Hermite C1-continuous; quaternion auto-dispatches nlerp below 1.8°; guard classifies interpolate/extrapolate. Kill: float in tick; divergence. needs: MP1.
- [ ] MP17 snapshot production wiring. Accept: 16-fighter jitter fixture interpolates C1 with guard classification; delta bytes unchanged; hash x2; `cargo test -p engine-net --lib snapshot` green. needs: MP10.
- [ ] ON30 online two-machine played proof. Accept: played bout converges with full checkpoint equality; punch-or-fallback within 300 ticks; hash x2 with the MP1 tape. needs: ON1, ON2, MP1.
- [ ] MP23 net: MP17 wiring re-proof after WIP, re-runs the MP17 accept on the current tree and posts the receipt. Tier B. Size S. Owns evidence under `target/mp17-reproof/` plus test-only hunks beside `crates/net/src/snapshot.rs` (no codec change; MP10 and MP17 frozen shapes stay). Accept: 16-fighter jitter fixture interpolates C1 with guard classification; delta bytes unchanged; hash x2; `cargo test -p engine-net --lib snapshot` green x2 with the receipt posted. needs: MP17.

| ID | Title | Tier | needs |
|---|---|---|---|
| MP8 | channel table + priority mapping | B | MP2 |
| MP12 | replication-over-LAN proof | B | MP2, RC14 |
| MP14 | resim auto-tune | A | MP6 |
| MP15 | dedicated auto-theater | A | MP1, MP4, CR1 |
| MP16 | 32-fighter LAN soak | B | MP2 |
| MP16a | 16-fighter LAN soak baseline within the current cap | B | MP2 |
| MP16b | 32-fighter LAN soak revival after the GE12 canonical-cap decision | B | MP2, GE12 |
| MP18 | channel + compression flood chart | B | MP8, MP9, MP16 |
| MP19 | every-match theater receipt | A | MP7, MP15 |
| MP21 | 64-player interest-handoff spike | C | MP16, ON3 |
| MP22 | backroll-style input-delay + first-incorrect resim | B | MP2 |
| ON6 | sessions | C | ON2, MP3 |
| ON7 | networked physics | C | ON1, DX4 |
| ON8 | proximity voice | C | ON1 |
| ON12 | voice proximity mix | C | ON1, ON8 |
| ON13 | invite-and-party hardening | C | ON2, ON6 |
| ON14 | datagram discipline | C | ON1, ON3, ON4, ON5 |
| ON15 | per-connection scope gate | C | ON3, MP2 |
| ON16 | token handshake + soak harness | C | ON1, ON2 |
| ON17 | per-class resend/memory discipline | C | ON1, ON3, ON4 |
| ON18 | version-stamped WAN resync | B | ON1, ON3 |
| ON19 | P2P-sync negative gate | B | ON1, ON5 |
| ON20 | delta-maintained scope views | B | ON3 |
| ON21 | rollback prediction from lightyear | C | ON1, MP1 |
| ON22 | GGRS input-prediction and desync-detection patterns | C | ON1, MP1 |
| ON23 | renet_netcode authenticated-transport audit | C | ON1 |
| ON24 | ICE conformance hardening | C | ON2 |
| ON25 | TURN allocation discipline | C | ON2 |
| ON26 | subscription-predicate interest | C | ON3, ON15 |
| ON27 | interest-aware relay spike | C | ON25, ON26 |
| ON28 | soft-collision net prediction | B | ON1, ON3 |
| ON29 | interest production wiring | B | ON3, ON9 |
| ON31 | voice-boundary proof with SN9 | B | ON1, ON8, AU1 |
| ON36 | confirmed-frame discard in P2P rollback | - | ON3 |

## 8. Quality gates

- Every row: loopback drop+rejoin and desync-resync converge with
  full checkpoint equality (AH4 bar); MP6 additionally converges
  the played bout on hardware.
- Bandwidth caps enforced in tests with charts posted (2/8/16 rows),
  never asserted in docs; a cap breach fails the gate, a target
  shortfall narrows the claim with numbers.
- Hash x2 on scripted multi-fighter tapes; AI inputs byte-identical
  with machine IDs swapped; shuffled inputs give identical bytes.
- No-fragment assert on every encode path (`<= 1200`); `OverBudget`
  refuses and counts, never fragments past the cap.
- Lag-window honesty: every attack press posts its window in a
  `HitReceipt`; a press outside its window is refused and counted,
  and a receipt that does not re-adjudicate is a test failure; the
  window is sized per slot from the measured lag.
- Lobby without typing: MP3 acceptance runs on two real machines
  with zero manual addresses and zero pasted IPs; versions match by
  digest or refuse loudly with the pair posted.
- Theater proof: MP7 replays three tapes and matches CR1's first
  divergent tick on each, or the row is red; every keyframe index
  is a proven resume point.
- Compression kill bar: MP9 lands only with ≥20% kbps saving at 16
  fighters and identical hashes, else it dies with numbers and no
  codec change ships.
- No WAN-shaped code in MP-owned files: grep gate for
  relay/hostname/stun/turn outside `quic.rs` + `relay.rs` +
  `steam_relay.rs` (the third WAN home, new this revision).
- Read-only seams: no MP row edits `interest.rs`,
  `tick_budget.rs`, `wan_soak.rs` or `guard.rs` (grep gate);
  `snapshot.rs` is the one new file and only because no existing
  file owns Hermite/quaternion interpolation.
- New-row kill bars are gates, never waivers: MP18 dies without
  its ≥20% saving or with any hash drift; MP21 dies on any
  digest drift, any float in the path, any per-client kbps over
  the ON9 cap, or any starve past `STARVE_TICKS=30`; MP19 is red
  unless the report matches CR1's first divergent tick with zero
  operator steps; MP20 is red when a hit outside its window
  passes or the sizing math leaves the test.
- Golden `d57b5c8ae3096614` unchanged x2 on every row (net feeds
  the tick through recorded inputs, never around it).

### Internet play (docs/archive/2026-10-08/plan/online.md section 8)

- Every row: MP1 loopback tapes converge over the new path (QUIC, relay, or
  interest-filtered) with full checkpoint equality; hash x2 on scripted tapes.
- Bandwidth caps enforced in tests with charts, not docs: 8 ≤ 64 kbps, 16 ≤ 96
  kbps, 64 + AI ≤ 128 kbps; snapshot encode > 1200 B returns `OverBudget`.
- WAN honesty: ON4/ON11 post per-fault reports (150 ms RTT, 30 ms jitter, 2%
  loss, reorder, drop/rejoin); a checkpoint divergence is a failure, not a shrug.
- Cheat honesty: seeded speed, rate and hit-claim cheats refused and logged with
  a replay tape that replays the refusal; a cheat outside its window is red;
  ON38 refusal quarantines the slot and the logged tape position rebuilds it x2.
- Channel honesty (ON37): Reliable rides streams, Unreliable/Sequenced ride
  datagrams on every path; a hint on the wrong path fails the row.
- History honesty (ON36/ON40): P2P tape and `netcode.rs` history stay ≤
  CAP+pending on the 64-frame bursty-loss tape with fold/tip/digest x2; an
  unbounded push fails the row.
- Interest honesty: far-rate LOD never drops combat-affecting state; a desync
  traced to a dropped combat delta fails the row; ON29 oracle equality x2.
- Physics honesty: ON7 wall break converges at 150 ms RTT replay-exact; joint
  state without tier-3 audit fails the gate by construction.
- Voice honesty: golden hash identical with voice on/off; mute/deaf fixtures pass.
- No-fleet grep gate: `relay|fleet|stun|turn|hostname` outside owned ON files
  fails proof; no `Unsupported`/`todo!`/stub in shipped ON paths.
- Two-machine proof for ON2/ON6/ON13/ON30: invite join and party form-up run on
  real hardware with zero pasted addresses; versions matched by digest.

