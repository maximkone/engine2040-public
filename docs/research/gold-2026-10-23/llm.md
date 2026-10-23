# LLM and agent gold cards (2026-10-23)

Scope: the model-facing layer (provider, playtest eval, door tool surface), looking beside the
ranked finish-line rows, not at them. Every claim below was re-read in the tree at HEAD
`9a6289a`; the earlier hunt docs were used only to avoid duplicate cards, never as evidence. No
card here is hunt-doc only.

Verdict: the repo has more AI plumbing than AI evidence. The expensive parts (real HTTP adapters,
a router, cassettes, a scoring oracle, a 95-verb door) are built. The cheap parts that connect
them are missing: one real model call in the matrix, one provenance stamp, one worked example,
one summed budget. Three plan rows already read the plumbing as proof.

## Already there (checked, not cards)

- Prompt-injection screen, 5 categories: `crates/endpoint/src/injection/mod.rs:247`, wired on the
  import path at `crates/endpoint/src/scene_tools.rs:1345`.
- MCP resources, prompts, progress, annotations, outputSchema, ttlMs: present in the door.
- Prompt versions, router, ladder, cassettes: `crates/provider/src/{prompt,router,ladder,cassette}.rs`.
  Built and tested, but `crates/provider/src/agent_loop.rs:6` still says "no in-repo consumer yet".
- Scene diff, session activity, replay bisect, result caps, typed error contract: live in
  `crates/endpoint/src/packs/{session,verify,replay}.rs`.
- Tool-surface size as a standalone row: AX10 already rejected it (`docs/plan/ai-native.md:67`).

## Ranked cards

### 1. The three-backend eval is one mock with three labels

**Gap:** every one of the 75 eval rows runs `MockAdapter`; `local` and `remote` change the policy
label, the model name and the cassette file name, not the code that produces the answer.

**Evidence:**
- `crates/playtest/src/door.rs:158-164`: `backend_label(call.backend)` then
  `MockAdapter::new(script_steps(call.script))` for every backend. There is no other adapter arm.
- `crates/playtest/src/eval.rs:669-730`: `backend_wiring` returns `(AdapterKind, model, privacy,
  policy, caps)`; `AdapterKind::Local`/`Remote` is a request field the mock then answers.
- `crates/playtest/src/eval.rs:1256-1279`: the 75 rows are `for backend in BACKENDS` around the
  same `run_authoring_row`/`run_play_row`.
- `crates/playtest/src/agent.rs:69-79`: the enum says it itself, "Labels only: the agent runs the
  same pipeline for every backend".
