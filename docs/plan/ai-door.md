# AI door systems — plan to the VISION.md bar

Five FOCUS Now slices as one system: a live model drives a gladiator inside a
recorded bout (AL1), door packs for net and perf (DP1), replay verify/bisect
verbs (RB1), the playtest verbs rival doors have (LF16), and the public
benchmark mirror (HS20). Owner lanes: provider, endpoint, playtest, arena, ai.
Prefix AID (new rows only; the five boarded rows keep their IDs). Base commit
`576cdc9726885ea65f936abff1442c511b86c05f`. Tag NOW, feeds the AI-door Core
proof (`cargo test -p engine-playtest --lib mirror` x2, plus each row's Accept).

## 1. Where it stands

Code beats docs. Status words below are what the crates confirm today; a
library function is not a door verb, and a mock-green test never certifies a
vendor wire or a real client.

| Capability | Status | Evidence |
|---|---|---|
| Live bout stepping (EG1) | unit-tested, integrated, ticked | `crates/arena/src/live.rs` (`LiveMatch::step`, tick log) |
| Tick-stamped model actions | unit-tested library, no bout wiring | `crates/ai/src/mind.rs` (validate → stamp `issued + delay` → `ActionLog::drain_due`; `apply.rs` window checks) |
| Provider-driven fighter (AL1) | absent | no `live_mind` bout test in `crates/arena`; replay-with-provider-off unproven |
| Net pack (DP1 half) | unit-tested, integrated | `crates/endpoint/src/packs/net.rs` (`net.loopback`, loss/latency/burst knobs, seeded-desync tick test, `api.describe` listing) |
| Perf pack (DP1 half) | unit-tested, integrated | `crates/endpoint/src/packs/perf.rs` (`perf.table`, fixture-csv parse, newest-csv pick, read-only listing) |
| DP1 remaining | proof-line wording only | full endpoint lib count + golden in the proof line |
| Replay library (RB1 substrate) | unit-tested | `crates/playtest/src/causal.rs` (`verify_replay`, `bisect_replay`), `replay.rs`, `seeded_bug.rs` (`SEEDED_TICK = 8`) |
| Replay door verbs (RB1) | absent | no `replay.verify` / `replay.bisect` in `crates/endpoint/src`; F2P holds |
| `play.step` (EG16, LF16 base) | unit-tested, integrated, ticked | `crates/endpoint/src/play/play_session.rs` (digest per step, session bout) |
| `play.step_until` (LF16 third) | unit-tested | `crates/endpoint/src/packs/play.rs` (`hp[1] < 900` stops on first tick) |
| `input.inject`, `state.digest` (LF16) | absent | no matches in `crates/endpoint/src`; verbs unknown |
| Mirror slices (HS29–HS31 substrate) | unit-tested | `crates/playtest/src/mirror_gdb.rs`, `mirror_craft.rs`, `mirror_logic.rs` (10 tasks each) |
| Mirror aggregate (HS20/HS32) | unit-tested | `crates/playtest/src/mirror.rs` (`run_mirror`, `parity()`, `RIVALS` UNKNOWN cells, recorded-replay backends, no model/socket/clock) |
| HS20 remaining | parity + honesty wording | 30/30 parity assert and UNKNOWN-with-reason print in the proof line |
| Eval harness (EV1, HS20 base) | unit-tested, integrated, ticked | `crates/playtest/src/eval.rs` (75 rows + external line) |
| Tool-list budget | enforced by gate | `TOOL_LIST_BUDGET_CHARS` + ledger beside `crates/endpoint/src/mcp.rs`; every new verb pays its chars (see §3) |
| Engine Insight index (API1) | unit-tested, integrated, ticked | `crates/endpoint/src/insight/` + `fixtures/insight-gold.json`; AID1 walks it via `api.describe` |
| External MCP client E2E | missing (operator-side) | scripted list > preview > apply > undo needs operator hosts; Tier B, not this plan |

Not this plan (no duplicates): HS29 HS30 HS31 HS32 stay in `hard-systems.md`;
SP61 SP64 stay in `spatial.md`; EV/AX/MCP/EA rows stay in `ai-native.md`;
`look.judge` (LJ1) and the agent scorecard (AG1) stay where boarded.

## 2. Finish line

- **Tier A (alpha, this plan):** a mock provider with fixed latency drives
  fighter 0 for 600 ticks and the taped replay with the provider off gives the
  same final hash (AL1); `net.loopback` + `perf.table` green x2 behind the
  `mcp` feature with the proof line carrying the full endpoint lib count and
  the golden (DP1); `replay.verify` / `replay.bisect` name the seeded tick on a
  dirty tape and verify clean tapes (RB1); `step_until` stops on the first
  `hp < 900` tick, `inject attack` for 3 ticks moves the bout like the same
  `FighterInput`, `digest` equals the replayed hash (LF16); the 30-task mirror
  prints identical mock/local/remote scores beside published figures or
  UNKNOWN with reason (HS20); every new verb is listed by `api.describe` with
  its capability card (AID1). Measurable: each Accept's command x2, golden
  `d57b5c8ae3096614` unchanged on every row.
- **Tier B (v1, not this plan):** live vendor wires and a scripted external MCP
  client E2E (operator keys/hosts missing); rival doors actually run instead of
  UNKNOWN; AL slice 2 (late-reply fallback to the scripted brain with a logged
  reason, multi-fighter driving).
- **Tier C (2040 tier, not this plan):** the mirror as a published scoreboard
  beside rival engines' own numbers; weak-model packet kit; agent scorecard
  every 5 waves.
- **CUT (never):** arbitrary execute/shell/file-write tools; model output
  entering the tick unrecorded; weights in git; vendor-specific abstractions
  ("OpenAI-compatible" stays one adapter, per the system spec).

## 3. Target design

Newcomer first: the model always runs outside the tick. Its replies are
validated, stamped for tick N+k, drained at the tick boundary into the bout
tape, and replay reads the tape with the provider off. The door is the only
way in: one typed verb per action, each undoable or read-only, refused by
default, priced before it runs.

Data flow: caller runs `api.search` before asking → builds `AiRequest`
(prompt context, capabilities, privacy mode, budget, deadline) → session
policy gates destinations and data classes → adapter negotiates capabilities
→ call with cancel/job handles → `AiResult` (proposal + unknown-stays-unknown
usage) or typed `AiError` → validator checks → fork play-tests balance where
the row needs it → CAP1 approval binds session/revision/action/expiry →
command service applies as one patch → provenance receipt → `log.digest` /
`log.query` record the digest. Replay and mirror paths never call a model:
recorded tapes and fixture bytes only.

Ownership: `provider` owns contracts, adapters, streaming and fallbacks;
`endpoint` owns the MCP surface, packs, guards and the per-tool ledger;
`playtest` owns eval, causal replay, seeded bugs, the mirror and the trace
export; `arena` owns the bout (`LiveMatch`); `ai` owns stamping
(`mind.rs`/`apply.rs`); `game` owns the tuned-fight consumer. RB1 verbs live
as a new endpoint pack that calls the playtest library (no second bisect
implementation). No provider SDK types cross into simulation, scene or
command types.

Threads: adapters, streams and forks run on worker/test threads; the fixed
tick never waits — accepted actions arrive as recorded inputs. The MCP server
serializes proposers on the document write queue with bounded readers (AX14
proves the 12-agent race). Captures ride the service path, never the tick.

Determinism: same seed + same tape → same digest and same bytes; same files
→ same Insight index and ranking; same report → same trace bytes;
export-import-export byte-identical. Backend labels differ by design — parity
pins task rows, previews, forks and effects, never source names. Unknown
costs stay unknown, never zero. Floats never reach the hash; integer cm and
`u64` digests (decimal strings on the wire) only.

Budgets: per-request ms + spend caps; per-session edit budget (CAP1); the
tool list stays under its char budget with the AX15 ledger (measured size +
reason per raise, 2048 headroom floor, +500 bloat refuses) — AID1 asserts the
new verbs fit before any other row lands more; streams cap at 64 chunks /
64 KiB; trace exports cap at 1024 rows; `MAX_DETAIL_CHARS = 256`.

API sketch: `AiRequest/AiResult/AiEvent/ProviderCapabilities/AiSessionPolicy`,
`BackendKind::{Mock,Local,Remote}`, `ActionLog::drain_due`,
`net.loopback(loss_pct, seed, latency, burst) -> {converged, first_tick}`,
`perf.table(csv?) -> rows{scenario, p50_us, p99_us}`,
`play.step_until(seed, tape, cap, fighter, hp_below) -> {tick, stopped, digest}`,
`input.inject(action, ticks)`, `state.digest() -> u64`,
`replay.verify(tape) -> ok`, `replay.bisect(tape) -> tick`,
`run_mirror() -> MirrorReport{to_text()}`, `api.describe` listing all of them.

## 4. Hard problems

| Problem | First approach | Why it works | Reference |
|---|---|---|---|
| A model reply arriving late must not stall or corrupt the tick | stamp `issued + delay`, drain due actions at the boundary, fall back to the scripted brain with a logged reason | the tick reads only due stamped actions; late replies are history, never force | `crates/ai/src/mind.rs`, `apply.rs` window checks |
| Proving a desync finder works, not just prints | seeded bug at tick 8 plus seeded 100%-loss loopback; bisect must name the exact tick x2 | a finder that cannot name a planted tick proves nothing | `seeded_bug.rs`, `causal.rs`, `packs/net.rs` tests |
| Claiming parity without proving it | `parity()` over 30 tasks across mock/local/remote in recorded-replay mode; divergent ids fail loudly | recorded replay removes model nondeterminism, so any split is a real bug | `mirror.rs:54-64` |
| Rival numbers without rival runs | print published figures beside ours, UNKNOWN with reason where we did not run; never subtract | honesty rule from the system spec; a fake comparison is worse than none | `mirror.rs:13-15`, `RIVALS` |
| Verb sprawl breaking agent token budgets | AID1 ledger gate: every new verb listed with measured chars before landing | the tool-list budget is a gate, not a doc line | AX15 precedent beside `mcp.rs` |
| Godot/Unity/Unreal doors already have play verbs | match the trio (`step_until`, `inject`, `digest`) as typed verbs on our sim, not their code | method parity only; implementation stays native and tick-stamped | LF16 board text (satelliteoflove godot-mcp gap note) |

## 5. Donors

Donors: `steals.md`; porting policy is AGENTS rule 4. No donor code is ported
by this plan: rival doors and public suites (GameDevBench, CraftBench-UE,
GameLogicBench shapes; godot-mcp/unity-mcp verb shapes) are method and
task-shape references only, reimplemented natively; benchmark figures are
quoted beside ours, never vendored.

## 6. Slim

- Delete: any second bisect/verify implementation (RB1 calls `causal.rs`); any
  second loopback or perf-csv reader (DP1 extends `packs/net.rs`/`perf.rs`);
  any second mirror aggregator (HS20 uses `mirror.rs`).
- Merge: LF16's three verbs land as hunks in the existing `play` pack beside
  `step_until` (one pack, one capability review); AID1 walks all five slices'
  verbs in one test.
- Freeze: `LiveMatch::step` shape, `ActionLog` stamp/drain semantics,
  `SEEDED_TICK = 8`, mirror slice task ids and `RIVALS` text — rows add hunks
  beside them, never forks.
- We will not build: live vendor adapters beyond the existing four (operator
  keys missing); a real external-client E2E; AL slice 2+ (multi-fighter, live
  fallback tuning); a scoreboard site; weights, training or tuning anywhere in
  these rows (the trace export exists for a future tuner; no tuning happens
  here).

## 7. Board tasks

Pointers first (boarded; slice with `node tools/board-slice.mjs <ID>`), then
one new row, then Row changes. New rows number from AID1.

- [ ] AL1 arena: a live model drives a gladiator inside one recorded bout,
  slice 1: stamped replies — the provider reply is validated, stamped for
  tick N+k and drained at the tick boundary into the bout's tape; replay with
  the provider off reads the tape and gives the same hash. Builds on EG1
  `LiveMatch`. Diff: good (determinism: the provider never runs inside the
  tick; only tape actions enter it). Tier A. Size M. Owns a bout hunk in
  `crates/arena`, the stamp hunk in `crates/ai/src/mind.rs` and one test.
  Accept: `cargo test -p engine-arena --lib live_mind` x2: a mock provider
  with fixed latency drives fighter 0 for 600 ticks; the recorded tape
  replays with the provider off to the same final hash. F2P: no bout test has
  a provider-driven fighter today. P2P: `cargo test -p engine-ai --lib`
  green; golden unchanged. needs: EG1.
- [ ] DP1 endpoint: door packs for the Core systems — `net` (open a loopback
  session, inject loss, latency and drop, read the first divergent tick) and
  `perf` (per-scenario p50 and p99 from the newest quiet perf csv), typed,
  read-only or undoable, refused unless enabled. Tier A. Size M. Owns
  `crates/endpoint/src/packs/net.rs` and `perf.rs` (new) plus their `mod.rs`
  lines. Accept: `cargo test -p engine-endpoint --features mcp --lib packs::net` and
  `packs::perf` x2 (packs live behind the `mcp` feature; the bare `--lib` filter matches 0 tests): a loopback session with 10% loss converges and reports
  its first divergent tick as none; a seeded desync reports its tick; perf
  reads a fixture csv into typed rows; both verbs listed by api.describe.
  F2P: no net or perf pack today (`crates/endpoint/src/packs/`). P2P: `cargo
  test -p engine-endpoint --lib` green; golden unchanged.
- [ ] RB1 playtest: `replay.verify` and `replay.bisect` door verbs — given a
  tape, print the first tick whose hash leaves the recorded chain. Tier A.
  Size S. Owns a verb hunk beside `crates/playtest/src/causal.rs` and
  `seeded_bug.rs`. Accept: `cargo test -p engine-playtest --lib
  replay_bisect` x2: a seeded_bug tape bisects to the seeded tick; a clean
  tape verifies with no divergence. F2P: no bisect verb today. P2P: `cargo
  test -p engine-playtest --lib` green; golden unchanged.
- [ ] LF16 endpoint: the playtest verbs other engines' doors have —
  `play.step_until` (a condition over `sim.observe` fields, tick cap),
  `input.inject` (a named action held N ticks), `state.digest` (hash of the
  live bout), beside EG16's `play.step` (satelliteoflove godot-mcp
  `godot_game_time`, `godot_input`, `godot_runtime_state`; `ai-native.md` verb
  gap). Diff: good. Tier A. Size M. Owns the endpoint play pack hunk. Accept:
  door tests x2: step_until `hp[1] < 900` stops on the first such tick; inject
  `attack` for 3 ticks moves the bout like the same FighterInput; digest
  equals the replayed hash. F2P: the verbs are unknown. P2P: endpoint lib
  green; golden unchanged. needs: EG16.
