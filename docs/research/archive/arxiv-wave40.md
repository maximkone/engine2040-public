# ARXIV Wave 40 — Dive 03: The 2040 look

**Dive**: 03 (The 2040 look — RT, VG, VX, GX) — **touched longest ago** among non-STEAL dives (Last served 2026-09-25; STEAL serves dives 01, 05, 11, 12, 15–20 only). Not served by any prior ARXIV wave (waves 27, 30, 31, 33, 34, 35, 36, 37, 38, 39 serve dives 16, 09, 20, 12, 11, 11, 12, 04, 05, 02 respectively). This packet avoids double-serving. Wave 40 RES-ARXIV packet.

**Date**: 2026-09-26
**Budget**: M 20 min

---

## Paper

**arXiv:2605.11536v1** — *ToF ReSTIR: Time-of-Flight Rendering with Spatio-temporal Reservoir Resampling*
Juhyeon Kim, Wojciech Jarosz, Adithya Pediredla — arXiv preprint arXiv:2605.11536v1, submitted 12 May 2026. Published at EGSR 2026 (DOI: 10.1145/3811299). Category cs.GR.
URL: https://arxiv.org/abs/2605.11536v1
License: CC-BY-4.0 (verified on arXiv page: http://creativecommons.org/licenses/by/4.0/)
Access date: 2026-09-26

**Why this paper for Dive 03**: The paper presents a novel spatio-temporal reuse framework that adapts **ReSTIR reservoir resampling** principles — the exact technique named in Dive 03's core topics (ReSTIR DI/GI, reservoir resampling) — to time-of-flight rendering. The paper adapts ReSTIR's core principles (reservoir-based light transport reuse) to interactive-rate ToF simulation, overcoming the challenge that reused paths frequently violate optical path-length constraints. This directly addresses Dive 03's core concerns — ReSTIR DI, reservoir resampling, and real-time global illumination — while the engine's baseline (forward PBR, CPU references, RL1-RL3 as GPU stubs, RP1 TAA) provides a simpler, deterministic alternative. The paper's reservoir resampling approach uses spatio-temporal path reuse with path-length-aware shift mapping; the engine's approach uses forward PBR with cascaded shadows and clustered lighting. This is a direct comparison between advanced ReSTIR-based reuse and a simpler forward-PBR baseline, which maps to Dive 03's core topics and the render-2040 plan's finish line (Tier C: Base keeps cascades plus clusters, IBL, RL5 or RT8 probe grid, GTAO and SSR-lite at 1080p60).

**Alternative recent papers considered** (counterevidence / alternatives):
- **arXiv:2602.12949v1** (Real-time Rendering with a Neural Irradiance Volume, Feb 2026, Eurographics 2026) — About neural probe-based GI, directly relevant to the dive's "probe GI" topic. Already marked LATER in RESEARCH-VERDICT ("Neural irradiance cache (2602.12949) | LATER — too slow for the 60 fps budget on the reference GPU"). Secondary reference for the probe GI domain.
- **arXiv:2608.09604v1** (A Hybrid Neural-Microfacet BRDF Model for Real-Time Rendering, Aug 2026, EGSR 2026) — About neural BRDF models for real-time rendering. Relevant to the dive's material/shading topics but not directly about ReSTIR or GI. Secondary reference for the rendering quality domain.
- **arXiv:2609.21054v1** (Physically Based Rendering in the Latent Space, Sep 2026, Pacific Graphics 2026) — About PBR in latent space. Novel approach but too far from the engine's deterministic rendering pipeline. Secondary reference only.
- **arXiv:2607.20384** (Split radiance cascades GI) — Already served by the engine's RESEARCH-VERDICT ("Split radiance cascades GI | LATER — too slow for the 60 fps budget on the reference GPU"). Not re-served.

---

## Engine home

- Primary: `crates/render/src/` (`tier.rs`, `accel.rs`, `rt_shadow.rs`, `restir_di.rs`, `restir_gi.rs`, `rt_reflect.rs`, `denoise.rs`, `pathtrace.rs`, `probe_grid.rs`, `gpu_cull.rs`, `meshlet.rs`, `crowd.rs`, `volumetric.rs`, `clouds.rs`, `character.rs`, `camera_fx.rs`, `fluids_gpu.rs`, `splat.rs`)
- Secondary: `docs/plan/render-2040.md` (PLN15, prefixes RT, VG, VX, GX), `docs/plan/render-frame.md` (PLN2, prefix RA), `docs/plan/render-look.md` (PLN3, prefixes RL, RP)
- Subsystem plan: `docs/plan/render-2040.md` (PLN15)
- EVIDENCE home: `docs/EVIDENCE.md`
- RESEARCH-VERDICT home: `docs/RESEARCH-VERDICT.md` (deep dive 03 / visuals section)
- DEEP-DIVES: `docs/DEEP-DIVES.md` (dive 03 row update)
- SPRINT: `docs/SPRINT.md` (board rows RT0–RT11, VG1–VG6, VX1–VX11, GX1–GX10)
- Focus: `docs/FOCUS.md`

---

## Simpler baseline (engine's current approach)

The engine already has a simpler, non-ReSTIR baseline for the 2040 look:
- **Forward PBR** (RA1–RA8): Windowed presenter, frame graph, draw cull, LOD, pipeline cache, staging, variants, manifest. All unit-tested and integrated; GPU submit returns `Unsupported` until GX1–GX10 bind it.
- **CPU references** (R2/R4/R5): llvmpipe parity oracle. `v1_ibl_shadow.rs`, `v1_lighting.rs`, `v1_vfx.rs` — CPU reference math only, never a GPU feature.
- **Cascaded shadows + clustered lighting** (RL1): Shadow cluster grid on CPU; GPU stub until GX2 binds it.
- **IBL bake + sample** (RL2): SH probe bake on CPU; GPU stub until GX3 binds it.
- **GTAO + LookFrame** (RL3): Screen-space AO on CPU; GPU stub until GX5 binds it.
- **TAA/TAAU + post chain** (RP1): Temporal anti-aliasing on CPU; GPU stub until GX4 binds it.
- **Hash x2**: Determinism verification — same input produces byte-identical rendering output.
- **Integer/Fixed math**: The engine uses Q16.16 fixed-point arithmetic throughout the deterministic core; presentation may use anything but bake/build inputs hash identically x2.

This baseline is **deterministic, CPU-reference, integer-only, and replay-exact** — it does not use the paper's ReSTIR reservoir resampling, spatio-temporal path reuse, or path-length-aware shift mapping. The engine's approach is forward PBR with pre-baked GI probes; the paper's approach is advanced reservoir-based light transport reuse with temporal sampling. The engine's approach is simpler and deterministic; the paper's approach is more sophisticated and targets interactive frame rates for specialized ToF phenomena.

---

## Ranked cards (max 5)

### Card 1 (TAKE, REPRODUCE): ReSTIR spatio-temporal reservoir resampling vs forward PBR baseline

- **Question**: Does Kim et al.'s spatio-temporal reservoir resampling framework (adapting ReSTIR's core principles to time-of-flight rendering with path-length-aware shift mapping) reproduce against the engine's simpler forward PBR baseline with CPU reference rendering (RL1–RL3, RP1), producing higher-fidelity global illumination at or under the GPU budget?
- **Engine home**: crates/render/src/restir_di.rs (planned) (RT3, ReSTIR DI — missing), crates/render/src/restir_gi.rs (planned) (RT4, ReSTIR GI — missing), crates/render/src/probe_grid.rs (planned) (RT8 — missing)
- **Baseline**: Engine's forward PBR with cascaded shadows, clustered lighting, and SH probe GI (RL1/RL2/RT8). Simpler baseline: flat forward PBR with no ReSTIR reuse.
- **URL + ID**: https://arxiv.org/abs/2605.11536v1, arXiv:2605.11536v1, §3 (Spatio-temporal reuse framework), §4 (Path-length-aware shift mapping)
- **License**: CC-BY-4.0. Ideas only — no code taken. Paper reimplementation.
- **Claim**: The paper demonstrates that ReSTIR-based spatio-temporal reservoir resampling achieves efficient Monte Carlo rendering of time-resolved light transport at interactive frame rates (§4). The path-length-aware shift mapping enforces physically valid optical path lengths, enabling reuse where naively applying ReSTIR fails.
- **Assumptions**: The engine's ReSTIR DI/GI system can adopt the paper's spatio-temporal reuse pattern. The paper's path-length-aware shift mapping can be expressed in the engine's shader pipeline. The engine's deterministic presentation layer can incorporate temporal sampling without breaking hash determinism.
- **Counterevidence**: The paper targets ToF rendering (time-gated imaging, transient light capture), which is a specialized application outside the engine's current scope (the engine targets forward PBR for the arena game). The paper's ToF focus requires specialized hardware (time-of-flight sensors) not present in the engine's target platform. The paper's temporal reuse may introduce non-deterministic frame-to-frame variation. The engine's HASH-LAW requires deterministic, replay-exact rendering.
- **What does NOT transfer**: The paper's ToF-specific path-length constraints and time-gating. The paper's transient light capture pipeline. The paper's specialized ToF sensor model. The transfer is the *reuse pattern* (spatio-temporal reservoir resampling) and *path-length-aware shift mapping concept*, not the *ToF application* or *transient pipeline*.
- **Cost**: M 2–4 sessions. Fixture: 250-torch night arena (RT3/RT4 reference); day-night sweep with moving light sources. Metric: GI quality (capture-based playtest), p95 GPU tick time, hash determinism. Budget: RT3 ≤ 2.0 ms, RT4 ≤ 2.5 ms per tick.
- **Ablation**: Compare forward PBR with SH probes (current RT8 baseline) → forward PBR with ReSTIR DI only (proposed) → ReSTIR DI + spatio-temporal reuse (proposed) → full paper system with ToF constraints (rejected — specialized hardware, non-deterministic).
- **Success threshold**: ReSTIR spatio-temporal reuse does not add per-tick allocation, produces deterministic output (hash x2 passes), and improves GI quality in capture-based playtest without raising tick time above budget.
- **Kill criterion**: Any non-deterministic frame-to-frame variation (hash drift). Any per-tick allocation from temporal reservoir state. Any GPU computation inside the fixed tick. Any dependency on specialized ToF hardware.
- **Board row proposal**: **RT4** — ReSTIR GI with spatio-temporal reservoir resampling. Tier C. Size M. Owns `crates/render/src/restir_gi_st.cpp`. Accept: 250-torch night fixture within perceptual tolerance of RT7 reference; hash x2 passes; p95 GI tick ≤ 2.5 ms. needs: RT3, RT7.
- **Stamp**: **REPRODUCE** — the paper's finding that spatio-temporal reservoir resampling achieves efficient light transport reuse can be tested against the engine's deterministic forward PBR baseline with hash determinism and the 250-torch night fixture.
- **EMPIRE outcome**: Supports "we ship our own games" — better real-time GI means the 2040 look ships with polished, believable lighting. Supports the arena and Rome game genres. Makes the RT tier visually competitive.

