# AI door (ai-native)

Pre-merge text: `git show a0e13976:docs/plan/ai-native.md`. Merged in below: `docs/archive/2026-10-10/plans/eyes.md` (section "Eyes", same EV prefix) and `docs/archive/2026-10-10/plans/axiom.md` (one paragraph in section 4). Those two files are archived under `docs/archive/2026-10-10/plans/`.

Goal: any model drives the engine through one typed door; every change is checked, undoable and receipted.

Acceptance:
1. The same proposal on mock, local and remote gives the same outcome vector (end state, digest, fork metrics, refusals).
2. A scripted external MCP client runs list > preview > apply > undo over stdio on a scratch project.
3. Every apply carries approval and provenance; the tool-ledger gate (AX15) stays green; EV21 pointing eval at 100% of 100 gold tasks.

Order: EV21, EV22, EV23 (eyes chain, Tier A), then the table in ID order (AX, EA, MCP, EV rows).

Owner crates: `provider`, `endpoint`, `playtest`, `editor`, `game` (eyes capture), `spatial` (brain substrate, read only).

Open board IDs: AX11 AX12 EA4 EV15 EV16 EV21 EV22 EV23 MCP12 MCP16 MCP17 MCP22 MCP23 MCP25 (ticked and never-boarded rows are dropped).

## 1. Where it stands

Proof separation (read 2026-09-30): status words below are board/plan
receipts for fixture/offline work only — not a real external client or a
live vendor wire (both missing operator-side, Tier B). Reports print which
columns are fixture-proven vs live-proven vs operator-proven (§8); a green
mock never certifies a vendor wire, a unit test never certifies the client.

