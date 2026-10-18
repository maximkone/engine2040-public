# Silicon World — the chip as a GTA city, plan to the VISION.md bar

Tag: LATER, no Core proof. Look and world half of `silicon-city.md` (read that
first for the sim, the score and the science gate). Owner asked for this on
2026-10-18: "visually like GTA city but in real RAM or chip, go crazy". No row
is on the board yet. Wave 0 spikes in `silicon-city.md` come before any row
here.

Goal: the chip is a city you can walk, drive and fly through. Every building,
car, light and sound is a real number from the truth sim. It looks great and
runs on a potato.

Acceptance:
1. Tier B world: walk and drive through 3 districts (core downtown, cache
   quarter, RAM harbor) while the sim runs; weather, day and night, and sound
   come from the sim; 6 missions play; the sim hash is the same with or without
   a player in the world.
2. Tier B zoom: one unbroken zoom from the street to the whole die, no load
   screen.
3. Tier C world: wire aging, bit-flip meteors and ECC police, the meltdown
   mode, the GPU district, the Heist, and a zoom out to the data hall.

Plain summary: you are a tiny citizen standing on a continent of silicon. The
wires are stacked highways. The caches are penthouses, suburbs and warehouses.
The RAM is a harbor of leaking water buckets that street sweepers must refill
every 64 ms. The branch predictor is a temple of fortune tellers. The clock is
a heartbeat that sweeps the city. You walk, drive a data packet, ride a cache
line, skate along a wire, or fly a drone. The city never lies: nothing is
decoration, and every visual is tied to a counter in the sim.

Owner lanes: `silicon` (game, skin), `render`, `audio`, `content`, `chipsim`
(only for SIL55 and SIL57). Prefix: SIL (rows SIL46 to SIL59 continue
`silicon-city.md`). Base commit: `c7ab2f7`.

## 1. Where it stands

The engine can draw, break, burn and play sound for the arena. Nothing draws a
chip city.

| Capability | Status | Evidence |
|---|---|---|
| Renderer, arena look | live; 704 lib tests passed (runner) | `docs/plan/status-2026-10-17.md` |
| Physical layer: a float Rapier world seeded by authority events, never hashed | exists | `crates/physfx` (`docs/CRATE-MAP.md`) |
| Destruction, fire, ragdoll proofs | Bodies rows ticked; some ticked rows have no code left | `docs/FOCUS.md` Now 2; `docs/plan/status-2026-10-17.md` |
| GPU crowd of 20 K reacting actors | a HARDEN target; not verified for this plan | `docs/FOCUS.md` HARDEN |
| First-person eye on a body | planned, rows FV1 to FV6 | `docs/plan/fps-view.md` |
| Audio: offline synth and headless voice mixer | exists | `crates/audio` |
| HUD as data | exists | `crates/ui` |
| Browser build | early: cleared canvas and one JSON line (WEB4) | `crates/web` |
| Potato gate | numbers, no verdict (G9) | `docs/plan/status-2026-10-17.md` |
| District kit, instanced tile arrays | not started | no such file (Glob 2026-10-18) |
| Continuous zoom camera | not started | same |
| Walk, drive, ride on a chip | not started | same |
| Sim-driven weather, sound, aging | not started | same |

## 2. Finish line

- **Tier B world.** Three districts from a blueprint. Walk, packet car,
  cache-line train. A time dial from 1 cycle per second to 10 M cycles per
  second. Weather, day and night, sound and radio from the sim. 6 missions.
  Lab mode. One zoom from street to die.
  - Frame p95 33 ms on the potato tier (reference GPU); baseline UNKNOWN.
  - Counter map has 0 orphans: every visual names its counter.
  - Sim hash equal with a player moving, standing still, or absent.
- **Tier C world.** Aging roads. Meteors and police. Meltdown mode. GPU
  district with 5000 workers. The Heist as a stealth mission. Zoom out to the
  data hall and in to one transistor.

## 3. Target design

### 3.1 Three rules

