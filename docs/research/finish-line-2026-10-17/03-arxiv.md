# arXiv ideas to steal for engine2040 (hunt of 2026-10-17)

Read first: `arxiv-neural.md`, `anim-feel-steals.md`, `LADDER.md`, cards. Nothing below repeats them (MoMask, NTBC, LPIPS, Cutscene Agent, LatticeWorld, GGRS, Spring-It-On, EASU are already taken).

Method: arXiv search pages + `arxiv.org/abs/<id>` opened for every id (title and date checked). Licenses read from the GitHub API and LICENSE files on 2026-10-17. UNVERIFIED = not confirmed. "Not on arXiv" is marked. VRAM numbers are from READMEs unless marked.

## What the engine already has (so I did not suggest it)

- `anim`: motion_match, inertialize, hit_react, ik (two_bone, fbik), springs, feet, warp_ground.
- `render`: restir_di (an oracle on the CPU side, uses a ray for torch visibility), upscale_easu, GTAO, TAA jitter, baked lightmaps + SH probes, meshlet, gpu_cull, dynres_gov, torch clusters.
- `net`: rollback, delta snapshots, interest management, WebTransport.
- `endpoint`: look judge hook (rubric L1..L9), `insight` module with `asset.refs` and `callers_of`.
- `assets`: native BC1/3/4/5/7 encoder in the cook.
- `destruction`, `combat` (blade vs limb sever), `playtest` (agent, seeded_bug, reachability, brief_eval).
- `proof`: SSIM windows only (no learned metric).

So: no rollback/netcode paper beat what is there (I found none worth a row). "Unity Insight" (2609.27585) is already built as `insight`.

## Top 20, ranked by value / effort

| # | Idea (paper) | Verdict | Where it lands | Effort |
|---|---|---|---|---|
| 1 | Calibrate the look judge on VLM-QA sets (VideoGameQA-Bench 2505.15952, RefGlitch-Bench 2604.11082, TempGlitch 2605.21443) | STEAL NOW | offline tool + door arg | S |
| 2 | CGVQM+D learned video metric for render artifacts (2506.11546) | STEAL NOW | offline gate in `proof` | S-M |
| 3 | Temporal registration audit for TAA (Frozen History 2610.15752) | STEAL NOW | `render` test | S |
| 4 | Frame contact sheet verb for the VLM (Fly, Fail, Fix 2507.12666) | STEAL NOW | door verb | S |
| 5 | Play@k metric + PlayTester (PlayCoder 2604.19742) | STEAL NOW | `playtest/brief_eval` | S |
| 6 | Verifier signal inside every build verb (SimWorld Studio 2605.09423) | STEAL NOW | `endpoint` audit | S |
| 7 | Living debug protocol + 3-axis game bench (OpenGame 2604.18394) | STEAL NOW | `playtest`, skills | S-M |
| 8 | ARDY motion generator with keyframe constraints (2607.08741) | STEAL NOW (spike) | offline bake tool | M |
| 9 | ReSTIR-sampled shadow maps for 250 torches (EG 2025, not arXiv) | STEAL NOW (spike) | `render` | M-L |
| 10 | Neural texture compression family (2606.26913, 2608.27888, RTXNTC) | LATER (measure first) | offline tool + load-time decode | M-L |
| 11 | Diff-targeted playtest (SMART 2512.12706) | LATER | `playtest` | M |
| 12 | Auto-rig (UniRig 2504.12451, Puppeteer 2508.10898) | LATER | offline tool + door verb | M |
| 13 | LLM writes building programs, not meshes (CityGenAgent 2602.05362, ProxyBuild 2609.23386) | LATER | `geo`/`spatial` DSL | M-L |
| 14 | Ready-to-React reaction policy (2502.20370) | LATER | offline + `distill` | M-L |
| 15 | MotionBricks (2604.24833) | LATER | offline | M |
| 16 | Split Radiance Cascades (2607.20384) | LATER | `render` | L |
| 17 | Generated PBR checked by the furnace test (MatLat 2512.17302, StableMaterials 2406.09293) | LATER | door verb | M |
| 18 | Voxel PBD tearing (Gram-Schmidt constraints; OmniBody; not arXiv) | LATER | `physfx` | M-L |
| 19 | WebGPU splat + mesh depth composition (Visionary 2512.08478) | LATER | `render` | L |
| 20 | KG-guided update tests (KLPEG 2511.02534) + BO grid explorer (2508.13121) | LATER | `playtest` | M |

