# Hunt wave 2, 2026-10-21: five hunters, a second pass

Owner ask (2026-10-21, evening): hunt again, harder, and steal from what the
first wave left. Wave 1 (`docs/research/hunt-2026-10-21/`) produced 35 board
rows, 70 ranked donors and 73 donor-to-row bridges. Wave 2 is the second pass:
five hunters, one area each, every donor opened and read, every number measured,
and every steal named to a file and a line in our tree.

Status: **CARD**. Every steal below is an idea with a measured donor number
attached; none of it is code in our tree yet. The board rows it feeds are named
in the last column, and the row texts carry the donor, the file, the accept and
the golden line.

## The one screen

| # | the steal | donor | our row |
|---|---|---|---|
| 1 | **a predicted input is a blend, not a boolean**: a mispredicted pose fades into the confirmed one instead of snapping | `nilpunch/unity-prediction-rollback` (48 stars, MIT, archived) | MP25, EG39 |
| 2 | **a desync report that names the cause**: a command digest beside the state hash says "the inputs differed" or "the sim drifted", plus the layer (component or system) | `xpTURN/Klotho` (65 stars, Apache-2.0) | RC27, ON30 |
| 3 | **price every loss in dB against an exact reference**, with an oracle bound, and print PSNR *and* LPIPS side by side | `RARS-oss/decibels` (MIT, 2 stars) | LF5c |
| 4 | **per-meshlet frustum and occlusion culling** in WGSL, with a software-rasterizer path that packs depth and octahedron normals into 32 bits | `Scthe/nanite-webgpu` (1124 stars, MIT) | HX7, E70 |
| 5 | **a shader library is a binary**: compile the WGSL once, check the byte size of the cooked blob | `GoogleChrome/shader-pack` -> the `bc.rs` path | OPT42 (tools) |
| 6 | **HRTF in the audio path**: a measured HRIR set turns a stereo pan into a positioned sound | `GoogleChrome/omnitone` (SH-HRIR) | `crates/audio/src/hrtf.rs` |
| 7 | **BC7 compression in the asset cook**: a smaller cooked texture for the same picture | `GPUOpen-Tools/compressonator` (BC7) | `crates/assets/src/bc.rs:608` |
| 8 | **the delay knob cannot change the outcome**: a bench table whose last column (the hash) is constant on every row | `m2-md/rollback-netcode-lockstep` | MP26 |

## What the wave changed about our picture

1. **The safest donor is a presentation donor.** `nilpunch/unity-prediction-rollback`
   is the only donor in the whole wave whose Accept can be written as "hash
   equal with correction on and off". It is also archived, so the shape is
   re-derived, never copied.
2. **A report that says *why* is worth two u64s.** Klotho's funnel classifies a
   desync as `class=Input|State` from a command digest beside the state hash
   and names the *layer*: a divergence held in a system (an RNG) cannot be
   pinned by components alone. Our `crates/net/src/merkle.rs:125` `locate`
   says *where* and not *why*.
3. **PSNR alone draws the wrong conclusion.** The decibels report measured a
   temporal-gradient test that *lowers* PSNR by 0.3 dB but moves LPIPS from
   0.178 to 0.115, and an FSR2-style luminance reset that *collapses* the
   result by -3.4 dB on aliased content. Any accept that reports one number is
   reporting the wrong one.
4. **Half-pixel jitter conventions are worth 1.12 dB on their own** (decibels,
   measured). Our `crates/render/src/post_jitter_motion.rs` has a jitter
   convention; nothing checks it.
5. **The meshlet path is not just a cull.** `nanite-webgpu` culls per instance
   *and* per meshlet, builds the LOD hierarchy in the browser with
   meshoptimizer and METIS, and packs depth `u16` + octahedron normals `2xu8`
   into 32 bits because WebGPU has no `atomic<u64>`. Our `cull.wgsl` has one
   Hi-Z wall test and a sphere-vs-6-planes reject.
6. **Wave 1's rows are not finished.** Most good donors land on rows that
   already exist (LF5c, HX7, MP25, MP26, RC26, RC27, EG39). The wave's job is
   to make those rows' Accepts measurable, not to add rows.

## The files

One file per hunter, each with the method, the ranked donor table (stars, the
LICENSE file and its byte size, the donor file paths and their byte sizes, the
steal, our file and line, the accept, and the golden line), the wild ideas, and
what was seen and skipped with the reason.

- `agent-1-tools-ci.md` — the tools and CI lane. Top donor
  `Kobzol/cargo-wizard` -> row OPT42.
- `agent-2-audio-ui.md` — the audio and UI lane. Top donor
  `GoogleChrome/omnitone` (SH-HRIR) -> `crates/audio/src/hrtf.rs`.
- `agent-3-assets-content.md` — the assets and content lane. Top donor
  `GPUOpen-Tools/compressonator` (BC7) -> `crates/assets/src/bc.rs:608`.
- `agent-4-sim-net.md` — the simulation and the net lane. Top donors
  `xpTURN/Klotho`, `pond3r/ggpo`, `matthart1983/ticktape`; steal first
  `nilpunch/unity-prediction-rollback` -> MP25 and EG39.
- `agent-5-papers-ideas.md` — the papers lane. Top donors
  `RARS-oss/decibels`, `NVIDIA-RTX/RTXTF`, `Scthe/nanite-webgpu`; steal first
  the decibels error budget on LF5c -> `crates/render/src/post/taa.rs:145`.

## The golden hash

`d57b5c8ae3096614` (seed 2040, 600 ticks). Every donor in the wave carries a
golden line, and the rule was the same as wave 1: a donor may not change the
bytes the tick hashes. A donor is safe when it is diagnostic-only (it reads a
digest and prints), presentation-only (it touches the drawn pose, never the
sim), or it lands in a **new file** with no call site in the tick yet. The
decibels budget lands in the render crate (presentation), the HRIR in the audio
crate (presentation), the BC7 in the asset cook (a cooked file, not a sim
byte), the meshlet cull in the render crate (presentation), and the prediction
fade in the net crate's drawn pose (presentation). None of the five touches the
hashed authority, so none of the five can move the hash.

## What this card does not claim

- No code landed. Every accept above is a spec, not a measurement of our tree.
- No cargo, rustc, clippy or nextest ran. The wave is read-only by design: the
  box was busy and a hunt that builds is a hunt that fights for the CPU.
- The numbers in the tables are the donor's own numbers, measured by the hunter
  reading the donor's repo on 2026-10-21. They are targets, not our results.
