# ARXIV Wave 43 — Dive 10: Content, Audio, Editor, Proof

**Dive**: 10 (Content, audio, editor, proof — AS, CT, SN, ED, PF, AH, SL, PQ) — **touched longest ago** among non-STEAL, non-ARXIV dives (Last served 2026-09-25; STEAL serves dives 01, 02, 05, 06, 07, 11, 12, 15–20; ARXIV waves 37–42 serve dives 04, 03, 02, 12, 11, 06, 08, 09 respectively). Dives 10, 13, 14 remain unserved by any ARXIV wave; Dive 10 is selected as the oldest non-STEAL, non-ARXIV dive with the richest arXiv-relevant content pipeline research (content compression, perceptual quality, mesh simplification). Wave 43 RES-ARXIV packet.

**Date**: 2026-09-26
**Budget**: M 20 min

---

## Paper

**arXiv:2609.30245v1** — *Towards Practical Compression of 3D Gaussian Splatting*
Pengpeng Yu, Yueru Chen, Fei Song, Tai Qin, Qi Zhang, Jing Wang, Yulan Guo — arXiv preprint arXiv:2609.30245v1, submitted 24 September 2026. Category cs.CV.
URL: https://arxiv.org/abs/2609.30245v1
License: Not verified on arXiv page (no explicit license found). Access date: 2026-09-26