## Details

### 1. Calibrate the look judge (STEAL NOW)
- Papers: VideoGameQA-Bench 2505.15952 (2025); RefGlitch-Bench 2604.11082 (2026); TempGlitch 2605.21443 (2026).
- Idea: VLMs find glitches in single frames up to ~83% (images) and ~78% (video), but they fail on body pose, clipping and common sense. Their false-positive rate is high. Models can't find the onset frame well (best ~36% within 5 s). RefGlitch: give the VLM a clean reference frame; the "last clean frame" rule works best. TempGlitch: temporal glitches are much harder than spatial ones.
- Cost: offline, API or local VLM. No GPU needed from us.
- Code: VideoGameQA data+code CC0-1.0 (asgaardlab); RefGlitch code MIT (github.com/PipiZong/RefGlitch-Bench); TempGlitch no link found.
- Build: (a) `tools/judge-calibrate.mjs` runs the registered look judge on their labeled sets plus our own `playtest/seeded_bug` frames and prints the false-positive rate and recall per glitch type. (b) Add an optional `ref_dir` to `LookJudgeArgs` (`endpoint/src/packs/view.rs`): pass the last clean capture next to the test frame. (c) Rule: a VLM verdict never gates ragdoll clipping or pops; keep geometric and keypoint checks (`spatial`, `playtest/keypoint.rs`) as the gate. VLM is advisory.
- Why now: the judge exists but has no measured error rate. This is the cheapest way to get one.

### 2. CGVQM+D (STEAL NOW)
- Paper: 2506.11546 (2025, Intel). Full-reference video metric built on pretrained 3D-CNN features. Trained on distortions from neural supersampling, frame interpolation, VRS, denoising. Existing metrics reach at most 0.78 Pearson on that data. Outputs a per-pixel error map and a global score.
- Cost: offline Python + torch; GPU use not stated (UNVERIFIED, expect small).
- Code: github.com/IntelLabs/CGVQM, Apache-2.0.
- Build: `tools/cgvqm-gate.py` that compares native-res reference clips with dynres / EASU / TAA clips. It catches ghosting and flicker that SSIM windows (`proof/tests/capture_ssim_windows.rs`) can miss. Pin weights and torch version, record in the receipt. Never in the tick.
- Note: this is a better pick than the LPIPS item in `arxiv-neural.md` because it is made for time artifacts.

### 3. Temporal registration audit (STEAL NOW)
- Paper: Frozen History 2610.15752 (Oct 2026). It names four registration errors that hide in learned supersampling: motion vectors that carry camera jitter; one motion field used for both resolution lattices; jittered depth; an unregistered skip path. With exact registration a plain net beat the best published PSNR by 0.34 to 0.62 dB.
- Cost: none at runtime. Code MIT (github.com/CV-AC/Frozen_history).
- Build: our TAA is not learned, but the same traps apply to `render/post_jitter_motion.rs` ("current-minus-history motion vectors"). Add tests: on a static scene with jitter on, motion vectors are exactly 0; depth used for reprojection is unjittered; history reads use the unjittered position. Also try the "normalised sample accumulator" idea for the resolve weight.
- Why: ghosting and shimmer are G2/G5 pain. This is a test, not a feature.

### 4. Frame contact sheet for the VLM (STEAL NOW)
- Paper: Fly, Fail, Fix 2507.12666 (2025). A playtest agent hands the designer model "a compact image strip summarising recent video frames" plus numeric play metrics; the model edits the config.
- grep of `crates/` finds no contact sheet / filmstrip.
- Build: door verb `playtest.strip(capture_dir, n, cols)` that tiles N frames into one PNG with tick labels. Cuts VLM tokens and gives it motion context (needed for TempGlitch-type bugs). Code in `endpoint` + `proof` capture reader. Deterministic (fixed layout, no resample).

