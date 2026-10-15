# DECISIONS-M1 — layer defaults (proposed, not installed)

Source: `ROADMAP.md` adopt table. Licenses below are expected, not verified,
except where marked VERIFIED. `deny.toml` is authoritative. No manifest,
lockfile, or deny config was edited here. Parent adds crates behind features,
then re-runs `cargo deny check`. `DEPENDENCIES.md` section 6 holds the row
table for these decisions.

Conventions: each record lists choice, rejected alternative, why, and license.
Verify status PROPOSED means absent from `Cargo.lock`. EVIDENCE rows owed are
listed under Provenance and are not filed in this change (file budget).

## M1-1 Math

- Choice: `glam` for vectors, matrices, quats outside `core`.
- Rejected: hand-rolled math, `nalgebra`.
- Why: SIMD, no-std, small graph; fixed-step sim numerics stay in std-only `core`.
- License: MIT/Apache-2.0 expected. Status: PROPOSED.
- Proposal: `glam = "0.30"` under `[workspace.dependencies]`, consumed by scene/render adapters only.
- WIRED 2026-09-23: `glam = "0.33"` in workspace deps, `math` feature on scene; versions verified live via crates.io API (max 0.33.8, MIT OR Apache-2.0); all-features deny ok.

## M1-2 Window/GPU

- Choice: `winit` + `wgpu`, already in use via `engine-render`.
- Rejected: SDL2/glfw bindings, raw Vulkan surface code.
- Why: in the lock graph, cross-platform, matches the window-feature gate.
- License: `winit` Apache-2.0, `wgpu` MIT/Apache-2.0. Status: VERIFIED (window graph, deny ok).
- Proposal: no change; keep `window = ["engine-render/window"]` as the GPU gate.

## M1-3 Import

- Choice: `gltf` facade + `image` for meshes and PBR textures.
- Rejected: hand-rolled glTF, Assimp bindings, `ktx2` crate.
- Why: maintained Rust parsers for a complex format; KTX2 stays an offline
  bake with runtime lookup; `ktx2` crate stays banned per policy.
- License: both MIT/Apache-2.0 expected. Status: PROPOSED.
- Proposal: `gltf = "1"`, `image = "0.25"` in the assets lane only, feature-gated; cook cache and persistent IDs land in N4.
- WIRED 2026-09-23: both in workspace deps (`1`/`0.25`), assets `import` feature; versions verified live via crates.io API (gltf max 1.4.1, image max 0.25.10, both MIT OR Apache-2.0); all-features deny ok. Cook cache landed (cooker.rs); real mesh extraction still open in N4.

## M1-4 Physics

- Choice: `rapier3d` with `enhanced-determinism`, `parry3d` as query layer.
- Rejected: second backend alongside (Jolt), custom XPBD now.
- Why: one primary 3D backend; determinism feature supports the hash law;
  character glue and hash coverage are built here.
- License: Apache-2.0 expected. Status: WIRED 2026-09-23.
- WIRED 2026-09-23: `rapier3d = { version = "0.35", default-features = false,
  features = ["dim3", "f32", "std", "enhanced-determinism"] }` in workspace deps,
  physics `rapier` feature; versions verified live via crates.io API (max 0.35.3,
  Apache-2.0, rust 1.86); no SIMD/parallel features; targeted `cargo check` green,
  deny ok. `parry3d` stays the query layer behind it.
- BUMPED 2026-10-04 (sprint, Break lane): `rapier3d` 0.35.3 -> 0.36.0 (parry3d 0.30.2 -> 0.31.1, one new
  `num-derive`); `PhysicsPipeline::step` takes a `SoftBodySet` (empty, soft bodies unused); same features;
  physics `rapier` tests, arena `sim` tests and the 600-tick golden `d57b5c8ae3096614` x2 unchanged.

## M1-5 Editor UI

