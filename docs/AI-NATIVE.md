# AI-NATIVE — what makes engine2040 a new kind of engine

Design thesis, grounded in 2025–2026 research and mapped to `SPRINT.md` board
tasks. Nothing here counts as implemented until its board task is ticked with
a receipt. Research IDs are arXiv unless noted; verdicts live in
`RESEARCH-VERDICT.md`.

Evidence note (2026-09-23 review): paper identifiers below are research pointers,
not verified implementation/performance evidence. Live GitHub references are
listed near the end and in `EVIDENCE.md`; no third-party source was copied. The
practical first delivery is `EDITOR_AI_CONTROL.md`'s bounded task panel and
`SMOOTH-BUILD.md`'s workflows, not all ten pillars implemented at once.

## The thesis

In 2026 external bridges such as Unity MCP and Godot MCP let agents operate an
editor. SolersEngine documents a native observe/change/run/verify loop;
OpenGameAgent documents game-authoritative agents and approvals. These are
useful references, not proof that competitors lack transactions or undo.

engine2040 is being built so supported AI adapters can **understand, change,
test and verify** a 3D
game world. The world is a typed, spatially understood document. Every change
is a checked, undoable transaction with a receipt. Every "what if" can be
forked and simulated deterministically before it lands. Replays must match
within the declared fixture/platform scope in `HASH-LAW.md`; the arena's
cross-machine proof is still open. Models propose; deterministic engine code
checks, simulates and decides. An assertion here does not pass a gate.

## Ten pillars

### 1. A world any AI can read

Stable IDs, a typed schema, a relation graph in layers (arena → zone →
object) and a bounded brief sized to a token budget; captures for models that
see images. LLMs reason about space much better from explicit relations and
scene graphs than from raw coordinates (relation encoding 2603.24721,
schema-guided scene graphs 2502.03450, 3DGraphLLM 2412.18450), and causal
context plus planning raises game-agent win rates (2607.22732).
Tasks: SP1, SP3, SP4, MCP3. Proof: a golden brief for the arena; minimally
contrastive scene pairs give different relation graphs.

### 2. A world any AI can change safely

