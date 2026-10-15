### Urban war (owner 2026-09-26, Finish B second game; `docs/plan/urban-war.md`)

The second game: a Ready or Not / SWAT 4-style tactical CQB shooter in a
fictional modern city, growing into dense ruined cities with structural
destruction. Engine rows (UW1–UW26, UC1–UC12) are genre-neutral and
dispatch like 2040-tier rows, after the FOCUS "Now" writers; game and
content rows wait for UW0 (Finish A met). Combat never imports physics
(the composer adapts `SegmentCast`); no new engine dependency edges.

- [ ] UW0 ops: Finish A gate — the game starts after Finish A (owner
  2026-09-26); the orchestrator ticks it only when `node tools/finish.mjs`
  prints A1–A10 met with evidence lines. Tier B. Size S. Owns the board
  tick. Accept: the receipt quotes that output. needs: GM3, Q1, EV7, EV8.
  WAITING row (deep-dive 2026-10-01: never dispatch writers here; the
  orchestrator rechecks each wave after GM3/Q1 land — 10 mis-scheduled
  helper runs proved only that no lane code exists).
- [ ] UW2 combat: projectile flight — any round (bullet, beanbag,
  pepperball, arrow) as Fixed Q16 state stepped one segment per tick
  with gravity and drag tables through the `SegmentCast` trait; spread
  seeded per (tick, shooter, shot). Tier B. Size M. Owns
  `crates/combat/src/projectile.rs`. Accept: 1,000-shot range hashes
  x2; drop within 2% of the table curve; 2,048 rounds ≤ 0.3 ms. needs:
  HR3.
- [ ] UW3 combat: penetration and ricochet — thickness × versioned
  material table (drywall, wood, brick, concrete, metal, glass, flesh)
  gives exit energy; shallow hits ricochet; low energy stops. Tier B.
  Size M. Owns `crates/combat/src/penetrate.rs`. Accept: a rifle round
  passes two drywall sheets and stops in 20 cm concrete; replays x2;
  unknown materials refuse. needs: UW1, UW2.
- [ ] UW4 combat: ranged weapon state — fire modes, rate in ticks,
  magazine and chamber, reload, jam, recoil plus seeded spread, sights,
  sway and weapon length, table-driven, emitting UW2 rounds. Tier B.
  Size M. Owns `crates/combat/src/firearm.rs`. Accept: 600-tick tape
  hashes x2; rate exact to the tick; empty never fires. needs: UW2.
- [ ] UW5 combat: hit zones, armor and wounds — zones per hitbox, armor
  tiers, wound states (light, heavy, incapacitated, dead), bleed-out and
  restrained states. Tier B. Size S. Owns `crates/combat/src/wounds.rs`.
  Accept: zone × armor × ammo matrix replays x2. needs: UW3.
- [ ] UW6 combat: suppression — near misses raise an integer meter with
  per-tick decay; pinned and shaken states out; never changes damage.
  Tier B. Size S. Owns `crates/combat/src/suppression.rs`. Accept: a
  burst at 1 m pins, at 10 m shakes; replays x2. needs: UW2.
- [ ] UW7 combat: devices — frag, flashbang, stinger, CS gas and
  breaching charge: arcs through UW2, fuses, occluded blast through
  `SegmentCast`, seeded fragments, door traps, impulses for DX1. Tier B.
  Size M. Owns `crates/combat/src/device.rs`. Accept: a flashbang
  reaches every exposed agent and none behind a closed door; hashes x2.
  needs: UW3, UW5.
- [ ] UW8 combat: status effects — taser stun, flash blind and deafen by
  sight line and distance, pepper and CS exposure from integer gas
  diffusion over a caller-passed room graph, beanbag stagger; masks and
  armor resist by table. Tier B. Size M. Owns
  `crates/combat/src/status.rs`. Accept: gas spreads only through open
  doors; durations exact; replays x2; ≤ 0.1 ms per tick. needs: UW5.
- [ ] UW9 physics: stances and weapon collision — stand, crouch, prone
  capsules, ceiling-blocked changes, lean checked against walls, the
  weapon raised when its length meets geometry. Tier B. Size M. Owns
  `crates/physics/src/stance.rs`. Accept: 1,000 seeded rooms with no
  clipping; hashes x2. needs: PX8.
