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

- [ ] GX13 render: IBL release-capture close — pilot only, no lane, no
  code. Re-run the wave-51 `chart_target_size` fix proof on the
  reference GPU: release window build (arena scene, boosted clocks),
  1600x900 viewport downscales with no `exceeds edge 256` exit,
  material chart PNGs + per-pass ms + RA8 manifest posted under
  `target/pilot-gx13/`. Tier A. Size S. Owns nothing in `crates/`
  (evidence only). Accept: window presents N frames with 0 validation
  errors; reference-GPU chart beside the RL6 chart within tol; per-pass
  ms posted; Q1 V1-V3 consume the same set. needs: GX1, GX3, GX12,
  RL6, PF2. (GX3 VERDICT wave 56.) (Owner 2026-10-10: Q1 dropped
  from needs, a cycle; GX13 feeds Q1.)
- [ ] GX14 render: GTAO boosted-release proof — pilot-measurement only,
  no pass code: pinned release window build, arena 1080p, PF2 per-pass
  GPU stamps (gtao probe via PF2), table + RA8 manifest + llvmpipe note;
  miss fails with numbers, bars never loosened. Tier A. Size S. Owns
  receipt hunks beside `target/evidence/gx5/` (`gtao.rs` frozen, no
  donors). Accept: gtao p95 at most 0.8 ms at 1080p with adapter line
  or posted FAIL; 0 validation errors; `node tools/proof.mjs` PASS
  golden x2. needs: GX1, PF2, RL12. (GX5 VERDICT wave 56: GX5 is
  gtao.rs, not skinning; DROP 078-gx5-r2; wave-105 split: GX5 no
  longer blocks this row.)

- [ ] RL5 render: baked lightmaps with validity mask + SH probes for
  fighters reusing the `ibl.rs` SH path. Tier B. Size L. Owns
  `crates/render/src/baked_gi.rs`. Accept: bake identical bytes x2;
  60 fps arena + 8 fighters on the reference GPU (reference GPU).
  Deep 3D pass 2026-09-25: lightmap UV unwrap and packing happen in the cook (no overlapping charts, bake identical x2); Base tier only. needs: GX1, RL2, AS1.
  Wave 44 progress (stays open): --scene/--fps flags, 120-frame ARENA
  capture @ 0.4 fps debug (not 60), bake x2 green; honest gap: 5-tick cap
   repeats checkpoints past frame 24 — needs fixed-step capture pacing.
- [ ] RL14 render: fixed-step capture pacing + release receipt for RL5 —
  frame-accurate checkpoint pacing past frame 24 plus a release-build
  capture receipt (adapter/driver/power + fps). Tier B. Size S. Owns
  `target/evidence/rl5-pacing/` (receipt + pacing log). Accept: 120-frame
  ARENA capture with no repeated checkpoints, release receipt on the
  reference GPU. needs: GX1, RL5, RL12, PF2, CT7. (PLAN-RL5 wave 56:
  RL5 blocked by missing ref-GPU evidence + capture-pacing bug, not
  prereqs/scope.)
- [ ] RL11 render: look-gate capture bundle — one reference-GPU capture
  set closing the RL5/RL7/RP4 look gates together (material chart, day
  ramp, tunnel seal, shimmer scene) with RA8 manifests, driven by WIN1
  and Q1 on the same build. Tier A. Size S. Owns the capture-bundle
  hunks in `crates/render/src/` fixtures + `tools/` manifest glue
  (orchestrator lands `tools/`). Accept: llvmpipe parity within posted
  tolerance; 0 validation errors; reference-GPU captures posted with
  per-pass ms; Q1 V1-V3 go green on the bundle. needs: GX1, RL5, RL7,
  RP4. (PLN26 render-look.md, reviewed 2026-09-25.)
- [ ] RL13 render: glTF-anchored material/exposure chart for GX3 IBL
  parity (READER-LOOK53 card 3). Tier A. Size S. Owns chart fixture +
  `crates/render/src/` IBL bind hunks. Accept: llvmpipe parity within
  tol + reference-GPU chart with RA8 manifests. needs: GX1, RL2, RL6,
  Q1. (LOOK53.)