Capability | Status
---|---|---|
Provider contract C2 (request/result/event/caps/errors) | unit-tested, integrated | `crates/provider/src/core.rs`, `protocol.rs`, `policy.rs`; board 9/9 |
Mock adapter + conformance suite (PV1) | unit-tested, integrated | `crates/provider/src/mock.rs` + `suite.rs`; same suite runs on every adapter |
Local + remote adapters (PV2) | unit-tested, integrated | `local.rs`, `remote.rs`; std TCP + serde_json; live sidecars opt-in |
Announcer/taunts with fallback (PV3) | unit-tested, integrated | `announce.rs`; arena advances with providers down |
Vendor adapters (AX1) | unit-tested, integrated, ticked | `openai.rs`, `anthropic.rs`, `gemini.rs`, `ollama.rs`; 4 live tests ignored by design |
Streaming with backpressure (PV4) | unit-tested, integrated, ticked | `crates/provider/src/stream.rs` (64-chunk / 64 KiB caps, tick never blocks); cancel within 100 ms |
Conformance matrix (PV5) | unit-tested, integrated, ticked | `crates/provider/tests/matrix.rs`, `fixtures/provider-matrix/`; mock/local/remote/external/offline columns |
MCP stdio door as a library (MCP1) | unit-tested, integrated, ticked | `crates/endpoint/src/mcp.rs`; version refusal + drift test |
Play + capture tools (MCP2) | unit-tested, integrated, ticked | `play.rs`, `play/play_session.rs`; software rasterizer never answers `unsupported_capture` |
Brief/spatial/place tools (MCP3) | unit-tested, integrated, ticked | `scene_tools.rs`; placements go through commands |
`e2040-mcp` launcher binary (MCP5) | unit-tested, integrated, ticked | `src/bin/e2040-mcp.rs` + `startup.rs`; scripted external list/preview/apply/undo |
Engine agent door install (MCP6) | unit-tested, integrated, ticked | `tools/mcp-install.mjs`, `opencode.jsonc`; subagent lists tools live |
Self-describing door (MCP7) | unit-tested, integrated, ticked | `discover.rs` (manifest, `scene.watch`, one `{code, message, data}` shape, batch preview) |
Scene tool guards (MCP8) | unit-tested, integrated, ticked | `scene_tools.rs` + `sandbox.rs` (deny wins, tier gate); single flight; `name [id]` briefs |
Declarative tools (MCP9) | unit-tested, integrated, ticked | `tool_macro.rs` (`e2_tool!` macro today, true attribute needs a proc-macro crate) |
EXPLAIN for AI edits (MCP10) | unit-tested, integrated, ticked | `explain.rs` (`ExplainCaps` + `check_batch` + refusal with invariant + recovery) |
Assistant loop skeleton→apply (EA1a–EA1d) | unit-tested, integrated, ticked | `assistant.rs` (transcript, token accounting, CAP1-bound apply/undo) |
Multi-backend loop (EA1e) | unit-tested, integrated, ticked | identical outcome vectors per backend (end state, revision/digest, fork metrics, undo/refusal/privacy effects), never identical call order; latency/cost receipts |
Tamper-evident door log (EA2) | unit-tested, integrated, ticked | `crates/endpoint/src/audit/`; 100/100 tamper; wave-50 door_dispatch seam |
Digest-chained receipts over ED8 (EA3) | unit-tested, integrated, ticked | hunks beside `crates/editor/src/durable.rs`; 100/100 detected; cap 256 with anchor |
Eval harness (EV1) | unit-tested, integrated, ticked | `crates/playtest/src/eval.rs` (2006 lines); 75 rows (20 authoring + 5 play x 3 backends) + honest external row |
Brief-to-game eval (EV2) | unit-tested, integrated, ticked | `crates/playtest/src/brief_eval.rs`, `fixtures/briefs/` |
Recorded A6/A7/A9/A10 runs (EV3/EV4/EV5/EV6) | unit-tested, integrated, ticked | `target/evidence/a6/`, `a7/`, `a9/`, `a10/`; EV3 75/75 green, parity 23/25 (a08/a12 remote privacy refusals by design) |
Evidence hardening + seal (EV11/EV14) | unit-tested, integrated, ticked | digests+seal at write time; HMAC verify; forgery FAILS, regens PASS |
Causal replay (CR1) | unit-tested, integrated, ticked | seeded bug attributed to exact tick |
Frame checks (QA2) | unit-tested, integrated, ticked | Black/MissingMesh/Clipping seeded faults caught on llvmpipe |
Findings panel (SP2) | unit-tested, integrated, ticked | board ui proofs |
Log tools (OB3) | unit-tested, integrated, ticked | `log_tools.rs` (digest + cursor pages, stable under permutation) |
Engine Insight index (API1) | unit-tested, integrated, ticked | `insight/` + `fixtures/insight-gold.json`; `api.search`, `api.describe`, `code.callers`, `asset.refs` |
Red-team tape (RV1) | unit-tested, integrated, ticked | `crates/endpoint/tests/redteam.rs`, `fixtures/redteam/` |
Stacked score-gate (RV3) | unit-tested, integrated, ticked | 12/22 baseline to 22/22 stacked (+83%), 0/20 FP; door wired at collect |
Injection defense (RV2) | unit-tested, integrated, ticked | `injection/` 5-category screen wired into `scene.place` |
Agent play session (VP1) | unit-tested, integrated, ticked | `play_session.rs` (step cursor, tape, PNG receipt) |
Tool-list budget | enforced by gate, total + per-tool table in code | `TOOL_LIST_BUDGET_CHARS = 38_944` in `crates/endpoint/src/mcp.rs:126` (raised 36864 → 38944 on 2026-09-30: live list 35872, headroom over the 2048 floor); per-tool ledger + bloat/headroom gates; board AX15 ticked |
Session-isolation chaos (AX14) | unit-tested, integrated, ticked | `crates/endpoint/tests/session_chaos.rs` (741 lines; 12 proposers x 1 object, one winner, typed conflicts); wave-54 TICK (session_chaos 6/0, endpoint 300+ green) |
Trace export (AX16) | unit-tested, integrated, ticked | `crates/playtest/src/trace_export.rs` (322 lines; `e2040-ev1-trace v1`, 75 rows + external line, escape `\\ \\t \\r \\n`, unknown versions refuse); wave-54 TICK (`dcaaa7a`, playtest 171/0, round-trip x2) |
Tool-budget ledger (AX15) | unit-tested, integrated, ticked | per-tool ledger structs + gates in code beside `mcp.rs` (`ToolCostRow`/`ToolBudgetLedger`, `check_tool_budget`, `tool_budget_gate_with_bloat`; no separate `ApprovalLedger` per-tool claim — the ledger is the per-tool table); ticked on the board with receipt; needs MCP7, API1 ticked |
Efficiency-penalty refit (BET-AX13) | HELD-BET moonshot, open | needs EV1, AX5, AX11 on its board text — AX5 never boarded (superseded by AX16 below); waits AX11 open + two-green-exports rule |
GEPA-style evolution (BET-AX14) | HELD moonshot, never boarded | LADDER P2 CARD wave-54 only (2 green AX16 exports to advisory diff, +pass 0 success loss to graduate); AX17 ticked so the agreement-probe gate is met, still waits the two green exports |
Tool-surface benchmark (AX10) | open | SPIKE wave-53 REJECT: 20 A6 tasks x mock/local/remote S20 vs S10/S30 measured 75/75 all surfaces (0.0pp) + synth flat 15/15/5, tokens 3k/6k/9k; kill rule <=5pp fires — no ROW on size alone; retrieval leg proposed below |
Efficiency-utility column (AX11) | open | RideWay idea-only; 75-row report + utility columns reproduce with one command; needs EV1, MCP5 |
Branch-merge receipts (AX12) | open | Chronos idea-only; 20-branch speedup + atomicity + x2 equality; needs FK1, EV1; MCP4 revives only on green |
Frontier verdicts (P2/P5) | CONFIRM, no rewrite, owned elsewhere | P2: AX11/API6/MCP17/AX12 texts match CARD intent, moonshot BET-AX13, tool-surface AX10; P5: EV15 ladder, SP19–SP21 takes, EV16/CR2/QA3/PQ9/PQ10 takes, BET-EV8 HELD; none duplicated here |
Wild cards (ON30/PF20/SP41) | spike-first, no row here | ON30 boarded open under docs/archive/2026-10-08/plan/online.md (needs ON1, ON2, MP1); PF20 CARD-only (>=10% wall at identical hash or killed); SP41 CARD-only (re-solve bars or killed); all ideas-only until spikes post numbers |
Endpoint hygiene + parked (MCP18–MCP24, EA4, API7/API8, WS1, LG1, MCP4) | MCP18 ticked, rest open/parked | API-review wave-49 rows, endpoint lane, never duplicated here; WS1 needs wasmtime license, LG1 an operator brief, MCP4 fork-ID plumbing |
Live vendor calls, external-client E2E | missing (operator-side) | opt-in env + operator keys/hosts; never faked |