- [ ] UW10 physics: doors — hinged doors with integer angles and states
  (closed, ajar, open, locked, wedged, trapped, broken); push, peek,
  kick, ram, shotgun and charge outcomes by table; state events feed
  nav links and audio. Tier B. Size M. Owns `crates/physics/src/door.rs`.
  Accept: every state × action fixture replays x2; a wedged door holds
  against a push; broken doors stop blocking casts. needs: UW1.
- [ ] UW11 anim: infantry layer — stance transitions, weapon upper body
  over AM15, aim through AM9 IK, reload, throw, cuff, hands-up,
  kneel-and-surrender, cower, and a true first-person view from the same
  body. Tier B. Size L. Owns `crates/anim/src/infantry.rs`. Accept: no
  pops across interrupts; the first-person camera never clips the body;
  0 allocations per frame. needs: AM15, AM9.
- [ ] UW12 nav: layered navmesh — several walkable spans per column,
  stairs as ramps between layers, `MESH_VERSION` 2 with v1 decoding.
  Tier B. Size L. Owns `crates/nav/src/layers.rs` plus `mesh.rs` version
  hunks. Accept: a 4-floor building routes ground to roof, bytes x2;
  stacked floors never merge; bake ≤ 2 s for 20 buildings. needs: NV1,
  NV7.
- [ ] UW13 nav: building links and room graph — doors, windows, ladders
  and holes as typed links in the NV2 registry that follow UW10 states;
  a room graph per building as plain data; new holes from DX10 breaks.
  Tier B. Size M. Owns `crates/nav/src/building.rs`. Accept: a locked
  door refuses routes; a breach adds one link and repaths x2. needs:
  UW12, NV2, DX10.
- [ ] UW14 brain: honest perception — sight through UW1 with light and
  cover factors, hearing (shots, steps, doors, shouts) with falloff,
  last-known positions with decay, squad sharing; every target choice
  cites its record. Tier B. Size M. Owns `crates/ai/src/perceive.rs`.
  Accept: a verifier fails a seeded shot without a record; replays x2;
  32 agents ≤ 0.3 ms. needs: UW1.
- [ ] UW15 brain: compliance and morale — morale 0..=1000 moved by
  events; responses comply, fake comply, flee, hide, fight, take
  hostage by archetype table with seeded hesitation; civilian fear
  (cower, comply, flee, freeze). Tier B. Size M. Owns
  `crates/ai/src/compliance.rs`. Accept: a 500-case event matrix
  replays x2; cornered-and-shouted suspects comply above the table
  floor; no response reads presentation. needs: UW14.
- [ ] UW16 brain: armed-agent tactics — cover, return fire, barricade,
  ambush at doors, flank, retreat to caches, shoot through walls only at
  heard targets. Tier B. Size L. Owns `crates/ai/src/tactics.rs`.
  Accept: 200 seeded fights, 0 stuck, replays x2; every shot passes
  the UW14 verifier. needs: GA2, GA6, UW6, UW14.
- [ ] UW17 brain: team clearing and activity queue — officers stack,
  breach, clear by sectors, cover, restrain, collect and report from a
  queue; a room turns clear only when every walkable cell was seen.
  Tier B. Size L. Owns `crates/ai/src/clear.rs`. Accept: 1,000 seeded
  clears, 0 stuck, 0 unseen cells at clear, 0 friendly hits, x2. needs:
  UW13, UW16.
- [ ] UW18 brain: squad orders — move, hold, fall in, stack, mirror,
  wedge, breach (open, kick, ram, shotgun, charge; then bang, stinger,
  gas or none), restrain, collect, split red/blue, as validated
  recorded actions. Tier B. Size S. Owns `crates/ai/src/orders.rs`.
  Accept: order tapes replay x2; bad orders refuse typed; legion
  goldens unchanged. needs: GA2.
- [ ] UW19 world: spawn rosters — seeded rosters over tagged spawn
  points (min/max per group, archetype, weapon, trap, evidence,
  objective); same seed, same placement. Tier B. Size S. Owns
  `crates/world/src/roster.rs`. Accept: 100 seeds place legally x2; an
  impossible roster refuses naming the group. needs: WD1.
- [ ] UW20 world: buildings from footprints — footprints plus levels
  become modular buildings from a kit table with collision, nav input
  and DX1 bond data out. Tier B. Size M. Owns
  `crates/world/src/buildings.rs`. Accept: 20 footprints build identical
  bytes x2; ≤ 50 ms per building. needs: WD13.
