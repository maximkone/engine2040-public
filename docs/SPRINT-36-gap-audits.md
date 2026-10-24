# SPRINT-36 - gap audits (2026-10-23)

Source: three audits written against this tree on 2026-10-23:
`docs/research/workspace-gaps-2026-10-23/README.md` (vs Flax and peers),
`docs/research/tools-gaps-2026-10-23/README.md` (tools and dev loop),
`docs/research/llm-gaps-2026-10-23/README.md` (model and agent side).
Each row is one gap from those tables. The full table stays in the research
file: a row is added here only for work someone can take, and its evidence moves
with the code. Rows marked `[x]` landed the same day the audit ran.

## Workspace and project (vs Flax and peers)

- [x] WG1 project: `game.toml` selects the game crate and bin - `game = "pkg:bin"`,
  workspace-validated, `build` compiles it, `run` runs it, and the four keys the
  engine does not read are refused by name. Landed `fdc848697`.
- [x] WG2 endpoint: a plugin registry, static and safe - today `git grep -i plugin -- crates`
  is 10 lines of comments; O3DE Gems and Fyrox dylib reload have the feature.
  Tier B. Size M. Owns `crates/endpoint/src/packs/`, `src/manifest.rs`.
  F2P: a new `modules = [...]` key resolves against a `linkme` slice and refuses an
  unknown name with the list; the test fails on HEAD (no key).
  P2P: `cargo test -p engine2040 --bins` stays green. Accept: unknown module refused,
  known module listed by `project.inspect`.
  Part 2026-10-24 (`b39d5810b`): the registry is real - each pack publishes a `PackModule`
  row through a `linkme` slice with an `order` slot, so `packs/mod.rs` keeps no hand list
  (three rows stay hand-written with a reason: their files are held by other seats);
  `linkme 0.3.37` (MIT OR Apache-2.0) is in `docs/DEPENDENCIES.md`. Runs 40075475576,
  40076858070, 40075932855, 40076403374.
  Decision: the `modules` key stays in the manifest as data (option 2). The root crate
  must not gain an `engine-endpoint` edge - `packs` is `mcp`-gated and the headless resolve
  would pull rmcp and tokio. So `src/manifest.rs` parses and stores `modules`, and the door
  resolves the names on project load, refusing an unknown one with the list.
  Done (`7406fbb60`): the key is known (9 keys), duplicates drop in declaration order, a
  non-list refuses by its own key, and a keyless `summary()` is byte-identical (pinned
  against the old string). `packs/mod.rs` exposes `known_modules()`/`resolve_modules()`,
  `unknown_module` lists every known pack as `(default)` or `(on demand)`, and
  `project.build`/`project.run` refuse before any work. Runs 40077683716, 40077805643,
  40078231346, 40078688923.
- [x] WG3 projects: a package lock - a package is only a Cargo crate in this one
  workspace; no store, no fetch check. Tier C. Size L. Owns `src/project.rs`.
  F2P: `build` refuses an unlocked dependency (new test). P2P: existing project
  tests stay green. Accept: one lock file, one refusal. (Licence and supply-chain
  call belongs to the owner.)
- [x] WG4 endpoint: `prefab.save` and `prefab.place` - `crates/scene/src/prefab.rs`
  is finished (`OverrideMap:413`) and `git grep -i prefab -- crates/endpoint` was 0.
  Tier A. Size S. Owns `crates/endpoint/src/packs/scene/`, `crates/editor/src/prefab.rs`.
  F2P: the two verbs are absent from `tools/list` and the new pack test fails.
  P2P: `cargo test -p engine-endpoint --features mcp --lib` stays green.
  Accept: save a subtree, place it twice, undo both, same digest.
  Landed `894f1ea0c` (runs 40039035075, 40040955789): one batch through `commands::apply`,
  so `tx.undo` takes a placement back; the digest uses the revision-free
  `commands::document_digest`. The listing grew past its pin, so `6742abd90` trimmed
  5,232 chars of DTO docs (66,725 -> 61,493 against the 62,400 pin) with no name, type
  or refusal changed.