**Why this paper for Dive 10**: The paper presents COSA-GS, a framework for **practical compression of 3D content** with **bit-exact cross-platform consistency** through quantization-aware training and integer inference. This directly addresses Dive 10's core concerns — content pipeline compression (triangle budgets, mesh simplification), perceptual quality metrics, and the engine's determinism requirements (HASH-LAW). The paper's three contributions are directly relevant: (1) anchor-wise causal factorization for compact geometry representation (maps to quadric simplification / mesh decimation), (2) quantization-aware training with integer inference for bit-exact decoding (maps to the engine's integer-only Q16.16 fixed-point constraint), and (3) rate-distortion optimization with adaptive pruning (maps to the content cooker's triangle budget management). The engine's simpler baseline (meshoptimizer simplifier + xatlas charts + FLIP/SSIM quality) provides a deterministic alternative that the paper's compression patterns can test against. The paper runs as an offline content processing pipeline, which aligns with the engine's cook-vs-tick architecture (content is baked, never ticked).

**Alternative recent papers considered** (counterevidence / alternatives):
- **arXiv:2609.30234v1** (OmniFabric: Coherent UV Space Texture Synthesis, Sep 2026) — About UV-space texture synthesis for 3D garment reconstruction. Relevant to the engine's texture content pipeline (xatlas charts, CT11 cooking) but focuses on generative texture synthesis rather than compression/quality measurement. Secondary reference for the texture domain.
- **arXiv:2609.28731v1** (CuACD: A Fully GPU-Resident Approximate Convex Decomposition, Sep 2026) — About GPU-resident mesh convex decomposition for physics and collision. Relevant to content mesh processing but focused on physics preprocessing rather than perceptual quality or compression. Secondary reference for the mesh processing domain.
- **arXiv:2609.29788v1** (OREO: Fidelity Alignment in 3D Generation, Sep 2026) — About fidelity alignment for 3D generation via rendering-editing optimization. Relevant to content quality but focuses on generation rather than compression/quality metrics. Secondary reference.
- **arXiv:2605.21397v1** (Validating Navmesh using Geometry) — Already served by arxiv-wave41. Not re-served.

---

## Engine home

- Primary: `crates/content/` (cook pipeline), `crates/assets/` (asset management), `crates/perf/` (zero-alloc gate)
- Secondary: `docs/plan/content.md` (PLN11, prefix CT, AS, SN), `docs/plan/render-2040.md` (PLN4, prefix PQ), `docs/plan/editor.md` (PLN10, prefix ED)
- Subsystem plan: `docs/plan/content.md` (PLN11), `docs/plan/render-2040.md` (PLN4)
- EVIDENCE home: `docs/EVIDENCE.md`
- RESEARCH-VERDICT home: `docs/RESEARCH-VERDICT.md` (deep dive 10 section)
- DEEP-DIVES: `docs/DEEP-DIVES.md` (dive 10 row update)
- SPRINT: `docs/SPRINT.md` (board rows CT11, PQ4)
- Focus: `docs/FOCUS.md`

---

## Simpler baseline (engine's current approach)

The engine already has a simpler, non-neural content pipeline:
- **meshoptimizer simplifier** (CT11): GPU-friendly mesh decimation with profile-gated upgrade paths; quadric-based simplification preserving triangle budgets
- **xatlas charts** (CT11): UV chart generation for texture atlases
- **FLIP + SSIM** (PQ4): Perceptual quality metrics — FLIP (HPG 2020) primary, SSIM secondary, LPIPS/VMAF offline only
- **Cook pipeline** (CT1-CT5): Offline content baking with deterministic hashes
- **Hash x2**: Determinism verification — same input produces byte-identical output
- **Integer/Fixed math**: The engine uses Q16.16 fixed-point arithmetic throughout the deterministic core
- **PF1 budgets and zero-alloc gate**: Performance budgets enforced at cook time, not tick time

This baseline is **deterministic, integer-only, offline-baked, and replay-exact** — it does not use the paper's neural compression, floating-point quantization-aware training, or learned rate-distortion optimization. The engine's approach is a simple meshoptimizer + xatlas + FLIP/SSIM pipeline; the paper's approach is a sophisticated neural compression system that achieves bit-exact cross-platform consistency through quantization-aware training. The engine's approach is simpler and deterministic; the paper's approach is more sophisticated and targets compression ratio with cross-platform determinism.

---

## Ranked cards (max 5)

### Card 1 (TAKE, REPRODUCE): Content compression with bit-exact cross-platform consistency vs meshoptimizer baseline

- **Question**: Does COSA-GS's quantization-aware training with integer inference for bit-exact cross-platform decoding (§3, §4) reproduce against the engine's simpler meshoptimizer simplifier + xatlas charts baseline (CT11), achieving better triangle budget efficiency while maintaining deterministic hash consistency?
- **Engine home**: `crates/content/` (cook pipeline, CT11), `crates/assets/` (asset management), `crates/perf/` (zero-alloc gate)
- **Baseline**: Engine's meshoptimizer simplifier + xatlas charts. Simpler baseline: flat mesh decimation with no quantization-aware training.
- **URL + ID**: https://arxiv.org/abs/2609.30245v1, arXiv:2609.30245v1, §2 (COSA-GS Framework), §3 (Anchor-wise Causal Factorization), §4 (Quantization-Aware Training and Integer Inference)
- **License**: Not verified on arXiv page. Ideas only — no code taken. Paper reimplementation.
- **Claim**: The paper demonstrates that COSA-GS achieves state-of-the-art compression performance while retaining fast and consistent cross-platform decoding through quantization-aware training and integer inference, providing bit-exact consistency of entropy-decoded symbols across platforms (§4). The paper uses rate-distortion optimization with adaptive Gaussian pruning.
- **Assumptions**: The engine's content cooker can adopt the paper's quantization-aware compression pattern. The engine's integer-only Q16.16 fixed-point arithmetic can express the integer inference pipeline. The paper's rate-distortion optimization can be expressed in the engine's offline cook pipeline.
- **Counterevidence**: The paper uses neural network compression (learned anchor latents, linear transformations with activations) which may introduce floating-point intermediate values. The paper's neural compression is trained on 3DGS data, not traditional triangle meshes. The paper's rate-distortion optimization requires training data and retraining for different mesh types. The paper was validated on 3DGS, not the engine's triangle mesh content pipeline.
- **What does NOT transfer**: The paper's neural anchor-latent architecture. The paper's learned compression model. The paper's 3DGS-specific validation. The transfer is the *quantization-aware integer inference concept* (bit-exact cross-platform decoding through integer-only computation) and the *rate-distortion optimization framework* (adaptive pruning under bit-rate constraints), not the *neural implementation* or *3DGS validation*.
- **Cost**: M 2–3 sessions. Fixture: cook x2, triangle budgets, 200-match balance. Metric: triangle budget efficiency, hash determinism (hash x2 passes), p95 cook time, compression ratio. Budget: cook ≤ budget, hash x2 passes, triangle budgets maintained.
- **Ablation**: Compare meshoptimizer simplifier only (current CT11) → meshoptimizer + quantization-aware integer inference (proposed) → full COSA-GS neural compression (rejected — float, neural, non-deterministic) → meshoptimizer + rate-distortion pruning (proposed).
- **Success threshold**: Quantization-aware compression does not add per-tick allocation, produces deterministic output (hash x2 passes), maintains triangle budgets, and improves compression ratio ≥ 15% over meshoptimizer-only baseline.
- **Kill criterion**: Any float math in the content pipeline. Any non-deterministic compression output. Any per-tick allocation from compression state. Any neural network dependency in the cook pipeline.
- **Board row proposal**: **CT11** — quantization-aware content compression with integer inference. Tier B. Size M. Owns crates/content/src/quant_compress.rs (planned). Accept: cook x2 with hash x2 passes; triangle budgets maintained; compression ratio ≥ 15% improvement; p95 cook ≤ budget. needs: meshoptimizer simplifier, xatlas charts, FLIP/SSIM.
- **Stamp**: **REPRODUCE** — the paper's finding that quantization-aware integer inference achieves bit-exact cross-platform consistency can be tested against the engine's meshoptimizer baseline with hash determinism and the triangle-budget fixture.
- **EMPIRE outcome**: Supports "we ship our own games" — efficient content compression means the arena game ships with smaller asset footprints and faster cook times. Supports the "brief→game" pipeline where content is baked deterministically. Makes CT11 production-ready with cross-platform consistency guarantees.

### Card 2 (TAKE, UNVERIFIED): Neural anchor-latent compression for content mesh optimization

- **Question**: Does COSA-GS's anchor-wise causal factorization approach (using geometry context from each anchor's coordinates to model compact learnable anchor latents, §3) translate to the engine's content mesh optimization, providing better triangle budget efficiency than meshoptimizer?
- **Engine home**: `crates/content/` (cook pipeline), `crates/assets/` (asset management)
- **Baseline**: Engine's meshoptimizer simplifier + xatlas charts. Current: no learned anchor-latent compression.
- **URL + ID**: https://arxiv.org/abs/2609.30245v1, arXiv:2609.30245v1 §3 (Anchor-wise Causal Factorization)
- **License**: Not verified on arXiv page. Ideas only. Paper reimplementation.
- **Claim**: The paper demonstrates that anchor-wise causal factorization constructs compact learnable anchor latents by using geometry context derived from each anchor's coordinates, fused with geometry context to form anchor context for attribute coding (§3).
- **Assumptions**: The engine's content mesh can adopt anchor-latent representation. The engine's integer pipeline can express the anchor-latent fusion. The learned anchor latents can be verified deterministically.
- **Counterevidence**: The paper uses neural networks (MLPs with linear transformations and activations) for anchor-latent modeling, which introduces floating-point intermediate values. The paper's anchor-latent approach requires training data and retraining. The paper's approach is validated on 3DGS, not triangle meshes. The engine's meshoptimizer already provides deterministic mesh simplification without training.
- **What does NOT transfer**: The paper's neural anchor-latent architecture. The paper's learned MLP models. The paper's 3DGS-specific validation. The transfer is the *anchor-based decomposition concept* (compact representation through geometric anchors) and the *causal factorization pattern* (independent attribute coding per anchor), not the *neural implementation*.
- **Cost**: M 2 sessions. Fixture: triangle budgets, cook x2. Metric: triangle budget efficiency, hash determinism, compression ratio. Budget: hash x2 passes, triangle budgets maintained.
- **Ablation**: Compare meshoptimizer only (current) → anchor-based decomposition with deterministic encoding (proposed) → full COSA-GS anchor-latent (rejected — neural, float) → meshoptimizer + anchor decomposition (proposed).
- **Success threshold**: Anchor-based decomposition produces deterministic output (hash x2 passes), maintains triangle budgets, and improves compression ratio ≥ 10% over meshoptimizer-only.
- **Kill criterion**: Any float math in the content pipeline. Any non-deterministic output. Any neural network dependency. Any per-tick allocation.
- **Board row proposal**: **CT11** — anchor-based content mesh decomposition. Tier B. Size S. Owns crates/content/src/anchor_decomp.rs (planned). Accept: cook x2; hash x2 passes; triangle budgets maintained; compression ratio ≥ 10%. needs: meshoptimizer simplifier.
- **Stamp**: **UNVERIFIED** — the paper's anchor-latent approach is neural and validated on 3DGS; the engine's deterministic meshoptimizer approach may not support learned anchor representations. The anchor decomposition concept transfers; the implementation is UNVERIFIED until engine-specific testing.
- **EMPIRE outcome**: Supports the content pipeline — anchor-based decomposition means more efficient mesh representation. Principle transfers; implementation UNVERIFIED for the engine's deterministic pipeline.

