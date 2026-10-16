# Skills - 14 agents, 85 skills in skills/ and 6 in .opencode/skills/

Merge record: center-external `knowledge/TEAMS.md` (not in this repo; 2026-09-27: 25 roles -> 9; each
merged agent's text kept as a skill). Disk wins.

Agents in `.opencode/agents/` (free Muse only); skills in `skills/`, one line each:
agent -> skill -> packet. Packets run foreground (`packet: <name>`).

| orchestrator (lead) | engine-orchestrate | one-offs: choose work, write packets, verify diffs/CI/replay/hash/perf |
| builder | engine-code | CODE: crates, tests, determinism, render, scene/editor, gameplay AI, deps |
| builder | engine-experiment | EXPERIMENT spikes: smallest repro + one ablation, adopt/reject |
| builder | engine-docs | DOCS: rustdoc, runnable examples, mdBook guides, missing_docs cover |
| builder / researcher | engine-proof | PROOF: MAINT repairs, tree gates; JAN audits read-only, ranked |
| builder | engine-assets | ASSETS: CC0/CC-BY cook into content/, source and terms recorded per asset |
| builder / pilot | engine-bench | BENCH: paired comparable runs, distribution, allocation numbers (profile-first overlap: `.opencode/skills/rust-perf-profiler` finds the bottleneck) |
| builder / pilot | engine-look | ART + LOOK REVIEW: captures-vs-reference scoring with per-pass cost |
| builder / planner | engine-game | DESIGN + FEEL reviews of fight, feel, fun from playtest evidence |
| pilot | engine-playtest | PLAYTEST: play the build via MCP play tools, report bugs and fun |
| pilot | engine-provider | PROVIDER: conformance matrix, recorded/offline replay |
| researcher | engine-research | RES-STEAL / RES-ARXIV / RES-WILD / EXPLORE, license discipline |
| judge | engine-review | REVIEW / CHECK / API-REVIEW: depth and evidence bar, Eyes on captures (Rust bar lives in `engine-rust-quality`, not repeated here) |
| judge | engine-rust-quality | REVIEW / CHECK: packet-gate, 0-5 rust score, what tools cannot see (soundness overlap: `unsafe-audit` was removed 2026-10-12, never loaded) |
| judge / builder / planner | engine-subsystem-review | tick/timestep, frame budget, hot-path alloc, handles, jobs, determinism, init order: 8 checks (Gregory GEA ch 5-7, via skillworks) |
| builder / judge | render-math-checklist | matrix/quat, normals, tangents, reverse-Z, shadow bias, linear color, float hygiene: 10 checks (Lengyel FGED 1-2, via skillworks) |
| builder / judge | render-pipeline-rules | frame pipeline: a pass is live only if pixels change, no GPU waits in a frame, fog, cascades, sort, normal maps, velocity: 10 rules (engine review 2026-10-10, render.md) |
| builder / judge | frame-loop-rules | live loop: no replay from tick 0, no thread per fork, off-thread results at boundaries, asset refs, audio queue, debug tools, crash hook, bucket writes: 8 rules (engine review 2026-10-10, runtime.md) |
| builder / judge | ai-door-rules | AI door: models outside the tick, idempotent undoable verbs, three guards, no god-agent, 1000-game eval, offline search, int8 net, dead reckoning: 8 rules (engine review 2026-10-10, ai.md) |
| builder / judge | audio-cue-rules | audio mix: 1/r falloff, cues not clips, steal by priority, ducking, music playlists: 5 rules (engine review 2026-10-10, ai.md audio lines) |
| builder / judge | anim-combat-rules | animation and bodies: sim-fed poses, frame order, cross-fade, loop samples, additive, root motion, ragdoll handoff, capsule moves, swing-twist, limb cuts: 10 rules (engine review 2026-10-10, bodies.md) |
| builder / judge | test-truth-rules | tests and gates: pixels not manifests, GPU skips, timing asserts, capture from the surface, crash path, oracle first, smoke first: 9 rules (engine review 2026-10-10, testing.md) |
| builder / judge | rust-craft-rules | Rust unsafe proofs, process-wide state, lock guards, real-time try_lock, scoped threads: 6 rules; the rest sit in engine-code (`cache-and-ordering-checks` removed 2026-10-12, never loaded) (engine review 2026-10-10, rust-craft.md) |
| orchestrator | engine-land | LAND: how the lead lands one judged row on master from the one tree (gate, tests, golden hash, ratchet, receipt, push) — model skill: shortest executable bar, copy its shape |
| pilot / planner | door-and-headless-runbook | headless staged fight, golden proof, arena capture, perf CSV, the e2040-mcp door, the playtest crate |
| miner | engine-subsystem-review | SKILL: turn one book chapter into a tested checklist skill shaped like engine-subsystem-review |
| perf | engine-bench | PERF: measure frame and tick time on the built binaries, file one cut packet |
| porter | engine-code | PORT: turn one donor file into a Rust port packet with its test |
| rot-hunter | engine-proof | CUT: find dead code, stubs and stale docs, file one cut packet per finding |
| runner | door-and-headless-runbook | RUN: run the built binaries and the AI door, report numbers, file a bug packet on a break |
| skill-hunter | opencode-loop | SKILL: find, test and install one skill that fixes a measured loop failure |
| smith | opencode-loop | TOOL/DOCS/SKILL: take node, docs and skill packets, edit only packet paths, run the node test |
| walker | engine-playtest | WALK: walk one journey as a user on the built game, score it, file at most 2 asks |
| every agent | opencode-loop | every turn + helper packet: foreground batches, RESULT line, pwsh-only shell |

## More skills in skills/ (no agent names them: they load by their trigger, so read the trigger first)

| who | skill | when |
|---|---|---|
| builder / judge | gaffer-netcode | crates/net session, snapshot, interpolation, online: 7 checks with the symbols to look at (no extrapolation) |
| builder / judge | physics-determinism | change a crates/physics solver or the rapier path without moving a hash |
| builder / judge | render-surface-device-loss | resize, minimize, surface loss, device loss in crates/render present |
| builder / judge | wgpu-validation-fix | a wgpu validation error or a PresentError in a crates/render submit |
| builder / judge | mcp-verb-authoring | add or change an MCP verb in crates/endpoint: name, card, refusals, dry run, audit, budget |
| builder / judge | checked-numerics | a numeric `as` cast or integer and float math in sim code |
| pilot / planner | bug-report-writing | gates for a bug queue packet: steps, expected, actual, build, proof |
| researcher / builder | book-search | search the game-engine and Rust books with `node tools/books.mjs search "<words>"`, cite book id and heading |
| builder | bevy-rust-ecs | port Bevy 0.19 names into the own renderer or sim; a Bevy viewer may not be built |
| builder / judge | wired-caller-checks | a new pub fn, helper, or module in crates: 7 checks, FAIL without a production caller |
| orchestrator (lead) | lane-index-guard | before dispatch and land.mjs index check: 7 checks on status, claims, diffstat, ratchet |
| builder / judge | link-env-triage | a link fail with a named symbol while check stays green: 7 checks, env never code churn |
| planner / builder | hotspot-recon | planning a row before reading code: 8 checks rank churn, hotspots, owners, fix rate |
| builder / judge | eval-regression-checks | an AI-door eval or win-rate claim: 6 checks, frozen baseline plus paired runs (with ai-door-rules) |
| builder / judge | crash-stack-triage | a crash with a stack or panic log: 6 checks map frames, rule out env, one bug packet |
| builder / judge | gate-red-fix | a red gate or CI check: 6 checks, smallest fix plus same-check rerun (hands depth to engine-review) |
| pilot / planner | bug-queue-triage | more than one open bug packet: 6 checks rank repro first plus hash impact before assign |
| builder / judge | systematic-debug | any bug, test failure, unexpected behavior, perf dip, or build break: root-cause-first gate with evidence before fixes |
| planner / builder | acquire-codebase | entering unfamiliar crates, onboarding, or risk check before reading code: map intent and boundaries first, hand off to hotspot-recon |
| every seat | evidence-pack | before writing DONE, green, a number, or a score: one claim bundles command, exit, hash, excerpt, verdict for replay |
| orchestrator (lead) | delegate-context-pack | before dispatching a delegate/builder packet: 40-line execution-ready pack, every item supports a decision |
| orchestrator (lead) | causal-triage-plan | one wave drops three or more linked errors: causal map plus delegation plan before any fix |
| orchestrator (lead) | refactor-gates | planning a cross-crate refactor before any edit: blast radius, staged batches, per-batch CI, rollback, HALT until approval |
| builder / judge | feel-fps-checklist | hit pause, shake, squash, flash or slow motion: prove the hit lands without breaking determinism or budget |
| pilot / runner | replay-capture-perf-triage | a replay, capture set, or perf run looks wrong: replay/desync vs capture-path vs perf noise before any bug packet |
| orchestrator (lead) | land-gate | landing a packet through land.mjs: gate the patch before any commit, refuse on FAIL |
| builder | asset-budget-checks | potato asset budget in crates/assets: tris, bones, influences, materials, texture pixels inside caps |
| builder | camera-arm-checks | third-person camera arm in crates/game that shortens at walls and pillars |
| builder / judge | capture-pixel-checks | window capture or frame gate: pass verdict comes from decoded pixels |
| builder | cargo-book | Cargo manifests, features, profiles, workspaces, config: lockfile, unification, defaults |
| builder | cargo-slot-wait | cargo build waits on busy slots or file lock: wait, do non-cargo work, or PARTIAL |
| builder | claim-paths-guard | seat guard blocked write no packet paths or out of packet paths before edits |
| builder | coder-overlap-guard | keeper batch starting a second coder packet held for paths overlap with running coder |
| builder / judge | combat-rows-checks | data-driven combat numbers in crates/combat: rows in text files with validation |
| builder / judge | deferred-tile-checks | deferred sun lighting, tile classification, G-buffer packing: check the light pass |
| builder / judge | fearless-concurrency-checks | threads, channels, shared counters: prove the handoff is ordered and safe |
| builder | fight-timing-tables | fight timing or parry windows in crates/combat before the bout ships |
| builder / judge | flow-shell-checks | title pause result or rematch shell: check flow states and tick accounting |
| builder / judge | fov-vision-checks | tile visibility, sight-gated pursuit, explored-map memory before the diff lands |
| builder / judge | ggrs-rollback-rules | ggrs 0.13 rollback loop: Config, save/load, time sync, desync, input delay |
| builder / judge | gtao-ao-checks | GTAO ambient occlusion in the live arena frame: check the darkening |
| builder / judge | hearing-cull-checks | hearing events in crates/ai: check the sound table |
| builder / judge | hud-overlay-checks | HUD, menu or text overlay: check order, coordinates, font atlases |
| builder / judge | ibl-light-checks | image-based lighting, normal/gloss maps, env cubes, IBL bake before live |
| builder / judge | isovist-polygon-checks | visibility polygon or isovist sweep: ray math, ordering, area |
| builder / judge | msaa-resolve-checks | 4x MSAA in crates/render: thin edges blend with resolve before blit |
| builder | proof-guard-checks | seat guard blocks proof or workspace cargo: crate-scoped proof plus packet |
| builder / judge | rust-test-checks | Rust unit or integration tests: prove behavior before the diff lands |
| builder / judge | sand-rest-checks | down pose resting on sand in crates/game: check sunk bodies |
| builder / judge | shadow-map-checks | arena point/spot/sun lights or shadow maps: check the lit frame |
| builder | sky-bake-checks | analytic sky bake, fog feed, exposure anchor before the sky goes live |
| builder | spatial-support-checks | stacked placement in crates/spatial resting on shelves and stairs |
| builder / judge | strain-based-body-checks | soft-body or cloth deformation: constraints, bending, volume hold |
| builder / judge | torch-light-checks | torch point lights in crates/render: check the forward shade |
| builder | tri-mesh-checks | triangle mesh cook or budget in crates/assets before import lands |
| builder / judge | visibility-cull-checks | frustum, light, shadow, portal, occluder culling: check the culled frame |
| builder | vram-budget-checks | VRAM math, staging belt, streaming pool, eviction rule before budget live |
| builder / judge | wgpu-render-rules | wgpu 30.0 code: instance, surface, pipelines, buffers, bind groups, alignment |
| builder / judge | wound-map-checks | wound paint maps for fighter cuts: check the queue |

Commands: only `/sprint` in `.opencode/commands/` (`/next`, `/gates` and `rust-coder`
are gone, merged per TEAMS.md). Restart OpenCode after changing plugins, configs or
env; the keeper body hot-reloads itself (only its shell or HOOKS list needs a restart);
agent/skill edits apply to new sessions. Verify: `opencode debug agent <name> --pure`.

## In `.opencode/skills/` (OpenCode project folder, not in this table)

- Shelf skills (one master: `C:\empire\center\skills-shared\`, loaded by path; no copies kept here since 2026-10-13): bash-abort-guard, edit-reread, engine-builder, git-one-branch, pwsh-for-bash-writers. `repo-read-first` lives only in the master (no copy here).
- Engine-only: desync-debug, grep-overflow-guard, loop-dupfix, rust-perf-profiler, smith-scope-guard.
- Installed from skillworks 2026-10-10: rust-book, refactoring-to-rust (Rust; both removed 2026-10-12, never loaded); reflection-gate (moved to `docs/archive/2026-10-12/skills/reflection-gate/` on 2026-10-12, never loaded), multi-agent-batch (removed 2026-10-12, never loaded), planning-rows (Dushkin loop skills). rust-borrow-fix was moved to `docs/archive/2026-10-12/skills/rust-borrow-fix/` on 2026-10-12: the bare model passed all 31 rustc repair tasks, so it adds nothing.
- Verified 2026-10-11 on the live server (`GET /skill`): OpenCode loads `skills/` (through `skills.paths`), `.opencode/skills/` and the global skill folder. A skill whose `description:` holds a colon and a space must be double-quoted, or it is not loaded.

Retired 2026-10-10 (moved to `docs/archive/2026-10-10/skills/`): bash-allowlist and bash-spawn-guard (merged into bash-abort-guard), edit-unique (merged into edit-reread), ready-file-check (the AGENTS.md rule covers it), read-offset-guard, real-browser-automation, the Bevy viewer copy (kept as a port map in `skills/bevy-rust-ecs`), and a notes file.

Retired 2026-10-11 (moved to `docs/archive/2026-10-11/skills/`): game-testing-check (keyword list, a dead command), combat-data-tables (a loot model the code does not have), rust-state-machines (examples that do not exist), tool-door-patterns (an HTTP checklist for a local MCP door; see mcp-verb-authoring), dead-reckoning-checks (the netcode never extrapolates; a note sits in gaffer-netcode), and from `.opencode/skills/`: systems-rust, idiomatic-rust, agent-ops-checklist (generic advice) and the second copy of rust-atomics (the one in `skills/` stays).