- Claims that read those rows as three providers: `docs/plan/ai-native.md:45` ("identical outcome
  vectors per backend"), `:48` ("75 rows ... x 3 backends"), `:67` ("mock/local/remote ... 0.0pp").

**Live donor:** llama.cpp at `1a24df163920a4744a95373d178d790b72a9bb2f:tools/server/README.md`
(OpenAI-compatible `/v1/chat/completions`). `crates/provider/src/local.rs:293` already speaks that
URL, so the real local leg needs wiring, not a new client.

**Why it matters:** 25/25 per backend proves the wiring policy is consistent, not that a model did
the task. Worse, AX10's rejection of the tool-surface row rests on "0.0pp across surfaces": if the
answer comes from a fixed script, 0.0pp is guaranteed before the experiment starts. One rejected
spike and three plan claims sit on a number that cannot vary.

**Effort:** S to route `Backend::Local`/`Remote` through `LocalAdapter`/`RemoteAdapter` behind a
recorded cassette; M to re-record the 75 rows against a real local model.

**Proof:** one test that points `Door` at a fake HTTP server, runs one row as `Backend::Local`, and
asserts the server saw exactly one `/v1/chat/completions` request and that the row's adapter field
is `Local`, not `Mock`. Today the server sees zero requests. Then the 75-row report prints the
adapter per row.

**Kill:** if the product decision is that the eval is offline by design, the honest fix is to
rename the labels `mock-a/b/c`, strike the per-backend claims, and re-run AX10 as unmeasured. That
is a doc fix, and this card dies as engineering.

**Why we missed it:** determinism hides it. A mock replays byte-identically, which reads as a
stable backend, and the receipt only compared outcome vectors, which a mock guarantees.

### 2. The untrusted-text manifest names 26 tools; 6 run the screen, and no write says "model"

**Gap:** the door advertises a pre-run screen for 26 untrusted-text tools, but only 6 calls reach
it; the write verb a model batch goes through, `tx.apply`, is named in the manifest and skips the
seam. Every batch it applies is stamped `script("mcp")`, so no later reader can tell model text
from human text.

**Evidence:**
- `crates/endpoint/src/sandbox/manifest.rs:20-47`: 26 names, `tx.apply` at line 25.
- `crates/endpoint/src/mcp_tools.rs:118`: `tx.apply` calls plain `dispatch`.
- Screened calls: `scene_tools.rs:1586` (`dispatch_screened`), `insight/mcp_tools.rs:246,276,308,340`
  and `packs/mod.rs:866` (`dispatch_with_payload`). That is six of the 26.
- `crates/endpoint/src/mcp.rs:566`: `build_batch` stamps every door batch
  `ProvenanceStamp::script("mcp")`, whatever wrote it.
- `crates/endpoint/src/packs/grammar.rs:96-110`: the grammar test proves 12 hostile strings parse
  clean once wrapped in a typed `set_label`; the gate checks shape, not meaning.
- `crates/endpoint/src/assistant.rs:324`: `fence_data` exists, but its only consumer is the
  assistant transcript (`assistant.rs:1092`), not a door answer.
- `crates/endpoint/src/mcp_dto.rs:165-176`: `SetComponent { value: Value }` can carry an arbitrary
  JSON string into the document.

**Live donor:** CaMeL, arXiv 2503.18813 (data-flow labels and capabilities; 77% on AgentDojo with
provable security). Test shape: AgentDojo, arXiv 2406.13352 (97 tasks, 629 security cases).

**Why it matters:** the manifest is a promise the dispatch seam does not keep, on the exact verb
that mutates the scene. A hostile string in a component value lands unmarked; the read side prints
poses and ids but no provenance, so the next agent cannot tell it from human data. This is stored
injection with the label removed, not a hypothetical prompt trick.

**Effort:** S. Route `tx.apply`, `tx.preview`, `scene.read` and `scene.brief` through the screened
seam (or delete them from the manifest and say why), and let `tx.apply` carry a model stamp when
the session is model-driven.

**Proof:** a routing test that every name in `SCANNED_TOOLS` reaches `check_before_run`, plus one
end-to-end `tx.apply` carrying the manifest's own hostile string, asserting `hostile_text` and an
unchanged revision. Then a 20-case write-then-read suite with 0 successes and one positive control.

**Kill:** if no `OpDto` variant can carry free text (all values numeric or closed enums), the write
half dies and the card shrinks to read-path provenance, which is XS.

**Why we missed it:** the screen was built for brief import (`scene.place`, AB1) and tested there;
the manifest then grew as a list, and no test asserts that every listed tool actually routes
through the seam.

### 3. The eval has an oracle but no reward loop

**Gap:** the door can score a replay against metric rules, but nothing lets a model propose, get
scored, and try again. Eval rows are fixed scripts; proposals never enter the loop.

**Evidence:**
- `crates/endpoint/src/play.rs:1555`: `play.run` drives a tape.
- `crates/endpoint/src/packs/verify.rs:21-32`: `hits.max <= 4`, `damage.final >= 0`,
  `no_divergence` and friends are an executable oracle over two real runs.
- `crates/playtest/src/eval.rs:236`: `TaskScript` is a closed enum (`Clean`, `Stale`,
  `Malformed`, `PrivacyProbe`, ...); rows are predetermined.
- `crates/playtest/src/agent.rs:104-125`: `Policy` seeds bugs (`Stall`, `StallAll`, `Divert`); it
  does not consume model output.
- `git grep "play.search"` returns nothing: no search, no best-of-N, no attempt table.

**Live donor:** AlphaCode, arXiv 2203.07814 (sample many, execute, filter, cluster); SWT-Bench,
arXiv 2406.12952 (tests as the verifiable reward).

**Why it matters:** the oracle and the runner are the expensive half and they already exist. Best
of N is the cheapest capability jump available, and the only one that turns the door's own proof
verbs into a model improvement loop rather than a one-shot check.

**Effort:** M. One harness that takes a seeded bug, asks the model for K typed patches, scores
each with `play.run` plus the verify rules, and keeps the best.

**Proof:** 10 seeded-bug tasks, K=5, best-of-5 pass rate at least first-try + 20 points, with the
reward distribution and token cost printed. Fail if the gain is inside one task's noise.

**Kill:** if the model cannot emit schema-valid proposals (malformed above 80%), the loop measures
the wire, not the model; the card waits on card 1.

**Why we missed it:** the eval culture is "replay twice, same bytes". That is the right rule for
proof and the wrong one for search, and nobody wrote the second harness.

### 4. Tool schemas have no call examples

**Gap:** every verb is described in prose plus JSON Schema; none carries a worked call.

**Evidence:**
- `git grep "examples"` under `crates/endpoint/src` returns nothing.
- `crates/endpoint/src/discover.rs:300-304`: the simplest args struct is one optional bool; the
  pattern is fields and doc lines only.
- `crates/endpoint/src/packs/verify.rs:41-42`: "DTO structs carry no field docs on purpose: every
  doc line inflates the advertised schemas, and the discovery budget is a gate." The budget
  actively pushed documentation out.
- The hardest verbs are nested and optional: `tx.apply` (`mcp_dto.rs:129-184`), `scene.query`,
  `play.run`.

**Live donor:** Anthropic, "Introducing advanced tool use", 2025-11-24, Tool Use Examples
(https://www.anthropic.com/news/advanced-tool-use). Vendor-reported: accuracy on complex parameter
handling rose from 72% to 90%.

**Why it matters:** JSON Schema says what is valid, not which combination is sane. `tx.apply`
alone has eight op kinds, an approval triple, an idempotency key and an expected revision. One
valid example per hard verb is the cheapest accuracy lever that touches no runtime code.

**Effort:** S, plus budget. Headroom to the 75,000 budget is 13,258 chars; the guardrail starts
trimming the advertisement at 72,952 (`crates/endpoint/src/mcp.rs:158-162`), so 11,210 chars are
usable before a load starts hiding verbs.

**Proof:** add one example to `tx.apply`, `scene.query` and `play.run`; a test parses and dispatches
every example; a 20-prompt local-model run shows first-call schema-valid rate up at least 10 points
at no more than 2,048 extra chars.

**Kill:** if the local model is not available to measure, or the examples do not clear the added
chars, drop the card. A presence-only test is not worth the surface.

**Why we missed it:** the gates count chars and tests, not model errors. Examples looked like pure
cost because nothing measured the errors they prevent.

### 5. No first-turn context budget: the door advertises 15.4k tokens to a 4k-token local model

**Gap:** each surface has its own cap; nothing adds up what one real first turn costs.

**Evidence:**
- `crates/endpoint/src/mcp.rs:158`: the default tool list measures 61,742 chars; the gym-loaded one
  is 74,084, against a 75,000 budget (`mcp.rs:162`).
- The repo's own rule is chars/4 (`crates/endpoint/src/assistant/transcript.rs:4,10`), so the
  default list is about 15,436 tokens before a single scene row.
- `crates/provider/src/policy.rs:46`: the local adapter's input limit is 4,096 tokens, and
  `local_http()` is `mock()` (`policy.rs:57-59`), so 4,096 is the local wire's real cap.
- `crates/endpoint/src/assistant.rs:49`: the whole assistant loop budget is 2,048 tokens; the
  prompt cap is 1,024 chars (`assistant.rs:55`).

**Live donor:** Anthropic, "Introducing advanced tool use", 2025-11-24, Tool Search Tool
(https://www.anthropic.com/news/advanced-tool-use). Vendor-reported: 85% fewer tokens, and Opus 4
accuracy on large tool libraries rose from 49% to 74%.

**Why it matters:** the door already has progressive disclosure (`api.search`, `api.load`), but a
fresh session still gets everything. A client using the repo's own local adapter cannot fit the
default list at all: 15,436 tokens against a 4,096-token cap. That is arithmetic, not taste, and
it means the local leg of card 1 cannot work until this is fixed.

**Effort:** S.

**Proof:** one test that sums instructions, the advertised list and one `scene.brief` for a
200-object scene, and asserts the total fits 4,096 tokens. Today it fails by roughly 4x. Expose the
same number on `door.status` so a client can budget before it calls.

**Kill:** if the 4,096-token limit is a prompt-only cap that excludes tool definitions on the wire,
the comparison is apples to oranges; then measure against the smallest declared client window, and
if none exists, the card is unfalsifiable and dies.

**Why we missed it:** every surface has its own cap and its own green test. Nobody added them up
for one turn.

## Top two

1. **Card 1, the mock-labeled matrix.** One small wiring change corrects three plan claims and one
   rejected spike. It also unblocks cards 4 and 5, which need a real model to measure.
2. **Card 2, the unscreened write and missing model stamp.** It is a security contract the door
   advertises and does not keep, on the verb that mutates the scene, for an S-sized fix.

Strongest reason either could be wrong: card 1 may be working as designed. `docs/plan/ai-native.md:73`
already says live vendor calls are operator-side, so the owner may rule that an offline eval is
the product and only the labels are wrong. Card 2's impact depends on a read path that echoes
untrusted component text; I could not prove one exists today, so its floor is defense-in-depth,
not a live exploit.

## One thing not to do

Do not raise `TOOL_LIST_BUDGET_CHARS` or `ASSISTANT_BUDGET_DEFAULT` again to make the numbers fit.
The comment at `crates/endpoint/src/mcp.rs:150-161` records eleven raises since 2026-10-18; each one
buys a green gate and moves the mismatch to the next client. The default list is already 3.8x the
local model's whole input limit. Fix disclosure (card 5) or examples (card 4), and let the caps stay
where they are.

## Sources

- llama.cpp `1a24df163920a4744a95373d178d790b72a9bb2f:tools/server/README.md` (OpenAI-compatible
  chat completions, verified live).
- Anthropic, "Introducing advanced tool use on the Claude Developer Platform", 2025-11-24,
  https://www.anthropic.com/news/advanced-tool-use (Tool Search Tool, Tool Use Examples).
- AgentDojo, arXiv 2406.13352. CaMeL, arXiv 2503.18813. AlphaCode, arXiv 2203.07814.
  SWT-Bench, arXiv 2406.12952.
- Vendor numbers (72 to 90 points, 85% tokens, 49 to 74 points) are Anthropic's own; they are
  reported as claims to test, not as measured repo facts.