- [ ] UW21 world: interiors — rooms, corridors, stairs, doors and
  windows per building with the room graph UW13 consumes; the "new
  house every run" generator. Tier B. Size L. Owns
  `crates/world/src/interiors.rs`. Accept: 100 seeded houses, every
  room reachable, stairs join every floor, x2; `doorClear` and
  `navConnect` green. needs: UW12, UW20.
- [ ] UW22 destruction: breach — door panels and thin walls as bonded
  collections; ram, shotgun and charge break sets name their dirty
  tiles (DX10); debris in DX4 budgets. Tier B. Size M. Owns
  `crates/destruction/src/breach.rs`. Accept: a charge breaks the same
  chunks x2; an agent routes through after rebake. needs: DX1, DX3,
  DX4, DX10.
- [ ] UW23 audio: gunfire and voice acoustics — distance filter and
  delay, supersonic crack, room and street reverb on SN1 zones, SN2
  occlusion through walls and doors, shouts heard through rooms;
  presentation only. Tier B. Size M. Owns `crates/audio/src/gunfire.rs`.
  Accept: blind listening fixture passes the audio judge; golden
  unchanged; 32 reports ≤ 0.2 ms. needs: SN1, SN2.
- [ ] UW24 render: shadowed spot lights — flashlights and lamps in the
  clustered path with a shadow atlas; presentation only. Tier B. Size M.
  Owns `crates/render/src/spot.rs`. Accept: 16 shadowed spots in a dark
  room within the P6 GPU budget (reference GPU); llvmpipe parity with a
  CPU reference; GX6 gate submits live. needs: GX2.
- [ ] UW25 net: ranged lag compensation — rewind hitboxes to the
  shooter's view tick inside a bounded window, replicate rounds and
  impacts under ON3. Tier B. Size M. Owns ranged hunks in
  `crates/net/src/lagcomp.rs` plus `crates/net/src/shots.rs`. Accept:
  ≥ 98% of shooter-seen hits register at 150 ms RTT and 2% loss;
  over-window rewinds refuse. needs: UW2, MP2, ON3.
- [ ] UW26 proof: headless CQB fixture — `fixtures/games/cqb/`: five AI
  officers breach a generated two-floor house against four suspects and
  three civilians, one arrest, one fake surrender, every room cleared;
  public APIs only. Tier B. Size M. Owns `fixtures/games/cqb/` plus its
  harness. Accept: 3 seeds x2; engine diff empty; in the GE9 gate.
  needs: UW4, UW7, UW8, UW10, UW15, UW17, UW18, UW19, UW21.
- [ ] UW27 ops: urban game crate — `games/urban` (package `urban-war`)
  on the ECS spine, public engine crates only (never `engine-arena` or
  `engine-game`); the orchestrator lands the root `Cargo.toml` edit.
  Tier B. Size S. Owns the `games/urban/` skeleton. Accept: `cargo check
  -p urban-war` green; no arena edge in `cargo tree`. needs: UW0, ARC2.
- [ ] UW28 content: officers, suspects, civilians — five officer
  variants with armor tiers, armed and unarmed suspects, civilians, and
  clips (move, stances, aim, reload, cuff, surrender, cower, hit, death)
  cooked by CT1; provenance rows. Tier B. Size L. Owns
  `content/urban/people/`. Accept: all animate in the window; every
  file provenanced. needs: UW0, UW11, CT1.
- [ ] UW29 content: weapons, tools and sounds — rifle, SMG, breaching
  shotgun, pistol, taser, beanbag, pepperball, pepper spray, flashbang,
  stinger, CS gas, charge, wedge, ram, with UW4 tables and recorded
  sounds. Tier B. Size L. Owns `content/urban/gear/`. Accept: every item
  works in the window with its own sounds; provenanced. needs: UW0, UW4,
  CT2.
- [ ] UW30 content: three mission maps — a two-floor house, a corner
  store with flats above, a motel floor; built on UW20/UW21 and dressed
  in the editor; spawn points tagged for UW19; dark interiors. Tier B.
  Size L. Owns `content/urban/maps/`. Accept: each loads, bakes nav,
  passes spatial checks and replays; provenanced. needs: UW0, UW19,
  UW21, UW22.
