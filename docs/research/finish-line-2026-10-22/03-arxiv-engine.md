# arXiv papers to apply to engine2040 (round of 2026-10-22)

Scope: papers from 2023-2026 (two older ones flagged) that the engine can use beyond `docs/research/finish-line-2026-10-17/03-arxiv.md`. Row ids use prefix ARX.

Method: arXiv abstract pages opened with WebFetch for every paper marked "abs checked". Papers marked "search snippet" were not opened and their numbers are unverified. "Not stated" means the abstract gives no number. Papers already on file in other research folders are listed in section 5 and not re-booked.

## 1. Verdict

- The best new fit is hit reaction and stance recovery (ARX01). `anim/hit_react.rs` and `anim/feet.rs` have no balance-based rule for stagger, step or fall. A 2026 humanoid recovery paper gives that rule.
- AI-built levels gain two concrete checks: repair the cells that break reachability first (ARX03), and keep a saved validity verdict on each generated layout (ARX07).
- Physics-filtered motion banks (ARX02) are the strongest offline idea for the melee clip library.
- Potato-tier rendering and WebGPU/wasm: no 2025-26 arXiv paper beats our current plan. The one hard number (wasm 45-55% slower than native) is a budget input, not a steal.
- Deterministic small learned controllers: none found. The one caution (a 2026 study where INT8 breaks a physics rollout earlier than fp32) means any integer policy needs a rollout-stability test before it ships.
- Netcode and rollback: nothing new. `crates/net` already does rollback, delta and interest management. Do not book a netcode paper.
- Book now: ARX01, ARX02, ARX03, ARX04, ARX05 (all S or M). The rest are LATER or SKIP.

## 2. Comparison table

| Topic | Best new find | Fit for us | Call |
|---|---|---|---|
| Physics hit reaction and recovery | 2603.08619 (Mar 2026) | Capture-point rule picks stagger, step or fall | Book (ARX01) |
| Physics-filtered clip banks | 2505.04002 PARC (May 2025) | Physics re-track fixes foot contacts before a clip enters the bank | Book (ARX02) |
| Contact-rich duel tracking | 2604.07984 (Apr 2026) | Offline tracker training only | LATER (ARX06) |
| Motion matching with obstacles | 2510.22632 | Already on board as LM4 in hunt2 | Already on file |
| Potato rendering | 2506.02774, 2505.23158, 2604.21749, 2507.13917 | Splat and dense-mesh work; our meshes are low-poly | LATER or SKIP (ARX09, ARX10, ARX14, ARX15) |
| WebGPU and wasm performance | 1901.09056 (2019, background) | Gives a wasm/native budget to measure | Book (ARX12) |
| Deterministic GPU readback | 2609.25624 (Sep 2026) | Fixed-order sum for GPU counts that feed a proof hash | Book (ARX04) |
| Automated testing, VLM judge | 2607.25921 (Jul 2026) | Prompt-wording spread for the look judge; VLM as filter only | Book (ARX05) |
| AI-built levels | 2410.23101 (Oct 2024), 2409.06349 (Sep 2024), 2509.09919 (Sep 2025) | Repair priority, validity verdict, optimizer split | Book ARX03 and ARX07; ARX08 LATER |
| Learned small deterministic controllers | none found | n/a | Gap (section 6) |
| Netcode and rollback | none new | n/a | Gap (section 6) |

## 3. Ranked papers

Book-now items first. Board rows use the style "ID area: what becomes true. Accept: the test that proves it."

### ARX01. Embedding Classical Balance Control Principles in Reinforcement Learning for Humanoid Recovery (2603.08619, 2026)
- Abs checked. Submitted 9 Mar 2026. First author Nehar Poddar.
- Idea we apply: the training critic sees balance indicators (capture point, centre of mass, centroidal momentum); the actor only sees proprioception. We keep the runtime part as a plain rule: where the capture point falls against the support polygon picks ankle, hip, step or stand-up.
- Gap it fills: `anim/hit_react.rs` and `anim/feet.rs` have no balance-based stance choice after a hit.
- Target: `crates/anim/src/hit_react.rs` (classifier), `crates/anim/src/feet.rs` (support polygon from foot contacts).
- Effort: S for the classifier. The RL training stays out of the repo.
- Board row: ARX01 anim: a hit picks stagger, step or fall from the capture point, not from a timer. Accept: a seeded push table gives the same stance class on two replays; a push that moves the capture point outside the support polygon gives a step, not a stagger; golden hash unchanged.
- Caveat: the 93.4% recovery figure is for a Unitree H1-2 in Isaac Lab, not our rig. No code link on the abstract page.

