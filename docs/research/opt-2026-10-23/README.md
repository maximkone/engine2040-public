# opt-2026-10-23 — research cards for the five finish gaps

One usable card per remaining blocker named in the finish audit
(`docs/plan/finish-line.md:48-62`). Ranked by the audit's order, then by the audit's own
next proof inside that blocker.

Every code line was checked in today's tree (2026-10-23, master). Live reads are named;
hunt-doc-only claims say so. Nothing here edits `crates/**` or `tools/**`.

**Rank**

| # | Card | Audit blocker | Why this rank | First proof | Cost |
|---|---|---|---|---|---|
| 1 | LF8: skin, metal, cloth shade apart | #1 look and materials | The audit's own next proof for #1 is "LF8 splits the BRDF" | Per-kind pixel diff in the part's rect; stone equal | M |
| 2 | Post remainder: auto-exposure on the live frame | #1 look and materials | Same blocker. Bloom (RL36) is in the tree now, TAA (LF5c) landed, grade (RL34) landed; the meter is the last built-but-off look lever, with lane numbers already green | Clipped-highlight share cut ≥15% vs fixed; flicker ≤0.5 EV/frame | S |
| 3 | G2: 1080p speed and the input tail | #2 1080p speed | Blocker #2, and two of its three asks already have code: the check times 1080p since `668aa4756`, and the 114.9 ms is the capture run's present leg, not input | 1920x1080 receipt at 60 fps p50 with input p95 ≤50 ms from the timing leg | S |
| 4 | G11: physical picture | #7 physical picture | Three open legs are named exactly; dust is a one-call wiring; shards are visual-only | Gate-scale capture: 8 ragdolls, 200 debris, 6 limbs, inside budget | M |
| 5 | E22/GM3: packaged frame score | #8 packaged run | Needs the RTX 3050, not new code; the mirror stays green with a labelled skip | `adapter="NVIDIA GeForce RTX 3050..."`, `frames>0`, frame score printed | S |

Read the cards in this order if you follow the audit. If you want the largest pixel change
per seat-hour, take #2 first: it is one enable line plus one receipt, and its lane numbers
already read a 0.727 → 0.000 clipped-highlight share.

**Two naming traps**

- Audit #8 says "E22". That is the *gate* in `docs/plan/finish-line.md`, not the board row
  `E22` (look-gate), which is closed and obsolete.
- RL36 bloom and LF5c TAA are in the working tree now but not committed
  (`crates/render/src/bloom_dual.rs`, `crates/render/src/present/gpu_present/bloom_dual.rs`,
  the three `shaders/post/bloom_dual_*.wgsl`, plus modified `post/chain.rs`, `blit.wgsl`,
  `taa_resolve.rs`, `live.rs`, `submit.rs`). Do not touch those hunks.

---

## 1. LF8 — skin, metal and cloth shade apart

**Gap.** One BRDF serves every part. The audit reads
`target/reports/product/runs/243211c27-20261023T1138Z/frame_000002.png` as flat ochre with
weak material separation, and "skin, metal and cloth share one BRDF"
(`docs/plan/finish-line.md:52`). Row `LF8` is open, needs `LF2` (landed).

**Owner today (verified).**

- Shader: `crates/render/src/shaders/forward.wgsl:63-70` `MaterialU` (base_color, metallic,
  roughness, exposure, `_pad0` at 24, `_pad1` at 28); `:641 d_ggx`; `:650
  v_smith_ggx_correlated`; `:669 fd_burley`; `:704 direct_light` — one lobe, no tangent
  parameter; `:784 eg12_world_normal` already builds the T/B frame but returns only the normal.
  The vertex carries a tangent at `:403` and `VOut` keeps it (`:468`, `:486`).
- Rust uniform: `crates/render/src/v1.rs:747 MaterialUniform` (24 B, `size_of == 24`),
  `:765 to_array` writes a 32-byte image with zeros at 24..32, `:860 build_material`.
- Bind image: `crates/render/src/present/gpu_present/submit.rs:1896 MATERIAL_BYTES = 32`,
  `:1902` the 24-byte assert. Per-draw sites: `submit.rs:964`, `live.rs:490`.
- Game: `crates/game/src/fighter_look.rs:1739` builds one `PbrMaterial` per part; no kind.

**Donor (live-read today).** `google/filament@479ee3548410f21c038e50f1fe9335fe09eb433b`,
Apache-2.0 (`LICENSE` read today, 11,362 B).

