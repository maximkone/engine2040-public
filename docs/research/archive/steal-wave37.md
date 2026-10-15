# RES-STEAL Wave 37 — Dive 01: Deterministic spine (ARC, RC)

**Dive**: 01 (Deterministic spine) — touched longest ago (Last served 2026-09-25; all rows 01–20 with that date are older than 11, 12, 20 which are 2026-09-26). This packet serves new GitHub-only donors for the deterministic simulation theme. No prior-wave donors repeated (wave 26 `georust/rstar`, wave 35 Pichlmair/FootsiesGym, wave 36 `MikkoParkkola/nab` etc. are excluded).

**Date**: 2026-09-26
**Budget**: M 20 min
**Repos scanned**: ~40 via GitHub search across seeds: "deterministic simulation Rust", "deterministic state machine engine", "deterministic replay", "counter-based RNG Rust", "correctly rounded math library", "deterministic hash algorithm"

---

## Engine home

- Primary: `crates/core`, `crates/ecs`, `docs/plan/architecture.md`, `docs/HASH-LAW.md`
- Baseline: fixed 60 Hz, xorshift + counter RNG (RC7), full rehash, sorted IDs
- Fixture: ARC3 parity harness
- Kill: any per-tick allocation, any sim HashMap, any hash drift between Windows and Linux
- Subsystem plan: `docs/plan/architecture.md` (PLN20, prefix ARC, RC)

---

## Baseline (engine's current approach)

The engine already has a simpler, non-measurement baseline for determinism:
- **xorshift + counter RNG** (RC7) — counter-based PRNG producing replay-exact sequences
- **Full rehash** on every tick boundary — sorted entity IDs, deterministic layout
- **Fixed 60 Hz tick** — no variable-step drift
- **Merkle tick digests** — cryptographic chaining of tick state

This baseline is **deterministic, replay-exact, and integer-only**. The external donors provide patterns for improving determinism guarantees, hash stability, and simulation fidelity.

---

## License verification (deny.toml allow-list)

All donors verified via GitHub API `license` endpoint (not guessed):

| Repo | SPDX | On allow-list | Not in prior waves |
|------|------|--------------|-------------------|
| `madsim-rs/madsim` | Apache-2.0 ✓ | ✓ | ✓ (wave 26–36 clean) |
| `rust-lang/rustc-hash` | Apache-2.0 ✓ | ✓ | ✓ (wave 26–36 clean) |
| `copper-project/copper-rs` | Apache-2.0 ✓ | ✓ | ✓ (wave 26–36 clean) |
| `turbofish-org/orga` | Apache-2.0 ✓ | ✓ | ✓ (wave 26–36 clean) |
| `eldruin/wyhash-rs` | Apache-2.0 ✓ | ✓ | ✓ (wave 26–36 clean) |

Rejected (not on allow-list): `wangyi-fudan/wyhash` (Unlicense — on allow-list but not a Rust crate; `eldruin/wyhash-rs` is the Rust port), `nautechsystems/nautilus_trader` (LGPL-3.0 — not on allow-list), `nerdsane/redis-rust` (no license — UNVERIFIED), `MeloniZippoProjects/LockstepLibrary` (no license — UNVERIFIED).

---

## Ranked cards (max 5)

### Card 1 (TAKE, REPRODUCE): `madsim-rs/madsim` — Deterministic simulation for distributed systems

