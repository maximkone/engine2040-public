# 02 Netcode and web multiplayer: what makes melee feel good and stay fair

Date 2026-10-17. Scope: `crates/net` (rollback, snapshots, transports, browser legs) and `crates/arena` (the duel and the siege).
Read first: `docs/plan/multiplayer.md`, `docs/plan/web-and-realism.md`, `crates/net/src/lib.rs` headers, and `docs/research/web-2026-10-17/04-browser-multiplayer.md` (certificate rules, WebSocket race, wtransport, matchbox, Chrome flags: not repeated here).

Legend. "Verified" = a page fetched in this session, URL given. "Unverified" = a search snippet only, a page that returned 403/404, or memory. Licences are as printed on the fetched page.

## 0. Repo facts used below (grep and read, 2026-10-17)

- `session.rs:107` REDUNDANCY = 3; `session.rs:109` INPUT_DELAY_TICKS = 2 (33 ms at 60 Hz).
- `delay.rs:19` MAX_DELAY = 8 ticks; `delay.rs:25` MAX_TARGET_ABS = 8.
- `delta.rs:46` DEFAULT_INTERP_DELAY_MS = 100; its comment reads "100 ms at 60 Hz is 6 ticks".
- `interest.rs:21-27`: MAX_INTEREST_ENTITIES 1024, MAX_CONNECTIONS 64, STARVE_TICKS 30, SNAPSHOT_EVERY_TICKS 6 (so snapshots run at 10 Hz).
- `quic.rs:83` CHANNEL_TABLE has 3 entries; `quic.rs:895` QUIC_KEEPALIVE = 5 s.
- `lib.rs:108`: `Rollback::advance` is documented "stub records only". The real resimulation lives in `rollback.rs`.
- Grep of `crates/net/src`: no call to `max_datagram_size`. `send_datagram` appears only in `link.rs:258`, the simulated link. The QUIC path does not read the live datagram limit.
- `arena/src/lib.rs:12-37`: `run` consumes tape inputs at fixed ticks only; the checkpoint hash covers hp, stamina, combat events, locomotion state and the controller hashes under `sim`. No frame time or render camera enters it.

## 1. Summary

| # | Technique | Licence (as printed on page) | Cost | Determinism / hash risk | Row |
|---|---|---|---|---|---|
| 1 | Rollback with repeat-last prediction | GGPO MIT; GGRS MIT or Apache-2.0; bevy_ggrs MIT or Apache-2.0 | S (review) | low if save/load is integer state | QN1 |
| 2 | Input delay as a tuning dial | GGRS docs.rs (API); GGPO README | S | medium: delay must be in MatchSpec | QN2 |
| 3 | Acked-baseline delta with per-field change bits | no licence on page (Fiedler) | M | medium: article's quaternion encode uses floats | QN3 |
| 4 | Interpolation delay sized by send rate and loss | no licence on page (Fiedler) | S | none (presentation only) | QN4 |
| 5 | Channel table and the 1200-byte boundary | Lightyear Apache-2.0 OR MIT | S | low | QN5 |
| 6 | QUIC datagram size gate | RFC 9221 and quinn docs (spec, docs) | S | low | QN6 |
| 7 | WebTransport datagram limit and cert validity | W3C spec, MDN (spec, docs) | S | low | QN7 |
| 8 | WebSocket fallback: send latest only | RFC 6455 (spec) | S | none | QN8 |
| 9 | Browser timer throttling and hidden tabs | Chrome blog (doc) | M | medium: pause must be a tape event | QN9 |
| 10 | Per-client visibility filter, no-op at 2 fighters | bevy_replicon Apache-2.0 OR MIT | S | high if bytes change on the duel | QN10 |
| 11 | Checksum desync detection and SyncTest self-check | GGRS MIT or Apache-2.0 | S | high by definition: the hash law | QN11 |
| 12 | Lag-compensation window with bounded claims | Lightyear Apache-2.0 OR MIT; Valve wiki unverified | M-L | medium: window in integer ticks | QN12 |

## 2. Techniques

### T1 Rollback with repeat-last prediction

