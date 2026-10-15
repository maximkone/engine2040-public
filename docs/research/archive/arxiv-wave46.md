# ARXIV Wave 46 — Dive 13: hard-problem queue (phased move-table validation, evidence-digest design, transcription vs machine receipts, capture pacing)

**Dive**: 13 (hard-problem queue: GP23 phased move-table validation, OB1/AX6 evidence-digest design, evidence-transcription vs machine receipts, capture pacing). Wave 46 RES-ARXIV packet.
**Date**: 2026-09-26
**Budget**: M 20 min
**Donor rotation**: DDGI-Resampling (2108.05263, wave 44), COSA-GS (wave 43), allocation-sampling profiler (2506.16883, wave 45), coordinated rename (2601.00482, wave 45), change-point ensembles (2606.18377 + 2408.08148, waves 44–45) explicitly NOT repeated — no new evidence on any of them. Split-RC stays verdict-LATER, cited as context only.

---

## Papers (donors, in own words)

**arXiv:2607.06514v1** — *FootsiesGym: A Fighting Game Benchmark for Two-Player Zero-Sum Imperfect-Information Games* (McDonald, Tsang, Kerr, Jul 2026). cs.AI/cs.GT. https://arxiv.org/abs/2607.06514 — License UNVERIFIED on arXiv page. Ideas only, reimplemented never copied.
**Reproduced claim (paper's, in own words)**: built on a minimalist 2D fighting game, the authors isolate the cyclic non-transitive neutral game so standard hardware can train and compare learning algorithms fast and repeatably, with a vectorized simulator and published code for reproducibility.

**arXiv:2609.12582v1** — *NovaFabric: Tamper-Evident, Replayable Evidence for Autonomous AI Agent Runs* (Ardebili, Sep 2026). cs.CR/cs.DC. https://arxiv.org/abs/2609.12582 — License UNVERIFIED. Ideas only.
**Reproduced claim (paper's, in own words)**: an agent run is recorded without touching agent logic into a portable fifteen-entity Run Capsule, sealed with a signature plus timestamp plus Merkle log plus a redaction attestation, re-runnable under a four-mode replay protocol, and exportable as an Evidence Bundle checkable with stock tooling. Measured scope, in their words: mocked replay serves every model response from the capsule with no live model call (10/10) but only 2/10 tool-using workloads complete (missing tool-response substitution); tampering rejected across three tested classes; declared-stream completeness 0.652 (±0.064, ten scenarios); repaired rule pack redacts 14/14 credential types preserving 9/9 decoys; diff localises 140/140 mutations; blast-radius queries 45.5 ms p99 over 10M edges; capsule REST ingest lossless but capped at 61.6 req/s (p99 26.8 s) by per-worker serialisation; six defects found (four fixed, one withdrawn, one open).

**arXiv:2606.04193v1** — *Notarized Agents: Receiver-Attested Confidential Receipts for AI Agent Actions* (Figuera, Jun 2026). cs.CR/cs.AI/cs.DC. https://arxiv.org/abs/2606.04193 — License UNVERIFIED. Ideas only. Counterevidence / second take for the receipts card, not a second digest donor.
**Reproduced claim (paper's, in own words)**: instead of trusting the agent's own log, the service receiving each agent call signs what it saw with its own key, encrypts the receipt to the owner's public key bound to the auth token, publishes it to a witness-cosigned Merkle log, and the owner rediscovers the trail by token reference (protocol Sello: receiver-side signing, HPKE-to-owner, public log, discovery by reference; microbenchmarks of the crypto ops only, no engine benchmark). Known limits they state: suppression attack, service collusion, adoption incentives.

**arXiv:2309.10089v1** — *HTEC: Human Transcription Error Correction* (Sun et al., Sep 2023). eess.AS/cs.AI/cs.CL/cs.HC. https://arxiv.org/abs/2309.10089 — License UNVERIFIED. Ideas only.
**Reproduced claim (paper's, in own words)**: human transcripts still err enough to matter (they cite ~2% ASR WER per 1% transcription WER when training on them), and generic ASR/grammar correctors underperform on human errors — so they split correction in two: a checker model that finds and masks bad words (with four novel ops for deletions plus phoneme-aware embeddings), then a sequence-to-sequence filler that rewrites the masked slots. They report beating other methods by a large margin, beating human annotators by 2.2–4.5% WER, and a deployed co-pilot improving transcription quality 15.1% without losing velocity.

**Negative result (searched, logged so no future wave re-hunts without new evidence)**:
- Capture pacing for fixed-step: wave 45 already ran the two query families (`fixed timestep frame pacing capture determinism simulation`, `game loop fixed timestep accumulator real-time frame pacing`) to mismatches (PaceMaker narrative pacing, LEO network pacing, solver-stability adaptive timesteps) and filed Card 3 donor-free. Dive 13 reuses that verdict — Card 4 below is a method card, not a new hunt.

---

## Engine home

- Primary: `docs/plan/gameplay.md` (PLN7, GP23 phased move-table schema, HR3 tables, GP18 feel gate, PQ2 tournament), `docs/plan/quality.md` (PLN14/PLN29: OB1 digest, AX6 evidence line via ai-native, PR1 provenance, SHOW1/Q1/PF9 captures, PF2 profiler), `docs/plan/ai-native.md` (PLN9: EA1 transcripts, EV1 eval, AX5 trace export, AX6–AX9 evidence lines)
- Consumers: GP23 row (phased tables), `crates/observe` digest + `crates/playtest` eval evidence lines, ED7 journal / PR1 provenance receipts, WIN1/Q1 capture path
- Subsystem plans: `docs/plan/gameplay.md`, `docs/plan/quality.md`, `docs/plan/ai-native.md`
- EVIDENCE home: `docs/EVIDENCE.md` (one row per method adoption)
- RESEARCH-VERDICT home: `docs/RESEARCH-VERDICT.md` (wave 46 section)
- SPRINT: GP23, HR3, GP18, PQ2, OB1, AX6, PR1, SHOW1/Q1/PF9 (PQ5 held STA/LTA moonshot, PQ6 declined wave 23 — nothing here duplicates that slot)

---

## Simpler baseline (what the engine already has)

- **Phased tables PROPOSED, not built**: GP23 is `[ ]` open in `docs/plan/gameplay.md` §7 (startup/active/endlag + gatling/meter cancels, 2 fighters fully phased, cancels window-gated, PQ2 band); HR3 versioned move tables are ticked (`crates/combat/src/tables.rs`, 20/20, validated swaps) with resolvers frozen beside `exchange.rs`/`sweep.rs`. No phased-fixture benchmark exists.
- **Digest DONE, evidence-bundle shape MISSING**: OB1 typed events + rings + count-desc/code-asc digest is unit-tested/integrated with MCP `log.digest`/`log.query` consumers; EA1 transcripts + EV1 eval rows + AX6 evidence-line proposals exist. There is no portable sealed bundle (schema + hash + redaction attestation + replay recipe) — receipts are per-row, not per-run exportable.
- **Machine receipts DONE, human-transcript correction MISSING**: journal → apply → re-read → rollback (ED7), PR1 provenance receipts (58/58), per-hit/per-tick receipts (MP5, LM1) are landed. EA1/EV1 transcripts have no checker/filler pass: a human-edited transcript ships as-edited with no error-localisation, and no rule says when a human edit beats a machine receipt.
- **Capture cadence RULE open**: fixed tick + golden `d57b5c8ae3096614` x2 DONE; PNG+manifest (SHOW1 external stitch, encoder deps DROP) DONE; wave-45 Card 3 posted the accumulator-pinned pacing rule as acceptance detail for SHOW1/Q1/PF9 — still unwired, cited here, not re-hunted.

---

## Ranked cards (max 5)

### Card 1 (TAKE, REPRODUCE): minimalist phased-fighter benchmark as the GP23 validation harness

- **Question**: does the FootsiesGym discipline — a *minimalist* fighter that isolates one strategic slice (here: phased windows + cancels) on a vectorized/fast fixture so validation runs in seconds — reproduce on the engine as the GP23 acceptance harness: every phased move provably window-gated, every cancel provably legal, with a PQ2-band tournament that fails on a seeded illegal cancel?
- **Engine home**: `docs/plan/gameplay.md` GP23 (table hunks beside `crates/combat/src/exchange.rs` + `sweep.rs`, resolvers frozen); consumers HR3 tables, GP18 feel gate, PQ2 tournament
- **Baseline**: HR3 validated swaps (versioned tables, ticked). Proposed: a `phased_footsies` fixture — 2 fighters, 3–4 moves with explicit startup/active/endlag + 1–2 gatling cancels — running the 600-tick staged-fight shape, seeded, hash-exact.
- **URL + ID**: https://arxiv.org/abs/2607.06514, arXiv:2607.06514v1 (benchmark-isolation + vectorized-fixture discipline only)
- **License**: UNVERIFIED. Ideas only — no simulator code taken, no HiFight/ Footsies bytes, std-only fixture.
- **Paper claim**: minimalist isolation + vectorized sim makes neutral-play learning efficient, accessible and reproducible on standard hardware (their game, their RL benchmarks — NOT our numbers).
- **Reproduced result**: NONE YET — no phased fixture exists. Thresholds below are bars, not forecasts.
- **Assumptions**: resolvers stay frozen during the spike (GP23 plan constraint); float never enters the tick (HASH-LAW); RL training itself is LATER per verdict — only the *fixture + validation* discipline transfers, never the learning algorithms.
- **Counterevidence**: the paper benchmarks RL algorithms, which the verdict parks post-alpha (bot harness LATER) — the take is the harness shape, not the learners. Minimalist isolation risks missing arena realities (8-fighter density, gate-tunnel occlusion, 250-torch night) — so the fixture is GP23's *unit gate*, and the PQ2 tournament on full content stays the integration gate.
- **What does NOT transfer**: any RL algorithm/rankings as acceptance; HiFight/Footsies game code or assets; paper training numbers as engine bars.
- **Experiment sketch (S, 1 session)**: build the 2-fighter phased fixture with window-exact assertions (startup frames never hit, active frames always resolve through the frozen resolver, endlag never cancels except listed gatlings); seed one illegal cancel and one off-by-one window; run fixture + PQ2-band check.
- **Benchmark/ablation**: fixture runtime (must be seconds, vectorized-batch shape if trivially parallel) vs full staged-fight cost; window assertions on vs off (seeded illegal cancel caught/missed); minimalist-fixture verdict vs full-content PQ2 verdict agreement on the same seeded fault.
- **Success threshold**: all window/cancel assertions green; both seeded faults fail loudly with the offending window named; fixture runs in seconds on the lane machine; golden x2 unchanged.
- **Kill criterion**: fixture needs per-move tuning to pass (demo, not gate); seeded illegal cancel passes silently twice (same fingerprint → BLOCKERS.md, GP23 stays resolver-frozen HR3); any tick/hash drift from the harness.
- **Board row proposal**: none — experiment sketch for existing GP23. EVIDENCE row on the fixture if it lands.
- **Stamp**: REPRODUCE (proposed).
- **EMPIRE outcome 4**: generated fighters need instant provable legality — a seconds-fast phased gate is what lets brief→game ship moves without a human frame-data audit.

### Card 2 (TAKE, REPRODUCE): sealed run-capsule discipline for the OB1 digest + AX6 evidence line

- **Question**: does the NovaFabric integration pattern — record the run into a portable schema, seal it (hash + timestamp + Merkle/log + redaction attestation), make it re-executable from the capsule alone, export a third-party-checkable bundle with stock tooling — reproduce on the engine as the OB1→AX6 evidence shape, at engine scale (no 314-machine ingest, no crypto novelty)?
- **Engine home**: `crates/observe` digest (`log.digest`/`log.query`) → `crates/playtest` AX6 evidence lines (`EvalReport::to_text` seam, needs EV1 + MCP5); PR1 provenance receipts ride along; consumers EV2 brief-to-game eval, QA1 bug tapes, factory showcase claims
- **Baseline**: per-row digests + receipts (landed, queryable). Proposed: per-*run* sealed bundle — schema version + event/tape/seed/build rows + FNV/hash seal (SL5 `engine_core::Fnv64`, never hand-rolled crypto per law) + redaction attestation + replay recipe.
- **URL + ID**: https://arxiv.org/abs/2609.12582, arXiv:2609.12582v1 (integration pattern only: capsule schema + seal + four-mode replay + exportable bundle)
- **License**: UNVERIFIED. Ideas only — no NovaFabric code, no new crypto (DSSE/in-toto/OTel are *named*, not vendored).
- **Paper claim**: mocked replay 10/10 model responses with no live call but only 2/10 tool workloads (missing substitution); 0.652 declared-stream completeness; 14/14 redactions with 9/9 decoys kept; 140/140 diff localisation; 45.5 ms p99 blast-radius over 10M edges; ingest lossless at 61.6 req/s cap (all theirs — quoted, never adopted).
- **Reproduced result**: NONE YET. Our numbers must be measured on lane fixtures (bug-bundle tapes, EV1 75-row runs).
- **Assumptions**: no new cryptography (law: never hand-roll crypto; FNV/hash + existing journal seal suffice for a game-evidence bundle); redaction = secret-scrub attestation for provider keys in traces, never a security proof; replay = mock/local backends from recorded outputs, never a live model call (HASH-LAW + provider law).
- **Counterevidence**: the paper's own numbers warn the pattern is incomplete — 2/10 tool workloads, 0.652 completeness, ingest capped at 61.6 req/s — so the engine take scopes to *deterministic* runs (seeds + tapes + recorded model outputs) where mocked replay is already the law, and claims completeness only over declared streams with the gap posted, never hidden. Six-defects-found honesty is the part to copy: ship the defect log with the bundle.
- **What does NOT transfer**: any crypto implementation, any ingest-service scaling claim, paper completeness/threshold numbers as acceptance, live-model replay.
- **Experiment sketch (S)**: seal one EV1 fixture run + one QA1 bug tape as a bundle (schema v1, hash seal, redaction attestation over a seeded fake secret, replay recipe); third-party check = a second lane replays from the bundle alone with stock tools (no donor code); mutate one tape byte and one receipt row → diff must localise both.
- **Benchmark/ablation**: bundle replay vs live run (hash equality x2); mutated vs clean (diff localisation hit/miss); declared-stream completeness posted on 10 fixture runs (report the gap like the paper does); bundle size vs tape size.
- **Success threshold**: replay-from-bundle hash-identical x2 with zero live calls; 140/140-style mutation localisation on the seeded set (our count posted); redaction attestation names every seeded secret with zero decoy loss; completeness gap posted honestly.
- **Kill criterion**: any replay needs the original lane state (then it is not a capsule); completeness below posted bar twice with the same fingerprint → BLOCKERS.md, per-row digests stay the answer; any crypto or secret bytes in git.
- **Board row proposal**: PROPOSE one S row under AX6 (bundle shape on the `to_text` seam) — orchestrator decides; otherwise folds into AX6 acceptance. EVIDENCE row only if it lands.
- **Stamp**: REPRODUCE (proposed).
- **EMPIRE outcome 4**: brief→game proof must be portable — a sealed bundle is what lets the factory sell what the engine proved without "trust our logs."

### Card 3 (TAKE, REPRODUCE with counterevidence built in): checker-plus-filler for human-edited transcripts, receiver-attested receipts stay machine-side

- **Question**: can the HTEC two-stage split — a checker that finds-and-masks bad words (with explicit deletion ops + phoneme-aware embeddings) then a filler that rewrites only masked slots — reproduce as the rule for *human-edited* EA1/EV1 transcripts (detect → mask → refill-or-escalate), while *machine* receipts stay receiver/journal-sealed per the Notarized-Agents inversion (receiver signs what it saw; the agent's own log is never the authority)?
- **Engine home**: `crates/playtest` transcripts + AX5 trace export (human-edit path) vs ED7 journal / PR1 / MP5-LM1 receipts (machine path); consumer EV1 75-row integrity, EA1e multi-backend parity
- **Baseline**: transcripts ship as-edited today (no correction pass); receipts are journal-sealed (landed). Proposed: every human edit passes checker → masked slots refill from the recorded run (deterministic fill first) or escalate to a human with the mask shown; machine receipts never accept a human fill.
- **URLs + IDs**: https://arxiv.org/abs/2309.10089, arXiv:2309.10089v1 (two-stage detect-then-fill + deletion ops + co-pilot discipline) + https://arxiv.org/abs/2606.04193, arXiv:2606.04193v1 (receiver-side signing inversion + suppression/collusion limits as counterevidence)
- **License**: both UNVERIFIED. Ideas only — no model weights, no crypto code; checker can be rules/lexical first (verdict: lexical before embeddings), filler is deterministic replay fill, never a generative model in the tick path.
- **Paper claims**: HTEC beats baselines by a large margin, beats annotators 2.2–4.5% WER, co-pilot +15.1% quality at equal velocity (their data, their models); Sello microbenchmarks crypto ops only with stated suppression/collusion limits (their protocol, no engine benchmark).
- **Reproduced result**: NONE YET. Our metric is seeded-error localisation + fill agreement on EA1/EV1 fixtures, not WER.
- **Assumptions**: lexical/rules checker first (verdict MINIAI line); phoneme-awareness transfers as *domain-aware features* (move names, window ids, token refs), not embeddings; filler priority: recorded-run fill > human confirm > reject; HAVOC: human edits never touch sealed receipts.
- **Counterevidence**: HTEC trains neural checker+filler models — verdict parks trained classifiers until M2+ real logs exist (LATER), so the engine reproduction is the *split discipline* with rules/deterministic fill, and neural stages wait for the log corpus. Notarized Agents warns of suppression attacks (a compromised operator drops receipts) and collusion — hence machine receipts stay inside the engine journal (no external log dependency), and the Sello discovery-by-reference shape transfers only as *token-referenced receipt lookup* in `log.query`, never as an external transparency log.
- **What does NOT transfer**: any trained weights or WER numbers as acceptance; generative fill inside gates; external Merkle/transparency-log dependency; crypto protocol bytes.
- **Experiment sketch (S)**: seed 3 human-edit fault shapes into EA1/EV1 transcripts (substitution, deletion, insertion) at known rows; run rules-checker → mask → deterministic refill from the recorded run; score localisation + fill agreement + escalations; verify machine receipts untouched by construction.
- **Benchmark/ablation**: checker vs no-checker (seeded faults caught/missed, false masks on clean transcripts); deterministic-fill vs escalate (agreement rate, escalation count); receipt-tamper probe (edited receipt row must fail journal re-read → rollback, per ED7).
- **Success threshold**: every seeded fault localised with the mask shown; fills agree with the recorded run on all deterministic slots; zero false masks on 3 clean transcripts; receipt probe rolls back loudly.
- **Kill criterion**: checker needs per-fault tuning (demo, not gate); any human fill lands in a sealed receipt (reject immediately); false masks on clean transcripts twice with the same fingerprint → BLOCKERS.md, transcripts ship as-edited with masks only advisory.
- **Board row proposal**: none — acceptance detail for EA1/EV1 transcript handling + AX5 export; EVIDENCE row only if the split lands.
- **Stamp**: REPRODUCE (proposed).
- **EMPIRE outcome 4**: agent-built games mix machine traces and human touch-ups — the split is what keeps human fixes helpful without ever forging machine proof.

### Card 4 (TAKE, method — NO arxiv donor): capture pacing pinned to the fixed-step accumulator (wave-45 rule, wired here)

- **Question**: with no usable arXiv donor (wave-45 negative search stands, not re-hunted), can the engine wire the posted capture-pacing rule — captures ride the accumulator cadence (every Nth tick stills vs every-presented-frame named in the manifest), spiral-of-death clamp named, capture work off the tick thread or behind the PF2 atomic gate, manifest records tick ids + cadence — so SHOW1 sequences and Q1/PF9 captures are reproducible?
- **Engine home**: window/capture path (RA8 manifest, SHOW1 PNG+manifest stitch) + `crates/perf/src/latency.rs` staged timestamps as measurement; consumers Q1 reference captures, PF9 rerun debugger, GP18 feel gate
- **Baseline**: PNG+manifest today with the rule posted-but-unwired (wave-45 Card 3). This card is the wiring sketch for existing rows.
- **URL + ID**: none (negative result carried from wave 45: PaceMaker 2408.15001 narrative pacing ≠ frame pacing; 2507.09798 network pacing ≠ capture cadence; adaptive-timestep physics = solver stability ≠ accumulator discipline).
- **Paper claim**: N/A — keep distinct: no reproduced paper result here, only an engine measurement rule.
- **Experiment sketch (S)**: post the rule in the manifest schema; run WIN1 loop captures on/off; compare tick hashes x2 + frame cadence log + tick-id continuity.
- **Benchmark/ablation**: on vs off (hash equality, p50/p95 via PF2, manifest continuity); N=1 vs N=2 vs N=6 (smoothness vs cost); uncapped vs vsync-capped present.
- **Success threshold**: hashes identical x2 captures-on; tick ids continuous, no duplicates/drops; per-cadence cost posted on lane hardware; Q1/PF9 cite the cadence.
- **Kill criterion**: any hash drift captures-on → reject the path change; capture allocs on the tick thread (SL6 violation) unless atomically gated; cadence needs per-scene tuning (demo, not gate).
- **Board row proposal**: none — acceptance detail for SHOW1/Q1/PF9. EVIDENCE row only if the rule lands.
- **Stamp**: TAKE (method, donor-free, honestly labeled).
- **EMPIRE outcome 4**: generated games need watchable comparable proof — reproducible cadence is what makes brief→game reels rankable run to run.

### Card 5 (REJECT): lifts of any donor pipeline as-is

- **Question**: lift FootsiesGym's simulator, NovaFabric's sealed stack, Sello's transparency log, or HTEC's trained models straight into the engine?
- **Verdict**: REJECT all four lifts; Cards 1–3 take the patterns through engine gates instead.
- **Reasons (one line each)**: Footsies simulator bytes are a 2D game, not the arena tick — fixture discipline only; NovaFabric's crypto/ingest stack is a service, not a crate — capsule shape only; Sello's external log reintroduces the trust/suppression problem the journal already solves in-engine; HTEC's trained models need the M2+ log corpus that does not exist yet — split discipline with rules fill only.
- **Revisit trigger**: Card 1/2/3 spikes beat baselines with receipts on engine fixtures — then the combination is already what Cards 1–3 describe. No new evidence needed beyond that.

---

## What does NOT transfer (explicit)

- Any donor source bytes (all UNVERIFIED; reimplement ideas only, std-only, no new dependency).
- Any RL algorithm, training number, or WER threshold as engine acceptance (their games/data, never ours).
- Any new cryptography or external transparency/ingest service (law: never hand-roll crypto; journal stays the seal).
- Any live-model replay or generative fill inside ticks, gates, or hashes (HASH-LAW + provider law).
- Any human edit landing in a sealed machine receipt (Card 3 hard boundary).
- Any statistics, sampling, or capture math leaking into the tick or hash (Cards 2–4 are diagnostic/presentation/offline by construction).
- A second benchmark harness, vendor GPU-capture pipelines, or per-shape-tuned detectors (CUT / kill rules above).

---

## Integration homes (per take)

| Take | Subsystem plan | EVIDENCE home | Consumer |
|---|---|---|---|
| Card 1 phased-fighter fixture | `docs/plan/gameplay.md` GP23 (+HR3, PQ2, GP18) | `docs/EVIDENCE.md` fixture row (only if lands) | GP23 row, PQ2 tournament |
| Card 2 sealed run capsule | `docs/plan/quality.md` OB1 + `docs/plan/ai-native.md` AX6 (`to_text` seam) | `docs/EVIDENCE.md` bundle row (only if lands) | EV2 eval, QA1 tapes, factory claims |
| Card 3 checker+filler / receiver-sealed receipts | `docs/plan/ai-native.md` EA1/EV1/AX5 vs ED7 journal + PR1 | `docs/EVIDENCE.md` rule row (only if lands) | EV1 integrity, EA1e parity |
| Card 4 capture pacing rule | `docs/plan/quality.md` SHOW1/Q1/PF9 (via render-2040 RA8 manifest) | `docs/EVIDENCE.md` rule row (only if lands) | Q1 captures, PF9 rerun, GP18 |
| Card 5 reject | — | — | — |

## Paper claim vs reproduced result vs integrated gain (kept distinct)

- **Paper claims**: FootsiesGym efficient reproducible neutral-play benchmarking on standard hardware (their game); NovaFabric 10/10 mocked model replay but 2/10 tool workloads, 0.652 completeness, 14/14 redactions, 140/140 diff localisation, 45.5 ms p99 blast-radius, 61.6 req/s ingest cap (their system); Sello receiver-signed confidential receipts with stated suppression/collusion limits (their protocol); HTEC +2.2–4.5% over annotators, +15.1% co-pilot quality (their data). All theirs — quoted, never adopted as engine numbers.
- **Reproduced results**: NONE — no spike has run for any card. Cards 1–3 propose reproductions with engine fixtures and kill rules; Card 4 is donor-free method work carried from wave 45.
- **Integrated gains**: NONE — no phased fixture, no sealed bundle, no checker pass, no pacing wiring has landed. Any future gain is measured in window-named failures, hash-equal replays, localisation counts, and per-cadence costs — never quoted from a paper.

---

## Budget and next step

Budget M 20 min: three query families (fighting-game hit-rich, tamper-evident double-hit, transcription hit-rich) + one carried negative result + four abstract reads + plan/code cross-checks + this note. At budget stop: takes + next step reported, no code touched.

Next step (orchestrator): file Card 1 as the GP23 experiment sketch, Card 2 as AX6 shape proposal (S row or acceptance fold), Card 3 as EA1/EV1 transcript acceptance detail, Card 4 as SHOW1/Q1/PF9 wiring detail (no new rows); log the Dive-13 donor set so future waves rotate past FootsiesGym/NovaFabric/Sello/HTEC without new evidence.

## Source traceability

- ArXiv queries 2026-09-26: `"fighting game" move validation OR phased animation state machine OR hitbox validation` → 2607.06514 (+7 others, mismatches logged by title); `provenance digest evidence log tamper-evident receipts` → 2609.12582, 2606.04193 (+6 others); `speech transcription human correction vs machine log error recovery` → 2309.10089 (+7 others).
- Abstracts verified live via arxiv tools 2026-09-26 (2607.06514, 2609.12582, 2606.04193, 2309.10089). No PDFs fetched; citation graphs not needed for method cards.
- Code cross-checks: `docs/plan/gameplay.md` (GP23 open PROPOSED, HR3 ticked, resolvers frozen), `docs/plan/quality.md` (OB1 landed, SHOW1/Q1/PF9 open), `docs/plan/ai-native.md` (EA1 transcripts, EV1 75 rows, AX5 export, AX6 evidence-line proposals, ED7 journal/PR1 context).
- Donor dedup: DDGI-Resampling (wave 44), COSA-GS (wave 43), allocation sampling + rename + CPD ensembles (wave 45), Split-RC (LATER) — none re-served.
- EMPIRE: outcome 4 ("AI makes whole games" — brief→game needs provable moves, portable proof, honest transcripts, comparable reels).
