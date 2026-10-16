# Steal 422 RB recast — Detour corridor-portal + string-pull funnel for NV6

Goal (part + question): FOCUS Now pillar 2, AI spatial understanding —
how Detour turns a poly corridor into a tightened straight path with
per-portal polygon references, and what transfers to our open NV6
(`crates/nav/src/funnel.rs`, portal funnel over the tiled mesh with
salted tile references so paths survive rebakes) without vendoring a
byte or touching the tick.
Scope: `docs/research/` owned file only
(`steal-422-rb-recast.md` + one LADDER line); code read-only
(`crates/nav/src/mesh.rs`, `crates/nav/src/tiles.rs`,
`docs/plan/game-ai.md:221-224` donors, SPRINT NV6).
Proof (read live 2026-10-05, rev + license + paths below): repo record
(SPDX Zlib, main, 7942 stars), HEAD SHA, `License.txt` Zlib text,
`DetourNavMeshQuery.cpp`/`.h` blob SHAs plus funnel fragments;
engine baseline via local Grep/Read. No 403/429 on either host.
Stop: 5 searches + 9 reads (caps 10/12); no test run here; the spike
is proposed with numbers and a kill rule, not claimed.

Angles used (5, narrow): (1) `tools/gh-search.mjs --type repos`
`recastnavigation recastnavigation` (HIT vein: forks/mirrors, known
repo confirmed, no second search); (2) MCP `search_repositories`
(same query, HIT: `recastnavigation/recastnavigation` record —
spdx_id Zlib, default branch main, 7942 stars, pushed 2026-02-27);
(3) MCP `search_code` `findStraightPath repo:...` (HIT:
`Detour/Source/DetourNavMeshQuery.cpp` blob `7edbe3b1`,
`Detour/Include/DetourNavMeshQuery.h` blob `dd8150c`,
both at ref `9f4ce64`, plus `DetourPathCorridor.cpp` as the
corridor consumer); (4) MCP `search_code` `portalApex repo:...`
(HIT: apex/left/right init + apex-advance restart in the same
`.cpp`); (5) MCP `search_code` `triarea2D portalLeft repo:...`
(HIT: `dtTriArea2D(portalApex, portalLeft, left)` signed-area
tightening with per-portal poly refs). Root listing read live
(`License.txt`, never guessed) + HEAD commit read live
(`9f4ce64458dfae86e1239c525ddc219c4e9e06f1`).

## Engine baseline (local grep/read, 2026-10-05)

- NV1 done: `crates/nav/src/mesh.rs:1-40` (`E2NAV1` bake, `MESH_VERSION`
  1, quantized sort + index-order flood fill, integer mm in bytes).
- NV3 done: `crates/nav/src/tiles.rs:10-11`
  (`PORT(recastnavigation/recastnavigation@main:DetourTileCache)` +
  `PORT(janhohenheim/rerecast@main:crates)`, EVIDENCE rows filed);
  dirty-tile rebake + flow fields + version-stamped walks landed.
- NV6 open, funnel missing: `grep funnel` over `crates/nav/src` is
  clean (no `funnel.rs`); SPRINT NV6 owns `crates/nav/src/funnel.rs`
  (portal funnel over the tiled mesh, salted refs survive rebakes;
  needs NV3 + DX1, both ticked). Bar: never longer than the grid
  path, >=10% shorter on >=80% of queries, p50 <= 0.2 ms, bytes x2.
- Donor table `docs/plan/game-ai.md:221-224`: recast covers NV1/NV3
  shapes, `vleue/polyanya` (Apache-2.0) covers the NV6 funnel math —
  the uncovered half is Detour's corridor-to-portal extraction plus
  straight-path refs/flags plus salted-ref survival across rebake,
  which polyanya (triangulation in, no tile versions) cannot supply.
  No dupe: this card owns only that half.

## CARD 1 — Detour corridor-portal + string-pull funnel with per-portal poly refs (NV6 detail)

- Source:
  `PORT(recastnavigation/recastnavigation@9f4ce64458dfae86e1239c525ddc219c4e9e06f1:Detour/Source/DetourNavMeshQuery.cpp)`
  (blob `7edbe3b1`) + `Detour/Include/DetourNavMeshQuery.h`
  (blob `dd8150c`), all read live 2026-10-05 via GitHub API.
