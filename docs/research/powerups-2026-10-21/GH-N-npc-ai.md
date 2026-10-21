# GH-N — NPC AI techniques a deterministic fixed tick can host

Lane question: which open-source GitHub repos hold techniques for GAME NPC AI that a deterministic
fixed tick can host — behaviour trees, utility AI, GOAP, blackboard architectures, squad and role
AI, steering and flocking, and LLM-driven NPC brains that stay deterministic?

## Answer

**The tree already owns most of the classic NPC-AI vocabulary, so the gap is not "a BT library" or
"a GOAP library" — it is three specific seams that the 6v6 war game needs and that no donor in this
lane fills for free.** OBSERVED by `git grep` on 2026-10-21: `crates/ai/src/bt.rs` is a serializable
integer behaviour tree (encode/decode, `tree_hash`, `BtBlackboard` with 8 slots, `MAX_NODES` 64,
`MAX_DEPTH` 8); `crates/ai/src/behavior.rs` is the BT executor; `crates/ai/src/goap.rs` is an A\*
GOAP planner over a 64-bit integer fact state; `crates/ai/src/blackboard.rs` is a sorted string→u64
blackboard; `crates/ai/src/utility.rs` is the utility picker (already fed by
`zkat/big-brain` composites); `crates/ai/src/legion.rs` is a 500-soldier squad brain with
formations, `Role::{Holder,Flanker}` and six `Order`s; `crates/ai/src/depth/steer.rs` is integer
steering with flock weights ported from `AizenKai1001/lunaris`; `crates/nav/src/orca.rs` is an
integer RVO2 port. Three of these are *unwired*: `bt.rs`/`behavior.rs`/`goap.rs`/`blackboard.rs`
have no caller outside `crates/ai` (board rows **PW31** and **PW58** say exactly this),
`nav/src/orca.rs` has 0 callers (board row **TC4**), and `eqs_bind.rs` is a `pub mod` with no
external caller found by `git grep -n "eqs_bind" -- crates/` beyond its own file and `lib.rs:62`.

**The three seams the war game actually needs are: (1) a capture-point state machine and the AI that
holds and fights for it — RY4/RY5 in `docs/plan/ryl1.md`, and NOTHING in the tree knows about
capture points** (`git grep -i "capture|objective|control_point" -- crates/` returns only a
`Slot::Objective` loot enum in `crates/world/src/roster.rs:41`, nothing in `crates/ai` or
`crates/rules`); (2) squad-level role and target assignment across the four AI seats per side —
`crates/ai/src/legion.rs` assigns roles per *squad* of 8 at 500-soldier scale, and nothing assigns
roles at the 4-seat-per-side scale the proof game uses; (3) an LLM-driven NPC brain that stays
deterministic under replay — `crates/ai/src/mind.rs` has the contract and
`crates/game/src/ludus_npcs.rs` has the provider-off replay, but no *combat* NPC uses it (the
ludus NPCs talk; the arena brains are `crates/ai/src/depth.rs`, pure integer).

**The best donor for (1) is `screeps/engine`**: its controller is a real capture point with an
owner, a progress counter, a downgrade timer, a safe mode, and a reservation — and the three
intent processors that mutate it (`claimController.js`, `attackController.js`,
`upgradeController.js`) plus `controllers/tick.js` are the exact arithmetic an integer capture-point
state machine needs. I read all four live. **The best donor for (2) is `bencbartlett/Overmind`**
(MIT): a real competitive RTS AI with per-role unit classes, an explicit priority ladder, a
"compute how many defenders a threat needs" formula, and a target-scoring rule — all of which map
onto a 4-seat squad. **The best donor for (3) is not a donor**: our own `mind.rs` + `ludus_npcs.rs`
already IS the deterministic-LLM-brain pattern, so that seam needs a *consumer*, not a third
library. The one genuinely new LLM-side find is `normal-computing/outlines` (Apache-2.0) for
*constrained* decoding: a grammar that makes the model emit only legal action ids, which is what
turns a free-text model into a tick-safe brain. Everything below is ranked best first.

