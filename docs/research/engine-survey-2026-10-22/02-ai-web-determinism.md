# Engine survey part 2: AI-native, browser and deterministic engines (2026-10-22)

Question: what should engine2040 steal for the finish line (a visually good AI-native
engine that runs the same game on Windows and in a browser, AI-native in the core,
hard systems one call)?

Method: GitHub REST API read on 2026-10-22. Every star count, license and last push
below comes from `https://api.github.com/repos/<owner>/<repo>` on that day. Sixteen
narrow searches were run, one per query, no repeats:
`topic:mcp+game+engine`, `agentic+game+engine`, `webgpu+game+engine`,
`wasm+game+engine+language:rust`, `deterministic+lockstep+game+engine`,
`rollback+netcode+game+engine`, `replay+deterministic+simulation+engine`,
`AI-native+game+engine`, `agent-native+game+engine`, `MCP+game+engine+rust`,
`ggrs+rollback+rust`, `webgpu+rust+renderer+engine`,
`deterministic+fixed+point+simulation+rust`, `headless+game+engine+AI+agent+build`,
`topic:webgpu+topic:game-engine`, `topic:deterministic+topic:game-engine`,
`topic:ai-native`, `topic:lockstep`, `topic:replay+topic:game`,
`topic:webgpu+language:rust`, `topic:game-engine+topic:ai`,
`topic:deterministic-simulation`, `topic:wasm-game`.

For the twelve deepest candidates the default branch was read live: the sha came from
`https://api.github.com/repos/<owner>/<repo>/commits?per_page=1` and the file body from
`https://raw.githubusercontent.com/<owner>/<repo>/<sha>/<path>`. The exact path read is
named in the steal table. No donor was built or run.

Already researched earlier and NOT repeated: `docs/research/finish-line-2026-10-17/02-github.md`
(its 25 steals stay valid), `docs/research/finish-line-2026-10-22/01-engines.md` (Bevy,
Godot, Fyrox, O3DE, Stride, Flax, Wicked, Defold, macroquad, Panda3D, Castle, Babylon.js,
PlayCanvas, quench3d, Open-Godot-MCP), `docs/research/steals-2026-10-11/ai-native.md`,
`docs/research/steals-2026-10-18/*`, `docs/research/web-2026-10-17/*`,
`docs/research/hunt-2026-10-21/*`, `docs/research/powerups-2026-10-21/*`.
Repos already named in those files are marked "known" in the table and are not ranked again.

## 1. The catalogue (80 repos, 2026-10-22)

Class A: AI-native / agentic engines and MCP game servers (40 rows).
Class B: browser and WebGPU engines (Rust or JS, wasm, WebGPU renderers) (20 rows).
Class C: deterministic / replay-first engines (20 rows).

All 80 rows are distinct repos. Some sit in two classes on purpose (a deterministic
engine that also runs in a browser). The count is 80 unique names, checked with a
regex over the table on 2026-10-22.

### Class A: AI-native and agentic engines, MCP game servers

