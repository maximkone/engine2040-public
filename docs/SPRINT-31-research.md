# Research steals 2026-10-22 (owner: "what to steal, apply arXiv to repo, engine and agents")

Spec: `docs/research/finish-line-2026-10-22/` (`00-verdict.md` first). Each row names its donor; the
report section has the mechanism. Feel donors live in `02-feel.md` beside the FEEL rows in
`docs/SPRINT-30-feel.md`.

- [ ] FIN1 plan: the finish line says what it is not and what it already has. Owner picks the must
  list (`01-engines.md` section 1); `docs/plan/finish-line.md` gains a non-goals line (no human
  editor until G7 is green; no mobile or console) and one row listing the shipped staples with
  their tests (rebinding, settings, save slots, menus, locale, accessibility, profiler, audio).
  Accept: the plan names the cut and every listed staple points at a passing test. Owner decision.
- [x] ENG5 endpoint: a `doctor` verb answers what is broken before a model starts. GPU adapter,
  content root, bundle freshness, the installed bridge age (DOOR5) and the last proof per crate,
  one line each with the fix. Donors: Open-Godot-MCP `health.py`, quench3d `doctor`. Files:
  `crates/endpoint/src/packs/perf.rs`. Accept: a test breaks each check once and reads its line. Landed 6330e67ba. The verb is `perf.doctor` (the door requires a dotted pack name), five checks each on one line with a fix: GPU adapter (reads `WGPU_ADAPTER_NAME`, else the newest `target/reports/arena-*.txt`, opens no device), content root, bundle freshness, the installed bridge age (DOOR5) and the last proof per crate; a missing proof log is a clean `skip` with a fix, a red or >72 h proof is `broken`. Proof: `cargo test -p engine-endpoint --features mcp --lib doctor` 7 passed (run 39866755240), each check broken once and its line read; clippy endpoint `--lib -D warnings` PASS (run 39868071875); `rust-quality --owned` both files PASS; rustfmt clean. Reuses `project_root` via one `pub(super)` visibility change in `packs/codex.rs` (no copied body). Open legs, named: plain `cargo clippy -p engine-endpoint --features mcp --all-targets -- -D warnings` fails at HEAD on `clippy::large_futures` in `crates/endpoint/src/bin/e2040-mcp.rs` (pre-existing, reproduced with 0 overlaid paths); the full endpoint lib run has 2 red `scene_tools.rs` spatial-delta tests in another lane's file (reproduce without this change).
