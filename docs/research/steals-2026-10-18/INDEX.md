# Steal wave 2026-10-18 — index of every card

Read-only (all donors whole: ideas only, zero bytes vendored). This folder holds
56 markdown files: 49 single-donor steal cards (blocks 1-10 of the wave and the
scout cards from the same day) and 7 wave digests (`00-combined.md` plus the six
hunter reports `01-render` .. `06-new-engines`). The table below has one row per
file, ranked by value to the duel (the best steal of each file; for a digest, its
highest-ranked row). Every card was read to fill it.

Licence column is the SPDX as the card recorded it (read live 2026-10-18).
`mixed` = a digest with several donors. `none` = no licence file in the donor
tree. `declared` = an SPDX in a manifest but no licence file on disk. Per AGENTS
rule 4 a donor may be ported from any source; the licence is recorded for the
record. Where a card says ideas only (GPL/AGPL/MPL/proprietary/custom), take the
shape, not the file.

## Top 10

1. **Streaming tick journal** — length-prefixed records, flush per tick, a
   truncated tail is a clean end, a damaged middle record fails hard.
   `ferrets.md` → `crates/proof/src/journal.rs` (LANDED 36e81785).
2. **Canonical float bytes** — collapse every NaN to one encoding at the write
   boundary, so x87/SSE/NEON agree on the digest. `lactose.md` →
   `crates/core/src/hash.rs`.
3. **Camera renders into a texture with a fractional viewport** — one draw list
   carries an optional target and a `0..=1` viewport. `fyrox.md` →
   `crates/render/src/present.rs`.
4. **Typed access is the schedule contract** — declared reads/writes derive the
   order and refuse a conflicting unordered pair. `verbum.md` →
   `crates/ecs/src/schedule.rs`.
5. **Sectioned world hash** — one digest per named section, then a fold over the
   ordered digests, format number inside the hash. `amoris.md` → `crates/proof`.
6. **Vertex-extrusion fighter outline** — offset each vertex along a cooked
   outline normal, one extra draw per fighter. `01-render.md` →
   `crates/render/src/outline.rs` + `crates/game/src/fighter_look.rs`.
7. **Diagnostics store** — bounded ring, O(1) running mean, time-aware EMA, NaN
   skipped. `04-core-tools.md` → `crates/perf` (LANDED 56a16c8c).
8. **Total tie-broken event comparator** — order/priority/speed/sub_order plus a
   monotonic creation counter as the last key. `dotzuki.md` → `crates/combat`.
9. **Determinism ladder per part** — every subsystem declares the strongest
   mode it supports; startup refuses by name. `RustingEngine.md` →
   `crates/arena/src/sync.rs` + `crates/proof`.
10. **Oracle-normalized agent score** — (mean − random) / (optimal − random),
    seeded bootstrap CI. `05-ai-native.md` → `crates/playtest` (LANDED 1cd4740f).

