# Proposed rows - AI-native and MCP (powerup wave, 2026-10-20)

Lane: AI-native / MCP. Source file: `docs/research/powerups-2026-10-20/04-ai-native-mcp-steals.md`
(12 ranked finds, donor paths read live on 2026-10-20). Board ids below are the
engine2040 row each find closes: `D16`-`D20` are gap items 16-20 of
`docs/plan/finish-line.md`; `AX28`, `HS20`, `NS13`, `OPT29`, `FU2` are live rows
on `docs/SPRINT.md` (read with `node tools/board-slice.mjs <ID>`).

**PORT-TYPE, every row:** shape into Rust, no vendored bytes. We read the donor
file, then write our own code in our own style with our own names. No donor
source is copied, no donor crate is added to `Cargo.toml`, and no donor licence
travels with our code. A donor that needs a paid or closed model is marked
RESEARCH-ONLY and is not ported.

**Status vocabulary (2026-10-21).** LANDED: the row's shape is in the tree and
its accept command prints its line. PARTLY: part of the row is in the tree, the
rest is named as still open. OPEN: nothing landed; the row keeps its accept
command as the next step, not as a proof.

**Lanes and collisions.** Every row names the crate it lands in. This lane owns
`crates/endpoint` only. Rows that want another crate are listed as
`handoff:` and are not implemented here:
- `crates/spatial` belongs to the smart-engine lane (spatial checks).
- `crates/playtest` belongs to the arXiv lane (`keypoint.rs` is landing today).
- `crates/render` belongs to GS44, `crates/assets` and `crates/game` to WEB6,
  `crates/anim` to AM24, `crates/provider` to OPT27.
- `crates/editor` is shared: `crates/editor/src/activity.rs` already owns
  `ActivityRing` + `SceneDiff` (ED20), so D17 reads it, it does not rewrite it.

---

## MCX-1 - game-agent tasks scored by a runtime probe, not a judge

