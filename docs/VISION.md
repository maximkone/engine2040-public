# VISION — Rust engine, AI-native tools, convincing 3D

engine2040 is a strong AI-native, AI-agnostic engine on Windows that does all the hard stuff Unity and Unreal do: ragdolls, animation, destruction and heavy multiplayer. Any AI, weak or strong, drives it through one MCP door with no human in the loop and gets the same result. It makes assets inside the game, reuses workflows and mechanics for any hard game, and takes big steals from Rust engines instead of rewriting the wheel. Goal: greater than all of them.

Gives: the Arena demo and target/showcase captures for factory and campaign-studio (the demo waits on GM3); finish.mjs numbers for the honest rival comparison; the MCP door (22 tools) with 12 hostile tapes for forge, skillworks and fp-research; the rust-quality and packet-gate patterns; tested tools in `arsenal.json`.
Takes: design-studio's game-ui kit for HUD and menus; skillworks' part-score tool and tested skills; fp-research attacks on the MCP door and anti-cheat; factory and campaign-studio for the release path; center's kernel and crews; forge's workflows (the same game is built there in Flax).
Finish line: `docs/FINISH-RICH.md` (`node tools/finish.mjs`)
Research tables: `VISION-TABLES.md`

The Core in `FOCUS.md` wins where this file differs (owner 2026-09-30; multiplayer is Core again, owner 2026-10-03).

## Product promises

- **Rust-first:** safe ownership, explicit dependencies, predictable frame
  budgets. WGSL and scene/behavior graphs are data. Narrow, audited native
  adapters are possible; Rust has no stable native plugin ABI.
- **Headless-capable:** simulation, import, cook, command validation and replay
  run without a window, GPU, model account or network connection.
- **Looks like 2040, scales down honestly:** every lighting feature runs on
  the GPU, not as CPU reference math. Hardware ray tracing (shadows, many
  lights, dynamic GI with no bake, reflections) and a path-traced reference;
  volumetrics; skin, eyes and hair; a cinematic camera. Quality tiers Base
  (raster), RT and Ultra pick per GPU, and each tier is measured, never
  assumed. See `VISUAL-TARGET.md` V5. Parked until rows land it:
  engine-owned denoiser (today one comment in `crates/render/src/tier.rs`),
  GPU-driven virtual geometry (2026-10-12: CPU meshlet build landed in
  `crates/assets/src/meshlet.rs` under row VG2, GPU cull/LOD still open,
  rows VG1-VG3 open) and froxel volumetrics (parked 2026-10-09: today 0 hits in `crates/`,
  rows VX1-VX2 open).
- **Multiplayer, Core (owner 2026-10-03: "engine2040 need multiplayer heavy",
  `FOCUS.md` Core 4):** server-authoritative multiplayer over the internet (QUIC,
  relay and NAT traversal) and LAN, up to 64 players plus AI per dedicated server,
  with rollback, interest management, lag compensation and replayable matches.
  Rows MP and ON; the proof is the arena duel played by 2+ people with the same
  hash on every PC and in replay.
- **General engine, three games (owner 2026-09-24, 2026-10-01):** engine
  systems, tools and contracts assume no genre; any genre can be built on it.
  Three games ship on it, in this order: Rome finishes A first (`FOCUS.md`);
  the second is the urban war shooter growing into MODERN NATIONAL, a national
  war in the Enlisted style — a squad of 8 with fireteams, sector capture and
  artillery call-in, ruined-city destruction done hard, reference-look only with
  no real photos and CC0/PD kits (owner 2026-10-01, `plan/urban-war.md`,
  `plan/urban-war.md`); the third is Primordial, the Stone Age survival,
  building and crafting game whose systems become EMPIRE reusable systems —
  tri-tier energy, knapping (parked 2026-10-09: today 0 hits in `crates/`,
  EMPIRE reusable-systems path, no row yet), tribe labor tick brains and
  recorded talk —
  reusable by the other two (DECISIONS 2026-10-01, owner Level 5). Plus the
  AXIOM AI-training game: a visual world the AI trains on, read as VERSES
  AXIOM / Gameworld (DECISIONS 2026-10-01, `plan/ai-native.md`, GW). Headless
  fixture games prove the generality.
- **Breakable bodies and breakable world (owner 2026-09-29):** a blade that
  meets an arm decides, the same way on every PC and in every replay, whether
  it cuts, where, and what the fighter can still do; a wall that loses a column
  falls by its load path; rope, fire, sand, corpses and debris follow the same
  rule. Two layers: the tick keeps a small exact truth (integers, a 16-byte cut
  event, one snapshot for debris, structure, fire, ties and ground); the GPU
  paints the rich look around it and can never change an outcome. Gore Off,
  Reduced and Full give the same hash. Any AI authors and balances it through
  typed verbs and proves it by replay. Plan: `plan/body-and-break.md` (BX).
