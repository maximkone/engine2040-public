# Proposed rows — G5 web, perf, assets, shipping (2026-10-21)

Lane G5 of the powerup wave (`docs/research/steals-2026-10-20/05-web-perf-shipping-steals.md`
holds the hunt). One row per find, in the shape the wave plan asks for. The lead merges the
board; nobody here touches `docs/SPRINT*.md`.

Ids are `WEBX-01` and up. The `Fills` column names the row the find closes. The `Accept`
column is a command and the one line it must print, so the row can only land on proof.

---

## WEBX-01 — the browser sink on an AudioWorklet, fed by the engine's mixer (A5)

**What it is.** The page stops using the deprecated `ScriptProcessorNode` and hands the sound
to an `AudioWorklet` processor: the worklet asks for a block of frames, the page renders it
with the engine's own mixer and posts it back, the worklet writes it into its output buffer.
The processor's JavaScript rides inside the wasm binary (a blob URL), so the web build ships
no extra file and no tool changes.

**Donor.** `RustAudio/cpal` v0.19.0 — https://github.com/RustAudio/cpal — files
`src/host/audioworklet.rs` and `src/host/audioworklet/worklet.js` (plus
`examples/audioworklet/`), read 2026-10-20. Licence: Apache-2.0 (`Cargo.toml:7
license = "Apache-2.0"`, and the `LICENSE` file read live). What is ported is the *shape*
(one processor, the audio thread asks for frames, the block travels as interleaved stereo);
the host is ours and much smaller (see "Why not the donor's host" below).

**Fills.** A5 (finish-line gap 5, "Web audio on an AudioWorklet, fed by the existing mixer").
**Prerequisite.** WEB14 (cross-origin isolation) is **not** required for this landing — see
the degrade path. WEB16 (the fight cues) is what still needs the game side.

**Owned paths.** `crates/web/src/audio_worklet.rs` (new), `crates/web/src/audio.rs`,
`crates/web/src/browser.rs`, `crates/web/src/lib.rs`, `crates/web/Cargo.toml`,
`crates/web/index.html`, `crates/audio/src/block.rs` (new), `crates/audio/src/lib.rs`.

**Accept.**
- `node tools/gh-check.mjs cargo test -p engine-web --lib audio` → `engine-web --lib audio
  tests passed (N tests)`.
- `node tools/gh-check.mjs cargo test -p engine-audio --lib block` → `engine-audio --lib block
  tests passed (5 tests)`.
- The web build's headless run prints `audio: path=worklet blocks=<n> underruns=0` and the
  receipt JSON carries `"audioPath":"worklet"` and `"audioBlocks":<n>`.

**Kill line.** Kill if the headless run shows `path=script` (the worklet could not start) or
if the worklet's peak stays 0 while the mixer holds a cue: then the chain is broken, not the
page. Kill the row (not the fallback) if the worklet costs more than 2 ms per 128-frame
block on the runner.

**Cost.** M (the processor, the blob module, the message pump, the fallback path).

**Why not the donor's host.** cpal's worklet host needs a shared-memory wasm build
(atomics): its own doc comment says "Available on WebAssembly with the `audioworklet` feature.
Requires atomics support." That needs cross-origin isolation (WEB14, marked LATER) and a
shared-memory wasm build, which changes the whole web build and the 8 MiB budget. Our page
already owns the `AudioContext` and the mixer, so the smaller path is a hand-written
processor plus a message pump: no new Cargo dependency, no `deny.toml` change, no atomics.
The worklet module rides inside the wasm binary as a blob URL, so `tools/web-build.mjs` (owned
by another agent) is not touched at all.

**Degrade path (landable with or without cross-origin isolation).** The block travels by
`postMessage`, so the page needs no `SharedArrayBuffer` and no COOP/COEP headers: the worklet
path works on a plain host. When the browser refuses the worklet module (no `audioWorklet`,
a blocked blob URL, a strict content policy), the page falls back to the deprecated
`ScriptProcessorNode`, fed by the same mixer, and the page says `path=script`. When even
that fails there is no sound and the page says `path=none`. The path in use is printed on the
page (`#audioworklet`) and in the receipt (`"audioPath"`), so the choice is never invisible.
The row therefore waits for nothing: **WEB14 is not a prerequisite.** It only becomes one if
we later want the shared-buffer variant (no message copy, no underruns).

