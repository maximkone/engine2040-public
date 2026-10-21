# Gameplay, game design and brains (Game, Bodies, AI door)

Outcome: the duel plays like a designed game, not a tech demo: every swing commits and reads, every attack has an answer, opponents climb in tiers a player can feel, the arena has places that matter, and AI fills empty slots fairly on ground that breaks.
Proof: (1) input to effect p95 at most 50 ms, hit-stop, reaction and sound on one tick (PQ1); move tables versioned, swaps validated; (2) same seed and tape give the same intents, paths and steering, x2; (3) NV4 and NV6 bars; (4) design bars (section 2, Game): each enemy tier beats the one below in 60%+ of 200 seeded duels, no move above 35% of kills, a new player reaches the first bout within 45 s of launch, capped at 60 s (`content/design/ladder.rows:13-15`), every live attack has a tell of 12+ ticks and an answer.
Order: FOCUS Now group 8 "Game" (owner 2026-10-12) holds GP27-GP42 and GA13-GA15 in its order; then GP18, GP21, NV4, NV6, GA7, GA8, then the section 7 table.
Crates: combat, game, input, ai, nav, arena, playtest, ui (duel HUD only).
Open board IDs: GP18 GP21 NV4 NV6 GA7 GA8, the new game rows GP27-GP42 GA13-GA15 (board shard `docs/SPRINT-20-game.md`), and the table in section 7. GP24 is not on the board (dropped). Landed 2026-10-13: GP28 GP29 GP30 (4c6d817); code-built ahead of tick: GP27 GP31 GP37 GP39, partial GP41 (see section 1). Data-built ahead of tick 2026-10-13: GA13 part (`enemies.rows` plus ai `TierRow`, no tournament hook), GP36 part (`arena-zones.rows`, no metric), GP40 part (`loop.rows` glory only, no beat chart).

Tag NOW (owner 2026-10-12: "i bought much more books ... a lot of game design", "need roles proper too for full game dev teams"). Prefixes: GP, CB (gameplay and design); GA, NV (brains and navigation). Plain summary: swords hit, shields block, tables version the moves; parry, riposte, clash, swept arcs, a deterministic camera and a ladder all exist as code. What was missing is the design layer the books teach: a written vision, a core loop with a resource ledger, combat rules that make attacks commit and read (buffer, commit lock, poise, per-kind hit-stop), opponents in tiers with a difficulty curve, an arena with walls, zones and entrances, a ladder paced by a beat chart, a HUD, and playtests that ask a question. Brains and navigation are unchanged below.

Sources (2026-10-12): `BOOK-GAPS-2026-10-12.md` (Gregory, Lengyel, Modern Game Testing) and `BOOK-GAPS-2-2026-10-12.md` (rows N1-N40: Gregory 3rd ed., Eberly, Zerbst, Orland, Wolverson, Matthews, Richards-Ford, Harmel-Law, Sekar, Versluis, Bycer *Soulslike*, Sellers *Advanced Game Design*, Adams-Dormans *Game Mechanics*, Rogers *Level Up!*, Killick, Prokhorenko, McLean-Sharp, Vazhenich *Worlds*, Short-Adams and Shaker-Togelius-Nelson on procedural generation, Wang, Perry, Trefry, Mitchell, Mandeville). Team seats that work this plan (center `crews/engine2040/standing/`): planner-director, planner-producer, builder-design-tech, builder-designer (combat and feel), builder-design-enemy, builder-design-systems, builder-design-level, builder-design-ux, builder-qa-lead, builder-wire.

Merged 2026-10-10 (batch A of the plans review): `docs/archive/2026-10-10/plans/game-ai.md` as `### brains and navigation`. Cut then: dated Status lines, the subsumption experiment (GA4, killed), the donors and the session estimate, in `docs/archive/2026-10-10/plans/gameplay-cut.md`. Pre-merge text: `git show d96091cd:docs/plan/gameplay.md`.

## 0. The game on one page (design vision, 2026-10-12)

The living copy is `docs/design/CONCEPT.md` (planner-director seat); this section is the plan-level summary and wins on conflict until the owner edits either.

- **Concept (under 140 chars):** Gladiators duel in a lit Colosseum; every swing commits, every hit reads, and any AI can play, judge or build the next bout.
- **One question** every feature must answer yes: does it make the fight more readable or more decisive?
- **Pillars** (a row that serves none is cut):
  1. *Commitment.* Do: an attack is a promise (a visible windup, a recovery you can punish). Don't: cancel out after the hit tick, mash to win.
  2. *Readability.* Do: a player can say why they lost (tell, hit-stop, sound cue, HUD state). Don't: hidden numbers, random damage, effects that hide a tell.
  3. *Answers.* Do: every attack has at least one answer (block, parry, dodge, spacing). Don't: an unanswerable move, a dominant option (no move above 35% of kills).
  4. *Exact.* Do: same seed and tape, same bout, on every PC and in replay; AI fills slots fairly. Don't: float in the tick, rubber-band cheats.
  5. *Spectacle serves information.* Do: crowd, camera, blood and light show who leads and what just happened. Don't: spectacle that costs a frame of the tell.
- **Core loop (one exchange):** start state (two fighters 8 m apart, full hp, stamina and poise) -> action (close distance, attack, defend, dodge) -> result (damage, stagger, stamina spent, a parry punish) -> changed state (hp, stamina, poise, round clock, round wins). Session loop: round (600 ticks, best of 5, `rome-spec.md`) -> ladder rung -> glory rank -> the next opponent tier.
- **Slice decision:** vertical. The arena duel is finished to the bar (look, feel, opponents, HUD, round loop) before district, siege or ludus get a design row (FOCUS "## The game" order stands).
- **Not in this slice** (logged with who and why, Prokhorenko ch4): economy, progression trees, crafting beyond repair-craft LIGHT, stealth, mounts. Creative cuts are the director seat's; business cuts are Maxim's.

## 1. Where it stands

Landed and ticked (kept out of the table; the board and git keep them): CB1
table-driven fight step, HR3 versioned move tables, CB2 gore presentation, IN1
bindings, TU1 tuning panel, HR5 practice mode, AR1/AR2 arena match, GP1 parry and
riposte, GP2 swept detection, GP3 camera director, GP4 ladder and glory, GP5
second-game proof, GP11 feel schemas, GP12 balance harness, GP15 and GP17 wiring
and content gate, GP19 staged rig, GP20 damping split, GP23 phased tables.

