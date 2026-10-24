### Finish line (Tier A, in dispatch order)

Sizes are agent sessions: S 1, M 2–4, L 5–10.

**Truth rule (operator 2026-09-24).** A `render` row is ticked only when its
pass submits on a real device in the window build; CPU reference math and
llvmpipe parity alone are merge gates, not done. RL1, RL2, RL3 and RP1 are
ticked but their `submit_gpu` still returns `Unsupported` (`graph.rs`,
`shadow_cluster.rs`, `ibl.rs`, `gtao.rs`, `post.rs`); GX1–GX5 make them live
and dispatch before every other render row. The same wave later ticked RL4,
RL6, RP2 and RP3 with `Unsupported` still in `sky.rs`, `material_layer.rs`,
`particles_gpu.rs` and `decals.rs` (operator caught it 2026-09-24); GX7–GX10
make those live, and every row built on them waits.

- [x] GX13 render: IBL release-capture close — pilot only, no lane, no
  code. Re-run the wave-51 `chart_target_size` fix proof on the
  reference GPU: release window build (arena scene, boosted clocks),
  1600x900 viewport downscales with no `exceeds edge 256` exit,
  material chart PNGs + per-pass ms + RA8 manifest posted under
  `target/pilot-gx13/`. Tier A. Size S. Owns nothing in `crates/`
  (evidence only). Accept: window presents N frames with 0 validation
  errors; reference-GPU chart beside the RL6 chart within tol; per-pass
  ms posted; Q1 V1-V3 consume the same set. needs: GX1, GX3, GX12,
  RL6, PF2. (GX3 VERDICT wave 56.) (Owner 2026-10-10: Q1 dropped
  from needs, a cycle; GX13 feeds Q1.) Landed 4e51bd7a (docs/receipts/2026-10-18-gpu-pilot-gx13-gx14-rl30-pf19.md). Chart at 1600x900 down to 256x144 with no edge-256 exit, max diff 1.2e-7 vs 1e-5, 0 validation errors in a 300-frame window on the RTX 3050 6GB Laptop GPU, RA8 manifest generated (target/pilot-gx13/).
- [x] GX14 render: GTAO boosted-release proof — pilot-measurement only, Progress 2026-10-19: pilot measured, the bar posted as FAIL (receipt docs/receipts/2026-10-19-gx14-gtao-pilot.md, evidence target/evidence/gx5/). Pinned release window build 4082c7d1 (`target/release/arena.exe`, sha256 C5F98DF9…373596C; the only later game/render commit is 42f1c2a0 RZ7, CPU-only), adapter NVIDIA GeForce RTX 3050 6GB Laptop GPU driver 32.0.16.1062, clocks observed max 1732 of 2100 MHz, shared loaded box (observed-only). A real 1080p target without code: the arena's own window forced borderless to a 1937x1080 client (SWP_NOSENDCHANGING, GetClientRect verified; live-1080p-r3 prints resolution=1937x1080), with a 640x360 control proving the resize reaches the renderer (gtao 0.125 ms vs 0.33 ms at 720p). Table, 600 frames, 8 fighters, validation_errors=0: gtao device-clock mean 0.365-0.441 ms at 1080p (5 runs) vs 0.337-0.375 ms at 720p (3 runs). RA8 manifest target/evidence/gx5/ra8.json (15 frames, ticks 1..300, sequence_fnv e7cd5be5c38a77de, tool validator ok); the ignored exact-1920x1080 oracle probe ran clean (upload_ms=45.27 submit_ms=90.07, host-inclusive, 0 validation errors); llvmpipe note: no llvmpipe adapter here and the pass has no software path, the gtao CPU/oracle leg is green on the PC and on the Linux runner (gh run 39211727608). `node tools/proof.mjs --skip-build`: legacy golden d57b5c8ae3096614 plus all four seed double-runs and the tape replay PASS, RESULT FAIL only on reds from other agents' files (GP5 engine diff, engine-render dup_fns 8 -> 9, endpoint docs coverage). Left: the Accept's p95-at-1080p bar is unproven — the PF2 ring holds p50/p99 only and the window prints the gtao mean, so no p95 stamp exists, and WINDOW_SIZE is a const with no --size flag (both code in render/game, outside this row's owned paths; gtao.rs untouched), which is an owner call on which row lands them; the tick also waits for proof.mjs green on a clean tree.
  no pass code: pinned release window build, arena 1080p, PF2 per-pass
  GPU stamps (gtao probe via PF2), table + RA8 manifest + llvmpipe note;
  miss fails with numbers, bars never loosened. Tier A. Size S. Owns
  receipt hunks beside `target/evidence/gx5/` (`gtao.rs` frozen, no
  donors). Accept: gtao p95 at most 0.8 ms at 1080p with adapter line
  or posted FAIL; 0 validation errors; `node tools/proof.mjs` PASS
  golden x2. needs: GX1, PF2, RL12. (GX5 VERDICT wave 56: GX5 is
  gtao.rs, not skinning; DROP 078-gx5-r2; wave-105 split: GX5 no
  longer blocks this row.) Progress 2026-10-18: 4e51bd7a not measured: the arena prints gtao only as a per-pass mean (0.19-0.31 ms at 720p) and the window cannot run at 1080p, so there is no p95 stamp.
  Owner chat 2026-10-19 (delegated): the p95 question is answered by the Accept's own escape — "gtao p95 at most 0.8 ms at 1080p with adapter line **or posted FAIL**" — and the pilot posted the FAIL with its receipt, so no new row or bar is needed and the `--size` flag stays a separate wish. What is left is the Accept's last clause, `node tools/proof.mjs` PASS with golden x2. Measured 2026-10-19 ~09:5x: the full gate stops on compile errors in three files that belong to live sessions, none of them this row's — `crates/net/src/handshake.rs` (E0277 `Pending: Clone`/`Debug`, RC15's in-flight file), `crates/render/src/particles_gpu.rs:491` (E0308), `crates/material/src/compile.rs:75,380` (`-D warnings`: excessive precision, `flat_map`). The row stays open until the tree is clean; the run is repeatable with `node tools/proof.mjs`, and this pass killed its own run rather than hold the machine.
  BLOCKED: needs a clean tree - live sessions' in-flight files stop `proof.mjs` (named above; as of 2026-10-19T10:38Z also `crates/net/src/quic.rs`, which crossed the 1500-line big-file ratchet so `rust-quality` reads engine-net big_files 0 -> 1); the row ticks the moment the gate passes. Unblocks per docs/plan/2026-10-19-owner-queue.md. RETIRED (low): pilot-measurement only, already measured FAIL; no player-visible gain (lead, trim audit 2026-10-24)

