# repomap: engine2040
_generated 2026-10-17T09:50:36.334Z | 6202 files mapped (post-exclusion: tree counts match the list, not raw disk) | 1159 hot (commits, last 14d) | hot-only: top 40 of 1159 hot files, max 15 symbols each | cap 16KB/500 lines_

## tree
- `.cargo/` - 2 files
- `.config/` - 2 files
- `.gitattributes` - 1 file
- `.github/` - 2 files
- `.gitignore` - 1 file
- `.ignore` - 1 file
- `.opencode/` - 35 files
- `AGENTS.md` - 1 file
- `AGENTS.test.mjs` - 1 file
- `CLAUDE.md` - 1 file
- `CONTRIBUTING.md` - 1 file
- `CONTRIBUTING.test.mjs` - 1 file
- `Cargo.lock` - 1 file
- `Cargo.test.mjs` - 1 file
- `Cargo.toml` - 1 file
- `README.md` - 1 file
- `arsenal.json` - 1 file
- `clients/` - 11 files
- `content/` - 822 files
- `crates/` - 1022 files
- `deny.toml` - 1 file
- `docs/` - 167 files
- `examples/` - 8 files
- `fixtures/` - 111 files
- `from-design-studio/` - 31 files
- `games/` - 1 file
- `llms.txt` - 1 file
- `opencode.jsonc` - 1 file
- `orders.csv` - 1 file
- `rustfmt.toml` - 1 file
- `scripts/` - 2 files
- `skills/` - 169 files
- `sprint/` - 3 files
- `src/` - 2 files
- `team/` - 8 files
- `tests/` - 5 files
- `tools/` - 226 files
- cold dirs left out (hot-only, no commit in 14d): 6 entries, biggest build/ 3549, .tools/ 2, .editorconfig 1, LICENSE-APACHE 1, SECURITY.md 1, rust-toolchain.toml 1
## big files (over one 64KB read - Grep, or read with offset+limit; never whole)
- `build/gm3/release/build/unicode-general-category-faf630b300439277/out/category.rs` - 618KB, ~158285 tok (not symbol-mapped)
- `.opencode/plugin/loop-keeper.js` - 278KB, ~71210 tok
- `crates/render/src/post.rs` - 229KB, ~58708 tok
- `crates/game/src/fighter_look.rs` - 207KB, ~52954 tok
- `docs/research/LADDER.md` - 199KB, ~50898 tok
- `crates/arena/src/lib.rs` - 189KB, ~48445 tok
- `crates/render/src/ibl.rs` - 181KB, ~46308 tok
- `crates/game/src/camera.rs` - 171KB, ~43806 tok
- `crates/endpoint/src/assistant.rs` - 168KB, ~43026 tok
- `docs/research/COMPARE.md` - 158KB, ~40435 tok
- `crates/game/src/bin/arena.rs` - 143KB, ~36574 tok
- `crates/endpoint/src/build_level.rs` - 138KB, ~35217 tok
- ... (+82 more over 64KB)

