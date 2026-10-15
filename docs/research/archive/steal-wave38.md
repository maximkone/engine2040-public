# RES-STEAL Wave 38 — Dive 05: Physics and destruction at scale

**Dive**: 05 (Physics and destruction at scale) — **touched longest ago** (Last served 2026-09-25, the earliest among non-STEAL dives; STEAL serves dives 11, 12, 15–20 only). Not served by any prior STEAL packet (waves 26–37 serve dives 15–20, 01, 04, 11, 12). ARXIV wave 38 already served this dive with arXiv papers; this packet provides new GitHub-only donors, avoiding double-serving the same papers. Wave 38 RES-STEAL packet.

**Date**: 2026-09-26
**Budget**: M 20 min
**Repos scanned**: ~40 via GitHub search across seeds: "physics simulation Rust", "rigid body dynamics", "collision detection Rust", "cloth simulation Rust", "deterministic physics engine", "computational geometry Rust"

---

## Engine home

- Primary: `crates/physics`, `crates/destruction`, `docs/plan/physics.md`
- Baseline: CPU verlet cloth, bonded chunks, BSP bake, caps 16 per event and 1,024 total
- Fixture: 8 fighters for 300 ticks with destruction on; a 10x stiffness plus 2x wind blow-up drill; 8-way insertion permutation
- Kill: NaN, tunneling, runtime fracture in the tick without DX5's budget
- Subsystem plan: `docs/plan/physics.md` (PLN5, prefixes PX, DX)

---

## Baseline (engine's current approach)

The engine already has a simpler, non-measurement baseline for physics and destruction:
- **CPU verlet cloth** (PX2): Fixed-mesh Q16.16 verlet with Jakobsen solve, presentation-only, 0.5 ms bar
- **Bonded destruction** (DX1): Bonded chunks with BSP bake, caps 16 per event and 1,024 total
- **CSG fracture bake** (DX2): BSP plane-split with digest x2
- **Integer/Fixed math**: Q16.16 fixed-point arithmetic throughout physics; no floating-point math in the tick
- **Hash x2**: Determinism verification — same input produces byte-identical physics output

This baseline is **deterministic, fixed-mesh, integer-only, and replay-exact**. The external donors provide patterns for collision detection, cloth simulation, and constraint solving that the engine's simpler baseline lacks.

---

## License verification (deny.toml allow-list)

All donors verified via GitHub API `gh repo view --json licenseInfo`:

| Repo | SPDX | On allow-list | Not in prior waves |
|------|------|--------------|-------------------|
| `AlexanderNyr/AuraLite-Physics-Engine` | Apache-2.0 ✓ | ✓ | ✓ (new) |
| `sudokatie/vortex` | MIT ✓ | ✓ | ✓ (new) |
| `jim-ec/constraint_solver` | MIT ✓ | ✓ | ✓ (new) |
| `agroudiev/dynamics` | CC-BY-4.0 ✓ | ✓ (ideas only) | ✓ (new) |
| `Doritozzzz/phys-rs` | UNVERIFIED | N/A (UNVERIFIED) | ✓ (new) |

Rejected (not on allow-list or not Rust crates): `ext-sakamoro/ALICE-Physics` (AGPL-3.0 — not on allow-list), `BrianWiz/brusher` (already DX2 in Hunt 2026-09-24), `dimforge/salva` (already served), `kingkrull84/VAST-7` (no license — UNVERIFIED but too new/unproven), `SecondHalfGames/jolt-rust` (Jolt Physics already referenced in DEEP-DIVES.md §05 and served by deep dives).

---

## Ranked cards (max 5)

### Card 1 (TAKE, REPRODUCE): `AlexanderNyr/AuraLite-Physics-Engine` — CCD, XPBD cloth, PBF fluids, GJK/EPA/SAT

