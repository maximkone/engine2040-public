# Motion papers to bake clips for engine2040 (hunt of 2026-10-20)

Continuation of `03-arxiv.md` (2026-10-17). That card's row 8 is ARDY; this card cites it and does not repeat it. None of this runs in the tick.

Method: opened `arxiv.org/abs/<id>` and the paper HTML (v1) for every id on 2026-10-20; read the title, the date and every number quoted below from those pages. Read licence, stars and last push from the GitHub API on 2026-10-20. Read VRAM, parameter counts and weight licences from the repo README and the Hugging Face model cards. Did not run a model, bake a clip or time anything on our 3050. UNVERIFIED = could not check.

## What the engine already has (so I did not suggest it)

- `anim/clip.rs`: `QTrack`/`QClip` hold 3-lane position tracks, bake and sample at the fixed tick, and measure foot slide (`slip`, `avg_slip`) — clip.rs:1, 6, 13, 203, 213. Quaternion lanes are not in `QTrack`: they live in `pose::QuatTrack` (pose.rs:33) and packed `PackedQuatTrack` (pack.rs:229).
- `anim/pack.rs`: AM5P wire, smallest-three u16 quaternion keys, 2 MB per fighter and 64 clips per fighter budgets — pack.rs:18, 26, 30, 218, 245.
- `anim/motiondb.rs`: bounded `DbSlice` over `motion_match::MotionDb`; the consumer is traversal ranking (`rank_approach`, traversal.rs:421) — motiondb.rs:3-4, 46.
- `nn`: integer-only `.e2nn` runtime; the arena bind is still open as EG24(d) — nn/src/lib.rs:1-5. `motion_match::TrajNet` (4 lanes in, 6 out) loads a net and predicts, but only tests call it — motion_match.rs:481-486. It calls the allocating `infer`, not `infer_into` — motion_match.rs:531.
- `distill`: student export to `.e2nn` bytes that the runtime reloads and checks — distill/src/export.rs:1, 163; training export beside it — nn/src/lib.rs:59-62.
- `assets/retarget.rs`: `BoneMap::build`, `retarget_clip`, `retarget_cook` and a stable `retarget_digest` already exist — retarget.rs:267-275, 404, 1238, 1324. `game` maps same-name rigs only — fighter_look.rs:1417, 1510.
- Socket data is in the cook: `SOCKET_` mesh names and `HitWindow.socket` — mesh_cook.rs:966, retarget.rs:891-897. The runtime attach is EG40, still open — SPRINT-18-gaps.md:695.
- Clip events exist: `TimedEvent`, `EventBank::at` — graph.rs:385, 394, 421.

So the missing piece is not "motion generation". It is one teacher path into the clip format and one map onto our rig.

## Ranked by value / effort

| # | Idea (paper) | Verdict | Where it lands | Effort |
|---|---|---|---|---|
| 1 | Kimodo text+keyframe teacher (2603.15546) | STEAL NOW (off-tick bake) | `tools/motion_bake` + `assets/retarget.rs` | M-L |
| 2 | PartiGen endpoint rule for clip length (2610.13373) | STEAL (small) | bake tool duration rule | S |
| 3 | ARDY streaming teacher (2607.08741) | ROW 8 STANDS (new facts only) | offline bake tool, second backend | M |
| 4 | TRACE typed support/root/hand events (2610.21515) | LATER (small steal) | sidecar tags + `anim/graph.rs` | M |
| 5 | Absolute coordinates (2505.19377) | READ (design check) | retarget map note | S |
| 6 | ELMA multi-segment eval (2610.13549) | LATER (eval only) | bake prompt set | S |
| 7 | Image diffusion across two skeletons (2610.14254) | WATCH | none yet | - |
| 8 | MotionBricks (2604.24833) | LATER (official repo still missing) | offline authoring | M |

## Details

