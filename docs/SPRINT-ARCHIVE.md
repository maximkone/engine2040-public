# SPRINT-ARCHIVE — moved board rows (history, not the live board)

The live board is `docs/SPRINT.md`. Rows below were moved WHOLE (byte-identical)
from it on 2026-10-02 (F06 board diet): done rows with no tick evidence newer than
2026-09-18, none of them a `needs:` target of a row staying on the board and none
a FINISH-RICH gate row — so `node tools/workspace-check.mjs`, `node tools/finish.mjs`
and `node tools/board-slice.mjs --find <ID>` keep resolving every id they need.
Never unticked, never edited: revive by moving a row back onto the board.
Earlier history also lives in git history (`git log -p -- docs/SPRINT.md`).

Move ledger: 49 rows on 2026-10-02 (F06).


### From: Finish line (Tier A, in dispatch order) (3 rows)

- [x] PF12 perf: arena allocation shape — `arena_tick*` baselines drifted
  (wave 44: recorded 58753 B/52 allocs vs live 95377 B/358 allocs, hashes
  identical; suspect GP15 per-tick resolver churn). Either fix the per-tick
  allocs and restore the 52-count shape, or adopt the live rows with the
  hash pinned and a comment naming the churn source. Tier A. Size S. Owns
  `crates/arena` resolver path (numbers reported; baselines landed by the
  orchestrator). Accept: `honest_rows_match_live_runs_exactly` green;
  golden `d57b5c8ae3096614` x2 unchanged. needs: PF1, GP15.
  Wave 45: match_live flag skips resolve/log on cold path (GP15 churn
  site named); honest suite green, perf lib 46 pass, arena 70+72;
  checker PASS, judge PASS (consumer bytes unchanged).
- [x] EV12 proof: version-gated golden receipts — `engine_version` enforced
  in the legacy golden gate (ADV46 wave 46: a stale-version receipt with
  the right hash passes today; the tape carries no build field).
  Tier A. Size S. Owns the version assert in `tools/proof.mjs` (+ tape
  field if needed). Accept: stale-version receipt fails loudly;
  current passes; golden unchanged. needs: none.
  Wave 47: 4-line assert (stale + missing FAIL, current PASS
  demonstrated); checker PASS, judge TICK.
- [x] EV17b editor: per-node pose-baked draw — EV17 drew every node but
  poses did not follow gizmo edits (`DrawItem` carried mesh+material
  only). Tier A. Size M. Owns render `DrawItem` hunk (or pose-baked
  upload in `ViewportSnapshot::capture`) + `crates/editor/src/app.rs`
  pose mapping. Accept: moved node re-renders at the new pose + wall /
  crate / hero geometry visible + editor + render suites green. needs:
  EV17. (EV17 follow-up wave 54.)
  Wave 54: TICK (pose-baked stand-ins in build_draw_list + capture,
  moved_node_rerenders_different_pixels + wall/crate/hero visible,
  editor 268/0, render 432/0, clippy/fmt clean, gates + quality PASS;
  ED12 filed for the pose_digest core-owner swap).

### From: Finish quality (6 rows)

- [x] MCP14 mcp: vault-backed one-child secret injection for launched
  servers. Tier A. Size S. Owns launcher hunks beside `e2040-mcp`
  (door frozen). Accept: 20 launches, 0 canary bytes outside the
  child; audit holds key-names-only; headless keyfile path works.
  needs: MCP5, MCP7. (PLN46 verdict TAKE.)
  Wave 47: 20/20 canaries clean, audit key-names-only, BadKeyfile
  headless; 10 tests; checker PASS, judge TICK (live launch is
  operator run).
