# AI-native game tools 2024-2026 for the engine2040 AI pipeline

Research date: 2026-10-24. Scope: text/image-to-3D, rigging, motion, audio, textures, world models, Gaussian splats, MCP servers for engines, LLM NPCs and agents, playtest bots, level and quest generation.

## How to read this

- Every entry below was checked by fetching its repo or model page (WebFetch) unless the entry says otherwise. Facts in "Output quality" are the authors' own claims unless marked "no independent test found".
- "Local fit" is measured against the RTX 3050 6 GB target. "Not local" means the stated minimum VRAM is above 6 GB. Heavy models are still useful as offline cook steps on a bigger GPU or on GitHub compute, which matches how heavy work is run in this repo.
- Licenses are the ones shown on the repo or model page. "Not named on page" means the page links a LICENSE file but its text was not shown. Check before shipping any output.
- Dates are the last update shown on the page. Pages often show no date; that is noted.

### donors-seen and plugin-doc check

Skipped because they are already in `docs/research/hunt-2026-10-21/donors-seen.txt` or in the 01-03 plugin docs of this folder:

- MCP: ahujasid/blender-mcp (and mcp-for-blender), CoplayDev/unity-mcp, Coding-Solo/godot-mcp and its forks (tomyud1, hybridindie, tugcantopaloglu), chongdashu/unreal-mcp, ChadAragorn and Fulviuus defold-mcp, MalloyTheDev/aseprite-mcp, Veradictus and maximkone flax-mcp, bevy MCP repos (natepiano, Nub, noeticbasho), bttf/ogremcp, dcc-mcp.
- 3D and motion: TripoSR, stable-fast-3d, microsoft/TRELLIS (v1), tencent/Hunyuan3D-2, VAST-AI/TripoSG, wgsxm/PartCrafter, nv-tlabs and NVlabs MotionBricks/Kimodo/SOMA-X, EricGuo5513 (MoMask), zju3dv (GVHMR), GuyTevet (MDM), Seed3D org entry (MagicArticulate), Colin97/OpenShape, allenai Holodeck and objaverse-xl.
- Audio, agents, eval: facebookresearch/audiocraft, suno/bark, DanielSWolf rhubarb, joonspk generative_agents, AgenticSTS, CharTyr STS2-Agent, alexzhang13 videogamebench, waynchi gamedevbench, roger-creus agentick, SimWorld-AI, ChatPCG PCGRLLM, Mawiszus TOAD-GAN, FreedomIntelligence gamecraft-bench, mem0, promptfoo, inspect_ai, openai/evals, Farama gym and PettingZoo, google-deepmind mujoco.
- World/splat: Decart Oasis, magikerwin burn-nerf-guided-splats, teamchong/splat, gschup ggrs, playcanvas org.
- Physics and plugin docs: Jolt, Box2D, Box3D, Rapier, Blast (not seen in donors but in plugin doc 02), Sverchok and Animation Nodes (plugin doc 01), Convai, NobodyWho, Behavior Designer, LimboAI, Beehave, MapMagic, Feel, Juicee, Final IK, PuppetMaster, FishNet, Mirror, Unity Sentis, Odin (plugin doc 03).

Two entries use an org name that appears in donors-seen as a bare org line (Seed3D, NVlabs). They are kept because the repo itself is not listed. Check that the earlier hunt did not already read them: Puppeteer (entry 10) and GEM-X (entry 12).

---

## A. Text and image to 3D mesh

### 1. Hi3DGen (ByteDance)
- Link: https://github.com/ByteDance/Hi3DGen
- License: MIT (per page). Free.
- What it does: image to high-fidelity geometry. Uses normal maps as an intermediate step.
- Output quality: author claims; geometry only on the page; no independent test found.
- Local fit: VRAM not stated.
- Steal idea: split geometry from texture in our cook, so either stage can be swapped.

### 2. Step1X-3D (StepFun)
- Link: https://github.com/stepfun-ai/Step1X-3D
- License: Apache-2.0. Free.
- What it does: two stages. Geometry (1.3B) then texture synthesis (3.5B), outputs a textured GLB. Released with training code and a 800k-identifier dataset.
- Output quality: no independent test found. Page does not mention PBR.
- Local fit: not local. Page lists about 27 to 29 GB VRAM. Latest update June 2025.
- Steal idea: adopt the Apache-licensed two-stage layout as a reference for an offline asset cook.