| Capability | Status | Evidence (file or commit) |
|---|---|---|
| Design vision, pillars, core loop | NEW (section 0) | `rg "concept statement|core loop|design pillar" docs/plan` was empty (N27) |
| Design data a seat can change without Rust | LANDED (GP27) | loader `crates/combat/src/tables_text.rs:176`, `pub mod tables_text` in `crates/combat/src/lib.rs:79`, `content/design/combat.rows:7-9`, `docs/design/FORMAT.md` |
| Input buffer | LANDED (GP28, 4c6d817) | v2 buffer queued at intake `combat/src/fight.rs:1010-1032` (`feel.rs:305` `buffer_window`), tests `fight.rs:3611-3678` |
| Commit lock on dodge | LANDED (GP29, 4c6d817) | v2 lock `combat/src/feel.rs:237-262`, dodge refusal `fight.rs:1096`, tests `fight.rs:1727-1880` |
| Poise and per-kind stamina, hit-stop | LANDED (GP30, GP31) | `poise_damage_for_kind` `combat/src/fight.rs:211`, `stamina_cost_for_kind` `:232`, `PoiseMeter` `combat/src/depth.rs:208`; per-kind freeze `crates/combat/src/hitstop_kind.rs:23,25,37` into `fight.rs:268` (GP30, 4c6d817; GP31, 624938f5) |
| Readable startup in the live match | LANDED (GP32) | V2 rows exist (`crates/combat/src/tables.rs:134`); the arena picks the table per match (`crates/arena/src/lib.rs:207,492,523`): the window and capture builds pass v2, the proof scene keeps v1 (N13, GP32) |
| Enemy tiers and a boss | PARTIAL (GA13 landed; GA15 open) | four tier rows in `content/design/enemies.rows:23-26`, `SHIPPED_TIERS` `crates/ai/src/depth.rs:440`, `parse_tiers` `:571`; the boss move set and its two phases are still open (N17, GA13, GA15) |
| Difficulty curve, human error model | PARTIAL (GA14 open) | the sweep and the 400-600 permille band live in `playtest/src/tournament.rs:38,43-47`; the seeded 2-4 tick human noise is `playtest/src/winrates.rs:45-49`; the strict rung-strength rise check is still open (N18, GA14) |
| Matchups and time-to-kill | PARTIAL (GP33 open) | a hand-written per-pair table ships in `docs/design/matchups.csv:12-21`; the `ttk` test and the odd-cycle check do not exist yet, and the table still carries the old sword numbers (N23, GP33) |
| Second balance harness | LANDED (GP34) | the bench mirrors the tournament bar (`game/src/bench.rs:69` `PQ2_KILL_SHARE_MAX_PERMILLE` 350); the caller is `game/src/bin/staged_fight.rs:499,507` (N22, GP34) |
| Arena walls in the match | LANDED (GP35) | the 11 physical kit actors become 6 floor boxes (`crates/arena/src/kit.rs:111,260-265`) and a match loads the kit world (N19, GP35) |
| Zones, cover, entrances | LANDED (GP36) | `content/design/arena-zones.rows` seats the centre (`:16`) and the gate mouth (`:9`); the tournament prints the zone shares and holds `entrance_share` under 250 permille (`crates/playtest/src/tournament.rs:1980`) (N20, GP36) |
| One Rome numbers file | LANDED (GP37) | `content/arena/rome-spec.json:2` half extent 10 m, drift test `crates/game/tests/it/rome_spec_drift.rs` wired in `main.rs:27` |
| Round and match loop | LANDED (GP39) | best-of-5 `crates/game/src/rounds.rs:1-8`, verdict `decide_round` `:24-37`, app fold `crates/game/src/lib.rs:790,981` |
| Duel HUD | LANDED (GP38) | `ui/src/lib.rs:855 DuelOverlay` is built for the app in `crates/game/src/windowed.rs:1091-1096` (N5, GP38) |
| Ladder pacing | LANDED (GP40) | the beat chart is `content/design/ladder.rows:25-32`, the gate is `tools/beat-lint.mjs`, the compiled chart is `game/src/ladder.rs:290` `Ladder::beat()` (N28, GP40) |
| Playtest that asks a question | LANDED (GP41) | charter runner `crates/playtest/src/charter.rs:95-127`, C1 ships `crates/playtest/charters/c1.toml:1-6`, C2 ships `crates/playtest/charters/c2.toml:1-6` (`charter.rs:158-174`) |
Status 2026-10-21: both halves of GP41 hold. The board says done: `node tools/board-slice.mjs --find GP41` prints one row, `GP41 done Game ... GP21 accepts on Landed 06134105`. C2 is on disk: `crates/playtest/charters/c2.toml:1-6` ships id c2 with question `a light hit shakes the camera at most a quarter of a heavy`, `crates/playtest/src/charter.rs:158` `C2_SRC` `include_str!`s that file, `:161 load_c2`, `:166 run_c2`, `:174 light_within_quarter` and test `:316 charter_c2_parses_and_prints_pass` cover it, so the shipped C2 question is camera shake, not camera separation; the C1 half still holds (`crates/playtest/charters/c1.toml:1-6` ships, `charter.rs:94-127` is `parse_charter` plus `evaluate`).
| Debug camera, pause and step in the game binary | LANDED (GP42) | the binary takes `--pause-at-tick <N>` and `--step` (`crates/game/src/bin/arena.rs:219-226`), plus `--camera-path` (`:3426`), and the free-fly camera flies the eye by hand (`crates/game/src/camera.rs:145,373`); the editor keeps its own lanes (`editor/src/play_in.rs:131-164`) (N31, GP42) |
| Camera feel cores (GP8) | open, board | flax-mcp camera rig and damping cores, MIT; needs GP3 |
| Combat depth cores (GP9) | partly on disk | `combat/src/depth.rs:202-275` PoiseMeter exists: GP30 wires it instead of a port |
| Juice pipeline, gamefeel port, feel-flow (GF1-GF3) | open, plan-only | LF10 hitstop hold, LF11 shake and GP31 cover the Rome need first |
| Feel quantification and keypoints (GP6, GP7) | ideas only, board | licences UNVERIFIED, no code |
| Impact instrumentation (GP13), third-person framing (GP14) | GP14 on board, GP13 plan-only | |
| Delta-battle patterns (CB3) | open, board | hit-stop, cancel and reaction patterns, MIT |
| Generated-move intake (HR6) | open, plan-only | CT14 clip to HR3 move; VK1 and PQ2 decide |
| Subsumption melee stack (GA4) | experiment killed with numbers | spike cost 2.00x against a 3x bar, win-rate neutral |
| Near-field ORCA (NV4), legion platoons (NV5), smooth funnel paths (NV6) | missing (rowed) | no `orca.rs`, `platoon.rs`, `funnel.rs` |
| Crowd neighbour search | all-pairs | `nav/src/crowd.rs:768` passes the full slice, `pick_nearest` `:309,605` loops all, `MAX_AGENTS` 256; a tested grid/SAP/BVH sits in `physics/src/broadphase.rs:8,43,72` with test callers only (N16: NV10 row change) |
| Shared-edge tile stitching (NV12), temp-obstacle queue (NV13) | missing, boarded | |
| Diverse offline students (GA7) | open (rowed) | RES-ARXIV diversity students over BR2; no GPU in the loop |