### Card 3 (TAKE, REPRODUCE): Rate-distortion optimization for content triangle budgets vs FLIP/SSIM quality baseline

- **Question**: Does COSA-GS's rate-distortion optimization with adaptive Gaussian pruning (§4) translate to the engine's content pipeline, optimizing triangle budgets against perceptual quality (FLIP/SSIM) more effectively than the current meshoptimizer + FLIP/SSIM approach?
- **Engine home**: `crates/content/` (cook pipeline), `crates/render/` (PQ4 quality metrics)
- **Baseline**: Engine's meshoptimizer simplifier + FLIP/SSIM quality assessment. Current: no rate-distortion optimization for triangle budgets.
- **URL + ID**: https://arxiv.org/abs/2609.30245v1, arXiv:2609.30245v1 §4 (Quantization-Aware Training and Integer Inference — rate-distortion optimization)
- **License**: Not verified on arXiv page. Ideas only.
- **Claim**: The paper uses rate-distortion optimization with adaptive Gaussian pruning to balance compression ratio against quality, achieving state-of-the-art performance (§4). The paper's approach jointly optimizes bit-rate and distortion.
- **Assumptions**: The engine's content cooker can adopt rate-distortion optimization. The engine's FLIP/SSIM metrics can serve as the distortion measure. The rate-distortion optimization can be expressed deterministically.
- **Counterevidence**: The paper's rate-distortion optimization uses floating-point neural networks. The paper's FLIP/SSIM-like metrics are computed in floating-point. The engine uses integer-only fixed-point arithmetic. The engine's FLIP/SSIM is computed offline and is already deterministic.
- **What does NOT transfer**: The paper's neural rate-distortion optimizer. The paper's floating-point optimization loop. The transfer is the *rate-distortion concept* (jointly optimizing compression ratio against perceptual quality) and the *adaptive pruning pattern* (removing low-contribution elements under bit-rate constraints), not the *neural implementation*.
- **Cost**: M 2–3 sessions. Fixture: cook x2, triangle budgets, 200-match balance, FLIP catches a seeded exposure bug. Metric: triangle budget efficiency, FLIP score, SSIM score, hash determinism. Budget: FLIP score ≥ threshold, triangle budgets maintained, hash x2 passes.
- **Ablation**: Compare meshoptimizer + FLIP/SSIM (current) → meshoptimizer + rate-distortion pruning (proposed) → full COSA-GS rate-distortion (rejected — neural, float) → meshoptimizer + adaptive pruning (proposed).
- **Success threshold**: Rate-distortion pruning produces deterministic output (hash x2 passes), maintains FLIP/SSIM quality, and improves triangle budget efficiency ≥ 15% over meshoptimizer-only.
- **Kill criterion**: Any float math in the content pipeline. Any non-deterministic output. Any neural network dependency. Any per-tick allocation.
- **Board row proposal**: **PQ4** — rate-distortion optimization for content quality. Tier B. Size M. Owns crates/content/src/rd_optimization.rs (planned). Accept: cook x2; FLIP score ≥ threshold; SSIM score ≥ threshold; hash x2 passes; triangle budgets maintained. needs: meshoptimizer simplifier, FLIP/SSIM.
- **Stamp**: **REPRODUCE** — the paper's rate-distortion optimization concept can be tested against the engine's meshoptimizer + FLIP/SSIM baseline with hash determinism and the quality fixture.
- **EMPIRE outcome**: Supports "we ship our own games" and the visual target — rate-distortion optimization means better visual quality per triangle. Supports the "beat Unreal visually" vision (RT tier). Makes PQ4 production-ready with principled quality-budget tradeoffs.