- [x] ED9 editor: automatic inspector — rows built from the RC4 component
  schemas with min, max, step, dropdown and read-only hints; every edit goes
  through a command batch with undo (bevy-inspector-egui pattern, MIT, no
  bevy_reflect). Tier B. Size S. Owns `crates/editor/src/inspector_auto.rs`.
  Accept: 20 schema types render; 5 out-of-range edits refused with the
  legal range named. needs: RC4, ED7. (Wave 32 CHECK FAIL: grid lands
  tested but shell never calls it — retry wave 33 wiring a shell caller.)
- [x] ED11 editor: rename/move-proof scene handles — path-string lint
  (Godot rename-breakage counterexample; Fyrox Handle discipline
  stands) (READER-WILD wave 51). Tier B. Size S. Accept: 20 seeded
  rename/move ops, 0 dangling refs, outliner/selection stable.
  needs: ED10. (WILD.)
  Wave 52: TICK 361fef7 (judge 3/4).
- [x] ED12 editor: pose-digest hash behind the core owner — EV17b's
  `pose_digest` mixes with a local 131 multiplier instead of the
  `engine-core` FNV owner (CODELAW 15). Tier C. Size XS. Owns the
  `engine-editor` → `engine-core` dep edge + `pose_digest` swap in
  `crates/editor/src/app.rs`. Accept: key bytes identical x2, editor +
  render suites green, quality PASS. needs: EV17. (EV17b follow-up.)
  Wave 54: TICK (Fnv64 owner, keys x2 identical, editor 268/0, deny ok,
  gate + quality PASS).
- [x] RV3 endpoint: stacked injection score-gate at import. Tier A.
  Size S. Owns gate hunks beside `crates/endpoint/src/injection/`
  (RV1/RV2 frozen). Accept: +20% catches over RV1+RV2 on the tape;
  0 FP on 20 legit briefs; import-time only; sim bytes x2. needs:
  RV1, RV2. (PLN46 verdict TAKE.)
  Wave 47: 12/22 baseline -> 22/22 stacked (+83%), 0/20 FP,
  ablation proves stacking, determinism x2; door wired at collect
  (refuse_injected via import_screen + 4th door test green);
  checker split noted (mirrors v0 observer pattern), judge TICK.
- [x] PQ7 playtest: P3 fight-and-AI measurement — the PQ2 tournament run at
  3 difficulties, reporting the P3 numbers. Tier A. Size S. Owns
  `target/evidence/p3/` (tournament report). Accept: AI win rate 40–60%
  per difficulty over 200 fork matches; no single move above 35% of kills;
  zero stuck fighters; parry, riposte and clash each reachable in the
  tapes; the report's evidence line lands in the `FINISH-RICH.md` status
  table. needs: PQ2, GP1, GA1. (Wave 39: 17 tournament tests green,
  report in `target/evidence/p3/`; no P3 status slot exists in
  FINISH-RICH.md — evidence in board receipt + handoff instead.)

### From: Code quality (1 rows)

- [x] CQ4 scene: deterministic containers — codec and prefab HashMap and
  HashSet (10 uses) become BTreeMap and BTreeSet; scene adopts
  `tools/sim-clippy.toml`. Tier A. Size S. Owns `crates/scene/src/codec.rs`,
  `crates/scene/src/prefab.rs`. Accept: scene nondet 0; codec bytes
  identical on the fixtures before and after; config adopted. needs: CQ1.
  Wave 57 TICK on COV-CQ evidence: nondet 0, codec/prefab untouched
  (bytes identical by construction), `crates/scene/clippy.toml`
  byte-identical to tools, scene 56/0 green (orchestrator re-ran).

### From: Accepted plan rows (Tier A/B future; dispatch like Finish line rows) (9 rows)

- [x] GP19 game: staged camera rig under the GP3 director. Tier B. Size S.
  Owns stage hunks beside `crates/game/src/camera.rs` (director API
  frozen). Accept: 600-tick 2-fighter orbit keeps both framed every tick;
  bypass-one-stage still frames; trauma x2 identical; 0 allocs;
  feed <= 0.05 ms. needs: GP3, GP16. (PLN45 gameplay, verdict TAKE.)
  Wave 46: 600/600 framed, bypass parity, trauma x2, 0 allocs, release
  probe < 0.05 ms (4000 rounds); camera 56 green; checker PASS,
  judge TICK (WIN1 binary wiring follow-up).
