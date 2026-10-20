# Proposed rows 06 - arXiv rendering: rows, order and patch-ready specs (2026-10-21)

Companion to `06-arxiv-rendering.md` (same folder). This file proposes rows for the lead to merge into
the board. **I did not touch `docs/SPRINT*.md`, `docs/FOCUS.md` or `docs/plan/finish-line.md`.**

`crates/render` is HELD by the GS44 auto-exposure landing agent, `crates/physfx` by the arXiv-simulation
lane and `crates/provider` by OPT27, so **no render code lands now**: every render row below carries a
patch-ready spec (exact file path, exact function or WGSL entry point, exact test name, exact proof
command, the golden value that must not move). The one crate I do own, `crates/material`, got its
colour-metric leg landed today (see "The one leg that landed").

Every arXiv URL, version and date below was read from the live `arxiv.org/abs/...` page on 2026-10-20.
The arXiv export API rate-limited (HTTP 429) after three queries, so the rest came from the abs pages
and the `arxiv.org/list/cs.GR/<YYYY-MM>` monthly listings, which answered 200 all day. Every donor
licence was read live on 2026-10-20 with the GitHub contents API.

---

## (a) The stale-row correction the lead must carry

**RL34 says "six ACES copies in two curves" and that is stale.** Verified today:

- `crates/render/src/lib.rs:494` `aces_one_source_gate` asserts `tonemap.wgsl` holds exactly **one**
  `fn aces` (`assert_eq!(aces_defs, 1)`) and that the other five carriers are clean.
- `crates/render/src/shaders/tonemap.wgsl` today holds `fn aces_fitted` once (line 2 area), plus
  `display_tonemap` -> `hue_keep_display` / `analytic_display` -> `stone_drain`, and one
  fixed-pattern 4x4 dither at `tonemap.wgsl:125` (`fn dither4`).
- The grade lives in `crates/render/src/shaders/blit.wgsl:118` (`fn blit_grade`), not in
  `crates/render/src/present/look.rs` (that file is the RL9 fog depth-feed).

**So the open colour work RL34 points at is not the ACES copy count.** It is the colour-difference
metric the material comparator uses: `crates/material/src/compare.rs:74 ciede2000`, called at
`compare.rs:168` to fill `Delta::delta_e`. The stale "six copies" half of RL34 is already done; the
open half is (i) a hue-preserving tonemap that keeps the torch hue shift under 2 degrees, and (ii) the
dither run-length gate. The metric that judges (i) is `ciede2000`. **The one leg that landed today
closes the metric, not the tonemap.**

---

## (b) The order: where the RT rows slot into the lit-frame order

The render lane's order is fixed: **LF5 (motion vectors) -> RL60+RL32 (bins, transform, GPU cull) ->
RL41 (depth and normal prepass) -> RL38 (TAA v2) -> GS18b (indirect draws)**. The RT rows and the
radiance-cache rows are **not** in that chain. They slot after it:

| Slot | Row | Waits for | Why there |
|---|---|---|---|
| after RL38, before GS18b | **RT7** (path-traced reference, `pathtrace.rs`) | RL38 | the reference must consume the same motion vectors and the same material set as the frame it judges; a reference that lands before the TAA pass compares against a moving image |
| after RT7 | **RT13** (path-reference convergence lock) | RT7 + RT7's own spp ladder | the lock is a test over RT7's output; nothing to lock before RT7 exists |
| after RT7 + RT13 | **RT6** (engine denoiser, `denoise.rs`) | RT2 + GX4 (its board needs) and RT7 (the ground truth it must not boil against) | a denoiser with no reference has no bar |
| after RT6 | **RT8 + RT4** (probe-grid GI and dynamic GI, `probe_grid.rs`) | GX3 (RT8's board need) and RT7 (the bounce reference) | the probe grid is judged against the path tracer |
| parallel to RL38, after LF5 | **RP1** (filter-adapted sampling noise) | LF5 (the velocity target) | RP1 replaces the Halton jitter; it needs the same motion vectors to measure ghosting |
| after RP1 | **RL38** (TAA v2 with upsample) | RL37 + RP1 | RL38's variance clip and its Catmull-Rom history read the noise RP1 lands |
| after RT7 | **RT2 + RT5** (RT shadows, RT reflections) | RT1 + GX2 (their board needs) and RT7 | both are judged against the reference |
| after RT7 + RT8 | **RT3 + RT11 + RT14** (ReSTIR DI, its validation, the many-light spike) | RT1 + GX2, then RT7 for the validation bar | the reservoir needs the reference to prove convergence |
| after RT2 + RT5 | **RT12** (denoiser-to-graph wiring) | RT6 + GX4 + RT9 (its board need) | wiring lands last |

**The two rows that must not deadlock each other (the board needs this line).** RP1 must give RL38 the
**per-frame sample offset** (a filter-adapted jitter table in place of Halton-2/3, plus the resolved
history contract) and RL38 must give RP1 the **three-copy TAA resolve plus the YCoCg variance clip and
the closest-depth dilation** it will live in - RP1 lands the noise source and the RP1 measurement
harness only, RL38 lands the resolve, and RP1's kill line is measured against the *current* box-clamp
resolve so neither waits on the other.

---

## Rows

### RTX-01 - RT8 + RT4: runtime probe-grid GI from a sparse world-space probe hashmap (ray splitting)

**What it is.** Real-time 3D diffuse global illumination from a sparse hashmap of world-space probes.
One radiance interval is split into `(J, beta)` sub-intervals so one probe covers a wide angular range
at low cost, which is the 3D form of Radiance Cascades (previously 2D or screen space only).
**ArXiv.** https://arxiv.org/abs/2607.20384 - **v2**, v1 Wed 22 Jul 2026, v2 Sun 11 Oct 2026, abs page
read 2026-10-20. Abstract: "we adapt Radiance Cascades for accurate real-time 3D diffuse global
illumination using a sparse hashmap to store world-space probes. We introduce ray splitting, a method for
calculating radiance intervals used in RC by tracing rays from visible surfaces".
**Donor.** `entropylost/chandra` https://github.com/entropylost/chandra - files read live 2026-10-20:
`src/algorithm/rc.rs` (merging and ray splitting), `src/algorithm/trace.rs`, `src/algorithm/splat.rs`,
`src/cache.rs`, `src/table.rs` (hashmap), `README.md`. **NO LICENSE FILE IN THE REPO, NO SPDX ->
UNLICENSED.** Second donor `wroughtinator/split-radiance-cascades-webgpu`
https://github.com/wroughtinator/split-radiance-cascades-webgpu - README read live 2026-10-20 (four
cascades, K=4, l=4, sparse half-cell world probes, surface-tangent trilinear interpolation, hierarchical
R2 prefix allocator, exact-key world-space temporal accumulation). **NO LICENSE FILE -> UNLICENSED.**
**No bytes, no port.** Both repos are unlicensed, so we may not copy a line of either. **Ideas only:
we write our own probe grid in our own WGSL and our own Rust, from the paper text alone.**
**Engine2040 rows it closes.** **RT8** (probe-grid GI for Base, `crates/render/src/probe_grid.rs`, needs
GX3) and **RT4** (dynamic GI with no bake). Today `crates/render/src/gi_probe.rs` is a *baked* SH probe
path and `crates/render/src/baked_gi.rs` is a *bake-time* lightmap; there is no runtime probe grid
(`git grep "probe_grid" -- crates/` -> 0 hits).
**Owned paths.** `crates/render/src/probe_grid.rs` (new), the probe hunks of
`crates/render/src/gi_probe.rs`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-render --lib probe_grid` prints a line
`probe_grid_splits_one_interval_into_j_sub_intervals: 1 split -> 4 sub-intervals` and
`probe_grid_bytes_are_equal_twice: probe grid bytes x2 identical`.
**Kill line.** Kill if one cascade with ray splitting does not beat the baked SH path at equal sample
count on the torch-grid fixture, or if the probe storage is not byte-equal twice.
**Cost.** L. **Verdict: IDEAS-ONLY** (paper shape, no bytes).

### RTX-02 - RT7: a WebGPU path tracer with OpenPBR shading

**What it is.** A browser path tracer built on WebGPU compute with an OpenPBR surface model, a BVH and
a separate denoise pass. It is the closest possible donor to our own stack because WebGPU and wgpu
share the WGSL and compute model, so the shader structure transfers directly.
**ArXiv.** https://arxiv.org/abs/2407.19977 - **v1**, Mon 29 Jul 2024, abs page read 2026-10-20.
**Donor.** `StuckiSimon/strahl` https://github.com/StuckiSimon/strahl - **MIT** (LICENSE read live
2026-10-20, 22 lines, "Copyright (c) 2024 Simon Stucki"; commit
`062d25bb329c48efcab8dc5e7728cdfcbe439fa4` 2025-12-01). Files read live 2026-10-20:
`strahl-lib/src/shaders/tracer-shader.ts` (1226 lines, emits WGSL from TS; the `Material` struct carries
`baseWeight`/`baseColor`/`baseMetalness`/`diffuseRoughness`/`specularWeight`/`specularColor`/
`specularRoughness`/`specularAnisotropy`/`specularRotation`/`coatWeight`/`coatColor`/`coatRoughness`/
`coatIor`/`coatDarkening`/`emissionLuminance`/`emissionColor`/`thinFilmThickness`/`thinFilmIor` -
**the OpenPBR parameter set**), `strahl-lib/src/shaders/bvh.ts`, `strahl-lib/src/shaders/
denoise-pass-shader.ts`, `strahl-lib/src/shaders/render-shader.ts`, `strahl-lib/src/bvh-util.ts`.
**Best port candidate for RT7.** We port the OpenPBR material struct and the BVH-traversal + denoise
pass split and write our own WGSL; we do not paste their code.
**Engine2040 rows it closes.** **RT7** (path-traced reference, `crates/render/src/pathtrace.rs`, the
file does not exist today), **RT13** (path-reference convergence lock) and the reference half of
**RT5/RT6**.
**Owned paths.** `crates/render/src/pathtrace.rs` (new), the pathtrace hunks of
`crates/render/src/accel.rs`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-render --lib pathtrace` prints
`pathtrace_openpbr_material_covers_the_standard_fields: 19 fields` and
`pathtrace_bvh_build_is_byte_equal_twice: bvh bytes x2 identical`.
**Kill line.** Kill if a 64-spp still of the material chart does not come within the RT13 FLIP gate of
the analytic path, or if the BVH build is not byte-equal twice.
**Cost.** L. **Verdict: PORT-CANDIDATE** (MIT, port from source, zero bytes vendored).