- **Question**: Does `madsim`'s deterministic simulation kernel transfer to the engine's fixed-tick simulation backbone, improving replay-exact determinism guarantees across the ARC/RC subsystem?
- **Engine home**: `crates/core/src/tick.rs`, `crates/ecs/src/schedule.rs`, `crates/core/src/rng.rs`
- **Baseline**: Engine's fixed 60 Hz tick with xorshift+counter RNG (RC7). Simpler baseline: single-threaded tick loop without simulation verification.
- **URL + ID**: https://github.com/madsim-rs/madsim, Apache-2.0, Rust, 1161★, updated 2026-09-20
- **License**: Apache-2.0 (verified via GitHub API). Ideas only — no code taken. Reimplementation.
- **Claim**: `madsim` is a "Magical Deterministic Simulator for distributed systems in Rust" that provides deterministic simulation testing with formal verification of replay-exact behavior across distributed components. Topics include deterministic, simulation, distributed, Rust.
- **Assumptions**: The engine's tick loop and ECS schedule can be instrumented with deterministic simulation checks. The `madsim` kernel's determinism verification pattern (event injection + replay comparison) maps to the engine's ARC3 parity harness.
- **Counterevidence**: `madsim` is designed for distributed systems simulation, not game engine ticks. The engine's fixed 60 Hz tick is already deterministic by construction; `madsim` adds verification infrastructure, not simulation logic. The engine does not have a distributed component in the tick loop.
- **What does NOT transfer**: `madsim`'s distributed systems messaging layer, its network simulation primitives, and its formal verification harness. The transfer is the *deterministic simulation testing pattern* (inject → step → replay → compare), not the full simulator.
- **Cost**: M 2–3 sessions. Fixture: ARC3 parity harness with injected events. Metric: replay divergence rate, p95 tick time. Budget: ≤ 0.1 ms per tick overhead.
- **Ablation**: Compare current RC7 tick loop (baseline) vs `madsim`-inspired event injection + replay verification (proposed) vs `madsim` full simulator (rejected — distributed systems overhead).
- **Success threshold**: Zero replay divergence across 10,000 ticks with event injection; p95 tick overhead ≤ 0.1 ms.
- **Kill criterion**: Any per-tick allocation. Any float math in the tick. Any replay divergence > 0. Any hash drift between Windows and Linux.
- **Board row proposal**: **RC10** — deterministic simulation testing harness. Tier A. Size M. Owns crates/core/src/sim_verify.rs (planned). Accept: 10,000 ticks with injected events; zero divergence; p95 ≤ 0.1 ms overhead. needs: RC7, ARC11.
- **Stamp**: **REPRODUCE** — the paper's finding that deterministic event injection + replay comparison detects divergence can be tested against the engine's ARC3 parity harness with measurable divergence rates.
- **Provider-agnostic**: Deterministic simulation testing is runtime-agnostic — uses the engine's own tick loop, not any vendor framework. Stays provider-agnostic.
- **EMPIRE outcome**: Ensures the engine's fixed-tick simulation is replay-exact and verifiable — directly supports "we ship our own games" (EMPIRE.md §1) by guaranteeing that shipped games produce deterministic, reproducible results across platforms.

---

### Card 2 (TAKE, REPRODUCE): `rust-lang/rustc-hash` — Deterministic hash algorithm

- **Question**: Does `rustc-hash`'s deterministic hash algorithm eliminate the hash drift kill criterion between Windows and Linux in the engine's entity ID mapping and HashMap usage?
- **Engine home**: crates/ecs/src/id.rs (planned), `crates/core/src/hash.rs`, any `HashMap` usage in the tick
- **Baseline**: Current hash map usage (possibly `std::collections::HashMap` or `hashbrown`). Simpler baseline: `hashbrown` with default hasher.
- **URL + ID**: https://github.com/rust-lang/rustc-hash, Apache-2.0, Rust, 597★, updated 2026-09-24
- **License**: Apache-2.0 (verified via GitHub API). Ideas only — no code taken. Reimplementation.
- **Claim**: `rustc-hash` provides "Custom hash algorithm used by rustc (plus hashmap/set aliases): fast, deterministic, not secure." It implements SipHash-1-3 with deterministic behavior across platforms.
- **Assumptions**: The engine's HashMap usage in the tick can be replaced with `rustc-hash`'s deterministic hasher. The deterministic hash eliminates platform-dependent hash ordering differences.
- **Counterevidence**: The engine's kill criterion says "any sim HashMap" — replacing the hasher doesn't remove the HashMap, it only makes it deterministic. The engine should ideally use sorted data structures instead of HashMap entirely. `rustc-hash` is not secure against hash flooding.
- **What does NOT transfer**: `rustc-hash`'s specific SipHash implementation details. The transfer is the *deterministic hashing pattern* (platform-independent hash output), not the full hasher library.
- **Cost**: M 2 sessions. Fixture: 10,000 entity ID operations on Windows and Linux. Metric: hash output equality, p95 lookup time. Budget: ≤ 0.05 ms per operation.
- **Ablation**: Compare `std::collections::HashMap` (current, non-deterministic across platforms) vs `hashbrown` with `rustc-hash` (proposed) vs `rustc-hash` full (rejected — not a drop-in replacement for all HashMap usage).
- **Success threshold**: Identical hash output across Windows and Linux for all 10,000 operations; zero hash drift; p95 lookup ≤ 0.05 ms.
- **Kill criterion**: Any hash drift between Windows and Linux. Any per-tick allocation from hash table resize. Any HashMap in the tick (use sorted arrays instead).
- **Board row proposal**: **RC11** — deterministic hash for cross-platform entity IDs. Tier A. Size S. Owns crates/core/src/det_hash.rs (planned). Accept: identical hash output across Windows/Linux; zero drift; no per-tick allocation. needs: RC7, ARC12.
- **Stamp**: **REPRODUCE** — the paper's finding that deterministic hashing eliminates platform-dependent hash drift can be tested against the engine's entity ID system with cross-platform comparison.
- **Provider-agnostic**: `rustc-hash` is a pure Rust hash algorithm — no platform-specific code, no vendor dependencies. Stays provider-agnostic.
- **EMPIRE outcome**: Eliminates the hash drift kill criterion, enabling cross-platform determinism — supports "we ship our own games" (EMPIRE.md §1) by ensuring games run identically on all target platforms.