Module notes: moves are versioned table rows with digests joining the rules
version (HR3); the per-tick resolution order is frozen with fighter-ID tiebreaks;
hit-stop freezes fighters in-sim while shake and flash stay presentation; `ai`
never names Rome (`depth.rs` owns Rome behind `ArenaPolicy`); `nav` is std-only,
integer mm inside, ties by stable id; the arena owns the chunk-to-tile map;
LOD switches on camera distance only, never frame timing.

## 2. Finish line

- **Tier A (alpha, fights with answers):** every attack has a timed answer on
  scripted tapes; thin fast weapons never tunnel by speed fixtures; scripted
  2-fighter orbits keep both fighters framed for 600 ticks; trauma replays exactly.
  Landed: GP1, GP2, GP3, GP15, GP16.
- **Tier A, Game (owner 2026-10-12, FOCUS Now group 8):** design numbers live in
  `content/design/*.rows` and load through one validated parser (GP27); the live
  duel runs tables v2 with an input buffer, a commit lock, poise and per-kind
  hit-stop (GP28-GP32), while the golden proof scene keeps v1 and
  `d57b5c8ae3096614`; kit walls stop fighters and the arena has measured zones
  (GP35, GP36); one Rome numbers file both builds read (GP37); a round is 600
  ticks, a match best of 5 with tie replays (GP39); the duel HUD is drawn
  (GP38); the debug camera, pause and step exist in the game binary (GP42).
- **Tier B (v1, a game around the fight):** three enemy tiers, each beating the
  one below in 60%+ of 200 seeded duels and losing to the one above (GA13); a
  difficulty sweep with a human-like error model shows rung strength rising
  strictly (GA14); a time-to-kill and matchup table with no dominant row (GP33);
  a beat chart lint passes the ladder (GP40); playtest charters ask one question
  each and file A/B/C bugs (GP41); the primus boss has its own move set and two
  phases (GA15); ladder runs start to glory-rank with the craft hook firing once
  per loop; PQ1 p95 at or under 50 ms; PQ2 win-rate 40-60% with no move above 35%
  of kills. Open here too: GP18 (baseline lock), GP22 (impact fan-out), GP21
  with a charter instead of "playtester confirms".
