# arXiv wave 2026-10-23 (Now items 4, 5, 9)

Exact form `https://export.arxiv.org/api/query?search_query=...&max_results=8`,
one narrow query per gap:

1. `all:%22physics-based%20character%20animation%22`
2. `all:%22motion%20matching%22`, `all:%22physics-based%22%20AND%20all:locomotion%20AND%20all:distillation`
3. `all:%22crowd%20rendering%22`, `all:%22large-scale%20crowd%22`, `all:%22crowd%20simulation%22%20AND%20all:rendering`
4. `all:%22integer-only%22%20AND%20all:inference`, `all:%22bit-exact%22%20AND%20all:inference`

Ids deduped before any text fetch. Eight papers opened at their HTML text;
abstract plus named sections read 2026-10-23.

Verdict: three can land (CPU, offline or presentation, no GPU in the tick);
the physics-based controllers and the Gaussian crowd cannot pay here.

| id | title | year | fit | needs |
|---|---|---|---|---|
| 2510.22632 | Environment-aware Motion Matching | 2025 | LAND | no weights; a motion DB with baked body ellipses, obstacle list, representative index |
| 2510.20558 | From Far and Near: Perceptual Evaluation of Crowd Representations Across Levels of Detail | 2025 | LAND | no training; one captured character, 4 LOD sets, 24 users (or our capture pair) |
| 2505.19976 | MAMM: Motion Control via Metric-Aligning Motion Matching | 2025 | LAND (offline) | no training, no paired data; a source clip plus a control sequence; CPU OT solver |
| 2607.23227 | INT8 Quantization Makes ARM Edge Inference Dispatch-Invariant | 2026 | rule only | nothing; it confirms int32 accumulation is the reproducible mode |
| 2406.00960 | PDP: Physics-Based Character Animation via Diffusion Policy | 2024 | no | 4-8 A100/V100 GPU-days, AMASS/AddBiomechanics, K denoise steps per tick |
| 2606.29148 | GPC: Large-Scale Generative Pretraining for Transferable Motor Control | 2026 | no | 600+ hours of clips, RL + GPT training, GPU sim |
| 2501.17792 | CrowdSplat: Exploring Gaussian Splatting For Crowd Rendering | 2025 | no | CUDA splat renderer, 3.5k agents at 31 fps on an RTX 4090, 0.5-6 GB |
| 2606.29209 | AnyBody: Free-Form Whole-Body Humanoid Control from Arbitrary Keypoint Guidance | 2026 | no | Isaac Lab, PPO 200k iterations, transformer + MLP student |

Rules when reading: reject a method that needs a GPU in the tick, float-only
sim state, or an online service. All three cards are CPU, two offline.

## P1 - obstacle penalty lanes in the integer motion search

Source: arXiv 2510.22632, `Environment-aware Motion Matching`, sections
4.1.2 (environment features), 4.2 (search), 5.4 (optimisations), 6.3.

Method, five lines:

1. Every DB pose carries a 2D body ellipse (semi-major along the move
   direction, both axes from projecting all joints), for 20, 40 and 60
   frames ahead.
2. The query is 27 pose/trajectory lanes (foot and hip velocities, foot
   positions, future positions and directions) plus 9 ellipse lanes.
3. Score = weighted query distance, then an obstacle penalty; the scan
   rejects a pose as soon as its partial score is worse than the best.
4. A sparse representative index (poses at least 8 frames apart and 5%
   different) is scanned first, then the band around a winner; the stride
   grows with distance from the current best and shrinks near it.
5. The scan starts near last frame's index, so a good first score exists
   early and most of the DB is rejected cheaply.

Paper claim, not ours: 0.80 +/- 0.16 ms per search in a dense obstacle
field, every 10 frames, single CPU thread, 184,553 poses; 0.22 +/- 0.05 ms
with no obstacles; collision time 0.88 s -> 0.08 s; pose diversity 1997 ->
4378. Floats throughout.