- [x] GP20 game: intent/smoothed-split camera damping. Tier B. Size S.
  Owns sync hunks beside `crates/game/src/camera.rs` + `camera_feed.rs`.
  Accept: intent bypass identical to raw; sim bytes x2 identical at
  30/60/120 Hz; no framed-tick loss vs GP3. needs: GP3, GP16.
  (PLN45 gameplay, verdict TAKE.)
  Wave 46: bypass identical raw, sim x2 at 3 rates, 600/600 framed,
  0 allocs; checker PASS, judge TICK.
- [x] GP23 game: phased move-table schema (startup/active/endlag +
  cancels). Tier B. Size M. Owns table hunks beside
  `crates/combat/src/exchange.rs` + `sweep.rs` (resolvers frozen).
  Accept: 2 fighters fully phased; cancels window-gated; PQ2 band holds
  x2; 0 float. needs: GP1, GP2, GP12. (PLN45 gameplay, verdict TAKE.)
  Wave 46: V0-legacy/V1-phased flag in match step, phased PQ2 118/82
  x2, 0 allocs, 0 float, golden x2; arena 75 + combat 122; checker
  split noted (v1 observers mirror v0 surface, Q1/RL11 consumers named),
  judge TICK (snapshots stay V0 follow-up).
- [x] AX14 mcp: session-isolation chaos proof (READER-P2 wave 54; needs
  MCP5, MCP8 ticked ✓). Tier A. Size S. Owns chaos hunks in
  crates/endpoint (no new pass code). Accept: 12 proposers x 1 object,
   zero unintended mutations, every conflict typed with fresh revision,
   p95 within +10% single-client, play.run never stalls, suites green.
   needs: MCP5, MCP8. (P2.)
  Wave 54: TICK (session_chaos 6/0 test-only, endpoint 300+ all green,
  clippy/fmt clean, gates PASS).
- [x] AX15 mcp: tool-budget ledger gate (READER-P2 wave 54; needs MCP7,
  API1 ticked ✓). Tier A. Size S. Owns ledger test + cost table in
   crates/endpoint/src/mcp.rs. Accept: per-tool cost table; seeded
   +500-char bloat fails gate; headroom <~2k forces raise-with-reason
   or refusal. needs: MCP7, API1. (P2.)
  Wave 55: TICK (cost table + ledger gate, bloat/headroom refusals,
  endpoint 303+ all green, gate + quality PASS; row-authorized
  32768→36864 budget raise with dated reason, 54-char headroom was
  below the 2k floor).
- [x] AX16 provider: trace export for prompt evolution (READER-P2 wave 54;
  needs EV1, QA1 ticked ✓; unblocks BET-AX13). Tier C. Size S. Owns
   export in crates/playtest + format docs. Accept: 75-row export
   re-imports byte-identically twice; unknown versions refuse loudly;
   golden unchanged. needs: EV1, QA1. (P2.)
   Wave 54: TICK (playtest 171/0, round-trip x2 + refusal tests, clippy
   clean, quality PASS, gates PASS).
- [x] AX17 playtest: second-trace agreement probe for the held refits.
  Tier C. Size S. Owns agreement-probe hunks beside
  `crates/playtest/src/trace_export.rs` (trace format v1 frozen).
  Accept: two green AX16 exports on different seeds each re-import x2;
  held-out agreement numbers posted with 0 success loss, advisory only;
   graduates BET-AX13's two-green-exports rule or kills with numbers;
   golden unchanged. needs: EV1, AX16. (PLN20 ai-native.)
  Wave 55: TICK (two green exports re-import x2, 0 success loss
  advisory, playtest 180/0 with SP42-PLAY combined, gates + quality
  PASS).