**Grade key:** OBSERVED = I read the file or page today (2026-10-21). DERIVED = I reasoned from what
I read. INFERRED = likely, not proven. UNKNOWN = not measured here. No donor's benchmark is quoted
as our runtime result.

| # | What it is | Donor + URL + date read | License | Fills (engine2040 row id) | What exactly to port (donor file path) | Cost S/M/L | Kill line | Verdict |
|---|---|---|---|---|---|---|---|---|
| 1 | Capture-point state machine: owner, progress counter, level, downgrade timer, safe mode, reservation. Three intents mutate it (claim / attack / upgrade) and one per-tick processor ages it. | `screeps/engine` — https://github.com/screeps/engine (read live 2026-10-21, `src/processor/intents/controllers/tick.js`, `src/processor/intents/creeps/claimController.js`, `attackController.js`, `upgradeController.js`, `src/game/structures.js`) | ISC-style permissive ("Permission to use, copy, modify, and/or distribute this software for any purpose with or without fee" — `LICENSE.txt` read live) | **RY4** (capture point state machine) + **RY5** (6v6 match loop) — RY rows are in `docs/plan/ryl1.md`, not yet on the board | Port the *shape*, write our own code: (a) the controller field set — owner, `level`, `progress`, `downgradeTime`, `safeMode`, `reservation` — from `src/game/structures.js:174-186` (OBSERVED: `StructureController` exposes exactly `ticksToDowngrade`, `reservation{username,ticksToEnd}`, `level`, `progress`, `progressTotal`, `upgradeBlocked`, `safeMode`, `safeModeCooldown`, `safeModeAvailable`); (b) the *contest* rule from `src/processor/intents/creeps/attackController.js:41-48` — an attacker inside range 1 subtracts `CLAIM_COUNT * CONTROLLER_CLAIM_DOWNGRADE` from the point's `downgradeTime` and sets `_upgradeBlocked = gameTime + CONTROLLER_ATTACK_BLOCKED_UPGRADE` (OBSERVED); (c) the *hold* rule from `src/processor/intents/creeps/upgradeController.js:67-81` — a worker inside range 3 adds `boostedEffect` to `progress`, and crossing `CONTROLLER_LEVELS[level]` raises the level and carries the remainder (OBSERVED); (d) the *decay* rule from `src/processor/intents/controllers/tick.js:49-64` — when `gameTime >= downgradeTime-1` the point loses a level and at level 0 it is released, `progress` reset, `user` null (OBSERVED). Numbers we invent; the *relations* are the steal. | M | Kills the row if the point is not on the hash: if RY4 ends up scoring the point from player positions instead of a point-owned integer, this donor's whole value (an owner+progress pair the tick reads) is gone. Second kill: if the war rules need a *continuous* contest (two teams pushing a progress bar) rather than a discrete claim, the level/progress pair still applies but the downgrade-timer half does not, and the row shrinks to S. | PORT |
| 2 | Squad brain for a 4-seat side: explicit role classes, a priority ladder, a "how many defenders does this threat need" formula, and a target-scoring rule. | `bencbartlett/Overmind` — https://github.com/bencbartlett/Overmind (read live 2026-10-21, `src/creepSetups/setups.ts`, `src/priorities/priorities.ts`, `src/targeting/CombatTargeting.ts`, `src/overlords/defense/meleeDefense.ts`, `src/overlords/colonization/claimer.ts`) | MIT (`LICENSE` read live 2026-10-21, SHA 92935c3af…) | **RY5** (four AI seats per side act without a model in replay) + **GA9** (legion-vs-legion fixture with orders) | Port the *shape* into `crates/ai/src/legion.rs` (or a new war-side module): (a) the role table — `src/creepSetups/setups.ts:6-26` gives named roles (`worker`, `melee`, `ranged`, `healer`, `claim`, `scout`) each with a per-role *body pattern* in `Setups`/`CombatSetups` (OBSERVED); our war classes Knight/Archer/Sorcerer from RY2 map onto exactly this slot; (b) the priority ladder — `src/priorities/priorities.ts:1-8` gives six named priorities (Critical/High/NormalHigh/Normal/NormalLow/Low) (OBSERVED), which becomes our per-seat duty order (hold point > contest > retreat); (c) the target score — `src/targeting/CombatTargeting.ts:48-52` (`findTarget`) scores a foe as `hitsMax - hitsPredicted + healPotential - 10 * range` (OBSERVED), and the in-range variant at `:19-22` drops the range term; we restate it as an integer formula over `MeleeView` (`crates/ai/src/melee.rs:104`, `NEAR_FOES = 3`); (d) the defender-count formula — `src/overlords/defense/meleeDefense.ts:39-46` computes `Math.ceil(.5 + 1.5 * healAmount / (worstDamageMultiplier * (zerglingDamage + towerDamage + 1)))` (OBSERVED), i.e. *how many of my four seats do I send* — we restate it as "how many of four". | M | Kills the row if the war AI turns out to be one seat per class with no cross-seat choice — then the priority ladder and the defender count have no consumer and only the target score survives, which is a one-day row, not a week. Second kill: if `crates/ai/src/legion.rs` cannot be reused because it assumes 8-soldier squads and 500-soldier budgets, the port becomes a new module and the row's cost is honestly L. | PORT |
| 3 | Role-aware "grab the point" behaviour written as *data* — a per-objective aspect that scores candidate locations and returns a rating, so the objective choice is a table, not code. | `wesnoth/wesnoth` — https://github.com/wesnoth/wesnoth (read live 2026-10-21, `data/ai/lua/ca_grab_villages.lua`, `data/ai/lua/ca_retreat_injured.lua`, `data/ai/lua/ca_move_to_any_enemy.lua`) | **GPL-2.0** (`COPYING` read live 2026-10-21) — **RISK LINE: copyleft. Research-only for code; ideas and table shapes are re-derived, no Lua is copied.** | **RY4** (capture point state) + **RY5** (AI holds and fights for a point) | Read `data/ai/lua/ca_grab_villages.lua` for the *shape* only: it enumerates candidate objective locations on the map and scores each one — a fixed +100 when an enemy can reach it in one move (OBSERVED, lines 66-69) and +10000 when the location is unowned (lines 71-74) — then returns a rating. The steal is "the objective's value is a scored table over map locations, and enemy reachability is one term in it". Do not port the Lua. Our version: one integer row per capture point in the class/rules data, with reachability as a term. | S | Kills the row if the three capture points are fixed map positions with hand-tuned behaviour — then a scored table is ceremony. Also kills it if we cannot write our own reachability term, since the value of the shape is the *scoring*, not the enumeration. | RESEARCH-ONLY (copyleft) |
| 4 | Hierarchical task network: compound tasks with named methods, conditions, and a planner that decomposes a goal into a task chain — the missing third layer above BT and GOAP. | `ptrefall/fluid-hierarchical-task-network` — https://github.com/ptrefall/fluid-hierarchical-task-network (read live 2026-10-21, `Fluid-HTN/Tasks/ITask.cs`, `Fluid-HTN/Tasks/TaskStatus.cs`, `Fluid-HTN/Planners/Planner.cs`, `Fluid-HTN/BaseDomainBuilder.cs`) | MIT (`LICENSE` read live 2026-10-21, SHA 2728a534fa…) | no row, propose a new row **"HTN goal decomposition for war objectives"** (a capture-point goal like "hold the east point" decomposes into move → contest → defend; our BT and GOAP cannot express that ordered chain — BT decides per tick, GOAP plans over 64 unordered facts) | The idea: a task has *conditions* and a *status* and lives in a *hierarchy* (`ITask.Parent`, `ITask.Conditions`, `ITask.IsValid(ctx)` — OBSERVED), and a compound task holds a method per situation. Port the *shape*: one `WarGoal` enum (Hold/Contest/Retreat), each with an ordered list of integer-guarded steps, planned on a goal change like GOAP already does. We write the code; no C# is ported. | M | Kills the row if BT+GOAP already cover the three war goals — GOAP's A\* over 64 facts can express "hold point" if the facts are `at_point`, `point_contested`, `hp_low`, so if the fact table can be written to cover it, HTN is a third planner in one crate and this row dies. Also dies if the owner says one planner only (PW31/PW58 pick BT-per-tick + GOAP-on-goal-change). | RESEARCH-ONLY (until the BT/GOAP fact table is shown to be too small) |
| 5 | Typed HTN world state: a value that is one of bool/int32/float/string/list, bound and unbound by scope, so a plan can read a *number* (a distance, a count) and not only a bit. | `Sandruski/htn-planner` — https://github.com/Sandruski/htn-planner (read live 2026-10-21, `HTNFramework/src/Core/HTNAtom.h`, `HTNFramework/src/Core/HTNScope.cpp`) | MIT (`LICENSE` read live 2026-10-21, SHA 6826619d57…) | no row, propose a new row **"typed facts in the GOAP world state"** (our `goap.rs` `Action` is bit-masks only: `pre_mask`/`pre_value`/`set_mask`/`set_value` on a `u64` — OBSERVED at `crates/ai/src/goap.rs:22-36` — so a fact can be "am I on the point" but not "how many enemies are on the point") | The idea: one atom carries a typed value, and a scope binds/unbinds it, so a plan condition can compare a number. Our version: keep `u64` but let a fact *mean* a small integer (0..=N encoded in a field) rather than a bit — the donor's contribution is the typed value and the scope binding, not the C++. | S | Kills the row if a bit-per-entity fact table (one bit per enemy per point, which is 12 bits at 6v6) is enough — at our scale it is, so this row is only alive if the count matters, and the count is a score, not a state. | RESEARCH-ONLY (small, and at 6v6 the bit table wins) |
| 6 | Deterministic utility AI as *stateless scoring of advertised actions*: the world advertises what it offers, the agent scores each offer against its needs with a response curve, personality and context multipliers, and a cost penalty, then picks by softmax over the top K. | `Otoru/geppetto` — https://github.com/Otoru/geppetto (read live 2026-10-21, `selection.go`, `types.go`, `curves.go`, `LICENSE`) | MIT (`LICENSE` read live 2026-10-21, SHA 1a41b2248c…) | no row, propose a new row **"needs-driven seat duty scoring"** (our `utility.rs` scores five fixed intents inside `gladiator.rs` — OBSERVED `crates/ai/src/utility.rs` `pick_gated`, `Gate::Highest` — with no notion of the world offering choices) | The idea: separate *scoring* from *choosing*, and let the scorer take a consideration set (needs) plus a cost term that stays a penalty (`ScoreAction` in `selection.go` — OBSERVED: `considerationBenefit * personalityMultiplier * contextMultiplier * distanceMultiplier - weightedCost`, and the comment "costs remain penalties so a cheap action cannot become expensive through a positive modifier"). Our version: integer only — replace the float multipliers and the softmax with an integer argmax plus a seeded tiebreak, and keep the "cost is never multiplied" rule. | M | Kills the row if the seat duty is decided by the BT/GOAP pair that PW31/PW58 already mandate — then utility scoring is a third chooser in the same crate and dies on the same "one brain, one consumer" rule. Also dies if the donor's float softmax and `math/rand/v2` cannot be restated as integer argmax without losing the "why" the score gives. | RESEARCH-ONLY |
| 7 | Constrained decoding for a model brain: a grammar makes the model emit only legal tokens, so a free model can never produce an illegal action and the tick never sees an unparsable proposal. | `normal-computing/outlines` — https://github.com/normal-computing/outlines (read live 2026-10-21, `LICENSE`, `src/outlines/`, repo tree) | Apache-2.0 (`LICENSE` read live 2026-10-21, SHA cac50cdfa…) | **NS14** (LLM NPC brain with provider-off replay) — already landed; this row is the *hardening* of it, plus **BX6** (capability observation and reaction: a model must be able to say "one-armed" and the sim must be able to read it) | The idea: the action grammar is the *interface* to the model — our `crates/provider/src/ladder.rs:361` `action_schema` already builds a JSON schema per action name, and `crates/provider/src/core.rs:399` already promises a "closed action grammar with bounded labels, coordinates and diagnostics" (OBSERVED). The steal is the *model-side* enforcement (regex/FSM-constrained sampling) so a free model cannot drift outside the schema — our provider is a *validator* today, not a *constrainer*. Port: an FSM/regex-constrained decode path for the action schema, keyed to the same `action_schema` output. | M | Kills the row if our provider's strict parse + provider-off replay (NS14 landed) already rejects every malformed proposal in CI — then the model-side constraint is belt-and-braces and the row is not worth a week. Also dies if the free local models we can ship (Qwen3-0.6B-class) cannot be driven by a constrained decoder at all on our potato target. | PORT (shape) |
| 8 | LLM workflow state with checkpointing so a model-driven agent replays identically with the provider off. | `dshills/langgraph-go` — https://github.com/dshills/langgraph-go (read live 2026-10-21, `LICENSE`) | MIT (`LICENSE` read live 2026-10-21) | **NS14** (already landed) — no new row | The idea only: "typed state, deterministic replay, checkpointing" is the same shape as our `crates/provider/src/cassette.rs` (which already records a request by digest + schema version and replays it) and `crates/ai/src/mind.rs` (which already keeps an `ActionLog` and a `MindRuntime::replay_offline`). Read it as confirmation that the shape is standard; port nothing. | S | Kills the row outright: this is a *correction*, not a find. `crates/game/src/ludus_npcs.rs` already replays the talk log byte-for-byte with the provider off (board row NS14 ticked, `cargo test -p engine-game --lib ludus_npcs` 13 passed per the row). Nothing here is missing. | REJECT (already in the tree — correction only) |
| 9 | Boids/flocking simulation in Rust: separation, alignment, cohesion over a neighbour set, with a spatial grid for neighbour queries. | `twitu/rboids` — https://github.com/twitu/rboids (read live 2026-10-21, `src/boid.rs`, `src/lib.rs`, `src/main.rs`, `LICENSE`) | MIT (`LICENSE` read live 2026-10-21, SHA 812f86c88c…) | no row, propose a new row **"flocking with real neighbours"** | The *idea* we still lack is narrow: our `crates/ai/src/depth/steer.rs:117` `flock_wish` is ported and unit-tested, but the live path calls it with an **empty** neighbour slice (`steer.rs:235` `flock_wish(mx, mz, &[])`) — OBSERVED, and `git grep -n "FlockNeighbour" -- crates/` finds construction only inside `steer.rs`'s own `#[cfg(test)]` block (lines 427-470). So the flock weights are dead in production. The steal is the *neighbour set construction* (a grid bucket per tick) and nothing else. | S | Kills the row if `crates/ai/tests/crowd_orca_bench.rs` (which already benches naive, kd-tree and grid neighbour feeds — per board row NV4) is judged sufficient and the arena keeps fighting as a duel. Also kills it if `crates/nav/src/orca.rs` gets its first caller under TC4, because ORCA then owns the separation and the flock weights stay unused on purpose. | PORT (the neighbour feed only) |
| 10 | Utility-AI composite scorers (AllOrNothing / Sum / Product with a 0..1 clamp) for a scored chooser. | `zkat/big-brain` — https://github.com/zkat/big-brain (already mined, LADDER card "composite AND/OR scorers") | Apache-2.0 | **GA10** (threshold-gated utility pick, landed) | Nothing. `crates/ai/src/utility.rs` already carries these composites and `crates/ai/src/gladiator.rs:144-179` calls them (`all_or_nothing`, `product_of`, `sum_of`). This is a CORRECTION: the donor was mined and the code is already ours. | S | Kills the row: it is already in the tree. Listed so nobody re-mines it. | REJECT (already in the tree — correction) |
| 11 | Rust behaviour trees for deterministic AI, with Python bindings. | `Sollimann/bonsai` — https://github.com/Sollimann/bonsai (already mined, STEAL 2026-10-03 / GA11) | MIT | **GA11** (deterministic behaviour-tree executor, landed) | Nothing to port: our `crates/ai/src/bt.rs` is already a *serializable* integer tree (encode/decode + `tree_hash`, `BT_VERSION`, `MAX_NODES` 64) — the wave-2026-10-20 lane recorded the same fact ("its trees are Rust structs, not data, so `crates/ai/src/bt.rs` is already ahead of it"). Correction. | S | Kills the row: already in the tree. | REJECT (already in the tree — correction) |
| 12 | Behaviour-tree runtime and a GOAP framework in C++/C# for comparison. | `BehaviorTree/BehaviorTree.CPP` — https://github.com/BehaviorTree/BehaviorTree.CPP and `Crashkonijn/Goap` — https://github.com/Crashkonijn/Goap (both read 2026-10-20 by the wave-2026-10-20 lane, recorded in `docs/research/powerups-2026-10-20/14-smart-ai-native-engine.md:96-98`) | MIT / Apache-2.0 (per that lane's reading) | **GA11**, **GA12** (both landed) | Nothing. That lane's verdict, quoted: "both are fine, but the engine already owns a BT executor and a GOAP planner. They would be a second implementation." We re-confirm by reading our own tree today: `crates/ai/src/behavior.rs` (executor) and `crates/ai/src/goap.rs` (A\* planner) both exist. Correction, not a find. | S | Kills the row: the second-implementation rule. Listed so nobody re-mines it. | REJECT (second implementation — correction) |