What it needs: no weights, no training, no service. The offline bake adds one
ellipse per pose (mm); the per-tick obstacle list is in reach (our ORCA discs
and `nav::crowd_contact` discs are the same shapes); the index bakes with DB.

Where it lands: `crates/anim/src/motion_match.rs` (adds env lanes and the
penalty path; the authority stays integer) and `crates/anim/src/motiondb.rs`
(representative slice). Row LM4, which already owns the feature-schema
freeze and the PF1 cost workload. No new row.

Fixture and spike: `fixtures/scenarios/duel_seed95.json`, one fighter on a
scripted slalom past 6 disc obstacles for 600 ticks, plus the same tape with
no obstacles. Baseline `search` today, candidate with env lanes. Metrics:
overlap ticks 0, path error mm, search microseconds per tick in a PF1
workload, and the `arena sim:` hash equal on the no-obstacle tape.

Kill: any overlap tick left; or cost over 0.25 ms per fighter per tick on
this box; or the no-obstacle hash moves; or the schema test cannot freeze
the new lanes (then LM4 splits first).

Honest note: the paper is float and nearest-wins; our lanes are integer with
a stable index tie-break, so the penalty constants and the early reject are
our work. No quantised version exists in the paper because no net is involved.

## P2 - crowd bands chosen by perception, not by taste

Source: arXiv 2510.20558, `From Far and Near`, sections 3.3 (impostor),
4.1 (study), 4.2 (metrics, memory), 5 (guidelines). Secondary: 2501.17792.

Method, five lines:

1. One character, four representations (mesh, impostor, NeRF, 3DGS), four
   LODs (100/50/25/12.5%) and five pixel densities (100% to 20%), shown
   side by side to 24 people.
2. The impostor is a 6 x 10 sprite sheet per LOD; the atlas is cropped to
   the union alpha box over the cycle and given one global scale per LOD,
   then played by UV offset with an unlit material.
3. Result: representation, distance and LOD all matter, and they interact;
   absence of motion did not change the judgement.
4. Impostors are the cheapest and best batched far choice, with the
   smallest footprint and no training; Gaussians take the middle band;
   meshes only earn their price near and large.
5. Practical: pick the far band by perceived similarity, not by polygon
   count; a sprite atlas with a stable bbox also stops the usual
   per-frame "breathing" and root-motion loss.

Paper claim, not ours: at the matched view impostor SSIM 0.965-0.971 with
LPIPS 0.006-0.030, while mesh stays preferred near. Those are their
captures, not our frame.

What it needs: no training and no weights for the impostor path. Our clip set
(AM5P with receipts), one cook pass that writes the atlas with the union box
and one scale per LOD, and a capture pair to score.

Where it lands: `crates/render/src/crowd_inst.rs` (the band constants
`CROWD_LOD_BANDS_M = [18, 45]` today) and
`crates/render/src/crowd_impostor.rs` (atlas cells). Row QA10, which already
owns the impostor path and asks for a receipt plus a capture.

Fixture and spike: 20k stands seats, static camera 3 m and 10 m from the front
row, web tier; baseline today's 18/45 m bands with the current octahedral
cells. Candidate: union-bbox atlas, band edge moved by the measured SSIM.
Metrics: SSIM against the near-mesh capture at each distance, instances and
draw calls, web-tier frame ms, receipt bytes twice equal.

Kill: SSIM under 0.95 at 10 m, or the web frame misses its bar, or the
capture cannot tell the two bands apart (then the card is taste and gets
rejected, not re-tuned).

Honest note: the paper compares representations, not engine bands, so the
0.95 bar is ours. It gives no per-instance cost on a 3050-class card.
CrowdSplat is rejected separately: 3,500 agents at 31 fps on a 4090 with CUDA
does not reach 20k on our tier, and 5000 characters at 3k Gaussians take
5.3 GB.

## P3 - clip alignment by Gromov-Wasserstein, offline

Source: arXiv 2505.19976, `MAMM`, sections 3.2 (FSUGW), 3.2.5
(coarse-to-fine), 4.5 (cross-skeleton), 7 (limits).