1. **Nothing is decoration.** Every light, car, sound and weather effect names a
   counter or an event in `content/silicon/counters.json`. A test fails on an
   orphan visual.
2. **The skin never feeds the sim.** The world reads the event lane. Player
   movement, camera and physics are skin. The sim hash must not move
   (`HASH-LAW.md`). Floats are fine here, never in `chipsim`.
3. **Cheap to draw.** Chips are mostly repeating tiles (cache and RAM arrays).
   Draw them as instanced tiles near and as one textured plane far. A huge city
   stays small on a potato.

### 3.2 Scale, camera and the time dial

You are about one micron tall. The die is a continent. Metal layers are stacked
highways; vias are ramps and lifts. One camera chain zooms through levels with
no load screen: transistor canyon, street, district, die, package, board, rack,
data hall. Every level keeps the same object ids, so a car you follow stays the
same car as you zoom out.

The **time dial** sets sim speed against real time: 1 cycle per second to ride
one instruction, 10 M cycles per second to watch weather. It changes how often
the skin reads events. It never changes the sim.

### 3.3 Districts (real part, city thing, sim source)

| Real part | City thing | What you see | Sim source |
|---|---|---|---|
| Fetch, decode, execute, retire | Airport, customs, factory towers, exit gate | Crowds queue at customs when decode is full | pipeline stage occupancy |
| L1 cache | Penthouses next to the core | Tiny, bright, always busy | L1 hit and miss events |
| L2, L3 | Suburbs, warehouse district | Trucks fetch from the warehouse | L2 and L3 events |
| DRAM | Harbor of billions of leaking buckets | Sweepers repaint buckets every 64 ms | refresh and bank events |
| Row buffer | Open container yard | Open yard is fast, closed yard is slow | row hit and miss |
| Branch predictor | Oracle Temple | A wrong guess recalls the whole parade | mispredict flush |
| Reorder buffer | Clearing house | Results wait for the stamp, in order | ROB occupancy |
| On-chip network | Subway | Packets are trains; rush hour is a jam | NoC credits and queues |
| GPU | Vegas | Thousands of workers march in lockstep | warp divergence counters |
| Speculation | Ghost town | Cars run ahead and get recalled; footprints stay | speculative events |
| Clock tree | Heartbeat | A shockwave sweeps the city each tick | tick counter |
| I/O | Airport and border | Slow cargo trains arrive from far away | I/O latency events |

### 3.4 Weather and law

| City thing | What it is | Sim source |
|---|---|---|
| Heat haze and glow | Hot spots | heat grid cell value |
| Throttle | The city dims by decree | DVFS state |
| Idle districts go dark | Power gating | gated block flags |
| Meteors | Cosmic-ray bit flips | flip events |
| Police and ambulances | ECC fixing flips | ECC events |
| Blackout riot | ECC overwhelmed | uncorrectable error |
| Potholes and cracks | Wire aging (electromigration) | wear map |
| Wanted level | How near the thermal limit | heat versus limit |

### 3.5 Moving and playing

- On foot, skate along a wire, wingsuit off the top layer, free-fly drone.
- Packet car: ride a real packet by id. Cache-line train: ride a line from
  RAM to L1. Instruction ride: `silicon-city.md` 3.6, the first-person mode.
- Hold-my-beer mode: push the clock past the limit. Timing errors, then
  runaway, then fire and collapse. Drawn by `physfx` from events. The sim only
  holds a "melted" flag.
- Lab mode: A/B compare, one-knob sweeps, CSV export, a shared replay anyone
  can re-verify.

### 3.6 Missions (the scenario deck)

| Mission | Chip event | Tests |
|---|---|---|
| Rush Hour | burst of work | queues, caches, memory speed |
| New Tenants | the software changes | flexibility |
| Heatwave | too hot, throttling | cooling, power limit |
| Blackout | power capped | energy per instruction |
| Earthquake | radiation flips bits | error correction |
| Recession | budget cut | area, yield, cost |
| Broken Factory | defective chips | spares, binning |
| The Heist | Red steals a secret, Blue guards | side-channel defense |