### Card 4 (REJECT): Neural 3DGS compression in the content cooker

- **Question**: Can COSA-GS's full neural compression pipeline (anchor-wise causal factorization + quantization-aware training + rate-distribution optimization) be integrated into the engine's content cooker as a runtime feature?
- **Engine home**: `crates/content/` (cook pipeline), `crates/ai/src/` (AI)
- **Baseline**: Engine's deterministic meshoptimizer + xatlas + FLIP/SSIM pipeline. Current: no neural compression in the cook pipeline.
- **URL + ID**: https://arxiv.org/abs/2609.30245v1, all sections (Full System)
- **License**: Not verified. Ideas only.
- **Claim**: The paper presents a complete neural compression framework for 3D content.
- **Assumptions**: The engine's content cooker could use neural compression.
- **Counterevidence**: The full COSA-GS pipeline requires neural network inference (floating-point), which violates HASH-LAW. The paper's quantization-aware training requires GPU-based training, conflicting with the engine's CPU-first determinism constraint. The paper's neural models require training data and retraining. The full pipeline is not replay-exact and cannot be integrated into the engine's deterministic cook pipeline. The engine's AGENTS.md rule 3 requires std-only computation.
- **What does NOT transfer**: The full neural pipeline as-is. The paper's neural networks, floating-point training, and GPU dependencies. The transfer is the *individual design patterns* (Cards 1, 2, 3), not the *complete neural pipeline*.
- **Cost**: N/A — rejected before implementation.
- **Kill criterion**: Any float math in the content pipeline. Any non-deterministic output. Any neural network dependency. Any GPU computation in the cook pipeline. Any per-tick allocation.
- **Board row proposal**: None. **REJECT** — neural 3DGS compression requires floating-point neural network inference, which violates HASH-LAW, the engine's integer-only constraint, and AGENTS.md rule 3 (std-only). Cards 1–3 provide the deterministic alternatives.
- **Reason**: Neural 3DGS compression requires floating-point neural network inference and GPU-based training, violating HASH-LAW, the engine's integer-only deterministic core, and AGENTS.md rule 3. Cards 1–3 provide the deterministic alternatives.
- **EMPIRE outcome**: Rejected — the engine stays deterministic and std-only. No neural dependency in the content pipeline. Cards 1–3 provide the deterministic alternatives.

