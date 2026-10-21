# The hunt, 2026-10-21: five hunters, one evening, 100+ repos

Ask (owner): "research smart ideas or repos on github to steal for reuse to our
workspace, go wild, hunt hard, use 5 subs for gold ideas to steal or reland or
reuse".

Answer: **20 board rows, 60+ donors read, 5 new board rows that no GitHub
engine has a donor for, and 25 numbers from public-domain Roman sources that
the Rome tables can cite.** The most useful thing the hunt found is not a repo:
it is that the finish-line gap list is stale, and that most of the good donors
land on rows that already exist.

## The one screen

| # | the steal | donor | our row |
|---|---|---|---|
| 1 | a whole LLM forward pass **in WGSL**, on our own GPU, in our process | `Beledarian/wgpu-llm` (13 stars, Apache-2.0) | FOCUS Now 4, gate G7 |
| 2 | a real TAA + TAAU resolve (231 lines, WGSL, in a Rust engine) | `Siroko/kansei` (28 stars, MIT) | LF5c, LF5b, LF5, PW01 |
| 3 | the **worker is the authority**: the browser hosts the sim and the page pays credits | `dzhng/battlesim` (5 stars) | WEB5, WEB16, WEB17 |
| 4 | fight feel as **integer table rows** (per-bone muscle, joint limits, hop-spread) | `blugart-dev/kickback` (28 stars, MIT) | PX9, C10, AM24 |
| 5 | the **AI bill of materials**: every door edit names the agent, the model, the prompt hash and the tape hash | `Open-fab-ai/openfab` (6 stars, Apache-2.0) | HX3 (new) |
| 6 | the **codex numbers** with their chapter refs: the camp, the ditch, the streets, the drill | Vegetius I.21-27, Pseudo-Hyginus 49-52 (public domain) | ROM1, ROM2, LEG3, LEG4, ROM4 |
| 7 | the **real Roman map**: roads, walls, aqueducts, the empire under Hadrian, 7,891 settlements | `AWMC/geodata` (ODbL), `Diasito/urbesetorbis` (BSD-2) | ROM2, LEG6, ROM5 |
| 8 | a WGSL **GPU radix sort** plus a 3D Gaussian-splat renderer in Rust and WebGPU | `KeKsBoTer/web-splat` (307 stars, Apache-2.0) | HX1 (new), WEB17 |
| 9 | the **WebGPU conformance suite** and a native runner with per-case isolation | `gpuweb/cts` (180 stars, BSD-3) | HX4 (new) |
| 10 | a **two-tier hash**: the sim moved, or the solver moved | `mrdav30/Gravitas` (MIT) | HS2, C15 |
| 11 | a **soft float**: f32's range with f32's exactness, in integers | `devlinzhou/deterministic_physics` (42 stars, Apache-2.0) | C15 |
| 12 | a **seqlock frame bus** and a seekable tick: the browser time machine | `nithish101/resonance` (MIT) | HX15, HX17 (new) |

## What the hunt changed about our picture

1. **The gap list is stale, and the hunters proved it by reading the tree.**
   A3 (browser net), A4 (the strict flag), A5 (AudioWorklet) and A6 (the tier
   ladder and the pipeline warm-up) are **built**: `crates/net/src/browser/*`,
   `crates/render/src/backend.rs:461`, `crates/web/src/audio_worklet.rs`,
   `crates/render/src/pipeline_warmup.rs`. A2 is built natively
   (`crates/assets/src/load_queue.rs`) and only the web half is missing. The
   live gaps in that lane are A1 (the cooked bodies and the exchange cues),
   WEB5 (the frame pair), ON30 (the two-machine internet run) and E23 (the
   netcode model decision).
2. **Most steals do not need a new row.** The donor map
   (`10-donor-map.md`) is donor -> existing open row. 40+ of the donors land
   on rows that are already on the board with a name, a plan and an Accept.
3. **The hunt found real numbers for the Rome game.** 25 figures with their
   chapter and the Latin behind them, including the two ditch figures that
   disagree (Pseudo-Hyginus 49 gives the marching camp, 5 x 3 ft; Vegetius 1.21
   gives the fortified camp, 9 x 7 ft hasty and 12 x 9 ft regular). A table that
   hides the disagreement is a bug, so the table keeps both with a `threat` key.
4. **The wildest idea costs the least.** One blue-noise texture shared by the
   TAA jitter, the GTAO rotation, the shadow PCF, the decal dither, the post
   dither and the metric window: one asset, one hash, one golden, and no GitHub
   engine ships a single shared blue-noise blueprint across all of them.

## The files in this folder

| file | what it is |
|---|---|
| `00-README.md` | this page |
| `06-outside-hunt.md` | hunter 0: the donors from **outside** the engine set, and the public-domain data |
| `07-codex-vegetius-hyginus.md` | the 25 codex numbers with the Latin and the chapter refs |
| `10-donor-map.md` | **the actionable table**: donor -> the open row it serves |
| `agent-1-render.md` | hunter 1: the lit frame and the look (12 ranked, 10 wild, 10 rejected) |
| `agent-2-web-net.md` | hunter 2: the browser and the network (12 ranked, 10 wild, 20 rejected) |
| `agent-3-physics-body.md` | hunter 3: bodies, physics, destruction, motion (12 ranked, 9 wild, 12 rejected) |
| `agent-4-ai-door.md` | hunter 4: the AI door, the memory, the evals |
| `agent-5-rome-world.md` | hunter 5: the Rome game, the world, the sim at scale |
| `ghs.mjs`, `ghf.mjs` | the hunt tools: repo search, repo tree, one file |
| `donors-seen.txt` | every owner/repo the earlier waves already mined, so nobody re-mines one |

The board rows live in `docs/SPRINT-29-hunt.md` (HX1-HX22).

## How to use the tools

```
node docs/research/hunt-2026-10-21/ghs.mjs "<query>" 25 stars
node docs/research/hunt-2026-10-21/ghf.mjs <owner/repo> meta
node docs/research/hunt-2026-10-21/ghf.mjs <owner/repo> tree "<regex>"
node docs/research/hunt-2026-10-21/ghf.mjs <owner/repo> file <path>
```

They use the authenticated `gh` CLI, so the limits are the account's, not the
anonymous 10-per-minute. The sharpest trick one hunter used: search code by
`filename:*.wgsl`, which finds WGSL files nobody writes about.
