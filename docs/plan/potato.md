# Potato tier and steals — plan to the VISION.md bar

Plain summary: every engine2040 game must look great and run on a weak PC (weak GPU, weak RAM, weak CPU). The owner chose Rust for this (2026-10-11). The 3050 / 16 GB box is the comfortable tier; the floor is an integrated GPU or a 2 to 4 GB card with 4 to 8 GB of RAM. This plan also holds the gates that stop finished modules from staying unwired, the free-content fetch, and the small steals from 74 scouts.
Owner lanes: render, perf, assets, tools, game, ai, spatial, combat, endpoint. Prefix: GS. Base commit: a1dfeb83.
Research: `docs/research/steals-2026-10-11/engine-landscape.md` (both waves). Ideas not yet rows: `landscape-ideas.md`.

## 1. Where it stands

| Capability | Status | Evidence |
|---|---|---|
| Frame graph as data plus executor | implemented, unit-tested; passes unwired in the live window | `crates/render/src/graph.rs`, `executor.rs`; FOCUS group 1 (LF1-LF7, EG8) |
| Dynamic resolution, SMAA fallback | implemented, not driven by a governor | `crates/render/src/aa_scale.rs` (hysteresis on PF2 frame ms); OPT20 open |
| Startup lighting tier | implemented (Base, Rt, Ultra), no low-end tier | `crates/render/src/tier.rs`; PLF10 (game presets) open |
| Adapter choice | every site asks for `HighPerformance`; no adapter log, no CPU-adapter guard | `present/gpu_present/presenter.rs` (3 sites), `backend.rs`, `gpu.rs`, `aa_scale.rs`, `baked_gi.rs` |
| Pipeline cache, async compile | implemented; no DX12 serialization | `crates/render/src/cache.rs` (wgpu #10547, #8526) |
| GPU pass timestamps | implemented | `present/gpu_present/timing.rs`; `tools/product-check.mjs` reads `name=Xms` |
| Counting allocator | implemented, no peak gate | `crates/perf/src/alloc.rs`, `tests/alloc.rs` |
| Asset 7-gate import | implemented, no size caps | `crates/assets/src/import.rs`, `bc.rs` (BC1/BC3 cook) |
| Steam, crash, capture, director camera | implemented behind null backends | `game/src/steam.rs`, `crash.rs`, `capture.rs`, `director.rs` |
| Content (meshes, textures, clips, audio) | placeholders only | `audio/src/bank.rs`, `content/` |
| Potato run on the Intel UHD | missing | none |

Code beats docs: most wave-1 "build X" ideas already exist as modules. The gap is wiring, proof and the low tier.

## 2. Finish line

Tier A (rows GS1 to GS16): a wiring scan and pass floors the loop prints; an adapter chosen on purpose and logged; the weakest wgpu limits proven; one quality table and one GPU rule table; a dyn-res governor; a VRAM budget; a peak-heap gate; asset caps; a `potato:` line from the Intel UHD.
Tier B (rows GS17 to GS33): LOD dither fade, GPU cull, wound maps, crowd vertex-animation textures; the fetch tool and two fetch lists; Blender cook; replay to video; camera arm; AI reaction delay, hearing, alert ladder; isovist; gladiator classes; telegraph report; `play.sweep`.
Tier C: gate G9 (finish line): arena on the Intel UHD with downlevel limits at proposal p50 50 ms, p99 80 ms, working set 3 GB; heap peak flat; assets inside the potato caps; the Potato knobs table drives PLF10. All numbers are proposals until the owner sets them.

## 3. Target design

One direction, no cycles. Presentation reads these; the fixed tick never does, and none of them enters the hash.

```
adapter list -> choose (GS6, env E2040_ADAPTER, never CPU) -> tier_rules (GS11) -> Quality (GS10 knobs)
  -> dynres governor (GS12) reads GPU ms (timing.rs) -> render scale
  -> vram budget (GS13) reads the OS query -> streaming pool / mip bias
  -> asset caps (GS15) at import; draw and tri counts in the `passes` line (GS3)
potato-check (GS16) runs the arena on the UHD with downlevel limits and prints one line
```

Budgets. Tier probe and rule match: O(1) at startup. Governor: O(1) per frame, integer. VRAM query: once per second. Wiring scan: pure text, no build. Coverage run: nightly only, never in the 40-seat loop (weak CPU).
Determinism. Quality, tier, adapter, dyn-res and VRAM are presentation. They must not feed `state_hash`. Tests assert the golden `d57b5c8ae3096614` is unchanged.
Weak CPU. Cook-time work moves to the GPU where it exists (BC compression, IBL prefilter); encode video with NVENC when present; one build per crate; no instrumented builds in the loop.

**Steals 2026-10-12** (board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`). Feature ladder with three modes (none, preprocess-only, culling) stamped in the manifest; potato assumes no count draws and no bindless (GS35). Quality knobs are WGSL override constants, not copies (GS36). The live frame creates only the targets it reads (GS34). Potato ladder: scale 0.67 with EASU and RCAS, SMAA, the neutral tonemap, lightmap plus ambient volume, one sun map, 4 torches, no GTAO, no froxel fog, far crowd as 8-byte impostors.

## 4. Hard problems

| Problem | Technique | Why | Donor |
|---|---|---|---|
| Hybrid laptop renders on the iGPU | choose from the adapter list, export `NvOptimusEnablement`, log it | `HighPerformance` returned Intel on Optimus | wgpu #257; GLFW `win32_init.c` |
| Silent software rendering | refuse `DeviceType::Cpu` unless allowed | Bevy #13113 | wgpu PR #5535 |
| DX12 heap corruption in parallel pipeline creation | gate creation to 1 in flight on Dx12 | FXC blob refcount race | wgpu #10547 |
| Cold-start hitch on DX12 | build all pipelines in the loading screen | no PIPELINE_CACHE on DX12 | wgpu #8526 |
| One game, three machines | one quality table plus a GPU rule table | first match wins; data an AI can edit | Unreal sg.* level 0; Chromium `gpu_control_list` |
| Frame time spikes | deadband, cooldown, panic drop, slow recovery | stable resolution | Unreal `r.DynamicRes.*` |
| Over-budget VRAM stutter | budget = (budget - reservation) x 0.95 | OS pages out above budget | Dawn residency manager |
| Memory creep | peak-heap ratchet with the existing counting allocator | 4 to 8 GB RAM floor | dhat-rs idea |
| Finished module never reached | scan for islands; floor each look pass; fake-pass guard | the recurring failure | cargo-workspace-unused-pub; EvilGenie |
| Empty content folder | scripted CC0 / CC-BY fetch with sidecars | biggest "richer" lever | Poly Haven API, ambientCG API |

## 5. Donors

Donors: steals.md; porting policy is AGENTS rule 4. The per-row donors are in each row and in the research report.

## 6. Slim

Do not build: Solari-class ray tracing, SDFGI, DirectStorage, a second GL renderer, neural texture compression at sample time, Cranelift dev builds (no unwinding on Windows), mimalloc or rpmalloc without a Windows measurement, full Steam Audio, cloud crash servers. Tuning the proposal numbers is the owner's call; the rows only measure and print them.

## 7. Board tasks

Rows from the steals (2026-10-12, board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`): GS34 live-frame hygiene, GS35 feature ladder, GS36 override constants.

Rows live in `docs/SPRINT-18-steals.md` (not repeated here). FOCUS Now group 7 holds them. IDs, crate, one line:

| ID | Crate | Line | needs |
|---|---|---|---|
| GS1 | tools | wiring scan: pub items with no production user, baseline ratchet | none |
| GS2 | tools | nightly coverage of the real arena run (cargo-llvm-cov), report only | none |
| GS3 | tools | product-check pass floors (`pass-bar.json`) | none |
| GS4 | tools | fake-pass guard in packet-gate | none |
| GS5 | render | one pipeline at a time on Dx12 | none |
| GS6 | render | adapter chooser, never the CPU adapter | none |
| GS7 | game | export Optimus and PowerXpress symbols | GS8 |
| GS8 | tools | read a PE export table | none |
| GS9 | render | `E2040_LIMITS=downlevel` | GS6 |
| GS10 | render | quality table (Potato, Comfort, High) | none |
| GS11 | render | GPU rule table to tier | GS10 |
| GS12 | render | dyn-res governor | none |
| GS13 | perf | VRAM budget math | none |
| GS14 | perf | peak heap gate | none |
| GS15 | assets | potato asset caps | none |
| GS16 | tools | `potato-check` on the Intel UHD | GS6 |
| GS17 | render | LOD dither cross-fade | none |
| GS18 | render | GPU frustum cull (hard) | none |
| GS19 | render | wound maps with a budget | none |
| GS20 | assets | vertex-animation textures for the crowd | none |
| GS21 | tools | `fetch-content` | none |
| GS22 | content | fetch list v1 (CC0 textures, HDRIs, props) | GS21 |
| GS23 | content | fetch list v2 (Roman models, crowd audio) | GS21 |
| GS24 | tools | headless Blender cook | none |
| GS25 | tools | replay to video (NVENC if present) | none |
| GS26 | game | camera arm sphere sweep | none |
| GS27 | ai | reaction delay ring | none |
| GS28 | ai | hearing events | none |
| GS29 | ai | alert ladder | none |
| GS30 | spatial | isovist | none |
| GS31 | combat | gladiator class rows | none |
| GS32 | combat | telegraph floors report | EG19 |
| GS33 | endpoint | `play.sweep` | LF16 |

Row changes: none to existing rows. PLF10, OPT20, OPT18, RL5, LF9, LF10 and E32 stay as they are; GS10, GS11, GS12 and GS13 are the data they will read.

## 8. Quality gates

- `node tools/workspace-check.mjs` PASS (board ids unique, needs resolve).
- Golden `d57b5c8ae3096614` unchanged by every row; hash x2.
- `node tools/wiring-scan.mjs --check` exits 0 (island count never rises); the pin lives in
  `tools/wiring-scan.baseline.json`, and a root that is not a directory exits 2 instead of
  passing on zero islands.
- `node tools/potato-check.mjs` prints the `potato:` line; with `--floor` it exits 0 once the owner accepts the numbers.
- `cargo test -p engine-perf` heap peak within 10% of the baseline.
- Every new pure module has a test that fails before and a `x2 equal` check.
- GPU tests skip loudly (via `test_gpu.rs`) when no adapter exists; they never pass silently.

## 9. Session estimate

Tier A: 16 rows, mostly S, 2 to 3 weeks of Muse waves on this CPU. Tier B: 17 rows, 3 to 4 weeks; GS18 needs one Claude session. Tier C: depends on the lit-frame work (FOCUS group 1) landing first, because the potato run measures the real frame.
