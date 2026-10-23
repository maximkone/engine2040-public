# Book gaps 4, speed and mobile (2026-10-23)

Rows from the 2026-10-23 book and optimization pass. Spec:
`docs/plan/BOOK-GAPS-4-2026-10-23.md`. BG1-BG6 are book-derived, BG7-BG9 are
the owner's mobile tier, BG10-BG13 are the optimization holes, BG14-BG16 are
disk-only residuals. Rules v1 golden `d57b5c8ae3096614` stays on every row;
BG6 and BG15 move rules v2 only and need a rules receipt.

- [x] BG1 endpoint: the door advertises a push channel — Landed 58c213bf3.
  `mcp.rs:1507-1516`
  returns the default `ServerCapabilities`; `rg "list_changed|resources/updated"
  crates` is 0, so a client polls. Advertise `tools.listChanged` and
  `resources.subscribe`, send one `tools/list_changed` when the tool set moves
  (the load-pack verb) and one `resources/updated` per subscribed session when
  the scene resource's hash moves. Diff: regular. Tier A. Size S. Owns
  `crates/endpoint/src/mcp.rs`, `crates/endpoint/src/resources.rs`. Accept:
  `cargo test -p engine-endpoint --features mcp --lib door_announces_list_change`
  - initialize advertises both flags; `api.load web` sends one
  `tools/list_changed`; a subscribed session gets one `resources/updated` for
  `e2040://scene`, an unsubscribed one gets none. F2P: no notification is ever
  sent. P2P: golden and the 75/75 door parity tapes unchanged. needs: none.
- [x] BG2 endpoint: resource templates have no child URI — Landed dfab6a88c.
  `resources.rs:109-122`
  advertises four `{uri}/{id}` templates and `:161-175` accepts only the four
  roots. Accept `e2040://door-reference/scene.read`,
  `e2040://error-catalog/bad_args` and `e2040://game-toml-schema/name` by
  splitting the child id and reading the same backing source the root prints;
  an unknown child refuses `unknown_uri`. No new tool, so the API6 budget does
  not move. Diff: good. Tier A. Size S. Owns `crates/endpoint/src/resources.rs`.
  Accept: `cargo test -p engine-endpoint --features mcp --lib
  resource_template_reads_one_child` green; `tool_count_pin` unchanged. F2P:
  today the child URI is `unknown_uri`. P2P: the four root reads unchanged.
  needs: none.
- [x] BG3 endpoint: `rules.set` has no read-back and no undo — Landed d9d9ca525.
  `packs/rules/verbs.rs:113-126` returns hashes only; `Rules::source/history`
  (`crates/rules/src/rules.rs:279,285`) have no door caller. Add `rules.get`
  (the installed text and its hash) and `rules.undo` (reinstall the previous
  install, keep a bounded stack). AD13 stays the approval row; this is restore.
  Diff: regular. Tier A. Size S. Owns `crates/endpoint/src/packs/rules/verbs.rs`,
  `crates/rules/src/rules.rs`. Accept: `rules_undo`: set A, set B, `rules.get`
  returns B's text, `rules.undo` reinstalls A (hash equal), a second undo
  restores B, undo with one install refuses `nothing_to_undo`. F2P: no undo, no
  read-back. P2P: `rules.set` output unchanged, v1 golden unchanged. needs: none.
- [x] BG4 endpoint: clip edit functions are not door verbs — Landed cf8969aa2.
  `anim/src/edit.rs:77-217` holds trim, retime, cleanup, strip and mirror;
  `packs/anim.rs:11-13` says they "join here later" and only `anim.graph.*`
  verbs exist. Add `anim.clip.trim|retime|mirror` over the same library calls,
  refusing by default like the other write verbs. Diff: regular. Tier A. Size S.
  Owns `crates/endpoint/src/packs/anim.rs`. Accept:
  `cargo test -p engine-endpoint --lib anim` green; each verb returns the same
  digest as the library call on the fixture, refuses without the write flag and
  is undoable through the existing clip store. F2P: the verb does not exist.
  P2P: the four `anim.graph.*` verbs and the golden unchanged. needs: none.