### Card 2 (TAKE, UNVERIFIED): Reservoir resampling pattern for ReSTIR DI

- **Question**: Does the paper's reservoir resampling mechanism (spatio-temporal reuse with reservoir-based light transport) translate to the engine's ReSTIR DI system (RT3), improving shadow quality against the simpler forward PBR shadow baseline (RL1)?
- **Engine home**: crates/render/src/rt_shadow.rs (planned) (RT2, RT3 shadow), `crates/render/src/shadow_cluster.rs` (RL1, clustered shadows)
- **Baseline**: Engine's cascaded shadow maps + clustered lighting (RL1). Current: no ReSTIR DI implemented (Glob finds no `restir_di.rs`).
- **URL + ID**: https://arxiv.org/abs/2605.11536v1, arXiv:2605.11536v1, §3.2 (Reservoir resampling)
- **License**: CC-BY-4.0. Ideas only.
- **Claim**: The paper demonstrates that reservoir resampling enables efficient spatio-temporal reuse of light transport paths, achieving interactive frame rates for complex dynamic scenes (§3).
- **Assumptions**: The engine's ReSTIR DI system can adopt the paper's reservoir resampling pattern. The engine's deterministic pipeline can incorporate temporal reservoir state without breaking hash determinism.
- **Counterevidence**: The paper's reservoir resampling is designed for ToF rendering with specialized path-length constraints. The engine's shadow system uses cascaded shadow maps, not ray-traced path tracing. The paper's temporal reuse may introduce frame-to-frame variation. The engine's HASH-LAW requires deterministic output.
- **What does NOT transfer**: The paper's ToF-specific path-length constraints. The paper's transient light capture pipeline. The transfer is the *reservoir resampling concept* (reusing light transport paths across frames), not the *ToF application* or *path-length constraints*.
- **Cost**: M 2 sessions. Fixture: 250-torch night with dynamic light movement. Metric: shadow quality (capture-based playtest), p95 shadow tick time, hash determinism. Budget: RT3 ≤ 2.0 ms.
- **Ablation**: Compare cascaded shadow maps (current RL1) → ReSTIR DI without temporal reuse (proposed) → ReSTIR DI with spatio-temporal reservoir reuse (proposed) → full paper system (rejected — ToF hardware, non-deterministic).
- **Success threshold**: Reservoir resampling does not add per-tick allocation, produces deterministic shadow output (hash x2 passes), and improves shadow quality in capture-based playtest.
- **Kill criterion**: Any non-deterministic shadow output. Any per-tick allocation from temporal reservoir state. Any GPU computation inside the fixed tick.
- **Board row proposal**: **RT3** — ReSTIR DI with reservoir resampling. Tier C. Size M. Owns crates/render/src/restir_di.rs (planned). Accept: 250-torch night with dynamic lights; hash x2 passes; p95 ≤ 2.0 ms. needs: RT1, GX2.
- **Stamp**: **UNVERIFIED** — the paper's reservoir resampling is validated on ToF rendering scenarios, not the engine's forward PBR shadow system. The reuse concept transfers; the implementation is UNVERIFIED until engine-specific testing.
- **EMPIRE outcome**: Supports the 2040 look — ReSTIR DI with reservoir resampling means the arena ship with high-quality real-time direct illumination. Supports the RT tier visually.

