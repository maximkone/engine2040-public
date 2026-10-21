# Proposed rows — arXiv AI-for-games wave (lane 08, 2026-10-20)

Every row below is a **proposal** for the lead to merge into the board. Nothing here is
committed and nothing here is on the board yet. The lane's research file is
`docs/research/powerups-2026-10-20/08-arxiv-ai-games.md` (12 ranked papers, the
rejections and the top three); this file is the row-shaped view of the same work.

**How the papers were read.** The arXiv export API (`export.arxiv.org/api/query`) returned
**HTTP 429** on every call on 2026-10-20, so no id was confirmed through the API. Every id
below was read at `arxiv.org/abs/<id>` on 2026-10-20 (title, version line, abstract), and
every donor repository was read live through the GitHub API on 2026-10-20 (root listing, the
named file, the LICENSE text). Grades: OBSERVED (read today), DERIVED (reasoned from what I
read), UNKNOWN.

**The three flags that must survive the merge.**

1. **Game-Creation-Agents (2610.16776) needs paid keys.** Its README states the pipeline
   needs Claude Code with project MCP support, an OpenAI API key and a Volcengine ARK key
   (OBSERVED, README read 2026-10-20). Reproducing their pipeline is therefore
   **RESEARCH-ONLY**. The *shape* — a verify-checkpoint tool that runs our own checks and
   returns a per-check verdict — is **PORT-CANDIDATE** with zero donor bytes.
2. **The AgenticSTS mod is AGPL-3.0.** The donor's root LICENSE is a monorepo map read live
   2026-10-20: `AgenticSTS/` is **Apache-2.0**, `AgenticSTS-Mod/` is **GNU AGPL v3.0**
   (a fork of CharTyr/STS2-Agent, whose AGPL terms are inherited) and `AgenticSTS-Data/` is
   **CC BY 4.0** (OBSERVED). We port the Apache-2.0 *shape* only and vendor **no bytes**; the
   copyleft mod is a risk line, not a source.
3. **Every number below that comes from a paper is the author's benchmark, not ours.** No
   figure in these rows is our runtime result. Where a row has a measured acceptance, the
   measurement is *ours*, on *our* tree.

**Lanes that hold crates.** `crates/endpoint` is held by the AI-native lane, `crates/provider`
by the OPT27 lane. This wave therefore lands code in `crates/playtest` only; the endpoint and
provider shapes are written as **specs** at the end of this file, in the right section, so
the lead can hand each to its owner without re-deriving it.

---

## AIX-1 — runtime state injection plus keypoint verification (GameGen-Verifier shape)

**Title.** Inject a declared runtime state, run a bounded interaction, judge the outcome.

**What it is.** A unit states the runtime state the engine must hold when the interaction
starts, the bounded interaction itself, and the end states that must not be mistaken for
success. The verifier reaches the declared state through the spec's scripted prefix, proves
the arrival by comparing the reached state with the declared one, then judges the outcome
against the spec's asserts. Every verdict carries a reason that names the keypoint.