- **Tier C:** none. Imported fighting-genre frame data stays CUT; the arena uses
  its own tables (startup, active and recovery ticks are the arena's own rows).

CUT (never): skilltrees-full, crafting-full, economy-full, progression-full,
stealth-full, survival, building and BR systems, naval, imported frame-data
tables, GPL combat code, donor moves or tables. A per-fight reward and a
penalty beside it are not economy (N25): they stay in the ladder table.

### brains and navigation

- **Tier A alpha (fights stay unstuck while the ground breaks):** NV3 dynamic
  tiles, flow fields and NV7 version-stamped paths have landed. What remains is
  NV4 ORCA near-field, NV6 smooth funnel, NV8 soak harness and the first live
  binding (GA8). Bar: a broken wall opens a path within 1 s of ticks; 500 agents
  steer under 1 ms; paths never longer than the grid path and at least 10%
  shorter on 80% of queries; a shield wall holds in fixtures; bytes x2 on every
  bake, query and steering batch; one arena fixture drives a fighter through EQS
  plus DepthBrain with replay x2.
- **Tier A-scale (this laptop plus P6):** NV10 broadphase, NV12 stitch proof and
  the 1,000-agent soak leg. Bar: 1,000-agent sim p99 at most 2 ms; steering at
  most 0.1 ms per agent flat from 50 to 1,000 agents; cross-tile paths byte-equal
  to single-mesh paths; server tick p99 at most 4 ms at 64 players plus 500 AI
  with interest on (ON10 bar). The GPU never touches the tick.
- **Tier B v1 (legions march cheap):** GA2 legion brain has landed (500 soldiers
  under 1.5 ms of AI, x2). Open: NV5 platoons and the GA5 thinking-LOD audit. Bar:
  at least 5x fewer nav queries for a 500-soldier legion with no accordion at
  60 Hz; replay x2; a legion-vs-legion fixture (GA9) shows orders and flanking
  deciding a 500-vs-500 battle.
- **Tier C (learned opponents):** offline distillations through the BR2 path as
  integer `.e2nn` nets (the DT and NN rows own training); adaptation fenced per
  ladder rung and disclosed. Bar: the student-vs-script tournament reproduces
  twice; disabling adaptation freezes the rung default.
- **CUT:** learned steering weights inside the tick; live RL in the match;
  server-side skill ratings; voice-directed squads; a behavior-tree visual editor
  (data plus TU1 covers authoring); telemetry off the machine.

## 3. Target design

Combat stays table-driven. `exchange.rs` resolves parry windows,
locked riposte and clash ties over CB1 table rows in integer ticks with
fighter-ID order; `sweep.rs` solves swept arcs continuously per tick as
segment-vs-capsule in fixed point (Q13) with shields as coverage cones;
the camera is a deterministic director (lock-on target, distance and
angle rules, trauma decaying in ticks, presented outside the sim, shaken in
presentation); modes are content over AR1 (ladder table, glory-rank
counters, repair-craft hooks, LIGHT slice only); the second-game proof
is a checklist enforced by a fixture build with an engine-diff gate.
Ownership: `crates/combat/src/exchange.rs` (GP1, GP15),
`crates/combat/src/sweep.rs` (GP2, GP15),
`crates/combat/src/feel.rs` (GP11, GP18, GP28, GP29),
`crates/combat/src/tables.rs` plus `tables_text.rs` (GP27, GP30, GP31),
`crates/game/src/camera.rs` (GP3, GP16, GP42),
`crates/game/src/ladder.rs` plus `crates/game/src/modes.rs` and
`content/` ladder tables (GP4, GP17, GP40), `crates/playtest/src/tournament.rs`
(GA13, GA14, GP33). Threads: simulation thread only; the camera reads checkpoints
and presentation reads juice output on its own clock. Budgets:
resolution at or under 0.4 ms per fighter, sweep at or under 0.1 ms per
weapon, camera at or under 0.05 ms per frame, ladder `apply` O(1),
feel tick O(events) with no heap. API sketch (already landed):

```rust
pub struct ParryWindow { pub start_tick: u32, pub active: u8 }
pub fn resolve_clash(a: &Move, b: &Move, poise: (i32, i32)) -> ClashOut;
```

Data flow per tick: inputs arrive as `FighterInput` slots, intents fold
through versioned move rows, movement integrates first, then sweep
volumes are solved in sorted weapon order, overlap resolves in
fighter-ID order through exchange tables, damage, parry, poise and
hit-stop apply, and typed events leave the tick for presentation. The
arena owns the step, combat owns the resolvers, game owns the director
and modes, input owns bindings, and the editor owns tuning lanes. No
row crosses those edges: arena never edits tables directly, combat
never touches presentation, game never reorders resolution, and tuning
never bypasses TU1 staging at tick boundaries. Determinism follows
`../HASH-LAW.md`: authoritative simulation uses only integer math plus
stable orders with x2 byte equality; the camera `sqrt` and any juice
shaping stay presentation-side and never enter a checkpoint hash.

### Design data (GP27: designers change numbers without Rust)

One format for every design table, in `content/design/<name>.rows`: line 1
`table <name> version <n>`; then one row per line `<id> <key>=<int> ...`;
whole numbers only, the unit in the key (`_ticks` at 60 Hz, `_permille`,
`_mm`, `_cm`); `#` comments; no floats. `crates/combat/src/tables_text.rs`
(std only) parses `combat.rows` into the existing `MoveTableSet`, runs
`validate_tables`, and refuses a bad row with its line number. The compiled
`TABLES_V1` and `v2()` stay the truth for the golden: the parsed v1 and v2
digests must equal them, so the file is a second door to the same numbers,
not a fork. Later tables: `enemies.rows` (GA13), `arena-zones.rows` (GP36),
`ladder.rows` beat chart (GP40), `controls.rows` and `cues.rows` (the UX
seat). Format notes for seats: `docs/design/FORMAT.md`. Hot reload rides EG29.

### Combat rules (the arena's own move tables, v2; Bycer ch3, ch6, ch8; Rogers L6)

Starting values, tuned only by the GA14 sweep, never by hand in Rust:

| Rule | v2 starting value | Why (book) | Row |
|---|---|---|---|
| Light startup (tell) | 12-18 ticks (200-300 ms) | human reaction 200-250 ms; 4 ticks (v1) is unreadable | EG19, GP32 |
| Heavy startup | 24-30 ticks, a visible windup pose from tick 1 | a heavy must be readable and punishable | GP30 |
| Parry window | 6-8 ticks | EG19 | EG19 |
| Input buffer | 8 ticks (133 ms), table value | a press in recovery is a promise kept, not lost | GP28 |
| Commit point | the hit tick: no dodge, block or new attack cancels after it | commitment pillar | GP29 |
| Cancel window | from startup tick 0 to the hit tick minus `cancel_grace` | a feint is allowed early, never late | GP29 |
| Hit-stop | light 3, heavy 6, parry 8 ticks (`crates/combat/src/tables.rs:107,116,125`); the v2 row keeps one flat `hitstop_ticks=4` (`content/design/combat.rows:9`) | weight reads in the freeze | GP31, LF10 |
| Poise | light hits below the stagger threshold, a heavy or two lights in 60 ticks stagger | poise turns damage into rhythm | GP30 |
| Stamina | per-kind cost (heavy above light, dodge above light), regen after a 30-tick delay | the slow negative leg: the leader who spams tires | GP30 |

v1 stays byte-frozen (HASH-LAW). The arena chooses the table version per match
(`MatchConfig`): the proof scene keeps v1 and the golden; the Rome duel and the
window build pass v2 (GP32).

### Enemies and difficulty (Bycer ch8, Perry ch14 and ch22, Killick ch6, Sellers ch10, Short-Adams ch24)

`content/design/enemies.rows` (GA13), one row per tier, loaded by `crates/ai`:
`react_ticks`, `noise_ticks` (seeded timing noise), `aggression_permille`,
`moves` (which v2 rows it may use), `hp_permille` (a narrow band, 900-1200),
`counter` (the answer that beats it), `silhouette` (one word for the look seat).
A tier is mostly skill, not stats: the P3 tournament `Difficulty`
(`playtest/src/tournament.rs:55-104`) stays morale-only for the mirror balance
proof, and tiers are a separate axis. Tiers: T1 grunt, T2 veteran, T3 elite, T4
primus (boss, GA15: its own move set and two phases, a new answer in phase 2,
never just more hp; names from `content/design/enemies.rows:10-13`). Rule: each
tier beats the one below in 60%+ of 200 seeded duels and loses to the one above
in 60%+ (an odd cycle check keeps no tier dominant). Difficulty (GA14): a sweep of
one `BalanceConfig` field writes `target/reports/difficulty-<knob>.csv`; scripted
players carry seeded timing noise of 2-4 ticks (33-66 ms) and act once per 8 ticks
(`content/design/enemies.rows:15-17`, `crates/playtest/src/winrates.rs:45-49`); rung
strength must rise strictly. The noise is part of the AI input tape, so replays stay exact.

### Resource ledger (Adams-Dormans ch4-7; every pool has a source and a drain)

| Pool | Source | Drain | Loop | File |
|---|---|---|---|---|
| hp | none in a round; full at round start | hits (`attack_damage` 120 permille, `content/design/combat.rows:9`) | the win condition | `combat/src/fight.rs` |
| stamina | regen after a 30-tick delay | attack, dodge, block (per kind) | negative: spam tires | `combat/src/tables.rs` (GP30) |
| poise | regen per tick | poise damage per hit | stagger turns hits into openings | `combat/src/depth.rs` PoiseMeter (GP30) |
| morale (AI) | landed hits, crowd | taken hits, hp share | AI aggression and retreat | `ai/src/gladiator.rs:146-158` |
| round wins | a won round | none | best of 5 | `game/src/lib.rs` (GP39) |
| glory | a won bout (rung weight) | a lost bout (flat 50, `content/design/loop.rows:9`, `crates/game/src/ladder.rs:26`) | ladder progress | `crates/game/src/ladder.rs` (GP40); the `penalty=25` column of `content/design/ladder.rows:25-32` is lint-bounded only (`tools/beat-lint.mjs:171-173`) |

### The arena as a level (Vazhenich *Worlds* II.6, Rogers L9, Killick ch5)

Numbers come from `content/arena/rome-spec.json` (GP37): half extent 10 m, spawns at -4 m and +4 m; the march speed 1.7 m/s is the Rome
design number (`docs/plan/rome-spec.md:31`). The sword does 120 permille damage with a 15-tick startup and an 8-tick recovery
(`content/design/combat.rows:9`, `content/design/weapons.rows:24`); the blade reach is 1500 mm (`crates/arena/src/match_wire.rs:33`), while the json keeps
`range_mm` 2000 for forge parity only. The approach beat is (8 - 1.5) m / 1.7 m/s = about 3.8 s (229 ticks) of a 600-tick round: the first
exchange must start by tick 240 in 90% of bot duels. Kit actors marked `physical` become obstacles in the match (GP35).
Zones in `content/design/arena-zones.rows` (GP36): the centre circle r 250 cm (`:16`) and the gate mouth r 200 cm (`:9`); the code classifier
bands are 3 m and 8 m (`crates/arena/src/zones.rs:17,19`). Metrics per tournament: time share per zone, `entrance_share` (time any fighter
holds the gate mouth, bar under 250 permille of fighter ticks, `crates/playtest/src/tournament.rs:1980`; the level check keeps a
floor-area band of 25..60, `docs/design/LEVELS.md:174`), kills per zone, cover uses. A layout change lands only with these numbers
before and after.

### Ladder pacing (Rogers ch4, Prokhorenko ch3, Trefry)

`content/design/ladder.rows` (GP40): per rung the enemy tier, at most 3 new elements (a move, a rule, an arena variant), the reward and the penalty, and the
expected minutes. The first bout starts within 45 s of launch, capped at 60 s (`content/design/ladder.rows:13-15`, `crates/game/src/ladder.rs:49,51`;
parked ED18 "first fight within 15 minutes" is replaced by this bar for Rome). Lint `tools/beat-lint.mjs`: new elements at most 3 per rung and equal to
the compiled chart (`tools/beat-lint.mjs:164-166`), tier never falls, the boss is last, minutes equal the compiled chart (`:183-185`); the first-bout cap is
not a lint rule (`tools/beat-lint.mjs:11-15`), `crates/game/src/ladder.rs` owns it.

### HUD and readability (Rogers L8, McLean-Sharp ch4)

The duel HUD (GP38, `ui/src/lib.rs:781 DuelOverlay`, drawn from
`game/src/windowed.rs`): hp pips, stamina bar, poise flash on stagger, round
wins (best of 5), round timer. Clean screen: no other HUD in a bout; the UX
seat's `controls.rows` and `cues.rows` name the key and the sound cue for each
verb (hit, block, parry, stagger, round won). The UI park lifts for the duel HUD
only (owner 2026-10-12 order for full game teams); menus, inventory and the
rest stay parked.

### Playtests that ask a question (McLean-Sharp ch10-12, Finska, Perry ch21)

A charter (GP41, `crates/playtest/charters/<id>.toml`): one question ("does a new
player land a parry in the first bout?"), a scripted or bot run, pass and fail
lines the run prints, and a log of what, where, why and conclusion. Bugs carry
severity A (blocks a gate), B (hurts a pillar) or C (cosmetic) and a repro. Bot
charters run in the loop; human sessions (5-7 newcomers) are Maxim's hands, asked
once through the inbox with the charter attached.
Status 2026-10-16: the "asked once" half is not on disk: `node C:/empire/center/empire.mjs inbox engine2040` lists 10 open items (cap 10) and none names the newcomer session, and the add is refused at that cap (`inbox engine2040 add ... --ref PLAYTEST-KIT-HANDS` prints `not added, 10 open items (cap 10): tick or merge one first`), so the ask waits on one ticked or merged item; the kit half is done (`docs/design/PLAYTEST.md` ships the script, survey, funnel and A/B/C grades, `node tools/playtest-log.mjs --check <csv>` grades a log row by row, and `node --test tools/playtest-log.test.mjs` is 1 pass).
Status 2026-10-17: that count is stale on disk: `node --test tools/playtest-log.test.mjs` is 2 pass (tests 2, pass 2, fail 0) because the missing-log case landed after that line (commit 4fd1d1e0, `tools/playtest-log.test.mjs` +12 lines), and `docs/design/PLAYTEST.md:7` already reads the same 2 pass. The "asked once" half is unchanged: a re-send at 2026-10-17T01:44Z (`node C:/empire/center/empire.mjs inbox engine2040 add "PLAYTEST kit is ready for a human round" --by "tech writer seat"`) was refused again with `engine2040: not added, 10 open items (cap 10): tick or merge one first`, so the ask still waits on one ticked or merged item.

### brains and navigation

Data flow (one direction, no cycles):

```
perception (clocked every N ticks) -> utility scoring (morale, stamina, role)
  -> intent (FighterInput bits, quantized once, recorded)
  -> path request (mesh query or link traverse or platoon hold)
  -> steering (separation plus arrive, ORCA near, flow far, funnel smoothing)
  -> arena applies at the tick boundary with request ids
```

Ownership: `ai` owns decisions, morale, squads, difficulty, enemy tiers and the
legion brain; `nav` owns meshes, tiles, links, queries, steering,
platoons and funnels; `arena` owns the match and calls both through
public APIs (arena maps snapshots into AI observation types; no reverse
imports). `arena/src/engage.rs:20` and `fork.rs:271` already use `engine_ai`
brains; nav steering is the unwired part (BOOK-GAPS W12). Traversal links are
authored in content (sidecar markers) and baked by the cooker; TV1 consumes them
at runtime. DX1 break sets invalidate tiles through the `rebake.rs` registry;
WD8 stitches per-cell NV1 bakes on shared edges once tiles land.
Status 2026-10-17: disk shows the S11 left line is gone, so the gap no longer holds: `rg -n "AI[-]to[-]arena" docs/plan/gameplay.md` exits 1 with zero hits, and the row's cite `docs/plan/gameplay.md:150` now reads "counters, repair-craft hooks, LIGHT slice only); the second-game proof", while the corrected text sits at `docs/plan/gameplay.md:305-306` ("`arena/src/engage.rs:20` and `fork.rs:271` already use `engine_ai` brains; nav steering is the unwired part (BOOK-GAPS W12)"); the disk half holds: `crates/arena/src/engage.rs:20` is `use engine_ai::{FoeView, MeleeView, Percept, Squad, StrikePhase};` and the brains field is at `crates/arena/src/fork.rs:271` `brains: Vec<engine_ai::DepthBrain>`, so only the nav-steering half (W12) stays open, with `docs/plan/gameplay.md:437` keeping GA8 "needs: -".

Determinism: same seed plus same tape gives same intents, paths and
steering. Bakes sort everything and emit versioned bytes (`E2NAV1` v1;
bump on layout change, refuse anything else). Ties resolve by stable
id. Unknown targets return typed errors, never the nearest guess.

**Steals 2026-10-12** (board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`). One camera stack: fighter_look's fight camera calls `camera.rs`; exact halflife springs on yaw, distance and look point; trauma noise is smooth on 3 axes plus roll; the FOV punch rides a timed spring (GP43). Kill slow-mo scales wall time at the tick accumulator, local and replay only, never in a netgame (GP45). Hit-stop presentation holds alpha and shudders the victim 1-2 cm; the sim counter stays the only freeze. The camera is view code: nothing is recorded in the sim.

**Steals 2026-10-12 (HUD, sound, input)** (board shard `../SPRINT-21-look-feel.md`, scout file `../research/steals-2026-10-12/audio-ui-matrix.md`). Hard problem: three clocks meet at the hit (the 60 Hz tick, the audio callback block, the display frame); measure all three on one run (SN14, PF25) and never move the tick to fix a feel gap. HUD: hp pips with a chip-lag tail, stamina under hp, poise as ring segments around the fighter, text >= 28 px at 1080p, contrast >= 4.5:1 for text and 3:1 for bars, never colour alone (UIX13; GP38 draws the duel overlay through it). Attack gestures (tap, hold, hold-and-release, chord) are counted in integer ticks before the tape (GP46).

## 4. Hard problems

- **New behavior without moving the golden.** Problem: every combat rule above
  changes the bout hash. Technique: version-gated rules (v2 only) plus a
  per-match table choice, the proof scene pinned to v1. Why: HASH-LAW keeps
  `d57b5c8ae3096614`; the live duel still gets the better game. Reference:
  `../HASH-LAW.md`, EG19.
- **Balance with step-function outcomes.** Problem: one seed flips a duel; a
  finite difference on one seed is noise (Orland ch12). Technique: average 5+
  seeds per knob value, halve the step when the averaged loss rises, stop when
  the sign flips across seeds; one CSV row per run. Reference: Adams-Dormans ch8.
- **A human-like opponent that stays exact.** Problem: perfect bots read tells
  instantly. Technique: reaction delay plus seeded timing noise inside the AI
  input tape (Short-Adams ch24), clamped per tier. Why: difficulty becomes a
  number and replay stays exact.
- **Earlier hard problems** (subsumption stack, nav) are in
  `docs/archive/2026-10-10/plans/gameplay-cut.md`.

## 5. Donors

Donors: `steals.md`; porting policy is AGENTS rule 4. Book methods (no code) are cited per row; the donor table (flax-mcp, Pichlmair, Lin, VK1, RES-ARXIV, recast and big-brain shapes) is in `docs/archive/2026-10-10/plans/gameplay-cut.md`.

## 6. Slim

- No new combat runtime: tables plus the two landed resolvers
  (`exchange.rs`, `sweep.rs`) only; GP15 proves wiring, never a third
  resolver. The design file loader (GP27) parses into the existing
  `MoveTableSet`, never a mirrored type.
- No cinematic camera system: director rules plus trauma only; GP16
  proves the WIN1 presentation feed; the debug camera (GP42) is a mode of
  `camera.rs`, never a second stack.
- GP4, GP17 and GP40 are LIGHT: ladder plus glory counters plus craft hooks
  plus a beat chart; full trees, economy, progression and survival stay CUT
  with a grep gate and no files.
- One balance bar: `playtest/src/tournament.rs`. `game/src/bench.rs` is
  deleted or rewired under that bar (GP34), never a second gate.
- No real second product: GP5 stays a fixture game plus a checklist doc
  with the engine-diff gate in proof.
- No mirrored types: resolvers reuse CB1 `Tables`, slot counts reuse
  `engine_input::MAX_PARTICIPANTS` (GE3 canonical cap), trauma stays one
  permille track; GP rows deepen tables, never fork them.
- What we will not build here: GP8/GP9/GF1/GF2/GF3 ports (their own
  rows own them), UNVERIFIED feel theories as gates (GP6/GP7/GP13/GP14
  stay ideas-only until a license and a keypoint exist), networked
  motion (LM3 owns it), rollback netcode (RC13/NET1 own it), a
  procedural arena generator (ten hand-made layouts beat a generator for
  one arena; the forge packs own procgen), BET-GP24 as a row.

## 7. Board tasks

Rows from the steals (HUD, sound, input; board shard `../SPRINT-21-look-feel.md`, scout file `../research/steals-2026-10-12/audio-ui-matrix.md`): UIX13 (GP38 needs it), SN12, GP46.

Rows from the steals (2026-10-12, board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`): GP43 one stateful fight camera, GP44 phase-warped attack with a telegraph meter (needs EG19c), GP45 kill slow-mo.

Rows live on the board; slice any row with `node tools/board-slice.mjs <ID>`. The game rows (FOCUS Now group 8) are in `docs/SPRINT-20-game.md` with full Accept, F2P and P2P lines; Tier A and Tier B rows of the old order follow with Accept lines verbatim, then the table.

Game (owner 2026-10-12, in FOCUS order):

| ID | Title | Tier | Needs |
|---|---|---|---|
| GP27 | design rows file and validated loader | A | - |
| GP28 | input buffer at intake (v2) | A | EG19 |
| GP29 | commit lock: no cancel after the hit tick (v2) | A | EG19 |
| GP30 | poise and per-kind stamina (v2) | A | EG19 |
| GP31 | per-kind hit-stop (v2) | A | EG19 |
| GP32 | the live duel runs v2, the proof scene keeps v1 | A | GP28 GP29 |
| GP37 | one Rome numbers file plus a drift test | A | - |
| GP35 | kit walls stop fighters | A | GP37 |
| GP39 | round and match loop (600 ticks, best of 5, tie replays) | A | - |
| GP38 | duel HUD drawn | A | GP39 |
| GP42 | debug camera, pause and step in the arena binary | A | - |
| GA13 | enemy tiers T1-T3 from enemies.rows | B | GP27 |
| GA14 | difficulty sweep with a human-like error model | B | GA13 |
| GP33 | time-to-kill and matchup table | B | GA13 |
| GP36 | arena zones and the entrance metric | B | GP35 |
| GP40 | ladder beat chart and lint | B | GA13 |
| GP41 | playtest charters with a question and A/B/C bugs | B | - |
| GP34 | one balance bar (bench.rs deleted or rewired) | B | - |
| GA15 | primus boss: own move set, two phases | B | GA13 GP30 |

Row changes (2026-10-12): GP21 Accept "playtester confirms small/big separation" becomes "charter C2 (GP41) prints PASS for small/big separation"; NV10 takes its neighbour lists from `physics/src/broadphase.rs` (grid) and lifts `MAX_AGENTS` 256 by measure (`crowd.rs:741-745`); GP9 is satisfied by wiring `PoiseMeter` (GP30), not a port; ED18 (`parked.md`) yields to the GP40 first-bout bar for Rome.

Row changes (2026-10-15): new producer rows GP47-GP50 board the producer seat's studio tracking (burndown line, STAGES.md, buffers, risks) so runs go through claims, caps and proof instead of ad-hoc appends; no existing row text changed.

Row changes (2026-10-15, planner-gameplay-r1 critic pass): GP49 F2P counted `passed:` lines but `docs/design/STAGES.md:22,29,37,48,59,68` already holds 6, so the fail-before proof could never fail; GP49 F2P now counts `last output:` lines (0 before, 6 after), P2P unchanged. GP50 F2P pattern tightened to `estimate-against-actual` (0 hits before; the prose `estimate vs actual` at `docs/design/STAGES.md:34` no longer matches). GP47 F2P holds (`tools/burndown.mjs` missing, Glob 2026-10-15). GP48 F2P holds (STAGES.md runs past line 741, Read 2026-10-15). Methods: Prokhorenko ch4-5 (stage exits, 15%/25% buffers, `passed: no` sign-off) and Sellers ch12 (estimate-vs-actual ratio, top-3 risk ledger); filed source is BOOK-GAPS-2 N30 (`docs/plan/BOOK-GAPS-2-2026-10-12.md:101`). No new rows; no row bullets changed.

Row changes (2026-10-15, planner-gameplay-r2): GP49 bullet F2P now counts `last output:` lines to match the r1 intent; no other row text changed.

Producer (studio stages and burndown, owner 2026-10-15): the producer seat keeps one honest line per run in `docs/design/burndown.tsv` and the stage table in `docs/design/STAGES.md` with the three risks that stop the next stage, each with an owner seat. Precedent: GP40 owns `tools/beat-lint.mjs`; producer rows own `tools/burndown.mjs` the same way. No inbox asks ride these rows; a decision only the owner can take goes through `docs/EMPIRE-INBOX.md`, never as a duplicate ask.

| ID | Title | Tier | Needs |
|---|---|---|---|
| GP47 | burndown line as a tool, not a hand count | B | - |
| GP48 | STAGES.md slim: 700 lines back under 150, history to receipts | B | - |
| GP49 | every stage carries its exit test output and one passed field | B | GP48 |
| GP50 | buffers recalibrated plus the risk ledger rule | B | GP49 |

- [ ] GP47 producer: burndown line as a tool, not a hand count. Tier B. Size S.
  Owns `tools/burndown.mjs` plus `tools/burndown.test.mjs` (new). Consumer: the producer seat run that appends the line. Accept: `node tools/burndown.mjs --dry-run` prints one tab-separated line with the 8 header fields of `docs/design/burndown.tsv:1`, every number reproducible from its named command; `node --test tools/burndown.test.mjs` green. F2P: the dry-run fails before (file missing), prints the 8-field line after. P2P: `node --test tools/burndown.test.mjs` exit 0. Stop: 1 session; the hand count stays until the tool lands. needs: none.
- [ ] GP48 producer: STAGES.md slim, history to receipts. Tier B. Size S.
  Owns `docs/design/STAGES.md` plus `docs/receipts/2026-10-producer.md` (new). Consumer: every later producer run and any reader. Accept: STAGES.md reads under 150 lines; Risks holds only the current top 3, each as risk | owner seat | done-when command; every older Re-verified paragraph moves verbatim to the receipts file; `node tools/workspace-check.mjs` shows no new FAIL line. F2P: the file reads 700 lines before, under 150 after. P2P: `node tools/workspace-check.mjs` exit 0. Stop: 1 session; no stage-table number changes in this row. needs: none.
- [ ] GP49 producer: every stage carries its exit test output and one passed field. Tier B. Size S.
  Owns `docs/design/STAGES.md`. Consumer: the owner sign-off per stage. Accept: each of the 6 stages names its exit test as one command from `docs/plan/finish-line.md` or `docs/FINISH-RICH.md` with the last output and date pasted beside it, plus one `passed:` field; every field reads `passed: no` until its exit test passes; a stage never passes without its exit test; no bar lowered. F2P: `Select-String docs/design/STAGES.md -Pattern 'last output:'` prints 0 lines before, 6 after. P2P: `node tools/workspace-check.mjs` exit 0. Stop: 1 session; exit-test failures become BLOCKERS rows, never edits to the bar. needs: GP48.
- [ ] GP50 producer: buffers recalibrated plus the risk ledger rule. Tier B. Size S.
  Owns `docs/design/STAGES.md`. Consumer: the next stage plan. Accept: each stage buffer recomputed at 15% for known work or 25% for research-heavy work of its current open count, with the estimate-against-actual ratio from the first landed Game rows (GP28-GP30, 4c6d817) recorded beside it; Risks lists the top 3 ranked by blocked rows, each with an owner seat and a done-when command; a risk older than 2 rounds without movement becomes an owner row or is killed with a one-line reason. F2P: no estimate-against-actual ratio in the file before, one ratio line after. P2P: `node tools/workspace-check.mjs` exit 0. Stop: 1 session; at most 2 inbox asks per run, never a duplicate of an open item. needs: GP49.

- [ ] GP18 feel baseline lock. Tier B. needs: -
  Accept: PQ1 p95 at or under 50ms; PQ2 40–60% win-rate with no move above 35% of kills; seeded float-math or magic-number regression fails the gate.
- [ ] GP21 trauma-curve shake stage. Tier B. needs: -
  Accept: jab <= 1/4 super displacement; restore holds every tick; sim bytes x2; playtester confirms small/big separation.
- [ ] NV4 near-field avoidance. Tier B. needs: -
  Accept: steering ≤ 0.1 ms per agent, flat from 50 to 500 agents; 0 stuck in a 1,000-seed corridor drill; replay x2.
- [ ] NV6 smooth paths. Tier B. needs: -
  Accept: paths never longer than the grid path and at least 10% shorter on 80% of queries; query at most 0.2 ms p50; a breach opens a path within 1 s; bytes x2.
- [ ] GA7 diverse fighting strategies via offline integer students. Tier B. needs: -
  Accept: ≥3 strategies per difficulty, PQ2 win-rate 40–60%, no float in tick. Kill: float in tick, diversity < 3, win-rate outside band.
- [ ] GA8 EQS arena bind. Tier A. needs: -
  Accept: one ranked-spot fixture drives a live arena fighter to cover and holds; replay x2; arena tick hash unchanged on seeds that never query. Proof: `cargo test -p engine-ai eqs` green plus the arena bind fixture run twice.

| ID | Title | Tier | Needs |
|---|---|---|---|
| CB3 | delta-battle design patterns | B | - |
| GP22 | unified impact fan-out | B | GP21 |
| GP8 | camera feel cores | B | - |
| GP6 | feel quantification | A | - |
| GP7 | feel keypoint extension | A | - |
| GP14 | camera framing for third-person melee | A | - |
| GP25 | play-step follow camera | B | - |
| GP26 | shake A-B bundle | B | GP21 |
| NV5 | legion platoons | B | NV4 |
| NV8 | 500-agent soak harness | A | NV4 NV5 |
| GA5 | thinking LOD plus perception-clock audit with GA2 | B | NV5 |
| GA9 | legion-vs-legion fixture | B | NV5 |
| GA10 | threshold-gated utility pick + otherwise fallback | B | - |
| GA11 | deterministic behavior-tree executor | B | - |
| GA12 | GOAP planner over the utility world-state | B | GA11 |
| NV10 | broadphase plus 1,000-agent P6 bar | A | NV4 |
| NV11 | grid-retirement proof | A | NV6 |
| NV12 | shared-edge stitch proof | A | - |
| NV13 | temp-obstacle request queue | A | - |
| NV14 | per-tile integer-mm rebake detail | A | NV13 |
| NV15 | salt-epoch handle detail | A | NV13 |
| NV16 | rational-NURBS spline math + parallel-transport frames | B | - |

## 8. Quality gates

- Every row: replay-exact x2; order-permutation tests keep the frozen
  resolution order plus fighter-ID tiebreak; a shuffled input order that
  changes an outcome fails the row.
- Golden: `d57b5c8ae3096614` unchanged on the proof scene (v1) under every
  game row; a v2 rule that leaks into v1 fails the row.
- Design files: every `content/design/*.rows` parses with zero refusals;
  the parsed v1 and v2 digests equal the compiled tables; a float, a missing
  unit or an unknown key refuses with its line number.
- Combat rules (v2): every live attack has startup of 12+ ticks and at least
  one answer; no cancel after the hit tick; heavy staggers and light does not
  at full poise; per-kind stamina cost orders heavy above light.
- Opponents: each tier beats the one below in 60%+ of 200 seeded duels and
  loses to the one above; rung strength rises strictly in the GA14 sweep; no
  move above 35% of kills; the noise-on and noise-off runs both replay x2.
- Arena: no fighter passes a physical kit actor in 1,000 seeded ticks; `entrance_share` under 250 permille of fighter ticks
  (`crates/playtest/src/tournament.rs:1980`; the level check keeps a floor-area band of 25..60, `docs/design/LEVELS.md:174`); the first
  exchange starts by tick 240 in 90% of bot duels; arena numbers equal `content/arena/rome-spec.json` (drift test).
- Clash mirror test: scripted exchange with fighter IDs swapped mirrors
  the outcome; container order never decides.
- Sweep coverage: every weapon length by speed bin keeps a tunneling
  fixture; an uncovered bin fails the row; thin fast rapier arcs are
  the bar.
- Camera framing: scripted 2-fighter orbits keep both fighters in frame
  every tick for 600 ticks through GP16; trauma track posted and
  replay-exact.
- Modes LIGHT gate: grep over owned files refuses skilltrees-full,
  crafting-full, economy, progression, survival, building and BR
  strings; craft hook fires exactly once per loop.
- GP5 engine-diff gate: `git diff -- crates/` stays empty after the
  fixture game lands, enforced in proof, never eyeballed.
- Feel kill bars: float math in the tick kills any GP row; PQ1 p95
  above 50 ms or PQ2 outside 40–60% (or any move above 35% of kills)
  fails GP18; balance changes ride EA1c fork metrics (bullies and
  degenerate freezes rejected with fork ID plus tape).
- Zero per-tick heap allocs on the exchange, sweep, camera and feel
  paths (counting-allocator gate where wired); tuning flows through
  TU1 lanes and `content/design/*.rows` only, with no magic numbers
  outside versioned tables.
- Allocation and timing budgets ride the PF1 harness where wired:
  resolution at or under 0.4 ms per fighter, sweep at or under 0.1 ms
  per weapon, camera at or under 0.05 ms per frame; a seeded regression
  that moves any bar fails with numbers posted, never with a weakened
  test.
- Save, net and proof compatibility: resolution and ladder folds stay
  inside the versioned envelope; a corrupt or future-version table
  refuses loudly with its version named; loopback drop-and-rejoin plus
  save round-trip converge with byte equality x2.

### brains and navigation

- `cargo test -p engine-ai -p engine-nav` green; NV1 plus NV2 plus NV3
  plus NV7 plus GA1 plus GA2 plus GA3 plus GA6 suites stay green under
  every row (wall-hold, rebake-x2, eqs fixtures, dialogue bake,
  funnel-shorter proof, zero-stuck drill, all bytes x2).
- Determinism: same seed twice gives identical intents, paths and
  steering across chunkings 1, 2, 4 and 8; shuffled inputs bake
  identically; golden `d57b5c8ae3096614` unchanged (nav and AI feed
  the tick through recorded inputs, never around it).
- Budgets: query at most 0.2 ms p50; steering at most 0.1 ms per agent
  flat 50 to 500 (NV10 extends the flat rail to 1,000); rebake at most
  50 ms per dirty tile p95; a breach opens a path within 1 s; 500 agents
  under 1 ms; 1,000-agent sim p99 within P6 2 ms; server tick p99 within
  the ON10 bar with interest on.
- Fresh-agent gates: probe fixtures pick TV1-expected actions;
  warped roots land within 1 cm; win-rate bounds hold on GA2
  output; a shuffled-teacher student collapses or the pipeline is
  rejected with numbers.
- License gate: every pattern port carries `PORT()` plus an
  `EVIDENCE.md` row with the verified SPDX URL above; anything
  UNVERIFIED stays a paper-shaped reimplementation with no donor
  code; EPL-2.0 sources stay ideas only, never imports.

## 9. Session estimate

Game rows: Tier A GP27-GP32, GP35, GP37-GP39, GP42 about 14 sessions (each S or M); Tier B GA13-GA15, GP33, GP34, GP36, GP40, GP41 about 12 sessions. Older rows carry their own estimate on the board; the pre-merge estimates are in `git show d96091cd:docs/plan/gameplay.md` (section 9) and `docs/archive/2026-10-10/plans/game-ai.md` (section 9).
