# AI-native from the core: research, 2026-10-11

Scope: web research only, no repo edits. "Verified" means the URL was opened with WebFetch on 2026-10-11. Items seen only in search snippets are marked UNVERIFIED. Licenses are info only.

## What AI-native from the core means

A finish line a stranger can test. Each bullet is one check with a pass number.

1. **Coverage.** A script diffs the editor's command registry against the verb table. Result: 0 mutating commands without a door verb.
2. **Refuse by default.** 1,000 randomized mutating calls with no grant leave the world hash unchanged, and each returns a refusal code.
3. **Undo and replay.** 50 random verb sequences of length 1 to 50: undo-all gives a byte-identical scene serialization in 50 of 50. A saved verb log replays in a fresh process to the same world hash in 3 of 3 runs, and on a second machine.
4. **Same task, three backends.** A fixed brief (for example "arena with 2 gates and 4 pillars") run on mock, local and remote models. Mock runs are hash-identical across 3 runs. Local and remote runs pass the same verify suite (run_tests, one headless playtest, one view capture) in at least 80% of runs.
5. **Self-verify gate.** 5 seeded bugs in AI-built levels are rejected by the automated gate with no human involved: 0 of 5 accepted.
6. **Deterministic playtest.** A 600-tick AI-vs-AI fight run headless 5 times gives the same state digest 5 of 5. Freeze, step and step-until verbs exist.
7. **Packet discipline.** Each door verb lands as a diff of at most 200 lines with its own test. A free weak model passes at least 6 of 10 packets on the first try, on a fixed packet set.
8. **Offline and provenance.** With the network off, build, play and the off-tick asset job all run with zero runtime outbound calls. Peak VRAM measured by nvidia-smi stays at or under 6 GB. Every shipped asset has a manifest with model id, seed, input hash and license tag.

## Top 12 to steal or wire

Packet sizes are estimates, not verified. "Door verb" means an MCP verb. "Editor job" means the off-tick asset job.

| # | Name | URL | What we take | Runs where | VRAM | Packet est. |
|---|---|---|---|---|---|---|
| 1 | Unreal_mcp (ChiR24) | https://github.com/ChiR24/Unreal_mcp | Four meta-verbs (search, describe, execute, configure) over about 400 capabilities. A weak model sees 4 tools, not 400. | Door surface | none | ~150 lines |
| 2 | godot-mcp (hybridindie) | https://github.com/hybridindie/godot-mcp | Toolsets off by default (193 tools in 29 categories; only inspection on). Undo on node, script and signal changes. Replay recording. | Door verbs and replay | none | ~120 lines per toolset |
| 3 | mcp-unreal (remiphilippe) | https://github.com/remiphilippe/mcp-unreal | run_tests, run_visual_tests, list_tests, get_test_log, build_project, capture_viewport, pie_control. Test verbs return structured results. | Door verbs test.*, view.capture, play.* | none | ~180 lines |
| 4 | godot-mcp (satelliteoflove) | https://github.com/satelliteoflove/godot-mcp | godot_game_time (freeze, step, step_until a condition), godot_input (named actions), godot_runtime_state digest. This is the deterministic playtest model. | Runtime stepping plus door verbs | none | ~250 lines |
| 5 | unity-mcp (CoplayDev) | https://github.com/CoplayDev/unity-mcp | batch_execute (many ops in one call). Tool groups toggled per session. 25 read-only resources next to 50 tools. | Door: batch and resources | none | ~150 lines |
| 6 | blender-mcp (ahujasid) | https://github.com/ahujasid/blender-mcp | `look` renders a view for the model to inspect. get_scene_info summary. Do not copy execute_blender_code as-is; our door refuses arbitrary code by default. | Door verb view.look | none | ~120 lines |
| 7 | llama-cpp-rs (utilityai) | https://github.com/utilityai/llama-cpp-rs | Local LLM for the agent door. CUDA feature flag, n_gpu_layers sets the VRAM split, GGUF models. Run as a child process so it can be killed to free VRAM. | Runtime child process (LLM door backend) | Sample: Llama-2-7B used a 3.8 GiB CUDA buffer on an RTX 3090 (quant not stated). Fit at 6 GB depends on quant: UNVERIFIED. | ~250 lines |
| 8 | MagicArticulate (Seed3D) | https://github.com/Seed3D/MagicArticulate | Mesh to skeleton (autoregressive transformer) plus diffusion skinning. Needs the Michelangelo checkpoint. | Editor job (Python child) | 4.6 GB stated, 1 to 2 s per inference | ~200 lines |
| 9 | Stable Fast 3D (Stability AI) | https://github.com/Stability-AI/stable-fast-3d | Image to UV-unwrapped GLB with material parameters. Gated Hugging Face weights. | Editor job | ~6 GB stated | ~150 lines |
| 10 | TripoSR (VAST) | https://github.com/VAST-AI-Research/TripoSR | MIT image to mesh, under 0.5 s on A100 (author claim). Vertex colors by default, optional texture bake. Fallback if the SF3D license is a problem. | Editor job | ~6 GB stated | ~120 lines |
| 11 | Material Anything (3DTopia) | https://github.com/3DTopia/MaterialAnything | Mesh in; albedo, roughness, metallic and bump out. Our texture-from-mesh step. Weights on Hugging Face. | Editor job | UNVERIFIED (none stated) | ~200 lines |
| 12 | MoMask (EricGuo5513) or Light-T2M (qinghuannn) | https://github.com/EricGuo5513/momask-codes ; https://github.com/qinghuannn/light-t2m | Text to 22-joint motion at 20 fps. MoMask exports BVH. Light-T2M claims 4.48M params vs MoMask 44.85M (paper claim). Bake melee clips offline and retarget to our rig. | Editor job | UNVERIFIED for Light-T2M (tested on RTX 3090); MoMask web UI runs on CPU | ~200 lines |