- `shaders/src/surface_brdf.fs:81 D_GGX_Anisotropic`, `:122
  V_SmithGGXCorrelated_Anisotropic`, `:94 D_Charlie`, `:139 V_Neubelt`, `:353
  distributionCloth`, `:359 visibilityCloth`.
- `shaders/src/surface_shading_model_cloth.fs:18-35`: spec `(D*V)*F`, diffuse Burley and
  **not** Fresnel-multiplied; optional `Fd_Wrap(NoL, 0.5)`.
- `shaders/src/surface_shading_model_subsurface.fs:14-18`:
  `forwardScatter = exp2((saturate(dot(V,-L)) - 1) * power)`,
  `backScatter = (NoL*thickness + (1 - thickness)) * 0.5`,
  `subsurface = mix(backScatter, 1, forwardScatter) * (1 - thickness)`,
  `color = subsurfaceColor * (subsurface * Fd_Lambert())`, then
  `Fd = mix(color, diffuseColor * Fd_Lambert(), pow4(occlusion))`.
- Filament's `at/ab` are alpha (perceptual roughness squared):
  `surface_shading_model_standard.fs:59-63` passes
  `at = max(alpha*(1+anisotropy), MIN)`, `ab = max(alpha*(1-anisotropy), MIN)`. Our
  `forward.wgsl:635 perceptual_roughness_to_roughness` is the same alpha.

**Buffer shape (no size change).** Keep the 32-byte image. Put `kind` in bytes 24..28 and
one parameter in 28..32 (`skin thickness` or `metal anisotropy`). The Rust struct stays 24 B;
the two words are written in `to_array` from a kind looked up by `draw.mesh`, so the
`:1902` assert and `MATERIAL_BYTES` do not move. Default kind 0 must take the old path
byte-for-byte, so stone is unchanged by construction.

**Algorithm, exact.**

- Metal (anisotropic GGX, Filament `surface_brdf.fs:81-131`):
  `a2 = at*ab`, `d = vec3(ab*ToH, at*BoH, a2*NoH)`, `b2 = a2/dot(d,d)`,
  `D = a2*b2*b2/PI`; `lambdaV = NoL*length(vec3(at*ToV, ab*BoV, NoV))`,
  `lambdaL = NoV*length(vec3(at*ToL, ab*BoL, NoL))`, `V = 0.5/(lambdaV+lambdaL)`.
  T/B come from the existing tangent (orthogonalize against N, `B = w*(N x T)`, the same
  steps as `eg12_world_normal`).
- Cloth (Charlie sheen): `invA = 1/alpha`, `sin2h = max(1-NoH^2, 0.0078125)`,
  `D = (2+invA)*pow(sin2h, invA*0.5)/(2*PI)`; `V = 1/(4*(NoL+NoV-NoL*NoV))`;
  `F = f0` (0.04); diffuse `fd_burley`, no Fresnel multiply; optional `Fd_Wrap(NoL, 0.5)`.
- Skin: wrap + thickness as above, applied to the diffuse lobe only. Keep the specular lobe
  as today.

**Kind source.** No material name survives the body cook:
`crates/assets/src/cooked.rs:92 CookedMaterial` holds factors plus texture refs only, and
`crates/assets/src/body_import.rs:95` keeps that struct. Two honest routes:
(a) add `kind: u8` to `CookedMaterial` and set it at import from the glTF material name
(`crates/material/src/gltf_import.rs:327 record_name` already reads the name), or
(b) derive from factors as a stopgap. Prefer (a): the acceptance needs skin and cloth
apart, and factors cannot tell cloth from skin.

**Papers.**

- arXiv 2203.11904 (Bringing Linearly Transformed Cosines to Anisotropic GGX) proves the LTC
  approximation existed **only for isotropic GGX**, so anisotropic shading is a real gap.
  It is about area lights, so it is ideas-only for our single directional sun; do not port
  its `8^4` LUT.
- arXiv 2606.27604 (Spectral Subsurface Scattering from RGB via Biophysical Skin Inversion)
  proves physical skin needs multi-layer spectral inversion for path tracing. It **rejects**
  any claim that a one-float wrap is physically correct. Write "a game skin lobe", never
  "physically accurate skin".
- No paper for Charlie sheen was found. Filament is the authority; say so.