### Notes on the tree, so the next lane does not re-grep

- `git grep -n "bt::\|behavior::tick\|goap::plan\|blackboard::" -- crates/` returns hits **only inside
  `crates/ai`** (`crates/ai/src/bout.rs:9` uses `bt::BtBlackboard`; `crates/ai/src/plan_trace.rs:25`
  uses `goap::{Action, GoalState, PlanError}`). The BT executor, the GOAP planner and the blackboard
  have **no consumer outside their own crate** — board rows **PW31** and **PW58** own that.
  OBSERVED 2026-10-21.
- `git grep -n "orca::" -- crates/` finds no caller outside `crates/nav/src/lib.rs`; board row
  **TC4** owns the first caller. OBSERVED.
- `git grep -n "eqs_bind" -- crates/` returns `crates/arena/src/lib.rs:62` (`pub mod eqs_bind;`) and
  the module's own test names only — no external caller. **GA8** is ticked; this is a second
  unwired module and nobody owns it. OBSERVED, 2026-10-21.
- `crates/ai/src/depth/steer.rs:235` calls `flock_wish(mx, mz, &[])` — the flock is quiescent in
  production. OBSERVED.
- Nothing in `crates/` knows about capture points, objectives, territory, owner, contest count or
  score tick. `git grep -i "capture|objective|control_point" -- crates/` gives only
  `crates/world/src/roster.rs:41` (`Slot::Objective`, a loot slot) and unrelated hits. OBSERVED.
  This is why row 1 (Screeps) is the top find and why a war-game brain is a **new row**, not an
  existing one.
