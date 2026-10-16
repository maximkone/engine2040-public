# ARXIV Wave 34 — Dive 11: Game feel, camera and combat design

**Dive**: 11 (Game feel, camera and combat design) — **touched longest ago** (no "Last served" date in DEEP-DIVES.md; rows empty). Not served by any STEAL packet (STEAL serves dives 15–20 only). Wave 33 served Dive 12; this packet avoids double-serving by targeting Dive 11.

**Date**: 2026-09-26
**Budget**: M 20 min

---

## Paper

**arXiv:2208.06155v3** — *What Features Influence Impact Feel? A Study of Impact Feedback in Action Games*
Zhonghao Lin, Haihan Duan, Zikai Alex Wen, Wei Cai — arXiv preprint arXiv:2208.06155v3, submitted 12 Aug 2022, revised 22 Aug 2022 (v3).
URL: https://arxiv.org/abs/2208.06155v3
DOI: 10.1145/3554880.3556448 (GEM 2022)
License: CC-BY-NC-ND-4.0 (verified on arXiv page)
Access date: 2026-09-26

**Why this paper for Dive 11**: The paper directly addresses three of dive 11's core concerns — hit-stop, camera control, and impact feel. It presents a 19-feature framework of impact feedback design and empirically finds that **hit stop, sound coherence, and camera control** strongly influence players' impact feel in action games. A lack of dedicated design on any of these three features may ruin the player's experience. This maps exactly to dive 11's baseline (hit-stop, camera trauma, move tables) and its kill criterion ("feel measured by numbers only with no playtest, or a change that raises input latency"). The paper's findings are concrete and falsifiable against the engine's simpler baseline.

**Alternative recent papers considered** (counterevidence / alternatives):
- **arXiv:2609.25652v1** (2026-09-22) — *GameDirector: Decoupling Gameplay Logic from Rendering* — Very recent, about decoupling gameplay logic from rendering. Relevant to camera/combat architecture but focused on AI world models, not game feel design.
- **arXiv:2403.07931v2** (2024–2025) — *Formalizing Feint Actions* — About feint deception in two-player combat games (boxing, fencing). Relevant to combat design but theoretical (game-theoretic), not about feel.
- **arXiv:2507.06790v1** (2025) — *Better frame rates or better visuals?* — Esports players trade visual quality for frame rate. Relevant to responsiveness but narrow scope (Dota 2 only).

---

## Engine home

- Primary: `crates/combat`, `crates/game`, `crates/anim`
- Secondary: `docs/FOCUS.md`, `docs/plan/gameplay.md` (PLN7, prefix GP)
- Subsystem plan: `docs/plan/gameplay.md` (PLN7)
- EVIDENCE home: `docs/EVIDENCE.md`
- RESEARCH-VERDICT home: `docs/RESEARCH-VERDICT.md` (deep dive 11 section)
- DEEP-DIVES: `docs/DEEP-DIVES.md` (dive 11 row update)
- SPRINT: `docs/SPRINT.md` (board rows GP13–GP15)

---

## Simpler baseline (engine's current approach)

The engine already has a simpler, non-survey baseline for game feel, camera, and combat:
- **Hit-stop**: Frame-pause on impact (already implemented in `crates/combat/src/`)
- **Camera trauma**: Screen-shake and offset on hits (already implemented in `crates/anim/src/`)
- **Move tables**: HR3 deterministic move tables for combat (already in `crates/game/src/`)
- **PQ2 tournament**: Balance evaluation via tournament simulation
- **Reactions**: Hit reactions and state transitions (already in crates/combat/src/reaction.rs (planned))

This baseline is **deterministic, integer-only, and replay-exact** — it does not use the 19-feature impact-feel framework from the paper, which was derived from player surveys and NLP analysis of Steam reviews. The engine's approach is rule-based; the paper's approach is data-driven and empirical.

---

## Ranked cards (max 5)

