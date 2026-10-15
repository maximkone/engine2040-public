# AI-native / LLM-driven game engines: survey vs engine2040

Date read: 2026-10-01. Method: `node tools/gh-search.mjs` discovery +
`gh repo view --json stargazerCount,licenseInfo,description` (stars/license
as of 2026-10-01). No code cloned, no test run here; behavior rows are
publisher claims (docs/description), not verified runs. Ideas are free to
use; no GPL code ports (none of the repos below is GPL; see license col).
Already covered by `docs/AI-NATIVE.md` + `docs/research/COMPARE.md` and NOT
repeated here: Unity-MCP 268-tool sprawl → MCP2 budget, OpenGameAgent
traces → EV1, Orak shape → EV1, nAIVE/llmgine hashes → tier-3 rule,
mcp-for-blender/Coding-Solo + tugcantopaloglu Godot-MCP counts → HS20.
Paper-only (no verified public repo this run): Project Sid/Altera,
GameCraft-Bench (2606.17861), PCG Benchmark (2503.21474), MS CraftBench,
Genie 3, Matrix-Game 3.0 — cited in AI-NATIVE.md, ideas reusable.

## Survey (34 repos; ★ = stargazers 2026-10-01; SPDX via gh API)

| # | Repo | ★ | License | AI actually does | Determinism / replay | Provider stance | Weakness |
|---|---|---|---|---|---|---|---|
| 1 | Unity-Technologies/ml-agents | 19716 | Other (custom) | Trains NPCs w/ RL + imitation; game = training env | Seeded training, no gameplay-tick hash proof | Framework-agnostic trainers (PyTorch/ONNX), Unity-locked | Engine-locked; no LLM/NPC-dialogue story; archived 2024 |
| 2 | edbeeching/godot_rl_agents | 1593 | MIT | Same pattern for Godot NPCs | Same as above, smaller harness | Trainer-agnostic, Godot-locked | No determinism proof; no LLM door at all |
| 3 | joonspk-research/generative_agents | 22177 | Apache-2.0 | Smallville NPCs: memory stream + reflection + planning dialogue | None — LLM calls non-replayable by design | Hardcoded OpenAI GPT-3.5 in original code | No engine, no physics, no replay, no budget; demo-only |
| 4 | a16z-infra/ai-town | 10575 | MIT | Deployable Smallville clone (chat/socialize NPCs) | None | Hardcoded OpenAI default; Convex backend | 2D tile demo; NPC brains die with the provider bill |
| 5 | nmatter1/smallville | 825 | MIT | Generative Agents ported into a game loop | None | Hardcoded OpenAI | Toy scope; unmaintained since 2023 |
| 6 | mluogh/eastworld | 227 | Apache-2.0 | Framework for generative agents in games | None | OpenAI-oriented | Thin wrapper, tiny community |
| 7 | HumanoidAgents/HumanoidAgents | 324 | Apache-2.0 | Human-like generative agents platform | None claimed | LLM-backed, vendor-mixed | Platform not engine; no tick, no world sim |
| 8 | google-deepmind/concordia | 1750 | Apache-2.0 | Generative social simulation library (GABM) | Seeded RNG for sims, LLM text still stochastic | Model-agnostic (Gemini default, others pluggable) | Library not engine; no 3D, no gameplay, no replay hash |
| 9 | camel-ai/oasis | 5215 | Apache-2.0 | 1M-agent social sim (Twitter-like), LLM actions | DB-backed state, no tick determinism | Multi-model via OpenAI-compatible API | Social sim only; name-collides with Decart Oasis; no game |
| 10 | mindcraft-bots/mindcraft | 5821 | MIT | LLM plays Minecraft via Mineflayer bot API | None — live LLM per action | Multi-provider (OpenAI/Claude/Gemini/local) = best in class | No determinism; bot API, not engine; cost per action |
| 11 | MineDojo/Voyager | 7235 | MIT | GPT-4 lifelong agent: curricula + auto-verified skill library | Skill code reuse, no world replay | Hardcoded GPT-4 + Mineflayer | Research script; dies without GPT-4; no engine ownership |
| 12 | MineDojo/MineDojo | 2262 | MIT | Internet-knowledge embodied-agent benchmark env | Seeded tasks, no exact replay | Backbone-agnostic | Env only; Minecraft-locked; heavyweight |
| 13 | etched-ai/open-oasis | 2136 | MIT | Inference script for Decart Oasis 500M playable world model | None — diffusion world model, stochastic frames | Weights-local, provider-free at runtime | No authority, no multiplayer, no determinism; GPU-hungry |
| 14 | arnaudstiegler/gameNgen-repro | 59 | Apache-2.0 | Open repro of GameNGen diffusion DOOM sim (+5 siblings) | None | Local weights | Repro-grade; no game logic, no determinism, no tooling |
| 15 | nomitchell/pytorch-genie-world-model | 4 | UNVERIFIED (no LICENSE) | Genie CoinRun case-study reimplementation | None | Local | 4★ stub; idea-only donor |
| 16 | krafton-ai/Orak | 167 | MIT | MCP-based game-agent benchmark harness | Fixed tasks, agent-side scoring | MCP-native, model-agnostic | Harness only; no engine, no world, tiny adoption |
| 17 | GameDevBench (+gamedevbench-mcp) | ~50 org-wide | UNVERIFIED (site repo empty) | Benchmark: can agents build playable games in real engines | Task validators, no engine replay | Model-agnostic by design | Benchmark only; new (2026); thin validators |
| 18 | EricSun0218/OpenGameAgent | 51 | MIT | TS runtime: durable NPC state, reliable actions, memory, tools + Unity/Godot/Unreal integrations | Durable state + append-only traces, no tick hash | Multi-engine, provider-pluggable | 51★ new; no engine of its own; no determinism proof |
| 19 | brandlll-lee/SolersEngine | 37 | MIT | Godot fork: AI agent builds/playtests/iterates inside editor | Inherits Godot (no extra proof) | Agent-side model choice | Godot-locked; 37★; agent edits unguarded by transactions |
| 20 | 7etsuo/anvil | 34 | UNVERIFIED | Agent-native multi-genre engine for coding agents | None stated | Agent-model-agnostic | 34★ new; no determinism, no replay, no proof |
| 21 | tettethu/VibeGame | 258 | Apache-2.0 | NL → playable 2D web game, self-evolving multi-agent, Claude Code/Codex built | None | Hardcoded Claude Code + Codex | 2D web only; vendor-locked; no determinism |
| 22 | tfoutrein/toile-engine | 3 | UNVERIFIED | 2D AI-native engine in Rust | None stated | Unknown | 3★; 2D-only; closest stack, farthest along? No — weakest proof |
| 23 | mathiscode/aige | 14 | MIT | "AI Game Engine" 2D experiments | None | Unknown | 14★ hobby scope |
| 24 | sumitaich1998/ai-game-engine | 2 | MIT | Browser GTA-like: prompts → live world mutations, NPC brains | None | Hardcoded provider (browser key) | Prompt-mutates-world with no guards = anti-pattern demo |
| 25 | SnyderConsulting/AI-Game-Engine | 0 | UNVERIFIED | 2D sandbox structured for coding agents | None | Agent-agnostic docs | Scaffolding only, no AI runtime |
| 26 | perlinson/ai-game-engine | 0 | UNVERIFIED | "AI-friendly" Three.js engine for agents | None | Unknown | 0★ new; README-level |
| 27 | CoplayDev/unity-mcp | 14626 | MIT | Biggest Unity↔LLM bridge: assets/scenes/scripts/play | None — direct editor mutation | Any MCP client model | No preview/undo discipline; tool sprawl; Unity-locked |
| 28 | IvanMurzak/Unity-MCP | 4369 | Apache-2.0 | Unity AI dev loop + CLI; 1-line C#→tool; token-efficient | None | Any model; local-remote parity claim | Still direct mutation; no transactional gate |
| 29 | AnkleBreaker-Studio/unity-mcp-server | 487 | Other (custom) | 268-tool Unity surface (most tools wins marketing) | None | Any MCP client | Sprawl = the MCP2 anti-pattern; custom license |
| 30 | Coding-Solo/godot-mcp | 5895 | MIT | Godot editor launch/run/debug bridge (14 tools) | None | Any MCP client | Thin; no scene write path discipline |
| 31 | tugcantopaloglu/godot-mcp | 472 | MIT | 157-tool full Godot control | None | Any MCP client | 157 verbs vs our 20-tool budget = sprawl example |
| 32 | satelliteoflove/godot-mcp | 171 | MIT | Godot scene edit + input injection + deterministic playtest + live state | Playtest determinism claimed (unverified here) | Any MCP client | Small; claims exceed evidence; Godot-locked |
| 33 | chongdashu/unreal-mcp | 2088 | UNVERIFIED | NL control of Unreal via MCP | None | Any MCP client | No license file; direct editor mutation; UE-locked |
| 34 | casting-dot-systems/llmgine | 7 | MIT | Lean tool-augmented LLM chat engine + MCP-server extension | Headless runs + state hashes claimed (see AI-NATIVE.md) | Tool-manager abstraction, pluggable | Chat engine, not game engine; 7★ |

