# NVIDIA and the graphics field, what we can actually use (2026-10-23)

Scope: what NVIDIA and the wider field ship in 2026 that a Rust + wgpu 30 engine
can use, either at cook time (offline tools and libraries) or in the tick/draw
path. Written for our tree at `C:\engine2040`, so every "fits" line names the
crate and the file that would change.

Number labels, same convention as `docs/research/locomotion-fast-2026-10-23`:

- **measured** = read by me from a file or page on 2026-10-23 (our tree, the
  donor's licence text, or a doc page), with the URL;
- **donor claim** = the donor's own sentence, no number of ours;
- **third-party measured** = someone else's benchmark, quoted with its URL;
- **estimate** = my arithmetic or guess, and the line after says what would
  measure it.

Nothing here was built or run: no vendor SDK was downloaded, no capture was
taken. Every "gain" below is a target, not a result.

## Verdict

There are exactly **three** things worth taking from this whole field this
quarter, and two of them are already half-built inside our tree:

1. **GPU-driven cluster culling** (Karis's Nanite hierarchy, plus Haar and
   Aaltonen's two-phase occlusion). We already cook meshlets with normal cones
   (`crates/assets/src/meshlet.rs`, `meshlet_cull.rs`) and already have an
   oracle-checked compute cull that writes indirect args
   (`crates/render/src/gpu_cull.rs`), and **neither is called by the draw path**:
   `present/gpu_present/submit_cull.rs` still culls on the CPU with per-draw
   spheres. The steal is the wiring plus an HZB, not new research.
2. **BC7 mode selection in the cook.** Our encoder writes mode 6 only and
   ignores the slot (`crates/assets/src/bc.rs:608`, `encode_bc7_block(texels,
   _slot)`). Every BC7 block is 16 bytes, so a better mode is free bytes and
   better pixels — it buys look (G4) at zero bundle cost.
3. **An external GPU-timing oracle** (Nsight Graphics or Nsight Systems, free
   tools, no code) because our 1080p speed gate is currently argued from
   in-engine timestamps only.

Everything else in the list is either **blocked by wgpu's feature set** (VRS,
DLSS, XeSS, NGX-based anything), **blocked by licence** (RTXGI/NTC/Mega
Geometry are under the proprietary NVIDIA RTX SDKs licence, not MIT), or **too
large for one scene** (USD, Omniverse Kit, PhysX-vs-Rapier, neural materials).

## 1. Cook-time tooling: what is licence-clean

| tool | licence (read live) | what it does that we lack | cook or tick? | take? |
|---|---|---|---|---|
| NVTT 3 (NVIDIA Texture Tools) | samples repo Apache-2.0; the NVTT 3 SDK itself is a separate download from developer.nvidia.com | real BC7 encoder with quality levels, GPU BC7 encode/decode, normal-map (slope-space) handling | cook | **yes, as an oracle only** (S1) |
| OpenUSD + usdview | "Tomorrow Open Source Technology License 1.0" = Apache-2.0 with a different trademark clause | scene interchange with DCC tools, layered scenes, usdview inspection | cook | no, `glTF` is enough (S2) |
| PhysX 5 (5.12) | BSD-3-Clause | a mature C++ solver, GPU rigid bodies, cloth, particles | tick (never) | no (S3) |
| Warp | Apache-2.0 | JIT Python kernels (CPU or CUDA) for batch numerics, USD I/O, differentiable sims | cook | maybe, sandbox only (S4) |
| Omniverse Kit | proprietary Omniverse licence, app-scale SDK | editor shell, USD-native app framework | neither | no (S5) |
| Nsight Graphics / Systems / Aftermath | free, proprietary tool; never redistributed | per-pass GPU ms, shader hot spots, GPU mini-dumps | measurement | **yes** (S6) |

### S1. NVTT 3 as the quality oracle for our BC7 cook (S)

- **What**: run `nvcompress`/the NVTT `compress` sample over the same five CC0
  1K arena maps our cook encodes, and compare bytes, PSNR and encode ms against
  `crates/assets/src/bc.rs`.
- **Source**: samples and licence,
  https://github.com/nvpro-samples/nvtt_samples (LICENSE read live: Apache-2.0);
  the library itself, https://developer.nvidia.com/gpu-accelerated-texture-compression
  (a download, not a repo — its own terms apply, so treat it as a **tool we run**,
  never a linked dependency).
- **Licence**: the sample code we would read is Apache-2.0; the NVTT 3 binary we
  would *run* is NVIDIA's SDK download. Running a tool and comparing outputs is
  clean; linking `nvtt` into `engine-assets` would drag a C++ SDK and its terms
  into our tree. So: oracle, not dependency.
- **Why it fits**: our BC7 is one mode (`crates/assets/src/bc.rs:603-608`,
  mode 6, one subset, `_slot` ignored) and our cook picks the format in
  `crates/assets/src/texture_cook.rs:35-42` (`pick_format`). A single PSNR line
  per map tells us whether the missing modes are worth writing.
- **Cost**: S. Author time ~half a day. CPU: one NVTT run per map. GPU: none.
- **Replaces**: nothing; it is the *measuring stick* for the BC7 work in S7.
- **Prove or kill**: one command per map over the five CC0 maps; record
  (bytes, PSNR-RGB, max channel error, ms). **Kill** if our mode-6 output is
  within 0.3 dB of NVTT *and* the encode is under 50 ms/map: then S7 is not
  worth writing.

### S2. OpenUSD + usdview as asset intake (skip, with a trigger)

- **What**: a scene interchange format with references, layering and a viewer.
- **Source**: https://github.com/PixarAnimationStudios/OpenUSD ,
  `LICENSE.txt` read live (`release`): the file is headed "Tomorrow Open Source
  Technology License 1.0" and states it "differs from the original Apache
  License 2.0 in the following manner. Section 6 (Trademarks) is different."
- **Licence**: licence-clean for us (Apache-2.0 shape, nominal trademark clause).
  That is not the problem.
- **Why it does not fit**: our intake is glTF through
  `crates/assets/src/gltf.rs` + `kit_import.rs` + `import.rs`, and every mesh,
  skin and clip we own is already cooked through that path. USD buys us DCC
  round-tripping we do not do, at the cost of a C++/Python stack (hundreds of MB
  of build) on a tree that must stay wasm-checkable.
- **Cost**: L (author weeks, a new native dependency, a new cook lane).
- **Trigger to re-open**: the first asset we cannot get as glTF (a Houdini or
  Maya scene handed to us directly). Not before.
- **Prove or kill**: if the trigger fires, one conversion test — one USD stage
  with two references and one variant set, flattened by `usdcat`, must produce
  the same cooked bundle as the glTF of the same model.

### S3. PhysX 5 instead of Rapier (no)

- **Source**: https://github.com/NVIDIA-Omniverse/PhysX — README read live
  carries the BSD-3-Clause text inline (Copyright 2008-2025, three clauses) and
  lists PhysX SDK 5.12.0. The GPU page
  (https://nvidia-omniverse.github.io/PhysX/physx/5.6.1/docs/GPURigidBodies.html)
  states GPU simulation "is implemented in CUDA and requires SM6.0 (Pascal) or
  later".
- **Licence**: BSD-3-Clause, clean. Again not the blocker.
- **Why it does not fit**: GPU PhysX means CUDA, which is outside wgpu and
  outside wasm; CPU PhysX would replace `crates/physics` (a deterministic,
  integer-friendly, wasm-proven layer behind golden `d57b5c8ae3096614`) with a
  C++ solver whose float order we do not control. The gates that would break are
  G6 (same hash on wasm32 and three native targets) and the G9 potato run.
- **Cost**: L. Nothing to gain on the 8-fighter scene; the wins are cloth and
  GPU particles, which we do not need yet.
- **Replaces**: `crates/physics` — which is exactly why it is a no.
- **Prove or kill**: not worth an experiment. Re-open only if a G11-class scene
  (8 ragdolls + 200 debris) exceeds budget on the CPU solver *and* the wasm leg
  is already cut by the owner.

### S4. Warp as an offline numerics sandbox (M, hold)

- **What**: Python functions JIT-compiled to CPU or CUDA kernels, with USD I/O.
- **Source**: https://github.com/NVIDIA/warp — Apache-2.0 (repo metadata read
  live), README: "JIT compiles them to efficient kernel code that can run on the
  CPU or GPU"; PyPI wheels want an R580+ driver and Turing+ for the CUDA path.
- **Licence**: Apache-2.0, clean as a tool.
- **Why it fits, narrowly**: it is the only *free* way to get a 1000× batch of
  something (contact sweeps, drop tests, cloth settling) without writing a
  vectorised Rust env — which is the stated blocker in `docs/plan/hard-systems.md`
  ("Agent gym (vectorized headless envs) | missing"). It would generate reference
  tables for tests, never the tick.
- **Why it does not fit yet**: it is a second physics engine, in Python, with its
  own float behaviour. Using it as an *oracle* for our Rapier results requires
  proving the two agree on a simple drop first, which we have never done.
- **Cost**: M. Author ~1 week to get one honest fixture through it; CPU path
  works without a GPU; CUDA path needs this PC's RTX 3050 + driver.
- **Replaces**: nothing.
- **Prove or kill**: one fixture — the existing 8-body drop test re-implemented
  in Warp, compare first-contact tick and rest height against
  `crates/physics` on the same numbers. **Kill** if the two disagree on the
  contact tick without an obvious cause; then Warp is a pre-visualiser, not an
  oracle, and we stop.

### S5. Omniverse Kit (no)

- **What**: NVIDIA's USD-native app/editor framework; Kit extensions are the
  plugin model.
- **Source**: https://docs.omniverse.nvidia.com/kit/docs/kit-manual/latest/guide/kit_overview.html
  (read live; the page is JS-rendered, so this is a citation of the product
  overview, not of a licence clause I could read as text).
- **Licence**: proprietary Omniverse terms, not on our allow-list. **Not
  verified line by line** — see "Still unverified".
- **Why it does not fit**: it is an application shell. We have `crates/editor`
  and an MCP door; adopting Kit would mean shipping an NVIDIA runtime with the
  game.
- **Cost**: L, plus a licence decision only the owner can make.
- **Verdict**: reject; no experiment.

### S6. Nsight as the external timing oracle (S, take)

- **What**: frame capture with per-event GPU time, shader hot spots, GPU
  mini-dumps on TDR.
- **Source**: https://developer.nvidia.com/nsight-graphics — read live: "a
  standalone developer tool ... to debug, profile, and export frames built with
  Direct3D, Vulkan, OpenGL, and OpenXR", plus GPU Trace and Aftermath in the same
  suite.
- **Licence**: free to use, proprietary; we never redistribute it, so it is a
  clean measurement tool (the same standing as RenderDoc).
- **Why it fits**: G2 is our weakest number — the product run reads "1080p at 8
  fighters and the lit-frame speed are still unmeasured" and G3's pass floors
  "read pending". Our only timing today is our own
  `present/gpu_present/timing.rs` ledger. wgpu on Windows submits D3D12, so a
  DX12 capture *should* see our passes; that is an assumption, not a fact
  (see "Still unverified").
- **Cost**: S. No code. One afternoon: capture one arena frame at 1920×1080,
  8 fighters, and file the per-pass ms next to `target/reports/pass-truth-arena.md`.
- **Replaces**: nothing; it audits the ledger we already print.
- **Prove or kill**: the capture must name our forward, depth, GTAO, shadow and
  post passes with non-zero GPU time and total within 10% of the in-engine
  frame number. **Kill** if wgpu's DX12 submission does not appear (then fall
  back to Nsight Systems for CPU-side and our own timestamps for GPU).