### 3. TRELLIS.2 (Microsoft)
- Link: https://github.com/microsoft/TRELLIS.2
- License: MIT for model and code; some dependencies (nvdiffrast, nvdiffrec) have their own terms. Free.
- What it does: 4B image-to-3D. O-Voxel sparse voxel representation. Outputs a PBR-ready GLB plus an MP4 preview.
- Output quality: page describes PBR GLB output. Timings on H100: about 3 s at 512 cubed, 17 s at 1024 cubed, 60 s at 1536 cubed.
- Local fit: not local. Page says an NVIDIA GPU with at least 24 GB is necessary. Tested on Linux only.
- Steal idea: run as a GitHub or cloud batch job that writes PBR GLBs, then let our Blender headless step only decimate and make LODs.

### 4. SPAR3D (Stability AI)
- Link: https://github.com/Stability-AI/stable-point-aware-3d
- License: LICENSE.md linked, name not shown on page. Gated Hugging Face access (accept terms, token, huggingface-cli login).
- What it does: single image to GLB, with an editable point cloud for conditioning. Built on Stable Fast 3D.
- Output: GLB. Texture resolution is configurable. Remeshing option: None, Triangle, or Quad.
- Output quality: no independent benchmark found.
- Local fit: borderline. Page says 10.5 GB by default, about 7 GB with SPAR3D_LOW_VRAM=1 (slower). One page note says about 6 GB for a single image; not confirmed.
- Steal idea: use the built-in Quad remesh as the first step of our hard-surface cook, then decimate to a face budget.

### 5. MeshAnythingV2 (buaacyw)
- Link: https://github.com/buaacyw/MeshAnythingV2
- License: not named on page (LICENSE.txt linked). Free to read.
- What it does: artist-style mesh generation from a dense mesh or point cloud. Trained on meshes with fewer than 1600 faces, so output is capped near that.
- Input: mesh or point cloud (.npy, N by 6 with normals). Meshes not made by marching cubes need the --mc flag. The authors suggest a dense input from Rodin or a similar generator.
- Output quality: artist-style topology is the claim. No independent test found.
- Local fit: about 8 GB VRAM, about 45 s per mesh on an A6000. Not on a 6 GB card.
- Steal idea: a low-face artist-style pass for hero props. Pair it with our face budget per tier.

### 6. SAM 3D Objects (Meta)
- Link: https://github.com/facebookresearch/sam-3d-objects
- License: "SAM License" (custom). Read the LICENSE file before commercial use. Gated Hugging Face weights.
- What it does: a segmented object in an image becomes 3D with pose, shape, texture and layout.
- Output: the example exports a Gaussian splat PLY. Mesh export is not explicit on the page.
- Output quality: no independent test found. Latest update 2026-06-02 (per page).
- Local fit: VRAM not stated.
- Steal idea: use the layout output to place a set from a reference photo. Not a mesh source for us unless mesh export is verified.

### 7. LLaMA-Mesh (NVIDIA and Tsinghua)
- Link: https://github.com/nv-tlabs/LLaMA-Mesh (weights: https://huggingface.co/Zhengyi/LLaMA-Mesh)
- License: LICENSE and LICENSE_LLAMA are in the repo; text not shown on page. Model card requires "Built with Llama" attribution.
- What it does: writes meshes as plain-text vertex and face lines inside an LLM. Interleaves text and mesh.
- Output quality: paper claims parity with models trained from scratch. The repo TODO still lists "Release inference code" unchecked on the fetched page.
- Local fit: base LLM size not stated on page.
- Steal idea: the text-mesh format means an agent can write a small prop as OBJ text and send it through our validator. No training needed.

---

## B. Auto-rigging and skinning

### 8. UniRig (VAST-AI-Research)
- Link: https://github.com/VAST-AI-Research/UniRig
- License: MIT. Free.
- What it does: skeleton and skinning prediction for any mesh. Input .obj, .fbx, .glb, .vrm. Skeleton and skinning results are exported as FBX, and a merge script writes a GLB.
- Status: skeleton and skinning checkpoint listed as available. Rig-XL and VRoid checkpoints, bone attributes listed as coming soon.
- Output quality: paper claims about 215% better rigging accuracy than prior work on hard data. Author claim.
- Local fit: 8 GB VRAM minimum. Not on 6 GB.
- Steal idea: run as a cook step on every humanoid and prop kit. Write skeleton and weights into our rig format and check bind pose automatically.

