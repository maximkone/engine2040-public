# Hunter 3: assets, cooking and the world

Wave 2, 2026-10-21. Area: `crates/assets`, `crates/render`, `content/`, and the
tools that cook and budget assets. Wave 1 (`docs/research/hunt-2026-10-21/`) was
read first; nothing already mined there is re-mined here.

**How every number below was measured.** Stars and forks: `gh api
repos/<owner>/<repo>` (`.stargazers_count`, `.forks_count`). File sizes and blob
counts: `gh api repos/<owner>/<repo>/git/trees/HEAD?recursive=1` (the size is
the raw blob size in bytes, printed as KiB). Licences: the repo's own LICENSE
file, read live. Our own file:line anchors: read from this tree on
2026-10-21. Nothing is estimated.

**Our pain, in one line.** VRAM and download size (BC7 mode 6 only, PNG-era web
bundle), cook time (cook cache exists, no incremental dep log), and a Rome
city that is still tables instead of geometry.

---

## Ranked donors (1..12)

| # | donor (owner/repo + url) | licence | stars | what to steal (one sentence) | our file:line | our row id | measurable accept |
|---|---|---|---|---|---|---|---|
| 1 | GPUOpen-Tools/compressonator - https://github.com/GPUOpen-Tools/compressonator | MIT (`license/license.txt`: "Copyright (c) 2024 Advanced Micro Devices, Inc." + MIT text; the BC7 files carry "Copyright (c) 2007-2024 AMD / 2004-2006 ATI" + the same MIT text) | 1461 | The full BC7 encoder - all 8 modes plus the 2/3/4-subset partition search - so our encoder stops writing mode 6 only and can carry alpha. | `crates/assets/src/bc.rs:608` (`pub fn encode_bc7_block`, "Encode one 4x4 RGBA block as BC7 mode 6 (16 bytes)"), plus the refusal at `crates/assets/src/bc.rs:85` (`Bc7Mode`) | GS37 | `node tools/gh-check.mjs cargo test -p engine-assets --lib bc` green, and the alpha gate is real: a 4x4 block with a transparent texel encodes to a mode != 6 and `decode_bc7_block` (`crates/assets/src/bc.rs:664`) reads it back; the existing gate `bc7_albedo_1024_reaches_40_db_is_4x_smaller_and_threads_agree` (`crates/assets/src/bc.rs:1156`) still measures >= 40 dB on a 1024x1024 albedo and 4x smaller than RGBA8. |
| 2 | KhronosGroup/KTX-Software - https://github.com/KhronosGroup/KTX-Software | Apache-2.0 (`LICENSE.md`: "SPDX-License-Identifier: Apache-2.0", "Files unique to this repository generally fall under the Apache 2.0 license") | 1386 | The KTX2 file validator and its message catalogue, so our own KTX2 writer is checked against the spec's rule set instead of only against our own reader. | `crates/assets/src/ktx2_cook.rs:486` (`pub fn cook_ktx2`) and the container writer it calls, `crates/assets/src/ktx2_cook.rs:355` (`fn encode_level`) | GS37, PW40 | `node tools/gh-check.mjs cargo test -p engine-assets --lib ktx2` green and the 15 cooked KTX2 files under `content/` pass the donor's KTX2 validator rules with 0 errors; the byte total stays at or under the number PW40 already records (15 files, 9.79 MB, 4.5x smaller than RGBA8). |
| 3 | microsoft/DirectXTex - https://github.com/microsoft/DirectXTex | MIT (repo `license.spdx_id` = MIT) | 2164 | The BC6H/BC7 encoder reference (`DirectXTex/BC6HBC7.cpp`, 157.9 KiB) and the GPU BC7 encoder shader (`DirectXTex/Shaders/BC7Encode.hlsl`, 70.0 KiB), used as the oracle our Rust encoder must match block for block. | `crates/assets/src/bc.rs:603` (the doc comment on `encode_bc7_block`) and `crates/assets/src/ktx2_cook.rs:185` (`fn downsample`, which today is a plain box filter) | GS37 | `node tools/gh-check.mjs cargo test -p engine-assets --lib bc` green and 1000 random 4x4 RGBA blocks encode to the same 16 bytes as the donor's BC7 mode 6; the mip chain PSNR against the donor's `filters.h` path is >= the value we measure today and is printed by the test. |
| 4 | CesiumGS/gltf-pipeline - https://github.com/CesiumGS/gltf-pipeline | Apache-2.0 (`LICENSE.md`, 10.8 KiB) | 2135 | The prune-and-report pair - `lib/removeUnusedElements.js` (26.7 KiB) and `lib/getStatistics.js` (6.0 KiB) - so a cooked asset prints its own tris / verts / VRAM estimate instead of only a byte count. | `crates/assets/src/optimize.rs:278` (`pub fn normalize_document`, the dedup+prune pass) and `crates/assets/src/budget.rs:9` (`pub struct Caps`, the cap table) | CT39, CT41 | `node tools/gh-check.mjs cargo test -p engine-assets --lib optimize` green and `content/fighters/BUNDLE-AUDIT.md` gains a per-asset row with tris, verts, texture pixels and estimated VRAM MB before and after the cook, matching the CT39 accept wording "BC encode measured MB before/after"; CT41's per-LOD tris/ms/VRAM table joins the same numbers. |
| 5 | GameTechDev/MaskedOcclusionCulling - https://github.com/GameTechDev/MaskedOcclusionCulling | Apache-2.0 (`license.txt`, 10.2 KiB) | 694 | Masked Hi-Z occlusion: the depth-pyramid read plus the two-bullet front/back mask, so the GPU cull pass can drop occluded clusters instead of only frustum + cone. | `crates/render/src/gpu_cull.rs:126` (`pub fn check_inputs`, the pass that feeds `shaders/cull.wgsl`) and `crates/render/src/shaders/cull.wgsl` (3,654 bytes) | HX7 | `node tools/gh-check.mjs cargo test -p engine-render --lib gpu_cull` green; the cull report prints `culled=N visible=M` and the visible set equals the CPU oracle in `crates/render/src/draw.rs:246`; a scene behind a wall reports the occluded clusters as culled and the count is printed, not asserted by eye. |
| 6 | GameTechDev/OcclusionCulling - https://github.com/GameTechDev/OcclusionCulling | Apache-2.0 (`license.txt`, 10.2 KiB) | 413 | The CPU side of the same idea: a depth-buffer masked rasterizer with a Hi-Z pyramid (`DepthBufferMaskedRasterizerScalarMT.cpp`, 18.8 KiB, and `DepthBufferRasterizerAVXMT.cpp`, 23.2 KiB) so occlusion is decided on the CPU without a readback stall. | `crates/render/src/draw.rs:246` (`pub fn cull_cpu_flags_into`, the CPU oracle HX7 compares against) | HX7 | `node tools/gh-check.mjs cargo test -p engine-render --lib draw` green; the CPU cull flags a cluster behind the wall as culled with 0 false negatives, and the CPU and GPU cull counts agree on the same scene (both numbers printed). |
| 7 | citiesskylines-csur/CSUR - https://github.com/citiesskylines-csur/CSUR | GPL-3.0 (`LICENSE`, 34.3 KiB, "GNU GENERAL PUBLIC LICENSE Version 3"); owner rule 2026-10-10 allows porting from any source, so this is a read-and-adapt donor, not a copy | 229 | The city grammar as data: node and segment presets with connection flags and per-type lane widths (`prefab/templates/node_presets.json`, 2.0 KiB, `segment_presets.json`, 3.5 KiB, `netinfo/{basic,bridge,elevated,slope,tunnel}.json`), plus the builder that turns an OSM road graph into a typed graph (`core/csur.py`, 23.3 KiB, `builder/builder.py`, 26.4 KiB). | `crates/world/src/lib.rs:4` (the module list - `crates/world/src/city.rs` does **not** exist yet; WD13 owns it and this is where `pub mod city;` goes) | WD13 | `cargo test -p engine-world --lib city` green: an OSM JSON slice parses into a directed 2D road graph, the interpreter walks a rule body, and the node table carries the 5 measured node kinds (default, transition, end, trafficlight, notrafficlight_nt); the JSON table is a new file with its own digest, printed twice and equal. |
| 8 | tuyoogame/YooAsset - https://github.com/tuyoogame/YooAsset | Apache-2.0 (`LICENSE`) | 3392 | The catalog idea: one small manifest that lists every bundle with its hash so the client downloads only the bundles it needs (`BuildMapContext.cs`, 5.0 KiB, `BuildBundleInfo.cs`, 8.1 KiB, `TaskCreateCatalog_AFBP.cs`, 0.6 KiB, `TaskCreateManifest_AFBP.cs`, 0.5 KiB). | `crates/assets/src/bundle_cook.rs:1` (the `.e2bundle` writer) and `tools/web-build.mjs:1` (the bundle step WEB6 still owes) | WEB6 | `node tools/gh-check.mjs cargo test -p engine-assets --lib bundle` green and the web bundle carries a catalog: the browser fetches the catalog first, then only the bundles the scene names; the bytes downloaded for the arena are printed and are below the `first_bundle` bytes PW40 records today; a missing blob still fails loudly by name. |
| 9 | ninja-build/ninja - https://github.com/ninja-build/ninja | Apache-2.0 (`COPYING`, 11.1 KiB) | 13315 | The deps log (`src/deps_log.cc`, 13.6 KiB) and build log (`src/build_log.cc`, 11.7 KiB): an *implicit* dependency (a file a scanner read, not a declared input) is recorded, so an edit to a sidecar invalidates exactly the cooks that read it. | `crates/assets/src/cooker.rs:1` (the cook cache and its blake3 key) and `crates/assets/src/recook.rs:1` (the debounced recook queue) | CT75, CT79 | `node tools/gh-check.mjs cargo test -p engine-assets --lib cooker` green and a cooked asset records the sidecar files it read; changing one sidecar recooks exactly the assets that read it and nothing else, with the recooked set printed; the cook cache still returns byte-identical bytes for the same key. |
| 10 | TokisanGames/Terrain3D - https://github.com/TokisanGames/Terrain3D | MIT (`LICENSE.txt`, 1.1 KiB) | 4380 | A terrain mesher that builds a LOD'd mesh from a heightmap off-thread and a region streamer that loads terrain tiles under a byte budget (`src/terrain_3d_mesher.cpp`, 16.8 KiB, `src/terrain_3d_region.cpp`, 22.5 KiB, `src/terrain_3d_data.cpp`, 59.6 KiB). | `crates/world/src/terrain.rs:1` (our heightmap + hole bitmask + splat, `TVER` version 1) and `crates/world/src/cells.rs:1` (the async cell stream with a byte budget) | NV14, CT62 | `cargo test -p engine-world --lib terrain` green and the mesher output for one 1024x1024 heightmap is byte-identical twice; the per-map bytes + VRAM + hitch count land in `content/maps/STREAM-REPORT.md` with the adapter line, as CT62 asks. |
| 11 | thought-machine/please - https://github.com/thought-machine/please | Apache-2.0 (`LICENSE`, 11.1 KiB) | 2620 | A content-addressed on-disk cache with a local directory store and an HTTP tier (`src/cache/dir_cache.go`, 16.1 KiB, `src/cache/http_cache.go`, 5.9 KiB, `src/cmap/hash.go`, 5.1 KiB), so a cook artifact is fetched by digest and never rebuilt when it is already on disk. | `crates/assets/src/archive.rs:1` (the zero-copy archive over a shared byte mapping, where a digest-keyed entry store fits) and `crates/assets/src/cooker.rs:1` | CT79 | `node tools/gh-check.mjs cargo test -p engine-assets --lib cooker` green and a second cook of an unchanged asset reads the artifact from the cache instead of running the closure; the receipt prints "cache hit" plus the digest, and the cache directory holds exactly one file per digest. |
| 12 | fsfe/reuse-tool - https://github.com/fsfe/reuse-tool | `pyproject.toml`: `license = "Apache-2.0 AND CC0-1.0 AND CC-BY-SA-4.0 AND GPL-3.0-or-later"`; the repo ships the 4 licence texts under `LICENSES/` (Apache-2.0 11.1 KiB, CC-BY-SA-4.0 19.7 KiB, CC0-1.0 6.9 KiB, GPL-3.0-or-later 34.3 KiB) | 594 | The REUSE sidecar pattern: every shipped file carries `SPDX-FileCopyrightText` and `SPDX-License-Identifier`, and the repo carries the licence texts, so the licence truth is machine-readable instead of prose in a PROVENANCE table. | `tools/license-gate.mjs:1` (the licence gate that already reads repo licence files and source headers) and `content/rome/PROVENANCE.md:1` (the prose table it would replace) | E37 | `node tools/license-gate.mjs` prints `unlicensed 0` for every file under `content/` and `crates/`, and every donor row in `content/rome/PROVENANCE.md` has a matching `SPDX-License-Identifier` sidecar; the gate counts are printed before and after. |

