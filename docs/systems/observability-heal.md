# SYS observability-heal — smart logging, retroactive logs, auto-heal, chaos

Lanes: observe (`crates/observe`, new, std-only), proof. Every other system
applies this spec. Board: OB1–OB3, HL1, HL2, FZ1.

Status 2026-10-06: OB1 landed and ticked — `crates/observe` ships typed events, ring buffers, spans plus the `heal` MAPE-K registry and `supervisor` fault catalog; HL2 chaos suite ticked (`crates/proof/src/chaos.rs`); FZ1 fuzz harness ticked. "Auto-heal" means a bounded, tested recovery, not arbitrary model
rewrites or turning failed correctness gates green.

## What is hard

- Free-text logs alone are difficult to query reliably. Keep useful human
  messages alongside typed events, stable codes, cause IDs and bounded summaries.
- Logging everything costs frame time; logging too little loses the bug.
- Self-healing that hides problems is worse than crashing. Healing inside the
  authoritative simulation must stay deterministic or it causes desyncs.

## Design

**Events.** Every subsystem emits typed events: `code` (stable dotted name,
e.g. `physics.body.nonfinite`), `severity`, `subsystem`, `tick`, `revision`,
object/entity IDs, `cause` (parent event or span ID), small typed fields, and
`hints` (recovery actions an agent can take, like the MCP door's legal
values). Codes are registered in one table that generates docs and the MCP
catalog; unknown codes fail a test.

**Spans.** Long work (import, cook, bake, provider call, MCP tool, fork run)
opens a span. Model and tool spans use the OpenTelemetry GenAI names
(`invoke_agent`, `chat`, `execute_tool`; `gen_ai.request.model`,
`gen_ai.usage.input_tokens`...) where the selected collector supports them; an
OTLP exporter is an optional adapter, never a core dependency. Pin the external
semantic-convention version in the adapter; engine event codes remain stable.

**Storage.** Bounded ring buffers per subsystem with explicit overflow
counters; crash reports keep the last N events, the input tape, seed and build
hash, enough to reproduce.

Bound field lengths and sensitive data. Credentials, authorization tokens and
private prompts/assets are excluded by default; bug-bundle export previews
what leaves the machine. Overflow/drop counters are visible, not missing data
reported as an empty error list.

**Digest for AI.** `digest(budget)` returns the most useful facts within a
token budget: top event codes with counts, first and last occurrence, affected
IDs, active heals, and the suggested next action. The MCP door exposes it as
`log.digest` with cursor-paged `log.query` (OB3).

**Retroactive logging.** Replaying complete input/action tapes with matching
build/rules/content can regenerate covered deterministic simulation events.
It cannot reconstruct missing OS faults, GPU timing or unrecorded provider
responses. Retain bounded service/presentation diagnostics separately. OB2
compares traced checkpoints and event summaries and measures overhead A/B;
there is no claim of zero runtime cost or unrestricted reproduction.

**Auto-heal, MAPE-K.** A registry maps fault codes to heal actions
(Monitor → Analyze → Plan → Execute over Knowledge; Kephart and Chess 2003;
2026 agent variants 2605.06737, 2606.01416). Rules:

1. Never silent: every heal emits `heal.applied` with cause, action,
   before/after digests; a heal that fails emits `heal.failed` and escalates.
2. Bounded: each fault has a retry budget; after it, quarantine the object or
   task and fail loudly.
3. Deterministic in the simulation: sim heals run only at tick boundaries,
   from the same inputs on every machine, and are recorded in the tape.
   Presentation and service heals may use wall time.
4. Heal to a known-good state (last snapshot, last good asset, fallback
    policy), never to a guess.
5. Return a typed outcome: recovered, retained-last-good, needs-approval,
   quarantined or unsupported. Include cause, scope, attempt count, before/after
   versions and validation result. A fallback is not a pass for missing content.
6. Telemetry cannot decide authoritative repairs. Sim invariants and correction
   rules live in the owning simulation code; any external correction is
   authority-validated and recorded. Never restore one body from a presentation
   cache while leaving contacts, RNG, timers and events from another tick.

| Fault | Detect | Heal | Where |
|---|---|---|---|
| NaN/inf in a body or transform | finite checks before publication | reject input or quarantine failing tick; only a validated full authority snapshot plus resimulation may recover; emit failure/tape otherwise | physics, authoritative |
| Fighter stuck | requested movement blocked outside idle/block/hit-stop windows | bounded deterministic replan under game rules; persistent failure produces a bug tape, not a silent teleport | brain, authoritative |
| Client desync | checkpoint hash mismatch | resimulate from server snapshot; count corrections | net |
| GPU device lost / surface lost | wgpu error callbacks | bounded surface/device recovery; cancel invalid captures, re-upload; unsupported recovery fails visibly | render, presentation |
| Shader or pipeline build fails | compile error | keep last good pipeline, show diagnostic | render |
| Asset missing or corrupt | digest/format check | fallback asset, retry import job, keep last good | assets |
| Provider timeout or malformed output | deadline, schema check | scripted fallback, bounded retry, mark unavailable; no unapproved local-to-cloud failover | provider, service |
| Stale AI edit | revision gate | return fresh brief and conflict set to the agent | commands |
| Editor or game crash | process exit | recover document from journal; child play restarts | editor |
| Save or tape corrupt | versioned checks | preserve original, offer validated previous generation; unknown schema refuses; never forge missing tape actions | assets, proof |
| Frame over budget | frame timing | lower presentation quality within floor/hysteresis (PA1), record changes; frozen baseline captures never adapt | render, presentation |

Build-time auto-fix (scene fixers, import fixers, `/fix` for code,
`tools/doctor.mjs` for the project) follows the same rules and is described in
`../SMOOTH-BUILD.md`.

## Determinism and authority

Events, spans and digests are outside the hash scope and can never feed back
into simulation state. Sim heals are part of the simulation and are hashed.

## Acceptance and chaos

- Event and heal codes documented and unique; digest stays within budget.
- Retroactive trace from a tape equals the live summary for the same run.
- Chaos suite (HL2): seeded fault injection for every row above as its
  subsystem lands; each fault either heals with a `heal.applied` record or
  fails loudly with a receipt. No fault may pass silently.
- Game fuzzing (FZ1): seeded random and adversarial input tapes against the
  arena with invariants (finite state, fighters in bounds, stamina ≥ 0,
   checkpoint continuity). Invariants proposed by agents (LISA-style,
   2607.18711) are reviewed, then added. Every failure ships as a tape.

Receipt matrix: one row per **shipped** subsystem/fault, fixture version,
injection command, expected outcome, observed outcome and artifact. Unsupported
recovery must show loud failure; it is not a recovered case. Stop a fault loop
after its attempt/time budget, preserve the original failure, and ensure a
recovery does not recursively trigger unlimited recoveries. Same input/recovery
record must reproduce authoritative checkpoints with logging on and off.