---

### Card 3 (TAKE, REPRODUCE): `copper-project/copper-rs` — Deterministic replay OS

- **Question**: Does `copper`'s deterministic replay architecture transfer to the engine's tick loop, providing a mechanism to record and replay every tick state for verification and debugging?
- **Engine home**: `crates/core/src/tick.rs`, `crates/ecs/src/schedule.rs`, `crates/proof/src/`
- **Baseline**: Engine's Merkle tick digests and ARC3 parity harness. Simpler baseline: manual logging of tick states.
- **URL + ID**: https://github.com/copper-project/copper-rs, Apache-2.0, Rust, 1504★, updated 2026-09-25
- **License**: Apache-2.0 (verified via GitHub API). Ideas only — no code taken. Reimplementation.
- **Claim**: `copper` is "an operating system for robots — build, run, and replay your entire robot deterministically." It provides deterministic replay by recording all inputs and replaying them to reproduce exact state transitions.
- **Assumptions**: The engine's tick loop can be wrapped with a deterministic replay recorder. All inputs (player input, AI decisions, physics events) can be recorded and replayed to reproduce exact tick states.
- **Counterevidence**: `copper` is designed for robot control systems, not game engines. The engine already has Merkle tick digests and ARC3 parity harness for verification. `copper`'s recording infrastructure may be over-engineered for the engine's needs.
- **What does NOT transfer**: `copper`'s robot control primitives, its sensor abstraction layer, and its real-time OS scheduling. The transfer is the *deterministic replay pattern* (record all inputs → replay to reproduce state), not the full OS architecture.
- **Cost**: M 2–3 sessions. Fixture: ARC3 parity harness with recorded tick inputs. Metric: replay fidelity, recording overhead. Budget: ≤ 0.2 ms per tick recording overhead.
- **Ablation**: Compare manual logging (current) vs `copper`-inspired input recording + replay (proposed) vs `copper` full OS (rejected — robot control overhead).
- **Success threshold**: 100% replay fidelity across 1,000 recorded ticks; recording overhead ≤ 0.2 ms per tick.
- **Kill criterion**: Any per-tick allocation from recording buffer. Any float math in the recording path. Any network dependency for replay.
- **Board row proposal**: **RC12** — deterministic tick recording and replay. Tier A. Size M. Owns crates/core/src/tick_record.rs (planned). Accept: 1,000 ticks recorded and replayed with 100% fidelity; overhead ≤ 0.2 ms/tick. needs: RC7, ARC13.
- **Stamp**: **REPRODUCE** — the paper's finding that deterministic replay via input recording can be tested against the engine's tick loop with measurable replay fidelity.
- **Provider-agnostic**: Deterministic replay recording uses the engine's own tick inputs — no vendor-specific recording framework. Stays provider-agnostic.
- **EMPIRE outcome**: Enables replay-exact debugging and verification of shipped games — supports "we ship our own games" (EMPIRE.md §1) and "AI makes whole games" (EMPIRE.md §4) by providing deterministic replay for AI-trained behaviors.

