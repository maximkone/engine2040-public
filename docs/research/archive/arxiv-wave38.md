# ARXIV Wave 38 — Dive 05: Physics and destruction at scale

**Dive**: 05 (Physics and destruction at scale) — **touched longest ago** (Last served 2026-09-25, the earliest among non-STEAL dives; STEAL serves dives 11, 12, 15–20 only). Not served by any prior ARXIV wave (waves 27, 30, 31, 33, 34, 35, 36, 37 serve dives 16, 09, 20, 12, 11, 11, 12, 04 respectively). This packet avoids double-serving. Wave 38 RES-ARXIV packet.

**Date**: 2026-09-26
**Budget**: M 20 min

---

## Paper

**arXiv:2608.17833v1** — *Variational r-Adaptive Cloth Simulation*
Jiahao Wen, Zhen Chen, Jernej Barbič, Danny M. Kaufman — arXiv preprint arXiv:2608.17833v1, submitted 18 Aug 2026. Category cs.GR.
URL: https://arxiv.org/abs/2608.17833v1
License: Not stated on arXiv page (paper under arXiv distribution). Code/data/weights: UNVERIFIED — paper provides a simulation method, not code. Ideas reimplemented, never copied.
Access date: 2026-09-26

**Why this paper for Dive 05**: The paper presents the first r-adaptive method for simulating cloth dynamics and statics with frictional contact in modern cloth pipelines. It directly addresses the challenge of high effective spatial resolution for wrinkles, folds, buckling, and sharp contact features — exactly the kind of cloth simulation that the engine's simpler CPU verlet cloth baseline (PX2, `crates/physics/src/cloth.rs`, 1031 lines, Q16.16 verlet, Jakobsen solve) cannot handle efficiently. The paper's r-adaptive approach uses incremental-potential optimization with quality regularization, while the engine's baseline uses a simpler fixed-mesh velocity servo approach. This is a direct comparison between adaptive variational cloth simulation and a fixed-mesh CPU verlet baseline, which maps to Dive 05's core concerns (cloth, contacts, collision) and its kill criterion ("NaN, tunneling, runtime fracture in the tick without DX5's budget"). The paper reports a 3–6× speedup over prior optimal ITR under equal vertex-count and time-budget constraints, which can be tested against the engine's 0.5 ms cloth bar.

**Alternative recent papers considered** (counterevidence / alternatives):
- **arXiv:2609.21201v1** (Adaptive Mesh Coarsening for Efficient Phase-Field Fracture Simulations, Sept 2026) — About adaptive mesh coarsening for phase-field fracture. Relevant to the engine's destruction system (DX2 BSP bake, DX5 runtime fracture) but focused on fracture, not cloth. Secondary reference for the destruction domain.
- **arXiv:2506.18867v4** (Efficient B-Spline Finite Elements for Cloth Simulation, Jun 2025) — About B-spline FEM for cloth. Relevant to cloth simulation but uses higher-order FEM which is float-based and computationally heavier. The engine's integer-only approach may not support B-spline bases directly.
- **arXiv:2604.15513v1** (Divide and Truncate: A Penetration and Inversion Free Framework for Coupled Multi-physics Systems, Apr 2026) — About penetration-free coupling for multi-physics systems. Relevant to the engine's cloth-ragdoll coupling (PX12 DAT coupling) but focused on general multi-physics, not cloth specifically.

---

## Engine home

- Primary: `crates/physics/src/cloth.rs`, `crates/destruction/src/bonded.rs`, `crates/destruction/src/csg.rs`
- Secondary: `docs/plan/physics.md` (PLN5, prefixes PX, DX), `docs/plan/physics.md` §5 Donors
- Subsystem plan: `docs/plan/physics.md` (PLN5)
- EVIDENCE home: `docs/EVIDENCE.md`
- RESEARCH-VERDICT home: `docs/RESEARCH-VERDICT.md` (deep dive 05 section)
- DEEP-DIVES: `docs/DEEP-DIVES.md` (dive 05 row update)
- SPRINT: `docs/SPRINT.md` (board rows DX8, PX12, DX5)
- Focus: `docs/FOCUS.md`

---

## Simpler baseline (engine's current approach)