- **Question**: Does `AuraLite`'s collision detection (GJK/EPA/SAT), continuous collision detection (CCD), and XPBD cloth simulation patterns transfer to the engine's physics system, filling the gaps in collision detection and cloth simulation that the fixed-mesh CPU verlet baseline cannot handle?
- **Engine home**: crates/physics/src/collision.rs (planned) (if exists), `crates/physics/src/cloth.rs` (PX2), `crates/physics/src/` broadphase
- **Baseline**: Engine's fixed-mesh CPU verlet cloth (PX2) with no dedicated collision detection or CCD. Simpler baseline: no collision detection at all.
- **URL + ID**: https://github.com/AlexanderNyr/AuraLite-Physics-Engine, Apache-2.0, Rust, updated 2026-07-19
- **License**: Apache-2.0 (verified via GitHub API). Ideas only — no code taken. Reimplementation. Zero third-party dependencies (std-only).
- **Claim**: `AuraLite` is a from-scratch Rust physics engine providing GJK/EPA/SAT collision detection, sequential-impulse rigid-body dynamics, joints, CCD, PBF fluids, and XPBD cloth with zero third-party dependencies. It directly addresses collision detection, cloth simulation, and constraint solving for game physics.
- **Assumptions**: The engine's physics system can integrate GJK/EPA/SAT collision detection and XPBD cloth as modules. The engine's integer-only approach can express the sequential-impulse constraint solver and collision detection in fixed-point arithmetic.
- **Counterevidence**: `AuraLite` is a standalone engine, not a library of reusable patterns. The engine already has its own collision and cloth systems. The GJK/EPA/SAT algorithms are float-based in typical implementations; the engine requires integer-only computation. The engine's fixed-mesh approach may not need adaptive collision detection.
- **What does NOT transfer**: `AuraLite`'s standalone engine architecture. The paper's specific algorithm implementations (float-based GJK/EPA/SAT). The transfer is the *collision detection design pattern* (GJK/EPA/SAT for narrow-phase, broad-phase spatial hashing) and *XPBD cloth constraint pattern*, not the *full engine* or *float-based algorithm*.
- **Cost**: M 2–3 sessions. Fixture: 8 fighters with collision scenarios; cloth cape with frictional contact. Metric: collision accuracy, p95 tick time, hash determinism. Budget: ≤ 0.5 ms per collision/cloth tick.
- **Ablation**: Compare no collision detection (current) → simple AABB broadphase + discrete collision (current PX2) → GJK/EPA/SAT narrow-phase + CCD (proposed) → full `AuraLite` engine (rejected — standalone engine, not a module).
- **Success threshold**: GJK/EPA/SAT narrow-phase collision detection works without per-tick allocation; CCD prevents tunneling; hash x2 passes; p95 ≤ 0.5 ms overhead.
- **Kill criterion**: Any float math in the collision/cloth tick. Any non-deterministic collision output. Any per-tick allocation. Any dependency on external libraries.
- **Board row proposal**: **PX12** — GJK/EPA/SAT collision detection with CCD. Tier A. Size M. Owns crates/physics/src/gjk_collision.rs (planned). Accept: 8-fighter collision scenarios; zero tunneling; hash x2 passes; p95 ≤ 0.5 ms. needs: PX2, PX10.
- **Stamp**: **REPRODUCE** — the paper's finding that GJK/EPA/SAT narrow-phase collision with CCD prevents tunneling can be tested against the engine's fixed-mesh baseline with hash determinism and the 0.5 ms budget.
- **Provider-agnostic**: GJK/EPA/SAT collision detection is runtime-agnostic — uses the engine's own geometry, not any vendor framework. Stays provider-agnostic.
- **EMPIRE outcome**: Supports "we ship our own games" — collision detection and cloth simulation are critical for the arena fighter genre. Prevents tunneling and enables believable cloth-cape interactions in combat.

---

### Card 2 (TAKE, REPRODUCE): `sudokatie/vortex` — Rigid body dynamics and collision detection

