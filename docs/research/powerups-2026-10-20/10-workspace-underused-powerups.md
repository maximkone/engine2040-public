# 10 - what the workspace has and nobody uses, and the powerup that finishes it

Read 2026-10-20. All paths are absolute; all numbers come from a command run today.

**The answer in one paragraph.** The empire is not short of code, it is short of callers. Every room
holds finished work that a tool, a gate or a game never reaches: engine2040 has nine `crates/ai`
modules (behavior tree, GOAP, goal stack, blackboard, spawn director, foe model, utility picker,
designer rows, curve text) with zero callers outside `crates/ai`, while board rows GA11 and GA12 are
already ticked as landed; its ORCA crowd solver, the 120-frame rerun ring, the MCAP writer, the render
heal, the debug-draw recorder and the generational asset handle all have no production caller; forge
ships 100 files (13,719 lines) that its own `measure.py` proves nothing reaches, plus 37 kit files
that have a proof but no game; center's arsenal lists 130 tools and 19 of them have no run file;
skillworks has 113 skill folders and only 21 sit on the shared shelf; jobhunt's whole prep chain is
done and sends nothing because three job families have no source. The fix is almost never "write more":
it is one row of wiring per shelf, and a donor that makes the wiring a one-liner. That is what the
powerup table is for.

## Corrections to the audit list (`docs/plan/forge-adoption.md` A.3 item 5)

The audit is dated 2026-10-11. Re-checked today with `git grep` in `C:\engine2040`:

| Audit claim (2026-10-11) | Today (2026-10-20) | Grade |
|---|---|---|
| `assets/import_repair` has no caller | **CORRECTION: it has one.** `crates/render/src/present/gpu_present/presenter.rs:2197` and `crates/render/src/v1.rs:256` call `import_repair::recompute_tangents` | OBSERVED |
| `observe/rerun_sink` has no caller | HOLDS. `git grep RerunSink -- crates` outside `crates/observe` = 0 hits. `crates/observe/src/mcap.rs` (std-only MCAP writer) also has no caller outside observe | OBSERVED |
| `RenderHeal` has no caller | HOLDS. `git grep RenderHeal -- crates` outside `crates/render` = 0 hits. Same for `DeviceHeal` in `crates/audio` | OBSERVED |
| `anim::lod` has no caller | HOLDS, one level deeper. Only `crates/anim/tests/it/anim_lod_pin.rs` and `crates/anim/src/crowd_scale.rs`; `crowd_scale` itself has no consumer outside `crates/anim` | OBSERVED |
| `ai::behavior` has no caller | HOLDS. `git grep "engine_ai::behavior::" -- crates` outside `crates/ai` = 0 hits, and the only mention outside is a comment in `crates/ai/src/gladiator.rs:12`. Row GA11 is ticked `[x]` | OBSERVED |
| `ai::utility` has no caller | HALF HOLDS. `pick_gated` is used by `crates/ai/src/gladiator.rs`; `pick`, `response_curve` and the `Scorer` trait have no outside caller | OBSERVED |
| `nav::crowd` (ORCA) has no consumer | HOLDS, renamed. The file is `crates/nav/src/orca.rs`; the only outside caller is `crates/ai/tests/crowd_orca_bench.rs` (a test). The grid and kd-tree neighbour feeds live in that test file too | OBSERVED |
| `engine-playtest` (36 files, 294 tests) has no consumer | **CORRECTION: it has one, off by default.** `crates/endpoint/Cargo.toml` has `engine-playtest = { workspace = true, optional = true }` under feature `mcp`, and `crates/endpoint/src/packs/verify.rs` serves `replay.verify`, `replay.bisect`, `bug.replay`, `trace.summary`, `verify.proof` under the separate `verify` feature, which is **off**. The crate is now 57 files, about 1035 test functions | OBSERVED |

`crates/render/src/debug_draw.rs` and `crates/assets/src/handle.rs` are two more orphans the audit
did not name: both are called only by their own tests (`git grep` outside their own crate = 0 hits),
and `docs/plan/finish-line.md` says so in rows 13 and 2.

## Inventory: 20 underused systems