The Heist is stealth and timing, not guns. It is the Red vs Blue duel of
`silicon-city.md` 3.9 played through the ghost town.

### 3.7 Radio and sound

Stations are workloads: Radio Browser, Radio Database, Radio AI-Training,
Radio Video-Encode, Radio Graph. The city sounds and moves like the program it
runs. Tempo follows the clock. A stall is a held breath. Sound is off the tick
and never read by the sim.

### 3.8 Look

Tron, Blade Runner 2049 and *Powers of Ten* are the mood. Real die photos give
the iridescent rainbow of the metal layers. Neon is baked on the potato tier;
bloom, haze and the 2040 look rows (`render-look.md`) switch on above it.
Distant districts become impostors. The same machine look score as the arena
(G4) judges it.

### 3.9 Trailer shots (the proof the world works)

1. The infinite zoom out to the data hall.
2. The 64 ms sweepers repainting the RAM harbor.
3. A Spectre-style Heist through the ghost town.

## 4. Hard problems

| Problem | Technique | Why | Reference | Donor |
|---|---|---|---|---|
| Zoom with no load screen | one object-id chain; impostors per level; cell streaming | the signature shot | FOCUS: streamed cells | existing render and world |
| Billions of cells on a potato | instanced tiles near, textured plane far, procedural arrays | chips repeat | 3.1 rule 3 | existing render |
| Every visual tied to a counter | `counters.json` plus an orphan test | the nerd promise | 3.1 rule 1 | none |
| Destruction without breaking the hash | `physfx` float world seeded by events; sim keeps a flag | skin never feeds sim | `crates/physfx` | existing physfx |
| Time scale span of 10^7 | time dial changes the event read rate only | one sim, many speeds | 3.2 | none |
| Readability in first person | stall cues are light, door and sound, not just text | no nausea, clear cause | `fps-view.md` | existing fps-view |
| Drive feel on a weak PC | packet car follows a path, no wheel physics | cheap and clear | none | none |
| Crowds at scale | GPU crowd path, instanced workers | thousands of actors | FOCUS GPU crowd | existing crowd target |
| Sound from events, off tick | event lane to the audio mixer | replay-safe | `crates/audio` | existing audio |

## 5. Donors

Donors: steals.md; porting policy is AGENTS rule 4.

## 6. Slim

- Not building: guns, violence or NPC lives. The city is non-lethal.
- Not building: a story, voice acting, or realistic human bodies.
- Not building: wheel or vehicle physics in the sim. Vehicles follow data.
- Not building: a second renderer, or a second audio stack.
- Not building: any visual that has no counter.
- Rule: no `chipsim` change except SIL55 and SIL57.
- Rule: no Rome row changes; no engine core crate gains a chip assumption.
- Freeze: no row here runs before the owner says go (D-SIL3 in `silicon-city.md`).

## 7. Board tasks

Not on the board yet. Format per `README.md`. Sizes: S 1 session, M 2 to 4,
L 5 to 10.

### Tier B world

- [ ] SIL46 silicon: district kit v1. Tier B. Size L. Owns
  `games/silicon/src/district/`, `content/silicon/kit/`. Accept: core downtown,
  cache quarter and RAM harbor draw from a blueprint; repeating tiles are
  instanced; potato frame p95 33 ms (reference GPU); 3 captures beside
  references. needs: SIL13.
- [ ] SIL47 silicon: walk and drive. Tier B. Size M. Owns
  `games/silicon/src/move.rs`. Accept: on foot, packet car and cache-line train;
  vehicles bind to real packet ids; sim hash equal with and without a moving
  player, x2. needs: SIL14, SIL46.
- [ ] SIL48 silicon: continuous zoom camera. Tier B. Size L. Owns
  `games/silicon/src/zoom.rs`. Accept: one zoom from street to die with 0 load
  screens and no frame over 50 ms; object ids stable across 3 level swaps.
  needs: SIL46.
