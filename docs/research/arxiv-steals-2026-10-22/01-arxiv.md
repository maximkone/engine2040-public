# arXiv steals for engine2040 (hunt of 2026-10-22)

Source: arXiv API hunt on 2026-10-22 (queries `all:"game engine"`, `all:"generative game"`,
`all:"generative rendering"`, `all:"agentic game"`; sorted by submittedDate descending; every hit id
opened). Fresh window: papers from 2609.03 up to today (2610.28).

Read first: `finish-line-2026-10-17/03-arxiv.md`, `steals-2026-10-20/`, `docs/plan/finish-line.md`
("What we still lack"). Nothing here repeats a taken item: MoMask, NTBC, LPIPS, Cutscene Agent,
LatticeWorld, GGRS, Spring-It-On, EASU, ReSTIR, CGVQM and the look judge subtree are owned or
covered.

## What the engine already has (so no steal duplicates it)

- `anim`: motion_match, inertialize, hit_react, ik (two_bone, fbik), springs, feet, warp_ground.
- `render`: restir_di (oracle on the CPU side, ray for torch visibility), upscale_easu, GTAO, TAA
  jitter, baked lightmaps + SH probes, meshlet, gpu_cull, dynres_gov, torch clusters, cloth.
- `proof`: SSIM windows only (no learned metric); no renderer-fidelity gate yet.
- `net`, `endpoint`, `assets`: rollback, door verbs, native BC encoder as in the 2026-10-17 hunt.

## Top steals, ranked by value per effort

| # | Idea (arXiv) | Verdict | Where it lands | Effort |
|---|---|---|---|---|
| 1 | Programmable World Model 2609.10540 | STEAL NOW | scene/world + render generative path | M |
| 2 | Matrix-Game 4.0 2610.27999 | STEAL NOW (spike) | render harness + proof | M-L |
| 3 | SolarRenderer 2610.28630 | STEAL NOW (offline) | render/golden path | S-M |
| 4 | DAGS 2610.02567 | STEAL (M) | render/torch | M |
| 5 | FlashRender 2609.03563 | STEAL (M) | render | M |
| 6 | Mallea 2610.21283 | STEAL (M) | endpoint/agent world builder | M |
| 7 | RSIGame 2609.39045 | STEAL (M) | agent loop / playtest eval | M |
| 8 | Recursive Game Creator 2610.08621 | STEAL (M) | AI-builds-game eval | M |
| 9 | OpenGameEval 2610.02563 | ALREADY ON FILE | playtest eval | - |
| 10 | Code2Games 2610.05033 | ALREADY ON FILE | AI-native build loop | - |

## Details

### 1. Programmable World Model (STEAL NOW)

- Paper: arXiv 2609.10540 (Sep 2026), AlayaLab (pwm). Decouples world-state evolution from visual
  observation generation.
- An agent translates natural language into an executable program: entity states plus transition
  rules. A lightweight engine executes the programs, so the world keeps persistent global state,
  off-screen entities included. User actions steer; text edits extend capabilities with state kept.
- The program feeds state-augmented oriented bounding boxes (OBBs) to a pretrained video generative
  renderer as pixel-aligned spatiotemporal conditioning. That renderer turns low-dimensional state
  into video.
- CombatStateBench: 94% count accuracy, 98% state accuracy, against prior interactive video world
  models.
- Lands in: `crates/scene` (program shape), `crates/world`, the render generative path, `crates/proof`.
- Effort M. Why now: this is the architecture behind Matrix-Game 4.0 and our own WEB5/WEB17 plan
  (real cooked fight in a browser page that plays the match it draws).

### 2. Matrix-Game 4.0 (STEAL NOW, spike)

- Paper: arXiv 2610.27999 (20 Oct 2026). Next step past controllable video: split game execution
  from visual generation. Harness keeps world state and executes interaction rules; a multimodal
  coding agent turns player ideas into game code and dynamic scene captions; a renderer based on an
  autoregressive video model turns engine-rendered low-poly proxies into detailed visuals.
