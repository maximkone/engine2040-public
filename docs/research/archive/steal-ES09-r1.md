# RES-STEAL ES09 r1 (engines seat) — AmbientRun/Ambient network shapes, std-only reimplementation

Seat: engines. `node tools/steal-next.mjs --seat engines --n 2` ranked ES09 first
(Online, gap 90, last read 2026-09-29); claimed via `node tools/steal-next.mjs --claim ES09`
(ES09 reads 2026-09-30). Frontier list checked first: `frontier.mjs spike` + `next`
show GAP Learned-physics/GNS already claimed 2026-09-30T07:00Z by researcher-wild-r1,
so this run notes first-free topic COLD-Physical-and-procedural-sound-synthesis in its
claim line and stays on its seated donor (no audio implementation read in Ambient this
run; no sound card proposed here — audio seat owns it).
One narrow GitHub code search (0 hits), then known-repo `get_file_contents` reads live.
Reimplement-only, zero bytes vendored. Pillar: FOCUS Now speed (pillar 3).

Donor: AmbientRun/Ambient @ ef44a742ef8e1467a6b7485e61f9e28095fc8764.
License: MIT + Apache-2.0, read live 2026-09-30 (LICENSE-MIT SHA 9cf106272ac3b56b0c4c80218e8fc10a664ca5f4;
LICENSE-APACHE SHA d9a10c0d8e868ebf8da0b3dc95bb0be634c34bfe). Default branch main
(prior ref-main reads live; directory listing SHAs below match the pinned rev).
Donor files read live this run (all `crates/network/src/`):
- client.rs SHA 04195c69979f681cb2220ed6f76cf664f9a336a4: `ClientState` transport+rpc_registry+user_id+game_state
  (:~110), `Bi/Uni/DatagramHandlers = HashMap<u32, ...>` (:~70), `NetworkStats`
  latency_ms/bytes_sent/bytes_received/packets_sent/packets_lost + loss ratio (:~NetworkStats),
  `GameClientWorld` render-target Element (:~GameClientWorld).
- connection.rs SHA 43c3d6a5ce10000dbd6d0c2c911edf41c8efad7c: `trait Connection`
  open_uni/open_bi/accept_uni/accept_bi/read_datagram/send_datagram (:6-14), quinn impl (:16-34).
- hooks.rs SHA cb1409d180cd5d1095ee13e8cde33e139f1bc021: `use_remote_component`
  content_version gate — refresh local value only when
  `get_component_content_version != cached` (:~50-75); `use_remote_components`
  changed()+despawned() query loops with per-entity update closures (:~80-140).

Vision Online `Steal next` piece: confirmed-frame discard + requested-frame eviction guard
(ggrs input_queue shape), home crates/net/src/rollback.rs P2pSession history/window.
Ambient has no input_queue; the analogous discard/evict/version shapes read here are the
version-gated refresh (hooks.rs) and per-connection loss/byte counters (client.rs NetworkStats).

## Baselines in this repo (read in code)

- crates/net/src/rollback.rs:26 MAX_ROLLBACK 8; :165 history Vec<u64> unbounded push :293,
  window-only evict :294-296 (window len capped MAX_ROLLBACK+1, hash tape never evicted).
- crates/net/src/netcode.rs:7 MAX_ROLLBACK 8; :96 history Vec<u64> full-length; :212 push;
  :214 frames windowed MAX_ROLLBACK+1.
- crates/net/src/replicate.rs:211-216 ReplicationServer tick/world/history VecDeque/acked BTreeMap;
  :253-255 bounded history push_back + pop_front over MAX_HISTORY; :1300 evicted history must resync full.
- crates/net/src/interest.rs:25 STARVE_TICKS 30; :29 ON3_CAP_BPS 32000; per-connection
  Near/Far/Combat/Culled + 256/128kbps caps (vision-r8); no per-connection loss/byte counters
  (grep packets_lost/bytes_sent nil in crates/net outside tests).
- crates/net/src/session.rs: INPUT_DELAY 2, 1200B caps; quic.rs 1200B datagram cap + 64KB rx buffer.

## CARD 1 (best-first)

- Idea: version-gated confirmed-slot refresh for replication diffs (Ambient hooks.rs
  content_version-gate shape, std-only reimplementation).
- License: MIT + Apache-2.0 (SHAs above). PORT(AmbientRun/Ambient@ef44a742:crates/network/src/hooks.rs).
- EVIDENCE: donor hooks.rs `use_remote_component` version-compare-then-set; ours
  replicate.rs:211-216 acked/history cursors re-encode every unacked slot every tick with no
  per-slot confirmed-version cache; ggrs discard_confirmed_frames (vision Online Steal next)
  is the discard side, this is the refresh side (complementary, not dupe of ON32 byte-dedup
  which caches last bytes, nor ON37 empty-guard which skips whole empty diffs).
