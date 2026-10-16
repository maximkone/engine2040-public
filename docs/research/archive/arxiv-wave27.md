# ARXIV27 — Wave-27 Accepted-Paper Verdict

**Task**: Accepted-paper verdict serving wave-27: fog/contact shadows for RL7, WAN rollback for ON4, NPC memory consistency for LW3.
**Date**: 2026-09-25
**Board reference**: `docs/SPRINT.md` — RL7, ON4, LW3 rows
**Method**: arxiv API search (export.arxiv.org) per domain; papers reimplemented, never copied; ideas are free, code needs `PORT()` + `EVIDENCE.md`.

---

## Summary Verdicts

| # | Board Row | Topic | Arxiv Hit | Verdict | Why |
|---|---|---|---|---|---|
| 1 | **RL7** | Height fog + contact shadows | **None** (no arxiv paper on contact shadows for real-time rendering) | **DO-NOT-TAKE** | The technique lives in industry Frostbite/Decima course material, not arxiv; `fog.rs` already implements the shape |
| 2 | **ON4** | WAN fault soak + rollback | **2604.15044** (CoGrid: rollback netcode in multi-agent frameworks) — adjacent, not on-point | **DO-NOT-TAKE** (bounded) | No arxiv paper on game WAN rollback netcode; existing GGPO/backroll-rs/lightyear patterns already cover the domain |
| 3 | **LW3** | NPC memory consistency | **2603.10062** (Multi-Agent Memory from a Computer Architecture Perspective) — explicitly addresses memory consistency | **TAKE** (bounded, ideas-only) | The memory hierarchy + consistency protocol framing directly informs LW3's bounded NPC memory design |

---

## 1. RL7 — Height Fog + Contact Shadows (render-look)

### Board row
`[ ] RL7 render: height fog + contact shadows for small details. Tier A. Size M. Owns crates/render/src/fog.rs.` Needs: GX7, GX1, RL1, RL4.

### Search results
- `all:"contact shadow"`: **0 results** in cs.GR (and 0 in all fields combined).
- `cat:cs.GR + all:fog + all:shadow`: **0 results**.
- `all:"height fog" + all:"real-time"`: **0 results**.
- The only cs.GR papers mentioning "fog" are about volumetric point clouds (2106.15539, unrelated) and atmospheric scattering — none about real-time height fog + contact shadows for games.

### Why no arxiv paper exists
Contact shadows and height fog for real-time game rendering are **industry trade-secret / SIGGRAPH course material** (Frostbite, Decima). The technique is documented in the engine's existing plan as "Frostbite/Decima height fog + contact shadows (fog school)" under `render-look.md` §5 Donors — explicitly as "papers and talks (reimplemented ideas, never copied code, no license needed)". These are not arxiv papers because they are production renderer know-how, not academic research.

### Current engine state
`fog.rs` (2438 lines) already has `FOG_PASS`, `CONTACT_PASS`, `from_sky_capture` ramp, `compile_fog_for_forward`, `submit_gpu_fog` — unit-tested, landed-but-OPEN (production depth-feed seam open via RL9). The rendering math is implemented; what's missing is the production wiring (RL9) and device evidence (GX7/GX1/RL1/RL4 need to land first).

### Falsifiable experiment
1. **Fog-ramp match**: bake the RL4 sky capture once (worker thread), read it three ways (RL8 skybox, RL2 IBL ambient, RL7 fog ramp). Verify fog color matches sky ramp within `FOG_SKY_RAMP_TOL` on llvmpipe. **Falsifiable**: if fog color diverges from sky ramp beyond tolerance, the single-bake-three-reader invariant is broken.
2. **Contact pass budget**: measure the contact shadow pass on the reference GPU (RTX 3050). **Falsifiable**: if contact pass > 0.3 ms at 1080p, the row fails its budget bar.

### Real consumer
`crates/render/src/fog.rs` (consumer: `game` via WIN1 arena match, `editor` via ED1 viewport). RL9 (`crates/render/src/present.rs` depth-readback seam) is the immediate dependency.

### Kill criteria
- Fog color diverges from RL4 sky bake beyond `FOG_SKY_RAMP_TOL` → kill RL7/RL9 fog path, revert to sky-color-only fog.
- Contact pass > 0.3 ms on reference GPU → kill contact shadows, keep height fog only.
- Any wgpu validation error in the fog or contact pass → row cannot merge until fixed.

### Verdict: **DO-NOT-TAKE**
No arxiv paper exists for this rendering technique. The knowledge is industry course material, not arxiv. The engine already implements the shape (`fog.rs`). Further arxiv hunting is wasted effort; the open work is engineering (RL9 depth-feed wiring), not research.