## Category fit notes (6 GB card)

- **Image-to-3D.** TRELLIS.2 (4B, 24 GB minimum) and TRELLIS (16 GB minimum) do not fit. SPAR3D states 10.5 GB default and about 7 GB in low-VRAM mode. Hunyuan3D-2 states 6 GB for shape alone and 16 GB for shape plus texture; the 2mini (0.6B) VRAM is UNVERIFIED. TripoSR and SF3D state about 6 GB. Info: Hunyuan3D's license excludes the EU, UK and South Korea and needs a Tencent grant above 1M MAU.
- **Rigging.** UniRig states at least 8 GB for generation, so it is over budget. Puppeteer (Apache-2.0) gives no VRAM figure; FBX export needs bpy 4.2.0. RigNet is GPL-3.0 and its last dated update is 2023.
- **Motion.** MoMask (MIT, weights, BVH built in). MDM (MIT, SMPL mesh render path, GPU). T2M-GPT (Apache-2.0, trained on a V100-32G). MotionGPT (MIT, T5-770M backbone). Info: AMASS is licensed for non-commercial use only, and HumanML3D is derived from AMASS. The SMPL homepage routes commercial licensing to a separate contact.
- **Materials and upscalers.** Material Palette (MIT code, OpenRAIL-M checkpoint, image to SVBRDF; the base SD 1.5 checkpoint is gone from Hugging Face). TEXGen (text to UV albedo; 24 GB for testing; no license stated) does not fit. StableMaterials (Hugging Face, openrail tag; VRAM UNVERIFIED). MatFuse: no repo found. Real-ESRGAN (BSD-3; portable ncnn-vulkan Windows binary; tile option) for texture upscaling.
- **LLM agents and layout.** Holodeck (Apache-2.0) needs the gpt-4o API, so it is not offline. LayoutGPT (MIT) examples use GPT-4 through the OpenAI API; a Llama-2 option is noted in its changelog. 3D-GPT (Chuny1/3DGPT) releases an agent that calls the OpenAI API and scripts Infinigen; its README marks the rest as coming soon. SceneCraft (OrangeSodahub) is a NeRF layout-to-scene method needing two or more GPUs, not an LLM script agent. Unity ML-Agents (Apache-2.0, Release 23) covers NPC and build-test bots. MIMIC-Py (FSE 2026) and TITAN (arXiv 2509.22170): no code link found, UNVERIFIED.
- **Runtimes.** candle (Apache-2.0 or MIT; CUDA; GGUF quantized llama; Whisper and SD examples). burn (MIT or Apache; CUDA via CubeCL; ONNX import limited; LibTorch removed in 0.22.0). ort (Apache-2.0 or MIT; ONNX Runtime wrapper; execution providers and Windows CUDA UNVERIFIED). whisper.cpp (MIT; CUDA or Vulkan; memory from 273 MB tiny to 3.9 GB large; only if we add voice).

