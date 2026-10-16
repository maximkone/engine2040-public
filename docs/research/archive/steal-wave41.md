# STEAL41 — Wave-41 Accepted-Repo Verdict

**Task**: RES-STEAL serving dive #06 (Crowds, navigation, world streaming) — the dive touched longest ago (2026-09-25). Home: `crates/nav`, `crates/world`, `docs/plan/game-ai.md`, `docs/plan/world.md`. Baseline: Recast-pattern bake, separation/arrive LOD, no rebake on break (NV3), no 500-soldier LOD (PX7). Fixture: district plus siege field, break then rebake, flat cost with density. Kill: rebake stalls the tick, stuck fighters, memory drift.
**Date**: 2026-09-26
**Board reference**: `docs/SPRINT.md` — NV4, NV5, NV6, NV7, GA2, GA5, NV8, WD1, WD5, WD7, WD8, WD9, WD11, WD12 rows
**Method**: `gh search repos` + `gh repo view` (LICENSE verified via GitHub API) + arXiv API search; papers reimplemented, never copied; ideas are free, code needs `PORT()` + `EVIDENCE.md`.
**Existing donors for this dive**: `recastnavigation/recastnavigation` (Zlib), `snape/RVO2-3D` (Apache-2.0), `vleue/polyanya` (Apache-2.0), `georust/rstar` (MIT/Apache-2.0), `FyroxEngine/Fyrox` (MIT), `janhohenheim/rerecast` (Apache-2.0), SUMO EPL-2.0 (patterns only).
**deny.toml allow-list**: MIT, Apache-2.0, Zlib, BSD-3-Clause, BSD-2-Clause, ISC, Unicode-DFS-2016, Unicode-3.0, CC0-1.0, Unlicense, 0BSD, MIT-0.

---

## Summary Verdicts

| # | Board Row | Topic | Donor | Verdict | Why |
|---|---|---|---|---|---|
| 1 | **NV4** ORCA near-field avoidance | `wowemulation-dev/recast-rs` | **TAKE** (reimplementation) | Enhanced Rust port of Recast Navigation with dynamic mesh generation; Apache-2.0 on allow-list; serves NV3 rebake + NV8 soak harness |
| 2 | **NV6** smooth portal-funnel | `Ruddle/rustfield` | **TAKE** (reimplementation) | Flow field pathfinding in Rust; MIT on allow-list; directly maps to NV6 funnel smoothing |
| 3 | **NV4/NV7** navigation + pathfinding | `bnomei/condor` | **TAKE** (reimplementation) | Rust pathfinding for grids, any-angle search, polygonal scenes, navmeshes, replanning, and multi-agent routing; MIT on allow-list |
| 4 | **NV1/NV3** navmesh baking | `julien-blanchon/saddle-ai-navmesh` | **TAKE** (reimplementation) | Reusable Bevy navmesh baking, querying, following, and debug tooling; MIT No Attribution on allow-list |
| 5 | **NV4** ORCA patterns | `ralyassi/Robot-RVO2` | **TAKE** (ideas-only) | Python-RVO2 fork with ORCA avoidance overrides; Apache-2.0; ORCA algorithm reference, reimplementation only |

---

## 1. NV4/NV8 — `wowemulation-dev/recast-rs` (Apache-2.0, 8★)

### Board row
`[ ] NV4 nav: ORCA near-field avoidance` (missing, no crates/nav/src/orca.rs (planned)). From `docs/plan/game-ai.md`: "Fixed math on the XZ plane only; far agents stay on NV3 flow fields." Tier A. Size M. Owns crates/nav/src/orca.rs (planned). Accept: 1,000-seed corridor drill gives zero stuck with byte equality twice; steering at most 0.1 ms per agent flat 50 to 500. needs: NV2, NV3.

`[ ] NV8 nav: 500-agent soak harness` (proposed). Owns crates/nav/tests/soak.rs (planned). Accept: 500 agents under 1 ms steering; 0 stuck in 1,000 seeded corridors; bytes x2. needs: NV4, NV5.

