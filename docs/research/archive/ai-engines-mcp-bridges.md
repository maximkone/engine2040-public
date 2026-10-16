# AI-controllable engines: MCP bridges survey (RES-AIENG-1)

Date read: 2026-10-01. Stars via `gh repo view --json stargazerCount,licenseInfo`
(licenseInfo reads the repo LICENSE file); tool details via raw README reads
same day (DOCS-CLAIM = vendor claim, not our run). No 403/429 hit on any host.
Prior rows not repeated: see `COMPARE.md` AI-door surveys (e-blendermcp,
e-openprompts, e-3dsearch) and `LADDER.md` AI-DOOR lines.
Ours: engine2040 MCP door = exactly 20 tools (`crates/endpoint/src/mcp.rs:129
`tool_names``): scene.read/brief/spatial/place/query, tx.preview/apply/undo,
sim.plan_goals, play.run/step, capture, log.digest/query, api.search/describe,
code.callers, asset.refs, rpc.discover, scene.watch. Single-use
revision-bound approvals, atomic apply, undo-as-new-revision, 38944-char
discovery budget w/ ledger gate, no shell/exec/file-write tool
(`docs/EDITOR_AI_CONTROL.md`).

Legend: R read-only inspect | Tx atomic revision-checked apply | U undo |
Rp deterministic replay/record | A approvals/consent/sandbox | S screenshots |
P playtest/headless loop. + yes, - no, ~ partial/weak, ? unverified.

## Godot bridges