- `crates/ai/src/melee.rs:19` `NEAR_FOES = 3` — the melee view already gives each brain three
  nearest foes plus one chosen target, so the Overmind target score in row 2 has a home.

## Rejections

- `Sollimann/bonsai` (MIT) — already mined by GA11; our `bt.rs` is already ahead (serializable
  integer tree, not Rust structs). Correction, row 11.
- `zkat/big-brain` (Apache-2.0) — already mined (LADDER card); the composites now live in
  `crates/ai/src/utility.rs` and are called from `gladiator.rs`. Correction, row 10.
- `victorb/dogoap` (MIT) — already ported into `crates/ai/src/goap.rs` (the file cites it). The
  missing half is a consumer, not a donor.
- `BehaviorTree/BehaviorTree.CPP` (MIT) + `Crashkonijn/Goap` (Apache-2.0) — read 2026-10-20 by the
  wave-2026-10-20 lane; we own both a BT executor and a GOAP planner. Correction, row 12.
- `Farama-Foundation/PettingZoo` (MIT) — read 2026-10-20 by the wave-2026-10-20 lane; our
  `crates/arena/src/fork.rs` spawns its own branches. No gap.
- `google-deepmind/open_spiel` (Apache-2.0, `LICENSE` read live 2026-10-21) — a research framework
  for turn-based games; we have a fixed tick and a tape already, and our arena is not turn-based.
  No gap for a 6v6 arena.