**arXiv.** https://arxiv.org/abs/2605.07442 — v1, submitted 8 May 2026; abs page read
2026-10-20 (OBSERVED). "GameGen-Verifier: Parallel Keypoint-Based Verification for
LLM-Generated Games via Runtime State Injection", NetX-lab; 92.2% accuracy against human
judgments versus 58.8% for a coverage-enforced agent-as-verifier baseline, 16.6x faster wall
clock (author's numbers, OBSERVED on the abs page; not ours).

**Donor.** `https://github.com/NetX-lab/GameGen-Verifier` — LICENSE read live 2026-10-20
(SHA 9d37794fe02aaab686af0a03fd3feb4f17d0acc8), **MIT**. Files read live 2026-10-20:
`skills/generative-state-construction/SKILL.md` (fuzzy `prestate`/`instruction`/`expected`
turned into a machine-usable contract, plus "what must not be mistaken for success"),
`skills/short-interaction-verification/SKILL.md` (inject the target precondition, run a
bounded interaction, write the before/after bundle), `harness/` (isolation, concurrency,
fault recovery), `spec/` (one keypoint spec per game). Zero donor bytes vendored; the shape
is reimplemented in Rust.

**Rows closed.** **D19** (verifier output in every build verb — this is the shape of the
verifier, not the wiring) and **D18** (agent memory and a task suite scored by programmatic
referees, no LLM judge: the unit is the referee, the near-miss set is the programmatic
referee). Secondary: **HS20/HS29/HS30/HS31** (the public-benchmark mirrors gain a per-check
verdict instead of a boolean).

**Owned paths.** `crates/playtest/src/keypoint/inject.rs` (new), `crates/playtest/src/keypoint.rs`
(module line and re-exports), `crates/playtest/src/keypoint/spine.rs` (one `pub(super)` on the
millimetre helper). Nothing outside `crates/playtest`. No `Cargo.toml` change.

**Why the file is `keypoint/inject.rs` and not `keypoint.rs`.** `crates/playtest/src/keypoint.rs`
already existed and already held the 42-spec keypoint suite, the seeded-bug sensitivity
harness and the spine track (read 2026-10-21). Overwriting it would have deleted landed
work, so the new shape went in as a sibling submodule beside `keypoint/json.rs`,
`keypoint/spec_parse.rs` and `keypoint/spine.rs` — the crate's own layout for keypoint code.
The public surface is re-exported from `keypoint.rs`, so a caller writes
`engine_playtest::keypoint::verify_unit(...)`.

**Accept.** `node tools/gh-check.mjs cargo test -p engine-playtest --lib keypoint::inject` must
print, from the run:
- `injected_unit_returns_a_verdict` green — a unit returns a verdict with both slots observed;
- `injected_state_that_matches_passes` green — a declared state the fight reached passes;
- `wrong_pose_fails_with_the_reason_and_the_keypoint_named` green — a pose 1000 mm off is a
  failing verdict whose reason names the keypoint and says expected/observed;
- `verdict_bytes_are_the_same_twice` green — the same unit gives the same verdict bytes and
  the same digest;
- `near_miss_end_state_is_not_success` green — a declared look-alike end state fails;
- `a_unit_with_no_check_is_refused`, `a_spine_unit_is_refused_by_name`,
  `one_bad_unit_does_not_stop_the_batch` green — typed refusals, batch isolation;
- `the_unit_makes_no_model_call` green — the production half of the file names no model
  vendor, so the unit is judged by the arena alone.

**Proof (2026-10-21, GitHub runner).**
- `cargo test -p engine-playtest --lib keypoint::inject` -> **9 passed; 0 failed; 442 filtered
  out** (run 39509417970). The nine names are exactly the ones above.
- `cargo clippy -p engine-playtest --lib --no-deps -- -D warnings` -> **PASS** (run
  39511554275). The `--no-deps` scope is needed only because another seat's in-flight
  `crates/physics/src/cloth.rs` has an unread `hash_cell_of` field that `-D warnings` turns
  into an error; that red is in a file this lane never touched.
- `cargo test -p engine-playtest --lib` (the whole crate) -> **444 passed; 3 failed; 4
  ignored** (run 39511992206). The three failures are `agent::tests::runner_cancels_and_
  expires_with_bounded_logs`, `spatial_eval::known_good::known_good_shipped_scenes_pin_zero_
  red_and_the_yellow_list` and `winrates::tests::invalid_configs_fail_without_simulating`.
  **Pre-existing, not ours:** with this lane's three files temporarily removed from the tree
  (backed up, `git checkout` on the two edited ones, the new one moved aside), the
  `winrates` test still fails (run 39512748775, "441 filtered out" — the nine new tests
  absent). The three live in `crates/playtest/src/agent.rs`,
  `crates/playtest/src/spatial_eval/known_good.rs` and `crates/playtest/src/winrates.rs`,
  none of which this lane touched.
- `rustfmt --edition 2021 --check` on the three touched files: clean.
- The golden `d57b5c8ae3096614` is not touched: the new module only reads a recorded
  dataset and folds integers; it never writes a sim row, a checkpoint or a hash.

**Kill line.** The golden `d57b5c8ae3096614` moves; the verdict stops being byte-stable
across two runs of the same unit; the unit needs a model call to reach a verdict; the new
module touches a file outside `crates/playtest`.

**Cost.** M (one module, one helper visibility bump, one test module).

**Verdict.** **PORT-CANDIDATE** for the shape (reimplemented, no bytes). The paper's 92.2%
figure is the authors' benchmark and needs their harness — **RESEARCH-ONLY** to reproduce.

---

## AIX-2 — a verify-checkpoint tool that returns a per-check verdict (Game-Creation-Agents shape)

**Title.** `verify_checkpoint`: run our own checks over a build and return one verdict per
check, each naming what broke.

**What it is.** The donor pipeline builds a machine-readable contract and deterministic
tests next to every artifact, then a checkpoint tool runs those checks and writes a
verifiable report; each blocker returns to the agent that owns the affected artifact for a
bounded repair, and the build ships only when every gate passes. The portable half is the
checkpoint: a tool that runs *our* checks and returns a per-check verdict, not one boolean.

**arXiv.** https://arxiv.org/abs/2610.16776 — v1, submitted 12 Oct 2026; abs page read
2026-10-20 (OBSERVED). "Game-Creation-Agents: Autonomous Game Production", Tencent; eight
specialized agents, a reduced manifest joining the parallel branches, build-linked real-UI
checks plus a code-blind critic that decides whether the game may ship (author's numbers,
OBSERVED; not ours).

**Donor.** `https://github.com/Tencent/Game-Creation-Agents` — LICENSE read live 2026-10-20
(SHA d54cbfc5145eb2bb0077121cbf82e83dccd47c66), **Apache-2.0** with the header clause
"except for the third-party components identified below, which remain subject to their
respective original license terms" (OBSERVED; third-party vendor components are a licence
risk line). Files read live 2026-10-20: `gca-mcp/src/gca_mcp/tools/verify_tools.py`
(`run_tests`, `gate_check`, `write_verify_report`, `play_game`, `visual_diff`,
`capture_layout`; a `ThreadPoolExecutor` and a `BoundedSemaphore` for concurrency),
`gca-mcp/src/gca_mcp/verify_checkpoint.py` (identity-bound checkpoints, `_input_sha256` over
the argument set, receipts with stdout/stderr omitted), `gca-mcp/src/gca_mcp/verify_epoch.py`
(runtime token receipt, canonical playtest path), `gca-mcp/src/gca_mcp/evidence.py` (atomic
write, build digest), `README.md`. Zero donor bytes vendored.

**Rows closed.** **D19** (verifier output in every build verb — this is the wiring half) and
**D17** (scene diff plus an editor activity feed tagged human or agent: the checkpoint's
identity binding is what makes an activity row attributable). Secondary: **AX23/AX24** (the
receipt says what changed) and **FU2** (the AI-native editor spike: a diff and merge window
whose merge carries a per-check verdict).

**Owned paths.** `crates/endpoint/src/audit/verify.rs` and the `verify` pack under
`crates/endpoint/src/packs/` — **held by the AI-native lane**; this wave writes the spec
only (see "For the endpoint lane" below) and lands no endpoint code.

**Accept.** (for the endpoint lane) `node tools/gh-check.mjs cargo test -p engine-endpoint
--features verify --lib verify_checkpoint` must print: a checkpoint over a passing build
returns one green verdict per check; a checkpoint over a failing build returns one red
verdict per failing check, each naming the check and the reason; the same arguments twice
give the same checkpoint receipt bytes; a refused check names its refusal code and the
recovery, never panics.

**Kill line.** The endpoint lane cannot own a per-check verdict without changing the
advertised schema budget (the tool list is already at the ledger bound); the checkpoint
receipt is not byte-stable; the tool needs a model call to produce a verdict.

**Cost.** M (one tool, one receipt type, one test module).

**Verdict.** **PORT-CANDIDATE** for the verify-tool shape. **RESEARCH-ONLY** to reproduce
their pipeline: it needs Claude Code, an OpenAI API key and a Volcengine ARK key (OBSERVED
in their README), all paid and closed.

---

## AIX-3 — a bounded memory contract for the agent loop (AgenticSTS shape)

**Title.** A retriever with a write gate: the agent memory cannot grow without bound.

**What it is.** Every decision is a fresh user message assembled by typed retrieval; no raw
cross-decision transcript is appended to the prompt, so the prompt stays bounded across runs
of any length and each memory layer can be ablated on its own. A write gate admits a claim
into the store only when a state predicate agrees, so a remembered "done" cannot become a
fact on the model's word alone.