### Eyes (docs/archive/2026-10-10/plans/eyes.md section 1)

| Capability | Status |
| --- | --- |
| Brain brief over MCP | unit-tested, integrated |
| Brief parity x3 providers | unit-tested, integrated |
| Stress sweep (dupes, hostile, overfull) | unit-tested, integrated |
| Unwired capture refusal | unit-tested, integrated |
| Wired capture receipt check | unit-tested, integrated |
| Live duel step with receipt | unit-tested, integrated |
| Step capture hook | present, GPU side unwired |
| Showcase stitch + set gate | unit-tested, integrated |
| Orbit showcase set (CPU twin) | evidence only, presenter-twin |
| Forge helmet-POV scene note | pilot note, not run here |
| Point-to-entity path | absent (grade F) |
| Model reads pixels | absent (grade F) |
| Model-written reports | absent (grade C-) |
| Eye topics in research | catalog only, not rows |
| Spatial gold eval harness | unit-tested, integrated |
| Provider conformance matrix | unit-tested, integrated |
| Replay tapes on disk | evidence only |
| Editor pick seed | evidence only |
| Frame checks (black, missing, clipping) | unit-tested, integrated |
| Capture manifest bounds | unit-tested, integrated |
| Brief token estimator | unit-tested, integrated |
| Cross-tool replay check | unit-tested, integrated |

The orbit set proves the stitch path works on CPU twin frames. It does
not prove the game capture path. The game still has no wired pixels.
The reader and the report writer do not exist.

## 2. Finish line

- **Tier A (alpha, done except operator-side proofs):** same proposal
  through mock/local/remote with identical outcome vectors (end state,
  revision/digest, fork metrics, undo/refusal/privacy effects — never call
  order); scripted external client over stdio; fork metrics reject
  balance-breakers; every apply carries approval + provenance; eval report
  reproduces with one command; the door holds headroom under the
  38944 budget (36864 → 38944 on 2026-09-30); 12-agent chaos elects one
  winner with zero unintended mutations; 75-row trace re-imports x2.
  Fixture/offline proof only — never certifies live wires or a real client
  (Tier B, missing). Measurable: EV1 75 rows green; endpoint suites green;
  matrix prints unknown-cost-never-zero; redteam green; ledger gate green
  (≥2048 headroom or recorded raise; +500 bloat refuses); `session_chaos`
  6/0; `trace_export` round-trip x2 + unknown-version refusal.
