# WRX - proposed board rows: the workspace's underused power (2026-10-20)

Read-only file. The lead merges the board: I did not touch `docs/SPRINT*.md`, `docs/FOCUS.md` or
`docs/plan/finish-line.md`. Nothing was committed and nothing was pushed.

Every number here comes from a command run on 2026-10-20 on this tree
(`C:\engine2040`, HEAD `615c62b6` unless a row says otherwise). Evidence grades: OBSERVED (I read
the file or ran the command today), DERIVED (reasoned from what I read), INFERRED, UNKNOWN.

## Row shape

**What it is** (two lines) - **Donor** (URL, licence, date read) - **Closes** (the engine2040 board
row id) - **Owned paths** - **Accept** (one command, the one line it must print) - **Kill line** -
**Cost** - **Lands in** (which repo).

A row that says SPEC ONLY is not a patch: it is the exact change another lane can apply when that
lane's files unlock. `crates/observe` is held by the auto-heal lane and `crates/nav` by the layers
lane, so PF9, N-11 and NV10 are specs, not code.

---

## WRX-01 - the unwired-module gate: no dead public module hides again

**What it is.** `tools/wiring-scan.mjs` finds pub ITEMS whose name appears in no other file; it
cannot see a module a crate uses on itself, and its AD5 diff gate only looks at NEW items. So the
shelf that already exists was never checked: `crates/ai` calls `crate::utility::pick_gated` from
inside `crates/ai`, so every item looks used while the module is unreachable from the product.
`tools/unwired-code-gate.mjs` asks the crate-boundary question instead: for every `pub mod` a crate
exports, does any OTHER crate reach it?

**Donor.** `TimonPost/cargo-unused-features` - https://github.com/TimonPost/cargo-unused-features -
MIT (LICENSE read 2026-10-20) - the feature side of the same question. Ported the idea (walk the
feature graph, report what nothing reaches); no bytes vendored, no new dependency.

**Closes.** AD5 (unwired-code gate), extended from "new items in a diff" to "the whole tree".

**Owned paths.** `tools/unwired-code-gate.mjs` (new, written and tested today).

**Accept.** `node tools/unwired-code-gate.mjs --self-test` prints
`PASS unwired-code-gate: 12 known dead modules found, 2 live modules not flagged, fixture ok`.
The 12 are `ai::behavior`, `ai::utility`, `ai::goap`, `ai::blackboard`, `ai::goalstack`,
`ai::director`, `ai::opponent`, `nav::orca` (the board row name is `nav::crowd`), `anim::lod`,
`observe::rerun_sink`, `render::heal` (RenderHeal) and `assets::import_repair`.
The negative control is a tiny fixture (`demo::used` called by another crate, `demo::unused` not)
plus two live modules on today's tree (`ai::melee`, called by `crates/arena/src/engage.rs`;
`ai::tier`, called by `crates/arena/src/fork.rs`).

**One honest correction.** `assets::import_repair` is NOT dead at module level any more: `render`
calls `engine_assets::import_repair::recompute_tangents` at
`crates/render/src/v1.rs:256` and `crates/render/src/present/gpu_present/presenter.rs:2197`. Both
call sites sit behind features the default build does not turn on (`v1` and `window`), and the
module itself is behind `import`, so the gate reports it as `behind-features:import,v1`. The 2026-10-11
audit ("no caller") is stale; the module is reachable only behind a flag.

**Kill line.** If the gate's list is dominated by feature-gated modules and the sharp part (`--reason=no-caller`)
stays empty on a tree with real dead code, the rule is wrong and the row dies with it.

**Cost.** S (done).

**The caller of the gate itself.** The one line in the receipt below, which the lead applies, or one
line in the CI workflow. Until one of them lands the gate is a tool nobody runs, which is the same
disease it reports.

**Lands in.** `C:\engine2040`.

### Receipt: the exact wiring for `tools/workspace-check.mjs` (the lead applies it, I did not edit the file)

`tools/workspace-check.mjs` line 878 ends the wiring block with:

```js
    console.log(`WARN wiring (report-only): ${wireOut}`);
```

Add ONE line right after it (inside the same `try` block is not needed; the line below stands on its
own and is report-only, so a red gate never blocks the audit):

```js
  try { const umOut = execFileSync(process.execPath, [join(repoDir, "tools", "unwired-code-gate.mjs"), "--reason=no-caller"], { encoding: "utf8" }).trim().split("\n").pop(); console.log(`WARN unwired modules (report-only): ${umOut}`); } catch (error) { console.log(`WARN unwired modules (report-only): check failed: ${String(error.message).split("\n")[0]}`); }
```

