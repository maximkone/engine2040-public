# Fleet of weak coders on one tree: arXiv papers and agent frameworks (2026-10-22)

Scope: how to make many weak free models productive on one shared `master` tree, with GitHub proofs and a strong judge. Row prefix AGT. The 2026-10-17 arXiv wave (`docs/research/finish-line-2026-10-17/03-arxiv.md`) covers game art and rendering papers; nothing below repeats it.

## Verification note

- Read in full: arXiv 2603.21489 (CAID), 2405.15793 (SWE-agent), 2607.19338 (CodeRescue, see the withdrawal note), the abstract page of 2512.08296, and the OpenCode agents docs (`opencode.ai/docs/agents/`).
- Search summaries only (title, date and headline numbers as reported by search, not opened): every other id below. Numbers from those are marked "snippet".
- Not found, so not cited as evidence: FrugalGPT (2305.05176), RouteLLM (2406.18665), RepoGraph (2410.14684), OpenHands (2407.16741), CodeR.
- CodeRescue (2607.19338): the fetched page says the paper was later withdrawn. Its numbers are not used as evidence here.
- Error-amplification figures (17.2x, 4.4x) come from the Hugging Face abstract snippet; the arXiv abstract page I opened does not show them.

## 1) Verdict

**Is "a lead spawns 5, each spawns 5 weak coders, a reviewer in between" a good shape? No.** The papers do not support it.

What the papers support:
- **One central validator, workers that do not talk to each other.** 2512.08296 (Kim et al., Dec 2025) reports error amplification of 17.2x for independent agents and 4.4x for centralized ones (snippet). The orchestrator acting as the validation point is the stated mechanism. CAID (2603.21489, Geng and Neubig, Mar 2026, read in full) also uses one manager, a dependency-aware plan and test-based merging. Its reported gains over a single agent are +25.6 points on PaperBench and +14.7 on Commit0.
- **One level of workers.** Nothing I found shows a depth-3 tree paying off on coding. The search turned up BOAD (2512.23631), HyperAgent and MarsCode-style role splits, but none gives a depth ablation.

What the papers argue against:
- **25 leaf coders under 5 mid-level agents.** Each extra layer is one more place where errors pass through unchecked. 2512.08296 finds multi-agent designs lose 39-70% on sequential reasoning and gain only on parallel subtasks. Our rows are mostly coupled (same interfaces, same golden hash), so most rows are the sequential case.
- **Two agents on coupled work.** CooperBench (2601.13295, Jan 2026, snippet for the number): two agents on conflicting features score about 30% lower than one agent doing both. Failures were stale messages, broken promises, and wrong guesses about the partner's plan.
- **A single orchestrator at large scale.** Agensh (2609.26781, snippet) argues a central orchestrator caps how far a system scales. At our size (3-4 helpers per wave) the lead is not the limit. The board rows and git history already serve as shared state, so the lead should not relay messages between helpers.

**Worker-count curve: unknown.** I found no paper that measures how many coding workers on one repo pay off. SIMAS (2606.00655, Fudan, May 2026, snippet) finds non-monotonic returns, and 2512.08296 reports saturation once single-agent accuracy is high (the search summary says about 45%, not confirmed in full text). So the number has to be measured here: AGT19.

**Shape the papers support for us:**
lead (plans, splits rows by file set, runs the head gate) -> up to 3-4 helpers per wave, each on one independent row, each committing its own proven step -> deterministic gates (AGT02, AGT04, AGT07) -> strong judge reads the gate result and the diff, not the chat. No helper spawns helpers (`permission.task: deny` in the OpenCode helper config; the docs confirm `permission.task` controls which subagents an agent may call).