**Proof.**

- Metric: pixels that move when the kind changes, inside the part's screen rect, from a
  capture pair. Stone must be **exactly 0**.
- Command shape: add `--no-material-kind` (same shape as `--no-break-look`
  `crates/game/src/bin/arena.rs:229` and `--no-ragdoll` `:233`), run
  `arena.exe --scene arena --fighters 8 --frames 60 --exit --capture-dir <a>` and the same
  with the flag off, then diff the PNGs and clip to the rect the `arena view:` line already
  prints per body.
- Unit: `crates/render/src/v1.rs:1054 shade_pbr` (CPU mirror) per kind differs; `cargo test
  -p engine-render --lib` green; golden untouched (presentation only).

**Plan (one seat).**

1. Add `MaterialKind` (stone 0, skin 1, metal 2, cloth 3) and the `kind` byte to
   `CookedMaterial`; map names at import; keep stone the default.
2. Thread it to the renderer as a `MeshKey -> kind` table on `Presenter` (one setter, called
   where `fighter_look.rs:1739` builds the material). No `DrawItem` change, so the 30+
   literals stay untouched.
3. Write bytes 24..28 in `to_array` from that table; add a unit test that stone's 32 bytes
   are unchanged.
4. Split `direct_light`: cloth first (no tangent), then skin, then metal (tangent frame).
   Land each separately.
5. Add the on/off flag and the pixel-diff proof; capture the 8-fighter frame.

**Risks.** `submit.rs` and `live.rs` are dirty from the RL36/LF5c lane today; land after it
commits, or touch only your added hunk. Do not change `MATERIAL_BYTES` or the stride. Do not
let the default path change by one bit; LF15's look score would read it as a regression.

---

## 2. Post remainder — put the built auto-exposure on the live frame

**Gap.** The audit's look blocker names "the run's live list has no bloom or TAA"
(`docs/plan/finish-line.md:52`). Both are moving now: RL36 bloom is in the tree, LF5c TAA
resolve landed (`9fcfb1651`), RL34 grade/tonemap landed. What is still off is the
percentile-trimmed auto-exposure: it is built, proven in-lane, and **not enabled on the
window path**. There is also no post/exposure column on the per-pass line, so RL18's own
ms gate cannot be read.

**Owner today (verified).**

- CPU meter: `crates/render/src/post.rs:183 pub(crate) mod auto_exposure`; constants in
  `crates/render/src/post/auto_exposure.rs:25` (64 bins), `:32` trim low 0.10, `:35` trim
  high 0.95, `:41` flicker gate, `:71` `adapt_ev_per_s`.
- GPU meter: `crates/render/src/present/gpu_present/auto_exposure_gpu.rs:45,68`, kernel
  `crates/render/src/shaders/auto_exposure_hist.wgsl`.
- Live frame: `crates/render/src/present/gpu_present/live.rs:1221` meter step, `:1381` the
  adapted multiplier written into the frame.
- Enable: only the console knob `crates/game/src/bin/console/mod.rs:155`. The env switch
  `E2040_AUTO_EXPOSURE` is read only by the offscreen smoke twin
  (`crates/game/src/windowed.rs:145`, `:1655`). The window match therefore rides fixed 1.0
  (`crates/game/src/bin/arena.rs:433-436`, `FrameLight.exposure` at
  `crates/game/src/view_truth.rs:168`).
- Per-pass line: `crates/game/src/bin/arena_frame/mod.rs:644` lists sky, layers, particles,
  decals, ibl, fog, gtao, shadow, forward — no post/exposure.

**Lane evidence (board row RL18, 2026-10-19).** Day sweep clipped share 0.727 → 0.000
(improvement 1.000 against a 0.15 gate); torch grid 1.8698 → 1.7365 stops (0.133 EV);
worst inter-frame step 0.05 stops against a 0.5 gate; GPU vs CPU parity 0.000000. The row is
blocked only on: per-pass ms with an adapter line, llvmpipe parity, Q1 captures, and the
live match riding the adapted EV.

**Papers.**

- arXiv 1502.06080 (Intra-and-Inter-Constraint-based Video Enhancement based on Piecewise
  Tone Mapping) adds **inter-frame constraints** to keep temporal quality consistent. It
  proves flicker is the right failure mode to measure for a per-frame tone curve.