- [x] SP32 spatial: probe-sync affordance gold — sync re-probe protocol over
  coarse reachability, scored through the SP12 harness with an independent
  gold (never sharing probe math). Tier A. Size S. Owns gold + probe-sync
  hunks beside `crates/playtest/src/spatial_eval.rs` + `crates/spatial`
  probe (SP12 harness frozen). Accept: oracle agreement + probe ms/object
  (budget <= 0.2) reproduce x2; ADOPT numbers wave-49 spike49 (87.8% vs
  71.1%, ~0.001 ms/object). needs: SP12. (SPIKE49 ADOPT.)
  Wave 50: packet LANDED (26281a6, live probe + independent gold +
  LiveProbe wire + dep edge, playtest 147 spatial 125, harness
  untouched); judge PASS depth 4/5 RUST 4/5 (22 held-peg gaps proven
  by design, Mirror test-only). TICK.
- [x] API7 endpoint: document the 38 `undoc_pub` — Insight/log/play/
  scene_query DTOs + brief/canon/receipts/transport stragglers. Tier B.
  Size S. Owns DTO docs only (MCP18 first). Accept: `undoc_pub` 38→0;
  feeds API2. needs: MCP18. (API review wave 49.)
  Wave 55: docs-only 11 files, undoc_pub 38→0, endpoint tests green, packet-gate PASS, judge PASS, rust-quality 79 PASS. TICK.

### From: Hard stuff (operator 2026-09-24): motion, ragdoll, physics, speed (2 rows)

- [x] PF7 perf: zero-copy asset load — rkyv archived bundles memory-mapped.
  Tier B. Size M. Owns `crates/assets/src/archive.rs`. Accept: load time
  against AS1 charted; a corrupt archive is refused. needs: AS1, AS2.
- [x] PF8 perf: 64-player server budget — dedicated server tick p99 under
  4 ms with 64 players and 500 AI on the reference CPU, per-system
  breakdown. Tier C. Size M. Owns `crates/perf/src/server_budget.rs`.
  Accept: budget report from a recorded siege tape. needs: PF1, ON3. Tick audit 2026-10-19: accepted as closed by its replacement (docs/plan/2026-10-19-owner-queue.md); the named perf `server_budget.rs` is gone.

### From: Empire (26 rows)

- [x] GE1 proof: fixture genres — a top-down twin-stick, a vehicle course on
  the D1 vehicle math and a third-person traversal course each boot, play and
  replay headless through public engine APIs with zero engine edits; the
  shipped game stays Rome. Tier C. Size L. Owns `fixtures/games/`. Accept:
  three fixtures green in proof; engine diff empty; hash x2 each. needs: GP5.
- [x] GE9 proof: generality ratchet in the coverage loop — proof runs the
  GE2 grep gate plus the GP5 engine-diff-empty gate every wave. Tier A.
  Size S. Owns the proof gate wiring (orchestrator lands `tools/` edits).
  Accept: a seeded engine edit for a fixture fails proof; GE1 fixtures stay
  listed until green. needs: GE1, GE2.
- [x] LW9 ai: memory save/load + budget audit — versioned per-NPC save
  envelope over the LW2 bytes (v1 stays readable; unknown versions refuse
  loudly) plus a cap audit that fails when any NPC exceeds 128 stored
  records. Tier B. Size S. Owns the save envelope in
  `crates/ai/src/memory.rs` + audit test. Accept: round-trip
  digest-identical twice; 10k-event soak holds caps; corrupt bytes refuse
  with a reason. needs: LW2, LW6.
