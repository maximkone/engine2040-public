# Steal 451 RB blast-check — occluded-blast + seeded-fragment shape for UW7/UC6

Goal (part + question): FOCUS Now pillar 4, Break the body, break the world —
how a deterministic game resolves a frag/flashbang/shell blast (pressure
falloff, wall occlusion, seeded fragments, impulses) with zero floats and a
byte-identical replay, and what transfers to our open UW7
(`crates/combat/src/device.rs`: arcs, fuses, occluded blast through
`SegmentCast`, seeded fragments, impulses for DX1) and UC6
(`crates/destruction/src/heavy.rs`: shells/rockets/charges driving UC4
breaks at runtime) without vendoring a byte.
Scope: `docs/research/` owned file only
(`steal-451-rb-blast.md` + LADDER lines); code read-only
(`crates/combat/src/projectile.rs`, board UW7/UC6 rows).
Proof (read live 2026-10-04/05, rev + license + paths below): repo record
(SPDX AGPL-3.0, master, 1481 stars), `src/brogue/` listing with
`Combat.c` blob SHA, code-search hits for the explosion code; engine
baseline via local `git grep`. No 403/429 on either host.
Stop: 10 searches + 7 reads (caps 10/12); no test run here; the spike
is proposed with numbers and a kill rule, not claimed.

Angles used (10 searches, narrow): (1-2) `tools/gh-search.mjs --type
repos` multi-word blast queries (empty vein: `--query` multi-word
returns `[]`, single-word works — wrapper quirk, no repeat); (3)
`tools/web-search.mjs` blast query (empty); (4) `gh-search.mjs bevy`
(host-alive check, HIT); (5) `gh-search.mjs explosion` (HIT vein:
confetti/UI toys only — rejects below); (6-7) `destruction rust` /
`destructible voxel rust` (empty, no repeat); (8) `gh-search.mjs
fragmentation` (HIT vein: Android-UI + netcode + landscape — all
wrong domain, rejects below); (9) `gh-search.mjs brogue` (HIT:
`tmewett/BrogueCE`, deterministic roguelike with grid explosions);
(10) `github_search_code` `explode repo:tmewett/BrogueCE` (HIT:
`src/brogue/Combat.c`, `Items.c`, `Globals.c`); (11) `spawnExplosion`
same repo (zero hits, ended the search step per contract).

## Engine baseline (local grep, 2026-10-04)

- Occlusion primitive EXISTS: `crates/combat/src/projectile.rs:232`
  (`pub trait SegmentCast`) + `NullCaster` (:250), `WallCaster`
  (:270), `ObstacleCaster` (:309) impls + `step_round` (:456),
  `step_volley` (:535) consumers. UW7's "occluded blast through
  `SegmentCast`" has its ray to stand on.
- Blast itself MISSING: `grep -i blast|explos|fragment|shockwave|
  overpressure` over combat/destruction/physics hits only
  `csg.rs:429,438` (brush-fragment centroid comments) and
  `solver.rs:599-602` / `verify.rs:64` ("no tunnel/explosion" guard
  strings). No `device.rs` in `crates/combat/src`, no `heavy.rs` in
  `crates/destruction/src`, no fuse/fragment/flashbang symbol
  anywhere. UW7 + UC6 are greenfield behind needs UW3/UW5 and
  UC4/UW7/DX5 respectively.
- Neighbour shapes: `hitsim.rs:1` (blade damage over many hits —
  the per-target damage ledger the blast writes into),
  `projectile.rs` volley stepping (the per-fragment step pattern),
  DX5 DONE (seeded Voronoi cook the heavy blast drives at runtime).

## CARD 1 — integer occluded-blast + seeded-fragment device model (UW7 detail, UC6 driver)

- Source:
  `PORT(tmewett/BrogueCE@104a20fedbe7d9360d2dfc8ff665571694212ad8:src/brogue/Combat.c)`
  (blob `d897430b14499453ecb987e441ca5f1bbeb4829e`), file existence
  + blob SHA read live 2026-10-04 via `get_file_contents`
  `src/brogue`; explosion code confirmed in `Combat.c` (+ `Items.c`,
  `Globals.c`) via live code search. Function body NOT read live —
  granularity stops at the file; the builder reads the detonation
  function at the pinned rev before porting any shape detail.
- License: **AGPL-3.0, idea-only** — repo-record `license.spdx_id`
  `AGPL-3.0` read live 2026-10-04 (`master`, 1481 stars, pushed
  2026-09-27); AGPL-3.0 is NOT on the `deny.toml` allow-list, and
  per FOCUS D5 Flax/Unreal/Unity/GPL code is ideas only. Zero bytes
  vendored, zero dependency edge, reimplementation from the
  engine-side design below.
- What it does — take, in own words (engine-side design, donor as
  prior art only): (a) fuse/arc state machine per device (throw arc
  from the projectile volley pattern, integer fuse ticks, bounce/
  stick resolved before detonation, never inside the tick's float
  path); (b) detonation resolves per target, not per cell: pressure
  `P = Q / (1 + k*r^2)` in integer Q16 with per-device Q/k tables
  (frag vs flashbang vs stinger vs breaching charge), one
  `SegmentCast` ray per target for occlusion (closed door = full
  block per the UW7 accept bar; partial cover scales, never adds);
  (c) fragments are seeded draws, not physics: Mott-count
  `N ~ M/Q` fragments over integer-Rng directions, each a
  `step_volley` ray with falloff, so the same seed gives the same
  wounds x2; (d) every detonation emits one impulse record for DX1
  and, for shells/charges, one energy event DX5's cook tables turn
  into cell counts (423-fracture shape); flashbang writes stun, not
  HP, through the same occluded path (exposed-only accept bar).