Also landed this wave (from `00-combined.md`): L1 declick fades 65b4fd5a, L4
strict-WebGPU switch 51e4b15c, L7 ack bitfield 0517bcdf, L9 anim event strategy
1c0ab6be, T2 backend-selection report 5e4d384f. Two hunter "0 hits" claims were
false-new: entity-to-pixel (`steel.md` #1) is EV21, ragdoll momentum handoff
(Fyrox) is `PhysFx::spawn_ragdoll`.

## Every card

| # | Card (file) | Repo | Licence (SPDX) | Best steal | Target |
|---|---|---|---|---|---|
| 1 | `ferrets.md` | valeriyr/ferrets | Apache-2.0 | Crash-tolerant streaming replay; truncated tail ends cleanly | `crates/proof/src/journal.rs` |
| 2 | `lactose.md` | MilkyEngineer/LACTOSE | MIT | Canonical NaN-collapsed float bytes before hash or save | `crates/core/src/hash.rs` |
| 3 | `fyrox.md` | FyroxEngine/Fyrox | MIT | Camera renders into a texture with a fractional viewport | `crates/render/src/present.rs` |
| 4 | `verbum.md` | scriptura/verbum | MIT OR Apache-2.0 (declared) | Typed access is the contract; order caught at compile time | `crates/ecs/src/schedule.rs` |
| 5 | `amoris.md` | qiulinfan/amoris | MIT | Sectioned world hash: digest per section, then fold | `crates/proof` + `crates/arena/src/sync.rs` |
| 6 | `01-render.md` | bevy_mod_outline + Veloren + kajiya (digest) | mixed | Vertex-extrusion outline: one extra draw per fighter | `crates/render/src/outline.rs` |
| 7 | `04-core-tools.md` | bevy + Fyrox + harness (digest) | mixed | Diagnostics store: ring, O(1) mean, time-aware EMA | `crates/perf` |
| 8 | `dotzuki.md` | liuyanghejerry/dotzuki | MIT OR Apache-2.0 | Total tie-broken comparator with monotonic creation counter | `crates/combat` + `crates/ecs` |
| 9 | `RustingEngine.md` | RustingStudio/RustingEngine | NOASSERTION (custom) | Determinism ladder declared per part, checked at startup | `crates/arena/src/sync.rs` + `crates/proof` |
| 10 | `05-ai-native.md` | agentick + ftl-bench + summer (digest) | mixed | Oracle-normalized agent score with seeded bootstrap CI | `crates/playtest/src/eval.rs` |
| 11 | `03-net-web-audio.md` | Firewheel + naia + Veloren (digest) | mixed | Reversible click-free declick fades; 32-bit ack bitfield | `crates/audio`, `crates/net` |
| 12 | `02-physics-anim.md` | tnua + Veloren + Fyrox (digest) | mixed | Jump shaping by extra gravities; five-tier poise ladder | `crates/physics/controller2.rs` |
| 13 | `AstraWeave.md` | lazyxeon/AstraWeave | MIT | N-run determinism traces naming first divergent tick and system | `crates/proof` + `tools/desync-diff.mjs` |
| 14 | `shards-engine.md` | larelgit/shards-engine | Apache-2.0 OR MIT | Per-call "what changed" list built from one digest pair | `crates/endpoint/src/receipts.rs` |
| 15 | `rl-engine.md` | rudehn/rl-engine | MIT OR Apache-2.0 | Symmetric shadowcasting into a caller-owned reused buffer | `crates/spatial` (isovist), `crates/ai` |
| 16 | `RoyaleSim.md` | RoyaleGym/RoyaleSim | MIT (API NOASSERTION) | Unit calibration exact under every candidate tick hypothesis | `crates/core/src/fixed.rs` |
| 17 | `oriera-engine.md` | OriEra-engine/engine | Apache-2.0 (notice, API NOASSERTION) | Frozen command vocabulary drained on a per-frame budget | `crates/provider`, `crates/ai` |
| 18 | `moba-engine.md` | JakeGT/moba-engine | MIT OR Apache-2.0 | Replay header pins content digest and sim-logic version | `crates/net/src/replay.rs` |
| 19 | `oteryn-game.md` | Oteryn/Oteryn-Game | MPL-2.0 | Named determinism profile and domain-separated state hash | `docs/HASH-LAW.md` + `crates/arena/src/sync.rs` |
| 20 | `daedalus.md` | sonan0721/Daedalus | Apache-2.0 OR MIT | Fixed clock with observable requested/executed/deferred debt | `crates/core/src/time.rs` |
| 21 | `ByroRedux.md` | matiaszanolli/ByroRedux | MIT | Crash-safe write: temp, fsync, readback, rename, dir fsync | `crates/assets/src/gen_intake.rs`, `cooker.rs` |
| 22 | `pfx.md` | gmrdad82/pfx | MIT | Versioned weights-blob container with named sorted tensors | `crates/nn` (`.e2nn`) |
| 23 | `all-is-cubes.md` | kpreid/all-is-cubes | MIT OR Apache-2.0 | Golden-image record: expected/actual/diff plus histogram | `crates/proof/src/matrix.rs` |
| 24 | `openXplane.md` | OpenXPlane/openXplane | GPL-3.0 | Oracle-run bit-exact vectors; seed and case count asserted | `crates/proof` + `tools/*-vectors.mjs` |
| 25 | `cougr.md` | salazarsebas/Cougr | MIT | System order as one declarative graph with a cycle error | `crates/ecs/src/schedule.rs` |
| 26 | `mid-engine.md` | Mid-D-Man/mid-engine | none | Declared read/write access sets with an exact conflict check | `crates/ecs/src/schedule.rs` |
| 27 | `byte-engine.md` | Game-Tek/Byte-Engine | MIT | One thread-confined GPU compute worker for offline bakes | `crates/render/src/uploads.rs`, assets cook |
| 28 | `phase.md` | phase-rs/phase | MIT OR Apache-2.0 | Paired-seed AI gate with a committed per-bout baseline | `crates/playtest/src/tournament.rs` |
| 29 | `GoudEngine.md` | aram-devdocs/GoudEngine | none | Layer lint: scan imports, fail on an upward edge | `tools/layers.mjs` |
| 30 | `horizon.md` | ludovic111/horizon | MIT | Dependency-tracked runner with content-keyed shared passes | `tools/affected.mjs` |
| 31 | `aethereal.md` | iamacoffeepot/aethereal | MIT OR Apache-2.0 | Canonical byte layout with a hostile-count decode budget | `crates/net/src/delta.rs`, `crates/save` |
| 32 | `unhuman-rs.md` | unhuman-engine/unhuman-rs | Apache-2.0 | Toolchain version in the shader cache key | `crates/render/src/cache.rs` |
| 33 | `eaw-remake.md` | msch128/eaw-remake | MIT | Cull by projected pixel size, not by distance | `crates/render/src/draw.rs` |
| 34 | `mittens.md` | rei-kurzweil/mittens | MIT | Planar mirror as one component plus one system | `crates/render/src/present/gpu_present/` |
| 35 | `proof-engine.md` | Mattbusel/proof-engine | MIT | Marching tetrahedra isosurface: closed mesh, exact normals | `crates/render` (particles/vfx) |
| 36 | `CardGameEngine.md` | EMachad0/CardGameEngine | none | One viewer-view function; observing changes nothing | `crates/net/src/interest.rs` |
| 37 | `joy-engine.md` | trentpolack/joy-engine | MIT | Schema-driven authoring; unknown fields round-trip untouched | `crates/editor` |
| 38 | `spookery.md` | legend-of-alan/spookery | AGPL-3.0-or-later (docs CC-BY-4.0) | Guarantees page where every promise names its test | `docs` + `tools/workspace-check.mjs` |
| 39 | `kodanu.md` | raaiden666/kodanu | MIT OR Apache-2.0 (declared) | Full stage ladder: fixed stages run N times per frame | `crates/ecs/src/schedule.rs` |
| 40 | `peridot.md` | Pctg-x8/peridot | MIT | Fence reactor thread wakes parked tasks instead of polling | `crates/render/src/accel.rs` |
| 41 | `dust.md` | dust-engine/dust | MPL-2.0 | Closest-depth HZB rebuilt per frame from the depth buffer | `crates/render` (occlusion) |
| 42 | `pomme-client.md` | PommeMC/Client | GPL-3.0 | Named allocations with a per-subsystem VRAM report | `crates/render` + `crates/perf` |
| 43 | `vcod.md` | janost/vcod | GPL-3.0-or-later | Frozen static Huffman tree over the delta alphabet | `crates/net/src/delta.rs` |
| 44 | `project-okran.md` | brayniac6-glitch/Project-Okran | MIT | Regressive best-first task planner over task definitions | `crates/ai` (planner) |
| 45 | `pulsar-native.md` | Far-Beyond-Pulsar/Pulsar-Native | MIT | Structural verify pass before the VM may run a module | `crates/rules`, `crates/nn` load |
| 46 | `RustyZoo.md` | RustyZoo/RustyZoo | GPL-3.0 | Replayable wall-clock seconds source in a seeded stream | `crates/proof/src/tape.rs` |
| 47 | `lp-engine.md` | gage-marshall/lp-engine | MIT | Recorded host-call log replayed against the port, frame by frame | `crates/proof/src/tape.rs` |
| 48 | `vkrgss-rs.md` | xuyanwen2012/vkrgss-rs | MIT (API NOASSERTION) | Frame timing: a skipped stage is None, CPU/device split | `crates/perf/src/profiler.rs` |
| 49 | `8b-is-engine.md` | peterlodri-sec/8b-is-engine | EOS-CLA v1.0 (NOASSERTION) | Ternary 1.58-bit weight lane, 4 trits per byte | `crates/nn` |
| 50 | `aeon-engine.md` | aethelisdev/aeon-engine | MPL-2.0 | Frame telemetry as plain structs: 1% lows, stutter, VRAM | `crates/perf/src/profiler.rs` |
| 51 | `WaterDropEngine.md` | explorablescience/WaterDropEngine | GPL-3.0 | Fixed-capacity gizmo line buffer, dropped past the cap | `crates/render/src/debug_draw.rs` |
| 52 | `steel.md` | SSSxCCC/steel | MIT | Entity id per primitive; picking is one pixel read | editor viewport (**EV21 already**) |
| 53 | `openOMSI.md` | openOMSI-org/openOMSI | AGPL-3.0 | Start guard so a broken release does not strand the player | `crates/game/src/crash.rs` |
| 54 | `shiloh.md` | Asterber/shiloh | MIT OR Apache-2.0 | Headless frame dump on a game-time cadence | arena capture tooling |
| 55 | `06-new-engines.md` | webgpu-worker + minecraft-2 + pfx (digest) | mixed | wgpu loop in a Web Worker on OffscreenCanvas | `crates/web/src/shell.rs` |
| 56 | `00-combined.md` | the wave's combined ranking | mixed | Combined landing table; L1-L9 landed this wave | `docs/research` |

## Read-depth notes

- Every single-donor card states the donor sha/head, the licence read live, the
  files read, and a "not read" list. Digests state per-row read depth.
- Not worth stealing (card verdicts): `shiloh.md` (2D storybook, 3 transferable
  harness bits only), `dust.md`/`WaterDropEngine.md`/`pomme-client.md`/
  `vocd.md`/`RustyZoo.md`/`openXplane.md` (GPL/MPL: shape only), `mid-engine.md`/
  `GoudEngine.md`/`CardGameEngine.md`/`8b-is-engine.md` (no licence or a custom
  CLA: ideas and numbers only).
- Parked or owner-call (change fight feel or every golden): jump shaping,
  five-tier poise, flank precision, crush threshold, AgX, chord clash — see
  `00-combined.md` "Parked" and `02-physics-anim.md`.

## LADDER size caps

`docs/research/LADDER.md` size caps (LADDER.test.mjs): under 399 lines and under
429,783 bytes. Before this wave's rungs: 245 lines / 205,390 bytes (E76 report
and a live `Get-Item`/`Get-Content` read agree). After adding the wave rungs
(see the LADDER file) the size stays under both caps; the new line/byte counts
are recorded in the commit and the LADDER file has room for later waves.