### ARX02. PARC: Physics-based Augmentation with Reinforcement Learning for Character Controllers (2505.04002, 2025)
- Abs checked. Submitted 6 May 2025.
- Idea we apply: a motion generator makes new clips; a physics tracker imitates each one to correct flaws (wrong foot contacts, abrupt jumps); the corrected clips go back into the dataset and the generator retrains. Loop.
- Use for us: a re-track filter in the offline bake, so generated or edited clips enter the bank only after physics has fixed their contacts.
- Target: `tools/motion_bake/src/bake.rs` (new subcommand; note the file has uncommitted edits from another session, so add only our hunks), contact tests from `crates/anim/src/feet.rs`.
- Effort: M.
- Board row: ARX02 bake: generated clips pass a physics re-track before they enter the clip bank. Accept: the foot-skate and contact tests pass on the re-tracked clip; a fixture clip with a deliberate wrong foot contact comes out fixed; golden hash unchanged (offline tool).
- Caveat: abstract gives no numbers; no code link.

### ARX03. Guided Game Level Repair via Explainable AI (2410.23101, 2024)
- Abs checked. Submitted 30 Oct 2024.
- Idea we apply: explainability finds the regions that make a generated level unsolvable; those regions get higher weight so the constraint repairer fixes them first.
- Use for us: when an arena layout fails reachability, repair only the cells on the failing path, not the whole layout.
- Target: `crates/spatial/src/place.rs` (repair order), `crates/nav/src/lib.rs` (reachability check that names the failing cells).
- Effort: M.
- Board row: ARX03 spatial: an unreachable arena layout is repaired on the cells that break reachability first. Accept: on a seeded unreachable layout the repair changes only cells on the failing path, and the result passes the nav reachability check; repair time is printed and no slower than the unweighted repair on the same seed.
- Caveat: the abstract says it was tested on three games and gives no speed figure.

### ARX04. Accelerating the Mitigation of LLM Inference Nondeterminism Across GPU Architectures (2609.25624, Sep 2026)
- Abs checked. Submitted 22 Sep 2026.
- Idea we apply: a float reduction's order depends only on the problem shape (partial sums combined in a fixed order in a second pass, not by atomicAdd in whatever order threads finish).
- Use for us: any GPU count or sum that is read back and then hashed into a proof line (cull counts, visible-set sizes) gets the fixed-order two-pass form.
- Target: `crates/render/src/gpu_cull.rs` (readback path), `crates/proof/src/lib.rs` (hash line).
- Effort: S.
- Board row: ARX04 render: GPU counts that feed a proof hash come from a fixed-order two-pass sum. Accept: the cull count is bit-equal across two runs with different dispatch sizes; the proof hash line is unchanged.
- Caveat: the paper is about LLM inference. The idea transfers only to float reductions. Our sim is integer, so this guards only the GPU readback path.

### ARX05. Evaluating VLMs for Autonomous Agent-Driven Geometry Clipping Detection in Video Game QA (2607.25921, Jul 2026)
- Abs checked. Submitted 28 Jul 2026.
- Idea we apply: run the same judge with several prompt wordings across several VLMs and measure the spread. The authors conclude VLMs work best as high-recall candidate filters, not standalone bug detectors.
- Use for us: the look judge reports whether its verdict flips when the wording changes; a flipping verdict is marked unstable and never gates a build.
- Target: `crates/endpoint/src/packs/view.rs` (`LookJudgeArgs`).
- Effort: S.
- Overlap: `03-arxiv.md` item 1 (calibrate the judge on labelled sets). This adds the prompt-wording axis; it does not repeat item 1.
- Board row: ARX05 endpoint: the look judge reports its verdict spread over four prompt wordings. Accept: a verdict that flips with wording prints as unstable and does not gate; a test runs two wordings on one frame and checks the flag.
- Caveat: no numbers in the abstract; no code.

### ARX06. Physics-Based Motion Tracking of Contact-Rich Interacting Characters (2604.07984, Apr 2026)
- Abs checked. Submitted 9 Apr 2026.
- Idea: a progressive network of experts, where training samples are assigned to experts by difficulty automatically (no hand-made schedule).
- Use for us: offline training of a tracker for duel contact clips. Nothing runs at game time.
- Target: `tools/motion_bake` (much later).
- Effort: L.
- Board row: none now (LATER). Revisit when the duel needs a learned tracker.
- Caveat: abstract gives no quantitative results; no code link.

