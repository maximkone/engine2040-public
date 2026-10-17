# Browser multiplayer scout (2026-10-17)

Read: `docs/plan/web-and-realism.md` (1-2), `docs/plan/multiplayer.md`, `docs/SPRINT-22-web-realism.md`. Claims come from fetched primary pages unless marked secondary. GUESS = not verified.

**Support (caniuse, read).** WebTransport first version: Chrome 97, Edge 98, Firefox 114, Safari 26.4 (desktop and iOS). The W3C spec is a Candidate Recommendation Snapshot dated 2026-07-30. Caniuse's Chrome Android value (154) looks wrong (GUESS).

## Ideas

| Idea | Source + URL | Licence | Copy or do | Row | Effort |
|---|---|---|---|---|---|
| serverCertificateHashes rules | W3C WebTransport, https://www.w3.org/TR/webtransport/ | W3C (GUESS: W3C Document licence) | SHA-256 over the full DER of the leaf cert, not SPKI. Key must be ECDSA P-256, RSA forbidden, validity at most 2 weeks. Regenerate each CI run, embed the hash in the page at build. | WEB9 | S |
| wtransport server | https://github.com/BiagioFesta/wtransport (docs.rs 0.7.2) | Apache-2.0 OR MIT | Server beside quinn: it needs quinn 0.11.6; repo lock has 0.11.12 (compatible). MSRV 1.88; repo pins 1.97.1. `Identity::self_signed` gives ECDSA P-256 but no hash method, so compute the hash yourself. README: not production-ready. | WEB9 | M |
| WebSocket fallback with a race | websocket.org note (secondary); Colyseus https://github.com/colyseus/colyseus | MIT (Colyseus) | Second transport impl behind the net trait. Race QUIC/WebTransport against WS at connect; first success wins. Copy the pattern, not a dependency. Colyseus README: state is "delta-compressed and binary-encoded". | WEB9 (adds the race) | M |
| Lightyear WebTransport glue | https://github.com/cBournhonesque/lightyear | Apache-2.0 OR MIT | WebTransport (via aeronet) is marked wasm-capable. Read its client glue for the wasm client. Do not adopt Bevy. | WEB9 | S |
| matchbox_socket | https://github.com/johanhelsing/matchbox | MIT or Apache-2.0 | WebRTC, wasm and native; `ggrs` feature. Signalling server must be reachable over HTTP(S). TURN and NAT not documented. Hosted demo signalling is offline (ggrs page). Plan B. | NEW WEB12 "WebRTC fallback spike" | M |
| webrtc-unreliable | https://github.com/triplehex/webrtc-unreliable | MIT | Rust server for browser unreliable data channels. Messages above about 1200 bytes are dropped (author's test, varies by browser), which fits our 1200-byte cap. | NEW WEB12 | M |
| ggrs | https://github.com/gschup/ggrs | MIT or Apache-2.0 | Browser only through the matchbox feature. Read its request-list API (no callbacks) for ON22; no dependency. | ON22 | S |
| Headless Chrome WebGPU flags | https://developer.chrome.com/blog/supercharge-web-ai-testing | Google page | Linux (Colab T4 example): `--enable-unsafe-webgpu --use-angle=vulkan --enable-features=Vulkan --disable-vulkan-surface`. Headless disables GPU by default. The SwiftShader adapter flag is not in this page (GUESS: `--use-webgpu-adapter=swiftshader`). | WEB4 | S |
| Cloudflare Realtime TURN | https://developers.cloudflare.com/realtime/sfu/pricing/ and /realtime/turn/ | Commercial | 1,000 GB/month free, shared by SFU and TURN; then $0.05/GB. TURN over TLS on 443/tcp. Relay only if WebRTC is chosen. Credential API not verified. | NEW WEB12 | S |
| Fly.io hosting | https://docs.fly.io/about/pricing | Commercial | No free tier for new orgs. shared-cpu-1x 256 MB $2.19/mo; egress $0.02/GB (NA, EU). Runs the WEB9 server at a fixed URL. | NEW WEB13 "host the net server" | S |
| GitHub runners as test server | https://docs.github.com/en/actions/reference/limits | GitHub terms | Public repo runners are free; job cap 6 h. Start server and headless Chrome in one job. | WEB9 | S |

Skipped after reading: geckos.io (BSD-3-Clause; needs 9208/tcp and UDP 1025-65535 open; its docs point people without port-forwarding experience to WebSocket), Nakama (Apache-2.0; WebSocket and rUDP only), Photon (commercial; pricing pages conflict).

## Top 5 this week

1. WEB9 transport spike: wtransport plus a DER-hash helper; one GitHub job with headless Chrome connecting to a self-signed cert. Size M.
2. WebSocket fallback with a QUIC race, same codec bytes. Size M.
3. Cert helper: ECDSA P-256, at most 14 days, SHA-256 of DER; unit test refuses RSA and long validity. Size S.
4. WEB4 adapter probe with the Chrome flags above; record the no-adapter and black-canvas cases. Size S.
5. Read lightyear's WebTransport glue before writing the wasm client. Size S.

## Traps

- **Corporate networks.** WebTransport needs UDP (QUIC). Blog sources say enterprise networks often block UDP. A Sophos KB says browsers usually fall back to TCP HTTPS when QUIC is blocked (secondary). WebTransport does not fall back on its own (GUESS), so the app must race WS.
- **WebSocket is not safe either.** websocket.org (secondary) says HTTP/1.1-only proxies can drop the Upgrade header, DPI can block non-HTTP on 443, and TLS-inspecting proxies break wss://. A proxy block looks like a server outage.
- **WebRTC** needs UDP ranges open and TURN for NAT. TURN over 443/tcp is the likely work-PC path (GUESS).
- **Two different hashes.** serverCertificateHashes uses SHA-256 of the DER leaf (W3C, read). Chrome's `--ignore-certificate-errors-spki-list` uses SPKI (Chromium source, secondary). Mixing them fails silently.
- **Cert expiry.** Maximum 14 days. A pinned page with an old cert dies; regenerate each build.
- **Safari and Firefox.** Safari has no WebTransport before 26.4, so WS fallback is required. Firefox: caniuse says 114, one secondary snippet says not shipped in early 2026. Check both at WEB4; Safari datagrams unchecked (GUESS).

Not verified: Photon pricing, libwebrtc licence (GUESS BSD-3), bevy_ggrs licence, Cloudflare credential API, the Chrome SPKI flag in headless (untested).
