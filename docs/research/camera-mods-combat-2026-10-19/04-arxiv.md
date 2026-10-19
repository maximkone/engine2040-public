# arXiv and papers — 2026-10-19

Ids read live 2026-10-19 (abstract, and one section where the method mattered).
Dedupe by id. Titles are as arXiv printed them.

## Camera and cinematography

| Id | Title | What we take |
|---|---|---|
| 2303.17041 | Automatic Camera Trajectory Control with Enhanced Immersion for Virtual Cinematography | Three named axes: frame aesthetics (rule of thirds), spatial action tracking, emotional status. A rule-of-thirds *adjustor* runs before the learned motion. Steal the shape: a composition fix pass, then a motion pass |
| 2506.00988 | LensCraft: Your Professional Virtual Cinematographer | Volume-aware (a subject is not a point), multi-modal conditioning (text, trajectory, volume, keyframes), real-time, **code + data + weights public**. The interface list is the steal: a shot request carries a subject with a volume |
| 2409.17331 | ChatCam: Empowering Camera Control through Conversational AI | An LLM agent + CineGPT (text -> trajectory) + an **Anchor Determinator** that decides which object the shot is about. Steal the anchor step: a shot verb must name its subject, then solve |
| 2402.16143 | Cinematographic Camera Diffusion Model | text -> camera motion with keyframe constraints and latent interpolation between motions; code at github.com/jianghd1996/Camera-control. Steal: keyframes as *constraints*, not inpainting |
| 2208.05213 | Automatic Camera Control and Directing with an Ultra-High-Definition Collaborative Recording System | Auto-director over several wide-angle streams. Steal: a director that shoots wide and **crops/directs afterward** — matches our hotsampling + replay path |
| 10.1145/3386569.3392427 | Example-driven virtual cinematography by learning camera behaviors (ACM TOG 2020) | The **low-dim cinematic feature space** (angle, distance, screen composition, character configuration) + a Mixture-of-Experts gate over learned behaviours. This is the best structure we found; steal the feature list as the camera-tuning schema |
| 10.1145/3478513.3480533 | Camera keyframing with style and control (ACM TOG 2021) | A style code + user keyframes as hard constraints. Steal: style is a small discrete id, keyframes are the guarantees |

## Impact feel (why the little camera things matter)

| Id | Title | What we take |
|---|---|---|
| 2208.06155 | What Features Influence Impact Feel? A Study of Impact Feedback in Action Games | A 19-feature framework. The three that decide impact feel: **hit stop, sound coherence, camera control**. Camera shake should carry a *semantic direction* (vertical on a knockdown, horizontal on a cross slash). Audio and camera must agree, or the impact reads as wrong. We have hit stop, poise, cues and trauma: the missing piece is agreement |

## PCG

| Id | Title | What we take |
|---|---|---|
| 2503.21474 | The Procedural Content Generation Benchmark: An Open-source Testbed | 12 problems, each with a representation, control parameters and **quality / diversity / controllability** metrics. Adopt as a proof surface for `crates/world`, the way the combat fixture proofs `combat` |
| 2410.15644 | PCG in Games: A Survey with Insights on Emerging LLM Integration | A taxonomy (bits / space / scenarios / design / derived) and the finding that ~half of recent PCG papers are 2D level generation. Use it to scope: our gap is 3D and space |
| 2510.04862 | Video Game Level Design as a Multi-Agent Reinforcement Learning Problem | Many small local agents beat one global agent, and generalise better to new map shapes. Steal the shape for a room-level generator: per-room local passes |
| 2503.12358 | IPCGRL: Language-Instructed RL for Procedural Level Generation | Natural-language instruction as the control signal (+21.4% controllability). Steal the goal, not the net: an AI-facing text instruction for a bake |
| 2508.09193 | MIPCGRL: Multi-Objective Instruction-Aware Representation Learning in PCGRL | Compositional instructions: split "which objective" from "how strongly". Steal for a recipe schema (objectives + weights) |
| 2502.10906 | PCGRLLM: LLM-Driven Reward Design for PCG RL | An LLM writes the reward function from a story. Steal as a *design-time* tool: an AI names the quality bar, we implement it as a check |
| 2510.15120 | Procedural Game Level Design with Deep Reinforcement Learning | Two agents: a generator and a solver (a hummingbird that must reach the content). Steal the **solver-in-the-loop check**: generated content is valid if an agent can play it. We have `playtest` verifiers already |
| 2503.18748 | Simulation-Driven Balancing of Competitive Game Levels with RL | Balancing as PCG: a swap-based representation, rewards for equal win rates, then read *which tiles affect balance*. Steal for our arena map: A/B win rate from the `bench` crate |
| 2505.16801 | A modular framework for automated evaluation of PCG in serious games with DRL agents | Uses RL agents as **automated testers**, and reports that PCG versions changed agent win rates and training time. Supports the "playtest as content compiler" pillar |
| 10.1109/tg.2025.3530419 | PCG for Cooperative Games — A Systematic Review | Finding: PCG for co-op must account for co-op design patterns (tasks, incentivised cooperation). Relevant when a 2v2 mode exists |

