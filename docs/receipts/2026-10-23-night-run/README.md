# Overnight run 2026-10-22/23: 66 rows, one tree, proofs green

Scope: a lead session orchestrated every live OpenChamber lane on the shared
master tree from 18:30Z to 01:15Z, dispatched helpers for rows in clean crates,
verified each helper against its own Accept, and ticked the board itself. It also
audited commits against the board and found three rows whose code had fully
landed but was never ticked (GEO9a, BX11, and the earlier RY3 class).
State at 01:20Z: HEAD `a3b947a1d`, 160+ commits since 18:30Z, 50 rows ticked in
the SPRINT files, golden `d57b5c8ae3096614` unmoved. Both identity proofs were
re-run at that HEAD: `proof_cli` 13/13 (run 39881639832) and the arena matrix
8/8 on both pins `0x605f6503a39082f4` (run 39881976844).

State at 06:30Z: 64 rows ticked in the SPRINT files, golden
`d57b5c8ae3096614` unmoved. The golden was re-proven at `55d519ac4` on a clean
snapshot (`--only crates/core/src/lib.rs` sends no overlay file, so the tree is
exactly HEAD; run 39909016834: `final_hash` d57b5c8ae3096614,
`golden_checked` true). The later window added fourteen rows, listed below.
Three standing tool reds were fixed on the way: the wasm gate's 7 new fs
hazards in the native manifest IO, the assets import-lane clippy lints, and the
VS3 slice case that pinned the open checkbox. PQ3's leaked-soak Accept test is
green too (`cargo test -p engine-proof --lib soak` 18 passed, 39911391845), but
the row stays open: it needs GM3 and the board audit refuses a tick over an
open need.

## Proofs seen tonight (run ids on the public mirror)

| Check | Result |
|---|---|
| `cargo test -p engine2040 --release --test proof_cli` | 13 passed, golden `d57b5c8ae3096614` (run 39870971670) |
| `cargo test -p engine2040 --release --test proof_cli` (re-run at HEAD) | 13 passed, golden `d57b5c8ae3096614` (run 39881639832) |
| `cargo test -p engine-proof --lib --features arena,spine arena_matrix` (re-run at HEAD) | 8 passed, both pins `0x605f6503a39082f4` (run 39881976844) |
| `cargo test -p engine-proof --lib --features arena,spine arena_matrix` | 8 passed, both pins `0x605f6503a39082f4` (run 39871955371) |
| `cargo check --workspace --all-targets` | PASS (run 39872346123) |
| `cargo check -p engine-game --features app --all-targets` | PASS on Windows (run 39876193696) |
| `cargo test -p engine-web --lib` | 182 passed (run 39873662176) |
| `cargo clippy -p engine-endpoint --features mcp --all-targets -- -D warnings` | PASS (run 39869827231) |
| `cargo test -p engine-net --lib` | 462 passed, 4 ignored (run 39869186390) |
| `cargo test -p engine-net --lib live_snapshot -- --nocapture` | 2 passed; siege `kbps_x100=4221`, live `kbps_x100=4703`, starve 24 (run 39876980369) |
| `cargo test -p engine-proof --lib chaos::tests::shap` | 4 passed (run 39868576928) |
| `cargo test -p engine-proof --lib shaped_duel` | 1 passed at HEAD (run 39878868607) |
| `cargo test -p engine-geo --features fbx --test fbx` | 4 passed, 1000 mutations 949 refused / 0 panics (run 39877431720) |
| `cargo clippy -p engine-geo --features fbx --all-targets -- -D warnings` | PASS (run 39877785835) |
| `cargo test -p engine-nav --test it` | 11 passed (run 39878399285) |
| `cargo test -p engine-audio --lib strike_sound` | 6 passed (run 39879426724) |
| `cargo clippy -p engine-audio --all-targets -- -D warnings` | PASS (run 39879306285) |
| `cargo test -p engine2040 --release --test proof_cli default_and_max_seed` (HEAD 55d519ac4, clean snapshot) | 1 passed, `final_hash` d57b5c8ae3096614, `golden_checked` true (run 39909016834) |
| `cargo test -p engine-net --lib interp` (HEAD) | 19 passed x2 (runs 39908343424, 39908530602) |
| `cargo test -p engine-anim --lib pack_file` (HEAD) | 12 passed x2 (runs 39907704138, 39907851439) |
| `cargo test -p engine-anim --lib look_at` (HEAD) | 6 passed (run 39907521078) |
| `cargo test -p engine-endpoint --lib --features mcp receipt_digests` | 1 passed (run 39906794354) |
| `cargo clippy -p engine-endpoint --features mcp --all-targets -- -D warnings` | PASS (run 39906794278) |