It prints one line like `WARN unwired modules (report-only): SUMMARY unwired modules: 429 of 754
public modules in 42 crates have no caller outside their own crate`. Report-only on purpose: the
tree has 429 no-caller modules today, so a hard gate would be red before any work lands. The hard
version is the same command with `--fail`.

---

## WRX-02 - PF9 SPEC ONLY: the rerun ring gets a caller and the MCAP file gets a reader

**What it is.** `crates/observe/src/rerun_sink.rs` holds a 120-frame ring of (tick, hash) rows with
`first_divergent`, and `crates/observe/src/mcap.rs` holds a std-only MCAP writer AND reader
(`mcap::read`, `mcap::first_divergent_tick`). No crate outside `crates/observe` calls either: the
gate prints `observe::rerun_sink no-caller`. Board row PF9 is open (`- [ ] PF9 perf: replay visual
debugger`); its own text says the MCAP leg landed on 2026-10-18 and the left part is the caller.

**Donor.** `foxglove/mcap` - https://github.com/foxglove/mcap - MIT (LICENSE read 2026-10-20; the
`mcap` crate on crates.io is 0.26.0, MIT) - the record reader and the schema validation.

**Closes.** PF9.

**Owned paths.** `crates/observe/src/rerun_sink.rs`, `crates/observe/src/mcap.rs` (HELD by the
auto-heal lane - do not touch). The caller lands in `crates/arena/src/fork.rs` (the duel runner),
which is not held.

**Exact change.** In the arena duel run: build `RerunSink::new()`, call `RerunSink::push(tick,
state_hash)` once per tick, call `RerunSink::to_mcap()` on exit and write the bytes next to the
capture. Exact function names to call: `RerunSink::new`, `RerunSink::push`, `RerunSink::len`,
`RerunSink::overflow_count`, `RerunSink::first_divergent`, `RerunSink::to_mcap`; on the file side
`McapWriter::new`, `McapWriter::channel`, `McapWriter::message`, `McapWriter::finish`,
`ReplayRecording::new`, `ReplayRecording::tick_hash`, `ReplayRecording::finish`,
`mcap::read`, `mcap::first_divergent_tick`.

**Exact test names.** Existing (keep green): `holds_120_frames_oldest_first`,
`identical_sinks_have_no_divergence`, `names_first_mismatched_hash`, `extra_tail_counts_as_divergent`,
`span_names_use_rerun_namespace`, `zone_lookup_accepts_set_and_refuses_strangers`,
`every_zone_mark_and_plot_name_lives_under_rerun`, `plot_labels_follow_tick_then_frame_order`.
New: `arena_run_writes_a_recording_whose_first_divergent_tick_names_the_seeded_tick`.

**Exact proof command.** `cargo test -p engine-observe --lib rerun_sink` and
`cargo test -p engine-arena --lib arena_run_writes_a_recording` and
`cargo test -p engine-proof --lib golden_hash_untouched` (the golden `d57b5c8ae3096614` unchanged).

**Kill line.** If the arena run cannot push a hash per tick without touching the hashed path, the
row dies: the ring must stay outside the tick (HASH-LAW).

**Cost.** M.

**Lands in.** `C:\engine2040` - the auto-heal lane owns the observe hunks, the arena lane owns the caller.

---

## WRX-03 - NV10 SPEC ONLY: ORCA gets a production caller and the test twin goes

**What it is.** `crates/nav/src/orca.rs` holds the real ORCA solver (`Orca::new`, `Orca::step`,
`Orca::build_grid`, `Orca::neighbors`) with its own tests, but NO production code calls it: the gate
prints `nav::orca no-caller`. The solver also has a second copy living in a test file,
`crates/ai/tests/crowd_orca_bench.rs` (`step_grid`, `step_kdtree`, `solve_agent`, `kd_build`,
`kd_query`). NV10 is ticked `[x]` and its own tick audit says it was REPLACED by `nav/orca.rs`, so
the row is closed while the module it names has no caller.

**Donor.** `snape/RVO2` - https://github.com/snape/RVO2 - Apache-2.0 (LICENSE read 2026-10-20);
paper https://doi.org/10.1109/ICRA.2011.5980409 - the reference solver, used as the referee the
integer ORCA is checked against, not as a dependency.

**Closes.** NV10 (the reach part), plus the "one crowd solver" rule of the layers lane.

**Owned paths.** `crates/nav/src/orca.rs` (HELD by the layers lane - do not touch),
`crates/ai/tests/crowd_orca_bench.rs` (the duplicate solver, move to `lab/` - never delete).