**Top five levers, by value per effort:**
1. Run the proof on the head after each helper commit, not only on the helper's own change (AGT02). Two changes can each pass alone and break each other (2609.25396; CooperBench).
2. Salvage a dying helper's diff into a fresh helper (AGT03). Directly prevents the wipe that happened tonight. Fail-Fast/RestartSmart (2608.03222, Aug 2026) reports that a fresh run given the interrupted diff as an optional overlay beats a cold restart: 71.8% vs 66.8% on SWE-bench Verified with Qwen3.6-27B at a 25% false-positive target.
3. Turn the AGENTS.md rules into a checked script (AGT04). SWE-Gate (2609.04167, Sep 2026): of 644 patches that passed functional tests, 221 (34.3%) broke review constraints.
4. Bounded retries, then escalate (AGT08). Cascade papers disagree. AutoMix (2310.12963) escalates on self-verification; CASTER (2601.19793) says cascades pay overhead and recommends sorting by difficulty up front. We do both: mechanical rows go to the cheapest model, code rows go to a weak model at most twice, then to the judge.
5. Edit rules by appended lines only (AGT11). ACE (2510.04618, ICLR 2026) reports that rewriting a context playbook wholesale causes context collapse; incremental updates avoid it.

**Premise check (decides which rows are live).**
- Owner memory `opencode-loop-retired` (2026-10-17) says the OpenCode sprint loop is being deleted, with no claims, packets or land.mjs. The brief and the repo notes still describe a 20-30 agent OpenCode fleet. `tools/` has no claim, lease or land script today, which fits the retirement. AGENTS.md (2026-10-22) still tells helpers to commit their own work. The rows below work for a small helper pattern too. Confirm which one is live before building AGT05, AGT08 and AGT19, which only matter when many workers run at once.
- Owner rule "one tree, no worktrees" rules out CAID's isolation step. We take its manager and dependency plan, not its worktrees.

## 2) Comparison table

| # | Source (id, year) | The one idea | How we use it | Effort | Row |
|---|---|---|---|---|---|
| 1 | Scaling Agent Systems, 2512.08296 (Dec 2025) | Central validator contains error amplification (4.4x vs 17.2x) | Lead gates each helper result; no helper-to-helper messages | S | AGT01 |
| 2 | Parallel agents, 2609.25396 (Sep 2026, abstract); CooperBench 2601.13295 (Jan 2026) | Parallel changes pass alone and fail together | Proof runs on the new head after each helper commit | S | AGT02 |
| 3 | Fail-Fast Restart-Smart, 2608.03222 (Aug 2026) | Restart with the interrupted diff as an overlay | Dead helper's diff saved as a patch file for the next helper | S | AGT03 |
| 4 | SWE-Gate, 2609.04167 (Sep 2026) | Functional tests miss 34% of review-constraint failures | Review rules as one script that fails red | S-M | AGT04 |
| 5 | AgentRoom 2608.23740 (Aug 2026); CooperBench 2601.13295 | Coordination (claims, status) matters more than parallelism | A row's file list is the claim; overlap refused at launch | S | AGT05 |
| 6 | CAID 2603.21489 (Mar 2026) | Central manager plus dependency-aware plan | Board rows name `depends on`; `--ready` hides blocked rows | S | AGT06 |
| 7 | Engineering Reliable Coding Agents 2608.13867 (Aug 2026) | Verification itself can be wrong; trust the gate only after a control | Seeded bad change must fail in every proof run | S | AGT07 |
| 8 | AutoMix 2310.12963 (2023); CASTER 2601.19793 (Jan 2026); SLMs 2506.02153 (Jun 2025) | Cheap first for simple calls; escalate after failed checks | Mechanical rows to cheapest model; 2 red gates then judge; `steps` cap | S | AGT08 |
| 9 | Agentless 2407.01489 (Jul 2024; FSE 2025) | Fixed order: localize, repair, validate; no agent loop | One skill file with a fixed helper recipe, at most 2 patches | S | AGT09 |
| 10 | AlphaCodium 2401.08500 (Jan 2024); MASAI 2406.11638 (Jun 2024) | Test-first iterative flow with bounded rounds | 3 fix rounds per row, then escalate | S | AGT10 |
| 11 | ACE 2510.04618 (ICLR 2026) | Incremental playbook updates avoid context collapse | AGENTS.md and skills change by appended lines only | S | AGT11 |
| 12 | ACON 2510.00615 (Oct 2025) | Compression guidelines keep what must survive | 5-line handoff note with a keep-list after each step | S | AGT12 |
| 13 | SWE-Gym 2412.21139 (Dec 2024); SWE-Lego 2601.01426 (Jan 2026) | Yes/no verifier beats regression scoring | Judge answers yes or no per candidate after the gate | M | AGT13 |
| 14 | MAST 2503.13657 (Mar 2025) | Failure taxonomy: spec, inter-agent, verification | Each red gate gets one tag | S | AGT14 |
| 15 | LocAgent 2503.09089 (ACL 2025); AutoCodeRover 2404.05427 (Apr 2024) | Code graph (imports, calls) for localization | Row file lists come from a `callers_of` query | M | AGT15 |
| 16 | A-Mem 2502.12110 (NeurIPS 2025); Subtask memory 2602.21611 (ICML 2026) | Memory at the subtask level, not whole episodes | Flat file of verified fixes keyed by error code | M | AGT16 |
| 17 | Agentic Harness Engineering 2604.25850 (May 2026) | Each rule edit carries a prediction checked next round | Rule log with predicted and observed effect | M | AGT17 |
| 18 | SWE-agent 2405.15793 (May 2024, read) | Agent-computer interface: short, paged tool output | Door verbs capped and paged | S | AGT18 |
| 19 | Scaling Agent Systems 2512.08296; SIMAS 2606.00655 (Jun 2026) | Returns are non-monotonic; no curve in the literature | Same 10 rows at 1, 2, 4 helpers; measure | M | AGT19 |