## Rows ticked in the window

AD7 AD14 AD16 AGT03 AGT04 AGT05 AM18 AM25 BX11 DOOR1 ENG1 ENG5 ENG6 ENG9 ENG16
ENG17 ENG19 GA5 GEO9a GS2 HC1 HC2 HC3 HC4 HC5 HC6 HC7 HC9 LEG10 LF2 LM6 NV5 NV8 ON29
PF27 PW14 QL8 SD1 SD2 SD3 SD4 SD5 SD6 SD7 TC4 VS3 WEB16 WEB17

Rows ticked in the later window (01:30Z-06:30Z), fourteen:

AD2 AD22 AGT01 AM21 AM29 CT12 DX7 FEEL6 FEEL10 GA9 GA14 LF13 LF14 MP25

- GA9 `2780bbe6a`: 500 shield-wall west vs 500 wedge east; the flanked west wall
  breaks first at tick 393 (seed 2040) and 395 (seed 2041), east never breaks,
  the no-flank control stands; orders 1 tick against the 2-tick bound; the
  mid-battle rout leg rebakes 1 tile and repaths 32 stale walks; legion 24
  passed / 1 ignored (39903364048), clippy (39903363328), golden 13/13
  (39903512834).
- GA14 `315044344`: `sweep_` 7 passed / 1 ignored x2 (39910287921, 39910654715)
  and `sweep_noise` 1 passed x2 (39910272285, 39910654917); the perfect-player
  curve keeps the P3 band 490/482/472 permille, the seeded human tape (2-4 tick
  delay, one action per 8 ticks) makes rung strength rise strictly T1 138 <
  T2 374 < T3 386 half points of 2 x 200, and both curves replay
  byte-identically; noise defaults off. Full lib 482 passed (39911146924),
  clippy PASS (39911668870). Design note: under noise T1 is easier (655
  permille) and T2/T3 near-lockouts (65/35) - smoothing that curve is a
  design-seat row, not a bug in this Accept.
- AM21 `af60a32fe`: `WarpWindow` pin; pinned landing 0 mm worst against the
  10 mm bar over 64 fighters x 4 tier lanes x 8 phases, control 1853.4 mm and
  17 held-root landings; replay `0xc1fed095b504a5ad` x2, seed 2041 differs;
  lod_warp 2/2, golden 13/13 (39902345065).
- CT12 `51ec5cf29` + `42f1ddc24`: seeded manifest beside the frozen cook owners;
  20/20 fixture rebuilds byte-equal (digest 82e66330f0ede22b...), 10/10 edits
  invalidate exactly the transitive closure, round-trip 13/13 and bin 3/3;
  golden 13/13 (39903948299). Park cleared; DT5 remains the consumer.
- AD2 `a08254413`: `TxApplyOutput` carries before/staged/after 64-hex digests
  read off the landed `Applied`; the receipt replay test is green, editor
  landed_digest 6/6 (39902687278), endpoint clippy `--features mcp` PASS
  (39904044483). The audit-head hex now goes through one encoder
  (`play::hex_bytes`) after the ratchet flagged the duplicate.
- DX7 `95496392c`: `fracture_tool.rs` paints bonds through `Op::SetBond` +
  `commands::apply` with undo, previews a break without writing, refuses a
  replay past the 600-tick cap; 9 fracture tests (39903893698), editor 293
  passed / 3 ignored (39904200572), clippy (39904200443).
- FEEL6 (sim Accept): the clash test holds both blades in recoil for exactly
  `0..CLASH_RECOIL_TICKS` and fires one Clash event (39906309942); the wire tape
  mirrors and replays (39906426534). Live-look leg named.
- FEEL10 (law Accept): `cargo test -p engine-physics --lib push` 7 passed
  (39906309439) - conserved momentum, rest stays rest, mirrored shoves mirror.
  V3 and live-clinch legs named.