- **Living world, parked row (owner 2026-09-30, `FOCUS.md`):** two layers.
  Deterministic brains run the fight tick and replay exactly; live models give
  NPCs talk, memory and goals asynchronously, recorded as actions so replays
  never call a model. Parked with its row kept: no new writer until a Core bar
  needs it; the Primordial tribe labor brains and recorded talk (DECISIONS
  2026-10-01) ride the EMPIRE reusable-systems path, not a new living-world lane.
- **AI builds inside the engine:** a brief becomes a level, a character, an
  encounter and finally a playable slice, through the one command door with
  preview, spatial checks, fork play-tests and undo. See
  `C:/empire/center/EMPIRE.md`.
- **AI-native:** deterministic NPC systems plus a typed observation/action
  contract shared by editor tools, automation and optional model services.
- **Provider-agnostic:** local, remote and mock backends use the same contract;
  no vendor's prompt format, SDK or credentials become a scene dependency. Any
  MCP client can drive the same command service; none gets a second write path.
- **Scene and spatial understanding in the core:** every AI, tool and script
  observes the scene through one bounded brief with relations and findings,
  proposes edits as typed batches or relation-based placement, and passes
  deterministic spatial checks before anything applies. Models give intent;
  engine code owns geometry.
- **Offline useful:** authoring and gameplay retain working manual/scripted
  paths when models are disabled, slow or unavailable.

What makes this a new kind of engine rather than an engine with an AI plugin
— counterfactual forks, causal replay, verifiers as content compilers and
provable provider independence — is set out in `AI-NATIVE.md`.

## Boundaries that make this possible

The authoritative fixed-step simulation never waits for inference, disk IO,
HTTP or GPU readback. Optional live inference runs asynchronously outside that
boundary; accepted actions enter through a validated, tick-stamped input log.
Replays consume the recorded actions rather than calling the model again.
Offline generation/baking remains the default for large assets and policies.

This replaces the old blanket “no neural runtime ever” rule: AI can be a live
game service without becoming an uncontrolled dependency of the tick loop.
AI being core means a first-class engine contract, not model weights in
`engine-core`. Details: `EDITOR_AI_CONTROL.md` and `HASH-LAW.md`.

## Priorities

The Core in `FOCUS.md` `## Core` (owner 2026-09-30) is the single source for
the order: the four proofs (Look, Bodies, AI door, Multiplayer) come first;
a writer row that moves no pixel, hash or timing in the arena duel waits
behind them. Statuses live there, not here.

Speed (OPT, PF) serves them: frame time on the arena scene and edit-to-play
latency, measured every 5 waves by BENCH; a regression is a red gate.

Trustworthy build, dependency boundaries and evidence underpin all four; one
scene that imports, renders, edits, saves, plays and replays is where they are
proved, and an AI-assisted workflow changes that scene safely. The declared
Windows scale is dropped from priorities (platform and shipping are parked per
`FOCUS.md`; rows stay, nothing is unticked).

The flagship is Rome at war in `FOCUS.md`: the arena first, then the city
district, the siege field and the ludus as streamed maps. The engine is done
when that game delivers all four Core proofs — Look, Bodies, AI door and
Multiplayer — on the arena duel, built, edited, AI-assisted, played by 2+ people
and replayed in it, at the V5 look on its declared tier (owner 2026-10-03:
multiplayer is Core again, so "played online and over LAN" is back as the
Multiplayer proof). Windows is the first acceptance target for the packaged
game; Linux runs the headless proofs in CI. Web, mobile, consoles and the mod
store are CUT — never — per `FOCUS.md` CUT and owner 2026-09-29 (DECISIONS:
"UGC and mod store OFF, phone OFF, Windows only"; local mods stay ok, mod-jail).
Business terms remain a separate decision in `ROYALTY.md`; they do not
define the engine architecture.

`ROADMAP.md` owns sequencing. Research catalogs supply candidates, never proof
that a feature is implemented. Dependencies obey `deny.toml`; third-party code
and assets require applicable license/provenance evidence.

## Final picture: what the finished engine looks like (owner 2026-09-28)

This section is the ground the research loop aims at. Every frontier card
(`docs/plan/frontier.md`) and ladder spike (`docs/research/LADDER.md`) names
which line below it moves, or it is dropped.

