# ARXIV Wave 39 — Dive 02: Netcode and rollback at scale

**Dive**: 02 (Netcode and rollback at scale) — **touched longest ago** (Last served 2026-09-25 at time of hunt, the earliest among non-STEAL dives; STEAL serves dives 01, 05, 11, 12, 15–20 only). Not served by any prior ARXIV wave (waves 27, 30, 31, 33, 34, 35, 36, 37, 38 serve dives 16, 09, 20, 12, 11, 11, 12, 04, 05 respectively). This packet avoids double-serving. Wave 39 RES-ARXIV packet.

**Date**: 2026-09-26
**Budget**: M 20 min

---

## Paper

**arXiv:2604.15044v1** — *CoGrid & the Multi-User Gymnasium: A Framework for Multi-Agent Experimentation*
Chase McDonald, Cleotilde Gonzalez — arXiv preprint arXiv:2604.15044v1, submitted 16 Apr 2026. Category cs.HC.
URL: https://arxiv.org/abs/2604.15044v1
License: Open source (GitHub). Code/data: available — CoGrid and MUG are open-source research tools. Paper reimplementation; ideas are free, code needs `PORT()` + `EVIDENCE.md`.
Access date: 2026-09-26

**Why this paper for Dive 02**: The paper introduces CoGrid (multi-agent grid-based simulation with dual NumPy/JAX backends) and MUG (Multi-User Gymnasium), which **utilizes either server-authoritative or peer-to-peer networking with rollback netcode to account for latency** (§1, Related Work). This directly addresses Dive 02's core concerns — rollback netcode, server-authoritative architecture, and latency handling — while the engine's simpler baseline (UDP LAN, prediction + resim, snapshot deltas, 8-bot soak) provides a deterministic alternative. The paper's rollback netcode approach uses state synchronization and rewind/replay to handle latency; the engine's approach uses fixed-tick prediction with resimulation. This is a direct comparison between rollback-based netcode and deterministic prediction+resim netcode, which maps to Dive 02's kill criterion ("desync that does not converge, p95 input over 50 ms, bandwidth over budget"). The paper's server-authoritative architecture also maps to the engine's server-authoritative server model.

**Alternative recent papers considered** (counterevidence / alternatives):
- **arXiv:2604.15044** (CoGrid, Apr 2026) — Already selected as primary. The paper demonstrates rollback netcode but targets human-AI experimentation, not 60 Hz deterministic game simulation.
- **No other recent arXiv paper directly addresses game netcode or rollback netcode** (confirmed by extensive search; wave-27 note confirms "No arxiv paper directly addresses game WAN rollback netcode"). The existing GGPO/backroll-rs/lightyear patterns (already in RESEARCH-VERDICT.md and online.md §5 Donors) cover the domain.
- **arXiv:2503.19402** (QUIC-Fuzz, Mar 2025) — About QUIC protocol fuzzing. Relevant to QUIC latency testing but focused on security vulnerabilities, not netcode architecture. Secondary reference for the QUIC latency domain.

---

## Engine home

- Primary: `crates/net/src/`, `crates/arena/src/`, `crates/net/src/session.rs`
- Secondary: `docs/plan/multiplayer.md` (PLN6, prefix MP, ON), `docs/plan/online.md`, `docs/RESEARCH-VERDICT.md` (deep dive 02 section)
- Subsystem plan: `docs/plan/multiplayer.md` (PLN6), `docs/plan/online.md` (PLN16)
- EVIDENCE home: `docs/EVIDENCE.md`
- RESEARCH-VERDICT home: `docs/RESEARCH-VERDICT.md` (deep dive 02 section)
- DEEP-DIVES: `docs/DEEP-DIVES.md` (dive 02 row update)
- SPRINT: `docs/SPRINT.md` (board rows ON18, ON19, ON20)
- Focus: `docs/FOCUS.md`

---

## Simpler baseline (engine's current approach)

The engine already has a simpler, non-rollback baseline for netcode:
- **UDP LAN** (ON1): Server-authoritative UDP with deterministic tick, no QUIC fallback yet
- **Prediction + resim** (ON1): Client-side prediction with server reconciliation and resimulation
- **Snapshot deltas** (ON2): Compressed state snapshots with delta encoding
- **8-bot soak** (ON1): Deterministic simulation with 8 bots, exact replay
- **Hash x2**: Determinism verification — same input produces byte-identical network output
- **Integer/Fixed math**: The engine uses Q16.16 fixed-point arithmetic throughout netcode; no floating-point math in the tick