- **Tier B (v1):** AX15 ledger (ticked) stops silent drift; AX11 names a
  failure EV1 misses or closes killed; EV18 decides AX10 (graduate/kill);
  EA5 keeps the door audit bounded; MCP25 files scope policy without widening
  the surface; live vendor receipts fill the EV1/PV5 live columns on operator
  keys; a real client runs list→preview→apply→undo on a scratch project.
  Proof cap: the live bind waits for operator keys; `e2040-mcp.exe` exists, so it stays possible. Measurable: live columns
  stamped live vs fixture; external E2E receipt; per-tool costs posted;
  +500 bloat fails; headroom under ~2k forces a raise-with-reason or refusal.
- **Tier C:** AX17 agreement probe feeds (never starts) prompt
  evolution; BET-AX13/BET-AX14 stay HELD-BET advisory-only behind the
  two-green-exports rule; API4 collapse and API5 delta resolve killed or
  landed; prompt evolution and intent-router stay LATER (traces first,
  lexical `api.search` first). Measurable: two green exports, agreement
  numbers posted, advisory diff only; 0 success loss or held; collapse ≥ 50%
  fewer tokens with 0 success loss or killed; delta 100/100 at p95 ≤ 200 ms
  or killed.

### Eyes (docs/archive/2026-10-10/plans/eyes.md section 2)

Tier A alpha (eyes joined): EV20 lands wired pixels with a 15-frame
sha-pinned set that fails closed on 0 PNG. EV21 lands the pick
manifest with a 100-task gold pointing eval at 100% on the engine
side. EV22 lands the model-reader adapter with 3-path parity and
offline replay. EV23 lands model-written fight reports with 3 tapes
replaying to identical reports. Bar: every capture carries a manifest
plus PNG magic; every region maps to an entity id or a 3D point; the
tick never sees model bytes.

Tier B v1 (brain remainder drains): the open SP rows in section 7
land oldest first, each with its gold task green. Bar: SP16 stacks
with 0 findings; SP19 briefs 2,000 objects under budget; SP20 answers
50 seeded questions exactly.

Tier C (reference bar): the full eye loop runs on the reference GPU
(RTX 3050 laptop, O5) with adapter plus driver plus power-mode lines
on every number, beside the forge PIE capture per the RomeScene note.

What moves which bar: EV20 moves the pixels-before-juice bar (real
frames replace twin placeholders). EV21 moves the grounding bar (a
click names an entity, never a guess). EV22 moves the AI-agnostic bar
(any model, same answers, replay with no model). EV23 moves the
reporting bar (a fight writes its own receipt). The brain sweep moves
the spatial bars (stacking, hierarchy, questions) that the eyes read.

## 3. Target design

Data flow: caller runs `api.search` over the Insight index before asking
→ builds `AiRequest` (prompt context, capabilities, privacy mode, budget,
deadline) → session policy gates destinations and data classes → adapter
negotiates capabilities → call with cancel/job handles, streamed through
the PV4 bounded inbox when long → `AiResult` (proposal + usage with
unknown-stays-unknown) or typed `AiError` → validator checks → MCP10
EXPLAIN prices the batch and refuses over-budget work pre-apply → fork
play-tests balance → CAP1 approval binds session/revision/action/expiry →
command service applies as one patch → provenance receipt minted → OB3
log tools record digest + cursor pages → eval row recorded → AX16 export
renders the 75 rows + external line as versioned tab-separated text a
future tuner could learn from; no tuning happens here.

Ownership: `provider` owns contracts, adapters, streaming, fallbacks and
notices; `endpoint` owns the MCP surface, the assistant loop, Insight,
discovery, guards, EXPLAIN, declarative tools, log tools, the door audit
and the per-tool ledger; `playtest` owns eval, causal replay, frame checks,
tournaments, the trace export and the agreement probe; `editor` owns panels,
approvals UX and templates; `game` owns the tuned-fight consumer. No
provider SDK types cross into simulation, scene or command types.

Threads: adapters and streams run on worker/test threads; the fixed tick
never waits — accepted actions arrive as recorded inputs. Forks run N ticks
on worker threads with identical metrics twice. The MCP server serializes
proposers on the document write queue with bounded readers; AX14 proves the
12-agent race (one winner, typed conflicts, fresh revision to retry). Cancel
during approval/play/stream/import, scene close mid-call, restart after
apply, approval reuse on changed arguments — all no-mutation proven.

Determinism: same proposal + same revision → same transcript, preview,
fork id and metrics, apply revision and digest; same files → same Insight
index and ranking; same SSE bytes → same stream text; same report → same
trace bytes; export-import-export is byte-identical. Backend labels differ
by design — parity pins task rows, previews, forks and effects, never
source names. Unknown costs stay unknown, never zero.

