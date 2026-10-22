---
title: "Engine survey 2026-10-22, part 1: Rust and small indie engines"
date: 2026-10-22
---

# Engine survey 2026-10-22, part 1: Rust and small indie engines

A wide catalogue: 126 game engines, libraries and runtimes, Rust first, then C, C++, C#, JavaScript, TypeScript, Go, Zig, Lua, Python and nine smaller languages. Every star count, licence and last-push date below was read from the GitHub REST API today (2026-10-22). Every claim about what a repo does comes from its README or from a file I read live at the pinned revision. Where I did not read the file, the row says "listed, not read".

This is a fresh sweep. It does not repeat `docs/research/finish-line-2026-10-17/02-github.md` (25 ranked steals) or the `docs/research/steals-*.md` cards. Where a donor here overlaps an earlier card, I say so.

## Method

- Search: one narrow GitHub search per language and per topic (`topic:game-engine`, `topic:gameengine`, `topic:ecs`, plus `mcp game engine` and `deterministic game engine`). No query repeated.
- Facts: `gh api repos/<owner>/<repo>` for stars, `license.spdx_id` and `pushed_at`, one call per repo, 2026-10-22. Unauthenticated `api.github.com` rate-limits hard, so `gh` was the path.
- Deep reads: 12 repos, default branch sha from `gh api repos/<o>/<r>/commits/<branch>`, then the exact file read over `raw.githubusercontent.com/<o>/<r>/<sha>/<path>`. Never a guessed raw URL. The file path I read is named in every steal row.
- Gaps: every "engine does not have it" claim was checked with the `grep` tool against `crates/`, always with `path` and `include`. The owning crate is named.

The table is a catalogue, not a ranking. It is here so the next reader does not have to re-search 126 repos to find one donor.

## 1. The catalogue (126 engines)

All rows read from the API on 2026-10-22. `NOASSERTION` means GitHub could not classify the licence file; `none` means there is no licence file. Sorted by stars inside each language.

### Rust (39)

