# Arena realism budget: 12 techniques (QA rows)

Date: 2026-10-18. Question: how a small gladiator arena (one sun, fixed walls, fighters, a stands crowd) reads as realistic on a modest GPU in the browser. Engine: Rust wgpu, WebGPU, web tier at 1024 textures and 2048 particles.

Labels used below:
- "fetched" = a page read in this pass; the claim is quoted or taken from it.
- "snippet" = search result text only, no page read.
- "repo" = a file read or grepped in this checkout.
- "ours" = arithmetic or reasoning in this doc, not a source claim.
- "unverified" = not confirmed by any source read.

## 1. Where the repo stands (repo)

- `content/arena/sources/PROVENANCE.md`: five ambientCG CC0 1K sets (Bricks097, Ground093C, Planks039, Metal046B, PavingStones150), GL normals, nothing cooked yet.
- `crates/assets/src/ktx2_cook.rs` (lines 39-46): colour BC1 sRGB, normal BC5 (linear, GL), ORM BC1, full mip chain to 1x1.
- `crates/render/src`: `ibl.rs` (HDRI bake), `baked_gi.rs` (lightmap plus validity mask plus fighter SH probes, RL5), `shadow_cascade.rs` (arena-day sun cascades, RL27), `gtao.rs` (half-res GTAO), `decals.rs` (deferred decals), `particles_gpu.rs`, `crowd_inst.rs` (instanced crowd staging), `post.rs` (TAA/TAAU, bloom, exposure, ACES Narkowicz fit, grade), `aa_scale.rs` (SMAA fallback, dynamic resolution).
- `crates/web/src/tier_view.rs` `facts()` (test helper, lines ~290-310): shadow 1024 px, 2 cascades, GTAO half, TAA off, 2048 particles, 128 decals, texture 1024. This is a test fixture; the real tier table was not read.
- `docs/plan/web-and-realism.md` gold item 7 already plans RL34 (hue-keeping tonemap), RL7 (contact shadows), LF3 (half-res GTAO on web) and LF5 (SMAA or FXAA as web default).

## 2. The 12 techniques

### QA15 Pick the GPU texture format at load
- Basis Universal transcoder wiki, https://github.com/BinomialLLC/basis_universal/wiki/How-to-Use-and-Configure-the-Transcoder (snippet): modes ETC1S, UASTC LDR, UASTC HDR; UASTC LDR transcodes "particularly fast" to ASTC 4x4 and BC7 (snippet from basis_universal README, https://github.com/zeux/basis_universal).
- WebGPU rule: an adapter must offer texture-compression-bc, or both -etc2 and -astc, https://www.w3.org/TR/webgpu/ (snippet; the MDN page was not fetched).
- Licence: basis_universal Apache-2.0 from memory, unverified in this pass.
- Bytes (ours): BC1 0.5 B/texel, BC5 and BC7 and ASTC 4x4 1 B/texel.
- Our cook uses BC1 colour, BC5 normal, BC1 ORM (repo). BC7 colour doubles colour bytes; the quality gain is not measured.
- Choice rule (ours): ASTC if astc, else BC7 if bc, else ETC2, else RGBA8.
- Maps onto: `crates/assets/src/ktx2_cook.rs`, `bc.rs`, `bin/arena_cook.rs`; the adapter feature request in `crates/web` (repo: `limits.rs` requests defaults only).
- Cost: M.
- Row: `- [ ] QA15 look: pick the GPU texture format at load (ASTC, BC7, ETC2, RGBA8) from the adapter features and log the choice. Diff: regular. Tier B. Size S. Owns a format-choice fn in crates/assets with a test, and the feature request in crates/web. Accept: cargo test -p engine-assets --lib format_choice prints the choice for bc-only, astc-only, both and neither (neither gives RGBA8). F2P: no runtime choice today. P2P: golden d57b5c8ae3096614 unchanged. needs: VR2.`

### QA2 Texel density and the 1K versus 2K budget
- Polycount forum threads (snippet, low authority): 512 px/m is common; 1024 px/m is "about as high as you'll see on current consoles" for human-scale environments.
- Ours: a 1K map over 2 m is 512 px/m; over 4 m it is 256 px/m.
- Ours bytes: a 1K map is 1,048,576 texels. Per material (BC1 + BC5 + BC1) is 2 B/texel = 2 MiB, 2.67 MiB with mips. Five sets is about 13.3 MiB on the GPU. 2K is 4x, about 53 MiB.
- The raw JPG set is 13.6 MiB on disk (repo, PROVENANCE); that is a different quantity.
- Web tier cap is 1024 (repo test helper). Use 2K only for hero trim sheets, if at all (ours).
- Maps onto: `content/arena` material table, `tier_view` texture cap.
- Cost: S.
- Row: `- [ ] QA2 look: fix the texel density per arena material (px/m) and a 1K/2K rule, with the GPU MiB per material printed. Diff: regular. Tier B. Size S. Owns the content/arena material table and the texture cap in crates/web tier facts. Accept: a receipt table of px/m and MiB per material at 1K and 2K; no material above the web cap. F2P: no density table. P2P: golden d57b5c8ae3096614 unchanged. needs: VR2.`