- [ ] RL16 render: twin material + IBL bind — the WIN1 window twin forward
  path samples the GX3 IBL ambient plus RL6 layered sand/cloth response
  instead of flat vertex-color brown, as bind glue only in a new
  `crates/render/src/twin_look.rs` (pass code and `arena_day.rs` frozen).
  Tier A. Size S. Owns `crates/render/src/twin_look.rs`. Accept: capture
  pair before `target/pilot-gx13/owner-arena/frame_000015.png` vs after
  on the reference GPU scores higher by this seat on materials + light
  with 0 validation errors; golden unchanged. needs: GX1, GX3, RL6.
  (LOOK-ART 2026-09-28.)
  LATER: sprint-2026-10-04 frozen by owner decision D7 (docs/FOCUS.md) until the first real Bevy frame lands; the loop skips it; unpark by deleting this line.
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
- [ ] RP8b render: variance-clip re-spike, ONLY if the lead overturns
  the RP8a REJECT — re-run ghost + shimmer fixtures with the named
  delta beside `crates/render/src/post.rs` (resolve frozen).
  Tier B. Size S. Accept: ghost-trail px cut ≥30% with shimmer
  ≤0.06 and ms +≤5%, or second REJECT with numbers. needs: RP8a.
  (A7, 2026-10-02.)
- [ ] RL21 render: twin arena-bundle bind — the WIN1 twin forward path
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
- [ ] RL22 render: editor viewport day-rig bind — the editor viewport
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
- [ ] RL23 render: twin grade + exposure bind — the WIN1 twin forward path
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
- [ ] RL24 render: viewport starburst source removal — name
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
- [ ] RL25 render: twin dome facet removal — the fullscreen
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
- [ ] RP11 render: twin silhouette fringe removal — a thin
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
- [ ] RL26 render: twin 8-fighter draw bind — the WIN1 twin
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
- [ ] RP10 render: twin frame-advance proof — the WIN1 twin presents 30
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
- [ ] RL27 render: cascaded shadow maps for the arena-day sun — cascade-split
  selection + per-cascade shadow-map sizing ported from `FyroxEngine/Fyrox`
  `fyrox-impl/src/renderer/shadow/csm.rs` (license MIT, adapt with
  attribution). Tier B. Size M. Owns `crates/render/src/shadow_cascade.rs`.
  Accept: split distances pinned as data; day-sweep fixture x2 byte-identical;
  llvmpipe parity within tol; golden `d57b5c8ae3096614` held. F2P:
  `cargo test -p engine-render --lib shadow_cascade` (fails before: day-rig
  sun uses one split, no cascade unit). P2P:
   `cargo test -p engine-render --lib ibl` stays green. needs: GX1, RL2.
   (STEAL 2026-10-02 engine2040.md #1; EB-2026-10-02-S93.)
- [ ] RL29 render: RL5 boosted-release capture pilot — evidence only under
  `target/rl29/`, no `crates/` hunks (code receipt
  `rl5_8fighter_capture_receipt_x2` at `crates/render/src/baked_gi.rs:1381`
  plus `rl5_ra1_receipt_consumes_the_bind` at `baked_gi.rs:1427` already
  green headless; 60 fps + 8-fighter capture missing). Tier B. Size S.
  Owns `target/rl29/` (pilot outputs only). Accept: release pilot
  `--scene arena --fighters 8 --frames 120 --exit` posts 3 PNGs + RA8
  manifests + PF2 ms + VRAM/draws; bake bytes x2; 60 fps on reference
  GPU. F2P: 8-fighter capture set absent before. P2P: 3 PNGs posted,
  manifests + ms rows posted, bake bytes x2 identical. needs: RL5, RL28,
  PF2.
- [ ] RL30 render: RL27 day-sweep parity pilot — evidence only under
  `target/rl30/` (code owns new `crates/render/src/shadow_cascade.rs`,
  absent on master; `SUN_CASCADES = 3` + `CASCADE_LAMBDA = 0.5` at
  `crates/render/src/shadow_cluster.rs:20-23`). Tier B. Size S. Owns
  `target/rl30/` (pilot outputs only). Accept: day-sweep A/B x2
  identical; llvmpipe parity in tol; 0 errors; golden held; 3 splits
  posted. F2P: cascade day-sweep set absent before. P2P: A/B x2
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