| Repo | Stars | SPDX | Tools | R Tx U Rp A S P | Trick worth stealing | Weakness we beat |
|---|---|---|---|---|---|---|
| [Coding-Solo/godot-mcp](https://github.com/Coding-Solo/godot-mcp) | ~5.9k | MIT | ~14 (prior count, `COMPARE.md:70`) launch/run/logs | + - - - ~ + + | Minimal launch+log loop, cheapest viable door | No tx/undo/replay/approvals |
| [tugcantopaloglu/godot-mcp](https://github.com/tugcantopaloglu/godot-mcp) | ~470 | MIT | 157 DOCS-CLAIM | + - - - - + + | Headless GDScript op runner (`--headless --script`), validate_script w/o running game | 157-tool bloat vs our 20-tool budget ledger; arbitrary exec, no approvals |
| [ee0pdt/Godot-MCP](https://github.com/ee0pdt/Godot-MCP) | tiny | MIT (README claim) | ~21 cmds + 3 MCP resources | + - - - - - ~ | MCP *resources* (`godot://script|scene|project`) for live context | No screenshots/undo/replay/tx |
| [satelliteoflove/godot-mcp](https://github.com/satelliteoflove/godot-mcp) | ~170 | MIT | 21 tools / 86 actions DOCS-CLAIM | + - - ~ - + + | Step-*until-condition* + cheap structured entity JSON instead of screenshots | Editor edits unguarded; no tx/approvals/undo |
| [wgt19861219/godot-mcp-enhanced](https://github.com/wgt19861219/godot-mcp-enhanced) | ~100 | Other | 46 tools / 271 actions DOCS-CLAIM | + - ~ + + + + | Verification pipeline (`run_and_verify`, `qa`, `verify_delivery`) + `playtest.seed` RNG lock + snapshot/restore | Vision screenshots exfiltrated to external groq API (self-disclosed); no atomic cross-layer tx |
| [NPGameDev/godot-mcp-toolkit](https://github.com/NPGameDev/godot-mcp-toolkit) | ~50 | Other | 112 tools / 150+ ops DOCS-CLAIM | + - ~ - ~ + + | GDScript extension API (projects add own tools) + Editor/Runtime dual channel | No atomic tx/replay; concurrent-edit safety scenario-limited |
| [youichi-uda/godot-mcp-pro](https://github.com/youichi-uda/godot-mcp-pro) | ~620 | Other (paid $15) | ~162-187 DOCS-CLAIM | + - + ~ - + + | Per-task surface profiles (`--3d` trims tool set); input record/replay; screenshot compare | Closed/paid; input-only replay; no tx/approvals |
| [IvanMurzak/Godot-MCP](https://github.com/IvanMurzak/Godot-MCP) | ~260 | Apache-2.0 | skills + tools (idempotent cli) | + - - - ~ + ~ | Auto-generated per-project skills + idempotent setup CLI | Cloud default (ai-game.dev) vs our offline-first; no tx/undo/replay |

## Unity bridges

| Repo | Stars | SPDX | Tools | R Tx U Rp A S P | Trick worth stealing | Weakness we beat |
|---|---|---|---|---|---|---|
| [CoplayDev/unity-mcp](https://github.com/CoplayDev/unity-mcp) | ~14.6k | MIT | dozens (? exact, main README thin) | + - - - - - - | First-mover simplicity; Aura-maintained | No tx/undo/screenshots/approvals on main |
| [IvanMurzak/Unity-MCP](https://github.com/IvanMurzak/Unity-MCP) | ~4.4k | Apache-2.0 | 70+ built-in + 10 extension packs | + - - - - + + | Reflection turns any C# method into a tool in one line + JSON-schema-via-reflection | `script-execute` = arbitrary Roslyn code; no tx/undo/replay |
| [FunplayAI/funplay-unity-mcp](https://github.com/FunplayAI/funplay-unity-mcp) | ~260 | MIT | 180 (core 40) DOCS-CLAIM | + - ~ - - + + | `IFunplayCommand` auto-Undo template + port pinning across domain reload | Explicitly *no* approval toggle; no atomic multi-op tx |
| [AnkleBreaker-Studio/unity-mcp-server](https://github.com/AnkleBreaker-Studio/unity-mcp-server) | ~490 | Other | 268-330 DOCS-CLAIM | + - ~ - - + + | Per-agent last-action undo (`unity_undo_last` + agentId) + compilation-error retrieval loop | Single-step undo only; no batch atomicity/replay |
| [meta-quest/Unity-MCP-Extensions](https://github.com/meta-quest/Unity-MCP-Extensions) | small | UNVERIFIED | XR extension pack | ? - - - - - - | Vendor extension-pack pattern on a base door | Inherits base-door safety gaps |
| [swax/UnityMCP-VRC](https://github.com/swax/UnityMCP-VRC) | small | UNVERIFIED | ~dozens (?) | + - - - - + ~ | Instance discovery records surviving reload | VRChat-niche; no tx/undo/replay |

## Unreal bridges

| Repo | Stars | SPDX | Tools | R Tx U Rp A S P | Trick worth stealing | Weakness we beat |
|---|---|---|---|---|---|---|
| [chongdashu/unreal-mcp](https://github.com/chongdashu/unreal-mcp) | ~2.1k | MIT (README badge; gh null) | dozens (?; actor/Blueprint/graph/editor) | + - - - - - - | FastMCP tool-module autoload dir; C++ TCP plugin :55557 | Experimental, zero safety surface |
| [kvick-games/UnrealMCP](https://github.com/kvick-games/UnrealMCP) | ~610 | MIT (full text in README) | ~5 (scene CRUD + execute_python) | + - - - - - - | None yet (ahujasid-inspired WIP) | WIP; localhost-open server; no safety |
| [runreal/unreal-mcp](https://github.com/runreal/unreal-mcp) | ~120 | MIT | 20 (counted table) | + - - - ~ + - | Zero-install via built-in Python Remote Execution | `editor_run_python` = arbitrary exec; no tx/undo/replay |
| [sam-david/unreal-mcp](https://github.com/sam-david/unreal-mcp) | ~7 | UNVERIFIED | 127 / 16 subsystems DOCS-CLAIM | + - ~ - - + - | 4 transport layers, no mandatory C++ plugin | Days old, unverified; no tx/replay |
| [appleweed/UnrealMCPBridge](https://github.com/appleweed/UnrealMCPBridge) | ~65 | MIT | handful + pattern | + - - - - - - | `@mcp.tool`/`@mcp.prompt` decorator pattern + canned build prompts | Tiny surface; no safety of any kind |
| [aadeshrao123/Unreal-MCP](https://github.com/aadeshrao123/Unreal-MCP) | new | UNVERIFIED | 288 cmds DOCS-CLAIM | + ~ - - - + - | Atomic material-graph builder + `.utrace` smart analysis | Upsell edition; no tx/undo/replay |
| [IvanMurzak/Unreal-MCP](https://github.com/IvanMurzak/Unreal-MCP) | ~40 | Apache-2.0 | 61 / 7 families DOCS-CLAIM | + - - - ~ + ~ | Game-thread dispatcher + bundled .NET sidecar + SceneCapture screenshots | Beta; cloud default; no tx/undo/replay |

## Blender / sandbox / Roblox / Bevy

| Repo | Stars | SPDX | Tools | R Tx U Rp A S P | Trick worth stealing | Weakness we beat |
|---|---|---|---|---|---|---|
| [ahujasid/blender-mcp](https://github.com/ahujasid/blender-mcp) | ~29.8k | MIT | dozens (? exec-core + asset search) | + - - - + + + | `BLENDER_MCP_SAFE_MODE` pre-exec script validator; exec generality | Arbitrary Python by default; no undo/tx/replay |
| [HoldMyBeer-gg/blend-ai](https://github.com/HoldMyBeer-gg/blend-ai) | ~150 | AGPL-3.0 (ideas only, never code) | 186 DOCS-CLAIM | + - - - - + - | ms-grade OpenGL viewport loop + auto-critique workflow prompts | Self-documents "no undo integration"; needs visible viewport |
| [sandraschi/blender-mcp](https://github.com/sandraschi/blender-mcp) | small | UNVERIFIED | 41 portmanteau / 150+ ops DOCS-CLAIM | + - - - - + + | Headless-by-default (`--background` subprocess) + live-bridge fallback | No undo/approvals/replay |
| [seehiong/blender-mcp-bridge](https://github.com/seehiong/blender-mcp-bridge) | small | UNVERIFIED | 98-106 DOCS-CLAIM | + - - ~ - + + | `--record session.json` + visual session editor (record/replay tool calls) | Replay re-sends calls, not deterministic resim; no tx |
| [kolbytn/mindcraft](https://github.com/kolbytn/mindcraft) | ~5.8k | MIT | n/a (Mineflayer agent, not MCP) | - - - - ~ - + | Provider-neutral 17-API + per-role models (chat/code/vision/embed/voice) + docker-on-risky | Plays Minecraft; authors nothing; coding sandbox self-disclosed vulnerable |
| [boshyxd/robloxstudio-mcp](https://github.com/boshyxd/robloxstudio-mcp) | ~490 | MIT | 43 (31 read-only) DOCS-CLAIM | + - - - - + ~ | Read-mostly ratio (31/43) as weak-model floor discipline | ARCHIVED 2026-06-06; no tx/undo/replay |
| [Chrrxs/robloxstudio-mcp](https://github.com/Chrrxs/robloxstudio-mcp) | ~270 | MIT | ~20s (?) | + - - - - + + | Non-pausing breakpoints + pre-add asset security scan + hash-readback chunk staging | No tx/undo/deterministic replay |
| [notmd/bevy_remote_inspector](https://github.com/notmd/bevy_remote_inspector) | ~30 | MIT | n/a (human UI on BRP) | + - - - - - - | BRP schema-driven generic inspector (entity tree + nested-type editing) | No AI door at all: BRP is an unclaimed MCP substrate |

Engine baselines (no MCP door; stars 2026-10-01): Godot 118k MIT; Bevy
48.5k Apache-2.0 (BRP built-in); Fyrox 9.6k MIT; O3DE 9.7k Apache-2.0
(gh: Other); Stride 7.8k MIT; Flax 7k custom EULA (gh: Other); Defold
6.3k custom (gh: Other). None offers preview/apply/undo/replay to agents.

## Capability matrix vs engine2040

| Capability | engine2040 | Best rival | Verdict |
|---|---|---|---|
| Bounded tool surface (budget ledger) | 20 tools, 38944-char gate | godot-mcp-pro profiles (`--3d`) | We win: budget enforced by test, theirs is manual |
| Read-only inspect | scene.read/brief/query, api.* | boshyxd 31/43 read-only | Tie on shape; we win on budget |
| Dry-run preview | tx.preview, no side effects | Chrrxs preview_asset (assets only) | We win: whole-batch preview |
| Atomic revision-checked apply | tx.apply + CAP1 | aadeshrao123 atomic material graph only | We win: only full-batch atomicity |
| Single-use approvals | revision+hash+expiry bound | wgt sandbox / ahujasid safe_mode | We win: only per-batch cryptographic-shape binding |
| Undo as new revision | tx.undo, monotonic, UndoneKey | AnkleBreaker per-action undo | We win: only revision-safe undo |
| Deterministic replay/resim | play.run/step + hash law | wgt seed-lock + snapshot/restore | We lead on paper; closest gap: expose seed-lock UX |
| Headless play loop | play.run jobs + poll/cancel | sandraschi headless-default | Tie; steal their default-headless posture |
| Capture/screenshots | capture receipt | blend-ai ms OpenGL loop + critique | We lose on latency/critique: steal it |
| Session record/replay | journal (internal) | seehiong session files + visual editor | We lose on exportability: steal it |
| No arbitrary code exec | by construction (test-gated) | almost all rivals exec | We win: biggest differentiator |
| Provider neutrality | adapters outside tick, mock/local/remote | mindcraft 17-API + per-role split | Tie on shape; steal per-role model split |
| Diagnostics loop | log.digest/query | AnkleBreaker compilation errors | We lose on compile-error specificity: steal it |
| Pre-add asset scan | envelope binds versions only | Chrrxs security scan | We lose: steal scan-before-bind |
| Generated skills | none | IvanMurzak per-project skills | We lose: steal skill emission |

## Top 10 steal-or-beat ideas (ranked)

1. Visual feedback loop: ms-grade capture + auto-critique prompt pack (blend-ai
   §Screenshot) — beat with budgeted capture. Owner `engine-endpoint`.
   Accept: `cargo test -p engine-endpoint --lib capture_budget`.
2. Session record/replay export: replayable tool-call tape file + diff view
   (seehiong Bridge Sessions) over our journal. Owner `engine-endpoint`.
   Accept: `cargo test -p engine-endpoint --lib session_record_replay`.
3. Seed-locked deterministic playtest UX: `playtest.seed` + fixed-delta +
   step-until-condition (wgt + satelliteoflove) bound to our hash law.
   Owner `engine-game`.
   Accept: `cargo test -p engine-game --lib seed_locked_step_until`.
4. Per-project skill emission: generate SKILL.md from the live tool surface
   (IvanMurzak cli) so weak models start constrained. Owner `engine-endpoint`.
   Accept: `cargo test -p engine-endpoint --lib skills_emit`.
5. Pre-add asset security scan before version bind (Chrrxs preview_asset) for
   the tx envelope asset refs. Owner `engine-editor`.
   Accept: `cargo test -p engine-editor --lib asset_scan_gate`.
6. Structured diagnostics retrieval loop: compilation/script error fetch as a
   first-class read tool (AnkleBreaker). Owner `engine-endpoint`.
   Accept: `cargo test -p engine-endpoint --lib diagnostics_fetch`.
7. Headless-by-default posture with live-bridge fallback (sandraschi):
   every scene op runnable without an attached editor. Owner `engine-editor`.
   Accept: `cargo test -p engine-editor --lib headless_apply`.
8. Agent-scoped undo tokens (`unity_undo_last` + agentId) layered on our
   revision-safe undo. Owner `engine-editor`.
   Accept: `cargo test -p engine-editor --lib undo_scope`.
9. Per-role model split: separate chat/code/vision/embedding models per
   request kind (mindcraft profiles) in `AiRequest`. Owner `engine-ai`.
   Accept: `cargo test -p engine-ai --lib role_models`.
10. Conformance vector proving the moat: run our 20-task authoring suite
    shape against rival claims (atomicity/approval/replay) and publish the
    fail table (AX15/EV18 family). Owner `engine-endpoint`.
    Accept: `cargo test -p engine-endpoint --lib conformance_vector`.

License hygiene: no GPL/AGPL code ports proposed (blend-ai AGPL ideas only);
all steals are reimplementable patterns. UNVERIFIED licenses: sam-david,
meta-quest ext, swax, sandraschi, seehiong, aadeshrao123 — treat as
ideas-only until re-checked.

RESULT: DONE - 29 repos surveyed + matrix + top-10 with owner crates and
acceptance commands | proof: docs/research/ai-engines-mcp-bridges.md:1
