# Silicon City — plan to the VISION.md bar

Tag: LATER, no Core proof. Owner asked for this plan in chat on 2026-10-18
("big plan for future game", "biggest league"). FOCUS D7 froze new plan files;
the owner's request wins, and the LATER tag keeps loop work off it. Rome stays
the flagship. No row below is on the board yet; Wave 0 spikes may run beside
the FOCUS Now rows because they touch only a new leaf crate.

Goal: a game where you build a CPU like a city and a factory, watch it run
from the inside, and compete on a public skyline. The best ideas are checked in
real open simulators and can become real research.

Acceptance:
1. Tier A "One Street": one core, caches and a branch predictor in the browser
   on the potato tier; the truth sim is within 10% of ChampSim on 5 open
   traces; same design twice gives one hash.
2. Tier B "Eras and Season 1": eras 1971 to 2025, 5 customers, human and bot
   classes on a verified skyline with hidden workloads.
3. Tier C "The Lab": a skyline design passes a reference-sim gate and exports
   to Verilog; one Red vs Blue season runs.

Plain summary: players plan a chip the way they plan a city. Blocks are
districts, wires are roads, caches are warehouses, heat is pollution. The chip
also runs like a factory: instructions flow down a line and the fun is finding
the jam. A headless, integer, hashed sim is the truth. The pretty city is a
skin that only reads its events. You can walk and drive through it like a GTA
city, and ride one instruction through the chip in first person. Eras teach you from a 1971 chip upward, so you need no
PC knowledge. Scores sit on a skyline of performance, power and area. Humans,
bots and human+bot teams share it. Designs that push the skyline are re-run
on hidden workloads and in a reference simulator before anyone calls them a
discovery.

Owner lanes: `chipsim` (the truth sim, a leaf crate), `silicon` (game, skin,
`games/silicon/`), `proof`, `endpoint`, `web`, `render`, `content`.
Prefix: SIL. Base commit: `2da5e84`. The look and world half (GTA-style city,
missions, zoom, sound; rows SIL46 to SIL59) is `silicon-world.md`.

## 1. Where it stands

The engine has the spine this game needs. Nothing chip-specific exists.

| Capability | Status | Evidence |
|---|---|---|
| Fixed tick, seeded rng, per-tick hash | integrated (arena) | `crates/core`; `docs/CRATE-MAP.md` |
| Replay by recorded tape, one hash | integrated (arena) | `README.md` run line; `docs/plan/status-2026-10-17.md` |
| Typed diagnostic events | exists, std-only | `crates/observe` (`docs/CRATE-MAP.md`) |
| HUD as data, drawn from snapshots | exists | `crates/ui` |
| Integer-only neural runtime | exists | `crates/nn` |
| Rules an AI writes as text, sandboxed, replay-exact | exists | `crates/rules`; FOCUS D2 |
| AI door (typed verbs, MCP feature) | integrated; endpoint 157 lib passed, mcp `packs::play` 7 passed (runner) | `docs/plan/status-2026-10-17.md` |
| Playtest verifiers and mirror proof | 49 passed (runner) | same page |
| Renderer | arena look live; 704 lib tests passed; no chip look | same page |
| Browser build | early: shell, WebGPU device, cleared canvas, one JSON line (WEB4) | `crates/web` (`docs/CRATE-MAP.md`); FOCUS Now 9 |
| Potato gate | numbers, no verdict (G9) | `docs/plan/status-2026-10-17.md` |
| ISA, trace reader, pipeline, cache, predictor models | not started | `docs/CRATE-MAP.md` lists no such crate (2026-10-18) |
| Floorplan, wires, NoC, power, heat, yield | not started | same |
| PPA score, skyline, verifier, receipts | not started | same |
| Leaderboard host | not started; the repo has no server | owner decision D-SIL2 below |
| ChampSim bridge, Verilog export | not started | same |

Layering today: `core` is the leaf under everything. `chipsim` must also be a
leaf (depends on `core` only). Rule 3 of `AGENTS.md`: engine crates take no
genre assumption, so `chipsim` is a game-side leaf like `world`, not an engine
core crate.

## 2. Finish line

Plain: three steps. First a tiny street that is true and fun. Then history and
real customers. Then the part that makes it science.