**Runtime pick.** llama-cpp-rs as a child process for the LLM door. candle only if a pure-Rust path matters. ort only for ONNX exports. Keep the PyTorch research models (SF3D, TripoSR, MagicArticulate, MoMask) in the editor job as Python child processes, since none of the verified READMEs ships ONNX. VRAM rule: editor-job models never run while the game is in play on the 6 GB card, and the job checks free VRAM before starting.

## Verb gap

Verbs other engines' MCP servers have that a fighting-game door should have. Check each against our current verb table.

- test.run, test.list, test.log (mcp-unreal run_tests, list_tests, get_test_log; Coplay RunTests and GetTestJob)
- view.capture and view.isolated (mcp-unreal capture_viewport; Godot-MCP screenshot-isolated; blender look)
- play.start, play.stop, play.restart (mcp-unreal pie_control; satelliteoflove godot_editor_edit)
- play.step and play.step_until(condition) (satelliteoflove godot_game_time)
- input.inject with named actions and held frames (satelliteoflove godot_input)
- state.digest and state.watch windows (satelliteoflove godot_runtime_state)
- perf.sample and perf.spikes (satelliteoflove godot_profiler)
- log.read (mcp-unreal get_output_log; Coplay ReadConsole)
- batch (Coplay batch_execute)
- undo, redo, history (hybridindie undo on mutating ops)
- toolset.enable by group (hybridindie 29 categories; Godot Toolkit 28 groups with 34 always-on)
- docs.lookup and class.describe (mcp-unreal lookup_docs, lookup_class), for weak models
- script.validate and rules.compile (Godot-MCP script-validate; mcp-unreal live_compile). Our Rhai parse check fits here.
- build.run and package.cook, headless (mcp-unreal build_project, cook_project; Coplay ManageBuild)
- prop.get and prop.set with undo (mcp-unreal get_property, set_property)
- asset.search and asset.import with a license field (blender-mcp search_assets, import_asset)
- engine.status (blender-mcp get_addon_status: versions, GPU, model presence)
- job.status and job.cancel for async asset jobs. No listed server had one; UNVERIFIED that none exist.
- reflect.find and reflect.call (Godot-MCP reflection), only behind a gated allowlist.

Gate hardest: the arbitrary-code verbs (godot_exec, execute_blender_code, Coplay ExecuteCode, mcp-unreal execute_script). Unity's in-editor MCP is deprecated in favor of the Unity CLI (`unity mcp`); the CLI's `unity eval` drives the editor from shell agents. We should not copy that shell path; our door stays MCP only.

## Other engines, verified

- Godot: no official Godot MCP server found; all listed options are community (hybridindie, satelliteoflove, IvanMurzak, NPGameDev). IvanMurzak's default backend is hosted (ai-game.dev), so use its verbs only in a self-hosted setup. NPGameDev's toolkit (https://github.com/NPGameDev/godot-mcp-toolkit) runs locally with 112 tools in 28 groups, 34 always-on.
- Unity: CoplayDev's server has 50 tools in 10 groups and 25 resources (https://coplaydev.github.io/unity-mcp/). Verified C# tool classes: BatchExecute, ManageScene, ManageComponents, ManageBuild, ManageEditor, ReadConsole, RunTests, GetTestJob, UnityReflect. MCP tool names are UNVERIFIED (likely snake_case). Unity's own in-editor server is deprecated (https://docs.unity.com/en-us/unity-cli/replace-mcp-server-unity-cli).
- Blender: 9 tools in total (execute_blender_code, look, get_scene_info, generate_3d, search_assets, import_asset, get_addon_status, disable_telemetry, record_trajectory_feedback).

## Unverified and caveats

- Coplay MCP tool names, Hunyuan3D-2mini VRAM, Light-T2M VRAM, Material Anything VRAM, Puppeteer VRAM, Material Palette VRAM, StableMaterials VRAM and repo text.
- MatFuse repo, MIMIC-Py code, TITAN code.
- ort execution providers and Windows CUDA; candle Windows CUDA depth.
- All packet-size estimates.
- The separate ahujasid/mcp-for-blender repo (search only).
- HumanML3D's own data terms (the repo shows MIT but does not state the dataset terms).
- hybridindie tool count: the fetched page says 193; a search snippet said 175.