The engine already has a simpler, non-adaptive baseline for cloth and destruction:
- **Cloth CPU verlet** (PX2): Fixed-mesh Q16.16 verlet with Jakobsen solve, presentation-only, 0.5 ms bar. Already implemented in `crates/physics/src/cloth.rs` (1031 lines). Unit-tested and integrated.
- **Bonded destruction** (DX1): Bonded chunks with BSP bake, caps 16 per event and 1,024 total. Already implemented in `crates/destruction/src/bonded.rs`. Unit-tested and integrated.
- **CSG fracture bake** (DX2): BSP plane-split with digest x2. Already implemented in `crates/destruction/src/csg.rs`.
- **BSP bake** (DX2): The baseline destruction approach uses baked BSP planes — a static, pre-computed approach that does not adapt at runtime.
- **Hash x2**: Determinism verification — same input produces byte-identical physics output.
- **Integer/Fixed math**: The engine uses Q16.16 fixed-point arithmetic throughout physics; no floating-point math in the tick.

This baseline is **deterministic, fixed-mesh, integer-only, and replay-exact** — it does not use the r-adaptive incremental-potential optimization, quality regularization, or dynamic mesh adaptation from the paper, which are variational methods requiring continuous optimization. The engine's approach is fixed-mesh and procedural; the paper's approach is adaptive and variational.

---

## Ranked cards (max 5)

### Card 1 (TAKE, reproduce): Variational r-adaptive cloth vs CPU verlet cloth baseline

- **Question**: Does Wen et al.'s r-adaptive cloth simulation with frictional contact (incremental-potential optimization with quality regularization, 3–6× speedup over prior ITR) reproduce against the engine's simpler fixed-mesh CPU verlet cloth baseline (PX2), producing higher-fidelity cloth at or under the 0.5 ms tick budget?
- **Engine home**: `crates/physics/src/cloth.rs` (PX2, Q16.16 verlet, Jakobsen solve, 0.5 ms bar), `crates/physics/src/` contact system
- **Baseline**: Engine's fixed-mesh CPU verlet cloth (PX2). Simpler baseline: flat fixed-mesh verlet with no adaptive resolution.
- **URL + ID**: https://arxiv.org/abs/2608.17833v1, arXiv:2608.17833v1, §3 (r-adaptive method), §4 (Experiments)
- **License**: Paper under arXiv distribution. Ideas only — no code taken. Paper reimplementation.
- **Claim**: The first r-adaptive method for cloth simulation with frictional contact achieves 3–6× speedup over prior optimal ITR under equal vertex-count and time-budget constraints, with higher visual fidelity than fixed meshes (§4, Experiments on challenging frictional-contact scenarios).
- **Assumptions**: The engine's cloth system can support adaptive mesh refinement within the fixed-tick framework. The paper's quality regularization and degeneracy-activated regularizer can be expressed in integer/fixed-point arithmetic.
- **Counterevidence**: The paper's IP optimization uses floating-point continuous methods; the engine's HASH-LAW requires integer-only fixed-point computation. The paper's adaptive mesh refinement may introduce per-tick allocation, violating the engine's determinism law. The paper's frictional contact model may not be replay-exact.
- **What does NOT transfer**: The paper's floating-point IP optimization solver. The paper's dynamic mesh refinement pipeline (requires runtime memory allocation). The paper's continuous contact model. The transfer is the *r-adaptivity concept* (adaptive resolution where needed) and *quality regularization pattern*, not the *floating-point solver* or *dynamic mesh pipeline*.
- **Cost**: M 2–4 sessions. Fixture: 8 fighters with cloth capes and frictional contact scenarios; a 10x stiffness plus 2x wind blow-up drill; 8-way insertion permutation. Metric: cloth fidelity (capture-based playtest), p95 tick time, hash determinism. Budget: ≤ 0.5 ms per cloth tick.
- **Ablation**: Compare fixed-mesh CPU verlet (current PX2) → fixed-mesh with quality regularization (proposed) → r-adaptive cloth with dynamic refinement (proposed) → full paper system (rejected — float, non-deterministic, requires allocation).
- **Success threshold**: r-adaptive cloth does not add per-tick allocation, produces deterministic output (hash x2 passes), and improves visual fidelity in capture-based playtest without raising tick time above 0.5 ms.
- **Kill criterion**: Any float math in the cloth tick. Any non-deterministic cloth output (hash drift). Any per-tick allocation from cloth processing. Any cloth simulation exceeding 0.5 ms p95.
- **Board row proposal**: **PX12** — r-adaptive cloth simulation with frictional contact. Tier A. Size M. Owns crates/physics/src/cloth_adaptive.rs (planned). Accept: frictional-contact scenarios; hash x2 passes; p95 cloth tick ≤ 0.5 ms; visual fidelity improvement in playtest. needs: PX2, PX10.
- **Stamp**: **REPRODUCE** — the paper's finding that r-adaptive cloth with quality regularization achieves 3–6× speedup over prior ITR can be tested against the engine's deterministic CPU verlet cloth baseline with hash determinism and the 0.5 ms budget.
- **EMPIRE outcome**: Supports "we ship our own games" — better cloth simulation means the Rome at war game ships with polished, believable character movement and environment cloth. Supports the fighter and arena genres.