### 9. SkinTokens (VAST-AI-Research)
- Link: https://github.com/VAST-AI-Research/SkinTokens
- License: MIT. Free.
- What it does: one model generates a full skeleton hierarchy and skin weights from a single 3D mesh (TokenRig). Examples write GLB. Checkpoints on Hugging Face.
- Output quality: no independent test found.
- Local fit: 14 GB VRAM minimum. Not local.
- Steal idea: one call instead of a two-stage rig pipeline for bespoke props. Runs as cloud batch.

### 10. Puppeteer (Seed3D org)
- Link: https://github.com/Seed3D/Puppeteer
- License: Apache-2.0. Free. NeurIPS 2025 spotlight.
- What it does: predicts skeleton and skinning weights, then animates the rigged mesh from video guidance using a differentiable optimisation. FBX export added (export.py, Sep 2025).
- Status: inference code and checkpoints released (per Sep 2025 news on page). Articulation-XL2.0 dataset posted.
- Output quality: paper and authors' claims. No independent test found.
- Local fit: VRAM not stated. Setup targets PyTorch 2.1.1 with CUDA 11.8.
- Steal idea: video-guided animation of a rigged prop gives motion without hand keyframes. Keep it offline and bake the result.

---

## C. Motion (text-to-motion and video-to-motion)

### 11. HY-Motion 1.0 (Tencent Hunyuan)
- Link: https://github.com/Tencent-Hunyuan/HY-Motion-1.0 (weights on Hugging Face as tencent/HY-Motion-1.0)
- License: custom "Tencent HY-Motion 1.0 community license" (from secondary coverage). License file not shown on fetched page. Read it before use, since commercial terms apply.
- What it does: text to skeleton-based 3D character animation. Flow-matching diffusion transformer, 1.0B params, plus a Lite 0.46B model. Paper reports 200+ motion categories.
- Output format: not named on the page (no BVH, FBX, or SMPL export stated).
- Output quality: paper claims; no independent test found.
- Local fit: not local. Page says 26 GB minimum for 1.0 and 24 GB for Lite. Page suggests one seed, prompts under 30 words and motions under 5 s to cut VRAM.
- Steal idea: offline batch to build a short clip library per verb (attack, parry, stagger) on GitHub compute, then bake into our animation rows.

### 12. GEM-X (NVIDIA)
- Link: https://github.com/NVlabs/GEM-X
- License: Apache-2.0 for most code. Some files use LicenseRef-OpenMDW-1.1. Model weights and assets follow their own terms.
- What it does: video to motion on the SOMA body model. Outputs .pt predictions. Body Motion needs 77-point 2D keypoints. Hand Pose added 2026-10-21.
- Output format: BVH or FBX export not stated on page.
- Output quality: no independent test found.
- Local fit: CUDA supported, CPU and ARM64 documented. VRAM not stated.
- Steal idea: video to SOMA motion for reference clips and a hand pass for weapon grips. Check the per-file license before using the weights.

### 13. WHAM (yohanshin)
- Link: https://github.com/yohanshin/WHAM
- License: MIT per the fetched page. Note: the search snippet said research-only. Re-check the LICENSE file before any use.
- What it does: world-grounded human motion from monocular video, with global trajectory. Uses ViTPose and DPVO or DROID-SLAM. CVPR 2024.
- Requirement: SMPL and SMPLify registration to download body models.
- Output quality: paper claims; no independent test found.
- Local fit: VRAM not stated.
- Steal idea: root-motion reference from a phone video of a fight, then retarget to our rig.

---

## D. Audio and sound effects

### 14. MMAudio (hkchengrex)
- Link: https://github.com/hkchengrex/MMAudio
- License: code MIT. Checkpoints CC-BY-NC 4.0; README says pre-trained models are not guaranteed suitable for commercial use. Non-commercial for shipping.
- What it does: video (plus optional text prompt) to synced foley and sound effects. CVPR 2025.
- Output: .flac audio and .mp4 video in ./output. Default clip length 8 s. Sample rate not stated.
- Output quality: authors claim sync; no independent test found.
- Local fit: about 6 GB GPU in 16-bit mode. Borderline on 3050.
- Steal idea: prototype impact sounds offline. Do not ship its output while the weights are non-commercial.