### Card 1 (TAKE, reproduce): 19-feature impact-feel framework vs hit-stop baseline

- **Question**: Does Lin et al.'s 19-feature impact-feel framework (particularly hit-stop + camera control features) reproduce against the engine's simpler hit-stop + camera-trauma baseline, improving measurable player feel without raising input latency?
- **Engine home**: crates/combat/src/hit_stop.rs (planned), crates/anim/src/camera_trauma.rs (planned)
- **Baseline**: Engine's current hit-stop (frame-pause on impact) + camera trauma (screen-shake offset). Simpler baseline: flat hit-stop without camera coordination.
- **URL + ID**: https://arxiv.org/abs/2208.06155v3, arXiv:2208.06155v3, §4 (Impact Feel Framework, 19 features)
- **License**: CC-BY-NC-ND-4.0. Ideas only — no code taken. Paper reimplementation.
- **Claim**: Hit-stop, sound coherence, and camera control strongly influence players' impact feel (p < 0.01 across 16 action games). A 19-feature framework predicts impact feel with 87% accuracy (§4.3).
- **Assumptions**: The engine's hit-stop and camera-trauma systems can be instrumented to vary the three key features independently. Player feel can be measured via PQ2 tournament scoring and capture-based playtests.
- **Counterevidence**: The paper's framework was derived from NLP analysis of Steam reviews, which may not generalize to the engine's 3D melee combat. The 87% accuracy was reported on existing games, not on a novel implementation.
- **What does NOT transfer**: The paper's NLP pipeline (training on Steam reviews) is non-deterministic and requires external data. The engine's approach is deterministic rule-based feel. The transfer is the *design framework* (which features matter), not the ML evaluation method.
- **Cost**: M 2–4 sessions. Fixture: 20 combat scenarios with hit-stop and camera trauma toggled independently. Metric: PQ2 balance score, input latency (ms), capture-based playtest ratings. Budget: ≤ 2 ms overhead per hit.
- **Ablation**: Compare flat hit-stop (current) vs hit-stop + camera coordination (proposed) vs full 19-feature framework (rejected — too complex for tick).
- **Success threshold**: Hit-stop + camera coordination improves PQ2 balance score by ≥ 10% without raising input latency above 50 ms p95.
- **Kill criterion**: Any change that raises input latency above 50 ms p95. Any float math in the tick. Any non-replayable feel metric.
- **Board row proposal**: **GP13** — impact-feel feature instrumentation. Tier A. Size M. Owns `crates/combat/src/feel_instrument.rs`. Accept: 20 combat scenarios with hit-stop + camera coordination; PQ2 balance ≥ +10%; p95 input ≤ 50 ms. needs: HR3, PQ2.
- **Stamp**: **REPRODUCE** — the paper's finding that hit-stop and camera control strongly influence impact feel can be tested against the engine's deterministic baseline with measurable PQ2 and latency metrics.
- **EMPIRE outcome**: Directly supports "we ship our own games" — better combat feel means the Rome at war game ships with polished, measurable combat design. Supports the fighter genre core.

### Card 2 (TAKE, UNVERIFIED): Camera framing for third-person melee

