# ARXIV31 — Wave-31 Accepted-Paper Verdict

**Task**: Accepted-paper verdict serving wave-31: legion AI / flocking (GA2), crash-safe journals (ED8), versioned paths (NV7).
**Date**: 2026-09-26
**Board reference**: `docs/SPRINT.md` — GA2, ED8, NV7 rows
**Method**: arxiv API search (export.arxiv.org) per domain; papers reimplemented, never copied; ideas are free, code needs `PORT()` + `EVIDENCE.md`.

---

## Summary Verdicts

| # | Board Row | Topic | Arxiv Hit | Verdict | Why |
|---|---|---|---|---|---|
| 1 | **GA2** | Legion AI / flocking formations | **cond-mat/9804180v1** (Toner & Tu, "Flocks, herds, and schools") + **2312.06383v1** (flocking/swarming multi-agent) | **TAKE** (bounded, ideas-only) | Foundational continuum theory of coherent collective motion maps to legion formations; the engine's integer-only deterministic approach supersedes float-based physics |
| 2 | **ED8** | Crash-safe journals (WAL) | **2406.09649v1** (SquirrelFS) + **2507.13062v1** (WAL in Rust) | **TAKE** (bounded, ideas-only) | SquirrelFS proves Rust typestate can enforce crash-consistency at compile time; the WAL-in-Rust paper confirms minimal overhead for the editor journal |
| 3 | **NV7** | Versioned paths over dynamic navmesh | **2602.04130v1** (Recast-based A* in dynamic game environments) | **TAKE** (bounded, reimplementation) | Directly addresses navigation in dynamic game environments with incremental phases; maps to NV7's tile-version-vector stamped paths |

---

## 1. GA2 — Legion AI / Flocking Formations (game AI)

### Board row
`[ ] GA2 legion AI: formations, shield wall, flanking` (missing; no `crates/ai/src/legion.rs`). From `docs/plan/game-ai.md`: "Orkin FEAR lineage (squad role planning for GA2)" and "Treuille et al. continuum crowds (SIGGRAPH 2006)" are named as paper/talk references. GA3 steering semantics port under ARC4, not a second brain. Tier B. Size L. Owns `crates/ai/src/legion.rs`. Accept: a shield wall holds against a charge in fixtures; thinking LOD rides with GA5. needs: GA1, NV3.

### Search results
Three arxiv hits are relevant to legion/flocking behavior in game-agent contexts:

**cond-mat/9804180v1** — "Flocks, herds, and schools: A quantitative theory of flocking" (Toner & Tu, 1998, Phys Rev E 58:4828).
- **What it claims**: A continuum theory of flocking — the collective coherent motion of large numbers of self-propelled organisms. Predicts an "ordered phase" where all members move with the same mean velocity. Spontaneously broken rotational symmetry produces Goldstone modes. The model resembles Navier-Stokes for a compressible fluid and a Ginzburg-Landau theory for an n-component ferromagnet. In d=2, flocks exhibit true long-range ordered spontaneous symmetry breaking (contrary to the Mermin-Wagner theorem for equilibrium systems).
- **What transfers**: The mathematical foundation for how large groups achieve coherent motion — directly maps to GA2's "formations, shield wall, flanking". The ordered-phase concept (all legion members moving together) is exactly what a shield wall requires. The Goldstone-mode fluctuations describe how the formation responds to perturbations (a charge breaking the line). The continuum-field approach maps to how the engine could compute legion-level steering as a field rather than per-agent queries.
- **What does NOT transfer**: The paper is a physics model using continuous fields and float mathematics; engine2040's nav and AI are integer-only with deterministic replay. The paper's simulation approach cannot be used inside the tick. The paper is a theoretical framework, not an algorithm.

