# Urban war

Freeze: FOCUS D7 holds game/content rows behind UW0; engine rows run
now after the FOCUS Now writers (owner 2026-09-30; `docs/FOCUS.md`).

Goal: a second, different game proves the engine is general.
Acceptance:
1. UW fixture on 3 seeds, replayed twice, empty diff.
2. UW39 packaged urban.exe: 3 missions solo with 4 AI and 5-player co-op; co-op replays exactly.
3. Engine rows UW1-UW26 and UC1-UC12 stay genre-neutral (GE ledger).
Order: UC1 UC12 UW0 UW2 UW9 UW10 UW11 UW12 UW14 UW18 UW19 UW20 UW22 UW23 UW24 UW40 UC2 UC4 UC11 UC13 UW3 UW4 UW6 UW13 UW15 UW21 UW25 UW27 UW28 UW41 UC3 UC5 UC8 UC9 UW5 UW16 UW29 UW30 UW36 UW42 UC10 UW7 UW8 UW17 UW31 UC6 UC14 UW26 UW32 UC7 UW33 UW34 UW35 UW37 UW38 UW39 UC15 UC16 UC19 UC17 UC18 UC20
Owner crates: arena, ai, combat, destruction, nav, world, endpoint.
Open board IDs: UC1 UC2 UC3 UC4 UC5 UC6 UC7 UC8 UC9 UC10 UC11 UC12 UC13 UC14 UC15 UC16 UC17 UC18 UC19 UC20 UW0 UW2 UW3 UW4 UW5 UW6 UW7 UW8 UW9 UW10 UW11 UW12 UW13 UW14 UW15 UW16 UW17 UW18 UW19 UW20 UW21 UW22 UW23 UW24 UW25 UW26 UW27 UW28 UW29 UW30 UW31 UW32 UW33 UW34 UW35 UW36 UW37 UW38 UW39 UW40 UW41 UW42
Pre-merge text: git show d96091cd:docs/plan/urban-war.md
Pre-merge text: git show d96091cd:docs/plan/urban-city.md (archived at docs/archive/2026-10-10/plans/urban-city.md)

Plain summary: the second game on engine2040 is a tactical close-quarters
shooter in the spirit of Ready or Not and SWAT 4: a five-officer team
breaches and clears buildings room by room under rules of engagement,
where arresting beats killing. Suspects surrender, fake surrender or
fight by morale; civilians panic; bullets pass through thin walls.
Rome stays the flagship and finishes Finish A first. Owner decision
2026-10-01: modern national era, fictional names, reference-look only,
CC0/PD kits through the provenance gate. The first slice is CQB in
single buildings; after it the game grows the Enlisted-style battle
layer (UW43–UW45 and UC21–UC26 are not on the board yet). Later tiers
grow it toward Arma-style squad war on large streamed maps.

Owner lanes: `physics`, `combat`, `anim`, `nav`, `brain`, `world`,
`destruction`, `audio`, `render`, `net`, `proof` for engine rows; `urban`
owns `games/urban/`; `content` owns `content/urban/`. Prefix: UW.
Base commit: `9de9b6a`.

## 1. Where it stands

The deterministic core, squads, destruction, online play and the AI
build pipeline exist; guns, doors, surrender logic, spot lights,
multi-storey navigation and modern content do not.