### QA3 Baked lightmap plus SH probes for the fighters
- Ramamoorthi and Hanrahan 2001, "An Efficient Representation for Irradiance Environment Maps", https://cseweb.ucsd.edu/~ravir/papers/envmap/envmap.pdf (snippet, paper not read in full): 9 SH coefficients (bands 0-2) give about 1% average error.
- Ours: 9 coefficients x RGB = 27 floats, 108 bytes at f32 per probe.
- Lightmap side: no source read in this pass.
- Repo: `baked_gi.rs` already does lightmap plus validity mask plus fighter SH (RL5).
- Maps onto: `crates/render/src/baked_gi.rs`, consumers ED1 and WIN1.
- Cost: M.
- Row: `- [ ] QA3 look: bake one fixed-sun lightmap for the arena floor and walls, and one SH probe per fighter, and show both in the lit frame. Diff: regular. Tier B. Size M. Owns crates/render/src/baked_gi.rs and its arena consumer. Accept: a pass-truth capture with baked GI on vs off differs on the floor under the stands; probe bytes printed. F2P: fighters light from a flat ambient. P2P: golden d57b5c8ae3096614 unchanged. needs: RL5.`

### QA4 One sun with cascaded shadows and a practical split
- GPU Gems 3 ch. 10, https://developer.nvidia.com/gpugems/gpugems3/part-ii-light-and-shadows/chapter-10-parallel-split-shadow-maps-programmable-gpus (fetched): the practical split is a weighted blend of logarithmic and uniform splits; lambda default 0.5; PSSM(3) and PSSM(4) "achieve a good trade-off".
- Same page (fetched, 2005 hardware, not ours): PSSM(3) at 1K x 1K maps ran at more than 31 fps on a GeForce 7800 GTX; PSSM(4) more than 27 fps. Hardware PCF is one instruction; shader PCF about 25.
- Repo: web tier has 2 cascades at 1024 (test helper); `shadow_cascade.rs` holds the split code.
- Maps onto: `crates/render/src/shadow_cascade.rs`, `twin_shadow.rs`.
- Cost: S.
- Row: `- [ ] QA4 look: set the arena sun to 2 cascades with the practical split at lambda 0.5 and print the split table for the arena camera. Diff: regular. Tier B. Size S. Owns crates/render/src/shadow_cascade.rs and its test. Accept: cargo test -p engine-render --lib shadow_cascade prints the splits; the arena capture shows no shadow swim when the camera turns. F2P: split values not printed. P2P: golden d57b5c8ae3096614 unchanged. needs: none.`

### QA5 Half-res GTAO on the web tier
- Jimenez et al. 2016, ATVI-TR-16-01, https://www.activision.com/cdn/research/PracticalRealtimeStrategiesTRfinal.pdf (snippet): the paper claims ground-truth-matching AO in "half a millisecond" on console hardware. The claim is theirs; not measured here.
- XeGTAO, https://github.com/GameTechDev/XeGTAO (snippet): DirectX/HLSL, MIT licence per the repo description, unverified by reading the file.
- Repo: `gtao.rs` is the half-res GTAO path and the web tier already says half.
- Cost: S.
- Row: `- [ ] QA5 look: GTAO at half res on the web tier with an on/off capture, and a timing receipt on the PC. Diff: regular. Tier B. Size S. Owns crates/render/src/gtao.rs and the web tier GTAO setting. Accept: a receipt with GTAO ms per frame at half and full res on the PC; the web tier capture shows the contact occlusion under the stands. F2P: no timing receipt. P2P: golden d57b5c8ae3096614 unchanged. needs: LF3.`