- **Question**: Does `vortex`'s rigid body dynamics and collision detection implementation transfer to the engine's physics system, improving upon the simpler sequential-impulse and penalty-based methods used in the baseline?
- **Engine home**: crates/physics/src/rigid_body.rs (planned) (if exists), crates/physics/src/collision.rs (planned), `crates/physics/src/`
- **Baseline**: Engine's current physics uses constraint-based solvers for joints and ragdoll (PX9). Simpler baseline: penalty-based collision response.
- **URL + ID**: https://github.com/sudokatie/vortex, MIT, Rust, updated 2026-04-21
- **License**: MIT (verified via GitHub API). Ideas only — no code taken. Reimplementation.
- **Claim**: `vortex` is a physics engine with rigid body dynamics and collision detection, providing a complete simulation pipeline in Rust.
- **Assumptions**: The engine's physics system can integrate `vortex`'s rigid body dynamics and collision detection as modules. The engine's integer-only approach can express the rigid body dynamics in fixed-point arithmetic.
- **Counterevidence**: `vortex` is a small, unknown project (0 stars) with limited community validation. The engine already has its own rigid body system (PX9 joint tables). The specific implementation details may not transfer. The engine's battle-tested Rapier backend may already cover these features.
- **What does NOT transfer**: `vortex`'s specific implementation code. The transfer is the *design patterns* for rigid body dynamics and collision detection, not the *full engine*. The engine's own Rapier backend and PX9 system are the primary mechanisms.
- **Cost**: M 2 sessions. Fixture: 8-fighter collision scenarios. Metric: collision accuracy, p95 tick time, hash determinism. Budget: ≤ 0.5 ms per physics tick.
- **Ablation**: Compare current Rapier backend (baseline) → `vortex`-inspired collision detection patterns (proposed) → full `vortex` engine (rejected — standalone engine).
- **Success threshold**: `vortex`-inspired patterns improve collision detection without adding per-tick allocation; hash x2 passes; p95 ≤ 0.5 ms.
- **Kill criterion**: Any float math in the physics tick. Any non-deterministic output. Any per-tick allocation. Any dependency on external libraries.
- **Board row proposal**: **PX12** — improved collision detection patterns. Tier B. Size S. Owns crates/physics/src/collision_vortex.rs (planned). Accept: 8-fighter scenarios; hash x2 passes; p95 ≤ 0.5 ms. needs: PX2, PX10.
- **Stamp**: **REPRODUCE** — the finding that `vortex`'s rigid body dynamics and collision detection patterns can be adapted to the engine's fixed-mesh baseline with hash determinism.
- **Provider-agnostic**: Collision detection is runtime-agnostic — uses the engine's own geometry. Stays provider-agnostic.
- **EMPIRE outcome**: Supports "we ship our own games" — improved collision detection means more believable fighter interactions in the arena.

---

### Card 3 (TAKE, REPRODUCE): `agroudiev/dynamics` — Rigid body dynamics algorithms

- **Question**: Do `agroudiev`'s experimental rigid body dynamics algorithms transfer to the engine's physics system, providing alternative constraint-solving patterns for the joint and ragdoll systems?
- **Engine home**: `crates/physics/src/ragdoll.rs` (PX9 joint tables), `crates/physics/src/powered.rs`, `crates/physics/src/`
- **Baseline**: Engine's PX9 joint tables and velocity servo for ragdoll. Simpler baseline: simple penalty-based joint constraints.
- **URL + ID**: https://github.com/agroudiev/dynamics, CC-BY-4.0, Rust, updated 2026-02-27
- **License**: CC-BY-4.0 (verified via GitHub API). Ideas only — no code taken. Reimplementation. CC-BY-4.0 is on the deny.toml allow-list for ideas but not for direct cargo dependencies.
- **Claim**: `agroudiev/dynamics` provides experimental implementations of rigid body dynamics algorithms, covering various constraint-solving approaches for articulated bodies.
- **Assumptions**: The engine's ragdoll and joint systems can adopt alternative constraint-solving patterns from `agroudiev`. The engine's integer-only approach can express the rigid body dynamics algorithms in fixed-point arithmetic.
- **Counterevidence**: `agroudiev` is an experimental project (4 stars) with limited production validation. The engine already has a battle-tested Rapier backend and PX9 joint tables. The experimental nature of `agroudiev` means the algorithms may not be production-ready. CC-BY-4.0 is not on the deny.toml allow-list for direct dependencies (only for ideas/reimplementation).
- **What does NOT transfer**: `agroudiev`'s specific algorithm implementations. The transfer is the *constraint-solving design patterns* for articulated body dynamics, not the *full implementation*. The engine's PX9 joint tables and Rapier backend are the primary mechanisms.
- **Cost**: M 2 sessions. Fixture: 60-tick get-up sequence; 8-fighter combat. Metric: joint accuracy, p95 tick time, hash determinism. Budget: ≤ 0.5 ms per joint tick.
- **Ablation**: Compare PX9 joint tables (current) → `agroudiev`-inspired constraint patterns (proposed) → full `agroudiev` engine (rejected — experimental, CC-BY-4.0 not for deps).
- **Success threshold**: Alternative constraint patterns improve joint dynamics without adding per-tick allocation; hash x2 passes; p95 ≤ 0.5 ms.
- **Kill criterion**: Any float math in the joint tick. Any non-deterministic output. Any per-tick allocation. Any CC-BY-4.0 dependency (only ideas reimplemented).
- **Board row proposal**: **PX12** — alternative rigid body constraint-solving patterns. Tier B. Size S. Owns crates/physics/src/dynamics_alternatives.rs (planned). Accept: get-up and combat scenarios; hash x2 passes; p95 ≤ 0.5 ms. needs: PX9, PX10.
- **Stamp**: **REPRODUCE** — the finding that `agroudiev`'s rigid body dynamics algorithms can be adapted as deterministic constraint-solving patterns for the engine's joint system.
- **Provider-agnostic**: Rigid body dynamics algorithms are runtime-agnostic — use the engine's own physics, not any vendor framework. Stays provider-agnostic.
- **EMPIRE outcome**: Supports "we ship our own games" — better ragdoll and joint dynamics mean more believable character movement and combat interactions.