---

## WEBX-02 — draw calls and CPU milliseconds in the web receipt, with a window (QW10, WEB11)

**What it is.** The receipt already carried the last frame's draw count and CPU time; now it
also carries the worst frame and the 95th percentile over the last 120 frames, the most draw
calls, and the audio path's counters. A window means a page that stutters every few seconds
cannot hide behind a good last frame.

**Donor.** `BabylonJS/Babylon.js` — https://github.com/BabylonJS/Babylon.js — file
`packages/dev/core/src/Instrumentation/sceneInstrumentation.ts` (the `drawCallsCounter`
getter at line 480, the per-frame `_drawCalls.fetchNewFrame()` at line 535) and
`packages/dev/core/src/Instrumentation/engineInstrumentation.ts` (the `captureGPUFrameTime`
flag), read 2026-10-20. Licence: Apache-2.0 (`license.md` read live). Only the counter
shape is ported; the numbers are ours.

**Fills.** QW10 (draw calls and CPU ms per frame in the receipt) and WEB11 (the web budget
gate can read a percentile, not one frame).

**Owned paths.** `crates/web/src/perf_count.rs` (new), `crates/web/src/arena_view.rs`,
`crates/web/src/arena.rs`, `crates/web/src/lib.rs`.

**Accept.** `node tools/gh-check.mjs cargo test -p engine-web --lib perf_count` →
`engine-web --lib perf_count tests passed (6 tests)`; the receipt line carries
`"cpuP95Ms"`, `"cpuMaxMs"`, `"drawsMax"`, `"audioPath"`, `"audioBlocks"`, `"audioUnderruns"`
and `"audioPeak"`.

**Kill line.** Kill if the window costs more than 0.1 ms per frame (it must stay a
counter, not a profiler) or if the receipt grows past one line.

**Cost.** S.

**Note.** The RenderBundle half of QW10 (recording the static arena draws into a bundle and
replaying it) needs the render crate, which is held by GS44. It is a separate landing; the
counters land first and tell it whether the CPU cost is worth it.

---

## WEBX-03 — kit textures in KTX2 with Basis, transcoded at load (QW5, QA15) — PATCH-READY SPEC

**What it is.** The web bundle holds KTX2 files with Basis-supercompressed payloads; at load
the page transcodes them into the first compressed format the adapter reports (ASTC, BC7,
ETC2, else RGBA) and prints the format it chose. The transcoder is pure Rust, so it runs in
the browser with no C++ and no emscripten.

**Donor.** `marcogomez/basisu` (the crates.io `basisu` 0.1.0 crate) —
https://github.com/marcogomez/basisu — files `basisu/src/ktx2.rs` (the KTX2 container read),
`basisu/src/transcoder.rs` and `basisu/src/dispatch.rs` (the target dispatch),
`basisu/src/astc/`, `basisu/src/etc/`, `basisu/src/pvrtc/` (the target decoders), read
2026-10-20. Licence: Apache-2.0 (`LICENSE` read live; crates.io metadata agrees). The
cook-time encoder is the C++ tool `BinomialLLC/basis_universal` —
https://github.com/BinomialLLC/basis_universal — file `basisu_tool.cpp`, Apache-2.0 (`LICENSE`
read live), run in CI, not in the browser.
**Correction.** `docs/research/steals-2026-10-11/engine-landscape.md:181` named "basisu 0.1.0
— Pure-Rust KTX2 transcoder for runtime" with no repo and no licence; that line is fixed in
this wave.

**Fills.** QW5 (kit textures in KTX2 for the browser) and QA15 (pick the GPU texture format
at load). **Prerequisite:** none for the codec choice; the *bundle* lands with WEB6
(`crates/assets` is held by the WEB6 landing agent), so this row waits for WEB6.

**Owned paths (when `crates/assets` frees).** `crates/assets/src/ktx2.rs` (new),
`crates/assets/src/cook.rs` (the KTX2 cook step), `crates/render/src/texture_ktx2.rs` (new,
the transcode path), `crates/web/src/content.rs` (the web bundle list). `crates/render` is
held by GS44, so the render side waits for it too.