### Notes on the ranking

- 1, 2 and 3 all land on **GS37**. That is the point: GS37 says the encoder has
  "BC1 and BC3 only", which is stale - `crates/assets/src/bc.rs:44` already has a
  `Bc7` format and `crates/assets/src/texture_cook.rs:32` picks BC7 for colour.
  The real gap is narrower and sharper: `crates/assets/src/bc.rs:666` reads
  **mode 6 only** and refuses everything else, so alpha cannot survive. Donor 1
  closes that, donor 2 makes the container legal, donor 3 is the oracle.
- 5 and 6 are one idea from two sides: 5 is the GPU read, 6 is the CPU decide.
  HX7 already names `nff747/meshlet-culling-wgsl` as its donor; these two add
  the occlusion half it does not have.
- 9 and 11 are the cook-time pair. Our `crates/assets/src/cooker.rs:1` already
  hashes source + settings + version + dependency keys; what it lacks is the
  *implicit* dependency (ninja's deps log) and a digest-keyed artifact store
  (please's dir cache).
- 8 is the only donor that closes a **user-visible** gap: WEB6 still owes the
  bundle step in `tools/web-build.mjs`, and a catalog is what makes "download
  only what the scene names" work in a browser.

---

## Wild ideas (10)

Not repos yet. Each has the measurement that would prove it.

1. **Cook the meshlets, do not derive them at load.** Bake the meshlet cluster
   set into the same bundle entry as the mesh, so the runtime never re-clusters.
   Proof: cook wall-clock for 1000 arena meshes drops from X s to Y s and the
   bundle grows by Z bytes; `cargo test -p engine-assets --test it meshlet_vectors::`
   green and the cluster bytes equal the cook-time bytes.
2. **A byte floor test in CI.** The web bundle and the arena texture set get a
   hard cap, checked by a tool, so VRAM and download size stop being opinions.
   Proof: `node tools/web-budget.mjs` exits 1 above the cap and prints the bytes;
   the cap is the number the first bundle measures today, minus 20%.
3. **Delta cooking over the catalog.** The browser asks for one digest and gets
   only the blobs that changed.
   Proof: a one-byte edit to one fighter GLB changes the downloaded byte count by
   less than 1% of the bundle; the count is printed by the fetch receipt.
4. **Rome insula table from Pompeii RDF.** `p-lod/p-lod-data` ships
   `insula.ttl` (503.3 KiB), `street.ttl` (29.6 KiB) and `region.ttl` (36.7 KiB)
   for Pompeii; cut the street centrelines and block rings into our table shape.
   Proof: `content/rome/streets.toml` gains N street rows and M block rows with
   their own blake3 digest, printed twice and equal; the licence question is
   answered before a byte is copied (see Rejected #8).
5. **Terrain meshlets from the heightmap.** Build the terrain clusters in the
   same shape as the mesh meshlets so terrain shares one cull path.
   Proof: the terrain draw call count drops from N to M and the reference-GPU ms
   for the terrain pass is printed with the adapter line in
   `content/maps/STREAM-REPORT.md`.
6. **One shared ORM texture for the whole kit.** Occlusion, roughness and
   metallic packed once into one RGBA map that every kit material samples.
   Proof: the distinct texture count in the kit drops from N to M and the arena
   VRAM estimate in `content/fighters/BUNDLE-AUDIT.md` falls by the measured
   megabytes; the per-material look is unchanged in a paired capture.
7. **A texture-LOD table computed at cook, not at load.** Pick the mip bias per
   material from its screen size, so the cook writes the residency table the
   streamer needs.
   Proof: `crates/assets/src/residency.rs` loads the table instead of guessing;
   the residency unit test shows a request over the budget being refused by
   name, and the first-frame hitch stays under 2 ms on the reference GPU.
8. **Street width from the road grade.** `tools/city-osm.mjs:8` sets every road
   to `roadW: 6` metres; the Roman road grades give a width per class, so the
   same geometry carries a plausible width.
   Proof: the generated `content/urban/pilot.json` road widths take at least
   three distinct values, and the triangle count of the city mesh falls by the
   measured percentage against the uniform 6 m street.
9. **A KTX2 lint in the cook.** Run the donor-2 validator rules over every
   cooked file the cook writes and refuse a file that fails.
   Proof: `node tools/gh-check.mjs cargo test -p engine-assets --lib ktx2` green
   with the validator case count printed, and a deliberately broken header is
   refused with the rule id named.
10. **WebP or AVIF for the browser fallback.** When the adapter has no
    compressed texture support, transcode the KTX2 to WebP instead of shipping
    RGBA8.
    Proof: the web bundle bytes fall by the printed ratio against the PNG
    baseline and the fallback frame is non-blank (a pixel-difference check, not
    a screenshot).

---

## Rejected (10)

Each was looked at and dropped, with the measured reason.

1. **KhronosGroup/glTF-Validator** (475 stars, Apache-2.0, `LICENSE` read
   live). Dropped as a *ranked* donor because AF4 already names it: "Donors:
   Unreal EditorValidatorBase, Godot configuration warnings, glTF-Validator
   (Apache-2.0), read 2026-10-20". Wave 1 read it. It stays useful as the
   conformance oracle for donor 4's prune pass, but there is nothing left to
   mine.
2. **zeux/meshoptimizer** (wave 1, `donors-seen.txt:2184`). The meshlet cone
   cull idea is already reimplemented in `crates/assets/src/meshlet_cull.rs:19`
   ("Cluster-cut idea adapted from meshoptimizer (Zeux, MIT)"). Re-mining it
   would repeat E70.
3. **BinomialLLC/basis_universal** and **marcogomez/basisu** (wave 1,
   `donors-seen.txt:264` and `:1173`). PW40 already names `basisu/src/ktx2.rs`,
   `basisu/src/transcoder.rs` and `basisu/src/dispatch.rs` as the transcode
   donor, with the licence read live on 2026-10-20. Donors 1 and 2 fill the
   gap PW40 left open (the encoder side and the container side).
4. **jpcy/xatlas** (wave 1, `donors-seen.txt:1036`). Already ported:
   `crates/assets/src/uv2_chart.rs:7` carries
   "PORT(jpcy/xatlas@f700c77:source/xatlas/xatlas.h)". No new work.
5. **donmccurdy/glTF-Transform** (1989 stars, MIT). Wave 1 (`donors-seen.txt:810`)
   and already ported twice: `crates/assets/src/optimize.rs:8` and `:10` name
   `donmccurdy/glTF-Transform@3797aad5` for dedup and prune. Donor 4 was picked
   over it because the prune + statistics pair is still missing from our tree.
6. **microsoft/UVAtlas** (928 stars, MIT, `archived = true`). Archived, and the
   isochart problem it solves is already solved by our own port of xatlas
   (`crates/assets/src/uv2_chart.rs:1`). Nothing to steal.
7. **apple-aiml-research/ARKitScenes** (971 stars). The `LICENSE` is Apple's
   own terms - "this Apple software is supplied to you by Apple Inc. ... in
   consideration of your agreement to the following terms" - not an OSI
   licence and not a dataset licence our legal lane can accept. The data
   licence is a separate, unclear term; dropped on the licence, not the stars.
8. **p-lod/p-lod-data** (5 stars, 18,249 KB). Its 15 blobs carry **no LICENSE
   file** and the README names the funders (PALP via the Getty Foundation, PBMP
   via the NEH) but states no licence at all. Pompeii street data is exactly
   what we want for wild idea 4, but a data set with no stated licence cannot
   pass our own `tools/license-gate.mjs`. Ask the owner before a byte moves.
9. **NASA-AMMOS/3DTilesRendererJS** (2503 stars, Apache-2.0). It needs a 3D
   Tiles endpoint to talk to (implicit tiling, b3dm payloads). We have no tile
   server, and `crates/world/src/cells.rs:1` already streams our own cells under
   a byte budget. Adding a tile protocol buys nothing we can host.
10. **core-code/LibVT** (122 stars, `license.spdx_id` = NOASSERTION, last push
    2015-09-25). Eleven years stale, no declared licence, and the virtual
    texture idea is better served by our own `crates/assets/src/residency.rs`
    budget model. Dropped on the licence plus the age.

---

## Donors seen but skipped

Wave-1 donors in my lane that I deliberately did **not** re-mine. The line
numbers are `docs/research/hunt-2026-10-21/donors-seen.txt`.

- `zeux/meshoptimizer` (:2184), `gwihlidal/meshopt-rs` (:901) - the meshlet
  cone cull is already ours (`crates/assets/src/meshlet_cull.rs:19`).
- `BinomialLLC/basis_universal` (:264), `marcogomez/basisu` (:1173),
  `KhronosGroup/Basis-Universal-Transcoders` (:1058) - PW40 already names the
  transcode donor.
- `hasenbanck/block_compression` (:910) - already ported
  (`crates/assets/src/texture_cook.rs:6`).
- `jpcy/xatlas` (:1036) - already ported
  (`crates/assets/src/uv2_chart.rs:7`).
- `donmccurdy/glTF-Transform` (:810) - already ported twice
  (`crates/assets/src/optimize.rs:8`).
- `BVE-Reborn/ktx2` (:325), `BVE-Reborn/rend3` (:326) - wave 1's KTX2 and
  renderer donors.
- `KhronosGroup/glTF` (:1059), `KhronosGroup/glTF-Sample-Viewer` (:1061),
  `KhronosGroup/glTF-Sample-Assets` - the sample corpus is wave 1's.
- `google/filament` (:867) - the PBR reference was read in wave 1.
- `mrdoob/three.js` (:1279) - PW40 already cites its `KTX2Loader.js` probe
  order.
- `tordanik/OSM2World` - already ported (`tools/city-osm.mjs:3`), which is why
  donor 7 is a *grammar table* donor rather than another OSM reader.
- `mxgmn/WaveFunctionCollapse` (:1290), `krychu/wfc` (:1077),
  `Coac/wave-function-collapse` (:393), `TUnit/WFC` (:2034) - our WFC already
  exists (`crates/world/src/wfc.rs`).
- `Electronicarts/fastnoise` (:620), `bones-ai/rust-procedural-world` (:287),
  `IgorKonovalov/voxel-engine` (:961) - procedural noise and voxels are in the
  wave-1 set.
- `AWMC/geodata`, `Diasito/urbesetorbis`, `johaahlf/dare` - the Rome geo data
  behind HX5; `content/rome/PROVENANCE.md:11` already records the two we cut.
- `nff747/meshlet-culling-wgsl` - named as the donor by HX7 itself; I looked at
  the occlusion half instead (donors 5 and 6).
- `Sollimann/bonsai` (:1837), `natepac/crowdbake` (:1297) - crowd and impostor
  bakes belong to the anim lane, not to the asset pipeline.