**Exact change.** One production caller for `Orca::step`: the arena fighter steering reads the
8 fighter positions each tick, builds `Vec<Agent>` (i64 mm coordinates), calls
`Orca::step(&agents, &[], &P, &mut out)` and writes the returned velocities onto the fighters.
Exact function names to call: `Orca::new`, `Orca::step`, `Orca::build_grid`, `Orca::neighbors`;
`Agent` and `Params::ARENA` are the inputs. The test twin `crates/ai/tests/crowd_orca_bench.rs`
moves to `lab/` in the same change so there is one solver, not two.

**Exact test names.** Existing in `crates/nav/src/orca.rs` (keep green):
`corridor_drill_leaves_nobody_stuck_in_1000_seeds`,
`steering_cost_per_agent_stays_flat_to_500_agents`,
`crowd_corridor_corpus_neighbours_equal_the_scan`,
`crowd_of_1000_replays_to_the_same_digest_twice`,
`crowd_step_percentiles_hold_the_p6_bar_to_1000_agents`.
New: `arena_fighters_separate_without_a_nav_mesh`.

**Exact proof command.** `cargo test -p engine-nav crowd` green, then
`cargo test -p engine-arena --lib arena_fighters_separate`, then
`cargo test -p engine-proof --lib golden_hash_untouched` (the golden `d57b5c8ae3096614` unchanged;
the crowd is client-local and never hashed).

**Kill line.** If ORCA changes the arena's hashed path, the row dies: the crowd may not move a
single hashed byte, so the fighters must keep their old velocity when the crowd result is empty.

**Cost.** M.

**Lands in.** `C:\engine2040` - the layers lane owns `crates/nav`, the arena lane owns the caller.

---

## WRX-04 - N-11 SPEC ONLY: one door-owned ring carries the render and audio heals

**What it is.** `crates/render/src/heal.rs` (`RenderHeal`) and `crates/audio/src/heal.rs`
(`DeviceHeal`) are supervisor heals with budgets, quarantine and a heal tape. Nothing constructs
them outside their own crate: `RenderHeal` has no caller at all (the gate prints `render::heal
no-caller`), and `DeviceHeal` has one caller, in a TEST module
(`crates/game/src/audio_mix.rs:390` is inside `#[cfg(test)]`). The gate that would show a refusal
(`crates/observe/src/event_queue.rs`) belongs to the auto-heal lane.

**Donor.** none needed for the wiring. For the ring shape: `foxglove/mcap` (MIT) as in WRX-02, if
the heal events land in the MCAP file rather than in a JSON log.

**Closes.** N-11 (one door-owned ring for net/render/audio heals; wire `RenderHeal`/`DeviceHeal`).

**Owned paths.** `crates/render/src/heal.rs`, `crates/audio/src/heal.rs` (free), and the app/arena
run file that owns the ring (not `crates/observe` - held by the auto-heal lane).

**Exact change.** Construct `RenderHeal::new()` and `DeviceHeal::new()` in the run, feed the
device-lost, present-error and OOM reports into them, and push one typed event per refusal into
the ring the auto-heal lane owns. Exact names to call: `RenderHeal::new`, `RenderHeal::report_oom`,
`RenderHeal::report_surface`, `RenderHeal::report_device_lost`, `RenderHeal::report_present_error`,
`RenderHeal::note_presented`, `RenderHeal::note_reconfigured`, `RenderHeal::state`,
`RenderHeal::is_quarantined`, `RenderHeal::last_good_generation`; on the audio side
`DeviceHeal::new`, `DeviceHeal::report_loss`, `DeviceHeal::report_change`,
`DeviceHeal::try_reopen`, `DeviceHeal::reopen_live`, `DeviceHeal::render_silent_frames`,
`DeviceHeal::is_quarantined`.

**Exact test names.** New: `a_refused_heal_writes_one_typed_event_into_the_ring` and
`a_heal_past_its_budget_is_quarantined_not_repeated`.

**Exact proof command.** `cargo test -p engine-render --lib heal` and
`cargo test -p engine-audio --lib heal` and
`cargo test -p engine-proof --lib golden_hash_untouched` (the golden `d57b5c8ae3096614` unchanged:
a heal is presentation-side and must never enter the hash).

**Kill line.** If the app run cannot hold the ring without a second ring inside `crates/observe`,
the row waits for the auto-heal lane. It never opens a second ring.

**Cost.** M.

**Lands in.** `C:\engine2040` - the render lane, the audio lane and the auto-heal lane together.

---

## WRX-05 - GA11: the behavior tree gets a shipped consumer (a foe brain that shows its plan)