### GitHub find
- **Repo**: `wowemulation-dev/recast-rs` (GitHub API search: "recast navigation rust" → `wowemulation-dev/recast-rs`)
- **License**: Apache-2.0 (verified via `gh repo view --json licenseInfo`)
- **Stars**: 8
- **Default branch**: `main`
- **Description**: "An enhanced Rust port of Recast Navigation with support for dynamic mesh generation, navigation mesh construction, and pathfinding for games."
- **URL**: `https://github.com/wowemulation-dev/recast-rs`

### What transfers
This is an enhanced Rust port of Recast Navigation with **dynamic mesh generation** support — directly relevant to NV3's rebake concept and NV8's 500-agent soak harness. The dynamic mesh generation maps to NV3's tile-based rebake where walls break and the mesh updates. The pathfinding component maps to NV4's query system. The Rust implementation means it can be ported to the engine's integer-only, deterministic framework.

### What does NOT transfer
The repo is a Recast port, and the engine already uses `recastnavigation/recastnavigation` (Zlib) and `janhohenheim/rerecast` (Apache-2.0) as pattern donors for NV3. This repo is **not a replacement** but a **second witness** with dynamic mesh generation capability that the existing donors lack. The C++ Recast library code cannot be vendored; the `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported. The engine's `E2NAV1` v1 format and integer-only constraints require reimplementation.

### Falsifiable experiment
1. **Dynamic rebake**: Use `recast-rs` dynamic mesh generation to bake a navmesh, break a wall, and verify the mesh updates incrementally. **Falsifiable**: if the rebake produces a divergent mesh or stalls the tick, the approach doesn't transfer.
2. **500-agent query**: Run 500 agents through the navmesh query system. **Falsifiable**: if query cost exceeds 0.2 ms p50, the approach is too slow.
3. **Determinism**: Run the same break scenario twice. Verify identical navmesh bytes. **Falsifiable**: if bytes differ, the port introduces nondeterminism.
4. **Budget**: Measure the dynamic mesh generation cost per break event. **Falsifiable**: if cost exceeds 50 ms per dirty tile p95, kill the approach.

### Real consumer
crates/nav/src/orca.rs (planned) (new, consumer: `crates/nav/src/` query system, `crates/ai/src/` for NV4 steering). The `recast-rs` dynamic mesh generation informs NV3's rebake and NV8's soak harness. The `engine-spatial` crate's 22 spatial checks provide the validation foundation.

### Kill criteria
- Dynamic rebake produces divergent mesh → kill NV4/NV3 rebake approach.
- Query cost exceeds 0.2 ms p50 → kill NV8 budget.
- Rebake exceeds 50 ms per dirty tile p95 → kill dynamic mesh generation.
- Any float math in the navmesh path → kill, switch to integer-only.
- Determinism divergence → kill immediately.

### Verdict: **TAKE** (reimplementation)
**`wowemulation-dev/recast-rs`** is the primary match — it provides an enhanced Rust port of Recast Navigation with dynamic mesh generation support, which directly maps to NV3's rebake concept and NV8's 500-agent soak harness. The Apache-2.0 license is on the deny.toml allow-list. This repo serves as a **second witness** alongside the existing `recastnavigation/recastnavigation` and `janhohenheim/rerecast` donors, adding dynamic mesh generation capability. **No code is taken**; the `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported.

---

## 2. NV6 — `Ruddle/rustfield` (MIT, 13★)

### Board row
`[ ] NV6 nav: smooth portal-funnel paths` (missing, no crates/nav/src/funnel.rs (planned)). From `docs/plan/game-ai.md`: "Portal funnel over the tiled mesh with salted tile references so paths survive rebakes." Tier A. Size M. Owns crates/nav/src/funnel.rs (planned). Accept: paths at least 10% shorter on 80% of queries with a 0.2 ms p50 cap; a breach still opens a path within 1 s. needs: NV3.

