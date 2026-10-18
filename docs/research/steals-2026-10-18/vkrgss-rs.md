STEAL-VKRGSS scout, 2026-10-18. Wave 2026-10-18/1, block 1: github.com/topics/game-engine (l=rust, o=desc, s=updated), position 31. Area: per-stage frame timing, validation-as-a-gate, provenance tables for numeric formulas.
Read only, ideas only; no code copied, nothing vendored. Every donor file below was fetched live on 2026-10-18.

== 1. WHAT IT IS ==
vkrgss-rs (xuyanwen2012, 1 star, updated 2026-10-18) is a Rust + Vulkan **runtime for the classic RPG Maker engines** (RGSS1/2/3 — XP, VX, VX Ace) on Linux, and is an engine reimplementation rather than a general-purpose engine: it reproduces a fixed 2D engine's exact behaviour (sprites, planes, windows, tilemaps, bitmap blend arithmetic, an embedded Ruby 1.8.7, audio decoding incl. MIDI/WMA) so shipped games run "pixel for pixel, byte for byte". It is included here as a 2D engine because it *implements* the engine layer, but the transferable value is not its Vulkan/tile-binning renderer — it is the discipline around it: per-stage frame timing, validation errors treated as failures, and a machine-checked provenance table.

== 2. LICENCE ==
MIT for the repository (LICENSE read live 2026-10-18: https://raw.githubusercontent.com/xuyanwen2012/vkrgss-rs/main/LICENSE — "MIT License, Copyright (c) 2026 Yanwen Xu"); the API reports `NOASSERTION` because the same file adds a third-party section: `third_party/ruby-1.8.7/` is Ruby 1.8.7 under the Ruby licence or GPL-2, and `third_party/oniguruma-5.9.6/` is BSD. Our own crates are untouched here; ideas only.

== 3. STEALS (ranked) ==
1. A frame-timing record where a stage that did not run is `None`, with CPU and device durations kept apart | xuyanwen2012/vkrgss-rs@main:crates/vkrgss-vk/src/frame_timing.rs:13-42 | TOOLING (perf) | engine2040 `perf/src/profiler.rs:518-577` FrameReport, XX1 pass timing | S
   Idea: `FrameTiming` is seven `Option<Duration>` fields, grouped by kind: processor stages `encode`, `bin`, `upload`, `submit`, `present`, then **device** stages `compose_device`, `present_device` (:14-30). `None` is meaningful — a stage that did not run that frame (a render before the first one, a present of a frame that was never shown, a queue without timestamps) is *absent*, not zero (:9-12) — and `stages()` hands back short names in the exact order a frame goes through them (:32-42).
   Maps: engine2040's FrameReport is building toward named production passes (XX1); the two ideas that transfer are "a stage that did not run is None, never 0" and "CPU stage time and GPU device time are different columns", which is what makes a p50/p99 table readable instead of misleading.
   Kill: if any timing value is ever fed into the hash, or a skipped stage prints as 0 ms -> fix the reporting or drop the idea; timings are presentation numbers only.

2. Validation errors are collected into a log that outlives the device and fail the run | xuyanwen2012/vkrgss-rs@main:crates/vkrgss-vk/src/validation.rs:9-40 | TOOLING (GPU gate) | engine2040 GPU tests / `node tools/gh-check.mjs`; test-and-proof "a GPU test that skipped" | S
   Idea: when the backend is created with a log, the Khronos validation layer is enabled if installed, `VK_EXT_layer_settings` turns its **synchronization validation** on too so a missing/wrong barrier is an error (:22-28), and every error-severity message is appended to a shared `ValidationLog` (:29-40). Two details matter: a missing layer is not a failure (it only applies "when the layer is installed"), and the log is handed out so errors reported while the backend is *dropped* — leaked objects — can still be read afterwards (:24-28). The README's CI badge states outright that Khronos validation errors fail CI.
   Maps: engine2040 wants a GPU proof that cannot pass by skipping; a log that collects errors (including leak-on-drop) and is asserted empty is a clean gate, and the "not installed -> not a failure, but then say so" rule is exactly the GPU-skip honesty rule.
   Kill: if "layer not installed" is ever recorded as a pass, or sync validation costs the arena more than a few percent -> gate on the collected error count only.

3. A provenance table keyed by each public formula, with a test that reads the file and fails on a missing entry | xuyanwen2012/vkrgss-rs@main:crates/vkrgss-pixel/src/arith.rs:1-45 | TOOLING (evidence) | engine2040 docs evidence culture (`docs/HASH-LAW.md`, perf baselines, EVIDENCE rows) | S
   Idea: `arith.rs` opens by declaring that **every** `pub fn` in the file has an entry in `PROVENANCE`, and that a test reads the source file and fails if one is missing (:1-2). Each entry is a struct `{name, formula, provenance}`, e.g. `{name: "effective_alpha", formula: "a = div255(alpha * opacity)", provenance: ReverseEngineered { source: "oracle measurement 2026-10-09, RGSS103J, screen blend probe" }}` (:35-45) — so a reverse-engineered number names the probe and the date that fixed it, and the notation is defined at the top of the file (:4-6).
   Maps: engine2040 has the same problem the RGSS blend maths has — numbers fitted from measurement — and its evidence lives in prose; a per-formula table that a test cross-checks against the source turns "why is this constant 0.005?" into a lookup that cannot rot silently.
   Kill: if the table drifts from the code (the test must read the file, as here), or a formula has no measurement to cite -> mark it explicitly rather than inventing a source.

== 4. NOT WORTH STEALING ==
- `crates/vkrgss-vk/src/compositor.rs:1-41` and `binning.rs`: a tile-binning compute compositor with its own descriptor set and `include_bytes!` SPIR-V. It is a device-specific optimiser for a fixed 2D draw stream; engine2040 is wgpu with its own pass graph, so the shape does not port.
- The embedded Ruby 1.8.7 + binding layers (`crates/vkrgss-ruby192-sys`, `vkrgss-binding*`): a whole interpreter host we neither need nor want.
- The `ash`/`gpu_allocator` Vulkan backend as a whole: no overlap with our `wgpu-render` path.

== 5. FILES READ ==
README.md, LICENSE, crates/vkrgss-vk/src/compositor.rs, validation.rs, frame_timing.rs, crates/vkrgss-pixel/src/arith.rs (all @main, 2026-10-18).