**arXiv.** https://arxiv.org/abs/2607.02255 — v1, submitted 2 Jul 2026; abs page read
2026-10-20 (OBSERVED). "AgenticSTS: A Bounded-Memory Testbed for Long-Horizon LLM Agents";
in a fixed-A0 ablation the no-store baseline wins 3/10 games and adding the skill layer 6/10
at that sample size — the authors call it directional, not significant (author's numbers,
OBSERVED; not ours).

**Donor.** `https://github.com/AlayaLab/AgenticSTS` — root LICENSE read live 2026-10-20
(SHA 43d8f3ae8462baf0da8e4eefd462024abd689bfa): a **monorepo map** — `AgenticSTS/`
**Apache-2.0** (the Python agent: decision engine, memory, skills, tooling), `AgenticSTS-Mod/`
**GNU AGPL v3.0** (a fork of CharTyr/STS2-Agent, AGPL terms inherited), `AgenticSTS-Data/`
**CC BY 4.0** (run archive and frozen store snapshots). Directory listing read live 2026-10-20:
`AgenticSTS/src/memory/` (`retriever.py`, `short_term.py`, `write_gate.py`,
`write_gate_judge.py`, `write_gate_lifecycle.py`, `card_memory_store.py`, `prompt_injector.py`,
`combat_store.py` and siblings), `AgenticSTS/src/skills/` (`library.py`, `merge_pipeline.py`,
`critic_prompt.py`, `stub_validators.py`). **We port the Apache-2.0 shape only and vendor no
bytes; the AGPL-3.0 mod is a copyleft risk line and is never a source.**

**Rows closed.** **D18** (agent memory and a task suite the door serves) and **AX28**
(play.run as an MCP task: a bounded prompt is what makes a long run affordable). Secondary:
**OPT29** (agent onboarding budget: tokens from discover to first applied edit, per vendor) and
**FU3** (the open-prompt eval spike on the A6 tasks).

**Owned paths.** `crates/provider/src/agent_loop.rs` — **held by the OPT27 lane**; this wave
writes the spec only (see "For the provider lane" below) and lands no provider code.

**Accept.** (for the provider lane) `node tools/gh-check.mjs cargo test -p engine-provider
--lib bounded_memory` must print: a loop over 200 decisions keeps the prompt at or under the
declared bound; two memory layers can be switched off one at a time and each ablation moves
the score in a named direction; a claim the write gate rejects does not appear in the next
prompt; the same transcript gives the same prompt bytes twice; the transcript itself is
never appended raw to the prompt.

**Kill line.** The bound cannot be met without dropping information the decision needs; the
write gate needs a model call to judge a claim; the provider lane's transcript golden moves.

**Cost.** M (one retriever, one write gate, one test module).

**Verdict.** **PORT-CANDIDATE** for the shape (reimplemented, no bytes, Apache-2.0 half
only). Their win-rate claim needs a commercial game and a paid model — **RESEARCH-ONLY**.

---

## AIX-4 — three lifecycle tracks for the task suite (GameXpert-Bench shape)

**Title.** Split the build-task suite into build, fix and optimize tracks.

**What it is.** The benchmark operationalizes three stages of the development lifecycle as
three tracks: GameGen (create a complete game from one request in an empty workspace),
GameFix (diagnose and repair a reported or self-found defect) and GameOpt (cumulative
optimization along request chains). Each track is scored by its own deterministic checks.

**arXiv.** https://arxiv.org/abs/2608.21833 — v1, submitted 22 Aug 2026; abs page read
2026-10-20 (OBSERVED). "GameXpert-Bench: How Far Are Coding Agents from Expert Game
Development?".

**Donor.** `https://github.com/Kwen-Chen/GameXpert-Bench` — root listing read live 2026-10-20
(`.gitignore`, `README.md`, `docs/`). **No LICENSE file**: the LICENSE path 404s on
2026-10-20 (OBSERVED), so there are no donor bytes to port; the shape is reimplemented from
the paper text alone.

**Rows closed.** **D18** (10 build tasks scored by programmatic referees with Play@k) and
**D19** (a fix track needs a verifier in the repair verb, not only in the build verb).
Secondary: **D20** (tune.sweep: one knob, a range, 5 seeds, a CSV and the win-rate curve —
that is the optimize track's score) and **AX11** (the efficiency-utility report column: the
three tracks report their own utility).

