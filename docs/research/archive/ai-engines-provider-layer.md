# AI-engine provider layer survey (RES-AIENG-3, 2026-10-01)

Goal: make `crates/provider` (C2 contract + mock/local/remote adapters +
AX1 vendor adapters + PV1/PV2 conformance suite) the best AI-agnostic layer
of any engine. Research only; no code edits, no donor bytes copied.

Our baseline (read 2026-10-01): `crates/provider/src/lib.rs` (C2 contract,
`PROTOCOL_VERSION=1`), `core.rs` (shared guards/jobs/scripted failures),
`protocol.rs` (engine-owned serializable types, canonical LE bytes, no serde),
`suite.rs` (identical PV1 failure suite runs unchanged on every adapter);
`docs/EDITOR_AI_CONTROL.md` (negotiated capabilities, privacy no-escalation,
tick never waits, replay from recorded actions); `docs/plan/platform-live-ai.md`
(LAI1-8 voice/moderation/vision-judge/recall/shipped-model/quota rows).

Method: GitHub REST `api.github.com/repos` (stars + license detection,
2026-10-01) until its unauthenticated quota 403'd; remainder via repo HTML
pages and docs.rs. Web search was down (transport error), so capability notes
are from repo descriptions/topics plus prior knowledge, marked accordingly.
"NOASSERTION" = GitHub could not classify the LICENSE file; read it before
porting anything. AGPL/EUPL rows are ideas-only, never code donors.

## Survey (27 repos; stars approx, read 2026-10-01)