**2312.06383v1** — "Flocking and swarming in a multi-agent dynamical system" (Sar & Ghosh, 2023, Chaos).
- **What it claims**: A minimal mathematical model of locally interacting multi-agent systems where agents simultaneously swarm in space and exhibit flocking behavior. Various cluster structures emerge depending on interaction range. When coupling strength exceeds a threshold, flocking behavior is observed.
- **What transfers**: Validates the flocking phase transition in multi-agent systems with configurable interaction ranges. Maps to GA2's need to tune legion cohesion parameters (how tightly the formation holds). The cluster-structure analysis maps to how different legion shapes (shield wall, flanking wedge) emerge from local interaction rules.
- **What does NOT transfer**: Uses float-based simulation. The coupling-strength threshold is continuous, not integer quantized. Must be adapted to the engine's fixed-tick integer framework.

**2105.09764v1** — "Human-agent coordination in a group formation game" (Takko et al., 2021, Scientific Reports).
- **What it claims**: Studies coordination between humans and autonomous agents in a group formation game on a small-world network. Agents swap places to form connected clusters of the same color. Shows that providing more control to humans maximizes performance in cooperative settings.
- **What transfers**: The group-formation game framing maps to GA2's formation mechanics — agents must arrange into specific shapes (shield wall, flanking). The human-agent coordination model maps to how the AI leader directs followers while the player controls the hero. The network topology approach maps to how legion members communicate within formation constraints.
- **What does NOT transfer**: The paper is about human-agent interaction experiments, not about autonomous game AI. The small-world network model doesn't map to the navmesh-based agent graph.

### Current engine state
- `crates/ai/src/` owns gladiator brains. GA1 (depth: phases, feints, stamina, squads) is unit-tested and integrated.
- GA2 is file-absent by glob (`no crates/ai/src/legion.rs`). The plan says: "GA3 steering semantics port under ARC4, not a second brain."
- `crates/nav/src/` owns the nav/crowd/steering layer. NV2 crowd steering + NV3 tiles + NV6 funnel are the movement primitives GA2 rides on.
- `DepthBrain` (GA1) owns the arena policy; legion logic must compose it, not replace it.
- The plan's donor table names: Fyrox (MIT, AI state-machine organization ideas for GA2 roles) and papers/talks (Treuille continuum crowds, Orkin FEAR lineage, R*-tree papers).

### Falsifiable experiment
1. **Formation coherence test**: Implement a legion of 50 agents using the Toner & Tu field-based steering model (adapted to integer math). Verify the legion achieves coherent motion (all agents within 5° of mean heading) within 100 ticks. **Falsifiable**: if the legion cannot achieve < 5° heading variance within 100 ticks, the field model doesn't transfer to integer math.
2. **Shield wall hold**: Run a charge scenario (10 attackers vs 50 shield-wall defenders). Verify the wall holds for ≥ 500 ticks without breaking. **Falsifiable**: if the wall breaks before 500 ticks, the formation cohesion parameters need retuning.
3. **Flanking maneuver**: Command the legion to execute a flanking maneuver around a barrier. Verify all agents reach their flanking position within 200 ticks and the formation re-coheres. **Falsifiable**: if agents diverge or the formation dissolves during the maneuver, the flanking logic is broken.
4. **Determinism**: Run the same formation scenario twice with the same seed. Verify byte-identical agent positions at every tick. **Falsifiable**: if positions diverge, the integer-only adaptation introduces nondeterminism.
5. **Budget**: Measure per-tick AI cost for a 500-agent legion. **Falsifiable**: if cost exceeds the named budget (per-agent steering ≤ 0.1 ms flat from 50 to 500 agents, per game-ai.md §3 budgets), the field model is too expensive.

### Real consumer
`crates/ai/src/legion.rs` (new file, consumer: `crates/ai/src/depth.rs` `DepthBrain` via `ArenaPolicy`, `crates/arena` for the match). The legion brain feeds intent through the same `FighterInput` pipeline as GA1/GA3. `crates/nav/src/crowd.rs` and `funnel.rs` provide the movement primitives.

### Kill criteria
- Formation cannot achieve < 5° heading variance within 100 ticks → kill GA2 field model, fall back to per-agent steering only.
- Shield wall breaks before 500 ticks in the charge fixture → kill GA2 cohesion parameters, redesign.
- Flanking maneuver fails to re-cohere → kill GA2 flanking logic.
- Any determinism divergence (positions differ between runs with same seed) → kill GA2 immediately; revert to GA1-style utility approaches.
- Per-tick AI cost for 500 agents exceeds the budget → kill GA2 field model, simplify.
- Any float math enters the tick → kill GA2; port to Fixed math.