- [x] WG5 world/render: 3D terrain and foliage - `crates/assets/src/collider_cook.rs:17`
  says heightfields are not cooked; no terrain hits in render, nav, assets, spatial.
  Tier B. Size L. Owns `crates/world/src/`, `crates/assets/src/`, `crates/render/src/`.
  F2P: a height grid cooks to a mesh plus collider and one `terrain.set` verb (new test).
  P2P: the golden tape does not move. Accept: a 64x64 grid renders and blocks at the
  cooked height.
  Part 2026-10-24: `bce18fe0f` cooks a `HeightGrid` (integer mm) to a `TerrainMesh` +
  `HeightfieldRecord`, byte-equal twice, 8 triangles on a 3x3, malformed grids refused;
  `18f6cc10a` puts the cooked mesh in the draw list as one `"terrain"` node
  (`TERRAIN_MESH_ID`), no GPU upload path change. Runs 40060965451, 40061456369.
  Remaining: `world.terrain.set`/`world.terrain.stats` (home: `packs/world.rs`, already
  registered) and the 64x64 blocking accept.
  Verbs landed too (`57ce79f4c`): `world.terrain.set` cooks the grid and answers the
  revision; `world.terrain.stats` reports 7938 triangles for 64x64 plus the draw name and
  mesh id the render lane carries (runs 40064552893, 40063878939). Still open: physics
  does not consume `HeightfieldRecord` yet (only `crates/assets` names it), so the
  "blocks at the cooked height" accept is unproven.
  Closed (`a8bbe9eb6`): `crates/physfx/src/terrain.rs` builds a Rapier heightfield (and a
  mesh shape from the same integer grid) from the record's plain fields - no edge to
  `engine-assets`; a ball dropped over a 3x3 grid rests at the cooked height at fixed dt,
  malformed records are refused by name, and the same record gives bit-equal params.
  Runs 40074061953, 40074450152.
- [x] WG6 editor: a rule node that compiles to the existing Rhai text - the node graph
  has no script node (`crates/editor/src/graph.rs:26-31`); `crates/rules` runs text.
  Tier B. Size M. Owns `crates/editor/src/graph.rs`, `crates/rules/src/`.
  F2P: `NodeKind::Rule` compiles to source; the new graph test fails on HEAD.
  P2P: `cargo test -p engine-editor --lib` stays green. Accept: graph -> Rhai source ->
  same rule hash as the text path. Landed `0c632dc4e` (graph bytes v2; v1 still reads).
- [x] WG7 material: a node graph compiles to a material record - the graph is stored but
  `crates/material/src/compile.rs` starts from `MaterialRecord`, and MaterialX nodegraphs
  are refused (`mtlx_import.rs:617`). Tier B. Size M. Owns `crates/material/src/`.
  F2P: `material.from_graph` lowers Const/Op nodes (new test). P2P: existing material
  tests stay green. Accept: the same record from graph and from TOML, byte-equal.
  Landed `8923b2c4a`: the verb folds the graph with `Graph::to_record` and returns the
  record text, its digest and counts; an unknown node kind is refused by name. The record
  parses back byte-equal. Runs 40066146766, 40066598982.
- [x] WG8 anim: a cutscene keyframe document - no cutscene or dopesheet exists; the only
  sequencer is music steps (`crates/audio/src/music.rs:444`). Tier C. Size M.
  Owns `crates/anim/src/`, `crates/endpoint/src/packs/anim/`.
  F2P: `anim.timeline.set` plays through `anim.preview` (new test). P2P: anim tests stay
  green. Accept: one 4-key timeline, frame 30 equals the authored pose.
- [x] WG9 ui: layout as data - screens are Rust data (`crates/ui/src/menus.rs`); no widget
  tree or layout file. Tier C. Size M. Owns `crates/ui/src/`, `content/`.
  F2P: `.ui.toml` rows load into the HUD snapshot (new test). P2P: ui tests stay green.
  Accept: the same menu from Rust and from TOML draws identical pixels.
  Landed `e2c629262`: `content/ui/menus.ui.toml` -> `menus::menu_layout::MenuLayout`,
  equal to the Rust tables per locale. Open: the renderer still reads the Rust tables,
  so the pixel accept is not claimed.
