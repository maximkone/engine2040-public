STEAL SCOUT: gmrdad82/pfx (BLOCK 3, position 13, 2026-10-18). Read-only pass. Donor files read live from raw.githubusercontent.com on 2026-10-18; no clone, no build, no run, no bytes vendored.
Repo: gmrdad82/pfx -- a game and render engine in Rust (part of PITO MD): live 60 fps renderer, offline path tracer with its own denoiser, physics, sound, input, mods, scene editor, scenes as plain TOML. 1 star, pushed 2026-10-18, default branch main. 36539 KB. Crates include live, trace (path tracer), denoise, post, film, sound, physics, materials, text, editor + ~60 docs.
Licence: MIT. LICENSE read live 2026-10-18: "MIT License / Copyright (c) 2026 Catalin Ilinca" (22 lines). GitHub API agrees spdx MIT.
Note: a sibling wave-2026-10-18 card already took `crates/live/src/demand.rs` (partial-redraw region). Not repeated here.
Plans read: render-look.md, quality.md (PF/PF9), HASH-LAW.md, node/GPU skills (render-cull-vram, render-pipeline-math).

== 1. WE ALREADY HAVE ==
- Clustered light assignment: crates/render/src/shaders/cluster.wgsl:1-20 -- tile/slice index and a clustered light loop, ported from playcanvas (PORT lines 3-5). MAX lights per tile is in crates/render/src/core.rs:30 (ForwardPlus).
- Named history textures with a validity flag: crates/render/src/temporal.rs:1-18 (kajiya take/export, history_valid false on first use, resize, dyn-res step and camera cut).
- A TAA resolve and jitter: crates/render/src/v1_temporal.rs, crates/render/src/post_jitter_motion.rs, crates/render/src/post.rs TaaParams (temporal.rs:14-16).
- An upscaler: crates/render/src/upscale_easu.rs. A dynamic-resolution governor: crates/render/src/dynres_gov.rs:1-16 (integer only, WINDOW_LEN 4, RISE_STEP_PCT 5).
- A GPU cull slice: crates/render/src/gpu_cull.rs:1-12 plus shaders/cull.wgsl. A pipeline warm-up: crates/render/src/pipeline_warmup.rs.
- A profiler with p50/p95/p99: crates/perf/src/profiler.rs:6,52,281,297.

== 2. STEALS (ranked by value for the Rome duel per hour) ==
1. Versioned weights-blob container for a GPU denoiser | gmrdad82/pfx@main:crates/denoise/src/tza.rs:3-9,14-48,50-60 (read 2026-10-18) | MIT | SIM-adjacent (asset format) | lands crates/nn (the `.e2nn` container) | S
   Idea: a tiny binary container with MAGIC 0x41D7, a major/minor version byte pair, and a u64 tensor table; each tensor is a `dims: Vec<usize>` plus a flat `data: Vec<f32>` in a `BTreeMap<String, Tensor>` (names, sorted, so the file is canonical). The reader is a cursor with checked `take` (overflow-safe `checked_add` then a bounds filter), so corrupt input returns a typed error and never indexes out of bounds; `major != 2` is refused with an explicit "unsupported version" message. MAX_DIM guards absurd shapes.
   Maps: this is exactly the shape our `.e2nn` integer net needs and does not fully state: a magic, a version gate, named tensors in sorted order, checked reads. Our format owner is crates/nn (lib.rs/quant.rs/qat.rs).
   Moves: today a version mismatch or a truncated `.e2nn` is not described by one container rule; with this it is one magic + one version + one cursor. Unmeasured.
   Proof: cargo test -p engine-nn --lib blob x2 (magic wrong refuses, major bump refuses with the version in the message, a truncated tail refuses, a round trip is byte-equal x2 and the bytes do not depend on map iteration order).
   Risk: f32 weights here; our runtime is integer-only, so keep f32 only as the shipping form and quantize at load. Kill: if the container adds a dependency or the quantized form cannot round trip byte-equal, keep the plain header only.

2. Film/tape grade as data: named constants plus a band table | gmrdad82/pfx@main:crates/post/src/tape.rs:5-40 (read 2026-10-18) | MIT | PRESENTATION | lands crates/render/src/post.rs and crates/render/src/post/ (grade/tone) | S
   Idea: the whole look is a small set of named constants, not shader magic: SPLIT [0.0034, 0.0039] and MIX [[0.41,0.59,0.0],[0.92,0.06,0.02],[0.45,0.08,0.47]] (split-tone), ECHOES [(0.007,0.25),(0.016,0.10)], SOFT 0.5, BLEED 0.75/1920 (a width in pixels, resolution-aware), LIFT 0.07, DESATURATE 0.22, GRAIN 0.008, SCANLINES 0.04, SHOVE 0.0025, JITTER 0.0003, plus a BANDS[3] table each with turn/start/speed/width/gain. Every knob is a number a look judge can name and move.
   Maps: our post chain lives in crates/render/src/post.rs and shaders/tonemap.wgsl. A tape/film grade is a look option for the Rome duel the same way our film grade already is; taking the constants as data (a table, not literals in WGSL) matches how materials and tiers are already data.
   Moves: grade controls 0 named -> ~14 named plus a 3-row band table; a look change becomes a table edit with a re-blessed look capture, not a shader edit. Unmeasured.
   Proof: a `grade_tape` look-off/look-on capture pair where pixels differ and the golden stays put; cargo test -p engine-render --lib tape_grade x2 (table clamps, unknown knob refuses, deterministic from a seed for grain).
   Risk: grain and scanlines must be seeded and history-independent or a capture is not reproducible. Kill: if any of it reads a clock or a frame counter that the capture path does not fix, keep split-tone and bleed only.