### Verdict: **TAKE** (bounded, ideas-only)
**Toner & Tu (cond-mat/9804180v1)** is the primary match — it provides the foundational mathematical framework for how large groups achieve coherent collective motion, which directly maps to GA2's "formations, shield wall, flanking" requirement. The ordered-phase concept and Goldstone-mode response are the theoretical shape for legion behavior. **2312.06383v1** validates the phase transition in multi-agent systems and the cluster-structure emergence. **2105.09764v1** provides the group-formation game framing.

**The papers are ideas-only**: the physics-based float models cannot run inside the engine's deterministic integer-only tick. The mathematical insight (field-based coherent motion, phase transitions, cluster emergence) must be reimplemented as integer arithmetic with stable tie-breaking. The plan already names Orkin FEAR lineage and Treuille continuum crowds as paper references; these arxiv papers extend that foundation. **No code is taken**; the `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported. GA2 composes GA1's `DepthBrain` — it is not a second brain.

---

## 2. ED8 — Crash-Safe Journals (editor)

### Board row
`[ ] ED8 editor: crash-safe journal — write-ahead log for the editor journal: one writer, append then checkpoint, readers pinned to an end mark, a torn tail recovered on open, each entry stamped with the LW6 tick and revision`. Tier B. Size M. Owns `crates/editor/src/wal.rs`. Accept: 100 edits with undo and redo, a killed process and a corrupt tail all reopen to the last good state x2; apply cost ≤ 2x today. needs: ED7.

### Search results
Three arxiv hits directly address crash-safe journals and WALs in Rust:

**2406.09649v1** — "SquirrelFS: using the Rust compiler to check file-system crash consistency" (LeBlanc, Taylor, Bornholt, Chidambaram, 2024, cs.OS).
- **What it claims**: A new approach to building crash-safe file systems for persistent memory using Rust's typestate pattern to enforce, at compile time, a specific order of operations. Introduces "Synchronous Soft Updates" — boils crash safety to enforcing ordering among updates to file-system metadata. Compiling SquirrelFS takes tens of seconds; successful compilation indicates crash consistency, while a compilation error provides a starting point for fixing the bug. Evaluated against NOVA and WineFS with similar or better performance.
- **What transfers**: This is the most directly relevant paper. It proves that Rust's type system can enforce crash-consistency invariants at compile time — exactly what ED8 needs for its WAL. The "one writer, append then checkpoint" pattern maps to SquirrelFS's "Synchronous Soft Updates" where ordering of metadata updates is enforced. The "torn tail recovered on open" maps to the recovery mechanism that SquirrelFS's typestate makes impossible by construction (the compiler refuses invalid states). The pattern of "readers pinned to an end mark" maps to SquirrelFS's metadata ordering enforcement.
- **What does NOT transfer**: SquirrelFS is a full file system, not a journal implementation. The paper targets persistent-memory file systems, not editor journals. The typestate approach is specific to file-system metadata operations. The paper's implementation cannot be vendored (it's a file-system implementation, not a library). The `PORT()` + `EVIDENCE.md` rule applies if the typestate pattern is adapted for the WAL.

**2507.13062v1** — "Design and Reliability of a User Space Write-Ahead Log in Rust" (Pellegatti & Vieira, 2025, cs.OS/cs.DB).
- **What it claims**: Describes the experience of creating a prototype user-space WAL in Rust. Observes that Rust is easy to use, compact, and has a rich set of libraries. The overhead is minimal, with the WAL prototype operating at basically the expected performance of the stable memory device. WALs must be reliable while maintaining high performance because all operations are written to the WAL to ensure stability.
- **What transfers**: Directly validates that a user-space WAL in Rust is practical and performant. Confirms that Rust's ownership and type system make WAL implementation straightforward ("easy to use, compact"). The minimal overhead finding means ED8's WAL will not add meaningful latency to editor operations. The reliability discussion maps to ED8's requirement that "100 edits with undo and redo, a killed process and a corrupt tail all reopen to the last good state x2."
- **What does NOT transfer**: The paper is about a prototype WAL, not a production-ready implementation. The specific implementation details (buffer management, fsync strategy, recovery algorithm) are context-specific. The paper's WAL is for database-style operations, not editor command batches. No code is taken; the design insight (Rust makes WALs practical and performant) is the transfer.