- LF14 `14e05190d` + `70c968805`: per-slot trail staging, a vertex-count guard
  in `trail.wgsl`, the pass encoded after particles, an idle clear; 14 trail
  tests and a device draw with 0 validation errors; capture
  `target/clips/lf14/frame_000110.png` shows the ribbon on one fighter and none
  on the other three. The three ratchet growths were accepted with reasons
  (CQ9 owes the split).
- AD22 `032c05b7d`: `playable` is six named gates (compile, play with 0
  errors, save then reopen, package, packaged launch, packaged smoke); `node
  --test tools/proof.test.mjs` 51 passed x2, every line names its gate, a
  `not_wired` or stale record FAILs and a green record cannot fill a missing
  gate; `tools/ship.mjs` prints the verdict after `pack` and a full `run`. Open
  leg named: no producer writes the compile/play/save-reopen records yet, so a
  real GM3 ship ends `RESULT FAIL (playable)` - honest, and the next consumer
  step.
- AGT01 `a3fa25bec`: `experimental.subagent_depth: 1` in `opencode.jsonc`, so
  helpers send none; config state at HEAD (the debug config dump cannot run
  under this harness).
- LF13 (look_at Accept): `engine_anim::ik::look_at` + `AimLimits` with 6 tests
  (39907521078) - 90 degrees at weight 1 reaches the 70 degree yaw clamp, weight
  0.5 halves it, straight up clamps at 30 pitch; `a0e61157d` took the test trig
  out. The `fighter_look.rs` hunk (head 1.0, chest 0.5) is the named open leg;
  AM24 held that file at the time.