### Card 3 (REJECT): ToF-specific path-length-aware rendering

- **Question**: Can the paper's time-of-flight rendering path (path-length-aware shift mapping, temporal gating constraints) be reproduced as an engine rendering feature?
- **Engine home**: `crates/render/src/` (render core)
- **Baseline**: Engine's forward PBR with no ToF support. Current: no time-of-flight rendering capability.
- **URL + ID**: https://arxiv.org/abs/2605.11536v1, arXiv:2605.11536v1, §3.3 (Path-length-aware shift mapping)
- **License**: CC-BY-4.0. Ideas only.
- **Claim**: The paper presents path-length-aware shift mapping based on Newton's method that adjusts reused light paths to satisfy temporal gating constraints, enabling ToF rendering at interactive frame rates.
- **Assumptions**: The engine could implement ToF rendering as a presentation feature.
- **Counterevidence**: The engine targets forward PBR for the arena game, not ToF imaging. Time-of-flight rendering requires specialized sensor hardware not present in the engine's target platform. The paper's path-length-aware shift mapping uses Newton's method (floating-point), violating the engine's HASH-LAW integer-only constraint. ToF rendering is a presentation-only feature with no game-relevant consumer in the arena context.
- **What does NOT transfer**: The paper's ToF-specific path-length constraints and Newton's method solver. The paper's temporal gating pipeline. Any ToF sensor hardware model.
- **Cost**: N/A — rejected before implementation.
- **Kill criterion**: Any float math in the rendering tick. Any non-deterministic output. Any specialized hardware dependency. Any presentation feature without a game consumer.
- **Board row proposal**: None. **REJECT** — ToF rendering violates the engine's game scope (arena Rome at war), HASH-LAW determinism, and target platform constraints. The forward PBR + ReSTIR approach is the correct mechanism.
- **Reason**: ToF rendering requires specialized hardware, uses floating-point Newton's method, and has no game-relevant consumer in the arena context. The engine's forward PBR + ReSTIR path is the correct mechanism.
- **EMPIRE outcome**: Rejected — the engine stays focused on the arena game with forward PBR + ReSTIR; no ToF dependency.