**2408.02911v2** — "Boosting File Systems Elegantly: A Transparent NVM Write-ahead Log for Disk File Systems" (Wang et al., 2024/2025, cs.OS, FAST '25).
- **What it claims**: NVLog, an NVM-based write-ahead log for disk file systems. Provides on-demand byte-granularity sync absorption, reserving the fast DRAM path for async operations. Highly efficient log structure with mechanisms for heterogeneous crash consistency, optimized for small writes. Delivers up to 15x acceleration and outperforms NOVA/SPFS.
- **What transfers**: The design of an efficient WAL optimized for small writes (which matches ED8's command-batch pattern). The "byte-granularity sync absorption" concept maps to how ED8 can batch multiple editor commands before fsync while maintaining crash safety. The heterogeneous crash consistency mechanisms map to the WAL's recovery design.
- **What does NOT transfer**: NVLog is a kernel file-system component, not an editor journal. The NVM-specific optimizations don't apply to the editor's in-memory + disk journal. The kernel-level implementation cannot be ported to a user-space Rust crate. Ideas only.

### Current engine state
- ED7 (grouped commands) is ticked and integrated — commands carry names, `is_significant` flags, and groups.
- E3 (save/reopen, child play, journal) is unit-tested and integrated (editor 52/52).
- ED8 is the crash-safe journal layer on top of ED7's command batches.
- The plan references "SQLite WAL design, public domain, patterns only" — the existing ED8 design already uses this pattern.
- The `wal.rs` file does NOT exist yet (ED8 is missing, file-absent by glob).

### Falsifiable experiment
1. **WAL append + crash recovery**: Write 100 edit commands through the WAL, kill the process mid-write, restart, and verify the journal recovers to the last good checkpoint with no corruption. **Falsifiable**: if the recovered journal contains partial commands or the wrong checkpoint, the WAL recovery is broken.
2. **Undo/redo through WAL**: Apply 100 edits with undo/redo through the WAL, then crash and recover. Verify undo/redo produces the same sequence of states as before the crash. **Falsifiable**: if undo/redo diverges after recovery, the WAL doesn't preserve command ordering.
3. **Corrupt tail recovery**: Corrupt the last 5 entries of the WAL tail, restart, and verify the reader is pinned to the last good end mark and the corrupt tail is discarded. **Falsifiable**: if the reader reads corrupt data or panics, the torn-tail recovery is broken.
4. **Apply cost**: Measure the apply cost of 100 edits through the WAL vs the baseline (without WAL). **Falsifiable**: if apply cost exceeds 2x the baseline, the WAL overhead is too high.
5. **Determinism**: Run the same 100-edit sequence twice. Verify the resulting journal bytes are identical. **Falsifiable**: if journal bytes differ, the WAL is non-deterministic.

### Real consumer
`crates/editor/src/wal.rs` (new file, consumer: `crates/editor/src/commands.rs` ED7, crates/editor/src/editor.rs (planned) E3). The WAL feeds the editor's durable journal and undo/redo system. The `proof` crate validates the WAL as part of the crash-safety gate.

### Kill criteria
- WAL recovery produces partial commands or wrong checkpoint → kill ED8 recovery design.
- Undo/redo diverges after crash recovery → kill ED8 command-ordering guarantee.
- Corrupt tail is read instead of being discarded → kill ED8 torn-tail recovery.
- Apply cost exceeds 2x baseline → kill ED8 WAL design, optimize or simplify.
- Journal bytes differ between runs with the same input → kill ED8 determinism.
- Any panic on corrupt input → kill ED8; the WAL must recover gracefully.

### Verdict: **TAKE** (bounded, ideas-only)
**SquirrelFS (2406.09649v1)** is the primary match — it proves that Rust's typestate pattern can enforce crash-consistency invariants at compile time, which directly informs ED8's WAL design. The "Synchronous Soft Updates" mechanism (ordering metadata updates) maps to ED8's "one writer, append then checkpoint" pattern. The compile-time enforcement concept maps to the WAL's requirement that invalid states (partial writes, torn tails) are impossible by construction. **2507.13062v1** validates that a user-space WAL in Rust is practical and performant with minimal overhead — directly confirming ED8's feasibility. **2408.02911v2** provides additional WAL design patterns for efficient small-write batching.

**The papers are ideas-only**: no code is taken. SquirrelFS is a full file-system implementation that cannot be vendored; the typestate pattern concept is what transfers. The engine's existing ED8 design already uses "SQLite WAL design, public domain, patterns only" — these papers reinforce and deepen the design with Rust-specific crash-safety guarantees. The `PORT()` + `EVIDENCE.md` rule applies if the typestate pattern is adapted for the WAL.

---

## 3. NV7 — Versioned Paths over Dynamic Navmesh (navigation)

### Board row
`[ ] NV7 nav: version-stamped paths over NV3 tiles — every path carries its tile-version vector; a rebake salts references and repaths exactly the stale set, so fighters never chase a dead polygon`. Tier A. Size S. Owns `crates/nav/src/tiles.rs` version hunks (no new file). Accept: break fixtures bump exactly the dirty tiles and repath exactly the stale paths; bytes x2. needs: NV3.

### Search results
One arxiv hit is directly on-point, plus one adjacent:

**2602.04130v1** — "Multi-threaded Recast-Based A* Pathfinding for Scalable Navigation in Dynamic Game Environments" (Madushanka & Madushanka, 2026, cs.GR/cs.RO).
- **What it claims**: A multi-threaded framework enhancing standard A* through Recast-based mesh generation, Bezier-curve trajectory smoothing, and density analysis for crowd coordination. Evaluated across ten incremental phases from 2D mazes to complex multi-level dynamic worlds. Maintains 350+ FPS with 1000 simultaneous agents and achieves collision-free crowd navigation through density-aware path coordination.
- **What transfers**: This is the most directly relevant paper. It addresses navigation in dynamic game environments with incremental phases — exactly the scenario NV7 handles with version-stamped tiles. The Recast-based mesh generation maps to NV3's tile system. The incremental phases concept maps to NV7's tile-version-vector approach where each rebake bumps versions. The density-aware path coordination maps to NV7's requirement that "fighters never chase a dead polygon" — stale paths are detected and repathed. The multi-threaded framework maps to the engine's async pathfinding design (pathfinding runs outside the tick with bounded budgets).
- **What does NOT transfer**: The paper uses float-based A* and Recast, while the engine uses integer-only navmesh with `E2NAV1` v1 format. The paper's Bezier-curve smoothing is float-based presentation work. The 350+ FPS result is on different hardware. The paper's density analysis uses float coordinates. All of this must be reimplemented in integer math. The `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported.

**2605.21397** (already used in wave-30 for NV3) — "Validating Navmesh using Geometry" — provides voxel-based navmesh validation that supports NV3's tile system, which NV7 builds on.

### Current engine state
- NV1 (navmesh bake + query) is unit-tested and integrated: `crates/nav/src/mesh.rs` (1221 lines, `E2NAV1` v1, sorted polygons, byte-identical bakes x2).
- NV3 (dynamic tiles + flow fields) is the rowed dependency: `crates/nav/src/rebake.rs` (396 lines, `BreakSet` sorted + deduped, `TileRegistry`, version bumps in id order). The `tiles.rs` file does NOT exist yet (NV7's file).
- NV7 depends on NV3: the `TileRegistry` in `rebake.rs` already bumps versions; NV7 adds the version-vector stamp to paths and the stale-set repath logic.
- `crates/nav/src/{astar,bake,flow}.rs` plus `lib.rs` `Grid` are the query fallback (legacy, to be retired by NV6 funnel).
- The plan says: "versioned tiles; each DX1 break set names exactly its dirty tiles, rebake bumps versions, fighters finish then repath on the new version."
- NV7 is Tier A and must land before the finish line.

### Falsifiable experiment
1. **Version-stamp correctness**: Bake a navmesh, query a path, then rebake with one dirty tile. Verify the path carries the old tile-version vector and is marked stale. **Falsifiable**: if the path's version vector doesn't match the tile version it was computed on, the stamping is broken.
2. **Stale-set repath**: After a rebake, verify exactly the stale paths (those whose tile versions changed) are repathed and non-stale paths are not. **Falsifiable**: if non-stale paths are repathed or stale paths are not, the stale-set detection is broken.
3. **Determinism**: Run the same rebake + path query twice. Verify identical bytes. **Falsifiable**: if path bytes differ, the version-stamped path computation is non-deterministic.
4. **Dead-polygon avoidance**: After a rebake invalidates a tile, verify no agent queries a path through the dead polygon. **Falsifiable**: if a path references a dead polygon, the stale detection failed.
5. **Budget**: Measure the version-stamp overhead per path query. **Falsifiable**: if the overhead exceeds 0.2 ms p50 (the path query budget from game-ai.md §3), NV7 is too slow.
6. **Break fixture**: Run a DX1 break set that invalidates specific tiles, then verify exactly those tiles are bumped and exactly the affected paths are repathed. **Falsifiable**: if the wrong tiles are bumped or the wrong paths are repathed, the version-vector system is broken.

### Real consumer
`crates/nav/src/tiles.rs` (new version hunks within existing file, consumer: `crates/nav/src/` query system, `crates/ai/src/` path requests, `crates/destruction/src/` DX1 break sets). The `TileRegistry` in `rebake.rs` provides the version-bump primitive. The arena match calls NV7 through `game` for unit movement.

### Kill criteria
- Version vector doesn't match the tile version the path was computed on → kill NV7 stamping.
- Stale paths are not repathed or non-stale paths are repathed → kill NV7 stale-set detection.
- Path bytes differ between runs with the same seed → kill NV7 determinism.
- A path references a dead polygon → kill NV7 immediately.
- Version-stamp overhead exceeds 0.2 ms p50 → kill NV7 budget, simplify.
- Break fixture bumps wrong tiles → kill NV7, redesign version-bump logic.

### Verdict: **TAKE** (bounded, reimplementation)
**2602.04130v1** is the primary match — it directly addresses navigation in dynamic game environments using Recast-based mesh generation with incremental phases and density-aware path coordination. The incremental-phase evaluation (from 2D mazes to complex multi-level dynamic worlds) maps to NV7's requirement to handle progressive tile invalidation. The density-aware path coordination maps to NV7's dead-polygon avoidance. The multi-threaded framework maps to the engine's async pathfinding design.

**This paper is a reimplementation candidate**: the concepts (Recast-based mesh, incremental navmesh updates, density-aware coordination) must be ported to engine2040's integer-only `E2NAV1` v1 format with deterministic replay. The paper's float-based A* and Bezier smoothing cannot be used as-is. The existing `rebake.rs` `TileRegistry` already provides the version-bump primitive; NV7 adds the version-vector stamp to paths and the stale-set repath logic. **The paper is ideas-only** for the mathematical approach; the `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported.

---

## Wave-31 Research Card Summary

| Field | GA2 | ED8 | NV7 |
|---|---|---|---|
| **Paper** | Toner & Tu cond-mat/9804180v1 + 2312.06383v1 + 2105.09764v1 | SquirrelFS 2406.09649v1 + WAL-in-Rust 2507.13062v1 | Recast A* 2602.04130v1 |
| **Verdict** | TAKE (bounded, ideas-only) | TAKE (bounded, ideas-only) | TAKE (bounded, reimplementation) |
| **Experiment** | Formation coherence + shield wall hold + flanking + determinism + budget | WAL append/recovery + undo/redo + corrupt tail + apply cost + determinism | Version-stamp correctness + stale-set repath + dead-polygon avoidance + determinism + budget |
| **Consumer** | `crates/ai/src/legion.rs` (new) | `crates/editor/src/wal.rs` (new) | `crates/nav/src/tiles.rs` version hunks |
| **Kill criterion** | Formation variance ≥ 5°; wall breaks < 500 ticks; determinism divergence; budget exceeded | Recovery produces partial commands; undo/redo diverges; apply cost > 2x; journal non-deterministic | Version mismatch; stale paths not repathed; dead polygon referenced; overhead > 0.2 ms |
| **What stays true** | GA1 `DepthBrain` composes legion logic; integer-only; no second brain | ED7 grouped commands exist; plan already uses SQLite WAL pattern; `wal.rs` absent | NV1 baked; NV3 `TileRegistry` exists; `rebake.rs` version bumps |
| **What does NOT transfer** | Float physics; continuum fields; GPU simulation | Full file-system code; kernel WAL; NVM-specific optimizations | Float A*; Bezier smoothing; Recast library; kernel-level concurrency |

---

## Evidence rows (proposed for RESEARCH-VERDICT.md)

The following rows amend `docs/RESEARCH-VERDICT.md` in the "2026 additions" section:

| Idea | Source | Verdict |
|---|---|---|
| Legion AI / flocking formations | Toner & Tu cond-mat/9804180v1 (ideas only); Sar & Ghosh 2312.06383v1 (supporting); Takko et al. 2105.09764v1 (supporting) | TAKE (bounded); continuum theory of coherent collective motion maps to GA2 formations; integer-only reimplementation required |
| Crash-safe journal / WAL in Rust | SquirrelFS 2406.09649v1 (ideas only); Pellegatti & Vieira 2507.13062v1 (supporting) | TAKE (bounded); Rust typestate enforces crash-consistency at compile time; confirms user-space WAL is practical and performant |
| Versioned paths in dynamic game environments | 2602.04130v1 (reimplementation) | TAKE (bounded); Recast-based incremental navigation in dynamic game worlds maps to NV7 tile-version-vector stamped paths; integer-only reimplementation required |

---

## Unverified

- **GA2 formation parameters**: The Toner & Tu model predicts phase transitions at specific coupling strengths, but the exact integer-quantized parameters for the engine's formation system are not yet determined. The 5° heading variance threshold and 100-tick coherence window are proposed experiment parameters, not validated numbers.
- **ED8 WAL performance**: SquirrelFS reports "tens of seconds" for compile-time verification and "similar or better performance" than NOVA/WineFS, but these numbers are for a full file system, not an editor journal. The 2x apply-cost kill threshold needs measurement against the engine's baseline.
- **NV7 version-vector overhead**: The paper reports 350+ FPS with 1000 agents, but this is on different hardware with float math. The 0.2 ms p50 path-query budget needs verification on the reference GPU after integer-only adaptation.
- **SquirrelFS typestate applicability**: The paper demonstrates typestate enforcement for file-system metadata operations. Whether the same pattern applies to command-batch journals (with undo/redo) is not yet verified — this is a research hypothesis, not a confirmed design.
- **2602.04130v1 paper details**: Only the abstract was read via arxiv API. The full paper's incremental-phase evaluation details, Bezier-curve implementation, and density-analysis algorithm were not read. The "reimplementation" take is bounded to the abstract-level framing.
- **No code was taken from any paper**: All three papers are ideas-only or reimplementation candidates. The engine's integer-only, deterministic, replay-exact approach supersedes all float-based physics simulations from the papers.
- **GA2 paper selection**: The Toner & Tu paper is the foundational physics model for flocking, but it was published in 1998 and targets physical simulation, not game AI. The 2312.06383v1 paper is a more recent multi-agent flocking model. The 2105.09764v1 paper is about human-agent coordination. These three papers together provide the full picture, but no single paper is a perfect match for GA2's specific "formations, shield wall, flanking" requirement.
- **ED8 "SQLite WAL design" reference**: The existing plan references "SQLite WAL design, public domain, patterns only" as the ED8 donor. The SquirrelFS paper provides a Rust-specific approach to the same problem (crash consistency via type enforcement) that complements, rather than replaces, the SQLite WAL pattern reference.
