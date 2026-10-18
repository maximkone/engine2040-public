# 03 Net, web, audio and input donors (steal hunt, 2026-10-18)

Scope: netcode, web/wasm, audio, input. Read-only pass: gh api only (repo metadata, directory listings, raw bodies). No clone, no build, no donor code run, no cargo, no git. Greps over crates/net/src, crates/audio/src and crates/input/src (path given each time).

Skipped as known: docs/plan/steals.md, docs/research/finish-line-2026-10-17/02-github.md, steals-2026-10-11/engine-landscape.md, steals-2026-10-12/physnet.md and audio-ui-matrix.md, web-2026-10-17/01, 04, 05, 07, quality-2026-10-17/02, 03, 04, plan/multiplayer.md and plan/web-and-realism.md.

Stars, license and pushed_at are as printed by gh api on 2026-10-18. Read depth: FILE BODY READ = raw body fetched with gh api in this pass; LISTING ONLY = directory listing, no body.

## Ranked steals (value per effort)

| # | donor repo@path (license, stars, pushed_at) | exact thing to take | our file it lands in | grep proof (pattern -> hits) | gate moved | Value / Effort | read depth |
|---|---|---|---|---|---|---|---|
| 1 | BillyDM/Firewheel@HEAD:crates/firewheel-core/src/dsp/declick.rs (Apache-2.0, 316, 2026-09-05) | Declicker state machine (SettledAt0/1, FadingTo0/1 with frames_left) that reverses mid-fade without a jump; precomputed linear and equal-power tables (DeclickValues, 3 ms default); two-buffer crossfade (dry/wet gain pair) for filter swaps and reverb blends. | crates/audio/src/lib.rs:334-355 (Q24 fade_step, q24_gain) and occlude.rs; new helper beside them | `declick` -> 0 hits in crates/audio/src; frames_left, fading_to, FadingTo, reverse_fade, equal_power, circular -> 0 hits; lib.rs:334-355 is the only voice fade | G9 (0 allocs), G10 (no clicks on WebAudio) | 4 / S | FILE BODY READ (whole file) |
| 2 | Leafwing-Studios/leafwing-input-manager@HEAD:src/clashing_inputs.rs (Apache-2.0, 970, 2026-10-15) | Clash rule: a chord that is a strict subset of another chord clashes; default PrioritizeLongest (only the longest chord fires), PressAll as the other strategy; BasicInputs Simple/Composite/Chord with clashes_with. Port as an integer rule on the held-button bit mask, applied before FighterInput bits are written. | crates/input/src/map.rs (eval; any-of binds :161-182) and fighter.rs:24-30 (button bits) | `clash, prioritize, superset, chord_wins` -> 0 hits in crates/input/src (only "Longest tap" comments, gestures.rs:183 and :244); map.rs:176-182 binds any-of with no subset check | G8 (a tape holds one result per press) | 4 / S | FILE BODY READ (first 9 KB: rules, BasicInputs, clashes_with; resolve_clash not read) |
| 3 | naia-lib/naia@HEAD:shared/src/connection/ack_manager.rs (Apache-2.0, 1182, 2026-10-17; loss_monitor.rs and sequence_buffer.rs named, not opened) | Per-packet sequence index and a 32-bit ack bitfield in every header (bit i = packet last_recv minus i+1 received); sent-packet table resolved by that bitfield into delivered or lost; loss percent from acked and lost counts. Integer throughout. | crates/net/src/session.rs (ack state beside the acked-baseline path at :922) and played.rs (loss feed for the QN2 sweep) | `ack_bits, ack_mask, AckBits, sequence_buffer, last_acked, acked_tick, ack_tick, ack_history` -> 0 hits in crates/net/src; "acked" appears only in comments (session.rs:308, :674, :922) | G8 (QN3 baseline; MP24 loss posts) | 4 / M | FILE BODY READ (whole ack_manager.rs) |
| 4 | simgine/bevy_replicon@HEAD:src/client/confirm_history.rs and src/client/server_mutate_ticks.rs (Apache-2.0, 654, 2026-10-15) | Per-entity 64-bit confirm mask (last tick plus one bit per older tick; ticks older than 64 count as applied); per-tick message count, so a tick is confirmed only when all its messages applied (confirm(tick, count)); a last_confirmed_tick frontier that never goes backwards. | crates/net/src/session.rs (acked-baseline bookkeeping for QN3) and snapshot.rs (confirm mask per field group) | `applied_mask, applied_ticks, confirm_hist, ServerMutate, mutate_tick, all_applied, ConfirmHistory` -> 0 hits in crates/net/src | G8 (QN3 "10 dropped acks" test gets a frontier to assert) | 4 / M | FILE BODY READ (confirm_history.rs first 6 KB; server_mutate_ticks.rs first 5 KB) |
| 5 | naia-lib/naia@HEAD:shared/src/connection/entity_priority.rs and bandwidth_accumulator.rs (Apache-2.0, 1182, 2026-10-17) | (a) Per-entity priority accumulator: add the gain each tick, send highest first, reset to 0 on send, one-shot boost and persistent gain override. (b) Token-bucket send budget: refill from target bytes/s, carry surplus across ticks, one packet of overshoot per tick so one MTU packet always goes. Port in Q8 integers, no Instant. | crates/net/src/interest.rs select (budget prefix :525-582, class order) and scale.rs (per-client budget) | `accumulat, priority_score, boost, urgen` -> 0 hits in crates/net/src; `bucket` -> 25 hits, all histogram buckets (link.rs:105-156, played.rs:63-793, split.rs:299-344), none a token bucket; `surplus, overshoot, carry` -> 0 budget hits (the carry hits are comments on blobs and state) | G8 (ON9 128 kbps bar, interest.rs:33), G9 (no alloc) | 3 / M | FILE BODY READ (entity_priority.rs first 7 KB; bandwidth_accumulator.rs first 4.5 KB) |
| 6 | veloren/veloren@HEAD:network/src/participant.rs, send_mgr lines 236-438 (GPL-3.0, 7629, 2026-10-17) | Per-stream priority (Prio, u8; tests use 7 and 9) plus guaranteed_bandwidth per stream; each tick flushes with the elapsed time as the byte budget; a 0.99/0.01 moving average of measured bandwidth feeds a stats watch. Take the ordering rule as integers; the tokio plumbing stays out. | crates/net/src/quic.rs (CHANNEL_TABLE :83, channel route) and netcode.rs:112-115 (priority table) | `guaranteed, stream_prio, per_stream_prio` -> 0 hits in crates/net/src; PRIORITY_LOCAL..IDLE at scale.rs:88-96 are per-entity classes, not per stream | G8 (MP8: combat state never starves under a 16-fighter flood); G8 kbps receipt | 3 / M | FILE BODY READ (partial: send_mgr only; Prio and Bandwidth types not opened) |
| 7 | cBournhonesque/lightyear@HEAD:crates/replication/replication/src/diff_history.rs (same dir: checkpoint.rs, send.rs) (Apache-2.0, 1190, 2026-10-16) | A diff-history ring that lets the sender code a delta against any tick still held, not only the last acked one (to verify from the body). | crates/net/src/delta.rs (acked baseline) | `diff_history, DiffHistory` -> 0 hits in crates/net/src | G8 (MP9 kbps at 16 fighters) | 2 / M | LISTING ONLY |
| 8 | gschup/ggrs@HEAD:src/sessions/p2p_spectator_session.rs (NOASSERTION, 697, 2026-08-25) | Spectator buffer of confirmed host inputs, replayed at a catch-up speed when behind (to verify from the body). | crates/net/src/rollback.rs (spectator read path, :562-563) plus a new feed | `spectat` -> 4 hits, all in rollback.rs (read path only); `catch.?up, max_frames_behind, frames_behind, catchup_speed` -> hits only in cells.rs:27-28 and 369-430 (cell-stream catch-up, not a spectator) | G8 (MP7 theater; live watch) | 2 / L | LISTING ONLY (src/sessions: builder, p2p_session, p2p_spectator_session, sync_test_session) |
| 9 | UkoeHB/renet2@HEAD:renet2_netcode/src/sockets.rs (+ native_socket.rs, websocket_socket/, webtransport_socket/, memory_socket/) (Apache-2.0, 61, 2026-10-16) | One socket layer under the netcode client and server for native UDP, WebSocket, WebTransport and an in-memory test socket (trait shape to verify from the body). A second opinion on the WEB9 seam. | crates/net/src/quic.rs:189 (Transport trait) and browser.rs | `trait Transport` -> 1 hit (quic.rs:189); webtransport.rs, webtransport/server.rs, websocket/server.rs and browser.rs already exist | G10 (WEB9 race) | 2 / M | LISTING ONLY |

