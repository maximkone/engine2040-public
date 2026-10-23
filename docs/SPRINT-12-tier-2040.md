### 2040 tier (operator 2026-09-24, Finish C; dispatch after Finish line rows)

Look (RT, VG, VX), online (ON) and world (WD) rows from `FOCUS.md` and
`VISUAL-TARGET.md` V5. Presentation rows never feed the tick; golden
`d57b5c8ae3096614` stays x2 on every tier. Patterns come from papers and
public talks; engine source from Unreal, Unity or Flax is never read.

Frontier spikes (FU) from `docs/plan/futures.md` (inbox S36): one
cpu-light test per GAP/COLD topic in the owning crate, bytes x2,
golden unchanged; kill bars are gates.


- [ ] RT7 render: path-traced reference — progressive, same materials and
  lights; the ground truth for Q captures and photo mode. Tier C. Size L.
  Owns `crates/render/src/pathtrace.rs`. Accept: converged stills of the
  material chart; RT-tier stills within perceptual tolerance of it.
  Deep 3D pass 2026-09-25: fixtures chart, night (24 of 250 torches) and tunnel; seeds 7 and 17 at 64 to 4,096 spp; converged when 1,024 vs 4,096 spp FLIP is at most 0.02; an ablation with NEE off must score clearly worse on night and tunnel, else the reference is rejected. needs: GX8, RT1, RL6.
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
  Correction 2026-10-21: duplicate - PW11 owns this work now; PW11's row says it closes RT7 and RT13. This row is not ticked and is not built on its own; retire it when PW11 lands.
- [x] ON14 net: datagram discipline — never fragment past 1,200 bytes
  (`OverBudget` drops and counts), far and idle poses shed first under
  congestion with 2-3 tick expiry, inputs and combat sent redundantly and
  never expired (RFC 9221 datagram rules). Tier C. Size M. Owns
  `crates/net/src/wan_discipline.rs`. Accept: 64 bots plus AI at 150 ms,
  30 ms jitter, 2% loss and reorder converge; no combat entity starved over
  500 ms; at most 128 kbps per client; hash x2. needs: ON1, ON3, ON4, ON5. Landed ad73685c. cargo test -p engine-net --no-fail-fast: 7 wan_discipline tests green (discipline_converges_the_siege_wan_matrix_x2, discipline_refuses_an_oversize_batch_and_counts_it, discipline_sheds_far_then_idle_before_combat, discipline_inputs_and_combat_repeat_once, discipline_replaces_stale_state_before_a_new_batch, discipline_report_bars_fail_loudly, discipline_refuses_bad_shapes_loudly) gh run 39190753182; clippy 39190753056. Measured: max_datagram=1200 (cap 1200), combat starve_max=6 ticks (bar 30), kbps_x100_max=9600 (cap 12800), submitted=7041182 written=3072000, superseded=3142446 shed=1442250 expired=768, x2 identical.
- [x] ON15 net: per-connection scope gate — room membership + per-user scope Progress 2026-10-17: room membership scope (entity_room, room_in_scope, set_rooms) and interest_64_room_converges (64 viewers, kbps under 12,800 x100 cap, starve under 30 ticks, x2 deterministic) passed in `cargo test -p engine-net --lib` 258 passed, gh run 39056801523 (40def1e2); no assertion of the 3x saving versus broadcast, and no production caller (ON29).
  predicate over `aoi.rs` feeding the ON3 priority queue (naia rooms/scopes,
  MIT OR Apache-2.0, patterns only; naia has no rate-LOD of its own).
  Tier C. Size M. Owns `crates/net/src/interest.rs`. Accept: 64-bot soak
  shows 3x or better saving vs broadcast at 128 kbps/client or less, no
  combat entity starved over 500 ms, zero desync. needs: ON3, MP2. Landed a695c4b9. interest_64_bot_soak_beats_broadcast_3x asserts saving >= 3x vs broadcast (saving_x100 >= 300), kbps_x100 <= 12800/client, combat word within the 30-tick (500 ms) bar and starve_max <= 30 for every viewer, x2 bit-identical and a delta-codec round-trip; cargo test -p engine-net --lib interest: 20 passed, gh run 39192165664; clippy 39192416037. Production call site stays ON29.