- [ ] UW31 game: officer player — camera at the eyes and rounds from the
  muzzle, stances, lean, weapon collision, flashlight, shout key, cuff,
  collect, minimal HUD option, remappable input. Tier B. Size L. Owns
  `games/urban/src/player.rs`. Accept: input to first shot p95 ≤ 50 ms;
  player tape replays x2; review panel scores gun feel. needs: UW4, UW9,
  UW11, UW24, UW27, UW28, UW29.
- [ ] UW32 game: command menu — SWAT 4-style context menu on the door or
  agent under the crosshair, red/blue teams, every UW18 order in ≤ 3
  presses on pad and keyboard; orders recorded. Tier B. Size M. Owns
  `games/urban/src/command.rs`. Accept: menu fixture reaches every
  order; officers obey in ≥ 99% of fixture runs. needs: UW18, UW31.
- [ ] UW33 game: missions and ROE grading — objectives (arrest,
  rescue, collect evidence, report), fail states (officer down, civilian
  killed, suspect escaped), an ROE judge that clears force only when the
  target was armed and threatening in the perception trace, score and
  grade, a short mode. Tier B. Size L. Owns `games/urban/src/mission.rs`.
  Accept: 50 seeded runs per map, 0 stuck; seeded unjustified shots
  graded down; runs replay exactly. needs: UW15, UW17, UW26, UW30, UW31.
- [ ] UW34 game: pre-mission planner — map view, entry points, drawn
  routes per team and loadouts, compiled into recorded orders. Tier B.
  Size M. Owns `games/urban/src/planner.rs`. Accept: a drawn plan runs
  hands-off with AI officers and replays x2. needs: UW32, UW33.
- [ ] UW35 game: career — officers persist with injuries, stress and
  permadeath option, versioned saves. Tier B. Size M. Owns
  `games/urban/src/career.rs`. Accept: save migration fixture passes; a
  killed officer stays gone. needs: UW33.
- [ ] UW36 game: impacts and look — muzzle-flash lights, bullet holes
  (GX10), dust and splinters by material (GX9), casings, gas clouds,
  blood decals with a gore toggle; presentation only. Tier B. Size M.
  Owns `games/urban/src/impacts.rs`. Accept: golden unchanged with it
  on, x2; 200 impacts per second in the P6 GPU budget (reference GPU).
  needs: UW3, UW24, UW27, GX9, GX10.
- [ ] UW37 game: brief to mission — an AB stage over the public AB7
  envelope: "barricaded suspect, two hostages, two-floor house, night"
  becomes a UW21 house, a UW19 roster and objectives, checked by fork
  play-tests. Tier B. Size M. Owns `games/urban/src/brief.rs`. Accept: 8
  of 10 seeded briefs yield playable missions; AB9 replay x2. needs:
  UW33, AB4.
- [ ] UW38 game: co-op online — five players on a dedicated server by
  invite code, AI fills empty officer slots, reconnect with AI takeover,
  on spine rollback. Tier B. Size M. Owns `games/urban/src/online.rs`.
  Accept: 5 clients under the ON4 fault model finish a mission and
  converge; replays exactly. needs: UW25, UW33, ARC5, ON2.
- [ ] UW39 proof: slice evidence — Finish B's second-game receipt:
  packaged `urban.exe` from a clean folder, three missions solo and
  co-op, captures on the reference GPU, review-panel scores, engine diff
  empty. Tier B. Size M. Owns `crates/proof/src/uw.rs`. Accept: every
  item has an evidence line. needs: UW34, UW35, UW36, UW37, UW38, GM3.
- [ ] UW40 nav: layered span table + `MESH_VERSION` 2 with v1 decode —
  bake records several walkable spans per column (Recast
  compact-heightfield shape, reimplemented; no new dependency) beside
  the v1 single-height path; v2 bytes canonical x2; every v1 blob still
  decodes to the same query answers. Tier B. Size S. Owns version hunks
  in `crates/nav/src/mesh.rs` (no new file). Accept: `cargo test -p
  engine-nav --lib mesh` green incl. v1-decode and v2-bytes-x2
  fixtures; a stacked two-floor fixture bakes two spans in one column
  where v1 averaged one; golden unchanged x2. needs: NV1, NV7.
  (PLN urban-war.md, proposed 2026-09-30; splits UW12 L.)