- [ ] HS20 playtest: public benchmark mirror — 30 tasks shaped like GameDevBench,
  CraftBench-UE and GameLogicBench, run through our door, deterministic checks
  only, scores printed beside each suite's published numbers. Tier A. Size L.
  Owns `crates/playtest/src/mirror.rs`. Accept: same results on mock, local,
  remote; UNKNOWN stays UNKNOWN for rivals we did not run; fills gap G2 and G6.
  needs: EV1.
- [ ] AID1 endpoint: every ai-door verb this plan ships is listed by
  api.describe with its capability card and measured tool-list chars. Tier A.
  Size S. Owns the describe-ledger test hunk in `crates/endpoint/src/packs/`
  (test only; verbs stay in their packs). Accept: `cargo test -p
  engine-endpoint --features mcp --lib packs::describe_ledger` x2 walks
  `net.loopback`, `perf.table`, `play.step_until`, `input.inject`,
  `state.digest`, `replay.verify`, `replay.bisect`: each listed, each with a
  card (Observe for read-only), total chars under the AX15 budget with the
  ledger row naming each verb's cost. F2P: new verbs land unlisted today.
  P2P: `cargo test -p engine-endpoint --lib` green; golden unchanged. needs:
  DP1, RB1, LF16.

Row changes (text deltas on boarded rows, never duplicates):