- [ ] RL5 render: baked lightmaps with validity mask + SH probes for
  fighters reusing the `ibl.rs` SH path. Tier B. Size L. Owns
  `crates/render/src/baked_gi.rs`. Accept: bake identical bytes x2;
  60 fps arena + 8 fighters on the reference GPU (reference GPU).
  Deep 3D pass 2026-09-25: lightmap UV unwrap and packing happen in the cook (no overlapping charts, bake identical x2); Base tier only. needs: GX1, RL2, AS1.
  Wave 44 progress (stays open): --scene/--fps flags, 120-frame ARENA
  capture @ 0.4 fps debug (not 60), bake x2 green; honest gap: 5-tick cap
   repeats checkpoints past frame 24 — needs fixed-step capture pacing. Progress 2026-10-19: operator/owner: every code leg in the row's own file is landed and green (`cargo test -p engine-render --lib baked_gi`: 25 passed, 2 ignored: bake_identical_bytes_twice, rl5_8fighter_capture_receipt_x2, rl5_ra1_receipt_consumes_the_bind, validity mask, sealed tunnel, forward-GI markers). Close still needs the reference-GPU 60 fps arena + 8 fighters receipt (RL29 boosted-release pilot) and the RL14 pacing log; the pacing fix itself sits in the game crate (`windowed.rs` WIN_MAX_STEPS_PER_FRAME, `bin/arena_frame`), which neither RL5 nor RL14 lists as an owned path, so the owner must say which row lands it.
  BLOCKED: the close needs the reference-GPU 60 fps arena plus 8 fighters receipt (operator; see this row's note). Unblocks per docs/plan/2026-10-19-owner-queue.md.
- [x] RL14 render: fixed-step capture pacing + release receipt for RL5 —
  frame-accurate checkpoint pacing past frame 24 plus a release-build
  capture receipt (adapter/driver/power + fps). Tier B. Size S. Owns
  `target/evidence/rl5-pacing/` (receipt + pacing log). Accept: 120-frame
  ARENA capture with no repeated checkpoints, release receipt on the
  reference GPU. needs: GX1, RL5, RL12, PF2, CT7. (PLAN-RL5 wave 56:
  RL5 blocked by missing ref-GPU evidence + capture-pacing bug, not
  prereqs/scope.) RETIRED (dup): it is RL5's capture pacing, covered by RL5's own Accept (lead, trim audit 2026-10-24)
- [ ] RL11 render: look-gate capture bundle — one reference-GPU capture
  set closing the RL5/RL7/RP4 look gates together (material chart, day
  ramp, tunnel seal, shimmer scene) with RA8 manifests, driven by WIN1
  and Q1 on the same build. Tier A. Size S. Owns the capture-bundle
  hunks in `crates/render/src/` fixtures + `tools/` manifest glue
  (orchestrator lands `tools/`). Accept: llvmpipe parity within posted
  tolerance; 0 validation errors; reference-GPU captures posted with
  per-pass ms; Q1 V1-V3 go green on the bundle. needs: GX1, RL5, RL7,
  RP4. (PLN26 render-look.md, reviewed 2026-09-25.)
- [x] RL13 render: glTF-anchored material/exposure chart for GX3 IBL
  parity (READER-LOOK53 card 3). Tier A. Size S. Owns chart fixture +
  `crates/render/src/` IBL bind hunks. Accept: llvmpipe parity within
  tol + reference-GPU chart with RA8 manifests. needs: GX1, RL2, RL6,
  Q1. (LOOK53.) RETIRED (low): a chart of materials for the owner; no engine change (lead, trim audit 2026-10-24)
- [x] RL16 render: twin material + IBL bind — the WIN1 window twin forward
  path samples the GX3 IBL ambient plus RL6 layered sand/cloth response
  instead of flat vertex-color brown, as bind glue only in a new
  `crates/render/src/twin_look.rs` (pass code and `arena_day.rs` frozen).
  Tier A. Size S. Owns `crates/render/src/twin_look.rs`. Accept: capture
  pair before `target/pilot-gx13/owner-arena/frame_000015.png` vs after
  on the reference GPU scores higher by this seat on materials + light
  with 0 validation errors; golden unchanged. needs: GX1, GX3, RL6.
  (LOOK-ART 2026-09-28.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
  Correction 2026-10-21: the Accept's before frame `target/pilot-gx13/owner-arena/frame_000015.png` is missing (`Test-Path` = False) and `crates/render/src/twin_look.rs` is absent, so the row cannot pass as written; it needs an owner amendment to a pair this seat renders itself, or it closes. RETIRED (dead): crates/render/src/twin_* is gone (window is the one path) (lead, trim audit 2026-10-24)
- [ ] RL18 render: percentile-trimmed histogram auto-exposure with temporal
  adaptation vs fixed exposure (LADDER P4 CANDIDATE ACCEPTED wave-58
  merge). Tier B. Size S. Owns exposure hunks beside
  `crates/render/src/post.rs` (resolve frozen; presentation only, never
  the sim hash). Accept: baseline fixed GradeSettings exposure 1.0 ->
  64-bin log-luma histogram (10-95 percentile trim, adapted EV at 3/s)
  on day-sweep EV100 10-14 + torch-grid HDR fixtures; metric
  clipped-highlight share + inter-frame EV flicker + per-pass ms with
  adapter line; llvmpipe parity within tol. Kill: clipped share improves
  <15% or flicker >0.5 EV/frame or ms +>5% or tick hash drifts.
   Consumer WIN1 match, Q1 captures. needs: GX4, GX7, PF2.
   (DO NOT DISPATCH 2026-10-05: CPU legs PASS in-lane; headless writers skip. Take RL18a pilot GPU-ms + Q1-capture closeout only, per 510 deep-dive.)
   Progress 2026-10-19: RL18a pilot GPU half run on the reference GPU (RTX 3050 6GB Laptop GPU, driver 32.0.16.1062). `cargo test -p engine-render --features window --lib auto_ -- --nocapture` through gh-check (local: the window feature never builds on the Linux runner) = 12 passed 0 failed 1 ignored, incl. `gpu_auto_chain_matches_cpu_chain` PASS parity 0.000000 (tol 0.005), adapted -4.986 stops; same filter without `window` = 11 passed 0 failed 1 ignored. Kill metrics in lane: day-sweep fixed clip 0.727 -> adapted 0.000 (improve 1.000, gate 0.15), torch grid 1.8698 -> 1.7365 stops (0.133 EV), blink and match-clip worst step 0.0500 stops (gate 0.5). No crate code touched, so the golden is untouched (presentation only). Left, not producible here: (1) per-pass ms with an adapter line - the live frame runs forward+blit only, post/exposure is probe-only, `arena per-pass` has no post column, and the device path splits bloom/exposure/tonemap into thirds, so no honest exposure ms exists (owner: RL21/RL26/RA14 or a PF2 presenter stamp); (2) llvmpipe parity - no llvmpipe adapter on this PC and the runner cannot build the window feature; (3) Q1 captures - Q1 is open on RL21, RL26, RA14. The live match also still rides fixed 1.0 (`FrameLight` in `crates/game/src/view_truth.rs`), so the WIN1 consumer is not on the adapted EV.
  BLOCKED: the remaining legs are not producible without the live adapter (per-pass ms with an adapter line); operator run. Unblocks per docs/plan/2026-10-19-owner-queue.md.
- [x] RP8b render: variance-clip re-spike, ONLY if the lead overturns
  the RP8a REJECT — re-run ghost + shimmer fixtures with the named
  delta beside `crates/render/src/post.rs` (resolve frozen).
  Tier B. Size S. Accept: ghost-trail px cut ≥30% with shimmer
  ≤0.06 and ms +≤5%, or second REJECT with numbers. needs: RP8a.
  (A7, 2026-10-02.) Progress 2026-10-19: operator/owner: the row's own gate
  is unmet — RP8b is boarded "ONLY if the lead overturns the RP8a REJECT",
  and no overturn exists. docs/research/LADDER.md:84-85 hold the two standing
  REJECTs (ghost cut 42.5%/41.1% PASS, shimmer PASS, ms +134.7%/+22.5% FAIL
  the +5% bar, box-clamp stays canonical); the archived parent row says
  "DO NOT DISPATCH ... RP8b only if the lead overturns with a fused bound
  inside the ms bar plus release-perf plus digest x2"
  (docs/archive/SPRINT-done-2026-10-10.md:453) and its deep-dive keeps RP8b
  parked (docs/archive/2026-10-10/plans/render-look-cut.md:207-213).
  No overturn and no such fused-bound patch exists (the lane patches are
  gone), so a re-run would only repeat the known +22.5% FAIL. post.rs
  untouched (resolve frozen); the row stays parked until the lead overturns.
  Landed. Owner chat 2026-10-19 (delegated): not boarded: RP8b is boarded only if the lead overturns the RP8a REJECT, and the delegated call is no overturn (docs/research/LADDER.md:84-85 still holds the two REJECTs).
- [x] RL21 render: twin arena-bundle bind — the WIN1 twin forward path
  draws the recooked arena bundle (CT8 recook 4996849) instead of the
  synthetic cube, as bind glue only in a new
  `crates/render/src/twin_arena.rs` (pass code, `shadow_cluster.rs`,
  `arena_day.rs`, `twin_shadow.rs`, `arena_backdrop.rs`,
  `arena_air.rs` frozen; CT22/CT29 JSONs untouched). Tier A. Size S.
  Owns `crates/render/src/twin_arena.rs`. Accept: capture pair before
  `target/pilot-owner-view/r2-20260929/arena/frame_000000.png` vs
  after on the reference GPU scores higher by this seat on silhouette
  + materials with 0 validation errors; golden unchanged. needs: CT8,
  GX1, WIN1. (ART wave 86.)
  (Owner 2026-10-10: unfrozen; the D7 Bevy-frame gate died with D8/D9.
  FOCUS Now Look row; Q1 needs it.)
  Progress 2026-10-19: operator/owner: the bind glue is landed and live in
  both twin paths (`crates/render/src/twin_arena.rs`, 26434e12, deepened
  since; the window `frame_list` and `push_kit_draws` swap the mesh-1 cube
  when the CT8 kit is resident), and the after frame on the reference GPU is
  clean, but the Accept's before frame
  `target/pilot-owner-view/r2-20260929/arena/frame_000000.png` is not on
  disk (`docs/plan/render-look.md:57`: zero reference PNGs on disk; Q1 owns
  them), so the pair and this seat's silhouette + materials score cannot be
  produced here. Evidence 2026-10-19 on the RTX 3050 6GB (driver
  32.0.16.1062): `target/release/arena.exe --scene arena --fighters 4
  --frames 60 --exit --no-break-look --capture-dir
  target/rl21-20261019/after-nobreak --capture-every 60 --fps 30 --sha
  2ca37f6 --rows RL21 --mute` printed `staged 11 kit draws (0 skipped)`,
  `arena capture: frames=1
  manifest="target/rl21-20261019/after-nobreak\manifest.json"
  sha=73fd3ced036e8be7`, `validation_errors=0`; the opened frame_000000.png
  shows the recooked kit walls, stands and sand with no synthetic cube (the
  centre box of the earlier run is the RZ4w break crate: it leaves with
  `--no-break-look`). Glue proof: `node tools/gh-check.mjs cargo test -p
  engine-render --features v1 --lib twin_arena` 23 passed 0 failed (run
  39176517660). This note changes no crate or content path, so the golden
  stays d57b5c8ae3096614. Left: the owner supplies the Q1 before PNG, or
  amends Accept to a before/after pair this seat renders itself (cube
  fallback twin frame vs this kit frame); then one score closes the row.
  BLOCKED: the Accept's before frame is not on disk; needs the owner's artefact or an operator capture (see this row's note). Unblocks per docs/plan/2026-10-19-owner-queue.md.
  Correction 2026-10-21: the bind is landed (26434e12) and only the Accept's before frame `target/pilot-owner-view/r2-20260929/arena/frame_000000.png` is missing (`Test-Path` = False), so the row cannot pass as written; the owner supplies the PNG or amends the Accept. RETIRED (dead): twin path gone (lead, trim audit 2026-10-24)
- [x] RL22 render: editor viewport day-rig bind — the editor viewport
  draws the CT8 arena establishing view under the RL4 sky bake plus
  RL15 day rig plus RL7 fog ramp instead of the brown starburst, as
  bind glue only in a new
  `crates/render/src/editor_day_preview.rs` (sky/fog/pass code frozen;
  `tunnel_mouth.rs` and `twin_arena.rs` untouched). Tier A. Size S.
  Owns `crates/render/src/editor_day_preview.rs`. Accept: capture pair
  before `target/pilot-owner-view/r2-20260929/editor-a3/viewport.png`
  vs after on the reference GPU scores higher by this seat on light +
  composition with 0 validation errors; golden unchanged. needs: RL4,
  CT8. (ART wave 86.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
  Correction 2026-10-21: the Accept's before frame `target/pilot-owner-view/r2-20260929/editor-a3/viewport.png` is missing (`Test-Path` = False) and `crates/render/src/editor_day_preview.rs` is absent, so the row cannot pass as written; rewrite the Accept or retire it. RETIRED (dead): crates/render/src/editor_day_preview.rs never existed (lead, trim audit 2026-10-24)
- [x] RL23 render: twin grade + exposure bind — the WIN1 twin forward path
  samples the RL4 EV100 exposure plus post grade instead of the current
  flat brown/black faces, as bind glue only in a new
  `crates/render/src/twin_grade.rs` (pass code, `twin_look.rs`,
  `twin_shadow.rs`, `twin_arena.rs` and `post.rs` frozen). Tier A.
  Size S. Owns `crates/render/src/twin_grade.rs`. Accept: capture pair
  before `target/pilot-owner-view/r3-20260929/arena/frame_000014.png`
  vs after on the reference GPU scores higher by this seat on light +
  materials with 0 validation errors; golden   unchanged. needs: GX1,
  RL4. (LOOK-ART 2026-09-29.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
  Correction 2026-10-21: the Accept's before frame `target/pilot-owner-view/r3-20260929/arena/frame_000014.png` is missing (`Test-Path` = False) and `twin_grade.rs`/`twin_look.rs` are absent, so the row cannot pass as written; rewrite the Accept or retire it. RETIRED (dead): twin path gone (lead, trim audit 2026-10-24)
- [x] RL24 render: viewport starburst source removal — name
  the pass emitting the radiating bands from frame center in the editor
  offscreen viewport and remove it, in
  `crates/editor/src/viewport_image.rs` (RL22's
  `editor_day_preview.rs` day-rig bind untouched; sky/fog/pass code
  frozen — RL22 replaces the scene, this row stops the artifact riding
  along into it). Tier A. Size S. Owns
  `crates/editor/src/viewport_image.rs`. Accept: capture pair before
  `target/pilot-owner-view/r4-20260929/editor-a3/viewport.png` vs after
  on the reference GPU scores higher by this seat on light +
  composition with 0 validation errors; golden unchanged. needs: ED1,
  GX1. (ART wave 91.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
  Correction 2026-10-21: the Accept's before frame `target/pilot-owner-view/r4-20260929/editor-a3/viewport.png` is missing (`Test-Path` = False; `crates/editor/src/viewport_image.rs` does exist), so the row cannot pass as written; rewrite the Accept as an on-disk pair. RETIRED (dead): twin path gone (lead, trim audit 2026-10-24)
- [x] RL25 render: twin dome facet removal — the fullscreen
  background renders as flat-shaded diamonds, identical facets in the
  dark r4 frame and the lighter rl15 day-rig frame so geometry not
  lighting; smooth the facet emitter — 342 diagnosis 2026-10-03: the
  facets are the 6 dressing scaled-cubes + central cube via
  `frame_list`/`push_dressing_draws` drawn by ra1-forward
  `encode_forward` (receipt draws = list length), not a sky dome
  (sky.rs is meshless compute + direction sampling; nothing consumes
  a dome mesh today). Tier A. Size S. Owns dressing/camera hunks
  beside `crates/game/src/windowed.rs` (RL4's bake plus RL8's
  forward-skybox sampling bind frozen). Accept: capture pair
  before `target/pilot-owner-view/r4-20260929/arena/frame_000014.png`
  vs after on the reference GPU scores higher by this seat on materials
  + light with 0 validation errors; golden unchanged. needs: GX1, RL4.
  (ART wave 91.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
  Correction 2026-10-21: the Accept's before frame `target/pilot-owner-view/r4-20260929/arena/frame_000014.png` is missing (`Test-Path` = False; `crates/game/src/windowed.rs` does exist), so the row cannot pass as written; rewrite the Accept or retire it. RETIRED (dead): twin path gone (lead, trim audit 2026-10-24)
- [x] RP11 render: twin silhouette fringe removal — a thin
  light rim outlines the sharp `v1::cube()` silhouette (no bevel in
  `v1.rs`, frozen frames rule out TAA ghosting); name the emitting pass
  (bloom, grade, vignette, depth-reject) and remove it in the fringe
  hunk of `crates/render/src/post.rs` (RP1 resolve, RP6 selection, RP8
  TAA reject, RP5 scale seam frozen). Tier A. Size S. Owns
  `crates/render/src/post.rs` (fringe hunk). Accept: capture pair
  before `target/pilot-owner-view/r4-20260929/arena/frame_000014.png`
  vs after on the reference GPU scores higher by this seat on
  silhouette + materials with 0 validation errors; golden unchanged.
  needs: GX1, RP1. (ART wave 91.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
  Correction 2026-10-21: the Accept's before frame `target/pilot-owner-view/r4-20260929/arena/frame_000014.png` is missing (`Test-Path` = False; `crates/render/src/post.rs` does exist), so the row cannot pass as written; rewrite the Accept or retire it. RETIRED (dead): twin path gone (lead, trim audit 2026-10-24)
- [x] RL26 render: twin 8-fighter draw bind — the WIN1 twin
  `frame_list` in `crates/game/src/bin/arena.rs` (HUD clear plus one lit
  cube, read 2026-09-29) ignores `--fighters 8 --scene arena` so sim
  advances (tick 30 `b26e378f`) with zero pixels (15/15 identical);
  draw the CT7 hero plus CT15 staged fighters in the twin as bind glue
  only in a new `crates/render/src/twin_fighters.rs` (pass code,
  `shadow_cluster.rs`, `arena_day.rs`, `twin_shadow.rs`, `twin_arena.rs`,
  `twin_grade.rs`, `twin_look.rs` frozen; CT29 JSON untouched; `frame_list`
  calls the new bind, `present.rs` untouched). Tier A. Size S. Owns
  `crates/render/src/twin_fighters.rs`. Accept: capture pair before
  `target/pilot-owner-view/r5-20260929/arena/frame_000014.png` vs after
  on the reference GPU scores higher by this seat on silhouette +
  composition with 0 validation errors; golden unchanged. needs: CT7,
  GX1, WIN1. (ART wave 92.)
  (Owner 2026-10-10: unfrozen; the D7 Bevy-frame gate died with D8/D9.
  FOCUS Now Look row; Q1 needs it.)
  progress 2026-10-10: `twin_fighters.rs` binds eight body slots (slot 0 is
  a red stand-in box for the hero, slots 1..7 the staged opponents, a fixed
  feet row), staged in `twin_headless_capture` and pushed from `frame_list`
  (`arena_capture.rs`). Proof: `cargo test -p engine-render --lib
  twin_fighters --features v1` 4 passed; ratchet and workspace-check pass.
  Not met: the skinned CT7 hero is not drawn (`FighterLook` needs
  `&mut Presenter`, which `frame_list` lacks), and the before/after capture
  pair with judge score on the reference GPU is still open (runner job).
  Progress 2026-10-19: operator/owner: the bind is landed and its pixels are
  now proven; the Accept's before frame is what is missing. `frame_list`
  (`crates/game/src/bin/arena_capture.rs`) pushes one draw per resident body
  staged by `twin_headless_capture`, and the new reference-GPU test
  `twin_fighters_reach_the_frame_pixels` renders the twin offscreen twice at
  128x96 (the `frame_list` frame with the eight staged bodies, and the same
  frame with their draws removed): the PNG bytes differ with validation
  clean, so an 8-fighter twin frame is no longer the old zero-pixel cube
  frame (before the bind `frame_list` pushed no body, so the removal changed
  nothing and this test failed). Proof 2026-10-19 on the RTX 3050 6GB
  (driver 32.0.16.1062), Landed 743e3dcc: `node tools/gh-check.mjs --local
  cargo test -p engine-game --features app --bin arena twin_fighters` 1
  passed, the whole bin 83 passed, `cargo test -p engine-render --features
  v1 --lib twin_fighters` 5 passed; the same `--fighters 8 --scene arena`
  window run files 4 distinct frames
  (`target/rl26-20261019/after8/manifest.json`, sequence_fnv
  469900e5f9e6814c, validation_errors=0, `fighters=8 seen=8 empty=0`,
  `fight_draws=32`, `fighter look bound 4 CT7 bodies`) and the opened
  frame_000002.png shows the eight fighters, kit walls, stands and sand (the
  twin's own slot 0 stays the CT7 hero graft bulk stand-in; the real skinned
  hero is still drawn by the window look). Left: the before half
  `target/pilot-owner-view/r5-20260929/arena/frame_000014.png` is not on
  disk and never was in git (`docs/plan/render-look.md:57`: zero reference
  PNGs on disk; Q1 owns them), so this seat cannot render the pair; the row
  closes on the Q1 reference or on an owner amendment to a pair this seat
  renders itself. Test-only hunk, no sim path touched, so the golden
  d57b5c8ae3096614 stands.
  BLOCKED: the Accept's before frame is not on disk; needs the owner's artefact or an operator capture (see this row's note). Unblocks per docs/plan/2026-10-19-owner-queue.md.
  Correction 2026-10-21: the bind landed (743e3dcc) and `target/rl26-20261019/after8/manifest.json` = True, so only the before half `target/pilot-owner-view/r5-20260929/arena/frame_000014.png` is missing (`Test-Path` = False); the row cannot pass as written until the owner amends the Accept to a pair this seat renders. RETIRED (done): a9b0a047c (lead, trim audit 2026-10-24)
- [x] RP10 render: twin frame-advance proof — the WIN1 twin presents 30
  distinct frames with advancing HUD plus tick instead of 30
  byte-identical cubes, in a new
  `crates/render/src/clip_advance.rs` reusing the RP1 TAA resolve and
  RP5 scale seam (resolve and `post.rs` frozen; `clip_hold.rs`
  untouched). Tier A. Size S. Owns
  `crates/render/src/clip_advance.rs`. Accept: manifest pair before
  `target/pilot-owner-view/r2-20260929/arena/manifest.json` (sha
  frozen 4th run) vs after (sha changes, 0 identical) on the reference
  GPU scores higher by this seat on motion + composition with
  ghost/shimmer inside the RP6 bars and 0 validation errors; golden
    unchanged. needs: GX1, GX4, RP5. (ART wave 86.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
  Correction 2026-10-21: the Accept's before manifest `target/pilot-owner-view/r2-20260929/arena/manifest.json` is missing (`Test-Path` = False) and `crates/render/src/clip_advance.rs` is absent, so the row cannot pass as written; rewrite the Accept or retire it. RETIRED (dead): crates/render/src/clip_advance.rs never existed (lead, trim audit 2026-10-24)
- [x] RL27 render: cascaded shadow maps for the arena-day sun — cascade-split Landed 8d5f2ea2. `cargo test -p engine-render --lib -- shadow_cascade post ibl` 91 passed, 2 ignored (13 shadow_cascade tests incl. splits_pinned_as_data and day_sweep_byte_identical_x2, ibl green); render tests run on this PC by design (GPU code).
  selection + per-cascade shadow-map sizing ported from `FyroxEngine/Fyrox`
  `fyrox-impl/src/renderer/shadow/csm.rs` (license MIT, adapt with
  attribution). Tier B. Size M. Owns `crates/render/src/shadow_cascade.rs`.
  Accept: split distances pinned as data; day-sweep fixture x2 byte-identical;
  llvmpipe parity within tol; golden `d57b5c8ae3096614` held. F2P:
  `cargo test -p engine-render --lib shadow_cascade` (fails before: day-rig
  sun uses one split, no cascade unit). P2P:
   `cargo test -p engine-render --lib ibl` stays green. needs: GX1, RL2.
   (STEAL 2026-10-02 engine2040.md #1; EB-2026-10-02-S93.)
- [x] RL29 render: RL5 boosted-release capture pilot — evidence only under
  `target/rl29/`, no `crates/` hunks (code receipt
  `rl5_8fighter_capture_receipt_x2` at `crates/render/src/baked_gi.rs:1381`
  plus `rl5_ra1_receipt_consumes_the_bind` at `baked_gi.rs:1427` already
  green headless; 60 fps + 8-fighter capture missing). Tier B. Size S.
  Owns `target/rl29/` (pilot outputs only). Accept: release pilot
  `--scene arena --fighters 8 --frames 120 --exit` posts 3 PNGs + RA8
  manifests + PF2 ms + VRAM/draws; bake bytes x2; 60 fps on reference
  GPU. F2P: 8-fighter capture set absent before. P2P: 3 PNGs posted,
  manifests + ms rows posted, bake bytes x2 identical. needs: RL5, RL28,
  PF2. RETIRED (dup): RL5's boosted-release pilot, covered by RL5 (lead, trim audit 2026-10-24)
- [x] RL30 render: RL27 day-sweep parity pilot — evidence only under
  `target/rl30/` (code owns new `crates/render/src/shadow_cascade.rs`,
  absent on master; `SUN_CASCADES = 3` + `CASCADE_LAMBDA = 0.5` at
  `crates/render/src/shadow_cluster.rs:20-23`). Tier B. Size S. Owns
  `target/rl30/` (pilot outputs only). Accept: day-sweep A/B x2
  identical; llvmpipe parity in tol; 0 errors; golden held; 3 splits
  posted. F2P: cascade day-sweep set absent before. P2P: A/B x2 Landed 4e51bd7a. Day sweep byte-identical twice (sha256 80297F11), splits 21.72/49.74/120.00 m, day rows 10/34.64/120 m, same row for all five sun elevations; Linux runner 13 shadow_cascade tests passed (run 39066373935); no llvmpipe adapter on this PC.
   identical posted, parity in tol. needs: RL27, RL2.
- [ ] RL31 render: Bevy-atmosphere Bruneton LUT bake beside the analytic
  sky - port the transmittance plus multiscattering plus sky-view plus
  aerial-view LUT bakes from `bevy_pbr-0.20.0-rc.2/src/atmosphere/` with
  `bevy_light` atmosphere settings (MIT, licence notice kept, own WGSL,
  zero donor bytes pasted) beside the analytic Hillaire-school scatter in
  `crates/render/src/sky.rs`, presentation only never in tick. Tier A.
  Size S. Owns `crates/render/src/sky.rs`. Accept: `cargo test -p
  engine-render --lib sky` green plus day-sweep 9 sun elevations holds
  horizon ramp within 0.02 with bake ms inside 0.4 and bake bytes x2,
  golden unchanged. F2P: Bruneton-bake fixture absent before. P2P: sky
  green x2 plus golden held. needs: RL4, GX7. (RES-STEAL 2026-10-05
  bevy-atmosphere r12; D7-frozen until BV3.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
- [ ] GM3 game: packaged Windows build runs clean (A8) — the packaged folder
  copied to `C:/e2040-clean/`, started with dev tools stripped from PATH, no
  source checkout, no Rust or Node, no model account and no dev env vars
  (owner 2026-09-24: agents prove it themselves). Size M. Accept: a scripted
  clean run plays 5 minutes and exits 0 with its log; a missing dependency
  fails it loudly. needs: GM2, WIN1, AS1.
  DECISION 2026-10-02 (GM3 tick): NOT ticked. The stock 30-frame PASS
  (`C:/Users/me/AppData/Local/Temp/opencode/gm3-clean30.log` 2026-09-29,
  ticks=30 frames=30, RTX 3050, REAL/REAL bundle fnv 624c5f0c9b87c208,
  same `target/as7-dist`) reverses PLAY_TIMEOUT-stock-x2 but does not meet
  this row's 5-minute acceptance, and its receipt is gone (a later
  18000-frame attempt restaged `C:/e2040-clean/` and FAILed PLAY_FAILED exit
  1, see `C:/e2040-clean/clean-run.log`).
  DEEP-DIVE 2026-10-03 (planner, root cause): the 18000-frame proof cannot
  fit a contended 45-min keeper — quiet-twin 600 frames took 83 s wall
  (~41 min projected, 2026-09-27) but the contended probe took 182.3 s
  (~91 min projected, BLOCKERS.md GM3 2026-10-02), so every shared-box run
  dies the identical keeper-timeout fingerprint; `--replace` on the shared
  `C:/e2040-clean/` then wiped the evidence; WIN13 deadline plumbing was
  proposed but never boarded, so no code path shortens the run. Next: ONE
  exclusive quiet-box pilot (no concurrent cargo or helpers on the box,
  scratch target, never `--replace` the shared `C:/e2040-clean/`) runs the
  full 5-minute proof `node tools/ship.mjs run --dist target/as7-dist
  --target C:/e2040-clean-gm3/ --frames 18000` (18000 frames, script budget
  ~9180000 ms, keeper 45 min; abort and file a BLOCKERS.md fingerprint row
  if the wall projects past the keeper instead of burning it; retry once on
  PLAY_TIMEOUT only on a proven-quiet box, then stop — second identical
  failure ends the step) and files its log plus `clean-run-receipt.json`
  under `target/gm3/`; tick only on CLEAN-RUN PASS exit 0 with both on
  disk. See BLOCKERS.md GM3. BLOCKED: GM3 until the stock 5-minute proof
  exits 0. Unblocks PQ3/A8.)
  2026-10-04 (GM3 worker): NOT ticked. Found and fixed a second wall bound
  (the window loop ignored `--deadline-ms`, so any run past 300 s failed
  itself), `tools/ship.mjs run` now passes it; fresh REAL package built in
  `target/gm3-dist` (arena fnv ec85b1d9f4b4bad4, bundle fnv 771a84a3ca55df01);
  the 30-frame clean-room run PASSES (`target/gm3/clean-run-30.log`). The
  18000-frame run started 05:15 (0.65 to 1.4 fps under the owner's CPU
  throttle plus the shared load) was killed at 35 min by the reaper's
  30-minute `smoke` class (`center/reaper.mjs`, the same kill as the
  2026-10-02 run at 63.7 min), so no 18000-frame run can finish while it is
  on: it needs a quiet hour with the reaper dry or exempting the run, see
  BLOCKERS.md GM3 for the command.