### 1. Kimodo (STEAL NOW, off-tick teacher)
- Paper: 2603.15546 (2026-03-16). "Kimodo: Scaling Controllable Human Motion Generation". 700 hours of optical mocap.
- It takes text plus full-body keyframes, sparse joint positions/rotations, 2D waypoints and paths. A two-stage denoiser splits root and body.
- Numbers: 282 M parameters (HF model card, and the paper HTML), 30 fps, max 10 s per clip (HF card), 2 to 5 s per clip on an RTX 3090 (paper HTML). Resolved 2026-10-20: the model runs on the 30-joint `somaskel30`, and public outputs, constraints and BVH use the 77-joint `somaskel77` (`docs/source/key_concepts/skeleton.md`, `docs/source/user_guide/output_formats.md`). So the map is 77 to 65, not 30 to 65. Measured 2026-10-20 with `gap.py`: 52 of the 65 bones match a SOMA joint by name and position, 13 need a rename, 12 SOMA joints are unused (face and eight fingertips). Four names are traps (`Spine1`, `Spine2`, `LeftLeg`, `RightLeg`), because the chains are offset by one.
- Cost: about 17 GB VRAM, mostly the text encoder; `TEXT_ENCODER_DEVICE=cpu` drops it under 3 GB (`docs/source/getting_started/quick_start.md`). Encoder resolved 2026-10-20: LLM2Vec on `McGill-NLP/LLM2Vec-Meta-Llama-3-8B-Instruct-mntp` plus PEFT, 4096-wide, and the base `meta-llama/Meta-Llama-3-8B-Instruct` is gated (`kimodo/model/load_model.py:22-36`). Pre-computed embeddings are not on the public call signature: only `TEXT_ENCODER_MODE=api` with `TEXT_ENCODER_URL`, `load_model(text_encoder=...)`, and the demo's `CachedTextEncoder` avoid the in-process encoder.
- BVH is SOMA-only: `kimodo_gen --bvh` and `kimodo_convert IN OUT --to soma-bvh` write a `somaskel77` hierarchy with root translation in centimetres (`docs/source/user_guide/output_formats.md`, `motion_convert.md`). G1 and SMPL-X have no BVH path.
- Code: github.com/nv-tlabs/kimodo, Apache-2.0, 3,826 stars, pushed 2026-10-15. Weights: NVIDIA Open Model License, last modified 2025-10-24, not Apache-2.0. Read 2026-10-20: models are commercially usable; the required notice is exactly "Licensed by NVIDIA Corporation under the NVIDIA Open Model License" in a Notice file; "An output is not a Derivative Model" and "NVIDIA claims no ownership rights in outputs", so distilling outputs is not blocked by this licence (the Cosmos-only clause does not name Kimodo). The SMPL-X variant is under the R&D licence, not this one.
- Windows: the README says Linux-first and suggests Docker for Windows. On this PC that is UNVERIFIED; nothing was run.
- Build: `tools/motion_bake` reads the NPZ, writes a glTF clip that `import_clip` already reads (retarget.rs:40-42), then calls `retarget_cook` and `pack`. Resample 30 to 60 Hz at the cook. Gate with `avg_slip` and the contact table. Never in the tick.
- Why: 2 to 5 s per clip is 120 to 300 ticks at 60 Hz. It cannot be in a tick even if the GPU were free.

### 2. PartiGen (STEAL, small)
- Paper: 2610.13373 (2026-10-09). Part-aware latent diffusion for the Unitree G1.
- Steal: the learned end-of-sequence predictor. With 16-frame primitives it cuts mean absolute duration error from 12.471 to 3.080 frames against a fixed-length cutoff (abstract).
- Build: in the bake tool, stop the teacher at the frame whose end-effector pose best matches the next key pose, instead of a fixed duration. Deterministic argmin over a small window; print the chosen frame and the match error. This is a rule, not a net.
- Code: no official repo found. A GitHub search returns only `andyhuangzz01/partigen-paper` (1 star, no licence), so code is UNVERIFIED.
- Why: a clip that ends on the key pose removes cut tails and makes "fill between poses" useful.

