# ARXIV Wave 45 — Dive 12: hard-problem queue (rename at scale, per-tick alloc profilers, capture pacing, budget-regression stats)

**Dive**: 12 (hard-problem queue: GE2b rename strategy at scale, per-tick allocation profilers, capture-pacing for fixed-step, budget-regression statistics). Wave 45 RES-ARXIV packet.
**Date**: 2026-09-26
**Budget**: M 20 min
**Donor rotation**: DDGI-Resampling (2108.05263, wave 44) and COSA-GS (wave 43) explicitly NOT repeated — no new evidence on either. Split-RC stays verdict-LATER, cited as context only.

---

## Papers (donors, in own words)

**arXiv:2601.00482v1** — *Multi-Agent Coordinated Rename Refactoring* (Bellur et al., Jan 2026). cs.SE/cs.AI. https://arxiv.org/abs/2601.00482 — License UNVERIFIED on arXiv page. Ideas only, reimplemented never copied.
**Reproduced claim (paper's, in own words)**: when one rename implies many related renames, the authors split the job three ways from the developer's first edit as the clue — one pass writes down the intended scope in words, one pass plans and applies edits only through the IDE's own trusted rename operations, one pass re-searches the whole project guided by that scope. Studied on ~609K commits across 100 projects plus a 205-developer survey; reports fewer missed follow-ups than pure heuristics and fewer incomplete edits than a bare model call.

**arXiv:2202.00173v2** — *Industry Experiences with Large-Scale Refactoring* (Ivers et al., 2022). Survey of 107 industry developers; large-scale refactors are common, team-scale, months-long, and developers lean on general IDE tooling over bespoke refactoring tools. Counterevidence / context for Card 1, not a second take.

**arXiv:2506.16883v1** — *Low Overhead Allocation Sampling in a Garbage Collected Virtual Machine* (Jung & Bolz-Tereick, Jun 2025). cs.PL. https://arxiv.org/abs/2506.16883 — License UNVERIFIED. Ideas only.
**Reproduced claim (paper's, in own words)**: instead of tracing every allocation, sample them on a tunable byte-period inside the collector; on their benchmarks a 4 MB period caps profiling time overhead around 25% over un-profiled runs. The transferable idea is the tunable period plus measured-overhead reporting — NOT the collector integration.

**arXiv:2606.18377v1** — *Exploring Statistical Change Point Detection Techniques for Performance Anomaly Detection at Mozilla* (Besbes et al., Jun 2026). cs.SE. https://arxiv.org/abs/2606.18377 — License UNVERIFIED. Ideas only.
**Reproduced claim (paper's, in own words)**: on one year of Mozilla perf data the shipped T-test pipeline yields ~12.5% false-positive alert groups and misses regressions in ~6.8% of groups; over 174 practitioner-annotated series, 25 change-point methods plus 15 ensembles were compared — offline/hybrid detectors win recall but lose precision alone, and vote-ensembles steady the trade-off at about +11% F1. Practitioner survey plus integration lessons included.

**arXiv:2408.08148v1** — *Early Detection of Performance Regressions by Bridging Local Performance Data and Architectural Models* (Liao et al., Aug 2024). Secondary reference for Card 4: component-level deviations propagated through an architecture model predict system-level regressions early, before full build-and-deploy testing. Two benchmark systems. Ideas only.

**Negative results (searched, nothing usable — itself a finding)**:
- Capture-pacing for fixed-step: `game loop fixed timestep accumulator real-time frame pacing` → narrative-pacing tool PaceMaker (2408.15001, design pacing — domain mismatch, do NOT transfer), network pacing (2507.09798, LEO congestion — mismatch), weather/game-engine neural papers (mismatch). Physics adaptive-timestep papers (2104.13995, 2401.02849, 1910.06349) solve solver stability, not accumulator-vs-capture cadence. **No game-loop capture-pacing donor on arXiv; Card 3 proceeds donor-free.**
- Per-tick allocation: first query (`per-tick allocation profiler heap allocation game engine`) returned allocator-security papers (FreeGuard, CAMP, SJMalloc) and game-theoretic "allocation games" — all mismatches. Refined query (`"allocation profiling" sampling overhead`) found 2506.16883, the one transferable donor.

---

## Engine home

- Primary: `docs/plan/general.md` (PLN18, GE2 ledger + GE11 refresh, GE6/GE10), `docs/plan/quality.md` (PLN14/PLN29: PF1/PF10 budgets, PF2 profiler, SL6/SL18–SL20/SL21 alloc floors, PQ3 soak, Q1 captures, PF9 rerun)
- Consumers: GE11 ledger refresh (rename scope), `crates/perf` budget gates + `proof` soak harness (regression stats), window/capture path + SHOW1 PNG-sequence stitch (capture pacing), SL21 submit-path audit (alloc profiler)
- Subsystem plans: `docs/plan/general.md`, `docs/plan/quality.md`
- EVIDENCE home: `docs/EVIDENCE.md` (one row per method adoption)
- RESEARCH-VERDICT home: `docs/RESEARCH-VERDICT.md` (wave 45 section)
- SPRINT: GE11, SL21, PF1/PF10, PF2, PQ3 (PQ5 held STA/LTA moonshot, PQ6 declined wave 23 — Card 4 must not duplicate that slot)

---

## Simpler baseline (what the engine already has)

- **GE2 DONE, GE11 open**: `tools/workspace-check.mjs` `genreLedger` gate fails unlisted `Fighter|Gladiator` names below `arena`/`game`; seeded-`Gladiator` refusal proven. Post-GE2 files (`ai/src/legion.rs`, `game/src/siege.rs`, `game/src/ludus_npcs.rs`) need verdicts — that refresh is exactly the rename-at-scale task.
- **Alloc floors DONE, profiler sampling MISSING**: `crates/perf/src/alloc.rs` counting allocator + `baselines.rs` exact byte/count rows enforced in the default suite; SL6/SL18–SL20 residuals closed by mutation proof (remove fix, floor rises). PF2 profiler is span-based (atomic-gated, puffin behind `profile`); there is no tunable allocation-sampling mode — every alloc counted exactly, which is precise but has no overhead knob for long soaks.
- **Fixed tick DONE, capture cadence UNGATED**: authoritative sim is fixed-step, replay-exact (golden `d57b5c8ae3096614` x2); captures are PNG+manifest (SHOW1: external ffmpeg stitch, encoder deps DROP). No posted rule ties capture cadence to the accumulator (every-Nth-tick stills vs every-presented-frame, spiral-of-death guard, capture-never-blocks-tick).
- **Budget gates DONE, regression statistics THIN**: PF1/PF10 fail on seeded byte/count/time regressions (exact today-vs-recorded comparison); PQ3 soak-onset picker is open with PQ5 STA/LTA held. No change-point statistics over perf time series, no ensemble, no false-alarm accounting like the Mozilla study.

---

## Ranked cards (max 5)

### Card 1 (TAKE, REPRODUCE): declared-scope coordinated rename for the GE11 ledger refresh

- **Question**: does the 2601.00482 pattern — developer's first edit as clue → written declared scope → plan executed only through trusted rename ops → scope-guided project-wide re-search — make the GE11 re-audit complete (zero missed follow-ups) without the false-positive flood the paper attributes to pure heuristics?
- **Engine home**: `tools/workspace-check.mjs` ledger hunks + `docs/plan/general.md` ledger table (GE11); trusted op = the existing ledger gate itself (seeded-refusal check), never a new dependency
- **Baseline**: today's ledger update done by hand-grep; heuristic-only rename (broad `grep Fighter|Gladiator` sweep) is the false-positive side; bare-model rename is the incomplete side
- **URL + ID**: https://arxiv.org/abs/2601.00482, arXiv:2601.00482v1 (method only). Counterevidence: https://arxiv.org/abs/2202.00173 (large refactors need team-scale tooling; general IDE ops beat bespoke tools — hence: use the existing gate, build no rename framework)
- **License**: both UNVERIFIED. Ideas only — reimplementation sketch, no code taken.
- **Paper claim**: fewer missed coordinated renames than heuristics, fewer incomplete edits than vanilla models (their 100-project corpus, their survey).
- **Reproduced result**: NONE YET — no engine spike exists. Thresholds below are bars, not forecasts.
- **Assumptions**: GE6 lands before GE11 (plan order stands) so the HUD move is recorded once; ledger verdicts stay keep/move/param.
- **Counterevidence**: 2202.00173 warns large-scale refactors stall on business priority and bespoke-tool fragility — so the take is a *discipline on existing tools* (gate + grep + review), not a new agent framework in-repo. No model writes code in the tick path; HASH-LAW untouched (rename-only, behavior-identical, golden checked x2).
- **What does NOT transfer**: the paper's multi-agent framework code, any model-driven edit path, any IDE plugin dependency.
- **Experiment sketch (S, 1 session)**: run GE11 as a declared-scope pass — (a) write the scope sentence first (which name families, which crates, which verdicts), (b) execute moves only through gate-checked edits with the seeded-refusal probe after each hunk, (c) close with a scope-guided full-tree re-search; log missed sites found in (c) vs (b).
- **Benchmark/ablation**: (a) heuristic-only sweep (count false-positive sites flagged), (b) declared-scope pass (count missed follow-ups caught in re-search + false positives filtered), (c) golden x2 + proof generality gates green. Metric: missed-site count, false-positive count, gate runs.
- **Success threshold**: zero unlisted genre names after the pass; seeded assumption still fails the gate; missed-site count in re-search is zero; golden unchanged x2.
- **Kill criterion**: re-search keeps finding missed sites after two passes with the same fingerprint → record in BLOCKERS.md, fall back to hand ledger (no framework built); any behavior/hash drift → reject immediately.
- **Board row proposal**: no new row — experiment sketch for existing GE11. EVIDENCE row on the refresh.
- **Stamp**: REPRODUCE (proposed).
- **EMPIRE outcome 4**: generality that never rots is what lets brief→game fixtures (any genre on public APIs) keep passing the engine-diff-empty gate while the tree grows.

### Card 2 (TAKE, REPRODUCE): tunable allocation-sampling profiler beside the exact counting floors

- **Question**: does a sampling allocation-profiler mode with a posted byte-period reproduce the 2506.16883 overhead discipline on the arena tick — bounded, tunable overhead with measured numbers — while the exact counting floors stay authoritative for gates?
- **Engine home**: `crates/perf/src/alloc.rs` (sample mode hunk, off by default like the PF2 atomic switch) → consumers SL21 submit-path audit, PF1 budget attribution, PQ3 long-soak trending
- **Baseline**: exact thread-local counting (authoritative, enforced in default suite). Proposed: sampling mode for long/overhead-sensitive runs only; gates never read sampled numbers.
- **URL + ID**: https://arxiv.org/abs/2506.16883, arXiv:2506.16883v1 (sampling + overhead-measurement discipline only)
- **License**: UNVERIFIED. Ideas only.
- **Paper claim**: 4 MB period → ≤25% time overhead on their GC-VM benchmarks (their runtime, their benchmarks — NOT our numbers).
- **Reproduced result**: NONE YET. Our numbers must be measured on the lane machine, release, single-threaded (`--perf` rules).
- **Assumptions**: sampling never feeds gates, hashes, or replays (HASH-LAW tier 3: alloc counts authoritative stay exact; sampled series are presentation/diagnostic).
- **Counterevidence**: engine allocators are exact-counted Rust, not a GC VM — the paper's collector integration does NOT transfer; sampling buys overhead control but loses exactness, so it can only *attribute*, never *enforce*. Allocator-security papers from the first search (FreeGuard/CAMP/SJMalloc) are do-not-take (security ≠ attribution).
- **What does NOT transfer**: any GC-VM code, any change to the exact floors, any sampled number in a gate.
- **Experiment sketch (S)**: add `sample_period_bytes` knob (default off); run `ArenaTick` canonical + SL21 submit path at 3 periods; record overhead % vs un-profiled + attribution agreement (does the sampler name the same top allocator as the exact floors?); mutation check (remove an SL18–SL20 reuse, sampler must see the floor rise).
- **Benchmark/ablation**: exact-only vs sampled at each period (overhead %, top-attribution match/mismatch); sampler-off must be one atomic load + branch (PF2 precedent).
- **Success threshold**: overhead posted per period with a ceiling the plan accepts; top-attribution agrees with exact floors on all 3 reuse sites; sampler-off overhead unmeasurable in `--perf`.
- **Kill criterion**: attribution disagrees with exact floors twice (same fingerprint → BLOCKERS.md); any sampled number leaks into a gate or hash; off-state costs a per-tick alloc (SL6 violation).
- **Board row proposal**: PROPOSE one S row under SL21/PF2 (sample-mode hunk, no new file) — orchestrator decides; otherwise folds into SL21 acceptance. EVIDENCE row only if it lands.
- **Stamp**: REPRODUCE (proposed).
- **EMPIRE outcome 4**: 24/7 brief→game loops need always-affordable attribution — exact floors for gates, cheap sampling for the fleet.

### Card 3 (TAKE, method — NO arxiv donor): capture pacing pinned to the fixed-step accumulator

- **Question**: with no usable arXiv donor (negative search logged above), can the engine post the capture-pacing rule the fixed-step loop needs — captures ride the accumulator cadence, never the wall clock, never blocking the tick — so SHOW1 sequences and Q1/PF9 captures are reproducible?
- **Engine home**: window/capture path (RA8 manifest, SHOW1 PNG+manifest stitch) + `crates/perf/src/latency.rs` staged timestamps as the measurement side; consumers Q1 reference captures, PF9 rerun debugger, GP18 feel gate
- **Baseline**: PNG+manifest today with no posted cadence rule; video-sweep verdict (SHOW1 external stitch, encoder deps DROP) stands.
- **URL + ID**: none (negative result). Mismatches cited so no future wave re-hunts: PaceMaker 2408.15001 (narrative pacing ≠ frame pacing), 2507.09798 (network pacing ≠ capture cadence), adaptive-timestep physics papers (solver stability ≠ accumulator discipline). Method source: engine's own fixed-step + accumulator practice (textbook, no paper claimed).
- **Paper claim**: N/A — keep distinct: no reproduced paper result here, only an engine measurement rule.
- **Experiment sketch (S)**: post the rule (capture every Nth tick on the accumulator; spiral-of-death clamp named; capture work off the tick thread or budgeted behind the PF2 switch; manifest records tick ids + cadence); run WIN1 loop with captures on/off; compare tick hashes x2 + frame cadence log.
- **Benchmark/ablation**: captures on vs off (tick hash equality, p50/p95 frame ms via PF2, manifest tick-id continuity); N=1 vs N=2 vs N=6 cadences (sequence smoothness vs cost); uncapped-present vs vsync-capped present.
- **Success threshold**: hashes identical x2 with captures on; manifest tick ids continuous with no duplicates/drops; capture cost posted per cadence on lane machine; Q1/PF9 rows can cite the cadence.
- **Kill criterion**: any hash drift with captures on → reject the capture path change immediately; capture allocates on the tick thread (SL6 violation) unless behind the atomic gate; cadence needs per-scene tuning (then it is a demo, not a gate).
- **Board row proposal**: none — acceptance detail for SHOW1/Q1/PF9 rows. EVIDENCE row only if the rule lands.
- **Stamp**: TAKE (method, donor-free, honestly labeled).
- **EMPIRE outcome 4**: generated games need watchable proof — reproducible capture cadence is what makes brief→game reels comparable run to run.

### Card 4 (TAKE, REPRODUCE): change-point ensembles + local-to-system bridge for budget-regression statistics

- **Question**: does the 2606.18377 ensemble discipline (vote over change-point detectors on perf time series, scored with false-alarm accounting on practitioner-annotated data) plus the 2408.08148 local→system bridge (component deviations propagated through an architecture model) catch seeded PF1/PQ3 regressions earlier and with fewer false alarms than today's exact today-vs-recorded gate?
- **Engine home**: `crates/perf` budget history + `crates/proof` soak harness (PQ3); held slot respected: PQ5 STA/LTA stays the soak-picker bet — Card 4 is the *statistics layer over it*, head-to-head scored, winner-takes-the-slot by numbers, never a duplicate row
- **Baseline**: PF1/PF10 exact gate (today must equal recorded) — precise, but a single-point comparison with no trend, no confidence, no false-alarm rate. Proposed: offline trend analysis *alongside* the exact gate; the gate still fails the build, statistics only explain and early-warn.
- **URL + IDs**: https://arxiv.org/abs/2606.18377 (primary, ensemble + annotation method) + https://arxiv.org/abs/2408.08148v1 (secondary, local→system early prediction)
- **License**: both UNVERIFIED. Ideas only — std-only reimplementation, no dependency.
- **Paper claims**: Mozilla study: 12.5% false-positive groups, 6.8% missed-regression groups in the shipped pipeline; ensembles ≈ +11% F1 over the single method (their data, their pipeline). Bridge paper: local deviations predict system regressions pre-deploy on two benchmarks (their systems).
- **Reproduced result**: NONE YET — our series (alloc bytes/count, ms per workload, soak alloc slope) do not exist as annotated history yet.
- **Assumptions**: statistics never weaken the exact gate (quality-plan slim rule: baselines append-only, never edited to fit); wall-clock ms stays presentation-only per HASH-LAW; ensembles run offline/operator-side, never in the tick.
- **Counterevidence**: the Mozilla result itself warns offline/hybrid detectors buy recall with precision — a naive port would spam alerts on the lane board; the bridge paper's two-benchmark scope is narrow vs our workload matrix. Wave-23 lesson holds: Richardson-style second pickers get DECLINED as duplicate rows — Card 4 merges into PQ3/PQ5 acceptance, never a new picker row, unless head-to-head numbers say otherwise.
- **What does NOT transfer**: Mozilla thresholds/alert routing as acceptance; any weakening of PF1 exactness; any in-tick statistics.
- **Experiment sketch (S, 1 session)**: collect short perf series (PF1 workloads × tracks, seeded regressions at known commits: linear creep, step, sawtooth); score T-test baseline vs 2–3 classical detectors (CUSUM/PELT + STA/LTA-as-PQ5) vs small vote-ensemble; local→system check: per-workload deviation propagated through the workload→frame composition (does a component wobble predict the end-to-end miss?).
- **Benchmark/ablation**: detection delay (commits-to-flag) + false-alarm rate on clean series for each detector and the ensemble; ensemble-vs-single F1 on our seeded set; bridge precision (predicted system misses that actually trip PF1).
- **Success threshold**: ensemble names every seeded regression within the posted delay with zero false alarms on the clean set; PQ5 head-to-head scored on the same tapes — winner becomes PQ3's picker detail, loser dissolves (no duplicate slot).
- **Kill criterion**: any detector needs per-shape tuning (demo, not gate); false alarms on clean series; any proposal to edit a baseline row to fit (rejected at review per slim rule); duplicates PQ5's slot — merge, don't row.
- **Board row proposal**: none — acceptance detail for PQ3/PQ5 + PF1 trending note. EVIDENCE row only if a detector lands.
- **Stamp**: REPRODUCE (proposed).
- **EMPIRE outcome 4**: money-machine reliability — trend statistics are what let the 24/7 loop fleet catch slow leaks before players do.

### Card 5 (REJECT): lifts of any donor pipeline as-is

- **Question**: lift the multi-agent rename framework, the GC-integrated sampler, the narrative pacer, or the Mozilla alert pipeline straight into the engine?
- **Verdict**: REJECT all four lifts; Cards 1–4 take the patterns through engine gates instead.
- **Reasons (one line each)**: agent framework needs model edit rights the provider law forbids in-repo; GC sampler integration mismatches exact-counted Rust floors; narrative pacer solves story beats, not accumulator cadence; Mozilla pipeline thresholds belong to their CI, and unported recall-chasing would spam our gates (their own precision warning).
- **Revisit trigger**: Card 1/2/4 spikes beat baselines with receipts on engine fixtures — then the combination is already what Cards 1–4 describe. No new evidence needed beyond that.

---

## What does NOT transfer (explicit)

- Any model-written code path into crates, tick, or gates (provider law; HASH-LAW).
- Any donor source bytes (all UNVERIFIED; reimplement ideas only, std-only, no new dependency).
- GC-VM sampler internals; allocator-security machinery (different problem).
- Narrative/network pacing numbers as frame-cadence acceptance (domain mismatches).
- Mozilla thresholds, alert routing, or any baseline edit to fit a run (slim rule).
- Any statistics, sampling, or capture math leaking into the tick or hash (HASH-LAW tier 3; Cards 2–4 are diagnostic/presentation/offline by construction).
- A second benchmark harness, vendor GPU-capture pipelines, or per-shape-tuned detectors (CUT / kill rules above).

---

## Integration homes (per take)

| Take | Subsystem plan | EVIDENCE home | Consumer |
|---|---|---|---|
| Card 1 declared-scope rename | `docs/plan/general.md` GE11 | `docs/EVIDENCE.md` refresh row | GE11 ledger, GP5 engine-diff-empty gate |
| Card 2 sampling profiler | `docs/plan/quality.md` SL21/PF2 | `docs/EVIDENCE.md` profiler row (only if lands) | SL21 submit audit, PQ3 soak trending |
| Card 3 capture pacing rule | `docs/plan/quality.md` SHOW1/Q1/PF9 (via render-2040 RA8 manifest) | `docs/EVIDENCE.md` rule row (only if lands) | Q1 captures, PF9 rerun, GP18 feel gate |
| Card 4 CPD ensemble + bridge | `docs/plan/quality.md` PQ3/PQ5 + PF1 trending | `docs/EVIDENCE.md` only if a detector lands | 24/7 loop fleet, PF1 budgets |
| Card 5 reject | — | — | — |

## Paper claim vs reproduced result vs integrated gain (kept distinct)

- **Paper claims**: 2601.00482 coordinated-rename wins on their corpus; 2506.16883 ≤25% overhead at 4 MB period on their GC-VM benchmarks; 2606.18377 +11% ensemble F1 with 12.5%/6.8% pipeline error rates on Mozilla data; 2408.08148 early local→system prediction on two benchmarks. All theirs — quoted, never adopted as engine numbers.
- **Reproduced results**: NONE — no spike has run for any card. Cards 1/2/4 propose reproductions with engine fixtures and kill rules; Card 3 is donor-free method work.
- **Integrated gains**: NONE — GE11 refresh unrun, no sampler mode, no pacing rule, no detector landed. Any future gain is measured in missed-site counts, overhead %, hash equality, detection delay / false alarms — never quoted from a paper.

---

## Budget and next step

Budget M 20 min: four query families (one hit-rich, one refined hit, one negative, one double-hit) + five abstract reads + plan/code cross-checks + this note. At budget stop: takes + next step reported, no code touched.

Next step (orchestrator): file Card 1 as the GE11 experiment sketch, Cards 3–4 as acceptance details on SHOW1/Q1/PF9 and PQ3/PQ5 (no new rows), decide whether Card 2 earns its S row or folds into SL21; log the capture-pacing negative search so no future wave re-hunts fixed-step pacing on arXiv without new evidence.

## Source traceability

- ArXiv queries 2026-09-26: `large-scale refactoring rename strategy codebase migration` → 2601.00482, 2202.00173 (+6 others); `per-tick allocation profiler heap allocation game engine` → allocator/game-theory mismatches (logged); `"allocation profiling" sampling overhead memory profiler` → 2506.16883; `fixed timestep frame pacing capture determinism simulation` → physics-timestep mismatches; `game loop fixed timestep accumulator real-time frame pacing` → PaceMaker/network/neural mismatches (logged); `performance regression detection statistical budget benchmark` → 2606.18377, 2408.08148.
- Abstracts verified live via arxiv tools 2026-09-26 (2601.00482, 2202.00173, 2506.16883, 2606.18377, 2408.08148). No PDFs fetched; citation graphs not needed for method cards.
- Code cross-checks: `docs/plan/general.md` (GE2 done, GE11 open, GE6-before-GE11 order), `docs/plan/quality.md` (PF1/PF10 exact gates, PF2 atomic switch, SL6/SL18–SL20 closed, SL21 open, PQ5 held/PQ6 declined), `crates/perf/src/{lib,alloc,baselines,budget}.rs` (counting allocator + exact rows confirmed via grep).
- Donor dedup: DDGI-Resampling (wave 44) and COSA-GS (wave 43) not repeated; Split-RC stays LATER.
- EMPIRE: outcome 4 ("AI makes whole games" — brief→game needs rotting-proof generality, affordable fleet attribution, reproducible captures, leak-free loops).