Budgets: per-request ms + spend caps; per-session edit budget (CAP1);
tool-list under 38944 chars with the AX15 ledger (measured size + reason
per raise, 2048 headroom floor, +500 bloat refuses); one loop under
`ASSISTANT_BUDGET_DEFAULT`; streams capped at 64 chunks / 64 KiB; watch
queues bounded; eval rows carry latency and cost; overflow is Unavailable,
never silent. Trace exports cap at 1024 rows (harness writes 75);
`MAX_DETAIL_CHARS = 256`; door audit appends within 500_000 ns per record.

API sketch: `AiRequest/AiResult/AiEvent/ProviderCapabilities/
AiSessionPolicy/Proposal/AiError`, `BackendKind::{Mock,Local,Remote}`,
`AssistantLoop::{observe,propose,check,fork_test,apply}`,
`ForkProof::replay()`, `backend_report`, `EvalReport::to_text()`,
`export_trace/import_trace`, `log.digest/log.query`,
`api.search/api.describe/code.callers/asset.refs`, `rpc.discover`,
`scene.watch`, `ExplainCaps::check_batch`, `e2_tool!`, `test.run`.

### Eyes (docs/archive/2026-10-10/plans/eyes.md section 3)

Data flow, one way, no loops. Game scene renders pixels. The capture
path hands PNG bytes plus a manifest to the endpoint. The pick path
maps regions to entity ids. The reader path feeds recorded PNG plus
brief text to a model behind the door. The report path takes tape plus
captures plus brief and returns schema-pinned JSON.

Ownership: `crates/game` owns the capture wiring (one capture path,
never a second). `crates/endpoint` owns the pick verb and the reader
adapter shape (mock, local, remote labels only). `crates/playtest`
owns the gold evals and the report writer. `crates/spatial` owns the
brief and the graph and changes nothing here. The forge repo owns its
PIE plus capture pilot; this plan names its note, never its code.

Threads: captures ride the service path, never the fixed tick. Live
inference submits validated recorded actions only. No model call
blocks a tick. No weights live in this repo.

Determinism: same seed plus same tape gives the same digest and the
same PNG bytes on the recorded set. Brief text stays byte-identical
across input orders. Reports replay identically with no model in the
loop. Floats never reach the hash; integer cm only.

Budgets: captures cap at 4 MB PNG and 2048 px per side (512 software).
Brief budgets stay byte-counted with the digest line mandatory. Pick
answers stay under 1 ms per query. Reader latency and cost ride the
receipt, never the tick.

API sketch: `capture_with` stays the single wired door. The pick verb
answers `region -> entity id | 3D point` beside `scene_tools.rs`. The
reader adapter answers `recorded PNG + brief -> scripted read` with a
`provider` label of mock, local or remote. The report writer answers
`tape + captures + brief -> JSON` with a digest line.

Per-row data shapes (frozen once the row lands):

- E1 in: live scene plus seed plus tape. E1 out: PNG bytes plus a
  manifest (adapter, driver, width, height, validation errors) plus a
  showcase manifest entry (file, sha256, bytes). The stitch tool
  checks every byte; a tampered frame fails loudly.
- E2 in: a capture plus a camera (eye, facing, projection). E2 out:
  one entity id per region plus the camera project and unproject
  pair. Regions tile the frame with no gaps and no overlaps. A point
  outside every region answers a 3D point, never a wrong id.
- E3 in: recorded PNG plus brief text plus a provider label. E3 out:
  a scripted read (entity ids, relations, counts) that never depends
  on the label. The mock read is the oracle; local and remote match
  it byte for byte or the path stays open.
- E4 in: a seed-fixed tape plus its captures plus its brief. E4 out:
  one schema-pinned JSON report (schema version, digest, rounds,
  hits, knockdowns, verdict). The digest covers the whole report;
  reruns compare bytes, never prose.

