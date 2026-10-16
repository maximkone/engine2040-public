# BLOCKERS-archive — resolved rows moved out of `docs/BLOCKERS.md`

Only resolved rows live here. Open blockers stay in `docs/BLOCKERS.md`;
git history keeps every move (`git log -p -- docs/BLOCKERS.md`).

## Resolved — 2026-10-09

- **GP18.b bench proof starved of build slots twice, GP18 stays OPEN on the bench half (owner: lead — re-proof in a quiet window).** Second identical failure: 653-gp18b (slot wait exit 124 x2) + 672-gp18b-reproof (slot wait exit 124 x2, feel.rs synced to master c2313fe, bench.rs +147 untouched in lane gp18). Code gates already green (packet-gate PASS, quality 95 A); only `cargo test -p engine-game --lib bench` + full lib green missing. Repro: dispatch any heavy cargo job while 40-agent waves hold all heavy.mjs slots. Fingerprint: gp18b-slot-timeout x2 (infra congestion, not a code defect). Next: re-proof gp18b alone in a narrow wave (no competing Rust builders); then REVIEW + tick GP18. `BLOCKED: GP18 tick until bench proof posts green.`
  Update 2026-10-09T21:13Z: proof posted green in a quiet slot via heavy.mjs (9 bench tests exit 0, packet-gate 1 path PASS, rust-quality game 96 A 0 regressions) and landed as 9e7c72fb (pushed 8792b8be..9e7c72fb, lane gp18b removed). GP18 stays OPEN for GP18c/d; this row closes when GP18 ticks.

Moved 2026-10-09 (speed-202610091753-f06): proof green + landed, so the slot-starvation block is resolved. GP18 tick stays on the board, not here.

## Moved 2026-10-10 (resolved or stale)

Moved verbatim from `docs/BLOCKERS.md`: 3 resolved (board rows ticked) and 9 stale (older than 2026-10-07, lanes gone). Each group keeps its original date section.

### Open — 2026-10-06

- **PX33 15%-faster settle premise unlandable on the velocity-override plant, twice (owner: planner/body seat — rewrite the row or split PX33.b).** Second identical result: builder-body-r12 PX33 PARTIAL + px33 PX33 PARTIAL, both PD law + clamp + dt-guard green (lib 3-5/5, integration 3-4/4, balance_matrix 4/4, golden `d57b5c8ae3096614` x2 unchanged, quality 99 A, packet-gate PASS) with the same measurement: P-only settles in 12-15/28 ticks vs any Kd worse-or-equal (Kd 2.0 never settles with 0.33 m static jitter; Kd 0.7 only 14 vs 15; P60 settles in 5 with no ring — first-order override, D structurally cannot speed it). Repro: damping sweeps (5 grids + trace) in either lane; headline test locks a 15% parity band + no-worse overshoot instead of 15%-faster. Fingerprint: px33-settle-unlandable x2 (override plant, D slows or ties). Next: never resend PX33 whole; planner rewrites the row (parity + overshoot + digest gates) or splits PX33.b (force-driven second-order servo where P rings and D genuinely wins); either lane's law hunks land under the rewritten row. `BLOCKED: PX33 15%-faster claim until the row is rewritten or PX33.b splits it.`

### Open — 2026-10-05

- **OPT6 nextest spike AFTER unmeasurable: `cargo nextest` binary absent
  (owner: center/owner — install `cargo-nextest` as portable
  `tools/bin/cargo-nextest.exe`, gitignored, or drop OPT6).** Second
  identical failure: speed-seat OPT6 spike wave-5 (observed-only BEFORE,
  no AFTER) + builder-speed-r6 OPT6 re-measure wave-7 (cold 18.32 s,
  warm 0.17 s cargo fallback, golden x2 identical — AFTER still
  unmeasurable, binary absent). Fingerprint: nextest-install-blocked x2
  in 24 h (same class as all "binary absent" infra blocks). Next: never
  resend the spike whole; the BEFORE numbers + golden x2 sit in
  `.lanes/opt6-speed/OPT6-spike-note.md`; install then re-measure, or
  the owner closes OPT6 as wont-install. Proof legs already fall back
  to `cargo test` with visible `via cargo` (475 landed), so nothing is
  red meanwhile. `BLOCKED: OPT6 AFTER (nextest binary absent x2).`

