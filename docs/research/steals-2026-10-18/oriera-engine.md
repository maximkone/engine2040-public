STEAL SCOUT: THE AI COMMAND QUEUE AND THE PROVIDER DOOR (OriEra-engine/engine)   scout date 2026-10-18, read-only run
Repo: https://github.com/OriEra-engine/engine  sha d3c7ab3ff838c55739034ed3536ef87883f64e4c (2026-10-19T00:20:48+08:00, "feat(infer): native HTTP ModelProvider adapter - feature native-http ... (#68) (#79)")
Files read live 2026-10-18 by shallow clone; no bytes vendored.
LICENCE: NOASSERTION on the GitHub API. Read live: LICENSE is a 15-line notice ("Apache License 2.0", "Copyright 2026 OriEra Contributors", the URL and the warranty disclaimer) - it
  is the Apache-2.0 intent without the full canonical text, which is why the API refuses to assert a spdx id. Treat as Apache-2.0 with that caveat; no license check or EVIDENCE row is
  needed for porting per our rules.
What it is: an "AI-native, mixed-reality, self-evolving open world engine" laid out as 11 strict layers L0..L11 (crates/oriera-common L0, pal L1, core L2, ecs L3, asset L4, render L5,
  subsys L6, infer L7, genai L8, agent L9, platform shells). The parts that matter to us are the boundary crates: L7 `oriera-infer` is a model boundary plus a per-frame command queue,
  and it is exactly the shape of our rule 3 - provider and inference stay outside the fixed tick, and the tick receives only validated, recorded commands.

== 1. WE HAVE ==
- Provider lives outside the tick already: crates/provider/ has core.rs, policy.rs, arbiter.rs, router.rs, cassette.rs, mock.rs, pins.rs, ladder.rs, retry.rs, serving.rs; prompts and
  keys do not reach the sim.
- AI systems are on the tick: crates/ai/{director.rs,goalstack.rs,utility.rs,behavior.rs,steer.rs,tier.rs} with a blackboard and a provenance record.
- Tick and clock: crates/core/src/tick.rs, crates/clock; commands from outside arrive through the arena/input path.
- There is no single frozen *engine command vocabulary* for the AI layer, and no per-frame command budget metric surface.

== 2. STEALS (ranked by value per hour) ==
1. A frozen engine command vocabulary drained on a per-frame budget, with reject/defer/coalesce counters | crates/oriera-infer/src/lib.rs:30-123, 207-303 | Apache-2.0 (LICENCE caveat above) | SIM-adjacent (AI door; commands are recorded actions) | lands crates/provider/src/{arbiter.rs,policy.rs} and crates/ai/src/{director.rs,apply.rs} | M
   Pattern: one `Command` enum is the whole action space (Spawn, Clear, SetGravity, Teleport, Count, SetSpeed, Nudge{entity,fx,fy}); an unrecognized intent is `Unknown(String)`,
   counted by `rejected`, "never executed" (:30-52, 298-300). `InferSystem::run` resets a per-frame handle list, computes `limit = frame_budget * allocator throttle` (never below
   one) and drains exactly that many commands, bumping `deferred` by the remainder (:207-225). Metrics kept: processed, rejected, deferred, coalesced, max_pending (:100-118).
   Randomness inside the command is a fixed LCG so replays spawn identically (:197-204).
   Maps onto: our rule 3 says live inference submits validated recorded actions. This is the machinery: a closed vocabulary, a budget that cannot blow the frame, a rejection counter
   for anything outside the vocabulary, and a deterministic RNG for anything the command does. It also gives us the one number a flood tester wants (`max_pending`).
   Moves: an AI burst of 1,000 commands executes at most `frame_budget` per tick, deferring the rest; the sim never sees an unvalidated command.
   Proof: `cargo test -p engine-provider --lib infer_queue` (budget caps per tick; deferred counts survivors; Unknown never mutates world) x2; golden d57b5c8ae3096614 unchanged because
   the queue is fed by the recorded tape in the golden run.
   Risk: a deferred command makes the AI feel late; the budget must be a tuned constant, not a config nobody reads. Kill: if any command can mutate the sim without passing the
   vocabulary, or if a deferral changes the golden replay.