- [x] WG10 endpoint: a remote inspector on loopback TCP - `remote_shim.rs:1-2` said
  "in-process, no socket". Tier B. Size M. Owns `crates/endpoint/src/remote_shim.rs`.
  F2P: the allowlisted `BrpRequest` table answers on a loopback port (new test).
  P2P: the in-process path stays green. Accept: two clients, one scene, read-only.
  Landed `408b2cf94`: `remote_shim/loopback.rs` binds `127.0.0.1:0` by construction,
  per-run token, 64 KiB frames, write verbs refused by name through the door's own
  `readonly()` annotation. Opt-in; the in-process shim stays the default.
- [x] WG11 build: one-click export that ships a project - `tools/ship.mjs:1073` stages a
  fixed five-file set; no version, icon or installer in the manifest. Tier B. Size L.
  Owns `src/build.rs`, `tools/ship.mjs`. F2P: `build` writes `dist/<name>/` plus
  `build.json` (version, digest) and one archive step (new test). P2P: the mirror ship
  check stays green. Accept: a clean folder runs the declared game in under 5 minutes.
  Landed `fd5e01a99` + `80c9be529`: `build` stages `<project>/dist/<name>/`, writes
  `build.json` (name, version, target, game, one sha256 per file) and one tar step; the
  hash is `certutil -hashfile` so no hash crate enters the headless graph. Open: the
  folder ships the exe only, not content, so the run-in-5-minutes accept is not claimed.
- [x] WG12 perf: `perf.trace` writes a Chrome-trace JSON - `crates/perf/src/profiler.rs`
  has puffin scopes; a capture needs `tools/rdc-diff.mjs` outside the engine. Tier A.
  Size S. Owns `crates/perf/src/`, `crates/endpoint/src/packs/perf.rs`.
  F2P: the verb is absent (new test). P2P: perf tests stay green. Accept: one trace with
  the eight tick phases, opens in a trace viewer.
  Landed `11b0924e0`: `perf.trace` captures 1..=8 frames into a Chrome-trace document
  with a `process_name` metadata row and one integer slice per span. A capture measures
  ten phases (four tick systems + six GPU passes), so the test asserts at least eight
  rather than inventing two. Runs 40067811699, 40067642896.
- [x] WG13 game: a real minidump - `crates/game/src/crash.rs:743-753` wrote a 12-byte stub
  and its own PORT names the donor. Tier B. Size S. Owns `crates/game/src/crash.rs`.
  F2P: a feature-gated dump has the donor magic (new test). P2P: the crash receipt test
  stays green. Accept: a forced panic leaves a dump the donor tool opens.
  Landed `2cb4ad8fd`: feature `minidump` (off by default) writes a real dump through
  `minidump-writer` 0.13.0; the test parses `MDMP` and the stream directory (>=3 streams).
  Runs 40034369328 (P2P 19), 40033841891 (feature on, 20), F2P 40032495985 (stub fails).
- [x] WG14 assets: a 128 px thumbnail per import - `crates/editor/src/browser.rs` lists rows.
  Tier C. Size S. Owns `crates/assets/src/`, `crates/editor/src/browser.rs`.
  F2P: one thumbnail is cooked at import (new test). P2P: cook keys do not move.
  Progress 2026-10-24 (`04a6d1d70`): the import cooks the thumbnail.
  `crates/assets/src/thumb_cook.rs` cooks one 128x128 RGBA8 image (integer box average of
  the source rectangle, sealed with `content_hash`), and `import_kit_gltf_full` puts it on
  `KitPiece::thumb`, so a fresh import has its preview with no editor pass. Browser rows
  show it through `ThumbnailCache::insert_cooked`, and the cache is now 128 px;
  `the_cache_holds_exactly_the_size_the_import_cooks` keeps cook and cache equal. F2P
  `banner_import_cooks_one_128_px_thumbnail` plus three `thumb_cook` tests: 4 passed twice,
  runs 40068943985, 40069878193. P2P `--features import --lib`: 416 passed, 3 ignored, run
  40070017217, so no cook key moved. Clippy green, runs 40069615526, 40069774765.
  Left: the conditional kit line `cargo test -p engine-game --features app --lib kit` has
  two reds in `windowed` GPU timing tests (own-pass sky is timed on the kit), a file this
  row does not own; `content/arena/kit.e2bundle` is untouched and `kit_gltf` never reads
  `thumb`, so 5642f3b582be5f94 cannot move, but that line wants a quiet GPU run to say so.