## Combat, duels and fighting-game AI

| Id | Title | What we take |
|---|---|---|
| 2507.00261 | VirtualFencer: Generating Fencing Bouts based on Strategies Extracted from In-the-Wild Videos | Unsupervised extraction of 3D fencing motion and strategy from video, then self-play, then against a real fencer. Steal the **strategy layer** idea: a small tactic model over high-level actions (step, lunge, parry) |
| 10.1145/3313831.3376714 | Touché: Data-Driven Interactive Sword Fighting in VR (CHI 2020) | The two-level split: a physical level (data-driven motion) and a **semantic level** where designers configure behaviour from parameterised blocks. This is our combat architecture: frame data (physical) + rules (semantic) |
| 10.1109/ictai.2016.0052 | A Virtual Character Learns to Defend Himself in Sword Fighting Based on Q-Network | A per-frame Q-network that blocks incoming attacks, with a body-dynamics model for the irrelevant joints. Steal: the defense policy is a small per-frame classifier over the incoming swing — a tractable NPC block |
| 2504.07425 | Enhancing Player Enjoyment with a Two-Tier DRL and LLM-Based Agent System for Fighting Games | Tier 1: an archive of agents with **distinct styles** (defensive, special-move, newbie) via modular rewards. Tier 2: an LLM picks the opponent from player data + feedback. Steal both: a style archive is just parameter sets; the LLM picker is one endpoint verb |
| 2406.01103 | Advancing DRL Agents in Commercial Fighting Games (Shūkai, Naruto Mobile) | One model for a whole roster (trained on 13% of characters), heterogeneous league training, and rewards aligned to human expectations. Steal: **one NPC policy generalises over the roster** if the state is normalised |
| 2510.14154 | Combining RL and Behavior Trees for NPCs with AMD Schola | RL inside behaviour trees in a real engine. Steal the practical shape: BT picks the situation, a small policy plays it; do not replace the tree |
| 2407.00187 | (sports RL benchmark; exact title UNVERIFIED — PDF read only) | Sports tasks for RL including a ball/puck target task. Low confidence; listed so it is not re-found |

## Animation (the deep-end option, for later)

| Id | Title | What we take |
|---|---|---|
| 10.1145/3386569.3392440 | Learned Motion Matching (ACM TOG 2020) | Motion matching + a learned, memory-free alternative. Relevant only if clips stop being enough; it is data-heavy and float |
| 2310.05215 | Motion Matching for Character Animation and VR Avatars in Unity | A practical implementation write-up (features, database, control) |
| 10.1145/3658209 | Categorical Codebook Matching for Embodied Character Controllers | Sparse sensors -> full body, one manifold learned end-to-end |
| 2407.18946 | WalkTheDog: Cross-Morphology Motion Alignment via Phase Manifolds | A shared 1D **phase manifold** across characters: the *phase variable* is the stealable idea (a cycle position as an index, cheap and discrete) |

## Verdict

Nothing here is spiked, so nothing is ADOPT or REJECT yet. The two papers worth a
spike first are the PCG Benchmark (2503.21474, as a `world` testbed) and
"Impact Feel" (2208.06155, because it names the camera-meaning rule we are
missing). Both are cheap: one is a test surface, one is a tuning rule.