- [ ] UW41 nav: layered bake core (held on UW40) — spans become
  per-layer polygons with stairs as ramps in new `layers.rs` over the
  UW40 table; stacked floors never merge; a 4-floor building routes
  ground to roof with bytes x2. Tier B. Size M. Owns new
  `crates/nav/src/layers.rs` (mesh.rs API frozen). Accept: `cargo test
  -p engine-nav --lib layers` green; 4-floor route ground to roof bytes
  x2; single-floor bakes keep the v1 query answers; golden unchanged x2.
  needs: UW40. (PLN urban-war.md, proposed 2026-09-30.)
- [ ] UW42 proof: layered bake budget + tile-version integration (held
  on UW41) — 20-building bake ≤ 2 s on the reference CPU and NV7
  dirty-tile bump plus stale-path repath exact on the layered mesh, in
  a new fixture. Tier B. Size S. Owns new `crates/nav/tests/layers.rs`
  (no src change). Accept: `cargo test -p engine-nav --test layers`
  green with the budget number printed and bytes x2; golden unchanged
  x2. needs: UW41. (PLN urban-war.md, proposed 2026-09-30.)
- [ ] UC1 world: district styles and lots — style tables (old city,
  dense blocks, modern, industrial) turn WD13 blocks into lots with
  footprint, storeys, style and a damage-field value. Tier C. Size M.
  Owns `crates/world/src/lots.rs`. Accept: a 1×1 km seed yields ≥ 400
  legal lots, bytes x2, ≤ 5 s. needs: WD13.
- [ ] UC2 world: facade variety — seeded module picks per building:
  window types, balconies, shutters, awnings, AC units, water tanks,
  weathering masks, from style tables. Tier C. Size M. Owns
  `crates/world/src/facade.rs`. Accept: 400 buildings with no two
  adjacent facades identical; bytes x2. needs: UC1, UW20.
- [ ] UC3 world: dressing scatter — seeded scatter of street and roof
  clutter (cars, cables, laundry, generators, barrels, signs, rubble)
  by density rules, every placement through the SP1 staged gate (no
  overlap, no floating, no blocked door, `navConnect` green). Tier C.
  Size M. Owns `crates/world/src/dressing.rs`. Accept: 10k placements
  with 0 spatial findings; bytes x2. needs: UC2, SP1, SP5.
- [ ] UC4 destruction: building support graph — whole buildings as
  bonded collections per storey (walls, floors, columns) linked by
  support paths to the ground; unsupported sets fall in capped batches
  over ticks. Tier C. Size L. Owns `crates/destruction/src/structure.rs`.
  Accept: removing a ground column drops exactly the unsupported storeys
  x2; a 6-storey collapse holds DX4 caps per tick; nav dirty tiles
  named. needs: DX1, DX3, DX9, UW22.
- [ ] UC5 destruction: ruin bake — offline seeded shelling over the UC1
  damage field runs UC4, settles debris, and freezes ruin states 0–3
  with rubble meshes, collision and nav input. Tier C. Size L. Owns
  `crates/destruction/src/ruin.rs`. Accept: same seed, same ruin bytes
  x2; every ruin passes `navConnect`; rubble stays live-breakable.
  needs: UC4, DX4.
- [ ] UC6 destruction: heavy-weapon damage — shells, rockets and charges
  drive UC4 breaks at runtime with UW7 blasts; rubble becomes cover and
  nav through DX10. Tier C. Size M. Owns
  `crates/destruction/src/heavy.rs`. Accept: a scripted barrage breaks
  the same chunks x2; AI takes cover behind new rubble after rebake.
  needs: UC4, UW7, DX5.
- [ ] UC7 net: deterministic destruction sync — servers send break
  events, clients rebuild identical chunks; a per-cell settled-debris
  checksum every 60 ticks, snapshot repair on mismatch. Tier C. Size M.
  Owns `crates/net/src/break_sync.rs`. Accept: 32 clients converge
  after a 100-break barrage under ON4 faults; events ≤ 1 KB each; a
  seeded drift repairs. needs: UC6, ON7.
- [ ] UC8 render: surface damage — bullet holes, chips and scorch per
  material as GX10 decals, glass shatter, damage blended on RL6 layers;
  presentation only. Tier C. Size M. Owns `crates/render/src/damage.rs`.
  Accept: golden unchanged; 2,000 holes in view inside the P6 GPU
  budget (reference GPU). needs: GX10, RL6, UW3.
- [ ] UC9 render: interior mapping — closed windows show seeded fake
  rooms (walls, lights, curtains) with no interior geometry. Tier C.
  Size S. Owns `crates/render/src/interior_map.rs`. Accept: 1,000 lit
  windows ≤ 0.3 ms GPU (reference GPU); parallax holds at street level.
  needs: UC2.