| # | Repo | Stars | SPDX (LICENSE verification) | Capability | What we lack vs it | Weakness |
|---|---|---|---|---|---|---|
| 1 | https://github.com/jeremychone/rust-genai (`genai` crate) | ~0.9k (docs.rs badge) | MIT (crates.io metadata; LICENSE file not individually fetched) | Multi-provider Rust client: OpenAI/Gemini/Anthropic/Ollama/Bedrock/Vertex/Groq/DeepSeek in one API | Per-provider adapter breadth (we have OpenAI-compat/Anthropic/Gemini/Ollama only); streaming SSE helpers | No conformance suite across backends; no determinism or replay story |
| 2 | https://github.com/0xPlaygrounds/rig | 8.8k | MIT (API-detected) | Modular Rust LLM apps: agents, RAG, vector stores, tools | Agent/RAG/vector-store abstractions; native tool-calling capability flag | No cross-backend equivalence tests; model output trusted, not validated |
| 3 | https://github.com/rustformers/llm | 6.2k | Apache-2.0 (API-detected) | Rust LLM inference ecosystem (GGML bindings) | In-process inference path for shipped models (LAI7 still open) | ARCHIVED/unmaintained; superseded by mistral.rs/candle |
| 4 | https://github.com/pepperoni21/ollama-rs | 1.1k | MIT (API-detected) | Typed Rust client for the Ollama API | Chat history builders; model-pull management surface | Single-backend only; no mock parity, no failure-mode suite |
| 5 | https://github.com/64bit/async-openai | 2.0k | MIT (API-detected) | Full async Rust OpenAI client (chat, audio, embeddings, files) | Embeddings/files/audio client surface; whisper-style STT shape for LAI2 | OpenAI-shaped: "OpenAI-compatible alone is not backend independence" |
| 6 | https://github.com/EricLBuehler/mistral.rs | 7.7k | MIT (API-detected) | Fast Rust LLM inference (many architectures), OpenAI-compat server, tool calling, structured output | In-process GPU inference + native constrained decoding; vision + tool-call serving | Inference server, not a provider-neutral contract; no sim-tick boundary |
| 7 | https://github.com/huggingface/candle | 21.1k | Apache-2.0 (API-detected) | Minimalist Rust ML framework (CPU/CUDA/Metal) | Tensor substrate if we ever train/distill in-repo | Framework, not LLM plumbing; no serving, tools, or replay story |
| 8 | https://github.com/tracel-ai/burn | 16.0k | Apache-2.0 (API-detected) | Full Rust DL framework, ONNX import, WebGPU/CUDA/ROCm | ONNX-importable training substrate; WebGPU backend lessons for GPU sharing | Heavyweight for inference-only NPC needs; no agent/provider layer |
| 9 | https://github.com/pykeio/ort | 2.5k | Apache-2.0 (API-detected) | Rust ONNX Runtime bindings (CPU/GPU, training) | Mature ONNX exec for shipped classifier/moderation models (LAI4 local arm) | Ships native ONNX libs; GPU-sharing with renderer is our problem, not theirs |
| 10 | https://github.com/sonos/tract | 3.1k | NOASSERTION (API; custom LICENSE file, read before porting) | Tiny self-contained TF/ONNX inference, pure Rust, no system deps | Zero-dependency local inference shape (ideal for moderation classifier) | License file unclassified; LLM coverage thin; no serving/tools |
| 11 | https://github.com/utilityai/llama-cpp-rs | 0.7k | Apache-2.0 (API-detected) | Safe Rust bindings to llama.cpp | review | Small bus factor; tracks upstream llama.cpp churn |
| 12 | https://github.com/BerriAI/litellm | 59.9k | NOASSERTION (API; claims MIT upstream, LICENSE unread here) | 100+ LLM APIs behind OpenAI format; cost tracking, guardrails, load balancing, logging, Rust core + Python SDK | Spend/cost ledger per key; gateway load-balancing + fallback routing; model-price table | Proxy-shaped, not embeddable; no deterministic replay; license file unverified |
| 13 | https://github.com/vercel/ai | 27.1k | NOASSERTION (API; claims Apache-2.0 upstream, LICENSE unread here) | TypeScript AI toolkit: unified provider calls, streaming UI, tool use, agents | Streaming text protocol maturity (we treat stream as preview-only, thinner impl); `stream.rs` could adopt their event shapes as ideas | TS/web-only; no Rust/game embedding; no offline replay |
| 14 | https://github.com/567-labs/instructor (moved from jxnl/instructor) | 14.0k | MIT (API-detected) | Structured outputs via validation/retry (pydantic) | Retry-with-validation loop policy as a provider capability (bounded repair count) | Retry-based, not grammar-guaranteed; Python-only; retries cost tokens |
| 15 | https://github.com/dottxt-ai/outlines | 15.9k | Apache-2.0 (API-detected) | Regex/CFG-guided structured generation, fast index | Grammar-constrained decoding idea: our `ProposalAction` enum is a closed grammar with no constrained sampler behind it | Python + server-side; needs model access; no game-tick or replay coupling |
| 16 | https://github.com/guidance-ai/guidance | 21.8k | MIT (API-detected) | Guidance language for controlling LLMs (templates + constraints) | Interleaved prompt/program control-flow idea for authoring runs (C2 state machine is fixed) | Largely superseded by llguidance; notebook-centric; no determinism story |
| 17 | https://github.com/BoundaryML/baml | 9.4k | Apache-2.0 (API-detected) | Schema-first language: function signatures as the contract, multi-provider | Function-signature-as-contract idea: our `TaskKind` set (6 kinds) is narrower than typed function routing | New language to learn; runtime outside Rust game loop; no replay |
| 18 | https://github.com/mlc-ai/xgrammar | 1.9k | Apache-2.0 (API-detected) | Fast portable structured generation (byte-level pushdown, C++) | Near-zero-overhead grammar enforcement for `Proposal` bytes; native lib, FFI-able | C++ dependency; needs engine-side grammar compiler for our action schema |
| 19 | https://github.com/guidance-ai/llguidance | 0.9k | MIT (API-detected) | Super-fast structured-output core in Rust | Rust-native constrained-decoding core we could call from a local adapter | Young; tokenizer co-versioning with the serving model is fiddly |
| 20 | https://github.com/betamaxpy/betamax | 0.6k | NOASSERTION (API; claims Apache-2.0 upstream, LICENSE unread here) | HTTP VCR cassettes for python-requests tests | Cassette match-once semantics + secret-scrubbing conventions for our audio/model cassettes (LAI1/LAI5) | Python-only, requests-only; no digest-keyed binary cassettes |
| 21 | https://github.com/modelcontextprotocol/rust-sdk (rmcp) | 4.0k | NOASSERTION (API; claims MIT upstream, LICENSE unread here) | Official Rust MCP client/server SDK | Sampling primitive + elicitation shapes for a future MCP-exposed provider door | SDK only, no AI-agnosticism: every server still assumes a model behind it |
| 22 | https://github.com/a2aproject/A2A | 26.0k | Apache-2.0 (API-detected) | Agent-to-agent interop protocol (opaque agents, task/artifacts) | Agent-card discovery idea for multi-NPC delegation (NPC asks specialist NPC) | Spec-heavy, JSON-RPC web-shaped; no tick-bounded game profile |
| 23 | https://github.com/ollama/ollama | 182k | MIT (API-detected) | Local model daemon, OpenAI-compat API, our `LocalAdapter` target | Model management + keep-alive discipline; vision/tool-call server parity | External daemon the player must install (plan notes this); GPU shared by luck |
| 24 | https://github.com/ggml-org/llama.cpp | 130k | MIT (page-verified) | Local inference runtime + OpenAI-compat `llama-server` (LAI7 donor runtime) | Shipped-model runtime under OPT27 arbiter; grammar-constrained `/chat` sampling | C++ runtime beside a Rust engine; GPU contention with renderer unsolved |
| 25 | https://github.com/nobodywho-ooo/nobodywho | 1.5k | EUPL-1.2 (page-verified; copyleft — ideas only, no code) | Godot plugin: local NPC dialogue in-engine, closest game-embeddable rival | NPC-bubble UX + per-NPC model config UX we should beat with moderation + replay | EUPL-1.2; prompt-in-scene; no action validation, no replay, no moderation gate |
| 26 | https://github.com/oobabooga/textgen (renamed from text-generation-webui) | 47.7k | AGPL-3.0 (page-verified; ideas only, never a donor) | Local LLM desktop app, OpenAI/Anthropic-compat API, tool calling | Compat-API surface checklist for our local adapters (what fields clients expect) | AGPL-3.0: no code ports, ever; app-shaped, not a library |
| 27 | https://github.com/dnaeon/go-vcr | 1.4k | BSD-2-Clause (page-verified) | HTTP record/replay cassettes for Go tests | Cassette versioning + custom matchers for request-digest-keyed model cassettes | Go-only; HTTP-level, knows nothing of prompts, actions, or ticks |