Method, five lines:

1. Cut the source motion and the control sequence into overlapping
   patches with stride one frame.
2. Build within-domain patch distance matrices only; no cross-domain
   mapping and no paired examples.
3. Minimise a fused semi-unbalanced Gromov-Wasserstein loss: one term
   keeps the aligned motion near the source, one makes its distance
   matrix look like the control's.
4. Alternate two steps, solve the transport plan then the aligned motion,
   coarse to fine, with soft keyframes when a pose must be pinned.
5. Output is a motion in our own domain, so a bone map or IK fit runs
   after it as usual.

Paper claim, not ours: 594 motion frames against 1441 control features
took 7 s; cross-skeletal motion-to-motion works without training. Their
limit: the whole control sequence is needed first, so it is not real time.

What it needs: no training, no weights, no service. It needs one source
clip, one control (another skeleton's motion, a sketch, audio features)
and an offline OT solver in Rust. The solver is float and lives outside
the tick, so the hash law is untouched.

Where it lands: `crates/endpoint/src/packs/anim.rs` (the offline verb) and
`crates/anim/src/contract.rs` (coverage report and refusal by bone name).
Row DOOR9, which already owns `anim.retarget` and asks for coverage 100%
on a Mixamo rig and a real preview frame.

Fixture and spike: one Mixamo walk clip (UE bone names, the DOOR9 case)
aligned to a walk AM5P cooked from the existing teacher through
`crates/anim/src/motion_bake.rs` (the tree has combat and stagger clips,
no walk). Baseline: the current linear time warp through
`BoneMap`/`retarget_cook`. Candidate: MAMM correspondence, then the same IK
fit. Metrics: bone coverage, foot slide mm, joint error, wall time, and the
preview frame's pixels.

Kill: foot slide or joint error worse than the linear warp on the pair; or
more than 60 s for a 20 s clip; or the solver's output needs a float in
the sim to be used. The last one is a hard stop: bake to AM5P first.

Honest note: there is no quantised version and no integer formulation in
the paper; that would be our work and is not proposed here. OT in Rust is
the main cost, and the paper's 7 s is Python on a desktop, not our number.

---

## Rejected, with the reason to keep

- 2406.00960 PDP: diffusion policy over RL expert rollouts. K denoise steps
  per action on CPU; 8 V100 for ~70 h (AMASS) or 4 A100 for ~24 h (KIT).
  Fails cost and tick budget. Re-open only with a GPU budget and a student.
- 2606.29148 GPC: FSQ motion vocabulary plus a GPT controller, 600+ hours of
  clips and RL. Their 99.98% reproduction is theirs. Cannot land here.
- 2501.17792 CrowdSplat: 3,500 agents at 31 fps on a 4090, CUDA, per-frame
  avatar templates. Not 20k, not our tier. Steal only its LOD distances.
- 2606.29209 AnyBody: PPO teacher distilled to a deterministic student with a
  unit-sphere latent, Isaac Lab, 200k iterations. Same pattern as PX5, but the
  compute does not fit. Steal the latent and "decoder is the motor prior".
- 2607.23227 INT8 dispatch-invariance: a determinism rule, not a method. Int8
  with int32 accumulators stays byte-identical across ARM kernels; the x86
  hazard is saturating int16 (PMADDUBSW). Our `.e2nn` already sums in
  saturating `i32` (`crates/nn/src/lib.rs`), so we comply. A two-CPU net-bytes
  test is worth one someday, not a card.

## What I could not verify

- Nothing was run: no GPU, no weights. Every number above is a paper claim,
  marked as such; nothing is reproduced.
- P1's cost on our DB is unknown (their DB is 184k poses, float search). Our
  integer scan may be cheaper, or the penalty path may dominate.
- P2 cannot be judged without a capture pair; the 0.95 SSIM bar is ours.
- DOOR9 has no cross-skeleton pair in the tree; sourcing the Mixamo clip is
  part of the spike. MAMM aligns, it does not train, so no data licence gate.