- [x] ON16 net: token handshake + soak harness — encrypted short-lived join
  tokens and a replay window feeding `wan_soak.rs` per-fault reports
  (netcode.io STANDARD + soak.c, BSD-3-Clause, patterns reimplemented in
  Rust, no C vendored). Tier C. Size M. Owns `crates/net/src/wan_soak.rs`.
  Accept: ON4 matrix at 16-32 converges on every checkpoint with a
  per-fault report; seeded rejoin converges in 5 s or less. needs: ON1, ON2. Landed 1bb9ecfa. run_matrix_sized/run_fault_sized soak the 5-fault matrix over every session 16..=32 and every entry holds the clean tape checkpoint (wan_soak_matrix_sized_16_to_32_holds_every_checkpoint, wan_soak_matrix_sized_repeats_identically_x2); the seeded rejoin converges in 9 ticks (1 transit) because the 32-generation window covers the 2 s outage (wan_rejoin_seeded_converges_in_five_seconds, wan_rejoin_past_the_window_blows_the_bar_loudly); the short-lived join token (30 s TTL, relay challenge round, deny-by-default wan_join_token_is_short_lived_and_refuses_deny_by_default) and the replay window (wan_replay_window_refuses_duplicates_and_stale_sequences, posted as FaultReport.replays_refused per fault) are the handshake half; cargo test -p engine-net --no-fail-fast 357 lib + 44 it green, gh run 39195136954; clippy 39194989098; engine-proof --no-fail-fast green 39195384569; rust-quality --owned PASS. Token confidentiality rides the ON1 transport session; token-level AEAD remains the RC15 dependency decision (docs/plan/2026-10-19-owner-queue.md), so no cipher was hand-rolled here.
- [x] ON17 net: per-class resend/memory discipline — unidirectional channel Landed ad73685c, folded into ON14 per this row's own clause (same owned file, same acceptance). Re-proven at HEAD 48f5a315: cargo test -p engine-net --lib wan_discipline 7 passed, gh run 39206953257, measured max_datagram=1200 (cap 1200), combat starve_max=6 ticks (bar 30), kbps_x100_max=9600 (cap 12800), x2 identical; cargo test -p engine-net --no-fail-fast 357 lib + 44 it green, gh run 39206679767, including the ON4 matrix cells (wan_fault_matrix_checkpoint_equality_per_cell) and the siege matrix half (discipline_converges_the_siege_wan_matrix_x2). Resend timers are Class::redundancy and shed caps are Class::expiry_ticks (near 6, far 2, idle 3) in `crates/net/src/wan_discipline.rs`; the queue is still a proof driver with no production caller, the same deferral ON14 recorded.
  configs with resend timers and shed caps behind the Transport trait
  (renet channel shapes, MIT OR Apache-2.0, reimplemented; may fold into
  ON14 acceptance instead of landing separately). Tier C. Size M. Owns
  `crates/net/src/wan_discipline.rs`. Accept: ON4 matrix converges with
  zero   combat starvation, 128 kbps/client or less, hash x2. needs: ON1,
  ON3, ON4.
- [x] ON18 net: version-stamped WAN resync — CC-OPI (arXiv 2609.19208v1)
  version-based sync + emergency pool reimplemented std-only in wan_soak
  as an alternative to the AH4 path (spike only, no codec change).
  Tier B. Size S. Owns `crates/net/src/wan_soak.rs`. Accept: ON4 matrix
  at 16-32 converges on every checkpoint with per-fault converge-time +
  byte-overhead report; overhead over 10 percent or any miss kills it
  with numbers. needs: ON1, ON3. Landed f563cb6c. wan_version_resync_matrix_16_to_32_posts_converge_and_overhead + wan_version_resync_matches_the_ah4_resync_ledger + wan_version_resync_kill_gate_names_numbers + wan_version_stamp_and_pool_are_bounded green: cargo test -p engine-net --no-fail-fast 361 lib + 44 it, gh run 39211468371; clippy 39211792217; rust-quality --owned PASS. ON4 matrix seed 7, 120 ticks, every size 16-32: every fault holds the clean tape checkpoint, misses 0, one generation 552 B. Overhead over the AH4 resync bytes: latency/jitter/loss 0% (no urgent resync; accepted), reorder 17.4-21.0%, disconnect 30.07% (reserve = one 1200 B datagram charged once per armed tape, plus a 12 B stamp and a 4 B pool ticket per resync); converge reorder 5 vs AH4 10 ticks, disconnect 0 vs 0. Verdict: KILLED by the spike's own 10% bar with numbers, so the version stamps and emergency pool stay spike code (no production path, no codec change).