## Where engine2040 is already ahead

1. **Deterministic tick + hash law.** Nobody above hashes gameplay; RL libs seed
   training, world models brag about stochasticity. HASH-LAW + tier-3 is ours alone.
2. **Transactional MCP door (20 tools, preview/apply/undo, receipts).** Bridges
   (#27–33) mutate editors directly; sprawl leaders (#29: 268, #31: 157) prove our
   MCP2 budget right. No rival has revision-checked idempotent apply.
3. **Provider-agnosticism as conformance.** Only #10 mindcraft is multi-provider,
   and it is a bot script. Our adapter + recorded-action replay story has no equal.
4. **Forks + causal replay.** Counterfactual fork metrics and do-operation replay
   (FK1/CR1) appear nowhere in the survey — Voyager reuses skill *code*, never
   re-simulates the *world*.
5. **Integer neural policy in the tick (.e2nn, NN1/BR2).** RL crowd trains floats
   offline; nobody ships bit-exact onboard nets with scripted fallback.
6. **Distillation teacher→student (DT1–3).** Voyager's skill library is code
   accumulation; ours is weight distillation with fork-tournament acceptance.
7. **Provenance chain (PR1).** Nobody tracks which AI made which object with what
   verifier result.
8. **Weak-model floor + discovery budget (AX10/MCP2).** Measured 0.0pp completion
   diff; rivals never test small models.

## Where engine2040 is behind

1. **No public agent benchmark shape.** Orak (#16) and GameDevBench (#17) let
   *outside researchers* score agents; our EV1 is internal-only. Visibility loss.
2. **No NPC memory/dialogue system.** Smallville/concordia (#3–8) ship memory
   stream + reflection + planning today; our announcer/taunt lines are parked
   outside the tick with scripted fallback.
3. **No code-generating content loop that outsiders can touch.** VibeGame (#21)
   turns NL into a playable game in minutes; our VF1 verifiers gate content that
   barely exists yet. Demo-velocity gap.
4. **No RL training env story.** ml-agents (#1, 19.7k★) and godot_rl_agents (#2)
   own the "train your NPCs" search traffic; our .e2nn pipeline has no trainer UX.
5. **No editor agent for meshes/scenes.** SolersEngine (#19) and Unity-MCP (#28)
   iterate inside real editors with asset pipelines; our CT10 import story is open.
6. **No skill/package portability demo.** OpenGameAgent (#18) ships portable skill
   packages across Unity/Godot/Unreal; our data-mods cover it on paper only.
7. **No million-agent or auto-curriculum story.** #9 (1M social agents) and #11
   (auto-curriculum) set scale narratives we cannot touch at arena scope — and
   should answer cheaply (budgets + forks) rather than match.
8. **Star-count reality.** The whole survey's median AI-native engine is <100★;
   category traffic goes to bridges (#27: 14.6k) and 2023 LLM demos (#3: 22k).
   A benchmark shape + playable demo is how this is beaten, not features.

## Ranked top-10 steal-or-beat ideas (owner crate/plan + task + accept)

1. **Orak-shaped public eval vector.** Owner `engine-playtest` (EV1). Task: publish
   fixed agent tasks over MCP with model-agnostic scoring in harness output.
   Accept: `cargo test -p engine-playtest --lib` green + outside client scores.
2. **GameDevBench-style task validators for generated content.** Owner
   `engine-proof` (VF1). Task: each generator task ships a validator (reachable
   spawns/gates, budget caps) that must pass before landing. Accept:
   `cargo test -p engine-proof --lib` green + one validator rejecting a bad map.
3. **Voyager skill-library → data-mod skill packs.** Owner `engine-distill` (DT1).
   Task: teacher play recorded as observation→action→outcome packs loadable as
   data mods, versioned like rules. Accept: `cargo test -p engine-distill --lib`
   green + pack replay without any provider.
4. **Memory-stream NPC lines outside the tick.** Owner `engine-ai` (BR1). Task:
   steal Smallville memory+reflection for announcer/taunts with scripted fallback,
   budgeted and cancellable, never in fixed tick. Accept:
   `cargo test -p engine-ai --lib` green + fallback fires with provider down.
5. **One-line-tool authoring (steal IvanMurzak #28).** Owner `engine-endpoint`
   (MCP2). Task: one macro attribute exposes a bounded, budgeted tool; sprawl
   over 20 tools fails the discovery-budget test. Accept:
   `cargo test -p engine-endpoint --lib` green + budget ledger unchanged.
6. **Deterministic-playtest claim with teeth (steal #32, beat it).**
   Owner `engine-playtest` (HS20). Task: input injection + frame-verify capture
   pairs through `play.run/step` with hash x2. Accept:
   `cargo test -p engine-playtest --lib` green, identical hash across two runs.
7. **Causal-replay-as-API (beat: nobody has it).** Owner `engine-core` (CR1).
   Task: remove/perturb-one-input resim names causing input + first divergent
   tick on a seeded bug tape. Accept: `cargo test -p engine-core --lib` green
   naming the planted input.
8. **.e2nn trainer UX (steal ml-agents #1 shape, beat on determinism).**
   Owner `engine-nn` (NN1) + `engine-distill` (DT2). Task: scripted-expert →
   dataset → student export with int/float agreement gate. Accept:
   `cargo test -p engine-nn --lib` green + bit-exact replay Windows/Linux.
9. **AI-provenance receipts queryable (beat: nobody has it).** Owner
   `engine-provider` + `engine-endpoint` (PR1). Task: every applied batch records
   source/provider/model/request-digest/verdicts; one query lists AI-made objects.
   Accept: `cargo test -p engine-provider --lib` green + chain complete offline.
10. **Concordia-style seeded social sim as fork workload (cheap answer to #8/#9).**
    Owner `engine-world` + `engine-playtest` (FK1). Task: 100-agent dialogue-free
    intent sim runs as a fork metric workload inside tick budgets. Accept:
    `cargo test -p engine-world --lib` green + identical metrics twice + budget held.

## Limits

Stars/licenses via gh API 2026-10-01; behavior cells are publisher claims, not
runs. No LADDER line (packet-scoped survey; ideas map to existing board IDs —
verify with `node tools/board-slice.mjs <ID>` before dispatch). No GPL donors;
UNVERIFIED rows are idea-only, never ports.