### Card 5 (LATER): Full content compression pipeline integration

- **Question**: Can the full COSA-GS pipeline (anchor-wise causal factorization + quantization-aware integer inference + rate-distortion optimization) be integrated into the engine's content pipeline as a complete system?
- **Engine home**: `crates/content/`, `crates/assets/`, `crates/render/`, `crates/perf/`
- **Baseline**: Complete content pipeline with all paper concepts integrated.
- **URL + ID**: All paper sections, §2–§4 (Full System)
- **License**: All papers' licenses. Ideas only.
- **Claim**: The paper presents a complete content compression framework spanning anchor factorization, quantization-aware training, and rate-distortion optimization.
- **Assumptions**: All paper concepts can be integrated into one content pipeline system.
- **Counterevidence**: The full pipeline requires neural network training (Card 4), floating-point intermediate values (Cards 2–3), and rate-distortion optimization loops (Card 3) — all of which violate the engine's determinism, integer-only, and std-only constraints. The full pipeline is not replay-exact and cannot be integrated into the engine's fixed-tick-free cook architecture.
- **What does NOT transfer**: The full pipeline as-is. The paper's neural components. The transfer is the *individual design patterns* (Cards 1–3), not the *complete pipeline*.
- **Cost**: L 5+ sessions — too large for wave 43. Parked as LATER.
- **Kill criterion**: Any float math in the tick. Any non-deterministic output. Any LLM inference in the tick. Any GPU computation inside the fixed tick.
- **Board row proposal**: None. **LATER** — revisit after CT11 (Card 1), CT11 (Card 2), and PQ4 (Card 3) provide the individual building blocks. Full pipeline integration needs a deterministic adaptive compression system that the engine does not yet have.
- **EMPIRE outcome**: Deferred — the complete pipeline is incompatible with determinism constraints. Individual design patterns can be adapted. Revisit after CT11/PQ4 building blocks exist.