- Do not take: any donor byte (AGPL), float pressure math, per-cell
  flood-fill propagation (unconfirmed donor detail — spike decides
  raycast-vs-propagation on cost), donor item/char systems, any
  `Cargo.toml` change.
- Spike (proposed, not run — no builds per researcher contract):
  seeded room fixture (closed-door room + open-door room + corridor,
  6 agents): closed-door baseline (agents behind closed door take
  exactly 0) vs candidate; metrics per-agent P-Q16 + wound ledger
  bytes x2 + detonation step ns + closed-door-zero holds over 50
  seeded layouts; adverse input: detonation inside a wall pocket
  (all rays blocked — must cost <= open-air, never more).
- Kill: one HP behind a closed door, one ledger-byte drift x2,
  detonation step over its slice budget, blocked-pocket cost above
  open-air cost, one float in the device path, or any golden
  `d57b5c8ae3096614` drift — then the card closes REJECTED and
  UW7 waits for its needs (UW3/UW5) instead.
- Integration home: UW7 detail, no new row (UW7 open, needs UW3 +
  UW5; home `crates/combat/src/device.rs` beside the frozen
  `projectile.rs` casters; UC6 consumes the energy event in
  `crates/destruction/src/heavy.rs`). Pillar: FOCUS Now 4
  (body-break: wounds, impulses, runtime breaks). EMPIRE outcome:
  AI-builds-inside (an agent authors a device through typed verbs
  and proves it by replay: flashbang reaches every exposed agent
  and none behind a closed door, hashes x2).
- Blast-check verdict: no allow-list donor carries an isolable
  deterministic blast model (all HITs were UI toys, netcode, or
  AGPL) — build in-house on `SegmentCast`, donor idea-only.

## EVIDENCE row proposal (builder appends to `docs/plan/urban-war.md` §5)

| `tmewett/BrogueCE` | AGPL-3.0 idea-only (repo-record `license.spdx_id` AGPL-3.0 read live 2026-10-04, `master`, 1481 stars, pushed 2026-09-27; NOT on the `deny.toml` allow-list; zero bytes vendored) | deterministic grid-explosion prior art for UW7: per-target occluded pressure + seeded fragments reimplemented integer-side over `SegmentCast` (`PORT(tmewett/BrogueCE@104a20f:src/brogue/Combat.c)`, file + blob `d897430b` read live, function body UNVERIFIED-live) | any donor bytes, float math, per-cell propagation unless the spike wins it on cost |

## Rejects (no spike specified — no take possible)

- R1: `tyrantgit/ExplosionField`, `835127729/ViewExplosion`,
  `daniel-lundin/react-dom-confetti`, `VincentCATILLON/
  react-native-confetti-cannon`, `herrethan/react-confetti-explosion`
  (`gh-search.mjs explosion` vein): view/confetti particle toys —
  presentation only, no damage model, no occlusion, no determinism
  bar. Re-open: never for UW7 (VFX-only; a capture-side use belongs
  to a look row, not this one).
- R2: `YoKeyword/Fragmentation` (Android fragment manager),
  `ValveSoftware/GameNetworkingSockets` (UDP fragmentation),
  `sebbbi/OffsetAllocator` (memory fragmentation)
  (`fragmentation` vein): name-collision domain only; the netcode
  one serves ON rows, never blast. Re-open: never for UW7.
- R3: `ExplosionEngine/Explosion` (WIP cross-platform engine):
  no isolable blast shape found, license UNVERIFIED (record not
  read — step ended). Re-open: only with a pinned file + SPDX +
  an occlusion/fragment shape UW7 lacks.

## Triage

- Answer: one CARD (in-house integer blast on `SegmentCast` for
  UW7, driving UC6; BrogueCE idea-only), three rejects.
- Evidence: `crates/combat/src/projectile.rs:232,250,270,309`
  (SegmentCast + casters), `csg.rs:429,438` / `solver.rs:599-602`
  (only blast-adjacent strings in tree), absent `device.rs` /
  `heavy.rs`, board UW7/UC6 rows via `board-slice.mjs`, donor
  record + listing + code search live above.
- Existing owner: combat crate (`device.rs`, UW7) + destruction
  crate (`heavy.rs`, UC6).
- Missing coverage: the device model itself (greenfield).
- Smallest packet: EXPERIMENT spike running the closed-door
  fixture with the numbers above, then a CODE packet for
  `device.rs` only on ADOPT (behind UW3/UW5 needs).
- Acceptance: `cargo test -p engine-combat --lib device` green
  incl. closed-door-zero x2 + ledger bytes x2 + golden unchanged.
- Unknowns: raycast-vs-propagation cost (spike decides); exact
  Q16 Q/k tables per device (spike fits); donor function-level
  shape (builder reads at pinned rev, idea-only regardless).
