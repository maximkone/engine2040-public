# 14 — Smart AI-native engine: what is smart, and what is written but switched off

Lane N4. Date read: 2026-10-20. All tree facts below were checked today with `git grep`,
`git ls-files` and `node tools/wiring-scan.mjs --orphans` (that tool printed `islands: 6460`
for the whole repo). Evidence grades: OBSERVED (I read the file today), DERIVED (reasoned
from what I read), INFERRED, UNKNOWN.

## Answer

Fast is the fixed tick: one hash per tick, integer math, `d57b5c8ae3096614` unmoved. Smart is
five more things, and this engine already has four of them. It can (a) run a **learned** policy
inside the tick from bytes it ships itself — `crates/nn` is an int8-only `.e2nn` runtime with a
zero-allocation `infer_into`, `crates/distill` trains the student offline, and
`fixtures/nets/depth_student.e2nn` is a tracked file today (OBSERVED, `git ls-files`);
(b) **ground** its decisions in an exact integer relation graph — `crates/spatial/src/relation.rs`
builds Supports/OnTopOf/Inside/NextTo/Facing and the door reads it (`crates/endpoint/src/scene_query.rs:1153`);
(c) **remember** — `crates/ai/src/memory.rs` + `mind.rs` give bounded per-NPC salience memory and
reflection, wired through `crates/game/src/ludus_npcs.rs`; (d) **choose** — three decision families
exist: utility AI (live, `crates/ai/src/utility.rs` → `gladiator.rs`), behaviour trees
(`crates/ai/src/bt.rs`, live only in the playtest bot harness), GOAP (`crates/ai/src/goap.rs`);
(e) **work with no provider at all** — cassettes, a scripted fallback
(`crates/provider/src/fallback.rs`), and a provider-off replay that returns the talk log byte for
byte (NS14 landed). What the engine is **not** yet is *learned in the arena*: `NetPolicy` has no
caller, the ORCA crowd solver has no caller, the goal stack has no caller, the arbiter has no
caller. The smart layer is a set of islands. The engine is fast and mostly smart, but the
**learned** and **crowded** halves of the smart layer are unreachable code, not features.

### Corrections to the brief (checked today)

- `crates/ai/src/utility.rs` is **not** dead. `crates/ai/src/gladiator.rs:144,168,179,219-269`
  and `crates/ai/src/personality.rs:155-159` call `utility::all_or_nothing / product_of / sum_of /
  clamp01 / pick_gated / Scorer / Thinker`. `docs/plan/forge-adoption.md` A.3 #5 is stale on this one.
- `crates/nav/src/crowd.rs` **does not exist** (rot cleanup of 2026-10-16). The ORCA module is
  `crates/nav/src/orca.rs`, and the NV10 row text says so itself.
- `engine-playtest` **has** a consumer now: `crates/endpoint/Cargo.toml:16,75` (optional, behind
  `mcp`), `crates/endpoint/src/packs/verify.rs:34-37`, `crates/endpoint/src/look_host.rs:7`, and the
  parity test `crates/endpoint/tests/door_parity.rs`. A.3 #3 / AD-08 is closed. The mirror suites
  still drive the private stand-in `crates/playtest/src/door.rs` (`eval.rs:30`, `mirror_craft.rs`,
  `mirror_gdb.rs`, `mirror_kit.rs`, `mirror_logic.rs`) — that is what the parity test compares.
- "No `.e2nn` is in the tree" is **stale**: `fixtures/nets/depth_student.e2nn` is tracked. EG24 is
  open only on (d) the arena bind and (e) the gate, exactly as the row text says.
- `crates/spatial/src/relation.rs` is **live**; `SPATIAL_CHECKS` is 18 (`crates/spatial/src/checks.rs:13`).
- `crates/ai/src/bt.rs` already has a versioned decoder with a load-time validator
  (`BtError`: BadVersion, BadTag, BadCond, Truncated, Trailing, TooManyNodes, TooDeep, BadAction,
  BadSlot, BadChildCount; `BtTree::decode` at `bt.rs:449`). GA11's "data tree + validator" is
  half done; what is missing is a consumer, not a decoder.
- Provider parity exists at the **contract** level: `crates/provider/src/suite.rs` runs the same
  failure modes through five adapters (`anthropic.rs:143`, `gemini.rs:112`, `local.rs:181`,
  `ollama.rs:116`, `openai.rs:132`). Door-level parity (NS13) is still open.
- `content/reference/` **does not exist**; only `crates/endpoint/src/packs/view.rs:1700` names the
  path. The Q1 reference set is empty, so the Look reference bundle has no baseline.
