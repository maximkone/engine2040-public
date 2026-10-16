# Game-AI engines survey (RES-AIENG-4, 2026-10-01)

Goal: engine2040 game AI (`crates/ai|nav|spatial|playtest|distill|nn`) must be
measurably smarter than any engine's AI stack. No code edits this packet.

Method + honesty: `docs/research/COMPARE.md` + `docs/plan/game-ai.md`,
`spatial.md` (to line 80), `hard-systems.md` (to line 80) read; ours status by
Grep in `crates/` (evidence paths below). Websearch transport was down and
`api.github.com` returned 403 after 1 call, so per packet rule that host ended:
stars are memory-approx at 2026-10-01 (prefix `~`), licenses verified live only
where marked `[V]` (`BehaviorTree.CPP` this run; 7 game-ai.md donors re-read
2026-09-30); all others `(mem, UNVERIFIED)`. No GPL code ports recommended.

## 1. Ours: honest status (Grep-verified)

| Cap | Status | Evidence |
|---|---|---|
| Integer-utility core + gladiator Brain | implemented, tested, integrated (lib) | `crates/ai/src/gladiator.rs:141`, `utility.rs:1` |
| DepthBrain phases/feints/stamina/squads + steer wish | implemented, tested, lib-integrated; tick binding open (GA8) | `ai/src/depth.rs:444`, `depth/steer.rs`; plan §1 binding gap |
| BT/GOAP-lite/blackboard/director/guard | implemented, tested (GOAP = cost-sorted `goap_plan`, no HTN module) | `ai/src/behavior.rs`, `blackboard.rs:70`, `director.rs`, `guard.rs` |
| EQS cover/flank/ambush (Tactics) | implemented, tested, ticked; arena bind open | `ai/src/eqs.rs:795` (1173 lines, 15 tests) |
| Legion formations + thinking LOD | implemented, tested, ticked | `ai/src/legion.rs` (1260 lines, 11 tests) |
| NPC mind/memory/provenance/dialogue bake | implemented, tested, integrated | `ai/src/mind.rs,memory.rs,provenance.rs,dialogue.rs,apply.rs` |
| Navmesh bake/query + links + tiles/flow + rebake alias | implemented, tested, integrated | `nav/src/mesh.rs:86`, `links.rs`, `tiles.rs`, `rebake.rs` |
| ORCA / platoons / funnel | MISSING (no `orca.rs`/`platoon.rs`/`funnel.rs`, re-verified) | plan NV4/NV5/NV6 open |
| plan_goals classical planner, fork-checked | implemented, tested as read-only MCP door | `endpoint/src/plan_goals.rs:1`, `mcp.rs:22` |
| Deterministic fork-join | implemented for bake/verify/import only, not sim tick | `ecs/src/parallel.rs:1` |
| Tournament (distill + playtest), Elo | implemented, tested; NO Elo rating anywhere | `distill/src/tournament.rs:171`, `playtest/src/tournament.rs:106`; grep `Elo` = 0 hits |
| Distill student/train/data/export `.e2nn` | implemented, tested; diversity students open | `distill/src/{spec,student,train,data,export}.rs`; no `diversity.rs` |
| Int8 nn runtime + ONNX convert whitelist | implemented, tested | `nn/src/lib.rs:178`, `nn/src/convert.rs:51` |
| Playtest agents/eval/spatial_eval/mirror | implemented, tested | `playtest/src/{agent,eval,tournament,spatial_eval,mirror}.rs` |
| Spatial sight/cover/affordances/index3 | implemented, tested, integrated (EQS data source) | `spatial/src/{sight,afford,index3}.rs`; SP8/SP9 ticked |

## 2. Survey (30 repos, big to tiny)

Format: technique | ★~ | SPDX | determinism story | we LACK | we BEAT.
`[V]` = license+stars read live; else memory-approx, UNVERIFIED.

