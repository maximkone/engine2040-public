# EDITOR_AI_CONTROL — one command contract, any model backend

Canonical for AI/editor boundaries. Status: target design except where a receipt
is named. `engine-editor::commands` implements in-memory atomic transactions,
monotonic revisions, idempotency and undo/redo. Durable crash recovery, provider
adapters and an interactive editor are not implemented. `engine-ai` still has
prototype direct Spawn/Kick helpers; `engine-endpoint` has protocol helpers, not
a working MCP server. Those helpers do not authorize a second write path.

## 1. Three complementary AI capabilities

| Capability | Where it runs | Authoritative input |
|---|---|---|
| Gameplay AI: perception, utility/BT/GOAP, navigation goals, memory | Fixed-tick simulation with operation budgets and stable tie-breaking | Simulation state and recorded inputs |
| Authoring AI: inspect, propose scene/material/behavior edits, test | Editor/service workers | Validated command transactions |
| Optional live inference: dialogue, high-level intent, adaptive suggestions | Local sidecar or remote service, asynchronously | Accepted, tick-stamped actions; deterministic fallback on failure |

Keep gameplay AI usable with no models. Put provider SDKs/HTTP/inference runtimes
in optional adapters outside `engine-core`. A shared first-class API is what
makes AI core to the engine. Offline bake remains preferred for generated assets,
embeddings and policies; live services are allowed within these boundaries.

## 2. Provider-neutral service protocol

Define engine-owned serializable types first, with versioned schemas and
conformance fixtures. Start with only the capabilities the first tasks require.

| Type | Minimum fields/behavior |
|---|---|
| `ProviderCapabilities` | Protocol version; text/tool/structured-output/vision/embedding support; input/output limits; local/remote mode; cancellation support |
| `AiRequest` | Request ID, task kind, model selector, context references + digests, expected output schema/version, budget, deadline, provenance policy |
| `AiEvent` / `AiResult` | Streaming progress where available, complete proposal, usage (unknown allowed), finish reason, structured error, request ID |
| `Proposal` | Source request, base document/world revision, typed actions, referenced object/asset versions and diagnostics |
| `AiError` | Unsupported capability, timeout, cancelled, unavailable, quota/auth, malformed output, stale context; retryability explicit |
| `AiSessionPolicy` | Enabled adapters; offline/local-only/remote-allowed mode; allowed destinations and data classes; tool/object scopes; request, concurrency and spend limits; explicit fallback order |

Capabilities are negotiated; do not assume every model supports tools, JSON
schemas, images or identical tokenization. Adapters translate vendor formats.
Canonical conversations/context are engine data; provider-specific caches are
disposable. Do not persist hidden reasoning or credentials into project files.

Bound concurrent requests, queue length, context size, output size, retry count,
token/cost budget and elapsed deadline. Treat streaming text as preview only;
validate the complete proposal before applying. Cancellation/scene-close makes
late output ineligible. Honor service backoff; retries reuse a request identity
but cannot silently duplicate a world mutation. Show offline/quota/error state.

Use a deterministic mock for CI, plus one local and one remote real adapter for
integration proof. “OpenAI-compatible” alone is not backend independence.
Switching providers requires config changes only, not scene, tool or gameplay
code changes. OpenCode's own provider selection is development tooling, separate
from this engine contract.

### Capability profiles, privacy and fallback

AI-native means engine services are callable both **inside the editor/game** and
through MCP. Neither an external coding assistant nor MCP is required for local
gameplay. Internal callers use the service API, not a loopback network roundtrip.

- The first profile is text observations to a schema-validated `Proposal`.
  Native tool calling and strict JSON support are independent capabilities; a
  text adapter may parse and validate a complete response, but never claim native
  structured-output support. Unsupported requests return `UnsupportedCapability`.
- Vision adds revision-stamped captures when supported. Embedding, speech and
  generation capabilities can be described without implementing adapters for
  them now. A text-only provider remains useful; no lowest-common-denominator
  API forces every adapter to pretend it supports every feature.
- Offline means no provider network calls. Local-only permits configured local
  endpoints only, with redirects and endpoint changes checked. Remote use needs
  explicit session authorization. **Never escalate local requests to a cloud
  provider on failure.** The default fallback is scripted behavior or a visible
  refusal. A configured fallback must still satisfy the original capability,
  privacy, cost and deadline limits.
- Credentials are adapter-local secret references, never scene/save fields,
  prompts, tapes or diagnostics. Context has provenance, revision, asset digests
  and a data classification; show what will leave the machine. Logs and bug
  bundles redact secrets and omit raw prompts/assets unless the user opts in.
- Usage and price can be unknown. Report unknown, not zero. A hard spend cap
  requires a conservative reservation or refusal when cost cannot be bounded.
  Retries share the original deadline and budget; backoff is not permission to
  evade a provider quota.
- One request's streaming text is preview, never a partial command. After
  cancellation, scene close, provider switch or expired approval, discard late
  output. Reconnect queries a job/receipt before retrying an uncertain write.

### One bounded assistant run (C2, PV1–PV2, EA1)

Use an explicit state machine: observe → propose → validate → await approval →
apply → run checks → report, with cancelled/failed/needs-user terminal states.
Persist only versioned task state and receipt references, not hidden reasoning.
On resume, re-read revisions and reconcile in-flight jobs; never blindly replay
writes. A run has a maximum tool count, repair count, concurrency and elapsed
deadline. Stop on the same failure twice with its reproduction.

The editor's task panel shows current selection/context, backend, privacy mode,
proposed diff, checks, cost (or unknown), cancel and undo. Start with one useful
job: diagnose a misplaced prop, propose a repair, preview/apply it, play-check
and undo. Manual, local, remote and external-MCP clients use the same commands.
No swarm, vector database or generated code execution is needed for that job.