- Row map for the rest of the lane's list: **E46** (`misc: heal + bevy + verdict + showcase gate`)
  owns the audio heal test and the showcase gate leg; its heal half is the consumer row for
  `render::RenderHeal` and `audio::DeviceHeal` (dead-code row 11). **GP33** (time-to-kill and
  matchup table) is the consumer row for GA14's difficulty sweep and GA15's primus phase 2;
  **GP41** (charters with a question) is the in-tree half of D18's task suite. **AD5** landed
  2026-10-20 as `tools/wiring-scan.mjs --diff --unlocks --orphans`, so every dead-code row below
  is a row that the gate can now name; **LF15** and **GA11** are already ticked, **NV10** is
  ticked against its replacement module `nav::orca`.

## Dead-code classes: capability that exists but is unreachable

| # | Module | git grep today | What a real consumer would call it | Row id |
|---|---|---|---|---|
| 1 | `crates/ai/src/behavior.rs` (275 lines, GA11 BT over the legacy `Brief`/`Op` seam) | 0 hits outside the file; `gladiator.rs:12` explains why it is not used ("those score a [`Brief`]") | `crates/arena/src/fork.rs` builds one tree per fighter and ticks it with the recorded `Percept` | GA11 (retire or wire), AD5 |
| 2 | `crates/ai/src/goap.rs` + `crates/ai/src/blackboard.rs` (`Blackboard`, `TaskStatus`, `goap_plan`) | only in-file tests and the `pub use` in `lib.rs` | the gladiator asks `goap::plan` once per round from `content/design/enemies.rows` | GA12 |
| 3 | `crates/ai/src/goalstack.rs` (ioq3 goal stack + fuzzy weight table) | only in-file tests | `gladiator::Brain` owns a `GoalStack` and asks `choose_ltg` / `choose_nbg` each tick | GA11, BR1 |
| 4 | `crates/ai/src/gladiator.rs::NetPolicy` (EG24's int8 in-tick fighter) | `NetPolicy::load` only inside `crates/ai` | `crates/arena/src/fork.rs` gains `ArenaPolicy::Net`, pinned by the net digest in the tape header | EG24 (d) |
| 5 | `crates/nav/src/orca.rs` (971 lines: ORCA half-planes + counting-sort broadphase) | `Orca` only inside `crates/nav/src/orca.rs` | `crates/ai/src/depth/steer.rs` calls `Orca::step` when two fighters are inside near-field range | NV4, NV10 |
| 6 | `crates/nav/src/flow.rs` (tile flow-field bake + follow, one bake serves N agents) | 0 hits outside `crates/nav` | the legion's long-range path request in `crates/arena/src/fork.rs` | NV10, NS22 |
| 7 | `crates/anim/src/lod.rs` (`assign_tiers`, `tier_for`) | only `crates/anim/tests/it/anim_lod_pin.rs` | the presentation loop assigns a tier per skeleton per frame | NS23, E10-5 |
| 8 | `crates/anim/src/motion_net.rs` (`MotionNet::load_e2nn`, `infer_pose`) | outside anim only `state_code` is used (`crates/arena/src/lib.rs:105`, `spine_step.rs:27`, `sync.rs:22`) | the arena's net-driven pose path replaces the clip lookup | NS8, CT14 |
| 9 | `crates/assets/src/import_repair.rs` — `diagnose`, `diagnose_with`, `repair`, `BudgetSet` | `recompute_tangents` **is** called outside (`crates/render/src/v1.rs:256`, `present/gpu_present/presenter.rs:2202`); the repair half has 0 callers outside assets | `crates/assets/src/import.rs` diagnoses every imported glTF and refuses on the finding | N-01, AD3 |
| 10 | `crates/observe/src/rerun_sink.rs` (`RerunSink`, 120-frame ring, `first_divergent`) | only `crates/observe/tests/it/w1_pf9_rerun.rs` | `crates/game/src/crash.rs` writes the ring; the door's `log.query` names the first divergent tick | N-11, PF16 |
| 11 | `crates/render/src/heal.rs` (`RenderHeal`); `crates/audio/src/heal.rs` (`DeviceHeal`) | no production constructor anywhere; `DeviceHeal` is only used at `crates/game/src/audio_mix.rs:390`, which is inside a `#[cfg(test)]` module | the present loop reports present error / OOM / device-lost into it; the mixer reports a device change into the audio heal | N-11, AD14, E46 |
| 12 | `crates/provider/src/arbiter.rs` (`Arbiter::observe`, `PlacementReader`) | 0 hits outside `crates/provider` | the window build feeds live frame p95 and VRAM use; the arbiter publishes GPU / CPU / defer / remote | OPT27 |

## Donor table

| # | What it is | Donor + URL + date read | License | Fills (engine2040 row id) | What exactly to port (donor file path) | Cost S/M/L | Kill line | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | A VRAM-budget to layer-count rule, so one 6 GB GPU can hold both the renderer and a local model | `ggerganov/llama.cpp` `tools/server/README.md` (`-ngl, --gpu-layers N`, `-fit`, `-fitt --fit-target MiB0,MiB1`, `-ncmoe, --n-cpu-moe N`) and `gguf-py/gguf/constants.py` (`GGUF_MAGIC`, `GGUF_VERSION`, quant-type table); read 2026-10-20 | MIT (LICENSE SHA e7dca554bcb802f98408383a864404e3aa4eacca) | OPT27, LAI7 | the rule "device budget minus a per-device margin = a layer count", plus the GGUF header fields the tape can pin | S | the arbiter still needs a human to pick the layer count, or the tick waits on the request | PORT |
| 2 | A free local text model that fits beside the renderer in 6 GB | `Qwen/Qwen3-0.6B` model card + HF API, read 2026-10-20: apache-2.0, 751,632,384 params | Apache-2.0 (model card) | LAI7, OPT27, GA7 | the model bytes + digest row + the `llama-server` command line the local adapter already speaks (`crates/provider/src/local.rs` is a llama.cpp-server adapter, loopback only) | S | the model needs more than 2 GB VRAM, or the window build misses the P6 frame budget | PORT |
| 3 | A free local vision model for the look judge | `HuggingFaceTB/SmolVLM2-2.2B-Instruct` card, read 2026-10-20: apache-2.0 | Apache-2.0 | LAI5, LF15 (advisory half) | the adapter shape + the cassette/replay rule; the 50 owner-labeled captures stay an owner task | M | agreement below 80% on the labeled set, or the score cannot be replayed from a cassette twice | PORT |
| 4 | Per-instance verdicts and a run report, one bucket per failure kind | `SWE-bench/SWE-bench` `swebench/harness/reporting.py` (`make_run_report`: completed / resolved / error / unresolved / incomplete / infra_failure / ambiguous / empty_patch) and `swebench/harness/grading.py`; read 2026-10-20 | MIT (LICENSE SHA 969657c73e7be4c9a796dfac715fb0d57b2acf8a) | D18, D19 | the bucket set and the roll-up shape (one JSON per instance + one summary), so every build verb names its own failing instance | S | a build verb cannot name the instance id that failed | PORT |
| 5 | A scorer/reducer layer that turns per-sample values into one metric and keeps the sample count | `UKGovernmentBEIS/inspect_ai` `src/inspect_ai/scorer/_scorer.py`, `src/inspect_ai/scorer/_reducer/reducer.py`, `_reducer/registry.py`; read 2026-10-20 | MIT (LICENSE SHA 72fc87742ef8a944fab4f28fe6231696c62f2fa4) | D18 (Play@k, pass rate per model), D19 | the reducer shape: a list of per-sample values to one metric, with the sample count and the seed kept | S | the score needs an LLM judge (LAI5 says no LLM judge) | PORT |
| 6 | A data behaviour tree that is validated at load time | `ncrothers/behaviortree-rs` `behaviortree-rs/src/tree.rs` (typed `ParseError`: `UnknownNode`, `NodeTypeMismatch`, `MissingRoot`, `UnexpectedEof`, `MissingAttribute`) + `basic_types.rs` (`NodeStatus`, `PortChecks`); read 2026-10-20 | MIT (LICENSE SHA 03cd65f7b5d869d2b390c7720fbbb3975b8d7462) | GA11 | the error taxonomy and the "children do not match the node type" check; then retire `ai::behavior` and give `ai::bt` a data file plus this validator | S | `ai::bt`'s 11 `BtError` variants already refuse every shape the donor refuses, so the port adds nothing | PORT |
| 7 | GOAP actions as loadable rows (name, cost, pre, post) | `tynril/rgoap` `src/lib.rs` (`State = BTreeMap<String,bool>`, `Action{name,cost,pre_conditions,post_conditions}`, `plan()` over `pathfinding::prelude::astar`, `use_serde` derive); read 2026-10-20 | MIT (LICENSE SHA a6825ef18fc62f46cbb1a57b6a820b731302f1ae) | GA12 | the Action-as-row shape into `content/design/*.rows` plus a loader; keep the engine's 64-fact integer state (the donor's String keys are a downgrade) | S-M | the loader needs string keys, so the port loses the integer-only rule | PORT (shape only) |
| 8 | One task list run against many backends with the same outcome digest | `EleutherAI/lm-evaluation-harness` (task + adapter registry), read 2026-10-20 | MIT (LICENSE.md SHA 12e6063183935e876e232db276568baf4954b492) | NS13, G7 | the task-to-adapter registry so the same 20 authoring tasks run on mock, local and remote with an equal outcome digest | M | one of the three backends needs a different task body | PORT |
| 9 | A small grounded VLM for referring expressions (231,567,705 params) | `microsoft/Florence-2-base` card, read 2026-10-20: license `mit` | MIT | SP33, SP20 | the grounded-output shape (`<loc_x>` boxes) turned into integer-cm node candidates over `crates/spatial/src/relation.rs`, ranked and advisory only | M | the engine's own relation graph answers the same questions exactly, so the model adds cost without answers | RESEARCH-ONLY |
| 10 | A bounded agent memory with add / search / update and a source per memory | `mem0ai/mem0` (memory add/search/update, graph of relations); read 2026-10-20 | Apache-2.0 (LICENSE SHA d20d5102c3cf97ecbee54afd65893de4a11d26fe) | D18 (project memory: decisions, failed fixes, tuned values, each with its tape hash) | the add/search/update split and the "every memory carries its source" rule, reimplemented over the engine's integer store | M | the memory path needs a network or an embedding model, so it can never be the runtime path | RESEARCH-ONLY |
| 11 | A live motion tracker that turns a clip into servo targets | `google-ai-edge/mediapipe` (pose landmarker + confidence per landmark); read 2026-10-20 | Apache-2.0 (LICENSE SHA 0e03e3911e0ac02c72ed527b34efe250f0fb9bef) | NS8 | the landmark-to-servo mapping plus the per-frame confidence gate (never a hard target under a floor); the clip-to-servo path stays offline | M | the tracker needs a model download at runtime, or the PC has no camera, so NS8 stays the offline path | RESEARCH-ONLY |
| 12 | A free offline locomotion teacher with no closed SDK | `google-deepmind/mujoco` (CPU physics, Apache-2.0, LICENSE SHA d645695673349e3947e8e5ae42332d0ac3164cd7). The classic curriculum repo `leggedrobotics/legged_gym` is BSD-3-Clause but needs the closed NVIDIA Isaac Gym download | Apache-2.0 (MuJoCo) / BSD-3-Clause + closed SDK (legged_gym) | GA7, EG24 | only the observation/action contract and the domain-randomisation list, into `crates/distill` (offline, never in the tick) | L | the teacher needs a GPU and a Python environment the build does not have, or the student loses to the scripted brain | RESEARCH-ONLY |