## Already have (grep proof, 2026-10-18)

- ggrs src/network/compression.rs (XOR-chain plus RLE input packing) is in our tree: session.rs:38-40 (module doc), :117-121 (INPUT_COMPRESSED_BIT, RLE tag), :996-1007 (encode_input_compressed), test at :2229. The multiplayer plan's "input compression missing" line is stale.
- ggrs input_queue discard (ON40): netcode.rs, landed e449553e (steals.md).
- Veloren budget-plus-priority shape for cells: cells.rs:7-9 (PORT-IDEA veloren@2dbddc2), :162 (priority order), :27-28 (catch-up bar).
- Starvation: interest.rs:21-27 (STARVE_TICKS 30), :708 (starve-forced leftovers). Per-client byte budget prefix: interest.rs:525-582; kbps bars :28-35.
- Priority classes per entity: scale.rs:88-96; priority table per wire kind: netcode.rs:112-115.
- Transport seam: quic.rs:189 `pub trait Transport` (send_unreliable, send_reliable, recv).
- Browser legs exist in code: webtransport.rs, webtransport/server.rs, websocket/server.rs, browser.rs. The plan text that says "not built" is stale; check the code before any new WEB9 row.
- Spectator read path: rollback.rs:562-563 spectator_hash_at.
- Chords and any-of binds: input/src/map.rs:53-64 (Chord), :161-182 (bind). No clash rule.
- Audio: Q24 steal fade (STEAL_FADE_MS 8, FADE_RESERVE 16) and duck levels at lib.rs:327-355. Music tween bank with one-bar crossfade at dsp.rs:143-152 (music layers only, not voices).
- "bucket" hits in net (link.rs, played.rs, split.rs) are histogram buckets, not budgets.