### Card 2 (TAKE, reproduce): Phase-field fracture with adaptive mesh coarsening vs BSP bake baseline

- **Question**: Does Dahal et al.'s adaptive mesh coarsening framework for phase-field fracture (replacing fractured regions with coarse crack bands, preserving essential mechanical behavior) translate to the engine's destruction system, improving runtime fracture efficiency against the simpler BSP bake baseline (DX2)?
- **Engine home**: `crates/destruction/src/csg.rs` (DX2, BSP plane-split), crates/destruction/src/fracture.rs (planned) (DX5, runtime Voronoi fracture — missing)
- **Baseline**: Engine's BSP bake (DX2) — pre-computed fracture planes. Simpler baseline: flat BSP bake with no adaptive coarsening.
- **URL + ID**: https://arxiv.org/abs/2609.21201v1, arXiv:2609.21201v1, §3 (Adaptive coarsening framework), §4 (Experiments)
- **License**: Paper under arXiv distribution. Ideas only.
- **Claim**: The paper presents an adaptive mesh coarsening framework that replaces the fractured region in the crack wake with a coarse crack band, reducing computational cost while preserving essential mechanical behavior. The framework is implemented in parallel within FEniCSx and demonstrated on benchmark problems involving crack propagation and nucleation under quasi-static and dynamic loading.
- **Assumptions**: The engine's destruction system can support adaptive mesh coarsening within the fixed-tick framework. The paper's physics-based refinement indicator (material strength surface violation) can be expressed in integer/fixed-point arithmetic.
- **Counterevidence**: The paper uses FEniCSx (Python/C++ finite element framework) with floating-point methods. The engine uses integer-only Q16.16 fixed-point arithmetic. The paper's parallel FEniCSx implementation is not compatible with the engine's deterministic single-threaded tick. The paper's phase-field fracture model requires continuous field variables.
- **What does NOT transfer**: The paper's FEniCSx implementation. The paper's floating-point phase-field model. The paper's parallel assembly. The transfer is the *adaptive coarsening concept* (coarse crack bands preserving mechanical behavior) and *physics-based refinement indicator*, not the *FEniCSx pipeline* or *floating-point field equations*.
- **Cost**: M 2 sessions. Fixture: 8 fighters with destruction on; 10x stiffness plus 2x wind blow-up drill. Metric: destruction performance, hash determinism, p95 tick time. Budget: ≤ 1 ms per destruction tick.
- **Ablation**: Compare BSP bake (current DX2) → BSP bake with adaptive coarsening (proposed) → full phase-field fracture (rejected — float, non-deterministic, requires FEniCSx).
- **Success threshold**: Adaptive coarsening does not add per-tick allocation, produces deterministic output (hash x2 passes), and improves destruction efficiency without raising tick time above budget.
- **Kill criterion**: Any float math in the destruction tick. Any non-deterministic destruction output. Any per-tick allocation. Any dependency on FEniCSx or external finite element libraries.
- **Board row proposal**: **DX5** — adaptive mesh coarsening for runtime fracture. Tier A. Size S. Owns crates/destruction/src/adaptive_coarsening.rs (planned). Accept: runtime fracture scenarios; hash x2 passes; no per-tick allocation. needs: DX2, DX4.
- **Stamp**: **REPRODUCE** — the paper's adaptive mesh coarsening concept can be adapted as a deterministic coarsening pattern for the engine's BSP bake baseline, tested against the 8-fighter destruction fixture.
- **EMPIRE outcome**: Supports "we ship our own games" — better runtime fracture means the Rome at war game ships with polished, believable environmental destruction. Supports the arena scale vision.

### Card 3 (REJECT): B-spline FEM cloth simulation