## Rejections

- `Sollimann/bonsai` (MIT) — already mined by the GA11 row (STEAL 2026-10-03). New fact: its trees are Rust structs, not data, so `crates/ai/src/bt.rs` is already ahead of it. Not a new find.
- `zkat/big-brain` (Apache-2.0) — already mined (LADDER card "composite AND/OR scorers"), and the composites it describes now live in `crates/ai/src/utility.rs` and are called by `gladiator.rs`. Correction, not a find.
- `victorb/dogoap` (MIT) — already ported into `crates/ai/src/goap.rs`; the file cites it. The missing half is a consumer, not a donor.
- `pengsongyou/OpenScene` (Apache-2.0) — read 2026-10-20; a dense 3D scene graph, but it needs a GPU and big weights while `crates/spatial/src/relation.rs` already answers the same questions exactly. Research-only, and row 9 is cheaper.
- `Qwen/Qwen2.5-VL-3B-Instruct` — license is `qwen-research`, not Apache-2.0. Research-only for a commercial build; `SmolVLM2` is the free replacement.
- `google/gemma-3-270m-it` — license `gemma`, not OSI. Research-only.
- `Tencent/legged_rl_gym` (BSD-3-Clause) — still needs Isaac Gym or MuJoCo for the training loop; same closed-dependency problem as row 12.
- `Farama-Foundation/PettingZoo` (MIT, LICENSE SHA ab161e2957698a097cc09ad06054321856ef04ed) — a good multi-agent API, but `crates/arena/src/fork.rs` already spawns its own branches. No gap to fill.
- `promptfoo`, `ollama` — already mined per RULES.md.
- `content/reference/` (Q1 reference set) — no donor: it needs the owner's own labeled captures. Not researchable.
- NVIDIA RTX SDKs, Unreal, Unity, Isaac Gym, Isaac Sim — proprietary. Research-only, never vendored.

## Top 3

1. `ggerganov/llama.cpp` — `tools/server/README.md` + `gguf-py/gguf/constants.py` — MIT — closes **OPT27** (and LAI7): a VRAM budget becomes a layer count, so the arbiter can place local inference on the GPU, the CPU, defer it, or route it remote without a human. Cost **S**.
2. `SWE-bench/SWE-bench` — `swebench/harness/reporting.py` + `grading.py` — MIT — closes **D18/D19**: every build task gets a per-instance programmatic verdict and a run report, no LLM judge. Cost **S**.
3. `Qwen/Qwen3-0.6B` (Apache-2.0, 751,632,384 params) + `UKGovernmentBEIS/inspect_ai` `src/inspect_ai/scorer/_reducer/reducer.py` (MIT) — closes **LAI7 / D18**: a free model that fits beside the renderer, scored by a reducer that keeps the sample count. Cost **S-M**.