### 15. HunyuanVideo-Foley (Tencent)
- Link: https://github.com/Tencent-Hunyuan/HunyuanVideo-Foley
- License: not shown on fetched page (the LICENSE link returned no text). Check before commercial use.
- What it does: video to 48 kHz sound effects, with an optional text prompt. Batch CSV mode and a Gradio UI.
- Sizes: XXL needs about 20 GB (12 GB with offload). XL needs about 16 GB (8 GB with offload). Updated 2025-09-29.
- Output quality: no independent test found.
- Local fit: not local at full size. XL with offload is at the 6 GB edge.
- Steal idea: 48 kHz foley for cutscene clips, offline only.

### 16. Stable Audio Open Small (Stability AI)
- Link: https://huggingface.co/stabilityai/stable-audio-open-small
- License: Stability AI Community License. Gated (accept conditions). Commercial use needs the terms at stability.ai/license; the page does not state a revenue threshold.
- What it does: text to short sound effects, drum loops, foley and ambience. Up to 11 s, 44.1 kHz stereo.
- Output quality: no independent test found.
- Local fit: yes. Sample code uses CUDA when present and falls back to CPU. Arm CPU guide linked.
- Steal idea: text-to-SFX batch in the audio cook. Prompts live in a .rows file, output wav, loudness normalised. Check the license threshold before shipping.

### 17. Tango 2 (declare-lab)
- Link: https://github.com/declare-lab/Tango (Tango 2 checkpoint: https://huggingface.co/declare-lab/tango2)
- License: not shown on fetched page (LICENSE.md linked, text not shown).
- What it does: text to audio, sound effects. Tango 2 is aligned with DPO on audio-alpaca. Writes WAV at 16 kHz.
- Output quality: no independent test found in this pass.
- Local fit: VRAM not stated. Page notes a CUDA-specific PyTorch build may be needed.
- Steal idea: fallback text-to-SFX when the MMAudio license blocks shipping. Verify the license first.

### 18. ACE-Step 1.5 (ace-step)
- Link: https://github.com/ace-step/ACE-Step-1.5
- License: MIT.
- What it does: text to music, 10 s to 10 min. Not an SFX tool. XL 4B DiT release on 2026-04-02.
- Local fit: abstract claims under 4 GB VRAM. The 0.6B language model is listed for 6 to 8 GB GPUs. XL needs 12 GB or more with offload.
- Output quality: vendor headline says it rivals commercial music. No independent test found.
- Steal idea: zone music beds generated at build time and looped. Use other tools for SFX.

---

## E. Textures and PBR materials

### 19. Marigold (prs-eth)
- Link: https://github.com/prs-eth/Marigold
- License: code Apache-2.0. Model weights under RAIL++-M, which has use restrictions. Read before shipping weights.
- What it does: image to depth, surface normals, intrinsic decomposition. The appearance model outputs albedo, roughness and metallicity.
- Output quality: no independent test found in this pass.
- Local fit: page says tested on an RTX 3090 with CUDA 11.7. No VRAM figure stated.
- Latest: v1.1 checkpoints 2025-05-15.
- Steal idea: photo to reference PBR maps for kit materials. Use as a reference or an artist starting point.

### 20. ComfyUI-TextureAlchemy (amtarr)
- Link: https://github.com/amtarr/ComfyUI-TextureAlchemy
- License: not shown on fetched page.
- What it does: a ComfyUI node pack that builds a PBR set from an image. Maps: albedo, normal, roughness, metallic, AO, height, curvature, emission, transparency. Uses Marigold and Lotus.
- Local fit: ComfyUI only. No VRAM listed. Depends on Marigold and Lotus.
- Output quality: no examples checked.
- Steal idea: a saved ComfyUI graph that gives a PBR set from a seed image, called headless from our script. Verify the license first.

