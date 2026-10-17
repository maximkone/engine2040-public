# DEPENDENCIES — what the engine depends on and why

`deny.toml` is authoritative for licenses and bans. A row here is not
permission to add a crate: the orchestrator reads its LICENSE, checks
`deny.toml`, adds a row and runs `cargo deny check` before the wave that
needs it. Every dependency stays behind a feature, off the headless default.

## 1. Wired

| Crate | License | Feature | Why |
|---|---|---|---|
| glam 0.33 | MIT/Apache-2.0 | scene `math`, render `v1` | vectors, matrices, quaternions |
| winit 0.30, wgpu 30 | Apache-2.0; MIT/Apache-2.0 | render `window` | window, event loop, GPU |
| gltf 1 | MIT/Apache-2.0 | assets `import` | glTF import |
| bevy =0.20.0-rc.2 | MIT/Apache-2.0 | none (NOT WIRED 2026-10-16: 0 hits in any Cargo.toml/Cargo.lock) | HISTORICAL (D1 DEAD 2026-10-06 per docs/FOCUS.md: no Bevy picture, own renderer only; already removed from Cargo.toml): drew the arena and skinned fighters from one frozen frame per tick; never in `default-members` or the sim graph |
| rhai =1.26.1 (default-features=false: std, only_i64, no_float, no_time, no_module, no_closure, no_custom_syntax, internals; transitive smartstring 1.0.1 is MPL-2.0+, see `deny.toml` exceptions) | MIT OR Apache-2.0 (crates.io API 2026-10-04, rhaiscript/rhai) | rules (off `default-members`) | the NS32 rules sandbox (owner decision D2): integer-only scripts, op/call/size limits, deny-all registry; never in the sim graph |
| manifold-rust 0.16.0 | Apache-2.0 (crates.io API 2026-10-04; Manifold port) | geo `boolean` | mesh booleans; its 2D clipper (clipper2-rust, Boost-1.0) has one named exception in `deny.toml` |
| xatlas (vendored, unmodified, with a C shim of ours) | MIT, plus MIT and BSD-3-Clause notices inside the file (`crates/geo/vendor/README.md`) | geo `uv` | UV unwrap and chart packing; vendored because `xatlas-rs-v2` needs libclang for bindgen; compiled by `build.rs` with `cc`, one thread |
| meshopt 0.6.2 | MIT OR Apache-2.0 (crates.io API 2026-10-04; meshoptimizer MIT) | geo `lod` | attribute-aware simplification with a seam lock |
| parry3d 0.30.2 (`alloc`, `enhanced-determinism`) | Apache-2.0 | geo `collision` | V-HACD convex decomposition and convex hulls; the same crate rapier3d links |
| image 0.25 (png, hdr) | MIT/Apache-2.0 | assets `import`, render `v1` | texture and HDRI decode; avif stays off |
| rapier3d 0.36 (`enhanced-determinism`) | Apache-2.0 | physics `rapier` | the one 3D physics backend; parry3d 0.31 is its query layer; bumped from 0.35.3 on 2026-10-04 (new `SoftBodySet` argument in `PhysicsPipeline::step`, soft bodies unused): physics rapier tests 153 + 7, arena sim 101 + 7, golden `d57b5c8ae3096614` x2 unchanged |
| egui, egui-wgpu, egui-winit 0.36 | MIT/Apache-2.0 | editor `ui` | editor UI |
| egui_dock 0.21 | MIT | editor `ui` | dock panels |
| transform-gizmo(-egui) 0.11 | MIT/Apache-2.0 | editor `ui` | translate, rotate, scale gizmos |
| serde, serde_json | MIT/Apache-2.0 | scene `json`, provider | versioned authored files, provider JSON |
| uuid, blake3 | MIT/Apache-2.0; Apache-2.0 | scene | persistent IDs, content digests |
| cpal 0.18, lewton 0.10, hound 3.5 | Apache-2.0; MIT/Apache-2.0; Apache-2.0 | audio `device` | device output, Vorbis and WAV decode |
| rmcp 3.4 | Apache-2.0 | endpoint `mcp` | official Rust MCP SDK |
| burn 0.21 (CPU backends) | MIT/Apache-2.0 | distill `train` | offline student trainer |
| tokio 1 (rt, macros; 1.53.1 in lock via rmcp) | MIT (crates.io API, tokio-rs/tokio) | endpoint `mcp` | the `e2040-mcp` launcher runtime for MCP5 |
| quinn 0.11 (default features: runtime-tokio, rustls-ring) | MIT (LICENSE-MIT read live 2026-09-25, quinn-rs/quinn) | net `quic` | QUIC transport + datagrams for ON1 (rustls transitive) |
| wtransport 0.7 | MIT OR Apache-2.0 | net `webtransport` | WebTransport server for browser clients beside quinn (WEB9) |
| tokio-tungstenite 0.29 + futures-util 0.3 (default-features=false: handshake; std, sink) | MIT (tokio-tungstenite), MIT OR Apache-2.0 (futures-util); both already in Cargo.lock via the endpoint MCP stack | net `websocket` | WebSocket server, the fallback leg when a network drops UDP (WEB12); plain ws, wss is terminated in front |
| wasm-bindgen 0.2, wasm-bindgen-futures 0.4, js-sys 0.3, web-sys 0.3 (WebSocket, MessageEvent, CloseEvent) | MIT OR Apache-2.0 | net (wasm32 only) | browser WebSocket and WebTransport clients behind the Transport trait; the versions engine-web already links |
| puffin 0.20 | MIT/Apache-2.0 (crates.io API 2026-09-24) | perf `profile` | profiler on observe spans for PF2 |
| sha2 0.10 | MIT (LICENSE-MIT read live 2026-09-25, RustCrypto/hashes) | game (capture manifests) | per-frame SHA-256 for SHOW1 manifests, cross-checked by tools/showcase.mjs |
| rustls 0.23 (default-features=false: std, tls12, ring) | Apache-2.0 OR ISC OR MIT (crates.io API, rustls/rustls) | provider `tls` | opt-in HTTPS handshake for cloud LLMs; ring already in the graph via quinn |
| rustls-native-certs 0.8 | Apache-2.0 OR ISC OR MIT (crates.io API, rustls/rustls-native-certs) | provider `tls` | platform-native roots for the rustls handshake (webpki-roots skipped: CDLA-Permissive-2.0 is outside deny.toml) |
| ggrs 0.13 | MIT OR Apache-2.0 | none (NOT WIRED 2026-10-16: 0 hits in any Cargo.toml/Cargo.lock, candidate only) | rollback candidate for the arena sync (E40c); hand-rolled resim still in use until wired |

## 2. Banned (`deny.toml` enforces)

| Crate or family | Why | Instead |
|---|---|---|
| symphonia | MPL-2.0; banned by decision | lewton, hound (claxon for FLAC if needed) |
| ktx2 crate | blocked by decision | offline KTX-Software bake if ever needed |
| zstd, zstd-safe | C bindings with a dual BSD/GPL license | no compression dependency yet |
- `deny.toml` governs Cargo dependency licenses only. Donor code from any source may be
  ported or adapted (owner 2026-10-10); no license check or `EVIDENCE.md` row needed. Never hand-roll crypto.

- Source code from any repo (Unreal, Flax, Unity, Blender, or otherwise) may be ported or used as inspiration within the rule above.

## 3. Candidates the plans name (license to verify before adding)

| Crate | For | Board row |
|---|---|---|
| gilrs | gamepads and rumble on Windows | PLN13 plan |
| parry3d (already in the graph through rapier) | collision hulls in the cook | CT2 |
| rayon | cook and bake loops only, never simulation | PLN14 plan |
| fluent, taffy, cosmic-text | localization, game UI layout, text | PLN13 plan |
| claxon | FLAC decode, only if content needs it | PLN12 plan |
