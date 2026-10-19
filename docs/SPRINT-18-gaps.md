### Look gaps (owner 2026-10-10, `docs/plan/render-look.md`)

Boarded 2026-10-10 from the plans review (`target/reports/plans-review-2026-10-10.md`
part E): the Look proof had no frame-cost rows and no machine score. GM3 needs
FC3 (at 3.29 fps the 18,000-frame run takes 91 min against a 45 min slot).

- [ ] FC1 perf: frame cost table for the windowed arena — the perf seat runs Progress 2026-10-17: table exists, `target/reports/perf-20261014-fc1.md` (QUIET x3, top 3 costs named with files: present, sun shadow, fighters). Not met: spread 50% against the 10% bar, report not named perf-<date>.md, no re-run after EG1 and EG2; arena.exe is STALE (100 code commits) so a fresh quiet run is needed.
  the real window game three times in a quiet window and names the top 3
  costs (render pass or system) with the file that owns each. `perf.exe` has
  no presenter, so it is not the target (render review 2026-10-10). Tier A.
  Size S. Owns `target/reports/perf-*.csv` and one packet (no code). Accept:
  `node tools/bins.mjs` shows arena FRESH (else run its rebuild line);
  `node tools/quiet.mjs --times 3 --out target/reports/perf-<date>-{n}.txt --
  target/release/arena.exe --scene arena --fighters 8 --frames 300 --exit`
  prints QUIET; the `arena frame-split` lines give sim, present and fighters
  ms; spread over the 3 runs under 10%; top 3 costs named with file paths in
  `target/reports/perf-<date>.md`. Run it again after EG1 and EG2 land.
  F2P: no arena frame table under target/reports today. P2P: no source
  change; golden `d57b5c8ae3096614` unchanged.
- [x] FC2 render: halve the top frame cost FC1 names — one pass, one file, a
  half-res or skip switch on that pass. EG2 (Claude) removes the blocking
  fog and GTAO readbacks; tick FC2 with EG2 if the numbers hold. Tier A. Size M. Owns the one file FC1
  names plus its test. Accept: the same quiet perf run shows that cost's p50
  at most half of FC1's number and the arena frame p50 drops by at least the
  same ms; the staged_fight frame_check shows no new failure. F2P: a test on
  the switch (pass skipped or run at half resolution) fails before. P2P:
  `cargo test -p engine-render --lib` green; golden unchanged. EG2 measured
  its own before and after (present 39.2 to 8.0 ms), so FC2 closed ahead of FC1.
- [ ] FC3 render: arena with 8 fighters at 30 fps (1080p, RTX 3050) — repeat Progress 2026-10-17: the quiet FC1 run of 2026-10-14 already reads interval p50 2.66-4.13 ms and p99 8.1-11.1 ms for 8 fighters (bar 33.3 and 50 ms), but only one run set, on a binary now STALE; the bar needs a fresh quiet x2 on the current HEAD with frame_check unchanged.
  FC2 on the next top cost until the quiet perf run shows the windowed arena
  p50 frame at most 33.3 ms. Tier A. Size L. Owns one render file per landing
  (FC2 shape). Accept: quiet perf run x2: windowed arena p50 at most 33.3 ms,
  p99 at most 50 ms; frame_check unchanged. F2P: FC1's arena p50 is above
  33.3 ms. P2P: golden unchanged; `cargo test -p engine-render --lib` green.
  needs: FC2.
- [x] LJ1 playtest: `look.judge` door verb — scores frames by machine, so
  Look rows accept on a number instead of an eye. Reads the staged_fight
  capture set, runs `frame_check` and a reference compare against
  `content/fighters/previews/` and the Q1 manifest, and returns rubric L1-L9
  scores (`docs/plan/render-look.md` Proof) with entity-grounded regions as
  typed JSON. Tier A. Size M. Owns `crates/playtest/src/look_judge.rs` (new,
  beside `look.rs` and `frame_check.rs`) and one verb in
  `crates/endpoint/src/packs/view.rs`. Accept: `cargo test -p
  engine-playtest --lib look_judge` x2: a cube-only frame scores 0 on L1
  (fighters present), a frame with drawn fighters scores above 0; the same
  frames give byte-identical JSON twice; the verb is listed by api.describe.
  F2P: no look.judge verb today. P2P: `cargo test -p engine-endpoint --lib`
  green; golden unchanged.

### Multiplayer decisions (owner 2026-10-10, `docs/plan/multiplayer.md`)

