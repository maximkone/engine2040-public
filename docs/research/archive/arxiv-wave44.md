# ARXIV Wave 44 — Dive 11: hard-problem queue (GI without bake, probe relight, latency method, soak change-point)

**Dive**: 11 (hard-problem queue: dynamic GI without bake, probe relight budgets, input-latency method, soak leak-change-point methods). Wave 44 RES-ARXIV packet.
**Date**: 2026-09-26
**Budget**: M 20 min
**Donor rotation**: COSA-GS (2609.30245, wave 43) explicitly NOT repeated — no new evidence. Split Radiance Cascades (2607.20384) already verdict-LATER — cited as context only, not re-served.

---

## Paper (primary donor)

**arXiv:2108.05263v1** — *Dynamic Diffuse Global Illumination Resampling*
Zander Majercik, Thomas Müller, Alexander Keller, Derek Nowrouzezahrai, Morgan McGuire — arXiv preprint, submitted 11 Aug 2021. Category cs.GR.
URL: https://arxiv.org/abs/2108.05263
License: not verified on arXiv page. Ideas only — reimplemented, never copied.

**Reproduced claim (paper's, in own words)**: the authors extend unidirectional path tracing for diffuse/multi-bounce transport by combining screen-space reservoir resampling with a sparse set of world-space probes, and report better sample efficiency at equal time and equal quality than purely path-traced and purely probe-based baselines, reaching interactive GI in complex scenes when paired with commodity denoisers.

**Why this paper for this queue**: it is the only fresh donor that hits two queue items at once — no-bake dynamic GI (RT4) and the probe-grid fallback (RT8) — with an evaluation discipline (equal-time / equal-quality vs baselines + converged reference) the engine can steal for RT7/RT11 gates. All other queue items have no fresh arXiv donor (negative search results reported honestly below); they get method cards against existing engine baselines.

**Alternative donors considered (counterevidence / context)**:
- **arXiv:2007.14394v1** (SDF Dynamic Diffuse GI, Hu et al. 2020) — SDF scene approximation + discretized irradiance, no precomputation, multi-bounce, claims large-open and indoor scenes without noise. Counterevidence: SDF approximation quality vs engine's proxy-accuracy acceptance (CT13/RT1 proxies) is unmeasured; leak behavior unreported. Secondary reference only.
- **arXiv:2505.02041v1** (Holographic Radiance Cascades, Freeman et al. 2025) — 2D single-shot radiance transfer, 1.85 ms at 512² on RTX 3080 Laptop. Counterevidence: 2D result, does not transfer to the 3D arena; cited to bound the RC family, not as a take.
- **arXiv:2607.20384v1** (Split Radiance Cascades, Freeman & Sannikov 2026) — already verdict-LATER (too slow on reference GPU; baked GI first). NOT re-served; cited only so a future spike can run the head-to-head if RT4's kill rule fires.
- **arXiv:2504.04422v1** (LeakGuard, Liang et al. 2025) — static, path-sensitive leak detection with 129 confirmed bugs. Does NOT transfer to runtime soak onset detection (different problem: static proof vs online change-point). Cited as the reason Card 4 stays with classical detectors, not ML/static tools.

**Negative results (searched, nothing usable — itself a finding)**:
- `ti:"radiance cascades" OR ti:"ReSTIR GI" OR ti:"dynamic diffuse global illumination"` → the four GI hits above (only 2108.05263 transfers).
- Input-latency / input-to-photon method queries → no game-pipeline latency paper on arXiv; Card 3 proceeds donor-free against `crates/perf/src/latency.rs`.
- Change-point / soak-onset queries → no leak-onset paper; classical CUSUM/PELT/STA-LTA are textbook methods, and PQ3's design already cites Georges OOPSLA 2007 (`docs/plan/quality.md` §PQ3). Card 4 positions against PQ5 (held STA/LTA moonshot), not a new paper.

---

## Engine home

- Primary: `docs/plan/render-2040.md` (PLN15, RT4 no-bake GI + world cache, RT8 probe grid, RT7 path reference, RT11 gates), `docs/plan/quality.md` (PLN14, PQ1 latency, PQ3 soak)
- Consumers: WIN1 arena match (day-to-night clip, no bake step), ED1 viewport, Q1 captures, PF2 per-pass timings
- Subsystem plans: `docs/plan/render-2040.md`, `docs/plan/quality.md`
- EVIDENCE home: `docs/EVIDENCE.md` (one row per ported file / method adoption)
- RESEARCH-VERDICT home: `docs/RESEARCH-VERDICT.md` (wave 44 section)
- SPRINT: board rows RT4, RT8, RT7, RT11, PQ1, PQ3 (PQ5 held, PQ6 declined wave 23)

---

## Simpler baseline (what the engine already has)

- **RT0/RT1 DONE**: tier probe + BLAS/TLAS from proxies (`tier.rs`, `accel.rs`, 1.5 ms refit budget). The ray substrate exists; no GI rides on it yet.
- **RT4/RT8 missing**: Glob finds no `restir_gi.rs`, no `probe_grid.rs`. Base-tier GI today is RL5 baked lightmaps (or nothing); the "day-to-night clip with no bake step" acceptance is unmet.
- **RT7 missing**: no `pathtrace.rs` converged reference, so no perceptual gate exists for any GI spike to beat.
- **PQ1 implemented, report unwired**: `crates/perf/src/latency.rs` (511 lines, integer tick math, `PRESENT_P95_BUDGET_MS = 50.0`, presentation-only `WallMeter`); zero reference-GPU numbers on disk.
- **PQ3 open**: hour soak unwired; PQ5 STA/LTA picker is the held moonshot, PQ6 Richardson declined (wave 23) — Card 4 must not duplicate either slot.

---

## Ranked cards (max 5)

### Card 1 (TAKE, REPRODUCE): reservoir + sparse-probe diffuse GI vs path-traced and probe-only baselines

- **Question**: does the 2108.05263 pattern (screen-space reservoir resampling feeding sparse world-space probes for diffuse-terminating transport) reproduce on the arena: better sample efficiency at equal time AND equal quality than a pure path-traced baseline and a pure probe baseline, with the gate-tunnel leak test passing?
- **Engine home**: crates/render/src/restir_gi.rs (planned) (new, RT4) + crates/render/src/probe_grid.rs (planned) (new, RT8 probe update path); reference in `pathtrace.rs` (RT7)
- **Baseline**: RT7 CPU path-tracer (to be built first, offline/converged) + trivial uniform probe grid. Paper §4-equivalent: equal-time + equal-quality comparisons, commodity denoiser on.
- **URL + ID**: https://arxiv.org/abs/2108.05263, arXiv:2108.05263v1 (full system + evaluation discipline)
- **License**: UNVERIFIED on arXiv page. Ideas only — reimplementation sketch, no code taken.
- **Paper claim**: clear win at equal time and equal quality over both baselines; interactive GI in complex scenes with denoisers.
- **Reproduced result**: NONE YET — no engine spike exists. This card proposes the reproduction; numbers below are thresholds, not forecasts.
- **Assumptions**: RT1 proxies are an adequate ray substrate; wgpu ray queries cover 1–2 shadow rays + 1 reflection ray (per verdict wave rounds 5-6); denoiser is RT6 engine-owned (vendor denoisers CUT).
- **Counterevidence**: paper scenes ≠ arena (gate tunnel, 250-torch night); paper's "complex scenes" ran above reference-GPU class with commodity denoisers doing heavy lifting — RT6 does not exist yet, so the spike must run denoiser-off ablations too; SDF-DDGI (2007.14394) claims no-precompute multi-bounce without reservoirs and could beat this on perf — keep as the challenger in the ablation.
- **What does NOT transfer**: any CUDA/optix code, any vendor denoiser, paper scenes/metrics as acceptance (arena fixtures rule).
- **Experiment sketch (S, 1 session spike)**: build RT7 converged stills on the material chart first; then `restir_gi.rs` minimal reservoir (1–2 resampling passes, fixed seed, integer RNG from RC7 counter-RNG) writing into a sparse probe grid; fixtures: gate-tunnel leak test, 250-torch night arena, day-to-night clip segment. All presentation-side, HASH-LAW untouched.
- **Benchmark/ablation**: (a) pure path trace at equal ms, (b) probe-only at equal ms, (c) reservoir+probe, (d) c + RT6-style temporal filter when it lands, (e) SDF-DDGI-pattern challenger (idea-only). Metric: FLIP vs RT7 converged stills (PQ4 primary) + per-pass ms via PF2 + leak-test pass/fail. Equal-time AND equal-quality — report both directions like the paper.
- **Success threshold**: beats both baselines on FLIP at equal ms AND at equal quality uses fewer ms; leak test passes; RT-tier stills within posted perceptual tolerance of RT7.
- **Kill criterion**: any tick/hash contamination; leak test fails twice (same fingerprint → BLOCKERS.md, RT8 probe grid stays the Base answer); misses 60 fps RT budget on reference GPU after denoiser lands; needs vendor denoiser or float in the tick.
- **Board row proposal**: RT4 spike row already exists — this card tightens its acceptance (equal-time/equal-quality + leak test + denoiser-off ablation), no new row. EVIDENCE row on the spike.
- **Stamp**: REPRODUCE (proposed — nothing reproduced yet).
- **EMPIRE outcome 4**: feeds "AI makes whole games" — no-bake GI is what lets a generated arena variant (brief→game) render day-to-night with no bake step and no artist intervention.

### Card 2 (TAKE, REPRODUCE): probe relight time-box as the Base-tier GI budget

- **Question**: can the world-space probe update (the paper's sparse-probe half, plus SDF-DDGI's no-precompute update) be time-boxed per frame so RT8 serves as the Base-tier GI fallback inside a posted relight budget?
- **Engine home**: crates/render/src/probe_grid.rs (planned) (RT8); PF2 per-pass ms names the relight pass
- **Baseline**: RL5 baked lightmaps (static) — relight cost today is a full rebake (unbounded). Proposed: bounded dirty-probe relight per frame.
- **URL + ID**: https://arxiv.org/abs/2108.05263 (sparse world-space probes) + https://arxiv.org/abs/2007.14394v1 §irradiance discretization (idea-only challenger)
- **License**: both UNVERIFIED. Ideas only.
- **Paper claim (transferred)**: sparse probes + discretization make dynamic update cheap enough to skip precomputation — but neither paper posts a console-class relight budget, so the budget is OURS: relight pass ≤ posted ms inside the 1080p60 Base frame on the reference GPU (number set by PF2/PF10 profiling, not by the papers).
- **Experiment sketch (S)**: probe grid with dirty-set tracking (only relight probes touched by changed lights/geometry); fixtures: torch-row toggle (N lights change), gate open/close (occlusion change); measure relight ms distribution + frames-to-converge.
- **Benchmark/ablation**: full-grid relight vs dirty-set relight vs RL5 rebake (off-line reference); frames-to-converge after a light step vs probe density.
- **Success threshold**: dirty-set relight ≤ budget on reference GPU; converges in ≤ N frames (posted); hash untouched (presentation only).
- **Kill criterion**: full-grid relight needed every frame to avoid visible lag; budget blown on reference GPU; leak test fails.
- **Board row proposal**: tightens existing RT8 acceptance (add "relight pass ≤ posted ms, dirty-set only"), no new row. Consumer: WIN1 arena Base tier + ED1 viewport.
- **Stamp**: REPRODUCE (proposed).
- **EMPIRE outcome 4**: brief→game needs instant relight when the AI moves a light — no rebake in the loop.

### Card 3 (TAKE, method — NO arxiv donor): input-latency staged meter with report wiring

- **Question**: with no usable arXiv donor (negative search logged above), can the engine's own staged method — input-sample → tick → render → present breakdown from `latency.rs` — be wired to the PQ1 report so feel regressions (GP18 kill bars) have numbers?
- **Engine home**: `crates/perf/src/latency.rs` (exists) → PQ1 report wiring; consumers GP18 (`PQ1 p95 ≤ 50 ms` gate), Q1 perf report
- **Baseline**: `WallMeter` presentation-only, unwired. No paper baseline — say so plainly.
- **URL + ID**: none (negative result). Method source: engine's own `latency.rs` + Mittal-style staged decomposition as textbook practice (UNVERIFIED — no paper claimed).
- **Paper claim**: N/A — keep distinct: there is no reproduced paper result here, only an engine measurement task.
- **Experiment sketch (S)**: wire staged timestamps (input poll, tick start/end, submit, present) on the WIN1 loop; post p50/p95 per stage; seeded input script for repeatability; llvmpipe numbers first, reference-GPU numbers operator-side.
- **Benchmark/ablation**: stage-breakdown before/after TAAU + dynamic-resolution toggles (which stage moves?); 60 Hz vs uncapped present.
- **Success threshold**: report green on both arena tracks; P2 p95 ≤ 50 ms bar evaluated with real numbers; GP18 gate enforceable.
- **Kill criterion**: instrumentation adds per-tick allocs (SL6 violation) — must be off-by-default atomic-gated like the PF2 profiler; numbers irreproducible across identical runs (then the harness, not the game, is broken → BLOCKERS.md).
- **Board row proposal**: existing PQ1 row stays; this card is its experiment sketch. No new row.
- **Stamp**: TAKE (method, donor-free, honestly labeled).
- **EMPIRE outcome 4**: "AI makes whole games" needs a feel gate the agent loop can read — latency numbers are what stop generated games feeling wrong.

### Card 4 (TAKE, method — classical detectors, NOT a paper port): soak leak-onset picker without duplicating PQ5/PQ6

- **Question**: can a classical online detector (CUSUM or PELT on alloc-bytes/alloc-count series from PF1 workloads) name the first bad minute of a seeded leak in the hour soak, without reopening the PQ5-vs-PQ6 slot fight (PQ5 STA/LTA held, PQ6 Richardson declined, wave 23)?
- **Engine home**: `crates/proof/src/` soak harness (PQ3); consumers PF1 budgets, SL21 audit
- **Baseline**: PQ5 STA/LTA picker (held moonshot) — Card 4 is its acceptance detail, NOT a rival row. LeakGuard (2504.04422v1) is static analysis: explicitly does NOT transfer (different problem).
- **URL + ID**: none fresh; cites Georges OOPSLA 2007 via existing PQ3 design + LeakGuard as do-not-take. Textbook methods (CUSUM/PELT) need no paper grant.
- **Experiment sketch (S)**: seed 3 leak shapes (linear creep, step, sawtooth-free-then-climb) into a short soak at known minutes; run CUSUM + STA/LTA pickers; score detection delay + false-alarm rate on clean runs.
- **Benchmark/ablation**: CUSUM vs STA/LTA (PQ5) head-to-head on the same seeded tapes; report delay/false-alarms; winner becomes PQ3's picker — if PQ5 wins, Card 4 dissolves into PQ5 (no new row ever).
- **Success threshold**: names the seeded minute within ±2 min on all 3 shapes; zero false alarms on 3 clean hour-equivalents (or short-matrix equivalents until the hour run is verified).
- **Kill criterion**: any picker needs tuning per leak shape (then it is not a gate, it is a demo); duplicates PQ5's slot — merge, don't row.
- **Board row proposal**: none — acceptance detail for PQ3/PQ5. EVIDENCE row only if a picker lands.
- **Stamp**: TAKE (method comparison, honestly donor-free).
- **EMPIRE outcome 4**: money-machine reliability — soak gates are what let 24/7 brief→game loops ship without slow leaks eating the fleet.

### Card 5 (REJECT): full DDGI-Resampling pipeline as-is into any tier

- **Question**: lift the paper's full pipeline (reservoir + probes + commodity denoiser, interactive on complex scenes) straight into the engine?
- **Verdict**: REJECT as a lift; Cards 1–2 take the patterns through engine gates instead.
- **Reason (one line each)**: paper leans on commodity denoisers the engine has not built (RT6 missing, vendor ones CUT); paper posts no reference-GPU-class budget; Base tier must keep RL5/RT8 raster fallbacks per verdict (V5 RT tier rule); full pipeline without RT7 reference + leak test is unmeasurable.
- **Revisit trigger**: RT6 lands + RT7 reference exists + Card 1 spike beats baselines — then the full combination is already what Cards 1–2 describe. No new evidence needed beyond that.

---

## What does NOT transfer (explicit)

- Paper's CUDA/OptiX code paths, vendor denoisers, paper scenes and thresholds as acceptance (arena fixtures rule).
- SDF-DDGI's SDF approximation as the ray substrate (RT1 proxies already own that; challenger idea only).
- Holographic-RC 2D numbers (domain mismatch).
- Split-RC as a new take (verdict-LATER; head-to-head only if Card 1's kill rule fires).
- LeakGuard internals for runtime soak (static ≠ online detection).
- Any neural upscaler / ray-regeneration / radiance-cache vendor path (verdict LATER/CUT lines hold).
- Any presentation math leaking into the tick or hash (HASH-LAW; Cards 1–2 are presentation-only by construction).

---

## Integration homes (per take)

| Take | Subsystem plan | EVIDENCE home | Consumer |
|---|---|---|---|
| Card 1 reservoir+probe GI | `docs/plan/render-2040.md` RT4 (+RT7 ref, RT11 gates) | `docs/EVIDENCE.md` spike row | WIN1 day-to-night clip, no bake |
| Card 2 probe relight budget | `docs/plan/render-2040.md` RT8 (+PF2 timings) | `docs/EVIDENCE.md` budget row | WIN1 Base tier, ED1 viewport |
| Card 3 latency report | `docs/plan/quality.md` PQ1 (+GP18 gate) | `docs/EVIDENCE.md` report row | GP18 feel gate, Q1 perf report |
| Card 4 soak picker detail | `docs/plan/quality.md` PQ3/PQ5 | `docs/EVIDENCE.md` only if lands | 24/7 loop fleet (factory) |
| Card 5 reject | — | — | — |

## Paper claim vs reproduced result vs integrated gain (kept distinct)

- **Paper claim**: 2108.05263v1 reports equal-time/equal-quality wins over path-traced and probe-only baselines + interactive GI with denoisers (their scenes, their hardware).
- **Reproduced result**: NONE — no spike has run. Cards 1–2 propose the reproduction with arena fixtures and kill rules.
- **Integrated gain**: NONE — RT4/RT7/RT8 files do not exist; nothing lands until the spike beats baselines with receipts. Any future gain is measured in FLIP-vs-RT7 + ms, never quoted from the paper.

---

## Budget and next step

Budget M 20 min: searches (GI hit + two negative results) + plan cross-checks + this note. At budget stop: takes + next step reported, no code touched.

Next step (orchestrator): file this note's Card 1/2 acceptance tightenings onto RT4/RT8 rows, keep PQ1/PQ3 rows unchanged (Cards 3–4 are sketches for existing rows), log the two negative searches so no future wave re-hunts latency/change-point donors on arXiv without new evidence.

## Source traceability

- ArXiv: `ti:"radiance cascades" OR ti:"ReSTIR GI" OR ti:"dynamic diffuse global illumination"` → 2108.05263v1, 2007.14394v1, 2505.02041v1, 2607.20384v1; latency/change-point queries → no usable donor (logged above); LeakGuard 2504.04422v1 read at abstract level 2026-09-26.
- Abstract verified live via arxiv tools 2026-09-26 (2108.05263). Citation-graph lookup timed out — not cited as evidence.
- Code cross-checks: `docs/plan/render-2040.md` (RT0/RT1 done, RT4/RT6–RT8 missing), `docs/plan/quality.md` (`latency.rs`, PQ1/PQ3 open, PQ5 held/PQ6 declined).
- Donor dedup: COSA-GS not repeated; Split-RC not re-served (verdict-LATER); wave 43 (Dive 10) untouched.
- EMPIRE: outcome 4 ("AI makes whole games" — brief→game needs no-bake GI, instant relight, feel gates, leak-free loops).