### 5. Play@k and PlayTester (STEAL NOW)
- Paper: PlayCoder 2604.19742 (2026). Benchmarks pass compile but get near-zero Play@3. Play@k = at least one of k generated candidates is playable end to end without logic errors. Closed loop: generate, play, repair. Repository-aware.
- Code: not found (UNVERIFIED).
- Build: add Play@k to `playtest/brief_eval.rs` (20 briefs). A brief counts only if a bot completes the staged slice with zero logic violations in a replay-x2 run. Gives a headline number for "AI builds a game".

### 6. Verifier inside every build verb (STEAL NOW, audit)
- Paper: SimWorld Studio 2605.09423 (2026). Rule verifier runs on every actor-changing tool call (collisions, vertical support, in-bounds). VLM verifier takes multi-view shots and returns structured feedback. Ablation on 9 held-out scenes: vanilla agent 0.16; plus MCP tools 0.45; plus verification loop +0.10; plus self-evolving skills +0.21. They also grow a curriculum near the learner's skill edge.
- Code: Apache-2.0 base repo SimWorld-AI/SimWorld exists; the Studio link in the paper returned 404 today.
- Build: audit every mutating `scene.*` verb: does it return the `spatial` checks (placement conflicts, support, bounds) in the same reply? If not, add it. Optional second signal: a 4-view capture for the look judge. Our `ConflictSet` and repair hints already cover most of this; the gap is "always inline".

### 7. Living debug protocol + 3-axis bench (STEAL NOW, small)
- Paper: OpenGame 2604.18394 (2026). Debug Skill = a log of (error signature, root cause, verified fix) plus cheap pre-run checks (asset key mismatch, missing config field). OpenGame-Bench scores Build Health, Visual Usability, Intent Alignment (headless run + VLM). Template Skill grows project skeletons from experience.
- Code: github.com/leigest519/OpenGame, Apache-2.0 (weights/model not checked).
- Build: `docs/` or skills file keyed by door error code, appended only after a verified fix (like `docs/BLOCKERS.md` but for fixes). Add the three axes to `brief_eval` output. Overlaps with the Voyager card C3 in `cards/2026-10-08-steal-papers-p5.md`; do both in one pass.

### 8. ARDY motion generator (STEAL NOW, spike)
- Paper: ARDY 2607.08741 (Jul 2026, SIGGRAPH 2026, NVIDIA). Streaming autoregressive diffusion with text plus constraints: root paths, full-body keyframes, sparse joints. Hybrid root + latent body. Supersedes MoMask (2312.00063, already in `arxiv-neural.md`) for our use because keyframe constraints fit "fill between our key poses".
- Cost: offline; GPU memory not stated (UNVERIFIED). Runtime zero if baked.
- Code: github.com/nv-tlabs/ardy, Apache-2.0. Weights on Hugging Face under the NVIDIA Open Model Agreement ("other"). Read it before shipping output. Data: Bones Rigplay.
- Build: `tools/ardy-bake` makes gladiator clips (stagger, shield bash recover, parry, death falls) from text + 2 key poses, retargets to our skeleton, writes the clip format `anim/clip.rs` already loads. Gate with the existing foot-skate and contact tests. Biggest cost is retargeting (M).

### 9. ReSTIR-sampled shadow maps (STEAL NOW, spike)
- Paper: Many-Light Rendering Using ReSTIR-Sampled Shadow Maps, EG 2025 (Utah + NVIDIA). Not on arXiv. ReSTIR picks the few lights that matter in view; only those get full-res shadow maps; the rest use imperfect (low-res) shadow maps.
- Why it fits: `render/restir_di.rs` is an oracle that needs a ray for torch visibility, and WebGPU has no ray tracing. This is a raster-only way to shadow the 250 torches.
- Cost: a handful of shadow maps per frame (e.g. 8 at 512²) fits 6 GB; potato tier uses fewer. Code: github.com/Utah-Graphics-Lab/ReSTIR-Shadow-Maps (LICENSE.md is the NVIDIA BSD-style text; read before porting; GitHub says NOASSERTION).
- Build: spike in `render` beside `shadow_cluster`: reservoir pick on the CPU (small), N cube/spot maps, imperfect maps for the rest. KILL if ms over bar on the 3050 or shimmer rises.