### Open — 2026-10-02

- **376-vs1-repair cancelled pre-start twice, never ran (owner:
  keeper infra or split acceptance).** Second identical failure:
  attempt 1 (packet 376-vs1-repair) cancelled by infra with zero
  report (recorded in the 380-vs1-repair2 packet), and the 20:45Z
  resend (8-batch) cancelled pre-start with zero report, zero diff,
  lane `.lanes/vs1` untouched. Fingerprint: full VS1 repair packet
  cancelled twice in 24 h while smaller packets in the same batches
  ran (same class as 355). Next: never resend 376 whole; the work
   proceeds via 380-vs1-repair2 (different title, eligible next wave).
   `BLOCKED: 376 whole-packet (infra-cancel x2).`
- **sp16-r1 review cancelled pre-start three times, never ran (owner:
  keeper infra or pool capacity).** Third identical failure: 403
  (wave-1) + 403 resend (wave-3) + fresh-title 420 (wave-4), all
  cancelled pre-start with zero report, zero diff; lane `.lanes/sp16-r1`
  and `target/lanes/sp16-r1.patch` (17526 bytes, 441 lines) untouched.
  Fingerprint: sp16-review whole-packet infra-cancel x3 across 3 waves
  while same-batch packets ran. Next: never resend whole; parked 403 +
  420 with PARKED rows; next is ONE judge resend alone in its batch (not inside
  a contended batch) or an RA14-style retarget of the REVIEW at the
  surviving lane/patch. `BLOCKED: sp16-r1 review whole-packet (infra-cancel x3).`
- **RA14 lane premise absent, twice (owner: builder one-off
  334).** Packet `326-ra14-rescope-review` BLOCKED then packet
  `330-ra14-window-repair` BLOCKED on the identical fingerprint: lane
  `ra14-top` registered in `target/lanes-worktrees.json` but
  `Test-Path .lanes/ra14-top` False, `git worktree list` shows only
  `lane/ra14` (no `lane/ra14-top`), and the premise files are absent
  everywhere (`arena_content.rs` nowhere, zero
  `arena_content|push_arena|stage_arena` symbols in `crates/game/src`,
  packet-cited `arena.rs:623` / `windowed.rs:1222,1245` do not hold
  stage/push calls). Fingerprint: registered-but-missing worktree +
  absent premise symbols. Repro: commands above on this tree. Next:
  one-off 334 retargets the repair onto the existing `.lanes/ra14`
  worktree (or recreates `ra14-top` via `lanes.mjs create` first) and
  rewrites the premise to the current dressing/cube code
  (`arena.rs:623-625 stage_dressing`, `windowed.rs:1206-1231`
  smoke_twin_list); the loop moves on to GM3/Q1 meanwhile.

### Open — 2026-09-23