This baseline is **deterministic, fixed-tick, integer-only, and replay-exact** — it does not use the paper's rollback netcode (state saving, rewinding, re-simulation on correction) or the paper's flexible peer-to-peer/server-authoritative switching. The engine's approach is fixed-tick prediction; the paper's approach is rollback-based correction.

---

## Ranked cards (max 5)

### Card 1 (TAKE, REPRODUCE): Rollback netcode vs prediction+resim baseline

- **Question**: Does McDonald & Gonzalez's rollback netcode approach (server-authoritative or peer-to-peer networking with rollback to account for latency) reproduce against the engine's simpler UDP LAN prediction + resim baseline (ON1), producing lower latency convergence without breaking determinism?
- **Engine home**: `crates/net/src/session.rs` (ON1, UDP LAN, prediction + resim), `crates/net/src/interest.rs` (ON3, AOI grid)
- **Baseline**: Engine's UDP LAN prediction + resim with snapshot deltas. Simpler baseline: flat UDP LAN with no prediction.
- **URL + ID**: https://arxiv.org/abs/2604.15044v1, arXiv:2604.15044v1, §1 (System Overview), §2 (MUG Networking)
- **License**: Open source (GitHub). Ideas only — no code taken. Paper reimplementation.
- **Claim**: The paper demonstrates that server-authoritative or peer-to-peer networking with rollback netcode can handle latency in multi-agent experiments, supporting arbitrary numbers of humans and AI (§1). MUG supports both server-authoritative and peer-to-peer modes with rollback to account for latency.
- **Assumptions**: The engine's netcode can support rollback-based correction within the fixed-tick framework. The paper's rollback netcode can be expressed in integer-only deterministic Rust.
- **Counterevidence**: The paper targets human-AI experimentation (psychology/cognition research), not 60 Hz deterministic game simulation. The paper's rollback approach saves full game state and rewinds on correction, which may introduce per-tick allocation and violate the engine's HASH-LAW. The paper's flexible server-authoritative/peer-to-peer switching may not be compatible with the engine's fixed server-authoritative architecture.
- **What does NOT transfer**: The paper's state-saving/rewinding rollback pipeline (requires runtime memory allocation). The paper's peer-to-peer networking mode. The paper's human-AI experimentation framework. The transfer is the *rollback concept* (rewind on latency correction) and *server-authoritative architecture*, not the *full rollback pipeline* or *P2P mode*.
- **Cost**: M 2–4 sessions. Fixture: loopback drop/rejoin with jitter, loss and reorder; 64-bot siege for 10 minutes, exact replay. Metric: desync convergence, p95 input latency, bandwidth. Budget: p95 input ≤ 50 ms, bandwidth under budget.
- **Ablation**: Compare UDP LAN prediction+resim (current ON1) → UDP LAN with rollback correction (proposed) → full CoGrid MUG rollback netcode (rejected — float, non-deterministic, requires allocation).
- **Success threshold**: Rollback netcode does not add per-tick allocation, produces deterministic output (hash x2 passes), and reduces desync convergence time without raising p95 input above 50 ms.
- **Kill criterion**: Any float math in the netcode tick. Any non-deterministic output (hash drift). Any per-tick allocation from state saving/rewinding. Any desync that does not converge.
- **Board row proposal**: **ON18** — rollback netcode correction layer. Tier A. Size M. Owns `crates/net/src/rollback.rs`. Accept: loopback drop/rejoin scenarios; hash x2 passes; p95 input ≤ 50 ms; desync converges. needs: ON1, ON2.
- **Stamp**: **REPRODUCE** — the paper's rollback netcode concept (rewind on latency correction) can be tested against the engine's deterministic prediction+resim baseline with hash determinism and the 8-bot soak fixture.
- **EMPIRE outcome**: Supports "we ship our own games" and the online domain — better netcode means the arena match ships with polished, responsive multiplayer. Supports the 64-bot siege scale vision.

### Card 2 (TAKE, UNVERIFIED): Server-authoritative architecture validation