Typed command batches or intent placement ("brazier against the north wall,
2 m from the gate"), turned into transforms by a deterministic solver that
returns the minimal conflicting set on failure. Schema, range, revision and
spatial checks run before anything applies; revisions are monotonic;
everything undoes. This follows the propose → solve → self-correct pattern
and SpatialGrammar's executable spatial language (2604.27555).
Tasks: N3 (done), SP1, SP5, MCP1, MCP3. Proof: `FINISH-RICH.md` A6 spatial proof.

### 3. A world that can be forked: counterfactual play-testing

`fork` snapshots the headless arena match, applies a proposal to the fork, runs
N ticks on a worker thread outside the real tick, and returns metrics (damage,
time to kill, stuck fighters, broken paths). FK1 must prove that identical
complete fixtures give identical metrics within the declared determinism tier;
that arena proof is not established yet. The real match stays untouched until
a validated proposal is approved. Research lets LLMs question a simulator with "what if" and "remove"
(AXIS 2505.17801), evaluate alternative intents in the loop (2601.20090) and
predict outcomes before acting (text world models 2606.09032); here the world
model is the exact engine, not a guess.
Tasks: FK1, EA1c (fork tools over MCP are parked). Proof: identical metrics
for the same fork twice; a balance-breaking proposal is rejected by its fork
before apply.

### 4. A world that explains its own failures: causal replay

For reproducible simulation bugs, the evidence is an input tape with checkpoints
and pinned build/rules/content. Causal replay removes or
perturbs one input or accepted action at a time, re-runs the tape and reports
the smallest change that makes the failure disappear, plus the first
divergent tick. This is Causal Agent Replay's do-operation (2606.08275) made
exact by determinism.
Tasks: QA1, CR1. Proof: for a seeded bug, the tool names the causing input.

### 5. Verifiers as compilers for game content

VF1/LG1 acceptance requires spatial checks, playability (spawns and gates
reachable), combat balance (AI-vs-AI fork win rates in bounds) and declared
draw/triangle/memory budgets before generated content lands.
Tool-using LLM content generation with environment feedback (Agentic PCG),
the PCG Benchmark (2503.21474) and dual-agent 3D map generation (2512.10501)
show generate → evaluate → repair loops work when the evaluator is real.
Tasks: VF1, EA1b. Proof: an AI-made change lands only when every verifier
passes, through the command service only (the brief-to-variant generator is
parked).

Status 2026-10-07: room scale landed. VF1 verifiers (reachability, win rate,
budgets) are ticked; a rejection reads as a compile error
(`crates/playtest/src/reachability.rs`). The brief chain AB1 → AB3 → AB4 with
AB9 offline replay is ticked: constraint-first compile (SP23), at most 3
repair rounds with solver hints (SP54), fork balance before apply (AB3). The
limit: `level.build` makes one 10 m room with at most 24 props
(`crates/endpoint/src/build_level.rs`). Next, proposed and not boarded: a
level graph above the room — rooms, locks and keys, critical path, optional
branches — checked by code, measured over many seeds, searched toward an
experience target and changed with keep-rules (AB17–AB22 in
`RESEARCH-VERDICT.md`, "World compiler paste").

Rule: never ask a model what an algorithm can prove. Reachability, budgets,
locks and keys, softlocks and overlaps are code. Models give intent and judge
taste; they never vote on a fact the engine can compute.

### 6. Any model, provably

Engine-owned request/result/capability types, provider adapters (mock,
local, remote) behind one conformance suite, deadlines, cancellation, bounded
queues, offline fallback, and recorded actions that replay without any model.
External agents use the MCP door (`rmcp`, spec 2026-07-28). MCP sampling is
deprecated in that spec (SEP-2577), so the engine never depends on a client's
model. Orak (2506.03610, ICLR 2026) benchmarks game agents through MCP, and
GameCraft-Bench (2606.17861) asks whether agents can build playable games in a
real engine; the eval harness reports the same way.
Tasks: C2, PV1–PV3, MCP1–MCP3, MCP5, AX1, EV1. Proof: `FINISH-RICH.md` A6
AI-agnostic proof.

### 7. AI players and QA that leave evidence

Gladiator AI is deterministic and replayable; optional live lines (announcer,
taunts) stay outside the tick with scripted fallback. AI playtest agents drive
the headless arena through MCP and file each bug with its tape, judging
progress from metrics and captures rather than compilation (TITAN, GBQA
2604.02648, coverage-aware playtesting 2512.12706; prompt evolution with GEPA
2606.17838 later).
Tasks: BR1, PV3, QA1, QA2. Proof: `FINISH-RICH.md` A7.

### 8. Neural brains inside the deterministic tick

The planned integer runtime must specify overflow, rounding, saturation,
operator order and state serialization before bit-exact replay can be claimed.
Integer weights alone do not make an implementation deterministic. NN1/BR2
must prove a learned policy survives rollback on the supported machines.
This is not a prerequisite for a useful assistant or good baseline locomotion.

- **Format `.e2nn`:** versioned and content-hashed; int8 weights, int32
  accumulators, per-layer requantization as an integer multiplier plus shift
  (integer-only inference, 1712.05877), lookup-table activations, fixed input
  scaling. Layers: dense, embedding and an integer GRU (iRNN 2109.09828);
  nothing more until a task needs it.
- **Runtime:** a std-only Rust interpreter with no floats and no SIMD-dependent
  paths; one forward pass per fighter per decision (for example 10 Hz). A
  32→64→64→8 network is about 6.7k multiply-adds per decision.
- **Pipeline:** train in any framework, export ONNX with int8 QDQ
  quantization; the converter checks an operator whitelist, folds scales into
  integer multipliers and writes `.e2nn`. A float reference run (tract) must
  agree with the integer run within tolerance before the artifact is
   accepted. Research on quantization (2607.23227, 2004.09602) does not establish
   this engine's accuracy or determinism; measure both on its own fixtures.
- **Authority:** the artifact digest joins the rules and asset versions of
  every fixture and replay, so new weights are a versioned rules change. Every
  net has a scripted fallback, and forks compare net against script before a
  net ships.

Tasks: NN1, NN2, BR2. Proof: a net-driven fight replays bit-exactly on
Windows and Linux; changing one weight changes the checkpoints.

### 9. Any model teaches; no model is needed at runtime

The strongest form of AI-agnostic: large models teach offline and tiny
integer students play online. A teacher — any provider through the adapters,
a scripted expert or a human's recorded play — drives fighters in headless
forks, and every decision is recorded as observation, action and outcome. A
student is trained on that data (Rust `burn` or any framework), exported
through the `.e2nn` converter, and must match or beat the scripted policy in
fork tournaments inside the balance verifiers before it ships. The game then
aims for useful offline behavior with no live provider dependency. Inference
still consumes CPU time, training costs resources, and quality must beat the
scripted baseline in a held-out evaluation. Swapping the teacher must not
change runtime APIs. Research: agent distillation into small models (2505.17612), step-wise
and on-policy distillation for small agents (2605.07725, 2606.15912),
distilled real-time game mechanics (UNBOUNDED).

Tasks: DT1, DT2, DT3. Proof: a student distilled from an LLM teacher wins at
least as often as the scripted policy over 100 fork matches and replays
bit-exactly.

### 10. Provenance for everything AI makes

Each command receipt records its source (human, script or model), provider
and model ID, a digest of the request and the verifier results; objects keep
that chain, so "which parts of this arena did an AI make, with what, and what
checked it" is one query. Exported captures and trailers may later carry C2PA
Content Credentials. This plan is not a legal-compliance opinion; disclosure,
attribution and redistribution obligations need a source-specific review.

Tasks: PR1; C2PA export LATER. Proof: every AI-applied batch in the eval
harness carries a complete provenance chain.

## Built to last until 2040

An engine outlives its first game only when its formats, interfaces and
evidence outlive its code.

1. **Open standards at every boundary:** glTF 2.0 in; OpenUSD (Core
   Specification 1.0 ratified; a pure-Rust `openusd` crate exists) for scene
   exchange later; WebGPU/WGSL through wgpu; MCP (now under the Linux
   Foundation) for agents; ONNX for models; C2PA for provenance later.
2. **Every format versioned, migrated and kept in a museum:** scenes, tapes,
   `.e2nn`, saves and receipts carry a version; the format museum keeps one
   fixture per released version and the gate proves each still loads,
   migrates and replays. A 2027 replay must still replay in 2040.
3. **Provider churn is normal:** models will change every month for fourteen
   years; capability negotiation, the conformance suite and distillation keep
   the engine independent of each of them.
4. **Evidence culture:** gates, receipts, reproducible benchmarks and replays
   let future users verify claims instead of trusting them.

Tasks: LT1. WASM plugins and OpenUSD are parked (operator 2026-09-23: the MCP
door and data mods cover skill packs).

## GitHub patterns checked in this review (2026-09-23)

Primary-source docs/code were read, not built or benchmarked. These are design
inputs mapped to existing consumers, not permission to vendor whole engines.
License observations and immutable revisions where available: `EVIDENCE.md`.

- **AI-Native-Game-Engine:** every host call goes through its own MCP server,
  with a record that proves what happened → one write path plus receipts.
- **OpenGameAgent:** structured context, tool use, planning, memory, reliable
  actions, portable skill packages, append-only traces → traces in the eval
  harness (EV1).
- **Unity MCP (268 tools):** tool sprawl costs tokens and accuracy →
  progressive discovery and a tool-list token budget (MCP2).
- **Orak:** an MCP-based agent benchmark → EV1 reports in the same shape, so
  outside researchers can benchmark agents on the arena.
- **nAIVE and llmgine:** deterministic headless runs and cross-runtime state
  hashes → the tier-3 rule.
- **wonnx and burn:** ONNX on WebGPU and Rust training on wgpu → student
  training (DT2) now, presentation-only neural effects on the render device
  later.

## What engine2040 deliberately does not do

- **Neural world models as the runtime.** Genie 3, Matrix-Game 3.0
  (2604.08995) and GameNGen-style models are impressive but not
  deterministic, not multiplayer-authoritative and far over the reference
  GPU budget. At most LATER, offline, for concept previs.
- Float models or provider calls inside the fixed tick (integer `.e2nn` nets
  only), models owning coordinates, or model output applied without checks and
  a receipt.
- MCP sampling (deprecated) or any vendor type in scenes and saves.
- Unbounded agent loops: every loop has a budget, a deadline and a maximum
  attempt count.

## What still has to be demonstrated

Individually, pieces exist: deterministic headless engines, MCP servers,
scene graphs for LLMs, agentic content generation. engine2040 aims to combine
3D, Rust, tier-3 deterministic LAN multiplayer, transactional edits with
receipts and undo, spatial understanding in the core, counterfactual forks,
causal replay, bit-exact integer neural policies distilled from any model,
provenance for AI-made content and provider conformance in one engine,
in one tested game, with versioned interfaces and maintained compatibility.
This is the work still ahead, not a uniqueness or competitor-parity claim.