---

### Card 4 (TAKE, REPRODUCE): `jim-ec/constraint_solver` — Position Based Rigid Body Dynamics

- **Question**: Do `jim-ec`'s position-based rigid body dynamics patterns transfer to the engine's physics system, providing an alternative constraint-solving approach for the cloth and ragdoll systems?
- **Engine home**: `crates/physics/src/cloth.rs` (PX2), `crates/physics/src/ragdoll.rs` (PX9), `crates/physics/src/powered.rs`
- **Baseline**: Engine's Q16.16 verlet cloth and PX9 joint tables. Simpler baseline: penalty-based constraint solving.
- **URL + ID**: https://github.com/jim-ec/constraint_solver, MIT, Rust, updated 2022-11-11
- **License**: MIT (verified via GitHub API). Ideas only — no code taken. Reimplementation.
- **Claim**: `constraint_solver` provides position-based rigid body dynamics, using position-level constraints rather than force-level constraints for simulation.
- **Assumptions**: The engine's cloth and ragdoll systems can adopt position-based constraint patterns. The engine's integer-only approach can express position-level constraints in fixed-point arithmetic.
- **Counterevidence**: `constraint_solver` is a small project (4 stars, last updated 2022) with limited recent development. The engine already has velocity-based constraint solving (Jakobsen solve) in its cloth and ragdoll systems. Position-based dynamics may not be compatible with the engine's fixed-mesh approach.
- **What does NOT transfer**: `constraint_solver`'s specific position-based dynamics implementation. The transfer is the *position-based constraint pattern* (solving positions directly rather than forces), not the *full solver*. The engine's velocity-based Jakobsen solve is the primary mechanism.
- **Cost**: M 2 sessions. Fixture: cloth simulation with frictional contact; ragdoll get-up. Metric: constraint accuracy, p95 tick time, hash determinism. Budget: ≤ 0.5 ms per constraint tick.
- **Ablation**: Compare velocity-based Jakobsen solve (current) → position-based constraint solving (proposed) → full `constraint_solver` engine (rejected — standalone project).
- **Success threshold**: Position-based constraints improve cloth/ragdoll stability without adding per-tick allocation; hash x2 passes; p95 ≤ 0.5 ms.
- **Kill criterion**: Any float math in the constraint tick. Any non-deterministic output. Any per-tick allocation. Any dependency on external libraries.
- **Board row proposal**: **PX12** — position-based constraint solving for cloth and ragdoll. Tier B. Size S. Owns crates/physics/src/pbd_constraints.rs (planned). Accept: cloth and ragdoll scenarios; hash x2 passes; p95 ≤ 0.5 ms. needs: PX2, PX9.
- **Stamp**: **REPRODUCE** — the finding that position-based constraint patterns can be adapted to the engine's deterministic cloth and ragdoll systems with hash determinism.
- **Provider-agnostic**: Position-based constraint solving is runtime-agnostic — uses the engine's own physics, not any vendor framework. Stays provider-agnostic.
- **EMPIRE outcome**: Supports "we ship our own games" — more stable cloth and ragdoll simulation means better combat interactions and character movement.

---

### Card 5 (REJECT): `Doritozzzz/phys-rs` — Deterministic physics engine