### QA6 Short-range screen-space contact shadows
- Unity HDRP docs, https://docs.unity3d.com/Packages/com.unity.render-pipelines.high-definition@14.0/manual/Override-Contact-Shadows.html (snippet): close-range shadows traced in screen space through the depth buffer; length set in meters.
- Unreal docs, https://dev.epicgames.com/documentation/en-us/unreal-engine/contact-shadows-in-unreal-engine (snippet): length is a fraction of the screen; shorter lengths give less noise.
- Depth-buffer ray march reference, https://gist.github.com/h3r2tic/9c8356bdaefbe80b1a22ae0aaee192db (snippet).
- Step count and reach: no source. Unverified.
- Repo: RL7 is planned (`docs/plan/web-and-realism.md`); a grep of crates/render/src found no contact-shadow file (not exhaustive).
- Cost: M.
- Row: `- [ ] QA6 look: add a short screen-space contact shadow pass for the sun at fighter feet and stand edges, off by default on the web tier. Diff: regular. Tier B. Size M. Owns a new pass in crates/render and its switch in the tier. Accept: on/off capture where feet and stand edges gain a dark contact line; the receipt prints ms per frame. F2P: no contact shadows. P2P: golden d57b5c8ae3096614 unchanged. needs: RL7.`

### QA7 Image-based light from one small HDRI with a prefiltered mip chain
- Karis 2013, Epic notes https://cdn2.unrealengine.com/Resources/files/2013SiggraphPresentationsNotes-26915738.pdf and slides https://blog.selfshadow.com/publications/s2013-shading-course/karis/s2013_pbs_epic_slides.pdf (snippet): split sum; prefiltered cubemap mips per roughness; a 2D BRDF LUT. The n = v = r assumption loses stretched highlights.
- Ours: a 128 px base cube has 8 mip levels (128 down to 1). The 128 base is an assumption, unverified as Karis's choice.
- Repo: `ibl.rs` bakes from `content/arena/hdri/polyhaven-studio-small-09-1k.hdr` (test include, line ~3425). The budget note says the 1.6 MB studio HDRI sits in the wasm. Its Poly Haven licence was not read here.
- Maps onto: `crates/render/src/ibl.rs`.
- Cost: S.
- Row: `- [ ] QA7 look: print the prefiltered IBL mip count and bytes for the arena HDRI, and capture a roughness ramp on the stone. Diff: regular. Tier B. Size S. Owns crates/render/src/ibl.rs receipts. Accept: a receipt with mip count and bytes; the roughness ramp capture shows the blur rising with roughness. F2P: no roughness ramp. P2P: golden d57b5c8ae3096614 unchanged. needs: none.`

### QA8 Ground contact: pooled decals and a surface-tinted dust burst
- Unreal footprint recipe, https://wacki.me/blog/2015/12/creating-sand-footprints-in-unreal-engine-4/ (snippet): line trace on foot contact, decal oriented to the hit normal, spawned from an animation notify.
- Cost note, the-folded-frontier issue #138, https://github.com/TheShield2594/the-folded-frontier/issues/138 (snippet, a 2D game): small pooled decals and a dust burst, checked with a perf overlay. Not a benchmark.
- Heightfield deformation is costlier (arXiv 2209.10215, snippet).
- Repo: `decals.rs` (web cap 128 in the test helper); `particles_gpu.rs` (web cap 2048).
- Cost: S.
- Row: `- [ ] QA8 look: foot-contact decals and a short sand dust burst on each landing, from a pooled decal set under the web caps. Diff: regular. Tier B. Size S. Owns crates/render/src/decals.rs use and a foot event in crates/game. Accept: a capture with prints in the sand; the pool never exceeds the cap; the receipt prints decal and particle counts. F2P: no ground contact. P2P: golden d57b5c8ae3096614 unchanged. needs: none.`

### QA9 Heat haze by screen-space offset (optional, small)
- Screen-space distortion resamples the frame with an offset from a scrolling noise or normal map (snippet, several tutorials). Wolfire 2006, https://www.wolfire.com/blog/2006/03/screen-space-distortion/ (snippet): much faster than cube-map distortion, because the scene is not rendered again.
- Keep the offset small: one shader example uses a strength under 0.01 (snippet; an example, not a spec).
- Ours: one half-res pass with a soft mask; it costs fill rate and cannot show off-screen content.
- Repo: no heat haze file found.
- Cost: S.
- Row: `- [ ] QA9 look: a half-res heat shimmer over the brazier and hot-sand patches only, with a soft mask and a strength cap. Diff: regular. Tier B. Size S. Owns a new post pass in crates/render and its mask. Accept: on/off capture differs only inside the mask; the receipt prints ms per frame. F2P: no shimmer. P2P: golden d57b5c8ae3096614 unchanged. needs: none.`