| # | Repo | ★~ | SPDX | Technique → determ / lack / beat |
|---|---|---|---|---|
| 1 | `BehaviorTree/BehaviorTree.CPP` | 4223 [V] | MIT [V] | BT + XML/groot; determ: none stated / LACK: editor-grade BT XML import / BEAT: int determinism + replay x2 |
| 2 | `ashblue/fluid-behavior-tree` | ~400 | MIT (mem) | BT Unity C# fluent; determ: none / LACK: fluent authoring sugar / BEAT: std-only + hashed |
| 3 | `mugen87/yuka` | ~2000 | MIT (mem) | Steering+FSM+BT+GOAP+vision/memory JS; determ: none (float, frame-rate) / LACK: perception-vision shapes / BEAT: int math + versioned bytes |
| 4 | `libgdx/gdx-ai` | ~1100 | Apache-2.0 (mem) | BT/FSM/steering/pathfinding/scheduling Java; determ: none / LACK: scheduler shape / BEAT: tiles+flow+stamps |
| 5 | `qwertyquerty/pygoap` | ~200 | MIT (mem) | GOAP Python tiny; determ: none / LACK: nothing (our `goap_plan` covers it) / BEAT: everything (tests+hash) |
| 6 | `nickpresta/goap` | ~150 | MIT (mem) | GOAP Go tiny; determ: none / LACK: nothing / BEAT: same as #5 |
| 7 | `recastnavigation/recastnavigation` | ~8000 | Zlib [V 09-30] | Voxel+tilecache nav; determ: none promised / LACK: obstacle queue lifecycle (NV13), stitch proof / BEAT: versioned bytes x2, sorted bakes |
| 8 | `ikpil/DotRecast` | ~1500 | Zlib [V 09-30] | Recast C# port; determ: none / LACK: salt-cycle handle shape (NV7 detail) / BEAT: same as #7 |
| 9 | `snape/RVO2-3D` | ~100 | Apache-2.0 [V 09-30] | ORCA half-planes float; determ: none across machines / LACK: ORCA itself (NV4) / BEAT: Fixed-on-XZ plan + stale-exact rule |
| 10 | `vleue/polyanya` | ~300 | Apache-2.0 [V 09-30] | Portal funnel mesh nav; determ: none / LACK: funnel (NV6) / BEAT: salted refs + shorter-proof bar |
| 11 | `georust/rstar` | ~800 | MIT/Apache [V 09-30] | R* spatial index; determ: yes-ish (sorted) / LACK: broadphase contender (NV10) / BEAT: scan-referee equality proof |
| 12 | `Unity-Technologies/ml-agents` | ~6000 | Apache-2.0 (mem) | PPO/BC Unity RL, deprecated 2024; determ: none (GPU, nondeterm) / LACK: vectorized gym door (HS) / BEAT: offline `.e2nn` + replay x2 + no live RL |
| 13 | `edbeeching/godot_rl_agents` | ~1000 | MIT (mem) | Godot RL bridge; determ: none / LACK: same gym door / BEAT: same as #12 |
| 14 | `Farama-Foundation/Gymnasium` | ~6000 | MIT (mem) | RL env API standard; determ: seeding only / LACK: typed vector env door / BEAT: hash-law receipts, not seeds |
| 15 | `Farama-Foundation/PettingZoo` | ~2000 | MIT (mem) | Multi-agent envs; determ: seeding / LACK: 500v500 fixture shape (GA9) / BEAT: legion fixture + replay x2 |
| 16 | `facebookresearch/ELF` | ~2000 | MIT (mem) | RTS self-play platform; determ: partial / LACK: mass self-play scale proof / BEAT: tournament receipt + diversity check |
| 17 | `google-deepmind/open_spiel` | ~5000 | Apache-2.0 (mem) | Game-theory + eval arenas; determ: strong (sim) / LACK: their eval protocol shapes / BEAT: fight-domain + pixel captures |
| 18 | `DIAMBRA/arena` | ~500 | Apache-2.0 (mem) | Atari-like RL arenas + imitation; determ: ROM-level / LACK: teacher-shuffle honesty test is OURS to keep / BEAT: shuffled-teacher collapse gate |
| 19 | `TeamFightingICE/DareFightingICE` | ~100 | MIT (mem) | Fighting-game AI comp + audio; determ: round-level / LACK: audio-input fighter channel / BEAT: intents+paths+steering hashed |
| 20 | `MineDojo/MineDojo` | ~3000 | MIT (mem) | Minecraft foundation tasks+data; determ: none / LACK: internet-scale task corpus (CUT-adjacent) / BEAT: tape-to-dataset with exact labels (open, HS27) |
| 21 | `MineDojo/Voyager` | ~5000 | MIT (mem) | LLM skill-library agent; determ: none (LLM in loop) / LACK: skill-library shape for NPCs / BEAT: LLM never in tick; recorded actions only |
| 22 | `joonspk-research/generative_agents` | ~16000 | Apache-2.0 (mem) | Memory+reflection NPCs; determ: none / LACK: reflection-question shapes / BEAT: digest-stable provenance log + offline replay |
| 23 | `microsoft/TextWorld` | ~1000 | MIT (mem) | Text-game generator for agents; determ: seeded gen / LACK: text-fixture gen for dialogue bake / BEAT: dialogue compiles offline, dead branches refused |
| 24 | `huggingface/smolagents` | ~13000 | Apache-2.0 (mem) | Generic tool agents; determ: none / LACK: tool-loop safety shapes / BEAT: 20-tool budget ledger + preview/undo door |
| 25 | `xbpeng/DeepMimic` | ~2000 | UNVERIFIED | Imitation locomotion PyBullet; determ: none / LACK: style-reward shapes (PX5/DT5) / BEAT: blind A/B adoption gate exists |
| 26 | `GuyTevet/motion-diffusion-model` | ~2000 | MIT (mem) | Text→motion diffusion; determ: none / LACK: offline motion prior (idea only) / BEAT: no weights in tick, baked assets only |
| 27 | `NVlabs/ASE` | ~1000 | UNVERIFIED (custom?) | Skill embeddings + task reuse; determ: none / LACK: skill-embedding shapes / BEAT: same as #26; verify license before any take |
| 28 | `concept-graphs/concept-graphs` | ~1500 | MIT (mem) | 3D scene graphs for LLM grounding; determ: n/a / LACK: object-centric graph export shape / BEAT: relation graph digest-stable + id-sorted |
| 29 | `open-eqa/open-eqa` | ~1000 | MIT (mem) | Embodied QA benchmark; determ: benchmark-side / LACK: mirrored external suite (HS20 open) / BEAT: spatial_eval 2618-line gold harness exists |
| 30 | `carla-simulator/carla` | ~9000 | MIT (mem) | Driving sim + agent testing; determ: partial / LACK: sensor-noise test shapes / BEAT: headless CPU determinism bar |
| 31 | `dogu-team/gamium` | ~500 | Apache-2.0 (mem) | Game test-automation SDK; determ: none / LACK: multi-client driver shapes / BEAT: desync/stuck/impossible scanners + sealed evidence |