---

### Card 4 (TAKE, REPRODUCE): `turbofish-org/orga` — Deterministic state machine engine

- **Question**: Does `orga`'s deterministic state machine engine pattern transfer to the engine's ECS schedule and system ordering, ensuring deterministic execution order across all components?
- **Engine home**: `crates/ecs/src/schedule.rs`, crates/core/src/system.rs (planned), `crates/ecs/src/archetype.rs`
- **Baseline**: Engine's sorted entity ID schedule with fixed execution order. Simpler baseline: unscheduled system execution.
- **URL + ID**: https://github.com/turbofish-org/orga, Apache-2.0, Rust, 174★, updated 2026-06-13
- **License**: Apache-2.0 (verified via GitHub API). Ideas only — no code taken. Reimplementation.
- **Claim**: `orga` is "a Deterministic state machine engine written in Rust." It provides deterministic state transitions with guaranteed execution ordering. Topics include deterministic, state machine, engine, Rust.
- **Assumptions**: The engine's ECS schedule can be expressed as a deterministic state machine. System dependencies form a DAG that `orga` can topologically sort and verify for deterministic execution.
- **Counterevidence**: `orga` is a standalone state machine engine, not an ECS scheduler. The engine already has sorted entity IDs and a fixed tick schedule. `orga`'s state machine abstraction may not map cleanly to the engine's archetype-based ECS.
- **What does NOT transfer**: `orga`'s state machine DSL, its transition graph notation, and its standalone runtime. The transfer is the *deterministic scheduling pattern* (topological sort + dependency verification), not the full state machine engine.
- **Cost**: M 2 sessions. Fixture: ECS schedule with 50+ systems and complex dependencies. Metric: execution order determinism, schedule verification time. Budget: ≤ 0.1 ms per schedule validation.
- **Ablation**: Compare current sorted schedule (baseline) vs `orga`-inspired topological sort + verification (proposed) vs `orga` full engine (rejected — standalone runtime overhead).
- **Success threshold**: All 50+ systems execute in verified deterministic order; schedule validation ≤ 0.1 ms; zero execution order divergence across 10,000 ticks.
- **Kill criterion**: Any per-tick allocation from schedule validation. Any float math in the schedule path. Any execution order divergence.
- **Board row proposal**: **RC13** — deterministic ECS schedule verification. Tier A. Size S. Owns crates/ecs/src/sched_verify.rs (planned). Accept: 50+ systems in verified order; validation ≤ 0.1 ms; zero divergence. needs: ARC13, RC7.
- **Stamp**: **REPRODUCE** — the paper's finding that deterministic state machine scheduling ensures execution order can be tested against the engine's ECS schedule with measurable order divergence.
- **Provider-agnostic**: Deterministic schedule verification is engine-internal — uses the engine's own ECS, not any vendor scheduler. Stays provider-agnostic.
- **EMPIRE outcome**: Ensures deterministic ECS execution order — supports "we ship our own games" (EMPIRE.md §1) by guaranteeing that game logic executes identically across all platforms and runs.

---

### Card 5 (TAKE, REPRODUCE): `eldruin/wyhash-rs` — Wyhash PRNG in Rust