- **Tier A, One Street (alpha).** One in-order core, L1 and L2, DRAM, a branch
  predictor slot, 5 open workloads, a PPA score, a local skyline, ride mode,
  X-ray lenses, a tutor. Runs native and in a browser tab.
  - Truth: IPC within 10% of ChampSim on 5 traces and 3 configs. Kill rule at
    15% after one calibration pass (SIL1).
  - Hash: every workload x config run twice gives one hash (SIL19).
  - Potato: frame p95 at or under 33 ms on the potato tier; sim speed target
    1 M instructions per second per thread in wasm; baselines UNKNOWN.
  - Tutor: the stall it names equals the top stall counter on every workload.
- **Tier B, Eras and Season 1 (v1).** Era table 1971 to 2025 with a tech tree;
  superscalar, out-of-order, multicore, NoC, floorplan, heat; 5 customers; the
  verifier; hidden workloads; classes Human, Bot, Centaur; a bot ladder.
  - A fresh clone verifies any receipt with one command and gets the same hash.
  - Season pack holds 4 hidden workloads sealed by hash before the season.
  - 4 reference bots publish scores (random, hill-climb, genetic, LLM through
    the door).
- **Tier C, The Lab (VISION bars).** A skyline design gets a "Reference-matched"
  badge only after ChampSim (or gem5) agrees within 10% on public and hidden
  traces. A predictor or cache block exports to Verilog and passes an
  equivalence check. One Red vs Blue season runs on async duels. Stretch:
  FPGA or Tiny Tapeout for one tiny block; "Rome on your chip".

The league: Season 1 uses contest-style rules (a fixed bit budget per block,
like the 64 KB per core in DPC-3). Labs and AI teams may enter bots. The bot
ladder is a public, reproducible benchmark of AI agents on hardware design,
which feeds the AI-door proof (`hard-systems.md`).

## 3. Target design

### 3.1 The game in one page

Loop: plan the chip, run a workload, read the city, fix the jam, score on the
skyline. Three genres in one:

- **Factory.** Instructions are items on a belt. Pipelines, queues and ports
  are the line. The fun is the bottleneck hunt, like Factorio.
- **City.** Blocks are zones. Wires are roads (upper metal layers are stacked
  highways, vias are ramps). Power is the grid. Heat is pollution. Area is land
  price and also yield. Caches are warehouses: close and small, or far and
  huge.
- **Contest.** A season has a customer, a budget and hidden workloads. The
  skyline ranks you. A Zachtronics-style histogram shows where you stand.

### 3.2 Zoom ladder

| Level | You place | Model | Speed target |
|---|---|---|---|
| L0 Gate (later, optional) | gates for a small unit | exact logic | tiny puzzles |
| L1 Block (the game) | pipeline stages, caches, predictors, ports | cycle-level, integer | 1 M instr/s per thread |
| L2 Die (the city) | block floorplan, wires, NoC, power, heat | tables + fixed-point grid | 100 K instr/s |
| L3 Region (later) | chips, DRAM, boards, racks | analytic | whole run in seconds |

Players work at the level they like and may "dig down" into one block. L1 is
what ships first.

### 3.3 Truth sim and skin (the one rule)

`engine-chipsim` is headless, std-only, integer only, hashed per tick like the
arena (`HASH-LAW.md`). It emits typed events (fetch, stall with reason, hit,
miss, mispredict flush, retire, heat tick) into the `observe` lane. The skin in
`games/silicon/` reads events and draws. The skin never feeds the sim.
Presentation may use floats; the sim may not.

Every model carries a calibration card: what it was checked against, the error,
the date, the sim version. If a model has no card, or its error is over the
limit, its badge falls to Toy and its results cannot reach the science gate.

Ladder of trust per design:
1. **Toy**: plays, no claim.
2. **Calibrated**: models carry a passing card.
3. **Reference-matched**: ChampSim or gem5 agrees within 10% on public and
   hidden traces.
4. **Silicon-proven**: the block ran on an FPGA or a shuttle chip.

The word "discovery" is allowed only at step 3 and above.

### 3.4 Workloads are the citizens

Each workload is a trace or a small RV32 program with its own personality
(branchy browser, pointer-chasing database, streaming media, graph analytics,
dense math). Open sources first. SPEC is paid and stays out. Every workload
gets a provenance note. Season packs add sampled phases (SimPoint-style) so a
season is short but true.