**Exact function names.**
- `crates/assets/src/ktx2.rs`: `pub fn cook_ktx2(png_bytes: &[u8], set: &str, map: &str) -> Result<Vec<u8>, Ktx2Error>`,
  `pub struct Ktx2Error`, `pub fn transcode_targets() -> &'static [TargetFormat]`.
- `crates/assets/src/cook.rs`: `pub fn cook_kit_texture(...) -> CookedTexture` with
  `CookedTexture { bytes: Vec<u8>, format: TextureFormat }`.
- `crates/render/src/texture_ktx2.rs`: `pub fn transcode_at_load(bytes: &[u8], wanted: &[TextureFormat]) -> Option<TranscodedTexture>`,
  `pub struct TranscodedTexture { pub format: TextureFormat, pub width: u32, pub height: u32, pub mips: Vec<Vec<u8>> }`,
  `pub fn adapter_prefers(features: &wgpu::Features) -> Option<TextureFormat>`.

**Exact test names.**
- `crates/assets/src/ktx2.rs`: `the_cooked_ktx2_is_smaller_than_the_png_and_has_the_same_pixels`,
  `the_cooked_ktx2_reads_back_through_our_own_reader`.
- `crates/render/src/texture_ktx2.rs`: `the_transcoder_picks_the_first_format_the_adapter_reports`,
  `forcing_a_feature_off_picks_the_next_format_and_still_renders`,
  `the_transcoded_bytes_are_the_same_every_run`.

**Exact proof command.** `node tools/gh-check.mjs cargo test -p engine-assets --lib ktx2` →
`engine-assets --lib ktx2 tests passed`; `node tools/gh-check.mjs cargo test -p engine-render
--lib texture_ktx2` → `engine-render --lib texture_ktx2 tests passed`.

**What the golden must stay at.** `d57b5c8ae3096614` (seed 2040, 600 ticks). The KTX2 cook
and the transcode are presentation only: no texture byte and no format choice may reach the
tick, the tape or the hash. The proof runs `cargo run -p engine2040 --release` twice and
both runs must print the golden.

**Kill line.** Kill if the transcoded wasm pushes `dist/web` over the 8 MiB cap that WEB4
set, or if the transcoded bytes differ from the C++ reference on the donor's own corpus
(it claims "verified byte-for-byte identical to it across a corpus of real textures").

**Cost.** M.