- **Question**: Can the paper's B-spline FEM cloth simulation approach (quadratic B-spline surfaces, globally C¹-continuous, 2× speedup over linear FEM) be reproduced as the engine's cloth simulation method?
- **Engine home**: `crates/physics/src/cloth.rs` (PX2)
- **Baseline**: Engine's fixed-mesh CPU verlet cloth (Q16.16, integer-only). Current: Q16.16 verlet with Jakobsen solve.
- **URL + ID**: https://arxiv.org/abs/2506.18867v4, arXiv:2506.18867v4, §3 (B-spline FEM pipeline), §4 (Experiments)
- **License**: Paper under arXiv distribution. Ideas only.
- **Claim**: The paper presents an efficient B-spline FEM for cloth simulation with quadratic B-spline basis functions, achieving globally C¹-continuous displacement fields and 2× speedup over linear FEM.
- **Assumptions**: The engine could use B-spline basis functions for cloth simulation instead of linear elements.
- **Counterevidence**: The paper uses floating-point B-spline evaluation and assembly. The engine's HASH-LAW requires integer-only computation. B-spline basis functions require continuous evaluation that is incompatible with fixed-point arithmetic. The paper's accelerated Hessian assembly and partial factorization are float-based linear algebra operations that cannot be expressed deterministically in integer math.
- **What does NOT transfer**: The paper's B-spline basis functions and evaluation. The paper's floating-point Hessian assembly. The paper's linear solver. The transfer is the *concept* of higher-order continuity for cloth accuracy, not the *mathematical implementation*.
- **Cost**: N/A — rejected before implementation.
- **Kill criterion**: Any float math in the cloth tick. Any non-deterministic cloth output. Any dependency on floating-point linear algebra.
- **Board row proposal**: None. **REJECT** — B-spline FEM uses floating-point basis functions and linear algebra that violate the engine's HASH-LAW determinism law and integer-only constraint. The engine's Q16.16 fixed-point CPU verlet approach is the correct mechanism.
- **Reason**: B-spline FEM requires floating-point basis function evaluation and continuous derivative computation, violating HASH-LAW and the integer-only physics constraint. The fixed-mesh CPU verlet baseline is deterministic and replay-exact.
- **EMPIRE outcome**: Rejected — the engine stays deterministic and provider-agnostic; no floating-point cloth simulation dependency.

### Card 4 (TAKE, UNVERIFIED): DAT penetration-free coupling for cloth-ragdoll

- **Question**: Does Chen et al.'s Divide and Truncate framework (penetration-free coupling for multi-physics systems, material-agnostic, solver-agnostic) translate to the engine's cloth-ragdoll coupling system (PX12 DAT coupling)?
- **Engine home**: `crates/physics/src/ragdoll.rs` (PX1, powered ragdoll), `crates/physics/src/cloth.rs` (PX2), `crates/physics/src/grapple.rs` (PX3)
- **Baseline**: Engine's current ragdoll-cloth coupling is a simple velocity-based approach. Simpler baseline: no coupling, ragdoll and cloth move independently.
- **URL + ID**: https://arxiv.org/abs/2604.15513v1, arXiv:2604.15513v1, §2 (DAT framework), §3 (Planar-DAT)
- **License**: Paper under arXiv distribution. Ideas only.
- **Claim**: The paper presents a unified framework for coupling multi-physics systems through penetration-free collision handling, guaranteeing penetration-free contact resolution by partitioning ambient space and truncating displacements. The framework is material-agnostic and solver-agnostic.
- **Assumptions**: The engine's cloth-ragdoll coupling can adopt the DAT partitioning and truncation approach. The engine's integer-only approach can express the space partitioning and displacement truncation.
- **Counterevidence**: The paper's DAT framework uses floating-point geometry and continuous displacement fields. The engine's Q16.16 fixed-point approach may not support the paper's continuous partitioning. The paper's material-agnostic framework is designed for general multi-physics, not specifically for game cloth-ragdoll coupling. The paper's solver-agnostic post-processing step may introduce per-tick allocation.
- **What does NOT transfer**: The paper's floating-point geometry partitioning. The paper's continuous displacement truncation. The paper's multi-physics coupling architecture. The transfer is the *penetration-free coupling concept* (partitioning space to prevent interpenetration) and *truncation principle*, not the *floating-point implementation*.
- **Cost**: M 2 sessions. Fixture: cloth-cape touching ragdoll mesh scenarios. Metric: penetration count, p95 tick time, hash determinism. Budget: ≤ 0.5 ms per coupling tick.
- **Ablation**: Compare no coupling (current) → simple velocity coupling (PX12 baseline) → DAT partitioning coupling (proposed) → full DAT framework (rejected — float, non-deterministic, complex).
- **Success threshold**: DAT coupling eliminates penetration without adding per-tick allocation, produces deterministic output (hash x2 passes), and does not raise tick time above 0.5 ms.
- **Kill criterion**: Any float math in the coupling tick. Any non-deterministic coupling output. Any per-tick allocation. Any dependency on external geometry libraries.
- **Board row proposal**: **PX12** — penetration-free cloth-ragdoll coupling. Tier B. Size S. Owns crates/physics/src/dat_coupling.rs (planned). Accept: cloth-ragdoll contact scenarios; zero penetration; hash x2 passes; p95 ≤ 0.5 ms. needs: PX1, PX2.
- **Stamp**: **UNVERIFIED** — the paper's DAT framework uses floating-point geometry and continuous methods; the engine's integer-only approach may not support the same level of penetration-free coupling. The penetration-free concept transfers; the implementation is UNVERIFIED until engine-specific testing.
- **EMPIRE outcome**: Supports "we ship our own games" — better cloth-ragdoll coupling means combat interactions with capes and cloth look more believable. Supports the fighter genre core.

