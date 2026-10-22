# Finish line vs 15 engines (research 2026-10-22)

Question (owner, 2026-10-22): is the finish line in `docs/plan/finish-line.md` (gaps A1-E23) worthy, compared with shipped and young engines on GitHub? What should we steal?

Method. Stars and last push come from the GitHub API read on 2026-10-22. Feature claims come from the project's own pages, release notes or repo listings, read with WebFetch or WebSearch on 2026-10-22. Donor file paths below were checked in the repo tree (a directory listing), not by reading file bodies, unless the row says "read". Anything I could not check says "unverified". Our side was checked with grep and `ls` against this tree. Licenses are not discussed (brief). Earlier research not repeated: `docs/research/steals-2026-10-11/engine-landscape.md`, `docs/research/steals-2026-10-18/06-new-engines.md`, `docs/research/finish-line-2026-10-17/02-github.md` (its 25 steals stay valid; ENG rows below are new unless marked "known").

## 1. Verdict

**Right bet, too big as written, and it leaves out three things a shipped engine always has.**

- **Right bet.** Of the engines whose pages I read, none claims a bit-identical simulation across native and web. Bevy's only determinism word is about schedule ordering. Godot's 4.6 page makes no determinism claim. Defold, Babylon.js, PlayCanvas, Stride, Flax, Wicked and Castle make none either. The AI door is only now appearing: Open-Godot-MCP (14 stars) and quench3d (0 stars) ship MCP tools for AI play and testing. quench3d is the nearest peer (Rust, wgpu, MCP, web, a fixed step), and it says itself that its golden digest is "a change alarm, not proof". Determinism plus wasm parity plus the AI door is a real niche. Keep it.
- **Too big as written.** 23 gap rows, several of them L (7, 11, 3) or owner decisions (23). Maturity is not on our side: Godot has 118,816 stars, Bevy 49,193, Babylon.js 26,203, and Bevy's own full editor is still "in progress" after 0.19 and 0.20. Competing on breadth is a losing race. Narrow to the must list below.
- **Missing a shipped-engine staple, two of them by omission:**
  1. **Update and patch delivery, crash reports.** Only gap 22 (ship check) exists. Defold ships Live Update out of the box (defold.com, read 2026-10-22). Godot 4.6 patch PCK files carry only the changed parts (search snippet, 2026). Nothing in our plan delivers a patched build or reports a crash.
  2. **Written non-goals.** A human editor (we have a headless dock shell, `crates/editor`; no daily-use editor) and mobile or console targets. Every shipped engine on this list has an editor. Bevy's is unfinished, so it is not fatal to skip one, but the finish line should say "the door is the editor, no human editor until G7 is green" instead of leaving it implied. Mobile and console are not in our bar (Windows, web, potato). Say so.
- **Undercounted.** The finish line does not mention several things we already have (section 3): input rebinding (`crates/input/src/bindings.rs`), a 1,722-line settings screen (`crates/ui/src/settings.rs`), a versioned save envelope with 16 slots and atomic write (`crates/save/src/lib.rs`), menus and a lobby UI (`crates/ui/src/menus.rs`, `lobby_ui.rs`), a static en/de locale with integer plurals (`crates/ui/src/locale.rs`), an accessibility pass with captions (`crates/ui/src/access.rs`), a puffin profiler with GPU timestamp scaffolding (`crates/perf/src/profiler.rs`), and an optional cpal audio device (`crates/audio`, feature `device`). A reader of the finish line would think these are missing. Add one row that lists them as done, with their tests.

**Cut to the must list (recommendation, owner decides).** Must for the finish line: A1, A2, A4, A5, B7, B8, B9, C10, C13, D16, D17, D19, E21, E22. Defer until G7 and G8 are green: A3 (browser multiplayer, L), C11 (rules layer, L), C14 (quaternion lanes), C15 (box3d spike), E23 (netcode decision). Add three rows: "ship delivery" (ENG9 below), "crash and receipt on every run" (the `physfx:`-style receipt for crashes), "non-goals written down" (no human editor, no mobile).

## 2. Comparison table

Stars and last push: GitHub API, 2026-10-22. "Read" means the project's own page, release notes or repo listing was read; "search" means only a search snippet. Our column is the finish line plus this tree.