### 21. dream-textures (carson-katri)
- Link: https://github.com/carson-katri/dream-textures
- License: GPL-3.0. Read the code, do not vendor it into a shipped build.
- What it does: local Stable Diffusion inside Blender for textures and concept art. Seamless tiling option, and depth-to-image projection onto scenes.
- Local fit: over 4 GB VRAM recommended. Fits 6 GB.
- Blender version range and latest release date: not shown on page.
- Steal idea: a seamless tile recipe for ground and wall materials, run headless.

---

## F. World models and neural game engines

### 22. DIAMOND (eloialonso)
- Link: https://github.com/eloialonso/diamond
- License: MIT.
- What it does: a diffusion world model that plays Atari (with an RL agent) and a CS:GO world model on the csgo branch. NeurIPS 2024 spotlight. Atari 100k mean human normalised score 1.46 (paper).
- Playback: 15 FPS default (--fps). Training needs CUDA. Apple Silicon needs a fallback flag for CS:GO.
- Output quality: research demo. A third-party article reported about 10 FPS on an RTX 3090 for the CS:GO model.
- Local fit: GPU memory not stated. Atari models are small; not verified on 3050.
- Steal idea: use the dreamed environment as a playtest substrate for bots, not for shipping. Research only.

### 23. Matrix-Game (SkyworkAI)
- Link: https://github.com/SkyworkAI/Matrix-Game
- License: Apache-2.0 for version 4.0 (per fetched page). Root repo MIT.
- What it does: interactive world model. Input: low-poly RGB frames plus text prompt, with the game handling controls. Output 864 by 480.
- Speed (per fetched page): steady-state 67 FPS on RTX 5080 and 58 FPS on RTX 5070 Ti. Page dated 2026-10-16.
- Conflict: an earlier search gave Matrix-Game 1.0 as 17B parameters, trained on Minecraft. The fetched page did not give parameter counts. Confirm before quoting.
- Local fit: not local for 3050 at these speeds.
- Steal idea: its input format (low-poly frames and action text) is close to a bot observation format. Use it as an offline "does this look right" oracle only.

### 24. Hunyuan-GameCraft-1.0 (Tencent)
- Link: https://github.com/Tencent-Hunyuan/Hunyuan-GameCraft-1.0
- License: not stated on fetched page. Secondary coverage says Tencent Hunyuan community license. Check before use.
- What it does: reference image plus keyboard and mouse actions to playable video, first or third person. Built on HunyuanVideo. Arxiv 2506.17201.
- Local fit: not local. Page says 24 GB minimum, 80 GB recommended. Example 704 by 1216 at 25 FPS.
- Output quality: demo claims; no independent test found.
- Steal idea: an action-conditioned frame generator as a reference for a frame-based test oracle. Cloud only.

---

## G. Gaussian splats

### 25. godot-gaussian-splatting / GDGS (ReconWorldLab)
- Link: https://github.com/ReconWorldLab/godot-gaussian-splatting
- License: MIT.
- What it does: imports .ply, .compressed.ply, .splat, .sog (v2) and renders splats in Godot 4.3+ with depth occlusion against normal 3D content.
- Backends: Compute needs Forward+ and compute shaders (desktop only). Raster is beta, runs on Mobile and Compatibility.
- Known limits: splats get no shadows, baked light cannot be removed by relighting, 4K can hit GPU memory.
- Latest: v3.3.0, 2026-07-30 (per page).
- Local fit: desktop GPU for Compute. Raster is the potato path.
- Steal idea: Raster splat path for scanned hero props on low-end tiers, with a baked blob shadow to cover the missing shadows.

### 26. UnityGaussianSplatting (aras-p)
- Link: https://github.com/aras-p/UnityGaussianSplatting
- License: code MIT. The original paper's training software is non-commercial, so check how the splat files were made.
- What it does: real-time splat viewer for PLY and SPZ in Unity. Editing docs are linked.
- Platforms: DX12 or Vulkan on Windows (DX11 does not work), Metal on Mac, Vulkan on Linux.
- Status: last status note Dec 2023, says no major development planned.
- Local fit: any desktop GPU that runs DX12 or Vulkan.
- Steal idea: read the splat sort and tile-raster code as a reference for our own splat pass. Read only.