- Home: crates/net/src/replicate.rs (per-slot confirmed-version map beside acked/history).
- Plan prefix + bar: ON (ON40 proposed); moves VISION Online 64-client 128kbps + zero-desync
  toward Finish C Online bar; consumer ON4 64-bot soak + ON9 siege; FOCUS Now pillar 3 speed.
- Spike: 64-bot idle-vs-combat fixture (600 ticks, seed 29; all-idle vs 8-combat-56-idle):
  metric encodes-built share + bytes-per-client + hash x2 (BTreeMap order, 1200B cap held,
  RESYNC_EVERY 16 held). Baseline to beat: current always-encode (0% skipped) at identical
  snapshots. Number: >=30% slot-encodes skipped in all-idle leg with byte-identical snapshots
  and 0 starve over STARVE_TICKS 30.
- Kill: any changed slot skipped once, any snapshot byte mismatch at identical inputs,
  any starve >30, any hash drift x2, any float/tokio/Element/quinn/HashMap in tick or hashed state.
- Effort S; std-only u32 versions + BTreeMap; AI-agnostic (same MCP door); Windows O5 path intact
  (no renderer/IO in tick); determinism golden x2 required (tick-adjacent encode path).

## CARD 2

- Idea: per-connection loss/byte counters for interest/starve decisions (Ambient client.rs
  NetworkStats shape, std-only u64 reimplementation).
- License: MIT + Apache-2.0 (SHAs above). PORT(AmbientRun/Ambient@ef44a742:crates/network/src/client.rs).
- EVIDENCE: donor NetworkStats latency_ms/bytes_sent/bytes_received/packets_sent/packets_lost +
  loss-ratio Display; ours interest.rs:25/29 STARVE_TICKS + ON3_CAP_BPS with no per-connection
  loss/byte counters (grep nil) — starve is tick-age only, caps are static.
- Home: crates/net/src/interest.rs (counters beside per-connection state; wire into starve/cap decisions).
- Plan prefix + bar: ON (ON41 proposed); moves VISION Online per-connection relevancy toward
  Finish C Online bar; consumer ON4 soak + ON3 caps; FOCUS Now pillar 3 speed.
- Spike: 64-bot WAN soak matrix (150ms RTT/30ms jitter/2% loss+reorder, 600 ticks seed 29):
  metric counter accuracy (loss ratio within 0.5pp of injected, bytes within 1%) + starve_max +
  hash x2. Baseline to beat: tick-age-only starve with static caps. Number: loss-aware
  downgrade fires on the 2%-loss leg with starve_max <= 30 and 0 false downgrades on the clean leg.
- Kill: any counter mismatch over tolerance, any false downgrade on clean leg, any starve >30
  caused by the counters, any hash drift, any float/async in tick.
- Effort S; std-only u64 counters; AI-agnostic; O5 intact; determinism x2 (counters are
  observations, never hashed into sim state — wire as sideline, prove with digest equality).

## REJECTS (with reason)

- REJECT: vendoring Ambient quinn Connection trait / BiStreamHandlers HashMap / Element
  GameClientWorld as-is (client.rs HashMap handlers + Element, connection.rs quinn types).
  Guard fail: tokio/quinn async + parking_lot + Element + HashMap in hashed paths violate
  std-only tick + BTreeMap-order determinism + deny minimal-deps; HASH-LAW golden x2.
  Re-open never as bytes; shape-only std reimplementation only (as in cards above).
- REJECT: Ambient master-client election / WASM guest / auto-sync package world as-is.
  Already rejected with evidence (LADDER-ARCHIVE + ON-line rejects 2026-09-29); no new
  implementation read this run changes the verdict; negative result preserved.
- REJECT (this run): learned-GNS world-model card and procedural-sound card from the frontier
  list. No donor implementation of either piece read live in Ambient this run (one-donor rule);
  GAP-learned-physics claimed by wild-r1; sound COLD noted but donor has no audio path read.
  Re-open in the owning seat with a live implementation read + spike fixture, never as analogy.

Unverified: donor default-branch `main` re-verification this run relies on prior ref-main reads
+ matching listing SHAs (no second branch call per budget); all card numbers are NOT RUN
(researcher runs no builds — spikes ride builder/pilot with `cargo test -p engine-net`).