### 3.5 Score and skyline

- Performance: geometric mean of instructions per second over the suite.
- Power: energy per instruction from activity counts times an integer energy
  table.
- Area: block table plus wire area; cost = area through a yield curve.
- Skyline: the scatter of designs; the frontier line is the Pareto set. A
  discovery is a point that pushes the line out on hidden workloads.
- Customers set weights and caps: a 1 mW watch, a radiation-hit rover (bit
  flips as weather), a potato phone, a console, a server.

### 3.6 Ride an instruction

A first-person mode. The camera rides one instruction (or one cache line)
through fetch, decode, execute and retire. Stalls are felt: the ride waits at a
door, the light dims, the hum drops. Mispredicts flush the corridor. It uses
the event lane and the FV eye (`fps-view.md`) when that lands. Sound follows
the clock: tempo is frequency, a stall is a held breath.

### 3.7 Era mode and alt history

Start with a 1971 budget (thousands of transistors). Reinvent pipelining,
caches, prediction, out-of-order, SIMD, multicore, NoC and chiplets as the
budget grows. Budgets are game values, not history. Alt-history branches remove
a tech or force a limit ("no caches until 1990", "power capped at 1 W") so the
player may find roads history skipped. This is the teaching path for a player
who knows no PC.

### 3.8 Bots and the AI door

Bots are first-class. Every class uses the same door (`engine-endpoint`, verb
pack `chip`): read blueprint, edit blueprint, run, read receipt. Classes:
Human, Bot, Centaur (human plus bot). Official reference bots: random search,
hill-climb, genetic, and an LLM through the door. The coach and tutor use the
sim's own counters; an LLM may only phrase the sentence, off-tick, from a
recorded cassette. Replay never needs a model.

### 3.9 Red vs Blue (async)

Blue publishes a fortress chip. Red submits a program that tries to leak a
secret through cache timing in a toy model. Both run in the deterministic sim,
so a match is a replay file, not netcode. Blue scores retained speed while
leaking under a limit. Red scores bits leaked per cycle. No real-time PvP in
v1 or v2; the rollback net is not used.

### 3.10 The Lab (science pipeline)

1. Player design lands on the skyline (verified receipt).
2. Candidate: top of the frontier at season end.
3. Hidden run: 4 sealed workloads, same hash law.
4. Reference run: ChampSim (later gem5) on GitHub through `tools/gh-check.mjs`;
   the badge moves to Reference-matched or drops.
5. Paper kit: design, method, ablations, receipts in one folder.
6. Export: Verilog for predictor and cache blocks; equivalence check; FPGA.

### 3.11 Look and world

The chip is a GTA-style city you walk, drive and fly through: districts for the
core, caches, RAM harbor, subway, GPU Vegas and a ghost town for speculation.
Every light, car and sound maps to a sim counter. The full world bible, the
missions, the time dial, the zoom chain and rows SIL46 to SIL59 live in
`silicon-world.md`. Potato tier: instanced tiles and packets, baked glow, no
ray tracing.

### 3.12 Layers and files

- `crates/chipsim/` (`engine-chipsim`, depends on `core` only): trace, pipeline,
  cache, predictor, floorplan, power, heat, score, blueprint, receipt.
- `games/silicon/`: scenes, ride camera, lenses, skyline UI, tutor; depends on
  `chipsim`, `render`, `ui`, `observe`, `audio`, `input`, `save`.
- `crates/endpoint` verb pack `chip` (feature); `crates/playtest` bot ladder.
- `tools/chipsim-champ.mjs` (calibration job), `tools/silicon-check.mjs` (gates).
- Files: blueprint (text, hashed, AI-writable, like `rules`), trace (compact
  binary), season pack (workload hashes sealed before the season), receipt
  (design hash, workload hashes, sim version, scores).

### 3.13 Budgets

Sim speed per level (3.2), wasm size under the web budget in `potato.md`, frame
p95 33 ms potato, replay file under 1 MB per receipt, verifier at most 60 s for
a season run on the reference box. All baselines UNKNOWN until SIL1 to SIL4 run.

## 4. Hard problems