- Distilled causal generator: 4-frame chunks with 1-2 denoising steps; 480p 60 FPS on one RTX5090,
  key-to-display latency 112 ms, 18.2 GiB peak GPU, single-step 83 FPS, steady 67 FPS on RTX5080 and
  58 on RTX5070 Ti. Released code, models and low-poly assets.
- Steal: the harness split and the distilled causal generator shape. Check the released code before
  porting anything.

### 3. SolarRenderer (STEAL NOW, offline)

- Paper: arXiv 2610.28630 (20 Oct 2026). Structure-conditioned video rendering with a frozen
  backbone plus two LoRAs; structural videos enter the native reference pathway at the frames they
  describe, plain text only; 4-step structure-guided distribution matching, 12.3x faster than 50-step;
  768p real-time, block-causal streaming at real-time 768p.
- Build: offline golden render path. The LoRA shape for structure conditioning is the steal.

### 4. DAGS (STEAL, M)

- Paper: arXiv 2610.02567 (Oct 2026). Frozen image DiT steered by attention-free appearance and
  geometry conditioning, plus a recurrent lighting stabilizer and training-free temporal guidance.
  +8.6/+10.1 dB PSNR over the real-time denoiser on a matched 1-spp + G-buffer input, 2.5-8x more
  temporal stability (temporal-LPIPS flicker).
- Lands in: `render` torch path.

### 5. FlashRender (STEAL, M)

- Paper: arXiv 2609.03563 (Sep 2026). RETA (representation transformation and alignment) plus
  MeanFlow plus on-policy flow distillation; 25x lower sampling cost; camera control kept consistent
  even out of distribution.
- Lands in: `render`.

### 6-10. Agentic game development steals (STEAL, M each)

- Mallea 2610.21283: coding agent builds, steers and memorizes worlds around one object graph; the
  program owns state, the renderer owns appearance. Lands in the AI-native world builder. NEW.
- RSIGame 2609.39045: local explore-diagnose-improve plus global loop, experience internalized into
  the generator. Lands in the agent loop and playtest eval. NEW.
- Recursive Game Creator 2610.08621: Designer/Builder/Player/Reviewer recursion; GameCraft-Bench
  77.89, GameASG-Bench strict task success +34.1%, runtime-check pass rate 93.4%. NEW.
- OpenGameEval 2610.02563: stateful engine eval harness; observation vs editing tool split; 13 models
  on 84 tasks; exploration predicts success. ALREADY ON FILE: `docs/research/hunt2-2026-10-21/agent-5-papers-ideas.md:27`
  read it live on 2026-10-21 and it feeds rows PW24 and HX29. The exploration split (13.4 pp on
  scene-only tasks) is already booked there. Do not book it twice.
- Code2Games 2610.05033: coding agents build worlds; execution-guided reconstruction from compile
  diagnostics, runtime feedback and gameplay tests; GameCode4D. ALREADY ON FILE: the same
  hunt2 agent-5 note (line 312 of the LADDER records it) read it live on 2026-10-20. The
  ten-prompt benchmark shape is already fed to the VF1 and AB rows. Do not book it twice.

## Order I would book

1. Programmable World Model as design doc plus spike (PROG1): the architecture behind the finish line.
2. Matrix-Game 4.0 harness split as the generative render path (spike): the browser-plays claim.
3. SolarRenderer 4-step as the offline golden render path.

## Honest limits

- Matrix-Game 4.0 numbers are RTX5090 with a released model; our bar is RTX3050 6 GB and a real sim
  hash.
- The OBB-to-pixel conditioning compile step is new work, not yet ported here.
- DAGS is not real-time; FlashRender is real-time on their backbones only.
- RSIGame internalization and Mallea memory details are unverified.

## Sources

- arXiv: 2609.10540, 2610.27999, 2610.28630, 2610.02567, 2609.03563, 2610.21283, 2609.39045,
  2610.08621, 2610.02563, 2610.05033, 2610.18834, 2610.11129, 2609.39564, 2609.02931, 2609.09002.
- API: https://export.arxiv.org/api/query?search_query=all:%22game+engine%22&max_results=25