- Choice: `egui` + `egui-wgpu`/`egui-winit`, `egui_dock` for panels, `transform-gizmo-egui` for the gizmo.
- Rejected: imgui bindings, retained-mode framework, web UI, `egui_tiles` as primary (kept as fallback).
- Why: immediate mode fits the schema inspector; wgpu/winit adapters exist;
  dock covers viewport/hierarchy/inspector/log layout; every edit routes through N3.
- License: `egui` MIT/Apache-2.0 verified; `egui_dock` MIT (allow-listed, no row owed);
  `transform-gizmo-egui` + `transform-gizmo` MIT OR Apache-2.0 verified, provenance recorded
  in the NE1 receipt. Status: WIRED 2026-09-23 (`egui-wgpu`/`egui-winit` stay PROPOSED for E2).
- WIRED 2026-09-23: `egui = "0.36"`, `egui_dock = "0.21"`, `transform-gizmo = "0.11"`,
  `transform-gizmo-egui = "0.11"` in workspace deps, editor `ui` feature; versions verified
  live via crates.io API (egui max 0.36.2 MIT OR Apache-2.0; egui_dock max 0.21.1 MIT,
  requires egui ^0.36; transform-gizmo-egui max 0.11.0 MIT OR Apache-2.0, requires egui
  ^0.36; transform-gizmo max 0.11.0 MIT OR Apache-2.0). Rejected: `egui-gizmo`
  (license `non-standard`, stale at 0.16.2 targeting egui ~0.27) — fails the license gate,
  never depend. Targeted `cargo check` green, deny ok.

## M1-6 Authored files

- Choice: `serde` + `serde_json` canonical text; `ron` kept for configs.
- Rejected: YAML/TOML as canonical, binary codecs for authored files, hand-rolled parser.
- Why: versioned schema, migrations, canonical hashing; human-readable diffable saves.
- License: MIT/Apache-2.0 expected. Status: `serde` direct via scene, `serde_json` WIRED 2026-09-23; `ron` PROPOSED.
- WIRED 2026-09-23: `serde = { version = "1", features = ["derive"] }`, `serde_json = "1"` in workspace deps (max 1.0.229 / 1.0.151, both MIT OR Apache-2.0, verified live via crates.io API); scene `json` feature; targeted check + deny ok. `ron` stays for configs (later).

## M1-7 IDs and digests

- Choice: `uuid` for persistent IDs, `blake3` for content/cook digests.
- Rejected: content hash as asset ID, FNV as content digest, hand-rolled digest.
- Why: reimport must retain references; artifact integrity needs a cryptographic digest.
- License: `uuid` MIT/Apache-2.0 expected; `blake3` Apache-2.0/CC0-1.0 expected (both on allow-list). Status: PROPOSED.
- Proposal: `uuid = { version = "1", features = ["v4", "serde"] }`, `blake3 = "1"` for assets/scene/save lanes.

## M1-8 Audio

- Choice: `cpal` for IO, `lewton` + `hound` + `claxon` for decode.
- Rejected: `symphonia` (banned, MPL-2.0), full mixer crate now.
- Why: cross-platform backends; the trio covers vorbis/WAV/FLAC without the banned dep; mixer stays hand-built in `audio`.
- License: `cpal` Apache-2.0 verified; `lewton` MIT/Apache-2.0 verified; `hound` Apache-2.0 verified (table said MIT); `claxon` MIT expected. Status: cpal/lewton/hound WIRED 2026-09-23, claxon PROPOSED.
- WIRED 2026-09-23: `cpal = "0.18"`, `lewton = "0.10"`, `hound = "3.5"` in workspace deps (max 0.18.2 / 0.10.2 / 3.5.1, verified live via crates.io API), audio `device` feature; targeted check + deny ok. `claxon` (FLAC) stays for AU1 if the worker needs it.

## M1-9 Netcode model (decided 2026-09-23)