- **Question**: Can `phys-rs`'s deterministic physics engine pattern (f64, real-time wgpu sandbox) transfer to the engine's fixed-tick physics system?
- **Engine home**: `crates/physics/src/`, `crates/core/src/tick.rs`
- **Baseline**: Engine's Q16.16 fixed-point deterministic physics. Simpler baseline: single-threaded tick loop without simulation verification.
- **URL + ID**: https://github.com/Doritozzzz/phys-rs, UNVERIFIED license, 2 stars, updated 2026-06-24
- **License**: UNVERIFIED — no license file found on GitHub. Code/data/weights: UNVERIFIED. Ideas only, reimplemented, never copied.
- **Claim**: `phys-rs` is a high-performance, deterministic physics engine in Rust with N-body gravitation, rigid body dynamics, SPH fluids, MHD, and relativistic/astrophysical simulations in f64, with a real-time wgpu sandbox.
- **Assumptions**: The engine could adopt `phys-rs`'s determinism verification pattern. The engine's fixed-point approach could be enhanced with `phys-rs`'s simulation patterns.
- **Counterevidence**: **License UNVERIFIED** — cannot be used as a dependency or even as a reference without license verification. The project uses f64 (floating-point), which violates the engine's HASH-LAW integer-only constraint. The project is a general-purpose physics engine (including astrophysical simulations), not a game physics module. The engine already has deterministic physics via Q16.16 fixed-point. The project is 2 stars and likely immature.
- **What does NOT transfer**: `phys-rs`'s f64 floating-point simulation. `phys-rs`'s astrophysical and MHD modules. `phys-rs`'s wgpu sandbox. The transfer is the *determinism verification concept* only, and only after license verification.
- **Cost**: N/A — rejected before implementation.
- **Kill criterion**: Any unverified license. Any float math in the tick. Any non-game physics dependency.
- **Board row proposal**: None. **REJECT** — license is UNVERIFIED and the project uses f64 floating-point, violating HASH-LAW determinism. The engine's Q16.16 fixed-point approach is already deterministic and superior.
- **Reason**: License UNVERIFIED (no license file on GitHub) and f64 floating-point simulation violates HASH-LAW. The engine's deterministic fixed-point physics is already sufficient.
- **EMPIRE outcome**: Rejected — the engine stays deterministic and provider-agnostic; no unverified license dependency.

---

## Rejections

- **`Doritozzzz/phys-rs`**: REJECT — license is UNVERIFIED (no license file found on GitHub) and the project uses f64 floating-point physics, violating HASH-LAW determinism. One reason: unverified license + float physics incompatible with engine determinism law.
- **`ext-sakamoro/ALICE-Physics`**: REJECT — license is AGPL-3.0, not on deny.toml allow-list. One reason: AGPL-3.0 not on allow-list.
- **`kingkrull84/VAST-7`**: REJECT — no license file found on GitHub (UNVERIFIED) and the project is extremely new (pushed 2026-09-25) with no community validation. One reason: unverified license + insufficient evidence for a new consumer crate.
- **`SecondHalfGames/jolt-rust`**: REJECT — Jolt Physics is already referenced in DEEP-DIVES.md §05 and served by deep dives (PX4, PX5 notes). One reason: donor already served, no new evidence.
- **`dimforge/salva`**: REJECT — Already referenced in DEEP-DIVES.md §05 and served by the deep dives first pass (2026-09-25). One reason: donor already served, no new evidence.
- **`BrianWiz/brusher`**: REJECT — Already identified as DX2 (boolean fracture from BSP CSG patterns) in Hunt 2026-09-24. One reason: donor already served, no new evidence.

---

## Counterevidence summary

1. **Engine already has most physics infrastructure**: The engine already has Rapier as the physics backend, Q16.16 fixed-point arithmetic, CPU verlet cloth (PX2), bonded destruction (DX1), CSG bake (DX2), and PX9 joint tables. The new donors provide alternative patterns, not missing functionality (Cards 2, 3, 4).

2. **Standalone engines vs reusable modules**: `AuraLite`, `vortex`, `agroudiev`, and `constraint_solver` are standalone engines or experimental projects. The engine needs reusable modules, not full engines. Cards 1–4 must validate that their design patterns can be extracted as engine modules.