**A player sees:** the Rome arena at night under 250 torches, day turning to
night with no bake, a 20k-50k reacting GPU crowd, sand, steel and skin reading
right next to the `VISUAL-TARGET.md` references; duels and team fights with
2-16 fighters, AI filling the ranks; then the district streets, the siege
field with breaking walls, and the ludus hub streaming with zero tick
hitches. In the second game — MODERN NATIONAL — an Enlisted-style squad of 8
with fireteams fighting over a sector in a ruined city: a five-officer team
breaching a generated two-floor house at night, flashlights and doors and
shouts, every room cleared and graded on the rules of engagement; sector
capture with tickets and waves, an artillery call-in, progressive structure
damage and rubble turned to nav — all in a reference look with no real photos
(CC0/PD kits). In the third, Primordial, a Stone Age band knaps flint, builds
a shelter and feeds the tribe through the same reusable survival, building
and crafting systems, which Rome and MODERN NATIONAL also draw on; the AXIOM
game trains AI inside these worlds (DECISIONS 2026-10-01). All three games
downloaded, installed and played on Windows, and the arena demo — the
stranger-15min proof — in the player's hands within 15 minutes; replays play
back exactly.

**The fight and the world break for real:** a sword takes a forearm at the
wrist on an unarmored arm and skids off the manica beside it; the shield drops,
the fighter staggers, crawls or yields, and the limb lies on the sand as a real
body that others trip on; arterial blood follows the bleed rate and the crowd
reacts. In the siege, a ram splits a gate along its grain, a cut winch drops the
portcullis, a burning beam brings down the floor above it, and the same collapse
plays identically on 64 clients and in the replay. The urban game has walls that
carry weight, doors that splinter by material and fire that spreads through a
ruined block. One build ships to every region: gore Off, Reduced or Full.

**A developer sees:** a newcomer building their first fight in 15 minutes or
less; a brief becoming a playable slice 8 times out of 10 through the one MCP
command service, with preview, spatial checks, fork play-tests and undo; 20
editor tasks passing at 90% or more on mock, local and remote providers with
the same result; every public item documented with an example; a clean
packaged build from one command.

**Our own agents see:** the engine as the reference spatial engine for AI — a
public benchmark of 20 editor tasks plus the spatial suite
(`crates/playtest/src/spatial_eval.rs`) that any MCP client can run, with our
scores printed beside the three rivals' MCP doors — Unreal (5.8 experimental
MCP server), Unity AI and Godot MCP — on the same mirrored tasks (HS20), each
rival's claim held UNVERIFIED until ES31 reads those docs, never a number we
invent. Two legs no rival prints today: the weak-model leg (the same task, the
same result on a weak model) and the replay leg (the same result with no model
at all, from the recorded tape). The spatial eval score and provider
parity print every 5 waves and go up; frame time and edit-to-play latency
print every 5 waves and go down.

## Final bars (TARGETS — only `tools/finish.mjs` says what is met)