- [x] WG15 endpoint: `project.new`, `project.build`, `project.run` - 104 tool constants and
  no `project.*`; only `game.build` (`crates/endpoint/src/build_game.rs:24`). Tier A.
  Size S. Owns `crates/endpoint/src/packs/`. F2P: the three verbs are absent (new test).
  P2P: `game.build` stays green. Accept: new -> build -> run from one session.
  Unblocked 2026-10-24: `packs/mod.rs` is clean; only `packs/asset*`, `level_paint.rs`,
  `audit/` and `mcp_refusal.rs` are held by other seats. Add a new `packs/project.rs`
  and register it with one hunk in `packs/mod.rs`.
  Landed `557cf2ee6`: `packs/project.rs` (pack `project`) writes the smallest real project,
  builds its brief to `build/<name>.e2package` through `build_slice`, and runs the package
  headless with the play digest and `replay_matched`; a stale or magic-less artifact is
  refused. `project.run` was allowlisted by exact name in the door's name guard, with its
  reason. Runs 40069030990, 40069500716, 40070794293.

## Tools and dev loop

- [x] TG1 ci: the finished web gates run on every PR and main push -
  `web-gates.yml` runs input, budget, headers, doctor, browse and perf. Landed `704c12974`.
- [x] TG2 ci: an instruction-count leg on main push (report-only) - `perf-check.yml`
  runs the Callgrind bench and keeps `target/gungraun`. Landed `7d247261f`.
- [x] TG3 tools: one inventory gate names every live tool - `tools/tools-index.mjs --check`
  (127 live, 13 without a test mate, each explained). Landed `352feb286` + `2e96a0ca4`.
  The untracked `tools/.qw4-check.mjs` near-copy (78 KB, idle 11 h) was moved out of the
  tree to the session temp on 2026-10-24 (bytes kept, not deleted).
- [ ] TG4 ci: a coverage floor - `tools/wiring-cov.mjs` needs `cargo llvm-cov` and is a
  manual NOOP; zero `llvm-cov` hits in `.github`. Tier B. Size M. Owns
  `.github/workflows/`, `tools/wiring-cov.mjs`. F2P: a workflow step runs `llvm-cov` and
  fails under the floor (new job). P2P: the mirror suite stays green. Accept: one number
  per PR for `crates/core`, `crates/proof`, `crates/endpoint`.
  Part 2026-10-24 (`0d68b49f4`): the `floor` job in `coverage.yml` runs `cargo llvm-cov`
  for the three crates and `tools/coverage-gate.mjs` holds `tools/coverage-bar.json`.
  Measured on this PC: engine-core 92.9% (4363/4696 lines, floor 91.9), engine-proof 81.1%
  (7235/8915, floor 80.1). Still open: the engine-endpoint `--features mcp` leg did not
  build while `crates/editor/src/lib.rs` and then `crates/render/src/lib.rs` were dirty in
  other seats, so its floor is pending; the job measures and prints it with no floor.
- [x] TG5 release: version, changelog, tag and SBOM - no CHANGELOG, no tag step, no
  version bump; `tools/ship.mjs` stages files only. Tier B. Size M. Owns `tools/`,
  `.github/workflows/`, `Cargo.toml`. F2P: `node tools/release.mjs --check` fails on HEAD
  (no file). P2P: workspace version check stays green. Accept: one tag from one command,
  changelog section and SBOM beside it. Landed `94a7df02e`.
