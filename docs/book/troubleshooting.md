# Troubleshooting

Each entry says what you see, why, and the command that checks the fix. Facts come from the runs in `docs/plan/web-and-realism.md`, the probe in `docs/research/web-2026-10-17/07-wasm-probe.md` and the scouts in the same folder. A claim from a scout that we have not tested on our own server is marked.

First, always:

```sh
node tools/web-doctor.mjs
```

It prints `OK` or `MISSING` for every piece of the web target, with the fix command, and ends with `web target: READY` or `NOT READY`.

## No WebGPU

You see: the red page "This browser cannot run the game", `data-state=fatal`, and the report line `"ok":false,"reason":"no-webgpu"`.

Why: the page needs `navigator.gpu`. It is on by default in Chrome and Edge 113 or newer, Firefox 141 on Windows and Safari 26. Browsers switch it off on plain `http` pages (use https or `localhost`), and managed Chrome can block it by group policy (not checked on the work PC). In headless Chrome on a machine with no GPU the adapter is `null` unless you pass the software-adapter flags.

Check: open the page from `http://localhost:8080/` after `python -m http.server 8080 --bind 127.0.0.1 -d dist/web`. In headless Chrome, use the flag line in [Build, run and publish the web target](web-build.md), then:

```sh
node tools/web-browse.mjs --dir dist/web --chrome google-chrome --out out/web --require-adapter
```

`attempts.json` in the output folder names the reason every flag set failed.

## Black canvas

You see: a page that loads, `navigator.gpu` present, and a black canvas; or a screenshot that is black.

Why, measured on the runner:

- Without the Vulkan loader and Mesa's software driver (`libvulkan1 mesa-vulkan-drivers`) Chrome still reports an adapter but cannot create the swap-chain shared image (`Could not find SharedImageBackingFactory ... WebgpuSwapChainTexture` in the Chrome log). The canvas never presents and the device is lost again and again. The shell stops after three losses in ten seconds and shows `the device was lost again and again`.
- `--use-vulkan=swiftshader` and `--use-vulkan=native` give an adapter and a blank canvas. `--use-angle=swiftshader` presents.
- A headless WebGPU canvas screenshot can be black even when drawing works. Gate on a readback (`mapAsync`, then hash the bytes), not on a screenshot.
- WebGPU init is async. A screenshot taken at load can be blank with no error; print the receipt after the device is ready.

Check:

```sh
sudo apt-get install -y libvulkan1 mesa-vulkan-drivers
```

Then rerun `web-browse` as above. A passing run takes a 1280x720 screenshot with many colours (150 on the first green run) and counts frames.

## Blocked UDP

You see: a browser client that cannot connect to a native server while a native client can.

Why: WebTransport runs over QUIC, which is UDP. Many offices block UDP, and a browser does not fall back to TCP on its own (scout `04-browser-multiplayer.md`, secondary sources, not yet tested on our server). A WebSocket fallback can fail too: an HTTP/1.1-only proxy can drop the Upgrade header and a TLS-inspecting proxy breaks `wss://`, so a proxy block looks like a server outage. Safari has WebTransport only from 26.4, so the WebSocket leg is required. The plan races both at connect and names the blocked leg (rows WEB9 and WEB12).

What exists: the dedicated server accepts WebTransport sessions beside QUIC in `crates/net/src/webtransport/`, and a session plays a 600-tick duel with a pinned client in the tests. The WebSocket fallback and the browser client are open rows.

Check the server side:

```sh
node tools/gh-check.mjs cargo test -p engine-net --lib webtransport_refuses_a_client_pinned_to_the_wrong_hash
```

## Wrong MIME type

You see: a slow start, a console warning, or a failed module load; `curl` shows a `content-type` other than `application/wasm` for `engine_bg.wasm`.

Why: the browser streams and compiles a module only when it is served as `application/wasm`. A wrong type is a slow path or an error, not a clue in the page. A hand-made `.br` file failed on github.io in a forum report (scout 05-ai-web-loop-and-ci.md); `curl -I` must show `application/wasm` before any `.br` file ships. The Pages workflow checks the type before it publishes and again after the deploy.

Check:

```sh
curl -sI http://localhost:8080/engine_bg.wasm
```

The published site is checked the same way:

```sh
curl -sI https://maximkone.github.io/engine2040-public/engine_bg.wasm
```

## Stale or wrong certificate

You see: a browser client that is refused by the server while the same server works for a native client.

Why: a browser accepts a self-signed certificate through `serverCertificateHashes` only if it is X.509v3 with an ECDSA P-256 key, valid for at most 14 days, with the current time inside the period, and the pin is the SHA-256 of the whole DER of the leaf, not of its public key. An old certificate, an RSA key or the wrong hash fails. Chrome's `--ignore-certificate-errors-spki-list` uses the SPKI hash instead; mixing the two fails silently.

What exists: `crates/net/src/webtransport/cert.rs` mints a fresh certificate (`PinnedCert::mint`, 1 to 14 days) and checks any DER against the same rules. Its refusals read `validity of N s exceeds the 14-day browser limit`, `public key is not ECDSA P-256`, `certificate is not valid yet` and `certificate has expired`. Mint a new one on every run and give the page its hash at build time.

Check:

```sh
node tools/gh-check.mjs cargo test -p engine-net --lib webtransport_cert_refuses_validity_over_14_days_and_accepts_exactly_14
```

## The wasm module fails at load

- `WebAssembly.Table.grow(): failed to grow table by 4`: the module was optimised by binaryen 108 (the apt package). Use release `version_133`.
- wasm-opt rejects the module: pass the feature flags listed in the web page.
- `RuntimeError: unreachable` at start: a call to `Instant::now()` or `SystemTime::now()` ran in wasm32-unknown-unknown, where they trap. Use the clock crate (`engine-clock`) or `PhysFx::with_clock`.
- `std::fs` calls return an error and threads fail to spawn on this target. Content must arrive by `fetch` as bytes.
- The wasm-bindgen CLI must match the crate version `Cargo.lock` locks exactly. The build tool prints the `cargo install wasm-bindgen-cli --version <v> --locked` line that fixes it.

Check the tool tests:

```sh
node --test tools/web-doctor.test.mjs
```

## Cargo says "Blocking waiting for file lock"

Another cargo build holds `target/`. Wait. Do not start a second cargo and do not delete anything under `target/`. Heavy proofs go to GitHub with `node tools/gh-check.mjs cargo test -p <crate> --lib <filter>`.