### Card 5 (LATER): Full physics simulation pipeline integration

- **Question**: Can the full pipeline from all four papers (r-adaptive cloth + phase-field fracture + B-spline FEM + DAT coupling) be integrated into the engine's physics system as a complete adaptive physics framework?
- **Engine home**: `crates/physics/src/`, `crates/destruction/src/`, `crates/physics/src/cloth.rs`, `crates/destruction/src/csg.rs`
- **Baseline**: Complete physics system with all four paper concepts integrated.
- **URL + ID**: All four papers, §3–§5 (Full Systems)
- **License**: Papers under arXiv distribution. Ideas only.
- **Claim**: The four papers together provide a complete physics simulation framework covering cloth, fracture, and coupling.
- **Assumptions**: All paper concepts can be integrated into one deterministic physics system.
- **Counterevidence**: The full pipeline requires floating-point optimization, dynamic mesh refinement, B-spline basis functions, and continuous geometry — all of which violate the engine's determinism, integer-only, and fixed-mesh constraints. The full pipeline is not replay-exact and cannot be integrated into the engine's fixed tick.
- **What does NOT transfer**: The full pipeline as-is. The transfer is the *individual design patterns* (Cards 1, 2, 4), not the *complete pipeline*.
- **Cost**: L 5+ sessions — too large for wave 38. Parked as LATER.
- **Kill criterion**: Any float math in the tick. Any non-deterministic output. Any GPU dependency. Any external data dependency.
- **Board row proposal**: None. **LATER** — revisit after PX12 (Card 1), DX5 (Card 2), and PX12 (Card 4) provide the individual building blocks. Full pipeline integration needs a deterministic adaptive mesh system that the engine does not yet have.
- **EMPIRE outcome**: Deferred — the complete pipeline is incompatible with determinism constraints. Individual design patterns can be adapted; the complete pipeline cannot.

---

## Counterevidence summary

1. **Floating-point vs integer-only**: All four papers use floating-point methods (IP optimization, phase-field FEM, B-spline evaluation, continuous geometry). The engine's HASH-LAW requires integer-only Q16.16 fixed-point computation. Cards 1, 2, 4 must validate that their design patterns can be expressed in integer math.

2. **Dynamic vs fixed mesh**: Cards 1 and 2 propose dynamic mesh adaptation/refinement, which requires runtime memory allocation and is incompatible with the engine's fixed-mesh determinism. The engine's approach is fixed-mesh with pre-baked topology. The r-adaptivity concept (adapt resolution where needed) transfers; the dynamic mesh pipeline does not.

3. **FEniCSx and external dependencies**: Card 2's paper uses FEniCSx (Python/C++ finite element framework) with external linear algebra. The engine uses no external dependencies for physics computation. Card 2 must validate that the adaptive coarsening concept works without FEniCSx.

4. **Continuous vs discrete geometry**: Cards 3 and 4 use continuous geometry (B-spline surfaces, continuous partitioning). The engine uses discrete, integer-based geometry (Q16.16 fixed-point). The continuous methods may not directly translate to the discrete engine representation.

