# Protocol — the 64-player wire

One page for `content/systems/protocol.json` (version 1, QUIC). Every number here is
a contract, not a wish: the bytes add up against the 900 B per player per second the
snapshot block declares, and the authority split is the one `docs/HASH-LAW.md` demands.

## The tick

| Thing | Value | Where it comes from |
|---|---|---|
| Wire tick | 30 Hz | `tick_hz` in the JSON. The replication clock. |
| Sim tick | 60 Hz | HASH-LAW pins the authoritative sim at a fixed 60 Hz. |
| Input frame | 60 Hz, 10 B | `session::INPUT_BYTES = 10`; one frame per sim tick. |
| Input buffer | 100 ms = 3 wire ticks = 6 sim ticks | `inputs.buffer_ms`. The client always sends intent for ticks it has not seen. |
| Snapshot | every 3 wire ticks = 10 Hz | `interest::SNAPSHOT_EVERY_TICKS = 6` sim ticks at 60 Hz is the same 10 Hz. |
| Interest | cell | `interest::ConnectionInterest::select`, `MAX_CONNECTIONS = 64`. |
| Delta | field | `delta.rs` mask bits (`MASK_X`..`MASK_STAMINA`); only changed fields ride. |
| Packet cap | 1200 B, never fragmented | `session::MAX_PACKET_BYTES`. |

**Open point, stated because it is a real one.** `tick_hz: 30` is the *wire* tick; the
authoritative sim still ticks at 60 Hz. One wire tick therefore carries two sim ticks:
the server authors sim ticks and folds each pair into one 30 Hz replication tick, and
every message carries the sim tick number so a client never guesses. If the owner wants
one clock instead, `tick_hz` becomes 60 and the snapshot cadence is every 6 ticks: the
same 10 Hz and the same bytes, with one extra packet header per second. The budget
below does not move either way.

## The budget arithmetic

Downlink, server to one client, steady state (`rate_hz x payload_bytes`):

| Message | rate_hz | payload_bytes | B/s |
|---|---|---|---|
| roster | 0.2 | 40 | 8 |
| roster update | 10 | 48 | 480 |
| hit | 5 | 18 | 90 |
| death | 0.5 | 24 | 12 |
| capture tick | 2 | 12 | 24 |
| window state | 2 | 16 | 32 |
| chat (room fan-out) | 0.5 | 64 | 32 |
| **total** | | | **678 B/s** |

Uplink, one client to the server:

| Message | rate_hz | payload_bytes | B/s |
|---|---|---|---|
| join | 0 (one-shot) | 48 | 0 (48 once) |
| move | 60 | 10 | 600 |
| cast | 10 | 6 | 60 |
| chat | 0.5 | 64 | 32 |
| fort build | 1 | 20 | 20 |
| packet overhead (6 B header x 20 packets/s) | | | 120 |
| **total** | | | **832 B/s** |

Both directions fit under the 900 B per player per second the snapshot block declares:
downlink **678 / 900** (222 B headroom, 24.7%), uplink **832 / 900** (68 B headroom, 7.6%).
The 64-player room therefore runs at 64 x 678 = **43,392 B/s (347.1 kbps)** down and
64 x 832 = **53,248 B/s (426.0 kbps)** up, worst case when every client sits at its cap.
Per client that is 678 B/s = 5.4 kbps, 4.2% of the per-connection cap
`interest::ON9_CAP_BPS` (16,000 B/s = 128 kbps), so the cap is never the binding
constraint here — the 900 B budget is.

The input stream is the expensive half and it is bounded on purpose: 60 frames/s of
10 B, bundled three frames to a packet (`session::REDUNDANCY = 3`), so one lost packet
costs one tick of input and the header cost stays 120 B/s. Position replication is the
other half: 480 B/s for up to four entities at 10 Hz, which is what 64 players on a
cell interest grid costs — not 64 x 48 B at 30 Hz.

## The message table

| id | Message | Dir | rate_hz | bytes | What it carries | Shipped? |
|---|---|---|---|---|---|---|
| 1 | join | c2s | 0 | 48 | token, wire version, build digest, name hash, client tick | `lobby::BuildId` |
| 2 | roster | s2c | 0.2 | 40 | present/team/alive masks, roster sequence, join-leave count | `lobby.rs` |
| 3 | move | c2s | 60 | 10 | tick, seq, move axes (q8), buttons — intent only | `session.rs` |
| 4 | cast | c2s | 10 | 6 | tick, ability id, aim (q8) — intent only | spec only |
| 5 | hit | s2c | 5 | 18 | tick, attacker, target, damage, cooldown left, flags | `lagcomp.rs` receipts |
| 6 | death | s2c | 0.5 | 24 | tick, slot, killer, cause, respawn tick, match hash | `arena` checkpoints |
| 7 | capture tick | s2c | 2 | 12 | tick, point id, holder team, progress (q8), next tick | spec only |
| 8 | chat | c2s | 0.5 | 64 | channel, sender slot, seq, text | spec only |
| 9 | roster update | s2c | 10 | 48 | base tick, tick, entity count, dx/dy/dz (mm), yaw (ddeg), hp (permille), flags | `delta.rs` + `interest.rs` |
| 10 | fort build | c2s | 1 | 20 | tick, structure type, cell id, rotation (q8), client nonce | spec only |
| 11 | window state | s2c | 2 | 16 | tick, structure id, structure type, hp, build state, open (q8) | spec only |