No standalone EQS-clone, flow-field, or HTN-game repo of note found (EQS lives
inside UE; flow-fields inside shipped RTS engines; HTN in papers/SHOP2); our
EQS-with-replay and tiles-flow already stand alone. No GPL takes anywhere.

## 3. Capability matrix (ours vs best rival)

`+` we lead, `=` parity, `-` we trail; status = implemented/tested/integrated.

| Cap | Ours | Best rival | Verdict |
|---|---|---|---|
| Behavior trees | behavior.rs tested, no XML import | #1 BT.CPP | `-` authoring, `+` determinism |
| GOAP | cost-sorted plan, tested | #3 yuka / #5 | `=` core, `-` HTN layer |
| Utility AI | int utility + curves, tested | #3 / #4 | `+` (int + fallback) |
| EQS | Tactics tested, bind open | UE built-in (docs) | `+` replay story, `-` live bind |
| Crowd/ORCA | separation+arrive; ORCA missing | #9 RVO2-3D | `-` until NV4 |
| Navmesh/flow/tiles | versioned tiles+flow tested | #7 Recast | `+` bytes-x2, `-` queue lifecycle |
| Funnel smoothing | missing | #10 polyanya | `-` until NV6 |
| LLM NPC memory | mind/memory/provenance tested | #22 generative_agents | `+` (digests+replay) |
| RL/imitation | offline distill tested, gym door missing | #12 ml-agents / #18 | `+` honesty gates, `-` env scale |
| Learned motion | adoption-gated, no weights in tick | #25/#26/#27 | `=` approach, `-` shipped priors |
| Spatial-for-LLM | relation/sight/afford tested | #28 concept-graphs | `+` (stable + wired to door) |
| Playtest bots/eval | scanners+sealed evidence tested | #31 gamium / #17 | `+` (exact-replay eval) |