## Traps

- Determinism: Firewheel's DeclickValues builds its tables with f32 sin. Build them at startup, presentation only; no gain value reaches the tape or the hash. The Q24 path is integer and stays off the hash.
- Naia's accumulator and bucket use f32 and Instant. Port in Q8 integers per tick, no wall clock (interest.rs already forbids one). Break ties by entity index, never HashMap order (naia's entity_priority.rs keys a HashMap).
- Naia's ack table is a HashMap of sent packets. In our port use a fixed ring or BTreeMap so notification order is stable. Loss percent feeds receipts only.
- Replicon compares ticks in wrapping order (is_newer). Our tick arithmetic must match on wrap. Confirm state gates resends only, never the sim.
- Chord clash changes which inputs reach the tape, so it is a v2 bit change (the XX6 rule: the proof scene keeps v1). Resolve in bind-index order so ties are deterministic.
- Veloren's send_mgr uses tokio, Instant::now() and an f32 EMA. Only the ordering rule ports; the IO stays off the fixed tick.
- ggrs spectator: read confirmed ticks only; catch-up changes pacing, never the tape.
- A grep for "bucket" in net hits histogram code, not a token bucket. Do not count it as proof either way.

## Not read

- winit web quirks (pointer lock, canvas focus, keyboard map): no donor body opened; web-2026-10-17/01 covers MDN pointer lock only.
- Touch controls: mobile is CUT in FOCUS; not read.
- cpal webaudio host (cpal 0.18.2 for wasm32, from 07-wasm-probe): listed there, not opened here; WEB7 owns it.
- bevy_ggrs, leafwing action_diff.rs, bevy_web_asset, bevy_ci_testing: not fetched.
- lightyear WebTransport glue, send.rs and checkpoint.rs bodies; ggrs spectator body; renet2 bodies; Veloren Prio and Bandwidth types: listings or partial reads only.
- matchbox source: known from 02-github, not re-read.
- Firewheel mixer and kira: out of scope (slim law: no second mixer, parked.md section 6).
- Gate numbers in the table are the board targets, not measured in this pass.