- AL1: keep. Note the stamp half already exists (`mind.rs` validate → stamp →
  `drain_due`, `apply.rs` window refusals); the missing hunk is the bout
  wiring plus the `live_mind` test. No accept change.
- DP1: code has since landed (`net.rs` latency/burst knobs with seeded-tick
  tests, `perf.rs` fixture parse + newest-csv pick). Change the proof line to
  carry the full endpoint lib test count and the golden hash beside the two
  pack filters. No scope change.
- RB1: library has since landed (`causal.rs`, `replay.rs`, `seeded_bug.rs`
  tick 8). Verbs live as a new endpoint pack calling that library (no second
  implementation); add `api.describe` listing of both verbs to the Accept. No
  scope change.
- LF16: `play.step_until` has since landed (`packs/play.rs` with the hp<900
  test); `input.inject` + `state.digest` are still unknown. Keep one row; land
  order inside it is inject, then digest, then the joint x2. No accept change.
- HS20: aggregate has since landed (`mirror.rs` + three 10-task slices +
  `RIVALS`). Pin the Accept: `parity() == (30, [])` x2 and the printed table
  shows the UNKNOWN count with a reason per unknown cell. No scope change.

### Landing order

Three waves, disjoint paths so they run beside each other, never stacked:

1. Wave 1: RB1 (pack calls the landed library) + AID1 ledger test skeleton
   (asserts the verbs that exist today, grows as the rest land).