| # | Path | What it is | Who could use it | One row of work to activate | Payoff | Grade |
|---|---|---|---|---|---|---|
| 1 | `C:\engine2040\crates\ai\src\behavior.rs`, `goap.rs`, `blackboard.rs`, `goalstack.rs`, `director.rs`, `opponent.rs`, `curves_text.rs`, `rows.rs`, `utility.rs` | Nine AI modules: BT executor, GOAP over 64 integer facts, blackboard, goal stack, spawn director, foe model, designer-row parser, curve text, utility picker. `engine_ai::<module>::` outside `crates/ai` = 0 hits for every one of them | the arena duel (`crates/arena/src/fork.rs` uses only `DepthBrain`), the Rome fighters, the door's `play` pack | one `Brain` impl that plans with `goap_plan` instead of `pick_intent`, plus a gate that a ticked board row needs a non-test caller | GA11/GA12 stop being paper; the door gains a "plan and show the trace" verb | OBSERVED |
| 2 | `C:\engine2040\crates\nav\src\orca.rs` + `crates\ai\tests\crowd_orca_bench.rs` | Real ORCA crowd avoidance and the grid/kd-tree neighbour search, living in a test file | the 8-fighter arena (fighters stop walking through each other), content crowd rows CT25/CT34, board row NV10 | move the grid into `crates/nav/src/crowd.rs` and call `Orca::step` from the arena fighter move | crowd behaviour without a nav mesh; the 1000-agent p50 becomes a number we own | OBSERVED |
| 3 | `C:\engine2040\crates\observe\src\rerun_sink.rs` + `crates\observe\src\mcap.rs` | A 120-frame ring of (tick, hash) rows with a first-divergent-tick search, and a std-only MCAP writer. Row PF9 is open | every engine session: a bug report becomes a file you can open | construct the sink in the arena run, write the MCAP on exit, open it in a viewer | "which tick first diverged" stops needing the engine's own tooling | OBSERVED |
| 4 | `C:\engine2040\crates\render\src\heal.rs` (`RenderHeal`) and `crates\audio\src\heal.rs` (`DeviceHeal`) | Supervisor heals with budgets, quarantine and a heal tape. No production constructor outside their own crate (row N-11) | the window build: a GPU device loss should heal, not kill the session | construct both in the app run and push one typed event per refusal into the observe ring | device loss heals inside the game with a budget instead of a crash | OBSERVED |
| 5 | `C:\engine2040\crates\render\src\debug_draw.rs` | A finished debug-draw recorder: groups, depth bias, `line_verts`, byte-replayable. Called only by its own tests | the game window and the dev console (rows EG26 and RL58, finish-line gap 13) | call it from `crates/game/src/windowed.rs` and add the console on top | half of the dev console is already written | OBSERVED |
| 6 | `C:\engine2040\crates\assets\src\handle.rs` | Generational `AssetHandle` with NotLoaded/Loading/Loaded/Failed states and 10 passing tests. Row AS4 is ticked; the handle has no caller (finish-line gap 2) | the content pipeline: every asset the game holds goes through it | route the AS2 streaming queue through the handle, add the browser fetch half | asset lifetime and unload become explicit | OBSERVED |
| 7 | `C:\engine2040\crates\endpoint\src\packs\verify.rs` (behind the off-by-default `verify` feature) | The door's verifier verbs: `replay.verify`, `replay.bisect`, `bug.replay`, `trace.summary`, `verify.proof`. The shipped `e2040-mcp` has none of them | every AI agent that edits the scene: it gets its own check back (finish-line gap 19) | build the door with `--features verify` in CI and publish the verbs | 57 playtest files and about 1035 tests become reachable from the door | OBSERVED |
| 8 | `C:\forge\docs\finish-line\unreachable.txt` | 100 files, 13,719 lines, that forge's own `measure.py` proves nothing reaches; plus a 37-file kit shelf in `C:\forge\scripts\kit-callers.test.mjs` where every kit has a proof but no game, and the shelf row names the game it waits for | the Rome duel and the arena brawl (forge's own rows name them) | take one shelf kit with a named game, e.g. `kits/ai/GladiatorBrainKit.cs` into the duel | forge's stock becomes stock in use, not stock on a shelf | OBSERVED |
| 9 | `C:\forge\sidecars\` | Ten sidecar services, each with its own `serve.py`, `refuse_tests.py` and requirements. `kimodo` and `nav-daemon` have zero mentions in `bridge`, `gateway` or `scripts` | the tool surface: media, asset and nav work that the bridge cannot do today | put all ten behind one MCP server with one schema, keep the refusal tests as the gate | ten hand-written HTTP contracts become one tool list | OBSERVED |
| 10 | `C:\empire\center\arsenal.json` | 130 tools listed; `node arsenal.mjs --list` prints 19 of them as "not callable (run file missing)" because the loop machinery was removed on 2026-10-17 and the registry kept the rows | every agent that asks the arsenal what it can run | delete or re-file the 19 rows and add a gate that fails when a listed tool has no run file | the arsenal stops lying about what exists | OBSERVED |
| 11 | `C:\empire\lab\skillworks\skills\` (113 folders) and `skills-shared\manifest.json` (21 skills) | 113 skill folders, 109 with a trials sheet, 21 on the shared shelf. `FINISH-LINE.md` S7 reads 3 skills with live lift and wants 8; the empire README says "36 of 114 proven" (the README's number, not re-measured today) | the repos that hit the same failure class: forge, engine2040, asset-vault, jobhunt | pick 5 more shelf candidates with a measured lift and install them where the failure class appears | the lab stops being a library nobody opens; S1/S2/S7 start counting real loads | OBSERVED |
| 12 | `C:\empire\studio\asset-vault\media\video\` | The whole video maker: 11 skills under `media\video\skills`, about 40 tool folders, a timeline/otio/render ladder. Parked until a video order opens | forge's `clip20` Rome reel; the factory's first sellable pack list (sound S1, pixel UI S2) | open one order (a 20 s Rome clip) and run the lane | a third sellable pack, and the reel tool gets raw material | OBSERVED |
| 13 | `C:\empire\money\jobhunt\` | The whole prep chain lands (fetch, match, BM25, ATS gate, RTL lint, CV render, kit check, a 22-row approve queue) and sends nothing. Rows JH-106, JH-112, JH-150, JH-154 are all blocked on "3 families only" | Maxim, who owns the send | add one public job-board source behind the same `data/companies.json` and robots rule | the 22-row approve queue gets real fit-60 rows instead of a 3-family ceiling | OBSERVED |
| 14 | `C:\empire\money\autonomous-factory\` | 44 product folders plus 14 in `packs\products` (150 products reviewed). `board\cash-list.md`: the only blockers are 2 owner gates (8 itch drafts, the Cloudflare Pages token) and `engine.drivers.gumroad` is "not tracked: the Gumroad driver is missing" | the firm: the store-ops row names a driver that does not exist | write the Gumroad driver, or drop the name from `FACTORY.md` and `arsenal.json` | the listing row stops naming a driver that does not exist | OBSERVED |
| 15 | `C:\empire\games\arena\` | A complete JS arena game: integer-only `src/sim.js`, code-drawn `src/render.js`, `ai.js`, `rules.js`, `tools/check.js`. `git grep "games/arena"` across the monorepo = 0 hits | the empire: a browser-playable that proves the shared sim rules | name its use (a browser playable) or move it to `_archive` | either a second arena that plays in a tab, or a smaller workspace | OBSERVED |
| 16 | `C:\empire\games\kam-web\` | M0-M5 accepted, M5 AI in progress, `docs/M6-SPIKE.md` written (lobby + relay + chat, lockstep commands + CRC relay) | the owner priority in `docs/PLAN.md`: "add LAN or internet multiplayer correctly" | finish the relay server and one two-client lockstep bout | the hobby project's stated goal lands | OBSERVED (docs read; not run) |
| 17 | `C:\empire\games\omni-main\` | An RTS autopilot: 19 profiles, `pypilot`, tactics, a bench, evidence folders. Frozen; board K-01..K-08 all open and `VISION.md`'s scorecard is all UNKNOWN | the project's own proof gate is `python -m pytest -q`; the scorecard is the outside row | fill the 5 scorecard rows from bench numbers the repo can already measure | the project gets a proof that is not UNKNOWN | OBSERVED |
| 18 | `C:\empire\lab\fp-research\tools\` | Twelve tools (drill, pow_bench, text, text_minhash, copy_score, tls_tap, audit_cards, break_ledger, browse, seed, score-api) that run only against its own honeysite | jobhunt (anti-abuse proof for its forms) and autonomous-factory (cost-asymmetry for any sign-up) | run `pow_bench` against the jobhunt sign-up path and file the numbers | jobhunt gets a cost number instead of an opinion | OBSERVED (tools present; other-repo use not measured) |
| 19 | `C:\empire\center\sprint\` and `C:\empire\center\hardening\` | center's README says `sprint/` is "old plans and history; nothing reads them any more"; `hardening/` is the old works repo's router and bench | center's C3 bar: at most 200 tracked files | move both to `_archive` (never delete) and record the move | the control layer gets room and stops carrying dead history | OBSERVED |
| 20 | `C:\_quarantine\2026-10-17\{loop-removal,monorepo-cutover,wave4-reviews}` and `C:\empire\_archive\{dormant,omni-donors,works}` | The old 10-repo history, the removed loop machinery (400+ files) and Maxim's own kept projects. `wave4-reviews` is the source of the 150-row factory review behind `cash-list.md` | the audit trail for what was cut | read; do not move | what was cut stays readable | OBSERVED |

## Powerups: 12 donors that turn the shelf into finished capability

| # | What it is | Donor + URL + date read | License | Fills (engine2040 row id) | What exactly to port (donor file path) | Cost S/M/L | Kill line | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | Find Cargo features nothing enables, so the off-by-default shelves cannot hide | `TimonPost/cargo-unused-features` - https://github.com/TimonPost/cargo-unused-features (read 2026-10-20) | MIT | AD5 (unwired-code gate); the `verify`, `geo`, `intake`, `rules` features of `engine-endpoint`; finish-line gap 19 | `src/main.rs` (the feature-graph walk that reports a feature no path enables) - port the idea into the existing `tools/packet-gate.mjs`, no second tool | S | the tool only reports Cargo features and never the pub items, so it cannot close AD5 on its own: then keep it as a report-only lane and say so | PORT |
| 2 | A reader and schema validator for the MCAP files the engine already writes | `foxglove/mcap` - https://github.com/foxglove/mcap (crates.io `mcap` 0.26.0, read 2026-10-20) | MIT | PF9 (replay visual debugger); N-11 (one door-owned ring) | `rust/mcap/src/read.rs` and `rust/mcap/src/records.rs` - port the record validation and the reader; the engine's own `crates/observe/src/mcap.rs` writer stays | S | if the engine's std-only writer already round-trips through a reader that opens in a viewer, the port is finished: no new dependency | PORT |
| 3 | An immediate-mode debug UI to build the dev console on | `emilk/egui` - https://github.com/emilk/egui (crates.io `egui` 0.37.0, read 2026-10-20) | MIT OR Apache-2.0 | EG26 (dev console over the door verbs); RL58 (debug-draw lines); finish-line gap 13 | `crates/egui/src/context.rs` - port the input routing and immediate-mode layout, not the whole crate | M | if `crates/game/src/console.rs` can be written in 200 lines without it, egui is dead weight and the row dies | PORT |
| 4 | A generational key and a dense/secondary map for asset handles | `orlp/slotmap` - https://github.com/orlp/slotmap (crates.io 1.1.1, read 2026-10-20) | Zlib | A2 (content by fetch with one load queue); AS4 (asset handles, ticked but uncalled) | `src/secondary.rs` and `src/dense.rs` - port the stale-key refusal and the secondary-map shape into `crates/assets/src/handle.rs` | S | the engine handle already refuses stale keys (`stale_handle_never_panics_and_never_ready`): then port only the secondary map | PORT |
| 5 | A browser HTTP client with abort, for the load queue on the web | `rustwasm/gloo` - https://github.com/rustwasm/gloo (crates.io `gloo-net` 0.7.0, read 2026-10-20) | MIT OR Apache-2.0 | A2 (one load queue: a thread natively, `fetch` on the web) | `crates/net/src/http/request.rs` - port the abort and the byte-budget shape; the native half stays a thread | M | if `crates/web` already has a fetch path with cancel, this row dies | PORT |
| 6 | A hashed dense search leg that is allowed to exist without changing the BM25 ranks | `unum-cloud/USearch` - https://github.com/unum-cloud/USearch (crates.io 2.26.5, read 2026-10-20) | Apache-2.0 | N-25 (local search bundle); E04-6 (a dense leg that may not change ranks); the empire-wide BM25 in `jobhunt\tools\lib\bm25.mjs` and `asset-vault\tools\find.mjs` | `include/usearch/index.hpp` - port the int8-quantised vector idea and the neighbour-list budget; use the donor as a referee, not as a dependency | M | the 90% BM25 gold floor (`endpoint/src/insight/eval.rs:499-519`) already answers every query: the dense leg stays off | RESEARCH-ONLY |
| 7 | The reference ORCA solver, as a referee for our integer ORCA | `snape/RVO2` - https://github.com/snape/RVO2 (read 2026-10-20); paper: https://doi.org/10.1109/ICRA.2011.5980409 | Apache-2.0 | NV10 (broadphase plus a 1000-agent bar); the arena crowd row | `src/Vector2.h`, `src/Agent.h`, `src/KdTree.h` - port the one-dimensional linear-programming velocity solve and the kd-tree neighbour rule | M | if our ORCA already matches the scan referee on 200 seeded layouts and the 1000-agent p50 is under the bar, the donor stops at a test oracle | PORT (test-only oracle) |
| 8 | A memory stream with three retrieval scores, for agent memory | `joonspk-research/generative_agents` - https://github.com/joonspk-research/generative_agents (read 2026-10-20); paper arXiv:2304.03442 https://arxiv.org/abs/2304.03442 | Apache-2.0 | D18 (agent memory and a task suite); `crates/ai/src/memory.rs` (one caller, in `crates/game/src/ludus_npcs.rs`); forge's `kits/ai/TemporalGraphKit.cs` (no caller) | `reverie/backend_server/persona/` - port the recency + importance + relevance scoring rule into `crates/ai/src/memory.rs` | M | if the tiered recall already answers every recall test in `crates/ai`, the donor stops at the paper's rule | PORT |
| 9 | A reliable UDP transport to stand behind forge's orphan net kits | `RevenantX/LiteNetLib` - https://github.com/RevenantX/LiteNetLib (read 2026-10-20) | MIT | empire: forge G8 (the Rome show game) and the net kit shelf - 7 files with proofs and no game: `NetInputLog`, `NetProtoRegistry`, `NetSnapshotBudget`, `ServerLifecycle`, `SessionPrivacyKit`, `SubtickKit`, `BotReplayTape` | `LiteNetLib/LiteNetManager.cs` and `LiteNetLib/InternalPackets.cs` - port the reliable-ordered-channels shape, then wire `NetProtoRegistry` to it | M | if forge's net lane stays a plan with no game, the transport has no home and the row dies with it | PORT |
| 10 | One MCP server shape for the ten hand-written sidecars | `modelcontextprotocol/python-sdk` - https://github.com/modelcontextprotocol/python-sdk (read 2026-10-20) | MIT | empire: forge G1/G3 (one tool surface); center C1 (every repo has a finish line with a proof) | `src/mcp/server/mcpserver/__init__.py` - port the one-decorator-per-tool shape into the sidecar base; each sidecar keeps its `refuse_tests.py` | M | if the sidecars are only used by their own CLI, the MCP shape adds nothing and the row dies | PORT |
| 11 | A task-suite and scorer runner, so a proven skill has an outside signal | `UKGovernmentBEIS/inspect_ai` - https://github.com/UKGovernmentBEIS/inspect_ai (read 2026-10-20) | MIT | empire: skillworks S7 (8 skills with live lift, 3 today); engine finish-line gap 18 (10 build tasks scored by programmatic referees, no LLM judge, Play@k) | `src/inspect_ai/_eval/task/task.py` and `src/inspect_ai/dataset/_dataset.py` - port the task/solver/scorer split; the skillworks trials sheet becomes a dataset | M | if the existing `tools/skill_cover.py` + `evals/*_trials.jsonl` already give every skill a repeatable score with a model-written answer, the donor is redundant | PORT |
| 12 | A clean job-source schema and a wider board list, for jobhunt's empty families | `speedyapply/JobSpy` - https://github.com/speedyapply/JobSpy (read 2026-10-20) | MIT | empire: jobhunt JH-106 and JH-112 (source coverage), JH-150 and JH-154 (three families with 0 fit-60 listings) | `jobspy/model.py` and `jobspy/records.py` - port the normalised job schema and the per-source request shape; obey each site's terms and robots.txt | S-M | if the added sources bring no new fit-60 row in three families, the row dies and the families stay empty | PORT |

## Rejections

- `auto-editor` (https://github.com/WyattBlue/auto-editor, read 2026-10-20): the project is now written in
  Nim (`ae.nimble`, `config.nims`), so it is not a donor for the Python video lane. Checked and dropped.
  The BSD-3-Clause alternative for the same lane is `Breakthrough/PySceneDetect`
  (https://github.com/Breakthrough/PySceneDetect, `scenedetect/detectors/content_detector.py`, read
  2026-10-20).
- `emilk/egui` for the render debug lines: egui is an immediate-mode UI, not a line renderer. Keep it for
  EG26's console only; the debug-draw lines stay with our own `crates/render/src/debug_draw.rs`.
- `foxglove/foxglove-studio` (https://github.com/foxglove/foxglove-studio): the page 404s today, so the
  viewer is not open source. Use the MIT `mcap` crate and a reader we write.
- `jlowin/fastmcp` (Apache-2.0, read 2026-10-20): the Python package layout moved and
  `src/fastmcp/server/server.py` 404s today, so its donor path is not stable. The official
  `modelcontextprotocol/python-sdk` (MIT) is the stable one and covers the same need.
- `Sollimann/bonsai` and `victorb/dogoap` (MIT): already used - GA11 and GA12 name them. They are not
  new finds, and the problem is the missing caller, not a missing donor.
- `Bevy` (MIT OR Apache-2.0): already mined, and AS4 already names the Bevy asset pattern.
- `BehaviourTree/BehaviorTree.CPP` (MIT, https://github.com/BehaviorTree/BehaviorTree.CPP, read
  2026-10-20) and `Crashkonijn/Goap` (Apache-2.0, https://github.com/Crashkonijn/Goap, read 2026-10-20):
  both are fine, but the engine already owns a BT executor and a GOAP planner. They would be a second
  implementation, so: rejected.
- `jrouwe/Jolt`, `dimforge/Rapier`, `box3d-rust`, `rust-av/ssimulacra2`, `moq-dev/web-transport`,
  `meshoptimizer`, `gltfpack`: already in `docs/research/LADDER.md` and the RULES mined list.
- `thomcc/rust-slotmap`: the crate's repository is `orlp/slotmap` (the `thomcc` path 404s). Use the
  `orlp` one.
- `modelcontextprotocol/registry` (MIT with new code moving to Apache-2.0, LICENSE read 2026-10-20):
  a valid way to publish center's tool list, but the arsenal's real problem is 19 dead rows, not
  discoverability. Not needed for this wave.
- `LiteNetLib` under `MetalNem/LiteNetLib`: that path 404s today; the live repo is
  `RevenantX/LiteNetLib`.

## Top 3

1. **`foxglove/mcap` (MIT) - https://github.com/foxglove/mcap** - the engine already writes MCAP
   (`crates/observe/src/mcap.rs`) and nobody reads it. Port the reader and the record validation
   (`rust/mcap/src/read.rs`, `rust/mcap/src/records.rs`) and the rerun ring gets a viewer.
   **Closes: PF9** (replay visual debugger), and part of **N-11** (one door-owned ring for the heals).
2. **`TimonPost/cargo-unused-features` (MIT) - https://github.com/TimonPost/cargo-unused-features** -
   the engine's `verify`, `geo`, `intake` and `rules` features are off by default, so 57 playtest
   files and a whole verifier pack sit in the tree unreachable. Port the feature-graph walk
   (`src/main.rs`) into the existing `tools/packet-gate.mjs`.
   **Closes: AD5** (unwired-code gate) and makes **finish-line gap 19** ("verifier output in every
   build verb") reachable.
3. **`snape/RVO2` (Apache-2.0) - https://github.com/snape/RVO2** - the engine's real ORCA solver and
   its grid/kd-tree neighbours live in a test file. Port the velocity solve and the neighbour rule
   (`src/Vector2.h`, `src/Agent.h`, `src/KdTree.h`) and the crowd stops being a test.
   **Closes: NV10** (broadphase plus a 1000-agent bar) and gives the arena fighter steering a real
   crowd.