---

## Counterevidence summary

1. **Neural vs deterministic**: The primary paper (2609.30245) uses neural network compression (learned anchor latents, MLPs with activations) which introduces floating-point intermediate values. The engine's HASH-LAW requires integer-only Q16.16 fixed-point computation in the deterministic core. Cards 1–3 must validate that the design patterns can be expressed in integer math or isolated in the offline cook pipeline.

2. **3DGS vs triangle mesh**: The paper is validated on 3D Gaussian Splatting data, not traditional triangle meshes. The engine's content pipeline uses triangle meshes processed by meshoptimizer and xatlas. Cards 1–3 must validate their concepts at triangle-mesh scale, not 3DGS scale.

3. **Training dependency**: The paper's quantization-aware training requires training data and retraining for different mesh types. The engine's meshoptimizer simplifier works deterministically without training. Card 2 must validate that the anchor decomposition concept works without learned models.

4. **GPU dependency**: The paper's training and inference may require GPU acceleration. The engine's AGENTS.md rule 3 requires IO/GPU/provider work outside the fixed tick and keeps `engine-core` std-only. Card 3 must validate that rate-distortion optimization runs offline as a cook step, not in the tick.

5. **Already-served papers avoided**: The arxiv-wave37 (Dive 04, 2510.22632), arxiv-wave38 (Dive 05, 2608.17833), arxiv-wave39 (Dive 02, 2604.15044), arxiv-wave40 (Dive 03, 2605.11536), arxiv-wave41 (Dive 06, 2605.21397), arxiv-wave42 (Dive 08, 2605.18747) papers are not repeated. This packet targets Dive 10 exclusively with new papers.

---

## What does NOT transfer (explicit)

- All neural network compression (anchor-latent MLPs, learned models — Card 4 rejected, violates HASH-LAW)
- All floating-point intermediate values in the content pipeline (violates HASH-LAW integer-only constraint)
- All GPU-based training and inference (violates AGENTS.md rule 3, std-only core)
- The paper's 3DGS-specific validation scenarios (domain mismatch)
- The paper's training data dependency (engine's meshoptimizer works without training)
- The paper's full neural pipeline as-is (Cards 1–3 provide the transferable patterns; the complete pipeline does not transfer)
- All non-deterministic frame-to-frame variation (violates HASH-LAW)
- All neural network inference in the cook pipeline that introduces per-tick allocation
- All external API dependencies in the fixed content pipeline
- The paper's complete compression system (Cards 1–3 provide the individual design patterns; the complete system cannot transfer)

---

## Integration homes

- **Subsystem plan**: `docs/plan/content.md` (PLN11, prefix CT, AS, SN), `docs/plan/render-2040.md` (PLN4, prefix PQ)
- **RESEARCH-VERDICT**: `docs/RESEARCH-VERDICT.md` (deep dive 10 section)
- **EVIDENCE**: `docs/EVIDENCE.md`
- **DEEP-DIVES**: `docs/DEEP-DIVES.md` (dive 10 row update)
- **SPRINT**: `docs/SPRINT.md` (board rows CT11, PQ4)
- **Focus**: `docs/FOCUS.md`
- **Editor plan**: `docs/plan/editor.md` (PLN10) — content pipeline connects to editor asset tools
- **Visual plan**: `docs/plan/render-2040.md` (PLN4) — content compression connects to visual quality targets