## 2. Rendering: culling, VRS, upscalers

### S7. GPU-driven cluster cull with normal cones and an HZB (L, the big one)

- **What**: one compute pass over cooked meshlet clusters that (a) rejects
  clusters outside the frustum, (b) rejects back-facing clusters by the cone test
  `dot(cone_axis, eye - center) > cone_cos` scaled by radius, (c) rejects
  occluded clusters against a hierarchical depth buffer from the previous frame,
  and (d) writes one indirect draw block, so the CPU submits one draw for the
  whole crowd.
- **Sources**:
  - the technique and its rules (cluster hierarchy, per-cluster LOD, cone
    culling, "persistent threads"): *A Deep Dive into Nanite Virtualized
    Geometry*, Karis, Stubbe, Wihlidal, SIGGRAPH 2021 Advances —
    https://advances.realtimerendering.com/s2021/index.html (syllabus read live;
    the talk is in Part I, 10:10). **Correction to the brief**: this is 2021, not
    2018. The 2018 course syllabus
    (https://advances.realtimerendering.com/s2018/index.htm, read live) has no
    Karis talk at all — Ubisoft, Sledgehammer, Epic (Abadie, bokeh), Unity,
    NVIDIA (Pharr), Lucasfilm. Cite 2021.
  - two-phase occlusion culling and GPU-generated draw calls: *GPU-Driven
    Rendering Pipelines*, Haar & Aaltonen (Ubisoft), SIGGRAPH 2015 Advances —
    https://advances.realtimerendering.com/s2015/index.html (syllabus read live,
    Part II 4:20).