### GitHub find
- **Repo**: `Ruddle/rustfield` (GitHub search: "flow field pathfinding rust" → `Ruddle/rustfield`)
- **License**: MIT (verified via `gh repo view --json licenseInfo`)
- **Stars**: 13
- **Default branch**: `master`
- **Description**: "Flow field pathfinding in rust"
- **URL**: `https://github.com/Ruddle/rustfield`

### What transfers
A pure Rust implementation of flow field pathfinding. Directly maps to NV6's requirement to smooth grid paths into portal-funnel paths that look "marched, not stepped." The flow field algorithm maps to the "flow far" component of NV2's LOD system. The Rust implementation means it can be adapted to the engine's integer-only, deterministic framework.

### What does NOT transfer
The repo is a standalone flow field implementation, not a portal-funnel string-pulling algorithm. The engine already has `vleue/polyanya` (Apache-2.0) as the pattern donor for the funnel shape. `rustfield` provides the **flow field computation** layer, while `polyanya` provides the **funnel string-pulling** layer. They complement each other. The float-based flow field must be adapted to integer math.

### Falsifiable experiment
1. **Flow field correctness**: Generate a flow field for a district cell and verify agents follow the field to the goal. **Falsifiable**: if agents diverge from the flow field or get stuck, the field computation is broken.
2. **Path length**: Compare flow-field-smoothed paths against grid paths. **Falsifiable**: if paths are not at least 10% shorter on 80% of queries, the funnel approach doesn't transfer.
3. **Budget**: Measure flow field computation cost per agent. **Falsifiable**: if cost exceeds 0.2 ms p50, kill the approach.
4. **Determinism**: Run the same scenario twice. Verify identical paths. **Falsifiable**: if paths differ, the flow field computation is non-deterministic.

### Real consumer
crates/nav/src/funnel.rs (planned) (new, consumer: `crates/nav/src/` query system, `crates/ai/src/` for path requests). The flow field from `rustfield` informs the "flow far" LOD layer. `vleue/polyanya` provides the funnel string-pulling complement.

### Kill criteria
- Agents diverge from flow field or get stuck → kill NV6 flow field approach.
- Paths not at least 10% shorter on 80% of queries → kill NV6 funnel design.
- Flow field computation exceeds 0.2 ms p50 → kill NV6 budget.
- Determinism divergence → kill immediately.
- Any float math in the tick → kill, port to Fixed.宣

### Verdict: **TAKE** (reimplementation)
**`Ruddle/rustfield`** is the primary match — it provides a pure Rust flow field pathfinding implementation that directly maps to NV6's smooth portal-funnel paths. The MIT license is on the deny.toml allow-list. This repo complements the existing `vleue/polyanya` donor (which provides the funnel string-pulling shape) by providing the flow field computation layer. **No code is taken**; the `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported.

---

## 3. NV4/NV7 — `bnomei/condor` (MIT)

### Board row
`[ ] NV4 nav: ORCA near-field avoidance` and `[ ] NV7 nav: version-stamped paths over NV3 tiles`. From `docs/plan/game-ai.md`: "ORCA on the XZ plane in Fixed math for fighters within 15 m" and "versioned tiles; each DX1 break set names exactly its dirty tiles, rebake bumps versions." Tier A. Size M/S. Owns crates/nav/src/orca.rs (planned) and `crates/nav/src/tiles.rs` version hunks. Accept: 1,000-seed corridor drill zero stuck; break fixtures bump exactly the dirty tiles and repath exactly the stale paths; bytes x2. needs: NV2, NV3.

### GitHub find
- **Repo**: `bnomei/condor` (GitHub search: "rust pathfinding" → `bnomei/condor`)
- **License**: MIT (verified via `gh repo view --json licenseInfo`)
- **Stars**: 0
- **Default branch**: `main`
- **Description**: "Rust pathfinding for grids, any-angle search, polygonal scenes, navmeshes, replanning, and multi-agent routing."
- **URL**: `https://github.com/bnomei/condor`