3. Compile-time Halton jitter cycle for TAA, offsets centred and fixed | gmrdad82/pfx@main:crates/live/src/taa.rs:3-14,16-53 (read 2026-10-18) | MIT | PRESENTATION | lands crates/render/src/post_jitter_motion.rs and crates/render/src/temporal.rs | S
   Idea: CYCLE 8, and the eight sample offsets are built by a `const fn` Halton(2,3) sequence that is then centred by subtracting the mean, so the cycle has no net drift; formats are pinned per resource (colour Rgba16Float, depth R32Float, motion Rg16Float, reactive R8Unorm, id R32Uint, normal Rgba8Snorm) and three history flags (HISTORY_VALID, HAS_ID, HAS_NORMAL) say what a history texel is worth, so a disocclusion with no id or no normal is not blended.
   Maps: our temporal.rs already carries a validity flag; the new part is the fixed 8-sample centred cycle as compile-time data, and the id/normal validity bits that tell the resolve "do not blend here". That is the honest gap between "history_valid" (whole texture) and per-texel rejection.
   Moves: jitter cycle from ad hoc -> 8 fixed centred offsets; disocclusion rejection from texture-wide -> per texel with id and normal. Unmeasured.
   Proof: cargo test -p engine-render --lib jitter x2 (the 8 offsets sum to ~0 and are stable; the same frame index gives the same offset) plus a TAA-on/off capture pair; golden unchanged.
   Risk: an added id and normal target costs VRAM and one more pass. Kill: if the id/normal targets cost > 0.2 ms at 1080p on the RTX 3050, keep the fixed offset cycle and drop the per-texel bits.

4. Cluster grid shape: fixed cell count, 24 z slices, exponential first slice | gmrdad82/pfx@main:crates/live/src/clusters.rs:3-7,38-52 (read 2026-10-18) | MIT | PRESENTATION | lands crates/render/src/shaders/cluster.wgsl and the cluster uniform | S
   Idea: TOTAL 4096 cells split into Z_SLICES 24, with the first slice only FIRST_SLICE_DEPTH 5.0 m deep and LIGHT_WORDS 20 (a 640-bit light mask per cell); `dims(aspect)` derives tiles_x/tiles_y from the aspect so the cell count stays 4096 whatever the window is, and a cone test rides beside each sphere so a spotlight culls by direction too (`Sphere { centre, radius, cone }`).
   Maps: we already do clustered light assignment (cluster.wgsl), so this is a capacity/shape comparison, not a new system. The slice count, the exponential first slice, the fixed 4096 total across aspects, and the per-light cone are the concrete numbers to weigh against our current tile+slice constants. Marked PARTLY HAVE.
   Moves: light cells fixed at 4096 across aspects (unmeasured against ours); spotlights able to leave cells by cone as well as radius.
   Proof: a capture with a cone-limited spotlight outside its cone whose lit pixels change by < 1%, and the cluster fixture prints 4096 cells at 16:9 and at 21:9; golden unchanged.
   Risk: 20 light words per cell is 80 bytes a cell = 320 KB of masks; ours may be smaller. Kill: if the mask bytes exceed our current cell memory by more than 2x for no visible win, keep our constants.

== 3. NOT WORTH STEALING ==
- The path tracer and its own denoiser (crates/trace/*, crates/denoise/src/net.rs, process.wgsl): a full offline tracer with a learned denoiser is a second renderer. We have restir_di.rs and rt_shadows.rs and one live renderer; a second offline path is not the Rome duel. Only the weights-blob container (steal 1) is worth taking.
- The dynamic-resolution governor (crates/live/src/upscale.rs, dynres): we have dynres_gov.rs already (integer, deadband, cooldown). The donor's upscale weights (upscale.rs:36-45 Catmull-Rom, :47-49 sRGB decode) overlap our upscale_easu.rs.
- The editor and the scene TOML/`film` machinery (crates/editor, crates/film, crates/recipe): a whole editor and a film-clip DSL. Not engine material for us.
- The 4K-in-4.4 ms headline numbers (README:9-16): measured on the author's box with the author's scene; our perf law wants our own p50/p99 on the RTX 3050 (docs/RUST-QUALITY.md, perf skill). Cite the method, never the number.

DONE 4 steals, 6 donor files read.
