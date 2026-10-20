# 13 — Auto-heal as a real capability: the healed state must be provable

Lane N13 (crates/observe, crates/render, crates/proof, crates/perf, crates/playtest are busy with other writers; this file is read-only research).
All reads marked OBSERVED were done today, 2026-10-20. Row ids come from `docs/SPRINT*.md`
(`node tools/board-slice.mjs <ID>`), from `docs/plan/finish-line.md` (gap D19, gates G1 and G9),
from `docs/plan/forge-adoption.md` (AD-05, AD-14, N-11) and from `docs/FINISH-RICH.md` (gate A9).

## Answer

The plan is real and the plumbing is done. What is missing is the one thing that turns
"auto-retry" into "auto-heal": nobody proves that the healed state is the last good state.

What is already true (OBSERVED 2026-10-20):

- `crates/observe/src/heal.rs` is the MAPE-K knowledge base: 9 `HealKind` values
  (`RetryTick`, `ResetBudget`, `DropJob`, `LastGood`, `Degrade`, `Reconnect`, `AiTakeover`,
  `Restore`, `Escalate`, plus `Quarantine`), one attempt counter per fault code, and
  `applies_at_tick` (heal.rs:154) which splits sim faults (physics, brain, net desync) from
  presentation and service faults.
- `crates/observe/src/supervisor.rs` is the planner: a 12-row `catalog()` (supervisor.rs:104)
  with code, kind, attempt budget, backoff base/cap and `retry_after_ticks`; exponential
  backoff with seeded jitter (`SPLITMIX_JITTER_SEED = 2040`, supervisor.rs:225); escalation
  rows; a deferred tick-boundary queue (`MAX_PENDING_HEALS = 64`) drained by
  `apply_tick_boundary` (supervisor.rs:1212); a bounded tape (`MAX_TAPE_RECORDS = 1024`);
  quarantine with a 10-tick cool-down and exactly one half-open probe
  (`QUARANTINE_COOLDOWN_TICKS = 10`, `HALF_OPEN_MAX_CALLS = 1`, supervisor.rs:35-38);
  a `RollingBreaker` (10-call window, 50% error opens it, one half-open probe) and a
  `HedgeBudget` (latency threshold, integer extra-percent cap).
- The breaker and the hedge ARE in the retry path today: `breaker_rejects` is called in
  `plan_fault` (supervisor.rs:1143) and `hedge_admits` at supervisor.rs:1177. Board row E75's
  note "the hedge path is still not called from supervisor.rs" is stale. But
  `enable_breaker` and `enable_hedge` are called only from the supervisor's own unit tests
  (supervisor.rs:1743 and :1825), so in every shipped subsystem both guards are OFF. The
  breaker, hedge and bulkhead models still live in `supervisor.rs` while their vectors live in
  `crates/observe/tests/it/{circuit_vectors,hedge_vectors,bulkhead_vectors}.rs`, which is what
  board row AD14 asks to move into `crates/observe/src/breaker.rs`.
- `crates/render/src/heal.rs:202` (`RenderHeal`) is a complete render heal: last-good frame,
  quality tiers with a floor, backoff, quarantine with a loud notice. It has NO production
  caller: `git grep RenderHeal -- crates` hits only its own file and two doc comments in
  `crates/render/src/present/surface.rs:252,272`. The tree's own gate says so out loud:
  `node tools/unwired-code-gate.mjs` prints `unwired-module render heal render/src/heal.rs
  no-caller` and `unwired-module audio heal audio/src/heal.rs no-caller`
  (`tools/unwired-code-gate.mjs:464` lists `render::heal` among the modules the self-test
  expects to be dead). The only heal with a caller inside its own crate is `NetHeal`
  (`crates/net/src/heal.rs:177`, used by `crates/net/src/dedicated.rs:523,567`), and even that
  module has no outside caller (the gate prints `net heal no-caller`).
- The chaos suite proves detection, not recovery: `crates/proof/src/chaos.rs:78` lists the 12
  table rows, and 8 of them are `FaultKind::Quarantine { kind, ships_in }`, which only names
  the row that will ship the real recovery. `docs/FINISH-RICH.md:194` records the A9 gate as
  "healed 3, quarantined 9". So A9 passes as "nothing fails silently", not as "auto-heal works".

What is not true (the gap, all OBSERVED 2026-10-20):