- **GM3 clean packaged run blocked: no real AS1 bundle in-repo (owner:
  content/proof lanes).** Wave 43: `MISSING_DIST` (no dist recipe, dry-run
  only). Wave 44: `tools/pack-clean.mjs` landed (release build + stage +
  5-min clean-run delegate, loud `PACK-CLEAN FAIL [reason]` refusals,
  behavior-checked against `tools/clean-run.mjs` with a stub exe) but the
  only `arena-bundle.bin` on disk is the 16-byte empty stub at
  `fixtures/clean-run/content/arena-bundle.bin`, so `BUNDLE_RESOLVE` stops
  loud; plus release-build wall time (~40 min) was not spent. Repro: `node
  tools/pack-clean.mjs --replace` without `--bundle` →
  `[MISSING_BUNDLE]`. Next: wire a real AS1 bundle producer (CT8/content
  proof output or operator-supplied file) to `--bundle` /
  `$E2040_PACKAGE_BUNDLE`, then run the full proof. Blocks GM3 tick, PQ3,
   A8.   `BLOCKED: GM3 until a real bundle exists.`
  Wave 90 GM3-PARTIAL (capture-202609291056): `tools/pack-clean.mjs`
  v3 refuses 72-byte STUBs and stages real arena + editor + bundle
  into `C:/e2040-clean/` (arena 30-frame capture manifest
  checkpoints 0000000000000000/cc3840773f1fe049 on RTX 3050, editor
  `--exit-frames 5` exit 0); clean-room still FAILs
  `[ABSOLUTE_DEV_PATH]` (release exe bakes dev paths — needs remapped
  rebuild CARGO_TARGET_DIR=build/gm3 + --remap-path-prefix, ~40 min);
   `fixtures/clean-run/bin/arena.exe` stays STUB by design, as7-dist
   STUB stale. Next: remap rebuild + port v3 checks to root
   `tools/pack-clean.mjs` + EDITOR_REL manifest promotion (219).
  Wave 94 GM3-BLOCKED (219-gm3-remap): remapped arena+editor
  restaged to `target/as7-dist` (REAL/REAL/REAL, matching windowed
  captures, manifest checkpoints 0000000000000000/cc3840773f1fe049
  RTX 3050) but clean-room still FAILs `[ABSOLUTE_DEV_PATH]
  bin/arena.exe (d:\)` — a FALSE POSITIVE: the `DEV_PATH_MARKERS`
  naive 3-byte match in `tools/clean-run.mjs` scanTree (+
  `crates/game/src/package.rs` verify_clean) fires on x86-64
  machine-code immediates (`44 3A 5C` arena@394023, `43 3A 5C`
  editor@279934) no remap can remove, while real registry paths
  are correctly remapped to `./cargo-home`. Next: path-aware gate
  fix (owned `tools/clean-run.mjs` + verify_clean hunks) + restage
  + `clean-run --frames 30` PASS; lane `tools/pack-clean.mjs`
  promotes to root (editor dual-build, MZ/STUB + zero-entry
  refusals); EDITOR_REL stays extra-file compat (6-row manifest
  needs coordinated parseManifest update). Repro:
  `node tools/clean-run.mjs --dist target/as7-dist --target
  C:/e2040-clean/ --replace --frames 30` → FAIL, log
  `C:/Users/me/AppData/Local/Temp/opencode/gm3-clean30.log`.
  Wave 97: gate fix JUDGE-PASS (path-aware match silences all 7
  x86-64 immediates + c:/9pgr noise, real leaks still loud) and
  editor-env DONE in the same lane (zero `env!`/
  `CARGO_MANIFEST_DIR` left in `crates/editor` via runtime
  `checkout_root()`; rebuilt + restaged as7-dist editor 15535104B;
  30-frame CLEAN-RUN PASS). Caveat: stock `node
  tools/clean-run.mjs` full-mode fail-fasts (0xC0000409) inside node
  24.14 `fs.cpSync` on this dist tree — environmental, not the
  gate; PASS ran a temp copy with identical gate logic but filewise
  copy. Next: judge gm3-editor-env delta, then merger lands lane
  gm3 once (gate fix + editor hunks + pack-clean v3→v4); gate owner
  ports the filewise-copy workaround to root.
  Wave 101 patch-clean DONE (gm3.patch gate 6 paths 0 findings
  2026-09-29). Wave 102 merger BLOCKED x2: (1) `git apply --check
  --3way` refused `tools/pack-clean.mjs: does not match index`;
  (2) `git apply --3way` conflicts UU on `tools/pack-clean.mjs`
  (lane v1 base vs root v2 with bundle-cook/remap), reverted clean.
  Repro: `git apply --3way target/lanes/gm3.patch` on head
  973393a. Next: rebase lane gm3 onto current root pack-clean v2
  (or regenerate patch from lane), then merger lands; GM3 tick
  still gated on 30-frame proof from merged tree.
  Wave 102 resolved: fresh lane gm3b ported the 6 hunks byte-identical
  onto head 8972b2e (builder DONE, judge PASS RUST 4/5 DEPTH 4/5);
  lead landed code as 7762236 (game 16/16 + editor 2/2, gate PASS,
  quality 80/79 PASS). GM3 tick still gated on the 30-frame
  clean-room proof from the merged tree.
  Wave 110: filewise-copy port judge-PASS x2 (243+244) and landed; stock
  proof now reaches PLAY but FAILs `[PLAY_TIMEOUT]` x2 identical (arena
  prints `audio live` then no exit in 195s; in-lane same-command PASS 1h
  earlier). Fingerprint: PLAY_TIMEOUT-stock-x2. Repro: `node
  tools/clean-run.mjs --dist target/as7-dist --target C:/e2040-clean/
  --replace --frames 30` → FAIL, log `C:/e2040-clean/clean-run.log`.
  Next: pilot proof retry beside the loops (no quiet hour), then GM3 tick. `BLOCKED: GM3
  tick until a stock 30-frame proof exits 0.`