- [ ] SIL49 silicon: time dial. Tier B. Size M. Owns `games/silicon/src/dial.rs`.
  Accept: 12 steps from 1 cycle per second to 10 M cycles per second; hash equal
  at every step; ride mode works at the slow end. needs: SIL14.
- [ ] SIL50 silicon: weather and day-night. Tier B. Size M. Owns
  `games/silicon/src/weather.rs`. Accept: haze equals the heat cell, a dark
  block equals a gated block, the pulse equals the tick; counter map has 0
  orphans. needs: SIL28, SIL46.
- [ ] SIL51 audio: soundscape and radio. Tier B. Size M. Owns
  `games/silicon/src/sound.rs`. Accept: tempo follows the clock; 5 stations, one
  per workload; a stall cue; sim hash unchanged x2. needs: SIL13.
- [ ] SIL52 content: 6 missions. Tier B. Size M. Owns
  `content/silicon/missions/`. Accept: Rush Hour, New Tenants, Heatwave, Blackout,
  Earthquake, Recession; each has a pass rule and one bot score. needs: SIL29.
- [ ] SIL53 silicon: Lab mode. Tier B. Size M. Owns `games/silicon/src/lab.rs`.
  Accept: A/B compare, a 10-step knob sweep, CSV export, a shared replay that
  re-verifies. needs: SIL32.

### Tier C world

- [ ] SIL54 silicon: meteors, police and ghosts. Tier C. Size M. Owns
  `games/silicon/src/crime.rs`. Accept: bit-flip meteors, ECC police and
  speculative footprints each tied to an event type; 0 orphans. needs: SIL23,
  SIL28.
- [ ] SIL55 chipsim: wire aging. Tier C. Size M. Owns
  `crates/chipsim/src/aging.rs`. Accept: wear grows with activity over simulated
  years; a failed wire is flagged; integer only; hash x2. needs: SIL26, SIL28.
- [ ] SIL56 silicon: meltdown mode. Tier C. Size M. Owns
  `games/silicon/src/meltdown.rs`. Accept: clock past the limit gives timing
  errors, then runaway, then fire and collapse from events via `physfx`; replay
  equal; sim hash unchanged by the visual physics. needs: SIL28, SIL51.
- [ ] SIL57 chipsim: GPU-lite and the Vegas district. Tier C. Size L. Owns
  `crates/chipsim/src/gpu.rs`, `games/silicon/src/district/vegas.rs`. Accept:
  lockstep groups of 32; divergence cost counted; 5000 workers at potato p95
  33 ms. needs: SIL25, SIL46.
- [ ] SIL58 silicon: far zoom and transistor canyon. Tier C. Size M. Owns
  `games/silicon/src/zoom_far.rs`. Accept: die to board to rack to data hall in
  one shot, and a visual-only dig into one transistor; 0 load screens. needs:
  SIL48, SIL45.
- [ ] SIL59 silicon: the Heist. Tier C. Size M. Owns
  `content/silicon/missions/heist/`. Accept: a Red vs Blue duel plays as a stealth
  mission; the replay hash equals the duel receipt. needs: SIL42, SIL54.

Row changes: none. No existing rows carry SIL46 to SIL59.

## 8. Quality gates

- Counter map: a test fails on any visual with no counter.
- Hash: sim hash equal with a player moving, standing still and absent, x2.
- Potato: `node tools/potato-check.mjs`; frame p95 33 ms; wasm size.
- Look: captures beside references; every look pass LIVE.
- Zoom: no load screen; no frame over 50 ms across level swaps.
- Sound: off tick; replay needs no audio.
- No float in `crates/chipsim` (SIL55 and SIL57 included).
- Docs: `node tools/workspace-check.mjs` after each doc edit.

## 9. Session estimate

Rough, sized from the rows (S 1, M 2 to 4, L 5 to 10).

- Tier B world (SIL46 to SIL53): 22 to 44 sessions.
- Tier C world (SIL54 to SIL59): 15 to 30 sessions.
- Total: 37 to 74 sessions, on top of `silicon-city.md`.