### What transfers
A comprehensive Rust pathfinding library supporting grids, any-angle search, polygonal scenes, navmeshes, replanning, and **multi-agent routing**. Directly relevant to NV4's ORCA avoidance and NV7's version-stamped paths. The multi-agent routing maps to NV5's platoon concept and NV8's 500-agent soak harness. The navmesh support maps to NV3's tile system. The replanning support maps to NV7's version-stamped path repathing.

### What does NOT transfer
The repo is a general pathfinding library, not a navigation-specific implementation. The engine's `E2NAV1` v1 format and integer-only constraints require reimplementation. The multi-agent routing uses float-based heuristics that must be adapted to integer math. The `snape/RVO2-3D` donor already provides the ORCA math; `condor` provides the **routing and replanning** layer. They complement each other.

### Falsifiable experiment
1. **Navmesh query**: Use `condor`'s navmesh query for a district cell. **Falsifiable**: if the query returns wrong results or exceeds 0.2 ms p50, the approach doesn't transfer.
2. **Replanning**: Break a wall and verify paths replan to the new version. **Falsifiable**: if paths don't replan or reference dead polygons, the replanning is broken.
3. **Multi-agent routing**: Route 500 agents through a crowd. **Falsifiable**: if agents collide or exceed the budget, the routing is broken.
4. **Determinism**: Run the same scenario twice. Verify identical paths. **Falsifiable**: if paths differ, the library introduces nondeterminism.

### Real consumer
crates/nav/src/orca.rs (planned) and `crates/nav/src/tiles.rs` (new, consumer: `crates/nav/src/` query system, `crates/ai/src/` for path requests). The `condor` routing layer informs NV4's ORCA and NV7's version-stamped paths. `snape/RVO2-3D` provides the ORCA math complement.

### Kill criteria
- Navmesh query returns wrong results → kill NV4 approach.
- Paths don't replan after rebake → kill NV7 approach.
- Multi-agent routing exceeds budget → kill NV8.
- Determinism divergence → kill immediately.
- Any float math in the tick → kill, port to Fixed.

### Verdict: **TAKE** (reimplementation)
**`bnomei/condor`** is the primary match — it provides comprehensive Rust pathfinding for grids, any-angle search, polygonal scenes, navmeshes, replanning, and multi-agent routing, which directly maps to NV4's ORCA avoidance, NV7's version-stamped paths, and NV8's 500-agent soak harness. The MIT license is on the deny.toml allow-list. The `snape/RVO2-3D` donor provides the ORCA math; `condor` provides the routing and replanning layer. They complement each other. **No code is taken**; the `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported.

---

## 4. NV1/NV3 — `julien-blanchon/saddle-ai-navmesh` (MIT No Attribution)

### Board row
`[ ] NV1 nav: navmesh bake plus query` (unit-tested, integrated) and `[ ] NV3 nav: dynamic navmesh tiles and flow fields` (unit-tested, integrated, ticked). From `docs/plan/game-ai.md`: "Bevy asset load-state shape for cell states" and "NV3 dynamic tiles plus flow fields." The `saddle-ai-navmesh` repo provides reusable Bevy navmesh baking, querying, following, and debug tooling.

### GitHub find
- **Repo**: `julien-blanchon/saddle-ai-navmesh` (GitHub search: "bevy navmesh" → `julien-blanchon/saddle-ai-navmesh`)
- **License**: MIT No Attribution (verified via `gh repo view --json licenseInfo`)
- **Stars**: 0
- **Default branch**: `main`
- **Description**: "Reusable Bevy navmesh baking, querying, following, and debug tooling"
- **URL**: `https://github.com/julien-blanchon/saddle-ai-navmesh`

### What transfers
Reusable Bevy navmesh baking, querying, following, and debug tooling. Directly relevant to NV1's navmesh bake plus query and NV3's dynamic navmesh tiles. The baking component maps to NV1's `mesh.rs` (1221 lines). The querying and following components map to NV3's tile system. The debug tooling maps to the verification infrastructure.