**Owned paths.** `crates/playtest/src/brief_eval.rs` (the 20 authoring + 5 play briefs and
the `EvalReport`) and `crates/playtest/src/seeded_bug.rs` (the fix track's planted defects) —
both in `crates/playtest`, which this wave owns.

**Accept.** `node tools/gh-check.mjs cargo test -p engine-playtest --lib brief_eval` must
print a report that names the three tracks separately, each with its own pass count and its
own digest, and the same report twice must be byte-identical.

**Kill line.** A track cannot be scored by a deterministic check; the fix track needs an LLM
judge; the report stops being byte-stable.

**Cost.** S (a track field and three counters in the existing report).

**Verdict.** **PORT-CANDIDATE** for the shape (ideas-only — no licence, no bytes). The
benchmark's own scores are the authors' — **RESEARCH-ONLY**.

---

## AIX-5 — a free-model capability ladder for the door (AgentFloor shape)

**Title.** A deterministic six-tier ladder that says which door work a small open-weight
model can do, and where the frontier is still needed.

**What it is.** A deterministic 30-task benchmark over 16 open-weight models from 0.27B to
32B parameters plus GPT-5, 16,542 scored runs. The result is a boundary of necessity: small
and mid open-weight models already cover most short-horizon structured tool use, the
strongest open-weight model matches GPT-5 in aggregate, and the gap sits in long-horizon
planning with sustained constraint tracking.

**arXiv.** https://arxiv.org/abs/2605.00334 — v1, submitted 1 May 2026; abs page read
2026-10-20 (OBSERVED). "AgentFloor: How Far Up the tool use Ladder Can Small Open-Weight
Models Go?".

**Donor.** **No code found.** GitHub repository search for "AgentFloor benchmark tool use" and
"AgentFloor open-weight tool use" returned 0 hits on 2026-10-20, and the abs page carries no
code link (OBSERVED). There is nothing to license and nothing to port; the ladder shape is
reimplemented from the paper text.

**Rows closed.** **D18** (pass rate per model, with a free-model row), **OPT29** (agent
onboarding budget: tokens from discover to first applied edit, per vendor — the ladder says
which vendor tier to budget for) and **EV15** (the research ladder). Secondary: **AX11** (the
efficiency-utility column: a tier is a utility band).

**Owned paths.** `crates/playtest/src/eval.rs` (the scorecard that already runs mock, local
and remote backends) and `crates/provider/src/router.rs` (the router that picks a backend per
task class) — the provider half is **held by the OPT27 lane**; the playtest half is ours.

**Accept.** `node tools/gh-check.mjs cargo test -p engine-playtest --lib eval` must print a
tiered report where each tier names the task class, the model, the pass count and the
tokens, with the tiers in a fixed order and the same report twice byte-identical.

**Kill line.** A tier needs a paid endpoint to rank; the ladder changes the mock/local/remote
parity the A6 proof pins; the report stops being byte-stable.

**Cost.** S (tier fields and a per-tier counter in the existing scorecard).

**Verdict.** **PORT-CANDIDATE** for the ladder shape (reimplemented, no code exists). Their
GPT-5 comparison is the authors' — **RESEARCH-ONLY**. This row matters most to us because
our model budget is free-only: the ladder is what tells us which tier a free model reaches.

---

## AIX-6 — evidence travels in the prompt, not in a file (Same Simulation, Different Evidence shape)

**Title.** Name the authoritative value in the prompt; never let the model re-derive it.

**What it is.** Agents that compute the right answer can still answer wrongly. With the
simulation held fixed, reading a long tool report through a section index instead of in
context costs GPT-5.1 and GPT-5.2 11.3 and 7.9 points of accuracy; both models compute the
keyed value about as often as before, so what changes is the evidence they answer from.
Under retrieval they skip the report, answer from their own higher-precision output files
and misround to a neighbouring option — the failure the paper calls evidence substitution.
Naming the authoritative source closes it.

**arXiv.** https://arxiv.org/abs/2610.26836 — v1, submitted 19 Oct 2026; abs page read
2026-10-20 (OBSERVED). "Same Simulation, Different Evidence: Where Retrieved Tool Output
Costs LLM Agents Accuracy" (the day before this wave; PHREEQC, 158 execution-verified
items, 11,007 logged simulator calls).

**Donor.** **No code found.** The abs page carries no repository link (OBSERVED, read
2026-10-20). Nothing to license; the rule is reimplemented from the paper text.

**Rows closed.** **OPT29** (the agent onboarding budget must count the evidence we send),
**AX11** (the efficiency-utility column) and **AX17** (the second-trace agreement probe).
Secondary: **D19** (a build verb's verdict must carry its own evidence, not a pointer to
where the evidence might be).

**Owned paths.** `crates/provider/src/prompt.rs` (the versioned prompt assets) and
`crates/provider/src/reader.rs` (the reader adapter that holds the oracle answer) — **held by
the OPT27 lane**; plus `crates/endpoint/src/scene_query.rs` and `crates/spatial/src/brief.rs`
for the door side — **held by the AI-native lane**. This wave writes the rule into the
row file and lands no code in either crate.

**Accept.** (for the two holding lanes) one test per lane that holds the world fixed and
changes only how the evidence is delivered, then reports the accuracy of both arms.

**Kill line.** The authoritative value cannot be named in the prompt; the in-context arm
costs more tokens than the retrieval arm saves; the trace export format moves.

**Cost.** S (one arm in the existing eval harness, one rule in the prompt builder).

**Verdict.** **PORT-CANDIDATE** for the rule (reimplemented, no code exists). Their
GPT-5.1/5.2 accuracy deltas are the authors' — **RESEARCH-ONLY**, and we must not quote
them as a result of our own free models.

---

## AIX-7 — world editing with an intervention depth and a preservation list (IGMWorld shape)

**Title.** Every edit carries a depth label and the properties that must not change.

**What it is.** World editing is framed as intervening on an executable world while
preserving the properties that should stay unchanged. Intervention depth — property, entity,
dynamics, system — is the axis; the checks are deterministic executability, behavioural,
preservation and visual criteria over 110 tasks and more than 1.1K criteria. Reliability
falls as depth rises, and the pattern holds even between tasks with the same number of
criteria.