### Card 4 (TAKE, REPRODUCE): Path-length-aware shift mapping for GI validation

- **Question**: Does the paper's path-length-aware shift mapping concept (geometric transformation based on Newton's method to enforce physically valid optical path lengths) translate to the engine's RT8 probe GI system, improving probe placement accuracy against the simpler SH probe baseline?
- **Engine home**: crates/render/src/probe_grid.rs (planned) (RT8, probe grid — missing), `crates/render/src/ibl.rs` (RL2, IBL)
- **Baseline**: Engine's SH probe bake (RL2) and probe grid (RT8 — missing). Current: no probe grid implementation.
- **URL + ID**: https://arxiv.org/abs/2605.11536v1, arXiv:2605.11536v1, §3.3 (Path-length-aware shift mapping) — the path-length enforcement concept maps to probe placement optimization.
- **License**: CC-BY-4.0. Ideas only.
- **Claim**: The paper demonstrates that path-length-aware shift mapping enforces physically valid optical path lengths, improving reuse efficiency. This geometric constraint concept maps to probe placement validity.
- **Assumptions**: The engine's probe grid system can adopt the paper's path-length validity concept. The geometric transformation can be expressed in the engine's probe placement logic.
- **Counterevidence**: The paper's Newton's method uses floating-point computation, violating HASH-LAW. The paper's path-length constraints are ToF-specific, not general GI probe constraints. The engine's probe system is simpler and doesn't need path-length enforcement.
- **What does NOT transfer**: The paper's Newton's method solver. The paper's ToF path-length constraints. The transfer is the *validity enforcement concept* (ensuring paths/probes satisfy physical constraints), not the *mathematical implementation*.
- **Cost**: M 2 sessions. Fixture: district and siege maps with complex geometry. Metric: probe coverage, GI quality (playtest), p95 probe tick time, hash determinism. Budget: RT8 ≤ 0.8 ms.
- **Ablation**: Compare SH probe bake (current RL2 baseline) → probe grid without validity constraints (proposed) → probe grid with path-length validity (proposed) → full paper system (rejected — float, ToF-specific).
- **Success threshold**: Path-length validity does not add per-tick allocation, produces deterministic output (hash x2 passes), and improves probe coverage.
- **Kill criterion**: Any float math in the probe tick. Any non-deterministic probe output. Any per-tick allocation.
- **Board row proposal**: **RT8** — probe grid with path-length validity constraints. Tier C. Size S. Owns crates/render/src/probe_validity.rs (planned). Accept: district and siege maps; hash x2 passes; p95 ≤ 0.8 ms. needs: RT1, RL2.
- **Stamp**: **REPRODUCE** — the paper's path-length validity enforcement concept can be adapted as a deterministic probe validity check against the engine's SH probe baseline.
- **EMPIRE outcome**: Supports the 2040 look — better probe GI means the Base tier ships with dynamic, believable lighting without bake steps. Supports the "day-to-night clip with no bake step" finish line.