- [x] ON19 net: P2P-sync negative gate — CRDT P2P VR (arXiv 2503.17826v1)
  evaluated and rejected for combat state (non-Byzantine assumption vs
  the ON5 cheat model). Tier B. Size S. Owns a negative-test block in
  `crates/net/src/relay.rs` tests. Accept: test asserts combat snapshots
  never merge peer-to-peer and every state change carries authority +
  tape; a P2P combat path fails the gate. needs: ON1, ON5. Landed f36bba7.
  `CombatSyncGate` in crates/net/src/relay.rs applies a change only for the
  match's own authority with a named tape row; a peer author, a missing tape
  row, a foreign authority and a held-row rewrite refuse with a typed
  `P2pRefusal`. combat_snapshots_never_merge_peer_to_peer +
  every_combat_state_change_carries_authority_and_tape +
  a_p2p_combat_path_fails_the_gate green: cargo test -p engine-net --lib relay
  21 passed, gh run 39212991775; --no-fail-fast lib + 44 it green, gh run
  39213607190; clippy 39213334777; rust-quality --owned PASS; no sim seam
  changed so the frozen hashes are untouched.
- [x] ON20 net: delta-maintained scope views — DBSP section-4 Z-set idea
  (arXiv 2203.16684v1, mechanism only, reimplemented std-only, no Feldera
  dep) for ON15 room/scope predicates over aoi.rs. Tier B. Size S. Owns
  `crates/net/src/interest.rs` scope-view block + `fixtures/scope-delta/`.
  Accept: 100/100 equality with full rebuild on the seeded trace,
  one-tick p95 within the ON10 tick budget at 5x or better median
  speedup, else killed with numbers. needs: ON3. Landed 7bff123c. `ScopeView` + `ScopeDelta` in the interest.rs scope-view block: the 0/1-weight Z-set view of the room gate is maintained from the mask delta alone (a view word is eight room lanes, so a flipped room costs one lane per word instead of a roster scan; fixed size, zero allocation, weight carried by the delta counts), over `fixtures/scope-delta/trace.txt` (seed 2040, 100 steps) and its owner generator. Proof: `cargo test -p engine-net --lib scope` 4 passed 1 ignored (gh run 39220830854) — scope_view_matches_full_rebuild_100x is 100/100 steps equal to the full rebuild on view, digest, weight, mask and the entered/left ids against the true set difference, plus scope_trace_fixture_is_the_seeded_walk, scope_view_lanes_match_the_room_word and scope_view_rejects_bad_shapes_loudly; the timing proof `cargo test -p engine-net --release --lib scope -- --ignored --nocapture --test-threads=1` PASS (run 39219795787): one_tick_us p50 0.030 p95 0.111 inside the 4 ms siege tick budget, delta_block_us_step 0.159 vs rebuild_block_us_step 0.935, median_speedup_x100 587 (min 580, max 688) over the 500 bar across 100 steps at len 1024 with 19 change steps; whole lib 368 passed 0 failed (39220387502). clippy PASS (39217730906); the later clippy runs 39221342851 and 39222113275 are red only in another session's uncommitted crates/net/src/rollback.rs:808 (`confirm_frame_wan` dead code, test callers only), not this path. rust-quality --owned PASS, workspace-check PASS. No sim seam changed: no caller outside the module until the session packets wire it, so no frozen hash moves.