---

## 2. ON4 — WAN Fault Soak + Rollback (online)

### Board row
`[ ] ON4 WAN fault soak 150ms/30ms/2% at 64 (missing)`. No `wan_soak.rs`. The plan references "Rollback under 150 ms RTT → ≤8-tick resim then AH4 resync, checkpoint hash per snapshot" (online.md §4, problem 4).

### Search results
- `all:"rollback" + all:"netcode"`: **1 result** — 2604.15044 (CoGrid & Multi-User Gymnasium), which mentions "rollback netcode to account for latency" in a multi-agent experimentation framework. **Not about game networking directly.**
- `all:"rollback" + all:"WAN" + cat:cs.NI`: **0 results**.
- `cat:cs.NI + all:rollback + all:latency`: **1 result** — 2609.14517 (6G AI lifecycle rollback), about telecom, not game networking.
- `all:"rollback" + all:"networking"`: **2 results** — 2510.07806 (Ancora: web intrusion recovery rollback) and 2609.02514 (AceSpec: edge-cloud LLM inference with WAN rollback). Both are about web security and LLM inference, not game rollback netcode.

### Why no direct arxiv paper exists
Game rollback netcode over WAN is an **engineering technique** documented in the GGPO paper (GDC talk), backroll-rs source, and lightyear design. It is not a traditional arxiv research topic — it's a domain-specific implementation pattern. The existing RESEARCH-VERDICT.md already records: "WAN rollback, interest and priority, relay and cheat checks (quinn, renet, lightyear verified)" in the Deep dives 01-12 section.

### Closest arxiv paper: 2604.15044 (CoGrid)
- **Title**: CoGrid & the Multi-User Gymnasium: A Framework for Multi-Agent Experimentation
- **What it takes**: Demonstrates rollback netcode in a research framework for human-AI multi-agent experiments. Uses server-authoritative or peer-to-peer networking with rollback netcode to account for latency.
- **What does NOT transfer**: The paper is about experimental tooling for psychology/cognition research, not about game-state rollback at 60 Hz with 64 fighters + 500 AI. The netcode shape is similar but the scale, determinism requirements, and tick budget are completely different.

### Current engine state
- `crates/net/src/session.rs` has `MAX_ROLLBACK=8`, NET1 UDP + prediction + resim working on LAN.
- `interest.rs` (ON3) handles AOI grid + priority with 64 connections.
- `wan_soak.rs` does NOT exist — ON4 is missing entirely.
- The existing deep dive already verified quinn, renet, lightyear for WAN patterns.

### Falsifiable experiment
1. **Fault injection convergence**: Run a 64-player match through the ON4 fault injector at 150 ms RTT / 30 ms jitter / 2% loss + reorder + drop/rejoin. Verify all authoritative checkpoints converge byte-identical. **Falsifiable**: if any checkpoint diverges, the WAN fault model breaks determinism.
2. **Rollback resimulation bound**: Under 150 ms RTT, verify ≤8-tick resim converges before AH4 resync kicks in. **Falsifiable**: if resim exceeds 8 ticks, the WAN latency exceeds the rollback window and AH4 resync is required — measure the frequency.
3. **Per-fault report**: Each fault type (loss, reorder, drop, reconnect) produces a report naming the first divergence tick. **Falsifiable**: if a divergence is not attributed to a specific fault, the report is incomplete.

### Real consumer
`crates/net/src/` — the `wan_soak.rs` fault injector (new file), `session.rs` (rollback + resim), `interest.rs` (ON3 priority filtering). Consumer: `game` via WIN1 arena match over QUIC, `proof` via ON11 online soak.

### Kill criteria
- Any checkpoint divergence under the ON4 fault model → kill ON4 until the divergence is attributed and fixed.
- Resim > 8 ticks under 150 ms RTT → ON4 fails the rollback bound; the row needs AH4 resync integration proof.
- Per-fault report missing a divergence attribution → row cannot merge.
- Server tick p99 > 4 ms at 64 + 500 AI under the ON4 fault model → kill ON4 scale.

### Verdict: **DO-NOT-TAKE** (bounded: ideas-only from 2604.15044)
No arxiv paper directly addresses game WAN rollback netcode. The CoGrid paper (2604.15044) demonstrates rollback netcode in a research framework but does not transfer to the 60 Hz, 64-fighter, deterministic-tick domain. The existing GGPO/backroll-rs/lightyear patterns (already in RESEARCH-VERDICT.md and online.md §5 Donors) cover the domain. 2604.15044 is kept as a **research reference only** — the fault-injection harness shape may inform the ON4 test architecture, but no code is taken.

