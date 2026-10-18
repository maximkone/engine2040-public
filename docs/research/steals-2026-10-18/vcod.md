STEAL SCOUT: janost/vcod (BLOCK 3, position 3, 2026-10-18). Read-only pass. Donor files read live from raw.githubusercontent.com on 2026-10-18; no clone, no build, no run, no bytes vendored.
Repo: janost/vcod -- Call of Duty (2003) multiplayer rebuilt in Rust: client, dedicated server, bots, speaking the original patch 1.1/1.5/UO wire protocol. 8 stars, pushed 2026-10-18, default branch master. 1160 files; crates client (123 src), common (66 src), server (78 src), gsc (13 src), plus 38 research docs.
Licence: GPL-3.0-or-later. LICENSE read live 2026-10-18 shows the GNU GPL v3 text (675 lines); NOTICE:3-6 says "either version 3 of the License, or (at your option) any later version"; GitHub API reports spdx GPL-3.0. Ported routines are named in NOTICE:8-31: Quake III Arena (GPL-2.0-or-later) and RTCW-MP (GPL-3.0 with additional terms). Our rule 4 allows porting or adapting donor code from any source; this note is the honest record, not a blocker.
Plans read: multiplayer.md (rollback, wire, lag comp), HASH-LAW.md, docs/plan/steals.md, FOCUS.md Now.

== 1. WE ALREADY HAVE ==
- A quake-lineage bitstream: crates/net/src/delta.rs:403 `Ioq3Msg` -- "Bitstream with overflow flag (MSG discipline, integer only, byte appends)". encode_delta :263, delta_wire_len :381. So the MSG discipline this repo ports from RTCW is already our shape; the steals below are the parts we did not take.
- A bounded lag window: crates/net/src/lagcomp.rs:1-16 (per-slot window = one-way trip in whole ticks + 1, capped at MAX_WINDOW_TICKS). Do not re-steal that.
- A WAN profile harness: tools/mp6-check.mjs and crates/net/src/wan_soak.rs (MP6/MP24 gates).
- Wall-muffled audio: crates/audio/src/occlude.rs:1-30 -- integer Q16 gains, integer cm, one-pole lowpass, ATTACK_MS 120 / RELEASE_MS 300 slew, per-tick occluder set as input data.
- A snapshot baseline path: crates/net/src/snapshot.rs plus delta.rs baselines.

== 2. STEALS (ranked by value for the Rome duel per hour) ==
1. Static Huffman message tree plus the adaptive connect-packet coder | janost/vcod@master:crates/common/src/net/huffman.rs:11-16,46-51,54-65,90-107 (read 2026-10-18) | GPL-3.0-or-later | SIM (wire, lossless) | lands crates/net/src/delta.rs inside the session codec (MP9) | M
   Idea: one frozen tree is built once from a stored symbol-frequency table (`HMAX` 256, `NYT` 256, `INTERNAL_NODE` 257, `POOL` 768) and gives a fixed code per symbol; a second adaptive tree starts with only a not-yet-transmitted node and learns from the bytes it sends, which is what the `connect` packet uses. The Rust keeps the C's two 768-entry pools as index arrays (`NIL` for NULL) so the port can be diffed against the original line by line (huffman.rs:8-9). The two structures are separate: `Huffman` is the frozen tree, `Builder` is the adaptive one.
   Maps: this is the next step after `Ioq3Msg` (delta.rs:403). Our changed-field bytes are raw; a frozen Huffman over the delta alphabet spends fewer bits on the common small deltas, and the code table is a constant so the tick hash cannot move.
   Moves: MP9 asks for >= 20% kbps saving at 16 fighters (multiplayer.md:494). Unmeasured here; the frozen-tree path is lossless so the round trip is exact by construction.
   Proof: cargo test -p engine-net --lib huffman x2 (encode then decode is byte-equal for a fixture corpus; a table mismatch refuses) plus `node tools/mp6-check.mjs` printing the kbps drop; golden d57b5c8ae3096614 unchanged.
   Risk: an adaptive coder carries state across packets, so it must live in the session and be rebuilt on reconnect. Kill: if the frozen tree alone gives < 20% on the MP9 corpus, drop it (the adaptive path is the only part worth the state).