## symbols (hot-only, xN = commits last 14d, +refN = inbound import refs, ~N tok = bytes/4 hot only)
### `docs/SPRINT.md` x130 (7.0KB, ~1789 tok)
- # SPRINT — board and receipts
- ## How the board runs
- ## Areas
- ## Board
- ## Parked (not on the board; add a row to revive)
- ## Receipt log
### `docs/BLOCKERS.md` x114 (81KB, ~20824 tok)
- # BLOCKERS - second identical failures, with owner, repro and remedy
- ## Open
### `docs/EMPIRE-INBOX.md` x81 (17KB, ~4355 tok)
- # Empire inbox — engine2040
- ## Open
- ## Done
- ## Archive 2026-10-02 (diagnose W1-03: 3 stale improver FP diagnostics pruned from Open, texts preserved, nothing lost)
- ## Archive 2026-10-03 (stale 7+ days; texts kept, reopen by moving a line back under ## Open)
### `docs/SPRINT-19-receipts.md` x53 (59KB, ~15186 tok)
- ## Receipts
### `docs/design/SLICE.md` x52 (32KB, ~8066 tok)
- # SLICE — the arena duel scorecard (director seat)
- ## Row verdicts (first 20 open FOCUS rows, 2026-10-17T04:05Z, verified at 631e719)
- ## Cut log
- ## Landscape dry-run (first 10 LI rows, refreshed 2026-10-17, re-verified at 631e719)
### `tools/workspace-check.mjs` x49 (42KB, ~10786 tok)
- `const allowedEdges`
- `const LFS_SIZE_LIMIT`
- `function largeBlobViolations`
- `const OWN_TEST_BINARY`
- `function testLayoutViolations`
- `function parseModmapHeadings`
- `function parseModmapPaths`
- `function structuralMapViolations`
- `function parseBoardTasks`
- `const WORKSPACE_CHECK_HELP`
- `function hasHelpFlag`
### `docs/SPRINT-18-gaps.md` x41 (118KB, ~30285 tok)
### `.opencode/plugin/loop-keeper.js` x38 (278KB, ~71210 tok) - loop-keeper: continues THIS repo's /sprint when its session goes idle, and
### `docs/FOCUS.md` x38 (13KB, ~3402 tok)
- # FOCUS — Rome at war, online, looking like 2040. Arena first, then the city and the siege field, on Windows.
- ## Now (owner 2026-10-10): four Core proofs, engine depth, multiplayer last
- ## Core (owner 2026-09-30; wins over everything below where they differ)
- ## The game
- ## HARDEN (address more, waves go here)
- ## EMPIRE (owner 2026-09-24, `center/EMPIRE.md`)
- ## Second game — urban war (owner 2026-09-26, `plan/urban-war.md` (city is inside it))
- ## LIGHT (minimal, one wave each max)
- ## CUT (never — no files, no deps, no waves)
- ## OPEN (undecided — not cut, not scheduled, do not build yet)
- ## Slim rule
### `crates/arena/src/lib.rs` x33 (189KB, ~48445 tok)
- `mod collide`
- `mod controller`
- `mod eqs_bind`
- `mod fork`
- `mod gameworld`
- `mod gym`
- `mod limbs`
- `mod live`
- `mod parity`
- `mod probe`
- `mod recorder`
- `mod seed`
- `mod spine_step`
- `mod sync`
- `mod world`
- ... (+25 more, see file)
### `docs/plan/BOOK-GAPS-2026-10-12.md` x33 (38KB, ~9815 tok)
- # Book gaps vs engine2040 (2026-10-12)
- ## 1. Verdict
- ## 2. Wired but unused (code with no live caller)
- ## 3. The render order (Gregory ch10 and Lengyel agree)
- ## 4. Gaps by subsystem (HAS / PARTIAL / MISSING)
- ## 5. Plan says X, code or book says Y
- ## 6. Netcode: findings and a decision for the engine2040 lead
- ## 7. What I re-checked, and what was wrong
### `docs/SMOOTH-BUILD.md` x30 (118KB, ~30237 tok)
- # SMOOTH-BUILD — build the game without friction; let problems fix themselves
- ## 1. See every change instantly
- ## 2. The engine fixes content for you
- ## 3. The engine fixes code for you
- ## 4. The project fixes itself
- ## 5. Bugs arrive ready to fix
- ## 6. Playtests run themselves
- ## Rules for anything that fixes itself
- ## First useful workflows (same project, not separate demos)
- ## OPT1 build baselines (observed-only, speed102, 2026-09-30)
- ## OPT4 re-verify (2026-10-12, KEEP rust-lld, no config change)
- ## OPT8 hakari spike (2026-10-12, builder-build-r3, evidence-only, no config change)
- ## OPT4/OPT5/OPT7 landed evidence (2026-10-12, builder-build-r6, docs-only, no re-verify)
- ## OPT9 sccache landed evidence (2026-10-12, builder-build-r7, docs-only, no config change)
- ## OPT32 proposal: warm sccache + one target-dir per slot (2026-10-13, config-only, no build run)
- ... (+9 more, see file)
### `docs/design/STAGES.md` x30 (58KB, ~14852 tok)
- # STAGES - where the game is, and what blocks the next stage
- ## Stage table
- ## 1. Preproduction - passed: no
- ## 2. First Playable (the duel plays start to end, product check green: G1) - passed: no
- ## 3. Vertical Slice (one bout at near-final quality) - passed: no
- ## 4. Alpha (feature complete for the slice scope) - passed: no
- ## 5. Beta (content and UI done; only fixes, stability and speed land) - passed: no
- ## 6. Release candidate (the release checklist) - passed: no
- ## Risks (top 3, ranked by blocked rows) - producer run 2026-10-17T05:23Z planner-producer-r406
- ## Unblock (Rule 4) - r406
- ## Run log (one line per run; r405 is first and newest, then r390 to r404 oldest first)
### `crates/game/src/bin/arena.rs` x29 (143KB, ~36574 tok) - ! arena.exe: window binary (winit loop, fixed-tick accumulator,
### `docs/VISION.md` x28 (35KB, ~9007 tok)
- # VISION — Rust engine, AI-native tools, convincing 3D
- ## Product promises
- ## Boundaries that make this possible
- ## Priorities
- ## Final picture: what the finished engine looks like (owner 2026-09-28)
- ## Final bars (TARGETS — only `tools/finish.mjs` says what is met)
- ## What stays undecided (OPEN — do not build yet)
- ## Anti-vision (CUT — never)
- ## Full engine depth: the worlds and workflows we intend to support
- ## Plans: which plan serves which part of the vision
- ## Steal map (scouts: what we read, oldest first)
- ## How research aims at this file
### `crates/endpoint/src/mcp.rs` x26 (118KB, ~30226 tok)
- `fn tool_names`
- `fn default_tool_names`
- `struct ToolCostRow`
- `struct ToolBudgetLedger`
- `fn check_tool_budget`
- `fn negotiate_version`
- `struct McpToolError`
- `fn new`
- `fn with_data`
- `fn code`
- `fn data`
- `fn wrap`
- `fn build_batch`
- `fn batch_error`
- `fn cap_error`
- ... (+25 more, see file)
### `crates/game/src/lib.rs` x25 (126KB, ~32250 tok)
- `mod anim_bridge`
- `mod arena_kit`
- `mod audio_director`
- `mod audio_mix`
- `mod balance_verdict`
- `mod bench`
- `mod camera`
- `mod camera_arm`
- `mod camera_feed`
- `mod capture`
- `mod crash`
- `mod crowd_gpu`
- `mod dev_hotpatch`
- `mod director`
- `mod fight_ring`
- ... (+25 more, see file)
### `tools/drift-check.mjs` x24 (37KB, ~9586 tok)
- `const focusFamilies`
- `const isShellJunk`
- `const shellJunk`
- `const DRIFT_ROW_RE`
- `const DRIFT_EXCUSE_SOURCE`
- `const DRIFT_NEG_RE`
- `const isExcuseLine`
- `const excuseLines`
- `const parseInboxOpen`
- `const parseKeeperState`
- `const STUCK_STATIC_WINDOW_MS`
- `const STUCK_STATIC_THINKER_MS`
- `const STUCK_TUNED_MIN_MS`
- `const medianMs`
- `const tuneStuckTimeout`
- ... (+4 more, see file)
### `docs/INDEX.md` x23 (5.4KB, ~1392 tok)
- # INDEX — active contracts
- ## Precedence
- ## Active set
- ## How-to (docs/book/)
### `crates/perf/src/main.rs` x21 (75KB, ~19088 tok) - ! perf: stress harness -> perf.csv. Presentation only, never hashed.
### `docs/SPRINT-07-accepted-plan-rows.md` x21 (79KB, ~20308 tok)
### `docs/research/LADDER.md` x21 (199KB, ~50898 tok)
- # Research ladder — every idea, one line, climbing
- ## Pillar cursor
- ## Lines
### `crates/anim/src/lib.rs` x20 (7.1KB, ~1830 tok)
- `mod blend`
- `mod clip`
- `mod contract`
- `mod crossfade`
- `mod crowd_scale`
- `mod edit`
- `mod events`
- `mod exert`
- `mod face`
- `mod fbik`
- `mod feet`
- `mod graph`
- `mod hit_react`
- `mod ik`
- `mod inertialize`
- ... (+25 more, see file)
### `crates/game/src/windowed.rs` x20 (127KB, ~32628 tok)
- `fn msaa_setting`
- `enum WinError`
- `enum SceneSelect`
- `fn parse`
- `fn id`
- `enum CameraPath`
- `fn seed`
- `fn frames`
- `fn pin_session_for_camera_path`
- `fn orbit_eye_for_frame`
- `fn apply_camera_orbit`
- `struct WinArgs`
- `fn wall_limit_ms`
- `fn smoke_fps`
- `fn smoke_muted`
- ... (+25 more, see file)
### `docs/ARCHITECTURE.md` x20 (18KB, ~4515 tok)
- # ARCHITECTURE — boundaries before more subsystems
- ## 1. Dependency direction
- ## 2. Rust workspace foundation — M0 checklist
- ## 3. Scene and data contracts — settle before a complex renderer
- ## 4. Three clocks, separate responsibilities
- ## 5. Plugins, reload and failures
- ## 6. Verification is part of the design
### `crates/assets/src/lib.rs` x19 (9.7KB, ~2493 tok)
- `mod budget`
- `mod datanode`
- `mod extras`
- `mod validate`
- `mod archive`
- `mod archived_cook`
- `mod bundle`
- `mod cooked`
- `mod cooker`
- `mod bundle_cook`
- `mod bc`
- `mod body_import`
- `mod clip_intake`
- `mod clip_receipt`
- `mod gen_intake`
- ... (+25 more, see file)
### `crates/combat/src/fight.rs` x19 (133KB, ~34127 tok)
- `struct Tables`
- `fn class_tables`
- `fn poise_damage_for_kind`
- `fn stamina_cost_for_kind`
- `fn poise_regen_delay_for_tables`
- `fn hitstop_for_impact`
- `struct FighterState`
- `struct FightState`
- `fn new`
- `enum FightEvent`
- `struct AttackStarted`
- `enum CombatError`
- `fn validate_inputs`
- `struct StrikePose`
- `fn validate_strike_poses`
- ... (+10 more, see file)
### `docs/plan/README.md` x19 (12KB, ~3058 tok)
- # Plans — the one index, and how a plan is written
- ## Plan index (owner 2026-10-10: one index for every file in `docs/plan/`)
- ## How a plan is written
- ## Rules
- ## Template (exact headings, 250–450 lines)
- # <Subsystem> — plan to the VISION.md bar
- ## 1. Where it stands capability | status | evidence (file or commit)
- ## 2. Finish line Tier A alpha, Tier B v1, Tier C VISION bars, measurable
- ## 3. Target design data flow, ownership, threads, determinism, budgets, API sketch
- ## 4. Hard problems problem → technique → why → reference → donor
- ## 5. Donors one line: `Donors: steals.md; porting policy is AGENTS rule 4.`
- ## 6. Slim delete, merge, freeze, and what we will not build
- ## 7. Board tasks rows in board format; then "Row changes" for existing rows
- ## 8. Quality gates tests, benchmarks, captures, budgets kept green forever
- ## 9. Session estimate sessions per tier A/B/C
- ... (+4 more, see file)
### `crates/game/src/fighter_look.rs` x18 (207KB, ~52954 tok)
- `enum LookError`
- `enum Act`
- `struct SkinSplit`
- `struct PresentReport`
- `enum Shot`
- `struct LookFrame`
- `fn shot_for_frame`
- `fn look_down_deg`
- `struct FighterLook`
- `fn fighters_content_dir`
- `fn look_mesh_key`
- `fn load`
- `fn bundle_shas`
- `fn rig_for_slot`
- `fn variant_for_slot`
- ... (+9 more, see file)
### `docs/SPRINT-18-steals.md` x18 (29KB, ~7501 tok)
### `docs/plan/render-look.md` x18 (40KB, ~10355 tok)
- # Render look (Look)
- ## 1. Where it stands
- ## 2. Finish line
- ## 3. Target design
- ## 4. Hard problems
- ## 5. Donors
- ## 6. Slim
- ## 7. Board tasks
- ## 8. Quality gates
- ## 9. Session estimate
### `docs/plan/steals.md` x18 (15KB, ~3965 tok)
- # Steals: open steals that land on a finish bar
- ## Steals
### `tools/proof.mjs` x18 (37KB, ~9454 tok)
- `function parseProofArgs`
- `const PROOF_HELP`
- `const IR_BENCHES`
- `const E12_QUARANTINE`
- `function irGateVerdict`
- `const FRAME_P95_BAR_MS`
- `const FRAME_CSV_PATH`
- `function frameCsvVerdict`
- `function changedCrateDirs`
- `function packageNameOf`
- `function toolTestSummary`
- `const NEXTEST_PROFILE`
- `const NEXTEST_CONFIG`
- `function nextestReceiptPath`
- `function junitLabel`
- ... (+7 more, see file)
### `tools/board-slice.mjs` x17 +ref10 (23KB, ~5880 tok)
- `function boardShardFiles`
- `function loadBoardText`
- `function archiveDoneFiles`
- `function archivedDoneRows`
- `function focusNowIds`
- `function parseBoard`
- `function needsOf`
- `function ownedOf`
- `function rowReady`
- `function focusAncestors`
- `function blockedBy`
- `function pickNext`
- `function ticksOf`
### `crates/ai/src/depth.rs` x16 (110KB, ~28119 tok)
- `enum DepthError`
- `enum Phase`
- `fn from_u8`
- `fn for_tick`
- `enum Role`
- `struct TierRow`
- `fn personality`
- `fn personality_for_phase`
- `fn from_row`
- `enum TierError`
- `fn parse_tiers`
- `struct FeintState`
- `fn update`
- `struct ArenaPolicy`
- `fn set_phase`
- ... (+25 more, see file)
### `crates/game/src/arena_kit.rs` x15 (77KB, ~19817 tok)
- `struct KitZoneRow`
- `fn kit_zone_rows`
- `fn kit_meets_tris_floor`
- `enum KitError`
- `fn kit_match_world`
- `fn kit_match_world_or_empty`
- `fn kit_prop_rows`
- `fn kit_asset_files`
- `fn kit_mesh_key`
- `struct FitParams`
- `fn aabb_of`
- `fn fit_params`
- `fn apply_fit`
- `fn kit_content_dir`
- `struct BakedKitMesh`
- ... (+11 more, see file)
### `crates/net/src/lib.rs` x15 (7.9KB, ~2016 tok)
- `mod cells`
- `mod courier`
- `mod dedicated`
- `mod delay`
- `mod delta`
- `mod heal`
- `mod interest`
- `mod interp`
- `mod link`
- `mod lobby`
- `mod netcode`
- `mod online`
- `mod played`
- `mod quic`
- `mod receipt`
- ... (+15 more, see file)
### `crates/physics/src/lib.rs` x15 (3.3KB, ~847 tok)
- `mod balance`
- `mod controller`
- `mod controller2`
- `mod islands`
- `mod kcc`
- `mod limb`
- `mod overlap`
- `mod pd`
- `mod powered`
- `mod ragdoll`
- `mod ragdoll_tape`
- `mod rapier`
- `mod rigid`
- `mod stagger_memory`
- `struct Space`
- ... (+5 more, see file)
### `crates/render/src/lib.rs` x15 (17KB, ~4430 tok)
- `mod window`
- `mod graph`
- `mod quality`
- `mod v1`
- `mod variants`
- `mod v1_ibl_shadow`
- `mod v1_skin`
- `mod v1_lighting`
- `mod v1_temporal`
- `mod v1_gpu`
- `mod present`
- `mod overlay`
- `mod pass_truth`
- `mod heal`
- `mod cache`
- ... (+25 more, see file)
### `docs/plan/ATLAS.md` x15 (10.0KB, ~2550 tok)
- # Plan atlas - where everything is planned, and how true each plan is today
- ## 1. Every plan: rows, crates and health
- ## 2. Row prefix -> plan
- ## 3. Crate -> plans

_hot-only: 1228 more files are below the cut and not listed_