- `AizenKai1001/lunaris` flock example — already ported into `crates/ai/src/depth/steer.rs:39`
  (the file says so). The missing half is the neighbour feed, not the flock math; that is why
  row 9 asks only for the neighbour set.
- `snape/RVO2` — already ported into `crates/nav/src/orca.rs:18` (the file says so). The missing
  half is a caller (TC4), not a donor.
- `screeps/screeps` (the server repo, not the engine) — its `LICENSE.txt` is the same permissive
  text as the engine's, but the server-side room/processor code is not in that repo; only
  `screeps/engine` carries the intent processors, which is what row 1 reads.
- `joonspk-research/generative_agents` — already cited by the wave-2026-10-20 workspace lane
  (memory stream with three retrieval scores). Not a game-NPC technique for our lane.
- `normal-computing/outlines` as a *code* donor — its Python sampler is not portable to our Rust
  provider; row 7 takes only the "grammar is the interface" idea, and the enforcement has to be
  built on our own `action_schema`.
- `Otoru/geppetto` as a *code* donor — Go, float math, `math/rand/v2`; row 6 takes only the
  scoring shape.
- `wesnoth/wesnoth` — GPL-2.0. Row 3 is RESEARCH-ONLY: no Lua is copied, only the table shape is
  re-derived. RISK LINE: copyleft, so no code may be vendored from it.