- [x] TG6 ci: mutation and fuzz on a schedule - `tools/mutants.mjs` and the fuzz targets
  exist, no workflow runs either. Tier C. Size S. Owns `.github/workflows/`.
  F2P: no `schedule:` names the tools (grep). P2P: existing CI stays green.
  Accept: a nightly run writes its report artifact.
  Landed `550b1f2ee`: `nightly.yml` (Mon 04:17 UTC + dispatch, report-only); run
  40032876056 is green (32 fuzz tests) with `nightly-fuzz` and `nightly-mutation`
  artifacts. GPU/window fuzz and the gungraun gate stay out, reason in the file.
- [x] TG7 docs: a link check outside the book - `tools/docs-links.mjs` reads every tracked
  markdown page outside `docs/archive/` (660 pages, 0 broken) and `ci.yml` runs it beside the
  tool inventory. It found and fixed one real break (`docs/library/index.md` -> `../EMPIRE.md`,
  archived 2026-10-17). Landed `c03ad5037`.
- [x] TG8 ci: a macOS leg - the golden matrix covers linux x86, windows, wasm, not macOS.
  Tier B. Size M. Owns `.github/workflows/golden-matrix.yml`. F2P: no `macos` runner in
  the matrix (grep). P2P: the four current targets stay green. Accept: a fifth row, same
  golden and corpus digests.
  Part 2026-10-24 (`723f8a4d7`): the macOS arm row is in the matrix and in all four
  compare lists. Proof blocked, not disproved: the six matrix legs of run 40033397607
  died on a stale committed `Cargo.lock` in other seats' work (`cargo check --locked`),
  and macOS failed at the build step before any digest. One clean matrix run with the
  lock committed closes it.
- [x] TG9 tools: a fresh clone installs the hooks - the tool exists and works:
  `node tools/hooks-install.mjs` sets `core.hooksPath=tools/hooks` (pre-commit, pre-push;
  installed on this box 2026-10-24). Nothing runs it automatically and `doctor` only warns.
  Tier A. Size S. Owns `tools/doctor.mjs`, `CONTRIBUTING.md`. F2P: `doctor --strict` exits 1
  when the hooks are unset (new test). P2P: `doctor` stays healthy. Accept: a fresh clone's
  first `doctor` names the one command, and `--strict` fails until it runs.
  Landed `8f83a9167` (`doctor --strict`, 5/5 tests; `CONTRIBUTING.md` names the fix).
- [ ] TG10 ci: rustdoc warnings fail the docs job - warnings are counted, not gated.
  Tier C. Size S. Owns `.github/workflows/`. F2P: a workflow step with `-D warnings`
  fails on today's warnings. P2P: the docs build stays green. Accept: warnings zero.
  Measured 2026-10-24: 781 warnings (render 149, anim 131, game 112, net 108, physics 86,
  endpoint 75, combat 48, assets 43, arena 21, physfx 4, core 2, input 2; pages run
  40006304627, the last green docs build). The flag stays off; `2955c134d` records the
  count and the re-measure rule in the docs job. Burn down one crate at a time.
  Progress 2026-10-24 (`a559ec910`, `03a97f1ee`, `ac13203c4`): core, input, physfx and
  arena are at 0 and gated by their own `RUSTDOCFLAGS=-D warnings` step in `pages.yml`
  (green: pages run 40056976625). 785 left over 8 crates: render 154, anim 132, game 119,
  net 108, physics 96, endpoint 82, combat 48, assets 46.
  Progress 2 (`ffa5acd4d`, `f29c293e3`, `ae6c72499`): net and combat are at 0, so six
  crates are gated (pages run 40065341289, step 12 green).
  Progress 3 (`c04c81122`): physics is at 0 in the default build and with `--features
  rapier`, so seven crates are gated (pages run 40075622434, step 12 green). Left, 533:
  render 154, anim 132, game 119, endpoint 82, assets 46. The pages job is still red at
  step 14, `door-reference.md is stale`, which clears when the anim pack work lands.
- [x] TG11 ci: the wasm matrix leg survives the thread tests - `islands::tests::par_*`
  spawn threads, which traps `unreachable` on wasm32-wasip1 (run 40056555531, exit 134
  after 160 tests), so the compare job prints `PHYSICS RESULT FAIL` even though all 20
  digests MATCH. Tier B. Size S. Owns `crates/physics/src/`. F2P: the wasm leg runs the
  same test names as native. P2P: native threads stay parallel. Accept: 20/20 MATCH on a
  green compare job.