- **Licence**: talks are ideas. No code is taken; our cook and cull are ours.
- **Why it fits, precisely**:
  - the cook already exists: `crates/assets/src/meshlet.rs` (cluster split at
    `MAX_MESHLET_VERTICES` 64 / `MAX_MESHLET_TRIANGLES` 124, bounds with
    `cone_axis`/`cone_cos`, `cook_meshlet_lod` with `LOD_RATIOS`
    `[1.0, 0.5, 0.25, 0.125]` and `LOD_MAX_ERROR` 0.25 — **measured** in the
    file);
  - the cone test already exists: `crates/assets/src/meshlet_cull.rs:144`
    `cone_culled(bounds, eye)` and `cull_meshlets` at :213, with an integration
    test (`crates/assets/tests/it/meshlet_cull_vectors.rs`);
  - the GPU cull already exists: `crates/render/src/gpu_cull.rs` (workgroup 64,
    5-word indirect args, a CPU oracle and `Mismatch` refusal, shares
    `shaders/cull.wgsl` with the CPU pin);
  - **the gap is the call site**: `gpu_cull::GpuCuller` has no caller outside its
    own file (**measured**: a tree-wide grep for `GpuCuller` finds only
    `gpu_cull.rs`), and `present/gpu_present/submit_cull.rs` culls per draw on
    the CPU with world spheres (`sphere_visible_cpu`), skipping skinned meshes
    because a posed bound is unknown;
  - the thing that needs it: `crates/render/src/crowd_inst.rs` caps
    `MAX_CROWD_SEATS` at **50,000** (**measured**), with 3 LOD bands at 18 m and
    45 m and a `CROWD_INST_BUDGET_MS` of 0.5.