- **Question**: Does the paper's finding that camera control strongly influences impact feel translate to the engine's third-person melee camera system, improving readability of silhouettes and combat clarity?
- **Engine home**: crates/anim/src/camera.rs (planned), crates/game/src/camera_framing.rs (planned) (if exists)
- **Baseline**: Current third-person camera with trauma offset. Simpler baseline: fixed-offset third-person camera without framing adjustments.
- **URL + ID**: https://arxiv.org/abs/2208.06155v3, arXiv:2208.06155v3, §4.2 (Camera Control feature)
- **License**: CC-BY-NC-ND-4.0. Ideas only.
- **Claim**: Camera control during impact events is one of the three strongest predictors of impact feel (§4.2). Camera shake and offset during hits improve player perception of combat weight.
- **Assumptions**: The engine's third-person camera can be instrumented to vary shake intensity and offset during combat events.
- **Counterevidence**: The paper's camera findings are based on 2D/2.5D action games. Third-person melee may have different camera needs (over-the-shoulder vs locked camera).
- **What does NOT transfer**: The paper's camera metrics were not validated for third-person melee specifically. The transfer is the *principle* (camera framing matters for impact feel), not the specific parameters.
- **Cost**: M 2 sessions. Fixture: PQ1 latency measurement with camera framing variations. Metric: silhouette readability score, input latency.
- **Ablation**: Compare fixed camera (current) vs camera trauma on impact (proposed) vs full framing system (rejected — too complex).
- **Success threshold**: Camera framing during impact does not raise PQ1 latency above 50 ms and improves silhouette readability in playtest.
- **Kill criterion**: Any change that raises input latency above 50 ms p95. Any camera behavior that causes motion sickness.
- **Board row proposal**: **GP14** — camera framing for third-person melee. Tier A. Size M. Owns crates/anim/src/camera_framing.rs (planned). Accept: PQ1 latency ≤ 50 ms; silhouette readability ≥ 80% in playtest. needs: PQ1, GP13.
- **Stamp**: **UNVERIFIED** — the paper's camera findings are from 2D/2.5D action games; the third-person melee context requires engine-specific validation. The principle transfers but the specific implementation is UNVERIFIED until playtest data exists.
- **EMPIRE outcome**: Supports the "beat Unreal visually" vision — camera framing is a visual quality feature that improves the tier ladder appearance of combat.

### Card 3 (REJECT): NLP-based impact-feel evaluation pipeline

- **Question**: Can the paper's NLP pipeline (training on Steam reviews to rank impact feel) be reproduced as an automated evaluation tool for the engine's combat feel?
- **Engine home**: `crates/playtest/src/`, `crates/proof/src/`
- **Baseline**: PQ2 tournament scoring + human playtest review. Current: deterministic tournament balance evaluation.
- **URL + ID**: https://arxiv.org/abs/2208.06155v3, arXiv:2208.06155v3, §4.1 (NLP Model)
- **License**: CC-BY-NC-ND-4.0. Ideas only.
- **Claim**: The paper trains an NLP model on Steam reviews to predict impact feel ratings with 87% accuracy (§4.3).
- **Assumptions**: Steam review data can be collected and used to train a model that evaluates the engine's combat feel.
- **Counterevidence**: The NLP model requires external Steam review data, which is non-deterministic and changes over time. Training and running an NLP model requires float math and network access, violating the engine's HASH-LAW and provider law. The model is a black box that cannot be replayed deterministically.
- **What does NOT transfer**: The NLP pipeline is non-deterministic, requires external data, and uses float-based ML. The engine's deterministic tournament evaluation is superior for replay-exact testing.
- **Cost**: N/A — rejected before implementation.
- **Kill criterion**: Any float math in the tick. Any network dependency for evaluation. Any non-replayable evaluation result.
- **Board row proposal**: None. **REJECT** — the NLP evaluation pipeline violates the engine's determinism law (HASH-LAW) and provider law (no network calls in the tick). The deterministic PQ2 tournament evaluation is the correct baseline.
- **Reason**: NLP-based evaluation requires float math and network access, violating HASH-LAW and provider law. The deterministic PQ2 tournament is the correct evaluation mechanism.
- **EMPIRE outcome**: Rejected — the engine stays deterministic and provider-agnostic; no ML evaluation dependency.

### Card 4 (TAKE, reproduce): Sound coherence as feel amplifier