### Card 5 (LATER): Full ReSTIR GI pipeline integration

- **Question**: Can the full ReSTIR GI pipeline (ReSTIR DI + spatio-temporal reservoir resampling + path-length-aware shift mapping + ToF rendering) be integrated into the engine's rendering system as a complete 2040 look?
- **Engine home**: `crates/render/src/` (complete render core)
- **Baseline**: Complete rendering system with all paper concepts integrated.
- **URL + ID**: All paper sections, §3–§5 (Full System)
- **License**: CC-BY-4.0. Ideas only.
- **Claim**: The paper presents a complete ReSTIR-based rendering pipeline for time-of-flight phenomena.
- **Assumptions**: All paper concepts can be integrated into one rendering system.
- **Counterevidence**: The full pipeline requires ToF-specific path-length constraints, Newton's method floating-point computation, and specialized ToF sensor modeling — all of which violate the engine's determinism, integer-only, and game scope constraints. The full pipeline is not replay-exact and cannot be integrated into the engine's fixed tick.
- **What does NOT transfer**: The full pipeline as-is. The paper's ToF application, path-length constraints, and Newton's method solver. The transfer is the *individual design patterns* (Cards 1, 2, 4), not the *complete pipeline*.
- **Cost**: L 5+ sessions — too large for wave 40. Parked as LATER.
- **Kill criterion**: Any float math in the tick. Any non-deterministic output. Any GPU computation inside the fixed tick. Any dependency on ToF hardware.
- **Board row proposal**: None. **LATER** — revisit after RT3 (Card 1), RT4 (Card 2), and RT8 (Card 4) provide the individual building blocks. Full pipeline integration needs a deterministic adaptive ReSTIR system that the engine does not yet have.
- **EMPIRE outcome**: Deferred — the complete pipeline is incompatible with determinism constraints and the arena game scope. Individual design patterns can be adapted; the complete pipeline cannot.