### 27. gsplat (nerfstudio-project)
- Link: https://github.com/nerfstudio-project/gsplat
- License: Apache-2.0.
- What it does: CUDA-accelerated Gaussian rasterisation library with PyTorch bindings. Not a mesher.
- Versions: v1.5.3 latest tag (news dated May 2025). v1.6.0 on main. Needs CUDA 12.8 or 13 and PyTorch 2.7 or newer.
- Local fit: CUDA GPU. Training runs offline.
- Steal idea: train splats offline from a turntable video of a prop, export PLY, and feed GDGS.

---

## H. MCP servers for engines and their verb sets

### 28. Unity-MCP (IvanMurzak)
- Link: https://github.com/IvanMurzak/Unity-MCP
- License: Apache-2.0.
- What it does: MCP server plus Unity plugin. Page says 70+ built-in tools in four to five groups. Named examples: scene-open, scene-save; gameobject-create, gameobject-modify; assets-find, assets-move; script-update-or-create, script-execute; editor-application-set-state (play mode); screenshot-game-view, screenshot-scene-view; tests-run (EditMode and PlayMode).
- Limits: no Unity version listed. Project path must not contain spaces. Date not shown.
- Local fit: no GPU needed. Needs Unity.
- Steal idea: copy the noun-verb naming and the pair of screenshot plus tests-run as the verbs our agents need for a fast self-check loop.

### 29. Godot-MCP (IvanMurzak)
- Link: https://github.com/IvanMurzak/Godot-MCP
- License: Apache-2.0.
- What it does: C# editor add-on for Godot 4.3 or newer (mono build). 23 built-in tools in 9 families: ping, rql, script, screenshot, console, reflection, skills, session, runtime-errors. Examples: rql-query, rql-mutate, script-create, screenshot-viewport.
- Gap: no run-project tool on the fetched page. Date not shown.
- Local fit: no GPU needed.
- Steal idea: the runtime-errors family. Let an agent read errors after a play run in one call.

### 30. unreal-engine-mcp (FFZackFair92)
- Link: https://github.com/FFZackFair92/unreal-engine-mcp
- License: MIT.
- What it does: 162 tools and 5 resources for UE 5.0 and newer (tested on 5.8). Covers Blueprint creation, actor spawn, move, delete, asset import (.glb, .gltf, .fbx, .wav), Play In Editor, and ue_screenshot.
- Key point: no C++ plugin to compile. Uses the built-in Python Editor Script Plugin and Remote Control API.
- Date not shown.
- Steal idea: the no-compile route (in-editor Python plus a local HTTP bridge) is the cheapest way to give any closed engine an MCP verb surface.

### 31. unreal-engine-mcp (flopperam)
- Link: https://github.com/flopperam/unreal-engine-mcp
- License: MIT. Open-source local version has 42 tools. The hosted service has 50+ more (not used here).
- Tools include procedural builders: create_castle_fortress, create_aqueduct, create_maze, create_suspension_bridge, create_tower, create_arch, create_wall, create_staircase, construct_house, create_town. Also Blueprint nodes, compile_blueprint, set_physics_properties, apply_material_to_actor.
- Requires: the bundled UnrealMCP C++ plugin over a TCP socket. UE 5.5 or newer.
- Steal idea: parametric architecture verbs (create_wall, create_arch, create_staircase) as a model for our Roman kit verbs.

### 32. blender-mcp-server (djeada)
- Link: https://github.com/djeada/blender-mcp-server
- License: MIT.
- What it does: MCP server (stdio) plus a Blender add-on on localhost port 9876 with a shared-token handshake. Tool count conflicts: 22 (About) vs 27 (README). Examples: blender_object_create, blender_scene_list_objects, blender_render_still, blender_python_exec.
- Warning: blender_python_exec runs arbitrary Python. The page says its import blocklist is a speed bump, not a sandbox.
- Date not shown.
- Steal idea: a ready verb map for our headless prop_gen. Keep the Python-exec verb off any shared endpoint.

---

## I. LLM NPCs and agent frameworks

### 33. AI Town (a16z-infra)
- Link: https://github.com/a16z-infra/ai-town
- License: MIT.
- What it does: open starter kit for a town of LLM-driven characters with memory and relationships. Stack: Convex (engine and database), PixiJS rendering, Replicate MusicGen for background music.
- LLM backends: Ollama is default (llama3 for chat, mxbai-embed-large for embeddings). Any OpenAI-compatible API also works.
- Limits: no hardware requirements listed. Maps are built in the external Tiled editor. The "v2 walkable map and editor" claim from a search was not found on this repo page.
- Local fit: depends on the Ollama model. Not stated.
- Steal idea: each citizen is a data row (backstory, goals, memory summary). The NPC loop is data-driven, the same idea we want for the AI editor.