**What it is.** 81 executable game-engine repair tasks where the reward comes
only from a deterministic native probe: the probe runs in the real engine, prints
sentinel lines, and the pass/fail is a binary reward. No LLM judge sits in the
scoring path.
**Donor.** `GamePhanesStudio/GamePhanes` (GameForgeBench),
https://github.com/GamePhanesStudio/GamePhanes, read live 2026-10-20. MIT
(LICENSE read live, `Copyright (c) 2026 GamePhanes contributors`). Files read
live: `release/tasks/godot-replay-orbit-camera/rubric.yaml`,
`release/tasks/godot-replay-orbit-camera/tests/orbit_camera_probe.gd`,
`release/tasks/godot-replay-orbit-camera/tests/test_outputs.py`,
`release/tasks/godot-replay-orbit-camera/tests/test.sh`,
`release/tasks/godot-replay-orbit-camera/task.toml`.
**Row closed.** D19 (verifier output in every build verb), D18 (task suite
scored by programmatic referees), HS20 (public benchmark mirror).
**PORT-TYPE.** Shape into Rust, no vendored bytes: the probe shape becomes our
own check rows, not the donor's GDScript.
**Status.** LANDED (2026-10-21). The probe shape is in
`crates/endpoint/src/verify_tasks.rs` (new): a per-verb check registry, a
sentinel-style check row per task, and a binary reward per task. The GCA
verify-tool shape was folded in here, so `crates/endpoint/src/audit/verify.rs`
was left alone (it holds the frozen audit-log seal checker) and the existing
`crates/endpoint/src/packs/verify.rs` (the AD8 proof pack, behind the `verify`
feature) was left alone too: the checks ride the three real build verbs
instead of a new pack.
**Landed in.** `crates/endpoint` (new `crates/endpoint/src/verify_tasks.rs`:
per-verb check registry, probe-style sentinel checks, binary reward per task;
the `verify` field rides `BuildLevelOutput`, `GameBuildOutput` and
`SimBuildEncounterOutput` as one string: the bucket first
(`bucket=resolved`), then one check per line (`name=verdict`, or
`name=fail:reason` when the check named a reason)). The shape was measured
before it was frozen: as a nested object it cost 290 chars per build tool
(871 for the three), as an array of strings 78, and the door's view-loaded
list had only 5 chars of headroom over its 2,048 floor, so the door paid for
one string, about 30 chars per build tool).
**Owned paths.** `crates/endpoint/src/verify_tasks.rs` (new),
`crates/endpoint/src/mcp_tools.rs`, `crates/endpoint/src/build_level/imp/door.rs`,
`crates/endpoint/src/lib.rs`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib verify_task`
prints `verify_task: 3 tasks, 1 reward each, judge-free PASS` and exits 0.
**Kill line.** The check needs a human to read the frame to decide pass/fail, or
the score needs an LLM judge, or one task's check cannot run headless.
**Cost.** M.

## MCX-2 - a build gate plus a rubric plus replayable traces

**What it is.** 140 game-generation tasks; each ships a `build_check` command
that gates the score, a hidden rubric with per-requirement aggregation
(`max` over demos for mechanics, `mean` for visuals), and replayable input
traces the verifier re-runs to record gameplay evidence.
**Donor.** `FreedomIntelligence/gamecraft-bench`,
https://github.com/FreedomIntelligence/gamecraft-bench, read live 2026-10-20.
Apache-2.0 (LICENSE read live). Files read live: `tasks/example/tests/rubric.json`,
`gamecraft_bench/verifier/score.py`, `gamecraft_bench/verifier/replay.py`,
`tasks/example/task.toml`.
**Row closed.** D18, D19, HS20.
**PORT-TYPE.** Shape into Rust, no vendored bytes: the gate shape and the
bucket shape become our own; the donor's Python verifier is not vendored.
**Status.** PARTLY (2026-10-21). The gate half landed: a build that returns
checks is graded per instance into a named bucket with a binary reward, judge
free. Still open: the rubric aggregation (`max` over demos for mechanics,
`mean` for visuals) and the replayable input traces the verifier re-runs. The
judge half stays RESEARCH-ONLY: it needs OpenAI or Anthropic keys and the
FREE-models-only rule forbids them, so no judge call exists in the crate.
**Landed in.** `crates/endpoint` (`verify_tasks.rs`: the gate half and the
bucket-and-verdict shape; the judge half is RESEARCH-ONLY).
**Owned paths.** `crates/endpoint/src/verify_tasks.rs` (new),
`crates/endpoint/src/mcp_tools.rs`, `crates/endpoint/src/build_level/imp/door.rs`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib verify_task`
prints `verify_task: 3 tasks, 1 reward each, judge-free PASS` and exits 0; the
same test scans the module's own production code and refuses any model
endpoint, so the judge path is not reachable from a free-model run.
**Kill line.** The only way to a number is the paid judge, or the gate cannot
run without a display.
**Cost.** M.

## MCX-3 - a Rust engine whose every edit re-runs the recorded session