## Gap matrix vs `crates/provider`

| Capability | Us (evidence) | Best rival | Gap |
|---|---|---|---|
| Provider breadth | 4 adapters: mock/local/remote + OpenAI/Anthropic/Gemini/Ollama (`lib.rs:11-26`) | litellm 100+; genai 10+ | Add adapters behind the same PV1 suite, never new shapes |
| Grammar-constrained actions | `ProposalAction` closed enum + post-hoc `validate_proposal` (`core.rs:400`) | xgrammar/llguidance/outlines enforce at sampling | No constrained sampler; malformed output wastes a roundtrip |
| Tool calling | None (text parse + validate only) | rig, mistral.rs, textgen servers | No native tool-call capability flag in `ProviderCapabilities` |
| Streaming | `AiEvent::Progress` preview-only (`protocol.rs:211`) | vercel/ai stream protocol | Thin: no delta-merge/token-meter standard |
| Token budget arbiter | `Budget.max_output_tokens/max_ms`, unknown-cost refusal (`protocol.rs:94`) | litellm cost ledger + router | No per-NPC/per-session spend ledger or price table |
| Local GPU sharing | OPT27 arbiter planned; no code | mistral.rs/candle in-process; llama.cpp sidecar | Renderer vs model GPU contention unsolved anywhere we own |
| Embedding recall | None (LW2 integer salience) | rig vector stores; async-openai embeddings surface | LAI6 open: no embedding adapter, no int8 top-k |
| Voice/moderation/vision | Plan rows LAI1/2/4/5, injection scorer landed | NobodyWho (NPC UX), ort/tract (local classifiers) | Local classifier runtime + moderation profiles unbuilt |
| Cassette replay | Mock script + recorded accepted actions (EDITOR_AI_CONTROL §4) | go-vcr/betamax matchers + scrubbing | No digest-keyed byte cassettes with secret scrub for audio/model I/O |
| Agent interop | Single-request C2 jobs | A2A cards; rmcp sampling | No NPC-delegation or sampling-exposed door (correctly out of scope now) |