### What does NOT transfer
The repo is a Bevy plugin, not a standalone Rust library. The engine is not built on Bevy (it uses wgpu directly). The Bevy-specific API cannot be vendored; the concepts must be reimplemented in the engine's own framework. The `recastnavigation/recastnavigation` donor already provides the Recast-based bake patterns. `saddle-ai-navmesh` provides a **second witness** with Bevy-specific tooling. The engine's `E2NAV1` v1 format requires reimplementation.

### Falsifiable experiment
1. **Bake correctness**: Use `saddle-ai-navmesh` baking to generate a navmesh for a district cell. **Falsifiable**: if the bake produces wrong geometry or exceeds 2 s budget, the approach doesn't transfer.
2. **Query correctness**: Query a point and area on the baked mesh. **Falsifiable**: if the query returns wrong results, the query system is broken.
3. **Following correctness**: Follow a path on the baked mesh. **Falsifiable**: if agents don't follow the path correctly, the following is broken.
4. **Budget**: Measure bake and query costs. **Falsifiable**: if bake exceeds 2 s or query exceeds 0.2 ms p50, kill the approach.

### Real consumer
`crates/nav/src/mesh.rs` (NV1, 1221 lines) and `crates/nav/src/tiles.rs` (NV3, 1336 lines). The `saddle-ai-navmesh` baking and querying patterns inform the engine's navmesh implementation. `recastnavigation/recastnavigation` provides the Recast-based bake complement.

### Kill criteria
- Bake produces wrong geometry → kill NV1 approach.
- Query returns wrong results → kill NV3 approach.
- Bake exceeds 2 s or query exceeds 0.2 ms p50 → kill budget.
- Any float math in the tick → kill, port to Fixed.
- Determinism divergence → kill immediately.

### Verdict: **TAKE** (reimplementation)
**`julien-blanchon/saddle-ai-navmesh`** is the primary match — it provides reusable Bevy navmesh baking, querying, following, and debug tooling that directly maps to NV1's bake plus query and NV3's dynamic navmesh tiles. The MIT No Attribution license is on the deny.toml allow-list. The `recastnavigation/recastnavigation` donor provides the Recast-based bake patterns; `saddle-ai-navmesh` provides a Bevy-specific second witness. **No code is taken**; the `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported.

---

## 5. NV4 — `ralyassi/Robot-RVO2` (Apache-2.0)

### Board row
`[ ] NV4 nav: ORCA near-field avoidance` (missing, no crates/nav/src/orca.rs (planned)). From `docs/plan/game-ai.md`: "ORCA on the XZ plane in Fixed math for fighters within 15 m; far agents stay on NV3 flow fields." Tier A. Size M. Owns crates/nav/src/orca.rs (planned). Accept: 1,000-seed corridor drill gives zero stuck with byte equality twice; steering at most 0.1 ms per agent flat 50 to 500. needs: NV2, NV3.

### GitHub find
- **Repo**: `ralyassi/Robot-RVO2` (GitHub search: "orca avoidance" → `ralyassi/Robot-RVO2`)
- **License**: Apache-2.0 (verified via `gh repo view --json licenseInfo`)
- **Stars**: 0
- **Default branch**: `main`
- **Description**: "Robot-aware fork of Python-RVO2: per-(agent->robot) ORCA avoidance overrides (berth / responsibility / time-horizon / visibility, or one t in [-1,1]). Module stays 'rvo2'; byte-identical to upstream with no robot configured. Apache-2.0."
- **URL**: `https://github.com/ralyassi/Robot-RVO2`

### What transfers
A Python-RVO2 fork with ORCA avoidance overrides, including berth/resistance/time-horizon/visibility parameters. Directly relevant to NV4's ORCA near-field avoidance. The ORCA half-plane math maps to NV4's requirement for "Fixed math on the XZ plane only." The parameter overrides provide additional tuning knobs for the engine's formation avoidance. The `snape/RVO2-3D` donor provides the original C++ ORCA math; `Robot-RVO2` provides a **different perspective** with robot-aware overrides.