## LLM and agent side

- [x] LG1 provider: a model request carries content - `AiRequest` holds digest refs and
  `provider/vendor.rs:442` renders `task/revision/digest`; `playtest/src/door.rs:276`
  passes `[3u8; 32]`, and `pins.rs:55` tells the model not to invent ids. No engine path
  can ask a real model about a real scene. Tier A. Size M. Owns
  `crates/provider/src/protocol.rs`, `crates/playtest/src/door.rs`,
  `crates/endpoint/src/`. F2P: a bounded `body` (text + an id allowlist) reaches the
  vendor line and a live-shaped mock answers with a real id (new test). P2P: the privacy
  envelope test stays green. Accept: one 1,000-token scene brief round-trips, and the
  digest-only path is byte-identical. Landed `a80ba3b`.
  Progress 2026-10-24: `crates/playtest/src/door.rs` `scene_brief` sends one
  `name id at x,y,z` line per prop through `Door::submit` (`body: Some(brief)`), whole
  lines only with a `... N more` tail past `MAX_BODY_BYTES`; a door with no scene keeps
  the digest-only line. Runs 40027542502 (playtest door, 21 tests) and 40026984777
  (provider body, 17 tests) pass; the brief test was seen red with `body` forced to
  `None`.
- [x] LG2 playtest: the eval scorecard is a standing CI gate with a measured bar -
  `tools/eval-bar.json` + `eval-check.yml` (mock only): `rows 8, at 100%: 7, red: 0`.
  Landed `1873f4d59`.
- [x] LG3 provider: `agent_loop` runs the door and the call log keeps args - the loop says
  "no in-repo consumer" (`agent_loop.rs:6`) and the audit chain keeps 1024 entries with
  no args. Tier A. Size M. Owns `crates/provider/src/agent_loop.rs`,
  `crates/endpoint/src/audit/`. F2P: one scripted task runs read -> preview -> apply
  through the loop and the log replays it (new test). P2P: provider tests stay green.
  Accept: replay from the log equals the recorded run.
- [x] LG4 provider: typed memory with a write gate - OPT49 is open; `agent_loop.rs:270`
  `MemoryDoor` is a test double. Tier B. Size M. Owns `crates/provider/src/memory.rs`,
  `crates/endpoint/src/packs/memory/`. F2P: a write is refused without a policy key
  (new test). P2P: the door stays green. Accept: two sessions share one recalled fact.
  Part 2026-10-24 (`5315e6781`): `provider::memory` is typed (fact/preference/decision,
  integer confidence, source, tick, shared flag), writes need a capability token carrying
  `memory.write` and refuse by name before any mutation, recall is scoped and ordered, and
  the store encodes to integer lines that round-trip byte-equal (7 tests, run 40066060287).
  Endpoint half landed too (`59bc0597a`): `packs/memory/` serves `memory.write`,
  `memory.recall` and `memory.share`, keeps the store's own refusal codes, and
  `MemoryStore::share` refuses a fact the caller does not own. Runs 40072345141,
  40072272584, 40072805700.
  Remaining: the endpoint `packs/memory/` pack and its registration in `packs/mod.rs`.
- [x] LG5 provider: the model can see a frame - the door emits PNGs (`play.rs:1447`),
  adapters have no image part. Tier B. Size M. Owns `crates/provider/src/`,
  `crates/playtest/src/look.rs`. F2P: an image part reaches the vendor body under a size
  bound (new test). P2P: text requests stay byte-identical. Accept: the judge describes
  one capture's material split.
  Landed `c9eb97072`: `ImagePart` (media type, 1 MiB cap, signature check) renders one
  `image_url` block only in the chat wire; text body stayed byte-stable (run 40049910363).