### QA10 Crowd in the stands: instanced impostor cards
- Impostors and pseudo-instancing for GPU crowd rendering, ACM 2006, https://dl.acm.org/doi/pdf/10.1145/1174429.1174436 (snippet): 1,048,576 impostor characters at 4.3-5.0 fps on that paper's hardware; mixing impostors with instanced geometry was much slower. Dated; not our hardware.
- Forum claim, low credibility (snippet): 20,000 instanced crowd members with impostors far away.
- Guide tip, low authority (snippet, mocaponline): offset animation start times so the crowd does not move in unison.
- Repo: `crates/render/src/crowd_inst.rs` (instanced reacting crowd, slice 1).
- Cost: M.
- Row: `- [ ] QA10 look: the stands crowd as instanced impostor cards with a random animation offset, close seats keep the mesh. Diff: regular. Tier B. Size M. Owns crates/render/src/crowd_inst.rs and its arena caller. Accept: a receipt with draw calls and instances for the stands on the web tier fixture; a capture with stands filled. F2P: crowd is empty boxes. P2P: golden d57b5c8ae3096614 unchanged. needs: none.`

### QA11 Tonemapper choice and vignette
- Khronos PBR Neutral, https://github.com/KhronosGroup/ToneMapping/blob/main/PBR_Neutral/README.md (fetched): F90 0.04, Ks = 0.8 - F90 (0.76), Kd 0.15; output sRGB only (two flavours); no gamut mapping; no cost claim. Adopted by Blender, Babylon.js, Filament and three.js (snippet). Licence Apache-2.0 per a forum post (snippet, unverified).
- AgX (Blender 4.0 default), secondary blogs only (snippet): less highlight desaturation than ACES; reports mixed. No Blender or Sobotka source read.
- Repo: `post.rs` runs the ACES Narkowicz fit; RL34 is the planned hue-keeping swap.
- Vignette: no source read. Ours: a radial multiply in the grade pass, near-zero cost.
- Cost: S.
- Row: `- [ ] QA11 look: add PBR Neutral as the tonemap option beside the ACES fit, with a radial vignette in the grade pass. Diff: regular. Tier B. Size S. Owns the tonemap pass in crates/render/src/post.rs. Accept: a pass-truth capture pair on the arena (ACES and PBR Neutral); the vignette strength is a named constant. F2P: one tonemap only. P2P: golden d57b5c8ae3096614 unchanged. needs: RL34.`

### QA12 Bloom and cheap AA on the web tier
- Bjorge 2015 dual filter, https://www.froyok.fr/blog/2024-01-breakdown-syndicate/resources/presentations/siggraph2015-mmg-marius-slides.pdf (snippet): alternating downsample and upsample at a constant distance; fewer memory reads. The "7% bandwidth" sentence is cut off in the snippet, so unverified.
- Dual Kawase write-up, https://blog.frost.kiwi/dual-kawase/ (fetched in part): qualitative only, no pass counts or timings.
- FXAA: NVIDIA whitepaper (2009), under 1 ms at 1920x1200 on a GTX 480 at preset 2 (snippet, https://developer.download.nvidia.com/assets/gamedev/files/sdk/11/FXAA_WhitePaper.pdf); NVIDIA FXAA 3.11 sample at 0.88 ms at 1080p on a GTX 460 (snippet). Old hardware.
- SMAA about 3x the cost of FXAA or CMAA2 on the Intel Bistro scene (Intel CMAA2 paper, snippet; table hard to parse).
- TAA needs motion vectors (repo: `post.rs` owns TAA/TAAU; web tier taa off).
- Repo: `aa_scale.rs` holds the SMAA fallback; the bloom blur type in `post.rs` was not read.
- Cost: S.
- Row: `- [ ] QA12 look: web tier default AA is SMAA (or FXAA), TAA off, and bloom uses the dual filter if it beats the current blur in a timing receipt. Diff: regular. Tier B. Size S. Owns the web AA choice in crates/render/src/aa_scale.rs and the bloom pass in post.rs. Accept: a receipt with bloom and AA ms on the PC; the web capture shows no shimmer on the stand edges. F2P: no web AA default. P2P: golden d57b5c8ae3096614 unchanged. needs: LF5.`

## 3. What I could not verify

- Every ms figure is old-hardware or console-era or a forum claim. None was measured on our adapters or on the potato floor.
- The 2001 SH paper, the GPU Gems chapter and the Karis notes were read in part or by snippet; the lightmap side of QA3 has no source read.
- Contact shadow reach and step counts (QA6) have no source.
- Tonemap, bloom, basis_universal and XeGTAO licences were not read in their repos.
- MDN and the WebGPU spec were not fetched; the feature rule is from a snippet.
- AgX and the texel-density figures are from forums and blogs.
- The web tier values come from a test helper, not the real tier table.
