# Outside the box: donors from outside game engines (2026-10-21)

Hunter 0 (the orchestrator). Rule for this file: every repo here is OUTSIDE the
engine-donor set the earlier waves mined (Bevy, Godot, Flax, o3de, Filament,
Falcor, WickedEngine, Daedalus, renderling...). Every row was verified live on
2026-10-21 with `gh api repos/<owner>/<repo>` (stars, license, last push) and
the donor file path was read, not guessed.

Verified with:

```
node docs/research/hunt-2026-10-21/ghf.mjs <owner/repo> meta
node docs/research/hunt-2026-10-21/ghf.mjs <owner/repo> tree "<regex>"
node docs/research/hunt-2026-10-21/ghf.mjs <owner/repo> file <path>
```

## 1. The shortlist (read in this order)

| # | donor | stars | lic | pushed | donor file | what we take | our home | gap / row | size | risk |
|---|---|---|---|---|---|---|---|---|---|---|
| X1 | `Beledarian/wgpu-llm` | 13 | Apache-2.0 | 2026-08-30 | `crates/core/shaders/gemm.wgsl`, `softmax.wgsl`, `rmsnorm.wgsl`, `rope.wgsl`, `crates/core/src/kv_cache.rs`, `crates/core/src/sampling.rs` | a **whole LLM forward pass in WGSL compute**, so the model runs on OUR wgpu device, in OUR process, with no llama.cpp and no second Vulkan context | `crates/provider` (local provider), `crates/distill` | FOCUS Now 4 "local inference sharing the 6 GB GPU"; G7 | M | decode-only, CPU-side sampling, no batching; 1.1B model on a 3050 6 GB leaves too little for the renderer at the same time -> time-slice the queue, not the frame |
| X2 | `KeKsBoTer/web-splat` | 307 | Apache-2.0 | 2026-10-13 | `src/shaders/radix_sort.wgsl` (23.8K), `src/shaders/preprocess.wgsl`, `src/shaders/gaussian.wgsl`, `src/gpu_rs.rs` | a **WGSL GPU radix sort** (histogram -> prefix -> scatter, 8-bit radices, workgroup memory, no subgroup ops) and a WebGPU Gaussian-splat renderer in Rust | `crates/render` (sort for particles + instancing + the 20k crowd), `crates/web` | B7 "live cull, sort and instancing"; G3 | S for the sort, M for the splat path | the sort shader is liftable near-verbatim; the splat path wants an .npz/ply loader and is a second renderer, not a drop-in |
| X3 | `Open-fab-ai/openfab` | 6 | Apache-2.0 | 2026-10-02 | `docs/generation-predicate-v0.1.md`, `src/core/provenance.rs` (31K), `src/core/identity.rs`, `src/ops.rs` | the **generation predicate**: who (human or agent), which model, the prompt fingerprint, the acceptance contract, and the sign-offs, bound to the artifact digests (an "AI Bill of Materials") | `crates/endpoint` (receipts), `crates/ai/src/provenance.rs` | D17 activity feed, D19 verifier output, E22 ship check | S-M | crypto (Sigstore, in-toto) is heavy; steal the *record shape and the verification rules*, not the whole stack; our receipts stay unsigned text |
| X4 | `Beledarian/mcp-local-memory` | 60 | MIT | 2026-09-14 | `README.md`, `docs/extended-guide.md` (30K) | an MCP memory server: hybrid recall (keyword + vector), time-decayed importance, a lifecycle (reinforce, mark outdated, restore, export, forget) | `crates/endpoint` (serve a project memory), `crates/ai` | D18 agent memory, D16 MCP resources | M | it is TypeScript and uses SQLite FTS5 + sqlite-vec: we reimplement the design over our own store, we do not vendor it |
| X5 | `AWMC/geodata` | 68 | ODbL-1.0 | 2024-04-22 | `Cultural-Data/roads/roads.geojson` (5.1M), `Cultural-Data/aqueducts/aqueducts.geojson`, `Cultural-Data/walls/walls.geojson`, `Cultural-Data/urban_areas/urban_areas.geojson`, `Cultural-Data/political_shading/roman_empire_ce_117_extent/roman_empire_ce_117_extent.geojson` | **real Roman geography** (roads, aqueducts, walls, urban areas, and the empire under Hadrian, AD 117) as GeoJSON | `crates/rome` + `content/rome/*.toml` | ROM2 (rome tables), LEG6 (dig layer), ROM5 (`rome.road`) | S-M | ODbL requires share-alike for the database: we ship a derived table with the ODbL notice and the source in `ref`, not a copy of the dataset; 5 MB of GeoJSON is not a game asset, it is a build input |
| X6 | `Diasito/urbesetorbis` | 0 | BSD-2 | 2026-06-30 | `public/downloads/301023_AD130.csv` (7,891 settlements, coords, province, chronology), `public/data/cultural/roads10.geojson` (Itiner-e roads) | a **digital gazetteer of the empire** under Hadrian, with cross-references to Pleiades, DARE, Vici, Wikidata | `crates/rome` (settlements table), `content/rome/places.toml` | ROM2, LEG6, chapter maps | S | 1.6 MB CSV; the cross-reference fields point at sources we would cite in `ref` |
| X7 | `siriusbontea/roman-empire` | 15 | BSD-3 | 2022-05-30 | `data/` (TopoJSON of Roman roads, fortifications, provinces) | a clean BSD-3 TopoJSON of roads and forts, with the sources named (AWMC, Mercator-E, DARE) | `content/rome/` build input | ROM5 `rome.road` | S | last push 2022: check the data against AWMC before use |
| X8 | `gpuweb/cts` + `infosia/webgpu-native-cts` | 180 + 0 | BSD-3 / n/a | 2026-10-20 / 2026-09-26 | the **WebGPU conformance suite**, and a runner that runs it against a **native** wgpu build. NOTE: our strict flag already exists (`crates/render/src/backend.rs`, `E2040_STRICT_WEBGPU=1` in CI), so the steal is the *suite*, not the flag | `tools/`, CI | A4 (the measurement half of the strict leg) | S-M | the CTS is big; we want one leg: a chosen slice of the suite against the strict instance, with per-case isolation and an expectations file; the native runner is 0 stars and unproven, read it before trusting it |
| X9 | `nff747/meshlet-culling-wgsl` | 2 | Apache-2.0 | 2026-10-19 | `src/shaders/meshlet_cull.wgsl.ts`, `src/math/cone_cull.ts`, `src/math/frustum.ts`, `src/runtime/culling_pipeline.ts`, `tests/` | **GPU-driven meshlet culling in WGSL**: cluster cull, normal-cone filtering, indirect multi-draw, with tests | `crates/render` (live cull + instancing), `crates/assets/src/meshlet.rs` | B7, VG12 | S | TypeScript, but the WGSL is the payload; the tests are the idea we steal (cull-efficiency assertions we can print) |
| X10 | `whlee503/ue5-gpu-driven-culling` | 3 | n/a | 2026-07-30 | the UE 5.7 compute frustum + HZB occlusion culling with indirect draw | the **HZB occlusion** half (we have no Hi-Z) | `crates/render` | B7, VG12 | M | UE-specific, C++ compute; read it for the algorithm and the benchmark shape, not the code |
| X11 | `DeharengOlivier/gpu-cloth-simulation` | 1 | MIT | 2026-08-28 | `src/compute.wgsl` (16.1K), `src/simulation.rs`, `src/timestep.rs`, `src/headless.rs` | **cloth in wgpu compute** with a headless mode and a throughput example | `crates/physfx` (capes and banners), `crates/render` | C10 (soft bodies for capes and banners) | S-M | the float physfx layer only, never hashed |
| X12 | `PrethamMuthappa/aero-sim` | 2 | n/a | 2026-09-15 | the D2Q9 lattice-Boltzmann wind tunnel in a compute shader | a **GPU wind field** a banner or a cape can read | `crates/physfx`, `crates/weather` | C10, wind in the arena | S | needs a real wind source (Vegetius gives camp siting, not wind) |
| X13 | `StarsX/ComputeRaster` | 124 | n/a | 2025-03-11 | the compute-shader rasterizer (IA + vertex + bin + raster) | a **reference rasterizer in compute**: our own picture can be checked against a second, simpler renderer | `crates/proof`, `tools/` | B8 (machine eyes), G1 | M | no license stated, so read it as an idea only; a full rasterizer is a big job |
| X14 | `Anandb71/minecraft-2` | 3 | MIT | 2026-10-04 | the sparse-voxel compute ray marcher | the **"mesh nothing"** idea: the arena drawn from data in a compute pass, no vertex buffers at all | `crates/render` | B7, crowd | M | a whole second renderer; the useful part is the sparse draw indirection |
| X15 | `rust-secure-code/cargo-repro` | 58 | Apache-2.0 | 2021-03-11 | the cargo reproducible-build workflow | **byte-for-byte reproducible Rust builds** and the check that proves it | `tools/`, CI | E22 (Windows ship check from CI) | S | our Windows build has to become reproducible first; the tool is old but the idea is exact |
| X16 | `omnibor/omnibor-rs` | 30 | Apache-2.0 | 2025-07-29 | the OmniBOR tree and identifier code | **fine-grained build dependency tracking**: name the file that changed, not the whole tree | `crates/endpoint` (scene.diff, D17), `tools/` | D17, E22 | S-M | a whole protocol; steal the tree-diff idea, not the spec |
| X17 | `MatteoMazzamurro/roman-road-networks` | 2 | CC-BY-SA-4.0 | 2026-05-29 | the analysis code and the network data | Roman road network **analysis** (not just geometry): centrality, gaps a road must fill | `crates/rome` | ROM5, chapter maps | S | share-alike on the data |
| X18 | `sdam-au/epigraphic_roads` | 1 | CC-BY-SA-4.0 | 2022-02-16 | the code that **detects roads from inscriptions** | the idea: a road is real when an inscription says so (the codex pattern) | `crates/rome` (codex), `content/rome` | ROM1, CDX rows | S | share-alike; the method is the steal |
| X19 | `MenokoOG/aquila-legions` | 1 | MIT | 2026-10-16 | the hex-tactics Roman legion game | unit and campaign **data** (three campaigns, seventeen battles) | `content/rome/` | ROM2, LEG rows | S | a hex game, not our line; data only |
| X20 | `GameTechDev/TAA` | 133 | n/a | 2023-01-03 | the Intel TAA shaders | the **TAA resolve** reference (Intel's implementation) | `crates/render` | B7, B9, LF5 | S | an HLSL reference to translate to WGSL; XeGTAO from the same group is already in our files |
| X21 | `vga-group/taa-kdop-optimizer` | 4 | MIT-0 | 2024-10-20 | the k-DOP ghosting mitigation tools | **k-DOP clipping**: the half-vector trick that kills TAA ghosting; plus the tool that finds the axes | `crates/render` | B9 (temporal artifacts), LF5 | S | a research tool, but the axes can be precomputed and shipped |
| X22 | `santhreal/vyre` | 6 | Apache-2.0 | 2026-09-21 | the WGSL branchy-compute work | **branchy work in a compute shader** (wave intrinsics, ballot, match): the thing a TAA or a cull pass wants | `crates/render` | B7 | S | a research project with 5,215 blobs; read the README and take the method |
| X23 | `siriusbontea/roman-empire` (data) + Edinburgh DataShare "GB Roman Roads" | 15 / n/a | BSD-3 / PDDL-1.0 | 2022 / 2017 | the shapefile and the GeoJSON | **Roman roads in Britain in the public domain** (PDDL): a clean licence for one region | `content/rome/road.toml` | ROM5 | S | a shapefile needs a converter; the PDDL licence is the cleanest we found |
| X24 | The Latin Library (thelatinlibrary.com/vegetius.html) + Perseus | n/a | public domain / CC-BY-SA-3.0 | n/a | Vegetius, *De re militari* 4 books | the **primary sources** for camp, drill, marching order and formation | `content/rome/*.toml` (`ref` field) | ROM1, CDX rows, LEG3, LEG4 | S | the text is the source we cite; our tables refuse a number with no `ref`, so this is the proof the pipeline needs |

## 2. Wild ideas from this file

1. **The model runs on the render device.** X1 gives a whole transformer in WGSL.
   Our door, our provider, our GPU. The in-game model is not a sidecar process:
   it is a compute pass on the same wgpu queue, time-sliced between frames, with
   the KV cache in a buffer we can spill. The AI door stops being a *remote* door.
2. **An AI Bill of Materials for every door edit.** X3. Every edit the door
   applies writes: who (human or agent id), which model, the prompt hash, the
   tape hash before and after, and which acceptance check passed. The engine can
   then answer "who changed the fight rule and what proved it".
3. **The arena as a splat.** X2. A phone scan of the arena, splatted in WebGPU,
   gives a photoreal reference in the browser with no mesh pipeline, and the same
   splat is a *reference image* for the look gate. Two birds: the look bar gets
   a photograph to compare against.
4. **Sort everything on the GPU.** X2 + X9. One radix sort, reused for particle
   depth, instance order and the 20k crowd's bone order. The crowd stops being a
   CPU problem.
5. **The real map.** X5 + X6 + X23. The chapter maps are the actual Roman
   geography: real roads, real walls, real aqueducts, the empire extent under
   Hadrian (our era). `rome.road` draws a road that existed, not a noise line.
6. **The codex pipeline.** X24 + X18. A script pulls a passage from a
   public-domain text, extracts the number, and writes `content/rome/*.toml`
   with a `ref`. Our tables already refuse an entry with no `ref`, so the
   pipeline is proof-carrying by construction: a number without a citation does
   not compile.
7. **Proof the build, then sign it.** X15 + X16 + X3. The Windows ship build is
   reproducible, its file tree is named by content hash, and the receipt is
   attached to the run. G1 becomes "never broken" with evidence, not a hope.
8. **Strict WebGPU as a CI leg.** X8. One leg that creates the instance with the
   strict flag and runs a slice of the official suite, so the browser target
   cannot drift from the native one.

## 3. Rejected (so nobody mines them twice)

| donor | why it loses |
|---|---|
| `paiml/aprender` (136 stars, MIT) | a pure-Rust ML and statistics monorepo; `crates/aprender-compute` is an ML compute backend, not a 3D renderer. Big, alive, and useless to us |
| `Nasjoe/Formations` (7 stars, CC0) | a false positive: a French web-course repo about Docker and CSS. "Formations" is a word, not Roman formations |
| `Lulzx/zeno` | GPU batched physics, but Apple Silicon only. We are on Windows and Vulkan |
| `rust-secure-code/cargo-repro` (2021) | accepted as an idea only (X15): the tool is stale, the idea is exact |
| `Open-fab-ai/openfab` full stack | the Sigstore and in-toto crypto is too much for a game door. Take the record shape, not the chain |
| `alertform/ue5-anime-renderer` | cel shading and outlines for an anime look. Our look bar is a Roman legionary, not anime |
| `infosia/webgpu-native-cts` alone | 0 stars and unproven; the suite (X8) is the real donor, the native runner is a hint |