- [x] AB5 mcp: planned encounters — the model turns a brief into goals in a
  small planning language and a classical planner in plain Rust solves them;
  the model never plans (PlanBench 2206.10498 shows why; 2304.11477 for the
  translation). Fast Downward is GPL: never a dependency. Tier B. Size M.
  Owns `crates/endpoint/src/plan_goals.rs`. Accept: 10 briefs give encounters
  inside the win-rate bounds; a seeded bad plan is refused by the fork check;
  at most 5 attempts per brief. needs: AB1, AB3. (Wave 33: planner proven;
  wave 34: `sim.plan_goals` registered — PlanGoalsTool + router 19→20,
  budget 29544/32768 measured, redteam/stdio/discover/startup consistent,
  judge re-PASS.)
- [x] AB13 mcp: dedup review-pipeline bind — the AB1–AB3 review path
  calls `dedup` so near-duplicates collapse before review. Tier B.
  Size S. Owns caller hunks in `crates/endpoint/src/` stage files
  (the `canon.rs` API stays frozen). Accept: 30 seeded sets shrink
  ≥30% through the pipeline with 0 false merges; suites green.
  needs: AB1, AB6. (Wave-39 lesson: file the adopter or adoption
  rots; wave 40: 3 stage callers wired, endpoint 268 green, checker
  PASS.)
- [x] AB8 mcp: ludus-visit vertical slice — AB1 builds a practice corner of
  the ludus over WD2, LW3 voices staff it, one scripted visit plays and
  replays exactly with an eval line. Tier C. Size M. Owns glue in
  `crates/endpoint/src/build_game.rs` call sites (AB4's file, slice flag
  only). Accept: 5 briefs boot, play 2 minutes and replay x2 with
  providers off; every write undoes cleanly. needs: AB1, AB7, LW3, WD2.
  (Wave 32 BLOCKED first occurrence: `build_game.rs` never landed — AB4
  open — plus endpoint has no edge to game/world and AB1 keeps a private
  envelope. Retry wave 33 only after WD6→AB4 lands with the edges.)
  (Wave 35: TICKED — slice flag +593/-0 purely additive, O(n²) tick
  honesty clause documented, 18 build_game tests green, judge PASS.)
- [x] SP11 mcp: intent macros — `scene.bootstrap`, `create.player`,
  `create.enemy`, `create.camera`, `arrange` (grid, circle, scatter, along a
  wall) and `furnish` compile to SP5 placements in one undoable batch, so an
  agent needs about 10 intents instead of 70 raw operations (O3DE AI
  companion intent API, MIT OR Apache-2.0, as a pattern). Tier B. Size M.
  Owns `crates/endpoint/src/intents.rs`. Accept: 20 briefs build with at
  least 50% fewer calls than raw placement, same checks clean, one undo
  reverts each. needs: SP10, ED7. (Wave 29: SP10 `scene.query` logic lands
  router-unregistered to hold the 18-tool gate; its SceneQueryTool struct +
  registration + budget 18→19 rides with this row.)
- [x] SP42 spatial: wire compose rankings into consumers — game director
  calls `best_shot` for hero framing + playtest look-harness calls
  `rank_order`/`spearman_permille` for the 20-shot panel (REVIEW-W54:
  zero non-test callers today; split one packet per crate). Tier B.
  Size S. Owns the two call-site hunks (compose API frozen) + compose
  doc corrections (allocation scope + consumer claims). Accept: both
  call sites green + panel rho holds + zero dead pub fns in compose.
  needs: SP13. (REVIEW-W54.)
  Wave 55: TICK (game director picks hero focus via best_shot, game
  254/0; playtest look harness ranks via rank_order/spearman with
  parity tests, playtest 180/0; game→spatial edge approved + lock;
  gates + quality PASS; workspace edge registration → EDGE-GAMESPATIAL
  packet).
- [x] SL17 ops: one owner for tick-tuning constants — `SNAPSHOT_EVERY_TICKS`
  + `STARVE_TICKS` are defined in both net and perf (janitor wave 26,
  CODELAW-1; game already imports from net). Tier A. Size S. Owns the
  move (core if tick-cadence, else perf→net edge) + import migration.
  Accept: exactly one definition site; both harnesses green; deny green.
  needs: none.