**Also note.** `deny.toml` bans the crate named `ktx2` (`[bans] deny = [..., "ktx2"]`). The
`basisu` crate is a different name and is not banned, so no `deny.toml` change is needed for
this row. If the team ever prefers the `BVE-Reborn/ktx2` container reader
(https://github.com/BVE-Reborn/ktx2, Apache-2.0, read 2026-10-20), that crate **is** banned
and the ban line must be lifted by the lead.

---

## WEBX-04 — one Perfetto-readable trace file for the CPU spans and the GPU passes (gap 13)

**What it is.** A single Chrome/Perfetto JSON trace the engine writes when asked: the tick
spans, the GPU passes and the frame marks. Perfetto reads the Chrome format, so no Perfetto
code is needed.

**Donor.** `thoren-d/tracing-chrome` — https://github.com/thoren-d/tracing-chrome — file
`src/lib.rs` (the `ChromeLayer` tracing layer: one JSON object per span and event with
`ts`/`dur`/`cat`, the writer thread, the `TraceStyle` switch), read 2026-10-20. Licence: MIT
(`LICENSE` read live).

**Fills.** finish-line gap 13 (C13: "one Perfetto trace file for CPU spans and GPU passes")
and PF27 (the in-game perf overlay).

**Owned paths.** `crates/perf/src/trace_file.rs` (new), `crates/perf/Cargo.toml`,
`crates/proof/src/trace.rs`.

**Accept.** `node tools/gh-check.mjs cargo test -p engine-perf --lib trace_file` →
`engine-perf --lib trace_file tests passed`; the trace file parses as JSON and every span
has `ts`, `dur` and `cat`.

**Kill line.** Kill if a `tracing` span or its allocation can reach the fixed tick path: the
hash must not move, and the layer stays behind an off-by-default feature.

**Cost.** S. **Prerequisite:** none.

**Note.** The workspace has no `tracing` dependency today (no `Cargo.toml` names it), so this
row adds `tracing`, `tracing-subscriber` and `tracing-chrome`. Their licences are MIT and
Apache-2.0, both already allowed in `deny.toml`; no allow-list change is needed.

---

## WEBX-05 — which crates cost the most to compile (OPT11, OPT32)

**What it is.** A report that counts the LLVM IR lines per generic function per crate, so
the crate fan-out cut (OPT11) knows which crate to split or feature-gate first, and the
build baseline (OPT32) has a second number next to the wall time.

**Donor.** `dtolnay/cargo-llvm-lines` — https://github.com/dtolnay/cargo-llvm-lines — files
`src/count.rs` and `src/table.rs` (the per-function IR line table that ranks crates by
generic code size), read 2026-10-20. Licence: MIT OR Apache-2.0 (`LICENSE-MIT` and
`LICENSE-APACHE` read live).

**Fills.** OPT11 (crate fan-out cut) and OPT32 (build baseline promotion).

**Owned paths.** `tools/llvm-lines.mjs` (the report), `.github/workflows/ci.yml` (the step).
Both are owned by the "speedup" agent; this row waits for it or lands as a report only.

**Accept.** `node tools/llvm-lines.mjs --top 20` → `crates/render: 12,345 IR lines (the
costliest crate)`.

**Kill line.** Kill if the report names no crate that the OPT1 timings also name as slow (no
new information, no split).

**Cost.** S. **Prerequisite:** OPT1 (the build matrix) must exist; it does not yet, so this
row waits for it.

---

## WEBX-06 — containerized CI caches: cook the dependencies before the source (OPT9)

**What it is.** A container build that compiles the dependencies in one layer and the source
in the next, so a source change reuses the dependency layer.

**Donor.** `LukeMathWalker/cargo-chef` — https://github.com/LukeMathWalker/cargo-chef — files
`src/recipe.rs` (the `Recipe` struct and the skeleton `Cargo.toml` that keeps only the
dependencies) and `src/main.rs` (the `prepare`/`cook` two-step), read 2026-10-20. Licence:
MIT OR Apache-2.0 (`LICENSE-MIT` + `LICENSE-APACHE` read live).

**Fills.** OPT9 (compile cache across lane pools) and gap 22 (E22, the Windows ship check
from CI).

**Owned paths.** `.github/workflows/` (a container job), `Dockerfile` (new).

**Accept.** A container build of the workspace prints the second layer's build time under
the first layer's.

**Kill line.** Kill if the web and CI builds stay runner-native with no container image:
`ci.yml` already uses `sccache-action` and `Swatinem/rust-cache`, and a recipe buys nothing
on a bare runner.

**Cost.** S. **Prerequisite:** the decision to move a build into a container (owner's call).

---

## WEBX-07 — the receipt as a stored snapshot (E23, PLF9, WEB11)

**What it is.** A receipt line is a stored snapshot: a change shows as a diff in review
instead of a wall of numbers, and the pending snapshots are listed by the CLI.

**Donor.** `mitsuhiko/insta` — https://github.com/mitsuhiko/insta — files `insta/src/` (the
snapshot assertion and the `.snap` store) and `cargo-insta/` (the CLI that lists pending
snapshots and writes them on accept), read 2026-10-20. Licence: Apache-2.0 (`LICENSE` read
live).

**Fills.** E23 (GM3: the receipt against the reference captures), PLF9 (the player benchmark
file) and WEB11 (the budget gate).

**Owned paths.** `crates/web/tests/it/` (the receipt snapshots), `crates/game/tests/it/`.

**Accept.** `cargo insta test --workspace --check` → `pending snapshots: 0`.

**Kill line.** Kill if the receipts are not byte-stable across two runs of the same build (a
snapshot of a volatile receipt is noise); `tools/ship.mjs:50` already normalises paths and
numbers.

**Cost.** S-M. **Prerequisite:** none.

---

## WEBX-08 — profile-guided optimization for the ship profile (PLF9, gap 22)

**What it is.** The ship profile is built with a profile: an instrumented run of the proof
scene collects the profile, and the release build uses it.

**Donor.** `rust-lang/rust` — https://github.com/rust-lang/rust — file
`src/doc/rustc/src/profile-guided-optimization.md` (the recipe: `-Cprofile-generate` on an
instrumented build, run the scene, then `-Cprofile-use`; it also says that sampling
profiles need `-Cprofile-sample-use` and that the two kinds do not convert), read
2026-10-20. Licence: dual MIT OR Apache-2.0 (the rust-lang/rust tree).

**Fills.** PLF9 (the player benchmark) and gap 22 (E22, the Windows ship check from CI).

**Owned paths.** `tools/pgo.mjs` (the recipe), `Cargo.toml` (the `ship` profile), the CI job.

**Accept.** `node tools/pgo.mjs --profile ship --bench` → `ship p50 14.2 ms (plain 16.8 ms)`.

**Kill line.** Kill if the PGO build does not beat the plain ship build by the printed
amount in two runs (measure, never guess).

**Cost.** M. **Prerequisite:** OPT1 (the quiet-box window) — PGO needs a quiet machine.

---

## WEBX-09 — pick the compressed texture format the adapter really has (QW5, QA15)

**What it is.** The probe order that finds out what the adapter can take (ASTC 4x4, ASTC 6x6,
BC7, ETC2, else RGBA) before anything is transcoded.

**Donor.** `mrdoob/three.js` — https://github.com/mrdoob/three.js — file
`examples/jsm/loaders/KTX2Loader.js` (the format table `RGBA_ASTC_4x4_Format` …
`RGBA_ETC2_EAC_Format`, the `VK_FORMAT_*` blocks it maps them to, and the support probe
order) plus the prebuilt transcoder it loads from `examples/jsm/libs/basis/`, read
2026-10-20. Licence: MIT (`LICENSE` read live). Same repo as the film-grain steal in
`docs/research/LADDER.md:222`, but a different file and a different idea.

**Fills.** QW5 and QA15.

**Owned paths.** `crates/render/src/texture_ktx2.rs` (with WEBX-03).

**Accept.** The receipt prints the chosen format and forcing a feature off picks the next
one.

**Kill line.** Kill if wgpu's own `Features` and adapter info already answer the question
(WEB4 already prints the adapter features): then only the probe order is left, and that is
five lines.

**Cost.** S. **Prerequisite:** WEBX-03.

---

## WEBX-10 — the packaged ship artifact and its config (E23, PLF5)

**What it is.** A declarative ship config: which binaries ship, which installers, what the
artifact folder looks like.

**Donor.** `axodotdev/cargo-dist` — https://github.com/axodotdev/cargo-dist — files
`cargo-dist/src/platform.rs` and `dist-workspace.toml` (the declarative ship config), read
2026-10-20. Licence: MIT OR Apache-2.0 (`LICENSE-MIT` + `LICENSE-APACHE` read live).

**Fills.** E23 (GM3) and PLF5 (the store build script).

**Owned paths.** `dist-workspace.toml` (new), `tools/ship.mjs`.

**Accept.** `node tools/ship.mjs pack` writes the artifacts and a manifest that lists them.

**Kill line.** Kill if `tools/ship.mjs` already stages and runs a clean folder (it does,
2232 lines: pack, stage, run and the `CLEAN-RUN` fingerprint ledger) and no installer is
ever wanted; then the donor only adds a config format.

**Cost.** M. **Prerequisite:** none.

---

## WEBX-11 — the prebuilt WebAssembly Basis transcoders, as a fallback (QW5)

**What it is.** Ready-made WASM transcoders for the browser, used only if the Rust
transcoder does not fit the size budget.

**Donor.** `KhronosGroup/Basis-Universal-Transcoders` —
https://github.com/KhronosGroup/Basis-Universal-Transcoders — file `README.md` (the target
table) and `assembly/` (the AssemblyScript build), read 2026-10-20. Licence: Apache-2.0
(`LICENSE.txt` read live).