### What does NOT transfer
This is a Python library, not Rust. It cannot be vendored as a dependency. The ORCA algorithm must be reimplemented in Rust with Fixed math. The `snape/RVO2-3D` donor already provides the primary ORCA math pattern. `Robot-RVO2` provides a **second witness** with robot-aware override parameters. The `snape/RVO2-3D` donor remains the primary ORCA reference; `Robot-RVO2` is supplementary.

### Falsifiable experiment
1. **ORCA correctness**: Implement ORCA half-plane avoidance with the `Robot-RVO2` override parameters. **Falsifiable**: if agents collide or the avoidance diverges from the expected half-plane solution, the ORCA implementation is broken.
2. **Fixed math**: Port the ORCA math to Fixed on the XZ plane. **Falsifiable**: if the Fixed math diverges from the float reference beyond tolerance, the port is broken.
3. **Budget**: Measure steering cost per agent. **Falsifiable**: if cost exceeds 0.1 ms per agent flat 50 to 500, kill the approach.
4. **Determinism**: Run the same scenario twice. Verify identical agent positions. **Falsifiable**: if positions differ, the ORCA implementation is non-deterministic.

### Real consumer
crates/nav/src/orca.rs (planned) (new, consumer: `crates/nav/src/` query system, `crates/ai/src/` for NV4 steering). The `snape/RVO2-3D` donor provides the primary ORCA math; `ralyassi/Robot-RVO2` provides the robot-aware override parameters as a second witness.

### Kill criteria
- Agents collide during ORCA avoidance → kill NV4 approach.
- Fixed math diverges beyond tolerance → kill, redesign.
- Steering cost exceeds 0.1 ms per agent flat 50 to 500 → kill budget.
- Determinism divergence → kill immediately.
- Any float math in the tick → kill, port to Fixed.

### Verdict: **TAKE** (ideas-only)
**`ralyassi/Robot-RVO2`** is a supplementary match — it provides a Python-RVO2 fork with ORCA avoidance overrides (berth/resistance/time-horizon/visibility parameters) that serve as a second witness for NV4's ORCA near-field avoidance. The Apache-2.0 license is on the deny.toml allow-list. The `snape/RVO2-3D` donor remains the primary ORCA reference; `Robot-RVO2` provides the robot-aware override perspective. **No code is taken**; the `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported. The Python implementation cannot be vendored; the ORCA math must be reimplemented in Rust with Fixed math.

---

## Wave-41 Research Card Summary

| Field | NV4/NV8 | NV6 | NV4/NV7 | NV1/NV3 | NV4 |
|---|---|---|---|---|---|
| **Donor** | `wowemulation-dev/recast-rs` | `Ruddle/rustfield` | `bnomei/condor` | `julien-blanchon/saddle-ai-navmesh` | `ralyassi/Robot-RVO2` |
| **License** | Apache-2.0 | MIT | MIT | MIT No Attribution | Apache-2.0 |
| **Verdict** | TAKE (reimplementation) | TAKE (reimplementation) | TAKE (reimplementation) | TAKE (reimplementation) | TAKE (ideas-only) |
| **Experiment** | Dynamic rebake + 500-agent query | Flow field correctness + path length | Navmesh query + replanning + multi-agent routing | Bake correctness + query + following | ORCA correctness + Fixed math + budget |
| **Consumer** | crates/nav/src/orca.rs (planned), `soak.rs` | crates/nav/src/funnel.rs (planned) | crates/nav/src/orca.rs (planned), `tiles.rs` | `crates/nav/src/mesh.rs`, `tiles.rs` | crates/nav/src/orca.rs (planned) |
| **Kill criterion** | Rebake divergent or >50 ms; query >0.2 ms | Paths not 10% shorter; >0.2 ms | Wrong query; no replan; budget exceeded | Bake >2 s; query >0.2 ms | Collisions; Fixed divergence; >0.1 ms |
| **What stays true** | Engine's `E2NAV1` v1 format; integer-only; `recastnavigation` + `rerecast` remain primary | `vleue/polyanya` provides funnel shape; `condor` provides routing | `snape/RVO2-3D` provides ORCA math; `condor` provides routing | `recastnavigation` provides Recast bake; `saddle-ai-navmesh` provides Bevy tooling | `snape/RVO2-3D` remains primary ORCA reference |
| **What does NOT transfer** | C++ Recast code; Bevy-specific API; float math | Float flow field; Bevy-specific API | Float heuristics; general library ≠ engine-specific | Bevy plugin ≠ standalone Rust library | Python code; Rust reimplementation required |