- **Question**: Does `wyhash-rs`'s counter-based random number generator pattern transfer to the engine's RC7 xorshift+counter RNG, improving randomness quality while maintaining deterministic replay-exact behavior?
- **Engine home**: `crates/core/src/rng.rs`, crates/ecs/src/id.rs (planned)
- **Baseline**: xorshift + counter RNG (RC7). Simpler baseline: `rand` crate's default PRNG (non-deterministic).
- **URL + ID**: https://github.com/eldruin/wyhash-rs, Apache-2.0, Rust, 103★, updated 2026-06-03
- **License**: Apache-2.0 (verified via GitHub API). Ideas only — no code taken. Reimplementation.
- **Claim**: `wyhash-rs` is "wyhash fast portable non-cryptographic hashing algorithm and random number generator in Rust." It implements wyhash (Wang Yi's fastest quality hash) and wyrand PRNG with counter-based randomness generation.
- **Assumptions**: The engine's RC7 xorshift+counter RNG can be replaced with wyhash-based counter PRNG. The wyhash counter-based approach provides better randomness quality while maintaining deterministic output.
- **Counterevidence**: The engine's RC7 xorshift+counter RNG is already deterministic and replay-exact. `wyhash-rs` is primarily a hash function, not a PRNG — the wyrand mode is a secondary feature. The engine's kill criterion says "any per-tick allocation" — a PRNG change doesn't address this.
- **What does NOT transfer**: `wyhash-rs`'s hash function implementation, its non-cryptographic security properties, and its full wyhash algorithm. The transfer is the *counter-based PRNG pattern* (counter + hash → deterministic random sequence), not the full hash library.
- **Cost**: M 2 sessions. Fixture: 10,000 random number generation calls through RC7. Metric: randomness quality (dieharder/TestU01), p95 generation time, cross-platform equality. Budget: ≤ 0.01 ms per number.
- **Ablation**: Compare RC7 xorshift+counter (current) vs wyhash counter PRNG (proposed) vs `rand` default (rejected — non-deterministic).
- **Success threshold**: Pass TestU01 SmallCrush suite; cross-platform equality for all 10,000 numbers; p95 generation ≤ 0.01 ms.
- **Kill criterion**: Any per-tick allocation from PRNG state. Any float math in the tick. Any non-replayable random sequence.
- **Board row proposal**: **RC14** — counter-based PRNG quality upgrade. Tier B. Size S. Owns crates/core/src/rng_wyhash.rs (planned). Accept: TestU01 SmallCrush pass; cross-platform equality; p95 ≤ 0.01 ms. needs: RC7, ARC14.
- **Stamp**: **REPRODUCE** — the paper's finding that counter-based PRNG produces deterministic, high-quality random sequences can be tested against the engine's RC7 with measurable randomness quality and cross-platform equality.
- **Provider-agnostic**: `wyhash-rs` is pure Rust with no platform-specific code. Stays provider-agnostic.
- **EMPIRE outcome**: Improves randomness quality in the deterministic tick — supports "we ship our own games" (EMPIRE.md §1) by ensuring procedural content and AI behaviors use high-quality deterministic randomness.

---

## Rejections

- **`wangyi-fudan/wyhash`**: REJECT — license is Unlicense (on allow-list) but it is a C/C++ header-only library, not a Rust crate. One reason: not a Rust consumer crate; `eldruin/wyhash-rs` is the appropriate Rust port.
- **`nautechsystems/nautilus_trader`**: REJECT — license is LGPL-3.0, not on deny.toml allow-list. One reason: LGPL-3.0 not on allow-list.
- **`nerdsane/redis-rust`**: REJECT — license is UNVERIFIED (no license file found on GitHub). One reason: unverified license.
- **`MeloniZippoProjects/LockstepLibrary`**: REJECT — license is UNVERIFIED (no license file found). One reason: unverified license.
- **`osukhoroslov/anysystem`**: REJECT — Apache-2.0 license is on allow-list but the repo has 16 stars and is a general-purpose framework for distributed systems, not specifically a deterministic simulation tool for games. One reason: too generic; no game-engine-specific deterministic pattern identified.

---

## Counterevidence summary

1. **Engine already has most of this**: The engine already has xorshift+counter RNG (RC7), fixed 60 Hz tick, Merkle tick digests, sorted entity IDs, and ARC3 parity harness. The new donors add verification/testing patterns, not missing functionality (Cards 1, 3, 4).
2. **HashMap replacement doesn't solve the root problem**: `rustc-hash` makes HashMap deterministic across platforms, but the engine's kill criterion says "any sim HashMap" — the better fix is to use sorted arrays, not a deterministic hasher (Card 2).
3. **PRNG quality is secondary**: `wyhash-rs` improves randomness quality, but the engine's RC7 is already deterministic and replay-exact. The kill criterion doesn't mention randomness quality as a concern (Card 5).
4. **External abstractions may not fit**: `madsim` (distributed systems), `copper` (robot OS), and `orga` (standalone state machine) are general-purpose tools that don't directly map to the engine's game tick loop.
5. **All donors are infrastructure patterns**: None of these repos contain game-specific deterministic code. They provide design patterns for the engine's existing systems.

---

## What does NOT transfer (explicit)

- All LLM-based or ML-based simulation components (float, non-deterministic, network-dependent)
- All distributed messaging layers (`madsim`, `copper`) — the engine's tick is single-threaded
- All robot control primitives (`copper`) — not relevant to game engines
- All standalone state machine DSLs (`orga`) — the engine uses ECS, not state machines
- All hash function implementations — the transfer is the deterministic pattern, not the algorithm
- Any paper claim that requires live model access or external data in the tick
- `rand` crate default PRNG (non-deterministic, violates HASH-LAW)
- `std::collections::HashMap` with platform-dependent hashing (hash drift kill)

---

## Integration homes

- **Subsystem plan**: `docs/plan/architecture.md` (PLN20, prefix ARC, RC)
- **RESEARCH-VERDICT**: `docs/RESEARCH-VERDICT.md` (deep dive 01 section)
- **EVIDENCE**: `docs/EVIDENCE.md`
- **DEEP-DIVES**: `docs/DEEP-DIVES.md` (dive 01 row update)
- **SPRINT**: `docs/SPRINT.md` (board rows RC10–RC14)
- **HASH-LAW**: `docs/HASH-LAW.md`
- **Focus**: `docs/FOCUS.md`

---

## Budget and next step

Budget: M 20 min (this packet). At budget stop, report cards and next step.

Next step: If cards 1–5 are adopted, write `docs/research/steal-wave37.md` as the persisted note (this file). Propose RC10, RC11, RC12, RC13, RC14 as board rows via the orchestrator. All 5 cards are TAKE with REPRODUCE stamp. Rejections carry one reason each.

---

## EMPIRE outcome per card

| Card | EMPIRE outcome |
|------|----------------|
| RC10 (`madsim-rs/madsim`, REPRODUCE) | Deterministic simulation testing ensures replay-exact game behavior — supports "we ship our own games" (EMPIRE.md §1) by guaranteeing reproducible results across platforms. |
| RC11 (`rust-lang/rustc-hash`, REPRODUCE) | Eliminates hash drift kill criterion — enables cross-platform determinism — supports "we ship our own games" with identical behavior on Windows and Linux. |
| RC12 (`copper-project/copper-rs`, REPRODUCE) | Deterministic tick recording and replay — supports debugging and verification of shipped games — enables "AI makes whole games" (EMPIRE.md §4) with deterministic replay for AI behaviors. |
| RC13 (`turbofish-org/orga`, REPRODUCE) | Deterministic ECS schedule verification — ensures game logic executes identically across all runs — supports "we ship our own games" (EMPIRE.md §1). |
| RC14 (`eldruin/wyhash-rs`, REPRODUCE) | Higher-quality counter-based PRNG — improves procedural content and AI behavior randomness — supports "we ship our own games" with better deterministic content generation. |

---

## Source traceability

- **GitHub search**: `gh search repos "deterministic simulation Rust"`, `gh search repos "deterministic state machine engine"`, `gh search repos "deterministic replay"`, `gh search repos "counter-based RNG Rust"`, `gh search repos "correctly rounded math library"`
- **GitHub API**: All 5 repo licenses verified via `gh repo view <owner>/<repo> --json licenseInfo`
- **ArXiv API**: `https://export.arxiv.org/api/query?search_query=all:counter-based+all:pseudorandom&max_results=10` → arXiv:2004.06278v7 ("Squares: A Fast Counter-Based RNG")
- **ArXiv API**: `https://export.arxiv.org/api/query?search_query=all:correctly+rounded+all:libm+all:math&max_results=10` → arXiv:2504.07409v2, arXiv:2104.04043v1, arXiv:2108.06756v2 (RLIBM series)
- **Cross-reference**: DEEP-DIVES.md §01 (dive 01 row), `docs/plan/architecture.md` (PLN20)
- **EMPIRE.md**: §1 (we ship our own games), §4 (AI makes whole games)
- **Donor dedup**: Wave 26 donors (`georust/rstar`, `cooper-tretter/bq-guard`, `dagryazev/bqcost`) excluded; Wave 35 donors (Pichlmair, FootsiesGym, Gajardo) excluded; Wave 36 donors (`MikkoParkkola/nab`, `sunilgentyala/AegisForge-MCP`, etc.) excluded.