| Problem | Technique | Why | Reference | Donor |
|---|---|---|---|---|
| Fun versus truth | two layers; calibration card; trust ladder | a pretty wrong sim teaches wrong | 3.3 | ChampSim, gem5 |
| Billions of instructions in a session | sampled phases; interval model for out-of-order; fast-forward | a game cannot replay a full benchmark | Sniper interval model | Sniper, SimPoint |
| Overfitting to public traces | sealed hidden workloads each season | the classic contest failure | 3.10 | CBP, DPC, CRC rules |
| Cheating with hidden state | every predictor and cache state is allocated through a counted bit budget | a 64 KB limit must be real | DPC-3 budget | ChampSim API |
| Authoring for non-coders | block editor; integer rules script (Rhai sandbox, FOCUS D2); wasm plug-in slot later | three skill levels, one slot | `crates/rules` | engine rules |
| Speed in a browser | integer sim, no floats, cache-friendly SoA, event sampling | potato target | `potato.md` | existing perf crate |
| Hold results without a server | verifier is a pure function; receipts are files | the repo has no center | owner D-SIL2 | none |
| Teaching a newcomer | era mode; tutor bound to counters | the owner knows no PC | 3.7 | none |
| Wire and heat realism cheaply | wire delay by length table; integer diffusion grid; DVFS throttle | enough truth, small cost | HotSpot idea | HotSpot, CACTI, McPAT |
| NoC congestion | credit flow control on a mesh or ring | traffic is the city | BookSim | BookSim, Garnet |
| DRAM timing | bank and row-buffer model | memory wall is the core lesson | DRAMsim3 | Ramulator, DRAMsim3 |
| Reference proof | differential tests on GitHub | no trust without a second sim | 3.10 | ChampSim, gem5 |
| Real chip path | Verilog generator; Verilator equivalence; FPGA | silicon-proven step | open EDA flow | Yosys, Verilator, OpenROAD |
| Safe security game | toy side-channel model only | a game, not an exploit kit | 3.9 | none |

## 5. Donors

Donors: steals.md; porting policy is AGENTS rule 4.

## 6. Slim

- Not building: a full gate-level game as the main loop (Turing Complete owns
  it); L0 is a later puzzle pack.
- Not building: real-time PvP or rollback for this game; async replays only.
- Not building: a custom ISA in v1 (RISC-V only), analog, or full RTL and EDA.
- Not building: SPEC workloads (paid) in v1.
- Not building: a second renderer. `crates/render` is the only picture.
- Freeze: no Wave 1 or later row runs before the owner says go. Wave 0 spikes
  only.
- Rule: no word "discovery" in UI, docs or press before trust step 3.
- Rule: no float in `chipsim`; a test greps for `f32`/`f64`.
- Rule: no Rome row changes; no engine core crate gains a chip assumption.

## 7. Board tasks

Not on the board yet. Format per `README.md`. Sizes: S 1 session, M 2 to 4,
L 5 to 10.

### Wave 0: spikes (each has one test, one baseline, a kill rule)

- [ ] SIL1 chipsim: truth spike. Tier A. Size M. Owns `crates/chipsim/src/trace.rs`,
  `crates/chipsim/src/cache.rs`. Accept: in-order core, 32 KB L1 and 256 KB L2,
  5 open traces; IPC within 10% of ChampSim; kill at 15% after one calibration
  pass, then drop all science wording; hash x2. needs: none.
- [ ] SIL2 silicon: ride spike. Tier A. Size M. Owns `games/silicon/src/ride.rs`.
  Accept: one instruction ridden through a 5-stage pipe at frame p95 33 ms on
  the potato tier (reference GPU); kill if over 50 ms. needs: none.
- [ ] SIL3 playtest: bot spike. Tier A. Size M. Owns `crates/playtest/src/chip_bot.rs`.
  Accept: a bot through the door cuts mispredicts per 1000 instructions by at
  least 3% over gshare on 5 traces within 200 runs, and beats random search;
  kill if not. needs: SIL1.
- [ ] SIL4 silicon: tutor spike. Tier A. Size S. Owns `games/silicon/src/tutor.rs`.
  Accept: the named top stall equals the top stall counter on 5 of 5 traces
  (test). needs: SIL1.

### Wave 1: One Street (Tier A)

- [ ] SIL5 chipsim: crate skeleton. Tier A. Size M. Owns `crates/chipsim/src/lib.rs`.
  Accept: fixed-point types, event lane, counted bit-budget allocator; a fuzz of
  1000 allocations never passes the budget; grep for `f32`/`f64` finds 0.
  needs: SIL1.