- [x] SL18 anim: locomotion buffer reuse — `Locomotion::update` allocates
  3/fighter/tick (pose positions, rotations, contacts); root-only advance
  or pose/contacts buffer-reuse entry for the authority tick. Tier A.
  Size S. Owns `crates/anim` reuse hunks. Accept: steady floors drop by
  the locomotion share; golden unchanged. needs: none. (Completes SL6;
  wave-28 attribution.)
- [x] SL19 destruction: support-refresh scratch reuse — `refresh_support`
  allocates 3/tick (supported map + work stack); scratch-buffer reuse
  plus `drain_into` for break events. Tier A. Size S. Owns
  `crates/destruction` reuse hunks. Accept: steady floors drop by the
  refresh share; golden unchanged. needs: none. (Completes SL6.)
- [x] SL20 combat: event-buffer reuse — event/break ticks allocate
  (+365 allocs/600 ticks @4f vs idle); `try_step_into` with a reused
  event buffer, quiet ticks already 0. Tier A. Size S. Owns
  `crates/combat` reuse hunks. Accept: AI-workload allocs drop by the
  event share; golden unchanged. needs: none. (Completes SL6.)
- [x] SL22 ops: pool-contention discipline — two lanes racing one
  `target/pool-*` dir serialize on the build-dir lock with green builds
  (ADV1 finding 4: queue time, never corruption); the lanes tool carries
  a comment naming the lock behavior and `target/` stays out of git.
  Tier B. Size XS. Owns the `tools/lanes.mjs` comment (orchestrator
  lands it). Accept: two lanes racing pool-3 both finish green with
  `Blocking waiting for file lock on build directory` in one log.
  needs: none. (ADV1 wave 32, fingerprint
  `adv1-pool3-build-dir-lock-serializes-two-lane-builds-no-corruption`;
  wave 41: comment landed, syntax checked, ADV1 evidence cited.)
- [x] SL25 ops: one digest helper in spatial — FNV basis/prime inlined at
  6 digest sites (checks, index3, relation x2, staged, sight)
  (JAN50-2). Tier B. Size S. Owns one helper + 6 call-site swaps.
  Accept: `cargo test -p engine-spatial` green + quality `hash_reimpl`
  down. needs: none. (JAN50.)
  Wave 51: packet LANDED (831e644, one helper + 6 swaps, 125 green);
  checker PASS, hash_reimpl 15→1, score D→C. TICK.
- [x] SL26 ops: typed parse errors in playtest — hand-rolled JSON
  parsers in keypoint/living/spatial_eval return `String` errors
  (JAN50-3). Tier B. Size S. Owns one shared `ParseError` + tokenizer
  core (grammars stay separate). Accept: `cargo test -p
  engine-playtest` green + quality `string_errors`/`dup_fns` down.
  needs: none. (JAN50.)
  Wave 51: packet LANDED (9e43975, parse module + 4 files, 153 green);
  checker PASS, string_errors 50→0, score 75. TICK.
- [x] SL39 game: unify the three local FNV-1a hex bodies —
  `capture.rs:546 fnv_hex` (+`536 fnv1a`, `551 fnv_mix`) vs
  `package.rs:324 digest_hex` vs `bin/staged_fight.rs:52 fnv_hex`.
  Tier B. Size S. Owns the digest-helper hunks in
  `crates/game/src/capture.rs` + `crates/game/src/package.rs` +
  `crates/game/src/bin/staged_fight.rs`. Accept: game suite green +
  `dup_fns` 11 down + goldens unchanged. needs: none. (JAN53-F1.)
  Wave 54: TICK (checker PASS; single fnv_hex owner, 239 green). Tick audit 2026-10-19: accepted as closed by its replacement (docs/plan/2026-10-19-owner-queue.md); the named game `package.rs` is gone.