## 3. One mutation service for UI, AI, scripts and automation

The inspector, gizmo, CLI and endpoint are clients of the same local typed
command service. MCP/HTTP are optional transports, never core engine types.

Transaction envelope: schema version, transaction ID, idempotency key, expected
revision, caller capabilities, typed operations and referenced asset versions.

1. Read a bounded snapshot: stable IDs, schema metadata, revision and diagnostics.
2. Validate all operations: types, finite/range constraints, references, capability
   scope, operation/resource budgets, scene rules and revision preconditions.
3. Prepare a preview/diff on staged state. Dry run must have no persistent side
   effects. Files from assets/prompts are data, not new tool permissions.
4. Apply atomically and publish a durable journal entry. Batches are all-or-nothing
   unless an explicitly named partial mode returns per-op outcomes. No silent
   “skip bad IDs” for authoring transactions.
5. Re-read postconditions; return revision, document digest, diagnostics and undo
   token. On failure, restore the prior state or recover from the durable log.

Same idempotency key + same payload returns the recorded result; same key with
a different payload is a conflict. Store that mapping durably. Undo applies a
version-aware inverse/snapshot transaction and detects intervening edits; it
does not blindly restore an old document over another user's work.

Revisions are monotonic: apply, undo and redo each publish a new revision and
a number is never reused, so a batch prepared before an undo stays stale even
when the hierarchy looks identical again. Retrying the key of an undone batch
is refused (`UndoneKey`), never reported as applied. Implemented in
`crates/editor/src/commands.rs` with regression tests for both races.
The full observe → propose → check → preview/apply → play-test → verify loop
is in `ARCHITECTURE.md`.

Asynchronous import/bake is a separate job with a job ID and immutable results;
the final document transaction binds validated artifacts. It is not an “atomic
edit” that has already overwritten external files halfway through validation.

Manual edits apply directly through this service. AI edits preview by default;
users can enable a revocable policy for approved reversible operations. That
policy is not a blanket approval token: the host must re-evaluate and issue a
fresh action/revision-bound authorization for each proposal, within the allowed
scope and expiry. The user can revoke it at any time. Approval
is not a substitute for validation, and keyword “injection guards” do not prove
safe authorization. Model/tool output cannot grant its own capabilities.

CAP1 enforces policy in the command service, independent of transport. Bind
approval to the session, document revision, canonical action digest and expiry;
changing any of them requires a fresh preview/approval. Do not implement custom
cryptography. Serialize applies per document, allow bounded concurrent readers
and reject stale conflicting writers. Asset metadata, tool output and retrieved
text are untrusted context, never instructions that widen authority. The first
MCP door exposes no arbitrary shell, code execution or unrestricted file writes.

## 4. Runtime authority and replay

For live NPC inference, snapshot relevant state at tick T and issue a request
outside the simulation. On receipt, the authority validates world/entity
generation, age, legal action and budget. It schedules accepted actions at an
explicit future tick, or records a rejection/fallback. Networking carries those
accepted actions, not different provider calls from every client.

If output is late, invalid or absent, deterministic gameplay proceeds using its
scripted policy. The tick never waits. Replay and rollback use recorded actions
and the full gameplay AI state; they never resend the prompt. Text/speech used
only for presentation can be un-hashed, but quest, memory and rule changes must
enter authoritative state. Declare which memory is authoritative vs a retrievable
editor cache and version the associated content.

## 5. Editor minimum product

- Project/open/save/recovery; asset browser/import diagnostics and job progress.
- One 3D viewport with selection/picking, hierarchy, transform gizmo and snapping.
- Schema-backed inspector, materials and prefab references; undo/redo and diff.
- Play/pause/step/stop with a child game process, logs and profiler; edits apply
  back only through a reviewed document diff. Runtime and editor share contracts.
- AI panel with context scope, backend/model, budget/latency/error indicators,
  proposal preview, validation diagnostics, apply/undo and play-test receipt.
- NPC debugger showing perception, current goal/BT state, budget and action log.
- Keyboard/focus/text input, DPI scaling and accessible labels from the start.

Choose one dock/UI stack after a working viewport prototype. Theme/icon choices
are polish, not architecture. Native-code hot reload is not needed for this MVP.

## 6. Acceptance suite

- All clients produce equivalent normalized commands for the same edit.
- 100 edit/undo/redo operations restore a saved document; save/reopen roundtrips.
- Stale revisions, stale entity generations, duplicate keys with changed payload,
  malformed numbers and mid-batch failure produce no partial world writes.
- Kill/restart between journal/apply/publication steps; recover one valid state.
- Cancel/close/reload while a request is in flight; late output cannot mutate it.
- Test slow, offline, malformed and quota-limited providers with bounded queues.
- Local-only failure produces no outbound remote request; secrets are absent
  from serialized scenes, traces and bug bundles. Unknown cost stays unknown.
- A forged, reused, expired or stale approval cannot write. Two agents proposing
  from the same revision cannot overwrite each other silently. Tool/schema
  drift and unsupported capabilities fail with legal alternatives.
- A restarted assistant reconciles a committed receipt instead of applying it
  twice; cancelled jobs cannot publish late scene changes. Malicious instructions
  in imported metadata cannot enable tools or change privacy policy.
- Replay at least 10,000 ticks using recorded accepted actions with providers
  disconnected; compare per-checkpoint state, not just output text.
- Evaluate 20 fixed authoring tasks across mock/local/remote backends; report
  task success, rejected bad proposals, human edits, latency and measured usage.
  Define the manual baseline before claiming productivity improvements.

Build transactions/schema before chat UI. Build one complete assisted workflow
before adding agent swarms, vector databases or provider-specific features.