- [ ] SIL6 chipsim: trace format and workload packs. Tier A. Size M. Owns
  `crates/chipsim/src/trace.rs`, `content/silicon/workloads/`. Accept: 5 open
  workloads with provenance notes; reader replays each to one hash x2. needs: SIL5.
- [ ] SIL7 chipsim: in-order pipeline with stall reasons. Tier A. Size M. Owns
  `crates/chipsim/src/pipeline.rs`. Accept: 6 stall reasons counted; counters sum
  to total cycles on 5 traces. needs: SIL5.
- [ ] SIL8 chipsim: caches and DRAM. Tier A. Size M. Owns
  `crates/chipsim/src/cache.rs`, `dram.rs`. Accept: LRU and one pluggable policy;
  hit and miss counts equal a reference cache script on 3 traces. needs: SIL5.
- [ ] SIL9 chipsim: branch predictors. Tier A. Size M. Owns
  `crates/chipsim/src/predict.rs`. Accept: bimodal, gshare, TAGE-lite; slot takes
  a 64 KB budget; MPKI printed on 5 traces. needs: SIL5.
- [ ] SIL10 chipsim: PPA v0. Tier A. Size M. Owns `crates/chipsim/src/ppa.rs`.
  Accept: energy, area and yield tables; score is stable to the integer on 3
  configs. needs: SIL7, SIL8.
- [ ] SIL11 chipsim: blueprint, validator, `chip` door verbs. Tier A. Size M.
  Owns `crates/chipsim/src/blueprint.rs`, `crates/endpoint/src/chip.rs`. Accept:
  12 malformed blueprints refused with typed errors; undo works; hash x2.
  needs: SIL5.
- [ ] SIL12 proof: ChampSim calibration job. Tier A. Size M. Owns
  `tools/chipsim-champ.mjs`. Accept: runs on GitHub via `tools/gh-check.mjs`;
  writes one calibration card per model with error in percent. needs: SIL8, SIL9.
- [ ] SIL13 silicon: street scene. Tier A. Size M. Owns `games/silicon/src/street.rs`.
  Accept: a blueprint draws as a city; 100 K events per second drawn, 0 sim
  reads from the skin (test). needs: SIL5, SIL11.
- [ ] SIL14 silicon: ride mode v1. Tier A. Size M. Owns `games/silicon/src/ride.rs`.
  Accept: ride any instruction in a replay; stall, miss and flush each have a
  distinct cue; capture beside a reference. needs: SIL2, SIL13.
- [ ] SIL15 silicon: X-ray lenses. Tier A. Size M. Owns `games/silicon/src/lens.rs`.
  Accept: traffic, heat and stall lenses; each lens value equals its counter on
  5 traces. needs: SIL13.
- [ ] SIL16 silicon: local skyline. Tier A. Size M. Owns `games/silicon/src/skyline.rs`.
  Accept: 20 saved designs plot; frontier line equals a brute-force Pareto set.
  needs: SIL10.
- [ ] SIL17 silicon: tutor v1. Tier A. Size S. Owns `games/silicon/src/tutor.rs`.
  Accept: 10 plain-words templates, each bound to a counter; claim equals
  counter on 5 traces. needs: SIL4, SIL15.
- [ ] SIL18 web: the street in a tab. Tier A. Size M. Owns `crates/web/src/silicon.rs`.
  Accept: the same blueprint gives the same hash native and in wasm; potato
  frame p95 33 ms (reference GPU). needs: SIL13, WEB rows for a playing tab.
- [ ] SIL19 proof: Tier A check. Tier A. Size S. Owns `tools/silicon-check.mjs`.
  Accept: prints gates A1 to A5 (truth, hash, potato, tutor, skyline) with
  numbers. needs: SIL12, SIL16, SIL17, SIL18.

### Wave 2: Eras (Tier B)

- [ ] SIL20 chipsim: RV32IM execution mode. Tier B. Size L. Owns
  `crates/chipsim/src/rv32.rs`. Accept: 20 small programs run to the same
  registers as a reference emulator; hash x2. needs: SIL7.
- [ ] SIL21 silicon: era table and tech tree. Tier B. Size M. Owns
  `content/silicon/eras.json`, `games/silicon/src/era.rs`. Accept: 8 eras with
  budgets and unlocks; a locked part cannot be placed (test). needs: SIL11.
