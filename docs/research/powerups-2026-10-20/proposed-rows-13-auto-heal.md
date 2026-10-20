# Proposed rows 13 — auto-heal as a real capability

Lane N13. Read-only research plus the `crates/observe` implementation.
All rows were read on 2026-10-20. Board ids come from `docs/SPRINT*.md`
(`node tools/board-slice.mjs <ID>`), `docs/plan/forge-adoption.md` (AD-05, AD-14, N-11),
`docs/plan/finish-line.md` (gap D19, gates G1 and G9) and `docs/FINISH-RICH.md` (gate A9).
The lead merges the board: this file edits nothing else.

## SPEC — what auto-heal must prove

These ten lines are the contract every row below implements. One row per line, so nothing is lost.

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

## Rows

### HLX-01 — A heal only runs inside its attempt budget
A heal is bounded: the per-code attempt budget in the shipped catalog (1 to 4) decides how many
tries a fault gets. Past the budget the object quarantines loudly: a `heal.failed` record, a
quarantine tape row and a cool-down, never a silent retry.
**Donor:** our own spec, `docs/systems/observability-heal.md` rule 2 ("bounded: each fault has a
retry budget; after it, quarantine the object or task and fail loudly"),
https://github.com/maximkone/engine2040/blob/master/docs/systems/observability-heal.md, read
2026-10-20. **Shape, not code:** we port the rule into Rust, not donor bytes (there are none).
**Closes:** AD14, A9. **Owned paths:** `crates/observe/src/supervisor.rs`,
`crates/observe/src/heal.rs`. **Status:** landed this pass (the budget was already there; the
proof that a heal may not reopen its own budget is HLX-06).
**Accept:** `node tools/gh-check.mjs cargo test -p engine-observe --lib` →
`test result: ok. 95 passed; 0 failed`. **Kill line:** if a fault past its budget can be healed
again without a loud quarantine row, the row dies. **Cost:** S.

### HLX-02 — Every heal is typed: presentation heal or state heal
`HealScope` splits the catalog in two: a presentation heal never touches simulation or scene
state (render, audio, assets, provider, commands, editor, save) and a state heal does (physics,
brain, net desync), so a state heal waits for a tick boundary and a presentation heal may land
at once.
**Donor:** Kubernetes probes, "Liveness, Readiness, and Startup Probes",
https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/ (Apache-2.0,
`kubernetes/kubernetes`, GitHub API 2026-10-20), read 2026-10-20. **Shape, not code:** the
liveness/readiness split is ported into Rust as `HealScope`; we vendor no bytes.
**Closes:** AD14, E46, N-11. **Owned paths:** `crates/observe/src/recovery.rs`,
`crates/observe/src/supervisor.rs`. **Status:** landed this pass.
**Accept:** `node tools/gh-check.mjs cargo test -p engine-observe --lib` →
`test result: ok. 95 passed; 0 failed`. **Kill line:** if a state heal can be applied off a tick
boundary, the row dies. **Cost:** S.

### HLX-03 — A state heal lands at a tick boundary and in the tape
The state rows (physics non-finite, fighter stuck, client desync) are deferred to the next tick
boundary and applied there from the same inputs on every machine; the applied heal is stamped on
its tape row so replay reproduces the same checkpoints.
**Donor:** our own spec rules 3 and 6,
`docs/systems/observability-heal.md:62-64` and `:70-73`,
https://github.com/maximkone/engine2040/blob/master/docs/systems/observability-heal.md, read
2026-10-20. **Shape, not code:** ported into Rust (`FaultPolicy::scope` drives the boundary).
**Closes:** AD14, A9. **Owned paths:** `crates/observe/src/supervisor.rs`. **Status:** the queue
and the tape stamp were already there; the scope now drives the boundary. Landed this pass.
**Accept:** `node tools/gh-check.mjs cargo test -p engine-observe --lib` →
`test result: ok. 95 passed; 0 failed`. **Kill line:** if a state heal is applied inside a tick,
the row dies. **Cost:** S.

### HLX-04 — Digest equality is the recovery proof
The healed state must hash to the last good state. Equal digest means recovered; a different
digest means not recovered, and the heal counts as a failed attempt. The supervisor owns the
check; the caller only reports the two digests.
**Donor:** Envoy outlier detection ("consecutive 5xx eject the bad host, the good hosts keep
serving, the ejection is counted"),
https://www.envoyproxy.io/docs/envoy/latest/intro/arch_overview/upstream/outlier (Apache-2.0,
`envoyproxy/envoy`, GitHub API 2026-10-20), read 2026-10-20. **Shape, not code:** the
"drop the bad one, keep the last good, count it" shape is ported into Rust as
`HealObservation::State { before_digest, after_digest }`; no donor bytes.
**Closes:** AD14, A9, G1. **Owned paths:** `crates/observe/src/recovery.rs`,
`crates/observe/src/supervisor.rs`. **Status:** landed this pass.
**Accept:** `node tools/gh-check.mjs cargo test -p engine-observe --lib heal_that_` →
`test result: ok. 2 passed; 0 failed`. **Kill line:** if a heal with a changed digest can still
be recorded as recovered, the row dies. **Cost:** M.

### HLX-05 — The heal tape row carries the proof
`HealTapeRecord` carries code, fault tick, applied tick, kind, attempt, backoff, scope, before
digest, after digest and the validation verdict. A row without digests is a row that says a heal
ran, not that it worked.
**Donor:** our own spec rule 1 and rule 5 (`docs/systems/observability-heal.md:58-59` and
`:67-69`), https://github.com/maximkone/engine2040/blob/master/docs/systems/observability-heal.md,
read 2026-10-20, and board row AD14 ("heal tape rows carry before and after digests",
`node tools/board-slice.mjs AD14`). **Shape, not code:** ported into Rust as four new fields on
`HealTapeRecord`; no donor bytes.
**Closes:** AD14, N-11, PF16. **Owned paths:** `crates/observe/src/supervisor.rs`. **Status:**
landed this pass. **Accept:** `node tools/gh-check.mjs cargo test -p engine-observe --lib` →
`test result: ok. 95 passed; 0 failed`. **Kill line:** if a tape row can be written without the
scope and the verdict, the row dies. **Cost:** M.

### HLX-06 — A budget reset needs a proof, and a new policy row keeps the budget
`Supervisor::reset(code)` is refused unless a verified recovery or a proved rollback is on file
for that code; the refusal is loud (a `heal.failed` event in the supervisor's own ring) and the
budget stays spent. `register_policy` keeps the attempt budget, the quarantine and the
escalation: new knowledge may change the policy, never the budget.
**Donor:** resilience4j circuit breaker, `CircuitBreaker.java` and `CircuitBreakerConfig.java`,
https://github.com/resilience4j/resilience4j (Apache-2.0, GitHub API 2026-10-20), read
2026-10-20. **Shape, not code:** the "a closed circuit only reopens on a proved recovery" shape
is ported into Rust as `VerifiedRecovery` plus `has_proof`; we vendor no Java bytes.
**Closes:** AD14, AD05. **Owned paths:** `crates/observe/src/supervisor.rs`,
`crates/observe/src/heal.rs`. **Status:** landed this pass.
**Accept:** `node tools/gh-check.mjs cargo test -p engine-observe --lib reset_without_the_proof_is_refused` →
`test result: ok. 1 passed; 0 failed`. **Kill line:** if a policy re-register hands back a fresh
budget, the row dies. **Cost:** S.

### HLX-07 — A heal that cannot prove itself is a failed attempt
A heal that ran but did not restore the last good digest spends an attempt and, when the budget
is gone, quarantines the code loudly. It never counts as recovered.
**Donor:** Polly hedging and retry shapes, `src/Polly.Core/Hedging/HedgingResilienceStrategy.cs`
and `src/Polly.Core/Retry`, https://github.com/App-vNext/Polly (BSD-3-Clause, GitHub API
2026-10-20), read 2026-10-20. **Shape, not code:** the "a failed attempt is an attempt" shape is
ported into Rust as `Registry::burn_attempt`; no C# bytes.
**Closes:** AD14, A9. **Owned paths:** `crates/observe/src/supervisor.rs`,
`crates/observe/src/heal.rs`. **Status:** landed this pass.
**Accept:** `node tools/gh-check.mjs cargo test -p engine-observe --lib heal_that_changes_the_digest_fails_and_quarantines` →
`test result: ok. 1 passed; 0 failed`. **Kill line:** if an unproven heal still leaves the code
live, the row dies. **Cost:** M.

### HLX-08 — A wrong heal is rolled back, and the rollback is proven
When the healed state is wrong, the engine restores the last-good snapshot and proves it: the
restored digest must equal the last-good digest. Equal digests clear the budget and put the run
back to green; different digests quarantine the code loudly.
**Donor:** our own counterfactual fork (the idea is already in the tree):
`crates/arena/src/fork.rs` (`Fork::from_snapshot`, `ForkTrace::first_divergence`) and
`crates/endpoint/src/assistant/fork.rs` (`ForkProof::replay`), read 2026-10-20, plus the Envoy
outlier page above. **Shape, not code:** the "replay the proof and it must match exactly" shape
is ported into Rust as `RollbackProof`; we write our own code.
**Closes:** AD14, G1. **Owned paths:** `crates/observe/src/recovery.rs`,
`crates/observe/src/supervisor.rs`. **Status:** landed this pass (the observe side; the sim side
that restores the snapshot is HLX-09's follow-up).
**Accept:** `node tools/gh-check.mjs cargo test -p engine-observe --lib rollback_` →
`test result: ok. 3 passed; 0 failed`. **Kill line:** if a rollback can be recorded without the
restored digest matching, the row dies. **Cost:** M.

### HLX-09 — A self-heal budget: over the cap the run is red
Heals are counted in a rolling window of one minute of ticks (3,600 ticks at the engine's 60 Hz
fixed tick). Past the cap the run reads red, never degraded, and the cap is loud.
**Donor:** Kubernetes liveness probes, where the deadline is `failureThreshold x periodSeconds`
before the restart, https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/
(Apache-2.0, read 2026-10-20), plus the Envoy outlier ejection counter above. **Shape, not code:**
the "threshold times window" shape is ported into Rust as `HealCap`; no donor bytes.
**Closes:** G1, A9, AD05, E46. **Owned paths:** `crates/observe/src/recovery.rs`,
`crates/observe/src/supervisor.rs`. **Status:** landed this pass; the default cap is one heal per
catalog row per minute (`default_heal_cap_max()` reads the catalog length, so the number is
measured, not guessed).
**Accept:** `node tools/gh-check.mjs cargo test -p engine-observe --lib over_the_heal_cap_the_run_reads_red` →
`test result: ok. 1 passed; 0 failed`. **Kill line:** if the cap cannot be measured from a real
run in one day of keeper runs, the row dies. **Cost:** S-M.

### HLX-10 — A presentation heal that only holds the last-good frame is a frozen frame
Holding the last-good frame is not a recovery: two byte-equal captures in a row are the G1
frozen-picture failure. Such a heal is marked `FrozenFrame`, never counts as green, and the run
reads red.
**Donor:** our own gate `tools/product-check.mjs` (a PNG under 60,000 bytes is blank, two
byte-equal frames in a row are frozen), read 2026-10-20, and gate G1 in
`docs/plan/finish-line.md:12`. **Shape, not code:** the rule is ported into Rust as
`HealValidation::FrozenFrame` and `HealObservation::HeldLastGood`; no donor bytes.
**Closes:** G1, E46, AD05. **Owned paths:** `crates/observe/src/recovery.rs`,
`crates/observe/src/supervisor.rs`. **Status:** landed this pass.
**Accept:** `node tools/gh-check.mjs cargo test -p engine-observe --lib presentation_heal_that_holds_the_last_good_frame_is_frozen_and_never_green` →
`test result: ok. 1 passed; 0 failed`. **Kill line:** if a held frame can be recorded as
recovered, the row dies. **Cost:** S.

### HLX-11 — The rolling breaker moves into `observe/src/breaker.rs` and turns on
The breaker already exists inside `supervisor.rs` and is called in the retry path, but
`enable_breaker` runs only in the supervisor's own tests, so the retry-storm guard never runs in
a shipped subsystem. The model moves into `crates/observe/src/breaker.rs`, the catalog rows turn
it on, and the vectors run against the src module.
**Donor:** resilience4j `CircuitBreaker.java` (five states, permitted test calls in half-open,
`CallNotPermittedException` when they run out) and `CircuitBreakerConfig.java`
(`failureRateThreshold`, `permittedNumberOfCallsInHalfOpenState`, `slidingWindowSize`,
`slidingWindowType` COUNT_BASED),
https://github.com/resilience4j/resilience4j (Apache-2.0, GitHub API 2026-10-20), read
2026-10-20. **Shape, not code:** the state machine and the window shape are ported into Rust;
we vendor no Java bytes.
**Closes:** E75, AD14. **Owned paths:** `crates/observe/src/breaker.rs` (new),
`crates/observe/src/supervisor.rs`, `crates/observe/tests/it/circuit_vectors.rs`. **Status:** open
(the retry path is wired; the file move and the per-row switch-on are not).
**Accept:** `node tools/gh-check.mjs cargo test -p engine-observe --lib breaker` →
`test result: ok. 3 passed; 0 failed`. **Kill line:** if switching the breaker on for every
catalog row changes the golden hash, the row dies. **Cost:** S.

### HLX-12 — The hedge path gets a delay and a cap
`HedgeBudget` today only zeroes the backoff of a late retry. It needs the delay before the
backup runs and the maximum number of parallel backups, both on the tape row.
**Donor:** Polly `src/Polly.Core/Hedging/HedgingResilienceStrategy.cs` (`HedgingDelay`,
`maxHedgedAttempts`, `TotalAttempts = maxHedgedAttempts + 1`, `DelayGenerator`),
https://github.com/App-vNext/Polly (BSD-3-Clause, GitHub API 2026-10-20), read 2026-10-20.
**Shape, not code:** the delay-plus-cap shape is ported into Rust; we vendor no C# bytes.
**Closes:** E75, AD14. **Owned paths:** `crates/observe/src/supervisor.rs`,
`crates/observe/tests/it/hedge_vectors.rs`. **Status:** open.
**Accept:** `node tools/gh-check.mjs cargo test -p engine-observe --lib hedge` →
`test result: ok. 3 passed; 0 failed`. **Kill line:** if the hedge pushes a frame past the G9
budget, the row dies. **Cost:** S.

### HLX-13 — The chaos suite becomes the heal gate
`crates/proof/src/chaos.rs` has 12 fault rows; 8 of them only prove detection and quarantine.
Every row needs a real recovery and a steady-state check, and a row that still only quarantines
is reported as "detection proven, recovery missing" instead of hiding inside the count.
**Donor:** Chaos Engineering (principles) https://arxiv.org/abs/1702.05843 ; Chaos Engineering in
the Wild https://arxiv.org/abs/2505.13654 ; chaostoolkit
https://github.com/chaostoolkit/chaostoolkit (Apache-2.0, GitHub API 2026-10-20) ; Netflix
Chaos Monkey https://github.com/Netflix/chaosmonkey (Apache-2.0, GitHub API 2026-10-20), all
read 2026-10-20. **Shape, not code:** the steady-state discipline is ported into Rust as the
chaos report's third list; no donor bytes and no donor code.
**Closes:** A9, E46, AD05. **Owned paths:** `crates/proof/src/chaos.rs`, `crates/proof/src/a9.rs`
(held by another lane today). **Status:** open.
**Accept:** `node tools/gh-check.mjs cargo test -p engine-proof --lib a9` →
`test result: ok. 15 passed; 0 failed`. **Kill line:** if the 8 quarantine-only rows cannot get a
real recovery on this PC, the row dies. **Cost:** M.

### HLX-14 — The render heal gets a production caller and a wiring gate
`RenderHeal` (crates/render/src/heal.rs, about 1,090 lines) has no production caller: the tree's
own gate prints `unwired-module render heal render/src/heal.rs no-caller`. It must be wired into
the presenter and typed as a presentation heal, so a held last-good frame is a frozen frame and
never a green run.
**Donor:** Netflix Chaos Monkey, "a resiliency tool that helps applications tolerate random
instance failures", https://github.com/Netflix/chaosmonkey (Apache-2.0, GitHub API 2026-10-20),
read 2026-10-20. **Shape, not code:** the practice (inject the real fault, check the steady
state) is ported into Rust as the render wiring and its test; no donor bytes.
**Closes:** A9, E46, AD05, N-11. **Owned paths:** `crates/render/src/heal.rs`,
`crates/render/src/present/gpu_present/presenter.rs` (held by GS44 today). **Status:** open, see
the spec below.
**Accept:** `node tools/gh-check.mjs cargo test -p engine-render --lib render_heal_is_wired_to_the_presenter` →
`test result: ok. 1 passed; 0 failed`. **Kill line:** if the render heal cannot be exercised
headless in the chaos suite, the row dies. **Cost:** M.

### HLX-15 — The audio heal reports into the mixer as a presentation heal
`DeviceHeal` (crates/audio/src/heal.rs, 749 lines) is called only from a `#[cfg(test)]` module.
The mixer path must report the device fault into it and must type it as a presentation heal: a
silent mixer that keeps rendering is a held frame, not a recovery.
**Donor:** Kubernetes probes (readiness takes the instance out of rotation without touching the
state; liveness restarts it),
https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/ (Apache-2.0,
read 2026-10-20). **Shape, not code:** the split is ported into Rust as the mixer reporting a
presentation heal; no donor bytes.
**Closes:** E46, N-11, A9. **Owned paths:** `crates/audio/src/heal.rs`,
`crates/audio/src/mixer.rs` (held by the smart lane today). **Status:** open, see the spec below.
**Accept:** `node tools/gh-check.mjs cargo test -p engine-audio --lib mixer_reports_the_device_fault_into_the_heal` →
`test result: ok. 1 passed; 0 failed`. **Kill line:** if the mixer cannot report a device fault
without blocking the audio thread, the row dies. **Cost:** M.

### HLX-16 — An asset heal proves its digest (the `import_repair` hole)
`assets/import_repair` is landed with no outside caller, and the assets rows heal by keeping the
last good bytes. The repaired asset must hash to the same digest as the last good asset, or the
heal is not a recovery. This is the same rule AD15 asks for at cook time: the fix is measured,
not asserted.
**Donor:** board row AD15 ("Cook-time clip gates: measured foot-skate, retarget coverage, prefix
strip", `node tools/board-slice.mjs AD15`) and the Envoy outlier page above. **Shape, not code:**
the digest proof is ported into Rust as the assets heal proof; no donor bytes.
**Closes:** AD15, AD05. **Owned paths:** `crates/assets/src/import_repair.rs`,
`crates/observe/src/supervisor.rs`. **Status:** open.
**Accept:** `node tools/gh-check.mjs cargo test -p engine-assets --lib import_repair` →
`test result: ok. 1 passed; 0 failed`. **Kill line:** if a repaired asset can land with a
different digest and still be called recovered, the row dies. **Cost:** S-M.

### HLX-17 — Property tests for the heal invariants
The heal invariants get shrinking property tests instead of hand-written vectors: a heal never
reopens its own budget, re-register keeps the budget, the tape is identical twice for one fault
sequence, quarantine stays closed for the cool-down and admits exactly one half-open probe, a
hedge never exceeds its extra-percent cap, and the breaker opens at 50% of a full 10-call window.
**Donor:** proptest, https://github.com/proptest-rs/proptest (Apache-2.0, GitHub API 2026-10-20),
read 2026-10-20. **Shape, not code:** proptest is already a Rust library; we add the invariants,
not the code.
**Closes:** AD14, E75. **Owned paths:** `crates/observe/tests/properties/` or a new
`crates/observe/tests/it/heal_properties.rs`. **Status:** open.
**Accept:** `node tools/gh-check.mjs cargo test -p engine-observe --test it heal_properties` →
`test result: ok. 1 passed; 0 failed`. **Kill line:** if proptest cannot be added as a dev
dependency under `deny.toml`, the vectors are written by hand and this row dies. **Cost:** S.

### HLX-18 — Keep `rerun_sink` and let it carry the heal evidence
**Decision: keep it, and give it a job.** The compaction lane wants `crates/observe/src/rerun_sink.rs`
deleted as dead code (321 lines, test-only today). But it is the only bounded, overflow-counted
record of (tick, state hash) in the engine, and the heal proof needs exactly that: a last-good
digest per tick and the first divergent tick after a bad heal. Deleting it would remove the
evidence channel the heal rows write into. So: keep it, add the heal tape to it, and let the
crash path (crates/game) push the ring.
**Donor:** our own tree: `crates/observe/src/rerun_sink.rs` (`RerunSink::push`,
`first_divergent`, `overflow_count`) and `crates/game/src/crash.rs` (the crash flight recorder
that writes the ring), read 2026-10-20; board rows N-11 ("One door-owned ring for net/render/audio
heals; wire `RenderHeal`/`DeviceHeal`; `RerunSink` in the arena run") and PF16 (the consumer of
the rerun sink). **Shape, not code:** our own code, no donor bytes.
**Closes:** N-11, PF16, AD14. **Owned paths:** `crates/observe/src/rerun_sink.rs` (mine),
`crates/game/src/crash.rs` (WEB6 lane, spec only). **Status:** the observe half landed this pass
(`RerunSink::push_heal`, `RerunSink::heal_rows`, `RerunSink::heal_len`,
`RerunSink::first_unproved_heal`, 32 heal rows inside the same overflow counter); the crash
caller is the WEB6 lane's, see Spec C.
**Accept:** `node tools/gh-check.mjs cargo test -p engine-observe --lib rerun_sink` →
`test result: ok. 10 passed; 0 failed`. **Kill line:** if the heal rows cannot ride the sink
without growing it past its 120-frame budget, the row dies and the sink is parked instead.
**Cost:** S.

## Specs for the lanes that hold other crates (no code here)

### Spec A — `crates/render/src/heal.rs`: a presentation heal for the renderer (held by GS44)

**Call site:** `crates/render/src/heal.rs:444` `RenderHeal::try_recover` (the recovery path) and
`crates/render/src/heal.rs:576` `RenderHeal::reset_render_budgets` (the budget clear).

**What breaks today (measured 2026-10-20 on the runner):**
`node tools/gh-check.mjs cargo test -p engine-render --lib heal` →
`test result: FAILED. 10 passed; 2 failed`, because `Supervisor::reset` is now refused without
a proof: `heal::tests::forced_loss_script_recovers_with_last_good` (crates/render/src/heal.rs:957,
`attempts` 1, wanted 0) and `heal::tests::verified_recovery_restores_full_quality_and_resets`
(crates/render/src/heal.rs:997, `attempts` 2, wanted 0). GS44 owns that file; this is the fix
list, not code from this lane.

**Function names to call:**
1. `engine_observe::recovery::HealObservation::Presented { digest }` when the rebuilt pipeline
   presents a fresh frame, and `HealObservation::HeldLastGood { frames, digest }` when the heal
   only holds the last good frame.
2. `engine_observe::supervisor::Supervisor::verify_heal(code, attempt, &observation, &mut ring)`,
   which returns the `HealValidation` (`Recovered`, `FrozenFrame`, `NotRecovered`).
3. `engine_observe::supervisor::Supervisor::reset_verified(&VerifiedRecovery::new(code, attempt,
   validation, before_digest, after_digest))` instead of the bare `reset(code)` in
   `reset_render_budgets`, because `reset` is now refused without a proof on file. The quick
   bridge, if the render heal has no digest yet, is
   `engine_observe::supervisor::Supervisor::reset_unverified(code)`, which keeps today's behavior
   and stays loud (it writes a `heal.failed` event into `supervisor.events()`).
4. `engine_observe::supervisor::Supervisor::run_health()` for the render notice: `RunHealth::Red`
   must read as "the renderer is holding a frozen frame", never as healthy.

**Test names to add in `crates/render/src/heal.rs`:**
`render_heal_that_presents_a_fresh_frame_is_recovered`,
`render_heal_that_holds_the_last_good_frame_is_a_frozen_frame`,
`render_heal_clears_its_budget_only_with_a_verified_recovery`.

**Proof command:** `node tools/gh-check.mjs cargo test -p engine-render --lib render_heal` →
`test result: ok. 3 passed; 0 failed`, and `node tools/unwired-code-gate.mjs` must not print
`unwired-module render heal render/src/heal.rs no-caller`.

### Spec B — `crates/audio/src/heal.rs`: what the mixer path must report (smart lane)

**Call site:** the mixer report path in `crates/audio/src/heal.rs` (the `DeviceHeal` methods around
`crates/audio/src/heal.rs:302`, where the supervisor fault is reported) and the mixer itself.

**Function names to call:**
1. `engine_observe::recovery::HealObservation::HeldLastGood { frames, digest }` when the device is
   gone and the mixer keeps rendering the held buffer: a held buffer is a frozen frame, so the
   heal is typed as a presentation heal and never counts as green.
2. `engine_observe::recovery::HealObservation::Presented { digest }` when the device reopened and
   a fresh buffer is presented.
3. `engine_observe::supervisor::Supervisor::verify_heal(DEVICE_FAULT_CODE, attempt, &observation,
   &mut self.ring)` right after the reopen attempt, so the audio budget is cleared only by a
   proof.

**What the mixer path must report into the heal:** the number of held frames and the digest of
the held buffer (frozen frame), or the digest of the fresh buffer after a reopen (recovered). No
proof, no budget reset: `Supervisor::reset` returns `ResetOutcome::Refused` and the audio fault
budget stays spent.

**Test names to add in `crates/audio/src/heal.rs`:** `mixer_reports_the_device_fault_into_the_heal`,
`device_heal_that_holds_the_mixer_is_a_frozen_frame`,
`device_heal_clears_its_budget_only_with_a_verified_recovery`.

**Proof command:** `node tools/gh-check.mjs cargo test -p engine-audio --lib device_heal` →
`test result: ok. 3 passed; 0 failed`.

### Spec C — `crates/observe/src/rerun_sink.rs`: keep it, carry the heal evidence (mine)

**Decision: keep the sink and give it a job.** The compaction lane wants
`crates/observe/src/rerun_sink.rs` deleted as dead code (321 lines, test-only today). It is the
only bounded, overflow-counted record of (tick, state hash) in the engine, and the heal proof
needs exactly that: a last-good digest per tick and the first divergent tick after a bad heal.
Deleting it would remove the evidence channel the heal rows write into.

**Call site:** `crates/observe/src/rerun_sink.rs:90` `RerunSink::push` (the per-tick record) and
`crates/observe/src/rerun_sink.rs:117` `RerunSink::first_divergent` (the hunt).

**What I own and landed in `crates/observe`:** a heal-aware row beside the frames, so the
last-good digest per tick and the heal rows travel together:
`RerunSink::push_heal(&HealTapeRecord)`, `RerunSink::heal_rows() -> impl Iterator<Item =
&HealTapeRecord>`, `RerunSink::heal_len()` and `RerunSink::first_unproved_heal()`. The heal ring
holds 32 rows (`MAX_RERUN_HEAL_ROWS`), evicts the oldest when full and counts the drop in the
same overflow counter, so nothing is dropped silently. The sink stays std-only and stays out of
the hash.

**What the WEB6 lane owns in `crates/game/src/crash.rs`:** push the crash ring into the sink and
push every heal row the run produced (`supervisor.tape()`), then name the first divergent tick
through `RerunSink::first_divergent`, so `log.query` can name the tick where the run left the
last good state and the heal that caused it.

**Test names to add in `crates/observe/src/rerun_sink.rs`:**
`rerun_sink_carries_the_heal_rows_inside_the_same_budget` (landed),
`rerun_sink_names_the_first_divergent_tick_after_a_bad_heal` (landed).

**Proof command:** `node tools/gh-check.mjs cargo test -p engine-observe --lib rerun_sink` →
`test result: ok. 10 passed; 0 failed`.

## What the gates read

- **G1 (never broken, 7 days):** the heal cap and the frozen-frame rule move no printed number
  yet: G1 reads the captures in `tools/product-check.mjs`, and the cap and the verdict live in
  `crates/observe`. Until `tools/product-check.mjs` writes a heal row into its history, a healed
  cycle can still sit in the history as GREEN. **The number does not move yet.**
- **A9 (chaos and fuzz):** the same: `crates/proof/src/a9.rs` reads the chaos report, and the
  chaos suite still proves detection, not recovery ("healed 3, quarantined 9"). Until HLX-13
  lands, the A9 count does not move. **The number does not move yet.**
