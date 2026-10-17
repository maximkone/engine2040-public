### Game (owner 2026-10-12, `docs/plan/gameplay.md` section 0-3; books `docs/plan/BOOK-GAPS-2-2026-10-12.md`)

Boarded 2026-10-12 from 30 books read against the code (owner: "i bought much more books ... a lot of game
design", "need roles proper too for full game dev teams there too all rich enough"). The engine had the parts
and lacked the design layer: numbers a designer can change, attacks that commit and read, opponents in tiers,
an arena with walls and places, a round loop, a HUD, and playtests that ask a question. Every row is one crate
or one content file, at most 200 changed lines, with a test that fails before. Hash law: every new combat
rule is v2-only and the proof scene keeps tables v1, so golden `d57b5c8ae3096614` never moves. Starting
values are proposals the GA14 sweep tunes, not owner decisions. Studio seats: center `crews/engine2040/standing/`
(director, producer, design-tech, designer, design-enemy, design-systems, design-level, design-ux, qa-lead).

**Design data and combat rules (Tier A)**

- [x] GP27 combat: design rows file and validated loader — numbers live in Rust Landed a8be93ea.
  consts (`crates/combat/src/tables.rs` `TABLES_V1`, `v2()` at `:100`), so no
  design seat can change a number. New `crates/combat/src/tables_text.rs` (std
  only): parse `content/design/combat.rows` (line 1 `table combat version <n>`,
  rows `<id> <key>=<int> ...`, whole numbers, unit in the key, `#` comments)
  into the existing `MoveTableSet`, run `validate_tables`, refuse a bad row
  with its line number; the file holds v1 and v2 copied from the compiled
  tables; `docs/design/FORMAT.md` documents the format. Diff: regular. Tier A.
  Size S. Owns `crates/combat/src/tables_text.rs`, one `pub mod` line in
  `crates/combat/src/lib.rs`, `content/design/combat.rows`,
  `docs/design/FORMAT.md`. Accept: `cargo test -p engine-combat --lib
  tables_text` x2: parsed v1 and v2 digests equal `TABLES_V1` and `v2()`; a
  float, an unknown key and a missing unit each refuse with the line number.
  F2P: no loader exists (the test fails to compile). P2P: combat lib green;
  golden unchanged. needs: none.
- [x] GP28 combat: input buffer at intake, v2 only — a tap in recovery is lost Landed 4c6d817.
  (`crates/combat/src/fight.rs:789` starts an attack only when not attacking);
  the 2-tick buffer in `crates/combat/src/feel.rs:206-209` has no caller. Wire
  it at intake (`fight.rs:773-800`) with a table value `buffer_ticks` (v2
  start 8): a press inside the window fires on the first ready tick, outside
  it is dropped. v1 rows carry 0, so v1 bouts are byte-identical. Diff: good
  (hash law: v2 only). Tier A. Size S. Owns the intake hunk of
  `crates/combat/src/fight.rs`, `crates/combat/src/feel.rs`, the v2 row field
  in `crates/combat/src/tables.rs`. Accept: `cargo test -p engine-combat --lib
  input_buffer` x2: under v2 a press 5 ticks before recovery ends fires on the
  first ready tick, one 12 ticks before is dropped; under v1 both are dropped as
  today. F2P: the v2 press is lost today. P2P: golden unchanged. needs: none.
- [x] GP29 combat: commit lock, no cancel after the hit tick, v2 only — a dodge Landed 4c6d817.
  clears `is_attacking` at any tick (`fight.rs:776-783`); `cancel_grace`
  (`feel.rs:199-210`) is unwired. Under v2 a dodge, block or new attack is
  refused from the hit tick to the end of recovery; before it, inside
  `cancel_grace`, a feint is allowed. Diff: good (hash law: v2 only). Tier A.
  Size S. Owns the dodge hunk of `crates/combat/src/fight.rs` and the
  `cancel_grace` wiring in `feel.rs`. Accept: `cargo test -p engine-combat --lib
  commit_lock` x2: v2 dodge at hit tick + 1 refused, recovery holds; dodge 2
  ticks into startup allowed; v1 unchanged. F2P: the late dodge cancels today.
  P2P: golden unchanged. needs: none.
- [x] GP30 combat: poise and per-kind stamina, v2 only — every hit gives the Landed 4c6d817.
  same stagger and every attack costs the same stamina (`tables.rs:153-174`);
  `PoiseMeter` (`crates/ai/src/depth.rs:202-275`) has no caller in arena or
  game. Add v2 row fields `poise_damage`, `stamina_cost` per kind (light,
  heavy, dodge, block) and a regen delay (start 30 ticks); a heavy, or two
  lights inside 60 ticks, staggers a full-poise fighter; a single light does
  not. Move the meter into combat or call it from the fight step (one owner,
  no mirrored type). Diff: good. Tier A. Size M. Owns `crates/combat/src/
  tables.rs` v2 fields, the poise hunk of `fight.rs`, `ai/src/depth.rs`
  PoiseMeter if moved. Accept: `cargo test -p engine-combat --lib poise_kinds`
  x2: heavy staggers, light does not, two lights in 60 ticks do; heavy costs
  more stamina than light; regen starts after the delay. F2P: one stagger value
  for all hits today. P2P: golden unchanged; v1 digest unchanged. needs: none.
- [x] GP31 combat: per-kind hit-stop, v2 only — every hit sets the same Landed 624938f5.
  `hitstop_ticks` (`fight.rs:918,959`). v2 rows carry `hitstop_ticks` per kind
  (start light 3, heavy 6, parry 8); LF10 already holds the clip clocks through
  it. Diff: regular. Tier A. Size S. Owns the hit-stop hunks of
  `crates/combat/src/fight.rs` and the v2 field. Accept: `cargo test -p
  engine-combat --lib hitstop_kinds` x2: a v2 heavy hit freezes 6 ticks, a
  light 3, a parry 8; v1 unchanged. F2P: one value today. P2P: golden
  unchanged. needs: none.
- [ ] GP32 arena: the live duel runs tables v2, the proof scene keeps v1 — the
  arena hardcodes `TABLES_V1` (`crates/arena/src/lib.rs:105,1302`), so the
  readable startups, buffer and commit lock never reach a player. Add a table
  version to the match config: default v1 (the proof scene and the golden),
  the window build and the Rome duel mode pass v2. Diff: good (hash law: the
  default stays v1). Tier A. Size S. Owns the table-choice hunk of
  `crates/arena/src/lib.rs` and the mode hunk in `crates/game/src/modes.rs`.
  Accept: `cargo test -p engine-arena --lib table_version` x2: a v2 match logs
  startup 12-18 and a v1 match 4; `node tools/proof.mjs --skip-build` prints
  golden `d57b5c8ae3096614` twice. F2P: no table choice exists. P2P: golden
  unchanged. needs: GP28, GP29.

**The arena as a level, the round, the HUD (Tier A)**

- [ ] GP37 content: one Rome numbers file plus a drift test — the shared numbers
  live in `docs/plan/rome-spec.json` and forge `SPEC.md:69-73` names a missing
  `content/arena/rome-spec.json`; engine walls sit at z -8 and +8 m
  (`content/arena/placement.json` ids 3-4) while the spec's half extent is
  10 m. Move the file to `content/arena/rome-spec.json` (docs keeps the
  reasoning in `rome-spec.md`), add a drift test that fails when an arena
  constant differs from it (half extent, tick_hz 60, max_ticks 600, damage,
  cooldown, range 2000 mm, spawns -4/+4 m, seed rule), and move the walls to the
  spec. Diff: regular. Tier A. Size S. Owns `content/arena/rome-spec.json`,
  `content/arena/placement.json` wall rows, a test module in
  `crates/game/tests/it/`. Accept: `cargo test -p engine-game --test it
  rome_spec_drift` x2 green; changing one wall z by 1 m turns it red. F2P: the
  file is not where both builds read it and the walls drift. P2P: golden
  unchanged (the proof scene loads no kit). needs: none.
- [ ] GP35 game: kit walls stop fighters — all 11 actors in
  `content/arena/placement.json` say `"physical": true` but
  `crates/game/src/arena_kit.rs` never reads the flag and `MatchWorld::try_new`
  has no call in `crates/game/src`, so the fight runs on open sand. Build
  static obstacles (capsule vs box, the arena's existing `collide.rs` shapes)
  from physical actors when a match loads the kit. Diff: good. Tier A. Size M.
  Owns the obstacle hunk of `crates/game/src/arena_kit.rs` and the match-load
  glue. Accept: `cargo test -p engine-game --features app --lib kit_walls` x2:
  a fighter driven into a wall for 600 ticks never crosses it; 1,000 seeded
  ticks of two bots see 0 crossings. F2P: the fighter walks through today.
  P2P: golden unchanged. needs: GP37.
- [x] GP39 game: round and match loop — `rome-spec.md:41-47`: a round is 600 Landed 0fe4582b.
  ticks, all opponents at 0 hp wins early, else higher total hp wins, an exact
  tie replays the round; a match is best of 5. The game has only a `wins`
  counter (`crates/game/src/lib.rs:494,971`). Add the round reset, the
  best-of-5 match and the tie replay as a mode over AR1. Diff: regular. Tier A.
  Size S. Owns the round hunk of `crates/game/src/lib.rs` or a new
  `crates/game/src/rounds.rs`. Accept: `cargo test -p engine-game --lib
  round_loop` x2: scripted tapes give 3-0, 3-2 and a tie-replay match with the
  expected round count and winner. F2P: no round reset today. P2P: golden
  unchanged. needs: none.
- [ ] GP38 ui: the duel HUD is drawn — `crates/ui/src/lib.rs:781 DuelOverlay` is
  built only in ui tests. Draw it from `crates/game/src/windowed.rs`: hp pips,
  stamina bar, poise flash on stagger, round wins (best of 5), round timer.
  The UI park lifts for the duel HUD only (owner 2026-10-12). View only. Diff:
  regular. Tier A. Size S. Owns the HUD hunk of `crates/game/src/windowed.rs`.
  Accept: `rg -n "DuelOverlay::new\(" crates/game/src` hits; a window capture
  of a bout shows the HUD (product check frame non-empty in the HUD region,
  `target/reports/product/latest.md`). F2P: no game caller today. P2P: game lib
  green; golden unchanged. needs: GP39, UIX13.
- [ ] GP42 game: debug camera, pause and step in the arena binary — only
  `--camera-path` exists (`crates/game/src/bin/arena.rs:250`); pause and step
  live in the editor only (`crates/editor/src/play_in.rs:131-164`). Add
  `--pause-at-tick N`, `--step` (one tick per key) and a free-fly mode in
  `crates/game/src/camera.rs` (presentation only). Diff: regular. Tier A. Size
  S. Owns the flag hunk of `bin/arena.rs` and the free mode in `camera.rs`.
  Accept: `arena.exe --scene arena --fighters 2 --pause-at-tick 120 --frames
  200 --exit` prints `paused tick=120` and the same hash as an unpaused run at
  tick 120; `cargo test -p engine-game --lib free_cam` green. F2P: the flag is
  unknown today. P2P: golden unchanged. needs: none.

**Opponents, balance, pacing, playtests (Tier B)**

- [ ] GA13 ai: enemy tiers from enemies.rows — no tier exists (`rg
  "grunt|veteran|elite|boss" crates/ai/src` empty). `content/design/enemies.rows`
  rows T1 recruit, T2 veteran, T3 champion with `react_ticks`, `noise_ticks`,
  `aggression_permille`, `moves`, `hp_permille` (900-1200), `counter`,
  `silhouette`, parsed by the GP27 format, loaded by `crates/ai`. Tiers are
  skill first; the P3 `Difficulty` stays morale-only. Diff: good. Tier B. Size
  M. Owns `content/design/enemies.rows`, a tier module in `crates/ai/src/`, the
  tournament hook in `crates/playtest/src/tournament.rs`. Accept: `cargo test
  -p engine-playtest --lib tiers` x2: over 200 seeded duels each tier beats the
  one below in 600+ permille and loses to the one above; replay x2. F2P: no
  tier rows. P2P: golden unchanged; P3 tournament green. needs: GP27.
- [ ] GA14 playtest: difficulty sweep with a human-like error model — the
  tournament holds one band (`tournament.rs:36-40`) with no sweep and bots read
  tells instantly. Add a sweep of one config field to
  `target/reports/difficulty-<knob>.csv` (5 seeds per value, averaged) and a
  seeded timing noise plus reaction delay in the AI input tape (default off).
  Diff: good. Tier B. Size M. Owns the sweep and noise hunks of
  `crates/playtest/src/tournament.rs`. Accept: `cargo test -p engine-playtest
  --lib sweep_noise` x2: a perfect-player run first; with noise on, rung
  strength rises strictly across T1-T3; noise on and off both replay x2. F2P:
  no sweep or noise. P2P: P3 bars unchanged with noise off. needs: GA13.
- [ ] GP33 playtest: time-to-kill and matchup table — no per-pair numbers exist.
  A test computes time-to-kill per tier and move kind from the loaded tables
  into `target/reports/rome-ttk.csv` and a win matrix from the GA13 tournament
  into `rome-matchups.csv`; an odd-cycle check fails any row that beats all
  others. Diff: regular. Tier B. Size S. Owns a ttk module in
  `crates/playtest/src/`. Accept: `cargo test -p engine-playtest --lib ttk`
  x2: the CSVs exist with one row per pair and no dominant row. F2P: no table.
  P2P: P3 green. needs: GA13.
- [ ] GP36 content: arena zones and the entrance metric — the only cover is a
  1 m crate (`crate_a`, id 10) and `gate_tunnel` (id 5) is the one entrance;
  nothing measures where fights happen. `content/design/arena-zones.rows`
  (centre r < 3 m, ring 3-8 m, wall band, gate mouth) and tournament metrics:
  time share per zone, `entrance_share`, kills per zone. Diff: regular. Tier
  B. Size S. Owns `content/design/arena-zones.rows`, a zone metric in
  `crates/playtest/src/tournament.rs`. Accept: `cargo test -p engine-playtest
  --lib zones` x2 prints the four shares summing to 1000 permille and
  `entrance_share` under 250. F2P: no zones. P2P: P3 green. needs: GP35.
- [ ] GP40 game: ladder beat chart and lint — the ladder has ranks and `apply`
  (`crates/game/src/ladder.rs`) but no per-rung enemy, new element, reward or
  minutes. `content/design/ladder.rows` plus `tools/beat-lint.mjs`: at most 3
  new elements per rung, the tier never falls, the boss is last, the first
  bout starts within 60 s of launch. Diff: regular. Tier B. Size S. Owns
  `content/design/ladder.rows`, `tools/beat-lint.mjs` and its test. Accept:
  `node --test tools/beat-lint.test.mjs`: a synthetic ladder with a clumped
  rung fails, the real file passes. F2P: no beat chart. P2P: ladder tests
  green. needs: GA13.
- [x] GP41 playtest: charters with a question and A/B/C bugs — GP21 accepts on Landed 06134105.
  "playtester confirms"; `crates/playtest` asks no question. A charter file
  (`crates/playtest/charters/<id>.toml`: question, run, pass and fail lines)
  run by a `charter` entry point that prints PASS or FAIL per line and a log of
  what, where, why and conclusion; bug packets carry `sev: A|B|C`. First
  charters: C1 "a bot with human noise lands a parry in its first bout", C2
  "a light hit shakes the camera at most a quarter of a heavy" (replaces the
  GP21 tester line). Diff: regular. Tier B. Size M. Owns
  `crates/playtest/charters/`, a charter module in `crates/playtest/src/`.
  Accept: `cargo test -p engine-playtest --lib charter` x2: C1 and C2 print
  their lines; a broken expectation prints FAIL with the line. F2P: no charter
  runner. P2P: P3 green. needs: none.
- [ ] GP34 game: one balance bar — `crates/game/src/bench.rs:61-63` keeps a kill
  ceiling of 1000 permille per side that cannot trip and has no caller; the live
  bar is `playtest/src/tournament.rs:40` (350). Delete `bench.rs` (git rm, its
  test with it) or route it through the tournament bar with a per-move count;
  one gate stays. Diff: regular. Tier B. Size S. Owns `crates/game/src/bench.rs`
  and its mod line. Accept: `rg -n "1000" crates/game/src/bench.rs` empty or
  the file is gone; `cargo test -p engine-game --lib` green. F2P: the dead
  ceiling exists. P2P: golden unchanged. needs: none.
- [ ] GA15 ai: primus boss, own move set and two phases — the ladder has no
  final opponent. T4 primus in `enemies.rows`: two phases (phase 2 at 500
  permille hp adds one new v2 move with its own answer), never just more hp; an
  owner veto stays open (inbox ref `BOSS-primus`). Diff: good. Tier B. Size M.
  Owns the T4 rows and the phase hunk of the GA13 tier module. Accept: `cargo
  test -p engine-playtest --lib primus` x2: T3 beats T4 phase 1 in under 40%
  of 200 duels; phase 2 starts at 500 permille hp; every T4 attack has an
  answer. F2P: no T4. P2P: tiers test green. needs: GA13, GP30.

### Engine rows from the books (2026-10-12, `BOOK-GAPS-2-2026-10-12.md` sections 2.1-2.12)

- [ ] RC36 runtime: the fighter step on the job pool — `crates/ecs/src/pool.rs`
  `JobPool` and `crates/physics/src/islands.rs:118 step_islands` give equal
  bytes at 1, 2, 4 and 8 threads, but their callers are tests only. Decision
  D6 (2026-10-12, center for the owner, "decide on all"): the sim may run an
  index-ordered fork-join when the hash is equal at every thread count. Run
  the per-fighter step through one shared pool with results merged in fighter
  order. Diff: hard. Tier B. Size M. Owns the step hunk of
  `crates/arena/src/lib.rs`. Accept: `node tools/proof.mjs --skip-build` with
  `E2040_THREADS=1` and `=8` both print golden `d57b5c8ae3096614`; ARENA
  frame-split p50 at 8 fighters before and after posted. F2P: no non-test
  caller of `JobPool::scope` in arena. P2P: golden unchanged. needs: none.
- [ ] RC37 arena: fork runs on the shared pool — `crates/arena/src/fork.rs:276-285`
  spawns one OS thread per branch (callers `endpoint/src/assistant.rs:1139`,
  `arena/src/probe.rs:55`), capping AX12's 20-branch speedup. Run branches on
  the RC36 pool, keep `ForkCrashed`. Diff: good. Tier B. Size S. Owns the
  worker hunk of `fork.rs`. Accept: `cargo test -p engine-arena --lib fork` x2
  green; a 20-branch run's wall time before and after posted. F2P: thread per
  branch. P2P: fork results byte-equal. needs: RC36.
- [ ] CQ17 quality: a loom model of the pool — no `loom` or TSan anywhere; Miri
  (CQ15) does not find races or lock cycles (Gregory 4.7-4.9). A `loom` test of
  `JobPool::scope` behind a `loom` cfg. Diff: good. Tier B. Size S. Owns a
  loom test in `crates/ecs/src/pool.rs` and a dev-dependency (orchestrator adds
  it with `deny.toml`). Accept: `RUSTFLAGS="--cfg loom" cargo test -p engine-ecs
  --lib loom_pool` green; a planted missing-wait fails it. F2P: no model. P2P:
  ecs lib green. needs: none.
- [ ] SL46 quality: the fight tick joins the float gate — `tools/rust-quality.mjs:21`
  SIM_CRATES lists 12 crates without `engine-arena`, whose `FighterState`
  (`arena/src/lib.rs:469-476`) keeps f32. Add it; copy `tools/sim-clippy.toml`
  to `crates/arena/clippy.toml`. Diff: regular. Tier A. Size S. Owns the list in
  `tools/rust-quality.mjs` and `crates/arena/clippy.toml`. Accept: `node
  tools/rust-quality.mjs --crate arena` reports its float count; a planted f32
  `sin()` in the arena step fails it. F2P: arena not scanned. P2P: the
  rust-quality baseline holds. needs: none.
- [ ] CQ18 quality: release keeps overflow checks — `[profile.release]`
  (`Cargo.toml:218-221`) sets none while comments at `:227,238` say checks stay
  on, and proof runs use `--release` (`tools/proof.mjs:401`). Set
  `overflow-checks = true`, measure the cost. Diff: regular. Tier A. Size S.
  Owns the profile lines of root `Cargo.toml` (orchestrator). Accept: `node
  tools/proof.mjs` prints golden `d57b5c8ae3096614` twice; ARENA frame-split
  p50 before and after posted (under 3% slower). F2P: release wraps where dev
  panics. P2P: golden unchanged. needs: none.
- [ ] CQ19 quality: GPU uniforms keep their layout, the ALLOC gate sees
  `.to_bytes()` — 0 `size_of` asserts in `crates/render/src`;
  `submit.rs:392,413` rebuild camera and light and call `to_bytes()` per draw
  item; the ALLOC regex (`tools/rust-quality.mjs:315`) misses it. One
  `const _: () = assert!(size_of::<T>() == N)` per uniform; build camera and
  light once per frame; add `.to_bytes()` to ALLOC. Diff: regular. Tier A.
  Size S. Owns the uniform structs' asserts, the per-frame hunk of
  `submit.rs`, the regex. Accept: `cargo test -p engine-render --lib` green;
  `node tools/rust-quality.mjs --crate render` ALLOC count drops by the moved
  calls. F2P: no asserts. P2P: captures byte-equal. needs: none.
- [ ] CQ20 quality: newtypes keep their field private, one owner for caches —
  about 30 IDs expose their field (`assets/src/registry.rs:32 Handle(pub u32)`,
  `core/src/fixed.rs:10`) against `docs/CODELAW.md:69`; `SUN_CACHES`
  (`submit.rs:24`) is a global Mutex. Make `Handle` and `Fixed` private with
  `from_raw`/`raw`; move `SUN_CACHES` into the presenter. Diff: regular. Tier B.
  Size M. Owns those two types and their call sites, the presenter hunk.
  Accept: `rg -n "struct Handle\(pub|struct Fixed\(pub" crates` empty;
  workspace tests green. F2P: public fields. P2P: golden unchanged. needs: none.
- [ ] CQ21 quality: the crate graph is tested acyclic and coupling has an alarm —
  `docs/ARCHITECTURE.md:9` promises an acyclic graph; `tools/workspace-check.mjs:308-318`
  only walks reachability; `tools/crate-map.mjs:4-5` prints fan-in and fan-out
  with no bar. Add a DFS cycle check with a seeded-cycle test and instability
  `CE / (CA + CE)` per crate. Diff: regular. Tier B. Size S. Owns the check in
  `tools/workspace-check.mjs` and `tools/crate-map.mjs`, their tests. Accept:
  `node --test tools/workspace-check.test.mjs`: a seeded cycle fails, the real
  graph passes; `node tools/crate-map.mjs` prints instability per crate. F2P:
  no cycle test. P2P: workspace-check result unchanged. needs: none.
- [ ] CQ22 docs: decision records — D1-D9 sit in `docs/FOCUS.md` prose and the
  book decisions D1-D6 (`BOOK-GAPS-2026-10-12.md:199-201`) as open lines; no
  record has a status or a check. `docs/adr/README.md` plus one record per
  decision (status, context, options, decision, consequences, the check that
  enforces it), starting with 0001 netcode path (D2) and 0002 sim threads (D6,
  decided with RC36). Diff: regular. Tier B. Size S. Owns `docs/adr/`.
  Accept: `(Get-ChildItem docs\adr -Filter '0*.md').Count` at least 9 and each
  names a status and a check path. F2P: no `docs/adr`. P2P: plan-index test
  green. needs: none.
- [ ] RL32 render: the live loop sorts and groups draws — `submit.rs:505-516`
  draws `list.items` in caller order, one bind and draw each; sorted groups
  exist (`render/src/draw.rs:615,660`) with no live caller; `draw.rs:502` sorts
  front to back only (Eberly 4.2.3-4.2.4). Key opaque draws by (pipeline,
  material, mesh) front to back, blended back to front with a stable id
  tie-break, and draw each group once. Diff: good. Tier A. Size M. Owns the
  draw loop of `crates/render/src/present/gpu_present/submit.rs`. Accept:
  ARENA prints draws and groups; at 16 and 256 kit copies draw calls fall to
  the group count; captures byte-equal before and after. F2P: one draw per
  item. P2P: golden unchanged. needs: RL60.
- [ ] RL33 game: cull bounds follow the posed body — `draw.rs:67-70` takes centre
  and radius from the caller and no fighter sets one, so wiring W7 on a
  bind-pose sphere culls limbs (Eberly 3.2). Bound each fighter from its posed
  joints in `crates/game/src/fighter_look.rs`. Diff: regular. Tier A. Size S.
  Owns the bound hunk of `fighter_look.rs`. Accept: `cargo test -p engine-game
  --features app --lib posed_bound`: an arm 1.5 m outside the bind sphere stays
  inside the bound. F2P: no fighter bound. P2P: captures unchanged. needs: none.
- [ ] PX41 game: one stepper for siege flight — `crates/game/src/ballistics.rs:634-636`
  is explicit Euler; the semi-implicit `crates/combat/src/projectile.rs:689-735`
  `step_round` has no game caller; the drop differs by g*dt*t (16 cm at 1 s,
  33 cm at 2 s) and the 2% test (`projectile.rs:999-1011`) hides it (Orland
  9.3). Use one stepper and make the test exact against the analytic drop.
  Diff: regular. Tier B. Size S. Owns the step hunk of `ballistics.rs` and the
  test. Accept: `cargo test -p engine-combat --lib projectile_exact` and `cargo
  test -p engine-game --lib ballistics` green; `rg -n "step_round" crates/game`
  hits. F2P: two steppers disagree. P2P: golden unchanged. needs: none.
- [ ] AX25 endpoint: fetched text is fenced as data — `crates/endpoint/src/assistant.rs`
  passes tool output and fetched text into a model prompt with no fence
  (`rg -i "untrusted|fence" assistant.rs` empty; Sekar 12). Wrap each in
  labelled data fences with a "data, not instructions" line and strip tag-like
  markup. Diff: regular. Tier B. Size S. Owns the prompt-build hunk of
  `assistant.rs`. Accept: `cargo test -p engine-endpoint --features mcp --lib
  fence` x2: 10 planted-instruction fixtures make 0 unlisted tool calls. F2P:
  no fence. P2P: endpoint tests green. needs: none.
- [ ] AX26 endpoint: long calls report progress — `play.run` uses a custom poll
  and cancel; "progress" is only a comment (`crates/endpoint/src/mcp.rs:924`).
  Check the current MCP spec first (`mcp.rs:49-51` notes 2026-07-28 changes),
  then send `notifications/progress` for `play.run`, and add a transcript test
  (initialize, tools/list, a good call, an error, an unknown tool). Diff:
  regular. Tier B. Size S. Owns the progress hunk of `mcp.rs` and the test.
  Accept: `cargo test -p engine-endpoint --features mcp --lib transcript` x2.
  F2P: no progress message. P2P: endpoint tests green. needs: none.