### 10. Neural texture compression family (LATER, measure first)
- Papers: Hypernetworks 2606.26913 (Jun 2026: one network emits latents + MLP weights, so no per-material training); Thread-Efficient Decoding 2608.27888 (Aug 2026: shared decoder + clustering, up to 8.48x on RX 9070 XT, cuts divergence 25-52%); older: Cooperative-vector NBTC 2506.06040. RTXNTC SDK has an "inference on load" mode: decode at load and transcode to BCn, works on any GPU with DP4a. Compressed latents ~5 bits/texel.
- Code: RTXNTC is under the "NVIDIA RTX SDKs LICENSE" (not open). Hypernetwork and thread-efficient papers: no code link found. So: reimplement the idea, do not vendor.
- Why LATER: `content/` is about 2.2 GB (fighters 1.3 GB, crowd 0.84 GB). I did not measure how much is textures. Step one is a size audit. If textures are the bulk of the web download, NTC "decode on load, store as BC" is the safe path (no per-sample net in shaders, so it works on UHD and WebGPU). Decoder in WGSL needs `dot4I8Packed` (check the wgpu 30 feature).
- Build (after audit): offline `tools/ntc-pack`, load-time compute decode in `assets`/`render`, existing BC encoder in-cook for the final block step.

### 11. Diff-targeted playtesting (LATER)
- SMART 2512.12706 (2025): an LLM reads the AST diff to make a reward so RL bots cover modified branches; 94% branch coverage of modified code, 98% task completion (Overcooked, Minecraft).
- For us: after a change in `combat` or `destruction`, bots get a bonus for hitting changed lines (needs cargo-llvm-cov on changed lines). Heavy plumbing, so LATER.

### 12. Auto-rig (LATER)
- UniRig 2504.12451 (2025, MIT, github.com/VAST-AI-Research/UniRig) and Puppeteer 2508.10898 (NeurIPS 2025, Apache-2.0, github.com/Seed3D/Puppeteer). UniRig README says generation needs a CUDA GPU with at least 8 GB VRAM, which is over our 6 GB. Rigel3D 2605.13129 (rig-aware latents, 2026) has no cost data I could verify.
- Build: offline `character.autorig` verb that must pass `asset-budget-checks` (bones, influences) and our skin tests. Value is for AI-built games with new characters, not the gladiator. LATER until we want AI-made characters.

### 13. LLM writes building programs (LATER)
- CityGenAgent 2602.05362 (Block Program + Building Program, trained with SFT then RL) and ProxyBuild 2609.23386 (Mesh-Anchored Procedural Proxies; LLM output decoupled from geometry). Code status UNVERIFIED for both.
- Matches our rule "model states relations, deterministic solver owns geometry" (`spatial/place.rs`). Build: a typed building-program schema + validator (insula, ludus, arch, colonnade) in `geo`; geometry stays deterministic. LATER: the arena needs few buildings.

### 14. Ready-to-React (LATER)
- 2502.20370 (ICLR 2025). Online reaction policy for two-character boxing: autoregressive model + one-layer MLP diffusion head; online decoder. Code github.com/zju3dv/ready_to_react under a "Project Registration License v1.0"; data (DuoBox) by form.
- Fit: duel opponent reactions. Offline use: make dodge/counter clips from attacker clips. Or distill into a tiny integer net (`crates/nn`, `distill`) that picks a reaction id in the tick. License and data access make it LATER.

### 15. MotionBricks (LATER)
- 2604.24833 (SIGGRAPH 2026, NVIDIA). One latent backbone over 350k clips; claims 15,000 FPS and 2 ms latency; "smart primitives" for navigation and objects. Repo not found at NVlabs/MotionBricks (404), so code status UNVERIFIED. Our `anim/motion_match` already gives runtime control; this would only matter for offline authoring, where ARDY is the pick.