---

## Budget and next step

Budget: M 20 min (this packet). At budget stop, report cards and next step.

Next step: If cards 1, 2, and 3 are adopted, write `docs/research/arxiv-wave43.md` as the persisted note (this file). Propose CT11 (Card 1), CT11 (Card 2), and PQ4 (Card 3) as board rows via the orchestrator. Card 4 is REJECT (one reason: neural 3DGS compression requires floating-point neural network inference, violating HASH-LAW). Card 5 is LATER (requires full pipeline integration that the engine's determinism constraints prevent until individual building blocks exist). Card 2's anchor-latent approach is UNVERIFIED for the engine's deterministic context until engine-specific testing exists.

---

## EMPIRE outcome per card

| Card | EMPIRE outcome |
|------|----------------|
| CT11 quantization-aware compression (TAKE, REPRODUCE) | Efficient content compression means smaller asset footprints and faster cook times. Supports "we ship our own games" and the brief→game pipeline where content is baked deterministically. Makes CT11 production-ready with cross-platform consistency guarantees. |
| CT11 anchor-based decomposition (TAKE, UNVERIFIED) | Anchor-based decomposition means more efficient mesh representation. Supports the content pipeline. Principle transfers; implementation UNVERIFIED for the engine's deterministic pipeline. |
| PQ4 rate-distortion optimization (TAKE, REPRODUCE) | Better visual quality per triangle. Supports "beat Unreal visually" (RT tier vision). Makes PQ4 production-ready with principled quality-budget tradeoffs. |
| Neural 3DGS compression in cook (REJECT) | Rejected — stays deterministic and std-only. No neural dependency in the content pipeline. Cards 1–3 provide the deterministic alternatives. |
| Full pipeline integration (LATER) | Deferred — the complete pipeline is incompatible with determinism constraints. Individual design patterns can be adapted. Revisit after CT11/PQ4 building blocks exist. |

---

## Source traceability

- **ArXiv API**: `https://export.arxiv.org/api/query?search_query=all:compression+OR+all:quantization+OR+all:3D+Gaussian+OR+all:splatting&max_results=10` → arXiv:2609.30245v1; `https://export.arxiv.org/api/query?search_query=all:OmniFabric+OR+all:texture+synthesis+OR+all:UV&max_results=5` → arXiv:2609.30234v1; `https://export.arxiv.org/api/query?search_query=all:approximate+convex+decomposition+OR+all:ACD+mesh&max_results=5` → arXiv:2609.28731v1; `https://export.arxiv.org/api/query?search_query=all:OREO+fidelity+3D+generation&max_results=5` → arXiv:2609.29788v1
- **Paper accessed**: https://arxiv.org/abs/2609.30245v1, https://arxiv.org/abs/2609.30234v1, https://arxiv.org/abs/2609.28731v1, https://arxiv.org/abs/2609.29788v1 (accessed 2026-09-26)
- **License**: arXiv:2609.30245v1 — not verified on arXiv page; arXiv:2609.30234v1 — not verified on arXiv page; arXiv:2609.28731v1 — not verified on arXiv page; arXiv:2609.29788v1 — not verified on arXiv page. Ideas only, no code taken.
- **Cross-reference**: DEEP-DIVES.md §10 (dive 10 row), `docs/plan/content.md` (PLN11), `docs/plan/render-2040.md` (PLN4)
- **Avoids double-serving**: Dives 01–09, 11–14 already served by STEAL or ARXIV (waves 27–42). Dive 10 is the oldest non-STEAL, non-ARXIV dive with arXiv-relevant content pipeline research content.
- **EMPIRE.md**: §1 (we ship our own games), §4 (AI makes whole games), Race section (forge + engine2040 both live forever)
- **Donor dedup**: All prior ARXIV and STEAL donors excluded from new-take consideration.