- [ ] SIL22 chipsim: hazards, forwarding, superscalar in-order. Tier B. Size M.
  Owns `crates/chipsim/src/pipeline.rs`. Accept: width 1 to 4; stall counters
  still sum to cycles. needs: SIL7.
- [ ] SIL23 chipsim: out-of-order core. Tier B. Size L. Owns
  `crates/chipsim/src/ooo.rs`. Accept: ROB, ports, load-store queue; IPC within
  10% of ChampSim on 5 traces. needs: SIL22, SIL12.
- [ ] SIL24 chipsim: prefetchers and replacement slots. Tier B. Size M. Owns
  `crates/chipsim/src/policy.rs`. Accept: 3 prefetchers and 3 policies; a
  ChampSim-style API subset; budget enforced. needs: SIL8.
- [ ] SIL25 chipsim: multicore and shared L3. Tier B. Size L. Owns
  `crates/chipsim/src/mc.rs`. Accept: 4 cores, simple coherence; hash x2 for
  any core order seed. needs: SIL23, SIL24.
- [ ] SIL26 chipsim: floorplan and wires. Tier B. Size L. Owns
  `crates/chipsim/src/floorplan.rs`. Accept: grid placement; wire delay by
  length table; area equals the sum of blocks plus wires on 5 plans. needs: SIL10.
- [ ] SIL27 chipsim: on-chip network. Tier B. Size M. Owns `crates/chipsim/src/noc.rs`.
  Accept: mesh and ring with credits; saturation point found on 3 patterns.
  needs: SIL26.
- [ ] SIL28 chipsim: power and heat. Tier B. Size M. Owns `crates/chipsim/src/heat.rs`.
  Accept: integer diffusion grid; throttle and DVFS; energy conserved to 1% on
  3 traces. needs: SIL26.
- [ ] SIL29 content: 5 customers. Tier B. Size M. Owns `content/silicon/customers/`.
  Accept: watch, rover, potato phone, console, server; each has caps and
  weights; one bot score each. needs: SIL10, SIL28.
- [ ] SIL30 silicon: alt-history branches. Tier B. Size S. Owns
  `content/silicon/whatif.json`. Accept: 6 what-ifs; each runs to a skyline
  point. needs: SIL21.
- [ ] SIL31 silicon: authoring levels. Tier B. Size M. Owns
  `games/silicon/src/author.rs`. Accept: block editor, integer script slot in the
  rules sandbox; a script over its op limit is refused with a typed error.
  needs: SIL11.

### Wave 3: Season 1 and bots (Tier B end)

- [ ] SIL32 chipsim: verifier. Tier B. Size M. Owns `crates/chipsim/src/verify.rs`.
  Accept: (blueprint, season pack) gives a receipt; 3 clean clones agree; 50
  fuzzed blueprints never pass the time or memory cap. needs: SIL11, SIL23.
- [ ] SIL33 content: season pack and sealed hold-out. Tier B. Size M. Owns
  `content/silicon/season1/`. Accept: 4 hidden workloads, hash committed before
  the season; reveal matches the commit. needs: SIL6, SIL32.
- [ ] SIL34 silicon: public skyline and classes. Tier B. Size M. Owns
  `games/silicon/src/board.rs`. Accept: Human, Bot, Centaur classes; every row
  has a receipt that re-verifies. needs: SIL32. Owner decision D-SIL2.
- [ ] SIL35 playtest: bot ladder. Tier B. Size M. Owns `crates/playtest/src/chip_ladder.rs`.
  Accept: 4 reference bots publish scores; reruns equal. needs: SIL3, SIL32.
- [ ] SIL36 silicon: design replay theater. Tier B. Size M. Owns
  `games/silicon/src/theater.rs`. Accept: any receipt replays with ride and
  lenses; hash equals the receipt. needs: SIL14, SIL32.
- [ ] SIL37 silicon: coach phrasing. Tier B. Size S. Owns `games/silicon/src/coach.rs`.
  Accept: a recorded model line replaces a template; the claim check still
  passes on 5 traces; replay needs no model. needs: SIL17.

### Wave 4: The Lab (Tier C)