- [x] SL40 game: unify the drifted SplitMix bodies behind the core owner
  — `camera.rs:605 splitmix64` (by-value) vs `capture.rs:561 splitmix64`
  (by-state) vs `camera.rs:1279 fixture_mix` + `1287
  fixture_splitmix_next` + `1294 fixture_unit_f32`. Tier B. Size S.
  Owns the splitmix hunks in `crates/game/src/camera.rs`. Accept: game
   suite green incl. shake/trauma + `hash_reimpl` 17 down + golden x2.
   needs: SL39. (JAN53-F2.)
  Wave 54: TICK (camera.rs only, game 251/0, golden x2, hash_reimpl 17→15,
  dup 11→9, gates + quality PASS; streams byte-identical since
  GOLDEN_GAMMA equals the old increment).
- [x] SL41 arena: zero-alloc recorder path — `recorder.rs:171 entries
  Vec::with_capacity` per step + `recorder.rs:257 snapshot()` clone per
  tick. Tier A. Size S. Owns scratch hunks in
  `crates/arena/src/recorder.rs` (+ call-site threading). Accept:
   counting-allocator 0 allocs after warmup + arena suite green + golden
   x2 + `hot_alloc` 1→0. needs: none. (JAN53-F3.)
  Wave 54: TICK (hoisted scratch + snapshot_into/as_slice, 2 clones → 0
  after warmup pinned by counted tests, arena 84/0, golden x2, gate +
  quality PASS).
- [x] SHOW2 game: auto director — a spectator and trailer camera that picks
  shots from sim events (hit, parry, KO, comeback, crowd) with continuity
  rules (180-degree line, match on action, no cut under 1 s, slew cap for
  comfort), presentation only, never in the hash. Tier B. Size M. Owns
  `crates/game/src/director.rs`. Accept: the cut log is identical twice for
  one tape; a seeded jump cut and a line cross are flagged; costs ≤ 0.05 ms
  per frame. needs: SHOW1, MP4.
  Wave 50: packet FAILed judge (depth 2/5: unwired — no production
  caller; ignored budget probe; vacuous 180-line; inverted
  match-on-action) and REVERTED (29f9a24). Repair must: hold Director
  in App + drive from real cues/positions, capture reads cuts/digest,
  release `--ignored` probe receipt. Stays open.
  Wave 51: REPAIR LANDED (3adf712, F1-F4 fixed, game 233+2, release
  probe 0us, gate+packet-gate PASS); judge PASS depth 4/5 RUST 4/5.
  TICK.
- [x] ADV52a tools: showcase string-frames hash bypass — string entries
  skip sha verification (ADV52 wave 52, fixtures `target/adv52/`).
  Tier A. Size S. Owns `tools/showcase.mjs` hunks. Accept: string
   tamper FAILs without flag; SHOW1 flow unaffected. needs: none.
   (ADV52.)
  Wave 52: TICK ae98ba8 (checker PASS-code; duel exit 0, strframes exit 1).
- [x] ADV52b tools: PF17 reseal forgery — row+seal together recompute
  equal on committed tamper (ADV52 wave 52). Tier A. Size S. Owns
   `crates/perf` seal hunks (keyed seal or two-person rule).
   Accept: committed reseal FAILs loudly; clean regens pass.
   needs: PF17. (ADV52.)
  Wave 52: TICK 617eb1c (judge 4/4).
- [x] ADV52c tools: director cutlog unbound from capture receipt —
  forged cutlog passes check (ADV52 wave 52). Tier B. Size S. Owns
   game capture/showcase glue. Accept: forged cutlog FAILs; real
   captures pass. needs: SHOW2. (ADV52.)
  Wave 52: TICK 40dd1ff (judge 3/3).