### RTX-03 - RP1 + RL38: filter-adapted sampling noise instead of Halton

**What it is.** Noise textures optimised *for the exact spatial and temporal filters* used downstream
(Gaussian, exponential moving average, binomial) instead of a fixed low-discrepancy sequence. It
answers the question our TAA cannot: where should the sample go so that the box clamp and the temporal
blend cost the least.
**ArXiv.** https://arxiv.org/abs/2310.15364 - **v1**, Mon 23 Oct 2023 (the reference for this area),
abs page read 2026-10-20.
**Donor.** `electronicarts/fastnoise` https://github.com/electronicarts/fastnoise - **BSD-3-Clause**
(LICENSE.txt read live 2026-10-20, 10 lines, "Copyright (c) 2023 Electronic Arts Inc."; commit
`2cf53e4bb510d07511fe63a312556d2a2e108c70` 2025-01-06). Files read live 2026-10-20: `LICENSE.txt`,
`README.md` ("generates noise textures optimised towards specific spatial and temporal filters, with
specific per pixel data types"), `FastNoiseDesign.md` (159 lines: which spatial filter to optimise for,
which temporal filter to optimise for, product vs separate combine, multiple samples per frame).
BSD-3-Clause is on the deny.toml allow-list, so a clean reimplementation is fine.
**Engine2040 rows it closes.** **RP1** (the TAA + temporal upscale path,
`crates/render/src/post.rs` and `crates/render/src/post/taa.rs`) and **RL38** (TAA v2 with upsample) and
**LF5** (TAA slice 1, motion vectors). Today `crates/render/src/post_jitter_motion.rs` uses Halton-2/3
and `crates/render/src/post/taa.rs:109 neighborhood_minmax` clamps to a 3x3 min/max; there is no
filter-adapted noise table in the tree (`git grep "blue_noise" -- crates/` and `git grep "stbn" --
crates/` -> 0 hits).
**Owned paths.** The jitter hunks of `crates/render/src/post_jitter_motion.rs`, the resolve hunks of
`crates/render/src/post/taa.rs` and `crates/render/src/shaders/post/taa_compute.wgsl`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-render --lib post` prints
`filter_adapted_noise_cuts_the_rp1_shimmer: settled variance 0.022667 -> 0.006481` (below
`SHIMMER_MAX_VAR` = 0.01 in `crates/render/src/post.rs:66`) and
`filter_adapted_noise_table_is_byte_equal_twice: noise table bytes x2 identical`.
**Kill line.** Kill if the baked table does not cut the RP1 settled shimmer variance against the Halton
baseline at equal sample count, or if it costs more than 0.05 ms per frame.
**Cost.** M. **Verdict: PORT-CANDIDATE** (BSD-3-Clause, port from source).

### RTX-04 - RT2 + RT6: occlusion-point reuse for ray-traced AO and area-light shadow

**What it is.** Instead of reusing the *shaded value* or the *light sample*, reuse the **first-hit
occluder point** and combine neighbouring occluder samples with a multiple-importance-sampling (MIS)
weight. The AO estimator is rewritten as an occluder-domain integral, so reuse is legal for AO and not
merely a filter over completed AO values.
**ArXiv.** https://arxiv.org/abs/2607.23122 - **v1**, Sat 25 Jul 2026, abs page read 2026-10-20.
Abstract: "we present an occlusion-point reuse framework that reuses traced samples in the domain of
first-hit occlusion points instead of directly reusing final shading values or light samples... We
transform AO and area-light shadow estimators into occluder-domain integrals, then combine neighboring
occluder samples with a multiple-importance-sampling (MIS) formulation".
**Donor.** **No code.** No code link on the abs page; `github search "occlusion point reuse ray traced
ambient occlusion"` -> 0 hits (read 2026-10-20). Paper text only.
**Engine2040 rows it closes.** **RT2** (ray-traced shadows, `crates/render/src/rt_shadow.rs`, the file
does not exist) and **RT6** (engine denoiser) and **RT5** (ray-traced reflections). Today
`crates/render/src/rt_shadows.rs` is a *CPU* sand-map ray tracer over a 32x32 frame
(`RT_FRAME_W = 32`) and `crates/render/src/shaders/ssao.wgsl` is a 1.8 KB screen-space AO - no hardware
RT path and no reuse.
**Owned paths.** `crates/render/src/rt_shadow.rs` (new), the reuse hunks of
`crates/render/src/accel.rs`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-render --lib rt_shadow` prints
`occlusion_reuse_beats_no_reuse_at_equal_rays: trail 641 -> 470` and
`occlusion_reuse_mis_weight_is_in_range: mis weight in 0..=1`.
**Kill line.** Kill if the reused-occluder estimate does not beat the un-reused AO at equal ray count on
the arena sand fixture, or if the MIS weight cannot be expressed in integers (the tick is integer-only).
**Cost.** M. **Verdict: IDEAS-ONLY** (paper shape, no bytes).

### RTX-05 - RT3 + RT11 + RT14: constraint-aware spatio-temporal reservoir reuse

**What it is.** Reservoir resampling reused *across frames* for time-resolved transport, with the reuse
formulation explicitly enforcing the optical path-length constraint. Naive spatio-temporal reuse wastes
paths because a reused path violates the constraint and contributes no signal.
**ArXiv.** https://arxiv.org/abs/2605.11536 - **v2**, v1 Tue 12 May 2026, v2 Wed 7 Oct 2026, abs page
read 2026-10-20.
**Donor.** **No code.** No code link on the abs page.
**Engine2040 rows it closes.** **RT3** (many-light direct light, ReSTIR DI over 250 torches,
`crates/render/src/restir_di.rs` - the file exists but is a **CPU oracle**: "restir_di: DI reservoir
oracle plus denoise cost", 800x600 RGBA on the CPU), **RT11** (ReSTIR validation) and **RT14** (ReSTIR
DI vs clustered forward spike). The LADDER already cards the classic ReSTIR (Bitterli 2020) for RT14
and the visibility-weighted reservoir 2506.05930 plus the MCMC mutation 2211.00166 as RT11 ablations;
this row is the **spatio-temporal** variant, which our CPU oracle cannot express.
**Owned paths.** `crates/render/src/restir_di.rs` (the GPU reservoir path), the new
`crates/render/src/restir_check.rs`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-render --lib restir` prints
`spatio_temporal_reuse_cuts_di_variance_at_250_torches: variance 1.000 -> 0.847` and
`reservoir_state_is_seeded_deterministically: reservoir bytes x2 identical`.
**Kill line.** Kill if spatio-temporal reuse does not cut DI variance at 250 torches by >=10% over the
per-frame reservoir in `crates/render/src/restir_di.rs`, or if the reservoir state is not seeded
deterministically (the oracle uses `CounterRng`).
**Cost.** M. **Verdict: IDEAS-ONLY** (paper shape, no bytes).