| Capability | Status |
|---|---|
| Deterministic tick, forks, replay | integrated (arena); golden `d57b5c8ae3096614` |
| Window, game exe, editor exe | integrated |
| ECS spine for a second game | not started (ARC1–ARC7 open) |
| Second-game boot receipt (RC34, runtime-owned) | open, blocked on UW0; companion to UW39 (game side) | board `docs/SPRINT-07-accepted-plan-rows.md:118-125`, runtime plan `docs/plan/runtime.md:270` |
| Urban game crate (UW27) | pilot only: `games/urban/src/boot.rs` headless corner boot lives on disk, but no Cargo package and not in workspace members | `games/urban/src/boot.rs:1-8`, `Cargo.toml:13`, board `docs/SPRINT-16-urban-war.md:159` |
| Slice evidence (UW39) | not started: no `crates/proof/src/uw.rs` (Glob 2026-10-14) | board `docs/SPRINT-16-urban-war.md:225` |
| Projectile flight (UW2) | partial: Fixed Q16 flight tables landed, no weapon emission or range-hash harness | `crates/combat/src/projectile.rs:15-56`, board `docs/SPRINT-16-urban-war.md:17` |
| Hearing falloff (UW14 start) | unit-tested integer `heard_by` over cm/tick/0..=1000 strength, no sight or record layer | `crates/ai/src/hearing.rs:64-72` |
| GP5 second-game fixture | superseded 2026-10-09, record only; not an urban boot | `docs/SECOND-GAME-CHECKLIST.md:1-6`, `fixtures/games/second-game/rules.json` |
| Projectile flight | integrated for siege engines (WD10) |
| Ray/segment queries | ticked (UW1); UW2 and UW14 consume it |
| Guns, less-lethal, devices, doors | missing |
| Morale | gladiator-only |
| Squads, cover queries | integrated (legion), unit-tested (EQS) |
| Door and reachability checks | unit-tested (`doorClear`, `navConnect`) |
| Navmesh | single layer; stacked floors average together |
| Destruction | integrated for siege walls (DX1–DX4, DX10) |
| Character controller | one capsule (PX8 open) |
| Animation | integrated (AM15 open) |
| Lights on the GPU | sun cascades, clustered point lights; no spot lights |
| Online | integrated (QUIC, relay, 64+500 soak) |
| AI builds inside | integrated headless (AB1–AB4, AB7, AB9) |
| Modern content | missing; every shipped asset is Roman (CT7–CT9) |

Layering (`9de9b6a`): `physics → anim → combat → input`,
`ai → physics, ecs, input, nn`, `destruction → core, nav`, `net →
arena`. Combat never imports physics/nav; second game reaches `net`
only after ARC5 moves rollback onto the spine.

## 2. Finish line

Scope tier (owner 2026-09-30, FOCUS.md): the second game is NOT
parked and keeps its board section; engine rows (UW1–UW26) run now,
game/content rows (UW27–UW39) wait for UW0. Not scope drift: Rome
first, Finish A first, no Finish A work here.

- **Tier A:** nothing; this plan adds no Finish A work.
- **Tier B, engine (now, UW1 ticked):** UW2–UW25 land; the headless CQB
  fixture (UW26) proves them together: five AI officers breach a
  generated two-floor house against four suspects and three civilians,
  arrest one, meet one fake surrender, clear every room; three seeds
  replay x2 with the engine diff empty.