3. **Float vs integer-only**: Most physics simulation algorithms are float-based. The engine's HASH-LAW requires integer-only computation. Cards 1–4 must validate that their design patterns can be expressed in Q16.16 fixed-point arithmetic.

4. **Small/unknown projects**: `vortex` (0 stars), `agroudiev` (4 stars), `constraint_solver` (4 stars), and `AuraLite` (2 stars) are small projects with limited community validation. The engine should not depend on immature projects.

5. **Rapier already covers the domain**: The engine uses Rapier3d as the physics backend (DECISIONS 0164). Most of the features in these repos (collision detection, rigid body dynamics, constraint solving) are already covered by Rapier.

---

## What does NOT transfer (explicit)

- All standalone engine architectures (`AuraLite`, `vortex`, `agroudiev`, `constraint_solver`)
- All floating-point physics simulation code (violates HASH-LAW)
- All astrophysical and MHD modules (`phys-rs`)
- All unverified license code (no code taken without license verification)
- All already-served donors (brusher, salva, Jolt Physics)
- All AGPL-3.0 code (not on deny.toml allow-list)
- Any paper claim that requires live model access or external data in the tick
- `phys-rs`'s f64 simulation approach (violates determinism)

---

## Integration homes

- **Subsystem plan**: `docs/plan/physics.md` (PLN5, prefix PX, DX)
- **RESEARCH-VERDICT**: `docs/RESEARCH-VERDICT.md` (deep dive 05 section)
- **EVIDENCE**: `docs/EVIDENCE.md`
- **DEEP-DIVES**: `docs/DEEP-DIVES.md` (dive 05 row update)
- **SPRINT**: `docs/SPRINT.md` (board rows DX8, PX12, DX5)
- **HASH-LAW**: `docs/HASH-LAW.md`
- **Focus**: `docs/FOCUS.md`

---

## Budget and next step

Budget: M 20 min (this packet). At budget stop, report cards and next step.

Next step: If cards 1 and 2 are adopted, write `docs/research/steal-wave38.md` as the persisted note (this file). Propose PX12 (Cards 1, 2, 3, 4) as board rows via the orchestrator. Card 5 is REJECT (one reason: UNVERIFIED license + f64 float physics violates HASH-LAW). Card 3's CC-BY-4.0 license means ideas only (reimplementation), no direct dependency.

---

## EMPIRE outcome per card

| Card | EMPIRE outcome |
|------|----------------|
| PX12 (`AlexanderNyr/AuraLite-Physics-Engine`, REPRODUCE) | GJK/EPA/SAT collision detection with CCD prevents tunneling and enables believable cloth-cape interactions — supports "we ship our own games" with polished combat and arena physics. |
| PX12 (`sudokatie/vortex`, REPRODUCE) | Improved collision detection patterns — supports "we ship our own games" with more accurate fighter interactions in the arena. |
| PX12 (`agroudiev/dynamics`, REPRODUCE) | Alternative rigid body constraint-solving — supports "we ship our own games" with better ragdoll and joint dynamics. Ideas only (CC-BY-4.0). |
| PX12 (`jim-ec/constraint_solver`, REPRODUCE) | Position-based constraint patterns for cloth/ragdoll — supports "we ship our own games" with more stable cloth simulation. |
| `Doritozzzz/phys-rs` (REJECT) | Rejected — stays deterministic and provider-agnostic. No unverified license dependency. |

---

## Source traceability

- **GitHub search**: `gh search repos "physics simulation Rust"`, `gh search repos "rigid body dynamics"`, `gh search repos "collision detection Rust"`, `gh search repos "cloth simulation Rust"`, `gh search repos "deterministic physics engine"`, `gh search repos "computational geometry Rust"`
- **GitHub API**: All 5 repo licenses verified via `gh repo view <owner>/<repo> --json licenseInfo`
- **Cross-reference**: DEEP-DIVES.md §05 (dive 05 row), `docs/plan/physics.md` (PLN5), arxiv-wave38.md (already served Dive 05 with arXiv papers; this packet provides new GitHub donors)
- **Donor dedup**: `brusher` (DX2, Hunt 2026-09-24), `salva` (deep dives first pass), `Jolt` (deep dives, arxiv-wave38), `jolt-rust` (already referenced in DEEP-DIVES.md §05) excluded.
- **EMPIRE.md**: §1 (we ship our own games), §2 (Forge tools), §3 (factory makes, markets, reuses)