- What: each client advances on predicted inputs (the last real input repeated). When the real input arrives, it loads the saved state from before the mismatch and resimulates. GGPO's README describes rollback as input prediction plus speculative execution (paraphrase).
- Source: https://github.com/pond3r/ggpo (MIT, verified). https://github.com/gschup/ggrs (MIT or Apache-2.0, verified, docs.rs ggrs 0.13.0). https://github.com/johanhelsing/bevy_ggrs (MIT or Apache-2.0, verified; saves only components registered with `register_rollback_type`).
- Numbers: GGPO README worked table (verified): broadband transit 5-150 ms = 1-9 frames of lag at 60 fps. GGRS default prediction window: unverified (not stated on docs.rs).
- Maps onto: `rollback.rs` and `netcode.rs` (MAX_ROLLBACK 9, per the plan's status line; not re-read). `lib.rs:108` `Rollback` is a stub that records hashes only, a second and smaller rollback type. QN1 settles which one the duel uses.
- Cost S (already built, NET1). Risk low: save and load must stay integer state; the arena checkpoint already covers it.
- Row: QN1.

### T2 Input delay as a dial

- What: delay each local input by N ticks so most remote inputs arrive before they are needed. More delay means fewer rollbacks and more input lag. This is the knob that decides how a fighter feels.
- Source: GGRS docs.rs 0.13.0 lists `SessionBuilder::with_input_delay(n)` (verified). GGPO README worked example: transit 3 frames (verified). Fiedler, Deterministic Lockstep: "the cost of increasing the buffer size is additional latency" (verified, https://gafferongames.com/post/deterministic_lockstep/, no licence on page). Coherence docs: a 3-frame input delay example, no default (verified, https://docs.coherence.io/manual/advanced-topics/competitive-games/determinism-prediction-rollback, licence not read).
- Numbers: ours INPUT_DELAY_TICKS 2 (33 ms at 60 Hz), MAX_DELAY 8 ticks (133 ms). Library defaults: unverified.
- Maps onto: `session.rs:109`, `delay.rs`. Arena takes fixed-tick inputs (`arena/src/lib.rs:35-37`), so delay in ticks is the only form that keeps replays exact. MP22 owns the tuning; QN2 adds the sweep receipt.
- Cost S. Risk medium: both sides must use the same delay, so it belongs in MatchSpec. Plan rule holds: ms never enters the seed.
- Row: QN2.

### T3 Acked-baseline delta with per-field change bits

- What: snapshot N is coded against the last snapshot the receiver acknowledged. Each entity has an "unchanged" bit, each field has a change flag, and entity IDs use relative index coding. Fiedler's article is the clearest public walk-through.
- Source: https://gafferongames.com/post/snapshot_compression/ (verified; page shows "Copyright 2004-2026 Glenn Fiedler" and links his code gist; licence terms unverified).
- Numbers (verified, one scene of 901 cubes at 60 Hz, not our scene): absolute state 80 bits (10 bytes) after bit packing, against 321 bits (40.125 bytes) unpacked; smallest-three orientation 29 bits; position delta averages 26.1 bits; orientation delta averages 23.3 bits; relative index averages 5.5 bits; fully stationary steady state about 15 kbps; a full snapshot of 900 cubes about 25 KB.
- Maps onto: `delta.rs:31-43` already has MASK_X, MASK_Y, MASK_Z, MASK_YAW, MASK_HP, MASK_STAMINA, which is the per-field change-bit idea. `session.rs` encode_delta is the ack-baseline path. MP9 owns the compression kill bar (20 percent saving at 16 fighters).
- Cost M. Risk medium: Fiedler's smallest-three quaternion uses float math; ours must stay integer (mm positions, ddeg yaw). Port the bit layout, not the float encode.
- Row: QN3 (an ack fallback correctness row; compression itself stays in MP9).

### T4 Interpolation delay sized by send rate and loss; jitter buffer

- What: remote entities render behind real time so a snapshot bracketing the render time almost always exists. The delay must cover two consecutive lost packets. Stale snapshots are dropped by 16-bit sequence number.
- Source: https://gafferongames.com/post/snapshot_interpolation/ (verified; no licence on page).
- Numbers (verified): at 2-5 percent loss the best delay is "3X the packet send rate". By send rate: 10 pps about 350 ms (300 ms plus margin), 30 pps about 150 ms, 60 pps about 85 ms. Jitter margin 1-2 frames at 60 fps (17-33 ms). Test condition in the article: 10 pps, 5 percent loss, plus or minus 2 frames of jitter.
- Maps onto: `delta.rs:46` DEFAULT_INTERP_DELAY_MS = 100, which is one snapshot interval at SNAPSHOT_EVERY_TICKS = 6 (10 Hz). By Fiedler's rule that sits below the two-loss floor (200 ms) and below 3x the send period (300 ms). The duel does not use this path (rollback), so the cost falls on siege and far fighters and on any entity outside rollback. Decide with data (MP6, MP20), not the rule alone.
- Cost S to measure; M if the cadence changes. Risk none: presentation only, per the plan rule.
- Row: QN4.

### T5 Channel table and the 1200-byte boundary (lightyear)

- What: each message kind maps to a channel (reliable or unreliable, ordered or sequenced) and a priority. Messages above about 1200 bytes fragment. Inputs are buffered per tick and resent across recent frames to survive loss.
- Source: https://github.com/cBournhonesque/lightyear (Apache-2.0 OR MIT, verified; README: "Packet fragmentation (for messages larger than ~1200 bytes)" and inputs "resent across recent frames").
- Numbers: 1200 bytes (lightyear's fragmentation point). Ours is MAX_PACKET_BYTES = 1200 with no fragmentation; over-budget returns OverBudget (plan). Lightyear gives no tick rate or bandwidth number (verified: none).
- Maps onto: `quic.rs:83` CHANNEL_TABLE, 3 entries. MP8 owns the table; ON37 owns channel honesty. Lightyear's fragmentation is a counter-example to our rule, not a donor.
- Cost S. Risk low: arrival order must reach the sim only through tick-indexed inputs.
- Row: QN5.

### T6 QUIC datagram size gate

- What: QUIC DATAGRAM frames are unreliable, not flow-controlled, not retransmitted, and subject to congestion control; a sender may hold or drop an unsent frame. The peer's limit is the transport parameter max_datagram_frame_size, and sending above it is a protocol violation.
- Source: RFC 9221 https://www.rfc-editor.org/rfc/rfc9221 (verified; spec). quinn Connection docs https://docs.rs/quinn/latest/quinn/struct.Connection.html (verified): `max_datagram_size()` returns None if unsupported or disabled, can change as the path-MTU estimate changes, and can be smaller than the peer's advertised limit; when the peer allows a large value it is "at least slightly over 1 KiB". `send_datagram` does not block; older unsent datagrams may be discarded to make room. `send_datagram_wait` waits and favours older datagrams.
- Numbers: our 1200 bytes against quinn's guaranteed floor of "slightly over 1 KiB" (verified). RFC 9221 recommends 65535 for the parameter "for most uses" (verified).
- Maps onto: no `max_datagram_size` call in `crates/net/src` (grep 2026-10-17); `send_datagram` only in the simulated `link.rs`. So the 1200-byte constant is not checked against the live limit on the QUIC path. Where the path estimate is below 1200, a send would be refused or dropped without a count. ON14 owns datagram discipline.
- Cost S. Risk low: a dropped input is a loss; REDUNDANCY 3 covers it.
- Row: QN6.

### T7 WebTransport datagram limit: silent drops, and certificate validity

- What: in the browser, datagrams have an implementation-defined maximum. An oversize write is discarded silently (the promise resolves). Incoming datagrams can expire, and the queue drops its head on overflow. The spec recommends 64 KiB buffers.
- Source: W3C WebTransport, Candidate Recommendation Snapshot dated 2026-07-30, https://www.w3.org/TR/webtransport/ (verified; spec). MDN https://developer.mozilla.org/en-US/docs/Web/API/WebTransport/WebTransport (verified): serverCertificateHashes must be ECDSA with secp256r1 (no RSA), algorithm sha-256, "validity period of less than 2 weeks", and only with allowPooling false.
- Numbers: maxDatagramSize is implementation-defined; for HTTP/3 it "tracks the estimated path MTU minus an implementation-defined overhead" (verified). Oversize write: silent drop (verified). Certificate: less than 2 weeks (verified). Whether a browser accepts 1200-byte datagrams on our paths: unverified (no browser run here).
- Maps onto: `webtransport.rs` (server, feature webtransport), `browser.rs` (wasm glue), `connect_race.rs`. The cert helper follows WEB9 (scout 04 has the rules).
- Correction: the plan and scout say "14 days". The spec says less than 2 weeks. Use 13 days.
- Cost S. Risk low.
- Row: QN7.

### T8 WebSocket fallback: ordered TCP, send latest only

- What: WebSocket is an independent TCP-based protocol that delivers fragments in the order sent. On a lossy path one stalled packet holds later messages behind it (head-of-line blocking). Verified at spec level only: RFC 6455 says the protocol is TCP-based and orders fragments. The size of a stall is unverified here; scout 04 reads a comparison page on the same point (secondary).
- Source: https://www.rfc-editor.org/rfc/rfc6455 (verified; spec). Default ports: ws 80, wss 443 (verified).
- Numbers: stall size: unverified.
- Maps onto: `websocket.rs` (wire plus native server), `connect_race.rs` (first success wins). Rule: on the WebSocket leg the sender writes only the newest snapshot each tick, never a backlog; a stale snapshot is dropped by sequence (see T4).
- Cost S. Risk none: presentation drops are harmless and inputs are redundant.
- Row: QN8.

### T9 Browser timer throttling and hidden tabs

- What: Chrome 88 intensive throttling applies when a page has been hidden over 5 minutes, its chain count is 5 or more, it has been silent for at least 30 s, and no WebRTC is in use. Intensive timers are checked once a minute. Standard throttling aligns timers to 1 s. A page that made sound in the last 30 s gets minimal throttling (4 ms floor). WebRTC use exempts a page.
- Source: https://developer.chrome.com/blog/timer-throttling-in-chrome-88 (verified; Google doc).
- Numbers (verified): 5 min hidden, chain count 5, 30 s silence, 1 min intensive check, 1 s standard alignment, 4 ms minimal floor. WebSocket exemption: unverified (the article names only a test flag). A hidden tab stopping requestAnimationFrame: unverified (not fetched).
- Maps onto: the browser client loop in `browser.rs`. A hidden tab's timers may fire rarely, so the net pump must not depend on a timer for its tick. A hidden or stalled duel should end in a recorded drop and rejoin (`heal.rs`; reconnect target under 5 s in online.md).
- Cost M. Risk medium: the pause must be a tape event (heal.rs records heal in the tape), never a wall-clock decision inside the sim.
- Row: QN9.

### T10 Per-client visibility filter, no-op at 2 fighters

- What: a server-side filter decides which entities each connection receives. bevy_replicon's README lists "control over client visibility of entities and components" (verified). Its filter API was not fetched, so the mechanism is unverified.
- Source: https://github.com/simgine/bevy_replicon (Apache-2.0 OR MIT, verified; server-authoritative replication crate).
- Numbers: none verified from the filter. Ours (`interest.rs`): MAX_INTEREST_ENTITIES 1024, MAX_CONNECTIONS 64, STARVE_TICKS 30.
- Maps onto: `interest.rs` ConnectionInterest::select (read-only for MP rows). The plan says `session.rs` and `scale.rs` do not call it yet (ON29 closes that seam). MP21 and ON29 own the wiring.
- Cost S for the no-op proof. Risk high if the filter changes any byte on a 2-fighter duel: the golden `d57b5c8ae3096614` must stay unchanged.
- Row: QN10.

### T11 Checksum desync detection and a SyncTest self-check

- What: peers compare a state checksum on confirmed frames. A SyncTest session re-simulates the last N frames each frame and compares checksums, which catches non-determinism on one machine. GGRS's DesyncDetection compares checksums between peers (verified). Coherence hashes state with ComputeHash and compares it across clients (verified). Fiedler: determinism is not guaranteed across compilers, OSes or architectures (verified).
- Source: https://docs.rs/ggrs (verified: DesyncDetection, SyncTestSession, MIT or Apache-2.0). https://docs.coherence.io/manual/advanced-topics/competitive-games/determinism-prediction-rollback (verified: determinism list; no interval).
- Numbers: GGRS check interval and default check distance: unverified. A bevy_ggrs example uses check distance 7 (search snippet, unverified).
- Maps onto: `arena/src/lib.rs` checkpoint hash; checkpoint hash every SNAPSHOT_EVERY_TICKS 6 ticks (`interest.rs:27`); AH4 heal; the plan's MP27 every-tick sync test. QN11 adds a check distance at the played-bout level.
- Cost S. Risk high by definition: any mismatch is a bug, never a tolerance.
- Row: QN11.

### T12 Lag-compensation window with bounded claims (cheat bounds)

- What: a melee hit is judged against the attacker's view time, clamped to a window in ticks. A claim outside the window is refused and counted. Rollback (T1) means the duel needs no rewind; the siege uses snapshots and needs this.
- Source: Lightyear (verified): lag compensation lets predicted entities interact with interpolated ones, mainly for FPS. Valve wiki https://developer.valvesoftware.com/wiki/Lag_Compensation returned 403 (unverified). A search snippet says sv_maxunlag defaults to and maxes at 1 second, and cl_interp_ratio 3 or 4 protects against one or two dropped packets (unverified, snippet only).
- Numbers: plan steals note (unverified here): window = render cursor 8 ticks + one-way latency 4.5 ticks, about 12.5 ticks at 150 ms RTT. The plan's 5-tick cap is stale per the status note; the last code used a 16-tick cap (`e2eab820`, per plan).
- Cheat bounds: the window cap limits how far back a client can claim. Rate and range checks were ON5 (cut). No anti-cheat source was fetched for this file. The claim that a window cap defeats rewind abuse is design reasoning, unverified.
- Maps onto: `lagcomp.rs` (removed in `3151bd46`; restore with `git show 3151bd46^:crates/net/src/lagcomp.rs`). Plan rows MP5 and MP20. `cargo test -p engine-net lagcomp::` matches 0 tests (plan status 2026-10-17).
- Cost M to L. Risk medium: window and verdict go into HitReceipt as integer ticks; the hit is an authority event recorded in the tape.
- Row: QN12.

## 3. Corrections to our own text

1. Certificate validity: the plan and scout say "14 days". The W3C spec and MDN say "less than 2 weeks" (verified). Use 13 days.
2. Rollback depth: GGPO's table is one-way transit (5-150 ms = 1-9 frames, verified). The plan writes "150 ms RTT is 9 ticks"; it should say which of one-way or RTT it means. 150 ms RTT is 75 ms one-way, about 4.5 ticks, as the plan's steals note already uses.
3. Interpolation: the 100 ms default is one snapshot interval at 10 Hz. Fiedler's rule gives about 300 ms at 10 pps and 2-5 percent loss (T4).
4. QUIC: the live datagram limit is not read (grep). T6.
5. Browser: oversize WebTransport datagrams are dropped silently (W3C). T7.
6. Plan donor "backroll-rs" (XOR and RLE input compression): the URL I tried, https://github.com/HeroicKatana/backroll-rs, returned 404. Repo location unverified. Not used here.
7. `lib.rs:108` `Rollback` stub versus `rollback.rs`. QN1.

## 4. Proposed board rows

Format from the board: F2P is a concrete test that fails today; P2P is what must stay green. The QN prefix is unused (grep 2026-10-17).

- [ ] QN1 net: one rollback path. Diff: regular. Tier B. Size S. Owns `crates/net/src/lib.rs` (the stub) and `crates/net/src/rollback.rs`. Accept: a grep receipt names every non-test caller of `engine_net::Rollback`; the stub is deleted only when there are none; duel hash x2. F2P: a grep gate that fails while a non-test caller of the stub exists. P2P: `cargo test -p engine-net --lib rollback` green. needs: MP2.
- [ ] QN2 net: input-delay sweep receipt. Diff: regular. Tier B. Size S. Owns `crates/net/src/delay.rs` (test and receipt line). Accept: a sweep of INPUT_DELAY_TICKS 0..=8 over the link.rs fault profiles (0, 2 and 5 percent loss, 30 ms jitter) posts misprediction rate and resim count per setting; the default is the smallest delay with no resim beyond 9 ticks at 2 percent loss; the delay sits in MatchSpec and changing it bumps the wire version. F2P: the receipt does not exist today. P2P: delay.rs tests and the golden unchanged. needs: MP22.
- [ ] QN3 net: acked-baseline fallback. Diff: regular. Tier B. Size S. Owns `crates/net/src/session.rs` (encode_delta test). Accept: when no ack arrives for N snapshots the sender falls back to a full snapshot; a delta is never coded against an unacknowledged baseline; a test drops all acks for 10 snapshots and asserts full-snapshot bytes; hash x2. F2P: the 10-drop test fails if the sender keeps coding against the old baseline. P2P: MP2 priority-delta tests and the 1200-byte assert. needs: MP2.
- [ ] QN4 net: interpolation delay from loss data. Diff: regular. Tier A. Size S. Owns `crates/net/src/delta.rs` (default constant and receipt), `crates/net/src/interp.rs` test. Accept: at 2 and 5 percent loss (link.rs profiles), the share of render samples with no bracketing snapshot is posted for delays 100, 150, 200 and 300 ms; the default becomes the smallest value with zero misses at 2 percent loss; delta bytes unchanged. F2P: the test fails with the 100 ms default at 5 percent loss. P2P: snapshot tests and golden unchanged. needs: MP6, MP17.
- [ ] QN5 net: channel contract on the QUIC path. Diff: regular. Tier B. Size S. Owns `crates/net/src/quic.rs` (CHANNEL_TABLE and its send callers). Accept: every send names a CHANNEL_TABLE entry; an unmapped kind returns an error and is counted, never sent on a default path. F2P: a test sends an unmapped kind and expects refusal. P2P: MP8 channel tests. needs: MP8, ON1.
- [ ] QN6 net: QUIC datagram size gate. Diff: regular. Tier A. Size S. Owns the quinn backend file under the `quic` feature in `crates/net` and its test. Accept: each datagram send reads the live max_datagram_size(); a payload above it is refused as OverBudget and counted, never sent; a test with a simulated limit of 1100 refuses a 1200-byte payload. F2P: the 1100-limit test fails today (no caller of max_datagram_size in `crates/net/src`). P2P: the 1200-byte assert on every encode path. needs: ON1, ON14.
- [ ] QN7 net: browser datagram gate and cert validity. Diff: regular. Tier B. Size S. Owns `crates/net/src/browser.rs` and the cert helper test (feature webtransport). Accept: a 1201-byte payload is refused before the write and counted; the cert helper refuses validity of 14 days or more and accepts 13; the receipt shows both counts. F2P: the 1201-byte test fails today (the write would be dropped silently). P2P: WEB9 cert tests. needs: WEB9.
- [ ] QN8 net: WebSocket leg sends latest only. Diff: regular. Tier B. Size S. Owns `crates/net/src/websocket.rs` and its test. Accept: with the writer stalled 300 ms, on resume only the newest snapshot is written and the skipped count is printed; no backlog; hash x2 on resume. F2P: a stalled-writer test that counts queued snapshots fails today (no such count exists). P2P: websocket wire tests. needs: WEB9, MP17.
- [ ] QN9 net: background-tab policy. Diff: regular. Tier B. Size M. Owns `crates/net/src/browser.rs` and the `heal.rs` test. Accept: a simulated 6 s stall with zero timer ticks (simulated, not real throttling, which is unverified in CI) produces a tape-recorded drop and rejoin with checkpoint equality x2, and the hidden duration appears in the receipt. F2P: no hidden-stall path exists today. P2P: AH4 heal tests. needs: WEB9, AH4.
- [ ] QN10 net: interest no-op on the duel. Diff: regular. Tier B. Size S. Owns the call site in `crates/net/src/session.rs` and a test beside `interest.rs`. Accept: with 2 fighters, ConnectionInterest::select returns the full set and the encoded bytes equal the no-interest encoding; golden `d57b5c8ae3096614` unchanged x2. F2P: the call-site test fails until ON29 wires select. P2P: interest.rs tests green. needs: ON29.
- [ ] QN11 net: SyncTest check on the played bout. Diff: good. Tier A. Size S. Owns `crates/net/src/played.rs` (test and receipt line). Accept: a sync test with check distance 7 (GGRS style; default unverified) over three tapes under the link fault profile reports zero checksum mismatches, x2; a mismatch names the first tick. F2P: no sync-test receipt exists today. P2P: MP1 played tests and the golden. needs: MP1.
- [ ] QN12 net: restore lag compensation with a caller. Diff: good. Tier A. Size M. Owns `crates/net/src/lagcomp.rs` (restored from `3151bd46^`), its caller in `played.rs`, and a test. Accept: fair hits under 80 ms scripted ping; a claim outside the window is refused and counted; the window is an integer tick count in every HitReceipt; a grep gate finds at least one non-test caller; hash x2. F2P: `cargo test -p engine-net lagcomp::` matches 0 tests today (plan status 2026-10-17). P2P: MP1 played tests and the golden. needs: MP6, MP20 (owner choice (a) in the plan status line).

## 5. What this file does not settle

- GGRS and GGPO defaults (prediction window, desync interval) are unverified; the docs did not state them.
- Valve lag-compensation numbers are unverified (wiki 403; snippet only).
- Fiedler's licence is unstated; the scene numbers are his, not ours.
- Browser behaviour (datagram sizes on real browsers, hidden-tab rAF, WebSocket exemption from throttling) needs a run on the owner's PC or the WEB4 runner, not this research.