**Steals 2026-10-12** (board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`). Result modality: pixels go out as an MCP image block (base64 PNG, max_size 768 by default) with their sha256, numbers as structuredContent, never a byte array (AX27). Long calls (play.run, judge sets, cooks) return an MCP task to clients that declare it and start-poll to the rest; a task result equals the blocking result byte for byte (AX28). Visual judging has three tiers: bit-exact on the software adapter, FLIP pooled p95/p99 with a per-adapter threshold table, and a VLM only on crops FLIP names, never alone (PQ22). Flicker is judged on consecutive frames after motion compensation (PQ21).
- Fix: this plan says the tool-list budget is 38_944 at `mcp.rs:126`; the code has 43_514 at `mcp.rs:149`.

## 4. Hard problems

- Backend independence → contract-first adapters with a shared failure
  suite (timeout/malformed/cancel/stale/duplicate/quota/overflow/late/
  privacy on every adapter), so no backend is the abstraction — the suite
  cannot drift per backend by construction. Reference: Rig capability
  contracts (MIT); Orak 2506.03610 eval shape (paper). Donor: Rig, README.
- Proposal safety → check + EXPLAIN + fork + approve before apply: conflict
  sets, priced batches, fork metrics with replayable tapes, CAP1 approval
  bound to exact bytes — staged validation catches what no single gate sees.
  Reference: OpenGameAgent approvals (MIT); bq-guard/bqcost dry-run gating
  (MIT). Donor: cooper-tretter/bq-guard, pattern only.
- Eval honesty under retrieval pressure → distractors + efficiency shape +
  rule-based ground truth: 5–10 distractor tools per task, TEFS reported
  never gated, before→after revision/digest/ForkMetrics never call order
  (exact-match would fail valid paths) — EV1's clean room misses saturation
  and strategic defeatism. Reference: MCP-Atlas 2602.00933, MCPAgentBench
  2512.24565, MCP-Bench 2508.20453, ComplexMCP 2605.10787 (papers). Home:
  EV18 feeds AX10/AX11.
- Context budgets → ledger + collapse: the list grew 9→20 tools and the
  budget 16384→32768 across five receipted raises; per-tool costs make growth
  a decision, not drift; packs-on-demand (API4) is the kill-ruled experiment.
  Reference: Unity-MCP skill files (idea only, UNVERIFIED). Home: AX15.
- Session isolation → race-first proof: one write queue, bounded readers,
  stale writers retry on the fresh revision, never a silent merge; the
  12-agent chaos asserts zero unintended mutations — stdio tests cannot
  certify the remote shim's concurrency. Home: AX14 landed; MCP25 files the
  policy without widening the table.
- Trace usefulness without tuning risk → versioned export + agreement probe
  + held moonshots: 75-row export re-imports x2, refuses unknown versions,
  agreement posts held-out numbers on two green exports, moonshots stay
  advisory-only — traces must exist and agree before any tuner learns.
  Reference: GEPA 2606.17838; RideWay 2609.17985v1 (papers). Home: AX16
  landed; AX17 probes; BET-AX13/BET-AX14 held.

### Axiom Gameworld (docs/archive/2026-10-10/plans/axiom.md, one paragraph)

The 10 arcade Gameworld games (rebuilt as deterministic ticks) are off the line. The one AI training door (headless, seeded step API, observation = objects + pixels, reward, replay tape) is the gym pack at `crates/endpoint/src/packs/gym.rs`.

## 6. Slim

Delete: nothing pending — one loop, one eval harness, one door binary, one
tool router; AX14 race harness test-only, AX16 export a pure function.
Merge: vendor adapters share TCP + envelope + suite (AX1 done); EA1a–e live
in one `assistant.rs` state machine; Insight in one `insight/` module;
trace export in one `trace_export.rs`, never a second harness. Freeze: C2
contract shapes; the transcript golden; the eval task list (20 authoring +
5 play — new tasks join only with backend-parity proof); the `e2_tool!`
spelling until the proc-macro crate lands; the AX16 trace format v1; the
tool router (new tools only through the AX15 ledger). We will not build:
a second provider abstraction; an in-tick model call (DROP, law); a skill
marketplace, relay WAN, full anticheat (CUT); voice/TTS/music runtime
(DROP, bake only); a 300-tool dump (distractors are fixtures, not product);
a tuner inside this plan (BETs stay advisory-only); a second ON/PF/SP row
for the wild cards (spike-first, no row until numbers).

## 7. Board tasks

Rows from the steals (2026-10-12, board shard `../SPRINT-21-look-feel.md`, scout files `../research/steals-2026-10-12/`): AX27 capture as an image block, AX28 play.run as a task, PQ21 visible flicker, PQ22 golden bless flow. Row changes: AX28 supersedes the progress hunk of AX26 (AX26 keeps its transcript test); LF15 adds pooled p95/p99 and an explicit pixels-per-degree.

Rows from the books (2026-10-12, board shard `../SPRINT-20-game.md`; `BOOK-GAPS-2-2026-10-12.md` 2.11): AX25 fetched text and tool output are fenced as data in `assistant.rs`, AX26 `play.run` reports `notifications/progress` plus a protocol transcript test.

Ticked rows are dropped; rows never boarded are not listed. Table below: open AX, EA, MCP and EV rows.

| ID | Title | Tier | needs |
|---|---|---|---|
| AX11 | EV1 efficiency-utility report column | B | EV1, MCP5 |
| AX12 | recorded branch-merge fork receipts | B | FK1, EV1 |
| EA4 | bound the door audit | A | none |
| EV15 | the research ladder | A | EV3 |
| EV16 | checksummed sparse rebuilds for brief-to-game eval | B | EV2, QA1 |
| EV21 | pick manifest | A | EV20 |
| EV22 | model-reader adapter behind the MCP door | A | EV20, EV21 |
| EV23 | model-written fight report | A | EV20, EV21, EV22 |
| MCP12 | WASM sandbox for tool execution | B | MCP9, MCP11 |
| MCP16 | capability manifests + static import scan before tool run | B | MCP9, MCP11 |
| MCP17 | turn-vs-tool capability refusal tags | A | MCP9, MCP10, MCP11 |
| MCP22 | un-mirror sibling types | B | none |
| MCP23 | strong types for units + actions | B | MCP18 |
| MCP25 | scope-policy file deepening the capability table | A | MCP9, MCP10 |

### AI-native gaps (2026-10-10 review)

Ranked candidate rows. Each is one line with its owner crate. Reasoning and evidence: `target/reports/plans-review-2026-10-10.md` section D. Boarded 2026-10-10 in `docs/SPRINT-18-gaps.md` (FOCUS Now): 1 = LJ1, 2 = AL1, 4 = DP1 (net and perf packs), 5 = RB1, 7 = AG1. 3 landed (`docs/CRATE-MAP.md`, error catalog). 6 and 8 stay candidates.

- [ ] LJ1 `look.judge` door verb (board: `docs/SPRINT-18-gaps.md`).
- [ ] AL1 live model drives a gladiator inside one recorded bout (board: `docs/SPRINT-18-gaps.md`).
- [ ] DP1 door packs for net and perf (board: `docs/SPRINT-18-gaps.md`).
- [ ] RB1 `replay.verify` and `replay.bisect` door verbs (board: `docs/SPRINT-18-gaps.md`).
- [ ] AG1 agent scorecard (board: `docs/SPRINT-18-gaps.md`).

1. Look judge loop: a `look.judge` door verb captures the pinned camera set on a release build, runs frame_check and a reference compare, and returns rubric L1-L9 scores with entity-grounded regions, identical x2 on one build; Q1 and RL rows accept on that number. Owner: `crates/playtest` (look.rs, frame_check.rs) + `crates/endpoint`.
2. Live inference in the tick, proven end to end: one arena bout where a live model drives a gladiator through validated actions stamped for tick N+k, recorded; replay with the provider off gives the same hash; a late reply falls back to the scripted brain with a logged reason. Owner: `crates/ai` (mind.rs), `provider`, `arena`.
3. Agent-readable crate map and error catalog.
   - DONE 2026-10-10: crate map, `docs/CRATE-MAP.md`, made by `tools/crate-map.mjs`. Owner: `tools/`.
   - DONE 2026-10-10: error catalog, the `rust-compile-errors` skill (`skills/rust-compile-errors`). Owner: `tools/` (skills).
   - Open: `api.describe` returns the same text as the crate map; a gate fails a pub module with no header; error codes come from the typed errors as `{code, message, data}`. Owner: `crates/endpoint/src/insight` + `tools/`.
4. Door verbs for the Core systems: `net` (open a loopback session, inject loss and latency, read the first divergent tick), `perf` (per-pass ms p50/p99), `physics` (ragdoll and cut probe) and `render` (pass toggle, capture) packs, typed, read-only or undoable, refused by default. Owner: `crates/endpoint/src/packs` + `net`, `perf`.
5. Replay as the test oracle: `replay.verify` and `replay.bisect` door verbs print the first divergent tick; wire the idle `desync-diff.mjs` and `mp-evidence.mjs` into the runner seat. Owner: `crates/playtest` (causal.rs, seeded_bug.rs), `crates/proof`, `tools/`.
6. One command path, proven: one Op batch applied through the editor, an MCP pack verb and a Rhai rule gives the same scene hash, the same receipt shape and one undo; move break, anim and material verbs onto the journal if they bypass it. Owner: `crates/editor` (commands.rs), `endpoint`, `rules`.
7. Agent scorecard: one command prints pass rate and tokens per task for EV1, spatial_eval, the mirrors and BreakBench, by model tier, every 5 waves; a drop is a red gate through bench-compare. Owner: `crates/playtest` eval + `tools/bench-compare.mjs`.
8. Weak-model packet kit: `packet-lint` refuses a row with no paths, no F2P/P2P command or no production caller; `board-slice` returns the plan excerpt of the row's own plan. Owner: `tools/`.

## 8. Quality gates

- Same 20 authoring + 5 play tasks through mock/local/remote with no code
  change; model-neutral outcome parity per backend (end state,
  revision/digest, fork metrics, undo/refusal/privacy effects — never
  call order); eval report reproduces with one command; EV18 A/B runs beside it, never inside it.
- Tool-list budget green with the ledger gate beside `mcp.rs`
  (`tool_budget_gate_with_bloat`; seeded +500 bloat refuses, sub-2k
  headroom refuses); board AX15 ticked — every raise still carries measured size + reason; a
  seeded description bloat fails the endpoint gate.
- Privacy gates green: local-only adapters never escalate (loopback-only
  construction asserted, zero socket attempts on Offline, zero remote
  calls on downgrade); secrets never appear in diagnostics (canary tests);
  unknown usage and cost stay unknown, never zero, with hard caps.
- Every apply carries approval + provenance with no secrets; forged/stale/
  reused approvals change nothing (CAP1 tests); AX14 chaos shows one
  winner and zero unintended mutations under a 12-agent race.
- Causal replay attributes the seeded bug; frame checks catch the seeded
  faults; fork metrics reject the bully and freeze-both fixtures; redteam
  refuses every fixture case with a logged reason and no secret in any log.
- Trace gates green forever: 75-row export re-imports byte-identically
  twice; unknown versions refuse loudly with the legal version named;
  golden unchanged; AX17 agreement numbers reproduce with one command.
- Moonshot gates green forever: BET-AX13/BET-AX14 advisory-only (no
  auto-apply, no tick touch, no weights in git); two-green-exports rule
  before any graduate claim; any success loss or format drift kills with
  numbers.
- Live adapter success, mock conformance, editor integration and frame
  checks keep separate receipts forever: a green mock never certifies a
  vendor wire, a passing unit test never certifies the external client,
  and a CPU reference never certifies a GPU capture. Reports print which
  columns are fixture-proven vs live-proven vs operator-proven.
- Timing quarantine inventory (all `#[ignore]`, all run under `--perf`
  release single-thread): vendor live opt-ins behind `E2040_AX1_*_URL`,
  animation match-db p99, assets reload latency, ECS archetype p99,
  AX14 p95 race timing. Promotion needs a load-proof of repeated green
  full-workspace runs, never a deleted attribute.