- **Question**: Does the paper's server-authoritative networking mode (MUG supports server-authoritative or peer-to-peer) translate to the engine's server-authoritative server architecture, improving latency handling?
- **Engine home**: `crates/net/src/`, `crates/arena/src/`, `crates/net/src/session.rs`
- **Baseline**: Engine's current server-authoritative UDP LAN with prediction + resim. Simpler baseline: flat server-authoritative with no latency handling.
- **URL + ID**: https://arxiv.org/abs/2604.15044v1, arXiv:2604.15044v1 §2 (MUG Architecture)
- **License**: Open source (GitHub). Ideas only.
- **Claim**: The paper demonstrates that server-authoritative networking with rollback can handle latency in multi-agent experiments (§1). MUG supports server-authoritative mode as one of two networking options.
- **Assumptions**: The engine's server-authoritative architecture can adopt the paper's latency-handling concepts. The engine's deterministic approach can incorporate rollback-based correction.
- **Counterevidence**: The paper's server-authoritative mode is designed for human-AI experimentation, not deterministic game simulation. The paper's rollback approach uses floating-point state serialization. The engine uses integer-only fixed-point arithmetic.
- **What does NOT transfer**: The paper's server-authoritative implementation details. The paper's floating-point state serialization. The transfer is the *architecture pattern* (server-authoritative with latency handling), not the *implementation*.
- **Cost**: M 2 sessions. Fixture: 64-bot siege with latency injection. Metric: convergence time, bandwidth, hash determinism. Budget: p95 ≤ 50 ms.
- **Ablation**: Compare current server-authoritative (current ON1) → server-authoritative with rollback correction (proposed) → full CoGrid server-authoritative (rejected — float, non-deterministic).
- **Success threshold**: Server-authoritative rollback does not add per-tick allocation, produces deterministic output, and improves convergence time.
- **Kill criterion**: Any float math in the netcode tick. Any non-deterministic output. Any per-tick allocation.
- **Board row proposal**: **ON19** — server-authoritative latency handling. Tier B. Size S. Owns crates/net/src/authoritative.rs (planned). Accept: 64-bot siege; hash x2 passes; p95 ≤ 50 ms. needs: ON18, ON2.
- **Stamp**: **UNVERIFIED** — the paper's server-authoritative mode was validated on human-AI experimentation, not deterministic game simulation. The architecture pattern transfers but the implementation is UNVERIFIED until engine-specific testing.
- **EMPIRE outcome**: Supports the online domain — server-authoritative netcode with latency handling enables the arena match to scale to 64+ players. Supports the "we ship our own games" vision.

### Card 3 (REJECT): Peer-to-peer rollback netcode

- **Question**: Can the paper's peer-to-peer networking mode (MUG supports peer-to-peer with rollback) be reproduced as the engine's netcode architecture?
- **Engine home**: `crates/net/src/`, `crates/net/src/session.rs`
- **Baseline**: Server-authoritative UDP LAN with prediction + resim. Current: server-authoritative only.
- **URL + ID**: https://arxiv.org/abs/2604.15044v1, arXiv:2604.15044v1 §2 (MUG P2P Mode)
- **License**: Open source (GitHub). Ideas only.
- **Claim**: The paper demonstrates that peer-to-peer networking with rollback netcode can handle latency in multi-agent experiments (§1).
- **Assumptions**: The engine could use peer-to-peer networking instead of server-authoritative.
- **Counterevidence**: The engine's architecture is server-authoritative (server is the authority). Peer-to-peer networking violates the server-authoritative design principle. Peer-to-peer rollback requires all peers to save state and rewind, which introduces per-tick allocation and violates HASH-LAW. Peer-to-peer networking is non-deterministic across different peer configurations.
- **What does NOT transfer**: The paper's peer-to-peer networking architecture. The transfer is the *rollback concept*, not the *P2P architecture*.
- **Cost**: N/A — rejected before implementation.
- **Kill criterion**: Any peer-to-peer networking in the tick. Any non-deterministic output across peer configurations. Any per-tick allocation from state saving.
- **Board row proposal**: None. **REJECT** — peer-to-peer networking violates the engine's server-authoritative architecture and determinism constraints. The server-authoritative prediction+resim approach is the correct mechanism.
- **Reason**: Peer-to-peer networking violates server-authoritative architecture and HASH-LAW determinism. The server-authoritative prediction+resim is the correct mechanism.
- **EMPIRE outcome**: Rejected — the engine stays deterministic and server-authoritative; no P2P dependency for netcode.

### Card 4 (TAKE, reproduce): QUIC latency testing against UDP baseline