### 34. Mantella (art-from-the-machine)
- Link: https://github.com/art-from-the-machine/Mantella
- License: AGPL-3.0. Copying code makes the project copyleft. Read for ideas only.
- What it does: speak to Skyrim and Fallout NPCs. STT: Moonshine or Whisper. TTS: Piper, xVASynth or XTTS. LLM: any. Python 3.11.
- Features (from search summary of release notes): NPC memories of past talks, awareness of in-game events, and an actions framework for modders. The fetched page did not show the actions framework itself.
- Activity: 620 commits. Last push Apr 2026 per a third-party tracker (not on the repo page).
- Steal idea: an NPC action list as a typed registry the LLM may call. The LLM can only pick declared actions, which is the same safety pattern as our verb surface.

### 35. Mindcraft (kolbytn)
- Link: https://github.com/kolbytn/mindcraft
- License: MIT.
- What it does: LLM-driven Minecraft bots built on Mineflayer. About 17 LLM providers, including local models through Ollama.
- Safety gate: code writing for new actions is off unless allow_insecure_coding is set (page).
- Date: not shown.
- Local fit: works with local Ollama models or with API keys.
- Steal idea: the opt-in code gate. Our agents should call declared verbs by default, and only a deliberate flag lets them run new code.

### 36. Word2World (umair-nasir14)
- Link: https://github.com/umair-nasir14/Word2World
- License: Apache-2.0.
- What it does: LLM writes a story and then places tiles to build a playable 2D world. Story and tile placement are both LLM-driven.
- LLM support: OpenAI models only (gpt-4-turbo-2024-04-09 recommended). Anthropic and Groq listed as to-dos. No local model support.
- Local fit: API only.
- Steal idea: story beats first, then a solver checks that the tile layout is reachable, and only then emit level rows.

---

## J. AI playtesting and RL bots

### 37. Godot RL Agents (edbeeching)
- Link: https://github.com/edbeeching/godot_rl_agents
- License: MIT.
- What it does: RL agent training in Godot. Wrappers for Stable-Baselines3, Sample Factory, Ray RLlib and CleanRL. 2D and 3D. Memory-based (LSTM or attention) agents supported.
- Training: can run inside the Godot editor, no exported build needed. Page describes automated gameplay testing through an RL agent.
- Latest release date: not found on page (Releases section empty). Paper arXiv 2112.03636.
- Local fit: small RL nets run on CPU or the 3050.
- Steal idea: a headless training loop that acts as a combat playtest bot. Output win rate and hit counts as a report row for tuning.

### 38. ML-Agents (Unity-Technologies)
- Link: https://github.com/Unity-Technologies/ml-agents
- License: Apache-2.0.
- What it does: Unity environments for RL and imitation learning, Python API for training.
- Status: fetched page shows no release date. A third-party mirror and a Unity forum title list ML-Agents 4.0.0 (Nov 2025) and later 4.0.x in 2026. Not verified on the official releases page. A late-2025 forum thread asked about deprecation (not read).
- Local fit: CPU training is fine for small scenes.
- Steal idea: behaviour cloning from recorded human fight traces to a baseline bot. Port the idea to our Rust bot harness; the Unity package itself is not needed.

---

## K. Level generation

### 39. MarioGPT (shyamsn97)
- Link: https://github.com/shyamsn97/mario-gpt
- License: MIT.
- What it does: text to Super Mario Bros level. Fine-tuned distilgpt2 on Super Mario Bros and The Lost Levels. Novelty search for diverse levels. NeurIPS 2023.
- Outputs: text level strings (.txt), PNG, interactive play and A* agent runs (A* and play need Java).
- Local fit: no hardware stated. Small model, CPU is fine.
- Date: not shown; about 40 commits.
- Note: 2023 work, older than the window, included because it is the clearest open text-to-level reference.
- Steal idea: a small text-to-level model plus novelty search to diversify, with the output converted into our level rows.