### ARX07. Improving Conditional Level Generation using Automated Validation in Match-3 Games (2409.06349, Sep 2024)
- Abs checked. Submitted 10 Sep 2024.
- Idea we apply: a generated layout is checked by an automated validity test before it is kept. The paper compares validity against an unconditioned baseline.
- Use for us: every generated arena layout carries a saved validity verdict (in bounds, no overlaps, reachable) with a named reason on failure.
- Target: `crates/geo` and `crates/spatial` (validity field on the layout). Most checks already exist (ConflictSet, nav); the new part is the saved verdict.
- Effort: S-M.
- Board row: ARX07 spatial: every generated arena layout carries a validity verdict before it is saved. Accept: the fixture set includes one invalid layout per check, and each is rejected with its named reason; the valid fixtures pass.
- Caveat: the paper is a learned generator. Only the validation-gate idea is ours. Abstract gives no numbers.

### ARX08. A Markovian Framing of WaveFunctionCollapse for Procedurally Generating Aesthetically Complex Environments (2509.09919, Sep 2025)
- Abs checked. Submitted 12 Sep 2025.
- Idea: WFC propagation enforces adjacency, and an external optimizer chooses tiles for the designer's objectives. The decoupled form beats joint search as task complexity rises.
- Use for us: keep adjacency inside propagation and the objective in a separate step, so the objective can change without touching the adjacency code.
- Target: `crates/endpoint/src/packs/grammar.rs` (we already own a WFC-style grammar; `hunt2` rejected a related database paper as a duplicate).
- Effort: S-M. Value: low-medium.
- Board row: ARX08 endpoint: the grammar pass keeps adjacency in propagation and the objective in a separate optimizer step. Accept: a test swaps the objective and shows adjacency results do not change.
- Caveat: no LLM use; no validity check described in the abstract.

### ARX09. Voyager: Real-Time Splatting City-Scale Gaussians on Resource-Constrained Devices (2506.02774, Jun 2025)
- Abs checked. Submitted 3 Jun 2025.
- Idea: under normal motion the number of newly visible items stays about constant, so the visible set can be updated incrementally instead of rebuilt.
- Use for us: only the incremental-list idea transfers to mesh culling in `gpu_cull`. The splat rasteriser does not.
- Target: `crates/render/src/gpu_cull.rs`.
- Effort: M. Value: low until our culling is a bottleneck on the 3050 or the potato tier.
- Board row: LATER. Accept (if booked): the visible-set update matches a full rebuild on a fixed camera path.
- Caveat: abstract reports up to 6.6x speedup and 85% energy saving; no frame times and no hardware named.

### ARX10. LODGE: Level-of-Detail Large-Scale Gaussian Splatting with Efficient Rendering (2505.23158, May 2025)
- Abs checked. Submitted 29 May 2025.
- Idea: chunked loading of splat LODs, with opacity blending at chunk edges so there is no seam.
- Use for us: only if a WebGPU backdrop uses splats (see `03-arxiv.md` item 19, Visionary, LATER).
- Target: `crates/render` (future backdrop path).
- Effort: L. LATER.
- Caveat: abstract mentions memory-constrained devices but gives no frame rates or hardware.

### ARX11. Learning to Control Physically-simulated 3D Characters via Generating and Mimicking 2D Motions (2512.08500, Dec 2025)
- Abs checked. Submitted 9 Dec 2025. Project page only.
- Idea: learn physics controllers from 2D keypoints taken from video, with no 3D mocap. Multi-view tracking adds 3D.
- Use for us: author reference clips for hit reactions and get-ups from reference video, offline.
- Target: `tools/motion_bake` (later).
- Effort: M-L. LATER.
- Caveat: no numbers in the abstract; no code link.

### ARX12. Not So Fast: Analyzing the Performance of WebAssembly vs. Native Code (1901.09056, 2019, background)
- Older paper, flagged. Search snippet only (abstract page not opened). Submitted to the 2019 USENIX ATC conference.
- Idea: measure the wasm/native ratio of your own hot loops and set a budget. The paper reports wasm about 45% slower in Firefox and 55% in Chrome on average on SPEC-style code, with peaks of 2.1x and 2.5x.
- Use for us: `crates/perf` budget workloads run in both native and wasm; the browser gate fails above a set ratio.
- Target: `crates/perf/src/lib.rs`.
- Effort: S.
- Board row: ARX12 perf: a browser build reports the wasm/native ratio for the same tick workload; the browser gate fails above 2.5x. Accept: the perf line prints both numbers; a fixture above the ratio fails.
- Caveat: a 2019 SPEC result. Our own ratio is the number that counts.