**Fills.** QW5 (fallback only).

**Owned paths.** `crates/web/` (the transcoder module), `tools/web-build.mjs`.

**Accept.** The browser prints the format it chose and the bundle grew by the printed bytes.

**Kill line.** Kill unless the Rust transcoder (WEBX-03) fails the size budget; and kill it
outright if the web tier must be ETC2: its own README marks BasisLZ/ETC1S as "planned" for
every target and ETC as "planned" for both columns — only UASTC is implemented (RGBA8, R8/RG8,
ASTC 4x4, BC7).

**Cost.** S. **Prerequisite:** WEBX-03.

---

## WEBX-12 — post-link layout optimisation for a Linux server build (PLF9)

**What it is.** BOLT reorders the code of a linked binary by what the profile says it does.

**Donor.** `llvm/llvm-project` — https://github.com/llvm/llvm-project — file `bolt/README.md`
(the recipe: an unstripped symbol table and a link with `--emit-relocs` / `-q`), read
2026-10-20. Licence: Apache-2.0 WITH LLVM Exceptions (`llvm/LICENSE.TXT` read live).

**Fills.** PLF9 (Linux build only; there is no Windows row for it).

**Owned paths.** `tools/bolt.mjs`, the Linux server job.

**Accept.** `node tools/bolt.mjs --bin target/release/arena` → `bolt p50 12.9 ms (plain 14.2 ms)`.