5. **Already-served papers avoided**: The arxiv-wave30 (SP12, LW7, NV3), arxiv-wave33 (Dive 12), arxiv-wave34/35 (Dive 11), arxiv-wave36 (Dive 12), and arxiv-wave37 (Dive 04) papers are not repeated. This packet targets Dive 05 exclusively with new papers.

---

## What does NOT transfer (explicit)

- All floating-point optimization solvers (IP, Newton-Raphson, Hessian assembly)
- All dynamic mesh refinement/coarsening pipelines (requires runtime allocation)
- All B-spline basis function evaluation and C¹ continuity computation
- All FEniCSx / external finite element framework dependencies
- All continuous geometry partitioning methods
- The paper's continuous contact models (frictional contact with continuous compliance)
- Any paper claim that requires external libraries or non-deterministic computation
- The full multi-physics pipeline as-is (Cards 1–4 provide the transferable patterns; the complete pipeline does not transfer)

---

## Integration homes

- **Subsystem plan**: `docs/plan/physics.md` (PLN5, prefix PX, DX)
- **RESEARCH-VERDICT**: `docs/RESEARCH-VERDICT.md` (deep dive 05 section)
- **EVIDENCE**: `docs/EVIDENCE.md`
- **DEEP-DIVES**: `docs/DEEP-DIVES.md` (dive 05 row update)
- **SPRINT**: `docs/SPRINT.md` (board rows DX8, PX12, DX5)
- **Focus**: `docs/FOCUS.md`

---

## Budget and next step

Budget: M 20 min (this packet). At budget stop, report cards and next step.

Next step: If cards 1 and 2 are adopted, write `docs/research/arxiv-wave38.md` as the persisted note (this file). Propose PX12 (Card 1), DX5 (Card 2), and PX12 (Card 4) as board rows via the orchestrator. Card 3 is REJECT (one reason: B-spline FEM uses floating-point basis functions violating HASH-LAW). Card 5 is LATER (requires full pipeline integration that the engine's determinism constraints prevent until individual building blocks exist). Card 4's DAT coupling claim is UNVERIFIED for the engine's integer-only context until playtest data exists.

---

## EMPIRE outcome per card

| Card | EMPIRE outcome |
|------|----------------|
| PX12 r-adaptive cloth (TAKE, REPRODUCE) | Better cloth simulation with adaptive resolution and frictional contact — supports "we ship our own games" with polished, believable character movement and environment cloth. Makes the fighter and arena genres viable on engine2040. |
| DX5 adaptive fracture (TAKE, REPRODUCE) | Better runtime fracture with adaptive mesh coarsening — supports "we ship our own games" with polished environmental destruction. Supports the arena scale vision. |
| B-spline FEM (REJECT) | Rejected — stays deterministic and provider-agnostic. No floating-point cloth simulation dependency. |
| PX12 DAT coupling (TAKE, UNVERIFIED) | Penetration-free cloth-ragdoll coupling — supports "we ship our own games" with believable combat interactions involving cloth. Principle transfers; implementation UNVERIFIED until engine-specific testing. |
| Full pipeline (LATER) | Deferred — the complete pipeline is incompatible with determinism constraints. Individual design patterns can be adapted. Revisit after PX12/DX5/PX12 building blocks exist. |

---

## Source traceability

- **ArXiv API**: `https://export.arxiv.org/api/query?search_query=physics+cloth+fracture+simulation+2025+2026&max_results=10`
- **Papers accessed**:
  - https://arxiv.org/abs/2608.17833v1 (accessed 2026-09-26)
  - https://arxiv.org/abs/2609.21201v1 (accessed 2026-09-26)
  - https://arxiv.org/abs/2506.18867v4 (accessed 2026-09-26)
  - https://arxiv.org/abs/2604.15513v1 (accessed 2026-09-26)
- **License verified**: All papers under arXiv distribution. Code/data/weights: UNVERIFIED (papers provide methods, not code). Ideas only, reimplemented, never copied.
- **Cross-reference**: DEEP-DIVES.md §05 (dive 05 row), `docs/plan/physics.md` (PLN5)
- **Avoids double-serving**: Dive 05 not served by any prior arxiv wave (waves 27, 30, 31, 33, 34, 35, 36, 37 serve dives 16, 09, 20, 12, 11, 11, 12, 04 respectively). Not served by STEAL (STEAL serves dives 11, 12, 15–20).
- **EMPIRE.md**: §1 (we ship our own games), §2 (Forge tools), §3 (factory makes, markets, reuses)