- [x] ADV55b arena: V1 observer flag + phased cursors dropped across
  snapshot/resume/fork — `MatchSnapshot` carries `match_state` only,
  `from_state` resets V0, so a V1 prefix/resume or fork suffix silently
  resolves legacy (ADV55 wave 54). Tier A. Size S. Owns
  `crates/arena/src/lib.rs` snapshot/resume + `match_wire.rs`
  `from_state` + `fork.rs` restore hunks (carry version + phased cursors
  or refuse V1 snapshots loudly). Accept: V1 prefix/resume suffix equals
  uninterrupted V1 rows tick-for-tick, V0 unchanged, unknown version
  still empty; suite green. needs: AQ4. (ADV55.)
  Wave 54: TICK (version + phased cursors carried via build_sim one-owner
  restore + run_prefix_versioned, V1 prefix/resume tick-identical, V0
  unchanged, unknown refused, arena 87/0, golden x2, clippy clean, gate
  + quality PASS).
- [x] AQ1 arena-docs: document `run_match_events` + repair the
  TickMatchEvents sentence + gate rustdoc links on `sim` (Controller
  link) and drop the private-`Sim` link in fork docs. Tier C. Size XS.
  Owns `crates/arena/src/lib.rs:331-351`, `crates/arena/src/fork.rs:1-28`.
  Accept: `cargo doc -p engine-arena --no-deps` zero warnings both legs
   + `undoc_pub` 1→0 + suite green. needs: none. (API-REVIEW wave 53.)
  Wave 54: TICK DOCS-ARENA (rustdoc + 5 passing doctests for try_new /
  run_match_events(+V1/bytes), TickMatchEvents sentence fixed, doc
  warnings 6→0 both legs, undoc 1→0, gate PASS).
- [x] AQ3 arena-ctor: additive `ArenaConfig::try_new(seed, ticks,
  fighters, half_extent_m) -> Result<Self, ArenaError>` (literal +
  `validate` in one step; existing literal path untouched). Tier B.
  Size XS. Owns `ArenaConfig` impl + one test. Accept: `try_new`
  rejects each bad field with the typed culprit variant + suite green.
   needs: none. (API-REVIEW wave 53.)
   Wave 54: TICK (checker PASS; arena 77 lib green).
  Wave 54 docs: rustdoc + doctests landed via AQ1 (DOCS-ARENA).
- [x] AQ4 arena-version: additive versioned match entry with published
  version consts; suffixed `_v1` fns become thin wrappers. Tier B.
  Size S. Owns `match_wire` const visibility + wrappers + equivalence
  test. Accept: v0/v1 outputs byte-identical through both spellings +
   suite green. needs: none. (API-REVIEW wave 53.)
   Wave 54: TICK (checker PASS; 80 green).
  Wave 54 docs: versioned-entry rustdoc + doctests landed via AQ1
  (DOCS-ARENA); ADV55b filed as snapshot/resume follow-up.

### From: 2040 tier (operator 2026-09-24, Finish C; dispatch after Finish line rows) (2 rows)

- [x] WD4 game: siege mode — 32v32 with AI legions filling ranks, siege
  engines on ballistics, walls through DX1–DX3 destruction. Tier C.
  Size L. Owns `crates/game/src/siege.rs`. Accept: a bot-filled 64-slot
  siege plays 10 minutes on a dedicated server and replays exactly.
  needs: WD1, ON3, DX4.
- [ ] WD10 game: siege engines and ballistics — deterministic
  projectile sim (fixed-point, sorted impacts) firing siege engines
  at walls through DX1–DX3 breaks under ON3 interest. Tier C.
  Size M. Owns `crates/game/src/ballistics.rs`. Accept: a scripted
  bombardment breaks the same chunks twice; wall break converges
  on all clients; replay exact. needs: WD4, DX4. Tick audit 2026-10-19: LATER per docs/plan/2026-10-19-owner-queue.md; the named game `ballistics.rs` is gone. Correction 2026-10-23: unticked, not done. The removal commit d89bd98e0 deleted `crates/game/src/ballistics.rs`, so the Accept (a scripted bombardment breaks the same chunks twice; replay exact) cannot run. PX41 records the same removal as an obsolete close.
