# Forge and old-plugin adoption (owner 2026-10-11)

Source: a 16-part audit of Forge (C:orge) and the old Flax plugin against this engine, merged and checked in code by an architect pass. Rows AD1..AD20 are on the board in `SPRINT-18-gaps.md` (section "Forge and old-plugin adoption"). Section J (second pass, 2026-10-11) adds 14 more: AD21..AD29, PX40, AX23, EA6, PF24, SP66. Items not marked [V] are auditor claims that nobody re-opened.

Scope: what engine2040 should take from Forge (C:\forge) and the old Flax MCP plugin, what it already does better, and what is broken in the engine itself.
Inputs: C1, C2, C3, E01-E13 (E13 was present and is included), arch-review.md. Read-only. Nothing was built or run. PROPOSALS only.

Legend
- Engine status: HAVE (implemented + tested + a production caller), PARTIAL, PLANNED (doc or board row only), MISSING.
- [V] = I opened the engine file and the cited lines myself. [F] = I opened the Forge or old-repo file. [A] = auditor claim I did not re-open. Anything not marked in a "Checked:" line is [A].
- Score = Impact (1-5) x Cheap (S 3, S-M 2.5, M 2, L 1) x Corroboration (1 audit 1.0, 2 audits 1.3, 3+ audits 1.6). Ties broken by judgement.
- Hash rule used everywhere: IO, GPU, models, providers stay outside the fixed tick. GOLDEN_600 `d57b5c8ae3096614` is not touched by any item below. Where a spatial/other pinned digest moves, the item says so.

---------------------------------------------------------------------------------------------------
## A. ONE-PAGE SUMMARY

### A.0 The owner's fear, answered
The engine did NOT miss the big systems. In all three named areas it already has the better core. What it missed are small, cheap, specific rules (scar tissue) and, more important, it has landed a lot of correct code that no production caller reaches. Most of the work below is wiring and honesty, not new subsystems.