Repos (not opened unless stated): CAID github.com/JiayiGeng/CAID; AlphaCodium github.com/Codium-ai/AlphaCodium; SWE-Gate github.com/DeepSoftwareAnalytics/SWE-Gate; ACE github.com/ace-agent/ace; ACON github.com/microsoft/acon; agentpause github.com/Champoleello/agentpause (README self-reports zero 429 errors in a 20-step test; treat as an idea only).

Effort: S = a Haiku helper with one test, under a day. M = two to four days, or needs a real run that costs quota.

## 3) Ranked steals

Each item: the source, the one idea, how we apply it (tool, AGENTS.md, or OpenCode config), effort, and the row.

### AGT01 Central validator, one helper level (2512.08296, snippet for the numbers)
- Idea: independent workers amplify errors; a central validator keeps them low.
- Apply: AGENTS.md rule: helpers do not spawn helpers; the lead accepts a helper's commit only after the gate passes on the new head. OpenCode: helper agent config sets `permission.task` to `deny`.
- Effort: S.
- Row: `AGT01 fleet: one helper level only; helper agents deny the task tool; the lead alone accepts commits. Accept: a test spawns a helper configured with permission.task deny and it has no subagent tool.`

### AGT02 Head gate after every helper commit (2609.25396 abstract; CooperBench 2601.13295)
- Idea: a change that passes alone can break a sibling change after merge. Benchmark "stale" measures this (abstract only read).
- Apply: `tools/gh-check.mjs` runs on the new master head after each helper commit; a red head is fixed by a new lead commit, never by reset or revert of someone else's edit.
- Effort: S.
- Row: `AGT02 head gate: the proof runs on the head after every helper commit. Accept: a seeded pair of changes that pass alone and break an interface is caught by the head run.`

