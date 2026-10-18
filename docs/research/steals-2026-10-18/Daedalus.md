# Steal card: sonan0721/Daedalus (2026-10-18)

## What it is

An independent, code-first game engine in Rust (its own words: "engine foundation under construction"). A Rust workspace with a parallel ECS runtime, bounded CPU tasks and snapshots, a persistent JSON CLI control surface, an independent wgpu renderer, Jolt/Rapier physics adapters, Kira audio, and a .NET gameplay host. 11 crates (`daedalus-core`, `daedalus-runtime`, `daedalus-tasks`, `daedalus-renderer`, `daedalus-content`, `daedalus-world`, `daedalus-cli`, …) plus a large architecture-doc tree. 0 stars, 2.2 MB tree, HEAD `261812a0048f` (2026-10-18T01:05Z), language Rust. Read live 2026-10-18 (crates, docs listing; read-only). Nothing built or run.

## Licence

**Apache-2.0 OR MIT** — `LICENSE-APACHE` first lines "Apache License / Version 2.0, January 2004" and `LICENSE-MIT` first line "MIT License" (both read live today); GitHub API `license.spdx_id` = `Apache-2.0`.

## Steals (ranked by value per hour)

1. **An exact rational fixed clock with retained catch-up debt and bounded work.** `crates/daedalus-runtime/src/clock.rs:9-17` (the internal advance data: first tick, requested/executed/deferred ticks, elapsed nanos, input and tick-counter saturation flags), `:19-32` (`FixedClockConfig { tick_rate_hz, max_catch_up_ticks }`, default 60/4, both required positive), `:42-53` (`ClockAdvance`: requested vs executed vs deferred ticks, `fractional_numerator`, `input_saturated`, `tick_counter_saturated`), `:55-101` (`advance` multiplies elapsed nanos by the rate in checked `u128`, saturates the accumulator on overflow, consumes at most `max_catch_up_ticks` and retains the rest as debt).
   - Maps onto engine2040: `crates/core/src/time.rs:56-64` (`IntAcc`: `acc_ns` + `rem_thirds`, "when the step cap is hit, banked whole steps are forgiven … `tick` is the truth; wall input never enters a hash") is the same idea in integers. The steal is the **observable advance breakdown** — requested/executed/deferred and the saturation flags — which makes a hitch debuggable without any of it entering the hash.
   - **Kill:** the fractional numerator must never enter a simulation hash (IntAcc already says wall input never does), and the golden at tick 600 must be unchanged.

2. **A one-slot snapshot mailbox that skips instead of waiting.** `crates/daedalus-core/src/snapshot.rs:8-21` (`RenderSnapshot { sequence, tick, Arc<[RenderEntitySnapshot]> }`: an owned immutable view for presentation), `:23-35` (a one-slot latest-value mailbox with `max_retained`, where retained generations are held as `Weak` so a dropped consumer frees the cap), `:47-52` (`PublicationStatus::Published` / `SkippedAtCapacity` / `SkippedAtEntityLimit`), `:72-110` (`try_publish` returns `Skipped…` rather than blocking a slow consumer; readers stay nonblocking and report `Busy`; extraction runs only after the capacity and entity-limit checks pass).
   - Maps onto engine2040: the sim→presentation handoff and "off-thread results at tick boundaries" (`tick-and-runtime`). `crates/ecs/src/snapshot.rs` is the state dump, not the handoff mailbox.
   - **Kill:** a skipped publish must be presentation-only — it must never change sim state or the golden, and a skipped frame must leave the last-good view intact.

3. **Bounded CPU task admission with cooperative cancellation.** `crates/daedalus-tasks/src/lib.rs:1` (bounded CPU work admission, cooperative cancellation, owned results), `:38-72` (typed errors: `TaskPoolError::NoWorkers`/`NoCapacity`; `SubmitError::Full`/`Closed`/`Busy`/`IdExhausted`; `ReceiveError::Busy`/`Poisoned`; `ShutdownError::CalledFromWorker`), `:74-129` (a `CancellationToken` is an `Arc<AtomicBool>` with Acquire/AcqRel; `TaskHandle::cancel` is a *request* and running code must check its token).
   - Maps onto engine2040: `crates/ecs/src/pool.rs` (`JobPool`) and `jobs.rs`. The steal is the typed admission result — `Full` vs `Busy` vs `Closed` lets a caller decide instead of blocking — plus a cancellation token the work actually checks.
   - **Kill:** `Busy` must be retried a bounded number of times or it becomes a spin; no admission error may panic inside the tick.

4. **A GPU owner phase machine that never allocates after handles return.** `crates/daedalus-renderer/src/gpu_owner.rs:8-17` (`WindowGpuOwnerPhase`: `NoDevice` → `DeviceRequest` → `RendererPartial` → `RendererReady` → `PacketRendererPartial` → `PacketRendererReady`), `:19-45` (one inline fixed `OwnerState` union inside a preallocated `Box` — "boxing each transition would allocate after GPU handles have returned" — and an opaque move-only owner that never owns a window or surface), `:53-95` (the owner preallocates on `new`; `initialize_renderer` keeps the phase at `DeviceRequest` on a request error and retains every later handle in `RendererPartial` until promotion to ready).
   - Maps onto engine2040: the renderer/device lifecycle (`crates/render/src/adapter.rs`, `present/gpu_present/presenter.rs`) and the `wgpu-render` skill's device-loss path.
   - **Kill:** any allocation on the device-lost or partial-init path fails the row, and the phase must be observable (a no-device test must read `NoDevice`).

## Not read

`crates/daedalus-runtime/src/session_control.rs` (379 KB), `live_session.rs` (97 KB), `crates/daedalus-renderer/src/packet_renderer.rs` (173 KB), `packet.rs` (204 KB), `crates/daedalus-project-host/src/host.rs` (131 KB), most `docs/03-architecture/*`.

DONE 4 steals, donor files read live 2026-10-18. Nothing built or run.
