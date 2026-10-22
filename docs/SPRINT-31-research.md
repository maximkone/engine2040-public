# Research steals 2026-10-22 (owner: "what to steal, apply arXiv to repo, engine and agents")

Spec: `docs/research/finish-line-2026-10-22/` (`00-verdict.md` first). Each row names its donor; the
report section has the mechanism. Feel donors live in `02-feel.md` beside the FEEL rows in
`docs/SPRINT-30-feel.md`.

- [ ] FIN1 plan: the finish line says what it is not and what it already has. Owner picks the must
  list (`01-engines.md` section 1); `docs/plan/finish-line.md` gains a non-goals line (no human
  editor until G7 is green; no mobile or console) and one row listing the shipped staples with
  their tests (rebinding, settings, save slots, menus, locale, accessibility, profiler, audio).
  Accept: the plan names the cut and every listed staple points at a passing test. Owner decision.
- [ ] ENG5 endpoint: a `doctor` verb answers what is broken before a model starts. GPU adapter,
  content root, bundle freshness, the installed bridge age (DOOR5) and the last proof per crate,
  one line each with the fix. Donors: Open-Godot-MCP `health.py`, quench3d `doctor`. Files:
  `crates/endpoint/src/packs/perf.rs`. Accept: a test breaks each check once and reads its line.
- [ ] ENG1 web: the backend is chosen by a probe before the first frame. WebGPU first, WebGL2
  fallback, the choice and the limits that forced it in the page receipt. Donors: quench3d
  `web_probe.rs`, PlayCanvas device split. Files: `crates/web/src/shell.rs`, `limits.rs`.
  Accept: headless Chrome with WebGPU off loads the fallback and prints why.
- [ ] ENG9 assets: a content patch ships by blob hash. A manifest of content hashes; the game
  fetches only changed blobs and refuses a patch whose hash does not match. Donors: Defold Live
  Update, Godot patch PCK. Files: `crates/assets`, `tools/`. Accept: change one texture, build a
  patch, the patched game loads it and the rest are untouched (byte compare).
- [ ] ENG16 game: every run ends in a receipt, a crash too. A panic hook writes the tick, the
  hash, the last door call and the backtrace to `target/reports/crash/`, and the next start
  prints it once. Files: `crates/game/src/windowed.rs`, `crates/perf`. Accept: a forced panic in
  a test run leaves one receipt with the tick and the hash.
- [ ] ENG6 endpoint: latency and loss are a door verb. `net.shape` sets delay, jitter and loss on
  the duel link so a model can test its netcode. Donor: Open-Godot-MCP `network.py`. Files:
  `crates/endpoint/src/packs/net.rs`, `crates/proof/src/chaos.rs`. Accept: 120 ms and 5% loss end
  a scripted duel on the same digest as a clean link.
- [ ] ARX01 physics: a hit picks stagger, step or fall by the capture point. The victim's capture
  point (centre of mass plus velocity times the pendulum time, integer mm) against its support:
  inside holds, near the edge steps, past it falls. Classic balance control (2603.08619 uses it as
  a training signal). Files: `crates/physics/src/balance.rs`, `crates/anim/src/hit_react.rs`.
  Accept: law tests: mirrored hits mirror the choice, a harder blow never picks a milder reaction.
- [ ] ARX05 endpoint: the look judge reports its spread. The VLM judge answers over three prompt
  wordings and returns the spread; it never gates alone (2607.25921). Files:
  `crates/endpoint/src/packs/view.rs`. Accept: a test with a fake judge that flips on one wording
  reports the spread and no pass.
- [ ] AGT01 tools: OpenCode helpers do not spawn helpers. `experimental.subagent_depth` 1 and
  `permission.task: deny` for helper agents (2512.08296: central verification spreads fewer
  errors; depth beyond one helper level is unmeasured). Owner decision (2026-10-04 chose 2).
  Accept: `opencode debug config` shows depth 1.
- [ ] AGT03 tools: a dead helper's work becomes a patch file, not a rot pile. `tools/leftovers.mjs
  --save` writes each idle group's diff to `target/leftovers/<group>.patch` with the board row it
  touches, so a fresh helper applies it instead of re-reading (2608.03222, restart from state).
  Accept: node test: an idle group writes one patch that `git apply --check` accepts.
- [ ] AGT04 tools: the AGENTS.md rules are one review-gate script. Commit only own paths, no
  whole-file rewrite over a dirty file, no new genre names, ratchet, rustfmt, proof line in the
  message: `node tools/review-gate.mjs <commit>` prints PASS or the broken rule (2609.04167: a
  third of passing patches broke review rules). Accept: node tests, one per rule.
- [ ] AGT05 tools: a row's file list is its lease. Two helpers never take rows whose file lists
  overlap in one wave; `node tools/board-slice.mjs --wave N` prints N rows with disjoint files
  (2608.23740). Accept: node test: overlapping rows never share a wave.
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