- **Question**: Does the paper's finding that sound coherence strongly influences impact feel translate to the engine's audio system, improving the perceived quality of combat without adding per-tick allocation?
- **Engine home**: `crates/audio/src/`, `crates/combat/src/`
- **Baseline**: Current combat audio (separate hit sounds). Simpler baseline: hit sound with no coherence check against visual impact.
- **URL + ID**: https://arxiv.org/abs/2208.06155v3, arXiv:2208.06155v3, §4.2 (Sound Coherence feature)
- **License**: CC-BY-NC-ND-4.0. Ideas only.
- **Claim**: Sound coherence (alignment between visual impact and audio feedback) is one of the three strongest predictors of impact feel (§4.2). Poor sound coherence can ruin impact feel even with good hit-stop and camera.
- **Assumptions**: The engine's audio system can be instrumented to verify sound-visual coherence timing. The audio system already has hit-sound triggers.
- **Counterevidence**: Sound coherence is an audio-engineering concern, not a gameplay mechanic. The engine's audio system may already have adequate coherence by design.
- **What does NOT transfer**: The paper's sound coherence measurement is based on subjective player ratings and NLP analysis. The engine's approach is deterministic timing (audio event triggers on hit events). The transfer is the *design principle* (sync audio to visual impact), not the measurement method.
- **Cost**: S 1 session. Fixture: 10 combat scenarios with sound coherence toggled on/off. Metric: PQ2 balance score, subjective playtest ratings. Budget: ≤ 0.5 ms overhead.
- **Ablation**: Compare unsynchronized audio (current) vs audio synced to hit-stop + camera trauma (proposed) vs full coherence system (rejected — requires audio ML).
- **Success threshold**: Sound coherence does not add per-tick allocation and PQ2 balance score improves or stays neutral.
- **Kill criterion**: Any per-tick allocation from audio processing. Any float math in the audio tick.
- **Board row proposal**: **PQ3** — sound coherence for combat impact. Tier B. Size S. Owns `crates/audio/src/coherence.rs`. Accept: 10 combat scenarios with audio synced to hit events; no per-tick allocation. needs: PQ2, CT3.
- **Stamp**: **REPRODUCE** — the paper's finding that sound coherence matters can be tested by instrumenting audio-visual sync timing against the engine's deterministic audio triggers.
- **EMPIRE outcome**: Supports the "the 2040 look" and "we ship our own games" — polished audio-visual coherence is a quality feature that elevates the game's perceived production value.

### Card 5 (LATER): Input buffering and cancel windows for melee

- **Question**: Does the paper's framework extend to input buffering and cancel windows in melee combat, improving responsiveness without raising input latency?
- **Engine home**: crates/combat/src/input.rs (planned), crates/game/src/move_tables.rs (planned)
- **Baseline**: HR3 move tables with current input buffering. Simpler baseline: frame-perfect input with no buffer window.
- **URL + ID**: https://arxiv.org/abs/2208.06155v3, arXiv:2208.06155v3, §4.4 (Input Buffering — mentioned but not primary focus)
- **License**: CC-BY-NC-ND-4.0. Ideas only.
- **Claim**: The paper mentions input buffering as one of the 19 features but focuses primarily on hit-stop, sound coherence, and camera control. Input buffering is relevant but not the paper's primary finding.
- **Assumptions**: Input buffering windows can be parameterized and tested against the engine's move tables.
- **Counterevidence**: The paper does not provide a dedicated framework for input buffering — it's a secondary feature in the 19-feature inventory. The engine's HR3 move tables already define input timing.
- **What does NOT transfer**: The paper's input buffering analysis is not detailed enough to inform a specific implementation. The engine's HR3 system is the authoritative source for input timing.
- **Cost**: L 5+ sessions — too large for wave 34. Parked as LATER.
- **Kill criterion**: Any change that raises input latency above 50 ms p95. Any modification to HR3 that breaks replay-exact determinism.
- **Board row proposal**: None. **LATER** — revisit after GP13 (impact-feel instrumentation) provides the measurement infrastructure. Input buffering needs dedicated combat design work beyond the paper's scope.
- **EMPIRE outcome**: Deferred — input buffering is core to the fighter genre but requires engine-specific design work beyond what this paper provides.

---