2. Wave 2: DP1 proof-line reword + LF16 `input.inject` + `state.digest`
   (both hunks in the `play` pack beside `step_until`).
3. Wave 3: AL1 bout wiring + `live_mind` test, then HS20 parity pin + the
   `--nocapture` table print. AID1 goes green last: it walks every verb.
4. Stop rule per row: two identical failures end the row; the row files one
   `docs/BLOCKERS.md` line with owner, repro and output, and never lands
   green. A row that moves the golden hash is killed on the spot (doors and
   presentation never move it).

## 8. Quality gates

- `cargo test -p engine-arena --lib live_mind` x2 (AL1); `cargo test -p
  engine-ai --lib` green beside it; golden unchanged.
- `cargo test -p engine-endpoint --features mcp --lib packs::net` and
  `packs::perf` x2 (DP1); bare `--lib` green beside them; golden unchanged.
- `cargo test -p engine-playtest --lib replay_bisect` x2 plus the new door-verb
  tests through `api.describe` (RB1); playtest lib green; golden unchanged.
- Endpoint play-pack tests x2: first-trip tick, 3-tick inject equivalence,
  digest-equals-replay (LF16); endpoint lib green; golden unchanged.
- `cargo test -p engine-playtest --lib mirror` x2 with `--nocapture` table:
  30/30 parity, UNKNOWN-with-reason rivals, G2/G6 named (HS20).
- AID1 ledger gate green: total + per-verb chars under budget, +500 seeded
  bloat refuses, sub-2048 headroom refuses (AX15 precedent).
- `node tools/workspace-check.mjs` PASS after any board edit; golden
  `d57b5c8ae3096614` moves only with an explicit receipt (it does not move
  here — presentation and doors only, zero tick bytes).
- Proof separation forever: mock-green never certifies a vendor wire, a unit
  test never certifies the external client, a CPU reference never certifies a
  GPU capture; reports print which columns are fixture-proven vs live-proven.

## 9. Session estimate

- Tier A (this plan): AL1 M (2–4), DP1 M (2–4, mostly proof-line now), RB1 S
  (1, library done), LF16 M (2–4, one verb done), HS20 L (5–10, aggregate done,
  parity wording left), AID1 S (1). Total 6 rows, 11–21 sessions if run in
  order; DP1/RB1/LF16-trio run beside AL1/HS20 (disjoint paths), so 3 waves.
- Tier B: live wires + external E2E, 2–4 sessions, waits operator keys/hosts.
- Tier C: published scoreboard + scorecard automation, 2–3 sessions, waits Tier
  A green.
