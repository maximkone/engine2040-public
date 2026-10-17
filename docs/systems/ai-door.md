# SYS ai-door — providers, MCP, eval, playtest agents, skill packs

Lanes: provider (`crates/provider`, new), mcp (`crates/endpoint`), playtest
(`crates/playtest`, new), skills (`skills/`, new). Contracts:
`../EDITOR_AI_CONTROL.md`, `../AI-NATIVE.md`. Board: C2, PV1–PV3, MCP1–MCP4,
QA1, QA2, CR1, VF1, LG1, EV1, RV1, WS1, OB3, CAP1.

Status: landed in part. The provider crate ships mock, local, and remote adapters (`crates/provider/src/`), and the MCP door serves tools from `crates/endpoint/src/`. Open rows: MCP16 manifests, PV6 cancel timing, PV7 tool-loop harness.
EA1 is the built-in editor consumer; internal calls use engine services directly,
not an external coding assistant or an MCP roundtrip. Source review:
`../EVIDENCE.md` (2026-09-23 reference-only rows).

## What is hard

Models change every month for fourteen years, differ in capabilities, time
out, return malformed output and cost money. External agents need enough
context to act well without flooding their token budgets. Thousands of agent
calls must never corrupt the world or bypass checks.

## Design

- **Provider service:** engine-owned `AiRequest`, `AiResult`, `AiEvent`,
  `Capabilities`, `AiError`; adapters for mock, local (llama.cpp server) and
  remote; capability negotiation; deadlines, cancellation, bounded queues,
  retries with backoff, cost and token budgets; offline fallback always.
  `AiSessionPolicy` controls privacy, destinations, data classes and tool scope.
  Unknown usage/cost remains unknown, never zero; hard caps reserve or refuse.
- **Conformance suite:** every adapter must pass the same tests: timeouts,
  malformed output, cancellation, stale revisions, duplicate delivery, quota
  errors, unsupported capabilities, backpressure, stale approvals and privacy
  downgrade attempts. "OpenAI-compatible" is one adapter, not the abstraction.
  Text, native tools, strict output schemas and vision are separate capabilities;
  embedding/speech capability flags do not promise those adapters are built.
- **MCP door:** official `rmcp`, pinned released version and negotiated protocol
  (2026-07-28 reference). Upstream is transitioning MIT to Apache-2.0; NM1 must
  verify the selected crate's license/features, not guess from the repo badge;
  tools generated from the command schema; progressive discovery (small
  default set, search → describe → call); a tool-list token budget enforced
  as a gate; long-running tasks for play, forks and bakes; errors return legal
  values, the conflict set and a recovery hint. Roots, sampling and protocol
  logging are deprecated-but-functional under SEP-2577; none is a dependency.
  Engine log tools remain available. No arbitrary execute/shell/file-write tool.
- **Loop for agents:** brief → propose → check → fork → apply → verify, with
  capability tokens per session (CAP1) and provenance on every batch. Approval
  binds the session, revision, action digest and expiry; a changed proposal
  requires approval again. Tool text cannot grant scope. One write queue per
  document, bounded parallel readers/jobs, stale conflicts returned explicitly.
- **Built-in assistant (EA1):** bounded observe/propose/validate/approve/apply/
  verify/report state machine with cancel/fail/needs-user states. Show the diff,
  checks, privacy mode and action timeline, not hidden reasoning. On restart,
  reconcile saved job/receipt IDs before retrying; never duplicate a mutation.
- **Context:** stable references with scene revision and content digest; selection,
  dependency summary, diagnostics and optional capture. Enforce byte/token limits
  and say what was omitted. Untrusted asset/log text cannot override permissions.
- **Jobs:** long imports, captures and playtests return handles with progress,
  deadline, bounded result/log size, cancellation, expiry and final receipt.
  Services marshal validated commands to the owning thread/boundary, never
  mutate a scene from an HTTP callback. Closed scenes invalidate late results.
- **Observability for agents:** OpenTelemetry GenAI span names for every model
  and tool call; `log.digest` and `log.query` tools (OB3).
- **Playtest agents (QA1–QA2, CR1, RV1):** play the headless arena through MCP,
  judge from metrics and captures, file bugs with tapes, attribute causes by
  causal replay, and a red-team agent hunts exploits (infinite stamina, wall
  clipping, parry spam) that become regression tapes.
- **Content loop (LG1):** text brief → relation placements → verifiers → at
  most 5 attempts → saved through commands.
- **Eval harness (EV1):** fixed authoring and play tasks across mock, local,
  remote and an external MCP client; Orak-style reports (completion,
  rejections, latency, cost).
- **Skill packs (WS1):** behaviors, verifiers and tool packs as WASM
  components (WASI 0.3) with declared capabilities; packs survive engine
  upgrades through WIT interfaces.

## Determinism and authority

Model output is always a proposal. Accepted actions are recorded and replayed
without providers. Nothing from a model enters the tick except recorded
actions and shipped integer nets.

## Auto-heal

Provider down → mark unavailable and use the scripted/offline path. Failover
to another adapter requires an explicit policy with equal privacy/capability
bounds; local-only never silently becomes remote. Malformed output → at most
one budgeted repair with validator errors, then reject. Repeated failed call →
stop with digest and conflict set. Tool flood → per-session backpressure. Do
not retry auth/permission failures until config changes. An uncertain write is
reconciled by receipt, not retried blindly. All recovery attempts are visible.

## Smart logging

Version-pinned GenAI span mapping (`invoke_agent`, `chat`, `execute_tool`) with model, tokens,
latency and finish reason; `ai.proposal.accepted|rejected` (reasons),
`mcp.call` (tool, session, µs), `ai.budget.exceeded`. Secrets and raw private
prompts/assets are excluded by default. Trace policy cannot affect the tick.

## Acceptance and chaos

Conformance suite green for every adapter; the same 20 tasks through local,
remote and external MCP with no code change; token budget test for the tool
list; causal replay attributes a seeded bug. Chaos: provider returns garbage,
hangs forever, answers after the scene closed, or 12 agents race on one
object.

Also: unsupported-capability refusal; local-only network-egress trap; cost
unknown vs zero; cancel during approval/play/import; scene close during a
provider call; restart after apply before acknowledgement; malicious imported
metadata; approval reuse against changed arguments. Assert zero unintended
mutations and bounded work, not just an error label. Live adapter success,
mock conformance, editor integration and frame-grounded checks get separate
receipts. EV1 requires real play fixtures (AR2/MCP2), not fabricated metrics.

## Board tasks

C2, PV1–PV3, MCP1–MCP4, EA1, QA1, QA2, CR1, VF1, LG1, EV1, RV1, WS1, OB3, CAP1.