- [x] ENG1 web: the backend is chosen by a probe before the first frame. WebGPU first, WebGL2
  fallback, the choice and the limits that forced it in the page receipt. Donors: quench3d
  `web_probe.rs`, PlayCanvas device split. Files: `crates/web/src/shell.rs`, `limits.rs`.
  Accept: headless Chrome with WebGPU off loads the fallback and prints why. Landed 2bf146d5a. Proof: `cargo test -p engine-web --test it` 5 passed, incl. `a_page_without_a_usable_webgpu_loads_webgl2_and_says_why` (run 39867475210); the browser leg (headless Chrome 155, `--disable-features=WebGPUService`, because 155 ignores `--disable-features=WebGPU`) loads the WebGL2 fallback and prints `backend=webgl2 reason="adapter request failed: null" limits=unknown` with `data-backend=webgl2`, `data-state=running`, golden `d57b5c8ae3096614` and 11/11 checks; wasm build exit 0 (4,255,067 bytes); wasm clippy exit 0 locally; `rust-quality --crate web` PASS 99. Open legs, named: `cargo test -p engine-web --lib` is red at HEAD (lib.rs's seat test reaches the wasm-only `fight` module; this row did not touch lib.rs); the arena page still requires WebGPU and `tools/web-browse.mjs`'s no-WebGPU expectations are not updated.
- [x] ENG9 assets: a content patch ships by blob hash. A manifest of content hashes; the game
  fetches only changed blobs and refuses a patch whose hash does not match. Donors: Defold Live
  Update, Godot patch PCK. Files: `crates/assets`, `tools/`. Accept: change one texture, build a
  patch, the patched game loads it and the rest are untouched (byte compare). Landed 0a6e86fe8.
  Proof: 7 lib tests + 1 bin test green (`a_patch_carries_only_the_changed_blob_and_applies_byte_for_byte`,
  `a_patch_blob_that_does_not_hash_is_refused_by_name`, stale base refused, empty tree, bundle base,
  byte-identical patch x2), scoped clippy green, `e2patch diff|verify` round-trips (runs
  39864469448/519/593). Open leg, named: the shipped run does not call `PatchedSource` yet; the
  row's file list stops at `crates/assets`/`tools`, so the game wiring is a follow-on, not claimed here.
  Progress 2026-10-23 (F24, the wiring): the open leg closes - `crates/game/src/arena_kit.rs` gains
  `with_kit_source`, which reads `content/patch.e2patch` beside the kit root, parses it, checks every
  base row with `verify_base` (a moved base file fails by path) and hands the import a `PatchedSource`
  over the directory; `stage_kit` and `stage_kit_or_default` both go through it, so the shipped arena
  window, the capture bin and the smoke apply a shipped patch and nothing changes when no file is
  there. Proof: `cargo test -p engine-game --features app --lib kit_source` 2 passed (patched blob wins,
  the untouched path stays on disk, no file reads the directory; another base is refused naming
  `src/b.txt` and `patch.e2patch`).
- [x] ENG16 game: every run ends in a receipt, a crash too. A panic hook writes the tick, the
  hash, the last door call and the backtrace to `target/reports/crash/`, and the next start
  prints it once. Files: `crates/game/src/windowed.rs`, `crates/perf`. Accept: a forced panic in
  a test run leaves one receipt with the tick and the hash. Landed f27f6f9c8.
  Proof: `cargo test -p engine-perf --lib crash_receipt` 6 passed (forced panic: one file, tick
  600, hash d57b5c8ae3096614; second read empty), clippy and `cargo check -p engine-game
  --features app` green. Open leg, named: the receipt is installed at the three `windowed.rs`
  entry points; the shipped `arena.rs` bin still installs only `crash::install_run_hook`, so wire
  the receipt there when `arena.rs` is free (it is FEEL7's file today).
- [x] ENG6 endpoint: latency and loss are a door verb. `net.shape` sets delay, jitter and loss on
  the duel link so a model can test its netcode. Donor: Open-Godot-MCP `network.py`. Files:
  `crates/endpoint/src/packs/net.rs`, `crates/proof/src/chaos.rs`. Accept: 120 ms and 5% loss end
  a scripted duel on the same digest as a clean link. Landed 325a54fa6 + 6b58f4792. Proof: `cargo test -p engine-proof --lib chaos::tests::shap` 4 passed (run 39868576928), incl. `shaped_duel_at_120ms_and_5pct_loss_matches_the_clean_digest` — the 120 ms / 5% duel over the shipped loopback wire folds the same tape, tip and sim-only digest as a clean link, twice (digest 17089590214351568806, seed 2040), and the seed-2040 shaper drops exactly 46/1000 packets; `cargo test -p engine-endpoint --features mcp --lib packs::net` 10 passed; clippy proof + endpoint `-D warnings` PASS; rustfmt clean; `rust-quality --crate endpoint --crate proof` PASS. The verb is `net.shape` with integer conversions (ms -> ticks at 60 Hz rounded up, permille -> whole percent rounded up) and `Observe` reads; out-of-range args refuse with `bad_args`. Golden `d57b5c8ae3096614` untouched (proof, endpoint and one doc line only, no sim crate).
- [x] ARX01 physics: a hit picks stagger, step or fall by the capture point. The victim's capture Landed 855540d. `crates/physics/src/capture_point.rs` picks hold, step or fall from the capture point against the support (integer mm) and `HitDir` carries the push; law tests: mirrored hits mirror the choice, a harder blow never picks a milder reaction; runs 40112186644 (10 tests) and 40112181770 (16 tests).
  point (centre of mass plus velocity times the pendulum time, integer mm) against its support:
  inside holds, near the edge steps, past it falls. Classic balance control (2603.08619 uses it as
  a training signal). Files: `crates/physics/src/balance.rs`, `crates/anim/src/hit_react.rs`.
  Accept: law tests: mirrored hits mirror the choice, a harder blow never picks a milder reaction.
- [x] ARX05 endpoint: the look judge reports its spread. The VLM judge answers over three prompt
  wordings and returns the spread; it never gates alone (2607.25921). Files:
  `crates/endpoint/src/packs/view.rs`. Accept: a test with a fake judge that flips on one wording
  reports the spread and no pass. Landed 7f2a70cf1: judge answers three wordings, score = lowest, pass needs all three and spread <= 100 permille (look_judge 6 passed run 40114889002, hook + real tests green).
- [x] ENG17 web: audio runs on an AudioWorklet fed by the existing mixer. The page ships a
  worklet processor that pulls from a ring buffer the mixer writes and keeps the
  ScriptProcessorNode only as the fallback when the worklet cannot load; the page receipt
  prints `audio: state=running underruns=<n>` (finish-line A5). Files: `crates/web`, the page's
  audio module, `crates/audio` (no device change). Accept: headless Chrome prints
  `state=running` with a counted underrun total; a scripted fight plays its cue set with no
  missing cue; a forced worklet load failure prints why the fallback was taken. Size S. Tier B.
  F2P: the page uses the deprecated ScriptProcessorNode. P2P: native audio unchanged; golden
  untouched. needs: none. Landed f858555f2 (+ bf1dd1ada). Proof: `cargo test -p engine-web --lib audio`
  23 passed (run 39872351345); `cargo clippy -p engine-web --all-targets -- -D warnings` PASS (run
  39873029061); wasm clippy `--target wasm32-unknown-unknown -- -D warnings` PASS; `node --test
  tools/web-input-check.test.mjs` 17 passed; rustfmt clean; `rust-quality --owned` PASS (web score
  99). One headless Chrome 155 run proves all three Accept legs: `audio: state=running underruns=0`,
  `audio: path=worklet blocks=15 underruns=0`, and the cue leg read from the mixer's own
  `MixerStats` — `audio: cues expected=6 played=6 culled=0 rejected=0` — then a forced 404 worklet:
  `audio: state=running worklet=fallen-back reason="AbortError: Failed to load worklet module
  script: ... (HTTP status: 404)"` with `audio: path=script ...` and the same 6/6 cues on the
  fallback path. The ring is 1024-frame blocks, 6 deep, low water 3; `arena.rs::step_fight` queues
  its cues into the live mixer. Open legs, named: no run provokes a real starvation, so
  `underruns` shows 0 (the counter's arithmetic is unit tested and the page takes `max()`); the
  `--native-tape` leg still prints NOT COMPARED (unchanged by this row). Native audio and the
  golden are untouched (no device or sim code). bf1dd1ada also clears a red `-D warnings` in
  `crates/web/tests/it/backend_choice.rs` left by 2bf146d5a (`type_complexity`, semicolon).
- [x] ENG19 proof: the Windows ship check runs the game, not only the build. The ship job
  (HX10, `.github/workflows/ship-check.yml`) also starts the shipped binary from the clean folder
  for N seconds, reads the run receipt (tick and hash) and quotes that line; when the runner
  exposes a GPU adapter it also compares one frame to `content/reference/` and prints the score,
  and names the adapter (`warp`) when it cannot. A missing receipt or a hash that is not the
  golden fails the job (finish-line E22, gate G1). Files: `.github/workflows/ship-check.yml`,
  `tools/ship*.mjs`, `crates/proof`. Accept: the job is green on the mirror and its log quotes
  `tick=... hash=d57b5c8ae3096614`; a deliberately broken receipt turns it red. Size M. Tier B.
  F2P: G1 "never broken" is unmeasured - the job compares two builds and never a run. P2P: no
  sim change. needs: none. Landed 85d5d5d26 + 88ca9b8dc + 15df03b7b + 88095e1f4. Proof: mirror
  run 39865729635 (snapshot of 88ca9b8dc) quotes `SHIP-RUN tick=600 hash=d57b5c8ae3096614` and the
  old named skip.
  Mirror run 40006304672 (snapshot of e058bba7f, with 15df03b7b and 88095e1f4) is green and its log
  quotes `SHIP-GAME tick=600 hash=7842af9a79d7bd11 frames=0 seconds=10.000 adapter=none
  headless=forced runs=2`, `SHIP-GAME presenter="presenter skipped: no wgpu adapter on this
  machine"`, `SHIP-FRAME skipped adapter=none reason=no-hardware-adapter`, and `SHIP-RUN PASS: the
  proof receipt is the golden and the shipped game played headless twice to one hash`; the checker
  step runs 32 tests, 32 pass, including the deliberately broken receipt that turns red. The first
  run 39863411687 failed on `arena: presenter error (no wgpu adapter available)`; 88ca9b8dc turns
  that into a narrow named skip. 15df03b7b adds the plain `--headless` route: 600 ticks, no window,
  no adapter, two fresh runs to one hash, gated on ticks, a 16-hex hash and the same hash twice; a
  no-adapter boot without the receipt is now red, not skipped. 88095e1f4 adds that gate to
  `tools/ship-run.mjs`. Open leg, named: the frame score still needs a scorer wired to
  `content/reference/` (the PNGs are on disk now); a runner with no adapter prints `SHIP-FRAME
  skipped adapter=none reason=no-hardware-adapter`.
- [x] ENG20 render: temporal artifacts get a gate (finish-line B9, gate O5). A TAA registration
  test proves the history is registered where the frame moves (a frozen or mis-registered history
  shows as ghosting over the bar and the test names it) and that a cut or resize resets it; a
  deterministic ghosting metric over a captured sequence scores a TAA-on run and a TAA-off run
  apart, so the offline gate is a number, not an opinion.
  Files: `crates/render/src/post/taa.rs` tests, `tools/temporal-gate.mjs` (new) + test.
  Accept: the registration test fails on a deliberately frozen history and passes on the real
  one; the metric separates the two captures; render lib green; golden unchanged.
  Open leg, named in the row and never claimed: the CGVQM+D model itself needs its published
  weights, so this row lands the registration test and the ghosting metric, not CGVQM+D.
  Size S-M. Tier A. Owns `crates/render/src/post/taa.rs` tests and `tools/temporal-gate.mjs`.
  needs: CQ25. Progress 2026-10-23 (846aa1cf0): the registration test and the ghosting metric
  landed — registered history is bit-exact (edge error 0.000 px vs bar 1.0), frozen flow lags
  7.883 px, a frozen history with the default clamp misses by 0.702 (gate 0.25), and the exact
  mis-registration mutation (`(x, y)` instead of `(x - mx, y - my)`) fails the test and was
  reverted; `tools/temporal-gate.mjs` scores real captures (spear-grapple 125 frames 0.225;
  rz2w-a 0.481; rz2-ab/off 0.631; rz2-ab/on 0.669; a stale frame 1.000) under a provisional 0.750
  bar. The labeled pair landed later in `4843a1df4`: `crates/render/tests/it/taa_pair.rs` writes
  three 12-frame 160x90 sequences (`target/captures/taa-pair/{off,on,registered}`) from the real
  resolve pass - `off` is the reference, `on` is a frozen history, `registered` is the real flow -
  and the test asserts `registered == off` bit for bit, the frozen arm differs at the block edge,
  and a second run repeats. The gate separates the two captures: `--pair off on` prints
  `separation 0.246` against the 0.05 bar (ghost_share 0.273, moved_share 0.018), while
  `--pair off registered` prints `separation 0` because the good arm reproduces the reference.
  Pair test 1 passed (CPU, 107 s); render lib 1349 passed (run 39984467748); golden
  `d57b5c8ae3096614` PASS (run 39988122554). CGVQM+D stays out as the row says.
  Landed 4843a1df4.
- [x] AGT01 tools: OpenCode helpers do not spawn helpers. `experimental.subagent_depth` 1 and
  `permission.task: deny` for helper agents (2512.08296: central verification spreads fewer
  errors; depth beyond one helper level is unmeasured). Owner decision (2026-10-04 chose 2).
  Accept: `opencode debug config` shows depth 1. Landed 16b5b284.
- [x] AGT03 tools: a dead helper's work becomes a patch file, not a rot pile. `tools/leftovers.mjs
  --save` writes each idle group's diff to `target/leftovers/<group>.patch` with the board row it
  touches, so a fresh helper applies it instead of re-reading (2608.03222, restart from state).
  Accept: node test: an idle group writes one patch that `git apply --check` accepts. Landed c9a558f2e.
- [x] AGT04 tools: the AGENTS.md rules are one review-gate script. Commit only own paths, no
  whole-file rewrite over a dirty file, no new genre names, ratchet, rustfmt, proof line in the
  message: `node tools/review-gate.mjs <commit>` prints PASS or the broken rule (2609.04167: a
  third of passing patches broke review rules). Accept: node tests, one per rule. Landed d4a094eb8.
  9 tests pass; R4/R5 read the working tree, so another lane's dirty Rust can colour them.
- [x] AGT05 tools: a row's file list is its lease. Two helpers never take rows whose file lists
  overlap in one wave; `node tools/board-slice.mjs --wave N` prints N rows with disjoint files
  (2608.23740). Accept: node test: overlapping rows never share a wave. Landed e1157b2ff.
- [ ] AGT19 tools: measure the worker-count curve here. Rows landed per hour and reds per landing at
  2, 4, 8 and 16 helpers over one day each, read from git and gh-check receipts. Accept: a CSV and
  a one-line verdict in `docs/research/`.
- [ ] PROG1 scene: the programmable world model, as a spike. One executable program owns the
  world state; the renderer only draws it (arXiv 2609.10540, AlayaLab/pwm: the program keeps the
  persistent global state, off-screen entities included, and the visual side is a separate
  generative stage). Today the state and the picture are one path: `crates/ecs/src/bridge.rs:306`
  `encode_arena` writes slot, position and yaw, and `crates/game/src/lib.rs:1959` `FighterView`
  carries the same three numbers, so a renderer can only read what the sim already computed. The
  spike adds one typed view of the sim state that names entities by kind and bounds, so a
  generative renderer can be fed without touching the sim: the program stays authoritative, the
  picture becomes a second consumer. Files: `crates/ecs/src/bridge.rs` (the entity view beside
  `encode_arena`), `crates/scene/src/schema.rs` (the typed view), `crates/proof` (the tape). Tier
  A. Size M. Accept: `cargo test -p engine-ecs --lib bridge` green with one test per view kind
  (a fighter, a weapon, a prop), the same tape gives the same view bytes twice, a view names the
  entity that changed since the last tick, and the golden `d57b5c8ae3096614` does not move; the
  research note records the CombatStateBench-style count and state accuracy the view reaches on
  one seeded arena frame. F2P: no typed entity view; the only state that leaves the sim is a
  position and a yaw. P2P: golden unchanged; the bridge wire layout is unchanged. needs: none.
- [ ] PROG2 render: the execution harness split, as a spike. The game execution and the visual
  generation are separate stages, and the renderer draws low-poly proxies the engine itself
  produced (arXiv 2610.27999, Matrix-Game 4.0: a lightweight engine keeps the world state and
  executes the interaction rules, and a renderer turns engine-rendered low-poly proxies into
  detailed visuals, conditioned on dynamic captions; their distilled causal generator runs
  4-frame chunks at 1-2 denoising steps). The engine already has the split's two halves on
  paper: `crates/web/src/frame_bus.rs` carries the sim state to the draw thread, and the arena
  page draws a procedural ring, but nothing renders the sim's own proxy geometry, so the web
  picture is not the engine's picture. The spike: the engine emits its own low-poly proxy
  geometry for the arena, and a capture pair shows the native frame and the web frame carrying
  the same proxies. Files: `crates/game/src/fighter_look_acts.rs` (the proxy draw),
  `crates/web/src/arena.rs` (the page reads the proxies). Tier A. Size M. Accept: one capture
  pair (native frame, web frame) with the same fighter count in both, the SSIM comparator in
  `crates/web/src/arena_view.rs:303` printing a number, the golden `d57b5c8ae3096614` unchanged,
  and the research note recording the ms per frame on the RTX 3050 against the paper's 480p
  60 FPS on an RTX 5090. F2P: the page draws stand-ins, not the engine's own geometry. P2P:
  golden unchanged; the native frame bytes unchanged. needs: none.
