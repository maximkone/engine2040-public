# Agent 2 — the browser and the network (hunt 2026-10-21)

Read-only pass. No cargo, no edits to `crates/`, no commits. One file written: this one.
Donors below were checked with `gh api` on 2026-10-21 (repo meta, tree, and file bodies read
live). Evidence grade in every cell: **OBSERVED** (file body read today), **LISTING** (tree or
meta only), **DERIVED** (reasoned from what was read).

## What our tree already has (so I do not propose it again)

Grepped `crates/web`, `crates/net`, `crates/assets`, `crates/render` on 2026-10-21:

- **WEB9/WEB12 browser net is built**: `crates/net/src/browser/{wt,ws,stream,inbox}.rs`, the
  `Transport` trait in `crates/net/src/quic.rs:189`, `connect_race.rs` (WebTransport vs
  WebSocket race), `crates/net/src/webduel.rs`, and the page side `crates/web/src/netplay.rs`
  (`net: transport=<webtransport|websocket> ticks=<n> hash=<hex>`). So A3 is largely **done**.
- **A4 strict WebGPU is built**: `crates/render/src/backend.rs:432-461` reads `E2040_STRICT_WEBGPU`
  and adds `InstanceFlags::STRICT_WEBGPU_COMPLIANCE`; `ci.yml:111` sets it. A4 is **done**
  (only infra missing: a GPU runner).
- **A5 AudioWorklet is built**: `crates/web/src/audio_worklet.rs` (the processor JS is inside the
  wasm as a blob URL), `crates/web/src/browser.rs:170-215` prefers the worklet and falls back to
  `ScriptProcessorNode`. A5 is **done**.
- **A6 tier and warm-up are built**: `crates/render/src/quality.rs`, `crates/web/src/tier_view.rs`
  (`Ladder`, `Cut`, `ProbeRun`, `Drift`, `Pace`), `pipeline_warmup.rs` (WEB13, `pipelines=21`).
  A6 is **done** except QT9 (texture cap from limits).
- **A2 load queue exists natively**: `crates/assets/src/load_queue.rs` (`LoadQueue` with
  `enqueue(path, priority, bytes)`, `pump(max_bytes)`, `cancel`, `unload`, `HandleBudget`),
  `crates/assets/src/stream.rs` (`StreamQueue`, `StreamBudget`), `crates/assets/src/residency.rs`
  (one byte budget for all resident classes, eviction by priority then LRU then id). **What is
  missing is the web half**: the page fetches one whole `.e2bundle`
  (`crates/web/src/content.rs:107-127`), it does not drive the same queue.
- **A1 is partly built**: `crates/web/src/fight.rs` plays the match (`Match::step`, `MATCH_TICKS`),
  `crates/web/src/arena.rs:747-770` fetches the arena kit bundle, `crates/web/src/arena_view.rs`
  has the capture hash line. The cooked **bodies** and the **exchange cues** (WEB16/WEB17) are
  still stand-ins.
- **WEB14 COI is built**: `crates/web/coi-serviceworker.js` + the `#coi` line in `index.html`.
- **QW9 is open**: `pollster::block_on` still appears in `crates/render/src/{aa_scale,baked_gi,ibl,backend}.rs`.

So the live gaps in my lane are: **A1 bodies+cues**, **A2 web half of the load queue**,
**WEB5 the browser/native frame pair**, **ON30 the real two-machine internet run**,
**MP6 the LAN duel**, and **E23 the netcode model decision**.

---

## 1. Ranked steals (best first)