- [ ] SIL38 proof: reference gate. Tier C. Size L. Owns `tools/chipsim-ref.mjs`.
  Accept: nightly job re-runs the top 10 designs in ChampSim; badge updates;
  error under 10% on public and hidden. needs: SIL12, SIL33.
- [ ] SIL39 chipsim: Verilog export and equivalence. Tier C. Size L. Owns
  `crates/chipsim/src/verilog.rs`. Accept: predictor and cache blocks export;
  Verilator agrees with the sim on 5 traces cycle by cycle. needs: SIL9, SIL24.
- [ ] SIL40 content: FPGA or shuttle path. Tier C. Size M. Owns
  `docs/silicon-fpga.md`. Accept: one tiny block runs on one board; cost note
  dated. needs: SIL39. Owner decision D-SIL4.
- [ ] SIL41 silicon: paper kit. Tier C. Size S. Owns `tools/silicon-paper.mjs`.
  Accept: one folder with design, method, 3 ablations and receipts for one
  skyline design. needs: SIL38.
- [ ] SIL42 chipsim: Red vs Blue. Tier C. Size L. Owns `crates/chipsim/src/sidechan.rs`.
  Accept: toy cache-timing model; one duel replays x2; red leaks more than 0
  bits on a naive chip and blue holds under the limit on a defended chip.
  needs: SIL23, SIL32.
- [ ] SIL43 content: side-mode packs. Tier C. Size M. Owns `content/silicon/modes/`.
  Accept: Data Port (memory only) and Compiler Factory (schedule code) as season
  packs on the same verifier. needs: SIL33, SIL20.
- [ ] SIL44 proof: Rome on your chip (stretch). Tier C. Size L. Owns
  `tools/silicon-rome-trace.mjs`. Accept: a trace of the arena sim runs as a
  workload; fights per second shown per chip; feasibility UNKNOWN, kill if no
  trace in 3 sessions. needs: SIL20.
- [ ] SIL45 chipsim: region level L3. Tier C. Size L. Owns `crates/chipsim/src/region.rs`.
  Accept: chips, DRAM and racks as an analytic model; a 1-hour run in 10 s;
  cooling plan scored. needs: SIL28, SIL25.

Row changes: none. No existing rows carry the SIL prefix.

### Open owner decisions

- D-SIL1 Name. Working title "Silicon City". Pick or change before any art.
- D-SIL2 Leaderboard host. Receipts as files checked by CI (free, slow, no
  server), or one small service (fast, costs money and needs care).
- D-SIL3 Start. Wave 0 spikes beside the FOCUS Now rows, or all of it after the
  Core proofs.
- D-SIL4 Silicon money. Tiny Tapeout and shuttle prices moved after the Efabless
  closure in 2025; check the live price before any spend.
- D-SIL5 Workloads. Open traces only in v1, or buy SPEC for the science gate.

## 8. Quality gates

- Hash law: every workload x config x design runs twice to one hash; golden per
  pack. Arena golden `d57b5c8ae3096614` stays unchanged.
- Calibration: error versus ChampSim at or under 10% per model; over the limit
  drops the badge to Toy by itself.
- Hold-out: a design reaches Reference-matched only if the hidden score is
  within 10% of the public score.
- Budget: fuzz of the bit-budget allocator; a state over budget is refused.
- Verifier: 50 fuzzed blueprints never pass the time or memory caps.
- No float: grep for `f32`/`f64` in `crates/chipsim` finds 0.
- Tutor: claim equals counter on every workload.
- Potato: `node tools/potato-check.mjs`; wasm size; frame p95 33 ms.
- Look: captures beside references; every look pass LIVE.
- Docs: `node tools/workspace-check.mjs` after each doc edit.

## 9. Session estimate

Rough, sized from the rows (S 1, M 2 to 4, L 5 to 10); revise after the spikes.

- Tier A (SIL1 to SIL19): 40 to 60 sessions.
- Tier B (SIL20 to SIL37): 60 to 110 sessions.
- Tier C (SIL38 to SIL45): 45 to 90 sessions.
- This file: 145 to 260 sessions. The world plan (`silicon-world.md`, SIL46 to
  SIL59) adds 37 to 74. Together about 180 to 330 sessions.
- If the spikes kill the science claim (SIL1 over 15%), the game still ships as
  a teaching and puzzle game; the science half of Tier C drops, the world stays.