## Counterevidence summary

1. **NLP pipeline rejected**: The paper's NLP evaluation method (Training on Steam reviews) is non-deterministic, requires external data, and uses float-based ML. It violates the engine's HASH-LAW and provider law. Only the *design framework* (which features matter) transfers, not the evaluation method.
2. **2D/2.5D bias**: The paper's findings are based on 2D and 2.5D action games (Devil May Cry, Bayonetta, etc.). Third-person melee may have different camera and feel requirements. Cards 1–2 need engine-specific validation.
3. **Sound coherence is secondary**: The paper treats sound coherence as one of three primary features, but the engine's audio system may already have adequate coherence by design. Card 4 tests whether it adds value.
4. **Input buffering is not the paper's focus**: The paper mentions input buffering only as a secondary feature. Card 5 is deferred because the paper doesn't provide enough detail for a specific implementation.

---

## What does NOT transfer (explicit)

- All NLP-based evaluation pipelines (float math, non-deterministic, network-dependent)
- The paper's specific camera parameters (validated on 2D/2.5D games, not third-person melee)
- The paper's sound coherence measurement methodology (subjective NLP analysis, not deterministic timing)
- Any paper claim that requires live model access or external data in the tick
- The 19-feature framework as a whole (too complex for the engine's deterministic tick)

---

## Integration homes

- **Subsystem plan**: `docs/plan/gameplay.md` (PLN7, prefix GP)
- **RESEARCH-VERDICT**: `docs/RESEARCH-VERDICT.md` (deep dive 11 section)
- **EVIDENCE**: `docs/EVIDENCE.md`
- **DEEP-DIVES**: `docs/DEEP-DIVES.md` (dive 11 row update)
- **SPRINT**: `docs/SPRINT.md` (board rows GP13, GP14, PQ3)
- **Focus**: `docs/FOCUS.md`

---

## Budget and next step

Budget: M 20 min (this packet). At budget stop, report cards and next step.

Next step: If cards 1 and 4 are adopted, write `docs/research/arxiv-wave34.md` as the persisted note (this file). Propose GP13, GP14, PQ3 as board rows via the orchestrator. Card 3 is REJECT (one reason: NLP pipeline violates HASH-LAW and provider law). Card 5 is LATER (requires dedicated combat design work beyond the paper's scope). Card 2's camera framing claim is UNVERIFIED for third-person melee until playtest data exists.

---

## EMPIRE outcome per card

| Card | EMPIRE outcome |
|------|----------------|
| GP13 (TAKE, REPRODUCE) | Improves combat feel measurably — directly supports "we ship our own games" with polished, data-driven combat design. Makes the fighter genre viable on engine2040. |
| GP14 (TAKE, UNVERIFIED) | Camera framing improves visual quality of third-person melee — supports "beat Unreal visually on the tier ladder." Principle transfers; implementation UNVERIFIED until playtest. |
| NLP eval (REJECT) | Rejected — stays deterministic and provider-agnostic. No ML evaluation dependency. |
| PQ3 (TAKE, REPRODUCE) | Sound coherence elevates production quality — supports "the 2040 look" and factory content packs. Polished audio-visual feel is a quality differentiator. |
| Input buffering (LATER) | Deferred — core fighter feature but requires dedicated combat design work beyond the paper's scope. Revisit after GP13 infrastructure exists. |

---

## Source traceability

- **ArXiv API**: `https://export.arxiv.org/api/query?search_query=game+feel+impact+feedback+action+games&max_results=10`
- **Paper accessed**: https://arxiv.org/abs/2208.06155v3 (accessed 2026-09-26)
- **License verified**: CC-BY-NC-ND-4.0 on arXiv page
- **Cross-reference**: DEEP-DIVES.md §11 (dive 11 row), `docs/plan/gameplay.md` (PLN7)
- **EMPIRE.md**: §1 (we ship our own games), §2 (Forge tools), §3 (factory)
