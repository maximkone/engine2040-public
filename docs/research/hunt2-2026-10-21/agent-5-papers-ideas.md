# Hunter 5: papers and fresh ideas

Hunt wave 2, 2026-10-21. Scope: arXiv (2024-2026) work and the GitHub repos that
implement it, turned into measurable engine work. Every row below names the exact
file in **our** tree it lands in (path + line, read in this session) and the open
board row it serves. Every number is measured: from a donor README, a donor file,
a paper abstract, or a repo listing read through `gh api` on 2026-10-21.

Rules I held: no donor that wave 1 already mined (`docs/research/hunt-2026-10-21/donors-seen.txt`
and the five wave-1 agent files were read first; every repo named here was checked
against that list and against the three wave-2 agent files already in this folder).
No cargo, no build, no commit. One file written.

Reading key for the "our file:line" column: `crates/…` paths are in this repo and
the line numbers are the ones I opened. Donor paths are named as `owner/repo:path`.

---

## Ranked papers and their code (1..12)

| rank | paper (title + arXiv id/url + year) | code repo (owner/repo, license, stars) | what to steal | our file:line | our row id | measurable accept |
|---|---|---|---|---|---|---|
| 1 | **Filtering After Shading With Stochastic Texture Filtering**<br>`arxiv.org/abs/2407.06107` (2024) | `NVIDIA-RTX/RTXTF`<br>license NOASSERTION (NVIDIA RTX SDK), **28** stars, pushed 2026-09-05<br>README links `research.nvidia.com/publication/2024-05_filtering-after-shading-stochastic-texture-filtering` | Shade first, filter after. RTXTF "effectively loads a texel per frame based on the target filter and the probability to select a texel in that filtering footprint. Over many frames, the texels are interpolated … with a TAA upscaler." Filter types: Linear, Cubic, Gaussian for Texture2D/2DArray/Cube/3D. Our resolve is the reconstructor: it currently clamps to a 3x3 min/max box, which is the cheapest possible "reconstruction" and throws away the filter identity. Make the filter an explicit stochastic per-texel choice and let the TAA history rebuild it. | `crates/render/src/shaders/post/taa_resolve.wgsl:7` (28-line `taa_resolve`, plain `min`/`max` box clamp at :20-:21, `clamp(hist, lo, hi)` at :24)<br>`crates/render/src/post/taa.rs:201` (`taa_resolve_jittered`) | **LF5c** (needs LF5b), **HX11** | LF5c keeps its own Accept (sand-edge luminance flicker drops ≥ 50 % vs off, running fighter's ghost < 3 px, validation errors 0) **and** gains one number: on a textured fixture the resolve output changes on ≥ 5 % of pixels versus the current box clamp (today it is byte-equal), and the CPU twin of the resolve still agrees with the GPU answer inside the tolerance the twin already pins. LF5b stays its own row (8 Halton(2,3) offsets inside (-0.5, 0.5) px, flag off leaves frames byte-equal). |
| 2 | **Where the decibels go: an error budget for temporal super-resolution**<br>research write-up, `github.com/RARS-oss/decibels/blob/main/docs/report.html` (2026; **no arXiv id yet**, the repo says "paper track in progress") — same family as row 1 (`arxiv.org/abs/2407.06107`, 2024) | `RARS-oss/decibels`<br>MIT, **2** stars, pushed 2026-09-11<br>30 numbered findings, each with a control | The method, not the filter: price every loss in dB against an exact reference instead of reporting one end-to-end number, and compute an **oracle bound** (rejection with access to the reference) that bounds what any history-rejection heuristic could ever achieve. Measured numbers I can reuse as targets: half-pixel jitter-convention bug **1.12 dB**; a relative depth-rejection threshold threw out **two thirds of valid history**, fixing the scale gave **+0.31 dB**; non-recursive resolve **+1.57 dB** on a static scene and **+0.6…+1.5 dB** over recursive TAA at every motion level; oracle rejection bound **+3.3 dB**, a colour test recovers **39 %** of it, an albedo-demodulated test **55 %**; per-material short memory **+2 dB** in mirror pixels; on Cycles every temporal method is **4–6 dB below Lanczos**, cause split soft shadow ≈ 60 % / bounce ≈ 40 %; a temporal-gradient test recovers **2.7 dB** while an FSR2-style luminance reset **collapses −3.4 dB** on aliased content; **PSNR under-values rejection** (adding the gradient test lowers PSNR by 0.3 dB but moves LPIPS **0.178 → 0.115**). | `crates/render/src/post/taa.rs:9` (`TaaParams`)<br>`crates/render/src/post/taa.rs:145` (`taa_resolve`)<br>`crates/render/src/post/taa.rs:201` (`taa_resolve_jittered`)<br>`crates/render/src/post_jitter_motion.rs:22` (`halton`), `:38` (`jitter_offset`), `:64` (`jitter_projection`) | **LF5c**, **LF5b**, **HX11** | LF5c's Accept gains a **dB budget**: the resolve prints one line per cause (jitter registration, depth-rejection scale, history length) with a measured dB each, and the sum is within **1 dB** of the end-to-end delta. A regression test fails when the half-pixel jitter convention is off (the donor measured 1.12 dB on that bug alone). The report prints **PSNR and LPIPS side by side**, because the donor's negative result is that PSNR alone draws the wrong conclusion. LF5b's Accept stays byte-equal-when-off and adds "the 8 offsets are the ones the budget used". |
| 3 | **Towards Practical Meshlet Compression**<br>`arxiv.org/abs/2404.06359` (2024) | `Scthe/nanite-webgpu`<br>MIT, **1124** stars, pushed 2026-05-09<br>(independent WebGPU Nanite, **not** the paper's code — GitHub code search for `2404.06359` returns only paper-digest repos, so the paper's own codec was never released) | A full GPU-driven virtual-geometry pipeline in WGSL. Per-**instance** *and* per-**meshlet** frustum + occlusion culling; meshlet LOD hierarchy built in-browser with meshoptimizer + METIS; a software-rasterizer path that packs depth `u16` + octahedron normals `2×u8` into 32 bits because WebGPU has no `atomic<u64>`; billboard impostors as 12 images around the UP axis blended with dithering; a scene of **1.7 b triangles** where "nearly 98 % of the triangles are software rasterized"; the Jinx scene is **640 m triangles**. Paper numbers to hold us to: **16:1** index-buffer compression, and 15.5 M triangles decompress + render in **0.59 ms** on an RX 7900 XTX. | `crates/render/src/shaders/cull.wgsl:64` (`hiz_hidden_wall` — the Hi-Z reject, currently one `behind && inside` test)<br>`crates/render/src/shaders/cull.wgsl:25` (`main`, sphere-vs-6-planes)<br>`crates/render/src/gpu_cull.rs:237` (`cull`), `:366` (`verify`)<br>`crates/render/src/meshlet.rs:39` (`Cluster`), `:47` (`cluster_cut`) | **HX7**, **E70**, **PW05** (owns the GS18b draw hook) | HX7 keeps its Accept (the cull report prints `culled=N visible=M` and the visible set matches the CPU oracle) and gains the **per-meshlet** half: the same report prints the cluster count the LOD cut chose, and a fixture whose cluster normals all point away from the camera is culled by the cone test while a mixed-normal mesh is not. E70 keeps its Accept (5 meshlet-cull vectors pass with **0 false negatives**) and gains the paper's ratio as a target line: the cooked meshlet index payload is ≤ 1/16 of the vertex payload on the fixture. |
| 4 | **Environment-aware Motion Matching**<br>`arxiv.org/abs/2510.22632` (2025) | `UPC-ViRVIG/Environment-aware-Motion-Matching`<br>MIT, **103** stars, pushed 2026-04-24<br>files read: `EnvironmentSearchBurst.cs` (14.7 K), `HeightSearchBurst.cs` (14.3 K), `EllipseFeatureExtractor.cs` (3.3 K), `HeightFeatureExtractor.cs` (1.7 K) | Features are shape + pose + trajectory, and the runtime search **penalises collision with a dynamic environment** while matching pose to trajectory in both directions (the paper's stated bug: "in crowd animation, body animation is frequently treated as a separate process from trajectory planning"). Our search already builds a fixed-shape query from pose+trajectory; the missing half is that the environment never enters the cost, so a fighter walks into a wall. | `crates/anim/src/motion_match.rs:131` (`make_feature` — the frozen feature vector)<br>`crates/anim/src/motion_match.rs:241` (`build_query` — the search query)<br>`crates/anim/src/motion_match.rs:379` (`search`) | **LM4**, **AM18** | LM4 keeps its Accept (PF1 gate fails on the seeded regression, 8-fighter tick in budget, a schema change refuses with a diff, golden unchanged) and gains: `build_query` carries an environment-penalty term, and a seeded fixture with a blocking obstacle makes `search` return a **different clip id** than the same fixture without the obstacle. The LM1 feature-schema freeze test still refuses a schema change. AM18 keeps its own budget (1 000 agents p99 ≤ 2 ms). |
| 5 | **OpenGameEval: Benchmarking Agentic Programming and Exploration in a Stateful Game Engine**<br>`arxiv.org/abs/2610.02563` (2026) | `Roblox/open-game-eval`<br>MIT, **73** stars<br>84 `Evals/*.lua` task files + `invoke_eval.py` (10.1 K) + `Places/*.rbxl` | **Separate observation tools from editing tools** so exploration is measurable on its own, then score each run with executable checks on the edited scene *and* in a simulated play session. Measured on **13** frontier models over **84** tasks × **16** attempts: the best model solves **51.7 %** on a single attempt and **39.4 %** five-of-five, and no model solves six tasks. Splitting tasks by work type spreads the top five by **5.0 pp** on script-authoring and **12.5 pp** on scene-change. The headline for us: a run that inspects every object a reference solution touches before acting passes **13.4 pp** more often on scene-only tasks and **9.8 pp** more on script-only tasks. | `crates/playtest/src/brief_eval.rs:34` (`Brief`), `:45` (`BRIEFS`, 20 authoring + 5 play briefs)<br>`crates/playtest/src/brief_eval.rs:537` (`run_report`)<br>`crates/endpoint/src/packs/` (the door's pack set — `play.rs`, `verify.rs`, `rules/verbs.rs`) | **PW24**, **HX29** | PW24 keeps its Accept (the door prints one line per task with its verdict and the pass rate, and the pass rate is the same twice for the same seed) and gains the **exploration split**: every task report separates "inspected" from "edited", so we can publish our own version of the 13.4 pp / 9.8 pp exploration gap on our briefs. HX29 keeps its Accept (a missing evidence source is a fail with the source named, the mutant of each criterion fails exactly that criterion, pass@k excludes the unscored cells, the verdict names the first failing predicate). |
| 6 | **lmgame-Bench: How Good are LLMs at Playing Games?**<br>`arxiv.org/abs/2505.15146` (2025) | `lmgame-org/GamingAgent`<br>MIT, **994** stars<br>code under `lmgame-bench/`; files read: `gamingagent/envs/gym_env_adapter.py`, `modules/memory_module.py`, `modules/perception_module.py`, `lmgame-bench/single_agent_runner.py` (48 K) | One **Gym-style API** over many games, with lightweight perception and memory scaffolds, designed to *stabilise prompt variance* and remove contamination. Measured across **13** leading models: the suite is hard but separates models well, and "every game probes a unique blend of capabilities often tested in isolation elsewhere". The transfer claim is the one we care about: **reinforcement learning on a single game transfers to unseen games and to external planning tasks**. Our door already has a gym pack; what is missing is that the same policy can walk to a second game without a re-tune. | `crates/endpoint/src/packs/gym.rs:42` (`PACK = "gym"`), `:44` (`TOOL_SIM_RESET`), `:46` (`TOOL_SIM_STEP`), `:373` (`pack`)<br>`crates/endpoint/src/packs/gym_observe.rs` | **GA7**, **PW24** | GA7 keeps its Accept (≥ 3 strategies per difficulty, PQ2 win-rate 40-60 %, no float in tick) and gains the transfer half: a policy trained on one seeded brief keeps its win rate band on a **second, unseen** brief without a re-tune (the donor's measured transfer). PW24's task suite gains a **gym-shaped** entry so an outside agent can `reset`/`step` our arena the same way. |
| 7 | **Orak: A Foundational Benchmark for Training and Evaluating LLM Agents on Diverse Video Games**<br>`arxiv.org/abs/2506.03610` (2025) | `krafton-ai/Orak`<br>MIT, **171** stars<br>files read: `src/mcp_game_servers/base_server.py` (4.5 K), `src/mcp_game_servers/gameio/io_env.py` (17.5 K), `src/mcp_game_servers/street_fighter/game/street_fighter_env.py` (19.9 K), 12 MCP game servers | **Plug-and-play over MCP**: one base server class, one env per game, so a new game is a module and not a fork. The evaluation frame is three tables, not one number: **game leaderboards**, **LLM battle arenas**, and **ablation studies** of input modality, agentic strategy and fine-tuning, plus an expert-trajectory fine-tuning set. The measured result that matters to us: **27 % fewer output tokens** for their tool-augmented planner, because it retrieves only the state a decision needs and issues one plan per domain instead of one giant prompt. | `crates/endpoint/src/packs/rules/verbs.rs:102` (`RulesSetArgs`), `:187` (`RulesTestArgs`), `:259` (`RulesExplainArgs`)<br>`crates/endpoint/src/packs/mod.rs`, `crates/endpoint/src/packs/pack` set in `crates/endpoint/src/packs/` | **PW24**, **GA7**, gate **G7** | PW24 keeps its Accept and gains an **ablation row**: the same brief runs with input modality on and off and both verdicts land in the report (the donor's ablation axis). Orak's per-game-module shape lands as one game server module behind our existing door verbs, and the token count of one run is printed so the 27 %-fewer-tokens claim is testable on our stack. Gate G7 stays: one command door for every model, local or remote, same results. |
| 8 | **Forget Superresolution, Sample Adaptively (when Path Tracing)**<br>`arxiv.org/abs/2602.08642` (2026, SIGGRAPH 2026) | `balintio/fssa`<br>Apache-2.0, **13** stars, pushed 2026-07-22<br>files read: `src/filters/gather_pyramid.py`, `src/tonemap.py`, `src/networks/jnds.py`, `src/loss/milo_loss.py`, `src/model/model_adaptive.py` | A **gather-based pyramidal denoising filter** for the sub-1-spp regime (K=5 pyramid; each level is a 5×5 gather with 29 learned taps, the coarsest 25, and the upsampled level contributes 4 more taps), plus a **tonemapping-aware** training pipeline with differentiable filmic operators and a perceptual loss, plus a learnable **albedo demodulation**. The point for a real-time engine: adaptive sampling beats uniform sampling because it exploits "variations in noise, reconstruction difficulty, and perceptual importance across the image" — and it preserves specular highlights and shadow boundaries that a uniform sampler smears. | `crates/render/src/post/upscale.rs:11` (`taa_upsample` — the frozen bilinear TAAU), `:120` (`fsr_easu_con`), `:65` (`FSR_SHARPEN_MIN`), `:71` (`FSR_SHIMMER_GATE`)<br>`crates/render/src/restir_di.rs:9` (`DI_TORCHES = 250`) | **RT6**, **RL34**, **PW12**, **HX11** | RT6 keeps its Accept (a moving-camera clip with **no boiling** at the RT-tier sample count, the ghosting test passes) and gains a number: with the 5×5 gather pyramid on top of the 250-torch ReSTIR DI frame, the temporal variance drops by a **measured factor** (printed) while the frame mean stays within **1 %**. RL34/PW12 keep their Accept (one hue-keeping tonemap + dither, torch hue shift under 2°) — the tonemap-aware half is exactly the "differentiable filmic operator" the donor trains through, so the dither decision is made against the same operator the denoiser assumes. HX11 keeps the "resolve differs from the plain box-clamp resolve" clause. |
| 9 | **Gaussian Light Transport**<br>`arxiv.org/abs/2609.11430` (2026) | `StokastX/gaussian-light-transport`<br>BSD-3-Clause, **21** stars, pushed 2026-10-19 | Express the light-transport solution as a **13D Gaussian mixture** over position, direction, surface normal and material, so scene properties live inside the representation instead of inside a network. Parameters are fitted by minimising the **residual of the rendering equation** (not a Neumann series), and an explicit **culling strategy** keeps the optimisation tractable and the render real-time. The win to copy: "including scene properties in the Gaussian representation drastically reduces the number of functions and speeds up evaluation". | `crates/render/src/restir_di.rs:31` (`RestirDi`), `:9` (`DI_TORCHES = 250`), `:26` (`DiReceipt`)<br>`crates/render/src/gi_probe.rs:120` (`update`), `:155` (`sample_trilerp`)<br>`crates/render/src/gi_probe.rs:88` (`ProbeGrid`) | **RT4** | RT4 keeps its Accept (a day-to-night sun sweep updates bounce light with **no bake step**, ReSTIR GI + cache ≤ **2.5 ms**; the gate tunnel leak test passes) and gains two printed lines per frame: the Gaussian count kept after culling and the **cull ratio**. The probe grid and the cache must agree within **2 %** after the sweep, so the two representations cannot drift apart silently. |
| 10 | **OctaOctree Neural Radiosity for Real-time Glossy Material Rendering**<br>`arxiv.org/abs/2606.08469` (2026) | `jieruijerry/OctaOctree`<br>MIT, **6** stars, pushed 2026-09-25<br>description: "a spatial-angular neural radiosity cache for real-time glossy global illumination" | An **adaptive octree in 3D** where every spatial node carries an **octahedral directional map**: fine spatial resolution where illumination and visibility change, coarser spatial levels with richer angular resolution for glossy and specular lobes. Result: **one network query at the primary intersection** covers diffuse interreflection *and* sharp glossy reflections. Our probe grid is a flat 4×2×4 lattice with a trilerp lookup — it cannot represent a direction-dependent lobe at all. | `crates/render/src/gi_probe.rs:23` (`PROBE_NX`), `:25` (`PROBE_NY`), `:27` (`PROBE_NZ`), `:29` (`PROBE_COUNT`)<br>`crates/render/src/gi_probe.rs:155` (`sample_trilerp`)<br>`crates/render/src/baked_gi.rs:60` (`sample_lightmap`) | **RT4** | RT4 keeps its Accept and gains the glossy half: the probe grid grows a **per-node directional lobe**, the day-to-night sweep re-lights the glossy floor within **2.5 ms**, and on a matte/glossy fixture pair the lobe's **direction** matches the donor's single-query output and its magnitude is within **10 %**. `sample_trilerp` stays the matte path and is byte-equal when the lobe is off. |
| 11 | **A Physics-embedded Deep Learning Framework for Cloth Simulation**<br>`arxiv.org/abs/2403.12820` (2024) | `Furkath/DL_Framework-for-PBS-Cloth-Simulation`<br>MIT, **25** stars, pushed 2024-05-28 (the paper's own release)<br>`ThomasConrad/gpu-cloth-sim2`<br>MIT, **1** star — **Rust + wgpu + XPBD**, our exact stack<br>`steampower33/XPBD-Cloth` MIT, 25 stars (Vulkan compute)<br>`kubanemil/physicall` MIT, 0 stars (Rust + WebGPU XPBD **with a CPU reference solver**) | The paper's lesson is *physics embedded in the model*: a convolutional network represents the **spatial correlations** of the mass-spring system and three branches learn linear, nonlinear and time-derivative features, so the solver needs no per-scene training. The engine lesson is from the donors, not the paper: XPBD "small steps" substepping, and — the part we actually need — a **CPU reference solver beside the GPU one**. Our HX8 Accept already asks for "two runs of 200 steps agree within 1e-3"; a GPU cloth solver with no CPU twin cannot produce that number. | `crates/physfx/src/lib.rs:1` (crate contract: "a float Rapier world seeded by authority events, never hashed")<br>`crates/physfx/src/verbs.rs:150` (`fx_limb` — the float, non-hashed layer)<br>`crates/physfx/src/lib.rs` (module list; `cloth.rs` is the new file HX8 owns) | **HX8** | HX8 keeps its Accept verbatim: a 8×8 sheet pinned at its top row sags under gravity and settles, **two runs of 200 steps agree within 1e-3**, the cloth never enters the hashed tick, and the ragdoll/debris budgets stay inside their caps. The steal that makes it possible: the GPU solver ships with a **CPU reference solver** (the `physicall` pattern) so the 1e-3 number is a real cross-check and not a self-comparison. |
| 12 | **A2Z GameSpec-Bench: How Faithfully Can Coding Agents Generate Games from Game Design Specifications?**<br>`arxiv.org/abs/2609.39564` (2026) | `krafton-ai/a2z-gamespec-bench`<br>MIT, **15** stars, pushed 2026-10-08<br>site repo `A2Z-GameSpec-Bench/a2z-gamespec-bench.github.io` MIT, 0 stars | Each long-form Game Design Document is turned into a **dependency-aware contract**: rules, constraints and **prerequisite relations** between requirements. The contract is **fixed across agents and revision rounds**, while the judgments and the evidence linked to each requirement move. Faithfulness is measured by whether the game satisfies the requirements *and preserves the relations among them*. Measured: current agents struggle to satisfy interdependent requirements jointly; **requirement-specific feedback improves GDD Fidelity by 10.9 %** relative to plain self-revision after two rounds. | `crates/playtest/src/brief_eval.rs:34` (`Brief` — our briefs are the contract's seed), `:45` (`BRIEFS`), `:105` (`BriefScore`)<br>`crates/endpoint/src/packs/verify.rs:398` (`VerifyProofArgs`), `:418` (`VerifyProofOutput`), `:407` (`MirrorTaskDigest`)<br>`crates/endpoint/src/packs/rules/verbs.rs:196` (`RulesTestCase`) | **PW24**, **HX29** | PW24 keeps its Accept and gains the **contract shape**: each brief becomes a contract of rules + constraints + prerequisite relations, and the referee names the first failing *rule* (not the first failing assertion) — the prerequisite order is what makes the failure explainable. HX29 keeps its Accept (named predicates, fixed severity order, a mutant that fails exactly its criterion, unevaluable is a fail). The measurable half: our requirement-specific feedback beats plain self-revision by **≥ 5 %** of GDD Fidelity after two rounds on the same brief set (the donor measures 10.9 %). |

---

## Wild ideas (10)

Each one is a *new* angle, not a repeat of wave 1 (wave 1's shared blue-noise blueprint
is item 4 in its "one screen", so item 2 below is deliberately a different blue-noise
idea: the **generator**, not the asset).

1. **One stochastic filter footprint for every textured surface** (row 1). RTXTF's
   trick — one texel per frame per pixel, reconstructed by the TAA upscaler — can be
   reused for the decal dither and the wound map's dither, not only for albedo. The
   landing is `crates/render/src/post/taa_resolve.wgsl:7`: the same stochastic
   selection that filters the albedo also filters the decal, so a decal stops
   shimmering for free. Wild because it changes what "resolve" means.

2. **A local, parallel, exactly-distributed blue-noise generator in a compute shader**
   (`arxiv.org/abs/2608.13446`, *Blue Noise as a Lattice Gibbs Ensemble*, 2026).
   The paper's contribution is that blue noise is a Gibbs distribution over a lattice,
   so tiles can be sampled independently and in any order with a bounded working
   window — peak memory under 51 MiB for a 14557×8418 image, set by the window and
   not the image. Wild idea: bake our blue-noise texture **at load time in a compute
   pass** instead of shipping it, so the jitter, the GTAO rotation and the decal
   dither each draw their own tile from a distribution we can name. Landing:
   `crates/render/src/post_jitter_motion.rs:22` (`halton`) is replaced by a sampler
   over that texture.

3. **Unit-variance white noise with controlled temporal correlation**
   (`arxiv.org/abs/2610.11653`, *Tabula Rasa*, 2026) as one shared asset for
   crowd jitter, particle jitter and the TAA dither, so three systems stop using
   three different hashes. The technique is "joint Monte-Carlo estimation of a pixel
   reconstruction and of its variance, using sketching from the database literature".
   Landing: `crates/render/src/post_jitter_motion.rs:55` (`jitter_uniform`) and the
   crowd's own jitter in `crates/anim/src/crowd_scale.rs:152`.

4. **A rejection test that reads the scene, not the pixels** (row 2's Cycles finding).
   The donor measured that on path-traced content a neighbourhood colour test is
   blind: a moving object re-lights the static floor by a few percent per frame
   (soft shadow ≈ 60 %, bounce ≈ 40 %), which is why every temporal method sits
   **4-6 dB below Lanczos** there. Their fix — re-render the current frame under
   the *previous* frame's scene state and compare the same sample points — recovers
   **2.7 dB**, and re-shading a quarter of the pixels keeps most of it. Wild idea:
   our velocity target (PW01) is already a "render twice, compare" structure, so the
   same machinery gives us a *lighting-change* channel, not only a motion channel.
   Landing: `crates/render/src/post_jitter_motion.rs:89` (`MotionField`) grows a
   second channel.

5. **Re-shading stored history instead of rejecting it** (row 2's Part V). The
   donor's ceiling measurement: re-shading every history sample at its own point
   gains **1.1 dB** on both analytic layouts, two thirds of the oracle rejection
   bound, and stacks with it — history under moving light is worth a lot, but only
   by re-lighting samples, not by estimating the change from a few probes. Wild
   idea: our TAA history keeps a *reference* to the light state, so a stored sample
   can be re-lit by the current sun instead of being thrown away.

6. **A cluster hierarchy that is a *lookup*, not a cut** (row 3 + row 10). Nanite's
   LOD error metric and our `cluster_cut` both answer "which cluster", but the
   donor's impostors answer "which image". Wild idea: one dithered billboard path
   for far clusters and one directional lobe per near cluster (row 10), so a
   cluster is either a texture or a lobe, never a mesh. Landing:
   `crates/render/src/meshlet.rs:39` (`Cluster`) grows a `kind` byte.

7. **The environment becomes a first-class term in the motion query** (row 4).
   The donor's `EnvironmentSearchBurst.cs` is a *burst* search that penalises
   collisions while matching pose — and the paper's own complaint is that pose and
   trajectory are solved separately, which is exactly our crowd's two-stage
   problem. Wild idea: one query object for both, so the same environment penalty
   steers a lone fighter and a crowd agent. Landing:
   `crates/anim/src/motion_match.rs:241` (`build_query`).

8. **A scene graph for the door, not for the renderer** (row 7 / SAGA,
   `arxiv.org/abs/2606.29932`). SAGA's *Map-Semantic Scene Graph* turns raw
   coordinates into per-entity statements of distance, direction and threat, and
   their Tool-Augmented Planner then **retrieves only the state a decision needs**,
   which is where the measured **27 % fewer output tokens** comes from. Wild idea:
   our door already has `scene.spatial` and `scene.next`; make the *spatial graph*
   the thing the model reads, so the token saving and the "spatial understanding"
   pillar land together. Landing: `crates/endpoint/src/packs/` and the
   `SPX4`/`SPF3` verbs.

9. **Learn the generator's primitives instead of the generator** (CAD,
   `arxiv.org/abs/2608.17947`). 160 runs, at least ten 50-generation runs in every
   cell of a 2×2 experiment; Continual Abstraction Discovery raises mean final
   best fitness in **all eight** domain-and-API comparisons, and the learned
   libraries are adopted by most later programs and keep rediscovering validation,
   reachability and structural utilities. Wild idea: our grammar pack becomes a
   place where a run leaves behind reusable level primitives, so the next run
   starts richer. Landing: `crates/endpoint/src/packs/grammar.rs:9`
   (`grammar_verbs`).

10. **Three referee classes instead of one** (rows 5 and 12). OpenGameEval scores
    each run twice — executable checks on the edited scene *and* a simulated play
    session — and A2Z scores the *relations* between requirements, not the
    requirements alone. Wild idea: every playtest task gets three verdicts
    (builds, plays, and satisfies its prerequisites), so "unevaluable is a fail"
    (HX29) has a per-class home instead of one boolean. Landing:
    `crates/playtest/src/brief_eval.rs:105` (`BriefScore`).

---

## Rejected (10)

Each of these is a 2024-2026 paper I read the abstract of and then dropped.
The most common reason is that **no public code exists**, which I checked with
GitHub code search on the exact title and on the arXiv id: every hit was a
paper-digest or arXiv-stats repository, never an implementation.

1. **Shallow Signed Distance Functions for Kinematic Collision Bodies** —
   `arxiv.org/abs/2411.06719` (2024). No public code found (code search on the
   title returns only paper digests). Rejected on our rules as well: a learned SDF
   for a kinematic avatar breaks the integer-tick contract, and cloth collision for
   our fighters is a shape we own, not a shape we learn.
2. **Projective Affine Body Dynamics for Multibody Systems** —
   `arxiv.org/abs/2609.02675` (2026). No public code found. Rejected on scope: a
   Hessian-free peridynamics solver is a new solver, and our physics authority is
   Rapier (`crates/physfx/src/lib.rs:1`).
3. **Geometry-Aware Global Feature Aggregation for Real-Time Indirect
   Illumination** — `arxiv.org/abs/2508.08826` (2025). No public code found.
   Rejected on scope: a trained neural estimator needs a training corpus and a
   runtime net, which is not a row-sized piece of work and does not fit the
   "the renderer owns the look" pillar.
4. **A Hybrid Neural-Microfacet BRDF Model for Real-Time Rendering** —
   `arxiv.org/abs/2608.09604` (2026). No public code found. Rejected on contract:
   a neural BRDF breaks the artist-editable material row and the "no float in the
   tick" spirit of our material tables.
5. **Aokana: A GPU-Driven Voxel Rendering Framework for Open World Games** —
   `arxiv.org/abs/2505.02017` (2025). No public code found (the name is also a
   Japanese romance game, which crowds the search). Rejected on size: an SVDAG
   voxel renderer with LOD and streaming is a renderer, not a row, and its win
   (up to 9× less memory, 4.8× faster) does not apply to our mesh arena.
6. **Real-Time Rendering of Dynamic Line Sets using Voxel Ray Tracing** —
   `arxiv.org/abs/2510.09081` (2025). No public code found. Rejected on domain:
   the target is MRI white-matter and unsteady-flow line sets; our line content
   is blade trails, which is a different problem.
7. **Neural Dynamic GI: Random-Access Neural Compression for Temporal Lightmaps**
   — `arxiv.org/abs/2604.12625` (2026). No public code found (only paper-note
   repos). Rejected on scope: it needs a trained net **and** a virtual-texturing
   system; RT4 is better served by rows 9 and 10, which need no training.
8. **A Database-Driven Framework for 3D Level Generation with LLMs** —
   `arxiv.org/abs/2508.18533` (2025). No public code found. Rejected on
   duplication: the room/facility/mechanics database duplicates the WFC grammar
   we already own (`crates/endpoint/src/packs/grammar.rs:9`).
9. **Blue Noise as a Lattice Gibbs Ensemble** — `arxiv.org/abs/2608.13446` (2026).
   No public code found (the author list on the author's site has no matching
   repository). The *idea* survives as wild idea 2, but there is no reference
   implementation to port, so it stays out of the ranked table.
10. **Procedural Content Metageneration via Program Search and Continual
    Abstraction Discovery** — `arxiv.org/abs/2608.17947` (2026). The only
    artefact is a 0-star website repo with **no licence and no code**
    (`matt-quant-heads-io/procedural-content-metageneration`). Rejected as a
    donor; the CAD idea moves to wild idea 9.

---

## Donors seen but skipped

These came up in my search and I did **not** take them, because wave 1 already
mined them (checked against `docs/research/hunt-2026-10-21/donors-seen.txt` and
the five wave-1 agent files). I name them so the next hunter does not re-mine them.

| donor | why skipped |
|---|---|
| `AlayaLab/AgenticSTS` (113★, NOASSERTION) — the official code for `arxiv.org/abs/2607.02255` | on the wave-1 donor list. It is a good bounded-memory testbed (298 released trajectories; a no-store baseline wins 3/10 and the skill layer 6/10, Fisher p ≈ 0.37 — directional, not decisive), so it is a real loss, but the rule is the rule. |
| `facebookresearch/Tabula-Rasa` (1★, NOASSERTION) — the official code for `arxiv.org/abs/2610.11653` | on the wave-1 donor list. The technique survives as wild idea 3; the repo does not. |
| `Scthe/frostbitten-hair-webgpu` (111★, MIT) | on the wave-1 donor list. Its sibling `Scthe/nanite-webgpu` is **not** on that list, which is why row 3 is allowed. |
| `github.com/sirujiang` (owner string on the wave-1 list) → `sirujiang/WorldSolver` (2★, MIT), the official code for `arxiv.org/abs/2610.08720` | the exact repo string is not on the list but the owner is, so I treated it as seen. The paper's shape is worth noting: 168 tasks from 61 classic CG papers across 7 domains, scored on Execution / Visual Fidelity / Physical Plausibility; the best frontier agents reach only **48.7 %** and **46.7 %**. That triple scoring is wild idea 10. |
| `NVIDIA-RTX/RTXCR` and `NVIDIA-RTX/SHARC` | on the wave-1 donor list. `NVIDIA-RTX/RTXTF` is **not** on it, which is why row 1 is allowed. |
| `vitalight/Velvet` (221★, MIT, CUDA XPBD cloth) and `nakjun/Cloth-Simulation-WebGPU` (25★) | both on the wave-1 donor list. Row 11 uses `Furkath/DL_Framework-for-PBS-Cloth-Simulation` (25★, MIT), `ThomasConrad/gpu-cloth-sim2` (1★, MIT, Rust+wgpu) and `steampower33/XPBD-Cloth` (25★, MIT) instead, none of which is on the list. |
| `Q-Minh/position-based-dynamics` (88★, BSL-1.0) | on the wave-1 donor list; it was wave 1's XPBD donor, so row 11 deliberately points at a different, Rust-native one. |
| `NRD/RTXDI` | on the wave-1 donor list; the ReSTIR donor lane is already mined, so rows 8-10 go through the GI-probe and upscaling rows instead. |
| `Siroko/kansei`, `vga-group/taa-kdop-optimizer`, `nff747/meshlet-culling-wgsl`, `DeharengOlivier/gpu-cloth-simulation`, `iodiot/webgpu-crowd` | wave 1's TAA, TAA-ghost, meshlet-cull, cloth and crowd donors. Rows 1-3 and 11 deliberately use *different* donors for the same rows (RTXTF + decibels, nanite-webgpu, and the Rust/wgpu cloth repos). |

---

## What I did not do

- I did not run cargo, rustc, clippy, nextest or any build. No proof is claimed
  here beyond what the donors and papers measured.
- I did not touch `crates/**`, `content/**` or any `docs/SPRINT*.md`.
- I did not commit, push, checkout, reset, stash or clean.
- Golden hash `d57b5c8ae3096614` is untouched; every row above is presentation-side
  or a new file inside a crate that a row already owns.