- [ ] BG5 game: animation events reach no consumer — `anim/src/events.rs:3-4`
  promises audio and combat; `rg "engine_anim::events" crates --glob '!crates/anim/**'`
  is 0; the live footstep is derived from stride
  (`window_audio_space.rs:395`). Deliver the clip's footstep event to the sand
  cue on its exact tick, presentation-side. Diff: good. Tier B. Size S-M. Owns
  `crates/game/src/window_audio_space.rs`, `crates/game/src/fighter_look*.rs`
  (read only: `crates/anim/src/events.rs`). Accept: a footstep event on the run
  clip fires the sand cue on the clip tick +-1; two replays are equal; a clip
  with no event fires none. F2P: the event has no consumer. P2P: golden
  unchanged, the stride cue still fires when no event exists. needs: none.
- [ ] BG6 arena: foe perception ignores cover — `arena/src/engage.rs:92-120`
  ranks by separation only; `spatial/src/sight.rs` has no tick caller;
  `arena/src/world.rs:59-99` has a 2D obstacle with no height. Add a cover test
  against the existing obstacle boxes before a sighting enters `FoeView`.
  Rules v2 only. Diff: regular. Tier B. Size M. Owns
  `crates/arena/src/engage.rs`, `crates/arena/src/world.rs`. Accept: a fighter
  3 m behind `crate_a` is absent from `FoeView` while one 6 m away in the open
  is present; the tournament band stays 400-600 permille; v1 golden unchanged.
  F2P: the blocked fighter is seen today. P2P: the existing engage tests and
  the v1 golden unchanged. needs: a rules v2 receipt.
- [ ] BG7 input: no touch lane — `crates/input` and `crates/ui` handle keyboard,
  pad and agent bits; nothing maps a touch point. Add a touch source (tap,
  drag, two-finger) that drives the same action bits as the pad, windows-native
  first. Diff: regular. Tier A. Size M. Owns `crates/input/src/touch.rs`,
  `crates/input/src/lib.rs`, one tape test. Accept: a tap/swipe maps to the
  same action bits as the pad mapping and hashes the same in a tape; two
  replays are equal. F2P: a touch point is dropped. P2P: the keyboard, pad and
  agent lanes unchanged, golden unchanged. needs: BG8's tier table (no).
- [ ] BG8 render: no mobile tier or thermal budget — `render/src/quality.rs:11-18`
  has Potato and Web only. Add a `Mobile` tier (DPR cap 2, one shadow cascade,
  GTAO off or half-res, texture and physfx caps) picked from adapter limits
  and a device-class hint, plus a soak harness that prints p50, RSS and a
  thermal-cliff flag. Diff: regular. Tier A. Size M. Owns
  `crates/render/src/quality.rs`, `crates/render/src/lib.rs`, one test.
  Accept: `cargo test -p engine-render --lib mobile_tier` green: the tier is
  picked from limits, every cap is under the desktop tier, and a 30-minute
  soak report prints p50, RSS and no cliff line. F2P: no Mobile variant exists.
  P2P: Potato and Web picks unchanged, golden unchanged. needs: none.
- [ ] BG9 build: no Android leg — no `aarch64-linux-android` in the tree or CI.
  Add the target to the check matrix with the NDK on the runner, for the arena
  shell only; the golden is not run there. Diff: regular. Tier B. Size L. Owns
  `.github/workflows/golden-matrix.yml`, `rust-toolchain.toml` (read only).
  Accept: `cargo check --target aarch64-linux-android -p engine-game --features app`
  prints PASS on the runner. F2P: the target is never built. P2P: the five
  existing matrix legs unchanged. needs: NDK on the runner (owner/runner call).
- [ ] BG10 web: the shipped build is not SIMD128 — `tools/web-build.mjs` has no
  `simd128`; only the golden matrix proves the target
  (`golden-matrix.yml:132-162`). Add a `--simd` switch that passes
  `-C target-feature=+simd128`, keep the 8 MiB dist / 4.7 MB wasm caps, and
  print the flag in the build receipt. Diff: good. Tier A. Size S. Owns
  `tools/web-build.mjs`, `tools/web-build.test.mjs`. Accept: the SIMD build is
  under both caps, the page receipt OK, the replay digest equal to the scalar
  build, and no p50 regression. F2P: the flag is never passed. P2P: the scalar
  build and the dist layout unchanged. needs: `dist/web` must not be dirty by
  another seat at the time of the build.