- License: **Zlib** — repo-record `license.spdx_id` Zlib read live
  2026-10-05, plus `License.txt` raw text read live at the pinned
  rev (Mikko Mononen 2009 Zlib grant); on the `deny.toml`
  allow-list (`"Zlib"`). Reimplement shape only, zero bytes
  vendored, attribution in the file header.
- What it does — HOW THEY HANDLE it, in own words: (a) walk the
  corridor poly-by-poly, emitting one portal (left/right edge)
  per shared edge between adjacent polys
  (`getPortalPoints`-equivalent), start and end positions cap the
  ends; (b) run the apex funnel — `portalApex` starts at the start
  pos with `portalLeft == portalRight == apex`, each portal
  tightens one side only when the signed-area test
  (`dtTriArea2D(apex, side, candidate)`) says the candidate is
  strictly inside, and the apex jumps (restart) when a portal
  crosses the opposite side; (c) every emitted straight-path
  point carries its polygon reference and flags
  (`straightPathRefs`, `DT_STRAIGHTPATH_*`), so a later tile
  rebake invalidates exactly the stamped walks (our NV7
  version-stamp shape) instead of the whole path.
- Take: portal extraction from a poly corridor + apex/left/right
  tightening with signed-area tests + per-point poly refs and
  straight-path flags, reimplemented std-only in integer mm over
  our `tiles.rs` registry with salted `TileId` refs, no floats in
  the tick path.
- Do not take: C++ sources, float math, `DetourCrowd`,
  `DetourTileCache` proper (already PORTed for NV3), the demo app,
  any vendored file, any dependency edge.
- Spike (proposed, not run — no builds per researcher contract):
  seeded `TiledNav` corridor fixture (straight 5-poly run + one
  L-bend, built from the NV3 tiles fixtures): corridor-center
  baseline vs funnel straight path; metrics path length + query
  p50 + bake/query bytes x2; 50 seeded queries for the NV6 bar
  (never longer than grid path; >=10% shorter on >=80%; p50
  <= 0.2 ms; bytes identical x2); adverse input: rebake under a
  stamped walk, salted refs must repath with zero stale resolves.
- Kill: one longer-than-grid path, <80% of queries >=10% shorter,
  p50 over 0.2 ms, one stale-ref resolve after rebake, one float
  in the tick path, or any golden `d57b5c8ae3096614` drift —
  then the card closes REJECTED and the polyanya-only shape stands.
- Integration home: NV6 detail, no new row (NV6 open, needs NV3 +
  DX1 ticked; home `crates/nav/src/funnel.rs` beside the frozen
  tiles registry; consumers NV8 soak + SP36 touched-rebake).
  Pillar: FOCUS Now 2 (AI spatial understanding). EMPIRE outcome:
  AI-makes-games (smooth version-stamped paths are the movement
  leg under brief-to-slice and AI-driven play).

## EVIDENCE row proposal (builder appends to `docs/plan/game-ai.md` §5)

| `recastnavigation/recastnavigation` | Zlib (`https://raw.githubusercontent.com/recastnavigation/recastnavigation/9f4ce64458dfae86e1239c525ddc219c4e9e06f1/License.txt`, Zlib text read live 2026-10-05 at pinned rev, repo-record spdx_id Zlib, main, 7942 stars) | corridor-portal extraction + apex string-pull + per-point poly refs/flags for NV6, reimplemented std-only integer-mm over salted TileIds (`PORT(recastnavigation/recastnavigation@9f4ce64:Detour/Source/DetourNavMeshQuery.cpp)`, no bytes vendored) | C++ sources, float paths, DetourCrowd, vendored files |

## Triage

- Answer: one CARD (Detour funnel detail for NV6), zero rejects.
- Evidence: `crates/nav/src/tiles.rs:10-11` (prior PORTs),
  `crates/nav/src/mesh.rs:1-40` (NV1 bake), SPRINT NV6 row via
  `board-slice.mjs NV6`, `docs/plan/game-ai.md:221-224` (donor
  table gap), donor blobs/HEAD/License read live above.
- Existing owner: nav crate (`crates/nav/src/funnel.rs`, NV6).
- Missing coverage: the funnel itself (no file on disk).
- Smallest packet: EXPERIMENT spike running the corridor
  fixture with the NV6 numbers, then a CODE packet for
  `funnel.rs` only on ADOPT.
- Acceptance: `cargo test -p engine-nav --lib funnel` green
  incl. the 50-query bar + rebake-stale fixture + bytes x2.
- Unknowns: exact integer-mm epsilon for the signed-area test
  (spike decides); off-mesh connections out of scope.