| # | repo (owner/repo) | stars | lic | donor file path | what we take | our crate + row/gate | size | risk |
|---|---|---|---|---|---|---|---|---|
| 1 | dzhng/battlesim | 5 | none found (`gh` says `license: none`, no LICENSE blob) | `web/src/battle/sim/authority.ts` (OBSERVED) | The **worker-is-the-authority** pattern: the sim runs in a Web Worker, publishes one `Float32Array` per tick into a **credit pool** (`PUBLICATION_POOL = 3` `ArrayBuffer`s, transferred zero-copy), and the page returns a *credit* when it has drawn the frame. The pool is the backpressure: when credits run out the authority sets status `waiting-consumer` and stops ticking. Also the **digest per publication** (`digest: b.digest()` next to `tick`) and the **hidden-tab rule** (`hidden` request → status `hidden`, and on unhide `nextTickAt = now` so the gap is dropped, never replayed). | `crates/web` (new `src/worker_authority.rs` + `crates/web/src/fight.rs` moves the tick into the worker) + `crates/net` (the per-tick digest rides the existing snapshot codec). Moves **WEB5** (a browser frame at a named tick, hash-named) and **A1** (the real cooked fight runs in the worker and the page draws it). Gate G10 "a headless Chrome client and a native client end a duel on one digest". | M | Medium-high: it is a restructure of the page's tick loop, and it touches the golden path (the page's own `d57b5c8ae3096614` line must not change). The digest must be the *server* digest, not a second one. Mitigation: keep the tick in `crates/game` unchanged; the worker only *hosts* it. |
| 2 | nithish101/resonance | 0 | MIT | `packages/web/src/lib/frames.ts` + `docs/decisions/0005-seqlock-feature-frames.md` (OBSERVED) | A **seqlock over `SharedArrayBuffer`**: one writer, many readers, nobody waits. Writer does `Atomics.add(seq,1)` (odd = writing), writes the payload, `Atomics.store(seq, even)`. Reader loads the counter, copies, reloads the counter; if it changed, the copy is torn and is discarded (4 tries). The **same class runs over a plain `ArrayBuffer`**, so the postMessage fallback and the shared path share one reader. | `crates/web` (a `src/frame_bus.rs` beside `src/anim.rs`; the tier/probe counters ride the same bus) + `crates/audio` (`crates/audio/src/feed.rs` already has an SPSC ring). Moves **A6** (a real web quality ladder read by the worker) and **WEB14** (the SAB the bus needs is what makes the COI worker worth it). | M | Low-medium: `SharedArrayBuffer` needs cross-origin isolation, which our Pages deploy cannot set (WEB14 note) so the SAB path stays opt-in; the `ArrayBuffer` fallback must be the default. |
| 3 | nithish101/resonance | 0 | MIT | `packages/web/src/audio/shared/ring.ts` + `docs/decisions/0002-pcm-ring-buffer.md` (OBSERVED) | The **consumer-side drop-oldest SPSC ring**: the producer only ever writes its own index and never reads the reader's (so no CAS on the audio thread); the consumer notices the overwrite itself by (a) skipping ahead when more than `capacity - margin` samples are pending and (b) re-reading the write index after the copy and discarding anything that could have been torn. The overrun count goes in a shared header. 32-bit wrapping counters, unsigned arithmetic, plus a float64 absolute index so every sample maps to the audio-context frame it was captured at. | `crates/audio` (`crates/audio/src/feed.rs` is our ring; this is the browser-facing one) + `crates/web` (`src/audio_worklet.rs` stops round-tripping blocks through the page). Moves **A5** hardening (the worklet writes, the mixer reads, no per-block `postMessage`) and **G9** (no per-quantum allocation). | S-M | Low: pure integer index math, no wall clock. Danger: our current worklet already works (page-rendered blocks); this is a better transport, not a fix. Do it only if the worklet shows underruns. |
| 4 | axelromero99/motumbo | 3 | MIT | `web/src/relay.ts` (OBSERVED) | A **reliable-ordered channel on top of an unreliable pub/sub link** (the relay fallback leg): every message gets a seq; every 33 ms the sender re-sends all still-unacked messages plus a **cumulative ACK of the contiguous prefix** it has received; the peer delivers strictly in seq order and drops duplicates. The comment says it plainly: *"lockstep needs every input to arrive — a missing tick blocks the game forever — but qos0 can drop and reorder."* This is exactly our `crates/net/src/relay.rs` hole-punch-fails-so-relay case. | `crates/net` (new `src/relay_rudp.rs`; the relay's forward path is `crates/net/src/relay.rs`, the courier is `src/courier.rs`). Moves **ON30** (the internet path: a punch that fails must not stall the fight) and **ON27** (interest-scoped relay forwarding). | M | Medium: our relay speaks its own framing; a second reliability layer must not double-sequence. Keep it *inside* the relay, never on the QUIC leg. |
| 5 | axelromero99/motumbo | 3 | MIT | `web/src/net.ts` (OBSERVED) | The **invite-code handshake with no server**: the SDP offer is base64'd into a copy-paste code, the guest pastes it and returns an answer code. Plus the **lockstep gate** (tick T advances only when both sides' inputs for T are known, the first `DELAY` ticks are zero) and the **hash cross-check every 60 ticks** (a `desync` flag set the moment the two hashes differ). | `crates/net/src/relay.rs` (`InviteCode` already mints and encodes a 6-byte code) and `crates/net/src/guard.rs`. Moves **ON13** (invite-and-party hardening), **ON30**, and **E23** (the owner decision: the browser can be a *peer* and not only a client). | S | Low: the code shape is small. Risk is scope: our `InviteCode` is 6 bytes, an SDP blob is kilobytes — steal the *shape* (offer/answer code + the hash cross-check), not the SDP. |
| 6 | markclausing/websoccer | 1 | MIT | `src/net/transport.js` (OBSERVED) | **Input delay that adapts to stalls** and a **self-repairing input window**: the last 7 ticks of input ride along on every send, so a lost packet repairs itself and nothing is ever re-requested; the delay grows when `stallWindow > 8` and shrinks after 8 calm seconds, bounded by `minDelay/maxDelay`. The comment names the rule: *"if it is not there anyway, the simulation waits (a stall) instead of guessing; that way the two sides cannot drift apart."* | `crates/net/src/delay.rs` (our delay is a fixed sweep result) and `crates/net/src/rollback.rs`. Moves **MP26** (clock slew and the advantage window) and **ON22** (input prediction). | S | Low: the trick is the trailing 7-tick window and the stall counter; both are integer and fit our `MatchSpec`. |
| 7 | braedonsaunders/voidstrike | 26 | MIT | `src/engine/network/MerkleTree.ts` (OBSERVED) | A **Merkle tree over entity state** so a desync is located in O(log n) instead of a full compare: the tree is grouped (category → group → entity), the root hash is compared first, and only the differing branch is walked; `serializeForNetwork` sends **only hashes per level**, not the tree, and a `MerkleCompareRequest` asks for the entity level of one group. | `crates/net/src/guard.rs` (our desync check) and `crates/net/src/interest.rs` (the same tree shape fits the per-connection scope). Moves **E23** and **MP27** (sync test), and **ON5**-style server checks (which were cut 2026-10-16, so this is a *new* row, not a revival). | M | Medium: it must be integer and hash-only to stay off the golden. A second hash tree is a second source of truth; keep it strictly a *diagnostic*, never a gate. |
| 8 | nithish101/resonance | 0 | MIT | `packages/web/src/render/quality.ts` + `packages/web/src/render/loop.ts` (OBSERVED) | The **quality ladder as a pure controller**: seven levels, each a triple (particle fraction, device-pixel scale, post on/off), with a *down* threshold of `budgetMs * 1.2` and an *up* threshold of `budgetMs * 1.05` **and** `workMs < budgetMs * 0.5`, plus separate `overMs`/`underMs` accumulators so a single slow frame cannot cut and a single fast frame cannot restore. And a `RefreshEstimator` that takes the **median** of the last 120 rAF deltas to learn the real refresh rate. | `crates/web/src/tier_view.rs` (our `Ladder` and `Pace`) and `crates/render/src/quality.rs`. Moves **A6** (the web tier ladder) and **QT5** (the ladder comes back) and **QT8** (drift). | S | Low: our `Ladder` already has the cut order; this adds the up-path and the median refresh estimate, both missing. |
| 9 | nithish101/resonance | 0 | MIT | `packages/web/tests/visual.spec.ts` + `docs/decisions/0007-deterministic-capture.md` (OBSERVED) | **A deterministic capture mode**: a separate page that decodes a fixed input, runs the same WASM over it, and exposes `renderAt(t)` which advances the state in **fixed steps** and then draws — no wall clock, so the same input gives the same pixels. The visual suite compares committed goldens with `maxDiffPixelRatio: 0.01`, and one test renders the same moment **in two fresh pages** and requires byte-identical output. | `crates/web/src/arena_view.rs` (the capture line) and `tools/web-browse.mjs` (the headless driver). Moves **WEB5** (the browser frame beside the native frame) and **G10**. | M | Medium: our capture is a live frame at an arbitrary tick; this needs a *seekable* tick (`?tick=N`) and a frozen clock. The headless Chrome screenshots come out black per our own WEB5 note, so the readback must stay on the GPU path. |
| 10 | security-union/videocall-rs | 1797 | Apache-2.0 OR MIT | `neteq/src/delay_manager.rs` (OBSERVED) | A **jitter-buffer delay manager**: a quantile estimate (0.97) over a 2 s history with an exponential forget factor (0.9993), resampled every 500 ms, a **reorder optimizer** that adds `ms_per_loss_percent * loss%` to the delay, and a bounded `[base_minimum, base_maximum]` window. The delay is a *number the caller posts*, not a guess. | `crates/net/src/interp.rs` (our `DEFAULT_INTERP_DELAY_MS = 300`) and `crates/net/src/delay.rs`. Moves **QN4** (interp delay from loss data) and **MP26** (clock slew). | S-M | Low: our QN4 already posts a delay per loss profile; this makes the delay *measure* the link instead of a table. The quantile and forget factor are floats — they must become integers before they touch our tick. |
| 11 | EndersGamesDev/EmberEngine | 1 | none found (no LICENSE blob at root; only vendored third-party licenses) | `web/loader.ts.j2` (OBSERVED) | A **streaming compile that also counts the bytes**: the response body is piped through a counting `ReadableStream` into `WebAssembly.compileStreaming`, so download and compile overlap *and* the byte count is exact; on a `CompileError` it falls back to `arrayBuffer()` + `WebAssembly.compile` and re-counts the remainder. The load is reported as a phase list (`download`, `compile`, `init`, `start`) with a `degraded` flag. | `crates/web/loader.js` (our `loadEngine` already splits download/compile ms but does not stream-count) and `tools/web-build.mjs`. Moves **QW3** (the receipt's three numbers) and **WEB11** (the budget). | S | Low: our loader.js is 220 lines and already node-tested; this is an upgrade of one function. Risk: the fallback path must not mask a real `CompileError` (their code re-throws with a phase label — do the same). |
| 12 | EndersGamesDev/EmberEngine | 1 | none found | `web/hosts.ts.j2` (OBSERVED) | The **published address book with a client-side chooser**: one `server.json` lists many hosts; **the client picks** (rank by rtt probe, protocol version and build stamp), so no host needs to know about any other. Every network path **fails soft** (a missing book, a 404 mirror, a host that never answers are absences, never exceptions), the name shape is validated by a regex, and untrusted mirrors are **bound to the one name the writer authorised** and may only fill a name the book does not carry. | `crates/net/src/dedicated.rs` (`MasterList` / `ServerBrowser` / `Beacon`) and `crates/net/src/lobby.rs`. Moves **ON6** (sessions / matchmaking-lite) and **ON13**. | M | Medium: our `ServerBrowser` is a LAN beacon list; this is the *internet* shape. The mirror-binding rule is the valuable part: a hostile third-party list must not be able to insert a host. |

---

## 2. Wild ideas (things no GitHub engine has)

1. **The browser is a first-class peer, not a thin client.** Nobody ships "the browser tab is
   another participant in the same simulation". Our page already runs the same `engine_game::App`
   as the window (`crates/web/src/fight.rs`). If the browser can *host* the authority for a
   second tab (a local duel with no server, the way *motumbo* does P2P), the page becomes a peer
   and the net crate gets a third peer kind besides client and server. The proof is one digest
   printed by three hosts: the window, the tab, and the native server.
2. **One digest ends the fight — and the browser can be the referee.** The server's digest is
   already on the wire. Let the *browser* recompute it from the same published state and compare
   it to the server's, per tick. A browser that disagrees is a desync witness, not a desync
   victim. The engine would then be able to say "the tab and the server saw different worlds"
   without a second server, and an AI could file that as a bug packet.
3. **The page is the load queue, not a file.** Right now `crates/web/src/content.rs` fetches one
   whole `.e2bundle`. Instead the *same* `engine_assets::LoadQueue` (priority, byte budget,
   cancel, unload) should drive `fetch` on the web: the page asks for a logical path, the engine
   returns a generational `AssetHandle` before the bytes arrive, and a cancel drops the fetch
   mid-flight. Then WEB17 (the cooked bodies) and the arena kit load through **one** queue, and
   the native and web paths are literally the same code with a different `AssetSource`.
4. **A render worker that owns the canvas, with the main thread left free.** Our page drives
   the tick *and* the draw on one thread. A worker that owns the `OffscreenCanvas` and gets a
   `SharedArrayBuffer` of sim state (seqlock) would leave the main thread for input only. On a
   potato laptop that is the difference between a 12 fps page and a 60 fps page. Not a GitHub
   engine does this for a *game*; the pattern exists (resonance, webgpu-worker) but for media and
   demos.
5. **The invitation is a URL, and the URL is the whole lobby.** *motumbo* puts a base64 offer in
   `#j=...`; our `InviteCode` is 6 bytes and a `BuildId`. A link that carries (code, build
   version, rules digest) means "copy this to your opponent" is a real product feature, and it
   makes the LAN path and the internet path the same code path.
6. **A browser-visible rate ledger.** Our kbps numbers live in test receipts. Put the *same*
   numbers on the page (`net: sent=… shed=… kbps=…`), and the page becomes the tool that shows
   an owner whether a fight is starved — the browser is the dashboard, not a second implementation.
7. **The tab is the anti-cheat witness.** Because the browser holds the same deterministic sim,
   a browser client can *re-simulate* the last N ticks from the server's input tape and check
   the server's snapshot against its own. That is a server-authority check performed *by the
   client*, which is a mirror of the idea nobody ships because nobody has a deterministic client.
8. **Cross-target golden as a web service.** Our golden tape is equal on wasm and native
   (WEB1/WEB3). Put the recorder behind the door: `engine2040 golden --target web` runs the
   tape under wasmtime *and* in a headless browser and prints both hashes. One command, two
   targets, one number — the thing the finish line's "same hash" gate wants but nobody packages.
9. **Interest management in the browser.** `crates/net/src/interest.rs` selects 64 of 1024
   entities per connection. Run the *same* selection in the page and show the player what it
   can see and what was filtered — an "AOI debug" overlay in the browser. Games do not show you
   the relevancy cone; an engine that does makes the netcode legible.
10. **A time-machine for the browser.** The `renderAt(t)` idea from resonance, applied to the
    *fight*: a URL like `?fight&tick=420` opens the page at tick 420 with the same bodies, the
    same camera and the same pixels on every machine. That turns "the browser played a match" into
    "the browser can be asked to show you tick 420", which is the thing a reviewer, a bug report
    or a door verb actually needs.

---

## 3. Rejected (checked and dropped, so nobody re-mines them)

- **moq-dev/web-transport** — do-not-mine, and our tree already has the WebTransport + WebSocket
  race (`crates/net/src/browser/*`, `connect_race.rs`). Nothing left to take.
- **quinn-rs/quinn, BiagioFesta/wtransport, gschup/ggrs, Kan7/Klotho, naia, matchbox, renet2,
  LiteNetLib, johanhelsing/matchbox** — all on the do-not-mine list; we also already depend on
  quinn and wtransport.
- **cloudflare/quiche, mozilla/neqo, aws/s2n-quic, Tencent/tquic** — QUIC stacks. We are not
  writing a QUIC stack; a donor here would be a rewrite, not a port.
- **moqtail/moqtail** (111 stars, Apache-2.0) — its `libs/moqtail-ssts/src/algorithms/backpressure/state.rs`
  is a **pure tier state machine driven by queue depth**, no bandwidth estimate, exhaustively
  testable by feeding a depth sequence (OBSERVED). It is a *beautiful* fit for our ON14/ON9
  byte-budget discipline. Rejected only because `crates/net/src/wan_discipline.rs` already
  implements the same idea in our own integers (5 classes, 1200-byte cap, shed counts) and the
  donor is a media protocol we do not speak.
- **n0-computer/iroh** (12786, Apache-2.0) — the relay protocol in
  `iroh-relay/src/protos/relay.rs` is excellent (a `Restarting` message that tells clients to
  wait and retry, `EndpointGone`, a ping/pong keepalive at half the QUIC idle timeout). Rejected:
  we already have `crates/net/src/relay.rs` with a ticket handshake, a 300-tick punch-or-fallback
  and `crates/net/src/relay_ice.rs` with the RFC 8489 B.1 vector. A second relay is a rewrite.
- **appr-tc/data** (19, Apache-2.0) — a pure-Rust WebRTC DataChannel. We have WebTransport plus
  a WebSocket fallback; a third transport is a project, not a port, and WebRTC in the browser
  cannot be a Rust server leg without a signalling server we do not have.
- **wasm-peers/wasm-peers** (132, Apache-2.0) — a WebRTC datachannel wrapper in Rust/WASM. Same
  objection as above, plus the last push is 2024.
- **triplehex/webrtc-unreliable** (MIT; upstream is kyren/webrtc-unreliable, 418 stars) — a Rust
  server for browser unreliable datachannels; the README says messages above ~1200 bytes are
  dropped, which matches our 1200-byte cap. Rejected: we would have to own a full WebRTC
  stack, and our `crates/net/src/webtransport/cert.rs` + `browser_gate` already hold the 1200-byte
  line on the WebTransport leg.
- **paulbatchelor/rust-wasm-audioworklet** (10, BSD-2-Clause) — Rust DSP inside an AudioWorklet
  via raw `#[no_mangle]` exports and `wasm-gc` (OBSERVED). The do-not-mine list already has
  `piutranq/wasm-audioworklet-example`; and our A5 is **built** (`crates/web/src/audio_worklet.rs`
  carries the processor JS inside the wasm), so the only thing left is the *transport* between
  worklet and mixer, which row 3 above covers better.
- **matthewjberger/webgpu-worker** (0, Apache-2.0) — already in `steals-2026-10-18/06-new-engines.md`
  as T3/row 1. Its `src/lib.rs` `Gpu::new_async` (the OffscreenCanvas `create_surface` +
  `request_adapter` with `compatible_surface`) is the one file we have not read, and our
  `crates/web/src/shell.rs:405-428` already does the same thing with the same wgpu calls.
- **openmoq/moq-playa** (21, Apache-2.0) — a full MoQ stack in TypeScript with 1077 blobs. The
  one interesting file is `packages/transport/vectors/d16-libmoq/manifest.json` (binary wire
  vectors with hex and expected decode, OBSERVED) — a **conformance vector table**. Worth keeping
  as an idea, not as a port: our `crates/net` wire versions are already pinned by golden digests.
- **open-ribbi/velocut** (455, MIT) — a browser video editor in Rust/WASM with WebGPU. Its
  `web/e2e/*.spec.ts` Playwright suite is real, but it is a React app, not a game engine, and we
  already have a CDP driver (`tools/web-browse.mjs`).
- **Zushah/WasmGPU** (45, MPL-2.0) — a WebGPU × WASM engine for *scientific workloads*; the
  `rust/src/*` is a math library (mat4, quat, cull, bounds). Copyleft (MPL-2.0) and irrelevant.
- **glashoff/noob_tube** (AGPL-3.0), **Nusi42/LooseScrews** (AGPL-3.0) — AGPL; a Bevy project and
  a TS project. Not ports.
- **EndersGamesDev/EmberEngine** `web/games/end-game/v12/quality.ts.j2` — a scale ladder that
  moves the render scale by 0.88/1.07 with a 22 ms/17.3 ms threshold. Our `tier_view.rs` ladder
  already owns the cut order; this is a second ladder, not an upgrade. The *hosts.ts.j2*
  address-book half of the same repo is row 12 above and is kept.
- **markclausing/websoccer** `server/ws.js` — a hand-written RFC 6455 WebSocket server in 100
  lines. We already have `tokio-tungstenite` in the lock and `crates/net/src/websocket/server.rs`.
- **dzhng/battlesim** `web/src/downloads.ts` — a `countedFetch` that counts bytes through a
  `TransformStream` and retries a `no-store` fetch when Chromium's private-window cache write
  fails. Nice, but our `crates/web/loader.js` already counts bytes and caches by sha256. Only
  the "retry once with `cache: no-store`" line is worth copying (S).
- **nithish101/resonance** `packages/web/src/coi/register.ts` + `sw-template.js` — the COI
  service worker. We already have `crates/web/coi-serviceworker.js` and the `#coi` line; their
  version adds a `sessionStorage` "tried once" guard so a blocked worker cannot cause a reload
  loop. That one guard is worth copying into our `index.html` (S).

---

## 4. Verdict

**The single best steal is `dzhng/battlesim`'s worker-as-authority credit pool
(`web/src/battle/sim/authority.ts`)** because it is the one piece that makes the browser a
*participant* rather than a viewer: the tick runs in a worker, the page draws a published
snapshot, the worker stops when the page stops paying credits, and every publication carries the
tick and its digest — which is exactly the "native client and headless Chrome end a duel on one
digest" accept that WEB5, WEB9 and G10 all ask for and none of our rows can currently produce.
It is also the only donor here that is a *shape* (a few hundred lines of plain TypeScript) rather
than a framework, so it ports as a port and not as a dependency.