| Area | Bar | Proof |
|---|---|---|
| Arena slice (Finish A) | 5-10 min multiplayer Roman arena, server-authoritative, 2 or more over LAN plus AI fill, ragdoll, gore, baked debris, anim, sound, crowd, save, replay | packaged `arena.exe`, play log |
| Look (Finish A + C) | Base 1080p60 on the RTX 3050 laptop; RT tier 60 fps goal from 720p internal or lower with denoiser; 250 torches at night; day-night with no bake; RT reflections; path-traced stills within tolerance; 100M triangles with no pop in motion; 20k-50k GPU crowd | Q1 captures on O5 beside `VISUAL-TARGET.md`, 8/10 hero shots passing the review panel |
| Online (Finish C) | invite through relay and NAT traversal; 64 clients converging under 150 ms and 2% loss; reconnect under 5 s; 1 h with zero desync | ON4 soak log |
| Rome beyond arena (Finish C) | ludus, district and siege streaming with zero tick hitches; 10-minute 64-fighter siege replays exactly | replay diff empty |
| Feel | input to effect p95 50 ms or less; slide under 1 cm; hit-stop, reaction, sound and trauma on the same tick; feel target = Valhall: Harbinger directional combat (owner 2026-10-01): stance-hold clash counts as parry, directional shield block, riposte tempo, rage, flanking AI — deterministic and replay-exact in the arena duel | feel suite plus the arena-duel replay diff |
| Body and break (Finish A, B) | a gladius sweep severs a bare forearm and cannot pass a manica under one table; the fighter drops, crawls, yields or fights one-handed; a 3-story 400-element building collapses by load path, identical across replay and rollback; ropes cut and drop their load; fire spreads by material and weakens the beam it burns; ground keeps craters and traction; gore Off, Reduced and Full give one golden hash; cut event 16 bytes, 10,000 seeded cuts byte-equal x2 on two PCs, at most 0.15 ms per cut, at most 0.8 ms added to the 64 + 500 server tick, 0.3 ms per cut on screen with 32 stumps live | crates/proof/src/break_corpus.rs (BX28), BX perf workload (BX29), Q1 captures on O5, 64-client soak with 8 cuts per second for 1 h |
| AI breaks and balances it (AI in editor) | an agent authors a destructible wall and a severable creature from a brief through typed verbs (`anatomy.set`, `material.set`, `structure.analyze`, `hit.simulate`), every edit undoes, BreakBench 20 pinned tasks (BX25) printed every 5 waves with the same result on mock, local, remote and one external MCP client, printed as parity beside the run rival tool column (G6) — no pass mark claimed until BX25's fixed denominator runs | BreakBench (BX25) printed every 5 waves, tool ledger parity |
| Speed | CPU p95 8 ms or less Base; GPU p95 14 ms or less; 1000-agent sim p99 2 ms or less; zero steady-state allocations; cold load 5 s or less; server p99 4 ms or less at 64 plus 500 AI | BENCH every 5 waves, regression is a red gate |
| AI in editor | brief to slice 8/10 or more; every edit undoes; fork rejects the balance-breaker | EV3-style run, 20 tasks across mock, local and remote |
| AI in gameplay | no AI shot without a perception record; room clear only when every walkable cell was seen; orders obeyed in 99% of fixtures or more | UW fixture on 3 seeds, replayed twice with empty diff |
| Urban war slice (Finish B) | packaged `urban.exe`: 3 missions solo with 4 AI plus 5-player co-op, ROE grading, planner, career; co-op replays exactly | UW39 proof |
| Ship | clean Windows exe, settings, remap, minimal access and localization, local crash reports, size budget | GM3 gate |

Reference hardware stays the RTX 3050 laptop (O5): the RT tier must look
stunning there; Ultra stays designed until the owner names the second
reference GPU. Taste calls go to the review panel (judge plus pilot through
bots and captures, scored against the brief); the owner may overrule later
and nothing waits on him.

## What stays undecided (OPEN — do not build yet)

Campaign including co-op, the full crafting tree (repair-craft lite is the
only kept half), and runtime neural texture compression stay OPEN per
`FOCUS.md`: no rows, no code, no dependencies. Offline encode of textures may
land earlier. A proposal that moves one of these to the board needs a written
case and the owner's word (level 5). Owner 2026-10-01 (DECISIONS, Level 5):
full crafting, survival and building as Rome content stay OPEN, while as game
3 (Primordial) and EMPIRE reusable systems they are approved and moving — see
the Anti-vision note below.

## Anti-vision (CUT — never)