- AM29 `f6ef1a686`: `pack_file` 12 passed x2 (39907704138, 39907851439) - the
  committed `stagger.am5p` loads from disk, carries every fighter joint, samples
  onto the rig, refuses another rig by name and keeps the slip under 0.08; the
  four combat clips also play on the CT7 body. The game-side attack-slot swap
  stays quarantined (`C:\_quarantine\2026-10-22\engine2040-combat-packs\`) and
  is the named open leg.
- MP25 (interp Accept): `Correction` in `crates/net/src/interp.rs`, 19 passed x2
  (39908343424, 39908530602) - first-frame step under 6 percent, under 2.5
  percent left after 60 frames, authoritative digest equal with correction on
  and off. The drawn half (`netgame.rs` note plus the window offset) is the
  named open leg.
- AM21 also fixed the source scans: `254811dfb` marks the warp test file as test
  code at the top and `4acf16b22` records the committed count 929927.

Notable landings with their commits and one-line proofs:

- ENG1 `2bf146d5a`: backend probe before the first frame; browser leg in headless
  Chrome (WebGL2 fallback prints its reason), 5 lib/IT tests.
- ENG5 `6330e67ba`: `perf.doctor`, five checks one line each with a fix, each
  check broken once by a test (7 passed).
- ENG6 `325a54fa6` + `6b58f4792`: `net.shape`; a 120 ms / 5% loss scripted duel
  folds the same tape, tip and sim-only digest as a clean link, twice
  (`17089590214351568806`), 46/1000 seeded drops.
- ENG17 `f858555f2` + `bf1dd1ada`: page audio on an AudioWorklet fed by the
  mixer; headless Chrome prints `audio: state=running underruns=0`, 6/6 cues on
  both the worklet and the forced-404 fallback; the fallback prints its reason.
- ENG19 `85d5d5d26` + `88ca9b8dc`: the Windows ship check starts the shipped
  binary, reads `tick=600 hash=d57b5c8ae3096614` (mirror run 39865729635).
- VS3 `0050d23f6`: the arena kit binds each file's glTF base-color image through
  VS2; the frame-60 sand region of the kept pair reads luminance variance 14.810
  against 0.881 untextured (16.8x, bar 4x; lead re-measured), and a named image
  the import does not carry refuses by name instead of falling back to white.
  52 arena_kit tests.
- LM6 `7e9a144b9`, TC4 `b0fde676e` (ORCA opt-in leg named), ON29 `b8876e998`
  (siege and ON3 arms oracle-equal x2, starve 24 ticks = 400 ms, 47.03 kbps).
- GEO9a `4eab39f9a` (landed 2026-10-20, never ticked; found by the commit audit):
  FBX import behind `fbx = ["dep:ufbx"]`; 2 meshes at Blender's own triangle
  counts, cube bounds inside 1e-4 of 1 m, digests equal x2, 1,000 seeded byte
  mutations gave 949 typed refusals and 0 panics in 88 ms (runs 39877431720,
  39877785835). Fixture is 31 KB.
- NV8 `a07afb0fc`: nav soak harness — 500 agents at 0.54 ms median / 0.79 ms p95
  on this PC (bar 1 ms), 0 stuck over seeds 0..999 at 8 agents and 1,200 ticks,
  and a per-tick trace hash equal x2 with a different seed that differs (runs
  39878187813, 39878189199, 39878399285).
- BX11 (audio, landed earlier, never ticked): the impact table is a dense
  8-kind x 6-surface matrix, `strike_sound.rs` holds the sever/landing/groan
  tables, every fight event maps to its own cue, and an event with no take
  queues nothing (runs 39879135878, 39879119898, 39879305423, 39879426724;
  clippy 39879306285).
- AM18 `a6a2d197d`: the 1,000-agent p99 is measured, not modeled — the lead
  re-ran the gate at HEAD and read p50 68,100 ns, p99 82,400 ns, max 194,100 ns
  against the 2 ms bar (24x under, seed 2040, 256 warmup + 2,048 timed ticks,
  release on the i5-13420H); the 8-nearest test is green on the same tape, the
  golden is 13/13 at HEAD (run 39880101869), and the new `crowd_scale_1000` PF
  workload runs 600 ticks at digest `11495894263503076227` with 0 allocations
  (run 39880084484). The row names two open legs: the budget covers the crowd
  LOD tick, not 1,000 full pose evals (the pose table caps at 64), and the p99
  receipt is local because CI is not the reference CPU.
- GA5 `a3b947a1d`: 500 soldiers at idle median 3.4 us / p95 8.6 us and moving
  median 12.8 us / p95 12.9 us (lead's release re-run at HEAD) against the
  1500 us/step GA2 bar; 0 stuck over seeds 2040..2043 x 600 ticks with a
  frozen-body control that fires at tick 16; seed 2040 final state
  `f0b920a98ca848b7` and trace `4ef34effd335a93a` x2, seed 2041 differs; the
  thinking LOD is outcome-exact per tick (77,100 decides vs 300,000, 3.9x fewer)
  and the perception clock keeps every squad inside its 1/2/4/8 wave and the
  8-tick cap (runs 39881640190, 39881032310, 39880948480, 39881202212).
- Arena digest pin moved once, with its own receipt
  `docs/receipts/2026-10-23-arena-digest-sim-day/` and `ccf4abe16`: the sim day
  of FEEL2/4/6/10, the Fixed rounding and the physics laws moved the arena line;
  the golden is a different identity and did not move.
- Tree hygiene during the run: `98bb6cea5` (the wasm-only seat test no longer
  breaks the native web lib), `94d6f4e16` (two 16 KB futures boxed, so endpoint
  all-targets clippy is green), `bf1dd1ada` (backend-choice test passes
  `-D warnings`), `62c0f45d8` (manifest IO is native-only, so wasm-gate drops
  7 fs hazards and the import-lane clippy lints are gone; wasm-gate 20 pass,
  assets lib 384 pass with `--features import`), `76b676345` and `db58a1411`
  (the combat-clips test carries the same import gate as its siblings; the VS3
  slice case tracks the live checkbox instead of pinning it open).

## Open legs, named and never claimed

- ENG20 (B9): the TAA registration test and the deterministic ghosting metric
  landed (`846aa1cf0`); the row is not ticked because the metric has not been
  shown to separate two real captured sequences. The render path has no TAA
  on/off switch at HEAD, so the pair needs a render-side flag (or the web
  quality tier) plus two 70-frame captures.
- ENG19: the shipped binary has no `--headless` route in `crates/game`; the
  receipt's `SHIP-GAME skipped adapter=none reason=no-wgpu-adapter` is expected
  on the runner.
- TC4: ORCA is opt-in; arming it by default would move the pinned digest and
  needs its own receipt.
- GM3 (A5, A8): the 5-minute clean-folder run needs a quiet box (18,000 frames).
- Q1 (A4): the HDRI is vendored (`content/incoming/studio_small_09_1k.hdr` with
  its provenance sidecar); the release pilot capture set is the remaining Accept
  clause, and the row is still marked HELD.
- B8: the `view.compare` diff image (this run's last helper) and the 10-per-tier
  set remain; the VLM calibration is dropped by the lead's 2026-10-22 call.
- C10: none of `fx.ragdoll`, `fx.limb`, `fx.break`, `fx.blast` exists at this
  HEAD; the one-call physical effects are the physics lane's open work.
- FEEL6: the live duel still runs `match_live = false`; the held recoil needs the
  arena authority and the look a recoil flag; V2 snapshots fail closed.
- FEEL10: dormant until `FIGHT_RULES_V3` / `ArenaRules::V3`; walking speed is not
  in the push yet; the clinch pass is not in the live look.
- LF13: the `fighter_look.rs` hunk (head 1.0, chest 0.5) is not wired; AM24 held
  the file.
- AM29: the game-side attack-slot swap is quarantined; the loader Accept is met.
- MP25: the drawn correction (`netgame.rs` note plus the window offset) is not
  wired; the net half is proven.
- AD22: the six-gate verdict is proven, but no producer writes the compile, play
  or save-reopen records yet, so a real GM3 ship honestly ends `RESULT FAIL
  (playable)` until AD4-style producers land.
- PQ3: the leak analysis and its seeded-leak Accept are proven (`cargo test -p
  engine-proof --lib soak` 18 passed, 39911391845; a leak at minute 24 fails and
  names its minute within 2). The row is not ticked because it needs GM3, still
  open, and the board audit refuses a tick over an open need. The real one-hour
  packaged soak (8 fighters, AI fill, destruction on) is the remaining leg.
- GA14: the Accept curve is proven; under the pinned human tape T1 is easier
  than the perfect player (655 vs 490 permille) and T2/T3 near-lockouts (65/35
  permille). Smoothing that cliff is a design-seat row.
- Finish scoreboard holes: B has no row set, so it scores 0/0 forever. C2, C3,
  C5, P1 and P8 still name parked rows whose IDs are on neither the live board
  nor the archive (RT2-RT8, VG1-VG4, VX1-VX5, SL3, WD3, WD5), so those gates
  cannot close until the owner unparks or remaps them. `finish.test.mjs` passes
  but does not enforce resolution for fallback rows; the mapping is the owner's
  decision.
- Three helpers from the 05:20Z batch are still running at 06:30Z: AM24 (pops),
  FEEL5 (particles/decal) and AD10 (capability undo). Their landings are outside
  this receipt. AD22 and GA14 from the same batch have landed and are above.

## Fleet notes

- The editor fracture helper (DX7) wedged: its last assistant message never
  completed and it had written no code of its own. The `crates/editor` diff in
  the tree at that moment (556 lines of before/staged/after content digests) is
  another lane's work and was left untouched.
- Two research sessions had been busy and silent for about 2.5 h; a lead
  check-in ended both without new commits.
- VS3 and ENG17 landed and were ticked; the B8 diff-image helper landed
  `912a44723` and its progress line went into `docs/plan/finish-line.md`.
- A commit-to-board audit found the rows whose code had fully landed but was
  never ticked: GEO9a (`4eab39f9a`, 2026-10-20) and BX11 (audio) were re-proven
  from their own Accept and ticked. The audit also cleared SP66's spatial half
  (green) but left it for the door lane, whose `scene_tools.rs` /
  `spatial_checks.rs` edits were live at write time.
- Two clean-crate helpers were dispatched for rows in clean crates. AM18 landed
  `a6a2d197d` + `9a8b35c6a` (anim crowd p99, perf workload, baseline) and GA5
  landed `a3b947a1d` (legion thinking LOD and perception clock); both were
  ticked only after the lead re-ran their timing gates and the identity
  proofs at HEAD.

## Reds at write time (all foreign, none on committed HEAD)

- `rust-quality`: three ratchet regressions at write time, all in files other
  lanes are writing — engine-endpoint `dup_fns` 3→5 (the door verify work),
  engine-provider `dup_fns` 0→1, and `crates/editor/src/commands.rs` lines
  3441→3510 (the editor digest batch). The pre-commit hook measures only staged
  crate files, so doc commits pass.
- `cargo clippy -p engine-anim --all-targets -- -D warnings` was red at 01:25Z
  inside `crates/anim/src/ik.rs` (a committed foreign file): `f32::atan2`,
  `f32::asin` and an ignored-unit pattern. LF13's `a0e61157d` took the test trig
  out of that file; the line now PASSES at HEAD (run 39912682270).
- `workspace-check`: at 01:25Z the src-line baseline PASSES (the AM18 helper
  re-recorded it at 922736 in `9a8b35c6a`); the one red is a live-tree file the
  game lane has not declared yet — `crates/game/src/faction_match.rs` is
  untracked and no `mod` line names it, so it is never compiled. It is that
  lane's WIP, not a committed red, and it is not this run's path.
- `leftovers`: none idle for 3 h.
- At 06:30Z: `workspace-check` PASS with the baseline recorded at 930094; the
  PQ3 tick was refused by the board audit (needs GM3 open) and became a progress
  note instead. Two leftovers are now idle past 3 h and are named, not adopted:
  `dist/web` (4 paths, 9.4 h, a web build whose source commits are already on
  master; rebuild and commit it with the web build tool when a web lane is free)
  and `opencode.jsonc` (9.5 h, drops `zen-proxy-muse`/`zen-proxy-mimo` and
  `alibaba-router` from the enabled providers; a fleet config decision for the
  owner, never reverted here). The provider `dup_fns` 0 -> 1 is the one unchanged
  foreign ratchet red; helpers' WIP is not measured. The anim clippy line is
  green again (39912682270).
- Later state (05:35Z): `workspace-check` is green (baseline 929927, determinism
  and genre PASS). `rust-quality` is red on one foreign ratchet: engine-provider
  `dup_fns` 0 -> 1 (a provider body copied elsewhere). The engine-endpoint
  `dup_fns` 3 -> 4 that the AD2 commit added is fixed by routing both callers
  through `play::hex_bytes`. Helpers' WIP is not measured here.
- Later state (07:30Z): 66 rows ticked (AD10 at `1692654b7`, FEEL5 at
  `d15dd736d`). The endpoint door red is fixed at `32a0202ef`: the default list
  measures 60,036 chars (AD2 receipt digests, SP66 dark/unknown rows,
  view.compare diff image), so both bounds carry 60,200 with that dated reason,
  and `TOOL_LIST_BUDGET_CHARS` 73,000 -> 75,000 because the gym pack measures
  72,378 against the old 70,952 ceiling; `cargo test -p engine-endpoint
  --features mcp --lib` is 789 passed / 1 ignored (39915437595). The
  provider/perf clone pair is gone at `3c9edca48`: `engine_core::text::clip_bytes`
  is the one owner (core text 5 passed, 39916641475; provider lib 278 passed /
  6 ignored, 39916641313; perf crash 6 passed, 39916641203; clippy three crates
  PASS, 39916640860). FEEL5's three commits are ticked with the 38 + 104 PNG
  captures. Re-tasked warm sessions: AD10 -> AX23, GA9 -> VS4, CT12 -> CT14,
  DX7 -> NV14, GA14 -> S27, AD22 -> the two committed clippy reds (physics
  `or_fun_call`, render `stage` 8 args). `dist/web` stays blocked: the web build
  fails in the active `faction_match.rs` WIP (`game/src/faction_match.rs:127`,
  `:591`) and DX7 left the four paths byte-identical. The remaining
  `rust-quality` regressions are active helper WIP (assets 5, endpoint 2, game
  2) and each helper got its FAIL lines. Baseline 931,716 was recorded at
  `1692654b7`; `workspace-check` is re-run after the helpers land.

- Later state (10:40Z): 17 more rows are ticked since `bcaf4f3b3` (VS4, CT14,
  NV14, S27, BX9, BX25, EG11b, AX23, LF6, LF7, GS22, GS23, AM24, RZ4, QN9,
  TC5, FEEL9), so the run holds 83 ticks. The lead reopened each Accept before
  ticking. RZ4: `break_look` 12 passed on the runner (39935703930), the
  crate-whole / pieces-apart capture pair at ticks 2 and 238 under
  `target/clips/rz4c`, sim hash `ef25059b432c9193` equal with the picture on and
  off; the shard meshes, multi-chunk hits and the GPU dust emitter stay as named
  legs. FEEL9: 16 `ragdoll_look` tests, head 12.5 cm vs layer off then 0.000 m
  after the 18-tick fade, on/off hashes equal, window run `hits=27
  knockdowns=3 pops=0`; the wide capture caveat is in the row. GS23: 12 rows,
  24,945,995 B, CC0 + CC-BY with attribution, real fetch 12 OK. S27: the lead
  opened the 3 PNG (ticks 1/121/241), 2 staged fighters in motion, 0 red-box
  stand-ins, NOTES.md with the retired-Lead-2 dated evidence. TC5: 7 passed,
  256 discs through in 8476 ticks, peak 1216 N, 5 mm bar, 0.255 ms at 512,
  0 allocations, golden x2. AX23: door list 64,060 under the 75,000 budget.
  Golden at `d10b1c007`: PASS (run 39939140557, `d57b5c8ae3096614`).
  `product-check` now compares fps only inside one present mode (`0739112e9`,
  tool tests 15/15). Not ticked, with the gap named: EG47 is partial at
  `4a3404180` (the lead found four `eprintln!` + return skip sites still live
  at trail_draw_gpu.rs:586, sun_shadow.rs:682/1006 and backend.rs:1060; Review
  is fixing them); HS23's 20 harnesses are landed and typechecked but Kani has
  no CI job yet (`1a170461e`, the proof->destruction edge is documented and
  workspace-check passes); FEEL7's one-tick test is red because the look does
  not record a landing on the sim's landing tick (Gaps is diagnosing); B8 still
  needs real captures per tier (the recipe is a stand-in, and 10 painted
  variants would not close it); GM3 waits for a quiet box, not a 13-session
  tree.

## Later window, 2026-10-23 day (must list A1, F24, B8; EG26)

Every line below is presentation, tooling or docs: no sim code changed, so the
golden `d57b5c8ae3096614` is untouched by this window.

- A1/WEB5 `4008eec0c`: `engine-render` now has `gpu` (wgpu, pollster,
  engine-material, v1; compiles for wasm32) and `window = gpu + winit +
  windows` (surface, DXGI VRAM read). `cargo check -p engine-render --features
  gpu` PASS (run 39974107288); the same line for `wasm32-unknown-unknown` PASS
  on this PC (9m17s); `cargo test -p engine-render --features window --lib` 1348
  passed / 0 failed / 17 ignored (run 39975257672). A1 has no open leg.
- F24 `0e00a25ee`: the shipped arena kit applies `content/patch.e2patch` beside
  the kit root through `with_kit_source` -> `verify_base` -> `PatchedSource`; no
  shipped path is the plain directory read. `kit_source` 2 passed x2 (runs
  39977827001, 39979747819, 39980470549).
- EG26 bitmap leg `f2e79db9f`: the live overlay atlas is the CC0 O-023 5x7 font
  (96x48 RGBA8 built once, nearest sampling; cell 0 opaque white so solid HP
  bars and the console panel stay flat; glyph cells carry coverage). The table
  equals `engine_ui::text::ATLAS` on all 95 rows; `push_text` width is
  `height * advance_ratio()` and the console follows. Render lib 1349 passed
  (run 39984467748); game app lib check PASS (run 39983379289); the atlas read
  back at 6x shows every glyph. EG26's door leg landed in `3b905f6b3`:
  `console_bridge_reads_the_live_endpoint_slot` 1 passed (run 39985590644).
  EG26 closed at `fda112d5b`: `console_step` 3 passed at HEAD (run
  39986328116: one step one tick, `slow 0.5` one tick per two frames, hash at
  600 = `d57b5c8ae3096614`), and the presenter text capture passed on this PC
  with the software adapter (`E2040_ALLOW_SOFTWARE=1`, Microsoft Basic Render
  Driver; no hardware adapter in this headless session) - its PNG shows three
  readable `I` glyphs on the sand background, not blocks.
- B9/ENG20 closed at `1a6c38e99` (ENG20 row `4843a1df4`): the labeled TAA pair
  `target/captures/taa-pair/{off,on,registered}` is written by
  `crates/render/tests/it/taa_pair.rs` from the real resolve pass; `off` vs the
  frozen arm scores `separation 0.246` against the 0.05 bar (ghost_share 0.273),
  while `off` vs the registered arm scores 0 (bit-exact). Pair test 1 passed
  (run 39988079847), render lib 1349 passed (39984467748), golden
  `d57b5c8ae3096614` PASS (run 39988122554). CGVQM+D stays dropped by the
  lead's 2026-10-22 call.

## Re-run

```
node tools/gh-check.mjs cargo test -p engine2040 --release --test proof_cli
node tools/gh-check.mjs cargo test -p engine-proof --lib --features arena,spine arena_matrix
node tools/gh-check.mjs cargo check --workspace --all-targets
node tools/gh-check.mjs cargo test -p engine-web --lib
node tools/workspace-check.mjs
```