### 3. ARDY (row 8 stands)
- Paper: 2607.08741 (2026-07-09, SIGGRAPH 2026 / ACM TOG). Streaming autoregressive diffusion with text and keyframe constraints. Row 8 is `03-arxiv.md:30` and `:87-91`.
- What changes: repo is Apache-2.0, 1,008 stars, pushed 2026-10-15; 192 M parameters (denoiser 157 M + tokenizer 36 M, text encoder excluded); 33 ms average for the 4-step model and 63 ms for the 10-step model on an A100 (paper HTML); 30-joint SOMA at 30 fps.
- Gate: the text encoder is the gated `meta-llama/Meta-Llama-3-8B-Instruct` and needs a Hugging Face account and token (README). That is the RAM gate.
- Verdict: row 8 keeps its M and its offline home. Try Kimodo first for quality; keep ARDY as the fast backend. The 33 ms is A100-only; our 3050 is UNVERIFIED.

### 4. TRACE (LATER, small steal)
- Paper: 2610.21515 (2026-10-15). Unitree G1; typed support, root and hand events; multi-turn editing with body-time scopes and source-relative goals.
- Fit: `EventBank` already fires integer `(tick, code)` cues (graph.rs:385-428) and the sidecar already carries tags and hit windows (retarget.rs:60-66, 891). The steal is the typed edit schema: support/root/hand, scope, preserve list.
- Code: no repo found. UNVERIFIED.
- Why LATER: editing is useful after the bake tool writes clips, not before.

### 5. Absolute coordinates (READ)
- Paper: 2505.19377 (2025-05-26). Absolute global joint coordinates and a plain Transformer, with "no auxiliary kinematic-aware losses" (abstract). Also supports editing and SMPL-H vertices.
- Code: github.com/neu-vi/ACMDM, MIT, 74 stars, pushed 2026-07-22.
- Fit: a design check for the retarget map. Keep the teacher's global joints until the cook, then convert once to local tracks. No engine change.
- Why read: it is evidence that the local-relative representation is not required, which lowers the risk of a one-way SOMA to fighter map.

### 6. ELMA (LATER, eval only)
- Paper: 2610.13549 (2026-10-09, NeurIPS 2026 D&B). 30-second clips with segment annotations; current models fail on multi-segment prompts with anaphora.
- Fit: a small prompt set for the bake tool. Two sentences, the second says "keep the left hand up". Score whether the second constraint holds to the end.
- Code: none found. UNVERIFIED.
- Why LATER: our bake prompts are short today; this matters when a fight is authored as a sequence.

### 7. Image diffusion as motion (WATCH)
- Paper: 2610.14254 (2026-10-09, under review). One motion autoencoder and one text-conditioned SiT trained on HumanML3D and KIT-ML, "without skeleton retargeting to a common joint layout" (abstract).
- Why it matters: it is the only candidate that attacks the retarget step itself. If one model can hold two skeletons, it could hold SOMA and the fighter rig.
- Code: none found; 16 pages, under review. UNVERIFIED.
- Verdict: watch. Do not plan around it.

### 8. MotionBricks (LATER)
- Paper: 2604.24833 (2026-04-27, SIGGRAPH 2026 / ACM TOG). 350k clips, one latent backbone, the abstract claims 15,000 FPS and 2 ms latency, plus smart primitives.
- Official repo: still not found. `NVlabs/MotionBricks` and `nv-tlabs/MotionBricks` both 404 on 2026-10-20.
- What is new: a third-party GGML/C++ port `localai-org/motion-bricks.cpp` (Apache-2.0, 308 stars, pushed 2026-10-19) and `Aero-Ex/Nvidia_MotionBricks` (13 stars, NOASSERTION). Neither is official; the port's weights licence and quality are UNVERIFIED.
- Verdict: LATER. `motion_match` already gives runtime control; this is offline authoring, where the Kimodo bake is the pick.

## Three homes