- **No executable shows anything (owner: render + game lanes).** No game or
  editor binary exists and GM3 packages a stub. Remedy: RA1, WIN1, ED1.
- **EA1 assistant loop died twice (owner: orchestrator).** Both attempts left
  no evidence. Remedy: the split rows EA1a–EA1e, one acceptance each.
- **Lane worktree GC prunes live helper lanes; patches lost (owner:
  orchestrator).** Found 2026-10-02 fix wave (30 agents): wave-1 lanes
  (cq10-nav DONE, cq6-endpoint DONE, cq5-spatial DONE, docs-bleh DONE,
  gm3-cleanrun DONE, cq8-core PARTIAL) present at builder report, absent
  ~1h later at judge time — 5/5 REVIEWs BLOCKED with no diff to read.
  Second identical failure: cq10-combat BLOCKED twice with the same
  signature (lead ran `remove` + `create`, helper found the worktree gone
  again before any edit). Fingerprint: `lanes.mjs diff/flags/list` say
  `already tracked`/`unknown lane` while `.lanes/<id>` is absent and no
  `lane/<id>` branch exists; `target/lanes-worktrees.json` keeps the stale
  entry. Salvage commit `6398cca` ("lane work salvaged, worktrees removed")
  holds main-tree docs only — no lane patches. Repro: dispatch a lane
  packet, wait out the batch, `git worktree list` before judging.
  Survivors (do not prune): `.lanes/cq14-anim` (CQ14 DONE, +88/-5, golden
  x2 green) and `.lanes/cq9-present` (CQ9 PARTIAL facade + 3 modules,
  green) — both ready for REVIEW + land. Next: builders run
  `lanes.mjs diff` immediately and the keeper collects the
  `target/lanes/<id>.patch` artifact before batch end; judges review from
  the patch, not the worktree; GC must spare lanes with live claims or
  recent diffs. `BLOCKED: landing wave-1 DONE lanes until re-reviewed
  from surviving worktrees or re-dispatched.`

### Open — 2026-10-02 (landing queue)

- **321-mcp20-boundary-review has no packet bundle (owner:
  keeper).** First occurrence: no lane `.lanes/321*`, no report/diff
  in tree. Next: re-issue with lane id + owned paths + report; do
  not judge vapor. `BLOCKED: 321 until re-issued.`
- **RL9 fog composite red at base (owner: render lane).**
  `present::tests_gpu::rl9_gpu_tests::fog_composite_shows_on_the_live_frame`
  fails deterministically on this box (file-top fogged bands read
  [117.6, 95.8, 81.2] vs ramp-times-AO [85.5, 67.6, 56.1], ao 0.605):
  fails in the vs2 lane with the patch AND with the patch stashed
  (496-test base run, same fingerprint), so it pre-exists VS2 and is
  not a textured-draw regression. VS2 scope (checker behavior, None
  parity, unknown-texture failure, 344 headless) is green around it.
  Next: fog owner re-pins the ramp-times-AO expectation or the
  contact mean wiring; look lanes leave the assertion untouched.
  `BLOCKED: fog composite assertion until fog owner re-pins.`
- **Lane registry drift (owner: toolsmith).** `node tools/lanes.mjs
  diff audit-split-pair-2` reports "unknown lane" while
  `.lanes/audit-split-pair-2` + branch `lane/audit-split-pair-2` are
  intact (7 files, gates PASS, golden holds — evidence via in-lane
  git diff instead). Fingerprint: registry
  (`target/lanes-worktrees.json`) lost a live lane. Next: self-heal
  or a `lanes.mjs sync` command (PROPOSAL in handoff).
  `BLOCKED: audit-split-pair-2 landing until registry repaired or
  judge reviews via in-lane git diff.`