1. No heal is ever verified. `HealTapeRecord` (supervisor.rs:266-281) has no before digest, no
   after digest and no validation result. `Registry::on_fault` pushes `heal.applied` the moment
   the DECISION is made (heal.rs:291-301), before the recovery has run. `Supervisor::reset`
   (supervisor.rs:758) takes no proof at all. The engine's own spec rule 1 already demands
   "every heal emits `heal.applied` with cause, action, before/after digests"
   (`docs/systems/observability-heal.md:58-59`), and rule 5 wants a typed outcome with a
   validation result (:67-69). Today a heal that does nothing is recorded as applied.
2. A policy can reopen its own budget: `register_policy` clears the quarantine entry and the
   escalation row (supervisor.rs:643-644). AD14 names this ("re-register does not reopen the
   budget").
3. A presentation heal and a state heal are the same call. `RenderHeal::try_recover` calls
   `reset_render_budgets()` (crates/render/src/heal.rs:576-580), which resets the fault budget
   because a pipeline rebuilt: proof is "the opener returned Ok", not "the state is good". A sim
   heal has no read-back at all.
4. There is no self-heal budget. `git grep -i "heal.*per minute|heals_per|heal_rate|self_heal_budget"
   -- crates tools docs` returns 0 hits. The only budgets are the per-code attempt counts (1-4)
   and the 10-tick quarantine cool-down. Nothing says "more than N heals in a minute is red".
5. There is no rollback of a wrong heal. The tree has forward motion (`reset`) and no undo. The
   only rollback machinery is the counterfactual fork: `crates/arena/src/fork.rs:259` (`Fork`),
   :287 (`Fork::from_snapshot`), `ForkTrace::first_divergence`, driven by
   `crates/endpoint/src/assistant/fork.rs:62` (`ForkProof`) and :85 (`ForkProof::replay`).
   No heal uses it.
6. G1 already punishes a presentation heal that holds the last good frame:
   `tools/product-check.mjs` counts a PNG under 60,000 bytes as blank and two byte-equal
   captures in a row as a frozen picture. A "hold last-good" heal produces byte-equal frames,
   which is literally the frozen-picture failure G1 forbids. A healed cycle must not sit in the
   history as GREEN, and it must not be allowed to reset the 7-day clock
   (`docs/plan/finish-line.md:12`).
7. The heal receipt is not a verifier output. `crates/playtest/src/lib.rs` already has
   `Verdict { failures, checks_run }` and a run with zero checks adds a Failure, but no heal
   path produces a verdict. Gap D19 ("verifier output in every build verb, so each edit returns
   its own check", `docs/plan/finish-line.md:73`) has the same shape for heals.

So the work is not "build auto-heal". It is: give the heal a proof, put the proof on the tape,
cap how often a heal may fire, and roll back a heal that is wrong.

## What to port (ranked best first)

| # | What it is | Donor + URL + date read | License | Fills (engine2040 row id) | What exactly to port (donor file path) | Cost S/M/L | Kill line | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | Verified heal receipt: every tape row carries before digest, after digest and a validation verdict; `reset` only works with a `VerifiedRecovery` proof | arXiv:2408.01055 "Towards Agentic Runtime Healing" (Healer: the preprint says plain rollback-to-checkpoint heals "struggle to adapt" and asks for safety checks on generated heals) https://arxiv.org/abs/2408.01055 + our own spec rule 1 `docs/systems/observability-heal.md:58-59`, read 2026-10-20; Crossref DOI 10.1145/3800691 (CACM 2026, same authors) confirmed 2026-10-20 | ideas-only (no donor bytes); paper is free to read | AD14, A9, D19, AD05 | No donor bytes. Write our own `VerifiedRecovery { before_digest, after_digest, validation }` on `HealTapeRecord` and a `reset(code, VerifiedRecovery)` in `crates/observe/src/supervisor.rs` | M | If a chaos row whose heal "applies" but whose read-back still faults does not end in quarantine, the row dies | PORT |
| 2 | The healed state must be digest-equal to the last good state, not merely alive | Envoy outlier detection: consecutive 5xx eject the bad host, the good hosts keep serving, and the ejection is counted https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/outlier (200 today) + our own `crates/arena/src/fork.rs:287` `Fork::from_snapshot` and `engine_core::hash::Fnv64`, read 2026-10-20 | Apache-2.0 (envoyproxy/envoy, GitHub API 2026-10-20) | AD14, A9, G1, G9 | The "bad replica is ejected and counted, the last good keeps serving" shape. Our code: after a heal, re-read the healed unit from the last-good snapshot and compare the state digest with the last-good digest; equal = recovered, different = not recovered | M | If the digest of the healed state cannot be compared byte-for-byte on the same inputs, the row dies | PORT |
| 3 | Rolling breaker that is actually on: sliding window, failure-rate threshold, permitted half-open calls, moved into `crates/observe/src/breaker.rs` | resilience4j `CircuitBreaker.java` (five states CLOSED/OPEN/HALF_OPEN/DISABLED/FORCED_OPEN, test calls allowed in half-open, then `CallNotPermittedException`) and `CircuitBreakerConfig.java` (`failureRateThreshold`, `permittedNumberOfCallsInHalfOpenState`, `slidingWindowSize`, `slidingWindowType` COUNT_BASED) https://github.com/resilience4j/resilience4j , files under `resilience4j-circuitbreaker/src/main/java/io/github/resilience4j/circuitbreaker/`, read 2026-10-20 | Apache-2.0 (GitHub API 2026-10-20) | E75, AD14, OPT22 | The window + threshold + half-open-permit shape. Our code: move `RollingBreaker` out of `supervisor.rs` into `crates/observe/src/breaker.rs`, turn it on per catalog row (today `enable_breaker` is called only from supervisor tests), and point `crates/observe/tests/it/circuit_vectors.rs` at the src module | S | If turning the breaker on changes the golden hash `d57b5c8ae3096614`, the row dies | PORT |
| 4 | Hedge path with a real delay: the backup runs after a delay while the primary is still running, capped by max hedged attempts | Polly `src/Polly.Core/Hedging/HedgingResilienceStrategy.cs`: `HedgingDelay`, `maxHedgedAttempts`, `TotalAttempts = maxHedgedAttempts + 1`, `DelayGenerator` https://github.com/App-vNext/Polly , read 2026-10-20 | BSD-3-Clause (GitHub API 2026-10-20) | E75, AD14 | The delay + attempt-cap shape. Our code: `HedgeBudget` today only zeroes the backoff (supervisor.rs:1176-1187); give it a delay in ticks and a maximum number of parallel backups, and record both on the tape row | S | If hedging pushes a frame past the G9 budget, the row dies | PORT |
| 5 | A self-heal budget: heals per rolling minute are capped, and over the cap the run is RED, not degraded | Kubernetes liveness probes: the deadline is `failureThreshold x periodSeconds`, and a liveness failure restarts the container https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/ (200 today, text read 2026-10-20); plus Envoy outlier ejection for the consecutive-error counter | Apache-2.0 (kubernetes/kubernetes, GitHub API 2026-10-20) | A9, G1, G9, E46, E59 | The threshold-times-window shape. Our code: integer heal counters per tick window in `crates/observe/src/supervisor.rs` (tick as clock, never wall time); `tools/product-check.mjs` writes a heal row in its history so a healed cycle is not GREEN; a `tools/heal-evidence.mjs` leg prints the heal rows and fails on an unproven heal (E59 family) | S-M | If the bar cannot be measured from real run data in one day of keeper runs, the row dies | PORT |
| 6 | Presentation heal and state heal are different things and get different receipts | Kubernetes probe semantics: liveness restarts the state, readiness only takes the instance out of rotation and leaves the state alone https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/ (200 today) | Apache-2.0 | A9, E46, AD14 | The liveness/readiness split. Our code: split the tape row into `heal.presentation` and `heal.state`; a presentation heal may not touch the sim hash and may not reset a state budget; a state heal must land in the tape or the journal or it does not run (spec rule 3, `docs/systems/observability-heal.md:62-64`) | S | If a presentation heal changes the golden hash, the row dies | PORT |
| 7 | `RenderHeal` gets a production caller, and the unwired-code gate keeps it wired | Netflix Chaos Monkey: "a resiliency tool that helps applications tolerate random instance failures" https://github.com/Netflix/chaosmonkey (GitHub API 200 today, Apache-2.0, default branch `master`); our own gate `node tools/unwired-code-gate.mjs` already prints `unwired-module render heal render/src/heal.rs no-caller` (read 2026-10-20) | Apache-2.0 | A9, E46, AD05, D19 | The practice, not bytes: inject a real surface fault into the render path instead of only the simulated `SimulatedSurface` test. Our code: wire `crates/render/src/heal.rs` into `crates/render/src/present` (the presenter can already flag device loss, `crates/render/src/present/gpu_present/presenter.rs:1550`, and its only caller today is a test, :299) and keep the gate line from ever printing again | M | If the render heal cannot be exercised headless in the chaos suite (no GPU on the runner), the row dies | PORT |
| 8 | The chaos suite becomes the heal gate: every fault row needs a real recovery and a steady-state check | Chaos Engineering (principles paper) arXiv:1702.05843 https://arxiv.org/abs/1702.05843 ; Chaos Engineering in the Wild arXiv:2505.13654 https://arxiv.org/abs/2505.13654 ; chaostoolkit https://github.com/chaostoolkit/chaostoolkit (Apache-2.0, GitHub API 2026-10-20); Netflix/chaosmonkey (Apache-2.0). All read 2026-10-20 | Apache-2.0 / paper free to read | A9, E46, E59, AD05 | The steady-state discipline, not bytes. Our code: `crates/proof/src/chaos.rs` gets three lists (healed and proved / detection proven, recovery missing / loud) and `crates/proof/src/a9.rs` reports the second list as open work instead of hiding it inside "quarantined 9" | M | If the 8 quarantine-only rows cannot get a real recovery on this PC, the row dies | PORT |
| 9 | A wrong heal is rolled back, and the rollback is provable | Our own counterfactual fork: `crates/arena/src/fork.rs:259` `Fork`, :287 `Fork::from_snapshot`, `ForkTrace::first_divergence`; `crates/endpoint/src/assistant/fork.rs:62` `ForkProof`, :85 `ForkProof::replay` (a proof that replays and must match exactly); plus Envoy outlier ejection for "drop the bad one" | Apache-2.0 (envoy) / own code | AD14, A9, G1, D19 | The replay-must-match-exactly shape. Our code: when a heal is proved wrong, restore the last-good snapshot through the fork path, then record a rollback row whose restored digest equals the last-good digest | M | If the rollback cannot restore the exact last-good digest, the row dies | PORT |
| 10 | Rust bug-fix loop: issue, patch, run the repo's own oracle, accept or reject | Multi-SWE-bench (Java, TypeScript, JavaScript, Go, **Rust**, C, C++, 1,632 instances) arXiv:2504.02605 https://arxiv.org/abs/2504.02605 ; SWE-agent https://github.com/SWE-agent/SWE-agent (MIT, GitHub API 2026-10-20); SWE-bench https://github.com/SWE-bench/SWE-bench (MIT, GitHub API 2026-10-20). All read 2026-10-20 | Apache-2.0 / MIT | OPT22, D19, E46 | The evaluation shape only, no model in the tree. Our code: `crates/playtest` already has `attribute_seeded`, `bisect_seeded`, `verify_seeded_clean` (`crates/playtest/src/seeded_bug.rs`); add the accept/reject loop on top of them, with the crate's own tests as the oracle | M | If the loop cannot run without a paid model, the row dies | PORT |
| 11 | Property tests for the heal invariants, with shrinking | proptest https://github.com/proptest-rs/proptest (Apache-2.0, GitHub API 2026-10-20), read 2026-10-20 | Apache-2.0 | OPT22, AD14, E75 | The shrink-and-report shape. Our invariants: a heal never reopens its own budget; re-register keeps the budget; the tape is identical twice for one fault sequence; quarantine stays closed for the cool-down and admits exactly one half-open probe; a hedge never exceeds its extra-percent cap; the breaker opens at 50% of a full 10-call window | S | If proptest cannot be added as a dev dependency under `deny.toml`, write the vectors by hand and kill the row | PORT |
| 12 | Verifier output in every build verb and in every heal receipt (gap D19) | SWE-agent https://github.com/SWE-agent/SWE-agent (MIT, read 2026-10-20): a patch only counts once the repo's own tests validate it; our own `crates/playtest/src/lib.rs` `Verdict { failures, checks_run }` already treats a zero-check run as a Failure | MIT | D19, A9, AD14, E59 | The "no check is not a pass" rule. Our code: a heal receipt with no read-back check is a `Failure`, not a pass, and the door verbs that trigger a heal return the same verdict shape | S | If the door verbs cannot carry a verdict row without breaking the door contract, the row dies | PORT |

## What auto-heal must prove (10 lines)

1. A heal runs only inside its attempt budget; past it the object quarantines loudly, never a silent retry.
2. Every heal is typed: presentation heal (no sim or scene state) or state heal (state changes).
3. A state heal applies only at a tick boundary, from the same inputs on every machine, and lands in the heal tape.
4. The healed state must be digest-equal to the last good state, not merely alive: equal digest = recovered, different digest = not recovered.
5. Every tape row carries code, fault tick, applied tick, kind, attempt, backoff, before digest, after digest and the validation verdict.
6. `reset(code)` only works with a `VerifiedRecovery` proof, and re-registering a policy keeps the budget.
7. A heal that applies but cannot prove itself counts as a failed attempt and ends in quarantine.
8. A heal that is wrong is rolled back to the last-good snapshot, and the rollback is proven by the restored digest.
9. Heals per rolling minute are capped; over the cap the run is red (G1), not degraded.
10. A presentation heal that holds the last-good frame is a frozen frame: it never counts as green.

## Rejections

- `mizcausevic-dev/reliability-toolkit-rs` (MIT) — already mined: the quarantine, cool-down and half-open donor note is already written into `crates/observe/src/supervisor.rs:33`. Not a new find.
- `Netflix/chaosmonkey` source code — RESEARCH-ONLY: it terminates AWS instances through a Go control service; only the practice (inject a real fault, check the steady state) is worth porting, no file.
- `chaostoolkit`, Chaos Mesh, LitmusChaos, Keploy — container and Kubernetes shaped; nothing fits a fixed game tick loop.
- CARDAMO — no arXiv record today (arXiv API `all:CARDAMO` returned 0 entries) and both `github.com/inesc-pt/carda` and `github.com/inesc-pt/cardamo` return 404 today; only third-party lists mention it, so there is nothing live to read and nothing to cite.
- CIRCA — the project page `https://utah-floss.github.io/circa/` returns 404 today and Crossref has no record of a "CIRCA debugging tool" title; the live readable relative is the same group's preprint arXiv:2408.01055 and the CACM version DOI 10.1145/3800691 (cacm.acm.org answers 403 to bots today). Cited through the arXiv page instead.
- AWS blue/green deployment doc (`docs.aws.amazon.com/elasticbeanstalk/.../environments-bluegreen.html`) — answered 200 but a 2.4 KB redirect stub with no readable body today; not used as a donor.
- Envoy request mirroring doc — the path returns 404 today; only the outlier-detection page is live, so only that one is cited.
- VIGIL (arXiv:2512.07094) — a reflective runtime with an emotion bank; no fit for a deterministic game tick.
- mRUBiS (arXiv:1804.00954) — model-based architectural self-healing for OSGi services; it wants a full runtime model, far heavier than a 12-row catalog.
- "Self-Healing by Means of Runtime Execution Profiling" (arXiv:1203.5748) — matches faults against a stored profile library we do not have; the survey arXiv:2403.00455 is the better read.
- resilience4j bulkhead `ThreadPoolBulkhead.java` — file read 200, but it holds no `maxConcurrentCalls` string; the path is cited, no number is claimed.
- `ggrs` rollback, Bevy, Kani, promptfoo, ollama — already mined per `docs/research/powerups-2026-10-20/RULES.md`.
- `crates/observe/src/breaker.rs` — does not exist today; the breaker and hedge models still sit inside `supervisor.rs` while their vectors sit in `crates/observe/tests/it/`, which is exactly what board row AD14 asks to change.

## Top 3

1. resilience4j circuit breaker — https://github.com/resilience4j/resilience4j (files `resilience4j-circuitbreaker/src/main/java/io/github/resilience4j/circuitbreaker/CircuitBreaker.java` and `CircuitBreakerConfig.java`, Apache-2.0, read 2026-10-20) — closes E75 and AD14 — cost S.
2. Envoy outlier detection (eject the bad one, count it, keep the last good) — https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/outlier (Apache-2.0, read 2026-10-20) — closes AD14, A9 and G1 (digest-equal read-back, rollback of a wrong heal) — cost M.
3. Polly hedging (`src/Polly.Core/Hedging/HedgingResilienceStrategy.cs`) — https://github.com/App-vNext/Polly (BSD-3-Clause, read 2026-10-20) — closes E75 and AD14 (a hedge with a delay and a cap, on by default) — cost S.