### AGT03 Salvage, do not resume (2608.03222)
- Idea: a fresh run given the failed run's diff as an optional overlay beats a cold restart. The monitor (0.6B model) needs trajectory data; skip the monitor, keep the restart.
- Apply: new read-only `tools/salvage.mjs` writes `git diff -- <row files>` of a dead helper to the scratch folder. The next helper's prompt names that file. No stash, clean or reset. "Commit as you go" in AGENTS.md already limits the loss.
- Effort: S.
- Row: `AGT03 salvage: a helper that dies mid-row leaves its diff as a patch file; the next prompt names it. Accept: a test kills a helper mid-row; the new prompt has the patch path and git status still shows the diff.`

### AGT04 Review gate as a script (2609.04167)
- Idea: passing functional tests is not enough; 34% of passes broke review constraints.
- Apply: one `tools/review-gate.mjs` checks the AGENTS.md rules: no stubs, no `#[allow]`, tests first for behavior rows, golden hash unchanged unless the row says so. Reuse `tools/arch-fitness.mjs` and `tools/beat-lint.mjs` (both exist).
- Effort: S-M.
- Row: `AGT04 review gate: AGENTS.md rules run as one script. Accept: it fails a seeded diff with an allow attribute and a stub, and passes the last 20 landed commits.`

### AGT05 Leases by plan row, serial when overlapping (2601.13295; AgentRoom 2608.23740)
- Idea: coordination (claims, status) carries the gain; AgentRoom's ablation shows losing the coordination tools loses most of it. Its CRDT layer is not needed for us.
- Apply: no claim tool (owner retired claims). The row's file list is the claim. The lead refuses to launch two helpers whose file lists overlap, and marks overlapping rows serial. Shared types and signatures go to a lead-owned row.
- Effort: S.
- Row: `AGT05 leases: two helpers never hold a shared file; overlap is refused at launch and the second row runs after. Accept: a test with two rows sharing one file refuses the second launch.`

### AGT06 Dependency plan from the manager (CAID 2603.21489, read)
- Idea: a central manager writes a dependency-aware plan; independent parts run concurrently; integration is checked by tests. +25.6 PaperBench, +14.7 Commit0 over one agent.
- Apply: the lead writes `depends on <ID>` in board rows. `tools/board-slice.mjs --ready` hides a row until its dependencies show Landed. Check the flag exists before relying on it (AGENTS.md names `--ready`).
- Effort: S.
- Row: `AGT06 deps: board rows name depends on; --ready hides a row until its deps land. Accept: a test with a row whose dependency is open is absent from --ready.`

### AGT07 Gate self-test (2608.13867, single-author monograph; not peer reviewed)
- Idea: flawed verification invalidates results downstream; check the check.
- Apply: every `gh-check` proof run includes one seeded bad change that must fail. A green run without the seeded failure is treated as red.
- Effort: S.
- Row: `AGT07 gate trust: every proof run includes one seeded bad change that must fail. Accept: a seeded run is red; the CI log shows the seeded failure line each run.`

### AGT08 Cheap first, escalate after two reds (2310.12963; 2601.19793; 2506.02153)
- Idea: a small model handles simple calls and hands over on failure (AutoMix); sort by difficulty first to avoid paying for failed attempts (CASTER); small models suit many agent calls, with a strong model for judgement (2506.02153, NVIDIA, Jun 2025).
- Apply: OpenCode per-agent `model`, and `steps` cap on each agent (verified in the docs: when the cap is hit, the agent is told to summarize and list what is left). AGENTS.md rule: board edits and log summaries to the cheapest model; code rows to a weak model with at most two red gates, then the judge takes the row.
- Effort: S.
- Row: `AGT08 cascade: two red gates move a row to the judge; every agent config has steps. Accept: the agent configs carry steps and the wave log counts escalations per row.`