- arXiv 2303.07327 (Unsupervised HDR Image and Video Tone Mapping via Contrastive Learning)
  names a temporal-feature-replaced module to improve temporal consistency of video
  tone-mapping. Use it as evidence that temporal consistency is a measured term; do not port
  the network.
- No paper was found that meters game HDR at 60 Hz. The row's kill metrics are the bar:
  clipped share improves ≥15%, flicker ≤0.5 EV/frame, ms +≤5%. State that plainly.

**Shape.** 64 log-luma bins over −12..+8 EV (0.3125 stops), 10–95% trim, adapted EV at the
row's stops/second, one-frame-late readback (the frame never waits). The multiplier already
lands in the frame; only the switch and the receipt are missing.

**Proof.**

- Run the window twice on the same 8-fighter line: fixed 1.0 and `E2040_AUTO_EXPOSURE=1`
  (add the env read to the window path next to the smoke twin's read, one line).
- Metric: clipped-highlight share in the capture (pixels with any channel ≥250), same PNG
  decoder as `tools/iqa-gate.mjs`; EV delta between consecutive frames (the renderer can
  print `auto_exposure_ev` per frame).
- Bar: relative clipped-share cut ≥15%, step ≤0.5 EV/frame, post ms within +5% of fixed.
- Add one `post` column to the per-pass line so the ms leg exists at all.

**Plan (one seat).**

1. Read `E2040_AUTO_EXPOSURE` on the window path (same pure parser, `windowed.rs:149`).
2. Add `post=` to `arena_frame/mod.rs:644` from the presenter's exposure timer (one field).
3. Run the pair at 1280x720 and at 1920x1080; file the clipped-share and EV-step numbers.
4. Only then flip the default (or keep it opt-in and record the owner's call).

**Risks.** `arena_frame/mod.rs`, `live.rs`, `blit.wgsl` are dirty from other lanes. Do not
retune `AUTO_TRIM_*` or the flicker gate to pass. The chain's EV and `FrameLight.exposure`
multiply; keep one of them fixed while measuring the other.

---

## 3. G2 — 1080p speed, and the input tail that is already explained

**Gap.** The audit reads 60.5 fps p50 at 1280x720, p99 21.254 ms, GPU p50 2.255 ms, with
1920x1080 unmeasured; it also reads "input p95 114.9 ms (FAIL 50)" and "80 slow frames"
(`docs/plan/finish-line.md:53`). The next proof it asks for: a product check at 1920x1080
with 8 fighters and pass floors; explain or remove the input tail.

**What is already built.** Two of the three asks have code. The check times the finish size:
`tools/product-check.mjs:44 TIMING_SIZE = "1920x1080"`, `:45 LOOK_SIZE = "1280x720"`, both
runs pass `--size` (look `:348`, timing `:351`), and the latency floors gate the timing leg
only with a size-aware fps reference (`:243-249`, `:161`) — all in `668aa4756` on 2026-10-23.
The player latency floors are real: `tools/product-bar.json:12 input_p95_ms_max: 50`, `:13
present_p95_ms_max: 50`. So the remaining work is one run on the RTX 3050 and a reading of
the two legs, not new tooling.

**What the receipts say today (run `243211c27`, both legs at 1280x720, read here).** Its
per-pass line says `resolution=1280x720`; the run predates `668aa4756`, so no 1080p leg
exists yet.

- `timing.txt` (no captures): interval p50 **16.540 ms**, p99 **21.254 ms**, GPU p50
  **2.255 ms** (forward 1.144, sky 0.069, blit 0.680), CPU p50 4.209, p99 21.116,
  **input p95 3.655 ms**, present p95 5.286 ms, slow_frames=7.
- `look.txt` (captures every 60 frames): input p95 4.617 / 4.380 ms,
  **present p95 117.137 / 114.867 ms**, slow_frames=80, with
  `slowest=0:556.828ms/present=547.836ms`, `240:230.438ms/present=229.346ms`,
  `60:160.484ms/present=158.608ms`.
- So the audit's 114.9 ms is the **capture run's present leg**, not the input leg. The game's
  input leg is 3.7 ms on the timing run. The capture's offscreen render plus PNG write sits
  inside the present leg on indices 0, 60, 120, ... and inflates it. The check now prints
  that number on its own line, labelled a capture run and never gated
  (`tools/product-check.mjs:308`).
- "80 slow frames" is not 80 dropped frames. `crates/game/src/bin/arena_frame/trace.rs:160`
  defines slow as **work wall > 2x the trace's work p50** (`SLOW_FRAME_FACTOR = 2`, function
  at `:164`). A 2 ms work p50 makes every 4+ ms frame "slow".

**Owner today (verified).**

- `--size 1920x1080`: `crates/game/src/windowed.rs:402 parse_window_size`, `:434-499`
  (`window_size()` at `:498`).
- Both legs pass a size today: `tools/product-check.mjs:348` (look, `LOOK_SIZE`) and `:351`
  (timing, `TIMING_SIZE`), from `668aa4756`. The latency gate reads the timing run only
  (`:243-249`).
- The per-pass line takes its resolution from the newest folded frame
  (`crates/game/src/bin/arena_frame/mod.rs:591`), so a run that boots at 720p but resizes
  must be labelled by the frame's size, not the boot size.
- The frame trace is already written and printed (`arena_frame/mod.rs:812-817`).

**Projection (label it a projection).** 1920x1080 is 2.25x the pixels of 1280x720. GPU p50
2.255 ms scales to about 5.1 ms; CPU p50 4.209 ms is unchanged; total about 9.3 ms of the
16.67 ms budget. p50 should hold 60 fps. The risk is the p99 tail, not the p50.

**Papers.**

- arXiv 2306.01691 (The Influence of Variable Frame Timing on First-Person Gaming): 4–12 ms
  frame-time variation changes perceived smoothness, but does **not** change FPS task
  completion time. So report p99 and the trace; do not claim a performance effect.
- arXiv 2608.20861 (Beyond Mean Frametime): means, percentiles and histograms discard
  temporal order; a time-series signature separates groups that distribution-only summaries
  miss. We already keep per-frame rows; keep them as the signature, not just p50/p99.

**Proof.**

- Command: `node tools/product-check.mjs` on the RTX 3050. It builds when stale, runs the
  look leg at 1280x720 with captures and the timing leg at 1920x1080, and writes
  `target/reports/product/latest.json`, `latest.md`, `history.jsonl`.
- By hand, if the build slot is busy: the same two lines it runs
  (`tools/product-check.mjs:348`, `:351`), reading `timing.txt` for speed and `look.txt` for
  pixels.
- Bar: p50 ≥60 fps at 1920x1080 (target in `tools/product-bar.json:23`), GPU total and each
  pass under the pass bar; input p95 and present p95 ≤50 ms from the timing leg
  (`tools/product-bar.json:12-13`). Quote the look leg only for pixels, or label it a
  capture leg.
- If the p50 fails: use the existing `ScaleController`
  (`crates/render/src/aa_scale.rs:720,783`) and the integer governor
  (`crates/render/src/dynres_gov.rs:1-6`) with EASU, not a new system.
- When the receipt lands, delete "with 1920x1080 unmeasured" from
  `docs/plan/finish-line.md:53` and file the numbers; no tool change is owed.

**Plan (one seat).**

1. Run `node tools/product-check.mjs` (or its two lines by hand); file both receipts.
2. Read `latest.json`: `timing.size` must read `1920x1080`, and `timing.latency` carries both
   p95 legs. Name the top three work spikes by phase (`sim`, `present`, `fighters`, `upload`).
3. If p50 holds, only the audit line and the report change. No code.
4. Explain the capture leg in the report: quote the timing leg for input, the look leg for
   pixels. Do not merge the two legs into one number.
5. If p50 misses, lower the internal scale through the governor and re-run; never lower a
   floor.

**Risks.** `arena_frame/mod.rs` and `crates/game/src/bin/arena.rs` are dirty from the C13
profiler lane. Do not quote `sky=86.984ms` from the look run: it is a probe-frame value
(`probe_frames=3`), not the live sky. `present_mode=Fifo` pins the p50 to the display period,
so "60.5 fps" is the period, not headroom.

---

## 4. G11 — the physical picture: shards, more than one chunk, dust

**Gap.** The audit reads RZ1/RZ2/RZ3/RZ4/RZ9 landed, but "a hit frees one chunk and chunks
still draw as boxes, the dust GPU emitter is missing; the latest run shows ragdolls=2,
debris=1" (`docs/plan/finish-line.md:58`). The RZ4 row names the same three open legs.

**Owner today (verified).**

- Boxes: `crates/game/src/break_window.rs:400 baked_box`; `BreakScene::push` (`:207`) uploads
  a box per fresh piece (call at `:222`). `PieceMesh` carries only `key` and `half`
  (`crates/game/src/break_look.rs:123`). The chunk data is an AABB plus mass:
  `crates/destruction/src/bake3d.rs:7-21 Chunk3`.
- The crate: 8 pieces (`crates/arena/src/lib.rs:2187 CRATE_PIECES`), baked by
  `BondedCollection::crate_prop` and handed out by `crate_rest` (`:2213`).
- One chunk per hit: `crates/arena/src/lib.rs:1604-1615` ("every sword hit strikes the chunk
  nearest the victim"), `nearest_chunk` at `:2243`.
- Dust: `BreakDust` (`break_look.rs:289`) and `take_dust` (`:519`) exist, and **no
  production caller** reads `take_dust` today (only tests). The GPU kind exists:
  `crates/render/src/particles_gpu.rs:297 EmitterKind::Dust`; the render already has the
  call `Presenter::fight_burst(kind, at_m, dir, count)`
  (`crates/render/src/present/gpu_present/fight_fx.rs:26`), and the knockdown path already
  feeds dust (`crates/game/src/bin/arena_frame/fight_fx.rs:141`).
- Blast: `BreakScene::observe_blast` (`break_window.rs:278`) also has no production caller
  (only the test at `:622`), so weapon blasts never raise a cloud.

**Donor (live-read today).** `dgreenheck/OpenFracture@6d0ddff64173193f024638c2de2dbeb8a2525965`,
MIT (Copyright 2021 Daniel Greenheck; `LICENSE.md`, `Runtime/Scripts/Fragment/Fragmenter.cs`
12,084 B and `MeshSlicer.cs` 16,645 B read today). Use it for the inside face and UV remap
when a shard gets a real cut. It is not needed for the first bevel landing.

**Paper.** arXiv 2310.13344 (DeepFracture) states that Voronoi and pre-fractured patterns are
effective for real time but **fail to incorporate collision conditions at runtime**, and
proposes a learned model for that. It supports pre-fractured visual shards selected at
runtime; it rejects runtime collision-driven geometry as the cheap route.

**Three landings, cheapest first.**

- **S — dust wiring (no sim change).** Where the tick's breaks are folded, read
  `scene.take_dust()` once and call
  `presenter.fight_burst(EmitterKind::Dust, dust.at_m, [0,1,0], dust.puffs)`. Wire
  `observe_blast` on the weapon-blast path if that path already has an authoritative blast;
  otherwise leave it and say so. Proof: `arena break: ... dust=N` grows on a break tick;
  a quiet tick adds none; golden unchanged.
- **S — shard mesh instead of a box (no sim change).** Generate one deterministic shard mesh
  per chunk from `(hx, hy, hz, seed)`: start from the `Chunk3` AABB, chamfer or jitter the
  eight corners inward by a seeded fraction, keep every vertex inside the AABB, append the
  inside faces. Upload once per piece key (8 uploads at window start), then the existing
  transform per piece. Bounds must stay inside the AABB so the physics picture never pokes
  out. Proof: the same seed gives byte-identical `MeshData` twice; `upload_mesh` validates;
  a box-vs-shard capture pair differs in the pieces' rect; the golden is untouched.
- **M — true cells (still visual).** Split each AABB into 2–4 cells with the existing
  `fracture_at` / `csg` (`crates/destruction/src/fracture.rs:412`, `crates/destruction/src/csg.rs:70`),
  keep one body per chunk, draw the cells under that chunk's transform. This is the
  OpenFracture shape (inside faces, UV remap) without touching the authority.
- **M — more than one chunk per hit (sim change, golden moves).** Apply the impulse to every
  chunk within the strike reach in index order instead of only the nearest. The recipe is
  already in a test: `break_look.rs:886-892` drives every chunk within `STRIKE_REACH_M` and
  asserts frees. This changes the match hash, so it needs an explicit golden receipt
  (`docs/HASH-LAW.md`); do not slip it in with the picture work.

**Gate-scale proof.** The audit asks for 8 ragdolls, 200 debris and 6 limbs inside budget.
Stage an 8-fighter line, trigger the multi-chunk break and the limb cuts, capture at a known
tick, and read `arena break: breaks= debris= awake=` plus the `arena per-pass` particles
column. The budget bar is RZ1's: 8 ragdolls plus 200 debris step under 4 ms per tick
(release). Keep the FX layer off the sim; the sim hash must be equal with the picture on and
off (`--no-break-look` `arena.rs:229` is the off switch).

**Plan (one seat).**

1. Dust wiring + one receipt (S, no golden).
2. Beveled shard mesh + determinism/bounds tests (S, no golden).
3. Box-vs-shard capture pair, one PNG opened (S).
4. Multi-chunk impulse behind a constant; run the golden, write the receipt (M).
5. Cell split per AABB if close-ups still read as boxes (M).
6. Gate-scale capture and the budget line (M).

**Risks.** `crates/game/src/bin/arena.rs` (C13 profiler) and `crates/arena/src/lib.rs`
(hurt constants) are dirty today. Do not touch their hunks. The multi-chunk step is the only
one that can move `d57b5c8ae3096614`; everything else is presentation and must leave it alone.

---

## 5. E22/GM3 — the packaged frame score

**Gap.** The mirror ship check is green twice, but its frame-score leg is a named skip
(`adapter=none`), and GM3 is open (`docs/plan/finish-line.md:59`). The next proof: a
clean-folder 5-minute run on the RTX 3050 with a real adapter and a frame score.

**Owner today (verified).**

- `tools/ship-run.mjs:21-28` documents the skip; `:123 adapterName`, `:134 noAdapter`,
  `:140 adapterLabel` (WARP/software/llvmpipe → `warp`); the skip line is `:491`:
  `no-hardware-adapter` for `none`/`warp`, otherwise `frame-score-not-wired`. So even a real
  adapter is skipped today; the scorer is the missing half.
- `.github/workflows/ship-check.yml:131-155` runs the clean-folder proof and the headless
  600-tick game twice; the runner has no adapter.
- A real reference set already exists: `content/reference/index.json` native rows at
  1280x720 with `pixel_fnv`, `source` naming
  `adapter=NVIDIA GeForce RTX 3050 6GB Laptop GPU driver=32.0.16.1062`.
- Capture path: `arena.exe --reference-set --reference-tier native`
  (`crates/game/src/bin/arena_capture/capture/reference_tier.rs:68-72`, `:152-156`; a machine
  with no adapter refuses loudly). Verifier: `tools/reference-captures.mjs --capture --tier
  native` (`:16-19`). Scorer: `tools/iqa-gate.mjs --threshold N <reference.png>
  <capture.png>` (book line `docs/book/ref-tools.md:341`), or `scorePair` from the same
  module.
- Software opt-in: `crates/render/src/adapter.rs:63 allow_software` (`E2040_ALLOW_SOFTWARE=1`
  only), refusal text at `:187`.

**Right route, two legs.**

- **Real leg (closes the audit).** On this PC, from the clean folder, run the shipped
  `arena.exe --reference-set --reference-tier native --reference-dir <tmp>` (offscreen twin,
  no window), then score one shot against `content/reference/native/arena600-t000.png` with
  `iqa-gate`. Print one line:
  `SHIP-FRAME adapter="NVIDIA GeForce RTX 3050 6GB Laptop GPU" resolution=1280x720
  score=<ssim_x10000> flip=<flip_x10000> verdict=pass`.
  Wire the same check into `ship-run.mjs` after the game leg, gated on `adapterLabel` not
  being `none`/`warp`; that removes the `frame-score-not-wired` reason.
- **Software leg (CI-visible, labelled).** Set `E2040_ALLOW_SOFTWARE=1`, run the same
  offscreen capture under WARP, print `adapter=warp reason=software-adapter`. It proves the
  pixels and the path, never the hardware. Keep the mirror's skip line exactly as it is.

**Bar.** Adapter named, `frames>0`, one score at or above the look gate's threshold (SSIM, or
FLIP ≤1200, the existing `look_flip_max` floor in `tools/product-bar.json`), receipt
committed. The 5-minute GM3 clean run is the other half: `tools/ship.mjs run --dist
<dist> --target C:/e2040-clean-gm3/ --frames 18000` on a quiet box; the row says it dies on a
contended keeper, so it is an ops slot, not code.

**Plan (one seat).**

1. Run the real leg by hand from a clean folder; file the score and the adapter line.
2. Add the score step to `ship-run.mjs` behind the adapter label; unit-test the pass/fail
   parsing.
3. Add the software leg with the `warp` label and a test that it never prints a hardware
   name.
4. File the GM3 5-minute receipt on a quiet box; do not `--replace` the shared clean folder.

**Research.** None needed. This is measurement plumbing; no paper improves it. Do not call a
software image a hardware pass.

**Landed 2026-10-23.** `fbb0df931` wired the frame leg into `tools/ship-run.mjs` (the look gate's own SSIM 9800 or FLIP <= 1200; a software adapter is labelled `verdict=software` and never a pass; 32 tool tests pass). `abd15596c` records the real reading on the RTX 3050 through the same `runFrameLeg` on the 17:59 release binary: `SHIP-FRAME adapter="NVIDIA GeForce RTX 3050 6GB Laptop GPU" resolution=1280x720 score=9998 flip=1 verdict=pass`. Still absent: the same line inside a clean-folder ship receipt (a fresh shipped binary is needed; `crates/game/src/bin/arena.rs` was C13's at the time, and the 17:59 exe predates the `--headless` route) and the GM3 5-minute quiet-box run. The mirror's `adapter=none` skip line is unchanged.

---

## What is genuinely missing

- **No paper for Charlie sheen or for a one-float skin wrap.** Filament is the donor and the
  authority; 2606.27604 only bounds the claim. Do not write "physically correct skin".
- **No game-specific auto-exposure metering paper found.** 1502.06080 and 2303.07327 justify
  measuring flicker, not a method. RL18's own kill metrics are the bar.
- **No fracture method paper for our scale.** 2310.13344 rejects runtime collision-driven
  shapes as the cheap route; the S/M shard landings are engineering, not research.
- **G2 and E22 need no research.** Their answers are in the repo receipts and the tools; the
  cited papers only justify reporting p99 with the trace and naming the adapter.

## Donors and papers read

| Item | Ref | License | Read | Used by |
|---|---|---|---|---|
| google/filament | `479ee3548410f21c038e50f1fe9335fe09eb433b` | Apache-2.0 | `LICENSE`, `surface_brdf.fs`, `surface_shading_model_standard.fs`, `surface_shading_model_subsurface.fs`, `surface_shading_model_cloth.fs` live today | LF8 |
| dgreenheck/OpenFracture | `6d0ddff64173193f024638c2de2dbeb8a2525965` | MIT (2021 Daniel Greenheck) | `LICENSE.md`, `Fragmenter.cs`, `MeshSlicer.cs` live today | G11 M step |
| djeedai/bevy_hanabi | `77630ff1eed8b104f34e91d23e9440428dc5ab01` | MIT OR Apache-2.0 | `LICENSE-MIT` live today; render dir listed (`.wesl`, renamed from the hunt doc's `.wgsl`) | pointer only, NS11 |
| arXiv 2203.11904 | v2 | — | title and abstract today | LF8 metal |
| arXiv 2606.27604 | v1 | — | title and abstract today | LF8 skin bound |
| arXiv 2306.01691 | v1 | — | title and abstract today | G2 reporting |
| arXiv 2608.20861 | v1 | — | title and abstract today | G2 trace |
| arXiv 1502.06080 | v1 | — | title and abstract today | post flicker |
| arXiv 2303.07327 | v2 | — | title and abstract today | post flicker |
| arXiv 2310.13344 | v2 | — | title and abstract today | G11 choice |

## Receipts read today

- `target/reports/product/runs/243211c27-20261023T1138Z/look.txt`, `timing.txt`,
  `per_pass_ms.json` — the 720p numbers in card 3.
- `docs/research/gpu-window-2026-10-22/product-run/look.txt`, `timing.txt` — the same split
  on an earlier run (input p95 4.9 ms quiet, present p95 128/285 ms with captures).
- `content/reference/index.json` — native 1280x720 shots with the RTX 3050 source line.

## Do not touch (dirty from other lanes today)

- RL36 bloom / LF5c TAA: `crates/render/src/present/gpu_present/{submit,live,taa_resolve}.rs`,
  `crates/render/src/shaders/blit.wgsl`, `crates/render/src/post/chain.rs`,
  the untracked `bloom_dual` files.
- C13 profiler: `crates/game/src/bin/arena.rs`, `crates/game/src/bin/arena_frame/mod.rs`.
- Hurt constants: `crates/arena/src/lib.rs`.

Land each card's first step after those lanes commit, or touch only the hunk you add.