---

## 3. LW3 — NPC Memory Consistency (living-world)

### Board row
`[ ] LW3 ludus comes alive (trainer, blacksmith, crowd voices with memory + goals)`. Needs LW2, WD2. The plan defines bounded memory: `Mind` owns one NPC's stream (512 events, 16 goals, 64 relations, 280-char texts, integer-only recall `salience * 1024 / (1 + age)` with earlier-wins ties), `MAX_RECALL = 8`, q8 salience. LW1/LW2 contract holds; LW3 adds voices that drain through LW8 apply.

### Search results
Multiple arxiv papers address agent memory consistency. The best candidates:

**2603.10062** — "Multi-Agent Memory from a Computer Architecture Perspective: Visions and Challenges Ahead" (cs.AR, 2026-03-09)
- **What it claims**: Frames multi-agent memory as a computer architecture problem. Proposes a three-layer memory hierarchy (I/O, cache, memory). Identifies two critical protocol gaps: cache sharing across agents and structured memory access control. **Explicitly states: "the most pressing open challenge is multi-agent memory consistency."**
- **What transfers**: The memory hierarchy framing (short-term/cache/long-term) maps directly to LW1/LW2's bounded stream + MemoryStore. The "structured memory access control" protocol gap maps to LW1's `ActionLog::drain_due` as the only door between provider text and the tick. The consistency challenge maps to LW3's requirement that NPC memories remain consistent across mock/local/remote providers with providers off.
- **What does NOT transfer**: The paper is a position paper about multi-agent LLM systems, not about deterministic game NPCs. The paper proposes protocol solutions for shared/distributed memory in LLM agents; LW3's solution is integer-only, bounded, recorded, and replay-exact (no LLM in the tick).

**2605.06716** — "From Storage to Experience: A Survey on the Evolution of LLM Agent Memory Mechanisms" (cs.AI, ACL 2026 Findings)
- **What it claims**: Formalizes agent memory evolution into three stages: Storage (trajectory preservation), Reflection (trajectory refinement), Experience (trajectory abstraction). Explores "long-range consistency" as a core driver.
- **What transfers**: The Storage→Reflection→Experience pipeline maps to LW1/LW2 (storage) → LW5 (reflection) → LW3+ (experience). The "long-range consistency" challenge maps to LW3's requirement that 20 scripted visits give consistent memories.
- **What does NOT transfer**: Survey-level, no specific algorithm. The paper's frameworks are for LLM agents; LW3's memory is integer-salience-based and replay-exact.

**2604.07798** — "Lightweight LLM Agent Memory with Small Language Models" (LightMem, ACL 2026)
- **What it claims**: Modularizes memory retrieval/writing/consolidation. Uses semantic consistency re-ranking. Short-term/mid-term/long-term memory separation.
- **What transfers**: The STM/MTM/LTM separation maps to LW1 stream + LW2 MemoryStore + LW5 reflection. The "semantic consistency re-ranking" concept maps to LW2's q8 salience ranking with earlier-wins ties.
- **What does NOT transfer**: The paper uses vector similarity for retrieval; LW2 uses integer q8 salience. The paper's LLM-based memory operations are replaced by LW1's deterministic `drain_due` contract.

### Current engine state
- LW1/LW2 contract holds: `Mind` (512/16/64/280/4 caps), `MemoryStore` (128 NPCs × 128 records, MAX_RECALL=8).
- `ActionLog::drain_due` is the only door between provider text and the tick.
- LW3 adds ludus voices (trainer, blacksmith, crowd) that drain through LW8 apply.
- The existing RESEARCH-VERDICT.md already records "Generative-agent memory streams" (Park et al. 2304.03442) as IN for LW1–LW4.

### Falsifiable experiment
1. **Memory consistency across providers**: Run 20 scripted ludus visits through mock and local providers (providers off). Verify memories are consistent (same recall set, same salience ranking) across all runs. **Falsifiable**: if any visit produces a different memory state, the consistency invariant is broken.
2. **Bounded memory hold**: Run the 10k-event soak with 128 NPCs × 128 records. Verify no NPC exceeds 128 stored records and no NPC exceeds 512 stream events. **Falsifiable**: if any NPC exceeds its cap, the boundedness invariant is broken.
3. **Replay exactness**: Record a ludus visit with a provider, replay offline with providers off. Verify the checkpoint reproduces byte-identical. **Falsifiable**: if the replay diverges, the `replay_offline` contract is broken.
4. **Contradiction rate**: Score 20 conversation+goal fixtures for contradiction rate (every said line cites its recall set; negations fall back). **Falsifiable**: if contradiction rate exceeds a named threshold, the reflection/consistency design needs revision.