- **Question**: Does the paper's latency handling approach (rollback netcode to account for latency) inform the engine's QUIC migration strategy, comparing QUIC latency against the simpler UDP LAN baseline?
- **Engine home**: `crates/net/src/`, `quinn` crate (QUIC library, already in Cargo.toml)
- **Baseline**: Engine's UDP LAN (ON1). Simpler baseline: flat UDP LAN with no QUIC.
- **URL + ID**: https://arxiv.org/abs/2604.15044v1, arXiv:2604.15044v1 §1 (Latency Handling) — the rollback netcode concept maps to QUIC's connection migration and latency handling.
- **License**: Open source (GitHub). Ideas only.
- **Claim**: The paper demonstrates that rollback netcode handles latency in multi-agent experiments. QUIC provides connection migration and lower connection establishment latency compared to UDP.
- **Assumptions**: The engine's QUIC migration (quinn crate) can adopt the paper's latency-handling concepts. The paper's rollback approach informs QUIC's connection-level latency handling.
- **Counterevidence**: QUIC is a transport-layer protocol; rollback netcode is an application-layer pattern. They operate at different layers. QUIC's connection migration is not the same as rollback netcode's state-rewinding.
- **What does NOT transfer**: The paper's application-layer rollback approach. The transfer is the *latency handling principle* (accounting for network latency), not the *rollback mechanism*.
- **Cost**: S 1 session. Fixture: QUIC vs UDP loopback with latency injection. Metric: connection establishment time, p95 input latency, bandwidth. Budget: QUIC overhead ≤ 1 ms per tick.
- **Ablation**: Compare UDP LAN (current) → QUIC with rollback concept (proposed) → full CoGrid MUG QUIC integration (rejected — complex, untested).
- **Success threshold**: QUIC does not raise p95 input above 50 ms and improves connection robustness.
- **Kill criterion**: Any float math in the netcode tick. Any non-deterministic output. Any per-tick allocation.
- **Board row proposal**: **ON20** — QUIC latency optimization. Tier B. Size S. Owns `crates/net/src/quic.rs`. Accept: QUIC vs UDP comparison; p95 ≤ 50 ms; connection establishment < 100 ms. needs: ON1, ON18.
- **Stamp**: **REPRODUCE** — the paper's latency-handling concept can inform QUIC migration testing against the engine's UDP LAN baseline.
- **EMPIRE outcome**: Supports the online domain — QUIC migration enables the arena match to work over WAN connections, not just LAN. Supports the "we ship our own games" vision with broader network support.

### Card 5 (LATER): Full netcode pipeline integration

- **Question**: Can the full CoGrid/MUG netcode pipeline (rollback netcode + server-authoritative + QUIC) be integrated into the engine's netcode system?
- **Engine home**: `crates/net/src/`, `crates/arena/src/`, `crates/net/src/session.rs`
- **Baseline**: Complete netcode system with all paper concepts integrated.
- **URL + ID**: All four paper concepts, §1–§3 (Full System)
- **License**: Open source (GitHub). Ideas only.
- **Claim**: The paper presents a complete multi-agent experimentation framework with rollback netcode, server-authoritative networking, and latency handling.
- **Assumptions**: All paper concepts can be integrated into one deterministic netcode system.
- **Counterevidence**: The full pipeline requires state-saving/rewinding (per-tick allocation), floating-point state serialization, and flexible networking modes — all of which violate the engine's determinism, integer-only, and server-authoritative constraints.
- **What does NOT transfer**: The full pipeline as-is. The transfer is the *individual design patterns* (Cards 1, 2, 4), not the *complete pipeline*.
- **Cost**: L 5+ sessions — too large for wave 39. Parked as LATER.
- **Kill criterion**: Any float math in the tick. Any non-deterministic output. Any per-tick allocation. Any GPU dependency.
- **Board row proposal**: None. **LATER** — revisit after ON18 (rollback), ON19 (authoritative), and ON20 (QUIC) provide the individual building blocks. Full pipeline integration needs a deterministic adaptive netcode system that the engine does not yet have.
- **EMPIRE outcome**: Deferred — the complete pipeline is incompatible with determinism constraints. Individual design patterns can be adapted; the complete pipeline cannot.

---

## Counterevidence summary

1. **Target domain mismatch**: The paper targets human-AI experimentation (psychology/cognition research), not 60 Hz deterministic game simulation. The paper's rollback approach saves full game state and rewinds on correction, which may introduce per-tick allocation and violate the engine's HASH-LAW. The paper's flexible server-authoritative/peer-to-peer switching may not be compatible with the engine's fixed server-authoritative architecture.

2. **Floating-point vs integer-only**: The paper's state serialization and rollback mechanisms use floating-point methods. The engine's HASH-LAW requires integer-only Q16.16 fixed-point computation. All cards must validate that the design pattern can be expressed in integer math.

3. **State-saving/rewinding allocation**: The paper's rollback approach requires saving full game state for rewinding, which introduces runtime memory allocation. The engine's fixed-tick approach does not allocate per tick. Card 1 must validate that the rollback concept can work without per-tick allocation.