### AGT09 Fixed recipe, no open agent loop (Agentless 2407.01489)
- Idea: a fixed order (localize, repair, validate) with sampled patches and test-based reranking beat open agent loops on SWE-bench Lite at about $0.70 per issue (snippet; FSE listing gives 32.67%).
- Apply: one skill file under `skills/` with the recipe: locate (modmap or `callers_of`), failing test, one patch, gate. At most two patches per row. Not the whole pipeline: our rows are Rust sim, feel and art work, not single-issue Python bug fixes.
- Effort: S.
- Row: `AGT09 recipe: helpers follow one skill: locate, test, patch, gate; at most 2 patches per row. Accept: one board row done end to end by a weak model via the recipe; gate green; the row lands with its sha.`

### AGT10 Bounded fix rounds (AlphaCodium 2401.08500; MASAI 2406.11638)
- Idea: a test-first iterative flow lifts results (AlphaCodium: GPT-4 pass@5 from 19% to 44%, paper summary). MASAI splits roles (reproducer, localizer, fixer); its 28.33% on Lite and under $2 per issue come from the snippet, and its role descriptions come from a secondary summary.
- Apply: the AGT09 recipe caps fix rounds at 3 per row; the gate log records the round count; a fourth round goes to the judge.
- Effort: S.
- Row: `AGT10 loop bound: 3 fix rounds per row, counted in the gate log. Accept: the recipe names the cap and a log test shows a fourth round is routed to the judge.`

### AGT11 Append-only rules (ACE 2510.04618, ICLR 2026)
- Idea: a playbook that is only appended avoids brevity bias and context collapse; ACE matches a top production agent on AppWorld with a smaller open model (paper abstract, read via search).
- Apply: a commit that touches AGENTS.md or `skills/` may only add lines unless the lead signs it. A check on the diff enforces it.
- Effort: S.
- Row: `AGT11 append rules: a helper commit to AGENTS.md or skills may only add lines. Accept: a test with a removed line is red; an added-line diff is green.`

### AGT12 Handoff keep-list (ACON 2510.00615)
- Idea: compress observations and history with explicit guidelines; keep what must survive. Peak tokens drop 26-54% (abstract, via search); a smaller compressor model works.
- Apply: each helper step ends with a five-line note in scratch: row id, files touched, last gate sha, open question, rejected idea. A fresh helper given only the note picks up the next step.
- Effort: S.
- Row: `AGT12 handoff: each step ends with a five-line note. Accept: a dry run where a fresh helper gets only the note and reaches the next step.`

### AGT13 Yes-or-no candidate pick (SWE-Gym 2412.21139; SWE-Lego 2601.01426)
- Idea: a reward model picks the best of sampled trajectories (SWE-Gym: 32.0%, +11.4 points). A generative yes/no verifier beats a regression head at 8B and 32B (SWE-Lego). We do not train; we copy the verdict form.
- Apply: when a helper gives two patches, the judge answers yes or no per patch, reading only patches that pass the gate.
- Effort: M.
- Row: `AGT13 pick: the judge answers yes or no per gate-passing patch. Accept: on 10 past rows the pick matches the landed patch in at least 7; the numbers are logged.`