- **Tier B, game (after UW0):** packaged `urban.exe` with three
  missions, solo with four AI officers and five-player co-op, ROE
  grading, planner and career; a co-op run replays exactly (UW39,
  Finish B's second-game receipt).
- **Tier C:** generated ruined cities and a 32-player battle, see
  `### city (UC)` below. Later, without rows: Arma-style squad war on a
  fictional 8×8 km region.

Bars: no shot without a perception record; clear only when every
walkable cell was seen; orders obeyed ≥ 99%; every order ≤ 3 presses;
0 stuck in 1,000 seeded clears.

### city (UC)

Tier C of the urban war game. NOT parked: content and the battle mode
wait for UW0; big maps (UC16–UC20) wait for the UW39 slice receipt.
The UC rows consume UW-owned rows (buildings and interiors are UW20 and
UW21; roads and layout are WD13). They never re-row them, and each
row's Needs column names what it waits on. Destruction core is DX-owned;
UC4 and UC6 compose it. Engine rows that also serve Rome's sieges and
district may dispatch after the urban-war engine rows.

## 3. Target design

Plain version: a shot, a shout and a door kick are all data. Rules
tables turn them into outcomes; AI brains read only what they could
perceive; presentation never feeds back.

```
input / orders / planner ─► UW4 weapon, UW7 device, UW10 door, shout
      │ rounds step one segment per tick (Fixed Q16)
      ▼
composer adapter ─► UW1 segment casts (sorted hits, material ids)
      ▼
UW3 penetration ─► UW5 wounds, UW8 status, UW6 suppression, UW22 breaks
      ▼ events (tick-stamped)
UW14 perception ─► UW15 compliance/morale ─► UW16 suspects / UW17 officers
      ▼
game: ROE judge, score, presentation (UW36), audio (UW23)
```

- **Ownership:** combat owns weapons, devices, flight, penetration,
  wounds, status and suppression as pure functions over versioned
  tables (HR3); physics owns colliders, queries, stances and doors; ai
  owns perception, compliance and tactics; world owns buildings,
  interiors and rosters. The composer (UW26) adapts physics to combat's
  `SegmentCast`, no engine edge.
- **Determinism:** Fixed Q16 and integer mm; seeded spread, spawns and
  hesitation; hits sorted by `(distance_mm, collider_id)`.
- **Budgets per tick:** 2,048 casts ≤ 1 ms; round step ≤ 0.3 ms; 32
  agents perceive ≤ 0.3 ms and decide ≤ 0.8 ms; zero steady-state
  allocations.

```rust
pub trait SegmentCast { // combat: caller supplies the world
    fn cast(&mut self, seg: SegmentMm, out: &mut HitBuf) -> Result<(), CastError>;
}
pub enum Response { Comply, FakeComply, Flee, Hide, Fight, TakeHostage }
pub enum SquadOrder { Move(PointMm), Hold, FallIn, Stack(DoorId),
    Mirror(DoorId), Wedge(DoorId), Breach { door: DoorId, how: BreachHow,
    then: Option<DeviceId> }, Restrain(AgentId), Collect(ItemId), Split(Team) }
```

### city (UC)

Plain version: a city is built in seven seeded stages; each stage is a
deterministic function of the seed plus tables, and each stage can be
previewed, checked and hand-polished in the editor. Layout, dressing
and ruin bakes are pure functions of seed and tables (bytes x2); runtime
breaks are server-authoritative events `(tick, impulse, seed)`.

```
seed + style tables
 1 layout     WD13 roads → blocks → lots (UC1)        footprints, storeys
 2 buildings  UW20 modules + facade variety (UC2)      shells, bonds
 3 interiors  UW21 for enterable, interior mapping (UC9) for the rest
 4 dressing   seeded scatter behind spatial checks (UC3)
 5 damage     offline shelling runs UC4 destruction → ruin states (UC5)
 6 cook       HLOD, meshlets, hulls, nav tiles, bond graphs (UC10)
 7 review     captures + spatial eval + review panel; editor polish
```

## 4. Hard problems

Plain version: the AI must earn every shot and every surrender, floors
must stop merging, and bullets must agree on all PCs.

| Problem | Technique | Why | Reference | Donor |
|---|---|---|---|---|
| Surrender that feels human (UW15) | morale 0..=1000 moved by events, archetype tables, seeded hesitation; binary gates checked before scoring so no surrender to no stimulus | integer state replays x2; designers tune tables, never code | Ready or Not utility-AI interview: archetypes, gates, commit interrupts (80.lv, 2026-05-23) | design only, reimplemented |
| Honest AI (UW14) | every shot cites a perception record; a verifier fails seeded shots without one; sight/hearing as integer factors with decay | no wall-hack AI; proofs stay fair; 32 agents ≤ 0.3 ms | §2 bar (no shot without a record); UW1 `QueryBvh` order | reimplemented on `crates/physics/src/query.rs`, `crates/ai/src/hearing.rs` |
| Flashlight in dark rooms (UW24) | shadowed spot lights in the clustered path with a shadow atlas; presentation only | golden hash never moves; dark interiors stay readable | internal GX2 gate | none |
| Layered spans (UW40–UW42) | span table beside the v1 path, `MESH_VERSION` 2 with v1 decode; stairs as ramps between layers | stacked floors stop averaging together; v1 blobs keep decoding | Recast compact-heightfield shape | `recastnavigation/recastnavigation@9f4ce64` (API listing 2026-10-14), shape reimplemented, no dependency |
| Deterministic penetration (UW3) | thickness × versioned material table; hits sorted by `(distance_mm, collider_id)`; unknown materials refuse | same bytes x2 on every PC; rifle through two drywalls, stopped by concrete | UW1 sorted-hit order | reimplemented |

## 5. Donors

Donors: `steals.md`; porting policy is AGENTS rule 4.

## 6. Slim

- Reuse, never fork: WD10 flight, legion LOD, EQS, `doorClear` /
  `navConnect`, bonded destruction, cells, AB brief chain, melee lag-comp
  ring, SV1 generations.
- One new package only: `games/urban` (second game proves public APIs).
- Rome consumers: UW2–UW4 archers/pila, UW10 gates, UW12/UW20
  multi-storey insulae, UW15 gladiator surrender.
- RoN-style first: hand-dressed maps + random rosters; UW21 houses double
  as "new house every run".
- Not in Tier B: vehicles, large terrain, campaign, mirror-gun view,
  shields, night vision, attachments, wind drift, collapsing floors.

## 7. Board tasks

Still-open rows only, read from the board (`node tools/board-slice.mjs
--ready --open`). Ticked rows are dropped (UW1 is ticked). Needs lists
the open rows each row waits on. Full Accept lines: `grep -h "^- \[ \]
<ID> " docs/SPRINT*.md`. Genre-neutral engine rows are flagged in Tier,
per Acceptance 3.

### UW rows (urban war)

| ID | Row | Tier | Needs |
|---|---|---|---|
| UW0 | Finish A gate | B | GM3 Q1 |
| UW2 | projectile flight | B, genre-neutral | none |
| UW3 | penetration and ricochet | B, genre-neutral | UW2 |
| UW4 | ranged weapon state | B, genre-neutral | UW2 |
| UW5 | hit zones, armor and wounds | B, genre-neutral | UW3 |
| UW6 | suppression | B, genre-neutral | UW2 |
| UW7 | devices | B, genre-neutral | UW3 UW5 |
| UW8 | status effects | B, genre-neutral | UW5 |
| UW9 | stances and weapon collision | B, genre-neutral | none |
| UW10 | doors | B, genre-neutral | none |
| UW11 | infantry layer | B, genre-neutral | AM15 |
| UW12 | layered navmesh | B, genre-neutral | none |
| UW13 | building links and room graph | B, genre-neutral | UW12 |
| UW14 | honest perception | B, genre-neutral | none |
| UW15 | compliance and morale | B, genre-neutral | UW14 |
| UW16 | armed-agent tactics | B, genre-neutral | UW6 UW14 |
| UW17 | team clearing and activity queue | B, genre-neutral | UW13 UW16 |
| UW18 | squad orders | B, genre-neutral | none |
| UW19 | spawn rosters | B, genre-neutral | none |
| UW20 | buildings from footprints | B, genre-neutral | WD13 |
| UW21 | interiors | B, genre-neutral | UW12 UW20 |
| UW22 | breach | B, genre-neutral | none |
| UW23 | gunfire and voice acoustics | B, genre-neutral | none |
| UW24 | shadowed spot lights | B, genre-neutral | none |
| UW25 | ranged lag compensation | B, genre-neutral | UW2 |
| UW26 | headless CQB fixture | B, genre-neutral | UW4 UW7 UW8 UW10 UW15 UW17 UW18 UW19 UW21 |
| UW27 | urban game crate | B | UW0 |
| UW28 | officers, suspects, civilians | B | UW0 UW11 |
| UW29 | weapons, tools and sounds | B | UW0 UW4 |
| UW30 | three mission maps | B | UW0 UW19 UW21 UW22 |
| UW31 | officer player | B | UW4 UW9 UW11 UW24 UW27 UW28 UW29 |
| UW32 | command menu | B | UW18 UW31 |
| UW33 | missions and ROE grading | B | UW15 UW17 UW26 UW30 UW31 |
| UW34 | pre-mission planner | B | UW32 UW33 |
| UW35 | career | B | UW33 |
| UW36 | impacts and look | B | UW3 UW24 UW27 |
| UW37 | brief to mission | B | UW33 |
| UW38 | co-op online | B | UW25 UW33 ARC5 |
| UW39 | slice evidence | B | UW34 UW35 UW36 UW37 UW38 GM3 |
| UW40 | layered span table + `MESH_VERSION` 2 with v1 decode | B | none |
| UW41 | layered bake core (held on UW40) | B | UW40 |
| UW42 | layered bake budget + tile-version integration (held on UW41) | B | UW41 |

### city (UC) rows

| ID | Row | Tier | Needs |
|---|---|---|---|
| UC1 | district styles and lots | C, genre-neutral | WD13 |
| UC2 | facade variety | C, genre-neutral | UC1 UW20 |
| UC3 | dressing scatter | C, genre-neutral | UC2 |
| UC4 | building support graph | C, genre-neutral | DX9 UW22 |
| UC5 | ruin bake | C, genre-neutral | UC4 |
| UC6 | heavy-weapon damage | C, genre-neutral | UC4 UW7 |
| UC7 | deterministic destruction sync | C, genre-neutral | UC6 ON7 |
| UC8 | surface damage | C, genre-neutral | UW3 |
| UC9 | interior mapping | C, genre-neutral | UC2 |
| UC10 | block HLOD | C, genre-neutral | VG2 UC5 |
| UC11 | smoke and dust volumes | C, genre-neutral | UW14 |
| UC12 | craters | C, genre-neutral | none |
| UC13 | urban kits | C | UW0 |
| UC14 | district benchmark | C | UC3 UC5 UC8 UC9 UC10 UC13 VG3 |
| UC15 | city battle mode | C | UW39 UC6 UC7 UC11 UC14 |
| UC16 | large terrain | C | UW39 |
| UC17 | vegetation and grass | C | UC16 VG1 |
| UC18 | roads on terrain | C | UC16 UC1 |
| UC19 | military vehicles | C | UW39 UW3 |
| UC20 | helicopter flight | C | UW39 UC16 |

### Row changes (2026-10-13, RC34 packet)

- RC34 stays runtime-owned (`docs/plan/runtime.md:270`, board
  `docs/SPRINT-07-accepted-plan-rows.md:118-125`); this plan adds no
  duplicate. UW39 stays the game-side receipt.
- No new UW/UC rows this packet: the boot path is already covered by
  UW0 → UW27 → UW39 plus RC34, and UW0 is WAITING on GM3/Q1
  (`docs/SPRINT-16-urban-war.md:10`). Slim means zero new rows until
  the gate opens.
- Order and Open board IDs above unchanged.
- CQB slice packet 2026-10-14 (PLN-urban-cqb-slice): no new rows. The
  slice is covered (UW2–UW26 plus the UW39 receipt); UW0 still WAITS on
  GM3/Q1 so game/content rows stay held; battle-layer UW43–UW45 and
  UC21–UC26 stay unboarded (out of slice). §1 evidence refreshed to
  code (boot.rs pilot, projectile partial, hearing unit-tested,
  proof/uw.rs still absent); §4 deepened from stub to table.
  Follow-up outside this packet's paths: add the Recast pin to
  `steals.md` with the commit quoted in §4.

## 8. Quality gates

- Every engine row: fixture hash x2; golden `d57b5c8ae3096614` unchanged;
  0 steady-state allocations; no float in hashed state.
- GE9 generality gate lists `fixtures/games/cqb` (UW26) and `games/urban`
  (UW27): engine diff empty.
- The UW14 honest-perception verifier and the UW17 seen-cells rule run in
  every proof after they land.
- Budgets above posted into PF1 workloads with kill bars.
- Every content file has an `EVIDENCE.md` row; before UW39, a one-hour
  co-op soak (P5 shape) with no crash, memory within 5%, no desync.

## 9. Session estimate

Session estimate: in the pre-merge text only; not kept here.
