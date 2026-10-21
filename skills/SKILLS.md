# Skills - 33 skills in skills/ (none in .opencode/skills/)

New here? Read `skills/START-HERE.md` first (the 10 skills to read once, then a task-to-skill table).

Any AI (Claude Code, OpenCode, anything) loads a skill by its trigger. `AGENTS.md` wins over every skill. One skill per job you are about to do; read the trigger, open the one that matches, and follow links between skills instead of loading many.

Merged 2026-10-17 from 86 skills (82 here, 4 in `.opencode/skills/`). Nothing true was dropped; the old folders are in git history (`git log --all -- skills/<old-name>`). Old name to new home: see the table at the end.

## Work flow

| skill | when |
|---|---|
| commit-and-board | commit or push on the shared tree, write a new board row, plan rows, add a doc that needs an index line |
| tick-a-row | finish a board row: `[x]` plus `Landed <sha7>` and evidence, or a Progress note; the sed traps |
| github-check | any cargo test, clippy or check proof, a red gate or CI check, a link failure, a blocked local cargo lock |
| gh-check-playbook | which gh-check or web-run form for which job; exit 2 or 3, 0 tests, push or dispatch errors, GPU lines hang |
| rust-quality-ratchet | the pre-commit hook refused, `tools/rust-quality.mjs` FAIL, casts, long fns, big files; tighten or accept |
| wiring-a-feature | code exists but nothing calls it, a wire row (`RZ4w`), or before you write something that may exist |
| recover-deleted-code | a file a row or plan names is gone (rot cleanup 2026-10-16): find it in git, restore or rewrite |
| repo-orientation | entering an unfamiliar crate, ranking risk before a plan, narrow search when a wide grep fails |
| debug-triage | a bug, failing test, crash or panic stack, several linked errors, a bug report to file |
| determinism-debug | a golden hash, replay, tape or golden-matrix row diverges; a replay, capture or perf run looks wrong |
| test-and-proof | writing a test, gate or capture check; before writing DONE, green, a number or a score |
| review | reviewing a diff or auditing an area: depth score, rust bar, callers, captures, API review |
| research | a research card, donor repo, arXiv paper, spike, the books (`tools/books.mjs`), Bevy ports |

## Writing engine code

| skill | when |
|---|---|
| engine-code | writing Rust in any crate: modules, manifests, features, deps, public API, rustdoc, refactors, donor ports |
| rust-safety | unsafe, statics, atomics, locks, threads, channels, real-time callbacks, numeric casts in sim code |
| tick-and-runtime | the fixed tick, live window loop, frame budget, per-tick alloc, asset handles, jobs, startup, crash hook |
| perf-bench | measuring or optimizing speed, frame or build time, any speedup claim |

## Subsystems

| skill | when |
|---|---|
| physics-bodies | crates physics, physfx, destruction: ragdoll, limbs, debris, breaks, materials, Rapier, cloth |
| anim-combat | clips, blending, root motion, foot IK, movement, camera boom, fight timing, combat rows, hit feel, sunk bodies |
| net-rollback | crates/net session, snapshot, interpolation, online, rollback loops |
| spatial-perception | FOV, isovist, sight-gated chase, explored maps, AI hearing, stacked placement |
| game-shell-audio-ui | title/pause/result shell, HUD and overlays, menus, how sound plays |
| ai-door | MCP verbs and packs in crates/endpoint, provider adapters, evals and win rates, offline search, int8 nets |
| door-and-headless-runbook | running without a window or a person: staged fight, golden proof, capture, perf CSV, the e2040 MCP door |
| arena-capture-and-view-line | run `arena.exe --exit`, read the `arena view:`, `arena break:` and `physfx:` lines (pops, foot slide, debris), take captures |
| design-and-playtest | game design data, feel and fun reviews, playing through the MCP play tools, look reviews |

## Render, web, content

| skill | when |
|---|---|
| wgpu-render | wgpu 30.0 code: setup, pipelines, buffers, bind groups, readback, validation errors, surface and device loss |
| render-pipeline-math | render passes, readback, fog, cascades, draw order, MSAA/TAA, WGSL, matrix/normal/depth/color math |
| render-lighting | shadows, point/spot/sun lights, torches, IBL, normal and gloss maps, GTAO, sky bake, wound maps |
| render-cull-vram | frustum, light, shadow, portal and occlusion culling; VRAM math, staging, streaming pools, eviction |
| web-wasm | wasm32 builds, the browser shell, dist/web, headless Chrome proof, web workflows |
| browser-test-on-this-pc | check the published Pages build in the Claude app's browser pane: golden line, frames, errors, commit match |
| assets-content | finding, licensing, cooking and budgeting content in content/ or crates/assets |

## Old name to new home (2026-10-17 merge)

| new skill | old skills folded in |
|---|---|
| commit-and-board | commit-by-path, board-row-writing, planning-rows |
| github-check | github-check, gate-red-fix, link-env-triage |
| repo-orientation | acquire-codebase, hotspot-recon, grep-overflow-guard |
| debug-triage | systematic-debug, crash-stack-triage, causal-triage-plan, bug-report-writing |
| determinism-debug | desync-debug, golden-matrix, replay-capture-perf-triage |
| test-and-proof | test-truth-rules, capture-pixel-checks, rust-test-checks, evidence-pack |
| review | engine-review, engine-rust-quality, wired-caller-checks, engine-proof |
| engine-code | engine-code, engine-docs, cargo-book, refactor-gates |
| rust-safety | rust-craft-rules, fearless-concurrency-checks, checked-numerics |
| tick-and-runtime | engine-subsystem-review, frame-loop-rules |
| perf-bench | engine-bench, rust-perf-profiler |
| physics-bodies | physics-determinism, two-physics-layers, strain-based-body-checks |
| anim-combat | anim-combat-rules, fight-timing-tables, combat-rows-checks, feel-fps-checklist, camera-arm-checks, sand-rest-checks |
| wgpu-render | wgpu-render-rules, wgpu-validation-fix, render-surface-device-loss |
| render-pipeline-math | render-pipeline-rules, render-math-checklist, msaa-resolve-checks |
| render-lighting | shadow-map-checks, torch-light-checks, ibl-light-checks, sky-bake-checks, gtao-ao-checks, deferred-tile-checks, wound-map-checks |
| render-cull-vram | visibility-cull-checks, vram-budget-checks |
| spatial-perception | fov-vision-checks, isovist-polygon-checks, hearing-cull-checks, spatial-support-checks |
| design-and-playtest | engine-game, engine-playtest, engine-look |
| game-shell-audio-ui | flow-shell-checks, hud-overlay-checks, audio-cue-rules |
| net-rollback | gaffer-netcode, ggrs-rollback-rules |
| ai-door | ai-door-rules, eval-regression-checks, engine-provider, mcp-verb-authoring |
| door-and-headless-runbook | door-and-headless-runbook |
| web-wasm | wasm-compat, web-build-and-test |
| assets-content | engine-assets, pbr-texture-sources, asset-budget-checks, tri-mesh-checks |
| research | engine-research, engine-experiment, book-search, bevy-rust-ecs |