2. In-process bad-network simulator with three knobs | janost/vcod@master:crates/common/src/net/netsim.rs:12-17,22-35,49-58,86-101,128-167 (read 2026-10-18) | GPL-3.0-or-later | SIM-adjacent (harness only, never in the tick) | lands crates/net/src/wan_soak.rs and tools/mp6-check.mjs | S
   Idea: one env string `VCOD_NETSIM="ping=100,jitter=20,loss=2"` sets a one-way delay (half the ping each way), a per-datagram jitter of +/- jitter/2, and a percentage dropped each way. Each direction is a `VecDeque<(Instant, Vec<u8>)>`; a pushed datagram never leaves before the one ahead of it (`at.max(last)`, netsim.rs:56), so order is kept the way queueing jitter keeps it. The loss draw and the jitter draw come from one xorshift64* seeded by the caller (netsim.rs:86-92), so a run is reproducible. Tests assert order kept and about 10% lost (netsim.rs:148-166).
   Maps: our WAN profiles are a table in tools/mp6-check.mjs; this is the same three knobs expressed as an in-process transport wrapper with a seeded RNG, which makes a failing profile reproducible in a unit test instead of only in the tool. It never touches sim state: datagrams are the only thing delayed.
   Moves: MP6/MP24 profiles become seeded tests; today the numbers only come from the mjs harness.
   Proof: cargo test -p engine-net --lib netsim x2 (a 1000-datagram run keeps order and drops within 60..140 at loss=10; the same seed gives the same drop set) plus `node tools/mp6-check.mjs` unchanged; golden unchanged.
   Risk: wall-clock `Instant` inside a test makes it flaky; take a virtual clock instead. Kill: any test that needs real sleeps, or any path where the shim can reach the fixed tick.

3. Staggered line-of-sight audio occlusion: one trace per voice every few frames | janost/vcod@master:crates/client/src/audio/occlusion.rs:15-27,46-70 (read 2026-10-18) | GPL-3.0-or-later | PRESENTATION | lands crates/audio/src/occlude.rs plus the game's wall feed | S
   Idea: a blocked path multiplies gain by 0.25 (about -12 dB) and the mix approaches the verdict at 0.35 of the remaining gap per frame (about 5 steps to settle at 60 fps), so a wall never zips. Each voice is retraced once every 6 frames, staggered by `id % TRACE_PERIOD`, so the cost is spread instead of spiking. Below a 4 m emitter distance nothing can stand between, so the trace is skipped. If the trace starts inside solid geometry the fraction says nothing and the previous belief is kept, which stops a voice going loud when it is stuck in a wall.
   Maps: we already have the gain and the slew (occlude.rs ATTACK_MS/RELEASE_MS, Q16 integer), so the new parts are the stagger schedule, the near-field skip, and the startsolid "keep the previous belief" rule. Those are exactly what a per-tick wall query needs if the occluder set ever grows.
   Moves: traces per frame -> ceil(voices/6) (unmeasured); a voice pressed into a wall stops flapping.
   Proof: cargo test -p engine-audio --lib occlude x2 (a scripted path into a wall and back gives a monotone settle within 8 ticks; startsolid keeps the prior gain; golden d57b5c8ae3096614 unchanged).
   Risk: a 6-frame staleness can show a step when a wall opens fast. Kill: if a listen-once A/B hears a muffle that arrives late, shorten TRACE_PERIOD to 3 or drop the stagger and keep the near-field skip only.

4. Snapshot delta ring with a missing-base rule | janost/vcod@master:crates/common/src/net/snapshot.rs:19-44,62-70 (read 2026-10-18) | GPL-3.0-or-later | SIM (wire) | lands crates/net/src/snapshot.rs and delta.rs | S
   Idea: a 32-slot ring indexed by `message_num % RING_CAP`; a frame carries `delta_num` (its base message, -1 when uncompressed) and resolves it against the ring. A frame whose base is missing is marked `valid = false` and is NOT kept as a base for later frames, so one lost packet cannot poison the whole chain. A new gamestate clears every slot and forces every frame to delta against dead baselines.
   Maps: our delta path picks a baseline; the "invalid frame is never a base" rule is the small robustness idea worth copying, and it is testable without a wire.
   Moves: a dropped snapshot today can make the next frame delta against a stale base; with the rule the receiver falls back to a full frame. Unmeasured.
   Proof: cargo test -p engine-net --lib snapshot x2 (a ring with one base removed produces a full frame at the next slot, then resumes deltas; bytes x2 equal for the healthy case); golden unchanged.
   Risk: a full-frame fallback costs bandwidth on a lossy link. Kill: if a 2% loss profile in tools/mp6-check.mjs shows more than 5% extra kbps, keep deltas against the last valid base instead.

== 3. NOT WORTH STEALING ==
- The GPL binary-reverse-engineering pipeline (tools/re/, docs/research/cod11-*.md, fields_v1.rs:1-14 generated from cod_lnxded 1.1d with md5 and vaddrs): the technique is fine but its output is a 2003 game's field tables. Nothing to port.
- The 1.1/1.5/UO protocol tables themselves (fields_v1.rs:19-60, 59 entity fields with per-field bit widths, array order = wire order): our wire is ours; only the "bit width per field, never reorder" idea is already in delta.rs.
- The id Tech quake path (pmove pepper: pmove/predict.rs:19-70, prediction from a wire playerstate) at large: we are rollback, not client prediction over a partial playerstate. The one reusable idea -- predict only the pm_types that are predictable (predict.rs:34-36) -- is already implied by our arena tick.
- The HUD, frontend, bots, GSC VM, and the retail-referee capture rig: 2003-game content, not engine material.

DONE 4 steals, 6 donor files read.