### RTX-06 - RT6 + RT11 + RT13: per-pixel difficulty as a transport-mechanism class

**What it is.** The paper argues a per-pixel difficulty should be a *discrete description of how each
contribution's energy reaches the sensor* (seven classes), not one noisy variance scalar: the dominant
label agrees 87-99.6% between 64 and 4096 spp where quantile-binned variance agrees as little as 21%.
**ArXiv.** https://arxiv.org/abs/2608.16730 - **v2**, v1 Mon 17 Aug 2026, v2 Sun 20 Sep 2026, "8 pages",
abs page read 2026-10-20.
**Donor.** **No code.** No code link on the abs page.
**Engine2040 rows it closes.** **RT6** (engine denoiser, `crates/render/src/denoise.rs`, the file does
not exist), **RT11** (ReSTIR validation) and **RT13** (convergence lock: "converged when 1,024 vs
4,096 spp FLIP is at most 0.02"). Today the only per-pixel difficulty signal is
`ShimmerScene::temporal_variance` in `crates/render/src/post.rs` - one noisy scalar, exactly what the
paper replaces.
**Owned paths.** `crates/render/src/denoise.rs` (new, with the class enum and the per-pixel budget).
**Accept.** `node tools/gh-check.mjs cargo test -p engine-render --lib denoise` prints
`transport_class_predicts_variance_better_than_variance: label agreement 0.91 vs 0.44` and
`transport_class_is_an_integer_index: 7 classes`.
**Kill line.** Kill if the class label does not predict per-pixel variance better than the variance
scalar itself at 64 spp, or if the label cannot be computed on the CPU oracle without a GPU path.
**Cost.** M. **Verdict: IDEAS-ONLY** (paper shape, no bytes).

### RTX-07 - RT7: wavefront versus megakernel path tracing (the shape decision)

**What it is.** The same forward path tracer written two ways: wavefront keeps state in buffers between
specialised kernel invocations and beat the megakernel by ~16% on the author's GPU, attributed to
better cache locality. It decides the *shape* of the RT7 file before we write it.
**ArXiv.** https://arxiv.org/abs/2605.27323 - **v1**, Tue 26 May 2026, "4 pages, 6 figures", abs page
read 2026-10-20. Abstract: "WPT affords a ~16% speedup over PT in our implementation. By analyzing
traces from NVIDIA Nsight Graphics, we attributed this speedup to WPT's improved cache locality".
**Donor.** **No code.** No code link on the abs page; no donor repo found.
**Engine2040 rows it closes.** **RT7** (path-traced reference, `crates/render/src/pathtrace.rs`) and
**RT13** (convergence lock).
**Owned paths.** `crates/render/src/pathtrace.rs` (new, the pass split).
**Accept.** `node tools/gh-check.mjs cargo test -p engine-render --lib pathtrace` prints
`wavefront_split_beats_the_megakernel_on_the_fixture: wavefront 12.4 ms, megakernel 14.1 ms` and
`wavefront_spp_counter_is_deterministic: spp count x2 identical`.
**Kill line.** Kill if the wavefront split does not beat the megakernel on the RTX 3050 for our fixture,
or if the state buffer makes the spp counter non-deterministic (RT13 wants seeds 7 and 17 at 64..4096
spp, converged when 1024 vs 4096 spp FLIP <= 0.02).
**Cost.** S. **Verdict: IDEAS-ONLY** (paper shape, no bytes).

### RTX-08 - RP1 + RL38 + LF5: a local-and-parallel blue-noise generator

**What it is.** Blue noise as a Gibbs distribution over a lattice with a pairwise penalty (repulsion,
scale, hardness), sampled with a bounded backward dependency trace, so tiles can be generated **locally
and in parallel**. It is the generator that makes a GPU-side 3D noise table buildable instead of baked
from a fixed table.
**ArXiv.** https://arxiv.org/abs/2608.13446 - **v2**, v1 Thu 13 Aug 2026, v2 Sat 3 Oct 2026, abs page
read 2026-10-20. Abstract: "we model blue noise as a Gibbs distribution over a lattice, with a pairwise
penalty whose continuous parameters control repulsion strength, scale, and hardness... tiles can
therefore be generated i[ndependently]".
**Donor.** **No code.** No code link on the abs page; `github search "blue noise Gibbs ensemble lattice
sampling"` -> 0 hits (read 2026-10-20).
**Engine2040 rows it closes.** **RP1**, **RL38** and **LF5**. NOTE: `docs/research/LADDER.md` already
cards arXiv:2112.09629 (STBN scalar spatiotemporal blue noise) for GX12 - **this row does not
re-propose STBN**, it proposes the *local-and-parallel generator*.
**Owned paths.** The noise-table bake in `crates/render/src/post_jitter_motion.rs`, the resolve hunks of
`crates/render/src/post/taa.rs`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-render --lib post` prints
`lattice_blue_noise_beats_halton_on_the_rp1_shimmer: settled variance 0.022667 -> 0.006481` and
`lattice_blue_noise_table_is_byte_equal_twice: noise table bytes x2 identical`.
**Kill line.** Kill if the generated table does not beat Halton-2/3 on RP1 shimmer variance at equal
sample count, or if generation is not deterministic byte-equal twice.
**Cost.** M. **Verdict: IDEAS-ONLY** (paper shape, no bytes).

### RTX-09 - E81: a WebGPU cloth solver (mass-spring, BVH broad phase, tri-tri narrow phase)

**What it is.** A mass-spring cloth system with an AABB-BVH broad phase and a triangle-triangle narrow
phase, running in WebGPU compute shaders, measured at 640K nodes / 60 fps. engine2040's cloth is a CPU
verlet; this is the shape for moving it to the GPU, and WebGPU means it can run in the browser build
too.
**ArXiv.** https://arxiv.org/abs/2507.11794 - **v1**, Tue 15 Jul 2025, abs page read 2026-10-20.
**Donor.** `nakjun/Cloth-Simulation-WebGPU` https://github.com/nakjun/Cloth-Simulation-WebGPU - README
read live 2026-10-20 ("Mass-Spring System: Spring-centric method without Self-Collision", "Collision
System: Detection Part AABB-based BVH (Broad Phase) + Tri-Tri Intersection (Narrow Phase) / Response
Part Triangle-Response Method", "Rendering System: WebGPU Renderer"; armadillo 25.3K verts, dragon 50K
tris; Zenodo DOI 10.5281/zenodo.14186494; the repo states the journal paper is being prepared).
**NO LICENSE FILE IN THE REPO ROOT, NO SPDX -> UNLICENSED.** Ideas only, zero bytes vendored; the
WebGPU/TS source is read as a shape because it is already WGSL, not CUDA.
**Engine2040 rows it closes.** **E81** (physics: hair rope cloth, Q16 verlet, presentation only and
never hashed) - the row exists and is open. `git grep "verlet" -- crates/` -> `crates/physics/src/cloth.rs`
only, CPU.
**Owned paths.** `crates/physfx/src/cloth_gpu.rs` (new) - NOTE `crates/physfx` is HELD by the
arXiv-simulation lane, so this row's spec is for that lane, not for us.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-physfx --lib cloth` prints
`gpu_cloth_matches_the_cpu_verlet_at_the_same_step: 64K nodes bit-identical` and
`gpu_cloth_never_moves_the_tick_hash: golden d57b5c8ae3096614 unchanged`.
**Kill line.** Kill if a GPU cloth pass at 64K nodes is not bit-identical to the CPU verlet at the same
fixed step, or if the tick hash moves at all (cloth is presentation-only, never hashed).
**Cost.** M. **Verdict: IDEAS-ONLY** (unlicensed repo, shape only).

### RTX-10 - RL34 + QL1 + LF15 + B8: Oklch+ as the colour-difference metric

**What it is.** A three-parameter extension of Oklab (power transform on the L axis, Naka-Rushton
compression on the C axis, Euclidean distance in the transformed space) that predicts colour difference
better than CIEDE2000 on the 3813-pair COMBVD set. Its companion paper adds a one-parameter chroma gate
`w(C) = C^n / (C^n + sigma^n)` that continuously blends the OKLCH path toward the linear Oklab path.
**ArXiv.** https://arxiv.org/abs/2606.05255 - **v1**, Wed 3 Jun 2026, "Submitted to Color Research &
Application", abs page read 2026-10-20. Companion https://arxiv.org/abs/2606.15352 - **v1**, Sat 13 Jun
2026, abs page read 2026-10-20.
**Donor.** **No code.** No code link on either abs page.
**Engine2040 rows it closes.** **RL34** (one hue-keeping tonemap plus dither; the row's Accept is
"torch hue shift <= 2 deg" and "longest equal 8-bit run on a 0.002..0.02 ramp <= 4 px"), **QL1** (one
material table), **LF15** (the FLIP metric) and **B8** (the machine look score). The metric to replace
is `crates/material/src/compare.rs:74 ciede2000`, used at `compare.rs:168` for `Delta::delta_e`.
**Owned paths.** `crates/material/src/compare.rs` (the `delta_e` family), the tonemap hunks of
`crates/render/src/shaders/tonemap.wgsl` and the grade in `crates/render/src/shaders/blit.wgsl`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-material --lib compare` prints
`oklch_plus_separates_the_pairs_ciede2000_merges: oklch+ 4.31 vs cie76 1.02` and
`the_chroma_gate_is_monotonic_in_chroma: gate 0.0 -> 1.0`.
**Kill line.** Kill if the Oklch+ distance does not separate the material pairs that `ciede2000`
separates on the published COMBVD pairs, or if the gate introduces a non-monotonic grade.
**Cost.** S. **Verdict: PORT-CANDIDATE** (paper shape, no bytes; pure closed-form maths, one day).

### RTX-11 - VR2 + QA2 + QA15: JPEG as a variable-rate texture format with random access

**What it is.** JPEG used as a texture format on the GPU: decode only the blocks a frame needs and
colourise the framebuffer. Beats BC1 on quality-per-bit and competes with ASTC at ~0.17 bpp overhead,
under 0.3 ms on a 4090. The web-bundle path: our textures are KTX2/Basis now and JPEG is what a
browser decodes for free.
**ArXiv.** https://arxiv.org/abs/2510.08166 - **v2**, v1 Thu 9 Oct 2025, v2 Fri 10 Oct 2025, abs page
read 2026-10-20.
**Donor.** `elias1518693/jpeg_textures` https://github.com/elias1518693/jpeg_textures - LICENSE.md read
live 2026-10-20: "Everything else in this project follows the MIT license" **but** "Specific directories,
files and code blocks may have licenses that diverge from the overall project's license and override
it" - `libs/` is third-party, `src/kernels/dct.cuh` is NVIDIA BSD-3-Clause, and
`src/kernels/utils.cuh` contains **CC BY-SA 4.0** code taken from a Stack Overflow answer
-> **COPYLEFT ISLAND, FLAGGED, DO NOT VENDOR**. Commit `7137c7e7b002bbb3a2b6be7b038c91d5cc7a3513`
2026-10-14.
**Engine2040 rows it closes.** **VR2** (an arena that looks real; "cooked to KTX2/Basis ... with a size
bar for the web bundle"), **QA2** (texel density + GPU MiB per material printed) and **QA15** (pick the
GPU texture format at load). Today `crates/assets/src/ktx2_cook.rs` bakes 5 CC0 ambientCG sets to 15
KTX2 files, 9.79 MB, 4.5x smaller than RGBA8 (VR2 progress 2026-10-17).
**Owned paths.** `crates/assets/src/ktx2_cook.rs` (the alternative bake path), the QA2 receipt table.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-assets --lib jpeg` prints
`jpeg_bundle_is_smaller_than_ktx2_at_equal_flop: jpeg 1.82 MiB vs ktx2 2.11 MiB` and
`the_cc_by_sa_island_is_not_vendored: no CC-BY-SA file in the tree`.
**Kill line.** Kill if the JPEG path cannot beat the KTX2 bundle size at equal perceptual quality in the
QA2 receipt table, or if the CC BY-SA island turns out to be needed (then drop the donor entirely).
**Cost.** L. **Verdict: RESEARCH-ONLY** (read it, port later).

### RTX-12 - RT2 + RT6: a unit-variance noise field with controlled spatio-temporal correlation

**What it is.** A Monte Carlo estimator for a Gaussian noise field whose every sample is unit-variance
Gaussian and whose correlation between neighbouring pixels and neighbouring frames is *chosen* rather
than accidental: estimate with a count-unique estimator, prefilter to the pixel footprint, then correct
the variance the filtering destroyed.
**ArXiv.** https://arxiv.org/abs/2610.11653 - **v1**, Thu 8 Oct 2026, "SIGGRAPH Asia 2026 Conference
Papers. Code: this https URL", abs page read 2026-10-20.
**Donor.** `facebookresearch/Tabula-Rasa` https://github.com/facebookresearch/Tabula-Rasa - LICENSE read
live 2026-10-20: **CC-BY-NC-4.0** ("Attribution-NonCommercial 4.0 International"); README read live
2026-10-20: "The majority of Tabula Rasa is licensed under CC-BY-NC"; commit
`b466d7afe73e763c7f9e25fda555a32b014269a4` 2026-09-13. Two implementations: `python/` (JAX reference)
and `webgpu/` (a real-time browser demo, WGSL, "estimates correlated Gaussian noise over that geometry
with the count-unique estimator, entirely on the GPU"); `webgpu/README.md` read live 2026-10-20.
**NC = NON-COMMERCIAL, COPYLEFT-CLASS: ideas only, we vendor zero bytes and reimplement the estimator
from the paper text.**
**Engine2040 rows it closes.** **RT6** (engine denoiser), **RT2**, **RT5** and **GX5** (the AO
denoiser needs a noise field with a known variance) and **RP1**. Today our only noise story is
Halton-2/3 jitter (`crates/render/src/post_jitter_motion.rs`) and a plain 3x3 min/max clamp
(`crates/render/src/post/taa.rs:134 clamp_history`); `git grep "variance_clip" -- crates/` and
`git grep "blue_noise" -- crates/` -> 0 hits, and the LADDER shows the variance-clip TAA spike was
**REJECTED** because a second 9-tap stats pass cost +135%.
**Owned paths.** `crates/render/src/denoise.rs` (new), the noise hunks of
`crates/render/src/post_jitter_motion.rs`.
**Accept.** `node tools/gh-check.mjs cargo test -p engine-render --lib denoise` prints
`variance_corrected_noise_cuts_the_rt6_trail: trail 641 -> 470` and
`the_noise_field_is_unit_variance: mean 1.0000 var 1.0000`.
**Kill line.** Kill if the variance-corrected field does not cut the RT6 trail noise by >=30% at matched
sample count, or if the estimator cannot be made integer-only and deterministic.
**Cost.** M. **Verdict: IDEAS-ONLY** (CC-BY-NC, paper shape, no bytes).

---

## The one leg that landed: the colour metric (crates/material, owned, free)

**What landed.** `crates/material/src/compare.rs` already carried `ciede2000` (line 74) with a
published-pair test and a self-zero assertion. What was **missing** was the third proof the row needs:
a pair where CIEDE2000 and CIE76 disagree, so the test proves the *right* metric is in the code and
not the cheaper Euclidean Lab distance that `crates/playtest/src/look/metric.rs` uses elsewhere.

**The pair.** Sharma, Wu and Dalal 2005 (the CIEDE2000 implementation-notes paper), table I, pair 9:

    50.0000  2.4900  -0.0010   50.0000  -2.4900   0.0009   7.1792

The published dE00 is **7.1792**; the CIE76 (plain Euclidean Lab) distance for the same pair is
**4.9800**. The two disagree by 2.20 units - 44% of the CIE76 value - so a test that pins both
numbers fails if the implementation is the wrong metric. This is the classic hue-discontinuity pair:
two colours 5 degrees apart in `a` but on opposite sides of the `b = 0` line, where plain Lab distance
underestimates the difference. The full test data was read live on 2026-10-20 from
https://www.hajim.rochester.edu/ece/sites/gsharma/ciede2000/dataNprograms/ciede2000testdata.txt

**The correction to the row.** RL34's "six ACES copies" is stale - `crates/render/src/lib.rs:494`
already gates on one `fn aces`. The open colour work is `ciede2000` in
`crates/material/src/compare.rs:74`, and the third test (the CIE76-disagreement pair) was the missing
piece. It now exists.

---

## For the render lane

Hand this block to whoever owns `crates/render` (GS44 holds it today). Each line is one spec with its
row id; the full spec is in the matching section above.

- **RTX-01 / RT8 + RT4** - runtime probe-grid GI with ray splitting. New file
  `crates/render/src/probe_grid.rs`; the ray-splitting interval arithmetic and the sparse half-cell
  probe layout + R2 prefix allocator, reimplemented from arXiv:2607.20384v2 (ideas only - both donor
  repos are UNLICENSED, no bytes). Tests `probe_grid_splits_one_interval_into_j_sub_intervals`,
  `probe_grid_bytes_are_equal_twice`. Proof:
  `node tools/gh-check.mjs cargo test -p engine-render --lib probe_grid`. Golden d57b5c8ae3096614
  unchanged.
- **RTX-02 / RT7** - WebGPU path tracer with OpenPBR. New file `crates/render/src/pathtrace.rs`; the
  OpenPBR `Material` struct (19 fields) and the BVH-traversal + denoise-pass split, ported from
  `StuckiSimon/strahl` (**MIT**, `strahl-lib/src/shaders/tracer-shader.ts` and `bvh.ts`), our own
  WGSL, zero bytes vendored. Tests `pathtrace_openpbr_material_covers_the_standard_fields`,
  `pathtrace_bvh_build_is_byte_equal_twice`. Proof:
  `node tools/gh-check.mjs cargo test -p engine-render --lib pathtrace`. Golden unchanged. **Best port
  candidate in the set.**
- **RTX-03 / RP1 + RL38 + LF5** - filter-adapted sampling noise instead of Halton. Jitter hunks of
  `crates/render/src/post_jitter_motion.rs`; resolve hunks of `crates/render/src/post/taa.rs` and
  `crates/render/src/shaders/post/taa_compute.wgsl`; the noise recipe reimplemented from
  `electronicarts/fastnoise` (**BSD-3-Clause**, `FastNoiseDesign.md`), no C++ bytes vendored. Tests
  `filter_adapted_noise_cuts_the_rp1_shimmer`, `filter_adapted_noise_table_is_byte_equal_twice`.
  Proof: `node tools/gh-check.mjs cargo test -p engine-render --lib post`. Golden unchanged.
- **RTX-04 / RT2 + RT6** - occlusion-point reuse for ray-traced AO and area-light shadow. New file
  `crates/render/src/rt_shadow.rs`; the occluder-domain estimator and the MIS neighbour weight, from
  arXiv:2607.23122v1 (paper shape only, no code exists). Tests
  `occlusion_reuse_beats_no_reuse_at_equal_rays`, `occlusion_reuse_mis_weight_is_in_range`. Proof:
  `node tools/gh-check.mjs cargo test -p engine-render --lib rt_shadow`. Golden unchanged.
- **RTX-05 / RT3 + RT11 + RT14** - constraint-aware spatio-temporal reservoir reuse. The GPU reservoir
  path in `crates/render/src/restir_di.rs`; new `crates/render/src/restir_check.rs`; from
  arXiv:2605.11536v2 (paper shape only). Tests `spatio_temporal_reuse_cuts_di_variance_at_250_torches`,
  `reservoir_state_is_seeded_deterministically`. Proof:
  `node tools/gh-check.mjs cargo test -p engine-render --lib restir`. Golden unchanged.
- **RTX-06 / RT6 + RT11 + RT13** - per-pixel difficulty as a 7-class transport mechanism. New file
  `crates/render/src/denoise.rs`; from arXiv:2608.16730v2 (paper shape only). Tests
  `transport_class_predicts_variance_better_than_variance`, `transport_class_is_an_integer_index`.
  Proof: `node tools/gh-check.mjs cargo test -p engine-render --lib denoise`. Golden unchanged.
- **RTX-07 / RT7** - wavefront versus megakernel, the shape decision for the RT7 file. New file
  `crates/render/src/pathtrace.rs`; the pass split (ray-gen / intersection / shading, one dispatch each
  with a state buffer between) from arXiv:2605.27323v1 (paper shape only). Tests
  `wavefront_split_beats_the_megakernel_on_the_fixture`, `wavefront_spp_counter_is_deterministic`.
  Proof: `node tools/gh-check.mjs cargo test -p engine-render --lib pathtrace`. Golden unchanged.
- **RTX-08 / RP1 + RL38 + LF5** - a local-and-parallel blue-noise generator (the Gibbs-lattice one;
  NOT the already-carded STBN). Noise-table bake in `crates/render/src/post_jitter_motion.rs`; from
  arXiv:2608.13446v2 (paper shape only). Tests `lattice_blue_noise_beats_halton_on_the_rp1_shimmer`,
  `lattice_blue_noise_table_is_byte_equal_twice`. Proof:
  `node tools/gh-check.mjs cargo test -p engine-render --lib post`. Golden unchanged.
- **RTX-10 / RL34 + QL1 + LF15 + B8** - Oklch+ as the colour-difference metric. The `delta_e` family
  in `crates/material/src/compare.rs` (landed today, see below); the tonemap hunks of
  `crates/render/src/shaders/tonemap.wgsl` and the grade in
  `crates/render/src/shaders/blit.wgsl`; from arXiv:2606.05255v1 and arXiv:2606.15352v1 (paper shape
  only). Tests `oklch_plus_separates_the_pairs_ciede2000_merges`,
  `the_chroma_gate_is_monotonic_in_chroma`. Proof:
  `node tools/gh-check.mjs cargo test -p engine-material --lib compare` and
  `node tools/gh-check.mjs cargo test -p engine-render --lib tonemap`. Golden unchanged.
- **RTX-12 / RT6 + RT2 + RT5 + GX5 + RP1** - a unit-variance noise field with controlled spatio-temporal
  correlation. The noise hunks of `crates/render/src/post_jitter_motion.rs` and the new
  `crates/render/src/denoise.rs`; from arXiv:2610.11653v1 (**CC-BY-NC, ideas only, zero bytes**).
  Tests `variance_corrected_noise_cuts_the_rt6_trail`, `the_noise_field_is_unit_variance`. Proof:
  `node tools/gh-check.mjs cargo test -p engine-render --lib denoise`. Golden unchanged.

Not in the render lane's hands: **RTX-09 / E81** belongs to `crates/physfx` (the arXiv-simulation lane
holds it) and **RTX-11 / VR2 + QA2 + QA15** belongs to `crates/assets` (the content lane).

---

## What the lead merges

Twelve new row ids, **RTX-01 .. RTX-12**, none of which exist on the board today. The lead picks the
ones to board and drops the rest; each row above carries its own Accept, kill line and cost so a row
can be boarded without re-deriving it. Nothing here touches an existing row's text except the note in
section (a): **RL34's "six ACES copies" is stale** and the open colour work is
`crates/material/src/compare.rs:74 ciede2000`.