**arXiv.** https://arxiv.org/abs/2610.02331 — v1, submitted 1 Oct 2026; abs page read
2026-10-20 (OBSERVED). "World Editing: Intervening on Executable Worlds at Increasing Depth";
the strongest configuration solves 78.2% of tasks at task level and 94.8% at criterion level
(author's numbers, OBSERVED; not ours).

**Donor.** `https://github.com/vinesmsuic/IGMWorld` — root listing read live 2026-10-20:
**`README.md` only**, no code and no LICENSE (OBSERVED). Nothing to port; the shape is
reimplemented from the paper text.

**Rows closed.** **D17** (scene.diff: a snapshot id then deltas only — the preservation list
is the "must not change" half of a diff) and **FU2** (the AI-native editor spike: a diff and
merge window). Secondary: **AX12** (recorded branch-merge fork receipts: a merge must carry
the preservation verdict) and **AX23/AX24** (the receipt says what changed).

**Owned paths.** `crates/editor/src/activity.rs` (the `SceneDiff` and `ActivityRing` that
already render a three-way diff) and `crates/endpoint/src/scene_tools.rs` (the `tx.apply`
receipt) — the endpoint half is **held by the AI-native lane**; the editor half is ours but
the row is a spec until the endpoint lane owns the verb.

**Accept.** (for the endpoint and editor lanes) one test that edits a scene through
`tx.apply` with a declared depth and a preservation list, and reports one verdict per
preserved property plus a depth label on the receipt.

**Kill line.** The depth label changes nothing measurable; a preservation property cannot be
stated in the engine's own numbers; the receipt grows past the advertised schema budget.

**Cost.** M (one depth field, one preservation list, one verdict per property).

**Verdict.** **PORT-CANDIDATE** for the shape (reimplemented, no code exists). Their 78.2% is
the authors' — **RESEARCH-ONLY**.

---

## AIX-8 — a search over generator programs with a reusable primitive library (PCG metageneration shape)

**Title.** Search over content *generators*, not over levels, and keep the primitives that
worked.

**What it is.** Language models mutate and cross over whole executable generator programs
rather than individual levels. A second idea, Continual Abstraction Discovery, extracts the
reusable primitives out of the high-fitness programs into a run-specific helper module. Over
160 runs (at least ten 50-generation runs per cell) the primitive library raises the mean
final best fitness in all eight domain-and-API comparisons, and the learned libraries are
adopted by most later programs.

**arXiv.** https://arxiv.org/abs/2608.17947 — v1, submitted 18 Aug 2026; abs page read
2026-10-20 (OBSERVED). "Procedural Content Metageneration via Program Search and Continual
Abstraction Discovery", accepted at IEEE Conference on Games 2026.

**Donor.** **No code found.** GitHub repository search for "procedural content metageneration
program search" returned 0 hits on 2026-10-20 and the abs page carries no code link
(OBSERVED). The reusable half for us is the *idea*, not a donor file: a program-shaped level
generator plus a library of the primitives that kept working.

**Rows closed.** **D18** (the primitive library is a memory the door serves: the helpers
that kept passing stay) and **D19** (a generator program is content the door compiles, so it
needs the same verifier as any other build verb). Secondary: **AB1**-family brief-to-level
rows and the `crates/rules` Rhai sandbox (a generator is a program, and the sandbox already
runs integer-only programs the door installs).

**Owned paths.** `crates/rules/src/rules.rs` and `crates/rules/src/script.rs` (the Rhai
sandbox that compiles, vets and smoke-runs a candidate script) — ours. The generator search
itself is a future row, not this one.

**Accept.** (for a future row) a run of N generations over one brief posts the best fitness
per generation, the count of primitives extracted, and the count of later programs that
adopt a primitive — all integers, no floats.

**Kill line.** The search needs a model call per generation and our budget is free-only; a
generator program cannot be expressed in the integer-only Rhai sandbox; the fitness measure
cannot be stated in the engine's own numbers.

**Cost.** L (a search loop, a fitness measure, a primitive store).

**Verdict.** **IDEAS-ONLY** for now (no code, and the search needs a paid model per
generation) — the free-model budget is the blocker, not the idea.

---

## AIX-9 — a small-model content generator scoped narrow (small-language-model content shape)

**Title.** Scope the task narrow, constrain the structure, and a small model is enough.

**What it is.** Instead of reaching for a cloud model, the paper argues that high-quality
generation comes from aggressive fine-tuning on deliberately scoped tasks with narrow
context and constrained structure, with the training corpus built by a DAG-based synthetic
data route. The proof of concept is one specialized model powering a minimal RPG loop.

**arXiv.** https://arxiv.org/abs/2601.23206 — v1 30 Jan 2026, last revised 19 May 2026
(**v2**); abs page read 2026-10-20 (OBSERVED). "High-quality generation of dynamic game
content via small language models: A proof of concept".

**Donor.** **No code found** on the abs page (OBSERVED, read 2026-10-20): the page links only
the Hugging Face hub and its docs, no repository. Nothing to license.

**Rows closed.** **D18** (the pass-rate-per-model row: a scoped task is what a free model
can hold) and **D19** (a generated content artifact goes through the same verifier as any
other build verb). Secondary: **OPT27** (the provider's inference arbiter: a narrow task
class is a routing decision, not a model choice).

**Owned paths.** `crates/provider/src/router.rs` and `crates/provider/src/ladder.rs` (the
backend picker and the structured-output ladder) — **held by the OPT27 lane**; the
crate-side scope rule is a spec only.

**Accept.** (for the provider lane) the router picks a backend per task class and the scorecard
reports the pass rate per class, so a narrow class and a wide class can be compared.

**Kill line.** The scope rule needs a fine-tuned model we cannot host; the router's parity
between mock, local and remote moves.

**Cost.** S (a task-class field and a report column) for the scope rule; L for the training
route, which is out of reach for us.

**Verdict.** **IDEAS-ONLY** for the training route (a fine-tuned model we cannot host, no
code released). The *scope* rule — narrow the task, constrain the structure, then pick the
cheapest backend that holds — is **PORT-CANDIDATE** as a router rule, and it is the rule
that keeps us inside the free-model budget.

---

## AIX-10 — a small-LM content generator's near-miss set (small-LM content shape, second half)

**Title.** Say what must not be mistaken for success, or a look-alike end state passes.

**What it is.** The same narrow-scope work, read the other way: the contract has to name
the near-miss states as well as the wanted one, or a plausible-but-wrong end state is
accepted. The GameGen-Verifier skill says the same thing in its own words: the contract
states what must be observed *and* what must not be mistaken for success.

**arXiv.** https://arxiv.org/abs/2601.23206 (v2) and https://arxiv.org/abs/2605.07442
(v1) — both abs pages read 2026-10-20 (OBSERVED). The two papers together give the pair:
the narrow scope from the first, the near-miss set from the second.

**Donor.** No code on either page (OBSERVED, read 2026-10-20). The shape is reimplemented
in the AIX-1 module.

**Rows closed.** **D19** (a build verb's verdict must reject a look-alike) and **D18** (the
referee is programmatic, so the near-miss set is what makes it strict). Secondary: **AX11**
(the efficiency column must count a rejected near-miss, not only a failure).

**Owned paths.** `crates/playtest/src/keypoint/inject.rs` (the `NearMiss` enum and the check)
— ours, landed with AIX-1.

**Accept.** `node tools/gh-check.mjs cargo test -p engine-playtest --lib keypoint::inject`
must print `near_miss_end_state_is_not_success` green — a declared look-alike end state
fails the verdict with a reason naming the keypoint.

**Kill line.** A near-miss check rejects a correct build; the near-miss set needs a model to
enumerate.

**Cost.** S (the enum and the check).

**Verdict.** **PORT-CANDIDATE** (landed with AIX-1). Both papers' scores are the authors' —
**RESEARCH-ONLY**.

---

## AIX-11 — a verifier that judges the interaction, not the artifact (CraftBench-UE correction)

**Title.** The asset checks can pass while the runtime assertions fail.

**What it is.** The CraftBench-UE harness reconstructs an agent's saved submission in a fresh
project and runs deterministic build, asset and runtime checks with **no LLM judge**. Two
numbers are the correction to our mirror row: on the 10 paired tasks (same gameplay, same
runtime tests, C++ versus Blueprint as the deliverable) C++ completion beats Blueprint by
30.0 and 42.9 points, and of on-time Blueprint submissions that pass the asset checks,
**42.2% and 50.0% fail explicit runtime assertions** — the assets are fine and the gameplay
is not.

**arXiv.** https://arxiv.org/abs/2609.23142 — v1, submitted 19 Sep 2026; abs page read
2026-10-20 (OBSERVED). "CraftBench-UE: Deterministic Evaluation for Coding Agents in
Unreal Engine" (author's numbers, OBSERVED; not ours).

**Donor.** `https://github.com/ramenvr/craftbench-ue` — LICENSE read live 2026-10-20
(SHA 39814ec73bf01d0615dd45848ac7823f012a6b72), **MIT**. Root listing read live 2026-10-20.
No bytes vendored.

**Rows closed.** **HS30** (the mirror CraftBench-UE slice, landed) and **HS32** (the
aggregate report) — this is a **CORRECTION** to a landed row, not a new row: the shape is
already ours, the numbers are the new fact. Secondary: **D19** (the asset/runtime split is
the reason a build verb must return both).

**Owned paths.** `crates/playtest/src/mirror_craft.rs` (the mirror and its paired-shape
test) and `crates/playtest/src/brief_eval.rs` (the `asset_pass_runtime_fail` counter) — ours.

**Accept.** `node tools/gh-check.mjs cargo test -p engine-playtest --lib mirror_craft` must
print the 10-task mirror report with the asset-pass-runtime-fail count beside the published
numbers, and the report twice byte-identical.

**Kill line.** The paired shape cannot be expressed in the mirror; the counter stays a
boolean with no rate.

**Cost.** S (a paired task in the mirror, one printed rate).

**Verdict.** **PORT-CANDIDATE** for the paired shape (MIT, no bytes). Their 42.2% and 50.0%
are the authors' — **RESEARCH-ONLY**; our number is the one we print.

---

## AIX-12 — the engine as the reward environment (agentic game development as a data engine shape)

**Title.** A game scene is an executable world specification; the engine is the referee.

**What it is.** A scene encoded by a game engine is an executable world specification: the
engine can check collision, physics, navigability and bounded playability cheaply, while
the developer supplies the global accept signal. Game development is therefore a reward
environment for spatial world models, with dense engine signals plus implicit human
acceptance from the development process. The portable half for us: the engine's own checks
are the referee and the human's accept/reject is the top-level signal.

**arXiv.** https://arxiv.org/abs/2608.25518 — v1, submitted 26 Aug 2026; abs page read
2026-10-20 (OBSERVED). "Agentic Game Development as a Verifiable Trajectory Data Engine for
Scaling World Models" (RLHEV). No code link on the abs page (OBSERVED).

**Donor.** **No code found** (no repository link on the abs page, read 2026-10-20). The
shape is reimplemented; the paper's RL post-training route is out of reach for us.

**Rows closed.** **D18** (the task suite's referee is the engine plus the human accept, not
an LLM judge) and **D19** (the build verb's verdict is the engine's checks). Secondary:
**D20** (the tune.sweep win-rate curve is the dense signal the sweep produces).

**Owned paths.** `crates/playtest/src/keypoint/inject.rs` (the unit is the engine check) and
`crates/endpoint/src/audit/verify.rs` (the per-check verdict the door returns) — the
endpoint half is **held by the AI-native lane**.

**Accept.** (for the endpoint lane) `verify_checkpoint` returns one verdict per engine
check, and the human's accept/reject is recorded as the top-level signal beside it.

**Kill line.** The engine checks cannot be stated in the engine's own numbers; the reward
route needs a training run we cannot fund.

**Cost.** M (the reward route) for the paper's shape; S for the referee half we already
have in AIX-1.

**Verdict.** **IDEAS-ONLY** for the RL post-training route (needs a training run and a paid
model) — the referee half is **PORT-CANDIDATE** and is already landed by AIX-1.

---

# For the endpoint lane

**Owner:** the AI-native lane. `crates/endpoint` is held; this wave lands no endpoint code.
Everything below is a spec, written so the owner can implement it without re-deriving it.

### Spec 1 — the verify-checkpoint tool (AIX-2)

**File path.** `crates/endpoint/src/audit/verify.rs` — the existing `AuditVerifier` (the
streaming seal check over the door audit log) is the natural owner: it already folds
integer records into a digest and already refuses tampering by name. Add the checkpoint
there rather than a second receipt type.

**Function and type names.**

- `pub const TOOL_VERIFY_CHECKPOINT: &str = "verify.checkpoint";` — the tool name, next to
  the existing `TOOL_VERIFY_PROOF` in `crates/endpoint/src/packs/verify.rs`.
- `pub struct CheckpointRequest { pub checks: Vec<String>, pub scene_revision: u64 }` — the
  argument shape: which checks to run and which scene revision they run against.
- `pub struct CheckpointReport { pub checkpoint_id: String, pub scene_revision: u64,
  pub verdicts: Vec<CheckVerdict>, pub digest: u64 }` — the receipt.
- `pub struct CheckVerdict { pub check: String, pub pass: bool, pub reason: String,
  pub digest: u64 }` — **one verdict per check**, each naming the check and the reason.
- `pub fn run_checkpoint(request: &CheckpointRequest) -> Result<CheckpointReport,
  McpToolError>` — the runner.

**Args.** `checks` (a non-empty list of check ids, at most 32, each 1..=64 bytes of
`[a-z0-9._-]`), `scene_revision` (the revision the checks run against; a mismatch with the
live revision refuses with `stale_scene`).

**Refusal behaviour.** An unknown check id refuses with the accepted ids named. A check
that throws becomes a **red verdict with that check's name and the reason**, never an
exception out of the tool. An empty `checks` list refuses (a checkpoint that checks nothing
is never a pass — the same rule the playtest crate already states). A stale revision
refuses with `stale_scene` and names the live revision.

**Receipt shape.** JSON with `checkpoint_id`, `scene_revision`, `verdicts[]` (each with
`check`, `pass`, `reason`, `digest`), and the `digest` line in the house style
(`digest <016x>`) that `FightReport::verify` already reads back. The receipt is written
atomically and the digest covers the verdicts in order, so the same arguments twice give the
same bytes.

**Proof command.** `node tools/gh-check.mjs cargo test -p engine-endpoint --features verify
--lib verify_checkpoint` — green with: a passing build returns one green verdict per check;
a failing build returns one red verdict per failing check with the reason; the same
arguments twice give the same receipt bytes; an unknown check id refuses with the accepted
ids named; an empty check list refuses.

### Spec 2 — the build-verb list D19 must cover (read 2026-10-20)

D19 wants a verifier in *every* build verb. Read from `crates/endpoint/src/packs/**` and
`crates/endpoint/src/mcp.rs` today, the verbs are exactly these.

**Core tools** (`crates/endpoint/src/mcp.rs`, `CORE_TOOL_NAMES`, 13):
`scene.read`, `tx.preview`, `tx.apply`, `tx.undo`, `play.run`, `capture`, `api.search`,
`api.describe`, `api.call`, `api.load`, `test.run`, `sim.build_encounter`, `game.build`.

**Pack verbs** (each pack under `crates/endpoint/src/packs/`):
- scene: `scene.brief`, `scene.spatial`, `scene.place`, `level.build`, `scene.query`,
  `scene.watch`, `scene.next`, `scene.pick`, `world.query`
- break_: `structure.analyze`, `break.fracture`, `hit.simulate`, `wound.table.get`
- anim: `anim.graph.get`, `anim.graph.set`, `anim.graph.undo`, `anim.preview`
- material: `material.list`, `material.get`, `material.schema`, `material.preview`,
  `material.param`, `material.compare`, `material.set`
- asset: `asset.generate`, `asset.intake` (feature `intake`)
- rules: `rules.set`, `rules.test`, `rules.explain`, plus `sim.plan_goals`
- geo: `geo.preview`, `geo.apply`
- gym: `sim.reset`, `sim.step`
- view: `look.judge`, `view.observe`, `view.laws`, `view.compare`
- net: `net.loopback`
- perf: `perf.table`
- play: `play.step_until`, `play.sweep`, `seat.open`, `seat.step`, `seat.close`,
  `ai.decisions`
- verify (feature `verify`): `replay.verify`, `replay.bisect`, `bug.replay`,
  `trace.summary`, `verify.proof`; without the feature the `replay` pack owns
  `replay.verify` and `replay.bisect`
- physics: `physics.read`, `physics.raycast`, `physics.shapecast`
- physics_settings: `settings.get`, `settings.set`, `settings.undo`
- web: `build.web`, `serve.web`, `browser.capture`, `browser.input`

**The verbs that change the game** — the ones D19 means — are the `CapabilityCard::Commit`
verbs plus the three core builders:
`scene.place`, `level.build`, `anim.graph.set`, `asset.intake`, `geo.apply`, `material.set`,
`settings.set`, `settings.undo`, `build.web`, `serve.web`, `browser.capture`,
`browser.input`, and from the core `tx.apply`, `tx.undo`, `sim.build_encounter`,
`game.build`. Every one of those should return a `verify.checkpoint` verdict (or at least
the check ids that ran). The read-only verbs (`scene.brief`, `scene.spatial`, `scene.query`,
`scene.watch`, `scene.next`, `scene.pick`, `world.query`, `structure.analyze`,
`hit.simulate`, `wound.table.get`, `anim.graph.get`, `anim.preview`, `material.list`,
`material.get`, `material.schema`, `material.preview`, `material.param`, `material.compare`,
`asset.generate`, `geo.preview`, `view.*`, `net.loopback`, `perf.table`, `play.step_until`,
`play.sweep`, `seat.*`, `ai.decisions`, `replay.*`, `bug.replay`, `trace.summary`,
`verify.proof`, `physics.*`, `settings.get`, `rules.*`, `sim.plan_goals`, `sim.reset`,
`sim.step`, `api.*`, `capture`, `scene.read`, `play.run`) do not need one.

**Note.** `character.build` exists as a name in `crates/endpoint/src/build_character.rs` but
that file says it is "not an MCP tool; `character.build` is a reserved name" (read
2026-10-20) — it is not a registered verb today, so it is not on this list. Likewise
`project.build` / `project.new` / `project.run` appear only in
`docs/plan/finish-line.md` item 12; no such verb exists in the tree today (grep
2026-10-20).

### Spec 3 — the keypoint check needs a door path (AIX-1)

`crates/playtest/src/keypoint/inject.rs` is **not** reachable from the door today. How
playtest is reached today: the endpoint depends on `engine-playtest` under the `mcp`
feature and uses it in three places (grep 2026-10-21) -
`crates/endpoint/src/look_host.rs` (`engine_playtest::look::judge`),
`crates/endpoint/src/packs/view.rs` (`engine_playtest::look::metric`) and
`crates/endpoint/src/packs/verify.rs` under the off-by-default `verify` feature
(`engine_playtest::mirror`, `engine_playtest::mirror_kit`, `engine_playtest::replay`,
`engine_playtest::seeded_bug`). The `verify` pack is the natural home for a keypoint verb:
it already links the playtest crate and already owns the replay and proof verbs.

**Proposed door path for the endpoint lane:** a `verify.keypoint` verb in
`crates/endpoint/src/packs/verify.rs` (feature `verify`) that takes a keypoint spec id and
returns the `UnitVerdict` from `engine_playtest::keypoint::inject` — the keypoint id, the
pass flag, the reason lines and the digest. The same pack can carry the injected-unit
checker once a brief-to-unit builder lands; until then the verb needs a spec file the
caller names, and it must refuse a spec id the playtest crate does not know.

**Proof command** for the lane: `node tools/gh-check.mjs cargo test -p engine-endpoint
--features verify --lib keypoint`.

### Spec 4 — the editor-lane half of AIX-7 (depth and preservation)

`crates/editor/src/activity.rs` already owns `ActivityRing` and `SceneDiff` and already
renders a three-way diff, and its header says the same pure functions feed the future door
verbs `session.activity` and `scene.diff`. The depth label and the preservation list belong
beside `SceneDiff` there; the door verb belongs to the endpoint lane. Left as a note for
the lead: the two halves are one row, owned by two lanes.

---

# For the provider lane

**Owner:** the OPT27 lane. `crates/provider` is held; this wave lands no provider code.
Everything below is a spec, written so the owner can implement it without re-deriving it.

### Spec 1 — the bounded memory contract (AIX-3)

**File path.** `crates/provider/src/agent_loop.rs` — the bounded perceive-plan-act loop
already lives there with `LoopBudget`, `StepRecord`, `LoopOutcome`, `transcript_checkpoint`
and a `Planner` trait that "sees the transcript". The bounded memory replaces the raw
transcript append with a typed retriever; the loop's own types stay.

**Struct and function names.**

- `pub struct MemoryStore { entries: BTreeMap<String, MemoryEntry> }` — the bounded store.
- `pub struct MemoryEntry { pub key: String, pub value: String, pub written_at_tick: u64,
  pub digest: u64 }` — one entry, with the digest that makes a write checkable.
- `pub const MEMORY_PROMPT_BUDGET_BYTES: usize = 8 * 1024;` — the bound: the assembled
  prompt never exceeds it.
- `pub const MEMORY_MAX_ENTRIES: usize = 64;` — the store never holds more.
- `pub struct MemoryRetriever { store: MemoryStore }` with
  `pub fn assemble(&self, step: &StepRecord) -> String` (the fresh user message for one
  decision) and `pub fn layer_enabled(&self, layer: &str) -> bool` (the ablation switch).
- `pub struct WriteGate` with `pub fn admit(&self, claim: &MemoryClaim,
  predicate: &dyn Fn(&MemoryClaim) -> bool) -> Result<(), MemoryError>` — a claim becomes a
  fact only when the predicate agrees; otherwise it is refused with a named reason.

**The bound.** `assemble` returns at most `MEMORY_PROMPT_BUDGET_BYTES` bytes and
`MEMORY_MAX_ENTRIES` entries; when the selection would exceed the budget the lowest
priority entry is dropped first (the same drop rule the door's brief budget already uses),
and the drop is counted, not silent. The raw transcript is **never** appended to the
prompt: `assemble` reads the store and the current step, nothing else.

**Test names** (for the provider lane's own file, `crates/provider/src/agent_loop.rs`):

- `bounded_memory_keeps_the_prompt_under_the_bound` — 200 decisions, the assembled prompt
  never exceeds `MEMORY_PROMPT_BUDGET_BYTES`, and the drop count is reported.
- `memory_layers_ablate_one_at_a_time` — each layer can be switched off on its own and each
  ablation moves the score in a named direction.
- `write_gate_refuses_a_claim_the_predicate_disagrees_with` — a refused claim is absent
  from the next prompt and the refusal names its reason.
- `bounded_memory_gives_the_same_prompt_bytes_twice` — the same transcript gives the same
  prompt bytes.
- `the_raw_transcript_is_never_appended_to_the_prompt` — the prompt holds no raw
  cross-decision transcript.

**Proof command.** `node tools/gh-check.mjs cargo test -p engine-provider --lib bounded_memory`.

**AGPL-3.0 flag on the donor.** The donor repo `AlayaLab/AgenticSTS` is a monorepo with
three licences, read live 2026-10-20: `AgenticSTS/` **Apache-2.0** (the Python agent:
decision engine, memory, skills, tooling, scripts, leaderboard, docs),
`AgenticSTS-Mod/` **GNU AGPL v3.0** (the C# game mod, a fork of CharTyr/STS2-Agent whose
AGPL terms are inherited — the root LICENSE says the Apache-2.0 grant on `AgenticSTS/` does
**not** extend to it), and `AgenticSTS-Data/` **CC BY 4.0** (the released run archive and
frozen L4/L5 store snapshots; the NOTICE says the verbatim game data and decompiled extracts
are covered there and are regenerated locally rather than shipped). **We port the shape
only and vendor no bytes from any of the three parts.** If the provider lane ever wants the
donor's retriever or write-gate code, only the `AgenticSTS/` Apache-2.0 half is eligible, and
the AGPL-3.0 mod and the CC BY 4.0 game data stay out of the tree entirely.

### Spec 2 — the scope rule for the free-model budget (AIX-9, AIX-5)

**File path.** `crates/provider/src/router.rs` (the scorecard router that picks a backend
per task class) and `crates/provider/src/ladder.rs` (the structured-output ladder).

**Names.** `pub struct TaskClass { pub name: String, pub scope: Scope, pub max_prompt_bytes:
usize }` with `pub enum Scope { Narrow, Wide }`; `pub fn classify(name: &str) -> Option<
TaskClass>`; the router prefers the cheapest backend that holds for a `Narrow` class and
reserves the expensive one for `Wide`.

**The rule.** A narrow task (one verb, one schema, one decision) goes to the cheapest
backend that holds; a wide task (a scene edit with many coupled steps) goes up the ladder.
The rule is what keeps us inside the free-model budget, and AgentFloor (2605.00334) is the
paper that says where the free models stop.

**Test names.** `narrow_classes_route_to_the_cheapest_backend_that_holds`;
`wide_classes_still_reach_the_top_of_the_ladder`; `the_router_never_moves_mock_local_remote
_parity`.

**Proof command.** `node tools/gh-check.mjs cargo test -p engine-provider --lib scope_rule`.

**Donor licence note.** AgentFloor (2605.00334) released **no code** (GitHub repository
search returned 0 hits on 2026-10-20; the abs page carries no link), so there is nothing to
license or vendor; the ladder shape is reimplemented from the paper text. The paper's
GPT-5 comparison is the authors' benchmark, not ours.

---

## What stays open after this wave

- **The endpoint lane owns the wiring.** AIX-2 (the verify-checkpoint tool) and the
  `verify.keypoint` door path are specs, not code: `crates/endpoint` is held.
- **The provider lane owns the memory contract.** AIX-3 and the scope rule are specs:
  `crates/provider` is held.
- **AIX-8 and AIX-12 stay ideas.** Both need a training run or a model call per
  generation, and our budget is free-only.
- **AIX-11 is a correction to a landed row,** not a new row: the CraftBench-UE shape is
  already mirrored by HS30; only the paired-task rate is new.
- **No number in this file is our result.** Every paper figure here is the authors'
  benchmark, read on the abs page on 2026-10-20.
- **The playtest crate has three pre-existing red tests** (the `agent`,
  `spatial_eval::known_good` and `winrates` ones above). They are not this lane's and
  this lane did not fix them; the row that owns them is the one that should.
- **The arena and physics crates were red from other seats' in-flight work** while this
  lane proved its own work. The GitHub runner builds everyone's dirty tree, so the first
  two attempts failed on `crates/arena` (a missing `Sim.cuts` initializer) and
  `crates/physics` (an unread `hash_cell_of` field) before the owners landed their fixes.