### ARX13. MOCHA: Real-Time Motion Characterization via Context Matching (2310.10079, Oct 2023)
- Abs checked. Submitted 16 Oct 2023.
- Idea: a neural context matcher gives a neutral motion the style and proportions of a target character, in real time.
- Use for us: a learned feature for motion matching. It conflicts with the frozen feature schema (LM1 in `hunt2`), so it needs a schema change first.
- Target: `crates/anim/src/motion_match.rs` (`make_feature`).
- Effort: L. LATER.
- Caveat: no runtime figure in the abstract; no code link.

### ARX14. CuRast: Cuda-Based Software Rasterization for Billions of Triangles (2604.21749, Apr 2026)
- Abs checked. Submitted 23 Apr 2026. Code at github.com/m-schuetz/CuRast.
- Idea: visibility buffer with atomicMin for small triangles; the authors report 2-5x faster than Vulkan on dense photogrammetry meshes, but Vulkan is about 10x faster on low-poly meshes.
- Use for us: none now. Our arena meshes are low-poly, and WebGPU has no 64-bit atomics.
- Target: `crates/render/src/gpu_cull.rs` (only if dense scans ever enter the game).
- Effort: L. SKIP.

### ARX15. Neural-GASh: A CGA-based neural radiance prediction pipeline for real-time shading (2507.13917, Jul 2025)
- Abs checked. Submitted 18 Jul 2025. Unity; targets mobile and VR.
- Idea: a neural model predicts per-vertex radiance, so animated meshes get shading without precomputed transfer.
- Use for us: none now. Needs training, and the abstract gives no frame times, hardware or code.
- Target: none.
- Effort: M-L. SKIP.

## 4. What NOT to copy, and why

- CuRast (2604.21749): dense-mesh only. Its own numbers say Vulkan wins on low-poly by about 10x; our meshes are low-poly.
- Neural-GASh (2507.13917): no frame times, no code, needs training, and it targets Unity mobile and VR.
- Voyager (2506.02774) and LODGE (2505.23158): splat renderers. We do not ship splats. Wait for the splat decision in `03-arxiv.md` item 19.
- MOCHA (2310.10079): breaks the frozen motion-matching feature schema (LM1).
- INT8 for a rollout policy: a 2026 physics-informed model study (2608.00737, embedded RISC-V) reports its trajectory collapses at INT8 before the fp32 MLP does. It is not a game paper, but any integer or quantized controller must pass a long-rollout stability test before it ships.
- Learning getting-up policies (2502.12152, Feb 2025): sim-to-real on a real robot. It is not a source of game clips.
- Markovian WFC (2509.09919): we already own a WFC-style grammar; take only the optimizer split (ARX08), not a new generator.
- Netcode and rollback: no new arXiv paper beats what `crates/net` already does. Do not book one.

## 5. Already on file (not new, not re-booked)

- 2510.22632 Environment-aware Motion Matching: `hunt2-2026-10-21/agent-5-papers-ideas.md` row 4 (LM4).
- 2608.21833 GameXpert-Bench: `powerups-2026-10-20/08-arxiv-ai-games.md` (repair track).
- 2508.18533 database-driven level generation: rejected in `hunt2-2026-10-21/agent-5-papers-ideas.md` (duplicates our WFC grammar).
- 2301.05262 Neural Shadow Mapping: rejected in `powerups-2026-10-20/06-arxiv-rendering.md` (needs training).
- 2604.02648 GBQA: `steals-2026-10-11/arxiv-neural.md`.
- 2409.14393 MaskedMimic: `LADDER.md` and `steals-2026-10-11/arxiv-neural.md`.
- 2502.20370 Ready-to-React, 2509.22170 TITAN, 2505.15952 VideoGameQA-Bench, 2604.11082 RefGlitch, 2605.21443 TempGlitch: `03-arxiv.md`.

## 6. Gaps: what I could not find

- Small, deterministic learned controllers that run at game time: no 2025-26 arXiv paper found with an integer or bit-exact runtime.
- Impact-aware ragdoll stagger (physics hit response): only commercial and mod pages turned up. No arXiv paper found.
- WebGPU or wasm performance study from 2025-26 on arXiv: none found. The 2025 WebGPU results were a Colombo undergraduate thesis and a TU Wien thesis (BAUER 2025), neither on arXiv.
- Netcode: nothing new beyond what `crates/net` covers.
- Unverified: 1901.09056 (search snippet only). The numbers for PARC, Mimic2DM and the contact-rich tracker are not in their abstracts.