## Moved 2026-10-16 (25 stale rows older than 3-day rule, stamps intact)

Moved verbatim from `docs/BLOCKERS.md` lines 24-48 (dated 2026-10-11 through 2026-10-13T00:26Z). Header/rule text in BLOCKERS.md untouched.

| 2026-10-11T15:00Z | FOCUS.md size vs 15KB cap: no seat may edit docs/FOCUS.md (smith and planner lanes exclude it; lead edits exclude it) | repro: size FAIL, 14.4KB | remedy: owner grants a one-line archival exception (superseded dated notes to docs/archive/focus-2026-10-11.md) or trims |
| 2026-10-11T16:33Z | loop:ratchet-render abandoned at 2nd identical FAIL (434 changed lines over max_diff 200, same FIX twice) | repro: target/abandoned/loop-ratchet-render-2026-10-11.patch (6 render files, decals+gtao fold + BindProbe) | remedy: replan as <=200-line slices landing decals+gtao first per FIX; keeper runs git restore --source=HEAD --staged --worktree on the 6 files (denied in this shell); never resend the 6-file fold |
| 2026-10-11T16:33Z | loop:vision-plans-table BLOCKED x2 on dirty docs/VISION.md (rows present uncommitted, vision-check green) | repro: git status shows M docs/VISION.md; smith r4 + r-next both BLOCKED dirty-tree | remedy: keeper lands the owned-path dirt (r3 DONE work) via land.mjs or commits it; never resend the packet while M |
| 2026-10-11T16:42Z | EG18 anim-crossfade abandoned at 2nd identical oversize failure (233 changed over max_diff 200 twice) | repro: target/abandoned/eg18-anim-crossfade-2026-10-11.patch (fade alone 207+/26- still over 200; FIX needs committed truth revert plus arena.rs outside paths) | remedy: keeper runs git restore --source=HEAD --staged --worktree -- crates/game/src/fighter_look.rs then replan as <=200-line slices; never resend the whole fade |
| 2026-10-11 | LJ1 coder runs cancelled twice (w38 long-prompt, w39 keeper exact-prompt packet: 0-lj1-look-judge-slice); fingerprint: Task cancelled, no diff, no proof; harness-side, packet unchanged and still valid | Replan LJ1 after 3h or as split title; DP1 chain continues | orchestrator#4322 | OPEN |
| 2026-10-11T21:45Z | loop:ratchet-render gtao repair abandoned at 2nd identical FAIL (238 changed lines over max_diff 200, same FIX twice: wave41 r8 + wave42 r10) | repro: target/abandoned/loop-ratchet-render-2026-10-11.patch (gtao.rs 122+/116-); twin_shadow twin also re-FAILED (1018 lines, existing 2026-10-10 cut-shadow-sun-depth x2 row stands, patch target/abandoned/EG11-2026-10-11.patch) | remedy: keeper runs git restore --source=HEAD --staged --worktree on the 8 render paths (denied in this shell); replan as <=200-line slices; never resend the fold |
| 2026-10-12T02:25Z | EG21 sound-packs abandoned at 2nd FAIL (repair wired should_stream as let _ = discarded, no pack/stream branch; review-2 FAIL DEPTH 1/5, same unwired root) | repro: target/abandoned/EG21-2026-10-12.patch (cue.rs+decode.rs 45 insertions) | remedy: keeper runs git restore --source=HEAD --staged --worktree on the 2 paths (denied in this shell); replan as behaviour-first slice (branch in CueBank::build + build-level test) ; never resend helpers-only |
| 2026-10-12T04:40Z | GS15 verify BLOCKED on 2nd identical cargo timeout (heavy runner exitCode=124 timedOut x2, engine-assets lib suite, no code touched) | repro: heavy-run-receipt-1791779878751 + 1791780287001, budget.rs complete on disk untracked | remedy: rerun verify off-peak or raise heavy-runner cap; code needs no change |
| 2026-10-12T07:05Z | GS27 verify BLOCKED on rust-lld link failure (undefined symbol driftsort_main/smallsort insert_tail, engine-ai lib test, x2, no code changed; module complete on disk) | repro: heavy-run-receipt-1791787606138 | remedy: fix toolchain link env or rerun off-peak; code needs no change, do not abandon the module |
| 2026-10-12T07:27Z | workspace edge-fix BLOCKED 2nd identical on staged dirt (tools/workspace-check.mjs staged alias-cleanup hunk; loop-workspace-endpoint-net-edge cannot edit; smith-5 NOOP then smith-edge BLOCKED, no code changed) | repro: node tools/workspace-check.mjs exit 1 FAIL engine-endpoint -> engine-net | remedy: keeper lands or parks the staged hunk first, then re-dispatch edge fix; never resend edge fix onto a dirty file |
| 2026-10-12T08:41Z | choke-door cut PARTIAL on rust-lld link env x2 (engine-spatial lib tests: undefined llvm symbols; cargo check/fmt/quality/gate green, deletion complete on disk; same toolchain fingerprint as GS27 row) | repro: cargo test -p engine-spatial --lib twice, rust-lld undefined symbol llvm | remedy: fix toolchain link env or verify off-peak; code needs no change, do not revert the deletion |
| 2026-10-12T09:35Z | SP63-a1 verify BLOCKED 2nd identical on foreign compile error (crates/render/src/quality.rs:197 13-tuple assert_eq with no Debug breaks engine-render build x2; SP63 code landed on disk unverified; quality.rs owned by running GS10 slice) | repro: cargo test -p engine-render --lib forward_id_tests fails to compile on quality.rs:197 twice | remedy: land GS10 quality.rs fix first, then re-verify SP63-a1; do not resend SP63 until the crate builds |
| 2026-10-12T10:05Z | gameplay seat cap-fail x2 (45m both, no output; r2 notes live foreign dirt in crates/ai mid-write by another seat) | repro: builder-gameplay r1+r2 both stopped at cap | remedy: sequence ai-crate seats one at a time, narrow gameplay scope, re-dispatch off-peak; no code fault recorded |
| 2026-10-12T14:10Z | E0610 blocks playtest lib (tournament.rs:865 u8 has no fields; LJ1 2nd identical compile failure; all playtest accepts + RB1 re-verify held; GA14 owns the files) | repro: cargo test -p engine-playtest --lib mirror_gdb E0610 x2 | remedy: GA14 fixes in place, then LJ1/RB1 re-verify; do not resend playtest packets until the crate compiles |
| 2026-10-12T15:05Z | LF4 verify BLOCKED 2nd identical on foreign sky.rs dirt (E0425 sky_view_upload, running LF1 lane uncommitted; LF4 slice scoped 114 lines, fmt/quality green) | repro: cargo test -p engine-render --lib grade E0425 x2 | remedy: LF1 lane lands or reverts sky.rs first, then LF4 re-verify; do not touch the lane's files |
| 2026-10-12T17:55Z | cut-relay-testpin land REFUSED x7 identical (rust-quality ratchet regresses staged net files, baseline not staged; working-copy net quality PASS 96A; deletion-only diff 64 lines, review PASS DEPTH 3/5 RUST 4/5) | repro: node tools/land.mjs cut-relay-testpin; fingerprint: refused: rust-quality ratchet regresses (staged files of:net) and tools/rust-quality.baseline.json is not staged | remedy: smith accepts/tightens the net baseline in the same commit per the land hint, or keeper lands once the baseline is staged; never resend land until the baseline is handled |
| 2026-10-12T18:10Z | GP30 poise-kinds abandoned at 2nd FAIL, same unwired root (review-1 FAIL public API zero prod caller; repair flipped pub to private; review-2 FAIL dead private helpers, callers only tests, fight.rs doc-only) | repro: target/abandoned/gp30-poise-kinds-2026-10-12.patch (depth.rs 2-line visibility flip); review FIX: DepthSet::observe route Hit/Blocked via apply_poise_kind or delete both fns | remedy: keeper runs git restore --source=HEAD --staged --worktree -- crates/combat/src/depth.rs (denied in this shell); replan only as observe-route or delete slice, never a visibility flip |
| 2026-10-12T18:12Z | smith-janitor-r3 land REFUSED x3 identical (review PASS, staged delete of tools/magic-scan.mjs, zero callers, gates green) | repro: node tools/land.mjs smith-janitor-r3; fingerprint: fatal: pathspec 'tools/magic-scan.mjs' did not match any files | remedy: land tool must commit a staged deletion by path; never resend land until it does — delete stays staged, harmless (file gone from disk and index, workspace-check 30/30 green) |
| 2026-10-12T18:45Z | OPT24 cassette tests BLOCKED on rust-lld link env x2 (engine-provider test binary: undefined serde_json map symbols; trim to 97 insertions complete on disk, check/quality/gate/fmt green) | repro: cargo test -p engine-provider --lib record_then_replay x2, same toolchain fingerprint as GS27/choke-door rows | remedy: fix toolchain link env or verify off-peak; code needs no change, do not revert the trim |
| 2026-10-12T19:09Z | GS31 class-rows abandoned at 2nd FAIL, same unwired root (review-1 FAIL public API zero callers + oversize; repair wired wrapper; review-2 FAIL class_tables/ClassId/MoveKind still zero production callers, tests-only surface) | repro: target/abandoned/gs31-class-rows-2026-10-12.patch (lib.rs +12 hunk); new classes.rs 182 lines removed by lead; review FIX: call crate::class_tables from tables_text/arena flow | remedy: keeper runs git restore --source=HEAD --staged --worktree -- crates/combat/src/lib.rs (denied in this shell); replan only as wrapper-plus-live-caller slice, never API-first |
| 2026-10-12T21:51Z | SP64 view-truth BLOCKED x2 identical on foreign game initializers (E0063 missing table_version in lib.rs:186 bench.rs:422,452 siege.rs:107; owned 3 files fmt/quality/gate green, proof unrunnable) | repro: cargo test -p engine-game --features app --lib brief E0063 x2 | remedy: loop:table-version-init-game lands V1 initializers first, then SP64 re-proves brief x2; do not touch game files from SP64 scope |
| 2026-10-12T22:04Z | provider cassette BLOCKED x2 identical on pre-existing HEAD fails (full crate 196 passed 3 failed before and after; owned opt24_double 2/2 green, fmt/quality/gate PASS) | repro: cargo test -p engine-provider --lib -- --test-threads=1 FAILED 196/3 x2; fingerprint: stream.rs:1051 assembled empty, openai.rs:547 + anthropic.rs:577 cancel under 100ms | remedy: owner packets in stream/openai/anthropic scope; cassette slice stays, do not revert |
| 2026-10-12T22:26Z | SP63 id slice BLOCKED x2 identical on pre-existing frame.rs:461 targets-none after resize (id_target 4 passed 1 failed twice; slice hunks in forward.wgsl/submit.rs green, packet-gate FAIL only on pre-existing long fns outside slice) | repro: cargo test -p engine-render --lib --features window id_target stays_clear FAILED x2 at frame.rs:461 | remedy: loop:sp63-frame-targets fixes resize targets first, then SP63 re-proves 5/5; do not touch frame.rs from SP63 scope |
| 2026-10-13T00:18Z | FOCUS coder seats r8/r9/r10 x3 stopped at 75-min cap with no record and no diff info (r9: 75m work plus 13m waiting for a build slot) | repro: keeper queue results failed r8/r9/r10; fingerprint: stopped after cap, slot-wait 13m | remedy: keeper narrows FOCUS packets to fit the cap or frees cargo slots (6 slots, heavy queue); lead stages nothing blind on unidentified packets; retry is keeper's call |
| 2026-10-13T00:26Z | arena live.rs dirt BLOCKED x2 identical (E0615 attempted to take value of method tick on LiveMatch at live.rs:248 step_stamped; EG8 run plus GP14 run, both stopped, app-feature builds all red) | repro: cargo test -p engine-game --lib --features app -- fight_camera_widens FAIL x2 at live.rs:248:30; fingerprint: E0615 step_stamped/self.tick, HEAD has no step_stamped | remedy: loop:arena-live-tick-fix owns live.rs alone, then app builds re-prove; no other lane touches live.rs meanwhile |