- **wgpu 30 facts that decide the shape** (read from
  https://docs.rs/wgpu/30.0.1/wgpu/struct.Features.html, **measured**):
  `MULTI_DRAW_INDIRECT_COUNT` exists ("Vulkan 1.2+ (or
  VK_KHR_draw_indirect_count)"), `INDIRECT_FIRST_INSTANCE` exists, and
  `EXPERIMENTAL_MESH_SHADER` exists but is experimental. So: **do not** build on
  mesh shaders (experimental, and D3D12-only in practice); build on
  compute + `multi_draw_indirect_count`, which is our existing `gpu_cull` shape.
- **Cost**: L. GPU: one compute pass, ~1 dispatch per frame over clusters; CPU:
  the per-draw sphere loop goes away (that is the point). Author time: ~2 weeks
  for cone+HZB+indirect wiring and a 50k-seat fixture, because the CPU oracle in
  `gpu_cull.rs` is the acceptance test and must keep passing.
- **Replaces**: the CPU sphere cull in `submit_cull.rs` for instanced/crowd
  draws, and it gives `crates/assets/src/meshlet.rs` its first consumer (today:
  cook + tests only).
- **Prove or kill**: one fixture — 50,000 seats, one camera path of 300 frames;
  measure (a) clusters tested, kept and drawn, (b) cull ms p50/p95, (c) identical
  pixels for the cone-only A/B against the CPU cull (the sim hash must not
  move: this is presentation only). **Kill** if the cull pass costs more than the
  0.5 ms `CROWD_INST_BUDGET_MS` it is meant to defend, or if clustering the
  crowd mesh changes any pixel with the HZB off (that would mean a bookkeeping
  bug, not a win).

### S8. Variable rate shading (BLOCKED, do not plan around it)

- **What**: shading a 2×2 or 4×4 block once (or at a lower rate) where the image
  can afford it.
- **Evidence**: the wgpu 30.0.1 `Features` list
  (https://docs.rs/wgpu/30.0.1/wgpu/struct.Features.html, **measured**) has no
  shading-rate, VRS or rate-combiner entry of any kind — I read the whole
  constant list: `CONSERVATIVE_RASTERIZATION`, `EXPERIMENTAL_MESH_SHADER`,
  `MULTI_DRAW_INDIRECT_COUNT`, `SHADER_EARLY_DEPTH_TEST`, and no VRS. Confirmed
  against the pinned source in this PC's registry
  (`wgpu-types-30.0.1`, `src/features.rs`): a case-insensitive grep for `vrs`
  and `shading_rate` over the whole crate returns **nothing**, while
  `MULTI_DRAW_INDIRECT_COUNT = 1 << 15` sits at line 887. WebGPU has no VRS in
  the spec either.
- **Licence**: n/a.
- **Verdict**: **reject now**. Re-open signal: a `SHADING_RATE`-class feature
  appears in wgpu's `Features`, or WebGPU ships one. Cost when it lands: M
  (a per-draw rate + a rate image); CPU none, GPU a real win on the 3050, which
  is exactly our potato target.
- **Do not** fake it with per-pass resolution scaling — we already do that in
  `crates/render/src/dynres_gov.rs` and `aa_scale.rs`; that is a different knob.

### S9. DLSS / FSR / XeSS in 2026: which are usable from wgpu at all

Short answer: **none of the vendor upscalers are usable from wgpu 30**, and the
reason is licence plus API, not quality.

| upscaler | state in 2026 (read live) | usable from wgpu? |
|---|---|---|
| DLSS 4.5 (Super Resolution, Ray Reconstruction, MFG) | https://developer.nvidia.com/rtx/dlss/get-started — "DLSS 4.5 introduces Dynamic MFG and a second generation transformer model"; integration is through Streamline | **no** |
| FSR 1 (EASU/RCAS) | MIT (https://github.com/GPUOpen-Effects/FidelityFX-FSR, repo licence read live: `mit`) | already ours, ported |
| FSR 2/3/4 SDK ("Redstone" 2.3.0) | https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK — `docs/license.md` read live: redistribution "in binary form only", "No reverse engineering, decompilation, or disassembly"; readme: "Vulkan is currently not supported in SDK" | **no** |
| XeSS 3 | https://github.com/intel/xess — licence metadata `NOASSERTION`; press: SDK published as Windows binaries, not open source (https://www.phoronix.com/news/Intel-XeSS-3-SDK-GitHub) | **no** |

Why DLSS is not just "hard": NGX is called *between* commands on a D3D12 or
Vulkan command list and its DLLs ship as redistributables. wgpu 30 exposes no
NGX hook and no way to inject an external command into its encoder
(**measured**: `wgpu-types-30.0.1` `src/features.rs` has no `dlss`/`ngx` entry,
and the whole crate's source greps clean for both).
On top of that the RTX SDK licence supplement — read live at
https://github.com/NVIDIA-RTX/RTXGI/blob/main/License.md — says at §1 that
"the DLSS SDK and NGX SDK are licensed for you to develop applications only for
their use in systems with NVIDIA GPUs", and at §4 that a commercial release
must be notified to NVIDIA first. So even a working integration would be
NVIDIA-only and paperwork-bound.

**What to do instead**: keep our own temporal upscaler
(`crates/render/src/post/upscale.rs`: engine-owned TAAU + FSR1-shaped EASU/RCAS,
with gates `FSR_MTF50_LIFT_MIN` 0.10 and `FSR_SHIMMER_GATE` 0.06 — **measured**
in the file) and spend the effort on S7 and S10, which are ours to ship on every
GPU including the potato.

## 3. Shading and material

### S10. Neural texture compression (NTC): the shape is the steal, not the SDK (M)

- **What**: compress all of a material's channels (albedo, normal, roughness,
  metalness, AO) *together* into a latent tensor plus a tiny MLP decoder; the
  decoder runs on load (transcode to BCn) or per sample.
- **Source**: https://github.com/NVIDIA-RTX/RTXNTC (README read live). Numbers
  it states (**donor claim**), for a 2k×2k bundle: raw 32.00 MB, BCn 12.00 MB,
  NTC-on-load 2.50 MB disk/traffic, NTC-on-sample 2.50 MB VRAM; and "many
  real-world texture bundles ... comparable to BCn (a PSNR of 40 to 50 dB) with a
  latent shape requiring about 5 bits/texel". Cooperative Vector gives "a 2-4x
  improvement in inference throughput" on Ada/Blackwell over the DP4a fallback
  (**donor claim**).
- **Licence**: `LICENSE.txt` read live — it is the *NVIDIA RTX SDKs LICENSE*
  (the same document as RTXGI's), not MIT: it forbids using the SDK "in any
  manner that would cause it to become subject to an open source software
  license" (§4e), and only lets you distribute SDK material "as incorporated in
  object code format" (§1c). Its compressor also wants Visual Studio 2022 plus
  the CUDA SDK, and compression needs "NVIDIA Turing (RTX 2000 series)" or newer
  (README, read live). So the SDK is **not** a build input for us, and its
  decoder shader source cannot be copied into our WGSL tree.
- **Third-party measured cost**, which is the number that decides it: on an RTX
  5070 at 1440p, inference-on-sample costs "roughly between 0.50-0.70 ms"
  versus BCn; ~1.20 ms at 4K; 0.60-0.70 ms at 1080p on a 5060
  (https://www.tomshardware.com/pc-components/gpus/benchmarking-nvidias-rtx-neural-texture-compression-tech-that-can-reduce-vram-usage-by-over-80-percent).
- **Why it fits, narrowly**: our whole texture budget is small — five CC0 1K
  arena maps, BC1/BC5/BC7 through `crates/assets/src/bc.rs` and
  `ktx2_cook.rs`. NTC's real win is *VRAM and disk for large bundles*; our
  arena's problem is not VRAM, it is that textures make the first-load bundle
  big on the web leg (G10's 2,040,000-brotli-byte budget). **NTC-on-load is
  exactly a download-size trick with zero runtime cost**, which is the only
  version of NTC a 3050/potato engine should want.
- **Cost**: M for the honest version (our own 2-layer MLP + latent grid trained
  per bundle at cook time, decoded in a WGSL compute pass at load, transcoded to
  BC7 by the existing encoder). GPU: none at runtime if we transcode on load.
  Author time: ~2 weeks, most of it a trainer, which `crates/distill` already
  has a shape for (`crates/distill/src/mlp.rs`).
- **Replaces**: nothing yet. It would feed `crates/assets/src/texture_cook.rs`
  as an extra source path, and `crates/assets/src/bundle.rs` for the byte win.
- **Prove or kill**: **first measure the denominator** — print the BC7+BC5 byte
  share of the arena bundle from the cook receipt. **Kill** if textures are under
  ~15% of the shipped bundle: then no texture codec can move the web budget and
  NTC is not our problem. If it is over that, the bounded experiment is one
  material bundle (5 maps) through a 2-layer MLP at ≤5 bits/texel, asserting
  PSNR ≥ 40 dB against the raw source and a smaller KTX2 than BC7.

### S11. Neural materials and "MSI" (unverified, reject for now)

- **What the brief asks about**: neural BRDFs / "multi-sample inference" (MSI).
- **What I could verify**: **nothing**. No NVIDIA page I could reach on
  2026-10-23 uses the term "MSI" or "multi-sample inference". The nearest real
  things are the NTC decoder (S10) and the RTX Neural Shaders material, whose
  hardware path (Cooperative Vector) is what NTC already uses.
- **Verdict**: reject this quarter on two independent grounds: (a) I cannot cite
  it, so it cannot earn a row; (b) a neural BRDF needs coop-vector-class
  throughput, and wgpu 30 has only `EXPERIMENTAL_COOPERATIVE_MATRIX`
  (**measured** in the `Features` list) — experimental, so it cannot be a gate.
- **Re-open signal**: a primary NVIDIA paper or SDK page naming MSI, plus a
  non-experimental coop-vector feature in wgpu.

### S12. Radiance cascades for dynamic GI (L, hold)

- **What**: a cascade of probe grids at increasing spacing, each storing radiance
  intervals, merged coarse-to-fine; a cheap alternative to tracing rays per
  pixel for multi-bounce GI.
- **Source**: the author's paper repo, https://github.com/Raikiri/RadianceCascadesPaper
  (repo licence read live: MIT; PDF in `out_latexmk2/RadianceCascades.pdf`). Its
  README states plainly: "I never sent it to any journal ... it uses the jcgt
  template, but was never published there."
- **Licence**: MIT — clean, and it is text plus maths, so we reimplement.
- **Why it might fit**: our lit frame is IBL + sun shadow + GTAO + bloom with
  probes in `crates/render/src/gi.rs` / `gi_probe.rs` / `baked_gi.rs`; there is
  no dynamic bounce, so torch-lit interiors are flat.
- **Why it does not fit yet**: the published results are overwhelmingly 2D and
  the 3D case is the hard one; it is unpublished, so there is no reference
  implementation to check against; and it is a per-frame GPU cost on a 3050 that
  already needs a 1080p win.
- **Cost**: L (author weeks); GPU: real and unmeasured.
- **Prove or kill**: a bounded experiment only — one arena interior frame, 6
  cascades at 1080p, measure pass ms and compare bounce colour against a baked
  reference (`baked_gi.rs`) at a fixed exposure. **Kill** if the pass costs more
  than the GTAO pass it would supplement, or if the 3D result needs more than one
  hand-tuned parameter per scene.

### S13. Surfel GI (EA SEED) as the alternative (hold, L)

- **Source**: *Global Illumination Based on Surfels*, Halen, Brinck, Hayward,
  Bei — SIGGRAPH 2021 Advances,
  https://advances.realtimerendering.com/s2021/index.html (syllabus read live,
  Part II 10:45). The same course also carries Epic's *Radiance Caching for
  Real-time Global Illumination* (10:10), which is the other credible route.
- **Licence**: talks, ideas only.
- **Why it does not fit yet**: surfels need a GPU scene representation and
  per-frame surfel spawning/merging — i.e. exactly the compute infrastructure
  S7 is building. It is downstream of S7, not parallel to it.
- **Cost**: L. **Verdict**: do S7 first; surfel GI is the natural follow-on once
  a GPU-side cluster list exists.

## 4. Measurable wins for a small studio

### S14. BC7 endpoint search beyond mode 6 (S, take)

- **What**: pick, per 4×4 block, among BC7 mode 6 (one subset, alpha) and one or
  more of modes 1/5/7 (2-3 subsets, no alpha or punch-through), choosing by
  squared error. Bytes are identical — every BC7 block is 16 bytes
  (**measured**: `BC7_BLOCK_BYTES` 16 in `crates/assets/src/bc.rs:26`).
- **Source**: the mode tables are in the Khronos Data Format spec
  (BC7 mode 0-7 layouts); the encoder shape we already cite is
  `PORT(hasenbanck/block_compression@f81ea755...)` in
  `crates/assets/src/texture_cook.rs:6-7`.
- **Licence**: our own code; the spec is a spec.
- **Why it fits**: `encode_bc7_block(texels, _slot)` writes mode 6 for *both*
  `BaseColor` and `MetallicRoughness` and ignores the slot
  (**measured**, `crates/assets/src/bc.rs:608`), while normals go to BC5
  (`pick_format`, `texture_cook.rs:35-42`). Mode 6 is the alpha-capable
  generalist; it is the wrong choice for opaque albedo with a strong gradient.
- **Cost**: S. CPU: 2-4× the encode time of one mode, at cook time only (cook is
  already offline; `blockcomp_cook.rs` proves cook-twice equality). GPU: none.
  Author: 2-3 days including the mode search and the tie-break rule.
- **Replaces**: the fixed mode-6 write; nothing else.
- **Prove or kill**: encode the five CC0 1K maps with mode 6 only vs
  mode-search; report PSNR-RGB and max channel error per map, encode ms, and
  that the cook-twice digest is still identical twice. **Kill** if PSNR moves
  less than 0.5 dB on all five, or if the cook exceeds 4× today's time and the
  bytes are unchanged (a look win we cannot see is not a win).

### S15. What we already took, so we do not re-buy it

- **Index codec** (edge-FIFO) is landed: `crates/assets/src/index_codec.rs`,
  LANDED 819ad5754 in `docs/research/LADDER.md`.
- **Vertex codec + quantised stream** is already a CARD on the ladder
  (`docs/research/LADDER.md`, the meshoptimizer `vertexcodec.cpp` line) — do not
  open a second row for it here.
- **Meshlets**: `docs/research/LADDER.md` already carries "VG12 proposed
  (meshlet virtual geometry build and culling)". VG12 is **free on the board**
  (checked by id) — S7 should board *that* id rather than mint a new one.
- **Sparse neural nets**: nothing to take. Sparse tensor acceleration is not
  reachable from wgpu (no sparse/structure feature; only
  `EXPERIMENTAL_COOPERATIVE_MATRIX`), and our nets are in `crates/nn` at
  ≤4,194,304 i8 params — a sparsity win there would be an integer-loop change,
  not an NVIDIA one.

## 5. What NOT to do (the traps)

1. **Never vendor an RTX SDK.** The one licence document covers DLSS, NGX,
   RTXGI, RTXDI, NRV/NRD, RTX Mega Geometry (https://github.com/NVIDIA-RTX/RTXMG,
   `NOASSERTION`) and NTC. Read live at
   https://github.com/NVIDIA-RTX/RTXGI/blob/main/License.md : §4e forbids using
   the SDK "in any manner that would cause it to become subject to an open source
   software license"; §8.1 reserves all rights beyond the grant. RTXGI is *not*
   MIT — the repo's `NOASSERTION` is the tell, and the file is titled "NVIDIA RTX
   SDKs LICENSE". Ideas from the papers are fine; the shader sources are not.
2. **Never vendor the FSR 2/3/4 SDK.** `docs/license.md` in
   https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK allows
   redistribution "in binary form only" and forbids reverse engineering. FSR 1
   (https://github.com/GPUOpen-Effects/FidelityFX-FSR) is MIT and is what we
   already ported the shape of. Do not mix the two up in a citation.
3. **Do not plan for CUDA-only steps.** The NTC compressor (Turing+, CUDA SDK,
   VS2022) and PhysX GPU bodies (CUDA, SM6.0) both take our cook or tick off the
   wasm/CI path. A cook step that only runs on one PC is a gate we cannot prove.
4. **Do not plan for DX12/Vulkan-only SDKs.** The FidelityFX SDK readme states
   "Vulkan is currently not supported in SDK"; NTC's DX12 path needs a *preview*
   Agility SDK and its README says of DX12 LinAlg: "DO NOT SHIP ANY PRODUCTS
   USING IT." Anything with that sentence is a research toy.
5. **Do not build on experimental wgpu features.** `EXPERIMENTAL_MESH_SHADER`,
   `EXPERIMENTAL_RAY_QUERY` and `EXPERIMENTAL_COOPERATIVE_MATRIX` exist in wgpu
   30 (measured) and none of them is a gate we can hold on the Intel UHD potato
   run. S7 stays on compute + `multi_draw_indirect_count`.
6. **Do not take vendor lock into a golden-hash engine.** DLSS is NVIDIA-only by
   licence (§1 supplement), and a frame path that exists on one vendor breaks G9
   (potato, Intel UHD) and G10 (WebGPU in a browser tab).
7. **Do not copy a GPL or paid-SDK idea as code.** Nothing here is GPL, but the
   Unreal DLSS plugin, Omniverse Kit and the NTC SDK are all "read the docs,
   write your own" material. Cite the page; write the code.

## Ranked

| # | steal | gain (target) | cost | first experiment |
|---|---|---|---|---|
| 1 | **S7** GPU-driven cluster cull: cones + HZB + one indirect draw, over the meshlets we already cook | the 50,000-seat crowd draws from one pass; the CPU per-draw sphere loop leaves `submit_cull.rs`; gives `meshlet.rs` its first consumer | L (≈2 wk, GPU +0.5 ms budget) | 50k seats, 300 frames: clusters tested/kept/drawn, cull ms p50/p95, pixels identical to the CPU cull |
| 2 | **S14** BC7 mode search instead of mode-6-only | look (G4) at **zero** extra bytes; today the slot argument is ignored | S (2-3 d, cook CPU 2-4×) | 5 CC0 1K maps, mode 6 vs search: PSNR, max error, ms, cook-twice digest identical |
| 3 | **S6** Nsight capture as the G2/G3 oracle | turns "1080p unmeasured" and "pass floors pending" into a per-pass ms table | S (1 afternoon, no code) | one 1920×1080 8-fighter capture; per-pass ms within 10% of our ledger |
| 4 | **S1** NVTT as the BC7 quality oracle | tells us if S14 is worth anything, before we write it | S (half a day) | run NVTT over the same 5 maps; our mode-6 within 0.3 dB → skip S14 |
| 5 | **S10** NTC-on-load shape (our own MLP, decode to BC7 at load) | web/first-load bytes only; runtime cost zero | M (≈2 wk) | measure the texture share of the arena bundle first; under ~15% → kill |
| 6 | **S4** Warp as a batch numerics sandbox | a vectorised env we lack, offline only | M (≈1 wk) | 8-body drop in Warp vs `crates/physics`: same first-contact tick? |
| 7 | **S12** radiance cascades | dynamic bounce where we have none | L | one interior frame, 6 cascades, pass ms vs GTAO, colour vs baked |
| — | **S8** VRS | would be a real 3050 win | blocked | n/a: no shading-rate feature in wgpu 30 |
| — | **S9** DLSS/FSR2+/XeSS | a big upscale win | blocked | n/a: licence + no wgpu hook |
| — | **S2/S3/S5/S11** USD, PhysX, Kit, MSI | — | — | rejected with a re-open signal each |

## Proposed row ids (not boarded here)

1. **VG12** — *GPU cluster cull: cones plus HZB, one indirect draw for the crowd.*
   Boards the id `docs/research/LADDER.md` already proposes ("meshlet virtual
   geometry build and culling"); the id is free on the board. Accept: 50,000
   seats, 300 frames, cull p95 inside `CROWD_INST_BUDGET_MS` 0.5, pixels
   identical to the CPU sphere cull, golden hash unmoved.
2. **TX7** — *BC7 mode search past mode 6, same bytes, better pixels.* Accept:
   +0.5 dB PSNR-RGB on at least three of five CC0 1K maps, cook-twice digest
   identical, cook time under 4× today.
3. **GX15** — *one Nsight capture as the G2/G3 timing oracle, filed beside
   pass-truth.* Accept: a per-pass GPU ms table for one 1920×1080 8-fighter
   frame, total within 10% of the in-engine frame number, checked into
   `target/reports/` with the adapter and the driver version.

## Still unverified

- **Nothing in this field was executed.** No vendor SDK was downloaded, no
  capture taken, no encoder compared. Every gain is a target.
- **Nsight on a wgpu app is an assumption.** The page lists D3D12/Vulkan/OpenGL
  capture; I did not verify that our wgpu 30 window (DX12 backend, or Vulkan if
  the env var says so) appears in a capture, nor that per-pass times land within
  10% of `present/gpu_present/timing.rs`.
- **"MSI" has no primary source I could reach.** If the brief meant NTC's
  per-texel inference or the NTC paper's multi-sample decode, that is S10; if it
  meant a separate NVIDIA technique, this card does not cover it.
- **The 2018 attribution is wrong.** The cluster/cone-culling talk is 2021
  (Karis, Stubbe, Wihlidal); the 2018 Advances syllabus has no Karis entry. I
  corrected it in S7 rather than repeating it.
- **The Omniverse Kit licence was not read as text.** Its docs page renders by
  script, so I could not quote a clause; "proprietary, not on the allow-list" is
  an inference from the product packaging, and the owner should confirm before
  anyone spends a day on it.
- **NTC's compression ratio is a donor claim on their own sample materials**
  (2k bundle, `MetalPlates013`). Tom's Hardware supplies the only
  third-party *cost* numbers, and they are on desktop RTX 50/40 parts — nothing
  on an Intel UHD or a 6 GB 3050, which are our actual targets.
- **No GPU timing in this card.** `CROWD_INST_BUDGET_MS` 0.5 and the 0.5-0.7 ms
  NTC figures are not comparable: one is our budget for a cull pass, the other
  is someone else's measured decode cost on different hardware.