| # | Repo | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| A1 | [Coding-Solo/godot-mcp](https://github.com/Coding-Solo/godot-mcp) | 6261 | MIT | 2026-04-16 | MCP server for Godot: launches the editor, runs projects |
| A2 | [gamedev-skills/awesome-gamedev-agent-skills](https://github.com/gamedev-skills/awesome-gamedev-agent-skills) | 2553 | Apache-2.0 | 2026-10-09 | 74 game-dev skills for AI coding agents across many engines |
| A3 | [thedivergentai/Game-Dev-Agentic-Skills](https://github.com/thedivergentai/Game-Dev-Agentic-Skills) | 878 | LGPL-3.0 | 2026-10-16 | Long-term memory for game AI agents, 99 Godot skills |
| A4 | [tugcantopaloglu/godot-mcp](https://github.com/tugcantopaloglu/godot-mcp) | 493 | MIT | 2026-10-09 | 157 tools for AI-driven Godot 4 development |
| A5 | [ukanwat/overtime](https://github.com/ukanwat/overtime) | 399 | MIT | 2026-10-18 | Give a coding agent a brief, not a chat; works across sessions |
| A6 | [wellingfeng/UltraGameStudio](https://github.com/wellingfeng/UltraGameStudio) | 326 | MIT | 2026-10-11 | AI coding agent for game development and assets |
| A7 | [tettethu/VibeGame](https://github.com/tettethu/VibeGame) | 278 | Apache-2.0 | 2026-10-13 | Self-evolving multi-agent framework for game making |
| A8 | [0x0funky/agent-game-forge](https://github.com/0x0funky/agent-game-forge) | 213 | Apache-2.0 | 2026-05-20 | Local-first 2D game IDE, bring-your-own-agent, multi-engine |
| A9 | [FreedomIntelligence/gamecraft-bench](https://github.com/FreedomIntelligence/gamecraft-bench) | 213 | Apache-2.0 | 2026-08-13 | Benchmark: can agents build playable games end to end |
| A10 | [SummerEngine/summer](https://github.com/SummerEngine/summer) | 86 | MIT | 2026-10-20 | "The game engine built for AI": MCP server, CLI, skills (known: LADDER card) |
| A11 | [KyosukeIshizu1008/berryscode](https://github.com/KyosukeIshizu1008/berryscode) | 151 | MIT | 2026-05-20 | Native IDE for Bevy built in Bevy, with an AI agent |
| A12 | [striderZA/OpenCodeGameStudios](https://github.com/striderZA/OpenCodeGameStudios) | 100 | MIT | 2026-10-06 | Agentic game dev: plan, build and test across engines |
| A13 | [LudicDynamics/WorldLines](https://github.com/LudicDynamics/WorldLines) | 98 | NOASSERTION | 2026-10-17 | Orchestrator-agents engine for living role-play worlds |
| A14 | [HurtzDonutStudios/ai-forge-mcp](https://github.com/HurtzDonutStudios/ai-forge-mcp) | 98 | none | 2026-04-09 | 565 AI-callable tools over 16 MCP servers for asset production |
| A15 | [KilledByAPixel/LittleJS-AI](https://github.com/KilledByAPixel/LittleJS-AI) | 84 | MIT | 2026-10-14 | AI-assisted toolkit for HTML5 games with LittleJS |
| A16 | [EricSun0218/OpenGameAgent](https://github.com/EricSun0218/OpenGameAgent) | 56 | MIT | 2026-10-21 | TS runtime for AI-native game characters, durable state |
| A17 | [IvanMurzak/Unreal-MCP](https://github.com/IvanMurzak/Unreal-MCP) | 51 | Apache-2.0 | 2026-10-22 | Unreal MCP plugin, C++ editor plugin plus .NET bridge |
| A18 | [praydog/re-engine-mcp](https://github.com/praydog/re-engine-mcp) | 41 | MIT | 2026-03-11 | MCP server giving agents live access to a running RE Engine game |
| A19 | [7etsuo/anvil](https://github.com/7etsuo/anvil) | 34 | none | 2026-07-16 | Agent-native multi-genre game engine for AI agents |
| A20 | [brandlll-lee/SolersEngine](https://github.com/brandlll-lee/SolersEngine) | 37 | MIT | 2026-09-05 | AI-native game engine built on Godot |
| A21 | [IvanMurzak/GameDev-MCP-Server](https://github.com/IvanMurzak/GameDev-MCP-Server) | 16 | Apache-2.0 | 2026-10-22 | Engine-agnostic MCP server shared by Unity, Godot, Unreal |
| A22 | [Tristan578/project-forge](https://github.com/Tristan578/project-forge) | 11 | NOASSERTION | 2026-10-21 | AI-native 3D browser engine with MCP commands (known: 2026-10-17 steal 12) |
| A23 | [FunplayAI/Funplay](https://github.com/FunplayAI/Funplay) | 9 | MIT | 2026-07-31 | AI game dev workbench across engines |
| A24 | [lsgmasa33/modoki-engine](https://github.com/lsgmasa33/modoki-engine) | 16 | Apache-2.0 | 2026-10-21 | Claude-friendly ECS engine plus visual editor, Three.js/WebGPU |
| A25 | [jsvd/arcane](https://github.com/jsvd/arcane) | 7 | Apache-2.0 | 2026-03-03 | Code-first, test-native, agent-native 2D engine |
| A26 | [holokat/AetherFX](https://github.com/holokat/AetherFX) | 6 | MIT | 2026-09-20 | AI-native game VFX authored by agents over MCP |
| A27 | [orkitec/orkige](https://github.com/orkitec/orkige) | 5 | Apache-2.0 | 2026-08-10 | C++20 engine with an AI-native editor, agents run and test games |
| A28 | [edwardgushchin/Electron2D](https://github.com/edwardgushchin/Electron2D) | 5 | MIT | 2026-10-21 | Agent-native cross-platform 2D engine |
| A29 | [qiulinfan/amoris](https://github.com/qiulinfan/amoris) | 5 | MIT | 2026-10-21 | Rust plus TypeScript 3D engine, native/WebGPU, web editor (known: 2026-10-17 steal 13) |
| A30 | [BlackBearCC/vitric](https://github.com/BlackBearCC/vitric) | 3 | MIT | 2026-08-25 | Glass-box engine for AI agents, every state visible and verifiable (known) |
| A31 | [tfoutrein/toile-engine](https://github.com/tfoutrein/toile-engine) | 3 | none | 2026-06-16 | 2D AI-native open-source engine in Rust |
| A32 | [YanfengLiu/civ-engine](https://github.com/yanfengliu/civ-engine) | 2 | MIT | 2026-10-18 | AI-first engine for civilization-scale simulation |
| A33 | [openwenyoo/wenyoo](https://github.com/openwenyoo/wenyoo) | 1 | MIT | 2026-09-07 | AI-native engine, worlds declared in YAML |
| A34 | [newtro/Artificer](https://github.com/newtro/Artificer) | 1 | none | 2026-10-12 | Headless-first Rust engine built for autonomous agents |
| A35 | [ryjin111/forgeengine](https://github.com/ryjin111/forgeengine) | 0 | MIT | 2026-06-11 | Agent-native deterministic engine, any AI builds games as data |
| A36 | [basedglye/claude-engine](https://github.com/basedglye/claude-engine) | 0 | MIT | 2026-09-04 | AI-native engine plus Claude Code plugin, deterministic sim kernel |
| A37 | [developerz-ai/nexus-engine](https://github.com/developerz-ai/nexus-engine) | 0 | MIT | 2026-10-07 | AI-first cross-platform engine, built and maintained by AI |
| A38 | [adrian-wulf/gamewache](https://github.com/adrian-wulf/gamewache) | 0 | MIT | 2026-09-20 | GameOps engine: crash sentinel, deterministic blackbox replay (known) |
| A39 | [Ubik42/Noemancer](https://github.com/Ubik42/Noemancer) | 1 | Apache-2.0 | 2026-09-09 | C++20 engine with agent-native editor and semantic runtime state |
| A40 | [Fulviuus/defold-mcp](https://github.com/Fulviuus/defold-mcp) | 13 | MIT | 2026-06-29 | MCP server for Defold: build, bundle, run, hot-reload, inspect |

### Class B: browser and WebGPU engines (Rust or JS, wasm, WebGPU renderers)

| # | Repo | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| B1 | [dylanebert/shallot](https://github.com/dylanebert/shallot) | 332 | MIT | 2026-10-22 | WebGPU game engine, Rust core plus TS, CEF/wry native shell |
| B2 | [esengine/estella](https://github.com/esengine/estella) | 693 | Apache-2.0 | 2026-10-21 | Fast 2D/3D engine, TypeScript SDK, C++/WebAssembly core, visual editor; in-tree bench for replication scaling, LOD, interest floor and residency |
| B3 | [Ruddle/oxidator](https://github.com/Ruddle/oxidator) | 361 | MIT | 2023-09-11 | RTS engine in Rust and WebGPU |
| B4 | [gents83/INOX](https://github.com/gents83/INOX) | 128 | MIT | 2026-09-28 | Rust game engine integrated in Blender, WebGPU ready |
| B5 | [Sunset-Studios/Sundown](https://github.com/Sunset-Studios/Sundown) | 124 | MIT | 2026-09-30 | WebGPU game engine |
| B6 | [flarelink-in/NullGraph](https://github.com/flarelink-in/NullGraph) | 103 | MIT | 2026-10-21 | Data-oriented WebGPU rendering framework for large web worlds |
| B7 | [2600th/Kinema](https://github.com/2600th/Kinema) | 61 | MIT | 2026-08-07 | Browser-native third-person gameplay lab |
| B8 | [Lichtso/contrast_renderer](https://github.com/Lichtso/contrast_renderer) | 86 | MIT | 2025-04-26 | WebGPU 2D render engine in Rust |
| B9 | [Refsa/pollus](https://github.com/Refsa/pollus) | 41 | MIT | 2026-09-16 | C# crossplatform engine on WGPU, native and browser |
| B10 | [tylerstuff/strata](https://github.com/tylerstuff/strata) | 0 | none | 2026-09-06 | WebGPU browser engine, TS API, Rust/WASM CPU core, WGSL renderer |
| B11 | [seedeai/valo](https://github.com/seedeai/valo) | 9 | MIT | 2026-10-20 | WebGPU-native 2D render engine in Rust, five platforms |
| B12 | [andreirx/zap-engine](https://github.com/andreirx/zap-engine) | 4 | none | 2026-07-07 | Headless, data-oriented engine for the modern web |
| B13 | [drftrun/driftengine](https://github.com/drftrun/driftengine) | 15 | Apache-2.0 | 2026-10-18 | 3D engine in strict TypeScript, WebGPU with WebGL2 fallback (known: hunt2 agent 4) |
| B15 | [not-fl3/macroquad](https://github.com/not-fl3/macroquad) | 4676 | Apache-2.0 | 2026-08-18 | Cross-platform Rust engine with a dependency-light web build (known) |
| B17 | [phase-rs/phase](https://github.com/phase-rs/phase) | 307 | Apache-2.0 | 2026-10-22 | Rules engine and client, Rust plus WASM plus React |
| B18 | [FWuermse/flow-ngin](https://github.com/FWuermse/flow-ngin) | 3 | Apache-2.0 | 2026-10-20 | Simple crossplatform instancing engine, full WASM compatibility |
| B19 | [CAD97/pyrr](https://github.com/CAD97/pyrr) | 3 | MPL-2.0 | 2024-01-24 | Experimental engine on the Rust wasm component model |
| B20 | [euuuuuuan/fatal-funnel-public](https://github.com/euuuuuuan/fatal-funnel-public) | 0 | Apache-2.0 | 2026-07-30 | Top-down tactical game in the browser, stop-time and line drawing |
| B21 | [braedonsaunders/voidstrike](https://github.com/braedonsaunders/voidstrike) | 26 | MIT | 2026-04-28 | Browser RTS with WebGPU graphics and a server |
| B22 | [sheng-neo/ra2-web](https://github.com/sheng-neo/ra2-web) | 20 | MIT | 2026-10-18 | Browser RTS engine, lockstep multiplayer, written by an AI overnight |

### Class C: deterministic / replay-first engines

| # | Repo | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| C1 | [gschup/ggrs](https://github.com/gschup/ggrs) | 698 | NOASSERTION (MIT or Apache-2.0 in tree) | 2026-10-19 | GGPO re-imagined, P2P rollback in Rust (known: several LADDER ports) |
| C2 | [renew-engine/renew](https://github.com/renew-engine/renew) | 28 | Apache-2.0 | 2026-09-21 | AI-first Rust engine, bit-deterministic fixed-tick, refusals catalogue |
| C3 | [JiepengTan/LockstepEngine](https://github.com/JiepengTan/LockstepEngine) | 966 | none | 2020-03-23 | Lockstep with deterministic math, collision, navmesh, behaviour tree libs |
| C4 | [mrdav30/LockstepRTSEngine](https://github.com/mrdav30/LockstepRTSEngine) | 186 | MIT | 2026-08-06 | Deterministic lockstep RTS engine (WIP) |
| C5 | [mrdav30/FixedMathSharp](https://github.com/mrdav30/FixedMathSharp) | 94 | MIT | 2026-10-20 | Deterministic fixed-point math library for .NET |
| C6 | [mrdav30/GridForge](https://github.com/mrdav30/GridForge) | 21 | MIT | 2026-10-07 | Deterministic voxel grid for spatial partitioning and simulation |
| C7 | [xpTURN/Klotho](https://github.com/xpTURN/Klotho) | 65 | Apache-2.0 | 2026-09-20 | Deterministic Unity and Godot framework for rollback, lockstep, server-auth |
| C8 | [FelixMiddelhoff/foldback](https://github.com/FelixMiddelhoff/foldback) | 0 | Apache-2.0 (MIT OR Apache-2.0 in tree) | 2026-10-20 | Desync and divergence debugger for lockstep/rollback games |
| C9 | [cosgunhalil/Tickwise](https://github.com/cosgunhalil/Tickwise) | 0 | Apache-2.0 | 2026-10-05 | Record, replay and diff deterministic simulations (known: LADDER card) |
| C10 | [ampactor-labs/two-top](https://github.com/ampactor-labs/two-top) | 1 | none | 2026-10-12 | Phone-versus-phone rollback brawler, Bevy plus ggrs plus Matchbox |
| C11 | [EndersGamesDev/EmberEngine](https://github.com/EndersGamesDev/EmberEngine) | 1 | none | 2026-09-19 | Rust plus wgpu, deterministic 60 Hz lockstep, one wasm bundle (known: hunt 2026-10-21) |
| C12 | [dhyabi2/klokwork](https://github.com/dhyabi2/klokwork) | 0 | MIT | 2026-06-15 | Deterministic rollback-netcode engine for Three.js, replays |
| C13 | [SahilKhutey/Primodials](https://github.com/SahilKhutey/Primodials) | 0 | NOASSERTION | 2026-08-21 | Data-oriented C++23 simulation engine, deterministic |
| C14 | [pietrobassi/deterministic-lockstep-demo](https://github.com/pietrobassi/deterministic-lockstep-demo) | 18 | MIT | 2025-04-13 | Deterministic lockstep networking demo in the browser |
| C15 | [valeriyr/ferrets](https://github.com/valeriyr/ferrets) | 5 | Apache-2.0 | 2026-10-21 | Deterministic RTS engine in Rust, lockstep, Lua content (known: 2026-10-18) |
| C16 | [ext-sakamoro/ALICE-Physics](https://github.com/ext-sakamoro/ALICE-Physics) | 3 | AGPL-3.0 | 2026-10-22 | Deterministic physics on 128-bit fixed point, bit-exact replay |
| C17 | [Invisible-Door-Studios/enaction-engine](https://github.com/Invisible-Door-Studios/enaction-engine) | 1 | AGPL-3.0 | 2026-10-21 | Rust foundations for deterministic sims, fixed-step timing |
| C18 | [forgottenhistory/Archon-Engine](https://github.com/ForgottenHistory/Archon-Engine) | 58 | MIT | 2026-10-21 | Grand strategy engine for Unity with deterministic simulation |
| C19 | [AgnosticArchitect/ace-os-v8](https://github.com/AgnosticArchitect/ace-os-v8) | 22 | MIT | 2026-05-28 | Deterministic causality engine for AI roleplay |
| C20 | [HannnibalKing/dds-ts-simulation-engine](https://github.com/HannnibalKing/dds-ts-simulation-engine) | 0 | NOASSERTION | 2026-10-13 | Bitwise-deterministic distributed sim, rollback, replay, fixed point |

## 2. The eight ranked steals (value per effort)

Value 1-5 (5 best). Effort S = under a day, M = days, L = a week or more.
"Engine has it" is answered with a grep path in this repo, read 2026-10-22.

| # | Steal | Repo @ rev | License | What exactly to take | Lands in | Engine has it? | V/E |
|---|---|---|---|---|---|---|---|
| 1 | The desync report that answers "whose fault" in one bit | [renew-engine/renew@91ba705](https://github.com/renew-engine/renew/tree/91ba70530be0e506b3b52d49b852840e7eaecf9a) `crates/net/src/desync.rs` (read live 2026-10-22) | Apache-2.0 | `DesyncReport`: tick, local peer, per-peer **state** digests AND per-peer **input** digests, `last_agreed_tick`, and the one-bit `inputs_agree()`. When inputs agree but state differs, the network is exonerated and the bug is in the sim, the toolchain, the content or the rules. Also `witnessed()`: a tick nobody reported must never read as agreement. | `crates/net/src/rollback.rs` (next to `check_desync`, line 1333) | No. `check_desync` compares one peer hash and returns a `Desync` error; there is no input digest on the wire and no report struct. Grep `input_digest` in `crates/net`: no hit. | 5 / S |
| 2 | The determinism test that cannot pass by doing nothing | [renew-engine/renew@91ba705](https://github.com/renew-engine/renew/tree/91ba70530be0e506b3b52d49b852840e7eaecf9a) `crates/frame/tests/determinism.rs` (read live 2026-10-22) | Apache-2.0 | `assert_not_vacuous()`: before any digest comparison, assert the trace actually stepped, the budget engaged, and the digest moved off its offset basis. Plus a negative control: a one-nanosecond perturbation of the input must change the answer, and a frozen digest constant that must fail when the arithmetic changes. | `crates/proof/src/fuzz.rs`, `crates/proof/src/tape_fuzz.rs` | Partly. `crates/proof/src/fuzz.rs` has a determinism fuzzer and `crates/proof/src/break_corpus.rs` a corpus, but no "the trace did nothing" guard. Grep `not_vacuous` in `crates/proof`: no hit. | 5 / S |
| 3 | The deterministic gate between an untrusted spec producer and the engine | [ryjin111/forgeengine@f600634](https://github.com/ryjin111/forgeengine/tree/f6006347cf1693a4eee43b13c540b085df2553a2) `src/builder/gate.ts` (read live 2026-10-22) | MIT | Fail-closed pipeline: parse -> limits -> vocabulary -> validate -> playability (headless matches, replay-verified, fixed seeds). Resource caps so an LLM cannot emit a valid 10^6-cell map. A repair callback injected, so the gate stays LLM-free and network-free. The only trusted spec is one that came out of the gate. | `crates/endpoint/src/packs/verify.rs` (verifier output in every build verb, gap D19) | Partly. `crates/endpoint/src/packs/verify.rs` returns `first_divergent_tick` and the door refuses bad briefs, but there is no resource cap stage and no "repair budget exhausted" verdict. | 5 / M |
| 4 | The replay tape as a first-class agent artifact | [7etsuo/anvil@699f0f6](https://github.com/7etsuo/anvil/tree/699f0f6a07c4d2c649c5a44f97a9a578db5677db) `anvil/packages/core/src/agent/replay.ts` (read live 2026-10-22) | none found in the API (no LICENSE field) | `ReplayTape` as a versioned JSON object (`v: 1`, seed, frames of actions, optional meta) with `ReplayRecorder` and `playReplay()`, which **refuses** when the live handle seed does not match the tape seed. An agent writes a tape, then replays it to prove "same actions, same outcome". | `crates/endpoint/src/packs/session.rs` (door verbs) plus `crates/proof` tape | Partly. `crates/observe/src/event_queue.rs` has `tape_digest` and `crates/net/src/replay.rs` replays a net tape, but there is no agent-facing tape with a seed-mismatch refusal. | 4 / S-M |
| 5 | Observe as a diff, never a dump | [7etsuo/anvil@699f0f6](https://github.com/7etsuo/anvil/tree/699f0f6a07c4d2c649c5a44f97a9a578db5677db) `anvil/packages/core/src/agent/observeDiff.ts` (read live 2026-10-22) | none found in the API | `observeDiff(from, to)`: added and removed entities, per-entity hp and position changes, scene change, and a one-line `summary` (`"scene a->b; +entities x; hp y:10->4"`). `observeSummary()` is a single short line for the LLM context. The comment says it plainly: agents should reason on deltas, not full dumps. | `crates/endpoint/src/packs/session.rs` (`scene.diff` already lands deltas; the per-entity view is the new part) | Partly. `crates/endpoint/src/packs/session.rs` has `scene.diff` (added/removed/changed nodes) and `session.activity`. No per-entity hp/position delta for the fight view. Grep `entity_hp` in `crates`: no hit. | 4 / S |
| 6 | Trap the wall clock and `Math.random` inside the sim | [dhyabi2/klokwork@36c1ca8](https://github.com/dhyabi2/klokwork/tree/36c1ca807a37e45a972b263de1921a0b3d53e4b8) `src/determinism-guard.js` (read live 2026-10-22) | MIT | `trapGlobals()` replaces `Math.random`, `Date.now` and `performance.now` with versions that **throw with the offending name**, so the bad line is named at once. Plus `fuzzDeterminism()`: N seeded input sequences, two fresh sims driven tick for tick, compare the hash every tick, report the first divergent tick; and `assertDeterministic()` for one scenario. | `crates/core/src/hash.rs` and `crates/core/src/rng.rs` (one dev-only guard), `crates/proof/src/fuzz.rs` | Partly. `crates/core/src/rng.rs` is a seeded xorshift64 plus SplitMix64 substream, and `crates/web/src/golden.rs` notes `Instant` traps on wasm. There is no runtime trap that names a forbidden wall-clock call inside the tick. | 4 / S |
| 7 | A signed match statement beside the tape | [ampactor-labs/two-top@3e953a1](https://github.com/ampactor-labs/two-top/tree/3e953a1be676b27078f13abc3218a671f16ab4c0) `crates/app/src/attest.rs` (read live 2026-10-22) | none found in the API | Both peers observe the same settled facts, build the **identical canonical statement**, sign it with a per-install key, and send the signature over the reliable channel. The statement lands beside the tape as `<stem>.attest.json` and can be checked end to end later: re-run the tape, rebuild the statement, verify both signatures. Also the settle rule: a match is only "decided" once no rollback can undo it. | `crates/net/src/replay.rs` and `crates/proof/src/tape.rs` | No. Grep `attest` in `crates`: hits are only `crates/rome/src/certainty.rs` (a historical certainty tier) and a test name. No match statement and no signature anywhere. | 4 / M |
| 8 | The GPU transfer budget with a lease that has no renewal | [tylerstuff/strata@47a70f8](https://github.com/tylerstuff/strata/tree/47a70f8e2389f33d1977574b1e0fe21142a9c2b1) `packages/core/src/geometry/transfer-budget.ts` (read live 2026-10-22) | none found in the API | `GeometryTransferLimits` with `maxRequests`, `pageStagingBytes`, `transientBytes`, `residentBytes`, `gpuBufferBytes` and `uploadBytesPerFrame`. A request gets a **lease**: `release()` frees it, `finishRequest()` frees the slot but keeps the validated payload reserved. A frame budget writes bytes and returns false on temporary frame pressure. A failed observer never strands another initializer. | `crates/assets` (load queue, gap A2) and `crates/render/src/present/cache.rs` | No. `crates/assets/src/asset_source/bundle.rs` sorts blobs by a static `(priority, path)` order; there is no byte budget, no lease and no cancel. Grep `transfer.?budget` in `crates/render`: no hit. | 4 / M |

Why these eight on top: 1, 2 and 6 are small and move a real gate (G6 same hash, G8 multiplayer).
3 is the missing half of "hard systems one call" for an agent: the engine's own verdict, not the
model's. 4 and 5 are the cheapest way to make the door cheaper to call, which is the whole
AI-native bet. 7 is the only thing in this survey that lets two machines prove they played
the same match. 8 is the one that makes the potato and the browser hold at once.

## 3. What these engines do that engine2040 does not

**1. The engine names the fault, not the symptom.** renew's `DesyncReport` carries an
input digest beside the state digest, so "the peers ran the same inputs and got different
state" is one bit (`inputs_agree`). Our `check_desync` (`crates/net/src/rollback.rs:1333`)
compares one peer hash and returns `RollbackError::Desync`. It says the two hashes differ.
It cannot say whether the network lied or the simulation drifted, so a human still has to
guess. `witnessed()` is the other half: a tick nobody reported must never read as
agreement. Source: https://github.com/renew-engine/renew/blob/91ba70530be0e506b3b52d49b852840e7eaecf9a/crates/net/src/desync.rs
(read 2026-10-22).

**2. A determinism test can fail by doing nothing, and nobody noticed.** renew's
`crates/frame/tests/determinism.rs` calls `assert_not_vacuous()` before every digest
comparison: the trace must have stepped, the step budget must have engaged, and the digest
must have moved off its offset basis. A negative control perturbs the input by one
nanosecond and requires the answer to change. Our `crates/proof/src/fuzz.rs` and
`crates/proof/src/break_corpus.rs` compare digests; a test that stopped exercising the sim
would still go green. Source:
https://github.com/renew-engine/renew/blob/91ba70530be0e506b3b52d49b852840e7eaecf9a/crates/frame/tests/determinism.rs
(read 2026-10-22).

**3. The browser build has a GPU policy, not just a GPU path.** shallot's
`crates/native/src/gpu_policy.rs` decides *before* the first frame whether a real hardware
GPU is required, and publishes that decision into the page
(`__shallotRequireHardwareGpu`). The page then knows why it is slow. Our web build has
one path: WebGPU. `crates/web/src/limits.rs:204` has a test named
`webgl2_class_limits_would_fail_the_renderer_need`, so a WebGL2 browser fails today with
no ladder. The 2026-10-22 engines report (ENG1) already proposed the probe; what shallot
adds is the *policy* half, the flag the page reads. Source:
https://github.com/dylanebert/shallot/blob/705fa94526b130061aa0d3a6cc5be7ac7c01402b/crates/native/src/gpu_policy.rs
(read 2026-10-22).

**4. Nobody trusts a model to say the build is good.** forgeengine's gate is LLM-free and
network-free: the repair callback is injected, so the gate itself never calls a model. It
runs the candidate game headless, plays it on fixed seeds, checks the replay, and returns
a stage name (`parse`, `limits`, `vocabulary`, `validate`, `playability`, `internal`) plus a
loud flag when any match hit the turn cap. The 2026-10-22 report already said a vision
model must not be a gate. forgeengine shows the same discipline for the **sim**: the model's
verdict never enters, only the engine's. Source:
https://github.com/ryjin111/forgeengine/blob/f6006347cf1693a4eee43b13c540b085df2553a2/src/builder/gate.ts
(read 2026-10-22).

**5. Two machines can prove they played the same match.** two-top signs a canonical match
statement with a per-install key and stores it beside the replay tape as
`<stem>.attest.json`; the check is "re-run the tape, rebuild the statement, verify both
signatures". The rule for when a match counts as decided is exact: settled when the
session's confirmed frame has passed the frame the decision was first seen at, not after
a timer. Our G8 gate wants a duel that ends on one digest, and `crates/net` has a digest
comparison, but there is no signed statement and no artifact a third party can check.
Source:
https://github.com/ampactor-labs/two-top/blob/3e953a1be676b27078f13abc3218a671f16ab4c0/crates/app/src/attest.rs
(read 2026-10-22).

## 4. Traps and honest limits

- **renew** has 28 stars and one author. Its `REFUSALS.md` is 71 KB and its tree has
  2188 files. It is a reference for the *shape* of a refusal and of a desync report, not a
  dependency. `crates/net/src/desync.rs` and `crates/frame/src/digest.rs` are the two files
  worth reading; the rest is a full engine we do not want.
- **foldback** (C8) is the closest tool to our `crates/proof` chaos lane: a desync
  debugger with `find_first_divergence` over per-tick per-peer hashes. Its `bisect.rs` is
  100 lines and its plan says Level 2 (per-entity) and Level 3 (per-field) are not
  designed yet. Our `crates/spatial/src/fuzz.rs:499` already has `first_divergence` and
  `crates/observe/src/timeline.rs:238` names a `TimelineDivergence`, so the tick level is
  covered. Read it for the peer-majority shape, not for a new capability.
- **ggrs** (C1) is known and already mined for `input_queue.rs` and `time_sync.rs`. It is
  in the table for the count, not as a new steal. Its repo license field is NOASSERTION
  because the tree holds two license files; the LADDER card records MIT.
- **anvil** (A19) has no license field in the API, so it is an idea-only donor. Read the
  shape; do not copy bytes. Same for **two-top** (C10) and **strata** (B10).
- **ALICE-Physics** (C16) and **enaction-engine** (C17) are AGPL-3.0. Do not copy code
  from either. Read the idea, write our own.
- **shalot's** native shell is CEF or wry, a whole browser in the process. We already have
  a page (`crates/web`). Take the policy flag, not the shell.
- **estella** (B2, 693 stars) is the biggest new engine in this survey and it is C++ plus
  a TypeScript SDK, not Rust. Its bench tree is the interesting part (replication scaling,
  LOD, interest floor, residency), and it is a C++ codebase we cannot port. It is a
  report to read, not a donor.
- **driftengine** (B13) was already read on 2026-10-21 (hunt2 agent 4): its
  `packages/script/src/bindings/rollback.ts` is a thin binding and the core was not
  readable in the time available, so no steal is named from it here either.
- **tokening, not characters.** modoki's `docs/mcp-response-budget.md` measures the
  `chars/4` estimate against a real BPE tokenizer and finds it under-reports JSON payloads
  by 25 to 38 percent, and that hex GUIDs fragment at 1.80 chars per token while trait
  names tokenize well. Our door budget in `crates/endpoint/src/mcp.rs:150` is measured in
  **characters**. That is the same trap, and it is worth a follow-up row before the budget
  is tightened again. Source:
  https://github.com/lsgmasa33/modoki-engine/blob/1d0fe431ef8c07451a8cb59626595edc74dc6304/docs/mcp-response-budget.md
  (read 2026-10-22).
- **The AI-native class is crowded at the top and empty below.** 30 of the 40 class-A
  repos are skills, prompts, wrappers over Godot or Unreal, or benchmark suites. Only a
  handful (A19 anvil, A34 Artificer, A35 forgeengine, A36 claude-engine, A37 nexus-engine)
  are engines whose core is designed for an agent. None of them ships a determinism
  guarantee across targets. The niche engine2040 holds (bit-identical sim, native plus
  wasm, an AI door) is still empty in the public repos, which is why the steal list above
  is about *shapes* (a report, a gate, a tape, a trap) and not about whole engines.

## 5. Sources and what I could not verify

Sources, all read 2026-10-22:

- `https://api.github.com/repos/<owner>/<repo>` for every repo in the catalogue: stars,
  license, last push, default branch.
- `https://api.github.com/repos/<owner>/<repo>/commits?per_page=1` for the sha of the
  twelve deep reads.
- `https://api.github.com/repos/<owner>/<repo>/git/trees/<sha>?recursive=1` for the file
  list of the deep reads.
- `https://raw.githubusercontent.com/<owner>/<repo>/<sha>/<path>` for every file body named
  in the steal table. The twelve files read in full: renew `crates/frame/src/digest.rs`,
  `crates/net/src/desync.rs`, `REFUSALS.md`, `crates/frame/tests/determinism.rs`; Artificer
  `crates/artificer_testkit/src/replay.rs`, `crates/artificer_agent/src/lib.rs`; klokwork
  `src/determinism-guard.js`; foldback `crates/foldback-core/src/bisect.rs`; anvil
  `anvil/packages/core/src/agent/replay.ts`, `anvil/packages/core/src/agent/observeDiff.ts`;
  two-top `crates/app/src/attest.rs`, `.github/workflows/determinism.yml`; shallot
  `crates/native/src/gpu_policy.rs`; strata
  `packages/core/src/geometry/transfer-budget.ts`; forgeengine `src/builder/gate.ts`;
  modoki `docs/mcp-response-budget.md`.

Not verified, stated plainly:

- No donor was built, run or benchmarked. Every "what it does" line comes from the file
  or the repo description, not from execution.
- The description one-liner in the catalogue is the GitHub API `description` field,
  shortened. It is the author's claim, not a verified feature list.
- B14 was a placeholder row for a candidate name the API would not confirm today. It was
  removed rather than guessed. The catalogue is 80 named repos, checked with a regex.
- `renew`, `anvil`, `two-top`, `strata` and `klokwork` were read at one sha each. Their
  default branches move; a later read may differ.
- The class-A repos were found by API search, so the set is what GitHub's index holds
  today. A repo with a different description (no "engine" word) is invisible to these
  queries and is missing from the table.
- Search quota: 10 searches per minute, 60 core API calls per hour. The core limit
  reset once mid-run; no search query was repeated.
- The door budget in `crates/endpoint/src/mcp.rs:150` is in characters. modoki's
  measurement says a real tokenizer disagrees by 25 to 38 percent on JSON. Our own
  numbers should be re-measured with a real tokenizer before the budget is trusted.
  Not measured here.