### Real consumer
`crates/ai/src/mind.rs` and `crates/ai/src/memory.rs` (consumer: `crates/game/src/ludus_npcs.rs` for LW3, `crates/playtest/src/living.rs` for LW4 eval). The `ActionLog::drain_due` contract is the consistency door.

### Kill criteria
- Any ludus visit produces inconsistent memory state across mock/local/remote providers → kill LW3 until the determinism contract is fixed.
- Any NPC exceeds 128 stored records or 512 stream events in the 10k soak → kill LW3 memory cap design.
- Replay offline produces a different checkpoint than the live run → kill LW3/LW8 apply contract.
- Contradiction rate exceeds the named threshold (to be set by LW4 eval) → kill LW5 reflection design.
- Any provider text enters the tick (violating the `drain_due` door) → kill LW3 immediately.

### Verdict: **TAKE** (bounded, ideas-only)
**2603.10062** is the best match — it explicitly frames "multi-agent memory consistency" as the most pressing open challenge and proposes a three-layer memory hierarchy with access-control protocols. The memory hierarchy (I/O/cache/memory) maps to LW1/LW2/LW5. The "structured memory access control" protocol maps to `ActionLog::drain_due`. **The paper is ideas-only**: no code is taken, the engine's deterministic integer-only approach supersedes the paper's LLM-based proposals, and the `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported. The other two papers (2605.06716, 2604.07798) are supporting references that validate the Storage→Reflection→Experience pipeline and the STM/MTM/LTM separation already in the plan.

---

## Wave-27 Research Card Summary

| Field | RL7 | ON4 | LW3 |
|---|---|---|---|
| **Paper** | None exists | 2604.15044 (adjacent) | 2603.10062 (on-point) |
| **Verdict** | DO-NOT-TAKE | DO-NOT-TAKE (bounded) | TAKE (bounded, ideas-only) |
| **Experiment** | Fog-ramp match + contact pass budget | 64-player fault injection convergence | 20-visit memory consistency replay |
| **Consumer** | `crates/render/src/fog.rs` | `crates/net/src/wan_soak.rs` (new) | `crates/ai/src/mind.rs` + `memory.rs` |
| **Kill criterion** | Fog diverges > tol or contact > 0.3 ms | Any checkpoint divergence under fault model | Inconsistent memory state across providers |
| **What stays true** | `fog.rs` math stands; open work is RL9 engineering | GGPO/backroll-rs/lightyear patterns already cover the domain | LW1/LW2 contract holds; paper informs the hierarchy framing |
| **What does NOT transfer** | Industry Frostbite/Decima know-how is not arxiv | CoGrid is experimental tooling, not game networking | Paper is LLM-based; engine is integer-only and replay-exact |

---

## Evidence rows (proposed for RESEARCH-VERDICT.md)

The following rows amend `docs/RESEARCH-VERDICT.md` in the "2026 additions" section:

| Idea | Source | Verdict |
|---|---|---|
| Contact shadows for real-time rendering | Frostbite/Decima course material (industry, not arxiv) | DO-NOT-TAKE; `fog.rs` implements the shape; no arxiv paper exists |
| WAN rollback netcode for games | CoGrid 2604.15044 (research framework only) | DO-NOT-TAKE; GGPO/backroll-rs/lightyear patterns already cover the domain |
| Multi-agent memory consistency hierarchy | 2603.10062 (ideas only) | TAKE (bounded); maps to LW1/LW2/LW5 memory hierarchy; engine uses integer-only replay-exact approach |

---

## Unverified

- **RL7 fog-ramp tolerance value** (`FOG_SKY_RAMP_TOL`): the plan mentions it but the exact numeric value needs to be read from `fog.rs` source. Not code-edited; just noted as needing verification.
- **ON4 fault model exact parameters**: the plan specifies 150 ms RTT / 30 ms jitter / 2% loss + reorder + drop/rejoin, but the `wan_soak.rs` implementation does not exist yet. The experiment design assumes these parameters.
- **LW3 contradiction rate threshold**: the plan says the threshold will be set by LW4 eval but does not name a specific number. The kill criterion needs this number from the LW4 row.
- **2603.10062 paper details**: only the abstract was read via arxiv API. The full paper's memory hierarchy details and protocol specifications were not read. The "ideas-only" take is bounded to the abstract-level framing.
- **No arxiv paper was found for contact shadows in real-time rendering**: this is confirmed by multiple arxiv API queries returning 0 results. The technique is confirmed to be industry course material per the existing plan's donor table.