## Top 10 steal-or-beat tasks (ranked)

1. Grammar-constrained proposal decoding — compile `ProposalAction` schema to an
   llguidance/xgrammar-style constraint for local adapters; malformed becomes
   unreachable, not merely rejected. Owner: `provider`. Accept:
   `cargo test -p engine-provider --lib grammar` (roundtrip + fuzz: 0 malformed).
2. Token-budget arbiter with spend ledger — per-NPC/per-session token + spend
   ledger, price table, reservation-or-refusal (litellm idea, our unknown-cost
   rule kept). Owner: `provider` (feeds LAI8). Accept:
   `cargo test -p engine-provider --lib quota`.
3. Native tool-call capability flag — `ProviderCapabilities.tool_calling`,
   negotiated like vision; text adapters keep parse-and-validate, never claim
   it (rig idea, our honesty rule kept). Owner: `provider`. Accept:
   `cargo test -p engine-provider --lib tool_cap`.
4. Digest-keyed byte cassettes with secret scrub — request-digest-keyed
   request/response cassettes incl. audio (go-vcr/betamax idea) for LAI1/LAI5;
   replays run offline x2. Owner: `provider`. Accept:
   `cargo test -p engine-provider --lib cassette`.
5. Local GPU-share arbiter (OPT27) — frame-budget-aware scheduler over an
   embedded runtime (llama.cpp donor runtime, mistral.rs/candle lessons);
   renderer always wins. Owner: `provider` (LAI7). Accept:
   `cargo test -p engine-provider --lib embedded` + frame-budget proof.
6. Embedding recall adapter — optional int8 embeddings, deterministic top-k
   ties-by-id, LW2 fallback (rig idea, our fallback law kept). Owner: `ai`
   (LAI6). Accept: `cargo test -p engine-ai --lib recall`.
7. Moderation profiles + local classifier — rating profiles over every model
   line, blocklist + ort/tract-classifier adapter, fail closed to scripted
   lines (beats NobodyWho: moderation + replay where it has neither). Owner:
   `provider` (LAI4). Accept: `cargo test -p engine-provider --lib moderate`.
8. Vision-judge calibration harness — 50 owner-labeled captures, ≥80%
   agreement before scores count, cassette-recorded (LAI5). Owner: `playtest`.
   Accept: `cargo test -p engine-playtest --lib vision_judge`.
9. Adapter breadth behind one suite — add DeepSeek/Groq-style adapters reusing
   the OpenAI-compat path, each passing the unchanged PV1 suite (genai breadth,
   our equivalence proof kept). Owner: `provider`. Accept:
   `cargo test -p engine-provider --lib` green incl. new adapter matrix rows.
10. Streaming delta standard — typed token-delta events with meter, still
    preview-only until the complete validated proposal (vercel/ai idea, our
    "streaming text is preview" law kept). Owner: `provider`. Accept:
    `cargo test -p engine-provider --lib stream`.

Rejected (not tasks): porting AGPL textgen or EUPL NobodyWho code (license);
a second write path for any MCP/A2A door (architecture law); model weights in
the tick (determinism law); provider SDK types leaking past adapters (C2 law).

## What makes us strictly smarter (no rival above has all of these)

- Determinism-safe action validation: closed `ProposalAction` enum, integer
  positions, bounded labels; invalid output is `MalformedOutput`, never applied.
- Fork-checked plans: `sim.plan_goals` verified against staged state before
  apply; rivals trust model text.
- Replay of model decisions with no model: recorded accepted actions resim
  10,000 ticks with providers disconnected; rivals replay prompts or nothing.
- Privacy no-escalation law: local-only failure never touches cloud; default
  fallback is scripted behavior or visible refusal.
- Unknown cost stays unknown: hard cap + unbounded cost is refusal
  (`QuotaAuth`), never a guessed zero — the opposite of dashboard optimism.
- Idempotent request identities: same key + payload replays the recorded
  result; same key + changed payload is a conflict, never a double mutation.
- One conformance suite for every backend: mock/local/remote run the identical
  PV1 failure modes, so "works on mock" means "works on remote".
- The tick never waits: late/invalid/absent output falls back to scripted
  policy; networking carries accepted actions, not per-client prompts.