### 16. Split Radiance Cascades (LATER)
- 2607.20384 (Jul 2026). Radiance Cascades in 3D with a sparse hashmap of world-space probes and "ray splitting". Gives diffuse GI without noise. Cost and tracing method not in the abstract (UNVERIFIED; may need ray tracing or a voxel march).
- Fit: dynamic torch bounce light. We have baked GI + SH probes, which cover static light. Wait until a measured need.

### 17. Generated PBR with a furnace gate (LATER)
- MatLat 2512.17302 (CVPR 2026, PBR texture on a mesh) and StableMaterials 2406.09293 (tileable PBR from text). Code and license not verified (the StableMaterials repo path I tried returned 404).
- Build: door verb `material.generate` whose output must pass `material/furnace.rs` and `compare.rs` before it is accepted. Good for "AI builds games". LATER.

### 18. Voxel PBD tearing (LATER, not arXiv)
- Gram-Schmidt voxel constraints (MIG 2024; Computers & Graphics 2025/26) and OmniBody (ACM 2026). Stylized soft-body destruction: 8 PBD particles per voxel, breakable face constraints, 1 + 3 compute partitions, hundreds of fps; anisotropic strain gives peel, strips, shards. No code found.
- Fit: cosmetic flesh/cloth tearing in `physfx` (float, view layer, never hashed). We already have `combat` sever and `destruction` bonded graphs; this only adds soft-body look. LATER.

### 19. WebGPU splats + mesh (LATER)
- Visionary 2512.08478 (Dec 2025): WebGPU 3D Gaussian splat renderer with GPU radix sort and depth-aware composition with meshes. Code: github.com/Visionary-Laboratory/visionary, Apache-2.0. PlayCanvas has a WebGPU hybrid raster splat path (commit 1b73d6d).
- Fit: a scanned backdrop (stands, far city) in the browser. Training is CUDA and we have crowd impostors already. LATER.

### 20. KG-guided update tests + BO explorer (LATER)
- KLPEG 2511.02534 (2025): a knowledge graph of elements and task dependencies; an LLM reads update logs and picks affected tests. Bayesian-optimisation grid explorer 2508.13121 (2025) for map coverage with few samples.
- Fit: `playtest/causal.rs` and `reachability.rs`. A BO explorer could hunt "walk through wall / into the void" bugs (TITAN's best new bug, 2509.22170). LATER.

## SKIP list (with reason)

- Unity Insight 2609.27585: we already have `endpoint/src/insight` with `asset.refs` and `callers_of`. Their 53% token cut (28 questions, 2 games) is a useful target to measure ours against.
- MeshFlow 2606.04621: FAIR Noncommercial license. No.
- Hunyuan3D 2.1 (2506.15442): README says 10 GB VRAM for shape, 21 GB for texture; Tencent community license. Over budget.
- SF3D 2408.00653: 2024, fast, UV-unwrapped PBR mesh. Stability community license; no low-poly output. Revisit only with a decimation step.
- ARMesh 2509.20824 (MIT): AR next-LOD mesh generation. Training is Linux-heavy; no current need.
- Neural Visibility Cache 2506.05930, Neural Irradiance Volume 2602.12949, Neural Caching of Prefiltered Radiance 2610.11702: need RT or network inference in shaders.
- PhysSkin 2603.23194 (CVPR 2026): net-driven secondary motion across shapes; we have `springs`. No license file.
- NanoVSR (MIT), LCS 2507.22873, GameSR: small CNN upscalers. EASU + RCAS already cost less; revisit only if CGVQM (item 2) shows a clear quality gap.
- RepDL 2510.09180 (bit-exact deep learning): `crates/nn` is integer already.
- AutoCompact 2610.02163, SpeedrunBench 2610.08076, OmniGameArena 2606.09826: dev-agent or benchmark only.
- Netcode / determinism: no recent arXiv paper beats what `net` already does (rollback, delta, interest, checksum, fault matrix). Not a gap.

## Order I would book

1. Items 1 and 4 together (judge calibration + contact sheet): one week of S work, gives the judge a measured error rate.
2. Items 2 and 3: new regression power for the G2/G5 look rows.
3. Items 5, 6, 7: raise the AI-builds-a-game score and make it measurable.
4. Items 8 and 9 as two spikes with kill rules.
5. Item 10 starts with a one-line size audit of `content/`.