One message per gameplay concept, no duplicates. "Spec only" means the wire is pinned
here but no crate sends it yet: the shipped game is a duel (`crates/game/src/siege.rs`
has the siege seats, the arena has no capture point, and nothing builds forts).
Unshipped coverage is unsupported coverage, not a green parity result.

## The authority split

| Owned by the server | Owned by the client |
|---|---|
| position (authored from admitted inputs) | prediction (own fighter only, repeat-last-input) |
| damage (hit resolution, `lagcomp::HitReceipt`) | animation (presentation only) |
| cooldown (ability and attack timers) | camera, chat text, UI selection |
| capture (point holder and progress) | nothing else |

The hash law is the reason the split is a split and not a courtesy: the server never
trusts a client number. A client sends **intent** (axes, buttons, ability id, build
order) and never a number it would like to be true — no position, no damage, no
cooldown, no capture claim. `anticheat.rs` masks an illegal bit (`F_SPEAR`) before the
input enters the sim, `guard.rs` refuses malformed, bad-field, wrong-slot, future,
stale, late, rewritten and over-rate packets and counts every refusal per slot in a
`SlotReceipt`, and a press outside the client's lag window is refused and counted, not
admitted. The server's checkpoints are the truth; a client that disagrees is corrected
by resimulation from the server snapshot and the mismatch is **counted**.

**The three server checks, and the replay that proves them.**

- **speed** — the server authors position from the admitted inputs, so a client cannot
  claim a position at all; the check is the derived-speed envelope over the authored
  path. The float speed/teleport module stays deleted, and that is the honest state.
- **cooldown** — the server holds every attack and ability timer; the client's press
  only asks. `lagcomp.rs` sizes the arrival window per slot from the measured lag
  (cap 16 ticks) and stamps it on every `HitReceipt`.
- **rate** — `guard.rs` refuses a slot above `RATE_MAX_PACKETS` (16) per arrival tick.

**The replay.** The evidence is the **input-tape replay**: `crates/net/src/replay.rs`
records the tape with a keyframe every 600 ticks (`KEYFRAME_EVERY`), the theater
replays it, and `divergence_report` names the **first divergent tick**. The server's
own refusals are folded into the bout digest (`crates/net/src/played.rs`, `BoutReport`),
which lands in the JSON receipt (`crates/net/src/receipt.rs`) that `tools/mp6-check.mjs`
reads. The runs that carry those receipts are `net_bout mp6` (LAN) and `net_bout on30`
(QUIC + relay) — both are still seeded-loopback receipts: the two-machine legs (MP6,
ON30) are open rows in `docs/plan/multiplayer.md`, so nothing here claims a hardware
proof. A refusal is reproducible from the tape alone: same tape, same admission, same
refusal, same first divergent tick. A claim without that tape is not evidence.

## The reconnect path

Window: **30 s** (`reconnect.window_s`).

1. **Drop.** The server notices the missing token. `heal.rs` routes the fault through
   the supervisor and returns a `ReconnectPlan`: the slot is **sticky for 30 s** and
   is AI-filled from the next tick boundary (the fight never stops), with 4 attempts,
   100-1600 ms backoff and one tick between retries. Past the budget the slot is
   quarantined loudly with an operator notice, never silently reused.
2. **Return.** The client reconnects with the same token inside 30 s and reclaims its
   own slot (`dedicated::HeadlessServer::reclaim_slot`). Outside the window the slot
   is gone for good: the client gets a new seat, or `SlotFull`, and rejoins as a new
   fighter. The token is the identity; the slot is the seat.
3. **Resync.** The server hands over the authoritative snapshot delta for the current
   tick (`resync(token)`); the client drops its local prediction and replays the last
   keyframe plus the recorded input tape to catch up. It is the same path a desync
   takes (`resync_via_supervisor` -> `DesyncPlan`).
4. **Verify.** The client reports its checkpoint hash (`verify(token, tick, hash)`).
   Equal means the resync landed; unequal means a desync, which is counted and healed
   again, never ignored (HASH-LAW).
5. **Again.** Steps 1-4 are the same loop for a drop at tick 10 or tick 6000. The
   heal tape records every attempt, so the receipt shows the whole reconnect history
   and the replay can rebuild it.

## What this page does not settle

- The 30 Hz wire tick beside the 60 Hz sim tick (folded above; one clock is the
  alternative).
- The 100 ms input buffer beside `session::INPUT_DELAY_TICKS = 2` (2 ticks = 33 ms).
  The spec wants the deeper buffer for WAN; the shipped LAN path uses the smaller one.
- The interest cell size. This page budgets four entities per client per update; the
  cell geometry and the far-LOD rate belong to the map, not to the protocol.
- Chat, capture, fort build and window state have no shipped code path. They are
  pinned here so the wire does not have to be re-cut later.