- [x] ON21 net: rollback prediction from lightyear — `LinkConditionerConfig` Landed 317e0f8. `LinkConditionerConfig` (latency and jitter each way, derives the local input delay that covers the relay), `InputBuffer` (per-seat authored inputs pressed `delay` ticks ahead, known input wins, a missing seat repeats its last authored input) and the N-seat `SeatSession` rollback plus resim, all in `crates/net/src/rollback.rs` (lightyear patterns only, std-only, no Bevy, no donor bytes). Proof: `node tools/gh-check.mjs cargo test -p engine-net --lib rollback::tests` 30 passed (gh run 39229826655) — the 4-client headless run `four_seat_server_two_frame_delay_holds_rollbacks_under_three` drives a server and four predicting clients over `LoopbackWire` (1 tick each way, 2-frame delay) for 1000 ticks with 0 rollback events, every client tip equals the server truth at every tick, and a delay-0 contrast on the same wire shows ~3000 rollbacks, so the result is not vacuous; the same run repeats byte-identical (hash x2). Also `conditioner_derives_the_delay_that_hides_the_relay`, `input_buffer_presses_delay_ticks_ahead_and_repeats_last`, `input_buffer_evicts_beyond_its_capacity`, `seat_session_resims_one_late_mispredict_to_the_same_tip`, `seat_session_refuses_a_tick_evicted_from_its_window`. golden d57b5c8ae3096614 unchanged: `node tools/gh-check.mjs cargo test -p engine-proof --lib golden_hash_untouched` 1 passed (gh run 39230357326); nothing outside the module calls the new types, so no frozen hash moves. `rustfmt --edition 2021 --check` clean; `node tools/rust-quality.mjs --owned crates/net/src/rollback.rs` PASS 0 ratchet regressions (dup_fns 1, long_fns 3, hot_alloc 1, casts 154 unchanged); module clippy clean (the engine-net clippy run 39230757703 is red only in other sessions' uncommitted prediction.rs and handshake.rs).
  + `InputBuffer` patterns (MIT/Apache-2.0, reimplemented, no Bevy dep).
  Tier C. Size S. Owns `crates/net/src/rollback.rs`. Accept: 4-client
  headless server with 2-frame delay shows <3 rollback events per 1000
  ticks; hash x2. needs: ON1, MP1.
- [x] ON22 net: GGRS input-prediction and desync-detection patterns -
  `InputPredictor` trait + checksum desync ported (MIT/Apache-2.0,
  reimplemented; P2P session model NOT taken). Tier C. Size S. Owns
  `crates/net/src/prediction.rs`. Accept: 3-client local simulation with
  2-frame delay shows <2 mis-predictions per 1000 frames; desync detected
  within 1 tick. needs: ON1, MP1. Landed f409472. `cargo test -p engine-net --lib prediction` 7 passed (gh run 39234118934): three_clients_two_frame_delay_mispredict_under_two_per_thousand (3 clients over `link::LoopbackWire`, one tick each way, delay 2, 1000 frames: 0 mispredictions, 0 per 1000, every client tip equals the authority, hash x2 equal; the delay-0 contrast leg mispredicts 2196 per 1000) and desync_is_detected_within_one_tick (seat 1 perturbed at tick 120: the probe names tick 120, lag 1). Plus detector_names_the_first_divergent_tick_and_keeps_it, detector_checks_only_ticks_on_its_interval, tracker_counts_one_misprediction_and_resets_the_streak, tracker_forgets_a_guess_that_left_its_ring, predictors_repeat_last_and_answer_neutral. clippy -p engine-net --all-targets -D warnings PASS (39233753939); golden `d57b5c8ae3096614` unchanged (`cargo test -p engine-proof --lib golden_hash_untouched`, 39234718412); rust-quality --owned PASS, workspace-check PASS. No sim seam: the module has no caller outside its own tests yet.
- [x] ON23 net: renet_netcode authenticated-transport audit — compare
  `NetcodeServerTransport` token-auth against quinn TLS for ON1 (patterns
  only, no dep). Tier C. Size S. Owns `crates/net/src/transport_audit.rs`.
  Accept: side-by-side connection-setup benchmark posted; quinn wins or
  ties. needs: ON1. Landed 820eccf2. `crates/net/src/transport_audit.rs` measures both setup paths for real: the token exchange through `handshake` (HMAC-SHA256 seals) and a full rustls TLS 1.3 handshake (same ring provider, TLS 1.3 pin and chain verification quinn runs) driven in memory. Posted numbers (gh run 39238435186): token 376 B / 5.3 us CPU, TLS 794 B / 141.7 us CPU, three flights each; the flights tie the link term, and TLS is inside the 5 percent tie margin from about 1.7 ms RTT up, so quinn wins or ties on every internet link (only a sub-millisecond LAN shows the token path ahead, posted in the same test). Tests: compare_ties_within_the_margin_and_names_the_winner, token_auth_setup_is_three_flights_with_exact_bytes, tls_setup_is_three_flights_and_verifies_the_chain (an unknown issuer refuses), connection_setup_benchmark_posts_quinn_wins_or_ties -> `cargo test -p engine-net --features quic,crypto --lib transport_audit` 4 passed, gh run 39240626410; clippy --features quic,crypto --all-targets -D warnings PASS 39239653643; golden d57b5c8ae3096614 unchanged (`cargo test -p engine-proof --lib golden_hash_untouched`, 39239147649); rust-quality --owned PASS (foreign engine-net big_files warn from another session's quic.rs hunk), workspace-check PASS. No new dependency, no sim seam.
- [x] ON24 net: ICE conformance hardening — MESSAGE-INTEGRITY on STUN
  binding from invite-ticket creds, candidate-pair nomination,
  consent-freshness keepalives (str0m is-crate shapes, MIT/Apache,
  reimplemented, no dep). Tier C. Size M. Owns `crates/net/src/relay.rs`
  ICE block. Accept: RFC vector fixtures pass; spoofed bindings refused
  100%; punch-or-fallback still within 300 ticks; hash x2. needs: ON2. Landed 950dcd83. `cargo test -p engine-net --lib relay --features crypto` 27 passed (gh run 39247784105) and the default build 26 passed (39247366025): stun_message_integrity_matches_rfc8489_b1 verifies the RFC 8489 B.1 vector as corrected by verified erratum 6290 (HMAC-SHA256 over the adjusted length, key = SHA-256(username:realm:password)); signed_stun_bindings_refuse_every_spoof refuses 7/7 spoof classes (wrong ticket key, flipped tag, flipped tx, flipped USERNAME, flipped type, unsigned, truncated) and relay_hub_requires_ticket_integrity_on_stun answers only ticket-signed bindings (unsigned and foreign-ticket bindings get no answer, counters refused); nomination_settles_on_the_best_pair pins RFC 8445 pair priority and the final USE-CANDIDATE; consent_keepalives_stay_fresh_and_the_punch_still_falls_back_at_300 keeps the relay fallback at exactly 300 ticks. The ICE block moved to `crates/net/src/relay_ice.rs` and is re-exported from `relay` (owner file 1241 lines, under the 1500 ratchet). clippy -p engine-net --all-targets -D warnings PASS (39248476320) and with --features crypto (39248177776); golden `d57b5c8ae3096614` unchanged, binary run x2 identical; rust-quality --owned PASS; workspace-check PASS. No new dependency.
- [x] ON25 net: TURN allocation discipline — Allocate/Refresh lifetimes,
  CreatePermission-before-forward, ChannelBind multiplexing on the relay
  path (webrtc-rs/turn shapes + coturn oracle, reimplemented, no dep).
  Tier C. Size M. Owns `crates/net/src/relay.rs` allocation block.
  Accept: zero unpermitted or stale-allocation forwards in fixtures; ON4
  fallback drill converges; hash x2. needs: ON2. Landed 577b9122.
  `cargo test -p engine-net --lib relay` 30 passed; ON30 QUIC relay leg 1
  passed; clippy -D warnings PASS; release binary hash `d57b5c8ae3096614` x2.
  Gate is opt-in via `with_turn`; ON27 wires the allocation path.
- [x] ON26 net: subscription-predicate interest — typed per-connection
  range/view/LOD predicates over aoi.rs with change-driven delta push
  (SpacetimeDB subscription idea, BSL-1.1, reimplemented, no donor code).
  Tier C. Size M. Owns `crates/net/src/interest.rs` scope-predicate
  block. Accept: 64-bot soak ≥3x vs broadcast at ≤128 kbps/client, no
  combat entity starved over 500 ms, hash x2. needs: ON3, ON15. Landed
  8595ac0. aoi.rs is gone (NETSLIM-R2 folded it into interest.rs, the owned
  path): `Subscription` carries typed range/view/LOD clauses and hands the
  same clauses to `select` as one `InterestCfg`; `PredicateView` holds the
  clean result set as a fixed bitset and `update()` returns only the
  entered/left row events, so an unchanged tick pushes 0 bytes and a left row
  costs one id tombstone. subscription_predicate_64_soak_beats_broadcast_3x:
  64 viewers x 65 rows x 600 ticks at 10 Hz, saving >= 3x vs broadcast,
  kbps <= 12800/client, combat row and worst starve <= 30 ticks, both arms x2
  bit-identical, ring-crossing walker proves a real delta; plus
  subscription_typed_predicates_split_range_view_and_lod and
  predicate_view_pushes_only_row_changes. cargo test -p engine-net --lib
  interest 27 passed, gh run 39494552479; full lib 430 passed, 39495016461;
  clippy 39495016264. Production call site stays ON29.
- [x] ON29 net: interest production wiring — `session.rs`/`scale.rs`
  call `ConnectionInterest::select` on the live snapshot path with
  the ON3 budgets, keeping the in-test consumer as the oracle. Tier
  B. Size S. Owns call-site hunks in `crates/net/src/session.rs` +
  `scale.rs` (interest API frozen). Accept: 64-bot soak kbps and
  starve numbers equal the in-test oracle x2; no combat entity
  starved over 500ms; hash x2. needs: ON3, ON9. (PLN online,
  reviewed 2026-09-26.) Landed b8876e998. Proof: `cargo test -p engine-net
  --lib live_snapshot -- --nocapture` 2/2 (run 39869460572) — siege arm
  `bytes=3377056 kbps_x100=4221 starve=24 combat_gap=6 digest=548745b1274c7acd`
  equals its oracle, and the live ON3 default `kbps_x100=4703 starve=24
  combat_gap=6 digest=6bac2a312aa41455` equals its oracle, both replay
  bit-identically x2; worst starve 24 ticks = 400 ms (bar 500), combat gap
  6 ticks = 100 ms; 47.03 kbps under the 256 bar, siege 42.21 under its 128;
  `cargo test -p engine-net --lib` 462 passed, 0 failed, 4 ignored (run
  39869186390); clippy all-targets `-D warnings` PASS (run 39869652778);
  `rust-quality --owned` PASS; rustfmt clean. Golden untouched (net only).
  Note: the 1500-production-line cap made `Server::state`/`snapshot_selection`/
  `snapshot_packet` go; the lane's public `state`/`selection`/`packet` in
  `scale.rs` are the consumer surface and session.rs sits at 1499/1500, so the
  next production addition there needs a few lines of headroom first.
- [ ] ON30 net: online two-machine played proof — 2 humans + 6 AI over
  QUIC through the ON2 relay converge mid-fight through a scripted
  drop+rejoin; measured loss/latency distributions posted. Tier A.
  Size M. Owns `crates/net/src/quic.rs` + `relay.rs` proof hunks +
  fixtures (no new transport). Accept: played bout converges with
  full checkpoint equality; punch-or-fallback within 300 ticks; hash
  x2 with the MP1 tape. needs: ON1, ON2, MP1. (PLN online, reviewed
  2026-09-26; loss, jitter and reorder are proven on loopback with a
  fault injector (`crates/net/src/wan_soak.rs`); only the real-NAT
  hole-punch leg needs a second network.)
  progress 2026-10-10: loopback only, Accept not met, row stays open. `net-bout on30` (--features quic) PASS: relay and direct bouts converge with full checkpoint equality. Open: real-NAT hole punch across two networks (no relay bin on HEAD yet); the hash x2 with the MP1 tape leg was not run. cargo test -p engine-net --features quic --lib: 261 pass, 1 red: lobby::tests::loopback_two_peers_form_fight_without_typed_addresses (beacon timeout under load; lobby.rs untouched by this work, not chased).
  progress 2026-10-10 (split-claude): real-NAT runner built. `net_relay [--bind 0.0.0.0:7041]` on a host both networks reach (RelayHub: STUN echo, token register, forward from the registered address only). Machine A: `net_bout on30-serve --relay R:7041` prints an invite code; machine B: `net_bout on30-join --relay R:7041 --code CODE`. Both register, learn the public address, trade candidates, punch (bound 300 ticks), then play WAN bouts x2 over the relay and x2 direct when the punch won; each equals its one-process twin. Receipts in `target/on30-nat` (online-bout, checkpoints, drop-rejoin, distributions, punch, real-net). The real leg rides UDP, not QUIC. Three-process loopback smoke: ON30 NAT PASS. Open: one run across two real networks.
  decision 2026-10-10 (owner delegated to Claude): the real-NAT leg counts over the UDP courier; QUIC through the relay is QR1 (LATER, `docs/SPRINT-18-gaps.md`). ON30 ticks on one run across two real networks with the `target/on30-nat` receipts and the hash x2 with the MP1 tape (operator run: `net_relay` on a reachable host, `on30-serve` and `on30-join` on two networks).
  progress 2026-10-18: the arena window pair (`arena.exe --host` / `--join`) now passes the relay scenario too: `node tools/arena-net.mjs --exe target/release/arena.exe --relay-exe target/release/net_relay.exe` = 8 runs (plain/spear/fault/relay x2), host == guest each, relay run 1 and 2 end on the plain duel's 8bcc8890475af689, spear on 244f2e2c63a267d3, fault equal to plain; receipt `target/arena-net/receipt.json`. Still open for the tick: the operator run across two real networks (the `target/on30-nat` receipts) and the MP1-tape hash x2, plus the two-keyboard LAN play line on the row above. Progress 2026-10-19: operator: two real networks and two humans; exact lines in `docs/plan/2026-10-19-owner-queue.md`.
  BLOCKED: needs two humans on two real networks (operator). Unblocks per docs/plan/2026-10-19-owner-queue.md.
- [x] ON36 net: confirmed-frame discard in P2P rollback — discard Progress 2026-10-17: rollback.rs discard tests queues_hold_128_and_discard_advances_tail, discard_stops_at_predicted_and_clamps_to_frame, loopback_seeded_loss_converges_replay_empty pass in `cargo test -p engine-net --lib` 258 passed, gh run 39056801523; the 64-frame bursty tape with history <= CAP+pending lives in netcode.rs (on40_history_bounded..., e449553e), not in rollback.rs.
  confirmed frames from the P2P session `history` behind a
  requested-frame guard, bounding memory (spike: 64→11 entries,
  512B→88B on the 64-frame bursty-loss tape, fold/tip/digest
  identical x2, 0 spurious refusals). Size S. Owns discard hunks in
  `crates/net/src/rollback.rs` (window/ring shape frozen). Accept:
  `cargo test -p engine-net --lib rollback` green plus the 64-frame
  bursty-loss tape with history ≤ CAP+pending and fold/tip/digest
  identical x2. Consumer: P2P duel + ON4 soak. needs: ON3.
  (Research-merge wave 90, ON36 spike.) Landed e449553e. rollback discard tests queues_hold_128_and_discard_advances_tail, discard_stops_at_predicted_and_clamps_to_frame, discard_keeps_the_unresolved_incorrect_frame, loopback_seeded_loss_converges_replay_empty (net --lib rollback 25 passed run 39101475622) plus netcode on40_history_bounded_cap_plus_pending_bursty_loss (64-frame bursty-loss tape, history <= CAP+pending, fold/tip/digest identical x2) in the full net suite run 39107627991.
- [x] WD7 world: cell publish gate — dual ahead-agent check Landed 1d01a410. `cargo test -p engine-world --lib` filter dual_gate: dual_gate_refuses_broken_seam_with_named_check, dual_gate_refuses_unreachable_island_with_named_check, dual_gate_nine_cell_walk_holds_budgets passed, gh run 39056058112.
  (geometric seam scan + navigational walk) before any cell goes
  live; bad cells refused with the failing check named. Tier C.
  Size S. Owns the gate in `crates/world/src/cells.rs` + fixtures.
  Accept: a cell with a broken seam and a cell with an unreachable
  nav island both refuse before publish; a 9-cell walk holds the
  AS2 budgets. needs: WD1.
- [x] WD8 world: versioned mesh tiles — per-cell NV1 bakes stitched Landed e7e12827 (rebake_mesh_tile live; tiles in a9766af6). `cargo test -p engine-nav --lib` 67 passed incl. apply_breaks_names_exact_dirty_tiles, rebake_bumps_only_its_tile_and_clears, walkers_repath_only_when_their_tile_advances, mesh_tile_rebake_bumps_through_the_registry, wiring_tests::root_mesh_tiles_and_rebake_bump_one_tile, gh run 39056263891. Versions live in TileRegistry, not in the mesh bytes.
  on shared edges with per-tile versions in the mesh bytes, so
  `rebake_mesh_tile` stops returning `Unsupported` and DX1 break
  sets rebake exactly their tiles. Tier C. Size M. Owns tile
  versions in `crates/nav/src/mesh.rs` + `rebake.rs`. Accept: break
  fixtures bump exactly the dirty tiles; untouched tiles keep
  versions; fighters finish then repath on the new version.
  needs: WD1, NV1.