2. Coalesce a command only when the merge is provably world- and RNG-identical, and refuse the float cases by name | crates/oriera-infer/src/lib.rs:149-190 | Apache-2.0 (caveat) | SIM (action queue) | lands crates/provider/src/arbiter.rs, crates/net/src/delta.rs (batch send) | S
   Pattern: `submit` merges only three pairs and says why each is safe - Spawn(m)+Spawn(n) -> Spawn(m+n) draws the same LCG sequence; SetGravity(a)+SetGravity(b) -> SetGravity(b) is a
   plain overwrite; Count+Count -> Count (:155-160). It *deliberately does not* merge Teleport/SetSpeed pairs or Nudge, because float add/mul is not associative, so `(x+dx1)+dx2` can
   differ from `x+(dx1+dx2)` in the last bit, and "anything that changes observable floats is not waste removal, it is a behavior change" (:162-168).
   Maps onto: any batching we add (AI commands, net deltas, cue queues) must state its merge rule and its non-merge rule. This is the sentence to copy: a merge that can move a float
   bit is a behavior change, not an optimisation.
   Moves: a documented merge table where each merge carries a one-line proof; float-op commands are never merged.
   Proof: `cargo test -p engine-provider --lib coalesce` x2: Spawn merges, Teleport does not, and the merged stream replays byte-identical.
   Risk: a merge whose proof is wrong is a silent desync. Kill: if any merge lacks a written proof (LCG-ident, overwrite, or read-at-execution), or if a float-op merge appears
   anywhere on the sim path.
3. A provider boundary whose error type separates budget signals from bugs, with a deterministic offline backend | crates/oriera-infer/src/provider.rs:1-80 (+ RuleProvider, InferError) | Apache-2.0 (caveat) | PRESENTATION (provider) | lands crates/provider/src/{core.rs,policy.rs,mock.rs,retry.rs} | S
   Pattern: `InferError` classifies Provider / Network / Timeout / Status{code,detail} / Parse, and the doc says what each *means*: Network = never reached the far side, Timeout =
   far side alive but slow, "a budget signal, not a bug"; Parse = provider answered garbage, "never retried; surface it"; keys and transport details never leak into the prompt layer
   (:44-71). `RuleProvider` is the deterministic offline backend that "keeps tests hermetic and lets the engine run without a key" (:8-10). Model access is owned by the engine, never
   by the app (:3-7).
   Maps onto: our provider crate already has mock/cassette/pins; the refinement is the *meaning* attached to each variant, so retry policy reads the classification instead of a
   string, and a Parse is never retried while a Timeout is. Hermetic offline runs with no key are our test-env rule already; this states it as a backend, not a special case.
   Moves: retry policy switches on the variant; Timeout backs off, Parse fails fast, and no key material is reachable from prompt code.
   Proof: `cargo test -p engine-provider --lib error_class` x2; a Parse error is not retried in the fake transport.
   Risk: over-classifying a real failure as Timeout hides a bug. Kill: if any variant is decided by a string match on a message rather than by the transport layer.

== 3. NOT WORTH IT ==
- The 11-layer L0..L11 workspace and its layer-order Cargo comment: our docs/ARCHITECTURE.md already fixes the acyclic order; a second numbering would only be a name clash.
- crates/oriera-genai (tripo, seedance, cogview) and the BCI/mixed-reality platforms: asset-generation and XR shells for a product we are not building.
- oriera-pal/capability.rs and the wasm/AR platform crates: capability probing for five platforms; we have our own cfg/feature story.
- The L9 cognition loop (oriera-agent/src/cognition.rs) and its token-usage meter: an agent loop is a product decision above the engine; read the queue, not the agent.

DONE 3 steals, 5 donor files read (oriera-infer/src/{lib.rs,provider.rs,http.rs}, LICENSE, Cargo.toml).