**What it is.** An agent-driven Rust engine where every file format is text
(so an agent can read, diff and edit it), every edit re-runs the session from
tick 0 to the current tick on the recorded input, and the whole format set ships
as JSON Schema generated from the types.
**Donor.** `wd30x/loom-engine`, https://github.com/wd30x/loom-engine, read live
2026-10-20. MIT (`LICENSE-MIT` read live) and Apache-2.0 (`LICENSE-APACHE` in
the root listing). Files read live: `crates/loom_mcp/src/server.rs`,
`crates/loom_engine/src/schema.rs`, `README.md`.
**Row closed.** D19 (every edit returns its own check), D16 (the four resources
and the five scenario prompts, served from the same text formats).
**PORT-TYPE.** Shape into Rust, no vendored bytes: the re-run shape becomes our
own checks; the donor's schema generator is not vendored.
**Status.** PARTLY (2026-10-21). The check half landed: every real build verb
returns its own check rows in its own answer (`level.build`, `game.build`,
`sim.build_encounter`; `character.build` is a reserved name and is not
registered, so it is not a task). Still open: the re-run-and-compare half, a
replayed edit landing the same scene digest as the blocking run.
**Landed in.** `crates/endpoint` (the re-run-and-check shape, in the endpoint's
own D19 verify path; the JSON Schema generator itself is a `crates/game` and
`crates/assets` concern and is a handoff).
**Owned paths.** `crates/endpoint/src/verify_tasks.rs` (new),
`crates/endpoint/src/mcp_tools.rs`, `crates/endpoint/src/build_level/imp/door.rs`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib verify_task`
prints `verify_task: 3 tasks, 1 reward each, judge-free PASS` and exits 0; the
replayed-edit-same-digest half is not built yet, so that clause stays open.
**Kill line.** The re-run costs more than the edit, or the schema cannot be
generated from our types.
**Cost.** M.

## MCX-4 - read-only views served as resources, not as tools

**What it is.** The gateway promotes read-only registry, diagnostics and
catalogue views out of `tools/list` and into MCP resources, so a client that
needs the view fetches it on demand with `resources/read` instead of paying for
tool definitions in every `tools/list`. One resource family per module, about
ten lines each, and `resources/list` names only the root pointers.
**Donor.** `dcc-mcp/dcc-mcp-core`,
https://github.com/dcc-mcp/dcc-mcp-core, read live 2026-10-20. MIT (LICENSE read
live). Files read live:
`crates/dcc-mcp-gateway/src/gateway/native_resources/mod.rs`,
`crates/dcc-mcp-gateway/src/gateway/native_resources/instances.rs`,
`crates/dcc-mcp-gateway/src/gateway/native_resources/catalog.rs`.
**Row closed.** D16 (the four resources: crate map, door reference, error
catalog, `game.toml` schema) and the tool-list budget that AX15 pins.
**PORT-TYPE.** Shape into Rust, no vendored bytes: four resources, our own
content, no donor file copied.
**Status.** LANDED (2026-10-21). `crates/endpoint/src/resources.rs` (new)
serves four resources over `resources/list` and `resources/read`:
`e2040://crate-map` (all 42 workspace crates, one honest line each),
`e2040://door-reference` (the tool reference rows the door already builds),
`e2040://error-catalog` (44 curated rows, each one checked against the
endpoint's own inline error literals) and `e2040://game-toml-schema`. The
resources cost zero tool-list budget: they are not tools.
**Landed in.** `crates/endpoint` (new `crates/endpoint/src/resources.rs`: the
four resources, one module each, root pointers only; served over the existing
`resources/list` and `resources/read`).
**Owned paths.** `crates/endpoint/src/resources.rs` (new),
`crates/endpoint/src/mcp.rs` (capabilities plus the resource handlers; no new
tool), `crates/endpoint/src/lib.rs`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib door_resource`
prints `door_resource: 4 resources, 0 tool-budget chars PASS` and exits 0; the
default tool list stays at or below its measured size.
**Kill line.** A resource cannot be fetched without a tool call, or serving the
four resources moves the tool list at all.
**Cost.** S.

## MCX-5 - a bounded, compact projection for a long catalogue

**What it is.** The discovery list of a large catalogue is projected to a
compact field set, a bounded page size and a bounded summary width, and the
degradation is measured: a shorter summary saved about 21% per page but forced
an extra round trip that cost more than the tokens saved, so the page was
bounded instead of the row.
**Donor.** `dcc-mcp/dcc-mcp-core` (same repo, different file),
https://github.com/dcc-mcp/dcc-mcp-core, read live 2026-10-20. MIT. File read
live: `crates/dcc-mcp-skills/src/catalog/list_projection.rs`.
**Row closed.** D16 (the tool-text budget half: our list is 66,500 chars with a
2,048-char floor), OPT29 (agent onboarding budget).
**PORT-TYPE.** Shape into Rust, no vendored bytes: a projection measured in our
own chars, not the donor's page shape.
**Status.** OPEN (2026-10-21). No new code landed for this lane. The door
already projects its own catalogue and pins the list budget; what this lane did
is re-measure that list before and after its own changes and keep both the
default list and the view-loaded list under the budget with the floor (see
MCX-10 for the numbers). A compact projection of the catalogue rows themselves
is still to be built.
**Landed in.** `crates/endpoint` (would land in `crates/endpoint/src/insight`;
nothing landed in this lane).
**Owned paths.** nothing landed; the shape would live in
`crates/endpoint/src/insight/index.rs`, `crates/endpoint/src/insight/search.rs`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib door_v2`
prints the default list and the loaded list, both under 66,500 with at least
2,048 headroom, and exits 0.
**Kill line.** The projection makes the agent make one more round trip than it
would have made anyway, so the tokens it saves are negative.
**Cost.** S.

## MCX-6 - cursor pagination on every list method, and the progress shape

**What it is.** Cursor-based pagination on `tools/list`, `resources/list`,
`prompts/list` and `resources/templates/list` with an opaque `nextCursor`, plus
the progress-notification and cancellation shapes: a progress token, a progress
number, an optional total and an optional message.
**Donor.** `microsoft/mcp-for-beginners`,
https://github.com/microsoft/mcp-for-beginners, read live 2026-10-20. MIT
(LICENSE read live). Files read live:
`04-PracticalImplementation/pagination/README.md`,
`05-AdvancedTopics/mcp-protocol-features/README.md`.
**Row closed.** D16 (progress notifications on long calls; the four resources
listed, not tool-listed), AX28 (the cancel half).
**PORT-TYPE.** Shape into Rust, no vendored bytes: the progress shape becomes
our own notification; the donor's SDK examples are not vendored.
**Status.** PARTLY (2026-10-21). The progress half landed: a long call
(`level.build`, `game.build`, `sim.build_encounter`, `play.run`) sends one
progress note before it and one after it, carrying the client's own progress
token, the progress number and a message; a client that sent no token gets
nothing, and the note costs zero tool-list budget because it is a notification
and not a tool. Still open: the cursor half, an opaque `nextCursor` on
`tools/list`, `resources/list`, `prompts/list` and
`resources/templates/list`.
**Landed in.** `crates/endpoint` (the progress notification on the long calls;
the cursor on the door's own lists is still open).
**Owned paths.** `crates/endpoint/src/mcp.rs` (the progress note, the pure
payload builder and the capabilities), `crates/endpoint/src/play.rs` (the cancel
half of AX28).
**Accept.** `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib progress_note`
prints `progress_note: progress notifications sent, 0 tool-budget chars PASS` and
exits 0.
**Kill line.** The client cannot carry a progress token, or the notification
cannot be sent without a tool that costs more than the notification saves.
**Cost.** S.

## MCX-7 - the tasks extension, and the handshake that gates it

**What it is.** The final MCP tasks extension: a server may answer
`tools/call` with an asynchronous task handle, the client polls `tasks/get`,
may cancel with `tasks/cancel`, the discriminator is `resultType: "task"`, and
the client signals support by declaring the extension in its per-request
capabilities. A server that returns a task to a client that did not declare the
extension is an error, not a task.
**Donor.** `modelcontextprotocol/modelcontextprotocol`,
https://github.com/modelcontextprotocol/modelcontextprotocol, file
`seps/2663-tasks-extension.md` read live 2026-10-20 (Apache-2.0 for new
contributions, MIT for older ones, per the LICENSE read live); and the worked
reference server `rohitg00/ai-engineering-from-scratch`,
https://github.com/rohitg00/ai-engineering-from-scratch, file
`certifications/mcpa/lessons/21-long-running-work-and-tasks/code/main.py` read
live 2026-10-20 (MIT, LICENSE read live): the capability check on `_meta`, the
`-32021` missing-capability error, the `-32022` unsupported-version error, the
task snapshot fields and the `tasks/get`, `tasks/update`, `tasks/cancel`
methods.
**Row closed.** AX28 (`play.run` as a cancellable MCP task).
**PORT-TYPE.** Shape into Rust, no vendored bytes: the handshake shape becomes
our own job registry and status vocabulary; the donor's Python server is not
vendored.
**Status.** PARTLY (2026-10-21). The AX28 half landed: `play.run` returns a job
handle, the job carries one of six status words, a cancel of a finished job is
refused by name, and the counters live on disk. The protocol-level task
extension did NOT land: no answer carries `resultType: "task"`, there is no
`tasks/get`, `tasks/update` or `tasks/cancel` method, and no per-request
capability check, so a client that did not declare the extension never receives
a task it cannot parse. The `play.run` job id is the handle we do return.
**Landed in.** `crates/endpoint` (`crates/endpoint/src/play.rs`: the job
handle, the status vocabulary, the disk-backed counters and the cancel
refusal; the door's call arm keeps start and poll).
**Owned paths.** `crates/endpoint/src/play.rs`, `crates/endpoint/src/mcp.rs`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib task_play_run`
prints `task_play_run: job handle returned, digest equal to blocking run, cancel
refused the job PASS` and exits 0.
**Kill line.** The digest of the task result differs from the blocking run, or
a client without the extension gets a task it cannot parse.
**Cost.** M.

## MCX-8 - one job registry, one status vocabulary, counters read from disk

**What it is.** A single registry owns every asynchronous job type, so an
untracked job type is a diagnosable state rather than a silent early return; the
status vocabulary and what counts as terminal live in one place; and a counter
that can be reconciled against the authoritative state on disk, so a cached zero
never wins over output that already exists.
**Donor.** `dcc-mcp/dcc-mcp-core`,
https://github.com/dcc-mcp/dcc-mcp-core, read live 2026-10-20. MIT. File read
live: `crates/dcc-mcp-job/src/poller.rs` (the six statuses, the terminal set, the
reported-versus-disk counter provenance, the poll contract, and the recorded
history of untracked job types and status queries that minted fresh jobs for
unknown ids).
**Row closed.** AX28 (`play.run` as one job with a status vocabulary and
disk-backed counters).
**PORT-TYPE.** Shape into Rust, no vendored bytes: one registry, our own
status words, our own counters file.
**Status.** LANDED (2026-10-21). `crates/endpoint/src/play.rs` owns one job
registry (`PlayJobs`), one status vocabulary (six words, `pending`, `running`,
`completed`, `failed`, `cancelled`, `expired`, with `as_str`, `parse` and
`is_terminal` in one place) and counters that are written to
`target/reports/play-jobs.json` on every transition and reconciled against that
file on the next start, so a cached zero never wins over output that already
exists. The answer names the counters file it wrote.
**Landed in.** `crates/endpoint` (`crates/endpoint/src/play.rs`: the job
registry, the status vocabulary, the disk-reconciled counters).
**Owned paths.** `crates/endpoint/src/play.rs`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib task_play_run`
prints `task_play_run: job handle returned, digest equal to blocking run, cancel
refused the job PASS` and exits 0; the same job status prints the same string
twice, and the counters round trip through the disk under their own path.
**Kill line.** Two job types answer two different status vocabularies, or a
cached counter disagrees with the file on disk.
**Cost.** S-M.

## MCX-9 - a merge window that decides a landing order, and a diff by role

**What it is.** A read-only cross-branch overlap oracle: per-symbol body hashes
between two refs, so a symbol changed on two arms is a true conflict and two
arms changing different symbols in one file is a textual risk; the landing order
is greedy fewest-conflicts-first with a deterministic tie-break. Beside it, a
diff whose unit is the statement, aligned on role and never on line number, and
whose absence is disclosed rather than silently empty.
**Donor.** `redhat-et/ripwire`, https://github.com/redhat-et/ripwire, read live
2026-10-20. Apache-2.0 (LICENSE read live). Files read live:
`src/mergescout.h`, `src/slicediff.h`, `src/editcheck.h`. C++23 donor: shape
only, our own Rust.
**Row closed.** FU2 (scene diff plus merge window), D19 (the edit-time contract
check), AX12 (recorded branch-merge fork receipts).
**PORT-TYPE.** Shape into Rust, no vendored bytes: a C++23 donor, shape only.
**Status.** PARTLY (2026-10-21). The diff half landed on the door side:
`scene.diff` in the new lazily loaded `session` pack takes a snapshot id and
answers only the deltas since it (added, removed, changed rows with their
revisions), computed from the journal the command service already keeps, with a
deterministic merge order. Still open: the merge-window half, the cross-branch
overlap oracle and the greedy landing order, which belong to the editor lane
(`crates/editor` owns the diff itself) and are a handoff.
**Landed in.** `crates/endpoint` (the door-side diff; the merge-window verdict
is a handoff for the editor lane).
**Owned paths.** `crates/endpoint/src/packs/session.rs` (new),
`crates/endpoint/src/packs/mod.rs` (the pack registry, 17 packs),
`crates/endpoint/src/mcp.rs` (the snapshot table).
**Accept.** `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib scene_diff`
prints `scene_diff: snapshot id then deltas only, merge order deterministic PASS`
and exits 0.
**Kill line.** The merge order is not deterministic, or the diff reports a
change where the dependence did not move.
**Cost.** M.

## MCX-10 - a token-budgeted, ranked repo map with a measured cost

**What it is.** Aider's repo map unbundled: exact token counts, a degradation
ladder that drops private symbols, then parameter names, then the lowest-ranked
files, with files ordered by score; and a benchmark that measures the tokens an
agent spends exploring before its first file edit, stopping the count at that
edit.
**Donor.** `fkenmar/atlas`, https://github.com/fkenmar/atlas, read live
2026-10-20. MIT (LICENSE read live). Files read live: `src/budget.rs`,
`src/rank.rs`, `benchmark/README.md`, `docs/FAQ.md`.
**Row closed.** OPT29 (tokens an outside agent spends from discover to its first
applied edit, with a budget and a ratchet), D18 (the project memory the door
serves), and the repo-map half of the door's own discovery.
**PORT-TYPE.** Shape into Rust, no vendored bytes: our own token count, our own
ranking.
**Status.** OPEN (2026-10-21). No code landed for the map or its benchmark. What
this lane did land is the measurement half of the door's own discovery: the tool
list is priced in chars before and after. Measured on the runner: the default
list was 56,737 chars at HEAD 37c33077 and is 56,897 after this lane (+160: the
D19 `verify` string on the three build outputs; the `session` pack and the
resources and prompts cost 0 chars of the default list). The `view`-loaded list
was 64,222 and is 64,382 (+160: the view pack is untouched by this lane, its
7,485 chars measured on both trees). The cap stays 66,500, so the loaded list
keeps 2,118 chars of headroom over the 2,048 floor. The repo map itself, its
degradation ladder and the benchmark that stops counting at the first applied
edit are still to be built.
**Landed in.** `crates/endpoint` (would land in `crates/endpoint/src/insight`;
only the measurement landed).
**Owned paths.** nothing landed; the shape would live in
`crates/endpoint/src/insight/index.rs`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib door_v2`
prints the default list and the loaded list, both under 66,500 with at least
2,048 headroom, and exits 0.
**Kill line.** The map needs a grammar we cannot licence-check into the
endpoint crate, or the measured cost shows no saving.
**Cost.** M.

## MCX-11 - every measurement is also a check

**What it is.** A diff that reports counts in named buckets that sum to the
total, so a change is never just a number, and an `expect` parameter that turns
the measurement into a verdict: the same call is a measurement without it and a
check with it.
**Donor.** `MalloyTheDev/aseprite-mcp`, https://github.com/MalloyTheDev/aseprite-mcp,
read live 2026-10-20. MIT (LICENSE read live). Files read live:
`src/aseprite_mcp/tools/inspect.py` (`assess_sprite` at line 519, `diff_sprites`
at line 992, the `expect` parameter at line 999, the four buckets and the
verdict at line 1120), `README.md`.
**Row closed.** D19 (verifier output in every build verb), FU2 (the scene diff
shows what changed in named buckets).
**PORT-TYPE.** Shape into Rust, no vendored bytes: our own buckets, our own
verdict, no donor Python.
**Status.** LANDED (2026-10-21). `crates/endpoint/src/verify_tasks.rs` grades a
build into one of eight named buckets (`completed`, `resolved`, `error`,
`unresolved`, `incomplete`, `infra_failure`, `ambiguous`, `empty_patch`) and
names the failing checks, so a change is never just a number; the same call
without the expectation is a measurement and with it a verdict. The SWE-bench
per-instance grading spec and its landing live in
`docs/research/powerups-2026-10-20/14-smart-ai-native-engine.md` (donor row 4),
which this row points at instead of repeating.
**Landed in.** `crates/endpoint` (the bucket-and-verdict shape on our own
verify receipts).
**Owned paths.** `crates/endpoint/src/verify_tasks.rs` (new),
`crates/endpoint/src/mcp_tools.rs`, `crates/endpoint/src/build_level/imp/door.rs`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib verify_task`
prints `verify_task: 3 tasks, 1 reward each, judge-free PASS` and exits 0.
**Kill line.** Our diff cannot bucket the change into named categories, so the
verdict has nothing to grade.
**Cost.** S.

## MCX-12 - project memory that knows when it went stale

**What it is.** Agent project memory as plain text notes, one per note, typed
(fact, decision, constraint, convention, failure, bug, command, environment),
each carrying a fingerprint of the files it is about, so a recall says the note
may be out of date instead of presenting it as current; a failed approach scores
higher because dead ends are expensive to rediscover; nothing is sent anywhere.
**Donor.** `sanjuraw/narrowbit-memory`,
https://github.com/sanjuraw/narrowbit-memory, read live 2026-10-21. MIT
(LICENSE read live). Files read live: `src/notes.ts` (the note types, the
file-hash staleness check, the failure weighting, the render with the stale
warning), `README.md`. Zero stars and an early 0.1 preview: the shape is
useful, the trust is not.
**Row closed.** D18 (a project memory the door serves: decisions, failed fixes,
tuned values, each with its tape hash), OPT29.
**PORT-TYPE.** Shape into Rust, no vendored bytes: plain text notes in our own
store, no donor TypeScript.
**Status.** OPEN (2026-10-21). No code landed. The door's resource module
exists (`crates/endpoint/src/resources.rs`, MCX-4) and serves the crate map, but
the memory store, the note types, the file-fingerprint staleness rule and the
failure weighting are still to be built.
**Landed in.** `crates/endpoint` (would land in
`crates/endpoint/src/memory.rs`; nothing landed in this lane).
**Owned paths.** nothing landed; the shape would live in
`crates/endpoint/src/memory.rs` (new) beside
`crates/endpoint/src/resources.rs`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-endpoint --features mcp --lib door_resource`
prints `door_resource: 4 resources, 0 tool-budget chars PASS` and exits 0; the
stale-note half is not built, so that clause stays open.
**Kill line.** The memory needs a network or an embedding model, so it can
never be the runtime path, or a note cannot carry the tape hash.
**Cost.** S-M.

---

## For the render lane

- **D16 resource `e2040://door-reference` and D17 `scene.diff`:** the diff
  answers added, removed and changed rows with their revisions; it does not say
  what changed in the frame. If the render lane wants the pixel truth for the
  same two snapshots, the hook is the existing `view.compare` verb
  (`crates/endpoint/src/packs/view.rs`, two PNGs, FLIP pooled p50/p99) plus
  `view.observe` (`crates/endpoint/src/packs/view.rs`, per-body act/clip/tint
  and bounds). The door needs a per-row (object id) attribution of the error
  mass, so a changed row can name the body that moved; today `view.compare`
  returns the two-image score and `look.judge` returns the per-region verdict,
  and the two are not joined by object id.
- **D16 resource `e2040://crate-map`:** the crate map is the engine's own
  `docs/CRATE-MAP.md`; the render lane owns no crate row of it.
- **D19 verify receipts on build verbs:** a material or mesh build must return
  the same check shape as a level build (a named check, a verdict, a digest).
  The render lane's `crates/material/src/compile.rs` and `crates/render` pass
  should report their own check so the endpoint can name which artifact owns a
  failing check.

## For the spatial lane

- **D17 `scene.diff`:** if the diff must also carry the spatial checks, the
  exact call is `engine_spatial::checks::SPATIAL_CHECKS` (18 checks today) run
  over the two snapshots, and the expected output shape is the same as
  `crates/endpoint/src/scene_tools.rs` already uses for `scene.place`: a list of
  finding lines, one per failed check, each naming the object id, the rule and
  the legal value. The endpoint reads that list and puts it beside the added,
  removed and changed rows; the spatial lane owns the rules, the endpoint owns
  the diff.
- **D19 verify receipts:** a build verb that places actors must return the
  spatial check result as its check row, so a failed build names the rule that
  failed instead of a bare `refused`.