- [x] BG11 ecs: every job allocates — Landed 541a1fe5. `ecs/src/pool.rs:58`
  boxes the closure and
  `:475` boxes each job; `par_chunks_map` builds slots and a result `Vec`. Add
  a probe that counts job allocations and remove the per-job box (an enum job
  or a stack slot), with an arena that survives warmup. Diff: regular. Tier A.
  Size S-M. Owns `crates/ecs/src/pool.rs`, one test. Accept: the probe prints
  allocations per job (>=1 today) and 0 after warmup; golden and the pool
  equality tests unchanged. F2P: no allocation count exists and the box is
  unconditional. P2P: `identical_poses_under_every_chunking_and_worker_count`
  and the golden unchanged. needs: none.
- [ ] BG12 ci: the loom model never runs — the model is at `pool.rs:494` and
  `rg loom .github/workflows` is 0. Add a CI leg that runs
  `RUSTFLAGS="--cfg loom" cargo test -p engine-ecs --lib loom_pool` and a note
  in the job pool. Diff: good. Tier A. Size S. Owns
  `.github/workflows/golden-matrix.yml` (or a new workflow), `Cargo.toml`
  (read only). Accept: the leg is green on a planted model with a missing wait
  red and the real model green. F2P: the model is only compiled by hand. P2P:
  the five matrix legs unchanged. needs: none.
- [x] BG13 arena: the bare build is claimed but not gated — Landed e310aa24e.
  `crates/arena/Cargo.toml:33-36` says `--no-default-features` builds;
  `authority.rs:275` is then unused (a bare-build warning). Make that clippy
  line exit 0 and print one PASS line per featureless crate in the check tool.
  Diff: good. Tier A. Size S. Owns `crates/arena/src/authority.rs`,
  `tools/web-build.mjs` or `tools/gh-check.mjs` (one leg). Accept:
  `cargo clippy -p engine-arena --no-default-features --all-targets -- -D warnings`
  exits 0; the tool prints PASS for the crate. F2P: the bare build warns.
  P2P: the default-feature build and golden unchanged. needs: none.
- [ ] BG14 endpoint: EG49 stopped at the tool surface — `tools/memory/memory.mjs`
  and `tools/tasks/run.mjs` exist, but `rg "e2040://memory" crates` is 0 and
  `run.mjs:26,203` scores only a canned log. Expose the same records as
  `e2040://memory?path=...`, `memory.record` (a 16-hex hash required) and
  `tasks.list` (ten jobs), and give `run.mjs --solve --arm scripted|mutation`
  a real pass@1. Diff: regular. Tier B. Size M. Owns `crates/endpoint/src/resources.rs`,
  `tools/tasks/run.mjs`. Accept: the resource returns rows in stamp order;
  `memory.record` refuses a row without a 16-hex hash; `tasks.list` names ten
  jobs; `--arm scripted` prints `pass@1 1.000` and `--arm mutation` `0.000`.
  F2P: no door surface exists. P2P: the tool count pin and the 75/75 tapes
  unchanged. needs: none.
- [ ] BG15 arena: kit wear never reaches a duel blow —
  `arena/src/authority.rs:334` and `limbs.rs:366` pass `wear_q16: 0` while
  `combat/src/cut.rs:311` already scales damage by wear. Carry the edge wear
  from the kit through the blow. Rules v2 only. Diff: regular. Tier B. Size M.
  Owns `crates/arena/src/authority.rs`, `crates/arena/src/limbs.rs`. Accept: a
  five-round headless duel prints a non-zero mean edge wear and a cut test
  shows damage drop proportional to wear; v1 golden unchanged. F2P: wear is
  always 0, so the scaling is dead. P2P: the v1 golden and the cut tests
  unchanged. needs: a rules v2 receipt.
- [ ] BG16 game: ladder arena variants are counts — `content/design/ladder.rows:32,34`
  set `new_arena=1` and `tools/beat-lint.mjs:160,198` only sums them; no
  variant key is read in `crates/game` or `crates/arena`. Make a rung name and
  load a second `placement.json`, with the beat lint still passing. Diff:
  regular. Tier B. Size M. Owns `content/design/ladder.rows`,
  `crates/game/src/march.rs`, `tools/beat-lint.mjs`. Accept: rung 4 runs a
  second placement and prints its id; beat-lint passes. F2P: the count is
  never acted on. P2P: the current ladder parse and the golden unchanged.
  needs: none.