---

## Dropped or not verified

- Direct3D-S2 (NJU3DV/Direct3D-S2): repo URL returned 404 on fetch. Weights status unknown. Dropped.
- Sparc3D and PartPacker: search returned no usable repo or license. Dropped.
- RigAnything: no official repo found in search (only downstream packages). Dropped.
- MIMIC-Py (FSE 2026 demo): project page only, no repo link found. Dropped.
- BugCraft (arXiv 2503.20036): repo not checked. Dropped.
- TITAN (arXiv 2509.22170) and GBQA (arXiv 2604.02648): papers only, no code link confirmed. Dropped.
- PixelLab MCP (pixellab-code): hosted paid API with a copyright notice, not open source. Dropped as a dependency; the verb list is useful for reference.
- erodenn/godot-mcp-runtime and mikeumus/godot-mcp-rts: repo not fetched. The RTS fork is of a seen repo.
- Voyager (MineDojo): MIT, but 2023. Outside the window.
- Blender Extensions platform and Superhive paid generators: already in the Blender doc (01).
- Matrix-Game parameter counts and the 1.0 vs 4.0 version history: conflicting sources. Kept with a note.
- Meshy, Rodin, Tripo: closed hosted services. Not picked.

---

## TOP 15 for AI-built games (ranked)

Ranked by how much each would speed or improve what AI agents build in engine2040 (asset cook, rigging, sound, verbs, playtest). Hardware is noted because most heavy models are not local on the 3050.

1. **UniRig (8)**: MIT auto-rig with FBX out and a released skeleton and skinning checkpoint. Biggest saving on hand rigging. Needs 8 GB, so run offline.
2. **Puppeteer (10)**: Apache-2.0 rig plus video-guided animation with FBX export. Gives motion for props without keyframes.
3. **Unity-MCP by IvanMurzak (28)**: 70+ tools with a clear noun-verb scheme, plus screenshot and tests-run. Best reference for our agent verb surface.
4. **MeshAnythingV2 (5)**: artist-style meshes under about 1600 faces. Fits our low-poly face budget. Needs about 8 GB.
5. **Godot RL Agents (37)**: MIT RL inside the editor with headless-capable training. Direct path to automated combat playtests.
6. **SPAR3D (4)**: GLB output with a built-in Quad remesh option. Gated and about 7 GB, so borderline locally.
7. **Stable Audio Open Small (16)**: text-to-SFX that runs on CPU, 44.1 kHz, cheap and local. Check the license threshold.
8. **Marigold (19) with ComfyUI-TextureAlchemy (20)**: photo to reference PBR maps. Watch the RAIL++-M weight license.
9. **SkinTokens (9)**: one-model skeleton and skinning from a mesh, MIT, GLB out. Needs 14 GB, cloud batch.
10. **flopperam Unreal MCP (31)**: procedural architecture verbs (wall, arch, staircase, aqueduct). Good model for Roman kit verbs.
11. **AI Town (33)**: MIT LLM citizens as data rows with memory. Strong fit for NPC design.
12. **blender-mcp-server (32)**: MIT Blender verb map for headless prop_gen. Treat its Python-exec verb as unsafe to expose.
13. **TRELLIS.2 (3)**: MIT PBR GLB generation. Not local (24 GB), but a good cloud batch source of PBR assets.
14. **MMAudio (14)**: video to synced foley. Non-commercial weights, so prototype only.
15. **LLaMA-Mesh (7)**: LLM writes OBJ text, so agents can author small props directly. Code release still unchecked on the page.

Honourable mentions outside the top 15: Godot-MCP by IvanMurzak (29, runtime-errors verb), FFZackFair92 Unreal MCP (30, no-compile route), Mindcraft (35, opt-in code gate), DIAMOND (22, dreamed playtest substrate), GDGS (25, Raster splats for potato tier).

## Notes

- Nothing was installed, run or benchmarked here. "Output quality" is taken from pages and papers unless marked otherwise.
- Licenses: 6 entries have non-commercial or restricted terms for weights or outputs (MMAudio, Marigold weights, HY-Motion, HunyuanVideo-Foley unclear, Hunyuan-GameCraft unclear, Mantella AGPL). Check before any shipped build.
- No file other than this one was touched. No git operations.