| # | Engine | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| 1 | [bevyengine/bevy](https://github.com/bevyengine/bevy) | 49,194 | Apache-2.0 | 2026-10-21 | A refreshingly simple data-driven game engine built in Rust |
| 2 | [kitao/pyxel](https://github.com/kitao/pyxel) | 18,437 | NOASSERTION | 2026-10-21 | A retro game engine for Python |
| 3 | [gfx-rs/wgpu](https://github.com/gfx-rs/wgpu) | 18,345 | Apache-2.0 | 2026-10-22 | Cross-platform, safe, pure-Rust graphics API |
| 4 | [FyroxEngine/Fyrox](https://github.com/FyroxEngine/Fyrox) | 9,609 | MIT | 2026-10-21 | 3D and 2D game engine written in Rust |
| 5 | [amethyst/amethyst](https://github.com/amethyst/amethyst) | 8,035 | NOASSERTION | 2021-12-06 | Data-oriented and data-driven game engine written in Rust |
| 6 | [image-rs/image](https://github.com/image-rs/image) | 5,894 | Apache-2.0 | 2026-09-15 | Encoding and decoding images in Rust |
| 7 | [gfx-rs/gfx](https://github.com/gfx-rs/gfx) | 5,393 | Apache-2.0 | 2023-02-27 | A low-overhead Vulkan-like GPU API for Rust |
| 8 | [PistonDevelopers/piston](https://github.com/PistonDevelopers/piston) | 4,711 | MIT | 2026-08-26 | A modular game engine written in Rust |
| 9 | [ggez/ggez](https://github.com/ggez/ggez) | 4,710 | MIT | 2026-10-21 | Rust library to create a Good Game Easily |
| 10 | [not-fl3/macroquad](https://github.com/not-fl3/macroquad) | 4,676 | Apache-2.0 | 2026-08-18 | Cross-platform game engine in Rust. |
| 11 | [AmbientRun/Ambient](https://github.com/AmbientRun/Ambient) | 3,908 | Apache-2.0 | 2025-01-07 | The multiplayer game engine |
| 12 | [valence-rs/valence](https://github.com/valence-rs/valence) | 3,281 | MIT | 2026-06-15 | A Rust framework for building Minecraft servers. |
| 13 | [PistonDevelopers/hematite](https://github.com/PistonDevelopers/hematite) | 1,918 | MIT | 2026-10-11 | A simple Minecraft written in Rust with the Piston game engine |
| 14 | [linebender/tiny-skia](https://github.com/linebender/tiny-skia) | 1,663 | BSD-3-Clause | 2026-08-20 | A tiny Skia subset ported to Rust |
| 15 | [gfx-rs/naga](https://github.com/gfx-rs/naga) | 1,566 | NOASSERTION | 2023-11-09 | Universal shader translation in Rust |
| 16 | [hecrj/coffee](https://github.com/hecrj/coffee) | 1,149 | MIT | 2024-03-03 | An opinionated 2D game engine for Rust |
| 17 | [hyperion-mc/hyperion](https://github.com/hyperion-mc/hyperion) | 1,092 | Apache-2.0 | 2026-10-15 | Minecraft game engine for massive custom events |
| 18 | [17cupsofcoffee/tetra](https://github.com/17cupsofcoffee/tetra) | 1,010 | MIT | 2026-07-22 | A simple 2D game framework written in Rust |
| 19 | [brettchalupa/usagi](https://github.com/brettchalupa/usagi) | 868 | Unlicense | 2026-10-09 | A simple 2D game engine for rapid prototyping with Lua, featuring live reload and cross- |
| 20 | [markusmoenig/Eldiron](https://github.com/markusmoenig/Eldiron) | 826 | MIT | 2026-10-15 | Create cross-platform classic RPGs in 2D and 3D with powerful built-in world editing too |
| 21 | [darthdeus/comfy](https://github.com/darthdeus/comfy) | 736 | Apache-2.0 | 2025-11-24 | Comfy is a fun 2D game engine built in Rust. It's designed to be opinionated, productive |
| 22 | [Far-Beyond-Pulsar/Pulsar-Native](https://github.com/Far-Beyond-Pulsar/Pulsar-Native) | 398 | MIT | 2026-10-21 | The next-generation game engine that you deserve implemented in rust https://pulsarnativ |
| 23 | [diptools/dip](https://github.com/diptools/dip) | 364 | NOASSERTION | 2026-04-13 | Rust application framework focuses on composability, ECS and Web3.  Powered by Bevy game |
| 24 | [fishfolk/bones](https://github.com/fishfolk/bones) | 319 | NOASSERTION | 2026-10-12 | An easy-to-use game engine for making real games. |
| 25 | [lowenware/dotrix](https://github.com/lowenware/dotrix) | 311 | MIT | 2026-09-18 | A 3D engine with ECS and Vulkan renderer for Rust developers |
| 26 | [toxoidengine/toxoid](https://github.com/toxoidengine/toxoid) | 300 | NOASSERTION | 2026-10-17 | A modern, cross-platform, highly modular / decoupled, data-driven, ECS-based game engine |
| 27 | [StarArawn/harmony](https://github.com/StarArawn/harmony) | 171 | Zlib | 2020-08-07 | A modern 3D/2D game engine that uses wgpu. |
| 28 | [PistonDevelopers/piston_window](https://github.com/PistonDevelopers/piston_window) | 161 | MIT | 2026-10-11 | The official Piston Window for the Piston game engine |
| 29 | [kettle11/koi](https://github.com/kettle11/koi) | 151 | Apache-2.0 | 2023-10-01 | A game engine. |
| 30 | [Zeenobit/moonshine_save](https://github.com/Zeenobit/moonshine_save) | 148 | MIT | 2026-10-11 | A save/load framework for Bevy game engine. |
| 31 | [PistonDevelopers/turbine](https://github.com/PistonDevelopers/turbine) | 145 | Apache-2.0 | 2026-10-08 | 3D game engine for content production |
| 32 | [gents83/INOX](https://github.com/gents83/INOX) | 128 | MIT | 2026-09-28 | Rust Game engine integrated in Blender [WebGPU ready] |
| 33 | [balaurengine/balaur](https://github.com/balaurengine/balaur) | 96 | MIT | 2026-10-18 | A 2D & 3D node-based game engine, fully deterministic, with scripts that reload in milli |
| 34 | [renew-engine/renew](https://github.com/renew-engine/renew) | 28 | Apache-2.0 | 2026-09-21 | renew engine: an AI-first game engine in Rust, built to be operated. Bit-deterministic f |
| 35 | [aethelisdev/aeon-engine](https://github.com/aethelisdev/aeon-engine) | 24 | MPL-2.0 | 2026-10-21 | The 3D Safe Modular game engine, written in Rust |
| 36 | [RustingStudio/RustingEngine](https://github.com/RustingStudio/RustingEngine) | 13 | NOASSERTION | 2026-10-20 | Rust game engine for huge physics scenes: 10,000 GPU-simulated cubes at 2,000+ FPS on an |
| 37 | [marmikshah/tono](https://github.com/marmikshah/tono) | 6 | MIT | 2026-10-21 | A deterministic Rust audio engine for sound effects, instruments, songs, and adaptive ga |
| 38 | [SpideyZac/polysim](https://github.com/SpideyZac/polysim) | 4 | Apache-2.0 | 2026-08-28 | Wraps the PolyTrack WASM physics engine in a safe, ergonomic API for deterministic, offl |
| 39 | [synthalorian/chronos-engine](https://github.com/synthalorian/chronos-engine) | 2 | Apache-2.0 | 2026-10-18 | A genre-agnostic ECS game engine in Rust. Zero dependencies (core). Deterministic by des |

### C++ (15)

| # | Engine | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| 1 | [godotengine/godot](https://github.com/godotengine/godot) | 118,819 | MIT | 2026-10-21 | Godot Engine  Multi-platform 2D and 3D game engine |
| 2 | [o3de/o3de](https://github.com/o3de/o3de) | 9,766 | NOASSERTION | 2026-10-21 | Open 3D Engine (O3DE) is an Apache 2.0-licensed multi-platform 3D engine that enables de |
| 3 | [turanszkij/WickedEngine](https://github.com/turanszkij/WickedEngine) | 7,305 | MIT | 2026-10-21 | 3D engine with modern graphics |
| 4 | [FlaxEngine/FlaxEngine](https://github.com/FlaxEngine/FlaxEngine) | 7,073 | NOASSERTION | 2026-10-21 | Flax Engine  multi-platform 3D game engine |
| 5 | [BoomingTech/Piccolo](https://github.com/BoomingTech/Piccolo) | 6,736 | MIT | 2024-09-30 | Piccolo (formerly Pilot)  mini game engine for games104 |
| 6 | [defold/defold](https://github.com/defold/defold) | 6,407 | NOASSERTION | 2026-10-21 | Defold is a completely free to use game engine for development of desktop, console, mobi |
| 7 | [Redot-Engine/redot-engine](https://github.com/Redot-Engine/redot-engine) | 6,141 | MIT | 2026-10-20 | Redot Engine  Multi-platform 2D and 3D game engine |
| 8 | [urho3d/urho3d](https://github.com/urho3d/urho3d) | 4,780 | MIT | 2023-01-25 | Game engine |
| 9 | [spring/spring](https://github.com/spring/spring) | 4,022 | NOASSERTION | 2024-03-31 | A powerful free cross-platform RTS game engine. - Report issues at https://springrts.com |
| 10 | [nem0/LumixEngine](https://github.com/nem0/LumixEngine) | 3,897 | MIT | 2026-10-21 | 3D C++ Game Engine - yet another open source game engine |
| 11 | [PanosK92/SpartanEngine](https://github.com/PanosK92/SpartanEngine) | 3,190 | NOASSERTION | 2026-10-21 | A game engine with a fully bindless, GPU-driven renderer featuring real-time path-traced |
| 12 | [crownengine/crown](https://github.com/crownengine/crown) | 2,461 | NOASSERTION | 2026-10-21 | A complete and cross-platform game engine designed for flexibility, performance and fast |
| 13 | [ezEngine/ezEngine](https://github.com/ezEngine/ezEngine) | 2,033 | MIT | 2026-10-20 | An open source C++ game engine. |
| 14 | [nCine/nCine](https://github.com/nCine/nCine) | 1,342 | MIT | 2026-10-18 | A cross-platform 2D game engine |
| 15 | [inexorgame/vulkan-renderer](https://github.com/inexorgame/vulkan-renderer) | 1,228 | MIT | 2026-10-12 | A new, render-graph based 3D game engine for Linux and Windows using C++20 and Vulkan AP |

### C (9)

| # | Engine | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| 1 | [raysan5/raylib](https://github.com/raysan5/raylib) | 35,360 | Zlib | 2026-10-21 | Simple and easy-to-use library for videogame programming |
| 2 | [bjornbytes/lovr](https://github.com/bjornbytes/lovr) | 2,612 | MIT | 2026-10-22 | Lua Virtual Reality Framework |
| 3 | [orx/orx](https://github.com/orx/orx) | 1,780 | Zlib | 2026-10-20 | ORX: Data-Driven Game Engine |
| 4 | [travisvroman/kohi](https://github.com/travisvroman/kohi) | 1,519 | Apache-2.0 | 2026-10-21 | A game engine made as part of the Kohi Game Engine series on YouTube (and Twitch!), wher |
| 5 | [scottcgi/Mojoc](https://github.com/scottcgi/Mojoc) | 1,293 | MIT | 2024-01-19 | A cross-platform, open-source, pure C  game engine for mobile game. |
| 6 | [RandyGaul/cute_framework](https://github.com/RandyGaul/cute_framework) | 853 | NOASSERTION | 2026-10-19 | The *cutest* framework out there for creating 2D and 3D games in C++! |
| 7 | [septag/rizz](https://github.com/septag/rizz) | 785 | BSD-2-Clause | 2022-10-24 | Small C game development framework |
| 8 | [r-lyeh/v1](https://github.com/r-lyeh/v1) | 526 | none | 2025-08-04 | 3D game framework in C, v1. |
| 9 | [clibequilibrium/EquilibriumEngine](https://github.com/clibequilibrium/EquilibriumEngine) | 347 | GPL-2.0 | 2025-09-02 | Data-oriented and multi-threaded C11 Game Engine with libraries & shaders hot-reloading. |

### C# (11)

| # | Engine | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| 1 | [OpenRA/OpenRA](https://github.com/OpenRA/OpenRA) | 17,618 | GPL-3.0 | 2026-10-20 | Open Source real-time strategy game engine for early Westwood games such as Command & Co |
| 2 | [MonoGame/MonoGame](https://github.com/MonoGame/MonoGame) | 14,555 | NOASSERTION | 2026-10-21 | One framework for creating powerful cross-platform games. |
| 3 | [egametang/ET](https://github.com/egametang/ET) | 9,943 | NOASSERTION | 2026-08-26 | Unity3D Client And C# Server Framework |
| 4 | [stride3d/stride](https://github.com/stride3d/stride) | 7,901 | MIT | 2026-10-21 | Stride (formerly Xenko), a free and open-source cross-platform C# game engine. |
| 5 | [sschmid/Entitas](https://github.com/sschmid/Entitas) | 7,677 | MIT | 2023-12-30 | Entitas is a super fast Entity Component System (ECS) Framework specifically made for C# |
| 6 | [isadorasophia/murder](https://github.com/isadorasophia/murder) | 3,348 | MIT | 2026-10-21 | Murder is a pixel art ECS game engine. |
| 7 | [AdamsLair/duality](https://github.com/AdamsLair/duality) | 1,422 | MIT | 2025-05-19 | a 2D Game Development Framework |
| 8 | [ProwlEngine/Prowl](https://github.com/ProwlEngine/Prowl) | 1,263 | MIT | 2026-10-20 | An Open Source C# 3D Game Engine under MIT license, inspired by Unity and featuring a co |
| 9 | [Doraku/DefaultEcs](https://github.com/Doraku/DefaultEcs) | 766 | MIT-0 | 2024-03-01 | Entity Component System framework aiming for syntax and usage simplicity with maximum pe |
| 10 | [space-wizards/RobustToolbox](https://github.com/space-wizards/RobustToolbox) | 705 | NOASSERTION | 2026-10-21 | Robust multiplayer game engine, used by Space Station 14 |
| 11 | [HexaEngine/HexaEngine](https://github.com/HexaEngine/HexaEngine) | 181 | MIT | 2026-08-22 | The official repo of the HexaEngine game engine |

### JavaScript (10)

| # | Engine | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| 1 | [mrdoob/three.js](https://github.com/mrdoob/three.js) | 116,743 | MIT | 2026-10-22 | JavaScript 3D Library. |
| 2 | [phaserjs/phaser](https://github.com/phaserjs/phaser) | 40,529 | MIT | 2026-08-21 | Phaser is a fun, free and fast 2D game framework for making HTML5 games for desktop and |
| 3 | [4ian/GDevelop](https://github.com/4ian/GDevelop) | 27,820 | NOASSERTION | 2026-10-21 | Open-source, cross-platform 2D/3D/multiplayer game engine designed for everyone. |
| 4 | [aframevr/aframe](https://github.com/aframevr/aframe) | 17,671 | MIT | 2026-07-13 | :a: Web framework for building virtual reality experiences. |
| 5 | [playcanvas/engine](https://github.com/playcanvas/engine) | 17,101 | MIT | 2026-10-21 | Powerful web graphics runtime built on WebGL, WebGPU, WebXR and glTF |
| 6 | [melonjs/melonJS](https://github.com/melonjs/melonJS) | 6,416 | MIT | 2026-10-22 | a modern & lightweight HTML5 game engine |
| 7 | [KilledByAPixel/LittleJS](https://github.com/KilledByAPixel/LittleJS) | 4,328 | MIT | 2026-10-20 | Tiny fast 2D/3D HTML5 game engine with many features and no dependencies. |
| 8 | [tentone/nunuStudio](https://github.com/tentone/nunuStudio) | 2,240 | MIT | 2026-03-15 | Web powered cross-platform 3D, WebXR game engine. |
| 9 | [eva-engine/eva.js](https://github.com/eva-engine/eva.js) | 1,786 | MIT | 2026-07-10 | Eva.js is a front-end game engine specifically for creating interactive game projects. |
| 10 | [pmgl/microstudio](https://github.com/pmgl/microstudio) | 1,193 | MIT | 2026-10-12 | Free, open source game engine online |

### TypeScript (6)

| # | Engine | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| 1 | [pixijs/pixijs](https://github.com/pixijs/pixijs) | 48,441 | MIT | 2026-10-21 | The HTML5 Creation Engine: Create beautiful digital content with the fastest, most flexi |
| 2 | [BabylonJS/Babylon.js](https://github.com/BabylonJS/Babylon.js) | 26,203 | Apache-2.0 | 2026-10-21 | Babylon.js is a powerful, beautiful, simple, and open game and rendering engine packed i |
| 3 | [excaliburjs/Excalibur](https://github.com/excaliburjs/Excalibur) | 2,358 | BSD-2-Clause | 2026-10-22 | Your friendly TypeScript 2D game engine for the web |
| 4 | [iamlukethedev/Claw3D](https://github.com/iamlukethedev/Claw3D) | 2,353 | MIT | 2026-08-21 | Claw3D is an open source 3D engine built on OpenClaw for creating games, simulations, an |
| 5 | [esengine/estella](https://github.com/esengine/estella) | 693 | Apache-2.0 | 2026-10-21 | A fast 2D and 3D game engine  TypeScript SDK, C++/WebAssembly core, visual editor. Ship |
| 6 | [suchipi/hex-engine](https://github.com/suchipi/hex-engine) | 675 | none | 2026-09-13 | A modern 2D game engine for the browser. |

### Go (9)

| # | Engine | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| 1 | [hajimehoshi/ebiten](https://github.com/hajimehoshi/ebiten) | 13,574 | Apache-2.0 | 2026-10-20 | A dead simple 2D game engine for Go |
| 2 | [KaijuEngine/kaiju](https://github.com/KaijuEngine/kaiju) | 4,724 | NOASSERTION | 2026-10-16 | General purpose 3D and 2D game engine using Go (golang) and Vulkan with built in editor |
| 3 | [g3n/engine](https://github.com/g3n/engine) | 3,115 | BSD-2-Clause | 2026-08-01 | Go 3D Game Engine (http://g3n.rocks) |
| 4 | [EngoEngine/engo](https://github.com/EngoEngine/engo) | 1,821 | MIT | 2026-03-30 | Engo is an open-source 2D game engine written in Go. |
| 5 | [oakmound/oak](https://github.com/oakmound/oak) | 1,665 | Apache-2.0 | 2026-08-10 | A pure Go game engine |
| 6 | [ikemen-engine/Ikemen-GO](https://github.com/ikemen-engine/Ikemen-GO) | 1,629 | NOASSERTION | 2026-10-21 | An open-source fighting game engine that supports MUGEN resources. |
| 7 | [dobyte/due](https://github.com/dobyte/due) | 973 | MIT | 2026-10-21 | A High-Performance Distributed Game Server Framework in Go |
| 8 | [cherry-game/cherry](https://github.com/cherry-game/cherry) | 820 | MIT | 2026-10-15 | Cherry  Go game server framework powered by Actor Model. Serial goroutine per actor, NA |
| 9 | [yohamta0/donburi-ecs](https://github.com/yohamta0/donburi-ecs) | 379 | NOASSERTION | 2026-05-17 | Just another ECS library for Go/Ebitengine |

### Zig (9)

| # | Engine | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| 1 | [zenith391/didot](https://github.com/zenith391/didot) | 90 | MIT | 2021-10-26 | Zig 3D game engine. |
| 2 | [JonSnowbd/slingworks](https://github.com/JonSnowbd/slingworks) | 26 | MIT | 2022-12-25 | Small to Medium scale 2d Game Engine for Zig |
| 3 | [zetaframe/zetaframe](https://github.com/zetaframe/zetaframe) | 13 | Apache-2.0 | 2020-09-05 | lightweight zig game framework. |
| 4 | [rzvxa/yume](https://github.com/rzvxa/yume) | 9 | MIT | 2025-12-09 | A batteries included game engine and editor written in Zig |
| 5 | [XFOSS/mfs](https://github.com/XFOSS/mfs) | 8 | Apache-2.0 | 2026-04-16 | Multi-Platform Game Engine Written in Zig |
| 6 | [weldengine/weld](https://github.com/weldengine/weld) | 2 | MIT | 2026-10-09 | Data-oriented game engine in pure Zig  runtime, editor, and scripting language. |
| 7 | [ironloom/loom](https://github.com/ironloom/loom) | 2 | Apache-2.0 | 2026-10-13 | loom is a declarative game engine, written in zig. |
| 8 | [justinGrosvenor/droids-engine](https://github.com/justinGrosvenor/droids-engine) | 1 | Zlib | 2026-07-18 | A Zig game engine built agent-first: stdin/stdout control protocol, headless rendering, |
| 9 | [PhilipLudington/AgentiteZ](https://github.com/PhilipLudington/AgentiteZ) | 1 | MIT | 2026-01-14 | A Zig based game engine built for and by AI |

### Lua (5)

| # | Engine | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| 1 | [Planimeter/game-engine-2d](https://github.com/Planimeter/game-engine-2d) | 752 | MIT | 2022-10-03 | Planimeter Game Engine 2D - LVE-based game engine for Lua |
| 2 | [NobleRobot/NobleEngine](https://github.com/NobleRobot/NobleEngine) | 509 | NOASSERTION | 2024-07-07 | A li'l game engine for Playdate. |
| 3 | [CapsAdmin/goluwa](https://github.com/CapsAdmin/goluwa) | 246 | GPL-3.0 | 2022-12-29 | game engine and framework written in luajit |
| 4 | [buildat-org/buildat](https://github.com/buildat-org/buildat) | 10 | NOASSERTION | 2026-10-21 | Buildat - A small engine and platform for networked 3D apps and games. Runtime-compiled |
| 5 | [itzKiwiSky/LitiumVM](https://github.com/itzKiwiSky/LitiumVM) | 9 | none | 2022-07-06 | A free and open-source fantasy console and game engine |

### Python (5)

| # | Engine | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| 1 | [SFTtech/openage](https://github.com/SFTtech/openage) | 14,531 | NOASSERTION | 2026-10-20 | Clone of the Age of Empires II engine |
| 2 | [pokepetter/ursina](https://github.com/pokepetter/ursina) | 2,599 | MIT | 2026-10-18 | A 3D and 2D game engine for Python |
| 3 | [ChenlizheMe/Infernux](https://github.com/ChenlizheMe/Infernux) | 1,983 | MIT | 2026-10-22 | Building a Neural Network-Native Engine (3N). Python authoring, C++ runtime, Vulkan/WebG |
| 4 | [efroemling/ballistica](https://github.com/efroemling/ballistica) | 712 | NOASSERTION | 2026-10-21 | The BombSquad Game Engine |
| 5 | [lordmauve/wasabi2d](https://github.com/lordmauve/wasabi2d) | 167 | LGPL-3.0 | 2026-09-06 | Cutting-edge 2D game framework for Python |

### Nim (2)

| # | Engine | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| 1 | [ftsf/nico](https://github.com/ftsf/nico) | 667 | MIT | 2024-07-09 | a Game Framework in Nim inspired by Pico-8. |
| 2 | [fragworks/frag](https://github.com/fragworks/frag) | 211 | MIT | 2017-06-07 | A cross-platform 2D/3D game framework for the Nim programming language |

### Odin (1)

| # | Engine | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| 1 | [jakubtomsu/ravn](https://github.com/jakubtomsu/ravn) | 104 | MIT | 2026-10-19 | [ALPHA] lightweight 2D and 3D engine for the joy of game development in Odin |

### D (2)

| # | Engine | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| 1 | [gecko0307/dagon](https://github.com/gecko0307/dagon) | 415 | NOASSERTION | 2026-10-17 | 3D game engine for D |
| 2 | [chances/teraflop-d](https://github.com/chances/teraflop-d) | 15 | BSD-3-Clause | 2024-05-17 | An ECS game engine on a WebGPU foundation |

### Haxe (1)

| # | Engine | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| 1 | [HaxeFlixel/flixel](https://github.com/HaxeFlixel/flixel) | 2,215 | MIT | 2026-10-15 | Free, cross-platform 2D game engine powered by Haxe and OpenFL |

### Kotlin (2)

| # | Engine | Stars | License | Last push | One-line scope |
|---|---|---|---|---|---|
| 1 | [AlmasB/FXGL](https://github.com/AlmasB/FXGL) | 4,868 | MIT | 2026-10-16 | Java / JavaFX / Kotlin Game Library (Engine) |
| 2 | [korlibs/korge](https://github.com/korlibs/korge) | 3,063 | NOASSERTION | 2026-09-19 | Korge is a Multiplatform Game Engine written in Kotlin for JVM, Web, Android and iOS. |

## 2. The 8 ranked steals

Value 1-5 (5 best). Effort: S under a day, M days, L a week or more. "Engine has it" is a grep result against `crates/`, with the path.

| # | Steal | Donor repo@rev, licence, stars, last push | What exactly to take | Lands in | Engine has it? | V / E |
|---|---|---|---|---|---|---|
| 1 | The digest is a list of labelled slices, and the divergence report names the slice | `balaurengine/balaur@c9fd9b2` `crates/balaur_core/src/digest.rs` (MIT, 96 stars, 2026-10-18) | `entries()` returns one `Entry { label, digest }` per state slice (per node: `/transform`, `/appearance`, `/tags`, per component), then `fold()` hashes label+digest in order. `first_divergence(a, b)` walks the two lists and names the first label that differs, including "the trees differ in shape" when the labels do not line up, and "missing on the right" when one side is longer. Node labels use the `StableId` when there is one, so a rename or reparent does not move the name, and fall back to a bounded path (`MAX_LABEL_DEPTH = 64`) so a hand-edited parent chain cannot name itself. A `TAG_LOCAL = "local"` tag keeps a node (a camera, a HUD) and its subtree out of the digest. | `crates/proof/src/slice_digest.rs` (new), `crates/arena/src/sync.rs` | Partly. `crates/arena/src/sync.rs:46` has `first_diverging_system` over five fixed subsystems (`world`, `space`, `combat`, `rng`, `ai`) and `SyncError` names the first tick and system. There is no per-entity or per-component slice, and no way to name the fighter that diverged. | 5 / M |
| 2 | A determinism oracle must prove it is not vacuous, in the same test file | `renew-engine/renew@91ba705` `crates/frame/tests/determinism.rs` (Apache-2.0, 28 stars, 2026-09-21) | The pattern, not the code: a determinism test that only asserts equality passes just as happily when the digest ignores its input. Renew states it in the file header and enforces it with `assert_not_vacuous(plans, stats)` called before *every* equality: at least one step ran, the step budget engaged, one frame was clamped, the remainder bank was non-empty, and the digest moved off its offset basis. Then the negative control: `a_one_nanosecond_perturbation_of_the_start_changes_the_digest` moves the anchor by 1 ns and asserts the digest differs. A second anti-vacuity test runs a fixed-point integer reference world beside the schedule digest, because a stable schedule can drive an unstable consumer. | `crates/proof/tests/` (new file), `crates/core/src/hash.rs` (doc rule) | No. `crates/proof` has golden and break-corpus tests (`crates/proof/src/a9.rs:346`, `crates/proof/tests/capture_compare.rs`) but no anti-vacuity helper and no one-nanosecond negative control. | 5 / S |
| 3 | The measured half and the gated half are different types | `renew-engine/renew@91ba705` `crates/frame/src/report.rs` (Apache-2.0, 28 stars, 2026-09-21) | `FrameStats` (deterministic: frames, ticks, steps dropped, schedule hash) and `FrameTiming` (measured: count, min, max, sum, drawn, skipped) are two types, so a wall-clock number can never leak into a digest. The digest is printed as a zero-padded hex *string*, not a JSON number, because a u64 exceeds the precision of a double-parsing reader and a silently rounded fingerprint is worse than none. Counters saturate. Presented-vs-skipped lives on the measured side so a dormant window cannot "run at 40,000 fps". | `crates/perf/src/profiler.rs`, `crates/proof/src/tape.rs` (JSON output) | Partly. `crates/perf/src/profiler.rs` has puffin scopes and `crates/arena/src/heft.rs:108` folds a hash, but there is no deterministic/measured type split and no hex-string rule for digests in JSON. | 4 / S |
| 4 | An EIR verifier that re-proves types without looking at the source | `esengine/estella@d2ec52e` `compiler/src/verify.ts` (Apache-2.0, 693 stars, 2026-10-21) | The verifier reads only the lowered IR, never the AST, on purpose: re-running the frontend's own reasoning would agree with the frontend's own bugs. It re-derives every service-question bit from the declared shape instead of trusting the frontend, checks arity and kind on every intrinsic and call, refuses a write through a read-only `Res` root ("`x` is written but the system declared Res, not ResMut"), refuses a `continue`/`break` outside a row loop, and checks that an event send carries exactly one argument per declared field in declaration order. | `crates/rules/src/verify.rs` (new), `crates/rules/src/rules.rs` | No. `crates/rules/src/rules.rs` runs Rhai inside the tick and returns `RulesError`, but there is no second pass that re-checks the script's effect against a declared shape. The Rhai host is trusted at runtime. | 5 / M |
| 5 | A line-based headless harness with a READY sentinel | `justinGrosvenor/droids-engine@326f35b` `src/harness.zig` (Zlib, 1 star, 2026-07-18) | One line of stdin is one command; after each response the harness writes a `READY` sentinel so the caller knows it may send the next one; closing stdin quits. The parser drains buffered lines before reading more (one `read` can deliver several newline-separated commands from `printf 'a\nb\n' | binary`), and copies the line into a second buffer so the parsed value stays valid after the next shift. A `Command` is a tagged union; an unknown line falls through to `line` and the game handles it, so the engine never has to know the game's verbs. The README's whole premise: the headless, scriptable path is the *primary* interface and the windowed path sits on top, so there is no separate automation API to keep in sync. | `crates/endpoint/src/harness.rs` (new), `crates/web/src/shell.rs` | Partly. `crates/endpoint` speaks MCP over stdio and `crates/web/src/shell.rs` prints one JSON line of facts, but there is no line protocol with a sentinel, and the browser path has no text console (gap C13). | 4 / M |
| 6 | A `check` verb that is the editor's Problems list, in CI | `balaurengine/balaur@c9fd9b2` `crates/balaur_cli/src/check.rs` (MIT, 96 stars, 2026-10-18) | One command prints every finding as `file:line:column: severity: message`, counts errors and warnings separately, exits 1 on any error, and prints `no problems` on a clean run. A project opts into strict mode from its own manifest (`[check] strict = true` in `project.toml`), so the flag is not only a CLI switch; the manifest decides. Warnings are counted even when not printed, so a quiet run still says there is something `--strict` would show. A manifest that does not parse reads as "not strict", because booting the project is about to say so with an error a person can act on. | `crates/endpoint/src/packs/verify.rs`, `tools/workspace-check.mjs` | Partly. `crates/endpoint/src/packs/verify.rs` and `tools/workspace-check.mjs` exist and `crates/endpoint/src/packs/check_profile.rs` is a profile pack, but no single command prints every finding with a file and a line and no manifest-driven strict mode. | 4 / S |
| 7 | Deterministic transcendentals for the audio path | `marmikshah/tono@ee4dd60` `crates/tono-core/src/det.rs` (MIT, 6 stars, 2026-10-21) | fdlibm kernels in pure f64 with pinned coefficients: Cody-Waite 2pi reduction in three parts, sin/cos polynomials on [-pi/4, pi/4], the Padé form of exp, the fdlibm log1p series, an exact `ldexp2` that builds the subnormal power of two from bits, and a cancellation-free `expm1` series for small arguments (tanh below 0.25 loses the whole digit otherwise). The file header states the reason: every kernel uses IEEE f64 with pinned coefficients, so platform libm differences cannot reach the output. | `crates/audio/src/det.rs` (new) | Partly, and differently. `crates/core/src/det_math.rs` has integer-only trig and exp (Q47.16, `det_sin`, `det_cos`, `det_exp`, `det_atan2za`) for the *sim*. The audio path needs float-rate accuracy for pitch and envelopes, which the integer kernel does not give, and the audio crate has no deterministic float kernel of its own. | 4 / M |
| 8 | The external I/O is suppressed while a recording plays back | `balaurengine/balaur@c9fd9b2` `crates/balaur_core/src/replay.rs` (MIT, 96 stars, 2026-10-18) | `suppressed(eng)` is true when a recording is playing *or* a rollback session is re-simulating. `ExternalIo::start` refuses to spawn outbound work while suppressed, so a subsystem cannot reach the real network by accident. `ExternalIo` wraps the three things every I/O subsystem forgets: the worker channel, this tick's arrivals, and their serialization; `capture()` and `restore()` push a recorded tick's arrivals back down the same channel so the subsystem's own dispatch runs unchanged. `ReplaySetupRegistry` is a second seam for state decided once before the first tick (bindings, locale, audio mix), because a replay derived from the replaying machine's settings is how a rebound key silently changes what a recording reproduces. `Origin` restores the tick, time, token and id counters so a script sees the numbers it saw. A file with no trailer ended in a crash, which is exactly the session worth having recorded. | `crates/proof/src/tape.rs`, `crates/net/src/rollback.rs` | Partly. `crates/proof/src/tape.rs` records and replays a bounded input tape and `crates/net/src/rollback.rs` re-simulates, but there is no "the outside world is off" rule and no separate once-per-session setup seam. | 5 / M |

## 3. Findings

### Finding 1: every engine in this class ships an editor, and the ones aimed at agents ship a *harness* instead

Of 126 catalogued engines, the large ones (Godot, Bevy, Fyrox, O3DE, Defold, Stride, Flax, Wicked, Babylon.js, PlayCanvas) all ship a human editor. The small ones aimed at agents do something different. droids-engine (`justinGrosvenor/droids-engine`) inverts the default: the headless, scriptable path is the primary interface and the windowed path sits on top, so there is no separate automation API to keep in sync with the real one. Balaur (`balaurengine/balaur`) ships both, and its `balaur check` command is the editor's Problems list runnable in CI. engine2040 has decided the door is the editor and there is no human editor until G7 is green (`docs/plan/finish-line.md:50`). That decision is right, but the class says the headless path must be *primary*, not a parallel copy. Evidence for the gap: `crates/endpoint` speaks MCP over stdio, and `crates/web/src/shell.rs` prints one JSON line of facts; there is no line protocol with a sentinel and no browser console (gap C13).

### Finding 2: a state hash that names *what* diverged is worth more than a hash that only says *that* it diverged

Balaur hashes the simulation in labelled slices, one per node and per component, and its `first_divergence` names the slice. engine2040's `crates/arena/src/sync.rs:46` names the *system* (five of them) and the tick, which is already better than a bare hash, but cannot name the fighter. `crates/arena/src/fork.rs:232` has a `first_divergence` over a fork trace. The class's shared answer: hash in slices, keep the labels stable across rename and reparent (Balaur uses a `StableId` for exactly this, and falls back to a bounded path), and let the report read "tick 4213, `n_ball_7/body2d`". Our own `crates/arena/src/sync.rs` header already says the byte layout "mirrors the authority sections so a diverging system points at the real field family", so the direction is set; the slice level is the next step.

### Finding 3: the honest engines prove their determinism oracle is not vacuous, in the same file

`renew-engine/renew` states the rule in the header of `crates/frame/tests/determinism.rs`: "A determinism test with no negative control passes just as happily when the digest ignores its input, and a loop that does nothing satisfies every equality assertion in this file." It then enforces it with `assert_not_vacuous` before every equality, a one-nanosecond negative control, and a second fixed-point reference world that checks the consumer, not just the schedule. It also documents a real defect it found: the timestep was excluded from the digest on a claim that turned out to be false, so two plans cut against different timesteps digested identically and would have drawn differently. A digest justified by a claim that happens to hold is "a digest waiting for the day it stops". Our tree has golden-hash and break-corpus gates (`crates/proof/src/a9.rs:346`, `crates/proof/tests/capture_compare.rs`) but no anti-vacuity helper and no perturbation control, so a golden that stopped reading the sim would stay green. This is cheap to add and it is the difference between a gate and a decoration.

### Finding 4: the small engines have a real asset and script *reload* story, and the class splits state by lifetime

Balaur reloads scripts "in milliseconds" (repo description) and separates state into two seams: per-tick external input (`ReplayRegistry`) and once-per-session state (`ReplaySetupRegistry`, for bindings, locale, audio mix). Renew splits the other way: `FrameStats` is the deterministic tally and `FrameTiming` is the measured one, so a wall-clock number can never enter a digest, and the digest is printed as a hex string because a u64 does not survive a double-parsing JSON reader. usagi (`brettchalupa/usagi`) ships Lua live reload; toxoid (`toxoidengine/toxoid`) ships a wasm guest that the host reloads on a socket message. engine2040 has no hot reload of the game crate (gap ENG12, deferred) and no once-per-session state seam in the tape. The split-by-lifetime idea is the cheap half and lands in `crates/proof/src/tape.rs` regardless of whether hot reload ever happens.

### Finding 5: none of these engines claims a bit-identical sim across native and web, and the class agrees determinism is a *comparison*, not a value

Renew calls itself "bit-deterministic" and its own evidence is a frozen digest recorded from its own implementation, with the comment that it "should hold on every platform; 'should' is not evidence, and the first three-platform run is". Balaur's digest is a comparison tool ("Determinism is a claim until two runs are compared"). Estella's verifier is the same instinct in the type domain: re-prove on the IR, not on the source. That is the same shape as our own rule "the same complete input fixture gives the same hash" (AGENTS.md rule 1). The gap in this class is not determinism; it is that almost none of them can compare a native run to a wasm run. Our G6/G10 matrix already does, which is the lead, and the class's contribution is the *method* of proving the oracle is not hollow (Finding 3).

## 4. What I could not verify

- **Not run.** I did not build or run any donor. Every claim about behaviour is from a README or a file body I read. Balaur's and Renew's "deterministic" claims are the donors' own.
- **Licences read from the API only**, not from the licence file text, except where the API gave NOASSERTION. `NOASSERTION` in the table means GitHub could not classify it, not that there is no licence.
- **Deep reads are 12 repos, not 126.** The other 114 rows are catalogue entries: stars, licence, last push and the repo's own one-line description. I did not open their source.
- **Only 9 of 126 repos were read in full**; the rest were read by tree listing to find a file, then one file was opened.
- **Search coverage is not exhaustive.** I used 24 distinct searches (one per language plus 4 topic searches plus `mcp game engine` and `deterministic game engine`). A `topic:gameengine` search and a `topic:ecs` search ran; I did not run a search per engine name, so an engine that does not tag itself `game-engine` may be missing.
- **Uncertain repos I did not use as donors:** `gfx-rs/gfx` and `gfx-rs/naga` are in maintenance mode (naga last pushed 2023-11-09, gfx 2023-02-27) and are listed for completeness, not as donors. `StarArawn/harmony` last pushed 2020-08-07 and `amethyst/amethyst` 2021-12-06; both are dead. `urho3d/urho3d` last pushed 2023-01-25; dead. `PistonDevelopers/piston` last pushed 2026-08-26 but its own `piston_window` and `turbine` crates moved on in October 2026, so the parent repo is behind its children.
- **`zenith391/didot` (Zig, 90 stars)**: the commit sha came back empty from the API, so I did not read a file from it. Not a donor.
- **Two repos I could not find at all** and therefore did not catalogue: `floooh/pixels` and `memononen/tinyling` return 404 (they are under other owners or names). `gfx-rs/glam` and `gfx-rs/wgpu-core` also 404. `cristianbg/arc` and `baconpaul1/pixelfl` 404.
- **Not checked:** whether any engine in the table has a determinism claim on its own web page. I only read the repo descriptions and the 12 files named above.

## 5. Next

Part 2 of this survey covers the AI-native and web engines (the other research folders for 2026-10-22 are `docs/research/engine-survey-2026-10-22/02-ai-web-determinism.md` and `docs/research/finish-line-2026-10-22/`). The 8 steals above are proposals, not rows; each names the crate it lands in and the shape of the test that would prove it.
