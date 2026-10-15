# arXiv neural research for engine2040 (2026-10-11)

Method: web search, then every arXiv id below opened at arxiv.org/abs/<id> this session. "(search)" marks a link taken from search results, not the abs page. VRAM and packet sizes are estimates unless stated. UNVERIFIED = not confirmed. Owner-named items older than 2021 (LPIPS, DISTS, FLIP) are marked OUT OF WINDOW.

## Top 10 to build

| # | Paper (id, year) | Code | Implementable core | 3050 6 GB cost | Improves | Packet |
|---|---|---|---|---|---|---|
| 1 | MoMask: Generative Masked Modeling of 3D Human Motions (2312.00063, 2023) | github.com/EricGuo5513/momask-codes (search) | Offline: masked motion tokens, temporal inpainting fills in-betweens; bake clips to engine format | Offline; repo says WebUI runs without GPU; training GPU UNVERIFIED | Animation | 2 (bake script ~120 lines, clip importer ~180) + retarget |
| 2 | Human Motion Diffusion Model (2209.14916, 2022) | github.com/GuyTevet/motion-diffusion-model (search) | Transformer predicts clean motion with foot-contact loss; sample offline to fill gaps | Offline; memory UNVERIFIED | Animation (fallback to 1) | 2-3 |
| 3 | Motion In-Betweening with Phase Manifolds (2308.12751, 2023, ACM SCA) | none found | Periodic-autoencoder phase channels drive interpolation between keyposes; builds on DeepPhase (no arXiv id found) | CPU-scale estimate | Animation | 3 |
| 4 | Neural Texture Block Compression (2407.09543, 2024, AMD) | none found | Per-material tiny MLP learns to emit standard BC1/BC4 blocks; shaders unchanged; "up to ~70%" storage cut (v2 abstract) | Offline training; zero extra runtime; BC1/BC4 only per third-party review | Look (VRAM budget) | 2-3 |
| 5 | The Unreasonable Effectiveness of Deep Features as a Perceptual Metric, LPIPS (1801.03924, 2018) | GitHub PerceptualSimilarity (linked on abs page); pypi lpips | Full-reference distance from pretrained AlexNet/VGG features with learned per-channel weights. Pin weight file and torch version | AlexNet scale; well under 1 GB at 1080p (estimate) | Judge: capture vs reference | 1 harness (~120 lines) + 1 CI gate script. OUT OF WINDOW |
| 6 | MUSIQ: Multi-scale Image Quality Transformer (2108.05997, 2021) | Google repo; pyiqa port (search) | No-reference quality on native-resolution images, multi-scale patches; ~27M params per paper summary | Under 1 GB (estimate) | Judge: no-ref quality (not match-to-reference) | 1 (~80 lines) |
| 7 | Q-Align: Teaching LMMs for Visual Scoring via Discrete Text-Defined Levels (2312.17090, 2023; ICML 2024) | github.com/Q-Future/Q-Align (abs page) | Score from probabilities over five text levels (method per paper; determinism is my inference, not stated) | Base LMM size not stated on abs page (UNVERIFIED); needs 4-bit for 6 GB; offline batch only | Judge: VLM quality/aesthetics | 2 (wrapper + calibration) |
| 8 | Cutscene Agent: LLM Agent Framework for Automated 3D Cutscene Generation (2604.25318, 2026) | none found | MCP tool set (asset query, character management, camera templates, scene perception); LLM writes edits as tool calls. Engine already has mcp.rs and scene_tools.rs | No local GPU; LLM API cost | AI door: authoring | 1 per tool (~60 lines each) |
| 9 | LatticeWorld (2509.05263, 2025) | none found | LLM emits a symbolic layout matrix; a deterministic renderer builds the level (UE5 in paper). Engine core = grid schema + validator + loader | LLM 7B class (LLaMA-2-7B per abstract), offline; no runtime GPU | AI door: level layout | 2 (schema+validator ~150 lines; loader ~150) |
| 10 | LLMs May Not Be Human-Level Players, But They Can Be Testers: Measuring Game Difficulty with LLM Agents (2410.02829, 2024) | none found | Agents play headless sim; compare their win/turn stats to designer difficulty labels. Abstract reports a significant correlation with human-rated difficulty | API calls; no local GPU | Balance / playtest | 3 (headless sim entry, agent loop, report) |