**What it is.** `crates/ai/src/behavior.rs` is a finished, proven behavior-tree executor (Sequence,
Select, Invert, Check, Parallel; byte-identical action traces x2) and GA11 is already ticked
`- [x] GA11 brain: deterministic behavior-tree executor - Landed 453811f`. Nothing calls it:
`git grep` finds `behavior::tick` only in a COMMENT (`crates/ai/src/gladiator.rs:12`), and the gate
prints `ai::behavior no-caller`. A ticked board row with no consumer is the exact disease the audit
found; this row gives it one.

**Donor.** none needed: the executor is ours and proven. The shape to copy for the plan trace:
`crates/ai/src/decision_log.rs` (already used by `crates/endpoint/src/packs/play.rs`) - the ring of
canonical bytes with the first divergent tick between two tapes.

**Closes.** GA11 (from "ticked but unused" to "used"), and it opens the door verb that shows a plan trace.

**Owned paths.** `crates/ai/src/behavior.rs`, `crates/ai/src/blackboard.rs` (the shared state the
tree reads), `crates/arena/src/fork.rs` (the duel brain seam), `crates/endpoint/src/packs/play.rs`
(the `plan.trace` verb).

**Exact change.** One BT-driven foe brain behind the existing `Brain` seam in the arena duel: the
tree's actions append ops to the caller's buffer (that is the module's own contract: "Stateless
actions; mutation flows through ops appended to a caller buffer"), and the node trace goes into
`engine_ai::decision_log` so the door can answer `plan.trace` with the plan and its first
divergent tick.

**Exact test names.** Existing (keep green): `behavior_tree_check_and_parallel_status`,
`behavior_tree_trace_is_byte_identical_x2`. New: `bt_foe_writes_its_plan_trace_into_the_decision_log`
and `bt_foe_duel_replays_equal_twice`.

**Exact proof command.** `cargo test -p engine-ai --lib behavior_tree` green,
`cargo test -p engine-arena --lib bt_foe` green, and
`cargo test -p engine-proof --lib golden_hash_untouched` (the golden `d57b5c8ae3096614` unchanged:
the tree must not change a hashed byte of the shipped duel, so it lands as an opt-in foe kind).

**Kill line.** If the tree cannot run without a float, a `HashMap` or a wall clock, the row dies -
`crates/ai` is integer-only and stays that way.

**Cost.** M.

**Lands in.** `C:\engine2040` - the ai lane and the arena lane.

---

## WRX-06 - GA12: the GOAP planner and the blackboard get the same consumer

**What it is.** `crates/ai/src/goap.rs` is an A* planner over a 64-fact integer world state and
`crates/ai/src/blackboard.rs` holds the shared BT/GOAP state plus the `goap_plan` entry point.
Both are ticked (GA12 `- [x] GA12 brain: GOAP planner over the utility world-state - Landed
49df5811`) and neither has any caller: the gate prints `ai::goap no-caller` and
`ai::blackboard no-caller`, and `git grep` finds no use of either outside `crates/ai/src/lib.rs`.

**Donor.** none needed (the planner is ours, ported from `victorb/dogoap`, MIT, and the board row
already names it). The shape to copy for the goal list: `crates/ai/src/goalstack.rs`.

**Closes.** GA12, and it needs WRX-05 (the executor that walks the plan).

**Owned paths.** `crates/ai/src/goap.rs`, `crates/ai/src/blackboard.rs`, `crates/arena/src/fork.rs`.

**Exact change.** The same BT-driven foe from WRX-05 gets a GOAP layer: the blackboard holds the
facts, the goal stack names the goal, `goap_plan` returns the step list and the executor walks one
action per tick. The plan and its trace are the same artifact the door serves.

**Exact test names.** Existing (keep green): `goap_plan` (5 tests today). New:
`goap_foe_replans_inside_one_tick_budget_on_a_state_flip` and
`the_planned_steps_replay_equal_twice`.

**Exact proof command.** `cargo test -p engine-ai --lib goap_plan` green,
`cargo test -p engine-arena --lib goap_foe` green, and
`cargo test -p engine-proof --lib golden_hash_untouched` (the golden `d57b5c8ae3096614` unchanged).

**Kill line.** If the plan needs more than one tick of search budget in the arena's frame, the row
dies; the planner must plan outside the tick, not inside it.

**Cost.** M.

**Lands in.** `C:\engine2040` - the ai lane.

---

## WRX-07 - GA14: the difficulty sweep reads the designer rows instead of a test twin

**What it is.** GA14 (`- [ ] GA14 playtest: difficulty sweep with a human-like error model`) says
the sweep, the seeded human-error tape and the CSV writer exist "under cfg(test) in
crates/playtest/src/tournament.rs". The rows they must read live in the ai crate:
`crates/ai/src/rows.rs` (the one scanner behind every `key=value` row) and
`crates/ai/src/curves_text.rs` (the designer numbers for aggression, caution and morale). Both are
reachable only from inside `crates/ai` (the gate prints them), so the sweep re-implements the row
scanner instead of using the one owner.

**Donor.** none needed (our own rows parser). The shape to copy: `content/design/enemies.rows` and
`crates/ai/src/depth.rs::parse_tiers`, which already read the same file.

**Closes.** GA14, and it gives `rows.rs` and `curves_text.rs` a second consumer.

**Owned paths.** `crates/playtest/src/tournament.rs`, `crates/ai/src/rows.rs`,
`crates/ai/src/curves_text.rs`, `content/design/enemies.rows`.

**Exact change.** The sweep reads the difficulty rows through `crate::rows` and
`crate::curves_text::parse_ai_config` (the same refusal rules as the tier parser), so a bad row
refuses the sweep instead of being skipped. The CSV lands in `target/reports/rome-ttk.csv` with a
win matrix from the GA13 tournament.

**Exact test names.** Existing (keep green): `sweep_rows_cover_every_difficulty_at_each_value`,
`sweep_skips_bloodless_variants`. New: `sweep_rows_come_from_the_designer_rows_file` and
`a_malformed_difficulty_row_refuses_the_sweep`.

**Exact proof command.** `cargo test -p engine-playtest --lib tournament` green, the CSV exists at
`target/reports/rome-ttk.csv`, and `cargo test -p engine-proof --lib golden_hash_untouched` (the
golden `d57b5c8ae3096614` unchanged).

**Kill line.** If the sweep needs a model in the loop to produce its rows, it is not a sweep and
the row dies (GA14 wants no noise by default).

**Cost.** M.

**Lands in.** `C:\engine2040` - the playtest lane.

---

## WRX-08 - GA15: the primus boss gets its pacing, its goals and its two phases

**What it is.** GA15 (`- [ ] GA15 ai: primus boss, own move set and two phases`) says the ladder
has no phase 2: `PRIMUS_TIER` (hp 1100, tell 15, stagger 700) ships in `crates/ai/src/depth.rs`
and as `tier_4` in `content/design/enemies.rows`, but phase 2 at 500 permille hp is missing. The
three modules that would carry it are all unused: `crates/ai/src/director.rs` ("pacing as data.
Tables + tension scalar, never hardcoded spawns"), `crates/ai/src/goalstack.rs` ("bounded ioq3-style
goal memory plus fuzzy intent weights") and the BT of WRX-05. `director` is the sharpest: its only
use today is inside `#[cfg(test)] mod tests` (`crates/ai/src/depth.rs:1630`), so it has no
production caller at all.

**Donor.** none needed. The shape to copy for the boss move set: `crates/ai/src/depth.rs` and the
tier rows in `content/design/enemies.rows` (GA13 owns them).

**Closes.** GA15.

**Owned paths.** `crates/ai/src/director.rs`, `crates/ai/src/goalstack.rs`,
`crates/ai/src/behavior.rs` (the two phases as one tree), `content/design/enemies.rows` (the tier_4
row), `crates/arena/src/fork.rs` (the boss bout).

**Exact change.** The boss runs one BT whose two phases are two sub-trees; phase 2 starts at 500
permille hp; the goal stack holds the phase goal; the director spends an intensity budget on the
phase's spawn pressure, so the pacing stays data.

**Exact test names.** Existing (keep green): the GA13 tier tests (`tier_bar_posts_600_plus_per_pair`).
New: `primus_phase_two_starts_at_500_permille_hp` and
`the_boss_plan_trace_is_byte_identical_x2`.

**Exact proof command.** `cargo test -p engine-ai --lib primus` green,
`cargo test -p engine-playtest --lib tier_bar` green (the 600 permille bar still holds), and
`cargo test -p engine-proof --lib golden_hash_untouched` (the golden `d57b5c8ae3096614` unchanged:
the boss lands as a new tier, not as a change to the shipped duel).

**Kill line.** If the boss needs a new move set written in Rust instead of in the rows file, the row
grows past M and waits for the owner's word on the boss.

**Cost.** M.

**Lands in.** `C:\engine2040` - the ai lane.

---

## WRX-09 - EMPIRE: forge's 100 unreachable files and 37 kit files with no game

**Exact path.** `C:\forge\docs\finish-line\unreachable.txt` (100 files, 13,719 lines - measured by
summing the line column) and the SHELF map in `C:\forge\scripts\kit-callers.test.mjs` (37 rows).
Forge's own `docs/finish-line/measure.py` writes both lists; the kit gate refuses a new caller-less
kit, so the shelf cannot grow, only shrink.

**Donor.** none needed to land one kit: the code exists and the kit gate already refuses a new
caller-less kit. If the game he names is the net lane, the donor for the seven orphan net kits is
`RevenantX/LiteNetLib` - https://github.com/RevenantX/LiteNetLib - MIT (LICENSE read 2026-10-20).

**Owned paths.** `C:\forge\docs\finish-line\unreachable.txt`, `C:\forge\scripts\kit-callers.test.mjs`
(the shelf map), and the one kit file the named game takes.

**What it is.** Finished code with proofs and no game. The shelf rows name the game each kit waits
for (`kits/ai/GladiatorBrainKit.cs` waits for the Rome duel, `kits/net/*` waits for the net lane,
`kits/move/*` waits for the fight grapple).

**What activating it needs.** One shelf kit wired into the game its own row names, in one window:
the kit gate (`node scripts/kit-callers.test.mjs`) then fails on the dead row and the shelf line is
deleted in the same change, which is the rule the repo already holds.

**The ONE decision only Maxim can make.** WHICH game gets the next window. Forge runs one reading
window at a time (the freeze lock refuses every commit while it is held), and the 37 shelf rows name
different games - the Rome duel (G8), the arena brawl, the topdown slice, the motion slice. He
picks the one game, the rest stay on the shelf; picking all of them is the same as picking none.

**Cost.** S per kit, M per window.

**Lands in.** `C:\forge` (the games room) - route through center's inbox.

---

## WRX-10 - EMPIRE: center's arsenal lists 19 tools that have no run file

**Exact path.** `C:\empire\center\arsenal.json`; read it with `node C:\empire\center\arsenal.mjs --list`
(130 entries; 19 print `not callable (run file missing: ...)`).

**Donor.** `modelcontextprotocol/registry` -
https://github.com/modelcontextprotocol/registry - MIT with new code moving to Apache-2.0
(LICENSE read 2026-10-20) - the server-metadata shape for a tool list an agent can trust: one row
per tool with the file that runs it and the licence it came from.

**What it is.** The agent-facing registry of every project's tested tool. Nineteen rows still name
tools whose run file was deleted when the loop machinery was removed on 2026-10-17. The 19 rows,
with the file each one says is missing (`node arsenal.mjs --list`, 2026-10-20):
`engine2040 pack_clean` (tools/pack-clean.mjs), `engine2040 clean_run` (tools/clean-run.mjs),
`center repomap` (repomap.mjs), `center metrics` (metrics.test.mjs), `center doctor`
(doctor.test.mjs), `center loopkit_new` (loopkit.mjs), `center reds` (reds.mjs),
`center round_proof` (round-proof.mjs), `center moved` (moved.mjs), `center reuse` (reuse.mjs),
`center retire` (reuse.mjs), `center boss_view` (press.mjs), `center boss_press` (press.mjs),
`center fail_memory` (fail-memory.mjs), `center land_sweep` (land-sweep.mjs),
`center lane_recycle` (serve-v2.mjs), `center permissions-lint` (agentlint.mjs),
`center go_order` (autopilot.test.mjs), `center row_pick` (row-pick.mjs).

**Owned paths.** `C:\empire\center\arsenal.json` and `C:\empire\center\arsenal.mjs` (the reader
and the gate that checks a listed tool has a run file).

**What activating it needs.** One of two things per row: the run file comes back (git history
before commit `6aa2e854` has the old code) or the row is deleted; plus one gate that fails when a
listed tool has no run file, so the list cannot rot again.

**The ONE decision only Maxim can make.** WHAT the registry MEANS. If it is a list of things that
exist, the 19 rows go and the registry is short. If it is a wish list of things to build, they
stay and every agent that reads it is reading a to-do list as if it were a menu. The agents read
this file, so the meaning is his call, not ours.

**Cost.** S.

**Lands in.** `C:\empire\center` (the control layer) - the decision goes to the owner inbox.

---

## WRX-11 - EMPIRE: jobhunt's prep chain lands and sends nothing because three job families have no source

**Exact path.** `C:\empire\money\jobhunt\` - `tools/fetch.mjs`, `tools/match.mjs`,
`data/companies.json`, `data/rss-feeds.json`, `private/approve-queue.json` (22 rows waiting on
Maxim), and the board rows JH-106, JH-112, JH-150, JH-154.

**What it is.** The whole prep chain works: fetch, the BM25 matcher (`tools/lib/bm25.mjs`), the ATS
gate, the Hebrew RTL lint, the CV renderer, the kit check, and an approve queue of 22 rows that
never sends (auto_apply is OFF and Maxim owns the send). The four board rows are all BLOCKED on
the same thing: families 3-7 (trust-safety, game-tools, marketing-automation, design-ops, producer)
have 0-2 fit-60 listings because no ATS board of ours carries them.

**Owned paths.** `C:\empire\money\jobhunt\data\companies.json`, `C:\empire\money\jobhunt\tools\fetch.mjs`,
`C:\empire\money\jobhunt\tools\match.mjs`, `C:\empire\money\jobhunt\docs\SOURCES.md`.

**What activating it needs.** One public job-board source added behind the same
`data/companies.json` + `docs/SOURCES.md` robots/terms rule the repo already holds (no logins, no
fake accounts, no captcha). A donor exists and is MIT: `speedyapply/JobSpy`
(https://github.com/speedyapply/JobSpy, LICENSE read 2026-10-20) - its `jobspy/model.py` is a
normalised job schema and `jobspy/records.py` the record shape; port the schema, obey each site's
terms.

**The ONE decision only Maxim can make.** WHICH boards may be added to the ATS list. Every added
source spends his tokens and his quota against site terms, and JH-107 (the 22-row approve queue)
is already an OWNER row: he decides the queue rows. One word from him opens the fetch; the code
follows.

**Cost.** S for the schema, M for the sources.

**Lands in.** `C:\empire\money\jobhunt` (the money room).

---

## WRX-12 - EMPIRE: the nine dead AI modules under the ticked rows GA11 and GA12 - one owner

**Exact path.** `C:\engine2040\crates\ai\src\behavior.rs`, `goap.rs`, `blackboard.rs`,
`goalstack.rs`, `director.rs`, `opponent.rs`, `curves_text.rs`, `rows.rs`, `utility.rs`.

**Donor.** none needed: the nine modules are ours. The shape to copy for the plan that lands with
them is `crates/ai/src/decision_log.rs` (ours: the ring of canonical bytes with the first divergent
tick between two tapes, already served by `crates/endpoint/src/packs/play.rs`).

**What it is.** Nine public modules of `engine-ai` with no caller outside `crates/ai`. Five have
no caller AT ALL - `behavior` (only a comment at `crates/ai/src/gladiator.rs:12` mentions
`behavior::tick`), `goap`, `blackboard`, `goalstack`, and `director` (whose only use is inside
`#[cfg(test)] mod tests` at `crates/ai/src/depth.rs:1630`). Four are reachable only from inside
`crates/ai`: `opponent` (`crates/ai/src/depth.rs`), `curves_text` (`crates/ai/src/gladiator.rs`),
`rows` (`crates/ai/src/curves_text.rs` and `depth.rs`), `utility` (`crates/ai/src/gladiator.rs`).
GA11 and GA12 are both ticked as landed, so two board rows claim work that no product reaches.

**What activating it needs.** WRX-05 (behavior), WRX-06 (goap, blackboard), WRX-07 (rows,
curves_text), WRX-08 (director, goalstack), and one consumer for `utility` and one for `opponent`
(both belong to the same foe brain as WRX-05). The gate from WRX-01 keeps the count from growing.

**The ONE decision only Maxim can make.** WHICH brain the arena duel and the Rome ladder ship: the
planner (the BT/GOAP stack of GA11/GA12) or the depth net that is already in the fighters. Both
cannot be the shipped foe brain, and GA15 (the primus boss's two phases) and GA14 (the difficulty
sweep) both read the winner - so this one word decides which of the nine modules get a consumer and
which stay a library. It is his call because the boss is a visible design choice.

**Cost.** S for the decision, M-L for the wiring (WRX-05 to WRX-08).

**Lands in.** `C:\engine2040` (the ai lane) plus the owner inbox for the decision.

---

## Extra check: the natural consumer of each of the nine AI modules

Read from each file's header comment today (OBSERVED):

| module | header says | natural consumer | row |
|---|---|---|---|
| `behavior.rs` | "behavior-tree. Stateless actions; mutation flows through ops appended to a caller buffer." | a foe brain in the arena duel that shows its plan; then the boss's two phases as two sub-trees | GA11 -> WRX-05, GA15 -> WRX-08 |
| `goap.rs` | "goap: A* action planner over a 64-fact integer world state. Owner: callers bake Action rows (preconditions, effects, costs) once and call plan when the state or the goal changes; an executor walks the returned step list one action per tick." | the same foe brain: the planner hands the step list to the executor of WRX-05 | GA12 -> WRX-06 |
| `blackboard.rs` | "blackboard: BT/GOAP shared state. Bake planners output tables; runtime reads ints." | the shared state between the tree and the planner, so it lands with WRX-05/06 | GA12 -> WRX-06 |
| `goalstack.rs` | "goalstack: bounded ioq3-style goal memory plus fuzzy intent weights. Owner: Brain keeps one stack plus one nerve table per fighter." | the fighter `Brain`: the boss's phase goal and the per-fighter nerve weights | GA15 -> WRX-08 |
| `director.rs` | "director: pacing as data. Tables + tension scalar, never hardcoded spawns." | the arena's wave and boss pacing: phase 2 of the primus boss spends an intensity budget | GA15 -> WRX-08 |
| `opponent.rs` | "opponent: a four-bucket foe model plus the action lease it feeds. Consumer: DepthBrain ... reads the table only when its opponent read is armed. Both are opt-in: the default brain leaves the lease at N = 0." | the depth brain in the arena duel (`crates/arena/src/fork.rs` builds `DepthBrain` duels); the lease is the seam a BT foe would also read | GA15/GA14 -> WRX-08, WRX-05 |
| `curves_text.rs` | "ai rows text: designer numbers without a Rust edit. Consumer: Brain::with_rows_text in gladiator.rs ... cold only, never tick." | the designer rows for the difficulty sweep: the aggression/caution/morale values GA14 sweeps | GA14 -> WRX-07 |
| `rows.rs` | "rows text: the one scanner behind every designer key=value row. Consumers: parse_tiers (enemy tiers) and parse_ai_config (ai curves) ... Caller: cold only, at match build, never tick." | the same row scanner for the sweep's own rows, so a bad row refuses the sweep | GA14 -> WRX-07 |
| `utility.rs` | "utility: utility-AI scorers. Pure decision, no NN, no rand." | a utility-scored foe: `pick_gated` already runs in `gladiator.rs`; the dead one is `pick` | GA14 -> WRX-07, GA15 -> WRX-08 |

### Which one to wire first, and why

**`crates/ai/src/behavior.rs` first (WRX-05).**

1. Its board row GA11 is already ticked, so the executor is proven (`behavior_tree_check_and_parallel_status`,
   `behavior_tree_trace_is_byte_identical_x2`) and the work is wiring, not building.
2. Its own contract already matches the arena seam: "mutation flows through ops appended to a
   caller buffer" is exactly how the arena's fighters take their actions, so no new seam is needed.
3. It is the substrate for the other three: `blackboard` is the shared state the tree reads,
   `goap` hands a step list to "an executor", and `goalstack` is the goal memory a tree consults.
   One consumer for the tree gives three of the nine modules a consumer in the same row.
4. It is the only one of the nine whose output is already a plan trace, and that trace is what the
   door verb "show me the plan" needs (`engine_ai::decision_log` is the owner of that ring).
5. It is opt-in by construction: a tree-driven foe kind can be added to the arena without touching
   a hashed byte, so the golden `d57b5c8ae3096614` stays where it is.

Runner-up: `crates/ai/src/director.rs` (WRX-08), because the primus boss's phase 2 has no pacing
source at all today and its only caller is a test module.

## Rejections and open questions

- `engine-playtest` is NOT a consumer-less crate any more: `crates/endpoint/Cargo.toml` has
  `engine-playtest = { workspace = true, optional = true }` under feature `mcp`, and
  `crates/endpoint/src/packs/verify.rs` serves `replay.verify`, `replay.bisect`, `bug.replay`,
  `trace.summary`, `verify.proof` under the separate `verify` feature, which is off by default.
  The 2026-10-11 audit ("36 files, 294 tests, no consumer") is stale; the crate is now 57 files.
- `assets/import_repair` is NOT a dead module any more (see the WRX-01 correction).
- `nav::crowd` does not exist: the file is `crates/nav/src/orca.rs` and NV10's own tick audit says
  it was REPLACED by it. The board text still names `crates/nav/src/crowd.rs`.
- `ai::utility::pick` is an ITEM island, not a module island: the module is used inside
  `crates/ai`. `tools/wiring-scan.mjs --orphans` is the tool for that half.
- The gate's full list is large by construction (550 of 754 public modules have no caller outside
  their own crate). The sharp part is `--reason=no-caller` (429 today). A future row may narrow the
  rule to "modules of crates that other crates depend on"; that is a different question and this
  file does not claim it.