- [ ] UC10 assets: block HLOD — per-block merged proxies and impostors
  with meshlets, rebuilt when a ruin state changes. Tier C. Size M. Owns
  `crates/assets/src/hlod.rs`. Accept: a 1 km view draws ≤ 2,000
  batches; no visible pop in the capture review; bytes x2. needs: VG2,
  UC5.
- [ ] UC11 world: smoke and dust volumes — a coarse integer smoke grid
  per cell from grenades, fires and collapse dust that blocks AI sight
  in the sim; VX1/VX5 draw it. Tier C. Size M. Owns
  `crates/world/src/smoke.rs`. Accept: an agent behind smoke is not seen
  (UW14 verifier); spread replays x2; ≤ 0.2 ms per cell. needs: UW14.
- [ ] UC12 world: craters — stamped heightfield deformation per cell
  with nav rebake and crater decals. Tier C. Size M. Owns
  `crates/world/src/crater.rs`. Accept: 100 craters replay x2; routes
  rebake only dirty tiles. needs: WD1, NV3.
- [ ] UC13 content: urban kits — modern concrete and plaster modular
  kits (walls, floors, stairs, balconies, roofs, rubble, rebar, props)
  on trim sheets, CC0 or owner-bought, with provenance rows. Tier C.
  Size L. Owns `content/urban/kits/`. Accept: every UC2/UC3 table entry
  resolves to a provenanced asset; texture memory per block within
  budget. needs: UW0.
- [ ] UC14 proof: district benchmark — a generated 1×1 km district in
  three looks: street fly-through captures and a release frame-time
  report at Base and RT tiers on the reference GPU; review panel against
  the style set. Tier C. Size M. Owns `crates/proof/src/uc.rs`. Accept:
  the `urban-war.md` §2 look and speed bars have evidence lines. needs:
  UC3, UC5, UC8, UC9, UC10, UC13, VG3.
- [ ] UC15 game: city battle mode — 32 players plus 200 AI in the
  district: objectives, fire teams, heavy weapons and live destruction;
  replays exactly. Tier C. Size L. Owns `games/urban/src/battle.rs`.
  Accept: a 20-minute bot-filled battle converges online and replays
  exactly. needs: UW39, UC6, UC7, UC11, UC14.
- [ ] UC16 world: large terrain — streamed 16-bit height tiles per cell
  with splat weights and holes for tunnels, a GPU LOD without cracks,
  collision and nav per tile; replaces the 8-bit seeded heightmap for
  big maps. Tier C. Size L. Owns `crates/world/src/terrain_tiles.rs`.
  Accept: an 8×8 km map streams with zero tick hitches; nav tiles bake
  per cell; bytes x2. needs: UW39, WD1, NV3.
- [ ] UC17 render: vegetation and grass — GPU-instanced trees, bushes and
  grass with wind, distance LOD and impostors, density from splat
  weights; presentation only except tree trunks as colliders. Tier C.
  Size L. Owns `crates/render/src/foliage.rs`. Accept: 1 km view of
  forest edge inside the P6 GPU budget (reference GPU); golden
  unchanged. needs: UC16, VG1.
- [ ] UC18 world: roads on terrain — spline roads that flatten and blend
  the terrain, bridges, road decals and road-following props. Tier C.
  Size M. Owns `crates/world/src/roads.rs`. Accept: a 5 km road network
  bakes identical bytes x2; no vehicle wheel sinks at seams. needs:
  UC16, UC1.
- [ ] UC19 physics: military vehicles — the D1 raycast vehicle extended
  to tracks, turrets with limits, crew seats, armor zones resolved by
  UW3 penetration and damage states. Tier C. Size L. Owns
  `crates/physics/src/armor_vehicle.rs`. Accept: a tank drives the UC16
  test course and a round's penetration depends on the armor zone hit;
  replays x2. needs: UW39, D1, UW3.
- [ ] UC20 physics: helicopter flight — a deterministic rotor model
  (collective, cyclic, tail rotor, ground effect, autorotation-lite) on
  fixed-point state. Tier C. Size M. Owns `crates/physics/src/rotor.rs`.
  Accept: hover, transit and landing tapes replay x2; a scripted pilot
  lands on a roof inside a 2 m circle. needs: UW39, UC16.