### AGT14 Failure tags (MAST 2503.13657)
- Idea: 14 failure modes in 3 groups (system design, inter-agent misalignment, task verification); inter-annotator kappa 0.88 (per the paper's later versions).
- Apply: each red gate line gets one tag: spec, inter-agent, verification, or system.
- Effort: S.
- Row: `AGT14 tags: every red gate line carries one tag. Accept: after 20 red gates, a count per tag is in the wave log.`

### AGT15 Graph localization (LocAgent 2503.09089; AutoCodeRover 2404.05427)
- Idea: a graph of files, classes, functions with imports, calls and inheritance; localization by graph queries. LocAgent indexes in seconds (abstract). AutoCodeRover uses program structure, not string search (its Lite numbers 19% at @1 and 26% at @3 are from the search summary).
- Apply: `endpoint/src/insight` already has `callers_of` (the 2026-10-17 research says so; confirm before use). Row file lists (five at most) come from a `callers_of` query, not from guesses. `.opencode/modmap.md` covers crate files.
- Effort: M.
- Row: `AGT15 localize: a row's file list (five at most) comes from a callers_of query. Accept: a test shows the prompt list equals the query output for one row.`

### AGT16 Fix memory keyed by error code (A-Mem 2502.12110; subtask memory 2602.21611; Mem0 2504.19413)
- Idea: memory at the level of subtasks helps (2602.21611 reports +4.7 points mean Pass@1 on SWE-bench Verified; +6.8 on Gemini 2.5 Pro; snippet). Graph memory stores are heavier than we need.
- Apply: a flat file of verified fixes keyed by error code. `docs/BLOCKERS.md` exists and is a candidate home; a separate file keeps it clean. Helper prompts include lines whose code matches.
- Effort: M.
- Row: `AGT16 fix memory: each verified fix adds one line keyed by error code; prompts include matching lines. Accept: a seeded error code finds its line in a test.`

### AGT17 Rule predictions (Agentic Harness Engineering 2604.25850)
- Idea: each harness edit comes with a prediction, checked on the next round. Terminal-Bench 2 pass@1 goes 69.7% to 77.0% over ten iterations. Secondary summaries flag weak regression prediction ("regression blindness").
- Apply: each AGENTS.md change in the wave log has a predicted effect; the next wave's red and green counts go beside it.
- Effort: M.
- Row: `AGT17 rule log: each rule change has a predicted effect and the next wave's result. Accept: after two waves the log has one row per change.`

### AGT18 Door output caps (SWE-agent 2405.15793, read)
- Idea: a tool interface built for the model (short, paged output) matters more than the model. SWE-agent reports pass@1 12.5% on SWE-bench and 87.7% on HumanEvalFix in its abstract.
- Apply: MCP door verbs return at most 200 lines plus a next cursor. Check the largest verb first.
- Effort: S.
- Row: `AGT18 door caps: every door verb returns at most 200 lines and a next cursor. Accept: a test on the largest verb cuts a 10,000-line result and returns the cursor.`

### AGT19 Wave size, measured here (2512.08296; 2606.00655)
- Idea: no paper measures the coding worker-count curve on one repo. Returns are non-monotonic (SIMAS) and saturate (2512.08296).
- Apply: run the same ten rows at 1, 2 and 4 helpers; count landed rows and gate minutes. Costs quota, so run it once the head gate and salvage rows are in.
- Effort: M.
- Row: `AGT19 wave size: the same 10 rows at 1, 2 and 4 helpers; landed rows and gate minutes recorded. Accept: a table with three rows in docs/research/.`

## 4) What NOT to copy, and why

- **CAID worktree isolation.** Owner rule: one tree. Take the manager and the dependency plan only (AGT06).
- **AgentRoom CRDT and MCP claim server.** Heavy infrastructure. Keep the lesson (AGT05), skip the stack.
- **SWE-Search MCTS (2410.20285).** Needs many rollouts and a value model; our binding limit is quota, not ideas.
- **Deep hierarchies (BOAD 2512.23631; HyperAgent; role-split systems with five or more levels).** No evidence of gain past one level, and every level adds unchecked hand-offs.
- **Multi-agent debate (CascadeDebate 2604.12262; SWE-Search discriminator).** Costs calls for little measured gain.
- **Trained routers, monitors and orchestrators.** CodeRescue's router (withdrawn, see above), Fail-Fast's 0.6B monitor, SWE-Gym and SWE-Lego training, and the 100-300 sub-agent RL orchestrators in the 2605.02801 survey. We have free models only and no training budget.
- **Vendor numbers.** The CooperBench figures quoted on a coordination-tool vendor's page, and agentpause's self-reported zero-429 test. Re-measure before quoting.
- **Graph memory databases.** A flat file keyed by error code does the job (AGT16).
- **Agentless as a whole pipeline.** SWE-bench is single-issue Python bug repair. Our rows are Rust simulation, feel and art work with GitHub proofs. Take the order, not the pipeline (AGT09).
- **Multi-agent on coupled rows.** The sequential penalty and CooperBench both say no. Serial by default; parallel only on disjoint file sets (AGT05).
- **More helpers before AGT19.** Add helpers only after the wave-size table shows a gain.

## 5) Open questions and what I could not confirm

- The worker-count curve for coding (AGT19) is the main gap. No source found.
- Error-amplification numbers (17.2x, 4.4x) and the saturation point (about 45%) are from search summaries. The paper's full text was not readable through the fetch tool (PDF binary).
- SWE-agent figures come from its abstract; OpenHands was not checked (id 2407.16741 not fetched).
- Mem0 (2504.19413): the abstract was not read.
- The owner's loop-retirement note conflicts with the brief's fleet premise. Confirm before building AGT05, AGT08 and AGT19.

## Sources (arXiv unless marked)

- 2512.08296 Towards a Science of Scaling Agent Systems: https://arxiv.org/abs/2512.08296
- 2603.21489 Effective Strategies for Asynchronous Software Engineering Agents: https://arxiv.org/abs/2603.21489
- 2601.13295 CooperBench: https://arxiv.org/abs/2601.13295
- 2609.25396 Passes Alone, Fails Together: https://arxiv.org/pdf/2609.25396
- 2608.03222 Fail-Fast, Restart-Smart: https://arxiv.org/pdf/2608.03222
- 2609.04167 SWE-Gate: https://arxiv.org/pdf/2609.04167
- 2608.23740 AgentRoom: https://arxiv.org/pdf/2608.23740
- 2608.13867 Engineering Reliable Coding Agents: https://arxiv.org/pdf/2608.13867
- 2601.19793 CASTER: https://arxiv.org/pdf/2601.19793
- 2310.12963 AutoMix: https://arxiv.org/pdf/2310.12963
- 2506.02153 Small Language Models are the Future of Agentic AI: https://arxiv.org/pdf/2506.02153
- 2407.01489 Agentless: https://arxiv.org/abs/2407.01489
- 2401.08500 AlphaCodium: https://arxiv.org/pdf/2401.08500
- 2406.11638 MASAI: https://arxiv.org/pdf/2406.11638
- 2510.04618 Agentic Context Engineering: https://arxiv.org/pdf/2510.04618
- 2510.00615 ACON: https://arxiv.org/pdf/2510.00615
- 2412.21139 SWE-Gym: https://arxiv.org/pdf/2412.21139
- 2601.01426 SWE-Lego: https://arxiv.org/pdf/2601.01426
- 2503.13657 Why Do Multi-Agent LLM Systems Fail?: https://arxiv.org/html/2503.13657v2
- 2503.09089 LocAgent: https://arxiv.org/pdf/2503.09089
- 2404.05427 AutoCodeRover: https://arxiv.org/pdf/2404.05427
- 2502.12110 A-Mem: https://arxiv.org/pdf/2502.12110v9
- 2504.19413 Mem0: https://arxiv.org/pdf/2504.19413
- 2602.21611 Structurally Aligned Subtask-Level Memory: https://arxiv.org/pdf/2602.21611
- 2604.25850 Agentic Harness Engineering: https://arxiv.org/pdf/2604.25850
- 2405.15793 SWE-agent: https://arxiv.org/abs/2405.15793
- 2607.19338 CodeRescue (withdrawn per fetched page): https://arxiv.org/abs/2607.19338
- 2606.00655 Scaling Behavior of Single LLM-Driven Multi-Agent Systems: https://arxiv.org/pdf/2606.00655
- 2609.26781 Agensh (snippet): https://arxiv.org/pdf/2609.26781
- 2410.20285 SWE-Search: https://arxiv.org/pdf/2410.20285
- 2512.23631 BOAD: https://arxiv.org/html/2512.23631
- OpenCode agents docs: https://opencode.ai/docs/agents/