XR/VR, mobile and 2D lanes, WASM ship, console cert, crossplay, autoscaling
fleets, UGC marketplace, liveops and seasons, analytics, text chat infra,
social sim, RTS/TD/BR/racing/platformer genre systems,
naval, kernel anticheat, seamless open world, full tutorials and subtitles:
`FOCUS.md` CUT governs. Building-full, survival-full and crafting-full moved
out of this never-list on owner 2026-10-01 (DECISIONS, Level 5: "CUT
survival-full/building-full/crafting-full/npc-schedules/farming-sim move to
EMPIRE reusable systems"): they land as game 3 (Primordial) EMPIRE reusable
systems — tri-tier energy, knapping, tribe labor tick brains and recorded
talk — using the same integer cut, anatomy, physics and AI tables as the
gladius and the flint spear, so Rome and MODERN NATIONAL reuse them; they
never become Rome content. Settlement and faction wars stay PARKED behind
Finish A and UW0. CUT still limits what the Rome game contains, never what
the engine can do; a CUT genre may appear only as a headless fixture proving
generality. No programmer art in a frame a player sees, ever.

## Full engine depth: the worlds and workflows we intend to support

The ambition is an integrated engine whose hard systems remain inspectable and
controllable by any AI. The atlas in `plan/frontier.md` maps each layer to its
existing plan, current gap, next useful consumer and proof. It includes:

- **Bodies and material worlds:** contact-aware controllers; joint-limited
  active ragdolls and physical recovery; skeletal blending, retargeting, motion
  matching and animation LOD; armor-aware wounds and severable limbs; fracture,
  structural load paths, cuttable ties, fire and deformable ground. Cloth,
  hair, blood, smoke and soft-body detail enrich presentation around recorded
  gameplay facts. Authoritative fluids/MPM/FEM remain CUT.
- **A scalable visual world:** measured Base/RT tiers, GPU visibility and
  indirect submission, geometry and texture residency, lighting/GI/shadows,
  denoising, atmosphere/sky/foliage/water, material response, VFX and cinematic
  capture. Neural rendering/compression and splats earn bounded offline or
  presentation experiments under the existing OPEN/CUT rules.
- **Space that agents can reason about:** semantic scene graphs, support and
  reachability, affordances, constrained layouts, conflict repair, dirty-tile
  navigation, formations/crowds and scene memory. 3D search includes geometry,
  asset metadata and motion retrieval; learned encoders are offline/advisory
  research where permitted, never a new dependency of the fixed tick.
- **An AI-neutral creation path:** Blender/DCC source assets, explicit units,
  rig and clip contracts, glTF now and the scoped USD experiment later,
  reproducible cook/cache/provenance, model-neutral tools, bounded jobs,
  cancellation, preview/apply/undo, co-editing and recorded provider-off replay.
  An open prompt is a candidate tested against the same tasks, not engine truth.
- **A complete product:** authoritative online play, prediction/rollback,
  bounded replay/save histories, migration and crash recovery; audio and music
  that players can hear; cameras, text, UI/accessibility and local data mods;
  hot reload, debugging, packaging and a timed first-fight experience. Rome,
  the MODERN NATIONAL war and Primordial exercise the same foundations, and
  the AXIOM training game reuses them. Vehicles remain the kept LIGHT
  fixture; platforms and other game genres do not silently become build work.

Research compares all these layers, including out-of-scope competitors' powers.
Comparison is not permission to build a CUT system. Each accepted improvement
has one consumer milestone and a proof that an outsider, player or agent benefits.

Moved to `VISION-TABLES.md`, same headings, text unchanged: Delivery evidence, Research contract, Scorecard, Look rubric r1, Parts vs the best, Open gaps. The Plans map and the Steal map below stay here (tools read them).

## Plans: which plan serves which part of the vision

Every plan in this repo has one row here with the Scorecard rows it serves;
`node C:/empire/center/vision-check.mjs engine2040` FAILs on a plan that is not in this map, a row whose file is
gone, and a Scorecard row no plan serves (the planner writes that plan). Where
several plans serve one part, the order in the last column is the reading order.

| Plan | Serves (Scorecard rows) | Board prefixes and notes |
|---|---|---|
| `docs/FOCUS.md` | every row: what comes first now (Now, HARDEN, LIGHT, CUT) | read first |
| `docs/FINISH-RICH.md` | every row: the gates Finish A, B, C and P | `node tools/finish.mjs` says what is met |
| `docs/ROADMAP.md` | every row: integrated sequencing and prerequisites | canonical order; current status comes from finish proof |
| `docs/AI-NATIVE.md` | AI builds the game; Spatial understanding for AI; AI in gameplay | architecture and differentiating contracts |
| `docs/VISUAL-TARGET.md` | Look on the RTX 3050 | tier-specific references and visual acceptance |
| `docs/plan/README.md` | none: how a plan is written | the template and the prefix table |
| `docs/plan/ATLAS.md` | none: the generated map of every plan, its rows, crates and health | `node tools/plan-atlas.mjs --write`; never hand-edited |
| `docs/plan/steals.md` | none: the scout seat's list of steals that land on a finish bar, open until landed | lines, not rows |
| `docs/plan/seats.md` | none: the 40-seat sprint's seats, inputs, outputs and first seeds | loop staffing (`.opencode/knobs.json` seat knobs); AW loop tools |
| `docs/plan/frontier.md` | Spatial understanding for AI; AI builds the game; Look on the RTX 3050; Sim speed | the five research pillars; FU spikes and NS north-star rows (futures and NORTH-STAR merged in); LADDER rungs |
| `docs/plan/render-look.md` | Look on the RTX 3050; Shipping a clean build | RL, RP, RA, GX, VS, WIN, GM (arena wiring and window and packaged run merged in); read 1st for Look |
| `docs/plan/content.md` | Look on the RTX 3050; The three games; Shipping a clean build | CT, AS (cook and delivery merged in) |
| `docs/plan/animation.md` | Look on the RTX 3050; AI in gameplay | AM, LM, TV (LM2 ground-feel note) |
| `docs/plan/runtime.md` | Sim speed; Determinism and exact replay; Headless proofs and tests | RC, ARC (spine), GE (genre-neutral ledger) |
| `docs/plan/quality.md` | Headless proofs and tests; Sim speed; Shipping a clean build; Online, 64 players; Determinism and exact replay | PF, AH, SL, CQ, OPT (build and runtime speed), FB (dependable receipts) |
| `docs/plan/potato.md` | Look on the RTX 3050; Sim speed; Shipping a clean build; Headless proofs and tests; Determinism and exact replay | GS (wiring gates, potato tier and gate G9, content fetch, small steals); read 1st for weak hardware |
| `docs/plan/landscape-ideas.md` | none: the research idea backlog, promoted to GS rows by the planner | LI ideas, not rows |
| `docs/plan/body-and-break.md` | Dismemberment and wounds; Destruction: structure, fracture, fire; AI breaks and balances it; Physics and destruction | BX, PX, DX (physics base merged in); read 1st for Bodies |
| `docs/plan/hard-systems.md` | AI builds the game; AI in gameplay; Determinism and exact replay; Headless proofs and tests; Sim speed | HS (AI-door 20 adversarial briefs in section 8); solvers, motion, learned rendering, agents, gym, benchmarks, verification |
| `docs/plan/multiplayer.md` | Online, 64 players; Determinism and exact replay | MP, ON (internet play merged in); read 1st for online |
| `docs/plan/living-world.md` | AI in gameplay; The three games; Online, 64 players; AI builds the game | LW, AB, LAI (live-AI half of platform-live-ai merged in) |
| `docs/plan/ai-native.md` | AI builds the game | AX, MCP, EA, EV (eyes merged in), GW (axiom paragraph) |
| `docs/plan/spatial.md` | Spatial understanding for AI | SP |
| `docs/plan/gameplay.md` | AI in gameplay; The three games | GP, GA, NV, CB (brains and navigation merged in); read 1st for the games (Rome first, MODERN NATIONAL second, Primordial third) |
| `docs/plan/ai-door.md` | AI builds the game; AI in gameplay | AL, DP, RB, LF16, HS20, AID1 (the five FOCUS Now AI-door slices plus the describe ledger); read after `ai-native.md` |
| `docs/plan/fps-view.md` | Look on the RTX 3050; The three games | FV (a first-person eye on a fighter body: eye pose, look channel beside the hashed tape, camera feed, no-clip rule; consumer UW11) |
| `docs/plan/parked.md` | Editor and tools; The three games; Spatial understanding for AI | WD, SN, ED, PLF, UIX (world, audio and editor parked; each names the proof that un-parks it) |
| `docs/plan/urban-war.md` | The three games | UW, UC (urban-city merged in); frozen by FOCUS D7 |
| `docs/plan/rome-spec.md` | The three games; Determinism and exact replay; AI in gameplay | Rome arena duel merge point with forge (map, bouts, tick rate, seed rule, AI fill); machine truth in `rome-spec.json`; open parity gaps digest |
| `docs/plan/finish-line.md` | Look on the RTX 3050; Sim speed; Determinism and exact replay; AI builds the game; Online, 64 players; Shipping a clean build; Headless proofs and tests | gates G1-G9 product check |
| `docs/plan/forge-adoption.md` | AI builds the game; AI in gameplay; Spatial understanding for AI; Dismemberment and wounds; Physics and destruction; Look on the RTX 3050; Determinism and exact replay; Headless proofs and tests; Editor and tools | AD1-AD29, PX40, AX23, EA6, PF24, SP66 |
| `docs/plan/BOOK-GAPS-2026-10-12.md` | Look on the RTX 3050; Sim speed; Determinism and exact replay; Online, 64 players; Headless proofs and tests; Shipping a clean build | W1-W18 + S1-S20 + D1-D6 book review 1; review, no new rows |
| `docs/plan/BOOK-GAPS-2-2026-10-12.md` | AI in gameplay; The three games; Look on the RTX 3050; Sim speed; Headless proofs and tests; AI builds the game | N1-N40 book review 2, boarded as GP27-GP42, GA13-GA15 and SPRINT-20-game.md rows; review, no new rows |

## Steal map (scouts: what we read, oldest first)

The engine's steal seats and the vision researcher take the row read longest
ago (`never` first), one deep read per run, and write its date back. At least
10 competitors and 10 adjacent sources; `node C:/empire/center/vision-check.mjs engine2040` counts them. No
crate work before its consumer exists: every accepted steal names the consumer
that wires it.

| ID | Kind | Sources | Part | Question | License (read live) | Last read |
|---|---|---|---|---|---|---|
| ES01 | competitor | bevyengine/bevy | sim core, app | How do schedules, change detection and the asset processor stay fast and deterministic enough? Read 2026-10-07: parallel executor runs systems by declared access, max parallel, safe; Changed/Added query filters plus Res.is_changed; table (fast iter) vs sparse-set (fast add/remove) storage | MIT/Apache-2.0 (LICENSE-MIT + LICENSE-APACHE read live 2026-10-07) | 2026-10-07 |
| ES02 | competitor | godotengine/godot (`core/object/undo_redo.cpp`, scene tree, SDFGI) | scene, look | Undo merging, scene inheritance, GI cascades on mid GPUs | MIT (undo_redo.cpp header read live 2026-10-07) | 2026-10-07 |
| ES03 | competitor | FyroxEngine/Fyrox | app (editor) | How does a Rust editor stay usable: scene editor layout, hot reload, plugins | MIT (LICENSE.md + variable.rs read live 2026-10-07) | 2026-10-07 |
| ES04 | competitor | stride3d/stride | app, assets | Asset pipeline and editor split in a mid-size open engine | MIT (LICENSE.md + README.md read live 2026-10-07) | 2026-10-07 |
| ES05 | competitor | o3de/o3de | online, scene | Prefabs and multiplayer components at scale | Apache-2.0/MIT | never |
| ES06 | competitor | Unreal Engine docs (Lumen, Nanite, replication graph) | look, online | What the 2040 look and 64-player relevancy need, as ideas | ideas only | never |
| ES07 | competitor | Unity docs (DOTS, Netcode, AI tools) | sim core, AI control | Data-oriented sim and AI-in-editor features buyers expect | ideas only | never |
| ES08 | competitor | veloren/veloren | online, world | A Rust multiplayer world at scale: server tick, networking, streaming (online is Core again, owner 2026-10-03: read it for the server tick and streaming) | GPL-3.0, ideas only | never |
| ES09 | competitor | AmbientRun/Ambient | online | Rust multiplayer engine design and why it stopped | MIT/Apache-2.0 | 2026-09-30 |
| ES10 | competitor | EmbarkStudios/kajiya | look | Real-time GI in Rust on consumer GPUs | MIT/Apache-2.0 | never |
| ES11 | competitor | AI-in-editor products (Roblox Assistant, Unity AI, Godot AI plugins) | AI control | What an AI-native editor must do on day one | ideas only | never |
| ES12 | adjacent | dimforge/rapier | sim core (physics) | Cross-platform determinism and destruction; the physics decision record (gap G3) | Apache-2.0 | 2026-09-30 |
| ES13 | adjacent | recastnavigation/recastnavigation, ikpil/DotRecast | sim core (nav) | Tiled navmesh rebuilds for destructible arenas | zlib | 2026-09-30 |
| ES14 | adjacent | gschup/ggrs | online | Rollback on a fixed deterministic tick | MIT/Apache-2.0 | 2026-09-29 |
| ES15 | adjacent | cBournhonesque/lightyear | online | Prediction, interpolation and interest management | MIT/Apache-2.0 | 2026-09-29 |
| ES16 | adjacent | BorisTheBrave/DeBroglie, mxgmn/WaveFunctionCollapse | world | Constraint world generation with a seed that replays | MIT | 2026-09-30 |
| ES17 | adjacent | GOAP, HTN and behavior tree libraries (crashkonijn/GOAP, ptrefall/fluid-hierarchical-task-network, zkat/big-brain) | AI in gameplay | Planner shapes, speed and debug views for 500 AI | read live | 2026-09-30 |
| ES18 | adjacent | orangeduck/Motion-Matching, sebastianstarke/AI4Animation | look (anim) | Motion matching quality checks: foot slide, joints | read live | never |
| ES19 | adjacent | KhronosGroup/glTF-Validator, donmccurdy/glTF-Transform, KhronosGroup/glTF-Sample-Assets | assets | Validate and optimize every imported asset | Apache-2.0, MIT | never |
| ES20 | adjacent | gfx-rs/wgpu | look | Backend features and limits we lean on | MIT/Apache-2.0 | never |
| ES21 | adjacent | sourcefrog/cargo-mutants, mitsuhiko/insta | proof and quality | Mutation score and snapshot tests on the sim and scene briefs | read live | never |
| ES22 | adjacent | EmbarkStudios/puffin, wolfpld/tracy | app and speed | Frame profiling that the BENCH gate can read | MIT/Apache-2.0, BSD-3 | never |
| ES23 | adjacent | MineDojo/MineDojo, MineDojo/Voyager, game-agent benchmarks | AI control | Tier C benchmark: tasks, metrics, competitor columns (gap G2) | MIT | never |
| ES24 | adjacent | arXiv: neural rendering, Gaussian splatting, learned upscaling | look | Which paper result survives a spike on the RTX 3050 | papers: ideas only | 2026-09-30 |
| ES25 | competitor | Unreal Chaos Destruction docs (Geometry Collections, fields, clustering) | sim core (destruction) | What breaks, how it replicates, what it cannot replay | ideas only | 2026-09-30 |
| ES26 | adjacent | Teardown and Red Faction Geo-Mod talks (voxel and CSG destruction, structural integrity) | sim core (destruction) | Load paths and collapse that read right at low cost | ideas only | 2026-09-29 |
| ES27 | adjacent | Blade and Sorcery, Overgrowth, Mordhau, Dead Space, The Last of Us Part II dismemberment and gore talks | combat, look | Cut feel, cap and stump looks, authored versus physical cuts | ideas only | never |
| ES28 | adjacent | arXiv and SIGGRAPH: fracture, cutting, sheet tearing, XPBD (Müller 2013, Bao 2007, Pfaff 2014, Macklin 2016) | sim core, look | Which method survives integer authority or belongs to presentation only | papers: ideas only | never |
| ES29 | adjacent | Dwarf Fortress and Rimworld wound and tissue models (wiki and dev logs) | combat, AI in gameplay | Wound depth and injury consequences that play well | ideas only | 2026-09-29 |
| ES30 | adjacent | Blender cell fracture and fracture modifier, Houdini RBD docs | editor | Fracture authoring UX for the break panel | GPL and proprietary, ideas only | 2026-09-29 |
| ES31 | competitor | Unreal Engine 5.8 MCP plugin and Unity AI MCP server docs | AI control | What the two big engines expose to agents today, and what they leave out (auth, verify, replay) — the source behind every UNVERIFIED MCP claim in this file; read it before any 5.8 MCP number exists | UNVERIFIED, ideas only, never read | never |
| ES32 | adjacent | GameDevBench, CraftBench-UE, GameLogicBench, GBQA, GameCraft-Bench (papers and repos) | AI control | Task shapes and scoring we mirror for HS20 | UNVERIFIED, read live | never |
| ES33 | adjacent | newton-physics/newton, savant117/avbd-demo3d | sim core (physics) | VBD and AVBD solver design for presentation cloth and flesh | Apache-2.0 per project page, MIT read 2026-09-29 | 2026-09-29 |
| ES34 | adjacent | jrouwe/JoltPhysics release notes and determinism discussions | sim core (physics) | Cross-platform determinism bugs and their fixes | UNVERIFIED, ideas only | 2026-09-29 |
| ES35 | adjacent | arXiv: motion diffusion, behavior foundation models, world models (ARDY, DartControl, BFM-Zero, Matrix-Game 3.x) | look (anim), AI in gameplay | Which result survives an integer, replay-exact tick | papers: ideas only | 2026-09-29 |
| ES36 | adjacent | model-checking and proof tools for Rust (Kani, Miri, loom) | proof and quality | Bounded proofs of Q16 math and quantizers | UNVERIFIED, read live | 2026-09-29 |

Card (every steal, in the repo's own card place): Source (repo@sha `path:line` or URL) and
license | What it does | Home (an existing file here; no home = reject) | Fixes (the part and
its axis: size, strength, speed, agent success, look, money) | Net lines (wc -l both sides) |
Proof (the test, fixture or command; no proof = reject) | Effort S/M/L and risk. At most 5 cards
a run plus a reject list; ideas only from GPL, AGPL, proprietary and reference-only sources.
The research merge seat judges every card (dupes out, scored on net lines, fit, proof, effort),
accepts the best into the backlog by step, and keeps the rejects.
## How research aims at this file

- Every card names one of the five frontier pillars and one line of the Final
  picture or Final bars above; `vision-fit` with neither is REJECTED.
- Spikes run on fixtures against a baseline with numbers; paper-only never
  becomes a row. Only moves up the ladder
  (CARD to SPIKE to ROW to LANDED to BAR, or REJECTED with reason) count.
- Tier A (next 10 waves): at least 1 spike per wave from the top ACCEPTED
  card, 5 spikes with numbers against baseline, 3 becoming rows, 1 landed
  moving A4, P1 or C1-C3, with R1 and R2 pixels first.
- Tier B (30 waves): each pillar lands one measured frontier feature.
- Tier C: the engine is the reference the benchmark names, with the spec
  (tasks, fixtures, metric, competitor columns) published beside the scores.