| Engine (stack) | Stars, last push | Maturity (what I read) | Ships that our finish line lacks | We have that it lacks | Verdict for us |
|---|---|---|---|---|---|
| **Bevy** (Rust, ECS, wgpu) | 49,193, 2026-10-21 | 0.19 on 2026-06-19 and 0.20 on 2026-10-08 (read). Full editor "in progress" (read). | BSN scene syntax; transform gizmo and infinite grid; render error handler (device loss: recover, stop, ignore); settings groups as resources with save and load; `EditableText`; a `Ready` event when children spawn (0.20); asset processor with mip generation (0.20) | Bit-exact sim across targets; a golden hash; an AI door with a same-result gate. Bevy's shuffle is seeded for ordering only ("deterministically, but arbitrarily", read). | Peer for ideas (error policy, ready event, settings). Not a base. |
| **Godot 4.6** (C++, GDScript/C#) | 118,816, 2026-10-21 | 4.6 stable in 2026 (read). 4.7 in beta per search. | Full editor (scene, animation, shader, tilemap); Jolt default physics (read); CSV translation with context and plural columns and template generation (read); ObjectDB snapshot compare (read); Tracy, Perfetto and Instruments hooks (read); patch PCK (search); multiplayer API; many export targets | No determinism claim on the 4.6 page (read). Web is WebGL2 Compatibility per secondary guides; a WebGPU claim for 4.7 conflicts across sources (unverified). | Largest reference for editor and asset pipeline. Do not copy the stack. |
| **Fyrox** (Rust) | 9,608, 2026-10-21 | 1.0.0 on 2026-03-24 per official blog (search). | FyroxEd scene editor; hot reload of game code as a dylib (0.34, search); export CLI for PC, WASM and Android (1.0, search); task-based scene loading | No determinism claim found. Its 1.0 notes are "consolidation". | Good donor for build/export and hot reload. Less mature than Godot or Bevy in editor depth. |
| **O3DE** (C++) | 9,766, 2026-10-21 | 26.05 stable on 2026-05-27 (Wikipedia, search). Experimental Open Particle System; auto LOD for FBX (search). | Full editor; asset processor; Gems (modular features); a big AAA renderer | Home page states no platforms, editor, networking, audio, UI, localization or determinism (read: "not stated"). | Too heavy. Asset processor idea only. |
| **Stride** (C#/.NET) | 7,901, 2026-10-21 | 4.3 in Nov 2025 (.NET 10, C# 14, Bepu physics option, Vulkan compute, read on stride3d.net). 4.4: Vulkan and D3D12 rewrite (read). | Editor and asset toolchain (read) | Web, profiler, UI, audio, networking, localization: "not stated" (read). | Skip for us: .NET runtime, float physics. |
| **Flax Engine** (C++/C#) | 7,073, 2026-10-21 | 1.10 and 1.11 exist per search; site page read. | Editor; C# and visual scripting (read); networking optimizations, bloom and new vertex format (1.10, search) | Web, profiler, localization and determinism: "unclear" (read). | Skip for us. |
| **Wicked Engine** (C++) | 7,305, 2026-10-21 | Active. Standalone "lightweight editor" and Lua scripting (read). Path tracing and SSGI (80.lv, search). | Standalone editor; Lua; path tracer | Web/WebGPU, profiler, physics and determinism: unclear (read). | Skip for us. |
| **Defold** (C++, Lua) | 6,407, 2026-10-21 | 1.12 series (late_update, engine throttling, search); 1.13.0 on 2026-06-29 (Wikipedia, search). Commercial use. | HTML5 target (read); visual, scene, tilemap and ParticleFX editors (read); profiler and hot reload (read); Live Update (read); `late_update` phase order (search) | No determinism claim (read). No AI door. | Donor for live update and frame phases. |
| **macroquad** (Rust, 2D) | 4,675, 2026-08-18 | 0.4.16 latest seen (docs.rs, search). | Dependency-light web build; immediate-mode UI; Android and wasm (README, search) | 3D, determinism, AI door, scale. Not a peer for a 3D duel. | Donor for the no-bindgen web shim (ENG2). |
| **Panda3D** (C++/Python) | 5,256, 2026-07-28 | 1.10.16 latest seen; no 1.11 release found (search). Site fetch failed (HTTP 500), so features are unverified. | Python-first scripting; mature 3D stack (unverified here) | Determinism, AI door, wasm parity. | No donor. Not read in depth. |
| **Castle Game Engine** (Pascal) | 1,259, 2026-10-20 | 7.0 alpha 3 (forum, undated); mirror lists 6.4 stable (2024). | Editor (read); UI with anchors and auto scaling (read); web, Android, iOS, Switch (read) | Networking, audio, localization, profiler, determinism: "not stated" (read). | Small donor for UI anchors only. |
| **Babylon.js** (TypeScript, web) | 26,203, 2026-10-21 | Active. Inspector v2 overlay and a desktop Babylon Editor (read). | Web-first runtime tooling; the inspector overlay (ENG3) | Native targets; determinism (none claimed on the page read). WebGPU is "partly stated" (read). | Donor for the runtime inspector overlay. |
| **PlayCanvas** (JS, web) | 17,101, 2026-10-21 | Active. "One of the first production 3D engines with full WebGPU support", and keeps WebGL2 (read). | WebGPU and WebGL2 backends side by side (repo listing: `src/platform/graphics/webgl/`, `webgpu/`); GPU profiler file (`gpu-profiler.js`); cloud collaborative editor; WebXR | Native targets; determinism; AI door (none stated). | Donor for the web backend split (ENG1) and GPU profiler (ENG7). |
| **quench3d** (Rust, wgpu) | 0, 2026-10-22 | 162 commits, self-reported tests, no community yet (README, read). Semi-implicit Euler; "about 1.6% position error in one second of free fall" (README, read). | MCP server with 9 tools (`doctor`, `validate`, `schema`, `render`, `movie`, `simulate`, `query`, `ask`) (README, read); WebGPU first with a WebGL2 fallback chosen by a probe (README, read); an egui editor shared by desktop and web (README, read) | Integer determinism; a hashed golden tape proved on three targets. Its own README says long contact runs "can't be bit-locked across refactors". | The closest peer in stack and door design. We lead on determinism and wasm parity. They lead on fallback, doctor and web editor. |
| **Open-Godot-MCP** (Python, Godot addon) | 14, 2026-07-23 | Early. MIT. Author post (read). | Freeze clock and step time; input injection; multiplayer host and client with latency injection; DAP debugger; state digests with diff-only returns (author: "about 90% fewer tokens") (read) | It is a Godot tool, not an engine: no determinism guarantee; no wasm parity. | Donor for door verbs (ENG5, ENG6). |
| **ludenea** | repo empty on 2026-10-22 (read) | Dead for now. | Nothing. | n/a | Dropped. |

## 3. Shipped-engine staples: what we have

| Staple | Shipped by (read unless marked) | What we have (file, checked 2026-10-22) | Finish-line status | Call |
|---|---|---|---|---|
| Editor (human) | Godot, Stride, Flax, Fyrox, Defold, Castle, Babylon, PlayCanvas, O3DE, Wicked. Bevy still "in progress". | `crates/editor/src/app.rs`: headless-first dock shell; edits go through the door (`crates/endpoint`) | Not named | Write down as a non-goal until G7 is green. Steal ENG13 for a read-only web view. |
| Asset pipeline | O3DE asset processor; Bevy 0.20 processor with mips; Godot import; Defold build | `crates/assets` cook, blob hashes (commit `40ae991ab`: 19 blobs verify) | Gap A2 (load queue) | Gap A2 stays; add ENG9 (patch). |
| Profiler | Defold, Godot (Tracy, Perfetto), PlayCanvas GPU profiler | `crates/perf/src/profiler.rs` (puffin scopes, GPU timestamp scaffolding); no trace file | Gap B13 | Keep B13; steal ENG7 for the trace file. |
| Input remapping | Godot InputMap; Bevy input | `crates/input/src/bindings.rs` (rebindable table, deadzone) | Not named | Count as done; test it. |
| Save and load | Godot resources; Bevy 0.19 settings groups | `crates/save/src/lib.rs` (versioned envelope, 16 slots, atomic write); `crates/ui/src/settings.rs` | Not named | Count as done; steal ENG14 for settings files. |
| Localization | Godot (CSV, plurals, templates, read); Defold (editor only, read) | `crates/ui/src/locale.rs`: static en and de, integer plurals, fallback to key (no panic) | Not named | Enough for a duel. Steal ENG10 when a second language is wanted. |
| UI | Godot Control, Castle anchors, Bevy Feathers, Babylon GUI | `crates/ui` (`menus.rs`, `lobby_ui.rs`, `settings.rs`, `access.rs`, `tutorial.rs`, `play_hud.rs`) | Not named (only HUD data lane in CRATE-MAP) | Count as done; the finish line should name it. |
| Audio | Godot buses; Stride; O3DE | `crates/audio` (offline synth, headless mixer, cpal behind feature `device`, `hrtf.rs`, `reverb.rs`); web audio is a ScriptProcessorNode (finish line A5) | Gap A5 (web AudioWorklet) | Keep A5. Count the rest as done. |
| Networking | Godot multiplayer API; Open-Godot-MCP latency injection; O3DE (unverified) | `crates/net` (rollback, UDP and QUIC, dedicated server latency reports); `crates/proof/src/chaos.rs` (fault list) | Gaps A3, G8, E23 | Keep deferred; steal ENG6 for latency and loss as a door verb. |
| Patch and update | Defold Live Update (read); Godot patch PCK (search) | Nothing in tree | Not named | Add ENG9. |
| Crash and receipts | Bevy render error handler (read, 0.19) | `crates/game/src/crash.rs` (exists); device loss is handled by hand in `crates/render/src/present/gpu_present/presenter.rs` and `crates/render/src/heal.rs` | Not named | Add one row: every crash writes a receipt in the `physfx:` style. |
| Mobile and console | Godot, Defold, Castle, Stride (read) | None | Not in bar | Write down as a non-goal. |

## 4. Ranked steals (ENG1-ENG15, value per effort for this engine)

Rank order: value (1-5, 5 best) and effort (S under 1 day, M days, L a week or more). Row ids follow the board style (`ID area: what becomes true. Accept: the proof.`). Each row is a proposal for `docs/FOCUS.md`; this report does not edit that file.

### ENG5. Doctor verb on the door (rank 1, value 4, S)
- **Donor.** `src/open_godot_mcp/tools/health.py` in masteryee-labs/Open-Godot-MCP (listed; not read). quench3d's `doctor` MCP tool (named in its README; `crates/quench3d-cli/src/mcp.rs` listed).
- **What it does.** One call that says what the machine and build can do: adapter, backend, limits, WebGL2 or WebGPU path, golden digest, free disk.
- **Ours.** No `doctor` or `health` verb in `crates/endpoint/src` (grep, 2026-10-22). `crates/endpoint/src/packs/perf.rs` is the place.
- **Lands in.** `crates/endpoint/src/packs/perf.rs` (new `doctor` verb).
- **Row.** `ENG5 door doctor: one call returns adapter, backend, limits, WebGPU or WebGL2 path, golden tape digest and free disk as JSON. Accept: a test parses the JSON on Windows and in headless Chrome.`
- **Gap.** D16, D19. The weak free models need it most: one call instead of five.

### ENG1. Web backend chosen by a probe before the first frame (rank 2, value 5, M)
- **Donor.** quench3d `crates/quench3d/src/web_probe.rs` (listed; not read). README: "tries WebGPU first and falls back to WebGL2, chosen by a probe before startup". PlayCanvas keeps WebGL2 beside WebGPU (repo listing `src/platform/graphics/webgl/` and `webgpu/`).
- **Ours.** The web target is WebGPU only (CRATE-MAP: `engine-web` "a WebGPU device"). `crates/web/src/limits.rs:204` already has a test named `webgl2_class_limits_would_fail_the_renderer_need`, so a WebGL2 browser would fail today.
- **Lands in.** `crates/web/src/shell.rs` (adapter pick), `crates/render/src/backend.rs` (the existing webgl mention).
- **Row.** `ENG1 web fallback: the page picks WebGPU or WebGL2 before the first frame and prints the choice in its JSON line. Accept: headless Chrome with WebGPU off boots the arena and its golden tape digest equals the WebGPU run.`
- **Caveat.** wgpu's GL backend on wasm as WebGL2 is unverified here. If the arena cannot run at WebGL2 limits, the row must say a lower tier, not a silent fail.
- **Gap.** G10, A1, G9 (potato).

### ENG7. Trace file for CPU spans and GPU passes (rank 3, value 4, S-M)
- **Donor.** Godot 4.6 external profiler hooks: Tracy, Perfetto, Instruments (read). `tracing-chrome` crate, Chrome trace JSON that opens in ui.perfetto.dev (crates.io listing 0.7.2, 2024, read via search; its repo not located, unverified). PlayCanvas `src/platform/graphics/gpu-profiler.js` (listed; not read).
- **Ours.** `crates/perf/src/profiler.rs` has puffin scopes and GPU timestamp scaffolding and writes no trace file.
- **Lands in.** `crates/perf/src/profiler.rs` (writer), `crates/endpoint/src/packs/perf.rs` (verb).
- **Row.** `ENG7 trace file: perf.trace writes one JSON with CPU spans for the eight tick phases and named GPU pass timers. Accept: the file parses and ui.perfetto.dev shows the eight phase names.`
- **Gap.** B13.

### ENG6. Latency and loss as a door verb (rank 4, value 4, S)
- **Donor.** `src/open_godot_mcp/tools/network.py` in Open-Godot-MCP (listed; not read). Its README and author post claim host and client instances with latency injection (read via forum post).
- **Ours.** `crates/proof/src/chaos.rs` has a fault list (healed attempts, loud reasons). `crates/net/src/dedicated.rs` reports latency only. No verb sets latency or loss on a live duel.
- **Lands in.** `crates/endpoint/src/packs/net.rs` (listed) and `crates/proof/src/chaos.rs`.
- **Row.** `ENG6 net shape: net.shape sets latency and loss on the loopback duel. Accept: 150 ms and 2% loss end on the same digest as 0 ms.`
- **Gap.** G8 (multiplayer), gap 23 (netcode). Rollback should make the digest equal; if it does not, that is the finding.

### ENG11. One export command per target (rank 5, value 4, M)
- **Donor.** Fyrox 1.0 export CLI for PC, WebAssembly and Android (search, 2026) and the `fyrox-build-tools` crate directory (listed).
- **Ours.** `crates/endpoint/src/build_game.rs` exists (grep). Gap 12 says `engine2040 new / build / run` is open.
- **Lands in.** `crates/endpoint/src/build_game.rs`.
- **Row.** `ENG11 one export: engine2040 build --target windows|web writes a folder with a manifest of golden digests. Accept: two runs give byte-equal manifests, and the web digest equals the native golden.`
- **Gap.** C12.

### ENG8. Late phase after the fixed ticks (rank 6, value 4, S)
- **Donor.** Defold 1.12 frame order: fixed updates run first as many times as needed, `update()` once per frame, `late_update()` after update and before rendering (read via Defold's H1 2026 post, search snippet).
- **Ours.** `crates/app/src/lib.rs` documents the phase order ("registration order inside fixed phases"). No late phase is named.
- **Lands in.** `crates/app/src/lib.rs` (phase list), `crates/ecs` (phases).
- **Row.** `ENG8 late phase: camera follow and foot IK read the post-tick pose in a late phase before draw. Accept: a capture test shows zero-lag follow at 60 and 144 Hz, and the sim hash does not change.`
- **Gap.** LF9, LF13, LF10 (the phase order).

### ENG3. Runtime inspector overlay (rank 7, value 4, M)
- **Donor.** Babylon.js `packages/dev/inspector-v2/` (listed; the Inspector v2 overlay read on babylonjs.com).
- **Ours.** No overlay. The debug kit (gap 13) is open.
- **Lands in.** `crates/editor` (panel model, headless), `crates/web/src/shell.rs` (overlay in the page).
- **Row.** `ENG3 inspector: an overlay lists the eight fighters (hp, stance, tick, hash) from the last digest. Accept: the overlay tick and hash equal state.digest in a capture.`
- **Gap.** C13.

### ENG9. Content patch by blob hash (rank 8, value 4, M)
- **Donor.** Defold Live Update (read: "out of the box"); Godot 4.6 patch PCK ships only changed parts (search snippet).
- **Ours.** `crates/assets` has cooked blobs with hashes (commit `40ae991ab`: 19 blobs verify). No patch path.
- **Lands in.** `crates/assets` (manifest diff), `tools/web-build.mjs` (bundle).
- **Row.** `ENG9 content patch: a rebuild ships only changed cooked blobs, by hash. Accept: a one-texture change ships under 10% of the bundle, and the golden tape digest is unchanged.`
- **Gap.** A2, E22 (ship check), new "delivery" row.

### ENG4. Page console for read-only door verbs (rank 9, value 3, S-M)
- **Donor.** quench3d `crates/quench3d/src/web_console.rs` (listed; not read).
- **Ours.** The web page has a HUD and a JSON line; no console.
- **Lands in.** `crates/web/src/shell.rs`, `crates/endpoint/src/packs/session.rs` (read-only verbs only).
- **Row.** `ENG4 page console: a line box in the browser runs read-only verbs (game.ticks, state.digest). Accept: game.ticks in the box equals the tick on the HUD, and a mutating verb is refused in the page.`
- **Gap.** C13, G7.

### ENG2. No-bindgen web loader (rank 10, value 3, S)
- **Donor.** miniquad `js/gl.js` (67,252 bytes, repo listing; the JS side that lets a plain wasm module call WebGL without wasm-bindgen).
- **Ours.** `tools/web-build.mjs` and `crates/web/src/browser.rs` (listed). Bundle today: 2.55 MB, 434 KB brotli (finish line G10).
- **Lands in.** `tools/web-build.mjs`, `crates/web/src/browser.rs`.
- **Row.** `ENG2 web loader: the page starts the wasm with one small JS shim, no bindgen glue. Accept: the Pages URL boots, and the shim plus page is under 100 KB gzipped (web-build prints it).`
- **Gap.** A2, G10.

### ENG13. Read-only editor on the web (rank 11, value 3, M)
- **Donor.** quench3d `crates/quench3d-editor/` (listed): an egui editor shared by desktop and web (README).
- **Ours.** `crates/editor/src/app.rs` (desktop dock shell, headless-first). No web build of it.
- **Lands in.** `crates/editor`, `crates/web/src/shell.rs`.
- **Row.** `ENG13 web editor view: the same dock opens read-only in the browser build. Accept: a browser capture shows the scene list and fighter table with the counts the window shows.`
- **Gap.** new (human view of the web build); D17.

### ENG10. Translation CSV with plurals (rank 12, value 2, S)
- **Donor.** Godot 4.6 CSV translation: context and plural columns, and template generation (read on godotengine.org/releases/4.6).
- **Ours.** `crates/ui/src/locale.rs`: static en and de, integer plurals, fallback to the key.
- **Lands in.** `crates/ui/src/locale.rs` (table from csv).
- **Row.** `ENG10 translation csv: one csv with key, context, en, de and plural columns builds the locale table. Accept: a test fails on a key with no en value, and a missing key prints the key, never panics.`
- **Gap.** new (localization beyond static tables). Do this only when a second language is wanted.

### ENG14. Settings file on the save envelope (rank 13, value 3, S)
- **Donor.** Bevy 0.19 app settings framework: settings groups as resources, loaded and saved (read on bevy.org/news/bevy-0-19).
- **Ours.** `crates/save/src/lib.rs` (versioned envelope, atomic write, 16 slots); `crates/ui/src/settings.rs` (1,722 lines).
- **Lands in.** `crates/ui/src/settings.rs` through `crates/save/src/lib.rs`.
- **Row.** `ENG14 settings file: settings save with the save envelope and atomic write. Accept: change a key, restart, the value persists; a corrupt file falls back to defaults and emits one observe event.`
- **Gap.** new (settings persistence), mostly done.

### ENG15. Ready event for spawned scenes (rank 14, value 2, S)
- **Donor.** Bevy 0.20: a scene entity triggers `Ready` once its children spawn (read on bevy.org/news/bevy-0-20).
- **Ours.** `crates/scene` has persistent identity (CRATE-MAP), no ready signal.
- **Lands in.** `crates/scene`, `crates/app`.
- **Row.** `ENG15 ready event: an entity emits Ready once its children exist. Accept: a duel scene emits one Ready per fighter on the same tick on every target.`
- **Gap.** D17 (door reads). Helps the AI wait for a state without polling.

### ENG12. Native hot reload of the game crate (rank 15, value 3, L, risky)
- **Donor.** Fyrox code hot reload as a dylib since 0.34 (search); `fyrox-dylib` crate (listed).
- **Ours.** Rhai hooks (gap 11, planned) reload as data; the Rust game crate does not.
- **Lands in.** `crates/game` (feature `hot`), `crates/app` (swap at a tick boundary).
- **Row.** `ENG12 hot reload: the game dylib reloads at a tick boundary. Accept: a no-op edit reloads and the tape digest is unchanged; a changed rule changes the digest in one replay.`
- **Caveat.** Unsafe code and determinism risk. Defer until D16 and D17 are green. Owner decision.

## 5. What NOT to copy and why

- **C# or .NET runtime (Stride, Flax, Godot C#).** Adds a runtime on top of wasm. Godot's C# projects cannot export to the web (secondary guides, unverified). Our rules live in Rhai and Rust.
- **Float physics as the sim (Stride Bepu, quench3d semi-implicit Euler, Jolt default in Godot 4.6).** Not hashed across targets. Our physics stays in the integer sim; float physics stays in physfx (never hashed), as the finish line already says.
- **Full editor race (Godot, Bevy, Fyrox, O3DE).** Bevy's own editor is unfinished after two releases. A second editor would eat the team for a year. The door is the editor.
- **Full Fluent localization now.** `locale.rs` already ports the key-lookup subset. Import more only when a second language is real.
- **Mobile and console SDK plumbing (Defold, Godot, Castle, Stride).** Not in our bar. Writing it down is the work; building it is not.
- **Python or Lua scripting (Panda3D, Defold, Wicked, Open-Godot-MCP's language).** Two runtimes on wasm for no gain. Rhai is already the rule language.
- **Babylon.js and PlayCanvas as the web engine.** They are good references for the web backend and inspector (ENG1, ENG3), not a second engine in the browser. Our wasm build is the product.
- **Quench3d physics as a reference for accuracy.** Its own README says semi-implicit Euler drifts about 1.6% in one second of free fall, and its golden digest is "a change alarm". Use it for the MCP and fallback shape, not the solver.
- **Open-Godot-MCP's screenshot judging ("free vision endpoints").** Our look judge is advisory (finish line G4, LF15). Do not make a vision model a gate.
- **O3DE's Gems system and asset processor as shape.** Too heavy a build for the solo loop. The asset ENG9 row covers the one part we need.
- **Ludenea.** Repo empty on 2026-10-22. Nothing to take.

## 6. Sources and unverified items

Sources read on 2026-10-22 (WebFetch or WebSearch):
- GitHub API for every repo above (stars, last push, description).
- https://bevy.org/news/bevy-0-19/ and https://bevy.org/news/bevy-0-20/ (Bevy 0.19 on 2026-06-19, 0.20 on 2026-10-08).
- https://godotengine.org/releases/4.6/ (Jolt default, CSV translation, tracing hooks; no determinism, networking or web claim on the page).
- https://defold.com/ (HTML5 target, profiler, editors, Live Update).
- https://www.babylonjs.com/ (Inspector v2, editor) and https://playcanvas.com/ (WebGPU with WebGL2).
- https://stride3d.net/ (4.4 Vulkan and D3D12 rewrite; web not stated).
- https://castle-engine.io/ (web, Android, iOS, Switch; editor; UI).
- https://flaxengine.com/ and https://www.wickedengine.net/ (partial; many items unclear).
- https://o3de.org/ (platforms and features not stated).
- Open-Godot-MCP author post: https://forum.godotengine.org/t/open-godot-mcp-ai-plays-tests-your-game-not-just-edits-files/141986 (repo https://github.com/masteryee-labs/Open-Godot-MCP).
- quench3d README at https://github.com/gh503/quench3d (read through WebFetch).
- Repo directory listings (GitHub API `contents`): miniquad `js/`, Fyrox root and `fyrox-build-tools/`, PlayCanvas `src/platform/graphics/`, Babylon `packages/dev/`, quench3d `crates/`, `crates/quench3d/src/`, `crates/quench3d-cli/src/`, `web/`, Open-Godot-MCP `src/`, `src/open_godot_mcp/tools/`.

Unverified or conflicting, stated plainly:
- Panda3D features (site fetch failed with HTTP 500). Panda3D 1.11 not found.
- O3DE networking, audio, UI, localization, platforms (page did not state them; the docs were not opened).
- Godot 4.7 WebGPU web export: sources conflict.
- Fyrox 1.0 date (official blog 2026-03-24 per search; another article gives a different month).
- Flax 1.10 and 1.11 contents (search only).
- Wicked web/WebGPU and profiler (page does not say).
- tracing-chrome repo location (crate listing read, repo not found).
- Donor file bodies were not read for: `web_probe.rs`, `web_console.rs`, `health.py`, `network.py`, `gpu-profiler.js`, `js/gl.js`, `inspector-v2`. Each row's "read" status is "listed" for those. Open the file before porting, per the brief.
- Stars and push dates are as read through the API on 2026-10-22. They may be a day stale.