Decided 2026-10-10 by the Claude session the owner delegated to ("u decide on
all"): the rollback window stays 8 ticks; QUIC through the relay waits.

- [x] MP24 net: adaptive input delay keeps every resim inside Landed 440c6bbb.
  `RESIM_CAP_TICKS` (8) — no deeper window. When the simulated round trip
  exceeds the window, the client delays its own input by ceil(rtt_ticks) - 8
  so prediction never runs deeper than 8 ticks (rollback fighting-game
  practice). The delay comes from the simulated link in game steps, so a bout
  stays a pure function of config. Tier A. Size M. Owns the client prediction
  hunks in `crates/net/src/played.rs`, `crates/net/src/scale.rs` and
  `tools/mp6-check.mjs`. Accept: every mp6 WAN profile reports `over_cap` 0
  (deepest rollback at most 8) with its input delay logged; mp6-check fails a
  receipt with `deeper_than_8_tick_window` above 0; LAN profile digests
  unchanged; hash x2. F2P: `net_bout mp6` then `node tools/mp6-check.mjs`
  reports `deeper_than_8_tick_window` above 0 today (deepest 19). P2P:
  `cargo test -p engine-net --lib snapshot` green; golden unchanged.
  needs: MP6.
- [ ] QR1 net: QUIC through the ON2 relay — a socket adapter so the real-NAT
  leg can ride QUIC instead of the UDP courier. LATER: ON30 counts the UDP
  courier (decision 2026-10-10). Tier B. Size M. Owns adapter hunks in
  `crates/net/src/courier.rs` and `crates/net/src/quic.rs`. Accept:
  `net_bout on30-serve --quic` and `on30-join --quic` converge with full
  checkpoint equality over the relay; digest equals the one-process twin x2.
  F2P: `--quic` is refused today. P2P: ON30 receipts unchanged; `cargo test
  -p engine-net --features quic --lib` green. needs: ON30.
- [x] MP28 net: replay header pins the world and the tick logic — a tape Landed bffca14a. cargo test -p engine-net --lib replay 20 passed (gh run 39150021689, the mismatched spec digest refuses as SpecMismatch naming both digests, mid-playback Diverged carries the tick); clippy clean (gh run 39150040441); golden d57b5c8ae3096614 unchanged.
  records the match identity its checkpoints came from (`spec_digest`, the
  `MatchSpec::identity` of the config, rules and world) and the tick logic
  version (`logic_version`, `SIM_LOGIC_VERSION`) beside `REPLAY_VERSION`, so
  every playback refusal is named: `ReplayError::SpecMismatch`/
  `LogicMismatch` carry both values, and a refusal raised mid-playback carries
  the tick. `record_tape_under`, `play_tape_under` and `Theater::new_under`
  play an explicit spec; the spec-less helpers gate on the frozen arena. The
  two fields are not folded into `tape_hash`, so existing tape bytes and the
  pinned tape digest stay unchanged, and the envelope version stays 1 (no
  serialized tape breaks). Diff: regular. Tier B. Size S. Owns hunks in
  `crates/net/src/replay.rs` and the `ReplayTape` literal in
  `crates/net/src/lib.rs`. Accept: `cargo test -p engine-net --lib replay`
  green, with a mismatched spec digest refused by the named variant naming
  both digests. F2P: a tape carries no world identity today, so a replay
  against a changed world only fails on a later tick. P2P: the frozen tape pin
  `0x3092_6e49_6049_04ce` and `engine-net` consumers unchanged; golden
  `d57b5c8ae3096614` unchanged. needs: none.

### AI-native door (owner 2026-10-10, `docs/plan/ai-native.md`)

Boarded 2026-10-10 from `docs/plan/ai-native.md` candidate rows 2, 4, 5 and 7
(1 is LJ1 above; 3 landed as `docs/CRATE-MAP.md` and the error catalog).

- [x] AL1 arena: a live model drives a gladiator inside one recorded bout, Landed b8960f9b.
  slice 1: stamped replies — the provider reply is validated, stamped for
  tick N+k and drained at the tick boundary into the bout's tape; replay with
  the provider off reads the tape and gives the same hash. Builds on EG1
  `LiveMatch`. Diff: good (determinism: the provider never runs inside the
  tick; only tape actions enter it). Tier A. Size M. Owns a bout hunk in
  `crates/arena`, the stamp hunk in `crates/ai/src/mind.rs` and one test.
  Accept: `cargo test -p engine-arena --lib live_mind` x2: a mock provider
  with fixed latency drives fighter 0 for 600 ticks; the recorded tape
  replays with the provider off to the same final hash. F2P: no bout test has
  a provider-driven fighter today. P2P: `cargo test -p engine-ai --lib`
  green; golden unchanged. needs: EG1.
- [x] AL1b arena: live mind slice 2, late replies — a reply past its deadline Landed 7d13d5f. live_mind_late green (3 passed with the EG46b pair), run 39057007421: 3-ticks-late replies refused each time, logged as fallback with reason provider.timeout, bout equals the scripted-only bout, no tape rows.
  (tick N+k already run) is never applied late: that tick runs the scripted
  brain and logs the fallback with a reason (an `engine-observe` code). Diff:
  regular. Tier A. Size S. Owns the deadline hunk beside AL1's. Accept:
  `cargo test -p engine-arena --lib live_mind_late` x2: a mock provider 3
  ticks late is logged as fallback each time, and the final hash equals the
  scripted-only bout's and its provider-off replay. F2P: no deadline path.
  P2P: as AL1. needs: AL1.
- [x] DP1 endpoint: door packs for the Core systems — `net` (open a loopback Landed 92fb1962.
  session, inject loss, latency and drop, read the first divergent tick) and
  `perf` (per-scenario p50 and p99 from the newest quiet perf csv), typed,
  read-only or undoable, refused unless enabled. Tier A. Size M. Owns
  `crates/endpoint/src/packs/net.rs` and `perf.rs` (new) plus their `mod.rs`
  lines. Accept: `cargo test -p engine-endpoint --features mcp --lib packs::net` and
  `packs::perf` x2 (packs live behind the `mcp` feature; the bare `--lib` filter matches 0 tests): a loopback session with 10% loss converges and reports
  its first divergent tick as none; a seeded desync reports its tick; perf
  reads a fixture csv into typed rows; both verbs listed by api.describe.
  F2P: no net or perf pack today (`crates/endpoint/src/packs/`). P2P: `cargo
  test -p engine-endpoint --lib` green; golden unchanged. Landed so far (2026-10-12 check): 3a1cd34c, 819371be (net and perf packs). Remaining: `net.loopback` takes latency and burst-drop knobs, the seeded-desync test pins the exact tick, the proof line carries the full endpoint lib count and the golden.
- [x] RB1 playtest: `replay.verify` and `replay.bisect` door verbs — given a
  tape, print the first tick whose hash leaves the recorded chain. Tier A.
  Size S. Owns a verb hunk beside `crates/playtest/src/causal.rs` and
  `seeded_bug.rs`. Accept: `cargo test -p engine-playtest --lib
  replay_bisect` x2: a seeded_bug tape bisects to the seeded tick; a clean
  tape verifies with no divergence. F2P: no bisect verb today. P2P: `cargo
  test -p engine-playtest --lib` green; golden unchanged. Landed 4c6d8177; accept 2026-10-15: `replay_bisect` 5 passed x2, playtest lib 345 passed, `cargo run -p engine2040 --release` final_hash=d57b5c8ae3096614.
- [x] AG1 playtest: agent scorecard — one command prints pass rate and
  tokens per task for EV1, spatial_eval, the mirrors (HS29-HS32) and
  BreakBench by model tier (free, strong, replay), written to
  `target/reports/scorecard-<date>.md`; a drop against the last card is a red
  line. Tier B. Size S. Owns `crates/playtest/src/eval.rs` scorecard hunk.
  Accept: `cargo test -p engine-playtest --lib scorecard` x2 on fixture
  results gives the same table twice; a fixture with a lower pass rate prints
  RED. F2P: no scorecard command today. P2P: `cargo test -p engine-playtest
  --lib` green. needs: HS20. Landed 4aef2869. eval/scorecard.rs: one card per day at target/reports/scorecard-<date>.md with pass rate and tokens per task for EV1, mirror and spatial_eval by tier (mock replay, local free, remote strong), BreakBench prints unbuilt (BX25), a drop against the newest earlier card prints RED. Command: SCORECARD_DATE=YYYY-MM-DD cargo test -p engine-playtest --lib scorecard_command -- --ignored --nocapture. Tests scorecard_table_is_the_same_twice_on_fixture_results, scorecard_lower_pass_rate_prints_red, scorecard_lost_lines_are_red_and_new_lines_are_new, scorecard_reads_back_what_it_wrote, scorecard_collects_every_suite_green_and_unbuilt_breakbench, scorecard_command_writes_the_card_and_compares_with_the_last. Tokens print unknown for replayed suites (a live recording supplies the figure). gh run 39105225319 (engine-playtest --lib highlights brief_eval trajectory scorecard frame_stale: 36 passed, 1 ignored) and full engine-playtest --lib 373 passed, run 39103617132.

- [x] AX24 endpoint: the receipt says what changed — the `tx.apply` receipt
  carries a bounded, read-only change list: the document revision before and
  after, the added, removed and moved counts (a world pose differs) and one row
  per change, clipped to `MAX_CHANGES` (60) with `truncated: true` when rows
  drop, so a count never lies. Built from the door's own before/after snapshots
  (the SPX3 pair already taken for the spatial delta), so no second diff engine.
  Donor: `larelgit/shards-engine` `crates/ai_core/src/tools.rs` (digest pair plus
  `clip_changes`, `MAX_CHANGES = 60`), read 2026-10-18. Diff: good. Tier A.
  Size S. Owns `crates/endpoint/src/receipts.rs` (`ChangeList` plus
  `MAX_CHANGES`), the `changes` field on `TxApplyOutput` and `ChangeListDto` in
  `crates/endpoint/src/mcp_dto.rs`, and the snapshot pair in
  `crates/endpoint/src/scene_tools/spatial_delta.rs`. Accept: `cargo test -p
  engine-endpoint --features mcp --lib receipt`: a one-op insert reports exactly
  one added with revision 0 -> 1 and no truncation; a one-op move reports one
  moved and a one-op remove reports one removed; `MAX_CHANGES + 7` rows set
  `truncated` true while `moved` stays the full total. F2P: `TxApplyOutput`
  carries no change list (effects, key and undo token only; `ChangeList` does not
  exist). P2P: `cargo test -p engine-endpoint --features mcp --lib` green (both
  tool-list bounds raised with dated reasons: the proximity bound to 53,000 at
  52,348 measured; `TOOL_LIST_BUDGET_CHARS` 57,500 -> 59,000 after the loaded
  list measured 56,513, 988 headroom under the 2,048 minimum);
  `cargo clippy -p engine-endpoint --features mcp --all-targets -- -D warnings`
  clean; golden hash unchanged. Consumer: an
  agent reads `changes` instead of re-reading the scene. needs: none.
  Landed 67ae5139 (reached HEAD in the auto-backup commit). `cargo test -p
  engine-endpoint --features mcp --lib receipt` 18 passed 0 failed; full `cargo
  test -p engine-endpoint --features mcp --lib` 628 passed 0 failed 1 ignored,
  gh run 39152299530; `cargo clippy -p engine-endpoint --features mcp
  --all-targets -- -D warnings` clean; `node tools/rust-quality.mjs --owned
  crates/endpoint/src` PASS; `node tools/workspace-check.mjs` PASS; golden
  d57b5c8ae3096614 unchanged.

### Engine review: frame and look (owner 2026-10-10, `docs/plan/render-look.md`)

Boarded 2026-10-10 from six book reviews in `target/reports/engine-review-2026-10-10/`
(render, runtime, bodies, ai, testing, rust-craft; Gregory GEA, Lengyel FGED, Game Engine
Gems 1-3, Rust books). Each row names its report gap; read that gap before coding.
`Diff:` routes the work: hard = Claude Opus, good = Claude Sonnet or a Muse xhigh
builder, regular = any loop coder. Loop builders skip `Diff: hard` rows unless a
planner splits them into good or regular rows. Files a `claude-*` line names in
`target/queue/claims.txt` are held: the keeper keeps such packets in ready/ and the
seat guard refuses their edits, so take the next row. Search the books first:
`node tools/books.mjs search "<topic>"`.

- [x] EG2 render: no GPU wait in the live frame — `present_frame` reads depth
  back and waits (`map_readback_f32`, `poll(wait_indefinitely)`), unprojects
  every pixel on the CPU (`decode_depth_feed`), re-uploads for fog and GTAO and
  reads back again; GTAO output is thrown away. Fog moves into the forward
  fragment shader (gems3 3.2 closed form, horizon colour uniform); the
  tranches go behind an opt-in; GPU timestamps print GPU ms p50/p99 in the
  `arena frame-split` line. Diff: good. Claude holds (Sonnet). Tier A. Size M.
  Owns `crates/render/src/present.rs`, `crates/render/src/present/**`,
  `crates/game/src/bin/arena.rs`. Accept: arena `--exit` smoke prints 0
  blocking maps or waits on live frames and fails above 0; frame-split p50
  before/after x3 on the RTX 3050. F2P: the wait counter test fails before.
  P2P: `cargo test -p engine-render --lib present` green; golden
  `d57b5c8ae3096614` unchanged. Review: render.md G1, G2. Closes FC2.
- [x] EG6 render: GPU tests say when they skip — 63 `no wgpu adapter` early
  returns in 23 files pass green with no GPU, and proof never counts them. One
  `engine_render::test_gpu::require_or_skip()` prints `SKIPPED <test>` and fails
  when `E2040_REQUIRE_GPU=1`; `tools/proof.mjs` counts SKIPPED lines into its
  RESULT line. Diff: regular. Tier A. Size M. Owns
  `crates/render/src/test_gpu.rs` (new), `crates/render/src/lib.rs` mod line,
  `crates/render/src/present/tests_gpu.rs`, `tools/proof.mjs`. Accept:
  `E2040_REQUIRE_GPU=1 cargo test -p engine-render --lib require_or_skip` fails
  with no adapter; with the variable unset it passes and prints SKIPPED; `node
  tools/proof.mjs` RESULT names the skipped count. F2P: the require_or_skip test
  fails before (no helper module). P2P: `cargo test -p engine-render --lib`
  green; golden `d57b5c8ae3096614` unchanged. Review: testing.md G4. needs: EG2.
- [x] EG8 render: a pass is live only if pixels change — the GX6 truth gate Landed 1bb6097.
  counts a pass live when it does not return Unsupported, so bind probes pass
  (`sky.rs` copies a buffer, the cascade "shadow" is a distance dim). New
  gate: render the arena frame with the pass on and off; live only if a
  masked region differs above epsilon. Write `target/reports/pass-truth-<date>.md`
  listing every look pass as LIVE or PROBE. Diff: regular. Tier A. Size M.
  Owns the GX6 gate file plus its test. Accept: the gate run x2 gives the
  same table; forward is LIVE, sky and decals are PROBE today. F2P: the
  current gate reports sky live. P2P: `cargo test -p engine-render --lib`
  green; golden unchanged. Review: render.md G7. Landed so far (2026-10-12 check): 6ea9df22, d8b80809 (table tests). Remaining: render the arena frame with each look pass on and off, set each pass's live flag from the masked diff, write `target/reports/pass-truth-<date>.md`; sky and decals stay PROBE only while their diff is under epsilon. needs: EG2.
- [x] EG9 render: linear HDR frame and one forward shader file — tone map and
  sRGB sit inside the forward fragment; the WGSL is built by three `replacen`
  patch layers; `v1_skin.rs` holds a second BRDF copy. Forward writes linear
  radiance (exposure only); the blit does ACES plus sRGB; WGSL moves to
  `crates/render/src/shaders/forward.wgsl` via `include_str!` with entry
  points (static, skinned, depth-only); anchor patching is deleted. Diff:
  good (Claude Sonnet after EG2). Tier A. Size M. Owns the forward shader,
  `presenter.rs`, `submit.rs`, `v1_skin.rs` BRDF hunk. Accept: arena capture
  at the same exposure matches the EG2 capture within tolerance in the sand
  region; no `replacen` on WGSL left. F2P: a test that the forward target is
  Rgba16Float linear (no tone map in the fragment) fails before. P2P:
  `cargo test -p engine-render --lib` green. Review: render.md G3. needs: EG2.
- [x] EG10 game: persistent fighter vertex buffers — `fighter_look.rs` and the
  presenter create about 64 GPU buffers per frame. Allocate one fixed-size
  vertex buffer per fighter part once; `queue.write_buffer` each frame. Diff:
  regular. Tier A. Size S. Owns `crates/game/src/fighter_look.rs` upload
  hunk and the presenter draw-buffer cache. Accept: a buffer-creation counter
  reads 0 per frame after frame 1 in the arena `--exit` smoke; frame-split
  present ms does not rise. F2P: the counter test fails before. P2P:
  `cargo test -p engine-game --features app --lib` green; golden unchanged.
  Review: render.md G4 step 1, bodies.md G11. needs: EG2.
- [x] EG11 render: sun shadow slice 1, the depth pass — no sun depth map is Landed 1d01a41. sun_single_box_is_nearer_than_plane (box texels nearer than the plane, x2 equal, 0 validation errors), sun_prepass_box_nearer_than_plane_live and eg11_live_sun_tests::live_frame_encodes_the_sun_depth_map (live frame records the 2048 map, shadow_submit_us=49 above 0, 0 validation errors) green on the RTX 3050 (engine-render --features window --lib, local PC run, no SKIPPED line).
  rendered today: `SunShadowPass::build` (`shadow_cluster.rs:259`) is CPU-only
  at init, `submit_shadow_cluster` (`:1051`) runs only in tests, and `fs`
  only dims the sun by cascade distance (`forward.wgsl:305-313`). Render one
  2048 `Depth32Float` map per frame from the live draw list, ortho sun camera
  fitted to the 30 m pit from the sun direction the forward uniform carries,
  before the forward pass; nothing samples it yet. If `twin_shadow.rs` holds
  `SUN_SHADOW_SIZE` (a single arena map staged on 2026-10-11, never landed),
  finish and land that instead of a second file. Print `shadow=<ms>` in
  `arena per-pass`. Diff: good. Tier A. Size M. Owns a new
  `crates/render/src/present/gpu_present/sun_shadow.rs` (or the
  `twin_shadow.rs` sun hunk) and the submit.rs pass hunk. Accept: a GPU test
  renders a box on a plane and reads the map back: texels under the box are
  nearer than the plane's, x2 equal; `arena per-pass` shadow above 0 ms on
  live frames; validation errors 0. F2P: no live shadow pass. P2P: `cargo
  test -p engine-render --lib` green; golden unchanged. needs: EG9.
- [ ] EG11b render: sun shadow slice 2, the forward shader samples it — bind Progress 2026-10-17: `forward.wgsl` samples the sun map through `sun_shadow_pcf` (2x2 `textureSampleCompare`), bound in the forward group (7ca0937). Left: the on-vs-off masked-region luminance test under a fixture box, 3x3 PCF, the stale `SHADOW_FORWARD_WGSL` doc claims.
  the map, a comparison sampler and the light view-projection in the forward
  group; `fs` multiplies the sun term by a 3x3 PCF `textureSampleCompare` and
  drops the cascade distance dim; correct the stale claims that
  `SHADOW_FORWARD_WGSL` is bound (`present.rs:12-13`, `twin_shadow.rs:497-505`).
  Diff: good. Tier A. Size M. Owns the forward.wgsl sun hunk and the
  presenter bind hunk. Accept: EG8 on vs off: the masked region under a
  fixture box darkens past the luminance floor, open ground stays within 1%;
  capture x2 equal; validation errors 0. F2P: on vs off frames are identical
  today. P2P: render lib green; golden unchanged. needs: EG11.
- [ ] EG11c render: sun shadow slice 3, steady and clean — snap the light Progress 2026-10-17: `sun_snap_origin_for_size` snaps the ortho origin to whole texels (`sun_half_switch_halves_texel_and_keeps_snap`). Left: bias is `SUN_SHADOW_BIAS` 0.002, not 2^-19 plus slope and normal offset; no 1 cm camera-move byte-equal test, no sun-facing plane 0 shadowed pixels test, no contact-shadow test.
  ortho origin to whole texels (no shimmer as the camera moves), depth clamp,
  bias 2^-19 plus slope-scaled bias and a normal offset (found2 8.3.3-8.3.4):
  no acne on sun-facing sand, no gap under feet. Diff: good. Tier A. Size S.
  Owns the sun shadow matrix and bias hunks. Accept: moving the camera 1 cm
  leaves every map texel byte-equal; a sun-facing fixture plane has 0
  shadowed pixels; a fixture foot on the plane casts contact shadow within 2
  px. F2P: the snapping test fails before. P2P: render lib green. Review:
  render.md G5 (RL27 keeps its split-distance unit). needs: EG11b.
- [ ] EG12 render: ambient, torches, normal and ORM maps — the forward shader Progress 2026-10-17: landing (a) has its pixel test (`ambient_sh9_on_off_differs_in_masked_region`, ibl.rs) and the forward shader holds an 8-slot `TorchU` block (b). Left: a torch on-vs-off pixel test, the tangent attribute and normal and ORM sampling (c).
  has a constant ambient, unlit torches, no tangents. Three landings, one per
  batch: (a) SH9 ambient uniform from the `ibl.rs` bake, AO scales ambient
  only; (b) up to 8 point lights in a uniform array for the torch actors in
  `placement.json`; (c) tangent attribute with handedness in w, Repeat
  sampler, normal and ORM sampling on sand and stone. Diff: good. Tier A.
  Size L. Owns forward shader hunks plus `crates/assets` tangent hunk.
  Accept: each landing has a pixel test (on vs off differs in a masked
  region) and a capture. F2P: the ambient on/off test fails before. P2P:
  `cargo test -p engine-render --lib` green. Review: render.md G6. needs: EG9.
- [ ] EG13 render: 4x MSAA — no antialiasing in the window; thin stands and Progress 2026-10-17: `present/gpu_present/msaa.rs` with 7 tests (`msaa_edge_intermediate_on_only` edge test, `msaa_forward_choice_resolves_only_when_on`, `msaa_edge_live_resolve_splits`, `msaa_toggle_drops_targets`, 4x resolve before the blit, off switch) green on the RTX 3050 (engine-render --features window --lib, local PC run). Left: the frame-split p50 rise at most 2 ms on a quiet GPU run (arena.exe STALE).
  weapons crawl. 4x MSAA on the HDR and depth targets with a resolve before
  the blit; a setting to turn it off. Diff: regular. Tier A. Size S. Owns
  the presenter target setup hunk. Accept: an edge-pixel test shows
  intermediate values with MSAA on and none with it off; frame-split p50
  rises at most 2 ms. F2P: the edge test fails before. P2P: `cargo test -p
  engine-render --lib` green. Review: render.md G8. needs: EG9.
- [x] EG14 render: HUD on screen — `windowed.rs` says HUD quads ride the Landed f737386. live_overlay_capture_tests::hp_rect_holds_bars_not_scene (a staged hp bar changes at least 16 pixels inside its screen rect and none outside, 0 validation errors) green on the RTX 3050 with live_overlay_draws_staged_bars_after_blit and overlay:: tests (engine-render --features window --lib, local PC run); overlay.rs drawn after the blit, windowed.rs stage_hp_bars feeds it.
  receipt, not the draw list, so no HUD, menu or text reaches the screen. One
  textured-quad pipeline after the blit (alpha blend, no depth, 0..1
  coordinates), a dynamic quad ring fed by `ui` layout cells, a bitmap font
  atlas, one draw call. Diff: regular. Tier A. Size M. Owns a new
  `crates/render/src/overlay.rs` plus the presenter call. Accept: the arena
  capture shows the hp bars (pixel test on their screen rect). F2P: that rect
  holds scene pixels today. P2P: `cargo test -p engine-render --lib` green.
  Review: render.md G11. needs: EG9.
- [x] EG30 playtest: machine eyes — the Look proof has no machine score: the Landed 125e55f9.
  capture hook is unregistered (`crates/endpoint/src/play.rs:902`
  `capture_with`) and `crates/playtest/src/look.rs` has no image metric. Add
  integer SSIM (fixed point, x10000), mean deltaE and a luminance histogram to
  `look.rs`, scored against reference PNGs; register the hook from the window
  host (`crates/game/src/bin/arena.rs` `capture_offscreen`, line 754). Diff:
  good. Tier A. Size M. Owns `crates/playtest/src/look.rs`,
  `crates/endpoint/src/play.rs` hook hunk, `crates/game/src/bin/arena.rs` hook
  hunk. Accept: `cargo test -p engine-playtest --lib look_metric` x2: a frame
  equal to its reference gives SSIM 10000 and deltaE 0; the sand region
  brightened 20% gives SSIM below 9800; the hook runs once per captured frame.
  F2P: no look_metric test today. P2P: `cargo test -p engine-playtest --lib`
  green; golden unchanged. Review: ai.md G9. Landed so far (2026-10-12 check): 32ed363c, 43e1a88e (look_metric tests). Remaining: a window host calls `register_look_judge` at startup and `capture_offscreen` hands each captured PNG to the judge once per frame; a test counts one judge call per frame. needs: EG2 (LJ1 builds on it).
- [ ] EG32 render: GPU skinning — the window skins on the CPU and re-uploads Progress 2026-10-17: GPU skinning is in (a8ee93b: 64-matrix palette blocks at forward binding 7, eight fighter blocks, `upload_skinned_mesh`, `write_skin_block`; tests `skin_window_draw_matches_the_cpu_skinned_mesh`, `skin_window_frame_matches_the_cpu_skinned_frame`, `skin_block_layout_matches_the_shader_palette`). Not met as written: the CPU-vs-GPU bar is 3 levels on 99.5% of pixels (worst up to 40), not 1 level per channel; no `skin_palette` x2 bytes test; no CPU-skin-under-1-ms-for-8-fighters number; `sample_pose_with_rots` allocation not checked.
  each fighter part every frame (`crates/game/src/fighter_look.rs:659-723`),
  `sample_pose_with_rots` (`crates/anim/src/pose.rs:205`) allocates per call,
  and `GpuSkinned` is unused. One 45-joint palette per fighter in a storage
  buffer, a skinned vertex entry in the EG9 forward shader, and last frame's
  palette kept for velocity; the shadow pass reuses them. Tier A. Diff: hard
  (GPU). Size M. Owns `crates/render/src/v1_skin.rs`,
  `crates/render/src/present/gpu_present/submit.rs` palette hunk,
  `crates/game/src/fighter_look.rs` palette hunk, `crates/anim/src/pose.rs`.
  Accept: `cargo test -p engine-render --lib skin_palette` x2 gives equal
  palette bytes; CPU and GPU skin at one tick differ by at most 1 level per
  channel; CPU skin stays under 1 ms for 8 fighters. F2P: no palette buffer
  today. P2P: `cargo test -p engine-render --lib` green; golden unchanged.
  Review: render.md G4 step 2, bodies.md G11 (geaen 11.10.5). needs: EG9, EG10.
- [ ] EG38 render: submission discipline — the opaque pass has no back-face Progress 2026-10-17: back-face cull (`cull_mode: Some(Back)` in submit.rs), bind-group memo (`bind_memo.rs`) and a live pipeline-grouped sort (`rl32_order_is_stable_and_groups_pipeline_adjacent`) are in; `draw_sort_submit_order_is_stable_and_culls_outside_sphere` green on the RTX 3050 (engine-render --features window --lib, local PC run). Left: the sphere-outside-frustum test runs through `eg38_submit_order`, which is tests-only; the live list never calls the sphere test.
  cull, bind groups are rebuilt every frame, the draw list is unsorted, and the
  sphere test in `crates/render/src/draw.rs` has no caller (the arena has 34
  draws). Do the cheap parts only: `cull_mode: Some(Back)`; cached bind groups
  per texture and one dynamic-offset uniform ring; sort by pipeline, then
  texture, then front to back; call the tested sphere test when the list is
  built. No GPU culling (VG rows). Diff: regular. Tier A. Size S. Owns
  `crates/render/src/draw.rs`, `crates/render/src/graph.rs` list hunk,
  `crates/render/src/present/gpu_present/submit.rs` bind hunk. Accept: `cargo
  test -p engine-render --lib draw_sort` x2 gives the same order twice, with
  same-pipeline draws adjacent; a sphere outside the frustum is not submitted
  (counter 0). F2P: no draw sort today. P2P: `cargo test -p engine-render --lib`
  green; golden unchanged. Review: render.md G12 (geaen 10.2.8.2-10.2.8.3).
  needs: EG9.

### Engine review: runtime, bodies, door (owner 2026-10-10, `docs/plan/gameplay.md`)

Same source and routing as the frame section above.

- [x] EG1 arena: incremental live match — `App::simulate_tick`
  (`crates/game/src/lib.rs`) reruns the whole match prefix three times per
  tick (`run_prefix`, `run_events`, `run_dataset`), so a match is O(n^2): 600
  ticks cost about 540,000 steps, a 3-minute bout about 175 million. A
  public `LiveMatch` in `crates/arena/src/live.rs` steps one `Sim` tick per
  call and returns checkpoint, events and dataset rows; App holds it. Diff:
  hard. Claude holds (Opus). Tier A. Size M. Owns
  `crates/arena/src/{lib,live}.rs`, `crates/game/src/lib.rs`. Accept: parity
  test vs the prefix oracle for 600 ticks at 2 and 8 fighters; a step counter
  shows 600 steps for 600 ticks. F2P: the step-count test fails before.
  P2P: golden `d57b5c8ae3096614` x2; `cargo test -p engine-game --lib`
  green. Review: runtime.md G1, bodies.md G1.
- [x] EG3 combat: a hit needs range, facing and team — `combat/src/fight.rs`
  hits every living foe at impact, even across the arena; block works from
  behind; there is no team id. Fixture v2 adds integer distance, facing and
  team to the step and gates `resolve_impacts` with the sweep capsule test,
  behind a new wire version; golden v1 stays frozen. Diff: hard (Claude
  Opus; hash law). Tier A. Size M. Owns `crates/combat/src/fight.rs` plus
  the arena hit gate. Accept: a foe 10 m away is not hit; a block from
  behind fails; a teammate is not hit; v2 digest x2 equal. F2P: the 10 m test
  fails before. P2P: golden `d57b5c8ae3096614` unchanged with a HASH-LAW
  note for v2. Review: bodies.md G2. needs: EG1.
- [x] EG4 game: crash hook and log file in arena.exe — `crates/game/src/crash.rs`
  has no caller and no `set_hook` exists in crates/. Install
  `std::panic::set_hook` in `bin/arena.rs` main: write the crash report (tape
  tail, last checkpoint, adapter) via `crash.rs`, flush a log file, exit with
  the crash code. The hook uses std only. Diff: regular. Tier A. Size S.
  Owns the hook hunk in `bin/arena.rs`, `crash.rs`, one integration test.
  Accept: a test-only forced panic in a smoke run leaves a report file that
  validates. F2P: no hook today. P2P: golden unchanged. Review: runtime.md
  G3, testing.md G2. needs: EG2.
- [x] EG5 physics: timing asserts out of the default suite —
  `crates/physics/src/solver.rs` (`fast_ns <= brute_ns`) and `cloth.rs`
  (`ms < 2000.0`) assert wall time in the default `cargo test`; they flake
  under load. Mark them `#[ignore = "timing: run under proof --perf"]` and
  keep their digest and alloc asserts in a default test. Diff: regular. Tier
  A. Size S. Owns those two test hunks. Accept: no wall-clock assert in a
  non-ignored physics test (grep `elapsed`, `as_nanos`). F2P: grep finds
  them today. P2P: `cargo test -p engine-physics --lib` green. Review:
  testing.md G3.
- [x] EG7 tools: frame gates read pixels — `tools/frame-triage.mjs` passes on Landed 7fdc4ca6.
  a self-reported manifest count (`gatePass = totalFighters > 0`). Count
  fighter pixels with `isFighterPixel` from `tools/fighter-count.mjs` above a
  floor per frame instead. Diff: regular. Tier A. Size S. Owns
  `tools/frame-triage.mjs` plus its test. Accept: a blank frame set fails, a
  real capture set passes. F2P: a blank set with a manifest passes today.
  P2P: `node --test tools/` green. Review: testing.md G1.
- [x] EG15 endpoint: door ids and hashes as strings — `mcp_dto.rs` sends
  `u64` hashes as JSON numbers; JS and LLM clients round them above 2^53 (a
  real run got `forged_approval`); `scene.place` wants UUIDs. Serialize every
  u64 hash, digest and id as a decimal string, accept string or number on
  input; accept a scene name where it is unique. Diff: regular. Tier A. Size
  S. Owns `crates/endpoint/src/mcp_dto.rs` plus the place-resolve hunk.
  Accept: a round-trip test through an f64 JSON parse keeps a hash above 2^53
  exact; the run-door approval replay passes. F2P: that test fails before.
  P2P: `cargo test -p engine-endpoint --lib` green. Review: ai.md G2.
- [x] EG16 endpoint: the door returns numbers — `play.step` gives a digest, a
  text line and a PNG of boxes; `sim.observe` is test-only. Register
  `sim.observe` with per-fighter hp, stamina, phase, position, facing and the
  last N hit, block and kill events; `sim.step` continues a held
  `LiveMatch` instead of re-running from tick 0. Diff: good. Tier A. Size M.
  Owns the endpoint play pack hunks. Accept: a door test steps 60 ticks and
  reads hp falling after a hit event; replay x2 equal. F2P: `sim.observe` is
  not in the tool list. P2P: `cargo test -p engine-endpoint --lib` green.
  Review: ai.md G1. needs: EG1.
- [x] EG21 audio: real sound in the arena — the window plays 3 placeholder Landed c1ddea0 2e12baf. callback_drains_a_thousand_commands_under_contention, feed_contention a_thousand_commands_cross_a_small_ring_in_order_with_nothing_lost, picker_replays_per_seed_and_never_repeats green in cargo test -p engine-audio (lib 245 passed, it 22 passed), run 39056802529; arena feed_tick_audio and load_cue_bank are wired in the window.
  synth sounds, flat, through a `try_lock` that drops commands; 67 CC0 clips
  sit unused in `content/audio`. Load WAVs into cues with 3-4 variants
  (seeded pick); route through `SpatialMixer` with the camera as listener;
  replace `try_lock` with a tick-stamped `std::sync::mpsc` command queue the
  callback drains. Diff: good. Tier A. Size M. Owns `crates/audio/src/device.rs`
  feed hunk plus the arena audio hunk. Accept: a test sends 1000 commands
  under contention and none are lost; a hit cue picks a variant
  deterministically per seed. F2P: the contention test loses commands
  today. P2P: `cargo test -p engine-audio --lib` green; golden unchanged.
  Review: ai.md G5, runtime.md G10, rust-craft.md G3. Landed so far (2026-10-12 check): 3399547b (contention and variant tests). Remaining: the arena builds `AudioEngine::with_feed` over `load_cue_bank(content/audio)`, sends hit, block, footstep cues and the camera listener through FeedTx, drops `Bank::placeholder` and the Mutex adapter; an arena smoke reports sent == applied with dropped_full 0. needs: EG2.
- [x] EG22 geo: the safe UV wrapper checks its inputs — `geo/src/uv.rs` `pub fn
  generate` calls C code and checks only `n`; AI-authored meshes reach it.
  Check `positions.len() % 3`, normal and UV lengths and index bounds; return
  `Err` on a bad mesh. Diff: regular. Tier A. Size S. Owns `crates/geo/src/uv.rs`.
  Accept: tests feed a short normals array and an out-of-range index and get
  `Err`. F2P: those calls reach the FFI today. P2P: `cargo test -p
  engine-geo --lib` green. Review: rust-craft.md G1.
- [x] EG23 arena: a fork panic is not zero metrics — `arena/src/fork.rs`
  turns `Err(_)` into zeroed `ForkMetrics`, which the playtest exploit sweep
  reads as a clean run. Return a typed crashed result and make the sweep
  report it. Diff: regular. Tier A. Size S. Owns `crates/arena/src/fork.rs`
  and the `playtest/src/exploit.rs` read hunk. Accept: a forced panic in a
  fork shows as CRASHED in the sweep. F2P: it shows as zero metrics today.
  P2P: `cargo test -p engine-arena --lib fork` green; golden unchanged.
  Review: rust-craft.md G4.
- [x] EG17 provider: pinned requests and TLS — every request body is `{model, Landed 18df0498. https_stub_roundtrip and pinned_request_body_is_byte_stable pass.
  one user message, max_tokens}` (`crates/provider/src/openai.rs:54`,
  `anthropic.rs:51`, `transport.rs:523`). Send a fixed system prompt,
  temperature 0, a seed where offered, and tool defs; turn `tls` on in endpoint
  and playtest; fill the empty HTTPS test with a local stub. Diff: good. Pinned
  requests landed (25f0d1a4); the TLS half is open. Tier A. Size M. Owns `crates/provider/src/openai.rs`,
  `crates/provider/src/anthropic.rs`, `crates/provider/src/transport.rs`,
  `crates/provider/src/lib.rs`, `crates/endpoint/Cargo.toml`,
  `crates/playtest/Cargo.toml` (tls feature). Accept: `cargo test -p
  engine-provider --lib pinned_request` x2: two builds of one prompt give equal
  bytes with temperature 0 and a system prompt; `cargo test -p engine-provider
  --lib https_stub` connects over TLS. F2P: the pinned_request test fails before
  (no system prompt or temperature). P2P: `cargo test -p engine-provider --lib`
  green; golden unchanged. Review: ai.md G3 (dushkin 21.1).
- [x] EG18 game: animation from sim state, slice 1 (narrowed to the caller) — the Landed 1d01a41.
  bridge module sits caller-less (`pub mod` only); wire it as the production
  owner in the look fold path, sampling stays put. Owns the `anim_bridge.rs`
  caller hunk. Accept: new `fighter_anim_bridge_drives_look_pick` green x2
  (fails before — look never reads the bridge today); `cargo test -p
  engine-game --lib fighter_anim` green x2; golden unchanged; `git diff --stat`
  at most 200 lines. Diff: good. Tier A. Size M. needs: EG1, EG10.
- [ ] EG18b game: animation from sim state, slice 2 — one `StateMachineDef` per
  slot via `Playback`, `rate_q8` carries run scale. Accept: new
  `state_machine_fighter_picks_from_flags` green x2 (fails before); anim plus
  game libs green; golden unchanged; at most 200 lines. needs: EG18.
- [ ] EG18c game: animation from sim state, slice 3 — sample leaving plus current
  through graph at playback weights, react through layers; landed
  midpoint/flinch tests stay green. Accept: new
  `fighter_anim_graph_sample_matches_hand_roll` bit-for-bit x2 (fails before);
  game `--features app` lib green; golden unchanged; at most 200 lines. needs:
  EG18b.
- [x] EG19 combat: readable fight timing, slice 1: tables v2 — startup is 4
  ticks (`crates/combat/src/fight.rs:36`) and parry 4 (`:56`), both below
  human reaction time. `MoveTableSet::v2()` (version 2): every light attack's
  startup 12-18 ticks, parry window 6-8; v1 stays byte-frozen. Diff: good
  (hash law: a new table version, v1 untouched, HASH-LAW note). Tier A. Size
  S. Owns `crates/combat/src/tables.rs` and the `crates/combat/src/fight.rs`
  v2 hunk. Accept: `cargo test -p engine-combat --lib tables_v2` x2: every
  light attack's startup is 12-18 ticks and the parry window 6-8; the v2
  digest is equal x2. F2P: the startup-range test fails before (v1 startup is
  4). P2P: golden `d57b5c8ae3096614` unchanged. Review: bodies.md G4 (geaen
  11.11.2.2). needs: EG3.
- [x] EG19b combat: an `AttackStarted` cue on the press tick under tables v2 Landed fee2dce.
  — the fight emits it when an attack begins (v2 only; the v1 event stream
  stays frozen) and the game receives it as a `FightCue`. Diff: good (hash
  law). Tier A. Size S. Owns the `crates/combat/src/fight.rs` event hunk and
  the `FightCue` mapping hunk. Accept: `cargo test -p engine-combat --lib
  attack_started` x2: a press emits exactly one `AttackStarted` on its tick
  under v2 and none under v1. F2P: no AttackStarted event. P2P: combat lib
  green; golden `d57b5c8ae3096614` unchanged. needs: EG19.
- [ ] EG19c game: the swing starts on the press and the hit lands on its frame
  — `fighter_look` starts the attack clip on `AttackStarted` (today on the
  outcome cue, 12 ticks late, `fighter_look.rs:613-625`) and times the clip so
  its impact frame meets the v2 startup. View only. Diff: regular. Tier A.
  Size S. Owns the fighter_look cue hunk. Accept: `cargo test -p engine-game
  --features app --lib swing_on_press`: under v2 the clip phase at the hit
  tick is within 2 ticks of the clip's impact frame. F2P: the clip starts at
  the outcome cue today. P2P: game lib green; golden unchanged. needs: EG19b.
- [x] EG20 arena: fighters collide — the default leg is a square clamp, so eight
  fighters stack on one point and the kit's pillars do not block
  (`crates/arena/src/lib.rs:1157-1168,1451-1490`). Integer capsule push-apart
  reusing the `controller2` fighter push, a round wall, and box colliders from
  the cooked kit, all in the default leg. Diff: hard (hash). Tier A. Size M.
  Owns `crates/arena/src/lib.rs` leg hunk, new `crates/arena/src/collide.rs`;
  `crates/physics/src/controller2.rs` is reused, not edited. Accept: `cargo test
  -p engine-arena --lib push_apart` x2: eight fighters placed on one point are
  at least two capsule radii apart after 30 ticks; no fighter centre enters a
  pillar box; the digest is equal x2. F2P: eight fighters stay on one point
  today. P2P: golden `d57b5c8ae3096614` unchanged, with a HASH-LAW note for the
  new collision tag. Review: bodies.md G6 (geaen 12.5.3.6; gems3 ch 18). needs:
  EG1.
- [ ] EG24 nn: a learned int8 fighter that runs in the tick — no `.e2nn` is in Progress 2026-10-17: (a)-(c) and the gate are in: `infer_into_allocates_nothing` (nn), `strength_gate_swaps_sides_and_repeats_exactly` plus `binomial_two_sided_p` (distill tournament.rs), `committed_net_passes_the_gate` (distill fps.rs). Left: (d) no `ArenaPolicy` in crates/arena; the Accept names `infer_into_no_alloc` and `student_gate`, neither exists.
  git, `NetPolicy` has no caller, and `infer` allocates. (a) `infer_into` with
  caller scratch; (b) quantize the f32 percept to i8 the same on every PC; (c)
  distill a 2-layer student from `DepthBrain` on `Fork` rollouts; (d) an
  `ArenaPolicy` option, hash-pinned by net digest in the tape header; (e) gate:
  1000 bouts, sides swapped, binomial test. Diff: hard. (a)-(c) landed
  (4d40e6bf); the arena bind (d) and gate (e) are open. Tier A. Size L. Owns `crates/nn/src/lib.rs`,
  `crates/distill/src/student.rs`,
  `crates/distill/src/train.rs`, `crates/distill/src/replay.rs`,
  `crates/ai/src/depth.rs`. Accept: `cargo test -p engine-nn --lib
  infer_into_no_alloc` x2: allocation count 0; `cargo test -p engine-distill
  --lib student_gate` prints wins and a p-value. F2P: no zero-alloc or gate test
  today. P2P: `cargo test -p engine-nn --lib` green; golden unchanged. Review:
  ai.md G6 (gogame 10.3). needs: EG1 (arena bind only).
- [x] EG25 game: a game flow shell in the window — the match length is fixed at Landed c7e6c665. flow.rs; engine-game lib 409 passed, run 39048249632.
  start and `tick_into` rejects any tick past it (`crates/game/src/lib.rs:701`),
  and `windowed.rs` imports only the HUD, so there is no title, pause, result or
  rematch. One `Flow` (Title, Match, Paused, Result) in new
  `crates/game/src/flow.rs` drives the existing menu libraries; pause stops the
  accumulator, not the tick count; a result leads to the next match. Minimal
  shell only: no settings screen (the owner parked UI rows). Diff: good. Tier A.
  Size M. Owns `crates/game/src/flow.rs` (new), `crates/game/src/windowed.rs`
  input hunk, `crates/game/src/bin/arena.rs` flow hunk. Accept: `cargo test -p
  engine-game --lib flow_cycle` x2: Title, Match, Result, Match again; 60 paused
  frames add 0 ticks; a bout with a pause in it has the same final hash as one
  without. F2P: no Flow type today. P2P: golden `d57b5c8ae3096614` unchanged;
  `cargo test -p engine-game --lib` green. Review: runtime.md G2 (geaen 14.1.3).
  needs: EG2.
- [ ] EG26 game: a dev console over the door verbs — none exists in the window. Progress 2026-10-17: only the render half exists (`crates/render/src/debug_draw.rs`); no `crates/game/src/console.rs`, no `DevConsole`, no `console_step` test.
  A `DevConsole` line runs read-only door verbs first (hash, tick, stats), then
  pause, step, time-scale and set-knob, drawn with the bitmap HUD text. Pause,
  step and slow-mo scale the accumulator at the frame boundary and never the dt
  inside a tick, so hashes match; a line and box debug-draw list in render.
  Diff: regular. Tier A. Size M. Owns new `crates/game/src/console.rs`,
  `crates/game/src/windowed.rs` key hunk, new `crates/render/src/debug_draw.rs`.
  Accept: `cargo test -p engine-game --lib console_step` x2: `step` advances one
  tick; `slow 0.5` makes two frames give one tick; `hash` at tick 600 equals
  `d57b5c8ae3096614`. F2P: no console verb today. P2P: golden unchanged; `cargo
  test -p engine-render --lib` green. Review: runtime.md G6, testing.md G12
  (geaen 9.2-9.6). needs: EG25. Progress 2026-10-18: not built (no console.rs). The time half now exists: crates/game/src/virt.rs VirtualClock (GP45, 033aaca3) is the thing a `slow` verb sets and pause/step already sit in bin/arena.rs (--pause-at-tick, --step). Left: (1) the window cannot print a console line: render has OverlayRing::push_text (crates/render/src/overlay.rs:131) but Presenter forwards only push_hp_bar and clear_overlay (present/gpu_present/presenter.rs, another session's file); (2) hash and stats are door verbs and engine-game cannot call engine-endpoint (the endpoint depends on game), so they need a small trait in game that the endpoint implements.
- [x] EG27 game: a gamepad backend — `crates/game/src/pad.rs` Landed 03a45f91. pad.rs poll_gilrs_once; same run.
  (`PadFrame::snapshot`, line 59) is data only: no device crate is in any
  Cargo.toml and nothing polls a pad, so a fighter on Windows has no pad. Add
  `gilrs` (a new dependency, so Claude adds it; check `deny.toml`) and poll it
  once per frame into `PadFrame::snapshot`. Diff: regular. Tier A. Size S. Owns
  `crates/game/src/pad.rs`, `crates/game/Cargo.toml`, the gilrs lines of
  `Cargo.lock`, `crates/game/src/windowed.rs` poll call. Accept: `cargo test -p
  engine-game --lib pad_gilrs` x2 with a recorded event list gives equal
  PadFrame bytes; the `deny.toml` licence check passes. F2P: no gilrs in the
  tree today. P2P: `cargo test -p engine-game --lib` green; golden unchanged.
  Review: runtime.md G7 (geaen 8.2, 8.5.5). needs: EG2.
- [x] EG28 ecs: a persistent job pool with one real use —
  `crates/ecs/src/jobs.rs:50,80`, `crates/ecs/src/parallel.rs:81` and
  `crates/physics/src/islands.rs:172-186` (`step_islands`) spawn scoped threads
  on every call. One `JobPool` in ecs: N workers started once, `scope`, a wait
  counter, results merged in index order, panics re-raised with `resume_unwind`.
  Its one caller is `step_islands`; the sim stays single-threaded. Diff: hard.
  Tier A. Size M. Owns `crates/ecs/src/jobs.rs`, `crates/ecs/src/parallel.rs`,
  `crates/ecs/src/lib.rs`, the call hunk in `crates/physics/src/islands.rs`.
  Accept: `cargo test -p engine-ecs --lib job_pool_threads` x2: a fixed workload
  gives identical bytes at 1, 2, 4 and 8 threads; a panic in a job re-raises on
  the caller; zero threads are spawned per call after warm-up (counter). F2P:
  the zero-spawn counter test fails before (a spawn per call). P2P: `cargo test
  -p engine-ecs --lib` green; golden unchanged. Review: runtime.md G9,
  rust-craft.md G5 (gearus 8.6.3).
- [x] EG29 game: hot reload in the window — `crates/assets/src/watcher.rs`, Landed 1d01a41. reload::tests reload_keeps_last_good (bad cook keeps the old generation and logs its reason, good edit applies at the frame boundary stamped with the tick), _after_a_bad_file_on_disk, _cooks_every_shipped_watched_file, _rows_stage_practice_only_when_accepted, _refuses_each_malformed_kind green in engine-game lib, run 39058609935; arena.rs polls the Watcher once per presented frame (bin/arena.rs:1418, settle_paths).
  `recook.rs` and `crates/game/src/practice.rs` are finished, but `bin/arena.rs`
  never calls them, so content edits need a restart. Poll `Watcher` once per
  presented frame for content/ and the WGSL files; `Recook` settles and keeps
  the last good asset when a cook fails; apply `ReloadNotice` at the frame
  boundary. Combat tuning goes through the practice swap at a tick boundary and
  is written to the tape. Diff: good (hard if it touches sim state). Tier A.
  Size M. Owns `crates/game/src/bin/arena.rs` poll hunk,
  `crates/game/src/windowed.rs` frame hunk, `crates/game/src/practice.rs` swap
  hunk. Accept: `cargo test -p engine-game --lib reload_keeps_last_good` x2: a
  bad cook leaves the old asset live and logs its reason; a good edit applies on
  a frame boundary, with the tick index unchanged mid-tick. F2P: no Watcher
  caller in the window (grep count 0 today). P2P: golden unchanged; `cargo test
  -p engine-assets --lib` green. Review: runtime.md G8 (gems1 2.3). needs: EG2.
- [x] EG31 save: durable writes fsync, and checked casts in nav —
  `crates/editor/src/durable.rs:186-189`, `crates/save/src/lib.rs:150-161` and
  `crates/save/src/fallback.rs:136-146` rename a tmp file with no `sync_all`; a
  power cut can lose the last save. Write, `sync_all`, rename, then sync the dir
  on Unix. In nav, `TileId` uses `u32::try_from`
  (`crates/nav/src/tiles.rs:555,726`) and rounding checks `is_finite`
  (`crates/nav/src/mesh.rs:239,674,861`). Diff: regular. Claude holds (Haiku).
  Tier A. Size M. Owns `crates/save/src/lib.rs`, `crates/save/src/fallback.rs`,
  `crates/editor/src/durable.rs`, `crates/nav/src/tiles.rs`,
  `crates/nav/src/mesh.rs`. Accept: `cargo test -p engine-save --lib
  fsync_order` x2: a fake sink records write, sync, rename; `cargo test -p
  engine-nav --lib try_from_ids` returns Err for id 2^32 and for NaN. F2P: the
  fsync order test fails before. P2P: `cargo test -p engine-save --lib` and `-p
  engine-nav --lib` green; golden unchanged. Review: rust-craft.md G7, G9.
- [x] EG33 ai: melee brains — `Percept` (`crates/ai/src/gladiator.rs:35`)
  carries one foe and a bool attack, with no own hp, stamina, attack phase or
  other fighters, so eight fighters all rush the nearest one. Extend `Percept`
  with own hp and stamina, the foe's attack phase and frames to active, and the
  nearest 3 foes, quantized at the edge; a max-two-attackers token per target
  from `SquadN` (`crates/ai/src/depth.rs:681`). GA8 stays the EQS bind. Diff:
  good. Tier A. Size M. Owns `crates/ai/src/gladiator.rs`,
  `crates/ai/src/depth.rs` squad hunk, `crates/arena/src/lib.rs` brain build
  line (`DepthBrain`, about line 810). Accept: `cargo test -p engine-ai --lib
  melee_percept` x2: a percept with 4 foes lists the nearest 3 in distance
  order; a third attacker on one target waits (token count never above 2). F2P:
  the 3-foe percept test fails before (one foe). P2P: `cargo test -p engine-ai
  --lib` green; golden unchanged. Review: ai.md G8, bodies.md G9 (gems3 ch 18).
  needs: EG1.
- [x] EG34 physics: ragdoll split decided and wired — stagger, balance and limb
  state never reach the tick: `crates/physics/src/balance.rs` and `ragdoll.rs`
  have no caller in the step, and PX36 is ticked but unwired (BLOCKERS). Record
  the split: authoritative stagger, knockdown and limb state is integer
  (`BalanceState`, `StaggerMemory`, `LimbPool`, cut) and hashed; the Rapier
  ragdoll is client-local, blended by powered drive, never hashed. Wire
  `BalanceState` into the step first. Diff: hard (hash law). Tier A. Size L.
  Owns `crates/physics/src/balance.rs`, `crates/physics/src/ragdoll.rs`,
  `crates/arena/src/lib.rs` step hunk, `docs/HASH-LAW.md` decision note. Accept:
  `cargo test -p engine-physics --lib balance_hash` x2: a knockdown at tick 100
  gives the same balance hash twice; a run with the Rapier ragdoll on and off
  gives equal digests. F2P: no BalanceState in the step today. P2P: golden
  `d57b5c8ae3096614` unchanged, with a HASH-LAW note for the balance wire
  version. Review: bodies.md G7 (geaen 12.5.3.8). needs: EG1.
- [ ] EG35 endpoint: a render door — an AI cannot see or steer the renderer:
  `crates/endpoint/src/packs/view.rs` holds log verbs only, and endpoint has no
  engine-render dependency. Add `view.capture` (an offscreen render of the
  current tick: path, SHA, per-pass ms), `render.material.set` (the look half of
  `MaterialRecord`, `crates/material/src/record.rs`, onto `PbrMaterial` by draw
  tag) and `render.pass.toggle`. All undoable; refused unless enabled. Diff:
  good. Tier A. Size M. Owns `crates/endpoint/src/packs/view.rs`, new
  `crates/endpoint/src/packs/render.rs`, `crates/endpoint/src/packs/mod.rs`
  line, `crates/endpoint/Cargo.toml` (engine-render dep). Accept: `cargo test -p
  engine-endpoint --lib packs::render` x2: two `view.capture` calls on one tick
  give equal SHA; `render.pass.toggle` is refused with the door off; undo
  restores the pass state; the three verbs appear in api.describe. F2P: no
  render verb today. P2P: `cargo test -p engine-endpoint --lib` green; golden
  unchanged. Review: render.md G10. needs: EG9.
- [x] EG36 endpoint: process-wide mutable statics become per-session —
  `STEP_HOOK` (`crates/endpoint/src/play/play_session.rs:113`), `REQUERY`
  (`crates/endpoint/src/play/requery.rs:167`, a cache shared by all sessions)
  and `TMP_CLIPS`/`TMP_SKIPPED` (`crates/destruction/src/fracture.rs:228-230`,
  read by `csg.rs:972-973`). Delete the TMP statics and return the counts from
  `fracture_at`; pass the hook and the cache through `PlaySession` only. Diff:
  good. Tier A. Size M. Owns `crates/endpoint/src/play/play_session.rs`,
  `crates/endpoint/src/play/requery.rs`, `crates/destruction/src/fracture.rs`,
  `crates/destruction/src/csg.rs` test hunk. Accept: `cargo test -p
  engine-destruction --lib fracture_counts` x2 with tests run in parallel give
  equal counts; a grep for top-level `static` in those four files finds nothing.
  F2P: grep finds `STEP_HOOK`, `REQUERY` and `TMP_CLIPS` today. P2P: `cargo test
  -p engine-endpoint --lib play` green; golden unchanged. Review: rust-craft.md
  G2 (idiomatic 10.7).
- [x] EG37 render: spawn failures return Err — `crates/render/src/sky.rs:730`
  calls `.expect("sky look worker spawns")` in the render path, and
  `crates/provider/src/stream.rs:555` calls `thread::spawn`, which panics,
  inside a function that returns Result. Use `thread::Builder::spawn` with
  `map_err` into a typed error in both, behind a test seam that forces the spawn
  to fail. Diff: regular. Tier A. Size S. Owns `crates/render/src/sky.rs` spawn
  hunk, `crates/provider/src/stream.rs` spawn hunk. Accept: `cargo test -p
  engine-provider --lib spawn_failure_is_err` and `cargo test -p engine-render
  --lib sky_spawn_err` each pass: the forced failure returns Err and does not
  panic. F2P: both tests panic before. P2P: `cargo test -p engine-render --lib`
  and `-p engine-provider --lib` green; golden unchanged. Review: rust-craft.md
  G11 (trpl ch 9). needs: EG2.
- [ ] EG39 game: presentation is rollback-safe — `SlotAnim` accumulates from
  cues (`swings`, `act_start`, `crates/game/src/fighter_look.rs:134-151`), so a
  resim re-emits cues and restarts clips, and a hop after a correction reads
  worse than a small error. Derive slot anim from sim state at tick N plus the
  last K cue ticks; add a decaying visual error offset on correction. Render
  only, never hashed. Feed the LM3 net-pose indices. Diff: good. Tier A. Size M.
  Owns `crates/game/src/fighter_look.rs` slot hunk,
  `crates/anim/src/net_pose.rs`. Accept: `cargo test -p engine-game --lib
  resim_no_restart` x2: after a 4-tick resim the clip phase equals the no-resim
  run; the visual offset is above 0 on the correction tick and under 1 cm after
  12 ticks. F2P: the resim test restarts the clip today. P2P: golden unchanged;
  `cargo test -p engine-game --lib` green. Review: bodies.md G10 (gems2 ch 18).
  needs: EG18.
- [ ] EG40 game: weapons and shields on sockets — the sword and shield are baked
  into the body mesh (`crates/game/src/fighter_bind.rs` hero graft), so a sword
  cannot drop after a hand cut or be swapped; nothing named `attach` exists in
  anim or game. Add `Attachment{joint, offset}` to `FighterLook`
  (`crates/game/src/fighter_look.rs:207`), separate sword and shield meshes from
  the cook (`crates/assets/src/body_import.rs`), and a body that turns
  physics-driven on drop. Diff: regular. Tier A. Size M. Owns
  `crates/game/src/fighter_look.rs` attachment hunk,
  `crates/game/src/fighter_bind.rs`, `crates/assets/src/body_import.rs` mesh
  hunk. Accept: `cargo test -p engine-game --lib attach_socket` x2: the sword
  follows the hand joint within 1 mm for 60 frames; a dropped sword leaves its
  socket and gets one physics body. F2P: a grep for `attach` in anim and game
  finds nothing today. P2P: golden unchanged; `cargo test -p engine-game --lib`
  green. Review: bodies.md G8 (geaen 11.11.5.1). needs: EG18.
- [x] EG41 perf: packaged smoke, soak and a frame-time gate — the packaged smoke Landed 1d01a41. node --test tools/proof.test.mjs 21 pass (frameCsvVerdict steady rows pass, slow tail fails on p95, same text x2 equal, real timed-smoke header parses), engine-perf lib 122 passed run 39057450320; arena writes target/perf/frame-times.csv (`write_frame_times_csv`) and `node tools/proof.mjs --perf` prints "frame times: frames=600 p50=9.219 ms p95=19.310 ms (bar 50 ms)"; `fps_est` is renamed (perf CSV header).
  is a marker only, the hour soak in `crates/proof/src/online_soak.rs` is
  `#[ignore]` and unwired, and no CI gate reads frame time. Use the existing
  `arena --frames N --exit --timed-smoke` to write per-frame wall times and
  input-to-present pairs to a CSV (`crates/perf/src/latency.rs`, `WallMeter`);
  the proof perf mode in `tools/proof.mjs` reads it, prints p50 and p95, and
  fails above the quality.md P2 bar (p95 50 ms). Rename `fps_est` to
  `cpu_workload_rate`. Diff: good. Tier A. Size M. Owns `tools/proof.mjs`,
  `tools/proof.test.mjs`, `crates/game/src/bin/arena.rs` timed-smoke CSV hunk,
  `crates/perf/src/latency.rs`, `crates/perf/src/main.rs`. Accept: `node --test
  tools/proof.test.mjs` x2 on a fixture CSV gives the same p50 and p95 twice; a
  fixture with a 60 ms p95 prints FAIL. F2P: no frame CSV reader today. P2P:
  `node --test tools/` green; `cargo test -p engine-perf --lib` green; golden
  unchanged. Review: testing.md G5, G6. Landed so far (2026-10-12 check): a63d3bde, 3f1f635a (CSV reader, 60 ms FAIL). Remaining: `arena --frames N --exit --timed-smoke` writes `target/perf/frame-times.csv` so `proof --perf` reads a real run and prints p50 and p95. needs: EG2.
- [ ] EG42 anim: baked per-move hit tracks — the sim has root and yaw only, so
  the blade path and bone capsules that `crates/combat/src/cut.rs:558` needs
  have no authoritative source; animation is float presentation. Bake like
  `crates/anim/src/loco_bake.rs` turns three attack clips into integer-mm
  `HitTrack` rows (blade p0 and p1, bone capsules per tick), and combat reads
  `(move, tick)`. Diff: hard (hash law). Tier A. Size L. Owns new
  `crates/anim/src/hit_track.rs`, `crates/anim/src/loco_bake.rs` sibling hunk,
  `crates/combat/src/fight.rs` read hunk. Accept: `cargo test -p engine-anim
  --lib hit_track_bake` x2: baked rows for one clip are equal twice and hold
  integer mm only; a sweep with a foe on the blade path registers one hit at the
  baked tick. F2P: no HitTrack type today. P2P: golden `d57b5c8ae3096614`
  unchanged, with a HASH-LAW note for the HitTrack wire version. Review:
  bodies.md G5 (geaen 12.3.1; see HR6, CT33). needs: EG3.
- [x] EG43 ai: v2 duels have winners — with rules v2 (EG3) identical brains
  close in lockstep: 399 of 400 seeds end in a same-tick double knockout and 0
  are decisive. Give each slot a seeded personality from the match seed (an
  integer reaction offset in ticks, preferred range, aggression) inside
  `engine-ai`, carried in the brain, never read from wall time. Diff: good.
  Tier A. Size M. Owns `crates/ai/src/depth.rs` personality hunk and the
  `crates/arena/src/lib.rs` brain build line. Accept: the ignored 400-seed v2
  split test (EG3) run x2 gives equal numbers with at least 60% decisive bouts,
  double knockouts under 20%, and slot 0's share of wins between 40% and 60%.
  F2P: today 0 decisive. P2P: `cargo test -p engine-arena --lib` green; golden
  `d57b5c8ae3096614` unchanged. Review: bodies.md G2 (EG3 receipt). needs: EG33.
- [x] EG44 net: rules version on the wire, window plays v2 — the window game Progress 2026-10-17: the rules version rides the lobby identity (`BuildId::of_spec` folds `MatchSpec::identity`; net `spec_lobby_refuses_other_rules_or_another_world_naming_both_digests`, 36083b4) and the window plays v2 (`into_duel` in windowed.rs and bin/arena.rs). Left: a v2 `App::new` equals `new_for_slot` test, window run x2 equal v2 hashes, the HASH-LAW note; `App::new` default stays v1 by design.
  stays on rules v1 because the LAN host replays v1 through `run_prefix`, so a
  v2 client would desync. Carry the rules version in the net session hello and
  in `run_prefix`; refuse a peer on another version with a typed error; then
  flip `App`'s `rules` field to v2 (one line). Diff: hard (hash law, net). Tier
  A. Size M. Owns the `crates/net` hello and replay hunks and the `App` rules
  line in `crates/game/src/lib.rs`. Accept: a v1 client joining a v2 host gets
  the typed refusal; the netgame `App::new` equals `new_for_slot` test passes
  on v2; the window arena run x2 gives equal v2 hashes. F2P: no rules version on
  the wire today. P2P: `cargo test -p engine-net --lib` green; golden
  unchanged with a HASH-LAW note. Review: bodies.md G2. needs: EG43. Progress 2026-10-18: the constructor half lands 9e624736 - `GameConfig` cannot select v2 (both constructors pin `TABLES_VERSION_V1`), so the test `app_new_equals_new_for_slot_on_rules_v2` states it: App::new stays v1 by design, both constructors agree before the duel, and after the window's own `into_duel` step both are `FIGHT_RULES_V2` with equal spec and tick 0..8 checkpoints/hashes; HASH-LAW note added; engine-game --lib 457 passed (gh runs 39127556745, 39127861274). Left: the window arena run x2 with equal v2 hashes (real window on this PC). Landed 9e624736. window v2 run x2 equal hash ef25059b432c9193 (commands: target\release\arena.exe --staged-fight --exit / target\release\arena.exe --staged-fight --exit).
- [x] EG45 game: the window shows balance verdicts — EG34 steps an integer Landed 1d01a41.
  `BalanceState` per slot under rules v2, but it is a private `Sim` field
  (`crates/arena/src/lib.rs`, `balance: Vec<BalanceState>`), so the window never
  shows a stumble or a fall. Add a read-only `Sim::balance(slot) ->
  Option<&BalanceState>` with its caller in `crates/game/src/fighter_look.rs`:
  a stumbling slot leans its root, a fallen slot lies flat, a catching slot
  kneels. The look only reads; the Rapier ragdoll blend is a later row. Diff:
  good. Tier A. Size S. Owns the `Sim::balance` accessor hunk in
  `crates/arena/src/lib.rs` and the `crates/game/src/fighter_look.rs` balance
  hunk. Accept: `cargo test -p engine-game --lib balance_look` x2: a slot
  floored in a scripted v2 bout draws its root below 0.4 m within 10 frames; a
  standing slot draws the same pose as before; the sim digest is equal with
  the balance look on and off. F2P: no `Sim::balance` read today. P2P: `cargo
  test -p engine-game --lib` green; golden `d57b5c8ae3096614` unchanged.
  Review: bodies.md G7 (EG34 receipt). needs: EG34.
- [x] EG46 physics: stagger memory in the v2 hash — EG34 hashes Landed 832575a3.
  `BalanceState` (tag `0xB1`), but `StaggerMemory`
  (`crates/physics/src/stagger_memory.rs`) has no caller in the step, so
  repeated stagger never reaches the authority. Step it in the v2 leg beside
  the balance wire and fold its hash under a new tag. Diff: good (hash law: v2
  only, one new tag, HASH-LAW note). Tier A. Size S-M. Owns the
  `crates/arena/src/lib.rs` step hunk and the HASH-LAW note. Accept: `cargo
  test -p engine-arena --lib stagger_hash` x2: three staggers inside the
  memory window floor the victim; the digest is equal x2 and differs from a
  two-stagger run. F2P: no StaggerMemory in the step. P2P: golden
  `d57b5c8ae3096614` unchanged, v1 hashes frozen; the HS2 golden-matrix tool
  (`tools/golden-matrix.mjs`) names no bad tick. Review: bodies.md G7. Landed so far (2026-10-12 check): 350b3b25 (step and 0xB2 fold). Remaining: `stagger_hash` tests in engine-arena (three hits in the window floor the victim, digest equal x2, differs from a two-hit run) and the 0xB2 line in docs/HASH-LAW.md. needs:
  EG34.
- [x] EG46b physics: limbs in the v2 hash — step `LimbPool` Landed 9748388. limb_hash_cut_repeats_x2 and limb_hash_differs_from_uncut_run green, run 39057007421; LimbPool steps in the v2 leg (lib.rs:1612) and folds under tag 0xB3. Note: no live caller of `spawn_limbs` yet, so a real fight cut still does not reach the hash (HASH-LAW says so).
  (`crates/physics/src/limb.rs`) in the v2 leg after stagger memory and fold
  its hash under its own tag, so a cut reaches the authority. Diff: good
  (hash law). Tier A. Size S-M. Owns the arena step hunk after EG46's.
  Accept: `cargo test -p engine-arena --lib limb_hash` x2: a scripted cut at
  tick 100 gives the same digest twice and differs from the uncut run. F2P:
  no LimbPool in the step. P2P: as EG46. needs: EG46.
- [ ] EG47 render: every GPU test skips through one helper — EG6 landed
  `engine_render::test_gpu::require_or_skip` (`crates/render/src/test_gpu.rs`)
  and moved `present/tests_gpu.rs` onto it, but 24 files still return early on
  `no wgpu adapter` with no SKIPPED line, so `E2040_REQUIRE_GPU=1` cannot catch
  them: render `ibl.rs` 6, `gtao.rs` 6, `graph.rs` 6, `shadow_cluster.rs` 5,
  `executor.rs` 5, `particles_gpu.rs` 3, `decals.rs` 3, `v1_skin.rs` 2,
  `v1_gpu.rs` 2, `sky.rs` 2, `post.rs` 2, `material_layer.rs` 2, `fog.rs` 2,
  `accel.rs` 2, `twin_shadow.rs` 1, `present/tests_linear.rs` 1,
  `present/surface.rs` 1, `baked_gi.rs` 1, `backend.rs` 1, `aa_scale.rs` 1;
  `crates/editor/src/app.rs` 4; game `windowed.rs` 2, `fighter_look.rs` 1,
  `bin/arena_capture.rs` 1. Copy the `init_or_skip` pattern at the top of
  `crates/render/src/present/tests_gpu.rs`: map the no-adapter error to
  `Err(NoAdapter("no wgpu adapter on this machine".into()))`, panic on any
  other init error, then `let Some(x) = require_or_skip("<test name>", probe)
  else { return; };` (test code only; a file on a `claude-*` claim waits).
  Diff: regular. Tier B. Size M. Owns the test hunks of the files above.
  Accept: every `rg -n "no wgpu adapter" crates` hit outside `test_gpu.rs` is a
  `NoAdapter(..)` reason handed to `require_or_skip`, with no `eprintln!` plus
  `return` left; `cargo test -p engine-render --lib -- --test-threads=1` gives
  the same pass count as before plus SKIPPED lines on a PC with no adapter.
  F2P: 24 files print and return on their own today. P2P: `cargo test -p engine-render --lib` and `-p engine-game --lib`
  green. Review: testing.md G4 (EG6 receipt). needs: EG6.

### Forge and old-plugin adoption (owner 2026-10-11, `docs/plan/forge-adoption.md`)

Boarded 2026-10-11 from the audit of Forge (C:\forge) and the old Flax plugin:
the top 20 items of `docs/plan/forge-adoption.md` section B. Every row has F2P and
P2P; none moves the golden. Rows that say BLOCKED wait for an owner decision
(section I of the plan). The next 30 items and the rest of the catalog stay in
the plan file until a row is needed. Endpoint rows serialize (one builder per
crate): AD3, AD9, AD20 step 1, AD7, AD2, AD12, AD13, AD8, then the second
pass: PX40, AD26, AD21, AD28, AX23, AD29, AD27, PF24, SP66, EA6.

- [x] AD1 playtest: stale-capture guard and typed ABSTAIN — the frozen check
  sees identical runs only. Add a pose-aware check: a camera pose digest that
  moved while the frame digest stayed the same is `FrameFault::Stale`; a
  timed-out or void judge result is `Abstain(reason)`, never Pass. Source:
  E06#4, E09#2, C3 G1/G2. Diff: good. Tier A. Size S. Owns
  `crates/playtest/src/frame_check.rs` hunks and
  `crates/playtest/src/look/judge.rs` hunk; the pose field in
  `crates/game/src/capture.rs` is a follow-up row if the manifest lacks it
  (UNVERIFIED). Accept: `cargo test -p engine-playtest --lib frame_stale` x2:
  two poses 26 m apart with equal digests give Stale; the same poses with
  different digests pass; a timeout and a void frame give Abstain; a negative
  control (flip one byte of the digest) flips the result. F2P:
  `check_digest_window` takes digests only and no pose, and `grep -ri abstain
  crates` finds nothing. P2P: `cargo test -p engine-playtest --lib
  frame_check` green; golden unchanged. Plan: `docs/plan/forge-adoption.md`
  AD-1. needs: none. Landed 33263e14. frame_check check_pose_sequence: consecutive captures with an equal digest while the camera moved over 100 cm are FrameFault::Stale; a timed-out or void capture is SequenceVerdict::Abstain(reason), never Pass (a fault in the arrived frames wins). Integer cm poses. Tests frame_stale_poses_26_m_apart_with_equal_digests_fault (2600 cm), frame_stale_same_poses_with_different_digests_pass, frame_stale_negative_control_one_digest_byte_flips_the_result, frame_stale_timeout_and_void_abstain_never_pass, frame_stale_a_still_camera_may_repeat_a_frame; frame_check suite green. Left: the look/judge.rs hunk, the judge module was removed in f1a4f9d7 (restore is LF15/PQ21 territory), and the pose field in game capture.rs manifest. gh run 39105225319 (engine-playtest --lib highlights brief_eval trajectory scorecard frame_stale: 36 passed, 1 ignored) and full engine-playtest --lib 373 passed, run 39103617132.
- [ ] AD2 editor: landed-digest read-back on every door write — `Applied`
  carries before, staged and after digests; the service refuses
  `landed_mismatch` when the landed digest differs from the staged one; a save
  then reopen then diff test covers every reference (a second save must not
  null one). Source: E01#1, E06#15, E09#5, C3 G3/G8. Diff: good. Tier A. Size
  S-M. Owns `crates/editor/src/commands.rs` hunks first (shared type), then
  `crates/endpoint/src/receipts.rs` and `crates/endpoint/src/mcp_dto.rs`
  (TxApplyOutput fields). Accept: `cargo test -p engine-editor --lib
  landed_digest` x2: an adapter that acks but changes nothing gives
  `no_effect`; `save_reopen_diff` fails when one ref is nulled; `cargo test -p
  engine-endpoint --lib receipt_digests` returns all three digests; same batch
  twice gives equal receipts. F2P: `Applied` has no digest fields today. P2P:
  `cargo test -p engine-editor --lib` and `-p engine-endpoint --lib` green;
  golden unchanged. Plan: `docs/plan/forge-adoption.md` AD-2. needs: none.
- [x] AD3 endpoint: honest spatial door — `scene.spatial` lists `checks_run`, Landed eeac21f. scene_tools::tests::spatial_door_fire green (every SPATIAL_CHECKS entry fires or is named in `skipped` with a reason, ran plus skipped covers the catalog once, bare nodes report size_source default above 0), cargo test -p engine-endpoint --features mcp --lib -- spatial_door_fire physics_read, run 39057713346.
  `skipped{check,reason}` and `size_source{collider,default}` counts;
  non-finite and parentless nodes become findings instead of silent drops; one
  door-level fire test per entry of `SPATIAL_CHECKS`. Source: E03#3. Diff:
  regular. Tier A. Size S. Owns `crates/endpoint/src/scene_tools.rs` (output
  struct and snapshot hunks) plus its tests. Accept: `cargo test -p
  engine-endpoint --lib spatial_door_fire` x2: every check either yields a
  finding on its door fixture or appears in `skipped` with a reason; a scene
  of bare nodes reports `size_source` default count above 0; output goldens
  updated with a named receipt. F2P: `SceneSpatialOutput` has no `checks_run`
  and 7 of 14 offline checks cannot fire through the door. P2P: `cargo test -p
  engine-endpoint --lib` green; golden unchanged. Completes with AD7. Plan:
  `docs/plan/forge-adoption.md` AD-3. needs: none.
- [x] AD4 tools: no hollow green — `Verdict` gets `checks_run` and passes only Progress 2026-10-17: `Verdict.checks_run` and the NoChecks failure are in (022cf59, playtest `zero_checks_is_never_a_pass`, `verify_counts_three_checks_and_prints_them`) and `proof.mjs` turns a zero-test cargo leg into RED ("zero tests ran"). Left: the `target/proof/<gate>-latest.json` green record with 25 h drift FAIL, the gpu_live `not_wired` print, and the node tests for them; no test is named `verdict_checks_run`.
  above 0; `proof.mjs` turns "running 0 tests" and a zero-test filter into
  RED; a green record `target/proof/<gate>-latest.json` (sha, time) is written
  on GREEN only and drift FAILs a stale one; gpu_live `live` comes from a
  device receipt else prints `not_wired`. Source: E06#5/#12/#13, E12#7. Diff:
  regular. Tier A. Size S. Owns `tools/proof.mjs`, `tools/proof.test.mjs`,
  `crates/playtest/src/lib.rs` Verdict hunk. Accept: `node --test
  tools/proof.test.mjs` x2: a filter that matches nothing is RED, a 25 h old
  record FAILs, a red run leaves the green record alone; `cargo test -p
  engine-playtest --lib verdict_checks_run` : empty `verify()` gives
  `pass=false`. F2P: `pass = failures.is_empty()` and no zero-test rule exist
  today. P2P: `node --test tools/` green; `cargo test -p engine-playtest
  --lib` green; golden unchanged. Plan: `docs/plan/forge-adoption.md` AD-4.
  needs: none. Landed 870231e1. node --test tools/proof.test.mjs x2 31 pass (ranAtLeastOneTest zero-test RED, driftVerdict 25 h FAIL, recordGreen red run leaves the record, gpuLiveReceipt not_wired); cargo test -p engine-playtest --lib verdict_checks_run 4 passed (run 39102360668). Full playtest --lib currently fails to compile on another session's eval.rs door.submit borrow (run 39102750013), not on the four renamed tests.
- [x] AD5 tools: unwired-code gate — `packet-gate` fails a new `pub` fn,
  struct or enum in the diff that has no non-test caller outside its own file
  unless the packet names a consumer row (UNLOCKS); `--orphans` prints a
  report-only list of existing orphans (no deletes). Source: E09#9, E12#10,
  E10#4, E11#14, E05 anti-6, C2 U6. Diff: regular. Tier A. Size S-M. Owns
  `tools/packet-gate.mjs` and `tools/packet-gate.test.mjs`. Accept: `node
  --test tools/packet-gate.test.mjs` x2: an uncalled pub fn FAILs; the same fn
  called from another module PASSes; an UNLOCKS row exempts it; fixtures
  shaped like px36, rt19, fix5 and fix3c FAIL. F2P: no caller rule exists in
  packet-gate (BLOCKERS holds four "unwired" fingerprints x2-3). P2P: `node
  --test tools/` green. Plan: `docs/plan/forge-adoption.md` AD-5. needs: none. Landed 84fe99f. The packet-gate tool and test were cut with the packet loop, so the rule lives in tools/wiring-scan.mjs --diff[=REV] --unlocks=ROW --orphans. node --test tools/wiring-scan.test.mjs 17 pass: uncalled new pub fn FAILs, called from another file PASSes, open UNLOCKS row exempts (closed row refused), fixtures shaped like px36, rt19, fix5 and fix3c FAIL.
- [ ] AD6 assets: one licence table with duties, weights out of git, licence
  filter in search — `provenance.rs` holds the one table (SPDX, may_ship,
  duties); `check_release` refuses CC-BY without credit text and marks
  past-due records Stale; share-alike refused until its duty exists; a
  repo-wide drift test reads the `.rs`, `.py` and `.mjs` lists;
  `workspace-check` refuses tracked `.safetensors/.onnx/.pt/.ckpt/.gguf`
  except digest-pinned fixtures; `editor/src/browser.rs` `SearchQuery` gets a
  licence filter. Source: E07#1/#3/#8, E04#3, E13#14, C3 G31/G11. Diff: good.
  Tier A. Size S. Owns `crates/assets/src/provenance.rs`, hunks in
  `gen_intake.rs` and `clip_intake.rs`, `tools/workspace-check.mjs`,
  `crates/editor/src/browser.rs` hunk (second step). Accept: `cargo test -p
  engine-assets --lib licence_table` x2: CC-BY without credit fails release,
  SA refused, drift test fails on any list difference; `node --test
  tools/workspace-check.test.mjs`: a staged `model.safetensors` fails and
  `fixtures/nets/depth_student.e2nn` passes. F2P: four lists disagree today
  (Rust x2, `content/ai-loop/license_gate.py`, `tools/gltf-intake.mjs`). P2P:
  `cargo test -p engine-assets --lib` green; golden unchanged. Plan:
  `docs/plan/forge-adoption.md` AD-6. BLOCKED: owner Decision 6 in
  `docs/plan/forge-adoption.md` section I. needs: none.
- [ ] AD7 editor then endpoint: one scene-to-actor adapter with real extents —
  a single function turns a `Document` into spatial `Actor` rows from Collider
  half extents and world scale (bare node = 1 m, labelled `default size`); the
  CAP1 gate, `findings.rs`, `scene.brief`, `scene.spatial`, `scene.place`,
  `scene.query`, intents, assistant and build_level all call it;
  `play/world_load.rs::blocks` keeps the arena height filter. Source: E03#1,
  E05#13, E13#1. Diff: good. Tier A. Size M. Owns step a:
  `crates/editor/src/spatial_view.rs` (new), hunks in `capability.rs` and
  `findings.rs`. Step b: endpoint consumer hunks in `scene_tools.rs`,
  `scene_query.rs`, `intents.rs`, `assistant.rs`, `build_level.rs`. Accept:
  `cargo test -p engine-editor --lib spatial_view` x2: a wall collider half
  [2,1.5,0.1] with a crate inside reports overlap; the adapter's rows equal
  the play blocks for one document; every caller gets equal rows; spatial
  digests re-pinned with a named receipt. F2P: `scene_tools.rs:490-492` and
  `capability.rs:486-489` size every node as a unit cube. P2P: `cargo test -p
  engine-editor --lib` and `-p engine-endpoint --lib` green; golden unchanged.
  Plan: `docs/plan/forge-adoption.md` AD-7. needs: none.
- [x] AD8 endpoint: a door for the verifiers — optional dependency on
  `engine-playtest` (and `engine-proof`) behind feature `verify`; a `verify`
  pack with read-only verbs `bug.replay`, `trace.summary`, `replay.verify`,
  `replay.bisect` (bodies from RB1), and a door-parity test: the same 20
  mirror tasks through `playtest/src/door.rs` and through the spawned
  `e2040-mcp` stdio binary must give equal outcome digests; the bin registers
  `look.judge`. Source: C1 gap 3, E06, E09#1, E13#2/#4, E11#3. Diff: good.
  Tier A. Size M. Owns `crates/endpoint/Cargo.toml`,
  `crates/endpoint/src/packs/verify.rs` (new),
  `crates/endpoint/tests/door_parity.rs` (new). Accept: `cargo test -p
  engine-endpoint --features mcp,verify --test door_parity` x2: digests equal
  on both paths; a seeded_bug tape bisects to the seeded tick through stdio;
  `look.judge` answers a fixture instead of `unsupported`. F2P: no manifest
  depends on `engine-playtest`; `register_look_judge` has no caller outside a
  test. P2P: `cargo test -p engine-endpoint --lib` green; default build gains
  no dependency; golden unchanged. Plan: `docs/plan/forge-adoption.md` AD-8.
  Owner Decision 2: yes (the `verify` feature edge). needs: RB1. Progress 2026-10-18: the host half lands - engine-endpoint gains the workspace `engine-playtest` edge under `mcp`, `look_host::host_look_judge` bridges the capture/preview/q1 args through `look::judge` (codes look_capture/look_manifest), and `e2040-mcp` registers it at start so `look.judge` no longer answers unsupported; tests look_judge_real (own process) and look_judge_hook green, full endpoint mcp suite 57 lib + 2 hook + 9 doc green (gh run 39121967253). Left: the `verify` feature with engine-proof, the verify pack verbs (bug.replay, trace.summary, replay.verify, replay.bisect) and tests/door_parity.rs. Landed acdde37e. door_parity x2 equal digests (gh runs 39139680891, 39139686099): the first 20 mirror tasks digest equal through the playtest door and the spawned e2040-mcp, and the A7 seeded tape bisects to tick 8 through stdio; feature `verify = ["mcp", "dep:engine-proof"]` (playtest already rides mcp), `packs/verify.rs` serves replay.verify/replay.bisect (playtest RB1 bodies) + bug.replay + trace.summary + verify.proof and takes the replay contract slot so each name has one owner, tool pin gains a `verify` term; endpoint --features mcp,verify lib 630 + it 57 green (39139056420, 39139056482), --features mcp suite green (39139056422), workspace-check and rust-quality PASS. The ARCHITECTURE edge and the workspace-check edge/OWN_TEST_BINARY entries ride commit 99e49fd8.
- [ ] AD9 endpoint: door session ring with a real writer and a `door.status`
  verb — `door_dispatch` pushes one typed event per refusal and failed
  handler; play jobs push their end state; new codes go in the one observe
  registry; with no writer `log.query` answers `not_wired`; `door.status` is
  read-only (revision, packs, budget headroom, audit head, last durable
  generation, build sha; unknown stays unknown). Source: E09#3/#11, E05#9, C3
  G10, E01#9. Diff: good. Tier A. Size S-M. Owns
  `crates/endpoint/src/door_dispatch.rs`, `log_tools.rs`, `packs/view.rs`
  hunks and `crates/observe/src/lib.rs` code table. Accept: `cargo test -p
  engine-endpoint --lib door_ring` x2: a refused `tx.apply` then `log.query`
  returns its code and tick; a fresh session says `not_wired`; `door.status`
  digest equal twice; an unregistered code fails a test. F2P:
  `ServerState::push_log` has only test callers (`log_tools.rs:423`). P2P:
  `cargo test -p engine-endpoint --lib` green; golden unchanged. Plan:
  `docs/plan/forge-adoption.md` AD-9. Owner Decision 3: yes (`door.status`).
  needs: none.
- [ ] AD10 editor: gate undo by capability and group undo by AI session —
  `undo_gated` checks the inverse ops against mask, scope and budget; human
  undo with no token is unchanged; receipts group by capability session.
  Source: E05#2, E13#8. Diff: regular. Tier A. Size S. Owns
  `crates/editor/src/capability.rs` hunk and the `tx_undo` hunk in
  `crates/endpoint/src/mcp.rs`. Accept: `cargo test -p engine-editor --lib
  undo_gated` x2: a token with INSERT only cannot undo a DELETE batch
  (`op_not_allowed`, journal unchanged); 20 AI batches plus 5 human edits on
  other objects, session undo restores the exact pre-session bytes and a
  pinned object is refused by name. F2P: `tx_undo` checks only token == newest
  key (`mcp.rs:892-921`). P2P: `cargo test -p engine-editor --lib` green;
  golden unchanged. Plan: `docs/plan/forge-adoption.md` AD-10. needs: none.
- [x] AD11 spatial: whole-scene unit-mismatch detector — a pure function Landed 30fd08e. unit_scale_x100_scene_flags_one_red (52 props x100 give one red finding) and unit_scale_small_population_is_silent (4 props silent, floor 16) green in engine-spatial lib 303 passed, run 39057202304. Note: the pure function has no door caller yet (waits for AD7).
  compares the median object size with the layout extent (population floor 16,
  integer cm) and returns one red root-cause finding listed first; evidence
  only, never a rescale; not added to `SPATIAL_CHECKS` (length pinned at 18).
  Source: E03#8, E02#12, C3 G24; Forge `SpatialScale.cs`, old
  `SceneSpatialService.Scale.cs`. Diff: regular. Tier A. Size S. Owns
  `crates/spatial/src/scale.rs` (new) and one `lib.rs` export line. Accept:
  `cargo test -p engine-spatial --lib unit_scale` x2: 52 props all sized x100
  give one red finding first; a 4-prop scene is silent; existing goldens
  unchanged. F2P: no unit-scale function exists (no "median" in
  `crates/spatial/src`). P2P: `cargo test -p engine-spatial --lib` green;
  golden unchanged. The door call needs AD7. Plan:
  `docs/plan/forge-adoption.md` AD-11. needs: none.
- [ ] AD12 editor then endpoint: durable door writes with a single-writer
  lease — compose the capability gate with `persist_sealed` using the
  `apply_persisted` pattern (stage on clones, seal, then commit); a failed
  persist is a refusal; a lease file on the project folder refuses a second
  door. Source: E05#1, E13#10, C3 G7/D3. Diff: good. Tier A. Size M. Owns
  `crates/editor/src/capability.rs` and `durable.rs` hunks, then
  `crates/endpoint/src/mcp.rs` and `startup.rs` hunks. Accept: `cargo test -p
  engine-endpoint --lib durable_door` x2: kill after ack leaves the edit
  present on restart; kill before ack leaves it absent; a forced persist
  failure returns an error and changes nothing; the same key after restart
  returns the stored receipt; a second door on the folder is refused. F2P:
  `tx_apply` and `tx_undo` never call a persist function. P2P: `cargo test -p
  engine-editor --lib` and `-p engine-endpoint --lib` green; golden unchanged.
  Plan: `docs/plan/forge-adoption.md` AD-12. Owner Decision 1: yes (tier 1
  default). needs: AD2.
- [ ] AD13 endpoint and rules: commit-class `rules.set`, no-effect refusal,
  hostile-input fuzz — `rules.set` gets the Commit card, approval bound to
  script hash and rules revision, and `expected_hash` when a script is
  installed; a script that fires no action in any smoke world is refused
  `NoEffect` unless declared a no-op; a fixed-seed matrix test (reuse the
  engine's std-only property pattern, no new dependency) feeds random token
  streams to vet; a mid-round `rules.set` applies at the next tick boundary
  and records ScriptSwap. Source: E08#4/#5/#6/#9, C3 M41/G29, E13#5/#13. Diff:
  good. Tier A. Size M. Owns `crates/endpoint/src/packs/rules.rs`,
  `packs/rules/verbs.rs`, `crates/rules/src/rules.rs`,
  `crates/rules/tests/vet_props.rs` (new). Accept: `cargo test -p
  engine-endpoint --features rules --lib rules_commit` x2: `rules.set` without
  approval refuses and changes nothing; a stale `expected_hash` names the
  current one; `cargo test -p engine-rules --lib no_effect`: `fn
  on_tick(me){let x=1;}` refuses; `vet_props` never panics and every failure
  is a typed `RulesError`; reload at tick 300 replays equal. F2P:
  `rules.set("")` is accepted (`tests.rs:303`) and the card is Observe. P2P:
  `cargo test -p engine-rules --lib` green; golden unchanged. Plan:
  `docs/plan/forge-adoption.md` AD-13. BLOCKED: owner Decision 1 in
  `docs/plan/forge-adoption.md` section I. needs: none.
- [ ] AD14 observe: supervisor hardening — heal tape rows carry before and
  after digests; `reset(code, VerifiedRecovery)` needs proof; re-registering a
  policy keeps the budget unless asked; circuit breaker (then hedge) moves
  from test files into `observe/src/breaker.rs`, std-only, tick or revision as
  clock; a heal that changes sim or scene state enters the tape or the journal
  or does not run. Source: E01#2/#11, E09#7/#8, E13#7. Diff: good. Tier A.
  Size M. Owns `crates/observe/src/supervisor.rs`,
  `crates/observe/src/breaker.rs` (new),
  `crates/observe/tests/circuit_vectors.rs` and `bulkhead_vectors.rs` (point
  at src). Accept: `cargo test -p engine-observe --lib supervisor_proof` x2: a
  heal that "applies" while read-back still faults ends in quarantine;
  re-register does not reopen the budget; tape rows carry both digests; the
  existing vectors run against the src module; a timeout storm opens the
  breaker and admits one half-open probe. F2P: `HealTapeRecord` has no
  digests, `reset` takes no proof, the breaker exists only in `tests/`. P2P:
  `cargo test -p engine-observe --lib` green; golden unchanged. Plan:
  `docs/plan/forge-adoption.md` AD-14. needs: none.
- [ ] AD15 assets: cook-time clip gates — measure raw-clip foot slide with
  `anim::feet` (assets already depends on anim) and refuse with clip name and
  mm; `BoneMap::build` returns coverage and refuses when a required set (root,
  hips, spine, arms, legs, head) is unmapped; an explicit prefix list
  (`mixamorig:`) is stripped on both sides before lookup. Source: E10#1/#2/#7,
  E07#5, C3 G32/G23. Diff: good. Tier A. Size M. Owns
  `crates/assets/src/retarget.rs`. Accept: `cargo test -p engine-assets --lib
  clip_gates` x2: a planted 3 cm foot slide refuses at cook, a still foot
  passes, a negative control fails the metric; a 65-joint target with 3
  unmapped joints lists exactly those 3; prefixed and plain names give equal
  bytes. F2P: `slide_mm` has no use in `crates/assets`; `retarget_clip`
  returns the clip only (`retarget.rs:396-412`). P2P: `cargo test -p
  engine-assets --lib` green; golden unchanged. Plan:
  `docs/plan/forge-adoption.md` AD-15. needs: none.
- [ ] AD16 physics: ragdoll joint table with a hinge kind — amends PX9. Typed Progress 2026-10-17: a `JointKind::{Hinge, Spherical}` table exists (`JointTable::humanoid`, `joint_table_humanoid_defaults`, ragdoll.rs, fb7466b) with knees and elbows as hinges. Left: the proven numbers (spine 20/20/35, forearm and leg hinge 1/145, thigh 70/40/30), the 0..180 degree and joints-equal-bodies-minus-one refusals, `joint_limits.rs`, and the `spawn` read (the table says it is measurement only); needs AD17 open.
  `JointKind::{Ball, Hinge}` keyed by humanoid bone, with the proven numbers
  (spine 20/20/35, arm ball 85/85/70, forearm and leg hinge 1/145, thigh ball
  70/40/30, head 35/35/45) and their failure notes as comments; refuse a limit
  outside 0..180 degrees; refuse a skeleton where joints differ from bodies
  minus one. The Rapier ragdoll is client-local and never hashed. Source:
  E02#2/#8; Forge `RagdollWizard.cs:60-84`; old `RagdollBodyPlan.cs`. Diff:
  good. Tier A. Size M. Owns `crates/physics/src/joint_limits.rs` (PX9) and
  `crates/physics/src/ragdoll.rs` spawn hunk. Accept: `cargo test -p
  engine-physics --features rapier --lib joint_table` x2:
  `knee_flex_reaches_145_not_more`; `no_joint_over_limit` over 1,000 seeds; a
  skeleton with a missing joint is refused; `balance_hash` unchanged. F2P:
  `ragdoll.rs:1139-1147` gives every joint one cone on all three axes and
  there is no hinge. P2P: `cargo test -p engine-physics --features rapier
  --lib` green; golden unchanged. Amends PX9 (same file, joint_limits.rs): the
  builder who takes PX9 reads this row first. PX1 and PX10 are done. Plan:
  `docs/plan/forge-adoption.md` AD-16. needs: AD17.
- [ ] AD17 physics: ragdoll params builder — one builder shared by
  `ragdoll.rs`, `powered.rs`, `rapier.rs` sets solver passes (chosen by test,
  Rapier 0.36 default is 4) and bone density from a mass fraction times a
  total (80 kg; sum within 1% or refuse); one `ccd_for` policy used at spawn;
  fix the CCD text in `physics/src/lib.rs:27-29` and PX10. Hash-free. Source:
  E02#1/#3/#5; Forge `RagdollWizard.cs`; old `RagdollBodyPlan.cs` (8/4 too
  low, 12/8 worked in PhysX). Diff: good. Tier A. Size S-M. Owns
  `crates/physics/src/ragdoll.rs`, `powered.rs`, `rapier.rs`, `lod.rs`, doc
  line in `lib.rs`. Accept: `cargo test -p engine-physics --features rapier
  --lib ragdoll_params` x2: `ragdoll_mass_sums_to_80kg`; a 12-bone chain under
  a spear-like impulse keeps every joint gap at or under 5 cm for 300 ticks; a
  ccd policy matrix (dynamic on, kinematic off); `balance_hash` unchanged.
  F2P: `ragdoll.rs:361-364` builds `IntegrationParameters` with dt only and
  `ragdoll.rs:387-389` uses density 1.0. P2P: `cargo test -p engine-physics
  --features rapier --lib` green; golden unchanged. Plan:
  `docs/plan/forge-adoption.md` AD-17. needs: none.
- [ ] AD18 spatial: 3D, yaw-aware overlap in the report, the solver and the
  gate (prerequisite of SP16) — one box test: 3D interval first, then the XZ
  footprint rotated by the yaw (exact at 90 degrees; integer bound
  `(hx+hz)*1415/2000` at 45-degree steps; otherwise the bounding circle with a
  yellow label). Same function in `place.rs` verify. No new check name
  (catalog stays 18). Source: E03#2/#5; old `SceneSpatialService.Pairs.cs`,
  `PlacementSolver.cs`. Diff: good. Tier A. Size M. Owns
  `crates/spatial/src/index.rs`, `checks.rs` overlap arm, `place.rs` verify
  hunk. Accept: `cargo test -p engine-spatial --lib overlap_3d` x2: a lamp on
  a table is clear; a crate under a table is clear; a 4 m plank at 45 degrees
  is never intersected by a solver result; 20 permutations give equal digests;
  flat-room goldens byte-identical, stacked-scene digests re-pinned with a
  named receipt. F2P: the overlap arm uses x and z only and `xyz_pairs` has
  test callers only. P2P: `cargo test -p engine-spatial --lib` green; golden
  unchanged. SP16 should follow AD18. Plan: `docs/plan/forge-adoption.md`
  AD-18. needs: none.
- [x] AD19 provider: status map and rate-limit gate — 400, 404 and 422 do not Landed aa644693. Run 39052718180.
  retry; 5xx, 408 and socket errors retry with capped seeded jitter inside the
  request deadline; 429 waits for Retry-After once behind one shared gate per
  provider; a spend cap refuses. May add one `AiError` variant (protocol call
  for the owner). Source: E01#3, C2 U1. Diff: good. Tier A. Size S-M. Owns
  `crates/provider/src/transport.rs` and, if a variant is added,
  `crates/provider/src/protocol.rs`. Accept: `cargo test -p engine-provider
  --lib status_map` x2 against a stub server: 400 once gives zero retries; 429
  with Retry-After 2 and 20 parallel callers gives one wait; 503 twice then
  200 gives two retries inside the deadline; the cassette replay is
  byte-equal. F2P: `map_status` sends 400, 404 and 422 to the retryable
  `Unavailable` and 429 to `QuotaAuth` (`transport.rs:505-511`); there is no
  Retry-After parsing. P2P: `cargo test -p engine-provider --lib` green;
  golden unchanged. Plan: `docs/plan/forge-adoption.md` AD-19. needs: none.
- [ ] AD20 endpoint: reach honesty at the door — step 1: `scene.query
  reachable` refuses `needs_nav_view` when no bake exists (the code
  `scene.spatial` already uses); step 2: one adapter from the nav mesh or
  tiles to `ReachView` and `NavView` in endpoint, read by `build_level` too
  (spatial stays nav-free). Source: E03#4, E13#1. Diff: regular. Tier A. Size
  S then M. Owns `crates/endpoint/src/scene_query.rs` (step 1 hunk, then the
  adapter hunk). Accept: `cargo test -p engine-endpoint --lib reach_door` x2:
  with no nav view `reachable` refuses `needs_nav_view`; two rooms joined by
  one door answer blocked across the wall once a view exists; mesh and tiles
  agree on 20 seeded layouts. F2P: every staged actor shares polygon 0 and
  every pair answers reachable (`scene_query.rs:952-956`, `:1062-1070`). P2P:
  `cargo test -p engine-endpoint --lib` green; golden unchanged. Step 2 is
  better after AD7. Plan: `docs/plan/forge-adoption.md` AD-20. needs: none. Progress 2026-10-18: crates/nav/src/reach.rs landed 135bb63f (reach_view gives every actor its real nav polygon, nav_view for the bake checks, two rooms read Blocked until a doorway joins them, mesh and tiles agree on 20 seeded layouts; run 39076864508, clippy 39077965931); left: endpoint needs_nav_view refusal and replacing open_floor in scene_query.rs (lane A). Progress 2026-10-18: endpoint half landed dc6710ae: scene.query reachable reads each staged actor's engine_nav mesh polygon from ServerState::with_nav and refuses needs_nav_view without a mesh, open_floor is gone; reach_door_refuses_needs_nav_view_when_no_mesh_is_held and reach_door_two_rooms_are_blocked_across_the_wall_until_a_door_joins_them green (run 39109659487); mesh and tiles on 20 layouts is engine-nav's reach_door test (135bb63f). Left: a host that bakes and hands the door a mesh (none does yet), and build_level reading the same view.

Second pass (owner 2026-10-11, `docs/plan/forge-adoption.md` section J): four
more research passes gave 43 candidate rows. These 14 are the best; the rest
stay in the plan, ranked. Every row has F2P and P2P and leaves the golden
hash alone. Ready now: AD23 and PX40. The others wait behind an earlier row
in their crate or on an owner decision. Three native ids were already taken,
so D-AX18 is AX23, D-SP61 is SP66 and D-PF21 is PF24.

- [ ] AD21 endpoint: claim check — a read-only verb `claim.check` takes an
  agent's own claim (`landed`, `confirmed` or `viewed`) and the receipt key it
  rests on. It answers `backed`, or refuses `claim_not_backed` and names the
  missing proof. `landed` needs an apply receipt for that key with equal
  digests (AD2); a preview key can never claim `landed`. `confirmed` needs a
  later read at that revision with checks_run above 0 and no findings (AD3).
  `viewed` needs a capture or view read at that revision. A key the journal no
  longer holds answers `unknown`, never `backed`. Source: A21 (Forge
  `NrpGate.cs`); the fix-edge half of that gate is cut. Diff: good. Tier A.
  Size S. Owns `crates/endpoint/src/receipts.rs` (the pure check) and the verb
  in `crates/endpoint/src/packs/view.rs`. Accept: `cargo test -p
  engine-endpoint --features mcp --lib claim_check` x2: a preview key claiming
  `landed` is refused; an apply key with equal digests is backed; `confirmed`
  with no later read is refused; one flipped digest byte flips the answer;
  every refusal names a live verb. Consumer: the AD25 harness (the agent never
  grades itself) and any agent's closing step. F2P: nothing checks an agent
  claim against a receipt (`grep -rniw claim crates/endpoint/src` finds two
  unrelated comments). P2P: `cargo test -p engine-endpoint --lib` green; the
  tool-list budget tests unchanged; golden hash unchanged. Plan:
  `docs/plan/forge-adoption.md` section J (A21). needs: AD2, AD3.
- [ ] AD22 tools: playable is six named gates — a `playable` verdict passes
  only when compile, play with 0 errors, save then reopen, package, packaged
  launch and packaged smoke all pass. Each gate reads one record: compile and
  play from `target/proof/<gate>-latest.json` (AD4), save then reopen from the
  AD2 record, and the last three from the `tools/ship.mjs` pack and run
  receipts. A gate with no record prints `not_wired` and counts as FAIL; a
  missing gate FAILs too; the line names the gate. Source: A29 (Forge
  `claim-gate.mjs`). Diff: regular. Tier A. Size S. Owns `tools/proof.mjs`
  (verdict hunk), `tools/proof.test.mjs` and the verdict print in
  `tools/ship.mjs`. Accept: `node --test tools/proof.test.mjs` x2: a stub with
  package `not_wired` FAILs and names `package`; all six passing PASSes; a
  stale record FAILs; a green record cannot make a missing gate pass.
  Consumer: the GM3 ship proof (`tools/ship.mjs pack` and `run` end by
  printing the verdict). F2P: `tools/proof.mjs` has no `not_wired` state and
  no playable verdict (`grep -n playable tools/proof.mjs` finds none). P2P:
  `node --test tools/` green; golden hash unchanged. Plan:
  `docs/plan/forge-adoption.md` section J (A29). needs: AD4.
- [x] AD23 playtest: trajectory checks — an eval task can require a call order
  (`scene.read` before `tx.apply`), and a write that names an object id no
  earlier read returned fails with `ungrounded: <id>`. One check in the eval
  harness; the door executor logs each call and each id a read returned.
  Source: A22 (Forge `eval-asserts.mjs` checkTrajectory). Diff: good. Tier A.
  Size S. Owns `crates/playtest/src/eval.rs` (task script and check hunks) and
  `crates/playtest/src/door.rs` (call log hunk). Accept: `cargo test -p
  engine-playtest --lib trajectory` x2: a run that writes a ghost id FAILs
  `ungrounded`; a run that skips a required read FAILs the order check; read
  then write PASSes; one flipped id byte flips the result; the same tape gives
  equal rows. Consumer: the A6 eval authoring rows (`script_steps` in
  `eval.rs`) and the AD25 harness. F2P: no order or grounding check exists in
  `crates/playtest/src` (the `trajectory` in `agent.rs` is a fighter pose
  path). P2P: `cargo test -p engine-playtest --lib` green; the 75-row eval
  report bytes unchanged; golden hash unchanged. Plan:
  `docs/plan/forge-adoption.md` section J (A22). needs: none. Landed 4aef2869. door.rs Door logs each call and the ids a read returned or a write named; eval/trajectory.rs check_trajectory judges order first then grounding (ungrounded: <id>); EvalTask::required_calls makes the clean apply rows (a13, a14) read then write on the fixture scene, row bytes unchanged. Tests trajectory_ghost_id_fails_ungrounded, trajectory_skipped_read_fails_the_order_check, trajectory_read_then_write_passes, trajectory_one_flipped_id_bit_flips_the_result, trajectory_apply_rows_hold_the_order_and_replay_equal; the EV1 suite stays green. gh run 39105225319 (engine-playtest --lib highlights brief_eval trajectory scorecard frame_stale: 36 passed, 1 ignored) and full engine-playtest --lib 373 passed, run 39103617132.
- [ ] AD24 playtest: capture visibility — each capture receipt names its focus
  subject's state, read from the same frame's view truth (SP62 `view.observe`:
  a body with a null screen rect is `not_in_view`; a body with a rect and
  cover above 0 is `visible`). No second projection is written. States:
  `not_found`, `disabled`, `not_in_view`, `visible`. The field is left out
  when unknown; `occluded` is never guessed, it waits for SP65 `view_sees`. A
  freshness key (pose rounded to 0.1 m, yaw and pitch to 0.1 degree, fov) is
  stored beside the frame digest; a preview capture never updates it. Source:
  A30 (Forge `ReceiptProof.cs`). Diff: good. Tier A. Size S-M. Owns
  `crates/playtest/src/frame_check.rs` (hunk) and the receipt field in
  `crates/game/src/capture.rs`. Accept: `cargo test -p engine-playtest --lib
  capture_subject` x2: a subject with a null rect gives `not_in_view`; one
  with a rect and cover gives `visible`; an unknown subject leaves the field
  out; a preview capture leaves the key alone; one flipped pose byte changes
  the key. Consumer: the look judge (`crates/playtest/src/look/judge.rs`)
  turns a capture whose subject is not `visible` into Abstain (AD1) instead of
  a score. F2P: `frame_check.rs` reads digests only and no capture receipt
  names its subject. P2P: `cargo test -p engine-playtest --lib frame_check`
  green; receipts without the field still parse; golden hash unchanged
  (outside the tick). Plan: `docs/plan/forge-adoption.md` section J (A30).
  needs: AD1, SP62.
- [ ] AD25 playtest: scene IQ eval — eight tasks over seeded scenes: find, fix
  floating, place on ground, separate overlap, top-down shot, camera follow,
  undo and light a scene. The agent never grades itself: the harness seeds the
  scene, reads the door state and the capture, then restores the scene.
  Harness calls are not charged to the agent's budget and a read is never cut
  mid-object. A do-nothing agent must FAIL all eight; the reference agent must
  PASS all eight. Source: A35 (Forge `scene-iq.mjs`, `scene-iq-sim.mjs`).
  Diff: good. Tier A. Size M. Owns `crates/playtest/src/spatial_eval.rs` (new
  section) and `fixtures/scene-iq/` (new). Accept: `cargo test -p
  engine-playtest --lib scene_iq` x2: do-nothing FAILs 8 of 8; the reference
  agent PASSes 8 of 8; the scene digest is equal before and after each task; a
  trajectory break (AD23) FAILs the run; the light task says `unknown`, not
  fail, when a range is unreadable (SP66). Consumer: the AG1 scorecard prints
  the scene IQ pass rate beside the 100 spatial tasks. F2P: `spatial_eval.rs`
  holds place, query and fix tasks only: no undo, camera or light task. P2P:
  the 100 spatial gold tasks keep their scores; `cargo test -p engine-playtest
  --lib` green; golden hash unchanged. Plan: `docs/plan/forge-adoption.md`
  section J (A35). needs: AD7, AD23, AD24, AD28, SP66.
- [ ] AD26 endpoint: refusals teach — one table maps every refusal code to a
  closed advice code (a short fixed list), a retry flag and one next call that
  is a live verb. A refusal comes back as a tool result with `isError` true
  that carries code, message, advice, retry flag and next call; unknown tool
  and malformed JSON-RPC stay protocol errors. A bad argument list reports
  every bad field in one reply, each with a JSON pointer, a reason code and a
  redacted hint; it never auto-corrects. Merges A25 (Forge `WireEnvelope.cs`),
  B43 (old `ErrorRemediation.cs`), B44 (old `ValidationErrorEnvelope.cs`) and
  C62 (MCP tools spec). Diff: good. Tier A. Size M. Owns
  `crates/endpoint/src/advice.rs` (new, the one table) and hunks in
  `crates/endpoint/src/discover.rs`, `mcp.rs` (`call_by_name`), `mcp_dto.rs`
  and `receipts.rs`. Accept: `cargo test -p engine-endpoint --features mcp
  --lib refusal_advice` x2: a test that scans every `McpToolError::new` code
  fails on a code with no table row; every next call is in the live registry;
  a stale-revision refusal arrives as `isError` true with advice and retry
  flag; `unknown_tool` stays an RPC error; a call with three bad fields
  returns three pointers; a secret-shaped value is redacted in the hint; equal
  bytes twice. Consumer: every AI caller of the door;
  `crates/endpoint/tests/mcp_stdio.rs` proves it end to end. F2P: every
  refusal leaves as an RPC `invalid_params` error (`discover.rs`,
  `From<RpcError> for ErrorData`); `Receipt` has a retry flag and two verbs
  but no advice code; no JSON pointer error type exists. P2P: `cargo test -p
  engine-endpoint --lib` green; a test that reads a refusal as an RPC error
  moves to the result shape and keeps its code and recovery assertions; golden
  hash unchanged. UNVERIFIED: whether a client shows RPC error text to the
  model; record what the stdio client shows for both shapes first. Plan:
  `docs/plan/forge-adoption.md` section J (A25, B43, B44, C62). needs: AD2.
- [ ] AD27 endpoint: playbook prompts — the door advertises the MCP prompts
  capability and serves up to ten recipes over `prompts/list` and
  `prompts/get`. Each recipe is a numbered list of real verbs with a
  precondition check, an abort rule and the rule "name the first failing
  condition as the cause". Start with ten, not the old 95 (the first ten): a
  scene edit did not land, an object floats, a rule never fires, the bout
  looks stuck, a replay differs, a capture looks wrong, a refusal repeats, an
  undo did not restore, the tool list is too long, a call is slow. A recipe
  may name only verbs that exist when it lands; one that needs a later verb
  waits for that row. Source: B41 (old `Prompts/*.md`, `PromptCatalog.cs`).
  Diff: good. Tier A. Size M. Owns `crates/endpoint/src/prompts.rs` (new) and
  hunks in `crates/endpoint/src/mcp.rs` (`get_info` and the two handlers).
  Accept: `cargo test -p engine-endpoint --features mcp --lib playbooks` x2:
  `prompts/list` names the recipes written;
  `every_verb_named_in_a_playbook_exists` checks each verb against the live
  registry and fails on an invented name (the old playbooks named verbs that
  never existed); an abort branch survives the round trip; equal bytes twice;
  the tool-list budget does not move. Consumer: any MCP client that lists
  prompts; a recipe is the first thing a weak model reads when a verb refuses.
  F2P: `get_info` advertises the tools capability only (`mcp.rs`,
  `ServerCapabilities`). P2P: `cargo test -p engine-endpoint --lib` green;
  `tool_count_pin` unchanged; golden hash unchanged. Ordered behind AD29: both
  edit `get_info`. Plan: `docs/plan/forge-adoption.md` section J (B41). needs:
  AD29.
- [ ] AD28 editor: `scene.why` — a read-only query `scene.why(object, field)`
  answers the last receipt that wrote that field: its revision, its source
  (human or AI session), whether it can be undone, and the receipt id. A field
  never written since the project opened answers `never_written`; a revision
  the journal dropped answers `unknown`. Source: C64 (journal read; idea from
  reverse debugging). Diff: good. Tier A. Size M. Owns a read helper in
  `crates/editor/src/commands.rs` (beside `parent_receipts`) and the query
  kind in `crates/endpoint/src/scene_query.rs`. Accept: `cargo test -p
  engine-endpoint --features mcp --lib scene_why` x2: after three edits to one
  field the answer names the third; a human edit and an AI edit are told
  apart; an undone write names the earlier writer; a fresh field answers
  `never_written`; equal bytes twice; `cargo test -p engine-editor --lib
  journal_replay_rebuilds_identical_chains` stays green. Consumer: AI callers
  of `scene.query` (a new kind beside `reachable`) and the AD25 undo task.
  F2P: nothing answers who wrote a field (`grep -rn scene.why crates` finds
  none; `parent_receipts` at `commands.rs:986` is private). P2P: `cargo test
  -p engine-editor --lib` and `-p engine-endpoint --lib` green; golden hash
  unchanged. Ordered behind AD2 (`commands.rs`) and AD20 (`scene_query.rs`):
  one builder per crate. Plan: `docs/plan/forge-adoption.md` section J (C64).
  needs: AD2, AD20.
- [ ] AD29 endpoint: read-only snapshots as MCP resources — the door
  advertises the resources capability and serves `resources/list` and
  `resources/read` for the scene brief, the spatial check list, the verb and
  pack map and the budgets. Snapshots cost nothing from the tool-list budget
  (`TOOL_LIST_BUDGET_CHARS` is 43,514 at `mcp.rs:149`); tools stay for
  actions. A read of an unknown uri refuses with a live next call (the AD26
  shape). Source: C61 (MCP resources spec; Forge `forge://map`). Diff: good.
  Tier A. Size M. Owns `crates/endpoint/src/resources.rs` (new) and hunks in
  `crates/endpoint/src/mcp.rs` and `discover.rs`. Accept: `cargo test -p
  engine-endpoint --features mcp --lib resources` x2: `resources/list` serves
  the four snapshots; `resources/read` of the brief equals the `scene.brief`
  bytes; the same uri twice gives equal bytes; an unknown uri refuses; the
  budget stays 43,514 and the budget tests stay green. Consumer: any MCP
  client that reads a resource instead of spending a tool call on a read. F2P:
  `get_info` advertises tools only and `resources/list` is not served. P2P:
  `cargo test -p engine-endpoint --lib` green; `tool_count_pin` unchanged;
  golden hash unchanged. Ordered behind AD26: both edit `mcp.rs` and
  `discover.rs`. Plan: `docs/plan/forge-adoption.md` section J (C61). needs:
  AD26.
- [x] PX40 endpoint: `physics.read` — a read-only verb in a new `physics` pack Landed ded4b98. packs::physics physics_read_orders_blows_over_the_line_and_the_fall_line, physics_read_matrix_equals_the_balance_owner, physics_read_refuses_a_zero_line_and_names_the_legal_range, physics_read_reports_live_bout_facts_as_not_wired green (run 39057713346) and tests/it mcp_stdio physics_read_answers_equal_bytes_twice_over_stdio green (run 39058159379); `node tools/crate-map.mjs --check` PASS.
  answers what a blow does to a fighter's balance: for a stagger line and a
  blow strength (both Q8) it returns the verdict (`Steady`, `Staggered`,
  `Stumbling` or `Fallen`), the recovery ticks and the five standard rows of
  the shove-response matrix. It is pure and integer: it reads no bout and
  writes nothing. The door's live bout runs without the balance rules, so live
  per-slot balance, bone velocities, bone contacts and joint drift answer
  `not_wired` (never zero) and stay plan only. Source: D-PX40 (fourth audit);
  plan N-29. Diff: good. Tier A. Size S. Owns
  `crates/endpoint/src/packs/physics.rs` (new), the pack line in
  `crates/endpoint/src/packs/mod.rs`, `crates/endpoint/Cargo.toml`
  (`engine-physics` joins the `mcp` feature, no rapier) and
  `docs/CRATE-MAP.md` (regenerated by `node tools/crate-map.mjs`). Accept:
  `cargo test -p engine-endpoint --features mcp --lib physics_read` x2: a blow
  at the stagger line answers `Staggered` and one under it `Steady`; a blow
  above the fall line answers `Fallen` with its recovery ticks; the matrix
  rows equal `response_matrix` for the same params; a stagger line of 0
  refuses `bad_args` and names the legal range; `live_bout` says `not_wired`;
  the same call through the stdio door in `crates/endpoint/tests/mcp_stdio.rs`
  gives equal bytes twice. Consumer: an AI that tunes an encounter asks this
  verb instead of guessing a blow strength (the shape of `hit.simulate` in the
  break pack). F2P: no door verb answers a balance fact (`sim.observe` returns
  hp, stamina, a phase word, position and yaw) and `engine-endpoint` has no
  edge to `engine-physics`. P2P: `cargo test -p engine-endpoint --lib` green;
  `node tools/crate-map.mjs --check` green; the budget and `tool_count_pin`
  tests unchanged; golden hash unchanged. Plan: `docs/plan/forge-adoption.md`
  section J (D-PX40). needs: none.
- [ ] AX23 endpoint: say what changed — `tx.apply` returns a semantic change
  list: the object ids it touched, the relation edges added and removed (the
  `engine-spatial` semantic diff) and the document digest before and after. A
  new `scene.diff(rev_a, rev_b)` query gives the same list between two
  revisions by replaying the journal on a staged clone. A revision the journal
  no longer holds answers `unknown_revision`, never an empty list. Source:
  D-AX18 (fourth audit; AX18 is taken by an old research note, so the row is
  AX23). Diff: good. Tier A. Size M. Owns the `tx_apply` and output hunks in
  `crates/endpoint/src/mcp.rs` and `crates/endpoint/src/mcp_dto.rs`, the query
  kind in `crates/endpoint/src/scene_query.rs`, and
  `crates/spatial/src/diff.rs` only if a field is missing. Accept: `cargo test
  -p engine-endpoint --features mcp --lib scene_diff` x2: a batch that moves
  one crate lists that id and its edges; a no-op batch lists none;
  `scene.diff` between two revisions equals the lists of the batches between
  them; an evicted revision answers `unknown_revision`; equal bytes twice.
  Consumer: AI callers of `tx.apply` read the change list instead of rereading
  the scene (`crates/endpoint/tests/mcp_stdio.rs` proves it). F2P:
  `TxApplyOutput` has one text line per op and no ids, edges or digest pair;
  `diff_graph` reaches `scene.place` (`scene_tools.rs:2428`) and `scene.query`
  only. P2P: `cargo test -p engine-endpoint --lib` and `-p engine-spatial
  --lib` green; the existing `tx.apply` fields are unchanged; golden hash
  unchanged. Plan: `docs/plan/forge-adoption.md` section J (D-AX18). needs:
  AD2, AD7.
- [ ] EA6 endpoint: a per-caller capability token — the door builds its
  session token from the host's scope policy and the tool cards (`ScopePolicy`
  and `CapabilityCard` in `tool_capabilities.rs`) instead of
  `Capability::unrestricted` in `build_server`. A write the card or scope does
  not grant is refused with the capability code. The default tier is the one
  Decision 1 picks. Source: D-EA6 (fourth audit). Diff: good. Tier A. Size
  S-M. Owns `crates/endpoint/src/startup.rs` (`build_server` and the config
  field) and a hunk in `crates/endpoint/src/tool_capabilities.rs`. Accept:
  `cargo test -p engine-endpoint --features mcp --lib door_token` x2: a token
  built from an INSERT-only policy refuses a DELETE batch with the capability
  code and leaves the journal unchanged; a policy file with a wrong version
  fails closed; a default-policy session still lands insert, move and set
  batches. Consumer: the `e2040-mcp` binary
  (`crates/endpoint/src/bin/e2040-mcp.rs`) reads the policy path at startup.
  F2P: `build_server` mints `Capability::unrestricted` for every caller, so
  the capability code cannot fire at the shipped door. P2P: `cargo test -p
  engine-endpoint --lib` green; golden hash unchanged. Plan:
  `docs/plan/forge-adoption.md` section J (D-EA6). Owner Decision 1: yes
  (tier 1 default). needs: AD10.
- [ ] PF24 perf: wall time on play receipts — `play.step` and `play.run` take
  an opt-in `timing` flag and then carry a `timing` object: step wall ms (p50
  and p99) labelled `software` (a CPU run, never a reference-GPU claim). A
  missing sample answers `unknown`, never zero. Wall time stays out of the
  receipt digest and out of every hash; with the flag off the output is
  byte-identical to today. Source: D-PF21 (fourth audit; PF21 is a ticked row
  in the archive, so the row is PF24). Diff: regular. Tier A. Size S. Owns
  `crates/endpoint/src/play.rs` (args and output hunks) and
  `crates/endpoint/src/play/play_session.rs` (a timing ring beside the step
  loop). Accept: `cargo test -p engine-endpoint --features mcp --lib
  play_timing` x2: with `timing` off the bytes equal the old bytes; with it on
  the object holds p50, p99 and the label; a run of zero steps answers
  `unknown`; the receipt digest is equal with timing on and off; `--test
  step20x2` stays green. Consumer: the external-MCP pass in
  `crates/playtest/src/eval.rs`, which records only a handshake time today.
  F2P: no play output carries a time (the `play.rs` header says "no wall
  clock"). P2P: `cargo test -p engine-endpoint --lib` green; golden hash
  unchanged. Ordered behind AD3: one builder per crate. Plan:
  `docs/plan/forge-adoption.md` section J (D-PF21). needs: AD3.
- [ ] SP66 spatial: light reach — a point is lit when any light's reach covers
  it. A directional light covers every point; a point or spot light covers the
  points inside its range; a light whose range cannot be read (not finite, or
  zero on a point or spot light) makes the answer `unknown`, never `dark`.
  `scene.spatial` and `scene.brief` list the dark objects and the unknown ones
  apart. Source: D-SP61 (fourth audit; SP61 to SP65 are now view-truth rows,
  so the row is SP66); plan N-14. This reads the scene's `Light` components;
  SP64 reads the render side. Diff: good. Tier A. Size M. Owns
  `crates/spatial/src/light.rs` (new, integer centimeters) with one export
  line in `crates/spatial/src/lib.rs`, and the output hunk in
  `crates/endpoint/src/scene_tools.rs`. Accept: `cargo test -p engine-spatial
  --lib light_reach` x2: a farther light covers a point the nearest light
  misses; a point outside every range is `dark`; an unreadable range gives
  `unknown`; digests equal x2; the 18-name `SPATIAL_CHECKS` list is unchanged.
  Consumer: `scene.spatial` through the door and the AD25 light task. F2P: no
  light logic exists in `crates/spatial/src` (grep finds none); `Light.range`
  is stored in `crates/scene/src/component.rs` and no spatial query reads it.
  P2P: `cargo test -p engine-spatial --lib` and `-p engine-endpoint --lib`
  green; golden hash unchanged. Plan: `docs/plan/forge-adoption.md` section J
  (D-SP61). needs: AD7.

### Finish line: wire the look, then the feel (owner 2026-10-11, `docs/plan/finish-line.md`)

Boarded 2026-10-11 after the owner saw every fighter strobe red at the arena start and
asked for "a really visually good AI-native engine". `crates/render` already holds GPU
code for the sky, IBL, GTAO, TAA, bloom, particles and decals (about 650 KB with its own
tests) and none of it reaches the live window: `arena per-pass` prints 0.000 ms for each.
Wire first; a steal from `docs/research/steals-2026-10-11/` replaces an island only when
the EG8 on vs off test shows the island is wrong. Every look row ends with
`node tools/product-check.mjs --no-file` GREEN and its pass listed under "Live GPU
passes" in `target/reports/product/latest.md`. The GPU idles today (gpu_ms p50 0.24 of a
60 ms frame), so wiring spends headroom FC3 has; FC1 measures each landing.

- [x] LF1 render: the window draws the `sky.rs` Hillaire bake — `forward.wgsl` Landed 1bb6097.
  `fs_sky` draws an analytic fog-form sky; `sky.rs` (physical sky bake plus
  EV100 exposure) is already baked on the CPU at presenter start
  (`presenter.rs:365-373`) but the live frame only reads two colours from it
  (`look_fog_color`, zenith); its GPU pass runs only under `--look-probes`
  (`live.rs` `present_frame_probed`). Upload the existing bake as the
  sky-view texture, sample it in `fs_sky`, feed its horizon colour to the
  fog uniform and its EV100 to `exposure`. Diff: good. Tier A. Size M. Owns
  `crates/render/src/present/**` sky hunk and `shaders/forward.wgsl` `fs_sky`.
  Accept: the sky region on vs off differs above the EG8 epsilon; bake bytes
  x2 equal; `arena per-pass` sky above 0 ms on live frames. F2P: a live-sky
  test fails before (`fs_sky` reads no texture). P2P: `cargo test -p
  engine-render --lib` green; golden unchanged. needs: EG9.
- [ ] LF2 render: the window lights with `ibl.rs` — `fs` adds a constant Landed so far (2026-10-17): 1bb6097 (ibl LIVE: armor 1074/1074 and sand 36511/36511 pixels change, capture equal twice). Remaining: a separate ibl ms in arena per-pass (it runs inside forward).
  `AMBIENT_FLOOR`; `ibl.rs` bakes a prefiltered cubemap and BRDF LUT on the
  CPU at presenter start from the embedded
  `content/incoming/studio_small_09_1k.hdr` (`gpu_present.rs:15`,
  `presenter.rs:379`; nothing reads `content/arena/hdri/`) but no live frame
  binds them (the GPU chart runs only under `--look-probes`). Bind both in the
  forward group, add split-sum specular and diffuse irradiance (share EG12
  (a)'s SH9 uniform if it landed). Steal only for a wrong bake: Filament
  `surface_light_indirect.fs`, LearnOpenGL 2.2.2 (`render-steals.md` item 8).
  Diff: good. Tier A. Size M. Owns the forward shader IBL hunk, the presenter
  bind hunk, `ibl.rs` only for a missing accessor. Accept: armor and sand
  regions differ on vs off; capture x2 equal; ibl above 0 ms live. F2P: on vs
  off frames are identical today. P2P: render lib green; golden unchanged.
  needs: EG9.
- [ ] LF3 render: GTAO in the live frame — the blit multiplies `ao_tex` Landed so far (2026-10-17): 1bb6097 (landing 1: GTAO on the device writes ao_tex, 0.165 ms at 720p, feet 0.9907 and wall base 0.9877 vs open sand 1.0000). Remaining: landing 2, the depth-only prepass so AO scales ambient only; GTAO under MSAA.
  (`shaders/blit.wgsl:49`), which stays 1.0; `gtao.rs` computes half-res GTAO
  on the GPU only under `--look-probes`. There is no depth prepass: the
  forward pass clears and stores depth once (`submit.rs:410-419`). First
  landing: run GTAO on that stored depth after forward and bind its full-res
  output as `ao_tex` (it darkens direct sun too). Second landing: a
  depth-only prepass so `fs` can apply AO to ambient and IBL only. Steal for a fix: Bevy
  `ssao.wesl`, XeGTAO (`render-steals.md` item 9). Diff: good. Tier A. Size M.
  Owns present/** GTAO hunk, `blit.wgsl` AO hunk. Accept: feet and wall-base
  regions darker on vs off, open sand within 2%; gtao above 0 ms live. F2P:
  `ao_tex` is constant 1 today. P2P: render lib green; golden unchanged.
  needs: EG2.
- [x] LF4 render: bloom and grade from `post.rs` — the RP1 chain (soft-knee Landed 99e624b.
  bloom extract, blur, add, exposure, ACES, grade) is not wired at all: no
  `Presenter` field holds it and `aa_choice`/`compile_taa_off` have no game
  caller; the blit does ACES alone. Put bloom and grade between forward and the blit's
  ACES; one tone map stays. Diff: good. Tier A. Size M. Owns present/** post
  hunk, `blit.wgsl`. Accept: a torch or sun-glint halo differs on vs off; a
  frame with no pixel above the knee is unchanged; validation errors 0. F2P:
  no bloom pass on live frames. P2P: render lib green. needs: EG9.
- [ ] LF5 render: TAA slice 1, motion vectors — no motion target and no
  prepass exist today. The forward pass writes a second colour target
  `Rg16Float`: current minus previous clip position per pixel, from last
  frame's view-projection and, for skinned bodies, last frame's posed
  positions kept beside the vertex buffer the look rewrites in place
  (`Presenter::refresh_mesh_vertices`). Diff: good. Tier A. Size M. Owns the
  present/** motion target hunks, the forward.wgsl motion output and the
  refresh_mesh_vertices previous-position hunk. Accept: a GPU test: a static
  camera and fixture give an all-zero target; a known camera pan gives the
  analytic shift within 0.1 px; a moved body's pixels carry its move;
  validation errors 0. F2P: no motion-vector target. P2P: render lib green;
  golden unchanged. needs: LF3.
- [ ] LF5b render: TAA slice 2, jitter — Halton(2,3) sub-pixel offsets on the Progress 2026-10-17: the pure half is in: `post_jitter_motion.rs` `halton`, `jitter_offset`, `jitter_ndc`, `jitter_projection` (`jitter_matches_halton_values`, frames inside (-0.5, 0.5), f737386). Left: the presenter flag and the camera-uniform hunk; nothing outside tests calls `jitter_projection`.
  projection, 8-frame cycle, behind a presenter flag that stays off until
  LF5c lands; motion vectors stay un-jittered. Diff: regular. Tier A. Size S.
  Owns the camera uniform jitter hunk. Accept: the 8 offsets match Halton(2,3)
  inside (-0.5, 0.5) px, x2 equal; flag off leaves frames byte-equal to
  before. F2P: no jitter. P2P: render lib green. needs: LF5.
- [ ] LF5c render: TAA slice 3, resolve — a history target, reprojection by
  the motion target, a 3x3 YCoCg neighbourhood box clamp and a 0.1 blend,
  history reset on resize and camera cut; port the RP1 resolve in `post.rs`
  (Bevy `taa.wgsl` when that is wrong; the `port-bevy-taa` packet). Diff:
  good. Tier A. Size M. Owns the present/** TAA resolve hunk and its shader.
  Accept: static camera, two frames: sand-edge luminance flicker drops at
  least 50% vs off; a running fighter's ghost stays under 3 px; validation
  errors 0. F2P: no resolve pass. P2P: render lib green. needs: LF5b.
- [ ] LF5d render: TAA slice 4, sharpen and choose — an RCAS pass after the
  resolve (Bevy `robust_contrast_adaptive_sharpening.wesl`, `render-steals.md`
  item 11), then keep TAA or EG13's MSAA by the calmer capture (sand-edge
  frame-to-frame diff) as the presenter default. Diff: regular. Tier A. Size
  S. Owns the RCAS hunk and the AA default. Accept: RCAS raises a fixture
  edge's contrast with no pixel above the input's max; the chosen mode's
  flicker number in latest.md is the lower of the two. F2P: no sharpen pass.
  P2P: render lib green. needs: LF5c, EG13.
- [ ] LF6 render: dust and sparks from `particles_gpu.rs` — the seeded pool
  exists; the window never spawns. Dust at footfalls and knockdowns, sparks
  on `Blocked` and `Parried` cues (seeded from the cue tick, view only), drawn
  after forward. Diff: good. Tier A. Size M. Owns `crates/game/src/fighter_look.rs`
  spawn hunk, present/** particles hunk. Accept: a capture after a block shows
  spark pixels near the blade; particles above 0 ms; one tape twice gives
  equal particle counts per frame. F2P: zero particles on live frames. P2P:
  `cargo test -p engine-game --features app --lib` and render lib green;
  golden unchanged. needs: LF2.
- [ ] LF7 render: blood on the sand from `decals.rs` — deferred blood decals
  with depth reject exist; spawn one per `Hit` cue within `TINT_REACH_M` at the
  victim's feet, a ring of 64, view only. Diff: good. Tier A. Size M. Owns the
  fighter_look decal hunk, present/** decal hunk. Accept: a capture after 10
  hits shows decal pixels under the victims and none before; decals above 0
  ms. F2P: no decal on live frames. P2P: as LF6. needs: LF3.
- [ ] LF8 render: skin, metal and cloth shade apart — one BRDF serves every
  part. A `MaterialKind` (skin, metal, cloth, stone) per part from the glTF
  material name; in `fs` metal gets anisotropic GGX (Filament `surface_brdf.fs`
  `D_GGX_Anisotropic`, `V_SmithGGXCorrelated_Anisotropic`), cloth Charlie sheen
  (`D_Charlie`, `V_Neubelt`), skin wrap diffuse plus thickness transmittance
  (`surface_shading_model_subsurface.fs`). Three landings. Diff: good. Tier A.
  Size M. Owns forward shader BRDF hunks, the render material kind field, the
  fighter_look kind map. Accept: per kind the part's region differs on vs off;
  stone unchanged. F2P: no kind field. P2P: render lib green. needs: LF2.
- [x] LF9 anim: inertialized clip switches — after EG18's crossfade, a sw Landed aa42e4d.itch
  stores per-joint offsets (source minus destination, shortest-arc for
  rotations) that a critically damped spring decays (orangeduck Spring-It-On
  `common.h` `halflife_to_damping`, `critical_spring_damper_exact`,
  `fast_negexp`; `inertialization.c` `inertialize_transition`,
  `inertialize_update`). View only; floats never reach the hash. Diff:
  regular. Tier A. Size M. Owns new `crates/anim/src/inertial.rs`, the
  fighter_look switch hunk. Accept: `cargo test -p engine-anim --lib inertial`
  x2: the first frame after a switch equals the old pose within 1e-5; after 5
  halflives the new clip within 1e-3; 60 Hz and 30 Hz stepping agree within
  1e-4; the product check's `arena view:` pops drop to 0 at attack and idle
  switches. F2P: no inertial module. P2P: anim lib and game `--features app` lib
  green; golden unchanged. needs: EG18.
- [x] LF10 game: poses hold through the sim's hitstop — combat sets Landed b34c6f8.
  `state.hitstop = tables.hitstop_ticks` on impact (`crates/combat/src/fight.rs`,
  about line 912) and freezes the fight, but the look's clip clocks keep
  running, so no hit lands visibly. Expose the count read-only on the app;
  hold the attacker's and victim's clip clocks while it is above 0. Diff:
  regular. Tier A. Size S. Owns a `crates/game/src/lib.rs` accessor and the
  fighter_look clock hunk. Accept: `cargo test -p engine-game --features app
  --lib hitstop_hold`: during hitstop the two slots' phases stay put while the
  others advance; afterwards all advance. F2P: phases advance through hitstop
  today. P2P: game lib green; golden unchanged. needs: none.
- [x] LF11 game: impact shake — one trauma value (add 0.4 per `Hit` within Landed c7e6c66.
  reach, 0.7 per `Knockdown`, clamp 1, decay 1.0 per s) offsets the fight
  camera eye and target by trauma squared times tick-seeded noise (at most
  0.12 m and 2 degrees roll), view only. Reuse `crates/game/src/camera.rs`
  (`shake_offset` at line 761: permille trauma, squared curve, tick-hashed
  direction, already tested) which today only `run_windowed` reaches; the
  arena fight camera never calls it (Bevy `2d_screen_shake.rs` model,
  `anim-feel-steals.md` item 3). Diff: regular. Tier A. Size S. Owns the
  fighter_look camera hunk. Accept: one hit gives a nonzero offset that is
  zero again within 1.0 s; the same ticks give the same offsets; the product
  check keeps framing empty 0. F2P: no shake. P2P: game lib green. needs: none.
- [ ] LF12 anim: feet planted and the sword in hand — two-bone IK (ozz Progress 2026-10-17 (3065d34): feet half landed — `solve_limb` two-bone IK plus planted feet in the fighter look; shoved-guard slide 10.3 cm to 0.0, duel p95 1.0 to 0.7 cm; left: the weapon hand (no grip socket yet).
  `ik_two_bone_job.h`, orangeduck Motion-Matching `controller.cpp`
  `ik_two_bone`): a foot slower than 0.15 m/s locks to its contact point, the
  pelvis drops by the largest gap; the weapon hand reaches the grip socket.
  View only. Two landings. Diff: good. Tier A. Size M. Owns new
  `crates/anim/src/ik.rs`, the fighter_look pose hunk. Accept: `ik` tests (a
  target at 0.9 chain length is reached within 1e-3, the pole side is kept)
  and an arena receipt `foot_slide_cm` p95 under 2 during plants. F2P: no IK
  in the window. P2P: anim and game libs green. needs: EG18.
- [ ] LF13 anim: heads and chests track the rival — look-at on the head
  (weight 1.0) and chest (0.5), clamped to 70 degrees yaw and 30 pitch
  (orangeduck `ik_look_at`, ozz `ik_aim_job.h`). Diff: regular. Tier A. Size S.
  Owns a look-at fn in `crates/anim/src/ik.rs`, the fighter_look hunk. Accept:
  a target 90 degrees right at weight 1 turns the head to the clamp; weight
  0.5 halves it. F2P: none. P2P: anim lib green. needs: LF12.
- [ ] LF14 game: weapon trails — a ring of the last 12 frames of blade base Progress 2026-10-17: the ring is in (`weapon_trail.rs` `TrailRing` 12 pairs, `trail_ring_holds_12_pairs_and_drops_idle`, fed by `start_swing` in fighter_look.rs, c7e6c66) and `draw.rs` `trail_strip_verts` builds the age-faded strip (`trail_strip_fades_alpha_by_age_and_skips_idle`). Left: the present/** strip draw, nothing calls `trail_strip_verts` outside its test, and the mid-swing capture.
  and tip per attacking slot, drawn as a triangle strip with alpha by age,
  view only. Diff: regular. Tier A. Size S-M. Owns the fighter_look trail hunk,
  a present/** strip draw. Accept: a capture mid-swing shows the strip; idle
  slots draw none. F2P: no trail. P2P: game and render libs green. needs: none.
- [ ] LF15 playtest: FLIP beside EG30's SSIM — NVlabs FLIP (HPG 2020,
  github.com/NVlabs/flip; no weights, deterministic): CSF-filtered colour
  difference plus edge and point feature difference, the mean as an integer
  x10000 in `crates/playtest/src/look.rs`. The product check then scores each
  run's middle capture against the last green run's and files
  `bug:product-look` above a floor. Diff: good. Tier A. Size M. Owns the
  `look.rs` FLIP fns. Accept: `look_metric` tests: identical frames 0, a 1 px
  shift small, a 20% brighter sand region larger than the shift, x2 equal.
  F2P: none. P2P: playtest lib green. needs: EG30.
- [x] LF16 endpoint: the playtest verbs other engines' doors have — Landed 2124db13.
  `play.step_until` (a condition over `sim.observe` fields, tick cap),
  `input.inject` (a named action held N ticks), `state.digest` (hash of the
  live bout), beside EG16's `play.step` (satelliteoflove godot-mcp
  `godot_game_time`, `godot_input`, `godot_runtime_state`; `ai-native.md` verb
  gap). Diff: good. Tier A. Size M. Owns the endpoint play pack hunk. Accept:
  door tests x2: step_until `hp[1] < 900` stops on the first such tick; inject
  `attack` for 3 ticks moves the bout like the same FighterInput; digest
  equals the replayed hash. F2P: the verbs are unknown. P2P: endpoint lib
  green; golden unchanged. needs: EG16.

### View truth: the engine knows its frame as numbers (owner 2026-10-11)

Owner 2026-10-11: "i want ai know blindly fast af whats in scene ... better than
screenshot 100x". 0948ba6f landed the first slice: `crates/game/src/view_truth.rs`
turns the CPU-skinned bodies into numbers, `arena view:` prints the run's laws
(pops, sinking, NaN, error material, camera in a body or under the sand, the sim
ticks shown), `--view-brief` writes one JSON line per frame (about 2 KB against a
1-2 MB PNG), and the product check scores seven view floors. These rows finish
it: no window, the door, the GPU's own numbers, props and light, questions. A
model reads numbers first; PNGs stay only for taste (LJ1, EG30, LF15).

- [x] SP61 game: the view truth with no GPU — split `FighterLook::stage_bodies` Landed 1d01a41.
  into a CPU pose step (clip sample, skin, `BodyTruth`) and the upload, so
  `FighterLook::load`, `observe_tick` and a new `pose_frame(fighters, alpha)`
  run with no `Presenter`; add a joints-only mode (bounds from joints padded by
  a per-rig radius, no vertex skin). A headless `view_run(seed, ticks,
  fighters)` steps the bout and folds `ViewLaws` per tick. Diff: good. Tier A.
  Size M. Owns the `crates/game/src/fighter_look.rs` pose split and
  `crates/game/src/view_truth.rs`. Accept: `cargo test -p engine-game
  --features app --lib view_run` x2: a seeded 600-tick 8-fighter run prints the
  same `arena view:` line twice with no GPU; joints-only runs at least 2000
  frames/s on the reference CPU and its bounds hold the vertex bounds within 10
  cm. F2P: the look needs a Presenter today. P2P: game lib green; golden
  unchanged. needs: none.
- [ ] SP62 endpoint: `view.observe` and `view.laws` — the door reads the live
  view as `sim.observe` reads the bout: camera, each drawn body (act, clip,
  tint, root, bounds, screen rect, cover) and the laws so far, from SP61 over
  the door's play session; refusals as `sim.observe` (`no_live_bout`). Diff:
  good. Tier A. Size M. Owns the `crates/endpoint/src/packs/view.rs` verb hunk.
  Accept: door tests x2: after `sim.step` 120 ticks `view.observe` names every
  live fighter with a finite rect or `null`; one tape twice gives the same
  bytes; `view.laws` pops equal SP61's line. F2P: the verbs are unknown. P2P:
  endpoint lib green; golden unchanged. needs: SP61, EG16. Progress 2026-10-19: door half landed 1c70578 - `view.observe` and `view.laws` register as observe verbs in the view pack: they read the play slot's live bout (seed, headcount, tick count) like `sim.observe`, answer `no_live_bout` first, `unsupported_view_source` until a host registers, and `bad_view` for a frame outside the contract (one body per live fighter, finite rect or null, ordered bounds, camera in range); tests view_observe_names_every_live_fighter_after_120_ticks, view_observe_one_tape_twice_is_byte_identical, view_laws_pops_the_sp61_line, view_laws_at_tick_zero_pops_the_empty_line, view_refuses_without_a_bout_then_without_a_source, view_refuses_a_host_frame_outside_the_contract, view_tools_register_in_the_pack_and_api_describe_reads_them green (endpoint lib 636 passed, gh run 39184597714); tool-list budget 59000 -> 61500 with the dated reason (loaded list 58703). Left: a shipped host that calls `register_view_source` from SP61 - `engine-game`'s view truth sits behind its `app` feature and the endpoint `mcp` graph enables only `default` (no engine-render dep), so enabling `app` there would pull winit/cpal and break the Linux Pages doc build; the host needs a row that owns an app-side registration.
- [x] SP63 render: the GPU's own numbers instead of PNGs, slice 1: a draw-id Landed 1d01a41. frame.rs id_tests id_target_box_readback_holds_box_id_and_clear (box pixels hold id 0, plane id 1, background u32::MAX, read-back x2 equal, 0 validation errors), id_target_cleared_to_max, id_target_reports_box_id, id_target_stays_clear_on_offscreen green on the RTX 3050 (engine-render --features window --lib, local PC run); forward.wgsl writes the id attachment.
  target — the forward pass writes a second attachment `R32Uint` holding the
  draw's index in the list (fighter parts tagged with their slot), cleared to
  `u32::MAX`; nothing reads it yet. Slices b and c reduce it to per-body
  numbers and score them. Diff: good. Tier A. Size M. Owns the present/** id
  target hunk and the forward.wgsl id output. Accept: offscreen fixture: the
  read-back id target holds the box's id on its pixels and the clear value
  elsewhere; x2 equal; validation errors 0. F2P: no id target. P2P: render lib
  green; golden unchanged. needs: EG2.
- [ ] SP63b render: GPU numbers slice 2, the reduction — one compute pass over
  the id and HDR targets: per id (up to 256) pixels covered, summed linear
  RGB, max luminance and NaN or Inf pixels, into a storage buffer copied to a
  ring of 3 readback buffers mapped two frames later (no stall, EG2), at most
  1 KB a frame. Diff: good. Tier A. Size M. Owns the present/** reduction hunk
  and its WGSL. Accept: on a fixture the counts equal a CPU count over the
  read-back targets exactly; a planted NaN material gives `nan_px` above 0;
  one frame x2 equal; the ring never blocks. F2P: no reduction. P2P: render
  lib green. needs: SP63.
- [ ] SP63c game: GPU numbers slice 3, the receipt and the check — arena
  prints `arena pixels:` (bodies with 0 px while their CPU cover is over 0.5%,
  `nan_px`, darkest and brightest body mean luminance) and the view brief
  gains `px` and `mean_rgb` per body; `tools/product-check.mjs` adds floors
  (missing bodies 0, `nan_px` 0, body mean luminance within 0.01-50). After a
  green week the PNG blank and frozen floors go. Diff: regular. Tier A. Size
  S-M. Owns `crates/game/src/view_truth.rs`, the arena receipt hunk,
  `tools/product-check.mjs` and `tools/product-bar.json`. Accept: the
  product-check tests parse the line and break each floor once; a real run is
  green; on a 2-fighter fixture each body's px is within 15% of the CPU
  projection's area. F2P: no `arena pixels:` line. P2P: game lib and `node
  --test tools/product-check.test.mjs` green. needs: SP63b.
- [x] SP64 game: props and light in the view truth — every arena-kit and ring Landed 1d01a41.
  draw adds a row (kind, bounds, screen rect, cover); the frame adds sun
  direction and elevation, exposure, fog density and the passes that ran
  (`arena per-pass` ms above 0). Laws: sun below the horizon, exposure outside
  0.25-4, a fighter inside a prop's bounds. Diff: regular. Tier A. Size S-M.
  Owns `crates/game/src/view_truth.rs`, bounds accessors in `arena_kit.rs` and
  `fight_ring.rs`. Accept: a 10-frame brief lists every kit draw once per
  frame, x2 equal; a fixture fighter inside a pillar breaks the law. F2P:
  props are not in the brief. P2P: game lib green. needs: none.
- [ ] SP65 game: ask the frame — `view_pick(x, y)` (NDC) returns the nearest
  body slot, part and world point by a CPU ray against the posed triangles
  (kit bounds next); `view_sees(a, b)` says whether fighter a's head sees
  fighter b's chest past other bodies and props. EV21's pick manifest consumes
  it. Diff: good. Tier A. Size M. Owns a `pick` hunk in
  `crates/game/src/view_truth.rs`. Accept: 100 seeded picks on a fixture frame
  name the body the analytic projection names (100/100), x2 equal; a body
  stepped between two fighters flips `view_sees`. F2P: none. P2P: game lib
  green. needs: SP61.
- [ ] LF17 game: the dead lie on the sand — the death clip sinks bodies 12-15 Progress 2026-10-17: b34c6f8 rests dead bodies on the sand (fighter_look.rs `sand_lift_rests_the_lowest_vertex_and_never_pushes_down`, `view_truth_down_lift_rests_the_fallen_start_pose_at_once`, `product_sunk_56cm_down_rest_reports_green`) and `tools/product-bar.json` `view_sunk_cm_max` is already 3. Left: the `sunk_cm_max` under 2 run on a fresh arena build (the bar note still says living bodies dip 5-14 cm at crossfades; arena.exe STALE).
  cm into the sand (`arena view:` `sunk_cm_max`, 0948ba6f). Lift a `down`
  body by its lowest vertex below the sand, eased over the fall, view only.
  Diff: regular. Tier A. Size S. Owns the fighter_look down-pose hunk. Accept:
  the product check prints `sunk_cm_max` under 2 and the same commit tightens
  `tools/product-bar.json` `view_sunk_cm_max` to 3; a fixture down body's
  lowest vertex sits within 1 cm of the sand. F2P: 12-15 cm today. P2P: game
  lib green; golden unchanged. needs: none.

### Operator (folded 2026-10-16 from `SPRINT-18-operator.md`, file removed)

- [ ] O10 Optional: name a second, stronger RT GPU for the Ultra tier;
  until then Ultra stays unverified.