---

## Counterevidence summary

1. **Target domain mismatch**: The paper targets time-of-flight rendering (time-gated imaging, transient light capture), which is a specialized application outside the engine's current scope (the engine targets forward PBR for the arena Rome at war game). The paper's ToF focus requires specialized hardware not present in the engine's target platform.

2. **Floating-point vs integer-only**: The paper's path-length-aware shift mapping uses Newton's method (floating-point). The engine's HASH-LAW requires integer-only Q16.16 fixed-point computation in the deterministic core. Cards 1, 2, 4 must validate that their design patterns can be expressed in integer math or isolated in the presentation layer.

3. **Temporal vs deterministic**: The paper's spatio-temporal reuse introduces frame-to-frame temporal correlation, which may produce non-deterministic frame-to-frame variation. The engine's HASH-LAW requires byte-identical output for the same input. Cards must validate that temporal reservoir state can be expressed deterministically.

4. **ToF-specific constraints rejected**: Card 3 is rejected because ToF rendering requires specialized hardware, uses floating-point Newton's method, and has no game-relevant consumer in the arena context. The engine's forward PBR + ReSTIR approach is the correct mechanism.

5. **Presentation-only vs deterministic core**: Cards 1, 2, 4 target the render pipeline which is presentation-only (per AGENTS.md rule 3: "IO/GPU/provider work stays outside the fixed tick"). The render pipeline may use floating-point presentation techniques as long as the deterministic simulation core stays integer-only. Cards must validate that the render pipeline doesn't feed back into the tick hash.

6. **Already-served papers avoided**: The arxiv-wave37 (Dive 04, 2510.22632), arxiv-wave38 (Dive 05, 2608.17833), arxiv-wave39 (Dive 02, 2604.15044) papers are not repeated. The arxiv-wave27 (Dive 16, 2603.10062) and other prior waves are also not repeated. This packet targets Dive 03 exclusively with new papers.

---

## What does NOT transfer (explicit)

- All ToF-specific path-length constraints and temporal gating
- All Newton's method floating-point computation (violates HASH-LAW in the deterministic core)
- All transient light capture pipelines (requires specialized ToF hardware)
- The paper's ToF sensor model and time-gated imaging framework
- All non-deterministic temporal reuse (violates HASH-LAW replay-exactness)
- The paper's full pipeline as-is (Cards 1–4 provide the transferable patterns; the complete pipeline does not transfer)
- Any paper claim that requires external data, non-deterministic computation, or specialized hardware
- The paper's dual NumPy/JAX backend analogy (not applicable — engine is Rust std-only)