- In-tick core: `.am5p` clips sampled by integer code (clip.rs:1), integer motion matching (motion_match.rs:8-10), and a net only through `infer_into` (nn/src/lib.rs:38-46). No file IO, GPU or provider inside the fixed tick (AGENTS.md:36).
- Off-tick cook: the teacher runs as an editor/build job (D3, FOCUS.md:36), writes hashed clips (`retarget_digest`, retarget.rs:1324) and uses free, commercial-use models only.
- Never in the tick: a diffusion step, the 8B text encoder, a Hugging Face download, or a 17 GB model (Kimodo README:76). The card is 6 GB (animation.md:66) and a tick is 16.6 ms.

## Dependencies, in order

1. C14 weapon sockets: no `socket` in `crates/anim`, but the cook already names `SOCKET_` joints and hit windows (mesh_cook.rs:966, retarget.rs:891). The runtime attach is EG40 (SPRINT-18-gaps.md:695), open, M. Do not book a second row. C14 still lists quaternion lanes (finish-line.md:65), but pack.rs:229 already packs them; the open half of C14 is sockets and rotation additive.
2. The retarget map: `BoneMap::build` and `retarget_cook` exist (retarget.rs:267-275, 1238); the work is the SOMA 77 name table (52 exact, 13 renames, 12 unused; `gap.py` GAP PASS, 2026-10-20), the bind scale and a foot-contact test. Effort M. It unblocks both teachers.
3. The bake tool: `loco_bake` bakes only the hero set (loco_bake.rs:187, 501); no external teacher path exists. The tool calls Kimodo, imports, retargets and packs. Effort M.
4. Teacher to student: `TrajNet` is built but has no production caller (motion_match.rs:481-486); export goes through `distill` (export.rs:163); the arena policy bind stays EG24(d) (nn/src/lib.rs:4-5). Effort L.

## What to book (3 rows)

### AM26 — SOMA-to-fighter retarget map
- Lands: `crates/assets/src/retarget.rs` plus a SOMA fixture in `crates/assets/tests/it/retarget_soma.rs`.
- Map size: 52 exact names, 13 renames, 12 SOMA joints unused (`gap.py` GAP PASS, 2026-10-20).
- Effort: M.
- Accept: `node tools/gh-check.mjs cargo test -p engine-assets --lib retarget_soma` x2 — the fixed SOMA fixture cooks twice to the same `retarget_digest`, every mapped joint is within 2 cm of the authored fighter target, and the cooked foot contacts equal the fixture's 8 windows.

### AM27 — offline motion bake from a teacher to `.am5p`
- Lands: `tools/motion_bake/` (NPZ to glTF, retarget, pack) plus `crates/anim/tests/it/motion_bake.rs`.
- Effort: M.
- Accept: `node tools/gh-check.mjs cargo test -p engine-anim --lib motion_bake` x2 — the committed `fixtures/clips/stagger.am5p` loads, `avg_slip` is under 0.08, `unpack_qclip` round-trips, and `qclip_hash` equals the receipt printed by two tool runs with seed 7.

### AM28 — distilled trajectory student in the tick
- Lands: `crates/anim/src/motion_match.rs`, `crates/distill/src/export.rs`, `fixtures/nets/traj.e2nn`.
- Effort: L.
- Accept: `node tools/gh-check.mjs cargo test -p engine-anim --lib traj_student` x2 — the net cuts mean endpoint error at least 20% against `predict_linear` on the held-out fixture, `infer_into` allocates nothing, and a two-fighter tape with the net on repeats its `arena sim:` hash. The default stays `predict_linear`, so the golden does not move.

## SKIP list (with reason)

- HY-Motion 2512.23464: skip confirmed. The README table lists 26 GB minimum for the 1.0B model and 24 GB for the 0.46B Lite. The Tencent community licence excludes the European Union, the United Kingdom and South Korea and forbids use outside the territory (`License.txt`). Over the 6 GB card twice, and unusable in those markets.
- MotionBricks official code: still missing (see 8). The third-party port is not a substitute until its weights licence is read.
- No other paper here beats `motion_match` for runtime control. Nothing in this card belongs in the tick.

Order: AM26, then AM27, then AM28. Sockets stay on EG40.