- Wild-card gates: PF20 needs >=10% wall at identical hash with deopt
  <5% or killed; SP41 needs +30% clean-first-pass or -50% re-solve p95
  at identical outcomes or killed; ON30 deterministic fault-sim variant
  needs 10/10 seeded desyncs with correct first tick and replay x2 — all
  as spikes first, never as rows here.

### Eyes (docs/archive/2026-10-10/plans/eyes.md section 8)

- `cargo test -p engine-spatial` green; brief suites stay green under
  every row; golden `d57b5c8ae3096614` held x2 unless a row names a
  migration receipt (none do here).
- Capture gate: every receipt carries manifest plus PNG magic; a 0-PNG
  set fails closed; sizes uniform or the set fails loudly.
- Pick gate: duplicate names resolve by id only; a shared name never
  mis-places; project/unproject round-trips within 1 integer cm.
- Reader gate: 3-path parity or the remote path stays open; replay
  with no model is byte-identical; no weights in the tree.
- Report gate: schema-pinned JSON with a digest line; 3/3 tapes
  identical x2; digest reads back the bytes.
- Reference-GPU numbers carry adapter plus driver plus power-mode
  lines, or they do not count toward Tier C.
- Proof commands per row (judge runs F2P then P2P, both exit 0):
  EV20 `node tools/showcase.mjs --set <setDir>` plus the game capture
  test; EV21 `cargo test -p engine-endpoint --lib scene_tools` plus
  the 100-task pointing eval; EV22 the provider matrix plus offline
  replay bytes x2; EV23 the 3-tape report rerun bytes x2 plus golden
  `d57b5c8ae3096614` x2.