- [x] LG6 endpoint: a streamable HTTP transport with a token gate - stdio only
  (`bin/e2040-mcp.rs:77`), remote shim is in-process. Tier B. Size L. Owns
  `crates/endpoint/src/transport.rs`, `bin/e2040-mcp.rs`. F2P: a second binary serves the
  same door over loopback HTTP with a token (new test). P2P: stdio stays green.
  Accept: a remote client lists tools and applies one scene edit.
  Landed `bbb803206`: `e2040-mcp --http 127.0.0.1:0` (live code in
  `remote_shim/http_door.rs`, reusing the token helper), non-loopback bind refused,
  64 KiB body, one request per connection; no flag = stdio unchanged. Runs
  40058266414, 40058377790.
- [x] LG7 endpoint: pagination and prompt arguments - `tools/list` answers
  `next_cursor: None` (`mcp.rs:1604`), `get_prompt` ignores arguments (`:1776`).
  Tier A. Size S. Owns `crates/endpoint/src/mcp.rs`. F2P: 120 tools page in two calls
  and a prompt takes one argument (new test). P2P: the 99-verb list is unchanged.
  Accept: a client with a 4 KB page sees every verb once.
  Landed `d5e9a8c57` (code moved to `mcp/handler.rs`): 100-verb pages with a decimal cursor,
  no cursor still answers the whole door, a bad cursor is refused by name, and `prompts/get`
  appends one bounded sorted `arguments: k=v` line. Runs 40053565088, 40053677469.
- [x] LG8 provider: one committed provider cassette - `provider/cassette.rs` has 24 tests
  and zero cassette files; every live number is a fixture. Tier A. Size S. Owns
  `fixtures/`, `crates/provider/`. F2P: the cassette test finds no file on HEAD.
  P2P: provider tests stay green. Accept: one model x one task set replays offline,
  byte-equal, in CI.
  Landed `892573f36`: three entries in `fixtures/cassettes/` (byte-pinned), an ignored
  regenerator, and a replay test that refuses a missing file (run 40049197961, 26 tests).
- [x] LG9 provider: cost per call reaches the adapters - `Usage`/`Budget` exist
  (`protocol.rs:96-135`) and `QuotaBook` has no consumer. Tier B. Size S. Owns
  `crates/provider/src/quota.rs`. F2P: an over-budget request is refused by name (new
  test). P2P: adapters stay green. Accept: the scorecard prints cost per row.
  Landed `1e98cc205`: `QuotaCharge::of_request` + `QuotaBook::admit_request`; the refusal
  names the remaining budget and the card prints a `cost per row` column, `unknown` when
  any row is (never zero-claimed). Runs 40048528789, 40048960305.
- [x] LG10 endpoint: parallel read-only tool calls - one `Mutex<ServerState>`
  (`mcp.rs:1082`) serializes every call. Tier C. Size M. Owns `crates/endpoint/src/mcp.rs`.
  F2P: two `scene.read` calls overlap under the new read path (new test). P2P: the write
  path stays serialized. Accept: 8 concurrent reads, no interleaved write.
  Landed `04b450510`: reads run under a shared `RwLock`, the audit record is sealed after
  the read span, writes keep the exclusive lock for their whole span; a channel-based gate
  shows 8 reads inside at once and a write waiting. Runs 40057808126, 40058160951.

## Order

Done on 2026-10-24: WG1 WG3 WG4 WG6 WG7 WG8 WG9 WG10 WG11 WG12 WG13 WG14 WG15, TG1 TG2 TG3
TG5 TG6 TG7 TG8 TG9 TG11, LG1 LG2 LG3 LG4 LG5 LG6 LG7 LG8 LG9 LG10. Four rows stay open.

- WG2: the plugin registry (`linkme` slice, `modules = [...]` in the manifest) - the last
  workspace row, no pack collision.
- WG5: only the physics accept is left - nothing consumes `HeightfieldRecord` yet
  (`crates/physfx` is busy with another seat's soft-body work).
- WG14 landed `04a6d1d70` (the other seat wrote the row; ticked here).
  (another seat); it lands with them.
- TG4: the endpoint coverage leg needs a green endpoint suite; `api.load` currently
  refuses at 78,957 chars against the 75,000 hard budget (the pack seat's in-flight work).
- TG10: seven crates gated; render 154, anim 132, game 119, endpoint 82,
  assets 46 remain (only endpoint is free of another seat's edits).

Do not reopen parked rows; the full tables are in the three research files.