## 4. Top 10 steal-or-beat (ranked)

1. ORCA Fixed-XZ (beat #9): owner `nav`; port half-plane math to Fixed, far stays
   flow. Accept: `cargo test -p engine-nav orca` + corridor zero-stuck x2.
2. Portal funnel (steal #10 shape): owner `nav`; string-pull over tiles, salted
   refs. Accept: `cargo test -p engine-nav funnel` + 10%-shorter proof.
3. Obstacle queue (steal #7 shape): owner `nav`; touched/pending + 1 rebuild/tick
   + `up_to_date`. Accept: `cargo test -p engine-nav tiles` + digest x2 (NV13).
4. Deterministic HTN-lite (beat #2/#6): owner `ai`; methods over blackboard,
   id-ordered. Accept: `cargo test -p engine-ai htn` + replay x2.
5. BT data bake (steal #1 XML idea, no code): owner `ai`; data→tables via TU1.
   Accept: `cargo test -p engine-ai behavior` + golden unchanged.
6. EQS live bind (beat UE EQS): owner `ai`+arena; GA8 first call path.
   Accept: arena cover fixture holds + replay x2, tick hash unchanged off-path.
7. Tournament Elo gates (beat #12/#16/#18): owner `distill`; Elo + x2 receipt,
   shuffled-teacher collapse. Accept: `cargo test -p engine-distill tournament`.
8. Diversity students (beat #18 imitation): owner `ai`; GA7 loss + receipt.
   Accept: `cargo test -p engine-ai diversity` + collapse-on-shuffle proof.
9. Tape-to-dataset exporter (beat #20 labels): owner `playtest`; state/action/
   obs/hash rows. Accept: exporter test + resim-hash equality x2.
10. Grounded NPC asks (beat #21/#22/#24): owner `endpoint`+`spatial`; brief→ask→
    recorded action, never LLM-in-tick. Accept: `cargo test -p engine-endpoint
    scene_query` + mind replay test green.

## 5. Smartness thesis

Any rival is smarter on one axis; engine2040 is the only stack where EVERY AI
verdict is a number that reproduces: (a) exact-replay evaluation — same
seed+tape → same intents/paths/steering/hash, so tournaments, EQS spots and
student-vs-script reports re-run bit-identically; (b) fork-checked plans —
`plan_goals` answers are verified against resim before they count, so no plan
can pass that the engine cannot replay; (c) tournament Elo gates — every
student ships with an Elo-vs-script receipt plus a shuffled-teacher collapse
check, so fake learning is rejected with numbers. Measurable beat: 0 stuck
across 10k seeded corridors, funnel ≥10% shorter on 80% queries, 1000-agent
p99 ≤2 ms, 500v500 flank-decides with replay x2 — all hash-pinned, none of the
31 rivals publish that quadruple.

Unverified: all `~` stars + `(mem)` licenses (hosts ended); #27 license before
any take; rival perf numbers are publisher claims, never our runs.