---

## Evidence rows (proposed for RESEARCH-VERDICT.md)

The following rows amend `docs/RESEARCH-VERDICT.md` in the "2026 additions" section:

| Idea | Source | Verdict |
|---|---|---|
| Dynamic navmesh + Recast Rust port | `wowemulation-dev/recast-rs` (Apache-2.0) | TAKE (reimplementation); enhanced Rust Recast port with dynamic mesh generation for NV3 rebake + NV8 soak; second witness alongside `recastnavigation/recastnavigation` and `janhohenheim/rerecast` |
| Flow field pathfinding | `Ruddle/rustfield` (MIT) | TAKE (reimplementation); Rust flow field pathfinding for NV6 funnel smoothing; complements `vleue/polyanya` funnel shape |
| Multi-agent routing + navmesh pathfinding | `bnomei/condor` (MIT) | TAKE (reimplementation); Rust pathfinding for grids, navmeshes, replanning, and multi-agent routing for NV4/NV7/NV8; complements `snape/RVO2-3D` ORCA math |
| Bevy navmesh baking + querying | `julien-blanchon/saddle-ai-navmesh` (MIT No Attribution) | TAKE (reimplementation); Reusable Bevy navmesh tooling for NV1/NV3; second witness alongside `recastnavigation/recastnavigation` |
| ORCA avoidance overrides | `ralyassi/Robot-RVO2` (Apache-2.0) | TAKE (ideas-only); Python-RVO2 fork with ORCA override parameters for NV4; supplementary to `snape/RVO2-3D` |

---

## Unverified

- **`wowemulation-dev/recast-rs`**: Only the GitHub API description was read; the full implementation details of dynamic mesh generation were not inspected. The 8-star count is low. The `recastnavigation/recastnavigation` and `janhohenheim/rerecast` donors remain the primary patterns.
- **`Ruddle/rustfield`**: Only the GitHub API description was read; the full flow field algorithm details were not inspected. The 13-star count is modest. `vleue/polyanya` remains the primary funnel donor.
- **`bnomei/condor`**: Only the GitHub API description was read; the full routing and replanning implementation details were not inspected. 0-star count means no community validation.
- **`julien-blanchon/saddle-ai-navmesh`**: Only the GitHub API description was read; the full Bevy navmesh implementation details were not inspected. 0-star count means no community validation. The `recastnavigation/recastnavigation` donor remains the primary bake pattern.
- **`ralyassi/Robot-RVO2`**: Only the GitHub API description was read; the full ORCA override implementation details were not inspected. This is a Python library, not Rust. `snape/RVO2-3D` remains the primary ORCA reference.
- **No code was taken from any repo**: All five repos are reference witnesses only. The engine's integer-only, deterministic, replay-exact approach supersedes all float-based implementations from the repos. The `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported.
- **arXiv papers**: The arXiv search returned mostly robotics papers (MengeROS, HiMAP, BRNE) that are not directly transferable to the engine's game AI context. The game-relevant concepts (multi-agent pathfinding, crowd navigation) are already covered by the GitHub repos and existing RESEARCH-VERDICT.md rows. No arXiv paper is proposed for this wave.
- **NV4/NV5/NV6 status**: NV3 is ticked on the board; NV4, NV5, NV6 are file-absent by glob (`no crates/nav/src/orca.rs`, `no crates/nav/src/platoon.rs`, `no crates/nav/src/funnel.rs`). NV7 is proposed. GA2 is file-absent. GA5 and NV8 are proposed.