## Your six areas, short

1. Judge. Full-reference (captures vs references): FLIP, LPIPS, DISTS. FLIP (HPG 2020, DOI 10.1145/3406183, no arXiv id; code github.com/NVlabs/flip) has no weights and is deterministic, so it is the best CI candidate. LPIPS (1801.03924, 2018) and DISTS (2004.07728, 2020; code github.com/dingkeyan93/DISTS) are learned and deterministic only with pinned weights and library version. DISTS tolerates texture substitution better. All three are OUT OF WINDOW. Stable CI gate: FLIP mean plus LPIPS, thresholded with margin on a fixed set of reference pairs. No-reference (MUSIQ, Q-Align, CLIP-IQA 2207.12396 with github.com/IceClear/CLIP-IQA) cannot say "matches reference"; use them for blur and noise checks. Q-Bench (2309.14181) finds MLLM low-level skills preliminary and unstable (abstract summary), so calibrate any VLM judge before trusting it.
2. Neural rendering. Only NTBC (item 4) fits a raster engine with no shader change. NVIDIA neural materials (2305.02678) run inline in ray-tracing shaders; NTC (2305.17105) needs per-sample network inference in shaders, with RTX 30 support UNVERIFIED. Radiance caching (2106.12372) needs a path tracer. No open-weight arXiv upscaler or game denoiser for raster was found; the denoisers found target volume or path-traced GI.
3. Gaussian splatting (2308.04079; code github.com/graphdeco-inria/gaussian-splatting (search); arXiv comments link repo-sam.inria.fr): training is CUDA, and rendering needs a tile sort and rasterizer, a packet too big for weak models. Revisit as a background-only spike.
4. Animation. Items 1-3 bake offline to clips, so runtime stays deterministic. PHC, ASE, CALM and MaskedMimic need a physics sim and per-frame policy inference, so defer them.
5. Agents. Items 8-10 fit the engine's MCP and scene tools. Playtest papers found are for MMORPG and match-3 games, so only the pattern transfers.
6. World models. None is usable now. GameNGen (2408.14837) runs DOOM only at 20 FPS on a single TPU. DIAMOND (2405.12399) trains per game. Genie (2402.15391, 11B params, no code) is a research model. Genie 2 and 3 have no arXiv paper found; Genie 3 figures (24 fps, 720p) are company claims. Oasis has press coverage only.

## Skip for now

- 3D Gaussian Splatting (2308.04079): see area 3.
- Real-Time Neural Appearance Models (2305.02678, 2023): needs ray-tracing shader inference.
- Real-time Neural Radiance Caching (2106.12372, 2021): needs path tracer and CUDA networks.
- Random-Access Neural Compression of Material Textures (2305.17105, 2023): see area 2.
- Fast Local Neural Regression (2410.11625, 2024): 1-spp path-traced GI denoiser; no GI in engine.
- GameNGen (2408.14837, 2024): DOOM only.
- DIAMOND (2405.12399, 2024; code via diamond-wm.github.io): per-game training.
- PHC (2305.06456, 2023), ASE (2205.01906, 2022), CALM (2305.02195, 2023), MaskedMimic (2409.14393, 2024): physics sim required; no code link on abs pages.
- DeepPhase: no arXiv id found (ACM TOG 2022); use 2308.12751.
- Learned Motion Matching (SIGGRAPH 2020): no arXiv id found; OUT OF WINDOW.
- CLIP-IQA (2207.12396, 2022): second opinion only; MUSIQ covers the role.
- Q-Instruct (2311.06783, 2023): dataset; weights not confirmed.
- Q-Bench (2309.14181, 2023): use to calibrate a VLM judge, not as a build item.
- TITAN, "Leveraging LLM Agents for Automated Video Game Testing" (2509.22170, 2025; no code): MMORPG; pattern only.
- Towards LLM-Based Automatic Playtest (2507.09490, 2025; no code): match-3 mobile; pattern only.
- GBQA (2604.02648, 2026; no code): benchmark; useful later for picking a QA model.
- AutoUE (2603.07106, 2026; no code): full-game Unreal generation; too broad, overlaps item 8.
- Genie (2402.15391, 2024): see area 6.
- Oasis (Decart/Etched, 2024): no paper found.