- Choice: server-authoritative match with client prediction and rollback over
  UDP (`std::net` first), AI filling empty slots; reference design CrystalOrb.
- Rejected: pure P2P rollback (GGRS) as the model — no single authority for AI
  slots and local guards; Lightyear — bound to Bevy's ECS, pattern only.
- Why: `FOCUS.md` asks for server-authoritative LAN fights with rollback; the
  headless `arena` match is the same code on server and client.
- Status: planned, board tasks AR1, AR2, NET1, NET2.

## M1-10 Determinism tier (decided 2026-09-23)

- Choice: tier 3 for the arena simulation, as ruled in `HASH-LAW.md` (one
  release binary, x86-64 Windows players, Linux CI cross-check).
- Consequence: no platform transcendental float math in simulation crates;
  Rapier with `enhanced-determinism` and no SIMD/parallel features.
- Status: audit reports violations now (`physics/src/vehicle.rs`); task D1
  makes the audit fail on them.

## M1-11 MCP SDK

- Choice: official `rmcp`, negotiated protocol (2026-07-28 reference). The
  upstream repo is in an MIT/Apache-2.0 transition; pin/check the released crate
  separately at NM1. Source receipt in `EVIDENCE.md`, not a dependency pass.
- Rejected: hand-rolled JSON-RPC; community SDKs.
- Why: long-running tasks fit play.run, subscriptions fit scene changes, and
  spec upgrades come with the SDK. Status: WIRED 2026-09-23 (NM1).
- WIRED 2026-09-23: `rmcp = { version = "3.4", default-features = false, features = ["server", "transport-io", "macros", "schemars"] }` in workspace deps, endpoint `mcp` feature; released crate verified live via crates.io API (max 3.4.0, Apache-2.0 SPDX — the repo MIT/Apache transition is not the receipt); stdio transport is `transport-io` (tokio/io-std), confirmed on docs.rs feature list; targeted check + deny ok.
  No dependency on deprecated roots/sampling/protocol logging; engine logs use
  its own bounded tools. No arbitrary execution tool in the first door.

## M1-12 Render references

- Choice: port proven WGSL (PBR, atmosphere, bloom, TAA, contact shadows,
  screen-space reflections, area lights) from Bevy 0.18/0.19 (MIT/Apache),
  one `EVIDENCE.md` row per ported file; temporal upscaling via the wgpu port
  of AMD FSR 3.1 (MIT) once its license is verified.
- Rejected for now: hardware ray-traced lighting and radiance-cascade GI —
  over the 60 fps budget on the reference GPU; LATER (V4).
- Status: board tasks R1–R6.
  Implementation must pin the actual Bevy tag/file and FSR3.1 wgpu-port URL,
  license and revision. FSR1 evidence does not clear that port; it remains
  unverified. A version named in this decision is not evidence of a port.

## M1-13 New crates

- `arena` (headless match), `provider` (AI service), `playtest` (AI QA
  agents), `game` (packaged app). The orchestrator creates each crate with its
  C-task, documents the dependency edges in `tools/workspace-check.mjs`, and
  keeps them out of the default headless graph unless std-only.

## Provenance owed (not filed here)

- `transform-gizmo-egui`, `uuid`, `blake3`, `gltf`, `image`, `rapier3d`, `egui` family,
  `serde_json`, `cpal`, `lewton`, `hound`, `claxon`, `rmcp`, each ported Bevy WGSL
  file and the FSR 3.1 wgpu port: expected licenses above need a
  registry/file read before depend; file the `EVIDENCE.md` row per crate at depend time.
- `egui_dock`, `egui_tiles`, `wgpu` need no row (allow-listed MIT/Apache path).

## Open

- Confirm `egui_dock` over `egui_tiles`; confirm JSON canonical with RON for configs.
- Parent picks exact versions/features, adds behind features, re-runs deny.
- N1 full acceptance (crates behind features + deny) waits on that parent change.