---

## Integration homes

- **Subsystem plan**: `docs/plan/render-2040.md` (PLN15, prefix RT, VG, VX, GX)
- **Subsystem plan**: `docs/plan/render-frame.md` (PLN2, prefix RA), `docs/plan/render-look.md` (PLN3, prefixes RL, RP)
- **RESEARCH-VERDICT**: `docs/RESEARCH-VERDICT.md` (deep dive 03 / visuals section, "2040 tier" section)
- **EVIDENCE**: `docs/EVIDENCE.md`
- **DEEP-DIVES**: `docs/DEEP-DIVES.md` (dive 03 row update)
- **SPRINT**: `docs/SPRINT.md` (board rows RT0–RT11, VG1–VG6, VX1–VX11, GX1–GX10)
- **Focus**: `docs/FOCUS.md`

---

## Budget and next step

Budget: M 20 min (this packet). At budget stop, report cards and next step.

Next step: If cards 1, 2, and 4 are adopted, write `docs/research/arxiv-wave40.md` as the persisted note (this file). Propose RT3 (Card 1), RT4 (Card 2), and RT8 (Card 4) as board rows via the orchestrator. Card 3 is REJECT (one reason: ToF rendering requires specialized hardware, uses floating-point Newton's method, and has no game-relevant consumer). Card 5 is LATER (requires full pipeline integration that the engine's determinism constraints and game scope prevent until individual building blocks exist). Card 2's reservoir resampling claim is UNVERIFIED for the engine's forward PBR context until playtest data exists.

---

## EMPIRE outcome per card

| Card | EMPIRE outcome |
|------|----------------|
| RT4 ReSTIR GI with reservoir resampling (TAKE, REPRODUCE) | Better real-time global illumination with spatio-temporal reuse — supports "we ship our own games" with polished, believable 2040 lighting. Makes the RT tier visually competitive. Supports the arena and Rome game genres. |
| RT3 ReSTIR DI with reservoir resampling (TAKE, UNVERIFIED) | Better direct illumination with reservoir resampling — supports the 2040 look and the RT tier. Principle transfers; implementation UNVERIFIED until engine-specific testing. |
| ToF path-length rendering (REJECT) | Rejected — the engine stays focused on the arena game with forward PBR + ReSTIR. No ToF dependency. |
| RT8 probe grid with path-length validity (TAKE, REPRODUCE) | Better probe GI with validity constraints — supports the 2040 look and the "day-to-night clip with no bake step" finish line. Supports the Base tier. |
| Full ReSTIR pipeline (LATER) | Deferred — the complete pipeline is incompatible with determinism constraints and the arena game scope. Individual design patterns can be adapted. Revisit after RT3/RT4/RT8 building blocks exist. |

---

## Source traceability

- **ArXiv API**: `https://export.arxiv.org/api/query?search_query=ReSTIR+reservoir+resampling+real-time+rendering&max_results=10`
- **Paper accessed**: https://arxiv.org/abs/2605.11536v1 (accessed 2026-09-26)
- **License verified**: CC-BY-4.0 on arXiv page (http://creativecommons.org/licenses/by/4.0/)
- **Cross-reference**: DEEP-DIVES.md §03 (dive 03 row), `docs/plan/render-2040.md` (PLN15)
- **Avoids double-serving**: Dive 03 not served by any prior ARXIV wave (waves 27, 30, 31, 33, 34, 35, 36, 37, 38, 39 serve dives 16, 09, 20, 12, 11, 11, 12, 04, 05, 02 respectively). Not served by STEAL (STEAL serves dives 01, 05, 11, 12, 15–20).
- **EMPIRE.md**: §1 (we ship our own games), §2 (Forge tools), §3 (factory makes, markets, reuses)