**Kill line.** Kill for the ship build: the same file says BOLT "operates on X86-64 and
AArch64 ELF binaries", and our ship target is a Windows PE. Keep it only for a Linux
dedicated server.

**Cost.** S. **Prerequisite:** a Linux server build that exists.

---

## Already in our tree (do not re-mine these)

Each line is a correction to the wave's earlier notes, with the file and line that proves it.

| Topic | Where it already is | Note |
|---|---|---|
| A4, the strict WebGPU CI leg | `crates/render/src/backend.rs:437-461` reads `E2040_STRICT_WEBGPU` and adds `wgpu::InstanceFlags::STRICT_WEBGPU_COMPLIANCE`; `.github/workflows/ci.yml:111` sets `E2040_STRICT_WEBGPU: "1"` for the test matrix | Closed. What is missing is a runner with a GPU adapter (the leg skips there), which is infra, not a steal. |
| WEB9/WEB12, the browser netcode | `crates/net/src/browser/wt.rs`, `crates/net/src/browser/ws.rs`, `crates/net/src/bin/net_serve.rs`, and the net CI job that plays a headless-Chrome duel | Closed and proven. The WebTransport API is declared by hand because web-sys gates it. `moq-dev/web-transport` is named in the plan; renet2, naia, lightyear and matchbox were read in `docs/research/steals-2026-10-18/03-net-web-audio.md`. |
| WEB14, cross-origin isolation | `crates/web/coi-serviceworker.js` (its own header names the donor `gzuidhof/coi-serviceworker`, MIT); `crates/web/src/shell.rs:76-79` reads `window.crossOriginIsolated`; the receipt carries it at `crates/web/src/report.rs:116-119` | The service worker is there. What is missing is a live deployment proof, and it is **not** a prerequisite for WEBX-01. |
| The wasm size tools | `.github/workflows/web-build.yml:65-80` installs binaryen `wasm-opt` and `twiggy`; `tools/web-budget.mjs:137` prints the twiggy lines; `tools/wasm-info.mjs` compares the two sizes | Closed. `razrfalcon/cargo-bloat` overlaps twiggy; `rustwasm/wasm-snip` is archived. |
| The quality ladder | `crates/render/src/tier.rs`, `crates/web/src/tier_view.rs`, the `WEB_CUT_ORDER` test, and the restore that landed as `c7ab2f70` | Closed. GS35 already names Bevy's `GpuPreprocessingSupport` shape. The one thing no repo supplies is a fixed ordered cut list: that is a new row (`WEB_LADDER_ORDER`), not a steal. |
| The potato gate (G9) | `tools/potato-check.mjs` (18346 bytes) with its own test, `crates/render/src/tier.rs`, and the GS10 tier rules that landed as `dc517ed` | Closed. |
| The draw count in the receipt | `crates/web/src/arena.rs` already carried `receipt.draws` and `last_cpu_ms` into `crates/web/src/arena_view.rs` before this wave | WEBX-02 adds the window over them, not the first count. |
| `basisu 0.1.0` | `docs/research/steals-2026-10-11/engine-landscape.md:181` | Fixed in this wave: the repo, the licence and the date read are now on that line. |