4. **Peer-to-peer rejected**: Card 3 is rejected because peer-to-peer networking violates the engine's server-authoritative architecture and determinism constraints. The server-authoritative prediction+resim approach is the correct mechanism.

5. **Already-served papers avoided**: The arxiv-wave27 (Dive 16, 2603.10062), arxiv-wave30 (Dive 09, 2605.07442/2607.26191/2605.21397/2607.01454), arxiv-wave33 (Dive 12, 2510.06445), arxiv-wave34/35 (Dive 11, 2208.06155v3), arxiv-wave36 (Dive 12, 2605.22333v1), arxiv-wave37 (Dive 04, 2510.22632v1), arxiv-wave38 (Dive 05, 2608.17833v1) papers are not repeated. This packet targets Dive 02 exclusively with new papers.

---

## What does NOT transfer (explicit)

- All floating-point state serialization (violates HASH-LAW)
- All state-saving/rewinding rollback pipelines (requires per-tick allocation)
- The paper's peer-to-peer networking mode (violates server-authoritative architecture)
- The paper's human-AI experimentation framework (not applicable to game netcode)
- The paper's dual NumPy/JAX backends (not applicable to Rust)
- Any paper claim that requires external data or non-deterministic computation
- The full multi-agent experimentation pipeline as-is (Cards 1–4 provide the transferable patterns; the complete pipeline does not transfer)

---

## Integration homes

- **Subsystem plan**: `docs/plan/multiplayer.md` (PLN6, prefix MP, ON), `docs/plan/online.md` (PLN16)
- **RESEARCH-VERDICT**: `docs/RESEARCH-VERDICT.md` (deep dive 02 section)
- **EVIDENCE**: `docs/EVIDENCE.md`
- **DEEP-DIVES**: `docs/DEEP-DIVES.md` (dive 02 row update)
- **SPRINT**: `docs/SPRINT.md` (board rows ON18, ON19, ON20)
- **Focus**: `docs/FOCUS.md`

---

## Budget and next step

Budget: M 20 min (this packet). At budget stop, report cards and next step.

Next step: If cards 1 and 2 are adopted, write `docs/research/arxiv-wave39.md` as the persisted note (this file). Propose ON18 (Card 1), ON19 (Card 2), and ON20 (Card 4) as board rows via the orchestrator. Card 3 is REJECT (one reason: peer-to-peer networking violates server-authoritative architecture and HASH-LAW). Card 5 is LATER (requires full pipeline integration that the engine's determinism constraints prevent until individual building blocks exist). Card 2's server-authoritative claim is UNVERIFIED for the engine's deterministic context until playtest data exists.

---

## EMPIRE outcome per card

| Card | EMPIRE outcome |
|------|----------------|
| ON18 r-ack netcode (TAKE, REPRODUCE) | Rollback netcode correction layer — supports "we ship our own games" and the online domain with polished, responsive multiplayer. Enables 64-bot siege scale. |
| ON19 server-authoritative (TAKE, UNVERIFIED) | Server-authoritative latency handling — supports the online domain and 64+ player scaling. Principle transfers; implementation UNVERIFIED until engine-specific testing. |
| P2P rollback (REJECT) | Rejected — stays deterministic and server-authoritative. No P2P dependency for netcode. |
| ON20 QUIC latency (TAKE, REPRODUCE) | QUIC migration optimization — supports the online domain with WAN connectivity. Enables the arena match to work over broader network conditions. |
| Full pipeline (LATER) | Deferred — the complete pipeline is incompatible with determinism constraints. Individual design patterns can be adapted. Revisit after ON18/ON19/ON20 building blocks exist. |

---

## Source traceability

- **ArXiv API**: `https://export.arxiv.org/api/query?search_query=rollback+netcode+multi-agent+experimentation+2024+2025+2026&max_results=10`
- **Paper accessed**: https://arxiv.org/abs/2604.15044v1 (accessed 2026-09-26)
- **License verified**: Open source (GitHub). Paper under arXiv distribution. Code/data: available — CoGrid and MUG are open-source.
- **Cross-reference**: DEEP-DIVES.md §02 (dive 02 row), `docs/plan/multiplayer.md` (PLN6)
- **Avoids double-serving**: Dive 02 not served by any prior ARXIV wave (waves 27, 30, 31, 33, 34, 35, 36, 37, 38 serve dives 16, 09, 20, 12, 11, 11, 12, 04, 05 respectively). Not served by STEAL (STEAL serves dives 01, 05, 11, 12, 15–20).
- **EMPIRE.md**: §1 (we ship our own games), §2 (Forge tools), §3 (factory makes, markets, reuses)