- `unitt`/`Unity`/`Unreal` — proprietary, research-only, not touched.

## Top 3

1. `screeps/engine` — `src/processor/intents/creeps/claimController.js`, `attackController.js`,
   `upgradeController.js`, `src/processor/intents/controllers/tick.js` — ISC-style permissive
   (`LICENSE.txt` read live 2026-10-21) — closes the **RY4** capture-point state machine and gives
   **RY5** an AI that can hold and fight for a point: an owner, a progress counter, a contest that
   drains a timer, a hold that feeds progress, and a decay that releases an unattended point. Cost
   **M**.
2. `bencbartlett/Overmind` — `src/creepSetups/setups.ts`, `src/priorities/priorities.ts`,
   `src/targeting/CombatTargeting.ts`, `src/overlords/defense/meleeDefense.ts` — MIT (read live
   2026-10-21) — closes the four-AI-seats-per-side half of **RY5** and the orders half of **GA9**:
   named role classes, a six-step priority ladder, a focus-fire target score, and a
   "how many defenders does this threat need" formula restated over four seats. Cost **M**.
3. `normal-computing/outlines` — `LICENSE` + `src/outlines/` (read live 2026-10-21) — Apache-2.0 —
   hardens **NS14** and **BX6**: the action grammar becomes the model-side constraint, not only the
   server-side validator, so a free local model can never emit an action the tick cannot parse.
   Cost **M**.