### A.1 What the engine already does BETTER (do not rebuild)
1. Auto-heal core: MAPE-K registry, 12-row catalog, quarantine with cool-down and one half-open probe, seeded jitter, tick-boundary heals, heal tape. `crates/observe/src/supervisor.rs`, `heal.rs` [V supervisor.rs:261-280, :370-379, :488-495]. Forge has no equivalent (only retry-proxy budgets and recovery wording).
2. Typed command service: base revision, idempotency (same key + other payload = conflict), preview does not mutate, atomic apply, undo as new revision, selective undo by receipt with conflict reasons. `crates/editor/src/commands.rs` [V :179-195, :436, :810], `commands/selective.rs` [V exists]. Forge text-patch + expected_hash is weaker.
3. Capability gate with single-use approval bound to session, revision, batch hash, expiry. `crates/editor/src/capability.rs` [V :24-31].
4. Staged-clone durable apply (seal first, commit after): `crates/editor/src/durable.rs::apply_persisted` [V :255-298]. Better than Forge's pending-journal marker. (It is not yet used by the door: see A.3 #2.)
5. Gameplay-code sandbox by construction: Rhai, integers only, deny-all registry, op/call/size budgets, script hash in the tape. `crates/rules` [V script.rs:55-56 budgets; tests.rs:303]. Forge/old used regex and Roslyn deny lists that fail open (E08 #10; E13 anti 3).
6. Save: previous-generation fallback, corrupt file never rotates over the backup. `crates/save/src/fallback.rs` [V exists; A detail].
7. Provider: strict structured-output ladder, content-addressed cassettes, arbiter. `crates/provider/src/{ladder,cassette,arbiter}.rs` [V exist; A detail]. Forge has no equivalent.
8. Spatial: 18 frozen checks, repair proposals with preview and stale-revision refusal, intent solver with conflict sets, budgeted brief with digest. `crates/spatial/src/{checks,repair,place,brief}.rs` [V checks.rs:9-28].
9. Replay and proof: GOLDEN_600 [V proof/src/lib.rs:36], bug bundles with first divergent tick (`proof/src/bug.rs`), rules tape replay, planted-bug agent (`playtest/src/{agent,seeded_bug}.rs`). Forge kept the words and lost the bots (E06 #6).
10. Ragdoll stack: Rapier ragdoll with per-bone CCD, powered drive, balance/stagger, limb bodies, cuts. `crates/physics/src/{ragdoll,powered,balance,limb}.rs` [V ragdoll.rs]. Forge's ragdoll wizard echoes text and applies nothing in places (E02 #4).
11. Anim: runtime foot lock and 1 cm slide meter `anim/src/feet.rs` [V slide_mm, 16 hits], state-machine lints (A).
12. Assets: licence + model-id gate before any work (`assets/src/gen_intake.rs` [V :31-49]), explicit-unit import repair (`import_repair.rs` [V exists]), 2048-byte floor (`assets/src/health.rs:4` [V]), STUB/REAL bundle refusal.
13. Search: BM25 with camel-case split and a 50-query gold set with a 90% floor `endpoint/src/insight/{search,eval}.rs` [V eval.rs:499-519]. Local search is NOT a hole.
14. Door safety: default-deny cards, tiers, secret handling with canary tests (`endpoint/src/{tool_capabilities,sandbox,secret_launch}.rs`, A).
15. Crowd: real ORCA in `nav/src/crowd.rs` [V :1-40] (but see A.3 #5: no consumer).
16. Loop: `.opencode/seat-guard.mjs`, `tools/{land,queue-cap,packet-gate,affected,slot}.mjs`. Forge has no seat guard (E12 #12).
17. Crash flight recorder `game/src/crash.rs:557` [V]. Perf: exact budgets default, ms ceilings only in --perf (E09 #13).

### A.2 The 5 biggest MISSES (ideas the engine does not have yet)
1. Read-back and "unknown is not pass": no landed-digest check on door writes, no ABSTAIN anywhere (`grep -ri abstain crates` = 0 hits [V]). 4 audits + census. -> AD-02, AD-04.
2. Stale-capture guard: the frozen check sees identical runs only, not "camera moved, pixels did not" [V frame_check.rs:619 takes digests only]. 3 audits + 9 old incident rows. -> AD-01.
3. Ragdoll tables: one spherical limit on all axes [V ragdoll.rs:1139-1147], no hinge, density 1.0 [V :387-389], Rapier default 4 solver passes [V rapier 0.36 integration_parameters.rs:416]. Forge and old plugin both hold the proven numbers and failure notes. -> AD-16, AD-17.
4. Spatial truths: whole-scene unit mismatch (old plugin: 541 findings from one unit error), 3D/yaw overlap, light reach. All absent [V no "median", no "light" in spatial/src]. -> AD-11, AD-18, N-14.
5. Process scar tissue: no unwired-code gate (BLOCKERS holds 4 "unwired" fingerprints x2-3 [V BLOCKERS.md:9-12, :18-23]), no hollow-green rule (zero tests = RED), no evidence age. -> AD-05, AD-04.

### A.3 The 5 biggest "fix what you already have" defects (all verified)
1. SPATIAL DOOR LIES BY DEFAULT. Every node is a 1 m cube [V scene_tools.rs:490-492, doc :12-14]; the CAP1 apply gate does the same [V capability.rs:24-31, :486-489]; `play.run` uses real colliders [V play/world_load.rs:58-78]. So 7 of the 14 offline checks cannot fire through the door (unknown-position, scale, orphan, giantObject, dupeSpam, sweep, grammar [V checks.rs:196-331 vs snapshot]), 4 nav checks are refused, overlap is XZ-only [V checks.rs:137-165; `xyz_pairs` used by tests only], `scene.query reachable` answers yes for every pair [V scene_query.rs:952-956, :1062-1070], and `SceneSpatialOutput` has no "checks run" list [V :316-333]. "clean" means little. ROADMAP M2 says every AI edit is checked for "blocked door or broken nav link" [V ROADMAP.md M2]: the gate checks overlap and floating only.
2. DOOR WRITES ARE NOT DURABLE, UNDO IS NOT GATED. `tx_apply`/`tx_undo` change memory only [V mcp.rs:843-921]; `apply_persisted` exists and nothing in the door calls it [V grep persist]. Receipt says landed; a crash loses the edit. `tx_undo` checks only that the token equals the newest key, not capability [V mcp.rs:892-921]. `anim.graph.set/undo` is a second write path with its own undo, outside journal and capability [V packs/anim.rs:1-12, :158-174]. The shipped door runs `Capability::unrestricted` and `tx_preview` mints the approval for the same caller [V startup.rs:168-178, mcp.rs:805-837].
3. THE PLAYTEST CRATE HAS NO CONSUMER. `engine-playtest` (36 files, 294 tests) is depended on by nobody [V Cargo.toml grep: only its own manifest]. The AI-door Core proof (HS20, HS29-32 "run through our door") runs on `playtest/src/door.rs`, a provider-contract call plus a private scene executor [V door.rs:1-12, :54-110], not the `e2040-mcp` stdio door. RB1 and LJ1 are "door verbs" whose Owns paths are playtest only [V SPRINT-18-gaps.md:36, :106]; `register_look_judge` is called only from a test [V]. Bug bundles, causal replay, trace summary and seeded-bug gates are unreachable from the door. ROADMAP M2 promises "AI playtest agents drive the headless arena through MCP" [V].
4. VERDICTS THAT CANNOT GO RED. `Verdict.pass = failures.is_empty()` with no check count [V playtest/lib.rs:197-219]; `proof.mjs` has no zero-test rule (a `cargo test -p X filter` that matches nothing exits 0; every FOCUS Now proof is that shape) [V proof.mjs grep]; `log.digest`/`log.query` always return empty because `push_log` has only test callers [V log_tools.rs:255, :423]; import gates read JSON words, not geometry, and have no caller outside assets [V import.rs:200-205, grep]; `rules.set("")` is valid [V tests.rs:303]; `verify_evidence` has 3 copies and zero tool callers [V grep]; gpu_live is report-only [V proof.mjs E12_QUARANTINE].
5. LANDED CODE WITH NO PRODUCTION CALLER, AND DOCS THAT OVER-CLAIM. No outside caller [V grep]: `assets/import_repair`, `observe/rerun_sink`, `RenderHeal`, `anim::lod`, `ai::behavior`, `ai::utility`, `nav::crowd` (ORCA), plus circuit/hedge/bulkhead that exist only inside test files [V circuit_vectors.rs:37]. Docs: `sandbox.rs:3-4` says the pre-run scan "fronts every tool call", only 5 tools use it [V door_dispatch.rs:128 callers]; AI-NATIVE says every what-if can be forked, the door has no fork verb and `tx_apply` ignores `ForkProof` [V mcp.rs:843-887]; ARCHITECTURE lists nav checks in the Check step, the door refuses them (A); README test count and "editor app: not yet" are stale (C1).

---------------------------------------------------------------------------------------------------
## B. TOP 20 ADOPTION ITEMS (ranked by score)

Board IDs in `engine-proposed-rows.md` are AD1..AD20 (same number).

### AD-01  Stale/frozen capture guard + typed ABSTAIN   [score 24.0]
- WHY: a camera-moved/pixels-same frame, or a timed-out judge, must never read as a pass; the Look proof (0 of 10) will be judged by machine on async-readback captures.
- Evidence: old `.agents/memory/state-first-doctrine-and-stale-capture-2026-09-02.md` (same 1,359,084 B frame at poses 26 m apart; "0 is TOOL FROZEN"; timeout = ABSTAIN) [F]; Forge `packs/playtest/CONTRACT.md:11` "Capture confirms only" [F], `eval/eval-runner.mjs:146` ABSTAIN [F]. Engine `playtest/src/frame_check.rs:589-640` `check_digest_window(&[String])` has no pose input [V]; no ABSTAIN in crates [V].
- Status: PARTIAL (identical-run detector HAVE; pose check MISSING; ABSTAIN MISSING).
- ADAPT: the engine owns its capture path, so the receipt carries (tick, camera pose digest, frame digest) by construction. New `FrameFault::Stale` when pose digest moved and frame digest did not; verdict enum gets `Abstain(reason)`. Pixels stay out of the hash (HASH-LAW). Reads only; outside the tick.
- Home: `crates/playtest/src/frame_check.rs` (owner of Frozen) + `look/judge.rs`. Pose field in `game/src/capture.rs` manifest only if absent (UNVERIFIED).
- Proof: pair of poses 26 m apart with equal digests -> Stale; different digests -> ok; timeout/void frame -> Abstain, never Pass; golden unchanged.
- Size S. Depends: none. Audits: E06 #4, E09 #2, C3 G1/G2 + M14, M46, A12, A13, D5, S1, S9. Risk note (mine): EG2 removed blocking readbacks, which raises stale-frame risk.

### AD-02  Landed-digest read-back on every door write   [score 20.0]
- WHY: "an ack is not an outcome"; silent no-op writes and slot-nulling saves cost hours in Flax.
- Evidence: Forge `bridge/Policy.cs:593-664` OutcomeMonitor (contracts, throwing predicate = violated, 2 recovery verbs) [F]; old `material-slot-null-on-save-2026-09-01.md`, `silent-noop-traps-postfx-viewport-2026-09-01.md` (7 slots nulled on 2nd save) (A). Engine: `receipts.rs` ok only on Landed (A); `Applied` has no digests; no read-back in endpoint/editor (A grep); one read-back in `assets/bundle_cook.rs:355` (A).
- Status: PARTIAL.
- ADAPT (prevent, not heal): the service already stages on a clone [V durable.rs:255-298]. Add `before_digest`, `staged_digest`, `after_digest` to `Applied`; refuse `landed_mismatch` if after != staged; typed ops use `deny_unknown_fields` [V component.rs:311] so unknown-field writes cannot silently drop. Add a save -> reopen -> diff-every-ref test (codec round trip exists, no slot case). Digests are of the document, not the sim.
- Home: `crates/editor/src/commands.rs` (Applied), `endpoint/src/{receipts,mcp_dto}.rs` (output).
- Proof: fake adapter that acks but changes nothing -> `no_effect`; second save that nulls one ref fails the diff; receipt shows three digests; run twice equal.
- Size S-M. Depends: none (AD-12 builds on it). Audits: E01 #1, E06 #15, E09 #5, C3 G3/G8 (+ M30, M43, D3, D4, A11, S5, S8). 

### AD-03  Honest spatial door: checks run/skipped, default-size flag, fire tests   [score 19.5]
- WHY: an agent reads "clean" as "all checked". Today half the checks cannot fire.
- Evidence: old `SceneSpatialService.Pairs.cs:91-99` 514-line narrow phase "the wiring never executed" (A, per E03); engine facts in A.3 #1 [V].
- Status: MISSING (output has no `checks_run`; names are synthetic on purpose [V scene_tools.rs:19-21]).
- ADAPT: fix at the producer. `SceneSpatialOutput` gains `checks_run`, `skipped{check,reason}`, `size_source{collider,default}` counts. Non-finite and parentless nodes become findings, not silent drops. One door-level fixture per `SPATIAL_CHECKS` entry must yield a finding or a named skip. Full activation needs AD-07.
- Home: `crates/endpoint/src/scene_tools.rs` + tests. Outside the tick.
- Proof: fire-test table; skipped nodes counted in output; default cubes reported; digests x2.
- Size S. Depends: none (AD-07 completes it). Audits: E03 #3 (+ E03 #1, E13 anti-lessons 2 and 6).

### AD-04  No hollow green: checks_run, zero-tests RED, evidence age, gpu_live not_wired   [score 19.5]
- WHY: every FOCUS Now proof is `cargo test -p X <filter>`; a filter that matches nothing exits 0.
- Evidence: Forge `scripts/fresh-clone.mjs:170-185` HOLLOW_SUITE_RE (c144 burn: exit 0 in 0.1 s counted GREEN) [F]; `eval-runner.mjs` exit 5 on zero tasks (A); `scripts/green-record.mjs` + drift age check (A). Engine `playtest/lib.rs:197-219` [V]; `tools/proof.mjs` zero-count RED only for instruction counts [V :81-92 logic]; board row EG7 already names the same disease for frame-triage [V].
- Status: PARTIAL.
- ADAPT: `Verdict.checks_run`; pass requires > 0. `proof.mjs` parses `test result:`/`running 0 tests` -> RED. `target/proof/<gate>-latest.json` (sha, time) written on GREEN only; drift FAILs a stale record or a sha that is not HEAD/ancestor. gpu_live `live` flag must come from a device receipt, else prints `not_wired`.
- Home: `tools/proof.mjs` (one owner), `crates/playtest/src/lib.rs`.
- Proof: filter matching nothing -> RED; empty `verify()` -> pass=false; 25 h old record -> FAIL; red run does not overwrite the green record.
- Size S. Depends: none. Audits: E06 #5, #12, #13, #8; E12 #7. (+ E06 G-list.)

### AD-05  Unwired-code gate at the packet gate   [score 16.0]
- WHY: five audits and four BLOCKERS rows show writers shipping code nobody calls; judges catch it after the work.
- Evidence: BLOCKERS px36-unwired x3, rt19-unwired x2, fix5-unwired x2, fix3c-unwired-island x2 [V BLOCKERS.md:9-12, :18-23]; no unwired rule in `tools/packet-gate.mjs` (A, E12 #10); my orphan greps in A.3 #5 [V]. Forge rule "wired into its real caller" is words only (A).
- Status: MISSING as a gate.
- ADAPT: `packet-gate.mjs` rule: each new `pub` fn/struct/enum in the diff needs one non-test caller outside its own file, or a consumer row (UNLOCKS). Same tool gets `--orphans` (report only, no deletes) to seed the Section E list (Forge `docs/finish-line/unreachable.txt` idea, C2 U6). No second tool.
- Home: `tools/packet-gate.mjs` (+ test). 
- Proof: seeded fixtures: uncalled pub fn FAILs; called from another module PASSes; UNLOCKS exempts; the four BLOCKERS shapes FAIL.
- Size S-M. Depends: none. Audits: E09 #9, E12 #10, E10 #4, E11 #14, E05 anti 6, C1 gap 3, C2 U6.

### AD-06  One licence table with duties; weights out of git; licence filter in search   [score 19.2]
- WHY: AI generators multiply asset sources; a wrong ship decision is a legal problem.
- Evidence [V]: `gen_intake.rs:31-45` and `provenance.rs:14-31` keep two Rust copies (feature-gated drift test); `clip_intake.rs:17-20` CC0 only; `content/ai-loop/license_gate.py:21-23` ships CC-BY-4.0 and CC-BY-SA-4.0; `tools/gltf-intake.mjs:10-20` CC0 only. No weight-extension rule in `workspace-check.mjs` or `.gitignore` [V grep]. Forge `packs/asset/AssetLicense.cs:97-116` allowlist + stale_after [F]; old `LicenseClassifier.cs:33-45` flags (credit, share-alike, GameSafe) [F]. `ProvenanceRecord` has no credit/duty fields (A).
- Status: PARTIAL (each gate real, they disagree).
- ADAPT: one table in `assets/src/provenance.rs`: SPDX id, may_ship, duties. `check_release` refuses CC-BY without credit text, marks past-due records Stale (never deletes); unknown = not shippable. Repo-wide drift test reads the `.rs`, `.py`, `.mjs` lists. `workspace-check.mjs` refuses tracked `.safetensors/.onnx/.pt/.ckpt/.gguf` unless a digest-pinned fixture (`fixtures/nets/depth_student.e2nn` is one). `SearchQuery` in `editor/src/browser.rs` gets a licence filter (a hidden row also cannot import).
- Home: `crates/assets/src/provenance.rs` (owner), `tools/workspace-check.mjs`, `editor/src/browser.rs`.
- Proof: CC-BY row without credit fails release; SA refused; drift test fails on any list difference; staged `model.safetensors` fails.
- Size S. Depends: Decision 6. Audits: E07 #1, #3, #8; E04 #3; E13 #14; C3 G31, G11; C2 U7.

### AD-07  One scene -> actor adapter with real extents (gate, report, solver, play)   [score 16.0]
- WHY: the AI asks "does X overlap Y" about 1 m cubes while play uses the real colliders.
- Evidence: Forge `Verbs.SpatialRules.cs` guesses the box from scale (A); old "Position without extent cannot determine overlap" (`scene-spatial-report-tooling.md:12-14`, A). Engine [V]: scene_tools.rs:454-499, capability.rs:486-489, world_load.rs:44-95 (the right one), component.rs:309-320 Collider; `editor/src/findings.rs:407-433` has a 4th copy (A); `intents.rs`, `assistant.rs`, `build_level.rs` more copies (A).
- Status: PARTIAL (colliders exist in the schema; only play reads them).
- ADAPT: put the adapter in `crates/editor` (editor already depends on scene + spatial per CRATE-MAP; endpoint depends on editor; so the CAP1 gate in editor and the door both call it, acyclic). My change to E03, which proposed endpoint: the gate cannot call endpoint. Bare node = 1 m default, always labelled `default size` in every report. Arena height filter stays in arena.
- Home: `crates/editor/src/spatial_view.rs` (new), consumers `capability.rs`, `findings.rs`, then endpoint `scene_tools/scene_query/intents/assistant/build_level`.
- Proof: wall collider half [2,1.5,0.1] + crate inside -> overlap reported; play blocks and spatial boxes equal for one document; every caller gets equal rows; spatial digests re-pinned with a named receipt.
- Size M (two steps: editor, then endpoint). Depends: none. Unblocks AD-03 full, AD-11, AD-18, AD-20, N-13, N-14. Audits: E03 #1, E05 #13, E13 #1.

### AD-08  Give the playtest/proof crates a door: bug.replay, trace.summary, verify, replay.verify, and a door-parity test   [score 16.0]
- WHY: the AI-door Core proof and every AI tester currently run beside the door, not through it.
- Evidence [V]: A.3 #3. Forge `packs/playtest/PlaytestPack.cs:44-57, :120-132` returned constant PASS text (A) — the lesson is that proof must call the real thing. Engine `proof/src/bug.rs:188,455` bundle replay (A, file exists [V]); `tests/mcp_stdio.rs:200-376` already spawns the binary (A).
- Status: HAVE as library; MISSING at the door and as a Cargo edge. No board row owns the edge [V grep of rows].
- ADAPT: optional dependency `engine-playtest` (+ `engine-proof` for bundles) behind a feature `verify` on `engine-endpoint` (acyclic: playtest depends on arena, core, provider, render(optional), spatial). Door pack `verify`: read-only verbs, typed, off unless enabled. Add a parity test: the same 20 mirror tasks through `playtest/door.rs` and through the stdio binary must give equal outcome digests, else the benchmark measured a stand-in. Hosts register `look.judge` at startup.
- Home: `crates/endpoint/src/packs/verify.rs` (new), `Cargo.toml`. Existing rows RB1, LJ1, DP1, HS20, AG1 supply the verb bodies.
- Proof: seeded_bug tape bisects to the seeded tick via stdio; parity digests equal; `look.judge` registered in the bin answers non-unsupported on a fixture.
- Size M. Depends: Decision 2; RB1. Audits: C1 gap 3, E06 (integration), E09 #1, E13 #2/#4, E11 #3, E10 #3.

### AD-09  Door session ring with a real writer + `door.status` (health) verb   [score 16.0]
- WHY: the agent's first diagnosis call returns data or says why not; today it returns empty forever.
- Evidence: Forge `bridge/Verbs.HealthSummary.cs`, `scripts/gateway-health.mjs` rules (A); `health.summary` is the first call of the Forge loop (AGENTS). Engine: ring has no production writer [V log_tools.rs:255, :410-424]; `health.summary` is on the retired list [V tools.rs:414-433] while RESEARCH-VERDICT.md:23 says IN (N8) [V]; no `not_wired` answer in endpoint (A).
- Status: PARTIAL.
- ADAPT: `door_dispatch.rs` seal point pushes one typed event per refusal and failed handler; play jobs push end state; new codes go in the one `observe` registry. With no writer, `log.query` answers `not_wired`, never zero. `door.status` (new name; Forge's retired names stay retired): read-only, revision, packs loaded, budget headroom, audit head, last durable generation, build SHA, `unknown` stays `unknown`. 
- Home: `endpoint/src/{door_dispatch,log_tools}.rs`, `packs/view.rs`; `observe/src/lib.rs` codes. Outside the tick.
- Proof: refused `tx.apply` then `log.query` shows its code and tick; fresh session says `not_wired`; status digest equal twice.
- Size S-M. Depends: Decision 3. Audits: E09 #3, #11; E05 #9; C3 G10; E01 #9.

### AD-10  Gate undo by capability; group undo by AI session   [score 15.6]
- WHY: a scoped token can revert a batch outside its scope; "transactional" without a gated inverse is a false promise.
- Evidence: `tx_undo` checks only token == newest key [V mcp.rs:892-921]; assistant undo calls `commands::undo` directly (A). `selective.rs` already does receipt-level undo with conflict reasons [V exists]. Old `Scene.BatchTransactional.cs:98-142` named transactional, no rollback (A).
- Status: PARTIAL (latent while the shipped token is unrestricted).
- ADAPT: `undo_gated` in `capability.rs` checks the inverse ops against mask, scope, budget; human undo with no token unchanged. Group receipts by capability session (SU1 is prose only).
- Home: `crates/editor/src/capability.rs`, `endpoint/src/mcp.rs` hunk.
- Proof: token with INSERT only cannot undo a DELETE batch (`op_not_allowed`, journal unchanged); 20 AI batches + 5 human edits on other objects: session undo restores exact pre-session canonical bytes.
- Size S. Audits: E05 #2, E13 #8.

### AD-11  Whole-scene unit-mismatch detector   [score 19.2, held back by AD-07]
- WHY: AI-made scenes arrive in the wrong unit; one root cause becomes hundreds of useless nudges.
- Evidence [F]: Forge `packs/scene/spatial/SpatialScale.cs:41-60` (MinBoxes 16, red, "fix the scale first"); old `SceneSpatialService.Scale.cs:1-30` (52 actors, 541 overlaps from one unit error). Engine: MISSING [V no "median" in spatial/src]. `import_repair` already refuses unitless files [V exists]. Related: E02 #12 units as types; C3 G24, M21.
- ADAPT: pure function in `spatial` (median object size vs layout extent, population floor 16, root-cause finding ranked first, evidence only, never a rescale). Not added to `SPATIAL_CHECKS` (length pinned at 18); the door calls it first.
- Home: `crates/spatial/src/scale.rs` (new). Integer cm.
- Proof: 52 props x100 -> one red finding listed first; 4-prop scene silent; existing goldens unchanged.
- Size S. Depends: AD-07 for real sizes at the door (the function itself is standalone). Audits: E03 #8, E02 #12, C3 G24/M21 + Forge + old.

### AD-12  Durable door writes with a single-writer project lease   [score 13.0]
- WHY: today an AI edit acked as landed disappears on exit or crash.
- Evidence [V]: mcp.rs:721-731 (rehydrate "no IO"), :843-921; durable.rs:137, :152, :255-298, :333; startup.rs:15, :168-178; no project lock in crates (grep `create_new|flock|lock_file` = 0 hits). Forge `Verbs.Write.cs:207-215` pending-journal marker, loud persist failure (A). Old `concurrent-agent-editor-crash-loop-2026-08-08.md` (two agents, one editor) [F exists], C3 G7.
- Status: PARTIAL (store real and tested; door never calls it).
- ADAPT: compose capability gate + `persist_sealed` with commit-after-seal (the `apply_persisted` pattern). A failed persist is a refusal, never a memory-only success. A lease file on the project folder refuses a second door. IO is outside the tick; document store only.
- Home: `crates/editor/src/{capability,durable}.rs`, `endpoint/src/{mcp,startup}.rs`.
- Proof: kill after ack -> edit present on restart; kill before ack -> absent; forced persist failure -> error and no change; same key after restart returns the stored receipt; second door on the dir refused.
- Size M. Depends: AD-02 (digests), Decision 1. Audits: E05 #1 (+ E13 #10 doc drift, C3 G7/D3).

### AD-13  Rules: commit-class `rules.set`, no-effect refusal, hostile-input property test   [score 14.0]
- WHY: AI-written gameplay rules are the only AI code the engine runs; silent do-nothing rules pass every gate (old plugin: dead pickup compiled clean and passed all gates).
- Evidence: old `script-lifecycle-and-dead-subscription-traps.md:19-36` [F, old repo; E08 mislabelled it Forge]; engine `packs/rules.rs:54-58` cards are `Observe` [V], empty script valid [V tests.rs:303], no `expected_hash`; `dialogue.rs` already refuses DeadNode with line numbers (A). The engine has NO proptest dependency; it uses a hand-rolled std-only matrix engine [V core/tests/property_matrix.rs:1-12].
- Status: PARTIAL (feature `rules` is off by default).
- ADAPT: Commit card for `rules.set`; approval bound to script hash + rules revision; `expected_hash` when one is installed; `NoEffect` refusal when no action fires in any smoke world (explicit no-op flag allowed); vet fuzz reuses the core matrix pattern (no new dependency); mid-round `rules.set` applies at the next tick boundary and enters the tape as ScriptSwap (hash-safe).
- Home: `endpoint/src/packs/rules*`, `crates/rules/src/rules.rs`, `crates/rules/tests/vet_props.rs`.
- Proof: `rules.set` without approval refuses and changes nothing; `fn on_tick(me){let x=1;}` refuses NoEffect; fixed-seed fuzz never panics, every failure typed; reload at tick 300 replays equal.
- Size M. Audits: E08 #4, #5, #6, #9; C3 M41/G29; E13 #5, #13.

### AD-14  Supervisor hardening: digests on the heal tape, verified reset, breaker in src   [score 12.8]
- WHY: heals must be provable and cannot reopen their own budget.
- Evidence [V]: spec rule 1 requires before/after digests (`systems/observability-heal.md` rules); `HealTapeRecord` has none (supervisor.rs:265-280); `reset(code)` takes no proof (:488-495); `register_policy` clears quarantine and escalation (:373-379); circuit/hedge/bulkhead models live only inside test files (circuit_vectors.rs:37, hedge_vectors.rs:17). `RenderHeal`/`DeviceHeal` have no production constructor [V grep]. Forge `retry-proxy.mjs:29-42` shared gate (A).
- Status: PARTIAL.
- ADAPT: add before/after digest to tape rows; `reset(code, VerifiedRecovery)`; re-register keeps the budget unless asked; move breaker (then hedge) into `observe/src/breaker.rs` (std-only, integer, revision/tick as clock); wire provider-timeout and non-finite rows; a heal that changes sim or scene must enter the tape or journal or it does not run (E13 #7). Presentation heals may use wall time.
- Home: `crates/observe`. 
- Proof: chaos row where the heal "applies" but read-back still faults ends in quarantine; re-register does not reopen; the existing vectors run against the src module; tape equal twice.
- Size M. Audits: E01 #2, #11; E09 #7, #8; E13 #7.

### AD-15  Cook-time clip gates: measured foot-skate, retarget coverage, prefix strip   [score 12.8]
- WHY: generated/retargeted clips skate or freeze unmapped joints; the runtime foot lock hides it.
- Evidence [V]: slide_mm lives only in anim (31 hits, 0 in assets); `BoneMap` doc: None rows hold bind pose silently (retarget.rs:262-264), `retarget_clip` returns the clip only (:396-412). CRATE-MAP: assets already depends on anim. Forge `factory/BurstMotion.cs:325-332` PoseGuard 25 mm, caller-supplied numbers [F] (do not copy: looser and unmeasured); Forge `AnimContracts.cs:29-33` index fallback splays (A). Old `bone-get-transform-field-swap.md` (A).
- Status: PARTIAL.
- ADAPT: measure the raw clip with `anim::feet` in the cook; refuse with clip name and mm. `BoneMap::build` returns coverage; refuse when a required set (root, hips, spine, arms, legs, head) is unmapped; strip an explicit prefix list (`mixamorig:`) on both sides.
- Home: `crates/assets/src/retarget.rs`. Cook-time only.
- Proof: planted 3 cm foot slide refuses at cook, still foot passes, negative control FAILs; 65-joint target with 3 unmapped lists those 3; prefixed and plain names give equal bytes x2.
- Size M. Audits: E10 #1, #2, #7; E07 #5; C3 G32, G23.

### AD-16  Ragdoll joint table with hinge kind (feed to PX9)   [score 10.4 — raised by owner priority on ragdoll fixes]
- WHY: a knee bent with the same cone as a shoulder fights the animation; old plugin recorded each wrong number.
- Evidence [F]: Forge `packs/physics/RagdollWizard.cs:60-84` table (spine ball 20/20/35, arm ball 85/85/70, forearm and leg hinge 1/145, thigh ball 70/40/30, head 35/35/45); old `RagdollBodyPlan.cs:90-150` with failure notes (spine 25 locked the torso; forearm 140 missed 5 deg; thigh 45 detonated hips; "bodies without joints fly apart"). Engine [V]: one SphericalJoint, same limit on AngX/Y/Z, from depth (ragdoll.rs:1127-1147, swing_limit). Board row PX9 (open) already names `joint_limits.rs` "from the skeleton instead of depth" but no hinge kind and no numbers [V SPRINT-08:138-148].
- Status: PLANNED (PX9; the file does not exist).
- ADAPT: amend PX9, do not add a second system. Typed `JointKind::{Ball{swing,twist}, Hinge{min,max}}` keyed by humanoid bone names; refuse any limit outside 0..180 deg; refuse `joints != bodies-1` (count from the graph). Rapier is client-local, never hashed [V HASH-LAW.md:92-101], so no golden moves.
- Home: `crates/physics/src/joint_limits.rs`, `ragdoll.rs` spawn.
- Proof: `knee_flex_reaches_145_not_more`; `no_joint_over_limit` over 1,000 seeds; missing-joint skeleton refused.
- Size M. Depends: PX1, PX10 (per PX9). Audits: E02 #2, #8 (+ Forge and old as two source systems).

### AD-17  Ragdoll params builder: mass from a total, solver passes, one CCD policy   [score 15.6 on S-size, shown after AD-16 for sequencing]
- WHY: bones at density 1.0 weigh ~0.04 kg while limbs weigh 3.75 kg; long chains drift on hits; docs and code disagree on CCD.
- Evidence [V]: ragdoll.rs:361-364 `IntegrationParameters{dt, ..Default}`, :374-378 CCD on for every bone, :387-389 density 1.0; lod.rs:144-152 policy "weapons/projectiles only"; physics lib.rs:27-29 repeats it; Rapier 0.36 defaults `num_solver_iterations 4`, pgs 1 (integration_parameters.rs:289-293, :414-416). Forge `RagdollWizard.cs:60-70` Plagenhoef fractions, 80 kg [F]; old `RagdollBodyPlan.cs` "8/4 too low; use 12/8" [F]. Plan row PX10 text says "ccd_enabled 0 hits" (A, stale).
- Status: PARTIAL.
- ADAPT: one params builder shared by `ragdoll.rs`, `powered.rs`, `rapier.rs`; solver passes set and tested on a 12-bone chain (PhysX 12/8 does not map 1:1 to Rapier fields: pick by test); bone density from fraction x total mass, refuse if the sum misses by >1%; one `ccd_for` policy used by spawn; fix doc text. Hash-free (E02 said "re-pin the golden": CORRECTED, ragdoll is not in it). The limb mass table (`limb.rs`) is on the hashed path later (EG46), so share the fraction table only after that.
- Home: `crates/physics/src/{ragdoll,powered,rapier,lod}.rs`.
- Proof: `ragdoll_mass_sums_to_80kg`; spear-like impulse leaves every joint gap <= 5 cm for 300 ticks; ccd policy matrix; `cargo test -p engine-physics --features rapier --lib balance_hash` unchanged.
- Size S-M. Depends: none. Unblocks PX9/AD-16. Audits: E02 #1, #3, #5.

### AD-18  3D, yaw-aware overlap (prerequisite of SP16 stacked placement)   [score 13.0]
- WHY: a lamp on a table is a RED overlap and the repair slides it off the table; "put it on the desk" is the most common placement.
- Evidence [V]: checks.rs:137-165 overlap uses x and z only; `xyz_pairs` (index.rs:151) used by tests only; yaw read only by accessFront (E03, A). Old `SceneSpatialService.Pairs.cs:66-110` Y early exit + oriented box + rotated hits capped yellow (A); old `PlacementSolver.cs` support-top grid search (A). SP16 is open and in FOCUS Now [V].
- Status: MISSING (the 3D pair test exists, the report does not use it).
- ADAPT: one box test for report, solver and gate: 3D interval first, then XZ footprint rotated by yaw (exact at 90 deg; integer bound `(hx+hz)*1415/2000` for 45 deg steps; else bounding circle, yellow). Same function in `place.rs` verify. No new check name, so the 18-length pin holds. Spatial digests on stacked scenes move; flat-room digests do not.
- Home: `crates/spatial/src/{index,checks,place}.rs`.
- Proof: lamp on table clear; crate under table clear; 4 m plank at 45 deg never intersected by the solver; 20 permutations equal; moved digests re-pinned with a named receipt.
- Size M. Depends: none (more useful after AD-07). Blocks SP16. Audits: E03 #2, #5.

### AD-19  Provider status map and a rate-limit gate   [score 11.7]
- WHY: a 400 is retried, a 429 is treated as a spend cap with no Retry-After; a live model in the tick (AL1) hits this first.
- Evidence [V]: `transport.rs:501-512` (401/403/429 -> QuotaAuth, 408 -> Timeout, everything else -> Unavailable) and `protocol.rs:246-251` (Unavailable is retryable); no Retry-After parsing in provider [V grep]. Forge `scripts/retry-proxy/retry-proxy.mjs:8-42` (A), `factory/RouterRetry.cs:20-35` (A).
- Severity corrected to 3: retries share the original deadline and budget [V protocol.rs doc], so the storm is bounded.
- ADAPT: 400/404/422 -> non-retryable; 5xx/408/socket -> capped seeded jitter inside the deadline; 429 -> wait Retry-After once behind one shared gate per provider; spend cap refuses. May need a new `AiError` variant (protocol change; owner call). Skip phone-IP rotation.
- Home: `crates/provider/src/{transport,protocol}.rs`.
- Proof: stub server: 400 once -> zero retries; 429 + Retry-After 2 with 20 callers -> one wait; 503 x2 then 200 -> two retries within deadline; cassette replay equal.
- Size S-M. Audits: E01 #3, C2 U1.

### AD-20  Reach honesty at the door, then one nav adapter   [score 10.0]
- WHY: "can the guard reach the gate?" must never default to yes.
- Evidence [V]: scene_query.rs:952-956, :1062-1070. `scene.spatial` already refuses nav checks with `needs_nav_view` (checks.rs `run_spatial` returns it) [V]. Six reach/BFS homes (nav tiles/flow/mesh, spatial sight.rs and navcheck.rs, playtest reachability.rs, build_level.rs:1319; A). Root cause I found: playtest has no dependents, so endpoint rebuilt its own BFS (see AD-08).
- Status: PARTIAL.
- ADAPT: step 1 (S): `reachable` refuses `needs_nav_view` when no bake exists. Step 2 (M): one adapter nav mesh/tiles -> `ReachView`/`NavView` in endpoint (spatial stays nav-free); build_level reads the same view.
- Home: `endpoint/src/scene_query.rs`, `nav/src/mesh.rs` as source.
- Proof: two rooms, one door: blocked across the wall with a view, refusal without; mesh and tiles agree on 20 seeded layouts.
- Size S then M. Audits: E03 #4 (+ E13 #1: reach exists but is not in the gate).

---------------------------------------------------------------------------------------------------
## C. NEXT 30 (ranked, short)
Format: id | name | audits | size | status | home | one-line proof.

- N-01 | Import gates read mesh bytes (edge use counts, tri count, UV overlap); "deferred" never counts as pass | E07 #2, C3 G4 | M | PARTIAL, no caller outside assets [V import.rs:183-208] | assets/import.rs | open-boundary glTF whose text has "indices" FAILs watertight.
- N-02 | Per-object preconditions in a batch (disjoint edits both land) | E05 #7, E13 #6 | M | MISSING | editor/commands.rs, scene | two disjoint batches from one base land in either order to equal bytes. Needs Decision 7.
- N-03 | ForkProof required for balance-affecting door batches | E13 #3, C1 gap 7 | M | PARTIAL (assistant gates, door does not) | editor capability + endpoint assistant | balance breaker refused at the door.
- N-04 | tx.preview requires an agent-stated intent; host declares client+model; apply returns the witness receipt | E05 #4, #5 | S | PARTIAL [V mcp.rs:503-533, door_dispatch.rs:293-322 stand-in] | endpoint/mcp_dto, startup | empty intent refuses; receipt source=Model.
- N-05 | Gate-order table test + gate trail in receipt | E05 #8 | S | PARTIAL | endpoint/door_dispatch tests | each refusal class names one gate and writes one audit record.
- N-06 | Crash hook in `e2040-mcp` (reuse `game/src/crash.rs`) | E09 #4 | S | MISSING [V install_run_hook only in arena bin] | endpoint bin | forced handler panic writes a valid report naming the last door call.
- N-07 | Registry digest covers tool schemas; verb metadata (use_when/not_when/success) in api.describe; pack-verb count cap | E05 #10, #11; C3 G14, G17; C2 U4 | S | PARTIAL (pin covers 26 core tools only [V tool_count_pin.rs:1-12]) | endpoint/tools.rs, packs/mod.rs | change one schema field -> digest changes.
- N-08 | Real MCP client acceptance receipt (list, preview, apply, undo; digests before/after undo) | E05 #12, C1 | S | MISSING | tools/ + docs/plan/ai-native.md | receipt file digests match run output.
- N-09 | Fingerprint ledger with owners; fixed holds only while later runs lack it; repeat 2x with no owner blocks | E09 #6, E01 #9, Forge `editor-log-triage.mjs:1-23` [F] | S-M | PARTIAL (rule in prose) | tools/ + BLOCKERS format | NEW code fails the gate; FIXED that returns becomes REOPENED.
- N-10 | One HMAC seal owner in engine-core; wire `verify_evidence` into proof.mjs; retire 2 copies | E09 #10, E06 #3 | M | PARTIAL [V 3 copies, 0 tool callers] | core/seal_key.rs, proof/a9.rs | edit one byte in target/evidence -> proof exits non-zero.
- N-11 | One door-owned ring for net/render/audio heals; wire `RenderHeal`/`DeviceHeal`; `RerunSink` in the arena run | E09 #7, #12 | M | PARTIAL [V no constructors] | observe, render/present, arena | seeded desync names its tick in `log.query`.
- N-12 | Typed suppression/verdict ledger (reason, expiry revision, visible flag; closed verdict enum) as a scene record written by command | E01 #15, E03 #6, #13 | M | MISSING [V no suppress/waive in spatial] | scene/provenance.rs, spatial input list | expired allow stops suppressing; malformed record refused. Decision 5.
- N-13 | Support propagation bottom-to-top (stack on a floating base) | E03 #9 | S | PARTIAL (one hop) | spatial/checks.rs | floor, A, B -> 0 findings; A raised to 3 m -> findings for both.
- N-14 | Light reach per renderable (any light in range counts; unknown range = unknown) | E03 #10, E11 #8 | M | MISSING [V 0 "light" in spatial/src; data at component.rs:345-357] | spatial new module | farther light covers a point the nearest misses.
- N-15 | Brief: name families, focus(id), top-down map as advisory text | E03 #7 | M | PARTIAL | spatial/brief.rs | 115-object fixture under budget; map equal twice.
- N-16 | scene.spatial returns per-finding proposals (Repair preview or Refused + conflict set) | E03 #12 | S | PARTIAL (editor only) | endpoint/scene_tools | every finding yields a preview or a refusal.
- N-17 | Run the 100 spatial tasks with a real model through the provider (GoldAgent stays as the oracle) | E03 #11; AG1 | M | PARTIAL | playtest/spatial_eval.rs | gold agent scores 100%; "clean to everything" scores the known count; unknown brightness fails.
- N-18 | Stuck ladder (detect, bounded replan with backoff, bug tape) + level soft-lock verifier | E02 #7, E13 #12, E01 #13 | S-M | PARTIAL | ai/gladiator.rs, playtest | wall fixture escapes within N ticks; frozen level raises stuck at the same tick x2.
- N-19 | Motion sanity labels (fall_through, orbit, stationary, vertical_drift, isFlying) + two samples before a motion claim | E10 #3, C3 G9 | S | MISSING [V 0 hits in playtest] | playtest | six seeded tapes give six labels.
- N-20 | Supported premise: body/controller with no collider below (skip ragdoll bones by parent) | E02 #6 | S | PARTIAL | spatial/rules.rs | floating body gives one finding; 12-bone ragdoll none.
- N-21 | Judge caps in code (blank, pink, dark, no result line, run errors); model score advisory only | E11 #3, E06 #14 | S | PARTIAL | playtest/look/judge.rs | flat capture scores <= 2 even if a stub model says 10.
- N-22 | Brief-to-game: 3 different briefs each pass numeric bars; phase order; refuse with choices | E11 #1, #2 | L | PARTIAL (one fixed Roman slice) | endpoint/build_game.rs + playtest | each package replays x2; tie refuses with choices. Decision: Roman only vs fixture games.
- N-23 | Integer ability/cooldown/threat store in rules (ticks, sorted tags, no float) | E11 #5 | M | MISSING | rules host | cooldown set at tick 10 for 90 is ready at 100, not 99.
- N-24 | BT executor with Running + depth refusal + decision trace; retire or wire `behavior.rs`/`utility.rs` | E11 #6 | M | PLANNED (GA11) | ai | Running for 5 ticks resumes on tick 6, x2.
- N-25 | Local search bundle: live crate index (fingerprint cache under target/), reply ceiling + nextCursor, health row for any optional dense backend, outcome memory per error code | E04 #7, #4, #2, #5 | M | PARTIAL [V index seeded from packs only, mcp.rs:683-690] | endpoint/insight | door finds `ObjectId` with no eval call; dead backend shows in the answer.
- N-26 | Loop bundle: dirty-tree guard, one claims parser and TTL, `queueMove` guard, red-build record | E12 #1-#4 | S+S+S+M | MISSING [V no dirty-guard in tools; queueMove writes `(read(from) ?? "") + extra` loop-keeper.js:1272-1279] | tools/, seat-guard.mjs; keeper fix goes to center's source | seeded repo: 7 h old unstaged file FAILs; missing source writes nothing.
- N-27 | Units as newtypes (Metres, Millimetres, Kilograms) at controller/walker boundaries | E02 #12, C3 G24 | S | PARTIAL | core units module | mixing units fails to compile.
- N-28 | Semantic render buffers (object-ID, depth, normal, per-object light hits) from the CPU twin | E13 #9; board EV21 | L | PLANNED partly (EV21 asks for entity-id per region) | render twin + playtest/look | ID pixel counts equal analytic projection.
- N-29 | Physics read-only query verbs (contact depth, support, foot-lock) over integer state | E13 #11 | M | HAVE in crates, not on the door | endpoint pack | same tape -> same records twice.
- N-30 | Hot reload of data/assets as a tape event at a tick boundary | E13 #13, E08 #9 | S | PARTIAL | app/rules tape | reload at tick 300 replays equal; off-boundary refused.

---------------------------------------------------------------------------------------------------
## D. FULL DEDUPED CATALOG (everything not in B or C)
Format: audit-finding | one line | status/verdict. Duplicates merged; supporters in brackets. Lines marked (A) were not re-opened by me.

### D.1 Heal / fix (E01, C2)
- E01-4 repair planner: pure plan, measured moves first, pile push-out ordered by volume, planner-gap kind | HAVE `spatial/repair.rs`; add push-out + gap kind (S) (A).
- E01-5 loop caps by budget not timer | HAVE `build_level.rs:1702-1739` MAX_REPAIRS 3 (A); keep.
- E01-6 import janitor: explicit units, two-poll stability, last-good | HAVE `assets/import_repair.rs`, `recook.rs`; import_repair has no outside caller [V] (see E list).
- E01-7 crash-safe log; failed rollback must say rollback_failed | HAVE `editor/wal.rs`; E01's "no live app" rested on stale README (corrected); folds into AD-12.
- E01-8 save heal: migrate in memory, persist only on explicit call | HAVE `save/fallback.rs` (A); add the rule (S).
- E01-10 heals respect other writers; doctor is report-only | HAVE `tools/doctor.mjs` (A); rule: no process restart without lease check.
- E01-12 cassette every AI fix in CI | HAVE `provider/cassette.rs` (A); use it.
- E01-14 ref janitor only on opted-in objects, never null a loaded ref | MISSING rule (S).
- C2-U2 session quarantine + bounded audit ring for gameplay AI guards | PARTIAL `ai/guard.rs` (A); ADAPT in tick order (S).
- C2-U3 durable long jobs: submit/status/collect/abandon, resume not retry | PARTIAL (provider `JobState`) (A); M.
- C2-U8 cook preflight + build-twice identical-hash gate | PARTIAL (S-M).
- C2-U11 status words for half-built work ("Parked: way back") | ADOPT in docs (S).

### D.2 Physics / ragdoll (E02)
- E02-4 echo text must be computed; refusals typed (not_wired, headless) | rule (S).
- E02-9 per-fighter sever budget with rest | PARTIAL `combat/cut.rs` caps per swing (S).
- E02-10 landing tier from fall height and speed | MISSING `balance.rs` (S).
- E02-11 rope/chain rules (anchored root, cut removes a joint) | MISSING; `tie.rs` planned (M).
- E02-13 one fighter controller, one push order; `kcc.rs` has no caller [V grep] | S; Decision 10.
- E02-14 cited constants keep their source | S.
- E02-15 NaN never simulates | HAVE; add seeded NaN fuzz on ragdoll step (S).

### D.3 Spatial (E03)
- E03-14 camera resolver for one target; one view test, not three | PARTIAL (M).
- E03-15 door clearance asks the agent radius; bake refuses radius/height outside bounds | PARTIAL (S-M).
- E03 anti: several rulebooks for floating (5 cm / 100 cm / 2 cm in Forge) | keep one threshold table.

### D.4 Local search (E04, C3)
- E04-1 lexical base with measured gate; optional backends must not change ranks | HAVE gold floor; add off-rule test (S).
- E04-6 no-model hashed dense leg in integer counts | MISSING; LATER; after the gold floor is measured leg-off (M).
- E04-8 scene.query | HAVE (doc lines scene_query.rs:4-5 stale) (S).
- E04-9 synonyms and typo tolerance | PARTIAL (S).
- E04-10 bounded dependency walk | PARTIAL (M).
- E04-11 tiered NPC memory recall | PARTIAL `ai/memory.rs` (S).
- E04-12 generated hot-file section in repo map; E04-13 byte cap on doc map | S each.
- E04-14 per-leg time budgets | SKIP now.
- C3-G15 repo search index; G26 "not found" names the roots scanned | PARTIAL / MISSING (S).

### D.5 Agent interface (E05)
- E05-3 anim graph second write path | move onto journal as a component op (M) [V A.3 #2].
- E05-6 tier/policy/token are constants; destructive_hint never set; fail-closed policy load; breaker after N refused removes | M (A).
- E05-14 hostile-text screen covers 5 tools; run on tx.preview/tx.apply text and fix sandbox comment | S [V].
- E05-15 intent -> verb recorder for tool-choice accuracy | S (A).
- C3-G6 idempotency fingerprint conflict | already HAVE (correction).
- C3-G18 refuse writes while a play run is live; G27 refuse modal prompts; G28 read-repeat cap | S each (A).

### D.6 Verification (E06)
- E06-1 replay compares and negative control | HAVE `rules/tape.rs:418-463` (A); keep as test pattern.
- E06-2 planted bug per class | HAVE `playtest/seeded_bug.rs`; wire into proof (S).
- E06-6 bot files replayable bug bundles | HAVE (A).
- E06-7 reachability before look | HAVE `playtest/reachability.rs` (A).
- E06-8 gate that checks the stub it prints | rule; covered by AD-08 parity.
- E06-9 narrowed run unions owners; unowned file widens | HAVE shape `tools/affected.mjs`; test the unowned rule (S).
- E06-10 print golden scope in every PASS line | S.
- E06-11 F2P/P2P enforced by tool (parent commit must FAIL) | M; Decision 9.
- E06-14 see N-21.

### D.7 Assets / PCG (E07, C3)
- E07-4 power-of-two + normal-map convention as explicit cook input | S.
- E07-6 one style record for AI assets | S.
- E07-7 quarantine as a registry state the cook refuses | S.
- E07-9 generated-prop intake refuses licence/model first | HAVE `gen_intake.rs` [V]; keep.
- E07-10 layout compiler: validate grammar first, <= 3 repairs, typed codes, never in tick | M.
- E07-11 generate -> score -> retry chain, max 3, deterministic gates only | S.
- E07-12 difficulty curves offline over tournament sweeps | M.
- E07-13 content-root / symlink check on asset reads (also E08-13 ContentPath) | S.
- E07-14 brief-to-game PASS = log clean + result line + capture bytes > 0 | S.
- E07-15 "ported" = one row in EVIDENCE.md vs AGENTS rule 4 | docs (S); Decision 6.
- C3-G4 asset must-load (texture refs decode) | folds into N-01. C3-G11 never fabricate a hash; G12 stage-validate-install | S / M (A).

### D.8 Code safety (E08)
- E08-1,2,3,7,8,15 sandbox by construction, default-deny door, typed service, no shell, secrets, cite-or-refuse | HAVE; keep. Promote the canary redact check out of `redteam.rs` into a production invariant (S).
- E08-10 Forge write gate slips (lesson only) | SKIP scanner.
- E08-11 extend banned-API clippy list (Command::new, fs::write, env::var, net) in sim crates | S.
- E08-12 three keyword-screen lists; one table + legal-lines false-positive corpus | S.
- E08-13 shared typed ContentPath | S.
- E08-14 host calls not charged to the op budget (`nearest_foe` scans all fighters) | S.

### D.9 Observability (E09)
- E09-8 observe core real and honest | HAVE; one registry table generating the door catalog (S).
- E09-13 perf budgets: open an observe Span per budget run | S.
- E09-14 door audit chain weak hash, no read verb; add `audit.tail`, pin head outside the log | S-M.
- E09-15 telemetry-free by construction; add a deny.toml check | S.
- E01-9 / E09-6 → N-09.

### D.10 Animation (E10)
- E10-5 animation LOD built, never called | wire in presentation loop (M) [V no outside caller].
- E10-6 three blend paths, two foot locks; one inertializer | M.
- E10-8 impact frame is the constant `IMPACT_TICKS = 12` (A); read from clip event track in presentation | M.
- E10-9 two trauma tables; `combat/feel.rs` juice table unused | S.
- E10-10 state-machine lints | HAVE.
- E10-11 rig quality report with measured flags | M. E10-12 decal recycle oldest-first | S. E10-13 AI clip verbs only after AD-15 | M. E10-14 cinematic tracks | M.
- E10-7 AI motion source (CT14) | L; licence list in refusal.

### D.11 Gameplay kits (E11)
- E11-4 typed round-result record | S. E11-7 ORCA exists, needs a consumer (NV4) | correction. E11-9 `RungController` has no caller | S. E11-10 HFSM snapshot inside GA11 | M. E11-11 camera occlusion/retreat rig, presentation only | S. E11-12 traversal chooser not called by arena | S. E11-13 Rome spec single machine copy + drift gate | S. E11-15 save keyed seal only if tamper evidence wanted | S.

### D.12 Loop / workspace (E12)
- E12-5 drift judges the working tree; count staged/HEAD, WIP prints WARN | S. E12-6 breaker counts timeouts per crate | S. E12-8 redispatch cap in the packet file | S. E12-9 helper fingerprint breaker (second identical failure ends the run) | M. E12-11 STOP must gate batch send | S. E12-12 seat guard | HAVE. E12-13 land refuses diff over max_diff | S. E12-14 held rows wake after 6 h | S. E12-15 move stale ready packets | S.

### D.13 Native (E13)
- E13-4 shrink a failing tape (ddmin); RB1 `replay.bisect` is the board row | M. E13-10 tool list in tokens with a pinned tokenizer; generate schema/doc/test stub from one table | S-M; Decision 11. E13-15 crate lanes + measured wait | S.

### D.14 Old-plugin incident rows (C3 §2, 103 rows)
- COVERED already: M5, M9, M17, M20, M24, M29, M38, M39, M44, A2, A10, D2, D7, D11, T2, T4-T7, T11, T12, S3, S4, S6, S11.
- Mapped to B/C: stale frame (M14, M46, A12, A13, D5, S1, S9) -> AD-01; read-back (M30, M43, A11, D3, D4, S5, S8) -> AD-02; dead subscription M41 -> AD-13; units M21 -> AD-11/N-27; G7 lock M13/T9 -> AD-12; hero frames D9/G25 -> AD-01 note.
- ADOPT/ADAPT small: M6 (known-pose per bone read), M33/T8 (not-found names roots), M35/T7 (writes during play), M40/T3 (modal prompt), M27 (read-repeat cap), M19/M28/R1 (must-load), M47/W1 (ack-only worker reply), A9 (two-phase delete), S13 (ledger-first dispatch).
- SKIP (Flax-only or process): M1-M4, M7, M10-M12, M15, M16, M18, M23, M25, M26, M31, M34, M37, M42, M45, M48-M51, A3-A9 (dupes), D1, D6, D8, D10, T1, T3 (Flax part), S10, S12, S14, S15.

---------------------------------------------------------------------------------------------------
## E. FIX-WHAT-YOU-HAVE (engine defects found in the engine; all [V] unless marked)
| # | Defect | File:line | Harm | Smallest fix | Size |
|---|---|---|---|---|---|
| 1 | Door sizes every node as a 1 m cube; ignores world scale and colliders | endpoint/scene_tools.rs:490-492; editor/capability.rs:486-489; findings.rs:407-433 (A) | overlap/floating answered for the wrong boxes; 7 checks dead | AD-03 output now, AD-07 adapter | S then M |
| 2 | Reach is open floor | endpoint/scene_query.rs:952-956, :1062-1070 | wrong "reachable: yes" | refuse `needs_nav_view` | S |
| 3 | Overlap XZ-only; `xyz_pairs` unused | spatial/checks.rs:137-165; index.rs:151 | lamp on table = RED; repair moves it sideways | AD-18 | M |
| 4 | Door writes not durable | endpoint/mcp.rs:843-887; durable.rs:272 unused | edit lost, receipt said landed | AD-12 | M |
| 5 | Undo ignores capability | mcp.rs:892-921 | scoped token reverts anything | AD-10 | S |
| 6 | Shipped door unrestricted; preview mints the apply approval for the same caller | startup.rs:168-178; mcp.rs:805-837 | no human consent anywhere | Decision 1 | S |
| 7 | Second write path for anim graphs (process-global static table) | packs/anim.rs:158-174 | own undo, no capability, no journal | component op on the journal | M |
| 8 | Provenance always `script:mcp`; intent is a stand-in string | mcp.rs:503-533; door_dispatch.rs:293-322 | cannot tell AI from script | N-04 | S |
| 9 | `log.*` ring has no production writer | log_tools.rs:255, :423 (test) | tools answer empty forever | AD-09 | S |
| 10 | `Verdict` has no check count; `proof.mjs` has no zero-test rule | playtest/lib.rs:197-219; proof.mjs | hollow green | AD-04 | S |
| 11 | `verify_evidence` x3, zero tool callers | proof/a9.rs:636; playtest/eval.rs:1480; seeded_bug.rs:416 | seal protects nothing | N-10 | M |
| 12 | gpu_live flags are literals; leg is report-only | proof.mjs E12_QUARANTINE | a claim, not device evidence | AD-04 | S |
| 13 | Import gates read words; no outside caller; `import_repair` no outside caller | assets/import.rs:183-208 | passes meshes it never read | N-01; wire import_repair | M |
| 14 | map_status retries 400/404/422; 429 = spend cap | provider/transport.rs:505-511 | wasted retries; no Retry-After | AD-19 | S-M |
| 15 | Supervisor: no tape digests; reset without proof; re-register reopens budget | observe/supervisor.rs:265-280, :373-379, :491-495 | a heal loop can reopen itself | AD-14 | M |
| 16 | Breaker/hedge/bulkhead exist only in test files | observe/tests/circuit_vectors.rs:37; hedge_vectors.rs:17 | the MCP door has no breaker | AD-14 | M |
| 17 | `RenderHeal`, `DeviceHeal`, `RerunSink` have no production constructor | grep | heals tested, never run | N-11 | M |
| 18 | `rules.set` card is Observe; empty script valid | packs/rules.rs:54-58; rules/tests.rs:303 | live script install skips the commit gate | AD-13 | M |
| 19 | `sandbox.rs` says the scan fronts every call; 5 tools use it | sandbox.rs:3-4; door_dispatch.rs:128 | hostile text reaches tx.apply | wire to tx.* or fix comment | S |
| 20 | Four licence lists disagree | gen_intake.rs:31-45; license_gate.py:21-23; clip_intake.rs:17-20; gltf-intake.mjs | CC-BY passes Python, fails Rust | AD-06 | S |
| 21 | Crash hook only in the arena bin | game/bin/arena.rs:175 | door panics leave no report | N-06 | S |
| 22 | Orphans: `anim::lod`, `ai::behavior`, `ai::utility`, `nav::crowd`, `physics::kcc` | grep (no outside caller) | looks like a feature, is not | AD-05 `--orphans`, then wire or park | S |
| 23 | Ragdoll: density 1.0; one cone for all axes; Rapier default 4 passes; CCD docs vs code | physics/ragdoll.rs:361-389, :1139-1147; lod.rs:144-152; lib.rs:27-29 | wrong impulses, bent knees, drift | AD-16, AD-17 | M |
| 24 | Playtest crate has no consumer; HS20 "through our door" runs on `playtest/door.rs`; `look.judge` has no host | Cargo graph; door.rs:1-12; view.rs:127; look_judge_hook.rs:62 | Core AI-door proof measures a stand-in | AD-08 | M |
| 25 | Doc drift: ROADMAP M2 claims (blocked-door/nav checks on every AI edit; playtest through MCP), ARCHITECTURE nav checks in Check step, AI-NATIVE "every what-if forked", `book/ai-door.md` "exactly twenty", README test count (1,220 vs 6,217 static) | docs | agents plan against claims the code does not meet | fix text or build the feature; add to AD-05 doc check | S |
| 26 | Tool-count pin covers 26 core tools; E05 says ~45 verbs reachable and the pin test may be red with `--features mcp` | tests/tool_count_pin.rs:1-12 | UNVERIFIED (not run) | run once; add pack-verb cap | S |

---------------------------------------------------------------------------------------------------
## F. ANTI-LESSONS (do NOT copy), deduped
1. Flat tool surface and mega routers: old 1,174 tools / 74 routers / 404 cases; Forge 177 pack ops behind `ext.call` [E05 A1, C3, arch-review].
2. Pass by default and constant success text: Forge 1,445 `PackResult.Pass` vs 12 `NotWired`; `PlaytestPack.cs:45,73,132,181`; `InputReplay.cs:31-50` [E05, E06, arch-review].
3. Deny lists as the boundary; allow-list default off (`CSharpDenylist.cs:21`); lists disagree (`ExecGuard.cs` vs `CodeScriptSafetyScan.cs`) [E08, E13].
4. Allowlisted interpreter = arbitrary code (`GatewayExecDirect.cs:133-134`); self-declared exemptions (generated-file marker; riskAccepted) [E08].
5. Placeholder "fixes" that pass checks (`SceneRepairService.Repair.cs:229-262` box brush; SceneDirector checker-grid fallback) [E01].
6. Kill-by-default editor restarts (`rebuild-and-relaunch.ps1:12-14`) and kill-by-pattern [E01].
7. IP rotation to beat quota (`retry-proxy.mjs:22-23`) [E01].
8. Learning a fix from adjacency in time (`FixLog.cs:30`); learn only from verified before/after digests [E01].
9. Plan reported as applied (`StuckRecovery.cs:58-59`, `SceneRepairOps.cs:101-104`, `AssertPass` constant) [E01].
10. Unit-cube stand-in without a flag: the engine's own `scene_tools.rs:490` [E03].
11. Several rulebooks for one fact (floating at 5/100/2 cm); camera rule in 4 homes [E03, arch-review].
12. A stage that never ran (514-line narrow phase) and gates nothing calls (`ToolBudgetLedger`, `ConfirmCheck`, `TimingSafeBearerEq`) — the engine has the same disease in `transport.rs`, `check_dry_run_id`, `receipts::Receipt` [E03, E04, E05].
13. Caller-supplied numbers as gates (`PoseGuard` 25 mm) and a flag that is not a measure (`poseGuard:true`) [E10].
14. Index-order fallback that splays bodies; string event bus; guessing rig family from names [E10].
15. Flax editor state: cm units, GUID endianness, kinematic toggle order, startup-only solver switch, compile latch and focus gate, modal prompts, viewport overlay as truth, editor-only gates [E02, E06, E08, E09, C3].
16. Zero as success; "transactional" with no rollback; log file as truth (missing log = zero errors; UTF-16 read as UTF-8; defaulted fps 60) [E09, E13].
17. Stub labelled "live" (promptfoo); proof that re-implements the product (PowerShell FNV); breadth as proof (391 gate hits) [E06].
18. Parking a flake in a lane that never fails — the engine's own `proof.mjs` quarantine list ("game netgame UDP loopback") [E06]. Give each parked test an owner and a due row.
19. Calendar sunsets as design (`RagdollAutofit.cs:11-12`) [E02].
20. Substring licence matching ("Standard License" read as ND); empty licence defaulting to CC-BY [E07].
21. Weights, venvs, node_modules, vendored plugins in git (1.78 GB) [C3, E07]. Persona/profile store in a repo; cookie-scraping lanes [C3].
22. Memo zoo, copies of one function (hashed embed x3), hand-curated API index with 60 of 2,258 entries [E04].
23. Starting daemons from a tool path (`flaxmcp-embeddings.mjs:71-111`) [E04].
24. Keeper copied into each repo (3,131 lines) [V loop-keeper.js]; lanes/worktree pools retired but code remains; CPU caps that hide load; two queues for one resource [E12].
25. Text-edit machinery (CRLF healing, replaceAll guards) for a string-replace tool [E08].
26. Plugin family churn and a 742-file namespace rename [C3, arch-review].
27. Keyword router for briefs; model-scored judge; result by log text; object pool and event bus; plan-only UI ops; a second rule language; genre folders [E11].
28. Dead weight: alias/pointer packs, `lab/` zoo, "Folded" leftovers, `dev-infra` dumping ground, committed run reports [C2].

---------------------------------------------------------------------------------------------------
## G. NEW GOLD (10 ideas; all YOUR IDEA = the architect's synthesis)
Each is grounded in evidence from the reports and marked with what proves or kills it.

G1. UNIVERSAL REPLAY BUNDLE. YOUR IDEA. One bundle (tape, rules hash, content digests, first divergent tick, expected metric) is the receipt for every failure class: spatial finding, stuck fighter, dead rule, desync, bad heal, fork-proof miss. Evidence: `proof/src/bug.rs:188,455` bundle replay (A), `supervisor.rs:261-264` heal rows are copied into the action log so replay matches [V], `ai/mind.rs` offline replay (A), RB1 verbs. Prove: plant 5 defect classes; each reproduces from one bundle by one verb to a bit-equal first divergent tick. Kill: any class needs a second verb or is not bit-equal.

G2. DOOR PARITY TEST (stand-in detector). YOUR IDEA. Run the same 20 mirror tasks through `playtest/door.rs` and through the spawned `e2040-mcp` stdio binary; outcome digests must match. Evidence: door.rs:1-12 [V], HS20 "through our door" [V], `tests/mcp_stdio.rs:200-376` already spawns the binary (A). Prove: equal digests. Kill: any mismatch means the benchmark measured something else; the mismatch itself is the finding.

G3. HEAL TAPE -> FIX MEMORY -> PREVENTION CANDIDATES. YOUR IDEA. Fingerprint = (code, scope, cause digest). A repeat ranks the fix that worked last time (E04 #5), and a second repeat files a "prevention row proposal" (a validator candidate) instead of another heal. Evidence: BLOCKERS fingerprints x2-3 [V], supervisor tape [V], Forge FixLog adjacency flaw (A), E12 #9. Outside the sim, never decides authoritative repairs (spec rule 6). Prove: 20 seeded faults, top-1 recovery accuracy >= 80%, no fix learned from adjacency. Kill: below 80% or any adjacency learning.

G4. SCAR -> VALIDATOR REGISTRY WITH MUTATION PROOF. YOUR IDEA. Every scar (2048-byte floor, joint count = bodies-1, unit mismatch, CC-BY credit, dead rule, stale frame) becomes one named validator in the command service with: scar source, a bad fixture, and a door-level fire test; a meta-test mutates one input per validator and demands red. Evidence: 7 of 14 checks cannot fire through the door [V], "plan step names a gate that does not exist" (E07 anti A5), old "every lint rule needs a bad fixture" (C3 G5), planted-bug-per-class (E06 #2). Prevention by construction, not after-the-fact. Prove: 100% of validators have a reachable fire test. Kill: >10% do not, or mutation finds a silent validator that stays green.

G5. SEMANTIC HEADLESS BUFFERS AS THE FRAME-TRUTH CHECK. YOUR IDEA (links E13 #9, EV21, E03 #10, E06 #4). The CPU twin emits object-ID, depth and per-object light-hit lists; "the frame is stale" and "this prop is dark" become exact queries, and the pose-moved/ID-map-same case cannot hide. Evidence: EV21 row [V], Light.range in the scene [V component.rs:345-357], CPU twin modules (C1, A). Prove: ID pixel counts equal analytic projection counts; a planted occluder changes visibility. Kill: CPU twin costs more than the capture budget or diverges from the GPU frame on the fixture.

G6. FORK-PROOFED NIGHT SHIFT. YOUR IDEA (revives parked BAL1 under a gate). When no match runs and the GPU is idle (arbiter), a local model proposes balance/level tweaks; each goes through preview, then a ForkProof over a declared seed suite with sample count and interval; auto-accept only if every band holds and nothing regresses, else queue for a human. Evidence: `ForkProof` [V assistant.rs:720], `provider/arbiter.rs` (A), tournament ladder (A), SMOOTH-BUILD seed-suite rule (A), AL1 row [V], BAL1 parked [V SPRINT.md]. Prove: of 3 seeded regressions and 1 improvement, only the improvement lands, x2. Kill: any regression accepted in 100 trials.

G7. CONFLICT-PREDICTING CLAIM PLANNER (30 parallel writers). YOUR IDEA. Before dispatch, compute reverse-dependency closure of each packet's paths (`tools/affected.mjs` exists) and refuse two writers that share a crate or both touch a hot shared crate (`core` has ~30 dependents; `endpoint` is the top sink). Evidence: CRATE-MAP [V], "max 1 builder per crate" [V SPRINT.md], LICGATE tree conflicts x2 [V BLOCKERS], FIX3 140-path drift [V]. Prove: replay the last 200 landed packets: predicted-conflict precision/recall vs actual. Kill: precision below 70%.

G8. MEASURE, THEN SPLIT `endpoint`. YOUR IDEA (small crates for cheap rebuilds). `crates/endpoint` is 65 files / 55,625 lines [V count] and the top sink; 8 of the 20 items in Section B land in it, and the board allows one builder per crate, so they serialize. Evidence: counts [V], file sizes mcp.rs 2,838, assistant.rs 4,126, build_level.rs 3,459 [V]. Do NOT split before measuring: record median incremental `cargo check -p engine-endpoint` time and the number of rows blocked per week by the one-builder rule. Prove/kill: split into door-core + packs only if rebuild exceeds the agreed budget AND at least 2 rows a week wait on the crate lock.

G9. VERDICT AS A TYPE. YOUR IDEA (prevention by construction for honesty). `Evidence = Pass{checks: NonZeroU32} | Fail(..) | Abstain(Reason) | Unknown` in `engine-core`, no `Default`, no `From<bool>`. "Zero checks is a pass" and "timeout is a pass" become unrepresentable. Evidence: `Verdict{pass: bool}` [V playtest/lib.rs:201], `gate_results -> [(String,bool);7]` [V import.rs:183], `verdict: String` in `SceneSpatialOutput` [V], 1,445 Pass vs 12 NotWired (arch-review, A). Prove: convert 5 verifiers; the conversion surfaces at least one latent hollow pass. Kill: it finds none, or breaks >20% of tests without a bug found.

G10. RECALL BY PROVENANCE. YOUR IDEA. If a licence problem surfaces after the fact, one verb lists every scene object descended from the asset (provenance parent receipt ids) and emits one selective-undo batch that names objects pinned by later human edits. Evidence: `scene/src/provenance.rs:1-5` parent receipt ids [V], `commands/selective.rs` conflicts [V exists], `assets/provenance.rs` release check (A), old `LicenseClassifier` flags [F]. Prove: 20 seeded dependency graphs: all dependents found; pinned objects named. Kill: any dependent missed.

---------------------------------------------------------------------------------------------------
## H. SEQUENCING (mapped to FOCUS "Now" and ROADMAP)

Engine order (FOCUS.md "## Now"): 1 Look, 2 Bodies, 3 AI door, 4 Engine depth, 5 Multiplayer last; groups 4 and 5 run beside 1-3; a held crate's coder takes the next row in another crate. ROADMAP milestone that fits: M2 "AI is a usable capability" (exit: one external MCP client, 20 fixtures, replay without a model), then M3 slice. Constraint: one builder per crate, 4 code builders per wave.

### Next 2 weeks (small, hash-free, different crates; run in parallel)
| Lane/crate | Item | Why now | FOCUS |
|---|---|---|---|
| tools | AD-04 (proof.mjs zero-test, evidence age), AD-05 (packet-gate unwired), weights check from AD-06 | cheap, stops fake green and dead code for everyone after | all |
| playtest | AD-01 (Stale + Abstain), then Verdict.checks_run | Look proof is judged by machine; EG2 raised stale risk | 1 Look |
| spatial | AD-11 (pure fn) | standalone, S | 4 |
| assets | AD-06 licence table + drift test | S; needs Decision 6 first | 4 |
| physics | AD-17 params/mass/CCD | S-M, hash-free, unblocks PX9 and AD-16 | 2 Bodies |
| endpoint (serial) | AD-03, then AD-09, then AD-20 step 1 | all S; keep them in one queue | 3 AI door |
| editor | AD-10 undo gate | S | 3 |
| provider | AD-19 | S-M; needs the variant call | 3 |

### Next milestone (M2 exit)
Order inside endpoint/editor (serial): AD-07a (editor adapter) -> AD-07b (endpoint consumers) -> AD-02 -> AD-12 -> AD-13 -> AD-08 (+ RB1, LJ1, DP1, parity test). Parallel elsewhere: AD-18 (spatial) then SP16; AD-16 (physics, PX9); AD-14 (observe); AD-15 (assets). Then N-01..N-12.

### Later
N-13..N-30, Section D items, G-ideas that need measurement first (G5, G8). Multiplayer (FOCUS 5) is untouched by this plan except AD-12's lease and AD-14's net heal ring.

### What blocks what
- AD-07 blocks the real value of AD-03, AD-11, AD-18, AD-20, N-13, N-14.
- AD-02 blocks AD-12 (persist what you verified).
- Decision 2 (Cargo edge) blocks AD-08, RB1/LJ1 being reachable, HS20 honest, G2.
- Decision 3 blocks AD-09 name; Decision 6 blocks AD-06; Decision 1 blocks AD-12/AD-10 defaults; Decision 7 blocks N-02.
- AD-17 -> AD-16 (PX9 needs PX1, PX10; fix PX10's stale "CCD off" text first).
- AD-18 blocks SP16 (an on-top object must not be RED).
- Endpoint is the bottleneck: 8 of 20 items live there; see G8 before adding more writers.
- No item touches `GOLDEN_600`; AD-18 and AD-07 move spatial digests (named receipt); AD-14/AD-13 may add tape rows (new versioned fixture, not the legacy one).

---------------------------------------------------------------------------------------------------
## I. OWNER DECISIONS (14: 1-12 first pass, 13-14 second pass; recommendation first)
1. DECIDED yes (owner, chat 2026-10-11). Door consent and default tier (E05 Q1, E08 Q1). Today the stdio door is unrestricted and preview mints its own approval [V]. Recommend: host-set `Capability.tier` default 1 (no removes), AI-driven `rules.set` is a Commit-class call, human yes only for removes and rule installs. Keeps agent loops fast, closes the worst hole.
2. DECIDED yes (owner, chat 2026-10-11). May `engine-endpoint` depend on `engine-playtest` (and `engine-proof`) behind feature `verify`? Recommend yes: acyclic, no new crate, and no other row owns the edge. Alternative (new thin crate) adds a home; against the one-home rule.
3. DECIDED yes (owner, chat 2026-10-11). Health verb: reverse the retired `health.summary` decision (RESEARCH-VERDICT N8 says IN [V]) under a new name `door.status`, unknown stays unknown. Recommend yes.
4. DECIDED yes (owner, chat 2026-10-11). Size truth (E03 Q1): the scene Collider is the one size source for report, gate, solver and play; bare nodes flagged `default size`. Recommend yes; "clean" will change meaning for existing scenes, say so once.
5. Layout intent (E03 Q2, E01 #15): deliberate overlaps as typed scene records written by command (reason, expiry revision, visible count). Recommend yes, coarse (pairs + may-float names), after AD-07.
6. Licence policy (E07 Q1-3, E13 Q2): recommend CC0 and CC-BY (credit duty enforced, credits file generated), refuse SA/NC/ND/unknown at release; one table; Python/JS gates call or are retired. Also resolve `.gitignore:7` ("all content usable") vs `content.md:288` and whether AGENTS rule 4 covers copyleft code (recommend: ideas only, never vendored).
7. Per-object revisions (E13 Q1): let disjoint AI batches both land; document revision stays the clock. Recommend yes, only after AD-02 and AD-12.
8. "HAVE needs a production caller" (E10 Q1): recommend yes; split plan status columns into implemented / tested / integrated; enforce with AD-05.
9. Evidence standard (E09 Q2, E06 Q1-2, E12 Q3): recommend HMAC seal everywhere (hmac and sha2 already in playtest/proof), GPU rows `not_wired` until a device receipt exists, F2P enforced by tool on rows that carry an F2P id, evidence age per bar (golden: HEAD sha; perf: 72 h).
10. Ragdoll scale and controller (E02 Q1-3): recommend 80 kg Dempster/Plagenhoef split for bones (no golden impact, ragdoll is client-local [V HASH-LAW]), CCD on for every dynamic bone with one policy fn, `controller2` as the owner for anything hashed and park `kcc.rs` (no caller). UNVERIFIED which controller arena locomotion calls (E02 says `controller.rs:1429`).
11. Search and local models (E04 Q1-3, E13 Q3): recommend live search stays game-agent first (already), crate index only as a `target/` cache for engine developers; lexical first, hashed dense leg only after the leg-off gold floor is measured; models fetched at run time, never in git; keep the 43,514-char tool budget until a pinned tokenizer is chosen.
12. Loop ownership (E12 Q1-3): recommend one driver (`sprint-loop.mjs` as a thin launcher over the keeper, or the reverse), keeper fixes made once in center's source and copied, one cargo queue with one timeout and one log.
13. Fork before apply (D-AX19, plan only; section J). The fourth audit wants a `tx.fork` verb: run the existing assistant fork test on a copy of the scene and refuse a balance breaker before the apply. The map from a scene batch to an arena variant is UNKNOWN: which scene fields change a fight (weapon stats, arena size, spawns)? Recommend: the owner names that map first; until then N-03 (ForkProof for balance-affecting door batches) is the only fork row.
14. Tolerant arguments (A-T2-10, plan only; section J). Forge accepts nested args, stringified JSON and extra keys, and says what it normalised. The engine refuses unknown fields (AD2: `deny_unknown_fields`). Recommend: keep the strict door; AD26 lists every bad field with a pointer, so one retry fixes them all.
Folded with default answers: WASM second runtime (E08 Q2) = no, keep Rhai; abilities/threat (E11 Q3) = integer state in `rules`, no new crate; BT shape (E11 Q2) = GA11 in Rust, data trees only with a load-time validator; brief scope (E11 Q1) = Roman first, fixture games only as proof; foot-skate bar (E10 Q2) = one 1 cm bar at cook; motion source (E10 Q3) = hand-made/CC0 first; Eval bar (E03 Q3) = 80% with unknown never a pass, mock/local/cassette only.

---------------------------------------------------------------------------------------------------
## J. SECOND PASS (owner 2026-10-11)

Owner ask: "all gold and smart ideas in the engine; AI native from the core: AI can measure and know everything, scene, spatial, reasoning." Four research passes found 43 candidate board rows and 47 smaller ideas. This section merges them, puts the best 14 on the board and keeps the rest here, ranked. On the board (`SPRINT-18-gaps.md`, section "Forge and old-plugin adoption"): AD21..AD29, PX40, AX23, EA6, PF24, SP66. Nothing here moves GOLDEN_600 `d57b5c8ae3096614`.

Sources and tags: A = Forge gold the first audit missed (rows A21..A35, smaller ideas A-T2-1..A-T2-23). B = old Flax plugin gold (rows B41..B52, smaller ideas B-Cnn). C = public repos and the MCP spec (rows C61..C70). D = a fourth audit with native ids (D-AX18, D-AX19, D-PX40, D-SP61, D-EA6, D-PF21). A tag is the researcher's own number with a letter in front: A21 is not board row AD21.

Licence legend (no line below is a port; no code was taken from anywhere):
- IDEA-F = idea from Forge (the owner's repo), rewritten small in Rust.
- IDEA-O = idea from the old Flax plugin (MIT, Flax Game Studio), rewritten small in Rust. Exceptions are named on the line.
- IDEA-G = idea from a public repo or spec named on the line, with its licence; rewritten small in Rust.
- IDEA-A = idea from the fourth audit of this repo.

### J.1 How the 14 were chosen
- The gain must be "the AI can measure or know something it could not", not "more tools".
- A real caller must exist or be named (CODELAW: no code without a caller).
- Hash-free: read-only verbs, eval code, tools, or output outside the tick.
- Small first: one session where possible; a chain of rows where not.
- Ready now: only AD23 (playtest) and PX40 (endpoint). Every other new row waits behind an earlier row in its crate (one builder per crate) or behind an owner decision. The map is in J.2.
- Endpoint is now the bottleneck twice over: 10 of the 14 new rows touch it, so 18 of the 34 rows on the board do. G8 (measure the endpoint crate, then split it) matters more now.

### J.2 The 14 boarded rows: why, evidence, what was cut
Evidence marks as in section A: [V] I opened the engine file or ran the grep this pass; [F] a Forge file named by researcher A; [O] an old-plugin file named by researcher B; [S] the MCP spec as read by researcher C.

- AD21 claim check (A21). Why: the agent's report is the weak point; a preview can be called "landed". Evidence: Forge `bridge/NrpGate.cs:27-41` CheckVerdict, wired at `Dispatcher.Inner.cs:468-493` [F]. Engine: no claim check [V grep -w claim: two unrelated comments]. Cut: Forge's fix-edge and correlation-id rules (the engine has no such model). Waits for AD2 (digests) and AD3 (checks_run).
- AD22 playable is six named gates (A29). Why: "playable" must be six facts, and "not wired" must read FAIL. Evidence: Forge `eval/claim-gate.mjs:5,32-52` [F]. Engine: `tools/proof.mjs` has no `not_wired` and no playable verdict [V]. The gates read records AD4 writes and the `tools/ship.mjs` receipts. Cut: Forge's gate wording; the engine names its own tools.
- AD23 trajectory checks (A22). Why: invented ids and skipped reads are the commonest AI tool mistakes. Evidence: Forge `eval/eval-asserts.mjs:101-141` [F]. Engine: no order or grounding check in `crates/playtest/src` [V]. Ready.
- AD24 capture visibility (A30). Why: a good frame does not prove the subject is in it. Evidence: Forge `bridge/ReceiptProof.cs:52-67,98-120` [F]. Engine: AD1 has the pose digest and Abstain, no subject state [V frame_check.rs reads digests only]. The view-truth rows SP61-SP65 (landed on the board while this pass ran) already compute a screen rect and cover per body, so AD24 reads them and writes no second projection; it waits for SP62. Cut (as A cut): `zero_scale`, `no_mesh`. Occluded is never guessed; it waits for SP65 `view_sees`.
- AD25 scene IQ eval (A35). Why: the way to score an AI's scene sense without letting it grade itself. Evidence: Forge `eval/scene-iq.mjs`, `scene-iq-sim.mjs` [F]. Engine: `spatial_eval.rs` has 100 place, query and fix tasks, no undo, camera, light or do-nothing control [V]. Last of the chain.
- AD26 refusals teach (A25 + B43 + B44 + C62). Why: today every refusal leaves as an RPC `invalid_params` error with a code and a message; the model gets no advice code, no retry rule, no next call, and sees one bad field at a time [V `discover.rs`: `McpToolError` to `RpcError` to `ErrorData::invalid_params`]. The `Receipt` type has a retry flag and two recovery verbs, but only `tests/redteam.rs` builds one (section F.12) [V]. Evidence: Forge `bridge/WireEnvelope.cs:6-17,61-72,98-113` [F]; old `ErrorRemediation.cs`, `SuggestedActionBuilder.cs`, `ValidationErrorEnvelope.cs` [O]; MCP tools spec: tool errors belong in the result with `isError`, protocol errors in RPC errors [S]. Cut: A24's run-time scorer stays plan only; its table test ("every next call is a live verb") is in the AD26 accept line. One merged row, not four: one table, one home.
- AD27 playbook prompts (B41). Why: the old plugin served debug recipes as MCP prompts; a weak model reads a recipe instead of guessing. Evidence: old `Prompts/*.md` (95 files), `PromptCatalog.cs` [O]. Engine: `get_info` advertises tools only [V]. Start with ten recipes. Warning from the old repo: some playbooks named verbs that never existed, so the accept line checks every named verb against the live registry.
- AD28 `scene.why` (C64). Why: "who moved this and can I undo it" is the first question after a surprise. Engine: the journal and `parent_receipts` exist (`commands.rs:986`, private); no verb reads them [V].
- AD29 snapshots as MCP resources (C61). Why: reads served as resources cost nothing from the 43,514-char tool-list budget [V `mcp.rs:149`]. Evidence: MCP resources spec [S]; Forge serves `forge://map` as a resource [F, local].
- PX40 `physics.read` (D-PX40). Changed shape; see J.6 item 4. The pure balance facts go on the door now; the live per-slot and bone half is plan only.
- AX23 say what changed (D-AX18). `scene.place` already uses the semantic diff (`scene_tools.rs:2428`); `tx.apply` does not [V]. The row gives `tx.apply` the same list and adds `scene.diff(rev_a, rev_b)`.
- EA6 capability token from the tool cards (D-EA6). `build_server` mints `Capability::unrestricted` for every caller [V `startup.rs`]. Waits for AD10 and owner Decision 1.
- PF24 wall time on play receipts (D-PF21). Opt-in, outside every hash; the `play.rs` header promises "no wall clock", so the default output must stay byte-identical.
- SP66 light reach (D-SP61; plan N-14). `Light.range` is stored in `crates/scene/src/component.rs` and no spatial query reads it [V]. Unknown range answers unknown, never dark. SP64 (props and light in the view truth) reads the render side; SP66 reads the scene's `Light` components, so they share no code.

Order on the board: ready now AD23, PX40. AD2 then AD26 then AD29 then AD27. AD2 and AD3 then AD21. AD2 and AD20 then AD28. AD2 and AD7 then AX23. AD3 then PF24. AD7 then SP66. AD4 then AD22. AD1 and SP62 then AD24. AD10 then EA6 (and Decision 1). AD7, AD23, AD24, AD28 and SP66 then AD25.

### J.3 Ranked catalog of ALL candidates (90 lines)
Rank = gain for "AI can measure and know" times cheapness, by judgement; boarded lines first. Format: rank | tag | what | licence | board id or plan only.

- R01 | A25 | closed advice code, retry flag and next call on every refusal | IDEA-F | AD26 (merged row)
- R02 | B43 | one recovery table, suggested next call that names live verbs | IDEA-O | merged into AD26
- R03 | B44 | all bad arguments in one reply, JSON pointers, redacted hints, never auto-fix | IDEA-O | merged into AD26
- R04 | C62 | refusals as tool results with `isError` (MCP tools spec) | IDEA-G (spec) | merged into AD26
- R05 | A21 | claim check: refuse a claim the receipt does not back | IDEA-F | AD21
- R06 | D-PX40 | `physics.read`: balance facts on the door (pure part now) | IDEA-A | PX40
- R07 | D-AX18 | `tx.apply` change list and `scene.diff(rev_a, rev_b)` | IDEA-A | AX23 (AX18 was taken)
- R08 | C64 | `scene.why(object, field)`: last writer from the journal | IDEA-G (rr, record and replay debugger, idea only) | AD28
- R09 | A30 | capture says whether the subject is visible; omit, never guess | IDEA-F | AD24
- R10 | C61 | read-only snapshots as MCP resources | IDEA-G (MCP spec) | AD29
- R11 | B41 | playbook prompts served as MCP prompts | IDEA-O | AD27
- R12 | A29 | playable is six named gates; not_wired is FAIL | IDEA-F | AD22
- R13 | A22 | trajectory checks: call order and grounded ids | IDEA-F | AD23
- R14 | A35 | scene IQ eval: the agent never grades itself | IDEA-F | AD25
- R15 | D-SP61 | light reach: any light in range lights a point; unknown stays unknown | IDEA-A | SP66 (SP61 to SP65 are taken)
- R16 | D-EA6 | capability token built from the tool cards | IDEA-A | EA6
- R17 | D-PF21 | wall time (p50, p99, labelled software) on play receipts | IDEA-A | PF24 (PF21 was taken)
- R18 | C63 | lenient scene reads: found items plus per-item errors | IDEA-G (Bevy `bevy_remote`, MIT and Apache-2.0, idea only) | plan only
- R19 | A26 | tool-search margin floor: top two within 5 percent refuse and name both | IDEA-F | plan only
- R20 | A32 | no silent drops: swallowed-error counters and store generations in `door.status` | IDEA-F | plan only (after AD9)
- R21 | C67 | component schema verb with units and ranges | IDEA-G (Bevy `registry.schema`) | plan only
- R22 | A34 | undo and rollback fence; creation ops need a remove inverse | IDEA-F | plan only (extends AD10)
- R23 | B50 | stuck classifier with integer ratios and a last-progress tick | IDEA-O | plan only (feeds N-18)
- R24 | B47 | receipt search on the engine's own BM25 | IDEA-O | plan only
- R25 | A23 | seeded one-shot fault; the final answer must name the code | IDEA-F | plan only (after AD23)
- R26 | B45 | tool-description lint (oversell words, destructive names its preview) | IDEA-O | plan only
- R27 | C65 | door stateful fuzz: random verb sequences with invariants and shrink | IDEA-G (Hypothesis stateful API; its licence is UNVERIFIED) | plan only
- R28 | C66 | `tick.schedule` read verb: systems in run order | IDEA-G (Bevy `schedule.list`) | plan only
- R29 | A27 | accept lines as data, with a parser fuzz corpus | IDEA-F | plan only
- R30 | A28 | designed refusals per lane and failure family | IDEA-F | plan only
- R31 | A33 | queue then drain for writes during a live run | IDEA-F | plan only
- R32 | B46 | per-scene edit lease; play start refused while leased | IDEA-O | plan only (after AD12)
- R33 | C68 | seat-scoped observation: hidden state refused | IDEA-G (TextArena, MIT) | plan only
- R34 | C69 | `test.list` and `test.log` beside `test.run` | IDEA-G (mcp-unreal, Apache-2.0) | plan only
- R35 | C70 | human verdict row in the trace export | IDEA-G (Blender MCP, MIT) | plan only
- R36 | A24 | recovery names a real next call: run-time scorer | IDEA-F | plan only; the table test is inside AD26
- R37 | A31 | strict known-fail list: a listed test that passes is red | IDEA-F | plan only (test-only in Forge)
- R38 | B42 | exemplar bank: few-shot rows only from green gate receipts | IDEA-O | plan only
- R39 | B51 | per-session token budget in the provider | IDEA-O | plan only (with AD19)
- R40 | B49 | integer rectangular assignment solver | IDEA-O (published 1987 method) | plan only
- R41 | B52 | cited constants table; refuse an uncited number | IDEA-O | plan only (feeds AD16, AD17)
- R42 | B48 | seeded overlapping wave function collapse | IDEA-O; the DeBroglie licence is UNVERIFIED, so reimplement the published method | plan only; check `crates/world` against GOLDEN_600 first
- R43 | D-AX19 | `tx.fork`: run the assistant fork test on a copy before apply | IDEA-A | plan only; blocked on Decision 13
- R44 | A-T2-1 | ground unknown: no floor or terrain means floating is not judged, y=0 said out loud | IDEA-F | plan only (detail for AD3, N-20)
- R45 | A-T2-2 | support rules: hanging is not floating; shallow vertical overlap is resting | IDEA-F | plan only (detail for AD18, N-13)
- R46 | A-T2-3 | read-handle expiry after 9 steps with a re-ground note | IDEA-F | plan only
- R47 | A-T2-12 | brief: worst-first problems (cap 5), tallies, 40x20 top-down text map | IDEA-F | plan only (detail for N-15)
- R48 | A-T2-11 | camera framing: the AI names the target, the engine computes eye and look-at | IDEA-F | plan only (same as E03-14)
- R49 | A-T2-5 | per-reply readiness line (ready, playing, importing) | IDEA-F | plan only
- R50 | A-T2-8 | identical-retry refusal by error class | IDEA-F | plan only (detail for E12-9)
- R51 | A-T2-7 | seeded task variants so an agent cannot recall answers | IDEA-F | plan only
- R52 | A-T2-13 | write gate: busy answer with retry hint, caller identity per write | IDEA-F | plan only (detail for AD12, AD10)
- R53 | A-T2-14 | one argument validator for dry run and live | IDEA-F | plan only (detail for N-04)
- R54 | A-T2-20 | saved refusal shapes replayed offline | IDEA-F | plan only (detail for AD8)
- R55 | A-T2-22 | atomic write: symlink confine, temp plus rename, before-hash compare | IDEA-F | plan only (detail for E07-13, AD12)
- R56 | A-T2-23 | receipts-only central audit ring | IDEA-F | plan only (detail for E09-14)
- R57 | A-T2-21 | clone and stale-port findings; unknown stays unknown | IDEA-F | plan only (detail for AD12)
- R58 | A-T2-19 | golden seed drift severity: breaking vs safe | IDEA-F | plan only (HASH-LAW)
- R59 | A-T2-9 | rerun failed tests once; fail then pass is a labelled flake | IDEA-F | plan only (detail for AD4; `tools/flaky-quarantine.mjs` exists)
- R60 | A-T2-10 | tolerant arguments: nested, stringified JSON, extra keys | IDEA-F | plan only; blocked on Decision 14
- R61 | A-T2-4 | editor blocker taxonomy (reload, modal, compile, play, focus) | IDEA-F | plan only (needs an editor sensor)
- R62 | A-T2-6 | cost per success and token goodput per task | IDEA-F | plan only (unwired in Forge)
- R63 | A-T2-15 | eval limits object: time, tokens, calls, cost | IDEA-F | plan only (low)
- R64 | A-T2-16 | two-model eval merge with a per-model 95 percent bar | IDEA-F | plan only (low)
- R65 | A-T2-17 | live editor pin: env, doc, single editor, else refuse with choices | IDEA-F | plan only (low)
- R66 | A-T2-18 | research cap per row | IDEA-F | already have (`tools/research-cap.mjs`, cap 3)
- R67 | B-C21 | collider vs visual size ratio and zero-size collider finding | IDEA-O | plan only (after AD7)
- R68 | B-C22 | combat gaps: hit location, backstab, aggro, loot, tether, shove, hitstun, crit | IDEA-O; the old team cited the hit table before writing it, so design ideas only | plan only
- R69 | B-C15 | seeded simulated annealing over integer vectors | IDEA-O | plan only
- R70 | B-C24 | camera presentation stack (offset stack, impulses, group framing, spring arm) | IDEA-O | plan only (presentation only)
- R71 | B-C25 | failure taxonomy shape: symptom, fix, evidence rule | IDEA-O | plan only (feeds G3)
- R72 | B-C13 | MarkovJunior, erosion, cave and dungeon generation | IDEA-O; MarkovJunior licence UNKNOWN, reimplement the published idea only | plan only
- R73 | B-C23 | audio clip analysis (BPM, onset, beat, pitch, loudness) | IDEA-O | plan only
- R74 | B-C32 | workflow fan-out templates and named stress probes | IDEA-O | plan only
- R75 | B-C16 | Kabsch rigid fit | IDEA-O | plan only (cook or presentation; floats stay out of the tick)
- R76 | B-C17 | marching squares contours | IDEA-O | plan only
- R77 | B-C18 | Perlin 2D noise | IDEA-O | plan only
- R78 | B-C19 | R-sequence quasirandom points | IDEA-O | plan only
- R79 | B-C26 | scope relatedness signal (intent vs changed paths) | IDEA-O | plan only (low)
- R80 | B-C27 | production DAG compiler | IDEA-O | plan only (low)
- R81 | B-C34 | log shaping and noise filter | IDEA-O | plan only (low; detail for N-25)
- R82 | B-C35 | external change scanner for scenes | IDEA-O | plan only (UNKNOWN; `assets/src/watcher.rs` exists)
- R83 | B-C11 | reciprocal rank fusion | IDEA-O | plan only (only behind a flag with the gold floor re-measured; E04-1)
- R84 | B-C36 | physics config lints: CCD on kinematic, gravity units, dynamic concave mesh | IDEA-O | folded into AD17 and N-27
- R85 | B-C10 | determinism pinning loop | IDEA-O | folded into AD27 recipes
- R86 | B-C28 | hidden stub tools | IDEA-O | folded into AD4
- R87 | B-C29 | compiled exposure and catalog | IDEA-O | folded into N-07
- R88 | B-C30 | screenshot freshness gate | IDEA-O | folded into AD1 and AD24
- R89 | B-C31 | why-is-it-dark diagnostic | IDEA-O | folded into N-14 and SP66
- R90 | B-C33 | weights placeholder marker | IDEA-O | folded into AD6

### J.4 Mapping: source tag to board id
Boarded: A21 to AD21. A22 to AD23. A29 to AD22. A30 to AD24. A35 to AD25. A25, B43, B44 and C62 to AD26 (merged). B41 to AD27. C64 to AD28. C61 to AD29. D-PX40 to PX40. D-AX18 to AX23. D-EA6 to EA6. D-PF21 to PF24. D-SP61 to SP66.

| Tag | Board id | Tag | Board id | Tag | Board id |
|---|---|---|---|---|---|
| A21 | AD21 | B41 | AD27 | C61 | AD29 |
| A22 | AD23 | B42 | plan only | C62 | AD26 (merged) |
| A23 | plan only | B43 | AD26 (merged) | C63 | plan only |
| A24 | plan only (table test inside AD26) | B44 | AD26 (merged) | C64 | AD28 |
| A25 | AD26 (merged) | B45 | plan only | C65 | plan only |
| A26 | plan only | B46 | plan only | C66 | plan only |
| A27 | plan only | B47 | plan only | C67 | plan only |
| A28 | plan only | B48 | plan only | C68 | plan only |
| A29 | AD22 | B49 | plan only | C69 | plan only |
| A30 | AD24 | B50 | plan only | C70 | plan only |
| A31 | plan only | B51 | plan only | D-AX18 | AX23 |
| A32 | plan only | B52 | plan only | D-AX19 | plan only (Decision 13) |
| A33 | plan only | | | D-PX40 | PX40 |
| A34 | plan only | | | D-SP61 | SP66 |
| A35 | AD25 | | | D-EA6 | EA6 |
| | | | | D-PF21 | PF24 |

Everything not in this table (A-T2-1..A-T2-23, B-Cnn) is plan only; J.3 gives each its line.

### J.5 The biggest "AI cannot know this yet" gaps (5, plus physics)
1. Real shapes. The door sizes every object as a 1 m cube, so "does the lamp overlap the table" and "is it lit" are answered for stand-ins. Rows: AD3 (say so in the output), AD7 (real extents), AD18 (3D overlap). Second pass: SP66 (light reach, after AD7), AX23 (edges, after AD7), AD25 (the eval that catches a stand-in).
2. Did the write land? An ack says landed, but there is no after-digest, a crash loses the edit, and undo is not gated. Rows: AD2, AD10, AD12. Second pass: AD21 (refuse a claim the receipt does not back), AX23 (change list and digest pair), AD28 (who wrote this field), EA6 (token from the cards); plan only: A34 (undo fence).
3. Can it reach? `scene.query reachable` answers yes for every pair. Rows: AD20 (refuse without a nav view, then one adapter), AD7. The second pass adds no board row here; plan only: A-T2-1 (ground unknown), B-C21 (collider ratio). Add a reach task to AD25 when AD20 step 2 lands.
4. Pixels and confidence. The door has no look scorer and no ABSTAIN, so a timed-out judge can read as a pass. Rows: AD1 (Stale and Abstain), LJ1, AD8 (a door for the verifiers). Second pass: AD24 (the capture says whether the subject is in view; omit, never guess), AD25; plan only: N-21 (judge caps).
5. What was checked. A verdict carries no check list, `scene.spatial` names no checks, and "clean" means little. Rows: AD3, AD4. Second pass: AD22 (six named gates; not wired is FAIL), AD26 (a refusal says what to do next), AD23 (trajectory checks), AD21.
6. Physics state is not on the door. `sim.observe` has hp, stamina, a phase word, position and yaw; it has no balance, gait or bones. PX40 puts the pure balance facts on the door now. The live half (per-slot balance, bone velocities, bone contacts, joint drift) is plan only: the door's bout runs without the balance rules and no bout holds a ragdoll (see J.6 item 4).

### J.6 Facts found in the second pass
1. The `explain.rs` header: the research said it claims "not wired". At HEAD it already names its real callers (`check_batch` at `mcp.rs:851` and `scene_tools.rs:902`); commit b6897fc1 fixed it [V]. Nothing left to do. One nit: its last line still says "eighteen-tool discovery budget".
2. Tool-list budget: the code says `TOOL_LIST_BUDGET_CHARS = 43_514` at `mcp.rs:149` [V]. Decision 11 in section I already says 43,514. The old number 38,944 is still in `docs/plan/ai-native.md` lines 61, 114 and 204, and line 61 cites `mcp.rs:126` (the constant is at line 149 now). This pass could not edit that file; fix those three numbers and the cite when someone does.
3. Ids already taken [V grep over docs, tools, crates]: AX18 and AX19 (LADDER research notes "AX18 detail proposed", "AX19 detail proposed"), SP61 (a LADDER spike when the audit was read; then commit a1dfeb83 landed SP61-SP65 as real board rows, the view-truth family, so SP64 and SP65 are taken too) and PF21 (a ticked row in `docs/archive/SPRINT-2026-10-05.md`). PX40 and EA6 are free. Ids were re-checked on 2026-10-11 after that commit. So D-AX18 is AX23, D-SP61 is SP66 and D-PF21 is PF24; D-AX19 would need AX24 if it is ever boarded.
4. PX40 changed shape. The audit asked for live bone velocities, bone contacts "with an integer impulse", joint drift and a balance verdict over the live bout. In code [V]: the door's bout (`GymLive`) builds with no balance rules (`balance` is off in `ArenaRules::V1`; the per-slot `BalanceState` is private at `arena/src/lib.rs:1006`); the ragdoll appears only in `crates/physics` and its tests; `BoneContact` has only the bone pair (`a`, `b`), no impulse. So the row is the pure part (`response_matrix` and `recovery_ticks` are integer and need no bout) and the live part answers `not_wired`. The live part needs a Bodies row that puts a ragdoll in the bout and turns the balance rules on for door bouts (check that no door digest moves).
5. The `play.rs` header says "no wall clock". PF24 keeps the default output byte-identical and puts timing behind a flag.
6. `Receipt` in `receipts.rs` has a retry flag and two recovery verbs, but only `tests/redteam.rs` builds one; the door never does (section F.12) [V]. AD26 reuses the type's shape or retires it; it adds no second type.
7. Wire shape: AD26 changes how every refusal reaches a client. The researcher could not confirm whether clients show RPC error text to the model; the first step of AD26 is to record what the stdio client shows for both shapes. Existing tests that read a refusal as an RPC error will move to the result shape; I did not count them.
8. View truth overlap. The loop boarded SP61-SP65 (headless view truth, `view.observe` and `view.laws`, GPU draw-id numbers, props and light in the brief, `view_pick` and `view_sees`) while this pass ran. They own "what the frame shows as numbers". AD24 therefore reads SP62 instead of projecting again; AD21's `viewed` claim accepts a `view.observe` read; AD25's top-down and camera tasks can read the same rows. Two more loop rows overlap older ones: GS1 (a wiring scan of existing islands) does the `--orphans` half of AD5, while AD5's diff rule in `packet-gate` stays (GS4 also edits `packet-gate`, so those two serialize); GS30 (isovist) is near SP65 `view_sees`. One home per job: the first to land owns it and the other drops its half.

### J.7 Not checked in this pass
- No build, test or run. Absence claims are greps of `crates`, `tools` and `docs` on 2026-10-11.
- The Forge, old-plugin and public-repo claims were not re-opened unless marked [V]. Unknown: whether Forge calls `claim-gate.mjs` on a live path (researcher A); the licences of Hypothesis and Unreal MCP (C); DeBroglie and MarkovJunior (B); whether `crates/world` is inside GOLDEN_600 (B48 must check first).
- Whether door bouts can run the balance rules without moving a door digest was not checked.

---------------------------------------------------------------------------------------------------
## K. HONESTY BOX

### Not covered / sampled
- No audit built or ran anything; test counts are static `#[test]` counts (C1: 6,217). "Compiled" and "passes" were not checked by anyone, including me.
- C1 plan statuses came from two Explore censuses (not re-checked line by line). C3 read no C# file whole; E04 and E05 sampled old memory notes by grep; E06 did not read FINISH-PROOF tiers or most playtest files; E10 read `feet.rs` in part; E12's loopreview numbers (340 builds / 10 ok, 225 dirty files) are cited from scratch notes and were not re-measured; E13 relied on read-only scouts for most [scout] lines.
- I opened roughly 60 engine, Forge and old-repo files for the checks above. Items in Sections C and D that are not marked [V] are auditor claims.

### Claims I could not verify
- Whether `tool_count_pin.rs` is red under `--features mcp` (E05 says reachable verbs are ~45).
- Which controller arena locomotion calls (controller.rs vs controller2).
- Whether a local ragdoll tape pin in `physics/tests` needs a re-pin after AD-17 (the golden does not).
- That Rapier 0.36 revolute joints are the right hinge type (very likely; not opened). PhysX 12/8 does not map 1:1 to Rapier's fields: the number must come from a test.
- Whether `game/src/capture.rs` already records the camera pose (AD-01 needs it).
- Forge counts: 1,445 Pass vs 12 NotWired (arch-review, a scratch count), 34 compat aliases, 501 lab files.
- E12's keeper findings other than `queueMove` (which I did read [V loop-keeper.js:1272-1279]).

### Where the small-model auditors disagreed or were wrong (12 corrections)
1. E02 said the golden hash moves and must be re-pinned for ragdoll mass/limit/solver changes. WRONG: the Rapier ragdoll is client-local and never hashed [V HASH-LAW.md:92-101].
2. E02 "MISSING joint_limits": the file is absent but PX9 (open) names it; status is PLANNED.
3. C3 G6 "idempotency partial, key only": HAVE, same key + other payload is a conflict [V commands.rs:436, :810].
4. C3 G17 "no tool-count test": a pin exists for 26 core tools [V tool_count_pin.rs]; the gap is pack verbs.
5. C2 U5 said crowd avoidance MISSING: `nav/src/crowd.rs` is real ORCA [V]; the gap is the missing consumer.
6. E11 #7 repeated the same error (near-field avoidance missing).
7. E06 said playtest is "used by the endpoint": it is a hook (`register_look_judge`) called only from a test, and no Cargo edge exists [V].
8. E08 #6 said proptest exists in core/net/render: no proptest dependency anywhere; there is a hand-rolled std-only matrix [V property_matrix.rs]. The fix reuses it.
9. E13 #9 presented semantic buffers as new: board row EV21 already asks for entity-id per region [V]; downgraded to an ADAPT of EV21.
10. E01 #3 severity 4: bounded by the shared deadline and budget [V protocol.rs doc]; set to 3.
11. E08 attributed the dead-subscription note to Forge; it lives in the old repo [F].
12. E01 #7 leaned on README "editor app: not yet"; README is stale (C1 gap 8), the bin exists and startup reopens durable generations [V startup.rs:15].
Unresolved: C1 vs E05 on how many verbs the door really serves; E05 vs E11 on whether comments "not registered today" in `build_game.rs` are stale (not opened).

### Small-model failure patterns seen
Confusing a hook with an integration; trusting README or plan text over code; treating a capped grep (10 hits) as proof of absence (C2 U5); line numbers that drift while a file changes (E12 saw the keeper change under it); counting a documented convention as a defect without reading its doc (unit cubes are documented in `scene_tools.rs:12-14`; the real defect is that the output does not say so).

### E13
E13 was written during this task. I read it in full and used it (AD-07, AD-08, AD-10, AD-12, AD-13, AD-14, N-02, N-03, N-28 to N-30, G-ideas).

## Plans

AI builds the game: AD-02 read-back digests, AD-08 verify pack, AD-26 refusals teach.
AI in gameplay: AD-13 rules commit gate, AD-20 reach honesty, N-18 stuck ladder.
Spatial understanding for AI: AD-03 honest checks, AD-07 real extents, AD-18 3D overlap.
Dismemberment and wounds: AD-16 hinge joint table fed to PX9.
Physics and destruction: AD-17 mass from total plus solver passes and one CCD policy.
Look on the RTX 3050: AD-01 stale capture plus Abstain, AD-24 subject visibility.
Determinism and exact replay: AD-02 staged digests, AD-04 zero-tests RED.
Headless proofs and tests: AD-04 evidence age, AD-05 unwired-code gate, AD-08 door parity.
Editor and tools: AD-10 gated undo, AD-09 door ring plus door.status.
