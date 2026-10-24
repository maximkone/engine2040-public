# GitHub mining: open-source game tech to steal (2026-10-24)

Scope: GitHub repos for engine2040 (deterministic Rust melee, Roman city builder, multiplayer, ragdolls and destruction, AI agents that drive the editor, must run on a weak PC). License does not matter for this pass. Star counts and languages are as each repo page showed them when fetched on 2026-10-24.

## Method and limits

- Every pick was fetched directly with WebFetch on its github.com page. A pick is listed only if the page loaded. Where the page left the language field empty, the entry says "not shown" and does not guess.
- Stars are approximate, as shown on the page at fetch time.
- The named awesome lists (awesome-gamedev, awesome-godot, awesome-procedural-generation, awesome-gamedev-ai, awesome-game-engine-dev, awesome-unity3d) were NOT opened as lists. Searches returned mirrors and aggregators only. The one list opened was nolantait/awesome-bevy (about 226 stars; Bevy 0.18+ resources, categories include Assets, Networking, Physics). So sourcing leans on repo pages, not curated lists. Coverage is thinner than the brief asked for.
- GitHub topic pages (motion-matching, gpu-driven-rendering, netcode, rollback) and GitHub Trending did not come back through search. Show HN and trending searches found no usable 2026 open-source engine post. Unverified leads from news snippets only: Cocos 4 (open-sourced, Jan 2026) and Stride (C#, about 7.2k stars per a snippet). Not in the list below.
- donors-seen.txt was read and every pick was grepped against it. None of the 44 picks is in it.
- The three plugin files were read. Anything already in them was skipped (see "Skipped").

---

## 1. Rendering and look on weak GPUs

**1. bgfx**: https://github.com/bkaradzic/bgfx
- ~17.6k stars. Language field empty; C/C++ API. BSD-2-Clause.
- What: cross-platform rendering library, graphics-API agnostic. Backends listed on the page: Direct3D 11/12, Metal, OpenGL, OpenGL ES, Vulkan, WebGL 2, WebGPU (Dawn only).
- Key idea: one submission API over every backend, so the weak-GPU fallback is a backend switch, not a renderer rewrite. (Draw sorting details are not on the page.)
- Steal: keep the renderer behind one submit interface with a GL or Vulkan path for the iGPU tier.

**2. sokol**: https://github.com/floooh/sokol
- ~10.4k stars. C. Zlib.
- What: minimal C libraries for graphics, windowing, audio, assets and timing.
- Key idea: single-header, standalone libraries that can be taken one at a time. (Per-frame overhead is not on the page.)
- Steal: use the audio or gfx header as a thin, dependency-free shim for the web or potato build.

**3. Slang**: https://github.com/shader-slang/slang
- ~5.8k stars. Language not shown. Apache-2.0 with LLVM exception.
- What: one shading language compiled to D3D12, Vulkan, Metal and CUDA, with automatic differentiation that generates forward and backward derivative code.
- Key idea: write each effect once for every backend, and get gradients for free.
- Steal: one shader source per effect for all backends; autodiff to tune a look parameter offline against reference frames.

**4. FidelityFX-SDK (AMD)**: https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK
- ~1.8k stars. Language not shown. MIT for the open samples.
- What: FSR upscaling (temporal and ML variants), frame generation, ray-regeneration denoising, radiance caching (preview).
- Key idea: render below native resolution and upscale. The page says DirectX 12 only; Vulkan is not supported.
- Steal: a render-scale tier setting for the potato floor. The older FSR repo is already in donors-seen (GPUOpen-Effects/FidelityFX-FSR); this is the wider SDK.

**5. splat (antimatter15)**: https://github.com/antimatter15/splat
- ~3.1k stars. JavaScript (WebGL 1.0). MIT.
- What: browser Gaussian-splat viewer.
- Key idea: each splat is a projected quad; the fragment shader fades opacity from the centre. Splats are sorted on the CPU in a web worker, not on the GPU.
- Steal: if captured props ever ship as splats, the worker-sort pattern keeps them viable on a weak browser GPU.

---

## 2. Physics, destruction, cloth, ropes

**6. Newton**: https://github.com/newton-physics/newton
- ~5.8k stars. Python on NVIDIA Warp. Apache-2.0 (code); CC-BY-4.0 (docs).
- What: GPU physics engine built on Warp. The page says MuJoCo Warp is the primary backend and lists OpenUSD support and differentiability.
- Key idea: solver examples named on the page include MPM, VBD and ADMM (not explained there).
- Steal: VBD and MPM as leads for cloth and soft contact; the differentiable path for auto-tuning. Needs an NVIDIA GPU, so offline or tooling only.

**7. NVIDIA Warp**: https://github.com/NVIDIA/warp
- ~7.2k stars. Python. Apache-2.0.
- What: Python functions JIT-compiled to CPU or GPU kernels; kernels are differentiable.
- Key idea: one kernel source runs on CPU or GPU.
- Steal: write sim kernels once and run the CPU path on the potato floor; differentiate hit-feel parameters offline.

**8. Chrono**: https://github.com/projectchrono/chrono
- ~3.1k stars. C++ (Python and C# APIs). BSD-3-Clause.
- What: multiphysics engine: rigid multibody as DAEs, deformable bodies via FEA, granular contact (non-smooth or smooth DAE forms), fluid-solid coupling.
- Key idea: granular contact formulations for piles of material.
- Steal: rubble and grain contact formulations, baked offline. Too heavy for runtime on a potato.

**9. Brax**: https://github.com/google/brax
- ~3.3k stars. JAX (Python). Apache-2.0.
- What: massively parallel rigid-body physics on accelerators, differentiable. The README says only brax/training is actively maintained as of 0.13.0 and points to MuJoCo Playground and MJX.
- Key idea: thousands of environments stepped in one batch.
- Steal: batch simulated melee duels offline to balance numbers. Tooling only, not the potato runtime.

**10. Fluid-Sim (Sebastian Lague)**: https://github.com/SebLague/Fluid-Sim
- ~1.3k stars. Unity 2022.3 project; language not shown. MIT.
- What: simple particle-based fluid (SPH), with SPH tutorial references.
- Key idea: SPH particles for splashes and dust. Spatial hashing is not verified on the page.
- Steal: a few hundred splash particles, baked to flipbook on the potato tier.

---

## 3. Animation and motion

**11. AI4Animation (Sebastian Starke)**: https://github.com/sebastianstarke/AI4Animation
- ~8.9k stars. Unity (language not shown). Research and education license; the page says not for commercial use or redistribution. (The owner says license is not a concern for this pass.)
- What: data-driven character animation with neural networks: PFNN (SIGGRAPH 2017), MANN (2018), DeepPhase periodic autoencoder (2022).
- Key idea: a learned phase manifold drives locomotion, so one model covers many gaits.
- Steal: one shared learned gait model for citizens and crowds, with cheap runtime cost once trained.

**12. ozz-animation**: https://github.com/guillaumeblanc/ozz-animation
- ~3.0k stars. C++. MIT.
- What: renderer-independent runtime skeletal animation: loading, sampling, blending. Data-oriented, SoA and SIMD. The page does not show IK.
- Key idea: structure-of-arrays pose storage for cheap blending.
- Steal: SoA poses and blending for hundreds of characters on a weak CPU.

---

## 4. Procedural generation, worldgen, city

**13. MarkovJunior**: https://github.com/mxgmn/MarkovJunior
- ~8.5k stars. C#. MIT.
- What: probabilistic rewrite-rule generation. Rules combine into sequence and Markov nodes, models are defined in XML, and constraint propagation steers toward goal states. The page lists 153 examples.
- Key idea: generation as declarative rule data, not code.
- Steal: the AI writes XML rule sets for city blocks, walls and ruins; a seed gives the same output.

**14. Julius (Caesar III re-implementation)**: https://github.com/bvschaik/julius
- ~3.4k stars. Language not shown. AGPL-3.0.
- What: open re-implementation of Caesar III that keeps the original logic and save compatibility, bugs included.
- Key idea: a ground-truth reference for Roman city economics.
- Steal: check our granary, market and labour numbers against its logic.

**15. Augustus (Julius fork)**: https://github.com/Keriew/augustus
- ~2.1k stars. Language not shown. AGPL-3.0.
- What: Julius fork adding roadblocks, market special orders, a global labour pool, partial warehouse storage, higher game limits and zoom.
- Key idea: concrete gameplay changes layered on the same logic.
- Steal: roadblocks and a global labour pool are direct candidate mechanics for the Roman builder.

**16. OpenTTD**: https://github.com/OpenTTD/OpenTTD
- ~8.4k stars. Language not shown. GPL-2.0.
- What: re-creation of Transport Tycoon Deluxe that mimics the original closely and adds features. Road and rail detail is not on the page.
- Key idea: a mature transport-economy simulation.
- Steal: route-based logistics reference for city supply lines.

**17. SUMO**: https://github.com/eclipse-sumo/sumo
- ~4.2k stars. Language not shown. EPL-2.0.
- What: microscopic traffic simulator for large road networks, with multimodal transport and scenario tools. Car-following models are not on the page.
- Key idea: per-vehicle traffic on real network graphs.
- Steal: bake traffic density for city roads offline; not a runtime system.

**18. Mindustry**: https://github.com/Anuken/Mindustry
- ~29.5k stars. Java. GPL-3.0.
- What: automation tower-defence RTS. The page shows entity classes generated at build time from component classes in mindustry.entities.comp. Conveyor and logic-processor details are not on the page.
- Key idea: factory automation as the core loop, with component-generated entity classes.
- Steal: component-generated entity classes for building types, and automation as a model for the science-output loop in the Silicon City plan.

**19. shapez.io**: https://github.com/tobspr-games/shapez.io
- ~7.0k stars. JavaScript (ES5). GPL-3.0.
- What: factory base-building game. The README says it is no longer actively maintained and points to shapez 2. Chunked rendering is not on the page.
- Key idea: shape recipes as a production graph.
- Steal: a reference for production-graph UX. Low priority.

---

## 5. AI, NPC, agents

**20. Voyager (MineDojo)**: https://github.com/MineDojo/Voyager
- ~7.3k stars. Language not shown (Python and Node.js install). MIT.
- What: LLM agent with an automatic curriculum. Learned behaviours are stored as executable code in a growing skill library and reused later.
- Key idea: verified code becomes a reusable skill.
- Steal: the editor-driving agent saves each verified macro as a skill file and reuses it.

**21. mindcraft**: https://github.com/mindcraft-bots/mindcraft
- ~5.9k stars. Node.js. MIT.
- What: LLM controls Mineflayer Minecraft bots through code-as-action. The page says code writing is disabled by default.
- Key idea: a default-off switch for model-written code.
- Steal: the same default-off gate for any AI that writes scripts into the editor.

**22. PufferLib**: https://github.com/PufferAI/PufferLib
- ~6.6k stars. Language not shown. MIT.
- What: fast reinforcement-learning library. The README says it trains tiny models in seconds. Vectorised environment internals are not on the page.
- Key idea: throughput-first RL.
- Steal: AI playtest throughput on a small box. Measure before adopting.

**23. godot_rl_agents**: https://github.com/edbeeching/godot_rl_agents
- ~1.6k stars. Python (language not shown). MIT.
- What: Godot-to-Python RL interface, working with Stable-Baselines3, Sample Factory, RLlib and CleanRL. The page notes that the Godot env freezes while the model updates in the default script.
- Key idea: the game runs as a stepped environment over a link.
- Steal: the env protocol for an editor-side AI harness.

**24. ML-Agents (Unity)**: https://github.com/Unity-Technologies/ml-agents
- ~19.8k stars. Language not shown. Apache-2.0.
- What: toolkit that makes games and simulations into RL and imitation-learning environments.
- Key idea: a stable observation and action contract between game and trainer.
- Steal: the same contract for AI agents that drive the editor.

---

## 6. Netcode and determinism

**25. GGPO (pond3r)**: https://github.com/pond3r/ggpo
- ~3.6k stars. Language not shown (C SDK). MIT.
- What: "Good Game, Peace Out" rollback networking SDK. It predicts remote inputs and runs speculatively, so local inputs take effect at once.
- Key idea: predict, simulate, roll back on mismatch.
- Steal: reference design for the melee rollback layer. Rollback needs a deterministic sim, which engine2040 has. The Rust ggrs port is already in donors-seen.

**26. OpenRA**: https://github.com/OpenRA/OpenRA
- ~17.6k stars. C#. GPL-3.0.
- What: open RTS engine for classic Westwood games. The page does not describe its networking, so lockstep is not verified here.
- Key idea: unverified. Lead only.
- Steal: read its sync and order code to see whether it runs lockstep, and whether that fits our tick model.

**27. netfox (foxssake)**: https://github.com/foxssake/netfox
- ~1.1k stars. GDScript. MIT.
- What: Godot addons for online multiplayer: rollback, client-side prediction with server reconciliation, interpolation, synced timing.
- Key idea: the timing and tick sync layer is a separate, reusable piece.
- Steal: a design check for our tick and replay API.

**28. renew (renew-engine)**: https://github.com/renew-engine/renew
- ~28 stars. Rust. Apache-2.0. Pre-0.1.
- What: fixed timestep with Q47.16 fixed-point arithmetic. The page says one build, seed and input set should give identical output, and CI checks that across five targets.
- Key idea: bit-identical output checked in CI on several targets.
- Steal: copy the cross-target identity CI check to back the determinism claim.

---

## 7. Tools, editor, debug

**29. rr**: https://github.com/rr-debugger/rr
- ~10.7k stars. Language not shown. License not shown on the page.
- What: records and replays Linux executions, with reverse execution in gdb.
- Key idea: record once, replay the same run as often as needed.
- Steal: record a failing PC run and replay it to chase non-reproducible crashes.

**30. bevy-inspector-egui**: https://github.com/jakobhellermann/bevy-inspector-egui
- ~1.6k stars. Rust (inferred). Apache-2.0 and MIT.
- What: runtime inspector for Bevy entities, resources and assets, built on Bevy's Reflect and egui.
- Key idea: reflection drives a live inspector with no hand-written UI.
- Steal: a reflect-driven live inspector, but only if engine2040 runs on Bevy or exposes a reflect registry.

**31. Dear ImGui**: https://github.com/ocornut/imgui
- ~76.8k stars. C++. MIT.
- What: immediate-mode GUI for developer tools. Widgets are function calls each frame; no retained widget tree; output is vertex and draw lists.
- Key idea: no UI state to keep in sync.
- Steal: debug overlays for physics and AI state.

**32. egui-snarl**: https://github.com/zakarumych/egui-snarl
- ~604 stars. Rust. MIT.
- What: generic node-graph widget. A graph data type plus a SnarlViewer trait supplies node UI; serde serialization.
- Key idea: the graph data and its UI are separate.
- Steal: an editable graph view for AI-authored procgen and behaviour graphs.

**33. proptest**: https://github.com/proptest-rs/proptest
- ~2.3k stars. Rust. Apache-2.0 and MIT.
- What: Hypothesis-style property testing with automatic shrinking to a minimal case; failing seeds saved to regression files.
- Key idea: shrink a failing input to the smallest repro.
- Steal: shrink determinism failures (same seed, different output) to a minimal case.

**34. tokio-console**: https://github.com/tokio-rs/console
- ~4.6k stars. Rust. MIT.
- What: live async-task viewer fed by a tracing Layer over gRPC.
- Key idea: live task view for async code.
- Steal: only if the server runs async tasks. Low priority.

**35. EnTT**: https://github.com/skypjack/entt
- ~13.2k stars. C++. MIT.
- What: entity-component system with component views, a pay-for-what-you-use policy and optional pointer stability. The page says sparse sets back its hash map, not the ECS storage. Groups are not on the page.
- Key idea: pay only for the features used.
- Steal: compare view-iteration cost with our ECS in the combat hot loop.

---

## 8. Audio

**36. fundsp**: https://github.com/SamiPerttu/fundsp
- ~1.2k stars. Rust. Apache-2.0 and MIT.
- What: audio DSP graphs written as operator expressions (pipe, bus, branch, stack), compiled to stack-allocated inlined code. Channel mismatches are caught at build time.
- Key idea: typed audio graphs with no runtime allocation.
- Steal: build impact and foley voices as graphs at load time.

**37. Faust**: https://github.com/grame-cncm/faust
- ~3.2k stars. Language not shown. License not shown on the page.
- What: functional DSP language compiled to C++, C, Java, LLVM IR and WebAssembly; libfaust JIT.
- Key idea: one DSP source, many targets.
- Steal: write combat SFX in the DSL and compile to WASM for the web build.

**38. DDSP (Magenta)**: https://github.com/magenta/ddsp
- ~3.4k stars. Python. Apache-2.0.
- What: differentiable synthesizers and filters that sit inside neural networks and train end to end.
- Key idea: a synth whose controls can be learned from audio.
- Steal: fit a small synth to recorded clips offline, then ship only the parameters.

**39. oddio**: https://github.com/Ralith/oddio
- ~165 stars. Rust. Apache-2.0 and MIT.
- What: small real-time game mixer with 3D placement, Doppler and propagation delay. The page calls it wait-free.
- Key idea: propagation delay for distant sources.
- Steal: arrow and siege-impact sounds with delay across the field.

**40. Resonance Audio web SDK**: https://github.com/resonance-audio/resonance-audio-web-sdk
- ~222 stars. JavaScript. Apache-2.0. ARCHIVED by its owner on 2026-04-19 (read-only).
- What: real-time spatial audio on Web Audio. Room acoustics and HRTF are not on the page.
- Key idea: browser-side spatial audio.
- Steal: reference only, because it is archived.

---

## 9. Wild and mind-blowing

**41. The Powder Toy**: https://github.com/The-Powder-Toy/The-Powder-Toy
- ~5.4k stars. C++. GPL-3.0.
- What: falling-particle sandbox with air pressure, velocity, heat and element interactions.
- Key idea: many interacting substances on a grid.
- Steal: a test bed for heat and fire rules before real destruction rules.

**42. sandspiel**: https://github.com/MaxBittker/sandspiel
- ~3.2k stars. Rust compiled to WebAssembly, WebGL, some JavaScript. MIT.
- What: falling-sand cellular automaton in the browser.
- Key idea: a cellular sim packed into WASM with WebGL output.
- Steal: a browser demo path for a cellular sim.

**43. Lenia**: https://github.com/Chakazul/Lenia
- ~3.9k stars. Language not shown (Python, Matlab, JavaScript, R and Android folders). MIT.
- What: continuous-space, time and state cellular automaton. Kernel and growth details are not on the page.
- Key idea: continuous CA produces lifelike, organic patterns.
- Steal: continuous CA as an organic-growth generator (moss, ivy, fungus) on the procgen layer.

**44. Luanti (formerly Minetest)**: https://github.com/luanti-org/luanti
- ~13.8k stars. C++ (lua topic). LGPL-2.1.
- What: voxel sandbox engine with a client-server model and a Lua mod API.
- Key idea: a sandboxed scripting API for content.
- Steal: a sandboxed scripting layer for designer-authored content.

---

## Skipped

Already in the three plugin files (not repeated): Blast (02, item 26), LimboAI (03), Terrain3D and godot_voxel (03), Verlet Rope 4 and godot-jolt (02), Chaos, Obi, RayFire, DinoFracture, Behavior Designer, MapMagic 2, Gaia, Feel, Final IK, PuppetMaster, Odin, Juicee, Editor Console Pro, Amplify Impostors, Automatic LOD, Dynamic Fog, Unity Sentis, Ghost Recorder, FishNet, Mirror, Godot Rollback Netcode, NobodyWho, Beehave, Dialogue Manager, Dungeon Architect, Tessera Pro, and the Blender addons (including Tissue, Sverchok, Pipeify).

Already in donors-seen.txt: balaurengine/balaur (its fork ryo33/balaur was checked and is the same project; deterministic, Rapier, record and replay, but not added), ggrs and bevy_ggrs, Fyrox, DioxusLabs/dioxus (which contains subsecond), and others.

## Dropped (not verified)

- IKPy (Phylliida/IKPy): 404.
- ZeroEGGS (ubisoft-laforge/ZeroEGGS): 404.
- citybound (aeplay/citybound): 404. Not retried under another name.
- Distant Horizons (Distant-Horizons-Mod/DistantHorizons): 404.
- egui_node_graph (setzer22/egui_node_graph): 404.
- Zero-K and supercollider: fetch errors (socket hang up). Not retried.
- Quake III Arena (id-Software): page loads, but prediction and lag compensation are not on it, so no key idea is claimed.
- sokol per-frame overhead, Blast support-graph details, Chrono solver detail, MarkovJunior kernel detail: not on the pages; not claimed beyond what is above.
- Awesome lists not opened as lists: awesome-gamedev, awesome-godot, awesome-procedural-generation, awesome-gamedev-ai, awesome-game-engine-dev, awesome-unity3d.
- Unverified news leads: Cocos 4 (open-sourced Jan 2026), Stride (C#).

---

## TOP 15 for engine2040 (ranked by impact)

1. **Julius and Augustus (Caesar III)**: ground-truth Roman city logic plus concrete mechanics (roadblocks, labour pool). Direct fit for the builder. [14, 15]
2. **MarkovJunior**: seeded, declarative rule generation the AI can author in XML, for blocks, walls and ruins. [13]
3. **GGPO**: the reference rollback design for melee netcode on a deterministic sim we already have. [25]
4. **Voyager**: a skill library where verified editor code is saved and reused. Direct model for AI agents that drive the editor. [20]
5. **Mindustry**: factory automation plus component-generated entity classes, a model for the science-output loop. [18]
6. **mindcraft**: code-as-action with a default-off code-writing switch, the safety pattern for model-written editor scripts. [21]
7. **PufferLib**: throughput-first RL for AI playtests on a small machine. [22]
8. **ozz-animation**: SoA poses and blending, cheap for many characters on a weak CPU. [12]
9. **renew**: fixed-point plus fixed timestep, with a CI identity check across targets, to back the determinism claim. [28]
10. **FidelityFX-SDK**: render below native and upscale, for the potato tier. DX12 only. [4]
11. **AI4Animation**: a learned phase model for locomotion and crowds. Non-commercial license noted. [11]
12. **egui-snarl**: editable node-graph UI for AI-authored procgen and behaviour graphs. [32]
13. **rr**: record and replay a failing run to chase non-reproducible crashes. [29]
14. **proptest**: shrink determinism failures to a minimal repro. [33]
15. **fundsp**: typed audio graphs built at load time, no runtime allocation, for impact and foley. [36]

---

Output: this file only. No git operations. No other file was touched.
